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
