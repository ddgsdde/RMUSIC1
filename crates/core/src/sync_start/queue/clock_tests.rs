//! Real coordinator/API; synthetic calibrated driver samples, never a physical device.
use super::*;

async fn released(f:&Fixture,p:&[Peer],commands:&[Value],commits:&[Value]) {
    for (i,peer) in p.iter().enumerate(){let mut value=final_value(&commands[i],&commits[i],0,48000,false,true);value["phase"]=json!("released");value["final"]=json!(false);report(f,peer,value).await;}
}
fn calibration(peer:&Peer,command:&Value,frames:u64,time:u64,epoch:u64,ppm:Option<f64>)->Value {
    let counter=(1u64<<60)+frames;
    json!({"version":1,"core_clock_id":command["start"]["core_clock_id"],"host_clock_id":command["start"]["host_clock_id"],"device_epoch":epoch,"source":"wasapi_ticks","timestamp_domain":"qpc","counter":counter,"nominal_frequency_hz":10_000_000,"core_sample_ns":time,"core_read_before_ns":time,"core_read_after_ns":time,"host_network_uncertainty_ns":1000,"driver_read_span_ns":100,"valid_until_core_ns":time+500_000_000,"running":true,"device_clock_mapped":true,"driver_accuracy_verified":false,"rate_observation":ppm.map(|ppm|json!({"observed_frequency_hz":10_000_000.*(1.+ppm/1e6),"observed_drift_ppm":ppm,"sample_count":4,"span_ns":1_000_000_000u64,"query_time_approximation":false,"rate_sane":true})),"programme_clock":{"version":1,"zone_id":peer.zone,"core_playback_id":command["playback_id"],"track_id":command["track"]["id"],"position":command["position"].as_f64().unwrap()+frames as f64/48000.,"source_frames":frames,"output_frame":frames,"sample_rate":48000,"device_epoch":epoch,"device_counter":counter,"core_sample_ns":time,"driver_accuracy_verified":false}})
}
async fn feed(f:&Fixture,peer:&Peer,value:Value) {
    f.s.endpoints.write().await.get_mut(&peer.endpoint).unwrap()["device_clock_calibration"]=value;
    sync_start::observe_device_clock(&f.s,&peer.endpoint,&peer.tx).await;
}
async fn diagnostic(f:&Fixture,path:&str)->Value {f.call("GET",path,None).await.1["clock_diagnostics"].clone()}
#[tokio::test]
async fn group_clock_source_clock_v2_uses_programme_phase_and_preserves_device_drift_identity(){
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable(&f,&p).await;
    for peer in &p{let mut endpoints=f.s.endpoints.write().await;let caps=&mut endpoints.get_mut(&peer.endpoint).unwrap()["capabilities"];caps["group_output_v2"]=json!(true);caps["measured_listens_v2"]=json!(true);}
    let mut req=request(&track,&p);req["measured_version"]=json!(2);let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req)).await.0,StatusCode::ACCEPTED);
    let commands=vec![receive(&mut p[0]).await,receive(&mut p[1]).await];
    for i in 0..2{assert_eq!(f.call("POST",&format!("/api/v1/zones/{}/prepare",p[i].zone),Some(json!({"endpoint_id":p[i].endpoint,"track_id":track,"playback_id":commands[i]["playback_id"],"measured_version":2}))).await.0,StatusCode::OK);let mut value=ready(&commands[i]);value["measured_version"]=json!(2);report(&f,&p[i],value).await;}
    let commits=vec![receive(&mut p[0]).await,receive(&mut p[1]).await];let deadline=f.call("GET",&path,None).await.1["deadline_core_ns"].as_u64().unwrap();tokio::time::sleep(std::time::Duration::from_nanos(deadline.saturating_sub(clock::now_ns())+1_010_000_000)).await;
    for i in 0..2{let mut value=final_value(&commands[i],&commits[i],0,48000,false,true);value["phase"]=json!("released");value["final"]=json!(false);value["measured_version"]=json!(2);report(&f,&p[i],value).await;}
    let time=clock::now_ns()-100_000;for i in 0..2{let mut value=calibration(&p[i],&commands[i],4800,time,1,Some(0.));let pair=&mut value["programme_clock"];pair["version"]=json!(2);pair["source_frame"]=json!(4800+i*2);pair["source_fraction"]=json!(if i==0{0.}else{0.25});pair["source_per_output"]=json!(if i==0{1.}else{1.0005});pair["track_output_frames"]=json!(4800);pair["position"]=json!((4800.+i as f64*2.25)/48000.);feed(&f,&p[i],value).await;}
    let value=diagnostic(&f,&path).await;assert_eq!(value["state"],"available","{value}");let row=&value["members"][1];let expected=46875.+(value["evaluated_at_core_ns"].as_u64().unwrap()-time) as f64*0.0005;assert!((row["estimated_offset_ns"].as_i64().unwrap() as f64-expected).abs()<=2.);assert_eq!(row["relative_drift_ppm"],0.);assert!((row["relative_programme_drift_ppm"].as_f64().unwrap()-500.).abs()<1e-6);assert!(row["processing_projection_uncertainty_ns"].as_u64().unwrap()>0);assert_eq!(value["strict_sync"],false);assert_eq!(total(&f).await,0.);
    feed(&f,&p[1],calibration(&p[1],&commands[1],4800,time,1,Some(0.))).await;assert_eq!(diagnostic(&f,&path).await["members"][1]["available"],false,"changing programme units within an output invalidates its binding");
}

