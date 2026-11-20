//! A missing local cursor has no heard-frame evidence. Authorize only a later explicit play.
use super::*;

#[derive(Serialize,Deserialize)]
struct Hold {endpoint:String,user:String,stamp:SystemSuspend,playback_id:Option<String>}
fn load(c:&Connection,zone:&str)->anyhow::Result<Option<Hold>> {
    c.query_row("SELECT data FROM sequence_output_holds WHERE zone_id=?1",[zone],|row|row.get::<_,String>(0)).optional()?.map(|data|Ok(serde_json::from_str(&data)?)).transpose()
}
fn store(c:&Connection,zone:&str,hold:&Hold)->anyhow::Result<()> {
    c.execute("INSERT INTO sequence_output_holds(zone_id,data) VALUES(?1,?2) ON CONFLICT(zone_id) DO UPDATE SET data=excluded.data",params![zone,serde_json::to_string(hold)?])?;Ok(())
}
pub(super) fn grant(tx:&Transaction<'_>,zone:&Zone)->anyhow::Result<()> {
    if let Some(mut hold)=load(tx,&zone.id)?.filter(|hold|hold.endpoint==zone.endpoint_id&&zone.status=="playing"&&zone.playback_id.is_some()) {
        hold.playback_id=zone.playback_id.clone();store(tx,&zone.id,&hold)?;
    }
    Ok(())
}
pub(super) async fn unavailable(s:&AppState,mut zone:Zone,user:&str,endpoint:&str,stamp:SystemSuspend,expected:Option<String>)->ApiResult<Json<Value>> {
    if !s.endpoints.read().await.get(endpoint).is_some_and(|e|e["capabilities"]["recovery_controls"]==true){return Err(ApiError::bad("系统挂起起播须由恢复控制端点确认"));}
    if expected.as_ref().is_some_and(|id|!uuid(id)){return Err(ApiError::bad("起播实例标识无效"));}
    if zone.endpoint_id!=endpoint||expected.as_ref().is_some_and(|id|zone.playback_id.as_ref()!=Some(id)) {
        return Ok(Json(json!({"applied":false,"accepted":false,"zone":zone})));
    }
    let id=zone.id.clone();let known=s.db.run(move|c|load(c,&id)).await?.filter(|hold|hold.endpoint==endpoint&&hold.user==user);
    if let Some(hold)=known.filter(|hold|!stamp.newer_than(Some(&hold.stamp))) {
        let accepted=stamp==hold.stamp&&zone.playback_id.is_some()&&hold.playback_id==zone.playback_id&&matches!(zone.status.as_str(),"playing"|"paused");
        return Ok(Json(json!({"applied":false,"accepted":accepted,"waiting_for_play":!accepted,"system_suspend_ack":if accepted{Some(stamp)}else{None},"zone":zone})));
    }
    crate::stats::finish(s,&zone,"disconnect").await?;
    zone.status="stopped".into();zone.playback_id=None;zone.updated_at=chrono::Utc::now().timestamp_millis();
    let saved=zone.clone();let hold=Hold {endpoint:endpoint.into(),user:user.into(),stamp,playback_id:None};
    s.db.run(move|c|{let tx=c.transaction()?;
        tx.execute("UPDATE zones SET data=?2 WHERE id=?1",params![saved.id,serde_json::to_string(&saved)?])?;
        tx.execute("DELETE FROM sequence_recovery WHERE zone_id=?1",[&saved.id])?;
        store(&tx,&saved.id,&hold)?;tx.commit()?;Ok(())
    }).await?;
    super::super::invalidate_zone(s,&zone.id,Some(&zone)).await;
    s.zones.write().await.insert(zone.id.clone(),zone.clone());s.emit("zones_changed",json!({}));
    Ok(Json(json!({"applied":true,"accepted":false,"waiting_for_play":true,"zone":zone})))
}
