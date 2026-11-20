//! PCM settings belong to the media ticket, including for preloaded songs.
use crate::{error::{ApiError, ApiResult}, models::{Track, Zone}, state::AppState};
use rmusic_audio::{dsp::DspConfig, fade::FadeConfig};

#[derive(Clone)]
pub(crate) struct Recipe {
    pub dsp: DspConfig,
    pub fade: FadeConfig,
}

impl Recipe {
    pub(crate) fn required(&self) -> bool {
        self.dsp.enabled || self.fade.enabled()
    }
}

pub(crate) async fn supported(s: &AppState, endpoint: &str) -> bool {
    s.endpoints.read().await.get(endpoint)
        .is_some_and(|value| value["capabilities"]["core_pcm_dsp_v1"] == true)
}

pub(crate) async fn path(s: &AppState, zone: &Zone, track: &Track, owner: Option<String>) -> ApiResult<String> {
    let scope = format!("media:{}", track.id);
    let encoded = url::form_urlencoded::byte_serialize(track.id.as_bytes()).collect::<String>();
    let mut hints = String::new();
    let recipe = if supported(s, &zone.endpoint_id).await
        && !matches!(track.format.to_ascii_lowercase().as_str(), "dsf" | "dff" | "dsd")
        && track.bit_depth != 1
    {
        let dsp: DspConfig = serde_json::from_value(zone.dsp.clone()).map_err(anyhow::Error::from)?;
        let fade = FadeConfig { fade_in_ms: zone.transitions.fade_in_ms, fade_out_ms: zone.transitions.fade_out_ms };
        let recipe = Recipe { dsp, fade };
        hints.push_str(if recipe.required() { "&core_pcm=render" } else { "&core_pcm=compatibility" });
        if let Some(rate) = recipe.dsp.sample_rate.filter(|_| recipe.dsp.enabled) {
            hints.push_str(&format!("&dsp_rate={rate}"));
        }
        Some(recipe)
    } else {
        None
    };
    let ticket = s.ticket_with_recipe(scope, 86400, owner, recipe).await;
    Ok(format!("api/v1/media/{encoded}?ticket={ticket}{hints}"))
}

pub(crate) async fn recipe(s: &AppState, token: Option<&str>, scope: &str) -> ApiResult<Recipe> {
    let token = token.ok_or_else(ApiError::forbidden)?;
    if !s.check_ticket(token, scope).await { return Err(ApiError::forbidden()); }
    // A general read credential cannot choose the programme sent to a bridge.
    s.tickets.lock().await.get(&crate::auth::hash(token))
        .filter(|entry| entry.scope == scope && entry.expires > chrono::Utc::now().timestamp())
        .and_then(|entry| entry.recipe.clone()).ok_or_else(ApiError::forbidden)
}
