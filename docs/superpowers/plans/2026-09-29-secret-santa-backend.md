# Secret Santa — Backend Rust Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Backend HTTP du Secret Santa : inscriptions avec deadline, tirage automatique, mails, profils et cadeaux, import de l'historique Java — déployable en stack Komodo `secret_santa_backend`.

**Architecture:** Binaire axum unique, état partagé `AppState` (pool SQLite, config, client SSO, mailer, horloge, `TaskTracker` des tâches de fond). La logique pure (tirage, parseur `players.txt`, photos, gabarits de mails) est isolée et testée unitairement ; les dépendances externes (SSO, SMTP, horloge) sont derrière des traits pour que les tests d'intégration pilotent tout le routeur sur une SQLite en mémoire.

**Tech Stack:** Rust 2024 (rustc 1.96), axum 0.8, tokio, sqlx 0.8 (SQLite, migrations embarquées), reqwest 0.12, lettre 0.11, rand 0.9, image 0.25, chrono + chrono-tz, tokio-util (TaskTracker). Tests : `cargo test`, wiremock, tower `oneshot`.

**Spec:** `docs/superpowers/specs/2026-09-29-secret-santa-design.md`

**Prérequis :** le plan `frontend/limperiam-auth/docs/superpowers/plans/2026-09-29-admin-users-listing.md` (route `GET /api/admin/users?app=`) — nécessaire en production seulement, les tests simulent le SSO.

## Global Constraints

- Toutes les routes sous `/api`. Erreurs JSON `{ "error": "CODE", "message": "texte FR" }` ; 401 ajoute `"loginUrl"`.
- Slug de l'application dans le SSO : `santa`. Cookie SSO : `lim_session`.
- Échec fermé : SSO injoignable ou réponse illisible → 401.
- Toute méthode autre que `GET`/`HEAD`/`OPTIONS` exige `Origin == PUBLIC_ORIGIN`, sinon 403.
- Personne, admin compris, ne voit les paires : aucune réponse admin ne contient de receveur.
- Minimum 3 participants ; 1000 essais max ; contrainte « pas la même cible que l'édition `drawn` d'année strictement inférieure la plus récente ».
- Une édition par année (année civile en `Europe/Paris`). Dates stockées et sérialisées en UTC (RFC 3339).
- Relance 48 h avant la deadline, une seule fois, seulement si l'édition a été créée au moins 48 h avant la deadline.
- Photos : JPEG/PNG/WebP détectés sur les octets, 5 Mo max, 1600 px max, réencodées en JPEG (EXIF supprimé, orientation appliquée).
- Rien n'est journalisé qui contienne une adresse mail ou une cible.
- **Dans une transaction, n'utiliser que `&mut *tx`** — jamais `&state.pool` : la base de test n'a qu'une connexion, un accès au pool pendant une transaction bloquerait.
- Code, commentaires, messages et noms de tests en français (comme le reste des projets de Nathan) ; identifiants Rust en anglais.
- Chaque commit se termine par `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Structure des fichiers

```
backend/secret_santa_rs/
  Cargo.toml
  migrations/0001_init.sql
  src/
    main.rs               câblage, démarrage, arrêt propre
    lib.rs                déclarations de modules, APP_SLUG
    config.rs             Config::from_env / from_lookup
    error.rs              ApiError -> réponse JSON
    clock.rs              Clock, SystemClock, ManualClock, paris_year
    db.rs                 connect / connect_in_memory + migrations
    state.rs              AppState
    draw.rs               tirage pur
    legacy_import.rs      parseur players.txt pur
    photos.rs             normalisation + stockage des photos
    auth.rs               AuthClient, HttpAuthClient, DevAuthClient
    mail/mod.rs           Mail, Mailer, SmtpMailer, LogMailer
    mail/templates.rs     les 4 mails
    repo/mod.rs
    repo/players.rs
    repo/editions.rs
    repo/assignments.rs
    repo/gifts.rs
    services/mod.rs
    services/mailing.rs   envois en tâche de fond
    services/draw_service.rs
    services/scheduler.rs
    routes/mod.rs         routeur
    routes/extract.rs     CurrentUser, AdminUser, contrôle d'origine
    routes/me.rs
    routes/editions.rs
    routes/players.rs
    routes/gifts.rs
    routes/admin.rs
    routes/import.rs
  tests/
    common/mod.rs         TestApp, FakeAuth, RecordingMailer, multipart
    health.rs, db.rs, auth_client.rs, me.rs, editions.rs, draw.rs,
    scheduler.rs, gifts.rs, import.rs
  Dockerfile, compose.yaml, .dockerignore, .gitignore, .env.example, README.md
```

---

### Task 1: Squelette, configuration, erreurs, `/api/health`

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/lib.rs`, `src/main.rs`, `src/config.rs`, `src/error.rs`, `src/routes/mod.rs`, `tests/health.rs`

**Interfaces:**
- Produces: `Config`, `SmtpConfig`, `DevUser`, `ConfigError`, `Config::from_lookup(impl Fn(&str) -> Option<String>)`, `Config::from_env()` ; `ApiError` (variantes `Unauthorized { login_url }`, `Forbidden`, `NotFound(String)`, `BadRequest { code, message }`, `Conflict { code, message }`, `Unprocessable { code, message }`, `Upstream`, `Internal(String)`), constructeurs `ApiError::bad_request/conflict/unprocessable/not_found`, `ApiResult<T>` ; `routes::router()` (remplacé en Task 7 par `router(state)`) ; `routes::health`.

- [ ] **Step 1: Créer le projet et ses dépendances**

```bash
cd /Users/nathanmercier/Documents/Project/backend/secret_santa_rs
cargo init --lib --name secret_santa
```

Remplacer `Cargo.toml` :

```toml
[package]
name = "secret_santa"
version = "0.1.0"
edition = "2024"

[dependencies]
async-trait = "0.1"
axum = { version = "0.8", features = ["multipart"] }
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.10"
hex = "0.4"
image = { version = "0.25", default-features = false, features = ["jpeg", "png", "webp"] }
lettre = { version = "0.11", default-features = false, features = ["builder", "hostname", "smtp-transport", "tokio1", "tokio1-rustls-tls"] }
rand = "0.9"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "sqlite", "chrono", "migrate", "macros"] }
thiserror = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time", "signal", "fs", "net"] }
tokio-util = { version = "0.7", features = ["rt"] }
tower-http = { version = "0.6", features = ["trace"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
http-body-util = "0.1"
tempfile = "3"
tower = { version = "0.5", features = ["util"] }
wiremock = "0.6"
```

Si `cargo build` refuse un nom de feature de lettre (elles ont bougé entre versions 0.11.x), lancer `cargo info lettre` et prendre l'équivalent « tokio1 + rustls ».

`.gitignore` :

```
/target
/data
.env
```

- [ ] **Step 2: Écrire les tests de configuration (échouent : module absent)**

Créer `src/config.rs` avec seulement le module de test pour commencer :

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| map.get(k).cloned()
    }

    const PROD: &[(&str, &str)] = &[
        ("ADMIN_EMAIL", "Nathan@X.fr"),
        ("SMTP_HOST", "smtp.x.fr"),
        ("SMTP_FROM", "Santa <santa@x.fr>"),
    ];

    #[test]
    fn valeurs_par_defaut_en_production() {
        let c = Config::from_lookup(lookup(PROD)).unwrap();
        assert_eq!(c.bind_addr, "0.0.0.0:8080");
        assert_eq!(c.database_path, "/data/santa.db");
        assert_eq!(c.uploads_dir, std::path::PathBuf::from("/data/uploads"));
        assert_eq!(c.auth_url, "http://limperiam-auth:3000");
        assert_eq!(c.auth_login_url, "https://auth.limperiam.com/login");
        assert_eq!(c.public_origin, "https://santa.limperiam.com");
        assert_eq!(c.admin_email, "nathan@x.fr");
        assert_eq!(c.mail_delay, std::time::Duration::from_millis(1000));
        assert!(!c.dev_mode);
        let smtp = c.smtp.unwrap();
        assert_eq!(smtp.port, 587);
        assert_eq!(smtp.user, None);
    }

    #[test]
    fn smtp_host_obligatoire_en_production() {
        let err = Config::from_lookup(lookup(&[("ADMIN_EMAIL", "a@x.fr"), ("SMTP_FROM", "a@x.fr")])).unwrap_err();
        assert_eq!(err, ConfigError::Missing("SMTP_HOST"));
    }

    #[test]
    fn admin_email_obligatoire() {
        let err = Config::from_lookup(lookup(&[("SMTP_HOST", "h"), ("SMTP_FROM", "a@x.fr")])).unwrap_err();
        assert_eq!(err, ConfigError::Missing("ADMIN_EMAIL"));
    }

    #[test]
    fn une_variable_vide_compte_comme_absente() {
        let err = Config::from_lookup(lookup(&[("ADMIN_EMAIL", "a@x.fr"), ("SMTP_HOST", "  "), ("SMTP_FROM", "a@x.fr")])).unwrap_err();
        assert_eq!(err, ConfigError::Missing("SMTP_HOST"));
    }

    #[test]
    fn developpement_sans_smtp() {
        let c = Config::from_lookup(lookup(&[("ADMIN_EMAIL", "a@x.fr"), ("APP_ENV", "development")])).unwrap();
        assert!(c.dev_mode);
        assert!(c.smtp.is_none());
    }

    #[test]
    fn utilisateur_de_dev_interdit_en_production() {
        let mut env = PROD.to_vec();
        env.push(("AUTH_DEV_USER", "a@x.fr,A,admin"));
        assert_eq!(Config::from_lookup(lookup(&env)).unwrap_err(), ConfigError::DevUserInProduction);
    }

    #[test]
    fn utilisateur_de_dev_lu_en_developpement() {
        let c = Config::from_lookup(lookup(&[
            ("ADMIN_EMAIL", "a@x.fr"),
            ("APP_ENV", "development"),
            ("AUTH_DEV_USER", " Moi@X.fr , Moi , admin "),
        ]))
        .unwrap();
        assert_eq!(c.dev_user, Some(DevUser { email: "moi@x.fr".into(), pseudo: "Moi".into(), is_admin: true }));

        let c = Config::from_lookup(lookup(&[
            ("ADMIN_EMAIL", "a@x.fr"),
            ("APP_ENV", "development"),
            ("AUTH_DEV_USER", "pote@x.fr,Pote"),
        ]))
        .unwrap();
        assert!(!c.dev_user.unwrap().is_admin);
    }

    #[test]
    fn port_smtp_invalide() {
        let mut env = PROD.to_vec();
        env.push(("SMTP_PORT", "abc"));
        assert!(matches!(Config::from_lookup(lookup(&env)).unwrap_err(), ConfigError::Invalid("SMTP_PORT", _)));
    }

    #[test]
    fn slash_final_retire_des_urls() {
        let mut env = PROD.to_vec();
        env.push(("PUBLIC_ORIGIN", "http://localhost:5173/"));
        env.push(("AUTH_URL", "http://sso:3000/"));
        let c = Config::from_lookup(lookup(&env)).unwrap();
        assert_eq!(c.public_origin, "http://localhost:5173");
        assert_eq!(c.auth_url, "http://sso:3000");
    }
}
```

`src/lib.rs` :

```rust
pub mod config;
pub mod error;
pub mod routes;

/// Slug de l'application dans le catalogue de limperiam-auth.
pub const APP_SLUG: &str = "santa";
```

- [ ] **Step 3: Lancer les tests, vérifier l'échec**

Run: `cargo test --lib config`
Expected: FAIL à la compilation — `Config`, `ConfigError`, `DevUser` introuvables.

- [ ] **Step 4: Implémenter `config.rs`**

Ajouter en tête de `src/config.rs`, au-dessus du module de test :

```rust
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
    pub from: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevUser {
    pub email: String,
    pub pseudo: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub database_path: String,
    pub uploads_dir: PathBuf,
    pub auth_url: String,
    pub auth_login_url: String,
    pub public_origin: String,
    pub admin_email: String,
    /// `None` seulement en développement : les mails sont alors journalisés.
    pub smtp: Option<SmtpConfig>,
    pub dev_mode: bool,
    pub dev_user: Option<DevUser>,
    /// Pause entre deux mails d'une même vague (1 s, comme le Java).
    pub mail_delay: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("la variable {0} est obligatoire")]
    Missing(&'static str),
    #[error("la variable {0} est invalide : {1}")]
    Invalid(&'static str, String),
    #[error("AUTH_DEV_USER court-circuite le SSO : interdit sans APP_ENV=development")]
    DevUserInProduction,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let var = |k: &str| get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let url = |k: &str, default: &str| var(k).unwrap_or_else(|| default.to_string()).trim_end_matches('/').to_string();

        let dev_mode = var("APP_ENV").as_deref() == Some("development");

        let dev_user = match var("AUTH_DEV_USER") {
            None => None,
            Some(_) if !dev_mode => return Err(ConfigError::DevUserInProduction),
            Some(raw) => Some(parse_dev_user(&raw)?),
        };

        let admin_email = var("ADMIN_EMAIL").ok_or(ConfigError::Missing("ADMIN_EMAIL"))?.to_lowercase();

        let smtp = match var("SMTP_HOST") {
            Some(host) => {
                let port = match var("SMTP_PORT") {
                    None => 587,
                    Some(p) => p.parse().map_err(|_| ConfigError::Invalid("SMTP_PORT", p.clone()))?,
                };
                let from = var("SMTP_FROM").ok_or(ConfigError::Missing("SMTP_FROM"))?;
                Some(SmtpConfig { host, port, user: var("SMTP_USER"), pass: var("SMTP_PASS"), from })
            }
            None if dev_mode => None,
            None => return Err(ConfigError::Missing("SMTP_HOST")),
        };

        let mail_delay_ms: u64 = match var("MAIL_DELAY_MS") {
            None => 1000,
            Some(v) => v.parse().map_err(|_| ConfigError::Invalid("MAIL_DELAY_MS", v.clone()))?,
        };

        Ok(Config {
            bind_addr: var("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into()),
            database_path: var("DATABASE_PATH").unwrap_or_else(|| "/data/santa.db".into()),
            uploads_dir: var("UPLOADS_DIR").unwrap_or_else(|| "/data/uploads".into()).into(),
            auth_url: url("AUTH_URL", "http://limperiam-auth:3000"),
            auth_login_url: url("AUTH_LOGIN_URL", "https://auth.limperiam.com/login"),
            public_origin: url("PUBLIC_ORIGIN", "https://santa.limperiam.com"),
            admin_email,
            smtp,
            dev_mode,
            dev_user,
            mail_delay: Duration::from_millis(mail_delay_ms),
        })
    }
}

fn parse_dev_user(raw: &str) -> Result<DevUser, ConfigError> {
    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    let invalid = || ConfigError::Invalid("AUTH_DEV_USER", "attendu : email,pseudo[,admin]".into());
    let (email, pseudo, is_admin) = match parts.as_slice() {
        [email, pseudo] => (*email, *pseudo, false),
        [email, pseudo, "admin"] => (*email, *pseudo, true),
        _ => return Err(invalid()),
    };
    if !email.contains('@') || pseudo.is_empty() {
        return Err(invalid());
    }
    Ok(DevUser { email: email.to_lowercase(), pseudo: pseudo.to_string(), is_admin })
}
```

- [ ] **Step 5: Implémenter `error.rs`**

```rust
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Connexion requise.")]
    Unauthorized { login_url: String },
    #[error("Accès refusé.")]
    Forbidden,
    #[error("{0}")]
    NotFound(String),
    #[error("{message}")]
    BadRequest { code: &'static str, message: String },
    #[error("{message}")]
    Conflict { code: &'static str, message: String },
    #[error("{message}")]
    Unprocessable { code: &'static str, message: String },
    #[error("Le service d'identité ne répond pas, réessaie dans un instant.")]
    Upstream,
    #[error("Erreur interne.")]
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::BadRequest { code, message: message.into() }
    }
    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::Conflict { code, message: message.into() }
    }
    pub fn unprocessable(code: &'static str, message: impl Into<String>) -> Self {
        Self::Unprocessable { code, message: message.into() }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::Unauthorized { .. } => (StatusCode::UNAUTHORIZED, "NOT_AUTHENTICATED"),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::BadRequest { code, .. } => (StatusCode::BAD_REQUEST, *code),
            ApiError::Conflict { code, .. } => (StatusCode::CONFLICT, *code),
            ApiError::Unprocessable { code, .. } => (StatusCode::UNPROCESSABLE_ENTITY, *code),
            ApiError::Upstream => (StatusCode::BAD_GATEWAY, "UPSTREAM"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
        };
        if let ApiError::Internal(detail) = &self {
            tracing::error!(%detail, "erreur interne");
        }
        let mut body = json!({ "error": code, "message": self.to_string() });
        if let ApiError::Unauthorized { login_url } = &self {
            body["loginUrl"] = json!(login_url);
        }
        (status, Json(body)).into_response()
    }
}
```

- [ ] **Step 6: Routeur minimal, `main.rs` et test de santé**

`src/routes/mod.rs` :

```rust
use axum::{Router, routing::get};

pub async fn health() -> &'static str {
    "ok"
}

/// Remplacé en Task 7 par `router(state)`.
pub fn router() -> Router {
    Router::new().nest("/api", Router::new().route("/health", get(health)))
}
```

`src/main.rs` :

```rust
use secret_santa::{config::Config, routes};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration invalide : {e}");
            std::process::exit(1);
        }
    };

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await.expect("impossible d'ouvrir le port");
    tracing::info!(addr = %config.bind_addr, "Secret Santa à l'écoute");
    axum::serve(listener, routes::router()).await.expect("serveur arrêté sur une erreur");
}
```

`tests/health.rs` :

```rust
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
```

- [ ] **Step 7: Lancer les tests**

Run: `cargo test`
Expected: PASS (9 tests de config + 1 de santé).

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore src tests
git commit -m "Squelette du backend : configuration, erreurs, /api/health

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Horloge, base SQLite et migration

**Files:**
- Create: `src/clock.rs`, `src/db.rs`, `migrations/0001_init.sql`, `tests/db.rs`
- Modify: `src/lib.rs` (ajouter `pub mod clock; pub mod db;`)

**Interfaces:**
- Produces: `trait Clock: Send + Sync { fn now(&self) -> DateTime<Utc>; }`, `SystemClock`, `ManualClock::new(DateTime<Utc>)`, `.set(DateTime<Utc>)`, `.advance(chrono::Duration)`, `paris_year(DateTime<Utc>) -> i64` ; `db::connect(&str) -> Result<SqlitePool, sqlx::Error>`, `db::connect_in_memory() -> Result<SqlitePool, sqlx::Error>`.

- [ ] **Step 1: Écrire les tests**

`tests/db.rs` :

```rust
use chrono::{TimeZone, Utc};
use secret_santa::clock::{Clock, ManualClock, paris_year};
use secret_santa::db;

#[tokio::test]
async fn la_migration_cree_toutes_les_tables() {
    let pool = db::connect_in_memory().await.unwrap();
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE '\\_%' ESCAPE '\\' AND name NOT LIKE 'sqlite%' ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(tables, ["assignments", "editions", "gifts", "players", "registrations"]);
}

#[tokio::test]
async fn les_cles_etrangeres_sont_actives() {
    let pool = db::connect_in_memory().await.unwrap();
    let res = sqlx::query("INSERT INTO registrations (edition_id, player_id, participating, updated_at) VALUES (99, 99, 1, '2026-01-01T00:00:00Z')")
        .execute(&pool)
        .await;
    assert!(res.is_err(), "une inscription vers une édition inexistante doit être refusée");
}

#[tokio::test]
async fn connect_cree_le_fichier_et_son_dossier() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sous/dossier/santa.db");
    let pool = db::connect(path.to_str().unwrap()).await.unwrap();
    pool.close().await;
    assert!(path.exists());
}

