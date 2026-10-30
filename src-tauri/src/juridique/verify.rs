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
// Juridix — Module Juridique — verify.rs
// ============================================================================
// Vérification des références d'articles de code citées par le LLM
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
    legifrance::{self, FiltreValeurs, SearchFond},
    routes::{cache_get, cache_set, Cache},
};

// ── Paramètres ────────────────────────────────────────────────────────────────

/// Nombre maximal de références *distinctes* vérifiées par requête.
pub const MAX_DISTINCT_LOOKUPS: usize = 25;
/// Appels Légifrance simultanés (borné pour ne pas marteler PISTE).
pub const LOOKUP_CONCURRENCY: usize = 4;
/// Taille maximale du texte analysé.
pub const MAX_TEXT_BYTES: usize = 400_000;
/// Nombre maximal d'occurrences renvoyées (garde-fou sur la taille de réponse).
pub(crate) const MAX_OCCURRENCES: usize = 300;
/// Durée de vie du cache de vérification. La clé contient la date du jour ;
/// ce TTL n'est qu'un plafond.
const TTL_VERIFICATION: Duration = Duration::from_secs(12 * 3600);

// ── Table des codes ───────────────────────────────────────────────────────────

/// Un code et ses désignations usuelles.
///
/// `nom` = intitulé officiel Légifrance (valeur de la facette `NOM_CODE`).
/// `aliases` : désignations en minuscules SANS accents, apostrophe droite.
/// Un `.` y est facultatif dans le texte source et un espace tolère les
/// espaces multiples : « c. trav. » reconnaît « C.trav. », « C. trav » etc.
struct CodeDef {
    nom: &'static str,
    aliases: &'static [&'static str],
}

static CODES: &[CodeDef] = &[
    CodeDef {
        nom: "Code civil",
        aliases: &["code civil", "c. civ."],
    },
    CodeDef {
        nom: "Code du travail",
        aliases: &["code du travail", "c. trav."],
    },
    CodeDef {
        nom: "Code pénal",
        aliases: &["code penal", "c. pen."],
    },
    CodeDef {
        nom: "Code de procédure civile",
        aliases: &["code de procedure civile", "c. proc. civ.", "cpc"],
    },
    CodeDef {
        nom: "Code de procédure pénale",
        aliases: &["code de procedure penale", "c. proc. pen.", "cpp"],
    },
    CodeDef {
        nom: "Code de commerce",
        aliases: &["code de commerce", "c. com."],
    },
    CodeDef {
        nom: "Code de la consommation",
        aliases: &["code de la consommation", "c. consom.", "c. conso."],
    },
    CodeDef {
        nom: "Code général des impôts",
        aliases: &["code general des impots", "cgi"],
    },
    CodeDef {
        nom: "Livre des procédures fiscales",
        aliases: &["livre des procedures fiscales", "lpf"],
    },
    CodeDef {
        nom: "Code de la sécurité sociale",
        aliases: &["code de la securite sociale", "c. sec. soc.", "css"],
    },
    CodeDef {
        nom: "Code de l'urbanisme",
        aliases: &["code de l'urbanisme", "c. urb."],
    },
    CodeDef {
        nom: "Code de la construction et de l'habitation",
        aliases: &["code de la construction et de l'habitation", "cch"],
    },
    CodeDef {
        nom: "Code monétaire et financier",
        aliases: &["code monetaire et financier", "cmf"],
    },
    CodeDef {
        nom: "Code de justice administrative",
        aliases: &["code de justice administrative", "cja"],
    },
    CodeDef {
        nom: "Code des relations entre le public et l'administration",
        aliases: &[
            "code des relations entre le public et l'administration",
            "crpa",
        ],
    },
    CodeDef {
        nom: "Code de l'environnement",
        aliases: &["code de l'environnement", "c. env."],
    },
    CodeDef {
        nom: "Code de la propriété intellectuelle",
        aliases: &["code de la propriete intellectuelle", "cpi"],
    },
    CodeDef {
        nom: "Code rural et de la pêche maritime",
        aliases: &[
            "code rural et de la peche maritime",
            "code rural",
            "c. rur.",
        ],
    },
    CodeDef {
        nom: "Code de l'action sociale et des familles",
        aliases: &["code de l'action sociale et des familles", "casf"],
    },
    CodeDef {
        nom: "Code de la santé publique",
        aliases: &["code de la sante publique", "c. sante publ.", "csp"],
    },
    CodeDef {
        nom: "Code de l'éducation",
        aliases: &["code de l'education", "c. educ."],
    },
    CodeDef {
        nom: "Code des assurances",
        aliases: &["code des assurances", "c. assur."],
    },
    CodeDef {
        nom: "Code général des collectivités territoriales",
        aliases: &["code general des collectivites territoriales", "cgct"],
    },
    CodeDef {
        nom: "Code de l'entrée et du séjour des étrangers et du droit d'asile",
        aliases: &[
            "code de l'entree et du sejour des etrangers et du droit d'asile",
            "ceseda",
        ],
    },
    CodeDef {
        nom: "Code de la route",
        aliases: &["code de la route"],
    },
    CodeDef {
        nom: "Code de la défense",
        aliases: &["code de la defense"],
    },
    CodeDef {
        nom: "Code des procédures civiles d'exécution",
        aliases: &["code des procedures civiles d'execution"],
    },
    CodeDef {
        nom: "Code de l'organisation judiciaire",
        aliases: &["code de l'organisation judiciaire", "coj"],
    },
    CodeDef {
        nom: "Code général de la fonction publique",
        aliases: &["code general de la fonction publique", "cgfp"],
    },
    CodeDef {
        nom: "Code des douanes",
        aliases: &["code des douanes"],
    },
    CodeDef {
        nom: "Code de la sécurité intérieure",
        aliases: &["code de la securite interieure", "csi"],
    },
    CodeDef {
        nom: "Code de l'énergie",
        aliases: &["code de l'energie"],
    },
    CodeDef {
        nom: "Code des transports",
        aliases: &["code des transports"],
    },
    CodeDef {
        nom: "Code du patrimoine",
        aliases: &["code du patrimoine"],
    },
    CodeDef {
        nom: "Code du sport",
        aliases: &["code du sport"],
    },
    CodeDef {
        nom: "Code du tourisme",
        aliases: &["code du tourisme"],
    },
    CodeDef {
        nom: "Code de la voirie routière",
        aliases: &["code de la voirie routiere"],
    },
    CodeDef {
        nom: "Code général de la propriété des personnes publiques",
        aliases: &[
            "code general de la propriete des personnes publiques",
            "cg3p",
        ],
    },
    CodeDef {
        nom: "Code de la recherche",
        aliases: &["code de la recherche"],
    },
    CodeDef {
        nom: "Code électoral",
        aliases: &["code electoral"],
    },
    CodeDef {
        nom: "Code de la justice pénale des mineurs",
        aliases: &["code de la justice penale des mineurs", "cjpm"],
    },
    CodeDef {
        nom: "Code des juridictions financières",
        aliases: &["code des juridictions financieres"],
    },
    CodeDef {
        nom: "Code de la mutualité",
        aliases: &["code de la mutualite"],
    },
    CodeDef {
        nom: "Code pénitentiaire",
        aliases: &["code penitentiaire"],
    },
    CodeDef {
        nom: "Code des postes et des communications électroniques",
        aliases: &[
            "code des postes et des communications electroniques",
            "cpce",
        ],
    },
    CodeDef {
        nom: "Code du cinéma et de l'image animée",
        aliases: &["code du cinema et de l'image animee"],
    },
];

/// Regex (source) d'un alias : `.` facultatif, espaces tolérants.
fn alias_pattern(alias: &str) -> String {
    let mut out = String::new();
    for c in alias.chars() {
        match c {
            '.' => out.push_str(r"\.?"),
            ' ' => out.push_str(r"\s*"),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    out
}

const CONNECTORS: &str =
    r"(?:du|de\s+la|de\s+l'|des|dans\s+le|dans\s+la|dans\s+l'|au|aux|le|la|l')\s*";

struct CompiledCode {
    nom: &'static str,
    /// Ancré en début de chaîne : « (du) Code civil » juste après le numéro.
    suffix: Regex,
    /// Ancré en fin de chaîne : « Code civil, art. » juste avant le numéro.
    head: Regex,
}

static COMPILED_CODES: Lazy<Vec<CompiledCode>> = Lazy::new(|| {
    CODES
        .iter()
        .map(|c| {
            // Alias les plus longs d'abord (alternation « leftmost-first »).
            let mut aliases: Vec<&str> = c.aliases.to_vec();
            aliases.sort_by_key(|a| std::cmp::Reverse(a.len()));
            let alts = aliases
                .iter()
                .map(|a| alias_pattern(a))
                .collect::<Vec<_>>()
                .join("|");
            CompiledCode {
                nom: c.nom,
                suffix: Regex::new(&format!(r"^\s*[,(]?\s*(?:{CONNECTORS})?(?:{alts})"))
                    .expect("regex suffix code"),
                head: Regex::new(&format!(
                    r"(?:^|[^a-z0-9])(?:{alts})\s*[,:(\-–—]?\s*(?:(?:articles?|arts?)\b\.?\s*)?$"
                ))
                .expect("regex head code"),
            }
        })
        .collect()
});

struct AnywhereCode {
    nom: &'static str,
    /// Alias entouré de frontières non alphanumériques, n'importe où dans le texte.
    re: Regex,
}

static ANYWHERE_CODES: Lazy<Vec<AnywhereCode>> = Lazy::new(|| {
    CODES
        .iter()
        .map(|c| {
            let mut aliases: Vec<&str> = c.aliases.to_vec();
            aliases.sort_by_key(|a| std::cmp::Reverse(a.len()));
            let alts = aliases
                .iter()
                .map(|a| alias_pattern(a))
                .collect::<Vec<_>>()
                .join("|");
            AnywhereCode {
                nom: c.nom,
                re: Regex::new(&format!(r"(?:^|[^a-z0-9])({alts})(?:$|[^a-z0-9])"))
                    .expect("regex code n'importe où"),
            }
        })
        .collect()
});

/// Premier code désigné n'importe où dans un texte **déjà passé par `fold`**
/// (« responsabilite du fait des choses code civil » → Code civil).
///
/// Retourne `(intitulé officiel, début, fin)` ; positions en octets dans le
/// texte replié, bornes de l'alias lui-même. En cas de plusieurs codes, le
/// premier cité l'emporte (à égalité de position, l'alias le plus long).
pub(crate) fn find_code_name(folded: &str) -> Option<(&'static str, usize, usize)> {
    ANYWHERE_CODES
        .iter()
        .filter_map(|c| {
            c.re.captures(folded)
                .and_then(|cap| cap.get(1))
                .map(|m| (c.nom, m.start(), m.end()))
        })
        .min_by_key(|&(_, s, e)| (s, std::cmp::Reverse(e - s)))
}

/// Après le numéro : le texte visé n'est PAS un code (loi, décret, Constitution,
/// convention, règlement…) → la référence est écartée, sans badge.
static RE_NON_CODE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"^\s*[,(]?\s*(?:{CONNECTORS}|d')?(?:loi|lois|decret|ordonnance|arrete|circulaire|constitution|convention|cedh|charte|reglement|directive|traite|tfue|tue|declaration|protocole|statuts?|contrat|accord|cgu|cgv|conditions|deliberation|instruction)\b"
    ))
    .expect("regex non-code")
});

/// Minuscules, sans accents, apostrophes et espaces normalisés.
pub(crate) fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => out.push('e'),
            'à' | 'â' | 'ä' | 'À' | 'Â' | 'Ä' => out.push('a'),
            'î' | 'ï' | 'Î' | 'Ï' => out.push('i'),
            'ô' | 'ö' | 'Ô' | 'Ö' => out.push('o'),
            'ù' | 'û' | 'ü' | 'Ù' | 'Û' | 'Ü' => out.push('u'),
            'ç' | 'Ç' => out.push('c'),
            'œ' | 'Œ' => out.push_str("oe"),
            'æ' | 'Æ' => out.push_str("ae"),
            '’' | '‘' | '`' | '´' => out.push('\''),
            '\u{a0}' | '\u{202f}' | '\u{2007}' => out.push(' '),
            c => out.extend(c.to_lowercase()),
        }
    }
    out
}

// ── Masquage du Markdown non prosaïque ────────────────────────────────────────

/// Remplace par des espaces (même longueur en octets → les positions restent
/// valides dans le texte d'origine) les zones où une référence ne doit pas être
/// annotée : blocs de code, code en ligne, liens/images Markdown, URL.
pub(crate) fn mask_non_prose(text: &str) -> String {
    static RE_LINK: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"!?\[[^\]\n]*\]\([^)\n]*\)").expect("regex lien"));
    static RE_URL: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"https?://[^\s<>)\]]+").expect("regex url"));

    fn blank(s: &str, out: &mut String) {
        for c in s.chars() {
            if c == '\n' {
                out.push('\n');
            } else {
                for _ in 0..c.len_utf8() {
                    out.push(' ');
                }
            }
        }
    }

    fn mask_line(line: &str, out: &mut String) {
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        // Code en ligne : suite de n backticks fermée par une suite de n backticks.
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'`' {
                let mut n = 0;
                while i + n < b.len() && b[i + n] == b'`' {
                    n += 1;
                }
                // Cherche la fermeture : une suite de exactement n backticks.
                let mut j = i + n;
                let mut close = None;
                while j < b.len() {
                    if b[j] == b'`' {
                        let mut m = 0;
                        while j + m < b.len() && b[j + m] == b'`' {
                            m += 1;
                        }
                        if m == n {
                            close = Some(j + m);
                            break;
                        }
                        j += m;
                    } else {
                        j += 1;
                    }
                }
                match close {
                    Some(end) => {
                        ranges.push((i, end));
                        i = end;
                    }
                    None => i += n,
                }
            } else {
                i += 1;
            }
        }
        for m in RE_LINK.find_iter(line) {
            ranges.push((m.start(), m.end()));
        }
        for m in RE_URL.find_iter(line) {
            ranges.push((m.start(), m.end()));
        }
        if ranges.is_empty() {
            out.push_str(line);
            return;
        }
        for (idx, c) in line.char_indices() {
            if ranges.iter().any(|&(s, e)| idx >= s && idx < e) && c != '\n' {
                for _ in 0..c.len_utf8() {
                    out.push(' ');
                }
            } else {
                out.push(c);
            }
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut fence: Option<&'static str> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let opens = if trimmed.starts_with("```") {
            Some("```")
        } else if trimmed.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        match fence {
            Some(marker) => {
                blank(line, &mut out);
                if trimmed.starts_with(marker) {
                    fence = None;
                }
            }
            None => match opens {
                Some(marker) => {
                    fence = Some(marker);
                    blank(line, &mut out);
                }
                None => mask_line(line, &mut out),
            },
        }
    }
    out
}

