//! Cached provider catalogues, account-bound mirrors and guarded three-way synchronization.
use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use axum::{extract::{Path,State},Json};
use rusqlite::{params,OptionalExtension,Connection};
use serde_json::{json,Value};
use std::collections::HashSet;

fn api(error:ApiError)->anyhow::Error {anyhow::anyhow!(error.2)}
pub async fn record_account(s:&AppState,provider:&str,health:&Value,allow_switch:bool)->ApiResult<String> {
    let context=health["account_context"].as_str().filter(|v|!v.is_empty()&&v.len()<=256).unwrap_or("public").to_owned();
    let key=provider.to_owned();let saved=context.clone();
    let login_pending=health["login_pending"]==true;
    let changed=s.db.run(move|c|{
        let old=c.query_row("SELECT context FROM provider_accounts WHERE provider=?1",[&key],|r|r.get::<_,String>(0)).optional()?;
        if old.as_deref()==Some("switching")&&!allow_switch&&login_pending{return Ok(false);}
        if old.as_deref()==Some(&saved){return Ok(false);}
        crate::provider_session::bump(c,&key)?;
        c.execute("INSERT INTO provider_accounts(provider,context,updated_at) VALUES(?1,?2,unixepoch()) ON CONFLICT(provider) DO UPDATE SET context=excluded.context,updated_at=excluded.updated_at",params![key,saved])?;
        Ok(true)
    }).await?;if changed{s.emit("plugins",json!({}));}Ok(context)
}
pub async fn switching(s:&AppState,provider:&str)->ApiResult<()> {
    let provider=provider.to_owned();s.db.run(move|c|{crate::provider_session::bump(c,&provider)?;c.execute("INSERT INTO provider_accounts(provider,context,updated_at) VALUES(?1,'switching',unixepoch()) ON CONFLICT(provider) DO UPDATE SET context='switching',updated_at=excluded.updated_at",[provider])?;Ok(())}).await?;s.emit("plugins",json!({}));Ok(())
}
async fn account(s:&AppState,provider:&str)->anyhow::Result<String> {
    let health=s.plugin.call(provider,"health",json!({})).await.map_err(api)?;
    if health["login_pending"]==true{anyhow::bail!("音源正在切换账号，暂停同步");}
    let context=record_account(s,provider,&health,false).await.map_err(api)?;
    if context=="anonymous"{anyhow::bail!("请先登录对应音源账号");}Ok(context)
}
pub async fn directory(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>> {
    a.require("read")?;if a.role=="external"{return Ok(Json(json!({"items":[]})));}
    let user=a.id.clone();let items=s.db.read(move|c|Ok(c.prepare("SELECT d.data,d.account FROM provider_playlist_directory d JOIN provider_accounts a ON a.provider=d.provider AND a.context=d.account WHERE d.user_id=?1 AND COALESCE((SELECT enabled FROM plugins WHERE id=d.provider),1)=1 ORDER BY json_extract(d.data,'$.name')")?.query_map([user],|r|{let mut value:Value=serde_json::from_str(&r.get::<_,String>(0)?).unwrap_or_default();value["account_context"]=json!(r.get::<_,String>(1)?);Ok(value)})?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    for provider in crate::connectors::providers(&s,"playlists").await {
        let user=a.id.clone();let source=provider.clone();let due=s.db.read(move|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM provider_accounts a WHERE a.provider=?2 AND a.context NOT IN ('anonymous','switching','public') AND COALESCE((SELECT MAX(d.updated_at) FROM provider_directory_refresh d WHERE d.user_id=?1 AND d.provider=a.provider AND d.account=a.context),0)<unixepoch()-300 AND NOT EXISTS(SELECT 1 FROM jobs j WHERE j.kind='playlist_directory' AND j.cursor=?1||':'||?2 AND j.updated_at>unixepoch()-300))",params![user,source],|r|r.get::<_,bool>(0))?)).await?;
        if due{crate::background::enqueue(&s,"playlist_directory",&format!("{}:{provider}",a.id),json!({"user_id":a.id,"provider":provider}),0).await?;}
    }
    Ok(Json(json!({"items":items})))
}
pub async fn refresh(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>> {
    a.require("read")?;for provider in crate::connectors::providers(&s,"playlists").await{crate::background::enqueue(&s,"playlist_directory",&format!("{}:{provider}",a.id),json!({"user_id":a.id,"provider":provider}),50).await?;}Ok(Json(json!({"status":"pending"})))
}
pub(crate) async fn refresh_directory(s:&AppState,job:&str,payload:&Value)->anyhow::Result<bool> {
    let provider=payload["provider"].as_str().ok_or_else(||anyhow::anyhow!("缺少音源"))?;
    crate::plugin::check_enabled(s,provider).await.map_err(api)?;let context=account(s,provider).await?;
    let mut entries=vec![];let mut page=1;
    loop{
        if !crate::background::disposition(s,job).await?{return Ok(false);}
        let result=s.plugin.call(provider,"playlists",json!({"collection":"all","page":page})).await.map_err(api)?;
        let items=result["items"].as_array().ok_or_else(||anyhow::anyhow!("音源歌单目录无效"))?;
        if items.iter().any(|v|v["source"]!=provider||v["id"].as_str().is_none_or(|id|id.is_empty()||id.len()>256)||v["name"].as_str().is_none()){anyhow::bail!("音源返回了不属于自己的歌单");}
        entries.extend(items.clone());if entries.len()>10000{anyhow::bail!("歌单目录超过一万项，未替换旧目录");}
        if result["next_page"].is_null(){break;}if result["next_page"].as_u64()!=Some(page+1){anyhow::bail!("音源歌单分页不完整");}page+=1;
        crate::background::progress(s,job,"刷新云端歌单目录",entries.len() as u64,0).await?;
    }
    if account(s,provider).await?!=context{anyhow::bail!("音源账号已切换，未保存其他账号的歌单");}
    let provider=provider.to_owned();let user=payload["user_id"].as_str().unwrap_or("").to_owned();
    s.db.run(move|c|{let tx=c.transaction()?;tx.execute("DELETE FROM provider_playlist_directory WHERE user_id=?1 AND provider=?2 AND account=?3",params![user,provider,context])?;
        for item in entries{tx.execute("INSERT INTO provider_playlist_directory(user_id,provider,account,remote_id,data,updated_at) VALUES(?1,?2,?3,?4,?5,unixepoch())",params![user,provider,context,item["id"].as_str().unwrap(),item.to_string()])?;}tx.execute("INSERT INTO provider_directory_refresh(user_id,provider,account,updated_at) VALUES(?1,?2,?3,unixepoch()) ON CONFLICT(user_id,provider,account) DO UPDATE SET updated_at=excluded.updated_at",params![user,provider,context])?;tx.commit()?;Ok(())}).await?;
    s.emit("playlists",json!({}));Ok(true)
}
pub fn schedule(c:&Connection,id:&str)->anyhow::Result<()> {
    if c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='playlist_sync' AND cursor=?1 AND status IN ('pending','running','interrupted','waiting','paused'))",[id],|r|r.get::<_,bool>(0))?{return Ok(());}
    c.execute("INSERT INTO jobs(id,kind,status,created_at,updated_at,cursor,payload,phase,priority) VALUES(?1,'playlist_sync','pending',unixepoch(),unixepoch(),?2,?3,'等待同步歌单',50)",params![uuid::Uuid::new_v4().to_string(),id,json!({"local_id":id}).to_string()])?;Ok(())
}
pub fn changed(c:&Connection,id:&str,value:&Value)->anyhow::Result<()> {
    if c.execute("UPDATE playlist_bindings SET local_version=local_version+1,status=CASE WHEN status='conflict' THEN status ELSE 'pending' END WHERE local_id=?1",[id])?==0{return Ok(());}
    c.execute("INSERT INTO playlist_operations(id,local_id,version,payload,created_at) SELECT ?2,local_id,local_version,?3,unixepoch() FROM playlist_bindings WHERE local_id=?1",params![id,uuid::Uuid::new_v4().to_string(),value.to_string()])?;schedule(c,id)
}
fn expected_account(value:&Value)->ApiResult<Option<String>> {
    match value.get("account_context") {
        None|Some(Value::Null)=>Ok(None),
        Some(Value::String(account)) if !account.is_empty()&&account.len()<=256=>Ok(Some(account.clone())),
        _=>Err(ApiError::bad("音源账号上下文无效，请刷新云端目录")),
    }
}
pub async fn bind(State(s):State<AppState>,a:Auth,Json(value):Json<Value>)->ApiResult<Json<Value>> {
    a.require("control")?;let provider=value["provider"].as_str().filter(|id|crate::connectors::valid_id(id)).ok_or_else(||ApiError::bad("音源无效"))?.to_owned();
    let remote=value["playlist_id"].as_str().filter(|id|!id.is_empty()&&id.len()<=256).ok_or_else(||ApiError::bad("歌单标识无效"))?.to_owned();
    let expected=expected_account(&value)?;
    let descriptor=crate::connectors::manifest(&s,&provider).await?;let write=descriptor["capabilities"].as_array().is_some_and(|c|c.iter().any(|v|v=="playlist_write_v1")&&c.iter().any(|v|v=="account_context_v1"));
    let id=s.db.run(move|c|{let tx=c.transaction()?;
        let (account,data)=tx.query_row("SELECT d.account,d.data FROM provider_playlist_directory d JOIN provider_accounts a ON a.provider=d.provider AND a.context=d.account WHERE d.user_id=?1 AND d.provider=?2 AND d.remote_id=?3 AND (?4 IS NULL OR d.account=?4)",params![a.id,provider,remote,expected],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?.ok_or_else(||anyhow::anyhow!("目录已变化，请刷新后重新选择歌单"))?;
        if let Some(id)=tx.query_row("SELECT local_id FROM playlist_bindings WHERE user_id=?1 AND provider=?2 AND remote_id=?3 AND account=?4",params![a.id,provider,remote,account],|r|r.get::<_,String>(0)).optional()?{return Ok(id);}
        let entry:Value=serde_json::from_str(&data)?;let name=entry["name"].as_str().unwrap_or("云端歌单");let id=uuid::Uuid::new_v4().to_string();
        let capabilities=if write&&entry["writable"]==true{entry["write_capabilities"].as_array().cloned().unwrap_or_default()}else{vec![]};
        tx.execute("INSERT INTO playlists(id,name,user_id,tracks) VALUES(?1,?2,?3,'[]')",params![id,name,a.id])?;
        tx.execute("INSERT INTO playlist_bindings(local_id,user_id,provider,remote_id,account,capabilities,local_baseline,baseline,next_due) VALUES(?1,?2,?3,?4,?5,?6,?7,?7,unixepoch()+3600)",params![id,a.id,provider,remote,account,json!(capabilities).to_string(),json!({"name":name,"track_ids":[]}).to_string()])?;
        schedule(&tx,&id)?;tx.commit()?;Ok(id)
    }).await?;s.emit("playlists",json!({}));Ok(Json(json!({"id":id,"status":"pending"})))
}
pub fn info(c:&Connection,id:&str)->anyhow::Result<Option<Value>> {
    Ok(c.query_row("SELECT provider,remote_id,status,error,last_sync,auto_sync,conflict,capabilities,account FROM playlist_bindings WHERE local_id=?1",[id],|r|Ok(json!({"provider":r.get::<_,String>(0)?,"playlist_id":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"error":r.get::<_,Option<String>>(3)?,"last_sync":r.get::<_,i64>(4)?,"auto_sync":r.get::<_,bool>(5)?,"conflict":r.get::<_,Option<String>>(6)?.and_then(|v|serde_json::from_str::<Value>(&v).ok()),"capabilities":serde_json::from_str::<Value>(&r.get::<_,String>(7)?).unwrap_or_default(),"account_context":r.get::<_,String>(8)?}))).optional()?)
}
fn detach(c:&Connection,id:&str)->anyhow::Result<Result<(),ApiError>> {
    if c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='playlist_sync' AND cursor=?1 AND status='running')",[id],|r|r.get::<_,bool>(0))?{return Ok(Err(ApiError(axum::http::StatusCode::CONFLICT,"sync_busy".into(),"歌单正在同步，请完成后再更换或解除绑定".into())));}
    c.execute("UPDATE jobs SET status='cancelled',updated_at=unixepoch() WHERE kind='playlist_sync' AND cursor=?1 AND status IN ('pending','waiting','interrupted','paused')",[id])?;
    c.execute("DELETE FROM playlist_bindings WHERE local_id=?1",[id])?;Ok(Ok(()))
}
pub async fn bind_favorites(State(s):State<AppState>,a:Auth,Json(value):Json<Value>)->ApiResult<Json<Value>> {
    a.require("control")?;
    let provider=value["provider"].as_str().filter(|id|crate::connectors::valid_id(id)).ok_or_else(||ApiError::bad("音源无效"))?.to_owned();
    let remote=value["playlist_id"].as_str().filter(|id|!id.is_empty()&&id.len()<=256).ok_or_else(||ApiError::bad("歌单标识无效"))?.to_owned();
    let manifest=crate::connectors::manifest(&s,&provider).await?;
    if !["playlist_write_v1","account_context_v1"].iter().all(|cap|manifest["capabilities"].as_array().is_some_and(|caps|caps.iter().any(|v|v==cap))){return Err(ApiError::bad("此音源不支持双向歌单同步"));}
    let expected=expected_account(&value)?;
    let takeover=value["take_over"]==true;
    let result=s.db.run(move|c|{let tx=c.transaction()?;let id=format!("favorites-{}",crate::auth::hash(&a.id));
        let entry=tx.query_row("SELECT d.account,d.data FROM provider_playlist_directory d JOIN provider_accounts a ON a.provider=d.provider AND a.context=d.account WHERE d.user_id=?1 AND d.provider=?2 AND d.remote_id=?3 AND (?4 IS NULL OR d.account=?4)",params![a.id,provider,remote,expected],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
        let Some((account,data))=entry else{return Ok(Err(ApiError::bad("目录已变化，请刷新后重新选择歌单")));};
        let entry:Value=serde_json::from_str(&data)?;let caps=entry["write_capabilities"].as_array().cloned().unwrap_or_default();
        if entry["writable"]!=true||!["add","remove"].iter().all(|cap|caps.iter().any(|v|v==cap)){return Ok(Err(ApiError::bad("请选择支持添加和删除歌曲的可写云端歌单")));}
        let previous=tx.query_row("SELECT local_id FROM playlist_bindings WHERE user_id=?1 AND provider=?2 AND remote_id=?3 AND account=?4",params![a.id,provider,remote,account],|r|r.get::<_,String>(0)).optional()?;
        if previous.as_deref()==Some(&id){return Ok(Ok(json!({"id":id,"sync":info(&tx,&id)?})));}
        if let Some(previous)=previous{
            if !takeover{return Ok(Err(ApiError(axum::http::StatusCode::CONFLICT,"binding_exists".into(),"目标已有本地同步副本；确认转移绑定后，原副本及其未同步修改会保留为本地内容".into())));}
            if let Err(error)=detach(&tx,&previous)?{return Ok(Err(error));}
        }
        if let Err(error)=detach(&tx,&id)?{return Ok(Err(error));}
        tx.execute("INSERT OR IGNORE INTO playlists(id,name,user_id,tracks) VALUES(?1,'我喜欢',?2,'[]')",params![id,a.id])?;
        let tracks:String=tx.query_row("SELECT tracks FROM playlists WHERE id=?1 AND user_id=?2",params![id,a.id],|r|r.get(0))?;
        tx.execute("INSERT INTO playlist_bindings(local_id,user_id,provider,remote_id,account,capabilities,baseline,local_baseline,next_due) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,unixepoch()+3600)",params![id,a.id,provider,remote,account,json!(caps).to_string(),json!({"name":entry["name"],"track_ids":[]}).to_string(),json!({"name":"我喜欢","track_ids":serde_json::from_str::<Value>(&tracks)?}).to_string()])?;
        schedule(&tx,&id)?;let sync=info(&tx,&id)?;tx.commit()?;Ok(Ok(json!({"id":id,"sync":sync})))
    }).await??;s.emit("playlists",json!({}));Ok(Json(result))
}
pub async fn unbind_favorites(State(s):State<AppState>,a:Auth)->ApiResult<axum::http::StatusCode>{
    a.require("control")?;s.db.run(move|c|{let tx=c.transaction()?;let id=format!("favorites-{}",crate::auth::hash(&a.id));if let Err(error)=detach(&tx,&id)?{return Ok(Err(error));}tx.commit()?;Ok(Ok(()))}).await??;
    s.emit("playlists",json!({}));Ok(axum::http::StatusCode::NO_CONTENT)
}
pub async fn request(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(value):Json<Value>)->ApiResult<Json<Value>> {
    a.require("control")?;
    if let Some(resolution)=value.get("resolve") {
        let use_cloud=match resolution.as_str() {
            Some("cloud")=>true,
            Some("local")=>false,
            _=>return Err(ApiError::bad("请选择保留 Core 或采用云端版本")),
        };
        let expected=value["conflict_id"].as_str().map(str::to_owned);
        resolve_conflict(&s,a.id,id,use_cloud,expected).await?;
    }else{
        s.db.run(move|c|{
            let tx=c.transaction()?;
            if !tx.query_row("SELECT EXISTS(SELECT 1 FROM playlist_bindings WHERE local_id=?1 AND user_id=?2)",params![id,a.id],|r|r.get::<_,bool>(0))? {
                return Ok(Err(ApiError::missing()));
            }
            if let Some(enabled)=value["auto_sync"].as_bool() {
                tx.execute("UPDATE playlist_bindings SET auto_sync=?2,next_due=unixepoch()+3600 WHERE local_id=?1",params![id,enabled])?;
            }else{schedule(&tx,&id)?;}
            tx.commit()?;
            Ok(Ok(()))
        }).await??;
    }
    s.emit("playlists",json!({}));
    Ok(Json(json!({"status":"pending"})))
}

fn stale_conflict()->ApiError {
    ApiError(axum::http::StatusCode::CONFLICT,"playlist_conflict_changed".into(),"同步冲突已有变化，请核对最新版本后重新确认".into())
}

async fn resolve_conflict(s:&AppState,user:String,id:String,use_cloud:bool,expected:Option<String>)->ApiResult<()> {
    let (binding,local)=load_binding(s,&id,Some(user.clone())).await?.ok_or_else(ApiError::missing)?;
    let previous=binding.conflict.as_ref().ok_or_else(stale_conflict)?;
    let displayed:Value=serde_json::from_str(previous).map_err(anyhow::Error::from)?;
    if expected.as_deref().is_some_and(|id|displayed["id"].as_str()!=Some(id)) {
        return Err(stale_conflict());
    }
    if !use_cloud&&binding.caps.is_empty(){return Err(ApiError::bad("该云端歌单为只读，请采用云端版本"));}
    crate::plugin::check_enabled(s,&binding.provider).await?;
    if account(s,&binding.provider).await?!=binding.account {
        return Err(ApiError::bad("请切回绑定的音源账号后重试"));
    }
    let generation=crate::provider_session::start(s,&binding.provider).await?;
    let cloud=snapshot(s,&binding,local["name"].as_str().unwrap_or("云端歌单")).await?;
    let result=s.db.run(move|c|{
        let tx=c.transaction()?;
        crate::provider_session::check(&tx,&binding.provider,generation)?;
        let current=tx.query_row(
            "SELECT b.conflict,p.name,p.tracks FROM playlist_bindings b JOIN playlists p ON p.id=b.local_id WHERE b.local_id=?1 AND b.user_id=?2",
            params![id,user],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)),
        ).optional()?;
        let Some((raw,name,tracks))=current else {return Ok(Err(ApiError::missing()));};
        if raw!=binding.conflict {return Ok(Err(stale_conflict()));}
        let local=json!({"name":name,"track_ids":serde_json::from_str::<Value>(&tracks)?});
        if local!=displayed["local"]||cloud["revision"]!=displayed["cloud"]["revision"] {
            let reason="歌单在确认前再次变化";
            let data=conflict_data(&binding,&local,&cloud,reason);
            store_conflict(&tx,&id,reason,&data)?;
            tx.commit()?;
            return Ok(Err(stale_conflict()));
        }
        if use_cloud {
            let old=ids(&local);
            let matched=provider_tracks(&tx,&binding.provider,&old)?;
            let remote=ids(&cloud);
            let favorites=id.starts_with("favorites-");
            let tracks=if favorites{favorites_order(&old,&matched,&remote,newest_first(&binding,&cloud).then_some(remote.as_slice()))}else{
                let mut tracks=remote;
                tracks.extend(old.into_iter().filter(|track|!matched.contains(track)));
                tracks
            };
            tx.execute("UPDATE playlists SET name=?2,tracks=?3 WHERE id=?1",
                params![id,if favorites{"我喜欢"}else{cloud["name"].as_str().unwrap_or("云端歌单")},json!(tracks).to_string()])?;
        }
        tx.execute("UPDATE playlist_bindings SET baseline=?2,local_baseline=?2,revision=?3,status='pending',conflict=NULL,error=NULL,local_version=local_version+1 WHERE local_id=?1",
            params![id,cloud.to_string(),cloud["revision"].as_str().unwrap_or("")])?;
        tx.execute("UPDATE jobs SET status='cancelled' WHERE kind='playlist_sync' AND cursor=?1 AND status='paused'",[&id])?;
        schedule(&tx,&id)?;
        tx.commit()?;
        Ok(Ok(()))
    }).await?;
    if result.is_err(){s.emit("playlists",json!({}));}
    result
}

struct Binding {id:String,provider:String,remote:String,account:String,caps:Vec<String>,baseline:Value,local_baseline:Value,version:i64,conflict:Option<String>}
fn ids(value:&Value)->Vec<String>{value["track_ids"].as_array().into_iter().flatten().filter_map(|v|v.as_str().map(String::from)).collect()}
fn strings(values:&[String])->HashSet<&str>{values.iter().map(String::as_str).collect()}
fn merge(base:&[String],local:&[String],remote:&[String])->Result<Vec<String>,String> {
    if local==base{return Ok(remote.to_vec());}if remote==base{return Ok(local.to_vec());}if local==remote{return Ok(local.to_vec());}
    let b=strings(base);let l=strings(local);let r=strings(remote);
    let keep=|id:&str|!b.contains(id)||(l.contains(id)&&r.contains(id));
    let order=|values:&[String]|values.iter().filter(|id|b.contains(id.as_str())&&keep(id)).cloned().collect::<Vec<_>>();
    let original=order(base);let left=order(local);let right=order(remote);
    if left!=original&&right!=original&&left!=right{return Err("本地与云端都调整了歌曲顺序".into());}
    let preferred=if right!=original{remote}else{local};let mut seen=HashSet::new();let mut result=vec![];
    for id in preferred.iter().chain(remote).chain(local){if keep(id)&&seen.insert(id){result.push(id.clone());}}
    Ok(result)
}
fn with_extras(local:&[String],matched:&HashSet<String>,merged:&[String])->Vec<String>{
    let mut next=merged.iter();let mut result=vec![];
    for id in local{if matched.contains(id){if let Some(id)=next.next(){result.push(id.clone());}}else{result.push(id.clone());}}
    result.extend(next.cloned());result
}
fn provider_tracks(c:&Connection,source:&str,ids:&[String])->anyhow::Result<HashSet<String>> {
    let mut tracks=HashSet::new();
    let mut belongs=c.prepare("SELECT source=?2 FROM tracks WHERE id=?1")?;
    for id in ids {
        if belongs.query_row(params![id,source],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false) {
            tracks.insert(id.clone());
        }
    }
    Ok(tracks)
}
fn merge_favorites(base:&[String],local:&[String],cloud:&[String])->Vec<String> {
    let baseline=strings(base);
    let local_set=strings(local);
    let cloud_set=strings(cloud);
    let mut seen=HashSet::new();
    cloud.iter().chain(local).filter(|id|{
        let id=id.as_str();
        (!baseline.contains(id)||(local_set.contains(id)&&cloud_set.contains(id)))&&seen.insert(id)
    }).cloned().collect()
}
fn newest_first(binding:&Binding,cloud:&Value)->bool {
    cloud["favorite_order"]=="newest_first" || (binding.provider=="qqmusic" && binding.remote=="liked")
}
fn favorites_order(local:&[String],matched:&HashSet<String>,incoming:&[String],cloud_recent:Option<&[String]>)->Vec<String> {
    let retained=strings(incoming);
    let existing=strings(local);
    let mut result=local.iter().filter(|id|!matched.contains(*id)||retained.contains(id.as_str()))
        .chain(incoming.iter().filter(|id|!existing.contains(id.as_str()))).cloned().collect::<Vec<_>>();
    if let Some(cloud)=cloud_recent {
        let synced=strings(cloud);
        // Favorites are stored oldest first. Keep the slots of any songs without a cloud match.
        let mut seen=HashSet::new();
        let mut ordered=cloud.iter().rev().filter(|id|retained.contains(id.as_str())&&seen.insert(id.as_str()));
        for id in &mut result {
            if synced.contains(id.as_str()) {
                if let Some(next)=ordered.next(){*id=next.clone();}
            }
        }
    }
    result
}
async fn snapshot(s:&AppState,b:&Binding,name:&str)->anyhow::Result<Value> {
    let mut value=s.plugin.call(&b.provider,"playlist_snapshot",json!({"playlist_id":b.remote})).await.map_err(api)?;
    validate_snapshot(s,b,&value).await?;
    if value["name"].as_str().is_none(){value["name"]=json!(name);}Ok(value)
}
async fn validate_snapshot(s:&AppState,b:&Binding,value:&Value)->anyhow::Result<()> {
    if value["playlist_id"]!=b.remote||value["revision"].as_str().is_none_or(str::is_empty)||!value["track_ids"].is_array(){anyhow::bail!("歌单快照身份／版本无效");}
    let tracks=crate::plugin::validated_tracks(&b.provider,value).map_err(api)?;
    if tracks.iter().map(|t|t.id.clone()).collect::<Vec<_>>()!=ids(value){anyhow::bail!("歌单快照曲目不完整");}
    crate::plugin::import(s,&b.provider,value).await.map_err(api)
}
fn conflict_data(b:&Binding,local:&Value,cloud:&Value,reason:&str)->Value {
    json!({"id":uuid::Uuid::new_v4().to_string(),"baseline":b.baseline,"local":local,"cloud":cloud,"reason":reason})
}
fn store_conflict(c:&Connection,id:&str,reason:&str,data:&Value)->anyhow::Result<()> {
    c.execute("UPDATE playlist_bindings SET status='conflict',error=?2,conflict=?3 WHERE local_id=?1",params![id,reason,data.to_string()])?;
    Ok(())
}
async fn conflict(s:&AppState,job:&str,b:&Binding,local:&Value,remote:&Value,reason:&str)->anyhow::Result<bool> {
    let data=conflict_data(b,local,remote,reason);
    let (id,job,reason)=(b.id.clone(),job.to_owned(),reason.to_owned());
    s.db.run(move|c|{
        let tx=c.transaction()?;
        store_conflict(&tx,&id,&reason,&data)?;
        tx.execute("UPDATE jobs SET status='paused',phase='等待处理歌单冲突',updated_at=unixepoch() WHERE id=?1",[job])?;
        tx.commit()?;Ok(())
    }).await?;
    s.emit("playlists",json!({}));Ok(false)
}
async fn apply(s:&AppState,job:&str,b:&Binding,current:&Value,local:&Value,action:&str,tracks:&[String],name:&str)->anyhow::Result<Option<Value>> {
    if !crate::background::disposition(s,job).await?{return Ok(None);}
    if account(s,&b.provider).await?!=b.account{anyhow::bail!("音源账号已切换，原账号歌单暂停同步");}
    if !crate::background::disposition(s,job).await?{return Ok(None);}
    let result=s.plugin.call(&b.provider,"playlist_apply",json!({"playlist_id":b.remote,"revision":current["revision"],"action":action,"track_ids":tracks,"name":name,"account_context":b.account})).await;
    match result {
        Ok(value)=>{validate_snapshot(s,b,&value).await?;Ok(Some(value))}
        Err(error)=>{
            let latest=snapshot(s,b,current["name"].as_str().unwrap_or(name)).await?;
            if latest["revision"]!=current["revision"] {
                conflict(s,job,b,local,&latest,"云端在推送期间变化，请确认后继续").await?;
                return Ok(None);
            }
            Err(api(error))
        }
    }
}
pub(crate) async fn run_job(s:&AppState,job:&str,payload:&Value)->anyhow::Result<bool> {
    let id=payload["local_id"].as_str().unwrap_or("").to_owned();
    let result=run_inner(s,job,&id).await;
    if let Err(error)=&result{let key=id;let error=error.to_string();s.db.run(move|c|{c.execute("UPDATE playlist_bindings SET status='failed',error=?2 WHERE local_id=?1 AND status!='conflict'",params![key,error])?;Ok(())}).await?;s.emit("playlists",json!({}));}
    result
}
async fn load_binding(s:&AppState,id:&str,user:Option<String>)->anyhow::Result<Option<(Binding,Value)>> {
    let id=id.to_owned();
    s.db.read(move|c|{
        let row=c.query_row(
            "SELECT b.local_id,b.provider,b.remote_id,b.account,b.capabilities,b.baseline,b.local_baseline,b.local_version,b.conflict,p.name,p.tracks
             FROM playlist_bindings b JOIN playlists p ON p.id=b.local_id WHERE b.local_id=?1 AND (?2 IS NULL OR b.user_id=?2)",
            params![id,user],|r|Ok((Binding{id:r.get(0)?,provider:r.get(1)?,remote:r.get(2)?,account:r.get(3)?,
                caps:serde_json::from_str(&r.get::<_,String>(4)?).unwrap_or_default(),
                baseline:serde_json::from_str(&r.get::<_,String>(5)?).unwrap_or_default(),
                local_baseline:serde_json::from_str(&r.get::<_,String>(6)?).unwrap_or_default(),
                version:r.get(7)?,conflict:r.get(8)?},
                json!({"name":r.get::<_,String>(9)?,"track_ids":serde_json::from_str::<Value>(&r.get::<_,String>(10)?).unwrap_or_default()}))),
        ).optional()?;
        Ok(row)
    }).await
}
async fn run_inner(s:&AppState,job:&str,id:&str)->anyhow::Result<bool> {
    let Some((b,local))=load_binding(s,id,None).await? else {return Ok(true);};
    if b.conflict.is_some(){return Ok(false);}
    crate::plugin::check_enabled(s,&b.provider).await.map_err(api)?;
    if account(s,&b.provider).await?!=b.account{anyhow::bail!("音源账号已切换，请切回绑定账号后重试");}
    crate::background::progress(s,job,"读取云端歌单与同步基线",0,0).await?;
    let cloud=snapshot(s,&b,local["name"].as_str().unwrap_or("云端歌单")).await?;
    let all_local=ids(&local);let cloud_ids=ids(&cloud);let base=ids(&b.baseline);
    let source=b.provider.clone();let candidates=all_local.clone();
    let matched=s.db.read(move|c|provider_tracks(c,&source,&candidates)).await?;
    let projected=all_local.iter().filter(|id|matched.contains(*id)).cloned().collect::<Vec<_>>();
    let favorites=b.id.starts_with("favorites-");
    let mut wanted=if favorites{merge_favorites(&base,&projected,&cloud_ids)}else{
        match merge(&base,&projected,&cloud_ids){Ok(v)=>v,Err(reason)=>return conflict(s,job,&b,&local,&cloud,&reason).await}
    };
    if b.baseline.get("revision").is_none(){let local_id=b.id.clone();let version=b.version;let changes=s.db.run(move|c|Ok(c.prepare("SELECT payload FROM playlist_operations WHERE local_id=?1 AND version<=?2 AND status='pending' ORDER BY version,created_at,id")?.query_map(params![local_id,version],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;for change in changes{let change:Value=serde_json::from_str(&change)?;if let Some(id)=change["track_change"]["track_id"].as_str(){if change["track_change"]["remove"]==true{wanted.retain(|track|track!=id);}else if matched.contains(id)&&!wanted.iter().any(|track|track==id){wanted.push(id.to_owned());}}}}
    let local_name=local["name"].as_str().unwrap_or("云端歌单");let remote_name=cloud["name"].as_str().unwrap_or(local_name);
    let renamed=!favorites&&local["name"]!=b.local_baseline["name"];
    if renamed&&b.baseline["name"].is_string()&&cloud["name"]!=b.baseline["name"]&&local_name!=remote_name{return conflict(s,job,&b,&local,&cloud,"本地与云端都重命名了歌单").await;}
    let desired_name=if renamed{local_name}else{remote_name};
    let mut remote=cloud.clone();let mut notes=vec![];let current=strings(&cloud_ids);let target=strings(&wanted);
    let removed=cloud_ids.iter().filter(|id|!target.contains(id.as_str())).cloned().collect::<Vec<_>>();let added=wanted.iter().filter(|id|!current.contains(id.as_str())).cloned().collect::<Vec<_>>();
    for(action,values)in [("remove",removed),("add",added)] {
        if values.is_empty(){continue;}if !b.caps.iter().any(|cap|cap==action){notes.push("音源只读或不支持此修改，已保留本地内容".to_owned());continue;}
        for batch in values.chunks(100){crate::background::progress(s,job,"推送本地歌单修改并读回确认",0,0).await?;
            let Some(value)=apply(s,job,&b,&remote,&local,action,batch,desired_name).await? else{return Ok(false);};
            remote=value;
        }
    }
    if desired_name!=remote_name{if b.caps.iter().any(|c|c=="rename"){let Some(value)=apply(s,job,&b,&remote,&local,"rename",&[],desired_name).await? else{return Ok(false);};remote=value;}else{notes.push("此音源暂不支持同步重命名".into());}}
    if !favorites&&ids(&remote)!=wanted {if b.caps.iter().any(|c|c=="reorder"){let Some(value)=apply(s,job,&b,&remote,&local,"reorder",&wanted,desired_name).await? else{return Ok(false);};remote=value;}else{notes.push("重复曲目或歌曲顺序未同步，保留本地顺序".into());}}
    if all_local.iter().any(|id|!matched.contains(id)){notes.push("本地文件及未匹配歌曲仅保留在本地".into());}
    if !crate::background::disposition(s,job).await?{return Ok(false);}
    let next_local=if favorites {
        let cloud_recent=newest_first(&b,&cloud).then(||ids(&remote));
        favorites_order(&all_local,&matched,&wanted,cloud_recent.as_deref())
    }else{with_extras(&all_local,&matched,&wanted)};
    let id=b.id.clone();let version=b.version;let baseline=remote.to_string();let revision=remote["revision"].as_str().unwrap().to_owned();
    let published=remote.clone();let provider=b.provider.clone();let account=b.account.clone();let playlist=b.remote.clone();
    let next=json!({"name":if favorites{"我喜欢"}else{desired_name},"track_ids":next_local});let status=if notes.is_empty(){"synced"}else{"partial"};let error=if notes.is_empty(){None}else{Some(notes.join("；"))};
    s.db.run(move|c|{let tx=c.transaction()?;
        let current=tx.query_row("SELECT local_version FROM playlist_bindings WHERE local_id=?1",[&id],|r|r.get::<_,i64>(0)).optional()?;let Some(current)=current else{return Ok(());};
        if current==version{tx.execute("UPDATE playlists SET name=?2,tracks=?3 WHERE id=?1",params![id,next["name"].as_str().unwrap(),next["track_ids"].to_string()])?;}
        tx.execute("UPDATE playlist_bindings SET baseline=?2,local_baseline=?3,revision=?4,status=?5,error=?6,last_sync=unixepoch(),next_due=unixepoch()+3600 WHERE local_id=?1",params![id,baseline,next.to_string(),revision,if current==version{status}else{"pending"},error])?;
        crate::playlist_pages::publish(&tx,&provider,&account,&playlist,&published)?;
        tx.execute("UPDATE playlist_operations SET status='completed' WHERE local_id=?1 AND version<=?2 AND status='pending'",params![id,version])?;tx.commit()?;Ok(())}).await?;
    s.emit("playlists",json!({}));Ok(true)
}
pub fn spawn(s:AppState){tokio::spawn(async move{let mut tick=tokio::time::interval(std::time::Duration::from_secs(30));loop{tick.tick().await;let _=s.db.run(|c|{let tx=c.transaction()?;let ids=tx.prepare("SELECT local_id FROM playlist_bindings WHERE auto_sync=1 AND conflict IS NULL AND next_due<=unixepoch()")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;for id in ids{schedule(&tx,&id)?;tx.execute("UPDATE playlist_bindings SET next_due=unixepoch()+3600 WHERE local_id=?1",[id])?;}tx.commit()?;Ok(())}).await;}});}

#[cfg(test)]mod tests{
    use super::*;
    fn list(ids:&[&str])->Vec<String>{ids.iter().map(|s|s.to_string()).collect()}
    #[test]fn concurrent_membership_changes_merge_but_conflicting_orders_stop(){
        assert_eq!(merge(&list(&["a","b"]),&list(&["b","l"]),&list(&["a","b","r"])).unwrap(),list(&["b","l","r"]));
        assert!(merge(&list(&["a","b","c"]),&list(&["b","a","c"]),&list(&["a","c","b"])).is_err());
        assert_eq!(with_extras(&list(&["a","local:file","b"]),&list(&["a","b"]).into_iter().collect(),&list(&["b","c"])),list(&["b","local:file","c"]));
    }
    #[test]fn favorites_follow_cloud_recency_without_moving_unmatched_tracks(){
        let local=list(&["old","offline","latest","recent","removed","pending"]);
        let matched=list(&["old","latest","recent","removed","pending"]).into_iter().collect();
        let cloud=list(&["new","latest","recent","old","removed"]);
        let wanted=list(&["old","latest","recent","new","pending"]);
        let result=favorites_order(&local,&matched,&wanted,Some(&cloud));
        assert_eq!(result,list(&["old","offline","recent","latest","pending","new"]));
        assert_eq!(favorites_order(&result,&matched,&wanted,Some(&cloud)),result);
    }
}
