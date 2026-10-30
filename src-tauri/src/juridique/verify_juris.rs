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
// Juridix — Module Juridique — verify_juris.rs
// ============================================================================
// Vérification des décisions de justice citées AUTREMENT que par un numéro de
// pourvoi (celui-ci reste traité par `verify_ext.rs`) :
//
//   1. l'ECLI (« ECLI:FR:CCASS:2024:SO00123 ») : la juridiction se lit dans
//      l'ECLI lui-même ;
//   2. le Conseil d'État, cité par son numéro de requête
//      (« CE, 2 juillet 2020, n° 428411 ») ;
//   3. les cours administratives d'appel (« CAA Nantes, …, n° 19NT01234 ») et les
//      tribunaux administratifs (7 chiffres) ;
//   4. les cours d'appel judiciaires, citées par leur numéro de RG
//      (« CA Paris, 12 mars 2021, RG n° 19/01234 »).
//
// Sources interrogées (champs et filtres vérifiés dans les spécifications PISTE) :
//   - Cour de cassation et cours d'appel : Judilibre (`/search`, `ecli` et `number` figurent
//     dans les résultats) ;
//   - Conseil d'État, cours et tribunaux administratifs : Légifrance, fond CETAT. Ce fond
//     n'offre PAS les champs `NUM_AFFAIRE` ni `ECLI` (réservés à JURI) : le numéro se
//     cherche dans `NUM_DEC` (« numéros de décision »), puis en texte intégral. La décision
//     est reconnue d'après le TITRE du résultat (« Conseil d'État, 5ème - 6ème chambres
//     réunies, 02/07/2020, 428411 ») : juridiction, date et numéro doivent correspondre.
//     La lecture `/consult/juri` (documentée pour le seul fonds JURI) n'est qu'un
//     enrichissement facultatif : si elle réussit, elle doit CONFIRMER le numéro ou
//     l'ECLI, sinon la décision est écartée.
//
// Principes (les mêmes que le reste de la vérification) :
//   - jamais de verdict négatif sans recherche aboutie : « non retrouvée » (❓) n'est pas
//     « introuvable » (❌) ; ni Judilibre ni la jurisprudence administrative de Légifrance
//     ne sont exhaustifs ;
//   - jamais d'erreur présentée comme un verdict ;
//   - une référence n'est « vérifiée » que si une décision de MÊME numéro (ou de même ECLI)
//     a été effectivement retrouvée.
//
// Un numéro de RG se répète d'une cour d'appel à l'autre : il ne suffit pas à identifier un
// arrêt. La décision n'est retenue que si la date citée, ou le siège cité (résolu par la
// taxonomie `location` de Judilibre et appliqué comme FILTRE de recherche), la désigne ;
// sinon la référence reste « non retrouvée » avec l'explication.
// ============================================================================

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

use super::{
    client::PisteClient,
    judilibre::{self, JudilibreSearchParams},
    legifrance::{self, SearchFond},
    verify::fold,
    verify_ext::{
        digits_of, first_date, pick_decision_by_ecli, pick_decisions, str_field, DecisionInfo,
        DecisionLookup, DecisionRef,
    },
};

// ── Juridictions ──────────────────────────────────────────────────────────────

/// Juridiction d'une décision citée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Court {
    /// Cour de cassation (Judilibre).
    Cassation,
    /// Cour d'appel judiciaire (Judilibre).
    CourAppel,
    /// Conseil d'État (Légifrance, fond CETAT).
    ConseilEtat,
    /// Cour administrative d'appel (Légifrance, fond CETAT).
    CourAdminAppel,
    /// Tribunal administratif (Légifrance, fond CETAT).
    TribunalAdmin,
    /// Autre juridiction (Conseil constitutionnel, CJUE, tribunal judiciaire…) :
    /// reconnue, mais pas interrogée.
    Autre,
}

impl Court {
    pub fn label(self) -> &'static str {
        match self {
            Court::Cassation => "Cour de cassation",
            Court::CourAppel => "Cour d'appel",
            Court::ConseilEtat => "Conseil d'État",
            Court::CourAdminAppel => "Cour administrative d'appel",
            Court::TribunalAdmin => "Tribunal administratif",
            Court::Autre => "Juridiction non interrogée",
        }
    }

    /// Base interrogée pour cette juridiction.
    pub fn source(self) -> Option<&'static str> {
        match self {
            Court::Cassation | Court::CourAppel => Some("Judilibre"),
            Court::ConseilEtat | Court::CourAdminAppel | Court::TribunalAdmin => {
                Some("Légifrance (jurisprudence administrative)")
            }
            Court::Autre => None,
        }
    }

    pub fn is_admin(self) -> bool {
        matches!(
            self,
            Court::ConseilEtat | Court::CourAdminAppel | Court::TribunalAdmin
        )
    }
}

/// Juridiction désignée par un ECLI « ECLI:<pays>:<code juridiction>:… ».
/// Seuls les codes dont l'attribution est certaine sont classés ; tout le reste est
/// « Autre » (reconnu, non interrogé) plutôt que deviné.
pub fn court_of_ecli(country: &str, code: &str) -> Court {
    if !country.eq_ignore_ascii_case("FR") {
        return Court::Autre;
    }
    let c = code.to_ascii_uppercase();
    if c == "CCASS" {
        Court::Cassation
    } else if c.starts_with("CE") {
        Court::ConseilEtat // CE, CECHR, CESSR, CEASS, CEORD…
    } else if c.starts_with("CAA") {
        // « CAA… » : cour administrative d'appel — mais aussi une cour d'appel judiciaire
        // dont le siège commence par « A » (Agen, Aix, Amiens, Angers…) : la recherche
        // tente la jurisprudence administrative puis Judilibre.
        Court::CourAdminAppel
    } else if c.starts_with("CA") {
        Court::CourAppel
    } else if c.starts_with("TA") {
        Court::TribunalAdmin
    } else {
        Court::Autre
    }
}

/// Clé d'appariement (regroupement et ancrage) d'un numéro de décision. `None` si le
/// numéro n'a pas la forme attendue pour cette juridiction.
///
///   Cassation → « 2212345 » (forme historique, inchangée : `dec|2212345`) ;
///   CE → « ce:439953 » ; CAA / TA → « adm:19pa01234 » ; CA → « ca:1901234 ».
pub fn number_key(court: Court, number: &str) -> Option<String> {
    let alnum: String = number
        .split('/')
        .next()
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_uppercase();
    let digits = digits_of(number);
    match court {
        Court::Cassation => (digits.len() == 7).then_some(digits),
        Court::ConseilEtat => (matches!(digits.len(), 5 | 6) && alnum.len() == digits.len())
            .then(|| format!("ce:{digits}")),
        Court::CourAdminAppel => RE_CAA_SHAPE
            .is_match(&alnum)
            .then(|| format!("adm:{}", alnum.to_ascii_lowercase())),
        Court::TribunalAdmin => (alnum.len() == 7 && alnum.chars().all(|c| c.is_ascii_digit()))
            .then(|| format!("adm:{alnum}")),
        Court::CourAppel => {
            (digits.len() == 7 && number.contains('/')).then(|| format!("ca:{digits}"))
        }
        Court::Autre => None,
    }
}

static RE_CAA_SHAPE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\d{2}[A-Z]{2}\d{5}$").expect("regex forme CAA"));

// ── Ce que l'on demande à une source ──────────────────────────────────────────

/// Une décision à retrouver : tout ce que l'on sait d'elle d'après la citation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionQuery {
    pub court: Court,
    /// Numéro normalisé : chiffres du pourvoi (« 2212345 »), chiffres de la requête
    /// (« 439953 »), numéro alphanumérique d'une CAA en majuscules (« 19PA01234 »),
    /// chiffres d'un RG avec la barre (« 19/01234 »).
    pub number: Option<String>,
    /// Numéro tel qu'affiché (« 22-12.345 »).
    pub display: String,
    /// ECLI normalisé en majuscules.
    pub ecli: Option<String>,
    /// Date citée « AAAA-MM-JJ » (sert à départager des numéros de RG identiques).
    pub date: Option<String>,
    /// Siège cité (cour d'appel), tel qu'écrit.
    pub place: Option<String>,
}

impl DecisionQuery {
    /// Clé du pourvoi quand c'est un simple numéro de pourvoi (chemin historique).
    pub fn pourvoi_key(&self) -> Option<&str> {
        if self.court == Court::Cassation && self.ecli.is_none() {
            self.number.as_deref()
        } else {
            None
        }
    }

