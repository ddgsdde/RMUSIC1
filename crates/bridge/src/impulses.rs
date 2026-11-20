//! Content-verified, bounded response loading. Dropping the future closes network reads.
use crate::connection::ConnectionProfile;
use anyhow::{bail,Context,Result};
use futures_util::{stream,StreamExt};
use rmusic_audio::impulse::{ConvolutionConfig,ImpulseBank,ImpulseResponse,MAX_WAV_BYTES};
use serde::Deserialize;
use sha2::{Digest,Sha256};
use std::{collections::HashSet,sync::Arc};

pub use rmusic_audio::impulse::{MAX_BANK_BYTES,MAX_BANK_RESPONSES};
#[derive(Deserialize)]
struct Asset {id:String,bytes:u64,sample_rate:u32,channels:usize,frames:usize,channel_mask:u32}
#[derive(Deserialize)]
struct Download {asset:Asset,api_path:String}
#[derive(Deserialize)]
struct Manifest {items:Vec<Download>}
struct Source {id:String,url:url::Url,asset:Option<Asset>}
pub async fn load(profile:&ConnectionProfile,token:&str,ids:&[String])->Result<Arc<ImpulseBank>> {
    ConvolutionConfig{response_ids:ids.to_vec()}.validate()?;
    let sources=ids.iter().map(|id|Ok(Source{id:id.clone(),url:profile.endpoint(&format!("api/v1/dsp/impulses/{id}/file"))?,asset:None})).collect::<Result<Vec<_>>>()?;
    load_sources(profile.http()?,token,sources).await
}
pub async fn load_selected(profile:&ConnectionProfile,token:&str,zone_id:&str,ids:&[String])->Result<Arc<ImpulseBank>> {
    let selection=ConvolutionConfig{response_ids:ids.to_vec()};selection.validate()?;
    let client=profile.http()?;
    let zone=url::form_urlencoded::byte_serialize(zone_id.as_bytes()).collect::<String>().replace('+',"%20");
    let mut response=client.post(profile.endpoint(&format!("api/v1/zones/{zone}/dsp/impulses"))?).bearer_auth(token).json(&selection).send().await?;
    if response.status()==reqwest::StatusCode::CONFLICT {bail!("HTTP 409: 音区响应组已改变，请重新读取当前设置");}
    if !response.status().is_success(){bail!("HTTP {}: 无法取得当前音区的卷积响应票据",response.status().as_u16());}
    if response.content_length().is_some_and(|size|size>65536){bail!("卷积资源票据响应过大");}
    let mut body=Vec::new();while let Some(chunk)=response.chunk().await?{if body.len()+chunk.len()>65536{bail!("卷积资源票据响应过大");}body.extend_from_slice(&chunk);}
    let manifest:Manifest=serde_json::from_slice(&body).context("卷积资源票据响应无效")?;
    let mut remaining=ids.iter().cloned().collect::<HashSet<_>>();let mut sources=Vec::new();
    for item in manifest.items {
        if !remaining.remove(&item.asset.id)||!item.api_path.starts_with(&format!("api/v1/dsp/impulses/{}/file?",item.asset.id)){bail!("卷积资源票据与所选文件不一致");}
        let url=profile.endpoint(&item.api_path)?;
        if !url.query_pairs().any(|(key,value)|key=="ticket"&&!value.is_empty())||!url.query_pairs().any(|(key,value)|key=="zone_id"&&value==zone_id)||!url.query_pairs().any(|(key,value)|key=="selection"&&!value.is_empty()){bail!("卷积资源票据缺少当前音区授权");}
        sources.push(Source{id:item.asset.id.clone(),url,asset:Some(item.asset)});
    }
    if !remaining.is_empty(){bail!("卷积资源票据缺少所选文件");}load_sources(client,token,sources).await
}
async fn load_sources(client:reqwest::Client,token:&str,sources:Vec<Source>)->Result<Arc<ImpulseBank>> {
    let mut pending=stream::iter(sources.into_iter().map(|source|{let client=&client;async move {
        let mut response=client.get(source.url).bearer_auth(token).send().await.map_err(|error|error.without_url())?;
        let status=response.status();
        if !status.is_success(){bail!("HTTP {}: 无法读取所选卷积响应，请检查权限、连接和文件是否仍存在",status.as_u16());}
        if response.content_length().is_some_and(|bytes|bytes>MAX_WAV_BYTES as u64){bail!("卷积响应文件超过 4 MiB");}
        let mut bytes=Vec::new();
        while let Some(chunk)=response.chunk().await.map_err(|error|error.without_url())? {
            if bytes.len().saturating_add(chunk.len())>MAX_WAV_BYTES {bail!("卷积响应文件超过 4 MiB");}
            bytes.extend_from_slice(&chunk);
        }
        if format!("{:x}",Sha256::digest(&bytes))!=source.id {bail!("卷积响应校验失败，请重新导入原文件");}
        let size=bytes.len();
        let impulse=tokio::task::spawn_blocking(move||ImpulseResponse::from_wav(&bytes)).await?.context("卷积响应 WAV 无效")?;
        if let Some(asset)=source.asset{if asset.bytes!=size as u64||asset.sample_rate!=impulse.sample_rate()||asset.channels!=impulse.channels()||asset.frames!=impulse.frames()||asset.channel_mask!=impulse.channel_mask(){bail!("卷积响应元数据与原文件不一致");}}
        Ok::<_,anyhow::Error>((size,Arc::new(impulse)))
    }})).buffer_unordered(4);
    let(mut loaded,mut total)=(Vec::new(),0usize);
    while let Some(response)=pending.next().await {
        let(size,response)=response?;total+=size;if total>MAX_BANK_BYTES {bail!("响应组原文件超过 32 MiB 的加载预算");}loaded.push(response);
    }
    Ok(Arc::new(ImpulseBank::new(loaded)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::{Body,Bytes},extract::{Path,Query},http::{HeaderMap,StatusCode},response::{IntoResponse,Response},routing::{get,post},Json,Router};
    use std::{collections::HashMap,sync::atomic::{AtomicUsize,Ordering},time::Duration};
    fn wav(rate:u32,gain:f64)->Vec<u8>{
        let values=[gain,-3.,-0.];let mut bytes=b"RIFF".to_vec();bytes.extend(60_u32.to_le_bytes());bytes.extend(b"WAVEfmt ");bytes.extend(16_u32.to_le_bytes());bytes.extend(3_u16.to_le_bytes());bytes.extend(1_u16.to_le_bytes());bytes.extend(rate.to_le_bytes());bytes.extend((rate*8).to_le_bytes());bytes.extend(8_u16.to_le_bytes());bytes.extend(64_u16.to_le_bytes());bytes.extend(b"data");bytes.extend(24_u32.to_le_bytes());for value in values{bytes.extend(value.to_le_bytes());}bytes
    }
    fn id(bytes:&[u8])->String{format!("{:x}",Sha256::digest(bytes))}
    fn profile(address:std::net::SocketAddr)->ConnectionProfile{ConnectionProfile{core_public_key:None,id:"test".into(),name:"QA".into(),address:format!("http://{address}/room"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None}}
    async fn server(router:Router)->(ConnectionProfile,tokio::task::JoinHandle<()>) {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(listener.local_addr().unwrap());let task=tokio::spawn(async move{axum::serve(listener,Router::new().nest("/room",router)).await.unwrap();});(profile,task)
    }
    fn manifest(bytes:&[u8],zone:&str)->serde_json::Value {
        let id=id(bytes);let query=url::form_urlencoded::Serializer::new(String::new()).append_pair("ticket","scoped-token").append_pair("zone_id",zone).append_pair("selection",&"f".repeat(64)).finish();
        serde_json::json!({"items":[{"asset":{"id":id,"name":"room.wav","bytes":bytes.len(),"sample_rate":48000,"channels":1,"frames":3,"channel_mask":0,"created_at":1},"api_path":format!("api/v1/dsp/impulses/{id}/file?{query}")}],"expires_at":300})
    }
    #[tokio::test]
    async fn selected_load_uses_control_manifest_and_its_scoped_file_url_without_read_scope(){
        let bytes=wav(48000,2.);let selected=id(&bytes);let expected=selected.clone();let response=manifest(&bytes,"zone test");
        let router=Router::new().route("/api/v1/zones/{zone}/dsp/impulses",post(move|Path(zone):Path<String>,headers:HeaderMap,Json(body):Json<serde_json::Value>|{let expected=expected.clone();let response=response.clone();async move{
            assert_eq!(zone,"zone test");assert_eq!(headers["authorization"],"Bearer control-only");assert_eq!(body,serde_json::json!({"response_ids":[expected]}));Json(response)
        }})).route("/api/v1/dsp/impulses/{id}/file",get(move|Path(id):Path<String>,Query(query):Query<HashMap<String,String>>,headers:HeaderMap|{let bytes=bytes.clone();let selected=selected.clone();async move{
            assert_eq!(id,selected);assert_eq!(query["ticket"],"scoped-token");assert_eq!(query["zone_id"],"zone test");assert_eq!(headers["authorization"],"Bearer control-only");bytes
        }}));let(profile,task)=server(router).await;
        let bank=load_selected(&profile,"control-only","zone test",&[id(&wav(48000,2.))]).await.unwrap();assert_eq!(bank.response(48000).unwrap().kernels()[0],[2.,-3.,-0.]);task.abort();
    }
    #[tokio::test]
    async fn forged_or_incomplete_manifests_never_request_files_or_fall_back_to_global_read(){
        let bytes=wav(48000,2.);let selected=id(&bytes);let good=manifest(&bytes,"zone test");let current=Arc::new(std::sync::Mutex::new(good.clone()));let response=current.clone();let files=Arc::new(AtomicUsize::new(0));let requested=files.clone();
        let router=Router::new().route("/api/v1/zones/{zone}/dsp/impulses",post(move||{let response=response.clone();async move{Json(response.lock().unwrap().clone())}})).route("/api/v1/dsp/impulses/{id}/file",get(move||{let bytes=bytes.clone();let requested=requested.clone();async move{requested.fetch_add(1,Ordering::Relaxed);bytes}}));let(profile,task)=server(router).await;
        let mut wrong_id=good.clone();wrong_id["items"][0]["asset"]["id"]=serde_json::json!("0".repeat(64));let mut duplicate=good.clone();duplicate["items"]=serde_json::json!([good["items"][0],good["items"][0]]);
        let mut cases=vec![serde_json::json!({"items":[]}),wrong_id,duplicate];
        for path in [format!("https://outside.invalid/api/v1/dsp/impulses/{selected}/file?ticket=x"),"api/v1/media/other?ticket=x".into(),format!("api/v1/dsp/impulses/{selected}/file?zone_id=zone+test&selection=x"),format!("api/v1/dsp/impulses/{selected}/file?ticket=x&zone_id=wrong&selection=x"),format!("api/v1/dsp/impulses/{selected}/file?ticket=x&zone_id=zone+test")]{let mut bad=good.clone();bad["items"][0]["api_path"]=serde_json::json!(path);cases.push(bad);}
        for case in cases{*current.lock().unwrap()=case;assert!(tokio::time::timeout(Duration::from_secs(2),load_selected(&profile,"control-only","zone test",&[selected.clone()])).await.unwrap().is_err());}
        assert_eq!(files.load(Ordering::Relaxed),0);task.abort();
    }
    #[tokio::test]
    async fn scoped_file_metadata_must_match_the_verified_original_wav(){
        let bytes=wav(48000,2.);let selected=id(&bytes);let good=manifest(&bytes,"zone");let current=Arc::new(std::sync::Mutex::new(good.clone()));let response=current.clone();
        let router=Router::new().route("/api/v1/zones/{zone}/dsp/impulses",post(move||{let response=response.clone();async move{Json(response.lock().unwrap().clone())}})).route("/api/v1/dsp/impulses/{id}/file",get(move||{let bytes=bytes.clone();async move{bytes}}));let(profile,task)=server(router).await;
        for(key,value)in[("bytes",69),("sample_rate",44100),("channels",2),("frames",2),("channel_mask",3)]{let mut bad=good.clone();bad["items"][0]["asset"][key]=serde_json::json!(value);*current.lock().unwrap()=bad;assert!(load_selected(&profile,"control-only","zone",&[selected.clone()]).await.unwrap_err().to_string().contains("元数据与原文件不一致"));}task.abort();
    }
    #[tokio::test]
    async fn selected_load_bounds_manifest_bytes_and_keeps_conflicts_explicit(){
        let mode=Arc::new(AtomicUsize::new(0));let response=mode.clone();let files=Arc::new(AtomicUsize::new(0));let requested=files.clone();let selected=id(&wav(48000,2.));
        let router=Router::new().route("/api/v1/zones/{zone}/dsp/impulses",post(move||{let response=response.clone();async move{match response.load(Ordering::Relaxed){0=>StatusCode::CONFLICT.into_response(),1=>Response::new(Body::from(vec![b'x';65537])),_=>Response::new(Body::from_stream(stream::iter((0..3).map(|_|Ok::<_,std::convert::Infallible>(Bytes::from(vec![b'x';32768]))))))}}})).route("/api/v1/dsp/impulses/{id}/file",get(move||{let requested=requested.clone();async move{requested.fetch_add(1,Ordering::Relaxed);StatusCode::OK}}));let(profile,task)=server(router).await;
        for index in 0..3{mode.store(index,Ordering::Relaxed);let error=load_selected(&profile,"control-only","zone",&[selected.clone()]).await.unwrap_err().to_string();assert!(error.contains(if index==0{"HTTP 409"}else{"票据响应过大"}),"{error}");}assert_eq!(files.load(Ordering::Relaxed),0);task.abort();
    }
    #[tokio::test]
    async fn cancelling_selected_load_closes_manifest_headers_and_body_reads(){
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        for body_started in [false,true]{
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(listener.local_addr().unwrap());let pending=tokio::spawn(async move{load_selected(&profile,"control-only","zone",&["0".repeat(64)]).await});
            let(mut socket,_)=tokio::time::timeout(Duration::from_secs(2),listener.accept()).await.unwrap().unwrap();let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n"){headers.push(socket.read_u8().await.unwrap());}let text=String::from_utf8(headers).unwrap().to_ascii_lowercase();let length=text.lines().find_map(|line|line.strip_prefix("content-length:")).unwrap().trim().parse::<usize>().unwrap();let mut body=vec![0;length];socket.read_exact(&mut body).await.unwrap();
            if body_started{socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\nContent-Type: application/json\r\n\r\n{\"items\":[").await.unwrap();}pending.abort();assert!(pending.await.unwrap_err().is_cancelled());assert!(matches!(tokio::time::timeout(Duration::from_secs(2),socket.read_u8()).await.unwrap().unwrap_err().kind(),std::io::ErrorKind::UnexpectedEof|std::io::ErrorKind::ConnectionReset));
        }
    }
    #[tokio::test]
    async fn real_http_keeps_auth_reverse_proxy_identity_and_raw_gain_for_each_selected_rate(){
        let a=wav(48000,2.);let b=wav(96000,-5.);let ids=vec![id(&b),id(&a)];let data=Arc::new(HashMap::from([(id(&a),a),(id(&b),b)]));
        let router=Router::new().route("/api/v1/dsp/impulses/{id}/file",get(move|Path(id):Path<String>,headers:HeaderMap|{let data=data.clone();async move{
            if headers.get("authorization").and_then(|value|value.to_str().ok())!=Some("Bearer synthetic-reader"){return StatusCode::FORBIDDEN.into_response();}
            ([("content-type","audio/wav")],data[&id].clone()).into_response()
        }}));let(profile,task)=server(router).await;
        let bank=load(&profile,"synthetic-reader",&ids).await.unwrap();assert_eq!(bank.response(48000).unwrap().kernels()[0][..2],[2.,-3.]);assert_eq!(bank.response(96000).unwrap().kernels()[0][..2],[-5.,-3.]);assert_eq!(bank.response(48000).unwrap().kernels()[0][2].to_bits(),(-0_f64).to_bits());assert!(bank.response(44100).is_err());
        assert!(load(&profile,"wrong",&ids).await.unwrap_err().to_string().contains("HTTP 403"));task.abort();
    }
    #[tokio::test]
    async fn invalid_or_duplicate_ids_are_rejected_before_any_network_request(){
        let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();let router=Router::new().fallback(move||{let count=count.clone();async move{count.fetch_add(1,Ordering::Relaxed);StatusCode::NOT_FOUND}});let(profile,task)=server(router).await;
        for ids in [vec![],vec!["../secret".into()],vec!["A".repeat(64)],vec!["g".repeat(64)],vec!["0".repeat(64);2],vec!["0".repeat(64);MAX_BANK_RESPONSES+1]] {assert!(load(&profile,"reader",&ids).await.is_err());}
        assert_eq!(calls.load(Ordering::Relaxed),0);task.abort();
    }
    #[tokio::test]
    async fn checksum_and_wav_validation_both_reject_without_substituting_a_response(){
        let good=wav(48000,2.);let expected=id(&good);let mut altered=good;altered[44..52].copy_from_slice(&4_f64.to_le_bytes());let invalid=b"not a WAV".to_vec();let invalid_id=id(&invalid);
        let router=Router::new().route("/api/v1/dsp/impulses/{id}/file",get(move|Path(id):Path<String>|{let altered=altered.clone();let invalid=invalid.clone();let expected=expected.clone();async move{if id==expected{altered}else{invalid}}}));let(profile,task)=server(router).await;
        assert!(load(&profile,"reader",&[id(&wav(48000,2.))]).await.unwrap_err().to_string().contains("校验失败"));assert!(load(&profile,"reader",&[invalid_id]).await.unwrap_err().to_string().contains("WAV 无效"));task.abort();
    }
    #[tokio::test]
    async fn known_length_and_chunked_files_are_bounded_before_checksum_or_parse(){
        let router=Router::new().route("/api/v1/dsp/impulses/{id}/file",get(|Path(id):Path<String>|async move{
            if id.starts_with('0'){return Response::new(Body::from(vec![0;MAX_WAV_BYTES+1]));}
            Response::new(Body::from_stream(stream::iter((0..5).map(|_|Ok::<_,std::convert::Infallible>(Bytes::from(vec![0;1024*1024]))))))
        }));let(profile,task)=server(router).await;
        for id in ["0".repeat(64),"1".repeat(64)]{assert!(load(&profile,"reader",&[id]).await.unwrap_err().to_string().contains("超过 4 MiB"));}task.abort();
    }
    #[tokio::test]
    async fn raw_bank_budget_also_counts_ignored_wav_chunks(){
        let mut files=HashMap::new();
        for rate in 8000..8009 {
            let mut bytes=wav(rate,1.);let padding=MAX_WAV_BYTES-bytes.len()-8;bytes.extend(b"JUNK");bytes.extend((padding as u32).to_le_bytes());bytes.resize(MAX_WAV_BYTES,0);bytes[4..8].copy_from_slice(&((MAX_WAV_BYTES-8) as u32).to_le_bytes());files.insert(id(&bytes),bytes);
        }
        let ids=files.keys().cloned().collect::<Vec<_>>();let files=Arc::new(files);let router=Router::new().route("/api/v1/dsp/impulses/{id}/file",get(move|Path(id):Path<String>|{let files=files.clone();async move{files[&id].clone()}}));let(profile,task)=server(router).await;
        assert!(load(&profile,"reader",&ids).await.unwrap_err().to_string().contains("超过 32 MiB"));task.abort();
    }
    #[tokio::test]
    async fn at_most_four_files_are_requested_while_slow_responses_are_pending(){
        let files=Arc::new((8000..8008).map(|rate|{let bytes=wav(rate,1.);(id(&bytes),bytes)}).collect::<HashMap<_,_>>());let ids=files.keys().cloned().collect::<Vec<_>>();let gate=Arc::new(tokio::sync::Semaphore::new(0));let release=gate.clone();let(sent,mut received)=tokio::sync::mpsc::unbounded_channel();
        let router=Router::new().route("/api/v1/dsp/impulses/{id}/file",get(move|Path(id):Path<String>|{let files=files.clone();let gate=gate.clone();let sent=sent.clone();async move{sent.send(()).unwrap();gate.acquire().await.unwrap().forget();files[&id].clone()}}));let(profile,task)=server(router).await;
        let pending=tokio::spawn(async move{load(&profile,"reader",&ids).await});for _ in 0..4{tokio::time::timeout(Duration::from_secs(2),received.recv()).await.unwrap().unwrap();}
        assert!(tokio::time::timeout(Duration::from_millis(100),received.recv()).await.is_err());release.add_permits(8);pending.await.unwrap().unwrap();for _ in 0..4{received.recv().await.unwrap();}task.abort();
    }
    #[tokio::test]
    async fn dropping_a_load_closes_pending_response_headers_and_bodies(){
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        for body_started in [false,true]{
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(listener.local_addr().unwrap());let pending=tokio::spawn(async move{load(&profile,"reader",&["0".repeat(64)]).await});
            let(mut socket,_)=tokio::time::timeout(Duration::from_secs(2),listener.accept()).await.unwrap().unwrap();let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n"){headers.push(socket.read_u8().await.unwrap());}
            if body_started{socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nRIFF").await.unwrap();}
            pending.abort();assert!(pending.await.unwrap_err().is_cancelled());assert!(matches!(tokio::time::timeout(Duration::from_secs(2),socket.read_u8()).await.unwrap().unwrap_err().kind(),std::io::ErrorKind::UnexpectedEof|std::io::ErrorKind::ConnectionReset));
        }
    }
}
