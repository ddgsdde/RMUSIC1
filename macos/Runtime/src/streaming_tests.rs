//! Generated audio and a private HTTP fixture only. No output device is created.
use super::*;
use serde_json::json;
use tokio::io::{AsyncReadExt,AsyncWriteExt};
use std::sync::atomic::{AtomicUsize,Ordering};

#[tokio::test]
async fn remote_flac_reuses_original_cache_and_keeps_account_ownership()->Result<()> {
 let temp=tempfile::tempdir()?;let file=temp.path().join("fixture.flac");
 let encoded=std::process::Command::new("ffmpeg").args(["-v","error","-f","lavfi","-i","sine=frequency=440:sample_rate=44100:duration=8","-ac","2","-sample_fmt","s16","-c:a","flac"]).arg(&file).status()?;
 anyhow::ensure!(encoded.success(),"fixture FLAC encoding failed");
 let audio=Arc::new(tokio::fs::read(&file).await?);
 let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await?;let address=listener.local_addr()?;
 let gets=Arc::new(AtomicUsize::new(0));let conversions=Arc::new(AtomicUsize::new(0));let source=audio.clone();let counted=gets.clone();let converted=conversions.clone();
 let server=tokio::spawn(async move{loop{
  let Ok((mut socket,_))=listener.accept().await else{break;};let audio=source.clone();let gets=counted.clone();let conversions=converted.clone();
  tokio::spawn(async move{
   let mut request=Vec::new();let mut block=[0;1024];while !request.windows(4).any(|v|v==b"\r\n\r\n"){let n=socket.read(&mut block).await.unwrap_or(0);if n==0{return;}request.extend_from_slice(&block[..n]);if request.len()>16384{return;}}
   let text=String::from_utf8_lossy(&request);let first=text.lines().next().unwrap_or("");let head=first.starts_with("HEAD ");let path=first.split_whitespace().nth(1).unwrap_or("");
   let track=json!({"id":"test","title":"Transport QA","artist":"QA","album":"QA","album_artist":"QA","duration":8,"track_number":1,"disc_number":1,"source":"local","format":"flac","sample_rate":44100,"bit_depth":16,"channels":2,"bitrate":128000,"recording_id":"qa","cover_url":null,"available":true,"created_at":0,"verified":true});
   let (body,mime)=if path.starts_with("/api/v1/library/tracks/test"){(serde_json::to_vec(&track).unwrap(),"application/json")}else if path.starts_with("/api/v1/media/test"){if !head{gets.fetch_add(1,Ordering::SeqCst);}((*audio).clone(),"audio/flac")}else{conversions.fetch_add(1,Ordering::SeqCst);(b"unexpected conversion".to_vec(),"text/plain")};
   let headers=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {mime}\r\nETag: \"fixture-original\"\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",body.len());if socket.write_all(headers.as_bytes()).await.is_err(){return;}
   if !head {for part in body.chunks(16384){if socket.write_all(part).await.is_err(){break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}}
  });
 }});
 let profile:ConnectionProfile=serde_json::from_value(json!({"id":"qa","name":"QA","address":format!("http://{address}")}))?;
 let mut policy=StreamingPolicy::default();policy.mode="remote".into();
 let local=crate::downloads::Downloads::open(temp.path(),"qa-core","user-a",profile.clone(),"synthetic".into())?;
 let media=ClientMedia{local,policy:Arc::new(RwLock::new(policy.clone())),token:"synthetic".into(),capability:tokio::sync::OnceCell::new()};
 let url=format!("http://{address}/api/v1/media/test?ticket=synthetic");
 let proxy=media.resolve(&profile,"test",&url).await?;let bytes=reqwest::get(&proxy.url).await?.error_for_status()?.bytes().await?;
 assert_eq!(&bytes[..],audio.as_slice());drop(proxy);tokio::time::sleep(std::time::Duration::from_millis(650)).await;
 let cached = media.resolve(&profile, "test", &url).await?;
 assert!(std::path::Path::new(&cached.url).is_absolute());
 assert_eq!(tokio::fs::read(&cached.url).await?, *audio);
 assert_eq!(gets.load(Ordering::SeqCst), 1, "second playback must use the complete local cache");
 let other=ClientMedia{local:crate::downloads::Downloads::open(temp.path(),"qa-core","user-b",profile.clone(),"other-synthetic".into())?,policy:Arc::new(RwLock::new(policy)),token:"other-synthetic".into(),capability:tokio::sync::OnceCell::new()};
 let separate=other.resolve(&profile,"test",&url).await?;let _=reqwest::get(&separate.url).await?.bytes().await?;assert_eq!(gets.load(Ordering::SeqCst),2);assert_eq!(conversions.load(Ordering::SeqCst),0,"original FLAC must not be transcoded");
 drop(separate);drop(cached);drop(other);drop(media);tokio::time::sleep(std::time::Duration::from_millis(650)).await;server.abort();Ok(())
}
