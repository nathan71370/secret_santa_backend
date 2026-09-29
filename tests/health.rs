use axum::{body::Body, http::Request};
use tower::ServiceExt;

#[tokio::test]
async fn health_repond_ok_sans_authentification() {
    let res = secret_santa::routes::router()
        .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}
