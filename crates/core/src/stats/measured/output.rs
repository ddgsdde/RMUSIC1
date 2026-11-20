//! Integer output coverage credits elapsed audio once; source coverage remains available for recovery.
use super::*;
pub(super) fn seconds(segment:&Segment,version:u32)->Option<f64> {
    match (version,&segment.output) {
        (1,None)=>(segment.end>segment.start).then_some(segment.end-segment.start),
        (2,Some(output))=>{
            if !(8000..=768000).contains(&output.sample_rate)||output.end<=output.start||output.end>i64::MAX as u64||output.end as f64/output.sample_rate as f64>604800.*1.001+1. {return None;}
            let seconds=(output.end-output.start) as f64/output.sample_rate as f64;
            (((segment.end-segment.start)-seconds).abs()<=seconds*0.0005+2./output.sample_rate as f64).then_some(seconds)
        },_=>None,
    }
}
pub(super) fn compatible(tx:&Transaction<'_>,m:&Measured)->anyhow::Result<bool> {
    if m.version==1 {return Ok(true);}
    let stored=tx.query_row("SELECT sample_rate FROM listen_output_intervals WHERE playback_id=?1 LIMIT 1",[&m.playback_id],|r|r.get::<_,u32>(0)).optional()?;
    Ok(stored.is_none_or(|rate|m.segments.iter().all(|segment|segment.output.as_ref().is_some_and(|output|output.sample_rate==rate))))
}
fn cover_frames(tx:&Transaction<'_>,playback:&str,output:&OutputRange)->anyhow::Result<u64> {
    let ranges=tx.prepare("SELECT start,end FROM listen_output_intervals WHERE playback_id=?1 ORDER BY start")?.query_map([playback],|r|Ok((r.get::<_,i64>(0)? as u64,r.get::<_,i64>(1)? as u64)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut cursor=output.start;let mut uncovered=0;
    for &(start,end) in &ranges {if end<=cursor{continue;}if start>=output.end{break;}uncovered+=start.min(output.end).saturating_sub(cursor);cursor=cursor.max(end).min(output.end);}
    uncovered+=output.end.saturating_sub(cursor);
    let mut merged=ranges;merged.push((output.start,output.end));merged.sort_unstable();let mut normalized:Vec<(u64,u64)>=Vec::new();
    for range in merged {if let Some(last)=normalized.last_mut().filter(|last|range.0<=last.1){last.1=last.1.max(range.1);}else{normalized.push(range);}}
    tx.execute("DELETE FROM listen_output_intervals WHERE playback_id=?1",[playback])?;
    for (start,end) in normalized {tx.execute("INSERT INTO listen_output_intervals(playback_id,start,end,sample_rate) VALUES(?1,?2,?3,?4)",params![playback,start as i64,end as i64,output.sample_rate])?;}
    Ok(uncovered)
}
pub(super) fn credit(tx:&Transaction<'_>,m:&Measured,segment:&Segment)->anyhow::Result<()> {
    let output=segment.output.as_ref().ok_or_else(||anyhow::anyhow!("missing measured output frames"))?;
    cover(tx,&m.playback_id,segment.start,segment.end)?;
    let frames=cover_frames(tx,&m.playback_id,output)?;let group=group(tx,&m.listen_session_id)?.ok_or_else(||anyhow::anyhow!("missing output listen group"))?;
    credit_amount(tx,&m.listen_session_id,&group,frames as f64/output.sample_rate as f64,segment.last_at)?;Ok(())
}
