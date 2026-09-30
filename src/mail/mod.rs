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
            (Some(user), Some(pass)) => {
                builder.credentials(Credentials::new(user.clone(), pass.clone()))
            }
            _ => builder,
        };
        let from = cfg.from.parse::<Mailbox>().map_err(|e| err(&e))?;
        Ok(Self {
            transport: builder.build(),
            from,
        })
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, mail: &Mail) -> Result<(), MailError> {
        let to = mail
            .to
            .parse::<Mailbox>()
            .map_err(|e| MailError(e.to_string()))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(&mail.subject)
            .multipart(MultiPart::alternative_plain_html(
                mail.text.clone(),
                mail.html.clone(),
            ))
            .map_err(|e| MailError(e.to_string()))?;
        self.transport
            .send(message)
            .await
            .map(|_| ())
            .map_err(|e| MailError(e.to_string()))
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
