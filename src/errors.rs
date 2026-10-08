// Copyright (C) 2026 Marcos Gabriel Miller
use crate::api::ApiError;

/// Everything a handler can fail with. `user_message` is the only text a user ever sees.
#[derive(Debug, thiserror::Error)]
pub enum BotError {
    #[error("not logged in")]
    NotLoggedIn,
    #[error("session expired")]
    SessionExpired,
    #[error("forbidden")]
    Forbidden,
    /// A message meant for the user as-is: usage help, validation, "not found" refs…
    #[error("{0}")]
    User(String),
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error("storage error: {0}")]
    Store(#[from] sqlx::Error),
    #[error("telegram error: {0}")]
    Telegram(#[from] teloxide::RequestError),
    #[error("dialogue storage error: {0}")]
    Dialogue(String),
}

pub type BotResult<T = ()> = Result<T, BotError>;

impl BotError {
    pub fn user(message: impl Into<String>) -> Self {
        BotError::User(message.into())
    }

    pub fn user_message(&self) -> String {
        match self {
            BotError::NotLoggedIn => "Iniciá sesión con /login.".into(),
            BotError::SessionExpired => {
                "Tu sesión venció. Enviá /login para volver a entrar.".into()
            }
            BotError::Forbidden => "No tenés permisos para esa operación.".into(),
            BotError::User(message) => message.clone(),
            BotError::Api(ApiError::Http {
                status, message, ..
            }) => match status {
                400 => format!("Datos inválidos: {message}"),
                401 => "Tu sesión venció. Enviá /login para volver a entrar.".into(),
                403 => "No tenés permisos para esa operación.".into(),
                404 => "No encontré ese registro (¿fue borrado?).".into(),
                409 => message.clone(),
                429 => "Demasiadas solicitudes, probá en un rato.".into(),
                _ => "El servidor no responde, probá en un rato.".into(),
            },
            BotError::Api(_) => "El servidor no responde, probá en un rato.".into(),
            BotError::Store(_) | BotError::Telegram(_) | BotError::Dialogue(_) => {
                "Ocurrió un error interno, probá de nuevo.".into()
            }
        }
    }

    /// Internal failures worth an `error` log; user mistakes are not.
    pub fn is_internal(&self) -> bool {
        match self {
            BotError::Store(_) | BotError::Telegram(_) | BotError::Dialogue(_) => true,
            BotError::Api(ApiError::Http { status, .. }) => *status >= 500,
            BotError::Api(_) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, message: &str) -> BotError {
        BotError::Api(ApiError::Http {
            status,
            code: String::new(),
            message: message.into(),
        })
    }

    #[test]
    fn maps_api_errors_to_spanish_messages() {
        assert_eq!(
            http(403, "x").user_message(),
            "No tenés permisos para esa operación."
        );
        assert_eq!(
            http(404, "x").user_message(),
            "No encontré ese registro (¿fue borrado?)."
        );
        assert_eq!(
            http(400, "name too long").user_message(),
            "Datos inválidos: name too long"
        );
        assert_eq!(
            http(409, "Account has 3 transactions").user_message(),
            "Account has 3 transactions"
        );
        assert_eq!(
            http(500, "boom").user_message(),
            "El servidor no responde, probá en un rato."
        );
        assert!(http(500, "boom").is_internal());
        assert!(!http(404, "x").is_internal());
        assert!(BotError::Api(ApiError::Transport("timeout".into())).is_internal());
    }
}
