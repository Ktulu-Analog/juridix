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

//! Export et effacement des données de l'utilisateur (RGPD) — chantier 3.
//!
//! ## Périmètre de « tout »
//! - la base SQLite chiffrée (`conversations.db` et ses fichiers annexes) ;
//! - sa sauvegarde de migration `conversations.db.pre-encryption.bak` (EN CLAIR) ;
//! - le fichier `prompts.yml` du dossier de données ;
//! - le trousseau système : clé de chiffrement de la base et secrets
//!   (bearer, clé Qdrant, secret PISTE) ;
//! - les données d'interface (`localStorage` du WebView : réglages, dossiers,
//!   favoris, annotations, historique) — gérées côté frontend, transmises à
//!   l'export et vidées par le frontend lors de l'effacement.
//!
//! Hors périmètre, signalé à l'utilisateur : les collections Qdrant distantes et
//! les serveurs MCP installés (binaires, sans donnée personnelle).
//!
//! ## Effacement en deux temps (sûr en cas d'arrêt brutal)
//! La base est ouverte pendant l'exécution : on ne peut pas la supprimer
//! proprement (Windows) ni fermer sa connexion partagée. L'effacement :
//! 1. écrit un marqueur `wipe.pending` ;
//! 2. vide immédiatement toutes les tables (`DELETE` + `VACUUM`, `secure_delete`) ;
//! 3. écrase puis supprime les fichiers qui ne sont pas ouverts ;
//! 4. supprime les entrées du trousseau ;
//! 5. l'application redémarre ; **au démarrage suivant, avant toute ouverture de
//!    la base**, `run_pending_wipe` écrase puis supprime les fichiers restants
//!    et le marqueur. Si l'application est interrompue entre-temps, le marqueur
//!    garantit que l'effacement sera terminé au lancement suivant.
//!
//! Limite : l'écrasement avant suppression est « au mieux ». Il n'offre aucune
//! garantie sur un SSD (nivellement d'usure) ni sur un système de fichiers à
//! copie sur écriture ; seul le chiffrement du disque entier les apporte.

use std::fs::File;
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};

const PENDING_MARKER: &str = "wipe.pending";
const DB_FILE: &str = "conversations.db";
const PROMPTS_FILE: &str = "prompts.yml";
/// Fichiers annexes de la base (suffixes ajoutés au nom complet du fichier).
const DB_SIDECAR_SUFFIXES: &[&str] = &[
    "-wal",
    "-shm",
    "-journal",
    ".pre-encryption.bak",
    ".encrypting.tmp",
];
const BACKUP_SUFFIX: &str = ".pre-encryption.bak";

/// Mot que l'utilisateur doit saisir pour confirmer l'effacement (vérifié aussi ici,
/// pas seulement dans l'interface).
pub const WIPE_CONFIRMATION: &str = "EFFACER";

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Emplacements des données utilisateur (état géré par Tauri).
#[derive(Clone, Debug)]
pub struct DataPaths {
    pub data_dir: PathBuf,
}

impl DataPaths {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    fn db_path(&self) -> PathBuf {
        self.data_dir.join(DB_FILE)
    }

    fn backup_path(&self) -> PathBuf {
        with_suffix(&self.db_path(), BACKUP_SUFFIX)
    }

    fn marker_path(&self) -> PathBuf {
        self.data_dir.join(PENDING_MARKER)
    }

    /// Tous les fichiers à détruire lors d'un effacement.
    fn user_files(&self) -> Vec<PathBuf> {
        let db = self.db_path();
        let mut v = vec![db.clone()];
        v.extend(DB_SIDECAR_SUFFIXES.iter().map(|s| with_suffix(&db, s)));
        v.push(self.data_dir.join(PROMPTS_FILE));
        v
    }
}

