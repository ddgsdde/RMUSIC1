use super::*;
use tokio::io::AsyncWriteExt;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Configuration {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    pub key: String,
}
impl Configuration {
    pub fn validate(&self) -> ApiResult<()> {
        if self.base_url.len() > 2048
            || self.model.len() > 200
            || self.key.len() > 8192
            || self.key.chars().any(char::is_control)
            || self.model.chars().any(char::is_control)
        {
            return Err(ApiError::bad("LLM 地址、模型或密钥格式无效"));
        }
        if !self.base_url.is_empty() {
            endpoint(&self.base_url, "models")?;
        }
        if self.enabled && (self.base_url.is_empty() || self.model.trim().is_empty()) {
            return Err(ApiError::bad("启用前请填写服务地址并选择模型"));
        }
        Ok(())
    }
    pub fn public(&self, revision: String) -> Value {
        json!({"enabled":self.enabled,"base_url":self.base_url,"model":self.model,"key_configured":!self.key.is_empty(),"revision":revision})
    }
}
fn path(s: &AppState) -> std::path::PathBuf {
    s.config.data.join("providers").join(".core-llm.json")
}
pub async fn load(s: &AppState) -> ApiResult<(Configuration, String)> {
    let path = path(s);
    if !tokio::fs::try_exists(&path)
        .await
        .map_err(anyhow::Error::from)?
    {
        return Ok((Configuration::default(), "0".into()));
    }
    if tokio::fs::metadata(&path)
        .await
        .map_err(anyhow::Error::from)?
        .len()
        > 16384
    {
        return Err(ApiError::bad("LLM 配置文件过大"));
    }
    let bytes = tokio::fs::read(path).await.map_err(anyhow::Error::from)?;
    let value = serde_json::from_slice::<Configuration>(&bytes)
        .map_err(|_| ApiError::bad("LLM 配置无法读取，请恢复配置"))?;
    let revision = crate::auth::hash(&data_encoding::BASE64.encode(&bytes));
    Ok((value, revision))
}
pub async fn save(s: &AppState, value: &Configuration) -> ApiResult<String> {
    let path = path(s);
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .map_err(anyhow::Error::from)?;
    let bytes = serde_json::to_vec(value).map_err(anyhow::Error::from)?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let result = async {
        let mut file = options.open(&temp).await?;
        file.write_all(&bytes).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temp, &path).await?;
        #[cfg(unix)]
        {
            let directory = tokio::fs::File::open(path.parent().unwrap()).await?;
            directory.sync_all().await?;
        }
        anyhow::Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    result?;
    Ok(crate::auth::hash(&data_encoding::BASE64.encode(&bytes)))
}
pub fn endpoint(base: &str, path: &str) -> ApiResult<url::Url> {
    let mut url = url::Url::parse(base).map_err(|_| ApiError::bad("请输入 http(s) 服务地址"))?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::bad(
            "服务地址须为 http(s)，不能包含账号、查询参数或片段",
        ));
    }
    let current = url.path().trim_end_matches('/');
    let api = if current.is_empty() { "/v1" } else { current };
    let full = format!("{api}/{path}");
    url.set_path(&full);
    Ok(url)
}
