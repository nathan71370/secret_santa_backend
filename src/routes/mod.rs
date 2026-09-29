pub mod extract;
pub mod me;

use crate::state::AppState;
use axum::{Router, extract::DefaultBodyLimit, routing::get};
use tower_http::trace::TraceLayer;

pub async fn health() -> &'static str {
    "ok"
}

pub fn router(state: AppState) -> Router {
    let api: Router<AppState> = Router::new()
        .route("/health", get(health))
        .route("/me", get(me::me));

    Router::new()
        .nest("/api", api)
        // Photos (5 Mo) + marge pour l'enveloppe multipart.
        .layer(DefaultBodyLimit::max(6 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
