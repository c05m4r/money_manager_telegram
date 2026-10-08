// Copyright (C) 2026 Marcos Gabriel Miller
//! `AUTH_METHOD=telegram`: signs backend JWTs for mapped Telegram users, no password involved.
//!
//! The backend accepts any HS256 token signed with its `JWT_SECRET` carrying
//! `{sub, role, exp, jti}`, so the bot can issue them without backend changes.
//! Holding `JWT_SECRET` lets the bot act as **any** backend user: treat it like the
//! backend's own secret.
use std::{collections::HashMap, time::Duration};

use chrono::Utc;
use jsonwebtoken::{EncodingKey, Header, encode};
use serde::Serialize;
use uuid::Uuid;

use crate::config::TelegramAuth;

/// Least-privileged role; used to read the user's own profile before knowing the real role.
pub const BOOTSTRAP_ROLE: &str = "user";

#[derive(Serialize)]
struct Claims<'a> {
    sub: String,
    role: &'a str,
    exp: usize,
    jti: String,
}

pub struct TokenMinter {
    key: EncodingKey,
    users: HashMap<i64, Uuid>,
    ttl: Duration,
}

impl TokenMinter {
    pub fn new(auth: &TelegramAuth) -> Self {
        Self {
            key: EncodingKey::from_secret(auth.jwt_secret.as_bytes()),
            users: auth.users.clone(),
            ttl: auth.token_ttl,
        }
    }

    /// Backend user mapped to this Telegram account, if any.
    pub fn user_for(&self, telegram_user_id: i64) -> Option<Uuid> {
        self.users.get(&telegram_user_id).copied()
    }

    /// Returns the token and its `exp` (unix seconds).
    pub fn mint(
        &self,
        user_uuid: Uuid,
        role: &str,
    ) -> Result<(String, i64), jsonwebtoken::errors::Error> {
        self.mint_for(user_uuid, role, self.ttl)
    }

    /// Short-lived, least-privileged token used only to read the user's own profile.
    pub fn mint_bootstrap(&self, user_uuid: Uuid) -> Result<String, jsonwebtoken::errors::Error> {
        self.mint_for(user_uuid, BOOTSTRAP_ROLE, Duration::from_secs(60))
            .map(|(token, _)| token)
    }

    fn mint_for(
        &self,
        user_uuid: Uuid,
        role: &str,
        ttl: Duration,
    ) -> Result<(String, i64), jsonwebtoken::errors::Error> {
        let exp = Utc::now().timestamp() + ttl.as_secs() as i64;
        let claims = Claims {
            sub: user_uuid.to_string(),
            role,
            exp: exp as usize,
            jti: Uuid::new_v4().to_string(),
        };
        Ok((encode(&Header::default(), &claims, &self.key)?, exp))
    }
}

/// Same normalization as the backend (`normalize_role`): unknown roles are `user`.
pub fn normalize_role(role: &str) -> &'static str {
    match role.trim().to_ascii_lowercase().as_str() {
        "admin" => "admin",
        "manager" => "manager",
        "auditor" => "auditor",
        _ => "user",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation, decode};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Decoded {
        sub: String,
        role: String,
        exp: usize,
        jti: String,
    }

    fn minter() -> (TokenMinter, Uuid) {
        let uuid = Uuid::new_v4();
        let auth = TelegramAuth {
            jwt_secret: "s3cret".into(),
            users: HashMap::from([(42, uuid)]),
            token_ttl: Duration::from_secs(600),
        };
        (TokenMinter::new(&auth), uuid)
    }

    #[test]
    fn tokens_validate_like_the_backend() {
        let (minter, uuid) = minter();
        let (token, exp) = minter.mint(uuid, "manager").unwrap();
        // The backend decodes with `Validation::default()` (HS256, exp required).
        let claims = decode::<Decoded>(
            &token,
            &DecodingKey::from_secret(b"s3cret"),
            &Validation::default(),
        )
        .unwrap()
        .claims;
        assert_eq!(claims.sub, uuid.to_string());
        assert_eq!(claims.role, "manager");
        assert_eq!(claims.exp as i64, exp);
        assert!(Uuid::parse_str(&claims.jti).is_ok());
        assert!((exp - Utc::now().timestamp() - 600).abs() <= 2);

        assert!(
            decode::<Decoded>(
                &token,
                &DecodingKey::from_secret(b"other"),
                &Validation::default()
            )
            .is_err()
        );
        assert_ne!(
            minter.mint(uuid, "user").unwrap().0,
            minter.mint(uuid, "user").unwrap().0,
            "unique jti"
        );
    }

    #[test]
    fn mapping_and_roles() {
        let (minter, uuid) = minter();
        assert_eq!(minter.user_for(42), Some(uuid));
        assert_eq!(minter.user_for(7), None);
        assert_eq!(normalize_role(" Admin "), "admin");
        assert_eq!(normalize_role("superuser"), "user");
    }
}
