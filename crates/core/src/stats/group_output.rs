//! Logical cohort time: max confirmed per-source output duration, with independent source position.
use super::*;

pub(crate) fn record_tx(tx:&Transaction<'_>,z:&Zone,owner:&str,baseline:f64,seconds:f64,now:i64,reason:Option<&str>)->anyhow::Result<()> {
    if !baseline.is_finite()||baseline<0.||!z.position.is_finite()||z.position<baseline||!seconds.is_finite()||!(0. ..=86400.).contains(&seconds){anyhow::bail!("invalid logical group output");}
    let playback=z.playback_id.as_deref().ok_or_else(||anyhow::anyhow!("group output requires a source identity"))?;
    if measured::for_playback(tx,Some(playback))?.is_none() {
        let mut initial=z.clone();initial.position=baseline;initial.status="playing".into();
        progress_tx(tx,&initial,now-(seconds*1000.).round() as i64,runtime_id(),false,Some(0.))?;
        measured::issue_current_timed(tx,owner,&initial,2)?;
    }
    let version=tx.query_row("SELECT timing_version FROM listen_claims WHERE playback_id=?1",[playback],|r|r.get::<_,u32>(0))?;
    if version!=2{anyhow::bail!("logical group output timing mismatch");}
    let(id,_,_)=measured::for_playback(tx,Some(playback))?.unwrap();
    let mut session=tx.query_row("SELECT p.id,p.track_id,p.title,p.artist,p.cover_url,p.duration,p.last_report_at,p.last_position,p.last_status,p.runtime_id,p.play_seconds,p.counted FROM play_sessions p JOIN listen_session_sources src ON src.session_id=p.id WHERE p.id=?1 AND p.zone_id=?2 AND src.playback_id=?3",params![id,z.id,playback],|r|Ok(Session{id:r.get(0)?,track_id:r.get(1)?,title:r.get(2)?,artist:r.get(3)?,cover_url:r.get(4)?,duration:r.get(5)?,last_report_at:r.get(6)?,last_position:r.get(7)?,last_status:r.get(8)?,runtime:r.get(9)?,seconds:r.get(10)?,counted:r.get(11)?}))?;
    if session.runtime!=runtime_id()||z.track.as_ref().is_none_or(|track|track.id!=session.track_id)||z.position<session.last_position{anyhow::bail!("logical group source moved backwards or changed runtime");}
    tx.execute("INSERT OR IGNORE INTO group_output_totals(playback_id,seconds) VALUES(?1,0)",[playback])?;
    let previous=tx.query_row("SELECT seconds FROM group_output_totals WHERE playback_id=?1",[playback],|r|r.get::<_,f64>(0))?;
    let total=seconds.max(previous);let at=now.max(session.last_report_at);
    measured::credit_group_seconds(tx,playback,&id,session.last_position,z.position,total-previous,at)?;
    let(_,heard,counted)=measured::for_playback(tx,Some(playback))?.unwrap();
    tx.execute("UPDATE group_output_totals SET seconds=?2 WHERE playback_id=?1",params![playback,total])?;
    tx.execute("UPDATE play_sessions SET play_seconds=?2,counted=?3,last_report_at=?4,last_position=?5,last_status=CASE WHEN finished_at IS NULL THEN ?6 ELSE 'stopped' END WHERE id=?1",params![id,heard,counted,at,z.position,z.status])?;
    session.seconds=heard;session.counted=counted;session.last_position=z.position;
    if let Some(reason)=reason{close(tx,&session,at,reason)?;if reason=="ended"{measured::mark_ended(tx,playback,Some(z.position),at)?;}}
    Ok(())
}

