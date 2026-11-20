//! Optional introspection augments API v1 without calling authenticated platform APIs.
use anyhow::{ensure, Result};
use serde_json::Value;
pub const METHODS: &[(&str, &str)] = &[
    ("search", "search"),
    ("resolve", "resolve"),
    ("qualities", "qualities"),
    ("metadata", "metadata"),
    ("lyrics", "lyrics"),
    ("playlists", "playlists"),
    ("playlist_tracks", "playlist_tracks"),
    ("liked_songs", "liked_songs"),
    ("recommendations", "recommendations"),
    ("public_recommendations_v1", "public_recommendations"),
    ("related_catalog_v1", "related_catalog"),
    ("public_metadata_v1", "public_metadata"),
    ("artwork_probe_v1", "artwork_probe"),
    ("artist_albums_v1", "artist_albums"),
    ("album_catalog_v1", "album_catalog"),
    ("comments_read_v1", "comments"),
];
pub fn validate_manifest(value: &Value) -> Result<()> {
    if value["kind"] == "extension" || value.get("capabilities").is_none() {
        return Ok(());
    }
    let capabilities = value["capabilities"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("音源须声明能力列表"))?;
    ensure!(capabilities.len() <= 64, "音源能力过多");
    let mut seen = std::collections::HashSet::new();
    for c in capabilities {
        let c = c
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("音源能力须为字符串"))?;
        ensure!(
            crate::connectors::valid_id(c) && seen.insert(c),
            "音源能力无效或重复"
        );
    }
    if let Some(options) = value["quality_options"].as_array() {
        ensure!(options.len() <= 32, "音质选项过多");
        let mut seen = std::collections::HashSet::new();
        for option in options {
            let id = option["id"].as_str().unwrap_or("");
            ensure!(
                crate::connectors::valid_id(id) && seen.insert(id),
                "音质ID无效或重复"
            );
        }
    }
    Ok(())
}
pub fn check_methods(manifest: &Value, report: &Value) -> Result<()> {
    ensure!(report["version"] == 1, "不支持此插件自检版本");
    let methods = report["methods"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("缺少已实现方法列表"))?;
    ensure!(methods.len() <= 128, "插件方法过多");
    for (capability, method) in METHODS {
        if manifest["capabilities"]
            .as_array()
            .is_some_and(|v| v.iter().any(|v| v == capability))
        {
            ensure!(
                methods.iter().any(|v| v == method),
                "已声明能力 {capability} 未实现方法 {method}"
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn capability_contract_is_optional_extensible_and_rejects_false_advertising() {
        let m = json!({"capabilities":["search","resolve","contract_probe_v1","future_v2"],"quality_options":[{"id":"original"}]});
        validate_manifest(&m).unwrap();
        assert!(check_methods(&m, &json!({"version":1,"methods":["search"]})).is_err());
        check_methods(&m, &json!({"version":1,"methods":["search","resolve"]})).unwrap();
        assert!(validate_manifest(&json!({"capabilities":["search","search"]})).is_err());
        assert!(validate_manifest(
            &json!({"capabilities":[],"quality_options":[{"id":"a"},{"id":"a"}]})
        )
        .is_err());
    }
}
