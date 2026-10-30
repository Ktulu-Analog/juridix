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
// Juridix — Module Juridique — temporal.rs
// ============================================================================
// Droit dans le temps : « quelle version de cet article s'appliquait à la date
// des faits, et a-t-il changé depuis ? »
//
// Source : `POST /consult/getArticle` renvoie, avec l'article, la liste de ses
// versions (`articleVersions` : id, dateDebut, dateFin, etat, numero — schéma
// `ArticleVersion` du Swagger Légifrance). On en déduit, SANS aucun calcul du
// modèle de langage :
//   - la version applicable à la date (la dernière entrée en vigueur à cette
//     date, si elle n'avait pas déjà pris fin) ;
//   - les versions postérieures (= ce qui a changé depuis).
//
// Principes communs à `verify.rs` : jamais d'erreur présentée comme un verdict
// (`NonVerifiable`), jamais de silence sur ce qui n'a pas pu être établi.
// ============================================================================

use chrono::{DateTime, Duration, NaiveDate};
use serde::Serialize;
use serde_json::Value;
use tracing::warn;

use super::client::PisteClient;
use super::code_query::as_millis;
use super::legifrance;
use super::verify::etat_libelle;
use super::verify_ext::article_text_of;

/// Nombre maximal de versions postérieures détaillées au client.
pub const MAX_MODIFICATIONS: usize = 6;
/// Longueur de l'aperçu du texte applicable (caractères).
const APERCU_LEN: usize = 700;
/// Année à partir de laquelle une `dateFin` signifie « sans fin » (Légifrance
/// utilise 2999-01-01 pour les versions ouvertes).
const ANNEE_SANS_FIN: i32 = 2900;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Une version d'un article.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionInfo {
    pub id: String,
    pub etat: Option<String>,
    pub etat_libelle: Option<String>,
    /// « AAAA-MM-JJ »
    pub date_debut: Option<String>,
    /// « AAAA-MM-JJ » ; `None` = version sans fin connue (en vigueur).
    pub date_fin: Option<String>,
    pub numero: Option<String>,
    #[serde(skip)]
    debut: Option<NaiveDate>,
    #[serde(skip)]
    fin: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatutADate {
    /// Une version de l'article était en vigueur à cette date.
    Applicable,
    /// Aucune : l'article n'existait pas encore, ou n'était plus en vigueur.
    AucuneVersion,
    /// Les versions n'ont pas pu être établies (erreur, réponse inattendue).
    NonVerifiable,
}

/// Résultat pour un article et une date.
#[derive(Debug, Clone, Serialize)]
pub struct DroitADate {
    /// « Code civil, art. 1103 » (celui de la vérification de citations).
    pub reference: String,
    pub code: Option<String>,
    pub numero: String,
    /// Date demandée (« AAAA-MM-JJ »).
    pub date: String,
    pub statut: StatutADate,
    pub version_applicable: Option<VersionInfo>,
    /// La version applicable n'est plus la dernière : l'article a changé depuis.
    pub modifie_depuis: bool,
    /// Versions postérieures à la date, de la plus ancienne à la plus récente.
    pub modifications_depuis: Vec<VersionInfo>,
    /// Début du texte de la version applicable (texte brut).
    pub texte_applicable: Option<String>,
    pub url_applicable: Option<String>,
    /// Explication courte (statuts sans verdict, date future…).
    pub detail: Option<String>,
}

// ── Lecture des versions (pur) ────────────────────────────────────────────────

/// `dateDebut` / `dateFin` → jour calendaire. Légifrance stocke des instants en
/// millisecondes : minuit à Paris tombe la veille à 22 h ou 23 h UTC. On ajoute
/// 12 h avant de prendre la date UTC, ce qui retombe sur le bon jour dans les
/// deux cas (minuit UTC compris).
fn jour(v: &Value) -> Option<NaiveDate> {
    let ms = as_millis(v)?;
    DateTime::from_timestamp_millis(ms).map(|d| (d + Duration::hours(12)).date_naive())
}

fn fin_ouverte(d: NaiveDate) -> Option<NaiveDate> {
    use chrono::Datelike;
    (d.year() < ANNEE_SANS_FIN).then_some(d)
}

fn version_depuis(node: &Value, id_defaut: Option<&str>) -> Option<VersionInfo> {
    let id = node
        .get("id")
        .and_then(Value::as_str)
        .or(id_defaut)?
        .to_string();
    if !id.starts_with("LEGIARTI") {
        return None;
    }
    let debut = node.get("dateDebut").and_then(jour);
    let fin = node.get("dateFin").and_then(jour).and_then(fin_ouverte);
    let etat = node.get("etat").and_then(Value::as_str).map(str::to_string);
    Some(VersionInfo {
        id,
        etat_libelle: etat.as_deref().map(etat_libelle),
        etat,
        date_debut: debut.map(|d| d.format("%Y-%m-%d").to_string()),
        date_fin: fin.map(|d| d.format("%Y-%m-%d").to_string()),
        numero: node
            .get("numero")
            .or_else(|| node.get("num"))
            .and_then(Value::as_str)
            .map(str::to_string),
        debut,
        fin,
    })
}