// ── Extraction ────────────────────────────────────────────────────────────────

/// Une référence d'article trouvée dans un texte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleRef {
    /// Début / fin dans le texte d'origine, en octets.
    pub start: usize,
    pub end: usize,
    /// Sous-chaîne exacte du texte d'origine (« L. 1234-5 », « article 1103 »).
    pub raw: String,
    /// Lettre de préfixe (L, R, D, A) pour les articles « L. 1234-5 ».
    pub letter: Option<char>,
    /// Partie numérique : « 1234-5 », « 1103 ».
    pub digits: String,
    /// Suffixe éventuel, en minuscules : « bis », « ter »…
    pub suffix: Option<String>,
    /// Intitulé officiel du code, si identifié.
    pub code: Option<&'static str>,
}

impl ArticleRef {
    /// Clé de comparaison : majuscules, sans espace ni point (« L1234-5BIS »).
    pub fn numero_key(&self) -> String {
        format!(
            "{}{}{}",
            self.letter.map(|c| c.to_string()).unwrap_or_default(),
            self.digits,
            self.suffix.clone().unwrap_or_default()
        )
        .to_uppercase()
    }

    /// Valeur envoyée à Légifrance dans le champ NUM_ARTICLE (« L1234-5 », « 199 ter »).
    pub fn numero_query(&self) -> String {
        let base = format!(
            "{}{}",
            self.letter.map(|c| c.to_string()).unwrap_or_default(),
            self.digits
        );
        match &self.suffix {
            Some(s) => format!("{base} {s}"),
            None => base,
        }
    }

    /// Numéro tel qu'affiché (« L. 1234-5 », « 1103 », « 199 ter »).
    pub fn numero_display(&self) -> String {
        let base = match self.letter {
            Some(l) => format!("{l}. {}", self.digits),
            None => self.digits.clone(),
        };
        match &self.suffix {
            Some(s) => format!("{base} {s}"),
            None => base,
        }
    }
}

const SUFFIXES: &str =
    "bis|ter|quater|quinquies|sexies|septies|octies|nonies|decies|undecies|duodecies|terdecies";

/// « L. 1234-5 », « R 4127-1 », « D.2 » … lettre majuscule obligatoire.
static RE_PREFIXED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"(?:^|[^\p{{L}}\p{{N}}])(?P<l>[LRDA])\s?\.?\s?(?P<d>\d{{1,4}}(?:-\d{{1,4}}){{0,4}})(?P<s>(?i:\s?(?:{SUFFIXES}))?)"
    ))
    .expect("regex préfixé")
});

/// « article 1103 », « art. 700 », « articles 1103 », « article 1er ».
static RE_PLAIN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"(?i)(?:^|[^\p{{L}}\p{{N}}])(?P<k>articles?|arts?)\b\.?\s*(?P<d>\d{{1,4}}(?:-\d{{1,4}}){{0,4}})(?P<e>er|ere|ère)?(?P<s>\s?(?:{SUFFIXES}))?"
    ))
    .expect("regex plain")
});

/// Suite d'une énumération : « , 1104 », « et 1105 », « à 1108 ».
static RE_CONT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(?:\s*,\s*(?:et|ou)\s+|\s*,\s*|\s+(?:et|ou|à|a)\s+)(?P<d>\d{1,4}(?:-\d{1,4}){0,4})(?P<e>er)?",
    )
    .expect("regex continuation")
});

/// Un nombre suivi d'une unité n'est pas un numéro d'article (« 1103 euros »).
static RE_UNIT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^\s*(?:€|%|eur\b|euros?\b|pour\s*cent|jours?\b|mois\b|ans?\b|annees?\b|heures?\b|fois\b|salaries?\b)",
    )
    .expect("regex unité")
});

/// Entre deux références d'une même énumération.
static RE_LIST_GAP: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(?:,\s*(?:et|ou)|,|et|ou|a|-|–)?\s*(?:(?:articles?|arts?)\b\.?\s*)?$")
        .expect("regex gap")
});

/// Le numéro se termine-t-il bien là (pas au milieu d'un numéro plus long) ?
fn boundary_ok(masked: &str, end: usize) -> bool {
    let mut it = masked[end..].chars();
    match it.next() {
        None => true,
        Some(c) if c.is_alphanumeric() => false,
        Some('-') | Some('/') | Some('.') => !matches!(it.next(), Some(n) if n.is_alphanumeric()),
        Some(_) => true,
    }
}

#[derive(Debug, Clone)]
struct RawRef {
    start: usize,
    end: usize,
    letter: Option<char>,
    digits: String,
    suffix: Option<String>,
}

/// Tronque `s` à `max` octets (sur une frontière de caractère).
fn take_prefix(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Idem pour la fin de `s`.
fn take_suffix(s: &str, max: usize) -> &str {
    let mut start = s.len().saturating_sub(max);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

enum CodeState {
    Known(&'static str),
    Unknown,
    NonCode,
}

fn resolve_code(masked: &str, r: &RawRef) -> CodeState {
    // 1. Suffixe : « … du Code civil ».
    let tail = fold(take_prefix(&masked[r.end..], 120));
    let tail = tail.split('\n').next().unwrap_or("");
    // « , alinéa 2, du Code… » / « al. 2 du Code… » : l'alinéa précise l'article,
    // il ne change pas le code visé → on le saute avant de chercher le code.
    static RE_ALINEA: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^\s*,?\s*(?:alinea|al\.?)\s*\d{1,2}(?:er|e|eme)?\b\.?\s*,?")
            .expect("regex alinéa")
    });
    let tail = match RE_ALINEA.find(tail) {
        Some(m) => &tail[m.end()..],
        None => tail,
    };
    for c in COMPILED_CODES.iter() {
        if let Some(m) = c.suffix.find(tail) {
            let next = tail[m.end()..].chars().next();
            if !matches!(next, Some(ch) if ch.is_alphanumeric()) {
                return CodeState::Known(c.nom);
            }
        }
    }
    if RE_NON_CODE.is_match(tail) {
        return CodeState::NonCode;
    }
    // 2. Préfixe : « Code civil, art. … ».
    let head = fold(take_suffix(&masked[..r.start], 100));
    let head = head.rsplit('\n').next().unwrap_or("");
    for c in COMPILED_CODES.iter() {
        if c.head.is_match(head) {
            return CodeState::Known(c.nom);
        }
    }
    CodeState::Unknown
}

/// Extrait les références d'articles de code d'un texte (Markdown ou brut).
///
/// Les blocs de code, le code en ligne, les liens et les URL sont ignorés. Les
/// références dont le texte visé n'est pas un code (loi, décret, Constitution,
/// convention…) sont écartées. Celles dont le code n'est pas identifié sont
/// renvoyées avec `code: None` (→ « non vérifiable »).
pub fn extract_article_refs(text: &str) -> Vec<ArticleRef> {
    let masked = mask_non_prose(text);
    debug_assert_eq!(masked.len(), text.len());
    let mut found: Vec<RawRef> = Vec::new();

    // ── « L. 1234-5 » ────────────────────────────────────────────────────────
    for c in RE_PREFIXED.captures_iter(&masked) {
        let whole = c.get(0).expect("match 0");
        let lead = c.name("l").expect("l").start();
        let digits = c.name("d").expect("d").as_str();
        // Au moins deux chiffres : écarte « L 5 », « D 1 »…
        if digits.chars().filter(|ch| ch.is_ascii_digit()).count() < 2 {
            continue;
        }
        let sfx = c.name("s").map(|m| m.as_str().trim().to_lowercase());
        let mut end = whole.end();
        let mut suffix = sfx.filter(|s| !s.is_empty());
        if !boundary_ok(&masked, end) {
            // Le suffixe (« bis ») était le début d'un mot : on le retire.
            let d_end = c.name("d").expect("d").end();
            if suffix.is_some() && boundary_ok(&masked, d_end) {
                end = d_end;
                suffix = None;
            } else {
                continue;
            }
        }
        found.push(RawRef {
            start: lead,
            end,
            letter: c.name("l").and_then(|m| m.as_str().chars().next()),
            digits: digits.to_string(),
            suffix,
        });
    }

    // ── « article 1103 », « articles 1103, 1104 et 1105 » ─────────────────────
    for c in RE_PLAIN.captures_iter(&masked) {
        let whole = c.get(0).expect("match 0");
        let d = c.name("d").expect("d");
        let digits = d.as_str().to_string();
        let mut end = whole.end();
        let mut suffix = c
            .name("s")
            .map(|m| m.as_str().trim().to_lowercase())
            .filter(|s| !s.is_empty());
        // « 1er » n'a de sens que pour l'article 1.
        if c.name("e").is_some() && digits != "1" {
            continue;
        }
        if !boundary_ok(&masked, end) {
            let base_end = c.name("e").map(|m| m.end()).unwrap_or(d.end());
            if suffix.is_some() && boundary_ok(&masked, base_end) {
                end = base_end;
                suffix = None;
            } else {
                continue;
            }
        }
        if RE_UNIT.is_match(&fold(take_prefix(&masked[end..], 16))) {
            continue;
        }
        let plural = {
            let k = c.name("k").expect("k").as_str().to_lowercase();
            k == "articles" || k == "arts"
        };
        let start = c.name("k").expect("k").start();
        found.push(RawRef {
            start,
            end,
            letter: None,
            digits,
            suffix,
        });

        // Énumération : « articles 1103, 1104 et 1105 » (pluriel seulement).
        if plural {
            let mut cursor = end;
            while let Some(cc) = RE_CONT.captures(&masked[cursor..]) {
                let dm = cc.name("d").expect("d");
                let digits = dm.as_str().to_string();
                let er = cc.name("e");
                if er.is_some() && digits != "1" {
                    break;
                }
                let n_start = cursor + dm.start();
                let n_end = cursor + er.map(|m| m.end()).unwrap_or(dm.end());
                if !boundary_ok(&masked, n_end)
                    || RE_UNIT.is_match(&fold(take_prefix(&masked[n_end..], 16)))
                {
                    break;
                }
                found.push(RawRef {
                    start: n_start,
                    end: n_end,
                    letter: None,
                    digits,
                    suffix: None,
                });
                cursor = n_end;
            }
        }
    }

    found.sort_by_key(|r| r.start);
    // Chevauchements (ne devrait pas arriver) : on garde la première.
    let mut dedup: Vec<RawRef> = Vec::with_capacity(found.len());
    for r in found {
        if dedup.last().map_or(true, |p| r.start >= p.end) {
            dedup.push(r);
        }
    }
    // Une référence dont l'extrait masqué diffère de l'original chevauche une
    // zone masquée (code en ligne…) : on l'ignore plutôt que d'annoter du code.
    dedup.retain(|r| masked[r.start..r.end] == text[r.start..r.end]);

    // ── Résolution du code ────────────────────────────────────────────────────
    let mut states: Vec<CodeState> = dedup.iter().map(|r| resolve_code(&masked, r)).collect();
    // Énumération partageant un code final : « L. 1233-3 et L. 1233-4 du Code du
    // travail » — chaque référence sans code hérite de la suivante si seul un
    // connecteur les sépare. Passe arrière pour propager sur toute la chaîne.
    for i in (0..dedup.len().saturating_sub(1)).rev() {
        if !matches!(states[i], CodeState::Unknown) {
            continue;
        }
        let gap = fold(&masked[dedup[i].end..dedup[i + 1].start]);
        if gap.len() <= 24 && RE_LIST_GAP.is_match(&gap) {
            states[i] = match &states[i + 1] {
                CodeState::Known(n) => CodeState::Known(n),
                CodeState::NonCode => CodeState::NonCode,
                CodeState::Unknown => CodeState::Unknown,
            };
        }
    }

    dedup
        .into_iter()
        .zip(states)
        .filter_map(|(r, st)| {
            let code = match st {
                CodeState::NonCode => return None,
                CodeState::Known(n) => Some(n),
                CodeState::Unknown => None,
            };
            Some(ArticleRef {
                raw: text[r.start..r.end].to_string(),
                start: r.start,
                end: r.end,
                letter: r.letter,
                digits: r.digits,
                suffix: r.suffix,
                code,
            })
        })
        .collect()
}

// ── Textes hors codes (loi, décret, ordonnance, arrêté…) ──────────────────────

const MONTHS: &str =
    "janvier|f[ée]vrier|mars|avril|mai|juin|juillet|ao[ûu]t|septembre|octobre|novembre|d[ée]cembre";

/// « décret n° 2020-1455 du 27 novembre 2020 », « arrêté du 8 décembre 2021 »,
/// « loi n° 78-17 ». Au moins un numéro OU une date est exigé (vérifié après
/// coup) : « la loi » ou « un décret » seuls ne désignent rien de précis.
static RE_TEXTE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"(?i)(?:^|[^\p{{L}}\p{{N}}])(?P<nat>loi\s+organique|loi\s+constitutionnelle|loi|ordonnance|d[ée]cret-loi|d[ée]cret|arr[êe]t[ée]|circulaire|instruction|d[ée]cision)(?:\s+(?:minist[ée]riel|interminist[ée]riel|pr[ée]fectoral|municipal|conjoint))?(?:\s*(?:n\s?[°º]|no\.?|num[ée]ro)\s*(?P<num>\d{{2,4}}\s?-\s?\d{{1,5}}))?(?:\s*,?\s*(?:en\s+date\s+du|du|pris\s+le)\s+(?P<d>1er|\d{{1,2}})\s+(?P<m>{MONTHS})\s+(?P<y>(?:18|19|20)\d{{2}}))?"
    ))
    .expect("regex texte")
});

/// Un texte juridique hors code cité par son numéro et/ou sa date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRef {
    pub start: usize,
    pub end: usize,
    pub raw: String,
    /// « loi », « ordonnance », « decret », « arrete », « circulaire », « instruction », « decision ».
    pub nature: &'static str,
    /// « 2020-1455 »
    pub numero: Option<String>,
    /// « AAAA-MM-JJ »
    pub date: Option<String>,
}

impl TextRef {
    /// Clés d'ancrage : par numéro et/ou par date (un texte est retrouvé si l'une suffit).
    fn keys(&self) -> Vec<String> {
        let mut k = Vec::new();
        if let Some(n) = &self.numero {
            k.push(format!("txt|{}|n:{n}", self.nature));
        }
        if let Some(d) = &self.date {
            k.push(format!("txt|{}|d:{d}", self.nature));
        }
        k
    }

