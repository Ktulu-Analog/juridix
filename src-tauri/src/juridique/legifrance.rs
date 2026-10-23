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
// Fonctions de recherche Légifrance (API PISTE).
// ============================================================================

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::debug;

use super::client::PisteClient;

// ── Base URL ──────────────────────────────────────────────────────────────────
// La base de l'API est portée par `PisteClient::legifrance_base()` (constante
// `client::LEGIFRANCE_BASE` en production ; surchargeable en test pour viser un
// mock local — voir `verify.rs`).

// ── Types publics ─────────────────────────────────────────────────────────────

/// Fonds disponibles pour la recherche Légifrance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SearchFond {
    Jorf,
    Cnil,
    Cetat,
    Juri,
    Jufi,
    Constit,
    Kali,
    CodeDate,
    CodeEtat,
    LodaDate,
    LodaEtat,
    All,
    Circ,
    Acco,
    Bocc,
    DebatParl,
    QuestionEcrite,
}

impl SearchFond {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jorf => "JORF",
            Self::Cnil => "CNIL",
            Self::Cetat => "CETAT",
            Self::Juri => "JURI",
            Self::Jufi => "JUFI",
            Self::Constit => "CONSTIT",
            Self::Kali => "KALI",
            Self::CodeDate => "CODE_DATE",
            Self::CodeEtat => "CODE_ETAT",
            Self::LodaDate => "LODA_DATE",
            Self::LodaEtat => "LODA_ETAT",
            Self::All => "ALL",
            Self::Circ => "CIRC",
            Self::Acco => "ACCO",
            // Ces fonds n'ont pas d'endpoint /search — redirigés vers ALL
            Self::Bocc => "ALL",
            Self::DebatParl => "ALL",
            Self::QuestionEcrite => "ALL",
        }
    }

    /// Tris valides par fond — source : doc officielle Excel PISTE
    /// (feuille "description-des-tris-et-filtres-de-l-api").
    pub fn valid_sorts(&self) -> &'static [&'static str] {
        match self {
            Self::Jorf => &[
                "PERTINENCE",
                "SIGNATURE_DATE_DESC",
                "SIGNATURE_DATE_ASC",
                "PUBLICATION_DATE_DESC",
                "PUBLICATION_DATE_ASC",
            ],
            Self::LodaDate | Self::LodaEtat => &[
                "PERTINENCE",
                "SIGNATURE_DATE_DESC",
                "SIGNATURE_DATE_ASC",
                "PUBLICATION_DATE_DESC",
                "PUBLICATION_DATE_ASC",
            ],
            Self::CodeDate | Self::CodeEtat => &[
                "PERTINENCE",
                "PUBLICATION_DATE_DESC",
                "PUBLICATION_DATE_ASC",
            ],
            Self::Juri | Self::Jufi | Self::Cetat | Self::Constit => {
                &["PERTINENCE", "DATE_DESC", "DATE_ASC"]
            }
            Self::Circ => &[
                "PERTINENCE",
                "SIGNATURE_DATE_DESC",
                "SIGNATURE_DATE_ASC",
                "PUBLI_DATE_DESC",
                "PUBLI_DATE_ASC",
            ],
            Self::Kali => &[
                "PERTINENCE",
                "MODIFICATION_DATE_DESC",
                "SIGNATURE_DATE_DESC",
                "SIGNATURE_DATE_ASC",
            ],
            Self::Acco => &["PERTINENCE", "DATE_DESC", "DATE_ASC"],
            Self::Cnil => &["PERTINENCE", "DATE_DECISION_DESC", "DATE_DECISION_ASC"],
            Self::Bocc => &["BOCC_SORT_DESC", "BOCC_SORT_ASC"],
            Self::DebatParl => &[
                "DEBAT_PARLEMENTAIRE_DESC",
                "DEBAT_PARLEMENTAIRE_ASC",
                "ID_ASC",
                "ID_DESC",
            ],
            Self::QuestionEcrite => &[
                "QUESTION_ECRITE_PARLEMENTAIRE_DESC",
                "QUESTION_ECRITE_PARLEMENTAIRE_ASC",
                "ID_ASC",
                "ID_DESC",
            ],
            _ => &["PERTINENCE"],
        }
    }

    /// Facette de date correcte selon le fond, si ce fond expose bien un
    /// filtre de date au sens de la doc officielle Excel PISTE (une feuille
    /// par fond). Retourne `None` quand aucun filtre de date n'existe pour
    /// ce fond — c'est le cas de CODE_ETAT (seul CODE_DATE a une colonne
    /// "Date de la version" renseignée dans la feuille CODE ; la variante
    /// ETAT ne propose que les filtres d'état juridique).
    ///
    /// # Anti-500
    /// Envoyer une facette de date inexistante pour un fond fait planter
    /// l'API (HTTP 500). `search_simple` doit donc ignorer silencieusement
    /// le filtre de dates si cette fonction retourne `None`.
    pub fn date_facette(&self) -> Option<&'static str> {
        match self {
            Self::CodeEtat => None,
            Self::CodeDate => Some("DATE_VERSION"),
            Self::LodaEtat => Some("TEXT_DATE_SIGNATURE"),
            Self::Constit | Self::Cetat | Self::Juri | Self::Jufi => Some("DATE_DECISION"),
            Self::Cnil => Some("DATE_DELIB"),
            Self::Bocc => Some("INTERVAL_PUBLICATION"),
            Self::DebatParl => Some("DATE_PUBLICATION"),
            Self::QuestionEcrite => Some("DATE_PUBLICATION"),
            // JORF, LODA_DATE, CIRC, KALI, ACCO
            _ => Some("DATE_SIGNATURE"),
        }
    }

    /// Facette de "nature" (type de texte) correcte selon le fond, si ce
    /// fond expose bien un filtre de nature simple et unique au sens de la
    /// doc officielle Excel PISTE. Retourne `None` sinon.
    ///
    /// D'après la doc officielle, seuls JORF, LODA_DATE et LODA_ETAT
    /// exposent une facette "nature" à proprement parler (valeurs LOI,
    /// DECRET, ORDONNANCE...). CODE (les codes n'ont pas de "nature" —
    /// seulement un nom et un état), CETAT, JURI, JUFI, KALI et ACCO
    /// n'ont PAS de facette NATURE générique dans leur feuille — y
    /// envoyer une facette "NATURE" provoque une erreur côté API.
    /// CONSTIT a plusieurs facettes de nature très spécifiques et non
    /// unifiables (contrôle de constitutionnalité / contentieux électoral
    /// / autres décisions) : on ne tente pas de les généraliser ici.
    /// CNIL (facettes TYPE et NATURE_DELIB), DebatParl
    /// (TYPE_DE_PUBLICATION) et QuestionEcrite (TYPE_PARLEMENT) sont
    /// volontairement laissés à `None` ici aussi : DebatParl/QuestionEcrite
    /// ne passent de toute façon plus par cette fonction (routés vers leurs
    /// endpoints `/list/*` dédiés, cf. frontend). CNIL reste accessible via
    /// le champ générique `RechercherRequest.facette_extra` (paire
    /// {facette, valeurs} libre) plutôt qu'un raccourci nommé ici, faute de
    /// valeurs d'énumération officielles fiables pour TYPE/NATURE_DELIB.
    ///
    /// # Anti-500
    /// Envoyer une facette "NATURE" inexistante pour un fond fait planter
    /// l'API (HTTP 500). `search_simple` doit donc ignorer silencieusement
    /// le filtre de nature si cette fonction retourne `None`.
    pub fn nature_facette(&self) -> Option<&'static str> {
        match self {
            Self::Jorf => Some("NATURE"),
            Self::LodaDate => Some("NATURE"),
            Self::LodaEtat => Some("TEXT_NATURE"),
            _ => None,
        }
    }

    /// Facette d'état juridique (VIGUEUR, ABROGE...) correcte selon le
    /// fond, si ce fond l'expose. D'après la doc officielle, seuls
    /// LODA_ETAT et CODE_ETAT (facette TEXT_LEGAL_STATUS) ainsi que KALI
    /// (facette LEGAL_STATUS) exposent ce filtre au niveau texte via
    /// `/search`. Les variantes _DATE n'ont pas ce filtre (elles filtrent
    /// par date de version à la place). Retourne `None` sinon.
    ///
    /// Valeurs possibles (enum officiel, communes à LODA/CODE) : VIGUEUR,
    /// ABROGE_DIFF, VIGUEUR_DIFF, VIGUEUR_ETEN, VIGUEUR_NON_ETEN, ABROGE,
    /// PERIME, ANNULE, MODIFIE, DISJOINT, SUBSTITUE, TRANSFERE, INITIALE,
    /// MODIFIE_MORT_NE, SANS_ETAT, DENONCE, REMPLACE.
    pub fn legal_status_facette(&self) -> Option<&'static str> {
        match self {
            Self::LodaEtat | Self::CodeEtat => Some("TEXT_LEGAL_STATUS"),
            Self::Kali => Some("LEGAL_STATUS"),
            _ => None,
        }
    }

    /// Vérifie si le fond est de type CODE (DATE ou ETAT).
    pub fn is_code(&self) -> bool {
        matches!(self, Self::CodeDate | Self::CodeEtat)
    }
}

