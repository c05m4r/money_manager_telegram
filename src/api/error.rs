// Copyright (C) 2026 Marcos Gabriel Miller
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The backend answered with a non-2xx status and its `{error, message}` body.
    #[error("HTTP {status} {code}: {message}")]
    Http {
        status: u16,
        code: String,
        message: String,
    },
    /// Connection, timeout or TLS failure.
    #[error("transport error: {0}")]
    Transport(String),
    /// The response body did not match the expected shape.
    #[error("unexpected response: {0}")]
    Decode(String),
}

impl ApiError {
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn is_unauthorized(&self) -> bool {
        self.status() == Some(401)
    }

    pub(crate) fn from_body(status: u16, body: &str) -> Self {
        #[derive(Deserialize)]
        struct ErrorBody {
            error: Option<String>,
            message: Option<String>,
        }
        let parsed = serde_json::from_str::<ErrorBody>(body).ok();
        ApiError::Http {
            status,
            code: parsed
                .as_ref()
                .and_then(|b| b.error.clone())
                .unwrap_or_default(),
            message: parsed
                .and_then(|b| b.message)
                .unwrap_or_else(|| body.chars().take(200).collect()),
        }
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(error: reqwest::Error) -> Self {
        // reqwest errors can include the URL but never the Authorization header.
        ApiError::Transport(error.without_url().to_string())
    }
}
