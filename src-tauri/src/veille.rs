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

// Veille juridique (chantier 8 de la feuille de route).
//
// Une veille = des critères de recherche (mots-clés, fond, nature, code…) que
// Juridix rejoue périodiquement sur Légifrance ou Judilibre. Tout texte ou
// décision correspondant, jamais vu pour cette veille, est enregistré comme
// résultat « non lu » ; l'interface en affiche le nombre (badge) et la liste.
//
// ── Détection de la nouveauté ────────────────────────────────────────────────
// Les API filtrent par DATE (signature, décision, version), pas par « date
// d'apparition dans l'index ». Un décret signé lundi peut n'être publié que
// mercredi : si on ne cherchait que depuis la dernière exécution (mardi), on le
// manquerait. On cherche donc toujours depuis `dernier succès − 14 jours`, et ce
// sont les contraintes `UNIQUE(veille, texte)` qui garantissent qu'un texte déjà
// connu n'est jamais re-signalé. Le même mécanisme assure le RATTRAPAGE : après
// une longue période application fermée, la fenêtre s'élargit d'elle-même.
//
// ── Ménager l'API PISTE ──────────────────────────────────────────────────────
// Une seule exécution à la fois, appels espacés, pages plafonnées, fréquence
// minimale de 6 h, et arrêt immédiat de la série en cas de HTTP 429 (reprise
// plus tard) — jamais de rafale.
//
// ── Limites assumées ─────────────────────────────────────────────────────────
// - L'application doit être ouverte pour que la veille tourne (le rattrapage
//   couvre les périodes de fermeture au prochain démarrage).
// - Les mots-clés sont obligatoires : une veille « tout le Code du travail » sans
//   mot-clé n'est pas supportée (comportement de PISTE sur une requête vide non
//   validé).
// - Pas de notification système : seul le badge dans l'application.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveDate, SecondsFormat, Utc};
use futures::future::BoxFuture;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::api::AppState;
use crate::db::Database;
use crate::juridique::{
    client::PisteClient,
    judilibre::{self, JudilibreSearchParams},
    legifrance::{self, FiltreValeurs, SearchFond},
    routes::HasJuridiqueState,
};

// ── Paramètres ────────────────────────────────────────────────────────────────

/// Fréquence minimale / par défaut / maximale, en heures.
pub const FREQ_MIN_H: u32 = 6;
pub const FREQ_DEFAULT_H: u32 = 24;
pub const FREQ_MAX_H: u32 = 24 * 30;
/// Nombre maximal de veilles (garde-fou sur le volume d'appels).
pub const MAX_VEILLES: usize = 30;
/// Recouvrement de la fenêtre de recherche (voir l'en-tête : publication différée).
pub const OVERLAP_DAYS: i64 = 14;
/// Profondeur de la première recherche d'une veille (modifiable par veille).
pub const BACKFILL_DAYS: i64 = 14;
/// Profondeur maximale autorisée pour `depuis`.
pub const MAX_BACKFILL_DAYS: i64 = 730;
const PAGE_SIZE: u32 = 50;
const MAX_PAGES: u32 = 4;
/// Pause entre deux appels PISTE d'une même série.
const SPACING: Duration = Duration::from_millis(1500);
/// Un échec est retenté plus tôt que la fréquence normale.
const RETRY_AFTER_H: i64 = 1;
const TICK: Duration = Duration::from_secs(10 * 60);
const STARTUP_DELAY: Duration = Duration::from_secs(45);
const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(30 * 60);
/// Résultats déjà lus conservés par veille (les non lus ne sont jamais purgés).
const KEEP_SEEN: i64 = 2000;
const MAX_NOM: usize = 80;
const MAX_QUERY: usize = 300;

/// Fonds Légifrance proposés en veille : ceux qui exposent un filtre de date.
pub const FONDS: &[&str] = &[
    "JORF",
    "LODA_DATE",
    "CODE_DATE",
    "CIRC",
    "JURI",
    "CETAT",
    "CONSTIT",
    "CNIL",
    "KALI",
    "ACCO",
];

// ── Modèle ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Criteres {
    /// `legifrance` | `judilibre`
    pub source: String,
    /// Mots-clés (obligatoires).
    #[serde(default)]
    pub query: String,
    /// Légifrance : fonds parmi `FONDS`.
    #[serde(default)]
    pub fond: Option<String>,
    /// Légifrance JORF / LODA : natures (`LOI`, `DECRET`, `ARRETE`…).
    #[serde(default)]
    pub natures: Vec<String>,
    /// Légifrance CODE_DATE : nom du code (« Code du travail »).
    #[serde(default)]
    pub code: Option<String>,
    /// Judilibre : juridictions (`cc`, `ca`, `tj`…). Vide = Cour de cassation.
    #[serde(default)]
    pub juridictions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Veille {
    pub id: String,
    pub nom: String,
    pub criteres: Criteres,
    pub active: bool,
    pub frequence_heures: u32,
    /// Date (AAAA-MM-JJ) à partir de laquelle chercher lors de la première exécution.
    pub depuis: String,
    pub created_at: String,
    /// Dernière tentative (réussie ou non).
    pub last_run_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    /// Remarque non bloquante de la dernière exécution (ex. résultats tronqués).
    pub last_notice: Option<String>,
    pub non_lus: i64,
    pub total: i64,
}

/// Résultat détecté, avant enregistrement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewResultat {
    pub texte_id: String,
    pub source: String,
    pub fond: String,
    pub titre: String,
    pub nature: Option<String>,
    pub date_texte: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resultat {
    pub id: i64,
    pub veille_id: String,
    pub texte_id: String,
    pub source: String,
    pub fond: String,
    pub titre: String,
    pub nature: Option<String>,
    pub date_texte: Option<String>,
    pub detecte_le: String,
    pub vu: bool,
}

// ── Validation ────────────────────────────────────────────────────────────────

fn clean(s: &str, max: usize) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

impl Criteres {
    /// Normalise et valide. Refuse ce qui produirait des appels inutiles ou invalides.
    pub fn validated(&self) -> Result<Criteres, String> {
        let query = clean(&self.query, MAX_QUERY);
        if query.is_empty() {
            return Err("Les mots-clés sont obligatoires.".into());
        }
        match self.source.as_str() {
            "legifrance" => {
                let fond = self
                    .fond
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .to_ascii_uppercase();
                if !FONDS.contains(&fond.as_str()) {
                    return Err(format!(
                        "Fonds non pris en charge pour la veille : « {fond} »."
                    ));
                }
                let natures: Vec<String> = if matches!(fond.as_str(), "JORF" | "LODA_DATE") {
                    let mut n: Vec<String> = self
                        .natures
                        .iter()
                        .map(|x| x.trim().to_ascii_uppercase())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if n.iter().any(|x| {
                        x.len() > 30 || !x.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
                    }) {
                        return Err("Nature de texte invalide.".into());
                    }
                    n.sort();
                    n.dedup();
                    n.truncate(10);
                    n
                } else {
                    Vec::new()
                };
                let code = if fond == "CODE_DATE" {
                    self.code
                        .as_deref()
                        .map(|c| clean(c, 120))
                        .filter(|c| !c.is_empty())
                } else {
                    None
                };
                Ok(Criteres {
                    source: "legifrance".into(),
                    query,
                    fond: Some(fond),
                    natures,
                    code,
                    juridictions: vec![],
                })
            }
            "judilibre" => {
                let mut j: Vec<String> = self
                    .juridictions
                    .iter()
                    .map(|x| x.trim().to_ascii_lowercase())
                    .filter(|x| !x.is_empty())
                    .collect();
                if j.iter()
                    .any(|x| x.len() > 4 || !x.chars().all(|c| c.is_ascii_alphabetic()))
                {
                    return Err("Juridiction invalide.".into());
                }
                j.sort();
                j.dedup();
                j.truncate(6);
                Ok(Criteres {
                    source: "judilibre".into(),
                    query,
                    fond: None,
                    natures: vec![],
                    code: None,
                    juridictions: j,
                })
            }
            other => Err(format!("Source inconnue : « {other} ».")),
        }
    }
}

pub fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

fn parse_ts(s: &Option<String>) -> Option<DateTime<Utc>> {
    s.as_deref()
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|d| d.with_timezone(&Utc))
}

pub fn ts(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Secs, true)
}

// ── Planification (pure) ──────────────────────────────────────────────────────

/// La veille doit-elle être exécutée maintenant ?
///
/// Jamais exécutée → oui. Dernière tentative en échec → on retente après
/// `RETRY_AFTER_H` (pas besoin d'attendre toute la fréquence). Sinon, une fois la
/// fréquence écoulée depuis la dernière tentative.
pub fn is_due(v: &Veille, now: DateTime<Utc>) -> bool {
    if !v.active {
        return false;
    }
    let Some(last) = parse_ts(&v.last_run_at) else {
        return true;
    };
    let wait = if v.last_error.is_some() {
        ChronoDuration::hours(RETRY_AFTER_H.min(v.frequence_heures as i64))
    } else {
        ChronoDuration::hours(v.frequence_heures as i64)
    };
    now - last >= wait
}

/// Fenêtre `[début, fin]` de la recherche. Début = `depuis` à la première exécution,
/// ensuite `dernier succès − OVERLAP_DAYS` (jamais avant `depuis`). Fin = aujourd'hui.
pub fn search_window(v: &Veille, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let depuis = parse_date(&v.depuis).unwrap_or(today - ChronoDuration::days(BACKFILL_DAYS));
    let start = match parse_ts(&v.last_success_at) {
        Some(ok) => (ok.date_naive() - ChronoDuration::days(OVERLAP_DAYS)).max(depuis),
        None => depuis,
    };
    (start.min(today), today)
}

// ── Extraction des résultats (pure) ───────────────────────────────────────────

fn strip_tags(s: &str) -> String {
    static RE: Lazy<regex::Regex> = Lazy::new(|| regex::Regex::new(r"<[^>]*>").expect("regex"));
    RE.replace_all(s, "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn s(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

/// Date de l'API (millisecondes, secondes, texte numérique ou ISO) → « AAAA-MM-JJ ».
pub fn normalize_date(v: &Value) -> Option<String> {
    let from_num = |n: f64| -> Option<String> {
        if n <= 0.0 {
            return None;
        }
        let ms = if n < 100_000_000_000.0 { n * 1000.0 } else { n } as i64;
        crate::juridique::verify::millis_to_date(ms)
    };
    match v {
        Value::Number(n) => n.as_f64().and_then(from_num),
        Value::String(t) => {
            let t = t.trim();
            if t.len() >= 10 && parse_date(&t[..10]).is_some() {
                return Some(t[..10].to_string());
            }
            t.parse::<f64>().ok().and_then(from_num)
        }
        _ => None,
    }
}

fn cap(t: String) -> String {
    t.chars().take(300).collect()
}

/// Résultats d'une recherche Légifrance (`/search`).
///
/// Codes (CODE_DATE) : un résultat par ARTICLE trouvé (`sections[].extracts[]`) — un
/// article modifié a un nouvel identifiant de version, donc réapparaît comme nouveau.
/// Autres fonds : un résultat par TEXTE ou décision (`titles[0].id`).
pub fn extract_legifrance(resp: &Value, fond: &str) -> Vec<NewResultat> {
    let mut out = Vec::new();
    for r in resp
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let t0 = r.get("titles").and_then(|t| t.get(0));
        let titre_texte = t0
            .and_then(|t| t.get("title"))
            .and_then(s)
            .or_else(|| r.get("title").and_then(s))
            .map(|t| strip_tags(&t));
        let date = ["dateSignature", "datePublication", "dateDecision", "date"]
            .iter()
            .find_map(|k| r.get(*k).and_then(normalize_date));
        let nature = r.get("nature").and_then(s);

        if fond == "CODE_DATE" {
            for sec in r
                .get("sections")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                for ex in sec
                    .get("extracts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let Some(id) = ex.get("id").and_then(s) else {
                        continue;
                    };
                    let num = ex
                        .get("num")
                        .and_then(s)
                        .or_else(|| ex.get("title").and_then(s))
                        .map(|n| strip_tags(&n));
                    let titre = match (&titre_texte, num) {
                        (Some(c), Some(n)) => {
                            format!("{c} — Article {}", n.trim_start_matches("Article ").trim())
                        }
                        (Some(c), None) => c.clone(),
                        (None, Some(n)) => format!("Article {n}"),
                        (None, None) => id.clone(),
                    };
                    out.push(NewResultat {
                        texte_id: id,
                        source: "legifrance".into(),
                        fond: fond.into(),
                        titre: cap(titre),
                        nature: Some("ARTICLE".into()),
                        date_texte: ex
                            .get("dateVersion")
                            .and_then(normalize_date)
                            .or_else(|| date.clone()),
                    });
                }
            }
            continue;
        }

        let id = t0
            .and_then(|t| t.get("id").and_then(s).or_else(|| t.get("cid").and_then(s)))
            .or_else(|| r.get("id").and_then(s));
        let Some(id) = id else { continue };
        out.push(NewResultat {
            texte_id: id.clone(),
            source: "legifrance".into(),
            fond: fond.into(),
            titre: cap(titre_texte.unwrap_or(id)),
            nature,
            date_texte: date,
        });
    }
    out
}

