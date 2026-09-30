use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::{path::Path, str::FromStr, time::Duration};

pub async fn connect(path: &str) -> Result<SqlitePool, sqlx::Error> {
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(sqlx::Error::Io)?;
        }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new().max_connections(5).connect_with(options).await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// Base de test. Une seule connexion, gardée ouverte : chaque connexion à
/// `:memory:` aurait sinon sa propre base, vide.
pub async fn connect_in_memory() -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| sqlx::Error::Migrate(Box::new(e)))
}
