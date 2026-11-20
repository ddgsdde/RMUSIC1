use super::*;
fn owner() -> Owner {
    Owner {
        core_id: "core-a".into(),
        user_id: "user-a".into(),
    }
}
fn recorder(store: Arc<Store>) -> Recorder {
    Recorder::new(
        store,
        owner(),
        "cached-track".into(),
        "离线音乐".into(),
        "QA".into(),
        120.,
    )
}
fn total(s: &Snapshot) -> f64 {
    s.days.values().map(|d| d.seconds).sum()
}
#[test]
fn actual_output_clock_seek_pause_and_midnight_are_measured_without_webview() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut r = recorder(store.clone());
    let at = chrono::DateTime::parse_from_rfc3339("2026-10-12T23:59:50Z")
        .unwrap()
        .timestamp_millis();
    r.row.snapshot.started_at = at;
    r.row.snapshot.updated_at = at;
    let control = rmusic_audio::output::PlaybackControl::default();
    r.sample_at(control.seconds(48000), true, true, at, 0.)
        .unwrap();
    control
        .frames_played
        .store(960000, std::sync::atomic::Ordering::Relaxed);
    control
        .frames_written
        .store(960000, std::sync::atomic::Ordering::Relaxed);
    r.sample_at(control.seconds(48000), false, false, at + 20000, 20.)
        .unwrap();
    r.sample_at(20., true, false, at + 25000, 25.).unwrap();
    r.sample_at(21., true, false, at + 26000, 26.).unwrap();
    r.sample_at(90., true, true, at + 26000, 26.).unwrap();
    r.sample_at(91., true, false, at + 27000, 27.).unwrap();
    r.sample_at(120., true, false, at + 90000, 90.).unwrap();
    r.sample_at(119., true, false, at + 91000, 91.).unwrap();
    r.sample_at(120., true, false, at + 92000, 92.).unwrap();
    r.sample_at(121., true, false, at + 3600000, 93.).unwrap();
    r.finish("stop").unwrap();
    store.drain().unwrap();
    let pending = store.pending(&owner()).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(total(&pending[0].snapshot), 23.);
    assert_eq!(pending[0].snapshot.days["2026-10-12"].seconds, 10.);
    assert_eq!(pending[0].snapshot.reason, "stop");
    let version = r.row.snapshot.version;
    r.checkpoint().unwrap();
    r.finish("replaced").unwrap();
    assert_eq!(r.row.snapshot.version, version);
}
#[test]
fn native_journal_survives_restart_and_acks_only_the_accepted_version_and_owner() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.remember("profile", &owner()).unwrap();
    let mut r = recorder(store.clone());
    let at = r.row.snapshot.started_at;
    r.sample_at(0., true, true, at, 0.).unwrap();
    r.sample_at(10., true, false, at + 10000, 10.).unwrap();
    store.drain().unwrap();
    let first = r.row.snapshot.clone();
    r.sample_at(15., false, false, at + 15000, 15.).unwrap();
    r.finish("interrupted").unwrap();
    let final_snapshot = r.row.snapshot.clone();
    store.drain().unwrap();
    store
        .acknowledge(
            &owner(),
            &SyncResult {
                accepted: vec![Ack {
                    id: first.id.clone(),
                    version: first.version,
                }],
                rejected: vec![],
            },
        )
        .unwrap();
    assert_eq!(
        store.pending(&owner()).unwrap()[0].snapshot.version,
        final_snapshot.version
    );
    let other = Owner {
        user_id: "another-user".into(),
        ..owner()
    };
    store
        .acknowledge(
            &other,
            &SyncResult {
                accepted: vec![Ack {
                    id: first.id.clone(),
                    version: final_snapshot.version,
                }],
                rejected: vec![],
            },
        )
        .unwrap();
    assert_eq!(store.pending(&owner()).unwrap().len(), 1);
    let client = store.client_id.clone();
    drop(r);
    drop(store);
    let restored = Store::open(dir.path()).unwrap();
    assert_eq!(restored.client_id, client);
    assert_eq!(restored.owner("profile").unwrap(), Some(owner()));
    assert_eq!(total(&restored.pending(&owner()).unwrap()[0].snapshot), 15.);
    restored
        .acknowledge(
            &owner(),
            &SyncResult {
                accepted: vec![Ack {
                    id: first.id.clone(),
                    version: final_snapshot.version,
                }],
                rejected: vec![],
            },
        )
        .unwrap();
    restored
        .save(Row {
            owner: owner(),
            snapshot: first,
        })
        .unwrap();
    restored.drain().unwrap();
    assert!(restored.pending(&owner()).unwrap().is_empty());
    restored.forget("profile").unwrap();
    assert!(restored.owner("profile").unwrap().is_none());
}
#[test]
fn native_rejected_version_is_retained_and_other_listens_can_sync() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut first = recorder(store.clone());
    let mut second = recorder(store.clone());
    first.finish("ended").unwrap();
    second.finish("ended").unwrap();
    store.drain().unwrap();
    store
        .acknowledge(
            &owner(),
            &SyncResult {
                accepted: vec![],
                rejected: vec![Rejected {
                    id: first.row.snapshot.id.clone(),
                    version: first.row.snapshot.version,
                    code: "invalid_snapshot".into(),
                }],
            },
        )
        .unwrap();
    let pending = store.pending(&owner()).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].snapshot.id, second.row.snapshot.id);
}
#[test]
fn account_changes_across_addresses_of_one_core_update_the_cached_listener() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.remember("lan", &owner()).unwrap();
    let changed = Owner {
        user_id: "user-b".into(),
        ..owner()
    };
    store.remember("remote", &changed).unwrap();
    assert_eq!(store.owner("lan").unwrap(), Some(changed));
    store.forget("remote").unwrap();
    assert!(store.owner("lan").unwrap().is_none());
}