    /// Clé de regroupement : le numéro s'il existe (plus discriminant), sinon la date.
    fn group_key(&self) -> String {
        self.keys().into_iter().next().unwrap_or_default()
    }
}

pub(crate) fn month_number(folded: &str) -> Option<u32> {
    Some(match folded {
        "janvier" => 1,
        "fevrier" => 2,
        "mars" => 3,
        "avril" => 4,
        "mai" => 5,
        "juin" => 6,
        "juillet" => 7,
        "aout" => 8,
        "septembre" => 9,
        "octobre" => 10,
        "novembre" => 11,
        "decembre" => 12,
        _ => return None,
    })
}

fn nature_key(folded: &str) -> &'static str {
    let f = folded.split_whitespace().collect::<Vec<_>>().join(" ");
    if f.starts_with("loi") {
        "loi" // organique / constitutionnelle : même série de numéros que les lois
    } else if f.starts_with("ordonnance") {
        "ordonnance"
    } else if f.starts_with("decret") {
        "decret"
    } else if f.starts_with("arrete") {
        "arrete"
    } else if f.starts_with("circulaire") {
        "circulaire"
    } else if f.starts_with("instruction") {
        "instruction"
    } else {
        "decision"
    }
}

/// Extrait les textes hors codes cités par numéro et/ou date. Mêmes exclusions
/// que `extract_article_refs` (blocs de code, code en ligne, liens, URL).
pub fn extract_text_refs(text: &str) -> Vec<TextRef> {
    let masked = mask_non_prose(text);
    let mut out = Vec::new();
    for c in RE_TEXTE.captures_iter(&masked) {
        let nat = c.name("nat").expect("nat");
        let numero = c.name("num").map(|m| {
            m.as_str()
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
        });
        let date = match (c.name("d"), c.name("m"), c.name("y")) {
            (Some(d), Some(m), Some(y)) => {
                let day: u32 = if d.as_str().eq_ignore_ascii_case("1er") {
                    1
                } else {
                    d.as_str().parse().unwrap_or(0)
                };
                match month_number(&fold(m.as_str())) {
                    Some(mo) if (1..=31).contains(&day) => {
                        Some(format!("{}-{:02}-{:02}", y.as_str(), mo, day))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if numero.is_none() && date.is_none() {
            continue;
        }
        let whole = c.get(0).expect("match 0");
        let (start, end) = (nat.start(), whole.end());
        if masked[start..end] != text[start..end] {
            continue; // chevauche une zone masquée
        }
        out.push(TextRef {
            start,
            end,
            raw: text[start..end].to_string(),
            nature: nature_key(&fold(nat.as_str())),
            numero,
            date,
        });
    }
    out
}

// ── Ancrage : ce que le modèle a reçu ─────────────────────────────────────────

/// Index des références présentes dans les sources fournies au modèle.
///
/// Sert aussi de format d'échange : le backend de chat en tire des *clés* compactes
/// (`grounding_keys`) à partir des résultats d'outils MCP et des extraits RAG, que
/// le frontend ramène ensuite ici — sans renvoyer les textes eux-mêmes.
#[derive(Debug, Default, Clone)]
pub struct GroundingIndex {
    keys: std::collections::HashSet<String>,
}

/// Plafonds sur les clés fournies par un client (l'API est locale, mais on ne
/// fait pas confiance à une entrée non bornée).
const MAX_EXTERNAL_KEYS: usize = 50_000;
const MAX_KEY_LEN: usize = 200;

impl GroundingIndex {
    /// Index de toutes les références trouvées dans `texts`.
    pub fn from_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> Self {
        let mut idx = Self::default();
        for t in texts {
            idx.keys.extend(grounding_keys(t));
        }
        idx
    }

    /// Ajoute des clés déjà calculées (`grounding_keys`) ; les clés mal formées sont ignorées.
    pub fn extend_keys<I: IntoIterator<Item = String>>(&mut self, keys: I) {
        for k in keys.into_iter().take(MAX_EXTERNAL_KEYS) {
            if k.len() <= MAX_KEY_LEN
                && (k.starts_with("art|")
                    || k.starts_with("txt|")
                    || k.starts_with("dec|")
                    || k.starts_with("idcc|"))
            {
                self.keys.insert(k);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Article `numero_key` du code `code`. Une source qui cite l'article sans
    /// nommer le code (`art|?|…`, cas d'un document « Article 1103 » seul) compte :
    /// on préfère manquer un « hors sources » que d'en signaler un à tort.
    fn has_article(&self, code: &str, numero_key: &str) -> bool {
        self.keys.contains(&format!("art|{code}|{numero_key}"))
            || self.keys.contains(&format!("art|?|{numero_key}"))
    }

    fn has_texte(&self, r: &TextRef) -> bool {
        r.keys().iter().any(|k| self.keys.contains(k))
    }

    /// Décision de la Cour de cassation (clé = chiffres du numéro de pourvoi).
    pub(crate) fn has_decision(&self, key: &str) -> bool {
        self.keys.contains(&format!("dec|{key}"))
    }

    /// Convention collective (IDCC sur 4 chiffres).
    pub(crate) fn has_idcc(&self, idcc: &str) -> bool {
        self.keys.contains(&format!("idcc|{idcc}"))
    }
}

/// Clés d'ancrage de tout ce qu'un texte cite (articles de codes, textes hors codes).
/// Triées et sans doublon. Format : `art|<code ou ?>|<numéro>` et `txt|<nature>|n:<num>` / `d:<date>`.
pub fn grounding_keys(text: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for r in extract_article_refs(text) {
        keys.push(format!("art|{}|{}", r.code.unwrap_or("?"), r.numero_key()));
    }
    for t in extract_text_refs(text) {
        keys.extend(t.keys());
    }
    for d in super::verify_ext::extract_decision_refs(text) {
        keys.push(format!("dec|{}", d.key));
    }
    for i in super::verify_ext::extract_idcc_refs(text) {
        keys.push(format!("idcc|{}", i.idcc));
    }
    keys.sort();
    keys.dedup();
    keys
}

// ── Résultat d'une recherche d'article ────────────────────────────────────────

/// Ce que Légifrance sait d'un article (code + numéro).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Lookup {
    /// Une version en vigueur existe.
    EnVigueur { id: String },
    /// L'article existe mais aucune version n'est en vigueur (abrogé, périmé,
    /// transféré, entrée en vigueur différée…).
    Autre {
        id: String,
        etat: Option<String>,
        /// Fin de validité de la version retenue, « AAAA-MM-JJ ».
        date_fin: Option<String>,
    },
    /// Recherche aboutie, aucun article de ce numéro dans ce code.
    /// `ailleurs` : codes où ce numéro existe (utile pour signaler une
    /// confusion de code : « L. 1234-5 » existe dans un autre code).
    Introuvable { ailleurs: Vec<String> },
}

/// Source de vérité sur les articles de code. Le trait isole la logique de
/// vérification de l'API réseau : les tests injectent un faux, la production
/// utilise `LegifranceLookup` (éventuellement enveloppé de `CachedLookup`).
///
/// `numero` est la valeur de recherche (« L1234-5 », « 199 ter »).
/// `Err` = la vérification n'a pas pu aboutir (réseau, API) — jamais un verdict.
pub trait ArticleLookup: Send + Sync {
    fn lookup<'a>(
        &'a self,
        code: &'a str,
        numero: &'a str,
    ) -> BoxFuture<'a, Result<Lookup, String>>;
}

// ── Rapport ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Statut {
    /// ✅ L'article existe et une version est en vigueur.
    EnVigueur,
    /// ⚠️ L'article existe mais n'est (ou n'est plus) pas en vigueur — voir
    /// `etat` / `etat_libelle` / `date_fin` pour la nuance exacte.
    Abroge,
    /// ❌ Aucun article de ce numéro dans ce code (recherche aboutie).
    Introuvable,
    /// Pas de verdict : code non identifié, ou limite de vérifications atteinte.
    NonVerifiable,
    /// Pas de verdict : Légifrance n'a pas répondu correctement.
    Erreur,
    /// ✅ La référence existe (décision, convention collective) ou la citation
    /// textuelle est retrouvée telle quelle dans la source.
    Verifiee,
    /// ⚠️ La source existe mais ce qui est cité ne concorde pas (date ou chambre
    /// d'une décision, date d'un texte, passage cité absent de l'article).
    Incoherente,
    /// ⚠️ Citation textuelle proche de la source sans lui être identique.
    Approchante,
    /// ❓ Recherche aboutie sans correspondance. Ce n'est pas une preuve
    /// d'inexistence (Judilibre n'est pas exhaustif) : à vérifier à la main.
    NonRetrouvee,
}

/// Position d'une occurrence dans le texte analysé. Offsets en unités UTF-16
/// (ceux de `String.prototype.slice` en JavaScript), `texte` sert de garde-fou
/// côté frontend (si le texte a changé entre-temps, l'annotation est ignorée).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Occurrence {
    pub debut: usize,
    pub fin: usize,
    pub texte: String,
}

/// Nature de la référence vérifiée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Article de code : existence et état vérifiés sur Légifrance.
    Article,
    /// Texte hors code (loi, ordonnance, décret) : existence et état vérifiés
    /// quand il est cité par son numéro ; sinon seul l'ancrage est évalué.
    Texte,
    /// Décision de la Cour de cassation citée par son numéro de pourvoi (Judilibre).
    Decision,
    /// Convention collective citée par son IDCC (Légifrance, fonds KALI).
    Convention,
    /// Passage cité entre guillemets, comparé au texte de l'article ou aux sources.
    Citation,
}

/// Présence de la référence dans ce qui a été fourni au modèle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ancrage {
    /// Retrouvée dans les documents, résultats d'outils ou extraits fournis.
    DansSources,
    /// Absente de tout ce qui a été fourni : vient de la mémoire du modèle.
    HorsSources,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verification {
    pub kind: Kind,
    /// `None` si l'ancrage n'a pas été demandé (ou pas de sources fournies).
    pub ancrage: Option<Ancrage>,
    /// Libellé lisible : « Code civil, art. 1103 ».
    pub reference: String,
    pub code: Option<String>,
    pub numero: String,
    pub statut: Statut,
    /// `Some(true/false)` quand la recherche a abouti, `None` sinon.
    pub existe: Option<bool>,
    /// État juridique brut Légifrance (« VIGUEUR », « ABROGE »…).
    pub etat: Option<String>,
    pub etat_libelle: Option<String>,
    /// Fin de validité (« AAAA-MM-JJ ») quand l'article n'est plus en vigueur.
    pub date_fin: Option<String>,
    pub url: Option<String>,
    /// Introuvable : codes dans lesquels ce numéro existe.
    pub existe_dans: Vec<String>,
    /// Explication courte pour les statuts sans verdict.
    pub detail: Option<String>,
    /// Éléments d'identité de la source retrouvée (titre, chambre, date, solution…),
    /// affichés dans l'info-bulle.
    pub infos: Vec<String>,
    /// Décision retrouvée : son identifiant (Judilibre ou Légifrance), qui donne accès à sa
    /// fiche (`GET /decision/:id/fiche`) : rapprochements et provenance.
    #[serde(default)]
    pub decision_id: Option<String>,
    pub occurrences: Vec<Occurrence>,
    /// Données de travail, jamais sérialisées (voir `verify_ext`).
    #[serde(skip)]
    pub intern: Intern,
}

/// Données de travail d'une vérification, non exposées au client.
#[derive(Debug, Clone, Default)]
pub struct Intern {
    /// `code|numéro` d'un article de code identifié.
    pub article_key: Option<String>,
    /// Identifiant LEGIARTI de la version retenue.
    pub article_id: Option<String>,
    /// Texte hors code cité par numéro : (nature, numéro, date « AAAA-MM-JJ »).
    pub texte: Option<(String, String, Option<String>)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Jour de la vérification (les états juridiques changent dans le temps).
    pub date: String,
    pub verifications: Vec<Verification>,
}

/// Libellé français d'un état juridique Légifrance.
pub fn etat_libelle(etat: &str) -> String {
    match etat.to_uppercase().as_str() {
        "VIGUEUR" => "en vigueur".into(),
        "VIGUEUR_DIFF" => "entrée en vigueur différée".into(),
        "ABROGE" => "abrogé".into(),
        "ABROGE_DIFF" => "abrogation différée".into(),
        "PERIME" => "périmé".into(),
        "ANNULE" => "annulé".into(),
        "MODIFIE" => "modifié (version remplacée)".into(),
        "TRANSFERE" => "transféré".into(),
        "DISJOINT" => "disjoint".into(),
        "SUBSTITUE" => "substitué".into(),
        "REMPLACE" => "remplacé".into(),
        "DENONCE" => "dénoncé".into(),
        other => other.to_lowercase().replace('_', " "),
    }
}

/// URL publique Légifrance d'un article de code.
pub fn article_url(id: &str) -> Option<String> {
    id.starts_with("LEGIARTI")
        .then(|| format!("https://www.legifrance.gouv.fr/codes/article_lc/{id}"))
}

pub(crate) fn utf16_offset(text: &str, byte_idx: usize) -> usize {
    text[..byte_idx].encode_utf16().count()
}

fn base_verification(code: Option<&str>, r: &ArticleRef) -> Verification {
    let reference = match code {
        Some(c) => format!("{c}, art. {}", r.numero_display()),
        None => format!("art. {}", r.numero_display()),
    };
    Verification {
        kind: Kind::Article,
        ancrage: None,
        reference,
        code: code.map(str::to_string),
        numero: r.numero_display(),
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
        intern: Intern {
            article_key: code.map(|c| format!("{c}|{}", r.numero_key())),
            ..Intern::default()
        },
    }
}

/// Vérifie les références extraites de `text`.
///
/// Les références identiques (même code, même numéro) sont regroupées : une
/// seule requête, plusieurs occurrences. Les appels sont parallèles, bornés à
/// `LOOKUP_CONCURRENCY`, et plafonnés à `MAX_DISTINCT_LOOKUPS`.
#[cfg(test)]
pub async fn verify_all(
    lookup: &dyn ArticleLookup,
    text: &str,
    refs: &[ArticleRef],
    today: &str,
) -> Report {
    build_report(lookup, text, refs, &[], None, today).await
}

/// Rapport complet : articles de codes (existence + ancrage) puis textes hors
/// codes (ancrage seul). `grounding: None` ⇒ aucun ancrage calculé.
pub async fn build_report(
    lookup: &dyn ArticleLookup,
    text: &str,
    refs: &[ArticleRef],
    texts: &[TextRef],
    grounding: Option<&GroundingIndex>,
    today: &str,
) -> Report {
    // Regroupement par (code, numéro), dans l'ordre de première apparition.
    let mut order: Vec<(Option<&'static str>, String)> = Vec::new();
    let mut groups: HashMap<(Option<&'static str>, String), Verification> = HashMap::new();
    let mut queries: HashMap<(Option<&'static str>, String), String> = HashMap::new();

    for r in refs.iter().take(MAX_OCCURRENCES) {
        let key = (r.code, r.numero_key());
        let entry = groups.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            queries.insert(key.clone(), r.numero_query());
            base_verification(r.code, r)
        });
        entry.occurrences.push(Occurrence {
            debut: utf16_offset(text, r.start),
            fin: utf16_offset(text, r.end),
            texte: r.raw.clone(),
        });
    }

    // Références à interroger : code connu, dans la limite du plafond.
    let mut to_check: Vec<(usize, String, String)> = Vec::new();
    for (i, key) in order.iter().enumerate() {
        let Some(code) = key.0 else {
            groups.get_mut(key).expect("groupe").detail =
                Some("Code non précisé : vérification impossible".into());
            continue;
        };
        if to_check.len() >= MAX_DISTINCT_LOOKUPS {
            groups.get_mut(key).expect("groupe").detail =
                Some("Limite du nombre de vérifications par réponse atteinte".into());
            continue;
        }
        to_check.push((i, code.to_string(), queries[key].clone()));
    }

    let mut outcomes: Vec<(usize, Result<Lookup, String>)> = stream::iter(to_check)
        .map(|(i, code, numero)| async move {
            let res = lookup.lookup(&code, &numero).await;
            (i, res)
        })
        .buffer_unordered(LOOKUP_CONCURRENCY)
        .collect()
        .await;
    outcomes.sort_by_key(|(i, _)| *i);

    for (i, res) in outcomes {
        let v = groups.get_mut(&order[i]).expect("groupe");
        match res {
            Ok(Lookup::EnVigueur { id }) => {
                v.statut = Statut::EnVigueur;
                v.existe = Some(true);
                v.etat = Some("VIGUEUR".into());
                v.etat_libelle = Some(etat_libelle("VIGUEUR"));
                v.url = article_url(&id);
                v.intern.article_id = Some(id);
            }
            Ok(Lookup::Autre { id, etat, date_fin }) => {
                v.statut = Statut::Abroge;
                v.existe = Some(true);
                v.etat_libelle = etat.as_deref().map(etat_libelle);
                v.etat = etat;
                v.date_fin = date_fin;
                v.url = article_url(&id);
                v.intern.article_id = Some(id);
            }
            Ok(Lookup::Introuvable { ailleurs }) => {
                v.statut = Statut::Introuvable;
                v.existe = Some(false);
                v.existe_dans = ailleurs;
            }
            Err(e) => {
                // Détail technique dans les logs, message générique au client.
                warn!("[Vérification] {} : {}", v.reference, e);
                v.statut = Statut::Erreur;
                v.detail = Some("Légifrance n'a pas pu être interrogé".into());
            }
        }
    }

    // Ancrage des articles (code connu seulement : sans code, pas de comparaison fiable).
    if let Some(g) = grounding {
        for key in &order {
            if let Some(code) = key.0 {
                let v = groups.get_mut(key).expect("groupe");
                v.ancrage = Some(if g.has_article(code, &key.1) {
                    Ancrage::DansSources
                } else {
                    Ancrage::HorsSources
                });
            }
        }
    }

    let mut verifications: Vec<Verification> = order
        .iter()
        .map(|k| groups.remove(k).expect("groupe"))
        .collect();

    // Textes hors codes : regroupés par numéro (ou date), ancrage seul.
    let mut t_order: Vec<String> = Vec::new();
    let mut t_groups: HashMap<String, Verification> = HashMap::new();
    let room = MAX_OCCURRENCES.saturating_sub(refs.len().min(MAX_OCCURRENCES));
    for t in texts.iter().take(room) {
        let key = format!("{}|{}", t.nature, t.group_key());
        let entry = t_groups.entry(key.clone()).or_insert_with(|| {
            t_order.push(key.clone());
            Verification {
                kind: Kind::Texte,
                ancrage: grounding.map(|g| {
                    if g.has_texte(t) {
                        Ancrage::DansSources
                    } else {
                        Ancrage::HorsSources
                    }
                }),
                reference: t.raw.clone(),
                code: None,
                numero: t.raw.clone(),
                statut: Statut::NonVerifiable,
                existe: None,
                etat: None,
                etat_libelle: None,
                date_fin: None,
                url: None,
                existe_dans: Vec::new(),
                detail: Some("Existence non vérifiée automatiquement (texte hors code)".into()),
                infos: Vec::new(),
                decision_id: None,
                occurrences: Vec::new(),
                intern: Intern {
                    texte: t
                        .numero
                        .clone()
                        .map(|n| (t.nature.to_string(), n, t.date.clone())),
                    ..Intern::default()
                },
            }
        });
        entry.occurrences.push(Occurrence {
            debut: utf16_offset(text, t.start),
            fin: utf16_offset(text, t.end),
            texte: t.raw.clone(),
        });
    }
    verifications.extend(
        t_order
            .iter()
            .map(|k| t_groups.remove(k).expect("groupe texte")),
    );
    Report {
        date: today.to_string(),
        verifications,
    }
}

/// Source « hors service » : utilisée quand aucune référence n'est vérifiable
/// (aucun appel n'est alors émis) pour ne pas exiger de client PISTE.
pub struct NoLookup;

impl ArticleLookup for NoLookup {
    fn lookup<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<Lookup, String>> {
        Box::pin(async { Err("aucune source configurée".to_string()) })
    }
}

// ── Cache ─────────────────────────────────────────────────────────────────────

/// Met en cache les réponses *abouties* (jamais les erreurs) par
/// (code, numéro, jour), dans le cache partagé du module juridique.
pub struct CachedLookup<L: ArticleLookup> {
    inner: L,
    cache: Cache,
    day: String,
}

impl<L: ArticleLookup> CachedLookup<L> {
    pub fn new(inner: L, cache: Cache, day: String) -> Self {
        Self { inner, cache, day }
    }
}

impl<L: ArticleLookup> ArticleLookup for CachedLookup<L> {
    fn lookup<'a>(
        &'a self,
        code: &'a str,
        numero: &'a str,
    ) -> BoxFuture<'a, Result<Lookup, String>> {
        Box::pin(async move {
            let key = format!("verif:{}:{}:{}", self.day, code, norm_num_key(numero));
            if let Some(v) = cache_get(&self.cache, &key, TTL_VERIFICATION).await {
                if let Ok(hit) = serde_json::from_value::<Lookup>(v) {
                    return Ok(hit);
                }
            }
            let res = self.inner.lookup(code, numero).await;
            if let Ok(ok) = &res {
                if let Ok(v) = serde_json::to_value(ok) {
                    cache_set(&self.cache, key, v).await;
                }
            }
            res
        })
    }
}

// ── Analyse des réponses Légifrance ───────────────────────────────────────────

pub(crate) fn strip_tags(s: &str) -> String {
    static RE_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]*>").expect("regex tag"));
    RE_TAG.replace_all(s, "").trim().to_string()
}

