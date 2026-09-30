//! Boucle de fond : chaque minute, tirer les éditions dont la deadline est
//! passée et relancer les retardataires. L'état vit en base : un conteneur
//! arrêté à la deadline tire au premier tour après son redémarrage.

use crate::{
    error::ApiResult,
    repo::editions,
    services::{
        draw_service::{self, DrawOutcome},
        mailing,
    },
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
                DrawOutcome::Failed(reason) => {
                    tracing::warn!(year = e.year, %reason, "tirage automatique impossible")
                }
            }
            continue;
        }
        let remind_at = e.deadline - reminder_before();
        let created_early_enough = e.created_at <= remind_at;
        if e.reminder_sent_at.is_none()
            && created_early_enough
            && now >= remind_at
            && editions::mark_reminder_sent(&state.pool, e.id, now).await?
        {
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
