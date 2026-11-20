//! Installed plugins are trusted code; checks run with a new empty data directory.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Cursor, Write};
const SDK: &str = include_str!("plugin_assets/sdk.py");
pub async fn catalog(a: Auth) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    Ok(Json(
        json!({"api_version":"1","sdk_version":"1","templates":[{"kind":"extension","title":"只读曲库扩展"},{"kind":"source","title":"音源骨架"}],"commands":["python3 scripts/plugin_dev.py init my-plugin --kind extension --output /path/to/my-plugin","python3 scripts/plugin_dev.py check /path/to/my-plugin","python3 scripts/plugin_dev.py check /path/to/my-plugin --scenarios /path/to/cases.json","python3 scripts/plugin_dev.py pack /path/to/my-plugin --output /path/to/my-plugin.zip"],"limits":{"compressed_mib":16,"expanded_mib":64,"rpc_kib":128,"extension_kib":64,"timeout_seconds":25},"trust":"插件是可信 Python 代码，不是沙箱。检查使用空数据目录，不提供已安装插件的登录状态。","contract_url":"/api/v1/openapi.json"}),
    ))
}
#[derive(Deserialize)]
pub struct Template {
    kind: String,
    id: String,
}
pub async fn template(a: Auth, Query(q): Query<Template>) -> ApiResult<impl IntoResponse> {
    a.require("admin")?;
    crate::plugin_lifecycle::validate_id(&q.id)
        .map_err(|_| ApiError::bad("插件 ID 无效或为系统保留名称"))?;
    if !["source", "extension"].contains(&q.kind.as_str())
        || !crate::connectors::valid_id(&q.id)
        || !q.id.starts_with(|c: char| c.is_ascii_lowercase())
    {
        return Err(ApiError::bad("请选择模板并填写小写字母开头的插件 ID"));
    }
    let (manifest, provider) = if q.kind == "source" {
        (
            include_str!("plugin_assets/source.json"),
            include_str!("plugin_assets/source.py"),
        )
    } else {
        (
            include_str!("plugin_assets/extension.json"),
            include_str!("plugin_assets/extension.py"),
        )
    };
    let mut manifest: Value = serde_json::from_str(manifest).map_err(anyhow::Error::from)?;
    manifest["id"] = json!(q.id);
    let manifest = serde_json::to_string_pretty(&manifest).map_err(anyhow::Error::from)?;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for(name,text)in [("manifest.json",manifest.as_str()),("provider.py",provider),("rmusic_plugin.py",SDK),("plugin_dev.py",include_str!("plugin_assets/dev.py")),("README.md","# R-Music plugin\n运行 python3 plugin_dev.py check . 检查，再用 python3 plugin_dev.py pack . --output ../plugin.zip 打包。插件拥有 Core 用户的权限；不要记录密钥或无授权操作云端数据。\n")]{zip.start_file(name,zip::write::SimpleFileOptions::default()).map_err(anyhow::Error::from)?;zip.write_all(text.as_bytes()).map_err(anyhow::Error::from)?;}
    let bytes = zip.finish().map_err(anyhow::Error::from)?.into_inner();
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=rmusic-plugin-template.zip",
            ),
        ],
        bytes,
    ))
}
pub async fn check(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if !crate::connectors::valid_id(&id) {
        return Err(ApiError::bad("插件 ID 无效"));
    }
    static GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let permit = GATE.try_acquire().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "plugin_check_busy".into(),
            "已有插件检查正在运行".into(),
        )
    })?;
    // A bounded check finishes cleanup even if the initiating browser disconnects.
    let task = tokio::spawn(async move {
        let _permit = permit;
        let runtime = crate::plugin_lifecycle::RUNTIME.get_or_init(|| tokio::sync::RwLock::new(()));
        let _read = runtime.read().await;
        let path = s.config.plugin_dir.join(&id);
        let result = crate::plugin_lifecycle::developer_probe(&path, &s.config.python).await;
        match result {
            Ok(()) => {
                let manifest:Value=serde_json::from_slice(&tokio::fs::read(path.join("manifest.json")).await.map_err(anyhow::Error::from)?).map_err(anyhow::Error::from)?;
                let mut checks=vec!["package_manifest","schema","rpc_manifest","health"];
                if manifest["capabilities"].as_array().is_some_and(|v|v.iter().any(|v|v=="contract_probe_v1")){checks.push("source_methods");}
                Ok(json!({"id":id,"passed":true,"checks":checks,"uses_installed_credentials":false}))
            },
            Err(_) => Err(ApiError::bad(
                "插件合同检查失败，请使用本地工具检查清单、RPC 和健康状态",
            )),
        }
    });
    Ok(Json(task.await.map_err(anyhow::Error::from)??))
}