impl std::fmt::Display for SearchFond {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Opérateur booléen entre critères ou champs.
/// Valeurs acceptées : ET | OU
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Operateur {
    Et,
    Ou,
}

impl Operateur {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Et => "ET",
            Self::Ou => "OU",
        }
    }
}

/// Type de recherche dans un critère.
/// Valeurs acceptées par l'API (enum officiel Swagger) :
///   UN_DES_MOTS | EXACTE | TOUS_LES_MOTS_DANS_UN_CHAMP |
///   AUCUN_DES_MOTS | AUCUNE_CORRESPONDANCE_A_CETTE_EXPRESSION
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TypeRecherche {
    UnDesMots,
    Exacte,
    TousLesMotsDansUnChamp,
    AucunDesMots,
    AucuneCorrespondanceACetteExpression,
}

impl TypeRecherche {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnDesMots => "UN_DES_MOTS",
            Self::Exacte => "EXACTE",
            Self::TousLesMotsDansUnChamp => "TOUS_LES_MOTS_DANS_UN_CHAMP",
            Self::AucunDesMots => "AUCUN_DES_MOTS",
            Self::AucuneCorrespondanceACetteExpression => {
                "AUCUNE_CORRESPONDANCE_A_CETTE_EXPRESSION"
            }
        }
    }
}

/// Paramètres d'un critère de recherche textuelle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Critere {
    pub valeur: String,
    pub type_recherche: TypeRecherche,
    pub operateur: Operateur,
    /// Proximité en nombre de mots (uniquement si TypeRecherche permet la
    /// proximité — cf. documentation PISTE).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proximite: Option<u32>,
}

/// Champ de recherche avec ses critères.
/// `type_champ` : "ALL" | "TITLE" | "TABLE" | "NOR" | "NUM" |
///   "ADVANCED_TEXTE_ID" | "NUM_DELIB" | "NUM_DEC" | "NUM_ARTICLE" |
///   "ARTICLE" | "MINISTERE" | "VISA" | "NOTICE" | "VISA_NOTICE" |
///   "TRAVAUX_PREP" | "SIGNATURE" | "NOTA" | "NUM_AFFAIRE" | "ABSTRATS" |
///   "RESUMES" | "TEXTE" | "ECLI" | "NUM_LOI_DEF" | "TYPE_DECISION" |
///   "NUMERO_INTERNE" | "REF_PUBLI" | "RESUME_CIRC" | "TEXTE_REF" |
///   "TITRE_LOI_DEF" | "RAISON_SOCIALE" | "MOTS_CLES" | "IDCC"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChampRecherche {
    pub type_champ: String,
    pub criteres: Vec<Critere>,
    pub operateur: Operateur,
}

/// Filtre par valeur(s) (liste de strings).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiltreValeurs {
    pub facette: String,
    pub valeurs: Vec<String>,
}

/// Filtre par intervalle de dates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiltreDates {
    pub facette: String,
    /// Format : "YYYY-MM-DD"
    pub start: Option<String>,
    /// Format : "YYYY-MM-DD"
    pub end: Option<String>,
}

/// Paramètres complets d'une recherche Légifrance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    pub fond: SearchFond,
    pub champs: Vec<ChampRecherche>,
    pub operateur: Operateur,
    /// Filtres par valeurs (nature, état juridique, etc.)
    #[serde(default)]
    pub filtres_valeurs: Vec<FiltreValeurs>,
    /// Filtres par plages de dates
    #[serde(default)]
    pub filtres_dates: Vec<FiltreDates>,
    /// Tri — sera validé selon le fond
    pub sort: Option<String>,
    /// Second tri (optionnel, selon documentation PISTE)
    pub second_sort: Option<String>,
    pub page_number: u32,
    pub page_size: u32,
}

