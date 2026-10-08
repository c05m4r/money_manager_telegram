// Copyright (C) 2026 Marcos Gabriel Miller
pub mod catalog;
pub mod crypto;
pub mod store;

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use uuid::Uuid;

use crate::{
    api::{ApiClient, ApiError, models::LoginResponse},
    errors::{BotError, BotResult},
};
use crypto::CredentialCipher;
use store::{SessionRow, Store};

/// Renew the JWT this many seconds before it expires.
const RENEW_MARGIN_SECS: i64 = 60;
pub const LOGIN_FAILURE_LIMIT: i64 = 5;
pub const LOGIN_FAILURE_WINDOW_SECS: i64 = 15 * 60;

/// What a handler needs to call the API on behalf of a Telegram user.
#[derive(Clone)]
pub struct Auth {
    pub telegram_user_id: i64,
    pub user_uuid: Uuid,
    pub role: String,
    pub token: String,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("telegram_user_id", &self.telegram_user_id)
            .field("user_uuid", &self.user_uuid)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

/// Reads `exp` from a JWT payload. The signature is not checked: the bot does not hold the key
/// and only uses `exp` to know when to renew.
pub fn jwt_expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()?
        .get("exp")?
        .as_i64()
}

/// Owns the Telegram user ↔ backend session mapping and transparent re-login.
pub struct SessionManager {
    api: ApiClient,
    store: Store,
    cipher: Option<CredentialCipher>,
    /// One lock per user so concurrent updates do not trigger several logins.
    locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
}