#[test]
fn horloge_manuelle_et_annee_de_paris() {
    let clock = ManualClock::new(Utc.with_ymd_and_hms(2026, 12, 31, 22, 30, 0).unwrap());
    // 23h30 à Paris : encore 2026 ; une heure plus tard, 2027.
    assert_eq!(paris_year(clock.now()), 2026);
    clock.advance(chrono::Duration::hours(1));
    assert_eq!(paris_year(clock.now()), 2027);
    clock.set(Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap());
    assert_eq!(paris_year(clock.now()), 2025);
}
```

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --test db`
Expected: FAIL à la compilation — modules `clock` et `db` absents.

- [ ] **Step 3: Écrire la migration**

`migrations/0001_init.sql` :

```sql
CREATE TABLE players (
    id            INTEGER PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    sso_user_id   INTEGER UNIQUE,
    pseudo        TEXT NOT NULL,
    has_access    INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL
);

CREATE TABLE editions (
    id               INTEGER PRIMARY KEY,
    year             INTEGER NOT NULL UNIQUE,
    budget_eur       INTEGER NOT NULL CHECK (budget_eur > 0),
    deadline         TEXT NOT NULL,
    status           TEXT NOT NULL CHECK (status IN ('open', 'drawn', 'failed')),
    failure_reason   TEXT,
    drawn_at         TEXT,
    reminder_sent_at TEXT,
    announced_at     TEXT,
    imported         INTEGER NOT NULL DEFAULT 0,
    created_at       TEXT NOT NULL
);

CREATE TABLE registrations (
    edition_id        INTEGER NOT NULL REFERENCES editions (id) ON DELETE CASCADE,
    player_id         INTEGER NOT NULL REFERENCES players (id),
    participating     INTEGER NOT NULL,
    answered_by_admin INTEGER NOT NULL DEFAULT 0,
    updated_at        TEXT NOT NULL,
    PRIMARY KEY (edition_id, player_id)
);

CREATE TABLE assignments (
    edition_id  INTEGER NOT NULL REFERENCES editions (id) ON DELETE CASCADE,
    giver_id    INTEGER NOT NULL REFERENCES players (id),
    receiver_id INTEGER NOT NULL REFERENCES players (id),
    mail_status TEXT NOT NULL CHECK (mail_status IN ('pending', 'sent', 'failed', 'none')),
    mail_error  TEXT,
    PRIMARY KEY (edition_id, giver_id),
    UNIQUE (edition_id, receiver_id)
);

CREATE TABLE gifts (
    id          INTEGER PRIMARY KEY,
    player_id   INTEGER NOT NULL REFERENCES players (id),
    year        INTEGER NOT NULL,
    description TEXT NOT NULL,
    photo_path  TEXT,
    created_at  TEXT NOT NULL
);

CREATE INDEX idx_gifts_player ON gifts (player_id, year DESC);
CREATE INDEX idx_assignments_pending ON assignments (mail_status) WHERE mail_status = 'pending';
```

- [ ] **Step 4: Implémenter `clock.rs` et `db.rs`**

`src/clock.rs` :

```rust
use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Europe::Paris;
use std::sync::Mutex;

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Horloge pilotée à la main : tests et scénarios de deadline.
pub struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    pub fn new(at: DateTime<Utc>) -> Self {
        Self(Mutex::new(at))
    }
    pub fn set(&self, at: DateTime<Utc>) {
        *self.0.lock().unwrap() = at;
    }
    pub fn advance(&self, by: chrono::Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

/// L'année d'une édition est l'année civile à Paris, pas en UTC : un tirage
/// le 31 décembre à 23h30 appartient encore à l'année qui se termine.
pub fn paris_year(at: DateTime<Utc>) -> i64 {
    i64::from(at.with_timezone(&Paris).year())
}
```

`src/db.rs` :

```rust
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::{path::Path, str::FromStr, time::Duration};

pub async fn connect(path: &str) -> Result<SqlitePool, sqlx::Error> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(sqlx::Error::Io)?;
        }
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new().max_connections(5).connect_with(options).await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// Base de test. Une seule connexion, gardée ouverte : chaque connexion à
/// `:memory:` aurait sinon sa propre base, vide.
pub async fn connect_in_memory() -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| sqlx::Error::Migrate(Box::new(e)))
}
```

`src/lib.rs` : ajouter `pub mod clock;` et `pub mod db;`.

- [ ] **Step 5: Lancer les tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add migrations src tests
git commit -m "Base SQLite, migration initiale et horloge injectable

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Algorithme de tirage

**Files:**
- Create: `src/draw.rs`
- Modify: `src/lib.rs` (`pub mod draw;`)

**Interfaces:**
- Produces: `type PlayerId = i64`, `MIN_PARTICIPANTS = 3`, `MAX_ATTEMPTS = 1000`, `enum DrawError { NotEnoughParticipants(usize), NoValidDraw(usize) }` (Display en français), `fn draw<R: Rng + ?Sized>(participants: &[PlayerId], previous: &HashMap<PlayerId, PlayerId>, rng: &mut R) -> Result<Vec<(PlayerId, PlayerId)>, DrawError>` — paires `(donneur, receveur)`.

- [ ] **Step 1: Écrire les tests**

`src/draw.rs` (tests seuls pour commencer) :

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    use std::collections::{HashMap, HashSet};

    fn rng(seed: u64) -> StdRng {
        StdRng::seed_from_u64(seed)
    }

    #[test]
    fn refuse_moins_de_trois_participants() {
        assert_eq!(draw(&[1, 2], &HashMap::new(), &mut rng(1)), Err(DrawError::NotEnoughParticipants(2)));
        assert_eq!(draw(&[], &HashMap::new(), &mut rng(1)), Err(DrawError::NotEnoughParticipants(0)));
    }

    #[test]
    fn chacun_donne_et_recoit_exactement_une_fois_jamais_a_soi_meme() {
        let ids = [10, 20, 30, 40, 50, 60];
        for seed in 0..300 {
            let pairs = draw(&ids, &HashMap::new(), &mut rng(seed)).unwrap();
            assert_eq!(pairs.len(), ids.len());
            let givers: HashSet<_> = pairs.iter().map(|(g, _)| *g).collect();
            let receivers: HashSet<_> = pairs.iter().map(|(_, r)| *r).collect();
            assert_eq!(givers, ids.iter().copied().collect());
            assert_eq!(receivers, ids.iter().copied().collect());
            assert!(pairs.iter().all(|(g, r)| g != r));
        }
    }

    #[test]
    fn forme_un_seul_cycle() {
        let ids = [1, 2, 3, 4, 5];
        for seed in 0..100 {
            let pairs: HashMap<_, _> = draw(&ids, &HashMap::new(), &mut rng(seed)).unwrap().into_iter().collect();
            let mut current = ids[0];
            let mut visited = HashSet::new();
            for _ in 0..ids.len() {
                assert!(visited.insert(current), "cycle trop court");
                current = pairs[&current];
            }
            assert_eq!(current, ids[0]);
        }
    }

    #[test]
    fn respecte_la_cible_de_l_edition_precedente() {
        let ids = [1, 2, 3, 4];
        let previous = HashMap::from([(1, 2), (2, 3), (3, 4), (4, 1)]);
        for seed in 0..300 {
            for (g, r) in draw(&ids, &previous, &mut rng(seed)).unwrap() {
                assert_ne!(previous.get(&g), Some(&r));
            }
        }
    }

    #[test]
    fn signale_un_tirage_impossible() {
        // À trois, seuls deux cycles existent : 1→2→3→1 et 1→3→2→1.
        // 1→2 interdit le premier, 2→1 interdit le second.
        let previous = HashMap::from([(1, 2), (2, 1)]);
        assert_eq!(draw(&[1, 2, 3], &previous, &mut rng(7)), Err(DrawError::NoValidDraw(MAX_ATTEMPTS)));
    }

    #[test]
    fn ignore_une_cible_precedente_qui_ne_participe_plus() {
        let previous = HashMap::from([(1, 99)]);
        assert!(draw(&[1, 2, 3], &previous, &mut rng(3)).is_ok());
    }

    #[test]
    fn messages_en_francais() {
        assert_eq!(DrawError::NotEnoughParticipants(2).to_string(), "Il faut au moins 3 participants (inscrits : 2).");
        assert!(DrawError::NoValidDraw(1000).to_string().contains("1000 essais"));
    }
}
```

`src/lib.rs` : ajouter `pub mod draw;`.

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --lib draw`
Expected: FAIL à la compilation — `draw`, `DrawError`, `MAX_ATTEMPTS` absents.

- [ ] **Step 3: Implémenter**

En tête de `src/draw.rs` :

```rust
//! Tirage pur : aucune I/O, le hasard est injecté pour que les tests soient
//! reproductibles. Même algorithme que la version Java : on mélange, on forme
//! un cycle unique (chacun offre au suivant, le dernier au premier), et on
//! recommence si quelqu'un retombe sur sa cible de l'édition précédente.

use rand::{Rng, seq::SliceRandom};
use std::collections::HashMap;

pub type PlayerId = i64;

pub const MIN_PARTICIPANTS: usize = 3;
pub const MAX_ATTEMPTS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DrawError {
    #[error("Il faut au moins {MIN_PARTICIPANTS} participants (inscrits : {0}).")]
    NotEnoughParticipants(usize),
    #[error("Impossible de trouver un tirage valide après {0} essais : la contrainte « pas la même cible que l'an dernier » bloque.")]
    NoValidDraw(usize),
}

pub fn draw<R: Rng + ?Sized>(
    participants: &[PlayerId],
    previous: &HashMap<PlayerId, PlayerId>,
    rng: &mut R,
) -> Result<Vec<(PlayerId, PlayerId)>, DrawError> {
    if participants.len() < MIN_PARTICIPANTS {
        return Err(DrawError::NotEnoughParticipants(participants.len()));
    }
    let mut order = participants.to_vec();
    for _ in 0..MAX_ATTEMPTS {
        order.shuffle(rng);
        let pairs: Vec<_> = (0..order.len()).map(|i| (order[i], order[(i + 1) % order.len()])).collect();
        if pairs.iter().all(|(giver, receiver)| previous.get(giver) != Some(receiver)) {
            return Ok(pairs);
        }
    }
    Err(DrawError::NoValidDraw(MAX_ATTEMPTS))
}
```

- [ ] **Step 4: Lancer les tests**

Run: `cargo test --lib draw`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
git add src
git commit -m "Algorithme de tirage avec contrainte de l'édition précédente

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Parseur de `players.txt`

**Files:**
- Create: `src/legacy_import.rs`
- Modify: `src/lib.rs` (`pub mod legacy_import;`)

**Interfaces:**
- Produces: `struct LegacyPair { giver_name, giver_email, receiver_name, receiver_email: String }` ; `#[derive(Serialize)] struct SkippedLine { line: usize, reason: String }` ; `struct ParsedLegacy { pairs: Vec<LegacyPair>, skipped: Vec<SkippedLine> }` ; `fn parse(content: &str) -> ParsedLegacy`. Emails en minuscules.

- [ ] **Step 1: Écrire les tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn pair(g: &str, ge: &str, r: &str, re: &str) -> LegacyPair {
        LegacyPair { giver_name: g.into(), giver_email: ge.into(), receiver_name: r.into(), receiver_email: re.into() }
    }

    #[test]
    fn fichier_nominal() {
        let parsed = parse("Alice;alice@x.fr;Bob\nBob;bob@x.fr;Chloe\nChloe;chloe@x.fr;Alice\n");
        assert!(parsed.skipped.is_empty());
        assert_eq!(parsed.pairs, vec![
            pair("Alice", "alice@x.fr", "Bob", "bob@x.fr"),
            pair("Bob", "bob@x.fr", "Chloe", "chloe@x.fr"),
            pair("Chloe", "chloe@x.fr", "Alice", "alice@x.fr"),
        ]);
    }

    #[test]
    fn tolere_bom_crlf_espaces_casse_et_lignes_vides() {
        let parsed = parse("\u{feff}Alice ; Alice@X.fr ; bob\r\n\r\nBob;bob@x.fr;ALICE\r\n");
        assert!(parsed.skipped.is_empty());
        assert_eq!(parsed.pairs, vec![
            pair("Alice", "alice@x.fr", "Bob", "bob@x.fr"),
            pair("Bob", "bob@x.fr", "Alice", "alice@x.fr"),
        ]);
    }

    #[test]
    fn ignore_les_lignes_invalides_avec_leur_raison() {
        let parsed = parse(concat!(
            "Alice;alice@x.fr;Bob\n",      // 1 ok
            "n'importe quoi\n",            // 2 mal formée
            "Bob;bob@x.fr\n",              // 3 pas de cible
            "Chloe;chloe@x.fr;Zoé\n",      // 4 cible introuvable
            "Dan;dan@x.fr;Bob\n",          // 5 Bob déjà ciblé par Alice
            "Alice2;ALICE@x.fr;Chloe\n",   // 6 adresse en double
            "Eve;eve@x.fr;Eve\n",          // 7 se cible soi-même
        ));
        assert_eq!(parsed.pairs, vec![pair("Alice", "alice@x.fr", "Bob", "bob@x.fr")]);
        let lines: Vec<_> = parsed.skipped.iter().map(|s| s.line).collect();
        assert_eq!(lines, vec![2, 3, 4, 5, 6, 7]);
        assert!(parsed.skipped[0].reason.contains("mal formée"));
        assert!(parsed.skipped[1].reason.contains("pas de cible"));
        assert!(parsed.skipped[2].reason.contains("introuvable"));
        assert!(parsed.skipped[3].reason.contains("déjà"));
        assert!(parsed.skipped[4].reason.contains("en double"));
        assert!(parsed.skipped[5].reason.contains("soi-même"));
    }

    #[test]
    fn nom_ambigu() {
        let parsed = parse("Max;max1@x.fr;Léa\nMax;max2@x.fr;Léa\nLéa;lea@x.fr;Max\n");
        let lea = parsed.skipped.iter().find(|s| s.line == 3).unwrap();
        assert!(lea.reason.contains("ambiguë"));
    }

    #[test]
    fn fichier_vide() {
        assert_eq!(parse(""), ParsedLegacy::default());
    }
}
```

`src/lib.rs` : ajouter `pub mod legacy_import;`.

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --lib legacy_import`
Expected: FAIL à la compilation.

- [ ] **Step 3: Implémenter**

```rust
//! Lecture du `players.txt` de la version Java : `nom;email;cible`, où la
//! cible est le NOM d'un autre joueur du même fichier. Après un tirage Java,
//! ce fichier contient exactement les paires de ce tirage.

use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPair {
    pub giver_name: String,
    pub giver_email: String,
    pub receiver_name: String,
    pub receiver_email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedLine {
    pub line: usize,
    pub reason: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedLegacy {
    pub pairs: Vec<LegacyPair>,
    pub skipped: Vec<SkippedLine>,
}

struct Entry {
    line: usize,
    name: String,
    email: String,
    target: Option<String>,
}

pub fn parse(content: &str) -> ParsedLegacy {
    let mut out = ParsedLegacy::default();
    let mut skip = |line: usize, reason: String| out.skipped.push(SkippedLine { line, reason });

    let mut entries = Vec::new();
    let mut emails = HashSet::new();
    for (index, raw) in content.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = index + 1;
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let parts: Vec<&str> = raw.split(';').map(str::trim).collect();
        if parts.len() < 2 || parts[0].is_empty() || !parts[1].contains('@') {
            skip(line, "ligne mal formée (attendu : nom;email;cible)".into());
            continue;
        }
        let email = parts[1].to_lowercase();
        if !emails.insert(email.clone()) {
            skip(line, format!("adresse {email} en double"));
            continue;
        }
        let target = parts.get(2).map(|t| t.to_string()).filter(|t| !t.is_empty());
        entries.push(Entry { line, name: parts[0].to_string(), email, target });
    }

    // Nom en minuscules → index de l'entrée ; None si deux joueurs portent ce nom.
    let mut by_name: HashMap<String, Option<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        by_name.entry(e.name.to_lowercase()).and_modify(|v| *v = None).or_insert(Some(i));
    }

    let mut taken = HashSet::new();
    let mut pairs = Vec::new();
    for e in &entries {
        let Some(target) = &e.target else {
            skip(e.line, "pas de cible".into());
            continue;
        };
        match by_name.get(&target.to_lowercase()) {
            None => skip(e.line, format!("cible « {target} » introuvable dans le fichier")),
            Some(None) => skip(e.line, format!("cible « {target} » ambiguë (nom en double)")),
            Some(Some(i)) => {
                let receiver = &entries[*i];
                if receiver.email == e.email {
                    skip(e.line, "se cible soi-même".into());
                } else if !taken.insert(receiver.email.clone()) {
                    skip(e.line, format!("cible « {} » déjà attribuée", receiver.name));
                } else {
                    pairs.push(LegacyPair {
                        giver_name: e.name.clone(),
                        giver_email: e.email.clone(),
                        receiver_name: receiver.name.clone(),
                        receiver_email: receiver.email.clone(),
                    });
                }
            }
        }
    }
    out.pairs = pairs;
    out.skipped.sort_by_key(|s| s.line);
    out
}
```

Note : `skip` emprunte `out` mutablement ; si le compilateur refuse l'affectation `out.pairs = pairs` tant que la fermeture vit, remplacer la fermeture par une fonction `fn skip(v: &mut Vec<SkippedLine>, line: usize, reason: String)` appelée avec `&mut out.skipped`.

- [ ] **Step 4: Lancer les tests**

Run: `cargo test --lib legacy_import`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src
git commit -m "Parseur du players.txt de la version Java

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Client SSO (HTTP avec cache, et mode dev)

**Files:**
- Create: `src/auth.rs`, `tests/auth_client.rs`
- Modify: `src/lib.rs` (`pub mod auth;`)

**Interfaces:**
- Consumes: `config::DevUser`, `APP_SLUG`.
- Produces: `SESSION_COOKIE = "lim_session"` ; `SsoSession { user_id: i64, email, pseudo: String, is_admin: bool, apps: Vec<String> }` ; `SsoUser { user_id: i64, email, pseudo: String }` ; `enum AuthError { Unavailable(String), Forbidden }` ; `#[async_trait] trait AuthClient: Send + Sync { async fn session(&self, token: &str) -> Result<Option<SsoSession>, AuthError>; async fn list_users(&self, token: &str, app: &str) -> Result<Vec<SsoUser>, AuthError>; }` ; `HttpAuthClient::new(base_url: impl Into<String>, ttl: Duration)` ; `DevAuthClient::new(DevUser)`.

- [ ] **Step 1: Écrire les tests**

`tests/auth_client.rs` :

```rust
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
    assert_eq!((s.user_id, s.pseudo.as_str(), s.is_admin), (7, "Alice", false));
    assert_eq!(s.apps, ["santa", "dashboard"]);
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
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "error": "NOT_AUTHENTICATED" })))
        .mount(&server)
        .await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    assert_eq!(client.session("tok").await, Ok(None));
}

#[tokio::test]
async fn erreur_serveur_ou_reponse_illisible_donne_unavailable() {
    let server = MockServer::start().await;
    Mock::given(path("/api/session")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    assert!(matches!(client.session("a").await, Err(AuthError::Unavailable(_))));

    let server = MockServer::start().await;
    Mock::given(path("/api/session")).respond_with(ResponseTemplate::new(200).set_body_string("pas du json")).mount(&server).await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    assert!(matches!(client.session("a").await, Err(AuthError::Unavailable(_))));
}

#[tokio::test]
async fn sso_injoignable_donne_unavailable() {
    let client = HttpAuthClient::new("http://127.0.0.1:1", Duration::from_secs(60));
    assert!(matches!(client.session("a").await, Err(AuthError::Unavailable(_))));
}

#[tokio::test]
async fn list_users_filtre_par_app_et_relaie_le_cookie() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/users"))
        .and(query_param("app", "santa"))
        .and(header("cookie", "lim_session=admin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "users": [
            { "userId": 1, "email": "a@x.fr", "pseudo": "A" },
            { "userId": 2, "email": "b@x.fr", "pseudo": "B" }
        ] })))
        .mount(&server)
        .await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    let users = client.list_users("admin", "santa").await.unwrap();
    assert_eq!(users.iter().map(|u| u.user_id).collect::<Vec<_>>(), [1, 2]);
}

#[tokio::test]
async fn list_users_refuse_un_non_admin() {
    let server = MockServer::start().await;
    Mock::given(path("/api/admin/users")).respond_with(ResponseTemplate::new(403)).mount(&server).await;
    let client = HttpAuthClient::new(server.uri(), Duration::from_secs(60));
    assert_eq!(client.list_users("pote", "santa").await, Err(AuthError::Forbidden));
}

#[tokio::test]
async fn client_de_dev() {
    let client = DevAuthClient::new(DevUser { email: "moi@x.fr".into(), pseudo: "Moi".into(), is_admin: true });
    let s = client.session("peu importe").await.unwrap().unwrap();
    assert!(s.is_admin);
    assert_eq!(s.apps, ["santa"]);
    assert_eq!(client.list_users("x", "santa").await.unwrap().len(), 1);
}
```

