//! Explicit external API surface, persistent resource ownership and media quota.
use super::*;
use axum::{body::{to_bytes,Body},extract::{Request,State},middleware::Next,response::{Response,IntoResponse}};
fn decode(value:&str)->String{url::form_urlencoded::parse(format!("id={}",value.replace('+',"%2B")).as_bytes()).next().map(|(_,v)|v.into_owned()).unwrap_or_default()}
fn allowed(path:&str,method:&str)->bool{
 let v=path.trim_matches('/').split('/').collect::<Vec<_>>();
 if matches!(path,"/auth/me"|"/auth/logout"|"/auth/account"|"/auth/totp/start"|"/auth/totp/confirm"|"/auth/totp/disable"|"/auth/sessions"|"/preferences"){return true;}
 if v.first()==Some(&"portable")&&v.len()==3{return matches!((v[2],method),("state","PUT"|"DELETE")|("ack","POST"));}
 if matches!(path,"/recommendations/radio"|"/recommendations/similar"){return method=="POST";}
 if path.starts_with("/recommendations/feedback/"){return method=="PUT";}
 if matches!(path,"/recommendations/discover"|"/recommendations/daily"|"/recommendations/settings"){return matches!(method,"GET"|"HEAD");}
 if path=="/connection-info" {return matches!(method,"GET"|"HEAD");}
 if path=="/auth/sessions/current-device"{return method=="PUT";}
 if path.starts_with("/auth/sessions/"){return method=="DELETE";}
 if matches!(path,"/zones"|"/devices/register"|"/endpoints/browser"|"/events/ticket"|"/events"|"/bridge/events"|"/favorites"|"/playlists"){return true;}
 if v.first()==Some(&"zones")&&v.len()>=2{return matches!(v.get(2).copied().unwrap_or(""),""|"play"|"pause"|"stop"|"next"|"previous"|"seek"|"progress"|"ended"|"queue"|"radio"|"dsp"|"prepare"|"handoff"|"recover"|"transfer"|"ready"|"visualization");}
 if v.first()==Some(&"outputs")&&v.len()>=2{return v.len()==2||matches!(v[2],"config"|"volume");}
 if v.first()==Some(&"devices"){if v.len()==2{return matches!(method,"PATCH"|"DELETE");}if v.len()==3&&v[2]=="restore"{return method=="POST";}}
 if v.first()==Some(&"endpoints")&&v.get(1)==Some(&"browser")&&v.len()==4&&v[3]=="disconnect"{return true;}
 if v.first()==Some(&"favorites")&&v.len()==2{return v[1]!="binding";}
 if v.first()==Some(&"playlists")&&v.len()>=2&&!matches!(v[1],"bind"|"import"){return v.len()==2||matches!(v[2],"tracks"|"export");}
 if matches!(path,"/library/links"|"/stats/offline"|"/smart-playlists/preview"|"/playlists/import"|"/playlists/import/preview"){return true;}
 if method=="PUT"&&v.len()>=4&&v[..2]==["library","tracks"]&&v[3]=="versions"{return true;}
 if !matches!(method,"GET"|"HEAD"){return false;}
 if matches!(path,"/settings"|"/plugins"|"/endpoints"|"/devices"|"/stats"|"/stats/history"|"/recommendations"|"/search"|"/library/search"|"/playlist-directory"|"/library/home"|"/library/tracks"|"/library/albums"|"/library/artists"){return true;}
 if v.first()==Some(&"library")&&v.get(1)==Some(&"catalog")&&v.len()>=4{return v.len()==4||matches!(v[4],"albums"|"profile"|"links");}
 if v.first()==Some(&"library")&&v.get(1)==Some(&"tracks")&&v.len()>=3{return v.len()==3||matches!(v[3],"lyrics"|"ticket"|"stats"|"audio-info"|"comments"|"background"|"related"|"versions");}
 if v.first()==Some(&"providers")&&v.len()>=3{return matches!(v[2],"search"|"recommendations")||v.len()==5&&v[2]=="tracks"&&v[4]=="qualities";}
 matches!(v.first().copied(),Some("media"|"covers"|"artwork"|"pcm"|"native-pcm"|"native-flac"|"mobile-opus"|"transcode"))&&v.len()==2
}
async fn body_access(s:&AppState,a:&Auth,p:Option<&Policy>,path:&str,value:&Value)->ApiResult<()> {
 let controls=path.starts_with("/zones")||path.starts_with("/outputs")||path.starts_with("/devices")||path.starts_with("/sync/")||path.starts_with("/output-groups");
 let mut retained=std::collections::HashSet::<String>::new();
 if path.starts_with("/playlists/"){let id=path.trim_start_matches("/playlists/").split('/').next().map(decode).unwrap_or_default();let uid=a.id.clone();retained=s.db.read(move|c|Ok(c.query_row("SELECT tracks FROM playlists WHERE id=?1 AND user_id=?2",rusqlite::params![id,uid],|r|r.get::<_,String>(0)).optional()?.and_then(|s|serde_json::from_str::<Vec<String>>(&s).ok()).unwrap_or_default().into_iter().collect())).await?;}
 if path.starts_with("/favorites/"){if value["liked"]!=false{if let Some(p)=p{scope::track(s,p,&decode(path.trim_start_matches("/favorites/"))).await?;}}}
 let mut pending=vec![value];let mut tracks=std::collections::HashSet::new();
 while let Some(value)=pending.pop(){match value{
  Value::Object(object)=>for (key,value) in object{match key.as_str(){
   "expected"=>continue,
   "endpoint_id" if controls=>{if let Some(id)=value.as_str(){scope::check(s,a,"endpoint",id).await?;}},
   "target_zone_id"|"zone_id" if controls=>{if let Some(id)=value.as_str(){scope::check(s,a,"zone",id).await?;}},
   "zone_ids" if controls=>if let Some(ids)=value.as_array(){for id in ids{if let Some(id)=id.as_str(){scope::check(s,a,"zone",id).await?;}}},
   "track_id"|"preferred_track_id"=>{if let Some(id)=value.as_str(){tracks.insert(id.to_owned());}},
   "track_ids"=>if let Some(ids)=value.as_array(){for id in ids{if let Some(id)=id.as_str(){tracks.insert(id.to_owned());}}},
   _=>pending.push(value),
  }},Value::Array(items)=>pending.extend(items),_=>{}
 }}
 if let Some(p)=p.filter(|_|path!="/stats/offline"&&path!="/preferences"&&path!="/library/links"){for id in tracks{if !retained.contains(&id){scope::track(s,p,&id).await?;}}}Ok(())
}
async fn access(s:&AppState,a:&Auth,p:Option<&Policy>,request:&mut Request)->ApiResult<()> {
 crate::lan_bridges::access(s,a,request).await?;
 let path=request.uri().path().to_owned();let parts=path.trim_matches('/').split('/').map(decode).collect::<Vec<_>>();
 if a.role=="external"&&!allowed(&path,request.method().as_str()){return Err(ApiError::forbidden());}
 if parts.len()>=2{let kind=match parts[0].as_str(){"zones"=>Some("zone"),"outputs"=>Some("endpoint"),"devices" if parts[1]!="register"=>Some("device"),_=>None};if let Some(kind)=kind{scope::check(s,a,kind,&parts[1]).await?;}}
 if parts.len()>=3&&parts[0]=="endpoints"&&parts[1]=="browser"{scope::check(s,a,"endpoint",&parts[2]).await?;}
 if let Some(p)=p{
  if url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes()).any(|(k,_)|k=="download_id"){return Err(ApiError::forbidden());}
  if parts.len()>=4&&parts[0]=="library"&&parts[1]=="catalog"{scope::catalog(s,p,&parts[2],&parts[3]).await?;}
  if parts.len()>=3&&parts[0]=="library"&&parts[1]=="tracks"&&parts.get(3).map(String::as_str)!=Some("stats"){scope::track(s,p,&parts[2]).await?;}
  if parts.len()==2&&matches!(parts[0].as_str(),"media"|"pcm"|"native-pcm"|"native-flac"|"mobile-opus"|"transcode"){scope::track(s,p,&parts[1]).await?;}
  if parts.len()>=2&&parts[0]=="providers"&&!p.providers.contains(&parts[1]){return Err(ApiError::forbidden());}
 }
 if !matches!(request.method().as_str(),"GET"|"HEAD")&&request.headers().get("content-type").is_some_and(|v|v.as_bytes().starts_with(b"application/json")){
  let bytes=to_bytes(std::mem::replace(request.body_mut(),Body::empty()),4*1024*1024+65536).await.map_err(|_|ApiError::bad("请求超过大小限制"))?;
  if let Ok(value)=serde_json::from_slice::<Value>(&bytes){body_access(s,a,p,&path,&value).await?;}*request.body_mut()=Body::from(bytes);
 }Ok(())
}
pub async fn boundary(State(s):State<AppState>,mut request:Request,next:Next)->Response {
 if matches!(request.uri().path(),"/auth/login"|"/status"|"/setup"){return next.run(request).await;}
 let token=request.headers().get("authorization").and_then(|v|v.to_str().ok()).and_then(|v|v.strip_prefix("Bearer "));
 let mut auth=if let Some(token)=token{crate::auth::authenticate(&s,token).await.ok()}else{None};let bearer=auth.is_some();
 if auth.is_none(){let ticket=url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes()).find(|(k,_)|k=="ticket").map(|(_,v)|v.into_owned());
  if let Some(ticket)=ticket{let owner=s.tickets.lock().await.get(&crate::auth::hash(&ticket)).filter(|t|t.expires>=chrono::Utc::now().timestamp()).and_then(|t|t.owner.clone());if let Some(owner)=owner{auth=crate::auth::authenticate_hash(&s,owner).await.ok();}}
 }
 let Some(mut a)=auth else{return next.run(request).await;};let credential_change=matches!(request.uri().path(),"/auth/account"|"/auth/totp/start"|"/auth/totp/confirm"|"/auth/totp/disable");let _lease=if !credential_change{Some(super::leases::read(&s,&a.id).await)}else{None};if _lease.is_some(){a=match crate::auth::authenticate_hash(&s,a.session_hash.clone()).await{Ok(a)=>a,Err(e)=>return e.into_response()};}let p=match policy(&s,&a).await{Ok(p)=>p,Err(e)=>return e.into_response()};
 if let Err(e)=access(&s,&a,p.as_ref(),&mut request).await{return e.into_response();}
 // Tickets remain restricted capabilities, never generic API authentication.
 if bearer{request.extensions_mut().insert(a.clone());}
 let json_safe=!request.uri().path().starts_with("/auth/");
 let media=matches!(request.uri().path().trim_start_matches('/').split('/').next(),Some("media"|"pcm"|"native-pcm"|"native-flac"|"mobile-opus"|"transcode"));
 let mut response=scope::REQUEST.scope((a.clone(),p.clone()),next.run(request)).await;
 if json_safe&&response.headers().get("content-type").is_some_and(|v|v.as_bytes().starts_with(b"application/json")){if let Some(p)=p.as_ref(){let(parts,body)=response.into_parts();let value=match to_bytes(body,8*1024*1024).await{Ok(bytes)=>serde_json::from_slice::<Value>(&bytes).ok(),Err(_)=>None};let Some(value)=value else{return ApiError::bad("响应超过账号读取限制").into_response();};let value=match scope::sanitize(&s,p,value).await{Ok(v)=>v,Err(e)=>return e.into_response()};response=Response::from_parts(parts,Body::from(serde_json::to_vec(&value).unwrap()));response.headers_mut().remove("content-length");}}

 if media {if let Some(p)=p{return bandwidth::wrap(s,a,p,response);}}response
}