#[test]
fn measured_frames_include_first_sample_resume_and_same_millisecond_audio() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    let mut r=recorder(store.clone()).with_measured(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),Instant::now());
    let at=chrono::Utc::now().timestamp_millis();
    r.sample_device_at(5.25,12000,48000,at,2.25).unwrap();
    r.sample_device_at(5.25,12000,48000,at+5000,7.25).unwrap();
    r.sample_device_at(5.5,24000,48000,at+5250,7.5).unwrap();
    r.sample_device_at(5.5+1./48000.,24001,48000,at+5250,7.5).unwrap();
    r.sample_device_at(50.,24002,48000,at+5250,7.5).unwrap(); // A seek requires a new source identity.
    r.finish("interrupted").unwrap();store.drain().unwrap();
    let s=&store.pending(&owner()).unwrap()[0].snapshot;
    assert!((total(s)-(0.5+1./48000.)).abs()<1e-9);
    let segments=&s.measured.as_ref().unwrap().segments;
    assert_eq!(segments.len(),1);assert_eq!(segments[0].start,5.);
    assert!((segments[0].end-s.position).abs()<1e-9);
    assert_eq!(s.started_at,at-250);
}

#[test]
fn measured_midnight_intervals_survive_restart_and_legacy_clients_cannot_read_them() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    let mut r=recorder(store.clone()).with_measured(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),Instant::now());
    let at=chrono::DateTime::parse_from_rfc3339("2026-10-13T00:00:00.100Z").unwrap().timestamp_millis();
    r.sample_device_at(0.2,9600,48000,at,0.2).unwrap();r.finish("ended").unwrap();store.drain().unwrap();
    assert_eq!(store.db.lock().unwrap().query_row("SELECT COUNT(*) FROM listens",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(store.pending_supported(&owner(),false).unwrap().is_empty());
    let id=r.row.snapshot.id.clone();let version=r.row.snapshot.version;
    drop(r);drop(store);let store=Store::open(dir.path()).unwrap();
    let rows=store.pending(&owner()).unwrap();let s=&rows[0].snapshot;
    assert!((s.days["2026-10-12"].seconds-0.1).abs()<1e-9);
    assert!((s.days["2026-10-13"].seconds-0.1).abs()<1e-9);
    assert_eq!(s.measured.as_ref().unwrap().segments.len(),2);
    store.acknowledge(&Owner {user_id:"wrong-user".into(),..owner()},&SyncResult {accepted:vec![Ack {id:id.clone(),version}],rejected:vec![]}).unwrap();
    assert_eq!(store.pending(&owner()).unwrap().len(),1);
    store.acknowledge(&owner(),&SyncResult {accepted:vec![Ack {id,version}],rejected:vec![]}).unwrap();
    assert!(store.pending(&owner()).unwrap().is_empty());
}

#[test]
fn an_old_core_can_sync_legacy_rows_behind_more_than_one_batch_of_measured_rows() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    for _ in 0..40 {
        let mut r=recorder(store.clone()).with_measured(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),Instant::now());
        r.sample_device_at(0.25,12000,48000,chrono::Utc::now().timestamp_millis(),0.25).unwrap();r.finish("ended").unwrap();
    }
    let mut legacy=recorder(store.clone());legacy.finish("ended").unwrap();store.drain().unwrap();
    let rows=store.pending_supported(&owner(),false).unwrap();
    assert_eq!(rows.len(),1);assert_eq!(rows[0].snapshot.id,legacy.row.snapshot.id);
    assert_eq!(store.pending(&owner()).unwrap().len(),32);
}

