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
// Juridix — Module Juridique — code_query.rs
// ============================================================================
// Recherche dans les CODES (fonds CODE_ETAT / CODE_DATE) : interprétation de
// la saisie libre et mise en forme des résultats. Partie PURE (aucun appel
// réseau) — l'orchestration des appels PISTE est dans `code_search.rs`.
//
// Pourquoi ce module ? `legifrance::search_simple` envoie toujours la saisie
// telle quelle, en « expression exacte », dans UN champ. Résultat :
//   - « code civil »                    → fonctionne (l'intitulé du code contient
//                                         exactement l'expression) ;
//   - « code civil, article 1140 »      → 0 résultat (aucun champ ne contient
//                                         cette expression) ;
//   - « responsabilité du fait des choses que l'on a sous sa garde »
//                                       → 0 résultat dès que la phrase n'est pas
//                                         recopiée mot pour mot.
// On distingue donc trois intentions (cf. `Parsed`) et on adapte la requête.
// ============================================================================

use serde_json::{json, Value};

use super::verify::{
    code_matches, extract_article_refs, find_code_name, fold, norm_num_key, strip_tags,
};

/// Nombre maximal d'articles distincts recherchés pour une même saisie
/// (« articles 1103, 1104 et 1105 du Code civil »).
pub const MAX_ARTICLES_PAR_SAISIE: usize = 5;

// ── Interprétation de la saisie ───────────────────────────────────────────────

/// Un numéro d'article à rechercher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumeroArticle {
    /// Valeur du champ NUM_ARTICLE (« 1140 », « L1234-5 », « 199 ter »).
    pub query: String,
    /// Clé de comparaison (majuscules, sans espace ni point : « L1234-5 »).
    pub key: String,
}

/// Ce que l'utilisateur cherche vraiment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Un ou plusieurs articles désignés par leur numéro, dans un code donné
    /// ou, si `code` est `None`, dans tous les codes.
    Article {
        code: Option<&'static str>,
        numeros: Vec<NumeroArticle>,
    },
    /// Du texte libre à chercher dans le contenu des articles, éventuellement
    /// restreint à un code.
    Text {
        code: Option<&'static str>,
        phrase: String,
    },
    /// Comportement historique inchangé (« code civil » seul, « code de la
    /// route », champ TITLE/TABLE explicite…).
    Passthrough,
}

/// Mots sans valeur pour décider « est-ce un numéro d'article seul ? ».
const FILLER: &[&str] = &[
    "article", "articles", "art", "arts", "du", "de", "des", "la", "le", "les", "l", "d", "au",
    "aux", "dans", "et", "ou", "code", "numero", "n", "no", "alinea", "al", "a", "sur", "en", "un",
    "une",
];

/// Mots vides retirés pour les paliers « tous les mots » / « un des mots ».
const STOPWORDS: &[&str] = &[
    "le", "la", "les", "l", "un", "une", "des", "du", "de", "d", "et", "ou", "a", "au", "aux",
    "en", "dans", "par", "pour", "sur", "sous", "avec", "sans", "que", "qu", "qui", "quoi", "dont",
    "est", "sont", "etre", "ce", "cet", "cette", "ces", "se", "sa", "son", "ses", "leur", "leurs",
    "il", "elle", "ils", "elles", "on", "ne", "n", "pas", "plus", "si", "y", "lorsque", "quand",
    "comme", "tout", "toute", "tous", "toutes", "meme", "ainsi", "alors", "donc", "or", "ni",
    "car",
];

/// Replie `q` caractère par caractère et renvoie, pour chaque octet du texte
/// replié, l'indice du caractère d'origine dont il provient (les positions
/// trouvées dans le texte replié se reportent ainsi sur le texte d'origine,
/// accents compris).
fn fold_map(q: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(q.len());
    let mut map = Vec::with_capacity(q.len());
    for (ci, ch) in q.chars().enumerate() {
        let f = fold(&ch.to_string());
        for _ in 0..f.len() {
            map.push(ci);
        }
        out.push_str(&f);
    }
    (out, map)
}

/// Indice de caractère pour chaque position d'octet de `q` (+ la fin).
fn byte_to_char(q: &str) -> Vec<usize> {
    let mut v = vec![0usize; q.len() + 1];
    let mut n = 0usize;
    for (ci, (bi, ch)) in q.char_indices().enumerate() {
        for k in 0..ch.len_utf8() {
            v[bi + k] = ci;
        }
        n = ci + 1;
    }
    v[q.len()] = n;
    v
}

