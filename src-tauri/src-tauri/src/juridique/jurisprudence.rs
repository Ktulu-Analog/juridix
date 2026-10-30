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
// Juridix — Module Juridique — jurisprudence.rs
// ============================================================================
// Fiche d'une décision de justice : identité, niveau (juge du fond / Cour de
// cassation) et RAPPROCHEMENTS que les API officielles fournissent, chacun avec sa
// PROVENANCE (base et champ d'où il vient).
//
// Ce que les API donnent réellement (spécifications PISTE) :
//   - Judilibre `GET /decision` : `contested` (décision attaquée, chaînage arrière),
//     `forward` (décision suivante, chaînage avant), `rapprochements` (rapprochements de
//     jurisprudence), `timeline` (dates clés), `visa` (textes appliqués) ;
//   - Légifrance `POST /consult/juri` : `decisionAttaquee` (date et formation seulement),
//     `liens` (liens typés vers d'autres textes ou décisions : `typeLien` et `sens`),
//     `renvoi`.
//
// Ce qu'elles NE donnent PAS, et que la fiche ne prétend donc jamais fournir :
//   - les décisions qui CITENT celle-ci (« cité par ») : absent de Judilibre ; Légifrance
//     restitue des liens typés dont le sens est affiché TEL QUEL, sans l'interpréter ;
//   - les revirements de jurisprudence : aucune des deux API ne les signale.
// Chacun de ces manques est listé dans `indisponibles`, avec sa raison : l'absence d'un
// rapprochement dans une fiche ne prouve jamais qu'il n'existe pas.
//
// « Décision postérieure » : une décision est dite postérieure (ou antérieure) à celle de
// la fiche d'après les dates que la source donne ; la provenance l'indique (« calcul à
// partir des dates »).
// ============================================================================

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{verify::fold, verify_juris::date_of_value};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Niveau de la juridiction : juge du fond ou Cour de cassation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Niveau {
    /// Cour de cassation (contrôle de droit).
    Cassation,
    /// Juge du fond : cours d'appel, tribunaux judiciaires et de commerce, conseils de
    /// prud'hommes, cours et tribunaux administratifs.
    Fond,
    /// Non déterminé d'après les informations de la source (ex. Conseil d'État).
    Inconnu,
}

impl Niveau {
    pub fn libelle(self) -> &'static str {
        match self {
            Niveau::Cassation => "Cour de cassation",
            Niveau::Fond => "Juge du fond",
            Niveau::Inconnu => "Niveau non déterminé",
        }
    }
}

/// Niveau d'une juridiction d'après son intitulé (« Cour d'appel de Paris », « cc », « ca »…).
pub fn niveau_of(text: &str) -> Niveau {
    let f = fold(text);
    let f = f.trim();
    if f.is_empty() {
        return Niveau::Inconnu;
    }
    if f == "cc" || f.contains("cour de cassation") {
        Niveau::Cassation
    } else if matches!(f, "ca" | "tj" | "tcom" | "cph")
        || f.contains("cour d'appel")
        || f.contains("cour administrative d'appel")
        || f.contains("tribunal")
        || f.contains("conseil de prud")
    {
        Niveau::Fond
    } else {
        Niveau::Inconnu
    }
}

/// Nature du rapprochement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Décision attaquée (chaînage arrière) : `contested` de Judilibre.
    DecisionAttaquee,
    /// Décision suivante (chaînage avant) : `forward` de Judilibre.
    DecisionSuivante,
    /// Rapprochement de jurisprudence : `rapprochements` de Judilibre.
    Rapprochement,
    /// Lien typé de Légifrance (`liens`), restitué sans interprétation.
    LienLegifrance,
}

/// Chronologie d'une décision liée par rapport à celle de la fiche, d'après les dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chronologie {
    Anterieure,
    MemeJour,
    Posterieure,
}

/// D'où vient une information : la base et le champ de sa réponse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source: String,
    pub champ: String,
}

fn prov(source: &str, champ: &str) -> Provenance {
    Provenance {
        source: source.to_string(),
        champ: champ.to_string(),
    }
}

/// Ce que vise un lien : une décision, un texte, ou on ne sait pas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cible {
    Decision,
    Texte,
    Inconnue,
}

