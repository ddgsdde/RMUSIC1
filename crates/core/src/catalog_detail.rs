//! Unified entity pages. Names are presentation, never cross-provider identity evidence.
use crate::{auth::Auth,error::{ApiError,ApiResult},models::Track,state::AppState};
use axum::{extract::{Path,Query,State},Json};
use rusqlite::{params,OptionalExtension};
use serde::Deserialize;
use serde_json::{json,Value};
use std::collections::BTreeMap;
#[derive(Default,Deserialize)]
pub struct Options {#[serde(default)]pub client_metadata:bool,pub sort:Option<String>,pub platform_sort:Option<String>,pub page:Option<u32>,pub local_page:Option<u32>,pub provider:Option<String>,pub remote:Option<bool>,#[serde(default)]pub refresh:bool}

// Only shared, unambiguous MusicBrainz entity assertions connect platforms. A collaborator's
// ID cannot leak into another performer's page; release identity is never inferred from a title.
pub(crate) fn direct_ids(kind:&str)->String {crate::catalog_aliases::direct(kind)}
pub(crate) fn membership(kind:&str)->String {
 let ids=direct_ids(kind);
 let role=if kind=="artist"{"performer"}else{"release"};let entity=if kind=="artist"{"artist"}else{"release"};
 format!("t.id IN ({ids} UNION SELECT b.track_id FROM music_assertions a JOIN music_entities e ON e.id=a.entity_id JOIN music_assertions b ON b.entity_id=e.id WHERE a.track_id=?2 AND e.namespace='musicbrainz' AND e.kind='{entity}' AND a.role='{role}' AND b.role='{role}' AND a.basis IN ('file_tags','manual') AND b.basis IN ('file_tags','manual') AND NOT EXISTS(SELECT 1 FROM music_assertions conflict JOIN music_entities ce ON ce.id=conflict.entity_id WHERE conflict.track_id=b.track_id AND ce.kind=e.kind AND conflict.role=b.role AND ce.namespace='musicbrainz' AND ce.id!=e.id AND (CASE conflict.basis WHEN 'manual' THEN 0 WHEN 'file_tags' THEN 1 ELSE 2 END)<=(CASE b.basis WHEN 'manual' THEN 0 WHEN 'file_tags' THEN 1 ELSE 2 END) AND (?3='' OR ce.title=?3)) AND (?3='' OR e.title=?3) AND NOT EXISTS(SELECT 1 FROM music_assertions x JOIN music_entities ex ON ex.id=x.entity_id WHERE x.track_id=?2 AND x.role='{role}' AND ex.namespace='musicbrainz' AND ex.kind='{entity}' AND ex.id!=e.id AND x.basis IN('manual','file_tags') AND (CASE x.basis WHEN 'manual' THEN 0 ELSE 1 END)<=(CASE a.basis WHEN 'manual' THEN 0 ELSE 1 END) AND (?3='' OR ex.title=?3)))")
}
pub async fn detail(State(s):State<AppState>,a:Auth,Path((kind,id)):Path<(String,String)>,Query(q):Query<Options>)->ApiResult<Json<Value>> {
 a.require("read")?;
 let order=crate::accounts::scope::stats_sql(&crate::library_query::order(q.sort.as_deref(),false,"t.disc_number,t.track_number,t.title")?);
 let permission=crate::accounts::scope::predicate("t");let preferred=if a.role=="external"{"1"}else{crate::library::PREFERRED_COPY};
 let page=q.page.unwrap_or(1);let local_page=q.local_page.unwrap_or(1);
 let local_size=if kind=="album"{100}else{50};
 if !["artist","album"].contains(&kind.as_str())||id.len()>256||page==0||page>100000||local_page==0||local_page>100000{return Err(ApiError::bad("资料分类或页码无效"));}
 if kind=="album"&&!id.starts_with("release:") {
  let identity=id.clone();let visible=crate::accounts::scope::predicate("t");
  let variants=s.db.read(move|c|Ok(c.query_row(&format!("SELECT COUNT(DISTINCT COALESCE(json_extract(t.provider_data,'$.release_id'),json_extract(t.provider_data,'$.album_mid'))) FROM tracks t WHERE t.album_id=?1 AND {visible}"),[identity],|r|r.get::<_,i64>(0))?)).await?;
  if variants>1{return Err(ApiError::bad("此同名专辑包含不同发行版本，请从对应歌曲的专辑名称进入。"));}
 }
 if kind=="album" && q.remote!=Some(false) {crate::artist_albums::ensure_release(&s,&id,page,q.refresh).await?;}
 let k=kind.clone();let identity=id.clone();
 let found=s.db.read(move|c|{
  let _=crate::catalog_aliases::component(c,&k,&identity)?;
  let direct=format!("t.id IN ({})",direct_ids(&k));
  let seed=c.query_row(&format!("SELECT t.* FROM tracks t WHERE {direct} AND {permission} ORDER BY (t.source='local'),t.available DESC,t.id LIMIT 1"),[&identity],Track::from_row).optional()?;
  let Some(seed)=seed else{return Ok(None);};
  let name=if k=="artist"{c.query_row("SELECT name FROM track_artists WHERE artist_id=?1 ORDER BY name LIMIT 1",params![identity],|r|r.get::<_,String>(0))?}else{if seed.album.trim().is_empty(){"未提供专辑信息".into()}else{seed.album.clone()}};
  let condition=format!("({}) AND {permission}",membership(&k));
  let visible=format!("{condition} AND t.available=1 AND {} AND {}",crate::library::COLLECTION_VISIBILITY,preferred);
  let total=c.query_row(&format!("SELECT COUNT(*) FROM tracks t WHERE {visible}"),params![identity,seed.id,name],|r|r.get::<_,i64>(0))?;
  let local=c.prepare(&format!("SELECT t.* FROM tracks t WHERE {visible} ORDER BY {order} LIMIT {local_size} OFFSET ?4"))?.query_map(params![identity,seed.id,name,(local_page-1)*local_size],Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
  // One anchor per provider, bounded by installed adapters. Do not scan or load the whole entity.
  let ids=c.prepare(&format!("SELECT MIN(t.id) FROM tracks t WHERE {condition} GROUP BY t.source ORDER BY t.source LIMIT 16"))?.query_map(params![identity,seed.id,name],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
  let anchors=ids.into_iter().map(|id|c.query_row("SELECT * FROM tracks WHERE id=?1",[id],Track::from_row)).collect::<rusqlite::Result<Vec<_>>>()?;
  Ok(Some((seed,name,local,total,anchors)))
 }).await?;
 let (seed,name,local,total,anchors)=match found {Some(value)=>value,None=>{
  if kind=="album" {if let Some(mut value)=crate::artist_albums::empty_release(&s,&id).await? {if q.remote==Some(false){value["platforms"]=json!([]);}return Ok(Json(value));}}
  return Err(ApiError::missing());
 }};
 let mut local=serde_json::to_value(local).map_err(anyhow::Error::from)?;
 local=s.db.read(move|c|{crate::catalog_order::decorate(c,&mut local)?;Ok(local)}).await?;
 let providers=if q.remote==Some(false){vec![]}else{crate::connectors::ordered_metadata_providers(&s).await.into_iter().filter(|provider|crate::accounts::scope::online_allowed(provider)).collect::<Vec<_>>()};
 if q.provider.as_ref().is_some_and(|p|!p.is_empty()&&!providers.contains(p)){return Err(ApiError::bad("资料平台未启用"));}
 let alias_ids={let kind=kind.clone();let id=id.clone();s.db.read(move|c|crate::catalog_aliases::component(c,&kind,&id)).await?};
 let delegated=if q.client_metadata{crate::catalog_cache::profile(&s,&kind,&id).await?["profile"]["_client_qqmusic_profile"].clone()}else{json!({})};
 let mut offers=json!({});
 let mut profile=json!({"name":name});let mut provenance=BTreeMap::<String,String>::new();let mut platforms=vec![];
 for provider in providers.into_iter().take(16) {
  let candidate=anchors.iter().find(|t|t.source==provider).or_else(||if seed.source=="local"{Some(&seed)}else{None});
  let Some(candidate)=candidate else{continue;};
  // Related-catalog performs strict recording and entity checks before any indexing.
  let artist=if kind=="artist" {alias_ids.iter().find_map(|alias|{let credits=crate::artist_credits::credits(&candidate.source,&candidate.artist,&candidate.provider_data);credits.iter().find(|v|&v.id==alias).and_then(|v|provider_artist(candidate,alias,&v.name))}).or_else(||provider_artist(candidate,&id,&name))}else{None};
  if kind=="artist"&&candidate.source!="local"&&artist.is_none(){continue;}
  let opts=crate::related_catalog::Options {kind:Some(kind.clone()),page:Some(if q.provider.as_ref().is_none_or(|p|p.is_empty()||p==&provider){page}else{1}),artist_name:if kind=="artist"&&artist.is_none(){Some(name.clone())}else{None},artist_mid:artist,refresh:q.refresh,sort:q.platform_sort.clone(),skip_profile:q.client_metadata&&provider=="qqmusic"};
  match platform_page(&s,candidate.clone(),&provider,opts).await {
   Ok(mut value) if value["matched"]==true=>{
    if q.client_metadata&&provider=="qqmusic" {value["profile"]=delegated.clone();}
    if q.refresh {if let Some(url)=value["profile"]["image_url"].as_str().filter(|s|s.starts_with("https://")) {
      let url=url.to_owned();let key=crate::auth::hash(&url);let _=s.db.run({let url=url.clone();move|c|{crate::media_assets::register(c,&url)?;Ok(())}}).await;
      let _=crate::media_assets::get(State(s.clone()),a.clone(),Path(key),Query(crate::media_assets::Options{refresh:true}),axum::http::HeaderMap::new()).await;
    }}
    if let Some(fields)=value["profile"].as_object(){for(field,v)in fields{if crate::catalog_cache::FIELDS.contains(&field.as_str()){if let Some(text)=v.as_str().filter(|v|!v.trim().is_empty()){let short:String=text.chars().take(8000).collect();if offers.get(field).is_none(){offers[field]=json!([]);}offers[field].as_array_mut().unwrap().push(json!({"source":provider,"value":short,"truncated":text.chars().count()>8000,"basis":value["identity_basis"]}));}}}}
    merge_profile(&mut profile,&mut provenance,&value["profile"],&provider);
    platforms.push(json!({"source":provider,"name":crate::connectors::manifest(&s,&provider).await.ok().and_then(|m|m["name"].as_str().map(str::to_owned)).unwrap_or(provider.clone()),"page":value["page"],"total":value["total"],"items":value["items"],"next_page":value["next_page"],"collection":value["collection"],"notice":value["notice"],"profile":value["profile"],"sorts":value["sorts"],"sort":value["sort"],"identity_basis":value["identity_basis"]}));
   },
   Ok(value)=>{platforms.push(json!({"source":provider,"items":[],"notice":value["notice"]}));},
   Err(error)=>{platforms.push(json!({"source":provider,"items":[],"notice":error.2}));}
  }
 }
 if !platforms.is_empty(){profile["_alias_members"]=json!(alias_ids);profile["_provenance"]=json!(provenance);profile["_alternatives"]=offers;crate::catalog_cache::save(&s,&kind,&id,profile.clone()).await?;}
 let saved=crate::catalog_cache::profile(&s,&kind,&id).await?;
 if saved["profile"].as_object().is_some_and(|v|!v.is_empty()){profile=saved["profile"].clone();profile["name"]=json!(name);}
 let (local,platforms)=s.db.read(move|c|{crate::media_assets::normalize_entries(c,&mut local);for platform in &mut platforms{crate::media_assets::normalize_entries(c,&mut platform["items"]);}Ok((local,platforms))}).await?;
 Ok(Json(json!({"id":id,"kind":kind,"name":name,"anchor":seed,"profile":profile,"profile_revision":saved["revision"],"manual_fields":saved["manual"],"metadata_updated_at":saved["updated_at"],"provenance":saved["provenance"],"field_sources":saved["field_sources"],"local":{"items":local,"total":total,"page":local_page,"next_page":if i64::from(local_page)*i64::from(local_size)<total{Some(local_page+1)}else{None}},"platforms":platforms,"page":page})))
}

// Entity album pages combine two bounded adapter pages into a 100-track view.
async fn platform_page(s:&AppState,seed:Track,provider:&str,mut q:crate::related_catalog::Options)->ApiResult<Value>{
 if q.kind.as_deref()!=Some("album"){return crate::related_catalog::browse_provider(s,seed,provider,q).await;}
 let page=q.page.unwrap_or(1);q.page=Some((page-1)*2+1);
 let mut value=crate::related_catalog::browse_provider(s,seed.clone(),provider,q.clone()).await?;
 if let Some(next)=value["next_page"].as_u64(){
  q.page=Some(next as u32);
  let second=crate::related_catalog::browse_provider(s,seed,provider,q).await?;
  if let Some(items)=second["items"].as_array(){value["items"].as_array_mut().unwrap().extend(items.iter().cloned());}
  value["next_page"]=if second["next_page"].is_number(){json!(page+1)}else{Value::Null};
 }
 value["page"]=json!(page);Ok(value)
}
pub(crate) fn provider_artist(t:&Track,id:&str,name:&str)->Option<String>{
 let credits=crate::artist_credits::credits(&t.source,&t.artist,&t.provider_data);
 let person=credits.iter().find(|c|c.id==id).or_else(||credits.iter().find(|c|c.name==name)).or_else(||if credits.len()==1{credits.first()}else{None})?;
 // Membership already established entity identity. A single credit also permits a verified alias; multiple unmatched credits stay unresolved.
 let data=&t.provider_data;
 if let Some(rows)=data["artist_credits"].as_array(){if let Some(v)=rows.iter().find(|v|v["name"].as_str()==Some(&person.name)){return v["id"].as_str().map(str::to_owned);}}
 if let Some(ids)=data["artist_mids"].as_array(){let names:Vec<_>=t.artist.split(" / ").collect();if ids.len()==names.len(){if let Some(i)=names.iter().position(|n|*n==person.name){return ids[i].as_str().map(str::to_owned);}}}
 for key in ["provider_metadata","qqmusic_metadata"] {let m=&data[key];if m["matched"]==true&&m["source"]==t.source {if let Some(v)=m["artists"].as_array().and_then(|v|v.iter().find(|v|v["name"].as_str()==Some(&person.name))){return v["id"].as_str().or_else(||v["mid"].as_str()).map(str::to_owned);}}}None
}
fn merge_profile(target:&mut Value,provenance:&mut BTreeMap<String,String>,incoming:&Value,provider:&str){
 if let Some(fields)=incoming.as_object(){for(key,value)in fields{if ["name","id","source"].contains(&key.as_str()){continue;}
 let Some(text)=value.as_str().filter(|s|!s.trim().is_empty()&&s.len()<=80000) else{continue;};
 if ["image_url","url"].contains(&key.as_str())&&(!text.starts_with("https://")||text.len()>2048){continue;}
 if target.get(key).is_none_or(|v|v.is_null()||v.as_str().is_some_and(|s|s.is_empty())){target[key]=value.clone();provenance.insert(key.clone(),provider.into());}
 }}
}
#[cfg(test)]mod tests{use super::*;
 #[test]fn confirmed_entity_alias_selects_only_a_single_provider_credit(){
 let make=|artist:&str,data:Value|Track{id:"second:song".into(),title:"Song".into(),artist:artist.into(),album:"Album".into(),album_artist:artist.into(),duration:100.,track_number:1,disc_number:1,source:"second".into(),format:"unknown".into(),sample_rate:0,bit_depth:0,channels:2,bitrate:0,cover_url:None,recording_id:"rec".into(),available:true,created_at:0,integrated_lufs:None,provider_data:data};
 let alias=make("이지은",json!({"artist_credits":[{"id":"second-iu","name":"이지은"}]}));assert_eq!(provider_artist(&alias,"first-iu","IU"),Some("second-iu".into()));
 let duet=make("이지은 / Other",json!({"artist_credits":[{"id":"second-iu","name":"이지은"},{"id":"other","name":"Other"}]}));assert_eq!(provider_artist(&duet,"first-iu","IU"),None);
 }
 #[test]fn preferred_profile_fills_only_missing_fields(){let mut target=json!({"name":"Artist"});let mut sources=BTreeMap::new();merge_profile(&mut target,&mut sources,&json!({"description":"preferred","genre":"","image_url":"https://first"}),"first");merge_profile(&mut target,&mut sources,&json!({"description":"other","genre":"Pop","id":"foreign","company":{"bad":"object"},"url":"javascript:bad"}),"second");assert_eq!(target["description"],"preferred");assert_eq!(target["genre"],"Pop");assert!(target.get("id").is_none());assert!(target.get("company").is_none());assert!(target.get("url").is_none());assert_eq!(sources["genre"],"second");}
}