fn is_edge_punct(c: char) -> bool {
    matches!(
        c,
        ',' | ';' | ':' | '(' | ')' | '[' | ']' | '"' | '«' | '»' | '“' | '”' | '.' | '?' | '!'
    )
}

/// Mots d'un texte (découpage sur les blancs, ponctuation de bord retirée).
fn words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| w.trim_matches(is_edge_punct).to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Mots porteurs de sens (hors `FILLER`).
fn content_words(s: &str) -> Vec<String> {
    words(s)
        .into_iter()
        .filter(|w| {
            let f = fold(w);
            // « d'ailleurs » / « l'article » : on regarde ce qui suit l'apostrophe.
            let f = f.rsplit('\'').next().unwrap_or("").to_string();
            !f.is_empty() && !FILLER.contains(&f.as_str())
        })
        .collect()
}

/// Numéro d'article « nu » : `1140`, `L. 1234-5`, `L1234-5`, `199 ter`, `1er`.
fn parse_bare_number(s: &str) -> Option<NumeroArticle> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?i)^(?P<l>[LRDA])?\s?\.?\s?(?P<d>\d{1,4}(?:-\d{1,4}){0,4})(?P<e>er|ere|ère)?(?:\s?(?P<s>bis|ter|quater|quinquies|sexies|septies|octies|nonies|decies|undecies|duodecies|terdecies))?$",
        )
        .expect("regex numéro nu")
    });
    let t = s.trim().trim_matches(is_edge_punct).trim();
    let c = RE.captures(t)?;
    let digits = c.name("d")?.as_str();
    // « 1er » n'a de sens que pour l'article 1 ; une lettre exige ≥ 2 chiffres.
    if c.name("e").is_some() && digits != "1" {
        return None;
    }
    let letter = c.name("l").map(|m| m.as_str().to_uppercase());
    if letter.is_some() && digits.chars().filter(|ch| ch.is_ascii_digit()).count() < 2 {
        return None;
    }
    let suffix = c.name("s").map(|m| m.as_str().to_lowercase());
    let mut query = format!("{}{}", letter.unwrap_or_default(), digits);
    if let Some(sfx) = &suffix {
        query.push(' ');
        query.push_str(sfx);
    }
    let key = norm_num_key(&query);
    Some(NumeroArticle { query, key })
}

