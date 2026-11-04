mod common;

use chrono::{TimeZone, Utc};
use common::TestApp;
use secret_santa::{auth::SsoUser, db, repo::players};

async fn avatar_of(conn: &mut sqlx::SqliteConnection, id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT avatar_path FROM players WHERE id = ?")
        .bind(id)
        .fetch_one(conn)
        .await
        .unwrap()
}

fn user(id: i64, pseudo: &str, avatar: Option<&str>) -> SsoUser {
    SsoUser {
        user_id: id,
        email: format!("{}@x.fr", pseudo.to_lowercase()),
        pseudo: pseudo.into(),
        avatar_path: avatar.map(Into::into),
    }
}

#[tokio::test]
async fn upsert_sso_enregistre_change_et_efface_la_photo() {
    let pool = db::connect_in_memory().await.unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
    let mut conn = pool.acquire().await.unwrap();

    let id = players::upsert_sso(&mut conn, 7, "a@x.fr", "A", Some("/avatars/7.jpg"), now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        avatar_of(&mut conn, id).await.as_deref(),
        Some("/avatars/7.jpg")
    );

    players::upsert_sso(&mut conn, 7, "a@x.fr", "A", Some("/avatars/7.webp"), now)
        .await
        .unwrap();
    assert_eq!(
        avatar_of(&mut conn, id).await.as_deref(),
        Some("/avatars/7.webp")
    );

    // Photo retirée sur le dashboard : les initiales reprennent.
    players::upsert_sso(&mut conn, 7, "a@x.fr", "A", None, now)
        .await
        .unwrap();
    assert_eq!(avatar_of(&mut conn, id).await, None);

    // Jamais d'URL vers un autre hôte.
    players::upsert_sso(&mut conn, 7, "a@x.fr", "A", Some("//evil.test/x.jpg"), now)
        .await
        .unwrap();
    assert_eq!(avatar_of(&mut conn, id).await, None);
}

#[tokio::test]
async fn upsert_sso_photo_d_un_joueur_importe_relie_par_adresse() {
    let pool = db::connect_in_memory().await.unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
    let mut conn = pool.acquire().await.unwrap();
    let imported = players::upsert_imported(&mut conn, "b@x.fr", "B", now)
        .await
        .unwrap();
    let id = players::upsert_sso(&mut conn, 9, "b@x.fr", "B", Some("/avatars/9.png"), now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(id, imported);
    assert_eq!(
        avatar_of(&mut conn, id).await.as_deref(),
        Some("/avatars/9.png")
    );
}

#[tokio::test]
async fn sync_access_enregistre_les_photos_de_tout_l_annuaire() {
    let pool = db::connect_in_memory().await.unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
    players::sync_access(
        &pool,
        &[
            user(1, "Alice", Some("/avatars/1.jpg")),
            user(2, "Bob", None),
        ],
        now,
    )
    .await
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    let alice: i64 = sqlx::query_scalar("SELECT id FROM players WHERE sso_user_id = 1")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    let bob: i64 = sqlx::query_scalar("SELECT id FROM players WHERE sso_user_id = 2")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(
        avatar_of(&mut conn, alice).await.as_deref(),
        Some("/avatars/1.jpg")
    );
    assert_eq!(avatar_of(&mut conn, bob).await, None);
    drop(conn);

    players::sync_access(
        &pool,
        &[
            user(1, "Alice", None),
            user(2, "Bob", Some("/avatars/2.png")),
        ],
        now,
    )
    .await
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(avatar_of(&mut conn, alice).await, None);
    assert_eq!(
        avatar_of(&mut conn, bob).await.as_deref(),
        Some("/avatars/2.png")
    );
}

fn set_directory_avatar(app: &TestApp, sso_id: i64, avatar: Option<&str>) {
    let mut users = app.auth.users.lock().unwrap();
    let u = users.iter_mut().find(|u| u.user_id == sso_id).unwrap();
    u.avatar_path = avatar.map(Into::into);
}

#[tokio::test]
async fn me_renvoie_l_url_absolue_de_la_photo() {
    let app = TestApp::new().await;
    let token = app.login(1, "Alice", false);
    let (_, body) = app.get("/api/me", &token).await;
    assert_eq!(body["avatarUrl"], serde_json::Value::Null);

    app.auth
        .sessions
        .lock()
        .unwrap()
        .get_mut(&token)
        .unwrap()
        .avatar_path = Some("/avatars/1.jpg".into());
    let (_, body) = app.get("/api/me", &token).await;
    assert_eq!(body["avatarUrl"], "https://auth.test/avatars/1.jpg");
}

#[tokio::test]
async fn un_admin_qui_ouvre_les_participants_recupere_toutes_les_photos() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let bob_token = app.login(2, "Bob", false);
    let pote = app.login(3, "Chloe", false);
    // Bob a joué (un cadeau), puis a mis une photo sans se reconnecter depuis.
    app.get("/api/me", &bob_token).await;
    app.get("/api/me", &pote).await;
    let bob = app.player_id("Bob").await;
    sqlx::query("INSERT INTO gifts (player_id, year, description, created_at) VALUES (?, 2025, 'Livre', '2025-12-25T00:00:00Z')")
        .bind(bob)
        .execute(&app.state.pool)
        .await
        .unwrap();
    set_directory_avatar(&app, 2, Some("/avatars/2.jpg"));

    let bob_url = |body: &serde_json::Value| {
        body.as_array()
            .unwrap()
            .iter()
            .find(|p| p["pseudo"] == "Bob")
            .unwrap()["avatarUrl"]
            .clone()
    };

    // Un joueur ordinaire ne relit pas l'annuaire.
    let (_, body) = app.get("/api/players", &pote).await;
    assert_eq!(bob_url(&body), serde_json::Value::Null);

    let (status, body) = app.get("/api/players", &admin).await;
    assert_eq!(status, 200);
    assert_eq!(bob_url(&body), "https://auth.test/avatars/2.jpg");

    let (_, profile) = app.get(&format!("/api/players/{bob}"), &pote).await;
    assert_eq!(profile["avatarUrl"], "https://auth.test/avatars/2.jpg");
}

#[tokio::test]
async fn annuaire_en_panne_la_liste_reste_servie_a_l_admin() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.get("/api/me", &admin).await;
    app.auth
        .directory_down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (status, body) = app.get("/api/players", &admin).await;
    assert_eq!(status, 200);
    assert!(body.is_array());
}
