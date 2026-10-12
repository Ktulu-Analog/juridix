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

use std::net::SocketAddr;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::Manager;
use tracing::info;

mod api;
mod config;
mod db;
mod extract;
mod juridique;
mod mcp;
mod mcp_grants;
mod mcp_pack;
mod mcp_stdio;
mod models;
mod rag;
mod rgpd;
mod secrets;
mod security;
mod spaces;
mod veille;
mod winnow;

pub use api::{AppState, Credentials};
pub use db::Database;

// ── Commande Tauri : initialise les credentials dans AppState ─────────────────
// Appelée depuis le frontend via invoke() — IPC synchrone, pas HTTP.
// Garantit que AppState est peuplé avant toute requête HTTP au serveur Axum.

#[derive(serde::Deserialize)]
struct CredentialsPayload {
    endpoint: String,
    bearer: String,
    qdrant_api_key: Option<String>,
    qdrant_url: Option<String>,
    user_email: Option<String>,
    /// Tokens JWT additionnels par collection (utilisateurs non-admin multi-collections)
    #[serde(default)]
    qdrant_collection_tokens: Vec<crate::api::CollectionToken>,
    /// Credentials PISTE pour l'API juridique (Légifrance + JUDILIBRE)
    piste_client_id: Option<String>,
    piste_client_secret: Option<String>,
}

#[tauri::command]
async fn init_credentials(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    payload: CredentialsPayload,
) -> Result<(), String> {
    let mut creds = state.credentials.write().await;
    creds.endpoint = payload.endpoint;
    creds.bearer = payload.bearer;
    creds.qdrant_api_key = payload.qdrant_api_key.filter(|k| !k.trim().is_empty());
    creds.qdrant_url = payload.qdrant_url.filter(|u| !u.trim().is_empty());
    creds.user_email = payload.user_email.filter(|e| !e.trim().is_empty());
    creds.qdrant_collection_tokens = payload
        .qdrant_collection_tokens
        .into_iter()
        .filter(|ct| !ct.token.trim().is_empty() && !ct.collection.trim().is_empty())
        .collect();

    // Ne jamais journaliser les clés/jetons (même partiellement) : seul un
    // récapitulatif non sensible est tracé.
    tracing::debug!(
        "init_credentials : nb_collection_tokens={} qdrant_url_set={}",
        creds.qdrant_collection_tokens.len(),
        creds.qdrant_url.is_some(),
    );

    if let (Some(id), Some(secret)) = (payload.piste_client_id, payload.piste_client_secret) {
        if !id.trim().is_empty() && !secret.trim().is_empty() {
            // Mêmes identifiants pour les serveurs MCP STDIO (Légifrance, Judilibre) :
            // transmis en mémoire à leur processus, jamais écrits sur disque.
            mcp_stdio::HUB.set_secret(mcp_stdio::SECRET_PISTE_ID, id.trim());
            mcp_stdio::HUB.set_secret(mcp_stdio::SECRET_PISTE_SECRET, secret.trim());
            state.juridique.reinit_client(id, secret);
            tracing::info!("[Juridique] Client PISTE réinitialisé depuis init_credentials");
        }
    }

    Ok(())
}

// ── Commandes Tauri : stockage sécurisé des secrets (trousseau système) ───────
// Remplace le stockage en clair dans le localStorage du frontend (voir
// frontend/src/secureStorage.ts, qui appelle ces trois commandes). Chaque
// commande décharge l'appel bloquant au trousseau sur le pool blocking de
// tokio pour ne jamais geler la boucle async le temps d'un accès Keychain /
// Credential Manager / Secret Service.
//
// `key` est vérifiée contre `secrets::FRONTEND_ALLOWED_KEYS` : le frontend ne
// peut ni lire ni écraser la clé de chiffrement de la base (`db_encryption_key`),
// qui n'y figure pas.

fn check_frontend_key(key: &str) -> Result<(), String> {
    if secrets::FRONTEND_ALLOWED_KEYS.contains(&key) {
        Ok(())
    } else {
        Err(format!(
            "clé de secret non autorisée depuis le frontend : {key}"
        ))
    }
}