    /// Clé de cache : tout ce qui peut changer le résultat de la recherche.
    pub fn cache_key(&self) -> String {
        format!(
            "{:?}|{}|{}|{}|{}",
            self.court,
            self.number.as_deref().unwrap_or(""),
            self.ecli.as_deref().unwrap_or(""),
            // Seul le RG dépend de la date et du siège pour le résultat.
            if self.court == Court::CourAppel {
                self.date.as_deref().unwrap_or("")
            } else {
                ""
            },
            if self.court == Court::CourAppel {
                fold(self.place.as_deref().unwrap_or(""))
            } else {
                String::new()
            },
        )
    }
}

impl DecisionRef {
    /// Ce qu'il faut demander à la source pour retrouver cette décision.
    pub fn query(&self) -> DecisionQuery {
        DecisionQuery {
            court: self.court,
            number: self.number.clone(),
            display: self.display.clone(),
            ecli: self.ecli.clone(),
            date: self.date.clone(),
            place: self.place.clone(),
        }
    }
}

// ── Extraction ────────────────────────────────────────────────────────────────

/// ECLI : « ECLI:FR:CCASS:2024:SO00123 », « ECLI:FR:CECHR:2020:428411.20200702 ».
/// Forme officielle : pays (2 lettres) : code juridiction (≤ 9) : année : identifiant
/// (≤ 25 caractères alphanumériques et points).
///
/// Les modèles écrivent souvent à la française, avec une espace (ordinaire, insécable ou
/// fine insécable) avant et après chaque deux-points : « ECLI :FR :CCASS :2024 :SO00123 ».
static RE_ECLI: Lazy<Regex> = Lazy::new(|| {
    let sp = r"[ \x{A0}\x{202F}]?";
    Regex::new(&format!(
        r"(?i)\bECLI{sp}:{sp}(?P<country>[A-Z]{{2}}){sp}:{sp}(?P<court>[A-Z0-9]{{1,9}}){sp}:{sp}(?P<year>\d{{4}}){sp}:{sp}(?P<ord>[A-Z0-9][A-Z0-9.]{{0,24}})"
    ))
    .expect("regex ECLI")
});

// Conseil d'État : numéro de requête de 5 ou 6 chiffres précédé de « n° » / « req. n° ».
static RE_CE_NUM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:^|[^\d\-\x{2011}/])(?P<span>(?P<pre>(?:req(?:u[êe]te)?\.?\s*)?(?:n\s?[°º]|no\.)\s*)(?P<num>\d{5,6}))",
    )
    .expect("regex numéro CE")
});
static RE_CE_CUE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i:Conseil\s+d[’'´]\s?[ée]tat)").expect("regex cue CE"));
/// « CE » seul est aussi l'abréviation de « comité d'entreprise » : il ne suffit qu'avec
/// une date de décision dans la même citation.
static RE_CE_WEAK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bCE\b").expect("regex CE"));

// Cour administrative d'appel : 2 chiffres, 2 lettres (siège), 5 chiffres (« 19NT01234 »).
static RE_CAA_NUM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        // Les espaces entre les blocs (« 19 NT 01234 ») sont tolérées ; les lettres doivent
        // être des majuscules (sans quoi « 12 de 12345 » ressemblerait à un numéro).
        r"(?i)\b(?P<span>(?P<pre>(?:req(?:u[êe]te)?\.?\s*)?(?:n\s?[°º]|no\.)\s*)?(?P<num>\d{2}[ \x{A0}\x{202F}]?(?-i:[A-Z]{2})[ \x{A0}\x{202F}]?\d{5}))",
    )
    .expect("regex numéro CAA")
});
static RE_CAA_CUE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i:cour\s+administrative\s+d[’'´]appel)|\bCAA\b").expect("regex cue CAA")
});

// Tribunal administratif : 7 chiffres, avec ou sans le suffixe « /3-1 ».
static RE_TA_NUM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:^|[^\d\-\x{2011}/])(?P<span>(?P<pre>(?:req(?:u[êe]te)?\.?\s*)?(?:n\s?[°º]|no\.)\s*)(?P<num>\d{7})(?:/\d{1,2}(?:-\d{1,2})?)?)",
    )
    .expect("regex numéro TA")
});
static RE_TA_CUE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i:tribunal\s+administratif)|\bTA\b").expect("regex cue TA"));

// Cour d'appel judiciaire : numéro de RG « 19/01234 », précédé de « RG » ou « n° ».
static RE_CA_NUM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:^|[^\d/])(?P<span>(?P<pre>(?:n\s?[°º]\s*)?R\.?\s?G\.?\s*(?:n\s?[°º]\s*|:\s*)?|n\s?[°º]\s*|no\.\s*)(?P<num>\d{2}/\d{5}))",
    )
    .expect("regex numéro RG")
});
static RE_CA_CUE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i:cour\s+d[’'´]appel)|\bC\.\s?app\.").expect("regex cue CA"));
/// « CA » seul est aussi « chiffre d'affaires » : il ne suffit qu'avec une date.
static RE_CA_WEAK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bCA\b").expect("regex CA"));

static RE_PLACE_PREFIX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?i:(?:de\s+la\s+|de\s+l[’']\s*|de\s+|du\s+|d[’']\s*))").expect("regex siège")
});

struct Spec {
    court: Court,
    num: &'static Regex,
    cue: &'static Regex,
    /// Mention faible de la juridiction (abréviation ambiguë) : retenue seulement si la
    /// citation porte aussi une date.
    weak_cue: Option<&'static Regex>,
    /// Un numéro précédé de « n° » est retenu même sans mention de la juridiction
    /// (forme du numéro assez spécifique).
    pre_suffices: bool,
}

/// Le caractère qui suit le numéro ne doit pas le prolonger (« 123456-7 », « 2020-1455 »).
fn number_ends_here(masked: &str, end: usize) -> bool {
    let mut it = masked[end..].chars();
    match it.next() {
        None => true,
        Some(c) if c.is_ascii_alphanumeric() => false,
        Some('-') | Some('\u{2011}') | Some('/') => false,
        Some('.') => !it.next().is_some_and(|d| d.is_ascii_digit()),
        Some(_) => true,
    }
}

/// Siège cité après la mention de la juridiction (« CA Aix-en-Provence, 5e ch. »,
/// « cour d'appel de Paris, pôle 6 ») : les mots jusqu'à la première virgule, un
/// chiffre ou un mot de structure (pôle, chambre, section).
fn place_after(window: &str, cue_end: usize) -> Option<String> {
    let rest = window.get(cue_end..)?.trim_start();
    let rest = RE_PLACE_PREFIX.replace(rest, "");
    let head: &str = rest.split([',', ';', '(', ')']).next().unwrap_or("");
    let mut words: Vec<&str> = Vec::new();
    for w in head.split_whitespace() {
        let f = fold(w);
        let f = f.trim_end_matches('.');
        if w.chars().any(|c| c.is_ascii_digit())
            || matches!(
                f,
                "pole" | "ch" | "chambre" | "section" | "sect" | "ass" | "cabinet"
            )
        {
            break;
        }
        words.push(w);
        if words.len() == 3 {
            break;
        }
    }
    let place = words.join(" ");
    let place = place
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    (place.chars().next().is_some_and(char::is_uppercase) && place.chars().count() <= 40)
        .then_some(place)
}

