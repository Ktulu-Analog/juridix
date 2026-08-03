// ============================================================================
// Juridix — Module Juridique — routes.rs
// ============================================================================

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use super::{
    client::PisteClient,
    judilibre::{self, JudilibreExportParams, JudilibreSearchParams},
    legifrance::{self, FiltreValeurs, SearchFond},
};

// ── Cache ─────────────────────────────────────────────────────────────────────

const TTL_SEARCH: Duration = Duration::from_secs(300);
const TTL_DOCUMENT: Duration = Duration::from_secs(1800);
const TTL_TAXONOMY: Duration = Duration::from_secs(3600);
const TTL_LIST: Duration = Duration::from_secs(900);

pub type Cache = Arc<RwLock<HashMap<String, (Value, Instant)>>>;

pub fn new_cache() -> Cache {
    Arc::new(RwLock::new(HashMap::new()))
}

async fn cache_get(cache: &Cache, key: &str, ttl: Duration) -> Option<Value> {
    let g = cache.read().await;
    if let Some((v, at)) = g.get(key) {
        if at.elapsed() < ttl {
            return Some(v.clone());
        }
    }
    None
}

async fn cache_set(cache: &Cache, key: String, val: Value) {
    let mut g = cache.write().await;
    g.insert(key, (val, Instant::now()));
    if g.len() > 500 {
        g.retain(|_, (_, at)| at.elapsed() < TTL_DOCUMENT);
    }
}

// ── JuridiqueState ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct JuridiqueState {
    pub piste_client: Arc<RwLock<Option<PisteClient>>>,
    pub cache: Cache,
}

impl JuridiqueState {
    pub fn new(client_id: Option<String>, client_secret: Option<String>) -> Self {
        let client = match (client_id, client_secret) {
            (Some(id), Some(secret)) if !id.trim().is_empty() && !secret.trim().is_empty() => {
                match PisteClient::new(id, secret) {
                    Ok(c) => {
                        info!("[Juridique] PisteClient initialisé");
                        Some(c)
                    }
                    Err(e) => {
                        error!("[Juridique] Init PisteClient: {}", e);
                        None
                    }
                }
            }
            _ => {
                info!("[Juridique] Credentials PISTE absents — module désactivé");
                None
            }
        };
        Self {
            piste_client: Arc::new(RwLock::new(client)),
            cache: new_cache(),
        }
    }

    pub fn reinit_client(&self, client_id: String, client_secret: String) {
        let arc = self.piste_client.clone();
        tokio::spawn(async move {
            match PisteClient::new(client_id, client_secret) {
                Ok(c) => {
                    *arc.write().await = Some(c);
                    info!("[Juridique] PisteClient rechargé");
                }
                Err(e) => {
                    error!("[Juridique] Rechargement PisteClient: {}", e);
                }
            }
        });
    }

    pub async fn client_async(&self) -> Result<PisteClient, Response> {
        let g = self.piste_client.read().await;
        g.clone().ok_or_else(|| {
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({
                "error": "Module juridique non configuré",
                "detail": "Configurez piste_client_id et piste_client_secret dans les paramètres."
            }))).into_response()
        })
    }
}

pub trait HasJuridiqueState: Send + Sync + 'static {
    fn juridique_state(&self) -> &JuridiqueState;
}

// ── Payloads ──────────────────────────────────────────────────────────────────

