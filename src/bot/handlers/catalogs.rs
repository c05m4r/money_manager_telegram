// Copyright (C) 2026 Marcos Gabriel Miller
//! Currencies and transaction types: everyone can list, `admin`/`manager` can manage.
use teloxide::types::MessageId;
use uuid::Uuid;

use super::{Ctx, require, usage};
use crate::{
    api::models::{
        CreateCurrency, CreateTransactionType, TransactionType, UpdateCurrency,
        UpdateTransactionType,
    },
    bot::{
        callbacks::{Callback, CurrencyField, TypeField},
        commands::can_manage_catalogs,
        format::escape,
        keyboards::{button, confirm, rows},
        state::State,
    },
    errors::{BotError, BotResult},
    services::parse::parse_description,
    session::catalog::{Catalog, EXPENSE_CODE, INCOME_CODE},
};

const TYPE_REF_KIND: &str = "type";

async fn require_manager(ctx: &Ctx) -> BotResult {
    require(can_manage_catalogs(&ctx.auth().await?.role))
}

fn length(value: &str, max: usize, what: &str) -> BotResult<String> {
    let value = value.trim();
    if (1..=max).contains(&value.chars().count()) {
        Ok(value.to_string())
    } else {
        Err(BotError::user(format!(
            "{what} debe tener entre 1 y {max} caracteres."
        )))
    }
}

fn is_builtin(t: &TransactionType) -> bool {
    t.code.eq_ignore_ascii_case(INCOME_CODE) || t.code.eq_ignore_ascii_case(EXPENSE_CODE)
}

// ---------- currencies ----------

pub async fn list_currencies(ctx: &Ctx) -> BotResult {
    let catalog = ctx.catalog().await?;
    let mut lines = vec!["<b>Monedas</b>".to_string()];
    lines.extend(catalog.currencies.iter().map(|c| {
        format!(
            "{} — {} ({})",
            escape(&c.code),
            escape(&c.name),
            escape(&c.symbol)
        )
    }));
    ctx.send(lines.join("\n")).await?;
    Ok(())
}

fn find_currency(catalog: &Catalog, code: &str) -> BotResult<String> {
    catalog
        .currency(code.trim())
        .map(|c| c.code.clone())
        .ok_or_else(|| {
            BotError::user(format!(
                "No existe la moneda «{}». Mirá /currencies",
                code.trim()
            ))
        })
}

/// `/currency_new BRL R$ Real brasileño`
pub async fn create_currency(ctx: &Ctx, args: &str) -> BotResult {
    require_manager(ctx).await?;
    let mut parts = args.split_whitespace();
    let (Some(code), Some(symbol)) = (parts.next(), parts.next()) else {
        return Err(usage(
            "/currency_new CÓDIGO SÍMBOLO Nombre (ej.: /currency_new BRL R$ Real brasileño)",
        ));
    };
    let name = parts.collect::<Vec<_>>().join(" ");
    if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(BotError::user("El código debe tener 3 letras (ISO 4217)."));
    }
    let body = CreateCurrency {
        code: code.to_uppercase(),
        symbol: length(symbol, 10, "El símbolo")?,
        name: length(&name, 100, "El nombre")?,
        description: None,
    };
    let api = &ctx.app.api;
    let body = &body;
    let currency = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.create_currency(&auth.token, body).await
        })
        .await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.send(format!("Moneda {} creada ✅", escape(&currency.code)))
        .await?;
    Ok(())
}

pub async fn edit_currency(ctx: &Ctx, args: &str) -> BotResult {
    require_manager(ctx).await?;
    if args.trim().is_empty() {
        return Err(usage("/currency_edit CÓDIGO"));
    }
    let code = find_currency(&*ctx.catalog().await?, args)?;
    let field = |label: &str, field: CurrencyField| {
        button(label, Callback::CurrencyEditField(field, code.clone()))
    };
    let keyboard = rows(vec![
        vec![
            field("Nombre", CurrencyField::Name),
            field("Símbolo", CurrencyField::Symbol),
        ],
        vec![
            field("Descripción", CurrencyField::Description),
            button("❌ Cancelar", Callback::Abort),
        ],
    ]);
    ctx.send_kb(format!("Editar {}. ¿Qué cambiás?", escape(&code)), keyboard)
        .await?;
    Ok(())
}

