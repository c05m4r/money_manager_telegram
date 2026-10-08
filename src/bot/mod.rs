// Copyright (C) 2026 Marcos Gabriel Miller
pub mod callbacks;
pub mod commands;
pub mod format;
pub mod handlers;
pub mod keyboards;
pub mod state;

use teloxide::{dispatching::UpdateHandler, prelude::*};

use crate::app::AppRef;
use handlers::Storage;
use state::State;

type HandlerError = Box<dyn std::error::Error + Send + Sync>;

/// Private chats only; when `ALLOWED_TELEGRAM_IDS` is set, other users are told once and ignored.
async fn is_allowed(bot: &Bot, app: &AppRef, chat: ChatId, telegram_user_id: i64) -> bool {
    if app.config.is_allowed(telegram_user_id) {
        return true;
    }
    let first_time = app
        .denied_notified
        .lock()
        .expect("denied set poisoned")
        .insert(telegram_user_id);
    if first_time {
        tracing::info!(telegram_user_id, "rejected user outside the allowlist");
        let _ = bot.send_message(chat, "No autorizado.").await;
    }
    false
}

pub fn schema() -> UpdateHandler<HandlerError> {
    let messages = Update::filter_message()
        .filter(|msg: Message| msg.chat.is_private() && msg.from.is_some())
        .filter_async(|bot: Bot, app: AppRef, msg: Message| async move {
            let uid = msg
                .from
                .as_ref()
                .map(|user| user.id.0 as i64)
                .unwrap_or_default();
            is_allowed(&bot, &app, msg.chat.id, uid).await
        })
        .enter_dialogue::<Message, Storage, State>()
        .endpoint(handlers::on_message);

    let callbacks = Update::filter_callback_query()
        .filter(|query: CallbackQuery| {
            query
                .regular_message()
                .is_some_and(|msg| msg.chat.is_private())
        })
        .filter_async(|bot: Bot, app: AppRef, query: CallbackQuery| async move {
            let chat = query
                .regular_message()
                .map(|msg| msg.chat.id)
                .unwrap_or(ChatId(0));
            is_allowed(&bot, &app, chat, query.from.id.0 as i64).await
        })
        .enter_dialogue::<CallbackQuery, Storage, State>()
        .endpoint(handlers::on_callback);

    dptree::entry().branch(messages).branch(callbacks)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use teloxide::{
        dispatching::dialogue::{SqliteStorage, serializer::Json},
        prelude::*,
    };

    use crate::{api::ApiClient, app::App, config::BotConfig, session::store::memory_store};

    /// dptree checks at build time that every handler parameter is provided; a missing
    /// dependency would only show up at startup otherwise.
    #[tokio::test]
    async fn dispatcher_dependencies_are_complete() {
        let config = BotConfig::from_lookup(|name| match name {
            "TELOXIDE_TOKEN" => Some("123:fake".into()),
            "STORE_CREDENTIALS" => Some("false".into()),
            _ => None,
        })
        .unwrap();
        let api = ApiClient::new("http://127.0.0.1:9", Duration::from_secs(1)).unwrap();
        let app = Arc::new(App::new(
            config,
            "test_bot".into(),
            api,
            memory_store().await,
        ));
        let dir = std::env::temp_dir().join(format!("mm_bot_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = SqliteStorage::open(dir.join("d.sqlite").to_str().unwrap(), Json)
            .await
            .unwrap();

        let _dispatcher = Dispatcher::builder(Bot::new("123:fake"), super::schema())
            .dependencies(dptree::deps![app, storage])
            .build();
        std::fs::remove_dir_all(dir).ok();
    }
}