/// Interprète la saisie d'un fond CODE.
///
/// `champ` : champ de recherche choisi dans l'interface (« ALL » par défaut).
/// Seuls ALL, NUM_ARTICLE et ARTICLE sont interprétés ; un champ TITLE ou
/// TABLE explicite garde le comportement historique.
pub fn classify(query: &str, champ: &str) -> Parsed {
    let q = query.trim();
    if q.is_empty() {
        return Parsed::Passthrough;
    }
    let champ = champ.trim().to_uppercase();
    let champ = if champ.is_empty() {
        "ALL".to_string()
    } else {
        champ
    };
    if !matches!(champ.as_str(), "ALL" | "NUM_ARTICLE" | "ARTICLE") {
        return Parsed::Passthrough;
    }

    // ── Repérage du code et des références d'articles ────────────────────────
    let chars: Vec<char> = q.chars().collect();
    let b2c = byte_to_char(q);
    let mut masked = vec![false; chars.len()];

    let (folded, fmap) = fold_map(q);
    let code_hit = find_code_name(&folded);
    if let Some((_, fs, fe)) = code_hit {
        if fe > fs {
            for m in masked.iter_mut().take(fmap[fe - 1] + 1).skip(fmap[fs]) {
                *m = true;
            }
        }
    }

    let refs = extract_article_refs(q);
    for r in &refs {
        for m in masked
            .iter_mut()
            .take(b2c[r.end.min(q.len())])
            .skip(b2c[r.start.min(q.len())])
        {
            *m = true;
        }
    }

    // Une désignation de code collée à une référence (« … du Code civil ») est
    // déjà résolue par `extract_article_refs` ; sinon on prend le premier code
    // nommé dans la saisie.
    let code = refs
        .iter()
        .find_map(|r| r.code)
        .or(code_hit.map(|(nom, _, _)| nom));

    let reste: String = chars
        .iter()
        .zip(&masked)
        .map(|(c, m)| if *m { ' ' } else { *c })
        .collect();
    let reste_words = content_words(&reste);

    // ── 1. Article(s) désigné(s) par leur numéro ─────────────────────────────
    if champ != "ARTICLE" {
        if !refs.is_empty() && (code.is_some() || reste_words.len() <= 2) {
            let mut numeros: Vec<NumeroArticle> = Vec::new();
            for r in &refs {
                let n = NumeroArticle {
                    query: r.numero_query(),
                    key: r.numero_key(),
                };
                if !numeros.iter().any(|x| x.key == n.key) {
                    numeros.push(n);
                }
                if numeros.len() >= MAX_ARTICLES_PAR_SAISIE {
                    break;
                }
            }
            return Parsed::Article { code, numeros };
        }
        // Numéro sans le mot « article » : « 1140 code civil », « code civil, 1103 »,
        // ou « 1140 » seul.
        if refs.is_empty() && reste_words.len() == 1 {
            if let Some(n) = parse_bare_number(&reste_words[0]) {
                return Parsed::Article {
                    code,
                    numeros: vec![n],
                };
            }
        }
        // « L. 1234-5 » écrit avec un espace : « L. », « 1234-5 » sont deux mots.
        if refs.is_empty() && reste_words.len() == 2 {
            if let Some(n) = parse_bare_number(&reste_words.join(" ")) {
                return Parsed::Article {
                    code,
                    numeros: vec![n],
                };
            }
        }
        if champ == "NUM_ARTICLE" {
            // Le champ « N° d'article » n'a de sens qu'avec un numéro.
            return Parsed::Passthrough;
        }
    }

    // ── 2. Texte libre ───────────────────────────────────────────────────────
    if champ == "ALL" {
        if code.is_some() && reste_words.is_empty() {
            // « code civil », « Code du travail » : on garde la recherche
            // historique, qui retourne le code lui-même.
            return Parsed::Passthrough;
        }
        if code.is_none() {
            let starts = folded.trim_start();
            if starts.starts_with("code ") || starts.starts_with("livre ") || starts == "code" {
                // « code de la route », « code rural » (hors table interne) : l'intitulé.
                return Parsed::Passthrough;
            }
        }
    }

    let phrase = if code.is_some() && refs.is_empty() {
        collapse(&reste)
    } else {
        collapse(q)
    };
    if phrase.is_empty() {
        return Parsed::Passthrough;
    }
    Parsed::Text { code, phrase }
}

/// Espaces normalisés, ponctuation de bord retirée.
fn collapse(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| is_edge_punct(c) || c.is_whitespace())
        .to_string()
}

// ── Paliers de recherche plein texte ──────────────────────────────────────────

/// Type de correspondance d'un palier (reflète `legifrance::TypeRecherche`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Correspondance {
    Exacte,
    TousLesMots,
    UnDesMots,
}

impl Correspondance {
    /// Libellé exposé au frontend (`recherche.palier`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Exacte => "exacte",
            Self::TousLesMots => "tous_les_mots",
            Self::UnDesMots => "un_des_mots",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palier {
    pub correspondance: Correspondance,
    pub valeur: String,
}

/// Mots-clés d'une phrase : sans mots vides ni mots d'une lettre, sans doublon.
pub fn mots_cles(phrase: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words(phrase) {
        // « l'on » / « qu'il » : on garde la partie après l'apostrophe.
        let w = w
            .rsplit(['\'', '’'])
            .next()
            .unwrap_or("")
            .trim_matches(is_edge_punct)
            .to_string();
        let f = fold(&w);
        if f.chars().count() < 2 || STOPWORDS.contains(&f.as_str()) {
            continue;
        }
        if !out.iter().any(|x| fold(x) == f) {
            out.push(w);
        }
    }
    out
}

/// Paliers du plus strict au plus large. Le premier qui donne des résultats
/// est retenu : une phrase recopiée mot pour mot reste prioritaire, une
/// description approximative bascule sur « tous les mots » puis « un des mots ».
pub fn paliers(phrase: &str) -> Vec<Palier> {
    let mut out = vec![Palier {
        correspondance: Correspondance::Exacte,
        valeur: phrase.to_string(),
    }];
    let kw = mots_cles(phrase);
    if kw.is_empty() {
        return out;
    }
    let joined = kw.join(" ");
    if fold(&joined) != fold(phrase) {
        out.push(Palier {
            correspondance: Correspondance::TousLesMots,
            valeur: joined.clone(),
        });
    }
    if kw.len() >= 2 {
        out.push(Palier {
            correspondance: Correspondance::UnDesMots,
            valeur: joined,
        });
    }
    out
}

// ── Lecture / mise en forme des résultats ─────────────────────────────────────

/// Un article trouvé dans une réponse `/search`.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    pub num: String,
    pub code_titre: Option<String>,
    pub code_id: Option<String>,
    pub etat: Option<String>,
    pub date_version: Option<i64>,
    /// Extraits surlignés (`<mark>`) renvoyés par l'API.
    pub values: Vec<String>,
}

