//! Envois en tâche de fond. Une vague de mails prend une seconde par
//! destinataire (comme la version Java) : jamais dans le cycle d'une requête.

use crate::{
    error::ApiError,
    mail::{Mail, templates},
    repo::{
        assignments::{self, MailStatus},
        editions::{self, Edition},
        players,
    },
    state::AppState,
};

/// Journalise l'échec sans le texte de l'erreur : celui-ci peut reprendre l'adresse du destinataire.
pub async fn send_logged(state: &AppState, mail: &Mail, kind: &str) -> bool {
    match state.mailer.send(mail).await {
        Ok(()) => true,
        Err(_) => {
            tracing::warn!(kind, "échec d'envoi d'un mail");
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
            tracing::warn!("échec d'envoi d'un résultat");
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
