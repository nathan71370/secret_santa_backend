use super::players::PlayerRef;
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum MailStatus {
    Pending,
    /// Réclamé par un envoi en cours.
    Sending,
    Sent,
    Failed,
    /// Édition importée : aucun mail n'est jamais parti.
    #[sqlx(rename = "none")]
    #[serde(rename = "none")]
    NotApplicable,
}

/// Ce que l'admin voit d'un envoi — jamais le receveur.
#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MailRow {
    pub player_id: i64,
    pub pseudo: String,
    pub mail_status: MailStatus,
    pub mail_error: Option<String>,
}

pub async fn insert(conn: &mut SqliteConnection, edition_id: i64, giver: i64, receiver: i64, status: MailStatus) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO assignments (edition_id, giver_id, receiver_id, mail_status) VALUES (?, ?, ?, ?)")
        .bind(edition_id)
        .bind(giver)
        .bind(receiver)
        .bind(status)
        .execute(conn)
        .await?;
    Ok(())
}

/// Paires de l'édition tirée la plus récente d'une année antérieure.
pub async fn previous_targets(conn: &mut SqliteConnection, year: i64) -> Result<HashMap<i64, i64>, sqlx::Error> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT giver_id, receiver_id FROM assignments
         WHERE edition_id = (SELECT id FROM editions WHERE status = 'drawn' AND year < ? ORDER BY year DESC LIMIT 1)",
    )
    .bind(year)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().collect())
}

pub async fn target_of(pool: &SqlitePool, edition_id: i64, giver: i64) -> Result<Option<PlayerRef>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id AS player_id, p.pseudo FROM assignments a JOIN players p ON p.id = a.receiver_id
         WHERE a.edition_id = ? AND a.giver_id = ?",
    )
    .bind(edition_id)
    .bind(giver)
    .fetch_optional(pool)
    .await
}

pub async fn mail_statuses(pool: &SqlitePool, edition_id: i64) -> Result<Vec<MailRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id AS player_id, p.pseudo, a.mail_status, a.mail_error FROM assignments a
         JOIN players p ON p.id = a.giver_id WHERE a.edition_id = ? ORDER BY lower(p.pseudo)",
    )
    .bind(edition_id)
    .fetch_all(pool)
    .await
}

pub async fn pending(pool: &SqlitePool, edition_id: i64) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT giver_id FROM assignments WHERE edition_id = ? AND mail_status = 'pending' ORDER BY giver_id")
        .bind(edition_id)
        .fetch_all(pool)
        .await
}

pub async fn set_mail_status(pool: &SqlitePool, edition_id: i64, giver: i64, status: MailStatus, error: Option<&str>) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("UPDATE assignments SET mail_status = ?, mail_error = ? WHERE edition_id = ? AND giver_id = ?")
        .bind(status)
        .bind(error)
        .bind(edition_id)
        .bind(giver)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}

/// Réclame l'envoi (`pending → sending`). `false` : quelqu'un d'autre l'a déjà
/// réclamé, ou il n'y a rien à envoyer.
pub async fn claim_for_sending(pool: &SqlitePool, edition_id: i64, giver: i64) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("UPDATE assignments SET mail_status = 'sending' WHERE edition_id = ? AND giver_id = ? AND mail_status = 'pending'")
        .bind(edition_id)
        .bind(giver)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}

/// Au démarrage : un envoi coupé par un arrêt redevient `pending`.
pub async fn reset_sending(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("UPDATE assignments SET mail_status = 'pending' WHERE mail_status = 'sending'").execute(pool).await?;
    Ok(done.rows_affected())
}

pub async fn editions_with_pending(pool: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT DISTINCT edition_id FROM assignments WHERE mail_status = 'pending'").fetch_all(pool).await
}
