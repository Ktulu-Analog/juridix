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
// Juridix — Pré-filtre local « Winnow » (serveur Ollaya)
// ============================================================================
// Insère, entre l'extraction des textes (API PISTE : Légifrance / JUDILIBRE)
// et le LLM, un modèle de DÉCISION servi localement par Ollaya
// (https://ollaya.dev/library/winnow). Un modèle de décision ne génère pas de
// texte : il répond à des questions typées (choix, score) avec des
// probabilités calibrées, en une passe. On l'utilise ici comme trieur :
//
//   texte extrait → segmentation en passages → Winnow (pertinence, rôle)
//                 → politique prudente → texte allégé (+ marqueurs) → LLM
//
// Trois garde-fous non négociables, propres au domaine juridique :
//   1. ÉCHEC OUVERT   — toute panne (serveur, modèle, délai) laisse passer le
//                        texte intégral ; jamais d'erreur remontée à l'utilisateur.
//   2. TRANSPARENCE   — chaque omission est signalée au LLM par un marqueur, et
//                        détaillée dans `Report` (affichable, auditable).
//   3. PRUDENCE       — seuils hauts, passages épinglés, plancher par document.
// ============================================================================

pub mod client;
pub mod policy;
pub mod segment;

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

use crate::config;
use client::WinnowError;
use policy::{DropReason, Omission, Policy};

// ── Disjoncteur ───────────────────────────────────────────────────────────────
// Évite de payer un timeout à chaque requête quand Ollaya est éteint.

static OPEN_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn open_breaker(secs: u64) {
    OPEN_UNTIL_MS.store(now_ms() + secs * 1_000, Ordering::Relaxed);
}

fn breaker_remaining_secs() -> u64 {
    OPEN_UNTIL_MS
        .load(Ordering::Relaxed)
        .saturating_sub(now_ms())
        .div_ceil(1_000)
}

// ── Types publics ─────────────────────────────────────────────────────────────

