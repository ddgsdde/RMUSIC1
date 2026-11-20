//! Interchange is metadata only: bounded preview, exact identity matching, explicit selection.
//! No imported URL is fetched and no provider playlist is mutated.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    models::Track,
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Entry {
    id: Option<String>,
    source: Option<String>,
    location: Option<String>,
    title: String,
    artist: String,
    album: String,
    duration: Option<f64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    format: String,
    content: String,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    request_id: Option<String>,
    name: String,
    track_ids: Vec<String>,
}
fn csv_rows(input: &str) -> ApiResult<Vec<Vec<String>>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' if quoted => quoted = false,
            '"' if field.is_empty() => quoted = true,
            ',' if !quoted => row.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !quoted => {}
            _ => field.push(ch),
        }
        if rows.len() > 10001 {
            return Err(ApiError::bad("最多导入一万首歌曲"));
        }
    }
    if quoted {
        return Err(ApiError::bad("CSV 引号未闭合"));
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}
fn parse(input: Input) -> ApiResult<Vec<Entry>> {
    if input.content.len() > 2 * 1024 * 1024 {
        return Err(ApiError::bad("播放列表文件不能超过 2 MiB"));
    }
    let content = input.content.trim_start_matches('\u{feff}');
    let entries = match input.format.to_lowercase().as_str() {
        "json" => {
            let value: Value = serde_json::from_str(content)
                .map_err(|_| ApiError::bad("JSON 播放列表格式无效"))?;
            if value["format"] != "rmusic-playlist" || value["version"] != 1 {
                return Err(ApiError::bad("不支持的 R-Music 播放列表版本"));
            }
            serde_json::from_value(value["tracks"].clone())
                .map_err(|_| ApiError::bad("播放列表曲目字段无效"))?
        }
        "m3u" | "m3u8" => {
            let mut items = vec![];
            let mut next = Entry::default();
            for line in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if let Some(info) = line.strip_prefix("#EXTINF:") {
                    if let Some((duration, label)) = info.split_once(',') {
                        next.duration = duration.parse().ok().filter(|d: &f64| *d > 0.);
                        if let Some((artist, title)) = label.split_once(" - ") {
                            next.artist = artist.into();
                            next.title = title.into();
                        } else {
                            next.title = label.into();
                        }
                    }
                } else if let Some(metadata) = line.strip_prefix("#RMUSIC:") {
                    next = serde_json::from_str(metadata)
                        .map_err(|_| ApiError::bad("M3U 身份信息无效"))?;
                } else if !line.starts_with('#') {
                    next.location = Some(line.into());
                    items.push(std::mem::take(&mut next));
                    if items.len() > 10000 {
                        return Err(ApiError::bad("最多导入一万首歌曲"));
                    }
                }
            }
            items
        }
        "pls" => {
            let mut rows = std::collections::BTreeMap::<usize, Entry>::new();
            for line in content.lines() {
                if let Some((key, value)) = line.trim().split_once('=') {
                    for prefix in ["File", "Title", "Length"] {
                        if let Some(index) = key
                            .strip_prefix(prefix)
                            .and_then(|v| v.parse::<usize>().ok())
                        {
                            if !(1..=10000).contains(&index) {
                                return Err(ApiError::bad("PLS 歌曲编号超出限制"));
                            }
                            let item = rows.entry(index).or_default();
                            match prefix {
                                "File" => item.location = Some(value.into()),
                                "Title" => {
                                    if let Some((artist, title)) = value.split_once(" - ") {
                                        item.artist = artist.into();
                                        item.title = title.into();
                                    } else {
                                        item.title = value.into();
                                    }
                                }
                                _ => item.duration = value.parse().ok().filter(|d: &f64| *d > 0.),
                            }
                        }
                    }
                }
            }
            rows.into_values()
                .filter(|e| e.location.is_some())
                .collect()
        }
        "csv" => {
            let mut rows = csv_rows(content)?.into_iter();
            let header = rows.next().ok_or_else(|| ApiError::bad("CSV 缺少表头"))?;
            let get = |row: &Vec<String>, key: &str| {
                header
                    .iter()
                    .position(|v| v.trim().eq_ignore_ascii_case(key))
                    .and_then(|i| row.get(i))
                    .cloned()
                    .unwrap_or_default()
            };
            if !header
                .iter()
                .any(|h| h.trim().eq_ignore_ascii_case("title"))
            {
                return Err(ApiError::bad(
                    "CSV 需要 title 表头，可附 artist、album、duration、id、source、location",
                ));
            }
            rows.filter(|r| r.iter().any(|v| !v.is_empty()))
                .map(|r| {
                    let optional = |key| {
                        let v = get(&r, key);
                        (!v.is_empty()).then_some(v)
                    };
                    Entry {
                        id: optional("id"),
                        source: optional("source"),
                        location: optional("location"),
                        title: get(&r, "title"),
                        artist: get(&r, "artist"),
                        album: get(&r, "album"),
                        duration: get(&r, "duration").parse().ok(),
                    }
                })
                .collect()
        }
        _ => return Err(ApiError::bad("支持 JSON、M3U／M3U8、PLS 和 CSV")),
    };
    if entries.is_empty() || entries.len() > 10000 {
        return Err(ApiError::bad("播放列表需要 1–10000 首歌曲"));
    }
    for entry in &entries {
        if [&entry.title, &entry.artist, &entry.album]
            .iter()
            .any(|v| v.len() > 2048 || v.contains(['\r', '\n', '\0']))
            || [&entry.id, &entry.source, &entry.location].iter().any(|v| {
                v.as_ref()
                    .is_some_and(|v| v.len() > 4096 || v.contains(['\r', '\n', '\0']))
            })
            || entry
                .duration
                .is_some_and(|v| !v.is_finite() || v < 0. || v > 604800.)
        {
            return Err(ApiError::bad("歌曲字段过长或无效"));
        }
    }
    Ok(entries)
}
fn summary(t: &Track) -> Value {
    json!({"id":t.id,"title":t.title,"artist":t.artist,"album":t.album,"duration":t.duration,"source":t.source,"available":t.available})
}
fn matching(c: &rusqlite::Connection, entry: Entry, core: &str) -> anyhow::Result<Value> {
    let permission=crate::accounts::scope::predicate("t");
    let mut id = entry.id.clone();
    let mut source = entry.source.clone();
    if let Some(location) = &entry.location {
        if let Ok(url) = url::Url::parse(location) {
            if url.scheme() == "rmusic" && url.host_str() == Some("track") {
                let parts = url
                    .path_segments()
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                if parts.len() == 2 {
                    source = Some(parts[0].into());
                    id = Some(parts[1].into());
                }
            }
        }
    }
    let exact = if let (Some(id), Some(source)) = (id, source) {
        c.query_row(
            &format!("SELECT t.* FROM tracks t WHERE t.id=?1 AND t.source=?2 AND {permission}"),
            params![id, source],
            Track::from_row,
        )
        .optional()?
    } else {
        None
    };
    let location = entry.location.as_ref().map(|s| {
        url::Url::parse(s)
            .ok()
            .filter(|u| u.scheme() == "file")
            .and_then(|u| u.to_file_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| s.clone())
    });
    let exact = if exact.is_some() {
        exact
    } else if let Some(path) = location {
        c.query_row(
            &format!("SELECT t.* FROM tracks t WHERE t.path=?1 AND t.source='local' AND {permission}"),
            [path],
            Track::from_row,
        )
        .optional()?
    } else {
        None
    };
    let candidates = if exact.is_none() && !entry.title.is_empty() && !entry.artist.is_empty() {
        c.prepare(&format!("SELECT t.* FROM tracks t WHERE title=?1 COLLATE NOCASE AND artist=?2 COLLATE NOCASE AND (?3='' OR album=?3 COLLATE NOCASE) AND (?4 IS NULL OR ABS(duration-?4)<=1.0) AND {permission} ORDER BY available DESC,id LIMIT 6"))?.query_map(params![entry.title,entry.artist,entry.album,entry.duration],Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        vec![]
    };
    Ok(
        json!({"entry":entry,"status":if exact.is_some(){"matched"}else if candidates.is_empty(){"missing"}else{"review"},"matched":exact.as_ref().map(summary),"candidates":candidates.iter().map(summary).collect::<Vec<_>>(),"core_id":core}),
    )
}
pub async fn preview(
    State(s): State<AppState>,
    a: Auth,
    Json(input): Json<Input>,
) -> ApiResult<Json<Value>> {
    a.require("control")?;
    let offset = input.offset;
    let limit = input.limit.unwrap_or(50).clamp(1, 100);
    let entries = parse(input)?;
    let total = entries.len();
    let core = s.core_id.clone();
    // Page preview to cap response size; client submits only reviewed IDs, never arbitrary metadata.
    let items =
        s.db.read(move |c| {
            entries
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|e| matching(c, e, &core))
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(
        json!({"items":items,"total":total,"offset":offset,"limit":limit,"automatic_download":false}),
    ))
}
pub async fn import(
    State(s): State<AppState>,
    a: Auth,
    Json(input): Json<Import>,
) -> ApiResult<Json<Value>> {
    a.require("control")?;
    if input.name.trim().is_empty()
        || input.name.len() > 100
        || input.track_ids.is_empty()
        || input.track_ids.len() > 10000
        || input.track_ids.iter().any(|s| s.len() > 512)
    {
        return Err(ApiError::bad("歌单名称或已确认曲目无效"));
    }
    let request = input
        .request_id
        .map(|id| {
            uuid::Uuid::parse_str(&id)
                .map(|v| v.to_string())
                .map_err(|_| ApiError::bad("导入请求编号须为 UUID"))
        })
        .transpose()?;
    let payload_hash = crate::auth::hash(
        &json!({"name":input.name.trim(),"track_ids":input.track_ids}).to_string(),
    );
    let result=s.db.run(move|c|{
        let tx=c.transaction()?;
        if let Some(request)=&request {
            if let Some((hash,id,count))=tx.query_row("SELECT payload_hash,playlist_id,track_count FROM playlist_import_receipts WHERE user_id=?1 AND request_id=?2",params![a.id,request],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,i64>(2)?))).optional()? {
                if hash!=payload_hash{return Ok(Err(ApiError(axum::http::StatusCode::CONFLICT,"playlist_import_conflict".into(),"此导入请求的内容已经改变，请重新预览".into())));}
                let Some(id)=id else{return Ok(Err(ApiError(axum::http::StatusCode::GONE,"playlist_import_deleted".into(),"此前导入的播放列表已被删除，请重新导入".into())));};
                return Ok(Ok(json!({"id":id,"count":count,"replayed":true})));
            }
        }
        for id in &input.track_ids {if !tx.query_row("SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1)",[id],|r|r.get::<_,bool>(0))?{return Ok(Err(ApiError::bad("部分曲目已被移除，请重新预览后导入")));}}
        let id=uuid::Uuid::new_v4().to_string();let count=input.track_ids.len();
        tx.execute("INSERT INTO playlists(id,name,user_id,tracks) VALUES(?1,?2,?3,?4)",params![id,input.name.trim(),a.id,serde_json::to_string(&input.track_ids)?])?;
        if let Some(request)=request{tx.execute("INSERT INTO playlist_import_receipts VALUES(?1,?2,?3,?4,?5,unixepoch())",params![a.id,request,payload_hash,id,count as i64])?;}
        tx.commit()?;Ok(Ok(json!({"id":id,"count":count,"replayed":false})))
    }).await??;
    if result["replayed"] != true {
        s.emit("playlists", json!({"id":result["id"]}));
    }
    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct Format {
    format: Option<String>,
}
fn clean(s: &str) -> String {
    s.replace(['\r', '\n', '\0'], " ")
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub async fn export(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Query(q): Query<Format>,
) -> ApiResult<Response> {
    a.require("read")?;
    let format = q.format.unwrap_or_else(|| "json".into());
    if !["json", "m3u8", "pls", "csv"].contains(&format.as_str()) {
        return Err(ApiError::bad("不支持的导出格式"));
    }
    let permission=crate::accounts::scope::predicate("t");let external=a.role=="external";
    let result =
        s.db.read(move |c| {
            let stored = c
                .query_row(
                    "SELECT name,tracks,smart_query FROM playlists WHERE id=?1 AND user_id=?2",
                    params![id, a.id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()?;
            let Some((name, tracks, smart)) = stored else {
                return Ok(None);
            };
            let mut item = json!({"id":id,"track_ids":serde_json::from_str::<Value>(&tracks)?});
            if let Some(smart) = smart {
                item["smart_query"] = serde_json::from_str(&smart)?;
                crate::playlists::resolve(c, &mut item)?;
                if item["truncated"] == true && item["smart_query"]["max_results"].is_null() {
                    anyhow::bail!("匹配超过一万首，请设置最多选取或收窄筛选后导出");
                }
            }
            let ids: Vec<String> = serde_json::from_value(item["track_ids"].clone())?;
            let mut entries = vec![];
            let mut missing = 0;
            for id in ids {
                if let Some((t, path)) = c
                    .query_row(&format!("SELECT t.* FROM tracks t WHERE id=?1 AND {permission}"), [&id], |r| {
                        Ok((Track::from_row(r)?, r.get::<_, Option<String>>("path")?))
                    })
                    .optional()?
                {
                    let location = if !external && t.source == "local" {
                        path.or_else(|| Some(format!("rmusic://track/{}/{}", t.source, t.id)))
                    } else {
                        Some(format!("rmusic://track/{}/{}", t.source, t.id))
                    };
                    entries.push(Entry {
                        id: Some(t.id),
                        source: Some(t.source),
                        location,
                        title: t.title,
                        artist: t.artist,
                        album: t.album,
                        duration: Some(t.duration),
                    });
                } else {
                    missing += 1;
                }
            }
            Ok(Some((name, entries, missing)))
        })
        .await?
        .ok_or_else(ApiError::missing)?;
    let (name, entries, missing) = result;
    if missing > 0 {
        return Err(ApiError::bad(format!(
            "歌单有 {missing} 首资料丢失，请先在歌单中处理；不会静默导出不完整列表"
        )));
    }
    let content = match format.as_str() {
        "json" => serde_json::to_string_pretty(
            &json!({"format":"rmusic-playlist","version":1,"name":name,"tracks":entries}),
        )
        .map_err(anyhow::Error::from)?,
        "m3u8" => format!(
            "#EXTM3U\n{}",
            entries
                .iter()
                .map(|e| format!(
                    "#EXTINF:{},{} - {}\n#RMUSIC:{}\n{}\n",
                    e.duration.unwrap_or(-1.),
                    clean(&e.artist),
                    clean(&e.title),
                    serde_json::to_string(e).unwrap(),
                    clean(e.location.as_deref().unwrap_or(""))
                ))
                .collect::<String>()
        ),
        "pls" => format!(
            "[playlist]\nNumberOfEntries={}\n{}Version=2\n",
            entries.len(),
            entries
                .iter()
                .enumerate()
                .map(|(i, e)| format!(
                    "File{}={}\nTitle{}={} - {}\nLength{}={}\n",
                    i + 1,
                    clean(e.location.as_deref().unwrap_or("")),
                    i + 1,
                    clean(&e.artist),
                    clean(&e.title),
                    i + 1,
                    e.duration.unwrap_or(-1.)
                ))
                .collect::<String>()
        ),
        _ => format!(
            "id,source,location,title,artist,album,duration\r\n{}",
            entries
                .iter()
                .map(|e| [
                    e.id.clone().unwrap_or_default(),
                    e.source.clone().unwrap_or_default(),
                    e.location.clone().unwrap_or_default(),
                    e.title.clone(),
                    e.artist.clone(),
                    e.album.clone(),
                    e.duration.unwrap_or_default().to_string()
                ]
                .iter()
                .map(|v| quote(v))
                .collect::<Vec<_>>()
                .join(",")
                    + "\r\n")
                .collect::<String>()
        ),
    };
    Ok((
        [
            (
                header::CONTENT_TYPE,
                if format == "json" {
                    "application/json; charset=utf-8"
                } else {
                    "text/plain; charset=utf-8"
                }
                .to_owned(),
            ),
            (header::CACHE_CONTROL, "no-store".into()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"playlist.{format}\""),
            ),
        ],
        content,
    )
        .into_response())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats_preserve_quotes_unicode_and_refuse_invalid_files() {
        let entries = parse(Input {
            offset: 0,
            limit: None,
            format: "csv".into(),
            content: "title,artist,album,duration\r\n\"歌曲,\"\"引号\"\"\",歌手,专辑,123\r\n"
                .into(),
        })
        .unwrap();
        assert_eq!(entries[0].title, "歌曲,\"引号\"");
        assert!(parse(Input {
            offset: 0,
            limit: None,
            format: "csv".into(),
            content: "title\n\"unfinished".into()
        })
        .is_err());
        let e = parse(Input {
            offset: 0,
            limit: None,
            format: "m3u8".into(),
            content: "\u{feff}#EXTM3U\n#EXTINF:12,歌手 - 歌曲\nfile:///music/song.flac\n".into(),
        })
        .unwrap();
        assert_eq!(e[0].artist, "歌手");
        assert_eq!(e[0].duration, Some(12.));
    }
}
