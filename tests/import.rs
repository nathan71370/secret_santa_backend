mod common;

use axum::http::{Method, StatusCode};
use chrono::Duration;
use common::{Part, TestApp, text};
use secret_santa::clock::Clock;
use serde_json::json;

fn file(content: &str) -> Part {
    Part::File { filename: "players.txt".into(), content_type: "text/plain".into(), bytes: content.as_bytes().to_vec() }
}

const LAST_YEAR: &str = "Alice;alice@x.fr;Bob\nBob;bob@x.fr;Chloe\nChloe;chloe@x.fr;Alice\n";

#[tokio::test]
async fn import_puis_contrainte_appliquee_au_tirage_suivant() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let (status, report) = app
        .multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2025")), ("budgetEur", text("25")), ("file", file(LAST_YEAR))])
        .await;
    assert_eq!(status, StatusCode::CREATED, "{report}");
    assert_eq!(report, json!({ "year": 2025, "imported": 3, "skipped": [] }));

    // Les joueurs importés se connectent : ils sont reliés par leur adresse.
    let deadline = (app.clock.now() + Duration::days(10)).to_rfc3339();
    app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": deadline })).await;
    let tokens: Vec<_> = ["Alice", "Bob", "Chloe"].iter().enumerate().map(|(i, p)| (p.to_string(), app.login(10 + i as i64, p, false))).collect();
    for (_, t) in &tokens {
        app.put("/api/editions/current/registration", t, json!({ "participating": true })).await;
    }
    let (status, _) = app.post("/api/admin/editions/current/draw", &admin, json!({})).await;
    assert_eq!(status, StatusCode::OK);

    // 2025 : A→B, B→C, C→A. Le seul cycle permis en 2026 est A→C→B→A.
    let expected = [("Alice", "Chloe"), ("Bob", "Alice"), ("Chloe", "Bob")];
    for (giver, receiver) in expected {
        let token = &tokens.iter().find(|(p, _)| p == giver).unwrap().1;
        let (_, target) = app.get("/api/editions/current/my-target", token).await;
        assert_eq!(target["pseudo"], receiver, "cible de {giver}");
    }
}

#[tokio::test]
async fn lignes_ignorees_rapportees() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let (status, report) = app
        .multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2024")), ("file", file("Alice;alice@x.fr;Bob\nBob;bob@x.fr\n"))])
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(report["imported"], 1);
    assert_eq!(report["skipped"], json!([{ "line": 2, "reason": "pas de cible" }]));
}

#[tokio::test]
async fn rien_d_importable_422_sans_rien_creer() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let (status, body) = app.multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2024")), ("file", file("n'importe quoi\n"))]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "NOTHING_TO_IMPORT");
    assert_eq!(body["skipped"][0]["line"], 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM editions").fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn annee_deja_existante_409_et_annee_future_400() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2025")), ("file", file(LAST_YEAR))]).await;
    let (status, body) = app.multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2025")), ("file", file(LAST_YEAR))]).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("EDITION_EXISTS")));
    let (status, body) = app.multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2027")), ("file", file(LAST_YEAR))]).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("INVALID_YEAR")));
}

#[tokio::test]
async fn reserve_a_l_admin() {
    let app = TestApp::new().await;
    let pote = app.login(2, "Bob", false);
    let (status, _) = app.multipart(Method::POST, "/api/admin/import", &pote, vec![("year", text("2025")), ("file", file(LAST_YEAR))]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn l_annee_en_cours_ne_s_importe_pas() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let (status, body) = app.multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2026")), ("file", file(LAST_YEAR))]).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("INVALID_YEAR")));
    assert_eq!(body["message"], "L'année doit être comprise entre 2000 et 2025.");
}