/// Corps de POST /rechercher — source = "legifrance" | "judilibre"
#[derive(Debug, Deserialize)]
pub struct RechercherRequest {
    pub source: String,
    // Légifrance
    pub fond: Option<String>,
    pub query: Option<String>,
    pub nature: Option<Vec<String>>,
    /// État juridique (LODA_ETAT / CODE_ETAT / KALI) — VIGUEUR, ABROGE, etc.
    pub legal_status: Option<Vec<String>>,
    /// Facette(s) additionnelle(s) génériques {facette, valeurs}, pour les
    /// filtres propres à un seul fond non couverts par un raccourci nommé
    /// (ex : CNIL TYPE / NATURE_DELIB). Ignoré côté JUDILIBRE.
    pub facette_extra: Option<Vec<FiltreValeurs>>,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    pub sort: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    /// Champ de recherche spécifique (ALL, TITLE, NOR, NUM, ARTICLE, etc.)
    pub champ_recherche: Option<String>,
    // JUDILIBRE
    pub operator: Option<String>,
    pub field: Option<Vec<String>>,
    pub jurisdiction: Option<Vec<String>>,
    pub location: Option<Vec<String>>,
    pub chamber: Option<Vec<String>>,
    pub formation: Option<Vec<String>>,
    pub publication: Option<Vec<String>>,
    pub solution: Option<Vec<String>>,
    pub theme: Option<Vec<String>>,
    pub batch: Option<u32>,
    pub batch_size: Option<u32>,
    pub resolve_references: Option<bool>,
    pub with_file_of_type: Option<Vec<String>>,
    pub particular_interest: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct DossierDoc {
    pub titre: String,
    pub source: String,
    /// Type du document (non utilisé côté backend actuellement, conservé pour
    /// correspondre au payload envoyé par le frontend).
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub type_: Option<String>,
    pub contenu: String,
}

#[derive(Debug, Deserialize)]
pub struct EnvoyerChatRequest {
    pub contenu: Option<String>,
    pub titre: Option<String>,
    pub mode: String,
    pub prompt_custom: Option<String>,
    /// Documents du dossier (mode dossier_*)
    pub dossier: Option<Vec<DossierDoc>>,
}

#[derive(Debug, Deserialize)]
pub struct SuggestRequest {
    pub query: String,
    pub supplies: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct LodaListRequest {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub natures: Option<Vec<String>>,
    pub legal_status: Option<Vec<String>>,
    pub sort: Option<String>,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodeListRequest {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub code_name: Option<String>,
    pub states: Option<Vec<String>>,
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExportJudilibreRequest {
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
    pub date_type: Option<String>,
    pub order: Option<String>,
    pub batch: Option<u32>,
    pub batch_size: Option<u32>,
    pub abridged: Option<bool>,
    pub with_file_of_type: Option<Vec<String>>,
    pub particular_interest: Option<bool>,
    pub resolve_references: Option<bool>,
}

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DecisionQuery {
    pub resolve_references: Option<bool>,
    pub query: Option<String>,
    pub operator: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaxonomieQuery {
    pub context_value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DateQuery {
    /// Date de vigueur souhaitée (défaut : aujourd'hui, format YYYY-MM-DD)
    pub date: Option<String>,
}

// ── Payloads — nouveaux endpoints ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BoccListRequest {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub interval_publication: Option<String>,
    pub idcc: Option<String>,
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BoccTextsRequest {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub interval_publication: Option<String>,
    pub idcc: Option<String>,
    pub titre: Option<String>,
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BoccsAndTextsRequest {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub id_global_bocc: Option<String>,
    pub interval_publication: Option<String>,
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BodmrListRequest {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub years: Option<Vec<i32>>,
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConventionsListRequest {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub idcc: Option<String>,
    pub titre: Option<String>,
    pub legal_status: Option<Vec<String>>,
    pub key_words: Option<Vec<String>>,
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DebatsParlementairesListRequest {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub date_parution: Option<String>,
    pub types_publication: Option<Vec<String>>,
    pub sort: Option<String>,
    pub second_sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DocsAdminsListRequest {
    pub years: Option<Vec<i32>>,
}

#[derive(Debug, Deserialize)]
pub struct DossiersLegislatifsListRequest {
    pub type_: String,
    pub legislature_id: i32,
}

#[derive(Debug, Deserialize)]
pub struct QuestionsEcritesListRequest {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub periode_publication: Option<String>,
    pub parlement_types: Option<Vec<String>>,
    pub sort: Option<String>,
    pub second_sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SameNumArticleRequest {
    pub text_cid: String,
    pub article_cid: String,
    pub date: String,
    pub article_num: String,
}

#[derive(Debug, Deserialize)]
pub struct JuriPlanClassementRequest {
    pub id: Option<String>,
    pub fond: Option<String>,
    pub libelle: Option<String>,
    pub niveau: Option<i32>,
    pub page: Option<i32>,
    pub search_suggest: Option<bool>,
    pub search_by_niveau: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ChronoTextRequest {
    pub text_cid: String,
    pub date_consult: String,
    pub start_year: i32,
    pub end_year: i32,
}

#[derive(Debug, Deserialize)]
pub struct ChronoArticleRequest {
    pub text_cid: String,
    pub element_cid: String,
}

#[derive(Debug, Deserialize)]
pub struct GetTablesRequest {
    pub end_year: i32,
    pub start_year: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct GetLastNJoRequest {
    pub nb_element: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct LawDecreeRequest {
    pub text_id: String,
    pub date: Option<String>,
    pub from_suggest: Option<bool>,
    pub searched_string: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LegiTableMatieresRequest {
    pub text_id: String,
    pub date: Option<String>,
    pub nature: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ServicePublicLinksRequest {
    pub article_cid: String,
    pub fond: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SuggestPdcRequest {
    pub search_text: String,
    pub fond: Option<String>,
    pub origin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetBoccPdfMetadataRequest {
    pub id: Option<String>,
    pub for_global_bocc: Option<bool>,
}

// ── Helper: réponse d'erreur ──────────────────────────────────────────────────

fn err_response(status: StatusCode, msg: String) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn gateway_err(label: &str, id: &str, e: anyhow::Error) -> Response {
    error!("[Juridique] {} {}: {}", label, id, e);
    err_response(StatusCode::BAD_GATEWAY, e.to_string())
}

// ── Handlers — Recherche ──────────────────────────────────────────────────────

pub async fn rechercher<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<RechercherRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    // Clé de cache exhaustive : dérivée du Debug de la requête entière plutôt
    // que d'une poignée de champs choisis à la main. Avant ce correctif, la
    // clé n'incluait que source/query/fond/page/date_start — deux recherches
    // identiques sur ces 5 champs mais avec des filtres différents (nature,
    // état juridique, tri, chambre, matière, juridiction, publication...)
    // recevaient alors à tort le même résultat en cache. Inclure tout `req`
    // via `{:?}` élimine cette classe de bug et reste correct automatiquement
    // si de nouveaux filtres sont ajoutés à `RechercherRequest` plus tard.
    let cache_key = format!("search:{:?}", req);
    if let Some(v) = cache_get(&js.cache, &cache_key, TTL_SEARCH).await {
        return Json(v).into_response();
    }

    let result: Result<Value, anyhow::Error> = match req.source.as_str() {
        "judilibre" => {
            let params = JudilibreSearchParams {
                query: req.query.unwrap_or_default(),
                operator: req.operator,
                field: req.field,
                type_: req.nature,
                theme: req.theme,
                chamber: req.chamber,
                formation: req.formation,
                jurisdiction: req.jurisdiction,
                location: req.location,
                publication: req.publication,
                solution: req.solution,
                date_start: req.date_start,
                date_end: req.date_end,
                sort: req.sort,
                order: Some("desc".to_string()),
                batch: req.batch.unwrap_or(0),
                batch_size: req.batch_size.unwrap_or(10).min(50),
                with_file_of_type: req.with_file_of_type,
                particular_interest: req.particular_interest.unwrap_or(false),
                resolve_references: req.resolve_references.unwrap_or(false),
            };
            judilibre::search(&client, params)
                .await
                .map(|r| serde_json::to_value(r).unwrap_or_default())
        }
        _ => {
            let fond = parse_fond(req.fond.as_deref().unwrap_or("LODA_ETAT"));
            let type_champ = req
                .champ_recherche
                .as_deref()
                .map(|c| c.to_uppercase())
                .filter(|c| !c.is_empty() && c != "ALL")
                .unwrap_or_else(|| "ALL".to_string());
            legifrance::search_simple(
                &client,
                req.query.as_deref().unwrap_or(""),
                fond,
                req.page.unwrap_or(1),
                req.page_size.unwrap_or(10).min(100),
                req.sort,
                req.nature,
                req.date_start,
                req.date_end,
                Some(type_champ),
                req.legal_status,
                req.facette_extra,
            )
            .await
            .map(|r| serde_json::to_value(r).unwrap_or_default())
        }
    };

    match result {
        Ok(v) => {
            cache_set(&js.cache, cache_key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("rechercher", "", e),
    }
}

// ── Handlers — Articles & sections ───────────────────────────────────────────

/// GET /article/:id
/// Tente getArticle puis getArticleByCid en fallback.
pub async fn get_article<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("article:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    let result = match legifrance::get_article(&client, &id).await {
        Ok(v) => Ok(v),
        Err(_) => legifrance::get_article_by_cid(&client, &id).await,
    };
    match result {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_article", &id, e),
    }
}

/// GET /article-version/:cid/:version_id
///
/// Récupère une version HISTORIQUE précise d'un article. `/consult/getArticle`
/// (utilisé par `get_article` ci-dessus) prend un id d'article mais ne
/// garantit pas de renvoyer le contenu correspondant à une version passée :
/// en pratique il semble toujours résoudre vers la version actuellement en
/// vigueur, quel que soit l'id de version historique fourni. Le seul
/// endpoint qui expose réellement chaque version avec son propre contenu est
/// `/consult/getArticleByCid` (« Récupère la LISTE des articles par leur
/// identifiant commun » — `GetListArticleResponse.listArticle`, un tableau
/// de versions complètes). On récupère donc la liste complète par le CID
/// (stable à travers les versions) puis on sélectionne la version demandée
/// par son id, et on la renvoie ré-emballée sous la forme
/// `GetArticleResponse` (`{"article": ...}`) pour que le frontend puisse la
/// traiter exactement comme une réponse `/article/:id` classique.
pub async fn get_article_version<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path((cid, version_id)): Path<(String, String)>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("article-version:{}:{}", cid, version_id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    let list = match legifrance::get_article_by_cid(&client, &cid).await {
        Ok(v) => v,
        Err(e) => return gateway_err("get_article_version", &cid, e),
    };
    let versions = list
        .get("listArticle")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let matched = versions
        .iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(version_id.as_str()))
        .cloned()
        // Repli : si l'id précis n'est pas retrouvé dans la liste (ex.
        // désynchronisation d'index côté DILA), on renvoie la première
        // version de la liste plutôt qu'une erreur sèche.
        .or_else(|| versions.first().cloned());

    match matched {
        Some(article) => {
            let wrapped = serde_json::json!({ "article": article });
            cache_set(&js.cache, key, wrapped.clone()).await;
            Json(wrapped).into_response()
        }
        None => gateway_err(
            "get_article_version",
            &cid,
            anyhow::anyhow!("Aucune version trouvée pour ce CID"),
        ),
    }
}

/// GET /section/:cid
pub async fn get_section<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(cid): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("section:{}", cid);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_section_by_cid(&client, &cid).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_section", &cid, e),
    }
}

// ── Handlers — Textes LODA ────────────────────────────────────────────────────

/// GET /loda/:cid?date=YYYY-MM-DD
pub async fn get_loda_text<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(cid): Path<String>,
    Query(q): Query<DateQuery>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("loda:{}:{}", cid, q.date.as_deref().unwrap_or(""));
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_loda_part(&client, &cid, q.date.as_deref()).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_loda", &cid, e),
    }
}

// ── Handlers — Codes ──────────────────────────────────────────────────────────

/// GET /code/:cid?date=YYYY-MM-DD
pub async fn get_code<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(cid): Path<String>,
    Query(q): Query<DateQuery>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("code:{}:{}", cid, q.date.as_deref().unwrap_or(""));
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_code_table_matieres(&client, &cid, q.date.as_deref()).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => {
            // `/consult/code/tableMatieres` (endpoint historique, déprécié côté
            // PISTE) échoue en pratique avec "L'expression à valider est
            // fausse" (HTTP 400) sur certains codes à la structure très
            // profondément imbriquée — le Code civil en particulier, alors
            // même que le payload envoyé est conforme au schéma
            // CodeConsultRequest (bug documenté côté DILA, indépendant de
            // notre requête). On retente alors automatiquement via
            // `/consult/legi/tableMatieres` (get_legi_table_matieres_v2),
            // l'alternative non dépréciée déjà implémentée pour ce cas —
            // jusqu'ici jamais appelée en repli automatique par cette route.
            warn!(
                "[Juridique] échec code/tableMatieres pour {cid}, repli sur legi/tableMatieres : {e}"
            );
            match legifrance::get_legi_table_matieres_v2(
                &client,
                &cid,
                q.date.as_deref(),
                Some("CODE"),
            )
            .await
            {
                Ok(v) => {
                    cache_set(&js.cache, key, v.clone()).await;
                    Json(v).into_response()
                }
                Err(e2) => {
                    // `anyhow::Error::context` ne fait apparaître, via `Display`
                    // (utilisé par `gateway_err`/`e.to_string()`), que le message
                    // le plus externe — le message réel de `e2` (le repli) était
                    // donc invisible côté utilisateur, masquant si le repli
                    // échoue pour la MÊME raison que l'appel initial ou pour une
                    // raison différente. On compose ici un message plat qui
                    // montre explicitement les deux échecs.
                    let e_str = e.to_string();
                    let e2_str = e2.to_string();
                    // Les deux endpoints (`code/tableMatieres` et `legi/tableMatieres`)
                    // renvoient la même erreur générique "L'expression à valider
                    // est fausse" sur les codes les plus volumineux/profondément
                    // imbriqués (Code civil, Code de la défense…) : ce n'est donc
                    // vraisemblablement pas un problème de choix d'endpoint, mais
                    // une limitation connue côté PISTE/DILA sur la taille de la
                    // réponse pour ces textes — aucun des deux endpoints exposés
                    // par l'API Légifrance ne permet de récupérer la structure
                    // d'un code par morceaux (sctCid est obligatoire pour
                    // /consult/code hors table des matières, donc inutilisable
                    // tant qu'on ne connaît pas déjà au moins les cid de premier
                    // niveau — impossible à obtenir sans que ce même appel réussisse).
                    let same_symptom = e_str.contains("expression à valider est fausse")
                        && e2_str.contains("expression à valider est fausse");
                    let msg = if same_symptom {
                        format!(
                            "Ce code semble trop volumineux pour être renvoyé en une fois par l'API Légifrance (les deux endpoints de table des matières échouent avec la même erreur \"{}\") — limitation connue côté PISTE/DILA sur les codes les plus imposants et profondément imbriqués (Code civil, Code de la défense…), indépendante de Juridix : aucun des deux endpoints ne permet de charger un code par sections plutôt qu'en une seule fois. En attendant, recherchez directement l'article ou la section qui vous intéresse (la recherche plein texte fonctionne normalement, seule l'ouverture du code entier échoue). Détail technique : code/tableMatieres → {e_str} ; legi/tableMatieres → {e2_str}",
                            "L'expression à valider est fausse"
                        )
                    } else {
                        format!("échec code/tableMatieres ({e_str}) ET échec du repli legi/tableMatieres ({e2_str})")
                    };
                    gateway_err("get_code", &cid, anyhow::anyhow!(msg))
                }
            }
        }
    }
}

// ── Handlers — JORF ───────────────────────────────────────────────────────────

/// GET /jorf-part/:cid
pub async fn get_jorf_part<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(cid): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("jorf-part:{}", cid);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_jorf_part(&client, &cid).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_jorf_part", &cid, e),
    }
}

/// GET /jorf-nor/:nor  — par numéro NOR
pub async fn get_jorf_nor<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(nor): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("jorf-nor:{}", nor);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_jorf_by_nor(&client, &nor).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_jorf_nor", &nor, e),
    }
}

// ── Handlers — Jurisprudence Légifrance (JURI / CETAT / CONSTIT / JUFI) ──────

/// GET /juri/:id
/// Consultation d'une décision JURI (JURITEXT…), CETAT, CONSTIT ou JUFI.
/// L'API /consult/juri fonctionne pour tous ces fonds.
pub async fn get_juri<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("juri:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_juri(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_juri", &id, e),
    }
}

// ── Handlers — KALI (conventions collectives) ─────────────────────────────────

/// GET /kali/:id  — texte de convention collective
pub async fn get_kali<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("kali:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    // KALITEXT → kaliText ; KALIARTI → kaliArticle ; KALI + idcc → kaliContIdcc
    let result = if id.starts_with("KALITEXT") {
        legifrance::get_kali_text(&client, &id).await
    } else if id.starts_with("KALIARTI") {
        legifrance::get_kali_article(&client, &id).await
    } else if id.chars().all(|c| c.is_ascii_digit()) {
        // IDCC numérique → kaliContIdcc
        legifrance::get_kali_cont_idcc(&client, &id).await
    } else {
        legifrance::get_kali_cont(&client, &id).await
    };

    match result {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_kali", &id, e),
    }
}

// ── Handlers — CNIL ───────────────────────────────────────────────────────────

/// GET /cnil/:id
pub async fn get_cnil<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("cnil:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_cnil(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_cnil", &id, e),
    }
}

// ── Handlers — Circulaires ────────────────────────────────────────────────────

/// GET /circulaire/:id
pub async fn get_circulaire<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("circ:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_circulaire(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_circulaire", &id, e),
    }
}

// ── Handlers — Accords d'entreprise (ACCO) ────────────────────────────────────

/// GET /acco/:id
pub async fn get_acco<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("acco:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_acco(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_acco", &id, e),
    }
}

