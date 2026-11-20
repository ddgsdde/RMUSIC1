//! Presentation metadata and bounded album-wide sorting; never pulls audio files.
use crate::{
    error::{ApiError, ApiResult},
    models::Track,
    state::AppState,
};
use serde_json::{json, Value};
static SORT_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
pub fn release_date(t: &Track) -> &str {
    for value in [
        &t.provider_data["release_date"],
        &t.provider_data["provider_metadata"]["background"]["release_date"],
        &t.provider_data["qqmusic_metadata"]["background"]["release_date"],
    ] {
        if let Some(v) = value.as_str().filter(|v| crate::library_query::date(v)) {
            return v;
        }
    }
    ""
}
pub fn decorate(c: &rusqlite::Connection, items: &mut Value) -> anyhow::Result<()> {
    if let Some(items) = items.as_array_mut() {
        for value in items {
            let t: Track = serde_json::from_value(value.clone())?;
            let date = release_date(&t);
            let today = chrono::Utc::now().date_naive();
            let is_new = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .ok()
                .is_some_and(|date| {
                    let days = (today - date).num_days();
                    (0..30).contains(&days)
                });
            let plays=c.query_row(&crate::accounts::scope::stats_sql("SELECT COALESCE(SUM(play_count),0) FROM track_stats WHERE track_id=?1 OR track_id=?2"),rusqlite::params![t.id,t.provider_data["cached_from"]["track_id"].as_str().unwrap_or("")],|r|r.get::<_,i64>(0))?;
            if !value["provider_data"].is_object() {
                value["provider_data"] = json!({});
            }
            value["provider_data"]["catalog_display"] =
                json!({"release_date":date,"is_new":is_new,"play_count":plays});
        }
    }
    Ok(())
}
pub async fn album(
    s: &AppState,
    seed: Track,
    provider: &str,
    q: crate::related_catalog::Options,
) -> ApiResult<Value> {
    let sort = q.sort.clone().unwrap_or_default();
    if ![
        "title",
        "title_desc",
        "duration",
        "duration_desc",
        "release_new",
        "release_old",
    ]
    .contains(&sort.as_str())
    {
        return Err(ApiError::bad("平台专辑不支持此排序"));
    }
    let target = q.page.unwrap_or(1);
    // Serialize shared preparation. Each source page retains the existing persistent cache.
    let key = format!(
        "album-order:{provider}:{}",
        seed.provider_data["album_mid"]
            .as_str()
            .or_else(|| seed.provider_data["release_id"].as_str())
            .unwrap_or(&seed.id)
    );
    let gate = crate::media_assets::gate(s, &key).await;
    let _guard = gate.lock().await;
    let _slot=tokio::time::timeout(std::time::Duration::from_secs(4),SORT_SLOTS.acquire()).await.map_err(|_|ApiError(axum::http::StatusCode::SERVICE_UNAVAILABLE,"catalog_busy".into(),"专辑排序繁忙，请稍后重试".into()))?.map_err(|_|ApiError::bad("专辑排序已停止"))?;
    let mut metadata_bytes=0usize;
    let mut rows = Vec::<Track>::new();
    let mut next = Some(1);
    let mut first = None;
    while let Some(page) = next {
        if page > 20 {
            return Err(ApiError::bad(
                "此专辑超过600首，请使用平台顺序浏览；未返回不完整排序",
            ));
        }
        let options = crate::related_catalog::Options {
            kind: Some("album".into()),
            page: Some(page),
            refresh: q.refresh,
            skip_profile: q.skip_profile,
            ..Default::default()
        };
        let mut value =
            crate::related_catalog::browse_provider_page(s, seed.clone(), provider, options)
                .await?;
        metadata_bytes=metadata_bytes.saturating_add(value["items"].to_string().len());
        if metadata_bytes>16*1024*1024 {return Err(ApiError::bad("专辑资料超过16 MiB，请使用平台顺序浏览"));}
        let items: Vec<Track> =
            serde_json::from_value(value["items"].take()).map_err(anyhow::Error::from)?;
        rows.extend(items);
        if rows.len() > 600 {
            return Err(ApiError::bad("此专辑超过600首，请使用平台顺序浏览"));
        }
        next = value["next_page"].as_u64().map(|n| n as u32);
        if first.is_none() {
            first = Some(value);
        }
    }
    rows.sort_by(|a, b| {
        let cmp = match sort.as_str() {
            "title" | "title_desc" => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            "duration" | "duration_desc" => a.duration.total_cmp(&b.duration),
            _ => {
                let (a, b) = (release_date(a), release_date(b));
                return a.is_empty().cmp(&b.is_empty()).then_with(|| {
                    if sort == "release_new" {
                        b.cmp(a)
                    } else {
                        a.cmp(b)
                    }
                });
            }
        };
        (if sort.ends_with("desc") {
            cmp.reverse()
        } else {
            cmp
        })
        .then_with(|| a.id.cmp(&b.id))
    });
    let total = rows.len();
    let mut value = first.unwrap_or(json!({}));
    value["items"] = json!(rows
        .into_iter()
        .skip((target as usize - 1) * 30)
        .take(30)
        .collect::<Vec<_>>());
    value["total"] = json!(total);
    value["page"] = json!(target);
    value["next_page"] = if target as usize * 30 < total {
        json!(target + 1)
    } else {
        Value::Null
    };
    value["sort"] = json!(sort);
    Ok(value)
}
