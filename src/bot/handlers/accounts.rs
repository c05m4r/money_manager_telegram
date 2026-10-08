// Copyright (C) 2026 Marcos Gabriel Miller
use teloxide::types::MessageId;
use uuid::Uuid;

use super::{Ctx, require, usage};
use crate::{
    api::{
        ApiError,
        models::{Account, CreateAccount, TransactionQuery, UpdateAccount},
    },
    bot::{
        callbacks::{AccountField, Callback},
        commands::can_write,
        format::{escape, money},
        keyboards::{button, confirm, grid, rows},
        state::State,
    },
    errors::{BotError, BotResult},
    services::parse::parse_description,
    session::catalog::Catalog,
};

const REF_KIND: &str = "account";

async fn resolve(ctx: &Ctx, catalog: &Catalog, args: &str, command: &str) -> BotResult<Account> {
    if args.trim().is_empty() {
        return Err(usage(&format!(
            "/{command} N (número de /accounts) o nombre"
        )));
    }
    ctx.resolve(
        REF_KIND,
        args,
        &catalog.accounts,
        |a| a.uuid.to_string(),
        |a| a.name.as_str(),
        "la cuenta",
    )
    .await
    .cloned()
}

fn validate_name(name: &str) -> BotResult<String> {
    let name = name.trim();
    if (1..=150).contains(&name.chars().count()) {
        Ok(name.to_string())
    } else {
        Err(BotError::user(
            "El nombre debe tener entre 1 y 150 caracteres.",
        ))
    }
}

pub async fn list(ctx: &Ctx) -> BotResult {
    let catalog = ctx.catalog().await?;
    if catalog.accounts.is_empty() {
        ctx.send("No tenés cuentas. Creá una con /account_new Nombre ARS")
            .await?;
        return Ok(());
    }
    let api = &ctx.app.api;
    let report = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.report_balance(&auth.token, auth.user_uuid, None).await
        })
        .await?;

    let refs: Vec<String> = catalog
        .accounts
        .iter()
        .map(|a| a.uuid.to_string())
        .collect();
    ctx.app.store.set_refs(ctx.uid, REF_KIND, &refs).await?;

    let mut lines = vec!["<b>Cuentas</b>".to_string()];
    for (index, account) in catalog.accounts.iter().enumerate() {
        let symbol = catalog
            .currency(&account.currency_code)
            .map(|c| c.symbol.as_str())
            .unwrap_or("");
        let balance = report
            .data
            .iter()
            .find(|row| row.account_uuid == account.uuid)
            .map(|row| row.balance)
            .unwrap_or_default();
        lines.push(format!(
            "{}. {}{} ({}) — {}",
            index + 1,
            if account.is_default { "⭐ " } else { "" },
            escape(&account.name),
            escape(&account.currency_code),
            money(balance, symbol),
        ));
    }
    lines.push("\n/account_new · /account_edit N · /account_default N · /account_delete N".into());
    ctx.send(lines.join("\n")).await?;
    Ok(())
}

/// `/account_new Visa USD`, or ask for the missing parts.
pub async fn create(ctx: &Ctx, args: &str) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    let catalog = ctx.catalog().await?;
    let args = args.trim();
    if args.is_empty() {
        ctx.set_state(State::AccountCreate { name: None }).await?;
        ctx.send("¿Nombre de la cuenta?").await?;
        return Ok(());
    }
    let (name, currency) = match args.rsplit_once(char::is_whitespace) {
        Some((name, code)) if catalog.currency(code).is_some() => {
            (name.trim(), Some(code.to_uppercase()))
        }
        _ => (args, None),
    };
    let name = validate_name(name)?;
    match currency {
        Some(code) => save_new(ctx, name, code).await,
        None => ask_currency(ctx, &catalog, name).await,
    }
}

async fn ask_currency(ctx: &Ctx, catalog: &Catalog, name: String) -> BotResult {
    ctx.set_state(State::AccountCreate { name: Some(name) })
        .await?;
    ctx.send_kb("¿Moneda?", currency_keyboard(catalog)).await?;
    Ok(())
}

fn currency_keyboard(catalog: &Catalog) -> teloxide::types::InlineKeyboardMarkup {
    grid(
        catalog
            .currencies
            .iter()
            .map(|c| button(c.code.clone(), Callback::Pick(Some(c.code.clone()))))
            .collect(),
        4,
    )
}

pub async fn on_create_text(ctx: &Ctx, name: Option<String>, text: &str) -> BotResult {
    let catalog = ctx.catalog().await?;
    match name {
        None => ask_currency(ctx, &catalog, validate_name(text)?).await,
        Some(name) => {
            let code = text.trim().to_uppercase();
            if catalog.currency(&code).is_none() {
                return Err(BotError::user(
                    "Moneda desconocida. Elegí de los botones o mirá /currencies.",
                ));
            }
            save_new(ctx, name, code).await
        }
    }
}

pub async fn on_create_currency(
    ctx: &Ctx,
    name: &str,
    callback: Callback,
    message: MessageId,
) -> BotResult {
    let Callback::Pick(Some(code)) = callback else {
        return Err(BotError::user("Elegí una moneda."));
    };
    ctx.clear_keyboard(message).await;
    save_new(ctx, name.to_string(), code).await
}

async fn save_new(ctx: &Ctx, name: String, currency_code: String) -> BotResult {
    let api = &ctx.app.api;
    let account = ctx
        .app
        .call(ctx.uid, |auth| {
            let body = CreateAccount {
                user_uuid: auth.user_uuid,
                name: name.clone(),
                currency_code: currency_code.clone(),
                description: None,
                is_default: None,
            };
            async move { api.create_account(&auth.token, &body).await }
        })
        .await?;
    ctx.exit().await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.send(format!(
        "Cuenta «{}» ({}) creada ✅",
        escape(&account.name),
        escape(&account.currency_code)
    ))
    .await?;
    Ok(())
}

