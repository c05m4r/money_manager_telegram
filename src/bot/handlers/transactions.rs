// Copyright (C) 2026 Marcos Gabriel Miller
use std::collections::HashMap;

use chrono::Utc;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId};
use uuid::Uuid;

use super::{Ctx, require, usage};
use crate::{
    api::models::{
        CategoryReportQuery, CreateCategory, CreateTransaction, Transaction, TransactionQuery,
        UpdateTransaction,
    },
    bot::{
        callbacks::{Callback, ListKind, MAX_NEW_CATEGORY_BYTES, TxField},
        commands::can_write,
        format::{escape, full_date, short_date, signed_money, truncate},
        keyboards::{button, confirm, grid, rows},
        state::{State, TxDraft, TxStep},
    },
    errors::{BotError, BotResult},
    services::{
        matching::{Match, find_by_name},
        parse::{
            Flow, Reference, parse_amount, parse_date, parse_description, parse_list_filters,
            parse_quick_entry, parse_reference, to_utc,
        },
    },
    session::catalog::{Catalog, EXPENSE_CODE, INCOME_CODE},
};

const REF_KIND: &str = "tx";
const MAX_CATEGORY_BUTTONS: usize = 30;

#[derive(Clone, Copy)]
pub enum Kind {
    Expense,
    Income,
}

impl Kind {
    fn code(self) -> &'static str {
        match self {
            Kind::Expense => EXPENSE_CODE,
            Kind::Income => INCOME_CODE,
        }
    }
}

fn sign_for(catalog: &Catalog, type_uuid: Uuid) -> Option<char> {
    match catalog
        .type_by_uuid(type_uuid)
        .map(|t| t.code.to_uppercase())
    {
        Some(code) if code == EXPENSE_CODE => Some('−'),
        Some(code) if code == INCOME_CODE => Some('+'),
        _ => None,
    }
}

fn category_name(catalog: &Catalog, uuid: Option<Uuid>) -> String {
    uuid.and_then(|uuid| catalog.category(uuid))
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "Sin categoría".into())
}

/// Detail view of a transaction.
fn render(catalog: &Catalog, tx: &Transaction, tz: chrono_tz::Tz) -> String {
    let (symbol, _) = catalog.symbol_for_account(tx.account_uuid);
    let type_name = catalog
        .type_by_uuid(tx.type_uuid)
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "?".into());
    let account = tx
        .account_uuid
        .and_then(|uuid| catalog.account(uuid))
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "—".into());
    format!(
        "<b>{}</b>\nTipo: {}\nCategoría: {}\nCuenta: {}\nFecha: {}\nDescripción: {}",
        signed_money(tx.amount, &symbol, sign_for(catalog, tx.type_uuid)),
        escape(&type_name),
        escape(&category_name(catalog, tx.category_uuid)),
        escape(&account),
        full_date(tx.transaction_date, tz),
        escape(tx.description.as_deref().unwrap_or("—")),
    )
}

fn actions(uuid: Uuid) -> InlineKeyboardMarkup {
    rows(vec![vec![
        button("✏️ Editar", Callback::TxEdit(uuid)),
        button("🗑 Borrar", Callback::TxDelete(uuid)),
    ]])
}

// ---------- quick entry (/expense, /income) ----------

pub async fn quick(ctx: &Ctx, kind: Kind, args: &str) -> BotResult {
    let auth = ctx.auth().await?;
    require(can_write(&auth.role))?;
    let catalog = ctx.catalog().await?;
    let entry = parse_quick_entry(args, ctx.app.today()).map_err(BotError::user)?;

    let tx_type = catalog.type_by_code(kind.code()).ok_or_else(|| {
        BotError::user(format!(
            "No existe un tipo de transacción con código {}. Usá /new para elegir otro.",
            kind.code()
        ))
    })?;

    let mut draft = TxDraft {
        type_uuid: Some(tx_type.uuid),
        amount: entry.amount,
        date: entry.date,
        description: Some(entry.description),
        quick: entry.amount.is_some(),
        ..Default::default()
    };

    draft.account = Some(match &entry.account {
        Some(name) => resolve_account(&catalog, name)?,
        None => {
            catalog
                .default_account()
                .ok_or_else(|| {
                    BotError::user("No tenés cuentas. Creá una con /account_new Nombre ARS")
                })?
                .uuid
        }
    });

    if let Some(name) = &entry.category {
        match find_by_name(&catalog.categories, name, |c| c.name.as_str()) {
            Match::One(category) => draft.category = Some(Some(category.uuid)),
            Match::Ambiguous(_) => {}
            Match::None => return offer_new_category(ctx, &catalog, draft, name).await,
        }
    }
    advance(ctx, draft).await
}

