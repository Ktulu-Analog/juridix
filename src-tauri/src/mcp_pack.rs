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

// Import des serveurs MCP officiels (pack `demeter-mcp-pack`) — adapté de Demeter.
//
// Même mécanisme que Demeter : un dossier de serveurs (un sous-dossier par
// serveur, avec `manifest.json` + un binaire par plateforme), alimenté depuis les
// releases GitHub du pack après vérification SHA-256. Les serveurs sont ensuite
// lancés en STDIO par `mcp_stdio.rs`.
//
// Différence avec Demeter, voulue : Juridix ne gère que les serveurs dont l'`id`
// figure dans la liste fournie par l'appelant (Légifrance et Judilibre). Les
// autres serveurs du pack (Grist, Image, Système de fichiers…) sont ignorés à
// toutes les étapes : jamais téléchargés, jamais installés, jamais découverts
// même s'ils se trouvent dans le dossier.
//
// Chaque release publie, en plus de l'archive complète `mcp-pack-<v>.zip`, un zip
// par serveur `<id>-<v>.zip` avec sa ligne dans `checksums.txt` : on ne télécharge
// que ceux-là (≈ 6 Mo chacun au lieu de ≈ 28 Mo pour le pack entier).
//
// Intégrité, en deux niveaux :
//  1. SHA-256 de chaque zip, lu dans `checksums.txt` de la même release : protège d'un
//     téléchargement corrompu ou tronqué, PAS d'une release compromise (qui pourrait
//     modifier l'archive ET son empreinte) ;
//  2. SIGNATURE minisign de `checksums.txt` (`checksums.txt.minisig`), vérifiée avec la clé
//     publique du fichier `src-tauri/mcp-pack.pub` (embarquée à la compilation) : sans la clé
//     privée, une release altérée ne peut plus être installée, tout en laissant les mises à
//     jour indépendantes des versions de Juridix (on ne fige donc pas les empreintes dans le
//     code). Le commentaire signé doit contenir `version:<version>` : c'est le format produit
//     par `sign-mcp-pack.sh` du dépôt Demeter, MÊME clé et MÊME signature pour Demeter et
//     Juridix. Il lie la signature à la release (contre le rejeu d'une ancienne release), et un
//     retour à une version antérieure à celle installée est refusé.
//
// Une build de release ne compile que si `mcp-pack.pub` contient une clé (voir plus bas) :
// il n'existe donc pas de binaire publié en « mode de transition » (SHA-256 seul).

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use futures::future::BoxFuture;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Dépôt GitHub du pack (surchargeable : `JURIDIX_MCP_PACK_REPO`).
pub const PACK_REPO: &str = "Ktulu-Analog/demeter-mcp-pack";

/// Plafonds anti-abus (une release légitime pèse ≈ 6 Mo / ≈ 16 Mo décompressée).
const MAX_DOWNLOAD_BYTES: u64 = 150 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 250 * 1024 * 1024;
const MAX_ENTRIES: usize = 200;
const MAX_CHECKSUMS_BYTES: u64 = 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 4096;

/// Clé publique minisign des releases du pack : fichier `src-tauri/mcp-pack.pub`, le MÊME que
/// celui de Demeter (une seule paire de clés, une seule signature par release du pack).
/// Format minisign : ligne `untrusted comment: …` puis la clé en base64 (« RWQ… »).
const EMBEDDED_PUBLIC_KEY_FILE: &str = include_str!("../mcp-pack.pub");

// AUDIT E-01 : en build de RELEASE, la compilation échoue si `mcp-pack.pub` ne contient pas de
// clé (ligne commençant par « RW »). Il n'existe donc plus de binaire publié fonctionnant en
// « mode de transition » (SHA-256 seul, que pourrait forger quiconque contrôle la release
// GitHub). La validité cryptographique de la clé est vérifiée par un test (`cargo test`).
#[cfg(not(debug_assertions))]
const _REQUIRE_PACK_PUBKEY: () = {
    if !has_key_line(EMBEDDED_PUBLIC_KEY_FILE) {
        panic!(
            "src-tauri/mcp-pack.pub ne contient pas de clé publique minisign (ligne « RWQ… ») : obligatoire pour une build de release"
        );
    }
};

/// `true` si une ligne du fichier commence par « RW » (clé publique minisign). `const fn` pour
/// servir au contrôle de compilation ci-dessus.
#[allow(dead_code)]
const fn has_key_line(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if (i == 0 || b[i - 1] == b'\n') && b[i] == b'R' && b[i + 1] == b'W' {
            return true;
        }
        i += 1;
    }
    false
}

/// Extrait la clé publique (base64) d'un fichier `.pub` minisign : on ignore les lignes vides,
/// les commentaires `#` et la ligne `untrusted comment:`.
fn parse_public_key_file(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("untrusted comment:"))
        .map(str::to_string)
}

fn configured_public_key() -> Option<String> {
    parse_public_key_file(EMBEDDED_PUBLIC_KEY_FILE)
}

// ── Manifest & découverte ─────────────────────────────────────────────────────

/// `manifest.json` d'un serveur. Seuls les champs utiles à Juridix sont lus ;
/// `params` (paramètres saisis dans Demeter) est ignoré : les identifiants PISTE
/// viennent des réglages de Juridix (voir `mcp_stdio::official_specs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Binaire par plateforme (`linux-x64`, `macos-arm64`, `macos-x64`, `windows-x64`).
    pub bin: HashMap<String, String>,
    /// Arguments de lancement (`["--transport", "stdio"]`).
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredServer {
    #[serde(flatten)]
    pub manifest: Manifest,
    #[serde(skip)]
    pub dir: PathBuf,
    /// `false` si le manifest ne fournit aucun binaire pour cette plateforme.
    pub available: bool,
}

/// « linux-x64 », « macos-arm64 », « windows-x64 »…
pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    format!("{os}-{arch}")
}

/// Un `id` de manifest sert de nom de dossier : on n'accepte que des identifiants sûrs.
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Lit le manifest d'un dossier de serveur ; `expected_id` doit correspondre
/// à l'`id` déclaré (un dossier ne peut pas se faire passer pour un autre serveur).
pub fn read_manifest(server_dir: &Path, expected_id: &str) -> Result<Manifest, String> {
    let path = server_dir.join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("manifest.json illisible ({}) : {e}", path.display()))?;
    let m: Manifest =
        serde_json::from_str(&text).map_err(|e| format!("manifest.json invalide : {e}"))?;
    if m.id != expected_id {
        return Err(format!(
            "manifest.json : id « {} » au lieu de « {expected_id} »",
            m.id
        ));
    }
    Ok(m)
}

/// Serveurs de `ids` présents dans `dir`. Tout autre sous-dossier est ignoré.
pub fn discover(dir: &Path, ids: &[String]) -> Vec<DiscoveredServer> {
    let mut found = Vec::new();
    for id in ids {
        if !safe_id(id) {
            continue;
        }
        let server_dir = dir.join(id);
        if !server_dir.is_dir() {
            continue;
        }
        match read_manifest(&server_dir, id) {
            Ok(manifest) => {
                let available = manifest.bin.contains_key(&platform_key());
                found.push(DiscoveredServer {
                    manifest,
                    dir: server_dir,
                    available,
                });
            }
            Err(e) => warn!("MCP: {id} ignoré — {e}"),
        }
    }
    found
}

/// Chemin du binaire pour la plateforme courante. Refuse tout chemin absolu ou
/// remontant (`..`) : un manifest ne doit pas pouvoir désigner un exécutable
/// extérieur à son dossier.
pub fn resolve_bin(server: &DiscoveredServer) -> Result<PathBuf, String> {
    let rel = server
        .manifest
        .bin
        .get(&platform_key())
        .ok_or_else(|| format!("aucun binaire pour {} dans le manifest", platform_key()))?;
    if !is_safe_relative(rel) {
        return Err(format!(
            "chemin de binaire non autorisé dans le manifest : {rel}"
        ));
    }
    Ok(server.dir.join(rel))
}

fn is_safe_relative(p: &str) -> bool {
    let path = Path::new(p);
    // `\` et `:` sont refusés sur toutes les plateformes : sous Linux « C:\x » serait un
    // nom de fichier banal, sous Windows un chemin absolu — la validation ne doit pas
    // dépendre de la machine. Les manifests du pack n'emploient que des `/`.
    !p.is_empty()
        && !p.contains('\\')
        && !p.contains(':')
        && !path.is_absolute()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}

