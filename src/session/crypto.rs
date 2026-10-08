// Copyright (C) 2026 Marcos Gabriel Miller
//! Credentials at rest: AES-256-GCM with a random nonce per write and the Telegram user id as
//! associated data, so a row copied to another user cannot be decrypted.
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
};
use zeroize::Zeroizing;

const NONCE_LEN: usize = 12;

#[derive(Debug, thiserror::Error)]
#[error("stored credentials cannot be decrypted")]
pub struct CryptoError;

pub struct Credentials {
    pub identifier: Zeroizing<String>,
    pub password: Zeroizing<String>,
}

#[derive(Clone)]
pub struct CredentialCipher {
    cipher: Aes256Gcm,
}

impl CredentialCipher {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new(key.into()),
        }
    }

    /// Returns `nonce || ciphertext`.
    pub fn encrypt(
        &self,
        telegram_user_id: i64,
        identifier: &str,
        password: &str,
    ) -> Result<Vec<u8>, CryptoError> {
        let mut plaintext =
            Zeroizing::new(Vec::with_capacity(identifier.len() + password.len() + 1));
        plaintext.extend_from_slice(identifier.as_bytes());
        plaintext.push(0);
        plaintext.extend_from_slice(password.as_bytes());

        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let aad = telegram_user_id.to_be_bytes();
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError)?;

        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ciphertext);
        Ok(blob)
    }

    pub fn decrypt(&self, telegram_user_id: i64, blob: &[u8]) -> Result<Credentials, CryptoError> {
        if blob.len() <= NONCE_LEN {
            return Err(CryptoError);
        }
        let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
        let aad = telegram_user_id.to_be_bytes();
        let plaintext = Zeroizing::new(
            self.cipher
                .decrypt(
                    Nonce::from_slice(nonce),
                    Payload {
                        msg: ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| CryptoError)?,
        );

        let separator = plaintext
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(CryptoError)?;
        let identifier = std::str::from_utf8(&plaintext[..separator]).map_err(|_| CryptoError)?;
        let password = std::str::from_utf8(&plaintext[separator + 1..]).map_err(|_| CryptoError)?;
        Ok(Credentials {
            identifier: Zeroizing::new(identifier.to_string()),
            password: Zeroizing::new(password.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn round_trip() {
        let cipher = CredentialCipher::new(&KEY);
        let blob = cipher
            .encrypt(42, "default", "ContraseniaSegura2026!")
            .unwrap();
        let credentials = cipher.decrypt(42, &blob).unwrap();
        assert_eq!(credentials.identifier.as_str(), "default");
        assert_eq!(credentials.password.as_str(), "ContraseniaSegura2026!");
    }

    #[test]
    fn nonce_is_random() {
        let cipher = CredentialCipher::new(&KEY);
        assert_ne!(
            cipher.encrypt(42, "a", "b").unwrap(),
            cipher.encrypt(42, "a", "b").unwrap()
        );
    }

    #[test]
    fn other_user_cannot_decrypt() {
        let cipher = CredentialCipher::new(&KEY);
        let blob = cipher.encrypt(42, "default", "secret").unwrap();
        assert!(cipher.decrypt(43, &blob).is_err());
    }

    #[test]
    fn wrong_key_or_tampering_fails() {
        let blob = CredentialCipher::new(&KEY)
            .encrypt(42, "default", "secret")
            .unwrap();
        assert!(CredentialCipher::new(&[8; 32]).decrypt(42, &blob).is_err());

        let mut tampered = blob.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(CredentialCipher::new(&KEY).decrypt(42, &tampered).is_err());
        assert!(CredentialCipher::new(&KEY).decrypt(42, &blob[..5]).is_err());
    }
}