impl SearchParams {
    /// Valide et retourne un body JSON prêt à envoyer à l'API.
    pub fn to_api_body(&self) -> Result<Value> {
        // ── Anti-500 : pagination ─────────────────────────────────────────────
        let page_number = self.page_number.max(1);
        let page_size = self.page_size.clamp(1, 100);

        // ── Tri ───────────────────────────────────────────────────────────────
        let sort = self.sort.as_deref().unwrap_or("PERTINENCE").to_uppercase();

        let valid_sorts = self.fond.valid_sorts();
        let sort = if valid_sorts.contains(&sort.as_str()) {
            sort
        } else {
            "PERTINENCE".to_string()
        };

        // ── Champs ────────────────────────────────────────────────────────────
        if self.champs.is_empty() {
            return Err(anyhow!("Au moins un champ de recherche est requis"));
        }

        let champs_json: Vec<Value> = self
            .champs
            .iter()
            .map(|c| {
                let criteres: Vec<Value> = c
                    .criteres
                    .iter()
                    .map(|cr| {
                        let mut obj = json!({
                            "valeur": cr.valeur,
                            "typeRecherche": cr.type_recherche.as_str(),
                            "operateur": cr.operateur.as_str(),
                        });
                        if let Some(p) = cr.proximite {
                            obj["proximite"] = json!(p);
                        }
                        obj
                    })
                    .collect();
                json!({
                    "typeChamp": c.type_champ,
                    "criteres": criteres,
                    "operateur": c.operateur.as_str(),
                })
            })
            .collect();

        // ── Filtres ───────────────────────────────────────────────────────────
        let mut filtres: Vec<Value> = Vec::new();

        for fv in &self.filtres_valeurs {
            // Règle anti-NOM_CODE : interdit sur CODE_DATE/CODE_ETAT en tant
            // que facette "NOM_CODE" — la bonne facette est "TEXT_NOM_CODE"
            // pour CODE_ETAT. On corrige silencieusement.
            let facette = if fv.facette == "NOM_CODE" && self.fond.is_code() {
                "TEXT_NOM_CODE".to_string()
            } else {
                fv.facette.clone()
            };

            // Règle : filtres_valeurs TOUJOURS en tableau
            filtres.push(json!({
                "facette": facette,
                "valeurs": fv.valeurs,
            }));
        }

        for fd in &self.filtres_dates {
            let mut dates = json!({});
            if let Some(s) = &fd.start {
                dates["start"] = json!(s);
            }
            if let Some(e) = &fd.end {
                dates["end"] = json!(e);
            }
            filtres.push(json!({
                "facette": fd.facette,
                "dates": dates,
            }));
        }

        // ── Construction du body ──────────────────────────────────────────────
        let mut recherche = json!({
            "champs": champs_json,
            "filtres": filtres,
            "operateur": self.operateur.as_str(),
            "pageNumber": page_number,
            "pageSize": page_size,
            "sort": sort,
            "typePagination": "DEFAUT",
            "fromAdvancedRecherche": false,
        });

        if let Some(ss) = &self.second_sort {
            let ss_upper = ss.to_uppercase();
            if valid_sorts.contains(&ss_upper.as_str()) && ss_upper != sort {
                recherche["secondSort"] = json!(ss_upper);
            }
        }

        let body = json!({
            "fond": self.fond.as_str(),
            "recherche": recherche,
        });

        debug!(
            "[Légifrance] search body={}",
            serde_json::to_string(&body).unwrap_or_default()
        );
        Ok(body)
    }
}

// ── Réponses typées ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    #[serde(rename = "totalResultNumber")]
    pub total: Option<i64>,
    pub results: Option<Vec<Value>>,
    pub facets: Option<Vec<Value>>,
    #[serde(rename = "executionTime")]
    pub execution_time: Option<i64>,
}

// ── Recherche ─────────────────────────────────────────────────────────────────

/// Recherche générique dans l'API Légifrance.
pub async fn search(client: &PisteClient, params: SearchParams) -> Result<SearchResponse> {
    let url = format!("{}/search", client.legifrance_base());
    let body = params.to_api_body()?;
    client.post_json::<Value, SearchResponse>(&url, &body).await
}

/// Recherche rapide (helper) : recherche textuelle dans un fond, 1 champ ALL.
/// Idéal pour les recherches simples depuis l'interface.
///
/// ## Règles anti-500 appliquées
/// - `fond = ALL` : aucun filtre (nature, état juridique ni date) ne doit
///   être envoyé. L'API plante côté serveur (HTTP 500) dans ce cas.
/// - Facettes de date, de nature et d'état juridique varient par fond —
///   centralisées dans `SearchFond::date_facette()`,
///   `SearchFond::nature_facette()` et `SearchFond::legal_status_facette()`.
///   Ces trois méthodes retournent `Option<&str>` : si le fond courant
///   n'expose pas le filtre correspondant, on l'omet silencieusement au
///   lieu d'envoyer une facette invalide (qui provoquerait un 500).
#[allow(clippy::too_many_arguments)]
pub async fn search_simple(
    client: &PisteClient,
    query: &str,
    fond: SearchFond,
    page: u32,
    page_size: u32,
    sort: Option<String>,
    nature: Option<Vec<String>>,
    date_start: Option<String>,
    date_end: Option<String>,
    type_champ: Option<String>,
    legal_status: Option<Vec<String>>,
    extra_filtres: Option<Vec<FiltreValeurs>>,
) -> Result<SearchResponse> {
    let is_all = matches!(fond, SearchFond::All);

    let mut filtres_valeurs = Vec::new();
    if !is_all {
        if let Some(natures) = nature.filter(|v| !v.is_empty()) {
            if let Some(facette) = fond.nature_facette() {
                filtres_valeurs.push(FiltreValeurs {
                    facette: facette.to_string(),
                    valeurs: natures,
                });
            }
        }
        if let Some(statuses) = legal_status.filter(|v| !v.is_empty()) {
            if let Some(facette) = fond.legal_status_facette() {
                filtres_valeurs.push(FiltreValeurs {
                    facette: facette.to_string(),
                    valeurs: statuses,
                });
            }
        }
        // Mécanisme générique : facettes propres à un seul fond et dont les
        // valeurs ne sont pas normalisées dans la doc officielle (ex : CNIL
        // TYPE / NATURE_DELIB — cf. feuille CNIL du classeur PISTE). Le
        // frontend envoie directement {facette, valeurs}, saisi librement
        // par l'utilisateur plutôt que via une liste figée potentiellement
        // fausse. Filtré des paires vides par sécurité.
        if let Some(extras) = extra_filtres {
            for f in extras {
                if !f.facette.trim().is_empty() && !f.valeurs.is_empty() {
                    filtres_valeurs.push(f);
                }
            }
        }
    }

    let mut filtres_dates = Vec::new();
    if !is_all && (date_start.is_some() || date_end.is_some()) {
        if let Some(facette) = fond.date_facette() {
            filtres_dates.push(FiltreDates {
                facette: facette.to_string(),
                start: date_start,
                end: date_end,
            });
        }
    }

    let champ = type_champ
        .as_deref()
        .filter(|c| !c.is_empty())
        .unwrap_or("ALL")
        .to_string();

    let params = SearchParams {
        fond,
        champs: vec![ChampRecherche {
            type_champ: champ,
            criteres: vec![Critere {
                valeur: query.to_string(),
                type_recherche: TypeRecherche::Exacte,
                operateur: Operateur::Et,
                proximite: None,
            }],
            operateur: Operateur::Et,
        }],
        operateur: Operateur::Et,
        filtres_valeurs,
        filtres_dates,
        sort,
        second_sort: None,
        page_number: page,
        page_size,
    };

    search(client, params).await
}

