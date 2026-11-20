//! Bounded search presentation. Identity comes from provider credits, never fuzzy merging.
use crate::models::Track;
use serde_json::{json, Value};
use std::collections::BTreeMap;
fn score(query: &str, value: &str) -> u32 {
    let value = value.trim().to_lowercase();
    if value == query {
        100
    } else if value.starts_with(query) {
        75
    } else if value.contains(query) {
        50
    } else {
        0
    }
}
fn track_relevance(query: &str, t: &Track) -> u32 {
    let title = score(query, &t.title);
    let artist = score(query, &t.artist);
    let album = score(query, &t.album);
    if title == 100 {
        1000
    } else if artist == 100 {
        900
    } else if album == 100 {
        850
    } else {
        title * 3 + artist * 2 + album
    }
}
pub fn present(query: &str, mut tracks: Vec<Track>, notice: Option<&str>) -> Value {
    let query = query.trim().to_lowercase();
    tracks.sort_by_cached_key(|t| std::cmp::Reverse(track_relevance(&query, t)));
    let mut albums = BTreeMap::new();
    let mut artists = BTreeMap::new();
    for t in &tracks {
        let mut anchor = t.clone();
        // Cards share identity with the stored track; omit descriptions/lyrics from repeated payloads.
        anchor.provider_data = json!({});
        for key in [
            "artist_credits",
            "artist_mids",
            "album_mid",
            "release_id",
            "cached_from",
            "quality_hint",
        ] {
            if let Some(value) = t.provider_data.get(key) {
                anchor.provider_data[key] = value.clone();
            }
        }

        if !t.album.trim().is_empty() {
            let id = crate::artist_credits::release_key(&t.source, &t.provider_data)
                .unwrap_or_else(|| {
                    crate::auth::hash(&format!("{}\0{}\0{}", t.source, t.album_artist, t.album))
                });
            albums.entry(id.clone()).or_insert_with(||json!({"id":id,"title":t.album,"artist":t.album_artist,"source":t.source,"cover_url":t.cover_url,"track":anchor,"relevance":score(&query,&t.album)*3+score(&query,&t.album_artist)}));
        }
        let credits = crate::artist_credits::credits(&t.source, &t.artist, &t.provider_data);
        // Unresolved combined captions are songs, not a new "combined artist" entity.
        if credits.len() == 1
            && t.artist.contains(" / ")
            && !t.provider_data["artist_credits"].is_array()
        {
            continue;
        }
        for credit in credits {
            // A song cover is not an artist portrait. Keep the portrait absent unless explicitly supplied.
            let portrait = t.provider_data["artist_credits"]
                .as_array()
                .and_then(|v| v.iter().find(|v| v["name"] == credit.name))
                .and_then(|v| v["image_url"].as_str())
                .or_else(|| {
                    ["provider_metadata", "qqmusic_metadata"]
                        .into_iter()
                        .filter_map(|key| t.provider_data[key]["artists"].as_array())
                        .flatten()
                        .find(|v| v["name"].as_str() == Some(&credit.name))
                        .and_then(|v| v["image_url"].as_str())
                        .filter(|v| v.starts_with("https://"))
                });
            let item=artists.entry(credit.id.clone()).or_insert_with(||json!({"strong_identity":credit.id!=crate::auth::hash(&credit.name),"image_url":portrait,"id":credit.id,"name":credit.name,"source":t.source,"track":anchor,"relevance":score(&query,&credit.name)*3}));
            if item["image_url"].is_null() {
                if let Some(url) = portrait {
                    item["image_url"] = json!(url);
                }
            }
        }
    }
    let strong = artists
        .values()
        .filter(|v| v["strong_identity"] == true)
        .map(|v| (v["source"].clone(), v["name"].clone()))
        .collect::<Vec<_>>();
    artists.retain(|_, v| {
        v["strong_identity"] == true
            || !strong
                .iter()
                .any(|(s, n)| s == &v["source"] && n == &v["name"])
    });
    let sort = |map: BTreeMap<String, Value>| {
        let mut rows: Vec<_> = map.into_values().collect();
        rows.sort_by_key(|v| std::cmp::Reverse(v["relevance"].as_u64().unwrap_or(0)));
        rows
    };
    json!({"total":tracks.len(),"items":tracks,"albums":sort(albums),"artists":sort(artists),"notice":notice,"source":"mixed","bounded":true,"catalog_scope":"matched_tracks"})
}
pub async fn artwork(
    s: &crate::state::AppState,
    mut value: Value,
) -> crate::error::ApiResult<Value> {
    Ok(s.db.run(move|c|{for kind in ["artists","albums"]{if let Some(items)=value[kind].as_array_mut(){for item in items{
  use rusqlite::OptionalExtension;
  let profile=c.query_row("SELECT automatic,manual FROM catalog_profiles WHERE kind=?1 AND entity_id=?2",rusqlite::params![if kind=="artists"{"artist"}else{"album"},item["id"].as_str().unwrap_or("")],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
  if let Some((automatic,manual))=profile {let auto:Value=serde_json::from_str(&automatic)?;let manual:Value=serde_json::from_str(&manual)?;if let Some(url)=manual.get("image_url").or_else(||auto.get("image_url")).and_then(Value::as_str){item["image_url"]=json!(url);}}
  if let Some(url)=item["image_url"].as_str(){if let Ok(local)=crate::media_assets::register(c,url){item["image_url"]=json!(local);}}
 }}}Ok(value)}).await?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relevance_prefers_exact_then_prefix_and_does_not_fold_editions() {
        assert!(score("iu", "IU") > score("iu", "IU live"));
        assert!(score("iu", "IU live") > score("iu", "with IU"));
        assert_eq!(score("iu", "Crush"), 0);
        let t = |id: &str, artist: &str, album: &str| Track {
            id: id.into(),
            title: id.into(),
            artist: artist.into(),
            album: album.into(),
            album_artist: artist.into(),
            source: "qqmusic".into(),
            provider_data: json!({"artist_mids":["crush","iu"]}),
            duration: 0.,
            track_number: 0,
            disc_number: 0,
            format: "unknown".into(),
            sample_rate: 0,
            bit_depth: 0,
            channels: 0,
            bitrate: 0,
            cover_url: None,
            recording_id: String::new(),
            available: true,
            created_at: 0,
            integrated_lufs: None,
        };
        let result = present(
            "IU",
            vec![
                t("one", "Crush / IU", "Live"),
                t("two", "Crush / IU", "Studio"),
            ],
            None,
        );
        assert_eq!(result["artists"].as_array().unwrap().len(), 2);
        assert_eq!(result["artists"][0]["name"], "IU");
        assert_eq!(result["albums"].as_array().unwrap().len(), 2);
        assert!(result["artists"][0].get("popularity").is_none());
        let solo = t("Solo", "IU", "Release");
        let mut feature = t("Other", "Other / IU", "Release");
        feature.title = "feat. IU".into();
        assert!(track_relevance("iu", &solo) > track_relevance("iu", &feature));
    }
}
