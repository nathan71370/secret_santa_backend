mod common;

use axum::http::StatusCode;
use chrono::Duration;
use common::TestApp;
use secret_santa::{
    clock::Clock,
    services::{
        draw_service::{self, DrawOutcome},
        mailing,
    },
};
use serde_json::json;
use std::collections::{HashMap, HashSet};

struct Game {
    app: TestApp,
    admin: String,
    tokens: Vec<(String, String)>, // (pseudo, jeton)
}

/// Édition ouverte (deadline J+20) où `pseudos` se sont inscrits.
async fn game(pseudos: &[&str]) -> Game {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let deadline = (app.clock.now() + Duration::days(20)).to_rfc3339();
    let (status, _) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": deadline })).await;
    assert_eq!(status, StatusCode::CREATED);
    let mut tokens = Vec::new();
    for (i, p) in pseudos.iter().enumerate() {
        let t = app.login(10 + i as i64, p, false);
        let (status, _) = app.put("/api/editions/current/registration", &t, json!({ "participating": true })).await;
        assert_eq!(status, StatusCode::OK);
        tokens.push((p.to_string(), t));
    }
    app.settle().await;
    app.mailer.sent.lock().unwrap().clear(); // on oublie les annonces
    Game { app, admin, tokens }
}

async fn edition_id(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT id FROM editions WHERE year = 2026").fetch_one(&app.state.pool).await.unwrap()
}

#[tokio::test]
async fn tirage_reussi_chacun_recoit_sa_cible_et_personne_d_autre_ne_la_voit() {
    let g = game(&["Alice", "Bob", "Chloe", "Dan"]).await;
    let (status, body) = g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "drawn");
    g.app.settle().await;

    let mut targets = HashMap::new();
    for (pseudo, token) in &g.tokens {
        let (status, target) = g.app.get("/api/editions/current/my-target", token).await;
        assert_eq!(status, StatusCode::OK);
        let target = target["pseudo"].as_str().unwrap().to_string();
        assert_ne!(&target, pseudo);
        let mails = g.app.mails_to(&format!("{}@x.fr", pseudo.to_lowercase()));
        assert_eq!(mails.len(), 1);
        assert!(mails[0].text.contains(&format!("tu offres un cadeau à : {target}")));
        targets.insert(pseudo.clone(), target);
    }
    let receivers: HashSet<_> = targets.values().cloned().collect();
    assert_eq!(receivers.len(), 4);

    // L'admin ne participe pas : pas de cible, et la route des mails ne dit rien des receveurs.
    let (status, _) = g.app.get("/api/editions/current/my-target", &g.admin).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, mails) = g.app.get("/api/admin/editions/current/mails", &g.admin).await;
    assert_eq!(status, StatusCode::OK);
    for row in mails.as_array().unwrap() {
        assert_eq!(row["mailStatus"], "sent");
        let keys: HashSet<_> = row.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, HashSet::from(["playerId".into(), "pseudo".into(), "mailStatus".into(), "mailError".into()]));
    }
}

#[tokio::test]
async fn ma_cible_avant_le_tirage_404() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    let (status, _) = g.app.get("/api/editions/current/my-target", &g.tokens[0].1).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn un_second_tirage_est_refuse() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    let (status, body) = g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("ALREADY_DRAWN")));
}