/// Une décision (ou un texte) liée à celle de la fiche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rapprochement {
    pub relation: Relation,
    /// Intitulé lisible de la relation : « Décision attaquée », « Rapprochement de
    /// jurisprudence »…
    pub libelle: String,
    pub cible: Cible,
    pub id: Option<String>,
    pub titre: Option<String>,
    pub numero: Option<String>,
    /// « AAAA-MM-JJ »
    pub date: Option<String>,
    pub juridiction: Option<String>,
    pub siege: Option<String>,
    pub chambre: Option<String>,
    pub solution: Option<String>,
    pub description: Option<String>,
    pub themes: Vec<String>,
    pub url: Option<String>,
    /// Niveau de la juridiction liée, quand l'intitulé le permet.
    pub niveau: Niveau,
    /// Antérieure / postérieure à la décision de la fiche, d'après les dates.
    pub chronologie: Option<Chronologie>,
    /// Lien Légifrance : `typeLien` et `sens` tels que l'API les donne.
    pub type_lien: Option<String>,
    pub sens: Option<String>,
    pub provenance: Provenance,
}

/// Date clé de la procédure (`timeline` de Judilibre).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateCle {
    pub libelle: String,
    pub date: Option<String>,
    pub provenance: Provenance,
}

/// Texte appliqué par la décision (`visa` de Judilibre).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visa {
    pub titre: String,
    pub url: Option<String>,
}

/// Ce que les API ne fournissent pas (ou n'ont pas renseigné) pour cette décision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Indisponible {
    /// `cite_par`, `revirements`, `decision_attaquee`, `decisions_posterieures`, `liens`.
    pub element: String,
    pub raison: String,
}

/// Fiche d'une décision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fiche {
    pub id: String,
    /// Base de la fiche : « Judilibre » ou « Légifrance ».
    pub source: String,
    pub titre: Option<String>,
    pub juridiction: Option<String>,
    pub niveau: Niveau,
    pub niveau_libelle: String,
    pub chambre: Option<String>,
    pub formation: Option<String>,
    pub numero: Option<String>,
    pub ecli: Option<String>,
    /// « AAAA-MM-JJ »
    pub date: Option<String>,
    pub solution: Option<String>,
    pub publication: Vec<String>,
    /// Qualifiée « d'intérêt particulier » par Judilibre.
    pub interet_particulier: Option<bool>,
    /// Décision publiée par extraits.
    pub partielle: bool,
    pub visas: Vec<Visa>,
    pub rapprochements: Vec<Rapprochement>,
    pub dates_cles: Vec<DateCle>,
    pub indisponibles: Vec<Indisponible>,
}

// ── Aides ─────────────────────────────────────────────────────────────────────

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

fn day(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(date_of_value)
}

fn strings(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Antérieure / postérieure, d'après deux dates « AAAA-MM-JJ ».
pub fn chronologie_of(reference: Option<&str>, other: Option<&str>) -> Option<Chronologie> {
    let (r, o) = (reference?.get(..10)?, other?.get(..10)?);
    Some(match o.cmp(r) {
        std::cmp::Ordering::Less => Chronologie::Anterieure,
        std::cmp::Ordering::Equal => Chronologie::MemeJour,
        std::cmp::Ordering::Greater => Chronologie::Posterieure,
    })
}

/// Un champ Judilibre qui est un objet (ou, par tolérance, une liste d'objets).
fn objects<'a>(v: &'a Value, k: &str) -> Vec<&'a Value> {
    match v.get(k) {
        Some(o @ Value::Object(_)) => vec![o],
        Some(Value::Array(a)) => a.iter().filter(|x| x.is_object()).collect(),
        _ => Vec::new(),
    }
}

// ── Judilibre ─────────────────────────────────────────────────────────────────

fn libelle_of(relation: Relation, niveau: Niveau) -> String {
    match (relation, niveau) {
        (Relation::DecisionAttaquee, Niveau::Fond) => "Décision attaquée (juge du fond)".into(),
        (Relation::DecisionAttaquee, _) => "Décision attaquée".into(),
        (Relation::DecisionSuivante, _) => "Décision suivante (chaînage avant)".into(),
        (Relation::Rapprochement, _) => "Rapprochement de jurisprudence".into(),
        (Relation::LienLegifrance, _) => "Lien Légifrance".into(),
    }
}

