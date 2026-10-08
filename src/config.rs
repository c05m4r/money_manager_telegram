// Copyright (C) 2026 Marcos Gabriel Miller
use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono_tz::Tz;
use uuid::Uuid;

const DEFAULT_BACKEND_API_URL: &str = "http://127.0.0.1:8000/api/v1";

/// How a Telegram user gets a backend session (`AUTH_METHOD`).
#[derive(Clone)]
pub enum AuthMethod {
    /// `/login` asks for the backend username/email and password.
    Password,
    /// No password: the Telegram account id is the identity and the bot signs backend JWTs.
    Telegram(TelegramAuth),
}

#[derive(Clone)]
pub struct TelegramAuth {
    /// Same value as the backend's `JWT_SECRET`.
    pub jwt_secret: String,
    /// Telegram user id → backend user UUID (`TELEGRAM_USERS`).
    pub users: HashMap<i64, Uuid>,
    /// Lifetime of each token the bot signs.
    pub token_ttl: Duration,
}

impl AuthMethod {
    pub fn name(&self) -> &'static str {
        match self {
            AuthMethod::Password => "password",
            AuthMethod::Telegram(_) => "telegram",
        }
    }
}

#[derive(Clone)]
pub struct BotConfig {
    pub telegram_token: String,
    pub auth: AuthMethod,
    pub backend_api_url: String,
    /// Empty means every Telegram user is allowed.
    pub allowed_telegram_ids: HashSet<i64>,
    pub database_path: PathBuf,
    pub tz: Tz,
    /// `Some` when credentials are stored (encrypted) for automatic re-login.
    pub credentials_key: Option<[u8; 32]>,
    pub cache_ttl: Duration,
    pub page_size: u64,
    pub http_timeout: Duration,
}

impl fmt::Debug for BotConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BotConfig")
            .field("telegram_token", &"***")
            .field("auth", &self.auth.name())
            .field("backend_api_url", &self.backend_api_url)
            .field("allowed_telegram_ids", &self.allowed_telegram_ids)
            .field("database_path", &self.database_path)
            .field("tz", &self.tz)
            .field("credentials_key", &self.credentials_key.map(|_| "***"))
            .field("cache_ttl", &self.cache_ttl)
            .field("page_size", &self.page_size)
            .field("http_timeout", &self.http_timeout)
            .finish()
    }
}

