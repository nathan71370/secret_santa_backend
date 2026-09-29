use axum::extract::FromRequest;

/// Comme `axum::Json` en entrée, mais les rejets sortent au format d'erreur JSON de l'API.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(crate::error::ApiError))]
pub struct ApiJson<T>(pub T);
