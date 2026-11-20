//! Personal accounting reuses measured device deltas, never client wall time.
use super::*;
use rusqlite::{params,Connection,Transaction};
/// These connection-local read views let existing statistics projections retain
/// their date/ranking contracts. They are always removed before returning a
/// pooled connection, including failed queries.
pub(crate) fn with_stats<T>(c:&Connection,user:Option<&str>,f:impl FnOnce()->anyhow::Result<T>)->anyhow::Result<T>{
 let Some(user)=user else{return f();};
 let raw=c.query_row("SELECT policy FROM main.external_accounts WHERE user_id=?1",[user],|r|r.get::<_,String>(0)).optional()?;
 let policy=raw.map(|v|serde_json::from_str::<Policy>(&v)).transpose()?.unwrap_or_default();let permission=policy.sql("t");let user=user.replace('\'',"''");
 struct Cleanup<'a>(&'a Connection);impl Drop for Cleanup<'_>{fn drop(&mut self){let _=self.0.execute_batch("DROP VIEW IF EXISTS temp.daily_stats;DROP VIEW IF EXISTS temp.listening_buckets;DROP VIEW IF EXISTS temp.track_stats;DROP VIEW IF EXISTS temp.listening_metadata;DROP VIEW IF EXISTS temp.tracks;");}}
 let _cleanup=Cleanup(c);
 c.execute_batch(&format!("CREATE TEMP VIEW tracks AS SELECT t.* FROM main.tracks t WHERE {permission}; CREATE TEMP VIEW listening_metadata AS SELECT track_id,album,album_artist,NULLIF(source,'unknown') source FROM main.account_listening_metadata WHERE user_id='{user}' UNION ALL SELECT m.track_id,m.album,m.album_artist,m.source FROM main.listening_metadata m JOIN temp.tracks t ON t.id=m.track_id WHERE NOT EXISTS(SELECT 1 FROM main.account_listening_metadata owned WHERE owned.user_id='{user}' AND owned.track_id=m.track_id);"))?;
 c.execute_batch(&format!("CREATE TEMP VIEW track_stats AS SELECT track_id,play_count,skip_count,play_seconds,last_played_at FROM main.account_track_stats WHERE user_id='{user}'; CREATE TEMP VIEW listening_buckets AS SELECT minute,track_id,title,artist,cover_url,play_count,skip_count,play_seconds,last_played_at FROM main.account_listening_buckets WHERE user_id='{user}'; CREATE TEMP VIEW daily_stats AS SELECT date(minute*60,'unixepoch') day,track_id,title,artist,cover_url,SUM(play_count) play_count,SUM(skip_count) skip_count,SUM(play_seconds) play_seconds,MAX(last_played_at) last_played_at FROM main.account_listening_buckets WHERE user_id='{user}' GROUP BY day,track_id;"))?;

 f()
}
pub(crate) fn metadata(c:&Connection,user:&str,track:&str,album:&str,artist:&str,source:&str,at:i64)->anyhow::Result<()> {
 c.execute("INSERT INTO account_listening_metadata(user_id,track_id,album,album_artist,source,updated_at) SELECT ?1,?2,?3,?4,?5,?6 WHERE EXISTS(SELECT 1 FROM external_accounts WHERE user_id=?1) ON CONFLICT(user_id,track_id) DO UPDATE SET album=CASE WHEN excluded.album!='' AND excluded.updated_at>=updated_at THEN excluded.album ELSE album END,album_artist=CASE WHEN excluded.album_artist!='' AND excluded.updated_at>=updated_at THEN excluded.album_artist ELSE album_artist END,source=CASE WHEN excluded.source!='unknown' AND excluded.updated_at>=updated_at THEN excluded.source ELSE source END,updated_at=MAX(updated_at,excluded.updated_at)",params![user,track,album,artist,source,at])?;Ok(())
}
pub(crate) fn remember(c:&Connection,user:&str,track:&crate::models::Track)->anyhow::Result<()> {
 metadata(c,user,&track.id,&track.album,&track.album_artist,track.provider_data["cached_from"]["source"].as_str().unwrap_or(&track.source),chrono::Utc::now().timestamp_millis())
}
pub(crate) fn own(c:&Connection,event:&str,user:&str)->anyhow::Result<()> {
 c.execute("INSERT OR IGNORE INTO account_session_owners(event_id,user_id,external,data_epoch) SELECT ?1,u.id,u.role='external',COALESCE(e.data_epoch,0) FROM users u LEFT JOIN external_accounts e ON e.user_id=u.id WHERE u.id=?2",params![event,user])?;Ok(())
}
pub(crate) fn own_zone(c:&Connection,event:&str,zone:&str)->anyhow::Result<()> {
 let user=c.query_row("SELECT user_id FROM external_resource_owners WHERE kind='zone' AND id=?1 AND realm!='family'",[zone],|r|r.get::<_,Option<String>>(0)).optional()?.flatten();
 if let Some(user)=user.or(crate::radio::owner_at(c,zone)?){own(c,event,&user)?;}Ok(())
}
pub(crate) fn aggregate(tx:&Transaction<'_>,event:&str,track:&str,title:&str,artist:&str,cover:&Option<String>,at:i64,seconds:f64,plays:i64,skips:i64)->anyhow::Result<bool>{
 let owner=tx.query_row("SELECT user_id,external FROM account_session_owners WHERE event_id=?1",[event],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?))).optional()?;
 let Some((user,external))=owner else{
  return Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM play_sessions p JOIN external_resource_owners o ON o.kind='zone' AND o.id=p.zone_id WHERE p.id=?1 AND o.realm!='family')",[event],|r|r.get(0))?);
 };
 if external {
  let active=tx.query_row("SELECT EXISTS(SELECT 1 FROM external_accounts e JOIN account_session_owners o ON o.user_id=e.user_id WHERE o.event_id=?1 AND e.enabled=1 AND e.data_epoch=o.data_epoch)",[event],|r|r.get::<_,bool>(0))?;
  if !active{return Ok(true);}
 }
 let last=if seconds>0.{Some(at/1000)}else{None};
 tx.execute("INSERT INTO account_track_stats(user_id,track_id,play_count,skip_count,play_seconds,last_played_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,track_id) DO UPDATE SET play_count=MAX(0,play_count+excluded.play_count),skip_count=MAX(0,skip_count+excluded.skip_count),play_seconds=MAX(0,play_seconds+excluded.play_seconds),last_played_at=MAX(COALESCE(last_played_at,0),COALESCE(excluded.last_played_at,0))",params![user,track,plays,skips,seconds,last])?;
 if !external {return Ok(false);}
 let minute=at.div_euclid(60000);
 tx.execute("INSERT INTO account_listening_buckets(user_id,minute,track_id,title,artist,cover_url,play_count,skip_count,play_seconds,last_played_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(user_id,minute,track_id) DO UPDATE SET play_count=MAX(0,play_count+excluded.play_count),skip_count=MAX(0,skip_count+excluded.skip_count),play_seconds=MAX(0,play_seconds+excluded.play_seconds),last_played_at=MAX(COALESCE(last_played_at,0),COALESCE(excluded.last_played_at,0))",params![user,minute,track,title,artist,cover,plays,skips,seconds,last])?;
 // Native contributions use stable minute/track origins, so export/re-import is
 // idempotent and can merge newer snapshots without double counting.
 let core:String=tx.query_row("SELECT json_extract(value,'$') FROM settings WHERE key='core_id'",[],|r|r.get(0))?;
 let key=format!("live:{minute}:{}",crate::auth::hash(track));
 let mut record=tx.query_row("SELECT data FROM account_stat_origins WHERE user_id=?1 AND origin_core=?2 AND origin_user=?1 AND origin_id=?3",params![user,core,key],|r|r.get::<_,String>(0)).optional()?.map(|s|serde_json::from_str::<archive::Stat>(&s)).transpose()?.unwrap_or(archive::Stat{revision:0,minute,track_id:track.into(),title:title.into(),artist:artist.into(),cover_url:cover.clone(),play_count:0,skip_count:0,play_seconds:0.,last_played_at:None});
 record.revision=record.revision.saturating_add(1);record.play_count=(record.play_count as i64+plays).max(0)as u64;record.skip_count=(record.skip_count as i64+skips).max(0)as u64;record.play_seconds=(record.play_seconds+seconds).max(0.);record.last_played_at=record.last_played_at.max(last);
 tx.execute("INSERT INTO account_stat_origins(user_id,origin_core,origin_user,origin_id,minute,track_id,data) VALUES(?1,?2,?1,?3,?4,?5,?6) ON CONFLICT(user_id,origin_core,origin_user,origin_id) DO UPDATE SET data=excluded.data",params![user,core,key,minute,track,serde_json::to_string(&record)?])?;Ok(true)
}
pub(crate) fn history(c:&Connection,user:&str,limit:usize,before:Option<(i64,String)>)->anyhow::Result<Value>{
 let (at,id)=before.unwrap_or((i64::MAX,String::new()));
 let mut items=c.prepare("SELECT event_id,started_at,data FROM account_history WHERE user_id=?1 AND (started_at,event_id)<(?2,?3) ORDER BY started_at DESC,event_id DESC LIMIT ?4")?.query_map(params![user,at,id,(limit+1)as i64],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
 let more=items.len()>limit;items.truncate(limit);let cursor=if more{items.last().map(|(id,at,_)|format!("{at}:{id}"))}else{None};
 let items=items.into_iter().map(|(_,_,s)|serde_json::from_str::<Value>(&s)).collect::<Result<Vec<_>,_>>()?;
 Ok(json!({"items":items,"next_cursor":cursor}))
}