// ── Handlers — Dossiers législatifs ──────────────────────────────────────────

/// GET /dossier/:id
pub async fn get_dossier<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!("dossier:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match legifrance::get_dossier_legislatif(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_dossier", &id, e),
    }
}

// ── Handlers — JUDILIBRE ─────────────────────────────────────────────────────

/// GET /decision/:id?resolve_references=true&query=…&operator=…
pub async fn get_decision<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
    Query(q): Query<DecisionQuery>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let resolve = q.resolve_references.unwrap_or(true);
    let key = format!(
        "decision:{}:{}:{}",
        id,
        resolve,
        q.query.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }

    match judilibre::get_decision(
        &client,
        &id,
        resolve,
        q.query.as_deref(),
        q.operator.as_deref(),
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_decision", &id, e),
    }
}

/// GET /taxonomie/:type?context_value=ca|tj
pub async fn get_taxonomie<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(type_id): Path<String>,
    Query(q): Query<TaxonomieQuery>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!(
        "taxonomie:{}:{}",
        type_id,
        q.context_value.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_TAXONOMY).await {
        return Json(v).into_response();
    }

    match judilibre::get_taxonomy(&client, &type_id, q.context_value.as_deref()).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_taxonomie", &type_id, e),
    }
}

/// GET /judilibre-stats
pub async fn judilibre_stats<S: HasJuridiqueState>(State(state): State<Arc<S>>) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = "judilibre:stats".to_string();
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }

    match judilibre::stats(&client).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("judilibre_stats", "", e),
    }
}