/// Résultats d'une recherche Judilibre (`/search`).
pub fn extract_judilibre(resp: &Value) -> Vec<NewResultat> {
    let mut out = Vec::new();
    for r in resp
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = r.get("id").and_then(s) else {
            continue;
        };
        let date = r.get("decision_date").and_then(normalize_date);
        let juridiction = r.get("jurisdiction").and_then(s).unwrap_or_default();
        let chambre = r.get("chamber").and_then(s);
        let numero = r.get("number").and_then(s);
        let mut titre = match juridiction.as_str() {
            "cc" => "Cour de cassation".to_string(),
            "ca" => "Cour d'appel".to_string(),
            "tj" => "Tribunal judiciaire".to_string(),
            "" => "Décision".to_string(),
            autre => format!("Juridiction {autre}"),
        };
        if let Some(c) = chambre {
            titre.push_str(&format!(" — {c}"));
        }
        if let Some(n) = numero {
            titre.push_str(&format!(" — n° {n}"));
        }
        if let Some(d) = &date {
            titre.push_str(&format!(" ({d})"));
        }
        out.push(NewResultat {
            texte_id: id,
            source: "judilibre".into(),
            fond: "JURI".into(),
            titre: cap(titre),
            nature: r.get("solution").and_then(s),
            date_texte: date,
        });
    }
    out
}

// ── Interrogation des sources ─────────────────────────────────────────────────

#[derive(Debug)]
pub enum FetchError {
    /// HTTP 429 : on arrête toute la série et on réessaie plus tard.
    RateLimited(String),
    Other(String),
}

impl FetchError {
    fn message(&self) -> &str {
        match self {
            FetchError::RateLimited(m) | FetchError::Other(m) => m,
        }
    }
}

#[derive(Debug, Default)]
pub struct FetchOutcome {
    pub items: Vec<NewResultat>,
    /// Il restait des résultats au-delà du plafond de pages.
    pub truncated: bool,
    pub total: Option<i64>,
}

/// Source de résultats. Le trait isole le planificateur de l'API réseau : les tests
/// injectent un faux, la production utilise `PisteFetcher`.
pub trait Fetcher: Send + Sync {
    fn fetch<'a>(
        &'a self,
        c: &'a Criteres,
        since: NaiveDate,
        until: NaiveDate,
    ) -> BoxFuture<'a, Result<FetchOutcome, FetchError>>;
}

/// Tri « les plus récents d'abord », valide pour le fond (vérifié par un test).
pub fn sort_for(fond: &str) -> &'static str {
    match fond {
        "JORF" | "LODA_DATE" | "CODE_DATE" => "PUBLICATION_DATE_DESC",
        "CIRC" => "PUBLI_DATE_DESC",
        "CNIL" => "DATE_DECISION_DESC",
        "KALI" => "MODIFICATION_DATE_DESC",
        _ => "DATE_DESC", // JURI, CETAT, CONSTIT, ACCO
    }
}

fn classify(e: anyhow::Error) -> FetchError {
    let m = e.to_string();
    if m.contains("HTTP 429") {
        FetchError::RateLimited(m)
    } else {
        FetchError::Other(m)
    }
}

pub struct PisteFetcher {
    client: PisteClient,
    spacing: Duration,
}

impl PisteFetcher {
    pub fn new(client: PisteClient) -> Self {
        Self {
            client,
            spacing: SPACING,
        }
    }

    #[cfg(test)]
    pub fn without_pause(client: PisteClient) -> Self {
        Self {
            client,
            spacing: Duration::ZERO,
        }
    }

    async fn fetch_legifrance(
        &self,
        c: &Criteres,
        since: NaiveDate,
        until: NaiveDate,
    ) -> Result<FetchOutcome, FetchError> {
        let fond_code = c.fond.clone().unwrap_or_default();
        let fond: SearchFond = crate::juridique::routes::parse_fond(&fond_code);
        let natures = (!c.natures.is_empty()).then(|| c.natures.clone());
        let extra = c.code.as_ref().map(|code| {
            vec![FiltreValeurs {
                facette: "NOM_CODE".into(),
                valeurs: vec![code.clone()],
            }]
        });
        let mut out = FetchOutcome::default();
        for page in 1..=MAX_PAGES {
            if page > 1 && !self.spacing.is_zero() {
                tokio::time::sleep(self.spacing).await;
            }
            let resp = legifrance::search_simple(
                &self.client,
                &c.query,
                fond.clone(),
                page,
                PAGE_SIZE,
                Some(sort_for(&fond_code).to_string()),
                natures.clone(),
                Some(since.format("%Y-%m-%d").to_string()),
                Some(until.format("%Y-%m-%d").to_string()),
                None,
                None,
                extra.clone(),
            )
            .await
            .map_err(classify)?;
            out.total = resp.total;
            let raw_count = resp.results.as_ref().map_or(0, Vec::len);
            let value =
                serde_json::to_value(&resp).map_err(|e| FetchError::Other(e.to_string()))?;
            out.items.extend(extract_legifrance(&value, &fond_code));
            let seen = (page * PAGE_SIZE) as i64;
            if raw_count < PAGE_SIZE as usize || resp.total.is_some_and(|t| seen >= t) {
                return Ok(out);
            }
        }
        out.truncated = true;
        Ok(out)
    }

    async fn fetch_judilibre(
        &self,
        c: &Criteres,
        since: NaiveDate,
        until: NaiveDate,
    ) -> Result<FetchOutcome, FetchError> {
        let mut out = FetchOutcome::default();
        for page in 0..MAX_PAGES {
            if page > 0 && !self.spacing.is_zero() {
                tokio::time::sleep(self.spacing).await;
            }
            let params = JudilibreSearchParams {
                query: c.query.clone(),
                jurisdiction: (!c.juridictions.is_empty()).then(|| c.juridictions.clone()),
                date_start: Some(since.format("%Y-%m-%d").to_string()),
                date_end: Some(until.format("%Y-%m-%d").to_string()),
                sort: Some("date".into()),
                order: Some("desc".into()),
                batch: page,
                batch_size: PAGE_SIZE,
                ..Default::default()
            };
            let resp = judilibre::search(&self.client, params)
                .await
                .map_err(classify)?;
            out.total = resp.total;
            let raw_count = resp.results.as_ref().map_or(0, Vec::len);
            let value =
                serde_json::to_value(&resp).map_err(|e| FetchError::Other(e.to_string()))?;
            out.items.extend(extract_judilibre(&value));
            if raw_count < PAGE_SIZE as usize || resp.next_batch.is_none() {
                return Ok(out);
            }
        }
        out.truncated = true;
        Ok(out)
    }
}

impl Fetcher for PisteFetcher {
    fn fetch<'a>(
        &'a self,
        c: &'a Criteres,
        since: NaiveDate,
        until: NaiveDate,
    ) -> BoxFuture<'a, Result<FetchOutcome, FetchError>> {
        Box::pin(async move {
            match c.source.as_str() {
                "legifrance" => self.fetch_legifrance(c, since, until).await,
                "judilibre" => self.fetch_judilibre(c, since, until).await,
                other => Err(FetchError::Other(format!("source inconnue : {other}"))),
            }
        })
    }
}

// ── Exécution ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub veille_id: String,
    /// Résultats jamais vus jusqu'ici pour cette veille.
    pub nouveaux: usize,
    /// Résultats trouvés dans la fenêtre (nouveaux ou déjà connus).
    pub trouves: usize,
    pub truncated: bool,
    pub error: Option<String>,
    #[serde(skip)]
    pub rate_limited: bool,
}

/// Une seule exécution à la fois (planifiée ou manuelle) : jamais deux séries
/// d'appels PISTE en parallèle.
static RUN_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

/// Exécute une veille : cherche dans la fenêtre, enregistre les nouveautés, note le résultat.
pub async fn run_one(
    db: &Database,
    fetcher: &dyn Fetcher,
    v: &Veille,
    now: DateTime<Utc>,
    today: NaiveDate,
) -> RunReport {
    let _guard = RUN_LOCK.lock().await;
    let (since, until) = search_window(v, today);
    let mut report = RunReport {
        veille_id: v.id.clone(),
        nouveaux: 0,
        trouves: 0,
        truncated: false,
        error: None,
        rate_limited: false,
    };
    match fetcher.fetch(&v.criteres, since, until).await {
        Ok(outcome) => {
            report.trouves = outcome.items.len();
            report.truncated = outcome.truncated;
            let notice = outcome.truncated.then(|| {
                format!(
                    "Plus de {} résultats dans la période : seuls les plus récents ont été examinés. Précisez vos critères pour ne rien manquer.",
                    MAX_PAGES * PAGE_SIZE
                )
            });
            match db
                .veille_insert_resultats(&v.id, outcome.items, &ts(now))
                .await
            {
                Ok(n) => {
                    report.nouveaux = n;
                    if let Err(e) = db
                        .veille_record_run(&v.id, &ts(now), true, None, notice)
                        .await
                    {
                        warn!(
                            "[Veille] {} : enregistrement du passage impossible — {e}",
                            v.nom
                        );
                    }
                    let _ = db.veille_prune(&v.id).await;
                    info!(
                        "[Veille] « {} » : {} nouveau(x) sur {} trouvé(s) ({} → {})",
                        v.nom, n, report.trouves, since, until
                    );
                }
                Err(e) => {
                    report.error = Some(format!("enregistrement impossible : {e}"));
                    let _ = db
                        .veille_record_run(&v.id, &ts(now), false, report.error.clone(), None)
                        .await;
                }
            }
        }
        Err(e) => {
            report.rate_limited = matches!(e, FetchError::RateLimited(_));
            // Message court pour l'utilisateur ; le détail technique reste dans les logs.
            warn!("[Veille] « {} » : {}", v.nom, e.message());
            let user_msg = if report.rate_limited {
                "Quota PISTE atteint : nouvelle tentative plus tard.".to_string()
            } else {
                "Légifrance / Judilibre n'a pas répondu correctement : nouvelle tentative plus tard.".to_string()
            };
            report.error = Some(user_msg.clone());
            let _ = db
                .veille_record_run(&v.id, &ts(now), false, Some(user_msg), None)
                .await;
        }
    }
    report
}

#[derive(Debug, Default)]
pub struct TickReport {
    pub runs: Vec<RunReport>,
    pub rate_limited: bool,
}

