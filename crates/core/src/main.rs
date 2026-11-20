mod connection_probe;
mod smart_rules;
mod remote_access;
mod lan_bridges;
mod music_versions;
mod music_version_query;
mod artwork_diagnostics;
mod catalog_artwork;
mod album_artwork;
mod cached_artwork;
mod client_artwork;
mod client_metadata;
mod source_observation;
mod music_playback_policy;
mod catalog_aliases;
mod llm;
mod plugin_developer;
mod job_listing;
mod cache_budget;
#[cfg(test)] mod integration_tests;
mod auth;
mod accounts;
mod lyric_cache;
mod autoplay;
mod cache_tags;
mod resources;
mod traffic;
mod playlist_pages;
mod background;
mod clock;
mod db;
mod data_lock;
mod discovery;
mod device_clock;
mod devices;
mod downloads;
mod impulse_assets;
mod error;
mod events;
mod visualization;
mod library;
mod library_query;
mod catalog_order;
mod artist_credits;
mod artist_albums;
mod search_catalog;
mod library_hub;
mod maintenance;
mod operations;
mod playlist_exchange;
mod library_files;
mod cache_versions;
mod cache_policy;
mod cache_repair;
mod loudness;
mod media;
mod source_stream;
mod metadata;
mod comments;
mod catalog;
mod related_catalog;
mod catalog_detail;
mod catalog_cache;
mod catalog_scrape;
mod media_assets;
mod artwork_images;
mod artwork_fetch;
mod artwork_files;
mod artwork_recovery;
mod eq_import;
mod render_media;
mod artwork_probe;
mod artwork_maintenance;
mod cache_identity;
mod music_identity;
mod identity_workflow;
mod fingerprints;
mod extensions;
mod connectors;
mod provider_session;
mod models;
mod openapi;
mod plugin;
mod plugin_contract;
mod plugin_lifecycle;
mod playlists;
mod playlist_sync;
mod recommendations;
mod radio;
mod portable;
mod settings;
mod sequence;
mod state;
mod sync_start;
mod output_groups;
mod device_groups;
mod stats;
mod transcode;
mod zones;
use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};
use state::{AppState, Config};
use std::{net::SocketAddr, path::PathBuf};
fn api() -> Router<AppState> {
    Router::new().route("/status",get(auth::status)).route("/setup",post(auth::setup))
        .route("/lan/bridge-challenge",get(lan_bridges::challenge))
        .route("/lan/bridges",post(lan_bridges::enroll))
        .route("/native/connect",get(remote_access::transport::native))
        .route("/discovery",get(discovery::list))
        .route("/devices",get(devices::list)).route("/devices/register",post(devices::register))
        .route("/devices/{id}/purge",post(devices::purge))
        .route("/devices/{id}",patch(devices::rename).delete(devices::remove)).route("/devices/{id}/restore",post(devices::restore)).route("/outputs/{id}/config",put(devices::configure))
        .route("/outputs/{id}",patch(devices::rename_output))
        .route("/outputs/{id}/volume",put(devices::volume))
        .route("/sync/starts",get(sync_start::list).post(sync_start::start)).route("/sync/starts/{id}",get(sync_start::get)).route("/sync/starts/{id}/cancel",post(sync_start::cancel)).route("/sync/starts/{id}/control",post(sync_start::control::command))
        .route("/sync/starts/{id}/rate",post(sync_start::rate_adjustment::command))
        .route("/sync/starts/{id}/final-receipts",post(sync_start::queue::final_receipt::submit))
        .route("/sync/starts/{id}/recovery",get(sync_start::recovery::get).post(sync_start::recovery::resume))
        .route("/device-groups",get(device_groups::list).post(device_groups::create)).route("/device-groups/{id}",delete(device_groups::delete).put(device_groups::edit))
        .route("/output-groups",get(output_groups::list)).route("/output-groups/{id}",put(output_groups::save).delete(output_groups::delete))
        .route("/auth/login",post(auth::login)).route("/auth/logout",post(auth::logout)).route("/auth/me",get(auth::me))
        .route("/connection-info",get(remote_access::connection_info))
        .route("/connection-probe",get(connection_probe::ping))
        .route("/connection-probe/payload",get(connection_probe::download))
        .route("/remote-access",get(remote_access::get).put(remote_access::save))
        .route("/auth/account",patch(auth::account::update)).route("/auth/totp/disable",post(auth::account::disable))
        .route("/auth/totp/start",post(auth::totp_start)).route("/auth/totp/confirm",post(auth::totp_confirm))
        .route("/auth/tokens",get(auth::list_tokens).post(auth::create_token)).route("/auth/tokens/{id}",axum::routing::delete(auth::delete_token)).route("/auth/users",get(auth::users).post(auth::create_user))
        .route("/auth/sessions",get(auth::sessions)).route("/auth/sessions/current-device",put(auth::identify_session)).route("/auth/sessions/{id}",axum::routing::delete(auth::delete_token))
        .route("/auth/external-accounts",get(accounts::admin::list).post(accounts::admin::create))
        .route("/auth/external-accounts/{id}",patch(accounts::admin::update).delete(accounts::admin::delete))
        .route("/auth/external-accounts/{id}/export-ticket",post(accounts::archive::export_ticket))
        .route("/auth/external-accounts/{id}/export",get(accounts::archive::export))
        .route("/auth/account-imports",get(accounts::archive::imports::list).post(accounts::archive::imports::create))
        .route("/auth/account-imports/{id}",get(accounts::archive::imports::get).delete(accounts::archive::imports::delete))
        .route("/auth/account-imports/{id}/authorize-upload",post(accounts::archive::imports::authorize_upload))
        .route("/auth/account-imports/{id}/file",put(accounts::archive::imports::upload).layer(axum::extract::DefaultBodyLimit::max(accounts::archive::MAX_BYTES)))
        .route("/auth/account-imports/{id}/apply",post(accounts::archive::imports::apply))
        .route("/filesystem/directories",get(maintenance::directories).post(maintenance::mkdir))
        .route("/maintenance/health",get(operations::health))
        .route("/maintenance/backups/{id}/verify",post(operations::verify_backup))
        .route("/maintenance/settings",get(maintenance::settings).put(maintenance::configure))
        .route("/maintenance/backups",get(maintenance::list).post(maintenance::create))
        .route("/maintenance/backups/{id}/file",get(maintenance::file))
        .route("/library/files",get(library_files::list))
        .route("/library/files/actions",post(library_files::action))
        .route("/library/files/bin",get(library_files::bin))
        .route("/library/files/bin/{id}",axum::routing::delete(library_files::purge))
        .route("/library/files/bin/{id}/restore",post(library_files::restore))
        .route("/library/home",get(library_hub::home)).route("/library/folders",get(library_hub::folders))
        .route("/library/tracks/{id}/cache",get(library_hub::cache_status).post(library_hub::cache))
        .route("/library/tracks",get(library::tracks)).route("/library/tracks/{id}",get(library::track))
        .route("/library/tracks/{id}/lyrics",get(library::lyrics)).route("/library/tracks/{id}/ticket",get(media::ticket)).route("/library/tracks/{id}/analyze",post(loudness::analyze))
        .route("/library/tracks/{id}/stats",get(stats::track)).route("/stats",get(stats::overview)).route("/stats/history",get(stats::history)).route("/stats/offline",post(stats::offline::sync))
        .route("/recommendations",get(recommendations::handler))
        .route("/portable/{id}/state",put(portable::publish).delete(portable::disconnect))
        .route("/portable/{id}/ack",post(portable::acknowledge))
        .route("/recommendations/daily",get(radio::personal::daily))
        .route("/recommendations/similar",post(radio::personal::similar))
        .route("/recommendations/discover",get(radio::discovery))
        .route("/recommendations/radio",post(radio::continuation))
        .route("/recommendations/settings",get(radio::settings).put(radio::set_algorithm))
        .route("/recommendations/feedback/{id}",put(radio::feedback))
        .route("/smart-playlists/preview",post(smart_rules::preview))
        .route("/library/tracks/{id}/versions",get(music_versions::list).put(music_versions::prefer))
        .route("/library/tracks/{id}/versions/policy",get(music_playback_policy::get).put(music_playback_policy::put))
        .route("/library/catalog/{kind}/{id}/links",get(catalog_aliases::get).put(catalog_aliases::put))
        .route("/library/tracks/{id}/edition",axum::routing::put(music_versions::edit))
        .route("/llm/settings",get(llm::settings).put(llm::configure))
        .route("/llm/models",post(llm::models))
        .route("/llm/recommend",post(llm::recommend))
        .route("/plugins/development",get(plugin_developer::catalog))
        .route("/plugins/development/template",get(plugin_developer::template))
        .route("/plugins/{id}/contract-check",post(plugin_developer::check))
        .route("/library/links",post(artist_credits::links))
        .route("/library/catalog/{kind}/{id}",get(catalog_detail::detail))
        .route("/library/catalog/artist/{id}/albums",get(artist_albums::list))
        .route("/library/catalog/{kind}/{id}/profile",get(catalog_cache::get).patch(catalog_cache::edit))
        .route("/artwork/{id}",get(media_assets::get))
        .route("/artwork/{id}/public-fetch",get(client_artwork::describe).put(client_artwork::complete).layer(axum::extract::DefaultBodyLimit::max(20*1024*1024)))
        .route("/library/catalog/{kind}/{id}/public-fetch",get(client_metadata::describe).post(client_metadata::complete))
        .route("/artwork-cache",get(artwork_maintenance::status))
        .route("/artwork-cache/events",get(artwork_diagnostics::list))
        .route("/artwork-cache/check",post(artwork_maintenance::check))
        .route("/artwork-cache/refresh",post(artwork_maintenance::refresh))
        .route("/library/albums",get(library::albums)).route("/library/artists",get(library::artists))
        .route("/library/roots",get(library::roots).post(library::add_root)).route("/library/roots/{id}",axum::routing::delete(library::delete_root))
        .route("/providers/{id}/settings",get(connectors::get).patch(connectors::patch))
        .route("/providers/{id}/tracks/{track_id}/qualities",get(plugin::qualities))
        .route("/library/scan",post(library::scan)).route("/jobs",get(job_listing::list)).route("/jobs/{id}",patch(library::job_action))
        .route("/media/{id}",get(media::media)).route("/covers/{id}",get(media::cover))
        .route("/library/tracks/{id}/audio-info",get(catalog::audio_info))
        .route("/library/tracks/{id}/background",get(catalog::background))
        .route("/library/tracks/{id}/related",get(related_catalog::browse))
        .route("/library/tracks/{id}/identity/preview",post(identity_workflow::preview))
        .route("/library/tracks/{id}/identity/restore",post(identity_workflow::restore))
        .route("/library/tracks/{id}/identity",get(music_identity::get).put(music_identity::put))
        .route("/music/entities",get(music_identity::search))
        .route("/music/entities/{id}",get(music_identity::entity))
        .route("/music/index",post(music_identity::enqueue_index))
        .route("/library/tracks/{id}/identity/history",get(music_identity::history))
        .route("/library/tracks/{id}/fingerprint",get(fingerprints::get).post(fingerprints::enqueue))
        .route("/library/tracks/{id}/comments",get(comments::get))
        .route("/search",get(catalog::search))
        .route("/library/search",get(catalog::search_local))
        .route("/transcode/{id}",get(transcode::transcode))
        .route("/pcm/{id}",get(transcode::pcm))
        .route("/native-pcm/{id}",get(transcode::native_pcm))
        .route("/native-flac/{id}",get(transcode::native_flac))
        .route("/mobile-opus/{id}",get(transcode::mobile_opus))
        .route("/zones",get(zones::list).post(zones::create)).route("/zones/{id}",get(zones::get).patch(zones::update).delete(zones::delete))
        .route("/zones/{id}/radio",get(autoplay::preview).post(autoplay::skip))
        .route("/zones/{id}/visualization",post(visualization::read)).route("/zones/{id}/visualization/report",post(visualization::publish))
        .route("/zones/{id}/transfer",post(zones::transfer::start)).route("/zones/{id}/ready",post(zones::transfer::ready))
        .route("/zones/{id}/visualization/lease",get(visualization::lease))
        .route("/zones/{id}/queue",get(zones::snapshots::queue).put(zones::queue_put).post(zones::queue_post).delete(zones::queue_clear)).route("/zones/{id}/queue/reorder",post(zones::queue_reorder)).route("/zones/{id}/dsp",put(zones::dsp))
        .route("/dsp/equalizer/import",post(eq_import::import))
        .route("/dsp/impulses",get(impulse_assets::list).post(impulse_assets::import).layer(axum::extract::DefaultBodyLimit::max(rmusic_audio::impulse::MAX_WAV_BYTES)))
        .route("/dsp/impulses/{id}",get(impulse_assets::get).delete(impulse_assets::delete))
        .route("/dsp/impulses/{id}/file",get(impulse_assets::file))
        .route("/zones/{id}/dsp/impulses",post(impulse_assets::manifest))
        .route("/zones/{id}/prepare",post(sequence::prepare)).route("/zones/{id}/handoff",post(sequence::handoff)).route("/zones/{id}/recover",post(sequence::recovery::recover)).route("/zones/{id}/recover/unavailable",post(sequence::recovery::unavailable)).route("/zones/{id}/{action}",post(zones::command))
        .route("/endpoints",get(zones::endpoints)).route("/endpoints/browser",post(zones::browser_register)).route("/endpoints/browser/{id}/disconnect",post(zones::browser_disconnect)).route("/events/ticket",get(events::ticket)).route("/events",get(events::events)).route("/bridge/events",get(events::bridge))
        .route("/library/cache-repair",get(cache_repair::status).post(cache_repair::start))
        .route("/library/cache-repair/{id}",patch(cache_repair::configure))
        .route("/cache",get(media::cache_status).patch(media::cache_config)).route("/cache/{id}",axum::routing::delete(media::cache_delete))
        .route("/cache/{id}/file",get(cache_tags::file)).route("/cache/{id}/export",post(media::cache_export))
        .route("/downloads",get(downloads::list).post(downloads::create)).route("/downloads/{id}",get(downloads::get).delete(downloads::delete)).route("/downloads/{id}/file",get(downloads::file))
        .route("/extensions",get(extensions::list))
        .route("/extensions/{id}/invoke/{method}",post(extensions::invoke).layer(axum::extract::DefaultBodyLimit::max(64*1024)))
        .route("/plugins/{id}/configuration",get(extensions::configuration).put(extensions::configure))
        .route("/plugins",get(plugin::list)).route("/plugins/{id}",patch(plugin::toggle)).route("/plugins/{id}/health",get(plugin::health))
        .route("/plugins/install",post(plugin_lifecycle::install).layer(axum::extract::DefaultBodyLimit::max(16*1024*1024))).route("/plugins/{id}/rollback",post(plugin_lifecycle::rollback))
        .route("/providers/{id}/recommendations",get(recommendations::platform))
        .route("/providers/{id}/search",get(plugin::search)).route("/providers/{id}/login",post(plugin::login_start)).route("/providers/{id}/login/{session}",get(plugin::login_poll)).route("/providers/{id}/logout",post(plugin::logout)).route("/providers/{id}/sync",post(plugin::sync))
        .route("/providers/{id}/playlists",get(plugin::playlists)).route("/providers/{id}/playlists/{playlist_id}/tracks",get(plugin::playlist_tracks))
        .route("/settings",get(settings::get).patch(settings::patch)).route("/preferences",get(settings::preferences).put(settings::save_preferences)).route("/playlists",get(playlists::list).post(playlists::create))
        .route("/playlists/import/preview",post(playlist_exchange::preview).layer(axum::extract::DefaultBodyLimit::max(4*1024*1024+65536)))
        .route("/playlists/import",post(playlist_exchange::import).layer(axum::extract::DefaultBodyLimit::max(2*1024*1024)))
        .route("/playlists/{id}/export",get(playlist_exchange::export))
        .route("/playlist-directory",get(playlist_sync::directory))
        .route("/playlist-directory/refresh",post(playlist_sync::refresh))
        .route("/playlists/bind",post(playlist_sync::bind)).route("/playlists/{id}/sync",post(playlist_sync::request)).route("/playlists/{id}/tracks",get(playlists::tracks).post(playlists::append))
        .route("/favorites",get(playlists::favorites)).route("/favorites/{id}",axum::routing::put(playlists::favorite))
        .route("/favorites/binding",axum::routing::put(playlist_sync::bind_favorites).delete(playlist_sync::unbind_favorites))
        .route("/playlists/{id}",get(playlists::get).put(playlists::update).delete(playlists::delete))
        .route("/openapi.json",get(openapi::document))
        .fallback(|| async { error::ApiError::missing() })
}
async fn response_privacy(request: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let path=request.uri().path();
    let private=["/auth/","/filesystem/","/maintenance/","/plugins/"].iter().any(|part|path.contains(part));
    let mut response=next.run(request).await;
    let headers=response.headers_mut();
    headers.insert("x-content-type-options",axum::http::HeaderValue::from_static("nosniff"));
    headers.insert("referrer-policy",axum::http::HeaderValue::from_static("no-referrer"));
    headers.insert("x-frame-options",axum::http::HeaderValue::from_static("SAMEORIGIN"));
    if private {headers.insert("cache-control",axum::http::HeaderValue::from_static("no-store"));}
    response
}
pub fn app(s: AppState) -> Router {
    let proxy_state=s.clone();
    let web = tower_http::services::ServeDir::new(&s.config.web)
        .append_index_html_on_directories(true)
        .not_found_service(tower_http::services::ServeFile::new(
            s.config.web.join("index.html"),
        ));
    let routes = Router::new()
        .nest("/api/v1", api().layer(axum::middleware::from_fn_with_state(s.clone(), accounts::guard::boundary)).layer(axum::middleware::from_fn_with_state(s.clone(), metadata::foreground_priority)))
        .fallback_service(web)
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(tower_http::trace::TraceLayer::new_for_http().make_span_with(|request:&axum::http::Request<axum::body::Body>|tracing::info_span!("request",method=%request.method(),path=request.uri().path())));
    let base = s.config.base_path.clone();
    let index = tower_http::services::ServeFile::new(s.config.web.join("index.html"));
    let router=if base.is_empty() {
        routes.route_service("/", index).with_state(s)
    } else {
        let root=format!("{base}/");
        Router::new().nest(&base, routes).route_service(&root,index).route(&base,get(move ||async move{axum::response::Redirect::permanent(&root)})).with_state(s)
    };
    router.layer(axum::middleware::from_fn(response_privacy)).layer(axum::middleware::from_fn_with_state(proxy_state,remote_access::proxy::boundary))
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rmusic_core=info,tower_http=info".into()),
        )
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let data = PathBuf::from(std::env::var("RMUSIC_DATA_DIR").unwrap_or_else(|_| "data".into()));
    tokio::fs::create_dir_all(&data).await?;
    let _data_lock=data_lock::acquire(&data)?;
    for dir in ["cache", "covers", "downloads", "providers"] {
        tokio::fs::create_dir_all(data.join(dir)).await?;
    }
    let data=tokio::fs::canonicalize(data).await?;
    operations::validate_database_location(&data)?;
    let db = db::Db::open(&data.join("library.db"))?;
    let initialized = db
        .run(|c| Ok(c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))? > 0))
        .await?;
    let bootstrap = if initialized {
        String::new()
    } else {
        let path = data.join("bootstrap.token");
        match tokio::fs::read_to_string(&path).await {
            Ok(v) => v.trim().to_string(),
            Err(_) => {
                let t = auth::random_token();
                tokio::fs::write(&path, &t).await?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                        .await?;
                }
                t
            }
        }
    };
    let base_path = std::env::var("RMUSIC_BASE_PATH")
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string();
    if !base_path.is_empty()
        && (!base_path.starts_with('/')
            || base_path.contains("..")
            || base_path.contains('?')
            || base_path.contains('#'))
    {
        anyhow::bail!("RMUSIC_BASE_PATH 必须为空或 /music 形式");
    }
    let config = Config {
        data: data.clone(),
        web: PathBuf::from(
            std::env::var("RMUSIC_WEB_DIR").unwrap_or_else(|_| "apps/web/dist".into()),
        ),
        base_path,
        public_url: std::env::var("RMUSIC_PUBLIC_URL").ok(),
        plugin_dir: std::env::var("RMUSIC_PLUGIN_DIR").map(PathBuf::from).unwrap_or_else(|_|data.join("plugins")),
        python: std::env::var("RMUSIC_PYTHON").unwrap_or_else(|_| {
            if cfg!(windows) {
                "python".into()
            } else {
                "python3".into()
            }
        }),
    };
    install_builtins(&config).await?;
    let state = AppState::new(db, config, bootstrap).await?;
    remote_access::network::spawn(state.clone());
    plugin_lifecycle::recover(&state).await?;
    stats::recover(&state).await?;
    cache_tags::spawn(state.clone());
    metadata::spawn(state.clone());
    loudness::spawn(state.clone());
    background::spawn(state.clone());
    maintenance::spawn(state.clone());
    playlist_sync::spawn(state.clone());
    recommendations::spawn_daily(state.clone());
    zones::monitor_browsers(state.clone());
    portable::monitor(state.clone());
    sync_start::monitor(state.clone());
    let addr=SocketAddr::new(std::env::var("RMUSIC_BIND").unwrap_or_else(|_|"0.0.0.0".into()).trim_matches(['[',']']).parse()?,std::env::var("RMUSIC_PORT").unwrap_or_else(|_|"7800".into()).parse()?);
    if !initialized {
        tracing::info!(path=%data.join("bootstrap.token").display(),"请从一次性初始化凭证文件复制令牌到首次配置页面");
    }
    tracing::info!(address=%addr,"R-Music core listening");
    match (
        std::env::var("RMUSIC_TLS_CERT").ok(),
        std::env::var("RMUSIC_TLS_KEY").ok(),
    ) {
        (Some(cert), Some(key)) => {
            let config = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
            let listener=std::net::TcpListener::bind(addr)?;listener.set_nonblocking(true)?;
            discovery::start(&state,listener.local_addr()?).await;
            let router=app(state.clone()).layer(axum::Extension(remote_access::SecureIngress));
            let handle=axum_server::Handle::new();let shutdown=handle.clone();
            tokio::spawn(async move{operations::shutdown_signal().await;shutdown.graceful_shutdown(Some(std::time::Duration::from_secs(15)));});
            axum_server::from_tcp_rustls(listener, config)?.handle(handle)
                .serve(router.into_make_service_with_connect_info::<SocketAddr>())
                .await?;
        }
        (None, None) => {
            let listener=std::net::TcpListener::bind(addr)?;listener.set_nonblocking(true)?;
            discovery::start(&state,listener.local_addr()?).await;
            let handle=axum_server::Handle::new();let shutdown=handle.clone();
            tokio::spawn(async move{operations::shutdown_signal().await;shutdown.graceful_shutdown(Some(std::time::Duration::from_secs(15)));});
            let local_proxy = remote_access::local_proxy::Capture::configured()?;
            axum_server::from_tcp(listener)?.acceptor(local_proxy).handle(handle).serve(app(state.clone()).into_make_service_with_connect_info::<SocketAddr>()).await?;
        }
        _ => anyhow::bail!("TLS 证书与私钥必须同时设置"),
    }
    state.remote.cancel.cancel();
    state.db.shutdown();
    let _=tokio::time::timeout(std::time::Duration::from_secs(5),state.plugin.stop_all()).await;
    Ok(())
}
async fn install_builtins(config:&Config)->anyhow::Result<()> {
    tokio::fs::create_dir_all(&config.plugin_dir).await?;
    let builtin=PathBuf::from(std::env::var("RMUSIC_BUILTIN_PLUGIN_DIR").unwrap_or_else(|_|"plugins".into()));
    if !builtin.is_dir(){return Ok(());}
    for entry in std::fs::read_dir(builtin)?{let entry=entry?;let source=entry.path();let target=config.plugin_dir.join(entry.file_name());if !source.join("manifest.json").is_file()||!source.join("provider.py").is_file()||target.exists(){continue;}
        tokio::fs::create_dir_all(&target).await?;for file in std::fs::read_dir(source)?{let file=file?;if file.file_type()?.is_file(){tokio::fs::copy(file.path(),target.join(file.file_name())).await?;}}
    }Ok(())
}
