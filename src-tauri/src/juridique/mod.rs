// ============================================================================
// Juridix — Module Juridique — mod.rs
// ============================================================================
// Module "Recherche juridique" : Search Engine PISTE (Légifrance + JUDILIBRE)
// + handlers Axum + état partagé.
//
// Intégration dans api.rs :
//
//   1. Ajouter `juridique: crate::juridique::routes::JuridiqueState` dans AppState
//   2. Implémenter `HasJuridiqueState` pour AppState
//   3. Dans build_router, nester juridique_router() sous "/api/juridique"
//      et "/api-proxy/api/juridique"
//   4. Dans init_credentials, alimenter JuridiqueState depuis les champs
//      piste_client_id / piste_client_secret
//
// Voir patch complet dans les fichiers _patch_*.rs livrés.
// ============================================================================

pub mod client;
pub mod judilibre;
pub mod legifrance;
pub mod routes;

// Re-exports pour api.rs
pub use routes::{juridique_router, HasJuridiqueState, JuridiqueState};