/// Versions d'un article d'après la réponse de `getArticle`, triées par date de
/// début. À défaut de `articleVersions`, l'article lui-même est l'unique version.
pub fn versions_of(resp: &Value) -> Vec<VersionInfo> {
    let art = resp.get("article").unwrap_or(resp);
    let mut out: Vec<VersionInfo> = art
        .get("articleVersions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|n| version_depuis(n, None)).collect())
        .unwrap_or_default();
    if out.is_empty() {
        if let Some(v) = version_depuis(art, None) {
            out.push(v);
        }
    }
    // Dédoublonnage par id (l'article courant peut figurer dans la liste).
    let mut vus: Vec<String> = Vec::new();
    out.retain(|v| {
        if vus.contains(&v.id) {
            false
        } else {
            vus.push(v.id.clone());
            true
        }
    });
    out.sort_by_key(|v| (v.debut.is_none(), v.debut));
    out
}

/// Version applicable à `date` et versions postérieures.
#[derive(Debug, PartialEq, Eq)]
pub struct Analyse {
    pub applicable: Option<VersionInfo>,
    pub depuis: Vec<VersionInfo>,
}

/// Règle : est applicable la version ENTRÉE EN VIGUEUR LA PLUS RÉCEMMENT au plus
/// tard à `date`, à condition qu'elle n'ait pas déjà pris fin ce jour-là (une fin
/// sans successeur = abrogation : aucune version applicable). Cette règle ne
/// dépend pas de la convention de Légifrance pour `dateFin` (veille du successeur
/// ou même jour).
pub fn analyser(versions: &[VersionInfo], date: NaiveDate) -> Analyse {
    let candidate = versions
        .iter()
        .filter(|v| v.debut.is_some_and(|d| d <= date))
        .max_by_key(|v| v.debut);
    let applicable = candidate
        .filter(|v| v.fin.map_or(true, |f| date <= f))
        .cloned();
    let depuis: Vec<VersionInfo> = versions
        .iter()
        .filter(|v| v.debut.is_some_and(|d| d > date))
        .cloned()
        .collect();
    Analyse { applicable, depuis }
}

fn fr(d: &str) -> String {
    // « 2018-01-01 » → « 01/01/2018 » (affichage dans les détails).
    let p: Vec<&str> = d.split('-').collect();
    if p.len() == 3 {
        format!("{}/{}/{}", p[2], p[1], p[0])
    } else {
        d.to_string()
    }
}

fn article_url(id: &str) -> String {
    format!("https://www.legifrance.gouv.fr/codes/article_lc/{id}")
}

fn tronquer(s: &str, max: usize) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        return t;
    }
    let mut o: String = t.chars().take(max).collect();
    o.push('…');
    o
}

/// Identifiant (LEGIARTI) de la version applicable à `date`, d'après la réponse de `getArticle`.
///
/// `Ok(None)` : l'article n'avait aucune version en vigueur à cette date (pas encore créé ou
/// déjà abrogé). `Err` : les versions n'ont pas pu être lues — jamais un verdict.
pub fn version_applicable_id(resp: &Value, date: NaiveDate) -> Result<Option<String>, String> {
    let versions = versions_of(resp);
    if versions.is_empty() {
        return Err("versions de l'article illisibles".to_string());
    }
    Ok(analyser(&versions, date).applicable.map(|v| v.id))
}

/// Construit le résultat à partir des versions (pur, sans texte).
pub fn construire(
    reference: &str,
    code: Option<&str>,
    numero: &str,
    date: NaiveDate,
    today: NaiveDate,
    versions: &[VersionInfo],
) -> DroitADate {
    let a = analyser(versions, date);
    let date_s = date.format("%Y-%m-%d").to_string();
    let mut modifs = a.depuis.clone();
    // « En vigueur différée » dans le futur n'est pas un changement déjà intervenu :
    // on le garde (signalé par son état) mais on ne le compte pas comme modification.
    let deja_intervenues = modifs
        .iter()
        .filter(|v| v.debut.is_some_and(|d| d <= today))
        .count();
    modifs.truncate(MAX_MODIFICATIONS);

    let mut detail: Option<String> = None;
    let statut = if versions.is_empty() {
        detail = Some("Aucune version de cet article n'a pu être lue sur Légifrance.".into());
        StatutADate::NonVerifiable
    } else if a.applicable.is_some() {
        StatutADate::Applicable
    } else {
        let premiere = versions.iter().find_map(|v| v.debut);
        detail = Some(match premiere {
            Some(p) if date < p => format!(
                "Cet article n'existait pas encore au {} (première version : {}).",
                fr(&date_s),
                fr(&p.format("%Y-%m-%d").to_string())
            ),
            _ => format!(
                "Aucune version de cet article n'était en vigueur au {} (article abrogé avant cette date).",
                fr(&date_s)
            ),
        });
        StatutADate::AucuneVersion
    };
    if date > today && statut == StatutADate::Applicable {
        detail = Some(
            "Date postérieure à aujourd'hui : version applicable selon le droit connu à ce jour."
                .into(),
        );
    }

    DroitADate {
        reference: reference.to_string(),
        code: code.map(str::to_string),
        numero: numero.to_string(),
        date: date_s,
        statut,
        modifie_depuis: deja_intervenues > 0,
        modifications_depuis: modifs,
        url_applicable: a.applicable.as_ref().map(|v| article_url(&v.id)),
        version_applicable: a.applicable,
        texte_applicable: None,
        detail,
    }
}

// ── Appels PISTE ──────────────────────────────────────────────────────────────

/// Droit applicable à `date` pour l'article `article_id` (une version quelconque
/// de l'article : la liste complète des versions est lue depuis celle-ci).
///
/// Ne renvoie jamais d'erreur : une indisponibilité donne `NonVerifiable`.
pub async fn droit_a_date(
    client: &PisteClient,
    article_id: &str,
    reference: &str,
    code: Option<&str>,
    numero: &str,
    date: NaiveDate,
    today: NaiveDate,
) -> DroitADate {
    droit_a_date_texte(
        client, article_id, reference, code, numero, date, today, APERCU_LEN,
    )
    .await
}

/// Comme `droit_a_date`, avec une longueur de texte au choix (`texte_applicable` tronqué à
/// `max_chars` caractères) : l'aperçu d'une info-bulle et le texte intégral donné au modèle
/// n'ont pas les mêmes besoins.
#[allow(clippy::too_many_arguments)]
pub async fn droit_a_date_texte(
    client: &PisteClient,
    article_id: &str,
    reference: &str,
    code: Option<&str>,
    numero: &str,
    date: NaiveDate,
    today: NaiveDate,
    max_chars: usize,
) -> DroitADate {
    let resp = match legifrance::get_article(client, article_id).await {
        Ok(v) => v,
        Err(e) => {
            warn!("[Droit à date] {reference} : {e}");
            let mut r = construire(reference, code, numero, date, today, &[]);
            r.detail = Some("Légifrance n'a pas répondu : version non vérifiable.".into());
            return r;
        }
    };
    let versions = versions_of(&resp);
    let mut r = construire(reference, code, numero, date, today, &versions);
    if let Some(v) = &r.version_applicable {
        // Texte de la version applicable : déjà dans la réponse si c'est celle qu'on a lue.
        let texte = if v.id == article_id {
            article_text_of(&resp)
        } else {
            match legifrance::get_article(client, &v.id).await {
                Ok(x) => article_text_of(&x),
                Err(e) => {
                    warn!("[Droit à date] texte de {} : {e}", v.id);
                    None
                }
            }
        };
        r.texte_applicable = texte.map(|t| tronquer(&t, max_chars));
    }
    r
}

