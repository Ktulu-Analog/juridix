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
// Juridix — Module Juridique — code_search.rs
// ============================================================================
// Recherche dans les CODES (fonds CODE_ETAT / CODE_DATE) — orchestration des
// appels PISTE. L'interprétation de la saisie est dans `code_query.rs`.
//
//   • « code civil, article 1140 » → recherche NUM_ARTICLE filtrée par code
//     (même stratégie que la vérification des citations, `verify.rs`) ;
//   • phrase libre (tous codes ou un code) → recherche dans le CONTENU des
//     articles, par paliers : expression exacte, puis tous les mots, puis un
//     des mots ;
//   • « code civil » seul, champ TITLE/TABLE… → `None` : l'appelant garde le
//     chemin historique (`legifrance::search_simple`).
//
// Les résultats sont renvoyés AU NIVEAU DE L'ARTICLE (id `LEGIARTI…`) : le
// frontend les ouvre directement via /article/:id.
// ============================================================================

use anyhow::Result;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::client::PisteClient;
use super::code_query::{
    classify, collect_hits, hit_to_result, only_code, paliers, trier_hits, Correspondance, Hit,
    NumeroArticle, Palier, Parsed,
};
use super::legifrance::{
    self, ChampRecherche, Critere, FiltreDates, FiltreValeurs, Operateur, SearchFond, SearchParams,
    TypeRecherche,
};
use super::verify_ext::article_text_of;

/// Résultats demandés à PISTE par recherche d'article (un numéro existe dans
/// plusieurs codes : 50 couvre largement le cas).
const PAGE_SIZE_ARTICLE: u32 = 50;
/// Nombre d'articles dont on va chercher le texte pour l'aperçu.
const APERCU_MAX: usize = 3;
/// Longueur de l'aperçu (caractères).
const APERCU_LEN: usize = 500;

/// Paramètres de la recherche, tels que reçus par `/rechercher`.
pub struct CodeSearchRequest<'a> {
    pub query: &'a str,
    /// `CodeEtat` ou `CodeDate`.
    pub fond: SearchFond,
    /// Champ choisi dans l'interface (« ALL » par défaut).
    pub champ: &'a str,
    pub page: u32,
    pub page_size: u32,
    pub sort: Option<String>,
    pub legal_status: Option<Vec<String>>,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    /// « Droit applicable au » (AAAA-MM-JJ) : versions en vigueur à cette date (fond CODE_DATE,
    /// facette DATE_VERSION en `singleDate`). Prioritaire sur `date_start` / `date_end`.
    pub date_applicable: Option<String>,
    /// Palier à rejouer pour les pages suivantes d'une recherche plein texte
    /// (valeur renvoyée dans `recherche.palier`).
    pub palier: Option<&'a str>,
}

/// `Ok(None)` : la saisie n'est pas concernée, l'appelant poursuit normalement.
pub async fn search_codes(
    client: &PisteClient,
    req: &CodeSearchRequest<'_>,
) -> Result<Option<Value>> {
    if !req.fond.is_code() {
        return Ok(None);
    }
    match classify(req.query, req.champ) {
        Parsed::Passthrough => Ok(None),
        Parsed::Article { code, numeros } => {
            debug!("[Codes] article code={:?} numeros={:?}", code, numeros);
            search_articles(client, req, code, &numeros).await.map(Some)
        }
        Parsed::Text { code, phrase } => {
            debug!("[Codes] texte code={:?} phrase={:?}", code, phrase);
            search_text(client, req, code, &phrase).await.map(Some)
        }
    }
}

// ── Construction des requêtes ─────────────────────────────────────────────────

/// Fond réellement interrogé. CODE_ETAT par défaut (filtres code + état
/// juridique, comme `verify.rs`). CODE_DATE seulement quand l'utilisateur a
/// borné une date : seul ce fond sait filtrer sur la date de version.
fn fond_effectif(req: &CodeSearchRequest<'_>) -> SearchFond {
    let periode = req.date_start.is_some() || req.date_end.is_some();
    if req.date_applicable.is_some() || (matches!(req.fond, SearchFond::CodeDate) && periode) {
        SearchFond::CodeDate
    } else {
        SearchFond::CodeEtat
    }
}

fn statuts_demandes(req: &CodeSearchRequest<'_>) -> Option<Vec<String>> {
    req.legal_status.clone().filter(|v| !v.is_empty())
}

