// ============================================================================
// Juridix — Pré-filtre Winnow — policy.rs
// ============================================================================
// Les questions posées au modèle de décision, et la politique qui transforme
// ses probabilités en « garder / écarter ».
//
// Asymétrie assumée : en droit, écarter à tort un passage (ex. l'exception qui
// renverse la règle) coûte bien plus cher que garder un passage inutile. La
// politique n'écarte donc que sur forte probabilité, jamais un passage
// « épinglé », et garantit un plancher de passages conservés par document.
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};

use super::client::Scores;

// ── Questions ─────────────────────────────────────────────────────────────────

/// Jeu de questions. `with_relevance` : ajoute la question de pertinence, posée
/// seulement s'il existe une demande explicite (`focus`) à laquelle se référer.
pub fn questions(with_relevance: bool) -> Value {
    let mut q = serde_json::Map::new();
    if with_relevance {
        q.insert(
            "relevance".into(),
            json!({
                "type": "score",
                "instructions": "Ce passage d'un texte juridique est-il utile pour répondre à la demande indiquée dans le champ « demande » ? Ne juge que le lien avec la demande, pas la qualité du texte.",
                "criteria": [
                    "Sans aucun rapport avec la demande",
                    "Rapport lointain : simple contexte marginal",
                    "Utile : apporte un élément de réponse",
                    "Essentiel : règle, exception ou solution directement applicable"
                ]
            }),
        );
    }
    q.insert(
        "role".into(),
        json!({
            "type": "choice",
            "instructions": "Quelle est la fonction de ce passage dans le document juridique ?",
            "criteria": {
                "regle": "Règle de droit, dispositif, solution ou décision : contenu normatif ou tranchant",
                "motivation": "Raisonnement, motifs, analyse, exposé des moyens ou des arguments",
                "contexte": "Visas, préambule, exposé des faits ou de la procédure",
                "annexe": "Tableau, liste de renvois, table de concordance, index",
                "mentions": "Mentions de publication, signatures, en-têtes, formules de style : aucun contenu juridique"
            }
        }),
    );
    Value::Object(q)
}

/// État soumis au modèle : la demande, le contexte documentaire et le passage.
pub fn state(focus: Option<&str>, doc_title: &str, heading: Option<&str>, passage: &str) -> Value {
    let mut s = serde_json::Map::new();
    if let Some(f) = focus.filter(|f| !f.trim().is_empty()) {
        s.insert("demande".into(), json!(f.trim()));
    }
    s.insert("document".into(), json!(doc_title));
    if let Some(h) = heading {
        s.insert("section".into(), json!(h));
    }
    s.insert("passage".into(), json!(passage));
    Value::Object(s)
}

// ── Politique ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    HorsSujet,
    SansContenu,
}

impl DropReason {
    pub fn label(self) -> &'static str {
        match self {
            DropReason::HorsSujet => "hors sujet",
            DropReason::SansContenu => "sans contenu juridique",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Omission {
    pub reason: DropReason,
    /// Probabilité sur laquelle repose la décision (sert au plancher : on
    /// restitue en premier les écartements les moins sûrs).
    pub p: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub drop_threshold: f64,
    pub role_threshold: f64,
    pub min_keep_ratio: f64,
}

/// `None` = conserver. Un passage épinglé, tronqué ou sans score exploitable est toujours conservé.
pub fn verdict(scores: &Scores, pinned: bool, pol: &Policy) -> Option<Omission> {
    if pinned || scores.truncated {
        return None;
    }
    if let Some(p0) = scores.p_irrelevant {
        if p0 >= pol.drop_threshold {
            return Some(Omission {
                reason: DropReason::HorsSujet,
                p: p0,
            });
        }
    }
    if let Some((label, p)) = &scores.role {
        if label == "mentions" && *p >= pol.role_threshold {
            return Some(Omission {
                reason: DropReason::SansContenu,
                p: *p,
            });
        }
    }
    None
}

/// Plancher : au moins `max(1, ⌈n·ratio⌉)` passages survivent. Si le filtre est
/// plus agressif, on restitue les écartements les moins sûrs.
pub fn enforce_floor(drops: &mut [Option<Omission>], min_keep_ratio: f64) {
    let n = drops.len();
    if n == 0 {
        return;
    }
    let min_keep = ((n as f64 * min_keep_ratio).ceil() as usize).max(1);
    let mut kept = drops.iter().filter(|d| d.is_none()).count();
    if kept >= min_keep {
        return;
    }
    let mut idx: Vec<usize> = (0..n).filter(|i| drops[*i].is_some()).collect();
    idx.sort_by(|a, b| {
        let pa = drops[*a].map(|d| d.p).unwrap_or(1.0);
        let pb = drops[*b].map(|d| d.p).unwrap_or(1.0);
        pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
    });
    for i in idx {
        if kept >= min_keep {
            break;
        }
        drops[i] = None;
        kept += 1;
    }
}

// ── Passages épinglés : références d'articles citées dans la demande ──────────

static RE_ART_REF: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b([LRDA])\s?\.?\s?(\d{1,4}(?:-\d+){0,3})\b").expect("regex art ref")
});