impl Hit {
    pub fn is_vigueur(&self) -> bool {
        self.etat.as_deref().map(str::to_uppercase).as_deref() == Some("VIGUEUR")
    }
}

fn as_str(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// Date d'une réponse PISTE → millisecondes. Le Swagger documente des chaînes
/// ISO (« 2021-04-15T16:49:47.707+0000 ») ; on accepte aussi un nombre ou une
/// chaîne numérique (millisecondes) et une date seule (« 2021-04-15 »).
pub(crate) fn as_millis(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    let s = v.as_str()?.trim();
    if let Ok(n) = s.parse::<i64>() {
        return Some(n);
    }
    use chrono::{DateTime, NaiveDate};
    DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z")
        .or_else(|_| DateTime::parse_from_rfc3339(s))
        .map(|d| d.timestamp_millis())
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)
                .map(|d| d.and_utc().timestamp_millis())
        })
}

/// Parcourt `results[].sections[].extracts[]` (et les résultats « article » de
/// premier niveau) et renvoie les articles trouvés.
///
/// `wanted_key` : si présent, seuls les articles de ce numéro sont gardés
/// (comparaison par `norm_num_key`) ; sinon tous les articles `LEGIARTI…`.
pub fn collect_hits(resp: &Value, wanted_key: Option<&str>) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    let results = resp.get("results").and_then(Value::as_array);
    for res in results.into_iter().flatten() {
        let first_title = res.get("titles").and_then(|t| t.get(0));
        let code_titre = first_title
            .and_then(|t| t.get("title"))
            .and_then(as_str)
            .or_else(|| res.get("title").and_then(as_str))
            .map(|t| strip_tags(&t));
        let code_id = first_title
            .and_then(|t| t.get("cid").or_else(|| t.get("id")))
            .and_then(as_str)
            .filter(|i| i.starts_with("LEGITEXT"));
        let res_etat = res
            .get("legalStatus")
            .or_else(|| res.get("etat"))
            .and_then(as_str);

        let mut push = |node: &Value| {
            let Some(id) = node.get("id").and_then(as_str) else {
                return;
            };
            if !id.starts_with("LEGIARTI") {
                return;
            }
            let num = node
                .get("num")
                .and_then(as_str)
                .or_else(|| node.get("title").and_then(as_str))
                .map(|n| strip_tags(&n))
                .unwrap_or_default();
            if let Some(w) = wanted_key {
                if norm_num_key(&num) != w {
                    return;
                }
            }
            if out.iter().any(|h| h.id == id) {
                return;
            }
            let etat = node
                .get("legalStatus")
                .or_else(|| node.get("etat"))
                .and_then(as_str)
                .or_else(|| res_etat.clone());
            let values = node
                .get("values")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(as_str).collect::<Vec<_>>())
                .unwrap_or_default();
            out.push(Hit {
                id,
                num,
                code_titre: code_titre.clone(),
                code_id: code_id.clone(),
                etat,
                date_version: node.get("dateVersion").and_then(as_millis),
                values,
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
                push(ex);
            }
        }
        if res
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|i| i.starts_with("LEGIARTI"))
        {
            push(res);
        }
    }
    out
}

/// Ne garde que les articles du code demandé (comparaison tolérante aux accents
/// et aux suffixes de l'intitulé).
pub fn only_code(hits: Vec<Hit>, code: &str) -> Vec<Hit> {
    hits.into_iter()
        .filter(|h| {
            h.code_titre
                .as_deref()
                .map_or(true, |t| code_matches(t, code))
        })
        .collect()
}

