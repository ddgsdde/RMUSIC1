use super::*;
use crate::media_proxy::MediaProxy;
use anyhow::Context;
use rmusic_audio::probe::{self, AudioMetadata, InvalidAudioHeader};
use std::path::Path;
use std::sync::Arc;

// Initial and preloaded songs must use the same integrity rule. Catalogue FLAC
// parameters do not contain STREAMINFO's exact programme extent.
fn usable_hint(value: &Value) -> Option<AudioMetadata> {
    rmusic_audio::probe::verified_pcm(value).filter(|metadata| metadata.format != "flac")
}
impl Config {
    pub async fn prepare_media(
        &self,
        track: &str,
        source: &str,
    ) -> Result<(Arc<MediaProxy>, AudioMetadata)> {
        let proxy = self.media_source(track, source).await?;
        match self.probe_media(track, &proxy).await {
            Ok(metadata) => Ok((proxy, core_rendered_metadata(source, metadata))),
            Err(error)
                if !self.token.is_empty()
                    && MediaProxy::core_pcm(source).is_none()
                    && (error.is::<InvalidAudioHeader>() || proxy.source_failed()) =>
            {
                let local = proxy.serves_local_file();
                drop(proxy);
                let ticket = self.media_ticket(track).await
                    .context("音频校验失败，无法重新获取播放地址")?;
                let source = self.profile.media_url(&ticket)?;
                let fresh = if local {
                    Arc::new(MediaProxy::start(&self.profile, source.as_str(), None).await?)
                } else {
                    self.media_source(track, source.as_str()).await?
                };
                let metadata = self.probe_media(track, &fresh).await?;
                Ok((fresh, metadata))
            }
            Err(error) => Err(error),
        }
    }

    async fn probe_media(&self, track: &str, proxy: &MediaProxy) -> Result<AudioMetadata> {
        if let Some(metadata) = proxy.prepared_metadata() {
            return Ok(metadata);
        }
        let path = Path::new(&proxy.url);
        if proxy.serves_local_file() { return probe::probe_async(path).await; }
        let id = url::form_urlencoded::byte_serialize(track.as_bytes()).collect::<String>();
        let hint = self.get_path(&format!("api/v1/library/tracks/{id}/audio-info")).await;
        if let Some(metadata) = hint.as_ref().ok().and_then(usable_hint) { return Ok(metadata); }
        match probe::probe_async(path).await {
            Err(_) if hint.as_ref().is_err_and(|error| error.is::<UnavailableProgramme>()) => Err(UnavailableProgramme.into()),
            result => result,
        }
    }
}
pub fn core_rendered_metadata(source:&str,mut metadata:AudioMetadata)->AudioMetadata {
    if MediaProxy::core_pcm(source)==Some(true) {
        if let Some(rate)=MediaProxy::core_sample_rate(source) { metadata.sample_rate=rate; }
        metadata.flac_samples=None;
        metadata.source_integer_lossless=false;
    }
    metadata
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preloaded_flac_never_skips_native_programme_boundary_probe() {
        let mut value = json!({"verified":true,"format":"flac","sample_rate":44100,"bit_depth":16,"channels":2,"duration":123.});
        assert!(rmusic_audio::probe::verified_pcm(&value).is_some());
        assert!(
            usable_hint(&value).is_none(),
            "preloading must obtain the same STREAMINFO boundary as first play"
        );
        value["format"] = json!("alac");
        assert!(usable_hint(&value).is_some());
    }

    #[tokio::test]
    async fn denied_online_programme_is_distinguished_from_a_transient_gateway_error() {
        use axum::{http::StatusCode, routing::get, Json, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/denied", get(|| async { (StatusCode::BAD_GATEWAY, Json(json!({"error":{"code":"permission_or_unavailable"}}))) }))
            .route("/temporary", get(|| async { (StatusCode::BAD_GATEWAY, Json(json!({"error":{"code":"source_refresh_cooldown"}}))) }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let client = reqwest::Client::new();
        let denied = media_json(client.get(format!("http://{address}/denied")).send().await.unwrap()).await.unwrap_err();
        let temporary = media_json(client.get(format!("http://{address}/temporary")).send().await.unwrap()).await.unwrap_err();
        assert!(denied.is::<UnavailableProgramme>());
        assert!(temporary.is::<reqwest::Error>());
        server.abort();
    }
}