pub struct DocIn<'a> {
    pub titre: &'a str,
    pub contenu: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct DroppedItem {
    pub doc: usize,
    pub doc_titre: String,
    pub heading: Option<String>,
    pub reason: &'static str,
    pub p: f64,
    pub preview: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Report {
    pub enabled: bool,
    /// Vrai si Winnow a effectivement scoré au moins un passage.
    pub applied: bool,
    pub model: String,
    pub total_passages: usize,
    pub scored: usize,
    pub kept: usize,
    pub dropped: usize,
    /// Passages conservés faute de score (épinglés, budget épuisé, erreur…).
    pub unscored: usize,
    pub chars_before: usize,
    pub chars_after: usize,
    pub elapsed_ms: u64,
    /// Raison pour laquelle le filtrage n'a pas (ou pas entièrement) eu lieu.
    pub fallback: Option<String>,
    pub dropped_items: Vec<DroppedItem>,
}

pub struct Outcome {
    /// Un contenu par document d'entrée, dans le même ordre. Identique à l'entrée si rien n'est écarté.
    pub contenus: Vec<String>,
    pub report: Report,
}

/// Note à ajouter au prompt quand des passages ont été retirés.
pub fn prompt_note(report: &Report) -> Option<String> {
    if report.dropped == 0 {
        return None;
    }
    Some(format!(
        "_Note : {} passage(s) jugé(s) sans lien avec la demande ou sans contenu juridique ont été \
         retirés automatiquement par un pré-filtre local (repérés par « […] »). N'en déduis pas \
         l'absence d'une disposition sur la seule base du texte fourni._",
        report.dropped
    ))
}

// ── Filtrage ──────────────────────────────────────────────────────────────────

struct Slot {
    doc: usize,
    passage: segment::Passage,
    pinned: bool,
    scores: Option<client::Scores>,
}

fn passthrough(docs: &[DocIn<'_>], mut report: Report, started: Instant) -> Outcome {
    report.elapsed_ms = started.elapsed().as_millis() as u64;
    Outcome {
        contenus: docs.iter().map(|d| d.contenu.to_string()).collect(),
        report,
    }
}

fn canonical_model(name: &str) -> String {
    let n = name.trim().to_lowercase();
    if n.contains(':') {
        n
    } else {
        format!("{n}:latest")
    }
}

/// Filtre `docs` selon `focus` (demande de l'utilisateur, facultative).
/// Sans `focus`, seule la question « rôle » est posée : le filtre se limite alors
/// à retirer les passages sans contenu juridique (mentions, en-têtes…).
///
/// `enabled` reflète la case à cocher des préférences utilisateur (et non une
/// variable d'environnement) : c'est à l'appelant HTTP de la transmettre à
/// chaque requête (voir `EnvoyerChatRequest::winnow_enabled`).
pub async fn filter_docs(docs: &[DocIn<'_>], focus: Option<&str>, enabled: bool) -> Outcome {
    let started = Instant::now();
    let chars_before: usize = docs.iter().map(|d| d.contenu.len()).sum();
    let mut report = Report {
        enabled,
        model: config::winnow_model(),
        chars_before,
        chars_after: chars_before,
        ..Default::default()
    };

    if !report.enabled {
        return passthrough(docs, report, started);
    }
    if chars_before < config::winnow_min_chars() {
        report.fallback = Some("volume trop faible : filtrage inutile".into());
        return passthrough(docs, report, started);
    }
    let remaining = breaker_remaining_secs();
    if remaining > 0 {
        report.fallback = Some(format!(
            "Ollaya indisponible (nouvel essai dans {remaining} s)"
        ));
        return passthrough(docs, report, started);
    }

    // Sonde + vérification que le modèle est chargé (sinon on le charge en tâche
    // de fond et on n'attend PAS : le texte part intégral cette fois-ci).
    if let Err(e) = client::version(Duration::from_millis(800)).await {
        open_breaker(30);
        report.fallback = Some(e.to_string());
        return passthrough(docs, report, started);
    }
    let want = canonical_model(&report.model);
    let loaded = client::list("/api/ps", Duration::from_secs(2))
        .await
        .map(|v| {
            v["models"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .any(|m| canonical_model(m["name"].as_str().unwrap_or("")) == want)
                })
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if !loaded {
        tokio::spawn(async {
            match client::preload().await {
                Ok(()) => info!("[Winnow] modèle chargé en arrière-plan"),
                Err(e) => {
                    warn!("[Winnow] préchargement impossible : {e}");
                    open_breaker(60);
                }
            }
        });
        report.fallback = Some(
            "modèle en cours de chargement : le filtre sera actif à la prochaine analyse".into(),
        );
        return passthrough(docs, report, started);
    }

    // ── Segmentation ─────────────────────────────────────────────────────────
    let (min_c, max_c) = (
        config::winnow_min_passage_chars(),
        config::winnow_max_passage_chars(),
    );
    let refs = focus.map(policy::pinned_refs).unwrap_or_default();
    let mut slots: Vec<Slot> = Vec::new();
    for (di, d) in docs.iter().enumerate() {
        for (pi, p) in segment::segment(d.contenu, min_c, max_c)
            .into_iter()
            .enumerate()
        {
            let pinned = pi == 0 || policy::mentions_ref(&p.text, &refs);
            slots.push(Slot {
                doc: di,
                passage: p,
                pinned,
                scores: None,
            });
        }
    }
    report.total_passages = slots.len();

    // ── Scoring (séquentiel : Ollaya traite une requête à la fois par modèle) ─
    let has_focus = focus.map(|f| !f.trim().is_empty()).unwrap_or(false);
    let questions = policy::questions(has_focus);
    let deadline = started + config::winnow_time_budget();
    let max_scored = config::winnow_max_passages();
    let mut consecutive_errors = 0u32;

    for slot in slots.iter_mut().filter(|s| !s.pinned) {
        if report.scored >= max_scored {
            report.fallback = Some(format!(
                "plafond de {max_scored} passages atteint : les suivants sont conservés"
            ));
            break;
        }
        if Instant::now() >= deadline {
            report.fallback = Some("budget de temps épuisé : passages restants conservés".into());
            break;
        }
        let state = policy::state(
            focus,
            docs[slot.doc].titre,
            slot.passage.heading.as_deref(),
            &slot.passage.text,
        );
        match client::decide(&state, &questions).await {
            Ok(s) => {
                consecutive_errors = 0;
                report.scored += 1;
                slot.scores = Some(s);
            }
            Err(e) if e.is_fatal() => {
                warn!("[Winnow] arrêt : {e}");
                open_breaker(60);
                report.fallback = Some(e.to_string());
                break;
            }
            Err(e) => {
                consecutive_errors += 1;
                warn!("[Winnow] décision échouée ({consecutive_errors}/3) : {e}");
                if consecutive_errors >= 3 {
                    open_breaker(30);
                    report.fallback = Some(format!("erreurs répétées : {e}"));
                    break;
                }
            }
        }
    }

    // ── Politique + plancher par document ────────────────────────────────────
    let pol = Policy {
        drop_threshold: config::winnow_drop_threshold(),
        role_threshold: config::winnow_role_threshold(),
        min_keep_ratio: config::winnow_min_keep_ratio(),
    };
    let mut drops: Vec<Option<Omission>> = slots
        .iter()
        .map(|s| {
            s.scores
                .as_ref()
                .and_then(|sc| policy::verdict(sc, s.pinned, &pol))
        })
        .collect();
    for di in 0..docs.len() {
        let idx: Vec<usize> = (0..slots.len()).filter(|i| slots[*i].doc == di).collect();
        let mut sub: Vec<Option<Omission>> = idx.iter().map(|i| drops[*i]).collect();
        policy::enforce_floor(&mut sub, pol.min_keep_ratio);
        for (k, i) in idx.iter().enumerate() {
            drops[*i] = sub[k];
        }
    }

    // ── Assemblage ───────────────────────────────────────────────────────────
    let mut contenus: Vec<String> = Vec::with_capacity(docs.len());
    for (di, d) in docs.iter().enumerate() {
        let idx: Vec<usize> = (0..slots.len()).filter(|i| slots[*i].doc == di).collect();
        if idx.iter().all(|i| drops[*i].is_none()) {
            contenus.push(d.contenu.to_string()); // strictement inchangé
            continue;
        }
        let mut out: Vec<String> = Vec::new();
        let mut run: Vec<DropReason> = Vec::new();
        let flush = |out: &mut Vec<String>, run: &mut Vec<DropReason>| {
            if run.is_empty() {
                return;
            }
            let mut labels: Vec<&str> = run.iter().map(|r| r.label()).collect();
            labels.dedup();
            out.push(format!(
                "[… {} passage(s) écarté(s) par le pré-filtre local : {} …]",
                run.len(),
                labels.join(", ")
            ));
            run.clear();
        };
        for i in idx {
            match drops[i] {
                Some(dr) => {
                    run.push(dr.reason);
                    if report.dropped_items.len() < 200 {
                        report.dropped_items.push(DroppedItem {
                            doc: di,
                            doc_titre: d.titre.to_string(),
                            heading: slots[i].passage.heading.clone(),
                            reason: dr.reason.label(),
                            p: (dr.p * 1000.0).round() / 1000.0,
                            preview: slots[i].passage.text.chars().take(120).collect(),
                        });
                    }
                    report.dropped += 1;
                }
                None => {
                    flush(&mut out, &mut run);
                    out.push(slots[i].passage.text.clone());
                }
            }
        }
        flush(&mut out, &mut run);
        contenus.push(out.join("\n\n"));
    }

    report.applied = report.scored > 0;
    report.kept = report.total_passages - report.dropped;
    report.unscored = slots
        .iter()
        .filter(|s| !s.pinned && s.scores.is_none())
        .count();
    report.chars_after = contenus.iter().map(|c| c.len()).sum();
    report.elapsed_ms = started.elapsed().as_millis() as u64;
    info!(
        "[Winnow] {} passages, {} scorés, {} écartés, {} → {} car., {} ms{}",
        report.total_passages,
        report.scored,
        report.dropped,
        report.chars_before,
        report.chars_after,
        report.elapsed_ms,
        report
            .fallback
            .as_deref()
            .map(|f| format!(" ({f})"))
            .unwrap_or_default()
    );
    Outcome { contenus, report }
}

// ── Statut & préchauffage (exposés à l'UI) ────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Status {
    pub enabled: bool,
    pub reachable: bool,
    pub version: Option<String>,
    pub model: String,
    pub model_installed: bool,
    pub model_loaded: bool,
    pub device: Option<String>,
    /// Commande ou action à effectuer pour rendre le filtre opérationnel.
    pub hint: Option<String>,
}

pub async fn status(enabled: bool) -> Status {
    let model = config::winnow_model();
    let want = canonical_model(&model);
    let mut st = Status {
        enabled,
        reachable: false,
        version: None,
        model: model.clone(),
        model_installed: false,
        model_loaded: false,
        device: None,
        hint: None,
    };
    if !enabled {
        return st; // pré-filtre désactivé dans les préférences : pas de sonde réseau
    }
    match client::version(Duration::from_millis(1_500)).await {
        Ok(v) => {
            st.reachable = true;
            st.version = Some(v);
        }
        Err(e) => {
            st.hint = Some(format!("{e} — lancer `ollaya serve`"));
            return st;
        }
    }
    let has = |v: &serde_json::Value| -> Option<serde_json::Value> {
        v["models"]
            .as_array()?
            .iter()
            .find(|m| canonical_model(m["name"].as_str().unwrap_or("")) == want)
            .cloned()
    };
    if let Ok(v) = client::list("/api/tags", Duration::from_secs(3)).await {
        st.model_installed = has(&v).is_some();
    }
    if let Ok(v) = client::list("/api/ps", Duration::from_secs(3)).await {
        if let Some(m) = has(&v) {
            st.model_loaded = true;
            st.device = m["device"].as_str().map(|s| s.to_string());
        }
    }
    if !st.model_installed {
        st.hint = Some(format!("ollaya pull {model}"));
    } else if st.device.as_deref() == Some("cpu") {
        st.hint = Some(
            "Winnow tourne sur CPU (≈ 5 s par passage) : préférer un GPU, ou un modèle encodeur plus léger (voir docs/WINNOW.md)".into(),
        );
    }
    st
}

/// Charge le modèle en mémoire (à appeler à l'activation du filtre ou au démarrage).
pub async fn warmup() -> Result<(), WinnowError> {
    let r = client::preload().await;
    if r.is_ok() {
        OPEN_UNTIL_MS.store(0, Ordering::Relaxed);
    }
    r
}
