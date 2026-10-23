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

/// Configuration centrale de Juridix.
///
/// Toutes les valeurs sont lues depuis les variables d'environnement,
/// elles-mêmes chargées depuis le fichier `.env` au démarrage (via dotenvy).
/// Les valeurs par défaut sont définies ici et documentées dans `.env.example`.
use std::time::Duration;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ── Serveur ───────────────────────────────────────────────────────────────────

/// Port d'écoute du serveur HTTP interne (défaut : 51873).
pub fn api_port() -> u16 {
    env_u16("API_PORT", 51873)
}

// ── Qdrant ────────────────────────────────────────────────────────────────────

/// URL de l'instance Qdrant locale (défaut : http://localhost:6333).
pub fn qdrant_url() -> String {
    env_str("QDRANT_URL", "http://localhost:6333")
}

/// Clé d'API Qdrant (optionnelle — None si vide ou absente).
pub fn qdrant_api_key() -> Option<String> {
    match std::env::var("QDRANT_API_KEY") {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

// ── RAG ───────────────────────────────────────────────────────────────────────

/// Modèle d'embedding Albert utilisé pour vectoriser les chunks et les requêtes.
/// Doit correspondre à la dimension `EMBEDDING_DIM`.
pub fn embedding_model() -> String {
    env_str("EMBEDDING_MODEL", "BAAI/bge-m3")
}

/// Dimension des vecteurs produits par `EMBEDDING_MODEL` (défaut : 1024 pour bge-m3).
pub fn embedding_dim() -> usize {
    env_usize("EMBEDDING_DIM", 1024)
}

/// Modèle de reranking Albert (défaut : BAAI/bge-reranker-v2-m3).
pub fn rerank_model() -> String {
    env_str("RERANK_MODEL", "BAAI/bge-reranker-v2-m3")
}

/// Nombre de chunks récupérés lors de la recherche initiale (défaut : 20).
pub fn rag_top_k() -> usize {
    env_usize("RAG_TOP_K", 20)
}

/// Nombre de chunks conservés après reranking (défaut : 5).
pub fn rag_top_rerank() -> usize {
    env_usize("RAG_TOP_RERANK", 5)
}

/// Score de similarité minimum pour qu'un chunk soit retenu (défaut : 0.15).
pub fn rag_min_score() -> f64 {
    env_f64("RAG_MIN_SCORE", 0.15)
}

/// Nombre de chunks chart_image récupérés via la recherche dédiée images (défaut : 5).
pub fn rag_image_top_k() -> usize {
    env_usize("RAG_IMAGE_TOP_K", 5)
}

// ── LLM ───────────────────────────────────────────────────────────────────────

/// Nombre maximum de tokens pour les appels LLM principaux (défaut : 100000).
pub fn llm_max_tokens() -> u64 {
    env_u64("LLM_MAX_TOKENS", 100_000)
}

/// Bornes acceptées pour le plafond choisi par l'utilisateur dans les préférences.
const USER_MAX_TOKENS_MIN: u64 = 256;
const USER_MAX_TOKENS_MAX: u64 = 1_000_000;

/// Plafond de tokens effectif d'une requête : la valeur demandée par l'utilisateur (préférences)
/// si elle est raisonnable, sinon la valeur du serveur (`LLM_MAX_TOKENS`, défaut 100000).
pub fn resolve_max_tokens(requested: Option<u64>) -> u64 {
    match requested {
        Some(n) if (USER_MAX_TOKENS_MIN..=USER_MAX_TOKENS_MAX).contains(&n) => n,
        _ => llm_max_tokens(),
    }
}

/// Nombre maximum de tokens pour la génération de titre (défaut : 20).
pub fn title_max_tokens() -> u64 {
    env_u64("TITLE_MAX_TOKENS", 20)
}

/// Nombre maximum de tours dans la boucle agentique MCP (défaut : 12).
pub fn mcp_max_turns() -> usize {
    env_usize("MCP_MAX_TURNS", 12)
}

/// Taille maximum en bytes du premier message utilisateur pour le titrage (défaut : 1200).
pub fn title_truncate_user() -> usize {
    env_usize("TITLE_TRUNCATE_USER", 1200)
}

/// Taille maximum en bytes de la première réponse assistant pour le titrage (défaut : 600).
pub fn title_truncate_assistant() -> usize {
    env_usize("TITLE_TRUNCATE_ASSISTANT", 600)
}

// ── Timeouts HTTP ─────────────────────────────────────────────────────────────

/// Timeout pour les appels LLM streamés (défaut : 120s).
pub fn timeout_llm_stream() -> Duration {
    Duration::from_secs(env_u64("TIMEOUT_LLM_STREAM_SECS", 120))
}

/// Timeout pour les appels LLM non-streamés / MCP (défaut : 120s).
pub fn timeout_llm_sync() -> Duration {
    Duration::from_secs(env_u64("TIMEOUT_LLM_SYNC_SECS", 120))
}

/// Timeout pour la génération de titre (défaut : 30s).
pub fn timeout_title() -> Duration {
    Duration::from_secs(env_u64("TIMEOUT_TITLE_SECS", 30))
}

/// Timeout pour les appels RAG / Albert search (défaut : 30s).
pub fn timeout_rag() -> Duration {
    Duration::from_secs(env_u64("TIMEOUT_RAG_SECS", 30))
}

/// Timeout pour le reranking (défaut : 30s).
pub fn timeout_rerank() -> Duration {
    Duration::from_secs(env_u64("TIMEOUT_RERANK_SECS", 30))
}

/// Timeout pour la résolution des noms de documents (défaut : 10s).
pub fn timeout_doc_resolve() -> Duration {
    Duration::from_secs(env_u64("TIMEOUT_DOC_RESOLVE_SECS", 10))
}

/// Timeout pour les appels de liste de modèles et autres appels courts (défaut : 8s).
pub fn timeout_models() -> Duration {
    Duration::from_secs(env_u64("TIMEOUT_MODELS_SECS", 8))
}

// ── Pré-filtre local « Winnow » (serveur Ollaya) ──────────────────────────────
//
// Couche optionnelle entre l'extraction des textes (Légifrance / JUDILIBRE) et
// le LLM distant : un modèle de décision (Winnow, servi par Ollaya sur la
// machine de l'utilisateur) écarte les passages hors-sujet ou sans contenu
// juridique avant construction du prompt. Voir docs/WINNOW.md.
//
// Principe directeur : ÉCHEC OUVERT. Serveur absent, modèle non téléchargé,
// délai dépassé → le texte part intégralement, comme avant.

// Note : l'activation du pré-filtre (WINNOW_ENABLED) est une préférence
// utilisateur — case à cocher dans Réglages ⚙, envoyée par requête (champ
// `winnow_enabled` de `EnvoyerChatRequest`), et non une variable d'environnement :
// deux instances de Juridix sur le même poste peuvent ainsi avoir des choix
// différents, et le changement est pris en compte sans redémarrage.
// Les réglages ci-dessous restent en .env : ce sont des détails d'installation
// locale (où tourne Ollaya, quel modèle) plutôt que des préférences d'usage.

/// URL du serveur Ollaya local (défaut : http://127.0.0.1:11435).
pub fn ollaya_url() -> String {
    env_str("OLLAYA_URL", "http://127.0.0.1:11435")
        .trim_end_matches('/')
        .to_string()
}

/// Clé d'API Ollaya (si le serveur est lancé avec OLLAYA_API_KEY).
pub fn ollaya_api_key() -> Option<String> {
    match std::env::var("OLLAYA_API_KEY") {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

/// Modèle de décision (défaut : winnow:e4b — voir https://ollaya.dev/library/winnow).
pub fn winnow_model() -> String {
    env_str("WINNOW_MODEL", "winnow:e4b")
}

/// Durée de maintien du modèle en mémoire après usage (défaut : 30m).
pub fn winnow_keep_alive() -> String {
    env_str("WINNOW_KEEP_ALIVE", "30m")
}

/// Un passage n'est écarté comme « hors sujet » que si P(niveau 0) ≥ seuil (défaut : 0.85).
/// Seuil volontairement haut : en droit, écarter à tort coûte plus cher que garder à tort.
pub fn winnow_drop_threshold() -> f64 {
    env_f64("WINNOW_DROP_THRESHOLD", 0.85)
}

/// Idem pour le rôle « mentions sans contenu juridique » (défaut : 0.85).
pub fn winnow_role_threshold() -> f64 {
    env_f64("WINNOW_ROLE_THRESHOLD", 0.85)
}

/// Nombre maximum de passages soumis à Winnow par requête ; le reste est conservé tel quel.
pub fn winnow_max_passages() -> usize {
    env_usize("WINNOW_MAX_PASSAGES", 400)
}

/// Budget de temps global du filtrage ; au-delà, les passages restants sont conservés.
pub fn winnow_time_budget() -> Duration {
    Duration::from_secs(env_u64("WINNOW_TIME_BUDGET_SECS", 30))
}

/// Timeout d'une décision unitaire (défaut : 20s).
pub fn winnow_request_timeout() -> Duration {
    Duration::from_secs(env_u64("WINNOW_REQUEST_TIMEOUT_SECS", 20))
}

/// Timeout du chargement initial du modèle en mémoire (défaut : 90s).
pub fn winnow_load_timeout() -> Duration {
    Duration::from_secs(env_u64("WINNOW_LOAD_TIMEOUT_SECS", 90))
}

/// En dessous de ce volume total (caractères), on n'appelle pas Winnow (défaut : 8000).
pub fn winnow_min_chars() -> usize {
    env_usize("WINNOW_MIN_CHARS", 8_000)
}

/// Taille maximale d'un passage (défaut : 5000 car. ≈ 1400 tokens ; contexte Winnow = 8192).
pub fn winnow_max_passage_chars() -> usize {
    env_usize("WINNOW_MAX_PASSAGE_CHARS", 5_000)
}

/// Les blocs plus courts que ce seuil sont fusionnés avec le suivant (défaut : 300).
pub fn winnow_min_passage_chars() -> usize {
    env_usize("WINNOW_MIN_PASSAGE_CHARS", 300)
}

/// Part minimale des passages d'un document qui doit survivre au filtre (défaut : 0.2).
pub fn winnow_min_keep_ratio() -> f64 {
    env_f64("WINNOW_MIN_KEEP_RATIO", 0.2)
}

#[cfg(test)]
mod max_tokens_tests {
    use super::*;

    #[test]
    fn a_reasonable_user_value_wins_over_the_server_default() {
        assert_eq!(resolve_max_tokens(Some(16_000)), 16_000);
        assert_eq!(resolve_max_tokens(Some(256)), 256);
        assert_eq!(resolve_max_tokens(Some(1_000_000)), 1_000_000);
    }

    #[test]
    fn absent_or_absurd_values_fall_back_to_the_server_default() {
        let defaut = llm_max_tokens();
        assert_eq!(resolve_max_tokens(None), defaut);
        assert_eq!(resolve_max_tokens(Some(0)), defaut);
        assert_eq!(resolve_max_tokens(Some(255)), defaut);
        assert_eq!(resolve_max_tokens(Some(1_000_001)), defaut);
    }
}
