use secret_santa::auth::{AuthClient, AuthError, DevAuthClient, HttpAuthClient};
use secret_santa::config::DevUser;
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn session_body() -> serde_json::Value {
    json!({ "userId": 7, "email": "Alice@X.fr", "pseudo": "Alice", "isAdmin": false, "groups": ["potes"], "apps": ["santa", "dashboard"] })
}

#[tokio::test]
async fn session_valide_relaie_le_cookie_et_est_mise_en_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/session"))
        .and(header("cookie", "lim_session=tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(session_body()))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    let s = client.session("tok").await.unwrap().unwrap();
    assert_eq!(
        (s.user_id, s.pseudo.as_str(), s.is_admin),
        (7, "Alice", false)
    );
    assert_eq!(s.apps, ["santa", "dashboard"]);
    assert_eq!(s.avatar_path, None);
    // Deuxième appel servi par le cache : `expect(1)` est vérifié à la fin du test.
    assert!(client.session("tok").await.unwrap().is_some());
}

#[tokio::test]
async fn cache_expire() {
    let server = MockServer::start().await;
    Mock::given(path("/api/session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(session_body()))
        .expect(2)
        .mount(&server)
        .await;
    let client = HttpAuthClient::new(server.uri(), Duration::ZERO);
    client.session("tok").await.unwrap();
    client.session("tok").await.unwrap();
}

#[tokio::test]
async fn session_inconnue_donne_none() {
    let server = MockServer::start().await;
    Mock::given(path("/api/session"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({ "error": "NOT_AUTHENTICATED" })),
        )
        .mount(&server)
        .await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    assert_eq!(client.session("tok").await, Ok(None));
}

#[tokio::test]
async fn erreur_serveur_ou_reponse_illisible_donne_unavailable() {
    let server = MockServer::start().await;
    Mock::given(path("/api/session"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    assert!(matches!(
        client.session("a").await,
        Err(AuthError::Unavailable(_))
    ));

    let server = MockServer::start().await;
    Mock::given(path("/api/session"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pas du json"))
        .mount(&server)
        .await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    assert!(matches!(
        client.session("a").await,
        Err(AuthError::Unavailable(_))
    ));
}

#[tokio::test]
async fn sso_injoignable_donne_unavailable() {
    let client = HttpAuthClient::new("http://127.0.0.1:1", Duration::from_secs(60));
    assert!(matches!(
        client.session("a").await,
        Err(AuthError::Unavailable(_))
    ));
}

#[tokio::test]
async fn list_users_filtre_par_app_et_relaie_le_cookie() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/users"))
        .and(query_param("app", "santa"))
        .and(header("cookie", "lim_session=admin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "users": [
            { "userId": 1, "email": "a@x.fr", "pseudo": "A", "avatarPath": "/avatars/1.jpg" },
            { "userId": 2, "email": "b@x.fr", "pseudo": "B", "avatarPath": null },
            { "userId": 3, "email": "c@x.fr", "pseudo": "C" }
        ] })))
        .mount(&server)
        .await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    let users = client.list_users("admin", "santa").await.unwrap();
    assert_eq!(
        users.iter().map(|u| u.user_id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        users
            .iter()
            .map(|u| u.avatar_path.as_deref())
            .collect::<Vec<_>>(),
        [Some("/avatars/1.jpg"), None, None]
    );
}

#[tokio::test]
async fn list_users_refuse_un_non_admin() {
    let server = MockServer::start().await;
    Mock::given(path("/api/admin/users"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    assert_eq!(
        client.list_users("pote", "santa").await,
        Err(AuthError::Forbidden)
    );
}

#[tokio::test]
async fn client_de_dev() {
    let client = DevAuthClient::new(DevUser {
        email: "moi@x.fr".into(),
        pseudo: "Moi".into(),
        is_admin: true,
    });
    let s = client.session("peu importe").await.unwrap().unwrap();
    assert!(s.is_admin);
    assert_eq!(s.apps, ["santa"]);
    assert_eq!(client.list_users("x", "santa").await.unwrap().len(), 1);
}
