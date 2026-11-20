use crate::config::{Challenge, Ingress};
use anyhow::{ensure, Context, Result};
use rustls::client::danger::ServerCertVerifier;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Files {
    pub cert: PathBuf,
    pub key: PathBuf,
}
/// Verifies PEM parsing, key/certificate agreement, hostname and validity before reload.
pub async fn validate(
    files: &Files,
    domain: &str,
) -> Result<axum_server::tls_rustls::RustlsConfig> {
    Ok(validated_config(files,domain).await?.0)
}
pub(crate) async fn validated_config(files:&Files,domain:&str) -> Result<(axum_server::tls_rustls::RustlsConfig,String)> {
    use sha2::{Digest,Sha256};
    let cert_bytes = crate::vault::read_bounded(&files.cert,1024*1024)?;
    let key_bytes = zeroize::Zeroizing::new(crate::vault::read_bounded(&files.key,128*1024)?);
    let certs =
        rustls_pemfile::certs(&mut cert_bytes.as_slice()).collect::<std::io::Result<Vec<_>>>()?;
    ensure!(!certs.is_empty(), "证书链为空");
    let name = rustls::pki_types::ServerName::try_from(domain.to_owned())?;
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()?;
    verifier.verify_server_cert(
        &certs[0],
        &certs[1..],
        &name,
        &[],
        rustls::pki_types::UnixTime::now(),
    )?;
    let key = rustls_pemfile::private_key(&mut key_bytes.as_slice())?.context("缺少证书私钥")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let private = provider.key_provider.load_private_key(key)?;
    let mut digest=Sha256::new();
    for cert in &certs { digest.update(cert.as_ref()); }
    let fingerprint=format!("{:x}",digest.finalize());
    let certified = rustls::sign::CertifiedKey::new(certs, private);
    certified.keys_match()?;
    Ok((axum_server::tls_rustls::RustlsConfig::from_pem(cert_bytes, key_bytes.to_vec()).await?,fingerprint))
}

