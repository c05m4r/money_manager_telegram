// Copyright (C) 2026 Marcos Gabriel Miller
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use uuid::Uuid;

use crate::api::models::{Account, Category, Currency, TransactionType};

pub const INCOME_CODE: &str = "IN";
pub const EXPENSE_CODE: &str = "OUT";

/// Per-user reference data the bot needs to resolve names and build keyboards.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub accounts: Vec<Account>,
    pub categories: Vec<Category>,
    pub currencies: Vec<Currency>,
    pub types: Vec<TransactionType>,
}

impl Catalog {
    pub fn new(
        mut accounts: Vec<Account>,
        mut categories: Vec<Category>,
        mut currencies: Vec<Currency>,
        types: Vec<TransactionType>,
    ) -> Self {
        accounts.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        categories.sort_by_key(|c| c.name.to_lowercase());
        currencies.sort_by(|a, b| a.code.cmp(&b.code));
        Self {
            accounts,
            categories,
            currencies,
            types,
        }
    }

    pub fn type_by_code(&self, code: &str) -> Option<&TransactionType> {
        self.types
            .iter()
            .find(|t| t.code.eq_ignore_ascii_case(code))
    }

    pub fn type_by_uuid(&self, uuid: Uuid) -> Option<&TransactionType> {
        self.types.iter().find(|t| t.uuid == uuid)
    }

    pub fn account(&self, uuid: Uuid) -> Option<&Account> {
        self.accounts.iter().find(|a| a.uuid == uuid)
    }

    pub fn category(&self, uuid: Uuid) -> Option<&Category> {
        self.categories.iter().find(|c| c.uuid == uuid)
    }

    pub fn currency(&self, code: &str) -> Option<&Currency> {
        self.currencies
            .iter()
            .find(|c| c.code.eq_ignore_ascii_case(code))
    }

    /// The account flagged as default, or the only/first one.
    pub fn default_account(&self) -> Option<&Account> {
        self.accounts
            .iter()
            .find(|a| a.is_default)
            .or_else(|| self.accounts.first())
    }

    /// Currency symbol for an account, falling back to its code.
    pub fn symbol_for_account(&self, account_uuid: Option<Uuid>) -> (String, String) {
        let code = account_uuid
            .and_then(|uuid| self.account(uuid))
            .map(|a| a.currency_code.clone())
            .unwrap_or_default();
        let symbol = self
            .currency(&code)
            .map(|c| c.symbol.clone())
            .unwrap_or_else(|| code.clone());
        (symbol, code)
    }
}

/// In-memory TTL cache of catalogs, keyed by Telegram user id.
pub struct CatalogCache {
    ttl: Duration,
    entries: Mutex<HashMap<i64, (Arc<Catalog>, Instant)>>,
}

impl CatalogCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, telegram_user_id: i64) -> Option<Arc<Catalog>> {
        let entries = self.entries.lock().expect("catalog cache poisoned");
        entries
            .get(&telegram_user_id)
            .filter(|(_, loaded_at)| loaded_at.elapsed() < self.ttl)
            .map(|(catalog, _)| catalog.clone())
    }

    pub fn put(&self, telegram_user_id: i64, catalog: Catalog) -> Arc<Catalog> {
        let catalog = Arc::new(catalog);
        self.entries
            .lock()
            .expect("catalog cache poisoned")
            .insert(telegram_user_id, (catalog.clone(), Instant::now()));
        catalog
    }

    pub fn invalidate(&self, telegram_user_id: i64) {
        self.entries
            .lock()
            .expect("catalog cache poisoned")
            .remove(&telegram_user_id);
    }
}
