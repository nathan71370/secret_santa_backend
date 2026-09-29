//! Lecture du `players.txt` de la version Java : `nom;email;cible`, où la
//! cible est le NOM d'un autre joueur du même fichier. Après un tirage Java,
//! ce fichier contient exactement les paires de ce tirage.

use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPair {
    pub giver_name: String,
    pub giver_email: String,
    pub receiver_name: String,
    pub receiver_email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedLine {
    pub line: usize,
    pub reason: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedLegacy {
    pub pairs: Vec<LegacyPair>,
    pub skipped: Vec<SkippedLine>,
}

struct Entry {
    line: usize,
    name: String,
    email: String,
    target: Option<String>,
}

pub fn parse(content: &str) -> ParsedLegacy {
    let mut out = ParsedLegacy::default();
    let mut skip = |line: usize, reason: String| out.skipped.push(SkippedLine { line, reason });

    let mut entries = Vec::new();
    let mut emails = HashSet::new();
    for (index, raw) in content.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = index + 1;
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let parts: Vec<&str> = raw.split(';').map(str::trim).collect();
        if parts.len() < 2 || parts[0].is_empty() || !parts[1].contains('@') {
            skip(line, "ligne mal formée (attendu : nom;email;cible)".into());
            continue;
        }
        let email = parts[1].to_lowercase();
        if !emails.insert(email.clone()) {
            skip(line, format!("adresse {email} en double"));
            continue;
        }
        let target = parts.get(2).map(|t| t.to_string()).filter(|t| !t.is_empty());
        entries.push(Entry { line, name: parts[0].to_string(), email, target });
    }

    // Nom en minuscules → index de l'entrée ; None si deux joueurs portent ce nom.
    let mut by_name: HashMap<String, Option<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        by_name.entry(e.name.to_lowercase()).and_modify(|v| *v = None).or_insert(Some(i));
    }

    let mut taken = HashSet::new();
    let mut pairs = Vec::new();
    for e in &entries {
        let Some(target) = &e.target else {
            skip(e.line, "pas de cible".into());
            continue;
        };
        match by_name.get(&target.to_lowercase()) {
            None => skip(e.line, format!("cible « {target} » introuvable dans le fichier")),
            Some(None) => skip(e.line, format!("cible « {target} » ambiguë (nom en double)")),
            Some(Some(i)) => {
                let receiver = &entries[*i];
                if receiver.email == e.email {
                    skip(e.line, "se cible soi-même".into());
                } else if !taken.insert(receiver.email.clone()) {
                    skip(e.line, format!("cible « {} » déjà attribuée", receiver.name));
                } else {
                    pairs.push(LegacyPair {
                        giver_name: e.name.clone(),
                        giver_email: e.email.clone(),
                        receiver_name: receiver.name.clone(),
                        receiver_email: receiver.email.clone(),
                    });
                }
            }
        }
    }
    out.pairs = pairs;
    out.skipped.sort_by_key(|s| s.line);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(g: &str, ge: &str, r: &str, re: &str) -> LegacyPair {
        LegacyPair { giver_name: g.into(), giver_email: ge.into(), receiver_name: r.into(), receiver_email: re.into() }
    }

    #[test]
    fn fichier_nominal() {
        let parsed = parse("Alice;alice@x.fr;Bob\nBob;bob@x.fr;Chloe\nChloe;chloe@x.fr;Alice\n");
        assert!(parsed.skipped.is_empty());
        assert_eq!(parsed.pairs, vec![
            pair("Alice", "alice@x.fr", "Bob", "bob@x.fr"),
            pair("Bob", "bob@x.fr", "Chloe", "chloe@x.fr"),
            pair("Chloe", "chloe@x.fr", "Alice", "alice@x.fr"),
        ]);
    }

    #[test]
    fn tolere_bom_crlf_espaces_casse_et_lignes_vides() {
        let parsed = parse("\u{feff}Alice ; Alice@X.fr ; bob\r\n\r\nBob;bob@x.fr;ALICE\r\n");
        assert!(parsed.skipped.is_empty());
        assert_eq!(parsed.pairs, vec![
            pair("Alice", "alice@x.fr", "Bob", "bob@x.fr"),
            pair("Bob", "bob@x.fr", "Alice", "alice@x.fr"),
        ]);
    }

    #[test]
    fn ignore_les_lignes_invalides_avec_leur_raison() {
        let parsed = parse(concat!(
            "Alice;alice@x.fr;Bob\n",      // 1 ok
            "n'importe quoi\n",            // 2 mal formée
            "Bob;bob@x.fr\n",              // 3 pas de cible
            "Chloe;chloe@x.fr;Zoé\n",      // 4 cible introuvable
            "Dan;dan@x.fr;Bob\n",          // 5 Bob déjà ciblé par Alice
            "Alice2;ALICE@x.fr;Chloe\n",   // 6 adresse en double
            "Eve;eve@x.fr;Eve\n",          // 7 se cible soi-même
        ));
        assert_eq!(parsed.pairs, vec![pair("Alice", "alice@x.fr", "Bob", "bob@x.fr")]);
        let lines: Vec<_> = parsed.skipped.iter().map(|s| s.line).collect();
        assert_eq!(lines, vec![2, 3, 4, 5, 6, 7]);
        assert!(parsed.skipped[0].reason.contains("mal formée"));
        assert!(parsed.skipped[1].reason.contains("pas de cible"));
        assert!(parsed.skipped[2].reason.contains("introuvable"));
        assert!(parsed.skipped[3].reason.contains("déjà"));
        assert!(parsed.skipped[4].reason.contains("en double"));
        assert!(parsed.skipped[5].reason.contains("soi-même"));
    }

    #[test]
    fn nom_ambigu() {
        let parsed = parse("Max;max1@x.fr;Léa\nMax;max2@x.fr;Léa\nLéa;lea@x.fr;Max\n");
        let lea = parsed.skipped.iter().find(|s| s.line == 3).unwrap();
        assert!(lea.reason.contains("ambiguë"));
    }

    #[test]
    fn fichier_vide() {
        assert_eq!(parse(""), ParsedLegacy::default());
    }
}