`src/lib.rs` : ajouter `pub mod auth;`.

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --test auth_client`
Expected: FAIL à la compilation.

- [ ] **Step 3: Implémenter `src/auth.rs`**

```rust
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
```

- [ ] **Step 4: Lancer les tests**

Run: `cargo test --test auth_client`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add src tests
git commit -m "Client du SSO limperiam-auth avec cache et mode développement

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Mails (trait, SMTP, journal, gabarits)

**Files:**
- Create: `src/mail/mod.rs`, `src/mail/templates.rs`
- Modify: `src/lib.rs` (`pub mod mail;`)

**Interfaces:**
- Consumes: `config::SmtpConfig`.
- Produces: `Mail { to, subject, text, html: String }` ; `MailError(String)` ; `#[async_trait] trait Mailer: Send + Sync { async fn send(&self, mail: &Mail) -> Result<(), MailError>; }` ; `SmtpMailer::new(&SmtpConfig) -> Result<SmtpMailer, MailError>` ; `LogMailer` ; dans `templates` : `paris(DateTime<Utc>) -> String`, `escape(&str) -> String`, `announcement(to, pseudo, year: i64, budget: i64, deadline, origin) -> Mail`, `reminder(to, pseudo, year, deadline, origin) -> Mail`, `result(to, giver_pseudo, receiver_pseudo, receiver_id: i64, year, budget, origin) -> Mail`, `draw_failed(to, year, reason, origin) -> Mail`. Liens du front : `/` (accueil), `/joueurs/{id}` (profil), `/admin`.

- [ ] **Step 1: Écrire les tests des gabarits**

`src/mail/templates.rs` (tests seuls pour commencer) :

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn deadline() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, 1, 22, 30, 0).unwrap()
    }

    #[test]
    fn date_en_heure_de_paris() {
        assert_eq!(paris(deadline()), "01/12/2026 à 23h30");
    }

    #[test]
    fn mail_de_resultat() {
        let m = result("a@x.fr", "Alice", "Bob", 42, 2026, 30, "https://santa.test");
        assert_eq!(m.to, "a@x.fr");
        assert!(m.subject.contains("2026"));
        assert!(m.text.contains("tu offres un cadeau à : Bob"));
        assert!(m.text.contains("30 €"));
        assert!(m.text.contains("https://santa.test/joueurs/42"));
        assert!(m.html.contains("https://santa.test/joueurs/42"));
    }

    #[test]
    fn le_html_echappe_les_pseudos() {
        let m = result("a@x.fr", "<b>Alice</b>", "Bob & Co", 1, 2026, 30, "https://santa.test");
        assert!(m.html.contains("&lt;b&gt;Alice&lt;/b&gt;"));
        assert!(m.html.contains("Bob &amp; Co"));
        assert!(!m.html.contains("<b>Alice"));
    }

    #[test]
    fn annonce_relance_et_echec() {
        let a = announcement("a@x.fr", "Alice", 2026, 25, deadline(), "https://santa.test");
        assert!(a.text.contains("25 €") && a.text.contains("01/12/2026 à 23h30"));
        let r = reminder("a@x.fr", "Alice", 2026, deadline(), "https://santa.test");
        assert!(r.subject.contains("48 h"));
        let f = draw_failed("admin@x.fr", 2026, "Il faut au moins 3 participants (inscrits : 2).", "https://santa.test");
        assert!(f.text.contains("inscrits : 2") && f.text.contains("https://santa.test/admin"));
    }
}
```

`src/mail/mod.rs` : pour l'instant seulement `pub mod templates;` plus la définition de `Mail` (Step 3). `src/lib.rs` : ajouter `pub mod mail;`.

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --lib mail`
Expected: FAIL à la compilation.

- [ ] **Step 3: Implémenter `src/mail/mod.rs`**

```rust
pub mod templates;

use crate::config::SmtpConfig;
use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart},
    transport::smtp::authentication::Credentials,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct MailError(pub String);

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, mail: &Mail) -> Result<(), MailError>;
}

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    /// Port 465 : TLS implicite ; tout autre port : STARTTLS obligatoire
    /// (même règle que limperiam-auth).
    pub fn new(cfg: &SmtpConfig) -> Result<Self, MailError> {
        let err = |e: &dyn std::fmt::Display| MailError(e.to_string());
        let builder = if cfg.port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
        }
        .map_err(|e| err(&e))?
        .port(cfg.port);
        let builder = match (&cfg.user, &cfg.pass) {
            (Some(user), Some(pass)) => builder.credentials(Credentials::new(user.clone(), pass.clone())),
            _ => builder,
        };
        let from = cfg.from.parse::<Mailbox>().map_err(|e| err(&e))?;
        Ok(Self { transport: builder.build(), from })
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, mail: &Mail) -> Result<(), MailError> {
        let to = mail.to.parse::<Mailbox>().map_err(|e| MailError(e.to_string()))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(&mail.subject)
            .multipart(MultiPart::alternative_plain_html(mail.text.clone(), mail.html.clone()))
            .map_err(|e| MailError(e.to_string()))?;
        self.transport.send(message).await.map(|_| ()).map_err(|e| MailError(e.to_string()))
    }
}

/// Développement : le mail est écrit dans les logs au lieu de partir.
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, mail: &Mail) -> Result<(), MailError> {
        tracing::info!(to = %mail.to, subject = %mail.subject, "mail (dev, non envoyé)\n{}", mail.text);
        Ok(())
    }
}
```

- [ ] **Step 4: Implémenter `src/mail/templates.rs`**

Au-dessus du module de test :

```rust
use super::Mail;
use chrono::{DateTime, Utc};
use chrono_tz::Europe::Paris;

pub fn paris(at: DateTime<Utc>) -> String {
    at.with_timezone(&Paris).format("%d/%m/%Y à %Hh%M").to_string()
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn build(to: &str, subject: String, lines: &[String], link_label: &str, link: String) -> Mail {
    let text = format!("{}\n\n{link_label} : {link}\n", lines.join("\n\n"));
    let paragraphs: String = lines
        .iter()
        .map(|l| format!(r#"<p style="margin:0 0 16px">{}</p>"#, escape(l)))
        .collect();
    let html = format!(
        r#"<!doctype html><html lang="fr"><body style="margin:0;background:#0b1220;padding:32px 16px;font-family:Georgia,'Times New Roman',serif"><div style="max-width:520px;margin:0 auto;background:#fffaf0;border-radius:18px;padding:32px;color:#1d2433;font-size:16px;line-height:1.55"><p style="margin:0 0 20px;font-size:30px">🎄</p>{paragraphs}<p style="margin:28px 0 0"><a href="{href}" style="display:inline-block;background:#b3261e;color:#fffaf0;text-decoration:none;padding:12px 22px;border-radius:999px;font-family:Arial,sans-serif">{label}</a></p></div></body></html>"#,
        href = escape(&link),
        label = escape(link_label),
    );
    Mail { to: to.to_string(), subject, text, html }
}

pub fn announcement(to: &str, pseudo: &str, year: i64, budget: i64, deadline: DateTime<Utc>, origin: &str) -> Mail {
    build(
        to,
        format!("🎄 Secret Santa {year} : les inscriptions sont ouvertes"),
        &[
            format!("Bonjour {pseudo},"),
            format!("Le Secret Santa {year} est lancé ! Budget : {budget} € maximum."),
            format!("Dis-nous si tu participes avant le {}. Sans réponse, tu ne participes pas.", paris(deadline)),
            "Joyeuses fêtes ! 🎅".into(),
        ],
        "Je réponds",
        format!("{origin}/"),
    )
}

pub fn reminder(to: &str, pseudo: &str, year: i64, deadline: DateTime<Utc>, origin: &str) -> Mail {
    build(
        to,
        format!("⏰ Secret Santa {year} : plus que 48 h pour t'inscrire"),
        &[
            format!("Bonjour {pseudo},"),
            format!("Tu n'as pas encore dit si tu participes au Secret Santa {year}."),
            format!("Les inscriptions ferment le {}. Sans réponse, tu ne participes pas.", paris(deadline)),
        ],
        "Je réponds",
        format!("{origin}/"),
    )
}

pub fn result(to: &str, giver_pseudo: &str, receiver_pseudo: &str, receiver_id: i64, year: i64, budget: i64, origin: &str) -> Mail {
    build(
        to,
        format!("🎁 Secret Santa {year} : ton tirage !"),
        &[
            format!("Bonjour {giver_pseudo},"),
            format!("Pour ce Secret Santa, tu offres un cadeau à : {receiver_pseudo} !"),
            format!("Pour rappel, le budget est de {budget} € maximum."),
            "Joyeuses fêtes ! 🎅".into(),
        ],
        "Voir son profil",
        format!("{origin}/joueurs/{receiver_id}"),
    )
}

pub fn draw_failed(to: &str, year: i64, reason: &str, origin: &str) -> Mail {
    build(
        to,
        format!("⚠️ Secret Santa {year} : le tirage a échoué"),
        &[
            "Le tirage automatique n'a pas pu se faire.".into(),
            format!("Raison : {reason}"),
            "Repousse la deadline ou ajuste les inscriptions, puis relance le tirage.".into(),
        ],
        "Ouvrir l'admin",
        format!("{origin}/admin"),
    )
}
```

- [ ] **Step 5: Lancer les tests**

Run: `cargo test --lib mail`
Expected: PASS (4 tests). `cargo build` doit aussi passer (vérifie `SmtpMailer`).

- [ ] **Step 6: Commit**

```bash
git add src
git commit -m "Mails : SMTP, journal de dev et gabarits en français

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: État, authentification des requêtes, `/api/me` et banc de test

**Files:**
- Create: `src/state.rs`, `src/repo/mod.rs`, `src/repo/players.rs`, `src/routes/extract.rs`, `src/routes/me.rs`, `tests/common/mod.rs`, `tests/me.rs`
- Modify: `src/lib.rs` (`pub mod state; pub mod repo;`), `src/routes/mod.rs` (routeur avec état), `src/main.rs` (temporairement : construire l'état), `tests/health.rs`

**Interfaces:**
- Consumes: `Config`, `AuthClient`, `SsoSession`, `SsoUser`, `SESSION_COOKIE`, `Mailer`, `Clock`, `paris_year`, `db::connect_in_memory`, `ApiError`.
- Produces:
  - `AppState { pool: SqlitePool, config: Arc<Config>, auth: Arc<dyn AuthClient>, mailer: Arc<dyn Mailer>, clock: Arc<dyn Clock>, tasks: TaskTracker }` (Clone) avec `now()`, `current_year() -> i64`, `spawn(fut)`.
  - `repo::players` : `Player { id, email, sso_user_id: Option<i64>, pseudo, has_access: bool }`, `PlayerRef { player_id: i64, pseudo: String }` (Serialize camelCase), `upsert_sso(&mut SqliteConnection, sso_user_id, email, pseudo, now) -> Result<i64>`, `sync_access(&SqlitePool, &[SsoUser], now)`, `find(&SqlitePool, id) -> Option<Player>`, `with_access(&SqlitePool) -> Vec<Player>`, `upsert_imported(&mut SqliteConnection, email, pseudo, now) -> Result<i64>`, `list_visible(&SqlitePool) -> Vec<PlayerRef>`.
  - `routes::extract` : `CurrentUser { player_id, email, pseudo, is_admin, token }`, `AdminUser(pub CurrentUser)`, `cookie_value(&HeaderMap, &str) -> Option<String>`.
  - `routes::router(state: AppState) -> Router`.
  - Banc de test `tests/common` : `TestApp::new()`, `.login(sso_id, pseudo, admin) -> String` (jeton), `.login_without_app(...)`, `.add_sso_user(sso_id, pseudo)`, `.get/.post/.put/.patch/.delete(path, token, body) -> (StatusCode, Value)`, `.send(Request) -> (StatusCode, Value)`, `.response(Request) -> Response`, `.multipart(method, path, token, parts)`, `.settle()`, `.player_id(pseudo) -> i64`, `.mails_to(email) -> Vec<Mail>`, `ORIGIN`, `Part`.

- [ ] **Step 1: Écrire `src/state.rs`, `src/repo/players.rs` (signatures) et le banc de test**

`src/state.rs` :

```rust
use crate::{auth::AuthClient, clock::{Clock, paris_year}, config::Config, mail::Mailer};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::{future::Future, sync::Arc};
use tokio_util::task::TaskTracker;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    pub auth: Arc<dyn AuthClient>,
    pub mailer: Arc<dyn Mailer>,
    pub clock: Arc<dyn Clock>,
    /// Tâches de fond (envois de mails) : attendues à l'arrêt, et par les tests.
    pub tasks: TaskTracker,
}

impl AppState {
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    pub fn current_year(&self) -> i64 {
        paris_year(self.now())
    }

