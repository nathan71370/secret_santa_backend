mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use common::{Part, TestApp, text};
use image::{ImageFormat, RgbImage};
use std::io::Cursor;

fn png() -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 48, image::Rgb([10, 120, 60])))
        .write_to(&mut out, ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn photo(bytes: Vec<u8>) -> Part {
    Part::File {
        filename: "cadeau.png".into(),
        content_type: "image/png".into(),
        bytes,
    }
}

#[tokio::test]
async fn ajouter_un_cadeau_avec_photo_et_le_voir_sur_le_profil() {
    let app = TestApp::new().await;
    let alice = app.login(1, "Alice", false);
    let bob = app.login(2, "Bob", false);

    let (status, gift) = app
        .multipart(
            Method::POST,
            "/api/me/gifts",
            &alice,
            vec![
                ("year", text("2025")),
                ("description", text("  Un pull moche avec un renne  ")),
                ("photo", photo(png())),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{gift}");
    assert_eq!(gift["description"], "Un pull moche avec un renne");
    let url = gift["photoUrl"].as_str().unwrap().to_string();
    assert!(url.starts_with("/api/uploads/") && url.ends_with(".jpg"));

    // Bob voit le profil d'Alice et sa photo.
    let alice_id = app.player_id("Alice").await;
    let (status, profile) = app.get(&format!("/api/players/{alice_id}"), &bob).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["pseudo"], "Alice");
    assert_eq!(profile["gifts"][0]["year"], 2025);

    let req = Request::get(&url)
        .header(header::COOKIE, format!("lim_session={bob}"))
        .body(Body::empty())
        .unwrap();
    let res = app.response(req).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "image/jpeg");

    // Alice apparaît dans la liste des joueurs grâce à son cadeau.
    let (_, players) = app.get("/api/players", &bob).await;
    assert!(
        players
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["pseudo"] == "Alice")
    );
}

#[tokio::test]
async fn photo_sans_session_401() {
    let app = TestApp::new().await;
    let req = Request::get("/api/uploads/0f8c2c1e-4b6a-4c43-9a55-2f1d7a3b9e10.jpg")
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.response(req).await.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn nom_de_photo_invalide_404() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let (status, _) = app.get("/api/uploads/..%2Fsanta.db", &t).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn validations() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let cases = vec![
        (
            vec![("year", text("1999")), ("description", text("x"))],
            "INVALID_YEAR",
        ),
        (
            vec![("year", text("2028")), ("description", text("x"))],
            "INVALID_YEAR",
        ),
        (
            vec![("year", text("2025")), ("description", text("   "))],
            "INVALID_DESCRIPTION",
        ),
        (
            vec![
                ("year", text("2025")),
                ("description", text("x".repeat(501))),
            ],
            "INVALID_DESCRIPTION",
        ),
        (
            vec![
                ("year", text("2025")),
                ("description", text("x")),
                ("photo", photo(b"pas une image".to_vec())),
            ],
            "INVALID_PHOTO",
        ),
    ];
    for (parts, code) in cases {
        let (status, body) = app
            .multipart(Method::POST, "/api/me/gifts", &t, parts)
            .await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some(code))
        );
    }
}

#[tokio::test]
async fn modifier_et_supprimer_seulement_ses_cadeaux() {
    let app = TestApp::new().await;
    let alice = app.login(1, "Alice", false);
    let bob = app.login(2, "Bob", false);
    let (_, gift) = app
        .multipart(
            Method::POST,
            "/api/me/gifts",
            &alice,
            vec![
                ("year", text("2024")),
                ("description", text("Un livre")),
                ("photo", photo(png())),
            ],
        )
        .await;
    let id = gift["id"].as_i64().unwrap();
    let old_file = gift["photoUrl"]
        .as_str()
        .unwrap()
        .trim_start_matches("/api/uploads/")
        .to_string();

    let (status, _) = app
        .multipart(
            Method::PUT,
            &format!("/api/me/gifts/{id}"),
            &bob,
            vec![("year", text("2024")), ("description", text("Piraté"))],
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.delete(&format!("/api/me/gifts/{id}"), &bob, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, updated) = app
        .multipart(
            Method::PUT,
            &format!("/api/me/gifts/{id}"),
            &alice,
            vec![
                ("year", text("2025")),
                ("description", text("Deux livres")),
                ("removePhoto", text("true")),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(
        (updated["year"].as_i64(), updated["description"].as_str()),
        (Some(2025), Some("Deux livres"))
    );
    assert!(updated["photoUrl"].is_null());
    assert!(
        !app.state.config.uploads_dir.join(&old_file).exists(),
        "l'ancienne photo est supprimée"
    );

    let (status, _) = app
        .delete(&format!("/api/me/gifts/{id}"), &alice, None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let alice_id = app.player_id("Alice").await;
    let (_, profile) = app.get(&format!("/api/players/{alice_id}"), &bob).await;
    assert_eq!(profile["gifts"], serde_json::json!([]));
}

#[tokio::test]
async fn profil_inconnu_404() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let (status, _) = app.get("/api/players/9999", &t).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn photo_de_7_mo_413() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let (status, body) = app
        .multipart(
            Method::POST,
            "/api/me/gifts",
            &t,
            vec![
                ("year", text("2025")),
                ("description", text("x")),
                ("photo", photo(vec![0u8; 7 * 1024 * 1024])),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        body,
        serde_json::json!({ "error": "PAYLOAD_TOO_LARGE", "message": "Photo trop lourde (5 Mo maximum)." })
    );
}
