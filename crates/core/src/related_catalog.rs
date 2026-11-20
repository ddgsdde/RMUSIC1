//! Explicit platform browsing anchored to a confirmed recording, never a name-only album merge.
use crate::{auth::Auth,error::{ApiError,ApiResult},models::Track,state::AppState};
use axum::{extract::{Path,Query,State},Json};
use serde::Deserialize;
use serde_json::{json,Value};

#[derive(Clone,Default,Deserialize)]
pub struct Options {#[serde(default)]pub skip_profile:bool,pub sort:Option<String>,pub kind:Option<String>,pub page:Option<u32>,#[serde(alias="artist_id")]pub artist_mid:Option<String>,pub artist_name:Option<String>,#[serde(default)]pub refresh:bool}
fn normalized(text:&str)->String {text.chars().filter(|c|!c.is_whitespace()).flat_map(char::to_lowercase).collect()}
fn version(track:&Track)->String {normalized(&format!("{} {}",track.title,track.provider_data["subtitle"].as_str().unwrap_or("")))}
pub(crate) fn same_edition(local:&Track,cloud:&Track)->bool {
    let tags=[(&local.title,&cloud.title),(&local.artist,&cloud.artist),(&local.album,&cloud.album)];
    tags.iter().all(|(a,b)|!a.trim().is_empty()&&!matches!(normalized(a).as_str(),"未知专辑"|"未知艺术家"|"unknown")&&normalized(a)==normalized(b))
        && local.duration.is_finite()&&local.duration>0.&&(local.duration-cloud.duration).abs()<=2.
        && (local.track_number==0||cloud.track_number==0||local.track_number==cloud.track_number)
        && (local.disc_number==0||cloud.disc_number==0||local.disc_number==cloud.disc_number)
        && ["live","remaster","remix","acoustic","instrumental","karaoke","cover","radioedit","现场","翻唱","重制","重录","伴奏","纯音乐","不插电"].iter().all(|tag|version(local).contains(tag)==version(cloud).contains(tag))
}
fn release_id(track:&Track)->Option<&str>{track.provider_data["release_id"].as_str().or_else(||track.provider_data["album_mid"].as_str())}
fn artist_ids(track:&Track)->Option<&Vec<Value>>{track.provider_data["artist_ids"].as_array().or_else(||track.provider_data["artist_mids"].as_array())}
fn invalid()->ApiError {ApiError::bad("音源目录与已确认的歌曲、专辑或歌手身份不一致")}
fn validate(provider:&str,seed:&Track,value:&Value,kind:&str,page:u32,artist:Option<&str>)->ApiResult<Vec<Track>> {
    if value["source"]!=provider||value["kind"]!=kind||value["page"]!=page {return Err(invalid());}
    let anchor:Track=serde_json::from_value(value["anchor"].clone()).map_err(|_|invalid())?;
    crate::plugin::validated_tracks(provider,&json!({"items":[anchor]}))?;
    if !(seed.source==provider&&seed.id==anchor.id||seed.source=="local"&&same_edition(seed,&anchor)) {return Err(invalid());}
    let id=value["collection"]["id"].as_str();
    let artists=value["artists"].as_array().ok_or_else(invalid)?;
    if artists.len()>32{return Err(invalid());}
    let tracks=crate::plugin::validated_tracks(provider,value)?;
    if tracks.len()>50||value["next_page"].as_u64().is_some_and(|next|next!=u64::from(page)+1){return Err(invalid());}
    if kind=="album" {
        if id.is_none()||id!=release_id(&anchor)||tracks.iter().any(|t|release_id(t)!=id){return Err(invalid());}
    }else if let Some(id)=id {
        if artist.is_some_and(|a|a!=id)||!artists.iter().any(|a|a["id"]==id||a["mid"]==id)||!artist_ids(&anchor).is_some_and(|a|a.iter().any(|a|a==id))
            || tracks.iter().any(|t|!artist_ids(t).is_some_and(|a|a.iter().any(|a|a==id))){return Err(invalid());}
    }else if !tracks.is_empty()||artists.len()<2 {return Err(invalid());}
    Ok(tracks)
}
pub async fn browse(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(q):Query<Options>)->ApiResult<Json<Value>> {
    a.require("read")?;
    let seed=crate::library::get_track(&s,id).await?;
    let provider=crate::connectors::metadata_provider(&s,&seed.source).await?;
    Ok(Json(browse_provider(&s,seed,&provider,q).await?))
}
pub(crate) async fn browse_provider(s:&AppState,seed:Track,provider:&str,q:Options)->ApiResult<Value> {
    if !crate::accounts::scope::online_allowed(provider){return Ok(json!({"matched":false,"items":[],"notice":"此账号未获该平台音源权限"}));}
    let sort=q.sort.as_deref().unwrap_or("default");
    let mut value=if q.kind.as_deref()==Some("album") && !["","default"].contains(&sort) {
        tokio::time::timeout(std::time::Duration::from_secs(40),crate::catalog_order::album(s,seed,provider,q)).await.map_err(|_|ApiError::bad("专辑排序准备超时，请稍后重试"))??
    }else{browse_provider_page(s,seed,provider,q).await?};
    let kind=value["kind"].as_str().unwrap_or("album");let manifest=crate::connectors::manifest(s,provider).await?;
    value["sorts"]=if kind=="artist" && manifest["capabilities"].as_array().is_some_and(|v|v.iter().any(|v|v=="catalog_order_v1")){json!(["default","hot","release_new","release_old"])}else if kind=="album"{json!(["default","title","title_desc","release_new","release_old","duration","duration_desc"])}else{json!(["default"])};
    let mut items=value["items"].take();items=s.db.read(move|c|{crate::catalog_order::decorate(c,&mut items)?;Ok(items)}).await?;value["items"]=items;Ok(value)
}
pub(crate) async fn browse_provider_page(s:&AppState,seed:Track,provider:&str,q:Options)->ApiResult<Value> {
    let kind=q.kind.unwrap_or_else(||"album".into());let page=q.page.unwrap_or(1);
    if !["album","artist"].contains(&kind.as_str())||!(1..=100000).contains(&page)||q.artist_mid.as_ref().is_some_and(|v|v.len()>128){return Err(ApiError::bad("目录分类、歌手或页码无效"));}
    let sort=q.sort.as_deref().unwrap_or("default");
    if !["","default","hot","release_new","release_old"].contains(&sort){return Err(ApiError::bad("平台不支持此排序"));}
    let artist=q.artist_mid.filter(|v|!v.is_empty());
    let manifest=crate::connectors::manifest(&s,&provider).await?;
    if !manifest["capabilities"].as_array().is_some_and(|c|c.iter().any(|c|c=="related_catalog_v1")){return Ok(json!({"matched":false,"items":[],"notice":"当前音源插件尚不支持已确认专辑／歌手目录，请更新插件。"}));}
    if !["","default"].contains(&sort) && !manifest["capabilities"].as_array().is_some_and(|v|v.iter().any(|v|v=="catalog_order_v1")){return Err(ApiError::bad("此插件尚未提供平台排序能力"));}
    let entity=if kind=="album" {seed.provider_data["release_id"].as_str().or_else(||seed.provider_data["album_mid"].as_str()).unwrap_or(&seed.id)}else{artist.as_deref().unwrap_or(&seed.id)};
    let key=if ["","default"].contains(&sort){format!("catalog:{provider}:{kind}:{entity}")}else{format!("catalog:{provider}:{kind}:{entity}:{sort}")};let gate=crate::media_assets::gate(s,&key).await;let _guard=gate.lock().await;
    let cached=if q.refresh{None}else{crate::catalog_cache::load_page(s,key.clone(),page).await?.filter(|value|validate(provider,&seed,value,&kind,page,artist.as_deref()).is_ok())};
    let fresh=cached.is_some() && crate::catalog_cache::page_fresh(s,key.clone(),page).await?;
    let epoch=crate::provider_session::start(s,provider).await?;
    let (mut value,updated)=if fresh {(cached.clone().unwrap(),false)}else{
        match s.plugin.call(&provider,"related_catalog",json!({"track":seed,"kind":kind,"page":page,"artist_id":artist,"artist_mid":artist,"artist_name":q.artist_name,"refresh":q.refresh,"sort":sort,"skip_profile":q.skip_profile})).await {
            Ok(value)=>(value,true),Err(error)=>if let Some(mut value)=cached{value["notice"]=json!("更新暂不可用，显示已保存目录");(value,false)}else{return Err(error);}
        }
    };
    crate::provider_session::validate(s,provider,epoch).await?;

    if value["source"]!=provider{return Err(invalid());}
    if value["matched"]!=true {return Ok(json!({"matched":false,"items":[],"notice":"无法唯一确认歌曲版本，保留本地专辑。"}));}
    let tracks=validate(&provider,&seed,&value,&kind,page,artist.as_deref())?;
    if value.get("profile").is_some() && (value["profile"]["id"]!=value["collection"]["id"] || value["profile"]["source"]!=provider) { return Err(invalid()); }
    if kind=="artist" && q.artist_name.as_ref().is_some_and(|name|value["collection"]["name"].as_str()!=Some(name)) { return Err(invalid()); }
    if updated {crate::catalog_cache::save_page(s,key,page,&value,q.refresh).await?;}
    // Same convention as cloud playlist browsing: index metadata only. No audio fetch/cache job.
    crate::plugin::import_mode_epoch(&s,&provider,&value,true,Some(epoch)).await?;
    let basis=if seed.source=="local"{"exact_tags_duration_unique_candidate"}else{"provider_id"};
    let anchor_id=value["anchor"]["id"].as_str().unwrap_or("").to_owned();
    let items=s.db.run(move|c|{
        let mut result=vec![];
        for cloud in tracks {
            let local=if cloud.id==anchor_id&&seed.source=="local"&&seed.available {Some(seed.clone())}else{
                let matches=c.prepare("SELECT * FROM tracks WHERE source='local' AND available=1 AND path IS NOT NULL AND recording_id=?1 LIMIT 2")?.query_map([&cloud.recording_id],|row|Ok((Track::from_row(row)?,row.get::<_,String>("path")?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                if matches.len()==1&&std::path::Path::new(&matches[0].1).is_file()&&same_edition(&matches[0].0,&cloud){Some(matches[0].0.clone())}else{None}
            };
            let mut track=local.unwrap_or(cloud);crate::media_assets::apply_stored_cover(c,&mut track)?;if let Some(url)=track.cover_url.as_ref(){track.cover_url=Some(crate::media_assets::register(c,url).unwrap_or_else(|_|url.clone()));}if !track.provider_data.is_object(){track.provider_data=json!({});}track.provider_data["catalog_location"]=json!(if track.source=="local"{"local"}else{"platform"});result.push(track);
        }
        Ok(result)
    }).await?;
    if let Some(artists)=value["artists"].as_array_mut(){for artist in artists{if artist["id"].is_null(){artist["id"]=artist["mid"].clone();}if artist["mid"].is_null(){artist["mid"]=artist["id"].clone();}}}
    value["items"]=serde_json::to_value(items).map_err(anyhow::Error::from)?;
    value["identity_basis"]=json!(basis);
    Ok(value)
}