#[allow(clippy::too_many_arguments)]
fn construire(
    fond: &SearchFond,
    champ: &str,
    valeur: &str,
    correspondance: Correspondance,
    code: Option<&str>,
    etats: Option<Vec<String>>,
    req: &CodeSearchRequest<'_>,
    page: u32,
    page_size: u32,
) -> SearchParams {
    let mut filtres_valeurs = Vec::new();
    // NOM_CODE n'est utilisé qu'avec CODE_ETAT (« NOM_CODE » y est traduit en
    // TEXT_NOM_CODE par `SearchParams::to_api_body`) ; sur CODE_DATE le code est
    // filtré après coup (`only_code`) plutôt que de risquer une facette invalide.
    if matches!(fond, SearchFond::CodeEtat) {
        if let Some(c) = code {
            filtres_valeurs.push(FiltreValeurs {
                facette: "NOM_CODE".into(),
                valeurs: vec![c.to_string()],
            });
        }
        if let Some(e) = etats.filter(|e| !e.is_empty()) {
            // « Etat juridique des articles » = ARTICLE_LEGAL_STATUS (fiche CODE de la
            // documentation PISTE) ; TEXT_LEGAL_STATUS désigne l'état du CODE lui-même.
            filtres_valeurs.push(FiltreValeurs {
                facette: "ARTICLE_LEGAL_STATUS".to_string(),
                valeurs: e,
            });
        }
    }

    let mut filtres_dates = Vec::new();
    if matches!(fond, SearchFond::CodeDate) {
        if let Some(f) = fond.date_facette() {
            filtres_dates.push(match &req.date_applicable {
                // Version applicable à une date : `singleDate` (exemples officiels PISTE).
                Some(d) => FiltreDates {
                    facette: f.to_string(),
                    start: None,
                    end: None,
                    single_date: Some(d.clone()),
                },
                None => FiltreDates {
                    facette: f.to_string(),
                    start: req.date_start.clone(),
                    end: req.date_end.clone(),
                    single_date: None,
                },
            });
        }
    }

    let type_recherche = match correspondance {
        Correspondance::Exacte => TypeRecherche::Exacte,
        Correspondance::TousLesMots => TypeRecherche::TousLesMotsDansUnChamp,
        Correspondance::UnDesMots => TypeRecherche::UnDesMots,
    };

    SearchParams {
        fond: fond.clone(),
        champs: vec![ChampRecherche {
            type_champ: champ.to_string(),
            criteres: vec![Critere {
                valeur: valeur.to_string(),
                type_recherche,
                operateur: Operateur::Et,
                proximite: None,
            }],
            operateur: Operateur::Et,
        }],
        operateur: Operateur::Et,
        filtres_valeurs,
        filtres_dates,
        sort: req.sort.clone(),
        second_sort: None,
        page_number: page,
        page_size,
    }
}

async fn lancer(client: &PisteClient, params: SearchParams) -> Result<Value> {
    let resp = legifrance::search(client, params).await?;
    Ok(serde_json::to_value(resp)?)
}

// ── Article(s) par numéro ─────────────────────────────────────────────────────

struct TrouveArticle {
    hits: Vec<Hit>,
    /// Codes où le numéro existe, quand il est introuvable dans le code demandé.
    ailleurs: Vec<String>,
}

