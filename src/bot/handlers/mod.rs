// Copyright (C) 2026 Marcos Gabriel Miller
pub mod accounts;
pub mod auth;
pub mod catalogs;
pub mod categories;
pub mod reports;
pub mod transactions;

use std::sync::Arc;

use teloxide::{
    ApiError as TelegramApiError, RequestError,
    dispatching::dialogue::{Dialogue, SqliteStorage, serializer::Json},
    prelude::*,
    types::{
        BotCommandScope, CallbackQuery, ChatId, InlineKeyboardMarkup, MessageId, ParseMode,
        Recipient,
    },
    utils::command::BotCommands,
};

use crate::{
    app::AppRef,
    errors::{BotError, BotResult},
    services::parse::{Reference, parse_reference},
    session::{Auth, catalog::Catalog},
};

use super::{
    callbacks::Callback,
    commands::{Command, commands_for},
    format::escape,
    state::State,
};

pub type Storage = SqliteStorage<Json>;
pub type Dlg = Dialogue<State, Storage>;
pub type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Per-update context shared by every handler.
#[derive(Clone)]
pub struct Ctx {
    pub bot: Bot,
    pub app: AppRef,
    pub dialogue: Dlg,
    pub chat: ChatId,
    /// Telegram user id (`from.id`).
    pub uid: i64,
}

impl Ctx {
    pub async fn send(&self, text: impl Into<String>) -> BotResult<Message> {
        Ok(self
            .bot
            .send_message(self.chat, text)
            .parse_mode(ParseMode::Html)
            .await?)
    }

