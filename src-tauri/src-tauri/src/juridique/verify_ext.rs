// ============================================================================
// Juridix — Assistant IA juridique desktop
// ============================================================================
// Auteur  : Pierre COUGET
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
//           https://www.gnu.org/licenses/agpl-3.0.html
// Année   : 2026
// ----------------------------------------------------------------------------
// Ce fichier fait partie du projet Juridix.
// Vous pouvez le redistribuer et/ou le modifier selon les termes de la
// licence AGPL-3.0 publiée par la Free Software Foundation.
// ============================================================================

// ============================================================================
// Juridix — Module Juridique — verify_ext.rs
// ============================================================================
// Extension de la vérification des citations.
//
// `verify.rs` vérifie les articles de CODES. Ce module ajoute, avec les mêmes
// principes (jamais de verdict négatif sans recherche aboutie, jamais d'erreur
// présentée comme un verdict) :
//
//   1. les DÉCISIONS de la Cour de cassation citées par leur numéro de pourvoi
//      (« Cass. soc., 12 mars 2024, n° 22-12.345 ») → recherche Judilibre ;
//      existence, puis cohérence de la date et de la chambre citées ;
//   2. les TEXTES hors codes cités par leur numéro (loi, ordonnance, décret :
//      « décret n° 2020-1455 du 27 novembre 2020 ») → recherche Légifrance
//      (fond LODA) ; existence, état juridique, cohérence de la date ;
//   3. les CONVENTIONS COLLECTIVES citées par leur IDCC → consultation KALI ;
//   3 bis. les décisions citées par ECLI, du Conseil d'État, des juridictions
//      administratives et des cours d'appel → `verify_juris.rs` ;
//   4. les CITATIONS TEXTUELLES (passages entre guillemets) :
//        - rattachées à un article de code sans ambiguïté : comparées au texte
//          de l'article sur Légifrance (verdict possible dans les deux sens) ;
//        - sinon : comparées aux documents fournis au modèle (verdict
//          uniquement positif : un passage absent des documents peut venir d'un
//          résultat d'outil ou d'un extrait RAG dont on n'a pas le texte).
//
// Une décision « non retrouvée » n'est PAS une décision inexistante : Judilibre
// n'est pas exhaustif. Le statut `NonRetrouvee` est volontairement distinct de
// `Introuvable` (articles de codes) et rendu par un ❓, pas par un ❌.
//
// ============================================================================

use std::collections::HashMap;
use std::time::Duration;

use futures::{future::BoxFuture, stream, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use super::{
    client::PisteClient,
    judilibre::{self, JudilibreSearchParams},
    legifrance::{self, SearchFond},
    routes::{cache_get, cache_set, Cache},
    verify::{
        etat_libelle, fold, mask_non_prose, month_number, utf16_offset, Ancrage, ArticleRef,
        GroundingIndex, Intern, Kind, Occurrence, Report, Statut, Verification, MAX_OCCURRENCES,
    },
    verify_juris::{self, Court, DecisionQuery},
};

// ── Paramètres ────────────────────────────────────────────────────────────────

/// Décisions distinctes vérifiées par requête.
pub const MAX_DECISION_LOOKUPS: usize = 15;
/// Textes (loi, ordonnance, décret) distincts vérifiés par requête.
pub const MAX_TEXTE_LOOKUPS: usize = 15;
/// Conventions collectives distinctes vérifiées par requête.
pub const MAX_IDCC_LOOKUPS: usize = 10;
/// Citations traitées par requête.
pub const MAX_QUOTES: usize = 12;
/// Textes d'articles téléchargés pour comparer des citations.
pub const MAX_ARTICLE_TEXTS: usize = 8;
/// Appels simultanés (borné pour ne pas marteler PISTE).
pub const EXT_CONCURRENCY: usize = 3;
/// Durée de vie du cache (la clé contient la date du jour : simple plafond).
const TTL_EXT: Duration = Duration::from_secs(12 * 3600);

/// Longueur minimale (en mots) d'une citation comparée au texte de l'article
/// auquel elle est rattachée / aux documents fournis.
const MIN_WORDS_ARTICLE: usize = 6;
const MIN_WORDS_SOURCES: usize = 8;
/// Part des triplets de mots de la citation retrouvés dans la source au-delà de
/// laquelle on parle de citation « proche ».
const APPROX_RATIO: f64 = 0.6;
/// Distance maximale (caractères) entre une citation et l'article auquel on la rattache.
const ATTACH_BEFORE: usize = 220;
const ATTACH_AFTER: usize = 90;

// ── Dates en français ─────────────────────────────────────────────────────────

const MOIS: &str = r"janv(?:ier)?\.?|f[ée]vr(?:ier)?\.?|mars|avr(?:il)?\.?|mai|juin|juill?(?:et)?\.?|ao[ûu]t|sept(?:embre)?\.?|oct(?:obre)?\.?|nov(?:embre)?\.?|d[ée]c(?:embre)?\.?";

static RE_DATE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"(?i)\b(?P<d>1er|\d{{1,2}})\s+(?P<m>{MOIS})\s+(?P<y>(?:19|20)\d{{2}})\b"
    ))
    .expect("regex date")
});

fn month_from_fr(m: &str) -> Option<u32> {
    let f = fold(m);
    let f = f.trim_end_matches('.');
    // Les mois complets passent par la table de verify.rs ; les abréviations par préfixe.
    if let Some(n) = month_number(f) {
        return Some(n);
    }
    [
        ("janv", 1),
        ("fevr", 2),
        ("mars", 3),
        ("avr", 4),
        ("mai", 5),
        ("juin", 6),
        ("juil", 7),
        ("aout", 8),
        ("sept", 9),
        ("oct", 10),
        ("nov", 11),
        ("dec", 12),
    ]
    .iter()
    .find(|(p, _)| f.starts_with(p))
    .map(|(_, n)| *n)
}

/// Première date « 12 mars 2024 » / « 1er déc. 2022 » de `s`, avec sa position.
pub(crate) fn first_date(s: &str) -> Option<(usize, usize, String)> {
    let c = RE_DATE.captures(s)?;
    let day: u32 = if c["d"].eq_ignore_ascii_case("1er") {
        1
    } else {
        c["d"].parse().ok()?
    };
    let mo = month_from_fr(&c["m"])?;
    if !(1..=31).contains(&day) {
        return None;
    }
    let whole = c.get(0)?;
    Some((
        whole.start(),
        whole.end(),
        format!("{}-{:02}-{:02}", &c["y"], mo, day),
    ))
}

/// « AAAA-MM-JJ » → « JJ/MM/AAAA ».
fn fr_date(iso: &str) -> String {
    let head = iso.get(..10).unwrap_or(iso);
    let mut it = head.split('-');
    match (it.next(), it.next(), it.next()) {
        (Some(y), Some(m), Some(d)) => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}

// ── Chambres de la Cour de cassation ──────────────────────────────────────────

/// Chambre canonique (`soc`, `comm`, `cr`, `civ1`, `civ2`, `civ3`, `pl`, `mi`) d'un
/// libellé cité (« Cass. soc. », « 1re civ. ») ou renvoyé par Judilibre
/// (« Chambre sociale », « soc », « civ1 »). `None` si on ne sait pas : la
/// comparaison est alors ignorée plutôt que de signaler une fausse incohérence.
pub(crate) fn canon_chamber(s: &str) -> Option<&'static str> {
    let f = fold(s).replace(['-', '_', '.', ','], " ");
    let tokens: Vec<&str> = f.split_whitespace().collect();
    let has_tok = |t: &str| tokens.contains(&t);
    let starts = |p: &str| tokens.iter().any(|x| x.starts_with(p));
    if starts("plen") || has_tok("pl") {
        return Some("pl");
    }
    if starts("mixte") || has_tok("mi") {
        return Some("mi");
    }
    if starts("soc") {
        return Some("soc");
    }
    if starts("comm") || has_tok("com") {
        return Some("comm");
    }
    if starts("crim") || has_tok("cr") {
        return Some("cr");
    }
    if starts("civ") || has_tok("premiere") || has_tok("deuxieme") || has_tok("troisieme") {
        let has = |alts: &[&str]| alts.iter().any(|a| has_tok(a) || f.contains(a));
        if has(&["civ1", "civ 1", "1re", "1ere", "premiere"]) {
            return Some("civ1");
        }
        if has(&["civ2", "civ 2", "2e", "2eme", "deuxieme"]) {
            return Some("civ2");
        }
        if has(&["civ3", "civ 3", "3e", "3eme", "troisieme"]) {
            return Some("civ3");
        }
    }
    None
}

// ── Décisions de la Cour de cassation ─────────────────────────────────────────

/// « n° 22-12.345 », « pourvoi n° 22-12345 », « n° de pourvoi : 22-12.345 ».
/// Le numéro est AA-NN.NNN (7 chiffres) ; les autres séries (décrets 2020-1455,
/// lois 78-17) ont une forme différente et ne sont pas confondues.
static RE_POURVOI: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:^|[^\d\-\x{2011}\x{2013}.])(?P<pre>(?:pourvoi|n\s?[°º]|no\.|num[ée]ro)(?:\s*(?:de\s+pourvoi|n\s?[°º]|no\.?))?\s*:?\s*)(?P<num>\d{2}\s?[-\x{2011}\x{2013}]\s?\d{2}\s?[.\s\x{a0}\x{202f}]?\s?\d{3})(?:[^\d]|$)",
    )
    .expect("regex pourvoi")
});

/// Indices que la décision citée est une décision de la Cour de cassation.
static RE_COURT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:cour\s+de\s+cassation|\bcass(?:ation)?\b\.?|\bass(?:embl[ée]e)?\.?\s*pl[ée]n(?:i[èe]re)?\.?|\bch(?:ambre|\.)?\s*mixte|(?:\bchambre\s+)?(?:sociale|commerciale|criminelle)\b|\b(?:soc|com|crim)\.|\b(?:civ|civile)\.?\s*[123]\s*(?:re|ère|e|ème|eme)?|\b[123]\s*(?:re|ère|e|ème|eme)\s*(?:ch(?:ambre|\.)?\s*)?civ(?:ile|\.)?|\b(?:premi[èe]re|deuxi[èe]me|troisi[èe]me)\s+chambre\s+civile)",
    )
    .expect("regex cour")
});

/// Une décision de la Cour de cassation citée par son numéro de pourvoi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRef {
    /// Début / fin dans le texte d'origine, en octets (de « Cass. » au numéro).
    pub start: usize,
    pub end: usize,
    pub raw: String,
    /// Chiffres du numéro de pourvoi (« 2212345 ») : clé de regroupement et d'ancrage.
    pub key: String,
    /// Numéro affiché (« 22-12.345 »).
    pub display: String,
    /// Chambre citée, canonique (voir `canon_chamber`).
    pub chambre: Option<&'static str>,
    /// Date citée, « AAAA-MM-JJ ».
    pub date: Option<String>,
    /// Juridiction (voir `verify_juris`) : Cour de cassation pour un numéro de pourvoi.
    pub court: Court,
    /// Numéro normalisé (voir `DecisionQuery::number`) ; `None` pour un ECLI.
    pub number: Option<String>,
    /// ECLI cité, en majuscules.
    pub ecli: Option<String>,
    /// Siège cité (cour d'appel).
    pub place: Option<String>,
}

fn display_pourvoi(key: &str) -> String {
    format!("{}-{}.{}", &key[0..2], &key[2..4], &key[4..7])
}

/// Extrait les décisions de la Cour de cassation citées par numéro de pourvoi.
/// Une décision n'est retenue que si le contexte le confirme (« pourvoi », nom de
/// la Cour ou d'une chambre) : un simple « n° 22-12.345 » ne suffit pas.
/// Mêmes exclusions que les autres extracteurs (code, liens, URL).
pub fn extract_decision_refs(text: &str) -> Vec<DecisionRef> {
    let masked = mask_non_prose(text);
    let mut out: Vec<DecisionRef> = Vec::new();
    let mut prev_end = 0usize;
    for c in RE_POURVOI.captures_iter(&masked) {
        let pre = c.name("pre").expect("pre");
        let num = c.name("num").expect("num");
        let digits: String = num.as_str().chars().filter(char::is_ascii_digit).collect();
        if digits.len() != 7 {
            continue;
        }
        let num_start = pre.start();

        // Fenêtre de contexte : même ligne, après la décision précédente et le dernier « ; ».
        let line_start = masked[..num_start].rfind('\n').map_or(0, |i| i + 1);
        let mut w_start = num_start.saturating_sub(180).max(line_start).max(prev_end);
        while w_start < num_start && !masked.is_char_boundary(w_start) {
            w_start += 1;
        }
        if let Some(i) = masked[w_start..num_start].rfind(';') {
            w_start += i + 1;
        }
        let window = &masked[w_start..num_start];

        let mut court_start: Option<usize> = None;
        let mut chambre: Option<&'static str> = None;
        for m in RE_COURT.find_iter(window) {
            court_start.get_or_insert(w_start + m.start());
            if chambre.is_none() {
                chambre = canon_chamber(m.as_str());
            }
        }
        let has_pourvoi_word = fold(pre.as_str()).contains("pourvoi");
        if !has_pourvoi_word && court_start.is_none() {
            continue; // « n° 22-12.345 » sans aucun contexte de décision
        }

        let date = first_date(window);
        let start = [
            court_start,
            date.as_ref().map(|(s, _, _)| w_start + *s),
            Some(num_start),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(num_start);
        let end = num.end();
        if masked[start..end] != text[start..end] {
            continue;
        }
        prev_end = end;
        out.push(DecisionRef {
            start,
            end,
            raw: text[start..end].to_string(),
            display: display_pourvoi(&digits),
            key: digits.clone(),
            chambre,
            date: date.map(|(_, _, d)| d),
            court: Court::Cassation,
            number: Some(digits),
            ecli: None,
            place: None,
        });
    }
    // Décisions citées par ECLI, du Conseil d'État, des juridictions administratives et
    // des cours d'appel : même contexte, mêmes exclusions, sans chevaucher les pourvois.
    let taken: Vec<(usize, usize)> = out.iter().map(|d| (d.start, d.end)).collect();
    out.extend(verify_juris::extract_other_decisions(text, &masked, &taken));
    out.sort_by_key(|d| d.start);
    out
}

// ── Conventions collectives (IDCC) ────────────────────────────────────────────

static RE_IDCC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bIDCC\s*(?:n\s?[°º]\s*|:\s*)?(?P<n>\d{3,4})\b").expect("regex idcc")
});

/// Une convention collective citée par son IDCC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdccRef {
    pub start: usize,
    pub end: usize,
    pub raw: String,
    /// IDCC sur 4 chiffres (« 1486 », « 0016 »).
    pub idcc: String,
}

pub fn extract_idcc_refs(text: &str) -> Vec<IdccRef> {
    let masked = mask_non_prose(text);
    RE_IDCC
        .captures_iter(&masked)
        .filter_map(|c| {
            let whole = c.get(0)?;
            let n = c.name("n")?.as_str();
            if masked[whole.start()..whole.end()] != text[whole.start()..whole.end()] {
                return None;
            }
            Some(IdccRef {
                start: whole.start(),
                end: whole.end(),
                raw: text[whole.start()..whole.end()].to_string(),
                idcc: format!("{n:0>4}"),
            })
        })
        .collect()
}

// ── Citations textuelles ──────────────────────────────────────────────────────

static RE_QUOTE_FR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"«[\s\x{a0}\x{202f}]*(?P<q>[^«»]+?)[\s\x{a0}\x{202f}]*»").expect("regex quote fr")
});
static RE_QUOTE_CURLY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"“\s*(?P<q>[^“”]+?)\s*”").expect("regex quote curly"));
/// Coupures d'une citation : ellipses et insertions entre crochets.
static RE_CUT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[[^\]]*\]|\([\s]*(?:…|\.{3})[\s]*\)|…|\.{3}").expect("regex cut"));

/// Un passage entre guillemets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteRef {
    /// Guillemets compris, octets dans le texte d'origine.
    pub start: usize,
    pub end: usize,
    /// Guillemet fermant seul : c'est lui qui porte le badge (un lien Markdown
    /// englobant toute la citation casserait sur ses crochets éventuels).
    pub close_start: usize,
    pub close_end: usize,
    /// Contenu de la citation.
    pub text: String,
    pub words: usize,
}

/// Mots (lettres/chiffres, sans accents ni casse) séparés par une espace.
/// Même normalisation des deux côtés : la ponctuation, les apostrophes
/// typographiques et la casse ne comptent pas.
pub(crate) fn tokens_string(s: &str) -> String {
    let f = fold(s);
    let mut out = String::with_capacity(f.len());
    let mut in_word = false;
    for ch in f.chars() {
        if ch.is_alphanumeric() {
            if !in_word && !out.is_empty() {
                out.push(' ');
            }
            out.push(ch);
            in_word = true;
        } else {
            in_word = false;
        }
    }
    out
}

fn word_count(s: &str) -> usize {
    let t = tokens_string(s);
    if t.is_empty() {
        0
    } else {
        t.split(' ').count()
    }
}

/// Longueur minimale (caractères) d'un passage entre guillemets droits : en deçà, il s'agit
/// presque toujours d'un mot mis en évidence, pas d'une citation.
const MIN_STRAIGHT_QUOTE_CHARS: usize = 20;

/// Citations entre guillemets droits `"…"`. Ambigus par nature (le guillemet ouvrant et le
/// fermant sont identiques) : on n'apparie que dans un paragraphe qui en compte un nombre PAIR,
/// l'ouvrant précédé d'un début de ligne, d'une espace ou d'une ponctuation et le fermant suivi
/// d'une fin de ligne, d'une espace ou d'une ponctuation.
fn extract_straight_quotes(text: &str, masked: &str) -> Vec<QuoteRef> {
    let mut out = Vec::new();
    let mut para_start = 0usize;
    for para in masked.split("\n\n") {
        let positions: Vec<usize> = para
            .match_indices('"')
            .map(|(i, _)| para_start + i)
            .collect();
        if positions.len() >= 2 && positions.len() % 2 == 0 {
            for pair in positions.chunks(2) {
                let (a, b) = (pair[0], pair[1]);
                let before = masked[..a].chars().last();
                let after = masked[b + 1..].chars().next();
                // `map_or` plutôt qu'`is_none_or` (1.82) : le MSRV du projet est 1.77.
                let open_ok = before.map_or(true, |c| !c.is_alphanumeric());
                let close_ok = after.map_or(true, |c| !c.is_alphanumeric());
                let inner = &masked[a + 1..b];
                if !open_ok
                    || !close_ok
                    || inner.trim().chars().count() < MIN_STRAIGHT_QUOTE_CHARS
                    || masked[a..=b] != text[a..=b]
                {
                    continue;
                }
                out.push(QuoteRef {
                    start: a,
                    end: b + 1,
                    close_start: b,
                    close_end: b + 1,
                    text: text[a + 1..b].to_string(),
                    words: word_count(inner),
                });
            }
        }
        para_start += para.len() + 2;
    }
    out
}

/// Blocs de citation Markdown (`> …`) SANS guillemets : un texte cité en bloc est une citation
/// même sans guillemets (c'est ainsi que les assistants présentent le texte d'un article).
/// Le badge se place sur le dernier caractère significatif du bloc.
fn extract_blockquotes(text: &str, masked: &str) -> Vec<QuoteRef> {
    struct Block {
        start: usize,
        end: usize,
        lines: Vec<String>,
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut cur: Option<Block> = None;
    let mut offset = 0usize;
    for raw in text.split_inclusive('\n') {
        let content = raw.trim_end_matches(['\n', '\r']);
        let trimmed = content.trim_start();
        let lead = content.len() - trimmed.len();
        let is_quote =
            trimmed.starts_with('>') && masked.get(offset..offset + content.len()) == Some(content);
        if is_quote {
            let inner = trimmed.trim_start_matches(['>', ' ']).to_string();
            let end = offset + content.len();
            match cur.as_mut() {
                Some(b) => {
                    b.end = end;
                    b.lines.push(inner);
                }
                None => {
                    cur = Some(Block {
                        start: offset + lead,
                        end,
                        lines: vec![inner],
                    })
                }
            }
        } else if let Some(b) = cur.take() {
            blocks.push(b);
        }
        offset += raw.len();
    }
    if let Some(b) = cur.take() {
        blocks.push(b);
    }

    blocks
        .into_iter()
        .filter_map(|b| {
            let inner = b.lines.join("\n");
            let words = word_count(&inner);
            if words < MIN_WORDS_ARTICLE {
                return None;
            }
            let block_text = &text[b.start..b.end];
            let (idx, ch) = block_text
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_alphanumeric() || ".;:!?)»”".contains(*c))?;
            Some(QuoteRef {
                start: b.start,
                end: b.end,
                close_start: b.start + idx,
                close_end: b.start + idx + ch.len_utf8(),
                text: inner,
                words,
            })
        })
        .collect()
}

/// Retire une étiquette d'article en tête d'un texte cité (« **Article 1140** Il y a… ») :
/// elle ne fait pas partie du texte de l'article.
pub(crate) fn strip_article_label(q: &str) -> &str {
    static RE_LABEL: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?i)^[\s*_>]*(?:article|art\.?)\s*[LRDA]?\s*\.?\s*\d[\d\-.]*(?:[\s*_]*[:.\-–—])?[\s*_]*",
        )
        .expect("regex label")
    });
    match RE_LABEL.find(q) {
        Some(m) => &q[m.end()..],
        None => q,
    }
}

pub fn extract_quotes(text: &str) -> Vec<QuoteRef> {
    let masked = mask_non_prose(text);
    let mut out: Vec<QuoteRef> = extract_straight_quotes(text, &masked);
    for re in [&*RE_QUOTE_FR, &*RE_QUOTE_CURLY] {
        for c in re.captures_iter(&masked) {
            let whole = c.get(0).expect("match 0");
            let q = c.name("q").expect("q");
            if q.as_str().contains("\n\n") {
                continue; // plusieurs paragraphes : guillemets orphelins, pas une citation
            }
            if masked[whole.start()..whole.end()] != text[whole.start()..whole.end()] {
                continue;
            }
            let close_len = text[whole.start()..whole.end()]
                .chars()
                .last()
                .map_or(1, char::len_utf8);
            out.push(QuoteRef {
                start: whole.start(),
                end: whole.end(),
                close_start: whole.end() - close_len,
                close_end: whole.end(),
                text: text[q.start()..q.end()].to_string(),
                words: word_count(q.as_str()),
            });
        }
    }
    // Un bloc de citation qui contient déjà une citation entre guillemets n'est pas compté deux fois.
    let blocks: Vec<QuoteRef> = extract_blockquotes(text, &masked)
        .into_iter()
        .filter(|b| !out.iter().any(|o| o.start >= b.start && o.end <= b.end))
        .collect();
    out.extend(blocks);
    out.sort_by_key(|q| q.start);
    out
}

/// Article de code auquel une citation est rattachée, ou `None` si le lien n'est
/// pas sans ambiguïté (aucun article proche dans le même paragraphe, ou plusieurs
/// articles différents : on ne compare pas plutôt que de risquer une fausse alerte).
pub(crate) fn attached_article(q: &QuoteRef, refs: &[ArticleRef], text: &str) -> Option<usize> {
    // Même paragraphe, ou UN saut de paragraphe quand ce qui précède annonce la citation
    // (« … dispose que :⏎⏎> « … » ») : c'est la mise en forme habituelle d'un texte cité.
    let same_paragraph = |a: usize, b: usize| {
        let gap = &text[a.min(b)..a.max(b)];
        match gap.matches("\n\n").count() {
            0 => true,
            1 => {
                a < b
                    && gap
                        .split("\n\n")
                        .next()
                        .is_some_and(|intro| intro.trim_end().ends_with(':'))
            }
            _ => false,
        }
    };
    let mut near: Vec<(usize, usize, bool)> = Vec::new(); // (index, distance, avant)
    for (i, r) in refs.iter().enumerate() {
        if r.code.is_none() {
            continue;
        }
        if r.end <= q.start {
            let d = q.start - r.end;
            if d <= ATTACH_BEFORE && same_paragraph(r.end, q.start) {
                near.push((i, d, true));
            }
        } else if r.start >= q.end {
            let d = r.start - q.end;
            if d <= ATTACH_AFTER && same_paragraph(q.end, r.start) {
                near.push((i, d, false));
            }
        }
    }
    let mut keys: Vec<(&'static str, String)> = near
        .iter()
        .map(|(i, _, _)| (refs[*i].code.expect("code"), refs[*i].numero_key()))
        .collect();
    keys.sort();
    keys.dedup();
    if keys.len() != 1 {
        return None;
    }
    // Le plus proche, en préférant l'article qui précède la citation.
    near.into_iter()
        .min_by_key(|(_, d, avant)| (*d, !*avant))
        .map(|(i, _, _)| i)
}

/// Résultat de la comparaison d'une citation à un texte source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuoteMatch {
    /// Chaque fragment de la citation figure tel quel dans la source.
    Exact,
    /// Proche (la plupart des triplets de mots retrouvés) sans être identique.
    Approx,
    /// Rien de comparable.
    Absent,
    /// Citation trop fragmentaire pour être comparée (ellipses, crochets).
    Unverifiable,
}

/// Source préparée : mots normalisés, entourés d'espaces pour des recherches de mots entiers.
pub(crate) fn haystack(source: &str) -> String {
    format!(" {} ", tokens_string(source))
}

pub(crate) fn compare_quote(quote: &str, hay: &str) -> QuoteMatch {
    let frags: Vec<String> = RE_CUT
        .split(quote)
        .map(tokens_string)
        .filter(|t| !t.is_empty() && t.split(' ').count() >= 3)
        .collect();
    if frags.is_empty() {
        return QuoteMatch::Unverifiable;
    }
    if frags.iter().all(|f| hay.contains(&format!(" {f} "))) {
        return QuoteMatch::Exact;
    }
    let (mut total, mut found) = (0usize, 0usize);
    for f in &frags {
        let w: Vec<&str> = f.split(' ').collect();
        for t in w.windows(3) {
            total += 1;
            if hay.contains(&format!(" {} {} {} ", t[0], t[1], t[2])) {
                found += 1;
            }
        }
    }
    if total >= 4 && (found as f64) / (total as f64) >= APPROX_RATIO {
        QuoteMatch::Approx
    } else {
        QuoteMatch::Absent
    }
}