async fn find_article(
    client: &PisteClient,
    req: &CodeSearchRequest<'_>,
    fond: &SearchFond,
    code: Option<&'static str>,
    num: &NumeroArticle,
) -> Result<TrouveArticle> {
    let choisis = statuts_demandes(req);
    let par_date = matches!(fond, SearchFond::CodeDate);
    let exact = |resp: &Value| {
        let h = collect_hits(resp, Some(&num.key));
        match code {
            Some(c) => only_code(h, c),
            None => h,
        }
    };
    let vide = |ailleurs| TrouveArticle {
        hits: Vec::new(),
        ailleurs,
    };

    // A. Version en vigueur (ou les états demandés) dans ce code.
    let etats_a = if par_date {
        None
    } else {
        Some(
            choisis
                .clone()
                .unwrap_or_else(|| vec!["VIGUEUR".to_string()]),
        )
    };
    let resp = lancer(
        client,
        construire(
            fond,
            "NUM_ARTICLE",
            &num.query,
            Correspondance::Exacte,
            code,
            etats_a,
            req,
            1,
            PAGE_SIZE_ARTICLE,
        ),
    )
    .await?;
    let hits = exact(&resp);
    if !hits.is_empty() {
        return Ok(TrouveArticle {
            hits,
            ailleurs: Vec::new(),
        });
    }

    // B. Tous états (article abrogé, périmé, transféré…) — sauf si l'utilisateur
    //    a lui-même choisi des états, ou si on filtre par date.
    if !par_date && choisis.is_none() {
        let resp = lancer(
            client,
            construire(
                fond,
                "NUM_ARTICLE",
                &num.query,
                Correspondance::Exacte,
                code,
                None,
                req,
                1,
                PAGE_SIZE_ARTICLE,
            ),
        )
        .await?;
        let hits = exact(&resp);
        if !hits.is_empty() {
            return Ok(TrouveArticle {
                hits,
                ailleurs: Vec::new(),
            });
        }
    }

    // C. Sans filtre de code : garde-fou contre une facette mal interprétée, et
    //    source de « existe dans un autre code ».
    let Some(code) = code else {
        return Ok(vide(Vec::new()));
    };
    let resp = lancer(
        client,
        construire(
            fond,
            "NUM_ARTICLE",
            &num.query,
            Correspondance::Exacte,
            None,
            None,
            req,
            1,
            PAGE_SIZE_ARTICLE,
        ),
    )
    .await?;
    let tous = collect_hits(&resp, Some(&num.key));
    let meme_code = only_code(tous.clone(), code);
    if !meme_code.is_empty() {
        return Ok(TrouveArticle {
            hits: meme_code,
            ailleurs: Vec::new(),
        });
    }
    let mut ailleurs: Vec<String> = tous.into_iter().filter_map(|h| h.code_titre).collect();
    ailleurs.sort();
    ailleurs.dedup();
    ailleurs.truncate(5);
    Ok(vide(ailleurs))
}

/// Aperçu (texte brut tronqué) d'un article, `None` si indisponible. Non
/// bloquant : une erreur n'empêche jamais d'afficher le résultat.
async fn apercu(client: &PisteClient, id: &str) -> Option<String> {
    match legifrance::get_article(client, id).await {
        Ok(v) => article_text_of(&v).map(|t| tronquer(&t, APERCU_LEN)),
        Err(e) => {
            warn!("[Codes] aperçu {id} : {e}");
            None
        }
    }
}

fn tronquer(s: &str, max: usize) -> String {
    let t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        return t;
    }
    let mut out: String = t.chars().take(max).collect();
    out.push('…');
    out
}

async fn search_articles(
    client: &PisteClient,
    req: &CodeSearchRequest<'_>,
    code: Option<&'static str>,
    numeros: &[NumeroArticle],
) -> Result<Value> {
    let fond = fond_effectif(req);
    let mut hits: Vec<Hit> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    for num in numeros {
        let trouve = find_article(client, req, &fond, code, num).await?;
        if trouve.hits.is_empty() {
            let numero = &num.query;
            notes.push(match (code, trouve.ailleurs.is_empty()) {
                (Some(c), true) => format!("Aucun article {numero} trouvé dans le {c}."),
                (Some(c), false) => format!(
                    "Aucun article {numero} dans le {c} ; ce numéro existe dans : {}.",
                    trouve.ailleurs.join(", ")
                ),
                (None, _) => format!("Aucun article {numero} trouvé dans les codes."),
            });
        }
        for h in trouve.hits {
            if !hits.iter().any(|x| x.id == h.id) {
                hits.push(h);
            }
        }
    }
    // Plusieurs numéros demandés : on garde l'ordre de saisie (tri stable par
    // état seulement à l'intérieur d'un même numéro serait plus fin, mais le tri
    // global « en vigueur d'abord » est ce que l'utilisateur attend).
    if numeros.len() == 1 {
        trier_hits(&mut hits);
    }

    // « Code civil » demandé mais seul l'article d'un autre code a été trouvé :
    // `find_article` ne renvoie alors AUCUN hit (on ne présente pas l'article du
    // Code de commerce comme celui du Code civil) ; la note le dit.
    let page = req.page.max(1);
    let mut results: Vec<Value> = Vec::new();
    if page == 1 {
        for (i, h) in hits.iter().enumerate() {
            let texte = if i < APERCU_MAX && h.values.is_empty() {
                apercu(client, &h.id).await
            } else {
                None
            };
            results.push(hit_to_result(h, texte.as_deref()));
        }
    }

    Ok(json!({
        "totalResultNumber": hits.len(),
        "results": results,
        "recherche": {
            "mode": "article",
            "code": code,
            "articles": numeros.iter().map(|n| n.query.clone()).collect::<Vec<_>>(),
            "date_applicable": req.date_applicable,
            "note": if notes.is_empty() { Value::Null } else { json!(notes.join(" ")) },
        },
    }))
}

