use anyhow::Result;
pub fn secret() -> String {
    use rand::RngCore;
    let mut bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    data_encoding::BASE64.encode(&bytes)
}
pub fn public_key(secret: &str) -> Result<String> {
    let bytes: [u8; 32] = data_encoding::BASE64
        .decode(secret.as_bytes())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("局域网设备密钥无效"))?;
    let public = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(bytes));
    Ok(data_encoding::BASE64.encode(public.as_bytes()))
}

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub schema: u32,
    pub endpoint_id: String,
    pub challenge: String,
    pub core_id: String,
    pub expires: i64,
    pub encryption_key: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capsule {
    pub ephemeral_key: String,
    pub nonce: String,
    pub ciphertext: String,
}
fn shared(secret: &str, public: &str) -> anyhow::Result<zeroize::Zeroizing<[u8; 32]>> {
    use anyhow::ensure;
    use sha2::{Digest, Sha256};
    let secret: [u8; 32] = data_encoding::BASE64
        .decode(secret.as_bytes())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("私钥无效"))?;
    let public: [u8; 32] = data_encoding::BASE64
        .decode(public.as_bytes())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("公钥无效"))?;
    let dh = x25519_dalek::StaticSecret::from(secret)
        .diffie_hellman(&x25519_dalek::PublicKey::from(public));
    ensure!(dh.was_contributory(), "设备公钥无效");
    let mut hash = Sha256::new();
    hash.update(b"R-Music LAN device enrollment v1\0");
    hash.update(dh.as_bytes());
    Ok(zeroize::Zeroizing::new(hash.finalize().into()))
}
pub fn encrypt<T: Serialize>(public: &str, value: &T, context: &[u8]) -> anyhow::Result<Capsule> {
    use chacha20poly1305::{
        aead::{Aead, KeyInit, Payload},
        ChaCha20Poly1305,
    };
    use rand::RngCore;
    let secret = secret();
    let key = shared(&secret, public)?;
    let mut nonce = [0; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let cipher = ChaCha20Poly1305::new_from_slice(&*key).unwrap();
    let raw = zeroize::Zeroizing::new(serde_json::to_vec(value)?);
    let encrypted = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: &raw,
                aad: context,
            },
        )
        .map_err(|_| anyhow::anyhow!("设备凭据封装失败"))?;
    Ok(Capsule {
        ephemeral_key: public_key(&secret)?,
        nonce: data_encoding::BASE64URL_NOPAD.encode(&nonce),
        ciphertext: data_encoding::BASE64URL_NOPAD.encode(&encrypted),
    })
}
pub fn decrypt<T: serde::de::DeserializeOwned>(
    secret: &str,
    value: Capsule,
    context: &[u8],
) -> anyhow::Result<T> {
    use chacha20poly1305::{
        aead::{Aead, KeyInit, Payload},
        ChaCha20Poly1305,
    };
    let key = shared(secret, &value.ephemeral_key)?;
    let nonce: [u8; 12] = data_encoding::BASE64URL_NOPAD
        .decode(value.nonce.as_bytes())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("随机数无效"))?;
    let encrypted = data_encoding::BASE64URL_NOPAD.decode(value.ciphertext.as_bytes())?;
    anyhow::ensure!(encrypted.len() <= 16384, "设备凭据过大");
    let cipher = ChaCha20Poly1305::new_from_slice(&*key).unwrap();
    let plain = zeroize::Zeroizing::new(
        cipher
            .decrypt(
                (&nonce).into(),
                Payload {
                    msg: &encrypted,
                    aad: context,
                },
            )
            .map_err(|_| anyhow::anyhow!("设备凭据校验失败"))?,
    );
    Ok(serde_json::from_slice(&plain)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_envelope_is_device_and_core_bound() {
        let secret = secret();
        let pubkey = public_key(&secret).unwrap();
        let data = serde_json::json!({"token":"test-only-device-token"});
        let envelope = encrypt(&pubkey, &data, b"core-one/bridge-one").unwrap();
        assert!(!serde_json::to_string(&envelope)
            .unwrap()
            .contains("test-only-device-token"));
        assert!(decrypt::<serde_json::Value>(&secret, envelope, b"other-core").is_err());
        let envelope = encrypt(&pubkey, &data, b"core-one/bridge-one").unwrap();
        assert_eq!(
            decrypt::<serde_json::Value>(&secret, envelope, b"core-one/bridge-one").unwrap(),
            data
        );
    }
}
