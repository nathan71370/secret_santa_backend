use super::{
    editions::{EditionView, edition_view, require_current},
    extract::AdminUser,
    json::{ApiJson, ApiPath},
};
use crate::{
    APP_SLUG,
    auth::AuthError,
    error::{ApiError, ApiResult},
    repo::{
        assignments::{self, MailRow, MailStatus},
        editions::{self, EditionStatus},
        players,
    },
    services::{
        draw_service::{self, DrawOutcome},
        mailing,
    },
    state::AppState,
};
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Europe::Paris;
use serde::{Deserialize, Serialize};

fn auth_error(e: AuthError) -> ApiError {
    match e {
        AuthError::Forbidden => ApiError::Forbidden,
        AuthError::Unavailable(detail) => {
            tracing::warn!(%detail, "annuaire SSO indisponible");
            ApiError::Upstream
        }
    }
}

fn validate_budget(budget: i64) -> ApiResult<()> {
    if (1..=1000).contains(&budget) {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "INVALID_BUDGET",
            "Le budget doit être compris entre 1 et 1000 €.",
        ))
    }
}

/// Future, et avant le 1er janvier suivant (minuit à Paris) : une édition
/// appartient à son année civile.
fn validate_deadline(state: &AppState, deadline: DateTime<Utc>) -> ApiResult<()> {
    if deadline <= state.now() {
        return Err(ApiError::bad_request(
            "INVALID_DEADLINE",
            "La deadline doit être dans le futur.",
        ));
    }
    let next_year =
        i32::try_from(state.current_year() + 1).map_err(|e| ApiError::Internal(e.to_string()))?;
    let year_end = Paris
        .with_ymd_and_hms(next_year, 1, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| ApiError::Internal("1er janvier introuvable à Paris".into()))?;
    if deadline >= year_end {
        return Err(ApiError::bad_request(
            "INVALID_DEADLINE",
            "La deadline doit tomber avant le 1er janvier.",
        ));
    }
    Ok(())
}

/// Rafraîchit l'instantané des comptes ayant accès à l'app, au nom de l'admin.
async fn refresh_directory(state: &AppState, admin: &AdminUser) -> ApiResult<()> {
    let users = state
        .auth
        .list_users(&admin.0.token, APP_SLUG)
        .await
        .map_err(auth_error)?;
    players::sync_access(&state.pool, &users, state.now()).await?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub player_id: i64,
    pub pseudo: String,
    pub email: String,
    pub registration: Option<bool>,
    pub answered_by_admin: bool,
}

