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