/// GET /judilibre-healthcheck
pub async fn judilibre_healthcheck<S: HasJuridiqueState>(State(state): State<Arc<S>>) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match judilibre::healthcheck(&client).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => gateway_err("judilibre_healthcheck", "", e),
    }
}

// ── Handlers — Listes ─────────────────────────────────────────────────────────

/// POST /loda-list
pub async fn loda_list<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<LodaListRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!(
        "loda-list:{}:{}:{}",
        req.page.unwrap_or(1),
        req.natures
            .as_ref()
            .map(|n| n.join(","))
            .unwrap_or_default(),
        req.date_start.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }

    match legifrance::list_loda(
        &client,
        req.page.unwrap_or(1),
        req.page_size.unwrap_or(20).min(100),
        req.natures,
        req.legal_status,
        req.sort,
        req.date_start,
        req.date_end,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("loda_list", "", e),
    }
}

/// POST /code-list
pub async fn code_list<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<CodeListRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let key = format!(
        "code-list:{}:{}",
        req.page.unwrap_or(1),
        req.code_name.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }

    match legifrance::list_codes(
        &client,
        req.page.unwrap_or(1),
        req.page_size.unwrap_or(50).min(100),
        req.code_name,
        req.states,
        req.sort,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("code_list", "", e),
    }
}

// ── Handlers — Suggest ────────────────────────────────────────────────────────

/// POST /suggest
pub async fn suggest<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<SuggestRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    if req.query.trim().is_empty() {
        return Json(json!({ "results": [] })).into_response();
    }

    let key = format!(
        "suggest:{}:{}",
        req.query,
        req.supplies
            .as_ref()
            .map(|s| s.join(","))
            .unwrap_or_default()
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_SEARCH).await {
        return Json(v).into_response();
    }

    match legifrance::suggest(&client, &req.query, req.supplies).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("suggest", &req.query, e),
    }
}