#[tauri::command]
async fn store_secret(key: String, value: String) -> Result<(), String> {
    check_frontend_key(&key)?;
    tokio::task::spawn_blocking(move || secrets::store(&key, &value))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn read_secret(key: String) -> Result<Option<String>, String> {
    check_frontend_key(&key)?;
    tokio::task::spawn_blocking(move || secrets::read(&key))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_secret(key: String) -> Result<(), String> {
    check_frontend_key(&key)?;
    tokio::task::spawn_blocking(move || secrets::delete(&key))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

// ── Commandes Tauri : export / effacement RGPD (chantier 3) ────────────────────
// Volontairement des commandes IPC et non des routes HTTP : elles ne sont pas
// joignables par l'API locale, même avec le jeton.

/// Exporte toutes les données de l'utilisateur dans une archive ZIP (AES-256 si un mot de
/// passe est fourni). `frontend_data` : `localStorage` sérialisé par le frontend.
#[tauri::command]
async fn export_user_data(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    dest: String,
    frontend_data: String,
    password: Option<String>,
) -> Result<rgpd::ExportReport, String> {
    let tables = state
        .db
        .dump_all_tables()
        .await
        .map_err(|e| format!("{e:#}"))?;
    let prompts = std::fs::read_to_string(&state.prompts_path).ok();
    let dest = std::path::PathBuf::from(dest);
    tokio::task::spawn_blocking(move || {
        rgpd::write_export_zip(&rgpd::ExportInput {
            dest: &dest,
            tables: &tables,
            prompts_yaml: prompts,
            frontend_json: &frontend_data,
            password: password.as_deref(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

/// Premier temps de l'effacement de toutes les données (voir rgpd.rs). Exige le mot de
/// confirmation ; le frontend vide ensuite son `localStorage` puis appelle `restart_app`.
#[tauri::command]
async fn wipe_user_data(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    paths: tauri::State<'_, rgpd::DataPaths>,
    confirmation: String,
) -> Result<rgpd::WipeReport, String> {
    if confirmation.trim() != rgpd::WIPE_CONFIRMATION {
        return Err("confirmation invalide".to_string());
    }
    rgpd::wipe_now(paths.inner(), &state.db)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Redémarre l'application (après un effacement : le second temps s'exécute au lancement).
#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

/// État de la sauvegarde en clair laissée par la migration vers SQLCipher (R-01).
#[tauri::command]
fn backup_status(paths: tauri::State<'_, rgpd::DataPaths>) -> rgpd::BackupStatus {
    rgpd::backup_status(paths.inner())
}

/// Supprime (écrasement puis suppression) la sauvegarde en clair.
#[tauri::command]
fn delete_backup(paths: tauri::State<'_, rgpd::DataPaths>) -> Result<(), String> {
    rgpd::delete_backup(paths.inner()).map_err(|e| format!("{e:#}"))
}

// ── Commande Tauri : jeton de l'API HTTP locale ───────────────────────────────
// Le frontend le lit une fois au démarrage (IPC) puis l'ajoute en en-tête
// `X-Juridix-Token` à chaque requête vers le serveur axum. Voir `security.rs`.
#[tauri::command]
fn get_api_token(state: tauri::State<'_, std::sync::Arc<AppState>>) -> String {
    state.api_token.clone()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Charge le fichier .env s'il existe (silencieux si absent)
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "juridix=info,tower_http=info".to_string()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            // ── DevTools en mode debug ────────────────────────────────────────
            #[cfg(debug_assertions)]
            if let Some(w) = app.get_webview_window("main") {
                w.open_devtools();
            }

            // ── Menu natif ────────────────────────────────────────────────────
            let handle = app.handle();

            let menu = Menu::with_items(
                handle,
                &[
                    // Juridix (app menu sur macOS)
                    &Submenu::with_items(
                        handle,
                        "Juridix",
                        true,
                        &[
                            &MenuItem::with_id(
                                handle,
                                "new_conv",
                                "Nouvelle conversation",
                                true,
                                Some("CmdOrCtrl+N"),
                            )?,
                            &PredefinedMenuItem::separator(handle)?,
                            &MenuItem::with_id(
                                handle,
                                "settings",
                                "Paramètres…",
                                true,
                                Some("CmdOrCtrl+,"),
                            )?,
                            &PredefinedMenuItem::separator(handle)?,
                            &MenuItem::with_id(
                                handle,
                                "quit",
                                "Quitter",
                                true,
                                Some("CmdOrCtrl+Q"),
                            )?,
                        ],
                    )?,
                    // Espaces & Prompts
                    &Submenu::with_items(
                        handle,
                        "Espaces",
                        true,
                        &[
                            &MenuItem::with_id(
                                handle,
                                "spaces_editor",
                                "Espaces & prompts…",
                                true,
                                Some("CmdOrCtrl+E"),
                            )?,
                            &MenuItem::with_id(
                                handle,
                                "rag",
                                "Ingestion RAG…",
                                true,
                                Some("CmdOrCtrl+R"),
                            )?,
                        ],
                    )?,
                    // Affichage
                    &Submenu::with_items(
                        handle,
                        "Affichage",
                        true,
                        &[
                            &MenuItem::with_id(
                                handle,
                                "toggle_sidebar",
                                "Afficher/Masquer sidebar",
                                true,
                                Some("CmdOrCtrl+B"),
                            )?,
                            &PredefinedMenuItem::separator(handle)?,
                            &PredefinedMenuItem::fullscreen(handle, None)?,
                        ],
                    )?,
                    // Aide
                    &Submenu::with_items(
                        handle,
                        "Aide",
                        true,
                        &[&MenuItem::with_id(
                            handle,
                            "about",
                            "À propos de Juridix…",
                            true,
                            None::<&str>,
                        )?],
                    )?,
                ],
            )?;

            app.set_menu(menu)?;

            // Déclencher un event DOM sur clic de menu via eval JS
            app.on_menu_event(move |app, event| {
                let id = event.id().0.clone();
                if id == "quit" {
                    app.exit(0);
                    return;
                }
                if let Some(window) = app.get_webview_window("main") {
                    let js = format!(
                        "window.dispatchEvent(new CustomEvent('tauri-menu', {{ detail: '{}' }}))",
                        id
                    );
                    let _ = window.eval(&js);
                }
            });

            // ── API server ────────────────────────────────────────────────────
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data dir");
            std::fs::create_dir_all(&app_data_dir).expect("Failed to create data dir");

            // Effacement RGPD en attente (demandé avant ce lancement) : à terminer AVANT de
            // récupérer la clé du trousseau et d'ouvrir la base — voir rgpd.rs.
            let data_paths = rgpd::DataPaths::new(app_data_dir.clone());
            rgpd::run_pending_wipe(&data_paths)
                .expect("Effacement des données interrompu : voir le message ci-dessus");
            if rgpd::backup_status(&data_paths).exists {
                tracing::warn!(
                    "Une sauvegarde EN CLAIR de l'ancienne base est présente sur le disque : \
                     à supprimer dans Réglages > Données et confidentialité"
                );
            }

            let prompts_path = app
                .path()
                .resource_dir()
                .unwrap_or_else(|_| app_data_dir.clone())
                .join("prompts.yml");

            let prompts_path = if prompts_path.exists() {
                prompts_path
            } else {
                let p = app_data_dir.join("prompts.yml");
                if !p.exists() {
                    std::fs::write(&p, include_str!("../prompts.yml"))
                        .expect("Failed to write default prompts.yml");
                }
                p
            };

            let db_path = app_data_dir.join("conversations.db");
            // Serveurs MCP STDIO (Légifrance, Judilibre) : importés dans le dossier de
            // données de l'application, un sous-dossier par serveur (cf. mcp_pack.rs).
            let mcp_dir = app_data_dir.join("mcp-servers");
            info!("MCP STDIO dossier: {}", mcp_dir.display());
            mcp_stdio::HUB.set_dir(mcp_dir);

            info!("Data dir: {}", app_data_dir.display());
            info!("DB path: {}", db_path.display());
            info!("Prompts: {}", prompts_path.display());

            let db_path_clone = db_path.clone();
            let prompts_path_clone = prompts_path.clone();

            // Clé de chiffrement SQLCipher : récupérée dans le trousseau système,
            // ou générée au tout premier démarrage — voir secrets.rs. Échec fatal
            // volontaire ici (pas d'échec ouvert possible) : sans trousseau
            // accessible, il n'y a aucun moyen sûr de garantir que la même clé
            // sera retrouvée au prochain démarrage.
            let db_key = secrets::get_or_create_db_key().expect(
                "Impossible d'accéder au trousseau système pour la clé de chiffrement de la base",
            );

            let db = tauri::async_runtime::block_on(async {
                Database::new(&db_path_clone, &db_key)
                    .await
                    .expect("Failed to open database")
            });

            let (router, app_state) = api::build_router(db, prompts_path_clone);

            // Veille juridique : planificateur de fond (rejoue les veilles à échéance et
            // rattrape les périodes où l'application était fermée).
            tauri::async_runtime::spawn(veille::run_scheduler(app_state.clone()));

            // Partager AppState et les chemins de données avec les commandes Tauri
            handle.manage(app_state);
            handle.manage(data_paths);

            tauri::async_runtime::spawn(async move {
                let addr = SocketAddr::from(([127, 0, 0, 1], config::api_port()));
                info!("API server listening on http://{}", addr);

                let listener = tokio::net::TcpListener::bind(addr)
                    .await
                    .expect("Failed to bind API port");

                axum::serve(listener, router)
                    .await
                    .expect("API server crashed");
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            init_credentials,
            get_api_token,
            export_user_data,
            wipe_user_data,
            restart_app,
            backup_status,
            delete_backup,
            store_secret,
            read_secret,
            delete_secret
        ])
        .run(tauri::generate_context!())
        .expect("error while running Juridix");
}
