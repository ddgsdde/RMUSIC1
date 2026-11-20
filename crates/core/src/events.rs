use crate::{
    auth::{authenticate, Auth},
    error::{ApiError, ApiResult},
    media::TicketQuery,
    state::AppState,
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::HeaderMap,
    response::Response,
    Json,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
#[derive(serde::Deserialize)]
pub struct EventQuery {
    #[serde(flatten)] auth:TicketQuery,
    zone_view:Option<String>,
}
pub async fn ticket(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("read")?;
    Ok(Json(json!({"ticket":s.ticket_for("events".into(),60,Some(a.session_hash)).await})))
}
pub async fn events(
    State(s): State<AppState>,
    Query(q): Query<EventQuery>,
    h: HeaderMap,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    crate::media::authorize(&s, &h, &q.auth, "events").await?;
    let compact=q.zone_view.as_deref()==Some("state");
    let owner=if let Some(ticket)=q.auth.ticket.as_ref(){s.tickets.lock().await.get(&crate::auth::hash(ticket)).and_then(|t|t.owner.clone())}else{let token=h.get("authorization").and_then(|h|h.to_str().ok()).and_then(|h|h.strip_prefix("Bearer ")).ok_or_else(ApiError::auth)?;Some(authenticate(&s,token).await?.session_hash)};
    Ok(ws
        .max_message_size(64 * 1024)
        .on_upgrade(move |socket| event_socket(s, socket, owner,compact)))
}
async fn zone_event(s:&AppState,compact:bool)->Value {
    if compact {json!({"type":"zones_changed","data":{}})} else {json!({"type":"zones","data":s.zone_list().await})}
}
async fn event_socket(s: AppState, socket: WebSocket,owner:Option<String>,compact:bool) {
    let (mut send, mut receive) = socket.split();
    let Some(hash)=owner.clone()else{return;};let Ok(auth)=crate::auth::authenticate_hash(&s,hash).await else{return;};
    let mut events = s.events.subscribe();
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(25));
    let _ = send
        .send(Message::Text(
            crate::accounts::scope::event(&s,&auth,zone_event(&s,compact).await).await.ok().flatten().unwrap_or(json!({"type":"resync","data":{}}))
                .to_string()
                .into(),
        ))
        .await;
    loop {
        tokio::select! {
            event=events.recv()=>{let value=match event{Ok(v) if matches!(v["type"].as_str(),Some("zones"|"zones_changed"|"resync"))=>zone_event(&s,compact).await,Ok(v)=>v,Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>zone_event(&s,compact).await,Err(_)=>break};if !s.session_active(auth.session_hash.clone()).await{break;}let Some(value)=crate::accounts::scope::event(&s,&auth,value).await.ok().flatten()else{continue;};if send.send(Message::Text(value.to_string().into())).await.is_err(){break;}},
            value=receive.next()=>match value{Some(Ok(Message::Ping(p)))=>{if send.send(Message::Pong(p)).await.is_err(){break;}},Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,_=>{}},
            _=heartbeat.tick()=>{if let Some(owner)=owner.as_ref(){if !s.session_active(owner.clone()).await{break;}}if send.send(Message::Ping(vec![].into())).await.is_err(){break;}},
        }
    }
}
pub async fn bridge(
    State(s): State<AppState>,
    h: HeaderMap,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    let token = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(ApiError::auth)?;
    let auth=authenticate(&s, token).await?;auth.require("control")?;
    Ok(ws
        .max_message_size(128 * 1024)
        .on_upgrade(move |socket| bridge_socket(s, socket,auth.session_hash)))
}
async fn bridge_socket(s: AppState, socket: WebSocket,owner:String) {
    let (mut send, mut receive) = socket.split();
    let (tx, mut commands) = tokio::sync::mpsc::channel::<Value>(64);
    let mut ids = Vec::new();
    let mut clocks=std::collections::HashMap::<String,crate::clock::Filter>::new();
    let mut device_clocks=std::collections::HashMap::<String,crate::device_clock::DeviceFilter>::new();
    let mut clock_tick=tokio::time::interval(std::time::Duration::from_secs(2));
    clock_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);clock_tick.tick().await;
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(20));
    let mut last_seen = std::time::Instant::now();
    loop {
        tokio::select! {
            command=commands.recv()=>{let Some(command)=command else{break;};let removed=command["type"]=="remove_endpoint";if send.send(Message::Text(command.to_string().into())).await.is_err(){break;}if removed{break;}},
            message=receive.next()=>{
                let received_at=crate::clock::now_ns();
                last_seen=std::time::Instant::now();let v=match message{Some(Ok(Message::Text(text)))=>serde_json::from_str::<Value>(&text).ok(),Some(Ok(Message::Ping(p)))=>{let _=send.send(Message::Pong(p)).await;None},Some(Ok(Message::Pong(_)))=>None,Some(Ok(Message::Close(_)))|Some(Err(_))|None=>break,_=>None};
                let Some(v)=v else{continue;};let Ok(initial_auth)=crate::auth::authenticate_hash(&s,owner.clone()).await else{break;};let _lease=crate::accounts::leases::read(&s,&initial_auth.id).await;let Ok(auth)=crate::auth::authenticate_hash(&s,owner.clone()).await else{break;};match v["type"].as_str().unwrap_or(""){
                    "register"=>{if ids.len()>=64{break;}let mut endpoint=v["endpoint"].clone();if let Some(object)=endpoint.as_object_mut(){object.remove("clock_calibration");object.remove("device_clock_calibration");object.remove("group_power");object.remove("group_power_clock_id");}let id=endpoint["id"].as_str().unwrap_or("").to_string();if id.is_empty()||id.len()>200||id=="browser-local"{continue;}
                        if crate::lan_bridges::endpoint(&auth).is_some_and(|own|own!=id){break;}
                        {let _lock=s.zone_lock.lock().await;if crate::accounts::scope::claim(&s,&auth,"endpoint",&id).await.is_err(){break;}if crate::devices::is_removed(&s,&id).await.unwrap_or(true){break;}let _=crate::sync_start::peer_changed(&s,&id).await;endpoint["online"]=json!(true);endpoint["verified"]=json!(false);s.endpoints.write().await.insert(id.clone(),endpoint);let mut bridges=s.bridges.write().await;bridges.insert(id.clone(),tx.clone());s.clock_filters.write().await.remove(&id);drop(bridges);s.bridge_owners.write().await.insert(id.clone(),owner.clone());if !ids.contains(&id){ids.push(id.clone());}}
                        let _=send.send(Message::Text(json!({"type":"registered","endpoint_id":id,"protocol_version":1}).to_string().into())).await;
                        device_clocks.remove(&id);clocks.remove(&id);if v["protocol_version"]==1&&v["endpoint"]["capabilities"]["host_clock_v1"]==true {let mut filter=crate::clock::Filter::default();let probe=filter.probe(&id,crate::clock::now_ns());clocks.insert(id.clone(),filter);if send.send(Message::Text(probe.to_string().into())).await.is_err(){break;}}
                        s.emit("endpoints",json!({}));
                    },
                    "visualization"=>{crate::visualization::accept(&s,&tx,v.clone()).await;},
                    "device_volume"=>{let Some(id)=v["endpoint_id"].as_str()else{continue;};if !ids.iter().any(|i|i==id)||!s.bridges.read().await.get(id).is_some_and(|registered|registered.same_channel(&tx)){continue;}
                        if v["state"]["value"].as_f64().is_some_and(|x|x.is_finite()&&(0. ..=1.).contains(&x)) {if let Some(endpoint)=s.endpoints.write().await.get_mut(id){endpoint["device_volume"]=v["state"].clone();}}
                        s.emit("device_volume",v.clone());
                    },
                    "clock_reply"=>{let Some(id)=v["endpoint_id"].as_str() else{continue;};let registered=s.bridges.read().await;if !registered.get(id).is_some_and(|registered|registered.same_channel(&tx)){continue;}
                        if let Some(filter)=clocks.get_mut(id){let now=crate::clock::now_ns();if filter.accept(&v,received_at){s.clock_filters.write().await.insert(id.into(),(tx.clone(),filter.clone()));if let Some(endpoint)=s.endpoints.write().await.get_mut(id){endpoint["clock_calibration"]=filter.estimate(now);endpoint["group_power"]=crate::sync_start::power_stamp(&v["group_power"]).unwrap_or(Value::Null);endpoint["group_power_clock_id"]=v["host_clock_id"].clone();if let Some(device)=device_clocks.get(id){endpoint["device_clock_calibration"]=device.latest(now,&filter.estimate(now));}}}}
                    },
                    "clock"=>{if let Some(id)=v["endpoint_id"].as_str(){if ids.iter().any(|i|i==id)&&s.bridges.read().await.get(id).is_some_and(|registered|registered.same_channel(&tx)){
                        {let mut endpoints=s.endpoints.write().await;if let Some(e)=endpoints.get_mut(id){e["clock"]=v.clone();if e["capabilities"]["device_clock_v1"]==true {let host=clocks.get(id).map(|f|f.estimate(received_at)).unwrap_or(Value::Null);e["device_clock_calibration"]=device_clocks.entry(id.into()).or_default().observe(&v,&host,received_at);}}}
                        crate::sync_start::observe_device_clock(&s,id,&tx).await;
                    }}},
                    "start_state"|"start_error"=>{if auth.role=="external"{continue;}let Some(id)=v["endpoint_id"].as_str()else{continue;};if !s.bridges.read().await.get(id).is_some_and(|registered|registered.same_channel(&tx)){continue;}let _lock=s.zone_lock.lock().await;if let Err(error)=crate::sync_start::report(&s,id,&tx,&v).await{tracing::warn!(%error,"group report failed");}},
                    "state"|"error"=>{let Some(zone_id)=v["zone_id"].as_str()else{continue;};let _lock=s.zone_lock.lock().await;let Some(mut z)=s.zones.read().await.get(zone_id).cloned()else{continue;};if !ids.contains(&z.endpoint_id)||!s.bridges.read().await.get(&z.endpoint_id).is_some_and(|registered|registered.same_channel(&tx)){continue;}
                        let timing=match v.get("measured_version"){None=>1,Some(value)=>match value.as_u64(){Some(1)=>1,Some(2)=>2,_=>continue}};
                        if timing==2&&(v["type"]!="state"||v["frames"].as_u64().is_none()||v["sample_rate"].as_u64().is_none_or(|r|!(8000..=768000).contains(&r))||v["position"].as_f64().is_none_or(|p|!p.is_finite()||p<0.)){continue;}
                        match crate::sync_start::report(&s,&z.endpoint_id,&tx,&v).await{Ok(true)=>continue,Err(error)=>{tracing::warn!(%error,"group device report failed");continue;},Ok(false)=>{}}
                        if !crate::zones::report_matches(&s,&z,v["track_id"].as_str(),v["core_playback_id"].as_str()).await&&!(v["device_failed"]==true&&v["recoverable"]==true&&v["system_suspend"].is_object()&&crate::zones::paused_system_report_matches(&z,v["track_id"].as_str(),v["core_playback_id"].as_str())){continue;}
                        if v["type"]=="state"&&crate::stats::output::version(&s,&z).await.ok()!=Some(timing){continue;}
                        // Pause and resume keep the playback ID, so an in-flight state can belong to the previous command.
                        let stale_transport = match v["status"].as_str() {
                            Some("playing") => z.status != "playing",
                            Some("paused") => z.status == "playing",
                            _ => false,
                        };
                        if v["type"]=="state"&&stale_transport&&s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["playback_instance"]==true){continue;}
                        if v["signal_path"].is_object(){let mut playback=v.clone();playback["reported_at"]=json!(chrono::Utc::now().timestamp_millis());if let Some(endpoint)=s.endpoints.write().await.get_mut(&z.endpoint_id){endpoint["playback"]=playback.clone();}s.emit("endpoint_signal",json!({"endpoint_id":z.endpoint_id,"playback":playback}));}
                        if v["status"]=="ended"{if z.status!="playing"{continue;}if let Some(position)=v["position"].as_f64().filter(|p|p.is_finite()&&*p>=0.){z.position=position;}else if let Some(t)=&z.track{if t.duration>0.{z.position=t.duration;}}let measured=s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["continuous_pcm"]==true);
                            if (measured||timing==2)&&v.get("frames").is_some(){
                                let Some(frames)=v["frames"].as_u64() else{continue;};let Some(rate)=v["sample_rate"].as_u64().and_then(|r|u32::try_from(r).ok()) else{continue;};
                                let accepted=if timing==2{crate::stats::output::progress(&s,&z,frames,rate,true).await}else{crate::stats::finish_device(&s,&z,frames,rate).await};if !accepted.unwrap_or(false){continue;}
                            }else{let _=crate::stats::finish(&s,&z,"ended").await;}let action=crate::zones::advance(&s,&mut z,false).await;let _=crate::zones::dispatch(&s,&z,&action).await;if z.status=="playing"{let _=crate::stats::record_progress(&s,&z).await;}}
                        else if v["type"]=="error"{
                            let stamp=match serde_json::from_value::<Option<crate::sequence::recovery::SystemSuspend>>(v.get("system_suspend").cloned().unwrap_or(Value::Null)){Ok(stamp) if stamp.as_ref().is_none_or(|s|s.validate().is_ok())=>stamp,_=>continue};
                            if stamp.is_some()&&(v["device_failed"]!=true||v["recoverable"]!=true){continue;}
                            if let Some(stamp)=&stamp {if !crate::sequence::recovery::new_system_suspend(&s,&z.id,stamp).await.unwrap_or(false){continue;}}
                            let _=crate::stats::finish(&s,&z,"disconnect").await;
                            s.emit("playback_error",v.clone());
                            if v["source_unavailable"]==true && z.status=="playing" && z.queue_index+1<z.queue.len() {
                                let mut next=z.clone();
                                let repeat=std::mem::replace(&mut next.repeat,"off".into());
                                let action=crate::zones::advance(&s,&mut next,false).await;
                                next.repeat=repeat;
                                if crate::zones::dispatch(&s,&next,&action).await.is_ok() {
                                    let _=crate::stats::record_progress(&s,&next).await;
                                    let _=s.save_zone(next.clone()).await;
                                    crate::autoplay::prepare(&s,&next).await;
                                    continue;
                                }
                            }
                            if v["recoverable"]==true&&s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["continuous_recovery"]==true){
                                let automatic=stamp.is_none()&&v["retryable"]==true&&v["automatic_recovery"]==true;
                                let result=if automatic {crate::sequence::recovery::suspend(&s,&z).await}
                                    else if v["device_failed"]==true {crate::sequence::recovery::suspend_device_with_stamp(&s,&z,stamp).await}
                                    else if v["source_failed"]==true {crate::sequence::recovery::suspend_source(&s,&z).await}
                                    else {crate::sequence::recovery::suspend(&s,&z).await};
                                if let Err(error)=result {tracing::warn!(%error,"recoverable endpoint suspension failed");}
                                continue;
                            }
                            z.status="paused".into();z.playback_id=None;
                        }
                        else{if let Some(status)=v["status"].as_str(){if ["playing","paused","stopped"].contains(&status){z.status=status.into();}}if let Some(position)=v["position"].as_f64(){if position.is_finite()&&position>=0.{z.position=position;}}}
                        z.updated_at=chrono::Utc::now().timestamp_millis();if v["type"]=="state"&&v["status"]!="ended"{
                            let measured=s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["continuous_pcm"]==true);
                            if (measured||timing==2)&&v.get("frames").is_some(){
                                let Some(frames)=v["frames"].as_u64() else{continue;};let Some(rate)=v["sample_rate"].as_u64().and_then(|r|u32::try_from(r).ok()) else{continue;};
                                if timing==2{if let Err(error)=s.publish_output_progress(z,frames,rate).await{tracing::warn!(%error,"output progress failed");}}else{s.publish_device_progress(z,frames,rate).await;}
                            }else{s.publish_progress(z).await;}
                        }else{let _=s.save_zone(z.clone()).await;crate::autoplay::prepare(&s,&z).await;}
                    },_=>{}
                }
            },
            _=clock_tick.tick()=>{crate::visualization::expire(&s).await;for (id,filter) in clocks.iter_mut(){if !s.bridges.read().await.get(id).is_some_and(|registered|registered.same_channel(&tx)){continue;}let now=crate::clock::now_ns();if let Some(endpoint)=s.endpoints.write().await.get_mut(id){endpoint["clock_calibration"]=filter.estimate(now);if let Some(device)=device_clocks.get(id){endpoint["device_clock_calibration"]=device.latest(now,&filter.estimate(now));}}let probe=filter.probe(id,crate::clock::now_ns());if send.send(Message::Text(probe.to_string().into())).await.is_err(){break;}}},
            _=heartbeat.tick()=>{if !s.session_active(owner.clone()).await||last_seen.elapsed()>std::time::Duration::from_secs(70){break;}let owns_output={let bridges=s.bridges.read().await;ids.iter().any(|id|bridges.get(id).is_some_and(|registered|registered.same_channel(&tx)))};if !ids.is_empty()&&!owns_output{break;}if send.send(Message::Ping(vec![].into())).await.is_err(){break;}},
        }
    }
    let _lock=s.zone_lock.lock().await;
    let mut offline=Vec::new();
    for id in ids {
        let mut bridges = s.bridges.write().await;
        if bridges.get(&id).is_some_and(|old| old.same_channel(&tx)) {
            bridges.remove(&id);
            crate::visualization::disconnected(&s,&id).await;
            s.clock_filters.write().await.remove(&id);
            offline.push(id.clone());
            let _=crate::sync_start::peer_changed(&s,&id).await;
            s.bridge_owners.write().await.remove(&id);
            if let Some(e) = s.endpoints.write().await.get_mut(&id) {
                e["online"] = json!(false);
                if e.get("clock_calibration").is_some(){e["clock_calibration"]=Value::Null;}if e.get("device_clock_calibration").is_some(){e["device_clock_calibration"]=Value::Null;}
                e["group_power"]=Value::Null;e["group_power_clock_id"]=Value::Null;
            }
        }
    }
    if !offline.is_empty(){let zones:Vec<_>=s.zones.read().await.values().filter(|z|offline.contains(&z.endpoint_id)).cloned().collect();for z in zones{crate::autoplay::cancel(&s,&z.id).await;if z.status=="playing"{let _=crate::stats::finish(&s,&z,"disconnect").await;}if let Err(error)=crate::sequence::recovery::suspend(&s,&z).await{tracing::warn!(%error,zone_id=z.id,"transport suspension failed");}}}
    s.emit("endpoints", json!({}));
}
