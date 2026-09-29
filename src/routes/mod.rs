pub mod admin;
pub mod editions;
pub mod extract;
pub mod json;
pub mod me;

use crate::state::AppState;
use axum::{Router, extract::DefaultBodyLimit, routing::{get, patch, post, put}};
use tower_http::trace::TraceLayer;

pub async fn health() -> &'static str {
    "ok"
}

pub fn router(state: AppState) -> Router {
    let api: Router<AppState> = Router::new()
        .route("/health", get(health))
        .route("/me", get(me::me))
        .route("/editions/current", get(editions::get_current))
        .route("/editions/current/registration", put(editions::put_registration))
        .route("/admin/sso-users", get(admin::sso_users))
        .route("/admin/editions", post(admin::create_edition))
        .route("/admin/editions/current", patch(admin::patch_edition))
        .route("/admin/editions/current/registrations/{player_id}", put(admin::put_registration_for))
        .route("/editions/current/my-target", get(editions::my_target))
        .route("/admin/editions/current/draw", post(admin::draw_now).delete(admin::cancel_draw))
        .route("/admin/editions/current/mails", get(admin::mails))
        .route("/admin/editions/current/mails/{player_id}/resend", post(admin::resend));

    Router::new()
        .nest("/api", api)
        // Photos (5 Mo) + marge pour l'enveloppe multipart.
        .layer(DefaultBodyLimit::max(6 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
