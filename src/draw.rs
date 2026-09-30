//! Tirage pur : aucune I/O, le hasard est injecté pour que les tests soient
//! reproductibles. Même algorithme que la version Java : on mélange, on forme
//! un cycle unique (chacun offre au suivant, le dernier au premier), et on
//! recommence si quelqu'un retombe sur sa cible de l'édition précédente.

use rand::{Rng, seq::SliceRandom};
use std::collections::HashMap;

pub type PlayerId = i64;

pub const MIN_PARTICIPANTS: usize = 3;
pub const MAX_ATTEMPTS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DrawError {
    #[error("Il faut au moins {MIN_PARTICIPANTS} participants (inscrits : {0}).")]
    NotEnoughParticipants(usize),
    #[error(
        "Impossible de trouver un tirage valide après {0} essais : la contrainte « pas la même cible que l'an dernier » bloque."
    )]
    NoValidDraw(usize),
}

pub fn draw<R: Rng + ?Sized>(
    participants: &[PlayerId],
    previous: &HashMap<PlayerId, PlayerId>,
    rng: &mut R,
) -> Result<Vec<(PlayerId, PlayerId)>, DrawError> {
    if participants.len() < MIN_PARTICIPANTS {
        return Err(DrawError::NotEnoughParticipants(participants.len()));
    }
    let mut order = participants.to_vec();
    for _ in 0..MAX_ATTEMPTS {
        order.shuffle(rng);
        let pairs: Vec<_> = (0..order.len())
            .map(|i| (order[i], order[(i + 1) % order.len()]))
            .collect();
        if pairs
            .iter()
            .all(|(giver, receiver)| previous.get(giver) != Some(receiver))
        {
            return Ok(pairs);
        }
    }
    Err(DrawError::NoValidDraw(MAX_ATTEMPTS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    use std::collections::{HashMap, HashSet};

    fn rng(seed: u64) -> StdRng {
        StdRng::seed_from_u64(seed)
    }

    #[test]
    fn refuse_moins_de_trois_participants() {
        assert_eq!(
            draw(&[1, 2], &HashMap::new(), &mut rng(1)),
            Err(DrawError::NotEnoughParticipants(2))
        );
        assert_eq!(
            draw(&[], &HashMap::new(), &mut rng(1)),
            Err(DrawError::NotEnoughParticipants(0))
        );
    }

    #[test]
    fn chacun_donne_et_recoit_exactement_une_fois_jamais_a_soi_meme() {
        let ids = [10, 20, 30, 40, 50, 60];
        for seed in 0..300 {
            let pairs = draw(&ids, &HashMap::new(), &mut rng(seed)).unwrap();
            assert_eq!(pairs.len(), ids.len());
            let givers: HashSet<_> = pairs.iter().map(|(g, _)| *g).collect();
            let receivers: HashSet<_> = pairs.iter().map(|(_, r)| *r).collect();
            assert_eq!(givers, ids.iter().copied().collect());
            assert_eq!(receivers, ids.iter().copied().collect());
            assert!(pairs.iter().all(|(g, r)| g != r));
        }
    }

    #[test]
    fn forme_un_seul_cycle() {
        let ids = [1, 2, 3, 4, 5];
        for seed in 0..100 {
            let pairs: HashMap<_, _> = draw(&ids, &HashMap::new(), &mut rng(seed))
                .unwrap()
                .into_iter()
                .collect();
            let mut current = ids[0];
            let mut visited = HashSet::new();
            for _ in 0..ids.len() {
                assert!(visited.insert(current), "cycle trop court");
                current = pairs[&current];
            }
            assert_eq!(current, ids[0]);
        }
    }

    #[test]
    fn respecte_la_cible_de_l_edition_precedente() {
        let ids = [1, 2, 3, 4];
        let previous = HashMap::from([(1, 2), (2, 3), (3, 4), (4, 1)]);
        for seed in 0..300 {
            for (g, r) in draw(&ids, &previous, &mut rng(seed)).unwrap() {
                assert_ne!(previous.get(&g), Some(&r));
            }
        }
    }

    #[test]
    fn signale_un_tirage_impossible() {
        // À trois, seuls deux cycles existent : 1→2→3→1 et 1→3→2→1.
        // 1→2 interdit le premier, 2→1 interdit le second.
        let previous = HashMap::from([(1, 2), (2, 1)]);
        assert_eq!(
            draw(&[1, 2, 3], &previous, &mut rng(7)),
            Err(DrawError::NoValidDraw(MAX_ATTEMPTS))
        );
    }

    #[test]
    fn ignore_une_cible_precedente_qui_ne_participe_plus() {
        let previous = HashMap::from([(1, 99)]);
        assert!(draw(&[1, 2, 3], &previous, &mut rng(3)).is_ok());
    }

    #[test]
    fn messages_en_francais() {
        assert_eq!(
            DrawError::NotEnoughParticipants(2).to_string(),
            "Il faut au moins 3 participants (inscrits : 2)."
        );
        assert!(
            DrawError::NoValidDraw(1000)
                .to_string()
                .contains("1000 essais")
        );
    }
}