/// Noms des entrées du trousseau à supprimer : clé de la base + secrets du frontend.
fn keychain_keys() -> Vec<&'static str> {
    std::iter::once(crate::secrets::DB_KEY_NAME)
        .chain(crate::secrets::FRONTEND_ALLOWED_KEYS.iter().copied())
        .collect()
}

// ── Suppression « au mieux sécurisée » ────────────────────────────────────────

/// Écrase le contenu d'un fichier avec des zéros puis le supprime. Un fichier
/// absent n'est pas une erreur. Voir la limite SSD dans la documentation du module.
pub fn secure_delete_file(path: &Path) -> std::io::Result<()> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.is_file() {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path) {
            let zeros = vec![0u8; 64 * 1024];
            let mut left = meta.len();
            while left > 0 {
                let n = left.min(zeros.len() as u64) as usize;
                if f.write_all(&zeros[..n]).is_err() {
                    break;
                }
                left -= n as u64;
            }
            let _ = f.sync_all();
        }
    }
    std::fs::remove_file(path)
}

// ── Sauvegarde en clair de la migration (R-01) ────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub exists: bool,
    pub path: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified: Option<String>,
}

/// État de la sauvegarde `conversations.db.pre-encryption.bak`, laissée EN CLAIR
/// sur le disque par la migration vers SQLCipher.
pub fn backup_status(paths: &DataPaths) -> BackupStatus {
    let p = paths.backup_path();
    match std::fs::metadata(&p) {
        Ok(m) if m.is_file() => BackupStatus {
            exists: true,
            path: Some(p.display().to_string()),
            size_bytes: Some(m.len()),
            modified: m
                .modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
        },
        _ => BackupStatus {
            exists: false,
            path: None,
            size_bytes: None,
            modified: None,
        },
    }
}

/// Supprime (écrasement puis suppression) la sauvegarde en clair.
pub fn delete_backup(paths: &DataPaths) -> Result<()> {
    let p = paths.backup_path();
    secure_delete_file(&p).with_context(|| format!("suppression de {}", p.display()))?;
    info!("Sauvegarde en clair supprimée : {}", p.display());
    Ok(())
}

// ── Effacement ────────────────────────────────────────────────────────────────

/// Écrit le marqueur d'effacement en attente (fsync).
fn schedule_wipe(paths: &DataPaths) -> Result<()> {
    let marker = paths.marker_path();
    let mut f =
        File::create(&marker).with_context(|| format!("création de {}", marker.display()))?;
    writeln!(f, "{}", chrono::Utc::now().to_rfc3339())?;
    f.sync_all()?;
    Ok(())
}

/// Termine un effacement interrompu ou différé. À appeler AU DÉMARRAGE, avant la
/// récupération de la clé du trousseau et l'ouverture de la base. Renvoie `true`
/// si un effacement a été exécuté.
pub fn run_pending_wipe(paths: &DataPaths) -> Result<bool> {
    run_pending_wipe_with(paths, &crate::secrets::delete)
}