impl BotConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Builds the config from any variable source, so it can be tested without touching the process env.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let var = |name: &str| {
            get(name)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };

        let telegram_token = var("TELOXIDE_TOKEN").ok_or("TELOXIDE_TOKEN is required")?;
        let backend_api_url = var("BACKEND_API_URL")
            .unwrap_or_else(|| DEFAULT_BACKEND_API_URL.to_string())
            .trim_end_matches('/')
            .to_string();

        let allowed_telegram_ids = match var("ALLOWED_TELEGRAM_IDS") {
            Some(list) => list
                .split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|id| {
                    id.parse::<i64>()
                        .map_err(|_| format!("ALLOWED_TELEGRAM_IDS contains an invalid id: {id}"))
                })
                .collect::<Result<HashSet<_>, _>>()?,
            None => HashSet::new(),
        };

        let database_path =
            PathBuf::from(var("DATABASE_PATH").unwrap_or_else(|| "data/bot.sqlite".to_string()));

        let tz_name = var("BOT_TZ").unwrap_or_else(|| "America/Argentina/Buenos_Aires".to_string());
        let tz = tz_name
            .parse::<Tz>()
            .map_err(|_| format!("BOT_TZ is not a valid IANA time zone: {tz_name}"))?;

        let auth = match var("AUTH_METHOD")
            .as_deref()
            .map(str::to_lowercase)
            .as_deref()
        {
            None | Some("password") => AuthMethod::Password,
            Some("telegram") => AuthMethod::Telegram(TelegramAuth {
                jwt_secret: var("JWT_SECRET").ok_or(
                    "JWT_SECRET is required when AUTH_METHOD=telegram (same value as the backend)",
                )?,
                users: parse_telegram_users(
                    &var("TELEGRAM_USERS")
                        .ok_or("TELEGRAM_USERS is required when AUTH_METHOD=telegram")?,
                )?,
                token_ttl: Duration::from_secs(
                    60 * match var("TELEGRAM_TOKEN_TTL_MINUTES") {
                        Some(value) => value
                            .parse::<u64>()
                            .ok()
                            .filter(|minutes| (1..=1440).contains(minutes))
                            .ok_or("TELEGRAM_TOKEN_TTL_MINUTES must be between 1 and 1440")?,
                        None => 60,
                    },
                ),
            }),
            Some(other) => {
                return Err(format!(
                    "AUTH_METHOD must be password or telegram, got {other}"
                ));
            }
        };

        // Stored credentials only make sense for the password method.
        let store_credentials = match var("STORE_CREDENTIALS").as_deref() {
            None | Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            Some(other) => {
                return Err(format!(
                    "STORE_CREDENTIALS must be true or false, got {other}"
                ));
            }
        };
        let credentials_key = if store_credentials && matches!(auth, AuthMethod::Password) {
            let encoded = var("CREDENTIALS_KEY").ok_or(
                "CREDENTIALS_KEY is required when STORE_CREDENTIALS=true (openssl rand -base64 32)",
            )?;
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| "CREDENTIALS_KEY must be valid base64")?;
            let key: [u8; 32] = bytes
                .try_into()
                .map_err(|_| "CREDENTIALS_KEY must decode to exactly 32 bytes")?;
            Some(key)
        } else {
            None
        };

        let number = |name: &str, default: u64| -> Result<u64, String> {
            match var(name) {
                Some(value) => value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or(format!("{name} must be a positive integer")),
                None => Ok(default),
            }
        };

        Ok(Self {
            telegram_token,
            auth,
            backend_api_url,
            allowed_telegram_ids,
            database_path,
            tz,
            credentials_key,
            cache_ttl: Duration::from_secs(number("CACHE_TTL_SECS", 300)?),
            page_size: number("PAGE_SIZE", 10)?.min(100),
            http_timeout: Duration::from_secs(number("HTTP_TIMEOUT_SECS", 10)?),
        })
    }

    pub fn is_allowed(&self, telegram_user_id: i64) -> bool {
        self.allowed_telegram_ids.is_empty()
            || self.allowed_telegram_ids.contains(&telegram_user_id)
    }

    /// True when the backend URL is plain HTTP to a host outside the local machine/network.
    pub fn backend_is_insecure(&self) -> bool {
        let Some(rest) = self.backend_api_url.strip_prefix("http://") else {
            return false;
        };
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        !(host == "localhost" || host == "127.0.0.1" || host == "::1" || !host.contains('.'))
    }
}

