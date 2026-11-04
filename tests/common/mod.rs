#![allow(dead_code)]

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::Response,
};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use secret_santa::{
    APP_SLUG,
    auth::{AuthClient, AuthError, SsoSession, SsoUser},
    clock::ManualClock,
    config::Config,
    db,
    mail::{Mail, MailError, Mailer},
    routes,
    state::AppState,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::task::TaskTracker;
use tower::ServiceExt;

pub const ORIGIN: &str = "https://santa.test";

#[derive(Default)]
pub struct FakeAuth {
    pub sessions: Mutex<HashMap<String, SsoSession>>,
    pub users: Mutex<Vec<SsoUser>>,
    pub down: AtomicBool,
    /// Annuaire seul indisponible, les sessions répondent.
    pub directory_down: AtomicBool,
}

#[async_trait]
impl AuthClient for FakeAuth {
    async fn session(&self, token: &str) -> Result<Option<SsoSession>, AuthError> {
        if self.down.load(Ordering::SeqCst) {
            return Err(AuthError::Unavailable("panne simulée".into()));
        }
        Ok(self.sessions.lock().unwrap().get(token).cloned())
    }

    async fn list_users(&self, token: &str, _app: &str) -> Result<Vec<SsoUser>, AuthError> {
        if self.down.load(Ordering::SeqCst) || self.directory_down.load(Ordering::SeqCst) {
            return Err(AuthError::Unavailable("panne simulée".into()));
        }
        match self.sessions.lock().unwrap().get(token) {
            Some(s) if s.is_admin => Ok(self.users.lock().unwrap().clone()),
            _ => Err(AuthError::Forbidden),
        }
    }
}

#[derive(Default)]
pub struct RecordingMailer {
    pub sent: Mutex<Vec<Mail>>,
    pub failing: Mutex<HashSet<String>>,
}

#[async_trait]
impl Mailer for RecordingMailer {
    async fn send(&self, mail: &Mail) -> Result<(), MailError> {
        if self.failing.lock().unwrap().contains(&mail.to) {
            return Err(MailError("boîte pleine (simulé)".into()));
        }
        self.sent.lock().unwrap().push(mail.clone());
        Ok(())
    }
}

pub enum Part {
    Text(String),
    File {
        filename: String,
        content_type: String,
        bytes: Vec<u8>,
    },
}

pub fn text(v: impl Into<String>) -> Part {
    Part::Text(v.into())
}

pub struct TestApp {
    pub state: AppState,
    pub router: Router,
    pub auth: Arc<FakeAuth>,
    pub mailer: Arc<RecordingMailer>,
    pub clock: Arc<ManualClock>,
    pub tmp: tempfile::TempDir,
}

impl TestApp {
    /// Horloge figée au 1er novembre 2026, midi UTC.
    pub async fn new() -> Self {
        Self::with_pool(
            tempfile::tempdir().unwrap(),
            db::connect_in_memory().await.unwrap(),
        )
    }

    /// Comme `new`, mais sur une vraie base fichier (WAL, 5 connexions) :
    /// pour les tests de concurrence entre connexions.
    pub async fn with_file_db() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("santa.db");
        let pool = db::connect(path.to_str().unwrap()).await.unwrap();
        Self::with_pool(tmp, pool)
    }

    fn with_pool(tmp: tempfile::TempDir, pool: sqlx::SqlitePool) -> Self {
        let config = Config {
            bind_addr: "127.0.0.1:0".into(),
            database_path: ":memory:".into(),
            uploads_dir: tmp.path().join("uploads"),
            auth_url: "http://sso.test".into(),
            auth_login_url: "https://auth.test/login".into(),
            auth_public_url: "https://auth.test".into(),
            public_origin: ORIGIN.into(),
            admin_email: "admin@x.fr".into(),
            smtp: None,
            dev_mode: false,
            dev_user: None,
            mail_delay: Duration::ZERO,
        };
        let auth = Arc::new(FakeAuth::default());
        let mailer = Arc::new(RecordingMailer::default());
        let clock = Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap(),
        ));
        let state = AppState {
            pool,
            config: Arc::new(config),
            auth: auth.clone(),
            mailer: mailer.clone(),
            clock: clock.clone(),
            tasks: TaskTracker::new(),
        };
        let router = routes::router(state.clone());
        Self {
            state,
            router,
            auth,
            mailer,
            clock,
            tmp,
        }
    }

    fn email_of(pseudo: &str) -> String {
        format!("{}@x.fr", pseudo.to_lowercase())
    }

    /// Compte SSO connu de l'annuaire (accès à l'app) mais jamais connecté.
    pub fn add_sso_user(&self, sso_id: i64, pseudo: &str) {
        let mut users = self.auth.users.lock().unwrap();
        if !users.iter().any(|u| u.user_id == sso_id) {
            users.push(SsoUser {
                user_id: sso_id,
                email: Self::email_of(pseudo),
                pseudo: pseudo.into(),
                avatar_path: None,
            });
        }
    }

    /// Ouvre une session SSO avec accès à l'app et renvoie le jeton du cookie.
    pub fn login(&self, sso_id: i64, pseudo: &str, admin: bool) -> String {
        self.add_sso_user(sso_id, pseudo);
        self.session(sso_id, pseudo, admin, vec![APP_SLUG.into()])
    }

    pub fn login_without_app(&self, sso_id: i64, pseudo: &str) -> String {
        self.session(sso_id, pseudo, false, vec!["dashboard".into()])
    }

    fn session(&self, sso_id: i64, pseudo: &str, admin: bool, apps: Vec<String>) -> String {
        let token = format!("tok-{sso_id}");
        self.auth.sessions.lock().unwrap().insert(
            token.clone(),
            SsoSession {
                user_id: sso_id,
                email: Self::email_of(pseudo),
                pseudo: pseudo.into(),
                is_admin: admin,
                apps,
                avatar_path: None,
            },
        );
        token
    }

    pub fn request(
        method: Method,
        path: &str,
        token: Option<&str>,
    ) -> axum::http::request::Builder {
        let mut req = Request::builder().method(method.clone()).uri(path);
        if let Some(t) = token {
            req = req.header(header::COOKIE, format!("theme=dark; lim_session={t}"));
        }
        if method != Method::GET {
            req = req.header(header::ORIGIN, ORIGIN);
        }
        req
    }

    pub async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let req = Self::request(method, path, token);
        let req = match body {
            Some(v) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        self.send(req).await
    }

    pub async fn get(&self, path: &str, token: &str) -> (StatusCode, Value) {
        self.call(Method::GET, path, Some(token), None).await
    }
    pub async fn post(&self, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::POST, path, Some(token), Some(body)).await
    }
    pub async fn put(&self, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::PUT, path, Some(token), Some(body)).await
    }
    pub async fn patch(&self, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::PATCH, path, Some(token), Some(body))
            .await
    }
    pub async fn delete(
        &self,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.call(Method::DELETE, path, Some(token), body).await
    }

    pub async fn response(&self, req: Request<Body>) -> Response {
        self.router.clone().oneshot(req).await.unwrap()
    }

    pub async fn send(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.response(req).await;
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
        };
        (status, value)
    }

    pub async fn multipart(
        &self,
        method: Method,
        path: &str,
        token: &str,
        parts: Vec<(&str, Part)>,
    ) -> (StatusCode, Value) {
        let boundary = "XSANTABOUNDARYX";
        let mut body = Vec::new();
        for (name, part) in parts {
            body.extend(format!("--{boundary}\r\n").as_bytes());
            match part {
                Part::Text(v) => {
                    body.extend(
                        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{v}\r\n")
                            .as_bytes(),
                    );
                }
                Part::File {
                    filename,
                    content_type,
                    bytes,
                } => {
                    body.extend(
                        format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n").as_bytes(),
                    );
                    body.extend(bytes);
                    body.extend(b"\r\n");
                }
            }
        }
        body.extend(format!("--{boundary}--\r\n").as_bytes());
        let req = Self::request(method, path, Some(token))
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        self.send(req).await
    }

    /// Attend la fin de toutes les tâches de fond (mails).
    pub async fn settle(&self) {
        self.state.tasks.close();
        self.state.tasks.wait().await;
        self.state.tasks.reopen();
    }

    pub async fn player_id(&self, pseudo: &str) -> i64 {
        sqlx::query_scalar("SELECT id FROM players WHERE pseudo = ?")
            .bind(pseudo)
            .fetch_one(&self.state.pool)
            .await
            .unwrap()
    }

    pub fn mails_to(&self, email: &str) -> Vec<Mail> {
        self.mailer
            .sent
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.to == email)
            .cloned()
            .collect()
    }
}