/// Variante injectable (le trousseau réel ne doit jamais être touché par les tests).
fn run_pending_wipe_with(
    paths: &DataPaths,
    delete_key: &dyn Fn(&str) -> Result<()>,
) -> Result<bool> {
    let marker = paths.marker_path();
    if !marker.exists() {
        return Ok(false);
    }
    info!("Effacement des données en attente : exécution avant l'ouverture de la base");

    let mut failures = Vec::new();
    for p in paths.user_files() {
        if let Err(e) = secure_delete_file(&p) {
            failures.push(format!("{} : {}", p.display(), e));
        }
    }
    for key in keychain_keys() {
        if let Err(e) = delete_key(key) {
            warn!(
                "Effacement : entrée « {} » du trousseau non supprimée : {:#}",
                key, e
            );
        }
    }
    if !failures.is_empty() {
        bail!(
            "effacement incomplet — supprimez manuellement ces fichiers puis relancez : {}",
            failures.join(" ; ")
        );
    }
    std::fs::remove_file(&marker).context("suppression du marqueur d'effacement")?;
    info!("Effacement terminé");
    Ok(true)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WipeReport {
    pub fichiers_supprimes: Vec<String>,
    pub avertissements: Vec<String>,
    pub redemarrage_requis: bool,
}

/// Premier temps de l'effacement (voir la documentation du module) ; le frontend
/// vide ensuite son `localStorage` puis redémarre l'application.
pub async fn wipe_now(paths: &DataPaths, db: &crate::db::Database) -> Result<WipeReport> {
    // 1. Le marqueur d'abord : toute interruption ultérieure sera rattrapée au démarrage.
    schedule_wipe(paths)?;

    let mut avertissements = Vec::new();

    // 2. Contenu de la base (la base est ouverte : son fichier sera détruit au redémarrage).
    if let Err(e) = db.purge_all().await {
        avertissements.push(format!("Vidage de la base : {:#}", e));
    }

    // 3. Fichiers non ouverts.
    let mut supprimes = Vec::new();
    for p in [
        paths.backup_path(),
        with_suffix(&paths.db_path(), ".encrypting.tmp"),
        paths.data_dir.join(PROMPTS_FILE),
    ] {
        let existait = p.exists();
        match secure_delete_file(&p) {
            Ok(()) if existait => supprimes.push(p.display().to_string()),
            Ok(()) => {}
            Err(e) => avertissements.push(format!("{} : {}", p.display(), e)),
        }
    }

    // 4. Trousseau (I/O bloquante).
    for key in keychain_keys() {
        let res = tokio::task::spawn_blocking(move || crate::secrets::delete(key)).await;
        match res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => avertissements.push(format!("Trousseau « {} » : {:#}", key, e)),
            Err(e) => avertissements.push(format!("Trousseau « {} » : {}", key, e)),
        }
    }

    Ok(WipeReport {
        fichiers_supprimes: supprimes,
        avertissements,
        redemarrage_requis: true,
    })
}

// ── Export ────────────────────────────────────────────────────────────────────

pub struct ExportInput<'a> {
    pub dest: &'a Path,
    /// Toutes les tables de la base : `(nom, lignes)` — voir `Database::dump_all_tables`.
    pub tables: &'a [(String, Vec<Value>)],
    pub prompts_yaml: Option<String>,
    /// `localStorage` sérialisé par le frontend (JSON).
    pub frontend_json: &'a str,
    /// Mot de passe facultatif : chiffrement AES-256 de l'archive.
    pub password: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub conversations: usize,
    pub fichiers: usize,
    pub chiffree: bool,
}

fn validate_dest(dest: &Path) -> Result<()> {
    let ext_ok = dest
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);
    if !ext_ok {
        bail!("le fichier d'export doit avoir l'extension .zip");
    }
    if dest.is_dir() {
        bail!("la destination est un dossier : {}", dest.display());
    }
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        bail!("dossier de destination introuvable : {}", parent.display());
    }
    Ok(())
}

/// Crée le fichier d'export lisible par son seul propriétaire (0600 sous Unix).
fn create_private(path: &Path) -> std::io::Result<File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

fn add_entry<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    data: &[u8],
    password: Option<&str>,
) -> Result<()> {
    let base = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    // `let` : une expression `match` en début d'instruction ne peut pas être suivie d'un appel de méthode.
    let started = match password {
        Some(p) => zip.start_file(name, base.with_aes_encryption(zip::AesMode::Aes256, p)),
        None => zip.start_file(name, base),
    };
    started.with_context(|| format!("ajout de {name} à l'archive"))?;
    zip.write_all(data)
        .with_context(|| format!("écriture de {name}"))?;
    Ok(())
}

