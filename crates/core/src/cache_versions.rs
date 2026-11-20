//! Replace only owned cache versions after the old programme and readers have finished.
use crate::{state::AppState,models::Track};
use rusqlite::params;
fn native(t:&Track)->bool{t.provider_data["cache_quality"]["enhanced"]!=true&&!t.provider_data["enhanced"].as_bool().unwrap_or(false)&&!t.provider_data["upscaled"].as_bool().unwrap_or(false)&&!t.provider_data["cached_from"]["enhanced"].as_bool().unwrap_or(false)}
fn better(new:&Track,old:&Track)->bool{
 let rank=|t:&Track|rmusic_audio::probe::quality_rank(&t.format,t.sample_rate,u16::try_from(t.bit_depth).unwrap_or(0),t.bitrate);
 new.format!="m4a"&&old.format!="m4a"&&native(new)&&native(old)&&new.channels==old.channels&&new.channels>0&&rank(old)>0&&rank(new)>rank(old)
  &&new.sample_rate>=old.sample_rate&&(rank(old)<1_000_000_000||new.bit_depth>=old.bit_depth)
}
pub(crate) async fn consider(s:&AppState,new:Track)->anyhow::Result<()>{
 if !native(&new){return Ok(());}let Some(new_cache)=new.provider_data["cached_from"]["cache_id"].as_str().map(String::from)else{return Ok(());};
 s.db.run(move|c|{let candidates=c.prepare("SELECT t.* FROM tracks t JOIN cache_entries e ON e.path=t.path WHERE t.source='local' AND t.available=1 AND t.root_id IS NULL AND t.id!=?1 AND e.complete=1 AND json_type(t.provider_data,'$.cached_from.cache_id')='text' AND t.recording_id=?2 AND t.recording_id<>''")?.query_map(params![new.id,new.recording_id],Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
  for old in candidates{if better(&new,&old){if let Some(cache)=old.provider_data["cached_from"]["cache_id"].as_str(){c.execute("INSERT INTO cache_replacements(old_cache,new_cache,old_track,new_track) VALUES(?1,?2,?3,?4) ON CONFLICT(old_cache) DO UPDATE SET new_cache=excluded.new_cache,new_track=excluded.new_track",params![cache,new_cache,old.id,new.id])?;c.execute("UPDATE cache_replacements SET new_cache=?2,new_track=?3 WHERE new_cache=?1",params![cache,new_cache,new.id])?;}}}Ok(())
 }).await
}
pub(crate) async fn cleanup(s:&AppState)->anyhow::Result<()>{
 let _guard=s.cache_lock.lock().await;
 let entries=s.db.run(|c|Ok(c.prepare("SELECT r.old_cache,r.new_cache,r.old_track,r.new_track,e.track_id,e.path FROM cache_replacements r JOIN cache_entries e ON e.id=r.old_cache JOIN cache_entries n ON n.id=r.new_cache AND n.complete=1 JOIN tracks t ON t.id=r.new_track AND t.available=1")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
 for(old_cache,new_cache,old_track,new_track,origin,path)in entries{
  if s.cache_active.lock().await.contains_key(&old_cache)||s.zones.read().await.values().any(|z|matches!(z.status.as_str(),"playing"|"paused")&&z.track.as_ref().is_some_and(|t|t.id==old_track||t.id==origin)){continue;}
  // Only the recorded cache-ID directory is owned; user music roots are never passed here.
  let p=std::path::Path::new(&path);if p.parent().and_then(|p|p.file_name()).and_then(|v|v.to_str())!=Some(old_cache.as_str()){continue;}
  crate::media::remove_export_directory(&old_cache,&path).await?;
  s.db.run(move|c|{let tx=c.transaction()?;
   tx.execute("INSERT INTO track_redirects(old_id,new_id) VALUES(?1,?2) ON CONFLICT(old_id) DO UPDATE SET new_id=excluded.new_id",params![old_track,new_track])?;
   tx.execute("UPDATE track_redirects SET new_id=?2 WHERE new_id=?1",params![old_track,new_track])?;
   tx.execute("UPDATE tracks SET available=0 WHERE id=?1 AND source='local' AND root_id IS NULL",[&old_track])?;
   tx.execute("UPDATE cache_entries SET pinned=MAX(pinned,COALESCE((SELECT pinned FROM cache_entries WHERE id=?2),0)) WHERE id=?1",params![new_cache,old_cache])?;
   let lists=tx.prepare("SELECT id,tracks FROM playlists WHERE EXISTS(SELECT 1 FROM json_each(playlists.tracks) WHERE value=?1)")?.query_map([&old_track],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
   for(id,tracks)in lists{let tracks:Vec<String>=serde_json::from_str(&tracks)?;let tracks=tracks.into_iter().map(|v|if v==old_track{new_track.clone()}else{v}).collect::<Vec<_>>();tx.execute("UPDATE playlists SET tracks=?2 WHERE id=?1",params![id,serde_json::to_string(&tracks)?])?;}
   tx.execute("DELETE FROM cache_entries WHERE id=?1",[&old_cache])?;tx.execute("DELETE FROM cache_replacements WHERE old_cache=?1",[old_cache])?;tx.commit()?;Ok(())
  }).await?;s.emit("library",serde_json::json!({}));s.emit("playlists",serde_json::json!({}));
 }Ok(())
}
