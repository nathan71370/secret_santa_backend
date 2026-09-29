mod common;

use axum::{body::Body, http::{Method, Request, StatusCode}};
use common::TestApp;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn sans_cookie_401_avec_url_de_connexion() {
    let app = TestApp::new().await;
    let (status, body) = app.call(Method::GET, "/api/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "NOT_AUTHENTICATED");
    assert_eq!(body["loginUrl"], "https://auth.test/login");
}

#[tokio::test]
async fn jeton_inconnu_401() {
    let app = TestApp::new().await;
    let (status, _) = app.get("/api/me", "inconnu").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sso_en_panne_echec_ferme() {
    let app = TestApp::new().await;
    let token = app.login(1, "Alice", false);
    app.auth.down.store(true, Ordering::SeqCst);
    let (status, _) = app.get("/api/me", &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sans_acces_a_l_app_403() {
    let app = TestApp::new().await;
    let token = app.login_without_app(1, "Alice");
    let (status, body) = app.get("/api/me", &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "FORBIDDEN");
}

#[tokio::test]
async fn me_cree_le_joueur_et_renvoie_son_profil() {
    let app = TestApp::new().await;
    let token = app.login(1, "Alice", true);
    let (status, body) = app.get("/api/me", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["pseudo"], "Alice");
    assert_eq!(body["email"], "alice@x.fr");
    assert_eq!(body["isAdmin"], true);
    assert_eq!(body["playerId"], app.player_id("Alice").await);
}

#[tokio::test]
async fn le_pseudo_suit_le_sso() {
    let app = TestApp::new().await;
    let token = app.login(1, "Alice", false);
    app.get("/api/me", &token).await;
    let id = app.player_id("Alice").await;
    app.auth.sessions.lock().unwrap().get_mut(&token).unwrap().pseudo = "Alicia".into();
    let (_, body) = app.get("/api/me", &token).await;
    assert_eq!(body["pseudo"], "Alicia");
    assert_eq!(body["playerId"], id);
}

#[tokio::test]
async fn health_sans_authentification() {
    let app = TestApp::new().await;
    let res = app.response(Request::get("/api/health").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
}