    pub fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tasks.spawn(fut);
    }
}
```

`src/repo/mod.rs` :

```rust
pub mod players;
```

`src/lib.rs` : ajouter `pub mod repo;` et `pub mod state;`.

`tests/common/mod.rs` :

```rust
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
        if self.down.load(Ordering::SeqCst) {
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
    File { filename: String, content_type: String, bytes: Vec<u8> },
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
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            bind_addr: "127.0.0.1:0".into(),
            database_path: ":memory:".into(),
            uploads_dir: tmp.path().join("uploads"),
            auth_url: "http://sso.test".into(),
            auth_login_url: "https://auth.test/login".into(),
            public_origin: ORIGIN.into(),
            admin_email: "admin@x.fr".into(),
            smtp: None,
            dev_mode: false,
            dev_user: None,
            mail_delay: Duration::ZERO,
        };
        let auth = Arc::new(FakeAuth::default());
        let mailer = Arc::new(RecordingMailer::default());
        let clock = Arc::new(ManualClock::new(Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap()));
        let state = AppState {
            pool: db::connect_in_memory().await.unwrap(),
            config: Arc::new(config),
            auth: auth.clone(),
            mailer: mailer.clone(),
            clock: clock.clone(),
            tasks: TaskTracker::new(),
        };
        let router = routes::router(state.clone());
        Self { state, router, auth, mailer, clock, tmp }
    }

    fn email_of(pseudo: &str) -> String {
        format!("{}@x.fr", pseudo.to_lowercase())
    }

    /// Compte SSO connu de l'annuaire (accès à l'app) mais jamais connecté.
    pub fn add_sso_user(&self, sso_id: i64, pseudo: &str) {
        let mut users = self.auth.users.lock().unwrap();
        if !users.iter().any(|u| u.user_id == sso_id) {
            users.push(SsoUser { user_id: sso_id, email: Self::email_of(pseudo), pseudo: pseudo.into() });
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
            SsoSession { user_id: sso_id, email: Self::email_of(pseudo), pseudo: pseudo.into(), is_admin: admin, apps },
        );
        token
    }

    pub fn request(method: Method, path: &str, token: Option<&str>) -> axum::http::request::Builder {
        let mut req = Request::builder().method(method.clone()).uri(path);
        if let Some(t) = token {
            req = req.header(header::COOKIE, format!("theme=dark; lim_session={t}"));
        }
        if method != Method::GET {
            req = req.header(header::ORIGIN, ORIGIN);
        }
        req
    }

    pub async fn call(&self, method: Method, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
        let req = Self::request(method, path, token);
        let req = match body {
            Some(v) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())),
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
        self.call(Method::PATCH, path, Some(token), Some(body)).await
    }
    pub async fn delete(&self, path: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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
            serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
        };
        (status, value)
    }

    pub async fn multipart(&self, method: Method, path: &str, token: &str, parts: Vec<(&str, Part)>) -> (StatusCode, Value) {
        let boundary = "XSANTABOUNDARYX";
        let mut body = Vec::new();
        for (name, part) in parts {
            body.extend(format!("--{boundary}\r\n").as_bytes());
            match part {
                Part::Text(v) => {
                    body.extend(format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{v}\r\n").as_bytes());
                }
                Part::File { filename, content_type, bytes } => {
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
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
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
        self.mailer.sent.lock().unwrap().iter().filter(|m| m.to == email).cloned().collect()
    }
}
```

`tests/me.rs` :

```rust
mod common;

use axum::{body::Body, http::{Method, Request, StatusCode}};
use common::TestApp;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn sans_cookie_401_avec_url_de_connexion() {
    let app = TestApp::new().await;
    let (status, body) = app.call(Method::GET, "/api/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "NOT_AUTHENTICATED");
    assert_eq!(body["loginUrl"], "https://auth.test/login");
}

#[tokio::test]
async fn jeton_inconnu_401() {
    let app = TestApp::new().await;
    let (status, _) = app.get("/api/me", "inconnu").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sso_en_panne_echec_ferme() {
    let app = TestApp::new().await;
    let token = app.login(1, "Alice", false);
    app.auth.down.store(true, Ordering::SeqCst);
    let (status, _) = app.get("/api/me", &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sans_acces_a_l_app_403() {
    let app = TestApp::new().await;
    let token = app.login_without_app(1, "Alice");
    let (status, body) = app.get("/api/me", &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "FORBIDDEN");
}

#[tokio::test]
async fn me_cree_le_joueur_et_renvoie_son_profil() {
    let app = TestApp::new().await;
    let token = app.login(1, "Alice", true);
    let (status, body) = app.get("/api/me", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["pseudo"], "Alice");
    assert_eq!(body["email"], "alice@x.fr");
    assert_eq!(body["isAdmin"], true);
    assert_eq!(body["playerId"], app.player_id("Alice").await);
}

#[tokio::test]
async fn le_pseudo_suit_le_sso() {
    let app = TestApp::new().await;
    let token = app.login(1, "Alice", false);
    app.get("/api/me", &token).await;
    let id = app.player_id("Alice").await;
    app.auth.sessions.lock().unwrap().get_mut(&token).unwrap().pseudo = "Alicia".into();
    let (_, body) = app.get("/api/me", &token).await;
    assert_eq!(body["pseudo"], "Alicia");
    assert_eq!(body["playerId"], id);
}

#[tokio::test]
async fn health_sans_authentification() {
    let app = TestApp::new().await;
    let res = app.response(Request::get("/api/health").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
}
```

Remplacer `tests/health.rs` par :

```rust
mod common;

use axum::{body::Body, http::{Request, StatusCode}};

#[tokio::test]
async fn health_repond_ok() {
    let app = common::TestApp::new().await;
    let res = app.response(Request::get("/api/health").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --test me`
Expected: FAIL à la compilation — `routes::router(state)`, `repo::players` absents.

- [ ] **Step 3: Implémenter `src/repo/players.rs`**

```rust
use crate::auth::SsoUser;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Player {
    pub id: i64,
    pub email: String,
    pub sso_user_id: Option<i64>,
    pub pseudo: String,
    pub has_access: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRef {
    pub player_id: i64,
    pub pseudo: String,
}

const COLS: &str = "id, email, sso_user_id, pseudo, has_access";

/// Relie un compte SSO à son joueur : par `sso_user_id` s'il est déjà connu,
/// sinon par l'adresse (joueur importé de `players.txt`), sinon création.
pub async fn upsert_sso(
    conn: &mut SqliteConnection,
    sso_user_id: i64,
    email: &str,
    pseudo: &str,
    now: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let email = email.trim().to_lowercase();
    let known: Option<i64> = sqlx::query_scalar("SELECT id FROM players WHERE sso_user_id = ?")
        .bind(sso_user_id)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(id) = known {
        sqlx::query("UPDATE players SET pseudo = ?, has_access = 1, updated_at = ? WHERE id = ?")
            .bind(pseudo)
            .bind(now)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        // Adresse changée dans le SSO : suivie, sauf si un autre joueur la porte déjà.
        sqlx::query("UPDATE players SET email = ? WHERE id = ? AND NOT EXISTS (SELECT 1 FROM players WHERE email = ? AND id <> ?)")
            .bind(&email)
            .bind(id)
            .bind(&email)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        return Ok(id);
    }
    sqlx::query_scalar(
        "INSERT INTO players (email, sso_user_id, pseudo, has_access, updated_at) VALUES (?, ?, ?, 1, ?)
         ON CONFLICT (email) DO UPDATE SET sso_user_id = excluded.sso_user_id, pseudo = excluded.pseudo,
             has_access = 1, updated_at = excluded.updated_at
         RETURNING id",
    )
    .bind(&email)
    .bind(sso_user_id)
    .bind(pseudo)
    .bind(now)
    .fetch_one(&mut *conn)
    .await
}

/// Instantané de l'annuaire SSO : seuls les comptes listés gardent l'accès.
pub async fn sync_access(pool: &SqlitePool, users: &[SsoUser], now: DateTime<Utc>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE players SET has_access = 0").execute(&mut *tx).await?;
    for u in users {
        upsert_sso(&mut tx, u.user_id, &u.email, &u.pseudo, now).await?;
    }
    tx.commit().await
}

pub async fn find(pool: &SqlitePool, id: i64) -> Result<Option<Player>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM players WHERE id = ?")).bind(id).fetch_optional(pool).await
}

pub async fn with_access(pool: &SqlitePool) -> Result<Vec<Player>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM players WHERE has_access = 1 ORDER BY lower(pseudo)"))
        .fetch_all(pool)
        .await
}

/// Joueur venu de `players.txt` : créé sans compte SSO s'il est inconnu, le
/// pseudo existant n'est jamais écrasé.
pub async fn upsert_imported(conn: &mut SqliteConnection, email: &str, pseudo: &str, now: DateTime<Utc>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO players (email, pseudo, has_access, updated_at) VALUES (?, ?, 0, ?)
         ON CONFLICT (email) DO UPDATE SET email = excluded.email
         RETURNING id",
    )
    .bind(email.trim().to_lowercase())
    .bind(pseudo)
    .bind(now)
    .fetch_one(&mut *conn)
    .await
}

/// Joueurs à montrer dans « Participants » : inscrits à une édition, présents
/// dans un tirage, ou ayant un cadeau dans leur historique.
pub async fn list_visible(pool: &SqlitePool) -> Result<Vec<PlayerRef>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id AS player_id, p.pseudo FROM players p
         WHERE EXISTS (SELECT 1 FROM registrations r WHERE r.player_id = p.id AND r.participating = 1)
            OR EXISTS (SELECT 1 FROM assignments a WHERE a.giver_id = p.id OR a.receiver_id = p.id)
            OR EXISTS (SELECT 1 FROM gifts g WHERE g.player_id = p.id)
         ORDER BY lower(p.pseudo)",
    )
    .fetch_all(pool)
    .await
}
```

- [ ] **Step 4: Implémenter `src/routes/extract.rs` et `src/routes/me.rs`**

`src/routes/extract.rs` :

```rust
use crate::{APP_SLUG, auth::SESSION_COOKIE, error::ApiError, repo::players, state::AppState};
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, Method, header, request::Parts},
};

pub struct CurrentUser {
    pub player_id: i64,
    pub email: String,
    pub pseudo: String,
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

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let unauthorized = || ApiError::Unauthorized { login_url: state.config.auth_login_url.clone() };

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
        let player_id = players::upsert_sso(&mut conn, session.user_id, &session.email, &session.pseudo, state.now()).await?;

        Ok(CurrentUser {
            player_id,
            email: session.email.trim().to_lowercase(),
            pseudo: session.pseudo,
            is_admin: session.is_admin,
            token,
        })
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
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
        h.append(header::COOKIE, HeaderValue::from_static("a=1; lim_session=abc; b=2"));
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
        h.insert(header::ORIGIN, HeaderValue::from_static("https://tierlistrr.test"));
        assert!(check_origin(&h, "https://santa.test").is_err());
        h.insert(header::ORIGIN, HeaderValue::from_static("https://santa.test"));
        assert!(check_origin(&h, "https://santa.test").is_ok());
    }
}
```

`src/routes/me.rs` :

```rust
use super::extract::CurrentUser;
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeView {
    pub player_id: i64,
    pub pseudo: String,
    pub email: String,
    pub is_admin: bool,
}

pub async fn me(user: CurrentUser) -> Json<MeView> {
    Json(MeView { player_id: user.player_id, pseudo: user.pseudo, email: user.email, is_admin: user.is_admin })
}
```

Remplacer `src/routes/mod.rs` :

```rust
pub mod extract;
pub mod me;

use crate::state::AppState;
use axum::{Router, extract::DefaultBodyLimit, routing::get};
use tower_http::trace::TraceLayer;

pub async fn health() -> &'static str {
    "ok"
}

pub fn router(state: AppState) -> Router {
    let api: Router<AppState> = Router::new()
        .route("/health", get(health))
        .route("/me", get(me::me));

    Router::new()
        .nest("/api", api)
        // Photos (5 Mo) + marge pour l'enveloppe multipart.
        .layer(DefaultBodyLimit::max(6 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

Adapter `src/main.rs` pour qu'il compile (câblage complet en Task 13) — remplacer l'appel au routeur par :

```rust
    let pool = secret_santa::db::connect(&config.database_path).await.expect("base de données");
    let state = secret_santa::state::AppState {
        pool,
        auth: std::sync::Arc::new(secret_santa::auth::HttpAuthClient::new(config.auth_url.clone(), std::time::Duration::from_secs(60))),
        mailer: std::sync::Arc::new(secret_santa::mail::LogMailer),
        clock: std::sync::Arc::new(secret_santa::clock::SystemClock),
        tasks: tokio_util::task::TaskTracker::new(),
        config: std::sync::Arc::new(config.clone()),
    };
    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await.expect("impossible d'ouvrir le port");
    tracing::info!(addr = %config.bind_addr, "Secret Santa à l'écoute");
    axum::serve(listener, routes::router(state)).await.expect("serveur arrêté sur une erreur");
```

- [ ] **Step 5: Lancer les tests**

Run: `cargo test`
Expected: PASS (tous, dont les 7 de `tests/me.rs` et les 3 de `extract`).

- [ ] **Step 6: Commit**

```bash
git add src tests
git commit -m "Authentification des requêtes par le SSO, /api/me et banc de test

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Éditions, inscriptions et administration de l'édition

**Files:**
- Create: `src/repo/editions.rs`, `src/services/mod.rs`, `src/services/mailing.rs`, `src/routes/editions.rs`, `src/routes/admin.rs`, `tests/editions.rs`
- Modify: `src/repo/mod.rs` (`pub mod editions;`), `src/lib.rs` (`pub mod services;`), `src/routes/mod.rs` (routes)

**Interfaces:**
- Consumes: `AppState`, `CurrentUser`, `AdminUser`, `players::{sync_access, with_access, PlayerRef, Player}`, `templates::announcement`, `AuthError`.
- Produces:
  - `repo::editions` : `EditionStatus { Open, Drawn, Failed }` (sérialisé `"open"|"drawn"|"failed"`), `Edition { id, year, budget_eur, deadline, status, failure_reason, drawn_at, reminder_sent_at, announced_at, imported, created_at }`, `by_year`, `by_id`, `create_open(pool, year, budget, deadline, now) -> Edition`, `update_budget`, `update_deadline` (repasse `failed` en `open`), `mark_announced`, `mark_reminder_sent -> bool`, `open_editions`, `set_registration(pool, edition_id, player_id, Option<bool>, by_admin, now)`, `registration_of -> Option<bool>`, `participants -> Vec<PlayerRef>`, `registrations -> HashMap<i64, (bool, bool)>`, `non_respondents_with_access -> Vec<Player>`.
  - `services::mailing::{announce(state, edition_id), send_logged(state, &Mail, kind) -> bool}`.
  - `routes::editions::{EditionView, edition_view(state, &Edition, &CurrentUser), current_edition(state)}`.
  - Routes : `GET /api/editions/current`, `PUT /api/editions/current/registration`, `GET /api/admin/sso-users`, `POST /api/admin/editions`, `PATCH /api/admin/editions/current`, `PUT /api/admin/editions/current/registrations/{player_id}`.

- [ ] **Step 1: Écrire les tests d'intégration**

`tests/editions.rs` :

```rust
mod common;

use axum::http::{Method, Request, StatusCode, header};
use axum::body::Body;
use chrono::Duration;
use common::TestApp;
use serde_json::json;
use std::sync::atomic::Ordering;

fn in_days(app: &TestApp, days: i64) -> String {
    use secret_santa::clock::Clock;
    (app.clock.now() + Duration::days(days)).to_rfc3339()
}

async fn open_edition(app: &TestApp, admin: &str) -> serde_json::Value {
    let (status, body) = app.post("/api/admin/editions", admin, json!({ "budgetEur": 30, "deadline": in_days(app, 20) })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

#[tokio::test]
async fn aucune_edition_renvoie_null() {
    let app = TestApp::new().await;
    let t = app.login(1, "Alice", false);
    let (status, body) = app.get("/api/editions/current", &t).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_null());
}

#[tokio::test]
async fn l_admin_cree_l_edition_et_l_annonce_part_a_tous() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.add_sso_user(2, "Bob");
    app.add_sso_user(3, "Chloe");

    let body = open_edition(&app, &admin).await;
    assert_eq!(body["year"], 2026);
    assert_eq!(body["status"], "open");
    assert_eq!(body["budgetEur"], 30);
    assert_eq!(body["participants"], json!([]));

    app.settle().await;
    for email in ["nathan@x.fr", "bob@x.fr", "chloe@x.fr"] {
        let mails = app.mails_to(email);
        assert_eq!(mails.len(), 1, "annonce pour {email}");
        assert!(mails[0].subject.contains("inscriptions sont ouvertes"));
    }
}

#[tokio::test]
async fn creation_refusee_aux_non_admins_et_validee() {
    let app = TestApp::new().await;
    let pote = app.login(2, "Bob", false);
    let (status, _) = app.post("/api/admin/editions", &pote, json!({ "budgetEur": 30, "deadline": in_days(&app, 5) })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let admin = app.login(1, "Nathan", true);
    let (status, body) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": in_days(&app, -1) })).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("INVALID_DEADLINE")));
    let (status, body) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 0, "deadline": in_days(&app, 5) })).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("INVALID_BUDGET")));

    open_edition(&app, &admin).await;
    let (status, body) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": in_days(&app, 5) })).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("EDITION_EXISTS")));
}

#[tokio::test]
async fn creation_impossible_si_le_sso_est_injoignable() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.get("/api/me", &admin).await; // session mise en place avant la panne
    app.auth.down.store(true, Ordering::SeqCst);
    let (status, _) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": in_days(&app, 5) })).await;
    // La session elle-même n'est plus vérifiable : échec fermé.
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn un_joueur_s_inscrit_puis_change_d_avis() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let bob = app.login(2, "Bob", false);
    open_edition(&app, &admin).await;

    let (status, body) = app.put("/api/editions/current/registration", &bob, json!({ "participating": true })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["myRegistration"], true);
    assert_eq!(body["participants"], json!([{ "playerId": app.player_id("Bob").await, "pseudo": "Bob" }]));

    let (_, body) = app.put("/api/editions/current/registration", &bob, json!({ "participating": false })).await;
    assert_eq!(body["myRegistration"], false);
    assert_eq!(body["participants"], json!([]));

    let (_, body) = app.get("/api/editions/current", &admin).await;
    assert_eq!(body["myRegistration"], serde_json::Value::Null);
}

#[tokio::test]
async fn inscriptions_closes_apres_la_deadline() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let bob = app.login(2, "Bob", false);
    open_edition(&app, &admin).await;
    app.clock.advance(Duration::days(21));
    let (status, body) = app.put("/api/editions/current/registration", &bob, json!({ "participating": true })).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("REGISTRATION_CLOSED")));
}

