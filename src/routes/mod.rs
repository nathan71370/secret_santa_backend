use axum::{Router, routing::get};

pub async fn health() -> &'static str {
    "ok"
}

/// Remplacé en Task 7 par `router(state)`.
pub fn router() -> Router {
    Router::new().nest("/api", Router::new().route("/health", get(health)))
}
