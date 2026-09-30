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
    // Verrou d'écriture pris dès l'ouverture : deux tirages concurrents
    // s'attendent (busy_timeout) au lieu de s'invalider en cours de route.
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;

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
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
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

/// Au démarrage : un arrêt pendant une vague d'envois laisse des `pending`,
/// et des `sending` si l'arrêt a coupé un envoi — ceux-là repartent aussi.
pub async fn resume_pending_mails(state: &AppState) {
    if let Err(e) = assignments::reset_sending(&state.pool).await {
        tracing::error!(error = %e, "remise en attente des envois interrompus impossible");
    }
    match assignments::editions_with_pending(&state.pool).await {
        Ok(ids) => {
            for id in ids {
                state.spawn(mailing::send_results(state.clone(), id));
            }
        }
        Err(e) => tracing::error!(error = %e, "lecture des mails en attente impossible"),
    }
}
