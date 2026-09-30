mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};

#[tokio::test]
async fn health_repond_ok() {
    let app = common::TestApp::new().await;
    let res = app
        .response(Request::get("/api/health").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
}
