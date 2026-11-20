//! Trusted Python provider packages: stage, validate, probe, then replace.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use anyhow::{bail, Context};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::{Cursor, Read, Write},
    path::{Component, Path as FsPath, PathBuf},
    sync::OnceLock,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::Mutex,
};

const ZIP_LIMIT: usize = 16 * 1024 * 1024;
const EXPANDED_LIMIT: u64 = 64 * 1024 * 1024;
static MUTATION: OnceLock<Mutex<()>> = OnceLock::new();
pub(crate) static RUNTIME: OnceLock<tokio::sync::RwLock<()>> = OnceLock::new();

#[derive(Clone, Debug, Deserialize)]
struct Manifest {
    id: String,
    version: String,
    api_version: String,
    core_api: String,
    entrypoint: String,
}
impl Manifest {
    fn validate(&self) -> anyhow::Result<()> {
        validate_id(&self.id)?;
        semver::Version::parse(&self.version).context("插件版本须为 SemVer")?;
        if self.api_version != "1" || self.entrypoint != "provider.py" {
            bail!("仅接受 API v1、入口 provider.py 的 Python 音源插件");
        }
        let core = semver::Version::parse(env!("CARGO_PKG_VERSION"))?;
        if !semver::VersionReq::parse(&self.core_api)?.matches(&core) {
            bail!("插件与当前核心版本不兼容");
        }
        Ok(())
    }
}

pub(crate) fn validate_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id.starts_with(|c: char| c.is_ascii_lowercase())
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        || reserved_name(id)
    {
        bail!("插件 ID 只能使用小写字母、数字、下划线和连字符，并以字母开头");
    }
    Ok(())
}
fn reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}
fn safe_entry(name: &str) -> anyhow::Result<PathBuf> {
    if name.contains(['\\', ':', '\0']) || name.starts_with('/') || name.len() > 240 {
        bail!("ZIP 含不安全路径");
    }
    let path = FsPath::new(name);
    for c in path.components() {
        let Component::Normal(part) = c else {
            bail!("ZIP 含不安全路径")
        };
        let p = part.to_str().context("ZIP 路径须为 UTF-8")?;
        if p.ends_with(['.', ' ']) || reserved_name(p) || p.eq_ignore_ascii_case(".venv") {
            bail!("ZIP 不得包含保留路径或预构建虚拟环境");
        }
    }
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        extension.as_str(),
        "exe" | "dll" | "pyd" | "so" | "dylib" | "pyc"
    ) {
        bail!("插件 ZIP 不接受预构建可执行文件；依赖应通过明确启用的 pip 安装");
    }
    Ok(path.to_path_buf())
}

fn manifest_file(dir: &FsPath) -> anyhow::Result<Manifest> {
    let file = std::fs::File::open(dir.join("manifest.json"))?;
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 {
        bail!("manifest.json 过大");
    }
    let value:Value=serde_json::from_slice(&bytes).context("manifest.json 格式无效")?;
    crate::plugin_contract::validate_manifest(&value)?;
    crate::extensions::validate_manifest(&value).map_err(|e|anyhow::anyhow!(e.2))?;
    let manifest: Manifest = serde_json::from_value(value)?;
    if !dir.join("provider.py").is_file() {
        bail!("缺少 provider.py");
    }
    Ok(manifest)
}

fn read_manifest(dir: &FsPath) -> anyhow::Result<Manifest> {
    let manifest = manifest_file(dir)?;
    manifest.validate()?;
    Ok(manifest)
}

fn extract(bytes: Vec<u8>, target: &FsPath) -> anyhow::Result<Manifest> {
    if bytes.len() > ZIP_LIMIT {
        bail!("插件 ZIP 不能超过 16 MiB");
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("插件包不是有效 ZIP")?;
    if archive.len() == 0 || archive.len() > 1000 {
        bail!("ZIP 文件数量须为 1–1000");
    }
    let mut names = HashSet::new();
    let mut expanded = 0u64;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let relative = safe_entry(file.name().trim_end_matches('/'))?;
        if file.enclosed_name().is_none() || file.is_symlink() {
            bail!("ZIP 不得包含路径逃逸或符号链接");
        }
        if !names.insert(relative.to_string_lossy().to_lowercase()) {
            bail!("ZIP 含重复文件路径");
        }
        expanded = expanded.checked_add(file.size()).context("ZIP 大小溢出")?;
        if expanded > EXPANDED_LIMIT {
            bail!("ZIP 解压总量不能超过 64 MiB");
        }
        let output = target.join(&relative);
        if file.is_dir() {
            std::fs::create_dir_all(output)?;
            continue;
        }
        std::fs::create_dir_all(output.parent().unwrap())?;
        let mut dest = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        let declared = file.size();
        let copied = std::io::copy(&mut file.by_ref().take(declared + 1), &mut dest)?;
        if copied != declared {
            bail!("ZIP 文件大小与声明不符");
        }
        dest.flush()?;
    }
    // A manifest at the archive root is deliberate: no ambiguous nested packages.
    read_manifest(target)
}

