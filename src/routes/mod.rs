pub mod admin;
pub mod editions;
pub mod extract;
pub mod gifts;
pub mod import;
pub mod json;
pub mod me;
pub mod players;

use crate::{error::ApiError, state::AppState};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, patch, post, put},
};
use tower_http::trace::TraceLayer;

pub async fn health() -> &'static str {
    "ok"
}

async fn not_found() -> ApiError {
    ApiError::not_found("Ressource introuvable.")
}

async fn method_not_allowed() -> ApiError {
    ApiError::MethodNotAllowed
}

pub fn router(state: AppState) -> Router {
    let api: Router<AppState> = Router::new()
        .route("/health", get(health))
        .route("/me", get(me::me))
        .route("/players", get(players::list))
        .route("/players/{id}", get(players::profile))
        .route("/me/gifts", post(gifts::create))
        .route("/me/gifts/{id}", put(gifts::update).delete(gifts::delete))
        .route("/uploads/{file}", get(gifts::upload))
        .route("/editions/current", get(editions::get_current))
        .route(
            "/editions/current/registration",
            put(editions::put_registration),
        )
        .route("/admin/import", post(import::import))
        .route("/admin/sso-users", get(admin::sso_users))
        .route("/admin/players", get(admin::all_players))
        .route("/admin/players/{id}/merge", post(admin::merge_player))
        .route("/admin/editions", post(admin::create_edition))
        .route("/admin/editions/current", patch(admin::patch_edition))
        .route(
            "/admin/editions/current/registrations/{player_id}",
            put(admin::put_registration_for),
        )
        .route("/editions/current/my-target", get(editions::my_target))
        .route(
            "/admin/editions/current/draw",
            post(admin::draw_now).delete(admin::cancel_draw),
        )
        .route("/admin/editions/current/mails", get(admin::mails))
        .route(
            "/admin/editions/current/mails/{player_id}/resend",
            post(admin::resend),
        )
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed);

    Router::new()
        .nest("/api", api)
        // Photos (5 Mo) + marge pour l'enveloppe multipart.
        .layer(DefaultBodyLimit::max(6 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