#[tokio::test]
async fn inscription_sans_edition_404() {
    let app = TestApp::new().await;
    let bob = app.login(2, "Bob", false);
    let (status, _) = app.put("/api/editions/current/registration", &bob, json!({ "participating": true })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ecriture_sans_origin_refusee() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    open_edition(&app, &admin).await;
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/api/editions/current/registration")
        .header(header::COOKIE, format!("lim_session={admin}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://tierlistrr.limperiam.com")
        .body(Body::from(json!({ "participating": true }).to_string()))
        .unwrap();
    let (status, _) = app.send(req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn l_admin_voit_l_annuaire_et_repond_a_la_place_d_un_joueur() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.add_sso_user(2, "Bob");
    app.add_sso_user(3, "Chloe");
    open_edition(&app, &admin).await;

    let (status, body) = app.get("/api/admin/sso-users", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let pseudos: Vec<_> = body.as_array().unwrap().iter().map(|u| u["pseudo"].as_str().unwrap().to_string()).collect();
    assert_eq!(pseudos, ["Bob", "Chloe", "Nathan"]);
    assert!(body.as_array().unwrap().iter().all(|u| u["registration"].is_null()));

    let chloe = app.player_id("Chloe").await;
    let (status, _) = app.put(&format!("/api/admin/editions/current/registrations/{chloe}"), &admin, json!({ "participating": true })).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get("/api/admin/sso-users", &admin).await;
    let row = body.as_array().unwrap().iter().find(|u| u["pseudo"] == "Chloe").unwrap();
    assert_eq!((row["registration"].as_bool(), row["answeredByAdmin"].as_bool()), (Some(true), Some(true)));
    assert_eq!(row["email"], "chloe@x.fr");

    let (status, _) = app.put(&format!("/api/admin/editions/current/registrations/{chloe}"), &admin, json!({ "participating": null })).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get("/api/admin/sso-users", &admin).await;
    let row = body.as_array().unwrap().iter().find(|u| u["pseudo"] == "Chloe").unwrap();
    assert!(row["registration"].is_null());
}

#[tokio::test]
async fn un_compte_retire_du_sso_disparait_de_l_annuaire() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.add_sso_user(2, "Bob");
    app.get("/api/admin/sso-users", &admin).await;
    app.auth.users.lock().unwrap().retain(|u| u.user_id != 2);
    let (_, body) = app.get("/api/admin/sso-users", &admin).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn l_admin_modifie_budget_et_deadline() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    open_edition(&app, &admin).await;
    let new_deadline = in_days(&app, 30);
    let (status, body) = app.patch("/api/admin/editions/current", &admin, json!({ "budgetEur": 40, "deadline": new_deadline })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["budgetEur"], 40);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(body["deadline"].as_str().unwrap()).unwrap(),
        chrono::DateTime::parse_from_rfc3339(&new_deadline).unwrap()
    );
}
```

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --test editions`
Expected: FAIL à la compilation.

- [ ] **Step 3: Implémenter `src/repo/editions.rs`**

```rust
use super::players::{Player, PlayerRef};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum EditionStatus {
    Open,
    Drawn,
    Failed,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Edition {
    pub id: i64,
    pub year: i64,
    pub budget_eur: i64,
    pub deadline: DateTime<Utc>,
    pub status: EditionStatus,
    pub failure_reason: Option<String>,
    pub drawn_at: Option<DateTime<Utc>>,
    pub reminder_sent_at: Option<DateTime<Utc>>,
    pub announced_at: Option<DateTime<Utc>>,
    pub imported: bool,
    pub created_at: DateTime<Utc>,
}

const COLS: &str = "id, year, budget_eur, deadline, status, failure_reason, drawn_at, reminder_sent_at, announced_at, imported, created_at";

pub async fn by_year(pool: &SqlitePool, year: i64) -> Result<Option<Edition>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM editions WHERE year = ?")).bind(year).fetch_optional(pool).await
}

pub async fn by_id(pool: &SqlitePool, id: i64) -> Result<Option<Edition>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM editions WHERE id = ?")).bind(id).fetch_optional(pool).await
}

pub async fn create_open(pool: &SqlitePool, year: i64, budget_eur: i64, deadline: DateTime<Utc>, now: DateTime<Utc>) -> Result<Edition, sqlx::Error> {
    sqlx::query_as(&format!(
        "INSERT INTO editions (year, budget_eur, deadline, status, created_at) VALUES (?, ?, ?, 'open', ?) RETURNING {COLS}"
    ))
    .bind(year)
    .bind(budget_eur)
    .bind(deadline)
    .bind(now)
    .fetch_one(pool)
    .await
}

pub async fn update_budget(pool: &SqlitePool, id: i64, budget_eur: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE editions SET budget_eur = ? WHERE id = ? AND status <> 'drawn'")
        .bind(budget_eur)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Une nouvelle deadline rouvre une édition dont le tirage a échoué.
pub async fn update_deadline(pool: &SqlitePool, id: i64, deadline: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE editions SET deadline = ?,
             status = CASE WHEN status = 'failed' THEN 'open' ELSE status END,
             failure_reason = CASE WHEN status = 'failed' THEN NULL ELSE failure_reason END
         WHERE id = ? AND status <> 'drawn'",
    )
    .bind(deadline)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_announced(pool: &SqlitePool, id: i64, now: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE editions SET announced_at = ? WHERE id = ?").bind(now).bind(id).execute(pool).await?;
    Ok(())
}

/// `true` si c'est cet appel qui a posé la marque (relance à envoyer).
pub async fn mark_reminder_sent(pool: &SqlitePool, id: i64, now: DateTime<Utc>) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("UPDATE editions SET reminder_sent_at = ? WHERE id = ? AND reminder_sent_at IS NULL")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}

pub async fn open_editions(pool: &SqlitePool) -> Result<Vec<Edition>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM editions WHERE status = 'open'")).fetch_all(pool).await
}

/// `None` efface la réponse (retour à « pas répondu »).
pub async fn set_registration(
    pool: &SqlitePool,
    edition_id: i64,
    player_id: i64,
    participating: Option<bool>,
    by_admin: bool,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    match participating {
        None => {
            sqlx::query("DELETE FROM registrations WHERE edition_id = ? AND player_id = ?")
                .bind(edition_id)
                .bind(player_id)
                .execute(pool)
                .await?;
        }
        Some(p) => {
            sqlx::query(
                "INSERT INTO registrations (edition_id, player_id, participating, answered_by_admin, updated_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT (edition_id, player_id) DO UPDATE SET
                     participating = excluded.participating,
                     answered_by_admin = excluded.answered_by_admin,
                     updated_at = excluded.updated_at",
            )
            .bind(edition_id)
            .bind(player_id)
            .bind(p)
            .bind(by_admin)
            .bind(now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

pub async fn registration_of(pool: &SqlitePool, edition_id: i64, player_id: i64) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar("SELECT participating FROM registrations WHERE edition_id = ? AND player_id = ?")
        .bind(edition_id)
        .bind(player_id)
        .fetch_optional(pool)
        .await
}

pub async fn participants(pool: &SqlitePool, edition_id: i64) -> Result<Vec<PlayerRef>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id AS player_id, p.pseudo FROM registrations r JOIN players p ON p.id = r.player_id
         WHERE r.edition_id = ? AND r.participating = 1 ORDER BY lower(p.pseudo)",
    )
    .bind(edition_id)
    .fetch_all(pool)
    .await
}

/// joueur → (participe, répondu par l'admin)
pub async fn registrations(pool: &SqlitePool, edition_id: i64) -> Result<HashMap<i64, (bool, bool)>, sqlx::Error> {
    let rows: Vec<(i64, bool, bool)> =
        sqlx::query_as("SELECT player_id, participating, answered_by_admin FROM registrations WHERE edition_id = ?")
            .bind(edition_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(id, p, a)| (id, (p, a))).collect())
}

pub async fn non_respondents_with_access(pool: &SqlitePool, edition_id: i64) -> Result<Vec<Player>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, email, sso_user_id, pseudo, has_access FROM players p
         WHERE has_access = 1
           AND NOT EXISTS (SELECT 1 FROM registrations r WHERE r.edition_id = ? AND r.player_id = p.id)
         ORDER BY lower(pseudo)",
    )
    .bind(edition_id)
    .fetch_all(pool)
    .await
}
```

`src/repo/mod.rs` : ajouter `pub mod editions;`.

- [ ] **Step 4: Implémenter `src/services/mailing.rs`**

`src/services/mod.rs` :

```rust
pub mod mailing;
```

`src/services/mailing.rs` :

```rust
//! Envois en tâche de fond. Une vague de mails prend une seconde par
//! destinataire (comme la version Java) : jamais dans le cycle d'une requête.

use crate::{
    error::ApiError,
    mail::{Mail, templates},
    repo::{editions, players},
    state::AppState,
};

/// Journalise l'échec sans l'adresse du destinataire.
pub async fn send_logged(state: &AppState, mail: &Mail, kind: &str) -> bool {
    match state.mailer.send(mail).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(kind, error = %e, "échec d'envoi d'un mail");
            false
        }
    }
}

pub async fn announce(state: AppState, edition_id: i64) {
    let result: Result<(), ApiError> = async {
        let Some(edition) = editions::by_id(&state.pool, edition_id).await? else { return Ok(()) };
        for p in players::with_access(&state.pool).await? {
            let mail = templates::announcement(&p.email, &p.pseudo, edition.year, edition.budget_eur, edition.deadline, &state.config.public_origin);
            send_logged(&state, &mail, "ouverture").await;
            tokio::time::sleep(state.config.mail_delay).await;
        }
        editions::mark_announced(&state.pool, edition_id, state.now()).await?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "annonce de l'édition interrompue");
    }
}
```

`src/lib.rs` : ajouter `pub mod services;`.

- [ ] **Step 5: Implémenter `src/routes/editions.rs`**

```rust
use super::extract::CurrentUser;
use crate::{
    error::{ApiError, ApiResult},
    repo::{
        editions::{self, Edition, EditionStatus},
        players::PlayerRef,
    },
    state::AppState,
};
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionView {
    pub year: i64,
    pub budget_eur: i64,
    pub deadline: DateTime<Utc>,
    pub status: EditionStatus,
    /// Réservé à l'admin.
    pub failure_reason: Option<String>,
    pub drawn_at: Option<DateTime<Utc>>,
    pub participants: Vec<PlayerRef>,
    pub my_registration: Option<bool>,
    pub imported: bool,
}

pub async fn edition_view(state: &AppState, e: &Edition, viewer: &CurrentUser) -> ApiResult<EditionView> {
    Ok(EditionView {
        year: e.year,
        budget_eur: e.budget_eur,
        deadline: e.deadline,
        status: e.status,
        failure_reason: if viewer.is_admin { e.failure_reason.clone() } else { None },
        drawn_at: e.drawn_at,
        participants: editions::participants(&state.pool, e.id).await?,
        my_registration: editions::registration_of(&state.pool, e.id, viewer.player_id).await?,
        imported: e.imported,
    })
}

pub async fn current_edition(state: &AppState) -> ApiResult<Option<Edition>> {
    Ok(editions::by_year(&state.pool, state.current_year()).await?)
}

pub async fn require_current(state: &AppState) -> ApiResult<Edition> {
    current_edition(state).await?.ok_or_else(|| ApiError::not_found("Aucune édition cette année."))
}

pub async fn get_current(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Option<EditionView>>> {
    match current_edition(&state).await? {
        None => Ok(Json(None)),
        Some(e) => Ok(Json(Some(edition_view(&state, &e, &user).await?))),
    }
}

#[derive(Deserialize)]
pub struct RegistrationBody {
    pub participating: bool,
}

pub async fn put_registration(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<RegistrationBody>,
) -> ApiResult<Json<EditionView>> {
    let e = require_current(&state).await?;
    if e.status != EditionStatus::Open || state.now() >= e.deadline {
        return Err(ApiError::conflict("REGISTRATION_CLOSED", "Les inscriptions sont closes."));
    }
    editions::set_registration(&state.pool, e.id, user.player_id, Some(body.participating), false, state.now()).await?;
    Ok(Json(edition_view(&state, &e, &user).await?))
}
```

- [ ] **Step 6: Implémenter `src/routes/admin.rs` (première partie)**

```rust
use super::{
    editions::{EditionView, edition_view, require_current},
    extract::AdminUser,
};
use crate::{
    APP_SLUG,
    auth::AuthError,
    error::{ApiError, ApiResult},
    repo::{
        editions::{self, EditionStatus},
        players,
    },
    services::mailing,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn auth_error(e: AuthError) -> ApiError {
    match e {
        AuthError::Forbidden => ApiError::Forbidden,
        AuthError::Unavailable(detail) => {
            tracing::warn!(%detail, "annuaire SSO indisponible");
            ApiError::Upstream
        }
    }
}

fn validate_budget(budget: i64) -> ApiResult<()> {
    if (1..=1000).contains(&budget) {
        Ok(())
    } else {
        Err(ApiError::bad_request("INVALID_BUDGET", "Le budget doit être compris entre 1 et 1000 €."))
    }
}

fn validate_deadline(state: &AppState, deadline: DateTime<Utc>) -> ApiResult<()> {
    if deadline > state.now() {
        Ok(())
    } else {
        Err(ApiError::bad_request("INVALID_DEADLINE", "La deadline doit être dans le futur."))
    }
}

/// Rafraîchit l'instantané des comptes ayant accès à l'app, au nom de l'admin.
async fn refresh_directory(state: &AppState, admin: &AdminUser) -> ApiResult<()> {
    let users = state.auth.list_users(&admin.0.token, APP_SLUG).await.map_err(auth_error)?;
    players::sync_access(&state.pool, &users, state.now()).await?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub player_id: i64,
    pub pseudo: String,
    pub email: String,
    pub registration: Option<bool>,
    pub answered_by_admin: bool,
}

pub async fn sso_users(State(state): State<AppState>, admin: AdminUser) -> ApiResult<Json<Vec<DirectoryEntry>>> {
    refresh_directory(&state, &admin).await?;
    let regs = match super::editions::current_edition(&state).await? {
        Some(e) => editions::registrations(&state.pool, e.id).await?,
        None => Default::default(),
    };
    let entries = players::with_access(&state.pool)
        .await?
        .into_iter()
        .map(|p| {
            let reg = regs.get(&p.id).copied();
            DirectoryEntry {
                player_id: p.id,
                pseudo: p.pseudo,
                email: p.email,
                registration: reg.map(|(participating, _)| participating),
                answered_by_admin: reg.is_some_and(|(_, by_admin)| by_admin),
            }
        })
        .collect();
    Ok(Json(entries))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEdition {
    pub budget_eur: i64,
    pub deadline: DateTime<Utc>,
}

pub async fn create_edition(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<CreateEdition>,
) -> ApiResult<(StatusCode, Json<EditionView>)> {
    validate_budget(body.budget_eur)?;
    validate_deadline(&state, body.deadline)?;
    let year = state.current_year();
    if editions::by_year(&state.pool, year).await?.is_some() {
        return Err(ApiError::conflict("EDITION_EXISTS", format!("L'édition {year} existe déjà.")));
    }
    refresh_directory(&state, &admin).await?;
    let edition = editions::create_open(&state.pool, year, body.budget_eur, body.deadline, state.now()).await?;
    state.spawn(mailing::announce(state.clone(), edition.id));
    Ok((StatusCode::CREATED, Json(edition_view(&state, &edition, &admin.0).await?)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchEdition {
    pub budget_eur: Option<i64>,
    pub deadline: Option<DateTime<Utc>>,
}

pub async fn patch_edition(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<PatchEdition>,
) -> ApiResult<Json<EditionView>> {
    let e = require_current(&state).await?;
    if e.status == EditionStatus::Drawn {
        return Err(ApiError::conflict("EDITION_DRAWN", "Le tirage a déjà eu lieu."));
    }
    if let Some(budget) = body.budget_eur {
        validate_budget(budget)?;
        editions::update_budget(&state.pool, e.id, budget).await?;
    }
    if let Some(deadline) = body.deadline {
        validate_deadline(&state, deadline)?;
        editions::update_deadline(&state.pool, e.id, deadline).await?;
    }
    let e = require_current(&state).await?;
    Ok(Json(edition_view(&state, &e, &admin.0).await?))
}

#[derive(Deserialize)]
pub struct AdminRegistration {
    #[serde(default)]
    pub participating: Option<bool>,
}

pub async fn put_registration_for(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(player_id): Path<i64>,
    Json(body): Json<AdminRegistration>,
) -> ApiResult<StatusCode> {
    let e = require_current(&state).await?;
    if e.status != EditionStatus::Open {
        return Err(ApiError::conflict("REGISTRATION_CLOSED", "Les inscriptions sont closes."));
    }
    if players::find(&state.pool, player_id).await?.is_none() {
        return Err(ApiError::not_found("Joueur inconnu."));
    }
    editions::set_registration(&state.pool, e.id, player_id, body.participating, true, state.now()).await?;
    Ok(StatusCode::OK)
}
```

- [ ] **Step 7: Brancher les routes**

Dans `src/routes/mod.rs`, ajouter `pub mod admin; pub mod editions;`, compléter l'import `use axum::routing::{get, patch, post, put};` et le routeur `api` :

```rust
    let api: Router<AppState> = Router::new()
        .route("/health", get(health))
        .route("/me", get(me::me))
        .route("/editions/current", get(editions::get_current))
        .route("/editions/current/registration", put(editions::put_registration))
        .route("/admin/sso-users", get(admin::sso_users))
        .route("/admin/editions", post(admin::create_edition))
        .route("/admin/editions/current", patch(admin::patch_edition))
        .route("/admin/editions/current/registrations/{player_id}", put(admin::put_registration_for));
```

- [ ] **Step 8: Lancer les tests**

Run: `cargo test`
Expected: PASS (dont les 11 de `tests/editions.rs`).

- [ ] **Step 9: Commit**

```bash
git add src tests
git commit -m "Éditions, inscriptions des joueurs et administration de l'édition

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Tirage, mails de résultat, ma cible, annulation

**Files:**
- Create: `src/repo/assignments.rs`, `src/services/draw_service.rs`, `tests/draw.rs`
- Modify: `src/repo/mod.rs`, `src/services/mod.rs`, `src/services/mailing.rs`, `src/routes/editions.rs`, `src/routes/admin.rs`, `src/routes/mod.rs`

**Interfaces:**
- Consumes: `draw::draw`, `Edition`, `players::{find, PlayerRef}`, `templates::{result, draw_failed}`, `mailing::send_logged`.
- Produces:
  - `repo::assignments` : `MailStatus { Pending, Sent, Failed, NotApplicable }` (sérialisé `"pending"|"sent"|"failed"|"none"`), `insert(&mut SqliteConnection, edition_id, giver, receiver, MailStatus)`, `previous_targets(&mut SqliteConnection, year) -> HashMap<i64, i64>`, `target_of(pool, edition_id, giver) -> Option<PlayerRef>`, `mail_statuses(pool, edition_id) -> Vec<MailRow>`, `pending(pool, edition_id) -> Vec<i64>`, `set_mail_status(pool, edition_id, giver, MailStatus, Option<&str>)`, `editions_with_pending(pool) -> Vec<i64>`.
  - `services::draw_service` : `enum DrawOutcome { Drawn, AlreadyDone, Failed(String) }`, `run_draw(&AppState, edition_id) -> ApiResult<DrawOutcome>`, `cancel_draw(&AppState, edition_id, new_deadline) -> ApiResult<bool>`, `resume_pending_mails(&AppState)`.
  - `mailing::{send_results(state, edition_id), send_result(&state, &Edition, giver_id) -> ApiResult<()>, notify_failure(state, edition_id)}`.
  - Routes : `GET /api/editions/current/my-target`, `POST|DELETE /api/admin/editions/current/draw`, `GET /api/admin/editions/current/mails`, `POST /api/admin/editions/current/mails/{player_id}/resend`.

- [ ] **Step 1: Écrire les tests d'intégration**

`tests/draw.rs` :

```rust
mod common;

use axum::http::StatusCode;
use chrono::Duration;
use common::TestApp;
use secret_santa::{clock::Clock, services::draw_service::{self, DrawOutcome}};
use serde_json::json;
use std::collections::{HashMap, HashSet};

struct Game {
    app: TestApp,
    admin: String,
    tokens: Vec<(String, String)>, // (pseudo, jeton)
}

/// Édition ouverte (deadline J+20) où `pseudos` se sont inscrits.
async fn game(pseudos: &[&str]) -> Game {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let deadline = (app.clock.now() + Duration::days(20)).to_rfc3339();
    let (status, _) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": deadline })).await;
    assert_eq!(status, StatusCode::CREATED);
    let mut tokens = Vec::new();
    for (i, p) in pseudos.iter().enumerate() {
        let t = app.login(10 + i as i64, p, false);
        let (status, _) = app.put("/api/editions/current/registration", &t, json!({ "participating": true })).await;
        assert_eq!(status, StatusCode::OK);
        tokens.push((p.to_string(), t));
    }
    app.settle().await;
    app.mailer.sent.lock().unwrap().clear(); // on oublie les annonces
    Game { app, admin, tokens }
}

async fn edition_id(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT id FROM editions WHERE year = 2026").fetch_one(&app.state.pool).await.unwrap()
}

#[tokio::test]
async fn tirage_reussi_chacun_recoit_sa_cible_et_personne_d_autre_ne_la_voit() {
    let g = game(&["Alice", "Bob", "Chloe", "Dan"]).await;
    let (status, body) = g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "drawn");
    g.app.settle().await;

    let mut targets = HashMap::new();
    for (pseudo, token) in &g.tokens {
        let (status, target) = g.app.get("/api/editions/current/my-target", token).await;
        assert_eq!(status, StatusCode::OK);
        let target = target["pseudo"].as_str().unwrap().to_string();
        assert_ne!(&target, pseudo);
        let mails = g.app.mails_to(&format!("{}@x.fr", pseudo.to_lowercase()));
        assert_eq!(mails.len(), 1);
        assert!(mails[0].text.contains(&format!("tu offres un cadeau à : {target}")));
        targets.insert(pseudo.clone(), target);
    }
    let receivers: HashSet<_> = targets.values().cloned().collect();
    assert_eq!(receivers.len(), 4);

    // L'admin ne participe pas : pas de cible, et la route des mails ne dit rien des receveurs.
    let (status, _) = g.app.get("/api/editions/current/my-target", &g.admin).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, mails) = g.app.get("/api/admin/editions/current/mails", &g.admin).await;
    assert_eq!(status, StatusCode::OK);
    for row in mails.as_array().unwrap() {
        assert_eq!(row["mailStatus"], "sent");
        let keys: HashSet<_> = row.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, HashSet::from(["playerId".into(), "pseudo".into(), "mailStatus".into(), "mailError".into()]));
    }
}

#[tokio::test]
async fn ma_cible_avant_le_tirage_404() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    let (status, _) = g.app.get("/api/editions/current/my-target", &g.tokens[0].1).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn un_second_tirage_est_refuse() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    let (status, body) = g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("ALREADY_DRAWN")));
}

#[tokio::test]
async fn deux_declenchements_simultanes_ne_tirent_qu_une_fois() {
    let g = game(&["Alice", "Bob", "Chloe", "Dan"]).await;
    let id = edition_id(&g.app).await;
    let (a, b) = tokio::join!(draw_service::run_draw(&g.app.state, id), draw_service::run_draw(&g.app.state, id));
    let outcomes = [a.unwrap(), b.unwrap()];
    assert_eq!(outcomes.iter().filter(|o| **o == DrawOutcome::Drawn).count(), 1);
    assert_eq!(outcomes.iter().filter(|o| **o == DrawOutcome::AlreadyDone).count(), 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assignments WHERE edition_id = ?").bind(id).fetch_one(&g.app.state.pool).await.unwrap();
    assert_eq!(count, 4);
}

#[tokio::test]
async fn moins_de_trois_participants_echec_signale_a_l_admin() {
    let g = game(&["Alice", "Bob"]).await;
    let (status, body) = g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("DRAW_FAILED")));
    assert!(body["message"].as_str().unwrap().contains("au moins 3"));
    g.app.settle().await;

    let admin_mails = g.app.mails_to("admin@x.fr");
    assert_eq!(admin_mails.len(), 1);
    assert!(admin_mails[0].subject.contains("échoué"));
    assert!(g.app.mails_to("alice@x.fr").is_empty());

    let (_, view) = g.app.get("/api/editions/current", &g.admin).await;
    assert_eq!(view["status"], "failed");
    assert!(view["failureReason"].as_str().unwrap().contains("au moins 3"));
    let (_, view) = g.app.get("/api/editions/current", &g.tokens[0].1).await;
    assert!(view["failureReason"].is_null(), "la raison est réservée à l'admin");

    // Repousser la deadline rouvre l'édition.
    let deadline = (g.app.clock.now() + Duration::days(3)).to_rfc3339();
    let (_, view) = g.app.patch("/api/admin/editions/current", &g.admin, json!({ "deadline": deadline })).await;
    assert_eq!(view["status"], "open");
    assert!(view["failureReason"].is_null());
}

#[tokio::test]
async fn contrainte_de_l_an_dernier_insoluble() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    let alice = g.app.player_id("Alice").await;
    let bob = g.app.player_id("Bob").await;
    let pool = &g.app.state.pool;
    let old: i64 = sqlx::query_scalar(
        "INSERT INTO editions (year, budget_eur, deadline, status, drawn_at, created_at)
         VALUES (2025, 30, '2025-12-01T00:00:00Z', 'drawn', '2025-12-01T00:00:00Z', '2025-11-01T00:00:00Z') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    for (giver, receiver) in [(alice, bob), (bob, alice)] {
        sqlx::query("INSERT INTO assignments (edition_id, giver_id, receiver_id, mail_status) VALUES (?, ?, ?, 'none')")
            .bind(old).bind(giver).bind(receiver).execute(pool).await.unwrap();
    }
    let (status, body) = g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["message"].as_str().unwrap().contains("l'an dernier"));
}

#[tokio::test]
async fn echec_smtp_puis_renvoi() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.mailer.failing.lock().unwrap().insert("bob@x.fr".into());
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    g.app.settle().await;

    let (_, mails) = g.app.get("/api/admin/editions/current/mails", &g.admin).await;
    let bob_row = mails.as_array().unwrap().iter().find(|r| r["pseudo"] == "Bob").unwrap().clone();
    assert_eq!(bob_row["mailStatus"], "failed");
    assert!(bob_row["mailError"].as_str().unwrap().contains("boîte pleine"));

    g.app.mailer.failing.lock().unwrap().clear();
    let bob = g.app.player_id("Bob").await;
    let (status, _) = g.app.post(&format!("/api/admin/editions/current/mails/{bob}/resend"), &g.admin, json!({})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    g.app.settle().await;
    let (_, mails) = g.app.get("/api/admin/editions/current/mails", &g.admin).await;
    let bob_row = mails.as_array().unwrap().iter().find(|r| r["pseudo"] == "Bob").unwrap().clone();
    assert_eq!(bob_row["mailStatus"], "sent");
    assert_eq!(g.app.mails_to("bob@x.fr").len(), 1);
}

#[tokio::test]
async fn renvoi_pour_un_non_participant_404() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    let nathan = g.app.player_id("Nathan").await;
    let (status, _) = g.app.post(&format!("/api/admin/editions/current/mails/{nathan}/resend"), &g.admin, json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn annuler_le_tirage_rouvre_les_inscriptions() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    g.app.settle().await;
    let deadline = (g.app.clock.now() + Duration::days(2)).to_rfc3339();
    let (status, view) = g.app.delete("/api/admin/editions/current/draw", &g.admin, Some(json!({ "deadline": deadline }))).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["status"], "open");
    assert_eq!(view["participants"].as_array().unwrap().len(), 3, "les inscriptions sont conservées");
    let (status, _) = g.app.get("/api/editions/current/my-target", &g.tokens[0].1).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = g.app.delete("/api/admin/editions/current/draw", &g.admin, Some(json!({ "deadline": deadline }))).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("NOT_DRAWN")));
}

