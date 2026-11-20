use super::*;
use futures_util::StreamExt;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const AUDIO: &[u8] = b"0123456789abcdefghijklmn";

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    bytes: Vec<u8>,
    length: usize,
}
impl Reply {
    fn audio(start: usize, end: usize, partial: bool, tag: Option<&str>) -> Self {
        let mut headers = vec![];
        if partial { headers.push(("Content-Range".into(), format!("bytes {start}-{end}/{}", AUDIO.len()))); }
        if let Some(tag) = tag { headers.push(("ETag".into(), tag.into())); }
        Self { status: if partial {206} else {200}, headers, bytes: AUDIO[start..=end].to_vec(), length: end-start+1 }
    }
    fn denied(status: u16) -> Self { Self {status, headers: vec![], bytes: vec![], length: 0} }
}
struct Upstream {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Upstream { fn drop(&mut self) { self.task.abort(); } }
async fn upstream(reply: impl Fn(&str, usize) -> Reply + Send + 'static) -> Upstream {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(vec![]));
    let recorded = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_headers(&mut socket).await;
            let index = { let mut rows = recorded.lock().unwrap(); let i=rows.len(); rows.push(request.clone()); i };
            let response = reply(&request, index);
            let mut headers = format!("HTTP/1.1 {} Test\r\nContent-Length: {}\r\nConnection: close\r\n", response.status, response.length);
            for (key, value) in response.headers { headers.push_str(&format!("{key}: {value}\r\n")); }
            headers.push_str("\r\n");
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(&response.bytes).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    Upstream {url, requests, task}
}
async fn read_headers(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes=vec![];
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(socket.read_u8().await.unwrap()); assert!(bytes.len()<16384);
    }
    String::from_utf8(bytes).unwrap().to_lowercase()
}
async fn provider(f: &Fixture, urls: Vec<String>, cache: bool) {
    f.collection_tracks().await;
    let mut track=library::get_track(&f.s,"qq-only".into()).await.unwrap();
    track.id="recovery-song".into(); track.source="qa-stream".into();
    f.s.db.run(move|c|library::upsert(c,&track,None,None,0,0,None,None)).await.unwrap();
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"stream_quality":"HIRES","playback_cache_mode":"partial"}))).await.0,StatusCode::OK);
    let dir=f.s.config.plugin_dir.join("qa-stream");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),json!({"id":"qa-stream","quality_options":[{"id":"HIRES","label":"fixture"}]}).to_string()).unwrap();
    std::fs::write(dir.join("fixture.json"),json!({"urls":urls,"cache":cache}).to_string()).unwrap();
    std::fs::write(dir.join("provider.py"),r#"import json,sys,time
from pathlib import Path
root=Path(__file__).parent
config=json.loads((root/'fixture.json').read_text())
n=0
for line in sys.stdin:
 r=json.loads(line)
 with (root/'requests.jsonl').open('a') as log: log.write(json.dumps(r)+'\n')
 time.sleep(.05)
 if config.get('fail_after') is not None and n>=config['fail_after']:
  n+=1
  print(json.dumps({'id':r['id'],'error':{'code':'fixture_denied','message':'fixture refresh unavailable'}}),flush=True)
  continue
 url=config['urls'][min(n,len(config['urls'])-1)];n+=1
 result={'url':url,'format':'flac','quality':r['params']['quality'],'expires_in':120,'cache_allowed':config['cache']}
 if config.get('variants'): result.update(config['variants'][min(n-1,len(config['variants'])-1)])
 print(json.dumps({'id':r['id'],'result':result}),flush=True)
"#).unwrap();
}
fn resolutions(f:&Fixture)->Vec<Value> {
    std::fs::read_to_string(f.s.config.plugin_dir.join("qa-stream/requests.jsonl")).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).collect()
}
async fn media_response(f: &Fixture, range: Option<&str>) -> axum::response::Response {
    let mut request=Request::builder().uri("/api/v1/media/recovery-song").header("Authorization",format!("Bearer {}",f.token));
    if let Some(range)=range {request=request.header("Range",range);}
    f.app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap()
}