pub async fn edit(ctx: &Ctx, args: &str) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    let catalog = ctx.catalog().await?;
    let account = resolve(ctx, &catalog, args, "account_edit").await?;
    let field = |label: &str, field: AccountField| {
        button(label, Callback::AccountEditField(field, account.uuid))
    };
    let keyboard = rows(vec![
        vec![
            field("Nombre", AccountField::Name),
            field("Descripción", AccountField::Description),
        ],
        vec![
            field("Moneda", AccountField::Currency),
            button("❌ Cancelar", Callback::Abort),
        ],
    ]);
    ctx.send_kb(
        format!("Editar «{}». ¿Qué cambiás?", escape(&account.name)),
        keyboard,
    )
    .await?;
    Ok(())
}

pub async fn on_edit_field(ctx: &Ctx, field: AccountField, uuid: Uuid) -> BotResult {
    ctx.set_state(State::AccountEditValue { uuid, field })
        .await?;
    match field {
        AccountField::Name => ctx.send("Nuevo nombre:").await?,
        AccountField::Description => ctx.send("Nueva descripción:").await?,
        AccountField::Currency => {
            ctx.send_kb("Nueva moneda:", currency_keyboard(&*ctx.catalog().await?))
                .await?
        }
    };
    Ok(())
}

pub async fn on_edit_value(ctx: &Ctx, uuid: Uuid, field: AccountField, text: &str) -> BotResult {
    let mut body = UpdateAccount::default();
    match field {
        AccountField::Name => body.name = Some(validate_name(text)?),
        AccountField::Description => {
            body.description = Some(
                parse_description(text)
                    .map_err(BotError::user)?
                    .ok_or_else(|| BotError::user("Escribí una descripción."))?,
            )
        }
        AccountField::Currency => {
            let code = text.trim().to_uppercase();
            if ctx.catalog().await?.currency(&code).is_none() {
                return Err(BotError::user("Moneda desconocida. Elegí de los botones."));
            }
            body.currency_code = Some(code);
        }
    }
    update(ctx, uuid, body, "Cuenta actualizada ✅").await
}

pub async fn on_edit_pick(
    ctx: &Ctx,
    uuid: Uuid,
    field: AccountField,
    callback: Callback,
    message: MessageId,
) -> BotResult {
    match (field, callback) {
        (AccountField::Currency, Callback::Pick(Some(code))) => {
            ctx.clear_keyboard(message).await;
            update(
                ctx,
                uuid,
                UpdateAccount {
                    currency_code: Some(code),
                    ..Default::default()
                },
                "Cuenta actualizada ✅",
            )
            .await
        }
        _ => Err(BotError::user("Esa opción ya no está activa.")),
    }
}

async fn update(ctx: &Ctx, uuid: Uuid, body: UpdateAccount, done: &str) -> BotResult {
    let api = &ctx.app.api;
    let body = &body;
    ctx.app
        .call(ctx.uid, |auth| async move {
            api.update_account(&auth.token, uuid, body).await
        })
        .await?;
    ctx.exit().await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.send(done).await?;
    Ok(())
}

pub async fn make_default(ctx: &Ctx, args: &str) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    let catalog = ctx.catalog().await?;
    let account = resolve(ctx, &catalog, args, "account_default").await?;
    let message = format!(
        "«{}» es ahora tu cuenta por defecto ⭐",
        escape(&account.name)
    );
    update(
        ctx,
        account.uuid,
        UpdateAccount {
            is_default: Some(true),
            ..Default::default()
        },
        &message,
    )
    .await
}

pub async fn delete(ctx: &Ctx, args: &str) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    let catalog = ctx.catalog().await?;
    let account = resolve(ctx, &catalog, args, "account_delete").await?;
    if catalog.accounts.len() == 1 {
        return Err(BotError::user(
            "Es tu única cuenta: creá otra antes de borrarla.",
        ));
    }
    if account.is_default {
        return Err(BotError::user(
            "Es tu cuenta por defecto: elegí otra con /account_default antes de borrarla.",
        ));
    }

    let api = &ctx.app.api;
    let query = TransactionQuery {
        account_uuid: Some(account.uuid),
        ..Default::default()
    };
    let query = &query;
    let count = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.count_transactions(&auth.token, auth.user_uuid, query)
                .await
        })
        .await?;
    if count > 0 {
        return Err(BotError::user(format!(
            "«{}» tiene {count} transacciones: movelas a otra cuenta o borralas antes.",
            account.name
        )));
    }
    ctx.send_kb(
        format!("¿Borrar la cuenta «{}»?", escape(&account.name)),
        confirm(Callback::AccountDeleteYes(account.uuid)),
    )
    .await?;
    Ok(())
}

pub async fn on_delete_confirmed(ctx: &Ctx, uuid: Uuid, message: MessageId) -> BotResult {
    let api = &ctx.app.api;
    let result = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.delete_account(&auth.token, uuid).await
        })
        .await;
    ctx.app.invalidate_catalog(ctx.uid);
    match result {
        Ok(()) => ctx.edit(message, "Cuenta borrada 🗑", None).await,
        Err(BotError::Api(ApiError::Http { status: 409, .. })) => {
            ctx.edit(
                message,
                "La cuenta tiene transacciones: movelas o borralas antes.",
                None,
            )
            .await
        }
        Err(error) => Err(error),
    }
}
