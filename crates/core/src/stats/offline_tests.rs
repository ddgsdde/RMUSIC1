use super::offline::*;
use super::*;
use std::collections::BTreeMap;
fn db() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(include_str!("../schema.sql")).unwrap();
    c
}
fn batch() -> Batch {
    let start = chrono::DateTime::parse_from_rfc3339("2026-10-12T23:59:40Z")
        .unwrap()
        .timestamp_millis();
    Batch {
        core_id: "core".into(),
        user_id: "user".into(),
        client_id: uuid::Uuid::new_v4().to_string(),
        sessions: vec![Snapshot {
            id: uuid::Uuid::new_v4().to_string(),
            version: 1,
            track_id: "removed-track".into(),
            title: "缓存曲目".into(),
            artist: "艺术家".into(),
            duration: 120.,
            started_at: start,
            updated_at: start + 40000,
            position: 40.,
            reason: "active".into(),
            measured:None,
            days: BTreeMap::from([
                (
                    "2026-10-12".into(),
                    Day {
                        seconds: 20.,
                        last_at: start + 19999,
                    },
                ),
                (
                    "2026-10-13".into(),
                    Day {
                        seconds: 20.,
                        last_at: start + 40000,
                    },
                ),
            ]),
        }],
    }
}
#[test]
fn offline_versions_duplicates_day_split_and_history_are_atomic() {
    let mut c = db();
    let mut b = batch();
    let now = b.sessions[0].updated_at;
    assert_eq!(import(&mut c, &b, now).unwrap()["changed"], true);
    assert_eq!(import(&mut c, &b, now).unwrap()["changed"], false);
    let stale = b.sessions[0].clone();
    b.sessions[0].version = 2;
    b.sessions[0].updated_at += 10000;
    b.sessions[0].position = 50.;
    b.sessions[0].days.get_mut("2026-10-13").unwrap().seconds = 30.;
    b.sessions[0].days.get_mut("2026-10-13").unwrap().last_at = now + 10000;
    b.sessions[0].reason = "stop".into();
    import(&mut c, &b, now + 10000).unwrap();
    import(&mut c, &b, now + 10000).unwrap();
    b.sessions[0] = stale;
    import(&mut c, &b, now + 10000).unwrap();
    let stats = track_at(&c, "removed-track").unwrap().unwrap();
    assert_eq!(stats["listened_seconds"], 50.);
    assert_eq!(stats["play_count"], 1);
    assert_eq!(stats["skip_count"], 1);
    let days = overview_at(&c, 2, now).unwrap();
    assert_eq!(days["days"][0]["listened_seconds"], 20.);
    assert_eq!(days["days"][1]["listened_seconds"], 30.);
    assert_eq!(days["days"][1]["play_count"], 1);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM play_sessions WHERE track_id='removed-track'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn offline_conflicting_versions_and_terminal_reopen_cannot_change_totals() {
    let mut c = db();
    let mut b = batch();
    let now = b.sessions[0].updated_at;
    import(&mut c, &b, now).unwrap();
    let original = b.sessions[0].clone();
    for change in 0..5 {
        b.sessions[0] = original.clone();
        b.sessions[0].version = 2;
        match change {
            0 => b.sessions[0].days.get_mut("2026-10-13").unwrap().seconds = 10.,
            1 => b.sessions[0].track_id = "another".into(),
            2 => b.sessions[0].started_at -= 1,
            3 => b.sessions[0].duration = 200.,
            _ => {
                b.sessions[0].version = 1;
                b.sessions[0].position = 30.;
            }
        }
        assert_eq!(
            import(&mut c, &b, now).unwrap()["rejected"][0]["code"],
            "conflicting_snapshot"
        );
    }
    b.sessions[0] = original;
    b.sessions[0].version = 2;
    b.sessions[0].reason = "interrupted".into();
    import(&mut c, &b, now).unwrap();
    b.sessions[0].version = 3;
    b.sessions[0].reason = "active".into();
    assert_eq!(
        import(&mut c, &b, now).unwrap()["rejected"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        track_at(&c, "removed-track").unwrap().unwrap()["listened_seconds"],
        40.
    );
}
#[test]
fn offline_invalid_snapshots_do_not_block_valid_rows_and_old_backfill_preserves_recency() {
    let mut c = db();
    let mut b = batch();
    let now = b.sessions[0].updated_at;
    import(&mut c, &b, now).unwrap();
    let recent = track_at(&c, "removed-track").unwrap().unwrap()["last_played_at"].clone();
    let mut earlier = b.sessions[0].clone();
    earlier.id = uuid::Uuid::new_v4().to_string();
    earlier.started_at -= 86400000;
    earlier.updated_at -= 86400000;
    earlier.days = earlier
        .days
        .into_iter()
        .map(|(_, mut d)| {
            d.last_at -= 86400000;
            (day(d.last_at), d)
        })
        .collect();
    let mut invalid = earlier.clone();
    invalid.id = uuid::Uuid::new_v4().to_string();
    invalid.days.values_mut().next().unwrap().seconds = 86401.;
    b.sessions = vec![invalid, earlier];
    let result = import(&mut c, &b, now).unwrap();
    assert_eq!(result["rejected"].as_array().unwrap().len(), 1);
    assert_eq!(result["accepted"].as_array().unwrap().len(), 1);
    let stats = track_at(&c, "removed-track").unwrap().unwrap();
    assert_eq!(stats["last_played_at"], recent);
    assert_eq!(stats["listened_seconds"], 80.);
}
#[test]
fn offline_owner_and_snapshot_limits_are_checked() {
    let mut b = batch();
    assert!(owner(&b, "core", "user").is_ok());
    assert!(owner(&b, "other-core", "user").is_err());
    assert!(owner(&b, "core", "other-user").is_err());
    let original = b.sessions[0].clone();
    let now = original.updated_at;
    for change in 0..7 {
        let mut s = original.clone();
        match change {
            0 => s.position = f64::NAN,
            1 => s.duration = -1.,
            2 => s.updated_at = now + 300001,
            3 => s.id = "bad".into(),
            4 => s.version = 0,
            5 => s.days.values_mut().next().unwrap().last_at = now,
            _ => s.days.values_mut().next().unwrap().seconds = -1.,
        }
        assert!(!valid(&s, now), "case {change}");
    }
    b.sessions = vec![original; 33];
    assert!(owner(&b, "core", "user").is_err());
}