/// Rapprochement tiré d'un objet `decisionLink` de Judilibre.
fn link_judilibre(
    l: &Value,
    relation: Relation,
    champ: &str,
    own_date: Option<&str>,
) -> Rapprochement {
    let juridiction = s(l, "jurisdiction");
    let siege = s(l, "location");
    // Le siège (« Cour d'appel de Toulouse ») précise plus que la juridiction seule.
    let niveau = match niveau_of(siege.as_deref().unwrap_or("")) {
        Niveau::Inconnu => niveau_of(juridiction.as_deref().unwrap_or("")),
        n => n,
    };
    let date = day(l, "date");
    Rapprochement {
        relation,
        libelle: libelle_of(relation, niveau),
        cible: Cible::Decision,
        id: s(l, "id"),
        titre: s(l, "title"),
        numero: s(l, "number"),
        chronologie: chronologie_of(own_date, date.as_deref()),
        date,
        juridiction,
        siege,
        chambre: s(l, "chamber"),
        solution: s(l, "solution"),
        description: s(l, "description"),
        themes: strings(l, "theme"),
        url: s(l, "url"),
        niveau,
        type_lien: None,
        sens: None,
        provenance: prov("Judilibre", champ),
    }
}

/// Fiche d'une décision d'après la réponse de Judilibre `GET /decision`.
/// `None` si la réponse n'a pas d'identifiant (forme inattendue).
pub fn fiche_judilibre(v: &Value) -> Option<Fiche> {
    let id = s(v, "id")?;
    let juridiction = s(v, "jurisdiction");
    let niveau = niveau_of(juridiction.as_deref().unwrap_or(""));
    let date = day(v, "decision_date").or_else(|| day(v, "decision_datetime"));
    let own = date.as_deref();

    let mut rapprochements: Vec<Rapprochement> = Vec::new();
    for l in objects(v, "contested") {
        rapprochements.push(link_judilibre(
            l,
            Relation::DecisionAttaquee,
            "contested",
            own,
        ));
    }
    for l in objects(v, "forward") {
        rapprochements.push(link_judilibre(
            l,
            Relation::DecisionSuivante,
            "forward",
            own,
        ));
    }
    for l in objects(v, "rapprochements") {
        rapprochements.push(link_judilibre(
            l,
            Relation::Rapprochement,
            "rapprochements",
            own,
        ));
    }

    let dates_cles: Vec<DateCle> = objects(v, "timeline")
        .into_iter()
        .map(|l| DateCle {
            libelle: s(l, "title").unwrap_or_else(|| "Date clé".to_string()),
            date: day(l, "date"),
            provenance: prov("Judilibre", "timeline"),
        })
        .collect();

    let visas: Vec<Visa> = objects(v, "visa")
        .into_iter()
        .filter_map(|l| {
            Some(Visa {
                titre: s(l, "title")?,
                url: s(l, "url"),
            })
        })
        .collect();

    let mut indisponibles = vec![
        Indisponible {
            element: "cite_par".into(),
            raison: "Judilibre ne fournit pas les décisions qui citent celle-ci.".into(),
        },
        Indisponible {
            element: "revirements".into(),
            raison: "Aucune API officielle (Judilibre, Légifrance) n'indique les revirements de jurisprudence : leur absence ici ne prouve pas qu'il n'y en ait pas.".into(),
        },
    ];
    if niveau == Niveau::Cassation
        && !rapprochements
            .iter()
            .any(|r| r.relation == Relation::DecisionAttaquee)
    {
        indisponibles.push(Indisponible {
            element: "decision_attaquee".into(),
            raison: "Judilibre ne renseigne pas la décision attaquée pour cette décision.".into(),
        });
    }
    if !rapprochements
        .iter()
        .any(|r| r.chronologie == Some(Chronologie::Posterieure))
    {
        indisponibles.push(Indisponible {
            element: "decisions_posterieures".into(),
            raison: "Aucune décision postérieure n'est renseignée par Judilibre ; cela n'exclut pas qu'il en existe.".into(),
        });
    }

    Some(Fiche {
        id,
        source: "Judilibre".into(),
        titre: s(v, "title"),
        juridiction,
        niveau,
        niveau_libelle: niveau.libelle().into(),
        chambre: s(v, "chamber"),
        formation: s(v, "formation"),
        numero: s(v, "number"),
        ecli: s(v, "ecli"),
        date,
        solution: s(v, "solution"),
        publication: strings(v, "publication"),
        interet_particulier: v.get("particularInterest").and_then(Value::as_bool),
        partielle: v.get("partial").and_then(Value::as_bool).unwrap_or(false),
        visas,
        rapprochements,
        dates_cles,
        indisponibles,
    })
}