// ── Handlers — Export JUDILIBRE ───────────────────────────────────────────────

/// POST /export-judilibre
pub async fn export_judilibre<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<ExportJudilibreRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let params = JudilibreExportParams {
        type_: req.type_,
        theme: req.theme,
        chamber: req.chamber,
        formation: req.formation,
        jurisdiction: req.jurisdiction,
        location: req.location,
        publication: req.publication,
        solution: req.solution,
        date_start: req.date_start,
        date_end: req.date_end,
        date_type: req.date_type,
        order: req.order,
        batch: req.batch.unwrap_or(0),
        batch_size: req.batch_size.unwrap_or(100).min(1000),
        abridged: req.abridged.unwrap_or(false),
        with_file_of_type: req.with_file_of_type,
        particular_interest: req.particular_interest.unwrap_or(false),
        resolve_references: req.resolve_references.unwrap_or(false),
    };
    match judilibre::export(&client, params).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => gateway_err("export_judilibre", "", e),
    }
}

// ── Handlers — Chrono ─────────────────────────────────────────────────────────

/// POST /chrono-text
pub async fn chrono_text<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<ChronoTextRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("chrono-text:{}:{}", req.text_cid, req.date_consult);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::chrono_text(
        &client,
        &req.text_cid,
        &req.date_consult,
        req.start_year,
        req.end_year,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("chrono_text", &req.text_cid, e),
    }
}

/// POST /chrono-article
pub async fn chrono_article<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<ChronoArticleRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("chrono-article:{}:{}", req.text_cid, req.element_cid);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::chrono_article(&client, &req.text_cid, &req.element_cid).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("chrono_article", &req.text_cid, e),
    }
}

// ── Handlers — Liens articles ─────────────────────────────────────────────────

/// GET /concordance/:id
pub async fn get_concordance<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("concordance:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_concordance_links(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_concordance", &id, e),
    }
}

/// GET /related-links/:id
pub async fn get_related_links<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("related-links:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_related_links(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_related_links", &id, e),
    }
}

/// POST /same-num-article
pub async fn same_num_article<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<SameNumArticleRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match legifrance::get_same_num_article(
        &client,
        &req.text_cid,
        &req.article_cid,
        &req.date,
        &req.article_num,
    )
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => gateway_err("same_num_article", &req.text_cid, e),
    }
}

/// POST /service-public-links
pub async fn service_public_links<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<ServicePublicLinksRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("sp-links:{}", req.article_cid);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_service_public_links(&client, &req.article_cid, req.fond.as_deref()).await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("service_public_links", &req.article_cid, e),
    }
}

// ── Handlers — ELI ────────────────────────────────────────────────────────────

/// GET /jorf-eli/:eli   (URL-encoded)
pub async fn get_jorf_eli<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(eli): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("jorf-eli:{}", eli);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_jorf_by_eli(&client, &eli).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_jorf_eli", &eli, e),
    }
}

/// GET /article-eli/:eli
pub async fn get_article_eli<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(eli): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("article-eli:{}", eli);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_article_by_eli(&client, &eli).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_article_eli", &eli, e),
    }
}

// ── Handlers — Anciens identifiants ──────────────────────────────────────────

/// GET /juri-ancien/:id
pub async fn get_juri_ancien<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("juri-ancien:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_juri_by_ancien_id(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_juri_ancien", &id, e),
    }
}

/// GET /code-ancien/:id
pub async fn get_code_ancien<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("code-ancien:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_code_by_ancien_id(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_code_ancien", &id, e),
    }
}

/// GET /cnil-ancien/:id
pub async fn get_cnil_ancien<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("cnil-ancien:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_cnil_by_ancien_id(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_cnil_ancien", &id, e),
    }
}

// ── Handlers — JORF ───────────────────────────────────────────────────────────

/// GET /jorf/:cid   (texte complet via /consult/jorf)
pub async fn get_jorf_full<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(cid): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("jorf-full:{}", cid);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_jorf(&client, &cid, None).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_jorf_full", &cid, e),
    }
}

// ── Handlers — Débats parlementaires ─────────────────────────────────────────

/// GET /debat/:id
pub async fn get_debat<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("debat:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_debat(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_debat", &id, e),
    }
}

/// POST /debats-list
pub async fn debats_list<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<DebatsParlementairesListRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "debats-list:{}:{}",
        req.page.unwrap_or(1),
        req.date_parution.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::list_debats_parlementaires(
        &client,
        req.page.unwrap_or(1),
        req.page_size.unwrap_or(20).min(100),
        req.date_parution,
        req.types_publication,
        req.sort,
        req.second_sort,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("debats_list", "", e),
    }
}

// ── Handlers — KALI section ───────────────────────────────────────────────────

/// GET /kali-section/:id
pub async fn get_kali_section<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Path(id): Path<String>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("kali-section:{}", id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_kali_section(&client, &id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_kali_section", &id, e),
    }
}

// ── Handlers — Jurisprudence plan de classement ────────────────────────────────

/// POST /juri-plan-classement
pub async fn juri_plan_classement<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<JuriPlanClassementRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "juri-plan:{}:{}",
        req.id.as_deref().unwrap_or(""),
        req.libelle.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::get_juri_plan_classement(
        &client,
        req.id.as_deref(),
        req.fond.as_deref(),
        req.libelle.as_deref(),
        req.niveau,
        req.page,
        req.search_suggest.unwrap_or(false),
        req.search_by_niveau.unwrap_or(false),
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("juri_plan_classement", "", e),
    }
}

// ── Handlers — Derniers JO ────────────────────────────────────────────────────

/// GET /last-jo?nb_element=5
pub async fn get_last_jo<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Query(q): Query<GetLastNJoRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let n = q.nb_element.unwrap_or(5).min(20);
    let key = format!("last-jo:{}", n);
    if let Some(v) = cache_get(&js.cache, &key, Duration::from_secs(3600)).await {
        return Json(v).into_response();
    }
    match legifrance::get_last_n_jo(&client, n).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_last_jo", "", e),
    }
}

// ── Handlers — Tables annuelles ───────────────────────────────────────────────

/// POST /tables
pub async fn get_tables_handler<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<GetTablesRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("tables:{}:{}", req.end_year, req.start_year.unwrap_or(0));
    if let Some(v) = cache_get(&js.cache, &key, TTL_TAXONOMY).await {
        return Json(v).into_response();
    }
    match legifrance::get_tables(&client, req.end_year, req.start_year).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_tables", "", e),
    }
}

