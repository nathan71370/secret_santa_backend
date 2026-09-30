use super::Mail;
use chrono::{DateTime, Utc};
use chrono_tz::Europe::Paris;

pub fn paris(at: DateTime<Utc>) -> String {
    at.with_timezone(&Paris)
        .format("%d/%m/%Y à %Hh%M")
        .to_string()
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn build(to: &str, subject: String, lines: &[String], link_label: &str, link: String) -> Mail {
    let text = format!("{}\n\n{link_label} : {link}\n", lines.join("\n\n"));
    let paragraphs: String = lines
        .iter()
        .map(|l| format!(r#"<p style="margin:0 0 16px">{}</p>"#, escape(l)))
        .collect();
    let html = format!(
        r#"<!doctype html><html lang="fr"><body style="margin:0;background:#0b1220;padding:32px 16px;font-family:Georgia,'Times New Roman',serif"><div style="max-width:520px;margin:0 auto;background:#fffaf0;border-radius:18px;padding:32px;color:#1d2433;font-size:16px;line-height:1.55"><p style="margin:0 0 20px;font-size:30px">🎄</p>{paragraphs}<p style="margin:28px 0 0"><a href="{href}" style="display:inline-block;background:#b3261e;color:#fffaf0;text-decoration:none;padding:12px 22px;border-radius:999px;font-family:Arial,sans-serif">{label}</a></p></div></body></html>"#,
        href = escape(&link),
        label = escape(link_label),
    );
    Mail {
        to: to.to_string(),
        subject,
        text,
        html,
    }
}

pub fn announcement(
    to: &str,
    pseudo: &str,
    year: i64,
    budget: i64,
    deadline: DateTime<Utc>,
    origin: &str,
) -> Mail {
    build(
        to,
        format!("🎄 Secret Santa {year} : les inscriptions sont ouvertes"),
        &[
            format!("Bonjour {pseudo},"),
            format!("Le Secret Santa {year} est lancé ! Budget : {budget} € maximum."),
            format!(
                "Dis-nous si tu participes avant le {}. Sans réponse, tu ne participes pas.",
                paris(deadline)
            ),
            "Joyeuses fêtes ! 🎅".into(),
        ],
        "Je réponds",
        format!("{origin}/"),
    )
}

pub fn reminder(to: &str, pseudo: &str, year: i64, deadline: DateTime<Utc>, origin: &str) -> Mail {
    build(
        to,
        format!("⏰ Secret Santa {year} : plus que 48 h pour t'inscrire"),
        &[
            format!("Bonjour {pseudo},"),
            format!("Tu n'as pas encore dit si tu participes au Secret Santa {year}."),
            format!(
                "Les inscriptions ferment le {}. Sans réponse, tu ne participes pas.",
                paris(deadline)
            ),
        ],
        "Je réponds",
        format!("{origin}/"),
    )
}

pub fn result(
    to: &str,
    giver_pseudo: &str,
    receiver_pseudo: &str,
    receiver_id: i64,
    year: i64,
    budget: i64,
    origin: &str,
) -> Mail {
    build(
        to,
        format!("🎁 Secret Santa {year} : ton tirage !"),
        &[
            format!("Bonjour {giver_pseudo},"),
            format!("Pour ce Secret Santa, tu offres un cadeau à : {receiver_pseudo} !"),
            format!("Pour rappel, le budget est de {budget} € maximum."),
            "Joyeuses fêtes ! 🎅".into(),
        ],
        "Voir son profil",
        format!("{origin}/joueurs/{receiver_id}"),
    )
}

pub fn draw_failed(to: &str, year: i64, reason: &str, origin: &str) -> Mail {
    build(
        to,
        format!("⚠️ Secret Santa {year} : le tirage a échoué"),
        &[
            "Le tirage automatique n'a pas pu se faire.".into(),
            format!("Raison : {reason}"),
            "Repousse la deadline ou ajuste les inscriptions, puis relance le tirage.".into(),
        ],
        "Ouvrir l'admin",
        format!("{origin}/admin"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn deadline() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, 1, 22, 30, 0).unwrap()
    }

    #[test]
    fn date_en_heure_de_paris() {
        assert_eq!(paris(deadline()), "01/12/2026 à 23h30");
    }

    #[test]
    fn mail_de_resultat() {
        let m = result("a@x.fr", "Alice", "Bob", 42, 2026, 30, "https://santa.test");
        assert_eq!(m.to, "a@x.fr");
        assert!(m.subject.contains("2026"));
        assert!(m.text.contains("tu offres un cadeau à : Bob"));
        assert!(m.text.contains("30 €"));
        assert!(m.text.contains("https://santa.test/joueurs/42"));
        assert!(m.html.contains("https://santa.test/joueurs/42"));
    }

    #[test]
    fn le_html_echappe_les_pseudos() {
        let m = result(
            "a@x.fr",
            "<b>Alice</b>",
            "Bob & Co",
            1,
            2026,
            30,
            "https://santa.test",
        );
        assert!(m.html.contains("&lt;b&gt;Alice&lt;/b&gt;"));
        assert!(m.html.contains("Bob &amp; Co"));
        assert!(!m.html.contains("<b>Alice"));
    }

    #[test]
    fn annonce_relance_et_echec() {
        let a = announcement(
            "a@x.fr",
            "Alice",
            2026,
            25,
            deadline(),
            "https://santa.test",
        );
        assert!(a.text.contains("25 €") && a.text.contains("01/12/2026 à 23h30"));
        let r = reminder("a@x.fr", "Alice", 2026, deadline(), "https://santa.test");
        assert!(r.subject.contains("48 h"));
        let f = draw_failed(
            "admin@x.fr",
            2026,
            "Il faut au moins 3 participants (inscrits : 2).",
            "https://santa.test",
        );
        assert!(f.text.contains("inscrits : 2") && f.text.contains("https://santa.test/admin"));
    }
}
