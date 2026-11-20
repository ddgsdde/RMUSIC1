//! Explicit experimental PCM targets. Automatic correction is a separate opt-in.
use super::*;

pub(super) fn supported(request:&super::Request,endpoint:&Value)->anyhow::Result<()> {
    if request.automatic_rate_adjustment&&(!request.pcm_rate_adjustment||endpoint["capabilities"]["group_rate_lease_v1"]!=true){anyhow::bail!("实验自动校正须全部输出支持本机变速租约");}
    if request.pcm_rate_adjustment&&(request.measured_version!=2||["pcm_rate_adjustment_v1","programme_clock_v2"].iter().any(|key|endpoint["capabilities"][*key]!=true)) {anyhow::bail!("实验变速须使用 v2 PCM 分组和支持变速／节目时钟 v2 的全部输出");}Ok(())
}
pub(super) fn decorate(job:&Job,value:&mut Value) {
    if !job.request.pcm_rate_adjustment{return;}
    value["rate_adjustment"]=json!({"enabled":true,"mode":"manual","automatic":false,"last_request_id":job.rate_request,"reset_on_rearm":true});
    if job.request.automatic_rate_adjustment {let rate=&mut value["rate_adjustment"];rate["mode"]=json!("experimental_auto");rate["automatic"]=json!(true);rate["controller"]=job.drift.value();if job.phase!="playing"{rate["controller"]["state"]=json!("inactive");}}
    for (row,member)in value["members"].as_array_mut().unwrap().iter_mut().zip(&job.members){row["requested_rate_ppm"]=json!(member.requested_rate_ppm);}
}
#[derive(Deserialize,Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {request_id:String,zone_id:String,playback_id:String,ppm:f64}
fn conflict(message:&str)->ApiError {ApiError(StatusCode::CONFLICT,"group_rate_conflict".into(),message.into())}
pub async fn command(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(mut request):Json<Request>)->ApiResult<(StatusCode,Json<Value>)> {
    a.require("control")?;
    request.request_id=uuid::Uuid::parse_str(&request.request_id).map_err(|_|ApiError::bad("调速请求须包含 UUID"))?.to_string();
    request.playback_id=uuid::Uuid::parse_str(&request.playback_id).map_err(|_|ApiError::bad("须指定当前原起播实例"))?.to_string();
    if request.zone_id.is_empty()||request.zone_id.len()>200||!request.ppm.is_finite()||request.ppm.abs()>500.{return Err(ApiError::bad("音区无效或变速超出 ±500 ppm"));}
    let encoded=json!({"action":"rate","request":request}).to_string();let rid=request.request_id.clone();
    let _zone=s.zone_lock.lock().await;let mut registry=s.sync_starts.lock().await;
    let previous=s.db.run(move|c|Ok(c.query_row("SELECT group_id,request FROM sync_controls WHERE id=?1",[rid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?)).await?;
    if let Some((group,body))=previous {
        if group!=id||body!=encoded{return Err(conflict("请求 ID 已用于另一项操作"));}
        if let Some(job)=registry.jobs.get(&id){return Ok((StatusCode::OK,Json(job.value())));}
        drop(registry);return Ok((StatusCode::OK,super::saved(s.clone(),id).await?));
    }
    let job=registry.jobs.get_mut(&id).ok_or_else(ApiError::missing)?;
    if job.request.automatic_rate_adjustment{return Err(conflict("此实验组由自动控制器管理，请重新创建手动组"));}
    if !job.request.pcm_rate_adjustment||job.request.measured_version!=2{return Err(conflict("此组没有显式启用实验变速"));}
    if job.phase!="playing"{return Err(conflict("请等待整组正在播放后再调速"));}
    let index=job.members.iter().position(|m|m.zone.id==request.zone_id).ok_or_else(ApiError::missing)?;let member=&job.members[index];
    if request.playback_id!=member.playback_id||member.phase!="playing"||!valid_gate(member,&json!({"gate":member.gate}),"released")||!s.bridges.read().await.get(&member.zone.endpoint_id).is_some_and(|tx|tx.same_channel(&member.tx)){return Err(conflict("输出实例、连接或起播状态已变化"));}
    let endpoint=s.endpoints.read().await.get(&member.zone.endpoint_id).cloned().unwrap_or(Value::Null);supported(&job.request,&endpoint).map_err(|e|conflict(&e.to_string()))?;
    let permit=member.tx.clone().try_reserve_owned().map_err(|_|conflict("输出控制队列繁忙"))?;
    let command=json!({"type":"group_rate","zone_id":member.zone.id,"playback_id":member.playback_id,"start":member.token,"group_queue":true,"measured_version":2,"ppm":request.ppm});
    let mut next=job.clone();next.members[index].requested_rate_ppm=request.ppm;next.rate_request=Some(request.request_id.clone());
    let stored=next.clone();let at=chrono::Utc::now().timestamp_millis();
    s.db.run(move|c|{let tx=c.transaction()?;tx.execute("INSERT INTO sync_controls(id,group_id,request,created_at) VALUES(?1,?2,?3,?4)",rusqlite::params![request.request_id,stored.request.request_id,encoded,at])?;tx.execute("UPDATE sync_starts SET result=?2,updated_at=?3 WHERE id=?1",rusqlite::params![stored.request.request_id,stored.value().to_string(),at])?;tx.commit()?;Ok(())}).await?;
    *job=next;permit.send(command);let value=job.value();s.emit("sync_start",value.clone());Ok((StatusCode::ACCEPTED,Json(value)))
}