impl SessionManager {
    pub fn new(api: ApiClient, store: Store, credentials_key: Option<[u8; 32]>) -> Self {
        Self {
            api,
            store,
            cipher: credentials_key.as_ref().map(CredentialCipher::new),
            locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn stores_credentials(&self) -> bool {
        self.cipher.is_some()
    }

    fn lock_for(&self, telegram_user_id: i64) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .expect("session locks poisoned")
            .entry(telegram_user_id)
            .or_default()
            .clone()
    }

    /// Seconds until a new login attempt is allowed, if the user is over the failure limit.
    pub async fn login_retry_after(&self, telegram_user_id: i64) -> BotResult<Option<i64>> {
        let now = chrono::Utc::now().timestamp();
        let (count, oldest) = self
            .store
            .login_failures_since(telegram_user_id, now - LOGIN_FAILURE_WINDOW_SECS)
            .await?;
        Ok((count >= LOGIN_FAILURE_LIMIT).then(|| {
            oldest
                .map(|oldest| oldest + LOGIN_FAILURE_WINDOW_SECS - now)
                .unwrap_or(LOGIN_FAILURE_WINDOW_SECS)
                .max(1)
        }))
    }

    /// Interactive `/login`. Wrong credentials are reported as `BotError::User` and counted.
    pub async fn login(
        &self,
        telegram_user_id: i64,
        chat_id: i64,
        identifier: &str,
        password: &str,
    ) -> BotResult<LoginResponse> {
        let lock = self.lock_for(telegram_user_id);
        let _guard = lock.lock().await;

        let response = match self.api.login(identifier, password).await {
            Ok(response) => response,
            // 400 too: older backends validated the password policy on login.
            Err(ApiError::Http {
                status: 400 | 401, ..
            }) => {
                self.store.record_login_failure(telegram_user_id).await?;
                return Err(BotError::user("Usuario o contraseña incorrectos."));
            }
            Err(error) => return Err(error.into()),
        };

        let credentials = match &self.cipher {
            Some(cipher) => Some(
                cipher
                    .encrypt(telegram_user_id, identifier, password)
                    .map_err(|_| BotError::user("No pude guardar las credenciales."))?,
            ),
            None => None,
        };

        self.store
            .upsert_session(&SessionRow {
                telegram_user_id,
                chat_id,
                user_uuid: response.user.uuid,
                username: response.user.username.clone(),
                role: response.user.role.clone(),
                jwt: Some(response.token.clone()),
                jwt_expires_at: jwt_expiry(&response.token),
                credentials,
            })
            .await?;
        self.store.clear_login_failures(telegram_user_id).await?;
        Ok(response)
    }

    /// A valid token for the user, re-logging in with stored credentials when needed.
    pub async fn auth(&self, telegram_user_id: i64) -> BotResult<Auth> {
        let lock = self.lock_for(telegram_user_id);
        let _guard = lock.lock().await;

        let session = self
            .store
            .get_session(telegram_user_id)
            .await?
            .ok_or(BotError::NotLoggedIn)?;
        let now = chrono::Utc::now().timestamp();
        if let (Some(token), Some(expires_at)) = (&session.jwt, session.jwt_expires_at)
            && expires_at > now + RENEW_MARGIN_SECS
        {
            return Ok(Auth {
                telegram_user_id,
                user_uuid: session.user_uuid,
                role: session.role,
                token: token.clone(),
            });
        }

        let (Some(cipher), Some(blob)) = (&self.cipher, &session.credentials) else {
            self.store.delete_session(telegram_user_id).await?;
            return Err(BotError::SessionExpired);
        };
        let Ok(credentials) = cipher.decrypt(telegram_user_id, blob) else {
            // CREDENTIALS_KEY changed or the row was tampered with.
            self.store.delete_session(telegram_user_id).await?;
            return Err(BotError::SessionExpired);
        };

        match self
            .api
            .login(&credentials.identifier, &credentials.password)
            .await
        {
            Ok(response) => {
                let expires_at = jwt_expiry(&response.token).unwrap_or(now + RENEW_MARGIN_SECS * 2);
                self.store
                    .update_token(
                        telegram_user_id,
                        &response.token,
                        expires_at,
                        &response.user.role,
                    )
                    .await?;
                tracing::info!(telegram_user_id, "re-logged in with stored credentials");
                Ok(Auth {
                    telegram_user_id,
                    user_uuid: response.user.uuid,
                    role: response.user.role,
                    token: response.token,
                })
            }
            Err(ApiError::Http {
                status: 400 | 401, ..
            }) => {
                // Password changed or user disabled: forget everything.
                self.store.delete_session(telegram_user_id).await?;
                Err(BotError::SessionExpired)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Runs an API call with the user's token, re-logging in and retrying once on `401`.
    pub async fn call<T, F, Fut>(&self, telegram_user_id: i64, operation: F) -> BotResult<T>
    where
        F: Fn(Auth) -> Fut,
        Fut: Future<Output = Result<T, ApiError>>,
    {
        let auth = self.auth(telegram_user_id).await?;
        match operation(auth).await {
            Err(error) if error.is_unauthorized() => {
                self.store.expire_token(telegram_user_id).await?;
                let auth = self.auth(telegram_user_id).await?;
                match operation(auth).await {
                    Err(error) if error.is_unauthorized() => {
                        self.store.delete_session(telegram_user_id).await?;
                        Err(BotError::SessionExpired)
                    }
                    other => other.map_err(Into::into),
                }
            }
            other => other.map_err(Into::into),
        }
    }

    /// Revokes the token in the backend (best effort) and forgets the session.
    pub async fn logout(&self, telegram_user_id: i64) -> BotResult<()> {
        let lock = self.lock_for(telegram_user_id);
        let _guard = lock.lock().await;
        if let Some(session) = self.store.get_session(telegram_user_id).await?
            && let Some(token) = &session.jwt
            && let Err(error) = self.api.logout(token).await
        {
            tracing::warn!(telegram_user_id, %error, "backend logout failed");
        }
        self.store.delete_session(telegram_user_id).await?;
        Ok(())
    }

    pub async fn has_credentials(&self, telegram_user_id: i64) -> BotResult<bool> {
        Ok(self
            .store
            .get_session(telegram_user_id)
            .await?
            .is_some_and(|session| session.credentials.is_some()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exp_from_jwt_payload() {
        // {"sub":"x","role":"user","exp":1900000000,"jti":"y"}
        let payload =
            URL_SAFE_NO_PAD.encode(br#"{"sub":"x","role":"user","exp":1900000000,"jti":"y"}"#);
        let token = format!("eyJhbGciOiJIUzI1NiJ9.{payload}.signature");
        assert_eq!(jwt_expiry(&token), Some(1_900_000_000));
        assert_eq!(jwt_expiry("not-a-jwt"), None);
    }

    use crate::session::store::memory_store;
    use serde_json::json;
    use std::time::Duration;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    const UID: i64 = 42;
    const USER: &str = "019f527b-6f64-7013-b1c1-8f4c5f1c96db";

    fn jwt(exp: i64) -> String {
        let payload = URL_SAFE_NO_PAD
            .encode(json!({"sub": USER, "role": "user", "exp": exp, "jti": "j"}).to_string());
        format!("h.{payload}.s")
    }

    fn login_ok(token: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "token": token,
            "user": {"uuid": USER, "username": "default", "email": "d@example.com", "role": "user",
                     "email_verified_at": null, "last_login_at": null}
        }))
    }

    async fn manager(server: &MockServer, key: Option<[u8; 32]>) -> SessionManager {
        let api = ApiClient::new(&server.uri(), Duration::from_secs(5)).unwrap();
        SessionManager::new(api, memory_store().await, key)
    }

    fn future() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    #[tokio::test]
    async fn login_stores_session_and_encrypted_credentials() {
        let server = MockServer::start().await;
        let token = jwt(future());
        Mock::given(method("POST"))
            .and(path("/auth/login"))
            .and(body_json(
                json!({"email": "d@example.com", "password": "pw"}),
            ))
            .respond_with(login_ok(&token))
            .expect(1)
            .mount(&server)
            .await;

        let sessions = manager(&server, Some([1; 32])).await;
        sessions
            .login(UID, UID, "d@example.com", "pw")
            .await
            .unwrap();

        let row = sessions.store.get_session(UID).await.unwrap().unwrap();
        assert_eq!(row.jwt.as_deref(), Some(token.as_str()));
        let blob = row.credentials.unwrap();
        assert!(
            !blob.windows(2).any(|w| w == b"pw"),
            "password must not be stored in clear"
        );
        assert!(sessions.has_credentials(UID).await.unwrap());

        let auth = sessions.auth(UID).await.unwrap();
        assert_eq!(auth.token, token);
    }

    #[tokio::test]
    async fn wrong_password_is_counted() {
        let server = MockServer::start().await;
        Mock::given(path("/auth/login"))
            .respond_with(
                ResponseTemplate::new(401).set_body_json(
                    json!({"error": "UNAUTHORIZED", "message": "Invalid credentials"}),
                ),
            )
            .mount(&server)
            .await;
        let sessions = manager(&server, Some([1; 32])).await;
        for _ in 0..LOGIN_FAILURE_LIMIT {
            let error = sessions
                .login(UID, UID, "default", "bad")
                .await
                .unwrap_err();
            assert_eq!(error.user_message(), "Usuario o contraseña incorrectos.");
        }
        assert!(sessions.login_retry_after(UID).await.unwrap().is_some());
        assert!(sessions.store.get_session(UID).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn expired_token_is_renewed_with_stored_credentials() {
        let server = MockServer::start().await;
        let old = jwt(chrono::Utc::now().timestamp() - 10);
        let new = jwt(future());
        Mock::given(path("/auth/login"))
            .respond_with(login_ok(&old))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let sessions = manager(&server, Some([1; 32])).await;
        sessions.login(UID, UID, "default", "pw").await.unwrap();

        server.reset().await;
        Mock::given(path("/auth/login"))
            .and(body_json(json!({"username": "default", "password": "pw"})))
            .respond_with(login_ok(&new))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(sessions.auth(UID).await.unwrap().token, new);
    }

    #[tokio::test]
    async fn call_retries_once_after_401() {
        let server = MockServer::start().await;
        let first = jwt(future());
        let second = jwt(future() + 1);
        Mock::given(path("/auth/login"))
            .respond_with(login_ok(&first))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let sessions = manager(&server, Some([1; 32])).await;
        sessions.login(UID, UID, "default", "pw").await.unwrap();

        Mock::given(path("/auth/login"))
            .respond_with(login_ok(&second))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/users/{USER}")))
            .and(header("authorization", format!("Bearer {first}").as_str()))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/users/{USER}")))
            .and(header("authorization", format!("Bearer {second}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uuid": USER, "username": "default", "email": "d@example.com", "role": "user",
                "email_verified_at": null, "last_login_at": null
            })))
            .expect(1)
            .mount(&server)
            .await;

        let api = sessions.api.clone();
        let user = sessions
            .call(UID, |auth| {
                let api = api.clone();
                async move { api.get_user(&auth.token, auth.user_uuid).await }
            })
            .await
            .unwrap();
        assert_eq!(user.username, "default");
    }

    #[tokio::test]
    async fn relogin_rejected_expires_session() {
        let server = MockServer::start().await;
        Mock::given(path("/auth/login"))
            .respond_with(login_ok(&jwt(chrono::Utc::now().timestamp() - 10)))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let sessions = manager(&server, Some([1; 32])).await;
        sessions.login(UID, UID, "default", "pw").await.unwrap();

        // Password changed in the backend.
        Mock::given(path("/auth/login"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        assert!(matches!(
            sessions.auth(UID).await,
            Err(BotError::SessionExpired)
        ));
        assert!(sessions.store.get_session(UID).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn without_stored_credentials_expiry_asks_for_login() {
        let server = MockServer::start().await;
        Mock::given(path("/auth/login"))
            .respond_with(login_ok(&jwt(chrono::Utc::now().timestamp() - 10)))
            .expect(1)
            .mount(&server)
            .await;
        let sessions = manager(&server, None).await;
        sessions.login(UID, UID, "default", "pw").await.unwrap();
        assert!(!sessions.has_credentials(UID).await.unwrap());
        assert!(matches!(
            sessions.auth(UID).await,
            Err(BotError::SessionExpired)
        ));
    }

    #[tokio::test]
    async fn logout_revokes_token_and_forgets_session() {
        let server = MockServer::start().await;
        let token = jwt(future());
        Mock::given(path("/auth/login"))
            .respond_with(login_ok(&token))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/auth/logout"))
            .and(header("authorization", format!("Bearer {token}").as_str()))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"message": "Logged out"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let sessions = manager(&server, Some([1; 32])).await;
        sessions.login(UID, UID, "default", "pw").await.unwrap();
        sessions.logout(UID).await.unwrap();
        assert!(matches!(
            sessions.auth(UID).await,
            Err(BotError::NotLoggedIn)
        ));
    }
}
