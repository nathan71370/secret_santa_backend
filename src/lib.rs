pub mod auth;
pub mod clock;
pub mod config;
pub mod db;
pub mod draw;
pub mod error;
pub mod legacy_import;
pub mod mail;
pub mod repo;
pub mod routes;
pub mod state;

/// Slug de l'application dans le catalogue de limperiam-auth.
pub const APP_SLUG: &str = "santa";
