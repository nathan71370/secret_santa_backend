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
