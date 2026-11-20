use anyhow::{Context, Result};
use rmusic_bridge::connection::ConnectionProfile;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

fn cover_source(source: &str) -> Result<String> {
    // QQ's directory still returns legacy HTTP image addresses. Upgrade only its
    // exact image host; never relax the external HTTPS policy or send Core auth.
    if source.starts_with("/api/v1/covers/") || source.starts_with("/api/v1/artwork/") { return Ok(source.to_owned()); }
    let mut url = reqwest::Url::parse(source)?;
    if url.scheme() == "http" && url.host_str() == Some("y.gtimg.cn") && url.port().is_none()
        && url.username().is_empty() && url.password().is_none() {
        url.set_scheme("https").map_err(|_|anyhow::anyhow!("封面协议无效"))?;
    }
    anyhow::ensure!(url.scheme()=="https"&&url.username().is_empty()&&url.password().is_none(),"封面地址须为 Core 图片或 HTTPS");
    Ok(url.to_string())
}
/// Artwork URLs come from authenticated Core tracks or collections. Credentials stay on exact Core cover paths.
pub async fn fetch(
    root: &Path,
    profile: &ConnectionProfile,
    token: &str,
    track: &Value,
) -> Result<Value> {
    let Some(source) = track["cover_url"].as_str().filter(|s| !s.is_empty()) else {
        return Ok(Value::Null);
    };
    anyhow::ensure!(source.len()<=4096,"封面地址过长");
    let source = cover_source(source)?;
    let source = source.as_str();
    if let Some(key)=source.strip_prefix("/api/v1/covers/").or_else(||source.strip_prefix("/api/v1/artwork/")) {
        anyhow::ensure!(key.len()==64&&key.bytes().all(|b|b.is_ascii_hexdigit()),"无效 Core 封面路径");
    } else {
        let url=reqwest::Url::parse(source)?;
        anyhow::ensure!(url.scheme()=="https"&&url.username().is_empty()&&url.password().is_none(),"封面地址须为 Core 图片或 HTTPS");
    }
    let key = format!("{:x}", Sha256::digest(source.as_bytes()));
    let path=root.join(format!("{key}.image"));
    let gate=gate(&path);let _guard=gate.lock().await;
    tokio::fs::create_dir_all(root).await?;
    let meta_path=root.join(format!("{key}.json"));
    let metadata:Value=match tokio::fs::read(&meta_path).await{Ok(bytes)if bytes.len()<=4096=>serde_json::from_slice(&bytes).unwrap_or(Value::Null),_=>Value::Null};
    let existing=read_valid(&path).await;
    let force=track["reload"].as_bool()==Some(true);
    let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs_f64();
    let valid=existing.as_ref().is_some_and(|b|metadata["sha"].as_str().is_none_or(|hash|hash==format!("{:x}",Sha256::digest(b))));
    let ttl=if source.starts_with("/api/v1/artwork/"){60.0}else{86400.0};
    let revalidate_after = track["revalidate"].as_f64().unwrap_or(0.0);
    if valid && !force && track["revalidate"] != true
        && metadata["checked"].as_f64().is_some_and(|at| at >= revalidate_after && now - at < ttl) {
        return Ok(json!({"path":path}));
    }
    if track["anonymous_metadata"]==true {
        if let Some(id)=source.strip_prefix("/api/v1/artwork/") {
            if let Ok(bytes)=crate::public_metadata::image(profile,token,id).await {
                anyhow::ensure!(complete(&bytes),"公开封面数据不完整");
                atomic_write(&path,&bytes).await?;
                atomic_write(&meta_path,&serde_json::to_vec(&json!({"checked":now,"sha":format!("{:x}",Sha256::digest(&bytes))}))?).await?;
                return Ok(json!({"path":path}));
            }
        }
    }
    let request=if source.starts_with("/api/v1/") {
        profile.http()?.get(profile.endpoint(source.trim_start_matches('/'))?).bearer_auth(token)
    }else{reqwest::Client::builder().https_only(true).build()?.get(source)};
    let mut request=request.timeout(Duration::from_secs(40));
    if valid&&!force {if let Some(etag)=metadata["etag"].as_str().filter(|v|v.len()<256){request=request.header("If-None-Match",etag);}}
    let response=request.send().await;
    let mut response=match response{Ok(r)=>r,Err(e)=>{if valid&&!force{return Ok(json!({"path":path,"stale":true}));}return Err(e.into());}};
    if response.status()==reqwest::StatusCode::NOT_MODIFIED&&valid {
        let mut updated=metadata;updated["checked"]=json!(now);atomic_write(&meta_path,&serde_json::to_vec(&updated)?).await?;return Ok(json!({"path":path}));
    }
    if !response.status().is_success(){
        if valid&&!force{return Ok(json!({"path":path,"stale":true}));}
        return Err(super::session::HttpFailure{status:response.status().as_u16(),message:"封面暂不可用".into()}.into());
    }
    anyhow::ensure!(response.headers().get("Content-Type").and_then(|v|v.to_str().ok()).is_some_and(|v|v.starts_with("image/")),"封面响应不是图片");
    let etag=response.headers().get("ETag").and_then(|v|v.to_str().ok()).filter(|v|v.len()<256).map(String::from);
    let mut bytes=Vec::new();
    while let Some(chunk)=response.chunk().await?{anyhow::ensure!(bytes.len()+chunk.len()<=20*1024*1024,"封面超过20 MiB");bytes.extend_from_slice(&chunk);}
    anyhow::ensure!(complete(&bytes),"封面数据不完整");
    let metadata=json!({"etag":etag,"checked":now,"sha":format!("{:x}",Sha256::digest(&bytes))});
    atomic_write(&path,&bytes).await.context("无法写入封面缓存")?;
    atomic_write(&meta_path,&serde_json::to_vec(&metadata)?).await?;
    Ok(json!({"path":path}))
}
static GATES:std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<std::path::PathBuf,std::sync::Weak<tokio::sync::Mutex<()>>>>>=std::sync::LazyLock::new(Default::default);
fn gate(path:&Path)->std::sync::Arc<tokio::sync::Mutex<()>>{let mut gates=GATES.lock().unwrap();gates.retain(|_,v|v.strong_count()>0);let gate=gates.get(path).and_then(std::sync::Weak::upgrade).unwrap_or_default();gates.insert(path.into(),std::sync::Arc::downgrade(&gate));gate}
fn complete(bytes:&[u8])->bool{
 if bytes.starts_with(b"\x89PNG\r\n\x1a\n"){bytes.len()>=24&&bytes.ends_with(b"\0\0\0\0IEND\xaeB`\x82")}
 else if bytes.starts_with(b"\xff\xd8\xff"){bytes[bytes.len().saturating_sub(64)..].windows(2).any(|v|v==b"\xff\xd9")}
 else{bytes.len()>=16&&bytes.starts_with(b"RIFF")&&bytes.get(8..12)==Some(b"WEBP")&&u32::from_le_bytes(bytes[4..8].try_into().unwrap())as usize+8==bytes.len()}
}
async fn read_valid(path:&Path)->Option<Vec<u8>>{let size=tokio::fs::metadata(path).await.ok()?.len();if size==0||size>20*1024*1024{return None;}let bytes=tokio::fs::read(path).await.ok()?;complete(&bytes).then_some(bytes)}
struct Temp(std::path::PathBuf);impl Drop for Temp{fn drop(&mut self){let _=std::fs::remove_file(&self.0);}}
async fn atomic_write(path:&Path,bytes:&[u8])->Result<()>{let temp=Temp(path.with_extension(format!("{}.tmp",uuid::Uuid::new_v4())));tokio::fs::write(&temp.0,bytes).await?;tokio::fs::rename(&temp.0,path).await?;Ok(())}

