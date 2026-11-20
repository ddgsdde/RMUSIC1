use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use axum::{body::Bytes,extract::{Path,Query,State},http::{header,HeaderMap,StatusCode},response::{IntoResponse,Response},Json};
use rmusic_audio::impulse::{ConvolutionConfig,ImpulseResponse,MAX_WAV_BYTES,MAX_BANK_BYTES,MAX_BANK_COEFFICIENTS};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use std::{collections::BTreeSet,fs,io::Write,path::{Path as FsPath,PathBuf}};

const MAX_ASSETS:usize=128;
const MAX_STORAGE:u64=256*1024*1024;
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct ImpulseAsset {
    pub id:String,
    pub name:String,
    pub bytes:u64,
    pub sample_rate:u32,
    pub channels:usize,
    pub frames:usize,
    pub channel_mask:u32,
    pub created_at:i64,
}
#[derive(Deserialize)]
pub struct ImportQuery {name:Option<String>}
#[derive(Deserialize,Default)]
pub struct FileQuery {ticket:Option<String>,zone_id:Option<String>,selection:Option<String>}
#[derive(Serialize)]
pub struct ImpulseDownload {asset:ImpulseAsset,api_path:String}
#[derive(Serialize)]
pub struct ImpulseManifest {items:Vec<ImpulseDownload>,expires_at:i64}
pub(crate) fn configuration(dsp:&Value)->ApiResult<Option<ConvolutionConfig>> {
    let Some(value)=dsp.get("convolution").filter(|value|!value.is_null()) else{return Ok(None);};
    if dsp.get("enabled").is_some_and(|value|!value.is_boolean()){return Err(ApiError::bad("DSP enabled 必须为布尔值"));}
    let selection:ConvolutionConfig=serde_json::from_value(value.clone()).map_err(|_|ApiError::bad("卷积配置须只包含 response_ids 文件列表"))?;
    selection.validate().map_err(|error|ApiError::bad(error.to_string()))?;Ok(Some(selection))
}
fn fingerprint(selection:&ConvolutionConfig)->String {
    let mut ids=selection.response_ids.clone();ids.sort();format!("{:x}",Sha256::digest(ids.join("\n").as_bytes()))
}
fn ticket_scope(zone:&str,selection:&str,id:&str)->String{format!("impulse:{zone}:{selection}:{id}")}
pub(crate) async fn validate_selection(s:&AppState,selection:&ConvolutionConfig)->ApiResult<Vec<ImpulseAsset>> {
    selection.validate().map_err(|error|ApiError::bad(error.to_string()))?;
    let lock=s.impulse_lock.clone().lock_owned().await;let root=root(s);let ids=selection.response_ids.clone();
    tokio::task::spawn_blocking(move||{
        let _lock=lock;let(mut rates,mut bytes,mut coefficients,mut assets)=(BTreeSet::new(),0_u64,0usize,Vec::new());
        for id in ids {
            let asset=record(&root,&id)?;
            if !rates.insert(asset.sample_rate){return Err(ApiError::bad("响应组每个采样率只能选择一个文件"));}
            bytes+=asset.bytes;coefficients+=asset.frames*asset.channels;
            if bytes>MAX_BANK_BYTES as u64||coefficients>MAX_BANK_COEFFICIENTS{return Err(ApiError::bad("响应组超过 32 MiB 原文件或 1048576 系数的加载预算"));}
            contents(&root,&asset)?;assets.push(asset);
        }Ok(assets)
    }).await.map_err(internal)?
}
pub(crate) async fn render_response(s: &AppState, selection: &ConvolutionConfig, rate: u32, channels: u16) -> ApiResult<ImpulseResponse> {
    let root = root(s);
    let ids = selection.response_ids.clone();
    let lock = s.impulse_lock.clone().lock_owned().await;
    tokio::task::spawn_blocking(move || {
        let _lock = lock;
        for id in ids {
            let asset = record(&root, &id)?;
            if asset.sample_rate != rate { continue; }
            let response = ImpulseResponse::from_wav(&contents(&root, &asset)?).map_err(internal)?;
            response.validate_output(rate, channels, Some(if channels == 1 { 4 } else { 3 }))
                .map_err(|error| ApiError::bad(error.to_string()))?;
            return Ok(response);
        }
        Err(ApiError::bad(format!("卷积响应组缺少 {} Hz 的滤波器", rate)))
    }).await.map_err(internal)?
}
fn internal(error:impl Into<anyhow::Error>)->ApiError{error.into().into()}
fn valid_id(id:&str)->bool{id.len()==64&&id.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))}
fn require_id(id:&str)->ApiResult<()>{if valid_id(id){Ok(())}else{Err(ApiError::bad("脉冲响应 ID 无效"))}}
fn root(s:&AppState)->PathBuf{s.config.data.join("dsp/impulses")}
fn record(root:&FsPath,id:&str)->ApiResult<ImpulseAsset>{
    require_id(id)?;
    let data=fs::read(root.join(format!("{id}.json"))).map_err(|e|if e.kind()==std::io::ErrorKind::NotFound{ApiError::missing()}else{internal(e)})?;
    let result:ImpulseAsset=serde_json::from_slice(&data).map_err(internal)?;
    if result.id!=id||result.bytes>MAX_WAV_BYTES as u64||result.frames==0||!(1..=32).contains(&result.channels)
        ||result.frames.checked_mul(result.channels).is_none_or(|n|n>rmusic_audio::convolution::MAX_COEFFICIENTS)||result.sample_rate<8000 {
        return Err(internal(anyhow::anyhow!("invalid stored impulse metadata")));
    }Ok(result)
}
fn contents(root:&FsPath,asset:&ImpulseAsset)->ApiResult<Vec<u8>> {
    let path=root.join(format!("{}.wav",asset.id));let size=fs::metadata(&path).map_err(internal)?.len();
    if size!=asset.bytes||size>MAX_WAV_BYTES as u64 {return Err(internal(anyhow::anyhow!("stored impulse size changed")));}
    let bytes=fs::read(path).map_err(internal)?;
    if format!("{:x}",Sha256::digest(&bytes))!=asset.id {return Err(internal(anyhow::anyhow!("stored impulse checksum changed")));}
    let parsed=ImpulseResponse::from_wav(&bytes).map_err(internal)?;
    if parsed.sample_rate()!=asset.sample_rate||parsed.channels()!=asset.channels||parsed.frames()!=asset.frames||parsed.channel_mask()!=asset.channel_mask {
        return Err(internal(anyhow::anyhow!("stored impulse metadata does not match samples")));
    }Ok(bytes)
}
fn records(root:&FsPath)->ApiResult<Vec<ImpulseAsset>> {
    if !root.exists(){return Ok(vec![]);}
    let mut records=Vec::new();
    for entry in fs::read_dir(root).map_err(internal)? {
        let path=entry.map_err(internal)?.path();
        if path.extension().is_some_and(|e|e=="json") {
            if let Some(id)=path.file_stem().and_then(|v|v.to_str()).filter(|v|valid_id(v)){records.push(record(root,id)?);}
        }
    }
    records.sort_by(|a,b|a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));Ok(records)
}
fn atomic_write(root:&FsPath,name:&str,bytes:&[u8])->ApiResult<()> {
    let staging=root.join(".staging");fs::create_dir_all(&staging).map_err(internal)?;
    let path=staging.join(format!("{}.partial",uuid::Uuid::new_v4()));
    let result=(||->std::io::Result<()>{let mut file=fs::File::create(&path)?;file.write_all(bytes)?;file.sync_all()?;drop(file);fs::rename(&path,root.join(name))})();
    if result.is_err(){let _=fs::remove_file(&path);}result.map_err(internal)
}
fn store(root:&FsPath,name:String,bytes:&[u8])->ApiResult<ImpulseAsset> {
    let parsed=ImpulseResponse::from_wav(bytes).map_err(|e|ApiError::bad(format!("脉冲响应 WAV 无效：{e}")))?;
    let id=format!("{:x}",Sha256::digest(bytes));fs::create_dir_all(root).map_err(internal)?;
    if root.join(format!("{id}.json")).exists(){let existing=record(root,&id)?;contents(root,&existing)?;return Ok(existing);}
    let wave=root.join(format!("{id}.wav"));let existing=wave.exists();let(mut count,mut stored)=(0usize,0u64);
    for entry in fs::read_dir(root).map_err(internal)? {
        let entry=entry.map_err(internal)?;
        if entry.path().extension().is_some_and(|e|e=="wav") {count+=1;stored=stored.saturating_add(entry.metadata().map_err(internal)?.len());}
    }
    if count+usize::from(!existing)>MAX_ASSETS||stored.saturating_add(if existing{0}else{bytes.len() as u64})>MAX_STORAGE {
        return Err(ApiError(StatusCode::CONFLICT,"impulse_storage_limit".into(),"脉冲响应库已满，请删除未使用的响应".into()));
    }
    let asset=ImpulseAsset{id:id.clone(),name,bytes:bytes.len() as u64,sample_rate:parsed.sample_rate(),channels:parsed.channels(),frames:parsed.frames(),channel_mask:parsed.channel_mask(),created_at:chrono::Utc::now().timestamp_millis()};
    if existing {contents(root,&asset)?;}else{atomic_write(root,&format!("{id}.wav"),bytes)?;}
    atomic_write(root,&format!("{id}.json"),&serde_json::to_vec(&asset).map_err(internal)?)?;Ok(asset)
}
pub async fn list(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>> {
    a.require("read")?;let lock=s.impulse_lock.clone().lock_owned().await;let root=root(&s);
    let items=tokio::task::spawn_blocking(move||{let _lock=lock;records(&root)}).await.map_err(internal)??;
    Ok(Json(json!({"items":items,"limits":{"max_files":MAX_ASSETS,"max_storage_bytes":MAX_STORAGE,"max_upload_bytes":MAX_WAV_BYTES}})))
}
pub async fn import(State(s):State<AppState>,a:Auth,Query(q):Query<ImportQuery>,bytes:Bytes)->ApiResult<Json<ImpulseAsset>> {
    a.require("control")?;let name=q.name.unwrap_or_else(||"impulse.wav".into());let name=name.trim().to_owned();
    if name.is_empty()||name.chars().count()>120||name.chars().any(char::is_control){return Err(ApiError::bad("响应名称须为 1 到 120 个字符，且不含控制字符"));}
    let lock=s.impulse_lock.clone().lock_owned().await;let root=root(&s);
    Ok(Json(tokio::task::spawn_blocking(move||{let _lock=lock;store(&root,name,&bytes)}).await.map_err(internal)??))
}
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<ImpulseAsset>> {
    a.require("read")?;require_id(&id)?;let lock=s.impulse_lock.clone().lock_owned().await;let root=root(&s);
    Ok(Json(tokio::task::spawn_blocking(move||{let _lock=lock;record(&root,&id)}).await.map_err(internal)??))
}
pub async fn file(State(s):State<AppState>,headers:HeaderMap,Query(query):Query<FileQuery>,Path(id):Path<String>)->ApiResult<Response> {
    let scope=if let(Some(zone),Some(tag))=(&query.zone_id,&query.selection){
        s.zones.read().await.get(zone).and_then(|zone|configuration(&zone.dsp).ok().flatten()).filter(|selection|selection.response_ids.contains(&id)&&fingerprint(selection)==*tag).map(|_|ticket_scope(zone,tag,&id))
    }else{None};
    let ticket=crate::media::TicketQuery{ticket:scope.as_ref().and(query.ticket)};
    crate::media::authorize(&s,&headers,&ticket,scope.as_deref().unwrap_or("")).await?;
    read_file(s,id).await
}
pub(crate) async fn read_file(s:AppState,id:String)->ApiResult<Response> {
    require_id(&id)?;let lock=s.impulse_lock.clone().lock_owned().await;let root=root(&s);let etag=format!("\"{id}\"");
    let bytes=tokio::task::spawn_blocking(move||{let _lock=lock;contents(&root,&record(&root,&id)?)}).await.map_err(internal)??;
    Ok(([(header::CONTENT_TYPE,"audio/wav".to_owned()),(header::CONTENT_LENGTH,bytes.len().to_string()),(header::CACHE_CONTROL,"private, no-store".into()),(header::ETAG,etag)],bytes).into_response())
}
pub async fn manifest(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(value):Json<Value>)->ApiResult<Json<ImpulseManifest>> {
    a.require("control")?;let expected=configuration(&json!({"convolution":value}))?.ok_or_else(||ApiError::bad("请选择响应文件"))?;
    let _zone=s.zone_lock.lock().await;let zone=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;
    let changed=||ApiError(StatusCode::CONFLICT,"impulse_selection_changed".into(),"音区响应组已改变，请重新读取当前设置".into());
    let selection=configuration(&zone.dsp)?.ok_or_else(changed)?;let tag=fingerprint(&selection);
    if tag!=fingerprint(&expected){return Err(changed());}
    let assets=validate_selection(&s,&selection).await?;let mut items=Vec::new();
    for asset in assets {
        let token=s.ticket_for(ticket_scope(&id,&tag,&asset.id),300,Some(a.session_hash.clone())).await;
        let query=url::form_urlencoded::Serializer::new(String::new()).append_pair("ticket",&token).append_pair("zone_id",&id).append_pair("selection",&tag).finish();
        let api_path=format!("api/v1/dsp/impulses/{}/file?{query}",asset.id);items.push(ImpulseDownload{asset,api_path});
    }Ok(Json(ImpulseManifest{items,expires_at:chrono::Utc::now().timestamp()+300}))
}
pub async fn delete(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<StatusCode> {
    a.require("control")?;require_id(&id)?;let zones=s.zone_lock.clone().lock_owned().await;
    if s.zones.read().await.values().any(|z|z.dsp["convolution"]["filter_id"].as_str()==Some(id.as_str())||z.dsp["convolution"]["response_ids"].as_array().is_some_and(|ids|ids.iter().any(|value|value.as_str()==Some(id.as_str())))) {
        return Err(ApiError(StatusCode::CONFLICT,"impulse_in_use".into(),"请先从音区移除此脉冲响应".into()));
    }
    let lock=s.impulse_lock.clone().lock_owned().await;let root=root(&s);
    tokio::task::spawn_blocking(move||->ApiResult<()>{
        let(_zones,_lock)=(zones,lock);
        if !root.join(format!("{id}.json")).exists()&&!root.join(format!("{id}.wav")).exists(){return Err(ApiError::missing());}
        for extension in ["wav","json"] {if let Err(error)=fs::remove_file(root.join(format!("{id}.{extension}"))){if error.kind()!=std::io::ErrorKind::NotFound{return Err(internal(error));}}}Ok(())
    }).await.map_err(internal)??;Ok(StatusCode::NO_CONTENT)
}
