//! Explicit coordinated recovery of a v2 group. Old clocks are never scheduled again.
use super::*;
use rusqlite::{params,Connection,Transaction};
use std::collections::BTreeMap;

fn conflict(message:&str)->ApiError {ApiError(StatusCode::CONFLICT,"group_recovery_unavailable".into(),message.into())}
fn anchor(z:&Zone)->String {crate::auth::hash(&json!([z.id,z.endpoint_id,z.updated_at,z.queue_index,z.track.as_ref().map(|t|&t.id),z.queue.iter().map(|t|&t.id).collect::<Vec<_>>(),z.status,z.playback_id]).to_string())}
pub(super) fn hold_tx(tx:&Transaction<'_>,id:&str,result:&Value)->anyhow::Result<()> {
    if result["measured_version"]!=2{return Ok(());}
    let request:String=tx.query_row("SELECT request FROM sync_starts WHERE id=?1",[id],|r|r.get(0))?;let request:RequestStart=serde_json::from_str(&request)?;
    let epoch:Option<String>=tx.query_row("SELECT data FROM sync_output_epochs WHERE group_id=?1 ORDER BY rowid DESC LIMIT 1",[id],|r|r.get(0)).optional()?;
    let Some(epoch)=epoch else{return Ok(())};let epoch:Value=serde_json::from_str(&epoch)?;if epoch["current_logical_playback_id"].as_str().is_none(){return Ok(());}
    let mut held=Vec::new();
    for member in result["members"].as_array().into_iter().flatten() {
        let zid=member["zone_id"].as_str().unwrap_or("");let data:Option<String>=tx.query_row("SELECT data FROM zones WHERE id=?1",[zid],|r|r.get(0)).optional()?;let Some(data)=data else{return Ok(())};let mut zone:Zone=serde_json::from_str(&data)?;
        let matches=zone.playback_id.as_deref()==member["current_playback_id"].as_str().or_else(||member["playback_id"].as_str());
        if zone.endpoint_id!=member["endpoint_id"]||(!matches&&!(zone.playback_id.is_none()&&epoch["dispatched"]==false))||zone.queue.iter().map(|t|&t.id).ne(request.queue.iter()){return Ok(());}
        zone.status="stopped".into();zone.playback_id=None;held.push((zone.id.clone(),anchor(&zone)));
    }
    for(zone,anchor)in held{tx.execute("INSERT INTO sync_group_holds(group_id,zone_id,anchor) VALUES(?1,?2,?3) ON CONFLICT(zone_id) DO UPDATE SET group_id=excluded.group_id,anchor=excluded.anchor",params![id,zone,anchor])?;}Ok(())
}
type RequestStart=super::Request;
struct Snapshot {checkpoint:String,request:RequestStart,header:Value,zones:Vec<Zone>,credits:BTreeMap<usize,f64>,index:usize,position:f64,old_logical:Option<String>,accounts:HashMap<String,String>}
fn load(c:&Connection,id:&str,user:&str)->ApiResult<Snapshot> {
    let saved:Option<(String,String)>=c.query_row("SELECT request,result FROM sync_starts WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;let(request,result)=saved.ok_or_else(ApiError::missing)?;let request:RequestStart=serde_json::from_str(&request).map_err(anyhow::Error::from)?;let result:Value=serde_json::from_str(&result).map_err(anyhow::Error::from)?;
    let latest:Option<(String,String,String)>=c.query_row("SELECT id,data,listener_id FROM sync_output_epochs WHERE group_id=?1 ORDER BY rowid DESC LIMIT 1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;let(checkpoint,data,listener)=latest.ok_or_else(||conflict("旧分组没有 v2 恢复记录，请重新起播"))?;if listener!=user{return Err(ApiError::forbidden());}
    let header:Value=serde_json::from_str(&data).map_err(anyhow::Error::from)?;
    if request.measured_version!=2||result["phase"]!="interrupted"||header["current_logical_playback_id"].as_str().is_none(){return Err(conflict("此分组不处于可手动恢复的中断状态"));}
    let mut epochs=c.prepare("SELECT id,data FROM sync_output_epochs WHERE group_id=?1")?;
    for row in epochs.query_map([id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))? {let(eid,data)=row?;let data:Value=serde_json::from_str(&data).map_err(anyhow::Error::from)?;
        if data["dispatched"]!=true&&!data["output_members"].is_array(){continue;}
        for m in data.get("output_members").filter(|v|v.is_array()).unwrap_or(&data["members"]).as_array().into_iter().flatten(){if m["final_report_received"]!=true&&!c.query_row("SELECT EXISTS(SELECT 1 FROM sync_output_final_receipts WHERE epoch_id=?1 AND endpoint_id=?2)",params![eid,m["endpoint_id"].as_str().unwrap_or("")],|r|r.get::<_,bool>(0))?{return Err(conflict("仍在等待原输出的最终帧确认，请重新连接原输出"));}}
    }
    let mut zones=Vec::new();let mut accounts=HashMap::new();
    for m in header["members"].as_array().into_iter().flatten() {
        let zid=m["zone_id"].as_str().ok_or_else(||conflict("分组音区记录不完整"))?;
        let held:Option<(String,String)>=c.query_row("SELECT z.data,h.anchor FROM zones z JOIN sync_group_holds h ON h.zone_id=z.id WHERE h.group_id=?1 AND z.id=?2",params![id,zid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let(data,expected)=held.ok_or_else(||conflict("原音区已被修改或已放弃恢复，请重新起播"))?;let z:Zone=serde_json::from_str(&data).map_err(anyhow::Error::from)?;
        if anchor(&z)!=expected||z.status!="stopped"||z.playback_id.is_some()||z.endpoint_id!=m["endpoint_id"]||z.queue.iter().map(|t|&t.id).ne(request.queue.iter()){return Err(conflict("原音区或播放意图已变化，请重新起播"));}
        if c.query_row("SELECT EXISTS(SELECT 1 FROM sequence_recovery WHERE zone_id=?1)",[zid],|r|r.get::<_,bool>(0))? {return Err(conflict("音区已有单端恢复计划"));}
        let account:Option<String>=c.query_row("SELECT s.account_id FROM sync_output_sources s JOIN sync_output_epochs e ON e.id=s.epoch_id WHERE e.group_id=?1 AND s.endpoint_id=?2 ORDER BY e.rowid DESC,s.rowid DESC LIMIT 1",params![id,z.endpoint_id],|r|r.get(0)).optional()?;
        accounts.insert(z.endpoint_id.clone(),account.ok_or_else(||conflict("原输出缺少账号授权记录，请重新起播"))?);zones.push(z);
    }
    if zones.is_empty()||zones.len()>4{return Err(conflict("分组音区记录不完整"));}
    let segment=header["segment"].as_u64().ok_or_else(||conflict("分组曲序记录不完整"))? as usize;let paused=header["phase"]=="paused";
    let mut index=segment;let mut position=header["position"].as_f64().ok_or_else(||conflict("分组位置记录不完整"))?;
    if !paused {
        let last:Option<i64>=c.query_row("SELECT MAX(queue_index) FROM sync_output_sources WHERE epoch_id=?1 AND (frames>0 OR ended=1)",[&checkpoint],|r|r.get(0))?;index=index.max(last.unwrap_or(segment as i64) as usize);
        let at:Option<f64>=c.query_row("SELECT MAX(position) FROM sync_output_sources WHERE epoch_id=?1 AND queue_index=?2 AND (frames>0 OR ended=1)",params![checkpoint,index as i64],|r|r.get(0))?;position=at.unwrap_or(if index==segment{position}else{0.});
        let eof:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sync_output_sources WHERE epoch_id=?1 AND queue_index=?2 AND ended=1 AND position>=?3)",params![checkpoint,index as i64,position],|r|r.get(0))?;if eof{index+=1;position=0.;}
    }
    if index>=request.queue.len(){return Err(conflict("整组队列已经播放结束，请重新起播"));}
    let old_logical=if paused||index==segment{header["current_logical_playback_id"].as_str().map(String::from)}else{c.query_row("SELECT json_extract(grant,'$.logical_playback_id') FROM sync_output_sources WHERE epoch_id=?1 AND queue_index=?2 LIMIT 1",params![checkpoint,index as i64],|r|r.get::<_,String>(0)).optional()?};
    let mut query=c.prepare("SELECT queue_index,SUM(seconds) FROM (SELECT DISTINCT s.queue_index,json_extract(s.grant,'$.logical_playback_id') AS logical,COALESCE(g.seconds,0) AS seconds FROM sync_output_sources s JOIN sync_output_epochs e ON e.id=s.epoch_id LEFT JOIN group_output_totals g ON g.playback_id=json_extract(s.grant,'$.logical_playback_id') WHERE e.group_id=?1) GROUP BY queue_index")?;
    let credits=query.query_map([id],|r|Ok((r.get::<_,i64>(0)? as usize,r.get::<_,f64>(1)?)))?.collect::<rusqlite::Result<BTreeMap<_,_>>>()?;
    Ok(Snapshot{checkpoint,request,header,zones,credits,index,position,old_logical,accounts})
}
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("control")?;let result=s.db.run(move|c|Ok(load(c,&id,&a.id))).await?;
    match result{Ok(p)=>Ok(Json(json!({"available":true,"checkpoint_id":p.checkpoint,"queue_index":p.index,"position":p.position}))),Err(error)if error.0==StatusCode::CONFLICT=>Ok(Json(json!({"available":false,"reason":error.2}))),Err(error)=>Err(error)}
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {request_id:String,checkpoint_id:String}
pub async fn resume(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(mut request):Json<Request>)->ApiResult<(StatusCode,Json<Value>)> {
    a.require("control")?;request.request_id=uuid::Uuid::parse_str(&request.request_id).map_err(|_|ApiError::bad("恢复请求须包含 UUID"))?.to_string();
    let _zone=s.zone_lock.lock().await;let mut registry=s.sync_starts.lock().await;let rid=request.request_id.clone();
    let previous=s.db.run(move|c|Ok(c.query_row("SELECT group_id,checkpoint_id,user_id FROM sync_group_recoveries WHERE request_id=?1",[rid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?)).await?;
    if let Some((group,checkpoint,user))=previous{if user!=a.id{return Err(ApiError::forbidden());}if group!=id||checkpoint!=request.checkpoint_id{return Err(conflict("恢复请求 ID 已用于另一份检查点"));}drop(registry);return Ok((StatusCode::OK,super::saved(s.clone(),id).await?));}
    let group=id.clone();let user=a.id.clone();let snapshot=s.db.run(move|c|Ok(load(c,&group,&user))).await??;
    if snapshot.checkpoint!=request.checkpoint_id{return Err(conflict("分组检查点已变化，请重新查询"));}
    if registry.jobs.contains_key(&id)||registry.jobs.values().filter(|j|j.active()).count()>=32{return Err(conflict("分组已恢复或当前活动分组过多"));}
    let tracks=queue::resolve(&s,&snapshot.request).await?;let template=&snapshot.zones[0].queue;
    if tracks.iter().zip(template).any(|(a,b)|a.id!=b.id||a.format!=b.format||a.sample_rate!=b.sample_rate||a.bit_depth!=b.bit_depth||a.channels!=b.channels){return Err(conflict("音源格式已变化，请重新起播"));}
    let now=crate::clock::now_ns();let generation=registry.jobs.values().flat_map(|job|job.members.iter()).filter_map(|m|m.token["generation"].as_u64()).max().unwrap_or(0).max(registry.generation).max(snapshot.header["generation"].as_u64().unwrap_or(0))+1;let mut members=Vec::new();let mut endpoints=HashSet::new();
    for original in &snapshot.zones {
        if !endpoints.insert(original.endpoint_id.clone())||registry.jobs.values().any(|j|j.active()&&(j.owns_zone(&original.id)||j.owns_endpoint(&original.endpoint_id)))||s.zones.read().await.values().any(|z|z.id!=original.id&&z.endpoint_id==original.endpoint_id&&(z.status!="stopped"||z.playback_id.is_some())){return Err(conflict("原输出已被其他播放占用"));}
        let owner=s.bridge_owners.read().await.get(&original.endpoint_id).cloned().ok_or_else(||conflict("原输出尚未重新连接"))?;let account=s.db.run(move|c|Ok(c.query_row("SELECT user_id FROM sessions WHERE hash=?1 AND expires>?2",params![owner,chrono::Utc::now().timestamp()],|r|r.get::<_,String>(0)).optional()?)).await?;
        if account.as_ref()!=snapshot.accounts.get(&original.endpoint_id){return Err(conflict("原输出账号已变化，请重新起播"));}
        let endpoint=s.endpoints.read().await.get(&original.endpoint_id).cloned().unwrap_or(Value::Null);if ["playback_instance","group_output_v2","measured_listens_v2","scheduled_queue_v1","group_output_final_v2"].iter().any(|key|endpoint["capabilities"][*key]!=true){return Err(conflict("所有原输出均须支持完整分组 v2 协议"));}
        rate_adjustment::supported(&snapshot.request,&endpoint).map_err(|e|conflict(&e.to_string()))?;
        let tx=s.bridges.read().await.get(&original.endpoint_id).cloned().ok_or_else(||conflict("原输出尚未重新连接"))?;let estimate=calibration(&s,&endpoint,&tx,now).await.map_err(|e|conflict(&e.to_string()))?;
        let mut zone=original.clone();zone.queue=tracks.clone();zone.queue_index=snapshot.index;zone.track=zone.queue.get(snapshot.index).cloned();zone.position=snapshot.position;zone.status="stopped".into();zone.renew_playback();crate::zones::processing_allowed(&s,&zone).await?;
        let manual_power=group_power(&endpoint).map_err(|e|conflict(&e.to_string()))?;let mut token=json!({"version":1,"plan_id":id,"generation":generation,"core_clock_id":estimate["core_clock_id"],"host_clock_id":estimate["host_clock_id"],"expires_host_ns":host_time(&estimate,now+PREPARE_NS)?});if let Some(stamp)=&manual_power{token["group_power"]=stamp.clone();}
        members.push(Member{requested_rate_ppm:0.,clock:clock_feedback::Binding::Unseen,original:original.clone(),playback_id:zone.playback_id.clone().unwrap(),zone,uncertainty:estimate["network_uncertainty_ns"].as_u64().unwrap_or(250_000_000),host_deadline:0,tx,token,manual_power,phase:"preparing".into(),rate:0,frames:0,gate:Value::Null,last_seen:now,final_report:false});
    }
    let permits=members.iter().map(|m|m.tx.clone().try_reserve_owned()).collect::<Result<Vec<_>,_>>().map_err(|_|conflict("输出控制队列繁忙"))?;
    let mut queue=queue::Queue::new(&members,tracks.len());queue.begin_epoch(snapshot.index,snapshot.position,&members);queue.credited=snapshot.credits;queue.completed.extend(0..snapshot.index);
    let job=Job{sync_monitor:Default::default(),drift:Default::default(),rate_request:None,control:None,queue:Some(queue),request:snapshot.request,controller:a.session_hash,listener:a.id.clone(),created:now,expires:now+PREPARE_NS,deadline:None,phase:"preparing".into(),error:None,dispatched:false,members,settle_until:None};let commands=job.prepare_commands(&s).await?;if crate::clock::now_ns()>=job.expires{return Err(conflict("整组恢复准备已超时，请重新查询"));}
    let prepared=queue::durable::preparing(&s,&job).await?;let stored=job.clone();let checkpoint=request.checkpoint_id;let rid=request.request_id;let old=snapshot.old_logical;let index=snapshot.index;let at=chrono::Utc::now().timestamp_millis();
    s.db.run(move|c|{let tx=c.transaction()?;
        if let Some(old)=old{if tx.query_row("SELECT EXISTS(SELECT 1 FROM listen_claims WHERE playback_id=?1)",[&old],|r|r.get::<_,bool>(0))?{let(mut zone,_)=queue::listening(&stored,index);zone.status="paused".into();zone.updated_at=at;crate::stats::restore_sequence_tx(&tx,&stored.listener,&old,Some(&old),&zone)?;}}
        queue::durable::checkpoint_tx(&tx,&stored,&[],at)?;queue::durable::prepare_tx(&tx,&prepared,at)?;tx.execute("UPDATE sync_starts SET result=?2,updated_at=?3 WHERE id=?1",params![stored.request.request_id,stored.value().to_string(),at])?;
        tx.execute("INSERT INTO sync_group_recoveries(request_id,group_id,checkpoint_id,user_id,created_at) VALUES(?1,?2,?3,?4,?5)",params![rid,stored.request.request_id,checkpoint,stored.listener,at])?;tx.execute("DELETE FROM sync_group_holds WHERE group_id=?1",[&stored.request.request_id])?;tx.commit()?;Ok(())}).await?;
    let value=job.value();registry.generation=generation;registry.jobs.insert(id,job);for(permit,command)in permits.into_iter().zip(commands){permit.send(command);}s.emit("sync_start",value.clone());Ok((StatusCode::ACCEPTED,Json(value)))
}