fn resolve_account(catalog: &Catalog, name: &str) -> BotResult<Uuid> {
    match find_by_name(&catalog.accounts, name, |a| a.name.as_str()) {
        Match::One(account) => Ok(account.uuid),
        Match::Ambiguous(matches) => Err(BotError::user(format!(
            "«{name}» coincide con varias cuentas: {}.",
            matches
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        Match::None => Err(BotError::user(format!(
            "No encontré la cuenta «{name}». Mirá /accounts"
        ))),
    }
}

/// Unknown `#name`: offer to create it, or pick an existing one.
async fn offer_new_category(
    ctx: &Ctx,
    catalog: &Catalog,
    mut draft: TxDraft,
    name: &str,
) -> BotResult {
    draft.step = TxStep::Category;
    let mut keyboard = category_keyboard(ctx, catalog, draft.type_uuid).await;
    if name.len() <= MAX_NEW_CATEGORY_BYTES {
        keyboard.inline_keyboard.insert(
            0,
            vec![button(
                format!("➕ Crear «{name}»"),
                Callback::NewCategory(name.to_string()),
            )],
        );
    }
    ctx.set_state(State::TxWizard { draft }).await?;
    ctx.send_kb(
        format!(
            "No existe la categoría «{}». ¿La creo o elegís otra?",
            escape(name)
        ),
        keyboard,
    )
    .await?;
    Ok(())
}

// ---------- wizard (/new) ----------

pub async fn wizard(ctx: &Ctx) -> BotResult {
    let auth = ctx.auth().await?;
    require(can_write(&auth.role))?;
    advance(ctx, TxDraft::default()).await
}

/// Asks for the next missing piece, or creates the transaction when complete.
async fn advance(ctx: &Ctx, mut draft: TxDraft) -> BotResult {
    let catalog = ctx.catalog().await?;
    loop {
        let Some(step) = draft.next_step() else {
            return create(ctx, &catalog, &draft).await;
        };
        // Skip choices with a single option.
        if step == TxStep::Account && catalog.accounts.len() == 1 {
            draft.account = Some(catalog.accounts[0].uuid);
            continue;
        }
        if step == TxStep::Account && catalog.accounts.is_empty() {
            ctx.exit().await?;
            return Err(BotError::user(
                "No tenés cuentas. Creá una con /account_new Nombre ARS",
            ));
        }
        draft.step = step;
        ctx.set_state(State::TxWizard {
            draft: draft.clone(),
        })
        .await?;
        return prompt(ctx, &catalog, &draft).await;
    }
}

async fn prompt(ctx: &Ctx, catalog: &Catalog, draft: &TxDraft) -> BotResult {
    match draft.step {
        TxStep::Type => {
            let buttons = catalog
                .types
                .iter()
                .map(|t| button(t.name.clone(), Callback::Pick(Some(t.uuid.to_string()))))
                .collect();
            ctx.send_kb("¿Qué tipo de movimiento?", grid(buttons, 2))
                .await?;
        }
        TxStep::Amount => {
            ctx.send("¿Monto? (ej.: 1500, 1.500,50)").await?;
        }
        TxStep::Category => {
            let keyboard = category_keyboard(ctx, catalog, draft.type_uuid).await;
            ctx.send_kb("¿Categoría? Elegí o escribí el nombre.", keyboard)
                .await?;
        }
        TxStep::Account => {
            let buttons = catalog
                .accounts
                .iter()
                .map(|a| {
                    let star = if a.is_default { "⭐ " } else { "" };
                    button(
                        format!("{star}{} ({})", a.name, a.currency_code),
                        Callback::Pick(Some(a.uuid.to_string())),
                    )
                })
                .collect();
            ctx.send_kb("¿Cuenta?", grid(buttons, 2)).await?;
        }
        TxStep::Date => {
            let keyboard = rows(vec![vec![
                button("Hoy", Callback::PickToday),
                button("Ayer", Callback::PickYesterday),
            ]]);
            ctx.send_kb("¿Fecha? Elegí o escribí dd/mm o dd/mm/yyyy.", keyboard)
                .await?;
        }
        TxStep::Description => {
            ctx.send_kb(
                "¿Descripción?",
                rows(vec![vec![button("Omitir", Callback::Skip)]]),
            )
            .await?;
        }
        TxStep::Confirm => {
            let preview = preview(ctx, catalog, draft);
            let keyboard = rows(vec![vec![
                button("✅ Crear", Callback::Confirm),
                button("❌ Cancelar", Callback::Abort),
            ]]);
            ctx.send_kb(format!("¿Confirmás?\n\n{preview}"), keyboard)
                .await?;
        }
    }
    Ok(())
}

fn preview(ctx: &Ctx, catalog: &Catalog, draft: &TxDraft) -> String {
    let date = draft.date.unwrap_or_else(|| ctx.app.today());
    let tx = Transaction {
        uuid: Uuid::nil(),
        amount: draft.amount.unwrap_or_default(),
        description: draft.description.clone().flatten(),
        transaction_date: to_utc(Some(date), ctx.app.config.tz, Utc::now()),
        type_uuid: draft.type_uuid.unwrap_or_default(),
        category_uuid: draft.category.flatten(),
        account_uuid: draft.account,
    };
    render(catalog, &tx, ctx.app.config.tz)
}

/// Categories, most used first for this type (from `/reports/by-category`), then alphabetical.
async fn category_keyboard(
    ctx: &Ctx,
    catalog: &Catalog,
    type_uuid: Option<Uuid>,
) -> InlineKeyboardMarkup {
    let type_code = type_uuid
        .and_then(|uuid| catalog.type_by_uuid(uuid))
        .map(|t| t.code.clone());
    let api = &ctx.app.api;
    let usage: HashMap<Uuid, u64> = match type_code {
        Some(code) => ctx
            .app
            .call(ctx.uid, |auth| {
                let query = CategoryReportQuery {
                    type_code: Some(code.clone()),
                    ..Default::default()
                };
                async move {
                    api.report_by_category(&auth.token, auth.user_uuid, &query)
                        .await
                }
            })
            .await
            .map(|report| {
                report
                    .data
                    .into_iter()
                    .filter_map(|row| Some((row.category_uuid?, row.transaction_count)))
                    .collect()
            })
            .unwrap_or_default(),
        None => HashMap::new(),
    };

    let mut categories: Vec<_> = catalog.categories.iter().collect();
    categories.sort_by(|a, b| {
        usage
            .get(&b.uuid)
            .unwrap_or(&0)
            .cmp(usage.get(&a.uuid).unwrap_or(&0))
    });
    let mut buttons: Vec<InlineKeyboardButton> = categories
        .into_iter()
        .take(MAX_CATEGORY_BUTTONS)
        .map(|c| button(c.name.clone(), Callback::Pick(Some(c.uuid.to_string()))))
        .collect();
    buttons.push(button("Sin categoría", Callback::Pick(None)));
    grid(buttons, 3)
}

pub async fn on_wizard_text(ctx: &Ctx, mut draft: TxDraft, text: &str) -> BotResult {
    let catalog = ctx.catalog().await?;
    match draft.step {
        TxStep::Amount => draft.amount = Some(parse_amount(text).map_err(BotError::user)?),
        TxStep::Date => {
            draft.date = Some(parse_date(text, ctx.app.today()).ok_or_else(|| {
                BotError::user("Fecha inválida. Usá hoy, ayer, dd/mm o dd/mm/yyyy.")
            })?)
        }
        TxStep::Description => {
            draft.description = Some(parse_description(text).map_err(BotError::user)?)
        }
        TxStep::Category => match find_by_name(&catalog.categories, text, |c| c.name.as_str()) {
            Match::One(category) => draft.category = Some(Some(category.uuid)),
            Match::Ambiguous(_) => {
                return Err(BotError::user(
                    "Coincide con varias categorías, elegí de los botones.",
                ));
            }
            Match::None => return offer_new_category(ctx, &catalog, draft, text.trim()).await,
        },
        TxStep::Account => draft.account = Some(resolve_account(&catalog, text)?),
        TxStep::Type => match find_by_name(&catalog.types, text, |t| t.name.as_str()) {
            Match::One(t) => draft.type_uuid = Some(t.uuid),
            _ => return Err(BotError::user("Elegí el tipo de los botones.")),
        },
        TxStep::Confirm => return Err(BotError::user("Usá los botones para confirmar o /cancel.")),
    }
    advance(ctx, draft).await
}

pub async fn on_wizard_callback(
    ctx: &Ctx,
    mut draft: TxDraft,
    callback: Callback,
    message: MessageId,
) -> BotResult {
    let parse_uuid = |value: Option<String>| value.and_then(|v| Uuid::parse_str(&v).ok());
    match (draft.step, callback) {
        (TxStep::Type, Callback::Pick(value)) => draft.type_uuid = parse_uuid(value),
        (TxStep::Category, Callback::Pick(value)) => draft.category = Some(parse_uuid(value)),
        (TxStep::Category, Callback::NewCategory(name)) => {
            let category = create_category(ctx, &name).await?;
            draft.category = Some(Some(category));
        }
        (TxStep::Account, Callback::Pick(value)) => draft.account = parse_uuid(value),
        (TxStep::Date, Callback::PickToday) => draft.date = Some(ctx.app.today()),
        (TxStep::Date, Callback::PickYesterday) => draft.date = ctx.app.today().pred_opt(),
        (TxStep::Description, Callback::Skip) => draft.description = Some(None),
        (TxStep::Confirm, Callback::Confirm) => {
            ctx.clear_keyboard(message).await;
            let catalog = ctx.catalog().await?;
            return create(ctx, &catalog, &draft).await;
        }
        _ => return Err(BotError::user("Esa opción ya no está activa.")),
    }
    ctx.clear_keyboard(message).await;
    advance(ctx, draft).await
}

async fn create_category(ctx: &Ctx, name: &str) -> BotResult<Uuid> {
    let api = &ctx.app.api;
    let category = ctx
        .app
        .call(ctx.uid, |auth| {
            let body = CreateCategory {
                user_uuid: auth.user_uuid,
                name: name.to_string(),
                description: None,
            };
            async move { api.create_category(&auth.token, &body).await }
        })
        .await?;
    ctx.app.invalidate_catalog(ctx.uid);
    Ok(category.uuid)
}

async fn create(ctx: &Ctx, catalog: &Catalog, draft: &TxDraft) -> BotResult {
    let (Some(type_uuid), Some(amount), Some(account)) =
        (draft.type_uuid, draft.amount, draft.account)
    else {
        return Err(BotError::user(
            "Faltan datos de la transacción. Empezá de nuevo con /new.",
        ));
    };
    let body = CreateTransaction {
        amount,
        description: draft.description.clone().flatten(),
        transaction_date: to_utc(draft.date, ctx.app.config.tz, Utc::now()),
        type_uuid,
        category_uuid: draft.category.flatten(),
        account_uuid: account,
    };
    let api = &ctx.app.api;
    let body = &body;
    let tx = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.create_transaction(&auth.token, body).await
        })
        .await?;
    ctx.exit().await?;

    let future_note = if tx.transaction_date > Utc::now() + chrono::Duration::hours(1) {
        "\n⚠️ La fecha es futura."
    } else {
        ""
    };
    ctx.send_kb(
        format!(
            "Listo ✅\n\n{}{future_note}",
            render(catalog, &tx, ctx.app.config.tz)
        ),
        actions(tx.uuid),
    )
    .await?;
    Ok(())
}

// ---------- list ----------

pub async fn list(ctx: &Ctx, args: &str) -> BotResult {
    let catalog = ctx.catalog().await?;
    let filters = parse_list_filters(args, ctx.app.today()).map_err(BotError::user)?;

    let mut query = TransactionQuery {
        date_from: filters.from,
        date_to: filters.to,
        search: filters.search,
        ..Default::default()
    };
    if let Some(name) = &filters.account {
        query.account_uuid = Some(resolve_account(&catalog, name)?);
    }
    if let Some(name) = &filters.category {
        query.category_uuid = match find_by_name(&catalog.categories, name, |c| c.name.as_str()) {
            Match::One(category) => Some(category.uuid),
            _ => {
                return Err(BotError::user(format!(
                    "No encontré la categoría «{name}». Mirá /categories"
                )));
            }
        };
    }
    if let Some(flow) = filters.flow {
        let code = if flow == Flow::Expense {
            EXPENSE_CODE
        } else {
            INCOME_CODE
        };
        query.type_uuid = Some(
            catalog
                .type_by_code(code)
                .ok_or_else(|| BotError::user(format!("No existe el tipo {code}.")))?
                .uuid,
        );
    }

    let json =
        serde_json::to_string(&query).map_err(|error| BotError::Dialogue(error.to_string()))?;
    ctx.app
        .store
        .set_list_state(ctx.uid, REF_KIND, &json)
        .await?;
    let (text, keyboard) = render_page(ctx, &catalog, &query, 1).await?;
    match keyboard {
        Some(keyboard) => ctx.send_kb(text, keyboard).await?,
        None => ctx.send(text).await?,
    };
    Ok(())
}

pub async fn on_page(ctx: &Ctx, kind: ListKind, page: u64, message: MessageId) -> BotResult {
    let ListKind::Transactions = kind;
    let catalog = ctx.catalog().await?;
    let query: TransactionQuery = match ctx.app.store.get_list_state(ctx.uid, REF_KIND).await? {
        Some(json) => serde_json::from_str(&json).unwrap_or_default(),
        None => TransactionQuery::default(),
    };
    let (text, keyboard) = render_page(ctx, &catalog, &query, page).await?;
    ctx.edit(message, text, keyboard).await
}

async fn render_page(
    ctx: &Ctx,
    catalog: &Catalog,
    query: &TransactionQuery,
    page: u64,
) -> BotResult<(String, Option<InlineKeyboardMarkup>)> {
    let api = &ctx.app.api;
    let per_page = ctx.app.config.page_size;
    let result = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.list_transactions(&auth.token, auth.user_uuid, query, page, per_page)
                .await
        })
        .await?;

    let refs: Vec<String> = result.data.iter().map(|tx| tx.uuid.to_string()).collect();
    ctx.app.store.set_refs(ctx.uid, REF_KIND, &refs).await?;

    if result.data.is_empty() {
        return Ok(("No hay movimientos con esos filtros.".into(), None));
    }

    let now = Utc::now();
    let tz = ctx.app.config.tz;
    let mut lines = vec![format!(
        "<b>Movimientos</b> — página {}/{} ({} en total)",
        result.meta.current_page, result.meta.total_pages, result.meta.total_records
    )];
    for (index, tx) in result.data.iter().enumerate() {
        let (symbol, _) = catalog.symbol_for_account(tx.account_uuid);
        let description = tx
            .description
            .as_deref()
            .map(|d| format!(" · {}", escape(&truncate(d, 30))))
            .unwrap_or_default();
        lines.push(format!(
            "{}. {} {} · {}{description}",
            index + 1,
            short_date(tx.transaction_date, tz, now),
            signed_money(tx.amount, &symbol, sign_for(catalog, tx.type_uuid)),
            escape(&category_name(catalog, tx.category_uuid)),
        ));
    }
    lines.push("\n/show N · /edit N · /delete N".into());

    let mut nav = Vec::new();
    if page > 1 {
        nav.push(button(
            "◀",
            Callback::Page(ListKind::Transactions, page - 1),
        ));
    }
    if page < result.meta.total_pages {
        nav.push(button(
            "▶",
            Callback::Page(ListKind::Transactions, page + 1),
        ));
    }
    Ok((lines.join("\n"), (!nav.is_empty()).then(|| rows(vec![nav]))))
}

