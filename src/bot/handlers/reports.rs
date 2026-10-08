// Copyright (C) 2026 Marcos Gabriel Miller
use std::collections::BTreeMap;

use chrono::Datelike;
use rust_decimal::Decimal;

use super::Ctx;
use crate::{
    api::models::{CategoryReportQuery, CategoryTotal},
    bot::format::{escape, money, number},
    errors::{BotError, BotResult},
    services::{
        matching::{Match, find_by_name},
        parse::{month_bounds, parse_month},
    },
    session::catalog::{Catalog, EXPENSE_CODE, INCOME_CODE},
};

const MAX_SUMMARY_ROWS: usize = 10;

fn account_arg(catalog: &Catalog, token: &str) -> BotResult<uuid::Uuid> {
    let name = token.trim_start_matches('@');
    match find_by_name(&catalog.accounts, name, |a| a.name.as_str()) {
        Match::One(account) => Ok(account.uuid),
        _ => Err(BotError::user(format!(
            "No encontré la cuenta «{name}». Mirá /accounts"
        ))),
    }
}

fn symbol(catalog: &Catalog, code: &str) -> String {
    catalog
        .currency(code)
        .map(|c| c.symbol.clone())
        .unwrap_or_else(|| code.to_string())
}

/// `/balance [@account]`
pub async fn balance(ctx: &Ctx, args: &str) -> BotResult {
    let catalog = ctx.catalog().await?;
    let account = match args.split_whitespace().next() {
        Some(token) => Some(account_arg(&catalog, token)?),
        None => None,
    };
    let api = &ctx.app.api;
    let report = ctx
        .app
        .call(ctx.uid, |auth| async move {
            api.report_balance(&auth.token, auth.user_uuid, account)
                .await
        })
        .await?;
    if report.data.is_empty() {
        ctx.send("No tenés cuentas. Creá una con /account_new Nombre ARS")
            .await?;
        return Ok(());
    }

    let mut lines = vec!["<b>Saldos</b>".to_string()];
    for row in &report.data {
        let symbol = symbol(&catalog, &row.currency_code);
        lines.push(format!(
            "\n<b>{}</b> ({})\n  Ingresos: {}\n  Gastos: {}\n  Saldo: <b>{}</b>",
            escape(&row.account_name),
            escape(&row.currency_code),
            money(row.income, &symbol),
            money(row.expense, &symbol),
            money(row.balance, &symbol),
        ));
        if !row.other.is_zero() {
            lines.push(format!(
                "  Otros movimientos: {}",
                money(row.other, &symbol)
            ));
        }
    }
    if account.is_none() && report.data.len() > 1 {
        lines.push("\n<b>Total por moneda</b>".into());
        for total in &report.totals {
            lines.push(format!(
                "{}: {}",
                escape(&total.currency_code),
                money(total.balance, &symbol(&catalog, &total.currency_code))
            ));
        }
    }
    ctx.send(lines.join("\n")).await?;
    Ok(())
}

/// `/summary [mm/yyyy] [@account]`: expenses per category for a month, plus income and net.
pub async fn summary(ctx: &Ctx, args: &str) -> BotResult {
    let catalog = ctx.catalog().await?;
    let today = ctx.app.today();
    let (mut year, mut month) = (today.year(), today.month());
    let mut account = catalog.default_account().map(|a| a.uuid);
    for token in args.split_whitespace() {
        if token.starts_with('@') {
            account = Some(account_arg(&catalog, token)?);
        } else {
            (year, month) = parse_month(token)
                .ok_or_else(|| BotError::user("Uso: /summary [mm/yyyy] [@cuenta]"))?;
        }
    }
    let (from, to) = month_bounds(year, month, ctx.app.config.tz)
        .ok_or_else(|| BotError::user("Mes inválido."))?;

    let api = &ctx.app.api;
    let query = |code: &str| CategoryReportQuery {
        account_uuid: account,
        from: Some(from),
        to: Some(to),
        type_code: Some(code.into()),
    };
    let (expenses, income) = (query(EXPENSE_CODE), query(INCOME_CODE));
    let (expenses, income) = (&expenses, &income);
    let (expenses, income) = ctx
        .app
        .call(ctx.uid, |auth| async move {
            tokio::try_join!(
                api.report_by_category(&auth.token, auth.user_uuid, expenses),
                api.report_by_category(&auth.token, auth.user_uuid, income),
            )
        })
        .await?;

    let account_name = account
        .and_then(|uuid| catalog.account(uuid))
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "todas".into());
    let mut lines = vec![format!(
        "<b>Resumen {month:02}/{year}</b> — cuenta: {}",
        escape(&account_name)
    )];

    if expenses.data.is_empty() && income.data.is_empty() {
        lines.push("\nSin movimientos en el mes.".into());
        ctx.send(lines.join("\n")).await?;
        return Ok(());
    }

    let mut by_currency: BTreeMap<String, Vec<&CategoryTotal>> = BTreeMap::new();
    for row in &expenses.data {
        by_currency
            .entry(row.currency_code.clone())
            .or_default()
            .push(row);
    }
    for (code, rows) in &by_currency {
        let symbol = symbol(&catalog, code);
        lines.push(format!("\n<b>Gastos por categoría ({})</b>", escape(code)));
        for row in rows.iter().take(MAX_SUMMARY_ROWS) {
            lines.push(format!(
                "{} — {} ({}%)",
                escape(row.category_name.as_deref().unwrap_or("Sin categoría")),
                money(row.total, &symbol),
                number(row.percentage).trim_end_matches(",00"),
            ));
        }
        if rows.len() > MAX_SUMMARY_ROWS {
            let rest: Decimal = rows
                .iter()
                .skip(MAX_SUMMARY_ROWS)
                .map(|row| row.total)
                .sum();
            lines.push(format!("Otras — {}", money(rest, &symbol)));
        }
    }

    let mut currencies: Vec<String> = expenses
        .data
        .iter()
        .chain(&income.data)
        .map(|row| row.currency_code.clone())
        .collect();
    currencies.sort();
    currencies.dedup();
    lines.push("\n<b>Totales</b>".into());
    for code in currencies {
        let symbol = symbol(&catalog, &code);
        let spent: Decimal = expenses
            .data
            .iter()
            .filter(|r| r.currency_code == code)
            .map(|r| r.total)
            .sum();
        let earned: Decimal = income
            .data
            .iter()
            .filter(|r| r.currency_code == code)
            .map(|r| r.total)
            .sum();
        lines.push(format!(
            "{}: ingresos {} · gastos {} · neto <b>{}</b>",
            escape(&code),
            money(earned, &symbol),
            money(spent, &symbol),
            money(earned - spent, &symbol),
        ));
    }
    ctx.send(lines.join("\n")).await?;
    Ok(())
}