/// Mots d'un texte, dans leur forme d'origine (apostrophes et traits d'union conservés) et
/// sous une forme comparable (sans accents ni casse, apostrophes normalisées).
fn diff_words(s: &str) -> Vec<(String, String)> {
    s.split_whitespace()
        .filter_map(|w| {
            let w = w.trim_matches(|c: char| !c.is_alphanumeric());
            (!w.is_empty()).then(|| (w.to_string(), fold(&w.replace('’', "'"))))
        })
        .collect()
}

/// Séries de mots consécutifs (indices contigus) regroupées en expressions lisibles.
fn runs(words: &[(String, String)], idx: &[usize]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut prev: Option<usize> = None;
    for &i in idx {
        if prev.is_some_and(|p| i != p + 1) && !cur.is_empty() {
            out.push(cur.join(" "));
            cur.clear();
        }
        cur.push(&words[i].0);
        prev = Some(i);
    }
    if !cur.is_empty() {
        out.push(cur.join(" "));
    }
    out
}

/// Écarts mot à mot entre une citation et le texte de l'article : (mots AJOUTÉS à la
/// citation, mots du texte OMIS dans la zone citée). Alignement par plus longue sous-suite
/// commune. Vide si les deux textes ne sont pas assez proches pour que la comparaison ait un
/// sens, ou trop volumineux.
pub(crate) fn quote_diff(quote: &str, article: &str) -> (Vec<String>, Vec<String>) {
    const MAX_CELLS: usize = 4_000_000;
    let q = diff_words(quote);
    let a = diff_words(article);
    if q.len() < 4 || a.is_empty() || q.len().saturating_mul(a.len()) > MAX_CELLS {
        return (Vec::new(), Vec::new());
    }
    // lcs[i][j] : longueur de la plus longue sous-suite commune de q[i..] et a[j..].
    let (n, m) = (q.len(), a.len());
    let mut lcs = vec![0u16; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if q[i].1 == a[j].1 {
                lcs[at(i + 1, j + 1)] + 1
            } else {
                lcs[at(i + 1, j)].max(lcs[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0usize, 0usize);
    let mut q_matched = vec![false; n];
    let mut a_matched = vec![false; m];
    while i < n && j < m {
        if q[i].1 == a[j].1 {
            q_matched[i] = true;
            a_matched[j] = true;
            i += 1;
            j += 1;
        } else if lcs[at(i + 1, j)] >= lcs[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let matched = q_matched.iter().filter(|m| **m).count();
    if (matched as f64) / (n as f64) < 0.6 {
        return (Vec::new(), Vec::new());
    }
    let added: Vec<usize> = (0..n).filter(|&k| !q_matched[k]).collect();
    // Mots de l'article omis : seulement dans la zone effectivement citée, élargie d'autant de
    // mots que la citation en compte de non appariés en tête / en queue (un mot REMPLACÉ en bout
    // de phrase laisse le mot officiel juste au-delà du dernier mot apparié).
    let first = a_matched.iter().position(|m| *m).unwrap_or(0);
    let last = a_matched.iter().rposition(|m| *m).unwrap_or(0);
    let q_first = q_matched.iter().position(|m| *m).unwrap_or(0);
    let q_last = q_matched.iter().rposition(|m| *m).unwrap_or(0);
    let start = first.saturating_sub(q_first);
    let end = (last + (n - 1 - q_last)).min(m - 1);
    let omitted: Vec<usize> = (start..=end).filter(|&k| !a_matched[k]).collect();
    (runs(&q, &added), runs(&a, &omitted))
}

/// Lignes d'info-bulle décrivant les écarts d'une citation « proche » (au plus 4 séries chacune).
fn diff_infos(quote: &str, article: &str) -> Vec<String> {
    let (added, omitted) = quote_diff(quote, article);
    let fmt = |label: &str, v: &[String]| -> Option<String> {
        (!v.is_empty()).then(|| {
            let shown: Vec<String> = v
                .iter()
                .take(4)
                .map(|r| {
                    let r: String = r.chars().take(80).collect();
                    format!("« {r} »")
                })
                .collect();
            let more = if v.len() > 4 { " …" } else { "" };
            format!("{label} : {}{more}", shown.join(", "))
        })
    };
    [
        fmt("Mots ajoutés par rapport au texte de l'article", &added),
        fmt("Mots du texte de l'article omis dans la citation", &omitted),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Texte brut d'un contenu HTML Légifrance (balises retirées, entités courantes décodées).
pub(crate) fn html_to_text(s: &str) -> String {
    static RE_BR: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)<\s*(?:br\s*/?|/p|/li|/div)\s*>").expect("regex br"));
    static RE_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]*>").expect("regex tag"));
    let with_breaks = RE_BR.replace_all(s, " ");
    let no_tags = RE_TAG.replace_all(&with_breaks, "");
    no_tags
        .replace("&nbsp;", " ")
        .replace("&laquo;", "«")
        .replace("&raquo;", "»")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

// ── Extraction d'ensemble ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ExtRefs {
    pub decisions: Vec<DecisionRef>,
    pub idccs: Vec<IdccRef>,
    pub quotes: Vec<QuoteRef>,
}

pub fn extract_ext_refs(text: &str) -> ExtRefs {
    ExtRefs {
        decisions: extract_decision_refs(text),
        idccs: extract_idcc_refs(text),
        quotes: extract_quotes(text),
    }
}

/// Un texte déjà extrait par `extract_text_refs` qui chevauche une décision (par
/// exemple « décision n° 22-12 » dans « décision n° 22-12.345 ») ne désigne pas
/// un texte hors code : on l'écarte.
pub fn overlaps_decision(start: usize, end: usize, decisions: &[DecisionRef]) -> bool {
    decisions.iter().any(|d| start < d.end && d.start < end)
}

/// Article de code mentionné dans la QUESTION de l'utilisateur, déjà identifié sur Légifrance.
/// Quand la réponse ne cite elle-même aucun article de code, c'est lui que désignent ses
/// citations (« Cite mot pour mot l'article 1240 du Code civil » → la réponse ne contient que
/// le texte, sans numéro).
#[derive(Debug, Clone)]
pub struct ContextArticle {
    /// « Code civil, art. 1240 »
    pub reference: String,
    pub code: Option<String>,
    /// Identifiant LEGIARTI de la version retenue.
    pub id: String,
    /// État en clair (« en vigueur », « abrogé »…).
    pub etat_libelle: Option<String>,
}

/// Article de code que désigne la question, sans ambiguïté : exactement UN article de code
/// distinct dans la question, et aucun dans la réponse (sinon la proximité décide), et au moins
/// une citation assez longue dans la réponse.
pub fn context_article_ref(
    contexte: &str,
    answer_refs: &[ArticleRef],
    quotes: &[QuoteRef],
) -> Option<ArticleRef> {
    if answer_refs.iter().any(|r| r.code.is_some())
        || !quotes.iter().any(|q| q.words >= MIN_WORDS_ARTICLE)
    {
        return None;
    }
    let ctx = super::verify::extract_article_refs(contexte);
    // Tous les articles de la question comptent, codés ou non : « l'article 1240 et l'article
    // 1241 du Code civil » en désigne deux, même si un seul porte le nom du code. Un même
    // numéro répété (« l'article 1240 du Code civil… bien l'article 1240 ») n'en désigne qu'un.
    let mut numeros: Vec<String> = ctx.iter().map(|r| r.numero_key()).collect();
    numeros.sort();
    numeros.dedup();
    if numeros.len() != 1 {
        return None;
    }
    let mut codes: Vec<&'static str> = ctx.iter().filter_map(|r| r.code).collect();
    codes.sort();
    codes.dedup();
    if codes.len() != 1 {
        return None; // aucun code précisé, ou codes contradictoires
    }
    ctx.into_iter().find(|r| r.code.is_some())
}

/// Faut-il interroger PISTE ? (sinon la route n'exige pas d'identifiants PISTE)
/// `ctx_article` : la question désigne un article auquel rattacher les citations.
pub fn needs_network(
    ext: &ExtRefs,
    texts: &[super::verify::TextRef],
    refs: &[ArticleRef],
    text: &str,
    ctx_article: bool,
) -> bool {
    ctx_article
        || ext.decisions.iter().any(|d| d.court != Court::Autre)
        || !ext.idccs.is_empty()
        || texts
            .iter()
            .any(|t| texte_is_lookupable(t.nature, t.numero.as_deref()))
        || ext
            .quotes
            .iter()
            .any(|q| q.words >= MIN_WORDS_ARTICLE && attached_article(q, refs, text).is_some())
}

fn texte_is_lookupable(nature: &str, numero: Option<&str>) -> bool {
    numero.is_some() && matches!(nature, "loi" | "ordonnance" | "decret")
}

// ── Résultats de recherche ────────────────────────────────────────────────────

/// Ce que Judilibre sait d'une décision.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionInfo {
    pub id: String,
    pub number: String,
    pub chamber: Option<String>,
    pub jurisdiction: Option<String>,
    pub formation: Option<String>,
    /// « AAAA-MM-JJ »
    pub date: Option<String>,
    pub solution: Option<String>,
    pub publication: Vec<String>,
    /// ECLI de la décision, quand la source le donne.
    #[serde(default)]
    pub ecli: Option<String>,
    /// Remarque à afficher avec la fiche (ex. : comparaison de l'ECLI indirecte).
    #[serde(default)]
    pub note: Option<String>,
    /// Base d'où vient la décision (« Judilibre », « Légifrance »).
    #[serde(default)]
    pub source: Option<String>,
    /// Lien vers la décision quand il ne se déduit pas de l'identifiant.
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum DecisionLookup {
    Trouvee(DecisionInfo),
    /// Recherche aboutie, aucune décision de ce numéro.
    Introuvable,
    /// Des décisions de ce numéro existent, mais rien ne permet de désigner celle qui est
    /// citée (numéro de RG partagé par plusieurs cours d'appel) : pas de verdict.
    Incertaine {
        raison: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TexteLookup {
    Trouve {
        id: String,
        titre: String,
        etat: Option<String>,
        /// Date de signature « AAAA-MM-JJ » (champ `dateSignature` de la recherche).
        #[serde(default)]
        date: Option<String>,
    },
    Introuvable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IdccLookup {
    Trouve {
        id: String,
        titre: Option<String>,
        etat: Option<String>,
    },
    Introuvable,
}

pub(crate) fn str_field(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn digits_of(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// Fiche d'une décision Judilibre (élément de `results`).
pub(crate) fn decision_info_from(r: &Value, id: String, number: String) -> DecisionInfo {
    let publication = r
        .get("publication")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    DecisionInfo {
        id,
        number,
        chamber: str_field(r, "chamber"),
        jurisdiction: str_field(r, "jurisdiction"),
        formation: str_field(r, "formation"),
        date: str_field(r, "decision_date").map(|d| d.chars().take(10).collect()),
        solution: str_field(r, "solution"),
        publication,
        ecli: str_field(r, "ecli"),
        note: None,
        source: Some("Judilibre".to_string()),
        url: None,
    }
}

/// Décisions de `results` dont le numéro (`number` ou `numbers[]`) vaut `key`.
pub(crate) fn pick_decisions(results: &[Value], key: &str) -> Vec<DecisionInfo> {
    let mut out = Vec::new();
    for r in results {
        let mut numbers: Vec<String> = Vec::new();
        if let Some(n) = str_field(r, "number") {
            numbers.push(n);
        }
        if let Some(arr) = r.get("numbers").and_then(Value::as_array) {
            numbers.extend(arr.iter().filter_map(Value::as_str).map(str::to_string));
        }
        let Some(matched) = numbers.iter().find(|n| digits_of(n) == key) else {
            continue;
        };
        let Some(id) = str_field(r, "id") else {
            continue;
        };
        out.push(decision_info_from(r, id, matched.clone()));
    }
    out
}

/// Première décision de `results` dont le numéro (`number` ou `numbers[]`) vaut `key`.
pub(crate) fn pick_decision(results: &[Value], key: &str) -> Option<DecisionInfo> {
    pick_decisions(results, key).into_iter().next()
}

/// Décision de `results` dont l'ECLI est exactement `ecli` (sans tenir compte de la casse).
pub(crate) fn pick_decision_by_ecli(results: &[Value], ecli: &str) -> Option<DecisionInfo> {
    results.iter().find_map(|r| {
        let found = str_field(r, "ecli")?;
        if !found.eq_ignore_ascii_case(ecli) {
            return None;
        }
        let id = str_field(r, "id")?;
        let number = str_field(r, "number").unwrap_or_default();
        Some(decision_info_from(r, id, number))
    })
}

/// `numero` figure-t-il dans `hay` comme un numéro entier ? (« 78-17 » ne doit pas
/// être trouvé dans « 2078-170 »).
pub(crate) fn contains_numero(hay: &str, numero: &str) -> bool {
    hay.match_indices(numero).any(|(i, _)| {
        let before = hay[..i].chars().last();
        let after = hay[i + numero.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit() || c == '-')
            && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

fn strip_tags(s: &str) -> String {
    static RE_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]*>").expect("regex tag"));
    RE_TAG.replace_all(s, "").trim().to_string()
}

/// Texte de `results` (réponse de recherche LODA) dont le titre commence par la
/// nature demandée et contient le numéro. Un arrêté qui cite seulement le numéro
/// d'un décret dans son titre ne compte pas.
///
/// Forme des résultats (spec Légifrance `SearchResult`) : `etat`, `num`, `nature`,
/// `dateSignature`, et `titles[]` portant `title`, `id`, `cid`, `legalStatus`.
pub(crate) fn pick_texte(resp: &Value, nature: &str, numero: &str) -> TexteLookup {
    let wanted_nature = match nature {
        "loi" => "loi",
        "ordonnance" => "ordonnance",
        _ => "decret",
    };
    for res in resp
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let res_etat = str_field(res, "etat").or_else(|| str_field(res, "legalStatus"));
        let res_date =
            str_field(res, "dateSignature").map(|d| d.chars().take(10).collect::<String>());
        // Candidats (titre, id, état) : le résultat lui-même et ses `titles`.
        let mut cands: Vec<(String, Option<String>, Option<String>)> = Vec::new();
        if let Some(t) = str_field(res, "title") {
            cands.push((t, str_field(res, "id"), res_etat.clone()));
        }
        for t in res
            .get("titles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(title) = str_field(t, "title") {
                cands.push((
                    title,
                    str_field(t, "id").or_else(|| str_field(t, "cid")),
                    str_field(t, "legalStatus")
                        .or_else(|| str_field(t, "etat"))
                        .or_else(|| res_etat.clone()),
                ));
            }
        }
        for (title, id, etat) in cands {
            let clean = strip_tags(&title);
            let folded = fold(&clean);
            if folded.starts_with(wanted_nature) && contains_numero(&folded, numero) {
                if let Some(id) = id {
                    return TexteLookup::Trouve {
                        id,
                        titre: clean,
                        etat,
                        date: res_date,
                    };
                }
            }
        }
    }
    TexteLookup::Introuvable
}

/// Clé du champ de recherche « numéro de pourvoi » dans la taxonomie Judilibre
/// (`GET /taxonomy?id=field`). La spec précise que sans `field`, la recherche exclut
/// les métadonnées : on cible donc le numéro quand la taxonomie le propose. La valeur
/// est lue à l'exécution plutôt que supposée.
pub(crate) fn number_field_from_taxonomy(v: &Value) -> Option<String> {
    // `result` : table { clé: intitulé }, liste [{key, value}] ou élément unique.
    let result = v.get("result").unwrap_or(v);
    let mut pairs: Vec<(String, String)> = Vec::new();
    match result {
        Value::Object(map) if map.contains_key("key") && map.contains_key("value") => {
            if let (Some(k), Some(l)) = (str_field(result, "key"), str_field(result, "value")) {
                pairs.push((k, l));
            }
        }
        Value::Object(map) => {
            for (k, label) in map {
                if let Some(l) = label.as_str() {
                    pairs.push((k.clone(), l.to_string()));
                }
            }
        }
        Value::Array(items) => {
            for it in items {
                if let (Some(k), Some(l)) = (str_field(it, "key"), str_field(it, "value")) {
                    pairs.push((k, l));
                }
            }
        }
        _ => {}
    }
    pairs
        .iter()
        .find(|(_, label)| fold(label).contains("pourvoi"))
        .or_else(|| pairs.iter().find(|(k, _)| k == "number"))
        .map(|(k, _)| k.clone())
}

/// Clé du champ « ECLI » dans la taxonomie Judilibre `field` : l'entrée dont l'intitulé ou
/// la clé est « ECLI ». `None` si la taxonomie n'en propose pas (la recherche se fait alors
/// en texte intégral).
pub(crate) fn ecli_field_from_taxonomy(v: &Value) -> Option<String> {
    let result = v.get("result").unwrap_or(v);
    let mut pairs: Vec<(String, String)> = Vec::new();
    match result {
        Value::Object(map) if map.contains_key("key") && map.contains_key("value") => {
            if let (Some(k), Some(l)) = (str_field(result, "key"), str_field(result, "value")) {
                pairs.push((k, l));
            }
        }
        Value::Object(map) => {
            for (k, label) in map {
                if let Some(l) = label.as_str() {
                    pairs.push((k.clone(), l.to_string()));
                }
            }
        }
        Value::Array(items) => {
            for it in items {
                if let (Some(k), Some(l)) = (str_field(it, "key"), str_field(it, "value")) {
                    pairs.push((k, l));
                }
            }
        }
        _ => {}
    }
    pairs
        .iter()
        .find(|(k, l)| k.eq_ignore_ascii_case("ecli") || fold(l).contains("ecli"))
        .map(|(k, _)| k.clone())
}

/// Réponse `/consult/kaliContIdcc`. `Err` si la forme est inattendue (pas de verdict).
pub(crate) fn parse_idcc(v: &Value) -> Result<IdccLookup, String> {
    let root = v.get("kaliCont").unwrap_or(v);
    let id = str_field(root, "id").or_else(|| str_field(root, "cid"));
    let titre = str_field(root, "titre")
        .or_else(|| str_field(root, "title"))
        .or_else(|| str_field(root, "nom"))
        .or_else(|| str_field(root, "intitule"));
    let etat = str_field(root, "etat").or_else(|| str_field(root, "legalStatus"));
    match id {
        Some(id) => Ok(IdccLookup::Trouve { id, titre, etat }),
        None => Err("réponse KALI inattendue".to_string()),
    }
}

/// Texte d'un article dans une réponse `/consult/getArticle`.
pub(crate) fn article_text_of(v: &Value) -> Option<String> {
    let a = v.get("article").unwrap_or(v);
    let raw = str_field(a, "texte").or_else(|| str_field(a, "texteHtml"))?;
    let t = html_to_text(&raw);
    (!t.trim().is_empty()).then_some(t)
}

// ── Source de vérité ──────────────────────────────────────────────────────────

/// Interrogations PISTE supplémentaires. Comme `ArticleLookup`, le trait isole la
/// logique de l'API réseau : les tests injectent un faux. `Err` = la vérification
/// n'a pas pu aboutir — jamais un verdict.
pub trait ExtLookup: Send + Sync {
    /// `key` : chiffres du pourvoi ; `display` : « 22-12.345 ».
    fn decision<'a>(
        &'a self,
        key: &'a str,
        display: &'a str,
    ) -> BoxFuture<'a, Result<DecisionLookup, String>>;
    /// `nature` : « loi », « ordonnance », « decret » ; `numero` : « 2020-1455 ».
    fn texte<'a>(
        &'a self,
        nature: &'a str,
        numero: &'a str,
    ) -> BoxFuture<'a, Result<TexteLookup, String>>;
    fn idcc<'a>(&'a self, idcc: &'a str) -> BoxFuture<'a, Result<IdccLookup, String>>;
    /// Décision de toute juridiction (ECLI, Conseil d'État, juridictions administratives,
    /// cours d'appel, ou simple numéro de pourvoi). Par défaut : seuls les numéros de
    /// pourvoi sont traités (via `decision`), le reste est « non disponible ».
    fn decision_ref<'a>(
        &'a self,
        q: &'a DecisionQuery,
    ) -> BoxFuture<'a, Result<DecisionLookup, String>> {
        if let Some(key) = q.pourvoi_key() {
            return self.decision(key, &q.display);
        }
        Box::pin(async { Err(NO_SOURCE.to_string()) })
    }
    /// Texte d'un article (identifiant LEGIARTI).
    fn article_text<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<String, String>>;
    /// Identifiant de la version de l'article `id` applicable à `date` (droit dans le temps) :
    /// `Ok(None)` = aucune version en vigueur à cette date ; `Err` = non vérifiable.
    /// Par défaut : non disponible.
    fn article_version_at<'a>(
        &'a self,
        _id: &'a str,
        _date: chrono::NaiveDate,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async { Err(NO_SOURCE.to_string()) })
    }
}

const NO_SOURCE: &str = "aucune source configurée";

/// Source « hors service » : aucune interrogation possible (aucun appel émis).
pub struct NoExt;

impl ExtLookup for NoExt {
    fn decision<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<DecisionLookup, String>> {
        Box::pin(async { Err(NO_SOURCE.to_string()) })
    }
    fn texte<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<TexteLookup, String>> {
        Box::pin(async { Err(NO_SOURCE.to_string()) })
    }
    fn idcc<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<IdccLookup, String>> {
        Box::pin(async { Err(NO_SOURCE.to_string()) })
    }
    fn article_text<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async { Err(NO_SOURCE.to_string()) })
    }
}

/// Clé du champ « numéro de pourvoi » par serveur Judilibre (`None` : la taxonomie n'en
/// propose pas). Mémorisée pour la durée du processus ; un échec réseau n'est pas mémorisé.
static NUMBER_FIELDS: Lazy<std::sync::Mutex<HashMap<String, Option<String>>>> =
    Lazy::new(Default::default);
/// Clé du champ « ECLI » par serveur Judilibre, lue dans la même taxonomie que le champ
/// « numéro de pourvoi » (donc remplie en même temps que `NUMBER_FIELDS`).
static ECLI_FIELDS: Lazy<std::sync::Mutex<HashMap<String, Option<String>>>> =
    Lazy::new(Default::default);
/// Sièges des cours d'appel (taxonomie Judilibre `location`, contexte `ca`) par serveur :
/// couples (clé, intitulé).
type SeatEntries = Vec<(String, String)>;
static CA_LOCATIONS: Lazy<std::sync::Mutex<HashMap<String, SeatEntries>>> =
    Lazy::new(Default::default);

/// Adaptateur PISTE (Judilibre + Légifrance).
pub struct PisteExt {
    client: PisteClient,
}

impl PisteExt {
    pub fn new(client: PisteClient) -> Self {
        Self { client }
    }

    /// Clé du champ « numéro de pourvoi » (taxonomie Judilibre), mémorisée par serveur.
    async fn number_field(&self) -> Option<String> {
        let base = self.client.judilibre_base().to_string();
        if let Some(known) = NUMBER_FIELDS
            .lock()
            .ok()
            .and_then(|m| m.get(&base).cloned())
        {
            return known;
        }
        match judilibre::get_taxonomy(&self.client, "field", None).await {
            Ok(v) => {
                let field = number_field_from_taxonomy(&v);
                if let Ok(mut m) = ECLI_FIELDS.lock() {
                    m.insert(base.clone(), ecli_field_from_taxonomy(&v));
                }
                if let Ok(mut m) = NUMBER_FIELDS.lock() {
                    m.insert(base, field.clone());
                }
                field
            }
            // Échec passager : on ne mémorise rien et on se rabat sur le texte intégral.
            Err(_) => None,
        }
    }

    /// Champs de la taxonomie Judilibre à cibler pour une recherche par ECLI : le champ
    /// « ECLI » s'il existe, puis le champ « numéro » (lus dans la même taxonomie).
    async fn ecli_fields(&self) -> Vec<String> {
        let number = self.number_field().await;
        let base = self.client.judilibre_base().to_string();
        let ecli = ECLI_FIELDS
            .lock()
            .ok()
            .and_then(|m| m.get(&base).cloned())
            .flatten();
        let mut out: Vec<String> = ecli.into_iter().collect();
        out.extend(number.filter(|n| !out.contains(n)));
        out
    }

    /// Clé du siège d'une cour d'appel dans la taxonomie `location` (contexte `ca`).
    async fn ca_location(&self, place: &str) -> Option<String> {
        let base = self.client.judilibre_base().to_string();
        let cached = CA_LOCATIONS.lock().ok().and_then(|m| m.get(&base).cloned());
        let entries = match cached {
            Some(v) => v,
            None => {
                let v = judilibre::get_taxonomy(&self.client, "location", Some("ca"))
                    .await
                    .ok()?;
                let entries = verify_juris::taxonomy_entries(&v);
                if !entries.is_empty() {
                    if let Ok(mut m) = CA_LOCATIONS.lock() {
                        m.insert(base, entries.clone());
                    }
                }
                entries
            }
        };
        verify_juris::location_key_for(&entries, place)
    }

    async fn decision_ref_impl(&self, q: &DecisionQuery) -> Result<DecisionLookup, String> {
        use verify_juris::{judilibre_by_ecli, judilibre_by_rg, legifrance_juri, narrow_ca};
        // Cour de cassation, par numéro de pourvoi : chemin historique, inchangé.
        if let Some(key) = q.pourvoi_key() {
            return self.decision_impl(key, &q.display).await;
        }
        match (q.court, &q.ecli, &q.number) {
            (Court::Cassation | Court::CourAppel, Some(ecli), _) => {
                let jur = if q.court == Court::Cassation {
                    "cc"
                } else {
                    "ca"
                };
                let fields = self.ecli_fields().await;
                Ok(
                    match judilibre_by_ecli(&self.client, ecli, jur, fields).await? {
                        Some(info) => DecisionLookup::Trouvee(info),
                        None => DecisionLookup::Introuvable,
                    },
                )
            }
            (Court::CourAppel, None, Some(rg)) => {
                let key = digits_of(rg);
                let field = self.number_field().await;
                let all = judilibre_by_rg(&self.client, &key, rg, field.clone(), None).await?;
                if all.is_empty() {
                    return Ok(DecisionLookup::Introuvable);
                }
                // Siège cité : appliqué comme filtre de recherche s'il est identifiable.
                let at_seat = match &q.place {
                    Some(place) => match self.ca_location(place).await {
                        Some(loc) => {
                            Some(judilibre_by_rg(&self.client, &key, rg, field, Some(loc)).await?)
                        }
                        None => None,
                    },
                    None => None,
                };
                Ok(narrow_ca(all, at_seat, q.date.as_deref()))
            }
            (court, _, _) if court.is_admin() => {
                let res = legifrance_juri(&self.client, q).await?;
                // « ECLI:FR:CAA… » peut aussi désigner une cour d'appel judiciaire
                // (Agen, Aix, Amiens…) : en dernier recours, Judilibre.
                if court == Court::CourAdminAppel && q.ecli.is_some() {
                    if let DecisionLookup::Introuvable = res {
                        let ecli = q.ecli.as_deref().unwrap_or_default();
                        let fields = self.ecli_fields().await;
                        return Ok(
                            match judilibre_by_ecli(&self.client, ecli, "ca", fields).await? {
                                Some(info) => DecisionLookup::Trouvee(info),
                                None => DecisionLookup::Introuvable,
                            },
                        );
                    }
                }
                Ok(res)
            }
            _ => Err(NO_SOURCE.to_string()),
        }
    }

    async fn decision_impl(&self, key: &str, display: &str) -> Result<DecisionLookup, String> {
        // 1. recherche ciblée sur le numéro de pourvoi (si la taxonomie le propose) ;
        // 2. texte intégral, même écriture ; 3. écriture sans point (« 22-12345 »).
        let mut attempts: Vec<(String, Option<Vec<String>>)> = Vec::new();
        if let Some(f) = self.number_field().await {
            attempts.push((display.to_string(), Some(vec![f])));
        }
        attempts.push((display.to_string(), None));
        attempts.push((format!("{}-{}", &key[0..2], &key[2..]), None));
        for (query, field) in attempts {
            let params = JudilibreSearchParams {
                query,
                operator: Some("exact".into()),
                field,
                batch: 0,
                batch_size: 10,
                resolve_references: true,
                ..Default::default()
            };
            let resp = judilibre::search(&self.client, params)
                .await
                .map_err(|e| e.to_string())?;
            // `relaxed` (requête élargie automatiquement) : sans effet ici, on ne retient
            // que les décisions dont le numéro correspond exactement.
            if let Some(info) = pick_decision(&resp.results.unwrap_or_default(), key) {
                return Ok(DecisionLookup::Trouvee(info));
            }
        }
        Ok(DecisionLookup::Introuvable)
    }

    async fn texte_impl(&self, nature: &str, numero: &str) -> Result<TexteLookup, String> {
        // Recette officielle : champ NUM (« numéros de texte »), recherche exacte.
        let first = legifrance::search_simple(
            &self.client,
            numero,
            SearchFond::LodaEtat,
            1,
            20,
            None,
            None,
            None,
            None,
            Some("NUM".into()),
            None,
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
        let v = serde_json::to_value(first).map_err(|e| e.to_string())?;
        if let found @ TexteLookup::Trouve { .. } = pick_texte(&v, nature, numero) {
            return Ok(found);
        }
        // Repli : texte intégral. Son échec n'invalide pas la première réponse.
        if let Ok(second) = legifrance::search_simple(
            &self.client,
            numero,
            SearchFond::LodaEtat,
            1,
            20,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        {
            if let Ok(v2) = serde_json::to_value(second) {
                return Ok(pick_texte(&v2, nature, numero));
            }
        }
        Ok(TexteLookup::Introuvable)
    }

    async fn idcc_impl(&self, idcc: &str) -> Result<IdccLookup, String> {
        // Consultation directe du conteneur (`/consult/kaliContIdcc`, propriété `id` = numéro
        // IDCC) : 404 documenté = convention inconnue. La recherche KALI par IDCC ne convient
        // pas : elle renvoie les TEXTES de la convention (avenants…) avec `idcc` vide, et leur
        // état n'est pas celui de la convention. « 0016 » puis « 16 » : l'écriture attendue
        // pour les IDCC à zéros de tête n'étant pas documentée.
        let stripped = idcc.trim_start_matches('0');
        let mut spellings = vec![idcc.to_string()];
        if !stripped.is_empty() && stripped != idcc {
            spellings.push(stripped.to_string());
        }
        for sp in spellings {
            match legifrance::get_kali_cont_idcc(&self.client, &sp).await {
                Ok(v) => return parse_idcc(&v),
                Err(e) => {
                    let m = e.to_string();
                    if !m.contains("HTTP 404") {
                        return Err(m);
                    }
                }
            }
        }
        Ok(IdccLookup::Introuvable)
    }

    async fn article_version_at_impl(
        &self,
        id: &str,
        date: chrono::NaiveDate,
    ) -> Result<Option<String>, String> {
        let v = legifrance::get_article(&self.client, id)
            .await
            .map_err(|e| e.to_string())?;
        super::temporal::version_applicable_id(&v, date)
    }

    async fn article_text_impl(&self, id: &str) -> Result<String, String> {
        let v = legifrance::get_article(&self.client, id)
            .await
            .map_err(|e| e.to_string())?;
        article_text_of(&v).ok_or_else(|| "texte de l'article absent de la réponse".to_string())
    }
}

impl ExtLookup for PisteExt {
    fn decision<'a>(
        &'a self,
        key: &'a str,
        display: &'a str,
    ) -> BoxFuture<'a, Result<DecisionLookup, String>> {
        Box::pin(self.decision_impl(key, display))
    }
    fn decision_ref<'a>(
        &'a self,
        q: &'a DecisionQuery,
    ) -> BoxFuture<'a, Result<DecisionLookup, String>> {
        Box::pin(self.decision_ref_impl(q))
    }
    fn texte<'a>(
        &'a self,
        nature: &'a str,
        numero: &'a str,
    ) -> BoxFuture<'a, Result<TexteLookup, String>> {
        Box::pin(self.texte_impl(nature, numero))
    }
    fn idcc<'a>(&'a self, idcc: &'a str) -> BoxFuture<'a, Result<IdccLookup, String>> {
        Box::pin(self.idcc_impl(idcc))
    }
    fn article_text<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(self.article_text_impl(id))
    }
    fn article_version_at<'a>(
        &'a self,
        id: &'a str,
        date: chrono::NaiveDate,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(self.article_version_at_impl(id, date))
    }
}

