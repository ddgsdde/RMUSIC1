//! Explicit v2 online receipts share integer coverage with durable history, not source-time deltas.
use super::*;
use measured::{OutputRange,Segment};

pub(crate) async fn version(s:&AppState,z:&Zone)->anyhow::Result<u32> {
    let id=z.playback_id.clone();s.db.run(move|c|Ok(c.query_row("SELECT timing_version FROM listen_claims WHERE playback_id=?1",[id],|r|r.get(0)).optional()?.unwrap_or(1))).await
}

pub(crate) fn initialize(tx:&Transaction<'_>,playback:&str,baseline:f64,at:i64,base:u64,rate:u32)->anyhow::Result<()> {
    tx.execute("INSERT OR IGNORE INTO listen_output_cursors(playback_id,frames,sample_rate,position,origin_at,base_frames,runtime_id) VALUES(?1,0,?6,?2,?3,?5,?4)",params![playback,baseline,at,runtime_id(),base as i64,rate])?;Ok(())
}
struct Bound {playback:String,group:String,baseline:f64,frames:u64,position:f64,origin:i64,base:u64,has_cursor:bool}
fn bound(tx:&Transaction<'_>,z:&Zone,frames:u64,rate:u32)->anyhow::Result<Option<Bound>> {
    if !(8000..=768000).contains(&rate)||frames as f64/rate as f64>86400.||!z.position.is_finite(){return Ok(None);}
    let Some(playback)=z.playback_id.as_deref()else{return Ok(None)};
    let claim=tx.query_row("SELECT p.group_id,p.baseline FROM listen_claims p JOIN listen_groups g ON g.id=p.group_id WHERE p.playback_id=?1 AND p.timing_version=2 AND g.track_id=?2",params![playback,z.track.as_ref().map(|track|&track.id)],|r|Ok((r.get::<_,String>(0)?,r.get::<_,f64>(1)?))).optional()?;
    let Some((group,baseline))=claim else{return Ok(None)};
    let stored=tx.query_row("SELECT frames,sample_rate,position,origin_at,runtime_id,base_frames FROM listen_output_cursors WHERE playback_id=?1",[playback],|r|Ok((r.get::<_,i64>(0)? as u64,r.get::<_,u32>(1)?,r.get::<_,f64>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)? as u64))).optional()?;
    let has_cursor=stored.is_some();let (previous,old_rate,position,origin,runtime,base)=stored.unwrap_or((0,0,baseline,0,runtime_id().into(),0));
    let existing_rate=tx.query_row("SELECT sample_rate FROM listen_output_intervals WHERE playback_id=?1 LIMIT 1",[playback],|r|r.get::<_,u32>(0)).optional()?;
    if runtime!=runtime_id()||(old_rate!=0&&old_rate!=rate)||existing_rate.is_some_and(|r|r!=rate)||frames<previous||z.position<position {return Ok(None);}
    if base.checked_add(frames).is_none_or(|total|total>i64::MAX as u64){return Ok(None);}
    let total=frames as f64/rate as f64;let delta=(frames-previous) as f64/rate as f64;
    if ((z.position-baseline)-total).abs()>total*0.0005+2./rate as f64||((z.position-position)-delta).abs()>delta*0.0005+2./rate as f64||(frames==previous&&z.position!=position){return Ok(None);}
    Ok(Some(Bound{playback:playback.into(),group,baseline,frames:previous,position,origin,base,has_cursor}))
}
fn segments(from:u64,to:u64,rate:u32,source_from:f64,source_to:f64,at:i64)->anyhow::Result<Vec<Segment>> {
    let count=to-from;if count==0{return Ok(Vec::new());}
    let start=at-((count as f64/rate as f64)*1000.).round() as i64;let mut cursor=start;let mut frame=from;let mut source=source_from;let mut segments=Vec::new();
    loop {
        let date=chrono::DateTime::from_timestamp_millis(cursor).ok_or_else(||anyhow::anyhow!("invalid output date"))?.date_naive();
        let midnight=date.succ_opt().ok_or_else(||anyhow::anyhow!("output date overflow"))?.and_hms_opt(0,0,0).unwrap().and_utc().timestamp_millis();
        let stop=at.min(midnight);let end=if stop==at{to}else{from+(count as u128*(stop-start) as u128/(at-start) as u128) as u64};
        let source_end=if stop==at{source_to}else{source_from+(source_to-source_from)*(end-from) as f64/count as f64};
        if end>frame {segments.push(Segment{start:source,end:source_end,last_at:if stop==midnight{stop-1}else{stop},output:Some(OutputRange{start:frame,end,sample_rate:rate})});}
        if stop==at{break;}cursor=stop;frame=end;source=source_end;
    }Ok(segments)
}
fn apply(tx:&Transaction<'_>,z:&Zone,frames:u64,rate:u32,at:i64,finish:bool)->anyhow::Result<bool> {
    let Some(b)=bound(tx,z,frames,rate)?else{return Ok(false)};
    let Some(session)=active(tx,&z.id)?else{return Ok(false)};
    if !b.has_cursor||session.id!=b.group||session.runtime!=runtime_id()||(b.base+frames) as f64/rate as f64>(at-b.origin) as f64/1000.+1.{return Ok(false);}
    let parts=segments(b.frames,frames,rate,b.position,z.position,at)?;
    for segment in parts {if !measured::credit_output(tx,&b.playback,&b.group,segment)?{anyhow::bail!("output segment failed validation");}}
    let (_,seconds,counted)=measured::for_playback(tx,Some(&b.playback))?.unwrap();
    tx.execute("UPDATE play_sessions SET last_report_at=MAX(last_report_at,?2),last_position=?3,last_status=?4,runtime_id=?5,play_seconds=?6,counted=?7 WHERE id=?1",params![session.id,at,z.position,z.status,runtime_id(),seconds,counted])?;
    tx.execute("UPDATE listen_output_cursors SET frames=?2,sample_rate=?3,position=?4 WHERE playback_id=?1",params![b.playback,frames as i64,rate,z.position])?;
    if finish {if let Some(session)=active(tx,&z.id)?{close(tx,&session,at.max(session.last_report_at),"ended")?;}measured::mark_ended(tx,&b.playback,Some(z.position),at.max(session.last_report_at))?;}
    Ok(true)
}
pub(crate) async fn progress(s:&AppState,z:&Zone,frames:u64,rate:u32,finish:bool)->anyhow::Result<bool> {
    let zone=z.clone();let now=Utc::now().timestamp_millis();
    let applied=s.db.run(move|c|{let tx=c.transaction()?;let accepted=apply(&tx,&zone,frames,rate,now,finish)?;if accepted{tx.commit()?;}Ok(accepted)}).await?;
    if applied{s.emit("stats",json!({"zone_id":z.id}));}Ok(applied)
}
pub(crate) fn sequence_tx(tx:&Transaction<'_>,ended:&[(&Zone,u64)],current:&Zone,current_frames:u64,rate:u32,now:i64)->anyhow::Result<bool> {
    let Some(&(first,first_frames))=ended.first()else{return Ok(false)};
    let Some(first_bound)=bound(tx,first,first_frames,rate)?else{return Ok(false)};
    let Some(session)=active(tx,&first.id)?else{return Ok(false)};
    if !first_bound.has_cursor||session.id!=first_bound.group||session.runtime!=runtime_id(){return Ok(false);}
    let Some(tail_frames)=ended.iter().skip(1).try_fold(current_frames,|sum,(_,frames)|sum.checked_add(*frames))else{return Ok(false)};
    let Some(total)=first_bound.base.checked_add(first_frames).and_then(|sum|sum.checked_add(tail_frames))else{return Ok(false)};
    if total>i64::MAX as u64||total as f64/rate as f64>(now-first_bound.origin) as f64/1000.+1.{return Ok(false);}
    let tail=tail_frames as f64/rate as f64;
    // Validate all future identities and rates before closing the current source.
    for &(zone,frames) in ended.iter().skip(1).chain(std::iter::once(&(current,current_frames))) {
        let Some(b)=bound(tx,zone,frames,rate)?else{return Ok(false)};if b.has_cursor{return Ok(false);}
    }
    // Carry one admission origin plus integer session-frame bases through every song.
    // Arrival delays do not restart the clock allowance, and tiny songs lose no clock fractions.
    let mut base=first_bound.base+first_frames;
    let mut at=now-(tail*1000.).round() as i64;
    if !apply(tx,first,first_frames,rate,at,true)?{return Ok(false);}
    for &(zone,frames) in ended.iter().skip(1) {
        let Some(b)=bound(tx,zone,frames,rate)?else{anyhow::bail!("validated output claim disappeared");};
        let mut baseline=zone.clone();baseline.position=b.baseline;baseline.status="playing".into();progress_tx(tx,&baseline,at,runtime_id(),true,Some(0.))?;initialize(tx,&b.playback,b.baseline,first_bound.origin,base,rate)?;
        at+=(frames as f64/rate as f64*1000.).round() as i64;if !apply(tx,zone,frames,rate,at,true)?{anyhow::bail!("validated output boundary failed");}base+=frames;
    }
    let b=bound(tx,current,current_frames,rate)?.unwrap();let mut baseline=current.clone();baseline.position=b.baseline;baseline.status="playing".into();progress_tx(tx,&baseline,at,runtime_id(),true,Some(0.))?;initialize(tx,&b.playback,b.baseline,first_bound.origin,base,rate)?;
    if !apply(tx,current,current_frames,rate,now.max(at),false)?{anyhow::bail!("validated output current source failed");}Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_online_clock_allowance_is_not_refilled_by_each_progress_poll() {
        let mut c=Connection::open_in_memory().unwrap();c.execute_batch(include_str!("../schema.sql")).unwrap();let at=chrono::DateTime::parse_from_rfc3339("2026-10-17T12:00:00Z").unwrap().timestamp_millis();
        let mut z=Zone::new("zone".into(),"zone".into(),"endpoint".into());z.status="playing".into();z.renew_playback();
        z.track=Some(serde_json::from_value(json!({"id":"track","title":"PCM","artist":"QA","album":"QA","album_artist":"QA","duration":30.,"track_number":1,"disc_number":1,"source":"local","format":"flac","sample_rate":48000,"bit_depth":24,"channels":2,"bitrate":0,"cover_url":null,"recording_id":"recording","available":true,"created_at":0})).unwrap());
        super::super::progress(&mut c,&z,at,runtime_id(),true).unwrap();let tx=c.transaction().unwrap();measured::issue_current_timed(&tx,"owner",&z,2).unwrap();tx.execute("UPDATE listen_output_cursors SET origin_at=?1",[at]).unwrap();tx.commit().unwrap();
        z.position=0.90045;let tx=c.transaction().unwrap();assert!(apply(&tx,&z,43200,48000,at,false).unwrap());tx.commit().unwrap();
        z.position=1.8009;let tx=c.transaction().unwrap();assert!(!apply(&tx,&z,86400,48000,at,false).unwrap(),"one second of clock tolerance is a session budget, not a fresh allowance for every message");
    }
    #[test]
    fn output_online_midnight_partition_preserves_frames_and_source_endpoints() {
        let at=chrono::DateTime::parse_from_rfc3339("2026-10-17T00:00:01.500Z").unwrap().timestamp_millis();let parts=segments(0,96000,48000,5.,7.001,at).unwrap();assert_eq!(parts.len(),2);
        assert_eq!(parts[0].output.as_ref().unwrap().end,24000);assert_eq!(parts[1].output.as_ref().unwrap().start,24000);assert_eq!(parts.iter().map(|segment|{let out=segment.output.as_ref().unwrap();out.end-out.start}).sum::<u64>(),96000);assert_eq!(day(parts[0].last_at),"2026-10-16");assert_eq!(day(parts[1].last_at),"2026-10-17");assert_eq!(parts[0].end,parts[1].start);assert_eq!(parts[1].end,7.001);
    }
}
