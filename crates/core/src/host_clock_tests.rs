use super::*;
use futures_util::{SinkExt,StreamExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest,Message};
type Socket=tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn next(socket:&mut Socket,kind:&str)->Value {
    tokio::time::timeout(std::time::Duration::from_secs(4),async {
        loop {match socket.next().await.unwrap().unwrap() {
            Message::Text(text)=>{let v:Value=serde_json::from_str(&text).unwrap();if v["type"]==kind{return v;}},
            Message::Ping(data)=>socket.send(Message::Pong(data)).await.unwrap(),_=>{}
        }}
    }).await.unwrap()
}
async fn barrier(socket:&mut Socket) {
    socket.send(Message::Ping(vec![7,8,9].into())).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(4),async{loop{match socket.next().await.unwrap().unwrap(){Message::Pong(data) if data[..]==[7,8,9]=>break,Message::Ping(data)=>socket.send(Message::Pong(data)).await.unwrap(),_=>{}}}}).await.unwrap();
}
#[tokio::test]
async fn programme_clock_exchange_is_owned_one_use_and_cleared_on_replacement_or_disconnect() {
    let f=Fixture::new().await;let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let app=f.app.clone();
    let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
    let mut request=format!("ws://{address}/api/v1/bridge/events").into_client_request().unwrap();request.headers_mut().insert("Authorization",format!("Bearer {}",f.token).parse().unwrap());
    let(mut socket,_)=tokio_tungstenite::connect_async(request.clone()).await.unwrap();
    socket.send(Message::Text(json!({"type":"register","protocol_version":1,"endpoint":{"id":"clock-qa","capabilities":{"host_clock_v1":true,"device_clock_v1":true,"strict_sync":false}}}).to_string().into())).await.unwrap();
    let p=next(&mut socket,"clock_probe").await;let host=uuid::Uuid::new_v4().to_string();let offset=5_000_000_000u64;
    let t2=crate::clock::now_ns()+offset;let mut reply=p.clone();reply["type"]=json!("clock_reply");reply["host_clock_id"]=json!(host);reply["t2_ns"]=json!(t2);reply["t3_ns"]=json!(crate::clock::now_ns()+offset);
    let power=json!({"host_id":uuid::Uuid::new_v4().to_string(),"generation":9,"wake_generation":2});reply["group_power"]=power.clone();
    socket.send(Message::Text(reply.to_string().into())).await.unwrap();barrier(&mut socket).await;
    let estimate=f.s.endpoints.read().await["clock-qa"]["clock_calibration"].clone();assert_eq!(estimate["sample_count"],1);assert_eq!(estimate["host_clock_id"],host);assert_eq!(estimate["device_clock_mapped"],false);
    let measured=estimate["host_minus_core_ns"].as_i64().unwrap();assert!((measured-offset as i64).unsigned_abs()<=estimate["network_uncertainty_ns"].as_u64().unwrap()+1);
    assert_eq!(f.s.endpoints.read().await["clock-qa"]["group_power"],power);reply["group_power"]["generation"]=json!(999);
    socket.send(Message::Text(reply.to_string().into())).await.unwrap();barrier(&mut socket).await;assert_eq!(f.s.endpoints.read().await["clock-qa"]["clock_calibration"],estimate);assert_eq!(f.s.endpoints.read().await["clock-qa"]["group_power"],power);
    let stamp=crate::clock::now_ns()+offset;
    let device=json!({"type":"clock","endpoint_id":"clock-qa","host_clock_id":host,"host_clock_basis":"qpc","monotonic_ns":stamp+1,"device_clock":{"version":1,"epoch":9,"source":"wasapi_ticks","timestamp_domain":"qpc","counter":12345,"nominal_frequency_hz":48000,"device_time_ns":stamp,"host_read_before_ns":stamp,"host_read_after_ns":stamp,"running":true}});
    let mut device=device;device["zone_id"]=json!("clock-zone");device["device_clock"]["programme"]=json!({"frames":12345,"sample_rate":48000});device["programme_clock"]=json!({"version":1,"device_epoch":9,"device_counter":12345,"output_frame":12345,"sample_rate":48000,"core_playback_id":uuid::Uuid::new_v4().to_string(),"track_id":"clock-track","position":12345./48000.,"source_frames":12345});
    socket.send(Message::Text(device.to_string().into())).await.unwrap();barrier(&mut socket).await;
    let calibrated=f.s.endpoints.read().await["clock-qa"]["device_clock_calibration"].clone();assert_eq!(calibrated["programme_clock"]["source_frames"],12345);assert_eq!(calibrated["programme_clock"]["zone_id"],"clock-zone");assert_eq!(calibrated["programme_clock"]["core_sample_ns"],calibrated["core_sample_ns"]);assert_eq!(calibrated["counter"],12345);assert_eq!(calibrated["device_clock_mapped"],true);assert_eq!(calibrated["driver_accuracy_verified"],false);println!("DEVICE_CLOCK_QA={calibrated}");
    let mut stale=device.clone();stale["monotonic_ns"]=json!(stamp+600_000_000u64);socket.send(Message::Text(stale.to_string().into())).await.unwrap();barrier(&mut socket).await;assert!(f.s.endpoints.read().await["clock-qa"]["device_clock_calibration"].is_null());
    socket.send(Message::Text(device.to_string().into())).await.unwrap();barrier(&mut socket).await;
    let(mut replacement,_)=tokio_tungstenite::connect_async(request).await.unwrap();
    replacement.send(Message::Text(json!({"type":"register","protocol_version":1,"endpoint":{"id":"clock-qa","clock_calibration":{"forged":true},"device_clock_calibration":{"forged":true},"capabilities":{"host_clock_v1":false}}}).to_string().into())).await.unwrap();next(&mut replacement,"registered").await;
    socket.send(Message::Text(device.to_string().into())).await.unwrap();socket.send(Message::Text(json!({"type":"clock","endpoint_id":"clock-qa","forged":true}).to_string().into())).await.unwrap();socket.send(Message::Text(reply.to_string().into())).await.unwrap();barrier(&mut socket).await;
    let endpoint=f.s.endpoints.read().await["clock-qa"].clone();assert!(endpoint["clock_calibration"].is_null());assert!(endpoint["device_clock_calibration"].is_null());assert!(endpoint["clock"]["forged"].is_null());assert!(endpoint["group_power"].is_null());
    socket.close(None).await.unwrap();replacement.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3),async {loop {if f.s.endpoints.read().await["clock-qa"]["online"]==false{break;}tokio::task::yield_now().await;}}).await.unwrap();
    assert!(f.s.endpoints.read().await["clock-qa"]["clock_calibration"].is_null());server.abort();
}