/// Cache des réponses *abouties* (jamais des erreurs), par jour, dans le cache
/// partagé du module juridique.
pub struct CachedExt<L: ExtLookup> {
    inner: L,
    cache: Cache,
    day: String,
}

impl<L: ExtLookup> CachedExt<L> {
    pub fn new(inner: L, cache: Cache, day: String) -> Self {
        Self { inner, cache, day }
    }

    async fn cached<T, F>(&self, key: String, fetch: F) -> Result<T, String>
    where
        T: Serialize + for<'de> Deserialize<'de>,
        F: std::future::Future<Output = Result<T, String>>,
    {
        let key = format!("vext:{}:{key}", self.day);
        if let Some(v) = cache_get(&self.cache, &key, TTL_EXT).await {
            if let Ok(hit) = serde_json::from_value::<T>(v) {
                return Ok(hit);
            }
        }
        let res = fetch.await;
        if let Ok(ok) = &res {
            if let Ok(v) = serde_json::to_value(ok) {
                cache_set(&self.cache, key, v).await;
            }
        }
        res
    }
}

impl<L: ExtLookup> ExtLookup for CachedExt<L> {
    fn decision<'a>(
        &'a self,
        key: &'a str,
        display: &'a str,
    ) -> BoxFuture<'a, Result<DecisionLookup, String>> {
        Box::pin(self.cached(format!("dec:{key}"), self.inner.decision(key, display)))
    }
    fn decision_ref<'a>(
        &'a self,
        q: &'a DecisionQuery,
    ) -> BoxFuture<'a, Result<DecisionLookup, String>> {
        // Un numéro de pourvoi partage l'entrée de cache de `decision`.
        if let Some(key) = q.pourvoi_key() {
            return self.decision(key, &q.display);
        }
        Box::pin(self.cached(
            format!("decq:{}", q.cache_key()),
            self.inner.decision_ref(q),
        ))
    }
    fn texte<'a>(
        &'a self,
        nature: &'a str,
        numero: &'a str,
    ) -> BoxFuture<'a, Result<TexteLookup, String>> {
        Box::pin(self.cached(
            format!("txt:{nature}:{numero}"),
            self.inner.texte(nature, numero),
        ))
    }
    fn idcc<'a>(&'a self, idcc: &'a str) -> BoxFuture<'a, Result<IdccLookup, String>> {
        Box::pin(self.cached(format!("idcc:{idcc}"), self.inner.idcc(idcc)))
    }
    fn article_text<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(self.cached(format!("art:{id}"), self.inner.article_text(id)))
    }
    fn article_version_at<'a>(
        &'a self,
        id: &'a str,
        date: chrono::NaiveDate,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(self.cached(
            format!("artv:{id}:{date}"),
            self.inner.article_version_at(id, date),
        ))
    }
}

// ── Rapport ───────────────────────────────────────────────────────────────────

/// Ce que `extend_report_at` reçoit en plus du rapport de base.
pub struct ExtInput<'a> {
    /// Texte analysé (les offsets des références s'y rapportent).
    pub text: &'a str,
    pub article_refs: &'a [ArticleRef],
    pub ext: &'a ExtRefs,
    pub grounding: Option<&'a GroundingIndex>,
    /// Textes fournis au modèle ; vide si l'ancrage n'a pas été demandé.
    pub sources: &'a [String],
    /// Article désigné par la question, quand la réponse n'en cite aucun.
    pub context_article: Option<&'a ContextArticle>,
}

fn blank(kind: Kind, reference: String, numero: String) -> Verification {
    Verification {
        kind,
        ancrage: None,
        reference,
        code: None,
        numero,
        statut: Statut::NonVerifiable,
        existe: None,
        etat: None,
        etat_libelle: None,
        date_fin: None,
        url: None,
        existe_dans: Vec::new(),
        detail: None,
        infos: Vec::new(),
        decision_id: None,
        occurrences: Vec::new(),
        intern: Intern::default(),
    }
}

/// Libellé court d'une référence : espaces normalisés, plafonné.
fn short_label(raw: &str) -> String {
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 110 {
        format!("{}…", one_line.chars().take(110).collect::<String>())
    } else {
        one_line
    }
}

fn excerpt(s: &str, words: usize) -> String {
    let w: Vec<&str> = s.split_whitespace().collect();
    if w.len() <= words {
        w.join(" ")
    } else {
        format!("{}…", w[..words].join(" "))
    }
}

fn technical_error(v: &mut Verification, source: &str, e: &str) {
    // PISTE non configuré : mode dégradé normal, pas un incident à journaliser à chaque réponse.
    if e != NO_SOURCE {
        warn!("[Vérification] {} : {}", v.reference, e);
    }
    v.statut = Statut::Erreur;
    v.detail = Some(format!("{source} n'a pas pu être interrogé"));
}

/// Identifiant de base d'un texte : Légifrance renvoie parfois l'identifiant de VERSION
/// (« LEGITEXT000042477679_30-10-2020 »), qui n'ouvre pas de page ; sans le suffixe,
/// le lien mène au texte, puis à sa version en vigueur.
fn text_base_id(id: &str) -> &str {
    id.split('_').next().unwrap_or(id)
}

fn legifrance_text_url(id: &str) -> Option<String> {
    let id = text_base_id(id);
    if id.starts_with("LEGITEXT") {
        Some(format!("https://www.legifrance.gouv.fr/loda/id/{id}"))
    } else if id.starts_with("JORFTEXT") {
        Some(format!("https://www.legifrance.gouv.fr/jorf/id/{id}"))
    } else {
        None
    }
}

fn is_vigueur(etat: &str) -> bool {
    etat.eq_ignore_ascii_case("VIGUEUR")
}

/// Complète `report` : vérifie les textes hors codes cités par numéro (entrées
/// déjà présentes), puis ajoute décisions, conventions collectives et citations.
#[cfg(test)]
pub async fn extend_report(lookup: &dyn ExtLookup, inp: ExtInput<'_>, report: &mut Report) {
    extend_report_at(lookup, inp, report, None).await;
}

/// Comme `extend_report`, avec une date des faits : les citations textuelles rattachées à
/// un article sont alors comparées à la rédaction applicable à cette date, et non au texte actuel.
pub async fn extend_report_at(
    lookup: &dyn ExtLookup,
    inp: ExtInput<'_>,
    report: &mut Report,
    date: Option<chrono::NaiveDate>,
) {
    apply_textes(lookup, report).await;
    apply_decisions(lookup, &inp, report).await;
    apply_idcc(lookup, &inp, report).await;
    apply_quotes(lookup, &inp, report, date).await;
}

async fn apply_textes(lookup: &dyn ExtLookup, report: &mut Report) {
    let mut jobs: Vec<(usize, String, String)> = Vec::new();
    for (i, v) in report.verifications.iter_mut().enumerate() {
        if v.kind != Kind::Texte {
            continue;
        }
        let Some((nature, numero, _)) = v.intern.texte.clone() else {
            continue;
        };
        if !texte_is_lookupable(&nature, Some(&numero)) {
            continue;
        }
        if jobs.len() >= MAX_TEXTE_LOOKUPS {
            v.detail = Some("Limite du nombre de vérifications par réponse atteinte".into());
            continue;
        }
        jobs.push((i, nature, numero));
    }
    let mut outcomes: Vec<(usize, Result<TexteLookup, String>)> = stream::iter(jobs)
        .map(|(i, nature, numero)| async move {
            let res = lookup.texte(&nature, &numero).await;
            (i, res)
        })
        .buffer_unordered(EXT_CONCURRENCY)
        .collect()
        .await;
    outcomes.sort_by_key(|(i, _)| *i);

    for (i, res) in outcomes {
        let v = &mut report.verifications[i];
        match res {
            Ok(TexteLookup::Trouve {
                id,
                titre,
                etat,
                date,
            }) => {
                v.existe = Some(true);
                v.detail = None;
                v.url = legifrance_text_url(&id);
                v.infos = vec![strip_tags(&titre)];
                v.etat_libelle = etat.as_deref().map(etat_libelle);
                v.statut = match etat.as_deref() {
                    Some(e) if is_vigueur(e) => Statut::EnVigueur,
                    Some(_) => Statut::Abroge,
                    None => Statut::Verifiee,
                };
                v.etat = etat;
                // Un numéro exact avec une autre date : référence mal recopiée.
                let cited = v.intern.texte.as_ref().and_then(|(_, _, d)| d.clone());
                let real = date.or_else(|| first_date(&titre).map(|(_, _, d)| d));
                if let (Some(cited), Some(real)) = (cited, real) {
                    if cited != real {
                        v.statut = Statut::Incoherente;
                        v.detail = Some(format!(
                            "La date citée ({}) ne correspond pas à celle du texte portant ce numéro ({}).",
                            fr_date(&cited),
                            fr_date(&real)
                        ));
                    }
                }
            }
            Ok(TexteLookup::Introuvable) => {
                v.statut = Statut::NonRetrouvee;
                v.detail = Some(
                    "Aucun texte de ce numéro n'a été retrouvé dans Légifrance (recherche par numéro) : à vérifier."
                        .into(),
                );
            }
            Err(e) => technical_error(v, "Légifrance", &e),
        }
    }
}

async fn apply_decisions(lookup: &dyn ExtLookup, inp: &ExtInput<'_>, report: &mut Report) {
    // Regroupement par numéro de pourvoi, dans l'ordre de première apparition.
    let mut order: Vec<&DecisionRef> = Vec::new();
    let mut groups: HashMap<&str, Verification> = HashMap::new();
    let mut room = MAX_OCCURRENCES.saturating_sub(
        report
            .verifications
            .iter()
            .map(|v| v.occurrences.len())
            .sum(),
    );
    for d in &inp.ext.decisions {
        if room == 0 {
            break;
        }
        room -= 1;
        let v = groups.entry(d.key.as_str()).or_insert_with(|| {
            order.push(d);
            let mut v = blank(Kind::Decision, short_label(&d.raw), d.display.clone());
            v.ancrage = inp.grounding.map(|g| {
                if g.has_decision(&d.key) {
                    Ancrage::DansSources
                } else {
                    Ancrage::HorsSources
                }
            });
            v
        });
        v.occurrences.push(Occurrence {
            debut: utf16_offset(inp.text, d.start),
            fin: utf16_offset(inp.text, d.end),
            texte: d.raw.clone(),
        });
    }

    // Les juridictions reconnues mais non interrogées (ECLI du Conseil constitutionnel, de la
    // CJUE…) ne coûtent aucun appel et ne comptent pas dans la limite.
    // Valeurs possédées (et non des références) : le futur reste `Send` dans le handler.
    let jobs: Vec<(usize, DecisionQuery)> = order
        .iter()
        .enumerate()
        .filter(|(_, d)| d.court != Court::Autre)
        .take(MAX_DECISION_LOOKUPS)
        .map(|(i, d)| (i, d.query()))
        .collect();
    let mut outcomes: Vec<(usize, Result<DecisionLookup, String>)> = stream::iter(jobs)
        .map(|(i, q)| async move {
            let res = lookup.decision_ref(&q).await;
            (i, res)
        })
        .buffer_unordered(EXT_CONCURRENCY)
        .collect()
        .await;
    outcomes.sort_by_key(|(i, _)| *i);
    let mut outcome_by_idx: HashMap<usize, Result<DecisionLookup, String>> =
        outcomes.into_iter().collect();

    for (i, d) in order.iter().enumerate() {
        let mut v = groups.remove(d.key.as_str()).expect("groupe décision");
        if d.court == Court::Autre {
            v.detail = Some(unqueried_detail(d));
            report.verifications.push(v);
            continue;
        }
        match outcome_by_idx.remove(&i) {
            None => {
                v.detail = Some("Limite du nombre de vérifications par réponse atteinte".into());
            }
            Some(Ok(DecisionLookup::Trouvee(info))) => {
                apply_decision_found(&mut v, d, &info);
                // Cité par ECLI (ou par un autre numéro), le même arrêt peut figurer dans les
                // sources fournies au modèle sous son numéro : ce n'est pas « hors sources ».
                if let (Some(g), Some(Ancrage::HorsSources)) = (inp.grounding, v.ancrage) {
                    if alt_grounding_keys(d, &info)
                        .iter()
                        .any(|k| g.has_decision(k))
                    {
                        v.ancrage = Some(Ancrage::DansSources);
                    }
                }
            }
            Some(Ok(DecisionLookup::Introuvable)) => {
                v.statut = Statut::NonRetrouvee;
                v.detail = Some(not_found_detail(d));
            }
            Some(Ok(DecisionLookup::Incertaine { raison })) => {
                v.statut = Statut::NonRetrouvee;
                v.detail = Some(raison);
            }
            Some(Err(e)) => technical_error(&mut v, d.court.source().unwrap_or("La source"), &e),
        }
        report.verifications.push(v);
    }
}

/// Message d'un ECLI d'une juridiction que Juridix n'interroge pas : la référence n'est
/// PAS vérifiée (ni confirmée ni infirmée), et le message dit où la contrôler.
fn unqueried_detail(d: &DecisionRef) -> String {
    let ecli = d.ecli.as_deref().unwrap_or(&d.display);
    let mut parts = ecli.split(':').skip(1);
    let (country, code) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let ou = match (country, code) {
        ("EU", _) => {
            "Cour de justice de l'Union européenne : à contrôler sur EUR-Lex (eur-lex.europa.eu)"
        }
        ("CE", _) => {
            "Cour européenne des droits de l'homme : à contrôler sur HUDOC (hudoc.echr.coe.int)"
        }
        ("FR", "CC") => "Conseil constitutionnel : à contrôler sur conseil-constitutionnel.fr",
        _ => "à contrôler sur le site de la juridiction concernée",
    };
    format!(
        "Non vérifiée par Juridix : cette juridiction n'est pas interrogée, l'existence de la référence n'est ni confirmée ni infirmée. {ou}."
    )
}

/// Message d'une décision non retrouvée : recherche aboutie, aucune décision correspondante.
/// Ce n'est jamais présenté comme une preuve d'inexistence.
fn not_found_detail(d: &DecisionRef) -> String {
    match (d.court, d.ecli.is_some()) {
        (Court::Cassation, false) => "Aucune décision portant ce numéro de pourvoi n'a été retrouvée dans Judilibre, qui n'est pas exhaustif : à vérifier.".into(),
        (Court::Cassation, true) => "Aucune décision portant cet ECLI n'a été retrouvée dans Judilibre, qui n'est pas exhaustif : à vérifier.".into(),
        (Court::CourAppel, _) => "Aucun arrêt correspondant n'a été retrouvé dans Judilibre, qui ne couvre qu'une partie des décisions des cours d'appel : à vérifier.".into(),
        (c, _) if c.is_admin() => "Aucune décision correspondante n'a été retrouvée dans la jurisprudence administrative de Légifrance, qui n'est pas exhaustive (notamment pour les cours et tribunaux) : à vérifier.".into(),
        _ => "Décision non retrouvée : à vérifier.".into(),
    }
}

/// Clés d'ancrage (`dec|…`) sous lesquelles la décision retrouvée peut figurer dans les
/// sources : son numéro, tel que la source officielle le donne.
fn alt_grounding_keys(d: &DecisionRef, info: &DecisionInfo) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(k) = verify_juris::number_key(d.court, &info.number) {
        keys.push(k);
    }
    // ECLI de la Cour de cassation / d'une cour d'appel : le numéro renvoyé par Judilibre
    // peut être un pourvoi ou un RG.
    for court in [
        Court::Cassation,
        Court::CourAppel,
        Court::ConseilEtat,
        Court::CourAdminAppel,
    ] {
        if let Some(k) = verify_juris::number_key(court, &info.number) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
    }
    keys
}

fn apply_decision_found(v: &mut Verification, d: &DecisionRef, info: &DecisionInfo) {
    v.existe = Some(true);
    v.url = info.url.clone().or_else(|| {
        Some(format!(
            "https://www.courdecassation.fr/decision/{}",
            info.id
        ))
    });
    v.statut = Statut::Verifiee;
    v.decision_id = Some(info.id.clone());

    let mut infos: Vec<String> = Vec::new();
    let court = match d.court {
        Court::Cassation => match (&info.chamber, &info.formation) {
            (Some(c), _) => format!("Cour de cassation — {c}"),
            (None, Some(f)) => format!("Cour de cassation — {f}"),
            _ => "Cour de cassation".to_string(),
        },
        other => match &info.chamber {
            Some(c) => format!("{} — {c}", other.label()),
            None => other.label().to_string(),
        },
    };
    infos.push(court);
    if let Some(date) = &info.date {
        infos.push(format!("Décision du {}", fr_date(date)));
    }
    if let Some(s) = &info.solution {
        infos.push(format!("Solution : {s}"));
    }
    if !info.publication.is_empty() {
        infos.push(format!("Publication : {}", info.publication.join(", ")));
    }
    if let Some(src) = info.source.as_deref().or_else(|| d.court.source()) {
        infos.push(format!("Source : {src}"));
    }
    if let Some(note) = &info.note {
        infos.push(note.clone());
    }
    v.infos = infos;

    let mut problems: Vec<String> = Vec::new();
    if let (Some(cited), Some(real)) = (&d.date, &info.date) {
        let real10: String = real.chars().take(10).collect();
        if *cited != real10 {
            problems.push(format!(
                "La date citée ({}) ne correspond pas à celle de la décision ({}).",
                fr_date(cited),
                fr_date(&real10)
            ));
        }
    }
    if let (Some(cited), Some(real_label)) = (d.chambre, &info.chamber) {
        if let Some(real) = canon_chamber(real_label) {
            if cited != real {
                problems.push(format!(
                    "La chambre citée ne correspond pas à celle de la décision ({real_label})."
                ));
            }
        }
    }
    if !problems.is_empty() {
        v.statut = Statut::Incoherente;
        v.detail = Some(problems.join(" "));
    }
}

async fn apply_idcc(lookup: &dyn ExtLookup, inp: &ExtInput<'_>, report: &mut Report) {
    let mut order: Vec<&IdccRef> = Vec::new();
    let mut groups: HashMap<&str, Verification> = HashMap::new();
    for r in &inp.ext.idccs {
        let v = groups.entry(r.idcc.as_str()).or_insert_with(|| {
            order.push(r);
            let mut v = blank(Kind::Convention, format!("IDCC {}", r.idcc), r.idcc.clone());
            v.ancrage = inp.grounding.map(|g| {
                if g.has_idcc(&r.idcc) {
                    Ancrage::DansSources
                } else {
                    Ancrage::HorsSources
                }
            });
            v
        });
        v.occurrences.push(Occurrence {
            debut: utf16_offset(inp.text, r.start),
            fin: utf16_offset(inp.text, r.end),
            texte: r.raw.clone(),
        });
    }
    let to_check: Vec<(usize, &IdccRef)> = order
        .iter()
        .take(MAX_IDCC_LOOKUPS)
        .copied()
        .enumerate()
        .collect();
    let jobs: Vec<(usize, String)> = to_check
        .into_iter()
        .map(|(i, r)| (i, r.idcc.clone()))
        .collect();
    let outcomes: Vec<(usize, Result<IdccLookup, String>)> = stream::iter(jobs)
        .map(|(i, idcc)| async move {
            let res = lookup.idcc(&idcc).await;
            (i, res)
        })
        .buffer_unordered(EXT_CONCURRENCY)
        .collect()
        .await;
    let mut by_idx: HashMap<usize, Result<IdccLookup, String>> = outcomes.into_iter().collect();

    for (i, r) in order.iter().enumerate() {
        let mut v = groups.remove(r.idcc.as_str()).expect("groupe idcc");
        match by_idx.remove(&i) {
            None => {
                v.detail = Some("Limite du nombre de vérifications par réponse atteinte".into())
            }
            Some(Ok(IdccLookup::Trouve { id, titre, etat })) => {
                v.existe = Some(true);
                v.url = Some(format!("https://www.legifrance.gouv.fr/conv_coll/id/{id}"));
                v.infos = titre.into_iter().collect();
                v.etat_libelle = etat.as_deref().map(etat_libelle);
                v.statut = match etat.as_deref() {
                    Some(e) if is_vigueur(e) => Statut::EnVigueur,
                    Some(_) => Statut::Abroge,
                    None => Statut::Verifiee,
                };
                v.etat = etat;
            }
            Some(Ok(IdccLookup::Introuvable)) => {
                v.statut = Statut::NonRetrouvee;
                v.detail = Some(format!(
                    "Aucune convention collective d'IDCC {} n'a été retrouvée dans Légifrance : à vérifier.",
                    r.idcc
                ));
            }
            Some(Err(e)) => technical_error(&mut v, "Légifrance", &e),
        }
        report.verifications.push(v);
    }
}

