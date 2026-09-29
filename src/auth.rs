//! Client du service d'identité limperiam-auth, joint par le réseau Docker
//! interne. Le cookie `lim_session` de la personne est relayé tel quel.

use crate::{APP_SLUG, config::DevUser};
use async_trait::async_trait;
use reqwest::{StatusCode, header::COOKIE};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

pub const SESSION_COOKIE: &str = "lim_session";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SsoSession {
    pub user_id: i64,
    pub email: String,
    pub pseudo: String,
    pub is_admin: bool,
    #[serde(default)]
    pub apps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SsoUser {
    pub user_id: i64,
    pub email: String,
    pub pseudo: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("service d'identité indisponible : {0}")]
    Unavailable(String),
    #[error("refusé par le service d'identité")]
    Forbidden,
}

#[async_trait]
pub trait AuthClient: Send + Sync {
    /// `Ok(None)` : pas de session valide pour ce jeton.
    async fn session(&self, token: &str) -> Result<Option<SsoSession>, AuthError>;
    /// Comptes actifs ayant accès à `app`. Le jeton doit être celui d'un admin.
    async fn list_users(&self, token: &str, app: &str) -> Result<Vec<SsoUser>, AuthError>;
}

pub struct HttpAuthClient {
    http: reqwest::Client,
    base_url: String,
    ttl: Duration,
    /// Clé : SHA-256 du jeton — le jeton en clair ne reste jamais en mémoire.
    cache: Mutex<HashMap<String, (Instant, SsoSession)>>,
}

impl HttpAuthClient {
    pub fn new(base_url: impl Into<String>, ttl: Duration) -> Self {
        Self {
            http: reqwest::Client::builder().timeout(Duration::from_secs(5)).build().expect("client HTTP"),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            ttl,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn cache_key(token: &str) -> String {
        hex::encode(Sha256::digest(token.as_bytes()))
    }

    fn cookie(token: &str) -> String {
        format!("{SESSION_COOKIE}={token}")
    }
}

fn unavailable(e: impl std::fmt::Display) -> AuthError {
    AuthError::Unavailable(e.to_string())
}

#[async_trait]
impl AuthClient for HttpAuthClient {
    async fn session(&self, token: &str) -> Result<Option<SsoSession>, AuthError> {
        let key = Self::cache_key(token);
        if let Some((at, session)) = self.cache.lock().unwrap().get(&key) {
            if at.elapsed() < self.ttl {
                return Ok(Some(session.clone()));
            }
        }

        let res = self
            .http
            .get(format!("{}/api/session", self.base_url))
            .header(COOKIE, Self::cookie(token))
            .send()
            .await
            .map_err(unavailable)?;

        match res.status() {
            StatusCode::UNAUTHORIZED => Ok(None),
            status if status.is_success() => {
                let session: SsoSession = res.json().await.map_err(unavailable)?;
                let mut cache = self.cache.lock().unwrap();
                if cache.len() > 1000 {
                    let ttl = self.ttl;
                    cache.retain(|_, (at, _)| at.elapsed() < ttl);
                }
                cache.insert(key, (Instant::now(), session.clone()));
                Ok(Some(session))
            }
            status => Err(unavailable(format!("statut {status}"))),
        }
    }

    async fn list_users(&self, token: &str, app: &str) -> Result<Vec<SsoUser>, AuthError> {
        #[derive(Deserialize)]
        struct Body {
            users: Vec<SsoUser>,
        }
        let res = self
            .http
            .get(format!("{}/api/admin/users", self.base_url))
            .query(&[("app", app)])
            .header(COOKIE, Self::cookie(token))
            .send()
            .await
            .map_err(unavailable)?;
        match res.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(AuthError::Forbidden),
            status if status.is_success() => Ok(res.json::<Body>().await.map_err(unavailable)?.users),
            status => Err(unavailable(format!("statut {status}"))),
        }
    }
}

/// Développement local uniquement (`AUTH_DEV_USER`, refusé hors
/// `APP_ENV=development` par la configuration).
pub struct DevAuthClient {
    user: DevUser,
}

impl DevAuthClient {
    pub fn new(user: DevUser) -> Self {
        Self { user }
    }
}

#[async_trait]
impl AuthClient for DevAuthClient {
    async fn session(&self, _token: &str) -> Result<Option<SsoSession>, AuthError> {
        Ok(Some(SsoSession {
            user_id: 1,
            email: self.user.email.clone(),
            pseudo: self.user.pseudo.clone(),
            is_admin: self.user.is_admin,
            apps: vec![APP_SLUG.to_string()],
        }))
    }

    async fn list_users(&self, _token: &str, _app: &str) -> Result<Vec<SsoUser>, AuthError> {
        Ok(vec![SsoUser { user_id: 1, email: self.user.email.clone(), pseudo: self.user.pseudo.clone() }])
    }
}