fn scan(
    text: &str,
    masked: &str,
    spec: &Spec,
    spans: &mut Vec<(usize, usize)>,
) -> Vec<DecisionRef> {
    let mut out: Vec<DecisionRef> = Vec::new();
    for c in spec.num.captures_iter(masked) {
        let (Some(span), Some(num)) = (c.name("span"), c.name("num")) else {
            continue;
        };
        let pre = c.name("pre");
        let (num_start, end) = (span.start(), span.end());
        if !number_ends_here(masked, end) {
            continue;
        }
        if spans.iter().any(|&(s, e)| num_start < e && s < end) {
            continue;
        }
        // Fenêtre de contexte : même ligne, après la décision précédente (de n'importe
        // quelle nature) et le dernier « ; ».
        let line_start = masked[..num_start].rfind('\n').map_or(0, |i| i + 1);
        let prev_end = spans
            .iter()
            .filter(|&&(_, e)| e <= num_start)
            .map(|&(_, e)| e)
            .max()
            .unwrap_or(0);
        let mut w_start = num_start.saturating_sub(180).max(line_start).max(prev_end);
        while w_start < num_start && !masked.is_char_boundary(w_start) {
            w_start += 1;
        }
        if let Some(i) = masked[w_start..num_start].rfind(';') {
            w_start += i + 1;
        }
        let window = &masked[w_start..num_start];
        let date = first_date(window);
        let strong = spec.cue.find(window);
        let weak = spec
            .weak_cue
            .filter(|_| date.is_some())
            .and_then(|re| re.find(window));
        let cue = strong.or(weak);
        if cue.is_none() && !(spec.pre_suffices && pre.is_some()) {
            continue;
        }
        let court_start = cue.map(|m| w_start + m.start());
        let start = [
            court_start,
            date.as_ref().map(|(s, _, _)| w_start + *s),
            Some(num_start),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(num_start);
        if masked[start..end] != text[start..end] {
            continue;
        }

        let raw_num = num.as_str();
        let number = match spec.court {
            Court::ConseilEtat | Court::TribunalAdmin => raw_num.to_string(),
            Court::CourAdminAppel => raw_num
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_ascii_uppercase(),
            _ => raw_num.to_string(),
        };
        let Some(key) = number_key(spec.court, &number) else {
            continue;
        };
        let place = if spec.court == Court::CourAppel {
            cue.and_then(|m| place_after(window, m.end()))
        } else {
            None
        };
        spans.push((start, end));
        out.push(DecisionRef {
            start,
            end,
            raw: text[start..end].to_string(),
            key,
            display: number.clone(),
            chambre: None,
            date: date.map(|(_, _, d)| d),
            court: spec.court,
            number: Some(number),
            ecli: None,
            place,
        });
    }
    out
}

fn extract_ecli(text: &str, masked: &str, spans: &mut Vec<(usize, usize)>) -> Vec<DecisionRef> {
    let mut out = Vec::new();
    for c in RE_ECLI.captures_iter(masked) {
        let (Some(whole), Some(country), Some(code), Some(year), Some(ord)) = (
            c.get(0),
            c.name("country"),
            c.name("court"),
            c.name("year"),
            c.name("ord"),
        ) else {
            continue;
        };
        // Un point final appartient à la phrase, pas à l'identifiant.
        let ord_s = ord.as_str().trim_end_matches('.');
        if ord_s.is_empty() {
            continue;
        }
        let (start, end) = (whole.start(), ord.start() + ord_s.len());
        if masked[start..end] != text[start..end] {
            continue;
        }
        if spans.iter().any(|&(s, e)| start < e && s < end) {
            continue;
        }
        let ecli = format!(
            "ECLI:{}:{}:{}:{}",
            country.as_str().to_ascii_uppercase(),
            code.as_str().to_ascii_uppercase(),
            year.as_str(),
            ord_s.to_ascii_uppercase()
        );
        spans.push((start, end));
        out.push(DecisionRef {
            start,
            end,
            raw: text[start..end].to_string(),
            key: ecli.to_ascii_lowercase(),
            display: ecli.clone(),
            chambre: None,
            date: None,
            court: court_of_ecli(country.as_str(), code.as_str()),
            number: None,
            ecli: Some(ecli),
            place: None,
        });
    }
    out
}

/// Décisions citées autrement que par un numéro de pourvoi : ECLI, Conseil d'État,
/// juridictions administratives, cours d'appel. `taken` : zones déjà prises par les
/// décisions de la Cour de cassation (aucun chevauchement).
pub(crate) fn extract_other_decisions(
    text: &str,
    masked: &str,
    taken: &[(usize, usize)],
) -> Vec<DecisionRef> {
    let mut spans: Vec<(usize, usize)> = taken.to_vec();
    let mut out = extract_ecli(text, masked, &mut spans);
    let specs = [
        Spec {
            court: Court::ConseilEtat,
            num: &RE_CE_NUM,
            cue: &RE_CE_CUE,
            weak_cue: Some(&RE_CE_WEAK),
            pre_suffices: false,
        },
        Spec {
            court: Court::CourAdminAppel,
            num: &RE_CAA_NUM,
            cue: &RE_CAA_CUE,
            weak_cue: None,
            pre_suffices: true,
        },
        Spec {
            court: Court::TribunalAdmin,
            num: &RE_TA_NUM,
            cue: &RE_TA_CUE,
            weak_cue: None,
            pre_suffices: false,
        },
        Spec {
            court: Court::CourAppel,
            num: &RE_CA_NUM,
            cue: &RE_CA_CUE,
            weak_cue: Some(&RE_CA_WEAK),
            pre_suffices: false,
        },
    ];
    for spec in &specs {
        out.extend(scan(text, masked, spec, &mut spans));
    }
    out.sort_by_key(|d| d.start);
    out
}

/// Extraction directe (tests et appelants qui n'ont pas déjà masqué le texte).
#[cfg(test)]
pub(crate) fn extract_others(text: &str) -> Vec<DecisionRef> {
    extract_other_decisions(text, &super::verify::mask_non_prose(text), &[])
}

// ── Dates renvoyées par Légifrance ────────────────────────────────────────────

/// « AAAA-MM-JJ » d'une date Légifrance : texte ISO (« 2020-07-02T00:00:00.000+0000 »),
/// « 02/07/2020 » ou nombre de millisecondes depuis 1970 (forme fréquente de l'API).
pub(crate) fn date_of_value(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => date_of_str(s),
        Value::Number(n) => n.as_i64().and_then(date_of_millis),
        _ => None,
    }
}

fn date_of_str(s: &str) -> Option<String> {
    static RE_ISO: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})").expect("regex ISO"));
    static RE_DMY: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^(\d{2})/(\d{2})/(\d{4})").expect("regex JJ/MM/AAAA"));
    let s = s.trim();
    if let Some(c) = RE_ISO.captures(s) {
        return Some(format!("{}-{}-{}", &c[1], &c[2], &c[3]));
    }
    if let Some(c) = RE_DMY.captures(s) {
        return Some(format!("{}-{}-{}", &c[3], &c[2], &c[1]));
    }
    if s.len() >= 10 && s.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return s.parse::<i64>().ok().and_then(date_of_millis);
    }
    None
}

/// Légifrance date les décisions à minuit heure de Paris, soit la veille à 22 h ou 23 h
/// en UTC : on ajoute 12 h avant de lire la date UTC pour retrouver le bon jour.
fn date_of_millis(ms: i64) -> Option<String> {
    let shifted = ms.checked_add(12 * 3600 * 1000)?;
    chrono::DateTime::from_timestamp_millis(shifted)
        .map(|d| d.naive_utc().date().format("%Y-%m-%d").to_string())
}

// ── Lecture d'une décision Légifrance (`/consult/juri`) ───────────────────────

/// Ce que l'on lit d'une décision de la jurisprudence administrative (schéma
/// `TexteSimple` du swagger Légifrance).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JuriDoc {
    pub juridiction: Option<String>,
    pub formation: Option<String>,
    pub solution: Option<String>,
    pub titre: Option<String>,
    pub numeros: Vec<String>,
    pub ecli: Option<String>,
    /// « AAAA-MM-JJ »
    pub date: Option<String>,
}

/// `None` si la réponse n'a pas la forme attendue (ni numéro d'affaire, ni ECLI, ni titre).
pub(crate) fn parse_juri(v: &Value) -> Option<JuriDoc> {
    let t = v.get("text").unwrap_or(v);
    let numeros: Vec<String> = match t.get("numeroAffaire") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => Vec::new(),
    };
    let doc = JuriDoc {
        juridiction: str_field(t, "juridiction"),
        formation: str_field(t, "formation"),
        solution: str_field(t, "solution"),
        titre: str_field(t, "titre"),
        numeros,
        ecli: str_field(t, "ecli"),
        date: ["dateTexte", "dateDecision", "datePubli"]
            .iter()
            .find_map(|k| t.get(*k).and_then(date_of_value)),
    };
    (!doc.numeros.is_empty() || doc.ecli.is_some() || doc.titre.is_some()).then_some(doc)
}

/// Une décision candidate d'une recherche CETAT : identifiant, titre et date du résultat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub id: String,
    pub title: Option<String>,
    /// Date « utile » du résultat (`date`), « AAAA-MM-JJ ».
    pub date: Option<String>,
}

/// Candidates d'une réponse de recherche CETAT (`results[].titles[]`), sans doublon,
/// plafonnées.
pub(crate) fn cetat_candidates_of(resp: &Value) -> Vec<Candidate> {
    const MAX_CANDIDATES: usize = 5;
    let mut out: Vec<Candidate> = Vec::new();
    for r in resp
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let result_date = r.get("date").and_then(date_of_value);
        for t in r
            .get("titles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = str_field(t, "id")
                .or_else(|| str_field(t, "cid"))
                .filter(|i| i.starts_with("CETATEXT"))
            else {
                continue;
            };
            if out.iter().all(|c| c.id != id) && out.len() < MAX_CANDIDATES {
                out.push(Candidate {
                    id,
                    title: str_field(t, "title"),
                    date: result_date.clone(),
                });
            }
        }
    }
    out
}