/// Clé de comparaison d'un numéro d'article : sans balises `<mark>`, sans
/// « Article », sans espace / point / astérisque, en majuscules.
pub(crate) fn norm_num_key(s: &str) -> String {
    let t = strip_tags(s);
    let upper = t.to_uppercase();
    let t = upper.strip_prefix("ARTICLE").unwrap_or(&upper);
    t.chars()
        .filter(|c| !c.is_whitespace() && *c != '.' && *c != '*')
        .collect()
}

/// L'intitulé de code renvoyé par Légifrance correspond-il au code demandé ?
pub(crate) fn code_matches(titre: &str, code: &str) -> bool {
    let (t, c) = (fold(&strip_tags(titre)), fold(code));
    t == c
        || t.strip_prefix(&c)
            .is_some_and(|rest| !rest.chars().next().is_some_and(|ch| ch.is_alphanumeric()))
}

/// Un article correspondant trouvé dans une réponse `/search`.
#[derive(Debug, Clone)]
pub(crate) struct Match {
    pub id: String,
    pub etat: Option<String>,
    pub date_version: Option<i64>,
    pub code_titre: Option<String>,
}

impl Match {
    fn is_vigueur(&self) -> bool {
        self.etat.as_deref().map(str::to_uppercase).as_deref() == Some("VIGUEUR")
    }
}

