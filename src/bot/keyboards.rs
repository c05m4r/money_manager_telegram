// Copyright (C) 2026 Marcos Gabriel Miller
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use super::callbacks::Callback;

pub fn button(text: impl Into<String>, callback: Callback) -> InlineKeyboardButton {
    InlineKeyboardButton::callback(text, callback.encode())
}

pub fn grid(buttons: Vec<InlineKeyboardButton>, columns: usize) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(
        buttons
            .chunks(columns.max(1))
            .map(<[_]>::to_vec)
            .collect::<Vec<_>>(),
    )
}

pub fn rows(rows: Vec<Vec<InlineKeyboardButton>>) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(rows)
}

pub fn confirm(yes: Callback) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        button("✅ Sí", yes),
        button("❌ No", Callback::Abort),
    ]])
}
