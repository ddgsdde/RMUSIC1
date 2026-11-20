//! Client-only WAN transport; local playback keeps its original cache/resolver.
use anyhow::{Context, Result};
use rmusic_bridge::{
    connection::ConnectionProfile,
    media_proxy::{MediaProxy, MediaResolver, StreamingPolicy},
};
use std::sync::{Arc, RwLock};

pub struct ClientMedia {
    pub local: Arc<crate::downloads::Downloads>,
    pub policy: Arc<RwLock<StreamingPolicy>>,
    pub token: String,
    pub capability: tokio::sync::OnceCell<()>,
}
impl MediaResolver for ClientMedia {
    fn resolve<'a>(
        &'a self,
        profile: &'a ConnectionProfile,
        track: &'a str,
        source: &'a str,
    ) -> futures_util::future::BoxFuture<'a, Result<Arc<MediaProxy>>> {
        Box::pin(self.prepare(profile, track, async { Ok(source.to_owned()) }, None))
    }
}
impl ClientMedia {
    async fn prepare(
        &self,
        profile: &ConnectionProfile,
        track: &str,
        source: impl std::future::Future<Output = Result<String>> + Send,
        output: Option<&rmusic_audio::output::OutputLimits>,
    ) -> Result<Arc<MediaProxy>> {
        if let Ok(Some(file)) = self.local.preferred_original(track).await {
            return Ok(file);
        }
        let mut policy = self
            .policy
            .read()
            .map_err(|_| anyhow::anyhow!("网络播放设置不可用"))?
            .clone();
        if let Some(output)=output {
            policy.max_sample_rate=policy.max_sample_rate.min(output.max_sample_rate.unwrap_or(768000));
            policy.max_bit_depth=policy.max_bit_depth.min(output.max_bit_depth.unwrap_or(32));
        }
        let remote=match policy.mode.as_str(){"lan"=>false,"remote"|"vpn"=>true,_=>crate::network_route::selected_is_remote(&profile.address).await};
        if !remote {
            return self.local.resolve(profile, track, &source.await?).await;
        }
        let private=matches!(rmusic_bridge::network::address_kind(&profile.address),rmusic_bridge::network::AddressKind::Private|rmusic_bridge::network::AddressKind::Loopback);
        let trusted_vpn=policy.mode=="vpn"&&matches!(rmusic_bridge::network::address_kind(&profile.address),rmusic_bridge::network::AddressKind::Private|rmusic_bridge::network::AddressKind::Overlay);
        anyhow::ensure!(
            reqwest::Url::parse(&profile.address)?.scheme() == "https"||private||trusted_vpn,
            "公网播放需要 HTTPS；已连接的可信 Tailscale/VPN 可在网络播放中明确选择 VPN 模式"
        );
        let id = url_encode(track);
        let audio_info = async {
            let response = profile.http()?
                .get(profile.endpoint(&format!("api/v1/library/tracks/{id}/audio-info"))?)
                .bearer_auth(&self.token)
                .timeout(std::time::Duration::from_secs(15))
                .send().await?;
            rmusic_bridge::sequence::media_json(response).await
        };
        let (source, info) = tokio::try_join!(source, audio_info)?;
        if passthrough(&policy,&info) {
            let proxy=self.local.resolve(profile,track,&source).await?;
            proxy.configure_network_buffer(rmusic_bridge::media_proxy::NetworkBuffer::for_audio(&info,5,32*1024*1024).with_preroll(5));
            proxy.fill_after_startup();
            return Ok(proxy);
        }
        self.capability
            .get_or_try_init(|| async {
                let status: serde_json::Value = profile
                    .http()?
                    .get(profile.endpoint("api/v1/status")?)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                anyhow::ensure!(
                    status["capabilities"]["native_flac_v1"] == true,
                    "此 Core 尚不支持外出 FLAC 流，请先更新 Core"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await?;
        let proxy = MediaProxy::remote_stream(profile, &source, policy, info).await?;
        proxy.fill_after_startup();
        Ok(Arc::new(proxy))
    }
}
fn passthrough(policy:&StreamingPolicy,info:&serde_json::Value)->bool {
 let format=info["format"].as_str().unwrap_or("");
 let bits=info["bit_depth"].as_u64().unwrap_or(0);
 // The original lossless compressed entity can be cached and resumed by byte range.
 info["verified"]==true && matches!(format,"flac"|"alac") && bits>0
    && bits<=u64::from(policy.max_bit_depth)
    && info["sample_rate"].as_u64().is_some_and(|rate|rate<=u64::from(policy.max_sample_rate))
    && info["channels"].as_u64().is_some_and(|channels|(1..=2).contains(&channels))
}
#[cfg(test)]mod tests {
 use super::*;use serde_json::json;
 #[test]fn remote_passes_through_original_lossless_compressed_sources(){
  let p=StreamingPolicy::default();
  for format in ["flac","alac"]{assert!(passthrough(&p,&json!({"verified":true,"format":format,"sample_rate":192000,"bit_depth":24,"channels":2})));}
  assert!(!passthrough(&p,&json!({"verified":true,"format":"wav","sample_rate":44100,"bit_depth":16,"channels":2})));
  assert!(!passthrough(&p,&json!({"verified":true,"format":"mp3","sample_rate":48000,"bit_depth":0,"channels":2})));
 }
}
pub(crate) fn url_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
#[cfg(test)]
#[path="streaming_tests.rs"]
mod transport_tests;
#[cfg(test)]
#[path="streaming_live_tests.rs"]
mod live_tests;

/// On the roaming player the renderer can pause, seek or replace preparation while DNS, a ticket or the first byte is pending.
pub struct TravelSource {
    pub api:crate::api::Api,
    pub inner: Arc<ClientMedia>,
    pub limits: rmusic_audio::output::OutputLimits,
}
impl MediaResolver for TravelSource {
    fn resolve<'a>(&'a self,_:&'a ConnectionProfile,track:&'a str,_:&'a str)->futures_util::future::BoxFuture<'a,Result<Arc<MediaProxy>>> {
        Box::pin(async move {
            let (selected, _) = self.api.selected_route().await?;
            let id = url_encode(track);
            self.inner.prepare(&selected, track, self.inner.local.source(&selected, &id), Some(&self.limits)).await
        })
    }
}