// ---------- show / edit / delete ----------

async fn resolve_tx(ctx: &Ctx, args: &str, command: &str) -> BotResult<Uuid> {
    match parse_reference(args) {
        Some(Reference::Position(position)) => ctx
            .app
            .store
            .get_ref(ctx.uid, REF_KIND, position)
            .await?
            .and_then(|target| Uuid::parse_str(&target).ok())
            .ok_or_else(|| {
                BotError::user(
                    "Referencia vencida o inexistente: volvé a listar con /transactions.",
                )
            }),
        Some(Reference::Uuid(uuid)) => Ok(uuid),
        _ => Err(usage(&format!("/{command} N (número de /transactions)"))),
    }
}

async fn fetch(ctx: &Ctx, uuid: Uuid) -> BotResult<Transaction> {
    let api = &ctx.app.api;
    ctx.app
        .call(ctx.uid, |auth| async move {
            api.get_transaction(&auth.token, uuid).await
        })
        .await
}

pub async fn show(ctx: &Ctx, args: &str) -> BotResult {
    let uuid = resolve_tx(ctx, args, "show").await?;
    let catalog = ctx.catalog().await?;
    let tx = fetch(ctx, uuid).await?;
    let auth = ctx.auth().await?;
    let text = render(&catalog, &tx, ctx.app.config.tz);
    if can_write(&auth.role) {
        ctx.send_kb(text, actions(uuid)).await?;
    } else {
        ctx.send(text).await?;
    }
    Ok(())
}