/// Article de code dont on compare le texte à une citation.
struct Target {
    id: String,
    /// « Code civil, art. 1240 »
    reference: String,
    /// État en clair.
    etat: Option<String>,
    code: Option<String>,
    /// Désigné par la question et non par la réponse.
    from_question: bool,
}

async fn apply_quotes(
    lookup: &dyn ExtLookup,
    inp: &ExtInput<'_>,
    report: &mut Report,
    date: Option<chrono::NaiveDate>,
) {
    let quotes: Vec<&QuoteRef> = inp.ext.quotes.iter().take(MAX_QUOTES).collect();
    if quotes.is_empty() {
        return;
    }

    // Article rattaché à chaque citation : par proximité dans la réponse, sinon, quand la
    // réponse ne cite aucun article, celui de la question.
    let attached: Vec<Option<usize>> = quotes
        .iter()
        .map(|q| {
            (q.words >= MIN_WORDS_ARTICLE)
                .then(|| attached_article(q, inp.article_refs, inp.text))
                .flatten()
        })
        .collect();
    let mut art_info: HashMap<String, (String, Option<String>, String)> = HashMap::new();
    // clé `code|numéro` → (id LEGIARTI, état en clair, libellé « Code civil, art. 1240 »)
    for v in &report.verifications {
        if let (Some(k), Some(id)) = (&v.intern.article_key, &v.intern.article_id) {
            art_info.insert(
                k.clone(),
                (id.clone(), v.etat_libelle.clone(), v.reference.clone()),
            );
        }
    }
    let key_of = |ri: usize| -> Option<String> {
        let r = &inp.article_refs[ri];
        Some(format!("{}|{}", r.code?, r.numero_key()))
    };
    let targets: Vec<Option<Target>> = quotes
        .iter()
        .enumerate()
        .map(|(qi, q)| {
            if let Some(ri) = attached[qi] {
                let r = &inp.article_refs[ri];
                key_of(ri)
                    .and_then(|k| art_info.get(&k).cloned())
                    .map(|(id, etat, reference)| Target {
                        id,
                        reference,
                        etat,
                        code: r.code.map(str::to_string),
                        from_question: false,
                    })
            } else if q.words >= MIN_WORDS_ARTICLE {
                inp.context_article.map(|c| Target {
                    id: c.id.clone(),
                    reference: c.reference.clone(),
                    etat: c.etat_libelle.clone(),
                    code: c.code.clone(),
                    from_question: true,
                })
            } else {
                None
            }
        })
        .collect();
    // Plusieurs citations et un seul article dans la question : on ne sait pas laquelle le
    // concerne. Seuls les verdicts positifs sont alors rendus (jamais « absent de l'article »).
    let question_quotes = targets.iter().flatten().filter(|t| t.from_question).count();

    let mut ids: Vec<String> = targets.iter().flatten().map(|t| t.id.clone()).collect();
    ids.sort();
    ids.dedup();
    ids.truncate(MAX_ARTICLE_TEXTS);

    // Droit dans le temps : version de chaque article applicable à la date des faits. Sans elle,
    // une citation fidèle à l'ancienne rédaction serait comparée au texte actuel et jugée fausse.
    let versions_at: HashMap<String, Result<Option<String>, String>> = match date {
        Some(d) => stream::iter(ids.clone())
            .map(|id| async move {
                let r = lookup.article_version_at(&id, d).await;
                (id, r)
            })
            .buffer_unordered(EXT_CONCURRENCY)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect(),
        None => HashMap::new(),
    };
    for r in versions_at.values() {
        if let Ok(Some(v)) = r {
            if !ids.contains(v) {
                ids.push(v.clone());
            }
        }
    }
    let date_fr = date.map(|d| d.format("%d/%m/%Y").to_string());

    let fetched: Vec<(String, Result<String, String>)> = stream::iter(ids)
        .map(|id| async move {
            let res = lookup.article_text(&id).await;
            (id, res)
        })
        .buffer_unordered(EXT_CONCURRENCY)
        .collect()
        .await;
    let article_raw: HashMap<String, String> = fetched
        .iter()
        .filter_map(|(id, r)| r.as_ref().ok().map(|t| (id.clone(), t.clone())))
        .collect();
    let article_hay: HashMap<String, Result<String, String>> = fetched
        .into_iter()
        .map(|(id, r)| (id, r.map(|t| haystack(&t))))
        .collect();

    // Documents fournis : préparés une seule fois, seulement si utiles.
    let sources_hay: Option<String> = (!inp.sources.is_empty() && inp.grounding.is_some())
        .then(|| haystack(&inp.sources.join("\n\n")));

    for (qi, q) in quotes.iter().enumerate() {
        let target = targets[qi].as_ref();
        let mut label = format!("Citation « {} »", excerpt(&q.text, 8));
        let mut code: Option<String> = None;
        let mut verdict: Option<(Statut, String, Vec<String>)> = None; // (statut, détail, infos)

        // 1. Rattachée à un article : texte officiel.
        let mut article_verdict: Option<QuoteMatch> = None;
        let mut desc = String::new();
        let mut from_question = false;
        // Identifiant du texte comparé (version applicable à la date des faits, sinon l'article courant).
        let mut cmp_id: Option<String> = target.map(|t| t.id.clone());
        // Verdict propre au droit dans le temps (prioritaire sur la comparaison ordinaire).
        let mut temporal_verdict: Option<(Statut, String)> = None;
        // Version à la date illisible : on ne rend qu'un verdict positif (jamais « absent »).
        let mut version_unknown = false;
        if let Some(t) = target {
            label = format!("Citation — {}", t.reference);
            code = t.code.clone();
            from_question = t.from_question;
            desc = format!(
                "{}{}{}",
                t.reference,
                t.etat
                    .as_deref()
                    .map(|e| format!(", {e}"))
                    .unwrap_or_default(),
                if t.from_question {
                    ", cité dans votre question"
                } else {
                    ""
                }
            );
            if let (Some(_), Some(df)) = (date, date_fr.as_deref()) {
                match versions_at.get(&t.id) {
                    Some(Ok(Some(v))) => {
                        // Rédaction applicable à la date : c'est elle qui fait foi.
                        desc = format!(
                            "{}, rédaction applicable au {df}{}",
                            t.reference,
                            if t.from_question {
                                ", cité dans votre question"
                            } else {
                                ""
                            }
                        );
                        cmp_id = Some(v.clone());
                    }
                    Some(Ok(None)) => {
                        if !t.from_question || question_quotes == 1 {
                            temporal_verdict = Some((
                                Statut::Incoherente,
                                format!(
                                    "Cet article n'était pas en vigueur au {df} ({}) : ce passage ne peut pas provenir de son texte applicable à cette date.",
                                    t.reference
                                ),
                            ));
                        }
                    }
                    _ => version_unknown = true,
                }
            }
            if let Some(Ok(hay)) = cmp_id.as_ref().and_then(|c| article_hay.get(c)) {
                article_verdict = Some(compare_quote(strip_article_label(&q.text), hay));
            }
            // Le passage figure dans la version ACTUELLE mais pas dans celle de la date des faits :
            // le modèle a cité la rédaction d'aujourd'hui alors que l'utilisateur raisonne à une date antérieure.
            if let (Some(df), Some(c)) = (date_fr.as_deref(), cmp_id.as_deref()) {
                if c != t.id && article_verdict != Some(QuoteMatch::Exact) {
                    if let Some(Ok(cur)) = article_hay.get(&t.id) {
                        if compare_quote(strip_article_label(&q.text), cur) == QuoteMatch::Exact
                            && (!t.from_question || question_quotes == 1)
                        {
                            temporal_verdict = Some((
                                Statut::Incoherente,
                                format!(
                                    "Ce passage figure dans la version actuelle de l'article ({}) mais pas dans sa rédaction applicable au {df} : la citation ne reflète pas le droit en vigueur à la date des faits.",
                                    t.reference
                                ),
                            ));
                        }
                    }
                }
            }
        }
        if let Some((statut, detail)) = temporal_verdict.take() {
            let mut v = blank(Kind::Citation, label.clone(), excerpt(&q.text, 10));
            v.code = code.clone();
            v.statut = statut;
            v.detail = Some(detail);
            v.occurrences.push(Occurrence {
                debut: utf16_offset(inp.text, q.close_start),
                fin: utf16_offset(inp.text, q.close_end),
                texte: inp.text[q.close_start..q.close_end].to_string(),
            });
            report.verifications.push(v);
            continue;
        }
        if version_unknown {
            // Rédaction à la date illisible : on ne compare pas au texte actuel (ce serait refaire l'erreur).
            if let Some(df) = date_fr.as_deref() {
                let mut v = blank(Kind::Citation, label.clone(), excerpt(&q.text, 10));
                v.code = code.clone();
                v.statut = Statut::NonVerifiable;
                v.detail = Some(format!(
                    "La rédaction de l'article applicable au {df} n'a pas pu être lue : passage non comparé."
                ));
                v.occurrences.push(Occurrence {
                    debut: utf16_offset(inp.text, q.close_start),
                    fin: utf16_offset(inp.text, q.close_end),
                    texte: inp.text[q.close_start..q.close_end].to_string(),
                });
                report.verifications.push(v);
                continue;
            }
        }
        match article_verdict {
            Some(QuoteMatch::Exact) => {
                verdict = Some((
                    Statut::Verifiee,
                    format!("Passage retrouvé tel quel dans le texte de l'article ({desc})."),
                    Vec::new(),
                ));
            }
            Some(QuoteMatch::Approx) | Some(QuoteMatch::Absent) => {
                // Le passage peut figurer tel quel dans un document fourni (autre version) : on le vérifie d'abord.
                if let Some(h) = &sources_hay {
                    if q.words >= MIN_WORDS_SOURCES
                        && compare_quote(strip_article_label(&q.text), h) == QuoteMatch::Exact
                    {
                        verdict = Some((
                            Statut::Verifiee,
                            "Passage retrouvé tel quel dans les documents fournis au modèle (il diffère du texte de l'article sur Légifrance)."
                                .into(),
                            Vec::new(),
                        ));
                    }
                }
                if verdict.is_none() {
                    if article_verdict == Some(QuoteMatch::Approx) {
                        let infos = cmp_id
                            .as_ref()
                            .and_then(|c| article_raw.get(c))
                            .map(|raw| diff_infos(strip_article_label(&q.text), raw))
                            .unwrap_or_default();
                        verdict = Some((
                            Statut::Approchante,
                            format!("Passage proche du texte de l'article ({desc}) mais non identique : la formulation a été modifiée."),
                            infos,
                        ));
                    } else if !from_question || question_quotes == 1 {
                        let tail = if from_question {
                            "citation inexacte, reformulée, ou passage qui ne provient pas de cet article"
                        } else {
                            "citation inexacte, reformulée ou rattachée au mauvais article"
                        };
                        verdict = Some((
                            Statut::Incoherente,
                            format!("Ce passage ne figure pas dans le texte de l'article ({desc}) : {tail}."),
                            Vec::new(),
                        ));
                    }
                }
            }
            _ => {}
        }

        // 2. Sinon : documents fournis au modèle (verdict positif seulement).
        if verdict.is_none() && q.words >= MIN_WORDS_SOURCES {
            if let Some(h) = &sources_hay {
                match compare_quote(strip_article_label(&q.text), h) {
                    QuoteMatch::Exact => {
                        verdict = Some((
                            Statut::Verifiee,
                            "Passage retrouvé tel quel dans les documents fournis au modèle.".into(),
                            Vec::new(),
                        ))
                    }
                    QuoteMatch::Approx => {
                        verdict = Some((
                            Statut::Approchante,
                            "Passage proche d'un texte des documents fournis au modèle mais non identique : la formulation a été modifiée."
                                .into(),
                            Vec::new(),
                        ))
                    }
                    _ => {}
                }
            }
        }

        let Some((statut, detail, infos)) = verdict else {
            continue;
        };
        let mut v = blank(Kind::Citation, label, excerpt(&q.text, 10));
        v.code = code;
        v.statut = statut;
        v.detail = Some(detail);
        v.infos = infos;
        v.occurrences.push(Occurrence {
            debut: utf16_offset(inp.text, q.close_start),
            fin: utf16_offset(inp.text, q.close_end),
            texte: inp.text[q.close_start..q.close_end].to_string(),
        });
        report.verifications.push(v);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::juridique::verify::{self, TextRef};

    // ── Dates, chambres ──────────────────────────────────────────────────────

    #[test]
    fn dates_in_french_are_parsed_in_all_usual_forms() {
        assert_eq!(first_date("le 12 mars 2024,").unwrap().2, "2024-03-12");
        assert_eq!(first_date("1er août 2019").unwrap().2, "2019-08-01");
        assert_eq!(first_date("14 déc. 2022").unwrap().2, "2022-12-14");
        assert_eq!(first_date("3 janv. 2021").unwrap().2, "2021-01-03");
        assert_eq!(first_date("5 juill. 2020").unwrap().2, "2020-07-05");
        assert!(first_date("12 marsupiaux 2024").is_none());
        assert!(first_date("sans date").is_none());
        assert_eq!(fr_date("2024-03-12"), "12/03/2024");
        assert_eq!(fr_date("2024-03-12T00:00:00"), "12/03/2024");
    }

    #[test]
    fn chambers_are_canonicalized_from_citations_and_from_judilibre_labels() {
        for (s, want) in [
            ("Cass. soc.", "soc"),
            ("Chambre sociale", "soc"),
            ("soc", "soc"),
            ("com.", "comm"),
            ("Chambre commerciale, financière et économique", "comm"),
            ("crim.", "cr"),
            ("Chambre criminelle", "cr"),
            ("cr", "cr"),
            ("Civ. 1re", "civ1"),
            ("1re civ.", "civ1"),
            ("Première chambre civile", "civ1"),
            ("civ1", "civ1"),
            ("Civ. 2e", "civ2"),
            ("Deuxième chambre civile", "civ2"),
            ("Civ. 3e", "civ3"),
            ("civ3", "civ3"),
            ("Assemblée plénière", "pl"),
            ("ass. plén.", "pl"),
            ("Chambre mixte", "mi"),
        ] {
            assert_eq!(canon_chamber(s), Some(want), "{s}");
        }
        assert_eq!(canon_chamber("Cour de cassation"), None);
        assert_eq!(canon_chamber("civ."), None, "chambre civile non précisée");
        assert_eq!(canon_chamber(""), None);
    }

    // ── Décisions ────────────────────────────────────────────────────────────

    fn dec(text: &str) -> Vec<DecisionRef> {
        extract_decision_refs(text)
    }

    #[test]
    fn decisions_are_extracted_with_chamber_and_date() {
        let t = "Voir Cass. soc., 12 mars 2024, n° 22-12.345, Bull.";
        let d = dec(t);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].key, "2212345");
        assert_eq!(d[0].display, "22-12.345");
        assert_eq!(d[0].chambre, Some("soc"));
        assert_eq!(d[0].date.as_deref(), Some("2024-03-12"));
        assert_eq!(d[0].raw, "Cass. soc., 12 mars 2024, n° 22-12.345");
        assert_eq!(&t[d[0].start..d[0].end], d[0].raw);
    }

    #[test]
    fn decisions_are_recognized_in_the_usual_citation_styles() {
        for (t, key, chambre) in [
            (
                "Cass. 1re civ., 5 juin 2024, pourvoi n° 23-10.123",
                "2310123",
                Some("civ1"),
            ),
            (
                "Civ. 3e, 7 mars 2024, n° 22-23.456",
                "2223456",
                Some("civ3"),
            ),
            (
                "Cass. com., 3 mai 2023, n° 21-15789",
                "2115789",
                Some("comm"),
            ),
            (
                "Cass. crim., 14 déc. 2022, n° 21-86.123",
                "2186123",
                Some("cr"),
            ),
            (
                "Cass. ass. plén., 17 mai 2023, n° 20-20.559",
                "2020559",
                Some("pl"),
            ),
            (
                "Cour de cassation, chambre sociale, 12 mars 2024, pourvoi n° 22-12.345",
                "2212345",
                Some("soc"),
            ),
            ("l'arrêt (pourvoi n° 22-12.345)", "2212345", None),
            ("n° de pourvoi : 22-12.345", "2212345", None),
            ("pourvoi 22-12.345", "2212345", None),
            (
                "Cass. soc., 5 janv. 2022, no. 20-10.001",
                "2010001",
                Some("soc"),
            ),
        ] {
            let d = dec(t);
            assert_eq!(d.len(), 1, "{t}");
            assert_eq!(d[0].key, key, "{t}");
            assert_eq!(d[0].chambre, chambre, "{t}");
        }
    }

    #[test]
    fn numbers_of_other_series_are_not_taken_for_pourvois() {
        for t in [
            "le décret n° 2020-1455 du 27 novembre 2020",
            "la loi n° 78-17 du 6 janvier 1978",
            "la loi n° 2016-1088 du 8 août 2016",
            "dossier n° 22-12.345 (sans contexte de décision)",
            "ordonnance n° 2017-1387 du 22 septembre 2017",
            "article L. 1234-5 du Code du travail",
            "le 12 mars 2024, n° 22-12.345", // pas de « pourvoi », pas de juridiction
        ] {
            assert!(dec(t).is_empty(), "{t}");
        }
    }

    #[test]
    fn several_decisions_in_one_sentence_keep_their_own_context() {
        let t = "Cass. soc., 12 mars 2024, n° 22-12.345 ; Cass. com., 3 mai 2023, n° 21-15.789.";
        let d = dec(t);
        assert_eq!(d.len(), 2);
        assert_eq!(
            (d[0].chambre, d[0].date.as_deref()),
            (Some("soc"), Some("2024-03-12"))
        );
        assert_eq!(
            (d[1].chambre, d[1].date.as_deref()),
            (Some("comm"), Some("2023-05-03"))
        );
        assert!(d[0].end <= d[1].start);
    }

    #[test]
    fn decisions_in_code_blocks_links_and_urls_are_ignored() {
        assert!(dec("```\nCass. soc., 12 mars 2024, n° 22-12.345\n```").is_empty());
        assert!(dec("`pourvoi n° 22-12.345`").is_empty());
        assert!(dec("[pourvoi n° 22-12.345](https://example.org)").is_empty());
    }

    #[test]
    fn a_decision_overlapping_a_text_ref_is_detected() {
        let t = "Cass. soc., décision n° 22-12.345.";
        let d = dec(t);
        let texts = verify::extract_text_refs(t);
        assert!(texts.iter().any(|x| overlaps_decision(x.start, x.end, &d)));
    }

    #[test]
    fn pick_decision_matches_by_number_in_any_written_form() {
        let results = vec![
            json!({"id": "A", "number": "20-99.999", "decision_date": "2020-01-01"}),
            json!({"id": "B", "number": "22-12.345", "numbers": ["22-12.345", "22-12.346"],
                   "chamber": "soc", "decision_date": "2024-03-12", "solution": "cassation",
                   "publication": ["b", "r"], "jurisdiction": "cc"}),
        ];
        let info = pick_decision(&results, "2212345").unwrap();
        assert_eq!(info.id, "B");
        assert_eq!(info.chamber.as_deref(), Some("soc"));
        assert_eq!(info.date.as_deref(), Some("2024-03-12"));
        assert_eq!(info.publication, vec!["b", "r"]);
        // Un numéro porté par `numbers` seulement (décisions jointes) est retrouvé aussi.
        assert_eq!(pick_decision(&results, "2212346").unwrap().id, "B");
        assert!(pick_decision(&results, "2200000").is_none());
        assert!(pick_decision(&[], "2212345").is_none());
        // Pas d'identifiant : inexploitable.
        assert!(pick_decision(&[json!({"number": "22-12.345"})], "2212345").is_none());
    }

    // ── IDCC ─────────────────────────────────────────────────────────────────

    #[test]
    fn idcc_are_extracted_and_padded() {
        let t = "La convention collective (IDCC 1486) et l'IDCC n° 16, ainsi que IDCC : 2216.";
        let v = extract_idcc_refs(t);
        let ids: Vec<&str> = v.iter().map(|i| i.idcc.as_str()).collect();
        assert_eq!(
            ids,
            vec!["1486", "2216"],
            "« IDCC n° 16 » (2 chiffres) n'est pas un IDCC valide"
        );
        let v = extract_idcc_refs("IDCC 016 et IDCC 0016");
        assert_eq!(
            v.iter().map(|i| i.idcc.clone()).collect::<Vec<_>>(),
            vec!["0016", "0016"]
        );
        assert!(extract_idcc_refs("IDCC").is_empty());
        assert!(extract_idcc_refs("`IDCC 1486`").is_empty());
    }

    #[test]
    fn the_pourvoi_field_is_read_from_the_taxonomy_in_every_documented_shape() {
        // Table clé → intitulé.
        let map = json!({"id": "field", "result": {"expose": "Exposé du litige", "number": "Numéro de pourvoi"}});
        assert_eq!(number_field_from_taxonomy(&map).as_deref(), Some("number"));
        // Liste d'éléments {key, value}, clé différente : c'est l'intitulé qui compte.
        let list = json!({"id": "field", "result": [
            {"key": "sommaire", "value": "Sommaire"}, {"key": "numero", "value": "Numéro de POURVOI"}]});
        assert_eq!(number_field_from_taxonomy(&list).as_deref(), Some("numero"));
        // Repli sur la clé `number` si l'intitulé ne dit rien.
        let by_key = json!({"result": {"number": "Numéro"}});
        assert_eq!(
            number_field_from_taxonomy(&by_key).as_deref(),
            Some("number")
        );
        // Aucun champ de ce genre : pas de ciblage.
        assert_eq!(
            number_field_from_taxonomy(&json!({"result": {"expose": "Exposé"}})),
            None
        );
        assert_eq!(number_field_from_taxonomy(&json!({})), None);
    }

    #[test]
    fn parse_idcc_requires_an_identifier() {
        let ok = parse_idcc(
            &json!({"id": "KALICONT000005635534", "titre": "Convention X", "etat": "VIGUEUR"}),
        )
        .unwrap();
        assert_eq!(
            ok,
            IdccLookup::Trouve {
                id: "KALICONT000005635534".into(),
                titre: Some("Convention X".into()),
                etat: Some("VIGUEUR".into())
            }
        );
        assert!(parse_idcc(&json!({"kaliCont": {"id": "KALICONT1"}})).is_ok());
        assert!(
            parse_idcc(&json!({})).is_err(),
            "forme inattendue = pas de verdict"
        );
    }

    // ── Textes : appariement ─────────────────────────────────────────────────

    #[test]
    fn text_urls_use_the_base_identifier_without_the_version_suffix() {
        assert_eq!(
            legifrance_text_url("LEGITEXT000042477679_30-10-2020").as_deref(),
            Some("https://www.legifrance.gouv.fr/loda/id/LEGITEXT000042477679")
        );
        assert_eq!(
            legifrance_text_url("JORFTEXT000042587977").as_deref(),
            Some("https://www.legifrance.gouv.fr/jorf/id/JORFTEXT000042587977")
        );
        assert_eq!(legifrance_text_url("KALITEXT000005678"), None);
    }

    #[test]
    fn contains_numero_requires_a_whole_number() {
        assert!(contains_numero(
            "loi n 2016-1088 du 8 aout 2016",
            "2016-1088"
        ));
        assert!(!contains_numero(
            "loi n 2016-10880 du 8 aout 2016",
            "2016-1088"
        ));
        assert!(!contains_numero("loi n 2078-17 du", "78-17"));
        assert!(contains_numero("loi n 78-17 du 6 janvier 1978", "78-17"));
        assert!(!contains_numero("loi n 78-170 du", "78-17"));
    }

    #[test]
    fn pick_texte_checks_nature_and_number() {
        let resp = json!({"results": [
            {"id": "JORFTEXT1", "title": "Arrêté du 1 mars 2021 modifiant le décret n° 2020-1455", "etat": "VIGUEUR"},
            {"titles": [{"id": "LEGITEXT000042", "title": "<mark>Décret</mark> n° 2020-1455 du 27 novembre 2020 relatif à X"}],
             "etat": "ABROGE", "dateSignature": "2020-11-27T00:00:00.000+0000"}
        ]});
        match pick_texte(&resp, "decret", "2020-1455") {
            TexteLookup::Trouve {
                id,
                titre,
                etat,
                date,
            } => {
                assert_eq!(id, "LEGITEXT000042");
                assert!(titre.starts_with("Décret n° 2020-1455"), "{titre}");
                assert_eq!(etat.as_deref(), Some("ABROGE"));
                assert_eq!(date.as_deref(), Some("2020-11-27"));
            }
            other => panic!("{other:?}"),
        }
        // L'arrêté qui cite seulement le numéro ne vaut pas texte de cette nature.
        let only_arrete = json!({"results": [{"id": "X", "title": "Arrêté du 1 mars 2021 modifiant le décret n° 2020-1455"}]});
        assert_eq!(
            pick_texte(&only_arrete, "decret", "2020-1455"),
            TexteLookup::Introuvable
        );
        assert_eq!(
            pick_texte(&json!({}), "loi", "2016-1088"),
            TexteLookup::Introuvable
        );
        assert_eq!(
            pick_texte(&resp, "loi", "2020-1455"),
            TexteLookup::Introuvable
        );
    }

    // ── Citations textuelles ─────────────────────────────────────────────────

    const ART_1240: &str = "Tout fait quelconque de l'homme, qui cause à autrui un dommage, oblige celui par la faute duquel il est arrivé à le réparer.";

    #[test]
    fn quotes_are_extracted_with_their_closing_mark() {
        let t = "L'article dispose que « Tout fait quelconque de l'homme, qui cause à autrui un dommage » puis “ceci est une autre citation assez longue pour compter”.";
        let q = extract_quotes(t);
        assert_eq!(q.len(), 2);
        assert!(q[0].text.starts_with("Tout fait quelconque"));
        assert_eq!(&t[q[0].close_start..q[0].close_end], "»");
        assert_eq!(&t[q[1].close_start..q[1].close_end], "”");
        assert!(q[0].words >= 9);
        assert!(extract_quotes(
            "```\n« citation dans du code assez longue pour compter ici »\n```"
        )
        .is_empty());
        assert!(
            extract_quotes("« un\n\nautre »").is_empty(),
            "guillemets sur deux paragraphes"
        );
    }

    #[test]
    fn exact_quotes_survive_typography_case_and_ellipses() {
        let hay = haystack(ART_1240);
        // Casse, apostrophe typographique, ponctuation.
        assert_eq!(
            compare_quote(
                "tout fait quelconque de l’homme qui cause à autrui un dommage",
                &hay
            ),
            QuoteMatch::Exact
        );
        // Ellipse et insertion entre crochets.
        assert_eq!(
            compare_quote("Tout fait quelconque de l'homme […] oblige celui par la faute duquel il est arrivé [dommage] à le réparer", &hay),
            QuoteMatch::Exact
        );
        assert_eq!(
            compare_quote("Tout fait quelconque de l'homme ... à le réparer", &hay),
            QuoteMatch::Exact
        );
    }

    #[test]
    fn altered_quotes_are_approximate_or_absent() {
        let hay = haystack(ART_1240);
        // Un mot changé : proche.
        assert_eq!(
            compare_quote("Tout fait quelconque de l'homme, qui cause à autrui un préjudice, oblige celui par la faute duquel il est arrivé à le réparer", &hay),
            QuoteMatch::Approx
        );
        // Paraphrase : rien de comparable.
        assert_eq!(
            compare_quote("Celui qui provoque un dommage par sa faute doit en assumer les conséquences financières", &hay),
            QuoteMatch::Absent
        );
        // Fragments trop courts pour être comparés.
        assert_eq!(
            compare_quote("a […] b […] c", &hay),
            QuoteMatch::Unverifiable
        );
    }

    fn art(code: &'static str, digits: &str, start: usize, end: usize) -> ArticleRef {
        ArticleRef {
            start,
            end,
            raw: String::new(),
            letter: None,
            digits: digits.into(),
            suffix: None,
            code: Some(code),
        }
    }

    #[test]
    fn a_quote_is_attached_only_to_an_unambiguous_nearby_article() {
        let t = "L'article 1240 du Code civil dispose que « Tout fait quelconque de l'homme, qui cause à autrui un dommage ».";
        let refs = verify::extract_article_refs(t);
        let q = extract_quotes(t);
        assert_eq!(attached_article(&q[0], &refs, t), Some(0));

        // Article après la citation.
        let t2 = "« Tout fait quelconque de l'homme, qui cause à autrui un dommage » (art. 1240 C. civ.)";
        let refs2 = verify::extract_article_refs(t2);
        let q2 = extract_quotes(t2);
        assert_eq!(attached_article(&q2[0], &refs2, t2), Some(0));

        // Deux articles différents avant la citation : ambigu, pas de rattachement.
        let t3 = "Les articles 1240 et 1241 du Code civil disposent que « Tout fait quelconque de l'homme, qui cause à autrui un dommage ».";
        let refs3 = verify::extract_article_refs(t3);
        let q3 = extract_quotes(t3);
        assert!(refs3.len() >= 2);
        assert_eq!(attached_article(&q3[0], &refs3, t3), None);

        // Autre paragraphe : pas de rattachement.
        let t4 = "L'article 1240 du Code civil.\n\nIl dispose que « Tout fait quelconque de l'homme, qui cause à autrui un dommage ».";
        let refs4 = verify::extract_article_refs(t4);
        let q4 = extract_quotes(t4);
        assert_eq!(attached_article(&q4[0], &refs4, t4), None);

        // Aucun article.
        assert_eq!(attached_article(&q[0], &[], t), None);
        let _ = art("Code civil", "1", 0, 1);
    }

    #[test]
    fn html_of_an_article_becomes_plain_text() {
        let h = "<p>Tout fait&nbsp;quelconque<br/>de l&#39;homme &amp; <b>autre</b></p>";
        assert_eq!(
            html_to_text(h)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            "Tout fait quelconque de l'homme & autre"
        );
        let v = json!({"article": {"texte": "<p>Corps de l'article</p>"}});
        assert_eq!(article_text_of(&v).unwrap().trim(), "Corps de l'article");
        assert!(article_text_of(&json!({"article": {}})).is_none());
        assert_eq!(
            article_text_of(&json!({"article": {"texteHtml": "<p>Via HTML</p>"}}))
                .unwrap()
                .trim(),
            "Via HTML"
        );
    }

    // ── Rapport avec un faux ExtLookup ───────────────────────────────────────

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    #[derive(Default)]
    struct FakeExt {
        decisions: Mutex<HashMap<String, Result<DecisionLookup, String>>>,
        textes: Mutex<HashMap<String, Result<TexteLookup, String>>>,
        idccs: Mutex<HashMap<String, Result<IdccLookup, String>>>,
        articles: Mutex<HashMap<String, Result<String, String>>>,
        /// id de l'article courant → version applicable à la date (`Ok(None)` : aucune).
        versions: Mutex<HashMap<String, Result<Option<String>, String>>>,
        calls: AtomicUsize,
    }

    impl ExtLookup for FakeExt {
        fn decision_ref<'a>(
            &'a self,
            q: &'a DecisionQuery,
        ) -> BoxFuture<'a, Result<DecisionLookup, String>> {
            if let Some(key) = q.pourvoi_key() {
                return self.decision(key, &q.display);
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            let id = q
                .ecli
                .clone()
                .or_else(|| q.number.clone())
                .unwrap_or_default();
            let r = self
                .decisions
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .unwrap_or(Ok(DecisionLookup::Introuvable));
            Box::pin(async move { r })
        }
        fn decision<'a>(
            &'a self,
            key: &'a str,
            _: &'a str,
        ) -> BoxFuture<'a, Result<DecisionLookup, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let r = self
                .decisions
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .unwrap_or(Ok(DecisionLookup::Introuvable));
            Box::pin(async move { r })
        }
        fn texte<'a>(
            &'a self,
            nature: &'a str,
            numero: &'a str,
        ) -> BoxFuture<'a, Result<TexteLookup, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let r = self
                .textes
                .lock()
                .unwrap()
                .get(&format!("{nature}:{numero}"))
                .cloned()
                .unwrap_or(Ok(TexteLookup::Introuvable));
            Box::pin(async move { r })
        }
        fn idcc<'a>(&'a self, idcc: &'a str) -> BoxFuture<'a, Result<IdccLookup, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let r = self
                .idccs
                .lock()
                .unwrap()
                .get(idcc)
                .cloned()
                .unwrap_or(Ok(IdccLookup::Introuvable));
            Box::pin(async move { r })
        }
        fn article_text<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<String, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let r = self
                .articles
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .unwrap_or(Err("absent".into()));
            Box::pin(async move { r })
        }
        fn article_version_at<'a>(
            &'a self,
            id: &'a str,
            _date: chrono::NaiveDate,
        ) -> BoxFuture<'a, Result<Option<String>, String>> {
            let r = self
                .versions
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .unwrap_or(Err("absent".into()));
            Box::pin(async move { r })
        }
    }

    fn info(chamber: &str, date: &str) -> DecisionInfo {
        DecisionInfo {
            id: "644f5a3e1bce0b5c0e5e7a1f".into(),
            number: "22-12.345".into(),
            chamber: Some(chamber.into()),
            jurisdiction: Some("Cour de cassation".into()),
            formation: None,
            date: Some(date.into()),
            solution: Some("Cassation".into()),
            publication: vec!["Publié au Bulletin".into()],
            ..Default::default()
        }
    }

    /// Rapport de base (articles + textes) puis extension, comme le fait la route.
    async fn run(ext: &FakeExt, text: &str, sources: &[String], ancrage: bool) -> Report {
        run_with_context(ext, text, sources, ancrage, None).await
    }

    async fn run_with_context(
        ext: &FakeExt,
        text: &str,
        sources: &[String],
        ancrage: bool,
        context_article: Option<&ContextArticle>,
    ) -> Report {
        let refs = verify::extract_article_refs(text);
        let mut texts: Vec<TextRef> = verify::extract_text_refs(text);
        let er = extract_ext_refs(text);
        texts.retain(|t| !overlaps_decision(t.start, t.end, &er.decisions));
        let grounding =
            ancrage.then(|| GroundingIndex::from_texts(sources.iter().map(String::as_str)));
        let art = FakeArticles;
        let mut report =
            verify::build_report(&art, text, &refs, &texts, grounding.as_ref(), "2026-10-23").await;
        extend_report(
            ext,
            ExtInput {
                text,
                article_refs: &refs,
                ext: &er,
                grounding: grounding.as_ref(),
                sources: if ancrage { sources } else { &[] },
                context_article,
            },
            &mut report,
        )
        .await;
        report
    }

    /// Comme `run`, avec une date des faits (droit applicable à cette date).
    async fn run_at(ext: &FakeExt, text: &str, date: &str) -> Report {
        let refs = verify::extract_article_refs(text);
        let texts: Vec<TextRef> = Vec::new();
        let er = extract_ext_refs(text);
        let art = FakeArticles;
        let mut report = verify::build_report(&art, text, &refs, &texts, None, "2026-10-27").await;
        extend_report_at(
            ext,
            ExtInput {
                text,
                article_refs: &refs,
                ext: &er,
                grounding: None,
                sources: &[],
                context_article: None,
            },
            &mut report,
            Some(chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap()),
        )
        .await;
        report
    }

    /// Légifrance « articles » factice : tout article demandé existe et est en vigueur.
    struct FakeArticles;
    impl verify::ArticleLookup for FakeArticles {
        fn lookup<'a>(
            &'a self,
            code: &'a str,
            numero: &'a str,
        ) -> BoxFuture<'a, Result<verify::Lookup, String>> {
            let id = format!("LEGIARTI_{}_{}", code.replace(' ', "_"), numero);
            Box::pin(async move { Ok(verify::Lookup::EnVigueur { id }) })
        }
    }

    fn only(report: &Report, kind: Kind) -> &Verification {
        let v: Vec<&Verification> = report
            .verifications
            .iter()
            .filter(|v| v.kind == kind)
            .collect();
        assert_eq!(v.len(), 1, "{kind:?} : {:#?}", report.verifications);
        v[0]
    }

    #[tokio::test]
    async fn decision_found_and_consistent_is_verified_with_its_identity() {
        let ext = FakeExt::default();
        ext.decisions.lock().unwrap().insert(
            "2212345".into(),
            Ok(DecisionLookup::Trouvee(info(
                "Chambre sociale",
                "2024-03-12",
            ))),
        );
        let r = run(&ext, "Cass. soc., 12 mars 2024, n° 22-12.345.", &[], false).await;
        let v = only(&r, Kind::Decision);
        assert_eq!(v.statut, Statut::Verifiee);
        assert_eq!(v.existe, Some(true));
        assert!(v
            .url
            .as_deref()
            .unwrap()
            .ends_with("/644f5a3e1bce0b5c0e5e7a1f"));
        assert!(v.infos.iter().any(|i| i.contains("Chambre sociale")));
        assert!(v.infos.iter().any(|i| i.contains("12/03/2024")));
        assert!(v.infos.iter().any(|i| i.contains("Solution : Cassation")));
        assert_eq!(v.occurrences.len(), 1);
        assert_eq!(v.ancrage, None);
    }

    #[tokio::test]
    async fn decision_with_the_wrong_date_or_chamber_is_incoherent() {
        let ext = FakeExt::default();
        ext.decisions.lock().unwrap().insert(
            "2212345".into(),
            Ok(DecisionLookup::Trouvee(info(
                "Chambre commerciale",
                "2024-03-13",
            ))),
        );
        let r = run(&ext, "Cass. soc., 12 mars 2024, n° 22-12.345.", &[], false).await;
        let v = only(&r, Kind::Decision);
        assert_eq!(v.statut, Statut::Incoherente);
        let d = v.detail.as_deref().unwrap();
        assert!(d.contains("12/03/2024") && d.contains("13/03/2024"), "{d}");
        assert!(d.contains("chambre"), "{d}");
    }

    #[tokio::test]
    async fn an_unknown_chamber_label_never_causes_a_false_incoherence() {
        let ext = FakeExt::default();
        let mut i = info("Formation exotique", "2024-03-12");
        i.chamber = Some("Formation exotique".into());
        ext.decisions
            .lock()
            .unwrap()
            .insert("2212345".into(), Ok(DecisionLookup::Trouvee(i)));
        let r = run(&ext, "Cass. soc., 12 mars 2024, n° 22-12.345.", &[], false).await;
        assert_eq!(only(&r, Kind::Decision).statut, Statut::Verifiee);
    }

    #[tokio::test]
    async fn decision_not_found_is_soft_and_errors_are_not_verdicts() {
        let ext = FakeExt::default();
        let r = run(&ext, "Cass. soc., 12 mars 2024, n° 22-12.345.", &[], false).await;
        let v = only(&r, Kind::Decision);
        assert_eq!(v.statut, Statut::NonRetrouvee);
        assert_eq!(v.existe, None, "pas de preuve d'inexistence");
        assert!(v.detail.as_deref().unwrap().contains("pas exhaustif"));

        ext.decisions
            .lock()
            .unwrap()
            .insert("2212345".into(), Err("HTTP 500".into()));
        let r = run(&ext, "Cass. soc., 12 mars 2024, n° 22-12.345.", &[], false).await;
        let v = only(&r, Kind::Decision);
        assert_eq!(v.statut, Statut::Erreur);
        assert_eq!(
            v.detail.as_deref(),
            Some("Judilibre n'a pas pu être interrogé")
        );
        assert!(
            !v.detail.as_deref().unwrap().contains("500"),
            "pas de détail technique au client"
        );
    }

    #[tokio::test]
    async fn the_same_pourvoi_cited_twice_costs_one_lookup() {
        let ext = FakeExt::default();
        ext.decisions.lock().unwrap().insert(
            "2212345".into(),
            Ok(DecisionLookup::Trouvee(info(
                "Chambre sociale",
                "2024-03-12",
            ))),
        );
        let t = "Cass. soc., 12 mars 2024, n° 22-12.345. Plus loin : pourvoi n° 22-12.345.";
        let r = run(&ext, t, &[], false).await;
        assert_eq!(only(&r, Kind::Decision).occurrences.len(), 2);
        assert_eq!(ext.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn decision_anchoring_flags_a_pourvoi_the_model_was_never_given() {
        let ext = FakeExt::default();
        ext.decisions.lock().unwrap().insert(
            "2212345".into(),
            Ok(DecisionLookup::Trouvee(info(
                "Chambre sociale",
                "2024-03-12",
            ))),
        );
        let t = "Cass. soc., 12 mars 2024, n° 22-12.345.";
        let r = run(&ext, t, &["Une autre pièce sans référence".into()], true).await;
        assert_eq!(only(&r, Kind::Decision).ancrage, Some(Ancrage::HorsSources));
        let r = run(
            &ext,
            t,
            &["Voir pourvoi n° 22-12.345 dans le dossier".into()],
            true,
        )
        .await;
        assert_eq!(only(&r, Kind::Decision).ancrage, Some(Ancrage::DansSources));
    }

    #[tokio::test]
    async fn a_decision_is_not_also_reported_as_a_text() {
        let ext = FakeExt::default();
        let t = "Cass. soc., décision n° 22-12.345.";
        let r = run(&ext, t, &[], false).await;
        assert!(
            r.verifications.iter().all(|v| v.kind != Kind::Texte),
            "{:#?}",
            r.verifications
        );
    }

    #[tokio::test]
    async fn texts_cited_by_number_get_existence_status_and_date_check() {
        let ext = FakeExt::default();
        ext.textes.lock().unwrap().insert(
            "decret:2020-1455".into(),
            Ok(TexteLookup::Trouve {
                id: "JORFTEXT000042587977".into(),
                titre: "Décret n° 2020-1455 du 27 novembre 2020 relatif à X".into(),
                etat: Some("VIGUEUR".into()),
                date: None,
            }),
        );
        ext.textes.lock().unwrap().insert(
            "loi:2016-1088".into(),
            Ok(TexteLookup::Trouve {
                id: "JORFTEXT000032983213".into(),
                titre: "Loi n° 2016-1088 du 8 août 2016 relative au travail".into(),
                etat: Some("VIGUEUR".into()),
                // Date de signature de la recherche : prioritaire sur celle du titre.
                date: Some("2016-08-08".into()),
            }),
        );
        ext.textes.lock().unwrap().insert(
            "ordonnance:2017-1387".into(),
            Ok(TexteLookup::Trouve {
                id: "LEGITEXT000035607898".into(),
                titre: "Ordonnance n° 2017-1387 du 22 septembre 2017".into(),
                etat: Some("ABROGE".into()),
                date: None,
            }),
        );
        let t = "Le décret n° 2020-1455 du 27 novembre 2020, la loi n° 2016-1088 du 9 août 2016 \
                 et l'ordonnance n° 2017-1387 du 22 septembre 2017, ainsi que la loi n° 99-999.";
        let r = run(&ext, t, &[], false).await;
        let by = |needle: &str| {
            r.verifications
                .iter()
                .find(|v| v.reference.contains(needle))
                .unwrap_or_else(|| panic!("{needle}: {:#?}", r.verifications))
        };
        let d = by("2020-1455");
        assert_eq!(d.statut, Statut::EnVigueur);
        assert!(d
            .url
            .as_deref()
            .unwrap()
            .contains("/jorf/id/JORFTEXT000042587977"));
        assert!(d.infos[0].starts_with("Décret n° 2020-1455"));
        assert_eq!(d.detail, None);

        let l = by("2016-1088");
        assert_eq!(l.statut, Statut::Incoherente, "date citée 9 août ≠ 8 août");
        assert!(l.detail.as_deref().unwrap().contains("09/08/2016"));
        assert!(l.detail.as_deref().unwrap().contains("08/08/2016"));

        let o = by("2017-1387");
        assert_eq!(o.statut, Statut::Abroge);
        assert_eq!(o.etat_libelle.as_deref(), Some("abrogé"));
        assert!(o.url.as_deref().unwrap().contains("/loda/id/LEGITEXT"));

        let missing = by("99-999");
        assert_eq!(missing.statut, Statut::NonRetrouvee);
        assert_eq!(missing.existe, None);
    }

    #[tokio::test]
    async fn texts_without_a_number_or_of_other_natures_are_left_alone() {
        let ext = FakeExt::default();
        let t = "L'arrêté du 8 décembre 2021 et la circulaire n° 2021-12 du 3 mars 2021.";
        let r = run(&ext, t, &[], false).await;
        assert_eq!(ext.calls.load(Ordering::SeqCst), 0);
        assert!(r
            .verifications
            .iter()
            .all(|v| v.statut == Statut::NonVerifiable));
    }

    #[tokio::test]
    async fn collective_agreements_are_checked_by_idcc() {
        let ext = FakeExt::default();
        ext.idccs.lock().unwrap().insert(
            "1486".into(),
            Ok(IdccLookup::Trouve {
                id: "KALICONT000005635534".into(),
                titre: Some(
                    "Convention collective nationale des bureaux d'études techniques".into(),
                ),
                // Un conteneur de convention n'a pas d'état juridique (spec Légifrance).
                etat: None,
            }),
        );
        let r = run(
            &ext,
            "Voir la convention collective (IDCC 1486) et l'IDCC 9999.",
            &[],
            false,
        )
        .await;
        let ok = r.verifications.iter().find(|v| v.numero == "1486").unwrap();
        assert_eq!(ok.kind, Kind::Convention);
        assert_eq!(ok.statut, Statut::Verifiee);
        assert_eq!(ok.etat_libelle, None);
        assert!(ok.url.as_deref().unwrap().ends_with("KALICONT000005635534"));
        assert!(ok.infos[0].contains("bureaux d'études"));
        let ko = r.verifications.iter().find(|v| v.numero == "9999").unwrap();
        assert_eq!(ko.statut, Statut::NonRetrouvee);
    }

    #[tokio::test]
    async fn a_quote_matching_its_article_is_verified_and_a_misquote_is_flagged() {
        let ext = FakeExt::default();
        ext.articles
            .lock()
            .unwrap()
            .insert("LEGIARTI_Code_civil_1240".into(), Ok(ART_1240.into()));
        let exact = "L'article 1240 du Code civil dispose que « Tout fait quelconque de l'homme, qui cause à autrui un dommage, oblige celui par la faute duquel il est arrivé à le réparer ».";
        let r = run(&ext, exact, &[], false).await;
        let v = only(&r, Kind::Citation);
        assert_eq!(v.statut, Statut::Verifiee);
        assert_eq!(v.occurrences[0].texte, "»");
        assert!(v.detail.as_deref().unwrap().contains("tel quel"));
        assert!(v.reference.contains("Code civil"), "{}", v.reference);

        let near = "L'article 1240 du Code civil dispose que « Tout fait quelconque de l'homme, qui cause à autrui un préjudice, oblige celui par la faute duquel il est arrivé à le réparer ».";
        assert_eq!(
            only(&run(&ext, near, &[], false).await, Kind::Citation).statut,
            Statut::Approchante
        );

        let wrong = "L'article 1240 du Code civil dispose que « Chacun est responsable des conséquences financières de ses actes volontaires et involontaires ».";
        let v = run(&ext, wrong, &[], false).await;
        let c = only(&v, Kind::Citation);
        assert_eq!(c.statut, Statut::Incoherente);
        assert!(c.detail.as_deref().unwrap().contains("ne figure pas"));
    }

    #[tokio::test]
    async fn a_quote_is_not_judged_when_the_article_text_is_unavailable() {
        let ext = FakeExt::default(); // article_text → Err
        let t = "L'article 1240 du Code civil dispose que « Chacun est responsable des conséquences financières de ses actes ».";
        let r = run(&ext, t, &[], false).await;
        assert!(r.verifications.iter().all(|v| v.kind != Kind::Citation));
    }

    // ── Citations et droit dans le temps ─────────────────────────────────────

    const ART_1140_ACTUEL: &str = "Il y a violence lorsqu'une partie s'engage sous la contrainte d'une violence physique ou morale exercée sur elle.";
    const ART_1140_2012: &str = "Les effets de l'obligation de donner ou de livrer un immeuble sont réglés au titre De la vente et au titre Des privilèges et hypothèques.";

    /// L'article 1140 : version courante `LEGIARTI_Code_civil_1140`, rédaction de 2012 `LEGIARTI_1140_2012`.
    fn ext_1140() -> FakeExt {
        let ext = FakeExt::default();
        {
            let mut a = ext.articles.lock().unwrap();
            a.insert(
                "LEGIARTI_Code_civil_1140".into(),
                Ok(ART_1140_ACTUEL.into()),
            );
            a.insert("LEGIARTI_1140_2012".into(), Ok(ART_1140_2012.into()));
        }
        ext.versions.lock().unwrap().insert(
            "LEGIARTI_Code_civil_1140".into(),
            Ok(Some("LEGIARTI_1140_2012".into())),
        );
        ext
    }

    #[tokio::test]
    async fn quote_of_the_old_wording_is_verified_against_the_version_at_the_date() {
        let ext = ext_1140();
        let t = "Le 27 octobre 2012, l'article 1140 du Code civil avait la rédaction suivante : « Les effets de l'obligation de donner ou de livrer un immeuble sont réglés au titre De la vente et au titre Des privilèges et hypothèques ».";
        // Sans date : comparée au texte actuel, la citation exacte de 2012 paraît fausse.
        let sans = run(&ext, t, &[], false).await;
        assert_eq!(only(&sans, Kind::Citation).statut, Statut::Incoherente);
        // Avec la date des faits : comparée à la rédaction de 2012.
        let r = run_at(&ext, t, "2012-10-27").await;
        let v = only(&r, Kind::Citation);
        assert_eq!(v.statut, Statut::Verifiee, "{:#?}", v);
        let d = v.detail.as_deref().unwrap();
        assert!(
            d.contains("tel quel") && d.contains("rédaction applicable au 27/10/2012"),
            "{d}"
        );
    }

    #[tokio::test]
    async fn quote_of_the_current_wording_is_flagged_when_the_date_is_earlier() {
        let ext = ext_1140();
        let t = "L'article 1140 du Code civil dispose que « Il y a violence lorsqu'une partie s'engage sous la contrainte d'une violence physique ou morale exercée sur elle ».";
        let r = run_at(&ext, t, "2012-10-27").await;
        let v = only(&r, Kind::Citation);
        assert_eq!(v.statut, Statut::Incoherente);
        let d = v.detail.as_deref().unwrap();
        assert!(
            d.contains("version actuelle") && d.contains("27/10/2012"),
            "{d}"
        );
        // Sans date, la même citation est vérifiée (texte actuel).
        assert_eq!(
            only(&run(&ext, t, &[], false).await, Kind::Citation).statut,
            Statut::Verifiee
        );
    }

    #[tokio::test]
    async fn altered_quote_of_the_old_wording_is_approximate_against_the_old_text() {
        let ext = ext_1140();
        let t = "L'article 1140 du Code civil disait : « Les effets de l'obligation de donner ou de livrer un immeuble sont régis au titre De la vente et au titre Des privilèges et hypothèques ».";
        let r = run_at(&ext, t, "2012-10-27").await;
        assert_eq!(only(&r, Kind::Citation).statut, Statut::Approchante);
    }

    #[tokio::test]
    async fn article_not_in_force_at_the_date_cannot_be_quoted() {
        let ext = ext_1140();
        ext.versions
            .lock()
            .unwrap()
            .insert("LEGIARTI_Code_civil_1140".into(), Ok(None));
        let t = "L'article 1140 du Code civil dispose que « Il y a violence lorsqu'une partie s'engage sous la contrainte d'une violence physique ou morale exercée sur elle ».";
        let v = run_at(&ext, t, "1990-01-01").await;
        let c = only(&v, Kind::Citation);
        assert_eq!(c.statut, Statut::Incoherente);
        assert!(c
            .detail
            .as_deref()
            .unwrap()
            .contains("n'était pas en vigueur au 01/01/1990"));
    }

    #[tokio::test]
    async fn unreadable_version_never_falls_back_to_a_verdict_on_the_current_text() {
        let ext = ext_1140();
        ext.versions
            .lock()
            .unwrap()
            .insert("LEGIARTI_Code_civil_1140".into(), Err("boom".into()));
        for t in [
            // fausse au regard du texte actuel : aucun « ⚠️ » à tort
            "L'article 1140 du Code civil disait « Les effets de l'obligation de donner ou de livrer un immeuble sont réglés au titre De la vente et au titre Des privilèges et hypothèques ».",
            // exacte au regard du texte actuel : pas davantage un « ✅ » sur la mauvaise version
            "L'article 1140 du Code civil dispose que « Il y a violence lorsqu'une partie s'engage sous la contrainte d'une violence physique ou morale exercée sur elle ».",
        ] {
            let r = run_at(&ext, t, "2012-10-27").await;
            let v = only(&r, Kind::Citation);
            assert_eq!(v.statut, Statut::NonVerifiable, "{t}");
            assert!(v.detail.as_deref().unwrap().contains("n'a pas pu être lue"));
        }
    }

    #[tokio::test]
    async fn quotes_are_compared_to_provided_documents_with_positive_verdicts_only() {
        let ext = FakeExt::default();
        let doc = "Attendu que le salarié a été licencié sans cause réelle et sérieuse, la cour d'appel a légalement justifié sa décision.";
        let src = vec![doc.to_string()];
        let exact = "La Cour juge que « le salarié a été licencié sans cause réelle et sérieuse, la cour d'appel a légalement justifié sa décision ».";
        let r = run(&ext, exact, &src, true).await;
        let v = only(&r, Kind::Citation);
        assert_eq!(v.statut, Statut::Verifiee);
        assert!(v.detail.as_deref().unwrap().contains("documents fournis"));

        let near = "La Cour juge que « le salarié a été licencié sans motif réel et sérieux, la cour d'appel a légalement justifié sa décision ».";
        assert_eq!(
            only(&run(&ext, near, &src, true).await, Kind::Citation).statut,
            Statut::Approchante
        );

        // Absent des documents : AUCUN verdict (ce peut être un résultat d'outil ou un extrait RAG).
        let other = "La Cour juge que « les parties doivent exécuter leurs obligations de bonne foi et sans retard injustifié ».";
        let r = run(&ext, other, &src, true).await;
        assert!(
            r.verifications.iter().all(|v| v.kind != Kind::Citation),
            "{:#?}",
            r.verifications
        );

        // Sans sources fournies : aucune comparaison.
        let r = run(&ext, exact, &[], false).await;
        assert!(r.verifications.iter().all(|v| v.kind != Kind::Citation));
    }

    #[tokio::test]
    async fn short_quotes_are_never_judged() {
        let ext = FakeExt::default();
        let src = vec!["le salarié a été licencié".to_string()];
        let r = run(&ext, "Il parle d'un « salarié licencié » ici.", &src, true).await;
        assert!(r.verifications.iter().all(|v| v.kind != Kind::Citation));
    }

    #[tokio::test]
    async fn the_number_of_lookups_per_request_is_capped() {
        let ext = FakeExt::default();
        let mut t = String::new();
        for i in 0..(MAX_DECISION_LOOKUPS + 5) {
            t.push_str(&format!(
                "Cass. soc., 12 mars 2024, n° 22-{:02}.{:03}.\n",
                10 + i % 80,
                100 + i
            ));
        }
        let r = run(&ext, &t, &[], false).await;
        assert_eq!(ext.calls.load(Ordering::SeqCst), MAX_DECISION_LOOKUPS);
        let capped = r
            .verifications
            .iter()
            .filter(|v| {
                v.kind == Kind::Decision
                    && v.detail.as_deref().is_some_and(|d| d.contains("Limite"))
            })
            .count();
        assert_eq!(capped, 5);
    }

    #[test]
    fn needs_network_only_when_something_can_be_checked() {
        let n = |t: &str| {
            let refs = verify::extract_article_refs(t);
            let texts = verify::extract_text_refs(t);
            let er = extract_ext_refs(t);
            needs_network(&er, &texts, &refs, t, false)
        };
        assert!(n("Cass. soc., 12 mars 2024, n° 22-12.345"));
        assert!(n("IDCC 1486"));
        assert!(n("le décret n° 2020-1455 du 27 novembre 2020"));
        assert!(!n("l'arrêté du 8 décembre 2021"));
        assert!(!n("une phrase sans référence"));
        assert!(n("L'article 1240 du Code civil dispose que « Tout fait quelconque de l'homme, qui cause à autrui un dommage »"));
        assert!(!n("Il dit que « Tout fait quelconque de l'homme, qui cause à autrui un dommage » sans article."));
    }

    // ── Cache ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn cache_keeps_successful_answers_only() {
        struct Counting {
            n: AtomicUsize,
            fail: bool,
        }
        impl ExtLookup for Counting {
            fn decision<'a>(
                &'a self,
                _: &'a str,
                _: &'a str,
            ) -> BoxFuture<'a, Result<DecisionLookup, String>> {
                let r = if self.fail {
                    Err("boom".to_string())
                } else {
                    Ok(DecisionLookup::Introuvable)
                };
                // Comptage au moment où la requête est réellement exécutée (futur interrogé).
                Box::pin(async move {
                    self.n.fetch_add(1, Ordering::SeqCst);
                    r
                })
            }
            fn texte<'a>(
                &'a self,
                _: &'a str,
                _: &'a str,
            ) -> BoxFuture<'a, Result<TexteLookup, String>> {
                Box::pin(async { Ok(TexteLookup::Introuvable) })
            }
            fn idcc<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<IdccLookup, String>> {
                Box::pin(async { Ok(IdccLookup::Introuvable) })
            }
            fn article_text<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<String, String>> {
                Box::pin(async { Err("x".to_string()) })
            }
        }
        let cache = crate::juridique::routes::new_cache();
        let ok = CachedExt::new(
            Counting {
                n: AtomicUsize::new(0),
                fail: false,
            },
            cache.clone(),
            "2026-10-23".into(),
        );
        ok.decision("2212345", "22-12.345").await.unwrap();
        ok.decision("2212345", "22-12.345").await.unwrap();
        assert_eq!(ok.inner.n.load(Ordering::SeqCst), 1);

        let ko = CachedExt::new(
            Counting {
                n: AtomicUsize::new(0),
                fail: true,
            },
            crate::juridique::routes::new_cache(),
            "2026-10-23".into(),
        );
        assert!(ko.decision("2212345", "22-12.345").await.is_err());
        assert!(ko.decision("2212345", "22-12.345").await.is_err());
        assert_eq!(
            ko.inner.n.load(Ordering::SeqCst),
            2,
            "les erreurs ne sont pas mémorisées"
        );
        let _ = Arc::new(());
    }

    // ── Adaptateur PISTE contre un faux serveur ──────────────────────────────

    use axum::{
        extract::{Query, State},
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };

    /// Faux serveur PISTE. Il valide la FORME des requêtes d'après les spécifications
    /// officielles (champs obligatoires de `/search`, propriété `id` de `kaliContIdcc`) et
    /// répond 400 sinon : une dérive du client est ainsi détectée par les tests.
    #[derive(Clone, Default)]
    struct Mock {
        judilibre_queries: Arc<Mutex<Vec<HashMap<String, String>>>>,
        legifrance_bodies: Arc<Mutex<Vec<Value>>>,
        taxonomy_calls: Arc<AtomicUsize>,
        taxonomy_down: bool,
        kali_consult_ids: Arc<Mutex<Vec<String>>>,
        /// `/consult/juri` (documenté pour le seul fonds JURI) refuse les décisions CETAT.
        consult_juri_unavailable: bool,
        consulted: Arc<Mutex<Vec<String>>>,
    }

    async fn judilibre_taxonomy(
        State(st): State<Mock>,
        Query(q): Query<HashMap<String, String>>,
    ) -> impl IntoResponse {
        st.taxonomy_calls.fetch_add(1, Ordering::SeqCst);
        if st.taxonomy_down {
            return (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response();
        }
        if q.get("id").map(String::as_str) == Some("location") {
            // Spec : `context_value` = `ca` pour les sièges de cours d'appel.
            assert_eq!(q.get("context_value").map(String::as_str), Some("ca"));
            return Json(json!({"id": "location", "result": {
                "ca75056": "Paris", "ca69123": "Lyon", "ca13001": "Aix-en-Provence"
            }}))
            .into_response();
        }
        assert_eq!(q.get("id").map(String::as_str), Some("field"));
        Json(json!({"id": "field", "result": {
            "expose": "Exposé du litige", "number": "Numéro de pourvoi", "sommaire": "Sommaire"
        }}))
        .into_response()
    }

    fn decision_json(id: &str, number: &str) -> Value {
        json!({
            "id": id, "number": number, "numbers": [number], "jurisdiction": "Cour de cassation",
            "chamber": "Chambre sociale", "decision_date": "2024-03-12", "solution": "Cassation",
            "publication": ["Publié au Bulletin"], "score": 1.0
        })
    }

    async fn judilibre_search(
        State(st): State<Mock>,
        Query(q): Query<HashMap<String, String>>,
    ) -> impl IntoResponse {
        st.judilibre_queries.lock().unwrap().push(q.clone());
        let query = q.get("query").cloned().unwrap_or_default();
        let targeted = q.get("field").map(String::as_str) == Some("number");
        let page = |results: Vec<Value>, relaxed: bool| {
            Json(
                json!({"page": 0, "page_size": 10, "total": results.len(), "took": 1,
                        "max_score": 1.0, "relaxed": relaxed, "results": results}),
            )
            .into_response()
        };
        let jur = q.get("jurisdiction").cloned().unwrap_or_default();
        let ca_dec = |id: &str, date: &str| {
            json!({"id": id, "number": "19/01234", "numbers": ["19/01234"], "jurisdiction": "ca",
                   "chamber": "Chambre sociale", "decision_date": date})
        };
        match (query.as_str(), targeted) {
            ("ECLI:FR:CCASS:2024:SO00123", false) if jur == "cc" => {
                let mut d = decision_json("ecli1", "22-12.345");
                d["ecli"] = json!("ECLI:FR:CCASS:2024:SO00123");
                page(vec![d], false)
            }
            // Résultat voisin d'un autre ECLI : n'est PAS la décision citée.
            ("ECLI:FR:CCASS:2024:SO00999", _) => {
                let mut d = decision_json("other", "22-99.999");
                d["ecli"] = json!("ECLI:FR:CCASS:2024:SO00001");
                page(vec![d], true)
            }
            ("19/01234", _) if jur == "ca" => {
                // Spec : `location` filtre sur le siège (clé de la taxonomie `location`).
                let loc = q.get("location").map(String::as_str);
                let all = [("ca-paris", "ca75056"), ("ca-lyon", "ca69123")];
                page(
                    all.iter()
                        .filter(|(_, l)| loc.is_none_or(|x| x == *l))
                        .map(|(id, _)| ca_dec(id, "2021-03-12"))
                        .collect(),
                    false,
                )
            }
            // Le numéro de pourvoi est une métadonnée : exclue de la recherche sans `field`.
            ("22-12.345", true) => page(vec![decision_json("644f5a3e", "22-12.345")], false),
            // Seule la recherche en texte intégral retrouve celle-ci.
            ("20-20.559", false) => page(vec![decision_json("9988aa", "20-20.559")], false),
            // Requête « élargie » automatiquement : d'autres décisions, aucune au bon numéro.
            ("21-15.789", _) | ("21-15789", _) => {
                page(vec![decision_json("zzz", "21-15.790")], true)
            }
            ("50-00.500", _) => (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response(),
            _ => page(vec![], false),
        }
    }

    /// Validation de forme de `POST /search` (champs `required` de la spec).
    fn search_shape_ok(b: &Value) -> bool {
        let r = &b["recherche"];
        b["fond"].is_string()
            && r["champs"].is_array()
            && r["operateur"].is_string()
            && r["pageNumber"].is_number()
            && r["pageSize"].is_number()
            && r["sort"].is_string()
            && r["typePagination"].is_string()
            && r["champs"][0]["typeChamp"].is_string()
            && r["champs"][0]["criteres"][0]["typeRecherche"].is_string()
            && r["champs"][0]["criteres"][0]["valeur"].is_string()
    }

    async fn legifrance_search(
        State(st): State<Mock>,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        st.legifrance_bodies.lock().unwrap().push(body.clone());
        if !search_shape_ok(&body) {
            return (StatusCode::BAD_REQUEST, "requête non conforme à la spec").into_response();
        }
        let fond = body["fond"].as_str().unwrap_or("").to_string();
        let champ = body["recherche"]["champs"][0]["typeChamp"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let valeur = body["recherche"]["champs"][0]["criteres"][0]["valeur"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let decret = |num: &str| {
            json!({"nature": "DECRET", "num": num, "etat": "VIGUEUR",
                   "dateSignature": "2020-11-27T00:00:00.000+0000",
                   "titles": [{"id": "JORFTEXT000042587977", "cid": "JORFTEXT000042587977",
                               "legalStatus": "VIGUEUR",
                               "title": format!("Décret n° {num} du 27 novembre 2020 relatif à X")}]})
        };
        // Champs du fond CETAT d'après « description-des-tris-et-filtres » : ALL, TITLE, NUM_DEC,
        // ABSTRATS, TEXTE, RESUMES. NUM_AFFAIRE et ECLI (fond JURI) y sont refusés.
        if fond == "CETAT"
            && !["ALL", "TITLE", "NUM_DEC", "ABSTRATS", "TEXTE", "RESUMES"]
                .contains(&champ.as_str())
        {
            return (StatusCode::BAD_REQUEST, "champ inconnu pour ce fond").into_response();
        }
        let cetat =
            |id: &str, title: &str| json!({"titles": [{"id": id, "cid": id, "title": title}]});
        let ce42 = || {
            cetat(
                "CETATEXT000042",
                "Conseil d'État, 5ème - 6ème chambres réunies, 02/07/2020, 428411, Publié au recueil Lebon",
            )
        };
        let ce43 = || {
            cetat(
                "CETATEXT000043",
                "Conseil d'État, 1ère chambre, 05/01/2021, 111111",
            )
        };
        let results: Vec<Value> = match (fond.as_str(), champ.as_str(), valeur.as_str()) {
            ("CETAT", "NUM_DEC" | "ALL", "428411") => vec![ce43(), ce42()],
            // Recherche qui ramène une décision VOISINE : son numéro n'est pas celui cité.
            ("CETAT", "NUM_DEC" | "ALL", "999999" | "111111") => vec![ce43()],
            ("CETAT", "NUM_DEC", "19NT01234") => vec![cetat(
                "CETATEXT000044",
                "CAA de NANTES, 1ère chambre, 12/03/2021, 19NT01234, Inédit au recueil Lebon",
            )],
            ("CETAT", "NUM_DEC", "50000") => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response()
            }
            ("LODA_ETAT", "NUM", "2020-1455") => vec![decret("2020-1455")],
            // « 2018-771 » : seul le texte intégral le retrouve.
            ("LODA_ETAT", "ALL", "2018-771") => vec![decret("2018-771")],
            _ => vec![],
        };
        Json(json!({"totalResultNumber": results.len(), "results": results})).into_response()
    }

    /// `POST /consult/kaliContIdcc` : la spec exige la propriété `id`.
    async fn kali_idcc(State(st): State<Mock>, Json(body): Json<Value>) -> impl IntoResponse {
        let Some(id) = body["id"].as_str() else {
            return (StatusCode::BAD_REQUEST, "propriété `id` requise").into_response();
        };
        st.kali_consult_ids.lock().unwrap().push(id.to_string());
        match id {
            // IDCC à zéros de tête : seule l'écriture sans zéros est connue du faux.
            "9999" | "0016" => (StatusCode::NOT_FOUND, "inconnu").into_response(),
            "7777" => (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response(),
            // Un conteneur : identifiant et titre, AUCUN champ d'état (cf. spec).
            _ => Json(json!({
                "id": "KALICONT000005635173",
                "titre": format!("Convention IDCC {id}"),
                "nature": "CONVENTION COLLECTIVE NATIONALE"
            }))
            .into_response(),
        }
    }

    /// `GET /decision` (schéma `decisionFull`) : chaînage arrière, avant et rapprochements.
    async fn judilibre_decision(Query(q): Query<HashMap<String, String>>) -> impl IntoResponse {
        match q.get("id").map(String::as_str) {
            Some("6500abc") => Json(json!({
                "id": "6500abc", "jurisdiction": "Cour de cassation", "chamber": "Chambre sociale",
                "number": "22-12.345", "ecli": "ECLI:FR:CCASS:2024:SO00123",
                "decision_date": "2024-03-12", "solution": "Cassation",
                "contested": {"id": "6400def", "title": "Arrêt du 5 mai 2022", "date": "2022-05-05",
                    "jurisdiction": "Cour d'appel", "location": "Cour d'appel de Paris", "number": "19/01234"},
                "forward": {"id": "6600ghi", "title": "Arrêt sur renvoi", "date": "2025-01-15",
                    "jurisdiction": "Cour d'appel", "location": "Cour d'appel de Lyon"},
                "rapprochements": [{"id": "6300aaa", "title": "Soc., 10 mars 2020", "date": "2020-03-10",
                    "jurisdiction": "Cour de cassation"}]
            }))
            .into_response(),
            Some("6bad") => Json(json!({"message": "réponse sans identifiant"})).into_response(),
            _ => (StatusCode::NOT_FOUND, "inconnue").into_response(),
        }
    }

    /// `POST /consult/juri` (schéma `TexteSimple`).
    async fn consult_juri(State(st): State<Mock>, Json(body): Json<Value>) -> impl IntoResponse {
        let Some(id) = body["textId"].as_str() else {
            return (StatusCode::BAD_REQUEST, "propriété `textId` requise").into_response();
        };
        st.consulted.lock().unwrap().push(id.to_string());
        if st.consult_juri_unavailable {
            return (StatusCode::NOT_FOUND, "texte du fonds CETAT non géré").into_response();
        }
        let text = match id {
            "JURITEXT000099" => json!({
                "titre": "Cour d'appel de Nancy, 12 mars 2021", "juridiction": "Cour d'appel",
                "natureJuridiction": "Cour d'appel", "numeroAffaire": ["19/01234"],
                "dateTexte": "2021-03-12T00:00:00.000+0000",
                "decisionAttaquee": {"date": "2019-01-07T00:00:00.000+0000", "formation": "Conseil de prud'hommes"},
                "liens": [{"id": "JURITEXT000041", "title": "Cour de cassation, 5 mai 2022",
                           "typeLien": "CITATION", "sens": "cible"}]
            }),
            "CETATEXT000042" => json!({
                "titre": "Conseil d'État, 5ème - 6ème chambres réunies, 02/07/2020, 428411, Publié au recueil Lebon",
                "juridiction": "CONSEIL D'ETAT", "formation": "5ème - 6ème chambres réunies",
                "solution": "Annulation", "numeroAffaire": ["428411"],
                "ecli": "ECLI:FR:CECHR:2020:428411.20200702", "dateTexte": "2020-07-02T00:00:00.000+0000"
            }),
            "CETATEXT000043" => json!({
                "titre": "Conseil d'État, 1ère chambre, 05/01/2021, 111111",
                "juridiction": "CONSEIL D'ETAT", "numeroAffaire": ["111111"],
                "ecli": "ECLI:FR:CECHR:2021:111111.20210105", "dateTexte": "2021-01-05T00:00:00.000+0000"
            }),
            "CETATEXT000044" => json!({
                "titre": "CAA de NANTES, 12/03/2021, 19NT01234",
                "juridiction": "COUR ADMINISTRATIVE D'APPEL DE NANTES", "numeroAffaire": ["19NT01234"],
                "dateTexte": 1615503600000i64
            }),
            _ => return (StatusCode::NOT_FOUND, "inconnu").into_response(),
        };
        Json(json!({"text": text})).into_response()
    }

    async fn get_article(Json(body): Json<Value>) -> impl IntoResponse {
        if body["id"] == "LEGIARTI1" {
            Json(json!({"article": {"id": "LEGIARTI1", "texte": "<p>Texte de l'article</p>"}}))
                .into_response()
        } else {
            (StatusCode::NOT_FOUND, "absent").into_response()
        }
    }

    async fn spawn(st: Mock) -> String {
        let app = Router::new()
            .route(
                "/oauth/token",
                post(|| async { Json(json!({"access_token": "t", "expires_in": 3600})) }),
            )
            .route("/judilibre/taxonomy", get(judilibre_taxonomy))
            .route("/judilibre/search", get(judilibre_search))
            .route("/judilibre/decision", get(judilibre_decision))
            .route("/legifrance/search", post(legifrance_search))
            .route("/legifrance/consult/kaliContIdcc", post(kali_idcc))
            .route("/legifrance/consult/getArticle", post(get_article))
            .route("/legifrance/consult/juri", post(consult_juri))
            .with_state(st);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn piste(st: &Mock) -> PisteExt {
        PisteExt::new(PisteClient::new_for_test(&spawn(st.clone()).await))
    }

    fn query_of(q: &HashMap<String, String>) -> (String, Option<String>) {
        (q["query"].clone(), q.get("field").cloned())
    }

    #[tokio::test]
    async fn adapter_decision_targets_the_pourvoi_field_found_in_the_taxonomy() {
        let st = Mock::default();
        let p = piste(&st).await;
        match p.decision("2212345", "22-12.345").await.unwrap() {
            DecisionLookup::Trouvee(i) => {
                assert_eq!(i.id, "644f5a3e");
                assert_eq!(i.date.as_deref(), Some("2024-03-12"));
                assert_eq!(i.chamber.as_deref(), Some("Chambre sociale"));
            }
            other => panic!("{other:?}"),
        }
        {
            let qs = st.judilibre_queries.lock().unwrap();
            assert_eq!(qs.len(), 1, "la recherche ciblée suffit");
            assert_eq!(
                query_of(&qs[0]),
                ("22-12.345".to_string(), Some("number".to_string()))
            );
            assert_eq!(qs[0]["operator"], "exact");
            assert_eq!(qs[0]["resolve_references"], "true");
        }
        // La clé du champ est mémorisée : une seconde décision ne relit pas la taxonomie, et
        // se retrouve par le texte intégral quand la recherche ciblée ne donne rien.
        match p.decision("2020559", "20-20.559").await.unwrap() {
            DecisionLookup::Trouvee(i) => assert_eq!(i.id, "9988aa"),
            other => panic!("{other:?}"),
        }
        assert_eq!(st.taxonomy_calls.load(Ordering::SeqCst), 1);
        let qs = st.judilibre_queries.lock().unwrap();
        let seq: Vec<(String, Option<String>)> = qs.iter().skip(1).map(query_of).collect();
        assert_eq!(
            seq,
            vec![
                ("20-20.559".to_string(), Some("number".to_string())),
                ("20-20.559".to_string(), None)
            ]
        );
    }

    #[tokio::test]
    async fn adapter_decision_still_works_when_the_taxonomy_is_unavailable() {
        let st = Mock {
            taxonomy_down: true,
            ..Mock::default()
        };
        let p = piste(&st).await;
        match p.decision("2020559", "20-20.559").await.unwrap() {
            DecisionLookup::Trouvee(i) => assert_eq!(i.id, "9988aa"),
            other => panic!("{other:?}"),
        }
        let qs = st.judilibre_queries.lock().unwrap();
        assert!(
            qs.iter().all(|q| !q.contains_key("field")),
            "pas de champ ciblé sans taxonomie"
        );
    }

    #[tokio::test]
    async fn adapter_decision_ignores_relaxed_results_and_gives_up_softly() {
        let st = Mock::default();
        let p = piste(&st).await;
        // Le faux renvoie une requête « élargie » (relaxed) : d'autres décisions, jamais celle-ci.
        assert_eq!(
            p.decision("2115789", "21-15.789").await.unwrap(),
            DecisionLookup::Introuvable
        );
        let qs = st.judilibre_queries.lock().unwrap();
        let seq: Vec<(String, Option<String>)> = qs.iter().map(query_of).collect();
        assert_eq!(
            seq,
            vec![
                ("21-15.789".to_string(), Some("number".to_string())),
                ("21-15.789".to_string(), None),
                ("21-15789".to_string(), None),
            ]
        );
    }

    #[tokio::test]
    async fn adapter_decision_http_error_is_an_error_not_a_verdict() {
        let st = Mock::default();
        let p = piste(&st).await;
        assert!(p.decision("5000500", "50-00.500").await.is_err());
    }

    #[tokio::test]
    async fn adapter_text_search_uses_the_official_num_field_then_full_text() {
        let st = Mock::default();
        let p = piste(&st).await;
        match p.texte("decret", "2020-1455").await.unwrap() {
            TexteLookup::Trouve { id, etat, date, .. } => {
                assert_eq!(id, "JORFTEXT000042587977");
                assert_eq!(etat.as_deref(), Some("VIGUEUR"));
                assert_eq!(
                    date.as_deref(),
                    Some("2020-11-27"),
                    "dateSignature de la recherche"
                );
            }
            other => panic!("{other:?}"),
        }
        {
            let bodies = st.legifrance_bodies.lock().unwrap();
            assert_eq!(bodies.len(), 1, "le champ NUM suffit");
            let b = &bodies[0];
            assert_eq!(b["fond"], "LODA_ETAT");
            assert_eq!(b["recherche"]["champs"][0]["typeChamp"], "NUM");
            assert_eq!(
                b["recherche"]["champs"][0]["criteres"][0]["typeRecherche"],
                "EXACTE"
            );
        }
        // Absent de l'index NUM : repli sur le texte intégral.
        assert!(matches!(
            p.texte("decret", "2018-771").await.unwrap(),
            TexteLookup::Trouve { .. }
        ));
        let kinds: Vec<String> = st
            .legifrance_bodies
            .lock()
            .unwrap()
            .iter()
            .map(|b| {
                b["recherche"]["champs"][0]["typeChamp"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(kinds, vec!["NUM", "NUM", "ALL"]);
        assert_eq!(
            p.texte("decret", "2000-1").await.unwrap(),
            TexteLookup::Introuvable
        );
    }

    #[tokio::test]
    async fn adapter_idcc_reads_the_container_through_the_consultation_with_the_id_property() {
        let st = Mock::default();
        let p = piste(&st).await;
        match p.idcc("1486").await.unwrap() {
            IdccLookup::Trouve { id, titre, etat } => {
                assert_eq!(id, "KALICONT000005635173");
                assert_eq!(titre.as_deref(), Some("Convention IDCC 1486"));
                assert_eq!(
                    etat, None,
                    "un conteneur de convention n'a pas d'état dans la spec"
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(*st.kali_consult_ids.lock().unwrap(), vec!["1486"]);
        assert!(
            st.legifrance_bodies.lock().unwrap().is_empty(),
            "aucune recherche KALI"
        );
    }

    #[tokio::test]
    async fn adapter_idcc_tries_the_spelling_without_leading_zeros_on_404() {
        let st = Mock::default();
        let p = piste(&st).await;
        assert!(matches!(
            p.idcc("0016").await.unwrap(),
            IdccLookup::Trouve { .. }
        ));
        assert_eq!(*st.kali_consult_ids.lock().unwrap(), vec!["0016", "16"]);
    }

    #[tokio::test]
    async fn adapter_idcc_404_is_not_found_and_other_errors_are_not_verdicts() {
        let st = Mock::default();
        let p = piste(&st).await;
        assert_eq!(p.idcc("9999").await.unwrap(), IdccLookup::Introuvable);
        assert!(p.idcc("7777").await.is_err());
    }

    #[tokio::test]
    async fn adapter_reads_the_text_of_an_article() {
        let st = Mock::default();
        let p = piste(&st).await;
        assert_eq!(
            p.article_text("LEGIARTI1").await.unwrap().trim(),
            "Texte de l'article"
        );
        assert!(p.article_text("LEGIARTI2").await.is_err());
    }

    // ── Guillemets droits et article de la question ──────────────────────────

    #[test]
    fn straight_quotes_are_paired_only_when_unambiguous() {
        let t = "Il dit : \"Tout fait quelconque de l'homme, qui cause à autrui un dommage\" fin.";
        let q = extract_quotes(t);
        assert_eq!(q.len(), 1);
        assert_eq!(&t[q[0].close_start..q[0].close_end], "\"");
        assert!(q[0].text.starts_with("Tout fait quelconque"));
        assert!(q[0].words >= 9);

        // Deux citations dans le même paragraphe : appariées deux à deux.
        let t2 = "A : \"première citation assez longue ici\" puis B : \"seconde citation assez longue aussi\".";
        assert_eq!(extract_quotes(t2).len(), 2);

        // Nombre impair de guillemets : on n'apparie rien.
        assert!(extract_quotes("Il dit \"une citation assez longue mais mal fermée").is_empty());
        assert!(extract_quotes("a \"x\" b \"une citation assez longue mais impaire").is_empty());
        // Mot isolé, pouces, attribut : pas des citations.
        assert!(extract_quotes("Le mot \"licenciement\" ici.").is_empty());
        assert!(extract_quotes("un écran de 5\" et un autre de 6\" de large").is_empty());
        // Dans du code : ignoré.
        assert!(extract_quotes("```\n\"une citation assez longue dans du code\"\n```").is_empty());
        // Blockquote Markdown comme dans l'interface.
        let t3 = "> \"Les obligations naissent, soit du contrat, soit du délit, soit du quasi-délit, soit de la loi.\"";
        assert_eq!(extract_quotes(t3).len(), 1);
    }

    fn quotes_of(t: &str) -> Vec<QuoteRef> {
        extract_quotes(t)
    }

    #[test]
    fn the_question_designates_an_article_only_when_the_answer_does_not() {
        let answer = "\"Tout fait quelconque de l'homme, qui cause à autrui un dommage, oblige celui qui l'a causé.\"";
        let q = quotes_of(answer);
        let refs = verify::extract_article_refs(answer);
        let r = context_article_ref("Cite mot pour mot l'article 1240 du Code civil.", &refs, &q)
            .unwrap();
        assert_eq!(r.code, Some("Code civil"));
        assert_eq!(r.numero_display(), "1240");

        // Deux articles distincts dans la question : ambigu.
        assert!(context_article_ref(
            "Compare l'article 1240 et l'article 1241 du Code civil",
            &refs,
            &q
        )
        .is_none());
        // Même article cité deux fois : toujours un seul.
        assert!(context_article_ref(
            "L'article 1240 du Code civil, bien l'article 1240 ?",
            &refs,
            &q
        )
        .is_some());
        // La réponse cite elle-même un article de code : la proximité décide.
        let with_ref = "L'article 1103 du Code civil : \"Les contrats légalement formés tiennent lieu de loi à ceux qui les ont faits\"";
        let refs2 = verify::extract_article_refs(with_ref);
        assert!(
            context_article_ref("l'article 1240 du Code civil", &refs2, &quotes_of(with_ref))
                .is_none()
        );
        // Pas de citation assez longue dans la réponse.
        let short = "Voici \"un mot isolé\" seulement.";
        assert!(
            context_article_ref("l'article 1240 du Code civil", &[], &quotes_of(short)).is_none()
        );
        // Pas d'article dans la question.
        assert!(context_article_ref("Parle-moi de la responsabilité", &refs, &q).is_none());
    }

    #[tokio::test]
    async fn the_exact_screenshot_case_a_wrong_quote_with_no_article_in_the_answer_is_flagged() {
        let ext = FakeExt::default();
        ext.articles
            .lock()
            .unwrap()
            .insert("LEGIARTI_Code_civil_1240".into(), Ok(ART_1240.into()));
        let question = "Cite mot pour mot l'article 1240 du Code civil.";
        let answer = "\"Les obligations naissent, soit du contrat, soit du délit, soit du quasi-délit, soit de la loi.\"";

        let refs = verify::extract_article_refs(answer);
        let er = extract_ext_refs(answer);
        let r = context_article_ref(question, &refs, &er.quotes).expect("article de la question");
        let ctx = crate::juridique::routes::resolve_context_article(&FakeArticles, &r)
            .await
            .unwrap();
        assert_eq!(ctx.reference, "Code civil, art. 1240");
        assert_eq!(ctx.etat_libelle.as_deref(), Some("en vigueur"));

        let report = run_with_context(&ext, answer, &[], false, Some(&ctx)).await;
        let v = only(&report, Kind::Citation);
        assert_eq!(v.statut, Statut::Incoherente);
        assert_eq!(v.occurrences[0].texte, "\"");
        let d = v.detail.as_deref().unwrap();
        assert!(
            d.contains("Code civil, art. 1240") && d.contains("cité dans votre question"),
            "{d}"
        );
        assert!(d.contains("ne figure pas"), "{d}");
    }

    #[tokio::test]
    async fn a_correct_quote_of_the_question_article_is_verified() {
        let ext = FakeExt::default();
        ext.articles
            .lock()
            .unwrap()
            .insert("LEGIARTI_Code_civil_1240".into(), Ok(ART_1240.into()));
        let ctx = ContextArticle {
            reference: "Code civil, art. 1240".into(),
            code: Some("Code civil".into()),
            id: "LEGIARTI_Code_civil_1240".into(),
            etat_libelle: Some("en vigueur".into()),
        };
        let answer = "Voici le texte : \"Tout fait quelconque de l'homme, qui cause à autrui un dommage, oblige celui par la faute duquel il est arrivé à le réparer.\"";
        let r = run_with_context(&ext, answer, &[], false, Some(&ctx)).await;
        let v = only(&r, Kind::Citation);
        assert_eq!(v.statut, Statut::Verifiee);
        assert_eq!(v.code.as_deref(), Some("Code civil"));
    }

    #[tokio::test]
    async fn with_several_quotes_the_question_article_never_yields_a_negative_verdict() {
        let ext = FakeExt::default();
        ext.articles
            .lock()
            .unwrap()
            .insert("LEGIARTI_Code_civil_1240".into(), Ok(ART_1240.into()));
        let ctx = ContextArticle {
            reference: "Code civil, art. 1240".into(),
            code: Some("Code civil".into()),
            id: "LEGIARTI_Code_civil_1240".into(),
            etat_libelle: None,
        };
        // Un passage de l'article et une citation d'une décision : on ne sait pas laquelle est « l'article ».
        let answer = "Le texte : \"Tout fait quelconque de l'homme, qui cause à autrui un dommage, oblige celui par la faute duquel il est arrivé à le réparer.\"\n\nLa Cour juge que \"le salarié a été licencié sans cause réelle et sérieuse par l'employeur\".";
        let r = run_with_context(&ext, answer, &[], false, Some(&ctx)).await;
        let cites: Vec<&Verification> = r
            .verifications
            .iter()
            .filter(|v| v.kind == Kind::Citation)
            .collect();
        assert_eq!(cites.len(), 1, "{cites:#?}");
        assert_eq!(
            cites[0].statut,
            Statut::Verifiee,
            "la décision n'est PAS déclarée absente de l'article"
        );
    }

    #[tokio::test]
    async fn the_question_article_is_ignored_when_its_text_is_unavailable() {
        let ext = FakeExt::default(); // article_text → Err
        let ctx = ContextArticle {
            reference: "Code civil, art. 1240".into(),
            code: Some("Code civil".into()),
            id: "LEGIARTI_inconnu".into(),
            etat_libelle: None,
        };
        let answer = "\"Les obligations naissent, soit du contrat, soit du délit, soit du quasi-délit, soit de la loi.\"";
        let r = run_with_context(&ext, answer, &[], false, Some(&ctx)).await;
        assert!(r.verifications.iter().all(|v| v.kind != Kind::Citation));
    }

    #[tokio::test]
    async fn needs_network_is_true_for_a_question_article() {
        let t = "\"Les obligations naissent, soit du contrat, soit du délit, soit du quasi-délit, soit de la loi.\"";
        let refs = verify::extract_article_refs(t);
        let texts = verify::extract_text_refs(t);
        let er = extract_ext_refs(t);
        assert!(!needs_network(&er, &texts, &refs, t, false));
        assert!(needs_network(&er, &texts, &refs, t, true));
    }

    // ── Blocs de citation et rattachement à travers une introduction ─────────

    /// Article 1140 du Code civil, tel que Légifrance l'affiche (version en vigueur depuis le
    /// 1er octobre 2016).
    const ART_1140: &str = "Il y a violence lorsqu'une partie s'engage sous la pression d'une contrainte qui lui inspire la crainte d'exposer sa personne, sa fortune ou celles de ses proches à un mal considérable.";

    /// Réponse sans outils (capture 1) : article nommé dans l'introduction, citation fausse dans
    /// un bloc séparé par une ligne vide.
    const ANSWER_WITHOUT_TOOLS: &str = "De mémoire, l'article 1140 du Code civil (dans sa version issue de l'ordonnance de réforme du droit des obligations de 2016) dispose que :\n\n> « Les obligations naissent, soit du contrat, soit du délit, soit du quasi-délit, soit de la loi. »\n\n**À vérifier** sur Légifrance pour confirmer le libellé exact.";

    /// Réponse avec MCP (capture 2) : bloc de citation SANS guillemets, étiqueté « Article 1140 »,
    /// avec un « et actuel » absent du texte officiel.
    const ANSWER_WITH_MCP: &str = "Voici l'article 1140 du Code civil (en vigueur) :\n\n> ***Article 1140*** Il y a violence lorsqu'une partie s'engage sous la pression d'une contrainte qui lui inspire la crainte d'exposer sa personne, sa fortune ou celles de ses proches à un mal considérable et actuel.\n\n---\n\n**Contexte** : cet article fait partie des vices du consentement (Art. 1130 et suivants).";

    #[test]
    fn a_quote_is_attached_across_one_paragraph_break_announced_by_a_colon() {
        let t = ANSWER_WITHOUT_TOOLS;
        let refs = verify::extract_article_refs(t);
        let q = extract_quotes(t);
        assert_eq!(
            q.len(),
            1,
            "la citation entre « » dans le bloc n'est pas comptée deux fois"
        );
        assert!(refs.iter().any(|r| r.code.is_some()));
        assert!(attached_article(&q[0], &refs, t).is_some());

        // Sans « : » en fin d'introduction, le saut de paragraphe coupe le lien.
        let t2 = t.replace("dispose que :", "dispose que.");
        let refs2 = verify::extract_article_refs(&t2);
        let q2 = extract_quotes(&t2);
        assert_eq!(attached_article(&q2[0], &refs2, &t2), None);

        // Deux sauts de paragraphe : trop loin.
        let t3 = t.replace(
            "dispose que :\n\n>",
            "dispose que :\n\nAutre paragraphe.\n\n>",
        );
        let refs3 = verify::extract_article_refs(&t3);
        let q3 = extract_quotes(&t3);
        assert_eq!(attached_article(&q3[0], &refs3, &t3), None);
    }

    #[test]
    fn a_blockquote_without_quotation_marks_is_a_quotation() {
        let t = ANSWER_WITH_MCP;
        let q = extract_quotes(t);
        assert_eq!(q.len(), 1);
        assert!(q[0].text.contains("Il y a violence lorsqu'une partie"));
        assert!(q[0].words >= 30);
        // Le badge se place sur le dernier caractère significatif (le point), pas sur `*` ou `_`.
        assert_eq!(&t[q[0].close_start..q[0].close_end], ".");
        assert!(t[q[0].close_end..].starts_with("\n\n---"));

        // Trop court : un intertitre en citation n'est pas un texte cité.
        assert!(extract_quotes("> Voir plus bas\n\nSuite").is_empty());
        // Plusieurs lignes `>` consécutives forment un seul bloc.
        let multi = "> Il y a violence lorsqu'une partie s'engage\n> sous la pression d'une contrainte qui lui inspire la crainte.";
        let m = extract_quotes(multi);
        assert_eq!(m.len(), 1);
        assert!(m[0].text.contains('\n'));
        // Dans du code : ignoré.
        assert!(extract_quotes(
            "```\n> Il y a violence lorsqu'une partie s'engage sous la pression\n```"
        )
        .is_empty());
    }

    #[test]
    fn an_article_label_heading_a_quotation_is_not_part_of_the_text() {
        for (input, want) in [
            ("***Article 1140*** Il y a violence", "Il y a violence"),
            ("**Article 1140 :** Il y a violence", "Il y a violence"),
            ("Art. 1140 - Il y a violence", "Il y a violence"),
            ("Article L. 1234-5 Le contrat", "Le contrat"),
            ("Il y a violence", "Il y a violence"),
            ("L'article 1140 dit que", "L'article 1140 dit que"),
        ] {
            assert_eq!(strip_article_label(input), want, "{input}");
        }
    }

    #[test]
    fn the_diff_names_the_words_added_to_the_official_text() {
        // Le cas de la capture : « et actuel » ajouté en fin de phrase.
        let quote = "Il y a violence lorsqu'une partie s'engage sous la pression d'une contrainte qui lui inspire la crainte d'exposer sa personne, sa fortune ou celles de ses proches à un mal considérable et actuel.";
        let (added, omitted) = quote_diff(quote, ART_1140);
        assert_eq!(added, vec!["et actuel"]);
        assert!(omitted.is_empty(), "{omitted:?}");
        // Texte identique : aucun écart.
        assert_eq!(quote_diff(ART_1140, ART_1140), (vec![], vec![]));
        // Apostrophes typographiques et casse n'y changent rien.
        let typo = ART_1140.replace('\'', "’").to_uppercase();
        assert_eq!(quote_diff(&typo, ART_1140), (vec![], vec![]));
    }

    #[test]
    fn the_diff_names_replaced_and_omitted_words() {
        // « sa fortune » supprimé, « considérable » remplacé par « grave ».
        let quote = "Il y a violence lorsqu'une partie s'engage sous la pression d'une contrainte qui lui inspire la crainte d'exposer sa personne ou celles de ses proches à un mal grave.";
        let (added, omitted) = quote_diff(quote, ART_1140);
        assert_eq!(added, vec!["grave"]);
        assert_eq!(omitted, vec!["sa fortune", "considérable"], "{omitted:?}");
        // Seule la zone effectivement citée compte : citer le début ne « omet » pas la fin.
        let start = "Il y a violence lorsqu'une partie s'engage sous la pression d'une contrainte";
        assert_eq!(quote_diff(start, ART_1140), (vec![], vec![]));
    }

    #[test]
    fn the_diff_stays_silent_when_the_texts_are_not_comparable() {
        let unrelated = "Les obligations naissent, soit du contrat, soit du délit, soit du quasi-délit, soit de la loi.";
        assert_eq!(quote_diff(unrelated, ART_1140), (vec![], vec![]));
        assert_eq!(quote_diff("trop court", ART_1140), (vec![], vec![]));
        assert_eq!(quote_diff(ART_1140, ""), (vec![], vec![]));
        // Garde de volume : pas de calcul quadratique sur de très longs textes.
        let long = "mot ".repeat(5_000);
        assert_eq!(quote_diff(&long, &long), (vec![], vec![]));
    }

    fn ctx_none() -> Option<&'static ContextArticle> {
        None
    }

    #[tokio::test]
    async fn capture_1_a_misquote_announced_in_the_introduction_is_flagged() {
        let ext = FakeExt::default();
        ext.articles
            .lock()
            .unwrap()
            .insert("LEGIARTI_Code_civil_1140".into(), Ok(ART_1140.into()));
        let r = run_with_context(&ext, ANSWER_WITHOUT_TOOLS, &[], false, ctx_none()).await;
        let v = only(&r, Kind::Citation);
        assert_eq!(v.statut, Statut::Incoherente);
        assert!(v.detail.as_deref().unwrap().contains("ne figure pas"));
        assert!(
            v.reference.contains("art. 1140") || v.reference.contains("1140"),
            "{}",
            v.reference
        );
        // L'article lui-même reste vérifié : ✅ sur la référence, ⚠️ sur la citation.
        assert!(r
            .verifications
            .iter()
            .any(|x| x.kind == Kind::Article && x.statut == Statut::EnVigueur));
    }

    #[tokio::test]
    async fn capture_2_an_extra_et_actuel_in_a_blockquote_is_flagged_as_close_but_different() {
        let ext = FakeExt::default();
        ext.articles
            .lock()
            .unwrap()
            .insert("LEGIARTI_Code_civil_1140".into(), Ok(ART_1140.into()));
        let r = run_with_context(&ext, ANSWER_WITH_MCP, &[], false, ctx_none()).await;
        let v = only(&r, Kind::Citation);
        assert_eq!(v.statut, Statut::Approchante, "{:#?}", v);
        assert!(v.detail.as_deref().unwrap().contains("non identique"));
        assert_eq!(v.occurrences[0].texte, ".");
        assert_eq!(
            v.infos,
            vec!["Mots ajoutés par rapport au texte de l'article : « et actuel »"],
            "l'info-bulle dit QUOI diffère"
        );
    }

    #[tokio::test]
    async fn the_exact_official_text_in_a_labelled_blockquote_is_verified() {
        let ext = FakeExt::default();
        ext.articles
            .lock()
            .unwrap()
            .insert("LEGIARTI_Code_civil_1140".into(), Ok(ART_1140.into()));
        let ok = ANSWER_WITH_MCP.replace(" et actuel.", ".");
        let r = run_with_context(&ext, &ok, &[], false, ctx_none()).await;
        let v = only(&r, Kind::Citation);
        assert_eq!(v.statut, Statut::Verifiee, "{:#?}", v);
        assert!(v.detail.as_deref().unwrap().contains("tel quel"));
    }

    // ── Handler HTTP de bout en bout ─────────────────────────────────────────

    use crate::juridique::routes::{verifier_citations_route, HasJuridiqueState, JuridiqueState};

    struct TestState {
        js: JuridiqueState,
    }
    impl HasJuridiqueState for TestState {
        fn juridique_state(&self) -> &JuridiqueState {
            &self.js
        }
    }

    async fn spawn_api(js: JuridiqueState) -> String {
        let app: Router = Router::new()
            .route(
                "/verifier-citations",
                verifier_citations_route::<TestState>(),
            )
            .route(
                "/decision/:id/fiche",
                get(super::super::routes::get_decision_fiche::<TestState>),
            )
            .with_state(Arc::new(TestState { js }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn post_json(api: &str, body: Value) -> (u16, Value) {
        let res = reqwest::Client::new()
            .post(format!("{api}/verifier-citations"))
            .json(&body)
            .send()
            .await
            .unwrap();
        (
            res.status().as_u16(),
            res.json().await.unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn handler_end_to_end_decisions_texts_and_agreements() {
        let st = Mock::default();
        let base = spawn(st.clone()).await;
        let js = JuridiqueState::new(None, None);
        *js.piste_client.write().await = Some(PisteClient::new_for_test(&base));
        let api = spawn_api(js).await;

        let texte = "Cass. soc., 13 mars 2024, n° 22-12.345 ; le décret n° 2020-1455 du 27 novembre 2020 ; \
                     la convention (IDCC 1486) ; Cass. com., 3 mai 2023, n° 21-15.789.";
        let (status, body) = post_json(&api, json!({ "texte": texte })).await;
        assert_eq!(status, 200);
        let v = body["verifications"].as_array().unwrap();
        let by = |kind: &str, needle: &str| -> &Value {
            v.iter()
                .find(|x| x["kind"] == kind && x["reference"].as_str().unwrap().contains(needle))
                .unwrap_or_else(|| panic!("{kind} {needle} : {v:#?}"))
        };

        let d = by("decision", "22-12.345");
        assert_eq!(
            d["statut"], "incoherente",
            "date citée 13 mars, décision du 12 mars"
        );
        assert!(d["detail"].as_str().unwrap().contains("13/03/2024"));
        assert_eq!(d["url"], "https://www.courdecassation.fr/decision/644f5a3e");
        assert!(d["infos"].as_array().unwrap().len() >= 2);
        assert!(
            d.get("intern").is_none(),
            "données de travail jamais exposées"
        );

        assert_eq!(by("decision", "21-15.789")["statut"], "non_retrouvee");
        assert_eq!(by("texte", "2020-1455")["statut"], "en_vigueur");
        assert_eq!(by("convention", "IDCC 1486")["statut"], "verifiee");
        let _ = st;
    }

    #[tokio::test]
    async fn handler_without_piste_still_reports_anchoring_and_never_fails() {
        let api = spawn_api(JuridiqueState::new(None, None)).await;
        let texte = "Cass. soc., 12 mars 2024, n° 22-12.345.";
        let (status, body) = post_json(
            &api,
            json!({ "texte": texte, "ancrage": true, "sources": ["autre chose"] }),
        )
        .await;
        assert_eq!(status, 200);
        let d = &body["verifications"][0];
        assert_eq!(d["kind"], "decision");
        assert_eq!(d["ancrage"], "hors_sources");
        assert_eq!(d["statut"], "erreur", "pas de verdict sans PISTE");

        // Sans PISTE, la comparaison des citations aux documents fournis fonctionne quand même.
        let doc = "Attendu que le salarié a été licencié sans cause réelle et sérieuse, la cour d'appel a légalement justifié sa décision.";
        let (status, body) = post_json(
            &api,
            json!({
                "texte": "La Cour juge que « le salarié a été licencié sans cause réelle et sérieuse, la cour d'appel a légalement justifié sa décision ».",
                "ancrage": true,
                "sources": [doc],
            }),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["verifications"][0]["kind"], "citation");
        assert_eq!(body["verifications"][0]["statut"], "verifiee");
    }

    // ── ECLI, Conseil d'État, juridictions administratives, cours d'appel ─────

    fn ce_info(date: &str) -> DecisionInfo {
        DecisionInfo {
            id: "CETATEXT000042".into(),
            number: "428411".into(),
            chamber: Some("5ème - 6ème chambres réunies".into()),
            date: Some(date.into()),
            solution: Some("Annulation".into()),
            source: Some("Légifrance".into()),
            url: Some("https://www.legifrance.gouv.fr/ceta/id/CETATEXT000042".into()),
            ..Default::default()
        }
    }

    fn found(ext: &FakeExt, id: &str, info: DecisionInfo) {
        ext.decisions
            .lock()
            .unwrap()
            .insert(id.into(), Ok(DecisionLookup::Trouvee(info)));
    }

    #[tokio::test]
    async fn a_conseil_d_etat_decision_is_verified_with_its_identity() {
        let ext = FakeExt::default();
        found(&ext, "428411", ce_info("2020-07-02"));
        let r = run(&ext, "Voir CE, 2 juillet 2020, n° 428411.", &[], false).await;
        let v = only(&r, Kind::Decision);
        assert_eq!(v.statut, Statut::Verifiee, "{v:#?}");
        assert_eq!(
            v.url.as_deref(),
            Some("https://www.legifrance.gouv.fr/ceta/id/CETATEXT000042")
        );
        assert!(v
            .infos
            .iter()
            .any(|i| i.starts_with("Conseil d'État — 5ème")));
        assert!(v.infos.iter().any(|i| i.contains("02/07/2020")));
        assert!(v.infos.iter().any(|i| i.contains("Source : Légifrance")));
    }

    #[tokio::test]
    async fn a_wrong_date_on_a_real_conseil_d_etat_decision_is_incoherent() {
        let ext = FakeExt::default();
        found(&ext, "428411", ce_info("2020-07-02"));
        let r = run(&ext, "CE, 3 juillet 2020, n° 428411", &[], false).await;
        let v = only(&r, Kind::Decision);
        assert_eq!(v.statut, Statut::Incoherente);
        assert!(v.detail.as_deref().unwrap().contains("03/07/2020"));
    }

    #[tokio::test]
    async fn an_invented_decision_is_never_presented_as_verified() {
        let ext = FakeExt::default(); // tout est « introuvable »
        let text = "CE, 2 juillet 2020, n° 428411 ; CAA Nantes, 12 mars 2021, n° 19NT01234 ; \
                    CA Paris, 12 mars 2021, RG n° 19/01234 ; ECLI:FR:CCASS:2024:SO00123.";
        let r = run(&ext, text, &[], false).await;
        let ds: Vec<&Verification> = r
            .verifications
            .iter()
            .filter(|v| v.kind == Kind::Decision)
            .collect();
        assert_eq!(ds.len(), 4, "{ds:#?}");
        for v in ds {
            assert_eq!(v.statut, Statut::NonRetrouvee, "{v:#?}");
            assert_ne!(v.existe, Some(true));
            assert!(v.detail.as_deref().unwrap().contains("à vérifier"));
        }
    }

    #[tokio::test]
    async fn an_uncertain_appeal_decision_gets_no_verdict() {
        let ext = FakeExt::default();
        ext.decisions.lock().unwrap().insert(
            "19/01234".into(),
            Ok(DecisionLookup::Incertaine {
                raison: "RG partagé par plusieurs cours.".into(),
            }),
        );
        let r = run(&ext, "CA Paris, 12 mars 2021, RG n° 19/01234", &[], false).await;
        let v = only(&r, Kind::Decision);
        assert_eq!((v.statut, v.existe), (Statut::NonRetrouvee, None));
        assert_eq!(v.detail.as_deref(), Some("RG partagé par plusieurs cours."));
    }

    #[tokio::test]
    async fn a_lookup_failure_is_an_error_not_a_verdict() {
        let ext = FakeExt::default();
        ext.decisions
            .lock()
            .unwrap()
            .insert("428411".into(), Err("HTTP 500".into()));
        let r = run(&ext, "CE, 2 juillet 2020, n° 428411", &[], false).await;
        let v = only(&r, Kind::Decision);
        assert_eq!(v.statut, Statut::Erreur);
        assert!(v.detail.as_deref().unwrap().contains("Légifrance"));
    }

    #[tokio::test]
    async fn an_ecli_of_an_unqueried_court_is_flagged_without_any_call() {
        let ext = FakeExt::default();
        let text = "ECLI:FR:CC:2020:2020.800.DC et ECLI:EU:C:2020:559";
        let r = run(&ext, text, &[], false).await;
        assert_eq!(ext.calls.load(Ordering::SeqCst), 0);
        let ds: Vec<&Verification> = r
            .verifications
            .iter()
            .filter(|v| v.kind == Kind::Decision)
            .collect();
        assert_eq!(ds.len(), 2);
        for v in ds {
            assert_eq!(v.statut, Statut::NonVerifiable);
            assert!(v
                .detail
                .as_deref()
                .unwrap()
                .starts_with("Non vérifiée par Juridix"));
        }
        let er = extract_ext_refs(text);
        assert!(
            !needs_network(&er, &[], &[], text, false),
            "aucun appel PISTE requis"
        );
    }

    #[tokio::test]
    async fn an_unqueried_ecli_says_where_to_check_it() {
        let ext = FakeExt::default();
        let text =
            "ECLI:EU:C:2020:559 ; ECLI:CE:ECHR:2020:0101JUD000000 ; ECLI:FR:CC:2020:2020.800.DC";
        let r = run(&ext, text, &[], false).await;
        let d: Vec<&str> = r
            .verifications
            .iter()
            .filter(|v| v.kind == Kind::Decision)
            .map(|v| v.detail.as_deref().unwrap())
            .collect();
        assert_eq!(d.len(), 3, "{d:?}");
        assert!(d[0].contains("EUR-Lex"), "{}", d[0]);
        assert!(d[1].contains("HUDOC"), "{}", d[1]);
        assert!(d[2].contains("conseil-constitutionnel.fr"), "{}", d[2]);
        assert!(d.iter().all(|x| x.contains("ni confirmée ni infirmée")));
    }

    #[tokio::test]
    async fn an_ecli_found_under_its_pourvoi_number_in_the_sources_is_anchored() {
        let ext = FakeExt::default();
        found(
            &ext,
            "ECLI:FR:CCASS:2024:SO00123",
            info("Chambre sociale", "2024-03-12"),
        );
        let sources = vec!["Arrêt Cass. soc., 12 mars 2024, pourvoi n° 22-12.345".to_string()];
        let r = run(&ext, "ECLI:FR:CCASS:2024:SO00123", &sources, true).await;
        let v = only(&r, Kind::Decision);
        assert_eq!(v.statut, Statut::Verifiee);
        assert_eq!(v.ancrage, Some(Ancrage::DansSources));
        // Une décision absente des sources reste « hors sources ».
        let r = run(
            &ext,
            "ECLI:FR:CCASS:2024:SO00123",
            &["autre chose".to_string()],
            true,
        )
        .await;
        assert_eq!(only(&r, Kind::Decision).ancrage, Some(Ancrage::HorsSources));
    }

    #[tokio::test]
    async fn the_cache_key_of_an_appeal_decision_depends_on_its_date_and_seat() {
        let q = |date: &str, place: &str| DecisionQuery {
            court: Court::CourAppel,
            number: Some("19/01234".into()),
            display: "19/01234".into(),
            ecli: None,
            date: Some(date.into()),
            place: Some(place.into()),
        };
        assert_ne!(
            q("2021-03-12", "Paris").cache_key(),
            q("2021-03-12", "Lyon").cache_key()
        );
        assert_ne!(
            q("2021-03-12", "Paris").cache_key(),
            q("2021-03-13", "Paris").cache_key()
        );
        assert_eq!(
            q("2021-03-12", "PARIS").cache_key(),
            q("2021-03-12", "Paris").cache_key()
        );
    }

    // ── Adaptateur PISTE contre le faux serveur : nouvelles juridictions ─────

    fn cq(court: Court, number: Option<&str>, ecli: Option<&str>) -> DecisionQuery {
        DecisionQuery {
            court,
            number: number.map(String::from),
            display: number.unwrap_or("").into(),
            ecli: ecli.map(String::from),
            date: None,
            place: None,
        }
    }

    #[tokio::test]
    async fn adapter_conseil_d_etat_is_identified_by_its_title_and_confirmed_by_reading() {
        let st = Mock::default();
        let p = piste(&st).await;
        match p
            .decision_ref(&cq(Court::ConseilEtat, Some("428411"), None))
            .await
            .unwrap()
        {
            DecisionLookup::Trouvee(i) => {
                assert_eq!(
                    i.id, "CETATEXT000042",
                    "le candidat 000043 (n° 111111) est écarté"
                );
                assert_eq!(i.date.as_deref(), Some("2020-07-02"));
                assert_eq!(i.chamber.as_deref(), Some("5ème - 6ème chambres réunies"));
                assert_eq!(
                    i.solution.as_deref(),
                    Some("Annulation"),
                    "enrichi par la lecture"
                );
                assert_eq!(i.note, None);
            }
            other => panic!("{other:?}"),
        }
        // Une recherche qui ramène seulement une décision VOISINE n'est pas une vérification.
        let none = p
            .decision_ref(&cq(Court::ConseilEtat, Some("999999"), None))
            .await
            .unwrap();
        assert_eq!(none, DecisionLookup::Introuvable);
        // Une erreur de l'API n'est jamais un verdict.
        assert!(p
            .decision_ref(&cq(Court::ConseilEtat, Some("50000"), None))
            .await
            .is_err());
        // D'après la spec, le numéro se cherche dans NUM_DEC sur le fond CETAT.
        let b = st.legifrance_bodies.lock().unwrap();
        assert_eq!(b[0]["fond"], "CETAT");
        assert_eq!(b[0]["recherche"]["champs"][0]["typeChamp"], "NUM_DEC");
        // Le candidat voisin (titre d'un autre numéro) n'est même pas lu.
        assert!(!st
            .consulted
            .lock()
            .unwrap()
            .contains(&"CETATEXT000043".to_string()));
    }

    #[tokio::test]
    async fn adapter_conseil_d_etat_is_verified_even_if_consult_juri_refuses_cetat_texts() {
        // `/consult/juri` est documenté pour le seul fonds JURI : la vérification ne doit
        // pas en dépendre.
        let st = Mock {
            consult_juri_unavailable: true,
            ..Mock::default()
        };
        let p = piste(&st).await;
        match p
            .decision_ref(&cq(Court::ConseilEtat, Some("428411"), None))
            .await
            .unwrap()
        {
            DecisionLookup::Trouvee(i) => {
                assert_eq!(i.id, "CETATEXT000042");
                assert_eq!(i.date.as_deref(), Some("2020-07-02"));
                assert_eq!(i.solution, None, "pas de lecture : pas de solution");
            }
            other => panic!("{other:?}"),
        }
        // Une ECLI est alors rapprochée du numéro et de la date qu'elle porte, et le dit.
        let q = cq(
            Court::ConseilEtat,
            None,
            Some("ECLI:FR:CECHR:2020:428411.20200702"),
        );
        match p.decision_ref(&q).await.unwrap() {
            DecisionLookup::Trouvee(i) => {
                assert!(i.note.as_deref().unwrap().contains("n'a pas pu être lue"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn adapter_administrative_appeal_court_checks_the_court_of_the_decision() {
        let p = piste(&Mock::default()).await;
        match p
            .decision_ref(&cq(Court::CourAdminAppel, Some("19NT01234"), None))
            .await
            .unwrap()
        {
            DecisionLookup::Trouvee(i) => assert_eq!(i.date.as_deref(), Some("2021-03-12")),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn adapter_ecli_of_the_conseil_d_etat_is_matched_on_its_number_and_date() {
        let st = Mock::default();
        let p = piste(&st).await;
        let q = cq(
            Court::ConseilEtat,
            None,
            Some("ECLI:FR:CECHR:2020:428411.20200702"),
        );
        match p.decision_ref(&q).await.unwrap() {
            DecisionLookup::Trouvee(i) => {
                assert_eq!(i.id, "CETATEXT000042");
                // Lue : l'ECLI de la décision est celui cité.
                assert_eq!(
                    i.ecli.as_deref(),
                    Some("ECLI:FR:CECHR:2020:428411.20200702")
                );
            }
            other => panic!("{other:?}"),
        }
        // Même numéro, autre date : l'ECLI cité ne désigne aucune décision retrouvée.
        let q = cq(
            Court::ConseilEtat,
            None,
            Some("ECLI:FR:CECHR:2021:428411.20210702"),
        );
        assert_eq!(
            p.decision_ref(&q).await.unwrap(),
            DecisionLookup::Introuvable
        );
        // ECLI de forme inconnue : recherche en texte intégral, rien à rapprocher.
        let q = cq(Court::ConseilEtat, None, Some("ECLI:FR:CECHR:2020:ABC"));
        assert_eq!(
            p.decision_ref(&q).await.unwrap(),
            DecisionLookup::Introuvable
        );
        // Jamais un champ que le fond CETAT ne propose pas (le faux serveur répondrait 400).
        let champs: Vec<String> = st
            .legifrance_bodies
            .lock()
            .unwrap()
            .iter()
            .map(|b| {
                b["recherche"]["champs"][0]["typeChamp"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert!(
            champs.iter().all(|c| c == "NUM_DEC" || c == "ALL"),
            "{champs:?}"
        );
    }

    #[tokio::test]
    async fn adapter_ecli_of_the_cour_de_cassation_requires_the_exact_ecli() {
        let st = Mock::default();
        let p = piste(&st).await;
        match p
            .decision_ref(&cq(
                Court::Cassation,
                None,
                Some("ECLI:FR:CCASS:2024:SO00123"),
            ))
            .await
            .unwrap()
        {
            DecisionLookup::Trouvee(i) => {
                assert_eq!(i.id, "ecli1");
                assert_eq!(i.ecli.as_deref(), Some("ECLI:FR:CCASS:2024:SO00123"));
            }
            other => panic!("{other:?}"),
        }
        // Judilibre renvoie une décision voisine : jamais « vérifiée ».
        let none = p
            .decision_ref(&cq(
                Court::Cassation,
                None,
                Some("ECLI:FR:CCASS:2024:SO00999"),
            ))
            .await
            .unwrap();
        assert_eq!(none, DecisionLookup::Introuvable);
        let qs = st.judilibre_queries.lock().unwrap();
        assert!(qs.iter().all(|q| q["jurisdiction"] == "cc"));
    }

    #[tokio::test]
    async fn adapter_appeal_court_rg_is_narrowed_by_date_and_seat_filter() {
        let st = Mock::default();
        let p = piste(&st).await;
        let mut q = cq(Court::CourAppel, Some("19/01234"), None);
        q.date = Some("2021-03-12".into());
        q.place = Some("Lyon".into());
        match p.decision_ref(&q).await.unwrap() {
            DecisionLookup::Trouvee(i) => assert_eq!(i.id, "ca-lyon"),
            other => panic!("{other:?}"),
        }
        // Le siège a été appliqué comme filtre `location` (clé de la taxonomie), pas deviné.
        let qs = st.judilibre_queries.lock().unwrap().clone();
        assert!(qs
            .iter()
            .any(|x| x.get("location").map(String::as_str) == Some("ca69123")));
        assert!(qs.iter().all(|x| x["jurisdiction"] == "ca"));
        // Date inexacte : pas de verdict.
        q.date = Some("2019-01-01".into());
        assert!(matches!(
            p.decision_ref(&q).await.unwrap(),
            DecisionLookup::Incertaine { .. }
        ));
        // Siège cité sans arrêt de ce numéro dans ce siège : autre cour d'appel.
        q.date = None;
        q.place = Some("Aix-en-Provence".into());
        assert!(matches!(
            p.decision_ref(&q).await.unwrap(),
            DecisionLookup::Incertaine { .. }
        ));
        // Siège identifié, sans date : l'arrêt du siège cité suffit.
        q.place = Some("Paris".into());
        match p.decision_ref(&q).await.unwrap() {
            DecisionLookup::Trouvee(i) => assert_eq!(i.id, "ca-paris"),
            other => panic!("{other:?}"),
        }
        // Ni date ni siège : jamais vérifiée.
        q.place = None;
        assert!(matches!(
            p.decision_ref(&q).await.unwrap(),
            DecisionLookup::Incertaine { .. }
        ));
    }

    #[tokio::test]
    async fn handler_end_to_end_new_courts_have_distinct_visible_statuses() {
        let st = Mock::default();
        let base = spawn(st.clone()).await;
        let js = JuridiqueState::new(None, None);
        *js.piste_client.write().await = Some(PisteClient::new_for_test(&base));
        let api = spawn_api(js).await;
        let texte = "CE, 2 juillet 2020, n° 428411 ; CE, 4 mai 2021, n° 999999 ; \
                     ECLI:FR:CCASS:2024:SO00123 ; ECLI:FR:CCASS:2024:SO00999 ; \
                     CA Paris, 12 mars 2021, RG n° 19/01234 ; ECLI:FR:CC:2020:2020.800.DC.";
        let (status, body) = post_json(&api, json!({ "texte": texte })).await;
        assert_eq!(status, 200);
        let v = body["verifications"].as_array().unwrap();
        let by = |needle: &str| -> &Value {
            v.iter()
                .find(|x| {
                    x["kind"] == "decision" && x["reference"].as_str().unwrap().contains(needle)
                })
                .unwrap_or_else(|| panic!("{needle} : {v:#?}"))
        };
        assert_eq!(by("428411")["statut"], "verifiee");
        assert_eq!(
            by("428411")["url"],
            "https://www.legifrance.gouv.fr/ceta/id/CETATEXT000042"
        );
        assert_eq!(by("999999")["statut"], "non_retrouvee");
        assert_eq!(by("SO00123")["statut"], "verifiee");
        assert_eq!(by("SO00999")["statut"], "non_retrouvee");
        assert_eq!(by("19/01234")["statut"], "verifiee");
        assert_eq!(by("2020.800.DC")["statut"], "non_verifiable");
        // Aucune référence inventée n'est « vérifiée » ni « introuvable ».
        for x in v.iter().filter(|x| x["kind"] == "decision") {
            if x["statut"] != "verifiee" {
                assert_ne!(x["existe"], true);
            }
            assert_ne!(x["statut"], "introuvable");
        }
    }

    // ── Fiche d'une décision : rapprochements et provenance ───────────────────

    async fn get_fiche(api: &str, id: &str) -> (u16, Value) {
        let res = reqwest::Client::new()
            .get(format!("{api}/decision/{id}/fiche"))
            .send()
            .await
            .unwrap();
        (
            res.status().as_u16(),
            res.json().await.unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn the_fiche_route_gives_the_links_of_a_judilibre_decision_with_their_provenance() {
        let st = Mock::default();
        let base = spawn(st.clone()).await;
        let js = JuridiqueState::new(None, None);
        *js.piste_client.write().await = Some(PisteClient::new_for_test(&base));
        let api = spawn_api(js).await;

        let (status, f) = get_fiche(&api, "6500abc").await;
        assert_eq!(status, 200, "{f}");
        assert_eq!(f["source"], "Judilibre");
        assert_eq!(f["niveau"], "cassation");
        let r = f["rapprochements"].as_array().unwrap();
        let rel: Vec<(&str, &str)> = r
            .iter()
            .map(|x| {
                (
                    x["relation"].as_str().unwrap(),
                    x["provenance"]["champ"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            rel,
            [
                ("decision_attaquee", "contested"),
                ("decision_suivante", "forward"),
                ("rapprochement", "rapprochements")
            ]
        );
        assert_eq!(r[0]["niveau"], "fond");
        assert_eq!(r[1]["chronologie"], "posterieure");
        assert!(f["indisponibles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["element"] == "revirements"));
    }

    #[tokio::test]
    async fn the_fiche_route_reads_legifrance_decisions_and_rejects_bad_requests() {
        let st = Mock::default();
        let base = spawn(st.clone()).await;
        let js = JuridiqueState::new(None, None);
        *js.piste_client.write().await = Some(PisteClient::new_for_test(&base));
        let api = spawn_api(js).await;

        let (status, f) = get_fiche(&api, "JURITEXT000099").await;
        assert_eq!(status, 200, "{f}");
        assert_eq!(f["source"], "Légifrance");
        assert_eq!(f["niveau"], "fond");
        assert_eq!(f["rapprochements"][1]["provenance"]["champ"], "liens");
        assert_eq!(f["rapprochements"][1]["type_lien"], "CITATION");
        // Réponse de la source sans la forme attendue : jamais une fiche vide.
        let (status, _) = get_fiche(&api, "6bad").await;
        assert_eq!(status, 502);
        // Identifiant non alphanumérique : rien ne part vers l'API.
        let (status, _) = get_fiche(&api, "a%2Fb%3Fx").await;
        assert_eq!(status, 400);
    }

    #[tokio::test]
    async fn a_verified_decision_carries_the_id_of_its_fiche() {
        let ext = FakeExt::default();
        found(&ext, "428411", ce_info("2020-07-02"));
        let r = run(&ext, "CE, 2 juillet 2020, n° 428411", &[], false).await;
        assert_eq!(
            only(&r, Kind::Decision).decision_id.as_deref(),
            Some("CETATEXT000042")
        );
        // Non retrouvée : pas de fiche à proposer.
        let r = run(
            &FakeExt::default(),
            "CE, 2 juillet 2020, n° 428411",
            &[],
            false,
        )
        .await;
        assert_eq!(only(&r, Kind::Decision).decision_id, None);
    }

    // ── Test réel (manuel) ───────────────────────────────────────────────────

    /// Contrôle contre la vraie API PISTE. Ignoré par défaut : il exige des identifiants
    /// et des références que VOUS savez exister. À lancer avant chaque release :
    ///
    /// ```text
    /// PISTE_CLIENT_ID=… PISTE_CLIENT_SECRET=… \
    /// JURIDIX_LIVE_POURVOI="22-12.345" JURIDIX_LIVE_DECRET="2020-1455" JURIDIX_LIVE_IDCC="1486" \
    /// cargo test --lib live_piste -- --ignored --nocapture
    /// ```
    ///
    /// Chaque variable `JURIDIX_LIVE_*` est facultative (le contrôle correspondant est
    /// sauté) ; mettez-y des références réelles : un pourvoi publié, un décret du JORF, un IDCC.
    #[tokio::test]
    #[ignore = "nécessite des identifiants PISTE et des références réelles"]
    async fn live_piste_decision_text_and_agreement_lookups() {
        let (Ok(id), Ok(secret)) = (
            std::env::var("PISTE_CLIENT_ID"),
            std::env::var("PISTE_CLIENT_SECRET"),
        ) else {
            panic!("PISTE_CLIENT_ID et PISTE_CLIENT_SECRET requis");
        };
        let p = PisteExt::new(PisteClient::new(id, secret).unwrap());

        if let Ok(n) = std::env::var("JURIDIX_LIVE_POURVOI") {
            let key = digits_of(&n);
            assert_eq!(key.len(), 7, "format AA-NN.NNN attendu");
            let res = p
                .decision(&key, &display_pourvoi(&key))
                .await
                .expect("appel Judilibre");
            println!("décision {n} → {res:#?}");
            assert!(
                matches!(res, DecisionLookup::Trouvee(_)),
                "décision non retrouvée par numéro"
            );
        }
        if let Ok(n) = std::env::var("JURIDIX_LIVE_DECRET") {
            let res = p.texte("decret", &n).await.expect("appel Légifrance");
            println!("décret {n} → {res:#?}");
            assert!(
                matches!(res, TexteLookup::Trouve { .. }),
                "décret non retrouvé par numéro"
            );
        }
        if let Ok(n) = std::env::var("JURIDIX_LIVE_IDCC") {
            let res = p.idcc(&format!("{n:0>4}")).await.expect("appel KALI");
            println!("IDCC {n} → {res:#?}");
            assert!(
                matches!(res, IdccLookup::Trouve { .. }),
                "IDCC non retrouvé"
            );
        }
        // Un numéro qui n'existe pas doit donner « introuvable », pas une erreur.
        let none = p
            .decision("9999999", "99-99.999")
            .await
            .expect("appel Judilibre");
        println!("pourvoi inexistant → {none:?}");
    }

    /// Contrôle réel des nouvelles juridictions. À lancer avant chaque release :
    ///
    /// ```text
    /// PISTE_CLIENT_ID=… PISTE_CLIENT_SECRET=… \
    /// JURIDIX_LIVE_CE="428411" JURIDIX_LIVE_ECLI_CCASS="ECLI:FR:CCASS:…" \
    /// JURIDIX_LIVE_ECLI_CE="ECLI:FR:CECHR:…" JURIDIX_LIVE_RG="19/01234" JURIDIX_LIVE_RG_DATE="2021-03-12" \
    /// cargo test --lib live_piste_juridictions -- --ignored --nocapture
    /// ```
    ///
    /// Chaque variable est facultative. Mettez-y des références réelles : ce test vérifie
    /// que la recherche (NUM_DEC sur CETAT, ECLI Judilibre, filtre location) et la forme des réponses réelles
    /// permettent bien de retrouver la décision.
    #[tokio::test]
    #[ignore = "nécessite des identifiants PISTE et des références réelles"]
    async fn live_piste_juridictions() {
        let (Ok(id), Ok(secret)) = (
            std::env::var("PISTE_CLIENT_ID"),
            std::env::var("PISTE_CLIENT_SECRET"),
        ) else {
            panic!("PISTE_CLIENT_ID et PISTE_CLIENT_SECRET requis");
        };
        let p = PisteExt::new(PisteClient::new(id, secret).unwrap());
        let env = |k: &str| std::env::var(k).ok();
        let q = |court, number: Option<String>, ecli: Option<String>| DecisionQuery {
            court,
            display: number.clone().unwrap_or_default(),
            number,
            ecli,
            date: env("JURIDIX_LIVE_RG_DATE"),
            place: None,
        };
        let mut checks: Vec<(&str, DecisionQuery)> = Vec::new();
        if let Some(n) = env("JURIDIX_LIVE_CE") {
            checks.push(("CE", q(Court::ConseilEtat, Some(n), None)));
        }
        if let Some(e) = env("JURIDIX_LIVE_ECLI_CCASS") {
            checks.push((
                "ECLI Cass.",
                q(Court::Cassation, None, Some(e.to_uppercase())),
            ));
        }
        if let Some(e) = env("JURIDIX_LIVE_ECLI_CE") {
            checks.push((
                "ECLI CE",
                q(Court::ConseilEtat, None, Some(e.to_uppercase())),
            ));
        }
        if let Some(n) = env("JURIDIX_LIVE_RG") {
            checks.push(("RG", q(Court::CourAppel, Some(n), None)));
        }
        for (name, query) in checks {
            let res = p.decision_ref(&query).await.expect("appel PISTE");
            println!("{name} → {res:#?}");
            assert!(
                matches!(res, DecisionLookup::Trouvee(_)),
                "{name} non retrouvée"
            );
        }
        // Références inventées : « non retrouvée », jamais « vérifiée », jamais une erreur.
        for query in [
            q(Court::ConseilEtat, Some("999998".into()), None),
            q(
                Court::Cassation,
                None,
                Some("ECLI:FR:CCASS:1999:SO99999".into()),
            ),
        ] {
            let res = p.decision_ref(&query).await.expect("appel PISTE");
            println!("inventée → {res:?}");
            assert!(!matches!(res, DecisionLookup::Trouvee(_)));
        }
    }

    /// Contrôle réel de la fiche d'une décision (rapprochements fournis par les API) :
    ///
    /// ```text
    /// PISTE_CLIENT_ID=… PISTE_CLIENT_SECRET=… JURIDIX_LIVE_FICHE_IDS="<id Judilibre>,JURITEXT…" \
    /// cargo test --lib live_piste_fiche -- --ignored --nocapture
    /// ```
    ///
    /// Affiche chaque fiche : vérifiez que `contested`, `forward` et `rapprochements`
    /// (Judilibre) ou `liens` (Légifrance) sont bien lus sur de vraies réponses.
    #[tokio::test]
    #[ignore = "nécessite des identifiants PISTE et des identifiants de décisions réels"]
    async fn live_piste_fiche() {
        use crate::juridique::jurisprudence::{
            fiche_judilibre, fiche_legifrance, is_legifrance_id,
        };
        let (Ok(id), Ok(secret), Ok(ids)) = (
            std::env::var("PISTE_CLIENT_ID"),
            std::env::var("PISTE_CLIENT_SECRET"),
            std::env::var("JURIDIX_LIVE_FICHE_IDS"),
        ) else {
            panic!("PISTE_CLIENT_ID, PISTE_CLIENT_SECRET et JURIDIX_LIVE_FICHE_IDS requis");
        };
        let client = PisteClient::new(id, secret).unwrap();
        for id in ids.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            let fiche = if is_legifrance_id(id) {
                let v = legifrance::get_juri(&client, id)
                    .await
                    .expect("appel Légifrance");
                fiche_legifrance(id, &v)
            } else {
                let v = judilibre::get_decision(&client, id, true, None, None)
                    .await
                    .expect("appel Judilibre");
                fiche_judilibre(&v)
            };
            println!("{id} → {fiche:#?}");
            assert!(fiche.is_some(), "{id} : réponse de forme inattendue");
        }
    }

    // ── Clés d'ancrage ───────────────────────────────────────────────────────

    #[test]
    fn grounding_keys_cover_decisions_and_agreements() {
        let k = verify::grounding_keys(
            "Cass. soc., 12 mars 2024, n° 22-12.345 ; IDCC 1486 ; art. 1103 du Code civil",
        );
        assert!(k.contains(&"dec|2212345".to_string()), "{k:?}");
        assert!(k.contains(&"idcc|1486".to_string()), "{k:?}");
        assert!(k.iter().any(|x| x.starts_with("art|Code civil|")), "{k:?}");
        let mut g = GroundingIndex::default();
        g.extend_keys(k);
        assert!(g.has_decision("2212345") && g.has_idcc("1486"));
        assert!(!g.has_decision("0000000"));
        // Clés étrangères ignorées.
        let mut g = GroundingIndex::default();
        g.extend_keys(vec!["xxx|1".to_string()]);
        assert!(g.is_empty());
    }
}