/// En vigueur d'abord, puis « vigueur différée », puis le reste ; à état égal,
/// la version la plus récente en premier. Tri stable : l'ordre de pertinence de
/// l'API est conservé à égalité.
pub fn trier_hits(hits: &mut [Hit]) {
    fn rank(h: &Hit) -> u8 {
        if h.is_vigueur() {
            0
        } else if h.etat.as_deref().map(str::to_uppercase).as_deref() == Some("VIGUEUR_DIFF") {
            1
        } else {
            2
        }
    }
    hits.sort_by_key(|h| {
        (
            rank(h),
            std::cmp::Reverse(h.date_version.unwrap_or(i64::MIN)),
        )
    });
}

/// « Article 1140 » — sans doubler le mot si l'API l'a déjà mis.
fn libelle_article(num: &str) -> String {
    let n = num.trim();
    if n.is_empty() {
        return "Article".to_string();
    }
    if fold(n).starts_with("article") {
        n.to_string()
    } else {
        format!("Article {n}")
    }
}

/// Résultat « article » au format attendu par le frontend (même forme qu'un
/// résultat `/search` : `id`, `titles`, `sections[].extracts[].values`).
///
/// `texte` : extrait lisible (texte brut) à afficher à défaut de surlignage.
pub fn hit_to_result(h: &Hit, texte: Option<&str>) -> Value {
    let titre = match &h.code_titre {
        Some(c) if !c.is_empty() => format!("{c} — {}", libelle_article(&h.num)),
        _ => libelle_article(&h.num),
    };
    let mut values: Vec<String> = h.values.clone();
    if values.is_empty() {
        if let Some(t) = texte.map(str::trim).filter(|t| !t.is_empty()) {
            values.push(t.to_string());
        }
    }
    let mut r = json!({
        "id": h.id,
        "title": titre,
        "titles": [{ "id": h.id, "cid": h.id, "title": titre }],
        "nature": "ARTICLE",
        "num": h.num,
        "sections": [{
            "extracts": [{ "id": h.id, "num": h.num, "values": values }]
        }],
    });
    if let Some(e) = &h.etat {
        r["etat"] = json!(e);
    }
    if let Some(d) = h.date_version {
        r["dateVersion"] = json!(d);
    }
    if let Some(c) = &h.code_titre {
        r["codeTitre"] = json!(c);
    }
    if let Some(c) = &h.code_id {
        r["codeId"] = json!(c);
    }
    r
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn art(code: Option<&'static str>, nums: &[&str]) -> Parsed {
        Parsed::Article {
            code,
            numeros: nums
                .iter()
                .map(|n| NumeroArticle {
                    query: n.to_string(),
                    key: norm_num_key(n),
                })
                .collect(),
        }
    }

    #[test]
    fn code_puis_article() {
        let attendu = art(Some("Code civil"), &["1140"]);
        for q in [
            "code civil, article 1140",
            "Code civil article 1140",
            "article 1140 du code civil",
            "art. 1140 C. civ.",
            "code civil art 1140",
            "1140 code civil",
            "code civil 1140",
            "code civil, 1140",
            "  Code Civil , Article 1140  ",
        ] {
            assert_eq!(classify(q, "ALL"), attendu, "saisie : {q:?}");
        }
    }

    #[test]
    fn article_avec_lettre_et_suffixe() {
        assert_eq!(
            classify("article L. 1234-5 du code du travail", "ALL"),
            art(Some("Code du travail"), &["L1234-5"])
        );
        assert_eq!(
            classify("code du travail L1233-3", "ALL"),
            art(Some("Code du travail"), &["L1233-3"])
        );
        assert_eq!(
            classify("code général des impôts article 199 ter", "ALL"),
            art(Some("Code général des impôts"), &["199 ter"])
        );
        assert_eq!(
            classify("code civil article 1er", "ALL"),
            art(Some("Code civil"), &["1"])
        );
    }

    #[test]
    fn plusieurs_articles() {
        match classify("articles 1103, 1104 et 1105 du code civil", "ALL") {
            Parsed::Article { code, numeros } => {
                assert_eq!(code, Some("Code civil"));
                let ks: Vec<_> = numeros.iter().map(|n| n.key.as_str()).collect();
                assert_eq!(ks, ["1103", "1104", "1105"]);
            }
            autre => panic!("attendu Article, obtenu {autre:?}"),
        }
    }

    #[test]
    fn article_sans_code() {
        assert_eq!(classify("article 1140", "ALL"), art(None, &["1140"]));
        assert_eq!(classify("1140", "ALL"), art(None, &["1140"]));
        assert_eq!(classify("1140", "NUM_ARTICLE"), art(None, &["1140"]));
        assert_eq!(classify("L. 1234-5", "ALL"), art(None, &["L1234-5"]));
    }

    #[test]
    fn code_seul_garde_le_comportement_historique() {
        for q in [
            "code civil",
            "Code du travail",
            "c. civ.",
            "code de la route",
            "code rural",
        ] {
            assert_eq!(classify(q, "ALL"), Parsed::Passthrough, "saisie : {q:?}");
        }
    }

    #[test]
    fn champs_explicites_non_interpretes() {
        assert_eq!(
            classify("code civil article 1140", "TITLE"),
            Parsed::Passthrough
        );
        assert_eq!(
            classify("code civil article 1140", "TABLE"),
            Parsed::Passthrough
        );
        assert_eq!(
            classify("responsabilité", "NUM_ARTICLE"),
            Parsed::Passthrough
        );
        assert_eq!(classify("", "ALL"), Parsed::Passthrough);
        assert_eq!(classify("   ", "ALL"), Parsed::Passthrough);
    }

    #[test]
    fn phrase_libre_tous_codes() {
        assert_eq!(
            classify(
                "responsabilité du fait des choses que l'on a sous sa garde",
                "ALL"
            ),
            Parsed::Text {
                code: None,
                phrase: "responsabilité du fait des choses que l'on a sous sa garde".into()
            }
        );
        assert_eq!(
            classify("délai de prescription", "ARTICLE"),
            Parsed::Text {
                code: None,
                phrase: "délai de prescription".into()
            }
        );
    }

    #[test]
    fn phrase_libre_dans_un_code() {
        assert_eq!(
            classify("vice du consentement code civil", "ALL"),
            Parsed::Text {
                code: Some("Code civil"),
                phrase: "vice du consentement".into()
            }
        );
        assert_eq!(
            classify("Code du travail : licenciement économique", "ALL"),
            Parsed::Text {
                code: Some("Code du travail"),
                phrase: "licenciement économique".into()
            }
        );
    }

    #[test]
    fn nombre_avec_unite_n_est_pas_un_article() {
        // « 1103 euros » n'est pas un numéro d'article (RE_UNIT de verify.rs).
        assert!(matches!(
            classify("indemnité de 1103 euros", "ALL"),
            Parsed::Text { .. }
        ));
    }

    #[test]
    fn mots_cles_sans_mots_vides() {
        assert_eq!(
            mots_cles("responsabilité du fait des choses que l'on a sous sa garde"),
            ["responsabilité", "fait", "choses", "on", "garde"]
                .iter()
                .filter(|w| **w != "on")
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(mots_cles("le  la  de"), Vec::<String>::new());
    }

    #[test]
    fn paliers_du_strict_au_large() {
        let p = paliers("responsabilité du fait des choses");
        let k: Vec<_> = p.iter().map(|x| x.correspondance).collect();
        assert_eq!(
            k,
            [
                Correspondance::Exacte,
                Correspondance::TousLesMots,
                Correspondance::UnDesMots
            ]
        );
        assert_eq!(p[0].valeur, "responsabilité du fait des choses");
        assert_eq!(p[1].valeur, "responsabilité fait choses");
    }

    #[test]
    fn un_seul_mot_un_seul_palier() {
        let p = paliers("prescription");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].correspondance, Correspondance::Exacte);
        // « la prescription » : la phrase exacte, puis le mot-clé seul.
        let p = paliers("la prescription");
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].valeur, "prescription");
    }

    fn reponse() -> Value {
        json!({
            "totalResultNumber": 2,
            "results": [
                {
                    "titles": [{ "id": "LEGITEXT000006070721", "cid": "LEGITEXT000006070721", "title": "Code civil" }],
                    "sections": [{
                        "extracts": [
                            { "id": "LEGIARTI000032040795", "num": "1140", "legalStatus": "VIGUEUR",
                              "dateVersion": 1454284800000i64, "values": ["Il y a violence <mark>…</mark>"] },
                            { "id": "LEGIARTI000006436000", "num": "1141", "legalStatus": "VIGUEUR" }
                        ]
                    }]
                },
                {
                    "titles": [{ "id": "LEGITEXT000006069414", "title": "Code de la consommation" }],
                    "sections": [{
                        "extracts": [{ "id": "LEGIARTI000999", "num": "Article 1140", "legalStatus": "ABROGE" }]
                    }]
                }
            ]
        })
    }

    #[test]
    fn collect_hits_filtre_par_numero() {
        let hits = collect_hits(&reponse(), Some("1140"));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "LEGIARTI000032040795");
        assert_eq!(hits[0].code_titre.as_deref(), Some("Code civil"));
        assert_eq!(hits[0].code_id.as_deref(), Some("LEGITEXT000006070721"));
        assert!(hits[0].is_vigueur());
        assert_eq!(hits[0].values.len(), 1);
        assert_eq!(hits[1].etat.as_deref(), Some("ABROGE"));

        let civil = only_code(hits, "Code civil");
        assert_eq!(civil.len(), 1);
    }

    #[test]
    fn collect_hits_sans_filtre_prend_tous_les_articles() {
        assert_eq!(collect_hits(&reponse(), None).len(), 3);
    }

    #[test]
    fn collect_hits_tolere_les_formes_inattendues() {
        assert!(collect_hits(&json!({}), None).is_empty());
        assert!(collect_hits(&json!({ "results": null }), None).is_empty());
        assert!(collect_hits(&json!({ "results": [{}] }), Some("1")).is_empty());
        // Un extrait qui n'est pas un article (section) est ignoré.
        let r = json!({ "results": [{ "sections": [{ "extracts": [{ "id": "LEGISCTA1", "num": "1" }] }] }] });
        assert!(collect_hits(&r, None).is_empty());
    }

    #[test]
    fn dates_iso_du_swagger_sont_lues() {
        let ms = |v: Value| as_millis(&v);
        assert_eq!(
            ms(json!("2021-04-15T16:49:47.707+0000")),
            Some(1618505387707)
        );
        assert_eq!(ms(json!("2021-04-15")), Some(1618444800000));
        assert_eq!(ms(json!(1454284800000i64)), Some(1454284800000));
        assert_eq!(ms(json!("1454284800000")), Some(1454284800000));
        assert_eq!(ms(json!("n'importe quoi")), None);
        // Dans une réponse : versions triées de la plus récente à la plus ancienne.
        let r = json!({ "results": [{ "titles": [{ "title": "Code civil" }], "sections": [{ "extracts": [
            { "id": "LEGIARTI1", "num": "1", "legalStatus": "VIGUEUR", "dateVersion": "2016-10-01T00:00:00.000+0000" },
            { "id": "LEGIARTI2", "num": "1", "legalStatus": "VIGUEUR", "dateVersion": "2020-01-01T00:00:00.000+0000" }
        ]}]}]});
        let mut h = collect_hits(&r, None);
        trier_hits(&mut h);
        assert_eq!(h[0].id, "LEGIARTI2");
    }

    #[test]
    fn tri_vigueur_d_abord_puis_version_recente() {
        let mut hits = collect_hits(&reponse(), Some("1140"));
        hits.reverse();
        trier_hits(&mut hits);
        assert!(hits[0].is_vigueur());
    }

    #[test]
    fn hit_to_result_est_ouvrable_par_le_frontend() {
        let h = &collect_hits(&reponse(), Some("1140"))[0];
        let r = hit_to_result(h, Some("Il y a violence lorsque…"));
        assert_eq!(r["id"], "LEGIARTI000032040795");
        assert_eq!(r["titles"][0]["title"], "Code civil — Article 1140");
        assert_eq!(r["etat"], "VIGUEUR");
        // Le surlignage de l'API est conservé, le texte de repli ne l'écrase pas.
        assert_eq!(
            r["sections"][0]["extracts"][0]["values"][0],
            "Il y a violence <mark>…</mark>"
        );
        // « Article 1140 » n'est pas doublé en « Article Article 1140 ».
        let h2 = &collect_hits(&reponse(), Some("1140"))[1];
        assert_eq!(
            hit_to_result(h2, None)["title"],
            "Code de la consommation — Article 1140"
        );
    }
}