pub async fn edit(ctx: &Ctx, args: &str) -> BotResult {
    let uuid = resolve_tx(ctx, args, "edit").await?;
    on_edit_menu(ctx, uuid).await
}

pub async fn on_edit_menu(ctx: &Ctx, uuid: Uuid) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    let catalog = ctx.catalog().await?;
    let tx = fetch(ctx, uuid).await?;
    let field = |label: &str, field: TxField| button(label, Callback::TxEditField(field, uuid));
    let keyboard = rows(vec![
        vec![
            field("Monto", TxField::Amount),
            field("Tipo", TxField::Type),
        ],
        vec![
            field("Categoría", TxField::Category),
            field("Cuenta", TxField::Account),
        ],
        vec![
            field("Fecha", TxField::Date),
            field("Descripción", TxField::Description),
        ],
        vec![button("❌ Cancelar", Callback::Abort)],
    ]);
    ctx.send_kb(
        format!(
            "¿Qué querés cambiar?\n\n{}",
            render(&catalog, &tx, ctx.app.config.tz)
        ),
        keyboard,
    )
    .await?;
    Ok(())
}

pub async fn on_edit_field(ctx: &Ctx, field: TxField, uuid: Uuid) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    let catalog = ctx.catalog().await?;
    match field {
        TxField::Amount | TxField::Date | TxField::Description => {
            ctx.set_state(State::TxEditValue { uuid, field }).await?;
            let question = match field {
                TxField::Amount => "Nuevo monto:",
                TxField::Date => "Nueva fecha (hoy, ayer, dd/mm o dd/mm/yyyy):",
                _ => "Nueva descripción (o «-» para borrarla):",
            };
            ctx.send(question).await?;
        }
        TxField::Type | TxField::Category | TxField::Account => {
            ctx.set_state(State::TxEditPick { uuid, field }).await?;
            let keyboard = match field {
                TxField::Type => grid(
                    catalog
                        .types
                        .iter()
                        .map(|t| button(t.name.clone(), Callback::Pick(Some(t.uuid.to_string()))))
                        .collect(),
                    2,
                ),
                TxField::Category => {
                    let tx = fetch(ctx, uuid).await?;
                    category_keyboard(ctx, &catalog, Some(tx.type_uuid)).await
                }
                _ => grid(
                    catalog
                        .accounts
                        .iter()
                        .map(|a| {
                            button(
                                format!("{} ({})", a.name, a.currency_code),
                                Callback::Pick(Some(a.uuid.to_string())),
                            )
                        })
                        .collect(),
                    2,
                ),
            };
            ctx.send_kb("Elegí el nuevo valor:", keyboard).await?;
        }
    }
    Ok(())
}

