// Copyright (C) 2026 Marcos Gabriel Miller
use teloxide::{types::BotCommand, utils::command::BotCommands};

#[derive(BotCommands, Clone, Debug, PartialEq)]
#[command(rename_rule = "snake_case")]
pub enum Command {
    #[command(description = "inicio")]
    Start,
    #[command(description = "ayuda y ejemplos")]
    Help,
    #[command(description = "cancelar la operación en curso")]
    Cancel,
    #[command(description = "iniciar sesión")]
    Login(String),
    #[command(description = "cerrar sesión")]
    Logout,
    #[command(description = "mi perfil")]
    Me,

    #[command(description = "gasto rápido: /expense 1500 #cafe @cuenta descripción")]
    Expense(String),
    #[command(description = "ingreso rápido: /income 250000 #sueldo")]
    Income(String),
    #[command(description = "nueva transacción paso a paso")]
    New,
    #[command(
        description = "listar movimientos (filtros: @cuenta #cat expenses income month:mm/yyyy)"
    )]
    Transactions(String),
    #[command(description = "ver transacción: /show 3")]
    Show(String),
    #[command(description = "editar transacción: /edit 3")]
    Edit(String),
    #[command(description = "borrar transacción: /delete 3")]
    Delete(String),

    #[command(description = "mis cuentas y saldos")]
    Accounts,
    #[command(description = "nueva cuenta: /account_new Visa USD")]
    AccountNew(String),
    #[command(description = "editar cuenta: /account_edit 2")]
    AccountEdit(String),
    #[command(description = "borrar cuenta: /account_delete 2")]
    AccountDelete(String),
    #[command(description = "cuenta por defecto: /account_default 2")]
    AccountDefault(String),

    #[command(description = "mis categorías")]
    Categories,
    #[command(description = "nueva categoría: /category_new Mascotas")]
    CategoryNew(String),
    #[command(description = "editar categoría: /category_edit 3")]
    CategoryEdit(String),
    #[command(description = "borrar categoría: /category_delete 3")]
    CategoryDelete(String),

    #[command(description = "monedas")]
    Currencies,
    #[command(description = "nueva moneda: /currency_new BRL R$ Real brasileño")]
    CurrencyNew(String),
    #[command(description = "editar moneda: /currency_edit BRL")]
    CurrencyEdit(String),
    #[command(description = "borrar moneda: /currency_delete BRL")]
    CurrencyDelete(String),

    #[command(description = "tipos de transacción")]
    Types,
    #[command(description = "nuevo tipo: /type_new transfer Transferencia")]
    TypeNew(String),
    #[command(description = "editar tipo: /type_edit 3")]
    TypeEdit(String),
    #[command(description = "borrar tipo: /type_delete 3")]
    TypeDelete(String),

    #[command(description = "saldo por cuenta: /balance [@cuenta]")]
    Balance(String),
    #[command(description = "resumen mensual: /summary [mm/yyyy] [@cuenta]")]
    Summary(String),
}

pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_MANAGER: &str = "manager";
pub const ROLE_AUDITOR: &str = "auditor";

/// Read-only role: no create/update/delete commands.
pub fn can_write(role: &str) -> bool {
    role != ROLE_AUDITOR
}

/// Roles allowed to manage currencies and transaction types (backend Casbin policy).
pub fn can_manage_catalogs(role: &str) -> bool {
    role == ROLE_ADMIN || role == ROLE_MANAGER
}

const PUBLIC: &[&str] = &["start", "help", "cancel", "login"];
const READ: &[&str] = &[
    "logout",
    "me",
    "transactions",
    "show",
    "accounts",
    "categories",
    "currencies",
    "types",
    "balance",
    "summary",
];
const WRITE: &[&str] = &[
    "expense",
    "income",
    "new",
    "edit",
    "delete",
    "account_new",
    "account_edit",
    "account_delete",
    "account_default",
    "category_new",
    "category_edit",
    "category_delete",
];
const CATALOG_ADMIN: &[&str] = &[
    "currency_new",
    "currency_edit",
    "currency_delete",
    "type_new",
    "type_edit",
    "type_delete",
];

/// Commands shown in the Telegram menu for a role (`None` = not logged in).
/// UX only: the backend remains the authority.
pub fn commands_for(role: Option<&str>) -> Vec<BotCommand> {
    let mut allowed: Vec<&str> = PUBLIC.to_vec();
    if let Some(role) = role {
        allowed.retain(|name| *name != "login");
        allowed.extend(READ);
        if can_write(role) {
            allowed.extend(WRITE);
        }
        if can_manage_catalogs(role) {
            allowed.extend(CATALOG_ADMIN);
        }
    }
    Command::bot_commands()
        .into_iter()
        .filter(|command| allowed.contains(&command.command.trim_start_matches('/')))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(role: Option<&str>) -> Vec<String> {
        commands_for(role)
            .into_iter()
            .map(|c| c.command.trim_start_matches('/').to_string())
            .collect()
    }

    #[test]
    fn parses_snake_case_commands() {
        assert_eq!(
            Command::parse("/account_new Visa USD", "bot").unwrap(),
            Command::AccountNew("Visa USD".into())
        );
        assert_eq!(
            Command::parse("/expense 1500 #cafe", "bot").unwrap(),
            Command::Expense("1500 #cafe".into())
        );
        assert_eq!(Command::parse("/new", "bot").unwrap(), Command::New);
        assert_eq!(
            Command::parse("/transactions", "bot").unwrap(),
            Command::Transactions(String::new())
        );
    }

    #[test]
    fn menu_per_role() {
        let anonymous = names(None);
        assert!(anonymous.contains(&"login".to_string()));
        assert!(!anonymous.contains(&"expense".to_string()));

        let user = names(Some("user"));
        assert!(user.contains(&"expense".to_string()) && user.contains(&"logout".to_string()));
        assert!(
            !user.contains(&"login".to_string()) && !user.contains(&"currency_new".to_string())
        );

        let auditor = names(Some("auditor"));
        assert!(auditor.contains(&"transactions".to_string()));
        assert!(!auditor.contains(&"expense".to_string()));

        assert!(names(Some("manager")).contains(&"type_new".to_string()));
        assert!(names(Some("admin")).contains(&"currency_delete".to_string()));
        // Every command is reachable by some role; admins see all but /login.
        assert_eq!(
            names(Some("admin")).len(),
            Command::bot_commands().len() - 1
        );
    }
}
