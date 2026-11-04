use crate::auth::SsoUser;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Player {
    pub id: i64,
    pub email: String,
    pub sso_user_id: Option<i64>,
    pub pseudo: String,
    pub has_access: bool,
    /// Relatif à l'origine publique du SSO (`/avatars/3.jpg`).
    pub avatar_path: Option<String>,
}

impl Player {
    pub fn avatar_url(&self, auth_public_url: &str) -> Option<String> {
        avatar_url(auth_public_url, self.avatar_path.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRef {
    pub player_id: i64,
    pub pseudo: String,
    /// URL absolue de la photo, servie par le SSO au navigateur connecté.
    /// Les requêtes lisent ici `avatar_path`, que `resolve_avatars` complète.
    pub avatar_url: Option<String>,
}

pub const COLS: &str = "id, email, sso_user_id, pseudo, has_access, avatar_path";

/// Colonnes d'un `PlayerRef` sur l'alias `p` : à passer ensuite par `resolve_avatars`.
pub const REF_COLS: &str = "p.id AS player_id, p.pseudo, p.avatar_path AS avatar_url";

/// Seul un chemin relatif à la racine est accepté : la photo doit rester sur
/// l'origine du SSO, jamais pointer vers un autre hôte (`//`, `https:`…).
fn clean_avatar_path(path: Option<&str>) -> Option<&str> {
    path.map(str::trim)
        .filter(|p| p.starts_with('/') && !p.starts_with("//") && !p.contains('\\'))
}

pub fn avatar_url(auth_public_url: &str, path: Option<&str>) -> Option<String> {
    clean_avatar_path(path).map(|p| format!("{}{p}", auth_public_url.trim_end_matches('/')))
}

/// Remplace le chemin lu par `REF_COLS` par l'URL absolue.
pub fn resolve_avatars(mut refs: Vec<PlayerRef>, auth_public_url: &str) -> Vec<PlayerRef> {
    for r in &mut refs {
        r.avatar_url = avatar_url(auth_public_url, r.avatar_url.as_deref());
    }
    refs
}

/// Relie un compte SSO à son joueur : par `sso_user_id` s'il est déjà connu,
/// sinon par l'adresse (joueur importé de `players.txt`), sinon création.
///
/// Renvoie `None` si l'adresse appartient déjà à un joueur relié à un AUTRE
/// compte SSO (adresse recyclée) : on ne réattribue jamais les inscriptions,
/// cadeaux et tirages d'un joueur à un nouveau compte.
pub async fn upsert_sso(
    conn: &mut SqliteConnection,
    sso_user_id: i64,
    email: &str,
    pseudo: &str,
    avatar_path: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Option<i64>, sqlx::Error> {
    let email = email.trim().to_lowercase();
    // Pas de photo dans le SSO (ou retirée) : NULL, les initiales reprennent.
    let avatar_path = clean_avatar_path(avatar_path);
    let known: Option<i64> = sqlx::query_scalar("SELECT id FROM players WHERE sso_user_id = ?")
        .bind(sso_user_id)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(id) = known {
        sqlx::query("UPDATE players SET pseudo = ?, avatar_path = ?, has_access = 1, updated_at = ? WHERE id = ?")
            .bind(pseudo)
            .bind(avatar_path)
            .bind(now)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        // Adresse changée dans le SSO : suivie, sauf si un autre joueur la porte déjà.
        sqlx::query("UPDATE players SET email = ? WHERE id = ? AND NOT EXISTS (SELECT 1 FROM players WHERE email = ? AND id <> ?)")
            .bind(&email)
            .bind(id)
            .bind(&email)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        return Ok(Some(id));
    }
    sqlx::query_scalar(
        "INSERT INTO players (email, sso_user_id, pseudo, avatar_path, has_access, updated_at) VALUES (?, ?, ?, ?, 1, ?)
         ON CONFLICT (email) DO UPDATE SET sso_user_id = excluded.sso_user_id, pseudo = excluded.pseudo,
             avatar_path = excluded.avatar_path, has_access = 1, updated_at = excluded.updated_at
         WHERE players.sso_user_id IS NULL OR players.sso_user_id = excluded.sso_user_id
         RETURNING id",
    )
    .bind(&email)
    .bind(sso_user_id)
    .bind(pseudo)
    .bind(avatar_path)
    .bind(now)
    .fetch_optional(&mut *conn)
    .await
}

/// Instantané de l'annuaire SSO : seuls les comptes listés gardent l'accès.
pub async fn sync_access(
    pool: &SqlitePool,
    users: &[SsoUser],
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE players SET has_access = 0")
        .execute(&mut *tx)
        .await?;
    for u in users {
        if upsert_sso(
            &mut tx,
            u.user_id,
            &u.email,
            &u.pseudo,
            u.avatar_path.as_deref(),
            now,
        )
        .await?
        .is_none()
        {
            tracing::warn!(
                sso_user_id = u.user_id,
                "adresse déjà liée à un autre compte SSO : compte ignoré"
            );
        }
    }
    tx.commit().await
}

pub async fn find(pool: &SqlitePool, id: i64) -> Result<Option<Player>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS} FROM players WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn with_access(pool: &SqlitePool) -> Result<Vec<Player>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLS} FROM players WHERE has_access = 1 ORDER BY lower(pseudo)"
    ))
    .fetch_all(pool)
    .await
}

/// Joueur venu de `players.txt` : créé sans compte SSO s'il est inconnu, le
/// pseudo existant n'est jamais écrasé.
pub async fn upsert_imported(
    conn: &mut SqliteConnection,
    email: &str,
    pseudo: &str,
    now: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO players (email, pseudo, has_access, updated_at) VALUES (?, ?, 0, ?)
         ON CONFLICT (email) DO UPDATE SET email = excluded.email
         RETURNING id",
    )
    .bind(email.trim().to_lowercase())
    .bind(pseudo)
    .bind(now)
    .fetch_one(&mut *conn)
    .await
}

/// Joueurs à montrer dans « Participants » : inscrits à une édition, présents
/// dans un tirage, ou ayant un cadeau dans leur historique.
pub async fn list_visible(
    pool: &SqlitePool,
    auth_public_url: &str,
) -> Result<Vec<PlayerRef>, sqlx::Error> {
    let refs = sqlx::query_as(&format!(
        "SELECT {REF_COLS} FROM players p
         WHERE EXISTS (SELECT 1 FROM registrations r WHERE r.player_id = p.id AND r.participating = 1)
            OR EXISTS (SELECT 1 FROM assignments a WHERE a.giver_id = p.id OR a.receiver_id = p.id)
            OR EXISTS (SELECT 1 FROM gifts g WHERE g.player_id = p.id)
         ORDER BY lower(p.pseudo)",
    ))
    .fetch_all(pool)
    .await?;
    Ok(resolve_avatars(refs, auth_public_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_absolue_seulement_pour_un_chemin_relatif() {
        let base = "https://auth.test/";
        assert_eq!(
            avatar_url(base, Some("/avatars/3.jpg")).as_deref(),
            Some("https://auth.test/avatars/3.jpg")
        );
        assert_eq!(avatar_url(base, None), None);
        assert_eq!(avatar_url(base, Some("")), None);
        assert_eq!(avatar_url(base, Some("//evil.test/x.jpg")), None);
        assert_eq!(avatar_url(base, Some("https://evil.test/x.jpg")), None);
        assert_eq!(avatar_url(base, Some("avatars/3.jpg")), None);
    }
}
