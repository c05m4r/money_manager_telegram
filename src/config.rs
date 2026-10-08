// Copyright (C) 2026 Marcos Gabriel Miller
use std::{collections::HashSet, fmt, path::PathBuf, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono_tz::Tz;

const DEFAULT_BACKEND_API_URL: &str = "http://127.0.0.1:8000/api/v1";

#[derive(Clone)]
pub struct BotConfig {
    pub telegram_token: String,
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

        let store_credentials = match var("STORE_CREDENTIALS").as_deref() {
            None | Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            Some(other) => {
                return Err(format!(
                    "STORE_CREDENTIALS must be true or false, got {other}"
                ));
            }
        };
        let credentials_key = if store_credentials {
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
}
