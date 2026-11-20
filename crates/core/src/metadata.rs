//! Provider metadata enrichment. File tags and manual edits remain authoritative.
use crate::{db::Db,state::AppState};
use anyhow::Result;
use rusqlite::{params,OptionalExtension};
use serde_json::{json,Value};
use std::time::Duration;
const JOB_ID:&str="metadata-enrichment";
use std::sync::atomic::Ordering;
pub fn frontend_busy(s:&AppState)->bool {
    s.plugin.frontend_busy()
}
struct Foreground(AppState);
impl Drop for Foreground {
    fn drop(&mut self){
        self.0.foreground_last_activity.store(chrono::Utc::now().timestamp(),Ordering::Relaxed);
        self.0.foreground_requests.fetch_sub(1,Ordering::Release);
    }
}
pub async fn foreground_priority(axum::extract::State(s):axum::extract::State<AppState>,request:axum::extract::Request,next:axum::middleware::Next)->axum::response::Response {
    let path=request.uri().path();
    let polling=request.method()==axum::http::Method::GET
        && (["/zones","/cache","/downloads"].iter().any(|suffix|path.ends_with(suffix))||path.rsplit('/').nth(1)==Some("zones"));
    let passive=polling||["/discovery","/jobs","/status","/events","/events/ticket","/bridge/events","/endpoints","/endpoints/browser","/progress"].iter().any(|suffix|path.ends_with(suffix));
    let _foreground=if passive {None} else {
        s.foreground_requests.fetch_add(1,Ordering::AcqRel);
        s.plugin.yield_background();
        Some(Foreground(s))
    };
    next.run(request).await
}
fn manual(value:&Value)->bool {
    value["manual_metadata"]==true || value["source"]=="manual" || value["manual"]==true
        || value.get("fields").and_then(Value::as_object).is_some_and(|v|v.values().any(manual))
}
async fn initialize(db:&Db)->Result<()> {
    db.run(|c|{
        c.execute("UPDATE metadata_queue SET status='pending' WHERE status='running'",[])?;
        c.execute("UPDATE metadata_queue SET status='pending',attempts=0,next_try=0 WHERE metadata LIKE '%musicbrainz%'",[])?;
        c.execute("CREATE INDEX IF NOT EXISTS metadata_due ON metadata_queue(status,next_try,album_id)",[])?;
        c.execute("INSERT OR IGNORE INTO jobs(id,kind,status,created_at)VALUES(?1,'metadata','idle',?2)",params![JOB_ID,chrono::Utc::now().timestamp()])?;
        Ok(())
    }).await
}
async fn progress(s:&AppState,status:&str,error:Option<&str>)->Result<()> {
    let status=status.to_owned();let error=error.map(str::to_owned);
    let event=s.db.run(move|c|{
        let (done,total):(i64,i64)=c.query_row("SELECT COALESCE(SUM(status IN ('complete','no_match','manual','failed')),0),COUNT(*) FROM metadata_queue",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
        c.execute("UPDATE jobs SET status=?1,processed=?2,total=?3,error=?4,updated_at=unixepoch() WHERE id=?5 AND COALESCE(json_extract(payload,'$.manual_pause'),0)=0",params![status,done,total,error,JOB_ID])?;
        Ok(json!({"id":JOB_ID,"kind":"metadata","status":status,"processed":done,"total":total,"error":error}))
    }).await?;s.emit("jobs",event);Ok(())
}
async fn claim(db:&Db)->Result<Option<(String,i64)>> {
    db.run(|c|{
        let tx=c.transaction()?;
        let row=tx.query_row("SELECT album_id,attempts FROM metadata_queue WHERE status IN ('pending','retry') AND next_try<=?1 ORDER BY next_try,album_id LIMIT 1",[chrono::Utc::now().timestamp()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).optional()?;
        if let Some((id,_))=&row {tx.execute("UPDATE metadata_queue SET status='running',attempts=attempts+1 WHERE album_id=?1",[id])?;}
        tx.commit()?;Ok(row)
    }).await
}
pub async fn save_track(s:&AppState,id:String,result:Value)->Result<()> {
    let provider=result["source"].as_str().filter(|id|crate::connectors::valid_id(id)).unwrap_or("").to_owned();
    if result["matched"]!=true||provider.is_empty(){return Ok(());}
    s.db.run(move|c|{
        let tx=c.transaction()?;
        let (data,cover,recording,source,artist_id,artist):(String,Option<String>,String,String,String,String)=tx.query_row("SELECT provider_data,cover_url,recording_id,source,artist_id,artist FROM tracks WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        let mut data:Value=serde_json::from_str(&data).unwrap_or(json!({}));
        if manual(&data){return Ok(());}
        if !data.is_object(){data=json!({});}
        data["provider_metadata"]=result.clone();
        let matched=result["mid"].as_str().filter(|mid|(5..=64).contains(&mid.len())&&mid.bytes().all(|b|b.is_ascii_alphanumeric()));
        // Provider metadata is an assertion, not permission to rewrite a recording identity.
        let _matched=matched;
        let cover=cover.filter(|s|!s.is_empty()).or_else(||result["track"]["cover_url"].as_str().filter(|url|url.starts_with("https://")).map(str::to_owned));
        let cover=cover.map(|url|crate::media_assets::register(&tx,&url).unwrap_or(url));
        let lyrics=result["lyrics"].as_str().unwrap_or("");
        tx.execute("UPDATE tracks SET provider_data=?1,cover_url=?2,recording_id=?3,lyrics=CASE WHEN lyrics IS NULL OR lyrics='' THEN ?4 ELSE lyrics END WHERE id=?5",params![data.to_string(),cover,recording,lyrics,id])?;
        if let Some(matched)=result["artists"].as_array().and_then(|artists|artists.iter().find(|value|value["name"].as_str().is_some_and(|name|name.trim().to_lowercase()==artist.trim().to_lowercase()))) {
            let image=matched["image_url"].as_str().filter(|url|url.starts_with("https://"));
            let value=json!({"source":provider,"image_url":image,"description":matched["description"],"mid":matched["mid"]});
            tx.execute("INSERT INTO artist_metadata(artist_id,data)VALUES(?1,?2) ON CONFLICT(artist_id)DO UPDATE SET data=excluded.data",params![artist_id,value.to_string()])?;
        }
        let mut updated=tx.query_row("SELECT * FROM tracks WHERE id=?1",[&id],crate::models::Track::from_row)?;
        if source==provider && result["track"]["id"]==id && !crate::library_query::known_album(&updated.album) {
            if let Some(album)=result["track"]["album"].as_str().filter(|v|crate::library_query::known_album(v)) {
                updated.album=album.to_owned();
                if let Some(artist)=result["track"]["album_artist"].as_str().filter(|v|!v.is_empty()){updated.album_artist=artist.into();}
                crate::library::upsert(&tx,&updated,None,None,0,0,None,None)?;
            }
        }
        crate::artist_credits::index(&tx,&updated)?;
        if let Some(key)=crate::artist_credits::release_key(&updated.source,&updated.provider_data){crate::album_artwork::refresh(&tx,&key)?;}
        tx.commit()?;Ok(())
    }).await
}
async fn enrich(s:&AppState,id:String,attempts:i64)->Result<bool> {
    let album=id.clone();
    let tracks=s.db.run(move|c|Ok(c.prepare("SELECT * FROM tracks WHERE album_id=?1 AND available=1")?.query_map([album],crate::models::Track::from_row)?.collect::<Result<Vec<_>,_>>()?)).await?;
    let provider=crate::connectors::metadata_provider(s,"").await.map_err(|e|anyhow::anyhow!(e.2))?;
    let mut matched=0;let mut failures=0;
    for track in tracks {
        if manual(&track.provider_data) {continue;}
        if track.provider_data["provider_metadata"]["matched"]==true||track.provider_data["qqmusic_metadata"]["matched"]==true {matched+=1;continue;}
        let paused=s.db.run(|c|Ok(c.query_row("SELECT COALESCE(json_extract(payload,'$.manual_pause'),0) FROM jobs WHERE id=?1",[JOB_ID],|r|r.get::<_,bool>(0))?)).await?;
        if paused||s.resources.over_budget()||frontend_busy(s)||s.zones.read().await.values().any(|z|z.status=="playing") {
            s.db.run(move|c|{c.execute("UPDATE metadata_queue SET status='pending',attempts=MAX(attempts-1,0) WHERE album_id=?1",[id])?;Ok(())}).await?;
            return Ok(false);
        }
        match s.plugin.background_metadata(&provider,json!({"track":track,"track_id":track.id})).await {
            Ok(result) if result["matched"]==true&&result["source"]==provider =>{save_track(s,track.id,result).await?;matched+=1;}
            Ok(_)=>{},
            Err(error) if error.1=="background_deferred"=>{
                s.db.run(move|c|{c.execute("UPDATE metadata_queue SET status='pending',attempts=MAX(attempts-1,0) WHERE album_id=?1",[id])?;Ok(())}).await?;
                return Ok(false);
            },
            Err(_)=>{failures+=1;break;},
        }
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }
    s.db.run(move|c|{
        let status=if failures>0 {if attempts>=4{"failed"}else{"retry"}}else if matched>0{"complete"}else{"no_match"};
        let next=if failures>0{chrono::Utc::now().timestamp()+60*(attempts+1)}else{0};
        c.execute("UPDATE metadata_queue SET status=?1,next_try=?2,metadata=?3 WHERE album_id=?4",params![status,next,json!({"source":provider,"matched":matched,"failures":failures}).to_string(),id])?;Ok(())
    }).await?;
    s.refresh_queue_metadata().await?;s.emit("library",json!({"reason":"provider_metadata"}));Ok(failures>0)
}
pub fn spawn(s:AppState){tokio::spawn(async move{
    if initialize(&s.db).await.is_err(){return;}
    loop {
        let enabled=s.setting("metadata_enabled",json!(true)).await==true;
        let playing=s.zones.read().await.values().any(|z|z.status=="playing");
        if !enabled||playing||frontend_busy(&s) {let _=progress(&s,"paused",None).await;tokio::time::sleep(Duration::from_secs(5)).await;continue;}
        if crate::connectors::metadata_provider(&s,"").await.is_err(){let _=progress(&s,"waiting_provider",Some("启用资料连接器后补全资料")).await;tokio::time::sleep(Duration::from_secs(15)).await;continue;}
        let manual_pause=s.db.run(|c|Ok(c.query_row("SELECT COALESCE(json_extract(payload,'$.manual_pause'),0) FROM jobs WHERE id=?1",[JOB_ID],|r|r.get::<_,bool>(0))?)).await.unwrap_or(false);
        if manual_pause {tokio::time::sleep(Duration::from_secs(2)).await;continue;}
        let Ok(_resource)=s.resources.background(&s,16*1024*1024,false).await else{tokio::time::sleep(Duration::from_secs(2)).await;continue;};
        match claim(&s.db).await {
            Ok(Some((id,attempts)))=>{let _=progress(&s,"running",None).await;match enrich(&s,id,attempts).await {Ok(true)=>{let _=progress(&s,"waiting_provider",Some("匿名资料请求失败，暂停一分钟后重试")).await;tokio::time::sleep(Duration::from_secs(60)).await;},Err(_)=>{let _=progress(&s,"error",Some("资料更新中断，重启后恢复")).await;},_=>{}}},
            _=>{let _=progress(&s,"idle",None).await;tokio::time::sleep(Duration::from_secs(15)).await;}
        }
    }
});}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn manual_fields_remain_authoritative(){assert!(manual(&json!({"manual_metadata":true})));assert!(manual(&json!({"fields":{"artist":{"source":"manual"}}})));assert!(!manual(&json!({"source":"qqmusic"})));}
}