// ── Légifrance ────────────────────────────────────────────────────────────────

/// Identifiant d'une décision des fonds de jurisprudence de Légifrance.
pub fn is_legifrance_id(id: &str) -> bool {
    ["JURITEXT", "CETATEXT", "CONSTEXT", "JUFITEXT"]
        .iter()
        .any(|p| id.starts_with(p))
}

/// Ce que vise un identifiant Légifrance, et son lien.
fn target_of(id: &str) -> (Cible, Option<String>) {
    let base = "https://www.legifrance.gouv.fr";
    if id.starts_with("JURITEXT") {
        (Cible::Decision, Some(format!("{base}/juri/id/{id}")))
    } else if id.starts_with("CETATEXT") {
        (Cible::Decision, Some(format!("{base}/ceta/id/{id}")))
    } else if id.starts_with("CONSTEXT") {
        (Cible::Decision, Some(format!("{base}/cons/id/{id}")))
    } else if id.starts_with("JORFTEXT") {
        (Cible::Texte, Some(format!("{base}/jorf/id/{id}")))
    } else if id.starts_with("LEGIARTI") {
        (Cible::Texte, Some(format!("{base}/codes/article_lc/{id}")))
    } else if id.starts_with("LEGITEXT") {
        (Cible::Texte, Some(format!("{base}/loda/id/{id}")))
    } else {
        (Cible::Inconnue, None)
    }
}

