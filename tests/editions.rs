mod common;

use axum::http::{Method, Request, StatusCode, header};
use axum::body::Body;
use chrono::Duration;
use common::TestApp;
use serde_json::json;
use std::sync::atomic::Ordering;

fn in_days(app: &TestApp, days: i64) -> String {
    use secret_santa::clock::Clock;
    (app.clock.now() + Duration::days(days)).to_rfc3339()
}

async fn open_edition(app: &TestApp, admin: &str) -> serde_json::Value {
    let (status, body) = app.post("/api/admin/editions", admin, json!({ "budgetEur": 30, "deadline": in_days(app, 20) })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

#[tokio::test]
async fn aucune_edition_renvoie_null() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let (status, body) = app.get("/api/editions/current", &t).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_null());
}

#[tokio::test]
async fn l_admin_cree_l_edition_et_l_annonce_part_a_tous() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.add_sso_user(2, "Bob");
    app.add_sso_user(3, "Chloe");

    let body = open_edition(&app, &admin).await;
    assert_eq!(body["year"], 2026);
    assert_eq!(body["status"], "open");
    assert_eq!(body["budgetEur"], 30);
    assert_eq!(body["participants"], json!([]));

    app.settle().await;
    for email in ["nathan@x.fr", "bob@x.fr", "chloe@x.fr"] {
        let mails = app.mails_to(email);
        assert_eq!(mails.len(), 1, "annonce pour {email}");
        assert!(mails[0].subject.contains("inscriptions sont ouvertes"));
    }
}

#[tokio::test]
async fn creation_refusee_aux_non_admins_et_validee() {
    let app = TestApp::new().await;
    let pote = app.login(2, "Bob", false);
    let (status, _) = app.post("/api/admin/editions", &pote, json!({ "budgetEur": 30, "deadline": in_days(&app, 5) })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let admin = app.login(1, "Nathan", true);
    let (status, body) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": in_days(&app, -1) })).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("INVALID_DEADLINE")));
    let (status, body) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 0, "deadline": in_days(&app, 5) })).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("INVALID_BUDGET")));

    open_edition(&app, &admin).await;
    let (status, body) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": in_days(&app, 5) })).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("EDITION_EXISTS")));
}

#[tokio::test]
async fn creation_impossible_si_le_sso_est_injoignable() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.get("/api/me", &admin).await; // session mise en place avant la panne
    app.auth.down.store(true, Ordering::SeqCst);
    let (status, _) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": in_days(&app, 5) })).await;
    // La session elle-même n'est plus vérifiable : échec fermé.
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn un_joueur_s_inscrit_puis_change_d_avis() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let bob = app.login(2, "Bob", false);
    open_edition(&app, &admin).await;

    let (status, body) = app.put("/api/editions/current/registration", &bob, json!({ "participating": true })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["myRegistration"], true);
    assert_eq!(body["participants"], json!([{ "playerId": app.player_id("Bob").await, "pseudo": "Bob" }]));

    let (_, body) = app.put("/api/editions/current/registration", &bob, json!({ "participating": false })).await;
    assert_eq!(body["myRegistration"], false);
    assert_eq!(body["participants"], json!([]));

    let (_, body) = app.get("/api/editions/current", &admin).await;
    assert_eq!(body["myRegistration"], serde_json::Value::Null);
}

#[tokio::test]
async fn inscriptions_closes_apres_la_deadline() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let bob = app.login(2, "Bob", false);
    open_edition(&app, &admin).await;
    app.clock.advance(Duration::days(21));
    let (status, body) = app.put("/api/editions/current/registration", &bob, json!({ "participating": true })).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("REGISTRATION_CLOSED")));
}

#[tokio::test]
async fn inscription_sans_edition_404() {
    let app = TestApp::new().await;
    let bob = app.login(2, "Bob", false);
    let (status, _) = app.put("/api/editions/current/registration", &bob, json!({ "participating": true })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ecriture_sans_origin_refusee() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    open_edition(&app, &admin).await;
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/api/editions/current/registration")
        .header(header::COOKIE, format!("lim_session={admin}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://tierlistrr.limperiam.com")
        .body(Body::from(json!({ "participating": true }).to_string()))
        .unwrap();
    let (status, _) = app.send(req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn l_admin_voit_l_annuaire_et_repond_a_la_place_d_un_joueur() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.add_sso_user(2, "Bob");
    app.add_sso_user(3, "Chloe");
    open_edition(&app, &admin).await;

    let (status, body) = app.get("/api/admin/sso-users", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let pseudos: Vec<_> = body.as_array().unwrap().iter().map(|u| u["pseudo"].as_str().unwrap().to_string()).collect();
    assert_eq!(pseudos, ["Bob", "Chloe", "Nathan"]);
    assert!(body.as_array().unwrap().iter().all(|u| u["registration"].is_null()));

    let chloe = app.player_id("Chloe").await;
    let (status, _) = app.put(&format!("/api/admin/editions/current/registrations/{chloe}"), &admin, json!({ "participating": true })).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get("/api/admin/sso-users", &admin).await;
    let row = body.as_array().unwrap().iter().find(|u| u["pseudo"] == "Chloe").unwrap();
    assert_eq!((row["registration"].as_bool(), row["answeredByAdmin"].as_bool()), (Some(true), Some(true)));
    assert_eq!(row["email"], "chloe@x.fr");

    let (status, _) = app.put(&format!("/api/admin/editions/current/registrations/{chloe}"), &admin, json!({ "participating": null })).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get("/api/admin/sso-users", &admin).await;
    let row = body.as_array().unwrap().iter().find(|u| u["pseudo"] == "Chloe").unwrap();
    assert!(row["registration"].is_null());
}

#[tokio::test]
async fn un_compte_retire_du_sso_disparait_de_l_annuaire() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.add_sso_user(2, "Bob");
    app.get("/api/admin/sso-users", &admin).await;
    app.auth.users.lock().unwrap().retain(|u| u.user_id != 2);
    let (_, body) = app.get("/api/admin/sso-users", &admin).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn l_admin_modifie_budget_et_deadline() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    open_edition(&app, &admin).await;
    let new_deadline = in_days(&app, 30);
    let (status, body) = app.patch("/api/admin/editions/current", &admin, json!({ "budgetEur": 40, "deadline": new_deadline })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["budgetEur"], 40);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(body["deadline"].as_str().unwrap()).unwrap(),
        chrono::DateTime::parse_from_rfc3339(&new_deadline).unwrap()
    );
}
