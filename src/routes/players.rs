use super::{
    admin::refresh_directory,
    extract::{AdminUser, CurrentUser},
    gifts::GiftView,
    json::ApiPath,
};
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
    pub avatar_url: Option<String>,
    pub gifts: Vec<GiftView>,
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<PlayerRef>>> {
    // Un admin qui ouvre la page relit l'annuaire du SSO : pseudos et photos
    // de tous les joueurs à jour d'un coup, pas seulement de ceux qui se sont
    // connectés depuis. Au mieux : une panne du SSO laisse la liste en base.
    if user.is_admin {
        let admin = AdminUser(user);
        if let Err(e) = refresh_directory(&state, &admin).await {
            tracing::warn!(error = ?e, "annuaire non relu, liste servie depuis la base");
        }
    }
    Ok(Json(
        players::list_visible(&state.pool, &state.config.auth_public_url).await?,
    ))
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
        avatar_url: player.avatar_url(&state.config.auth_public_url),
        pseudo: player.pseudo,
        gifts,
    }))
}