pub async fn obtain(
    ingress: &Ingress,
    root: &Path,
    lego: &Path,
    cloudflare: Option<&str>,
    eab: Option<&str>,
    cancel: CancellationToken,
) -> Result<Files> {
    obtain_with_alpn(ingress, root, lego, cloudflare, eab, cancel, None).await
}
pub async fn obtain_with_alpn(
    ingress: &Ingress,
    root: &Path,
    lego: &Path,
    cloudflare: Option<&str>,
    eab: Option<&str>,
    cancel: CancellationToken,
    alpn_port: Option<u16>,
) -> Result<Files> {
    ensure!(ingress.managed(), "当前模式由反向代理管理证书");
    ingress.validate()?;
    if matches!(
        ingress.certificate.challenge,
        Challenge::Imported | Challenge::External
    ) {
        let files = Files {
            cert: PathBuf::from(
                ingress
                    .certificate
                    .external_cert
                    .as_ref()
                    .context("缺少证书路径")?,
            ),
            key: PathBuf::from(
                ingress
                    .certificate
                    .external_key
                    .as_ref()
                    .context("缺少私钥路径")?,
            ),
        };
        validate(&files, &ingress.domain).await?;
        return Ok(files);
    }
    ensure!(
        !ingress.certificate.email.is_empty()
            && ingress.certificate.email.len() <= 254
            && !ingress.certificate.email.chars().any(char::is_control),
        "请配置证书联系邮箱"
    );
    crate::vault::private_directory(root)?;
    let staging = root.join(format!("request-{}", uuid::Uuid::new_v4()));
    crate::vault::private_directory(&staging)?;
    struct RequestCleanup(PathBuf);
    impl Drop for RequestCleanup {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    // Only the fresh UUID request directory created above is removed. Active
    // certificates and persisted ACME accounts are outside this temporary tree.
    let _cleanup = RequestCleanup(staging.clone());
    let accounts = root.join("accounts");
    if accounts.is_dir() { copy_private_tree(&accounts,&staging.join("accounts"),0)?; }
    let config = lego_configuration(ingress,&staging,eab,alpn_port)?;
    let config_file = staging.join("lego.json");
    crate::vault::atomic_private(&config_file,&serde_json::to_vec(&config)?)?;
    let mut command = tokio::process::Command::new(lego);
    command.kill_on_drop(true).env_clear();
    for name in ["PATH", "SSL_CERT_FILE", "SSL_CERT_DIR", "SYSTEMROOT"] {
        if let Some(value)=std::env::var_os(name) { command.env(name,value); }
    }
    if ingress.certificate.challenge == Challenge::DnsCloudflare {
        let token = cloudflare.filter(|v|!v.is_empty()).context("DNS-01 需要 Cloudflare Token")?;
        let file = staging.join("dns-token");
        crate::vault::atomic_private(&file,token.as_bytes())?;
        command.env("CLOUDFLARE_DNS_API_TOKEN_FILE",file);
    }
    command.args(["--config"]).arg(&config_file).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let mut child = command.spawn().context("无法启动 lego 5.x；请安装配套证书工具")?;
    let stderr = child.stderr.take().context("证书工具输出不可用")?;
    let stderr = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut bytes=zeroize::Zeroizing::new(Vec::new());
        let _=stderr.take(65536).read_to_end(&mut bytes).await; bytes
    });
    let outcome = tokio::select! {result=tokio::time::timeout(Duration::from_secs(300),child.wait())=>result.context("证书验证超时")??,_=cancel.cancelled()=>{let _=child.kill().await;let _=child.wait().await;anyhow::bail!("证书任务已取消");}};
    let stderr=stderr.await?;
    ensure!(outcome.success(), "证书申请失败：{}；当前证书保持不变", diagnostic(&stderr,cloudflare,eab));
    let candidate = Files {
        cert: staging
            .join("certificates")
            .join("rmusic.crt"),
        key: staging
            .join("certificates")
            .join("rmusic.key"),
    };
    validate(&candidate, &ingress.domain).await?;
    let cert = crate::vault::read_bounded(&candidate.cert,1024*1024)?;
    let key = zeroize::Zeroizing::new(crate::vault::read_bounded(&candidate.key,128*1024)?);
    let release = root.join(format!("issued-{}", uuid::Uuid::new_v4()));
    crate::vault::private_directory(&release)?;
    let files = Files {
        cert: release.join("chain.pem"),
        key: release.join("private.pem"),
    };
    crate::vault::atomic_private(&files.cert, &cert)?;
    crate::vault::atomic_private(&files.key, &key)?;
    if staging.join("accounts").is_dir() {
        copy_private_tree(&staging.join("accounts"), &accounts, 0)?;
    }
    crate::vault::atomic_private(&root.join("active.json"), &serde_json::to_vec(&files)?)?;
    Ok(files)
}
/// lego 5.x: named configuration avoids secrets in process arguments and keeps
/// challenge listeners independent of the public ACME protocol ports.
fn lego_configuration(ingress:&Ingress,staging:&Path,eab:Option<&str>,alpn_port:Option<u16>)->Result<serde_json::Value>{
    use serde_json::json;
    let mut account=json!({"server":ingress.certificate.directory,"email":ingress.certificate.email,"keyType":"EC256","acceptsTermsOfService":true});
    if let Some(kid)=ingress.certificate.eab_kid.as_deref().filter(|v|!v.is_empty()) {
        let hmac=eab.filter(|v|!v.is_empty()).context("EAB 需要私有 HMAC 密钥")?;
        ensure!(!data_encoding::BASE64URL_NOPAD.decode(hmac.as_bytes()).context("EAB HMAC 须为不补位的 Base64URL")?.is_empty(),"EAB HMAC 为空");
        account["eab"]=json!({"kid":kid,"hmacKey":hmac});
    }
    let challenge=match ingress.certificate.challenge {
        Challenge::DnsCloudflare=>{
            let mut dns=json!({"provider":"cloudflare","resolvers":ingress.certificate.dns_resolvers});
            if ingress.certificate.dns_wait_seconds>0 {dns["propagation"]=json!({"wait":format!("{}s",ingress.certificate.dns_wait_seconds)});}
            json!({"dns":dns})
        },
        Challenge::Http01=>json!({"http":{"address":format!(":{}",ingress.ports.acme_http_listen)}}),
        Challenge::TlsAlpn01=>json!({"tls":{"address":alpn_port.map(|p|format!("127.0.0.1:{p}")).unwrap_or_else(||format!(":{}",if ingress.ports.acme_tls_listen==0{443}else{ingress.ports.acme_tls_listen}))}}),
        _=>anyhow::bail!("外部证书不调用 ACME 工具"),
    };
    Ok(json!({"storage":staging,"accounts":{"rmusic":account},"challenges":{"rmusic":challenge},"certificates":{"rmusic":{"challenge":"rmusic","account":"rmusic","domains":[ingress.domain]}},"log":{"level":"warn","format":"json"}}))
}
fn diagnostic(raw:&[u8],cloudflare:Option<&str>,eab:Option<&str>)->String {
    let mut text=String::from_utf8_lossy(raw).into_owned();
    for secret in [cloudflare,eab].into_iter().flatten().filter(|v|!v.is_empty()) {text=text.replace(secret,"[已隐藏]");}
    let text=text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty(){return "工具退出，请检查证书验证与网络状态".into();}
    text.chars().rev().take(1600).collect::<String>().chars().rev().collect()
}
pub fn renewal_due(files: &Files) -> Result<bool> {
    let bytes = crate::vault::read_bounded(&files.cert,1024*1024)?;
    let cert = rustls_pemfile::certs(&mut bytes.as_slice())
        .next()
        .context("证书为空")??;
    let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
        .map_err(|_| anyhow::anyhow!("证书格式无效"))?;
    let start = parsed.validity().not_before.timestamp();
    let end = parsed.validity().not_after.timestamp();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    Ok(now >= start + (end - start) * 2 / 3)
}
fn copy_private_tree(source: &Path, target: &Path, depth: u8) -> Result<()> {
    ensure!(depth <= 8, "ACME 状态目录过深");
    crate::vault::private_directory(target)?;
    for entry in std::fs::read_dir(source)?.take(1000) {
        let entry = entry?;
        let meta = entry.file_type()?;
        ensure!(!meta.is_symlink(), "ACME 状态不能包含链接");
        let dest = target.join(entry.file_name());
        if meta.is_dir() {
            copy_private_tree(&entry.path(), &dest, depth + 1)?;
        } else if meta.is_file() {
            let bytes = zeroize::Zeroizing::new(crate::vault::read_bounded(&entry.path(),1024*1024)?);
            crate::vault::atomic_private(&dest, &bytes)?;
        }
    }
    Ok(())
}