/// Fiche d'une décision d'après la réponse de Légifrance `POST /consult/juri`.
pub fn fiche_legifrance(id: &str, v: &Value) -> Option<Fiche> {
    let t = v.get("text").unwrap_or(v);
    if !t.is_object() || t.as_object().is_some_and(|o| o.is_empty()) {
        return None;
    }
    let juridiction = s(t, "juridiction").or_else(|| s(t, "natureJuridiction"));
    // La nature de juridiction (« Cour d'appel ») précise la juridiction seule.
    let niveau = match niveau_of(s(t, "natureJuridiction").as_deref().unwrap_or("")) {
        Niveau::Inconnu => niveau_of(juridiction.as_deref().unwrap_or("")),
        n => n,
    };
    let date = day(t, "dateTexte");
    let own = date.as_deref();

    let mut rapprochements: Vec<Rapprochement> = Vec::new();
    // `decisionAttaquee` : la date et la formation seulement, sans identifiant.
    if let Some(a) = t.get("decisionAttaquee").filter(|a| a.is_object()) {
        let adate = day(a, "date");
        let formation = s(a, "formation");
        if adate.is_some() || formation.is_some() {
            rapprochements.push(Rapprochement {
                relation: Relation::DecisionAttaquee,
                libelle: libelle_of(Relation::DecisionAttaquee, Niveau::Inconnu),
                cible: Cible::Decision,
                id: None,
                titre: None,
                numero: None,
                chronologie: chronologie_of(own, adate.as_deref()),
                date: adate,
                juridiction: None,
                siege: None,
                chambre: formation,
                solution: None,
                description: None,
                themes: Vec::new(),
                url: None,
                niveau: Niveau::Inconnu,
                type_lien: None,
                sens: None,
                provenance: prov("Légifrance", "decisionAttaquee"),
            });
        }
    }
    for l in t
        .get("liens")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|l| l.is_object())
    {
        let lid = s(l, "id").or_else(|| s(l, "cidTexte"));
        let (cible, url) = lid
            .as_deref()
            .map(target_of)
            .unwrap_or((Cible::Inconnue, None));
        let ldate = day(l, "dateSignaTexte").or_else(|| day(l, "datePubliTexte"));
        rapprochements.push(Rapprochement {
            relation: Relation::LienLegifrance,
            libelle: libelle_of(Relation::LienLegifrance, Niveau::Inconnu),
            cible,
            id: lid,
            titre: s(l, "title"),
            numero: s(l, "numTexte").or_else(|| s(l, "num")),
            chronologie: chronologie_of(own, ldate.as_deref()),
            date: ldate,
            juridiction: None,
            siege: None,
            chambre: None,
            solution: None,
            description: s(l, "natureTexte"),
            themes: Vec::new(),
            url,
            niveau: Niveau::Inconnu,
            type_lien: s(l, "typeLien"),
            sens: s(l, "sens"),
            provenance: prov("Légifrance", "liens"),
        });
    }

    let mut indisponibles = vec![
        Indisponible {
            element: "cite_par".into(),
            raison: "Légifrance ne fournit pas de liste « cité par » : les liens ci-dessus sont restitués avec leur type et leur sens tels que l'API les donne, sans interprétation.".into(),
        },
        Indisponible {
            element: "revirements".into(),
            raison: "Aucune API officielle (Judilibre, Légifrance) n'indique les revirements de jurisprudence : leur absence ici ne prouve pas qu'il n'y en ait pas.".into(),
        },
    ];
    if !rapprochements
        .iter()
        .any(|r| r.relation == Relation::LienLegifrance)
    {
        indisponibles.push(Indisponible {
            element: "liens".into(),
            raison: "Légifrance ne renvoie aucun lien pour cette décision.".into(),
        });
    }

    let numero = t
        .get("numeroAffaire")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|x| !x.is_empty())
        .or_else(|| s(t, "numeroAffaire"));

    Some(Fiche {
        id: id.to_string(),
        source: "Légifrance".into(),
        titre: s(t, "titre"),
        juridiction,
        niveau,
        niveau_libelle: niveau.libelle().into(),
        chambre: s(t, "formation"),
        formation: None,
        numero,
        ecli: s(t, "ecli"),
        date,
        solution: s(t, "solution"),
        publication: [
            s(t, "typePublicationBulletin"),
            s(t, "numeroPublicationBulletin").map(|n| format!("Bulletin n° {n}")),
        ]
        .into_iter()
        .flatten()
        .collect(),
        interet_particulier: None,
        partielle: false,
        visas: Vec::new(),
        rapprochements,
        dates_cles: Vec::new(),
        indisponibles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn judilibre_cass() -> Value {
        json!({
            "id": "6500abc", "jurisdiction": "Cour de cassation", "chamber": "Chambre sociale",
            "formation": "Formation de section", "number": "22-12.345",
            "ecli": "ECLI:FR:CCASS:2024:SO00123", "decision_date": "2024-03-12",
            "solution": "Cassation", "publication": ["b", "r"], "particularInterest": true,
            "contested": {"id": "6400def", "title": "Arrêt de la cour d'appel de Paris du 5 mai 2022",
                "date": "2022-05-05", "jurisdiction": "Cour d'appel", "location": "Cour d'appel de Paris",
                "number": "19/01234", "url": "https://x/ca"},
            "forward": {"id": "6600ghi", "title": "Arrêt sur renvoi", "date": "2025-01-15",
                "jurisdiction": "Cour d'appel", "location": "Cour d'appel de Lyon", "number": "24/00012"},
            "rapprochements": [
                {"id": "6300aaa", "title": "Soc., 10 mars 2020", "date": "2020-03-10",
                 "jurisdiction": "Cour de cassation", "number": "18-11.111", "theme": ["Licenciement", "Preuve"]},
                {"title": "Soc., 1er juin 2025", "date": "2025-06-01", "jurisdiction": "Cour de cassation"},
                {"title": "Sans date"}
            ],
            "timeline": [{"title": "Rapport", "date": "2024-01-10"}],
            "visa": [{"id": 1, "title": "Article L. 1235-3 du code du travail", "url": "https://x/t"}]
        })
    }

    #[test]
    fn levels_are_read_from_the_court_label() {
        for (t, n) in [
            ("Cour de cassation", Niveau::Cassation),
            ("cc", Niveau::Cassation),
            ("Cour d'appel de Toulouse", Niveau::Fond),
            ("ca", Niveau::Fond),
            ("Tribunal judiciaire de Lyon", Niveau::Fond),
            ("Conseil de prud'hommes de Nancy", Niveau::Fond),
            ("COUR ADMINISTRATIVE D'APPEL DE NANTES", Niveau::Fond),
            ("CONSEIL D'ETAT", Niveau::Inconnu),
            ("", Niveau::Inconnu),
        ] {
            assert_eq!(niveau_of(t), n, "{t}");
        }
    }

    #[test]
    fn a_judilibre_fiche_lists_each_link_with_its_provenance() {
        let f = fiche_judilibre(&judilibre_cass()).unwrap();
        assert_eq!(
            (f.source.as_str(), f.niveau),
            ("Judilibre", Niveau::Cassation)
        );
        assert_eq!(f.niveau_libelle, "Cour de cassation");
        assert_eq!(f.interet_particulier, Some(true));
        assert_eq!(f.publication, ["b", "r"]);
        assert_eq!(f.visas.len(), 1);
        assert_eq!(f.dates_cles[0].libelle, "Rapport");

        let champs: Vec<(&str, Relation)> = f
            .rapprochements
            .iter()
            .map(|r| (r.provenance.champ.as_str(), r.relation))
            .collect();
        assert_eq!(
            champs,
            [
                ("contested", Relation::DecisionAttaquee),
                ("forward", Relation::DecisionSuivante),
                ("rapprochements", Relation::Rapprochement),
                ("rapprochements", Relation::Rapprochement),
                ("rapprochements", Relation::Rapprochement),
            ]
        );
        assert!(f
            .rapprochements
            .iter()
            .all(|r| r.provenance.source == "Judilibre"));
    }

    #[test]
    fn the_contested_decision_is_distinguished_as_a_decision_of_the_merits() {
        let f = fiche_judilibre(&judilibre_cass()).unwrap();
        let a = &f.rapprochements[0];
        assert_eq!(a.niveau, Niveau::Fond);
        assert_eq!(a.libelle, "Décision attaquée (juge du fond)");
        assert_eq!(a.chronologie, Some(Chronologie::Anterieure));
        assert_eq!(a.siege.as_deref(), Some("Cour d'appel de Paris"));
        assert_eq!(a.url.as_deref(), Some("https://x/ca"));
    }

    #[test]
    fn later_decisions_are_those_dated_after_the_fiche() {
        let f = fiche_judilibre(&judilibre_cass()).unwrap();
        let by = |i: usize| f.rapprochements[i].chronologie;
        assert_eq!(by(1), Some(Chronologie::Posterieure)); // forward
        assert_eq!(by(2), Some(Chronologie::Anterieure)); // 2020
        assert_eq!(by(3), Some(Chronologie::Posterieure)); // 2025
        assert_eq!(by(4), None, "sans date : pas de chronologie devinée");
        assert_eq!(f.rapprochements[2].themes, ["Licenciement", "Preuve"]);
        // Des décisions postérieures existent : pas de mention « aucune postérieure ».
        assert!(f
            .indisponibles
            .iter()
            .all(|i| i.element != "decisions_posterieures"));
    }

    #[test]
    fn what_the_api_does_not_provide_is_always_said() {
        let f = fiche_judilibre(&judilibre_cass()).unwrap();
        let e: Vec<&str> = f.indisponibles.iter().map(|i| i.element.as_str()).collect();
        assert!(
            e.contains(&"cite_par") && e.contains(&"revirements"),
            "{e:?}"
        );
        // Décision de cassation sans décision attaquée renseignée : signalé.
        let mut v = judilibre_cass();
        v.as_object_mut().unwrap().remove("contested");
        v.as_object_mut().unwrap().remove("forward");
        v.as_object_mut().unwrap().remove("rapprochements");
        let f = fiche_judilibre(&v).unwrap();
        let e: Vec<&str> = f.indisponibles.iter().map(|i| i.element.as_str()).collect();
        assert!(
            e.contains(&"decision_attaquee") && e.contains(&"decisions_posterieures"),
            "{e:?}"
        );
        assert!(f.rapprochements.is_empty());
    }

    #[test]
    fn an_appeal_decision_is_a_decision_of_the_merits_and_needs_no_contested() {
        let v = json!({"id": "x", "jurisdiction": "ca", "decision_date": "2021-03-12", "number": "19/01234"});
        let f = fiche_judilibre(&v).unwrap();
        assert_eq!(f.niveau, Niveau::Fond);
        assert!(f
            .indisponibles
            .iter()
            .all(|i| i.element != "decision_attaquee"));
    }

    #[test]
    fn an_unexpected_response_gives_no_fiche_and_tolerant_shapes_work() {
        assert!(fiche_judilibre(&json!({})).is_none());
        assert!(fiche_judilibre(&json!({"title": "sans id"})).is_none());
        // `contested` sous forme de liste : toléré.
        let v = json!({"id": "x", "jurisdiction": "cc", "decision_date": "2024-01-01",
            "contested": [{"title": "A", "date": "2023-01-01"}]});
        assert_eq!(fiche_judilibre(&v).unwrap().rapprochements.len(), 1);
    }

    fn legifrance_juri() -> Value {
        json!({"text": {
            "titre": "Cour d'appel de Nancy, 12 mars 2021", "juridiction": "Cour d'appel",
            "natureJuridiction": "Cour d'appel", "formation": "Chambre sociale", "solution": "Confirmation",
            "numeroAffaire": ["19/01234"], "ecli": "ECLI:FR:CANANCY:2021:X1", "dateTexte": 1615503600000i64,
            "decisionAttaquee": {"date": 1546819200000i64, "formation": "Conseil de prud'hommes"},
            "typePublicationBulletin": "Publié",
            "liens": [
                {"id": "JURITEXT000041", "title": "Cour de cassation, 5 mai 2022", "typeLien": "CITATION",
                 "sens": "cible", "numTexte": "21-10.000", "dateSignaTexte": "2022-05-05"},
                {"id": "LEGIARTI000006900", "title": "Code du travail", "typeLien": "VISA", "sens": "source"}
            ]
        }})
    }

    #[test]
    fn a_legifrance_fiche_gives_links_as_typed_by_the_api_without_interpretation() {
        let f = fiche_legifrance("JURITEXT000099", &legifrance_juri()).unwrap();
        assert_eq!((f.source.as_str(), f.niveau), ("Légifrance", Niveau::Fond));
        assert_eq!(f.date.as_deref(), Some("2021-03-12"));
        assert_eq!(f.numero.as_deref(), Some("19/01234"));
        let a = &f.rapprochements[0];
        assert_eq!(a.relation, Relation::DecisionAttaquee);
        assert_eq!(a.date.as_deref(), Some("2019-01-07"));
        assert_eq!(a.chambre.as_deref(), Some("Conseil de prud'hommes"));
        assert_eq!(a.provenance, prov("Légifrance", "decisionAttaquee"));
        let l = &f.rapprochements[1];
        assert_eq!(l.relation, Relation::LienLegifrance);
        // Type et sens restitués tels quels ; jamais « cité par » déduit.
        assert_eq!(
            (l.type_lien.as_deref(), l.sens.as_deref()),
            (Some("CITATION"), Some("cible"))
        );
        assert_eq!(l.cible, Cible::Decision);
        assert_eq!(
            l.url.as_deref(),
            Some("https://www.legifrance.gouv.fr/juri/id/JURITEXT000041")
        );
        assert_eq!(l.chronologie, Some(Chronologie::Posterieure));
        assert_eq!(l.provenance, prov("Légifrance", "liens"));
        let t = &f.rapprochements[2];
        assert_eq!(t.cible, Cible::Texte);
        assert!(f.indisponibles.iter().any(|i| i.element == "cite_par"));
        assert!(f.indisponibles.iter().all(|i| i.element != "liens"));
    }

    #[test]
    fn a_legifrance_decision_without_links_says_so() {
        let v = json!({"text": {"titre": "CAA", "juridiction": "COUR ADMINISTRATIVE D'APPEL DE NANTES",
            "numeroAffaire": ["19NT01234"], "dateTexte": "2021-03-12T00:00:00.000+0000"}});
        let f = fiche_legifrance("CETATEXT000044", &v).unwrap();
        assert_eq!(f.niveau, Niveau::Fond);
        assert!(f.rapprochements.is_empty());
        assert!(f.indisponibles.iter().any(|i| i.element == "liens"));
        assert!(fiche_legifrance("JURITEXT1", &json!({})).is_none());
        assert!(fiche_legifrance("JURITEXT1", &json!({"text": {}})).is_none());
    }

    #[test]
    fn legifrance_ids_are_recognised() {
        for id in [
            "JURITEXT000041",
            "CETATEXT000044",
            "CONSTEXT000017",
            "JUFITEXT0001",
        ] {
            assert!(is_legifrance_id(id));
        }
        assert!(!is_legifrance_id("6500abc1234"));
        assert!(!is_legifrance_id("LEGIARTI000006900"));
    }
}