/// Exécute les veilles arrivées à échéance, l'une après l'autre. Une limite de
/// débit interrompt toute la série.
pub async fn run_due(
    db: &Database,
    fetcher: &dyn Fetcher,
    now: DateTime<Utc>,
    today: NaiveDate,
    spacing: Duration,
) -> TickReport {
    let mut tick = TickReport::default();
    let veilles = match db.veille_list().await {
        Ok(v) => v,
        Err(e) => {
            warn!("[Veille] liste illisible : {e}");
            return tick;
        }
    };
    let mut first = true;
    for v in veilles.iter().filter(|v| is_due(v, now)) {
        if !first && !spacing.is_zero() {
            tokio::time::sleep(spacing).await;
        }
        first = false;
        let report = run_one(db, fetcher, v, now, today).await;
        let limited = report.rate_limited;
        tick.runs.push(report);
        if limited {
            tick.rate_limited = true;
            break;
        }
    }
    tick
}

/// Boucle de fond lancée au démarrage : sonde les veilles à échéance toutes les
/// 10 minutes. Sans identifiants PISTE (poussés par le frontend après lecture du
/// trousseau), elle attend simplement le tick suivant.
pub async fn run_scheduler(state: Arc<AppState>) {
    tokio::time::sleep(STARTUP_DELAY).await;
    loop {
        let mut pause = TICK;
        match state.juridique_state().client_async().await {
            Ok(client) => {
                let fetcher = PisteFetcher::new(client);
                let tick = run_due(
                    &state.db,
                    &fetcher,
                    Utc::now(),
                    Local::now().date_naive(),
                    SPACING,
                )
                .await;
                if tick.rate_limited {
                    pause = RATE_LIMIT_BACKOFF;
                }
            }
            Err(_) => debug!(
                "[Veille] identifiants PISTE absents : prochain essai dans {:?}",
                TICK
            ),
        }
        tokio::time::sleep(pause).await;
    }
}

// ── Stockage ──────────────────────────────────────────────────────────────────

/// Tables de la veille (ajoutées au schéma par `Database::new`).
pub const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS veilles (
    id               TEXT PRIMARY KEY,
    nom              TEXT NOT NULL,
    criteres         TEXT NOT NULL,
    active           INTEGER NOT NULL DEFAULT 1,
    frequence_heures INTEGER NOT NULL DEFAULT 24,
    depuis           TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    last_run_at      TEXT,
    last_success_at  TEXT,
    last_error       TEXT,
    last_notice      TEXT
);
CREATE TABLE IF NOT EXISTS veille_resultats (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    veille_id   TEXT NOT NULL,
    texte_id    TEXT NOT NULL,
    source      TEXT NOT NULL,
    fond        TEXT NOT NULL,
    titre       TEXT NOT NULL,
    nature      TEXT,
    date_texte  TEXT,
    detecte_le  TEXT NOT NULL,
    vu          INTEGER NOT NULL DEFAULT 0,
    UNIQUE (veille_id, texte_id)
);
CREATE INDEX IF NOT EXISTS idx_veille_resultats_veille ON veille_resultats (veille_id, vu, id);
";

#[derive(Debug, Clone)]
pub struct NewVeille {
    pub nom: String,
    pub criteres: Criteres,
    pub active: bool,
    pub frequence_heures: u32,
    pub depuis: String,
}

type DbResult<T> = anyhow::Result<T>;

fn row_veille(row: &rusqlite::Row<'_>) -> rusqlite::Result<Veille> {
    let criteres: String = row.get(2)?;
    Ok(Veille {
        id: row.get(0)?,
        nom: row.get(1)?,
        criteres: serde_json::from_str(&criteres).unwrap_or_default(),
        active: row.get::<_, i64>(3)? != 0,
        frequence_heures: row
            .get::<_, i64>(4)?
            .clamp(FREQ_MIN_H as i64, FREQ_MAX_H as i64) as u32,
        depuis: row.get(5)?,
        created_at: row.get(6)?,
        last_run_at: row.get(7)?,
        last_success_at: row.get(8)?,
        last_error: row.get(9)?,
        last_notice: row.get(10)?,
        non_lus: row.get(11)?,
        total: row.get(12)?,
    })
}

const SELECT_VEILLE: &str =
    "SELECT v.id, v.nom, v.criteres, v.active, v.frequence_heures, v.depuis, v.created_at,
        v.last_run_at, v.last_success_at, v.last_error, v.last_notice,
        (SELECT COUNT(*) FROM veille_resultats r WHERE r.veille_id = v.id AND r.vu = 0),
        (SELECT COUNT(*) FROM veille_resultats r WHERE r.veille_id = v.id)
     FROM veilles v";

impl Database {
    pub async fn veille_list(&self) -> DbResult<Vec<Veille>> {
        Ok(self
            .conn()
            .call(|db| {
                let mut stmt = db.prepare(&format!(
                    "{SELECT_VEILLE} ORDER BY v.created_at ASC, v.rowid ASC"
                ))?;
                let rows = stmt
                    .query_map([], row_veille)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?)
    }

    pub async fn veille_get(&self, id: String) -> DbResult<Option<Veille>> {
        Ok(self
            .conn()
            .call(move |db| {
                let mut stmt = db.prepare(&format!("{SELECT_VEILLE} WHERE v.id = ?1"))?;
                let mut rows = stmt.query_map([&id], row_veille)?;
                Ok(rows.next().transpose()?)
            })
            .await?)
    }

    pub async fn veille_create(&self, n: NewVeille, now: &str) -> DbResult<Veille> {
        let id = uuid::Uuid::new_v4().to_string();
        let criteres = serde_json::to_string(&n.criteres)?;
        let (id2, now) = (id.clone(), now.to_string());
        self.conn()
            .call(move |db| {
                db.execute(
                    "INSERT INTO veilles (id, nom, criteres, active, frequence_heures, depuis, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![id2, n.nom, criteres, n.active as i64, n.frequence_heures as i64, n.depuis, now],
                )?;
                Ok(())
            })
            .await?;
        self.veille_get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("veille introuvable après création"))
    }

    /// Met à jour une veille. Si les critères changent, l'historique des résultats est
    /// conservé mais la prochaine exécution repart de `depuis` (pas de rattrapage partiel
    /// sur d'anciens critères).
    pub async fn veille_update(&self, id: String, n: NewVeille) -> DbResult<Option<Veille>> {
        let criteres = serde_json::to_string(&n.criteres)?;
        let id2 = id.clone();
        let changed = self
            .conn()
            .call(move |db| {
                let old: Option<String> = db
                    .query_row("SELECT criteres FROM veilles WHERE id = ?1", [&id2], |r| r.get(0))
                    .ok();
                let Some(old) = old else { return Ok(false) };
                let reset = old != criteres;
                db.execute(
                    "UPDATE veilles SET nom = ?2, criteres = ?3, active = ?4, frequence_heures = ?5, depuis = ?6,
                         last_success_at = CASE WHEN ?7 THEN NULL ELSE last_success_at END,
                         last_run_at     = CASE WHEN ?7 THEN NULL ELSE last_run_at END,
                         last_error      = CASE WHEN ?7 THEN NULL ELSE last_error END,
                         last_notice     = CASE WHEN ?7 THEN NULL ELSE last_notice END
                     WHERE id = ?1",
                    rusqlite::params![id2, n.nom, criteres, n.active as i64, n.frequence_heures as i64, n.depuis, reset],
                )?;
                Ok(true)
            })
            .await?;
        if !changed {
            return Ok(None);
        }
        self.veille_get(id).await
    }

    pub async fn veille_delete(&self, id: String) -> DbResult<bool> {
        Ok(self
            .conn()
            .call(move |db| {
                let tx = db.transaction()?;
                tx.execute("DELETE FROM veille_resultats WHERE veille_id = ?1", [&id])?;
                let n = tx.execute("DELETE FROM veilles WHERE id = ?1", [&id])?;
                tx.commit()?;
                Ok(n > 0)
            })
            .await?)
    }

    /// Enregistre les résultats jamais vus ; renvoie combien étaient réellement nouveaux.
    pub async fn veille_insert_resultats(
        &self,
        veille_id: &str,
        items: Vec<NewResultat>,
        now: &str,
    ) -> DbResult<usize> {
        let (vid, now) = (veille_id.to_string(), now.to_string());
        Ok(self
            .conn()
            .call(move |db| {
                let tx = db.transaction()?;
                let mut inserted = 0usize;
                {
                    let mut stmt = tx.prepare(
                        "INSERT OR IGNORE INTO veille_resultats
                            (veille_id, texte_id, source, fond, titre, nature, date_texte, detecte_le, vu)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)",
                    )?;
                    for it in &items {
                        inserted += stmt.execute(rusqlite::params![
                            vid, it.texte_id, it.source, it.fond, it.titre, it.nature, it.date_texte, now
                        ])?;
                    }
                }
                tx.commit()?;
                Ok(inserted)
            })
            .await?)
    }