#[cfg(all(test,unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failed_tool_keeps_active_certificate_hides_secrets_and_removes_its_request_files() {
        use std::os::unix::fs::PermissionsExt;
        let temp=tempfile::tempdir().unwrap(); let root=temp.path().join("certificates"); crate::vault::private_directory(&root).unwrap();
        std::fs::write(root.join("active.json"),b"previous certificate pointer").unwrap();
        let cloudflare="test-only-cloudflare-secret-not-a-real-token";
        let eab="dGVzdC1vbmx5LWVhYi1zZWNyZXQ";
        let tool=temp.path().join("lego");
        std::fs::write(&tool,format!("#!/bin/sh\ntest \"$1\" = --config || exit 91\ntest -f \"$2\" || exit 92\nprintf '%s\\n' \"$@\" > \"$(dirname \"$2\")/../observed-argv\"\nprintf '%s\\n' '{cloudflare} {eab}' >&2\nexit 1\n")).unwrap();
        std::fs::set_permissions(&tool,std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut ingress=Ingress::default(); ingress.mode=crate::config::IngressMode::ManagedHttps; ingress.domain="test.example.com".into();
        ingress.certificate.email="test@example.com".into(); ingress.certificate.eab_kid=Some("test-key-id".into());
        let error=obtain(&ingress,&root,&tool,Some(cloudflare),Some(eab),CancellationToken::new()).await.unwrap_err().to_string();
        assert!(!error.contains(cloudflare)); assert!(!error.contains(eab)); assert!(error.contains("已隐藏"));
        let argv=std::fs::read_to_string(root.join("observed-argv")).unwrap(); assert!(!argv.contains(cloudflare)); assert!(!argv.contains(eab));
        assert_eq!(std::fs::read(root.join("active.json")).unwrap(),b"previous certificate pointer");
        assert!(std::fs::read_dir(&root).unwrap().all(|p| !p.unwrap().file_name().to_string_lossy().starts_with("request-")));
    }
}
