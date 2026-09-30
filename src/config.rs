use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
    pub from: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevUser {
    pub email: String,
    pub pseudo: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub database_path: String,
    pub uploads_dir: PathBuf,
    pub auth_url: String,
    pub auth_login_url: String,
    pub public_origin: String,
    pub admin_email: String,
    /// `None` seulement en développement : les mails sont alors journalisés.
    pub smtp: Option<SmtpConfig>,
    pub dev_mode: bool,
    pub dev_user: Option<DevUser>,
    /// Pause entre deux mails d'une même vague (1 s, comme le Java).
    pub mail_delay: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("la variable {0} est obligatoire")]
    Missing(&'static str),
    #[error("la variable {0} est invalide : {1}")]
    Invalid(&'static str, String),
    #[error("AUTH_DEV_USER court-circuite le SSO : interdit sans APP_ENV=development")]
    DevUserInProduction,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let var = |k: &str| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let url = |k: &str, default: &str| {
            var(k)
                .unwrap_or_else(|| default.to_string())
                .trim_end_matches('/')
                .to_string()
        };

        let dev_mode = var("APP_ENV").as_deref() == Some("development");

        let dev_user = match var("AUTH_DEV_USER") {
            None => None,
            Some(_) if !dev_mode => return Err(ConfigError::DevUserInProduction),
            Some(raw) => Some(parse_dev_user(&raw)?),
        };

        let admin_email = var("ADMIN_EMAIL")
            .ok_or(ConfigError::Missing("ADMIN_EMAIL"))?
            .to_lowercase();

        let smtp = match var("SMTP_HOST") {
            Some(host) => {
                let port = match var("SMTP_PORT") {
                    None => 587,
                    Some(p) => p
                        .parse()
                        .map_err(|_| ConfigError::Invalid("SMTP_PORT", p.clone()))?,
                };
                let from = var("SMTP_FROM").ok_or(ConfigError::Missing("SMTP_FROM"))?;
                Some(SmtpConfig {
                    host,
                    port,
                    user: var("SMTP_USER"),
                    pass: var("SMTP_PASS"),
                    from,
                })
            }
            None if dev_mode => None,
            None => return Err(ConfigError::Missing("SMTP_HOST")),
        };

        let mail_delay_ms: u64 = match var("MAIL_DELAY_MS") {
            None => 1000,
            Some(v) => v
                .parse()
                .map_err(|_| ConfigError::Invalid("MAIL_DELAY_MS", v.clone()))?,
        };

        Ok(Config {
            bind_addr: var("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into()),
            database_path: var("DATABASE_PATH").unwrap_or_else(|| "/data/santa.db".into()),
            uploads_dir: var("UPLOADS_DIR")
                .unwrap_or_else(|| "/data/uploads".into())
                .into(),
            auth_url: url("AUTH_URL", "http://limperiam-auth:3000"),
            auth_login_url: url("AUTH_LOGIN_URL", "https://auth.limperiam.com/login"),
            public_origin: url("PUBLIC_ORIGIN", "https://santa.limperiam.com"),
            admin_email,
            smtp,
            dev_mode,
            dev_user,
            mail_delay: Duration::from_millis(mail_delay_ms),
        })
    }
}

fn parse_dev_user(raw: &str) -> Result<DevUser, ConfigError> {
    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    let invalid = || ConfigError::Invalid("AUTH_DEV_USER", "attendu : email,pseudo[,admin]".into());
    let (email, pseudo, is_admin) = match parts.as_slice() {
        [email, pseudo] => (*email, *pseudo, false),
        [email, pseudo, "admin"] => (*email, *pseudo, true),
        _ => return Err(invalid()),
    };
    if !email.contains('@') || pseudo.is_empty() {
        return Err(invalid());
    }
    Ok(DevUser {
        email: email.to_lowercase(),
        pseudo: pseudo.to_string(),
        is_admin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    const PROD: &[(&str, &str)] = &[
        ("ADMIN_EMAIL", "Nathan@X.fr"),
        ("SMTP_HOST", "smtp.x.fr"),
        ("SMTP_FROM", "Santa <santa@x.fr>"),
    ];

    #[test]
    fn valeurs_par_defaut_en_production() {
        let c = Config::from_lookup(lookup(PROD)).unwrap();
        assert_eq!(c.bind_addr, "0.0.0.0:8080");
        assert_eq!(c.database_path, "/data/santa.db");
        assert_eq!(c.uploads_dir, std::path::PathBuf::from("/data/uploads"));
        assert_eq!(c.auth_url, "http://limperiam-auth:3000");
        assert_eq!(c.auth_login_url, "https://auth.limperiam.com/login");
        assert_eq!(c.public_origin, "https://santa.limperiam.com");
        assert_eq!(c.admin_email, "nathan@x.fr");
        assert_eq!(c.mail_delay, std::time::Duration::from_millis(1000));
        assert!(!c.dev_mode);
        let smtp = c.smtp.unwrap();
        assert_eq!(smtp.port, 587);
        assert_eq!(smtp.user, None);
    }

    #[test]
    fn smtp_host_obligatoire_en_production() {
        let err = Config::from_lookup(lookup(&[
            ("ADMIN_EMAIL", "a@x.fr"),
            ("SMTP_FROM", "a@x.fr"),
        ]))
        .unwrap_err();
        assert_eq!(err, ConfigError::Missing("SMTP_HOST"));
    }

    #[test]
    fn admin_email_obligatoire() {
        let err = Config::from_lookup(lookup(&[("SMTP_HOST", "h"), ("SMTP_FROM", "a@x.fr")]))
            .unwrap_err();
        assert_eq!(err, ConfigError::Missing("ADMIN_EMAIL"));
    }

    #[test]
    fn une_variable_vide_compte_comme_absente() {
        let err = Config::from_lookup(lookup(&[
            ("ADMIN_EMAIL", "a@x.fr"),
            ("SMTP_HOST", "  "),
            ("SMTP_FROM", "a@x.fr"),
        ]))
        .unwrap_err();
        assert_eq!(err, ConfigError::Missing("SMTP_HOST"));
    }

    #[test]
    fn developpement_sans_smtp() {
        let c = Config::from_lookup(lookup(&[
            ("ADMIN_EMAIL", "a@x.fr"),
            ("APP_ENV", "development"),
        ]))
        .unwrap();
        assert!(c.dev_mode);
        assert!(c.smtp.is_none());
    }

    #[test]
    fn utilisateur_de_dev_interdit_en_production() {
        let mut env = PROD.to_vec();
        env.push(("AUTH_DEV_USER", "a@x.fr,A,admin"));
        assert_eq!(
            Config::from_lookup(lookup(&env)).unwrap_err(),
            ConfigError::DevUserInProduction
        );
    }

    #[test]
    fn utilisateur_de_dev_lu_en_developpement() {
        let c = Config::from_lookup(lookup(&[
            ("ADMIN_EMAIL", "a@x.fr"),
            ("APP_ENV", "development"),
            ("AUTH_DEV_USER", " Moi@X.fr , Moi , admin "),
        ]))
        .unwrap();
        assert_eq!(
            c.dev_user,
            Some(DevUser {
                email: "moi@x.fr".into(),
                pseudo: "Moi".into(),
                is_admin: true
            })
        );

        let c = Config::from_lookup(lookup(&[
            ("ADMIN_EMAIL", "a@x.fr"),
            ("APP_ENV", "development"),
            ("AUTH_DEV_USER", "pote@x.fr,Pote"),
        ]))
        .unwrap();
        assert!(!c.dev_user.unwrap().is_admin);
    }

    #[test]
    fn port_smtp_invalide() {
        let mut env = PROD.to_vec();
        env.push(("SMTP_PORT", "abc"));
        assert!(matches!(
            Config::from_lookup(lookup(&env)).unwrap_err(),
            ConfigError::Invalid("SMTP_PORT", _)
        ));
    }

    #[test]
    fn slash_final_retire_des_urls() {
        let mut env = PROD.to_vec();
        env.push(("PUBLIC_ORIGIN", "http://localhost:5173/"));
        env.push(("AUTH_URL", "http://sso:3000/"));
        let c = Config::from_lookup(lookup(&env)).unwrap();
        assert_eq!(c.public_origin, "http://localhost:5173");
        assert_eq!(c.auth_url, "http://sso:3000");
    }
}
