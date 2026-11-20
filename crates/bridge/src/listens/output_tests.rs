use super::*;
fn owner()->Owner {Owner{core_id:"core-qa".into(),user_id:"user-qa".into()}}
fn recorder(store:Arc<Store>,start:f64)->Recorder {
    Recorder::new(store,owner(),"track".into(),"PCM".into(),"QA".into(),120.).with_measured_output(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),Instant::now(),start).unwrap()
}
fn total(s:&Snapshot)->f64{s.days.values().map(|day|day.seconds).sum()}
#[test]
fn dual_listen_journal_counts_output_frames_and_rejects_unpaired_or_changed_counters() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let mut r=recorder(store,10.);let at=chrono::DateTime::parse_from_rfc3339("2026-10-17T12:00:00Z").unwrap().timestamp_millis();
    r.sample_device_at(11.0005,768000,768000,at,1.).unwrap();assert_eq!(total(&r.row.snapshot),1.);assert_eq!(r.row.snapshot.position,11.0005);
    let before=serde_json::to_value(&r.row).unwrap();
    for (position,frames,rate,mono) in [(12.,768000,768000,2.),(12.,768001,768000,2.),(10.,700000,768000,2.),(12.0005,864000,96000,2.),(12.0005,1536000,768000,f64::NAN)] {r.sample_device_at(position,frames,rate,at+1000,mono).unwrap();assert_eq!(serde_json::to_value(&r.row).unwrap(),before);}
    r.sample_at(90.,true,false,at+1000,2.).unwrap();assert_eq!(serde_json::to_value(&r.row).unwrap(),before);
    r.sample_device_at(12.001,1536000,768000,at+1000,2.).unwrap();assert_eq!(total(&r.row.snapshot),2.);let m=r.row.snapshot.measured.as_ref().unwrap();assert_eq!(m.version,2);assert_eq!(m.segments.len(),1);assert_eq!(m.segments[0].output.as_ref().unwrap().end,1536000);assert_eq!(m.segments[0].start,10.);assert_eq!(m.segments[0].end,12.001);
    r.sample_device_at(13.0015,2304000,768000,at+500,3.).unwrap();assert_eq!(total(&r.row.snapshot),3.);
    assert_eq!(r.row.snapshot.measured.as_ref().unwrap().segments[0].last_at,at+1000,"a wall-clock step backwards must not regress an already persisted segment timestamp");
}
#[test]
fn dual_listen_midnight_frame_partition_survives_reopen_without_legacy_downgrade() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let mut r=recorder(store.clone(),5.);let at=chrono::DateTime::parse_from_rfc3339("2026-10-17T00:00:01.500Z").unwrap().timestamp_millis();
    r.sample_device_at(7.001,96000,48000,at,2.).unwrap();r.finish("ended").unwrap();store.drain().unwrap();assert_eq!(total(&r.row.snapshot),2.);
    assert_eq!(r.row.snapshot.days["2026-10-16"].seconds,0.5);assert_eq!(r.row.snapshot.days["2026-10-17"].seconds,1.5);let m=r.row.snapshot.measured.as_ref().unwrap();assert_eq!(m.segments.len(),2);assert_eq!(m.segments[0].output.as_ref().unwrap().end,24000);assert_eq!(m.segments[1].output.as_ref().unwrap().start,24000);
    let snapshot=serde_json::to_value(&r.row.snapshot).unwrap();assert_eq!(snapshot["measured"]["version"],2);drop(r);drop(store);
    let store=Store::open(dir.path()).unwrap();let rows=store.pending(&owner()).unwrap();assert_eq!(rows.len(),1);assert_eq!(serde_json::to_value(&rows[0].snapshot).unwrap(),snapshot);assert!(store.pending_timing(&owner(),1).unwrap().is_empty());
    let db=store.db.lock().unwrap();let legacy:i64=db.query_row("SELECT (SELECT COUNT(*) FROM listens)+(SELECT COUNT(*) FROM measured_listens)",[],|row|row.get(0)).unwrap();assert_eq!(legacy,0);
}
#[test]
fn dual_listen_new_rows_do_not_starve_v1_or_change_v1_wire_shape() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let at=chrono::Utc::now().timestamp_millis();
    for _ in 0..40 {let mut r=recorder(store.clone(),0.);r.sample_device_at(0.10005,4800,48000,at,0.1).unwrap();r.finish("ended").unwrap();}
    let mut legacy=Recorder::new(store.clone(),owner(),"v1".into(),"old".into(),"QA".into(),120.).with_measured(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),Instant::now());legacy.sample_device_at(0.25,12000,48000,at,0.25).unwrap();legacy.finish("ended").unwrap();store.drain().unwrap();
    assert_eq!(store.pending(&owner()).unwrap().len(),32);let old=store.pending_timing(&owner(),1).unwrap();assert_eq!(old.len(),1);assert_eq!(old[0].snapshot.id,legacy.row.snapshot.id);let value=serde_json::to_value(&old[0].snapshot).unwrap();assert!(value["measured"].get("version").is_none());assert!(value["measured"]["segments"][0].get("output").is_none());assert!(store.pending_timing(&owner(),0).unwrap().is_empty());
}
#[test]
fn dual_listen_collector_keeps_one_mode_and_ignores_zero_or_retired_sources() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let mut collector=Collector::new(store.clone(),owner());
    collector.sample_output("a",10.,10.,0,768000,||panic!("unheard source must not read metadata")).unwrap();
    collector.sample_output("a",10.,11.0005,768000,768000,||Ok(Some(Source{listen_session_id:"listen".into(),track_id:"track".into(),title:"PCM".into(),artist:"QA".into(),duration:120.}))).unwrap();
    assert!(collector.sample("a",11.0005,768000,768000,||panic!("existing source must not reopen")).is_err());collector.finish("a","ended").unwrap();collector.sample_output("a",10.,12.001,1536000,768000,||panic!("retired source must not reopen")).unwrap();store.drain().unwrap();let rows=store.pending(&owner()).unwrap();assert_eq!(rows.len(),1);assert_eq!(total(&rows[0].snapshot),1.);assert_eq!(rows[0].snapshot.measured.as_ref().unwrap().version,2);
}

#[test]
fn dual_listen_client_serializer_matches_the_shared_core_wire_fixture() {
    let golden:serde_json::Value=serde_json::from_str(include_str!("../../testdata/dual-listen-v2.json")).unwrap();let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let mut r=recorder(store,10.);let at=golden["updated_at"].as_i64().unwrap();r.sample_device_at(11.0005,768000,768000,at,1.).unwrap();r.finish("ended").unwrap();let mut value=serde_json::to_value(&r.row.snapshot).unwrap();
    value["id"]=golden["id"].clone();value["measured"]["playback_id"]=golden["measured"]["playback_id"].clone();value["measured"]["listen_session_id"]=golden["measured"]["listen_session_id"].clone();assert_eq!(value,golden);
}