// ── Handlers — lawDecree ──────────────────────────────────────────────────────

/// POST /law-decree
pub async fn law_decree<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<LawDecreeRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "law-decree:{}:{}",
        req.text_id,
        req.date.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_law_decree(
        &client,
        &req.text_id,
        req.date.as_deref(),
        req.from_suggest.unwrap_or(false),
        req.searched_string.as_deref(),
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("law_decree", &req.text_id, e),
    }
}

// ── Handlers — legi/tableMatieres v2 ─────────────────────────────────────────

/// POST /legi-table-matieres
pub async fn legi_table_matieres<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<LegiTableMatieresRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "legi-tm:{}:{}",
        req.text_id,
        req.date.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_DOCUMENT).await {
        return Json(v).into_response();
    }
    match legifrance::get_legi_table_matieres_v2(
        &client,
        &req.text_id,
        req.date.as_deref(),
        req.nature.as_deref(),
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("legi_table_matieres", &req.text_id, e),
    }
}

// ── Handlers — BOCC ──────────────────────────────────────────────────────────

/// POST /bocc-list
pub async fn bocc_list<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<BoccListRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "bocc-list:{}:{}",
        req.page.unwrap_or(1),
        req.interval_publication.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::list_bocc(
        &client,
        req.page.unwrap_or(1),
        req.page_size.unwrap_or(20).min(100),
        req.interval_publication,
        req.idcc,
        req.sort,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("bocc_list", "", e),
    }
}

/// POST /bocc-texts-list
pub async fn bocc_texts_list<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<BoccTextsRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "bocc-texts:{}:{}",
        req.page.unwrap_or(1),
        req.idcc.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::list_bocc_texts(
        &client,
        req.page.unwrap_or(1),
        req.page_size.unwrap_or(20).min(100),
        req.interval_publication,
        req.idcc,
        req.titre,
        req.sort,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("bocc_texts_list", "", e),
    }
}

/// POST /boccs-and-texts
pub async fn boccs_and_texts<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<BoccsAndTextsRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "boccs-texts:{}:{}",
        req.page.unwrap_or(1),
        req.id_global_bocc.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::list_boccs_and_texts(
        &client,
        req.page.unwrap_or(1),
        req.page_size.unwrap_or(20).min(100),
        req.id_global_bocc,
        req.interval_publication,
        req.sort,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("boccs_and_texts", "", e),
    }
}

/// GET /bocc-pdf-metadata
pub async fn bocc_pdf_metadata<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<GetBoccPdfMetadataRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match legifrance::get_bocc_pdf_metadata(
        &client,
        req.id.as_deref(),
        req.for_global_bocc.unwrap_or(false),
    )
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => gateway_err("bocc_pdf_metadata", "", e),
    }
}

// ── Handlers — BODMR ─────────────────────────────────────────────────────────

/// POST /bodmr-list
pub async fn bodmr_list<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<BodmrListRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("bodmr-list:{}", req.page.unwrap_or(1));
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::list_bodmr(
        &client,
        req.page.unwrap_or(1),
        req.page_size.unwrap_or(20).min(100),
        req.years,
        req.sort,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("bodmr_list", "", e),
    }
}

// ── Handlers — Conventions collectives ────────────────────────────────────────

/// POST /conventions-list
pub async fn conventions_list<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<ConventionsListRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "conventions:{}:{}",
        req.page.unwrap_or(1),
        req.idcc.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::list_conventions(
        &client,
        req.page.unwrap_or(1),
        req.page_size.unwrap_or(20).min(100),
        req.idcc,
        req.titre,
        req.legal_status,
        req.key_words,
        req.sort,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("conventions_list", "", e),
    }
}

// ── Handlers — Docs administratifs ────────────────────────────────────────────

/// POST /docs-admins
pub async fn docs_admins<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<DocsAdminsListRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "docs-admins:{}",
        req.years
            .as_ref()
            .map(|y| y
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(","))
            .unwrap_or_default()
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::list_docs_admins(&client, req.years).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("docs_admins", "", e),
    }
}

// ── Handlers — Dossiers législatifs ───────────────────────────────────────────

/// POST /dossiers-legislatifs-list
pub async fn dossiers_legislatifs_list<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<DossiersLegislatifsListRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!("dossiers-list:{}:{}", req.type_, req.legislature_id);
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::list_dossiers_legislatifs(&client, &req.type_, req.legislature_id).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("dossiers_legislatifs_list", "", e),
    }
}

/// GET /legislatures
pub async fn get_legislatures<S: HasJuridiqueState>(State(state): State<Arc<S>>) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = "legislatures".to_string();
    if let Some(v) = cache_get(&js.cache, &key, TTL_TAXONOMY).await {
        return Json(v).into_response();
    }
    match legifrance::list_legislatures(&client).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("get_legislatures", "", e),
    }
}

// ── Handlers — Questions écrites parlementaires ────────────────────────────────

/// POST /questions-ecrites-list
pub async fn questions_ecrites_list<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<QuestionsEcritesListRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = format!(
        "questions-ecrites:{}:{}",
        req.page.unwrap_or(1),
        req.periode_publication.as_deref().unwrap_or("")
    );
    if let Some(v) = cache_get(&js.cache, &key, TTL_LIST).await {
        return Json(v).into_response();
    }
    match legifrance::list_questions_ecrites(
        &client,
        req.page.unwrap_or(1),
        req.page_size.unwrap_or(20).min(100),
        req.periode_publication,
        req.parlement_types,
        req.sort,
        req.second_sort,
    )
    .await
    {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("questions_ecrites_list", "", e),
    }
}

// ── Handlers — Suggest spécialisés ────────────────────────────────────────────

/// POST /suggest-acco
pub async fn suggest_acco<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<SuggestRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    if req.query.trim().is_empty() {
        return Json(json!({ "results": [] })).into_response();
    }
    match legifrance::suggest_acco(&client, &req.query).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => gateway_err("suggest_acco", &req.query, e),
    }
}

