use anyhow::{bail, Result};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// One address only. TLS signatures are still verified, even when PKI/name/time checks are skipped.
pub(super) fn config(pin: Option<&str>, skip: bool) -> Result<Arc<rustls::ClientConfig>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let builder = rustls::ClientConfig::builder();
    let verifier = match (pin, skip) {
        (Some(_), true) => bail!("证书固定与跳过证书校验不能同时启用"),
        (Some(pin), false) => Some(ExplicitCertificate::new(pin)?),
        (None, true) => Some(ExplicitCertificate { sha256: None }),
        (None, false) => None,
    };
    let config = match verifier {
        Some(verifier) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth(),
        None => builder
            .with_root_certificates(rustls::RootCertStore::from_iter(
                webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
            ))
            .with_no_client_auth(),
    };
    Ok(Arc::new(config))
}

#[derive(Debug)]
pub(super) struct ExplicitCertificate {
    sha256: Option<[u8; 32]>,
}
impl ExplicitCertificate {
    pub(super) fn new(input: &str) -> Result<Self> {
        let normalized = input.replace(':', "").to_lowercase();
        if normalized.len() != 64 || !normalized.chars().all(|x| x.is_ascii_hexdigit()) {
            bail!("SHA-256 certificate pin must contain exactly 64 hex digits")
        }
        let mut sha256 = [0; 32];
        for (i, byte) in sha256.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&normalized[i * 2..i * 2 + 2], 16)?;
        }
        Ok(Self {
            sha256: Some(sha256),
        })
    }
}
impl ServerCertVerifier for ExplicitCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if self
            .sha256
            .is_none_or(|pin| Sha256::digest(end_entity.as_ref()).as_slice() == pin)
        {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "core certificate SHA-256 changed; explicit profile update required".into(),
            ))
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
