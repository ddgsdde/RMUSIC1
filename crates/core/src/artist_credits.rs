//! Indexed performer credits. Display separators alone are never identity evidence.
use rusqlite::{params,Connection,OptionalExtension};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Credit {pub id:String,pub name:String}
fn credit(source:&str,name:&str,provider_id:Option<&str>)->Credit {
 Credit{id:crate::auth::hash(&provider_id.filter(|s|!s.is_empty()).map(|id|format!("artist\0{source}\0{id}")).unwrap_or_else(||name.to_owned())),name:name.to_owned()}
}
pub fn credits(source:&str,artist:&str,data:&Value)->Vec<Credit>{
 let source=data["cached_from"]["source"].as_str().filter(|_|source=="local").unwrap_or(source);
 let mut result=Vec::new();
 if let Some(values)=data["artist_credits"].as_array().filter(|v|!v.is_empty()&&v.len()<=64&&v.iter().all(|c|c["name"].as_str().is_some_and(|n|!n.trim().is_empty()))){
  for value in values {if let Some(name)=value["name"].as_str().filter(|v|!v.trim().is_empty()){result.push(credit(source,name,value["id"].as_str()));}}
 }
 // Older indexed tracks may carry performer IDs only inside confirmed metadata.
 // Require the same provider and exact credit text; never apply a name-only match.
 if result.is_empty() {
  for key in ["provider_metadata","qqmusic_metadata"] {
   let metadata=&data[key];
   if metadata["matched"]==true&&metadata["source"].as_str()==Some(source)&&metadata["track"]["artist"].as_str()==Some(artist) {
    if let Some(rows)=metadata["artists"].as_array().filter(|v|!v.is_empty()&&v.len()<=64) {
     if rows.iter().all(|v|v["name"].as_str().is_some_and(|n|!n.trim().is_empty())&&v["id"].as_str().or_else(||v["mid"].as_str()).is_some_and(|n|!n.is_empty())) {
      let names=rows.iter().map(|v|v["name"].as_str().unwrap()).collect::<Vec<_>>().join(" / ");
      if names==artist {result=rows.iter().map(|v|credit(source,v["name"].as_str().unwrap(),v["id"].as_str().or_else(||v["mid"].as_str()))).collect();break;}
     }
    }
   }
  }
 }
 // Legacy QQ payloads have parallel provider IDs and a known, provider-generated separator.
 if result.is_empty()&&source=="qqmusic" {
  if let Some(ids)=data["artist_mids"].as_array(){let names:Vec<_>=artist.split(" / ").collect();
   if ids.len()==names.len()&&ids.len()<=64&&ids.iter().all(|id|id.as_str().is_some_and(|v|!v.is_empty())) {
    result=names.iter().zip(ids).map(|(name,id)|credit(source,name,id.as_str())).collect();
   }
  }
 }
 if result.is_empty(){result.push(credit(source,artist,None));}
 let mut seen=std::collections::HashSet::new();result.retain(|c|seen.insert(c.id.clone()));result
}
pub fn index(c:&Connection,t:&crate::models::Track)->anyhow::Result<()> {
 c.execute("DELETE FROM track_artists WHERE track_id=?1",[&t.id])?;
 for (position,credit) in credits(&t.source,&t.artist,&t.provider_data).iter().enumerate(){
  c.execute("INSERT INTO track_artists(track_id,artist_id,name,position) VALUES(?1,?2,?3,?4)",params![t.id,credit.id,credit.name,position as i64])?;
  for key in ["provider_metadata","qqmusic_metadata"] {
   let metadata=&t.provider_data[key];
   if metadata["matched"]!=true {continue;}
   if let Some(person)=metadata["artists"].as_array().and_then(|v|v.iter().find(|v|v["name"].as_str()==Some(&credit.name))) {
    let image=person["image_url"].as_str().filter(|u|u.starts_with("https://"));
    if image.is_some()||person["description"].as_str().is_some_and(|s|!s.is_empty()) {
     let image=image.and_then(|u|crate::media_assets::register(c,u).ok());
     let value=json!({"source":metadata["source"],"image_url":image,"description":person["description"]});
     c.execute("INSERT INTO artist_metadata(artist_id,data)VALUES(?1,?2) ON CONFLICT(artist_id)DO UPDATE SET data=excluded.data",params![credit.id,value.to_string()])?;
    }
   }
  }
 }Ok(())
}
pub fn migrate(c:&Connection)->anyhow::Result<()> {
 if c.query_row("SELECT EXISTS(SELECT 1 FROM settings WHERE key='artist_credits_v2')",[],|r|r.get::<_,bool>(0))?{return Ok(());}
 let tx=c.unchecked_transaction()?;let mut after=0i64;
 loop {
  let batch=tx.prepare("SELECT rowid AS migration_rowid,* FROM tracks WHERE rowid>?1 ORDER BY rowid LIMIT 256")?.query_map([after],|r|Ok((r.get::<_,i64>("migration_rowid")?,crate::models::Track::from_row(r)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
  if batch.is_empty(){break;}
  for(row,t)in batch{index(&tx,&t)?;after=row;}
 }
 tx.execute("INSERT INTO settings(key,value) VALUES('artist_credits_v2','true')",[])?;tx.commit()?;Ok(())
}
pub fn release_key(source:&str,data:&Value)->Option<String>{
 let provider=data["cached_from"]["source"].as_str().filter(|_|source=="local").unwrap_or(source);
 let external=data["release_id"].as_str().or_else(||data["album_mid"].as_str())?;
 if crate::connectors::valid_id(provider)&&!external.is_empty()&&external.len()<=128&&!external.chars().any(char::is_control){Some(format!("release:{provider}:{external}"))}else{None}
}
/// Upgrade legacy combined captions using the already-known provider recording ID.
/// Preserve all other metadata; a separator alone never creates performer identities.
async fn hydrate_credits(s:&crate::state::AppState,track:crate::models::Track)->crate::error::ApiResult<crate::models::Track>{
 if track.source=="local"||!track.artist.contains(" / ")||credits(&track.source,&track.artist,&track.provider_data).len()>1{return Ok(track);}
 let manifest=crate::connectors::manifest(s,&track.source).await?;
 if !manifest["capabilities"].as_array().is_some_and(|v|v.iter().any(|v|v=="metadata")){return Ok(track);}
 let value=s.plugin.call(&track.source,"metadata",json!({"track":track,"section":"basic"})).await?;
 if value["matched"]!=true||value["source"]!=track.source||value["track"]["id"]!=track.id {return Ok(track);}
 let Some(people)=value["artists"].as_array().filter(|v|!v.is_empty()&&v.len()<=64) else{return Ok(track);};
 if people.iter().any(|v|v["name"].as_str().is_none_or(|n|n.trim().is_empty())||v["id"].as_str().or_else(||v["mid"].as_str()).is_none_or(|id|id.is_empty()||id.len()>128)){return Ok(track);}
 if people.iter().map(|v|v["name"].as_str().unwrap()).collect::<Vec<_>>().join(" / ")!=track.artist{return Ok(track);}
 let people=people.iter().map(|v|json!({"name":v["name"],"id":v["id"].as_str().or_else(||v["mid"].as_str())})).collect::<Vec<_>>();
 let updated=s.db.run(move|c|{
  let tx=c.transaction()?;let mut current=tx.query_row("SELECT * FROM tracks WHERE id=?1",[&track.id],crate::models::Track::from_row)?;
  if current.artist==track.artist&&current.source==track.source {
   if !current.provider_data.is_object(){current.provider_data=json!({});}
   current.provider_data["artist_credits"]=json!(people);
   tx.execute("UPDATE tracks SET provider_data=?1 WHERE id=?2",params![current.provider_data.to_string(),current.id])?;
   index(&tx,&current)?;crate::music_identity::index(&tx,&current,None)?;
  }
  tx.commit()?;Ok(current)
 }).await?;
 s.emit("library",json!({"reason":"artist_credits"}));Ok(updated)
}
#[derive(Deserialize)]
pub struct LinkRequest {#[serde(default)]pub resolve_credits:bool,#[serde(default)]pub track_id:Option<String>,pub artist:String,pub album:String,#[serde(default)]pub album_artist:Option<String>,#[serde(default)]pub source:String,#[serde(default)]pub provider_data:Value}
pub async fn links(axum::extract::State(s):axum::extract::State<crate::state::AppState>,a:crate::auth::Auth,axum::Json(mut v):axum::Json<LinkRequest>)->crate::error::ApiResult<axum::Json<Value>> {
 a.require("read")?;
 if v.artist.len()>8192||v.album.len()>8192||v.source.len()>128{return Err(crate::error::ApiError::bad("音乐资料过长"));}
 if a.role=="external" {
  // Resolve only indexed, authorized records. Caller metadata never creates or
  // hydrates a shared catalogue identity on behalf of an external account.
  let id=v.track_id.take().filter(|v|!v.is_empty()&&v.len()<=512).ok_or_else(crate::error::ApiError::missing)?;
  let name=v.artist.clone();let permission=crate::accounts::scope::predicate("t");
  let track=s.db.read(move|c|Ok(c.query_row(&format!("SELECT t.* FROM tracks t WHERE (t.id=?1 OR (t.album_id=?1 AND t.artist=?2)) AND {permission} ORDER BY t.id=?1 DESC,t.id LIMIT 1"),params![id,name],crate::models::Track::from_row).optional()?)).await?.ok_or_else(crate::error::ApiError::missing)?;
  v.artist=track.artist;v.album=track.album;v.album_artist=Some(track.album_artist);v.source=track.source;v.provider_data=track.provider_data;
 } else if let Some(id)=v.track_id.take().filter(|v|v.len()<=512) {
  let name=v.artist.clone();
  if let Some(track)=s.db.read(move|c|Ok(c.query_row("SELECT * FROM tracks WHERE id=?1 OR (album_id=?1 AND artist=?2) ORDER BY id=?1 DESC,id LIMIT 1",params![id,name],crate::models::Track::from_row).optional()?)).await?{
   let track=if v.resolve_credits {hydrate_credits(&s,track).await?}else{track};
   v.artist=track.artist;v.album=track.album;v.album_artist=Some(track.album_artist);v.source=track.source;v.provider_data=track.provider_data;
  }
 }
 let album_artist=v.album_artist.as_deref().unwrap_or(&v.artist);
 Ok(axum::Json(json!({"resolved":!v.source.is_empty(),"artists":credits(&v.source,&v.artist,&v.provider_data),"album":{"id":release_key(&v.source,&v.provider_data).unwrap_or_else(||crate::auth::hash(&format!("{}\0{}\0{}",v.source,album_artist,v.album))),"title":v.album}})))
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn provider_ids_separate_collaborators_without_splitting_band_names(){
  let a=credits("qqmusic","Crush / IU",&json!({"artist_mids":["crush","iu"]}));assert_eq!(a.len(),2);assert_eq!(a[1].id,credits("qqmusic","IU",&json!({"artist_mids":["iu"]}))[0].id);
  let legacy=json!({"provider_metadata":{"matched":true,"source":"qqmusic","track":{"artist":"Crush / IU"},"artists":[{"mid":"crush","name":"Crush"},{"mid":"iu","name":"IU"}]}});assert_eq!(credits("qqmusic","Crush / IU",&legacy)[1].id,a[1].id);assert_eq!(credits("qqmusic","Another",&legacy).len(),1);
  assert_eq!(credits("local","AC/DC",&json!({})).len(),1);assert_eq!(credits("local","Earth, Wind & Fire",&json!({})).len(),1);
  assert_eq!(credits("qqmusic","A / B",&json!({"artist_mids":["a"]})).len(),1);
  assert_ne!(credits("qqmusic","IU",&json!({"artist_mids":["other"]}))[0].id,a[1].id);
 }
}
