// Copyright (C) 2026 Marcos Gabriel Miller
mod api;
mod app;
mod bot;
mod config;
mod errors;
mod services;
mod session;

#[cfg(test)]
mod live_tests;

use std::{process::ExitCode, sync::Arc};

use teloxide::{
    dispatching::dialogue::{SqliteStorage, serializer::Json},
    prelude::*,
};
use tracing_subscriber::EnvFilter;

use crate::{
    api::ApiClient, app::App, bot::commands::commands_for, config::BotConfig, session::store::Store,
};

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::from_filename("env/.env")
        .or_else(|_| dotenvy::dotenv())
        .ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let config =
        BotConfig::from_env().map_err(|error| format!("invalid configuration: {error}"))?;
    if config.backend_is_insecure() {
        tracing::warn!(url = %config.backend_api_url, "BACKEND_API_URL is plain HTTP to a remote host; use https://");
    }
    match &config.auth {
        config::AuthMethod::Telegram(auth) => {
            tracing::warn!(
                users = auth.users.len(),
                "AUTH_METHOD=telegram: the bot signs backend JWTs with JWT_SECRET and can act as any backend user; protect this host like the backend"
            );
            if auth.jwt_secret.len() < 32
                || ["dev-secret", "dev-change-me"].contains(&auth.jwt_secret.as_str())
            {
                tracing::warn!("JWT_SECRET looks weak or is a development default");
            }
        }
        config::AuthMethod::Password if config.credentials_key.is_none() => {
            tracing::info!(
                "STORE_CREDENTIALS=false: users will be asked to /login again when their token expires"
            );
        }
        config::AuthMethod::Password => {}
    }

    let store = Store::open(&config.database_path)
        .await
        .map_err(|error| format!("cannot open {}: {error}", config.database_path.display()))?;
    let path = config
        .database_path
        .to_str()
        .ok_or("DATABASE_PATH must be valid UTF-8")?;
    let storage = SqliteStorage::open(path, Json)
        .await
        .map_err(|error| format!("cannot open dialogue storage: {error}"))?;
    let api = ApiClient::new(&config.backend_api_url, config.http_timeout)
        .map_err(|error| error.to_string())?;

    let bot = Bot::new(&config.telegram_token);
    let me = bot
        .get_me()
        .await
        .map_err(|error| format!("cannot reach Telegram (check TELOXIDE_TOKEN): {error}"))?;
    let username = me.username().to_string();
    if let Err(error) = bot.set_my_commands(commands_for(None)).await {
        tracing::warn!(%error, "set_my_commands failed");
    }

    tracing::info!(bot = %username, backend = %config.backend_api_url, auth = config.auth.name(), "starting");
    let app = Arc::new(App::new(config, username, api, store));

    Dispatcher::builder(bot, bot::schema())
        .dependencies(dptree::deps![app, storage])
        .default_handler(|_| async {})
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
    Ok(())
}