#[tokio::test]
async fn group_clock_projects_one_time_observes_relative_rate_and_keeps_queue_epoch_identity() {
    let(f,_,mut p,_,path)=setup(true).await;let(commands,commits)=epoch(&f,&mut p,48000).await;released(&f,&p,&commands,&commits).await;
    tokio::time::sleep(std::time::Duration::from_millis(1010)).await;let time=clock::now_ns()-10_000_000;
    feed(&f,&p[0],calibration(&p[0],&commands[0],4800,time,1,Some(0.))).await;feed(&f,&p[1],calibration(&p[1],&commands[1],5280,time,1,Some(100.))).await;
    let value=diagnostic(&f,&path).await;assert_eq!(value["state"],"available");assert_eq!(value["reference_zone_id"],p[0].zone);assert!(value["total_uncertainty_ns"].is_null());assert_eq!(value["strict_sync"],false);assert_eq!(value["correction"],"off");
    let expected=10_000_000.+(value["evaluated_at_core_ns"].as_u64().unwrap()-time) as f64*0.0001;assert!((value["members"][1]["estimated_offset_ns"].as_i64().unwrap() as f64-expected).abs()<=1.);
    assert!((value["members"][1]["relative_drift_ppm"].as_f64().unwrap()-100.).abs()<1e-6);assert_eq!(value["members"][1]["known_host_uncertainty_ns"],2000);assert!(value["valid_for_ms"].as_u64().unwrap()<=490);assert_eq!(total(&f).await,0.,"clock reads must never credit listening");
    let plan=prepare(&f,&p[1],&commands[1]).await;let body=handoff(&commands[1],&plan,0,4800,960);
    let(status,result)=f.call("POST",&format!("/api/v1/zones/{}/handoff",p[1].zone),Some(body)).await;assert_eq!(status,StatusCode::OK,"{result}");assert_eq!(result["accepted"],true);
    let time=clock::now_ns()-500_000;let before=total(&f).await;feed(&f,&p[0],calibration(&p[0],&commands[0],4800,time,1,None)).await;
    let mut next=calibration(&p[1],&commands[1],5760,time,1,None);next["programme_clock"]["core_playback_id"]=plan["items"][0]["playback_id"].clone();next["programme_clock"]["source_frames"]=json!(960);next["programme_clock"]["position"]=json!(0.02);feed(&f,&p[1],next).await;
    let value=diagnostic(&f,&path).await;assert_eq!(value["state"],"available");assert_eq!(value["members"][1]["estimated_offset_ns"],20_000_000);assert!(value["members"][1]["relative_drift_ppm"].is_null());assert_eq!(value["members"][1]["projection"],"nominal");assert_eq!(total(&f).await,before);
    f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut p).await;report(&f,&p[0],final_value(&commands[0],&commits[0],4800,48000,false,true)).await;
    let mut old=boundary(&commands[1]["playback_id"],&commands[1]["track"]["id"],4800,48000,0.);old["ended"]=json!(true);let mut new=boundary(&plan["items"][0]["playback_id"],&commands[1]["track"]["id"],960,48000,0.);new["ended"]=json!(false);
    report(&f,&p[1],final_report_value(&commands[1],&commits[1],5760,json!([old,new]))).await;assert_eq!(diagnostic(&f,&path).await["state"],"inactive");assert_eq!(open_sessions(&f).await,0);
}

