use anyhow::{ensure, Result};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub struct Identity(SigningKey);
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed<T> {
    pub payload: T,
    pub public_key: String,
    pub signature: String,
}
impl Identity {
    pub fn from_seed(seed: &str) -> Result<Self> {
        let bytes: [u8; 32] = data_encoding::BASE64URL_NOPAD
            .decode(seed.as_bytes())?
            .try_into()
            .map_err(|_| anyhow::anyhow!("身份密钥格式无效"))?;
        Ok(Self(SigningKey::from_bytes(&bytes)))
    }
    pub fn seed(&self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(data_encoding::BASE64URL_NOPAD.encode(&self.0.to_bytes()))
    }
    pub fn create() -> Self {
        Self(SigningKey::generate(&mut rand::rngs::OsRng))
    }
    pub fn persist(&self, root: &Path) -> Result<()> {
        crate::vault::atomic_private(&root.join("identity.key"), &self.0.to_bytes())
    }
    pub fn open(root: &Path) -> Result<Self> {
        let path = root.join("identity.key");
        let bytes = crate::vault::private_key(&path)?;
        Ok(Self(SigningKey::from_bytes(&bytes)))
    }
    pub fn public_key(&self) -> String {
        data_encoding::BASE64URL_NOPAD.encode(self.0.verifying_key().as_bytes())
    }
    pub fn tls(&self, core_id: &str) -> Result<std::sync::Arc<rustls::ServerConfig>> {
        use ed25519_dalek::pkcs8::EncodePrivateKey;
        let key = self.0.to_pkcs8_der()?;
        let der = rustls::pki_types::PrivatePkcs8KeyDer::from(key.as_bytes().to_vec());
        let pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&der, &rcgen::PKCS_ED25519)?;
        let certificate =
            rcgen::CertificateParams::new(vec![core_hostname(core_id)])?.self_signed(&pair)?;
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        Ok(std::sync::Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()?
                .with_no_client_auth()
                .with_single_cert(vec![certificate.der().clone()], der.into())?,
        ))
    }
    pub fn sign<T: Serialize>(&self, payload: T) -> Result<Signed<T>> {
        let mut bytes = b"rmusic-connection-manifest-v1\0".to_vec();
        bytes.extend(serde_json::to_vec(&payload)?);
        Ok(Signed {
            signature: data_encoding::BASE64URL_NOPAD.encode(&self.0.sign(&bytes).to_bytes()),
            public_key: self.public_key(),
            payload,
        })
    }
}
pub fn core_hostname(core: &str) -> String {
    format!("core-{core}.rmusic.invalid")
}
#[derive(Debug)]
struct CoreVerifier {
    key: [u8; 32],
    hostname: String,
    algorithms: rustls::crypto::WebPkiSupportedAlgorithms,
}
impl rustls::client::danger::ServerCertVerifier for CoreVerifier {
    fn verify_server_cert(
        &self,
        end: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        name: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let (_, cert) = x509_parser::parse_x509_certificate(end.as_ref())
            .map_err(|_| rustls::Error::General("Core 证书格式无效".into()))?;
        if cert.public_key().subject_public_key.data.as_ref() != self.key
            || name.to_str() != self.hostname
            || !cert.validity().is_valid()
        {
            return Err(rustls::Error::General("Core 身份或证书有效期不匹配".into()));
        }
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        sig: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, sig, &self.algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        sig: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, sig, &self.algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
pub fn core_tls_client(
    core: &str,
    public_key: &str,
) -> Result<std::sync::Arc<rustls::ClientConfig>> {
    pinned_tls_client(core_hostname(core), public_key)
}
/// An explicit key pin still verifies the TLS handshake signature and validity.
pub fn pinned_tls_client(
    hostname: String,
    public_key: &str,
) -> Result<std::sync::Arc<rustls::ClientConfig>> {
    let key: [u8; 32] = data_encoding::BASE64URL_NOPAD
        .decode(public_key.as_bytes())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Core 公钥无效"))?;
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let verifier = CoreVerifier {
        key,
        hostname,
        algorithms: provider.signature_verification_algorithms,
    };
    Ok(std::sync::Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(verifier))
            .with_no_client_auth(),
    ))
}
impl<T: Serialize> Signed<T> {
    pub fn verify(&self, expected: &str) -> Result<()> {
        ensure!(
            self.public_key == expected,
            "Core 身份发生变化，已停止自动连接"
        );
        let key: [u8; 32] = data_encoding::BASE64URL_NOPAD
            .decode(expected.as_bytes())?
            .try_into()
            .map_err(|_| anyhow::anyhow!("公钥格式无效"))?;
        let signature = ed25519_dalek::Signature::from_slice(
            &data_encoding::BASE64URL_NOPAD.decode(self.signature.as_bytes())?,
        )?;
        let mut bytes = b"rmusic-connection-manifest-v1\0".to_vec();
        bytes.extend(serde_json::to_vec(&self.payload)?);
        VerifyingKey::from_bytes(&key)?.verify_strict(&bytes, &signature)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_is_persistent_and_detects_changes() {
        let d = tempfile::tempdir().unwrap();
        let i = Identity::open(d.path()).unwrap();
        let mut s = i
            .sign(serde_json::json!({"core_id":"one","revision":2}))
            .unwrap();
        s.verify(&i.public_key()).unwrap();
        assert_eq!(
            i.public_key(),
            Identity::open(d.path()).unwrap().public_key()
        );
        s.payload["core_id"] = serde_json::json!("two");
        assert!(s.verify(&i.public_key()).is_err());
    }
}
#[cfg(test)]
mod tls_tests {
    use super::*;
    #[tokio::test]
    async fn core_key_pins_real_tls() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::open(dir.path()).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let server = identity.tls(&id).unwrap();
        let client = core_tls_client(&id, &identity.public_key()).unwrap();
        let (a, b) = tokio::io::duplex(65536);
        let receive = tokio::spawn(async move {
            let mut io = tokio_rustls::TlsAcceptor::from(server)
                .accept(a)
                .await
                .unwrap();
            io.write_all(b"core").await.unwrap();
        });
        let mut io = tokio_rustls::TlsConnector::from(client)
            .connect(
                rustls::pki_types::ServerName::try_from(core_hostname(&id)).unwrap(),
                b,
            )
            .await
            .unwrap();
        let mut bytes = [0; 4];
        io.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"core");
        receive.await.unwrap();
        let other = tempfile::tempdir().unwrap();
        let wrong = Identity::open(other.path()).unwrap();
        let server = identity.tls(&id).unwrap();
        let (a, b) = tokio::io::duplex(65536);
        let receive =
            tokio::spawn(async move { tokio_rustls::TlsAcceptor::from(server).accept(a).await });
        let result =
            tokio_rustls::TlsConnector::from(core_tls_client(&id, &wrong.public_key()).unwrap())
                .connect(
                    rustls::pki_types::ServerName::try_from(core_hostname(&id)).unwrap(),
                    b,
                )
                .await;
        assert!(result.is_err());
        let _ = receive.await;
    }
}
