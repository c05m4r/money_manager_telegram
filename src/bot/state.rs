// Copyright (C) 2026 Marcos Gabriel Miller
//! Dialogue states, persisted as JSON by teloxide's `SqliteStorage`.
//! Never put secrets here: the password is read and dropped inside the handler.
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::callbacks::{AccountField, CategoryField, CurrencyField, TxField, TypeField};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum State {
    #[default]
    Idle,
    LoginAskIdentifier,
    LoginAskPassword {
        identifier: String,
    },
    TxWizard {
        draft: TxDraft,
    },
    /// Waiting for a typed value (amount, date, description).
    TxEditValue {
        uuid: Uuid,
        field: TxField,
    },
    /// Waiting for a button pick (type, category, account).
    TxEditPick {
        uuid: Uuid,
        field: TxField,
    },
    AccountCreate {
        name: Option<String>,
    },
    AccountEditValue {
        uuid: Uuid,
        field: AccountField,
    },
    CategoryCreate,
    CategoryEditValue {
        uuid: Uuid,
        field: CategoryField,
    },
    CurrencyEditValue {
        code: String,
        field: CurrencyField,
    },
    TypeEditValue {
        uuid: Uuid,
        field: TypeField,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxStep {
    #[default]
    Type,
    Amount,
    Category,
    Account,
    Date,
    Description,
    Confirm,
}

/// A transaction being built by `/new` or completed after a partial `/expense`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TxDraft {
    pub step: TxStep,
    pub type_uuid: Option<Uuid>,
    pub amount: Option<Decimal>,
    /// `Some(None)` = explicitly without category.
    pub category: Option<Option<Uuid>>,
    pub account: Option<Uuid>,
    pub date: Option<NaiveDate>,
    /// `Some(None)` = explicitly without description.
    pub description: Option<Option<String>>,
    /// Started from `/expense`/`/income`: create right after the missing piece, no confirmation.
    pub quick: bool,
}

impl TxDraft {
    /// First step that still needs input, or `Confirm` (or `None` for quick drafts) when complete.
    pub fn next_step(&self) -> Option<TxStep> {
        let step = if self.type_uuid.is_none() {
            TxStep::Type
        } else if self.amount.is_none() {
            TxStep::Amount
        } else if self.category.is_none() {
            TxStep::Category
        } else if self.account.is_none() {
            TxStep::Account
        } else if self.date.is_none() && !self.quick {
            TxStep::Date
        } else if self.description.is_none() && !self.quick {
            TxStep::Description
        } else if self.quick {
            return None;
        } else {
            TxStep::Confirm
        };
        Some(step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_steps_in_order() {
        let mut draft = TxDraft::default();
        assert_eq!(draft.next_step(), Some(TxStep::Type));
        draft.type_uuid = Some(Uuid::nil());
        assert_eq!(draft.next_step(), Some(TxStep::Amount));
        draft.amount = Some(Decimal::ONE);
        assert_eq!(draft.next_step(), Some(TxStep::Category));
        draft.category = Some(None);
        assert_eq!(draft.next_step(), Some(TxStep::Account));
        draft.account = Some(Uuid::nil());
        assert_eq!(draft.next_step(), Some(TxStep::Date));
        draft.date = NaiveDate::from_ymd_opt(2026, 10, 8);
        assert_eq!(draft.next_step(), Some(TxStep::Description));
        draft.description = Some(None);
        assert_eq!(draft.next_step(), Some(TxStep::Confirm));
    }

    #[test]
    fn quick_draft_finishes_without_date_or_description() {
        let draft = TxDraft {
            type_uuid: Some(Uuid::nil()),
            amount: Some(Decimal::ONE),
            category: Some(None),
            account: Some(Uuid::nil()),
            quick: true,
            ..Default::default()
        };
        assert_eq!(draft.next_step(), None);
    }

    #[test]
    fn state_round_trips_as_json() {
        let state = State::TxWizard {
            draft: TxDraft {
                amount: Some(Decimal::new(15050, 2)),
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<State>(&json).unwrap(), state);
    }
}
