use super::players::{Player, PlayerRef};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum EditionStatus {
    Open,
    Drawn,
    Failed,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Edition {
    pub id: i64,
    pub year: i64,
    pub budget_eur: i64,
    pub deadline: DateTime<Utc>,
    pub status: EditionStatus,
    pub failure_reason: Option<String>,
    pub drawn_at: Option<DateTime<Utc>>,
    pub reminder_sent_at: Option<DateTime<Utc>>,
    pub announced_at: Option<DateTime<Utc>>,
    pub imported: bool,
    pub created_at: DateTime<Utc>,
}

const COLS: &str = "id, year, budget_eur, deadline, status, failure_reason, drawn_at, reminder_sent_at, announced_at, imported, created_at";

pub async fn by_year(pool: &SqlitePool, year: i64) -> Result<Option<Edition>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM editions WHERE year = ?")).bind(year).fetch_optional(pool).await
}

pub async fn by_id(pool: &SqlitePool, id: i64) -> Result<Option<Edition>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM editions WHERE id = ?")).bind(id).fetch_optional(pool).await
}

pub async fn create_open(pool: &SqlitePool, year: i64, budget_eur: i64, deadline: DateTime<Utc>, now: DateTime<Utc>) -> Result<Edition, sqlx::Error> {
    sqlx::query_as(&format!(
        "INSERT INTO editions (year, budget_eur, deadline, status, created_at) VALUES (?, ?, ?, 'open', ?) RETURNING {COLS}"
    ))
    .bind(year)
    .bind(budget_eur)
    .bind(deadline)
    .bind(now)
    .fetch_one(pool)
    .await
}

pub async fn update_budget(pool: &SqlitePool, id: i64, budget_eur: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE editions SET budget_eur = ? WHERE id = ? AND status <> 'drawn'")
        .bind(budget_eur)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Une nouvelle deadline rouvre une édition dont le tirage a échoué.
pub async fn update_deadline(pool: &SqlitePool, id: i64, deadline: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE editions SET deadline = ?,
             status = CASE WHEN status = 'failed' THEN 'open' ELSE status END,
             failure_reason = CASE WHEN status = 'failed' THEN NULL ELSE failure_reason END
         WHERE id = ? AND status <> 'drawn'",
    )
    .bind(deadline)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_announced(pool: &SqlitePool, id: i64, now: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE editions SET announced_at = ? WHERE id = ?").bind(now).bind(id).execute(pool).await?;
    Ok(())
}

/// `true` si c'est cet appel qui a posé la marque (relance à envoyer).
pub async fn mark_reminder_sent(pool: &SqlitePool, id: i64, now: DateTime<Utc>) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("UPDATE editions SET reminder_sent_at = ? WHERE id = ? AND reminder_sent_at IS NULL")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}

pub async fn open_editions(pool: &SqlitePool) -> Result<Vec<Edition>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM editions WHERE status = 'open'")).fetch_all(pool).await
}

/// `None` efface la réponse (retour à « pas répondu »).
pub async fn set_registration(
    pool: &SqlitePool,
    edition_id: i64,
    player_id: i64,
    participating: Option<bool>,
    by_admin: bool,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    match participating {
        None => {
            sqlx::query("DELETE FROM registrations WHERE edition_id = ? AND player_id = ?")
                .bind(edition_id)
                .bind(player_id)
                .execute(pool)
                .await?;
        }
        Some(p) => {
            sqlx::query(
                "INSERT INTO registrations (edition_id, player_id, participating, answered_by_admin, updated_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT (edition_id, player_id) DO UPDATE SET
                     participating = excluded.participating,
                     answered_by_admin = excluded.answered_by_admin,
                     updated_at = excluded.updated_at",
            )
            .bind(edition_id)
            .bind(player_id)
            .bind(p)
            .bind(by_admin)
            .bind(now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Réponse d'un joueur, écrite seulement si l'édition est encore ouverte et
/// sa deadline pas passée — vérifié dans la même instruction que l'écriture,
/// pour qu'un tirage concurrent ne la laisse pas passer. `false` : close.
pub async fn register_while_open(pool: &SqlitePool, edition_id: i64, player_id: i64, participating: bool, now: DateTime<Utc>) -> Result<bool, sqlx::Error> {
    let done = sqlx::query(
        "INSERT INTO registrations (edition_id, player_id, participating, answered_by_admin, updated_at)
         SELECT ?, ?, ?, 0, ?
         WHERE EXISTS (SELECT 1 FROM editions WHERE id = ? AND status = 'open' AND julianday(deadline) > julianday(?))
         ON CONFLICT (edition_id, player_id) DO UPDATE SET
             participating = excluded.participating,
             answered_by_admin = 0,
             updated_at = excluded.updated_at",
    )
    .bind(edition_id)
    .bind(player_id)
    .bind(participating)
    .bind(now)
    .bind(edition_id)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

pub async fn registration_of(pool: &SqlitePool, edition_id: i64, player_id: i64) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar("SELECT participating FROM registrations WHERE edition_id = ? AND player_id = ?")
        .bind(edition_id)
        .bind(player_id)
        .fetch_optional(pool)
        .await
}

pub async fn participants(pool: &SqlitePool, edition_id: i64) -> Result<Vec<PlayerRef>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id AS player_id, p.pseudo FROM registrations r JOIN players p ON p.id = r.player_id
         WHERE r.edition_id = ? AND r.participating = 1 ORDER BY lower(p.pseudo)",
    )
    .bind(edition_id)
    .fetch_all(pool)
    .await
}

/// joueur → (participe, répondu par l'admin)
pub async fn registrations(pool: &SqlitePool, edition_id: i64) -> Result<HashMap<i64, (bool, bool)>, sqlx::Error> {
    let rows: Vec<(i64, bool, bool)> =
        sqlx::query_as("SELECT player_id, participating, answered_by_admin FROM registrations WHERE edition_id = ?")
            .bind(edition_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(id, p, a)| (id, (p, a))).collect())
}

pub async fn non_respondents_with_access(pool: &SqlitePool, edition_id: i64) -> Result<Vec<Player>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, email, sso_user_id, pseudo, has_access FROM players p
         WHERE has_access = 1
           AND NOT EXISTS (SELECT 1 FROM registrations r WHERE r.edition_id = ? AND r.player_id = p.id)
         ORDER BY lower(pseudo)",
    )
    .bind(edition_id)
    .fetch_all(pool)
    .await
}