// ── Texte libre ───────────────────────────────────────────────────────────────

/// Exécute les paliers dans l'ordre et renvoie le premier qui donne des
/// articles. `code_facette` : filtre de code côté API (`None` = pas de filtre).
async fn jouer_paliers(
    client: &PisteClient,
    req: &CodeSearchRequest<'_>,
    fond: &SearchFond,
    code_facette: Option<&str>,
    code_filtre: Option<&str>,
    liste: &[Palier],
) -> Result<Option<(Palier, Value, Vec<Hit>)>> {
    for p in liste {
        let params = construire(
            fond,
            "ARTICLE",
            &p.valeur,
            p.correspondance,
            code_facette,
            statuts_demandes(req),
            req,
            req.page.max(1),
            req.page_size.clamp(1, 100),
        );
        let resp = lancer(client, params).await?;
        let mut hits = collect_hits(&resp, None);
        if let Some(c) = code_filtre {
            hits = only_code(hits, c);
        }
        if !hits.is_empty() {
            return Ok(Some((p.clone(), resp, hits)));
        }
    }
    Ok(None)
}

fn note_palier(p: &Palier) -> Option<String> {
    match p.correspondance {
        Correspondance::Exacte => None,
        Correspondance::TousLesMots => Some(format!(
            "Aucun article ne contient exactement cette expression : résultats contenant tous les mots « {} ».",
            p.valeur
        )),
        Correspondance::UnDesMots => Some(format!(
            "Aucun article ne contient tous les mots : résultats approchés, contenant au moins un des mots « {} », classés par pertinence.",
            p.valeur
        )),
    }
}

async fn search_text(
    client: &PisteClient,
    req: &CodeSearchRequest<'_>,
    code: Option<&'static str>,
    phrase: &str,
) -> Result<Value> {
    let fond = fond_effectif(req);
    let mut liste = paliers(phrase);
    // Page suivante : on rejoue le palier qui avait donné la page 1.
    if let Some(demande) = req.palier {
        if let Some(p) = liste.iter().find(|p| p.correspondance.label() == demande) {
            liste = vec![p.clone()];
        }
    }

    // Avec filtre de code côté API (CODE_ETAT), sinon filtre après coup.
    let facette = if matches!(fond, SearchFond::CodeEtat) {
        code
    } else {
        None
    };
    let mut trouve = match jouer_paliers(client, req, &fond, facette, code, &liste).await {
        Ok(t) => t,
        // Facette de code rejetée par l'API (erreur, pas seulement « 0 résultat ») :
        // on retente sans facette, le filtrage par code se fait alors côté Juridix.
        Err(e) if facette.is_some() => {
            warn!(
                "[Codes] recherche avec facette de code en échec ({e}) — nouvel essai sans facette"
            );
            None
        }
        Err(e) => return Err(e),
    };
    // Garde-fou : rien trouvé avec la facette de code → on réessaie sans
    // facette en filtrant côté Juridix (une facette mal interprétée par l'API ne
    // doit jamais se traduire par « aucun résultat »).
    if trouve.is_none() && facette.is_some() {
        trouve = jouer_paliers(client, req, &fond, None, code, &liste).await?;
    }

    let Some((palier, resp, hits)) = trouve else {
        return Ok(json!({
            "totalResultNumber": 0,
            "results": [],
            "recherche": {
                "mode": "texte",
                "code": code,
                "phrase": phrase,
                "palier": Value::Null,
                "note": match code {
                    Some(c) => format!("Aucun article du {c} ne correspond à « {phrase} »."),
                    None => format!("Aucun article de code ne correspond à « {phrase} »."),
                },
            },
        }));
    };

    let total = resp
        .get("totalResultNumber")
        .and_then(Value::as_i64)
        .unwrap_or(hits.len() as i64);
    let results: Vec<Value> = hits.iter().map(|h| hit_to_result(h, None)).collect();
    let n = results.len();

    Ok(json!({
        "totalResultNumber": total,
        "results": results,
        "recherche": {
            "mode": "texte",
            "code": code,
            "phrase": phrase,
            "palier": palier.correspondance.label(),
            "date_applicable": req.date_applicable,
            "articles_affiches": n,
            "note": note_palier(&palier),
        },
    }))
}