pub async fn fetch_track(
    root: std::path::PathBuf,
    profile: ConnectionProfile,
    token: String,
    id: String,
    mut options: Value,
) -> Result<Value> {
    anyhow::ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)),
        "invalid track identity"
    );
    let track: Value = profile
        .http()?
        .get(profile.endpoint(&format!("api/v1/library/tracks/{id}"))?)
        .bearer_auth(&token)
        .timeout(Duration::from_secs(40))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    options["cover_url"] = track["cover_url"].clone();
    fetch(&root, &profile, &token, &options).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn legacy_qq_covers_upgrade_without_allowing_other_http_or_credentials() {
        assert_eq!(cover_source("http://y.gtimg.cn/music/cover.jpg?n=1").unwrap(), "https://y.gtimg.cn/music/cover.jpg?n=1");
        for bad in ["http://example.com/a.jpg", "http://y.gtimg.cn.example.com/a.jpg", "http://user:pass@y.gtimg.cn/a.jpg", "http://y.gtimg.cn:8080/a.jpg"] {
            assert!(cover_source(bad).is_err());
        }
    }

    fn profile(address: String) -> ConnectionProfile {
        ConnectionProfile {core_public_key:None, id: "artwork-test".into(), name: "QA".into(), address,
            certificate_sha256: None, skip_certificate_verification: false,
            alternatives: vec![], core_id: None }
    }

    #[tokio::test]
    async fn collection_cover_uses_its_core_url_without_requesting_a_track() -> Result<()> {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let profile=profile(format!("http://{}/",listener.local_addr()?));
        let cover=format!("/api/v1/covers/{}","a".repeat(64));
        let expected=cover.clone();
        let server=tokio::spawn(async move {
            let (mut socket,_)=listener.accept().await.unwrap();
            let mut bytes=vec![0;8192];let mut used=0;
            while !bytes[..used].windows(4).any(|s|s==b"\r\n\r\n") {
                let n=socket.read(&mut bytes[used..]).await.unwrap();assert!(n>0);used+=n;
            }
            let request=String::from_utf8_lossy(&bytes[..used]).to_lowercase();
            assert!(request.starts_with(&format!("get {} http/1.1",expected)));
            assert!(request.contains("authorization: bearer test-only"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 6\r\nConnection: close\r\n\r\n\xff\xd8\xff\xe0\xff\xd9").await.unwrap();
        });
        let root=std::env::temp_dir().join(format!("rmusic-artwork-test-{}",uuid::Uuid::new_v4()));
        let value=fetch(&root,&profile,"test-only",&json!({"id":"album-not-track","cover_url":cover})).await?;
        assert_eq!(tokio::fs::read(value["path"].as_str().unwrap()).await?,b"\xff\xd8\xff\xe0\xff\xd9");
        tokio::time::timeout(Duration::from_secs(3),server).await??;
        tokio::fs::remove_dir_all(root).await?;
        Ok(())
    }

    #[tokio::test]
    async fn invalid_cover_paths_are_rejected_before_creating_a_cache() {
        let root=std::env::temp_dir().join(format!("rmusic-artwork-invalid-{}",uuid::Uuid::new_v4()));
        let profile=profile("http://127.0.0.1:9/".into());
        for source in ["/api/v1/covers/../../settings","/api/v1/artwork/../../settings","http://example.test/cover","https://user:password@example.test/cover","/api/v1/library/tracks/album/cover"] {
            assert!(fetch(&root,&profile,"never-send",&json!({"cover_url":source})).await.is_err());
        }
        assert!(!root.exists());
    }
}

#[cfg(test)]mod recovery_tests{
 use super::*;use tokio::io::{AsyncReadExt,AsyncWriteExt};
 #[tokio::test]async fn disk_loss_corruption_and_conditional_refresh_do_not_repeat_good_bodies()->Result<()>{
  let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await?;let address=format!("http://{}/",listener.local_addr()?);
  let server=tokio::spawn(async move{
   for index in 0..4{let(mut sock,_)=listener.accept().await.unwrap();let mut b=vec![0;4096];let n=sock.read(&mut b).await.unwrap();let request=String::from_utf8_lossy(&b[..n]);
    if index==1{assert!(request.to_lowercase().contains("if-none-match: \"v1\""));sock.write_all(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();}
    else{assert!(!request.to_lowercase().contains("if-none-match"));sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nETag: \"v1\"\r\nContent-Length: 6\r\nConnection: close\r\n\r\n\xff\xd8\xff\xe0\xff\xd9").await.unwrap();}
   }
  });
  let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"qa".into(),address,certificate_sha256:None,skip_certificate_verification:false,alternatives:vec![],core_id:None};
  let root=std::env::temp_dir().join(format!("rmusic-artwork-recovery-{}",uuid::Uuid::new_v4()));let cover=json!({"cover_url":format!("/api/v1/artwork/{}","b".repeat(64))});
  let(a,b)=tokio::join!(fetch(&root,&profile,"qa",&cover),fetch(&root,&profile,"qa",&cover));let path=std::path::PathBuf::from(a?["path"].as_str().unwrap());assert_eq!(b?["path"].as_str(),path.to_str());
  let mut revalidate=cover.clone();
  revalidate["revalidate"]=json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs_f64());
  fetch(&root,&profile,"qa",&revalidate).await?;
  fetch(&root,&profile,"qa",&revalidate).await?;
  tokio::fs::remove_file(&path).await?;fetch(&root,&profile,"qa",&cover).await?;
  tokio::fs::write(&path,b"broken").await?;fetch(&root,&profile,"qa",&cover).await?;
  assert_eq!(tokio::fs::read(path).await?,b"\xff\xd8\xff\xe0\xff\xd9");tokio::time::timeout(Duration::from_secs(5),server).await??;tokio::fs::remove_dir_all(root).await?;Ok(())
 }
}
