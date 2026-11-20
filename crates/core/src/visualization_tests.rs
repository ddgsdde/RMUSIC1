use super::*;
#[tokio::test]
async fn visualization_viewers_share_one_lease_and_stale_or_replaced_outputs_are_rejected() {
    let f=Fixture::new().await;let (owner,mut commands)=tokio::sync::mpsc::channel(16);
    let mut zone=crate::models::Zone::new("visual-zone".into(),"QA".into(),"visual-output".into());zone.status="playing".into();zone.playback_id=Some("visual-instance".into());
    let track:crate::models::Track=serde_json::from_value(json!({"id":"visual-track","title":"QA","artist":"QA","album":"QA","duration":10.,"format":"wav","sample_rate":48000,"bit_depth":16,"channels":2,"source":"local","album_artist":"QA","track_number":1,"disc_number":1,"bitrate":1536000,"cover_url":null,"recording_id":"","available":true,"created_at":0})).unwrap();zone.track=Some(track);
    f.s.zones.write().await.insert(zone.id.clone(),zone.clone());
    f.s.bridges.write().await.insert(zone.endpoint_id.clone(),owner.clone());
    f.s.endpoints.write().await.insert(zone.endpoint_id.clone(),json!({"online":true,"capabilities":{"visualization_v1":true}}));
    // Two simultaneous viewers, one decoder-side request. Core never opens /not-opened.wav.
    let first=f.call("POST","/api/v1/zones/visual-zone/visualization",Some(json!({})));
    let second=f.call("POST","/api/v1/zones/visual-zone/visualization",Some(json!({})));
    let (a,b)=tokio::join!(first,second);assert_eq!(a.0,StatusCode::OK);assert_eq!(b.0,StatusCode::OK);assert!(a.1["frames"].as_array().unwrap().is_empty());
    assert_eq!(commands.try_recv().unwrap()["type"],"visualization_lease");assert!(commands.try_recv().is_err());
    let report=json!({"version":1,"endpoint_id":zone.endpoint_id,"zone_id":zone.id,"core_playback_id":zone.playback_id,"track_id":"visual-track","position":0.5,"frames":[{"playback_id":"visual-instance","position":0.4,"bands":[0,1,2,3,4,5,6,7],"peak":20,"energy":10}]});
    crate::visualization::accept(&f.s,&owner,report.clone()).await;
    let read=f.call("POST","/api/v1/zones/visual-zone/visualization",Some(json!({}))).await;assert_eq!(read.1["frames"].as_array().unwrap().len(),1);
    let next=f.call("POST","/api/v1/zones/visual-zone/visualization",Some(json!({"after":0.4}))).await;assert!(next.1["frames"].as_array().unwrap().is_empty());
    let (replacement,_)=tokio::sync::mpsc::channel(16);f.s.bridges.write().await.insert(zone.endpoint_id.clone(),replacement);
    crate::visualization::accept(&f.s,&owner,report).await;
    let replaced=f.call("POST","/api/v1/zones/visual-zone/visualization",Some(json!({}))).await;assert!(replaced.1["frames"].as_array().unwrap().is_empty());
}
