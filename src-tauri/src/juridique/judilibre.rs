// ============================================================================
// Juridix — Module Juridique — judilibre.rs
// ============================================================================
// Fonctions de recherche JUDILIBRE (décisions de justice — Cour de cassation,
// Cours d'appel, Tribunaux judiciaires, Conseil d'État).
// API REST — GET avec query params.
//
// Corrections / ajouts par rapport à la version initiale :
//   - Paramètres manquants ajoutés au struct : location, with_file_of_type,
//     particular_interest (présents dans le swagger officiel).
//   - Pagination : l'API JUDILIBRE utilise `page` (index base 0) et
//     `page_size`. Les champs `batch`/`batch_size` sont conservés comme
//     alias pour la rétrocompatibilité avec routes.rs, mais la sérialisation
//     vers l'URL utilise les noms officiels (`page` / `page_size`).
//   - `sort` officiel : "score" | "scorepub" | "date" (défaut scorepub).
//     L'ancien champ `order` correspond à l'ordre du tri ("asc"/"desc").
//   - `export` : nouvelle fonction pour le téléchargement en lot.
//   - `get_decision` : ajout du paramètre `query` (highlighting contextuel).
// ============================================================================

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use super::client::PisteClient;

// ── Base URL ──────────────────────────────────────────────────────────────────

const JUDILIBRE_BASE: &str = "https://api.piste.gouv.fr/cassation/judilibre/v1.0";

// ── Types publics ─────────────────────────────────────────────────────────────

/// Paramètres de recherche JUDILIBRE — alignés sur le swagger officiel v1.0.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JudilibreSearchParams {
    /// Texte libre (obligatoire pour /search — une requête vide retourne 0 résultats).
    pub query: String,

    /// Opérateur logique entre les termes de `query`.
    /// Valeurs : "or" (défaut) | "and" | "exact"
    pub operator: Option<String>,

    /// Champs ciblés : "expose", "moyens", "motivations", "dispositif",
    /// "annexes", "sommaire", "titrage", etc.
    /// Valeurs disponibles via GET /taxonomy?id=field.
    /// Si absent, la recherche porte sur l'intégralité de la décision.
    pub field: Option<Vec<String>>,

    /// Nature de décision : "arret", "qpc", "ordonnance", "saisie", etc.
    /// Valeurs via GET /taxonomy?id=type.
    pub type_: Option<Vec<String>>,

    /// Matière (nomenclature Cour de cassation).
    /// Valeurs via GET /taxonomy?id=theme.
    pub theme: Option<Vec<String>>,

    /// Chambre.
    /// Valeurs via GET /taxonomy?id=chamber.
    pub chamber: Option<Vec<String>>,

    /// Formation.
    /// Valeurs via GET /taxonomy?id=formation.
    pub formation: Option<Vec<String>>,

    /// Juridiction : "cc" (Cour de cassation, défaut), "ca", "tj", "ce", etc.
    /// Valeurs via GET /taxonomy?id=jurisdiction.
    pub jurisdiction: Option<Vec<String>>,

    /// Code de siège de juridiction (ex : "tj33063").
    /// Valeurs via GET /taxonomy?id=location&context_value=ca|tj.
    pub location: Option<Vec<String>>,

    /// Niveau de publication : "b", "bi", "br", "l", "r", "c", etc.
    /// Valeurs via GET /taxonomy?id=publication.
    pub publication: Option<Vec<String>>,

    /// Type de solution : "annulation", "avis", "cassation", "decheance",
    /// "designation", "irrecevabilite", "nonlieu", "qpc", "rabat", etc.
    /// Valeurs via GET /taxonomy?id=solution.
    pub solution: Option<Vec<String>>,

    /// Date début au format ISO-8601 (ex: "2021-05-13").
    pub date_start: Option<String>,

    /// Date fin au format ISO-8601.
    pub date_end: Option<String>,

    /// Valeur de tri.
    /// Valeurs : "score" (pertinence), "scorepub" (pertinence + publication,
    /// défaut), "date".
    pub sort: Option<String>,

    /// Ordre de tri : "asc" | "desc" (défaut).
    pub order: Option<String>,

    /// Numéro de page (base 0). Correspond au paramètre `page` de l'API.
    /// Alias : `batch` (conservé pour rétrocompatibilité).
    pub batch: u32,

    /// Résultats par page (max 50, défaut 10). Correspond à `page_size`.
    /// Alias : `batch_size` (conservé pour rétrocompatibilité).
    pub batch_size: u32,

    /// Filtrer par type de document associé aux décisions.
    /// Valeurs : "prep_rapp", "prep_avis", "prep_oral", "comm_comm",
    /// "comm_note", "comm_nora", "comm_lett", "comm_trad".
    /// Valeurs via GET /taxonomy?id=filetype.
    pub with_file_of_type: Option<Vec<String>>,

    /// Si true, restreint aux décisions à intérêt particulier.
    pub particular_interest: bool,

    /// Résoudre les références (retourne labels plutôt que codes).
    pub resolve_references: bool,
}