/// Ce que dit le titre d'un résultat CETAT :
/// « Conseil d'État, 5ème - 6ème chambres réunies, 02/07/2020, 428411, Publié au recueil Lebon ».
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TitleInfo {
    pub juridiction: Option<String>,
    pub formation: Option<String>,
    /// « AAAA-MM-JJ »
    pub date: Option<String>,
    pub numeros: Vec<String>,
}

pub(crate) fn parse_title(title: &str) -> TitleInfo {
    static RE_DMY: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^\d{2}/\d{2}/\d{4}$").expect("regex date titre"));
    static RE_NUM: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^(?:\d{2}[A-Z]{2}\d{5}|\d{5,7}(?:/\d{1,2}(?:-\d{1,2})?)?)$")
            .expect("regex numéro titre")
    });
    let segs: Vec<&str> = title.split(',').map(str::trim).collect();
    let Some(i) = segs.iter().position(|s| RE_DMY.is_match(s)) else {
        return TitleInfo::default();
    };
    let numeros = segs[i + 1..]
        .iter()
        .take_while(|s| RE_NUM.is_match(s))
        .map(|s| s.to_string())
        .collect();
    TitleInfo {
        juridiction: (i > 0)
            .then(|| segs[0].to_string())
            .filter(|s| !s.is_empty()),
        formation: (i > 1)
            .then(|| segs[1..i].join(", "))
            .filter(|s| !s.is_empty()),
        date: date_of_str(segs[i]),
        numeros,
    }
}

/// Numéro et date d'un ECLI administratif « ECLI:FR:CECHR:2020:428411.20200702 » : le
/// dernier élément est « <numéro>.<AAAAMMJJ> ».
pub(crate) fn ecli_number_and_date(ecli: &str) -> Option<(String, String)> {
    static RE_ORD: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^(?P<n>[0-9A-Z]{5,12})\.(?P<y>\d{4})(?P<m>\d{2})(?P<d>\d{2})$")
            .expect("regex identifiant ECLI")
    });
    let ord = ecli.split(':').nth(4)?.to_ascii_uppercase();
    let c = RE_ORD.captures(&ord)?;
    Some((
        c["n"].to_string(),
        format!("{}-{}-{}", &c["y"], &c["m"], &c["d"]),
    ))
}

/// Le titre du résultat désigne-t-il la décision cherchée ? Renvoie le numéro retrouvé.
///   - citée par numéro : juridiction de la classe citée et numéro identique ;
///   - citée par ECLI : même juridiction, même numéro ET même date que ceux que l'ECLI
///     porte dans son identifiant.
pub(crate) fn title_matches(t: &TitleInfo, q: &DecisionQuery) -> Option<String> {
    let juridiction = t.juridiction.as_deref()?;
    if !court_matches(q.court, juridiction) {
        return None;
    }
    let (wanted, wanted_date) = match (&q.ecli, &q.number) {
        (Some(e), _) => {
            let (n, d) = ecli_number_and_date(e)?;
            (n, Some(d))
        }
        (None, Some(n)) => (norm_case_number(n), None),
        _ => return None,
    };
    if let Some(d) = wanted_date {
        if t.date.as_deref() != Some(d.as_str()) {
            return None;
        }
    }
    t.numeros
        .iter()
        .find(|n| norm_case_number(n) == wanted)
        .cloned()
}

/// Numéro d'affaire comparable : majuscules alphanumériques, suffixe « /… » écarté
/// (« 2012345/3-1 » → « 2012345 »).
fn norm_case_number(s: &str) -> String {
    s.split('/')
        .next()
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_uppercase()
}

/// La juridiction indiquée par Légifrance est-elle celle de la citation ?
fn court_matches(court: Court, juridiction: &str) -> bool {
    let f = fold(juridiction);
    match court {
        Court::ConseilEtat => f.contains("conseil d'etat"),
        Court::CourAdminAppel => f.contains("cour administrative d'appel") || f.starts_with("caa"),
        Court::TribunalAdmin => f.contains("tribunal administratif") || f.starts_with("ta "),
        _ => true,
    }
}

/// La décision lue est-elle bien celle que l'on cherche ?
///   - citée par ECLI : l'ECLI doit être identique (c'est un identifiant unique, la
///     juridiction n'est pas recontrôlée) ;
///   - citée par son numéro : le numéro d'affaire doit figurer parmi ceux de la décision,
///     et la juridiction doit être celle citée (quand Légifrance l'indique).
pub(crate) fn doc_matches(doc: &JuriDoc, q: &DecisionQuery) -> Option<String> {
    if let Some(e) = &q.ecli {
        return doc
            .ecli
            .as_deref()
            .filter(|x| x.trim().eq_ignore_ascii_case(e))
            .map(|x| x.trim().to_string());
    }
    let wanted = q.number.as_deref().map(norm_case_number)?;
    if let Some(j) = &doc.juridiction {
        if !court_matches(q.court, j) {
            return None;
        }
    }
    doc.numeros
        .iter()
        .find(|n| norm_case_number(n) == wanted)
        .cloned()
}

fn publication_from_titre(titre: &str) -> Vec<String> {
    let f = fold(titre);
    if f.contains("publie au recueil lebon") {
        vec!["Publié au recueil Lebon".to_string()]
    } else if f.contains("mentionne dans les tables") {
        vec!["Mentionné dans les tables du recueil Lebon".to_string()]
    } else if f.contains("inedit au recueil lebon") {
        vec!["Inédit au recueil Lebon".to_string()]
    } else {
        Vec::new()
    }
}

/// Fiche d'une décision CETAT : le titre du résultat de recherche, complété par la lecture
/// de la décision quand elle a pu être faite.
pub(crate) fn info_from_cetat(
    cand: &Candidate,
    title: &TitleInfo,
    doc: Option<&JuriDoc>,
    number: String,
    note: Option<String>,
) -> DecisionInfo {
    let publication = cand
        .title
        .as_deref()
        .or_else(|| doc.and_then(|d| d.titre.as_deref()))
        .map(publication_from_titre)
        .unwrap_or_default();
    DecisionInfo {
        id: cand.id.clone(),
        number,
        chamber: doc
            .and_then(|d| d.formation.clone())
            .or_else(|| title.formation.clone()),
        jurisdiction: doc
            .and_then(|d| d.juridiction.clone())
            .or_else(|| title.juridiction.clone()),
        formation: None,
        date: title
            .date
            .clone()
            .or_else(|| doc.and_then(|d| d.date.clone()))
            .or_else(|| cand.date.clone()),
        solution: doc.and_then(|d| d.solution.clone()),
        publication,
        ecli: doc.and_then(|d| d.ecli.clone()),
        source: Some("Légifrance".to_string()),
        url: Some(format!(
            "https://www.legifrance.gouv.fr/ceta/id/{}",
            cand.id
        )),
        note,
    }
}

// ── Arrêts de cours d'appel : départager des numéros de RG identiques ─────────