/// Références normalisées (« l1234-5 ») trouvées dans la demande.
pub fn pinned_refs(focus: &str) -> Vec<String> {
    RE_ART_REF
        .captures_iter(focus)
        .map(|c| format!("{}{}", c[1].to_lowercase(), &c[2]))
        .collect()
}

/// Le passage mentionne-t-il l'une des références (ponctuation et espaces ignorés) ?
pub fn mentions_ref(passage: &str, refs: &[String]) -> bool {
    if refs.is_empty() {
        return false;
    }
    let norm: String = passage
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '.')
        .flat_map(|c| c.to_lowercase())
        .collect();
    refs.iter().any(|r| norm.contains(r.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const POL: Policy = Policy {
        drop_threshold: 0.85,
        role_threshold: 0.85,
        min_keep_ratio: 0.2,
    };

    fn sc(p0: Option<f64>, role: Option<(&str, f64)>) -> Scores {
        Scores {
            p_irrelevant: p0,
            role: role.map(|(l, p)| (l.to_string(), p)),
            truncated: false,
        }
    }

    #[test]
    fn drops_only_above_threshold() {
        assert!(verdict(&sc(Some(0.90), None), false, &POL).is_some());
        assert!(verdict(&sc(Some(0.84), None), false, &POL).is_none());
    }

    #[test]
    fn pinned_and_truncated_are_never_dropped() {
        assert!(verdict(&sc(Some(0.99), None), true, &POL).is_none());
        let mut s = sc(Some(0.99), None);
        s.truncated = true;
        assert!(verdict(&s, false, &POL).is_none());
    }

    #[test]
    fn mentions_role_drops_but_regle_does_not() {
        let d = verdict(&sc(None, Some(("mentions", 0.95))), false, &POL).unwrap();
        assert_eq!(d.reason, DropReason::SansContenu);
        assert!(verdict(&sc(None, Some(("regle", 0.99))), false, &POL).is_none());
        assert!(verdict(&sc(None, Some(("mentions", 0.60))), false, &POL).is_none());
    }

    #[test]
    fn floor_restores_least_confident_drops_first() {
        let mk = |p| {
            Some(Omission {
                reason: DropReason::HorsSujet,
                p,
            })
        };
        // 5 passages, tous écartés ; plancher = ceil(5*0.2) = 1
        let mut d = vec![mk(0.99), mk(0.86), mk(0.95), mk(0.90), mk(0.97)];
        enforce_floor(&mut d, 0.2);
        assert!(d[1].is_none(), "le moins sûr (0.86) doit être restitué");
        assert_eq!(d.iter().filter(|x| x.is_none()).count(), 1);
    }

    #[test]
    fn floor_is_noop_when_enough_survive() {
        let mut d = vec![
            None,
            Some(Omission {
                reason: DropReason::HorsSujet,
                p: 0.9,
            }),
        ];
        enforce_floor(&mut d, 0.2);
        assert!(d[1].is_some());
    }

    #[test]
    fn article_refs_are_extracted_and_matched() {
        let refs = pinned_refs("Quel est le préavis selon l'article L. 1234-5 et R 1234-1 ?");
        assert_eq!(refs, vec!["l1234-5", "r1234-1"]);
        assert!(mentions_ref("Article L. 1234-5 : le préavis…", &refs));
        assert!(mentions_ref("cf. art. R1234-1", &refs));
        assert!(!mentions_ref("Article L. 1234-6", &refs));
        assert!(!mentions_ref("n'importe quoi", &[]));
    }
}
