use crate::error::ApiError;
use axum::extract::{FromRequest, FromRequestParts, Multipart, Request, multipart::MultipartError};
use axum::http::StatusCode;

/// Comme `axum::Json` en entrée, mais les rejets sortent au format d'erreur JSON de l'API.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);

/// Comme `axum::extract::Path` : un segment illisible donne un 404 JSON.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct ApiPath<T>(pub T);

/// Comme `Multipart` : un corps qui n'est pas un formulaire donne un 400 JSON.
pub struct ApiMultipart(pub Multipart);

impl<S: Send + Sync> FromRequest<S> for ApiMultipart {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(Multipart::from_request(req, state).await?))
    }
}

/// Erreur de lecture d'un champ : corps au-delà de `DefaultBodyLimit` → 413
/// avec `too_large`, sinon formulaire invalide.
pub fn multipart_error(too_large: &'static str) -> impl Fn(MultipartError) -> ApiError + Copy {
    move |e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::PayloadTooLarge(too_large.into())
        } else {
            ApiError::bad_request("INVALID_FORM", "Formulaire invalide.")
        }
    }
}