/// POST /suggest-pdc
pub async fn suggest_pdc<S: HasJuridiqueState>(
    State(state): State<Arc<S>>,
    Json(req): Json<SuggestPdcRequest>,
) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    if req.search_text.trim().is_empty() {
        return Json(json!({ "results": [] })).into_response();
    }
    match legifrance::suggest_pdc(
        &client,
        &req.search_text,
        req.fond.as_deref(),
        req.origin.as_deref(),
    )
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => gateway_err("suggest_pdc", &req.search_text, e),
    }
}

// ── Handlers — Misc ───────────────────────────────────────────────────────────

/// GET /dates-without-jo
pub async fn dates_without_jo<S: HasJuridiqueState>(State(state): State<Arc<S>>) -> Response {
    let js = state.juridique_state();
    let client = match js.client_async().await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let key = "dates-without-jo".to_string();
    if let Some(v) = cache_get(&js.cache, &key, TTL_TAXONOMY).await {
        return Json(v).into_response();
    }
    match legifrance::get_dates_without_jo(&client).await {
        Ok(v) => {
            cache_set(&js.cache, key, v.clone()).await;
            Json(v).into_response()
        }
        Err(e) => gateway_err("dates_without_jo", "", e),
    }
}

// ── Handler — Pont LLM ────────────────────────────────────────────────────────