// ── Consultation d'articles ───────────────────────────────────────────────────

/// Récupère un article par son identifiant LEGIARTI (ou JORFARTI, etc.).
/// Body attendu : { "id": "LEGIARTI000..." }
pub async fn get_article(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/getArticle", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un article par son CID (Chronical Identifier).
/// Body attendu : { "cid": "LEGIARTI000..." }
pub async fn get_article_by_cid(client: &PisteClient, cid: &str) -> Result<Value> {
    let url = format!("{}/consult/getArticleByCid", client.legifrance_base());
    let body = json!({ "cid": cid });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un article par son id et son numéro.
/// Body attendu : { "id": "...", "num": "..." }
#[allow(dead_code)]
pub async fn get_article_with_id_and_num(
    client: &PisteClient,
    id: &str,
    num: &str,
) -> Result<Value> {
    let url = format!(
        "{}/consult/getArticleWithIdAndNum",
        client.legifrance_base()
    );
    let body = json!({ "id": id, "num": num });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère une section par son CID.
/// Body attendu : { "cid": "LEGISCTA000..." }
pub async fn get_section_by_cid(client: &PisteClient, cid: &str) -> Result<Value> {
    let url = format!("{}/consult/getSectionByCid", client.legifrance_base());
    let body = json!({ "cid": cid });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consultation de textes ────────────────────────────────────────────────────

/// Récupère un texte LODA (loi, décret, arrêté) par son CID et une date de
/// vigueur.
///
/// # CORRECTION par rapport à la version initiale
/// L'API POST /consult/legiPart attend un `LegiConsultRequest` :
///   { "textId": "LEGITEXT...", "date": "YYYY-MM-DD" }
/// L'ancienne version envoyait { "id": "..." } ce qui provoquait une erreur
/// 400/404 côté PISTE.
///
/// `date` : date de vigueur souhaitée. Si None, utilise la date du jour.
pub async fn get_loda_part(
    client: &PisteClient,
    text_cid: &str,
    date: Option<&str>,
) -> Result<Value> {
    let url = format!("{}/consult/legiPart", client.legifrance_base());
    let date_str = date
        .map(|d| d.to_string())
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let body = json!({
        "textId": text_cid,
        "date": date_str,
    });
    debug!(
        "[Légifrance] legiPart textId={} date={}",
        text_cid, date_str
    );
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un texte LODA avec le texte recherché (pour le highlighting).
/// Surcharge de `get_loda_part` avec `searchedString`.
#[allow(dead_code)]
pub async fn get_loda_part_with_search(
    client: &PisteClient,
    text_cid: &str,
    date: Option<&str>,
    searched_string: Option<&str>,
) -> Result<Value> {
    let url = format!("{}/consult/legiPart", client.legifrance_base());
    let date_str = date
        .map(|d| d.to_string())
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let mut body = json!({
        "textId": text_cid,
        "date": date_str,
    });
    if let Some(s) = searched_string {
        body["searchedString"] = json!(s);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère une décision de justice (fond JURI) par son id JURITEXT.
///
/// # CORRECTION
/// L'API POST /consult/juri attend { "textId": "JURITEXT..." }
/// (JuriConsultRequest), pas { "id": "..." }.
pub async fn get_juri(client: &PisteClient, text_id: &str) -> Result<Value> {
    let url = format!("{}/consult/juri", client.legifrance_base());
    let body = json!({ "textId": text_id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un texte du JORF complet par son NOR.
/// Body attendu : { "nor": "..." }
pub async fn get_jorf_by_nor(client: &PisteClient, nor: &str) -> Result<Value> {
    let url = format!("{}/consult/getJoWithNor", client.legifrance_base());
    let body = json!({ "nor": nor });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un conteneur JORF (partie d'un numéro du JO).
/// Body attendu : { "id": "..." } — correct dans la version initiale.
#[allow(dead_code)]
pub async fn get_jorf_cont(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/jorfCont", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère une partie de JORF par son CID (JorfPartRequest).
/// Body attendu : { "textCid": "JORFTEXT..." }
pub async fn get_jorf_part(client: &PisteClient, text_cid: &str) -> Result<Value> {
    let url = format!("{}/consult/jorfPart", client.legifrance_base());
    let body = json!({ "textCid": text_cid });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Codes ─────────────────────────────────────────────────────────────────────

/// Récupère la table des matières d'un code (structure arborescente des
/// sections et articles).
///
/// `cid` : identifiant du code (ex: LEGITEXT000006069414 pour le Code civil).
/// `date` : date de vigueur souhaitée (défaut : aujourd'hui).
pub async fn get_code_table_matieres(
    client: &PisteClient,
    cid: &str,
    date: Option<&str>,
) -> Result<Value> {
    let url = format!("{}/consult/code/tableMatieres", client.legifrance_base());
    let date_str = date
        .map(|d| d.to_string())
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let body = json!({
        "textId": cid,
        "date": date_str,
    });
    debug!(
        "[Légifrance] code/tableMatieres cid={} date={}",
        cid, date_str
    );
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère le contenu complet d'un code (avec sections et articles).
/// Similaire à `get_code_table_matieres` mais retourne le corps du texte.
/// `sct_cid` : optionnel, CID d'une section spécifique à consulter.
#[allow(dead_code)]
pub async fn get_code_consult(
    client: &PisteClient,
    text_id: &str,
    date: Option<&str>,
    sct_cid: Option<&str>,
) -> Result<Value> {
    let url = format!("{}/consult/code", client.legifrance_base());
    let date_str = date
        .map(|d| d.to_string())
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let mut body = json!({
        "textId": text_id,
        "date": date_str,
    });
    if let Some(sct) = sct_cid {
        body["sctCid"] = json!(sct);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère la table des matières globale Légifrance (liste des codes).
#[allow(dead_code)]
pub async fn get_legi_table_matieres(client: &PisteClient) -> Result<Value> {
    let url = format!("{}/consult/legi/tableMatieres", client.legifrance_base());
    let body = json!({});
    client.post_json::<Value, Value>(&url, &body).await
}

// ── KALI (conventions collectives) ────────────────────────────────────────────

/// Récupère un texte de convention collective KALI par son id.
/// Body attendu : { "id": "KALITEXT..." } (KaliTextConsultRequest).
pub async fn get_kali_text(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/kaliText", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un article de convention collective KALI par son id.
/// Body attendu : { "id": "KALIARTI..." } (KaliTextConsultArticleRequest).
pub async fn get_kali_article(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/kaliArticle", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un conteneur KALI par son id.
pub async fn get_kali_cont(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/kaliCont", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un conteneur KALI par son IDCC.
pub async fn get_kali_cont_idcc(client: &PisteClient, idcc: &str) -> Result<Value> {
    let url = format!("{}/consult/kaliContIdcc", client.legifrance_base());
    let body = json!({ "idcc": idcc });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── CNIL ──────────────────────────────────────────────────────────────────────

/// Récupère un texte CNIL par son id.
///
/// # CORRECTION
/// L'API attend { "textId": "..." } (CnilConsultRequest),
/// pas { "id": "..." }.
pub async fn get_cnil(client: &PisteClient, text_id: &str) -> Result<Value> {
    let url = format!("{}/consult/cnil", client.legifrance_base());
    let body = json!({ "textId": text_id });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Circulaires ───────────────────────────────────────────────────────────────

/// Récupère une circulaire par son id.
/// Body attendu : { "id": "..." } — correct.
pub async fn get_circulaire(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/circulaire", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Accords d'entreprise (ACCO) ───────────────────────────────────────────────

/// Récupère un accord d'entreprise par son id.
/// Body attendu : { "id": "..." } (AccoConsultRequest).
pub async fn get_acco(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/acco", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Dossiers législatifs ──────────────────────────────────────────────────────

/// Récupère un dossier législatif par son id.
/// Body attendu : { "id": "..." } (DossierLegislatifRequest).
pub async fn get_dossier_legislatif(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/dossierLegislatif", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Listes ────────────────────────────────────────────────────────────────────

/// Liste les textes LODA (lois, décrets, arrêtés) avec filtres optionnels.
///
/// `page_number` : numéro de page (min 1).
/// `page_size` : résultats par page (max 100).
/// `natures` : ex ["LOI", "ORDONNANCE", "DECRET"].
/// `legal_status` : ex ["VIGUEUR", "ABROGE"].
/// `sort` : "SIGNATURE_DATE_DESC" | "SIGNATURE_DATE_ASC" |
///          "PUBLICATION_DATE_DESC" | "PUBLICATION_DATE_ASC" | "PERTINENCE".
#[allow(clippy::too_many_arguments)]
pub async fn list_loda(
    client: &PisteClient,
    page_number: u32,
    page_size: u32,
    natures: Option<Vec<String>>,
    legal_status: Option<Vec<String>>,
    sort: Option<String>,
    signature_date_start: Option<String>,
    signature_date_end: Option<String>,
) -> Result<Value> {
    let url = format!("{}/list/loda", client.legifrance_base());
    let page_number = page_number.max(1);
    let page_size = page_size.clamp(1, 100);

    let mut body = json!({
        "pageNumber": page_number,
        "pageSize": page_size,
    });

    if let Some(n) = natures {
        body["natures"] = json!(n);
    }
    if let Some(ls) = legal_status {
        body["legalStatus"] = json!(ls);
    }
    if let Some(s) = sort {
        body["sort"] = json!(s);
    }
    if signature_date_start.is_some() || signature_date_end.is_some() {
        let mut sig_date = json!({});
        if let Some(s) = signature_date_start {
            sig_date["start"] = json!(s);
        }
        if let Some(e) = signature_date_end {
            sig_date["end"] = json!(e);
        }
        body["signatureDate"] = sig_date;
    }

    debug!(
        "[Légifrance] list/loda page={} size={}",
        page_number, page_size
    );
    client.post_json::<Value, Value>(&url, &body).await
}

/// Liste les codes disponibles.
///
/// `code_name` : filtre optionnel sur le nom du code.
/// `states` : états juridiques (ex ["VIGUEUR"]).
pub async fn list_codes(
    client: &PisteClient,
    page_number: u32,
    page_size: u32,
    code_name: Option<String>,
    states: Option<Vec<String>>,
    sort: Option<String>,
) -> Result<Value> {
    let url = format!("{}/list/code", client.legifrance_base());
    let page_number = page_number.max(1);
    let page_size = page_size.clamp(1, 100);

    let mut body = json!({
        "pageNumber": page_number,
        "pageSize": page_size,
    });

    if let Some(cn) = code_name {
        body["codeName"] = json!(cn);
    }
    if let Some(s) = states {
        body["states"] = json!(s);
    }
    if let Some(s) = sort {
        body["sort"] = json!(s);
    }

    client.post_json::<Value, Value>(&url, &body).await
}

// ── Suggestion ────────────────────────────────────────────────────────────────

/// Suggestions de résultats (autocomplétion).
///
/// # CORRECTION — comportement entièrement corrigé
/// L'ancienne implémentation faisait un GET /suggest?query=...&fond=...
/// ce qui est **incorrect** — l'endpoint est POST /suggest et attend un
/// body JSON `SuggestSupplyRequest` :
///   { "searchText": "...", "supplies": ["JORF", "LODA_LIST", ...] }
///
/// `supplies` : liste des fonds à interroger.
/// Valeurs possibles : "ALL" | "ALL_SUGGEST" | "LODA_LIST" | "CODE_LIST" |
///   "CODE_RELEASE_DATE" | "CODE_RELEASE_DATE_SUGGEST" |
///   "CODE_LEGAL_STATUS" | "LODA_RELEASE_DATE" |
///   "LODA_RELEASE_DATE_SUGGEST" | "LODA_LEGAL_STATUS" |
///   "KALI" | "KALI_TEXT" | "CONSTIT" | "CETAT" | "JUFI" |
///   "JURI" | "JORF" | "JORF_SUGGEST" | "CNIL" | "ARTICLE" |
///   "CIRC" | "ACCO" | "PDF"
///
/// Si `supplies` est vide ou None, on envoie ["ALL_SUGGEST"] par défaut.
pub async fn suggest(
    client: &PisteClient,
    query: &str,
    supplies: Option<Vec<String>>,
) -> Result<Value> {
    let url = format!("{}/suggest", client.legifrance_base());
    let supplies_val = supplies.unwrap_or_else(|| vec!["ALL_SUGGEST".to_string()]);
    let body = json!({
        "searchText": query,
        "supplies": supplies_val,
    });
    debug!(
        "[Légifrance] suggest query={:?} supplies={:?}",
        query, &body["supplies"]
    );
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Ping ─────────────────────────────────────────────────────────────────────

/// Vérifie la disponibilité du service Légifrance (endpoint /consult/ping).
#[allow(dead_code)]
pub async fn ping(client: &PisteClient) -> Result<Value> {
    let url = format!("{}/consult/ping", client.legifrance_base());
    client.get_json::<Value>(&url).await
}

// ── Chrono ────────────────────────────────────────────────────────────────────

/// Récupère la chronologie d'un texte LEGI (versions par date).
/// Body : { "textCid": "...", "dateConsult": "YYYY-MM-DD", "startYear": N, "endYear": N }
pub async fn chrono_text(
    client: &PisteClient,
    text_cid: &str,
    date_consult: &str,
    start_year: i32,
    end_year: i32,
) -> Result<Value> {
    let url = format!("{}/chrono/textCid", client.legifrance_base());
    let body = json!({
        "textCid": text_cid,
        "dateConsult": date_consult,
        "startYear": start_year,
        "endYear": end_year,
    });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère la chronologie d'un élément (article/section) dans un texte.
/// Body : { "textCid": "...", "elementCid": "..." }
pub async fn chrono_article(
    client: &PisteClient,
    text_cid: &str,
    element_cid: &str,
) -> Result<Value> {
    let url = format!("{}/chrono/textCidAndElementCid", client.legifrance_base());
    let body = json!({
        "textCid": text_cid,
        "elementCid": element_cid,
    });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — liens articles ──────────────────────────────────────────────────

/// Liens de concordance d'un article.
/// Body : { "articleId": "LEGIARTI..." }
pub async fn get_concordance_links(client: &PisteClient, article_id: &str) -> Result<Value> {
    let url = format!(
        "{}/consult/concordanceLinksArticle",
        client.legifrance_base()
    );
    let body = json!({ "articleId": article_id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Liens relatifs d'un article.
/// Body : { "articleId": "LEGIARTI..." }
pub async fn get_related_links(client: &PisteClient, article_id: &str) -> Result<Value> {
    let url = format!("{}/consult/relatedLinksArticle", client.legifrance_base());
    let body = json!({ "articleId": article_id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Articles ayant eu le même numéro dans des versions précédentes.
/// Body : { "textCid": "...", "articleCid": "...", "date": "YYYY-MM-DD", "articleNum": "..." }
pub async fn get_same_num_article(
    client: &PisteClient,
    text_cid: &str,
    article_cid: &str,
    date: &str,
    article_num: &str,
) -> Result<Value> {
    let url = format!("{}/consult/sameNumArticle", client.legifrance_base());
    let body = json!({
        "textCid": text_cid,
        "articleCid": article_cid,
        "date": date,
        "articleNum": article_num,
    });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Liens service public d'un article.
/// Body : { "articleCid": "...", "fond": "..." }
pub async fn get_service_public_links(
    client: &PisteClient,
    article_cid: &str,
    fond: Option<&str>,
) -> Result<Value> {
    let url = format!(
        "{}/consult/servicePublicLinksArticle",
        client.legifrance_base()
    );
    let mut body = json!({ "articleCid": article_cid });
    if let Some(f) = fond {
        body["fond"] = json!(f);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — ELI / Alias ─────────────────────────────────────────────────────

/// Récupère un texte JORF par son identifiant ELI ou alias.
/// Body : { "idEliOrAlias": "/eli/decret/..." }
pub async fn get_jorf_by_eli(client: &PisteClient, id_eli_or_alias: &str) -> Result<Value> {
    let url = format!(
        "{}/consult/eliAndAliasRedirectionTexte",
        client.legifrance_base()
    );
    let body = json!({ "idEliOrAlias": id_eli_or_alias });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un article par son identifiant ELI ou alias.
/// Body : { "idEliOrAlias": "/eli/decret/..." }
pub async fn get_article_by_eli(client: &PisteClient, id_eli_or_alias: &str) -> Result<Value> {
    let url = format!(
        "{}/consult/getArticleWithIdEliOrAlias",
        client.legifrance_base()
    );
    let body = json!({ "idEliOrAlias": id_eli_or_alias });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — Anciens identifiants ────────────────────────────────────────────

/// Récupère un texte JURI par son ancien identifiant.
pub async fn get_juri_by_ancien_id(client: &PisteClient, ancien_id: &str) -> Result<Value> {
    let url = format!("{}/consult/getJuriWithAncienId", client.legifrance_base());
    let body = json!({ "ancienId": ancien_id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un Code par son ancien identifiant.
pub async fn get_code_by_ancien_id(client: &PisteClient, ancien_id: &str) -> Result<Value> {
    let url = format!("{}/consult/getCodeWithAncienId", client.legifrance_base());
    let body = json!({ "ancienId": ancien_id });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Récupère un texte CNIL par son ancien identifiant.
pub async fn get_cnil_by_ancien_id(client: &PisteClient, ancien_id: &str) -> Result<Value> {
    let url = format!("{}/consult/getCnilWithAncienId", client.legifrance_base());
    let body = json!({ "ancienId": ancien_id });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — JORF ────────────────────────────────────────────────────────────

/// Récupère un texte JORF complet par son CID (différent de jorfPart).
/// Body : { "textCid": "JORFTEXT..." }
pub async fn get_jorf(
    client: &PisteClient,
    text_cid: &str,
    searched_string: Option<&str>,
) -> Result<Value> {
    let url = format!("{}/consult/jorf", client.legifrance_base());
    let mut body = json!({ "textCid": text_cid });
    if let Some(s) = searched_string {
        body["searchedString"] = json!(s);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — Débats parlementaires ──────────────────────────────────────────

/// Récupère un débat parlementaire par son identifiant.
/// Body : { "id": "AN_2020-090.pdf" }
pub async fn get_debat(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/debat", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — KALI section ────────────────────────────────────────────────────

/// Récupère le contenu d'une section KALI par son identifiant.
pub async fn get_kali_section(client: &PisteClient, id: &str) -> Result<Value> {
    let url = format!("{}/consult/kaliSection", client.legifrance_base());
    let body = json!({ "id": id });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — Jurisprudence plan de classement ────────────────────────────────

/// Récupère le plan de classement de la jurisprudence.
#[allow(clippy::too_many_arguments)]
pub async fn get_juri_plan_classement(
    client: &PisteClient,
    id: Option<&str>,
    fond: Option<&str>,
    libelle: Option<&str>,
    niveau: Option<i32>,
    page: Option<i32>,
    search_suggest: bool,
    search_by_niveau: bool,
) -> Result<Value> {
    let url = format!("{}/consult/getJuriPlanClassement", client.legifrance_base());
    let mut body = json!({
        "searchSuggest": search_suggest,
        "searchByNiveau": search_by_niveau,
    });
    if let Some(v) = id {
        body["id"] = json!(v);
    }
    if let Some(v) = fond {
        body["fond"] = json!(v);
    }
    if let Some(v) = libelle {
        body["libelle"] = json!(v);
    }
    if let Some(v) = niveau {
        body["niveau"] = json!(v);
    }
    if let Some(v) = page {
        body["page"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — Derniers JO ─────────────────────────────────────────────────────

/// Récupère les N derniers Journaux Officiels.
/// Body : { "nbElement": N }
pub async fn get_last_n_jo(client: &PisteClient, nb_element: u32) -> Result<Value> {
    let url = format!("{}/consult/lastNJo", client.legifrance_base());
    let body = json!({ "nbElement": nb_element });
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — Tables annuelles ────────────────────────────────────────────────

/// Tables annuelles Légifrance sur une période.
/// Body : { "endYear": N, "startYear": N }
pub async fn get_tables(
    client: &PisteClient,
    end_year: i32,
    start_year: Option<i32>,
) -> Result<Value> {
    let url = format!("{}/consult/getTables", client.legifrance_base());
    let mut body = json!({ "endYear": end_year });
    if let Some(y) = start_year {
        body["startYear"] = json!(y);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — lawDecree ───────────────────────────────────────────────────────

/// Récupère un texte LODA (loi ou décret) par son id et date — alternative
/// à legiPart avec paramètre `fromSuggest`.
pub async fn get_law_decree(
    client: &PisteClient,
    text_id: &str,
    date: Option<&str>,
    from_suggest: bool,
    searched_string: Option<&str>,
) -> Result<Value> {
    let url = format!("{}/consult/lawDecree", client.legifrance_base());
    let date_str = date
        .map(|d| d.to_string())
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let mut body = json!({
        "textId": text_id,
        "date": date_str,
        "fromSuggest": from_suggest,
    });
    if let Some(s) = searched_string {
        body["searchedString"] = json!(s);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — legi/tableMatieres ─────────────────────────────────────────────

/// Table des matières globale (CODE ou LODA) par identifiant et date.
/// Meilleure alternative à `/consult/code/tableMatieres` (non deprecated).
pub async fn get_legi_table_matieres_v2(
    client: &PisteClient,
    text_id: &str,
    date: Option<&str>,
    nature: Option<&str>,
) -> Result<Value> {
    let url = format!("{}/consult/legi/tableMatieres", client.legifrance_base());
    let date_str = date
        .map(|d| d.to_string())
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let mut body = json!({
        "textId": text_id,
        "date": date_str,
    });
    if let Some(n) = nature {
        body["nature"] = json!(n);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Consult — BOCC ────────────────────────────────────────────────────────────

/// Métadonnées PDF d'un texte BOCC.
pub async fn get_bocc_pdf_metadata(
    client: &PisteClient,
    id: Option<&str>,
    for_global_bocc: bool,
) -> Result<Value> {
    let url = format!(
        "{}/consult/getBoccTextPdfMetadata",
        client.legifrance_base()
    );
    let mut body = json!({ "forGlobalBocc": for_global_bocc });
    if let Some(v) = id {
        body["id"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Listes — BOCC ─────────────────────────────────────────────────────────────

/// Liste les Bulletins Officiels des Conventions Collectives (BOCC globaux).
pub async fn list_bocc(
    client: &PisteClient,
    page_number: u32,
    page_size: u32,
    interval_publication: Option<String>,
    idcc: Option<String>,
    sort: Option<String>,
) -> Result<Value> {
    let url = format!("{}/list/bocc", client.legifrance_base());
    let mut body = json!({
        "pageNumber": page_number.max(1),
        "pageSize": page_size.clamp(1, 100),
        "searchForGlobalBocc": true,
    });
    if let Some(v) = interval_publication {
        body["intervalPublication"] = json!(v);
    }
    if let Some(v) = idcc {
        body["idccs"] = json!([v]);
    }
    if let Some(v) = sort {
        body["sortValue"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

/// Liste les textes d'un BOCC.
/// NB : le schéma officiel `BoccListRequest` (swagger Légifrance) n'a pas de
/// champ `idcc` (singulier) mais `idccs` (tableau de chaînes) — un champ
/// inconnu dans le body fait planter PISTE en 500 ("Une exception non gérée
/// est survenue"). D'où l'encapsulation de `idcc` dans un tableau à un
/// élément avant l'envoi.
pub async fn list_bocc_texts(
    client: &PisteClient,
    page_number: u32,
    page_size: u32,
    interval_publication: Option<String>,
    idcc: Option<String>,
    titre: Option<String>,
    sort: Option<String>,
) -> Result<Value> {
    let url = format!("{}/list/boccTexts", client.legifrance_base());
    let mut body = json!({
        "pageNumber": page_number.max(1),
        "pageSize": page_size.clamp(1, 100),
    });
    if let Some(v) = interval_publication {
        body["intervalPublication"] = json!(v);
    }
    if let Some(v) = idcc {
        body["idccs"] = json!([v]);
    }
    if let Some(v) = titre {
        body["titre"] = json!(v);
    }
    if let Some(v) = sort {
        body["sortValue"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

/// Liste BOCC globaux et leurs textes.
pub async fn list_boccs_and_texts(
    client: &PisteClient,
    page_number: u32,
    page_size: u32,
    id_global_bocc: Option<String>,
    interval_publication: Option<String>,
    sort: Option<String>,
) -> Result<Value> {
    let url = format!("{}/list/boccsAndTexts", client.legifrance_base());
    let mut body = json!({
        "pageNumber": page_number.max(1),
        "pageSize": page_size.clamp(1, 100),
    });
    if let Some(v) = id_global_bocc {
        body["idGlobalBocc"] = json!(v);
    }
    if let Some(v) = interval_publication {
        body["intervalPublication"] = json!(v);
    }
    if let Some(v) = sort {
        body["sortValue"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Listes — BODMR ────────────────────────────────────────────────────────────

/// Liste les Bulletins Officiels Des Marchés Réglementés.
pub async fn list_bodmr(
    client: &PisteClient,
    page_number: u32,
    page_size: u32,
    years: Option<Vec<i32>>,
    sort: Option<String>,
) -> Result<Value> {
    let url = format!("{}/list/bodmr", client.legifrance_base());
    let mut body = json!({
        "pageNumber": page_number.max(1),
        "pageSize": page_size.clamp(1, 100),
    });
    if let Some(v) = years {
        body["years"] = json!(v);
    }
    if let Some(v) = sort {
        body["sort"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Listes — Conventions collectives ─────────────────────────────────────────

/// Liste les conventions collectives (KALI).
#[allow(clippy::too_many_arguments)]
pub async fn list_conventions(
    client: &PisteClient,
    page_number: u32,
    page_size: u32,
    idcc: Option<String>,
    titre: Option<String>,
    legal_status: Option<Vec<String>>,
    key_words: Option<Vec<String>>,
    sort: Option<String>,
) -> Result<Value> {
    let url = format!("{}/list/conventions", client.legifrance_base());
    let mut body = json!({
        "pageNumber": page_number.max(1),
        "pageSize": page_size.clamp(1, 100),
    });
    if let Some(v) = idcc {
        body["idcc"] = json!(v);
    }
    if let Some(v) = titre {
        body["titre"] = json!(v);
    }
    if let Some(v) = legal_status {
        body["legalStatus"] = json!(v);
    }
    if let Some(v) = key_words {
        body["keyWords"] = json!(v);
    }
    if let Some(v) = sort {
        body["sort"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Listes — Débats parlementaires ───────────────────────────────────────────

/// Liste les débats parlementaires.
pub async fn list_debats_parlementaires(
    client: &PisteClient,
    page_number: u32,
    page_size: u32,
    date_parution: Option<String>,
    types_publication: Option<Vec<String>>,
    sort: Option<String>,
    second_sort: Option<String>,
) -> Result<Value> {
    let url = format!("{}/list/debatsParlementaires", client.legifrance_base());
    let mut body = json!({
        "pageNumber": page_number.max(1),
        "pageSize": page_size.clamp(1, 100),
    });
    if let Some(v) = date_parution {
        body["dateParution"] = json!(v);
    }
    if let Some(v) = types_publication {
        body["typesPublication"] = json!(v);
    }
    if let Some(v) = sort {
        body["sortValue"] = json!(v);
    }
    if let Some(v) = second_sort {
        body["secondSortValue"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Listes — Docs administratifs ─────────────────────────────────────────────

/// Liste les documents administratifs.
pub async fn list_docs_admins(client: &PisteClient, years: Option<Vec<i32>>) -> Result<Value> {
    let url = format!("{}/list/docsAdmins", client.legifrance_base());
    let mut body = json!({});
    if let Some(v) = years {
        body["years"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Listes — Dossiers législatifs ────────────────────────────────────────────

/// Liste les dossiers législatifs par législature.
pub async fn list_dossiers_legislatifs(
    client: &PisteClient,
    type_: &str,
    legislature_id: i32,
) -> Result<Value> {
    let url = format!("{}/list/dossiersLegislatifs", client.legifrance_base());
    let body = json!({
        "type": type_,
        "legislatureId": legislature_id,
    });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Liste les législatures disponibles.
pub async fn list_legislatures(client: &PisteClient) -> Result<Value> {
    let url = format!("{}/list/legislatures", client.legifrance_base());
    let body = json!({});
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Listes — Questions écrites parlementaires ─────────────────────────────────

/// Liste les questions écrites parlementaires.
pub async fn list_questions_ecrites(
    client: &PisteClient,
    page_number: u32,
    page_size: u32,
    periode_publication: Option<String>,
    parlement_types: Option<Vec<String>>,
    sort: Option<String>,
    second_sort: Option<String>,
) -> Result<Value> {
    let url = format!(
        "{}/list/questionsEcritesParlementaires",
        client.legifrance_base()
    );
    let mut body = json!({
        "pageNumber": page_number.max(1),
        "pageSize": page_size.clamp(1, 100),
    });
    if let Some(v) = periode_publication {
        body["periodePublication"] = json!(v);
    }
    if let Some(v) = parlement_types {
        body["parlementTypes"] = json!(v);
    }
    if let Some(v) = sort {
        body["sortValue"] = json!(v);
    }
    if let Some(v) = second_sort {
        body["secondSortValue"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Recherche — Versions canoniques ──────────────────────────────────────────

/// Recherche la version canonique d'un texte (la plus représentative).
///
/// Non encore câblée à une route/commande ; conservée pour couverture complète
/// de l'API Legifrance (endpoints `/search/*`).
#[allow(dead_code)]
pub async fn search_canonical_version(client: &PisteClient, body: Value) -> Result<Value> {
    let url = format!("{}/search/canonicalVersion", client.legifrance_base());
    client.post_json::<Value, Value>(&url, &body).await
}

/// Recherche la version canonique d'un article.
#[allow(dead_code)]
pub async fn search_canonical_article_version(client: &PisteClient, body: Value) -> Result<Value> {
    let url = format!(
        "{}/search/canonicalArticleVersion",
        client.legifrance_base()
    );
    client.post_json::<Value, Value>(&url, &body).await
}

/// Recherche la version la plus proche d'une date.
#[allow(dead_code)]
pub async fn search_nearest_version(client: &PisteClient, body: Value) -> Result<Value> {
    let url = format!("{}/search/nearestVersion", client.legifrance_base());
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Suggest — Accords / PDC ───────────────────────────────────────────────────

/// Suggestions spécifiques aux accords d'entreprise.
pub async fn suggest_acco(client: &PisteClient, search_text: &str) -> Result<Value> {
    let url = format!("{}/suggest/acco", client.legifrance_base());
    let body = json!({ "searchText": search_text });
    client.post_json::<Value, Value>(&url, &body).await
}

/// Suggestions pour le plan de classement (PDC).
pub async fn suggest_pdc(
    client: &PisteClient,
    search_text: &str,
    fond: Option<&str>,
    origin: Option<&str>,
) -> Result<Value> {
    let url = format!("{}/suggest/pdc", client.legifrance_base());
    let mut body = json!({ "searchText": search_text });
    if let Some(v) = fond {
        body["fond"] = json!(v);
    }
    if let Some(v) = origin {
        body["origin"] = json!(v);
    }
    client.post_json::<Value, Value>(&url, &body).await
}

// ── Misc ──────────────────────────────────────────────────────────────────────

/// Dates sans Journal Officiel (jours fériés, etc.).
pub async fn get_dates_without_jo(client: &PisteClient) -> Result<Value> {
    let url = format!("{}/misc/datesWithoutJo", client.legifrance_base());
    client.get_json::<Value>(&url).await
}