// ── Source des releases ───────────────────────────────────────────────────────

/// D'où viennent les releases. Les valeurs par défaut visent GitHub ; les tests
/// pointent vers un serveur local.
#[derive(Debug, Clone)]
pub struct PackSource {
    pub repo: String,
    /// `https://github.com`
    pub web_base: String,
    /// `https://api.github.com`
    pub api_base: String,
    /// Version figée (`v0.3.1`) ; `None` = dernière release.
    pub pinned: Option<String>,
    /// Clé publique minisign : `Some` = la signature de `checksums.txt` est obligatoire.
    pub public_key: Option<String>,
}

impl Default for PackSource {
    fn default() -> Self {
        // Surcharges du dépôt et de la version : réservées aux builds de DÉVELOPPEMENT. En
        // release, l'application ne fait confiance qu'au dépôt officiel et à sa dernière release.
        let overrides = cfg!(debug_assertions);
        let repo = if overrides {
            std::env::var("JURIDIX_MCP_PACK_REPO").unwrap_or_else(|_| PACK_REPO.to_string())
        } else {
            PACK_REPO.to_string()
        };
        let pinned = if overrides {
            std::env::var("JURIDIX_MCP_PACK_VERSION")
                .ok()
                .filter(|v| !v.trim().is_empty())
        } else {
            None
        };
        Self {
            repo,
            web_base: "https://github.com".into(),
            api_base: "https://api.github.com".into(),
            pinned,
            public_key: configured_public_key(),
        }
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; Juridix/1.0)")
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_default()
}

impl PackSource {
    /// Tag de la dernière release (« v0.3.1 »).
    ///
    /// 1. Redirection de `/releases/latest` (pas de quota) ;
    /// 2. à défaut, l'API GitHub — limitée à 60 requêtes/h par IP, d'où ce second choix :
    ///    derrière un proxy d'entreprise partagé, le quota est vite épuisé.
    pub async fn latest_tag(&self) -> Result<String, String> {
        if let Some(t) = &self.pinned {
            return Ok(t.clone());
        }
        let no_redirect = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (compatible; Juridix/1.0)")
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        let url = format!("{}/{}/releases/latest", self.web_base, self.repo);
        if let Ok(resp) = no_redirect.get(&url).send().await {
            if resp.status().is_redirection() {
                if let Some(tag) = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|loc| loc.rsplit_once("/tag/").map(|(_, t)| t.trim().to_string()))
                    .filter(|t| safe_tag(t))
                {
                    return Ok(tag);
                }
            }
        }
        #[derive(Deserialize)]
        struct Rel {
            tag_name: String,
        }
        let api = format!("{}/repos/{}/releases/latest", self.api_base, self.repo);
        let resp = no_redirect
            .get(&api)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| format!("GitHub injoignable : {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "GitHub a répondu {} (quota dépassé ou dépôt introuvable)",
                resp.status()
            ));
        }
        let rel: Rel = resp.json().await.map_err(|e| e.to_string())?;
        if !safe_tag(&rel.tag_name) {
            return Err(format!("tag de release inattendu : {}", rel.tag_name));
        }
        Ok(rel.tag_name)
    }

    fn asset_url(&self, tag: &str, asset: &str) -> String {
        format!(
            "{}/{}/releases/download/{}/{}",
            self.web_base, self.repo, tag, asset
        )
    }
}

/// Un tag est inséré dans une URL et un nom de fichier : caractères sûrs uniquement.
fn safe_tag(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 40
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

// ── Mise à jour : état ────────────────────────────────────────────────────────

/// Version du pack installée dans `dir` (fichier `.pack-version`).
pub fn installed_version(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(".pack-version"))
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub update_available: bool,
    /// Pourquoi `latest` est inconnu (hors ligne, quota…), pour l'afficher.
    pub error: Option<String>,
    /// `true` : une clé de signature est compilée, la signature des releases est obligatoire.
    pub signature_enforced: bool,
}

/// Ne fait jamais échouer l'appelant : hors ligne, `latest` reste `None`.
pub async fn check_update(src: &PackSource, installed: Option<String>) -> UpdateStatus {
    match src.latest_tag().await {
        Ok(tag) => {
            let latest = tag.trim_start_matches('v').to_string();
            let update_available = installed.as_deref() != Some(latest.as_str());
            UpdateStatus {
                installed,
                latest: Some(latest),
                update_available,
                error: None,
                signature_enforced: src.public_key.is_some(),
            }
        }
        Err(e) => UpdateStatus {
            installed,
            latest: None,
            update_available: false,
            error: Some(e),
            signature_enforced: src.public_key.is_some(),
        },
    }
}

// ── Installation ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ServerInstall {
    pub id: String,
    pub installed: bool,
    pub error: Option<String>,
}

/// Résultat du contrôle de signature de la release installée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureStatus {
    /// `checksums.txt` signé par la clé publique compilée dans Juridix.
    Verified,
    /// Aucune clé compilée : seul le SHA-256 de la release a été contrôlé.
    NotConfigured,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallReport {
    pub version: String,
    pub servers: Vec<ServerInstall>,
    pub signature: SignatureStatus,
}

/// Vérifie la signature minisign `signature_text` de `data` (`checksums.txt`) avec la clé
/// publique `public_key_b64`. Seules les signatures pré-hachées (format par défaut de minisign)
/// sont acceptées, et le commentaire signé doit contenir `version:<version>` (format de
/// `sign-mcp-pack.sh` de Demeter) : une signature légitime d'une AUTRE release (rejeu) est
/// refusée. `version` est le tag sans « v » initial (« v0.3.3 » → « 0.3.3 »).
pub fn verify_minisign(
    public_key_b64: &str,
    data: &[u8],
    signature_text: &str,
    version: &str,
) -> Result<(), String> {
    let pk = PublicKey::from_base64(public_key_b64.trim())
        .map_err(|e| format!("clé publique de signature invalide : {e}"))?;
    let sig =
        Signature::decode(signature_text).map_err(|e| format!("signature illisible : {e}"))?;
    pk.verify(data, &sig, false)
        .map_err(|e| format!("signature INVALIDE : {e}"))?;
    let expected = format!("version:{version}");
    if !sig
        .trusted_comment()
        .split_whitespace()
        .any(|w| w == expected)
    {
        return Err(format!(
            "la signature ne désigne pas la release {version} (commentaire signé : « {} », attendu « {expected} ») — rejeu d'une autre release ?",
            sig.trusted_comment()
        ));
    }
    Ok(())
}

/// Contrôle de signature de `checksums.txt` avant tout usage de son contenu.
async fn verify_release_signature(
    src: &PackSource,
    client: &reqwest::Client,
    tag: &str,
    version: &str,
    checksums: &[u8],
) -> Result<SignatureStatus, String> {
    let Some(public_key) = src.public_key.as_deref() else {
        warn!(
            "MCP: aucune clé de signature (mcp-pack.pub) — intégrité du pack limitée au SHA-256 de la release \
             (build de développement uniquement)"
        );
        return Ok(SignatureStatus::NotConfigured);
    };
    let sig = download(client, &src.asset_url(tag, "checksums.txt.minisig"), MAX_SIGNATURE_BYTES)
        .await
        .map_err(|e| format!("signature de la release {tag} introuvable (checksums.txt.minisig) — installation refusée : {e}"))?;
    let sig = String::from_utf8(sig)
        .map_err(|_| "signature illisible (UTF-8) — installation refusée".to_string())?;
    verify_minisign(public_key, checksums, &sig, version)
        .map_err(|e| format!("{e} — installation refusée"))?;
    info!("MCP: signature de la release {tag} vérifiée");
    Ok(SignatureStatus::Verified)
}