/// `123456789:<uuid>,987654321:<uuid>`
fn parse_telegram_users(list: &str) -> Result<HashMap<i64, Uuid>, String> {
    let users = list
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (id, uuid) = entry.split_once(':').ok_or(format!(
                "TELEGRAM_USERS entry must be <telegram_id>:<user_uuid>, got {entry}"
            ))?;
            let id = id
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("TELEGRAM_USERS has an invalid Telegram id: {id}"))?;
            let uuid = Uuid::parse_str(uuid.trim())
                .map_err(|_| format!("TELEGRAM_USERS has an invalid user UUID: {uuid}"))?;
            Ok((id, uuid))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    if users.is_empty() {
        return Err("TELEGRAM_USERS must map at least one Telegram id".into());
    }
    Ok(users)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

    #[test]
    fn defaults() {
        let config =
            BotConfig::from_lookup(lookup(&[("TELOXIDE_TOKEN", "t"), ("CREDENTIALS_KEY", KEY)]))
                .unwrap();
        assert_eq!(config.backend_api_url, DEFAULT_BACKEND_API_URL);
        assert!(config.allowed_telegram_ids.is_empty());
        assert!(config.is_allowed(42));
        assert_eq!(config.tz, chrono_tz::America::Argentina::Buenos_Aires);
        assert!(config.credentials_key.is_some());
        assert_eq!(config.page_size, 10);
    }

    #[test]
    fn missing_token_fails() {
        assert!(BotConfig::from_lookup(lookup(&[])).is_err());
    }

    #[test]
    fn credentials_key_required_unless_disabled() {
        assert!(BotConfig::from_lookup(lookup(&[("TELOXIDE_TOKEN", "t")])).is_err());
        let config = BotConfig::from_lookup(lookup(&[
            ("TELOXIDE_TOKEN", "t"),
            ("STORE_CREDENTIALS", "false"),
        ]))
        .unwrap();
        assert!(config.credentials_key.is_none());
        assert!(
            BotConfig::from_lookup(lookup(&[
                ("TELOXIDE_TOKEN", "t"),
                ("CREDENTIALS_KEY", "c2hvcnQ=")
            ]))
            .is_err()
        );
    }

    #[test]
    fn allowlist_and_validation() {
        let config = BotConfig::from_lookup(lookup(&[
            ("TELOXIDE_TOKEN", "t"),
            ("STORE_CREDENTIALS", "false"),
            ("ALLOWED_TELEGRAM_IDS", "1, 2,3"),
            ("BACKEND_API_URL", "https://api.example.com/api/v1/"),
        ]))
        .unwrap();
        assert!(config.is_allowed(2));
        assert!(!config.is_allowed(4));
        assert_eq!(config.backend_api_url, "https://api.example.com/api/v1");
        assert!(
            BotConfig::from_lookup(lookup(&[
                ("TELOXIDE_TOKEN", "t"),
                ("STORE_CREDENTIALS", "false"),
                ("ALLOWED_TELEGRAM_IDS", "1,x"),
            ]))
            .is_err()
        );
        assert!(
            BotConfig::from_lookup(lookup(&[
                ("TELOXIDE_TOKEN", "t"),
                ("STORE_CREDENTIALS", "false"),
                ("BOT_TZ", "Mars/Olympus"),
            ]))
            .is_err()
        );
    }

    #[test]
    fn insecure_backend_detection() {
        let mut config = BotConfig::from_lookup(lookup(&[
            ("TELOXIDE_TOKEN", "t"),
            ("STORE_CREDENTIALS", "false"),
        ]))
        .unwrap();
        assert!(!config.backend_is_insecure());
        config.backend_api_url = "http://backend:8000/api/v1".into();
        assert!(!config.backend_is_insecure());
        config.backend_api_url = "http://api.example.com/api/v1".into();
        assert!(config.backend_is_insecure());
        config.backend_api_url = "https://api.example.com/api/v1".into();
        assert!(!config.backend_is_insecure());
    }

    #[test]
    fn telegram_auth_method() {
        let uuid = "019f527b-6f64-7013-b1c1-8f4c5f1c96db";
        let users = format!("123:{uuid}, 456:{uuid}");
        let config = BotConfig::from_lookup(lookup(&[
            ("TELOXIDE_TOKEN", "t"),
            ("AUTH_METHOD", "telegram"),
            ("JWT_SECRET", "s3cret"),
            ("TELEGRAM_USERS", &users),
        ]))
        .unwrap();
        let AuthMethod::Telegram(auth) = &config.auth else {
            panic!("expected telegram auth");
        };
        assert_eq!(auth.users.len(), 2);
        assert_eq!(auth.users[&123].to_string(), uuid);
        assert_eq!(auth.token_ttl, Duration::from_secs(3600));
        assert!(
            config.credentials_key.is_none(),
            "no CREDENTIALS_KEY needed"
        );
        assert!(!format!("{config:?}").contains("s3cret"));

        let base = [("TELOXIDE_TOKEN", "t"), ("AUTH_METHOD", "telegram")];
        assert!(
            BotConfig::from_lookup(lookup(&base)).is_err(),
            "JWT_SECRET required"
        );
        let with_secret = [base[0], base[1], ("JWT_SECRET", "s")];
        assert!(
            BotConfig::from_lookup(lookup(&with_secret)).is_err(),
            "TELEGRAM_USERS required"
        );
        for bad in ["123", "x:uuid", &format!("123:{uuid}x"), " , "] {
            let pairs = [
                with_secret[0],
                with_secret[1],
                with_secret[2],
                ("TELEGRAM_USERS", bad),
            ];
            assert!(BotConfig::from_lookup(lookup(&pairs)).is_err(), "{bad}");
        }
        assert!(
            BotConfig::from_lookup(lookup(&[("TELOXIDE_TOKEN", "t"), ("AUTH_METHOD", "magic")]))
                .is_err()
        );
    }
}
