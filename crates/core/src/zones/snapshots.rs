//! Small state snapshots; ordered queue pages are fetched only after a revision changes.
use super::*;
use crate::models::Track;

pub fn track(t: &Track) -> Value {
    let mut provider = serde_json::Map::new();
    for key in [
        "artist_credits",
        "release_id",
        "catalog_order",
        "source_audio",
        "quality_hint",
        "cached_from",
        "playback_selection",
        "artist_mids",
        "album_mid",
        "release_date",
    ] {
        if let Some(value) = t.provider_data.get(key) {
            provider.insert(key.into(), value.clone());
        }
    }
    json!({"id":t.id,"title":t.title,"artist":t.artist,"album":t.album,"album_artist":t.album_artist,
        "duration":t.duration,"track_number":t.track_number,"disc_number":t.disc_number,"source":t.source,
        "format":t.format,"sample_rate":t.sample_rate,"bit_depth":t.bit_depth,"channels":t.channels,
        "bitrate":t.bitrate,"cover_url":t.cover_url,"recording_id":t.recording_id,"available":t.available,
        "created_at":t.created_at,"integrated_lufs":t.integrated_lufs,
        "true_peak_dbfs":t.provider_data["analysis"]["loudness"]["true_peak_dbfs"].as_f64(),"provider_data":provider})
}
pub fn state(z: &Zone) -> Value {
    json!({"id":z.id,"name":z.name,"endpoint_id":z.endpoint_id,"status":z.status,"volume":z.volume,
        "muted":z.muted,"repeat":z.repeat,"shuffle":z.shuffle,"position":z.position,"playback_id":z.playback_id,
        "track":z.track.as_ref().map(track),"queue":[],"queue_length":z.queue.len(),"queue_revision":z.queue_revision,
        "queue_index":z.queue_index,"updated_at":z.updated_at,"dsp":z.dsp,"autoplay":z.autoplay,"transitions":z.transitions})
}
pub async fn list(s: &AppState) -> Value {
    let zones = s.zones.read().await;
    let mut rows = zones.values().map(state).collect::<Vec<_>>();
    rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    json!({"items":rows,"queue_view":"versioned","queue_epoch":*s.queue_epoch})
}
#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    offset: usize,
    #[serde(default = "page_limit")]
    limit: usize,
    revision: Option<u64>,
    epoch: Option<String>,
}
fn page_limit() -> usize {
    200
}
pub async fn queue(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Query(q): Query<Page>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    crate::accounts::scope::check(&s, &a, "zone", &id).await?;
    let page = |z: &Zone| {
        if q.epoch.as_ref().is_some_and(|epoch| epoch != s.queue_epoch.as_ref())
            || q.revision.is_some_and(|revision| revision != z.queue_revision)
        {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "queue_revision_changed".into(),
                "队列已更新，请重新加载".into(),
            ));
        }
        let offset = q.offset.min(z.queue.len());
        let end = (offset + q.limit.clamp(1, 500)).min(z.queue.len());
        Ok(Json(json!({
            "id":id,"epoch":*s.queue_epoch,"revision":z.queue_revision,"offset":offset,"total":z.queue.len(),
            "items":z.queue[offset..end].iter().map(track).collect::<Vec<_>>()
        })))
    };
    if let Some(key) = crate::portable::control_id(&s, &a, &id).await {
        crate::portable::with_zone(&s, &a, &key, page).await
    } else {
        let zones = s.zones.read().await;
        page(zones.get(&id).ok_or_else(ApiError::missing)?)
    }
}
