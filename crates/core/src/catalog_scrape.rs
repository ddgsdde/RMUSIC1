//! Opt-in, resumable metadata scraping; never downloads audio or guesses entity identities.
use crate::{auth::Auth, state::AppState};
use futures_util::{stream, StreamExt};
use rusqlite::params;
use serde_json::json;
const ENTITIES:&str="SELECT 'artist' kind,credit.artist_id id FROM track_artists credit JOIN tracks t ON t.id=credit.track_id WHERE t.available=1 AND COALESCE(json_extract(t.provider_data,'$.catalog_browse_only'),0)=0 GROUP BY credit.artist_id UNION SELECT 'album',CASE WHEN COALESCE(NULLIF(json_extract(t.provider_data,'$.release_id'),''),NULLIF(json_extract(t.provider_data,'$.album_mid'),'')) IS NOT NULL THEN 'release:'||CASE WHEN t.source='local' THEN COALESCE(json_extract(t.provider_data,'$.cached_from.source'),t.source) ELSE t.source END||':'||COALESCE(NULLIF(json_extract(t.provider_data,'$.release_id'),''),json_extract(t.provider_data,'$.album_mid')) ELSE t.album_id END FROM tracks t WHERE t.available=1 AND COALESCE(json_extract(t.provider_data,'$.catalog_browse_only'),0)=0 AND trim(t.album)<>'' GROUP BY 2";
pub(crate) async fn due(s: &AppState, limit: usize) -> anyhow::Result<Vec<(String, String)>> {
    s.db.read(move|c|Ok(c.prepare(&format!("SELECT e.kind,e.id FROM ({ENTITIES}) e LEFT JOIN catalog_scrape_state s ON s.kind=e.kind AND s.entity_id=e.id WHERE COALESCE(s.checked_at,0)<unixepoch()-86400 AND COALESCE(s.retry_at,0)<=unixepoch() ORDER BY COALESCE(s.checked_at,0),e.kind,e.id LIMIT ?1"))?.query_map([limit as i64],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}
pub async fn schedule(s: &AppState) -> anyhow::Result<()> {
    if s.setting("catalog_scrape_enabled", json!(false)).await != true {
        return Ok(());
    }
    if !due(s, 1).await?.is_empty() {
        crate::background::enqueue(s, "catalog_scrape", "all", json!({}), -20).await?;
    }
    Ok(())
}
pub async fn run(s: &AppState, job: &str) -> anyhow::Result<bool> {
    if s.setting("catalog_scrape_enabled", json!(false)).await != true {
        return Ok(true);
    }
    let n = s
        .setting("catalog_scrape_concurrency", json!(1))
        .await
        .as_u64()
        .unwrap_or(1)
        .clamp(1, 4) as usize;
    let items = due(s, n).await?;
    if items.is_empty() {
        return Ok(true);
    }
    let mut work=stream::iter(items.into_iter().map(|(kind,id)|async move {
  let auth=Auth{id:"system-catalog".into(),username:"后台资料刮削".into(),totp_enabled:false,interactive:false,role:"admin".into(),scopes:vec!["read".into()],session_hash:String::new()};
  let q=crate::catalog_detail::Options{page:Some(1),local_page:Some(1),provider:None,sort:None,platform_sort:None,remote:Some(true),refresh:false,client_metadata:false};
  let result=crate::catalog_detail::detail(axum::extract::State(s.clone()),auth,axum::extract::Path((kind.clone(),id.clone())),axum::extract::Query(q)).await;
  let error=match result{Ok(value)=>{
   // Persist only the first bounded catalog page; missing images are fetched once per shared source.
   if let Some(image)=value.0["profile"]["image_url"].as_str().and_then(|v|v.strip_prefix("/api/v1/artwork/")){let _=crate::artwork_recovery::source(s,image,false).await;}
   let notices=value.0["platforms"].as_array().is_some_and(|rows|!rows.is_empty()&&rows.iter().all(|v|v["notice"].as_str().is_some_and(|v|!v.is_empty())));if notices{Some("资料暂不可用，稍后重试".to_owned())}else{None}
  },Err(e)=>Some(e.2)};
  let(error,kind,id)=(error,kind,id);
  s.db.run(move|c|{c.execute("INSERT INTO catalog_scrape_state(kind,entity_id,checked_at,retry_at,error)VALUES(?1,?2,CASE WHEN ?3 IS NULL THEN unixepoch() ELSE 0 END,CASE WHEN ?3 IS NULL THEN 0 ELSE unixepoch()+300 END,?3) ON CONFLICT(kind,entity_id)DO UPDATE SET checked_at=CASE WHEN excluded.error IS NULL THEN excluded.checked_at ELSE checked_at END,retry_at=excluded.retry_at,error=excluded.error",params![kind,id,error])?;Ok(())}).await
 })).buffer_unordered(n);
    while let Some(result) = work.next().await {
        result?;
        if !crate::background::disposition(s, job).await? {
            return Ok(false);
        }
        let key = job.to_owned();
        s.db.run(move|c|{c.execute("UPDATE jobs SET processed=processed+1,phase='后台刮削专辑与艺术家',updated_at=unixepoch() WHERE id=?1",[key])?;Ok(())}).await?;
    }
    Ok(due(s, 1).await?.is_empty())
}
