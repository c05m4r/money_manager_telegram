// Copyright (C) 2026 Marcos Gabriel Miller
use std::{
    collections::HashSet,
    future::Future,
    sync::{Arc, Mutex},
};

use chrono::{NaiveDate, Utc};

use crate::{
    api::{ApiClient, ApiError},
    config::{AuthMethod, BotConfig},
    errors::BotResult,
    session::{
        Auth, SessionManager,
        catalog::{Catalog, CatalogCache},
        minter::TokenMinter,
        store::Store,
    },
};

/// Shared dependencies of every handler.
pub struct App {
    pub config: BotConfig,
    /// Needed to parse `/command@bot_username`.
    pub bot_username: String,
    pub api: ApiClient,
    pub store: Store,
    pub sessions: SessionManager,
    pub catalogs: CatalogCache,
    /// Users already told they are not in the allowlist (told only once).
    pub denied_notified: Mutex<HashSet<i64>>,
}

pub type AppRef = Arc<App>;

impl App {
    pub fn new(config: BotConfig, bot_username: String, api: ApiClient, store: Store) -> Self {
        let minter = match &config.auth {
            AuthMethod::Telegram(auth) => Some(TokenMinter::new(auth)),
            AuthMethod::Password => None,
        };
        let sessions =
            SessionManager::new(api.clone(), store.clone(), config.credentials_key, minter);
        let catalogs = CatalogCache::new(config.cache_ttl);
        Self {
            config,
            bot_username,
            api,
            store,
            sessions,
            catalogs,
            denied_notified: Mutex::new(HashSet::new()),
        }
    }

    pub async fn auth(&self, telegram_user_id: i64) -> BotResult<Auth> {
        self.sessions.auth(telegram_user_id).await
    }

    /// API call with the user's token and transparent re-login.
    pub async fn call<T, F, Fut>(&self, telegram_user_id: i64, operation: F) -> BotResult<T>
    where
        F: Fn(Auth) -> Fut,
        Fut: Future<Output = Result<T, ApiError>>,
    {
        self.sessions.call(telegram_user_id, operation).await
    }

    pub async fn catalog(&self, telegram_user_id: i64) -> BotResult<Arc<Catalog>> {
        if let Some(catalog) = self.catalogs.get(telegram_user_id) {
            return Ok(catalog);
        }
        let api = &self.api;
        let catalog = self
            .call(telegram_user_id, |auth| async move {
                let (accounts, categories, currencies, types) = tokio::try_join!(
                    api.list_accounts(&auth.token, auth.user_uuid),
                    api.list_categories(&auth.token, auth.user_uuid),
                    api.list_currencies(&auth.token),
                    api.list_types(&auth.token),
                )?;
                Ok(Catalog::new(accounts, categories, currencies, types))
            })
            .await?;
        Ok(self.catalogs.put(telegram_user_id, catalog))
    }

    pub fn invalidate_catalog(&self, telegram_user_id: i64) {
        self.catalogs.invalidate(telegram_user_id);
    }

    pub fn today(&self) -> NaiveDate {
        Utc::now().with_timezone(&self.config.tz).date_naive()
    }
}
