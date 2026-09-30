use super::{
    extract::AdminUser,
    json::{ApiMultipart, multipart_error},
};
use crate::{
    error::{ApiError, ApiResult},
    legacy_import,
    repo::{
        assignments::{self, MailStatus},
        editions, players,
    },
    state::AppState,
};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub async fn import(
    State(state): State<AppState>,
    _admin: AdminUser,
    ApiMultipart(mut mp): ApiMultipart,
) -> ApiResult<Response> {
    let bad = multipart_error("Fichier trop lourd.");
    let (mut content, mut year, mut budget) = (None, None, None);
    while let Some(field) = mp.next_field().await.map_err(bad)? {
        match field.name().unwrap_or_default() {
            "file" => {
                let bytes = field.bytes().await.map_err(bad)?;
                content = Some(String::from_utf8(bytes.to_vec()).map_err(|_| {
                    ApiError::bad_request("INVALID_FILE", "Le fichier doit être encodé en UTF-8.")
                })?);
            }
            "year" => year = Some(field.text().await.map_err(bad)?),
            "budgetEur" => budget = Some(field.text().await.map_err(bad)?),
            _ => {}
        }
    }

    let content =
        content.ok_or_else(|| ApiError::bad_request("INVALID_FILE", "Fichier manquant."))?;
    let current = state.current_year();
    let year: i64 = year
        .and_then(|y| y.trim().parse().ok())
        // L'année en cours se crée par le formulaire d'édition, pas par import.
        .filter(|y| (2000..current).contains(y))
        .ok_or_else(|| {
            ApiError::bad_request(
                "INVALID_YEAR",
                format!("L'année doit être comprise entre 2000 et {}.", current - 1),
            )
        })?;
    let budget: i64 = match budget.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        None => 30,
        Some(b) => b
            .parse()
            .ok()
            .filter(|b| (1..=1000).contains(b))
            .ok_or_else(|| {
                ApiError::bad_request(
                    "INVALID_BUDGET",
                    "Le budget doit être compris entre 1 et 1000 €.",
                )
            })?,
    };

    if editions::by_year(&state.pool, year).await?.is_some() {
        return Err(ApiError::conflict(
            "EDITION_EXISTS",
            format!("L'édition {year} existe déjà."),
        ));
    }

    let parsed = legacy_import::parse(&content);
    if parsed.pairs.is_empty() {
        let body = json!({ "error": "NOTHING_TO_IMPORT", "message": "Aucune paire valide dans ce fichier.", "skipped": parsed.skipped });
        return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response());
    }

    let now = state.now();
    let mut tx = state.pool.begin().await?;
    let edition_id: i64 = sqlx::query_scalar(
        "INSERT INTO editions (year, budget_eur, deadline, status, drawn_at, imported, created_at) VALUES (?, ?, ?, 'drawn', ?, 1, ?) RETURNING id",
    )
    .bind(year)
    .bind(budget)
    .bind(now)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    for pair in &parsed.pairs {
        let giver =
            players::upsert_imported(&mut tx, &pair.giver_email, &pair.giver_name, now).await?;
        let receiver =
            players::upsert_imported(&mut tx, &pair.receiver_email, &pair.receiver_name, now)
                .await?;
        assignments::insert(
            &mut tx,
            edition_id,
            giver,
            receiver,
            MailStatus::NotApplicable,
        )
        .await?;
    }
    tx.commit().await?;

    tracing::info!(
        year,
        imported = parsed.pairs.len(),
        skipped = parsed.skipped.len(),
        "historique importé"
    );
    let body = json!({ "year": year, "imported": parsed.pairs.len(), "skipped": parsed.skipped });
    Ok((StatusCode::CREATED, Json(body)).into_response())
}