pub async fn on_currency_edit_field(ctx: &Ctx, field: CurrencyField, code: String) -> BotResult {
    require_manager(ctx).await?;
    ctx.set_state(State::CurrencyEditValue { code, field })
        .await?;
    ctx.send(match field {
        CurrencyField::Name => "Nuevo nombre:",
        CurrencyField::Symbol => "Nuevo símbolo:",
        CurrencyField::Description => "Nueva descripción:",
    })
    .await?;
    Ok(())
}

pub async fn on_currency_edit_value(
    ctx: &Ctx,
    code: &str,
    field: CurrencyField,
    text: &str,
) -> BotResult {
    let mut body = UpdateCurrency::default();
    match field {
        CurrencyField::Name => body.name = Some(length(text, 100, "El nombre")?),
        CurrencyField::Symbol => body.symbol = Some(length(text, 10, "El símbolo")?),
        CurrencyField::Description => {
            body.description = Some(
                parse_description(text)
                    .map_err(BotError::user)?
                    .ok_or_else(|| BotError::user("Escribí una descripción."))?,
            )
        }
    }
    let api = &ctx.app.api;
    let body = &body;
    ctx.app
        .call(ctx.uid, |auth| async move {
            api.update_currency(&auth.token, code, body).await
        })
        .await?;
    ctx.exit().await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.send(format!("Moneda {} actualizada ✅", escape(code)))
        .await?;
    Ok(())
}

pub async fn delete_currency(ctx: &Ctx, args: &str) -> BotResult {
    require_manager(ctx).await?;
    if args.trim().is_empty() {
        return Err(usage("/currency_delete CÓDIGO"));
    }
    let catalog = ctx.catalog().await?;
    let code = find_currency(&catalog, args)?;
    if catalog.accounts.iter().any(|a| a.currency_code == code) {
        return Err(BotError::user(format!(
            "Tenés cuentas en {code}: cambiales la moneda antes de borrarla."
        )));
    }
    ctx.send_kb(
        format!("¿Borrar la moneda {}?", escape(&code)),
        confirm(Callback::CurrencyDeleteYes(code)),
    )
    .await?;
    Ok(())
}

pub async fn on_currency_delete_confirmed(ctx: &Ctx, code: &str, message: MessageId) -> BotResult {
    require_manager(ctx).await?;
    let api = &ctx.app.api;
    ctx.app
        .call(ctx.uid, |auth| async move {
            api.delete_currency(&auth.token, code).await
        })
        .await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.edit(message, format!("Moneda {} borrada 🗑", escape(code)), None)
        .await
}

// ---------- transaction types ----------

async fn resolve_type(
    ctx: &Ctx,
    catalog: &Catalog,
    args: &str,
    command: &str,
) -> BotResult<TransactionType> {
    if args.trim().is_empty() {
        return Err(usage(&format!("/{command} N (número de /types) o nombre")));
    }
    ctx.resolve(
        TYPE_REF_KIND,
        args,
        &catalog.types,
        |t| t.uuid.to_string(),
        |t| t.name.as_str(),
        "el tipo",
    )
    .await
    .cloned()
}

pub async fn list_types(ctx: &Ctx) -> BotResult {
    let catalog = ctx.catalog().await?;
    let refs: Vec<String> = catalog.types.iter().map(|t| t.uuid.to_string()).collect();
    ctx.app
        .store
        .set_refs(ctx.uid, TYPE_REF_KIND, &refs)
        .await?;
    let mut lines = vec!["<b>Tipos de transacción</b>".to_string()];
    lines.extend(
        catalog
            .types
            .iter()
            .enumerate()
            .map(|(index, t)| format!("{}. {} ({})", index + 1, escape(&t.name), escape(&t.code))),
    );
    ctx.send(lines.join("\n")).await?;
    Ok(())
}