/// Analyse une date « AAAA-MM-JJ » saisie par l'utilisateur.
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

// ── Rédaction applicable fournie au modèle (chat) ─────────────────────────────

/// Articles dont la rédaction à la date est fournie au modèle (un appel Légifrance chacun).
pub const MAX_ARTICLES_CONTEXTE: usize = 4;
/// Longueur maximale du texte fourni par article (caractères).
const CONTEXTE_TEXTE_LEN: usize = 6000;

/// Bloc de prompt à partir des résultats (pur). `None` si rien d'établi à dire.
///
/// Les articles « non vérifiables » sont omis : on ne dit jamais au modèle ce qu'on n'a pas lu.
pub fn bloc_contexte(date: NaiveDate, items: &[DroitADate]) -> Option<String> {
    let date_iso = date.format("%Y-%m-%d").to_string();
    let date_fr = fr(&date_iso);
    let mut parts: Vec<String> = Vec::new();
    for d in items {
        match d.statut {
            StatutADate::Applicable => {
                let Some(v) = &d.version_applicable else {
                    continue;
                };
                let Some(texte) = d.texte_applicable.as_deref() else {
                    continue;
                };
                let mut en_tete =
                    format!("### {} — rédaction en vigueur le {date_fr}", d.reference);
                match (&v.date_debut, &v.date_fin) {
                    (Some(a), Some(b)) => {
                        en_tete.push_str(&format!(" (du {} au {})", fr(a), fr(b)))
                    }
                    (Some(a), None) => en_tete.push_str(&format!(" (depuis le {})", fr(a))),
                    _ => {}
                }
                let mut bloc = format!("{en_tete}\n« {texte} »");
                if d.modifie_depuis {
                    let depuis: Vec<String> = d
                        .modifications_depuis
                        .iter()
                        .filter_map(|m| m.date_debut.as_deref().map(fr))
                        .collect();
                    bloc.push_str(&format!(
                        "\nCet article a été modifié depuis ({}) : la rédaction actuelle diffère.",
                        depuis.join(", ")
                    ));
                }
                parts.push(bloc);
            }
            StatutADate::AucuneVersion => {
                parts.push(format!(
                    "### {} — aucune rédaction applicable le {date_fr}\n{}",
                    d.reference,
                    d.detail
                        .as_deref()
                        .unwrap_or("Article non en vigueur à cette date.")
                ));
            }
            StatutADate::NonVerifiable => {}
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "\n\n## Textes applicables à la date des faits ({date_fr})\n\
Juridix a lu sur Légifrance la rédaction EN VIGUEUR LE {date_fr} des articles ci-dessous, cités par l'utilisateur. \
Pour ces articles, ce texte fait foi et PRIME sur tout texte renvoyé par un outil : un outil Légifrance renvoie en général la version actuelle. \
Recopie ce texte à l'identique et indique sa date d'entrée en vigueur. Si un outil te renvoie une rédaction différente, \
c'est la rédaction actuelle : présente-la comme telle (« rédaction actuelle »), jamais comme celle applicable aux faits.\n\n{}\n",
        parts.join("\n\n")
    ))
}

/// Rédaction à `date` des articles de code cités dans `textes` (du plus récent au plus ancien),
/// sous forme de bloc de prompt. Au plus `MAX_ARTICLES_CONTEXTE` articles distincts ;
/// `None` si aucun article de code n'est cité ou si rien n'a pu être lu.
pub async fn contexte_a_date(
    lookup: &dyn super::verify::ArticleLookup,
    client: &PisteClient,
    textes: &[&str],
    date: NaiveDate,
    today: NaiveDate,
) -> Option<String> {
    use super::verify::{extract_article_refs, Lookup};
    let mut vus: Vec<String> = Vec::new();
    let mut resultats: Vec<DroitADate> = Vec::new();
    'tout: for texte in textes {
        for r in extract_article_refs(texte) {
            let Some(code) = r.code else {
                continue;
            };
            let cle = format!("{code}|{}", r.numero_key());
            if vus.contains(&cle) {
                continue;
            }
            if vus.len() >= MAX_ARTICLES_CONTEXTE {
                break 'tout;
            }
            vus.push(cle);
            let numero = r.numero_query();
            let id = match lookup.lookup(code, &numero).await {
                Ok(Lookup::EnVigueur { id }) | Ok(Lookup::Autre { id, .. }) => id,
                _ => continue,
            };
            let reference = format!("{code}, art. {}", r.numero_display());
            resultats.push(
                droit_a_date_texte(
                    client,
                    &id,
                    &reference,
                    Some(code),
                    &numero,
                    date,
                    today,
                    CONTEXTE_TEXTE_LEN,
                )
                .await,
            );
        }
    }
    bloc_contexte(date, &resultats)
}

// ── Instruction donnée au modèle (chat) ───────────────────────────────────────

/// Bloc ajouté au prompt système quand l'utilisateur a fixé une date des faits.
///
/// Honnête sur ses limites : le modèle ne peut pas garantir la version d'un texte
/// qu'il n'a pas consultée. La vérification (`droit_a_date`) contrôle ensuite, par
/// le code, la version applicable de chaque article cité.
pub fn instruction_chat(date_iso: &str) -> Option<String> {
    let d = parse_date(date_iso)?;
    let iso = d.format("%Y-%m-%d").to_string();
    let fr = fr(&iso);
    Some(format!(
        "\n\n## Droit applicable à une date donnée\n\
L'utilisateur raisonne sur des faits datés du {fr} ({iso}). Applique le droit TEL QU'IL ÉTAIT EN VIGUEUR à cette date, et non sa version actuelle.\n\
- Pour chaque article ou texte, cite la rédaction applicable le {fr}. Si le texte a été modifié ou abrogé depuis, ou n'existait pas encore à cette date, dis-le explicitement.\n\
- Quand tu interroges Légifrance avec un outil, demande la version en vigueur au {iso} chaque fois que l'outil le permet. Si l'outil ne renvoie que la version actuelle, présente-la comme « rédaction actuelle » : elle peut différer de celle applicable aux faits. Cette consigne prime sur la reproduction des textes d'outils : ne fais jamais passer la version actuelle pour celle de la date des faits.\n\
- N'écris jamais de mémoire une ancienne rédaction : si tu n'as pas obtenu la version applicable à cette date, indique que tu ne l'as pas vérifiée.\n\
- Pour la jurisprudence, tiens compte de la date de la décision par rapport aux faits (revirements, décisions postérieures).\n"
    ))
}

