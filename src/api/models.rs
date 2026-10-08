// Copyright (C) 2026 Marcos Gabriel Miller
//! Mirrors of `money_manager_backend/src/models`. The bot is an HTTP client, so these are
//! duplicated on purpose instead of sharing a crate with the backend.
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize, Serializer};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct Meta {
    pub total_records: u64,
    pub current_page: u64,
    pub total_pages: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Paginated<T> {
    pub data: Vec<T>,
    pub meta: Meta,
}

// ---------- auth / users ----------

#[derive(Serialize)]
pub struct LoginRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<&'a str>,
    pub password: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: User,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub uuid: Uuid,
    pub username: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub last_login_at: Option<DateTime<Utc>>,
}

// ---------- currencies ----------

#[derive(Debug, Clone, Deserialize)]
pub struct Currency {
    pub code: String,
    pub name: String,
    pub symbol: String,
}

#[derive(Debug, Serialize)]
pub struct CreateCurrency {
    pub code: String,
    pub name: String,
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct UpdateCurrency {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ---------- transaction types ----------

#[derive(Debug, Clone, Deserialize)]
pub struct TransactionType {
    pub uuid: Uuid,
    pub name: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct CreateTransactionType {
    pub name: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct UpdateTransactionType {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ---------- accounts ----------

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub uuid: Uuid,
    pub name: String,
    pub is_default: bool,
    pub currency_code: String,
}

#[derive(Debug, Serialize)]
pub struct CreateAccount {
    pub user_uuid: Uuid,
    pub name: String,
    pub currency_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
}

#[derive(Debug, Default, Serialize)]
pub struct UpdateAccount {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
}

// ---------- categories ----------

#[derive(Debug, Clone, Deserialize)]
pub struct Category {
    pub uuid: Uuid,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateCategory {
    pub user_uuid: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct UpdateCategory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ---------- transactions ----------

#[derive(Debug, Clone, Deserialize)]
pub struct Transaction {
    pub uuid: Uuid,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount: Decimal,
    pub description: Option<String>,
    pub transaction_date: DateTime<Utc>,
    pub type_uuid: Uuid,
    pub category_uuid: Option<Uuid>,
    pub account_uuid: Option<Uuid>,
}

fn decimal_as_string<S: Serializer>(value: &Decimal, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.normalize().to_string())
}

fn optional_decimal_as_string<S: Serializer>(
    value: &Option<Decimal>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => decimal_as_string(value, serializer),
        None => serializer.serialize_none(),
    }
}

#[derive(Debug, Serialize)]
pub struct CreateTransaction {
    #[serde(serialize_with = "decimal_as_string")]
    pub amount: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub transaction_date: DateTime<Utc>,
    pub type_uuid: Uuid,
    pub category_uuid: Option<Uuid>,
    pub account_uuid: Uuid,
}

/// PATCH body. `category_uuid`/`account_uuid`: `None` = keep, `Some(None)` = send `null` (unlink).
#[derive(Debug, Default, Serialize)]
pub struct UpdateTransaction {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "optional_decimal_as_string"
    )]
    pub amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_date: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_uuid: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_uuid: Option<Option<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_uuid: Option<Option<Uuid>>,
}

/// Query params of `GET /transactions`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TransactionQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_uuid: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_uuid: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_uuid: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_from: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_to: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
}

// ---------- reports ----------

#[derive(Debug, Clone, Deserialize)]
pub struct AccountBalance {
    pub account_uuid: Uuid,
    pub account_name: String,
    pub currency_code: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub income: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub expense: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub other: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub balance: Decimal,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CurrencyBalance {
    pub currency_code: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub balance: Decimal,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BalanceReport {
    pub data: Vec<AccountBalance>,
    pub totals: Vec<CurrencyBalance>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CategoryTotal {
    pub category_uuid: Option<Uuid>,
    pub category_name: Option<String>,
    pub currency_code: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub total: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub percentage: Decimal,
    pub transaction_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CategoryReport {
    pub data: Vec<CategoryTotal>,
}

#[derive(Debug, Clone, Default)]
pub struct CategoryReportQuery {
    pub account_uuid: Option<Uuid>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub type_code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn update_transaction_serializes_partial_and_null() {
        let body = UpdateTransaction {
            amount: Some(Decimal::new(150050, 2)),
            category_uuid: Some(None),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            json!({"amount": "1500.5", "category_uuid": null})
        );
        assert_eq!(
            serde_json::to_value(UpdateTransaction::default()).unwrap(),
            json!({})
        );
    }

    #[test]
    fn transaction_amount_parses_from_string() {
        let tx: Transaction = serde_json::from_value(json!({
            "uuid": "019f527b-7c68-79e0-aa5d-a3f41893c527",
            "amount": "100.50",
            "description": null,
            "transaction_date": "2026-10-01T12:00:00Z",
            "type_uuid": "019f527b-7c68-79e0-aa5d-a3f41893c527",
            "category_uuid": null,
            "account_uuid": null,
            "created_at": "2026-10-01T12:00:00Z"
        }))
        .unwrap();
        assert_eq!(tx.amount, Decimal::new(10050, 2));
    }
}
