mod common;

use axum::http::StatusCode;
use chrono::Duration;
use common::TestApp;
use secret_santa::{clock::Clock, services::scheduler};
use serde_json::json;

async fn setup(deadline_in: Duration, joined: &[&str], silent: &[&str]) -> (TestApp, String) {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    for (i, p) in silent.iter().enumerate() {
        app.add_sso_user(100 + i as i64, p);
    }
    let tokens: Vec<_> = joined
        .iter()
        .enumerate()
        .map(|(i, p)| app.login(10 + i as i64, p, false))
        .collect();
    let deadline = (app.clock.now() + deadline_in).to_rfc3339();
    let (status, _) = app
        .post(
            "/api/admin/editions",
            &admin,
            json!({ "budgetEur": 30, "deadline": deadline }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    for t in &tokens {
        app.put(
            "/api/editions/current/registration",
            t,
            json!({ "participating": true }),
        )
        .await;
    }
    app.settle().await;
    app.mailer.sent.lock().unwrap().clear();
    (app, admin)
}

#[tokio::test]
async fn rien_avant_la_fenetre_de_relance() {
    let (app, _) = setup(Duration::days(10), &["Alice", "Bob", "Chloe"], &["Dan"]).await;
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert!(app.mailer.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn relance_une_seule_fois_aux_seuls_non_repondants() {
    let (app, admin) = setup(
        Duration::days(10),
        &["Alice", "Bob", "Chloe"],
        &["Dan", "Eve"],
    )
    .await;
    app.clock.advance(Duration::days(8) + Duration::hours(1)); // J-47h
    scheduler::tick(&app.state).await.unwrap();
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;

    let sent = app.mailer.sent.lock().unwrap().clone();
    let mut to: Vec<_> = sent.iter().map(|m| m.to.clone()).collect();
    to.sort();
    // Nathan (admin) n'a pas répondu non plus : il est relancé.
    assert_eq!(to, ["dan@x.fr", "eve@x.fr", "nathan@x.fr"]);
    assert!(sent.iter().all(|m| m.subject.contains("48 h")));
    let _ = admin;
}

#[tokio::test]
async fn pas_de_relance_si_la_deadline_etait_deja_proche() {
    let (app, _) = setup(Duration::hours(30), &["Alice", "Bob", "Chloe"], &["Dan"]).await;
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert!(app.mailer.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn tirage_automatique_a_la_deadline() {
    let (app, admin) = setup(Duration::days(10), &["Alice", "Bob", "Chloe"], &[]).await;
    app.clock.advance(Duration::days(10));
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    let (_, view) = app.get("/api/editions/current", &admin).await;
    assert_eq!(view["status"], "drawn");
    for email in ["alice@x.fr", "bob@x.fr", "chloe@x.fr"] {
        assert!(
            app.mails_to(email)
                .iter()
                .any(|m| m.subject.contains("ton tirage"))
        );
    }
    // Un tour de plus ne refait rien.
    app.mailer.sent.lock().unwrap().clear();
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert!(app.mailer.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn un_echec_n_est_pas_retente_automatiquement() {
    let (app, admin) = setup(Duration::days(1), &["Alice", "Bob"], &[]).await;
    app.clock.advance(Duration::days(2));
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert_eq!(app.mails_to("admin@x.fr").len(), 1);
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert_eq!(app.mails_to("admin@x.fr").len(), 1);
    let (_, view) = app.get("/api/editions/current", &admin).await;
    assert_eq!(view["status"], "failed");
}