/// Bloc ajouté au message système du chat quand une date des faits est fixée : la consigne générale,
/// puis la rédaction applicable à cette date des articles de code cités dans les derniers messages
/// de l'utilisateur (`messages_utilisateur`, du plus récent au plus ancien).
///
/// Chaîne vide si la date est absente ou invalide. Sans client PISTE ou en cas d'échec de lecture,
/// seule la consigne générale est fournie : le modèle n'est jamais renseigné sur ce qui n'a pas été lu.
pub async fn bloc_chat(
    state: &super::routes::JuridiqueState,
    messages_utilisateur: &[&str],
    date_iso: Option<&str>,
) -> String {
    let Some(date) = date_iso.and_then(parse_date) else {
        return String::new();
    };
    let iso = date.format("%Y-%m-%d").to_string();
    let Some(mut bloc) = instruction_chat(&iso) else {
        return String::new();
    };
    let client = state.piste_client.read().await.clone();
    if let Some(client) = client {
        let lookup = super::verify::LegifranceLookup::new(client.clone());
        let today = chrono::Local::now().date_naive();
        if let Some(ctx) =
            contexte_a_date(&lookup, &client, messages_utilisateur, date, today).await
        {
            bloc.push_str(&ctx);
        }
    }
    bloc
}

/// Rappel de la date des faits ajouté au DERNIER message de l'utilisateur dans la charge envoyée
/// au modèle (jamais affiché ni enregistré). Une consigne placée dans le message système pèse
/// moins que la question elle-même : sans ce rappel, un modèle qui vient de recevoir d'un outil
/// la version actuelle d'un texte la recopie sans tenir compte de la date.
///
/// `false` si la date est invalide ou s'il n'y a aucun message utilisateur exploitable.
pub fn rappel_dernier_message(payload: &mut [Value], date_iso: &str) -> bool {
    let Some(d) = parse_date(date_iso) else {
        return false;
    };
    let rappel = format!(
        "[Rappel Juridix — date des faits : {}. Réponds d'après le droit en vigueur à cette date, \
en t'appuyant sur les textes fournis dans tes instructions. Un outil qui ne renvoie que la version \
actuelle ne répond pas à cette question : présente alors sa réponse comme « rédaction actuelle ».]",
        fr(&d.format("%Y-%m-%d").to_string())
    );
    let Some(msg) = payload
        .iter_mut()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
    else {
        return false;
    };
    match msg.get_mut("content") {
        Some(Value::String(s)) => {
            s.push_str("\n\n");
            s.push_str(&rappel);
            true
        }
        Some(Value::Array(parts)) => {
            parts.push(serde_json::json!({ "type": "text", "text": rappel }));
            true
        }
        _ => false,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, routing::post, Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn d(s: &str) -> NaiveDate {
        parse_date(s).unwrap()
    }

    /// Instant « minuit à Paris » (heure d'été : veille 22 h UTC) en millisecondes.
    fn ms_paris(jour: &str, ete: bool) -> i64 {
        let base = d(jour)
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        base - if ete { 2 } else { 1 } * 3_600_000
    }

    fn article_json() -> Value {
        // 3 versions : 2016-10-01 → 2020-02-29, 2020-03-01 → 2023-06-30, 2023-07-01 → sans fin.
        json!({ "article": {
            "id": "LEGIARTI0003", "num": "1140", "etat": "VIGUEUR",
            "texte": "Version 3 du texte.",
            "articleVersions": [
                { "id": "LEGIARTI0002", "etat": "MODIFIE", "numero": "1140",
                  "dateDebut": ms_paris("2020-03-01", false), "dateFin": ms_paris("2023-06-30", true) },
                { "id": "LEGIARTI0001", "etat": "MODIFIE", "numero": "1140",
                  "dateDebut": ms_paris("2016-10-01", true), "dateFin": ms_paris("2020-02-29", false) },
                { "id": "LEGIARTI0003", "etat": "VIGUEUR", "numero": "1140",
                  "dateDebut": ms_paris("2023-07-01", true), "dateFin": 32472144000000i64 }
            ]
        }})
    }

    #[test]
    fn minuit_a_paris_retombe_sur_le_bon_jour() {
        let v = versions_of(&article_json());
        let ds: Vec<_> = v.iter().map(|x| x.date_debut.clone().unwrap()).collect();
        assert_eq!(ds, ["2016-10-01", "2020-03-01", "2023-07-01"]);
        // Version sans fin : 2999-01-01 n'est pas une vraie date de fin.
        assert_eq!(v[2].date_fin, None);
        assert_eq!(v[0].date_fin.as_deref(), Some("2020-02-29"));
        assert_eq!(v[1].date_fin.as_deref(), Some("2023-06-30"));
    }

    #[test]
    fn version_applicable_selon_la_date() {
        let v = versions_of(&article_json());
        let id = |s: &str| analyser(&v, d(s)).applicable.map(|x| x.id);
        assert_eq!(id("2016-10-01").as_deref(), Some("LEGIARTI0001")); // jour d'entrée en vigueur
        assert_eq!(id("2018-05-12").as_deref(), Some("LEGIARTI0001"));
        assert_eq!(id("2020-02-29").as_deref(), Some("LEGIARTI0001")); // dernier jour
        assert_eq!(id("2020-03-01").as_deref(), Some("LEGIARTI0002")); // lendemain
        assert_eq!(id("2023-06-30").as_deref(), Some("LEGIARTI0002"));
        assert_eq!(id("2023-07-01").as_deref(), Some("LEGIARTI0003"));
        assert_eq!(id("2026-10-26").as_deref(), Some("LEGIARTI0003"));
        assert_eq!(id("2016-09-30"), None); // avant la première version
    }

    #[test]
    fn convention_fin_egale_debut_du_successeur() {
        // Certaines données donnent dateFin = dateDebut du successeur : le successeur l'emporte ce jour-là.
        let json = json!({ "article": { "id": "LEGIARTI0002", "articleVersions": [
            { "id": "LEGIARTI0001", "dateDebut": ms_paris("2018-01-01", false), "dateFin": ms_paris("2020-01-01", false) },
            { "id": "LEGIARTI0002", "dateDebut": ms_paris("2020-01-01", false), "dateFin": 32472144000000i64 }
        ]}});
        let v = versions_of(&json);
        assert_eq!(
            analyser(&v, d("2020-01-01")).applicable.unwrap().id,
            "LEGIARTI0002"
        );
        assert_eq!(
            analyser(&v, d("2019-12-31")).applicable.unwrap().id,
            "LEGIARTI0001"
        );
    }

    #[test]
    fn id_de_la_version_applicable() {
        let r = article_json();
        assert_eq!(
            version_applicable_id(&r, d("2018-05-12")),
            Ok(Some("LEGIARTI0001".into()))
        );
        assert_eq!(
            version_applicable_id(&r, d("2025-01-01")),
            Ok(Some("LEGIARTI0003".into()))
        );
        assert_eq!(version_applicable_id(&r, d("2010-01-01")), Ok(None));
        assert!(version_applicable_id(&json!({}), d("2018-05-12")).is_err());
    }

    #[test]
    fn article_abroge_sans_successeur() {
        let json = json!({ "article": { "id": "LEGIARTI0009", "etat": "ABROGE", "articleVersions": [
            { "id": "LEGIARTI0009", "etat": "ABROGE", "dateDebut": ms_paris("2000-01-01", false),
              "dateFin": ms_paris("2016-09-30", true) }
        ]}});
        let v = versions_of(&json);
        assert!(analyser(&v, d("2010-01-01")).applicable.is_some());
        let r = construire(
            "Code X, art. 9",
            Some("Code X"),
            "9",
            d("2020-01-01"),
            d("2026-10-26"),
            &v,
        );
        assert_eq!(r.statut, StatutADate::AucuneVersion);
        assert!(r.detail.unwrap().contains("abrogé avant cette date"));
    }

    #[test]
    fn article_pas_encore_cree() {
        let v = versions_of(&article_json());
        let r = construire(
            "Code civil, art. 1140",
            Some("Code civil"),
            "1140",
            d("2010-01-01"),
            d("2026-10-26"),
            &v,
        );
        assert_eq!(r.statut, StatutADate::AucuneVersion);
        assert!(r
            .detail
            .unwrap()
            .contains("n'existait pas encore au 01/01/2010"));
        assert!(r.version_applicable.is_none());
    }

    #[test]
    fn modifications_depuis_la_date() {
        let v = versions_of(&article_json());
        let r = construire(
            "Code civil, art. 1140",
            Some("Code civil"),
            "1140",
            d("2018-05-12"),
            d("2026-10-26"),
            &v,
        );
        assert_eq!(r.statut, StatutADate::Applicable);
        assert!(r.modifie_depuis);
        let ids: Vec<_> = r
            .modifications_depuis
            .iter()
            .map(|x| x.id.as_str())
            .collect();
        assert_eq!(ids, ["LEGIARTI0002", "LEGIARTI0003"]);
        assert_eq!(
            r.url_applicable.as_deref(),
            Some("https://www.legifrance.gouv.fr/codes/article_lc/LEGIARTI0001")
        );

        // Version actuelle appliquée à une date récente : rien n'a changé depuis.
        let r = construire("x", None, "1140", d("2024-01-01"), d("2026-10-26"), &v);
        assert!(!r.modifie_depuis && r.modifications_depuis.is_empty());
    }

    #[test]
    fn entree_en_vigueur_differee_n_est_pas_une_modification_deja_intervenue() {
        let json = json!({ "article": { "id": "LEGIARTI0001", "articleVersions": [
            { "id": "LEGIARTI0001", "etat": "VIGUEUR", "dateDebut": ms_paris("2020-01-01", false), "dateFin": ms_paris("2027-12-31", false) },
            { "id": "LEGIARTI0002", "etat": "VIGUEUR_DIFF", "dateDebut": ms_paris("2028-01-01", false), "dateFin": 32472144000000i64 }
        ]}});
        let v = versions_of(&json);
        let r = construire("x", None, "1", d("2024-01-01"), d("2026-10-26"), &v);
        assert!(
            !r.modifie_depuis,
            "la version future n'a pas encore modifié le droit"
        );
        assert_eq!(r.modifications_depuis.len(), 1);
        assert_eq!(
            r.modifications_depuis[0].etat_libelle.as_deref(),
            Some("entrée en vigueur différée")
        );
    }

    #[test]
    fn instruction_chat_contient_la_date_et_refuse_une_date_invalide() {
        let t = instruction_chat("2018-05-12").unwrap();
        assert!(t.contains("12/05/2018") && t.contains("2018-05-12"));
        assert!(t.contains("jamais de mémoire"));
        assert!(instruction_chat("hier").is_none());
        assert!(instruction_chat("").is_none());
    }

    #[test]
    fn date_future_est_signalee() {
        let v = versions_of(&article_json());
        let r = construire("x", None, "1140", d("2030-01-01"), d("2026-10-26"), &v);
        assert_eq!(r.statut, StatutADate::Applicable);
        assert!(r.detail.unwrap().contains("postérieure à aujourd'hui"));
    }

    #[test]
    fn sans_liste_de_versions_l_article_est_l_unique_version() {
        let json = json!({ "article": { "id": "LEGIARTI0001", "etat": "VIGUEUR",
            "dateDebut": ms_paris("2020-01-01", false), "dateFin": 32472144000000i64 }});
        let v = versions_of(&json);
        assert_eq!(v.len(), 1);
        assert_eq!(
            analyser(&v, d("2021-01-01")).applicable.unwrap().id,
            "LEGIARTI0001"
        );
        // Réponse inexploitable : jamais de verdict.
        let r = construire(
            "x",
            None,
            "1",
            d("2021-01-01"),
            d("2026-10-26"),
            &versions_of(&json!({})),
        );
        assert_eq!(r.statut, StatutADate::NonVerifiable);
    }

    #[test]
    fn dates_en_chaines_iso_acceptees() {
        let json = json!({ "article": { "id": "LEGIARTI0001", "articleVersions": [
            { "id": "LEGIARTI0001", "dateDebut": "2020-01-01T00:00:00.000+0000", "dateFin": "2999-01-01T00:00:00.000+0000" }
        ]}});
        let v = versions_of(&json);
        assert_eq!(v[0].date_debut.as_deref(), Some("2020-01-01"));
        assert_eq!(v[0].date_fin, None);
    }

    // ── Appels PISTE (faux serveur) ──────────────────────────────────────────

    #[derive(Clone)]
    struct Mock {
        appels: Arc<Mutex<Vec<String>>>,
        casse: bool,
    }

    async fn get_article(State(st): State<Mock>, Json(b): Json<Value>) -> axum::response::Response {
        use axum::response::IntoResponse;
        let id = b["id"].as_str().unwrap_or("").to_string();
        st.appels.lock().unwrap().push(id.clone());
        if st.casse {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response();
        }
        let mut a = article_json();
        let texte = match id.as_str() {
            "LEGIARTI0001" => "Les effets de l'obligation de donner ou de livrer un immeuble sont réglés au titre De la vente.",
            "LEGIARTI0002" => "Texte de 2020.",
            _ => "Texte actuel.",
        };
        a["article"]["texte"] = json!(texte);
        a["article"]["id"] = json!(id);
        Json(a).into_response()
    }

    async fn demarrer(casse: bool) -> (PisteClient, Mock) {
        let st = Mock {
            appels: Arc::new(Mutex::new(Vec::new())),
            casse,
        };
        let app = Router::new()
            .route(
                "/oauth/token",
                post(|| async { Json(json!({"access_token": "t", "expires_in": 3600})) }),
            )
            .route("/legifrance/consult/getArticle", post(get_article))
            .with_state(st.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (PisteClient::new_for_test(&format!("http://{addr}")), st)
    }

    #[tokio::test]
    async fn droit_a_date_lit_les_versions_et_le_texte_applicable() {
        let (client, st) = demarrer(false).await;
        let r = droit_a_date(
            &client,
            "LEGIARTI0003",
            "Code civil, art. 1140",
            Some("Code civil"),
            "1140",
            d("2018-05-12"),
            d("2026-10-26"),
        )
        .await;
        assert_eq!(r.statut, StatutADate::Applicable);
        assert_eq!(r.version_applicable.as_ref().unwrap().id, "LEGIARTI0001");
        assert!(r.modifie_depuis);
        assert_eq!(r.texte_applicable.as_deref(), Some("Les effets de l'obligation de donner ou de livrer un immeuble sont réglés au titre De la vente."));
        // 2 appels : la liste des versions, puis le texte de la version applicable.
        assert_eq!(
            st.appels.lock().unwrap().as_slice(),
            ["LEGIARTI0003", "LEGIARTI0001"]
        );
    }

    #[tokio::test]
    async fn version_courante_applicable_ne_coute_qu_un_appel() {
        let (client, st) = demarrer(false).await;
        let r = droit_a_date(
            &client,
            "LEGIARTI0003",
            "x",
            None,
            "1140",
            d("2025-01-01"),
            d("2026-10-26"),
        )
        .await;
        assert_eq!(r.version_applicable.as_ref().unwrap().id, "LEGIARTI0003");
        assert_eq!(r.texte_applicable.as_deref(), Some("Texte actuel."));
        assert_eq!(st.appels.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn erreur_api_donne_non_verifiable_jamais_un_verdict() {
        let (client, _) = demarrer(true).await;
        let r = droit_a_date(
            &client,
            "LEGIARTI0003",
            "x",
            None,
            "1140",
            d("2018-05-12"),
            d("2026-10-26"),
        )
        .await;
        assert_eq!(r.statut, StatutADate::NonVerifiable);
        assert!(r.version_applicable.is_none() && !r.modifie_depuis);
        assert!(r.detail.unwrap().contains("non vérifiable"));
    }

    // ── Route /verifier-citations de bout en bout ────────────────────────────

    use crate::juridique::routes::{verifier_citations_route, HasJuridiqueState, JuridiqueState};

    struct TestState {
        js: JuridiqueState,
    }
    impl HasJuridiqueState for TestState {
        fn juridique_state(&self) -> &JuridiqueState {
            &self.js
        }
    }

    async fn search(Json(_): Json<Value>) -> Json<Value> {
        Json(json!({ "totalResultNumber": 1, "results": [{
            "titles": [{ "id": "LEGITEXT000006070721", "cid": "LEGITEXT000006070721", "title": "Code civil" }],
            "sections": [{ "extracts": [
                { "id": "LEGIARTI0003", "num": "1140", "legalStatus": "VIGUEUR" }
            ]}]
        }]}))
    }

    async fn api(casse: bool) -> (String, Mock) {
        let st = Mock {
            appels: Arc::new(Mutex::new(Vec::new())),
            casse: false,
        };
        let app: Router = Router::new()
            .route(
                "/oauth/token",
                post(|| async { Json(json!({"access_token": "t", "expires_in": 3600})) }),
            )
            .route("/legifrance/search", post(search))
            .route("/legifrance/consult/getArticle", post(get_article))
            .with_state(st.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let pa = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let _ = casse;

        let js = JuridiqueState::new(None, None);
        *js.piste_client.write().await = Some(PisteClient::new_for_test(&format!("http://{pa}")));
        let front: Router = Router::new()
            .route(
                "/verifier-citations",
                verifier_citations_route::<TestState>(),
            )
            .with_state(Arc::new(TestState { js }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fa = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, front).await.unwrap() });
        (format!("http://{fa}"), st)
    }

    async fn verifier(base: &str, body: Value) -> Value {
        reqwest::Client::new()
            .post(format!("{base}/verifier-citations"))
            .json(&body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn route_ajoute_droit_a_date_seulement_si_la_date_est_fournie() {
        let (base, _) = api(false).await;
        let texte = "L'article 1140 du Code civil définit la violence.";

        // Sans date : réponse inchangée, aucune clé supplémentaire.
        let sans = verifier(&base, json!({ "texte": texte })).await;
        assert_eq!(sans["verifications"][0]["statut"], "en_vigueur", "{sans:#}");
        assert!(sans.get("droit_a_date").is_none());

        // Avec date : version applicable + modifications depuis.
        let avec = verifier(
            &base,
            json!({ "texte": texte, "date_applicable": "2018-05-12" }),
        )
        .await;
        let da = avec["droit_a_date"].as_array().expect("droit_a_date");
        assert_eq!(da.len(), 1);
        assert_eq!(da[0]["statut"], "applicable");
        assert_eq!(da[0]["version_applicable"]["id"], "LEGIARTI0001");
        assert_eq!(da[0]["version_applicable"]["date_debut"], "2016-10-01");
        assert_eq!(da[0]["modifie_depuis"], true);
        assert_eq!(da[0]["modifications_depuis"].as_array().unwrap().len(), 2);
        assert_eq!(da[0]["texte_applicable"], "Les effets de l'obligation de donner ou de livrer un immeuble sont réglés au titre De la vente.");
        assert!(
            da[0].get("debut").is_none(),
            "champs internes jamais exposés"
        );

        // Les citations textuelles sont comparées à la rédaction applicable à la date, pas au texte actuel.
        let q = "L'article 1140 du Code civil disait : « Les effets de l'obligation de donner ou de livrer un immeuble sont réglés au titre De la vente ».";
        let a_date = verifier(
            &base,
            json!({ "texte": q, "date_applicable": "2018-05-12" }),
        )
        .await;
        let cit = a_date["verifications"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["kind"] == "citation")
            .unwrap_or_else(|| panic!("citation absente : {a_date:#}"));
        assert_eq!(cit["statut"], "verifiee", "{cit:#}");
        assert!(cit["detail"]
            .as_str()
            .unwrap()
            .contains("applicable au 12/05/2018"));
        // Sans date, la même citation est comparée au texte actuel (« Texte actuel. ») : incohérente.
        let sans_date = verifier(&base, json!({ "texte": q })).await;
        let cit = sans_date["verifications"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["kind"] == "citation");
        assert!(cit.map_or(true, |c| c["statut"] != "verifiee"));

        // Date invalide : ignorée, jamais une erreur.
        let mauvaise = verifier(&base, json!({ "texte": texte, "date_applicable": "hier" })).await;
        assert!(mauvaise.get("droit_a_date").is_none());
        assert_eq!(mauvaise["verifications"][0]["statut"], "en_vigueur");
    }

    // ── Test réel (manuel) ───────────────────────────────────────────────────

    /// Contrôle contre la vraie API PISTE. Ignoré par défaut. À lancer avant la release :
    ///
    /// ```text
    /// PISTE_CLIENT_ID=… PISTE_CLIENT_SECRET=… \
    /// JURIDIX_LIVE_ARTICLE_ID="LEGIARTI000032040795" JURIDIX_LIVE_DATE="2015-06-01" \
    /// cargo test --lib live_piste_droit_a_date -- --ignored --nocapture
    /// ```
    ///
    /// `JURIDIX_LIVE_ARTICLE_ID` : n'importe quelle version d'un article qui a été modifié
    /// (ex. un article du Code civil réécrit par l'ordonnance de 2016) ;
    /// `JURIDIX_LIVE_DATE` : une date antérieure à sa modification.
    #[tokio::test]
    #[ignore = "nécessite des identifiants PISTE et un article réel"]
    async fn live_piste_droit_a_date() {
        let (Ok(id), Ok(secret)) = (
            std::env::var("PISTE_CLIENT_ID"),
            std::env::var("PISTE_CLIENT_SECRET"),
        ) else {
            panic!("PISTE_CLIENT_ID et PISTE_CLIENT_SECRET requis");
        };
        let article = std::env::var("JURIDIX_LIVE_ARTICLE_ID").expect("JURIDIX_LIVE_ARTICLE_ID");
        let date = parse_date(&std::env::var("JURIDIX_LIVE_DATE").expect("JURIDIX_LIVE_DATE"))
            .expect("JURIDIX_LIVE_DATE au format AAAA-MM-JJ");
        let client = PisteClient::new(id, secret).unwrap();
        let today = chrono::Local::now().date_naive();
        let r = droit_a_date(&client, &article, "réf.", None, "?", date, today).await;
        println!("{r:#?}");
        assert_ne!(
            r.statut,
            StatutADate::NonVerifiable,
            "versions illisibles : {:?}",
            r.detail
        );
    }

    // ── Rédaction fournie au modèle ──────────────────────────────────────────

    #[test]
    fn bloc_contexte_donne_la_redaction_a_la_date_et_signale_les_modifications() {
        let v = versions_of(&article_json());
        let mut r = construire(
            "Code civil, art. 1140",
            Some("Code civil"),
            "1140",
            d("2018-05-12"),
            d("2026-10-27"),
            &v,
        );
        r.texte_applicable = Some("Texte de la version 2016.".into());
        let b = bloc_contexte(d("2018-05-12"), &[r]).unwrap();
        assert!(b.contains("Textes applicables à la date des faits (12/05/2018)"));
        assert!(b.contains("### Code civil, art. 1140 — rédaction en vigueur le 12/05/2018 (du 01/10/2016 au 29/02/2020)"), "{b}");
        assert!(b.contains("« Texte de la version 2016. »"));
        assert!(b.contains("modifié depuis (01/03/2020, 01/07/2023)"), "{b}");
        assert!(b.contains("PRIME sur tout texte renvoyé par un outil"));
    }

    #[test]
    fn bloc_contexte_omet_ce_qui_n_a_pas_ete_lu() {
        let v = versions_of(&article_json());
        // Article inexistant à la date : dit explicitement.
        let r = construire(
            "Code civil, art. 1140",
            Some("Code civil"),
            "1140",
            d("2010-01-01"),
            d("2026-10-27"),
            &v,
        );
        let b = bloc_contexte(d("2010-01-01"), &[r]).unwrap();
        assert!(b.contains("aucune rédaction applicable le 01/01/2010"));
        // Non vérifiable, ou version sans texte lisible : jamais de texte inventé.
        let nv = construire("x", None, "1", d("2018-05-12"), d("2026-10-27"), &[]);
        assert!(bloc_contexte(d("2018-05-12"), &[nv]).is_none());
        let sans_texte = construire(
            "Code civil, art. 1140",
            Some("Code civil"),
            "1140",
            d("2018-05-12"),
            d("2026-10-27"),
            &v,
        );
        assert!(bloc_contexte(d("2018-05-12"), &[sans_texte]).is_none());
        assert!(bloc_contexte(d("2018-05-12"), &[]).is_none());
    }

    async fn state_avec_piste() -> super::super::routes::JuridiqueState {
        let st = Mock {
            appels: Arc::new(Mutex::new(Vec::new())),
            casse: false,
        };
        let app: Router = Router::new()
            .route(
                "/oauth/token",
                post(|| async { Json(json!({"access_token": "t", "expires_in": 3600})) }),
            )
            .route("/legifrance/search", post(search))
            .route("/legifrance/consult/getArticle", post(get_article))
            .with_state(st);
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let js = JuridiqueState::new(None, None);
        *js.piste_client.write().await = Some(PisteClient::new_for_test(&format!("http://{addr}")));
        js
    }

    #[tokio::test]
    async fn bloc_chat_fournit_la_redaction_a_la_date_pour_l_article_demande() {
        let js = state_avec_piste().await;
        let b = bloc_chat(
            &js,
            &["donne-moi le texte de l'article 1140 du code civil"],
            Some("2018-05-12"),
        )
        .await;
        // Consigne générale + texte officiel de la version applicable.
        assert!(b.contains("Droit applicable à une date donnée"), "{b}");
        assert!(b.contains("12/05/2018"));
        assert!(
            b.contains("### Code civil, art. 1140 — rédaction en vigueur le 12/05/2018"),
            "{b}"
        );
        assert!(
            b.contains("Les effets de l'obligation de donner ou de livrer un immeuble"),
            "{b}"
        );
        assert!(
            !b.contains("Texte actuel."),
            "le texte actuel ne doit pas être présenté : {b}"
        );
    }

    #[tokio::test]
    async fn bloc_chat_sans_article_cite_ou_sans_date() {
        let js = state_avec_piste().await;
        // Pas d'article cité : consigne générale seule.
        let b = bloc_chat(
            &js,
            &["quel est le délai de prescription ?"],
            Some("2018-05-12"),
        )
        .await;
        assert!(
            b.contains("Droit applicable à une date donnée") && !b.contains("Textes applicables")
        );
        // Pas de date, ou date invalide : rien.
        assert!(bloc_chat(&js, &["article 1140 du code civil"], None)
            .await
            .is_empty());
        assert!(
            bloc_chat(&js, &["article 1140 du code civil"], Some("hier"))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn bloc_chat_sans_client_piste_donne_la_consigne_seule() {
        let js = JuridiqueState::new(None, None);
        let b = bloc_chat(&js, &["article 1140 du code civil"], Some("2018-05-12")).await;
        assert!(
            b.contains("Droit applicable à une date donnée") && !b.contains("Textes applicables")
        );
    }

    #[tokio::test]
    async fn contexte_limite_le_nombre_d_articles_distincts() {
        let js = state_avec_piste().await;
        let client = js.piste_client.read().await.clone().unwrap();
        let lookup = super::super::verify::LegifranceLookup::new(client.clone());
        // 6 articles distincts + doublons : au plus MAX_ARTICLES_CONTEXTE interrogés.
        let t = "articles 1140, 1141, 1142, 1143, 1144, 1145 et 1140 du code civil";
        let ctx = contexte_a_date(&lookup, &client, &[t], d("2018-05-12"), d("2026-10-27")).await;
        let n = ctx.map_or(0, |c| c.matches("### Code civil").count());
        assert!(n <= MAX_ARTICLES_CONTEXTE, "{n}");
        assert!(n >= 1);
    }

    /// Le handler de chat (axum) exige des futures `Send` : garde-fou de compilation, car `api.rs`
    /// n'est pas compilé par les tests de ce module.
    #[test]
    fn bloc_chat_est_send() {
        fn assert_send<T: Send>(_: &T) {}
        let js = JuridiqueState::new(None, None);
        let f = bloc_chat(&js, &["article 1140 du code civil"], Some("2018-05-12"));
        assert_send(&f);
    }

    #[test]
    fn rappel_ajoute_au_dernier_message_utilisateur_seulement() {
        let mut p = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "première question"}),
            json!({"role": "assistant", "content": "réponse"}),
            json!({"role": "user", "content": "Quel est le texte de l'article 1140 ?"}),
        ];
        assert!(rappel_dernier_message(&mut p, "1981-10-27"));
        let dernier = p[3]["content"].as_str().unwrap();
        assert!(dernier.starts_with("Quel est le texte de l'article 1140 ?"));
        assert!(dernier.contains("date des faits : 27/10/1981"), "{dernier}");
        // Les autres messages ne sont pas touchés.
        assert_eq!(p[1]["content"], "première question");
        assert_eq!(p[0]["content"], "sys");
    }

    #[test]
    fn rappel_gere_les_contenus_en_parties_et_les_cas_vides() {
        let mut p =
            vec![json!({"role": "user", "content": [{"type": "text", "text": "Question"}]})];
        assert!(rappel_dernier_message(&mut p, "2018-05-12"));
        let parts = p[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert!(parts[1]["text"].as_str().unwrap().contains("12/05/2018"));
        // Date invalide, aucun message utilisateur : rien n'est modifié.
        let mut q = vec![json!({"role": "user", "content": "x"})];
        assert!(!rappel_dernier_message(&mut q, "hier"));
        assert_eq!(q[0]["content"], "x");
        let mut r = vec![json!({"role": "assistant", "content": "x"})];
        assert!(!rappel_dernier_message(&mut r, "2018-05-12"));
    }
}
