//! Shared SQL predicates and stable ordering. All filtering happens before pagination.
use crate::error::{ApiError, ApiResult};
use rusqlite::types::Value as Sql;
use serde::Deserialize;

pub const KNOWN_ALBUM: &str = "(trim(t.album)!='' AND (lower(trim(t.album)) NOT IN ('none','null','unknown','unknown album','未知专辑','未提供专辑信息') OR COALESCE(NULLIF(json_extract(t.provider_data,'$.album_mid'),''),NULLIF(json_extract(t.provider_data,'$.release_id'),''),'')!=''))";
pub fn known_album(value:&str)->bool {!["","none","null","unknown","unknown album","未知专辑","未提供专辑信息"].contains(&value.trim().to_lowercase().as_str())}
pub const RELEASE: &str = "COALESCE(NULLIF(json_extract(t.provider_data,'$.release_date'),''),NULLIF(json_extract(t.provider_data,'$.provider_metadata.background.release_date'),''),NULLIF(json_extract(t.provider_data,'$.qqmusic_metadata.background.release_date'),''),'')";
pub const PLAYS: &str = "COALESCE((SELECT SUM(st.play_count) FROM track_stats st WHERE st.track_id=t.id OR st.track_id=json_extract(t.provider_data,'$.cached_from.track_id')),0)";
#[derive(Default, Deserialize)]
pub struct Filters {
    pub sort: Option<String>,
    pub album_status: Option<String>,
    pub format: Option<String>,
    pub listening: Option<String>,
    pub artwork: Option<String>,
    pub playlist: Option<String>,
    pub min_rate: Option<String>,
    pub min_depth: Option<String>,
    pub min_seconds: Option<String>,
    pub max_seconds: Option<String>,
    pub released_after: Option<String>,
    pub released_before: Option<String>,
}
pub fn sort(value: Option<&str>) -> ApiResult<&str> {
    let value = value.unwrap_or("default");
    if ![
        "",
        "default",
        "title",
        "title_desc",
        "artist",
        "album",
        "added",
        "release_new",
        "release_old",
        "duration",
        "duration_desc",
        "plays",
    ]
    .contains(&value)
    {
        return Err(ApiError::bad("排序方式无效"));
    }
    Ok(value)
}
pub fn date(value: &str) -> bool {
    value.len() == 10 && chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}
