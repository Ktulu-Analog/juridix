// ============================================================================
// Juridix — Pré-filtre Winnow — segment.rs
// ============================================================================
// Découpe un texte juridique en « passages » (unités soumises au modèle de
// décision). Le texte reçu peut être :
//   - structuré (paragraphes séparés par des retours à la ligne) ;
//   - « à plat » : les dossiers déjà stockés côté frontend ont été passés dans
//     `stripHtml`, qui écrase tous les espaces. On réinsère alors des coupures
//     devant les marqueurs juridiques usuels (Article…, Attendu que…, Vu…).
//
// Invariant : la concaténation des passages (séparés par une ligne vide)
// reproduit le texte d'origine à la normalisation des espaces près — aucun
// mot n'est ajouté ni supprimé.
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub struct Passage {
    /// Dernier intitulé rencontré (Titre, Chapitre, Article…), donné en contexte au modèle.
    pub heading: Option<String>,
    pub text: String,
}

static RE_FLAT_BREAK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"([\.;:\)])\s+(Article\s+[LRDA]?\.?\s?\d|Attendu que|Mais attendu|Vu l['’]|Vu la |Vu le |Vu les |Sur le [a-zéèû]+ moyen|Sur la [a-zéèû]+ branche|PAR CES MOTIFS|Considérant que|Alors que|Titre [IVXLC\d]+|Chapitre [IVXLC\d]+|Section [IVXLC\d]+)",
    )
    .expect("regex flat break")
});

static RE_HEADING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(titre|chapitre|section|sous-section|livre|partie|paragraphe|motifs|dispositif|sur le [a-zéèû]+ moyen|article\s+[LRDA]?\.?\s?\d[\w\-\. ]*$)",
    )
    .expect("regex heading")
});

/// Texte « à plat » : presque aucun retour à la ligne par rapport à sa longueur.
fn is_flat(text: &str) -> bool {
    let newlines = text.matches('\n').count();
    text.len() > 1_500 && newlines < text.len() / 2_000 + 1
}

fn is_heading(block: &str) -> bool {
    let t = block.trim();
    let n = t.chars().count();
    if n == 0 || n > 140 || t.ends_with('.') || t.ends_with(';') || t.ends_with(',') {
        return false;
    }
    if RE_HEADING.is_match(t) {
        return true;
    }
    // Tout en majuscules (ex. « PAR CES MOTIFS », « TITRE PREMIER »)
    let letters: Vec<char> = t.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 4 && letters.iter().all(|c| c.is_uppercase())
}

fn is_abbrev(prefix: &str) -> bool {
    // Mot (ou lettre) juste avant le point : « L. 1234 », « art. 5 », « n° », « al. 2 »…
    let word: String = prefix
        .chars()
        .rev()
        .take_while(|c| c.is_alphabetic())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let w = word.to_lowercase();
    if word.chars().count() == 1 {
        return true; // L. R. D. A. M. etc.
    }
    matches!(
        w.as_str(),
        "art" | "al" | "no" | "nos" | "p" | "pp" | "cf" | "s" | "ss" | "obs" | "req" | "dr" | "me"
    )
}

