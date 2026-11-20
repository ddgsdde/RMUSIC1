use super::*;
use futures_util::SinkExt;
use serde_json::{json,Value};
use tokio_tungstenite::{connect_async_tls_with_config,Connector,tungstenite::{client::IntoClientRequest,Message}};

pub struct ConnectionEvents(tokio::task::JoinHandle<()>);
impl Drop for ConnectionEvents {fn drop(&mut self){self.0.abort();}}
impl ConnectionProfile {
    /// A visible controller receives state changes without polling its remote zones.
    pub fn follow_events(&self,token:String,receive:Arc<dyn Fn(Value)+Send+Sync>)->ConnectionEvents {
        self.events_with_route(token,receive,None)
    }

    pub fn follow_selected_events(&self, token:String, receive:Arc<dyn Fn(Value)+Send+Sync>, routes:tokio::sync::watch::Receiver<Self>)->ConnectionEvents {
        self.events_with_route(token,receive,Some(routes))
    }

    fn events_with_route(&self, token:String, receive:Arc<dyn Fn(Value)+Send+Sync>, mut routes:Option<tokio::sync::watch::Receiver<Self>>)->ConnectionEvents {
        let profile=self.clone();
        ConnectionEvents(tokio::spawn(async move {
            let mut delay=1;
            let mut offline_reported=false;
            loop {
                let current=routes.as_mut().map(|rx|rx.borrow_and_update().clone());
                let on_change=async {
                    match routes.as_mut() {
                        Some(rx)=>rx.changed().await,
                        None=>std::future::pending().await,
                    }
                };
                let attempt=async {
                    let _result:Result<()>=async {
                        let selected=match current {Some(value)=>value,None=>profile.resolve().await?};
                        let mut url=selected.endpoint("api/v1/events")?;
                        url.query_pairs_mut().append_pair("zone_view","state");
                        url.set_scheme(if url.scheme()=="https"{"wss"}else{"ws"}).map_err(|_|anyhow::anyhow!("事件地址无效"))?;
                        let mut request=url.as_str().into_client_request()?;
                        request.headers_mut().insert("Authorization",format!("Bearer {token}").parse()?);
                        let config=tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(32*1024*1024));
                        let (mut socket,_)=tokio::time::timeout(Duration::from_secs(15),connect_async_tls_with_config(request,Some(config),false,Some(Connector::Rustls(selected.tls_config()?)))).await??;
                        delay=1;
                        offline_reported=false;
                        receive(json!({"type":"connection_ready"}));
                        while let Some(message)=tokio::time::timeout(Duration::from_secs(60),socket.next()).await? {
                            match message? {
                                Message::Text(text)=>receive(serde_json::from_str(&text)?),
                                Message::Ping(bytes)=>socket.send(Message::Pong(bytes)).await?,
                                Message::Close(_)=>break,
                                _=>{}
                            }
                        }
                        Ok(())
                    }.await;
                    if !offline_reported {
                        receive(json!({"type":"connection_interrupted"}));
                        offline_reported=true;
                    }
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    delay=(delay*2).min(30);
                };
                tokio::select! {
                    _=attempt=>{},
                    changed=on_change=>{if changed.is_err(){break;}delay=1;},
                }
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn selected_events_move_off_a_still_reachable_backup() {
        use axum::{extract::WebSocketUpgrade,extract::ws::Message as ServerMessage,routing::get};
        async fn source(label:&'static str)->(ConnectionProfile,tokio::task::JoinHandle<()>) {
            let app=axum::Router::new().route("/api/v1/events",get(move |ws:WebSocketUpgrade|async move {
                ws.on_upgrade(move |mut socket|async move {
                    socket.send(ServerMessage::Text(json!({"type":label}).to_string().into())).await.unwrap();
                    let _=socket.recv().await;
                })
            }));
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr=listener.local_addr().unwrap();
            let profile=serde_json::from_value(json!({"id":"events","name":"Core","address":format!("http://{addr}")})).unwrap();
            (profile,tokio::spawn(async move {axum::serve(listener,app).await.unwrap();}))
        }
        let (backup,first)=source("backup").await;
        let (home,second)=source("home").await;
        let (send,mut received)=tokio::sync::mpsc::unbounded_channel();
        let (routes,updates)=tokio::sync::watch::channel(backup.clone());
        let subscription=backup.follow_selected_events("test".into(),Arc::new(move |v|{let _=send.send(v);}),updates);
        tokio::time::timeout(Duration::from_secs(2),async {
            assert_eq!(received.recv().await.unwrap()["type"],"connection_ready");
            assert_eq!(received.recv().await.unwrap()["type"],"backup");
            routes.send_replace(home);
            assert_eq!(received.recv().await.unwrap()["type"],"connection_ready");
            assert_eq!(received.recv().await.unwrap()["type"],"home");
        }).await.unwrap();
        drop(subscription);
        first.abort();second.abort();
    }
    #[tokio::test]
    async fn normal_close_reports_interruption_until_the_event_stream_returns() {
        use axum::{extract::WebSocketUpgrade, extract::ws::Message as ServerMessage, routing::get};
        let reconnect = Arc::new(tokio::sync::Notify::new());
        let next = reconnect.clone();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let app= axum::Router::new()
            .route("/api/v1/status", get(|| async { axum::Json(json!({"core_id":"reconnect-test"})) }))
            .route("/api/v1/events", get(move |ws: WebSocketUpgrade| {
                let first = attempts.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0;
                let next = next.clone();
                async move {
                    if !first { next.notified().await; }
                    ws.on_upgrade(move |mut socket| async move {
                        if first { socket.send(ServerMessage::Close(None)).await.unwrap(); }
                        else { let _ = socket.recv().await; }
                    })
                }
            }));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address=listener.local_addr().unwrap();
        let server=tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let profile:ConnectionProfile=serde_json::from_value(json!({"id":"event-retry","name":"QA","address":format!("http://{address}"),"core_id":"reconnect-test"})).unwrap();
        let (send,mut events)=tokio::sync::mpsc::unbounded_channel();
        let watch=profile.follow_events("test-token".into(),Arc::new(move|value|{let _=send.send(value);}));
        assert_eq!(events.recv().await.unwrap()["type"],"connection_ready");
        assert_eq!(tokio::time::timeout(Duration::from_millis(500),events.recv()).await.unwrap().unwrap()["type"],"connection_interrupted");
        reconnect.notify_one();
        assert_eq!(tokio::time::timeout(Duration::from_secs(3),events.recv()).await.unwrap().unwrap()["type"],"connection_ready");
        drop(watch);
        server.abort();
    }
    #[tokio::test]
    async fn visible_event_subscription_receives_changes_and_drop_closes_the_socket() {
        use axum::{routing::get,extract::ws::{WebSocketUpgrade,Message as ServerMessage}};
        let closed=Arc::new(tokio::sync::Notify::new());let on_close=closed.clone();
        let app=axum::Router::new()
            .route("/api/v1/status",get(||async{axum::Json(json!({"core_id":"events-test","initialized":true}))}))
            .route("/api/v1/events",get(move|headers:axum::http::HeaderMap,ws:WebSocketUpgrade|{
                assert_eq!(headers["authorization"],"Bearer test-token");
                let closed=on_close.clone();async move{ws.on_upgrade(move|mut socket|async move{
                    socket.send(ServerMessage::Text(json!({"type":"zones","data":{"items":[]}}).to_string().into())).await.unwrap();
                    while let Some(Ok(message))=socket.recv().await {if matches!(message,ServerMessage::Close(_)){break;}}
                    closed.notify_one();
                })}
            }));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address=listener.local_addr().unwrap();let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap()});
        let profile:ConnectionProfile=serde_json::from_value(json!({"id":"events-test","name":"QA","address":format!("http://{address}"),"core_id":"events-test"})).unwrap();
        let (send,mut events)=tokio::sync::mpsc::unbounded_channel();
        let watch=profile.follow_events("test-token".into(),Arc::new(move|value|{let _=send.send(value);}));
        let event=tokio::time::timeout(Duration::from_secs(5),events.recv()).await.unwrap().unwrap();
        assert_eq!(event["type"],"connection_ready");
        let event=tokio::time::timeout(Duration::from_secs(5),events.recv()).await.unwrap().unwrap();
        assert_eq!(event["type"],"zones");
        drop(watch);tokio::time::timeout(Duration::from_secs(2),closed.notified()).await.unwrap();
        server.abort();
    }
}