// ============================================================================
// Tests — faux serveur Légifrance (même principe que `verify.rs`)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, routing::post, Json, Router};
    use std::sync::{Arc, Mutex};

    /// Un article du corpus factice.
    #[derive(Clone)]
    struct Art {
        id: &'static str,
        code: &'static str,
        code_id: &'static str,
        num: &'static str,
        etat: &'static str,
        texte: &'static str,
    }

    #[derive(Clone)]
    struct Mock {
        corpus: Arc<Vec<Art>>,
        /// Corps reçus par /search (pour vérifier les requêtes envoyées).
        recus: Arc<Mutex<Vec<Value>>>,
        /// Simule une facette de code ignorée par l'API.
        ignore_facette_code: bool,
    }

    fn mots(s: &str) -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect()
    }

    async fn mock_search(State(st): State<Mock>, Json(body): Json<Value>) -> Json<Value> {
        st.recus.lock().unwrap().push(body.clone());
        let rech = &body["recherche"];
        let champ = &rech["champs"][0];
        let type_champ = champ["typeChamp"].as_str().unwrap_or("");
        let crit = &champ["criteres"][0];
        let valeur = crit["valeur"].as_str().unwrap_or("").to_string();
        let type_rech = crit["typeRecherche"].as_str().unwrap_or("");

        let mut code_demande: Option<String> = None;
        let mut etats: Option<Vec<String>> = None;
        for f in rech["filtres"].as_array().into_iter().flatten() {
            let vals: Vec<String> = f["valeurs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            match f["facette"].as_str() {
                Some("TEXT_NOM_CODE") if !st.ignore_facette_code => {
                    code_demande = vals.first().cloned()
                }
                Some("ARTICLE_LEGAL_STATUS") => etats = Some(vals),
                _ => {}
            }
        }

        let ok = |a: &Art| -> bool {
            if let Some(c) = &code_demande {
                if a.code != c {
                    return false;
                }
            }
            if let Some(e) = &etats {
                if !e.iter().any(|x| x == a.etat) {
                    return false;
                }
            }
            match type_champ {
                "NUM_ARTICLE" => a
                    .num
                    .replace(' ', "")
                    .eq_ignore_ascii_case(&valeur.replace(' ', "")),
                "ARTICLE" => {
                    let texte = mots(a.texte);
                    let voulus = mots(&valeur);
                    match type_rech {
                        "EXACTE" => a.texte.to_lowercase().contains(&valeur.to_lowercase()),
                        "TOUS_LES_MOTS_DANS_UN_CHAMP" => voulus.iter().all(|w| texte.contains(w)),
                        "UN_DES_MOTS" => voulus.iter().any(|w| texte.contains(w)),
                        _ => false,
                    }
                }
                _ => false,
            }
        };

        // Regroupement par code, comme l'API (un résultat par code).
        let mut codes: Vec<(&str, &str)> = Vec::new();
        for a in st.corpus.iter().filter(|a| ok(a)) {
            if !codes.iter().any(|(c, _)| *c == a.code) {
                codes.push((a.code, a.code_id));
            }
        }
        let results: Vec<Value> = codes
            .iter()
            .map(|(c, cid)| {
                let extracts: Vec<Value> = st
                    .corpus
                    .iter()
                    .filter(|a| a.code == *c && ok(a))
                    .map(|a| {
                        json!({
                            "id": a.id, "num": a.num, "legalStatus": a.etat,
                            "values": [format!("<mark>{}</mark>", a.texte)],
                        })
                    })
                    .collect();
                json!({
                    "titles": [{ "id": cid, "cid": cid, "title": c }],
                    "sections": [{ "id": "LEGISCTA0", "extracts": extracts }],
                })
            })
            .collect();
        Json(json!({ "totalResultNumber": results.len(), "results": results }))
    }

    async fn mock_get_article(State(st): State<Mock>, Json(body): Json<Value>) -> Json<Value> {
        let id = body["id"].as_str().unwrap_or("");
        let a = st.corpus.iter().find(|a| a.id == id);
        Json(json!({ "article": { "id": id, "texte": a.map(|a| a.texte).unwrap_or("") } }))
    }

    fn corpus() -> Vec<Art> {
        vec![
            Art {
                id: "LEGIARTI000032040795",
                code: "Code civil",
                code_id: "LEGITEXT000006070721",
                num: "1140",
                etat: "VIGUEUR",
                texte: "Il y a violence lorsqu'une partie s'engage sous la pression d'une contrainte.",
            },
            Art {
                id: "LEGIARTI000006436000",
                code: "Code civil",
                code_id: "LEGITEXT000006070721",
                num: "1240",
                etat: "VIGUEUR",
                texte: "Tout fait quelconque de l'homme, qui cause à autrui un dommage, oblige celui par la faute duquel il est arrivé à le réparer.",
            },
            Art {
                id: "LEGIARTI000006436100",
                code: "Code civil",
                code_id: "LEGITEXT000006070721",
                num: "1242",
                etat: "VIGUEUR",
                texte: "On est responsable non seulement du dommage que l'on cause par son propre fait, mais encore de celui qui est causé par le fait des choses que l'on a sous sa garde.",
            },
            Art {
                id: "LEGIARTI000006900099",
                code: "Code civil",
                code_id: "LEGITEXT000006070721",
                num: "1384",
                etat: "ABROGE",
                texte: "On est responsable du dommage causé par le fait des choses que l'on a sous sa garde (ancienne numérotation).",
            },
            Art {
                id: "LEGIARTI000006900999",
                code: "Code de commerce",
                code_id: "LEGITEXT000005634379",
                num: "1140",
                etat: "VIGUEUR",
                texte: "Article du code de commerce portant le même numéro.",
            },
            Art {
                id: "LEGIARTI000006901000",
                code: "Code de commerce",
                code_id: "LEGITEXT000005634379",
                num: "L110-1",
                etat: "VIGUEUR",
                texte: "La loi répute actes de commerce tout achat de biens meubles pour les revendre.",
            },
            Art {
                id: "LEGIARTI000006901111",
                code: "Code du travail",
                code_id: "LEGITEXT000006072050",
                num: "L1234-5",
                etat: "VIGUEUR",
                texte: "Lorsque le salarié n'exécute pas le préavis, il a droit à une indemnité compensatrice.",
            },
        ]
    }

    async fn demarrer(ignore_facette_code: bool) -> (PisteClient, Mock) {
        let st = Mock {
            corpus: Arc::new(corpus()),
            recus: Arc::new(Mutex::new(Vec::new())),
            ignore_facette_code,
        };
        let app = Router::new()
            .route(
                "/oauth/token",
                post(|| async { Json(json!({"access_token": "t", "expires_in": 3600})) }),
            )
            .route("/legifrance/search", post(mock_search))
            .route("/legifrance/consult/getArticle", post(mock_get_article))
            .with_state(st.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (PisteClient::new_for_test(&format!("http://{addr}")), st)
    }

    fn req<'a>(query: &'a str, champ: &'a str) -> CodeSearchRequest<'a> {
        CodeSearchRequest {
            query,
            fond: SearchFond::CodeEtat,
            champ,
            page: 1,
            page_size: 10,
            sort: None,
            legal_status: None,
            date_start: None,
            date_end: None,
            date_applicable: None,
            palier: None,
        }
    }

    fn ids(v: &Value) -> Vec<String> {
        v["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn code_civil_article_1140_renvoie_uniquement_cet_article() {
        let (client, st) = demarrer(false).await;
        for q in [
            "code civil, article 1140",
            "article 1140 du code civil",
            "1140 code civil",
        ] {
            let v = search_codes(&client, &req(q, "ALL"))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(ids(&v), ["LEGIARTI000032040795"], "saisie : {q:?}");
            assert_eq!(v["results"][0]["title"], "Code civil — Article 1140");
            assert_eq!(v["recherche"]["mode"], "article");
            assert_eq!(v["recherche"]["code"], "Code civil");
            // Aperçu récupéré via getArticle (le mock ne surligne pas NUM_ARTICLE… si).
            assert!(v["results"][0]["sections"][0]["extracts"][0]["values"][0]
                .as_str()
                .unwrap()
                .contains("violence"));
        }
        // La requête envoyée : NUM_ARTICLE + facette de code + état VIGUEUR de l'article.
        let recu = st.recus.lock().unwrap()[0].clone();
        assert_eq!(recu["fond"], "CODE_ETAT");
        assert_eq!(recu["recherche"]["champs"][0]["typeChamp"], "NUM_ARTICLE");
        assert_eq!(
            recu["recherche"]["champs"][0]["criteres"][0]["valeur"],
            "1140"
        );
        let facettes: Vec<String> = recu["recherche"]["filtres"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["facette"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(facettes, ["TEXT_NOM_CODE", "ARTICLE_LEGAL_STATUS"]);
    }

    #[tokio::test]
    async fn un_article_en_vigueur_coute_un_seul_appel_search() {
        let (client, st) = demarrer(false).await;
        search_codes(&client, &req("code civil article 1140", "ALL"))
            .await
            .unwrap();
        assert_eq!(st.recus.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn article_abroge_retrouve_a_l_etape_b() {
        let (client, _) = demarrer(false).await;
        let v = search_codes(&client, &req("code civil article 1384", "ALL"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ids(&v), ["LEGIARTI000006900099"]);
        assert_eq!(v["results"][0]["etat"], "ABROGE");
    }

    #[tokio::test]
    async fn article_sans_code_donne_un_resultat_par_code() {
        let (client, _) = demarrer(false).await;
        let v = search_codes(&client, &req("article 1140", "ALL"))
            .await
            .unwrap()
            .unwrap();
        let titres: Vec<_> = v["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["title"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            titres,
            [
                "Code civil — Article 1140",
                "Code de commerce — Article 1140"
            ]
        );
    }

    #[tokio::test]
    async fn numero_existant_ailleurs_est_signale_sans_etre_presente_comme_trouve() {
        let (client, _) = demarrer(false).await;
        let v = search_codes(&client, &req("code du travail article 1140", "ALL"))
            .await
            .unwrap()
            .unwrap();
        assert!(ids(&v).is_empty());
        let note = v["recherche"]["note"].as_str().unwrap();
        assert!(
            note.contains("Aucun article 1140 dans le Code du travail"),
            "{note}"
        );
        assert!(
            note.contains("Code civil") && note.contains("Code de commerce"),
            "{note}"
        );
    }

    #[tokio::test]
    async fn facette_de_code_ignoree_par_l_api_le_filtrage_se_fait_cote_juridix() {
        let (client, _) = demarrer(true).await;
        let v = search_codes(&client, &req("code civil article 1140", "ALL"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ids(&v), ["LEGIARTI000032040795"]);
    }

    #[tokio::test]
    async fn plusieurs_articles_une_saisie() {
        let (client, _) = demarrer(false).await;
        let v = search_codes(&client, &req("articles 1140 et 1240 du code civil", "ALL"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ids(&v), ["LEGIARTI000032040795", "LEGIARTI000006436000"]);
    }

    #[tokio::test]
    async fn phrase_exacte_dans_tous_les_codes() {
        let (client, _) = demarrer(false).await;
        let v = search_codes(
            &client,
            &req("achat de biens meubles pour les revendre", "ALL"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(ids(&v), ["LEGIARTI000006901000"]);
        assert_eq!(v["recherche"]["palier"], "exacte");
        assert!(v["recherche"]["note"].is_null());
        // Recherche dans le contenu des articles, pas dans « tous les champs ».
        // (le mock ne répond que sur ARTICLE et NUM_ARTICLE)
    }

    #[tokio::test]
    async fn phrase_approximative_bascule_sur_tous_les_mots() {
        let (client, st) = demarrer(false).await;
        let v = search_codes(
            &client,
            &req(
                "responsabilité du fait des choses que l'on a sous sa garde",
                "ALL",
            ),
        )
        .await
        .unwrap()
        .unwrap();
        // « responsabilité » n'est pas dans le texte (« responsable ») → tous les mots échoue ;
        // « un des mots » ramène les articles pertinents.
        assert_eq!(v["recherche"]["palier"], "un_des_mots");
        assert!(ids(&v).contains(&"LEGIARTI000006436100".to_string()));
        assert!(v["recherche"]["note"]
            .as_str()
            .unwrap()
            .contains("approchés"));
        let types: Vec<String> = st
            .recus
            .lock()
            .unwrap()
            .iter()
            .map(|b| {
                b["recherche"]["champs"][0]["criteres"][0]["typeRecherche"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            types,
            ["EXACTE", "TOUS_LES_MOTS_DANS_UN_CHAMP", "UN_DES_MOTS"]
        );
    }

    #[tokio::test]
    async fn description_en_mots_cles_trouve_avec_tous_les_mots() {
        let (client, _) = demarrer(false).await;
        let v = search_codes(&client, &req("fait des choses sous sa garde", "ALL"))
            .await
            .unwrap()
            .unwrap();
        // Pas d'expression exacte (« fait des choses que l'on a sous sa garde »)… « fait choses garde » : tous les mots.
        assert_eq!(v["recherche"]["palier"], "tous_les_mots");
        let r = ids(&v);
        assert!(r.contains(&"LEGIARTI000006436100".to_string()), "{r:?}");
    }

    #[tokio::test]
    async fn phrase_restreinte_a_un_code() {
        let (client, _) = demarrer(false).await;
        let v = search_codes(&client, &req("violence code civil", "ALL"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ids(&v), ["LEGIARTI000032040795"]);
        assert_eq!(v["recherche"]["code"], "Code civil");
        // Le mot « code civil » n'est pas cherché dans le contenu.
        assert_eq!(v["recherche"]["phrase"], "violence");
    }

    #[tokio::test]
    async fn aucun_resultat_donne_une_note_et_pas_une_erreur() {
        let (client, _) = demarrer(false).await;
        let v = search_codes(&client, &req("zzzxyz qqqwww", "ALL"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(v["totalResultNumber"], 0);
        assert!(ids(&v).is_empty());
        assert!(v["recherche"]["note"]
            .as_str()
            .unwrap()
            .contains("Aucun article de code"));
    }

    #[tokio::test]
    async fn page_suivante_rejoue_le_palier_demande() {
        let (client, st) = demarrer(false).await;
        let mut r = req(
            "responsabilité du fait des choses que l'on a sous sa garde",
            "ALL",
        );
        r.page = 2;
        r.palier = Some("un_des_mots");
        search_codes(&client, &r).await.unwrap().unwrap();
        let recus = st.recus.lock().unwrap();
        assert_eq!(recus.len(), 1, "un seul appel : le palier est imposé");
        assert_eq!(recus[0]["recherche"]["pageNumber"], 2);
        assert_eq!(
            recus[0]["recherche"]["champs"][0]["criteres"][0]["typeRecherche"],
            "UN_DES_MOTS"
        );
    }

    #[tokio::test]
    async fn saisies_non_concernees_laissent_le_chemin_historique() {
        let (client, st) = demarrer(false).await;
        for (q, champ) in [("code civil", "ALL"), ("code civil", "TITLE"), ("", "ALL")] {
            assert!(search_codes(&client, &req(q, champ))
                .await
                .unwrap()
                .is_none());
        }
        // Fond non code : jamais concerné.
        let mut r = req("article 1140", "ALL");
        r.fond = SearchFond::LodaEtat;
        assert!(search_codes(&client, &r).await.unwrap().is_none());
        assert!(st.recus.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn filtre_date_utilise_code_date_sans_facette_de_code() {
        let (client, st) = demarrer(false).await;
        let mut r = req("code civil article 1140", "ALL");
        r.fond = SearchFond::CodeDate;
        r.date_start = Some("2020-01-01".into());
        search_codes(&client, &r).await.unwrap();
        let recu = st.recus.lock().unwrap()[0].clone();
        assert_eq!(recu["fond"], "CODE_DATE");
        let facettes: Vec<String> = recu["recherche"]["filtres"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["facette"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(facettes, ["DATE_VERSION"]);
    }

    #[tokio::test]
    async fn droit_applicable_au_cherche_les_versions_en_vigueur_a_la_date() {
        let (client, st) = demarrer(false).await;
        let mut r = req("code civil article 1140", "ALL");
        r.date_applicable = Some("2018-05-12".into());
        // Une période saisie par ailleurs n'est pas combinée à la date unique.
        r.date_start = Some("2000-01-01".into());
        let v = search_codes(&client, &r).await.unwrap().unwrap();
        assert_eq!(ids(&v), ["LEGIARTI000032040795"]);
        assert_eq!(v["recherche"]["date_applicable"], "2018-05-12");
        let recu = st.recus.lock().unwrap()[0].clone();
        assert_eq!(recu["fond"], "CODE_DATE");
        assert_eq!(
            recu["recherche"]["filtres"],
            json!([{ "facette": "DATE_VERSION", "singleDate": "2018-05-12" }])
        );
    }

    #[tokio::test]
    async fn droit_applicable_au_sur_une_phrase_libre() {
        let (client, st) = demarrer(false).await;
        let mut r = req("violence code civil", "ALL");
        r.date_applicable = Some("2018-05-12".into());
        let v = search_codes(&client, &r).await.unwrap().unwrap();
        // Code filtré côté Juridix (pas de facette de code sur CODE_DATE).
        assert_eq!(ids(&v), ["LEGIARTI000032040795"]);
        let recu = st.recus.lock().unwrap()[0].clone();
        assert_eq!(recu["fond"], "CODE_DATE");
        assert_eq!(recu["recherche"]["filtres"][0]["singleDate"], "2018-05-12");
    }
}