/// POST /envoyer-chat
/// Formate le document courant (ou le dossier) pour injection dans le chat IA.
pub async fn envoyer_chat(Json(req): Json<EnvoyerChatRequest>) -> Response {
    let titre = req.titre.as_deref().unwrap_or("document juridique");
    let contenu = req.contenu.as_deref().unwrap_or("");

    let message = match req.mode.as_str() {
        // ── Mode document unique ──────────────────────────────────────────────
        "resumer" => {
            let p = req.prompt_custom.as_deref().unwrap_or(
                "Résume ce document juridique de façon structurée : \
                 (1) nature et objet du texte, (2) champ d'application, \
                 (3) dispositions principales, (4) date d'entrée en vigueur. \
                 Restitution factuelle uniquement, sans interprétation.",
            );
            format!("{}\n\n---\n**{}**\n\n{}", p, titre, contenu)
        }
        "analyser" => {
            let p = req.prompt_custom.as_deref().unwrap_or(
                "Analysez ce document juridique. Précisez : qualification juridique, \
                 implications pratiques, risques et points d'attention, recommandations.",
            );
            format!(
                "⚖️ **Analyse juridique demandée**\n\n{}\n\n---\n**{}**\n\n{}",
                p, titre, contenu
            )
        }

        // ── Mode dossier — synthèse globale ───────────────────────────────────
        "dossier_synthese" => {
            let docs = req.dossier.as_deref().unwrap_or(&[]);
            if docs.is_empty() {
                return Json(json!({ "error": "Dossier vide" })).into_response();
            }
            let corpus: String = docs
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    format!(
                        "--- Document {} : {} ({}) ---\n\n{}",
                        i + 1,
                        d.titre,
                        d.source,
                        d.contenu
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            format!(
                "📋 **Synthèse de dossier juridique — {} document(s)**\n\n\
                 Produisez une synthèse structurée et unifiée de l'ensemble des documents \
                 ci-dessous. Organisation suggérée : \
                 (1) Contexte juridique commun, \
                 (2) Principales dispositions ou principes dégagés, \
                 (3) Points de cohérence entre les textes, \
                 (4) Conclusion opérationnelle.\n\n\
                 Restitution factuelle uniquement, sans interprétation.\n\n---\n\n{}",
                docs.len(),
                corpus
            )
        }

        // ── Mode dossier — analyse contradictoire ─────────────────────────────
        "dossier_contradictoire" => {
            let docs = req.dossier.as_deref().unwrap_or(&[]);
            if docs.is_empty() {
                return Json(json!({ "error": "Dossier vide" })).into_response();
            }
            let corpus: String = docs
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    format!(
                        "--- Document {} : {} ({}) ---\n\n{}",
                        i + 1,
                        d.titre,
                        d.source,
                        d.contenu
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            format!(
                "⚔️ **Analyse contradictoire — {} document(s)**\n\n\
                 Identifiez et analysez les convergences et divergences entre les documents \
                 suivants. Structure attendue :\n\
                 **A. Points de convergence** — dispositions ou principes concordants\n\
                 **B. Points de tension ou de divergence** — contradictions, évolutions, \
                 interprétations concurrentes\n\
                 **C. Hiérarchie des normes** — comment articuler ces textes entre eux ?\n\
                 **D. Incidences pratiques** — quelles conséquences pour le praticien ?\n\n---\n\n{}",
                docs.len(),
                corpus
            )
        }

        // ── Mode dossier — points clés par doc + synthèse ────────────────────
        "dossier_points_cles" => {
            let docs = req.dossier.as_deref().unwrap_or(&[]);
            if docs.is_empty() {
                return Json(json!({ "error": "Dossier vide" })).into_response();
            }
            let corpus: String = docs
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    format!(
                        "--- Document {} : {} ({}) ---\n\n{}",
                        i + 1,
                        d.titre,
                        d.source,
                        d.contenu
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            format!(
                "🎯 **Points clés par document + synthèse — {} document(s)**\n\n\
                 Pour chacun des documents ci-dessous, extrayez en 3 à 5 points clés \
                 les éléments essentiels (objet, portée, règle applicable, date, exceptions). \
                 Puis concluez par une **Synthèse transversale** en 4 à 6 lignes.\n\n---\n\n{}",
                docs.len(),
                corpus
            )
        }

        _ => format!("---\n**{}**\n\n{}", titre, contenu),
    };
    Json(json!({ "message": message, "mode": req.mode })).into_response()
}

// ── Handler — Statut ──────────────────────────────────────────────────────────

/// GET /status
pub async fn status<S: HasJuridiqueState>(State(state): State<Arc<S>>) -> Response {
    let js = state.juridique_state();
    let g = js.piste_client.read().await;
    match &*g {
        None => Json(json!({ "configured": false, "message": "Credentials PISTE absents" }))
            .into_response(),
        Some(client) => match client.bearer_token().await {
            Ok(_) => Json(json!({ "configured": true, "piste": "ok" })).into_response(),
            Err(e) => {
                Json(json!({ "configured": true, "piste": "error", "message": e.to_string() }))
                    .into_response()
            }
        },
    }
}

// ── Routeur ───────────────────────────────────────────────────────────────────

pub fn juridique_router<S>() -> Router<Arc<S>>
where
    S: HasJuridiqueState,
{
    Router::new()
        // ── Recherche unifiée
        .route("/rechercher", post(rechercher::<S>))
        // ── Légifrance — articles & sections
        .route("/article/:id", get(get_article::<S>))
        .route(
            "/article-version/:cid/:version_id",
            get(get_article_version::<S>),
        )
        .route("/section/:cid", get(get_section::<S>))
        .route("/concordance/:id", get(get_concordance::<S>))
        .route("/related-links/:id", get(get_related_links::<S>))
        .route("/same-num-article", post(same_num_article::<S>))
        .route("/service-public-links", post(service_public_links::<S>))
        // ── Légifrance — ELI / alias
        .route("/jorf-eli/*eli", get(get_jorf_eli::<S>))
        .route("/article-eli/*eli", get(get_article_eli::<S>))
        // ── Légifrance — anciens identifiants
        .route("/juri-ancien/:id", get(get_juri_ancien::<S>))
        .route("/code-ancien/:id", get(get_code_ancien::<S>))
        .route("/cnil-ancien/:id", get(get_cnil_ancien::<S>))
        // ── Légifrance — textes LODA (lois, décrets, arrêtés)
        .route("/loda/:cid", get(get_loda_text::<S>))
        .route("/law-decree", post(law_decree::<S>))
        // ── Légifrance — codes
        .route("/code/:cid", get(get_code::<S>))
        .route("/legi-table-matieres", post(legi_table_matieres::<S>))
        // ── Légifrance — JORF
        .route("/jorf/:cid", get(get_jorf_full::<S>))
        .route("/jorf-part/:cid", get(get_jorf_part::<S>))
        .route("/jorf-nor/:nor", get(get_jorf_nor::<S>))
        .route("/last-jo", get(get_last_jo::<S>))
        // ── Légifrance — jurisprudence (JURI / CETAT / CONSTIT / JUFI)
        .route("/juri/:id", get(get_juri::<S>))
        .route("/juri-plan-classement", post(juri_plan_classement::<S>))
        // ── Légifrance — KALI (conventions collectives)
        .route("/kali/:id", get(get_kali::<S>))
        .route("/kali-section/:id", get(get_kali_section::<S>))
        // ── Légifrance — CNIL
        .route("/cnil/:id", get(get_cnil::<S>))
        // ── Légifrance — circulaires
        .route("/circulaire/:id", get(get_circulaire::<S>))
        // ── Légifrance — accords d'entreprise
        .route("/acco/:id", get(get_acco::<S>))
        // ── Légifrance — dossiers législatifs
        .route("/dossier/:id", get(get_dossier::<S>))
        .route(
            "/dossiers-legislatifs-list",
            post(dossiers_legislatifs_list::<S>),
        )
        .route("/legislatures", get(get_legislatures::<S>))
        // ── Légifrance — débats parlementaires
        .route("/debat/:id", get(get_debat::<S>))
        .route("/debats-list", post(debats_list::<S>))
        // ── Légifrance — questions écrites parlementaires
        .route("/questions-ecrites-list", post(questions_ecrites_list::<S>))
        // ── Légifrance — BOCC
        .route("/bocc-list", post(bocc_list::<S>))
        .route("/bocc-texts-list", post(bocc_texts_list::<S>))
        .route("/boccs-and-texts", post(boccs_and_texts::<S>))
        .route("/bocc-pdf-metadata", post(bocc_pdf_metadata::<S>))
        // ── Légifrance — BODMR
        .route("/bodmr-list", post(bodmr_list::<S>))
        // ── Légifrance — conventions collectives (liste)
        .route("/conventions-list", post(conventions_list::<S>))
        // ── Légifrance — documents administratifs
        .route("/docs-admins", post(docs_admins::<S>))
        // ── Légifrance — tables & chrono
        .route("/tables", post(get_tables_handler::<S>))
        .route("/chrono-text", post(chrono_text::<S>))
        .route("/chrono-article", post(chrono_article::<S>))
        // ── Légifrance — listes
        .route("/loda-list", post(loda_list::<S>))
        .route("/code-list", post(code_list::<S>))
        // ── Légifrance — autocomplétion
        .route("/suggest", post(suggest::<S>))
        .route("/suggest-acco", post(suggest_acco::<S>))
        .route("/suggest-pdc", post(suggest_pdc::<S>))
        // ── Légifrance — misc
        .route("/dates-without-jo", get(dates_without_jo::<S>))
        // ── JUDILIBRE — décisions
        .route("/decision/:id", get(get_decision::<S>))
        // ── JUDILIBRE — taxonomies
        .route("/taxonomie/:type", get(get_taxonomie::<S>))
        // ── JUDILIBRE — export par lots
        .route("/export-judilibre", post(export_judilibre::<S>))
        // ── JUDILIBRE — utilitaires
        .route("/judilibre-stats", get(judilibre_stats::<S>))
        .route("/judilibre-healthcheck", get(judilibre_healthcheck::<S>))
        // ── Pont LLM
        .route("/envoyer-chat", post(envoyer_chat))
        // ── Statut
        .route("/status", get(status::<S>))
}

// ── Helper ────────────────────────────────────────────────────────────────────

pub fn parse_fond(s: &str) -> SearchFond {
    match s.to_uppercase().as_str() {
        "JORF" => SearchFond::Jorf,
        "CNIL" => SearchFond::Cnil,
        "CETAT" => SearchFond::Cetat,
        "JURI" => SearchFond::Juri,
        "JUFI" => SearchFond::Jufi,
        "CONSTIT" => SearchFond::Constit,
        "KALI" => SearchFond::Kali,
        "CODE_DATE" => SearchFond::CodeDate,
        "CODE_ETAT" => SearchFond::CodeEtat,
        "LODA_DATE" => SearchFond::LodaDate,
        "LODA_ETAT" => SearchFond::LodaEtat,
        "CIRC" => SearchFond::Circ,
        "ACCO" => SearchFond::Acco,
        "BOCC" => SearchFond::Bocc,
        "DEBAT" => SearchFond::DebatParl,
        "QUESTION_ECRITE" => SearchFond::QuestionEcrite,
        _ => SearchFond::All,
    }
}