impl JudilibreSearchParams {
    /// Construit la query string à annexer à l'URL /search.
    pub fn to_query_string(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        if !self.query.is_empty() {
            parts.push(format!("query={}", urlencoding::encode(&self.query)));
        }

        if let Some(op) = &self.operator {
            parts.push(format!("operator={}", urlencoding::encode(op)));
        }

        if let Some(fields) = &self.field {
            for f in fields {
                parts.push(format!("field={}", urlencoding::encode(f)));
            }
        }

        if let Some(types) = &self.type_ {
            for t in types {
                parts.push(format!("type={}", urlencoding::encode(t)));
            }
        }

        if let Some(themes) = &self.theme {
            for t in themes {
                parts.push(format!("theme={}", urlencoding::encode(t)));
            }
        }

        if let Some(chambers) = &self.chamber {
            for c in chambers {
                parts.push(format!("chamber={}", urlencoding::encode(c)));
            }
        }

        if let Some(formations) = &self.formation {
            for f in formations {
                parts.push(format!("formation={}", urlencoding::encode(f)));
            }
        }

        if let Some(jurisdictions) = &self.jurisdiction {
            for j in jurisdictions {
                parts.push(format!("jurisdiction={}", urlencoding::encode(j)));
            }
        }

        // Filtre par siège de juridiction (nouveau — absent de l'ancienne version)
        if let Some(locations) = &self.location {
            for l in locations {
                parts.push(format!("location={}", urlencoding::encode(l)));
            }
        }

        if let Some(pubs) = &self.publication {
            for p in pubs {
                parts.push(format!("publication={}", urlencoding::encode(p)));
            }
        }

        if let Some(solutions) = &self.solution {
            for s in solutions {
                parts.push(format!("solution={}", urlencoding::encode(s)));
            }
        }

        if let Some(d) = &self.date_start {
            parts.push(format!("date_start={}", urlencoding::encode(d)));
        }
        if let Some(d) = &self.date_end {
            parts.push(format!("date_end={}", urlencoding::encode(d)));
        }

        // Tri : paramètre "sort" dans l'API officielle
        let sort_val = self.sort.as_deref().unwrap_or("scorepub");
        parts.push(format!("sort={}", urlencoding::encode(sort_val)));

        // Ordre : paramètre "order" dans l'API officielle
        let order_val = self.order.as_deref().unwrap_or("desc");
        parts.push(format!("order={}", urlencoding::encode(order_val)));

        // Pagination : paramètres officiels "page" et "page_size"
        let page_size = self.batch_size.clamp(1, 50);
        parts.push(format!("page={}", self.batch));
        parts.push(format!("page_size={}", page_size));

        // Filtres supplémentaires (nouveaux)
        if let Some(file_types) = &self.with_file_of_type {
            for ft in file_types {
                parts.push(format!("withFileOfType={}", urlencoding::encode(ft)));
            }
        }

        if self.particular_interest {
            parts.push("particularInterest=true".to_string());
        }

        if self.resolve_references {
            parts.push("resolve_references=true".to_string());
        }

        parts.join("&")
    }
}

/// Réponse de recherche JUDILIBRE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudilibreSearchResponse {
    pub total: Option<i64>,
    pub results: Option<Vec<Value>>,
    /// Index de la page suivante (null si dernière page).
    pub next_batch: Option<i64>,
    /// Index de la page précédente (null si première page).
    pub previous_batch: Option<i64>,
    pub took: Option<i64>,
}