/// Coupe après « . ; ? ! » suivi d'un espace, hors abréviations juridiques courantes.
fn split_sentences(s: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    for i in 0..chars.len() {
        let (idx, c) = chars[i];
        if !matches!(c, '.' | ';' | '?' | '!') {
            continue;
        }
        let next_is_space = chars
            .get(i + 1)
            .map(|(_, n)| n.is_whitespace())
            .unwrap_or(false);
        if !next_is_space {
            continue;
        }
        if c == '.' && is_abbrev(&s[start..idx]) {
            continue;
        }
        let end = idx + c.len_utf8();
        out.push(&s[start..end]);
        start = end;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// Découpe dur, sur frontière de caractère, pour une « phrase » plus longue que `max`.
fn hard_split(s: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if cur.len() + ch.len_utf8() > max {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Découpe un bloc trop long en morceaux ≤ `max` caractères, à des frontières de phrases.
fn split_oversize(block: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for sent in split_sentences(block) {
        if sent.len() > max {
            if !cur.trim().is_empty() {
                out.push(std::mem::take(&mut cur).trim().to_string());
            }
            for piece in hard_split(sent, max) {
                out.push(piece.trim().to_string());
            }
            continue;
        }
        if cur.len() + sent.len() > max && !cur.trim().is_empty() {
            out.push(std::mem::take(&mut cur).trim().to_string());
        }
        cur.push_str(sent);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Découpe `text` en passages de `min_chars`..=`max_chars` caractères environ.
pub fn segment(text: &str, min_chars: usize, max_chars: usize) -> Vec<Passage> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let prepared = if is_flat(&normalized) {
        RE_FLAT_BREAK
            .replace_all(&normalized, "$1\n\n$2")
            .into_owned()
    } else {
        normalized
    };

    // 1. Blocs = lignes non vides
    let blocks: Vec<String> = prepared
        .split('\n')
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect();

    // 2. Rattacher les intitulés au bloc suivant
    struct Unit {
        heading: Option<String>,
        text: String,
    }
    let mut units: Vec<Unit> = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut pending: Vec<String> = Vec::new();
    for b in blocks {
        if is_heading(&b) {
            current_heading = Some(b.clone());
            pending.push(b);
            continue;
        }
        let text = if pending.is_empty() {
            b
        } else {
            let mut t = pending.join("\n");
            t.push('\n');
            t.push_str(&b);
            pending.clear();
            t
        };
        units.push(Unit {
            heading: current_heading.clone(),
            text,
        });
    }
    if !pending.is_empty() {
        // Intitulés orphelins en fin de texte : on ne les perd pas
        units.push(Unit {
            heading: current_heading.clone(),
            text: pending.join("\n"),
        });
    }

    // 3. Scinder l'énorme, fusionner le minuscule
    let mut passages: Vec<Passage> = Vec::new();
    let mut buf: Option<Passage> = None;
    for u in units {
        let pieces = if u.text.len() > max_chars {
            split_oversize(&u.text, max_chars)
        } else {
            vec![u.text]
        };
        for piece in pieces {
            match buf.take() {
                Some(mut b)
                    if b.text.len() < min_chars && b.text.len() + piece.len() <= max_chars =>
                {
                    b.text.push_str("\n\n");
                    b.text.push_str(&piece);
                    buf = Some(b);
                }
                Some(b) => {
                    passages.push(b);
                    buf = Some(Passage {
                        heading: u.heading.clone(),
                        text: piece,
                    });
                }
                None => {
                    buf = Some(Passage {
                        heading: u.heading.clone(),
                        text: piece,
                    });
                }
            }
        }
    }
    if let Some(b) = buf {
        passages.push(b);
    }
    passages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(|w| w.to_string()).collect()
    }

    #[test]
    fn structured_text_keeps_every_word_in_order() {
        let t = "Titre Ier\n\nArticle L. 1234-5\nLe salarié bénéficie d'un préavis. Il est fixé par décret.\n\nArticle L. 1234-6\nLe préavis est réduit en cas de faute grave.";
        let p = segment(t, 20, 500);
        let rebuilt = p
            .iter()
            .map(|x| x.text.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        assert_eq!(words(t), words(&rebuilt));
        assert!(p.len() >= 2);
    }

    #[test]
    fn flat_text_is_split_on_legal_markers() {
        let mut t = String::new();
        for i in 1..=30 {
            t.push_str(&format!(
                "Article L. 100{i}-1 Le présent article dispose que la règle numéro {i} s'applique aux contrats conclus après la date d'entrée en vigueur. "
            ));
        }
        assert!(t.len() > 1_500 && !t.contains('\n'));
        let p = segment(&t, 100, 1_000);
        assert!(
            p.len() >= 10,
            "attendu un découpage fin, obtenu {}",
            p.len()
        );
        let rebuilt = p
            .iter()
            .map(|x| x.text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(words(&t), words(&rebuilt));
    }

    #[test]
    fn oversize_block_is_split_under_max() {
        let sentence =
            "Attendu que la cour d'appel a constaté que les conditions étaient réunies. ";
        let t = sentence.repeat(200);
        let p = segment(&t, 100, 1_000);
        assert!(p.iter().all(|x| x.text.len() <= 1_000));
        assert!(p.len() > 5);
    }

    #[test]
    fn abbreviations_do_not_split_article_references() {
        let s = "Il résulte de l'art. 12 et de l'article L. 1234-5 du code que la règle s'applique. Fin.";
        let sents = split_sentences(s);
        assert_eq!(sents.len(), 2, "{sents:?}");
    }

    #[test]
    fn headings_are_attached_and_tracked() {
        let t = "CHAPITRE II\n\nArticle 5\nTexte de l'article cinq, suffisamment long pour ne pas être fusionné avec autre chose, et qui contient plusieurs mots utiles afin de dépasser le seuil minimal.";
        let p = segment(t, 50, 2_000);
        assert_eq!(p.len(), 1);
        assert!(p[0].text.starts_with("CHAPITRE II"));
        assert_eq!(p[0].heading.as_deref(), Some("Article 5"));
    }

    #[test]
    fn multibyte_hard_split_never_panics() {
        let t = "é".repeat(5_000);
        let p = segment(&t, 10, 999);
        assert!(p.iter().all(|x| x.text.len() <= 999));
    }
}
