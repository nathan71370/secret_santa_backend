use chrono::{TimeZone, Utc};
use secret_santa::clock::{Clock, ManualClock, paris_year};
use secret_santa::db;

#[tokio::test]
async fn la_migration_cree_toutes_les_tables() {
    let pool = db::connect_in_memory().await.unwrap();
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE '\\_%' ESCAPE '\\' AND name NOT LIKE 'sqlite%' ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        tables,
        [
            "assignments",
            "editions",
            "gifts",
            "players",
            "registrations"
        ]
    );
}

#[tokio::test]
async fn les_cles_etrangeres_sont_actives() {
    let pool = db::connect_in_memory().await.unwrap();
    let res = sqlx::query("INSERT INTO registrations (edition_id, player_id, participating, updated_at) VALUES (99, 99, 1, '2026-01-01T00:00:00Z')")
        .execute(&pool)
        .await;
    assert!(
        res.is_err(),
        "une inscription vers une édition inexistante doit être refusée"
    );
}

#[tokio::test]
async fn connect_cree_le_fichier_et_son_dossier() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sous/dossier/santa.db");
    let pool = db::connect(path.to_str().unwrap()).await.unwrap();
    pool.close().await;
    assert!(path.exists());
}

#[test]
fn horloge_manuelle_et_annee_de_paris() {
    let clock = ManualClock::new(Utc.with_ymd_and_hms(2026, 12, 31, 22, 30, 0).unwrap());
    // 23h30 à Paris : encore 2026 ; une heure plus tard, 2027.
    assert_eq!(paris_year(clock.now()), 2026);
    clock.advance(chrono::Duration::hours(1));
    assert_eq!(paris_year(clock.now()), 2027);
    clock.set(Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap());
    assert_eq!(paris_year(clock.now()), 2025);
}

#[tokio::test]
async fn les_migrations_gardent_l_index_des_envois_et_acceptent_sending() {
    let pool = db::connect_in_memory().await.unwrap();
    let index: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_assignments_pending'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(index.is_some());
    let sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'assignments'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(sql.contains("'sending'"), "{sql}");
}