/// Paramètres d'export JUDILIBRE (téléchargement en lot).
#[derive(Debug, Clone, Default)]
pub struct JudilibreExportParams {
    pub type_: Option<Vec<String>>,
    pub theme: Option<Vec<String>>,
    pub chamber: Option<Vec<String>>,
    pub formation: Option<Vec<String>>,
    pub jurisdiction: Option<Vec<String>>,
    pub location: Option<Vec<String>>,
    pub publication: Option<Vec<String>>,
    pub solution: Option<Vec<String>>,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    /// "creation" | "modification" | "decision" (défaut "decision")
    pub date_type: Option<String>,
    /// Ordre : "asc" (défaut) | "desc"
    pub order: Option<String>,
    pub batch: u32,
    pub batch_size: u32,
    /// Si true, version abrégée des décisions (sans texte intégral).
    pub abridged: bool,
    pub with_file_of_type: Option<Vec<String>>,
    pub particular_interest: bool,
    pub resolve_references: bool,
}

// ── Fonctions publiques ───────────────────────────────────────────────────────

/// Recherche dans les décisions de justice.
pub async fn search(
    client: &PisteClient,
    params: JudilibreSearchParams,
) -> Result<JudilibreSearchResponse> {
    if params.query.trim().is_empty() {
        return Ok(JudilibreSearchResponse {
            total: Some(0),
            results: Some(vec![]),
            next_batch: None,
            previous_batch: None,
            took: None,
        });
    }

    let qs = params.to_query_string();
    let url = format!("{}/search?{}", JUDILIBRE_BASE, qs);
    debug!("[JUDILIBRE] search url={}", url);

    client.get_json::<JudilibreSearchResponse>(&url).await
}

/// Récupère une décision complète par son identifiant unique.
///
/// # Paramètres
/// - `id` : identifiant de la décision (ex : "644f5a3e1bce0b5c0e5e7a1f")
/// - `resolve_references` : si true, retourne les labels complets plutôt que
///   les codes courts.
/// - `query` : si fourni, active le highlighting des termes dans le texte de
///   la décision retournée (même sémantique que le paramètre `query` de /search).
/// - `operator` : opérateur pour le highlighting ("or" | "and" | "exact").
pub async fn get_decision(
    client: &PisteClient,
    id: &str,
    resolve_references: bool,
    query: Option<&str>,
    operator: Option<&str>,
) -> Result<Value> {
    let mut parts = vec![
        format!("id={}", urlencoding::encode(id)),
        format!("resolve_references={}", resolve_references),
    ];
    if let Some(q) = query {
        if !q.is_empty() {
            parts.push(format!("query={}", urlencoding::encode(q)));
            if let Some(op) = operator {
                parts.push(format!("operator={}", urlencoding::encode(op)));
            }
        }
    }
    let url = format!("{}/decision?{}", JUDILIBRE_BASE, parts.join("&"));
    debug!("[JUDILIBRE] decision id={}", id);
    client.get_json::<Value>(&url).await
}