#[test]
fn shared_collector_records_only_heard_frames_and_finishes_each_source_once() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let mut collector=Collector::new(store.clone(),owner());
    collector.sample("a",0.,0,48000,||panic!("zero-frame preloads must not read metadata")).unwrap();
    collector.sample("a",0.25,12000,48000,||Ok(Some(Source {listen_session_id:"listen-a".into(),track_id:"track-a".into(),title:"A".into(),artist:"QA".into(),duration:2.}))).unwrap();
    collector.finish("a","ended").unwrap();
    collector.sample("a",0.5,24000,48000,||panic!("late terminal callback must not reopen a source")).unwrap();
    collector.sample("b",0.,0,48000,||panic!("prefetched but unheard source must not create history")).unwrap();
    collector.finish("a","interrupted").unwrap();collector.finish_all("interrupted").unwrap();store.drain().unwrap();
    let rows=store.pending(&owner()).unwrap();assert_eq!(rows.len(),1);assert_eq!(rows[0].snapshot.reason,"ended");assert_eq!(total(&rows[0].snapshot),0.25);
    assert_eq!(rows[0].snapshot.measured.as_ref().unwrap().playback_id,"a");
}
#[test]
fn system_suspend_hold_survives_reopen_without_changing_listening_history() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();assert_eq!(store.system_suspend_state().unwrap(),(0,false));
    store.save_system_suspend(7,true).unwrap();drop(store);
    let store=Store::open(dir.path()).unwrap();assert_eq!(store.system_suspend_state().unwrap(),(7,true));
    store.save_system_suspend(7,false).unwrap();drop(store);
    let store=Store::open(dir.path()).unwrap();assert_eq!(store.system_suspend_state().unwrap(),(7,false));
    let count:i64=store.db.lock().unwrap().query_row("SELECT count(*) FROM listens",[],|r|r.get(0)).unwrap();assert_eq!(count,0);
}
#[test]
fn system_suspend_corrupt_state_is_rejected_instead_of_implicitly_releasing_hold() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    store.db.lock().unwrap().execute("INSERT INTO settings(key,value) VALUES('system_suspend','{\"generation\":0,\"held\":true}')",[]).unwrap();assert!(store.system_suspend_state().is_err());
    store.db.lock().unwrap().execute("UPDATE settings SET value='not-json' WHERE key='system_suspend'",[]).unwrap();assert!(store.system_suspend_state().is_err());
}

#[test]
fn stopped_transport_uses_the_protocol_reason_and_requeues_unaccepted_legacy_records() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    let mut record=recorder(store.clone());record.finish("stopped").unwrap();store.drain().unwrap();
    assert_eq!(store.pending(&owner()).unwrap()[0].snapshot.reason,"stop");
    store.db.lock().unwrap().execute("UPDATE listens SET payload=json_set(payload,'$.snapshot.reason','stopped'),error='invalid_snapshot'",[]).unwrap();
    assert!(store.pending(&owner()).unwrap().is_empty());drop(record);drop(store);
    let restored=Store::open(dir.path()).unwrap();let rows=restored.pending(&owner()).unwrap();
    assert_eq!(rows.len(),1);assert_eq!(rows[0].snapshot.reason,"stop");
}
