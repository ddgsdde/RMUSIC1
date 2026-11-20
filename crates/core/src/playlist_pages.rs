//! Account-isolated cloud pages. Stale pages remain readable while refresh is queued.
use crate::{state::AppState,error::ApiResult};
use rusqlite::{OptionalExtension,params};
use serde_json::{Value,json};
pub fn publish(c:&rusqlite::Connection,provider:&str,account:&str,playlist:&str,snapshot:&Value)->anyhow::Result<()> {
 let Some(items)=snapshot["items"].as_array()else{return Ok(());};
 let pages=c.prepare("SELECT id,json_extract(id,'$[4]') FROM provider_playlist_pages WHERE provider=?1 AND account=?2 AND json_extract(id,'$[3]')=?3")?.query_map(params![provider,account,playlist],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)? as usize)))?.collect::<rusqlite::Result<Vec<_>>>()?;
 for(id,page)in pages{let start=page.saturating_sub(1)*100;let value=json!({"playlist_id":playlist,"name":snapshot["name"],"total":items.len(),"next_page":if start+100<items.len(){Some(page+1)}else{None},"items":items.iter().skip(start).take(100).collect::<Vec<_>>()});c.execute("UPDATE provider_playlist_pages SET data=?2,updated_at=unixepoch() WHERE id=?1",params![id,value.to_string()])?;}Ok(())
}
pub async fn context(s:&AppState,provider:&str)->anyhow::Result<Option<String>> {
 let provider=provider.to_owned();s.db.run(move|c|Ok(c.query_row("SELECT context FROM provider_accounts WHERE provider=?1 AND context NOT IN ('anonymous','switching','public')",[provider],|r|r.get(0)).optional()?)).await
}
pub async fn get(s:&AppState,user:&str,provider:&str,playlist:&str,page:u32)->ApiResult<Option<Value>> {
 let Some(account)=context(s,provider).await? else{return Ok(None);};let key=json!([user,provider,account,playlist,page]).to_string();let lookup=key.clone();
 let saved=s.db.run(move|c|Ok(c.query_row("SELECT data,updated_at<unixepoch()-300 FROM provider_playlist_pages WHERE id=?1",[lookup],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?))).optional()?)).await?;
 if let Some((data,stale))=saved{if stale{crate::background::enqueue(s,"playlist_page",&key,json!({"user":user,"provider":provider,"account":account,"playlist":playlist,"page":page}),20).await?;}return Ok(Some(serde_json::from_str(&data).map_err(anyhow::Error::from)?));}Ok(None)
}
pub async fn save(s:&AppState,user:&str,provider:&str,playlist:&str,page:u32,expected:Option<&str>,value:&Value)->ApiResult<()> {
 let Some(account)=context(s,provider).await? else{return Ok(());};if Some(account.as_str())!=expected{return Ok(());}
 let key=json!([user,provider,account,playlist,page]).to_string();let data=value.to_string();let provider=provider.to_owned();
 s.db.run(move|c|{let current=c.query_row("SELECT context FROM provider_accounts WHERE provider=?1",[&provider],|r|r.get::<_,String>(0)).optional()?;if current.as_deref()!=Some(account.as_str()){return Ok(());}c.execute("INSERT INTO provider_playlist_pages(id,provider,account,data,updated_at) VALUES(?1,?2,?3,?4,unixepoch()) ON CONFLICT(id) DO UPDATE SET data=excluded.data,updated_at=excluded.updated_at",params![key,provider,account,data])?;Ok(())}).await?;Ok(())
}
pub async fn refresh(s:&AppState,payload:&Value)->anyhow::Result<bool> {
 let provider=payload["provider"].as_str().unwrap_or("");if context(s,provider).await?.as_deref()!=payload["account"].as_str(){return Ok(true);}
 let playlist=payload["playlist"].as_str().unwrap_or("");let page=payload["page"].as_u64().unwrap_or(1) as u32;
 crate::plugin::check_enabled(s,provider).await.map_err(|e|anyhow::anyhow!(e.2))?;
 let epoch=crate::provider_session::start(s,provider).await.map_err(|e|anyhow::anyhow!(e.2))?;
 let result=s.plugin.call(provider,"playlist_tracks",json!({"playlist_id":playlist,"page":page})).await.map_err(|e|anyhow::anyhow!(e.2))?;
 if result["playlist_id"]!=playlist||!result["items"].is_array()||result["total"].as_u64().is_none(){anyhow::bail!("歌单分页格式无效");}
 crate::provider_session::validate(s,provider,epoch).await.map_err(|e|anyhow::anyhow!(e.2))?;
 crate::plugin::import_mode_epoch(s,provider,&result,false,Some(epoch)).await.map_err(|e|anyhow::anyhow!(e.2))?;
 save(s,payload["user"].as_str().unwrap_or(""),provider,playlist,page,payload["account"].as_str(),&result).await.map_err(|e|anyhow::anyhow!(e.2))?;s.emit("playlists",json!({}));Ok(true)
}
