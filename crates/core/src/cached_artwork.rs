//! Owned audio copies inherit artwork from their exact source, without a detail-page fetch.
use crate::models::Track;
use rusqlite::{params, Connection, OptionalExtension};

/// Recover a provider song image already resolved for an exact owned copy.
pub fn restore_source(c:&Connection,track:&mut Track)->anyhow::Result<()> {
    if track.source=="local" || track.cover_url.as_ref().is_some_and(|v|!v.is_empty()) {return Ok(());}
    let locked=c.query_row("SELECT EXISTS(SELECT 1 FROM catalog_profiles WHERE kind='track' AND entity_id=?1 AND json_type(manual,'$.image_url') IS NOT NULL)",[&track.id],|r|r.get::<_,bool>(0))?;
    if locked{return Ok(());}
    let url=c.query_row("SELECT json_extract(provider_data,'$.provider_metadata.track.cover_url') FROM tracks WHERE recording_id=?3 AND source='local' AND json_extract(provider_data,'$.cached_from.track_id')=?1 AND json_extract(provider_data,'$.cached_from.source')=?2 AND json_extract(provider_data,'$.provider_metadata.matched')=1 AND json_extract(provider_data,'$.provider_metadata.track.cover_url') LIKE 'https://%' ORDER BY rowid DESC LIMIT 1",params![track.id,track.source,track.recording_id],|r|r.get::<_,String>(0)).optional()?;
    if let Some(url)=url {
        if let Ok(cover)=crate::media_assets::register(c,&url){
            c.execute("UPDATE tracks SET cover_url=?2 WHERE id=?1 AND COALESCE(cover_url,'')=''",params![track.id,cover])?;
            crate::artwork_recovery::bind(c,"track",&track.id,&cover)?;
            track.cover_url=Some(cover);
        }
    }
    Ok(())
}

