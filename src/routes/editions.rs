use super::{extract::CurrentUser, json::ApiJson};
use crate::{
    error::{ApiError, ApiResult},
    repo::{
        editions::{self, Edition, EditionStatus},
        players::PlayerRef,
    },
    state::AppState,
};
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionView {
    pub year: i64,
    pub budget_eur: i64,
    pub deadline: DateTime<Utc>,
    pub status: EditionStatus,
    /// Réservé à l'admin.
    pub failure_reason: Option<String>,
    pub drawn_at: Option<DateTime<Utc>>,
    pub participants: Vec<PlayerRef>,
    pub my_registration: Option<bool>,
    pub imported: bool,
}

pub async fn edition_view(
    state: &AppState,
    e: &Edition,
    viewer: &CurrentUser,
) -> ApiResult<EditionView> {
    Ok(EditionView {
        year: e.year,
        budget_eur: e.budget_eur,
        deadline: e.deadline,
        status: e.status,
        failure_reason: if viewer.is_admin {
            e.failure_reason.clone()
        } else {
            None
        },
        drawn_at: e.drawn_at,
        participants: editions::participants(&state.pool, e.id).await?,
        my_registration: editions::registration_of(&state.pool, e.id, viewer.player_id).await?,
        imported: e.imported,
    })
}

pub async fn current_edition(state: &AppState) -> ApiResult<Option<Edition>> {
    Ok(editions::by_year(&state.pool, state.current_year()).await?)
}

pub async fn require_current(state: &AppState) -> ApiResult<Edition> {
    current_edition(state)
        .await?
        .ok_or_else(|| ApiError::not_found("Aucune édition cette année."))
}

pub async fn get_current(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Option<EditionView>>> {
    match current_edition(&state).await? {
        None => Ok(Json(None)),
        Some(e) => Ok(Json(Some(edition_view(&state, &e, &user).await?))),
    }
}

#[derive(Deserialize)]
pub struct RegistrationBody {
    pub participating: bool,
}

pub async fn put_registration(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiJson(body): ApiJson<RegistrationBody>,
) -> ApiResult<Json<EditionView>> {
    let e = require_current(&state).await?;
    if !editions::register_while_open(
        &state.pool,
        e.id,
        user.player_id,
        body.participating,
        state.now(),
    )
    .await?
    {
        return Err(ApiError::conflict(
            "REGISTRATION_CLOSED",
            "Les inscriptions sont closes.",
        ));
    }
    Ok(Json(edition_view(&state, &e, &user).await?))
}

pub async fn my_target(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<PlayerRef>> {
    let not_found = || ApiError::not_found("Pas de cible pour toi cette année.");
    let e = current_edition(&state).await?.ok_or_else(not_found)?;
    if e.status != EditionStatus::Drawn {
        return Err(not_found());
    }
    let target = crate::repo::assignments::target_of(&state.pool, e.id, user.player_id).await?;
    target.map(Json).ok_or_else(not_found)
}