#[tokio::test]
async fn group_clock_rejects_stale_foreign_unmapped_reset_and_retired_output_feedback() {
    let(f,_,mut p,_,path)=setup(true).await;let(commands,commits)=epoch(&f,&mut p,48000).await;released(&f,&p,&commands,&commits).await;let time=clock::now_ns()-500_000;
    let original=calibration(&p[1],&commands[1],0,time,1,None);feed(&f,&p[0],calibration(&p[0],&commands[0],0,time,1,None)).await;feed(&f,&p[1],original.clone()).await;assert_eq!(diagnostic(&f,&path).await["state"],"available");
    for field in ["stale","foreign","opaque","rate"] {
        let mut value=original.clone();match field {"stale"=>value["valid_until_core_ns"]=json!(0),"foreign"=>value["programme_clock"]["core_playback_id"]=json!(uuid::Uuid::new_v4().to_string()),"opaque"=>value["device_clock_mapped"]=json!(false),_=>value["rate_observation"]=json!({"rate_sane":false})};feed(&f,&p[1],value).await;
        let value=diagnostic(&f,&path).await;assert_eq!(value["state"],"partial","{field}: {value}");assert_eq!(value["members"][1]["available"],false);assert!(value["members"][1]["estimated_offset_ns"].is_null());
    }
    feed(&f,&p[1],calibration(&p[1],&commands[1],0,time,2,None)).await;feed(&f,&p[1],original.clone()).await;let value=diagnostic(&f,&path).await;assert_eq!(value["members"][1]["reason"],"设备输出已重置，请重新起播");
    control(&f,&path,operation("pause")).await;stops(&mut p).await;finals(&f,&p,&commands,&commits,[0,0],48000,false).await;assert_eq!(diagnostic(&f,&path).await["state"],"inactive");
    control(&f,&path,operation("resume")).await;let(resumed,commits)=epoch(&f,&mut p,48000).await;released(&f,&p,&resumed,&commits).await;feed(&f,&p[1],original).await;assert_ne!(diagnostic(&f,&path).await["state"],"available");
    let time=clock::now_ns()-500_000;for(i,peer)in p.iter().enumerate(){feed(&f,peer,calibration(peer,&resumed[i],0,time,2,None)).await;}assert_eq!(diagnostic(&f,&path).await["state"],"available");
    let(tx,_rx)=mpsc::channel(8);f.s.bridges.write().await.insert(p[1].endpoint.clone(),tx);assert_eq!(diagnostic(&f,&path).await["members"][1]["reason"],"当前输出连接已变化");f.s.bridges.write().await.insert(p[1].endpoint.clone(),p[1].tx.clone());assert_eq!(total(&f).await,0.);
    f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut p).await;finals(&f,&p,&resumed,&commits,[0,0],48000,false).await;
}
