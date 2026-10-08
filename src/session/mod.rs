// Copyright (C) 2026 Marcos Gabriel Miller
pub mod catalog;
pub mod crypto;
pub mod minter;
pub mod store;

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use uuid::Uuid;

use crate::{
    api::{
        ApiClient, ApiError,
        models::{LoginResponse, User},
    },
    errors::{BotError, BotResult},
};
use crypto::CredentialCipher;
use minter::{TokenMinter, normalize_role};
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
    /// `Some` with `AUTH_METHOD=telegram`: tokens are signed by the bot, no password.
    minter: Option<TokenMinter>,
    /// One lock per user so concurrent updates do not trigger several logins.
    locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
}

impl SessionManager {
    pub fn new(
        api: ApiClient,
        store: Store,
        credentials_key: Option<[u8; 32]>,
        minter: Option<TokenMinter>,
    ) -> Self {
        Self {
            api,
            store,
            cipher: credentials_key.as_ref().map(CredentialCipher::new),
            minter,
            locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn uses_telegram_auth(&self) -> bool {
        self.minter.is_some()
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

    /// `AUTH_METHOD=telegram` login: the Telegram account id is the identity, no password.
    pub async fn login_telegram(&self, telegram_user_id: i64, chat_id: i64) -> BotResult<User> {
        let lock = self.lock_for(telegram_user_id);
        let _guard = lock.lock().await;

        let minter = self
            .minter
            .as_ref()
            .ok_or_else(|| BotError::user("El login por Telegram no está habilitado."))?;
        let user_uuid = minter.user_for(telegram_user_id).ok_or_else(|| {
            BotError::user(format!(
                "Tu cuenta de Telegram no está habilitada en este bot. \
                 Pedile al operador que agregue tu ID {telegram_user_id} a TELEGRAM_USERS."
            ))
        })?;
        let (user, token, expires_at) = self.mint_verified(minter, user_uuid).await?;
        self.store
            .upsert_session(&SessionRow {
                telegram_user_id,
                chat_id,
                user_uuid,
                username: user.username.clone(),
                role: normalize_role(&user.role).to_string(),
                jwt: Some(token),
                jwt_expires_at: Some(expires_at),
                credentials: None,
            })
            .await?;
        tracing::info!(telegram_user_id, %user_uuid, "logged in via Telegram identity");
        Ok(user)
    }

    /// Signs a token with the user's real role, after checking with the backend that the user
    /// exists and is active (signed tokens skip the backend's own login checks).
    async fn mint_verified(
        &self,
        minter: &TokenMinter,
        user_uuid: Uuid,
    ) -> BotResult<(User, String, i64)> {
        let bootstrap = minter
            .mint_bootstrap(user_uuid)
            .map_err(|error| BotError::Token(error.to_string()))?;
        let user = match self.api.get_user(&bootstrap, user_uuid).await {
            Ok(user) => user,
            Err(ApiError::Http { status: 401, .. }) => {
                tracing::error!(
                    "backend rejected a bot-signed token: JWT_SECRET differs from the backend's"
                );
                return Err(BotError::user(
                    "El backend rechazó el token del bot (JWT_SECRET no coincide). Avisale al operador.",
                ));
            }
            Err(ApiError::Http { status: 404, .. }) => {
                return Err(BotError::user(
                    "El usuario configurado para tu Telegram no existe en el backend.",
                ));
            }
            Err(error) => return Err(error.into()),
        };
        if !user.is_active {
            return Err(BotError::user("Tu usuario del backend está desactivado."));
        }
        let (token, expires_at) = minter
            .mint(user_uuid, normalize_role(&user.role))
            .map_err(|error| BotError::Token(error.to_string()))?;
        Ok((user, token, expires_at))
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

        if let Some(minter) = &self.minter {
            // Mapping removed from TELEGRAM_USERS, or pointed to another user: end the session.
            if minter.user_for(telegram_user_id) != Some(session.user_uuid) {
                self.store.delete_session(telegram_user_id).await?;
                return Err(BotError::SessionExpired);
            }
            return match self.mint_verified(minter, session.user_uuid).await {
                Ok((user, token, expires_at)) => {
                    let role = normalize_role(&user.role);
                    self.store
                        .update_token(telegram_user_id, &token, expires_at, role)
                        .await?;
                    Ok(Auth {
                        telegram_user_id,
                        user_uuid: session.user_uuid,
                        role: role.to_string(),
                        token,
                    })
                }
                Err(error @ BotError::User(_)) => {
                    self.store.delete_session(telegram_user_id).await?;
                    Err(error)
                }
                Err(error) => Err(error),
            };
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
            "user": {"uuid": USER, "username": "default", "email": "d@example.com", "role": "user", "is_active": true,
                     "email_verified_at": null, "last_login_at": null}
        }))
    }

    async fn manager(server: &MockServer, key: Option<[u8; 32]>) -> SessionManager {
        let api = ApiClient::new(&server.uri(), Duration::from_secs(5)).unwrap();
        SessionManager::new(api, memory_store().await, key, None)
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
                "uuid": USER, "username": "default", "email": "d@example.com", "role": "user", "is_active": true,
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

    // ---------- AUTH_METHOD=telegram ----------

    use crate::config::TelegramAuth;
    use jsonwebtoken::{DecodingKey, Validation, decode};

    const SECRET: &str = "backend-secret";

    fn telegram_manager_with(
        server: &MockServer,
        store: Store,
        users: &[(i64, &str)],
    ) -> SessionManager {
        let auth = TelegramAuth {
            jwt_secret: SECRET.into(),
            users: users
                .iter()
                .map(|(id, uuid)| (*id, Uuid::parse_str(uuid).unwrap()))
                .collect(),
            token_ttl: Duration::from_secs(600),
        };
        let api = ApiClient::new(&server.uri(), Duration::from_secs(5)).unwrap();
        SessionManager::new(api, store, None, Some(TokenMinter::new(&auth)))
    }

    fn user_body(role: &str, active: bool) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "uuid": USER, "username": "default", "email": "d@example.com", "role": role,
            "is_active": active, "email_verified_at": null, "last_login_at": null
        }))
    }

    fn role_of(token: &str) -> String {
        let mut validation = Validation::default();
        validation.validate_exp = true;
        decode::<serde_json::Value>(
            token,
            &DecodingKey::from_secret(SECRET.as_bytes()),
            &validation,
        )
        .unwrap()
        .claims["role"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn telegram_login_signs_token_with_backend_role() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/users/{USER}")))
            .respond_with(user_body("Manager", true))
            .expect(1)
            .mount(&server)
            .await;
        let sessions = telegram_manager_with(&server, memory_store().await, &[(UID, USER)]);

        let user = sessions.login_telegram(UID, UID).await.unwrap();
        assert_eq!(user.username, "default");
        let auth = sessions.auth(UID).await.unwrap();
        assert_eq!(auth.role, "manager");
        assert_eq!(
            role_of(&auth.token),
            "manager",
            "token carries the backend role"
        );
        assert!(
            !sessions.has_credentials(UID).await.unwrap(),
            "no password stored"
        );

        // The profile lookup used a least-privileged bootstrap token.
        let request = &server.received_requests().await.unwrap()[0];
        let bearer = request
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(role_of(bearer.trim_start_matches("Bearer ")), "user");
    }

    #[tokio::test]
    async fn telegram_login_rejects_unmapped_inactive_and_bad_secret() {
        let server = MockServer::start().await;
        let sessions = telegram_manager_with(&server, memory_store().await, &[(UID, USER)]);
        let error = sessions.login_telegram(7, 7).await.unwrap_err();
        assert!(
            error.user_message().contains("TELEGRAM_USERS"),
            "{}",
            error.user_message()
        );
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "unmapped users never reach the backend"
        );

        Mock::given(path(format!("/users/{USER}")))
            .respond_with(user_body("user", false))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let error = sessions.login_telegram(UID, UID).await.unwrap_err();
        assert_eq!(
            error.user_message(),
            "Tu usuario del backend está desactivado."
        );

        Mock::given(path(format!("/users/{USER}")))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let error = sessions.login_telegram(UID, UID).await.unwrap_err();
        assert!(error.user_message().contains("JWT_SECRET"));
        assert!(sessions.store.get_session(UID).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn telegram_session_renews_and_ends_when_unmapped() {
        let server = MockServer::start().await;
        Mock::given(path(format!("/users/{USER}")))
            .respond_with(user_body("user", true))
            .mount(&server)
            .await;
        let store = memory_store().await;
        let sessions = telegram_manager_with(&server, store.clone(), &[(UID, USER)]);
        sessions.login_telegram(UID, UID).await.unwrap();
        let first = sessions.auth(UID).await.unwrap().token;

        store.expire_token(UID).await.unwrap();
        let renewed = sessions.auth(UID).await.unwrap().token;
        assert_ne!(
            first, renewed,
            "expired token is re-signed without any password"
        );

        // Operator removed the user from TELEGRAM_USERS and restarted the bot.
        store.expire_token(UID).await.unwrap();
        let other = telegram_manager_with(&server, store.clone(), &[(99, USER)]);
        assert!(matches!(
            other.auth(UID).await,
            Err(BotError::SessionExpired)
        ));
        assert!(store.get_session(UID).await.unwrap().is_none());
    }
}
