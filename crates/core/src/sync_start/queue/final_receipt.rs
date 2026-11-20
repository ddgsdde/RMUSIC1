//! Historical group finals only. No zone mutation or scheduling from persisted clocks.
use super::*;
use rusqlite::{params,Connection,Transaction};

#[derive(Deserialize,Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {request_id:String,endpoint_id:String,captured_at:i64,captured_host_ns:u64,report:Final}
#[derive(Deserialize,Serialize)]
struct Final {#[serde(rename="type")]kind:String,version:u32,measured_version:u32,phase:String,#[serde(rename="final")]finished:bool,start:Value,zone_id:String,core_playback_id:String,gate:Value,sample_rate:u32,programme_frames_played:u64,sequence_final:Vec<Source>}
#[derive(Deserialize,Serialize)]
#[serde(deny_unknown_fields)]
struct Source {playback_id:String,track_id:String,position:f64,frames:u64,ended:bool}
struct Row {playback:String,index:usize,grant:Value,frames:u64,rate:u32,position:f64,ended:bool}
struct Applied {increments:BTreeMap<usize,f64>,public:Value,changed:bool}
fn bad()->ApiError {ApiError::bad("分组最终回执的授权、源顺序、帧数或时刻无效")}
fn conflict()->ApiError {ApiError(StatusCode::CONFLICT,"group_final_conflict".into(),"分组仍在输出／收尾，或最终回执身份已用于其他内容".into())}

pub async fn submit(State(s):State<AppState>,a:Auth,Path(group):Path<String>,Json(mut request):Json<Request>)->ApiResult<Json<Value>> {
    a.require("control")?;request.request_id=uuid::Uuid::parse_str(&request.request_id).map_err(|_|bad())?.to_string();
    if request.endpoint_id.is_empty()||request.endpoint_id.len()>200||request.report.sequence_final.len()>9{return Err(bad());}
    let epoch=durable::epoch_key(&group,&request.report.start);let _zone=s.zone_lock.lock().await;let mut registry=s.sync_starts.lock().await;
    if registry.jobs.get(&group).is_some_and(|job|durable::epoch_key(&group,&job.members[0].token)==epoch&&(job.active()||job.settle_until.is_some_and(|until|crate::clock::now_ns()<until))){return Err(conflict());}
    let now=chrono::Utc::now().timestamp_millis();let hash=crate::auth::hash(&serde_json::to_string(&request).map_err(anyhow::Error::from)?);let gid=group.clone();let eid=epoch.clone();
    let applied=s.db.run(move|c|Ok(settle(c,&gid,&eid,&a.id,&request,&hash,now))).await??;
    if applied.changed {
        if let Some(job)=registry.jobs.get_mut(&group){if let Some(q)=&mut job.queue{for (index,delta) in &applied.increments{*q.credited.entry(*index).or_default()+=delta;}}
            if durable::epoch_key(&group,&job.members[0].token)==epoch {for member in &mut job.members{if applied.public["members"].as_array().is_some_and(|rows|rows.iter().any(|m|m["endpoint_id"]==member.zone.endpoint_id&&m["final_report_received"]==true)){member.final_report=true;}}}
            s.emit("sync_start",job.value());
        }else{s.emit("sync_start",applied.public);}
        s.emit("stats",json!({"group_id":group}));
    }Ok(Json(json!({"applied":applied.changed})))
}
fn maxima(tx:&Transaction<'_>,epoch:&str,index:usize)->anyhow::Result<(f64,f64,bool)> {
    let (seconds,position,ended)=tx.query_row("SELECT COALESCE(MAX(CASE WHEN sample_rate>0 THEN 1.0*frames/sample_rate ELSE 0 END),0),COALESCE(MAX(position),0),MIN(ended) AND COUNT(*)=(SELECT json_array_length(data,'$.members') FROM sync_output_epochs WHERE id=?1) FROM sync_output_sources WHERE epoch_id=?1 AND queue_index=?2",params![epoch,index as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get::<_,Option<bool>>(2)?.unwrap_or(false))))?;Ok((seconds,position,ended))
}
fn settle(c:&mut Connection,group:&str,epoch:&str,account:&str,request:&Request,hash:&str,now:i64)->ApiResult<Applied> {
    let tx=c.transaction()?;let r=&request.report;
    let header:Option<String>=tx.query_row("SELECT data FROM sync_output_epochs WHERE id=?1 AND group_id=?2",params![epoch,group],|row|row.get(0)).optional()?;let mut header:Value=serde_json::from_str(&header.ok_or_else(ApiError::missing)?).map_err(anyhow::Error::from)?;
    let mut rows={let mut q=tx.prepare("SELECT playback_id,queue_index,grant,frames,sample_rate,position,ended,account_id FROM sync_output_sources WHERE epoch_id=?1 AND endpoint_id=?2 ORDER BY queue_index")?;let raw=q.query_map(params![epoch,request.endpoint_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)? as usize,r.get::<_,String>(2)?,r.get::<_,i64>(3)? as u64,r.get::<_,u32>(4)?,r.get::<_,f64>(5)?,r.get::<_,bool>(6)?,r.get::<_,String>(7)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if raw.is_empty(){return Err(ApiError::missing());}if raw.iter().any(|row|row.7!=account){return Err(ApiError::forbidden());}
        raw.into_iter().map(|(playback,index,grant,frames,rate,position,ended,_)|Ok(Row{playback,index,grant:serde_json::from_str(&grant)?,frames,rate,position,ended})).collect::<anyhow::Result<Vec<_>>>()?};
    let prior:Option<(String,String)>=tx.query_row("SELECT request_id,payload_hash FROM sync_output_final_receipts WHERE (epoch_id=?1 AND endpoint_id=?2) OR request_id=?3",params![epoch,request.endpoint_id,request.request_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((id,old))=prior {if id!=request.request_id||old!=hash{return Err(conflict());}return Ok(Applied{increments:BTreeMap::new(),public:Value::Null,changed:false});}
    let member=header.get("output_members").filter(|v|v.is_array()).unwrap_or(&header["members"]).as_array().and_then(|members|members.iter().find(|m|m["endpoint_id"]==request.endpoint_id)).ok_or_else(bad)?;
    if r.kind!="start_state"||r.version!=1||r.measured_version!=2||!r.finished||!matches!(r.phase.as_str(),"cancelled"|"expired")||r.start["plan_id"]!=group
        ||rows.iter().any(|row|row.grant["start"]!=r.start||row.grant["start_playback_id"]!=r.core_playback_id||row.grant["zone_id"]!=r.zone_id)
        ||r.gate["expires_host_ns"]!=r.start["expires_host_ns"]||request.captured_at<0||request.captured_at>now.saturating_add(1000){return Err(bad());}
    let released=r.gate["released_host_ns"].as_u64().ok_or_else(bad)?;let unreleased=r.programme_frames_played==0&&released==0;
    let rate=r.sample_rate;
    if !(8000..=768000).contains(&rate)&&!(unreleased&&rate==0)||member["sample_rate"].as_u64().is_some_and(|old|old>0&&old!=rate as u64){return Err(bad());}
    if !unreleased {
        let deadline=member["deadline_host_ns"].as_u64().filter(|n|*n>0).ok_or_else(bad)?;let at=header["deadline_at_ms"].as_i64().ok_or_else(bad)?;
        if header["dispatched"]!=true&&!header["output_members"].is_array()||r.gate["phase"]!="released"||r.gate["deadline_host_ns"]!=deadline||!(deadline..=deadline.saturating_add(20_000_000)).contains(&released)||request.captured_host_ns<released{return Err(bad());}
        let seconds=r.programme_frames_played as f64/rate as f64;
        if seconds>86400.||seconds>request.captured_host_ns.saturating_sub(deadline) as f64/1e9+1.||seconds>(request.captured_at as i128-at as i128) as f64/1000.+1.{return Err(bad());}
    }else if rows.iter().any(|row|row.frames>0){return Err(bad());}
    let current=member["queue_index"].as_u64().ok_or_else(bad)? as usize;
    let mut changes=BTreeMap::new();let mut previous=None;let mut first=None;
    for (offset,source) in r.sequence_final.iter().enumerate() {
        let row=rows.iter_mut().find(|row|row.playback==source.playback_id).ok_or_else(bad)?;let base=row.grant["source_position_base"].as_f64().ok_or_else(bad)?;
        let epsilon=source.position.abs().max(row.position.abs()).max(1.)*f64::EPSILON*4.;
        if previous.is_some_and(|index|row.index!=index+1)||offset+1<r.sequence_final.len()&&!source.ended||row.grant["track"]["id"]!=source.track_id||!source.position.is_finite()||source.frames<row.frames||row.rate>0&&row.rate!=rate||source.position+epsilon<row.position||row.ended&&!source.ended{return Err(bad());}
        let seconds=source.frames as f64/rate.max(1) as f64;let delta=(source.frames-row.frames) as f64/rate.max(1) as f64;let tolerance=2./rate.max(1) as f64;
        if unreleased&&(source.frames>0||source.position!=base)||!unreleased&&(((source.position-base)-seconds).abs()>seconds*0.0005+tolerance||((source.position-row.position)-delta).abs()>delta*0.0005+tolerance)||source.frames==row.frames&&(source.position-row.position).abs()>epsilon{return Err(bad());}
        changes.insert(row.index,maxima(&tx,epoch,row.index)?.0);row.frames=source.frames;row.rate=rate;row.position=row.position.max(source.position);row.ended=source.ended;first.get_or_insert(row.index);previous=Some(row.index);
    }
    if !unreleased&&(first.is_none_or(|index|index>current)||previous.is_none_or(|index|index<current)){return Err(bad());}
    if rows.iter().try_fold(0u64,|sum,row|sum.checked_add(row.frames))!=Some(r.programme_frames_played){return Err(bad());}
    for row in &rows {if changes.contains_key(&row.index){tx.execute("UPDATE sync_output_sources SET frames=?4,sample_rate=?5,position=?6,ended=?7,updated_at=?8 WHERE epoch_id=?1 AND endpoint_id=?2 AND playback_id=?3",params![epoch,request.endpoint_id,row.playback,i64::try_from(row.frames).map_err(|_|bad())?,row.rate,row.position,row.ended,now])?;}}
    let mut increments=BTreeMap::new();
    for (index,before) in changes {
        let(after,position,ended)=maxima(&tx,epoch,index)?;let delta=(after-before).max(0.);let row=rows.iter().find(|row|row.index==index).unwrap();
        if after>0. {let mut zone=Zone::new(format!("sync:{group}:{index}"),"同步播放组".into(),String::new());zone.playback_id=Some(row.grant["logical_playback_id"].as_str().ok_or_else(bad)?.into());zone.track=Some(serde_json::from_value(row.grant["track"].clone()).map_err(anyhow::Error::from)?);zone.position=position;zone.status="stopped".into();
            crate::stats::group_output::late_tx(&tx,&zone,row.grant["listener_id"].as_str().ok_or_else(bad)?,row.grant["source_position_base"].as_f64().ok_or_else(bad)?,delta,request.captured_at,ended)?;}
        if delta>0.{increments.insert(index,delta);}
    }
    let delta: f64=increments.values().sum();let data:String=tx.query_row("SELECT result FROM sync_starts WHERE id=?1",[group],|r|r.get(0))?;let mut public:Value=serde_json::from_str(&data).map_err(anyhow::Error::from)?;public["listened_seconds"]=json!(public["listened_seconds"].as_f64().unwrap_or(0.)+delta);
    for m in header["members"].as_array_mut().ok_or_else(bad)? {if m["endpoint_id"]==request.endpoint_id {m["final_report_received"]=json!(true);}}
    if let Some(members)=header.get_mut("output_members").and_then(Value::as_array_mut){for m in members{if m["endpoint_id"]==request.endpoint_id{m["final_report_received"]=json!(true);}}}
    header["listened_seconds"]=json!(header["listened_seconds"].as_f64().unwrap_or(0.)+delta);
    // Only the matching historical generation may change a public final flag.
    for m in public["members"].as_array_mut().ok_or_else(bad)? {if m["endpoint_id"]==request.endpoint_id&&m["playback_id"]==r.core_playback_id {m["final_report_received"]=json!(true);}}
    tx.execute("UPDATE sync_output_epochs SET data=?2,updated_at=?3 WHERE id=?1",params![epoch,header.to_string(),now])?;
    tx.execute("UPDATE sync_starts SET result=?2,updated_at=?3 WHERE id=?1",params![group,public.to_string(),now])?;
    tx.execute("INSERT INTO sync_output_final_receipts(epoch_id,endpoint_id,request_id,payload_hash,created_at) VALUES(?1,?2,?3,?4,?5)",params![epoch,request.endpoint_id,request.request_id,hash,now])?;tx.commit()?;
    Ok(Applied{increments,public,changed:true})
}
