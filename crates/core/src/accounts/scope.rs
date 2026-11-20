//! Ownership is persistent and checked on the server, including socket reports.
use super::*;
use rusqlite::params;
use std::collections::HashMap;

tokio::task_local! { pub static REQUEST: (Auth, Option<Policy>); }
std::thread_local!{static BLOCKING:std::cell::RefCell<Option<(Auth,Option<Policy>)>>=const{std::cell::RefCell::new(None)};}
pub struct BlockingScope(Option<(Auth,Option<Policy>)>);
impl Drop for BlockingScope{fn drop(&mut self){BLOCKING.with(|v|{v.replace(self.0.take());});}}
pub fn enter_blocking(value:Option<(Auth,Option<Policy>)>)->BlockingScope{BlockingScope(BLOCKING.with(|v|v.replace(value)))}
pub fn current()->Option<(Auth,Option<Policy>)>{REQUEST.try_with(Clone::clone).ok().or_else(||BLOCKING.with(|v|v.borrow().clone()))}
pub fn realm(a:&Auth)->String{if a.role=="external"{format!("external:{}",a.id)}else{"family".into()}}
pub async fn owners(s:&AppState,kind:&str)->ApiResult<HashMap<String,String>>{
 let kind=kind.to_owned();Ok(s.db.read(move|c|Ok(c.prepare("SELECT id,realm FROM external_resource_owners WHERE kind=?1")?.query_map([kind],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)).await?)
}
pub async fn check(s:&AppState,a:&Auth,kind:&str,id:&str)->ApiResult<()> {
 if crate::portable::is_portable(id){return crate::portable::check(s,a,id).await;}

 let kind=kind.to_owned();let id=id.to_owned();let owner=s.db.read(move|c|Ok(c.query_row("SELECT realm FROM external_resource_owners WHERE kind=?1 AND id=?2",rusqlite::params![kind,id],|r|r.get::<_,String>(0)).optional()?)).await?;
 if owner.as_deref().unwrap_or("family")==realm(a){Ok(())}else{Err(ApiError::missing())}
}
pub async fn claim(s:&AppState,a:&Auth,kind:&str,id:&str)->ApiResult<()> {
 let kind=kind.to_owned();let id=id.to_owned();let realm=realm(a);let user=if a.role=="external"{Some(a.id.clone())}else{None};
 let ok=s.db.run(move|c|{let tx=c.transaction()?;
  let old=tx.query_row("SELECT realm FROM external_resource_owners WHERE kind=?1 AND id=?2",params![kind,id],|r|r.get::<_,String>(0)).optional()?;
  if let Some(old)=old{return Ok(old==realm);}
  if realm!="family"&&tx.query_row("SELECT COUNT(*) FROM external_resource_owners WHERE user_id=?1 AND kind=?2",params![user,kind],|r|r.get::<_,i64>(0))?>=256{return Err(invalid("此账号的设备或播放端点已达上限，请移除旧设备"));}
  // Existing unlabelled resources belong to the family. An external client may
  // never adopt a historical device/zone by guessing its identifier.
  let exists=match kind.as_str(){"zone"=>tx.query_row("SELECT EXISTS(SELECT 1 FROM zones WHERE id=?1)",[&id],|r|r.get::<_,bool>(0))?,"device"=>tx.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1)",[&id],|r|r.get(0))?,"endpoint"=>id=="browser-local"||tx.query_row("SELECT EXISTS(SELECT 1 FROM devices d,json_each(d.outputs) o WHERE json_extract(o.value,'$.id')=?1)",[&id],|r|r.get(0))?,_=>true};
  if exists&&realm!="family"{return Ok(false);}
  tx.execute("INSERT INTO external_resource_owners(kind,id,realm,user_id) VALUES(?1,?2,?3,?4)",params![kind,id,realm,user])?;tx.commit()?;Ok(true)
 }).await?;
 if ok{Ok(())}else{Err(ApiError::missing())}
}
pub async fn sanitize(s:&AppState,p:&Policy,value:Value)->ApiResult<Value>{
 let predicate=p.sql("t");Ok(s.db.read(move|c|{
  fn walk(c:&rusqlite::Connection,predicate:&str,mut value:Value,known:&mut HashMap<String,bool>,depth:usize)->anyhow::Result<Value>{
   if depth>24{return Ok(Value::Null);}
   match &mut value{
    Value::Object(o)=>{
     if o.contains_key("format")&&o.contains_key("duration")&&o.contains_key("source"){
      if let Some(id)=o.get("id").and_then(Value::as_str){let allowed=if let Some(v)=known.get(id){*v}else{let v=c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM tracks t WHERE t.id=?1 AND {predicate})"),[id],|r|r.get::<_,bool>(0))?;known.insert(id.into(),v);v};if !allowed{return Ok(Value::Null);}}
     }
     for key in ["password","token","cookie","authorization","headers","play_url","audio_url","media_url"]{o.remove(key);}
     for v in o.values_mut(){*v=walk(c,predicate,std::mem::take(v),known,depth+1)?;}
    },Value::Array(a)=>{for v in a.iter_mut(){*v=walk(c,predicate,std::mem::take(v),known,depth+1)?;}a.retain(|v|!v.is_null());},_=>{}
   }Ok(value)
  }
  walk(c,&predicate,value,&mut HashMap::new(),0)
 }).await?)
}
pub async fn track(s:&AppState,p:&Policy,id:&str)->ApiResult<()> {
 let predicate=p.sql("t");let mut key=id.to_owned();
 let permitted=s.db.read(move|c|{
  for _ in 0..16{if let Some(next)=c.query_row("SELECT new_id FROM track_redirects WHERE old_id=?1",[&key],|r|r.get::<_,String>(0)).optional()?{key=next;}else{break;}}
  Ok(c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM tracks t WHERE t.id=?1 AND {predicate})"),[key],|r|r.get::<_,bool>(0))?)
 }).await?;
 if permitted{Ok(())}else{Err(ApiError::missing())}
}
pub async fn current_track(s:&AppState,id:&str)->ApiResult<()> {
 if let Some((_,Some(policy)))=current(){track(s,&policy,id).await?;}Ok(())
}
pub async fn catalog(s:&AppState,p:&Policy,kind:&str,id:&str)->ApiResult<()> {
 if kind=="track"{return track(s,p,id).await;}
 if !["album","artist"].contains(&kind){return Err(ApiError::missing());}
 if kind=="album"&&id.strip_prefix("release:").and_then(|v|v.split_once(':')).is_some_and(|(provider,_)|p.providers.iter().any(|v|v==provider)){return Ok(());}
 let predicate=p.sql("t");let direct=crate::catalog_detail::direct_ids(kind);let id=id.to_owned();
 let allowed=s.db.read(move|c|Ok(c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM tracks t WHERE t.id IN ({direct}) AND {predicate})"),[id],|r|r.get::<_,bool>(0))?)).await?;
 if allowed{Ok(())}else{Err(ApiError::missing())}
}
pub async fn artwork(s:&AppState,p:&Policy,id:&str,cover:bool)->ApiResult<()> {
 if id.is_empty()||id.len()>200||!id.bytes().all(|b|b.is_ascii_alphanumeric()||b"_-".contains(&b)){return Err(ApiError::missing());}
 let predicate=p.sql("t");let key=id.to_owned();
 let allowed=s.db.read(move|c|{let path=format!("/api/v1/{}/{}",if cover{"covers"}else{"artwork"},key);let nested=format!("%{path}%");
  Ok(c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM tracks t WHERE {predicate} AND (t.cover_url=?1 OR t.provider_data LIKE ?2 OR EXISTS(SELECT 1 FROM artist_metadata m WHERE m.artist_id=t.artist_id AND m.data LIKE ?2) OR EXISTS(SELECT 1 FROM artwork_sources a WHERE a.cover_id=?3 AND t.cover_url='/api/v1/artwork/'||a.id)))"),rusqlite::params![path,nested,key],|r|r.get::<_,bool>(0))?)
 }).await?;if allowed{Ok(())}else{Err(ApiError::missing())}
}
pub fn predicate(alias:&str)->String{current().and_then(|(_,p)|p).map(|p|p.sql(alias)).unwrap_or_else(||"1".into())}
pub fn stats_sql(sql:&str)->String{
 if let Some((a,Some(_)))=current(){sql.replace("track_stats",&format!("(SELECT * FROM account_track_stats WHERE user_id='{}')",a.id.replace('\'',"''")))}else{sql.into()}
}
pub fn online_allowed(source:&str)->bool{current().and_then(|(_,p)|p).is_none_or(|p|p.providers.iter().any(|v|v==source))}
pub async fn zone_user(s:&AppState,id:&str)->ApiResult<Option<String>>{
 let id=id.to_owned();Ok(s.db.read(move|c|Ok(c.query_row("SELECT user_id FROM external_resource_owners WHERE kind='zone' AND id=?1 AND realm!='family'",[id],|r|r.get::<_,Option<String>>(0)).optional()?.flatten())).await?)
}
pub async fn zone_policy(s:&AppState,id:&str)->ApiResult<Option<Policy>>{
 let id=id.to_owned();let row=s.db.read(move|c|Ok(c.query_row("SELECT o.realm,e.policy,e.enabled FROM external_resource_owners o LEFT JOIN external_accounts e ON e.user_id=o.user_id WHERE o.kind='zone' AND o.id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<bool>>(2)?))).optional()?)).await?;
 match row{None=>Ok(None),Some((realm,_,_))if realm=="family"=>Ok(None),Some((_,Some(raw),Some(true)))=>{let p:Policy=serde_json::from_str(&raw).map_err(anyhow::Error::from)?;p.validate()?;Ok(Some(p))},_=>Err(ApiError::forbidden())}
}
pub async fn disconnect(s:&AppState,user:&str)->ApiResult<()> {
 let _lock=s.zone_lock.lock().await;let realm=format!("external:{user}");
 let endpoints=owners(s,"endpoint").await?.into_iter().filter(|(_,r)|r==&realm).map(|(id,_)|id).collect::<Vec<_>>();
 let zones=owners(s,"zone").await?.into_iter().filter(|(_,r)|r==&realm).map(|(id,_)|id).collect::<Vec<_>>();
 for id in &zones{let current=s.zones.read().await.get(id).cloned();if let Some(mut zone)=current{let _=crate::zones::dispatch(s,&zone,"stop").await;crate::autoplay::cancel(s,id).await;let _=crate::stats::finish(s,&zone,"account_revoked").await;zone.status="stopped".into();zone.playback_id=None;let _=s.save_zone(zone).await;}}
 for id in endpoints{if let Some(tx)=s.bridges.write().await.remove(&id){let _=tx.try_send(json!({"type":"remove_endpoint","endpoint_id":id}));}s.browser_owners.write().await.remove(&id);s.bridge_owners.write().await.remove(&id);if let Some(e)=s.endpoints.write().await.get_mut(&id){e["online"]=json!(false);}}
 s.emit("endpoints",json!({}));Ok(())
}
/// Only ownership-labelled events leave this boundary. Unknown global events
/// are invalidations for the family, never a source of external account data.
pub async fn event(s:&AppState,a:&Auth,mut event:Value)->ApiResult<Option<Value>> {
 let kind=event["type"].as_str().unwrap_or("").to_owned();
 match kind.as_str(){
  "portable_command"|"portable_loudness"=>{
   if event["data"]["session"]!=a.session_hash{return Ok(None);}
   if kind=="portable_loudness" {
    if let Some(policy)=crate::accounts::policy(s,a).await? {
     let Some(id)=event["data"]["track_id"].as_str() else{return Ok(None);};
     if track(s,&policy,id).await.is_err(){return Ok(None);}
    }
   }
   event["data"].as_object_mut().unwrap().remove("session");Ok(Some(event))
  },
  "zones_changed"=>Ok(Some(json!({"type":"zones_changed","data":{}}))),
  "zones"|"resync"=>{let map=owners(s,"zone").await?;let compact=event["data"]["queue_view"]=="versioned";if let Some(items)=event["data"]["items"].as_array_mut(){items.retain(|v|map.get(v["id"].as_str().unwrap_or("")).map(String::as_str).unwrap_or("family")==realm(a));crate::portable::overlay(items,crate::portable::zones(s,a,compact).await);}Ok(Some(event))},
  "zone_progress"|"stats"|"playback_error"|"autoplay"|"sequence_invalidated"=>{if let Some(realm)=event["data"]["account_realm"].as_str(){return Ok(if realm==self::realm(a){Some(event)}else{None});}let id=event["data"]["zone_id"].as_str().or_else(||event["data"]["id"].as_str());if let Some(id)=id{if check(s,a,"zone",id).await.is_ok(){return Ok(Some(event));}}Ok(None)},
  "endpoint_signal"|"device_volume"|"browser_visualization_lease"=>{if let Some(id)=event["data"]["endpoint_id"].as_str(){if check(s,a,"endpoint",id).await.is_ok(){return Ok(Some(event));}}Ok(None)},
  "endpoints"|"devices"|"playlists"=>Ok(Some(json!({"type":kind,"data":{}}))),
  _=>Ok(if a.role=="external"{None}else{Some(event)}),
 }
}
