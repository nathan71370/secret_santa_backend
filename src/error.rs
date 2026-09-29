use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Connexion requise.")]
    Unauthorized { login_url: String },
    #[error("Accès refusé.")]
    Forbidden,
    #[error("{0}")]
    NotFound(String),
    #[error("{message}")]
    BadRequest { code: &'static str, message: String },
    #[error("{message}")]
    Conflict { code: &'static str, message: String },
    #[error("{message}")]
    Unprocessable { code: &'static str, message: String },
    #[error("Le service d'identité ne répond pas, réessaie dans un instant.")]
    Upstream,
    #[error("Erreur interne.")]
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::BadRequest { code, message: message.into() }
    }
    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::Conflict { code, message: message.into() }
    }
    pub fn unprocessable(code: &'static str, message: impl Into<String>) -> Self {
        Self::Unprocessable { code, message: message.into() }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::Unauthorized { .. } => (StatusCode::UNAUTHORIZED, "NOT_AUTHENTICATED"),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::BadRequest { code, .. } => (StatusCode::BAD_REQUEST, *code),
            ApiError::Conflict { code, .. } => (StatusCode::CONFLICT, *code),
            ApiError::Unprocessable { code, .. } => (StatusCode::UNPROCESSABLE_ENTITY, *code),
            ApiError::Upstream => (StatusCode::BAD_GATEWAY, "UPSTREAM"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
        };
        if let ApiError::Internal(detail) = &self {
            tracing::error!(%detail, "erreur interne");
        }
        let mut body = json!({ "error": code, "message": self.to_string() });
        if let ApiError::Unauthorized { login_url } = &self {
            body["loginUrl"] = json!(login_url);
        }
        (status, Json(body)).into_response()
    }
}
