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