/// Export par lots — pour récupérer de grandes quantités de décisions sans
/// critère de texte libre (contrairement à /search).
pub async fn export(client: &PisteClient, params: JudilibreExportParams) -> Result<Value> {
    let mut parts: Vec<String> = Vec::new();
    let batch_size = params.batch_size.clamp(1, 1000);
    parts.push(format!("page={}", params.batch));
    parts.push(format!("batch_size={}", batch_size));

    if let Some(types) = &params.type_ {
        for t in types {
            parts.push(format!("type={}", urlencoding::encode(t)));
        }
    }
    if let Some(themes) = &params.theme {
        for t in themes {
            parts.push(format!("theme={}", urlencoding::encode(t)));
        }
    }
    if let Some(chambers) = &params.chamber {
        for c in chambers {
            parts.push(format!("chamber={}", urlencoding::encode(c)));
        }
    }
    if let Some(formations) = &params.formation {
        for f in formations {
            parts.push(format!("formation={}", urlencoding::encode(f)));
        }
    }
    if let Some(jurisdictions) = &params.jurisdiction {
        for j in jurisdictions {
            parts.push(format!("jurisdiction={}", urlencoding::encode(j)));
        }
    }
    if let Some(locations) = &params.location {
        for l in locations {
            parts.push(format!("location={}", urlencoding::encode(l)));
        }
    }
    if let Some(pubs) = &params.publication {
        for p in pubs {
            parts.push(format!("publication={}", urlencoding::encode(p)));
        }
    }
    if let Some(solutions) = &params.solution {
        for s in solutions {
            parts.push(format!("solution={}", urlencoding::encode(s)));
        }
    }
    if let Some(d) = &params.date_start {
        parts.push(format!("date_start={}", urlencoding::encode(d)));
    }
    if let Some(d) = &params.date_end {
        parts.push(format!("date_end={}", urlencoding::encode(d)));
    }
    if let Some(dt) = &params.date_type {
        parts.push(format!("date_type={}", urlencoding::encode(dt)));
    }
    let order = params.order.as_deref().unwrap_or("asc");
    parts.push(format!("order={}", urlencoding::encode(order)));
    if params.abridged {
        parts.push("abridged=true".to_string());
    }
    if let Some(file_types) = &params.with_file_of_type {
        for ft in file_types {
            parts.push(format!("withFileOfType={}", urlencoding::encode(ft)));
        }
    }
    if params.particular_interest {
        parts.push("particularInterest=true".to_string());
    }
    if params.resolve_references {
        parts.push("resolve_references=true".to_string());
    }

    let url = format!("{}/export?{}", JUDILIBRE_BASE, parts.join("&"));
    debug!("[JUDILIBRE] export url={}", url);
    client.get_json::<Value>(&url).await
}

/// Export par lots (legacy scan) — conservé pour rétrocompatibilité.
/// Préférer `export()` pour les nouvelles intégrations.
#[allow(dead_code)]
pub async fn scan(
    client: &PisteClient,
    batch: u32,
    batch_size: u32,
    jurisdiction: Option<Vec<String>>,
    date_start: Option<String>,
    date_end: Option<String>,
) -> Result<Value> {
    let params = JudilibreExportParams {
        batch,
        batch_size,
        jurisdiction,
        date_start,
        date_end,
        ..Default::default()
    };
    export(client, params).await
}

/// Récupère les valeurs valides d'une taxonomie JUDILIBRE.
///
/// Valeurs de `id` : "field", "type", "theme", "chamber", "formation",
/// "jurisdiction", "publication", "solution", "filetype", "location".
///
/// Pour "location", un `context_value` est nécessaire pour filtrer par
/// type de juridiction : "ca" (cours d'appel) ou "tj" (tribunaux judiciaires).
pub async fn get_taxonomy(
    client: &PisteClient,
    id: &str,
    context_value: Option<&str>,
) -> Result<Value> {
    let mut url = format!(
        "{}/taxonomy?id={}",
        JUDILIBRE_BASE,
        urlencoding::encode(id)
    );
    if let Some(ctx) = context_value {
        url.push_str(&format!("&context_value={}", urlencoding::encode(ctx)));
    }
    debug!("[JUDILIBRE] taxonomy id={} context={:?}", id, context_value);
    client.get_json::<Value>(&url).await
}

/// Vérifie la disponibilité du service JUDILIBRE.
pub async fn healthcheck(client: &PisteClient) -> Result<Value> {
    let url = format!("{}/healthcheck", JUDILIBRE_BASE);
    client.get_json::<Value>(&url).await
}

/// Stats globales de la base JUDILIBRE.
pub async fn stats(client: &PisteClient) -> Result<Value> {
    let url = format!("{}/stats", JUDILIBRE_BASE);
    client.get_json::<Value>(&url).await
}

/// Historique transactionnel (créations/modifications de décisions).
/// Utile pour détecter les mises à jour depuis une date donnée.
#[allow(dead_code)]
pub async fn transactional_history(
    client: &PisteClient,
    date_start: Option<String>,
    date_end: Option<String>,
    batch: u32,
    batch_size: u32,
) -> Result<Value> {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("page={}", batch));
    parts.push(format!("batch_size={}", batch_size.clamp(1, 1000)));
    if let Some(d) = date_start {
        parts.push(format!("date_start={}", urlencoding::encode(&d)));
    }
    if let Some(d) = date_end {
        parts.push(format!("date_end={}", urlencoding::encode(&d)));
    }
    let url = format!(
        "{}/transactionalhistory?{}",
        JUDILIBRE_BASE,
        parts.join("&")
    );
    client.get_json::<Value>(&url).await
}