/// Retire des données d'interface tout ce qui ressemble à un secret : le frontend ne
/// les inclut déjà pas, ceci est une seconde barrière. Renvoie un JSON indenté.
pub fn scrub_frontend_json(raw: &str) -> String {
    const SECRET_FIELDS: &[&str] = &["bearer", "qdrant_api_key", "piste_client_secret"];
    let mut v: Value = serde_json::from_str(raw).unwrap_or_else(|_| json!({ "brut": raw }));
    if let Some(obj) = v.as_object_mut() {
        obj.retain(|k, _| !k.starts_with("juridix_secret_"));
        for val in obj.values_mut() {
            // Les valeurs du localStorage sont des chaînes ; `juridix_settings` contient du JSON.
            if let Some(s) = val.as_str() {
                if let Ok(mut inner) = serde_json::from_str::<Value>(s) {
                    if let Some(o) = inner.as_object_mut() {
                        let before = o.len();
                        o.retain(|k, _| !SECRET_FIELDS.contains(&k.as_str()));
                        if o.len() != before {
                            *val = Value::String(inner.to_string());
                        }
                    }
                }
            }
        }
    }
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string())
}

/// Texte lisible d'un message de conversation (chaîne, liste de parties, ou autre).
fn message_text(m: &Value) -> String {
    match m.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                p.get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| p.as_str().map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// Rendu Markdown lisible des conversations (en plus du JSON brut).
pub fn conversations_markdown(rows: &[Value], date: &str) -> String {
    let mut out = format!("# Conversations — export du {date}\n");
    if rows.is_empty() {
        out.push_str("\nAucune conversation.\n");
    }
    for c in rows {
        let titre = c
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("(sans titre)");
        let cree = c.get("created_at").and_then(Value::as_str).unwrap_or("?");
        let modifie = c.get("updated_at").and_then(Value::as_str).unwrap_or("?");
        out.push_str(&format!(
            "\n## {titre}\n\n*Créée le {cree} · modifiée le {modifie}*\n"
        ));
        let messages: Vec<Value> = match c.get("messages") {
            Some(Value::Array(a)) => a.clone(),
            Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_default(),
            _ => Vec::new(),
        };
        for m in &messages {
            let role = match m.get("role").and_then(Value::as_str) {
                Some("user") => "Vous",
                Some("assistant") => "Assistant",
                Some(autre) => autre,
                None => "?",
            };
            out.push_str(&format!("\n**{role}** :\n\n{}\n", message_text(m)));
        }
    }
    out
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn readme(date: &str, chiffree: bool) -> String {
    let protection = if chiffree {
        "Cette archive est chiffrée (AES-256) : le mot de passe est nécessaire pour l'ouvrir. \
         L'explorateur de fichiers de Windows ne sait pas ouvrir les archives AES ; utilisez 7-Zip, \
         WinRAR, Keka ou un outil équivalent."
    } else {
        "Cette archive n'est PAS chiffrée : elle contient vos données en clair. Conservez-la dans \
         un emplacement protégé et supprimez-la quand elle n'est plus nécessaire."
    };
    format!(
        "Export des données Juridix — {date}\n\
         Version de l'application : {version}\n\n\
         {protection}\n\n\
         Contenu :\n\
         - conversations_lisibles.md : vos conversations, au format lisible ;\n\
         - donnees/<table>.json : le contenu brut de chaque table de la base (conversations, \
           métadonnées de collections, veilles et leurs résultats) ;\n\
         - interface/donnees_interface.json : données de l'interface (réglages, dossiers, favoris, \
           annotations, historique) ;\n\
         - prompts.yml : prompts des espaces de travail.\n\n\
         Non inclus : vos identifiants (jeton LLM, clé Qdrant, secret PISTE) et la clé de \
         chiffrement de la base, qui restent dans le trousseau du système ; les documents \
         indexés dans vos collections Qdrant distantes ; les serveurs MCP installés.\n",
        version = env!("CARGO_PKG_VERSION"),
    )
}

/// Écrit l'archive d'export. Fonction bloquante (à appeler via `spawn_blocking`).
pub fn write_export_zip(input: &ExportInput) -> Result<ExportReport> {
    validate_dest(input.dest)?;
    let password = input.password.map(str::trim).filter(|p| !p.is_empty());
    let date = chrono::Local::now().format("%d/%m/%Y %H:%M").to_string();

    let file = create_private(input.dest)
        .with_context(|| format!("création de {}", input.dest.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let mut fichiers = 0usize;

    add_entry(
        &mut zip,
        "LISEZMOI.txt",
        readme(&date, password.is_some()).as_bytes(),
        password,
    )?;
    fichiers += 1;

    let mut nb_conversations = 0usize;
    for (table, rows) in input.tables {
        let json = serde_json::to_string_pretty(rows).context("sérialisation d'une table")?;
        add_entry(
            &mut zip,
            &format!("donnees/{}.json", sanitize_name(table)),
            json.as_bytes(),
            password,
        )?;
        fichiers += 1;
        if table.as_str() == "conversations" {
            nb_conversations = rows.len();
            add_entry(
                &mut zip,
                "conversations_lisibles.md",
                conversations_markdown(rows, &date).as_bytes(),
                password,
            )?;
            fichiers += 1;
        }
    }

    add_entry(
        &mut zip,
        "interface/donnees_interface.json",
        scrub_frontend_json(input.frontend_json).as_bytes(),
        password,
    )?;
    fichiers += 1;

    if let Some(p) = &input.prompts_yaml {
        add_entry(&mut zip, PROMPTS_FILE, p.as_bytes(), password)?;
        fichiers += 1;
    }

    zip.finish().context("finalisation de l'archive")?;
    info!(
        "Export des données écrit : {} ({} fichiers)",
        input.dest.display(),
        fichiers
    );
    Ok(ExportReport {
        path: input.dest.display().to_string(),
        conversations: nb_conversations,
        fichiers,
        chiffree: password.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::Mutex;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new() -> Self {
            let d = std::env::temp_dir().join(format!("juridix-rgpd-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&d).unwrap();
            Self(d)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn touch(p: &Path, content: &str) {
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn secure_delete_removes_the_file_and_tolerates_a_missing_one() {
        let d = TmpDir::new();
        let f = d.0.join("secret.bin");
        touch(&f, &"x".repeat(200_000));
        secure_delete_file(&f).unwrap();
        assert!(!f.exists());
        secure_delete_file(&f).unwrap(); // déjà absent : pas une erreur
    }

    #[test]
    fn backup_status_and_deletion() {
        let d = TmpDir::new();
        let paths = DataPaths::new(d.0.clone());
        assert!(!backup_status(&paths).exists);

        touch(&paths.backup_path(), "ancienne base en clair");
        let s = backup_status(&paths);
        assert!(s.exists);
        assert_eq!(s.size_bytes, Some("ancienne base en clair".len() as u64));
        assert!(s
            .path
            .unwrap()
            .ends_with("conversations.db.pre-encryption.bak"));

        delete_backup(&paths).unwrap();
        assert!(!backup_status(&paths).exists);
        delete_backup(&paths).unwrap(); // idempotent
    }

    #[test]
    fn without_marker_nothing_is_touched() {
        let d = TmpDir::new();
        let paths = DataPaths::new(d.0.clone());
        touch(&paths.db_path(), "base");
        let appels = Mutex::new(Vec::<String>::new());
        let r = run_pending_wipe_with(&paths, &|k| {
            appels.lock().unwrap().push(k.to_string());
            Ok(())
        })
        .unwrap();
        assert!(!r);
        assert!(paths.db_path().exists());
        assert!(
            appels.lock().unwrap().is_empty(),
            "le trousseau ne doit pas être touché sans marqueur"
        );
    }

    #[test]
    fn pending_wipe_destroys_user_files_keychain_entries_and_marker_only() {
        let d = TmpDir::new();
        let paths = DataPaths::new(d.0.clone());
        for p in paths.user_files() {
            touch(&p, "donnée");
        }
        std::fs::create_dir_all(d.0.join("mcp-servers")).unwrap();
        touch(&d.0.join("mcp-servers").join("manifest.json"), "{}");
        touch(&d.0.join("autre.txt"), "à conserver");
        schedule_wipe(&paths).unwrap();
        assert!(paths.marker_path().exists());

        let supprimes = Mutex::new(Vec::<String>::new());
        let r = run_pending_wipe_with(&paths, &|k| {
            supprimes.lock().unwrap().push(k.to_string());
            Ok(())
        })
        .unwrap();
        assert!(r);

        for p in paths.user_files() {
            assert!(!p.exists(), "{} aurait dû être supprimé", p.display());
        }
        assert!(!paths.marker_path().exists());
        assert!(
            d.0.join("autre.txt").exists(),
            "les fichiers étrangers sont conservés"
        );
        assert!(
            d.0.join("mcp-servers").join("manifest.json").exists(),
            "les serveurs MCP installés sont conservés"
        );

        let cles = supprimes.lock().unwrap();
        assert!(cles.contains(&crate::secrets::DB_KEY_NAME.to_string()));
        for k in crate::secrets::FRONTEND_ALLOWED_KEYS {
            assert!(
                cles.contains(&k.to_string()),
                "{k} doit être supprimé du trousseau"
            );
        }

        // Idempotent : plus de marqueur, plus rien à faire.
        assert!(!run_pending_wipe_with(&paths, &|_| Ok(())).unwrap());
    }

    #[test]
    fn a_failing_keychain_does_not_block_the_wipe() {
        let d = TmpDir::new();
        let paths = DataPaths::new(d.0.clone());
        touch(&paths.db_path(), "base");
        schedule_wipe(&paths).unwrap();
        let r = run_pending_wipe_with(&paths, &|_| anyhow::bail!("trousseau verrouillé")).unwrap();
        assert!(r);
        assert!(!paths.db_path().exists());
        assert!(!paths.marker_path().exists());
    }

    #[test]
    fn scrub_removes_secrets_from_frontend_data() {
        let raw = json!({
            "juridix_settings": json!({ "model": "m", "bearer": "SECRET1", "qdrant_api_key": "SECRET2", "piste_client_secret": "SECRET3" }).to_string(),
            "juridix_secret_bearer": "SECRET4",
            "juridique_favorites": "[1,2]",
        })
        .to_string();
        let out = scrub_frontend_json(&raw);
        for s in [
            "SECRET1",
            "SECRET2",
            "SECRET3",
            "SECRET4",
            "juridix_secret_",
        ] {
            assert!(!out.contains(s), "{s} ne doit pas figurer dans l'export");
        }
        assert!(out.contains("juridique_favorites"));
        assert!(out.contains("model"));
        // Entrée non JSON : conservée telle quelle, sans panique.
        assert!(scrub_frontend_json("pas du json").contains("pas du json"));
    }

    #[test]
    fn markdown_rendering_of_conversations() {
        let rows = vec![json!({
            "title": "Délai d'appel",
            "created_at": "2026-10-01T10:00:00Z",
            "updated_at": "2026-10-01T10:05:00Z",
            "messages": [
                { "role": "user", "content": "Quel est le délai ?" },
                { "role": "assistant", "content": [{ "type": "text", "text": "Un mois." }] },
            ],
        })];
        let md = conversations_markdown(&rows, "07/10/2026");
        assert!(md.contains("## Délai d'appel"));
        assert!(md.contains("**Vous** :\n\nQuel est le délai ?"));
        assert!(md.contains("**Assistant** :\n\nUn mois."));
        assert!(conversations_markdown(&[], "x").contains("Aucune conversation"));
    }

    fn sample_input<'a>(
        dest: &'a Path,
        tables: &'a [(String, Vec<Value>)],
        password: Option<&'a str>,
    ) -> ExportInput<'a> {
        ExportInput {
            dest,
            tables,
            prompts_yaml: Some("espace: test".to_string()),
            frontend_json: r#"{"juridique_dossiers":"[]","juridix_secret_bearer":"SECRET"}"#,
            password,
        }
    }

    fn sample_tables() -> Vec<(String, Vec<Value>)> {
        vec![
            (
                "conversations".into(),
                vec![
                    json!({ "id": "c1", "title": "T", "messages": [{ "role": "user", "content": "Bonjour" }] }),
                ],
            ),
            (
                "veilles".into(),
                vec![json!({ "id": 1, "nom": "Télétravail" })],
            ),
        ]
    }

    fn read_entry(
        archive: &mut zip::ZipArchive<File>,
        name: &str,
        password: Option<&[u8]>,
    ) -> zip::result::ZipResult<String> {
        let idx = archive.index_for_name(name).expect("entrée absente");
        let mut f = match password {
            Some(p) => archive.by_index_decrypt(idx, p)?,
            None => archive.by_index(idx)?,
        };
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        Ok(s)
    }

    #[test]
    fn export_roundtrip_without_password() {
        let d = TmpDir::new();
        let dest = d.0.join("export.zip");
        let tables = sample_tables();
        let rep = write_export_zip(&sample_input(&dest, &tables, None)).unwrap();
        assert_eq!(rep.conversations, 1);
        assert!(!rep.chiffree);

        let mut a = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let noms: Vec<String> = (0..a.len())
            .map(|i| a.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        for attendu in [
            "LISEZMOI.txt",
            "donnees/conversations.json",
            "donnees/veilles.json",
            "conversations_lisibles.md",
            "interface/donnees_interface.json",
            "prompts.yml",
        ] {
            assert!(
                noms.iter().any(|n| n == attendu),
                "{attendu} manquant dans {noms:?}"
            );
        }
        assert!(read_entry(&mut a, "donnees/conversations.json", None)
            .unwrap()
            .contains("Bonjour"));
        assert!(read_entry(&mut a, "conversations_lisibles.md", None)
            .unwrap()
            .contains("Bonjour"));
        let ui = read_entry(&mut a, "interface/donnees_interface.json", None).unwrap();
        assert!(ui.contains("juridique_dossiers"));
        assert!(!ui.contains("SECRET"), "aucun secret dans l'export");
        assert!(read_entry(&mut a, "LISEZMOI.txt", None)
            .unwrap()
            .contains("PAS chiffrée"));
    }

    #[test]
    fn export_with_password_is_aes_encrypted() {
        let d = TmpDir::new();
        let dest = d.0.join("export-chiffre.zip");
        let tables = sample_tables();
        let rep = write_export_zip(&sample_input(&dest, &tables, Some("mot de passe"))).unwrap();
        assert!(rep.chiffree);

        let mut a = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        assert!(
            read_entry(&mut a, "donnees/conversations.json", None).is_err(),
            "illisible sans mot de passe"
        );
        let clair =
            read_entry(&mut a, "donnees/conversations.json", Some(b"mot de passe")).unwrap();
        assert!(clair.contains("Bonjour"));
        let brut = std::fs::read(&dest).unwrap();
        assert!(
            !brut.windows(7).any(|w| w == b"Bonjour"),
            "le contenu ne doit pas apparaître en clair dans le fichier"
        );
    }

    #[test]
    fn export_rejects_bad_destinations() {
        let d = TmpDir::new();
        let tables = sample_tables();
        assert!(
            write_export_zip(&sample_input(&d.0.join("export.txt"), &tables, None)).is_err(),
            "extension .zip obligatoire"
        );
        assert!(
            write_export_zip(&sample_input(
                &d.0.join("absent").join("export.zip"),
                &tables,
                None
            ))
            .is_err(),
            "dossier inexistant"
        );
        assert!(
            write_export_zip(&sample_input(&d.0, &tables, None)).is_err(),
            "un dossier n'est pas une destination"
        );
    }
}
