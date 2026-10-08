// Copyright (C) 2026 Marcos Gabriel Miller
use teloxide::types::MessageId;
use uuid::Uuid;

use super::{Ctx, require, usage};
use crate::{
    api::models::{Category, CreateCategory, TransactionQuery, UpdateCategory},
    bot::{
        callbacks::{Callback, CategoryField},
        commands::can_write,
        format::escape,
        keyboards::{button, confirm, rows},
        state::State,
    },
    errors::{BotError, BotResult},
    services::{matching::normalize, parse::parse_description},
    session::catalog::Catalog,
};

const REF_KIND: &str = "category";

async fn resolve(ctx: &Ctx, catalog: &Catalog, args: &str, command: &str) -> BotResult<Category> {
    if args.trim().is_empty() {
        return Err(usage(&format!(
            "/{command} N (número de /categories) o nombre"
        )));
    }
    ctx.resolve(
        REF_KIND,
        args,
        &catalog.categories,
        |c| c.uuid.to_string(),
        |c| c.name.as_str(),
        "la categoría",
    )
    .await
    .cloned()
}

fn validate_name(catalog: &Catalog, name: &str, except: Option<Uuid>) -> BotResult<String> {
    let name = name.trim().trim_start_matches('#').trim();
    if !(1..=120).contains(&name.chars().count()) {
        return Err(BotError::user(
            "El nombre debe tener entre 1 y 120 caracteres.",
        ));
    }
    let normalized = normalize(name);
    if catalog
        .categories
        .iter()
        .any(|c| Some(c.uuid) != except && normalize(&c.name) == normalized)
    {
        return Err(BotError::user(format!("Ya existe una categoría «{name}».")));
    }
    Ok(name.to_string())
}

pub async fn list(ctx: &Ctx) -> BotResult {
    let catalog = ctx.catalog().await?;
    let refs: Vec<String> = catalog
        .categories
        .iter()
        .map(|c| c.uuid.to_string())
        .collect();
    ctx.app.store.set_refs(ctx.uid, REF_KIND, &refs).await?;
    if catalog.categories.is_empty() {
        ctx.send("No tenés categorías. Creá una con /category_new Nombre")
            .await?;
        return Ok(());
    }
    let mut lines = vec!["<b>Categorías</b>".to_string()];
    lines.extend(
        catalog
            .categories
            .iter()
            .enumerate()
            .map(|(index, c)| format!("{}. {}", index + 1, escape(&c.name))),
    );
    lines.push("\n/category_new · /category_edit N · /category_delete N".into());
    ctx.send(lines.join("\n")).await?;
    Ok(())
}

pub async fn create(ctx: &Ctx, args: &str) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    if args.trim().is_empty() {
        ctx.set_state(State::CategoryCreate).await?;
        ctx.send("¿Nombre de la categoría?").await?;
        return Ok(());
    }
    save_new(ctx, args).await
}

pub async fn on_create_text(ctx: &Ctx, text: &str) -> BotResult {
    save_new(ctx, text).await
}

async fn save_new(ctx: &Ctx, name: &str) -> BotResult {
    let catalog = ctx.catalog().await?;
    let name = validate_name(&catalog, name, None)?;
    let api = &ctx.app.api;
    let category = ctx
        .app
        .call(ctx.uid, |auth| {
            let body = CreateCategory {
                user_uuid: auth.user_uuid,
                name: name.clone(),
                description: None,
            };
            async move { api.create_category(&auth.token, &body).await }
        })
        .await?;
    ctx.exit().await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.send(format!("Categoría «{}» creada ✅", escape(&category.name)))
        .await?;
    Ok(())
}

pub async fn edit(ctx: &Ctx, args: &str) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    let catalog = ctx.catalog().await?;
    let category = resolve(ctx, &catalog, args, "category_edit").await?;
    let field = |label: &str, field: CategoryField| {
        button(label, Callback::CategoryEditField(field, category.uuid))
    };
    let keyboard = rows(vec![vec![
        field("Nombre", CategoryField::Name),
        field("Descripción", CategoryField::Description),
        button("❌ Cancelar", Callback::Abort),
    ]]);
    ctx.send_kb(
        format!("Editar «{}». ¿Qué cambiás?", escape(&category.name)),
        keyboard,
    )
    .await?;
    Ok(())
}

pub async fn on_edit_field(ctx: &Ctx, field: CategoryField, uuid: Uuid) -> BotResult {
    ctx.set_state(State::CategoryEditValue { uuid, field })
        .await?;
    ctx.send(match field {
        CategoryField::Name => "Nuevo nombre:",
        CategoryField::Description => "Nueva descripción:",
    })
    .await?;
    Ok(())
}

pub async fn on_edit_value(ctx: &Ctx, uuid: Uuid, field: CategoryField, text: &str) -> BotResult {
    let catalog = ctx.catalog().await?;
    let body = match field {
        CategoryField::Name => UpdateCategory {
            name: Some(validate_name(&catalog, text, Some(uuid))?),
            ..Default::default()
        },
        CategoryField::Description => UpdateCategory {
            description: Some(
                parse_description(text)
                    .map_err(BotError::user)?
                    .ok_or_else(|| BotError::user("Escribí una descripción."))?,
            ),
            ..Default::default()
        },
    };
    let api = &ctx.app.api;
    let body = &body;
    ctx.app
        .call(ctx.uid, |auth| async move {
            api.update_category(&auth.token, uuid, body).await
        })
        .await?;
    ctx.exit().await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.send("Categoría actualizada ✅").await?;
    Ok(())
}

pub async fn delete(ctx: &Ctx, args: &str) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    let catalog = ctx.catalog().await?;
    let category = resolve(ctx, &catalog, args, "category_delete").await?;
    let api = &ctx.app.api;
    let query = TransactionQuery {
        category_uuid: Some(category.uuid),
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
    let warning = if count > 0 {
        format!("\nSus {count} transacciones quedarán «Sin categoría».")
    } else {
        String::new()
    };
    ctx.send_kb(
        format!(
            "¿Borrar la categoría «{}»?{warning}",
            escape(&category.name)
        ),
        confirm(Callback::CategoryDeleteYes(category.uuid)),
    )
    .await?;
    Ok(())
}

pub async fn on_delete_confirmed(ctx: &Ctx, uuid: Uuid, message: MessageId) -> BotResult {
    let api = &ctx.app.api;
    ctx.app
        .call(ctx.uid, |auth| async move {
            api.delete_category(&auth.token, uuid).await
        })
        .await?;
    ctx.app.invalidate_catalog(ctx.uid);
    ctx.edit(message, "Categoría borrada 🗑", None).await
}
