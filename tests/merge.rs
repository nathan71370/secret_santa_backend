mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::json;

/// Tirage 2024 importé avec l'ancienne adresse de Bob, plus un cadeau à son
/// nom. Bob s'est depuis connecté avec sa nouvelle adresse. Renvoie
/// (ancienne fiche, nouvelle fiche, jeton admin).
async fn doublon(app: &TestApp) -> (i64, i64, String) {
    let admin = app.login(1, "Nathan", true);
    let bob_token = app.login(2, "Bob", false);
    app.get("/api/me", &admin).await;
    app.get("/api/me", &bob_token).await;
    let pool = &app.state.pool;
    let id = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(pool)
            .await
            .unwrap()
    };
    let old = id("INSERT INTO players (email, pseudo, has_access, updated_at) VALUES ('bob.ancien@x.fr', 'Bob', 0, '2024-12-01T00:00:00Z') RETURNING id").await;
    let chloe = id("INSERT INTO players (email, pseudo, has_access, updated_at) VALUES ('chloe@x.fr', 'Chloe', 0, '2024-12-01T00:00:00Z') RETURNING id").await;
    let edition = id("INSERT INTO editions (year, budget_eur, deadline, status, drawn_at, imported, created_at) VALUES (2024, 30, '2024-12-01T00:00:00Z', 'drawn', '2024-12-01T00:00:00Z', 1, '2024-12-01T00:00:00Z') RETURNING id").await;
    let nathan = app.player_id("Nathan").await;
    for (g, r) in [(nathan, old), (old, chloe), (chloe, nathan)] {
        sqlx::query("INSERT INTO assignments (edition_id, giver_id, receiver_id, mail_status) VALUES (?, ?, ?, 'none')")
            .bind(edition)
            .bind(g)
            .bind(r)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO gifts (player_id, year, description, created_at) VALUES (?, 2024, 'Écharpe', '2024-12-25T00:00:00Z')")
        .bind(old)
        .execute(pool)
        .await
        .unwrap();
    let new: i64 = sqlx::query_scalar("SELECT id FROM players WHERE sso_user_id = 2")
        .fetch_one(pool)
        .await
        .unwrap();
    (old, new, admin)
}

async fn count(app: &TestApp, sql: &str, id: i64) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn la_liste_admin_distingue_l_ancienne_adresse() {
    let app = TestApp::new().await;
    let (old, new, admin) = doublon(&app).await;
    let (status, body) = app.get("/api/admin/players", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let row = |id: i64| {
        body.as_array()
            .unwrap()
            .iter()
            .find(|p| p["playerId"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(row(old)["email"], "bob.ancien@x.fr");
    assert_eq!(row(old)["linked"], false);
    assert_eq!(row(old)["years"], json!([2024]));
    assert_eq!(row(new)["linked"], true);
    assert_eq!(row(new)["years"], json!([]));
}

#[tokio::test]
async fn fusion_deplace_tirages_et_cadeaux_puis_supprime_l_ancienne_fiche() {
    let app = TestApp::new().await;
    let (old, new, admin) = doublon(&app).await;

    let (status, body) = app
        .post(
            &format!("/api/admin/players/{old}/merge"),
            &admin,
            json!({ "into": new }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "registrations": 0, "assignments": 2, "gifts": 1 })
    );

    assert_eq!(
        count(&app, "SELECT count(*) FROM players WHERE id = ?", old).await,
        0
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM assignments WHERE giver_id = ?1 OR receiver_id = ?1",
            new
        )
        .await,
        2
    );
    let (_, profile) = app.get(&format!("/api/players/{new}"), &admin).await;
    assert_eq!(profile["gifts"][0]["description"], "Écharpe");

    // Une seule fiche « Bob » chez les participants.
    let (_, players) = app.get("/api/players", &admin).await;
    let bobs = players
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["pseudo"] == "Bob")
        .count();
    assert_eq!(bobs, 1);
}

#[tokio::test]
async fn fusion_refusee_si_les_deux_fiches_ont_joue_la_meme_annee() {
    let app = TestApp::new().await;
    let (old, new, admin) = doublon(&app).await;
    sqlx::query("INSERT INTO registrations (edition_id, player_id, participating, updated_at) SELECT id, ?, 1, '2024-11-01T00:00:00Z' FROM editions WHERE year = 2024")
        .bind(new)
        .execute(&app.state.pool)
        .await
        .unwrap();

    let (status, body) = app
        .post(
            &format!("/api/admin/players/{old}/merge"),
            &admin,
            json!({ "into": new }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "MERGE_OVERLAP");
    assert!(body["message"].as_str().unwrap().contains("2024"));
    // Rien n'a bougé.
    assert_eq!(
        count(&app, "SELECT count(*) FROM gifts WHERE player_id = ?", old).await,
        1
    );
}

#[tokio::test]
async fn fusion_refusee_depuis_une_fiche_reliee_au_sso() {
    let app = TestApp::new().await;
    let (old, new, admin) = doublon(&app).await;
    let (status, body) = app
        .post(
            &format!("/api/admin/players/{new}/merge"),
            &admin,
            json!({ "into": old }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "MERGE_SOURCE_LINKED");
    assert_eq!(
        count(&app, "SELECT count(*) FROM players WHERE id = ?", new).await,
        1
    );
}

#[tokio::test]
async fn fusion_controles_d_acces_et_de_saisie() {
    let app = TestApp::new().await;
    let (old, new, admin) = doublon(&app).await;
    let pote = app.login(3, "Chloé", false);

    let url = format!("/api/admin/players/{old}/merge");
    let (status, _) = app.post(&url, &pote, json!({ "into": new })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = app.post(&url, &admin, json!({ "into": old })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "MERGE_SAME_PLAYER");

    let (status, _) = app.post(&url, &admin, json!({ "into": 9999 })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = app.get("/api/admin/players", &pote).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
