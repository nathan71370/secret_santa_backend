use crate::{APP_SLUG, auth::SESSION_COOKIE, error::ApiError, repo::players, state::AppState};
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, Method, header, request::Parts},
};

pub struct CurrentUser {
    pub player_id: i64,
    pub email: String,
    pub pseudo: String,
    /// URL absolue de la photo de profil, `None` sans photo.
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    /// Jeton du cookie SSO, relayé pour lire l'annuaire au nom de l'admin.
    pub token: String,
}

pub struct AdminUser(pub CurrentUser);

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
}

/// Le cookie SSO part vers tous les sous-domaines de limperiam.com : seule
/// l'origine du front Santa peut déclencher une écriture.
fn check_origin(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        Some(origin) if origin.trim_end_matches('/') == expected => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let unauthorized = || ApiError::Unauthorized {
            login_url: state.config.auth_login_url.clone(),
        };

        let token = match cookie_value(&parts.headers, SESSION_COOKIE) {
            Some(t) => t,
            None if state.config.dev_user.is_some() => "dev".to_string(),
            None => return Err(unauthorized()),
        };

        let session = match state.auth.session(&token).await {
            Ok(Some(s)) => s,
            Ok(None) => return Err(unauthorized()),
            Err(e) => {
                tracing::warn!(error = %e, "session SSO illisible : accès refusé");
                return Err(unauthorized());
            }
        };

        if !session.apps.iter().any(|a| a == APP_SLUG) {
            return Err(ApiError::Forbidden);
        }

        if !matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS) {
            check_origin(&parts.headers, &state.config.public_origin)?;
        }

        let mut conn = state.pool.acquire().await?;
        let Some(player_id) = players::upsert_sso(
            &mut conn,
            session.user_id,
            &session.email,
            &session.pseudo,
            session.avatar_path.as_deref(),
            state.now(),
        )
        .await?
        else {
            tracing::warn!(
                sso_user_id = session.user_id,
                "adresse déjà liée à un autre compte SSO : accès refusé"
            );
            return Err(ApiError::Forbidden);
        };

        Ok(CurrentUser {
            player_id,
            email: session.email.trim().to_lowercase(),
            pseudo: session.pseudo,
            avatar_url: players::avatar_url(
                &state.config.auth_public_url,
                session.avatar_path.as_deref(),
            ),
            is_admin: session.is_admin,
            token,
        })
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if !user.is_admin {
            return Err(ApiError::Forbidden);
        }
        Ok(AdminUser(user))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn lit_le_cookie_parmi_d_autres() {
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("a=1; lim_session=abc; b=2"),
        );
        assert_eq!(cookie_value(&h, "lim_session").as_deref(), Some("abc"));
        assert_eq!(cookie_value(&h, "absent"), None);
    }

    #[test]
    fn cookie_vide_ignore() {
        let mut h = HeaderMap::new();
        h.append(header::COOKIE, HeaderValue::from_static("lim_session="));
        assert_eq!(cookie_value(&h, "lim_session"), None);
    }

    #[test]
    fn controle_d_origine() {
        let mut h = HeaderMap::new();
        assert!(check_origin(&h, "https://santa.test").is_err());
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://tierlistrr.test"),
        );
        assert!(check_origin(&h, "https://santa.test").is_err());
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://santa.test"),
        );
        assert!(check_origin(&h, "https://santa.test").is_ok());
    }
}
