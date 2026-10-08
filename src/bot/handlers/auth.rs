// Copyright (C) 2026 Marcos Gabriel Miller
use teloxide::{prelude::*, types::MessageId};
use zeroize::Zeroizing;

use super::{Ctx, help_text};
use crate::{
    bot::{
        callbacks::Callback,
        format::{escape, full_date},
        keyboards,
        state::State,
    },
    errors::{BotError, BotResult},
};

pub async fn start(ctx: &Ctx) -> BotResult {
    match ctx.app.store.get_session(ctx.uid).await? {
        Some(session) => {
            ctx.refresh_menu(Some(&session.role)).await;
            ctx.send(format!(
                "Hola {} 👋\n\n{}",
                escape(&session.username),
                help_text(Some(&session.role))
            ))
            .await?;
        }
        None => {
            ctx.refresh_menu(None).await;
            ctx.send(
                "Hola 👋 Soy el bot de <b>Money Manager</b>.\n\n\
                 Iniciá sesión una vez con /login usando tu usuario o email y tu contraseña del backend.",
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn help(ctx: &Ctx) -> BotResult {
    let role = ctx
        .app
        .store
        .get_session(ctx.uid)
        .await?
        .map(|session| session.role);
    let mut text = help_text(role.as_deref());
    if role.as_deref().is_some_and(crate::bot::commands::can_write) {
        text.push_str(
            "\n\n<b>Ejemplos</b>\n\
             /expense 1.500,50 #cafe cortado\n\
             /expense 8000 ayer #super @visa\n\
             /income 250000 #sueldo\n\
             /transactions @visa month:10/2026\n\
             /summary 09/2026",
        );
    }
    ctx.send(text).await?;
    Ok(())
}

pub async fn login(ctx: &Ctx, args: &str) -> BotResult {
    if let Some(wait) = ctx.app.sessions.login_retry_after(ctx.uid).await? {
        return Err(too_many_attempts(wait));
    }
    let identifier = args.trim();
    if identifier.is_empty() {
        ctx.set_state(State::LoginAskIdentifier).await?;
        ctx.send("Enviame tu <b>usuario o email</b> de Money Manager.")
            .await?;
        return Ok(());
    }
    on_identifier(ctx, identifier).await
}

pub async fn on_identifier(ctx: &Ctx, text: &str) -> BotResult {
    let identifier = text.trim();
    if !(3..=254).contains(&identifier.chars().count()) || identifier.contains(char::is_whitespace)
    {
        return Err(BotError::user(
            "Usuario o email inválido. Probá de nuevo o /cancel.",
        ));
    }
    ctx.set_state(State::LoginAskPassword {
        identifier: identifier.to_string(),
    })
    .await?;
    ctx.send("Ahora enviame tu <b>contraseña</b>. Voy a borrar el mensaje apenas lo lea.")
        .await?;
    Ok(())
}

/// Reads the password, deletes the message first, and never stores it in the dialogue.
pub async fn on_password(ctx: &Ctx, msg: &Message, identifier: &str) -> BotResult {
    let password = Zeroizing::new(msg.text().unwrap_or_default().to_string());
    if ctx.bot.delete_message(msg.chat.id, msg.id).await.is_err() {
        ctx.send("⚠️ No pude borrar tu mensaje con la contraseña: borralo vos.")
            .await?;
    }
    ctx.exit().await?;

    if password.trim().is_empty() || password.starts_with('/') {
        return Err(BotError::user(
            "Login cancelado. Enviá /login para empezar de nuevo.",
        ));
    }
    if let Some(wait) = ctx.app.sessions.login_retry_after(ctx.uid).await? {
        return Err(too_many_attempts(wait));
    }

    let response = ctx
        .app
        .sessions
        .login(ctx.uid, ctx.chat.0, identifier, &password)
        .await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.refresh_menu(Some(&response.user.role)).await;

    let remembered = if ctx.app.sessions.stores_credentials() {
        "Guardé tus credenciales cifradas para renovar la sesión solo."
    } else {
        "Cuando venza la sesión te voy a pedir /login de nuevo."
    };
    ctx.send(format!(
        "Hola {} 👋 Sesión iniciada.\n{remembered}\n\nProbá /expense 1500 #cafe o mirá /help",
        escape(&response.user.username)
    ))
    .await?;
    Ok(())
}

fn too_many_attempts(wait_secs: i64) -> BotError {
    BotError::user(format!(
        "Demasiados intentos fallidos, esperá {} minutos.",
        (wait_secs + 59) / 60
    ))
}

pub async fn logout(ctx: &Ctx) -> BotResult {
    if ctx.app.store.get_session(ctx.uid).await?.is_none() {
        return Err(BotError::NotLoggedIn);
    }
    ctx.send_kb(
        "¿Cerrar sesión? Se revoca el token y se borran tus credenciales guardadas.",
        keyboards::confirm(Callback::LogoutYes),
    )
    .await?;
    Ok(())
}

pub async fn on_logout_confirmed(ctx: &Ctx, message: MessageId) -> BotResult {
    ctx.app.sessions.logout(ctx.uid).await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.refresh_menu(None).await;
    ctx.edit(message, "Sesión cerrada. Para volver: /login", None)
        .await
}

pub async fn me(ctx: &Ctx) -> BotResult {
    let tz = ctx.app.config.tz;
    let api = &ctx.app.api;
    let user = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.get_user(&auth.token, auth.user_uuid).await
        })
        .await?;
    let stored = ctx.app.sessions.has_credentials(ctx.uid).await?;
    ctx.send(format!(
        "<b>{}</b>\nEmail: {}\nRol: {}\nEmail verificado: {}\nÚltimo login: {}\nCredenciales guardadas: {}",
        escape(&user.username),
        escape(&user.email),
        escape(&user.role),
        user.email_verified_at.map(|at| full_date(at, tz)).unwrap_or_else(|| "no".into()),
        user.last_login_at.map(|at| full_date(at, tz)).unwrap_or_else(|| "—".into()),
        if stored { "sí (cifradas)" } else { "no" },
    ))
    .await?;
    Ok(())
}
