//! Local Chromaprint evidence. It never downloads media or silently merges recordings.
use crate::{auth::Auth,error::{ApiError,ApiResult},models::Track,state::AppState};
use axum::{extract::{Path,Query,State},Json};
use rusqlite::{params,OptionalExtension};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use std::{path::Path as FsPath,process::Stdio,time::{Duration,UNIX_EPOCH}};
use tokio::{io::AsyncReadExt,process::Command};
const ALGORITHM:&str="chromaprint-test2-120s-v1";
const MAX_BYTES:u64=128*1024;
#[derive(Serialize,Deserialize)]
struct Evidence {duration:f64,fingerprint:Vec<u32>,sampled_seconds:u32}
fn stamp(path:&str)->anyhow::Result<String>{let m=std::fs::metadata(path)?;anyhow::ensure!(m.is_file(),"音频文件不存在");Ok(format!("{}:{}:{}",path,m.len(),m.modified()?.duration_since(UNIX_EPOCH)?.as_nanos()))}
async fn source(s:&AppState,id:&str)->anyhow::Result<Option<String>>{
 let id=id.to_owned();s.db.run(move|c|Ok(c.query_row("SELECT CASE WHEN source='local' THEN path ELSE (SELECT path FROM cache_entries e WHERE e.track_id=t.id AND e.complete=1 ORDER BY e.last_access DESC LIMIT 1) END FROM tracks t WHERE id=?1 AND available=1",[id],|r|r.get::<_,Option<String>>(0)).optional()?.flatten())).await
}
pub async fn enqueue(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
 a.require("library")?;if source(&s,&id).await?.is_none(){return Err(ApiError::bad("指纹分析仅使用本地文件或已有完整缓存，不会下载云端歌曲"));}
 let job=crate::background::enqueue(&s,"fingerprint",&id,json!({"track_id":id}),20).await?;Ok(Json(json!({"id":job,"status":"pending"})))
}
fn parse(bytes:&[u8])->anyhow::Result<Evidence>{
 let value:Value=serde_json::from_slice(bytes)?;let duration=value["duration"].as_f64().filter(|n|n.is_finite()&&(30. ..=86400.).contains(n)).ok_or_else(||anyhow::anyhow!("指纹音频时长不足或无效"))?;
 let raw=value["fingerprint"].as_array().filter(|a|(100..=2048).contains(&a.len())).ok_or_else(||anyhow::anyhow!("指纹长度无效或音频不足"))?;
 let fingerprint=raw.iter().map(|n|n.as_u64().filter(|n|*n<=u32::MAX as u64).map(|n|n as u32).or_else(||n.as_i64().filter(|n|*n>=i32::MIN as i64&&*n<0).map(|n|n as u32)).ok_or_else(||anyhow::anyhow!("指纹数据不是32位整数"))).collect::<anyhow::Result<Vec<_>>>()?;
 anyhow::ensure!(fingerprint.iter().collect::<std::collections::HashSet<_>>().len()>=16,"静音或信息不足的指纹不用于录音匹配");
 Ok(Evidence{duration,fingerprint,sampled_seconds:120u32.min(duration.ceil() as u32)})
}
pub(crate) async fn run(s:&AppState,job:&str,payload:&Value)->anyhow::Result<bool>{
 let id=payload["track_id"].as_str().ok_or_else(||anyhow::anyhow!("缺少指纹曲目"))?;
 let Some(path)=source(s,id).await? else{anyhow::bail!("来源文件不可用；不会为指纹下载歌曲");};
 let _lease=crate::media::retain_path(s,&path).await?;let before=stamp(&path)?;
 let mut cmd=Command::new(std::env::var_os("RMUSIC_FPCALC").unwrap_or_else(||"fpcalc".into()));
 // Files only, no provider URL or user-supplied command. Drop always kills the child.
 let mut child=cmd.args(["-json","-raw","-algorithm","2","-length","120"]).arg(FsPath::new(&path)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn().map_err(|_|anyhow::anyhow!("Chromaprint fpcalc 未安装；请安装服务端指纹依赖"))?;
 let stdout=child.stdout.take().ok_or_else(||anyhow::anyhow!("指纹进程无输出"))?;
 let operation=async{let mut bytes=Vec::new();stdout.take(MAX_BYTES+1).read_to_end(&mut bytes).await?;anyhow::ensure!(bytes.len()<=MAX_BYTES as usize,"指纹响应超限");let status=child.wait().await?;anyhow::ensure!(status.success(),"音频解码／指纹分析失败");parse(&bytes)};
 let evidence=tokio::time::timeout(Duration::from_secs(180),operation).await.map_err(|_|anyhow::anyhow!("指纹分析超时"))??;
 if stamp(&path)?!=before||source(s,id).await?.as_deref()!=Some(&path){return Ok(false);}
 let key=id.to_owned();let job=job.to_owned();let data=serde_json::to_string(&evidence)?;
 let saved=s.db.run(move|c|{let tx=c.transaction()?;if !tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND status='running')",[job],|r|r.get::<_,bool>(0))?{return Ok(false);}
  tx.execute("INSERT INTO audio_fingerprints(track_id,algorithm,source_stamp,data,created_at) VALUES(?1,?2,?3,?4,unixepoch()) ON CONFLICT(track_id) DO UPDATE SET algorithm=excluded.algorithm,source_stamp=excluded.source_stamp,data=excluded.data,created_at=excluded.created_at",params![key,ALGORITHM,before,data])?;tx.commit()?;Ok(true)
 }).await?;Ok(saved)
}
fn similarity(a:&[u32],b:&[u32])->Option<f64>{
 // Tiny codec start-delay tolerance only. This is evidence, never a release/mother-master identity.
 if a.len()<100||b.len()<100{return None;}
 (-2i32..=2).filter_map(|offset|{let a=&a[offset.max(0) as usize..];let b=&b[(-offset).max(0) as usize..];let n=a.len().min(b.len());if n<100{return None;}let bits=a.iter().zip(b).take(n).map(|(a,b)|(a^b).count_ones() as u64).sum::<u64>();Some(1.-bits as f64/(n*32) as f64)}).max_by(f64::total_cmp)
}
fn edition(a:&Track,b:&Track)->bool{
 let norm=|v:&str|v.to_lowercase().split_whitespace().collect::<String>();
 let tags=|t:&Track|format!("{} {}",norm(&t.title),norm(t.provider_data["subtitle"].as_str().unwrap_or("")));
 !a.artist.trim().is_empty()&&norm(&a.artist)==norm(&b.artist)&&norm(&a.title)==norm(&b.title)&&(a.duration-b.duration).abs()<=2.&&["live","remaster","remix","acoustic","cover","instrumental","现场","翻唱","重制","伴奏"].iter().all(|v|tags(a).contains(v)==tags(b).contains(v))
}
#[derive(Deserialize)]pub struct Page {#[serde(default)]offset:u32}
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(page):Query<Page>)->ApiResult<Json<Value>>{
 a.require("read")?;if page.offset>1000000{return Err(ApiError::bad("指纹候选分页超出范围"));}let offset=page.offset;let Some(path)=source(&s,&id).await? else{return Ok(Json(json!({"available":false,"reason":"需要本地文件或已有完整缓存","automatic_merge":false})));};let current=stamp(&path).ok();
 let key=id.clone();let rows=s.db.run(move|c|{let own=c.query_row("SELECT t.*,f.data fingerprint_data,f.source_stamp fingerprint_stamp FROM tracks t JOIN audio_fingerprints f ON f.track_id=t.id WHERE t.id=?1 AND f.algorithm=?2",params![key,ALGORITHM],|r|Ok((Track::from_row(r)?,r.get::<_,String>("fingerprint_data")?,r.get::<_,String>("fingerprint_stamp")?))).optional()?;
  let Some((track,data,stored))=own else{return Ok(None);};if current.as_deref()!=Some(&stored){return Ok(None);}
  let candidates=c.prepare("SELECT t.*,f.data fingerprint_data,f.source_stamp fingerprint_stamp FROM audio_fingerprints f JOIN tracks t ON t.id=f.track_id WHERE f.algorithm=?1 AND t.available=1 AND t.id!=?2 AND ABS(t.duration-?3)<=2 ORDER BY t.id LIMIT 101 OFFSET ?4")?.query_map(params![ALGORITHM,key,track.duration,offset],|r|Ok((Track::from_row(r)?,r.get::<_,String>("fingerprint_data")?,r.get::<_,String>("fingerprint_stamp")?)))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(Some((track,data,candidates)))
 }).await?;
 let Some((track,data,candidates))=rows else{return Ok(Json(json!({"available":false,"reason":"尚未分析或来源文件已改变","automatic_merge":false})));};
 let evidence:Evidence=serde_json::from_str(&data).map_err(anyhow::Error::from)?;let mut items=vec![];let truncated=candidates.len()>100;
 for(candidate,data,stored)in candidates.into_iter().take(100){if !edition(&track,&candidate){continue;}let Some(path)=source(&s,&candidate.id).await? else{continue;};if stamp(&path).ok().as_deref()!=Some(&stored){continue;}let other:Evidence=serde_json::from_str(&data).map_err(anyhow::Error::from)?;
  if (evidence.duration-other.duration).abs()>2.{continue;}if let Some(score)=similarity(&evidence.fingerprint,&other.fingerprint).filter(|score|*score>=0.97){items.push(json!({"track":candidate,"similarity":score,"basis":"chromaprint_and_edition_tags","recording_candidate":true,"release_verified":false}));}
 }
 Ok(Json(json!({"available":true,"algorithm":ALGORITHM,"sampled_seconds":evidence.sampled_seconds,"items":items,"candidate_scan_limit":100,"candidate_scan_truncated":truncated,"offset":offset,"next_offset":if truncated{Some(offset+100)}else{None},"automatic_merge":false,"remote_lookup":false})))
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn rejects_silence_short_nonfinite_and_bounded_fingerprints(){assert!(parse(br#"{"duration":120,"fingerprint":[1,2,3]}"#).is_err());let data=json!({"duration":120,"fingerprint":vec![1u32;300]});assert!(parse(data.to_string().as_bytes()).is_err());let words=(0..200u32).map(|i|i.wrapping_mul(123456789)).collect::<Vec<_>>();let good=json!({"duration":120,"fingerprint":words});assert!(parse(good.to_string().as_bytes()).is_ok());}
 #[test]fn fingerprint_score_distinguishes_different_recordings_and_small_codec_offsets(){let a=(0..300u32).map(|i|i.wrapping_mul(123456789)).collect::<Vec<_>>();let mut b=vec![999];b.extend(&a);assert_eq!(similarity(&a,&b),Some(1.));let c=a.iter().map(|n|!n).collect::<Vec<_>>();assert!(similarity(&a,&c).unwrap()<0.97);}
}