#[tokio::test]
async fn deux_declenchements_simultanes_ne_tirent_qu_une_fois() {
    let g = game(&["Alice", "Bob", "Chloe", "Dan"]).await;
    let id = edition_id(&g.app).await;
    let (a, b) = tokio::join!(draw_service::run_draw(&g.app.state, id), draw_service::run_draw(&g.app.state, id));
    let outcomes = [a.unwrap(), b.unwrap()];
    assert_eq!(outcomes.iter().filter(|o| **o == DrawOutcome::Drawn).count(), 1);
    assert_eq!(outcomes.iter().filter(|o| **o == DrawOutcome::AlreadyDone).count(), 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assignments WHERE edition_id = ?").bind(id).fetch_one(&g.app.state.pool).await.unwrap();
    assert_eq!(count, 4);
}

#[tokio::test]
async fn moins_de_trois_participants_echec_signale_a_l_admin() {
    let g = game(&["Alice", "Bob"]).await;
    let (status, body) = g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("DRAW_FAILED")));
    assert!(body["message"].as_str().unwrap().contains("au moins 3"));
    g.app.settle().await;

    let admin_mails = g.app.mails_to("admin@x.fr");
    assert_eq!(admin_mails.len(), 1);
    assert!(admin_mails[0].subject.contains("échoué"));
    assert!(g.app.mails_to("alice@x.fr").is_empty());

    let (_, view) = g.app.get("/api/editions/current", &g.admin).await;
    assert_eq!(view["status"], "failed");
    assert!(view["failureReason"].as_str().unwrap().contains("au moins 3"));
    let (_, view) = g.app.get("/api/editions/current", &g.tokens[0].1).await;
    assert!(view["failureReason"].is_null(), "la raison est réservée à l'admin");

    // Repousser la deadline rouvre l'édition.
    let deadline = (g.app.clock.now() + Duration::days(3)).to_rfc3339();
    let (_, view) = g.app.patch("/api/admin/editions/current", &g.admin, json!({ "deadline": deadline })).await;
    assert_eq!(view["status"], "open");
    assert!(view["failureReason"].is_null());
}

#[tokio::test]
async fn contrainte_de_l_an_dernier_insoluble() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    let alice = g.app.player_id("Alice").await;
    let bob = g.app.player_id("Bob").await;
    let pool = &g.app.state.pool;
    let old: i64 = sqlx::query_scalar(
        "INSERT INTO editions (year, budget_eur, deadline, status, drawn_at, created_at)
         VALUES (2025, 30, '2025-12-01T00:00:00Z', 'drawn', '2025-12-01T00:00:00Z', '2025-11-01T00:00:00Z') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    for (giver, receiver) in [(alice, bob), (bob, alice)] {
        sqlx::query("INSERT INTO assignments (edition_id, giver_id, receiver_id, mail_status) VALUES (?, ?, ?, 'none')")
            .bind(old).bind(giver).bind(receiver).execute(pool).await.unwrap();
    }
    let (status, body) = g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["message"].as_str().unwrap().contains("l'an dernier"));
}

#[tokio::test]
async fn echec_smtp_puis_renvoi() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.mailer.failing.lock().unwrap().insert("bob@x.fr".into());
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    g.app.settle().await;

    let (_, mails) = g.app.get("/api/admin/editions/current/mails", &g.admin).await;
    let bob_row = mails.as_array().unwrap().iter().find(|r| r["pseudo"] == "Bob").unwrap().clone();
    assert_eq!(bob_row["mailStatus"], "failed");
    assert!(bob_row["mailError"].as_str().unwrap().contains("boîte pleine"));

    g.app.mailer.failing.lock().unwrap().clear();
    let bob = g.app.player_id("Bob").await;
    let (status, _) = g.app.post(&format!("/api/admin/editions/current/mails/{bob}/resend"), &g.admin, json!({})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    g.app.settle().await;
    let (_, mails) = g.app.get("/api/admin/editions/current/mails", &g.admin).await;
    let bob_row = mails.as_array().unwrap().iter().find(|r| r["pseudo"] == "Bob").unwrap().clone();
    assert_eq!(bob_row["mailStatus"], "sent");
    assert_eq!(g.app.mails_to("bob@x.fr").len(), 1);
}

#[tokio::test]
async fn renvoi_pour_un_non_participant_404() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    let nathan = g.app.player_id("Nathan").await;
    let (status, _) = g.app.post(&format!("/api/admin/editions/current/mails/{nathan}/resend"), &g.admin, json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn annuler_le_tirage_rouvre_les_inscriptions() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    g.app.settle().await;
    let deadline = (g.app.clock.now() + Duration::days(2)).to_rfc3339();
    let (status, view) = g.app.delete("/api/admin/editions/current/draw", &g.admin, Some(json!({ "deadline": deadline }))).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["status"], "open");
    assert_eq!(view["participants"].as_array().unwrap().len(), 3, "les inscriptions sont conservées");
    let (status, _) = g.app.get("/api/editions/current/my-target", &g.tokens[0].1).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = g.app.delete("/api/admin/editions/current/draw", &g.admin, Some(json!({ "deadline": deadline }))).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("NOT_DRAWN")));
}

