use aes_gcm::{
    aead::{rand_core::RngCore, Aead, OsRng, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

const PREFIX: &str = "enc:v1:";

pub struct SecretCipher(Aes256Gcm);

impl SecretCipher {
    pub fn from_encoded_key(encoded_key: &str) -> Result<Self, String> {
        let encoded_key = encoded_key.trim();
        if encoded_key.is_empty() {
            return Err("TGBOT_SECRET_ENCRYPTION_KEY is required".into());
        }
        let key = if encoded_key.len() == 64
            && encoded_key.bytes().all(|value| value.is_ascii_hexdigit())
        {
            hex::decode(encoded_key).map_err(|_| "invalid secret encryption key".to_string())?
        } else {
            BASE64
                .decode(encoded_key)
                .map_err(|_| "secret encryption key must be 32-byte hex or base64".to_string())?
        };
        if key.len() != 32 {
            return Err("secret encryption key must decode to exactly 32 bytes".into());
        }
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| "invalid secret encryption key".to_string())?;
        Ok(Self(cipher))
    }

    pub fn encrypt(
        &self,
        tenant_id: &str,
        secret_name: &str,
        plaintext: &str,
    ) -> Result<String, String> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let aad = associated_data(tenant_id, secret_name);
        let ciphertext = self
            .0
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "secret encryption failed".to_string())?;
        let mut envelope = nonce_bytes.to_vec();
        envelope.extend(ciphertext);
        Ok(format!("{PREFIX}{}", BASE64.encode(envelope)))
    }

    pub fn decrypt(
        &self,
        tenant_id: &str,
        secret_name: &str,
        envelope: &str,
    ) -> Result<String, String> {
        let encoded = envelope
            .strip_prefix(PREFIX)
            .ok_or_else(|| "unsupported secret envelope; re-enter the secret".to_string())?;
        let envelope = BASE64
            .decode(encoded)
            .map_err(|_| "invalid secret envelope".to_string())?;
        if envelope.len() < 12 + 16 {
            return Err("invalid secret envelope".into());
        }
        let (nonce, ciphertext) = envelope.split_at(12);
        let aad = associated_data(tenant_id, secret_name);
        let plaintext = self
            .0
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "secret decryption failed".to_string())?;
        String::from_utf8(plaintext).map_err(|_| "decrypted secret is not valid UTF-8".into())
    }
}

fn associated_data(tenant_id: &str, secret_name: &str) -> String {
    format!("tgbot\0{tenant_id}\0{secret_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn round_trips_tenant_bound_secret() {
        let cipher = SecretCipher::from_encoded_key(KEY).unwrap();
        let encrypted = cipher
            .encrypt("tenant-a", "BOT_TOKEN", "123:secret")
            .unwrap();
        assert!(encrypted.starts_with(PREFIX));
        assert_eq!(
            cipher.decrypt("tenant-a", "BOT_TOKEN", &encrypted).unwrap(),
            "123:secret"
        );
    }

    #[test]
    fn rejects_wrong_tenant_and_invalid_keys() {
        let cipher = SecretCipher::from_encoded_key(KEY).unwrap();
        let encrypted = cipher
            .encrypt("tenant-a", "BOT_TOKEN", "123:secret")
            .unwrap();
        assert!(cipher.decrypt("tenant-b", "BOT_TOKEN", &encrypted).is_err());
        assert!(SecretCipher::from_encoded_key("short").is_err());
    }
}
