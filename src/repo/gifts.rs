use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Gift {
    pub id: i64,
    pub player_id: i64,
    pub year: i64,
    pub description: String,
    pub photo_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

const COLS: &str = "id, player_id, year, description, photo_path, created_at";

pub async fn list_for(pool: &SqlitePool, player_id: i64) -> Result<Vec<Gift>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLS} FROM gifts WHERE player_id = ? ORDER BY year DESC, id DESC"
    ))
    .bind(player_id)
    .fetch_all(pool)
    .await
}

pub async fn find_owned(
    pool: &SqlitePool,
    id: i64,
    player_id: i64,
) -> Result<Option<Gift>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLS} FROM gifts WHERE id = ? AND player_id = ?"
    ))
    .bind(id)
    .bind(player_id)
    .fetch_optional(pool)
    .await
}

pub async fn insert(
    pool: &SqlitePool,
    player_id: i64,
    year: i64,
    description: &str,
    photo: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Gift, sqlx::Error> {
    sqlx::query_as(&format!(
        "INSERT INTO gifts (player_id, year, description, photo_path, created_at) VALUES (?, ?, ?, ?, ?) RETURNING {COLS}"
    ))
    .bind(player_id)
    .bind(year)
    .bind(description)
    .bind(photo)
    .bind(now)
    .fetch_one(pool)
    .await
}

pub async fn update(
    pool: &SqlitePool,
    id: i64,
    year: i64,
    description: &str,
    photo: Option<&str>,
) -> Result<Gift, sqlx::Error> {
    sqlx::query_as(&format!(
        "UPDATE gifts SET year = ?, description = ?, photo_path = ? WHERE id = ? RETURNING {COLS}"
    ))
    .bind(year)
    .bind(description)
    .bind(photo)
    .bind(id)
    .fetch_one(pool)
    .await
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM gifts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