/// `/type_new transfer Transferencia`
pub async fn create_type(ctx: &Ctx, args: &str) -> BotResult {
    require_manager(ctx).await?;
    let Some((code, name)) = args.trim().split_once(char::is_whitespace) else {
        return Err(usage(
            "/type_new código Nombre (ej.: /type_new transfer Transferencia)",
        ));
    };
    let body = CreateTransactionType {
        code: length(code, 50, "El código")?,
        name: length(name, 100, "El nombre")?,
        description: None,
    };
    let api = &ctx.app.api;
    let body = &body;
    let created = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.create_type(&auth.token, body).await
        })
        .await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.send(format!("Tipo «{}» creado ✅", escape(&created.name)))
        .await?;
    Ok(())
}

pub async fn edit_type(ctx: &Ctx, args: &str) -> BotResult {
    require_manager(ctx).await?;
    let catalog = ctx.catalog().await?;
    let t = resolve_type(ctx, &catalog, args, "type_edit").await?;
    let field =
        |label: &str, field: TypeField| button(label, Callback::TypeEditField(field, t.uuid));
    let mut first_row = vec![field("Nombre", TypeField::Name)];
    if !is_builtin(&t) {
        first_row.push(field("Código", TypeField::Code));
    }
    let keyboard = rows(vec![
        first_row,
        vec![
            field("Descripción", TypeField::Description),
            button("❌ Cancelar", Callback::Abort),
        ],
    ]);
    ctx.send_kb(
        format!("Editar «{}». ¿Qué cambiás?", escape(&t.name)),
        keyboard,
    )
    .await?;
    Ok(())
}

pub async fn on_type_edit_field(ctx: &Ctx, field: TypeField, uuid: Uuid) -> BotResult {
    require_manager(ctx).await?;
    ctx.set_state(State::TypeEditValue { uuid, field }).await?;
    ctx.send(match field {
        TypeField::Name => "Nuevo nombre:",
        TypeField::Code => "Nuevo código:",
        TypeField::Description => "Nueva descripción:",
    })
    .await?;
    Ok(())
}

pub async fn on_type_edit_value(ctx: &Ctx, uuid: Uuid, field: TypeField, text: &str) -> BotResult {
    let catalog = ctx.catalog().await?;
    let mut body = UpdateTransactionType::default();
    match field {
        TypeField::Name => body.name = Some(length(text, 100, "El nombre")?),
        TypeField::Code => {
            if catalog.type_by_uuid(uuid).is_some_and(is_builtin) {
                return Err(BotError::user(
                    "No se puede cambiar el código de los tipos IN/OUT: /expense e /income dependen de ellos.",
                ));
            }
            body.code = Some(length(text, 50, "El código")?);
        }
        TypeField::Description => {
            body.description = Some(
                parse_description(text)
                    .map_err(BotError::user)?
                    .ok_or_else(|| BotError::user("Escribí una descripción."))?,
            )
        }
    }
    let api = &ctx.app.api;
    let body = &body;
    ctx.app
        .call(ctx.uid, |auth| async move {
            api.update_type(&auth.token, uuid, body).await
        })
        .await?;
    ctx.exit().await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.send("Tipo actualizado ✅").await?;
    Ok(())
}

pub async fn delete_type(ctx: &Ctx, args: &str) -> BotResult {
    require_manager(ctx).await?;
    let catalog = ctx.catalog().await?;
    let t = resolve_type(ctx, &catalog, args, "type_delete").await?;
    if is_builtin(&t) {
        return Err(BotError::user(
            "No se pueden borrar los tipos IN/OUT: /expense e /income dependen de ellos.",
        ));
    }
    ctx.send_kb(
        format!("¿Borrar el tipo «{}»?", escape(&t.name)),
        confirm(Callback::TypeDeleteYes(t.uuid)),
    )
    .await?;
    Ok(())
}

pub async fn on_type_delete_confirmed(ctx: &Ctx, uuid: Uuid, message: MessageId) -> BotResult {
    require_manager(ctx).await?;
    if ctx
        .catalog()
        .await?
        .type_by_uuid(uuid)
        .is_some_and(is_builtin)
    {
        return Err(BotError::user("No se pueden borrar los tipos IN/OUT."));
    }
    let api = &ctx.app.api;
    ctx.app
        .call(ctx.uid, |auth| async move {
            api.delete_type(&auth.token, uuid).await
        })
        .await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.edit(message, "Tipo borrado 🗑", None).await
}