/// Make a command absolute without following the last symlink (venv Python on Linux).
fn absolute_command(value: &FsPath) -> anyhow::Result<PathBuf> {
    let candidate = if value.components().count() > 1 || value.is_absolute() {
        value.to_path_buf()
    } else {
        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&path)
            .flat_map(|dir| {
                let plain = dir.join(value);
                if cfg!(windows) {
                    vec![plain.clone(), plain.with_extension("exe")]
                } else {
                    vec![plain]
                }
            })
            .find(|p| p.is_file())
            .context("找不到配置的 Python 解释器")?
    };
    if !candidate.is_file() {
        bail!("Python 解释器不存在");
    }
    Ok(
        std::fs::canonicalize(candidate.parent().unwrap_or(FsPath::new(".")))?
            .join(candidate.file_name().unwrap()),
    )
}
fn interpreter(dir: &FsPath, configured: &str) -> anyhow::Result<PathBuf> {
    let local = dir.join(if cfg!(windows) {
        ".venv/Scripts/python.exe"
    } else {
        ".venv/bin/python"
    });
    absolute_command(if local.is_file() {
        &local
    } else {
        FsPath::new(configured)
    })
}

async fn run_command(command: &mut Command, seconds: u64) -> anyhow::Result<()> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(Duration::from_secs(seconds), command.status())
        .await
        .context("安装依赖超时")??;
    if !status.success() {
        bail!("Python 依赖安装失败；请检查 Python、网络与固定版本 requirements.txt");
    }
    Ok(())
}
async fn dependencies(dir: &FsPath, configured: &str) -> anyhow::Result<()> {
    let requirements = tokio::fs::read_to_string(dir.join("requirements.txt"))
        .await
        .context("请求安装依赖，但包中没有 requirements.txt")?;
    if requirements.len() > 64 * 1024 {
        bail!("requirements.txt 过大");
    }
    // Do not let a requirements file replace the registry or invoke editable/local installs.
    for line in requirements
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        if line.starts_with('-') || line.contains(['/', '\\', '@', ':']) || !line.contains("==") {
            bail!("依赖须为 PyPI 上的固定版本 name==version，不接受 URL、本地路径或 pip 参数");
        }
    }
    let python = absolute_command(FsPath::new(configured))?;
    run_command(
        Command::new(python)
            .arg("-m")
            .arg("venv")
            .arg(dir.join(".venv")),
        120,
    )
    .await?;
    let python = interpreter(dir, configured)?;
    run_command(
        Command::new(python)
            .arg("-m")
            .arg("pip")
            .arg("--isolated")
            .arg("install")
            .arg("--index-url")
            .arg("https://pypi.org/simple")
            .arg("--disable-pip-version-check")
            .arg("--no-input")
            .arg("--quiet")
            .arg("-r")
            .arg(dir.join("requirements.txt")),
        300,
    )
    .await
}