fn fr(iso: &str) -> String {
    let h = iso.get(..10).unwrap_or(iso);
    let mut it = h.split('-');
    match (it.next(), it.next(), it.next()) {
        (Some(y), Some(m), Some(d)) => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}

/// Arrêts de cour d'appel de même numéro de RG → verdict. Le RG se répète d'une cour à
/// l'autre : un arrêt n'est retenu que si la date citée le désigne, ou si le siège cité
/// (appliqué comme filtre de recherche) le désigne. Sinon la référence reste « non
/// retrouvée », avec la raison.
///
///   - `all` : arrêts de ce numéro, toutes cours confondues ;
///   - `at_seat` : les mêmes, recherchés dans le seul siège cité — `None` si le siège n'est
///     pas cité ou n'a pas pu être identifié dans la taxonomie.
pub(crate) fn narrow_ca(
    all: Vec<DecisionInfo>,
    at_seat: Option<Vec<DecisionInfo>>,
    date: Option<&str>,
) -> DecisionLookup {
    if all.is_empty() {
        return DecisionLookup::Introuvable;
    }
    let (list, seat_confirmed) = match at_seat {
        Some(s) if !s.is_empty() => (s, true),
        Some(_) => {
            return DecisionLookup::Incertaine {
                raison: "Judilibre contient des arrêts de ce numéro de RG, mais d'une autre cour d'appel que celle citée. À vérifier.".to_string(),
            }
        }
        None => (all, false),
    };
    let list: Vec<DecisionInfo> = match date {
        Some(d) => list
            .into_iter()
            .filter(|c| c.date.as_deref().and_then(|x| x.get(..10)) == Some(d))
            .collect(),
        None => list,
    };
    if list.is_empty() {
        return DecisionLookup::Incertaine {
            raison: format!(
                "Judilibre contient des arrêts de ce numéro de RG, mais aucun daté du {} (date inexacte, ou autre cour d'appel : le RG se répète d'une cour à l'autre). À vérifier.",
                fr(date.unwrap_or_default())
            ),
        };
    }
    if date.is_some() || seat_confirmed {
        return DecisionLookup::Trouvee(list.into_iter().next().expect("liste non vide"));
    }
    DecisionLookup::Incertaine {
        raison: "Un numéro de RG se répète d'une cour d'appel à l'autre : Judilibre contient des arrêts de ce numéro, mais ni la date ni la cour citées ne permettent d'en désigner un. À vérifier.".to_string(),
    }
}

// ── Interrogation des sources ─────────────────────────────────────────────────

/// ECLI → décision Judilibre (Cour de cassation `cc` ou cours d'appel `ca`).
/// `Ok(None)` : recherche aboutie sans décision de cet ECLI.
///
/// Sans `field`, Judilibre exclut les métadonnées de la recherche : on cible d'abord le
/// champ « ECLI » de la taxonomie, puis le champ « numéro », puis le texte intégral. Dans
/// tous les cas la décision n'est retenue que si SON `ecli` est identique à celui cité.
pub(crate) async fn judilibre_by_ecli(
    client: &PisteClient,
    ecli: &str,
    jurisdiction: &str,
    fields: Vec<String>,
) -> Result<Option<DecisionInfo>, String> {
    let mut attempts: Vec<Option<Vec<String>>> =
        fields.into_iter().map(|f| Some(vec![f])).collect();
    attempts.push(None);
    for field in attempts {
        let params = JudilibreSearchParams {
            query: ecli.to_string(),
            operator: Some("exact".into()),
            field,
            jurisdiction: Some(vec![jurisdiction.to_string()]),
            batch: 0,
            batch_size: 10,
            resolve_references: true,
            ..Default::default()
        };
        let resp = judilibre::search(client, params)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(info) = pick_decision_by_ecli(&resp.results.unwrap_or_default(), ecli) {
            return Ok(Some(info));
        }
    }
    Ok(None)
}

/// Numéro de RG → arrêts de cours d'appel de Judilibre portant ce numéro. `location` :
/// clé du siège (taxonomie `location`, contexte `ca`) pour restreindre la recherche à une cour.
pub(crate) async fn judilibre_by_rg(
    client: &PisteClient,
    key: &str,
    display: &str,
    number_field: Option<String>,
    location: Option<String>,
) -> Result<Vec<DecisionInfo>, String> {
    let mut attempts: Vec<Option<Vec<String>>> = Vec::new();
    if let Some(f) = number_field {
        attempts.push(Some(vec![f]));
    }
    attempts.push(None);
    for field in attempts {
        let params = JudilibreSearchParams {
            query: display.to_string(),
            operator: Some("exact".into()),
            field,
            jurisdiction: Some(vec!["ca".to_string()]),
            location: location.clone().map(|l| vec![l]),
            batch: 0,
            batch_size: 10,
            resolve_references: true,
            ..Default::default()
        };
        let resp = judilibre::search(client, params)
            .await
            .map_err(|e| e.to_string())?;
        let found = pick_decisions(&resp.results.unwrap_or_default(), key);
        if !found.is_empty() {
            return Ok(found);
        }
    }
    Ok(Vec::new())
}

/// Couples (clé, intitulé) d'une réponse `/taxonomy` : table { clé: intitulé }, liste
/// [{key, value}] ou élément unique.
pub(crate) fn taxonomy_entries(v: &Value) -> Vec<(String, String)> {
    let result = v.get("result").unwrap_or(v);
    let mut out = Vec::new();
    match result {
        Value::Object(map) if map.contains_key("key") && map.contains_key("value") => {
            if let (Some(k), Some(l)) = (str_field(result, "key"), str_field(result, "value")) {
                out.push((k, l));
            }
        }
        Value::Object(map) => {
            for (k, label) in map {
                if let Some(l) = label.as_str() {
                    out.push((k.clone(), l.to_string()));
                }
            }
        }
        Value::Array(items) => {
            for it in items {
                if let (Some(k), Some(l)) = (str_field(it, "key"), str_field(it, "value")) {
                    out.push((k, l));
                }
            }
        }
        _ => {}
    }
    out
}

/// Clé du siège d'une cour d'appel dans la taxonomie Judilibre `location` : l'entrée dont
/// l'intitulé est le siège cité (« Paris », « Cour d'appel de Paris »). `None` si aucune
/// entrée, ou plusieurs, correspondent : le siège n'est alors pas utilisé.
pub(crate) fn location_key_for(entries: &[(String, String)], place: &str) -> Option<String> {
    let wanted = fold(place).replace('-', " ");
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    let hits: Vec<&(String, String)> = entries
        .iter()
        .filter(|(_, label)| {
            let l = fold(label).replace('-', " ");
            l == wanted || l.ends_with(&format!(" {wanted}")) || l.ends_with(&format!("'{wanted}"))
        })
        .collect();
    (hits.len() == 1).then(|| hits[0].0.clone())
}

/// Recherche CETAT d'une valeur, dans un champ ou en texte intégral. Un champ refusé par
/// l'API (HTTP 400) n'est pas une erreur : `None`, on passe à la tentative suivante.
async fn cetat_search(
    client: &PisteClient,
    value: &str,
    champ: Option<&str>,
) -> Result<Option<Vec<Candidate>>, String> {
    let res = legifrance::search_simple(
        client,
        value,
        SearchFond::Cetat,
        1,
        10,
        None,
        None,
        None,
        None,
        champ.map(str::to_string),
        None,
        None,
    )
    .await;
    match res {
        Ok(resp) => {
            let v = serde_json::to_value(resp).map_err(|e| e.to_string())?;
            Ok(Some(cetat_candidates_of(&v)))
        }
        Err(e) if e.to_string().contains("HTTP 400") => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Conseil d'État, cours et tribunaux administratifs : numéro de requête ou ECLI →
/// décision CETAT. `Introuvable` seulement si la recherche a abouti sans qu'aucun résultat
/// ne porte le numéro (et, pour un ECLI, la date) cité.
pub(crate) async fn legifrance_juri(
    client: &PisteClient,
    q: &DecisionQuery,
) -> Result<DecisionLookup, String> {
    // Valeur cherchée : le numéro cité, ou celui que l'ECLI porte dans son identifiant.
    let value = match (&q.ecli, &q.number) {
        (Some(e), _) => ecli_number_and_date(e)
            .map(|(n, _)| n)
            .unwrap_or_else(|| e.clone()),
        (None, Some(n)) => n.clone(),
        _ => return Ok(DecisionLookup::Introuvable),
    };
    let mut verified: Vec<DecisionInfo> = Vec::new();
    for champ in [Some("NUM_DEC"), None] {
        let Some(cands) = cetat_search(client, &value, champ).await? else {
            continue;
        };
        for cand in cands {
            let title = cand.title.as_deref().map(parse_title).unwrap_or_default();
            let by_title = title_matches(&title, q);
            // Un titre exploitable qui ne correspond pas écarte le candidat sans autre appel.
            if by_title.is_none() && cand.title.is_some() {
                continue;
            }
            // Lecture facultative : elle enrichit, et doit confirmer (jamais contredire).
            let doc = match legifrance::get_juri(client, &cand.id).await {
                Ok(v) => parse_juri(&v),
                Err(_) => None,
            };
            let (number, note) = match (&doc, by_title) {
                (Some(d), _) => match doc_matches(d, q) {
                    Some(n) => (n, None),
                    None => continue, // la décision lue contredit le titre : écartée
                },
                (None, Some(n)) => (
                    n,
                    q.ecli.as_ref().map(|_| {
                        "ECLI rapproché du numéro et de la date qu'il porte (juridiction, numéro et date identiques) ; la décision n'a pas pu être lue pour comparer l'ECLI lui-même."
                            .to_string()
                    }),
                ),
                (None, None) => continue,
            };
            verified.push(info_from_cetat(&cand, &title, doc.as_ref(), number, note));
        }
        if !verified.is_empty() {
            break;
        }
    }
    // Plusieurs décisions de même numéro (ordonnance puis décision…) : celle de la date citée.
    let pick = q
        .date
        .as_deref()
        .and_then(|d| {
            verified
                .iter()
                .position(|i| i.date.as_deref().and_then(|x| x.get(..10)) == Some(d))
        })
        .or(if verified.is_empty() { None } else { Some(0) });
    Ok(match pick {
        Some(i) => DecisionLookup::Trouvee(verified.swap_remove(i)),
        None => DecisionLookup::Introuvable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn french_typography_around_colons_is_tolerated_in_an_ecli() {
        // Forme réellement produite par un modèle : « ECLI :FR :CCASS :1999 :SO99999 ».
        for t in [
            "ECLI :FR :CCASS :1999 :SO99999",
            "ECLI\u{a0}:FR\u{a0}:CCASS\u{a0}:1999\u{a0}:SO99999",
            "ECLI\u{202f}: FR\u{202f}: CCASS\u{202f}: 1999\u{202f}: SO99999",
            "ECLI: FR: CCASS: 1999: SO99999",
        ] {
            let d = one(&format!("Voir {t}."));
            assert_eq!(d.court, Court::Cassation, "{t:?}");
            assert_eq!(
                d.ecli.as_deref(),
                Some("ECLI:FR:CCASS:1999:SO99999"),
                "{t:?}"
            );
            assert_eq!(d.key, "ecli:fr:ccass:1999:so99999");
        }
        let d = one("ECLI :EU :C :2020 :559");
        assert_eq!(
            (d.court, d.ecli.as_deref()),
            (Court::Autre, Some("ECLI:EU:C:2020:559"))
        );
        // Texte courant : « ECLI : » seul n'est pas un ECLI.
        assert!(extract_others("L'ECLI : identifiant européen de la jurisprudence").is_empty());
    }

    #[test]
    fn a_spaced_administrative_appeal_number_is_normalised() {
        let d = one("Cour administrative d'appel de Nantes – 12 mars 2021 – n° 19 NT 99999");
        assert_eq!(d.number.as_deref(), Some("19NT99999"));
        assert_eq!(d.key, "adm:19nt99999");
        let d = one("CAA Nantes, 12 mars 2021, n° 19NT 01234");
        assert_eq!(d.number.as_deref(), Some("19NT01234"));
        assert!(extract_others("CAA : voir 12 de 12345 et 12 ab 12345").is_empty());
    }

    #[test]
    fn the_five_references_of_a_real_run_are_all_extracted() {
        // Recopie d'un modèle (tirets demi-cadratins, espaces avant les deux-points).
        let t = "* Conseil d’État – 2 juillet 2020 – n° 999998\n\
                 * Cour administrative d’appel de Nantes – 12 mars 2021 – n° 19 NT 99999\n\
                 * Cour d’appel de Paris – 12 mars 2021 – RG n° 99/99999\n\
                 * ECLI :FR :CCASS :1999 :SO99999\n\
                 * ECLI :EU :C :2020 :559";
        let d = crate::juridique::verify_ext::extract_decision_refs(t);
        let k: Vec<&str> = d.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(
            k,
            [
                "ce:999998",
                "adm:19nt99999",
                "ca:9999999",
                "ecli:fr:ccass:1999:so99999",
                "ecli:eu:c:2020:559"
            ]
        );
    }

    fn one(t: &str) -> DecisionRef {
        let d = extract_others(t);
        assert_eq!(d.len(), 1, "{t} → {d:#?}");
        d.into_iter().next().unwrap()
    }

    #[test]
    fn ecli_is_extracted_and_its_court_is_read_from_it() {
        for (t, court, ecli) in [
            (
                "ECLI:FR:CCASS:2024:SO00123",
                Court::Cassation,
                "ECLI:FR:CCASS:2024:SO00123",
            ),
            (
                "ECLI:FR:CECHR:2020:428411.20200702",
                Court::ConseilEtat,
                "ECLI:FR:CECHR:2020:428411.20200702",
            ),
            (
                "ECLI:FR:CAANT:2021:19NT01234.20210312",
                Court::CourAdminAppel,
                "ECLI:FR:CAANT:2021:19NT01234.20210312",
            ),
            (
                "ECLI:FR:CAPARIS:2021:A1234",
                Court::CourAppel,
                "ECLI:FR:CAPARIS:2021:A1234",
            ),
            (
                "ECLI:FR:TACRE:2022:2012345.20220301",
                Court::TribunalAdmin,
                "ECLI:FR:TACRE:2022:2012345.20220301",
            ),
            (
                "ECLI:FR:CC:2020:2020.800.DC",
                Court::Autre,
                "ECLI:FR:CC:2020:2020.800.DC",
            ),
            ("ECLI:EU:C:2020:559", Court::Autre, "ECLI:EU:C:2020:559"),
            (
                "ecli:fr:ccass:2024:so00123",
                Court::Cassation,
                "ECLI:FR:CCASS:2024:SO00123",
            ),
        ] {
            let d = one(&format!("Voir {t}."));
            assert_eq!(d.court, court, "{t}");
            assert_eq!(d.ecli.as_deref(), Some(ecli), "{t}");
            assert!(
                d.raw.ends_with(&t[t.len() - 3..]),
                "le point final n'en fait pas partie : {}",
                d.raw
            );
        }
    }

    #[test]
    fn ecli_in_code_or_links_is_ignored_and_malformed_is_not_taken() {
        assert!(extract_others("`ECLI:FR:CCASS:2024:SO00123`").is_empty());
        assert!(extract_others("[x](https://e.org/ECLI:FR:CCASS:2024:SO00123)").is_empty());
        assert!(extract_others("ECLI:FRANCE:CCASS:2024:SO1").is_empty());
        assert!(extract_others("ECLI:FR:CCASS:24:SO1").is_empty());
    }

    #[test]
    fn conseil_d_etat_is_extracted_with_date() {
        let t = "Voir CE, 5e et 6e ch. réunies, 2 juillet 2020, n° 428411, Lebon.";
        let d = one(t);
        assert_eq!(d.court, Court::ConseilEtat);
        assert_eq!(d.number.as_deref(), Some("428411"));
        assert_eq!(d.key, "ce:428411");
        assert_eq!(d.date.as_deref(), Some("2020-07-02"));
        assert_eq!(&t[d.start..d.end], d.raw);
        assert!(d.raw.starts_with("CE,") && d.raw.ends_with("428411"));
        let d = one("Le Conseil d'État, 12 mars 2021, req. n° 439953 a jugé");
        assert_eq!(d.key, "ce:439953");
        let d = one("Conseil d’Etat, sect., requête n° 12345");
        assert_eq!(d.key, "ce:12345");
    }

    #[test]
    fn ce_alone_is_not_enough_without_a_date() {
        // « CE » = comité d'entreprise : pas de décision sans date.
        assert!(extract_others("Le CE a rendu l'avis n° 123456 hier").is_empty());
        assert!(extract_others("ce jugement, n° 123456").is_empty());
        assert_eq!(one("CE, 2 juillet 2020, n° 428411").key, "ce:428411");
    }

    #[test]
    fn other_number_series_are_not_taken_for_decisions() {
        for t in [
            "le décret n° 2020-1455 du 27 novembre 2020 (Conseil d'État)",
            "la loi n° 78-17 du 6 janvier 1978, CE, 2 juillet 2020",
            "Conseil d'État, avis n° 123456-7",
            "Conseil d'État, 2020, n° 1234",
            "Conseil d'État, le 12/03/2024, 428411",
        ] {
            assert!(extract_others(t).is_empty(), "{t}");
        }
    }

    #[test]
    fn administrative_courts_are_extracted() {
        let d = one("CAA Nantes, 12 mars 2021, n° 19NT01234");
        assert_eq!(
            (d.court, d.key.as_str()),
            (Court::CourAdminAppel, "adm:19nt01234")
        );
        assert_eq!(d.number.as_deref(), Some("19NT01234"));
        assert_eq!(d.date.as_deref(), Some("2021-03-12"));
        // Forme très spécifique : retenue avec « n° » même sans mention de la cour.
        assert_eq!(one("l'arrêt n° 20MA00456").court, Court::CourAdminAppel);
        assert!(extract_others("référence 20MA00456 seule").is_empty());
        let d = one("TA Lyon, 3 mars 2022, n° 2012345/3-1");
        assert_eq!(
            (d.court, d.key.as_str()),
            (Court::TribunalAdmin, "adm:2012345")
        );
        assert!(d.raw.ends_with("2012345/3-1"));
        assert!(extract_others("n° 2012345 du dossier").is_empty());
    }

    #[test]
    fn appeal_courts_are_extracted_by_rg_with_their_seat() {
        let d = one("CA Paris, pôle 6, ch. 2, 12 mars 2021, RG n° 19/01234");
        assert_eq!(d.court, Court::CourAppel);
        assert_eq!(d.number.as_deref(), Some("19/01234"));
        assert_eq!(d.key, "ca:1901234");
        assert_eq!(d.place.as_deref(), Some("Paris"));
        assert_eq!(d.date.as_deref(), Some("2021-03-12"));
        let d = one("Cour d'appel d'Aix-en-Provence, 5 mai 2022, n° RG : 20/04567");
        assert_eq!(d.place.as_deref(), Some("Aix-en-Provence"));
        assert_eq!(d.key, "ca:2004567");
        assert_eq!(
            one("cour d'appel de Lyon, RG 21/00123").place.as_deref(),
            Some("Lyon")
        );
        // Une date n'est pas un RG ; « CA » seul (chiffre d'affaires) sans date non plus.
        assert!(extract_others("le 12/03/2024, cour d'appel de Paris").is_empty());
        assert!(extract_others("Le CA a progressé, RG n° 19/01234").is_empty());
        assert_eq!(
            one("CA Paris, 12 mars 2021, RG n° 19/01234").court,
            Court::CourAppel
        );
    }

    #[test]
    fn mixed_decisions_keep_their_own_context_and_never_overlap() {
        let t = "Cass. soc., 12 mars 2024, n° 22-12.345 ; CE, 2 juillet 2020, n° 428411 ; CA Paris, 1er juin 2021, RG n° 19/01234 ; ECLI:FR:CCASS:2024:SO00123.";
        let d = crate::juridique::verify_ext::extract_decision_refs(t);
        let k: Vec<&str> = d.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(
            k,
            [
                "2212345",
                "ce:428411",
                "ca:1901234",
                "ecli:fr:ccass:2024:so00123"
            ]
        );
        for w in d.windows(2) {
            assert!(w[0].end <= w[1].start, "{d:#?}");
        }
        assert_eq!(d[0].court, Court::Cassation);
        assert_eq!(d[1].date.as_deref(), Some("2020-07-02"));
        assert_eq!(d[2].date.as_deref(), Some("2021-06-01"));
    }

    #[test]
    fn number_keys_per_court() {
        assert_eq!(
            number_key(Court::Cassation, "22-12.345").as_deref(),
            Some("2212345")
        );
        assert_eq!(
            number_key(Court::ConseilEtat, "428411").as_deref(),
            Some("ce:428411")
        );
        assert_eq!(
            number_key(Court::CourAdminAppel, "19nt01234").as_deref(),
            Some("adm:19nt01234")
        );
        assert_eq!(
            number_key(Court::TribunalAdmin, "2012345/3-1").as_deref(),
            Some("adm:2012345")
        );
        assert_eq!(
            number_key(Court::CourAppel, "19/01234").as_deref(),
            Some("ca:1901234")
        );
        assert_eq!(number_key(Court::CourAppel, "1901234"), None);
        assert_eq!(number_key(Court::Autre, "1"), None);
    }

    #[test]
    fn legifrance_dates_come_in_several_shapes() {
        assert_eq!(
            date_of_value(&json!("2020-07-02T00:00:00.000+0000")).as_deref(),
            Some("2020-07-02")
        );
        assert_eq!(
            date_of_value(&json!("02/07/2020")).as_deref(),
            Some("2020-07-02")
        );
        // Minuit heure de Paris (UTC+2) = 22 h la veille en UTC : le jour reste le 2 juillet.
        assert_eq!(
            date_of_value(&json!(1593640800000i64)).as_deref(),
            Some("2020-07-02")
        );
        assert_eq!(
            date_of_value(&json!("1593640800000")).as_deref(),
            Some("2020-07-02")
        );
        assert_eq!(date_of_value(&json!(null)), None);
        assert_eq!(date_of_value(&json!("bientôt")), None);
    }

    fn juri(numeros: &[&str], ecli: Option<&str>, juridiction: &str) -> Value {
        json!({"text": {
            "titre": "Conseil d'État, 5ème - 6ème chambres réunies, 02/07/2020, 428411, Publié au recueil Lebon",
            "juridiction": juridiction, "formation": "5ème - 6ème chambres réunies",
            "solution": "Annulation", "numeroAffaire": numeros, "ecli": ecli,
            "dateTexte": 1593640800000i64
        }})
    }

    #[test]
    fn a_consulted_decision_must_carry_the_cited_number_or_ecli() {
        let doc = parse_juri(&juri(
            &["428411", "428412"],
            Some("ECLI:FR:CECHR:2020:428411.20200702"),
            "CONSEIL D'ETAT",
        ))
        .unwrap();
        assert_eq!(doc.date.as_deref(), Some("2020-07-02"));
        let q = |number: Option<&str>, ecli: Option<&str>, court| DecisionQuery {
            court,
            number: number.map(String::from),
            display: number.unwrap_or("").into(),
            ecli: ecli.map(String::from),
            date: None,
            place: None,
        };
        assert_eq!(
            doc_matches(&doc, &q(Some("428412"), None, Court::ConseilEtat)).as_deref(),
            Some("428412")
        );
        assert_eq!(
            doc_matches(&doc, &q(Some("428413"), None, Court::ConseilEtat)),
            None
        );
        // Même numéro, autre juridiction : ce n'est pas la décision citée.
        assert_eq!(
            doc_matches(&doc, &q(Some("19NT01234"), None, Court::CourAdminAppel)),
            None
        );
        let caa = parse_juri(&juri(
            &["19NT01234"],
            None,
            "COUR ADMINISTRATIVE D'APPEL DE NANTES",
        ))
        .unwrap();
        assert!(doc_matches(&caa, &q(Some("19NT01234"), None, Court::CourAdminAppel)).is_some());
        assert!(doc_matches(&caa, &q(Some("19NT01234"), None, Court::ConseilEtat)).is_none());
        assert!(doc_matches(
            &doc,
            &q(
                None,
                Some("ecli:fr:cechr:2020:428411.20200702"),
                Court::ConseilEtat
            )
        )
        .is_some());
        assert!(doc_matches(
            &doc,
            &q(
                None,
                Some("ECLI:FR:CECHR:2020:999999.20200702"),
                Court::ConseilEtat
            )
        )
        .is_none());
        // ECLI absent de la décision lue : pas de correspondance.
        assert!(doc_matches(
            &caa,
            &q(
                None,
                Some("ECLI:FR:CAANT:2021:19NT01234.20210312"),
                Court::CourAdminAppel
            )
        )
        .is_none());
    }

    #[test]
    fn unexpected_consult_shapes_yield_no_document() {
        assert!(parse_juri(&json!({})).is_none());
        assert!(parse_juri(&json!({"text": {"juridiction": "X"}})).is_none());
        assert_eq!(
            parse_juri(&json!({"text": {"numeroAffaire": "428411"}}))
                .unwrap()
                .numeros,
            vec!["428411"]
        );
    }

    #[test]
    fn judilibre_ecli_is_matched_exactly() {
        let r = vec![
            json!({"id": "a", "ecli": "ECLI:FR:CCASS:2024:SO00999", "number": "22-00.999"}),
            json!({"id": "b", "ecli": "ECLI:FR:CCASS:2024:SO00123", "number": "22-12.345", "chamber": "soc"}),
        ];
        assert_eq!(
            pick_decision_by_ecli(&r, "ecli:fr:ccass:2024:so00123")
                .unwrap()
                .id,
            "b"
        );
        assert!(pick_decision_by_ecli(&r, "ECLI:FR:CCASS:2024:SO00124").is_none());
        assert!(
            pick_decision_by_ecli(&[json!({"id": "c"})], "ECLI:FR:CCASS:2024:SO00123").is_none()
        );
        let taxo =
            json!({"result": {"expose": "Exposé", "ecli": "ECLI", "number": "Numéro de pourvoi"}});
        assert_eq!(
            crate::juridique::verify_ext::ecli_field_from_taxonomy(&taxo).as_deref(),
            Some("ecli")
        );
        assert_eq!(
            crate::juridique::verify_ext::ecli_field_from_taxonomy(&json!({"result": {"a": "b"}})),
            None
        );
    }

    // ── Titres de résultats CETAT (forme relevée sur Légifrance) ──────────────

    const TITRE_CE: &str =
        "Conseil d'État, 5ème - 6ème chambres réunies, 02/07/2020, 428411, Publié au recueil Lebon";

    fn qry(court: Court, number: Option<&str>, ecli: Option<&str>) -> DecisionQuery {
        DecisionQuery {
            court,
            number: number.map(String::from),
            display: number.unwrap_or("").into(),
            ecli: ecli.map(String::from),
            date: None,
            place: None,
        }
    }

    #[test]
    fn a_cetat_title_is_read_into_court_formation_date_and_numbers() {
        let t = parse_title(TITRE_CE);
        assert_eq!(t.juridiction.as_deref(), Some("Conseil d'État"));
        assert_eq!(t.formation.as_deref(), Some("5ème - 6ème chambres réunies"));
        assert_eq!(t.date.as_deref(), Some("2020-07-02"));
        assert_eq!(t.numeros, ["428411"]);
        let t = parse_title(
            "CAA de NANTES, 1ère chambre, 12/03/2021, 19NT01234, Inédit au recueil Lebon",
        );
        assert_eq!(t.date.as_deref(), Some("2021-03-12"));
        assert_eq!(t.numeros, ["19NT01234"]);
        let t = parse_title("Tribunal administratif de Paris, 03/03/2022, 2012345/3-1");
        assert_eq!(t.numeros, ["2012345/3-1"]);
        let t = parse_title("Conseil d'État, 3ème chambre, 01/01/2020, 111111, 222222");
        assert_eq!(t.numeros, ["111111", "222222"]);
        assert_eq!(parse_title("Titre sans date"), TitleInfo::default());
    }

    #[test]
    fn a_title_matches_only_the_cited_court_number_and_for_ecli_date() {
        let t = parse_title(TITRE_CE);
        assert_eq!(
            title_matches(&t, &qry(Court::ConseilEtat, Some("428411"), None)).as_deref(),
            Some("428411")
        );
        assert_eq!(
            title_matches(&t, &qry(Court::ConseilEtat, Some("428412"), None)),
            None
        );
        // Même numéro, juridiction d'une autre classe.
        assert_eq!(
            title_matches(&t, &qry(Court::TribunalAdmin, Some("428411"), None)),
            None
        );
        let ecli = "ECLI:FR:CECHR:2020:428411.20200702";
        assert!(title_matches(&t, &qry(Court::ConseilEtat, None, Some(ecli))).is_some());
        // ECLI de même numéro mais d'une autre date : pas la même décision.
        let other = "ECLI:FR:CECHR:2021:428411.20210702";
        assert!(title_matches(&t, &qry(Court::ConseilEtat, None, Some(other))).is_none());
        // ECLI dont l'identifiant n'a pas la forme « numéro.date » : rien à rapprocher.
        let odd = "ECLI:FR:CECHR:2020:ABC";
        assert!(title_matches(&t, &qry(Court::ConseilEtat, None, Some(odd))).is_none());
        // Titre sans juridiction : on ne devine pas.
        let bare = TitleInfo {
            numeros: vec!["428411".into()],
            ..Default::default()
        };
        assert!(title_matches(&bare, &qry(Court::ConseilEtat, Some("428411"), None)).is_none());
    }

    #[test]
    fn ecli_identifier_carries_number_and_date() {
        assert_eq!(
            ecli_number_and_date("ECLI:FR:CECHR:2020:428411.20200702"),
            Some(("428411".into(), "2020-07-02".into()))
        );
        assert_eq!(
            ecli_number_and_date("ecli:fr:caant:2021:19nt01234.20210312"),
            Some(("19NT01234".into(), "2021-03-12".into()))
        );
        assert_eq!(ecli_number_and_date("ECLI:FR:CCASS:2024:SO00123"), None);
    }

    #[test]
    fn candidates_are_the_cetat_results_with_their_titles() {
        let v = json!({"results": [
            {"date": "2020-07-02T00:00:00.000+0000",
             "titles": [{"id": "CETATEXT000001", "title": "a"}, {"id": "JURITEXT000002"}]},
            {"titles": [{"id": "CETATEXT000001"}]},
            {"titles": [{"cid": "CETATEXT000004"}]},
            {"titles": [{"id": "CETATEXT000005"}, {"id": "CETATEXT000006"},
                        {"id": "CETATEXT000007"}, {"id": "CETATEXT000008"}]},
        ]});
        let c = cetat_candidates_of(&v);
        let ids: Vec<&str> = c.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "CETATEXT000001",
                "CETATEXT000004",
                "CETATEXT000005",
                "CETATEXT000006",
                "CETATEXT000007"
            ]
        );
        assert_eq!(c[0].title.as_deref(), Some("a"));
        assert_eq!(c[0].date.as_deref(), Some("2020-07-02"));
        assert!(cetat_candidates_of(&json!({})).is_empty());
    }

    #[test]
    fn a_cetat_info_takes_its_identity_from_the_title_and_the_reading() {
        let cand = Candidate {
            id: "CETATEXT000042".into(),
            title: Some(TITRE_CE.into()),
            date: None,
        };
        let t = parse_title(TITRE_CE);
        let i = info_from_cetat(&cand, &t, None, "428411".into(), None);
        assert_eq!(i.chamber.as_deref(), Some("5ème - 6ème chambres réunies"));
        assert_eq!(i.date.as_deref(), Some("2020-07-02"));
        assert_eq!(i.publication, vec!["Publié au recueil Lebon"]);
        assert_eq!(
            i.url.as_deref(),
            Some("https://www.legifrance.gouv.fr/ceta/id/CETATEXT000042")
        );
        let doc = parse_juri(&juri(
            &["428411"],
            Some("ECLI:FR:CECHR:2020:428411.20200702"),
            "CONSEIL D'ETAT",
        ))
        .unwrap();
        let i = info_from_cetat(&cand, &t, Some(&doc), "428411".into(), None);
        assert_eq!(i.solution.as_deref(), Some("Annulation"));
        assert_eq!(
            i.ecli.as_deref(),
            Some("ECLI:FR:CECHR:2020:428411.20200702")
        );
    }

    // ── Cours d'appel : départager des numéros de RG identiques ──────────────

    fn ca(id: &str, date: &str) -> DecisionInfo {
        DecisionInfo {
            id: id.into(),
            number: "19/01234".into(),
            date: Some(date.into()),
            ..Default::default()
        }
    }

    fn id_of(l: DecisionLookup) -> String {
        match l {
            DecisionLookup::Trouvee(i) => i.id,
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn an_rg_number_alone_never_identifies_an_appeal_decision() {
        let all = vec![ca("paris", "2021-03-12"), ca("lyon", "2021-03-12")];
        assert_eq!(
            narrow_ca(vec![], None, Some("2021-03-12")),
            DecisionLookup::Introuvable
        );
        // Date citée (siège non identifié) : la date suffit.
        assert_eq!(
            id_of(narrow_ca(all.clone(), None, Some("2021-03-12"))),
            "paris"
        );
        // Autre date : pas de verdict.
        assert!(matches!(
            narrow_ca(all.clone(), None, Some("2020-01-01")),
            DecisionLookup::Incertaine { .. }
        ));
        // Sans date ni siège : jamais « vérifiée ».
        assert!(matches!(
            narrow_ca(all.clone(), None, None),
            DecisionLookup::Incertaine { .. }
        ));
        // Siège identifié et arrêt retrouvé dans ce siège : suffit, même sans date.
        let lyon = Some(vec![ca("lyon", "2021-03-12")]);
        assert_eq!(id_of(narrow_ca(all.clone(), lyon.clone(), None)), "lyon");
        // Siège + date qui ne concordent pas : pas de verdict.
        assert!(matches!(
            narrow_ca(all.clone(), lyon, Some("2019-01-01")),
            DecisionLookup::Incertaine { .. }
        ));
        // Siège identifié mais aucun arrêt de ce numéro dans ce siège : autre cour.
        assert!(matches!(
            narrow_ca(all, Some(vec![]), Some("2021-03-12")),
            DecisionLookup::Incertaine { .. }
        ));
    }

    #[test]
    fn the_cited_seat_is_resolved_in_the_location_taxonomy() {
        let e = |k: &str, l: &str| (k.to_string(), l.to_string());
        let entries = vec![
            e("ca75056", "Paris"),
            e("ca69123", "Lyon"),
            e("ca13001", "Aix-en-Provence"),
            e("cax", "Cour d'appel de Metz"),
        ];
        assert_eq!(
            location_key_for(&entries, "Paris").as_deref(),
            Some("ca75056")
        );
        assert_eq!(
            location_key_for(&entries, "PARIS").as_deref(),
            Some("ca75056")
        );
        assert_eq!(
            location_key_for(&entries, "Aix en Provence").as_deref(),
            Some("ca13001")
        );
        assert_eq!(location_key_for(&entries, "Metz").as_deref(), Some("cax"));
        assert_eq!(location_key_for(&entries, "Rennes"), None);
        assert_eq!(location_key_for(&entries, ""), None);
        // Ambigu : on n'en choisit pas un au hasard.
        let dup = vec![e("a", "Paris"), e("b", "Cour d'appel de Paris")];
        assert_eq!(location_key_for(&dup, "Paris"), None);
    }

    #[test]
    fn taxonomy_entries_accepts_the_three_shapes() {
        assert_eq!(
            taxonomy_entries(&json!({"result": {"a": "A"}})),
            [("a".to_string(), "A".to_string())]
        );
        assert_eq!(
            taxonomy_entries(&json!({"result": [{"key": "a", "value": "A"}]})).len(),
            1
        );
        assert_eq!(
            taxonomy_entries(&json!({"result": {"key": "a", "value": "A"}})).len(),
            1
        );
        assert!(taxonomy_entries(&json!({})).is_empty());
    }
}