pub async fn on_edit_value(ctx: &Ctx, uuid: Uuid, field: TxField, text: &str) -> BotResult {
    let mut body = UpdateTransaction::default();
    match field {
        TxField::Amount => body.amount = Some(parse_amount(text).map_err(BotError::user)?),
        TxField::Date => {
            let date = parse_date(text, ctx.app.today()).ok_or_else(|| {
                BotError::user("Fecha inválida. Usá hoy, ayer, dd/mm o dd/mm/yyyy.")
            })?;
            body.transaction_date = Some(to_utc(Some(date), ctx.app.config.tz, Utc::now()));
        }
        TxField::Description => {
            if text.trim() == "-" {
                return Err(BotError::user(
                    "El backend no permite vaciar la descripción; escribí una nueva o /cancel.",
                ));
            }
            body.description = parse_description(text).map_err(BotError::user)?;
        }
        _ => return Err(BotError::user("Elegí una opción de los botones o /cancel.")),
    }
    patch(ctx, uuid, body).await
}

pub async fn on_edit_pick(
    ctx: &Ctx,
    uuid: Uuid,
    field: TxField,
    callback: Callback,
    message: MessageId,
) -> BotResult {
    let Callback::Pick(value) = callback else {
        return Err(BotError::user("Esa opción ya no está activa."));
    };
    let picked = value.and_then(|v| Uuid::parse_str(&v).ok());
    let mut body = UpdateTransaction::default();
    match field {
        TxField::Type => {
            body.type_uuid = Some(picked.ok_or_else(|| BotError::user("Elegí un tipo."))?)
        }
        TxField::Category => body.category_uuid = Some(picked),
        TxField::Account => {
            body.account_uuid = Some(Some(
                picked.ok_or_else(|| BotError::user("Elegí una cuenta."))?,
            ))
        }
        _ => return Err(BotError::user("Esa opción ya no está activa.")),
    }
    ctx.clear_keyboard(message).await;
    patch(ctx, uuid, body).await
}