fn as_str(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn as_millis(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Parcourt `results[].sections[].extracts[]` (et les résultats « article »
/// de premier niveau) et garde les articles dont le numéro vaut `wanted_key`.
///
/// `implied_etat` : état à attribuer quand l'API n'en précise pas (cas d'une
/// recherche déjà filtrée sur un état).
pub(crate) fn collect_matches(
    resp: &Value,
    wanted_key: &str,
    implied_etat: Option<&str>,
) -> Vec<Match> {
    let mut out = Vec::new();
    let results = resp.get("results").and_then(Value::as_array);
    for res in results.into_iter().flatten() {
        let code_titre = res
            .get("titles")
            .and_then(|t| t.get(0))
            .and_then(|t| t.get("title"))
            .and_then(as_str)
            .or_else(|| res.get("title").and_then(as_str))
            .map(|t| strip_tags(&t));
        let res_etat = res
            .get("legalStatus")
            .or_else(|| res.get("etat"))
            .and_then(as_str);

        let mut push = |node: &Value, fallback_etat: &Option<String>| {
            let num = node
                .get("num")
                .and_then(as_str)
                .or_else(|| node.get("title").and_then(as_str));
            let Some(num) = num else { return };
            if norm_num_key(&num) != wanted_key {
                return;
            }
            let Some(id) = node.get("id").and_then(as_str) else {
                return;
            };
            let etat = node
                .get("legalStatus")
                .or_else(|| node.get("etat"))
                .and_then(as_str)
                .or_else(|| fallback_etat.clone())
                .or_else(|| implied_etat.map(str::to_string));
            out.push(Match {
                id,
                etat,
                date_version: node.get("dateVersion").and_then(as_millis),
                code_titre: code_titre.clone(),
            });
        };

        for section in res
            .get("sections")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for ex in section
                .get("extracts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                push(ex, &res_etat);
            }
        }
        // Résultat « article » de premier niveau (fonds paginés à l'article).
        if res
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|i| i.starts_with("LEGIARTI"))
        {
            push(res, &res_etat);
        }
    }
    out
}

/// Millisecondes epoch → « AAAA-MM-JJ ». Les dates sentinelles de « pas de fin »
/// (2999-01-01) donnent `None`.
///
/// Les dates Légifrance sont des minuits (UTC ou Paris selon les fonds) : on
/// ajoute 12 h avant de tronquer au jour, ce qui donne le bon jour calendaire
/// dans les deux cas (sans cela, « minuit Paris » = 22 h/23 h UTC de la veille).
pub(crate) fn millis_to_date(ms: i64) -> Option<String> {
    use chrono::{Datelike, TimeZone, Utc};
    let dt = Utc
        .timestamp_millis_opt(ms.checked_add(12 * 3600 * 1000)?)
        .single()?;
    if dt.year() >= 2999 {
        return None;
    }
    Some(dt.format("%Y-%m-%d").to_string())
}

/// `(état, date de fin)` d'une réponse `/consult/getArticle`.
pub(crate) fn article_info(resp: &Value) -> (Option<String>, Option<String>) {
    let a = resp.get("article").unwrap_or(resp);
    let etat = a.get("etat").and_then(as_str).filter(|e| !e.is_empty());
    let date_fin = a
        .get("dateFin")
        .and_then(as_millis)
        .and_then(millis_to_date);
    (etat, date_fin)
}

/// Meilleur candidat : en vigueur d'abord, puis « vigueur différée », puis la
/// version la plus récente.
fn best(matches: &[Match]) -> Option<&Match> {
    fn rank(m: &Match) -> u8 {
        match m.etat.as_deref().map(str::to_uppercase).as_deref() {
            Some("VIGUEUR") => 0,
            Some("VIGUEUR_DIFF") => 1,
            _ => 2,
        }
    }
    matches.iter().min_by_key(|m| {
        (
            rank(m),
            std::cmp::Reverse(m.date_version.unwrap_or(i64::MIN)),
        )
    })
}

// ── Adaptateur Légifrance ─────────────────────────────────────────────────────

/// États juridiques autres que « en vigueur » (facette TEXT_LEGAL_STATUS).
const ETATS_NON_VIGUEUR: &[&str] = &[
    "ABROGE",
    "ABROGE_DIFF",
    "VIGUEUR_DIFF",
    "PERIME",
    "ANNULE",
    "MODIFIE",
    "TRANSFERE",
    "SUBSTITUE",
    "REMPLACE",
    "DISJOINT",
];

pub struct LegifranceLookup {
    client: PisteClient,
}

impl LegifranceLookup {
    pub fn new(client: PisteClient) -> Self {
        Self { client }
    }

    /// Recherche CODE_ETAT sur le champ NUM_ARTICLE, filtrée (ou non) par code
    /// et par état juridique.
    async fn search(
        &self,
        code: Option<&str>,
        etats: Option<Vec<String>>,
        numero: &str,
    ) -> Result<Value, String> {
        let extra = code.map(|c| {
            vec![FiltreValeurs {
                // « NOM_CODE » est traduit en TEXT_NOM_CODE par SearchParams::to_api_body.
                facette: "NOM_CODE".into(),
                valeurs: vec![c.to_string()],
            }]
        });
        let resp = legifrance::search_simple(
            &self.client,
            numero,
            SearchFond::CodeEtat,
            1,
            50,
            None,
            None,
            None,
            None,
            Some("NUM_ARTICLE".into()),
            etats,
            extra,
        )
        .await
        .map_err(|e| e.to_string())?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    }

    /// Transforme des candidats en résultat (en vigueur, ou autre état + date de fin).
    async fn conclude(&self, candidates: &[Match]) -> Option<Lookup> {
        let m = best(candidates)?;
        if m.is_vigueur() {
            return Some(Lookup::EnVigueur { id: m.id.clone() });
        }
        // Pas en vigueur : l'article lui-même donne l'état exact et la date de fin.
        let (mut etat, mut date_fin) = (m.etat.clone(), None);
        match legifrance::get_article(&self.client, &m.id).await {
            Ok(v) => {
                let (e, d) = article_info(&v);
                etat = e.or(etat);
                date_fin = d;
            }
            // Non bloquant : on garde ce que la recherche a donné.
            Err(e) => warn!("[Vérification] getArticle {} : {}", m.id, e),
        }
        Some(Lookup::Autre {
            id: m.id.clone(),
            etat,
            date_fin,
        })
    }

    async fn lookup_impl(&self, code: &str, numero: &str) -> Result<Lookup, String> {
        let wanted = norm_num_key(numero);
        let only_code = |ms: Vec<Match>| -> Vec<Match> {
            ms.into_iter()
                .filter(|m| {
                    m.code_titre
                        .as_deref()
                        .map_or(true, |t| code_matches(t, code))
                })
                .collect()
        };

        // A. Version en vigueur dans ce code (cas nominal : un seul appel).
        let resp = self
            .search(Some(code), Some(vec!["VIGUEUR".into()]), numero)
            .await?;
        let mut candidates = only_code(collect_matches(&resp, &wanted, Some("VIGUEUR")));
        if let Some(m) = candidates.iter().find(|m| m.is_vigueur()) {
            return Ok(Lookup::EnVigueur { id: m.id.clone() });
        }

        // B. Autres états dans ce code (abrogé, périmé, transféré…).
        let others: Vec<String> = ETATS_NON_VIGUEUR.iter().map(|s| s.to_string()).collect();
        let resp = self.search(Some(code), Some(others), numero).await?;
        candidates.extend(only_code(collect_matches(&resp, &wanted, None)));
        if let Some(l) = self.conclude(&candidates).await {
            return Ok(l);
        }

        // C. Sans filtre de code ni d'état : garde-fou contre un filtre de code
        //    mal interprété par l'API (on ne conclut « introuvable » qu'après
        //    avoir regardé partout) et source de « existe dans un autre code ».
        let resp = self.search(None, None, numero).await?;
        let all = collect_matches(&resp, &wanted, None);
        let same_code = only_code(all.clone());
        if let Some(l) = self.conclude(&same_code).await {
            return Ok(l);
        }
        let mut ailleurs: Vec<String> = all.into_iter().filter_map(|m| m.code_titre).collect();
        ailleurs.sort();
        ailleurs.dedup();
        ailleurs.truncate(5);
        Ok(Lookup::Introuvable { ailleurs })
    }
}

impl ArticleLookup for LegifranceLookup {
    fn lookup<'a>(
        &'a self,
        code: &'a str,
        numero: &'a str,
    ) -> BoxFuture<'a, Result<Lookup, String>> {
        Box::pin(self.lookup_impl(code, numero))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::{
        extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router,
    };
    use serde_json::json;

    use super::super::routes::{
        new_cache, verifier_citations_route, HasJuridiqueState, JuridiqueState, MAX_SOURCES_BYTES,
    };

    // ── Aides ─────────────────────────────────────────────────────────────────

    fn refs(text: &str) -> Vec<ArticleRef> {
        let r = extract_article_refs(text);
        // Invariant : les positions pointent bien sur `raw` dans le texte d'origine.
        for a in &r {
            assert_eq!(
                &text[a.start..a.end],
                a.raw,
                "position incohérente pour {a:?}"
            );
        }
        r
    }

    /// (clé de numéro, code) de chaque référence.
    fn keys(text: &str) -> Vec<(String, Option<&'static str>)> {
        refs(text)
            .iter()
            .map(|r| (r.numero_key(), r.code))
            .collect()
    }

    // ── Extraction : références avec code ─────────────────────────────────────

    #[test]
    fn prefixed_article_with_code_suffix() {
        let t = "Selon l'article L. 1234-5 du Code du travail, le préavis s'applique.";
        let r = refs(t);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].raw, "L. 1234-5");
        assert_eq!(r[0].numero_key(), "L1234-5");
        assert_eq!(r[0].numero_query(), "L1234-5");
        assert_eq!(r[0].numero_display(), "L. 1234-5");
        assert_eq!(r[0].code, Some("Code du travail"));
    }

    #[test]
    fn prefixed_spacing_variants_are_equivalent() {
        for variante in [
            "L. 1234-5",
            "L.1234-5",
            "L 1234-5",
            "L1234-5",
            "L.\u{a0}1234-5",
            "L.\u{202f}1234-5",
        ] {
            let t = format!("L'article {variante} du Code du travail prévoit…");
            assert_eq!(
                keys(&t),
                vec![("L1234-5".to_string(), Some("Code du travail"))],
                "variante {variante:?}"
            );
        }
    }

    #[test]
    fn plain_article_with_various_code_notations() {
        let cas = [
            ("l'article 1103 du Code civil", "Code civil"),
            ("l'article 1103 du code civil", "Code civil"),
            ("art. 1103 C. civ.", "Code civil"),
            ("art. 1103 C.civ.", "Code civil"),
            ("article 700 du CPC", "Code de procédure civile"),
            (
                "article 700 du Code de procédure civile",
                "Code de procédure civile",
            ),
            (
                "article 700 du Code de procedure civile",
                "Code de procédure civile",
            ),
            ("article 1240 du Code civil", "Code civil"),
            ("article 121-3 du Code pénal", "Code pénal"),
            ("article 145 du CGI", "Code général des impôts"),
            ("article L. 1234-5 c. trav.", "Code du travail"),
            ("article R. 4127-1 du CSP", "Code de la santé publique"),
            (
                "Article L. 121-1 du code de la consommation",
                "Code de la consommation",
            ),
        ];
        for (texte, code) in cas {
            let r = refs(texte);
            assert_eq!(r.len(), 1, "{texte}");
            assert_eq!(r[0].code, Some(code), "{texte}");
        }
    }

    #[test]
    fn article_premier_and_suffixed_numbers() {
        let r = refs("l'article 1er du Code civil");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].raw, "article 1er");
        assert_eq!(r[0].numero_key(), "1");

        let r = refs("l'article 199 ter du CGI et l'article L. 123-1 bis du Code de commerce");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].numero_key(), "199TER");
        assert_eq!(r[0].numero_query(), "199 ter");
        assert_eq!(r[0].raw, "article 199 ter");
        assert_eq!(r[1].numero_key(), "L123-1BIS");
        assert_eq!(r[1].code, Some("Code de commerce"));
    }

    #[test]
    fn alineas_do_not_alter_the_article_reference() {
        // L'alinéa précise l'article, il n'en fait pas partie : on vérifie l'article.
        for t in [
            "l'article L. 1234-5, alinéa 2, du Code du travail",
            "l'article L. 1234-5 alinéa 2 du Code du travail",
            "l'article L. 1234-5 al. 2 du Code du travail",
        ] {
            let r = refs(t);
            assert_eq!(r.len(), 1, "{t}");
            assert_eq!(r[0].numero_key(), "L1234-5", "{t}");
            assert_eq!(
                r[0].code,
                Some("Code du travail"),
                "le code doit survivre à l'alinéa : {t}"
            );
        }
        // Sans code explicite après l'alinéa, pas de verdict inventé.
        assert_eq!(
            keys("l'article 1103, alinéa 1er, dispose"),
            vec![("1103".into(), None)]
        );
    }

    #[test]
    fn hyphenated_numbers_are_taken_whole() {
        let r = refs("Voir L. 1234-5-1 du Code du travail.");
        assert_eq!(r[0].numero_key(), "L1234-5-1");
        // Un début de mot après le numéro ne doit pas être pris pour un suffixe.
        let r = refs("L 1234-5 bistrot du Code du travail");
        assert_eq!(r[0].numero_key(), "L1234-5");
        assert_eq!(r[0].raw, "L 1234-5");
    }

    #[test]
    fn code_in_prefix_position() {
        assert_eq!(
            keys("Code civil, article 1103, dispose que…"),
            vec![("1103".into(), Some("Code civil"))]
        );
        assert_eq!(
            keys("(Code du travail, art. L. 1234-5)"),
            vec![("L1234-5".into(), Some("Code du travail"))]
        );
        assert_eq!(
            keys("Code de commerce : article L. 110-1"),
            vec![("L110-1".into(), Some("Code de commerce"))]
        );
    }

    #[test]
    fn prefix_code_on_previous_line_is_not_used() {
        // Un code cité à la ligne précédente n'est pas assez proche pour être affirmé.
        assert_eq!(
            keys("Code du travail\narticle L. 1234-5"),
            vec![("L1234-5".into(), None)]
        );
    }

    #[test]
    fn lists_share_the_final_code() {
        assert_eq!(
            keys("Voir les articles 1103, 1104 et 1105 du Code civil."),
            vec![
                ("1103".into(), Some("Code civil")),
                ("1104".into(), Some("Code civil")),
                ("1105".into(), Some("Code civil")),
            ]
        );
        assert_eq!(
            keys("les articles L. 1233-3 et L. 1233-4 du Code du travail"),
            vec![
                ("L1233-3".into(), Some("Code du travail")),
                ("L1233-4".into(), Some("Code du travail")),
            ]
        );
        assert_eq!(
            keys("articles 1103 à 1105 du Code civil"),
            vec![
                ("1103".into(), Some("Code civil")),
                ("1105".into(), Some("Code civil")),
            ]
        );
    }

    #[test]
    fn different_codes_in_one_text_stay_separate() {
        assert_eq!(
            keys("L'article 1103 du Code civil et l'article L. 1234-5 du Code du travail."),
            vec![
                ("1103".into(), Some("Code civil")),
                ("L1234-5".into(), Some("Code du travail")),
            ]
        );
        // Un code final ne « fuit » pas au-delà d'un vrai texte intermédiaire.
        assert_eq!(
            keys(
                "l'article 1103, puis, en droit du travail, l'article L. 1234-5 du Code du travail"
            ),
            vec![
                ("1103".into(), None),
                ("L1234-5".into(), Some("Code du travail"))
            ]
        );
    }

    // ── Extraction : ce qui doit être ignoré ou laissé sans verdict ───────────

    #[test]
    fn non_code_texts_are_dropped() {
        for t in [
            "l'article 6 de la loi n° 78-17 du 6 janvier 1978",
            "l'article 12 du décret n° 2020-1234",
            "l'article 6 de la CEDH",
            "l'article 1er de la Constitution",
            "l'article 267 du TFUE",
            "l'article 5 du règlement (UE) 2016/679",
            "les articles 5 et 6 de la loi",
            "l'article 9 de la Convention européenne",
            "l'article 3 de l'ordonnance du 2 novembre 1945",
        ] {
            assert!(
                refs(t).is_empty(),
                "devrait être ignoré : {t} → {:?}",
                refs(t)
            );
        }
    }

    #[test]
    fn unknown_code_gives_no_verdict_material() {
        assert_eq!(
            keys("l'article 1240 dispose que…"),
            vec![("1240".into(), None)]
        );
        assert_eq!(
            keys("l'article L. 1234-5 dudit code"),
            vec![("L1234-5".into(), None)]
        );
        assert_eq!(
            keys("selon l'article L. 1234-5 du présent code"),
            vec![("L1234-5".into(), None)]
        );
        // Un mot commençant comme un alias n'est pas cet alias.
        assert_eq!(
            keys("article 1103 du Code civilement"),
            vec![("1103".into(), None)]
        );
    }

    #[test]
    fn numbers_with_units_or_without_article_word_are_not_refs() {
        assert!(refs("Il a été condamné à 1103 euros d'amende.").is_empty());
        assert!(refs("le contrat de 12 mois").is_empty());
        assert!(refs("SARL 1234 et PL 2024").is_empty());
        assert!(refs("la loi n° 2016-1088 du 8 août 2016").is_empty());
        // Énumération : « 12 mois » n'est pas un numéro d'article.
        assert_eq!(
            keys("les articles 1103 et 12 mois du Code civil"),
            vec![("1103".into(), None)]
        );
    }

    #[test]
    fn truncated_numbers_are_rejected_not_shortened() {
        // « L. 1234-5a » : on n'invente pas « L. 1234-5 ».
        assert!(refs("L. 1234-5a du Code du travail").is_empty());
        assert!(refs("l'article 1103/2 du Code civil").is_empty());
    }

    #[test]
    fn code_blocks_inline_code_and_links_are_ignored() {
        let t = "Texte.\n```\nl'article 1103 du Code civil\n```\nEt `article 1104 du Code civil` en ligne.\n\
                 Voir [l'article 1105 du Code civil](https://exemple.fr/art-1105) et https://exemple.fr/L-1234-56.\n\
                 Enfin l'article 1106 du Code civil.";
        let r = refs(t);
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(r[0].numero_key(), "1106");
    }

    #[test]
    fn unterminated_fence_masks_to_end() {
        assert!(refs("Intro\n```rust\narticle 1103 du Code civil").is_empty());
    }

    #[test]
    fn multibyte_text_keeps_byte_positions_valid() {
        let t = "Œuvre 😀 « défense » — l'article 1103 du Code civil ✓ puis art. 1104 C. civ.";
        let r = refs(t);
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|a| a.code == Some("Code civil")));
    }

    #[test]
    fn formatting_around_a_ref_does_not_break_it() {
        let r = refs("**article L. 1234-5 du Code du travail** et _art. 1103 C. civ._");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].code, Some("Code du travail"));
        assert_eq!(r[1].code, Some("Code civil"));
    }

    // ── Analyse des réponses Légifrance ───────────────────────────────────────

    #[test]
    fn number_key_normalisation() {
        assert_eq!(norm_num_key("L. 1234-5"), "L1234-5");
        assert_eq!(norm_num_key("Article L1234-5"), "L1234-5");
        assert_eq!(norm_num_key("<mark>1103</mark>"), "1103");
        assert_eq!(norm_num_key("R*4127-1"), "R4127-1");
        assert_eq!(norm_num_key("199 ter"), "199TER");
    }

    #[test]
    fn code_title_matching() {
        assert!(code_matches("Code civil", "Code civil"));
        assert!(code_matches("<mark>Code</mark> civil", "Code civil"));
        assert!(code_matches(
            "Code de procédure civile",
            "Code de procedure civile"
        ));
        assert!(code_matches("Code civil (ancien)", "Code civil"));
        // Préfixe suivi d'un mot distinct : accepté (ex. « Code civil (ancien) »)…
        assert!(code_matches("Code civil local", "Code civil"));
        // …mais jamais si le mot se prolonge (« Code civilité »).
        assert!(!code_matches("Code civilité", "Code civil"));
        assert!(!code_matches("Code de commerce", "Code civil"));
        assert!(!code_matches("Code pénitentiaire", "Code pénal"));
    }

    fn fixture_search() -> Value {
        json!({
            "totalResultNumber": 2,
            "results": [
                {
                    "titles": [{"id": "LEGITEXT000006070721", "cid": "LEGITEXT000006070721", "title": "Code civil"}],
                    "sections": [{
                        "id": "LEGISCTA000006118123",
                        "title": "Section 1",
                        "extracts": [
                            {"id": "LEGIARTI000032040792", "num": "1103", "legalStatus": "VIGUEUR",
                             "dateVersion": 1454284800000i64, "type": "article", "values": ["<mark>1103</mark>"]},
                            {"id": "LEGIARTI000006436298", "num": "1103", "legalStatus": "ABROGE",
                             "dateVersion": 1136073600000i64, "type": "article"},
                            {"id": "LEGIARTI000000000001", "num": "1103-1", "legalStatus": "VIGUEUR"}
                        ]
                    }]
                },
                {
                    "titles": [{"id": "LEGITEXT000005634379", "title": "<mark>Code</mark> de commerce"}],
                    "sections": [{"extracts": [{"id": "LEGIARTI000000000002", "title": "Article 1103", "legalStatus": "VIGUEUR"}]}]
                }
            ]
        })
    }

    #[test]
    fn collect_matches_reads_extracts_and_filters_by_number() {
        let m = collect_matches(&fixture_search(), "1103", None);
        assert_eq!(m.len(), 3); // 2 versions Code civil + 1 Code de commerce (num via « title »)
        assert!(
            m.iter().all(|x| x.id != "LEGIARTI000000000001"),
            "1103-1 ≠ 1103"
        );
        let civil: Vec<_> = m
            .iter()
            .filter(|x| {
                x.code_titre
                    .as_deref()
                    .map_or(false, |t| code_matches(t, "Code civil"))
            })
            .collect();
        assert_eq!(civil.len(), 2);
        let com = m.iter().find(|x| x.id == "LEGIARTI000000000002").unwrap();
        assert_eq!(com.code_titre.as_deref(), Some("Code de commerce")); // balises retirées
    }

    #[test]
    fn best_prefers_in_force_then_latest_version() {
        let m = collect_matches(&fixture_search(), "1103", None);
        let civil: Vec<Match> = m
            .into_iter()
            .filter(|x| {
                x.code_titre
                    .as_deref()
                    .map_or(false, |t| code_matches(t, "Code civil"))
            })
            .collect();
        // Deux versions (VIGUEUR + ABROGE) : la version en vigueur l'emporte.
        assert_eq!(best(&civil).unwrap().id, "LEGIARTI000032040792");
        let only_abroge = vec![
            Match {
                id: "old".into(),
                etat: Some("ABROGE".into()),
                date_version: Some(1),
                code_titre: None,
            },
            Match {
                id: "new".into(),
                etat: Some("ABROGE".into()),
                date_version: Some(2),
                code_titre: None,
            },
        ];
        assert_eq!(best(&only_abroge).unwrap().id, "new");
        let mixed = vec![
            only_abroge[1].clone(),
            Match {
                id: "v".into(),
                etat: Some("VIGUEUR".into()),
                date_version: Some(0),
                code_titre: None,
            },
        ];
        assert_eq!(best(&mixed).unwrap().id, "v");
    }

    #[test]
    fn implied_etat_fills_missing_status() {
        let resp = json!({"results": [{"titles": [{"title": "Code civil"}],
            "sections": [{"extracts": [{"id": "LEGIARTI1", "num": "1103"}]}]}]});
        assert_eq!(
            collect_matches(&resp, "1103", Some("VIGUEUR"))[0]
                .etat
                .as_deref(),
            Some("VIGUEUR")
        );
        assert_eq!(collect_matches(&resp, "1103", None)[0].etat, None);
    }

    #[test]
    fn collect_matches_tolerates_odd_shapes() {
        assert!(collect_matches(&json!({}), "1103", None).is_empty());
        assert!(collect_matches(&json!({"results": null}), "1103", None).is_empty());
        assert!(
            collect_matches(&json!({"results": [{"sections": null}, {}]}), "1103", None).is_empty()
        );
    }

    #[test]
    fn dates_from_millis() {
        use chrono::{TimeZone, Utc};
        let minuit_utc = Utc
            .with_ymd_and_hms(2021, 3, 31, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        let minuit_paris_ete = Utc
            .with_ymd_and_hms(2021, 3, 30, 22, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(millis_to_date(minuit_utc).as_deref(), Some("2021-03-31"));
        assert_eq!(
            millis_to_date(minuit_paris_ete).as_deref(),
            Some("2021-03-31")
        );
        assert_eq!(
            millis_to_date(32472144000000),
            None,
            "sentinelle « pas de fin » (2999)"
        );
    }

    #[test]
    fn article_info_reads_etat_and_end_date() {
        let (etat, fin) =
            article_info(&json!({"article": {"etat": "ABROGE", "dateFin": "1470787200000"}}));
        assert_eq!(etat.as_deref(), Some("ABROGE"));
        assert_eq!(fin.as_deref(), Some("2016-08-10"));
        let (etat, fin) = article_info(&json!({"etat": "VIGUEUR", "dateFin": 32472144000000i64}));
        assert_eq!(etat.as_deref(), Some("VIGUEUR"));
        assert_eq!(fin, None);
        assert_eq!(article_info(&json!({})), (None, None));
    }

    #[test]
    fn etat_labels() {
        assert_eq!(etat_libelle("ABROGE"), "abrogé");
        assert_eq!(etat_libelle("abroge_diff"), "abrogation différée");
        assert_eq!(etat_libelle("QUELQUE_CHOSE"), "quelque chose");
        assert_eq!(
            article_url("LEGIARTI000032040792").as_deref(),
            Some("https://www.legifrance.gouv.fr/codes/article_lc/LEGIARTI000032040792")
        );
        assert_eq!(article_url("JORFARTI000001"), None);
    }

    // ── Orchestration avec une source simulée ─────────────────────────────────

    /// Source simulée : table (code, clé de numéro) → résultat ; compte les appels.
    #[derive(Default)]
    struct MockLookup {
        table: HashMap<(String, String), Result<Lookup, String>>,
        calls: Mutex<Vec<(String, String)>>,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        delay_ms: u64,
    }

    impl MockLookup {
        fn with(mut self, code: &str, num: &str, r: Result<Lookup, String>) -> Self {
            self.table.insert((code.to_string(), norm_num_key(num)), r);
            self
        }
        fn n_calls(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl ArticleLookup for MockLookup {
        fn lookup<'a>(
            &'a self,
            code: &'a str,
            numero: &'a str,
        ) -> BoxFuture<'a, Result<Lookup, String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push((code.to_string(), numero.to_string()));
                let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_in_flight.fetch_max(now, Ordering::SeqCst);
                if self.delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
                }
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                self.table
                    .get(&(code.to_string(), norm_num_key(numero)))
                    .cloned()
                    .unwrap_or(Ok(Lookup::Introuvable { ailleurs: vec![] }))
            })
        }
    }

    #[tokio::test]
    async fn verify_all_maps_each_outcome_to_a_status() {
        let text = "L'article 1103 du Code civil (en vigueur), l'article L. 1234-99 du Code du travail (abrogé), \
                    l'article 9999 du Code civil (inventé), l'article L. 1234-5 du Code de commerce (mauvais code) \
                    et l'article 12 sans code, enfin L. 7-7 du Code du sport.";
        let mock = MockLookup::default()
            .with(
                "Code civil",
                "1103",
                Ok(Lookup::EnVigueur {
                    id: "LEGIARTI000032040792".into(),
                }),
            )
            .with(
                "Code du travail",
                "L1234-99",
                Ok(Lookup::Autre {
                    id: "LEGIARTI000006900000".into(),
                    etat: Some("ABROGE".into()),
                    date_fin: Some("2016-08-10".into()),
                }),
            )
            .with(
                "Code de commerce",
                "L1234-5",
                Ok(Lookup::Introuvable {
                    ailleurs: vec!["Code du travail".into()],
                }),
            )
            .with("Code du sport", "L7-7", Err("HTTP 502".into()));
        let r = refs(text);
        let rep = verify_all(&mock, text, &r, "2026-09-30").await;

        assert_eq!(rep.date, "2026-09-30");
        let by = |k: &str| {
            rep.verifications
                .iter()
                .find(|v| v.numero == k)
                .unwrap_or_else(|| panic!("{k}"))
        };

        let v = by("1103");
        assert_eq!((v.statut, v.existe), (Statut::EnVigueur, Some(true)));
        assert_eq!(
            v.url.as_deref(),
            Some("https://www.legifrance.gouv.fr/codes/article_lc/LEGIARTI000032040792")
        );
        assert_eq!(v.reference, "Code civil, art. 1103");

        let v = by("L. 1234-99");
        assert_eq!((v.statut, v.existe), (Statut::Abroge, Some(true)));
        assert_eq!(v.etat.as_deref(), Some("ABROGE"));
        assert_eq!(v.etat_libelle.as_deref(), Some("abrogé"));
        assert_eq!(v.date_fin.as_deref(), Some("2016-08-10"));

        let v = by("9999");
        assert_eq!((v.statut, v.existe), (Statut::Introuvable, Some(false)));

        let v = by("L. 1234-5");
        assert_eq!(v.statut, Statut::Introuvable);
        assert_eq!(v.existe_dans, vec!["Code du travail".to_string()]);

        // Sans code : aucun verdict, et aucun appel réseau.
        let v = by("12");
        assert_eq!((v.statut, v.existe), (Statut::NonVerifiable, None));
        assert!(v.detail.as_deref().unwrap().contains("Code non précisé"));

        // Erreur d'API : PAS « introuvable ».
        let v = by("L. 7-7");
        assert_eq!((v.statut, v.existe), (Statut::Erreur, None));
        assert!(
            !v.detail.as_deref().unwrap().contains("502"),
            "pas de fuite du détail technique"
        );

        assert_eq!(mock.n_calls(), 5, "5 références avec code, 1 sans");
    }

    #[tokio::test]
    async fn identical_refs_are_looked_up_once_with_all_occurrences() {
        let text = "L'article 1103 du Code civil. Plus loin : art. 1103 C. civ. — encore l'article 1103 du Code civil.";
        let mock = MockLookup::default().with(
            "Code civil",
            "1103",
            Ok(Lookup::EnVigueur {
                id: "LEGIARTI1".into(),
            }),
        );
        let r = refs(text);
        let rep = verify_all(&mock, text, &r, "2026-09-30").await;
        assert_eq!(rep.verifications.len(), 1);
        assert_eq!(rep.verifications[0].occurrences.len(), 3);
        assert_eq!(mock.n_calls(), 1);
    }

    #[tokio::test]
    async fn occurrence_offsets_are_utf16_and_slice_back_to_the_text() {
        let text = "😀 « Réponse » — l'article 1103 du Code civil.";
        let mock = MockLookup::default().with(
            "Code civil",
            "1103",
            Ok(Lookup::EnVigueur {
                id: "LEGIARTI1".into(),
            }),
        );
        let r = refs(text);
        let rep = verify_all(&mock, text, &r, "2026-09-30").await;
        let occ = &rep.verifications[0].occurrences[0];
        // Ce que fait le frontend : découper en unités UTF-16.
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let slice = String::from_utf16(&utf16[occ.debut..occ.fin]).unwrap();
        assert_eq!(slice, occ.texte);
        assert_eq!(slice, "article 1103");
    }

    #[tokio::test]
    async fn distinct_lookups_are_capped_and_the_rest_flagged() {
        let text: String = (1000..1030)
            .map(|n| format!("l'article {n} du Code civil ; "))
            .collect();
        let mock = MockLookup::default();
        let r = refs(&text);
        assert_eq!(r.len(), 30);
        let rep = verify_all(&mock, &text, &r, "2026-09-30").await;
        assert_eq!(mock.n_calls(), MAX_DISTINCT_LOOKUPS);
        assert_eq!(rep.verifications.len(), 30);
        let sans: Vec<_> = rep
            .verifications
            .iter()
            .filter(|v| v.statut == Statut::NonVerifiable)
            .collect();
        assert_eq!(sans.len(), 30 - MAX_DISTINCT_LOOKUPS);
        assert!(sans[0].detail.as_deref().unwrap().contains("Limite"));
        // L'ordre d'apparition est conservé.
        assert_eq!(rep.verifications[0].numero, "1000");
        assert_eq!(rep.verifications[29].numero, "1029");
    }

    #[tokio::test]
    async fn lookups_run_in_parallel_but_bounded() {
        let text: String = (2000..2012)
            .map(|n| format!("l'article {n} du Code civil ; "))
            .collect();
        let mock = MockLookup {
            delay_ms: 30,
            ..Default::default()
        };
        let r = refs(&text);
        let t0 = std::time::Instant::now();
        let _ = verify_all(&mock, &text, &r, "2026-09-30").await;
        let max = mock.max_in_flight.load(Ordering::SeqCst);
        assert!(max > 1, "devrait paralléliser (max simultané = {max})");
        assert!(
            max <= LOOKUP_CONCURRENCY,
            "borné à {LOOKUP_CONCURRENCY} (max simultané = {max})"
        );
        assert!(
            t0.elapsed() < Duration::from_millis(30 * 12),
            "plus rapide que le séquentiel"
        );
    }

    #[tokio::test]
    async fn no_refs_means_empty_report() {
        let rep = verify_all(
            &NoLookup,
            "Bonjour, aucune référence ici.",
            &[],
            "2026-09-30",
        )
        .await;
        assert!(rep.verifications.is_empty());
    }

    // ── Cache ─────────────────────────────────────────────────────────────────

    struct Counting {
        n: AtomicUsize,
        fail_first: bool,
    }
    impl ArticleLookup for Counting {
        fn lookup<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<Lookup, String>> {
            Box::pin(async move {
                let n = self.n.fetch_add(1, Ordering::SeqCst);
                if self.fail_first && n == 0 {
                    Err("panne".into())
                } else {
                    Ok(Lookup::EnVigueur {
                        id: "LEGIARTI1".into(),
                    })
                }
            })
        }
    }

    #[tokio::test]
    async fn cache_serves_repeat_lookups_per_day() {
        let cache = new_cache();
        let l = CachedLookup::new(
            Counting {
                n: AtomicUsize::new(0),
                fail_first: false,
            },
            cache.clone(),
            "2026-09-30".into(),
        );
        assert!(l.lookup("Code civil", "1103").await.is_ok());
        assert!(l.lookup("Code civil", "1103").await.is_ok());
        assert!(l.lookup("Code civil", "L. 1103").await.is_ok()); // clé normalisée
        assert_eq!(
            l.inner.n.load(Ordering::SeqCst),
            2,
            "1103 puis L1103 : 2 appels réels seulement"
        );

        // Un autre jour ne réutilise pas le cache.
        let autre_jour = CachedLookup::new(
            Counting {
                n: AtomicUsize::new(0),
                fail_first: false,
            },
            cache,
            "2026-10-01".into(),
        );
        assert!(autre_jour.lookup("Code civil", "1103").await.is_ok());
        assert_eq!(autre_jour.inner.n.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn errors_are_never_cached() {
        let l = CachedLookup::new(
            Counting {
                n: AtomicUsize::new(0),
                fail_first: true,
            },
            new_cache(),
            "2026-09-30".into(),
        );
        assert!(l.lookup("Code civil", "1103").await.is_err());
        assert!(
            l.lookup("Code civil", "1103").await.is_ok(),
            "la panne ne doit pas être mémorisée"
        );
        assert_eq!(l.inner.n.load(Ordering::SeqCst), 2);
    }

    // ── Faux Légifrance (axum) : adaptateur + handler de bout en bout ─────────

    #[derive(Clone)]
    struct MockArticle {
        id: &'static str,
        code: &'static str,
        num: &'static str,
        etat: &'static str,
        date_fin_ms: Option<i64>,
    }

    #[derive(Clone)]
    struct MockState {
        articles: Arc<Vec<MockArticle>>,
        searches: Arc<Mutex<Vec<Value>>>,
        get_articles: Arc<AtomicUsize>,
        /// Simule une API qui ignorerait le filtre de code.
        ignore_code_filter: bool,
    }

    impl MockState {
        fn new(articles: Vec<MockArticle>) -> Self {
            Self {
                articles: Arc::new(articles),
                searches: Arc::default(),
                get_articles: Arc::default(),
                ignore_code_filter: false,
            }
        }
    }

    fn facet(filtres: &[Value], name: &str) -> Option<Vec<String>> {
        filtres.iter().find(|f| f["facette"] == name).map(|f| {
            f["valeurs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        })
    }

    async fn mock_search(
        State(st): State<MockState>,
        Json(body): Json<Value>,
    ) -> axum::response::Response {
        st.searches.lock().unwrap().push(body.clone());
        let rech = &body["recherche"];
        let valeur = rech["champs"][0]["criteres"][0]["valeur"]
            .as_str()
            .unwrap_or("");
        if valeur == "500" {
            return (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response();
        }
        let filtres = rech["filtres"].as_array().cloned().unwrap_or_default();
        let codes = facet(&filtres, "TEXT_NOM_CODE");
        let etats = facet(&filtres, "TEXT_LEGAL_STATUS");
        let wanted = norm_num_key(valeur);

        let mut by_code: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
        for a in st.articles.iter().filter(|a| norm_num_key(a.num) == wanted) {
            if let Some(cs) = &codes {
                if !st.ignore_code_filter && !cs.iter().any(|c| c == a.code) {
                    continue;
                }
            }
            if let Some(es) = &etats {
                if !es.iter().any(|e| e == a.etat) {
                    continue;
                }
            }
            by_code.entry(a.code).or_default().push(json!({
                "id": a.id, "num": a.num, "legalStatus": a.etat,
                "title": format!("Article {}", a.num), "type": "article",
            }));
        }
        let results: Vec<Value> = by_code
            .into_iter()
            .map(|(code, extracts)| {
                json!({
                    "titles": [{"id": "LEGITEXT0", "cid": "LEGITEXT0", "title": code}],
                    "sections": [{"id": "LEGISCTA0", "title": "Section", "extracts": extracts}],
                })
            })
            .collect();
        Json(json!({"totalResultNumber": results.len(), "results": results})).into_response()
    }

    async fn mock_get_article(
        State(st): State<MockState>,
        Json(body): Json<Value>,
    ) -> axum::response::Response {
        st.get_articles.fetch_add(1, Ordering::SeqCst);
        let id = body["id"].as_str().unwrap_or("");
        match st.articles.iter().find(|a| a.id == id) {
            Some(a) => Json(json!({"article": {
                "id": a.id, "num": a.num, "etat": a.etat,
                "dateFin": a.date_fin_ms.unwrap_or(32472144000000i64),
            }}))
            .into_response(),
            None => (StatusCode::NOT_FOUND, "absent").into_response(),
        }
    }

    async fn spawn_mock(st: MockState) -> String {
        let app = Router::new()
            .route(
                "/oauth/token",
                post(|| async { Json(json!({"access_token": "t", "expires_in": 3600})) }),
            )
            .route("/legifrance/search", post(mock_search))
            .route("/legifrance/consult/getArticle", post(mock_get_article))
            .with_state(st);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn corpus() -> Vec<MockArticle> {
        vec![
            MockArticle {
                id: "LEGIARTI000032040792",
                code: "Code civil",
                num: "1103",
                etat: "VIGUEUR",
                date_fin_ms: None,
            },
            MockArticle {
                id: "LEGIARTI000006436298",
                code: "Code civil",
                num: "1134",
                etat: "ABROGE",
                date_fin_ms: Some(1470787200000),
            },
            MockArticle {
                id: "LEGIARTI000006900001",
                code: "Code du travail",
                num: "L1234-99",
                etat: "ABROGE",
                date_fin_ms: Some(1470787200000),
            },
            MockArticle {
                id: "LEGIARTI000006900002",
                code: "Code du travail",
                num: "L1234-5",
                etat: "VIGUEUR",
                date_fin_ms: None,
            },
            MockArticle {
                id: "LEGIARTI000006900003",
                code: "Code de commerce",
                num: "1103",
                etat: "VIGUEUR",
                date_fin_ms: None,
            },
        ]
    }

    async fn lookup_via_mock(st: &MockState, code: &str, num: &str) -> Result<Lookup, String> {
        let base = spawn_mock(st.clone()).await;
        LegifranceLookup::new(PisteClient::new_for_test(&base))
            .lookup(code, num)
            .await
    }

    #[tokio::test]
    async fn adapter_in_force_article_costs_one_call_with_the_right_request() {
        let st = MockState::new(corpus());
        let r = lookup_via_mock(&st, "Code civil", "1103").await.unwrap();
        assert_eq!(
            r,
            Lookup::EnVigueur {
                id: "LEGIARTI000032040792".into()
            }
        );

        let calls = st.searches.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let b = &calls[0];
        assert_eq!(b["fond"], "CODE_ETAT");
        assert_eq!(b["recherche"]["champs"][0]["typeChamp"], "NUM_ARTICLE");
        assert_eq!(b["recherche"]["champs"][0]["criteres"][0]["valeur"], "1103");
        let f = b["recherche"]["filtres"].as_array().unwrap();
        assert_eq!(
            facet(f, "TEXT_NOM_CODE"),
            Some(vec!["Code civil".to_string()])
        );
        assert_eq!(
            facet(f, "TEXT_LEGAL_STATUS"),
            Some(vec!["VIGUEUR".to_string()])
        );
        assert_eq!(
            st.get_articles.load(Ordering::SeqCst),
            0,
            "pas de getArticle pour un article en vigueur"
        );
    }

    #[tokio::test]
    async fn adapter_reports_repealed_article_with_end_date() {
        let st = MockState::new(corpus());
        let r = lookup_via_mock(&st, "Code du travail", "L1234-99")
            .await
            .unwrap();
        assert_eq!(
            r,
            Lookup::Autre {
                id: "LEGIARTI000006900001".into(),
                etat: Some("ABROGE".into()),
                date_fin: Some("2016-08-10".into()),
            }
        );
        assert_eq!(
            st.searches.lock().unwrap().len(),
            2,
            "recherche « en vigueur » puis « autres états »"
        );
        assert_eq!(st.get_articles.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn adapter_invented_article_is_not_found_only_after_looking_everywhere() {
        let st = MockState::new(corpus());
        let r = lookup_via_mock(&st, "Code civil", "9999").await.unwrap();
        assert_eq!(r, Lookup::Introuvable { ailleurs: vec![] });
        let calls = st.searches.lock().unwrap();
        assert_eq!(calls.len(), 3);
        // La 3e recherche n'a ni filtre de code ni filtre d'état (garde-fou).
        let f3 = calls[2]["recherche"]["filtres"].as_array().unwrap();
        assert!(facet(f3, "TEXT_NOM_CODE").is_none() && facet(f3, "TEXT_LEGAL_STATUS").is_none());
    }

    #[tokio::test]
    async fn adapter_flags_number_that_exists_only_in_another_code() {
        let st = MockState::new(corpus());
        // L1234-5 existe dans le Code du travail, pas dans le Code civil.
        let r = lookup_via_mock(&st, "Code civil", "L1234-5").await.unwrap();
        assert_eq!(
            r,
            Lookup::Introuvable {
                ailleurs: vec!["Code du travail".into()]
            }
        );
    }

    #[tokio::test]
    async fn adapter_does_not_trust_a_code_filter_the_api_ignored() {
        // Le mock renvoie l'art. 1103 du Code de commerce même quand on demande le
        // Code civil filtré : on ne doit pas le prendre pour l'article du Code civil…
        let mut st = MockState::new(vec![MockArticle {
            id: "LEGIARTI000006900003",
            code: "Code de commerce",
            num: "1103",
            etat: "VIGUEUR",
            date_fin_ms: None,
        }]);
        st.ignore_code_filter = true;
        let r = lookup_via_mock(&st, "Code civil", "1103").await.unwrap();
        assert_eq!(
            r,
            Lookup::Introuvable {
                ailleurs: vec!["Code de commerce".into()]
            }
        );
        // … mais le demander pour le bon code fonctionne malgré le filtre ignoré.
        let r = lookup_via_mock(&st, "Code de commerce", "1103")
            .await
            .unwrap();
        assert!(matches!(r, Lookup::EnVigueur { .. }));
    }

    #[tokio::test]
    async fn adapter_api_error_is_an_error_not_a_verdict() {
        let st = MockState::new(corpus());
        assert!(lookup_via_mock(&st, "Code civil", "500").await.is_err());
    }

    // ── Handler HTTP de bout en bout ──────────────────────────────────────────

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
            .with_state(Arc::new(TestState { js }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    async fn post_texte(api: &str, texte: &str) -> (StatusCode, Value) {
        let res = reqwest::Client::new()
            .post(format!("{api}/verifier-citations"))
            .json(&json!({ "texte": texte }))
            .send()
            .await
            .unwrap();
        let status = StatusCode::from_u16(res.status().as_u16()).unwrap();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn handler_end_to_end_ok_repealed_invented_and_cache() {
        let st = MockState::new(corpus());
        let base = spawn_mock(st.clone()).await;
        let js = JuridiqueState::new(None, None);
        *js.piste_client.write().await = Some(PisteClient::new_for_test(&base));
        let api = spawn_api(js).await;

        let texte = "L'article 1103 du Code civil est en vigueur ; l'article 1134 du Code civil a été abrogé ; \
                     l'article 7777 du Code civil n'existe pas.";
        let (status, body) = post_texte(&api, texte).await;
        assert_eq!(status, StatusCode::OK);
        let v = body["verifications"].as_array().unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0]["statut"], "en_vigueur");
        assert_eq!(v[0]["existe"], true);
        assert_eq!(v[1]["statut"], "abroge");
        assert_eq!(v[1]["date_fin"], "2016-08-10");
        assert_eq!(v[2]["statut"], "introuvable");
        assert_eq!(v[2]["existe"], false);
        assert_eq!(v[0]["occurrences"][0]["texte"], "article 1103");

        // 2e passage identique : tout vient du cache, Légifrance n'est plus sollicité.
        let avant = st.searches.lock().unwrap().len();
        let (status, body2) = post_texte(&api, texte).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body2["verifications"], body["verifications"]);
        assert_eq!(
            st.searches.lock().unwrap().len(),
            avant,
            "aucun appel supplémentaire"
        );
    }

    #[tokio::test]
    async fn handler_reports_api_failure_as_erreur_not_introuvable() {
        let st = MockState::new(corpus());
        let base = spawn_mock(st).await;
        let js = JuridiqueState::new(None, None);
        *js.piste_client.write().await = Some(PisteClient::new_for_test(&base));
        let api = spawn_api(js).await;
        let (status, body) = post_texte(&api, "Voir l'article 500 du Code civil.").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["verifications"][0]["statut"], "erreur");
        assert!(body["verifications"][0]["existe"].is_null());
    }

    #[tokio::test]
    async fn handler_needs_piste_only_when_something_is_verifiable() {
        let api = spawn_api(JuridiqueState::new(None, None)).await;

        // Aucune référence, ou référence sans code : pas besoin de PISTE.
        let (status, body) = post_texte(&api, "Bonjour.").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["verifications"].as_array().unwrap().is_empty());
        let (status, body) = post_texte(&api, "Voir l'article 1240.").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["verifications"][0]["statut"], "non_verifiable");

        // Référence vérifiable mais PISTE non configuré : 503 explicite.
        let (status, _) = post_texte(&api, "Voir l'article 1240 du Code civil.").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_rejects_oversized_text() {
        let api = spawn_api(JuridiqueState::new(None, None)).await;
        let (status, _) = post_texte(&api, &"a".repeat(MAX_TEXT_BYTES + 1)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    // ── Textes hors codes ─────────────────────────────────────────────────────

    fn texts(t: &str) -> Vec<TextRef> {
        let r = extract_text_refs(t);
        for x in &r {
            assert_eq!(&t[x.start..x.end], x.raw, "position incohérente pour {x:?}");
        }
        r
    }

    #[test]
    fn text_refs_by_number_and_or_date() {
        let r = texts("Le décret n° 2020-1455 du 27 novembre 2020 organise la PJJ.");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].raw, "décret n° 2020-1455 du 27 novembre 2020");
        assert_eq!(
            (r[0].nature, r[0].numero.as_deref(), r[0].date.as_deref()),
            ("decret", Some("2020-1455"), Some("2020-11-27"))
        );

        let r = texts("Vu l'arrêté du 8 décembre 2021 portant fermeture.");
        assert_eq!(
            (r[0].nature, r[0].numero.as_deref(), r[0].date.as_deref()),
            ("arrete", None, Some("2021-12-08"))
        );
        assert_eq!(r[0].raw, "arrêté du 8 décembre 2021");

        let r = texts("la loi n° 78-17 du 6 janvier 1978 et l'ordonnance n°2016-131");
        assert_eq!(r.len(), 2);
        assert_eq!(
            (r[0].nature, r[0].numero.as_deref(), r[0].date.as_deref()),
            ("loi", Some("78-17"), Some("1978-01-06"))
        );
        assert_eq!(
            (r[1].nature, r[1].numero.as_deref(), r[1].date.as_deref()),
            ("ordonnance", Some("2016-131"), None)
        );
    }

    #[test]
    fn text_refs_notation_variants() {
        for t in [
            "Décret n°2020-1455",
            "décret no 2020-1455",
            "DÉCRET N° 2020-1455",
            "décret n\u{a0}°\u{a0}2020-1455",
            "décret nº 2020-1455",
            "décret numéro 2020-1455",
            "décret n° 2020 - 1455",
        ] {
            let r = texts(t);
            assert_eq!(r.len(), 1, "{t}");
            assert_eq!(r[0].numero.as_deref(), Some("2020-1455"), "{t}");
        }
        // 1er, mois accentués ou non, « en date du », virgule, adjectif.
        let r = texts("arrêté ministériel, en date du 1er février 2019");
        assert_eq!(r[0].date.as_deref(), Some("2019-02-01"));
        assert_eq!(
            texts("arrete du 3 aout 2020")[0].date.as_deref(),
            Some("2020-08-03")
        );
        assert_eq!(
            texts("loi organique n° 2001-692 du 1er août 2001")[0].nature,
            "loi"
        );
        assert_eq!(texts("décret-loi du 29 juillet 1939")[0].nature, "decret");
    }

    #[test]
    fn text_refs_ignore_generic_mentions_code_and_links() {
        assert!(texts(
            "la loi prévoit que le décret s'applique à tout arrêté ; une décision est rendue"
        )
        .is_empty());
        assert!(texts("loisir n° 12-3 et décrétons que").is_empty());
        assert!(texts("date impossible : arrêté du 45 mars 2020").len() <= 1); // jour 45 : pas de date
        assert!(texts("arrêté du 45 mars 2020").is_empty());
        let t = "`décret n° 2020-1455` et [décret n° 2021-1](https://x.fr/décret-n-2021-2)\n```\nloi n° 99-1\n```";
        assert!(texts(t).is_empty(), "{:?}", texts(t));
    }

    #[test]
    fn article_of_a_law_yields_the_law_as_text_not_a_code_article() {
        let t = "l'article 6 de la loi n° 78-17 du 6 janvier 1978";
        assert!(refs(t).is_empty());
        assert_eq!(texts(t).len(), 1);
    }

    // ── Clés et index d'ancrage ───────────────────────────────────────────────

    #[test]
    fn grounding_keys_format_and_stability() {
        let k = grounding_keys(
            "Code civil, art. 1103 ; art. L. 7-7 ; le décret n° 2020-1455 du 27 novembre 2020.",
        );
        assert_eq!(
            k,
            vec![
                "art|?|L7-7".to_string(),
                "art|Code civil|1103".to_string(),
                "txt|decret|d:2020-11-27".to_string(),
                "txt|decret|n:2020-1455".to_string(),
            ]
        );
        assert!(grounding_keys("rien à signaler").is_empty());
    }

    #[test]
    fn index_matches_articles_leniently_when_source_has_no_code() {
        let idx = GroundingIndex::from_texts([
            "Article 1103\nLes contrats légalement formés…",
            "l'article 700 du Code de procédure civile",
        ]);
        assert!(
            idx.has_article("Code civil", "1103"),
            "source sans code : on ne contredit pas"
        );
        assert!(idx.has_article("Code de procédure civile", "700"));
        assert!(
            !idx.has_article("Code civil", "700"),
            "700 est ancré à un AUTRE code"
        );
        assert!(!idx.has_article("Code civil", "1104"));
    }

    #[test]
    fn index_matches_texts_by_number_or_by_date() {
        let idx = GroundingIndex::from_texts([
            "Arrêté du 21 décembre 2022 portant création",
            "décret n° 2021-99",
        ]);
        let by_date = &texts("arrêté du 21 décembre 2022")[0];
        let by_num = &texts("décret n° 2021-99 du 3 mars 2021")[0]; // date différente, numéro présent
        let other = &texts("décret n° 2020-1455 du 27 novembre 2020")[0];
        let wrong_nature = &texts("loi n° 2021-99")[0];
        assert!(idx.has_texte(by_date));
        assert!(idx.has_texte(by_num));
        assert!(!idx.has_texte(other));
        assert!(
            !idx.has_texte(wrong_nature),
            "la nature compte : décret ≠ loi"
        );
    }

    /// Formats RÉELS produits par les serveurs du pack (fmt_article / fmt_search de
    /// mcp-legifrance) : ce que le LLM reçoit d'un appel d'outil doit ancrer ses citations.
    #[test]
    fn mcp_tool_outputs_ground_later_citations() {
        // fmt_article : le code n'est PAS nommé (« ### Article 1103 »), seulement le numéro.
        let article = "### Article 1103 — Des contrats\n*État : VIGUEUR*\n*En vigueur depuis : 2016-10-01*\n*ID : `LEGIARTI000032040777`*\n\nLes contrats légalement formés tiennent lieu de loi à ceux qui les ont faits.";
        let prefixed = "### Article L1234-5\n*État : VIGUEUR*\n*ID : `LEGIARTI000006901000`*\n\nLorsque le licenciement…";
        // fmt_search : titres de textes (LODA) avec numéro et/ou date.
        let search = "3 résultat(s) — 3 affiché(s)\n\n**1**. Décret n° 2020-1455 du 27 novembre 2020 relatif aux missions de la protection judiciaire de la jeunesse\n   Date : 2020-11-29\n   ID : `JORFTEXT000042584322`\n\n**2**. Arrêté du 21 décembre 2022 portant création d'un service territorial éducatif de milieu ouvert\n   ID : `JORFTEXT000046967783`\n";
        let keys: Vec<String> = [article, prefixed, search]
            .iter()
            .flat_map(|t| grounding_keys(t))
            .collect();
        let mut idx = GroundingIndex::default();
        idx.extend_keys(keys);

        // « Article 1103 » sans code côté outil : compte pour le Code civil (indulgent par construction).
        assert!(idx.has_article("Code civil", "1103"));
        assert!(idx.has_article("Code du travail", "L1234-5"));
        assert!(idx.has_texte(&texts("décret n° 2020-1455 du 27 novembre 2020")[0]));
        assert!(idx.has_texte(&texts("arrêté du 21 décembre 2022")[0]));
        // Ce que l'outil n'a PAS ramené reste « hors sources ».
        assert!(!idx.has_article("Code civil", "1104"));
        assert!(
            !idx.has_texte(&texts("décret n° 2019-1-99")[0])
                || texts("décret n° 2019-1-99").is_empty()
        );
        assert!(
            !idx.has_texte(&texts("loi n° 2020-1455")[0]),
            "autre nature, même numéro"
        );
    }

    #[test]
    fn external_keys_are_validated_and_capped() {
        let mut idx = GroundingIndex::default();
        idx.extend_keys([
            "art|Code civil|1103".to_string(),
            "n'importe quoi".to_string(),
            format!("txt|{}", "x".repeat(300)),
        ]);
        assert!(idx.has_article("Code civil", "1103"));
        assert_eq!(
            idx.keys.len(),
            1,
            "clés mal formées ou trop longues ignorées"
        );
        assert!(GroundingIndex::default().is_empty());
    }

    // ── Rapport avec ancrage ──────────────────────────────────────────────────

    /// Le scénario de la capture : le dossier contient deux arrêtés ; la réponse
    /// ajoute une « base légale implicite » (décret, article de code) absente des sources.
    #[tokio::test]
    async fn dossier_answer_flags_what_came_from_model_memory() {
        let sources = "📋 Synthèse de dossier — 2 documents\n\n**Arrêté du 21 décembre 2022 portant création d'un service territorial éducatif de milieu ouvert à Toulon (83)**\nVu l'article 1103 du Code civil…\n\n\
                       **Arrêté du 8 décembre 2021 portant fermeture de l'hélistation de Toulon-Saint-Mandrier**\nModifie l'arrêté du 23 novembre 1962 fixant la liste des aérodromes.";
        let answer = "Arrêté du 21 décembre 2022 : création d'un STEMO. Arrêté du 8 décembre 2021 : fermeture. Il modifie l'arrêté du 23 novembre 1962. \
                      Base légale implicite : le décret n° 2020-1455 du 27 novembre 2020, et l'article 1103 du Code civil, et l'article L. 1234-5 du Code du travail.";
        let mock = MockLookup::default()
            .with(
                "Code civil",
                "1103",
                Ok(Lookup::EnVigueur {
                    id: "LEGIARTI1".into(),
                }),
            )
            .with(
                "Code du travail",
                "L1234-5",
                Ok(Lookup::EnVigueur {
                    id: "LEGIARTI2".into(),
                }),
            );
        let idx = GroundingIndex::from_texts([sources]);
        let r = refs(answer);
        let t = texts(answer);
        let rep = build_report(&mock, answer, &r, &t, Some(&idx), "2026-09-30").await;

        let find = |needle: &str| {
            rep.verifications
                .iter()
                .find(|v| v.numero.contains(needle) || v.reference.contains(needle))
                .unwrap_or_else(|| panic!("{needle}"))
        };
        // Fournis par le dossier → ancrés.
        assert_eq!(find("21 décembre 2022").ancrage, Some(Ancrage::DansSources));
        assert_eq!(find("8 décembre 2021").ancrage, Some(Ancrage::DansSources));
        assert_eq!(find("23 novembre 1962").ancrage, Some(Ancrage::DansSources));
        assert_eq!(find("1103").ancrage, Some(Ancrage::DansSources));
        // Sortis de la mémoire du modèle → signalés, même si l'article existe bel et bien.
        assert_eq!(find("2020-1455").ancrage, Some(Ancrage::HorsSources));
        let l = find("1234-5");
        assert_eq!(
            (l.statut, l.ancrage),
            (Statut::EnVigueur, Some(Ancrage::HorsSources))
        );
        // Les textes hors codes ne sont jamais déclarés existants ni inexistants.
        let d = find("2020-1455");
        assert_eq!(
            (d.kind, d.statut, d.existe),
            (Kind::Texte, Statut::NonVerifiable, None)
        );
        assert_eq!(find("1103").kind, Kind::Article);
    }

    #[tokio::test]
    async fn without_grounding_there_is_no_ancrage_at_all() {
        let t = "décret n° 2020-1455 du 27 novembre 2020 et article 1103 du Code civil";
        let mock = MockLookup::default().with(
            "Code civil",
            "1103",
            Ok(Lookup::EnVigueur { id: "L1".into() }),
        );
        let rep = build_report(&mock, t, &refs(t), &texts(t), None, "2026-09-30").await;
        assert_eq!(rep.verifications.len(), 2);
        assert!(rep.verifications.iter().all(|v| v.ancrage.is_none()));
    }

    #[tokio::test]
    async fn text_occurrences_are_grouped_and_utf16_positioned() {
        let t = "😀 Le décret n° 2020-1455 du 27 novembre 2020 … puis décret n°2020-1455.";
        let rep = build_report(&NoLookup, t, &[], &texts(t), None, "2026-09-30").await;
        assert_eq!(
            rep.verifications.len(),
            1,
            "même texte cité deux fois = un seul regroupement"
        );
        let v = &rep.verifications[0];
        assert_eq!(v.occurrences.len(), 2);
        let utf16: Vec<u16> = t.encode_utf16().collect();
        for o in &v.occurrences {
            assert_eq!(String::from_utf16(&utf16[o.debut..o.fin]).unwrap(), o.texte);
        }
    }

    // ── Handler : ancrage de bout en bout ─────────────────────────────────────

    #[tokio::test]
    async fn handler_ancrage_end_to_end() {
        let st = MockState::new(corpus());
        let base = spawn_mock(st).await;
        let js = JuridiqueState::new(None, None);
        *js.piste_client.write().await = Some(PisteClient::new_for_test(&base));
        let api = spawn_api(js).await;
        let client = reqwest::Client::new();
        let post = |body: Value| {
            let client = client.clone();
            let api = api.clone();
            async move {
                let res = client
                    .post(format!("{api}/verifier-citations"))
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                (
                    res.status().as_u16(),
                    res.json::<Value>().await.unwrap_or(Value::Null),
                )
            }
        };
        let texte = "Décret n° 2020-1455 du 27 novembre 2020 ; article 1103 du Code civil ; arrêté du 8 décembre 2021.";
        let by = |b: &Value, needle: &str| {
            b["verifications"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["reference"].as_str().unwrap().contains(needle))
                .cloned()
                .unwrap()
        };

        // Ancrage demandé, sources fournies : le dossier ne contient que l'arrêté et l'art. 1103.
        let (s, b) = post(json!({"texte": texte, "ancrage": true,
            "sources": ["Arrêté du 8 décembre 2021 portant fermeture… Voir l'article 1103 du Code civil."]})).await;
        assert_eq!(s, 200);
        assert_eq!(by(&b, "2020-1455")["ancrage"], "hors_sources");
        assert_eq!(by(&b, "2020-1455")["kind"], "texte");
        assert_eq!(by(&b, "8 décembre 2021")["ancrage"], "dans_sources");
        assert_eq!(by(&b, "1103")["ancrage"], "dans_sources");
        assert_eq!(by(&b, "1103")["statut"], "en_vigueur");

        // Clés précalculées (provenance d'un outil MCP / RAG) : le décret devient ancré.
        let (_, b) = post(json!({"texte": texte, "ancrage": true, "sources": ["x"],
            "cles_sources": ["txt|decret|n:2020-1455"]}))
        .await;
        assert_eq!(by(&b, "2020-1455")["ancrage"], "dans_sources");

        // Ancrage non demandé : rien.
        let (_, b) = post(json!({"texte": texte, "sources": ["Arrêté du 8 décembre 2021"]})).await;
        assert!(b["verifications"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["ancrage"].is_null()));

        // Ancrage demandé mais aucune source : pas de verdict « hors sources » (tout le serait).
        let (_, b) = post(json!({"texte": texte, "ancrage": true})).await;
        assert!(b["verifications"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["ancrage"].is_null()));

        // Sources trop volumineuses : ancrage désactivé plutôt que faux.
        let gros = "a".repeat(MAX_SOURCES_BYTES + 1);
        let (s, b) = post(json!({"texte": texte, "ancrage": true, "sources": [gros]})).await;
        assert_eq!(s, 200);
        assert!(b["verifications"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["ancrage"].is_null()));
    }

    #[tokio::test]
    async fn handler_text_only_answers_do_not_need_piste() {
        let api = spawn_api(JuridiqueState::new(None, None)).await; // PISTE non configuré
        let res = reqwest::Client::new()
            .post(format!("{api}/verifier-citations"))
            .json(&json!({"texte": "Le décret n° 2020-1455 du 27 novembre 2020.", "ancrage": true, "sources": ["autre chose"]}))
            .send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200);
        let b: Value = res.json().await.unwrap();
        assert_eq!(b["verifications"][0]["ancrage"], "hors_sources");
    }
}
