//! One artwork reference per confirmed provider release; captions never establish identity.
use crate::models::Track;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub fn default_cover(c: &rusqlite::Connection, t: &Track) -> anyhow::Result<Option<(String, Option<String>)>> {
    let Some(key) = crate::artist_credits::release_key(&t.source, &t.provider_data) else { return Ok(None) };
    let profile = c.query_row("SELECT automatic,manual FROM catalog_profiles WHERE kind='album' AND entity_id=?1", [&key], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
    let Some((automatic, manual)) = profile else { return Ok(None) };
    let automatic: Value = serde_json::from_str(&automatic)?;
    let manual: Value = serde_json::from_str(&manual)?;
    let image = manual.get("image_url").or_else(|| automatic.get("image_url").filter(|v|v.as_str().is_some_and(|s|!s.is_empty())));
    let Some(image) = image else { return Ok(None) };
    let cover = image.as_str().filter(|s| !s.is_empty()).map(|url| crate::media_assets::register(c, url)).transpose()?;
    Ok(Some((key, cover)))
}

pub fn map(c: &rusqlite::Connection, t: &Track, specific: Option<String>, data: &mut Value, locked: bool) -> anyhow::Result<Option<String>> {
    if locked {
        if let Some(data) = data.as_object_mut() { data.remove("artwork_album"); }
        return Ok(specific);
    }
    let old = &t.provider_data["artwork_album"];
    let specific = if specific.as_deref() == old["url"].as_str() && old["id"].is_string() {
        old["fallback"].as_str().map(str::to_owned)
    } else { specific };
    let mut specific = specific;
    if let Some((cover, inherited_lock)) = crate::cached_artwork::inherited(c,t)? {
        if inherited_lock {
            if let Some(data)=data.as_object_mut(){data.remove("artwork_album");}
            return Ok(cover);
        }
        specific = cover;
    }
    if !locked {
        if let Some((key, cover)) = default_cover(c, t)? {
            data["artwork_album"] = json!({"id":key,"url":cover,"fallback":specific});
            return Ok(cover);
        }
    }
    if let Some(data) = data.as_object_mut() { data.remove("artwork_album"); }
    Ok(specific)
}

/// Refresh only this exact release and its owned cache copies, in bounded batches.
pub fn refresh(c: &rusqlite::Connection, key: &str) -> anyhow::Result<usize> {
    let Some((provider, release)) = key.strip_prefix("release:").and_then(|v|v.split_once(':')) else { return Ok(0) };
    let mut after = 0i64;
    let mut changed = 0;
    loop {
        let rows = c.prepare("SELECT rowid,t.* FROM tracks t WHERE rowid>?1 AND COALESCE(json_extract(provider_data,'$.cached_from.source'),source)=?2 AND COALESCE(json_extract(provider_data,'$.release_id'),json_extract(provider_data,'$.album_mid'))=?3 ORDER BY rowid LIMIT 128")?.query_map(params![after,provider,release], |r|Ok((r.get::<_,i64>(0)?,Track::from_row(r)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() { break }
        for (row, track) in rows {
            after = row;
            let locked = c.query_row("SELECT EXISTS(SELECT 1 FROM catalog_profiles WHERE kind='track' AND entity_id=?1 AND json_type(manual,'$.image_url') IS NOT NULL)", [&track.id], |r|r.get::<_,bool>(0))?;
            if locked { continue }
            let mut data = track.provider_data.clone();
            let cover = map(c, &track, track.cover_url.clone(), &mut data, false)?;
            if cover != track.cover_url || data != track.provider_data {
                c.execute("UPDATE tracks SET cover_url=?1,provider_data=?2 WHERE id=?3", params![cover,data.to_string(),track.id])?;
                if let Some(url) = cover.as_deref() { crate::artwork_recovery::bind(c,"track",&track.id,url)?; }
                changed += 1;
            }
        }
    }
    Ok(changed)
}

pub fn migrate(c: &rusqlite::Connection) -> anyhow::Result<()> {
    if c.query_row("SELECT EXISTS(SELECT 1 FROM settings WHERE key='release_artwork_v1')",[],|r|r.get::<_,bool>(0))? { return Ok(()) }
    let tx = c.unchecked_transaction()?;
    let keys = tx.prepare("SELECT entity_id FROM catalog_profiles WHERE kind='album' AND entity_id LIKE 'release:%'")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for key in keys { refresh(&tx,&key)?; }
    tx.execute("INSERT INTO settings(key,value)VALUES('release_artwork_v1','true')",[])?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    fn track(id:&str, release:&str, cover:Option<&str>) -> Track {
        Track {id:id.into(),title:"Same caption".into(),artist:"Artist".into(),album:"Same album caption".into(),album_artist:"Artist".into(),duration:10.,track_number:1,disc_number:1,source:"qa".into(),format:"unknown".into(),sample_rate:0,bit_depth:0,channels:2,bitrate:0,cover_url:cover.map(str::to_owned),recording_id:id.into(),available:true,created_at:0,integrated_lufs:None,provider_data:json!({"release_id":release})}
    }
    #[test] fn release_artwork_is_shared_but_same_named_editions_and_manual_images_stay_independent() {
        let dir=tempfile::tempdir().unwrap();let db=crate::db::Db::open(&dir.path().join("test.db")).unwrap();let c=db.0.lock().unwrap();
        let original="https://art.example/original.jpg";let deluxe="https://art.example/deluxe.jpg";
        for (key,url) in [("release:qa:original",original),("release:qa:deluxe",deluxe)] {
            c.execute("INSERT INTO catalog_profiles(kind,entity_id,automatic)VALUES('album',?1,?2)",params![key,json!({"image_url":url}).to_string()]).unwrap();
        }
        for t in [track("one","original",None),track("two","original",Some("https://art.example/song.jpg")),track("three","deluxe",None)] {
            crate::library::upsert(&c,&t,None,None,0,0,None,None).unwrap();
        }
        let cover=|id:&str|c.query_row("SELECT cover_url FROM tracks WHERE id=?1",[id],|r|r.get::<_,Option<String>>(0)).unwrap();
        assert_eq!(cover("one"),cover("two"));assert_ne!(cover("one"),cover("three"));
        assert_eq!(cover("one"),Some(crate::media_assets::register(&c,original).unwrap()));
        c.execute("INSERT INTO catalog_profiles(kind,entity_id,manual)VALUES('track','two','{\"image_url\":\"\"}')",[]).unwrap();
        let two=track("two","original",None);crate::library::upsert(&c,&two,None,None,0,0,None,None).unwrap();
        c.execute("UPDATE catalog_profiles SET automatic=?1 WHERE entity_id='release:qa:original'",[json!({"image_url":"https://art.example/updated.jpg"}).to_string()]).unwrap();
        assert_eq!(refresh(&c,"release:qa:original").unwrap(),1);assert_eq!(cover("two"),None);
        assert_eq!(cover("three"),Some(crate::media_assets::register(&c,deluxe).unwrap()));
        assert_eq!(refresh(&c,"release:qa:original").unwrap(),0);
    }
    #[test] fn cached_copy_follows_its_exact_provider_release_and_unassigned_song_keeps_its_image() {
        let dir=tempfile::tempdir().unwrap();let db=crate::db::Db::open(&dir.path().join("test.db")).unwrap();let c=db.0.lock().unwrap();
        let mut copy=track("copy","release",None);copy.source="local".into();copy.provider_data["cached_from"]=json!({"source":"qa","track_id":"qa:song"});
        crate::library::upsert(&c,&copy,None,None,0,0,None,None).unwrap();
        c.execute("INSERT INTO catalog_profiles(kind,entity_id,automatic)VALUES('album','release:qa:release',?1)",[json!({"image_url":"https://art.example/album.jpg"}).to_string()]).unwrap();
        assert_eq!(refresh(&c,"release:qa:release").unwrap(),1);
        let mut single=track("single","",Some("https://art.example/single.jpg"));single.provider_data=json!({});
        let mut data=single.provider_data.clone();assert_eq!(map(&c,&single,single.cover_url.clone(),&mut data,false).unwrap(),single.cover_url);
        c.execute("UPDATE catalog_profiles SET automatic='{}' WHERE entity_id='release:qa:release'",[]).unwrap();
        copy.cover_url=Some("https://art.example/song-only.jpg".into());let mut data=copy.provider_data.clone();
        assert_eq!(map(&c,&copy,copy.cover_url.clone(),&mut data,false).unwrap(),copy.cover_url);
    }
}