/// Add only the verified increase in an old epoch's cohort maximum. A later source may
/// share this listen group: update its accumulated statistics without rebinding its cursor.
pub(crate) fn late_tx(tx:&Transaction<'_>,z:&Zone,owner:&str,baseline:f64,delta:f64,at:i64,ended:bool)->anyhow::Result<()> {
    let playback=z.playback_id.as_deref().ok_or_else(||anyhow::anyhow!("missing historical group source"))?;let track=z.track.as_ref().unwrap();
    if measured::for_playback(tx,Some(playback))?.is_none(){measured::issue_next_timed(tx,owner,playback,track,2)?;tx.execute("UPDATE listen_claims SET baseline=?2 WHERE playback_id=?1",params![playback,baseline])?;}
    let(id,_,_)=measured::for_playback(tx,Some(playback))?.unwrap();
    tx.execute("INSERT OR IGNORE INTO play_sessions(id,zone_id,track_id,title,artist,cover_url,duration,started_at,last_report_at,last_position,last_status,runtime_id,finished_at,finish_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'stopped','group-final',?9,'disconnect')",params![id,z.id,track.id,track.title,track.artist,track.cover_url,track.duration,at-(delta*1000.).round() as i64,at,baseline])?;
    tx.execute("INSERT OR IGNORE INTO listen_session_sources(session_id,playback_id) VALUES(?1,?2)",params![id,playback])?;
    measured::credit_group_seconds(tx,playback,&id,baseline,z.position,delta,at)?;let(_,heard,counted)=measured::for_playback(tx,Some(playback))?.unwrap();
    tx.execute("INSERT INTO group_output_totals(playback_id,seconds) VALUES(?1,?2) ON CONFLICT(playback_id) DO UPDATE SET seconds=seconds+excluded.seconds",params![playback,delta])?;
    tx.execute("UPDATE play_sessions SET play_seconds=?2,counted=?3,last_report_at=MAX(last_report_at,?4),last_position=CASE WHEN EXISTS(SELECT 1 FROM listen_session_sources WHERE session_id=?1 AND playback_id=?5) THEN MAX(last_position,?6) ELSE last_position END WHERE id=?1",params![id,heard,counted,at,playback,z.position])?;
    let current=tx.query_row("SELECT playback_id FROM listen_session_sources WHERE session_id=?1",[&id],|r|r.get::<_,String>(0))?;
    if current==playback {
        tx.execute("UPDATE play_sessions SET finished_at=COALESCE(finished_at,?2),last_status='stopped' WHERE id=?1",params![id,at])?;
        if ended{measured::mark_ended(tx,playback,Some(z.position),at)?;}
    }Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    fn record(c:&mut Connection,z:&Zone,base:f64,seconds:f64,at:i64,reason:Option<&str>)->anyhow::Result<()> {let tx=c.transaction()?;record_tx(&tx,z,"owner",base,seconds,at,reason)?;tx.commit()?;Ok(())}
    fn total(c:&Connection)->(f64,i64){c.query_row("SELECT SUM(play_seconds),SUM(play_count) FROM track_stats",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()}
    #[test]
    fn group_output_utc_seek_closed_tail_private_claims_and_transaction_rollback() {
        let mut c=Connection::open_in_memory().unwrap();c.execute_batch(include_str!("../schema.sql")).unwrap();
        let at=chrono::DateTime::parse_from_rfc3339("2026-10-16T23:59:59.500Z").unwrap().timestamp_millis();
        let mut z=Zone::new("sync:qa:0".into(),"QA".into(),String::new());z.status="playing".into();z.renew_playback();
        z.track=Some(serde_json::from_value(json!({"id":"track","title":"QA","artist":"QA","album":"QA","album_artist":"QA","duration":10.,"track_number":1,"disc_number":1,"source":"local","format":"flac","sample_rate":48000,"bit_depth":24,"channels":2,"bitrate":0,"cover_url":null,"recording_id":"recording","available":true,"created_at":0})).unwrap());
        record(&mut c,&z,0.,0.,at,None).unwrap();z.position=6.003;record(&mut c,&z,0.,6.,at+6000,None).unwrap();record(&mut c,&z,0.,6.,at+7000,None).unwrap();assert_eq!(total(&c),(6.,1));
        let old=z.playback_id.clone().unwrap();let(group,_,_)=measured::for_playback(&c,Some(&old)).unwrap().unwrap();
        let snapshot:offline::Snapshot=serde_json::from_value(json!({"id":uuid::Uuid::new_v4().to_string(),"version":1,"track_id":"track","title":"QA","artist":"QA","duration":10.,"started_at":at,"updated_at":at+6000,"position":6.003,"reason":"active","days":{(day(at)):{"seconds":0.5,"last_at":at+499},(day(at+6000)):{"seconds":5.5,"last_at":at+6000}},"measured":{"playback_id":old,"listen_session_id":group,"version":2,"segments":[{"start":0.,"end":0.50025,"last_at":at+499,"output":{"start":0,"end":24000,"sample_rate":48000}},{"start":0.50025,"end":6.003,"last_at":at+6000,"output":{"start":24000,"end":288000,"sample_rate":48000}}]}})).unwrap();
        let tx=c.transaction().unwrap();assert!(measured::import(&tx,"owner",&snapshot).unwrap().is_none(),"private cohort output totals cannot accept individual durable receipts");tx.commit().unwrap();assert_eq!(total(&c),(6.,1));
        c.execute_batch("CREATE TRIGGER reject_group_output BEFORE UPDATE ON group_output_totals BEGIN SELECT RAISE(ABORT,'group output failure');END;").unwrap();z.position=8.004;assert!(record(&mut c,&z,0.,8.,at+8000,Some("stop")).is_err());assert_eq!(total(&c),(6.,1));assert_eq!(c.query_row("SELECT MAX(end) FROM listen_intervals",[],|r|r.get::<_,f64>(0)).unwrap(),6.003);
        c.execute_batch("DROP TRIGGER reject_group_output").unwrap();record(&mut c,&z,0.,8.,at+8000,Some("stop")).unwrap();z.position=8.50425;record(&mut c,&z,0.,8.5,at+9000,Some("stop")).unwrap();assert_eq!(total(&c),(8.5,1));
        z.renew_playback();z.position=1.5;z.updated_at=at+10000;let tx=c.transaction().unwrap();restore_sequence_tx(&tx,"owner",&old,Some(&old),&z).unwrap();tx.commit().unwrap();
        let version=c.query_row("SELECT timing_version FROM listen_claims WHERE playback_id=?1",[&z.playback_id],|r|r.get::<_,u32>(0)).unwrap();assert_eq!(version,2);
        z.position=3.501;record(&mut c,&z,1.5,2.,at+12000,Some("ended")).unwrap();record(&mut c,&z,1.5,2.,at+13000,Some("ended")).unwrap();assert_eq!(total(&c),(10.5,1));
        let days=c.prepare("SELECT day,play_seconds FROM daily_stats ORDER BY day").unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,f64>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();assert_eq!(days,vec![("2026-10-16".into(),0.5),("2026-10-17".into(),10.)]);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM play_sessions",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
}