#[tokio::test]
async fn unread_playback_response_does_not_hold_the_next_songs_source_slot() {
    let origin = upstream(|_, _| Reply::audio(0, AUDIO.len() - 1, false, Some("\"v1\""))).await;
    let f = Fixture::new().await;
    provider(&f, vec![origin.url.clone()], false).await;
    let mut next = library::get_track(&f.s, "recovery-song".into()).await.unwrap();
    next.id = "next-song".into();
    f.s.db.run(move |c| library::upsert(c, &next, None, None, 0, 0, None, None)).await.unwrap();
    assert_eq!(f.call("PATCH", "/api/v1/settings", Some(json!({"song_fetch_concurrency":1}))).await.0, StatusCode::OK);

    let paused = media_response(&f, None).await;
    assert_eq!(paused.status(), StatusCode::OK);
    let request = Request::builder()
        .uri("/api/v1/media/next-song")
        .header("Authorization", format!("Bearer {}", f.token))
        .body(Body::empty()).unwrap();
    let switched = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let response = f.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 100).await.unwrap()
    }).await.expect("a response waiting for its reader must release its source slot");
    assert_eq!(switched.as_ref(), AUDIO);
    assert_eq!(to_bytes(paused.into_body(), 100).await.unwrap().as_ref(), AUDIO);
    f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn full_cache_discards_quick_skips_and_promotes_buffer_only_after_thirty_percent() {
    let origin = upstream(|_, _| Reply::audio(0, AUDIO.len() - 1, false, Some("\"v1\""))).await;
    let f = Fixture::new().await;
    provider(&f, vec![origin.url.clone()], true).await;
    assert_eq!(f.call("PATCH", "/api/v1/settings", Some(json!({"playback_cache_mode":"full"}))).await.0, StatusCode::OK);
    f.s.db.run(|c| { c.execute("UPDATE tracks SET duration=100 WHERE id='recovery-song'", [])?; Ok(()) }).await.unwrap();
    let mut zone = crate::models::Zone::new("preview-zone".into(), "Preview".into(), "preview-output".into());
    zone.track = Some(library::get_track(&f.s, "recovery-song".into()).await.unwrap());
    zone.status = "playing".into();
    zone.renew_playback();
    f.s.zones.write().await.insert(zone.id.clone(), zone.clone());

    let root = f.s.cache_dir().await;
    let memory = || async { f.s.resources.status(&f.s).await["used_bytes"].as_u64().unwrap() };
    let job_count = || async {
        f.s.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM jobs WHERE kind='cache_fill'", [], |r| r.get::<_, i64>(0))?)).await.unwrap()
    };
    let db = &f.s.db;
    let wait_clock = |milliseconds| async move {
        db.run(move |c| {
            c.execute("UPDATE play_sessions SET last_report_at=?1 WHERE zone_id='preview-zone' AND finished_at IS NULL",
                [chrono::Utc::now().timestamp_millis() - milliseconds])?;
            Ok(())
        }).await.unwrap();
    };

    assert_eq!(to_bytes(media_response(&f, None).await.into_body(), 100).await.unwrap().as_ref(), AUDIO);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    assert!(memory().await >= AUDIO.len() as u64);
    crate::stats::record_progress(&f.s, &zone).await.unwrap();
    zone.position = 80.;
    crate::stats::reposition(&f.s, &zone).await.unwrap();
    wait_clock(1000_i64).await;
    zone.position = 81.;
    crate::stats::record_progress(&f.s, &zone).await.unwrap();
    crate::stats::finish(&f.s, &zone, "next").await.unwrap();
    f.s.zones.write().await.remove(&zone.id);
    media::discard_inactive_previews(&f.s).await.unwrap();
    assert_eq!(memory().await, 0);
    assert_eq!(job_count().await, 0);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);

    zone.position = 0.;
    zone.renew_playback();
    f.s.zones.write().await.insert(zone.id.clone(), zone.clone());
    crate::stats::record_progress(&f.s, &zone).await.unwrap();
    assert_eq!(to_bytes(media_response(&f, None).await.into_body(), 100).await.unwrap().as_ref(), AUDIO);
    wait_clock(29500_i64).await;
    zone.position = 30.;
    crate::stats::record_progress(&f.s, &zone).await.unwrap();
    assert_eq!(job_count().await, 0);
    wait_clock(1000_i64).await;
    zone.position = 31.;
    crate::stats::record_progress(&f.s, &zone).await.unwrap();
    assert_eq!(job_count().await, 1);
    f.s.zones.write().await.remove(&zone.id);
    media::discard_inactive_previews(&f.s).await.unwrap();
    assert!(memory().await >= AUDIO.len() as u64);
    let (job,payload)=f.s.db.run(|c|{let row=c.query_row("SELECT id,payload FROM jobs WHERE kind='cache_fill'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?;c.execute("UPDATE jobs SET status='running' WHERE id=?1",[&row.0])?;Ok(row)}).await.unwrap();
    assert!(media::fill_cache(&f.s, &job, &serde_json::from_str(&payload).unwrap()).await.unwrap());
    let path = f.s.db.read(|c| Ok(c.query_row("SELECT path FROM cache_entries WHERE track_id='recovery-song' AND complete=1", [], |r| r.get::<_,String>(0))?)).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), AUDIO);
    assert_eq!(origin.requests.lock().unwrap().len(), 2, "promotion must reuse the fully buffered source");
    f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn expired_source_refreshes_once_preserving_range_and_hires() {
    let origin=upstream(|request,_|if request.starts_with("get /expired ") {Reply::denied(403)}else{Reply::audio(4,15,true,Some("\"v1\""))}).await;
    let f=Fixture::new().await;provider(&f,vec![format!("{}/expired",origin.url),format!("{}/fresh",origin.url)],false).await;
    let response=media_response(&f,Some("bytes=4-15")).await;
    assert_eq!(response.status(),StatusCode::PARTIAL_CONTENT);
    assert_eq!(to_bytes(response.into_body(),100).await.unwrap().as_ref(),&AUDIO[4..16]);
    let requests=origin.requests.lock().unwrap();assert_eq!(requests.len(),2);
    assert!(requests.iter().all(|r|r.contains("range: bytes=4-15\r\n")));drop(requests);
    let calls=resolutions(&f);assert_eq!(calls.len(),2);assert!(calls.iter().all(|r|r["params"]["quality"]=="HIRES"));
    f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn concurrent_audio_info_and_media_coalesce_source_resolution() {
    let origin=upstream(|_,_|Reply::audio(0,AUDIO.len()-1,false,Some("\"v1\""))).await;
    let f=Fixture::new().await;provider(&f,vec![origin.url.clone()],false).await;
    let shared=&f;
    let requests=(0..8).map(|i|async move {if i%2==0 {assert_eq!(shared.call("GET","/api/v1/library/tracks/recovery-song/audio-info",None).await.0,StatusCode::OK);}else{let response=media_response(shared,None).await;assert_eq!(response.status(),StatusCode::OK);assert_eq!(to_bytes(response.into_body(),100).await.unwrap().as_ref(),AUDIO);}});
    futures_util::future::join_all(requests).await;
    assert_eq!(resolutions(&f).len(),1,"one provider RPC per recording / quality");f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn concurrent_expired_requests_share_one_refresh() {
    let origin=upstream(|r,_|if r.starts_with("get /expired ") {Reply::denied(403)}else{Reply::audio(0,AUDIO.len()-1,false,Some("\"v1\""))}).await;
    let f=Fixture::new().await;provider(&f,vec![format!("{}/expired",origin.url),format!("{}/fresh",origin.url)],false).await;
    futures_util::future::join_all((0..8).map(|_|async {
        let response=media_response(&f,None).await;assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(to_bytes(response.into_body(),100).await.unwrap().as_ref(),AUDIO);
    })).await;
    assert_eq!(resolutions(&f).len(),2,"initial resolution and one shared refresh");f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn source_denials_have_a_refresh_cooldown_and_other_statuses_do_not_refresh() {
    for status in [401,403,404,410,416,429,503] {
        let origin=upstream(move|_,_|Reply::denied(status)).await;
        let f=Fixture::new().await;provider(&f,vec![origin.url.clone()],false).await;
        for _ in 0..3 {let response=media_response(&f,None).await;assert_eq!(response.status(),if status==416 {StatusCode::RANGE_NOT_SATISFIABLE}else{StatusCode::BAD_GATEWAY});}
        assert_eq!(resolutions(&f).len(),if [401,403,404,410].contains(&status) {2}else{1},"status {status}");
        f.s.plugin.stop("qa-stream").await;
    }
}

#[tokio::test]
async fn failed_refresh_does_not_repeat_provider_rpc_during_cooldown() {
    let origin=upstream(|_,_|Reply::denied(403)).await;
    let f=Fixture::new().await;provider(&f,vec![origin.url.clone()],false).await;
    let path=f.s.config.plugin_dir.join("qa-stream/fixture.json");
    std::fs::write(path,json!({"urls":[origin.url],"cache":false,"fail_after":1}).to_string()).unwrap();
    for _ in 0..3 {assert_eq!(media_response(&f,None).await.status(),StatusCode::BAD_GATEWAY);}
    assert_eq!(resolutions(&f).len(),2);f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn interrupted_source_resumes_only_missing_bytes_and_completes_cache() {
    let origin=upstream(|_,index|{let mut r=Reply::audio(if index==0 {0}else{8},AUDIO.len()-1,index>0,Some("\"v1\""));if index==0 {r.bytes.truncate(8);}r}).await;
    let f=Fixture::new().await;provider(&f,vec![origin.url.clone()],true).await;
    let response=media_response(&f,None).await;assert_eq!(response.status(),StatusCode::OK);
    assert_eq!(to_bytes(response.into_body(),100).await.unwrap().as_ref(),AUDIO);
    let requests=origin.requests.lock().unwrap();assert_eq!(requests.len(),2);assert!(requests[1].contains("range: bytes=8-23\r\n"));assert!(requests[1].contains("if-range: \"v1\"\r\n"));drop(requests);
    let (_,cache)=f.call("GET","/api/v1/cache?include_partial=true",None).await;assert_eq!(cache["entries"][0]["complete"],true);assert_eq!(cache["used_bytes"],AUDIO.len());
    let response=media_response(&f,None).await;assert_eq!(response.headers()["x-rmusic-cache"],"complete");assert_eq!(to_bytes(response.into_body(),100).await.unwrap().as_ref(),AUDIO);
    assert_eq!(resolutions(&f).len(),1);f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn midstream_expiry_refreshes_without_changing_quality_or_restarting_bytes() {
    let origin=upstream(|_,i|{
        if i==1 {return Reply::denied(403);}
        let mut reply=Reply::audio(if i==0 {0}else{8},23,i>0,Some("\"v1\""));
        if i==0 {reply.bytes.truncate(8);}reply
    }).await;
    let f=Fixture::new().await;provider(&f,vec![format!("{}/original",origin.url),format!("{}/renewed",origin.url)],false).await;
    let response=media_response(&f,None).await;assert_eq!(to_bytes(response.into_body(),100).await.unwrap().as_ref(),AUDIO);
    let requests=origin.requests.lock().unwrap();assert_eq!(requests.len(),3);assert!(requests[2].starts_with("get /renewed "));
    assert!(requests[1..].iter().all(|r|r.contains("range: bytes=8-23\r\n")&&r.contains("if-range: \"v1\"\r\n")));drop(requests);
    let calls=resolutions(&f);assert_eq!(calls.len(),2);assert!(calls.iter().all(|r|r["params"]["quality"]=="HIRES"));f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn resume_never_changes_encoding_quality_or_cache_permission() {
    for replacement in [json!({"format":"mp3"}),json!({"quality":"MP3_320"}),json!({"cache_allowed":false})] {
        let origin=upstream(|_,i|{
            if i==1 {return Reply::denied(403);}
            let mut r=Reply::audio(if i==0 {0}else{8},23,i>0,Some("\"v1\""));if i==0 {r.bytes.truncate(8);}r
        }).await;
        let f=Fixture::new().await;provider(&f,vec![origin.url.clone()],true).await;
        std::fs::write(f.s.config.plugin_dir.join("qa-stream/fixture.json"),json!({"urls":[origin.url],"cache":true,"variants":[{},replacement]}).to_string()).unwrap();
        let response=media_response(&f,None).await;assert!(to_bytes(response.into_body(),100).await.is_err(),"replacement {replacement}");
        let (_,cache)=f.call("GET","/api/v1/cache?include_partial=true",None).await;assert_eq!(cache["used_bytes"],8);assert_eq!(cache["entries"][0]["complete"],false);
        f.s.plugin.stop("qa-stream").await;
    }
}

#[tokio::test]
async fn resume_rejects_changed_or_unverifiable_audio() {
    for mode in ["changed", "weak", "missing", "wrong_start", "wrong_total", "full"] {
        let origin=upstream(move|_,i|{
            let tag=match mode {"weak"=>Some("W/\"v1\""),"missing"=>None,"changed" if i>0=>Some("\"v2\""),_=>Some("\"v1\"")};
            let mut r=Reply::audio(if i==0||mode=="full" {0}else{8},AUDIO.len()-1,i>0&&mode!="full",tag);
            if i==0 {r.bytes.truncate(8);}
            if i>0&&mode=="wrong_start" {r.headers[0].1="bytes 7-23/24".into();}
            if i>0&&mode=="wrong_total" {r.headers[0].1="bytes 8-23/25".into();}
            r
        }).await;
        let f=Fixture::new().await;provider(&f,vec![origin.url.clone()],true).await;
        let response=media_response(&f,None).await;assert!(to_bytes(response.into_body(),100).await.is_err(),"mode {mode}");
        let (_,cache)=f.call("GET","/api/v1/cache?include_partial=true",None).await;assert_eq!(cache["used_bytes"],8);assert_eq!(cache["entries"][0]["complete"],false);
        assert_eq!(origin.requests.lock().unwrap().len(),if ["weak","missing"].contains(&mode) {1}else{2});f.s.plugin.stop("qa-stream").await;
    }
}

#[tokio::test]
async fn source_retries_are_bounded_and_partial_cache_stays_incomplete() {
    let origin=upstream(|_,i|{let start=if i==0 {0}else{8};let mut r=Reply::audio(start,AUDIO.len()-1,i>0,Some("\"v1\""));r.bytes.truncate(if i==0 {8}else{0});r}).await;
    let f=Fixture::new().await;provider(&f,vec![origin.url.clone()],true).await;
    let response=media_response(&f,None).await;assert!(to_bytes(response.into_body(),100).await.is_err());
    assert_eq!(origin.requests.lock().unwrap().len(),3,"initial request and at most two resumes");
    tokio::time::timeout(std::time::Duration::from_secs(3),async{while !f.s.cache_active.lock().await.is_empty(){tokio::task::yield_now().await;}}).await.unwrap();
    let (_,cache)=f.call("GET","/api/v1/cache?include_partial=true",None).await;assert_eq!(cache["used_bytes"],8);assert_eq!(cache["entries"][0]["complete"],false);assert_eq!(resolutions(&f).len(),1);f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn separate_ranges_merge_only_with_the_same_strong_entity() {
    for mode in ["stable","changed","weak","missing"] {
        let origin=upstream(move|_,i|Reply::audio(if i==0 {0}else{12},if i==0 {11}else{23},true,
            match mode {"weak"=>Some("W/\"v1\""),"missing"=>None,"changed" if i>0=>Some("\"v2\""),_=>Some("\"v1\"")})).await;
        let f=Fixture::new().await;provider(&f,vec![origin.url.clone()],true).await;
        for range in ["bytes=0-11","bytes=12-23"] {
            let response=media_response(&f,Some(range)).await;assert_eq!(response.status(),StatusCode::PARTIAL_CONTENT);to_bytes(response.into_body(),100).await.unwrap();
        }
        let (_,cache)=f.call("GET","/api/v1/cache?include_partial=true",None).await;let entries=cache["entries"].as_array().unwrap();
        // Independent partial files also account for their seek holes in the quota.
        assert_eq!(cache["used_bytes"],if mode=="stable" {24}else{36});assert_eq!(entries.len(),if mode=="stable" {1}else{2});
        assert!(entries.iter().all(|e|e["complete"]==(mode=="stable")),"mode {mode}");
        if mode=="stable" {let response=media_response(&f,None).await;assert_eq!(response.headers()["x-rmusic-cache"],"complete");assert_eq!(to_bytes(response.into_body(),100).await.unwrap().as_ref(),AUDIO);}
        f.s.plugin.stop("qa-stream").await;
    }
}

#[tokio::test]
async fn dropping_source_body_cancels_pending_resume() {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let url=format!("http://{}",listener.local_addr().unwrap());
    let (started,ready)=tokio::sync::oneshot::channel();let (closed,observed)=tokio::sync::oneshot::channel();
    let server=tokio::spawn(async move {
        let(mut first,_)=listener.accept().await.unwrap();read_headers(&mut first).await;
        first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 24\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n01234567").await.unwrap();first.shutdown().await.unwrap();drop(first);
        let(mut second,_)=listener.accept().await.unwrap();let request=read_headers(&mut second).await;assert!(request.contains("range: bytes=8-23\r\n"));started.send(()).unwrap();
        let mut byte=[0];assert_eq!(second.read(&mut byte).await.unwrap(),0);closed.send(()).unwrap();
    });
    let f=Fixture::new().await;provider(&f,vec![url],true).await;
    let mut body=media_response(&f,None).await.into_body().into_data_stream();assert_eq!(body.next().await.unwrap().unwrap().as_ref(),&AUDIO[..8]);
    let pending=tokio::spawn(async move {body.next().await});
    tokio::time::timeout(std::time::Duration::from_secs(3),ready).await.unwrap().unwrap();pending.abort();assert!(pending.await.unwrap_err().is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(3),observed).await.unwrap().unwrap();server.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3),async{while !f.s.cache_active.lock().await.is_empty(){tokio::task::yield_now().await;}}).await.unwrap();
    let (_,cache)=f.call("GET","/api/v1/cache?include_partial=true",None).await;assert_eq!(cache["used_bytes"],8);assert_eq!(cache["entries"][0]["complete"],false);assert_eq!(resolutions(&f).len(),1);f.s.plugin.stop("qa-stream").await;
}

#[tokio::test]
async fn closure_full_cache_fills_the_missing_range_after_restart_without_playback_reader(){
    let origin=upstream(|request,_|{let bounds=request.lines().find_map(|line|line.strip_prefix("range: bytes="));let start=bounds.and_then(|b|b.split('-').next()?.parse::<usize>().ok()).unwrap_or(0);let end=bounds.and_then(|b|b.split('-').nth(1)?.parse::<usize>().ok()).unwrap_or(AUDIO.len()-1);Reply::audio(start,end,bounds.is_some(),Some("\"v1\""))}).await;
    let f=Fixture::new().await;provider(&f,vec![origin.url.clone()],true).await;
    f.s.db.run(|c|{c.execute("UPDATE tracks SET provider_data=?1 WHERE id='recovery-song'",[json!({"integrated_lufs":-16.,"provenance":{"integrated_lufs":{"source":"ffmpeg_loudnorm_ebu_r128"}},"analysis":{"loudness":{"cache_id":"obsolete"}}}).to_string()])?;Ok(())}).await.unwrap();
    let response=media_response(&f,Some("bytes=0-7")).await;assert_eq!(to_bytes(response.into_body(),100).await.unwrap().as_ref(),&AUDIO[..8]);
    tokio::time::timeout(std::time::Duration::from_secs(2),async{while !f.s.cache_active.lock().await.is_empty(){tokio::task::yield_now().await;}}).await.unwrap();
    crate::cache_policy::schedule(&f.s, "recovery-song", "HIRES", "manual", None, 50).await.unwrap();
    let (job,payload)=f.s.db.run(|c|{let row=c.query_row("SELECT id,payload FROM jobs WHERE kind='cache_fill'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?;c.execute("UPDATE jobs SET status='running' WHERE id=?1",[&row.0])?;Ok(row)}).await.unwrap();
    f.s.plugin.stop("qa-stream").await;let restarted=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();let key=job.clone();restarted.db.run(move|c|{assert_eq!(c.query_row("SELECT status FROM jobs WHERE id=?1",[&key],|r|r.get::<_,String>(0))?,"interrupted");c.execute("UPDATE jobs SET status='running' WHERE id=?1",[key])?;Ok(())}).await.unwrap();
    let moved=f.s.config.data.join("new-cache-default");std::fs::create_dir_all(&moved).unwrap();*restarted.cache_directory.write().await=moved.clone();
    assert!(media::fill_cache(&restarted,&job,&serde_json::from_str(&payload).unwrap()).await.unwrap());
    assert_eq!(std::fs::read_dir(moved).unwrap().count(),0,"existing entities must keep their recorded directory");
    let(path,complete)=restarted.db.run(|c|Ok(c.query_row("SELECT path,complete FROM cache_entries WHERE track_id='recovery-song'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?)))?)).await.unwrap();assert!(complete);assert_eq!(std::fs::read(path).unwrap(),AUDIO);
    assert!(library::get_track(&restarted,"recovery-song".into()).await.unwrap().integrated_lufs.is_none(),"a new cache version cannot inherit an old analysis");
    assert!(origin.requests.lock().unwrap().iter().any(|r|r.contains("range: bytes=8-")),"resume must start after committed prefix");restarted.plugin.stop("qa-stream").await;
}