pub fn predicates(
    f: &Filters,
    user: &str,
    condition: &mut String,
    args: &mut Vec<Sql>,
) -> ApiResult<()> {
    sort(f.sort.as_deref())?;
    match f.album_status.as_deref().unwrap_or("") {""=>{},"known"=>condition.push_str(&format!(" AND {KNOWN_ALBUM}")),"unassigned"=>condition.push_str(&format!(" AND NOT ({KNOWN_ALBUM})")),_=>return Err(ApiError::bad("专辑资料筛选无效"))}
    if let Some(format) = f.format.as_deref().filter(|v| !v.is_empty()) {
        if ![
            "flac", "mp3", "aac", "alac", "wav", "aiff", "ogg", "opus", "dsf", "dff", "unknown",
        ]
        .contains(&format)
        {
            return Err(ApiError::bad("音频格式无效"));
        }
        condition.push_str(" AND lower(t.format)=?");
        args.push(format.to_owned().into());
    }
    match f.listening.as_deref().unwrap_or("") {
        "" => {}
        "unplayed" => condition.push_str(&format!(" AND {PLAYS}=0")),
        "played" => condition.push_str(&format!(" AND {PLAYS}>0")),
        _ => return Err(ApiError::bad("聆听筛选无效")),
    }
    match f.artwork.as_deref().unwrap_or("") {
        "" => {}
        "with" => condition.push_str(" AND COALESCE(t.cover_url,'')!=''"),
        "without" => condition.push_str(" AND COALESCE(t.cover_url,'')=''"),
        _ => return Err(ApiError::bad("封面筛选无效")),
    }
    match f.playlist.as_deref().unwrap_or("") {
        "" => {}
        value @ ("in" | "out") => {
            condition.push_str(if value == "out" {
                " AND NOT EXISTS("
            } else {
                " AND EXISTS("
            });
            condition.push_str("SELECT 1 FROM playlists pl,json_each(pl.tracks) entry WHERE pl.user_id=? AND (entry.value=t.id OR entry.value=json_extract(t.provider_data,'$.cached_from.track_id')))");
            args.push(user.to_owned().into());
        }
        _ => return Err(ApiError::bad("歌单筛选无效")),
    }
    let number = |v: &Option<String>| -> ApiResult<Option<u32>> {
        v.as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<u32>().map_err(|_| ApiError::bad("筛选数值无效")))
            .transpose()
    };
    let (min_rate, min_depth, min_seconds, max_seconds) = (
        number(&f.min_rate)?,
        number(&f.min_depth)?,
        number(&f.min_seconds)?,
        number(&f.max_seconds)?,
    );
    if min_rate.is_some_and(|n| n > 11289600)
        || min_depth.is_some_and(|n| n > 64)
        || min_seconds.is_some_and(|n| n > 86400)
        || max_seconds.is_some_and(|n| n > 86400)
        || min_seconds.zip(max_seconds).is_some_and(|(a, b)| a > b)
    {
        return Err(ApiError::bad("音频筛选范围无效"));
    }
    for (column, op, value) in [
        ("t.sample_rate", ">=", min_rate),
        ("t.bit_depth", ">=", min_depth),
        ("t.duration", ">=", min_seconds),
        ("t.duration", "<=", max_seconds),
    ] {
        if let Some(value) = value {
            condition.push_str(&format!(" AND {column}{op}?"));
            args.push(i64::from(value).into());
        }
    }
    if f.released_after
        .as_ref()
        .zip(f.released_before.as_ref())
        .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && a > b)
    {
        return Err(ApiError::bad("发行日期范围无效"));
    }
    for (op, value) in [(">=", &f.released_after), ("<=", &f.released_before)] {
        if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
            if !date(value) {
                return Err(ApiError::bad("发行日期须为 YYYY-MM-DD"));
            }
            condition.push_str(&format!(" AND {RELEASE}!='' AND {RELEASE}{op}?"));
            args.push(value.to_owned().into());
        }
    }
    Ok(())
}
pub fn order(value: Option<&str>, grouped: bool, default: &str) -> ApiResult<String> {
    let s = sort(value)?;
    let release = if grouped {
        format!("MAX({RELEASE})")
    } else {
        RELEASE.into()
    };
    let key = match s {
        "" | "default" => default.to_owned(),
        "title" => "t.title COLLATE NOCASE".into(),
        "title_desc" => "t.title COLLATE NOCASE DESC".into(),
        "artist" => {
            "t.artist COLLATE NOCASE,t.album COLLATE NOCASE,t.disc_number,t.track_number".into()
        }
        "album" => "t.album COLLATE NOCASE,t.disc_number,t.track_number".into(),
        "added" => if grouped {
            "MAX(t.created_at) DESC"
        } else {
            "t.created_at DESC"
        }
        .into(),
        "release_new" | "release_old" => format!(
            "({release}=''),{release} {}",
            if s == "release_old" { "ASC" } else { "DESC" }
        ),
        "duration" | "duration_desc" => format!(
            "{} {}",
            if grouped {
                "SUM(t.duration)"
            } else {
                "t.duration"
            },
            if s == "duration_desc" { "DESC" } else { "ASC" }
        ),
        "plays" => format!(
            "{} DESC",
            if grouped {
                format!("SUM({PLAYS})")
            } else {
                PLAYS.into()
            }
        ),
        _ => unreachable!(),
    };
    Ok(format!("{key},t.id"))
}