pub fn inherited(c: &Connection, track: &Track) -> anyhow::Result<Option<(Option<String>, bool)>> {
    if track.source != "local" || track.recording_id.is_empty() {
        return Ok(None);
    }
    let lineage = &track.provider_data["cached_from"];
    let (Some(id), Some(source)) = (lineage["track_id"].as_str(), lineage["source"].as_str())
    else {
        return Ok(None);
    };
    if source == "local" || source.is_empty() {
        return Ok(None);
    }
    let origin = c
        .query_row(
            "SELECT * FROM tracks WHERE id=?1 AND source=?2 AND recording_id=?3",
            params![id, source, track.recording_id],
            Track::from_row,
        )
        .optional()?;
    let Some(origin) = origin else {
        return Ok(None);
    };
    if crate::artist_credits::release_key(&track.source, &track.provider_data)
        != crate::artist_credits::release_key(&origin.source, &origin.provider_data)
    {
        return Ok(None);
    }
    let manual = c.query_row("SELECT EXISTS(SELECT 1 FROM catalog_profiles WHERE kind='track' AND entity_id=?1 AND json_type(manual,'$.image_url')='text')", [id], |r|r.get::<_,bool>(0))?;
    let cover = origin
        .cover_url
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|url| crate::media_assets::register(c, url))
        .transpose()?;
    Ok((cover.is_some() || manual).then_some((cover, manual)))
}
fn repair(c: &Connection, track: &Track) -> anyhow::Result<()> {
    let locked = c.query_row("SELECT EXISTS(SELECT 1 FROM catalog_profiles WHERE kind='track' AND entity_id=?1 AND json_type(manual,'$.image_url') IS NOT NULL)", [&track.id], |r|r.get::<_,bool>(0))?;
    if locked {
        return Ok(());
    }
    let mut data = track.provider_data.clone();
    let cover = crate::album_artwork::map(c, track, track.cover_url.clone(), &mut data, false)?;
    if cover != track.cover_url || data != track.provider_data {
        c.execute(
            "UPDATE tracks SET cover_url=?1,provider_data=?2 WHERE id=?3",
            params![cover, data.to_string(), track.id],
        )?;
        if let Some(url) = cover {
            crate::artwork_recovery::bind(c, "track", &track.id, &url)?;
        }
    }
    Ok(())
}
pub fn refresh(c: &Connection, origin: &Track) -> anyhow::Result<()> {
    if origin.source == "local" || origin.recording_id.is_empty() {
        return Ok(());
    }
    // tracks_recording bounds this lookup to the recording, not the whole library.
    let mut after = 0i64;
    loop {
        let rows=c.prepare("SELECT rowid,t.* FROM tracks t WHERE rowid>?4 AND recording_id=?1 AND source='local' AND json_extract(provider_data,'$.cached_from.track_id')=?2 AND json_extract(provider_data,'$.cached_from.source')=?3 ORDER BY rowid LIMIT 128")?.query_map(params![origin.recording_id,origin.id,origin.source,after],|r|Ok((r.get::<_,i64>(0)?,Track::from_row(r)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            break;
        }
        for (row, track) in rows {
            after = row;
            repair(c, &track)?;
        }
    }
    Ok(())
}
pub fn migrate(c: &Connection) -> anyhow::Result<()> {
    if c.query_row(
        "SELECT EXISTS(SELECT 1 FROM settings WHERE key='cached_artwork_lineage_v1')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let tx = c.unchecked_transaction()?;
    let mut after = 0i64;
    loop {
        let rows=tx.prepare("SELECT rowid,t.* FROM tracks t WHERE source='local' AND rowid>?1 AND json_type(provider_data,'$.cached_from.track_id')='text' ORDER BY rowid LIMIT 128")?.query_map([after],|r|Ok((r.get::<_,i64>(0)?,Track::from_row(r)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            break;
        }
        for (row, track) in rows {
            after = row;
            repair(&tx, &track)?;
        }
    }
    tx.execute(
        "INSERT INTO settings(key,value)VALUES('cached_artwork_lineage_v1','true')",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn source(id: &str, release: &str) -> Track {
        Track {
            id: id.into(),
            title: "Same song".into(),
            artist: "Artist".into(),
            album: "Same album".into(),
            album_artist: "Artist".into(),
            duration: 1.,
            track_number: 1,
            disc_number: 1,
            source: "qa".into(),
            format: "flac".into(),
            sample_rate: 44100,
            bit_depth: 16,
            channels: 2,
            bitrate: 0,
            cover_url: Some("https://art.example/source.jpg".into()),
            recording_id: id.into(),
            available: true,
            created_at: 1,
            integrated_lufs: None,
            provider_data: json!({"release_id":release}),
        }
    }
    fn copy(origin: &Track, id: &str) -> Track {
        let mut t = origin.clone();
        t.id = id.into();
        t.source = "local".into();
        t.cover_url = None;
        t.provider_data["cached_from"] =
            json!({"source":origin.source,"track_id":origin.id,"cache_id":id});
        t
    }
    fn save(c: &Connection, t: &Track) {
        crate::library::upsert(c, t, None, None, 0, 0, None, None).unwrap();
    }
    fn cover(c: &Connection, id: &str) -> Option<String> {
        c.query_row("SELECT cover_url FROM tracks WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    }
    #[test]
    fn cached_artwork_follows_exact_origin_on_creation_and_late_source_updates() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Db::open(&dir.path().join("db")).unwrap();
        let c = db.0.lock().unwrap();
        let mut origin = source("qa:song", "edition");
        let cached = copy(&origin, "copy");
        save(&c, &cached);
        assert_eq!(cover(&c, "copy"), None);
        save(&c, &origin);
        assert_eq!(cover(&c, "copy"), cover(&c, "qa:song"));
        origin.cover_url = Some("https://art.example/updated.jpg".into());
        save(&c, &origin);
        assert_eq!(cover(&c, "copy"), cover(&c, "qa:song"));
        c.execute("UPDATE tracks SET cover_url=NULL WHERE id='copy'", [])
            .unwrap();
        c.execute(
            "DELETE FROM settings WHERE key='cached_artwork_lineage_v1'",
            [],
        )
        .unwrap();
        migrate(&c).unwrap();
        assert_eq!(cover(&c, "copy"), cover(&c, "qa:song"));
        migrate(&c).unwrap();
    }
    #[test]
    fn cached_artwork_never_crosses_recordings_releases_or_manual_locks() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Db::open(&dir.path().join("db")).unwrap();
        let c = db.0.lock().unwrap();
        let origin = source("qa:song", "edition");
        save(&c, &origin);
        let mut wrong = copy(&origin, "wrong-recording");
        wrong.recording_id = "different".into();
        save(&c, &wrong);
        assert_eq!(cover(&c, &wrong.id), None);
        let mut other = copy(&origin, "other-edition");
        other.provider_data["release_id"] = json!("deluxe");
        save(&c, &other);
        assert_eq!(cover(&c, &other.id), None);
        let owned = copy(&origin, "owned");
        save(&c, &owned);
        c.execute("INSERT INTO catalog_profiles(kind,entity_id,manual)VALUES('track','owned','{\"image_url\":\"\"}')",[]).unwrap();
        save(&c, &owned);
        assert_eq!(cover(&c, &owned.id), None);
        let owned2 = copy(&origin, "owned2");
        save(&c, &owned2);
        c.execute("INSERT INTO catalog_profiles(kind,entity_id,manual)VALUES('track','qa:song','{\"image_url\":\"\"}')",[]).unwrap();
        save(&c, &origin);
        assert_eq!(cover(&c, "owned2"), None);
        assert_eq!(cover(&c, "owned"), None);
    }
}