pub async fn sso_users(
    State(state): State<AppState>,
    admin: AdminUser,
) -> ApiResult<Json<Vec<DirectoryEntry>>> {
    refresh_directory(&state, &admin).await?;
    let regs = match super::editions::current_edition(&state).await? {
        Some(e) => editions::registrations(&state.pool, e.id).await?,
        None => Default::default(),
    };
    let entries = players::with_access(&state.pool)
        .await?
        .into_iter()
        .map(|p| {
            let reg = regs.get(&p.id).copied();
            DirectoryEntry {
                player_id: p.id,
                pseudo: p.pseudo,
                email: p.email,
                registration: reg.map(|(participating, _)| participating),
                answered_by_admin: reg.is_some_and(|(_, by_admin)| by_admin),
            }
        })
        .collect();
    Ok(Json(entries))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEdition {
    pub budget_eur: i64,
    pub deadline: DateTime<Utc>,
}

pub async fn create_edition(
    State(state): State<AppState>,
    admin: AdminUser,
    ApiJson(body): ApiJson<CreateEdition>,
) -> ApiResult<(StatusCode, Json<EditionView>)> {
    validate_budget(body.budget_eur)?;
    validate_deadline(&state, body.deadline)?;
    let year = state.current_year();
    if editions::by_year(&state.pool, year).await?.is_some() {
        return Err(ApiError::conflict(
            "EDITION_EXISTS",
            format!("L'édition {year} existe déjà."),
        ));
    }
    refresh_directory(&state, &admin).await?;
    let edition = editions::create_open(
        &state.pool,
        year,
        body.budget_eur,
        body.deadline,
        state.now(),
    )
    .await?;
    state.spawn(mailing::announce(state.clone(), edition.id));
    Ok((
        StatusCode::CREATED,
        Json(edition_view(&state, &edition, &admin.0).await?),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchEdition {
    pub budget_eur: Option<i64>,
    pub deadline: Option<DateTime<Utc>>,
}

pub async fn patch_edition(
    State(state): State<AppState>,
    admin: AdminUser,
    ApiJson(body): ApiJson<PatchEdition>,
) -> ApiResult<Json<EditionView>> {
    let e = require_current(&state).await?;
    if e.status == EditionStatus::Drawn {
        return Err(ApiError::conflict(
            "EDITION_DRAWN",
            "Le tirage a déjà eu lieu.",
        ));
    }
    if let Some(budget) = body.budget_eur {
        validate_budget(budget)?;
    }
    if let Some(deadline) = body.deadline {
        validate_deadline(&state, deadline)?;
    }
    if let Some(budget) = body.budget_eur {
        editions::update_budget(&state.pool, e.id, budget).await?;
    }
    if let Some(deadline) = body.deadline {
        editions::update_deadline(&state.pool, e.id, deadline).await?;
    }
    let e = require_current(&state).await?;
    Ok(Json(edition_view(&state, &e, &admin.0).await?))
}

#[derive(Deserialize)]
pub struct AdminRegistration {
    #[serde(default)]
    pub participating: Option<bool>,
}

pub async fn put_registration_for(
    State(state): State<AppState>,
    _admin: AdminUser,
    ApiPath(player_id): ApiPath<i64>,
    ApiJson(body): ApiJson<AdminRegistration>,
) -> ApiResult<StatusCode> {
    let e = require_current(&state).await?;
    if e.status != EditionStatus::Open {
        return Err(ApiError::conflict(
            "REGISTRATION_CLOSED",
            "Les inscriptions sont closes.",
        ));
    }
    if players::find(&state.pool, player_id).await?.is_none() {
        return Err(ApiError::not_found("Joueur inconnu."));
    }
    editions::set_registration(
        &state.pool,
        e.id,
        player_id,
        body.participating,
        true,
        state.now(),
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn draw_now(
    State(state): State<AppState>,
    admin: AdminUser,
) -> ApiResult<Json<EditionView>> {
    let e = require_current(&state).await?;
    if e.status == EditionStatus::Drawn {
        return Err(ApiError::conflict(
            "ALREADY_DRAWN",
            "Le tirage a déjà eu lieu.",
        ));
    }
    match draw_service::run_draw(&state, e.id).await? {
        DrawOutcome::Drawn => {
            let e = require_current(&state).await?;
            Ok(Json(edition_view(&state, &e, &admin.0).await?))
        }
        DrawOutcome::AlreadyDone => Err(ApiError::conflict(
            "ALREADY_DRAWN",
            "Le tirage a déjà eu lieu.",
        )),
        DrawOutcome::Failed(reason) => Err(ApiError::unprocessable("DRAW_FAILED", reason)),
    }
}

#[derive(Deserialize)]
pub struct CancelDraw {
    pub deadline: DateTime<Utc>,
}

pub async fn cancel_draw(
    State(state): State<AppState>,
    admin: AdminUser,
    ApiJson(body): ApiJson<CancelDraw>,
) -> ApiResult<Json<EditionView>> {
    validate_deadline(&state, body.deadline)?;
    let e = require_current(&state).await?;
    if !draw_service::cancel_draw(&state, e.id, body.deadline).await? {
        return Err(ApiError::conflict(
            "NOT_DRAWN",
            "Il n'y a pas de tirage à annuler.",
        ));
    }
    let e = require_current(&state).await?;
    Ok(Json(edition_view(&state, &e, &admin.0).await?))
}

pub async fn mails(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> ApiResult<Json<Vec<MailRow>>> {
    let e = require_current(&state).await?;
    Ok(Json(assignments::mail_statuses(&state.pool, e.id).await?))
}

pub async fn resend(
    State(state): State<AppState>,
    _admin: AdminUser,
    ApiPath(player_id): ApiPath<i64>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let e = require_current(&state).await?;
    if !assignments::set_mail_status(&state.pool, e.id, player_id, MailStatus::Pending, None)
        .await?
    {
        return Err(ApiError::not_found(
            "Ce joueur ne fait pas partie du tirage.",
        ));
    }
    let task_state = state.clone();
    state.spawn(async move {
        if let Err(err) = mailing::send_result(&task_state, &e, player_id).await {
            tracing::error!(error = %err, "renvoi impossible");
        }
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "mailStatus": "pending" })),
    ))
}
