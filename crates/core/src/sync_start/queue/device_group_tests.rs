//! End-to-end API/channel group checks; synthetic clocks and frames, no audio hardware.
use super::*;
async fn create_device(f: &Fixture, p: &mut [Peer], track: &str, repeat: &str) -> Value {
    enable_v2(f, p).await;
    for peer in p.iter_mut() {
        let mut endpoint = f.s.endpoints.write().await;
        for cap in [
            "group_output_final_v2",
            "device_clock_v1",
            "programme_clock_v1",
            "visualization_v1",
            "group_volume_v1",
        ] {
            endpoint.get_mut(&peer.endpoint).unwrap()["capabilities"][cap] = json!(true);
        }
        drop(endpoint);
        let mut z = f.s.zones.read().await[&peer.zone].clone();
        let t = library::get_track(&f.s, track.into()).await.unwrap();
        z.queue = vec![t.clone()];
        z.track = Some(t);
        z.status = "paused".into();
        z.repeat = repeat.into();
        z.autoplay.enabled = false;
        f.s.save_zone(z).await.unwrap();
    }
    let(status,group)=f.call("POST","/api/v1/device-groups",Some(json!({"request_id":uuid::Uuid::new_v4().to_string(),"name":"客厅与书房","zone_ids":p.iter().map(|p|&p.zone).collect::<Vec<_>>(),"source_zone_id":p[0].zone}))).await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    for peer in p {
        assert_eq!(receive(peer).await["type"], "stop");
    }
    group
}
async fn ready_device(f: &Fixture, p: &mut [Peer]) -> (Vec<Value>, Vec<Value>) {
    let mut commands = vec![];
    for peer in p.iter_mut() {
        let stop = receive(peer).await;
        assert_eq!(stop["type"], "stop");
        let command = receive(peer).await;
        assert_eq!(command["type"], "prepare_start");
        prepare_v2(f, peer, &command).await;
        commands.push(command);
    }
    for (i, command) in commands.iter().enumerate() {
        let mut v = ready(command);
        v["measured_version"] = json!(2);
        v["sample_rate"] = json!(768000);
        report(f, &p[i], v).await;
    }
    let mut commits = vec![];
    for peer in p.iter_mut() {
        let commit = receive(peer).await;
        assert_eq!(commit["type"], "commit_start");
        commits.push(commit);
    }
    let id = commands[0]["start"]["plan_id"].as_str().unwrap();
    let state = f
        .call("GET", &format!("/api/v1/sync/starts/{id}"), None)
        .await
        .1;
    let deadline = state["deadline_core_ns"].as_u64().unwrap();
    tokio::time::sleep(std::time::Duration::from_nanos(
        deadline.saturating_sub(clock::now_ns()) + 1_000_000,
    ))
    .await;
    for i in 0..p.len() {
        let mut v = final_report_value(&commands[i], &commits[i], 0, json!([]));
        v["phase"] = json!("released");
        v["final"] = json!(false);
        v["measured_version"] = json!(2);
        v["sample_rate"] = json!(768000);
        report(f, &p[i], v).await;
    }
    (commands, commits)
}
#[tokio::test]
async fn device_group_transport_projects_real_progress_and_restores_queues_after_eof() {
    let f = Fixture::new().await;
    let (track, _) = f.add_audio().await;
    let mut p = peers(&f, 32).await;
    let group = create_device(&f, &mut p, &track, "off").await;
    let path = format!("/api/v1/zones/{}", group["zone_id"].as_str().unwrap());
    let (status, v) = f
        .call("POST", &format!("{path}/play"), Some(json!({})))
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (commands, _) = ready_device(&f, &mut p).await;
    for i in 0..2 {
        report(&f,&p[i],json!({"type":"state","zone_id":p[i].zone,"core_playback_id":commands[i]["playback_id"],"track_id":track,"status":"playing","measured_version":2,"position":0.001,"frames":768,"sample_rate":768000})).await;
    }
    let z = f.call("GET", &path, None).await.1;
    assert_eq!(z["status"], "playing");
    assert_eq!(z["position"], 0.001);
    assert!(z["playback_id"]
        .as_str()
        .is_some_and(|v| v.starts_with(group["id"].as_str().unwrap())));
    let original_volume=f.s.zones.read().await[&p[0].zone].volume;
    f.s.endpoints.write().await.get_mut(&p[1].endpoint).unwrap()["capabilities"]["group_volume_v1"]=json!(false);
    assert_eq!(f.call("PATCH",&path,Some(json!({"volume":0.3}))).await.0,StatusCode::CONFLICT);
    assert_eq!(f.s.zones.read().await[&p[0].zone].volume,original_volume);
    assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()),"failed preflight must send no partial member volumes");
    f.s.endpoints.write().await.get_mut(&p[1].endpoint).unwrap()["capabilities"]["group_volume_v1"]=json!(true);
    assert_eq!(f.call("PATCH",&path,Some(json!({"volume":0.3}))).await.0,StatusCode::OK);
    for peer in &mut p{let v=receive(peer).await;assert_eq!(v["type"],"group_volume");assert_eq!(v["volume"],0.3);}
    let original_volume=f.s.zones.read().await[&p[0].zone].volume;
    f.s.endpoints.write().await.get_mut(&p[1].endpoint).unwrap()["capabilities"]["group_volume_v1"]=json!(false);
    assert_eq!(f.call("PATCH",&path,Some(json!({"volume":0.3}))).await.0,StatusCode::CONFLICT);
    assert_eq!(f.s.zones.read().await[&p[0].zone].volume,original_volume);
    assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()),"failed preflight must send no partial member volumes");
    f.s.endpoints.write().await.get_mut(&p[1].endpoint).unwrap()["capabilities"]["group_volume_v1"]=json!(true);
    assert_eq!(f.call("PATCH",&path,Some(json!({"volume":0.3}))).await.0,StatusCode::OK);
    for peer in &mut p{let v=receive(peer).await;assert_eq!(v["type"],"group_volume");assert_eq!(v["volume"],0.3);}
    let before=crate::device_groups::load(&f.s,group["id"].as_str().unwrap()).await.unwrap().unwrap().job_id;
    assert_eq!(f.call("POST",&format!("{path}/queue"),Some(json!({"track_ids":[track]}))).await.0,StatusCode::OK);
    let after=crate::device_groups::load(&f.s,group["id"].as_str().unwrap()).await.unwrap().unwrap().job_id;
    assert_eq!(before,after,"tail append must keep the current output epoch");assert!(p.iter_mut().all(|p|p.rx.try_recv().is_err()));
    let body=json!({"after":0.});let visual=f.call("POST",&format!("{path}/visualization"),Some(body.clone())).await.1;assert_eq!(visual["supported"],true);assert_eq!(receive(&mut p[0]).await["type"],"visualization_lease");
    crate::visualization::accept(&f.s,&p[0].tx,json!({"version":1,"zone_id":p[0].zone,"endpoint_id":p[0].endpoint,"track_id":track,"core_playback_id":commands[0]["playback_id"],"position":0.001,"frames":[{"playback_id":commands[0]["playback_id"],"position":0.001,"bands":[1,2,3,4,5,6,7,8],"energy":20,"peak":50}]})).await;
    let visual=f.call("POST",&format!("{path}/visualization"),Some(body)).await.1;assert_eq!(visual["playback_id"],z["playback_id"]);assert_eq!(visual["frames"][0]["playback_id"],z["playback_id"]);assert_eq!(visual["frames"][0]["energy"],20);
    assert_eq!(f.call("POST",&format!("{path}/pause"),Some(json!({"control":{"endpoint_id":group["endpoint_id"],"track_id":track,"playback_id":"stale"}}))).await.0,StatusCode::CONFLICT);
    assert_eq!(f.call("PUT",&format!("{path}/queue"),Some(json!({"track_ids":[track]}))).await.0,StatusCode::OK);
    for i in 0..2 {
        let(status,v)=f.call("POST",&format!("/api/v1/zones/{}/ended",p[i].zone),Some(json!({"track_id":track,"playback_id":commands[i]["playback_id"],"measured_version":2,"position":0.002,"frames":1536,"sample_rate":768000}))).await;
        assert_eq!(status, StatusCode::OK, "{v}");
    }
    assert_eq!(f.call("GET", &path, None).await.1["status"], "stopped");
    assert_eq!(
        f.call(
            "DELETE",
            &format!("/api/v1/device-groups/{}", group["id"].as_str().unwrap()),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    for peer in p {
        let z = f.s.zones.read().await[&peer.zone].clone();
        assert_eq!(z.status, "paused");
        assert_eq!(z.queue.len(), 1);
        assert_eq!(z.position, 0.);
    }
    assert_eq!(
        sessions(&f).await,
        1,
        "one shared listen, never one per member"
    );
}
#[tokio::test]
async fn device_group_pause_retains_confirmed_position_then_resume_uses_one_common_deadline() {
    let f = Fixture::new().await;
    let (track, _) = f.add_audio().await;
    let mut p = peers(&f, 32).await;
    let group = create_device(&f, &mut p, &track, "off").await;
    let path = format!("/api/v1/zones/{}", group["zone_id"].as_str().unwrap());
    assert_eq!(
        f.call("POST", &format!("{path}/play"), Some(json!({})))
            .await
            .0,
        StatusCode::OK
    );
    let (commands, commits) = ready_device(&f, &mut p).await;
    assert_eq!(
        f.call("POST", &format!("{path}/pause"), Some(json!({})))
            .await
            .0,
        StatusCode::OK
    );
    for peer in &mut p {
        assert_eq!(receive(peer).await["type"], "cancel_start");
    }
    for i in 0..2 {
        let source = json!({"playback_id":commands[i]["playback_id"],"track_id":track,"frames":768,"position":0.001,"ended":false});
        let mut v = final_report_value(&commands[i], &commits[i], 768, json!([source]));
        v["measured_version"] = json!(2);
        v["sample_rate"] = json!(768000);
        report(&f, &p[i], v).await;
    }
    let z = f.call("GET", &path, None).await.1;
    assert_eq!(z["status"], "paused");
    assert_eq!(z["position"], 0.001);
    assert_eq!(
        f.call("POST", &format!("{path}/play"), Some(json!({})))
            .await
            .0,
        StatusCode::OK
    );
    let mut newer = vec![];
    for peer in &mut p {
        let c = receive(peer).await;
        assert_eq!(c["type"], "prepare_start");
        assert_eq!(c["position"], 0.001);
        prepare_v2(&f, peer, &c).await;
        newer.push(c);
    }
    for i in 0..2 {
        let mut v = ready(&newer[i]);
        v["measured_version"] = json!(2);
        v["sample_rate"] = json!(768000);
        report(&f, &p[i], v).await;
    }
    let a = receive(&mut p[0]).await;
    let b = receive(&mut p[1]).await;
    assert_eq!(a["type"], "commit_start");
    assert_eq!(b["type"], "commit_start");
    assert_eq!(
        a["deadline_host_ns"].as_i64().unwrap() - p[0].offset,
        b["deadline_host_ns"].as_i64().unwrap() - p[1].offset
    );
}

#[tokio::test]
async fn device_group_repeat_one_restarts_the_group_without_consuming_the_saved_member_queue() {
    let f = Fixture::new().await;
    let (track, _) = f.add_audio().await;
    let mut p = peers(&f, 32).await;
    let group = create_device(&f, &mut p, &track, "one").await;
    let path = format!("/api/v1/zones/{}", group["zone_id"].as_str().unwrap());
    assert_eq!(
        f.call("POST", &format!("{path}/play"), Some(json!({})))
            .await
            .0,
        StatusCode::OK
    );
    let (commands, _) = ready_device(&f, &mut p).await;
    for i in 0..2 {
        let(status,v)=f.call("POST",&format!("/api/v1/zones/{}/ended",p[i].zone),Some(json!({"track_id":track,"playback_id":commands[i]["playback_id"],"measured_version":2,"position":0.001,"frames":768,"sample_rate":768000}))).await;
        assert_eq!(status, StatusCode::OK, "{v}");
    }
    for peer in &mut p {
        assert_eq!(receive(peer).await["type"], "stop");
        let mut next = receive(peer).await;
        if next["type"]=="cancel_preload" {next=receive(peer).await;}
        assert_eq!(next["type"], "prepare_start");
        assert_eq!(next["position"], 0.);
        assert_eq!(next["track"]["id"], track);
        assert_ne!(next["playback_id"], commands[0]["playback_id"]);
    }
    let z = f.call("GET", &path, None).await.1;
    assert_eq!(z["repeat"], "one");
    assert_eq!(z["queue"].as_array().unwrap().len(), 1);
    let original = crate::device_groups::load(&f.s, group["id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.zone_ids.len(), 2);
    assert_eq!(original.job_offset, 0);
}

#[tokio::test]
async fn device_group_active_transfer_prepares_before_stopping_and_reuses_ready_output() {
    let f=Fixture::new().await;
    let (track,_)=f.add_audio().await;
    let mut p=peers(&f,32).await;
    let group=create_device(&f,&mut p,&track,"off").await;
    let group_id=group["zone_id"].as_str().unwrap();
    let mut source=f.s.zones.read().await[&p[0].zone].clone();
    source.id="travel-zone".into();source.endpoint_id="travel-endpoint".into();
    source.queue.push(source.queue[0].clone());source.queue_index=1;source.status="playing".into();
    source.playback_id=Some("travel-playback".into());source.volume=0.25;
    source.updated_at=chrono::Utc::now().timestamp_millis();
    f.s.save_zone(source.clone()).await.unwrap();
    let(tx,mut rx)=tokio::sync::mpsc::channel(32);
    f.s.bridges.write().await.insert(source.endpoint_id.clone(),tx);
    f.s.endpoints.write().await.insert(source.endpoint_id.clone(),json!({"id":source.endpoint_id,"kind":"alsa","online":true,"capabilities":{"playback_instance":true}}));
    let path=format!("/api/v1/zones/{}/transfer",source.id);
    let body=json!({"target_zone_id":group_id,"expected_playback_id":source.playback_id});
    let (moved,(commands,commits))=tokio::join!(f.call("POST",&path,Some(body)),ready_device(&f,&mut p));
    assert_eq!(moved.0,StatusCode::OK,"{}",moved.1);
    assert_eq!(moved.1["target"]["status"],"playing");assert_eq!(moved.1["target"]["queue_index"],1);
    assert_eq!(moved.1["target"]["queue"],serde_json::to_value(&source.queue).unwrap());
    assert_eq!(rx.recv().await.unwrap()["type"],"stop");
    let path=format!("/api/v1/zones/{group_id}/transfer");
    let body=json!({"target_zone_id":source.id,"expected_playback_id":moved.1["target"]["playback_id"]});
    let (returned,prepared)=tokio::join!(f.call("POST",&path,Some(body)),async {
        let prepared=rx.recv().await.unwrap();assert_eq!(prepared["type"],"play");
        assert_eq!(prepared["paused"],true);
        f.s.endpoints.write().await.get_mut(&source.endpoint_id).unwrap()["ready_playback_id"]=prepared["playback_id"].clone();
        let resumed=rx.recv().await.unwrap();assert_eq!(resumed["playback_id"],prepared["playback_id"]);
        for i in 0..p.len(){
            assert_eq!(receive(&mut p[i]).await["type"],"cancel_start");
            let mut final_value=final_report_value(&commands[i],&commits[i],0,json!([]));
            final_value["measured_version"]=json!(2);final_value["sample_rate"]=json!(768000);
            report(&f,&p[i],final_value).await;
        }
        prepared
    });
    assert_eq!(returned.0,StatusCode::OK,"{}",returned.1);
    assert_eq!(returned.1["target"]["status"],"playing");assert_eq!(returned.1["target"]["volume"],0.25);
    assert_eq!(returned.1["target"]["queue_index"],1);assert_eq!(returned.1["target"]["playback_id"],prepared["playback_id"]);
    assert_eq!(returned.1["source"]["status"],"stopped");
}
