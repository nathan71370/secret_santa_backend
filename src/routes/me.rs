use super::extract::CurrentUser;
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeView {
    pub player_id: i64,
    pub pseudo: String,
    pub avatar_url: Option<String>,
    pub email: String,
    pub is_admin: bool,
}

pub async fn me(user: CurrentUser) -> Json<MeView> {
    Json(MeView {
        player_id: user.player_id,
        pseudo: user.pseudo,
        avatar_url: user.avatar_url,
        email: user.email,
        is_admin: user.is_admin,
    })
}