    pub async fn send_kb(
        &self,
        text: impl Into<String>,
        keyboard: InlineKeyboardMarkup,
    ) -> BotResult<Message> {
        Ok(self
            .bot
            .send_message(self.chat, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?)
    }

    /// Replaces a bot message in place (pagination, confirmations). "Not modified" is not an error.
    pub async fn edit(
        &self,
        message: MessageId,
        text: impl Into<String>,
        keyboard: Option<InlineKeyboardMarkup>,
    ) -> BotResult {
        let mut request = self
            .bot
            .edit_message_text(self.chat, message, text)
            .parse_mode(ParseMode::Html);
        if let Some(keyboard) = keyboard {
            request = request.reply_markup(keyboard);
        }
        match request.await {
            Ok(_) | Err(RequestError::Api(TelegramApiError::MessageNotModified)) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Removes the buttons of a message once its action was taken.
    pub async fn clear_keyboard(&self, message: MessageId) {
        let _ = self.bot.edit_message_reply_markup(self.chat, message).await;
    }

    pub async fn state(&self) -> BotResult<State> {
        self.dialogue
            .get_or_default()
            .await
            .map_err(|error| BotError::Dialogue(error.to_string()))
    }

    pub async fn set_state(&self, state: State) -> BotResult {
        self.dialogue
            .update(state)
            .await
            .map_err(|error| BotError::Dialogue(error.to_string()))
    }

    pub async fn exit(&self) -> BotResult {
        self.dialogue
            .exit()
            .await
            .map_err(|error| BotError::Dialogue(error.to_string()))
    }

    pub async fn auth(&self) -> BotResult<Auth> {
        self.app.auth(self.uid).await
    }

    pub async fn catalog(&self) -> BotResult<Arc<Catalog>> {
        self.app.catalog(self.uid).await
    }

    /// Shows the right command menu for the user's role in this chat (UX only).
    pub async fn refresh_menu(&self, role: Option<&str>) {
        let scope = BotCommandScope::Chat {
            chat_id: Recipient::Id(self.chat),
        };
        if let Err(error) = self
            .bot
            .set_my_commands(commands_for(role))
            .scope(scope)
            .await
        {
            tracing::warn!(uid = self.uid, %error, "set_my_commands failed");
        }
    }

    /// Sends the error to the user and logs it. Handlers always end successfully for teloxide.
    pub async fn report(&self, result: BotResult) -> HandlerResult {
        if let Err(error) = result {
            if error.is_internal() {
                tracing::error!(uid = self.uid, %error, "handler failed");
            } else {
                tracing::debug!(uid = self.uid, %error, "handler returned a user error");
            }
            if matches!(error, BotError::SessionExpired | BotError::NotLoggedIn) {
                let _ = self.exit().await;
                if matches!(error, BotError::SessionExpired) {
                    self.app.invalidate_catalog(self.uid);
                    self.refresh_menu(None).await;
                }
            }
            if let Err(send_error) = self.send(escape(&error.user_message())).await {
                tracing::error!(uid = self.uid, %send_error, "could not report error to user");
            }
        }
        Ok(())
    }

    /// Resolves `3`, `#3`, a UUID or a name. Positions come from the last listing of `kind`,
    /// falling back to `fallback` order (the same order the listing uses).
    pub async fn resolve<'a, T>(
        &self,
        kind: &str,
        token: &str,
        items: &'a [T],
        id_of: impl Fn(&T) -> String,
        name_of: impl Fn(&T) -> &str,
        noun: &str,
    ) -> BotResult<&'a T> {
        use crate::services::matching::{Match, find_by_name};

        let not_found = || {
            BotError::user(format!(
                "No encontré {noun} «{}». Listalos para ver los números.",
                token.trim()
            ))
        };
        match parse_reference(token)
            .ok_or_else(|| BotError::user(format!("Indicá {noun} por número, nombre o UUID.")))?
        {
            Reference::Position(position) => {
                let target = self.app.store.get_ref(self.uid, kind, position).await?;
                match target {
                    Some(target) => items
                        .iter()
                        .find(|item| id_of(item) == target)
                        .ok_or_else(not_found),
                    None => items.get(position as usize - 1).ok_or_else(not_found),
                }
            }
            Reference::Uuid(uuid) => items
                .iter()
                .find(|item| id_of(item) == uuid.to_string())
                .ok_or_else(not_found),
            Reference::Name(name) => match find_by_name(items, &name, &name_of) {
                Match::One(item) => Ok(item),
                Match::Ambiguous(matches) => Err(BotError::user(format!(
                    "«{name}» coincide con varios: {}. Sé más específico.",
                    matches
                        .iter()
                        .map(|item| name_of(item))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))),
                Match::None => Err(not_found()),
            },
        }
    }
}

fn ctx_for(bot: &Bot, app: &AppRef, dialogue: &Dlg, chat: ChatId, uid: i64) -> Ctx {
    Ctx {
        bot: bot.clone(),
        app: app.clone(),
        dialogue: dialogue.clone(),
        chat,
        uid,
    }
}

pub async fn on_message(bot: Bot, app: AppRef, dialogue: Dlg, msg: Message) -> HandlerResult {
    let Some(user) = msg.from.as_ref() else {
        return Ok(());
    };
    let ctx = ctx_for(&bot, &app, &dialogue, msg.chat.id, user.id.0 as i64);
    let result = route_message(&ctx, &msg).await;
    ctx.report(result).await
}

async fn route_message(ctx: &Ctx, msg: &Message) -> BotResult {
    let state = ctx.state().await?;

    // The password message is deleted before anything else, even if it is not text.
    if let State::LoginAskPassword { identifier } = &state {
        return auth::on_password(ctx, msg, identifier).await;
    }

    let Some(text) = msg.text() else {
        return Err(BotError::user("Enviame texto o un comando. /help"));
    };

    if text.starts_with('/') {
        let username = ctx.app.bot_username.as_str();
        return match Command::parse(text, username) {
            Ok(command) => {
                ctx.exit().await?;
                route_command(ctx, command).await
            }
            Err(_) => Err(BotError::user(
                "Comando desconocido o con argumentos inválidos. Mirá /help",
            )),
        };
    }

    match state {
        State::Idle => Err(BotError::user(
            "No entendí. Probá /expense 1500 #cafe o mirá /help",
        )),
        State::LoginAskIdentifier => auth::on_identifier(ctx, text).await,
        State::LoginAskPassword { .. } => unreachable!("handled above"),
        State::TxWizard { draft } => transactions::on_wizard_text(ctx, draft, text).await,
        State::TxEditValue { uuid, field } => {
            transactions::on_edit_value(ctx, uuid, field, text).await
        }
        State::TxEditPick { .. } => {
            Err(BotError::user("Elegí una opción de los botones o /cancel."))
        }
        State::AccountCreate { name } => accounts::on_create_text(ctx, name, text).await,
        State::AccountEditValue { uuid, field } => {
            accounts::on_edit_value(ctx, uuid, field, text).await
        }
        State::CategoryCreate => categories::on_create_text(ctx, text).await,
        State::CategoryEditValue { uuid, field } => {
            categories::on_edit_value(ctx, uuid, field, text).await
        }
        State::CurrencyEditValue { code, field } => {
            catalogs::on_currency_edit_value(ctx, &code, field, text).await
        }
        State::TypeEditValue { uuid, field } => {
            catalogs::on_type_edit_value(ctx, uuid, field, text).await
        }
    }
}

async fn route_command(ctx: &Ctx, command: Command) -> BotResult {
    use transactions::Kind;
    match command {
        Command::Start => auth::start(ctx).await,
        Command::Help => auth::help(ctx).await,
        Command::Cancel => ctx.send("Operación cancelada.").await.map(|_| ()),
        Command::Login(args) => auth::login(ctx, &args).await,
        Command::Logout => auth::logout(ctx).await,
        Command::Me => auth::me(ctx).await,
        Command::Expense(args) => transactions::quick(ctx, Kind::Expense, &args).await,
        Command::Income(args) => transactions::quick(ctx, Kind::Income, &args).await,
        Command::New => transactions::wizard(ctx).await,
        Command::Transactions(args) => transactions::list(ctx, &args).await,
        Command::Show(args) => transactions::show(ctx, &args).await,
        Command::Edit(args) => transactions::edit(ctx, &args).await,
        Command::Delete(args) => transactions::delete(ctx, &args).await,
        Command::Accounts => accounts::list(ctx).await,
        Command::AccountNew(args) => accounts::create(ctx, &args).await,
        Command::AccountEdit(args) => accounts::edit(ctx, &args).await,
        Command::AccountDelete(args) => accounts::delete(ctx, &args).await,
        Command::AccountDefault(args) => accounts::make_default(ctx, &args).await,
        Command::Categories => categories::list(ctx).await,
        Command::CategoryNew(args) => categories::create(ctx, &args).await,
        Command::CategoryEdit(args) => categories::edit(ctx, &args).await,
        Command::CategoryDelete(args) => categories::delete(ctx, &args).await,
        Command::Currencies => catalogs::list_currencies(ctx).await,
        Command::CurrencyNew(args) => catalogs::create_currency(ctx, &args).await,
        Command::CurrencyEdit(args) => catalogs::edit_currency(ctx, &args).await,
        Command::CurrencyDelete(args) => catalogs::delete_currency(ctx, &args).await,
        Command::Types => catalogs::list_types(ctx).await,
        Command::TypeNew(args) => catalogs::create_type(ctx, &args).await,
        Command::TypeEdit(args) => catalogs::edit_type(ctx, &args).await,
        Command::TypeDelete(args) => catalogs::delete_type(ctx, &args).await,
        Command::Balance(args) => reports::balance(ctx, &args).await,
        Command::Summary(args) => reports::summary(ctx, &args).await,
    }
}

pub async fn on_callback(
    bot: Bot,
    app: AppRef,
    dialogue: Dlg,
    query: CallbackQuery,
) -> HandlerResult {
    let _ = bot.answer_callback_query(query.id.clone()).await;
    let Some(message) = query.regular_message() else {
        return Ok(());
    };
    let ctx = ctx_for(
        &bot,
        &app,
        &dialogue,
        message.chat.id,
        query.from.id.0 as i64,
    );
    let Some(callback) = query.data.as_deref().and_then(Callback::decode) else {
        return Ok(());
    };
    let result = route_callback(&ctx, callback, message.id).await;
    ctx.report(result).await
}

async fn route_callback(ctx: &Ctx, callback: Callback, message: MessageId) -> BotResult {
    let state = ctx.state().await?;
    match callback {
        Callback::Abort => {
            ctx.exit().await?;
            ctx.edit(message, "Operación cancelada.", None).await
        }
        Callback::LogoutYes => auth::on_logout_confirmed(ctx, message).await,
        Callback::Page(kind, page) => transactions::on_page(ctx, kind, page, message).await,
        Callback::TxEdit(uuid) => transactions::on_edit_menu(ctx, uuid).await,
        Callback::TxEditField(field, uuid) => transactions::on_edit_field(ctx, field, uuid).await,
        Callback::TxDelete(uuid) => transactions::on_delete(ctx, uuid).await,
        Callback::TxDeleteYes(uuid) => transactions::on_delete_confirmed(ctx, uuid, message).await,
        Callback::AccountEditField(field, uuid) => accounts::on_edit_field(ctx, field, uuid).await,
        Callback::AccountDeleteYes(uuid) => accounts::on_delete_confirmed(ctx, uuid, message).await,
        Callback::CategoryEditField(field, uuid) => {
            categories::on_edit_field(ctx, field, uuid).await
        }
        Callback::CategoryDeleteYes(uuid) => {
            categories::on_delete_confirmed(ctx, uuid, message).await
        }
        Callback::CurrencyEditField(field, code) => {
            catalogs::on_currency_edit_field(ctx, field, code).await
        }
        Callback::CurrencyDeleteYes(code) => {
            catalogs::on_currency_delete_confirmed(ctx, &code, message).await
        }
        Callback::TypeEditField(field, uuid) => {
            catalogs::on_type_edit_field(ctx, field, uuid).await
        }
        Callback::TypeDeleteYes(uuid) => {
            catalogs::on_type_delete_confirmed(ctx, uuid, message).await
        }
        Callback::Pick(_)
        | Callback::PickToday
        | Callback::PickYesterday
        | Callback::Skip
        | Callback::Confirm
        | Callback::NewCategory(_) => match state {
            State::TxWizard { draft } => {
                transactions::on_wizard_callback(ctx, draft, callback, message).await
            }
            State::TxEditPick { uuid, field } => {
                transactions::on_edit_pick(ctx, uuid, field, callback, message).await
            }
            State::AccountCreate { name: Some(name) } => {
                accounts::on_create_currency(ctx, &name, callback, message).await
            }
            State::AccountEditValue { uuid, field } => {
                accounts::on_edit_pick(ctx, uuid, field, callback, message).await
            }
            _ => {
                ctx.clear_keyboard(message).await;
                Err(BotError::user("Esa opción ya no está activa."))
            }
        },
    }
}

/// Fails with "no permission" without calling the API (backend still enforces it).
pub fn require(allowed: bool) -> BotResult {
    if allowed {
        Ok(())
    } else {
        Err(BotError::Forbidden)
    }
}

/// Usage hint for commands that need an argument.
pub fn usage(text: &str) -> BotError {
    BotError::user(format!("Uso: {text}"))
}

/// Help text for every command available to the role.
pub fn help_text(role: Option<&str>) -> String {
    let descriptions = Command::descriptions().to_string();
    let allowed: Vec<String> = commands_for(role)
        .into_iter()
        .map(|c| c.command.trim_start_matches('/').to_string())
        .collect();
    let lines: Vec<String> = descriptions
        .lines()
        .filter(|line| {
            line.trim_start_matches('/')
                .split([' ', '—', '-'])
                .next()
                .is_some_and(|name| allowed.iter().any(|allowed| allowed == name))
        })
        .map(escape)
        .collect();
    lines.join("\n")
}