#[tokio::test]
async fn les_mails_en_attente_repartent_au_demarrage() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    g.app.post("/api/admin/editions/current/draw", &g.admin, json!({})).await;
    g.app.settle().await;
    sqlx::query("UPDATE assignments SET mail_status = 'pending'").execute(&g.app.state.pool).await.unwrap();
    g.app.mailer.sent.lock().unwrap().clear();

    draw_service::resume_pending_mails(&g.app.state).await;
    g.app.settle().await;
    assert_eq!(g.app.mailer.sent.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn routes_de_tirage_reservees_a_l_admin() {
    let g = game(&["Alice", "Bob", "Chloe"]).await;
    let (status, _) = g.app.post("/api/admin/editions/current/draw", &g.tokens[0].1, json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = g.app.get("/api/admin/editions/current/mails", &g.tokens[0].1).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
```

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --test draw`
Expected: FAIL à la compilation.

- [ ] **Step 3: Implémenter `src/repo/assignments.rs`**

```rust
use super::players::PlayerRef;
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum MailStatus {
    Pending,
    Sent,
    Failed,
    /// Édition importée : aucun mail n'est jamais parti.
    #[sqlx(rename = "none")]
    #[serde(rename = "none")]
    NotApplicable,
}

/// Ce que l'admin voit d'un envoi — jamais le receveur.
#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MailRow {
    pub player_id: i64,
    pub pseudo: String,
    pub mail_status: MailStatus,
    pub mail_error: Option<String>,
}

pub async fn insert(conn: &mut SqliteConnection, edition_id: i64, giver: i64, receiver: i64, status: MailStatus) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO assignments (edition_id, giver_id, receiver_id, mail_status) VALUES (?, ?, ?, ?)")
        .bind(edition_id)
        .bind(giver)
        .bind(receiver)
        .bind(status)
        .execute(conn)
        .await?;
    Ok(())
}

/// Paires de l'édition tirée la plus récente d'une année antérieure.
pub async fn previous_targets(conn: &mut SqliteConnection, year: i64) -> Result<HashMap<i64, i64>, sqlx::Error> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT giver_id, receiver_id FROM assignments
         WHERE edition_id = (SELECT id FROM editions WHERE status = 'drawn' AND year < ? ORDER BY year DESC LIMIT 1)",
    )
    .bind(year)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().collect())
}

pub async fn target_of(pool: &SqlitePool, edition_id: i64, giver: i64) -> Result<Option<PlayerRef>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id AS player_id, p.pseudo FROM assignments a JOIN players p ON p.id = a.receiver_id
         WHERE a.edition_id = ? AND a.giver_id = ?",
    )
    .bind(edition_id)
    .bind(giver)
    .fetch_optional(pool)
    .await
}

pub async fn mail_statuses(pool: &SqlitePool, edition_id: i64) -> Result<Vec<MailRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id AS player_id, p.pseudo, a.mail_status, a.mail_error FROM assignments a
         JOIN players p ON p.id = a.giver_id WHERE a.edition_id = ? ORDER BY lower(p.pseudo)",
    )
    .bind(edition_id)
    .fetch_all(pool)
    .await
}

pub async fn pending(pool: &SqlitePool, edition_id: i64) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT giver_id FROM assignments WHERE edition_id = ? AND mail_status = 'pending' ORDER BY giver_id")
        .bind(edition_id)
        .fetch_all(pool)
        .await
}

pub async fn set_mail_status(pool: &SqlitePool, edition_id: i64, giver: i64, status: MailStatus, error: Option<&str>) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("UPDATE assignments SET mail_status = ?, mail_error = ? WHERE edition_id = ? AND giver_id = ?")
        .bind(status)
        .bind(error)
        .bind(edition_id)
        .bind(giver)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}

pub async fn editions_with_pending(pool: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT DISTINCT edition_id FROM assignments WHERE mail_status = 'pending'").fetch_all(pool).await
}
```

`src/repo/mod.rs` : ajouter `pub mod assignments;`.

- [ ] **Step 4: Compléter `src/services/mailing.rs`**

Ajouter les imports `repo::assignments::{self, MailStatus}` et `repo::editions::Edition`, puis :

```rust
pub async fn send_results(state: AppState, edition_id: i64) {
    let result: Result<(), ApiError> = async {
        let Some(edition) = editions::by_id(&state.pool, edition_id).await? else { return Ok(()) };
        for giver in assignments::pending(&state.pool, edition_id).await? {
            send_result(&state, &edition, giver).await?;
            tokio::time::sleep(state.config.mail_delay).await;
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "envoi des résultats interrompu");
    }
}

/// Envoie (ou renvoie) le résultat d'un donneur et enregistre le statut.
pub async fn send_result(state: &AppState, edition: &Edition, giver_id: i64) -> Result<(), ApiError> {
    let giver = players::find(&state.pool, giver_id).await?.ok_or_else(|| ApiError::not_found("Joueur inconnu."))?;
    let target = assignments::target_of(&state.pool, edition.id, giver_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Pas de tirage pour ce joueur."))?;
    let mail = templates::result(
        &giver.email,
        &giver.pseudo,
        &target.pseudo,
        target.player_id,
        edition.year,
        edition.budget_eur,
        &state.config.public_origin,
    );
    match state.mailer.send(&mail).await {
        Ok(()) => assignments::set_mail_status(&state.pool, edition.id, giver_id, MailStatus::Sent, None).await?,
        Err(e) => {
            tracing::warn!(error = %e, "échec d'envoi d'un résultat");
            assignments::set_mail_status(&state.pool, edition.id, giver_id, MailStatus::Failed, Some(&e.to_string())).await?
        }
    };
    Ok(())
}

pub async fn notify_failure(state: AppState, edition_id: i64) {
    let Ok(Some(edition)) = editions::by_id(&state.pool, edition_id).await else { return };
    let reason = edition.failure_reason.clone().unwrap_or_default();
    let mail = templates::draw_failed(&state.config.admin_email, edition.year, &reason, &state.config.public_origin);
    send_logged(&state, &mail, "échec du tirage").await;
}
```

- [ ] **Step 5: Implémenter `src/services/draw_service.rs`**

```rust
use crate::{
    draw,
    error::ApiResult,
    repo::assignments::{self, MailStatus},
    services::mailing,
    state::AppState,
};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawOutcome {
    Drawn,
    /// Quelqu'un d'autre (tâche de fond ou admin) a tiré entre-temps.
    AlreadyDone,
    Failed(String),
}

/// Tirage idempotent : la transition `open|failed → drawn` est le premier
/// ordre de la transaction et ne réussit qu'une fois.
pub async fn run_draw(state: &AppState, edition_id: i64) -> ApiResult<DrawOutcome> {
    let now = state.now();
    let mut tx = state.pool.begin().await?;

    let claimed = sqlx::query(
        "UPDATE editions SET status = 'drawn', drawn_at = ?, failure_reason = NULL WHERE id = ? AND status IN ('open', 'failed')",
    )
    .bind(now)
    .bind(edition_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if claimed == 0 {
        tx.rollback().await?;
        return Ok(DrawOutcome::AlreadyDone);
    }

    let year: i64 = sqlx::query_scalar("SELECT year FROM editions WHERE id = ?").bind(edition_id).fetch_one(&mut *tx).await?;
    let participants: Vec<i64> =
        sqlx::query_scalar("SELECT player_id FROM registrations WHERE edition_id = ? AND participating = 1 ORDER BY player_id")
            .bind(edition_id)
            .fetch_all(&mut *tx)
            .await?;
    let previous = assignments::previous_targets(&mut tx, year).await?;

    // Le générateur ne doit pas vivre au-delà d'un `.await` (il n'est pas Send).
    let result = {
        let mut rng = rand::rng();
        draw::draw(&participants, &previous, &mut rng)
    };

    match result {
        Err(err) => {
            let reason = err.to_string();
            sqlx::query("UPDATE editions SET status = 'failed', failure_reason = ?, drawn_at = NULL WHERE id = ?")
                .bind(&reason)
                .bind(edition_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            tracing::warn!(year, %reason, "tirage impossible");
            state.spawn(mailing::notify_failure(state.clone(), edition_id));
            Ok(DrawOutcome::Failed(reason))
        }
        Ok(pairs) => {
            for (giver, receiver) in pairs {
                assignments::insert(&mut tx, edition_id, giver, receiver, MailStatus::Pending).await?;
            }
            tx.commit().await?;
            tracing::info!(year, participants = participants.len(), "tirage effectué");
            state.spawn(mailing::send_results(state.clone(), edition_id));
            Ok(DrawOutcome::Drawn)
        }
    }
}

/// Dernier recours : efface les paires et rouvre les inscriptions. Les mails
/// déjà partis ne sont pas rattrapés.
pub async fn cancel_draw(state: &AppState, edition_id: i64, new_deadline: DateTime<Utc>) -> ApiResult<bool> {
    let mut tx = state.pool.begin().await?;
    let reopened = sqlx::query(
        "UPDATE editions SET status = 'open', drawn_at = NULL, deadline = ?, reminder_sent_at = NULL WHERE id = ? AND status = 'drawn'",
    )
    .bind(new_deadline)
    .bind(edition_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if reopened == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    sqlx::query("DELETE FROM assignments WHERE edition_id = ?").bind(edition_id).execute(&mut *tx).await?;
    tx.commit().await?;
    tracing::warn!(edition_id, "tirage annulé par l'admin");
    Ok(true)
}

/// Au démarrage : un arrêt pendant une vague d'envois laisse des `pending`.
pub async fn resume_pending_mails(state: &AppState) {
    match assignments::editions_with_pending(&state.pool).await {
        Ok(ids) => {
            for id in ids {
                state.spawn(mailing::send_results(state.clone(), id));
            }
        }
        Err(e) => tracing::error!(error = %e, "lecture des mails en attente impossible"),
    }
}
```

`src/services/mod.rs` : ajouter `pub mod draw_service;`.

- [ ] **Step 6: Routes `my-target` et administration du tirage**

Dans `src/routes/editions.rs`, ajouter :

```rust
pub async fn my_target(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<PlayerRef>> {
    let not_found = || ApiError::not_found("Pas de cible pour toi cette année.");
    let e = current_edition(&state).await?.ok_or_else(not_found)?;
    if e.status != EditionStatus::Drawn {
        return Err(not_found());
    }
    let target = crate::repo::assignments::target_of(&state.pool, e.id, user.player_id).await?;
    target.map(Json).ok_or_else(not_found)
}
```

Dans `src/routes/admin.rs`, ajouter les imports `crate::repo::assignments::{self, MailRow, MailStatus}` et `crate::services::draw_service::{self, DrawOutcome}`, puis :

```rust
pub async fn draw_now(State(state): State<AppState>, admin: AdminUser) -> ApiResult<Json<EditionView>> {
    let e = require_current(&state).await?;
    if e.status == EditionStatus::Drawn {
        return Err(ApiError::conflict("ALREADY_DRAWN", "Le tirage a déjà eu lieu."));
    }
    match draw_service::run_draw(&state, e.id).await? {
        DrawOutcome::Drawn => {
            let e = require_current(&state).await?;
            Ok(Json(edition_view(&state, &e, &admin.0).await?))
        }
        DrawOutcome::AlreadyDone => Err(ApiError::conflict("ALREADY_DRAWN", "Le tirage a déjà eu lieu.")),
        DrawOutcome::Failed(reason) => Err(ApiError::unprocessable("DRAW_FAILED", reason)),
    }
}

#[derive(Deserialize)]
pub struct CancelDraw {
    pub deadline: DateTime<Utc>,
}

pub async fn cancel_draw(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<CancelDraw>,
) -> ApiResult<Json<EditionView>> {
    validate_deadline(&state, body.deadline)?;
    let e = require_current(&state).await?;
    if !draw_service::cancel_draw(&state, e.id, body.deadline).await? {
        return Err(ApiError::conflict("NOT_DRAWN", "Il n'y a pas de tirage à annuler."));
    }
    let e = require_current(&state).await?;
    Ok(Json(edition_view(&state, &e, &admin.0).await?))
}

pub async fn mails(State(state): State<AppState>, _admin: AdminUser) -> ApiResult<Json<Vec<MailRow>>> {
    let e = require_current(&state).await?;
    Ok(Json(assignments::mail_statuses(&state.pool, e.id).await?))
}

pub async fn resend(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(player_id): Path<i64>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let e = require_current(&state).await?;
    if !assignments::set_mail_status(&state.pool, e.id, player_id, MailStatus::Pending, None).await? {
        return Err(ApiError::not_found("Ce joueur ne fait pas partie du tirage."));
    }
    let task_state = state.clone();
    state.spawn(async move {
        if let Err(err) = mailing::send_result(&task_state, &e, player_id).await {
            tracing::error!(error = %err, "renvoi impossible");
        }
    });
    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({ "mailStatus": "pending" }))))
}
```

Dans `src/routes/mod.rs`, compléter l'import (`use axum::routing::{get, patch, post, put};` → ajouter si besoin) et les routes :

```rust
        .route("/editions/current/my-target", get(editions::my_target))
        .route("/admin/editions/current/draw", post(admin::draw_now).delete(admin::cancel_draw))
        .route("/admin/editions/current/mails", get(admin::mails))
        .route("/admin/editions/current/mails/{player_id}/resend", post(admin::resend))
```

- [ ] **Step 7: Lancer les tests**

Run: `cargo test`
Expected: PASS (dont les 11 de `tests/draw.rs`).

- [ ] **Step 8: Commit**

```bash
git add src tests
git commit -m "Tirage idempotent, mails de résultat, ma cible et annulation

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Planificateur (tirage automatique et relance)

**Files:**
- Create: `src/services/scheduler.rs`, `tests/scheduler.rs`
- Modify: `src/services/mod.rs`, `src/services/mailing.rs`

**Interfaces:**
- Consumes: `editions::{open_editions, mark_reminder_sent, non_respondents_with_access}`, `draw_service::run_draw`, `templates::reminder`.
- Produces: `scheduler::tick(&AppState) -> ApiResult<()>`, `scheduler::spawn(AppState) -> JoinHandle<()>`, `scheduler::reminder_before() -> chrono::Duration` (48 h), `mailing::remind(state, edition_id)`.

- [ ] **Step 1: Écrire les tests**

`tests/scheduler.rs` :

```rust
mod common;

use axum::http::StatusCode;
use chrono::Duration;
use common::TestApp;
use secret_santa::{clock::Clock, services::scheduler};
use serde_json::json;

async fn setup(deadline_in: Duration, joined: &[&str], silent: &[&str]) -> (TestApp, String) {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    for (i, p) in silent.iter().enumerate() {
        app.add_sso_user(100 + i as i64, p);
    }
    let tokens: Vec<_> = joined.iter().enumerate().map(|(i, p)| app.login(10 + i as i64, p, false)).collect();
    let deadline = (app.clock.now() + deadline_in).to_rfc3339();
    let (status, _) = app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": deadline })).await;
    assert_eq!(status, StatusCode::CREATED);
    for t in &tokens {
        app.put("/api/editions/current/registration", t, json!({ "participating": true })).await;
    }
    app.settle().await;
    app.mailer.sent.lock().unwrap().clear();
    (app, admin)
}

#[tokio::test]
async fn rien_avant_la_fenetre_de_relance() {
    let (app, _) = setup(Duration::days(10), &["Alice", "Bob", "Chloe"], &["Dan"]).await;
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert!(app.mailer.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn relance_une_seule_fois_aux_seuls_non_repondants() {
    let (app, admin) = setup(Duration::days(10), &["Alice", "Bob", "Chloe"], &["Dan", "Eve"]).await;
    app.clock.advance(Duration::days(8) + Duration::hours(1)); // J-47h
    scheduler::tick(&app.state).await.unwrap();
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;

    let sent = app.mailer.sent.lock().unwrap().clone();
    let mut to: Vec<_> = sent.iter().map(|m| m.to.clone()).collect();
    to.sort();
    // Nathan (admin) n'a pas répondu non plus : il est relancé.
    assert_eq!(to, ["dan@x.fr", "eve@x.fr", "nathan@x.fr"]);
    assert!(sent.iter().all(|m| m.subject.contains("48 h")));
    let _ = admin;
}

#[tokio::test]
async fn pas_de_relance_si_la_deadline_etait_deja_proche() {
    let (app, _) = setup(Duration::hours(30), &["Alice", "Bob", "Chloe"], &["Dan"]).await;
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert!(app.mailer.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn tirage_automatique_a_la_deadline() {
    let (app, admin) = setup(Duration::days(10), &["Alice", "Bob", "Chloe"], &[]).await;
    app.clock.advance(Duration::days(10));
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    let (_, view) = app.get("/api/editions/current", &admin).await;
    assert_eq!(view["status"], "drawn");
    for email in ["alice@x.fr", "bob@x.fr", "chloe@x.fr"] {
        assert!(app.mails_to(email).iter().any(|m| m.subject.contains("ton tirage")));
    }
    // Un tour de plus ne refait rien.
    app.mailer.sent.lock().unwrap().clear();
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert!(app.mailer.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn un_echec_n_est_pas_retente_automatiquement() {
    let (app, admin) = setup(Duration::days(1), &["Alice", "Bob"], &[]).await;
    app.clock.advance(Duration::days(2));
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert_eq!(app.mails_to("admin@x.fr").len(), 1);
    scheduler::tick(&app.state).await.unwrap();
    app.settle().await;
    assert_eq!(app.mails_to("admin@x.fr").len(), 1);
    let (_, view) = app.get("/api/editions/current", &admin).await;
    assert_eq!(view["status"], "failed");
}
```

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --test scheduler`
Expected: FAIL à la compilation.

- [ ] **Step 3: Ajouter `remind` à `src/services/mailing.rs`**

```rust
pub async fn remind(state: AppState, edition_id: i64) {
    let result: Result<(), ApiError> = async {
        let Some(edition) = editions::by_id(&state.pool, edition_id).await? else { return Ok(()) };
        for p in editions::non_respondents_with_access(&state.pool, edition_id).await? {
            let mail = templates::reminder(&p.email, &p.pseudo, edition.year, edition.deadline, &state.config.public_origin);
            send_logged(&state, &mail, "relance").await;
            tokio::time::sleep(state.config.mail_delay).await;
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "relance interrompue");
    }
}
```

- [ ] **Step 4: Implémenter `src/services/scheduler.rs`**

```rust
//! Boucle de fond : chaque minute, tirer les éditions dont la deadline est
//! passée et relancer les retardataires. L'état vit en base : un conteneur
//! arrêté à la deadline tire au premier tour après son redémarrage.

use crate::{
    error::ApiResult,
    repo::editions,
    services::{draw_service::{self, DrawOutcome}, mailing},
    state::AppState,
};
use std::time::Duration;
use tokio::{task::JoinHandle, time::MissedTickBehavior};

pub fn reminder_before() -> chrono::Duration {
    chrono::Duration::hours(48)
}

pub async fn tick(state: &AppState) -> ApiResult<()> {
    let now = state.now();
    for e in editions::open_editions(&state.pool).await? {
        if e.deadline <= now {
            match draw_service::run_draw(state, e.id).await? {
                DrawOutcome::Drawn => tracing::info!(year = e.year, "tirage automatique effectué"),
                DrawOutcome::AlreadyDone => {}
                DrawOutcome::Failed(reason) => tracing::warn!(year = e.year, %reason, "tirage automatique impossible"),
            }
            continue;
        }
        let remind_at = e.deadline - reminder_before();
        let created_early_enough = e.created_at <= remind_at;
        if e.reminder_sent_at.is_none() && created_early_enough && now >= remind_at && editions::mark_reminder_sent(&state.pool, e.id, now).await? {
            state.spawn(mailing::remind(state.clone(), e.id));
        }
    }
    Ok(())
}

pub fn spawn(state: AppState) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(e) = tick(&state).await {
                tracing::error!(error = %e, "tour du planificateur en échec");
            }
        }
    })
}
```

`src/services/mod.rs` : ajouter `pub mod scheduler;`.

- [ ] **Step 5: Lancer les tests**

Run: `cargo test`
Expected: PASS (dont les 5 de `tests/scheduler.rs`).

- [ ] **Step 6: Commit**

```bash
git add src tests
git commit -m "Planificateur : tirage automatique à la deadline et relance à 48 h

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Profils, cadeaux et photos

**Files:**
- Create: `src/photos.rs`, `src/repo/gifts.rs`, `src/routes/players.rs`, `src/routes/gifts.rs`, `tests/gifts.rs`
- Modify: `src/lib.rs` (`pub mod photos;`), `src/repo/mod.rs`, `src/routes/mod.rs`

**Interfaces:**
- Consumes: `players::{find, list_visible, PlayerRef}`, `CurrentUser`, `Config::uploads_dir`.
- Produces:
  - `photos` : `MAX_BYTES`, `MAX_SIDE`, `PhotoError { TooLarge, UnsupportedFormat, Unreadable }`, `normalize(&[u8]) -> Result<Vec<u8>, PhotoError>` (JPEG), `is_valid_name(&str) -> bool`, `store(&Path, &[u8]) -> io::Result<String>`, `remove(&Path, &str)`.
  - `repo::gifts` : `Gift { id, player_id, year, description, photo_path, created_at }`, `list_for`, `find_owned(pool, id, player_id)`, `insert`, `update`, `delete`.
  - Routes : `GET /api/players`, `GET /api/players/{id}` → `{ playerId, pseudo, gifts: [{ id, year, description, photoUrl }] }`, `POST /api/me/gifts`, `PUT|DELETE /api/me/gifts/{id}`, `GET /api/uploads/{file}`. Champs multipart : `year`, `description`, `photo` (fichier), `removePhoto` (`"true"`).

- [ ] **Step 1: Écrire les tests unitaires des photos**

`src/photos.rs` (tests seuls pour commencer) :

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbImage};
    use std::io::Cursor;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30])))
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn reduit_et_reencode_en_jpeg() {
        let jpeg = normalize(&png(3200, 1000)).unwrap();
        assert_eq!(image::guess_format(&jpeg).unwrap(), ImageFormat::Jpeg);
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (1600, 500));
    }

    #[test]
    fn petite_image_gardee_a_sa_taille() {
        let img = image::load_from_memory(&normalize(&png(400, 300)).unwrap()).unwrap();
        assert_eq!((img.width(), img.height()), (400, 300));
    }

    #[test]
    fn refuse_ce_qui_n_est_pas_une_image_prise_en_charge() {
        assert_eq!(normalize(b"bonjour"), Err(PhotoError::UnsupportedFormat));
        assert_eq!(normalize(b"GIF89a\x01\x00\x01\x00\x00\x00\x00"), Err(PhotoError::UnsupportedFormat));
    }

    #[test]
    fn refuse_plus_de_5_mo() {
        assert_eq!(normalize(&vec![0u8; MAX_BYTES + 1]), Err(PhotoError::TooLarge));
    }

    #[test]
    fn noms_de_fichier() {
        assert!(is_valid_name("0f8c2c1e-4b6a-4c43-9a55-2f1d7a3b9e10.jpg"));
        assert!(!is_valid_name("../santa.db"));
        assert!(!is_valid_name("0f8c2c1e-4b6a-4c43-9a55-2f1d7a3b9e10.png"));
        assert!(!is_valid_name(""));
    }
}
```

`src/lib.rs` : ajouter `pub mod photos;`.

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --lib photos`
Expected: FAIL à la compilation.

- [ ] **Step 3: Implémenter `src/photos.rs`**

```rust
//! Photos de cadeaux : vérifiées sur leurs octets (jamais l'extension),
//! orientées, réduites, puis réencodées en JPEG — ce qui efface les
//! métadonnées EXIF, géolocalisation comprise.

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, codecs::jpeg::JpegEncoder, imageops::FilterType, metadata::Orientation};
use std::{io::Cursor, path::Path};

pub const MAX_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_SIDE: u32 = 1600;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PhotoError {
    #[error("Photo trop lourde (5 Mo maximum).")]
    TooLarge,
    #[error("Format non pris en charge (JPEG, PNG ou WebP).")]
    UnsupportedFormat,
    #[error("Image illisible.")]
    Unreadable,
}

pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>, PhotoError> {
    if bytes.len() > MAX_BYTES {
        return Err(PhotoError::TooLarge);
    }
    let format = image::guess_format(bytes).map_err(|_| PhotoError::UnsupportedFormat)?;
    if !matches!(format, ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP) {
        return Err(PhotoError::UnsupportedFormat);
    }
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(|_| PhotoError::Unreadable)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|_| PhotoError::Unreadable)?;
    img.apply_orientation(orientation);
    if img.width() > MAX_SIDE || img.height() > MAX_SIDE {
        img = img.resize(MAX_SIDE, MAX_SIDE, FilterType::Lanczos3);
    }
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(&img.to_rgb8())
        .map_err(|_| PhotoError::Unreadable)?;
    Ok(out)
}

/// `<uuid v4>.jpg` uniquement : ferme la porte aux `../` sur la route de lecture.
pub fn is_valid_name(name: &str) -> bool {
    name.len() == 40
        && name.ends_with(".jpg")
        && name[..36].chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

pub async fn store(dir: &Path, jpeg: &[u8]) -> std::io::Result<String> {
    tokio::fs::create_dir_all(dir).await?;
    let name = format!("{}.jpg", uuid::Uuid::new_v4());
    tokio::fs::write(dir.join(&name), jpeg).await?;
    Ok(name)
}

pub async fn remove(dir: &Path, name: &str) {
    if is_valid_name(name) {
        if let Err(e) = tokio::fs::remove_file(dir.join(name)).await {
            tracing::warn!(error = %e, "photo non supprimée");
        }
    }
}
```

Si `ImageDecoder::orientation` / `apply_orientation` n'existent pas dans la version résolue d'image (< 0.25.2), faire `cargo update -p image` ; ils sont présents depuis 0.25.2.

Run: `cargo test --lib photos`
Expected: PASS (5 tests).

- [ ] **Step 4: Écrire les tests d'intégration**

`tests/gifts.rs` :

```rust
mod common;

use axum::{body::Body, http::{Method, Request, StatusCode, header}};
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
    Part::File { filename: "cadeau.png".into(), content_type: "image/png".into(), bytes }
}

#[tokio::test]
async fn ajouter_un_cadeau_avec_photo_et_le_voir_sur_le_profil() {
    let app = TestApp::new().await;
    let alice = app.login(1, "Alice", false);
    let bob = app.login(2, "Bob", false);

    let (status, gift) = app
        .multipart(Method::POST, "/api/me/gifts", &alice, vec![
            ("year", text("2025")),
            ("description", text("  Un pull moche avec un renne  ")),
            ("photo", photo(png())),
        ])
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

    let req = Request::get(&url).header(header::COOKIE, format!("lim_session={bob}")).body(Body::empty()).unwrap();
    let res = app.response(req).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "image/jpeg");

    // Alice apparaît dans la liste des joueurs grâce à son cadeau.
    let (_, players) = app.get("/api/players", &bob).await;
    assert!(players.as_array().unwrap().iter().any(|p| p["pseudo"] == "Alice"));
}

#[tokio::test]
async fn photo_sans_session_401() {
    let app = TestApp::new().await;
    let req = Request::get("/api/uploads/0f8c2c1e-4b6a-4c43-9a55-2f1d7a3b9e10.jpg").body(Body::empty()).unwrap();
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
        (vec![("year", text("1999")), ("description", text("x"))], "INVALID_YEAR"),
        (vec![("year", text("2028")), ("description", text("x"))], "INVALID_YEAR"),
        (vec![("year", text("2025")), ("description", text("   "))], "INVALID_DESCRIPTION"),
        (vec![("year", text("2025")), ("description", text("x".repeat(501)))], "INVALID_DESCRIPTION"),
        (vec![("year", text("2025")), ("description", text("x")), ("photo", photo(b"pas une image".to_vec()))], "INVALID_PHOTO"),
    ];
    for (parts, code) in cases {
        let (status, body) = app.multipart(Method::POST, "/api/me/gifts", &t, parts).await;
        assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some(code)));
    }
}

#[tokio::test]
async fn modifier_et_supprimer_seulement_ses_cadeaux() {
    let app = TestApp::new().await;
    let alice = app.login(1, "Alice", false);
    let bob = app.login(2, "Bob", false);
    let (_, gift) = app
        .multipart(Method::POST, "/api/me/gifts", &alice, vec![("year", text("2024")), ("description", text("Un livre")), ("photo", photo(png()))])
        .await;
    let id = gift["id"].as_i64().unwrap();
    let old_file = gift["photoUrl"].as_str().unwrap().trim_start_matches("/api/uploads/").to_string();

    let (status, _) = app.multipart(Method::PUT, &format!("/api/me/gifts/{id}"), &bob, vec![("year", text("2024")), ("description", text("Piraté"))]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.delete(&format!("/api/me/gifts/{id}"), &bob, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, updated) = app
        .multipart(Method::PUT, &format!("/api/me/gifts/{id}"), &alice, vec![("year", text("2025")), ("description", text("Deux livres")), ("removePhoto", text("true"))])
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!((updated["year"].as_i64(), updated["description"].as_str()), (Some(2025), Some("Deux livres")));
    assert!(updated["photoUrl"].is_null());
    assert!(!app.state.config.uploads_dir.join(&old_file).exists(), "l'ancienne photo est supprimée");

    let (status, _) = app.delete(&format!("/api/me/gifts/{id}"), &alice, None).await;
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
```

- [ ] **Step 5: Vérifier l'échec**

Run: `cargo test --test gifts`
Expected: FAIL (routes absentes).

- [ ] **Step 6: Implémenter `src/repo/gifts.rs`**

```rust
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Gift {
    pub id: i64,
    pub player_id: i64,
    pub year: i64,
    pub description: String,
    pub photo_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

const COLS: &str = "id, player_id, year, description, photo_path, created_at";

pub async fn list_for(pool: &SqlitePool, player_id: i64) -> Result<Vec<Gift>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM gifts WHERE player_id = ? ORDER BY year DESC, id DESC"))
        .bind(player_id)
        .fetch_all(pool)
        .await
}

pub async fn find_owned(pool: &SqlitePool, id: i64, player_id: i64) -> Result<Option<Gift>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM gifts WHERE id = ? AND player_id = ?"))
        .bind(id)
        .bind(player_id)
        .fetch_optional(pool)
        .await
}

pub async fn insert(pool: &SqlitePool, player_id: i64, year: i64, description: &str, photo: Option<&str>, now: DateTime<Utc>) -> Result<Gift, sqlx::Error> {
    sqlx::query_as(&format!(
        "INSERT INTO gifts (player_id, year, description, photo_path, created_at) VALUES (?, ?, ?, ?, ?) RETURNING {COLS}"
    ))
    .bind(player_id)
    .bind(year)
    .bind(description)
    .bind(photo)
    .bind(now)
    .fetch_one(pool)
    .await
}

pub async fn update(pool: &SqlitePool, id: i64, year: i64, description: &str, photo: Option<&str>) -> Result<Gift, sqlx::Error> {
    sqlx::query_as(&format!("UPDATE gifts SET year = ?, description = ?, photo_path = ? WHERE id = ? RETURNING {COLS}"))
        .bind(year)
        .bind(description)
        .bind(photo)
        .bind(id)
        .fetch_one(pool)
        .await
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM gifts WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}
```

`src/repo/mod.rs` : ajouter `pub mod gifts;`.

- [ ] **Step 7: Implémenter `src/routes/gifts.rs`**

```rust
use super::extract::CurrentUser;
use crate::{
    error::{ApiError, ApiResult},
    photos,
    repo::gifts::{self, Gift},
    state::AppState,
};
use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{StatusCode, header},
    response::Response,
};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiftView {
    pub id: i64,
    pub year: i64,
    pub description: String,
    pub photo_url: Option<String>,
}

impl From<Gift> for GiftView {
    fn from(g: Gift) -> Self {
        GiftView { id: g.id, year: g.year, description: g.description, photo_url: g.photo_path.map(|p| format!("/api/uploads/{p}")) }
    }
}

#[derive(Default)]
struct GiftForm {
    year: Option<String>,
    description: Option<String>,
    photo: Option<Vec<u8>>,
    remove_photo: bool,
}

async fn read_form(mut mp: Multipart) -> ApiResult<GiftForm> {
    let bad = |e: axum::extract::multipart::MultipartError| ApiError::bad_request("INVALID_FORM", e.to_string());
    let mut form = GiftForm::default();
    while let Some(field) = mp.next_field().await.map_err(bad)? {
        match field.name().unwrap_or_default() {
            "year" => form.year = Some(field.text().await.map_err(bad)?),
            "description" => form.description = Some(field.text().await.map_err(bad)?),
            "photo" => {
                let bytes = field.bytes().await.map_err(bad)?;
                if !bytes.is_empty() {
                    form.photo = Some(bytes.to_vec());
                }
            }
            "removePhoto" => form.remove_photo = field.text().await.map_err(bad)? == "true",
            _ => {}
        }
    }
    Ok(form)
}

fn validate(form: &GiftForm, current_year: i64) -> ApiResult<(i64, String)> {
    let year = form
        .year
        .as_deref()
        .and_then(|y| y.trim().parse::<i64>().ok())
        .filter(|y| (2000..=current_year + 1).contains(y))
        .ok_or_else(|| ApiError::bad_request("INVALID_YEAR", format!("L'année doit être comprise entre 2000 et {}.", current_year + 1)))?;
    let description = form.description.as_deref().unwrap_or_default().trim().to_string();
    if description.is_empty() || description.chars().count() > 500 {
        return Err(ApiError::bad_request("INVALID_DESCRIPTION", "La description doit faire entre 1 et 500 caractères."));
    }
    Ok((year, description))
}

/// Normalise (hors du runtime async : c'est du calcul) puis enregistre.
async fn save_photo(state: &AppState, bytes: Vec<u8>) -> ApiResult<String> {
    let jpeg = tokio::task::spawn_blocking(move || photos::normalize(&bytes))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::bad_request("INVALID_PHOTO", e.to_string()))?;
    photos::store(&state.config.uploads_dir, &jpeg).await.map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn create(State(state): State<AppState>, user: CurrentUser, mp: Multipart) -> ApiResult<(StatusCode, Json<GiftView>)> {
    let form = read_form(mp).await?;
    let (year, description) = validate(&form, state.current_year())?;
    let photo = match form.photo {
        Some(bytes) => Some(save_photo(&state, bytes).await?),
        None => None,
    };
    let gift = gifts::insert(&state.pool, user.player_id, year, &description, photo.as_deref(), state.now()).await?;
    Ok((StatusCode::CREATED, Json(gift.into())))
}

pub async fn update(State(state): State<AppState>, user: CurrentUser, Path(id): Path<i64>, mp: Multipart) -> ApiResult<Json<GiftView>> {
    let existing = gifts::find_owned(&state.pool, id, user.player_id).await?.ok_or_else(|| ApiError::not_found("Cadeau introuvable."))?;
    let form = read_form(mp).await?;
    let (year, description) = validate(&form, state.current_year())?;
    let mut photo = existing.photo_path.clone();
    if let Some(bytes) = form.photo {
        photo = Some(save_photo(&state, bytes).await?);
    } else if form.remove_photo {
        photo = None;
    }
    if let Some(old) = &existing.photo_path {
        if photo.as_deref() != Some(old) {
            photos::remove(&state.config.uploads_dir, old).await;
        }
    }
    let gift = gifts::update(&state.pool, id, year, &description, photo.as_deref()).await?;
    Ok(Json(gift.into()))
}

pub async fn delete(State(state): State<AppState>, user: CurrentUser, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let existing = gifts::find_owned(&state.pool, id, user.player_id).await?.ok_or_else(|| ApiError::not_found("Cadeau introuvable."))?;
    gifts::delete(&state.pool, id).await?;
    if let Some(p) = &existing.photo_path {
        photos::remove(&state.config.uploads_dir, p).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn upload(State(state): State<AppState>, _user: CurrentUser, Path(file): Path<String>) -> ApiResult<Response> {
    let not_found = || ApiError::not_found("Photo introuvable.");
    if !photos::is_valid_name(&file) {
        return Err(not_found());
    }
    let bytes = tokio::fs::read(state.config.uploads_dir.join(&file)).await.map_err(|_| not_found())?;
    Response::builder()
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal(e.to_string()))
}
```

- [ ] **Step 8: Implémenter `src/routes/players.rs`**

```rust
use super::{extract::CurrentUser, gifts::GiftView};
use crate::{
    error::{ApiError, ApiResult},
    repo::{gifts, players::{self, PlayerRef}},
    state::AppState,
};
use axum::{Json, extract::{Path, State}};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub player_id: i64,
    pub pseudo: String,
    pub gifts: Vec<GiftView>,
}

pub async fn list(State(state): State<AppState>, _user: CurrentUser) -> ApiResult<Json<Vec<PlayerRef>>> {
    Ok(Json(players::list_visible(&state.pool).await?))
}

pub async fn profile(State(state): State<AppState>, _user: CurrentUser, Path(id): Path<i64>) -> ApiResult<Json<ProfileView>> {
    let player = players::find(&state.pool, id).await?.ok_or_else(|| ApiError::not_found("Joueur introuvable."))?;
    let gifts = gifts::list_for(&state.pool, id).await?.into_iter().map(GiftView::from).collect();
    Ok(Json(ProfileView { player_id: player.id, pseudo: player.pseudo, gifts }))
}
```

Dans `src/routes/mod.rs` : ajouter `pub mod gifts; pub mod players;` et les routes :

```rust
        .route("/players", get(players::list))
        .route("/players/{id}", get(players::profile))
        .route("/me/gifts", post(gifts::create))
        .route("/me/gifts/{id}", put(gifts::update).delete(gifts::delete))
        .route("/uploads/{file}", get(gifts::upload))
```

- [ ] **Step 9: Lancer les tests**

Run: `cargo test`
Expected: PASS (dont les 6 de `tests/gifts.rs`).

- [ ] **Step 10: Commit**

```bash
git add src tests
git commit -m "Profils, historique des cadeaux et photos

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Import de `players.txt`

**Files:**
- Create: `src/routes/import.rs`, `tests/import.rs`
- Modify: `src/routes/mod.rs`

**Interfaces:**
- Consumes: `legacy_import::{parse, SkippedLine}`, `players::upsert_imported`, `assignments::{insert, MailStatus::NotApplicable}`, `AdminUser`.
- Produces: `POST /api/admin/import` (multipart : `file`, `year`, `budgetEur` optionnel, défaut 30) → `201 { year, imported, skipped: [{ line, reason }] }` ; `409 EDITION_EXISTS` ; `422 { error: "NOTHING_TO_IMPORT", message, skipped }` ; `400 INVALID_FILE|INVALID_YEAR|INVALID_BUDGET`.

- [ ] **Step 1: Écrire les tests**

`tests/import.rs` :

```rust
mod common;

use axum::http::{Method, StatusCode};
use chrono::Duration;
use common::{Part, TestApp, text};
use secret_santa::clock::Clock;
use serde_json::json;

fn file(content: &str) -> Part {
    Part::File { filename: "players.txt".into(), content_type: "text/plain".into(), bytes: content.as_bytes().to_vec() }
}

const LAST_YEAR: &str = "Alice;alice@x.fr;Bob\nBob;bob@x.fr;Chloe\nChloe;chloe@x.fr;Alice\n";

#[tokio::test]
async fn import_puis_contrainte_appliquee_au_tirage_suivant() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let (status, report) = app
        .multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2025")), ("budgetEur", text("25")), ("file", file(LAST_YEAR))])
        .await;
    assert_eq!(status, StatusCode::CREATED, "{report}");
    assert_eq!(report, json!({ "year": 2025, "imported": 3, "skipped": [] }));

    // Les joueurs importés se connectent : ils sont reliés par leur adresse.
    let deadline = (app.clock.now() + Duration::days(10)).to_rfc3339();
    app.post("/api/admin/editions", &admin, json!({ "budgetEur": 30, "deadline": deadline })).await;
    let tokens: Vec<_> = ["Alice", "Bob", "Chloe"].iter().enumerate().map(|(i, p)| (p.to_string(), app.login(10 + i as i64, p, false))).collect();
    for (_, t) in &tokens {
        app.put("/api/editions/current/registration", t, json!({ "participating": true })).await;
    }
    let (status, _) = app.post("/api/admin/editions/current/draw", &admin, json!({})).await;
    assert_eq!(status, StatusCode::OK);

    // 2025 : A→B, B→C, C→A. Le seul cycle permis en 2026 est A→C→B→A.
    let expected = [("Alice", "Chloe"), ("Bob", "Alice"), ("Chloe", "Bob")];
    for (giver, receiver) in expected {
        let token = &tokens.iter().find(|(p, _)| p == giver).unwrap().1;
        let (_, target) = app.get("/api/editions/current/my-target", token).await;
        assert_eq!(target["pseudo"], receiver, "cible de {giver}");
    }
}

#[tokio::test]
async fn lignes_ignorees_rapportees() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let (status, report) = app
        .multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2024")), ("file", file("Alice;alice@x.fr;Bob\nBob;bob@x.fr\n"))])
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(report["imported"], 1);
    assert_eq!(report["skipped"], json!([{ "line": 2, "reason": "pas de cible" }]));
}

#[tokio::test]
async fn rien_d_importable_422_sans_rien_creer() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    let (status, body) = app.multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2024")), ("file", file("n'importe quoi\n"))]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "NOTHING_TO_IMPORT");
    assert_eq!(body["skipped"][0]["line"], 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM editions").fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn annee_deja_existante_409_et_annee_future_400() {
    let app = TestApp::new().await;
    let admin = app.login(1, "Nathan", true);
    app.multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2025")), ("file", file(LAST_YEAR))]).await;
    let (status, body) = app.multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2025")), ("file", file(LAST_YEAR))]).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("EDITION_EXISTS")));
    let (status, body) = app.multipart(Method::POST, "/api/admin/import", &admin, vec![("year", text("2027")), ("file", file(LAST_YEAR))]).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("INVALID_YEAR")));
}

#[tokio::test]
async fn reserve_a_l_admin() {
    let app = TestApp::new().await;
    let pote = app.login(2, "Bob", false);
    let (status, _) = app.multipart(Method::POST, "/api/admin/import", &pote, vec![("year", text("2025")), ("file", file(LAST_YEAR))]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
```

- [ ] **Step 2: Vérifier l'échec**

Run: `cargo test --test import`
Expected: FAIL (route absente → 404/405).

- [ ] **Step 3: Implémenter `src/routes/import.rs`**

```rust
use super::extract::AdminUser;
use crate::{
    error::{ApiError, ApiResult},
    legacy_import,
    repo::{assignments::{self, MailStatus}, editions, players},
    state::AppState,
};
use axum::{
    Json,
    extract::{Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub async fn import(State(state): State<AppState>, _admin: AdminUser, mut mp: Multipart) -> ApiResult<Response> {
    let bad = |e: axum::extract::multipart::MultipartError| ApiError::bad_request("INVALID_FORM", e.to_string());
    let (mut content, mut year, mut budget) = (None, None, None);
    while let Some(field) = mp.next_field().await.map_err(bad)? {
        match field.name().unwrap_or_default() {
            "file" => {
                let bytes = field.bytes().await.map_err(bad)?;
                content = Some(String::from_utf8(bytes.to_vec()).map_err(|_| ApiError::bad_request("INVALID_FILE", "Le fichier doit être encodé en UTF-8."))?);
            }
            "year" => year = Some(field.text().await.map_err(bad)?),
            "budgetEur" => budget = Some(field.text().await.map_err(bad)?),
            _ => {}
        }
    }

    let content = content.ok_or_else(|| ApiError::bad_request("INVALID_FILE", "Fichier manquant."))?;
    let current = state.current_year();
    let year: i64 = year
        .and_then(|y| y.trim().parse().ok())
        .filter(|y| (2000..=current).contains(y))
        .ok_or_else(|| ApiError::bad_request("INVALID_YEAR", format!("L'année doit être comprise entre 2000 et {current}.")))?;
    let budget: i64 = match budget.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        None => 30,
        Some(b) => b.parse().ok().filter(|b| (1..=1000).contains(b)).ok_or_else(|| ApiError::bad_request("INVALID_BUDGET", "Le budget doit être compris entre 1 et 1000 €."))?,
    };

    if editions::by_year(&state.pool, year).await?.is_some() {
        return Err(ApiError::conflict("EDITION_EXISTS", format!("L'édition {year} existe déjà.")));
    }

    let parsed = legacy_import::parse(&content);
    if parsed.pairs.is_empty() {
        let body = json!({ "error": "NOTHING_TO_IMPORT", "message": "Aucune paire valide dans ce fichier.", "skipped": parsed.skipped });
        return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response());
    }

    let now = state.now();
    let mut tx = state.pool.begin().await?;
    let edition_id: i64 = sqlx::query_scalar(
        "INSERT INTO editions (year, budget_eur, deadline, status, drawn_at, imported, created_at) VALUES (?, ?, ?, 'drawn', ?, 1, ?) RETURNING id",
    )
    .bind(year)
    .bind(budget)
    .bind(now)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    for pair in &parsed.pairs {
        let giver = players::upsert_imported(&mut tx, &pair.giver_email, &pair.giver_name, now).await?;
        let receiver = players::upsert_imported(&mut tx, &pair.receiver_email, &pair.receiver_name, now).await?;
        assignments::insert(&mut tx, edition_id, giver, receiver, MailStatus::NotApplicable).await?;
    }
    tx.commit().await?;

    tracing::info!(year, imported = parsed.pairs.len(), skipped = parsed.skipped.len(), "historique importé");
    let body = json!({ "year": year, "imported": parsed.pairs.len(), "skipped": parsed.skipped });
    Ok((StatusCode::CREATED, Json(body)).into_response())
}
```

Dans `src/routes/mod.rs` : ajouter `pub mod import;` et `.route("/admin/import", post(import::import))`.

- [ ] **Step 4: Lancer les tests**

Run: `cargo test`
Expected: PASS (dont les 5 de `tests/import.rs`).

- [ ] **Step 5: Commit**

```bash
git add src tests
git commit -m "Import de l'historique players.txt de la version Java

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 13: Démarrage complet, Docker, Komodo, README

**Files:**
- Modify: `src/main.rs`
- Create: `Dockerfile`, `.dockerignore`, `compose.yaml`, `.env.example`, `README.md`

**Interfaces:**
- Consumes: tout ce qui précède.
- Produces: image Docker `secret-santa-backend`, service Compose `secret-santa-backend` sur le réseau `traefik`, routé par `Host(santa.limperiam.com) && PathPrefix(/api)`.

- [ ] **Step 1: Réécrire `src/main.rs`**

```rust
use secret_santa::{
    auth::{AuthClient, DevAuthClient, HttpAuthClient},
    clock::SystemClock,
    config::Config,
    db,
    mail::{LogMailer, Mailer, SmtpMailer},
    routes,
    services::{draw_service, scheduler},
    state::AppState,
};
use std::{process::ExitCode, sync::Arc, time::Duration};
use tokio_util::task::TaskTracker;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("configuration invalide : {e}");
            return ExitCode::FAILURE;
        }
    };

    let pool = match db::connect(&config.database_path).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("base de données inaccessible : {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = tokio::fs::create_dir_all(&config.uploads_dir).await {
        tracing::error!("dossier des photos inaccessible : {e}");
        return ExitCode::FAILURE;
    }

    let auth: Arc<dyn AuthClient> = match &config.dev_user {
        Some(user) => {
            tracing::warn!("AUTH_DEV_USER actif : le SSO est court-circuité (développement uniquement)");
            Arc::new(DevAuthClient::new(user.clone()))
        }
        None => Arc::new(HttpAuthClient::new(config.auth_url.clone(), Duration::from_secs(60))),
    };

    let mailer: Arc<dyn Mailer> = match (&config.smtp, config.dev_mode) {
        (Some(smtp), false) => match SmtpMailer::new(smtp) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                tracing::error!("configuration SMTP invalide : {e}");
                return ExitCode::FAILURE;
            }
        },
        _ => {
            tracing::warn!("mode développement : les mails sont journalisés, pas envoyés");
            Arc::new(LogMailer)
        }
    };

    let bind_addr = config.bind_addr.clone();
    let state = AppState {
        pool,
        config: Arc::new(config),
        auth,
        mailer,
        clock: Arc::new(SystemClock),
        tasks: TaskTracker::new(),
    };

    draw_service::resume_pending_mails(&state).await;
    scheduler::spawn(state.clone());

    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("impossible d'écouter sur {bind_addr} : {e}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(addr = %bind_addr, "Secret Santa à l'écoute");

    if let Err(e) = axum::serve(listener, routes::router(state.clone())).with_graceful_shutdown(shutdown_signal()).await {
        tracing::error!("serveur arrêté sur une erreur : {e}");
        return ExitCode::FAILURE;
    }

    // Laisser finir une vague de mails en cours ; au pire, ils repartiront au
    // prochain démarrage (statut `pending`).
    state.tasks.close();
    if tokio::time::timeout(Duration::from_secs(30), state.tasks.wait()).await.is_err() {
        tracing::warn!("arrêt avant la fin des envois : ils reprendront au prochain démarrage");
    }
    ExitCode::SUCCESS
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
```

Run: `cargo build && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: build OK, tous les tests PASS, clippy sans avertissement (corriger ce qu'il signale).

- [ ] **Step 2: Vérifier le démarrage en local**

```bash
APP_ENV=development ADMIN_EMAIL=admin@example.com AUTH_DEV_USER='admin@example.com,Nathan,admin' \
DATABASE_PATH=./data/santa.db UPLOADS_DIR=./data/uploads BIND_ADDR=127.0.0.1:8080 PUBLIC_ORIGIN=http://localhost:5173 \
MAIL_DELAY_MS=0 cargo run
```

Dans un second terminal :

```bash
curl -s http://127.0.0.1:8080/api/health
curl -s http://127.0.0.1:8080/api/me
curl -s -X POST http://127.0.0.1:8080/api/admin/editions -H 'Origin: http://localhost:5173' -H 'Content-Type: application/json' -d '{"budgetEur":30,"deadline":"2026-12-20T18:00:00Z"}'
```

Expected : `ok` ; un JSON `{"playerId":1,"pseudo":"Nathan",...,"isAdmin":true}` ; un JSON d'édition `"status":"open"` et, dans les logs du serveur, le mail d'ouverture journalisé. Arrêter avec Ctrl+C : le serveur s'arrête proprement. Supprimer ensuite `./data`.

- [ ] **Step 3: Dockerfile et `.dockerignore`**

`Dockerfile` :

```dockerfile
FROM rust:1.96-slim-bookworm AS build
WORKDIR /app
# Couche de dépendances : recompilée seulement quand Cargo.toml / Cargo.lock changent.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && touch src/lib.rs \
    && cargo build --release && rm -rf src
COPY migrations migrations
COPY src src
RUN touch src/main.rs src/lib.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /data santa \
    && mkdir -p /data/uploads && chown -R santa /data
COPY --from=build /app/target/release/secret_santa /usr/local/bin/secret_santa
USER santa
ENV DATABASE_PATH=/data/santa.db \
    UPLOADS_DIR=/data/uploads \
    BIND_ADDR=0.0.0.0:8080 \
    TZ=Europe/Paris
EXPOSE 8080
CMD ["secret_santa"]
```

`.dockerignore` :

```
target
data
.env
docs
tests
.git
```

- [ ] **Step 4: `compose.yaml` et `.env.example`**

`compose.yaml` :

```yaml
services:
  secret-santa-backend:
    build: .
    image: secret-santa-backend:latest
    # Pas de container_name : Komodo nomme les conteneurs d'après le Stack.
    restart: unless-stopped

    networks:
      - default
      - traefik

    # Pas de `ports:` : Traefik joint le conteneur par le réseau `traefik`.
    # Même hôte que le frontend : seul `/api` arrive ici, d'où la priorité
    # explicite, supérieure à celle du routeur du frontend.
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.secret-santa-backend.rule=Host(`${SANTA_HOST:-santa.limperiam.com}`) && PathPrefix(`/api`)"
      - "traefik.http.routers.secret-santa-backend.entrypoints=websecure"
      # tls=true sans certresolver : le résolveur par défaut est global à Traefik.
      - "traefik.http.routers.secret-santa-backend.tls=true"
      - "traefik.http.routers.secret-santa-backend.priority=100"
      - "traefik.http.services.secret-santa-backend.loadbalancer.server.port=8080"

    environment:
      # Destinataire des mails d'échec du tirage automatique.
      ADMIN_EMAIL: ${ADMIN_EMAIL:?adresse de l'administrateur}
      # Sans SMTP, aucun tirage n'arriverait à personne : refus de démarrer.
      SMTP_HOST: ${SMTP_HOST:?hôte du serveur SMTP}
      SMTP_PORT: ${SMTP_PORT:-587}
      SMTP_USER: ${SMTP_USER:-}
      SMTP_PASS: ${SMTP_PASS:-}
      # Doit être un expéditeur autorisé par le fournisseur SMTP.
      SMTP_FROM: ${SMTP_FROM:?expéditeur, ex. Secret Santa <santa@limperiam.com>}
      AUTH_URL: ${AUTH_URL:-http://limperiam-auth:3000}
      AUTH_LOGIN_URL: ${AUTH_LOGIN_URL:-https://auth.limperiam.com/login}
      PUBLIC_ORIGIN: ${PUBLIC_ORIGIN:-https://santa.limperiam.com}
      TZ: ${TZ:-Europe/Paris}
      RUST_LOG: ${RUST_LOG:-info}

    volumes:
      # Base SQLite + photos. Sans ce volume, tout disparaît au redéploiement.
      - secret-santa-data:/data

    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://127.0.0.1:8080/api/health"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s

networks:
  default:
  traefik:
    external: true
    name: traefik

volumes:
  secret-santa-data:
```

`.env.example` :

```
ADMIN_EMAIL=admin@example.com
SMTP_HOST=smtp.example.com
SMTP_PORT=587
SMTP_USER=
SMTP_PASS=
SMTP_FROM=Secret Santa <santa@limperiam.com>
```

Run: `docker build -t secret-santa-backend:test .`
Expected : image construite sans erreur.

Run: `docker run --rm -e APP_ENV=development -e ADMIN_EMAIL=a@x.fr -e AUTH_DEV_USER='a@x.fr,A,admin' -p 18080:8080 secret-santa-backend:test` puis, dans un autre terminal, `curl -s http://127.0.0.1:18080/api/health`
Expected : `ok`. Arrêter le conteneur (Ctrl+C) : arrêt propre en moins de 2 s.

- [ ] **Step 5: README**

`README.md` :

````markdown
# secret_santa_rs

Backend du Secret Santa de `santa.limperiam.com` : inscriptions avec deadline,
tirage automatique, mails de résultat, profils et historique des cadeaux.
Remplace `secret_santa_v2` (Java, fichier `players.txt`).

Design : [`docs/superpowers/specs/2026-09-29-secret-santa-design.md`](docs/superpowers/specs/2026-09-29-secret-santa-design.md)
Frontend : `frontend/secret-santa` (stack Komodo séparée, même domaine).

## Développement local

```bash
APP_ENV=development \
ADMIN_EMAIL=ton@email.fr \
AUTH_DEV_USER='ton@email.fr,Nathan,admin' \
DATABASE_PATH=./data/santa.db UPLOADS_DIR=./data/uploads \
BIND_ADDR=127.0.0.1:8080 PUBLIC_ORIGIN=http://localhost:5173 MAIL_DELAY_MS=0 \
cargo run
```

`AUTH_DEV_USER` (`email,pseudo[,admin]`) court-circuite le SSO ; le serveur
refuse de démarrer s'il est défini sans `APP_ENV=development`. En
développement, les mails sont écrits dans les logs au lieu de partir.
`PUBLIC_ORIGIN` doit valoir l'origine du front (Vite : `http://localhost:5173`),
sinon les écritures sont refusées (contrôle `Origin`).

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

## Variables d'environnement

| Variable | Obligatoire | Défaut | Rôle |
|---|---|---|---|
| `ADMIN_EMAIL` | **oui** | — | Reçoit le mail d'échec du tirage automatique |
| `SMTP_HOST` | **oui** (hors dev) | — | Serveur SMTP |
| `SMTP_FROM` | **oui** avec `SMTP_HOST` | — | Expéditeur autorisé par le fournisseur |
| `SMTP_PORT` | non | `587` | `465` = TLS implicite, sinon STARTTLS |
| `SMTP_USER` / `SMTP_PASS` | non | — | |
| `AUTH_URL` | non | `http://limperiam-auth:3000` | SSO, par le réseau Docker |
| `AUTH_LOGIN_URL` | non | `https://auth.limperiam.com/login` | Renvoyé au front sur 401 |
| `PUBLIC_ORIGIN` | non | `https://santa.limperiam.com` | Contrôle `Origin`, liens des mails |
| `DATABASE_PATH` | non | `/data/santa.db` | |
| `UPLOADS_DIR` | non | `/data/uploads` | Photos des cadeaux |
| `BIND_ADDR` | non | `0.0.0.0:8080` | |
| `MAIL_DELAY_MS` | non | `1000` | Pause entre deux mails d'une vague |
| `APP_ENV` | non | `production` | `development` : mails journalisés, `AUTH_DEV_USER` permis |
| `AUTH_DEV_USER` | non | — | Dev uniquement |
| `RUST_LOG` | non | `info` | |

## Mise en service

1. Déployer `limperiam-auth` avec la route `GET /api/admin/users`, puis dans
   `https://auth.limperiam.com/admin` créer l'application `santa`
   (URL `https://santa.limperiam.com`), ouverte aux groupes `potes` et `admin`.
2. Komodo : Stack `secret_santa_backend` sur ce dépôt (`compose.yaml`), avec
   `ADMIN_EMAIL`, `SMTP_HOST`, `SMTP_FROM` (+ `SMTP_USER`/`SMTP_PASS`).
   Déployer ; vérifier `https://santa.limperiam.com/api/health` → `ok`.
3. Déployer le Stack `secret_santa_frontend`.
4. Dans l'admin du site : importer le `players.txt` de l'an dernier.
5. Créer l'édition de l'année (budget + deadline) : l'annonce part à tous.

## Fonctionnement

- **Tirage automatique** : une boucle vérifie chaque minute les éditions dont
  la deadline est passée. L'état est en base : si le conteneur était arrêté à
  la deadline, le tirage part au redémarrage. La transition vers `drawn` est
  conditionnelle : impossible de tirer deux fois.
- **Relance** : 48 h avant la deadline, une fois, aux comptes sans réponse.
- **Échec** (moins de 3 inscrits, contrainte de l'an dernier insoluble) :
  l'édition passe en `failed`, l'admin reçoit un mail ; repousser la deadline
  la rouvre.
- **Mails** : un par seconde, en tâche de fond. Un arrêt en pleine vague laisse
  des envois `pending`, renvoyés au démarrage suivant.
- **Secret** : aucune route ne révèle les paires, admin compris ; chacun ne
  voit que sa propre cible.
- **Sauvegarde** : tout est dans le volume `secret-santa-data` (`santa.db` +
  `uploads/`).
````

- [ ] **Step 6: Commit**

```bash
git add src Dockerfile .dockerignore compose.yaml .env.example README.md
git commit -m "Démarrage complet, image Docker et stack Komodo

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
