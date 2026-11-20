use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub async fn list(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let rows=s.db.run(|c|{let mut q=c.prepare("SELECT d.id,d.track_id,d.status,d.size,d.error,t.title,t.artist,t.format FROM downloads d LEFT JOIN tracks t ON t.id=d.track_id ORDER BY d.created_at DESC LIMIT 1000")?;let rows=q.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"track_id":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"size":r.get::<_,i64>(3).map(|n|n as u64)?,"error":r.get::<_,Option<String>>(4)?,"title":r.get::<_,Option<String>>(5)?,"artist":r.get::<_,Option<String>>(6)?,"format":r.get::<_,Option<String>>(7)?})))?.collect::<Result<Vec<_>,_>>()?;Ok(rows)}).await?;
    Ok(Json(json!({"items":rows})))
}
/// Poll a single transfer without repeatedly sending the whole download catalogue.
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("read")?;
    let (mut value,path)=s.db.read(move|c|Ok(c.query_row("SELECT d.id,d.track_id,d.status,d.size,d.error,t.title,t.artist,t.format,d.path FROM downloads d LEFT JOIN tracks t ON t.id=d.track_id WHERE d.id=?1",[id],|r|Ok((json!({"id":r.get::<_,String>(0)?,"track_id":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"size":r.get::<_,i64>(3)?,"error":r.get::<_,Option<String>>(4)?,"title":r.get::<_,Option<String>>(5)?,"artist":r.get::<_,Option<String>>(6)?,"format":r.get::<_,Option<String>>(7)?}),r.get::<_,Option<String>>(8)?))).optional()?)).await?.ok_or_else(ApiError::missing)?;
    if value["status"]=="complete" {
        let path=path.ok_or_else(ApiError::missing)?;
        // Inspect the approved file once at completion, not the track's playback entitlement.
        let audio=rmusic_audio::probe::probe_async(std::path::Path::new(&path)).await.map_err(|_|ApiError::bad("下载文件缺失或无效，请重新下载"))?;
        value["format"]=json!(audio.format);
        value["source_audio"]=json!({"verified":true,"format":audio.format,"sample_rate":audio.sample_rate,"bit_depth":audio.bit_depth,"channels":audio.channels,"bitrate":audio.bitrate,"duration":audio.duration});
    }
    Ok(Json(value))
}
/// A conversion of a download may only read this exact completed approval.
pub(crate) async fn approved_path(s:&AppState,id:&str,track:&str)->ApiResult<std::path::PathBuf> {
    let id=id.to_owned();let track=track.to_owned();
    let path=s.db.read(move|c|Ok(c.query_row("SELECT path FROM downloads WHERE id=?1 AND track_id=?2 AND status='complete'",params![id,track],|r|r.get::<_,String>(0)).optional()?)).await?.ok_or_else(ApiError::missing)?;
    let path=std::path::PathBuf::from(path);
    if !tokio::fs::metadata(&path).await.is_ok_and(|m|m.is_file()&&m.len()>0){return Err(ApiError::missing());}
    Ok(path)
}
#[derive(Deserialize)]
pub struct NewDownload {
    track_id: String,
}
pub async fn create(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<NewDownload>,
) -> ApiResult<Json<Value>> {
    a.require("control")?;
    let t = crate::library::get_track(&s, v.track_id.clone()).await?;
    let tid = v.track_id.clone();
    let existing=s.db.run(move|c|Ok(c.query_row("SELECT id,status FROM downloads WHERE track_id=?1 AND status IN ('pending','interrupted','downloading','complete') LIMIT 1",[tid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?)).await?;
    if let Some((id, status)) = existing {
        return Ok(Json(json!({"id":id,"track_id":v.track_id,"status":status})));
    }
    let generation=if t.source=="local"{None}else{Some(crate::provider_session::start(&s,&t.source).await?)};
    let _source = if t.source == "local" {
        None
    } else {
        crate::plugin::check_enabled(&s, &t.source).await?;
        let result = s
            .plugin
            .call(
                &t.source,
                "resolve",
                json!({"track_id":t.id,"quality":"best","for_download":true}),
            )
            .await?;
        crate::provider_session::validate(&s,&t.source,generation.unwrap()).await?;
        if !result["download_allowed"].as_bool().unwrap_or(false) || result["preview"]==true || result["protected"]==true {
            return Err(ApiError::bad("该音源或账号没有下载权限"));
        }
        Some(result)
    };
    let id = uuid::Uuid::new_v4().to_string();
    let did = id.clone();
    let trackid = v.track_id.clone();
    let provider=t.source.clone();
    s.db.run(move |c| {
        if let Some(generation)=generation{crate::provider_session::check(c,&provider,generation)?;}
        c.execute(
            "INSERT INTO downloads(id,track_id,status,created_at) VALUES(?1,?2,'pending',?3)",
            params![did, trackid, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    })
    .await?;
    crate::background::enqueue(&s,"download",&id,json!({"download_id":id,"track_id":v.track_id,"provider_generation":generation}),0).await?;
    Ok(Json(json!({"id":id,"track_id":v.track_id,"status":"pending"})))
}
pub(crate) async fn run_job(s:&AppState,job:&str,payload:&Value)->anyhow::Result<bool> {
    let id=payload["download_id"].as_str().ok_or_else(||anyhow::anyhow!("缺少下载标识"))?;
    let track=crate::library::get_track(s,payload["track_id"].as_str().unwrap_or("").into()).await.map_err(|e|anyhow::anyhow!(e.2))?;
    let quality=crate::catalog::quality(s,&track.source).await;
    let _song=if track.source!="local"{Some(s.traffic.song(&format!("{}:{quality}",track.id),true).await)}else{None};
    let key=id.to_owned();let exists=s.db.run(move|c|Ok(c.execute("UPDATE downloads SET status='downloading',error=NULL WHERE id=?1 AND status!='complete'",[key])?>0)).await?;
    if !exists{return Ok(true);}
    let result=async {
        let generation=if track.source=="local"{None}else{Some(payload["provider_generation"].as_i64().ok_or_else(||anyhow::anyhow!("旧下载未绑定音源会话，请重新创建下载"))?)};
        let source=if track.source=="local"{None}else{
            crate::plugin::check_enabled(s,&track.source).await.map_err(|e|anyhow::anyhow!(e.2))?;
            crate::provider_session::validate(s,&track.source,generation.unwrap()).await.map_err(|e|anyhow::anyhow!(e.2))?;
            let quality=crate::catalog::quality(s,&track.source).await;
            let value=s.plugin.call(&track.source,"resolve",json!({"track_id":track.id,"quality":quality,"for_download":true})).await.map_err(|e|anyhow::anyhow!(e.2))?;
            crate::provider_session::validate(s,&track.source,generation.unwrap()).await.map_err(|e|anyhow::anyhow!(e.2))?;
            if value["download_allowed"]!=true||value["preview"]==true||value["protected"]==true {anyhow::bail!("该音源或账号没有完整音频下载权限");}Some(value)
        };
        download(s,id,&track,source,job,generation).await
    }.await;
    let key=id.to_owned();let status=match &result {Ok(true)=>"complete",Ok(false)=>"pending",Err(_)=>"failed"};let error=result.as_ref().err().map(ToString::to_string);
    s.db.run(move|c|{c.execute("UPDATE downloads SET status=?2,error=?3 WHERE id=?1",params![key,status,error])?;Ok(())}).await?;
    s.emit("downloads",json!({}));result
}

async fn download(
    s: &AppState,
    id: &str,
    t: &crate::models::Track,
    source: Option<Value>,
    job:&str,
    generation:Option<i64>,
) -> anyhow::Result<bool> {
    let temp = s
        .config
        .data
        .join("downloads")
        .join(format!("{id}.partial"));
    let key=id.to_owned();let saved=s.db.run(move|c|Ok(c.query_row("SELECT source_etag,size FROM downloads WHERE id=?1",[key],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,i64>(1)? as u64)))?)).await?;
    let mut size=tokio::fs::metadata(&temp).await.map_or(0,|m|m.len()).min(saved.1);
    let mut out=tokio::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&temp).await?;
    out.set_len(size).await?;
    let saved=saved.0;
    let expected;
    let format = source
        .as_ref()
        .and_then(|v| v["format"].as_str())
        .unwrap_or(&t.format)
        .to_string();
    let mut input: futures_util::stream::BoxStream<
        'static,
        Result<axum::body::Bytes, std::io::Error>,
    > = if let Some(v) = source {
        let url = v["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("音源未返回下载地址"))?;
        let mut req = s.http.get(url);
        if let Some(h) = v["headers"].as_object() {
            for (k, v) in h {
                if let Some(v) = v.as_str() {
                    req = req.header(k, v);
                }
            }
        }
        let mut req=req.header(header::ACCEPT_ENCODING,"identity");
        if size>0&&saved.is_some(){req=req.header(header::RANGE,format!("bytes={size}-")).header(header::IF_RANGE,saved.as_deref().unwrap());}
        let mut response=req.send().await?.error_for_status()?;
        let entity=crate::source_stream::strong_etag(&response);
        let resume=size>0&&entity==saved&&entity.is_some()&&response.status()==StatusCode::PARTIAL_CONTENT&&response.headers().get(header::CONTENT_RANGE).and_then(|h|h.to_str().ok()).and_then(crate::source_stream::content_range).is_some_and(|r|r.0==size);
        if !resume {size=0;out.set_len(0).await?;
            if response.status()==StatusCode::PARTIAL_CONTENT {let mut req=s.http.get(url).header(header::ACCEPT_ENCODING,"identity");if let Some(headers)=v["headers"].as_object(){for(k,v)in headers{if let Some(v)=v.as_str(){req=req.header(k,v);}}}response=req.send().await?.error_for_status()?;}
        }
        let entity=crate::source_stream::strong_etag(&response);let key=id.to_owned();s.db.run(move|c|{c.execute("UPDATE downloads SET source_etag=?2 WHERE id=?1",params![key,entity])?;Ok(())}).await?;
        expected=if resume{response.headers().get(header::CONTENT_RANGE).and_then(|h|h.to_str().ok()).and_then(crate::source_stream::content_range).map(|r|r.2)}else{response.content_length().or(v["expected_size"].as_u64())};
        response
            .bytes_stream()
            .map(|v| v.map_err(std::io::Error::other))
            .boxed()
    } else {
        let tid = t.id.clone();
        let p: String =
            s.db.run(move |c| {
                Ok(c.query_row("SELECT path FROM tracks WHERE id=?1", [tid], |r| r.get(0))?)
            })
            .await?;
        let mut file=tokio::fs::File::open(p).await?;
        expected=Some(file.metadata().await?.len());
        // Local sources are cheap to re-read; without a saved entity stamp do not mix versions.
        size=0;out.set_len(0).await?;file.seek(std::io::SeekFrom::Start(0)).await?;
        tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024).boxed()
    };
    out.seek(std::io::SeekFrom::Start(size)).await?;
    let mut out=tokio::io::BufWriter::with_capacity(1024*1024,out);let mut reported=size;
    while let Some(chunk) = input.next().await {
        let bytes = chunk?;
        out.write_all(&bytes).await?;
        size += bytes.len() as u64;
        if size-reported>=1024*1024 {if let Some(generation)=generation{crate::provider_session::validate(s,&t.source,generation).await.map_err(|e|anyhow::anyhow!(e.2))?;}reported=size;out.flush().await?;let did=id.to_owned();s.db.run(move|c|{c.execute("UPDATE downloads SET size=?2 WHERE id=?1",params![did,size as i64])?;Ok(())}).await?;
            if !crate::background::progress(s,job,"下载音乐文件",size,expected.unwrap_or(0)).await? {return Ok(false);}}
        if !crate::background::disposition(s,job).await?||s.traffic.playback_waiting(){out.flush().await?;return Ok(false);}
    }
    out.flush().await?;
    let out=out.into_inner();
    if size == 0 || expected.is_some_and(|e| e != size) {
        anyhow::bail!("下载不完整：长度校验失败");
    }
    out.sync_all().await?;
    drop(out);
    let safe_format = if format.chars().all(|c| c.is_ascii_alphanumeric()) {
        format
    } else {
        "bin".into()
    };
    let path = s
        .config
        .data
        .join("downloads")
        .join(format!("{id}.{safe_format}"));
    let p = path.to_string_lossy().to_string();
    let did = id.to_string();
    let provider=t.source.clone();
    s.db.run(move |c| {
        if let Some(generation)=generation{crate::provider_session::check(c,&provider,generation)?;}
        std::fs::rename(&temp,&path)?;
        c.execute(
            "UPDATE downloads SET status='complete',size=?1,path=?2 WHERE id=?3",
            params![size as i64, p, did],
        )?;
        Ok(())
    })
    .await?;
    Ok(true)
}
pub async fn delete(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    a.require("control")?;
    let did = id.clone();
    let path =
        s.db.run(move |c| {
            let p = c
                .query_row("SELECT path FROM downloads WHERE id=?1", [&did], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .optional()?
                .flatten();
            c.execute("UPDATE jobs SET status='cancelled' WHERE kind='download' AND cursor=?1",[&did])?;
            c.execute("DELETE FROM downloads WHERE id=?1", [did])?;
            Ok(p)
        })
        .await?;
    if let Some(p) = path {
        let _ = tokio::fs::remove_file(p).await;
    }
    s.emit("downloads", json!({}));
    Ok(StatusCode::NO_CONTENT)
}
pub async fn file(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<crate::media::TicketQuery>,
    h: HeaderMap,
    method: Method,
) -> ApiResult<Response> {
    crate::media::authorize(&s, &h, &q, &format!("download:{id}")).await?;
    let v=s.db.run(move|c|Ok(c.query_row("SELECT d.path,t.title,t.format FROM downloads d JOIN tracks t ON d.track_id=t.id WHERE d.id=?1 AND d.status='complete'",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?)).await?.ok_or_else(ApiError::missing)?;
    let mut f = tokio::fs::File::open(&v.0)
        .await
        .map_err(|_| ApiError::missing())?;
    let metadata = f.metadata().await.map_err(anyhow::Error::from)?;
    let total = metadata.len();
    // Completed approvals are immutable managed files. Bind ranges to this opened incarnation,
    // so a replaced file can never be spliced into a client's partial download.
    let etag = format!("\"{}\"", crate::auth::hash(&format!("{}:{total}:{}", v.0,
        metadata.modified().ok().and_then(|t|t.duration_since(std::time::UNIX_EPOCH).ok()).map(|t|t.as_nanos()).unwrap_or(0))));
    let requested = if h.get(header::IF_RANGE).and_then(|v|v.to_str().ok()).is_some_and(|v|v!=etag) {
        None
    } else { h.get(header::RANGE).and_then(|v|v.to_str().ok()) };
    let format = std::path::Path::new(&v.0)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or(&v.2);
    let Ok((start, end, partial)) =
        crate::media::range(requested, total)
    else {
        return Ok((
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{total}"))],
        )
            .into_response());
    };
    f.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(anyhow::Error::from)?;
    let encoded = url::form_urlencoded::byte_serialize(format!("{}.{}", v.1, format).as_bytes())
        .collect::<String>().replace('+', "%20");
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from_stream(tokio_util::io::ReaderStream::new(f.take(end - start + 1)))
    };
    let mut r = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, crate::media::mime(format))
        .header(header::CONTENT_LENGTH, end - start + 1)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename*=UTF-8''{encoded}"),
        );
    if partial {
        r = r.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total}"),
        );
    }
    Ok(r.body(body).map_err(anyhow::Error::from)?)
}