async fn patch(ctx: &Ctx, uuid: Uuid, body: UpdateTransaction) -> BotResult {
    let api = &ctx.app.api;
    let body = &body;
    let tx = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.update_transaction(&auth.token, uuid, body).await
        })
        .await?;
    ctx.exit().await?;
    let catalog = ctx.catalog().await?;
    ctx.send_kb(
        format!(
            "Actualizada ✅\n\n{}",
            render(&catalog, &tx, ctx.app.config.tz)
        ),
        actions(uuid),
    )
    .await?;
    Ok(())
}

pub async fn delete(ctx: &Ctx, args: &str) -> BotResult {
    let uuid = resolve_tx(ctx, args, "delete").await?;
    on_delete(ctx, uuid).await
}

pub async fn on_delete(ctx: &Ctx, uuid: Uuid) -> BotResult {
    require(can_write(&ctx.auth().await?.role))?;
    let catalog = ctx.catalog().await?;
    let tx = fetch(ctx, uuid).await?;
    ctx.send_kb(
        format!(
            "¿Borrar esta transacción?\n\n{}",
            render(&catalog, &tx, ctx.app.config.tz)
        ),
        confirm(Callback::TxDeleteYes(uuid)),
    )
    .await?;
    Ok(())
}

pub async fn on_delete_confirmed(ctx: &Ctx, uuid: Uuid, message: MessageId) -> BotResult {
    let api = &ctx.app.api;
    ctx.app
        .call(ctx.uid, |auth| async move {
            api.delete_transaction(&auth.token, uuid).await
        })
        .await?;
    ctx.edit(message, "Transacción borrada 🗑", None).await
}