#[tokio::test]
async fn les_mails_en_attente_repartent_au_demarrage() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    g.app.settle().await;
    sqlx::query("UPDATE assignments SET mail_status = 'pending'").execute(&g.app.state.pool).await.unwrap();
    g.app.mailer.sent.lock().unwrap().clear();

    draw_service::resume_pending_mails(&g.app.state).await;
    g.app.settle().await;
    assert_eq!(g.app.mailer.sent.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn routes_de_tirage_reservees_a_l_admin() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    let (status, _) = g.app.post("/api/admin/editions/current/draw", &g.tokens[0].1, json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = g.app.get("/api/admin/editions/current/mails", &g.tokens[0].1).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cinq_tirages_simultanes_sur_une_base_fichier_un_seul_passe() {
    let app = TestApp::with_file_db().await;
    let admin = app.login(1, "Nathan", true);
    let deadline = (app.clock.now() + Duration::days(20)).to_rfc3339();
    let (status, _) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": deadline })).await;
    assert_eq!(status, StatusCode::CREATED);
    for (i, p) in ["Alice", "Bob", "Chloe", "Dan", "Emma"].iter().enumerate() {
        let t = app.login(10 + i as i64, p, false);
        let (status, _) = app.put("/api/editions/current/registration", &t, json!({ "participating": true })).await;
        assert_eq!(status, StatusCode::OK);
    }
    app.settle().await;
    let id = edition_id(&app).await;

    let runs = (0..5).map(|_| {
        let state = app.state.clone();
        tokio::spawn(async move { draw_service::run_draw(&state, id).await })
    });
    let mut outcomes = Vec::new();
    for run in runs.collect::<Vec<_>>() {
        outcomes.push(run.await.unwrap().expect("aucun tirage ne doit échouer sur un verrou"));
    }
    assert_eq!(outcomes.iter().filter(|o| **o == DrawOutcome::Drawn).count(), 1, "{outcomes:?}");
    assert_eq!(outcomes.iter().filter(|o| **o == DrawOutcome::AlreadyDone).count(), 4, "{outcomes:?}");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assignments WHERE edition_id = ?").bind(id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(count, 5);
    app.settle().await;
}

#[tokio::test]
async fn renvoi_d_un_mail_deja_parti_un_seul_nouveau_mail() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    g.app.settle().await;
    g.app.mailer.sent.lock().unwrap().clear();

    let alice = g.app.player_id("Alice").await;
    let (status, body) = g.app.post(&format!("/api/admin/editions/current/mails/{alice}/resend"), &g.admin, json!({})).await;
    assert_eq!((status, body["mailStatus"].as_str()), (StatusCode::ACCEPTED, Some("pending")));
    g.app.settle().await;
    assert_eq!(g.app.mails_to("alice@x.fr").len(), 1);
    assert_eq!(g.app.mailer.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn deux_envois_simultanes_du_meme_resultat_un_seul_mail() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    g.app.settle().await;
    g.app.mailer.sent.lock().unwrap().clear();

    let id = edition_id(&g.app).await;
    let alice = g.app.player_id("Alice").await;
    sqlx::query("UPDATE assignments SET mail_status = 'pending' WHERE edition_id = ? AND giver_id = ?")
        .bind(id)
        .bind(alice)
        .execute(&g.app.state.pool)
        .await
        .unwrap();
    let edition = secret_santa::repo::editions::by_id(&g.app.state.pool, id).await.unwrap().unwrap();
    let (a, b) = tokio::join!(
        mailing::send_result(&g.app.state, &edition, alice),
        mailing::send_result(&g.app.state, &edition, alice)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(g.app.mails_to("alice@x.fr").len(), 1);
    let status: String = sqlx::query_scalar("SELECT mail_status FROM assignments WHERE edition_id = ? AND giver_id = ?")
        .bind(id)
        .bind(alice)
        .fetch_one(&g.app.state.pool)
        .await
        .unwrap();
    assert_eq!(status, "sent");
}

#[tokio::test]
async fn un_envoi_interrompu_repart_au_demarrage() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    g.app.settle().await;
    sqlx::query("UPDATE assignments SET mail_status = 'sending'").execute(&g.app.state.pool).await.unwrap();
    g.app.mailer.sent.lock().unwrap().clear();

    draw_service::resume_pending_mails(&g.app.state).await;
    g.app.settle().await;
    assert_eq!(g.app.mailer.sent.lock().unwrap().len(), 3);
}
