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
// Juridix — Module Juridique — mod.rs
// ============================================================================
// Module "Recherche juridique" : Search Engine PISTE (Légifrance + JUDILIBRE)
// + handlers Axum + état partagé.
// ============================================================================

pub mod client;
pub mod code_query;
pub mod code_search;
pub mod judilibre;
pub mod jurisprudence;
pub mod legifrance;
pub mod routes;
pub mod temporal;
pub mod verify;
pub mod verify_ext;
pub mod verify_juris;

// Re-exports pour api.rs
pub use routes::{juridique_router, HasJuridiqueState, JuridiqueState};
