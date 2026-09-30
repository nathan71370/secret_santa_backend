use super::{extract::CurrentUser, gifts::GiftView, json::ApiPath};
use crate::{
    error::{ApiError, ApiResult},
    repo::{
        gifts,
        players::{self, PlayerRef},
    },
    state::AppState,
};
use axum::{Json, extract::State};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub player_id: i64,
    pub pseudo: String,
    pub gifts: Vec<GiftView>,
}

pub async fn list(
    State(state): State<AppState>,
    _user: CurrentUser,
) -> ApiResult<Json<Vec<PlayerRef>>> {
    Ok(Json(players::list_visible(&state.pool).await?))
}

pub async fn profile(
    State(state): State<AppState>,
    _user: CurrentUser,
    ApiPath(id): ApiPath<i64>,
) -> ApiResult<Json<ProfileView>> {
    let player = players::find(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Joueur introuvable."))?;
    let gifts = gifts::list_for(&state.pool, id)
        .await?
        .into_iter()
        .map(GiftView::from)
        .collect();
    Ok(Json(ProfileView {
        player_id: player.id,
        pseudo: player.pseudo,
        gifts,
    }))
}