/// Composantes numériques d'une version (« v1.2.3-rc1 » → [1, 2, 3]).
fn version_parts(v: &str) -> Vec<u64> {
    let core: String = v
        .trim_start_matches('v')
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    core.split('.')
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// `a < b` pour deux versions numériques (composantes manquantes = 0).
fn version_lt(a: &str, b: &str) -> bool {
    let (mut x, mut y) = (version_parts(a), version_parts(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x < y
}

/// Hachage SHA-256 attendu pour `filename` dans `checksums.txt`
/// (lignes « <hex>  <nom> », avec ou sans `*` ni `./`).
pub fn expected_hash(checksums: &str, filename: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let mut it = line.split_whitespace();
        let hash = it.next()?;
        let name = it.next()?.trim_start_matches('*').trim_start_matches("./");
        (name == filename && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Télécharge `url` en bornant la taille (en-tête ET flux : un serveur peut mentir sur la longueur).
async fn download(client: &reqwest::Client, url: &str, max: u64) -> Result<Vec<u8>, String> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("téléchargement impossible : {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("téléchargement échoué ({}) : {url}", resp.status()));
    }
    if resp.content_length().is_some_and(|n| n > max) {
        return Err(format!(
            "fichier trop volumineux (> {} Mo)",
            max / 1024 / 1024
        ));
    }
    let mut out: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("téléchargement interrompu : {e}"))?
    {
        if out.len() as u64 + chunk.len() as u64 > max {
            return Err(format!(
                "fichier trop volumineux (> {} Mo)",
                max / 1024 / 1024
            ));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Extrait `bytes` (zip d'un serveur) dans `staging`, en n'acceptant que des
/// entrées de la forme `<id>/...` sans chemin dangereux.
fn extract_server_zip(bytes: &[u8], id: &str, staging: &Path) -> Result<(), String> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("archive illisible : {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("archive suspecte ({} entrées)", zip.len()));
    }
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| format!("archive illisible : {e}"))?;
        // `enclosed_name` rejette `..` et les chemins absolus (« zip slip »).
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| format!("chemin dangereux dans l'archive : {}", entry.name()))?;
        let mut comps = rel.components();
        match comps.next() {
            Some(Component::Normal(first)) if first == id => {}
            _ => {
                return Err(format!(
                    "entrée hors de « {id}/ » dans l'archive : {}",
                    entry.name()
                ))
            }
        }
        let inner: PathBuf = comps.collect();
        let dest = staging.join(&inner);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            continue;
        }
        if entry.is_symlink() {
            return Err(format!(
                "lien symbolique refusé dans l'archive : {}",
                entry.name()
            ));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED_BYTES {
            return Err("archive trop volumineuse une fois décompressée".into());
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut data = Vec::with_capacity(entry.size().min(64 * 1024 * 1024) as usize);
        entry
            .by_ref()
            .take(MAX_UNPACKED_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| format!("décompression : {e}"))?;
        if data.len() as u64 != entry.size() {
            return Err(format!("taille incohérente pour {}", entry.name()));
        }
        let mut f = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
        f.write_all(&data).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Binaires exécutables ; le reste en lecture/écriture simple.
            let exec = inner.starts_with("bin");
            let _ = std::fs::set_permissions(
                &dest,
                std::fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }),
            );
        }
    }
    Ok(())
}

/// Installe un serveur depuis son zip déjà vérifié : extraction en zone temporaire,
/// validation du manifest et du binaire, puis remplacement du dossier existant.
/// En cas d'échec, l'ancienne version reste en place.
fn install_one(dir: &Path, id: &str, zip_bytes: &[u8]) -> Result<(), String> {
    let staging = dir.join(format!(".staging-{id}"));
    let backup = dir.join(format!(".old-{id}"));
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::create_dir_all(&staging).map_err(|e| format!("dossier temporaire : {e}"))?;

    let result = (|| {
        extract_server_zip(zip_bytes, id, &staging)?;
        let manifest = read_manifest(&staging, id)?;
        let rel = manifest.bin.get(&platform_key()).ok_or_else(|| {
            let dispo: Vec<&str> = manifest.bin.keys().map(String::as_str).collect();
            format!(
                "cette version ne fournit pas de binaire pour {} (disponibles : {})",
                platform_key(),
                dispo.join(", ")
            )
        })?;
        if !is_safe_relative(rel) {
            return Err(format!(
                "chemin de binaire non autorisé dans le manifest : {rel}"
            ));
        }
        if !staging.join(rel).is_file() {
            return Err(format!("binaire annoncé mais absent de l'archive : {rel}"));
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    let target = dir.join(id);
    let had_old = target.exists();
    if had_old {
        std::fs::rename(&target, &backup).map_err(|e| {
            format!("remplacement impossible (serveur en cours d'utilisation ?) : {e}")
        })?;
    }
    if let Err(e) = std::fs::rename(&staging, &target) {
        if had_old {
            let _ = std::fs::rename(&backup, &target); // retour arrière
        }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("installation impossible : {e}"));
    }
    let _ = std::fs::remove_dir_all(&backup);
    // AUDIT E-01 : empreinte du binaire installé, conservée dans le trousseau système
    // (hors du dossier des serveurs, inscriptible par tout processus de l'utilisateur).
    record_bin_hash(&target, id)?;
    Ok(())
}

fn bin_hash_key(id: &str) -> String {
    format!("mcp_bin_sha256_{id}")
}

/// Chemin du binaire de `id` pour la plateforme courante, d'après son manifest.
#[cfg_attr(test, allow(dead_code))]
fn installed_bin_path(server_dir: &Path, id: &str) -> Result<PathBuf, String> {
    let manifest = read_manifest(server_dir, id)?;
    let rel = manifest
        .bin
        .get(&platform_key())
        .ok_or_else(|| format!("aucun binaire pour {} dans le manifest", platform_key()))?;
    if !is_safe_relative(rel) {
        return Err(format!(
            "chemin de binaire non autorisé dans le manifest : {rel}"
        ));
    }
    Ok(server_dir.join(rel))
}

#[cfg(not(test))]
fn record_bin_hash(server_dir: &Path, id: &str) -> Result<(), String> {
    let bin = installed_bin_path(server_dir, id)?;
    let bytes = std::fs::read(&bin).map_err(|e| format!("lecture du binaire installé : {e}"))?;
    crate::secrets::store(&bin_hash_key(id), &sha256_hex(&bytes))
        .map_err(|e| format!("enregistrement de l'empreinte du binaire (trousseau) : {e:#}"))
}

#[cfg(test)]
fn record_bin_hash(_server_dir: &Path, _id: &str) -> Result<(), String> {
    Ok(())
}

/// À appeler AVANT chaque lancement d'un serveur : le binaire sur disque doit être celui qui a
/// été installé (et vérifié par signature + SHA-256). Échec fermé : une empreinte absente
/// (installation antérieure à ce contrôle) ou différente interdit le lancement.
#[cfg(not(test))]
pub fn verify_bin_integrity(server: &DiscoveredServer, bin: &Path) -> Result<(), String> {
    let id = &server.manifest.id;
    let expected = crate::secrets::read(&bin_hash_key(id))
        .map_err(|e| format!("lecture de l'empreinte du binaire (trousseau) : {e:#}"))?
        .ok_or_else(|| {
            format!("« {id} » : aucune empreinte enregistrée — réimportez le serveur depuis les réglages")
        })?;
    let bytes = std::fs::read(bin).map_err(|e| format!("lecture du binaire : {e}"))?;
    if sha256_hex(&bytes) != expected {
        warn!("MCP: binaire de « {id} » modifié depuis son installation — lancement refusé");
        return Err(format!(
            "« {id} » : le binaire a été modifié depuis son installation — lancement refusé, réimportez le serveur"
        ));
    }
    Ok(())
}

#[cfg(test)]
pub fn verify_bin_integrity(_server: &DiscoveredServer, _bin: &Path) -> Result<(), String> {
    Ok(())
}

/// Importe les serveurs `ids` (et eux seuls) depuis la release `src` dans `dir`.
///
/// `on_replace(id)` est attendu juste avant de remplacer un serveur : l'appelant y
/// arrête le processus en cours (sous Windows, un exécutable en cours d'exécution
/// ne peut pas être remplacé).
pub type OnReplace<'f> = &'f (dyn for<'a> Fn(&'a str) -> BoxFuture<'a, ()> + Sync);

pub async fn install(
    src: &PackSource,
    dir: &Path,
    ids: &[String],
    on_replace: OnReplace<'_>,
) -> Result<InstallReport, String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("dossier des serveurs MCP inaccessible : {e}"))?;
    let tag = src.latest_tag().await?;
    let version = tag.trim_start_matches('v').to_string();

    // Anti-retour arrière : une source compromise ne doit pas pouvoir nous ramener à une ancienne
    // version (légitimement signée mais vulnérable). Seule une version figée explicitement
    // (builds de développement) peut descendre ; sinon, supprimez `.pack-version` pour forcer.
    if src.pinned.is_none() {
        if let Some(current) = installed_version(dir) {
            if version_lt(&version, &current) {
                return Err(format!(
                    "la release {version} est antérieure à la version installée ({current}) : retour arrière refusé"
                ));
            }
        }
    }
    let client = http_client();

    let checksums = download(
        &client,
        &src.asset_url(&tag, "checksums.txt"),
        MAX_CHECKSUMS_BYTES,
    )
    .await?;
    // La signature est vérifiée sur les octets bruts, AVANT toute lecture de leur contenu.
    let signature = verify_release_signature(src, &client, &tag, &version, &checksums).await?;
    let checksums =
        String::from_utf8(checksums).map_err(|e| format!("checksums.txt illisible : {e}"))?;

    let mut servers = Vec::new();
    let mut any_ok = false;
    for id in ids {
        let outcome: Result<(), String> = async {
            if !safe_id(id) {
                return Err("identifiant de serveur invalide".into());
            }
            let asset = format!("{id}-{version}.zip");
            let expected = expected_hash(&checksums, &asset)
                .ok_or_else(|| format!("« {asset} » absent de checksums.txt (release {tag})"))?;
            let bytes = download(&client, &src.asset_url(&tag, &asset), MAX_DOWNLOAD_BYTES).await?;
            let actual = sha256_hex(&bytes);
            if actual != expected {
                return Err(format!(
                    "empreinte SHA-256 invalide pour {asset} (attendue {expected}, reçue {actual}) — fichier corrompu ou altéré, installation annulée"
                ));
            }
            on_replace(id).await;
            install_one(dir, id, &bytes)
        }
        .await;
        match outcome {
            Ok(()) => {
                info!("MCP: « {id} » {version} installé dans {}", dir.display());
                any_ok = true;
                servers.push(ServerInstall {
                    id: id.clone(),
                    installed: true,
                    error: None,
                });
            }
            Err(e) => {
                warn!("MCP: installation de « {id} » échouée — {e}");
                servers.push(ServerInstall {
                    id: id.clone(),
                    installed: false,
                    error: Some(e),
                });
            }
        }
    }
    if any_ok {
        let _ = std::fs::write(dir.join(".pack-version"), &version);
    }
    Ok(InstallReport {
        version,
        servers,
        signature,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::{
        extract::{Path as AxPath, State},
        http::{header, StatusCode},
        response::{IntoResponse, Response},
        routing::get,
        Router,
    };

    // ── Aides ─────────────────────────────────────────────────────────────────

    pub(crate) struct TmpDir(pub PathBuf);
    impl TmpDir {
        pub fn new(tag: &str) -> Self {
            static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir().join(format!(
                "juridix-pack-{tag}-{}-{nanos}-{}",
                std::process::id(),
                N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Zip d'un serveur tel que publié : `<id>/manifest.json` + `<id>/bin/<binaire>`.
    fn server_zip(
        id: &str,
        bin_content: &str,
        manifest_id: Option<&str>,
        with_platform_bin: bool,
    ) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            let bin_rel = format!("bin/{id}-{}", platform_key());
            let bin_map = if with_platform_bin {
                serde_json::json!({ platform_key(): bin_rel })
            } else {
                serde_json::json!({ "plan9-mips": "bin/x" })
            };
            let manifest = serde_json::json!({
                "id": manifest_id.unwrap_or(id), "name": id, "description": "test",
                "bin": bin_map, "args": ["--transport", "stdio"],
            });
            z.add_directory(format!("{id}/"), opts).unwrap();
            z.start_file(format!("{id}/manifest.json"), opts).unwrap();
            z.write_all(manifest.to_string().as_bytes()).unwrap();
            if with_platform_bin {
                z.start_file(format!("{id}/{bin_rel}"), opts).unwrap();
                z.write_all(bin_content.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    fn zip_with_entries(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, content) in entries {
                z.start_file(*name, opts).unwrap();
                z.write_all(content.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[derive(Clone)]
    struct Mock {
        /// nom d'asset → octets
        assets: Arc<HashMap<String, Vec<u8>>>,
        checksums: Arc<String>,
        requested: Arc<Mutex<Vec<String>>>,
        /// `true` : `/releases/latest` répond 404 (force le repli sur l'API).
        no_redirect: bool,
    }

    async fn latest_redirect(State(m): State<Mock>) -> Response {
        m.requested.lock().unwrap().push("latest".into());
        if m.no_redirect {
            return StatusCode::NOT_FOUND.into_response();
        }
        (
            StatusCode::FOUND,
            [(header::LOCATION, "/o/r/releases/tag/v1.2.3")],
        )
            .into_response()
    }
    async fn api_latest(State(m): State<Mock>) -> Response {
        m.requested.lock().unwrap().push("api".into());
        axum::Json(serde_json::json!({ "tag_name": "v1.2.3" })).into_response()
    }
    async fn asset(
        State(m): State<Mock>,
        AxPath((tag, name)): AxPath<(String, String)>,
    ) -> Response {
        m.requested.lock().unwrap().push(name.clone());
        assert_eq!(tag, "v1.2.3");
        if name == "checksums.txt" {
            return m.checksums.as_str().to_string().into_response();
        }
        match m.assets.get(&name) {
            Some(b) => b.clone().into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    async fn spawn_mock(m: Mock) -> PackSource {
        let app = Router::new()
            .route("/o/r/releases/latest", get(latest_redirect))
            .route("/api/repos/o/r/releases/latest", get(api_latest))
            .route("/o/r/releases/download/:tag/:name", get(asset))
            .with_state(m);
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        PackSource {
            repo: "o/r".into(),
            web_base: format!("http://{addr}"),
            api_base: format!("http://{addr}/api"),
            pinned: None,
            public_key: None,
        }
    }

    fn mock_with(assets: Vec<(&str, Vec<u8>)>, no_redirect: bool) -> Mock {
        let mut checks = String::new();
        let mut map = HashMap::new();
        for (name, bytes) in assets {
            checks.push_str(&format!("{}  {}\n", sha256_hex(&bytes), name));
            map.insert(name.to_string(), bytes);
        }
        Mock {
            assets: Arc::new(map),
            checksums: Arc::new(checks),
            requested: Arc::default(),
            no_redirect,
        }
    }

    fn ids() -> Vec<String> {
        vec!["mcp-legifrance".into(), "mcp-judilibre".into()]
    }
    fn noop(_: &str) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    // ── Manifest & découverte ─────────────────────────────────────────────────

    #[test]
    fn platform_key_shape() {
        let k = platform_key();
        assert!(
            ["linux", "macos", "windows"]
                .iter()
                .any(|o| k.starts_with(o))
                && (k.ends_with("x64") || k.ends_with("arm64")),
            "{k}"
        );
    }

    #[test]
    fn safe_paths_and_ids() {
        for ok in ["bin/x", "bin/mcp-legifrance-linux-x64", "a/b/c.exe"] {
            assert!(is_safe_relative(ok), "{ok}");
        }
        for bad in ["", "/etc/passwd", "../x", "bin/../../x", "C:\\x", "./../x"] {
            assert!(!is_safe_relative(bad), "{bad}");
        }
        assert!(safe_id("mcp-legifrance") && !safe_id("../x") && !safe_id("a b") && !safe_id(""));
        assert!(safe_tag("v0.3.1") && !safe_tag("v1/../x") && !safe_tag("") && !safe_tag("a b"));
    }

    #[test]
    fn discover_only_lists_requested_servers_and_rejects_id_mismatch() {
        let d = TmpDir::new("disc");
        // Deux serveurs voulus, un « intrus » (Grist), un dossier au manifest trompeur.
        for (dir, manifest_id) in [
            ("mcp-legifrance", "mcp-legifrance"),
            ("mcp-grist", "mcp-grist"),
            ("mcp-judilibre", "mcp-legifrance"),
        ] {
            let p = d.0.join(dir);
            std::fs::create_dir_all(&p).unwrap();
            let m = serde_json::json!({"id": manifest_id, "name": dir, "bin": {platform_key(): "bin/x"}});
            std::fs::write(p.join("manifest.json"), m.to_string()).unwrap();
        }
        let found = discover(&d.0, &ids());
        let got: Vec<_> = found.iter().map(|s| s.manifest.id.as_str()).collect();
        assert_eq!(
            got,
            ["mcp-legifrance"],
            "Grist ignoré ; mcp-judilibre rejeté (id du manifest ≠ dossier)"
        );
        assert!(found[0].available);
    }

    #[test]
    fn resolve_bin_refuses_escaping_paths() {
        let mk = |rel: &str| DiscoveredServer {
            manifest: Manifest {
                id: "x".into(),
                name: "x".into(),
                description: String::new(),
                bin: HashMap::from([(platform_key(), rel.to_string())]),
                args: vec![],
            },
            dir: PathBuf::from("/srv/mcp/x"),
            available: true,
        };
        assert_eq!(
            resolve_bin(&mk("bin/x")).unwrap(),
            PathBuf::from("/srv/mcp/x/bin/x")
        );
        assert!(resolve_bin(&mk("../../bin/sh")).is_err());
        assert!(resolve_bin(&mk("/bin/sh")).is_err());
    }

    #[test]
    fn checksums_parsing() {
        let h = "a".repeat(64);
        let txt = format!("{h}  mcp-legifrance-1.2.3.zip\n{}  *mcp-judilibre-1.2.3.zip\n{}  ./x.zip\nnot-a-hash  bad.zip\n", "B".repeat(64), "c".repeat(64));
        assert_eq!(expected_hash(&txt, "mcp-legifrance-1.2.3.zip"), Some(h));
        assert_eq!(
            expected_hash(&txt, "mcp-judilibre-1.2.3.zip"),
            Some("b".repeat(64)),
            "casse normalisée, « * » toléré"
        );
        assert_eq!(expected_hash(&txt, "x.zip"), Some("c".repeat(64)));
        assert_eq!(expected_hash(&txt, "bad.zip"), None, "pas un SHA-256");
        assert_eq!(expected_hash(&txt, "absent.zip"), None);
    }

    // ── Installation (serveur GitHub simulé) ──────────────────────────────────

    #[tokio::test]
    async fn installs_only_the_requested_servers_and_never_downloads_the_others() {
        let mock = mock_with(
            vec![
                (
                    "mcp-legifrance-1.2.3.zip",
                    server_zip("mcp-legifrance", "BIN-LEGI", None, true),
                ),
                (
                    "mcp-judilibre-1.2.3.zip",
                    server_zip("mcp-judilibre", "BIN-JUDI", None, true),
                ),
                (
                    "mcp-grist-1.2.3.zip",
                    server_zip("mcp-grist", "BIN-GRIST", None, true),
                ),
                (
                    "mcp-system-1.2.3.zip",
                    server_zip("mcp-system", "BIN-SYS", None, true),
                ),
                ("mcp-pack-1.2.3.zip", vec![0u8; 10]),
            ],
            false,
        );
        let requested = mock.requested.clone();
        let src = spawn_mock(mock).await;
        let d = TmpDir::new("inst");
        let rep = install(&src, &d.0, &ids(), &noop).await.unwrap();

        assert_eq!(rep.version, "1.2.3");
        assert!(rep.servers.iter().all(|s| s.installed), "{rep:?}");
        assert_eq!(installed_version(&d.0).as_deref(), Some("1.2.3"));
        let bin =
            d.0.join("mcp-legifrance")
                .join(format!("bin/mcp-legifrance-{}", platform_key()));
        assert_eq!(std::fs::read_to_string(&bin).unwrap(), "BIN-LEGI");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(&bin).unwrap().permissions().mode() & 0o111,
                0,
                "binaire exécutable"
            );
        }
        assert!(!d.0.join("mcp-grist").exists() && !d.0.join("mcp-system").exists());
        assert!(
            !d.0.join(".staging-mcp-legifrance").exists(),
            "zone temporaire nettoyée"
        );

        let req = requested.lock().unwrap().clone();
        assert!(
            req.iter()
                .all(|r| !r.contains("grist") && !r.contains("system") && !r.contains("pack-")),
            "seuls Légifrance et Judilibre sont téléchargés : {req:?}"
        );
        assert!(
            req.contains(&"mcp-legifrance-1.2.3.zip".to_string())
                && req.contains(&"mcp-judilibre-1.2.3.zip".to_string())
        );
        // Découverte ensuite possible, Grist absent.
        assert_eq!(discover(&d.0, &ids()).len(), 2);
    }

    #[tokio::test]
    async fn tag_lookup_falls_back_to_the_api_when_the_redirect_fails() {
        let mock = mock_with(
            vec![(
                "mcp-legifrance-1.2.3.zip",
                server_zip("mcp-legifrance", "B", None, true),
            )],
            true,
        );
        let requested = mock.requested.clone();
        let src = spawn_mock(mock).await;
        assert_eq!(src.latest_tag().await.unwrap(), "v1.2.3");
        let req = requested.lock().unwrap().clone();
        assert_eq!(req, ["latest", "api"], "redirection d'abord, API en repli");
    }

    #[tokio::test]
    async fn pinned_version_skips_any_lookup() {
        let src = PackSource {
            repo: "o/r".into(),
            web_base: "http://127.0.0.1:1".into(),
            api_base: "http://127.0.0.1:1".into(),
            pinned: Some("v9.9.9".into()),
            public_key: None,
        };
        assert_eq!(src.latest_tag().await.unwrap(), "v9.9.9");
    }

    #[tokio::test]
    async fn bad_checksum_is_rejected_and_the_previous_install_survives() {
        let good = server_zip("mcp-legifrance", "OLD-BIN", None, true);
        let mock = mock_with(vec![("mcp-legifrance-1.2.3.zip", good.clone())], false);
        let src = spawn_mock(mock).await;
        let d = TmpDir::new("sum");
        install(&src, &d.0, &["mcp-legifrance".into()], &noop)
            .await
            .unwrap();

        // Release « altérée » : l'archive servie ne correspond plus à checksums.txt.
        let mut tampered = mock_with(
            vec![(
                "mcp-legifrance-1.2.3.zip",
                server_zip("mcp-legifrance", "EVIL-BIN", None, true),
            )],
            false,
        );
        tampered.checksums = Arc::new(format!("{}  mcp-legifrance-1.2.3.zip\n", sha256_hex(&good)));
        let src2 = spawn_mock(tampered).await;
        let rep = install(&src2, &d.0, &["mcp-legifrance".into()], &noop)
            .await
            .unwrap();
        assert!(!rep.servers[0].installed);
        assert!(
            rep.servers[0].error.as_deref().unwrap().contains("SHA-256"),
            "{rep:?}"
        );
        let bin =
            d.0.join("mcp-legifrance")
                .join(format!("bin/mcp-legifrance-{}", platform_key()));
        assert_eq!(
            std::fs::read_to_string(bin).unwrap(),
            "OLD-BIN",
            "l'ancienne version n'a pas été touchée"
        );
    }

    #[tokio::test]
    async fn missing_checksum_entry_or_asset_is_a_per_server_error() {
        let mut mock = mock_with(
            vec![(
                "mcp-legifrance-1.2.3.zip",
                server_zip("mcp-legifrance", "B", None, true),
            )],
            false,
        );
        // Judilibre : ni asset ni ligne dans checksums.txt.
        mock.checksums = Arc::new(mock.checksums.as_str().to_string());
        let src = spawn_mock(mock).await;
        let d = TmpDir::new("miss");
        let rep = install(&src, &d.0, &ids(), &noop).await.unwrap();
        let by = |id: &str| rep.servers.iter().find(|s| s.id == id).unwrap();
        assert!(
            by("mcp-legifrance").installed,
            "un échec n'empêche pas l'autre serveur"
        );
        assert!(
            !by("mcp-judilibre").installed
                && by("mcp-judilibre")
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("checksums.txt")
        );
    }

    #[tokio::test]
    async fn malicious_archives_are_refused() {
        let cases: Vec<(&str, Vec<u8>, &str)> = vec![
            (
                "zip slip",
                zip_with_entries(&[
                    ("mcp-legifrance/manifest.json", "{}"),
                    ("mcp-legifrance/../../evil.txt", "x"),
                ]),
                "dangereux",
            ),
            (
                "autre racine",
                zip_with_entries(&[("evil/manifest.json", "{}")]),
                "hors de",
            ),
            (
                "chemin absolu",
                zip_with_entries(&[("/etc/cron.d/x", "x")]),
                "dangereux",
            ),
            (
                "id du manifest trompeur",
                server_zip("mcp-legifrance", "B", Some("mcp-autre"), true),
                "id",
            ),
            (
                "binaire absent pour la plateforme",
                server_zip("mcp-legifrance", "B", None, false),
                "binaire pour",
            ),
        ];
        for (label, zip_bytes, expect) in cases {
            let mock = mock_with(vec![("mcp-legifrance-1.2.3.zip", zip_bytes)], false);
            let src = spawn_mock(mock).await;
            let d = TmpDir::new("evil");
            let rep = install(&src, &d.0, &["mcp-legifrance".into()], &noop)
                .await
                .unwrap();
            let s = &rep.servers[0];
            assert!(!s.installed, "{label} : doit être refusé");
            assert!(
                s.error.as_deref().unwrap().contains(expect),
                "{label} : {:?}",
                s.error
            );
            assert!(
                !d.0.join("mcp-legifrance").exists(),
                "{label} : rien d'installé"
            );
            assert!(
                !d.0.parent().unwrap().join("evil.txt").exists(),
                "{label} : rien d'écrit hors du dossier"
            );
            assert_eq!(
                installed_version(&d.0),
                None,
                "{label}: pas de version enregistrée"
            );
        }
    }

    #[tokio::test]
    async fn on_replace_is_called_before_an_existing_server_is_replaced() {
        let mock = mock_with(
            vec![(
                "mcp-legifrance-1.2.3.zip",
                server_zip("mcp-legifrance", "B", None, true),
            )],
            false,
        );
        let src = spawn_mock(mock).await;
        let d = TmpDir::new("repl");
        let calls = Mutex::new(Vec::<String>::new());
        // `coerce` fixe la signature « pour toute durée de vie » que l'inférence ne devine pas.
        fn coerce<F: for<'a> Fn(&'a str) -> BoxFuture<'a, ()> + Sync>(f: F) -> F {
            f
        }
        let cb = coerce(|id| {
            calls.lock().unwrap().push(id.to_string());
            Box::pin(async {})
        });
        install(&src, &d.0, &["mcp-legifrance".into()], &cb)
            .await
            .unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            ["mcp-legifrance"],
            "l'appelant peut arrêter le processus avant remplacement"
        );
    }

    #[tokio::test]
    async fn oversized_download_is_refused() {
        let d = TmpDir::new("big");
        let client = http_client();
        // checksums.txt dépasse son plafond (1 Mo).
        let mock = Mock {
            assets: Arc::default(),
            checksums: Arc::new("x".repeat(1024 * 1024 + 10)),
            requested: Arc::default(),
            no_redirect: false,
        };
        let src = spawn_mock(mock).await;
        let err = download(
            &client,
            &src.asset_url("v1.2.3", "checksums.txt"),
            MAX_CHECKSUMS_BYTES,
        )
        .await
        .unwrap_err();
        assert!(err.contains("trop volumineux"), "{err}");
        drop(d);
    }

    #[tokio::test]
    async fn check_update_reports_without_failing_offline() {
        let mock = mock_with(vec![], false);
        let src = spawn_mock(mock).await;
        let st = check_update(&src, Some("1.0.0".into())).await;
        assert_eq!(
            (st.latest.as_deref(), st.update_available),
            (Some("1.2.3"), true)
        );
        let st = check_update(&src, Some("1.2.3".into())).await;
        assert!(!st.update_available);
        // Hors ligne : jamais d'erreur côté appelant, juste `latest = None` + raison.
        let dead = PackSource {
            repo: "o/r".into(),
            web_base: "http://127.0.0.1:1".into(),
            api_base: "http://127.0.0.1:1".into(),
            pinned: None,
            public_key: None,
        };
        let st = check_update(&dead, None).await;
        assert!(st.latest.is_none() && !st.update_available && st.error.is_some());
    }

    /// Contre la VRAIE release GitHub (réseau). Ignoré par défaut :
    ///   cargo test --lib real_github_release -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "nécessite un accès à github.com"]
    async fn real_github_release_installs_the_two_servers() {
        let src = PackSource {
            pinned: Some("v0.3.1".into()),
            ..PackSource::default()
        };
        let d = TmpDir::new("real");
        let rep = install(&src, &d.0, &ids(), &noop).await.unwrap();
        println!("{rep:#?}");
        for s in &rep.servers {
            assert!(s.installed, "{s:?}");
        }
        let found = discover(&d.0, &ids());
        for f in &found {
            println!(
                "{} — disponible sur {} : {}",
                f.manifest.id,
                platform_key(),
                f.available
            );
            assert!(f.available);
            assert!(resolve_bin(f).unwrap().is_file());
        }
        assert_eq!(found.len(), 2);
        let names: Vec<_> = std::fs::read_dir(&d.0)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !names
                .iter()
                .any(|n| n.contains("grist") || n.contains("system") || n.contains("image")),
            "{names:?}"
        );
    }

    // ── Signature des releases (minisign) ─────────────────────────────────────
    //
    // Vecteurs au format minisign (clés jetables, sans lien avec la clé de production) :
    //   minisign -S -s A.key -m checksums.txt -t "demeter-mcp-pack version:1.2.3"  (format de sign-mcp-pack.sh)
    // `SIGNED_CHECKSUMS` liste l'empreinte du zip statique `SIGNED_ZIP_B64`, de sorte qu'une
    // installation signée peut être testée de bout en bout sans signer pendant le test.

    const PUBKEY_A: &str = "RWSsppOOsLatYF1nHCwCztyOXWw44nYCq6M5iPaxnqRCNU8cTHeabuXc";
    const PUBKEY_B: &str = "RWSQU/1x+zfz80UASWunKK2nC3mLKsGhsQYS3Zpjjm0kzcXmHeUF5+5S";
    const SIGNED_CHECKSUMS: &str = "94797a1c64db89693e3bd9e0a2fd07093d55478927d20de1c1fff506478f5c21  mcp-legifrance-1.2.3.zip\n";
    const SIG_OK: &str = "untrusted comment: signature from minisign secret key\nRUSsppOOsLatYGfWTdRGuXEHkkDHouw7tl4o7081YRRjnS07vcLOwv5x3Ggs+DdfVZObFshnvBcyB4tfKjSWz2a9WgaURbyk9A8=\ntrusted comment: demeter-mcp-pack version:1.2.3\njjbpwNTnjYK51XWe2FKOEMPBxMZbBpWP3ddRxjOcvhNJyhitaSZCb3/2uwZF+P0O2AzRYq2tNpGMYIArCK2uBw==\n";
    const SIG_OTHER_RELEASE: &str = "untrusted comment: signature from minisign secret key\nRUSsppOOsLatYGfWTdRGuXEHkkDHouw7tl4o7081YRRjnS07vcLOwv5x3Ggs+DdfVZObFshnvBcyB4tfKjSWz2a9WgaURbyk9A8=\ntrusted comment: demeter-mcp-pack version:1.0.0\nC5Ef2giEfl6ixUpgIVjzpul3XB4QrlAXlGGRwO96TiuQirwb3DBqbfOCovEkDPRgzKq8YRT2JOfn6bM32osYAw==\n";
    const SIG_LEGACY: &str = "untrusted comment: signature from minisign secret key\nRWSsppOOsLatYHBdv7dDC5Ooj81HTcUassnc+X+W+bJZLPR83XP9NTKQ0JGfURTaekH4t89h6GNHpBJK0JgH0+dlkB3HHjoPNAs=\ntrusted comment: demeter-mcp-pack version:1.2.3\nDVeqzFkxru/XIQPmuJjpvqcCZR+LlHVfweiNy45G+8WIuFUStdosBPfwHua4UBNhRGN5iVbfAsa4NtHcpKWiAQ==\n";
    const SIGNED_ZIP_B64: &str = "UEsDBBQAAAAAAAAAIVxn93vklgEAAJYBAAAcAAAAbWNwLWxlZ2lmcmFuY2UvbWFuaWZlc3QuanNvbnsiaWQiOiAibWNwLWxlZ2lmcmFuY2UiLCAibmFtZSI6ICJtY3AtbGVnaWZyYW5jZSIsICJkZXNjcmlwdGlvbiI6ICJ0ZXN0IiwgImJpbiI6IHsibGludXgteDY0IjogImJpbi9tY3AtbGVnaWZyYW5jZS1saW51eC14NjQiLCAibGludXgtYXJtNjQiOiAiYmluL21jcC1sZWdpZnJhbmNlLWxpbnV4LWFybTY0IiwgIm1hY29zLWFybTY0IjogImJpbi9tY3AtbGVnaWZyYW5jZS1tYWNvcy1hcm02NCIsICJtYWNvcy14NjQiOiAiYmluL21jcC1sZWdpZnJhbmNlLW1hY29zLXg2NCIsICJ3aW5kb3dzLXg2NCI6ICJiaW4vbWNwLWxlZ2lmcmFuY2Utd2luZG93cy14NjQiLCAid2luZG93cy1hcm02NCI6ICJiaW4vbWNwLWxlZ2lmcmFuY2Utd2luZG93cy1hcm02NCJ9LCAiYXJncyI6IFsiLS10cmFuc3BvcnQiLCAic3RkaW8iXX1QSwMEFAAAAAAAAAAhXNI8CMYKAAAACgAAACsAAABtY3AtbGVnaWZyYW5jZS9iaW4vbWNwLWxlZ2lmcmFuY2UtbGludXgteDY0QklOLVNJR05FRFBLAwQUAAAAAAAAACFc0jwIxgoAAAAKAAAALQAAAG1jcC1sZWdpZnJhbmNlL2Jpbi9tY3AtbGVnaWZyYW5jZS1saW51eC1hcm02NEJJTi1TSUdORURQSwMEFAAAAAAAAAAhXNI8CMYKAAAACgAAAC0AAABtY3AtbGVnaWZyYW5jZS9iaW4vbWNwLWxlZ2lmcmFuY2UtbWFjb3MtYXJtNjRCSU4tU0lHTkVEUEsDBBQAAAAAAAAAIVzSPAjGCgAAAAoAAAArAAAAbWNwLWxlZ2lmcmFuY2UvYmluL21jcC1sZWdpZnJhbmNlLW1hY29zLXg2NEJJTi1TSUdORURQSwMEFAAAAAAAAAAhXNI8CMYKAAAACgAAAC0AAABtY3AtbGVnaWZyYW5jZS9iaW4vbWNwLWxlZ2lmcmFuY2Utd2luZG93cy14NjRCSU4tU0lHTkVEUEsDBBQAAAAAAAAAIVzSPAjGCgAAAAoAAAAvAAAAbWNwLWxlZ2lmcmFuY2UvYmluL21jcC1sZWdpZnJhbmNlLXdpbmRvd3MtYXJtNjRCSU4tU0lHTkVEUEsBAhQDFAAAAAAAAAAhXGf3e+SWAQAAlgEAABwAAAAAAAAAAAAAAKQBAAAAAG1jcC1sZWdpZnJhbmNlL21hbmlmZXN0Lmpzb25QSwECFAMUAAAAAAAAACFc0jwIxgoAAAAKAAAAKwAAAAAAAAAAAAAA7QHQAQAAbWNwLWxlZ2lmcmFuY2UvYmluL21jcC1sZWdpZnJhbmNlLWxpbnV4LXg2NFBLAQIUAxQAAAAAAAAAIVzSPAjGCgAAAAoAAAAtAAAAAAAAAAAAAADtASMCAABtY3AtbGVnaWZyYW5jZS9iaW4vbWNwLWxlZ2lmcmFuY2UtbGludXgtYXJtNjRQSwECFAMUAAAAAAAAACFc0jwIxgoAAAAKAAAALQAAAAAAAAAAAAAA7QF4AgAAbWNwLWxlZ2lmcmFuY2UvYmluL21jcC1sZWdpZnJhbmNlLW1hY29zLWFybTY0UEsBAhQDFAAAAAAAAAAhXNI8CMYKAAAACgAAACsAAAAAAAAAAAAAAO0BzQIAAG1jcC1sZWdpZnJhbmNlL2Jpbi9tY3AtbGVnaWZyYW5jZS1tYWNvcy14NjRQSwECFAMUAAAAAAAAACFc0jwIxgoAAAAKAAAALQAAAAAAAAAAAAAA7QEgAwAAbWNwLWxlZ2lmcmFuY2UvYmluL21jcC1sZWdpZnJhbmNlLXdpbmRvd3MteDY0UEsBAhQDFAAAAAAAAAAhXNI8CMYKAAAACgAAAC8AAAAAAAAAAAAAAO0BdQMAAG1jcC1sZWdpZnJhbmNlL2Jpbi9tY3AtbGVnaWZyYW5jZS13aW5kb3dzLWFybTY0UEsFBgAAAAAHAAcAagIAAMwDAAAAAA==";

    /// Serveur factice dont `checksums.txt` est EXACTEMENT `checksums` (et non recalculé).
    fn signed_mock(checksums: &str, signature: Option<&str>, with_zip: bool) -> Mock {
        use base64::Engine;
        let mut assets = HashMap::new();
        if with_zip {
            let zip = base64::engine::general_purpose::STANDARD
                .decode(SIGNED_ZIP_B64)
                .unwrap();
            assets.insert("mcp-legifrance-1.2.3.zip".to_string(), zip);
        }
        if let Some(s) = signature {
            assets.insert("checksums.txt.minisig".to_string(), s.as_bytes().to_vec());
        }
        Mock {
            assets: Arc::new(assets),
            checksums: Arc::new(checksums.to_string()),
            requested: Arc::default(),
            no_redirect: false,
        }
    }

    fn legifrance() -> Vec<String> {
        vec!["mcp-legifrance".into()]
    }

    #[test]
    fn minisign_vectors() {
        let data = SIGNED_CHECKSUMS.as_bytes();
        assert!(verify_minisign(PUBKEY_A, data, SIG_OK, "1.2.3").is_ok());

        // Contenu altéré d'un seul octet.
        let mut tampered = data.to_vec();
        tampered[0] = if tampered[0] == b'0' { b'1' } else { b'0' };
        let e = verify_minisign(PUBKEY_A, &tampered, SIG_OK, "1.2.3").unwrap_err();
        assert!(e.contains("INVALIDE"), "{e}");

        // Autre clé publique.
        assert!(verify_minisign(PUBKEY_B, data, SIG_OK, "1.2.3").is_err());
        // Signature légitime d'une AUTRE release (rejeu) : valide cryptographiquement, refusée.
        let e = verify_minisign(PUBKEY_A, data, SIG_OTHER_RELEASE, "1.2.3").unwrap_err();
        assert!(e.contains("ne désigne pas la release"), "{e}");
        // Format « legacy » (non pré-haché) refusé.
        assert!(verify_minisign(PUBKEY_A, data, SIG_LEGACY, "1.2.3").is_err());
        // Entrées invalides : erreurs claires, jamais de panique.
        assert!(verify_minisign(PUBKEY_A, data, "n'importe quoi", "1.2.3")
            .unwrap_err()
            .contains("illisible"));
        assert!(verify_minisign("pas une clé", data, SIG_OK, "1.2.3")
            .unwrap_err()
            .contains("clé publique"));
    }

    // ── Compatibilité avec la signature de production (partagée avec Demeter) ──────────

    /// `checksums.txt` et `checksums.txt.minisig` RÉELS de la release 0.3.3 du pack, signés avec
    /// la clé de production par `sign-mcp-pack.sh` (Demeter). Prouve que Juridix accepte la
    /// signature telle que Demeter la produit : une seule signature pour les deux applications.
    const PROD_CHECKSUMS: &str = "026085ca212cc82f0b2f62e27b7af1c76ef5adc872b39873ffdba5a16aebd0fe  mcp-grist-0.3.3.zip\n797bf988c819f56ecafeb65bcf5998b1737522ed8ed1124a66c91e8b0640b91b  mcp-image-0.3.3.zip\n7038d59d4abb70591c86dc7570570fa86461e8f207511c5a79260086e09cb09a  mcp-judilibre-0.3.3.zip\n92b22cea97f7b633d506fa6faddce7958753c77b6c9af538f25b3739fca1583b  mcp-legifrance-0.3.3.zip\n8790dde27a8d566dfcdd157e1854adfd142f3c759ed8aa3c46d2e72269fd994a  mcp-pack-0.3.3.zip\n587a52fc71a412f2df14b6399b69bef562c5b44cd61067074760465baead7f0f  mcp-system-0.3.3.zip\n";
    const PROD_SIG: &str = "untrusted comment: signature from minisign secret key\nRUQPw2uvrkzLBKNhxdtS+yaMWGYrnMF1vbO5/ZD97zI63iW2lLLHA72VpU3l41WfwbnHplLIB6IAxvn42Xe/eHihcVs1gNX25A0=\ntrusted comment: demeter-mcp-pack version:0.3.3\nCFWN3u56IgbXzVfDneu6g94n+h4ntAKjL2tfZHYQIDEa92x5RPmKP7Gr7Qu29ptyvxo+0ind9Zj02AVjOMW9Bg==\n";

    #[test]
    fn the_embedded_key_is_valid_and_verifies_the_demeter_signature_of_release_0_3_3() {
        let key = configured_public_key().expect("mcp-pack.pub doit contenir une clé");
        assert!(has_key_line(EMBEDDED_PUBLIC_KEY_FILE));
        assert!(
            PublicKey::from_base64(&key).is_ok(),
            "clé publique invalide"
        );
        // Tag GitHub « v0.3.3 » → version « 0.3.3 » (commentaire signé : version:0.3.3).
        let version = "v0.3.3".trim_start_matches('v');
        verify_minisign(&key, PROD_CHECKSUMS.as_bytes(), PROD_SIG, version).unwrap();
        // Rejeu sous une autre version, ou fichier modifié : refusés.
        assert!(verify_minisign(&key, PROD_CHECKSUMS.as_bytes(), PROD_SIG, "0.3.4").is_err());
        let tampered = PROD_CHECKSUMS.replacen('0', "1", 1);
        assert!(verify_minisign(&key, tampered.as_bytes(), PROD_SIG, "0.3.3").is_err());
    }

    #[test]
    fn public_key_file_parsing() {
        assert_eq!(
            parse_public_key_file("untrusted comment: x\nRWABC\n").as_deref(),
            Some("RWABC")
        );
        assert_eq!(parse_public_key_file("# commentaire seul\n\n"), None);
        assert!(has_key_line("untrusted comment: x\nRWABC\n"));
        assert!(!has_key_line("untrusted comment: x\n# RWABC\n"));
        assert!(!has_key_line(""));
    }

    #[test]
    fn version_ordering() {
        assert!(version_lt("1.2.3", "1.2.4"));
        assert!(version_lt("0.9.0", "1.0.0"));
        assert!(version_lt("1.2", "1.2.1"));
        assert!(
            version_lt("v1.9.0", "1.10.0"),
            "comparaison numérique, pas lexicale"
        );
        assert!(!version_lt("1.2.3", "1.2.3"));
        assert!(!version_lt("1.2.0", "1.2"), "composantes manquantes = 0");
        assert!(!version_lt("2.0.0", "1.99.99"));
        assert!(!version_lt("1.2.3-rc1", "1.2.3"), "le suffixe est ignoré");
    }

    #[tokio::test]
    async fn a_signed_release_installs_and_is_reported_verified() {
        let mock = signed_mock(SIGNED_CHECKSUMS, Some(SIG_OK), true);
        let mut src = spawn_mock(mock).await;
        src.public_key = Some(PUBKEY_A.into());
        let d = TmpDir::new("signed");
        let rep = install(&src, &d.0, &legifrance(), &noop).await.unwrap();
        assert_eq!(rep.signature, SignatureStatus::Verified);
        assert!(rep.servers.iter().all(|s| s.installed), "{rep:?}");
        assert_eq!(installed_version(&d.0).as_deref(), Some("1.2.3"));
        let bin =
            d.0.join("mcp-legifrance")
                .join(format!("bin/mcp-legifrance-{}", platform_key()));
        assert_eq!(std::fs::read_to_string(bin).unwrap(), "BIN-SIGNED");
    }

    #[tokio::test]
    async fn with_a_configured_key_a_missing_signature_refuses_the_install() {
        let mock = signed_mock(SIGNED_CHECKSUMS, None, true);
        let requested = mock.requested.clone();
        let mut src = spawn_mock(mock).await;
        src.public_key = Some(PUBKEY_A.into());
        let d = TmpDir::new("nosig");
        let e = install(&src, &d.0, &legifrance(), &noop).await.unwrap_err();
        assert!(e.contains("signature") && e.contains("refusée"), "{e}");
        assert!(
            !requested
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.ends_with(".zip")),
            "aucun zip ne doit être téléchargé"
        );
        assert!(installed_version(&d.0).is_none());
        assert!(!d.0.join("mcp-legifrance").exists());
    }

    #[tokio::test]
    async fn tampered_checksums_are_refused_before_any_zip_is_downloaded() {
        // `checksums.txt` modifié (l'attaquant remplace l'empreinte) mais signature d'origine.
        let forged = SIGNED_CHECKSUMS.replacen(&SIGNED_CHECKSUMS[..8], "00000000", 1);
        let mock = signed_mock(&forged, Some(SIG_OK), true);
        let requested = mock.requested.clone();
        let mut src = spawn_mock(mock).await;
        src.public_key = Some(PUBKEY_A.into());
        let d = TmpDir::new("forged");
        let e = install(&src, &d.0, &legifrance(), &noop).await.unwrap_err();
        assert!(e.contains("INVALIDE"), "{e}");
        assert!(!requested
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.ends_with(".zip")));
        assert!(!d.0.join("mcp-legifrance").exists());
    }

    #[tokio::test]
    async fn a_valid_signature_of_another_release_a_legacy_one_or_another_key_are_refused() {
        for (sig, key, motif) in [
            (SIG_OTHER_RELEASE, PUBKEY_A, "ne désigne pas la release"),
            (SIG_LEGACY, PUBKEY_A, "signature"),
            (SIG_OK, PUBKEY_B, "signature"),
        ] {
            let mock = signed_mock(SIGNED_CHECKSUMS, Some(sig), true);
            let mut src = spawn_mock(mock).await;
            src.public_key = Some(key.into());
            let d = TmpDir::new("refused");
            let e = install(&src, &d.0, &legifrance(), &noop).await.unwrap_err();
            assert!(e.contains(motif), "{e}");
            assert!(!d.0.join("mcp-legifrance").exists());
        }
    }

    #[tokio::test]
    async fn without_a_configured_key_the_checksum_only_mode_is_reported_and_no_signature_is_fetched(
    ) {
        let mock = mock_with(
            vec![(
                "mcp-legifrance-1.2.3.zip",
                server_zip("mcp-legifrance", "BIN", None, true),
            )],
            false,
        );
        let requested = mock.requested.clone();
        let src = spawn_mock(mock).await;
        assert!(src.public_key.is_none());
        let d = TmpDir::new("nokey");
        let rep = install(&src, &d.0, &legifrance(), &noop).await.unwrap();
        assert_eq!(rep.signature, SignatureStatus::NotConfigured);
        assert!(!requested
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.ends_with(".minisig")));
        let st = check_update(&src, None).await;
        assert!(!st.signature_enforced);
    }

    #[tokio::test]
    async fn update_status_reports_that_a_signature_is_enforced() {
        let mock = signed_mock(SIGNED_CHECKSUMS, Some(SIG_OK), true);
        let mut src = spawn_mock(mock).await;
        src.public_key = Some(PUBKEY_A.into());
        assert!(check_update(&src, None).await.signature_enforced);
    }

    #[tokio::test]
    async fn rolling_back_to_an_older_pack_is_refused_but_a_reinstall_is_allowed() {
        let mock = mock_with(
            vec![(
                "mcp-legifrance-1.2.3.zip",
                server_zip("mcp-legifrance", "BIN", None, true),
            )],
            false,
        );
        let src = spawn_mock(mock).await;

        let d = TmpDir::new("rollback");
        std::fs::create_dir_all(&d.0).unwrap();
        std::fs::write(d.0.join(".pack-version"), "2.0.0").unwrap();
        let e = install(&src, &d.0, &legifrance(), &noop).await.unwrap_err();
        assert!(e.contains("retour arrière refusé"), "{e}");
        assert!(!d.0.join("mcp-legifrance").exists());
        assert_eq!(
            installed_version(&d.0).as_deref(),
            Some("2.0.0"),
            "la version installée n'est pas touchée"
        );

        // Même version : réinstallation autorisée (réparation).
        std::fs::write(d.0.join(".pack-version"), "1.2.3").unwrap();
        assert!(install(&src, &d.0, &legifrance(), &noop).await.is_ok());
        // Version plus ancienne installée : mise à jour normale.
        std::fs::write(d.0.join(".pack-version"), "1.0.0").unwrap();
        assert!(install(&src, &d.0, &legifrance(), &noop).await.is_ok());
        assert_eq!(installed_version(&d.0).as_deref(), Some("1.2.3"));
    }
}