async fn bounded_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> anyhow::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            bail!("插件在健康检查期间退出");
        }
        let end = chunk.iter().position(|b| *b == b'\n').map(|i| i + 1);
        let n = end.unwrap_or(chunk.len());
        if line.len() + n > 1024 * 1024 {
            bail!("插件健康检查响应过大");
        }
        line.extend_from_slice(&chunk[..n]);
        reader.consume(n);
        if end.is_some() {
            return Ok(line);
        }
    }
}
async fn probe(dir: &FsPath, configured: &str, expected: &Manifest) -> anyhow::Result<()> {
    let python = interpreter(dir, configured)?;
    let entry = std::fs::canonicalize(dir.join("provider.py"))?;
    // A candidate is not given the installed provider's login credentials.
    let health_data = tempfile_path(dir.parent().context("插件目录缺少父路径")?);
    tokio::fs::create_dir_all(&health_data).await?;
    let check = async {
        let mut child = Command::new(python)
            .arg("-u")
            .arg(entry)
            .current_dir(dir)
            .env("RMUSIC_PROVIDER_DATA_DIR", &health_data)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut input = child.stdin.take().context("缺少插件输入管道")?;
        let mut output = BufReader::new(child.stdout.take().context("缺少插件输出管道")?);
        let descriptor:Value=serde_json::from_slice(&std::fs::read(dir.join("manifest.json"))?)?;
        let mut methods=vec!["manifest","health"];
        if descriptor["capabilities"].as_array().is_some_and(|v|v.iter().any(|v|v=="contract_probe_v1")){methods.push("contract");}
        let result = async {
            for method in methods {
                let request = json!({"id":method,"method":method,"params":{}});
                input
                    .write_all(serde_json::to_string(&request)?.as_bytes())
                    .await?;
                input.write_all(b"\n").await?;
                input.flush().await?;
                let value: Value = serde_json::from_slice(&bounded_line(&mut output).await?)?;
                if value["id"] != method || value.get("error").is_some() {
                    bail!("插件 {method} 检查失败");
                }
                let result = &value["result"];
                if method == "manifest" {
                    crate::extensions::validate_manifest(result).map_err(|e|anyhow::anyhow!(e.2))?;
                    let actual: Manifest = serde_json::from_value(result.clone())?;
                    actual.validate()?;
                    if actual.id != expected.id
                        || actual.version != expected.version
                        || actual.core_api != expected.core_api
                    {
                        bail!("插件 RPC 清单与包内清单不一致");
                    }
                } else if method=="contract" {crate::plugin_contract::check_methods(&descriptor,result)?;
                } else if result["status"] != "ready" {
                    bail!("插件未就绪");
                }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        let _ = child.kill().await;
        result
    };
    let result = tokio::time::timeout(Duration::from_secs(25), check)
        .await
        .context("插件健康检查超时");
    let _ = tokio::fs::remove_dir_all(&health_data).await;
    result?
}
fn tempfile_path(parent: &FsPath) -> PathBuf {
    parent.join(format!(".health-{}", uuid::Uuid::new_v4()))
}

#[derive(Default, Deserialize)]
pub struct InstallOptions {
    #[serde(default)]
    pub install_dependencies: bool,
}

pub async fn install(
    State(s): State<AppState>,
    auth: Auth,
    Query(options): Query<InstallOptions>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    auth.require("admin")?;
    if body.len() > ZIP_LIMIT {
        return Err(ApiError::bad("插件 ZIP 不能超过 16 MiB"));
    }
    // The job outlives an HTTP disconnect; the lock also excludes rollback.
    let task = tokio::spawn(async move { install_inner(s, body.to_vec(), options).await });
    let result = task.await.map_err(anyhow::Error::from)??;
    Ok(Json(result))
}
async fn install_inner(s: AppState, bytes: Vec<u8>, options: InstallOptions) -> ApiResult<Value> {
    let _guard = MUTATION.get_or_init(|| Mutex::new(())).lock().await;
    let root = tokio::fs::canonicalize(&s.config.plugin_dir)
        .await
        .map_err(anyhow::Error::from)?;
    let staging = root.join(".staging").join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(anyhow::Error::from)?;
    let work = async {
        let destination = staging.clone();
        let manifest = tokio::task::spawn_blocking(move || extract(bytes, &destination)).await??;
        if options.install_dependencies {
            dependencies(&staging, &s.config.python).await?;
        }
        probe(&staging, &s.config.python, &manifest).await?;
        replace(&s, &root, &staging, &manifest).await
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    work.map_err(|e: anyhow::Error| ApiError::bad(format!("插件安装失败：{e}")))
}

fn backups(root: &FsPath, id: &str) -> anyhow::Result<Vec<PathBuf>> {
    let dir = root.join(".rollback").join(id);
    if !dir.is_dir() {
        return Ok(vec![]);
    }
    let mut items = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|d| d.file_type().is_ok_and(|t| t.is_dir()))
        .map(|d| d.path())
        .filter(|d| d.join("manifest.json").is_file())
        .collect::<Vec<_>>();
    items.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    Ok(items)
}
fn backup_path(root: &FsPath, id: &str) -> PathBuf {
    root.join(".rollback").join(id).join(format!(
        "{:020}-{}",
        chrono::Utc::now().timestamp_millis(),
        uuid::Uuid::new_v4()
    ))
}
async fn record(s: &AppState, manifest: &Manifest, previous: Option<String>, replaced: bool) -> anyhow::Result<()> {
    let id = manifest.id.clone();
    let version = manifest.version.clone();
    s.db.run(move |db| {
        let tx = db.transaction()?;
        let changed = tx.execute("INSERT INTO plugins(id,enabled,version,previous_version) VALUES(?1,1,?2,?3) ON CONFLICT(id) DO UPDATE SET version=excluded.version,previous_version=excluded.previous_version WHERE ?4 OR plugins.version IS NOT excluded.version OR plugins.previous_version IS NOT excluded.previous_version",
            rusqlite::params![id, version, previous, replaced])?;
        if changed != 0 { crate::provider_session::bump(&tx, &id)?; }
        tx.commit()?;
        Ok(())
    }).await
}
async fn replace(
    s: &AppState,
    root: &FsPath,
    staged: &FsPath,
    manifest: &Manifest,
) -> anyhow::Result<Value> {
    let active = root.join(&manifest.id);
    let previous = if active.is_dir() {
        let installed = manifest_file(&active)?;
        if installed.id != manifest.id {
            bail!("已安装目录与清单 ID 不一致");
        }
        Some(installed.version)
    } else {
        None
    };
    let backup = backup_path(root, &manifest.id);
    tokio::fs::create_dir_all(backup.parent().unwrap()).await?;
    let _runtime = RUNTIME
        .get_or_init(|| tokio::sync::RwLock::new(()))
        .write()
        .await;
    s.plugin.stop(&manifest.id).await;
    if previous.is_some() {
        tokio::fs::rename(&active, &backup).await?;
    }
    if let Err(e) = tokio::fs::rename(staged, &active).await {
        if previous.is_some() {
            tokio::fs::rename(&backup, &active)
                .await
                .context("新目录切换失败，旧目录恢复失败")?;
        }
        return Err(e.into());
    }
    if let Err(error) = record(s, manifest, previous.clone(), true).await {
        tokio::fs::rename(&active, staged)
            .await
            .context("数据库提交失败，新目录撤回失败")?;
        if previous.is_some() {
            tokio::fs::rename(&backup, &active)
                .await
                .context("数据库提交失败，旧目录恢复失败")?;
        }
        return Err(error);
    }
    // Retain exactly one known previous version after a successful commit.
    for old in backups(root, &manifest.id)? {
        if old != backup {
            let _ = tokio::fs::remove_dir_all(old).await;
        }
    }
    s.emit("plugins", json!({"id":manifest.id,"status":"installed"}));
    Ok(
        json!({"id":manifest.id,"version":manifest.version,"previous_version":previous,"status":"installed"}),
    )
}

pub async fn rollback(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    auth.require("admin")?;
    validate_id(&id).map_err(|e| ApiError::bad(e.to_string()))?;
    let task = tokio::spawn(async move {
        let _guard = MUTATION.get_or_init(|| Mutex::new(())).lock().await;
        let root = tokio::fs::canonicalize(&s.config.plugin_dir).await?;
        let backup = backups(&root, &id)?
            .into_iter()
            .next()
            .context("没有可回滚的版本")?;
        let manifest = read_manifest(&backup)?;
        if manifest.id != id {
            bail!("回滚版本 ID 不匹配");
        }
        probe(&backup, &s.config.python, &manifest).await?;
        replace(&s, &root, &backup, &manifest).await
    });
    let result = task
        .await
        .map_err(anyhow::Error::from)?
        .map_err(|e| ApiError::bad(format!("插件回滚失败：{e}")))?;
    Ok(Json(result))
}

/// Called once before serving: recover a crash between renames, reconcile DB versions.
pub async fn recover(s: &AppState) -> anyhow::Result<()> {
    let _guard = MUTATION.get_or_init(|| Mutex::new(())).lock().await;
    tokio::fs::create_dir_all(&s.config.plugin_dir).await?;
    let root = tokio::fs::canonicalize(&s.config.plugin_dir).await?;
    let rollback = root.join(".rollback");
    let mut restored = HashSet::new();
    if rollback.is_dir() {
        for item in std::fs::read_dir(&rollback)? {
            let item = item?;
            let id = item.file_name().to_string_lossy().to_string();
            if validate_id(&id).is_err() || root.join(&id).exists() {
                continue;
            }
            if let Some(previous) = backups(&root, &id)?.into_iter().next() {
                let manifest = read_manifest(&previous)?;
                if manifest.id != id {
                    bail!("回滚清单 ID 不匹配");
                }
                tokio::fs::rename(previous, root.join(&id)).await?;
                restored.insert(id);
            }
        }
    }
    for item in std::fs::read_dir(&root)? {
        let item = item?;
        if !item.file_type()?.is_dir() || item.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let manifest = match read_manifest(&item.path()) {
            Ok(manifest) => manifest,
            Err(error) => {
                tracing::warn!(path=?item.path(),%error,"skip incompatible provider during recovery");
                continue;
            }
        };
        if item.file_name().to_string_lossy() != manifest.id {
            bail!("已安装目录与清单 ID 不一致");
        }
        let previous = backups(&root, &manifest.id)?
            .first()
            .and_then(|p| read_manifest(p).ok())
            .map(|m| m.version);
        record(s, &manifest, previous, restored.contains(&manifest.id)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(version: &str) -> Value {
        json!({"id":"example","version":version,"api_version":"1","core_api":">=0.1.0, <0.2.0","entrypoint":"provider.py"})
    }
    fn zip_files(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
    fn fixture(dir: &FsPath, version: &str, healthy: bool) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("manifest.json"), manifest(version).to_string()).unwrap();
        let status = if healthy { "ready" } else { "broken" };
        std::fs::write(dir.join("provider.py"), format!("import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line)\n result=json.loads(pathlib.Path(__file__).with_name('manifest.json').read_text()) if r['method']=='manifest' else {{'status':'{status}'}}\n print(json.dumps({{'id':r['id'],'result':result}}),flush=True)\n")).unwrap();
    }
    async fn state(dir: &FsPath) -> AppState {
        let config = crate::state::Config {
            data: dir.join("data"),
            web: dir.join("web"),
            base_path: String::new(),
            public_url: None,
            plugin_dir: dir.join("plugins"),
            python: std::env::var("RMUSIC_TEST_PYTHON").unwrap_or_else(|_|if cfg!(windows){"python".into()}else{"python3".into()}),
        };
        std::fs::create_dir_all(&config.data).unwrap();
        std::fs::create_dir_all(&config.plugin_dir).unwrap();
        let db = crate::db::Db::open(&dir.join("db.sqlite")).unwrap();
        AppState::new(db, config, "test".into()).await.unwrap()
    }
    #[test]
    fn rejects_unsafe_paths_and_incompatible_manifests() {
        for path in [
            "../outside",
            "/outside",
            "C:/file",
            "a\\b",
            "a/../b",
            "CON/file",
            "x. /file",
            ".venv/bin/python",
            "x/test.pyd",
        ] {
            assert!(safe_entry(path).is_err(), "accepted {path}");
        }
        let mut incompatible = manifest("0.1.0");
        incompatible["core_api"] = json!(">=9.0.0");
        assert!(serde_json::from_value::<Manifest>(incompatible)
            .unwrap()
            .validate()
            .is_err());
        assert!(validate_id("con").is_err());
        assert!(validate_id("QQmusic").is_err());
    }
    #[test]
    fn extracts_only_root_package_and_bounds_expansion() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = zip_files(&[
            ("manifest.json", manifest("0.1.0").to_string().into_bytes()),
            ("provider.py", b"# fixture".to_vec()),
        ]);
        assert_eq!(extract(bytes, temp.path()).unwrap().id, "example");
        let bad = tempfile::tempdir().unwrap();
        assert!(extract(zip_files(&[("../escape", vec![1])]), bad.path()).is_err());
        assert!(!bad.path().parent().unwrap().join("escape").exists());
        let mut linked = zip::ZipWriter::new(Cursor::new(Vec::new()));
        linked
            .add_symlink(
                "alias",
                "../outside",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        let link_dir = tempfile::tempdir().unwrap();
        assert!(extract(linked.finish().unwrap().into_inner(), link_dir.path()).is_err());
        let large = tempfile::tempdir().unwrap();
        assert!(extract(
            zip_files(&[("large", vec![0; EXPANDED_LIMIT as usize + 1])]),
            large.path()
        )
        .is_err());
    }
    #[tokio::test]
    async fn failed_health_preserves_installed_version() {
        let temp = tempfile::tempdir().unwrap();
        let s = state(temp.path()).await;
        fixture(&s.config.plugin_dir.join("example"), "0.1.0", true);
        let stage = temp.path().join("candidate");
        fixture(&stage, "0.1.1", false);
        let bytes = zip_files(&[
            (
                "manifest.json",
                std::fs::read(stage.join("manifest.json")).unwrap(),
            ),
            (
                "provider.py",
                std::fs::read(stage.join("provider.py")).unwrap(),
            ),
        ]);
        assert!(install_inner(s.clone(), bytes, InstallOptions::default())
            .await
            .is_err());
        assert_eq!(
            read_manifest(&s.config.plugin_dir.join("example"))
                .unwrap()
                .version,
            "0.1.0"
        );
        assert!(backups(&s.config.plugin_dir, "example").unwrap().is_empty());
    }
    #[tokio::test]
    async fn update_and_rollback_preserve_enable_state_and_previous_version() {
        let _guard = MUTATION.get_or_init(|| Mutex::new(())).lock().await;
        let temp = tempfile::tempdir().unwrap();
        let s = state(temp.path()).await;
        let root = std::fs::canonicalize(&s.config.plugin_dir).unwrap();
        fixture(&root.join("example"), "0.1.0", true);
        s.db.run(|db| {
            db.execute(
                "INSERT INTO plugins(id,enabled,version) VALUES('example',0,'0.1.0')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let stage = root.join("candidate");
        fixture(&stage, "0.1.1", true);
        let new = read_manifest(&stage).unwrap();
        probe(&stage, &s.config.python, &new).await.unwrap();
        replace(&s, &root, &stage, &new).await.unwrap();
        assert_eq!(
            read_manifest(&root.join("example")).unwrap().version,
            "0.1.1"
        );
        let old = backups(&root, "example").unwrap().remove(0);
        let old_manifest = read_manifest(&old).unwrap();
        probe(&old, &s.config.python, &old_manifest).await.unwrap();
        replace(&s, &root, &old, &old_manifest).await.unwrap();
        assert_eq!(
            read_manifest(&root.join("example")).unwrap().version,
            "0.1.0"
        );
        let row =
            s.db.run(|db| {
                Ok(db.query_row(
                    "SELECT enabled,version,previous_version FROM plugins WHERE id='example'",
                    [],
                    |r| {
                        Ok((
                            r.get::<_, bool>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )?)
            })
            .await
            .unwrap();
        assert_eq!(row, (false, "0.1.0".into(), "0.1.1".into()));
        assert_eq!(backups(&root, "example").unwrap().len(), 1);
    }
    #[tokio::test]
    async fn restart_recovers_interrupted_directory_swap() {
        let temp = tempfile::tempdir().unwrap();
        let s = state(temp.path()).await;
        fixture(&backup_path(&s.config.plugin_dir, "example"), "0.1.0", true);
        assert!(!s.config.plugin_dir.join("example").exists());
        recover(&s).await.unwrap();
        let epoch = crate::provider_session::start(&s, "example").await.unwrap();
        recover(&s).await.unwrap();
        assert_eq!(crate::provider_session::start(&s, "example").await.unwrap(), epoch);
        assert_eq!(
            read_manifest(&s.config.plugin_dir.join("example"))
                .unwrap()
                .version,
            "0.1.0"
        );
        assert_eq!(
            s.db.run(|db| Ok(db.query_row(
                "SELECT version FROM plugins WHERE id='example'",
                [],
                |r| r.get::<_, String>(0)
            )?))
            .await
            .unwrap(),
            "0.1.0"
        );
    }

    #[tokio::test]
    async fn database_failure_restores_old_directory() {
        let temp = tempfile::tempdir().unwrap();
        let s = state(temp.path()).await;
        let root = std::fs::canonicalize(&s.config.plugin_dir).unwrap();
        fixture(&root.join("example"), "0.1.0", true);
        let stage = root.join("candidate");
        fixture(&stage, "0.1.1", true);
        s.db.run(|db| {
            db.execute("DROP TABLE plugins", [])?;
            Ok(())
        })
        .await
        .unwrap();
        assert!(replace(&s, &root, &stage, &read_manifest(&stage).unwrap())
            .await
            .is_err());
        assert_eq!(
            read_manifest(&root.join("example")).unwrap().version,
            "0.1.0"
        );
        assert_eq!(read_manifest(&stage).unwrap().version, "0.1.1");
        assert!(backups(&root, "example").unwrap().is_empty());
    }
}

pub(crate) async fn developer_probe(dir:&FsPath,python:&str)->anyhow::Result<()> {let manifest=manifest_file(dir)?;manifest.validate()?;probe(dir,python,&manifest).await}
