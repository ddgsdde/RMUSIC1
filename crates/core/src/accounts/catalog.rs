//! External discovery uses its own taste and the permitted shared catalogue.
use super::*;
use crate::{models::Track,recommendations::Selection};
pub async fn select(s:&AppState,user:&str,seed:Option<String>,exclude:Vec<String>,limit:usize)->ApiResult<Selection>{
 let uid=user.to_owned();let raw=s.db.read({let uid=uid.clone();move|c|Ok(c.query_row("SELECT policy FROM external_accounts WHERE user_id=?1 AND enabled=1",[uid],|r|r.get::<_,String>(0)).optional()?)}).await?.ok_or_else(ApiError::auth)?;
 let p:Policy=serde_json::from_str(&raw).map_err(anyhow::Error::from)?;p.validate()?;let predicate=p.sql("t");
 let items=s.db.read(move|c|{
  let mut excluded=exclude.into_iter().collect::<std::collections::HashSet<_>>();if let Some(seed)=seed{excluded.insert(seed);}
  let mut artists=HashMap::<String,f64>::new();
  let favorites=c.prepare("SELECT t.artist,SUM(a.play_seconds) FROM account_track_stats a JOIN tracks t ON t.id=a.track_id WHERE a.user_id=?1 GROUP BY t.artist ORDER BY SUM(a.play_seconds) DESC LIMIT 100")?.query_map([&uid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,f64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
  for (artist,seconds) in favorites{artists.insert(artist,seconds.max(0.).ln_1p());}
  let sql=format!("SELECT t.* FROM tracks t WHERE t.available=1 AND {predicate} ORDER BY t.created_at DESC,t.id LIMIT 1000");
  let mut tracks=c.prepare(&sql)?.query_map([],Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
  let personal=format!("SELECT t.* FROM account_track_stats a JOIN tracks t ON t.id=a.track_id WHERE a.user_id=?1 AND t.available=1 AND {predicate} ORDER BY a.last_played_at DESC LIMIT 100");
  tracks.extend(c.prepare(&personal)?.query_map([&uid],Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?);
  let now=chrono::Utc::now().timestamp()/86400;let mut scored=Vec::new();
  for track in tracks{if !excluded.insert(track.id.clone()){continue;}let hash=crate::auth::hash(&format!("{uid}:{now}:{}",track.id));let random=u32::from_str_radix(&hash[..8],16).unwrap_or(0)as f64/u32::MAX as f64;let score=random*4.+artists.get(&track.artist).copied().unwrap_or(0.).min(8.);scored.push((score,track));}
  scored.sort_by(|a,b|b.0.total_cmp(&a.0).then_with(||a.1.id.cmp(&b.1.id)));Ok(scored.into_iter().take(limit.clamp(1,100)).map(|(_,t)|t).collect::<Vec<_>>())
 }).await?;
 Ok(Selection{items,source:"personal".into(),fallback:false,message:None,sections:vec![]})
}
use std::collections::HashMap;
