//! Online device frames and durable client history share issued source intervals and counters.
use super::*;
use crate::models::Track;
use serde::Serialize;
mod output;
fn legacy_timing()->u32{1}
fn is_legacy_timing(version:&u32)->bool{*version==1}

#[derive(Clone,Debug,Deserialize,Serialize,PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub start:f64,pub end:f64,pub last_at:i64,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub output:Option<OutputRange>,
}
#[derive(Clone,Debug,Deserialize,Serialize,PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OutputRange {pub start:u64,pub end:u64,pub sample_rate:u32}
#[derive(Clone,Debug,Deserialize,Serialize,PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Measured {
    pub playback_id:String,pub listen_session_id:String,pub segments:Vec<Segment>,
    #[serde(default="legacy_timing",skip_serializing_if="is_legacy_timing")]
    pub version:u32,
}
struct Group {session:Session,started_at:i64,skipped:bool,skipped_at:Option<i64>,ended:bool}
fn group(c:&Connection,id:&str)->anyhow::Result<Option<Group>> {
    Ok(c.query_row("SELECT track_id,title,artist,cover_url,duration,started_at,updated_at,seconds,counted,skipped,skipped_at,ended FROM listen_groups WHERE id=?1",[id],|r|Ok(Group {
        session:Session {id:id.into(),track_id:r.get(0)?,title:r.get(1)?,artist:r.get(2)?,cover_url:r.get(3)?,duration:r.get(4)?,last_report_at:r.get(6)?,last_position:0.,last_status:"stopped".into(),runtime:"measured".into(),seconds:r.get(7)?,counted:r.get(8)?},
        started_at:r.get(5)?,skipped:r.get(9)?,skipped_at:r.get(10)?,ended:r.get(11)?,
    })).optional()?)
}
fn issue_timed(tx:&Transaction<'_>,owner:&str,playback:&str,id:&str,track:&Track,baseline:f64,started:i64,seconds:f64,counted:bool,version:u32)->anyhow::Result<String> {
    if ![1,2].contains(&version){anyhow::bail!("unsupported listening timing version");}
    tx.execute("INSERT OR IGNORE INTO listen_groups(id,track_id,title,artist,cover_url,duration,started_at,updated_at,seconds,counted) VALUES(?1,?2,?3,?4,?5,?6,?7,?7,?8,?9)",params![id,track.id,track.title,track.artist,track.cover_url,track.duration,started,seconds,counted])?;
    tx.execute("INSERT OR IGNORE INTO listen_claims(playback_id,group_id,baseline,timing_version) VALUES(?1,?2,?3,?4)",params![playback,id,baseline,version])?;
    let (group,stored):(String,u32)=tx.query_row("SELECT group_id,timing_version FROM listen_claims WHERE playback_id=?1",[playback],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if stored!=version {anyhow::bail!("listening timing version is already bound to this playback");}
    tx.execute("INSERT OR IGNORE INTO listen_claim_users(playback_id,user_id) VALUES(?1,?2)",params![playback,owner])?;
    crate::accounts::listening::own(tx,&group,owner)?;
    crate::accounts::listening::remember(tx,owner,track)?;
    Ok(group)
}
pub(crate) fn issue_current(tx:&Transaction<'_>,owner:&str,z:&Zone)->anyhow::Result<String> {issue_current_timed(tx,owner,z,1)}
pub(crate) fn issue_current_timed(tx:&Transaction<'_>,owner:&str,z:&Zone,version:u32)->anyhow::Result<String> {
    if active(tx,&z.id)?.is_none_or(|s|z.track.as_ref().is_none_or(|t|t.id!=s.track_id)) {
        // An explicit paused seek after transport suspension has a fresh Core identity,
        // but no live session yet. Preparing it establishes only the actual-position baseline.
        let now=chrono::Utc::now().timestamp_millis();let mut baseline=z.clone();baseline.status="playing".into();
        progress_tx(tx,&baseline,now,runtime_id(),true,Some(0.))?;
        progress_tx(tx,z,now,runtime_id(),true,Some(0.))?;
    }
    let current=active(tx,&z.id)?.filter(|s|z.track.as_ref().is_some_and(|t|t.id==s.track_id)).ok_or_else(||anyhow::anyhow!("missing active listen session"))?;
    let started=tx.query_row("SELECT started_at FROM play_sessions WHERE id=?1",[&current.id],|r|r.get(0))?;
    bind_source(tx,&current.id,z.playback_id.as_deref())?;
    let playback=z.playback_id.as_deref().ok_or_else(||anyhow::anyhow!("missing playback identity"))?;
    let group=issue_timed(tx,owner,playback,&current.id,z.track.as_ref().unwrap(),z.position,started,current.seconds,current.counted,version)?;
    if version==2 {crate::stats::output::initialize(tx,playback,z.position,chrono::Utc::now().timestamp_millis(),0,0)?;}
    crate::accounts::listening::own(tx,&group,owner)?;
    Ok(group)
}
pub(crate) fn issue_next_timed(tx:&Transaction<'_>,owner:&str,playback:&str,track:&Track,version:u32)->anyhow::Result<()> {
    issue_timed(tx,owner,playback,playback,track,0.,0,0.,false,version)?;Ok(())
}
/// Source-local EOF survives a cursor lag; group EOF alone is ambiguous after a seek.
pub(crate) fn mark_ended(tx:&Transaction<'_>,playback:&str,position:Option<f64>,at:i64)->anyhow::Result<()> {
    let Some((group,_,_))=for_playback(tx,Some(playback))? else{return Ok(())};
    let position=position.unwrap_or(tx.query_row("SELECT COALESCE(MAX(end),0) FROM listen_intervals WHERE playback_id=?1",[playback],|r|r.get(0))?);
    tx.execute("INSERT INTO listen_source_ends(playback_id,position,updated_at) VALUES(?1,?2,?3) ON CONFLICT(playback_id) DO UPDATE SET position=MAX(position,excluded.position),updated_at=MIN(updated_at,excluded.updated_at)",params![playback,position,at])?;
    // A late device EOF can correct a disconnected historical session, but never a newer seek.
    tx.execute("UPDATE play_sessions SET finish_reason='ended',finished_at=MIN(finished_at,?3),last_position=MAX(last_position,?4) WHERE id=?1 AND finished_at IS NOT NULL AND EXISTS(SELECT 1 FROM listen_session_sources WHERE session_id=?1 AND playback_id=?2)",params![group,playback,at,position])?;
    let g=self::group(tx,&group)?.unwrap();close(tx,&g.session,at,"ended")?;Ok(())
}
pub(super) fn bind_source(tx:&Transaction<'_>,id:&str,playback:Option<&str>)->anyhow::Result<()> {
    if let Some(playback)=playback {tx.execute("INSERT INTO listen_session_sources(session_id,playback_id) VALUES(?1,?2) ON CONFLICT(session_id) DO UPDATE SET playback_id=excluded.playback_id",params![id,playback])?;}Ok(())
}
pub(crate) fn recovery_position(c:&Connection,owner:&str,playback:&str,track:&str)->anyhow::Result<Option<(f64,f64,Option<f64>)>> {
    Ok(c.query_row("SELECT p.baseline,MAX(p.baseline,COALESCE((SELECT MAX(end) FROM listen_intervals WHERE playback_id=p.playback_id),0)),e.position FROM listen_claims p JOIN listen_claim_users u ON u.playback_id=p.playback_id JOIN listen_groups g ON g.id=p.group_id LEFT JOIN listen_source_ends e ON e.playback_id=p.playback_id WHERE p.playback_id=?1 AND u.user_id=?2 AND g.track_id=?3",params![playback,owner,track],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?)
}
pub(super) fn restore(tx:&Transaction<'_>,owner:&str,old:&str,z:&Zone,now:i64)->anyhow::Result<()> {
    let (id,_,_)=for_playback(tx,Some(old))?.ok_or_else(||anyhow::anyhow!("missing recovery claim"))?;
    let g=group(tx,&id)?.ok_or_else(||anyhow::anyhow!("missing recovery listen group"))?;
    let track=z.track.as_ref().unwrap();
    let version=tx.query_row("SELECT timing_version FROM listen_claims WHERE playback_id=?1",[old],|r|r.get::<_,u32>(0))?;
    issue_timed(tx,owner,z.playback_id.as_deref().unwrap(),&id,track,z.position,g.started_at,g.session.seconds,g.session.counted,version)?;
    tx.execute("INSERT INTO play_sessions(id,zone_id,track_id,title,artist,cover_url,duration,started_at,last_report_at,last_position,last_status,runtime_id,play_seconds,counted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) ON CONFLICT(id) DO UPDATE SET zone_id=excluded.zone_id,last_report_at=excluded.last_report_at,last_position=excluded.last_position,last_status=excluded.last_status,runtime_id=excluded.runtime_id,finished_at=NULL,finish_reason=NULL,play_seconds=excluded.play_seconds,counted=excluded.counted",
        params![id,z.id,track.id,track.title,track.artist,track.cover_url,track.duration,if g.started_at==0{now}else{g.started_at},now,z.position,z.status,runtime_id(),g.session.seconds,g.session.counted])?;
    bind_source(tx,&id,z.playback_id.as_deref())?;
    Ok(())
}
pub(super) fn for_playback(c:&Connection,playback:Option<&str>)->anyhow::Result<Option<(String,f64,bool)>> {
    let Some(id)=playback else{return Ok(None)};
    Ok(c.query_row("SELECT g.id,g.seconds,g.counted FROM listen_claims p JOIN listen_groups g ON g.id=p.group_id WHERE p.playback_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?)
}
fn cover(tx:&Transaction<'_>,playback:&str,from:f64,to:f64)->anyhow::Result<f64> {
    if to<=from{return Ok(0.);}
    let ranges=tx.prepare("SELECT start,end FROM listen_intervals WHERE playback_id=?1 ORDER BY start")?.query_map([playback],|r|Ok((r.get::<_,f64>(0)?,r.get::<_,f64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut cursor=from;let mut uncovered=0.;
    for &(start,end) in &ranges {if end<=cursor{continue;}if start>=to{break;}uncovered+=(start.min(to)-cursor).max(0.);cursor=cursor.max(end).min(to);}
    uncovered+=(to-cursor).max(0.);
    let mut merged=ranges;merged.push((from,to));merged.sort_by(|a,b|a.0.total_cmp(&b.0));
    let mut normalized:Vec<(f64,f64)>=Vec::new();
    for range in merged {if let Some(last)=normalized.last_mut().filter(|last|range.0<=last.1){last.1=last.1.max(range.1);}else{normalized.push(range);}}
    tx.execute("DELETE FROM listen_intervals WHERE playback_id=?1",[playback])?;
    for (start,end) in normalized {tx.execute("INSERT INTO listen_intervals(playback_id,start,end) VALUES(?1,?2,?3)",params![playback,start,end])?;}
    Ok(uncovered)
}
pub(super) fn credit(tx:&Transaction<'_>,playback:&str,from:f64,to:f64,at:i64)->anyhow::Result<Option<(f64,bool)>> {
    let claim:Option<(String,f64,u32)>=tx.query_row("SELECT group_id,baseline,timing_version FROM listen_claims WHERE playback_id=?1",[playback],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((id,baseline,version))=claim else{return Ok(None)};
    let Some(g)=group(tx,&id)? else{anyhow::bail!("missing issued listen group")};
    if version!=1{return Ok(Some((0.,g.session.counted)));} // v2 never infers output duration from source positions.
    let seconds=cover(tx,playback,from.max(baseline),to)?;
    Ok(Some(credit_amount(tx,&id,&g,seconds,at)?))
}
fn credit_amount(tx:&Transaction<'_>,id:&str,g:&Group,seconds:f64,at:i64)->anyhow::Result<(f64,bool)> {
    let counted=g.session.counted||g.session.seconds+seconds>=threshold(g.session.duration);
    if seconds>0.||counted!=g.session.counted {aggregate(tx,&g.session,at,seconds,i64::from(counted&&!g.session.counted),0)?;}
    crate::cache_policy::heard(tx, &g.session.track_id, g.session.duration, g.session.seconds, seconds)?;
    let start=at-(seconds*1000.).round() as i64;
    tx.execute("UPDATE listen_groups SET seconds=seconds+?2,counted=?3,updated_at=MAX(updated_at,?4),started_at=CASE WHEN started_at=0 THEN ?5 ELSE MIN(started_at,?5) END WHERE id=?1",params![id,seconds,counted,at,start])?;
    Ok((seconds,counted))
}
/// The group coordinator already validated each endpoint. Its source and output maxima may
/// come from different endpoints, so their difference is not an individual-device rate envelope.
pub(super) fn credit_group_seconds(tx:&Transaction<'_>,playback:&str,id:&str,from:f64,to:f64,seconds:f64,at:i64)->anyhow::Result<()> {
    cover(tx,playback,from,to)?;
    if seconds==0.{return Ok(());}
    let start=at-(seconds*1000.).round() as i64;let mut cursor=start;let mut remaining=seconds;
    loop {
        let date=chrono::DateTime::from_timestamp_millis(cursor).ok_or_else(||anyhow::anyhow!("invalid group output date"))?.date_naive();
        let midnight=date.succ_opt().ok_or_else(||anyhow::anyhow!("group output date overflow"))?.and_hms_opt(0,0,0).unwrap().and_utc().timestamp_millis();
        let stop=at.min(midnight);let amount=if stop==at{remaining}else{seconds*(stop-cursor) as f64/(at-start) as f64};
        let last=if stop==midnight{stop-1}else{stop};let group=group(tx,id)?.ok_or_else(||anyhow::anyhow!("missing group output listen"))?;
        credit_amount(tx,id,&group,amount,last)?;remaining-=amount;
        if stop==at{break;}cursor=stop;
    }Ok(())
}
pub(super) fn credit_output(tx:&Transaction<'_>,playback:&str,group:&str,segment:Segment)->anyhow::Result<bool> {
    if output::seconds(&segment,2).is_none(){return Ok(false);}
    let m=Measured{playback_id:playback.into(),listen_session_id:group.into(),version:2,segments:vec![segment.clone()]};
    if !output::compatible(tx,&m)? {return Ok(false);}
    output::credit(tx,&m,&segment)?;Ok(true)
}
pub(super) fn close(tx:&Transaction<'_>,s:&Session,at:i64,reason:&str)->anyhow::Result<bool> {
    let Some(g)=group(tx,&s.id)? else{return Ok(false)};
    if reason=="ended" {
        if let Some(skipped_at)=g.skipped_at.filter(|_|g.skipped){aggregate(tx,&g.session,skipped_at,0.,0,-1)?;}
        tx.execute("UPDATE listen_groups SET ended=1,skipped=0,skipped_at=NULL WHERE id=?1",[&s.id])?;
    }else if !g.ended&&!g.skipped&&super::early_skip(reason,g.session.seconds,s.last_position,s.duration) {
        aggregate(tx,&g.session,at,0.,0,1)?;
        tx.execute("UPDATE listen_groups SET skipped=1,skipped_at=?2 WHERE id=?1",params![s.id,at])?;
    }Ok(true)
}
pub(super) fn valid(s:&offline::Snapshot)->bool {
    let Some(m)=&s.measured else{return true};
    if ![1,2].contains(&m.version)||uuid::Uuid::parse_str(&m.playback_id).is_err()||uuid::Uuid::parse_str(&m.listen_session_id).is_err()||m.segments.len()>64{return false;}
    let mut days=std::collections::BTreeMap::<String,f64>::new();let mut end=0.;let mut output_end=0;let mut output_rate=None;
    for segment in &m.segments {
        if !segment.start.is_finite()||!segment.end.is_finite()||segment.start<end||segment.end<segment.start||segment.end>s.position+0.001||segment.last_at<s.started_at||segment.last_at>s.updated_at{return false;}
        let Some(seconds)=output::seconds(segment,m.version) else{return false;};
        if let Some(output)=&segment.output {if output.start<output_end||output_rate.is_some_and(|rate|rate!=output.sample_rate){return false;}output_end=output.end;output_rate=Some(output.sample_rate);}
        *days.entry(day(segment.last_at)).or_default()+=seconds;end=segment.end;
    }
    days.len()==s.days.len()&&days.iter().all(|(key,seconds)|s.days.get(key).is_some_and(|d|(d.seconds-seconds).abs()<1e-6))
}
pub(super) fn successor(old:&Measured,next:&Measured)->bool {
    old.version==next.version&&old.playback_id==next.playback_id&&old.listen_session_id==next.listen_session_id&&next.segments.len()>=old.segments.len()&&old.segments.iter().zip(&next.segments).all(|(old,next)|old.start==next.start&&next.end>=old.end&&next.last_at>=old.last_at&&match (&old.output,&next.output){(None,None)=>true,(Some(old),Some(next))=>old.start==next.start&&old.sample_rate==next.sample_rate&&next.end>=old.end,_=>false})
}
pub(super) fn import(tx:&Transaction<'_>,owner:&str,s:&offline::Snapshot)->anyhow::Result<Option<(bool,bool)>> {
    let m=s.measured.as_ref().unwrap();
    let claim:Option<(String,f64,String,u32)>=tx.query_row("SELECT p.group_id,p.baseline,g.track_id,p.timing_version FROM listen_claims p JOIN listen_claim_users u ON u.playback_id=p.playback_id JOIN listen_groups g ON g.id=p.group_id WHERE p.playback_id=?1 AND u.user_id=?2 AND NOT EXISTS(SELECT 1 FROM group_output_totals o WHERE o.playback_id=p.playback_id)",params![m.playback_id,owner],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((id,baseline,track,_))=claim.filter(|(id,_,track,version)|id==&m.listen_session_id&&track==&s.track_id&&*version==m.version) else{return Ok(None)};
    if m.segments.iter().any(|segment|segment.start+0.001<baseline){return Ok(None);}
    if !output::compatible(tx,m)? {return Ok(None);}
    for segment in &m.segments {if m.version==1 {credit(tx,&m.playback_id,segment.start,segment.end,segment.last_at)?;}else{output::credit(tx,m,segment)?;}}
    let mut g=group(tx,&id)?.unwrap();g.session.last_position=s.position;
    close(tx,&g.session,s.updated_at,&s.reason)?;
    if s.reason=="ended" {mark_ended(tx,&m.playback_id,Some(s.position),s.updated_at)?;}
    g=group(tx,&id)?.unwrap();
    let started=if g.started_at==0 {s.started_at}else{g.started_at.min(s.started_at)};
    let existed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM play_sessions WHERE id=?1)",[&id],|r|r.get(0))?;
    tx.execute("INSERT INTO play_sessions(id,zone_id,track_id,title,artist,cover_url,duration,started_at,last_report_at,last_position,last_status,runtime_id,play_seconds,counted,finished_at,finish_reason) VALUES(?1,'offline-measured',?2,?3,?4,?5,?6,?7,?8,?9,'stopped','offline',?10,?11,?8,?12) ON CONFLICT(id) DO UPDATE SET started_at=MIN(started_at,excluded.started_at),play_seconds=excluded.play_seconds,counted=excluded.counted",params![id,track,g.session.title,g.session.artist,g.session.cover_url,g.session.duration,started,s.updated_at,s.position,g.session.seconds,g.session.counted,if s.reason=="active"{"interrupted"}else{&s.reason}])?;
    if !existed {bind_source(tx,&id,Some(&m.playback_id))?;}
    Ok(Some((g.session.counted,g.skipped)))
}
