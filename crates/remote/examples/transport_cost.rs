//! Isolated loopback transport measurement; no production Core or audio devices.
use axum::{body::Body,extract::WebSocketUpgrade,routing::get,Router};
use rmusic_remote::{identity::Identity,native::{open_core,serve_core}};
use serde_json::json;
use std::{sync::{Arc,atomic::{AtomicUsize,Ordering}},time::Instant};

#[tokio::main]
async fn main()->anyhow::Result<()> {
 let identity=Identity::create();let core=uuid::Uuid::new_v4().to_string();let tls=identity.tls(&core)?;
 let connections=Arc::new(AtomicUsize::new(0));let count=connections.clone();
 let inner=Router::new().route("/api/v1/control",get(||async{axum::Json(json!({"ok":true}))}))
  .route("/api/v1/media/test",get(||async{Body::from(vec![0u8;8*1024*1024])}));
 let cancel=tokio_util::sync::CancellationToken::new();let stop=cancel.clone();let tunneled=inner.clone();
 let router=inner.route("/api/v1/native/connect",get(move|ws:WebSocketUpgrade|{
  let tls=tls.clone();let inner=tunneled.clone();let cancel=stop.child_token();let count=count.clone();
  async move{ws.on_upgrade(move|socket|async move{
   count.fetch_add(1,Ordering::Relaxed);let(a,b)=tokio::io::duplex(256*1024);
   let wire=rmusic_remote::tunnel::bridge_server(socket,a,cancel.clone());
   let server=serve_core(b,inner,Some(tls),cancel.clone());
   let _=tokio::join!(wire,server);
  })}
 }));
 let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await?;let base=format!("http://{}",listener.local_addr()?);
 let server=tokio::spawn(async move{let _=axum::serve(listener,router).await;});
 let gateway=open_core(&base,core,identity.public_key(),None,false).await?;
 let http=reqwest::Client::builder().no_proxy().build()?;
 async fn measure(http:&reqwest::Client,base:&str)->anyhow::Result<serde_json::Value>{
  let mut times=Vec::new();
  for index in 0..65 {let at=Instant::now();let value=http.get(format!("{base}/api/v1/control")).send().await?.error_for_status()?.json::<serde_json::Value>().await?;assert_eq!(value["ok"],true);if index>0{times.push(at.elapsed().as_secs_f64()*1000.);}}
  times.sort_by(f64::total_cmp);let at=Instant::now();let mut response=http.get(format!("{base}/api/v1/media/test")).send().await?.error_for_status()?;let mut bytes=0;
  while let Some(chunk)=response.chunk().await?{bytes+=chunk.len();}assert_eq!(bytes,8*1024*1024);
  Ok(json!({"warm_control_p50_ms":times[32],"warm_control_p95_ms":times[60],"audio_bytes":bytes,"audio_elapsed_ms":at.elapsed().as_secs_f64()*1000.}))
 }
 let direct=measure(&http,&base).await?;let facade=measure(&http,&gateway.address).await?;
 println!("{}",json!({"scope":"loopback HTTP versus loopback HTTP + WebSocket + pinned Core TLS; all peers in one process; no audio decode","direct":direct,"facade":facade,"inner_connections":connections.load(Ordering::Relaxed)}));
 cancel.cancel();server.abort();Ok(())
}
