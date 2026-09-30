use super::extract::CurrentUser;
use crate::{
    error::{ApiError, ApiResult},
    photos,
    repo::gifts::{self, Gift},
    state::AppState,
};
use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{StatusCode, header},
    response::Response,
};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiftView {
    pub id: i64,
    pub year: i64,
    pub description: String,
    pub photo_url: Option<String>,
}

impl From<Gift> for GiftView {
    fn from(g: Gift) -> Self {
        GiftView { id: g.id, year: g.year, description: g.description, photo_url: g.photo_path.map(|p| format!("/api/uploads/{p}")) }
    }
}

#[derive(Default)]
struct GiftForm {
    year: Option<String>,
    description: Option<String>,
    photo: Option<Vec<u8>>,
    remove_photo: bool,
}

async fn read_form(mut mp: Multipart) -> ApiResult<GiftForm> {
    let bad = |_: axum::extract::multipart::MultipartError| ApiError::bad_request("INVALID_FORM", "Formulaire invalide.");
    let mut form = GiftForm::default();
    while let Some(field) = mp.next_field().await.map_err(bad)? {
        match field.name().unwrap_or_default() {
            "year" => form.year = Some(field.text().await.map_err(bad)?),
            "description" => form.description = Some(field.text().await.map_err(bad)?),
            "photo" => {
                let bytes = field.bytes().await.map_err(bad)?;
                if !bytes.is_empty() {
                    form.photo = Some(bytes.to_vec());
                }
            }
            "removePhoto" => form.remove_photo = field.text().await.map_err(bad)? == "true",
            _ => {}
        }
    }
    Ok(form)
}

fn validate(form: &GiftForm, current_year: i64) -> ApiResult<(i64, String)> {
    let year = form
        .year
        .as_deref()
        .and_then(|y| y.trim().parse::<i64>().ok())
        .filter(|y| (2000..=current_year + 1).contains(y))
        .ok_or_else(|| ApiError::bad_request("INVALID_YEAR", format!("L'année doit être comprise entre 2000 et {}.", current_year + 1)))?;
    let description = form.description.as_deref().unwrap_or_default().trim().to_string();
    if description.is_empty() || description.chars().count() > 500 {
        return Err(ApiError::bad_request("INVALID_DESCRIPTION", "La description doit faire entre 1 et 500 caractères."));
    }
    Ok((year, description))
}

/// Normalise (hors du runtime async : c'est du calcul) puis enregistre.
async fn save_photo(state: &AppState, bytes: Vec<u8>) -> ApiResult<String> {
    let jpeg = tokio::task::spawn_blocking(move || photos::normalize(&bytes))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::bad_request("INVALID_PHOTO", e.to_string()))?;
    photos::store(&state.config.uploads_dir, &jpeg).await.map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn create(State(state): State<AppState>, user: CurrentUser, mp: Multipart) -> ApiResult<(StatusCode, Json<GiftView>)> {
    let form = read_form(mp).await?;
    let (year, description) = validate(&form, state.current_year())?;
    let photo = match form.photo {
        Some(bytes) => Some(save_photo(&state, bytes).await?),
        None => None,
    };
    let gift = match gifts::insert(&state.pool, user.player_id, year, &description, photo.as_deref(), state.now()).await {
        Ok(gift) => gift,
        Err(e) => {
            if let Some(name) = &photo {
                photos::remove(&state.config.uploads_dir, name).await;
            }
            return Err(e.into());
        }
    };
    Ok((StatusCode::CREATED, Json(gift.into())))
}

pub async fn update(State(state): State<AppState>, user: CurrentUser, Path(id): Path<i64>, mp: Multipart) -> ApiResult<Json<GiftView>> {
    let existing = gifts::find_owned(&state.pool, id, user.player_id).await?.ok_or_else(|| ApiError::not_found("Cadeau introuvable."))?;
    let form = read_form(mp).await?;
    let (year, description) = validate(&form, state.current_year())?;
    let mut photo = existing.photo_path.clone();
    if let Some(bytes) = form.photo {
        photo = Some(save_photo(&state, bytes).await?);
    } else if form.remove_photo {
        photo = None;
    }
    let replaced = photo != existing.photo_path;
    let gift = match gifts::update(&state.pool, id, year, &description, photo.as_deref()).await {
        Ok(gift) => gift,
        Err(e) => {
            if let (true, Some(new)) = (replaced, &photo)
                && existing.photo_path.as_ref() != Some(new) {
                    photos::remove(&state.config.uploads_dir, new).await;
                }
            return Err(e.into());
        }
    };
    if let (true, Some(old)) = (replaced, &existing.photo_path) {
        photos::remove(&state.config.uploads_dir, old).await;
    }
    Ok(Json(gift.into()))
}

pub async fn delete(State(state): State<AppState>, user: CurrentUser, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let existing = gifts::find_owned(&state.pool, id, user.player_id).await?.ok_or_else(|| ApiError::not_found("Cadeau introuvable."))?;
    gifts::delete(&state.pool, id).await?;
    if let Some(p) = &existing.photo_path {
        photos::remove(&state.config.uploads_dir, p).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn upload(State(state): State<AppState>, _user: CurrentUser, Path(file): Path<String>) -> ApiResult<Response> {
    let not_found = || ApiError::not_found("Photo introuvable.");
    if !photos::is_valid_name(&file) {
        return Err(not_found());
    }
    let bytes = tokio::fs::read(state.config.uploads_dir.join(&file)).await.map_err(|_| not_found())?;
    Response::builder()
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal(e.to_string()))
}
