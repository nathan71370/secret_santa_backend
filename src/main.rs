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
}
