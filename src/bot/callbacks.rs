// Copyright (C) 2026 Marcos Gabriel Miller
//! Inline button payloads. Telegram limits `callback_data` to 64 bytes; a UUID takes 36.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_CALLBACK_BYTES: usize = 64;
/// Longest category name that fits in a `nc:` payload.
pub const MAX_NEW_CATEGORY_BYTES: usize = 40;

macro_rules! field_enum {
    ($name:ident { $($variant:ident = $code:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub fn code(self) -> &'static str {
                match self { $(Self::$variant => $code),+ }
            }

            pub fn from_code(code: &str) -> Option<Self> {
                match code { $($code => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

field_enum!(TxField { Amount = "a", Type = "t", Category = "c", Account = "u", Date = "d", Description = "s" });
field_enum!(AccountField { Name = "n", Description = "d", Currency = "c" });
field_enum!(CategoryField { Name = "n", Description = "d" });
field_enum!(CurrencyField { Name = "n", Symbol = "y", Description = "d" });
field_enum!(TypeField { Name = "n", Code = "c", Description = "d" });

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Transactions,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Callback {
    /// Choice inside the current dialogue (type, category, account, currency…). `None` = "none".
    Pick(Option<String>),
    PickToday,
    PickYesterday,
    Skip,
    Confirm,
    Abort,
    Page(ListKind, u64),
    TxEdit(Uuid),
    TxEditField(TxField, Uuid),
    TxDelete(Uuid),
    TxDeleteYes(Uuid),
    AccountEditField(AccountField, Uuid),
    AccountDeleteYes(Uuid),
    CategoryEditField(CategoryField, Uuid),
    CategoryDeleteYes(Uuid),
    CurrencyEditField(CurrencyField, String),
    CurrencyDeleteYes(String),
    TypeEditField(TypeField, Uuid),
    TypeDeleteYes(Uuid),
    NewCategory(String),
    LogoutYes,
}

impl Callback {
    pub fn encode(&self) -> String {
        let data = match self {
            Callback::Pick(Some(value)) => format!("pk:{value}"),
            Callback::Pick(None) => "pk:".into(),
            Callback::PickToday => "wd:t".into(),
            Callback::PickYesterday => "wd:y".into(),
            Callback::Skip => "sk".into(),
            Callback::Confirm => "ok".into(),
            Callback::Abort => "no".into(),
            Callback::Page(ListKind::Transactions, page) => format!("pg:tx:{page}"),
            Callback::TxEdit(uuid) => format!("te:{uuid}"),
            Callback::TxEditField(field, uuid) => format!("tf:{}:{uuid}", field.code()),
            Callback::TxDelete(uuid) => format!("td:{uuid}"),
            Callback::TxDeleteYes(uuid) => format!("tdy:{uuid}"),
            Callback::AccountEditField(field, uuid) => format!("af:{}:{uuid}", field.code()),
            Callback::AccountDeleteYes(uuid) => format!("ady:{uuid}"),
            Callback::CategoryEditField(field, uuid) => format!("cf:{}:{uuid}", field.code()),
            Callback::CategoryDeleteYes(uuid) => format!("cdy:{uuid}"),
            Callback::CurrencyEditField(field, code) => format!("yf:{}:{code}", field.code()),
            Callback::CurrencyDeleteYes(code) => format!("ydy:{code}"),
            Callback::TypeEditField(field, uuid) => format!("kf:{}:{uuid}", field.code()),
            Callback::TypeDeleteYes(uuid) => format!("kdy:{uuid}"),
            Callback::NewCategory(name) => format!("nc:{name}"),
            Callback::LogoutYes => "lo:y".into(),
        };
        debug_assert!(
            data.len() <= MAX_CALLBACK_BYTES,
            "callback data too long: {data}"
        );
        data
    }

    pub fn decode<'a>(data: &'a str) -> Option<Self> {
        let uuid = |value: &str| Uuid::parse_str(value).ok();
        let (tag, rest) = data.split_once(':').unwrap_or((data, ""));
        let field_and = |rest: &'a str| rest.split_once(':');
        Some(match tag {
            "pk" => Callback::Pick((!rest.is_empty()).then(|| rest.to_string())),
            "wd" if rest == "t" => Callback::PickToday,
            "wd" if rest == "y" => Callback::PickYesterday,
            "sk" => Callback::Skip,
            "ok" => Callback::Confirm,
            "no" => Callback::Abort,
            "pg" => {
                let (kind, page) = rest.split_once(':')?;
                match kind {
                    "tx" => Callback::Page(ListKind::Transactions, page.parse().ok()?),
                    _ => return None,
                }
            }
            "te" => Callback::TxEdit(uuid(rest)?),
            "tf" => {
                let (field, id) = field_and(rest)?;
                Callback::TxEditField(TxField::from_code(field)?, uuid(id)?)
            }
            "td" => Callback::TxDelete(uuid(rest)?),
            "tdy" => Callback::TxDeleteYes(uuid(rest)?),
            "af" => {
                let (field, id) = field_and(rest)?;
                Callback::AccountEditField(AccountField::from_code(field)?, uuid(id)?)
            }
            "ady" => Callback::AccountDeleteYes(uuid(rest)?),
            "cf" => {
                let (field, id) = field_and(rest)?;
                Callback::CategoryEditField(CategoryField::from_code(field)?, uuid(id)?)
            }
            "cdy" => Callback::CategoryDeleteYes(uuid(rest)?),
            "yf" => {
                let (field, code) = field_and(rest)?;
                Callback::CurrencyEditField(CurrencyField::from_code(field)?, code.to_string())
            }
            "ydy" => Callback::CurrencyDeleteYes(rest.to_string()),
            "kf" => {
                let (field, id) = field_and(rest)?;
                Callback::TypeEditField(TypeField::from_code(field)?, uuid(id)?)
            }
            "kdy" => Callback::TypeDeleteYes(uuid(rest)?),
            "nc" if !rest.is_empty() => Callback::NewCategory(rest.to_string()),
            "lo" if rest == "y" => Callback::LogoutYes,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_size() {
        let id = Uuid::new_v4();
        let all = [
            Callback::Pick(Some(id.to_string())),
            Callback::Pick(Some("USD".into())),
            Callback::Pick(None),
            Callback::PickToday,
            Callback::PickYesterday,
            Callback::Skip,
            Callback::Confirm,
            Callback::Abort,
            Callback::Page(ListKind::Transactions, 12),
            Callback::TxEdit(id),
            Callback::TxEditField(TxField::Description, id),
            Callback::TxDelete(id),
            Callback::TxDeleteYes(id),
            Callback::AccountEditField(AccountField::Currency, id),
            Callback::AccountDeleteYes(id),
            Callback::CategoryEditField(CategoryField::Name, id),
            Callback::CategoryDeleteYes(id),
            Callback::CurrencyEditField(CurrencyField::Symbol, "USD".into()),
            Callback::CurrencyDeleteYes("USD".into()),
            Callback::TypeEditField(TypeField::Code, id),
            Callback::TypeDeleteYes(id),
            Callback::NewCategory("x".repeat(MAX_NEW_CATEGORY_BYTES)),
            Callback::LogoutYes,
        ];
        for callback in all {
            let data = callback.encode();
            assert!(data.len() <= MAX_CALLBACK_BYTES, "{data}");
            assert_eq!(Callback::decode(&data), Some(callback), "{data}");
        }
    }

    #[test]
    fn rejects_garbage() {
        for data in [
            "",
            "zz",
            "te:not-a-uuid",
            "tf:q:00000000-0000-0000-0000-000000000000",
            "pg:xx:1",
            "nc:",
            "wd:z",
        ] {
            assert_eq!(Callback::decode(data), None, "{data}");
        }
    }
}