    pub async fn veille_record_run(
        &self,
        id: &str,
        now: &str,
        success: bool,
        error: Option<String>,
        notice: Option<String>,
    ) -> DbResult<()> {
        let (id, now) = (id.to_string(), now.to_string());
        self.conn()
            .call(move |db| {
                if success {
                    db.execute(
                        "UPDATE veilles SET last_run_at = ?2, last_success_at = ?2, last_error = NULL, last_notice = ?3 WHERE id = ?1",
                        rusqlite::params![id, now, notice],
                    )?;
                } else {
                    db.execute(
                        "UPDATE veilles SET last_run_at = ?2, last_error = ?3 WHERE id = ?1",
                        rusqlite::params![id, now, error],
                    )?;
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Supprime les plus anciens résultats DÉJÀ LUS au-delà de `KEEP_SEEN` (jamais un non lu).
    pub async fn veille_prune(&self, veille_id: &str) -> DbResult<usize> {
        let vid = veille_id.to_string();
        Ok(self
            .conn()
            .call(move |db| {
                Ok(db.execute(
                    "DELETE FROM veille_resultats WHERE veille_id = ?1 AND vu = 1
                       AND id NOT IN (SELECT id FROM veille_resultats WHERE veille_id = ?1 ORDER BY id DESC LIMIT ?2)",
                    rusqlite::params![vid, KEEP_SEEN],
                )?)
            })
            .await?)
    }

    pub async fn veille_resultats(
        &self,
        veille_id: &str,
        non_lus: bool,
        limit: i64,
        offset: i64,
    ) -> DbResult<Vec<Resultat>> {
        let vid = veille_id.to_string();
        Ok(self
            .conn()
            .call(move |db| {
                let mut stmt = db.prepare(
                    "SELECT id, veille_id, texte_id, source, fond, titre, nature, date_texte, detecte_le, vu
                       FROM veille_resultats
                      WHERE veille_id = ?1 AND (?2 = 0 OR vu = 0)
                      ORDER BY id DESC LIMIT ?3 OFFSET ?4",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![vid, non_lus as i64, limit, offset], |r| {
                        Ok(Resultat {
                            id: r.get(0)?,
                            veille_id: r.get(1)?,
                            texte_id: r.get(2)?,
                            source: r.get(3)?,
                            fond: r.get(4)?,
                            titre: r.get(5)?,
                            nature: r.get(6)?,
                            date_texte: r.get(7)?,
                            detecte_le: r.get(8)?,
                            vu: r.get::<_, i64>(9)? != 0,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?)
    }

    /// Marque comme lus les résultats `ids` (ou tous ceux de la veille si `None`).
    pub async fn veille_marquer_lus(
        &self,
        veille_id: &str,
        ids: Option<Vec<i64>>,
    ) -> DbResult<usize> {
        let vid = veille_id.to_string();
        Ok(self
            .conn()
            .call(move |db| match ids {
                None => Ok(db.execute("UPDATE veille_resultats SET vu = 1 WHERE veille_id = ?1 AND vu = 0", [&vid])?),
                Some(ids) => {
                    let tx = db.transaction()?;
                    let mut n = 0;
                    for id in ids {
                        n += tx.execute(
                            "UPDATE veille_resultats SET vu = 1 WHERE veille_id = ?1 AND id = ?2 AND vu = 0",
                            rusqlite::params![vid, id],
                        )?;
                    }
                    tx.commit()?;
                    Ok(n)
                }
            })
            .await?)
    }

    /// Nombre de résultats non lus : (total, par veille).
    pub async fn veille_non_lus(&self) -> DbResult<(i64, Vec<(String, i64)>)> {
        Ok(self
            .conn()
            .call(|db| {
                let mut stmt = db.prepare(
                    "SELECT veille_id, COUNT(*) FROM veille_resultats WHERE vu = 0 GROUP BY veille_id ORDER BY veille_id",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                let total = rows.iter().map(|(_, n)| n).sum();
                Ok((total, rows))
            })
            .await?)
    }
}

// ── Routes ────────────────────────────────────────────────────────────────────

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    warn!("[Veille] {e}");
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Erreur interne de la veille.",
    )
}

#[derive(Debug, Deserialize)]
pub struct VeilleInput {
    pub nom: String,
    pub criteres: Criteres,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub frequence_heures: Option<u32>,
    #[serde(default)]
    pub depuis: Option<String>,
}

/// Valide une saisie. `today` rend la validation de `depuis` testable.
pub fn validate_input(inp: VeilleInput, today: NaiveDate) -> Result<NewVeille, String> {
    let nom = clean(&inp.nom, MAX_NOM);
    if nom.is_empty() {
        return Err("Le nom de la veille est obligatoire.".into());
    }
    let criteres = inp.criteres.validated()?;
    let freq = inp.frequence_heures.unwrap_or(FREQ_DEFAULT_H);
    if !(FREQ_MIN_H..=FREQ_MAX_H).contains(&freq) {
        return Err(format!(
            "Fréquence invalide : entre {FREQ_MIN_H} h et {FREQ_MAX_H} h (pour ménager l'API PISTE)."
        ));
    }
    let depuis = match inp
        .depuis
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        None => today - ChronoDuration::days(BACKFILL_DAYS),
        Some(d) => {
            let date = parse_date(d).ok_or("Date de départ invalide (AAAA-MM-JJ).")?;
            if date > today {
                return Err("La date de départ ne peut pas être dans le futur.".into());
            }
            if today - date > ChronoDuration::days(MAX_BACKFILL_DAYS) {
                return Err(format!(
                    "Date de départ trop ancienne (maximum {MAX_BACKFILL_DAYS} jours)."
                ));
            }
            date
        }
    };
    Ok(NewVeille {
        nom,
        criteres,
        active: inp.active.unwrap_or(true),
        frequence_heures: freq,
        depuis: depuis.format("%Y-%m-%d").to_string(),
    })
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

async fn list(State(state): State<Arc<AppState>>) -> Response {
    match state.db.veille_list().await {
        Ok(v) => Json(json!({
            "veilles": v,
            "limites": { "frequence_min_h": FREQ_MIN_H, "frequence_max_h": FREQ_MAX_H, "frequence_defaut_h": FREQ_DEFAULT_H,
                         "max_veilles": MAX_VEILLES, "fonds": FONDS, "recouvrement_jours": OVERLAP_DAYS },
        }))
        .into_response(),
        Err(e) => internal(e),
    }
}

async fn create(State(state): State<Arc<AppState>>, Json(inp): Json<VeilleInput>) -> Response {
    let n = match validate_input(inp, today()) {
        Ok(n) => n,
        Err(m) => return err(StatusCode::BAD_REQUEST, m),
    };
    match state.db.veille_list().await {
        Ok(v) if v.len() >= MAX_VEILLES => {
            return err(
                StatusCode::BAD_REQUEST,
                format!("Nombre maximal de veilles atteint ({MAX_VEILLES})."),
            )
        }
        Err(e) => return internal(e),
        _ => {}
    }
    match state.db.veille_create(n, &ts(Utc::now())).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => internal(e),
    }
}

async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(inp): Json<VeilleInput>,
) -> Response {
    let n = match validate_input(inp, today()) {
        Ok(n) => n,
        Err(m) => return err(StatusCode::BAD_REQUEST, m),
    };
    match state.db.veille_update(id, n).await {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "Veille introuvable."),
        Err(e) => internal(e),
    }
}

async fn remove(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.db.veille_delete(id).await {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "Veille introuvable."),
        Err(e) => internal(e),
    }
}

#[derive(Debug, Deserialize)]
struct ResultatsQuery {
    #[serde(default)]
    non_lus: bool,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn resultats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<ResultatsQuery>,
) -> Response {
    let veille = match state.db.veille_get(id.clone()).await {
        Ok(Some(v)) => v,
        Ok(None) => return err(StatusCode::NOT_FOUND, "Veille introuvable."),
        Err(e) => return internal(e),
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    match state
        .db
        .veille_resultats(&id, q.non_lus, limit, offset)
        .await
    {
        Ok(r) => Json(json!({ "resultats": r, "total": veille.total, "non_lus": veille.non_lus }))
            .into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Debug, Deserialize, Default)]
struct LuBody {
    /// Absent = tous les résultats de la veille.
    ids: Option<Vec<i64>>,
}

async fn marquer_lus(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<LuBody>>,
) -> Response {
    let ids = body.and_then(|Json(b)| b.ids);
    if ids.as_ref().is_some_and(|i| i.len() > 1000) {
        return err(StatusCode::BAD_REQUEST, "Trop d'identifiants.");
    }
    match state.db.veille_marquer_lus(&id, ids).await {
        Ok(n) => Json(json!({ "marques": n })).into_response(),
        Err(e) => internal(e),
    }
}

async fn non_lus(State(state): State<Arc<AppState>>) -> Response {
    match state.db.veille_non_lus().await {
        Ok((total, par)) => {
            let map: serde_json::Map<String, Value> =
                par.into_iter().map(|(k, n)| (k, json!(n))).collect();
            Json(json!({ "total": total, "par_veille": map })).into_response()
        }
        Err(e) => internal(e),
    }
}

/// POST /:id/executer — vérifie maintenant (indépendamment de l'échéance).
async fn executer(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let veille = match state.db.veille_get(id).await {
        Ok(Some(v)) => v,
        Ok(None) => return err(StatusCode::NOT_FOUND, "Veille introuvable."),
        Err(e) => return internal(e),
    };
    let client = match state.juridique_state().client_async().await {
        Ok(c) => c,
        Err(r) => return r, // 503 explicite : identifiants PISTE non configurés
    };
    let fetcher = PisteFetcher::new(client);
    let report = run_one(&state.db, &fetcher, &veille, Utc::now(), today()).await;
    Json(report).into_response()
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/non-lus", get(non_lus))
        .route("/:id", axum::routing::put(update).delete(remove))
        .route("/:id/resultats", get(resultats))
        .route("/:id/resultats/lu", post(marquer_lus))
        .route("/:id/executer", post(executer))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use axum::{extract::Query as AxQuery, routing::get as ax_get, routing::post as ax_post};

    // ── Le futur du planificateur doit être `Send` (tauri::async_runtime::spawn) ──
    // `lib.rs` n'est pas compilé dans cette suite : cette assertion fait échouer la
    // compilation des tests si une évolution rend le futur non-`Send`.
    #[allow(dead_code)]
    fn scheduler_future_is_send(state: Arc<AppState>) {
        fn check<T: Send>(_t: T) {}
        check(run_scheduler(state));
    }

    // ── Aides ─────────────────────────────────────────────────────────────────

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn crit_lf(query: &str, fond: &str) -> Criteres {
        Criteres {
            source: "legifrance".into(),
            query: query.into(),
            fond: Some(fond.into()),
            ..Default::default()
        }
    }

    fn veille(id: &str) -> Veille {
        Veille {
            id: id.into(),
            nom: "Test".into(),
            criteres: crit_lf("télétravail", "LODA_DATE"),
            active: true,
            frequence_heures: 24,
            depuis: "2026-09-01".into(),
            created_at: "2026-09-01T00:00:00Z".into(),
            last_run_at: None,
            last_success_at: None,
            last_error: None,
            last_notice: None,
            non_lus: 0,
            total: 0,
        }
    }

    struct TmpDb {
        db: Database,
        _dir: crate::mcp_pack::tests::TmpDir,
    }

    async fn tmp_db() -> TmpDb {
        let dir = crate::mcp_pack::tests::TmpDir::new("veille");
        let db = Database::new(&dir.0.join("t.db"), &"ab".repeat(32))
            .await
            .unwrap();
        TmpDb { db, _dir: dir }
    }

    async fn make(db: &Database, nom: &str, c: Criteres) -> Veille {
        db.veille_create(
            NewVeille {
                nom: nom.into(),
                criteres: c,
                active: true,
                frequence_heures: 24,
                depuis: "2026-09-01".into(),
            },
            "2026-09-01T00:00:00Z",
        )
        .await
        .unwrap()
    }

    fn item(id: &str, titre: &str) -> NewResultat {
        NewResultat {
            texte_id: id.into(),
            source: "legifrance".into(),
            fond: "LODA_DATE".into(),
            titre: titre.into(),
            nature: Some("DECRET".into()),
            date_texte: Some("2026-09-20".into()),
        }
    }

    // ── Validation ────────────────────────────────────────────────────────────

    #[test]
    fn criteria_require_keywords_and_a_supported_source() {
        assert!(crit_lf("", "LODA_DATE")
            .validated()
            .unwrap_err()
            .contains("mots-clés"));
        assert!(crit_lf("   \n ", "LODA_DATE").validated().is_err());
        assert!(crit_lf("x", "BOCC")
            .validated()
            .unwrap_err()
            .contains("Fonds non pris en charge"));
        assert!(crit_lf("x", "").validated().is_err());
        assert!(Criteres {
            source: "web".into(),
            query: "x".into(),
            ..Default::default()
        }
        .validated()
        .unwrap_err()
        .contains("Source inconnue"));
    }

    #[test]
    fn criteria_are_normalised_and_irrelevant_fields_dropped() {
        let c = Criteres {
            source: "legifrance".into(),
            query: "  télétravail   et\tfrais ".into(),
            fond: Some("loda_date".into()),
            natures: vec![" decret ".into(), "LOI".into(), "decret".into(), "".into()],
            code: Some("Code du travail".into()),
            juridictions: vec!["cc".into()],
        }
        .validated()
        .unwrap();
        assert_eq!(c.query, "télétravail et frais");
        assert_eq!(c.fond.as_deref(), Some("LODA_DATE"));
        assert_eq!(c.natures, ["DECRET", "LOI"], "triées, dédoublonnées");
        assert!(
            c.code.is_none() && c.juridictions.is_empty(),
            "code (CODE_DATE) et juridictions (Judilibre) ne valent pas ici"
        );

        let code = crit_lf("congés", "CODE_DATE");
        let code = Criteres {
            code: Some("Code du travail".into()),
            natures: vec!["LOI".into()],
            ..code
        }
        .validated()
        .unwrap();
        assert_eq!(code.code.as_deref(), Some("Code du travail"));
        assert!(
            code.natures.is_empty(),
            "les natures ne s'appliquent pas aux codes"
        );

        let j = Criteres {
            source: "judilibre".into(),
            query: "licenciement".into(),
            juridictions: vec![" CA ".into(), "cc".into(), "ca".into()],
            fond: Some("JORF".into()),
            ..Default::default()
        }
        .validated()
        .unwrap();
        assert_eq!(j.juridictions, ["ca", "cc"]);
        assert!(j.fond.is_none());
    }

    #[test]
    fn criteria_reject_injection_like_values() {
        let bad_nature = Criteres {
            natures: vec!["LOI'; DROP".into()],
            ..crit_lf("x", "JORF")
        };
        assert!(bad_nature.validated().is_err());
        let bad_jur = Criteres {
            source: "judilibre".into(),
            query: "x".into(),
            juridictions: vec!["../cc".into()],
            ..Default::default()
        };
        assert!(bad_jur.validated().is_err());
        let long = crit_lf(&"a".repeat(1000), "JORF").validated().unwrap();
        assert_eq!(long.query.chars().count(), MAX_QUERY, "requête plafonnée");
    }

    fn input(nom: &str, freq: Option<u32>, depuis: Option<&str>) -> VeilleInput {
        VeilleInput {
            nom: nom.into(),
            criteres: crit_lf("x", "JORF"),
            active: None,
            frequence_heures: freq,
            depuis: depuis.map(String::from),
        }
    }

    #[test]
    fn input_validation_protects_the_piste_quota() {
        let today = d("2026-10-05");
        assert!(validate_input(input("  ", None, None), today).is_err());
        let ok = validate_input(input("Veille TT", None, None), today).unwrap();
        assert_eq!(
            (ok.frequence_heures, ok.depuis.as_str(), ok.active),
            (FREQ_DEFAULT_H, "2026-09-21", true),
            "défauts : 24 h, 14 jours d'historique"
        );
        assert!(
            validate_input(input("x", Some(1), None), today)
                .unwrap_err()
                .contains("Fréquence"),
            "pas plus d'une fois toutes les 6 h"
        );
        assert!(validate_input(input("x", Some(FREQ_MIN_H), None), today).is_ok());
        assert!(validate_input(input("x", Some(FREQ_MAX_H + 1), None), today).is_err());
        assert!(validate_input(input("x", None, Some("2026-10-06")), today)
            .unwrap_err()
            .contains("futur"));
        assert!(validate_input(input("x", None, Some("2020-01-01")), today)
            .unwrap_err()
            .contains("ancienne"));
        assert!(validate_input(input("x", None, Some("pas une date")), today).is_err());
        assert_eq!(
            validate_input(input("x", None, Some("2026-01-15")), today)
                .unwrap()
                .depuis,
            "2026-01-15"
        );
        assert_eq!(
            validate_input(input(&"n".repeat(500), None, None), today)
                .unwrap()
                .nom
                .chars()
                .count(),
            MAX_NOM
        );
    }

    // ── Planification ─────────────────────────────────────────────────────────

    #[test]
    fn due_logic_first_run_frequency_retry_and_inactive() {
        let now = at("2026-10-05T12:00:00Z");
        let mut v = veille("a");
        assert!(is_due(&v, now), "jamais exécutée → à exécuter");

        v.last_run_at = Some("2026-10-05T00:00:00Z".into()); // il y a 12 h, fréquence 24 h
        assert!(!is_due(&v, now));
        v.last_run_at = Some("2026-10-04T11:00:00Z".into()); // 25 h
        assert!(is_due(&v, now));

        // Échec récent : on réessaie après 1 h, sans attendre 24 h.
        v.last_run_at = Some("2026-10-05T11:30:00Z".into()); // il y a 30 min
        v.last_error = Some("panne".into());
        assert!(!is_due(&v, now), "30 min : trop tôt");
        v.last_run_at = Some("2026-10-05T11:01:00Z".into()); // 59 min
        assert!(!is_due(&v, now));
        v.last_run_at = Some("2026-10-05T10:59:00Z".into()); // 61 min
        assert!(
            is_due(&v, now),
            "retenté après l'échec, sans attendre les 24 h"
        );
        v.frequence_heures = 6;
        v.last_run_at = Some("2026-10-05T11:30:00Z".into());
        assert!(!is_due(&v, now));

        v.active = false;
        assert!(!is_due(&v, now), "une veille suspendue ne tourne pas");
        v.active = true;
        v.last_run_at = Some("date illisible".into());
        assert!(
            is_due(&v, now),
            "horodatage corrompu → on exécute plutôt que de bloquer"
        );
    }

    #[test]
    fn window_first_run_uses_depuis_then_overlaps_the_last_success() {
        let today = d("2026-10-05");
        let mut v = veille("a"); // depuis 2026-09-01
        assert_eq!(
            search_window(&v, today),
            (d("2026-09-01"), today),
            "1re exécution : depuis la date de départ"
        );

        v.last_success_at = Some("2026-10-04T08:00:00Z".into());
        assert_eq!(
            search_window(&v, today).0,
            d("2026-09-20"),
            "dernier succès − 14 jours : un texte signé avant mais publié après est rattrapé"
        );

        v.last_success_at = Some("2026-09-05T08:00:00Z".into());
        assert_eq!(
            search_window(&v, today).0,
            d("2026-09-01"),
            "jamais avant `depuis`"
        );
    }

    #[test]
    fn window_catches_up_after_a_long_closure() {
        let today = d("2026-12-20");
        let mut v = veille("a");
        v.depuis = "2026-01-01".into();
        v.last_success_at = Some("2026-10-05T08:00:00Z".into()); // application fermée 2,5 mois
        let (start, end) = search_window(&v, today);
        assert_eq!(
            (start, end),
            (d("2026-09-21"), today),
            "la fenêtre couvre toute la période de fermeture"
        );
    }

    #[test]
    fn window_is_never_inverted() {
        let today = d("2026-10-05");
        let mut v = veille("a");
        v.depuis = "2026-12-01".into(); // incohérent (futur) : ne doit pas produire début > fin
        let (s, e) = search_window(&v, today);
        assert!(s <= e);
        v.depuis = "illisible".into();
        let (s, e) = search_window(&v, today);
        assert_eq!((s, e), (today - ChronoDuration::days(BACKFILL_DAYS), today));
    }

    #[test]
    fn sorts_are_valid_for_every_supported_fond() {
        for f in FONDS {
            let fond = crate::juridique::routes::parse_fond(f);
            assert_ne!(format!("{fond:?}"), "All", "{f} doit être reconnu");
            assert!(
                fond.valid_sorts().contains(&sort_for(f)),
                "{f} : tri {} invalide",
                sort_for(f)
            );
        }
    }

    // ── Extraction ────────────────────────────────────────────────────────────

    #[test]
    fn dates_are_normalised_from_every_api_format() {
        assert_eq!(
            normalize_date(&json!("2026-09-20")).as_deref(),
            Some("2026-09-20")
        );
        assert_eq!(
            normalize_date(&json!("2026-09-20T00:00:00.000+0000")).as_deref(),
            Some("2026-09-20")
        );
        assert_eq!(
            normalize_date(&json!(1789948800000i64)).as_deref(),
            Some("2026-09-21")
        );
        assert_eq!(
            normalize_date(&json!("1789948800000")).as_deref(),
            Some("2026-09-21")
        );
        assert_eq!(
            normalize_date(&json!(1789948800)).as_deref(),
            Some("2026-09-21"),
            "secondes"
        );
        assert_eq!(
            normalize_date(&json!(32472144000000i64)),
            None,
            "date sentinelle « sans fin »"
        );
        for bad in [json!(null), json!(""), json!("hier"), json!(0), json!(true)] {
            assert_eq!(normalize_date(&bad), None, "{bad}");
        }
    }

    #[test]
    fn legifrance_texts_one_item_per_text_with_clean_titles() {
        let resp = json!({ "totalResultNumber": 3, "results": [
            { "titles": [{"id": "JORFTEXT000050000001", "cid": "JORFTEXT000050000001", "title": "<mark>Décret</mark> n° 2026-900 du 19 septembre 2026 relatif au <mark>télétravail</mark>"}],
              "nature": "DECRET", "dateSignature": "2026-09-19", "sections": [{"extracts": [{"id": "JORFARTI000050000002"}]}] },
            { "titles": [{"id": "LEGITEXT000050000003", "title": "Arrêté du 1er octobre 2026"}], "dateSignature": 1790812800000i64, "nature": "ARRETE" },
            { "sections": [{"extracts": [{"id": "x"}]}] },
            { "id": "JORFTEXT000050000009", "title": "Sans titres", "datePublication": "2026-10-02" },
        ]});
        let items = extract_legifrance(&resp, "LODA_DATE");
        assert_eq!(
            items.len(),
            3,
            "un résultat sans identifiant de texte est écarté"
        );
        assert_eq!(
            items[0].texte_id, "JORFTEXT000050000001",
            "identifiant du TEXTE, pas de l'article trouvé"
        );
        assert_eq!(
            items[0].titre, "Décret n° 2026-900 du 19 septembre 2026 relatif au télétravail",
            "balises <mark> retirées"
        );
        assert_eq!(
            (items[0].nature.as_deref(), items[0].date_texte.as_deref()),
            (Some("DECRET"), Some("2026-09-19"))
        );
        assert_eq!(items[1].date_texte.as_deref(), Some("2026-10-01"));
        assert_eq!(
            (
                items[2].texte_id.as_str(),
                items[2].titre.as_str(),
                items[2].date_texte.as_deref()
            ),
            ("JORFTEXT000050000009", "Sans titres", Some("2026-10-02"))
        );
        assert!(items
            .iter()
            .all(|i| i.source == "legifrance" && i.fond == "LODA_DATE"));
    }

    #[test]
    fn code_veille_yields_one_item_per_article_version() {
        let resp = json!({ "results": [{
            "titles": [{"id": "LEGITEXT000006072050", "title": "Code du travail"}],
            "sections": [{ "title": "Chapitre", "extracts": [
                {"id": "LEGIARTI000050000010", "num": "L1222-9", "dateVersion": "2026-10-01"},
                {"id": "LEGIARTI000050000011", "title": "Article L1222-10"},
                {"num": "sans id"},
            ]}]
        }]});
        let items = extract_legifrance(&resp, "CODE_DATE");
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].texte_id, "LEGIARTI000050000010",
            "l'identifiant de VERSION : une modification = un nouveau résultat"
        );
        assert_eq!(items[0].titre, "Code du travail — Article L1222-9");
        assert_eq!(
            items[1].titre, "Code du travail — Article L1222-10",
            "« Article » du titre non doublé"
        );
        assert_eq!(
            (items[0].nature.as_deref(), items[0].date_texte.as_deref()),
            (Some("ARTICLE"), Some("2026-10-01"))
        );
    }

    #[test]
    fn judilibre_decisions_get_a_readable_title() {
        let resp = json!({ "results": [
            {"id": "644f5a3e1bce0b5c0e5e7a1f", "jurisdiction": "cc", "chamber": "Chambre sociale", "number": "24-12.345", "decision_date": "2026-09-30", "solution": "cassation"},
            {"id": "644f5a3e1bce0b5c0e5e7a20", "jurisdiction": "ca"},
            {"jurisdiction": "cc"},
        ]});
        let items = extract_judilibre(&resp);
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].titre,
            "Cour de cassation — Chambre sociale — n° 24-12.345 (2026-09-30)"
        );
        assert_eq!(
            (items[0].source.as_str(), items[0].nature.as_deref()),
            ("judilibre", Some("cassation"))
        );
        assert_eq!(items[1].titre, "Cour d'appel");
    }

    #[test]
    fn extraction_tolerates_odd_shapes() {
        for v in [
            json!({}),
            json!({"results": null}),
            json!({"results": [null, 3, "x", {}]}),
            json!(null),
        ] {
            assert!(extract_legifrance(&v, "JORF").is_empty());
            assert!(extract_judilibre(&v).is_empty());
        }
    }

    // ── Stockage ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn crud_roundtrip_and_counts() {
        let t = tmp_db().await;
        let v = make(&t.db, "Télétravail", crit_lf("télétravail", "LODA_DATE")).await;
        assert_eq!(
            (v.non_lus, v.total, v.active, v.frequence_heures),
            (0, 0, true, 24)
        );
        assert_eq!(v.criteres.query, "télétravail");

        let n =
            t.db.veille_insert_resultats(
                &v.id,
                vec![item("A", "a"), item("B", "b")],
                "2026-10-02T08:00:00Z",
            )
            .await
            .unwrap();
        assert_eq!(n, 2);
        let got = t.db.veille_get(v.id.clone()).await.unwrap().unwrap();
        assert_eq!((got.non_lus, got.total), (2, 2));

        let upd =
            t.db.veille_update(
                v.id.clone(),
                NewVeille {
                    nom: "Renommée".into(),
                    criteres: v.criteres.clone(),
                    active: false,
                    frequence_heures: 12,
                    depuis: "2026-09-01".into(),
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                upd.nom.as_str(),
                upd.active,
                upd.frequence_heures,
                upd.total
            ),
            ("Renommée", false, 12, 2)
        );
        assert!(t
            .db
            .veille_update(
                "inconnue".into(),
                NewVeille {
                    nom: "x".into(),
                    criteres: v.criteres.clone(),
                    active: true,
                    frequence_heures: 24,
                    depuis: "2026-09-01".into()
                }
            )
            .await
            .unwrap()
            .is_none());

        assert!(t.db.veille_delete(v.id.clone()).await.unwrap());
        assert!(
            !t.db.veille_delete(v.id.clone()).await.unwrap(),
            "déjà supprimée"
        );
        assert!(
            t.db.veille_resultats(&v.id, false, 50, 0)
                .await
                .unwrap()
                .is_empty(),
            "ses résultats disparaissent avec elle"
        );
        assert!(t.db.veille_list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_result_is_never_reported_twice_for_the_same_veille() {
        let t = tmp_db().await;
        let v = make(&t.db, "V", crit_lf("x", "JORF")).await;
        let w = make(&t.db, "W", crit_lf("y", "JORF")).await;
        assert_eq!(
            t.db.veille_insert_resultats(&v.id, vec![item("A", "a")], "2026-10-02T08:00:00Z")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            t.db.veille_insert_resultats(
                &v.id,
                vec![item("A", "a"), item("B", "b")],
                "2026-10-03T08:00:00Z"
            )
            .await
            .unwrap(),
            1,
            "A est déjà connu : seul B est nouveau"
        );
        assert_eq!(
            t.db.veille_insert_resultats(&w.id, vec![item("A", "a")], "2026-10-03T08:00:00Z")
                .await
                .unwrap(),
            1,
            "chaque veille a sa propre mémoire"
        );
        // Une lecture ne rend pas un texte « nouveau » à la passe suivante.
        t.db.veille_marquer_lus(&v.id, None).await.unwrap();
        assert_eq!(
            t.db.veille_insert_resultats(
                &v.id,
                vec![item("A", "a"), item("B", "b")],
                "2026-10-04T08:00:00Z"
            )
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            t.db.veille_get(v.id.clone())
                .await
                .unwrap()
                .unwrap()
                .non_lus,
            0
        );
    }

    #[tokio::test]
    async fn read_state_pagination_and_global_unread_counts() {
        let t = tmp_db().await;
        let v = make(&t.db, "V", crit_lf("x", "JORF")).await;
        let w = make(&t.db, "W", crit_lf("y", "JORF")).await;
        let items: Vec<_> = (1..=5)
            .map(|i| item(&format!("T{i}"), &format!("t{i}")))
            .collect();
        t.db.veille_insert_resultats(&v.id, items, "2026-10-02T08:00:00Z")
            .await
            .unwrap();
        t.db.veille_insert_resultats(&w.id, vec![item("Z", "z")], "2026-10-02T08:00:00Z")
            .await
            .unwrap();

        let all = t.db.veille_resultats(&v.id, false, 50, 0).await.unwrap();
        assert_eq!(
            all.iter().map(|r| r.texte_id.as_str()).collect::<Vec<_>>(),
            ["T5", "T4", "T3", "T2", "T1"],
            "le plus récemment détecté d'abord"
        );
        assert_eq!(
            t.db.veille_resultats(&v.id, false, 2, 1)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            t.db.veille_resultats(&v.id, false, 2, 4)
                .await
                .unwrap()
                .len(),
            1
        );

        let (total, par) = t.db.veille_non_lus().await.unwrap();
        assert_eq!(total, 6);
        assert_eq!(par.len(), 2);

        // Marquer certains résultats lus.
        let ids: Vec<i64> = all.iter().take(2).map(|r| r.id).collect();
        assert_eq!(
            t.db.veille_marquer_lus(&v.id, Some(ids.clone()))
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            t.db.veille_marquer_lus(&v.id, Some(ids)).await.unwrap(),
            0,
            "déjà lus"
        );
        assert_eq!(
            t.db.veille_resultats(&v.id, true, 50, 0)
                .await
                .unwrap()
                .len(),
            3,
            "filtre « non lus »"
        );
        // Un identifiant d'une AUTRE veille ne peut pas être marqué via celle-ci.
        let other = t.db.veille_resultats(&w.id, false, 50, 0).await.unwrap()[0].id;
        assert_eq!(
            t.db.veille_marquer_lus(&v.id, Some(vec![other]))
                .await
                .unwrap(),
            0
        );
        assert_eq!(t.db.veille_marquer_lus(&v.id, None).await.unwrap(), 3);
        assert_eq!(
            t.db.veille_non_lus().await.unwrap().0,
            1,
            "reste le résultat de W"
        );
    }

    #[tokio::test]
    async fn changing_criteria_restarts_the_search_but_keeps_history() {
        let t = tmp_db().await;
        let v = make(&t.db, "V", crit_lf("x", "JORF")).await;
        t.db.veille_insert_resultats(&v.id, vec![item("A", "a")], "2026-10-02T08:00:00Z")
            .await
            .unwrap();
        t.db.veille_record_run(
            &v.id,
            "2026-10-02T08:00:00Z",
            true,
            None,
            Some("note".into()),
        )
        .await
        .unwrap();
        let mk = |c: Criteres| NewVeille {
            nom: "V".into(),
            criteres: c,
            active: true,
            frequence_heures: 24,
            depuis: "2026-09-01".into(),
        };

        // Même critères (simple renommage / pause) : l'état d'exécution est conservé.
        let same =
            t.db.veille_update(v.id.clone(), mk(v.criteres.clone()))
                .await
                .unwrap()
                .unwrap();
        assert!(same.last_success_at.is_some() && same.last_notice.is_some());

        // Critères modifiés : la prochaine passe repart de `depuis`, l'historique reste.
        let changed =
            t.db.veille_update(v.id.clone(), mk(crit_lf("autre chose", "JORF")))
                .await
                .unwrap()
                .unwrap();
        assert!(
            changed.last_success_at.is_none()
                && changed.last_run_at.is_none()
                && changed.last_notice.is_none()
        );
        assert_eq!(changed.total, 1);
    }

    #[tokio::test]
    async fn record_run_tracks_success_and_failure() {
        let t = tmp_db().await;
        let v = make(&t.db, "V", crit_lf("x", "JORF")).await;
        t.db.veille_record_run(
            &v.id,
            "2026-10-02T08:00:00Z",
            false,
            Some("panne".into()),
            None,
        )
        .await
        .unwrap();
        let f = t.db.veille_get(v.id.clone()).await.unwrap().unwrap();
        assert_eq!(
            (
                f.last_run_at.as_deref(),
                f.last_error.as_deref(),
                f.last_success_at.as_deref()
            ),
            (Some("2026-10-02T08:00:00Z"), Some("panne"), None)
        );
        t.db.veille_record_run(&v.id, "2026-10-02T09:00:00Z", true, None, None)
            .await
            .unwrap();
        let ok = t.db.veille_get(v.id.clone()).await.unwrap().unwrap();
        assert_eq!(
            (ok.last_error, ok.last_success_at.as_deref()),
            (None, Some("2026-10-02T09:00:00Z")),
            "un succès efface l'erreur"
        );
    }

    #[tokio::test]
    async fn pruning_keeps_every_unread_result() {
        let t = tmp_db().await;
        let v = make(&t.db, "V", crit_lf("x", "JORF")).await;
        let n = (KEEP_SEEN + 30) as usize;
        let items: Vec<_> = (0..n).map(|i| item(&format!("T{i:05}"), "t")).collect();
        t.db.veille_insert_resultats(&v.id, items, "2026-10-02T08:00:00Z")
            .await
            .unwrap();
        assert_eq!(
            t.db.veille_prune(&v.id).await.unwrap(),
            0,
            "tout est non lu : rien n'est purgé"
        );
        t.db.veille_marquer_lus(&v.id, None).await.unwrap();
        assert_eq!(t.db.veille_prune(&v.id).await.unwrap(), 30);
        let left = t.db.veille_get(v.id.clone()).await.unwrap().unwrap();
        assert_eq!(left.total, KEEP_SEEN);
        // Les plus récents sont conservés.
        let newest = t.db.veille_resultats(&v.id, false, 1, 0).await.unwrap();
        assert_eq!(newest[0].texte_id, format!("T{:05}", n - 1));
    }

    #[tokio::test]
    async fn schema_migration_is_idempotent_and_survives_reopening() {
        let dir = crate::mcp_pack::tests::TmpDir::new("veille-reopen");
        let path = dir.0.join("t.db");
        let key = "cd".repeat(32);
        let id = {
            let db = Database::new(&path, &key).await.unwrap();
            let v = make(&db, "Persistée", crit_lf("x", "JORF")).await;
            db.veille_insert_resultats(&v.id, vec![item("A", "a")], "2026-10-02T08:00:00Z")
                .await
                .unwrap();
            v.id
        };
        let db = Database::new(&path, &key).await.unwrap(); // rejoue les CREATE IF NOT EXISTS
        let v = db.veille_get(id).await.unwrap().unwrap();
        assert_eq!((v.nom.as_str(), v.total, v.non_lus), ("Persistée", 1, 1));
    }

    // ── Exécution avec une source simulée ─────────────────────────────────────

    struct FakeFetcher {
        script: Mutex<VecDeque<Result<FetchOutcome, FetchError>>>,
        calls: Mutex<Vec<(String, NaiveDate, NaiveDate)>>,
    }
    impl FakeFetcher {
        fn new(script: Vec<Result<FetchOutcome, FetchError>>) -> Self {
            Self {
                script: Mutex::new(script.into()),
                calls: Mutex::default(),
            }
        }
    }
    impl Fetcher for FakeFetcher {
        fn fetch<'a>(
            &'a self,
            c: &'a Criteres,
            since: NaiveDate,
            until: NaiveDate,
        ) -> BoxFuture<'a, Result<FetchOutcome, FetchError>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push((c.query.clone(), since, until));
                self.script
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or(Ok(FetchOutcome::default()))
            })
        }
    }
    fn ok(items: Vec<NewResultat>) -> Result<FetchOutcome, FetchError> {
        Ok(FetchOutcome {
            items,
            truncated: false,
            total: None,
        })
    }

    #[tokio::test]
    async fn run_one_detects_only_what_is_new_and_records_the_pass() {
        let t = tmp_db().await;
        let v = make(&t.db, "V", crit_lf("télétravail", "LODA_DATE")).await;
        let f = FakeFetcher::new(vec![
            ok(vec![item("A", "a"), item("B", "b")]),
            ok(vec![item("B", "b"), item("C", "c")]),
        ]);

        let r1 = run_one(&t.db, &f, &v, at("2026-10-05T08:00:00Z"), d("2026-10-05")).await;
        assert_eq!((r1.nouveaux, r1.trouves, r1.error.is_none()), (2, 2, true));

        let v = t.db.veille_get(v.id.clone()).await.unwrap().unwrap();
        assert_eq!(v.last_success_at.as_deref(), Some("2026-10-05T08:00:00Z"));
        let r2 = run_one(&t.db, &f, &v, at("2026-10-06T08:00:00Z"), d("2026-10-06")).await;
        assert_eq!(
            (r2.nouveaux, r2.trouves),
            (1, 2),
            "B déjà connu : seul C est signalé"
        );

        let calls = f.calls.lock().unwrap().clone();
        assert_eq!(
            calls[0].1,
            d("2026-09-01"),
            "1re passe : depuis la date de départ"
        );
        assert_eq!(
            calls[1].1,
            d("2026-09-21"),
            "2e passe : dernier succès − 14 jours"
        );
        assert_eq!(t.db.veille_get(v.id).await.unwrap().unwrap().non_lus, 3);
    }

    #[tokio::test]
    async fn failures_never_lose_state_and_hide_technical_details() {
        let t = tmp_db().await;
        let v = make(&t.db, "V", crit_lf("x", "JORF")).await;
        let f = FakeFetcher::new(vec![Err(FetchError::Other(
            "PISTE POST https://api.piste.gouv.fr/... — HTTP 502: <html>secret</html>".into(),
        ))]);
        let r = run_one(&t.db, &f, &v, at("2026-10-05T08:00:00Z"), d("2026-10-05")).await;
        let msg = r.error.clone().unwrap();
        assert!(
            !msg.contains("piste.gouv.fr") && !msg.contains("502") && !msg.contains("secret"),
            "message utilisateur sans détail technique : {msg}"
        );
        assert!(!r.rate_limited);
        let after = t.db.veille_get(v.id.clone()).await.unwrap().unwrap();
        assert_eq!(
            (
                after.last_success_at.as_deref(),
                after.last_error.is_some(),
                after.last_run_at.is_some()
            ),
            (None, true, true)
        );
        // Au passage suivant, la fenêtre n'a pas bougé : rien n'est « sauté ».
        assert_eq!(search_window(&after, d("2026-10-05")).0, d("2026-09-01"));
    }

    #[tokio::test]
    async fn truncated_results_leave_a_visible_notice() {
        let t = tmp_db().await;
        let v = make(&t.db, "V", crit_lf("x", "JORF")).await;
        let f = FakeFetcher::new(vec![Ok(FetchOutcome {
            items: vec![item("A", "a")],
            truncated: true,
            total: Some(900),
        })]);
        let r = run_one(&t.db, &f, &v, at("2026-10-05T08:00:00Z"), d("2026-10-05")).await;
        assert!(r.truncated);
        let n =
            t.db.veille_get(v.id)
                .await
                .unwrap()
                .unwrap()
                .last_notice
                .unwrap();
        assert!(n.contains("200 résultats") && n.contains("Précisez"), "{n}");
    }

    #[tokio::test]
    async fn run_due_skips_inactive_and_not_due_and_stops_on_rate_limit() {
        let t = tmp_db().await;
        let a = make(&t.db, "A", crit_lf("a", "JORF")).await;
        let b = make(&t.db, "B", crit_lf("b", "JORF")).await;
        let c = make(&t.db, "C", crit_lf("c", "JORF")).await;
        let inactive = make(&t.db, "Off", crit_lf("off", "JORF")).await;
        t.db.veille_update(
            inactive.id.clone(),
            NewVeille {
                nom: "Off".into(),
                criteres: inactive.criteres.clone(),
                active: false,
                frequence_heures: 24,
                depuis: "2026-09-01".into(),
            },
        )
        .await
        .unwrap();
        // C a tourné il y a 1 h : pas à échéance.
        t.db.veille_record_run(&c.id, "2026-10-05T07:00:00Z", true, None, None)
            .await
            .unwrap();

        let f = FakeFetcher::new(vec![
            Err(FetchError::RateLimited("HTTP 429".into())),
            ok(vec![item("X", "x")]),
        ]);
        let tick = run_due(
            &t.db,
            &f,
            at("2026-10-05T08:00:00Z"),
            d("2026-10-05"),
            Duration::ZERO,
        )
        .await;
        let called: Vec<String> = f
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.0.clone())
            .collect();
        assert_eq!(
            called,
            ["a"],
            "la 1re veille a reçu un 429 : toute la série s'arrête, B n'est pas appelée"
        );
        assert!(tick.rate_limited && tick.runs.len() == 1);
        let _ = (a, b);

        // Série suivante (sans limite) : A (en échec, retentée après 1 h) et B tournent, ni C ni Off.
        let f2 = FakeFetcher::new(vec![]);
        let tick2 = run_due(
            &t.db,
            &f2,
            at("2026-10-05T10:00:00Z"),
            d("2026-10-05"),
            Duration::ZERO,
        )
        .await;
        let called2: Vec<String> = f2
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.0.clone())
            .collect();
        assert_eq!(called2, ["a", "b"]);
        assert!(!tick2.rate_limited);
    }

    #[tokio::test]
    async fn restart_after_a_long_closure_catches_up_in_one_pass() {
        let t = tmp_db().await;
        let v = make(&t.db, "V", crit_lf("x", "JORF")).await;
        // Dernier succès il y a 2 mois et demi (application fermée).
        t.db.veille_record_run(&v.id, "2026-10-05T08:00:00Z", true, None, None)
            .await
            .unwrap();
        let f = FakeFetcher::new(vec![ok((0..7)
            .map(|i| item(&format!("N{i}"), "n"))
            .collect())]);
        let tick = run_due(
            &t.db,
            &f,
            at("2026-12-20T09:00:00Z"),
            d("2026-12-20"),
            Duration::ZERO,
        )
        .await;
        assert_eq!(tick.runs[0].nouveaux, 7);
        let (_, since, until) = f.calls.lock().unwrap()[0].clone();
        assert_eq!(
            (since, until),
            (d("2026-09-21"), d("2026-12-20")),
            "une seule requête couvre toute la période de fermeture"
        );
    }

    // ── Source réelle contre de faux serveurs Légifrance / Judilibre ──────────

    #[derive(Clone, Default)]
    struct Mock {
        lf_bodies: Arc<Mutex<Vec<Value>>>,
        jud_queries: Arc<Mutex<Vec<std::collections::HashMap<String, String>>>>,
        /// nombre de résultats « fabriqués » (pour la pagination)
        total: usize,
        status_429: bool,
    }

    fn lf_result(i: usize) -> Value {
        json!({ "titles": [{"id": format!("JORFTEXT{i:012}"), "title": format!("Décret n° 2026-{i}")}], "nature": "DECRET", "dateSignature": "2026-09-30" })
    }

    async fn lf_search(State(m): State<Mock>, Json(body): Json<Value>) -> Response {
        if m.status_429 {
            return (StatusCode::TOO_MANY_REQUESTS, "slow down").into_response();
        }
        let page = body["recherche"]["pageNumber"].as_u64().unwrap_or(1) as usize;
        let size = body["recherche"]["pageSize"].as_u64().unwrap_or(10) as usize;
        m.lf_bodies.lock().unwrap().push(body);
        let start = (page - 1) * size;
        let results: Vec<Value> = (start..m.total.min(start + size)).map(lf_result).collect();
        Json(json!({ "totalResultNumber": m.total, "results": results })).into_response()
    }

    async fn jud_search(
        State(m): State<Mock>,
        AxQuery(q): AxQuery<std::collections::HashMap<String, String>>,
    ) -> Response {
        let page: usize = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(0);
        let size: usize = q
            .get("page_size")
            .and_then(|p| p.parse().ok())
            .unwrap_or(10);
        m.jud_queries.lock().unwrap().push(q);
        let start = page * size;
        let results: Vec<Value> = (start..m.total.min(start + size))
            .map(|i| json!({"id": format!("{i:024x}"), "jurisdiction": "cc", "decision_date": "2026-09-30", "number": format!("24-{i}")}))
            .collect();
        let next = (start + size < m.total).then_some(page + 1);
        Json(json!({ "total": m.total, "results": results, "next_batch": next })).into_response()
    }

    async fn spawn_mock(m: Mock) -> PisteFetcher {
        let app = Router::new()
            .route(
                "/oauth/token",
                ax_post(|| async { Json(json!({"access_token": "t", "expires_in": 3600})) }),
            )
            .route("/legifrance/search", ax_post(lf_search))
            .route("/judilibre/search", ax_get(jud_search))
            .with_state(m);
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        PisteFetcher::without_pause(PisteClient::new_for_test(&format!("http://{addr}")))
    }

    #[tokio::test]
    async fn legifrance_request_carries_dates_sort_nature_and_code_filters() {
        let m = Mock {
            total: 3,
            ..Default::default()
        };
        let bodies = m.lf_bodies.clone();
        let f = spawn_mock(m).await;
        let c = Criteres {
            natures: vec!["DECRET".into(), "LOI".into()],
            ..crit_lf("télétravail", "LODA_DATE")
        };
        let out = f.fetch(&c, d("2026-09-21"), d("2026-10-05")).await.unwrap();
        assert_eq!(
            (out.items.len(), out.truncated, out.total),
            (3, false, Some(3))
        );

        let b = bodies.lock().unwrap()[0].clone();
        assert_eq!(b["fond"], "LODA_DATE");
        assert_eq!(b["recherche"]["sort"], "PUBLICATION_DATE_DESC");
        assert_eq!(b["recherche"]["pageSize"], PAGE_SIZE);
        let raw = b.to_string();
        assert!(
            raw.contains("2026-09-21") && raw.contains("2026-10-05"),
            "fenêtre de dates transmise : {raw}"
        );
        assert!(
            raw.contains("télétravail") && raw.contains("DECRET") && raw.contains("LOI"),
            "{raw}"
        );

        // CODE_DATE : filtre sur le nom du code.
        let m2 = Mock {
            total: 1,
            ..Default::default()
        };
        let bodies2 = m2.lf_bodies.clone();
        let f2 = spawn_mock(m2).await;
        let c2 = Criteres {
            code: Some("Code du travail".into()),
            ..crit_lf("congés", "CODE_DATE")
        };
        f2.fetch(&c2, d("2026-09-21"), d("2026-10-05"))
            .await
            .unwrap();
        let raw2 = bodies2.lock().unwrap()[0].to_string();
        assert!(
            raw2.contains("TEXT_NOM_CODE") && raw2.contains("Code du travail"),
            "{raw2}"
        );
    }

    #[tokio::test]
    async fn legifrance_pagination_stops_when_complete_and_flags_truncation() {
        // 120 résultats : 3 pages de 50 (50 + 50 + 20) → complet.
        let m = Mock {
            total: 120,
            ..Default::default()
        };
        let bodies = m.lf_bodies.clone();
        let f = spawn_mock(m).await;
        let out = f
            .fetch(&crit_lf("x", "JORF"), d("2026-09-21"), d("2026-10-05"))
            .await
            .unwrap();
        assert_eq!((out.items.len(), out.truncated), (120, false));
        assert_eq!(
            bodies.lock().unwrap().len(),
            3,
            "pas d'appel superflu après la dernière page"
        );

        // 1000 résultats : plafonné à MAX_PAGES pages, et signalé.
        let m = Mock {
            total: 1000,
            ..Default::default()
        };
        let bodies = m.lf_bodies.clone();
        let f = spawn_mock(m).await;
        let out = f
            .fetch(&crit_lf("x", "JORF"), d("2026-09-21"), d("2026-10-05"))
            .await
            .unwrap();
        assert_eq!(
            (out.items.len(), out.truncated),
            ((MAX_PAGES * PAGE_SIZE) as usize, true)
        );
        assert_eq!(bodies.lock().unwrap().len(), MAX_PAGES as usize);

        // Pile 50 résultats : une seule page suffit.
        let m = Mock {
            total: 50,
            ..Default::default()
        };
        let bodies = m.lf_bodies.clone();
        let f = spawn_mock(m).await;
        assert_eq!(
            f.fetch(&crit_lf("x", "JORF"), d("2026-09-21"), d("2026-10-05"))
                .await
                .unwrap()
                .items
                .len(),
            50
        );
        assert_eq!(bodies.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rate_limit_from_piste_is_recognised() {
        let f = spawn_mock(Mock {
            status_429: true,
            ..Default::default()
        })
        .await;
        let e = f
            .fetch(&crit_lf("x", "JORF"), d("2026-09-21"), d("2026-10-05"))
            .await
            .unwrap_err();
        assert!(matches!(e, FetchError::RateLimited(_)), "{e:?}");
    }

    #[tokio::test]
    async fn judilibre_request_and_pagination() {
        let m = Mock {
            total: 70,
            ..Default::default()
        };
        let qs = m.jud_queries.clone();
        let f = spawn_mock(m).await;
        let c = Criteres {
            source: "judilibre".into(),
            query: "licenciement".into(),
            juridictions: vec!["cc".into(), "ca".into()],
            ..Default::default()
        };
        let out = f.fetch(&c, d("2026-09-21"), d("2026-10-05")).await.unwrap();
        assert_eq!((out.items.len(), out.truncated), (70, false));
        let q = qs.lock().unwrap();
        assert_eq!(q.len(), 2, "50 + 20 : deux pages");
        assert_eq!(q[0]["query"], "licenciement");
        assert_eq!(
            (q[0]["date_start"].as_str(), q[0]["date_end"].as_str()),
            ("2026-09-21", "2026-10-05")
        );
        assert_eq!(
            (
                q[0]["sort"].as_str(),
                q[0]["order"].as_str(),
                q[0]["page"].as_str()
            ),
            ("date", "desc", "0")
        );
        assert_eq!(q[1]["page"], "1");
    }

    /// Bout en bout : fenêtre → requête PISTE simulée → enregistrement → dédoublonnage.
    #[tokio::test]
    async fn end_to_end_with_the_real_fetcher() {
        let t = tmp_db().await;
        let m = Mock {
            total: 4,
            ..Default::default()
        };
        let f = spawn_mock(m).await;
        let v = make(
            &t.db,
            "Décrets télétravail",
            crit_lf("télétravail", "LODA_DATE"),
        )
        .await;
        let r1 = run_one(&t.db, &f, &v, at("2026-10-05T08:00:00Z"), d("2026-10-05")).await;
        assert_eq!((r1.nouveaux, r1.error.is_none()), (4, true));
        let v = t.db.veille_get(v.id).await.unwrap().unwrap();
        let r2 = run_one(&t.db, &f, &v, at("2026-10-06T08:00:00Z"), d("2026-10-06")).await;
        assert_eq!(
            (r2.nouveaux, r2.trouves),
            (0, 4),
            "mêmes résultats : rien de nouveau"
        );
        let res = t.db.veille_resultats(&v.id, true, 50, 0).await.unwrap();
        assert_eq!(res.len(), 4);
        assert!(
            res[0].texte_id.starts_with("JORFTEXT") && res[0].titre.starts_with("Décret n° 2026-")
        );
    }

    // ── Routes HTTP (routeur réel de l'application) ───────────────────────────

    struct Api {
        base: String,
        state: Arc<AppState>,
        _db: TmpDb,
    }

    async fn spawn_api() -> Api {
        let t = tmp_db().await;
        // `Database` n'est pas Clone : on rouvre la même base pour le routeur.
        let path = t._dir.0.join("t.db");
        let db = Database::new(&path, &"ab".repeat(32)).await.unwrap();
        let (router, state) = crate::api::build_router(db, std::path::PathBuf::from("prompts.yml"));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        // Le routeur est protégé par `security::guard` : on retient le jeton de CE serveur de
        // test (un par test, port éphémère) pour que `call` puisse s'authentifier.
        TEST_TOKENS
            .lock()
            .unwrap()
            .insert(format!("http://{addr}"), state.api_token.clone());
        tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
        Api {
            base: format!("http://{addr}/api/veilles"),
            state,
            _db: t,
        }
    }

    /// Jeton d'API de chaque serveur de test, par origine (`http://127.0.0.1:<port>`).
    static TEST_TOKENS: once_cell::sync::Lazy<
        std::sync::Mutex<std::collections::HashMap<String, String>>,
    > = once_cell::sync::Lazy::new(Default::default);

    async fn call(method: &str, url: &str, body: Option<Value>) -> (u16, Value) {
        let c = reqwest::Client::new();
        let rb = match method {
            "GET" => c.get(url),
            "POST" => c.post(url),
            "PUT" => c.put(url),
            "DELETE" => c.delete(url),
            _ => unreachable!(),
        };
        // Garde de l'API locale : en-tête Host attendu (port officiel, pas le port éphémère du
        // test) + jeton `X-Juridix-Token`.
        let origin: String = url.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
        let token = TEST_TOKENS
            .lock()
            .unwrap()
            .get(&origin)
            .cloned()
            .unwrap_or_default();
        let rb = rb
            .header(
                reqwest::header::HOST,
                format!("127.0.0.1:{}", crate::config::api_port()),
            )
            .header(crate::security::TOKEN_HEADER, token);
        let rb = match body {
            Some(b) => rb.json(&b),
            None => rb,
        };
        let r = rb.send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }

    fn payload(nom: &str, query: &str) -> Value {
        json!({ "nom": nom, "criteres": { "source": "legifrance", "query": query, "fond": "LODA_DATE", "natures": ["decret"] }, "frequence_heures": 12 })
    }

    #[tokio::test]
    async fn http_crud_validation_and_error_codes() {
        let api = spawn_api().await;
        let (s, b) = call("GET", &api.base, None).await;
        assert_eq!(s, 200);
        assert_eq!(b["veilles"].as_array().unwrap().len(), 0);
        assert_eq!(b["limites"]["frequence_min_h"], FREQ_MIN_H);
        assert!(b["limites"]["fonds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "CODE_DATE"));

        // Création valide : 201, critères normalisés.
        let (s, v) = call(
            "POST",
            &api.base,
            Some(payload("  Télétravail  ", "télétravail")),
        )
        .await;
        assert_eq!(s, 201, "{v}");
        assert_eq!(
            (
                v["nom"].as_str(),
                v["frequence_heures"].as_u64(),
                v["active"].as_bool()
            ),
            (Some("Télétravail"), Some(12), Some(true))
        );
        assert_eq!(v["criteres"]["natures"], json!(["DECRET"]));
        let id = v["id"].as_str().unwrap().to_string();

        // Validations → 400 avec un message lisible.
        for (label, body, needle) in [
            ("sans mots-clés", payload("x", ""), "mots-clés"),
            ("nom vide", payload("  ", "x"), "nom"),
            (
                "fréquence trop basse",
                json!({"nom": "x", "criteres": {"source": "legifrance", "query": "x", "fond": "JORF"}, "frequence_heures": 1}),
                "Fréquence",
            ),
            (
                "fonds interdit",
                json!({"nom": "x", "criteres": {"source": "legifrance", "query": "x", "fond": "BOCC"}}),
                "Fonds",
            ),
            (
                "source inconnue",
                json!({"nom": "x", "criteres": {"source": "web", "query": "x"}}),
                "Source",
            ),
        ] {
            let (s, b) = call("POST", &api.base, Some(body)).await;
            assert_eq!(s, 400, "{label}");
            assert!(
                b["error"].as_str().unwrap().contains(needle),
                "{label} : {b}"
            );
        }
        // Corps mal formé : rejeté par axum (4xx), pas de 500.
        let (s, _) = call("POST", &api.base, Some(json!({"nom": 3}))).await;
        assert!((400..500).contains(&s), "{s}");

        // Mise à jour, absente → 404, suppression.
        let (s, u) = call("PUT", &format!("{}/{id}", api.base), Some(json!({"nom": "Renommée", "criteres": {"source": "legifrance", "query": "télétravail", "fond": "LODA_DATE", "natures": ["decret"]}, "active": false}))).await;
        assert_eq!(
            (s, u["nom"].as_str(), u["active"].as_bool()),
            (200, Some("Renommée"), Some(false))
        );
        assert_eq!(
            call(
                "PUT",
                &format!("{}/inconnue", api.base),
                Some(payload("x", "x"))
            )
            .await
            .0,
            404
        );
        assert_eq!(
            call("DELETE", &format!("{}/{id}", api.base), None).await.0,
            200
        );
        assert_eq!(
            call("DELETE", &format!("{}/{id}", api.base), None).await.0,
            404
        );
        assert_eq!(
            call("GET", &format!("{}/{id}/resultats", api.base), None)
                .await
                .0,
            404
        );
    }

    #[tokio::test]
    async fn http_veille_count_is_capped() {
        let api = spawn_api().await;
        for i in 0..MAX_VEILLES {
            assert_eq!(
                call("POST", &api.base, Some(payload(&format!("V{i}"), "x")))
                    .await
                    .0,
                201
            );
        }
        let (s, b) = call("POST", &api.base, Some(payload("de trop", "x"))).await;
        assert_eq!(s, 400);
        assert!(b["error"].as_str().unwrap().contains("maximal"), "{b}");
    }

    #[tokio::test]
    async fn http_run_read_and_badge_flow() {
        let api = spawn_api().await;
        // Sans identifiants PISTE : 503 explicite, rien n'est enregistré comme échec de la veille.
        let (_, v) = call("POST", &api.base, Some(payload("Décrets", "télétravail"))).await;
        let id = v["id"].as_str().unwrap().to_string();
        let (s, b) = call("POST", &format!("{}/{id}/executer", api.base), None).await;
        assert_eq!(s, 503, "{b}");

        // Avec un (faux) Légifrance.
        let m = Mock {
            total: 3,
            ..Default::default()
        };
        let app = Router::new()
            .route(
                "/oauth/token",
                ax_post(|| async { Json(json!({"access_token": "t", "expires_in": 3600})) }),
            )
            .route("/legifrance/search", ax_post(lf_search))
            .with_state(m);
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        *api.state.juridique.piste_client.write().await =
            Some(PisteClient::new_for_test(&format!("http://{addr}")));

        // Aucun non lu au départ ; la route statique /non-lus ne doit pas être prise pour un :id.
        let (s, b) = call("GET", &format!("{}/non-lus", api.base), None).await;
        assert_eq!((s, b["total"].as_i64()), (200, Some(0)));

        let (s, r) = call("POST", &format!("{}/{id}/executer", api.base), None).await;
        assert_eq!(
            (s, r["nouveaux"].as_u64(), r["error"].is_null()),
            (200, Some(3), true),
            "{r}"
        );

        let (_, b) = call("GET", &format!("{}/non-lus", api.base), None).await;
        assert_eq!(
            (b["total"].as_i64(), b["par_veille"][&id].as_i64()),
            (Some(3), Some(3))
        );

        let (_, list) = call("GET", &api.base, None).await;
        assert_eq!(
            (
                list["veilles"][0]["non_lus"].as_i64(),
                list["veilles"][0]["total"].as_i64()
            ),
            (Some(3), Some(3))
        );
        assert!(list["veilles"][0]["last_success_at"].is_string());

        let (s, res) = call("GET", &format!("{}/{id}/resultats?limit=2", api.base), None).await;
        assert_eq!(
            (
                s,
                res["resultats"].as_array().unwrap().len(),
                res["total"].as_i64()
            ),
            (200, 2, Some(3))
        );
        let first = res["resultats"][0].clone();
        assert!(
            first["texte_id"].as_str().unwrap().starts_with("JORFTEXT") && first["vu"] == false
        );

        // Marquer un résultat lu, puis tout le reste (corps absent = tous).
        let (_, m1) = call(
            "POST",
            &format!("{}/{id}/resultats/lu", api.base),
            Some(json!({"ids": [first["id"]]})),
        )
        .await;
        assert_eq!(m1["marques"], 1);
        let (_, nl) = call(
            "GET",
            &format!("{}/{id}/resultats?non_lus=true", api.base),
            None,
        )
        .await;
        assert_eq!(nl["resultats"].as_array().unwrap().len(), 2);
        let (_, m2) = call("POST", &format!("{}/{id}/resultats/lu", api.base), None).await;
        assert_eq!(m2["marques"], 2);
        assert_eq!(
            call("GET", &format!("{}/non-lus", api.base), None).await.1["total"],
            0
        );

        // Une 2e exécution ne re-signale rien.
        let (_, r2) = call("POST", &format!("{}/{id}/executer", api.base), None).await;
        assert_eq!(
            (r2["nouveaux"].as_u64(), r2["trouves"].as_u64()),
            (Some(0), Some(3))
        );
        // Trop d'identifiants : refusé.
        let many: Vec<i64> = (0..1001).collect();
        assert_eq!(
            call(
                "POST",
                &format!("{}/{id}/resultats/lu", api.base),
                Some(json!({"ids": many}))
            )
            .await
            .0,
            400
        );
    }
}
