mod common;

use axum::http::{Method, StatusCode};
use common::TestApp;
use serde_json::json;

#[tokio::test]
async fn identifiant_non_numerique_404_json() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let (status, body) = app.get("/api/players/abc", &t).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        json!({ "error": "NOT_FOUND", "message": "Ressource introuvable." })
    );
}

#[tokio::test]
async fn route_inconnue_404_json() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let (status, body) = app.get("/api/nope", &t).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        json!({ "error": "NOT_FOUND", "message": "Ressource introuvable." })
    );
}

#[tokio::test]
async fn methode_non_autorisee_405_json() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let (status, body) = app.delete("/api/me", &t, None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        body,
        json!({ "error": "METHOD_NOT_ALLOWED", "message": "Méthode non autorisée." })
    );
}

#[tokio::test]
async fn formulaire_attendu_mais_json_recu_400_json() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let (status, body) = app
        .call(
            Method::POST,
            "/api/me/gifts",
            Some(&t),
            Some(json!({ "year": 2025 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({ "error": "INVALID_FORM", "message": "Formulaire invalide." })
    );
}
