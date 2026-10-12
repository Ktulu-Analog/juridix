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

// Serveurs MCP en STDIO, lancés automatiquement comme sous-processus.
//
// Même principe que Demeter : un serveur est un dossier (`manifest.json` + un
// binaire par plateforme, importé par `mcp_pack.rs`) ; Juridix le démarre à la
// première utilisation et dialogue en JSON-RPC sur stdin/stdout. Il se référence
// dans les réglages par `stdio:<id>` (ex. `stdio:mcp-legifrance`).
//
// Juridix ne gère que Légifrance et Judilibre (`official_specs`) : tout autre
// dossier du pack est ignoré, même présent dans le répertoire.
//
// ── Ce que Juridix décide (et non le manifest) ───────────────────────────────
// Les arguments de lancement (`--transport stdio`), l'environnement du processus
// et la liste des outils exposés sont fixés ICI. Le manifest n'apporte que le
// chemin du binaire : un manifest altéré ne peut ni ajouter d'arguments ni
// injecter de variables d'environnement.
//
// ── Identifiants ─────────────────────────────────────────────────────────────
// Les identifiants PISTE saisis dans Juridix (réglages juridiques) sont passés
// aux serveurs sous les noms qu'ils attendent (`LEGIFRANCE_CLIENT_ID`,
// `JUDILIBRE_CLIENT_ID`…). Aucun secret n'est écrit sur disque par ce module.
// Sans identifiants, le serveur n'est pas lancé : on évite de proposer au LLM des
// outils qui échoueraient tous.
//
// ── Outils exposés ───────────────────────────────────────────────────────────
// Légifrance en offre 62, Judilibre 8 : tous les exposer coûte ≈ 7 600 jetons de
// définitions à CHAQUE requête de chat et peut dégrader le choix d'outil du modèle.
// Par défaut (« Essentiels »), chaque spécification limite le catalogue (≈ 1 450
// jetons au total) ; l'utilisateur peut choisir « Tous » ou une liste
// personnalisée (`ToolSelection`, mémorisée dans `.tools.json`). La sélection vaut
// aussi à l'appel : le modèle ne peut pas invoquer un outil qu'on ne lui a pas exposé.
//
// ── Cycle de vie ─────────────────────────────────────────────────────────────
// Un processus par serveur, réutilisé (handshake `initialize` une seule fois).
// Les requêtes d'un même serveur sont sérialisées. Toute erreur de transport
// (délai, processus mort, sortie illisible) abandonne le processus : le prochain
// appel en relance un neuf. Une erreur *applicative* du serveur (outil inconnu,
// arguments invalides) ne touche pas à la session. Quand Juridix se ferme, les
// tubes stdin/stdout se ferment et les serveurs s'arrêtent d'eux-mêmes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::mcp_pack;

/// Préfixe d'un serveur STDIO dans la liste des serveurs MCP des réglages.
pub const SCHEME: &str = "stdio:";

/// `npx` n'est pas utilisé, mais un antivirus peut ralentir un premier lancement.
const T_INIT: Duration = Duration::from_secs(60);
const T_LIST: Duration = Duration::from_secs(20);
const T_CALL: Duration = Duration::from_secs(60);
/// Garde-fou : une ligne JSON-RPC ne dépasse pas cette taille.
const MAX_LINE: usize = 32 * 1024 * 1024;
const MAX_LIST_PAGES: usize = 10;
const PROTOCOL_VERSION: &str = "2025-03-26";

// ── Adressage ─────────────────────────────────────────────────────────────────

pub fn is_stdio(url: &str) -> bool {
    url.trim().to_ascii_lowercase().starts_with(SCHEME)
}

/// `stdio:mcp-legifrance` → `mcp-legifrance`. Tolère `stdio://…` et les espaces.
pub fn server_name(url: &str) -> &str {
    let t = url.trim();
    let rest = t.get(SCHEME.len()..).unwrap_or("").trim();
    rest.trim_start_matches('/').trim_end_matches('/').trim()
}

// ── Serveurs gérés ────────────────────────────────────────────────────────────

/// Un serveur géré par Juridix.
#[derive(Debug, Clone)]
pub struct Spec {
    /// Identifiant = nom du dossier = `id` du manifest (`mcp-legifrance`).
    pub id: String,
    /// Variables d'environnement du processus : (nom attendu par le serveur, modèle `${…}`).
    pub env: Vec<(String, String)>,
    /// Secrets obligatoires (noms internes) ; sans eux le serveur n'est pas lancé.
    pub requires: Vec<String>,
    /// Outils exposés au LLM et appelables (noms exacts ou préfixes `xxx*`) ; vide = tous.
    pub tools: Vec<String>,
}

pub const SECRET_PISTE_ID: &str = "JURIDIX_PISTE_CLIENT_ID";
pub const SECRET_PISTE_SECRET: &str = "JURIDIX_PISTE_CLIENT_SECRET";

/// Légifrance et Judilibre — les deux seuls serveurs du pack que Juridix importe et lance.
pub fn official_specs() -> Vec<Spec> {
    let tpl = |s: &str| format!("${{{s}}}");
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    vec![
        Spec {
            id: "mcp-legifrance".into(),
            env: vec![
                ("LEGIFRANCE_CLIENT_ID".into(), tpl(SECRET_PISTE_ID)),
                ("LEGIFRANCE_CLIENT_SECRET".into(), tpl(SECRET_PISTE_SECRET)),
                // Facultatif (« sandbox ») : repris de l'environnement de Juridix s'il existe.
                ("LEGIFRANCE_ENV".into(), tpl("LEGIFRANCE_ENV")),
            ],
            requires: s(&[SECRET_PISTE_ID, SECRET_PISTE_SECRET]),
            tools: s(&[
                "legifrance_rechercher",
                "legifrance_article_par_numero",
                "legifrance_obtenir_article",
                "legifrance_loi_decret",
                "legifrance_jorf",
                "legifrance_versions_article",
            ]),
        },
        Spec {
            id: "mcp-judilibre".into(),
            env: vec![
                ("JUDILIBRE_CLIENT_ID".into(), tpl(SECRET_PISTE_ID)),
                ("JUDILIBRE_CLIENT_SECRET".into(), tpl(SECRET_PISTE_SECRET)),
            ],
            requires: s(&[SECRET_PISTE_ID, SECRET_PISTE_SECRET]),
            tools: s(&[
                "judilibre_rechercher",
                "judilibre_decision",
                "judilibre_taxonomie",
            ]),
        },
    ]
}

/// Définition de lancement, construite à partir d'une `Spec` et du dossier découvert.
#[derive(Debug, Clone)]
pub struct ServerDef {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    /// Outils autorisés. `None` = tous ; `Some(liste)` = seulement ceux-là (noms exacts ou
    /// préfixes `xxx*`) — une liste VIDE n'autorise donc aucun outil.
    pub tools: Option<Vec<String>>,
}

/// `pattern` = nom exact, ou préfixe terminé par `*`.
fn matches_pattern(pattern: &str, tool: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => tool.starts_with(prefix),
        None => pattern == tool,
    }
}

impl ServerDef {
    /// Cet outil peut-il être exposé / appelé ?
    pub fn allows(&self, tool: &str) -> bool {
        match &self.tools {
            None => true,
            Some(list) => list.iter().any(|p| matches_pattern(p, tool)),
        }
    }
}

/// Quels outils d'un serveur sont exposés au LLM. Chaque définition d'outil part avec
/// CHAQUE requête de chat : exposer les 62 outils de Légifrance coûte ≈ 5 900 jetons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ToolSelection {
    /// Le jeu restreint défini par Juridix pour ce serveur (`Spec::tools`). Par défaut.
    #[default]
    Essential,
    /// Tous les outils du serveur.
    All,
    /// Une liste choisie par l'utilisateur (jamais vide).
    Custom { tools: Vec<String> },
}

/// Un outil du catalogue d'un serveur, tel que l'interface l'affiche.
#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    /// Coût estimé de sa définition, en jetons (≈ caractères / 4), à chaque requête.
    pub tokens: usize,
    /// Fait partie du jeu « Essentiels ».
    pub essential: bool,
    /// Actuellement exposé au LLM.
    pub exposed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Catalog {
    pub tools: Vec<ToolInfo>,
    pub total: usize,
    pub selection: ToolSelection,
    /// Coût total estimé des outils exposés, en jetons par requête.
    pub tokens_exposed: usize,
}

/// Noms d'outils acceptés dans une sélection (ceux de la spec MCP : lettres, chiffres, `_ - .`).
fn valid_tool_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Fichier des sélections, dans le dossier des serveurs (aucun secret).
const SELECTION_FILE: &str = ".tools.json";

/// Remplace les `${NOM}` de `value` via `lookup` ; un nom inconnu devient une
/// chaîne vide (et est signalé dans les logs, sans jamais afficher de valeur).
fn expand(value: &str, lookup: &(dyn Fn(&str) -> Option<String> + Sync)) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find('}') {
            Some(end) => {
                let name = &rest[start + 2..start + 2 + end];
                match lookup(name) {
                    Some(v) => out.push_str(&v),
                    None => debug!("[MCP stdio] variable ${{{}}} non définie", name),
                }
                rest = &rest[start + 2 + end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

// ── Session ───────────────────────────────────────────────────────────────────

/// Erreur d'un appel : `Transport` ⇒ la session est inutilisable ;
/// `Remote` ⇒ le serveur a répondu par une erreur JSON-RPC (session intacte).
#[derive(Debug)]
enum RpcError {
    Transport(String),
    Remote(Value),
}

impl RpcError {
    fn message(&self) -> String {
        match self {
            RpcError::Transport(m) => m.clone(),
            RpcError::Remote(v) => v
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| v.to_string()),
        }
    }
}

struct Session {
    _child: Child, // kill_on_drop : le processus meurt avec la session
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    /// Version des secrets au lancement (cf. `Hub::env_version`).
    env_version: u64,
}

impl Session {
    async fn send(&mut self, msg: &Value) -> Result<(), RpcError> {
        let mut line =
            serde_json::to_string(msg).map_err(|e| RpcError::Transport(e.to_string()))?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| RpcError::Transport(format!("écriture stdin : {e}")))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| RpcError::Transport(format!("écriture stdin : {e}")))
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), RpcError> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;

        let deadline = Instant::now() + timeout;
        let mut line = String::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RpcError::Transport(format!(
                    "délai dépassé ({}s) sur {method}",
                    timeout.as_secs()
                )));
            }
            line.clear();
            let n = tokio::time::timeout(remaining, self.stdout.read_line(&mut line))
                .await
                .map_err(|_| {
                    RpcError::Transport(format!(
                        "délai dépassé ({}s) sur {method}",
                        timeout.as_secs()
                    ))
                })?
                .map_err(|e| RpcError::Transport(format!("lecture stdout : {e}")))?;
            if n == 0 {
                return Err(RpcError::Transport("le processus MCP s'est arrêté".into()));
            }
            if line.len() > MAX_LINE {
                return Err(RpcError::Transport("message MCP trop volumineux".into()));
            }
            let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
                debug!("[MCP stdio] ligne non JSON ignorée");
                continue; // certains serveurs impriment des bannières sur stdout
            };
            let has_method = v.get("method").is_some();
            let id_val = v.get("id");

            if has_method {
                // Requête du serveur vers nous (id présent) : on répond, sinon il attendrait.
                if let Some(rid) = id_val {
                    let reply = match v["method"].as_str() {
                        Some("ping") => json!({"jsonrpc": "2.0", "id": rid, "result": {}}),
                        _ => json!({"jsonrpc": "2.0", "id": rid,
                                    "error": {"code": -32601, "message": "méthode non prise en charge par le client"}}),
                    };
                    self.send(&reply).await?;
                }
                continue; // notification (logs, progress…) : ignorée
            }
            if id_val == Some(&json!(id)) {
                if let Some(err) = v.get("error") {
                    return Err(RpcError::Remote(err.clone()));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
            // Réponse à une requête antérieure abandonnée : ignorée.
        }
    }
}

/// `Sync` : la référence est conservée à travers des `.await`, et le futur du
/// handler HTTP qui l'appelle doit rester `Send`.
type Lookup<'a> = &'a (dyn Fn(&str) -> Option<String> + Sync);

async fn spawn_session(
    name: &str,
    def: &ServerDef,
    lookup: Lookup<'_>,
    env_version: u64,
) -> Result<Session, RpcError> {
    let mut cmd = Command::new(&def.command);
    cmd.args(&def.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in &def.env {
        let value = expand(v, lookup);
        // Variable vide = non définie (ex. `LEGIFRANCE_ENV` absent) : le serveur garde son défaut.
        if !value.is_empty() {
            cmd.env(k, value);
        }
    }
    if let Some(cwd) = &def.cwd {
        cmd.current_dir(cwd);
    }
    #[cfg(windows)]
    {
        // Pas de fenêtre console qui clignote.
        cmd.creation_flags(0x0800_0000);
    }

    let mut child = cmd.spawn().map_err(|e| {
        RpcError::Transport(format!("lancement de « {} » impossible : {e}", def.command))
    })?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| RpcError::Transport("stdin indisponible".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| RpcError::Transport("stdout indisponible".into()))?;

    // stderr : journalisé (niveau debug), jamais affiché à l'utilisateur ; le
    // vider évite aussi de bloquer le serveur si son tampon stderr se remplit.
    if let Some(stderr) = child.stderr.take() {
        let tag = name.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                debug!("[MCP stdio {tag}] {l}");
            }
        });
    }

    let mut s = Session {
        _child: child,
        stdin,
        stdout: BufReader::new(stdout),
        next_id: 1,
        env_version,
    };
    s.request(
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "Juridix", "version": env!("CARGO_PKG_VERSION") },
        }),
        T_INIT,
    )
    .await?;
    s.notify("notifications/initialized", json!({})).await?;
    info!("[MCP stdio] « {name} » démarré");
    Ok(s)
}

// ── Hub ───────────────────────────────────────────────────────────────────────

type Slot = Arc<Mutex<Option<Session>>>;

/// État d'un serveur, pour l'écran des réglages (aucun secret, aucun chemin sensible).
#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed: bool,
    /// Binaire disponible pour cette plateforme.
    pub available: bool,
    /// Processus actuellement démarré.
    pub running: bool,
    /// Identifiants PISTE manquants : le serveur ne peut pas être lancé.
    pub needs_credentials: bool,
    /// Sélection d'outils en vigueur (Essentiels / Tous / Personnalisé).
    pub selection: ToolSelection,
    /// Outils réellement offerts par le serveur ; `None` tant que le serveur n'a jamais été
    /// interrogé (il faut le lancer pour le savoir).
    pub tools_total: Option<usize>,
    /// Outils exposés au LLM avec la sélection en vigueur (`None` si « Tous » et total inconnu).
    pub tools_exposed: Option<usize>,
}

/// Entrée du catalogue d'un serveur : (nom de l'outil, description, estimation de jetons).
type CatalogEntry = (String, String, usize);

/// Ensemble des serveurs STDIO : spécifications, dossier, secrets et sessions vivantes.
pub struct Hub {
    specs: RwLock<Vec<Spec>>,
    dir: RwLock<Option<PathBuf>>,
    secrets: RwLock<HashMap<String, String>>,
    /// Incrémenté quand un secret change : les sessions plus anciennes sont relancées.
    env_version: AtomicU64,
    slots: std::sync::Mutex<HashMap<String, Slot>>,
    /// Sélection d'outils par serveur (absent = Essentiels). Persistée dans `.tools.json`.
    selections: RwLock<HashMap<String, ToolSelection>>,
    /// Catalogue connu de chaque serveur : (nom, description, jetons). Alimenté à chaque
    /// `tools/list` ; permet d'afficher « 6 sur 62 » sans relancer le serveur.
    catalogs: RwLock<HashMap<String, Vec<CatalogEntry>>>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    /// Hub des serveurs officiels de Juridix (Légifrance, Judilibre).
    pub fn new() -> Self {
        Self::with_specs(official_specs())
    }

    pub fn with_specs(specs: Vec<Spec>) -> Self {
        Self {
            specs: RwLock::new(specs),
            dir: RwLock::new(None),
            secrets: RwLock::new(HashMap::new()),
            env_version: AtomicU64::new(0),
            slots: std::sync::Mutex::new(HashMap::new()),
            selections: RwLock::new(HashMap::new()),
            catalogs: RwLock::new(HashMap::new()),
        }
    }

    /// Remplace les spécifications (tests : le Hub global n'a que les serveurs officiels).
    #[cfg(test)]
    pub fn replace_specs(&self, specs: Vec<Spec>) {
        *self.specs.write().unwrap() = specs;
        self.slots.lock().unwrap().clear();
    }

    /// Dossier des serveurs (un sous-dossier par serveur).
    pub fn set_dir(&self, path: PathBuf) {
        // Sélections d'outils enregistrées : un fichier absent ou illisible = valeurs par défaut.
        let loaded: HashMap<String, ToolSelection> =
            std::fs::read_to_string(path.join(SELECTION_FILE))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
        *self.selections.write().unwrap() = loaded;
        *self.dir.write().unwrap() = Some(path);
    }

    /// Sélection d'outils en vigueur pour un serveur.
    pub fn selection(&self, id: &str) -> ToolSelection {
        self.selections
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_default()
    }

    /// Filtre d'outils correspondant à la sélection (`None` = tous).
    fn tool_filter(&self, spec: &Spec) -> Option<Vec<String>> {
        match self.selection(&spec.id) {
            ToolSelection::Essential if spec.tools.is_empty() => None,
            ToolSelection::Essential => Some(spec.tools.clone()),
            ToolSelection::All => None,
            ToolSelection::Custom { tools } => Some(tools),
        }
    }

    pub fn dir(&self) -> Option<PathBuf> {
        self.dir.read().unwrap().clone()
    }

    /// Identifiants des serveurs gérés (les seuls que le pack peut installer ici).
    pub fn ids(&self) -> Vec<String> {
        self.specs
            .read()
            .unwrap()
            .iter()
            .map(|s| s.id.clone())
            .collect()
    }

    /// Fournit un secret utilisable dans `env` sous la forme `${NOM}`.
    /// Sans effet si la valeur est inchangée.
    pub fn set_secret(&self, name: &str, value: &str) {
        let mut g = self.secrets.write().unwrap();
        if g.get(name).map(String::as_str) != Some(value) {
            g.insert(name.to_string(), value.to_string());
            self.env_version.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn lookup(&self, name: &str) -> Option<String> {
        self.secrets
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
            .filter(|v| !v.is_empty())
    }

    /// `mcp-legifrance` ou `legifrance` → `mcp-legifrance`. Inconnu ⇒ erreur : seuls les
    /// serveurs de `specs` sont utilisables, quoi que contienne le dossier.
    fn resolve_id(&self, name: &str) -> Result<Spec, String> {
        let n = name.trim();
        let specs = self.specs.read().unwrap();
        specs
            .iter()
            .find(|s| s.id == n || s.id.strip_prefix("mcp-") == Some(n))
            .cloned()
            .ok_or_else(|| format!("serveur MCP « {n} » non pris en charge par Juridix"))
    }

    fn def_for(&self, spec: &Spec) -> Result<ServerDef, String> {
        let dir = self
            .dir()
            .ok_or("dossier des serveurs MCP non initialisé")?;
        let found = mcp_pack::discover(&dir, std::slice::from_ref(&spec.id))
            .into_iter()
            .next()
            .ok_or_else(|| {
                format!(
                    "serveur MCP « {} » non installé — importez-le depuis les réglages",
                    spec.id
                )
            })?;
        if !found.available {
            return Err(format!(
                "« {} » n'a pas de binaire pour {} dans la version installée",
                spec.id,
                mcp_pack::platform_key()
            ));
        }
        let bin = mcp_pack::resolve_bin(&found)?;
        Ok(ServerDef {
            command: bin.to_string_lossy().to_string(),
            // Fixé par Juridix, jamais lu du manifest.
            args: vec!["--transport".into(), "stdio".into()],
            env: spec.env.iter().cloned().collect(),
            cwd: Some(found.dir.to_string_lossy().to_string()),
            tools: self.tool_filter(spec),
        })
    }

    fn missing_credentials(&self, spec: &Spec) -> bool {
        spec.requires.iter().any(|r| self.lookup(r).is_none())
    }

    /// État de chaque serveur géré.
    pub fn status(&self) -> Vec<ServerStatus> {
        let specs = self.specs.read().unwrap().clone();
        let dir = self.dir();
        specs
            .iter()
            .map(|spec| {
                let found = dir.as_ref().and_then(|d| {
                    mcp_pack::discover(d, std::slice::from_ref(&spec.id))
                        .into_iter()
                        .next()
                });
                // `try_lock` : un appel en cours tient le verrou, donc le processus tourne.
                let running = self.slot(&spec.id).try_lock().map_or(true, |g| g.is_some());
                let tools_total = self.catalogs.read().unwrap().get(&spec.id).map(Vec::len);
                let tools_exposed = match self.tool_filter(spec) {
                    None => tools_total,
                    Some(list) => match self.catalogs.read().unwrap().get(&spec.id) {
                        // Catalogue connu : on compte les outils réellement présents.
                        Some(cat) => Some(
                            cat.iter()
                                .filter(|(n, _, _)| list.iter().any(|p| matches_pattern(p, n)))
                                .count(),
                        ),
                        None => Some(list.len()),
                    },
                };
                ServerStatus {
                    id: spec.id.clone(),
                    name: found
                        .as_ref()
                        .map(|f| f.manifest.name.clone())
                        .unwrap_or_else(|| spec.id.clone()),
                    description: found
                        .as_ref()
                        .map(|f| f.manifest.description.clone())
                        .unwrap_or_default(),
                    installed: found.is_some(),
                    available: found.as_ref().is_some_and(|f| f.available),
                    running,
                    needs_credentials: self.missing_credentials(spec),
                    selection: self.selection(&spec.id),
                    tools_total,
                    tools_exposed,
                }
            })
            .collect()
    }

    fn slot(&self, id: &str) -> Slot {
        self.slots
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    }

    /// Arrête le processus d'un serveur (sans effet s'il ne tourne pas). Utilisé avant
    /// de remplacer ses fichiers : sous Windows, un exécutable en cours ne peut pas l'être.
    pub async fn stop(&self, name: &str) {
        if let Ok(spec) = self.resolve_id(name) {
            *self.slot(&spec.id).lock().await = None;
        }
    }

    /// Exécute `f` sur la session de `name` (lancée au besoin). Sérialise les
    /// appels d'un même serveur ; abandonne la session sur erreur de transport.
    async fn with_session<T, F>(&self, name: &str, f: F) -> Result<T, String>
    where
        T: Send,
        F: for<'a> FnOnce(&'a mut Session) -> futures::future::BoxFuture<'a, Result<T, RpcError>>
            + Send,
    {
        let spec = self.resolve_id(name)?;
        let slot = self.slot(&spec.id);
        let mut guard = slot.lock().await;
        let version = self.env_version.load(Ordering::SeqCst);

        if guard.as_ref().is_some_and(|s| s.env_version != version) {
            *guard = None; // secrets modifiés : relance avec les nouveaux
        }
        if guard.is_none() {
            if self.missing_credentials(&spec) {
                return Err(format!(
                    "identifiants PISTE non configurés : « {} » n'est pas lancé (réglages juridiques)",
                    spec.id
                ));
            }
            let def = self.def_for(&spec)?;
            let lookup = |n: &str| self.lookup(n);
            *guard = Some(
                spawn_session(&spec.id, &def, &lookup, version)
                    .await
                    .map_err(|e| e.message())?,
            );
        }
        let session = guard.as_mut().expect("session présente");
        match f(session).await {
            Ok(v) => Ok(v),
            Err(RpcError::Remote(v)) => Err(RpcError::Remote(v).message()),
            Err(RpcError::Transport(m)) => {
                warn!("[MCP stdio] « {} » : {m} — session abandonnée", spec.id);
                *guard = None;
                Err(m)
            }
        }
    }

    /// `tools/list` (pagination suivie), restreint aux outils autorisés du serveur.
    pub async fn list_tools(&self, name: &str) -> Result<Vec<Value>, String> {
        let spec = self.resolve_id(name)?;
        let def = self.def_for(&spec)?;
        let all = self.list_tools_unfiltered(&spec.id).await?;
        if def.tools.is_none() {
            return Ok(all);
        }
        let (kept, dropped): (Vec<Value>, Vec<Value>) = all
            .into_iter()
            .partition(|t| t["name"].as_str().is_some_and(|n| def.allows(n)));
        // Une entrée de la liste qui ne correspond à aucun outil du serveur est
        // presque toujours un outil renommé dans une nouvelle version : on le dit, sans échouer.
        for p in def.tools.iter().flatten() {
            let hit = kept
                .iter()
                .chain(dropped.iter())
                .any(|t| t["name"].as_str().is_some_and(|n| matches_pattern(p, n)));
            if !hit {
                warn!(
                    "[MCP stdio] « {} » : l'outil autorisé « {p} » n'existe pas sur ce serveur",
                    spec.id
                );
            }
        }
        Ok(kept)
    }

    async fn list_tools_unfiltered(&self, name: &str) -> Result<Vec<Value>, String> {
        let tools = self.list_tools_raw(name).await?;
        if let Ok(spec) = self.resolve_id(name) {
            let cat: Vec<(String, String, usize)> = tools
                .iter()
                .filter_map(|t| {
                    let n = t["name"].as_str()?.to_string();
                    let d = t["description"].as_str().unwrap_or("").to_string();
                    Some((n, d, t.to_string().chars().count() / 4))
                })
                .collect();
            self.catalogs.write().unwrap().insert(spec.id, cat);
        }
        Ok(tools)
    }

    async fn list_tools_raw(&self, name: &str) -> Result<Vec<Value>, String> {
        self.with_session(name, |s| {
            Box::pin(async move {
                let mut tools: Vec<Value> = Vec::new();
                let mut cursor: Option<String> = None;
                for _ in 0..MAX_LIST_PAGES {
                    let params = match &cursor {
                        Some(c) => json!({ "cursor": c }),
                        None => json!({}),
                    };
                    let r = s.request("tools/list", params, T_LIST).await?;
                    if let Some(a) = r.get("tools").and_then(Value::as_array) {
                        tools.extend(a.iter().cloned());
                    }
                    cursor = r
                        .get("nextCursor")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if cursor.is_none() {
                        break;
                    }
                }
                Ok(tools)
            })
        })
        .await
    }

    /// Catalogue complet du serveur (lance le serveur si nécessaire), avec pour chaque outil
    /// son coût en jetons et s'il est exposé avec la sélection en vigueur.
    pub async fn catalog(&self, name: &str) -> Result<Catalog, String> {
        let spec = self.resolve_id(name)?;
        let def = self.def_for(&spec)?;
        self.list_tools_unfiltered(&spec.id).await?;
        let cat = self
            .catalogs
            .read()
            .unwrap()
            .get(&spec.id)
            .cloned()
            .unwrap_or_default();
        let tools: Vec<ToolInfo> = cat
            .into_iter()
            .map(|(name, description, tokens)| ToolInfo {
                essential: spec.tools.iter().any(|p| matches_pattern(p, &name)),
                exposed: def.allows(&name),
                name,
                description,
                tokens,
            })
            .collect();
        let tokens_exposed = tools.iter().filter(|t| t.exposed).map(|t| t.tokens).sum();
        Ok(Catalog {
            total: tools.len(),
            tokens_exposed,
            selection: self.selection(&spec.id),
            tools,
        })
    }

    /// Change la sélection d'outils d'un serveur et la mémorise. Pour `Custom`, chaque nom doit
    /// exister sur le serveur (vérifié contre son catalogue réel) et la liste ne peut pas être vide.
    pub async fn set_selection(&self, name: &str, sel: ToolSelection) -> Result<(), String> {
        let spec = self.resolve_id(name)?;
        let dir = self
            .dir()
            .ok_or("dossier des serveurs MCP non initialisé")?;
        let sel = match sel {
            ToolSelection::Custom { mut tools } => {
                tools.retain(|t| !t.trim().is_empty());
                tools.sort();
                tools.dedup();
                if tools.is_empty() {
                    return Err("Choisissez au moins un outil (ou utilisez « Essentiels »).".into());
                }
                if let Some(bad) = tools.iter().find(|t| !valid_tool_name(t)) {
                    return Err(format!("nom d'outil invalide : « {bad} »"));
                }
                // Contre le catalogue RÉEL : une faute de frappe ne doit pas produire un serveur muet.
                self.def_for(&spec)?;
                self.list_tools_unfiltered(&spec.id).await?;
                let known = self
                    .catalogs
                    .read()
                    .unwrap()
                    .get(&spec.id)
                    .cloned()
                    .unwrap_or_default();
                if let Some(unknown) = tools
                    .iter()
                    .find(|t| !known.iter().any(|(n, _, _)| n == *t))
                {
                    return Err(format!("l'outil « {unknown} » n'existe pas sur ce serveur"));
                }
                ToolSelection::Custom { tools }
            }
            other => other,
        };
        let snapshot = {
            let mut g = self.selections.write().unwrap();
            if sel == ToolSelection::Essential {
                g.remove(&spec.id);
            } else {
                g.insert(spec.id.clone(), sel);
            }
            g.clone()
        };
        // Écriture atomique (fichier temporaire puis renommage) : jamais de fichier à moitié écrit.
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("dossier des serveurs inaccessible : {e}"))?;
        let tmp = dir.join(format!("{SELECTION_FILE}.tmp"));
        let json = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, json).map_err(|e| format!("enregistrement impossible : {e}"))?;
        std::fs::rename(&tmp, dir.join(SELECTION_FILE))
            .map_err(|e| format!("enregistrement impossible : {e}"))?;
        Ok(())
    }

    /// `tools/call` → champ `result` brut (contenu + éventuel `isError`).
    pub async fn call_tool(
        &self,
        name: &str,
        tool: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        let spec = self.resolve_id(name)?;
        if !self.def_for(&spec)?.allows(tool) {
            return Err(format!(
                "outil « {tool} » non autorisé pour le serveur « {} »",
                spec.id
            ));
        }
        let tool = tool.to_string();
        let arguments = arguments.clone();
        self.with_session(&spec.id, move |s| {
            Box::pin(async move {
                s.request(
                    "tools/call",
                    json!({ "name": tool, "arguments": arguments }),
                    T_CALL,
                )
                .await
            })
        })
        .await
    }
}

/// Instance partagée par toute l'application.
pub static HUB: Lazy<Hub> = Lazy::new(Hub::new);

/// Rappel de `mcp_pack::install` : arrête le serveur avant que ses fichiers soient remplacés.
pub fn stop_before_replace(id: &str) -> futures::future::BoxFuture<'_, ()> {
    Box::pin(HUB.stop(id))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_pack::tests::TmpDir;

    /// Faux serveur MCP (Python) : un outil par comportement à tester.
    const FAKE_SERVER: &str = r#"
import sys, json, os, time
print("bannière non JSON sur stdout", flush=True)
sys.stderr.write("log sur stderr\n"); sys.stderr.flush()
def send(o): sys.stdout.write(json.dumps(o) + "\n"); sys.stdout.flush()
TOOLS_P1 = [{"name": "echo", "description": "renvoie le texte", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}]
TOOLS_P2 = [{"name": "pid", "description": "pid"}, {"name": "env", "description": "env"}, {"name": "boom", "description": "x"},
            {"name": "crash", "description": "x"}, {"name": "argv", "description": "x"}, {"name": "slow", "description": "x"}, {"name": "big", "description": "x"}]
for line in sys.stdin:
    m = json.loads(line)
    if "method" not in m or "id" not in m:
        continue                       # notifications, et réponses du client (ex. à notre ping)
    i, method, p = m["id"], m["method"], m.get("params", {})
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": i, "result": {"protocolVersion": "2025-03-26", "capabilities": {"tools": {}},
                                                      "serverInfo": {"name": "fake", "version": "0"}}})
    elif method == "tools/list":
        if p.get("cursor") == "p2":
            send({"jsonrpc": "2.0", "id": i, "result": {"tools": TOOLS_P2}})
        else:
            send({"jsonrpc": "2.0", "id": i, "result": {"tools": TOOLS_P1, "nextCursor": "p2"}})
    elif method == "tools/call":
        name, a = p["name"], p.get("arguments", {})
        # notification + requête serveur→client (ping) avant la réponse : doivent être gérées
        send({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "x"}})
        send({"jsonrpc": "2.0", "id": 900 + i, "method": "ping"})
        if name == "echo":
            send({"jsonrpc": "2.0", "id": i, "result": {"content": [{"type": "text", "text": "écho : " + a.get("text", "")}]}})
        elif name == "pid":
            send({"jsonrpc": "2.0", "id": i, "result": {"content": [{"type": "text", "text": str(os.getpid())}]}})
        elif name == "env":
            send({"jsonrpc": "2.0", "id": i, "result": {"content": [{"type": "text", "text": os.environ.get(a["var"], "<absent>")}]}})
        elif name == "argv":
            send({"jsonrpc": "2.0", "id": i, "result": {"content": [{"type": "text", "text": json.dumps(sys.argv[1:])}]}})
        elif name == "boom":
            send({"jsonrpc": "2.0", "id": i, "result": {"isError": True, "content": [{"type": "text", "text": "échec métier"}]}})
        elif name == "crash":
            sys.exit(3)
        elif name == "slow":
            time.sleep(30)
        elif name == "big":
            send({"jsonrpc": "2.0", "id": i, "result": {"content": [{"type": "text", "text": "é" * 3_000_000}]}})
        else:
            send({"jsonrpc": "2.0", "id": i, "error": {"code": -32602, "message": "outil inconnu : " + name}})
"#;

    fn python() -> Option<&'static str> {
        if cfg!(windows) {
            return None; // les tests lancent des scripts `#!/bin/sh`
        }
        ["python3", "python"].into_iter().find(|p| {
            std::process::Command::new(p)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
    }

    macro_rules! need_python {
        () => {
            match python() {
                Some(p) => p,
                None => {
                    eprintln!("python/unix indisponible : test ignoré");
                    return;
                }
            }
        };
    }

    fn text(result: &Value) -> String {
        result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    /// Installe un faux serveur au format du pack : `<dir>/<id>/manifest.json`
    /// + `bin/run` (script qui lance le faux serveur Python).
    fn install_fake(dir: &std::path::Path, id: &str, py: &str, manifest_extra: Value) {
        let sd = dir.join(id);
        std::fs::create_dir_all(sd.join("bin")).unwrap();
        std::fs::write(sd.join("fake_mcp.py"), FAKE_SERVER).unwrap();
        let run = sd.join("bin/run");
        std::fs::write(
            &run,
            format!(
                "#!/bin/sh\nexec \"{py}\" \"{}\" \"$@\"\n",
                sd.join("fake_mcp.py").display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut m = json!({ "id": id, "name": format!("Faux {id}"), "description": "test",
                            "bin": { mcp_pack::platform_key(): "bin/run" } });
        if let (Some(mo), Some(eo)) = (m.as_object_mut(), manifest_extra.as_object()) {
            for (k, v) in eo {
                mo.insert(k.clone(), v.clone());
            }
        }
        std::fs::write(sd.join("manifest.json"), m.to_string()).unwrap();
    }

    fn fake_spec(id: &str, env: &[(&str, &str)], requires: &[&str], tools: &[&str]) -> Spec {
        Spec {
            id: id.into(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            requires: requires.iter().map(|s| s.to_string()).collect(),
            tools: tools.iter().map(|s| s.to_string()).collect(),
        }
    }

    struct Fx {
        dir: TmpDir,
        hub: Hub,
    }

    /// Hub avec un seul faux serveur `mcp-fake` (sans prérequis ni restriction d'outils).
    fn fixture(py: &str, env: &[(&str, &str)]) -> Fx {
        let dir = TmpDir::new("stdio");
        install_fake(&dir.0, "mcp-fake", py, json!({}));
        let hub = Hub::with_specs(vec![fake_spec("mcp-fake", env, &[], &[])]);
        hub.set_dir(dir.0.clone());
        Fx { dir, hub }
    }

    // ── Les futurs publics doivent être `Send` (handlers axum) ────────────────
    // Vérifié à la compilation : si une évolution rend un futur non-`Send`, ce
    // test ne compile plus — au lieu de casser `api.rs` qui n'est pas dans ce module.
    #[allow(dead_code)]
    fn futures_are_send() {
        fn check<T: Send>(_t: T) {}
        let hub = Hub::new();
        let args = json!({});
        check(hub.list_tools("x"));
        check(hub.call_tool("x", "y", &args));
        check(hub.stop("x"));
        let client = reqwest::Client::new();
        check(crate::mcp::list_tools(&client, "stdio:x"));
        check(crate::mcp::call_tool(&client, "stdio:x", "y", &args));
        check(crate::mcp::collect_all_tools(&client, &[]));
    }

    // ── Adressage, spécifications (sans processus) ────────────────────────────

    #[test]
    fn scheme_and_names() {
        assert!(is_stdio("stdio:mcp-legifrance"));
        assert!(is_stdio("  STDIO:Mcp-Legifrance "));
        assert!(is_stdio("stdio://legifrance"), "ancienne forme tolérée");
        assert!(!is_stdio("https://mcp.data.gouv.fr/mcp"));
        assert_eq!(server_name("stdio:mcp-legifrance"), "mcp-legifrance");
        assert_eq!(server_name(" stdio://judilibre/ "), "judilibre");
        assert_eq!(server_name("stdio:"), "");
    }

    #[test]
    fn env_expansion() {
        let look = |n: &str| match n {
            "A" => Some("1".to_string()),
            "B" => Some("deux".to_string()),
            _ => None,
        };
        assert_eq!(expand("x", &look), "x");
        assert_eq!(expand("${A}", &look), "1");
        assert_eq!(expand("pre-${A}-${B}-post", &look), "pre-1-deux-post");
        assert_eq!(expand("${INCONNU}", &look), "");
        assert_eq!(expand("non fermé ${A", &look), "non fermé ${A");
        assert_eq!(expand("$A {B}", &look), "$A {B}");
    }

    /// Juridix ne gère que Légifrance et Judilibre, avec des noms d'environnement
    /// vérifiés contre les vrais serveurs, et sans aucun secret en clair.
    #[test]
    fn official_specs_are_exactly_legifrance_and_judilibre() {
        let specs = official_specs();
        let ids: Vec<_> = specs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["mcp-legifrance", "mcp-judilibre"]);
        for s in &specs {
            assert!(
                !s.tools.is_empty() && s.tools.len() < 12,
                "{} : allowlist resserrée",
                s.id
            );
            assert_eq!(s.requires, [SECRET_PISTE_ID, SECRET_PISTE_SECRET]);
            for (k, v) in &s.env {
                assert!(
                    v.starts_with("${") && v.ends_with('}'),
                    "{}.{k} : un secret ne doit jamais être en clair",
                    s.id
                );
            }
        }
        let env_names = |i: usize| {
            specs[i]
                .env
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            env_names(0),
            [
                "LEGIFRANCE_CLIENT_ID",
                "LEGIFRANCE_CLIENT_SECRET",
                "LEGIFRANCE_ENV"
            ]
        );
        assert_eq!(
            env_names(1),
            ["JUDILIBRE_CLIENT_ID", "JUDILIBRE_CLIENT_SECRET"]
        );
        assert!(specs[0].tools.iter().all(|t| t.starts_with("legifrance_")));
        assert!(specs[1].tools.iter().all(|t| t.starts_with("judilibre_")));
    }

    #[test]
    fn resolve_id_accepts_known_servers_only() {
        let hub = Hub::new();
        assert_eq!(
            hub.resolve_id("mcp-legifrance").unwrap().id,
            "mcp-legifrance"
        );
        assert_eq!(
            hub.resolve_id("legifrance").unwrap().id,
            "mcp-legifrance",
            "forme courte tolérée"
        );
        assert_eq!(
            hub.resolve_id(" mcp-judilibre ").unwrap().id,
            "mcp-judilibre"
        );
        for bad in [
            "mcp-grist",
            "mcp-system",
            "mcp-image",
            "../mcp-legifrance",
            "",
            "grist",
            "mcp-",
        ] {
            assert!(
                hub.resolve_id(bad).is_err(),
                "{bad:?} ne doit pas être utilisable"
            );
        }
    }

    // ── Erreurs de configuration ──────────────────────────────────────────────

    #[tokio::test]
    async fn setup_errors_are_clear_and_never_panic() {
        let hub = Hub::new();
        assert!(hub
            .list_tools("mcp-legifrance")
            .await
            .unwrap_err()
            .contains("non initialisé"));

        let d = TmpDir::new("empty");
        hub.set_dir(d.0.clone());
        let e = hub.list_tools("mcp-legifrance").await.unwrap_err();
        assert!(e.contains("non installé") && e.contains("importez"), "{e}");
        assert!(hub
            .call_tool("mcp-grist", "x", &json!({}))
            .await
            .unwrap_err()
            .contains("non pris en charge"));
        assert!(hub
            .call_tool("mcp-legifrance", "legifrance_rechercher", &json!({}))
            .await
            .unwrap_err()
            .contains("non installé"));
    }

    #[tokio::test]
    async fn an_unrequested_server_in_the_folder_is_ignored() {
        let py = need_python!();
        let d = TmpDir::new("other");
        install_fake(&d.0, "mcp-grist", py, json!({})); // présent sur le disque, pas géré par Juridix
        let hub = Hub::new();
        hub.set_dir(d.0.clone());
        let st = hub.status();
        assert_eq!(
            st.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["mcp-legifrance", "mcp-judilibre"]
        );
        assert!(st.iter().all(|s| !s.installed));
        assert!(hub
            .call_tool("mcp-grist", "echo", &json!({}))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn missing_binary_is_a_clean_error() {
        let d = TmpDir::new("nobin");
        let sd = d.0.join("mcp-fake");
        std::fs::create_dir_all(&sd).unwrap();
        let m = json!({"id": "mcp-fake", "name": "x", "bin": { mcp_pack::platform_key(): "bin/inexistant" }});
        std::fs::write(sd.join("manifest.json"), m.to_string()).unwrap();
        let hub = Hub::with_specs(vec![fake_spec("mcp-fake", &[], &[], &[])]);
        hub.set_dir(d.0.clone());
        let e = hub.list_tools("mcp-fake").await.unwrap_err();
        assert!(e.contains("impossible"), "{e}");
    }

    #[tokio::test]
    async fn no_binary_for_this_platform_is_reported() {
        let d = TmpDir::new("plat");
        let sd = d.0.join("mcp-fake");
        std::fs::create_dir_all(&sd).unwrap();
        std::fs::write(
            sd.join("manifest.json"),
            json!({"id": "mcp-fake", "name": "x", "bin": {"plan9-mips": "bin/x"}}).to_string(),
        )
        .unwrap();
        let hub = Hub::with_specs(vec![fake_spec("mcp-fake", &[], &[], &[])]);
        hub.set_dir(d.0.clone());
        let e = hub.list_tools("mcp-fake").await.unwrap_err();
        assert!(e.contains("pas de binaire pour"), "{e}");
        let st = hub.status();
        assert!(st[0].installed && !st[0].available);
    }

    // ── Prérequis : identifiants PISTE ────────────────────────────────────────

    #[tokio::test]
    async fn server_is_not_launched_without_credentials() {
        let py = need_python!();
        let d = TmpDir::new("creds");
        install_fake(&d.0, "mcp-fake", py, json!({}));
        let hub = Hub::with_specs(vec![fake_spec(
            "mcp-fake",
            &[("PISTE_CLIENT_ID", "${JURIDIX_PISTE_CLIENT_ID}")],
            &["JURIDIX_PISTE_CLIENT_ID", "JURIDIX_PISTE_CLIENT_SECRET"],
            &[],
        )]);
        hub.set_dir(d.0.clone());

        let e = hub.list_tools("mcp-fake").await.unwrap_err();
        assert!(e.contains("identifiants PISTE non configurés"), "{e}");
        let st = hub.status();
        assert!(
            st[0].needs_credentials && !st[0].running,
            "aucun processus lancé sans identifiants"
        );

        hub.set_secret("JURIDIX_PISTE_CLIENT_ID", "id");
        assert!(
            hub.list_tools("mcp-fake").await.is_err(),
            "il manque encore le secret"
        );
        hub.set_secret("JURIDIX_PISTE_CLIENT_SECRET", "secret");
        assert!(hub.list_tools("mcp-fake").await.is_ok());
        let st = hub.status();
        assert!(!st[0].needs_credentials && st[0].running);
    }

    // ── Avec un vrai processus (faux serveur) ─────────────────────────────────

    #[tokio::test]
    async fn list_tools_follows_pagination_and_tolerates_noise() {
        let py = need_python!();
        let fx = fixture(py, &[]);
        let tools = fx.hub.list_tools("mcp-fake").await.unwrap();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            ["echo", "pid", "env", "boom", "crash", "argv", "slow", "big"]
        );
    }

    #[tokio::test]
    async fn call_tool_roundtrip_with_server_pings_and_unicode() {
        let py = need_python!();
        let fx = fixture(py, &[]);
        let r = fx
            .hub
            .call_tool(
                "mcp-fake",
                "echo",
                &json!({"text": "article 1103 — é à 😀"}),
            )
            .await
            .unwrap();
        assert_eq!(text(&r), "écho : article 1103 — é à 😀");
    }

    #[tokio::test]
    async fn session_is_reused_across_calls() {
        let py = need_python!();
        let fx = fixture(py, &[]);
        let a = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        let b = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(a, b, "un seul processus pour les deux appels");
    }

    #[tokio::test]
    async fn concurrent_calls_are_serialized_and_all_succeed() {
        let py = need_python!();
        let fx = Arc::new(fixture(py, &[]));
        let mut handles = Vec::new();
        for n in 0..6 {
            let fx = fx.clone();
            handles.push(tokio::spawn(async move {
                let r = fx
                    .hub
                    .call_tool("mcp-fake", "echo", &json!({"text": n.to_string()}))
                    .await
                    .unwrap();
                (n, text(&r))
            }));
        }
        for h in handles {
            let (n, t) = h.await.unwrap();
            assert_eq!(t, format!("écho : {n}"), "chaque appel reçoit SA réponse");
        }
    }

    /// Le manifest n'apporte que le chemin du binaire : il ne peut ni ajouter
    /// d'arguments ni injecter de variables d'environnement.
    #[tokio::test]
    async fn manifest_cannot_inject_arguments_or_environment() {
        let py = need_python!();
        let d = TmpDir::new("inject");
        install_fake(
            &d.0,
            "mcp-fake",
            py,
            json!({ "args": ["--evil", "--exfiltrate"], "env": { "EVIL": "1" }, "params": [{"key": "EVIL"}] }),
        );
        let hub = Hub::with_specs(vec![fake_spec("mcp-fake", &[], &[], &[])]);
        hub.set_dir(d.0.clone());
        let argv = text(&hub.call_tool("mcp-fake", "argv", &json!({})).await.unwrap());
        assert_eq!(argv, r#"["--transport", "stdio"]"#);
        assert_eq!(
            text(
                &hub.call_tool("mcp-fake", "env", &json!({"var": "EVIL"}))
                    .await
                    .unwrap()
            ),
            "<absent>"
        );
    }

    #[tokio::test]
    async fn secrets_and_process_env_are_expanded_into_the_child_env() {
        let py = need_python!();
        std::env::set_var("JURIDIX_TEST_PROC_VAR", "depuis-le-processus");
        let fx = fixture(
            py,
            &[
                ("PISTE_CLIENT_ID", "${JURIDIX_PISTE_CLIENT_ID}"),
                ("DEPUIS_ENV", "${JURIDIX_TEST_PROC_VAR}"),
                ("LITTERAL", "tel-quel"),
                ("VIDE", "${JURIDIX_TEST_JAMAIS_DEFINI}"),
            ],
        );
        fx.hub.set_secret("JURIDIX_PISTE_CLIENT_ID", "id-secret-1");
        let get = |v: &'static str| {
            let hub = &fx.hub;
            async move {
                text(
                    &hub.call_tool("mcp-fake", "env", &json!({"var": v}))
                        .await
                        .unwrap(),
                )
            }
        };
        assert_eq!(get("PISTE_CLIENT_ID").await, "id-secret-1");
        assert_eq!(get("DEPUIS_ENV").await, "depuis-le-processus");
        assert_eq!(get("LITTERAL").await, "tel-quel");
        assert_eq!(
            get("VIDE").await,
            "<absent>",
            "valeur vide = variable non définie (le serveur garde son défaut)"
        );
    }

    #[tokio::test]
    async fn changing_a_secret_restarts_the_server_with_the_new_value() {
        let py = need_python!();
        let fx = fixture(py, &[("PISTE_CLIENT_ID", "${JURIDIX_PISTE_CLIENT_ID}")]);
        fx.hub.set_secret("JURIDIX_PISTE_CLIENT_ID", "v1");
        let pid1 = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        // Même valeur : pas de relance.
        fx.hub.set_secret("JURIDIX_PISTE_CLIENT_ID", "v1");
        assert_eq!(
            pid1,
            text(
                &fx.hub
                    .call_tool("mcp-fake", "pid", &json!({}))
                    .await
                    .unwrap()
            )
        );
        // Valeur changée : relance, et le nouvel environnement est pris en compte.
        fx.hub.set_secret("JURIDIX_PISTE_CLIENT_ID", "v2");
        let v = text(
            &fx.hub
                .call_tool("mcp-fake", "env", &json!({"var": "PISTE_CLIENT_ID"}))
                .await
                .unwrap(),
        );
        assert_eq!(v, "v2");
        assert_ne!(
            pid1,
            text(
                &fx.hub
                    .call_tool("mcp-fake", "pid", &json!({}))
                    .await
                    .unwrap()
            )
        );
    }

    #[tokio::test]
    async fn remote_errors_keep_the_session_and_is_error_is_passed_through() {
        let py = need_python!();
        let fx = fixture(py, &[]);
        let pid = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        let e = fx
            .hub
            .call_tool("mcp-fake", "nexiste_pas", &json!({}))
            .await
            .unwrap_err();
        assert!(e.contains("outil inconnu"), "{e}");
        assert_eq!(
            pid,
            text(
                &fx.hub
                    .call_tool("mcp-fake", "pid", &json!({}))
                    .await
                    .unwrap()
            )
        );
        let r = fx
            .hub
            .call_tool("mcp-fake", "boom", &json!({}))
            .await
            .unwrap();
        assert_eq!(r["isError"], true);
        assert_eq!(text(&r), "échec métier");
    }

    #[tokio::test]
    async fn crash_fails_the_call_then_the_next_call_respawns() {
        let py = need_python!();
        let fx = fixture(py, &[]);
        let pid1 = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        let e = fx
            .hub
            .call_tool("mcp-fake", "crash", &json!({}))
            .await
            .unwrap_err();
        assert!(
            e.contains("arrêté") || e.contains("lecture") || e.contains("écriture"),
            "{e}"
        );
        let pid2 = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        assert_ne!(pid1, pid2, "un nouveau processus a été lancé");
    }

    #[tokio::test]
    async fn large_results_come_through() {
        let py = need_python!();
        let fx = fixture(py, &[]);
        let r = fx
            .hub
            .call_tool("mcp-fake", "big", &json!({}))
            .await
            .unwrap();
        assert_eq!(text(&r).chars().count(), 3_000_000);
    }

    #[tokio::test]
    async fn status_and_stop_follow_the_process_lifecycle() {
        let py = need_python!();
        let fx = fixture(py, &[]);
        let st = fx.hub.status();
        assert_eq!(
            (st[0].installed, st[0].available, st[0].running),
            (true, true, false)
        );
        assert_eq!(st[0].name, "Faux mcp-fake");

        let pid1 = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        assert!(fx.hub.status()[0].running);
        fx.hub.stop("mcp-fake").await;
        assert!(!fx.hub.status()[0].running, "arrêté");
        let pid2 = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        assert_ne!(pid1, pid2, "relancé à la demande après l'arrêt");
        fx.hub.stop("inconnu").await; // sans effet, sans panique
    }

    // ── Allowlist ─────────────────────────────────────────────────────────────

    #[test]
    fn allowlist_semantics() {
        let mk = |t: Option<Vec<&str>>| ServerDef {
            command: "x".into(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            tools: t.map(|v| v.into_iter().map(String::from).collect()),
        };
        assert!(mk(None).allows("n_importe_quoi"), "None = tous les outils");
        assert!(
            !mk(Some(vec![])).allows("n_importe_quoi"),
            "liste vide = AUCUN outil (jamais « tous »)"
        );
        let d = mk(Some(vec!["legifrance_article_par_numero", "judilibre_*"]));
        assert!(d.allows("legifrance_article_par_numero"));
        assert!(
            !d.allows("legifrance_article_par_numero_bis"),
            "nom exact, pas préfixe"
        );
        assert!(!d.allows("legifrance_rechercher"));
        assert!(
            d.allows("judilibre_rechercher") && d.allows("judilibre_"),
            "préfixe terminé par *"
        );
        assert!(!d.allows("xjudilibre_rechercher"));
    }

    // ── Sélection d'outils (Essentiels / Tous / Personnalisé) ──────────────────

    /// Hub avec un faux serveur dont les « Essentiels » sont `echo` et `bo*`.
    fn selection_fixture(py: &str) -> Fx {
        let dir = TmpDir::new("sel");
        install_fake(&dir.0, "mcp-fake", py, json!({}));
        let hub = Hub::with_specs(vec![fake_spec("mcp-fake", &[], &[], &["echo", "bo*"])]);
        hub.set_dir(dir.0.clone());
        Fx { dir, hub }
    }

    fn names(v: &[Value]) -> Vec<String> {
        v.iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn selection_modes_change_what_is_exposed_and_callable() {
        let py = need_python!();
        let fx = selection_fixture(py);
        let hub = &fx.hub;

        // Par défaut : « Essentiels ».
        assert_eq!(hub.selection("mcp-fake"), ToolSelection::Essential);
        assert_eq!(
            names(&hub.list_tools("mcp-fake").await.unwrap()),
            ["echo", "boom"]
        );
        assert!(hub
            .call_tool("mcp-fake", "pid", &json!({}))
            .await
            .unwrap_err()
            .contains("non autorisé"));

        // « Tous » : les 8 outils du serveur, sans redémarrer le serveur.
        hub.set_selection("mcp-fake", ToolSelection::All)
            .await
            .unwrap();
        assert_eq!(hub.list_tools("mcp-fake").await.unwrap().len(), 8);
        assert!(
            hub.call_tool("mcp-fake", "pid", &json!({})).await.is_ok(),
            "un outil jusque-là refusé est appelable"
        );

        // « Personnalisé » : exactement la liste choisie.
        hub.set_selection(
            "mcp-fake",
            ToolSelection::Custom {
                tools: vec!["pid".into(), "argv".into(), "pid".into()],
            },
        )
        .await
        .unwrap();
        assert_eq!(
            names(&hub.list_tools("mcp-fake").await.unwrap()),
            ["pid", "argv"],
            "doublons retirés"
        );
        assert!(
            hub.call_tool("mcp-fake", "echo", &json!({}))
                .await
                .unwrap_err()
                .contains("non autorisé"),
            "echo n'est plus choisi"
        );

        // Retour aux « Essentiels ».
        hub.set_selection("mcp-fake", ToolSelection::Essential)
            .await
            .unwrap();
        assert_eq!(
            names(&hub.list_tools("mcp-fake").await.unwrap()),
            ["echo", "boom"]
        );
    }

    #[tokio::test]
    async fn custom_selection_is_validated_against_the_real_catalog() {
        let py = need_python!();
        let fx = selection_fixture(py);
        let hub = &fx.hub;
        let custom = |t: &[&str]| ToolSelection::Custom {
            tools: t.iter().map(|s| s.to_string()).collect(),
        };

        assert!(hub
            .set_selection("mcp-fake", custom(&[]))
            .await
            .unwrap_err()
            .contains("au moins un outil"));
        assert!(hub
            .set_selection("mcp-fake", custom(&["", "  "]))
            .await
            .unwrap_err()
            .contains("au moins un outil"));
        assert!(
            hub.set_selection("mcp-fake", custom(&["echo", "nexiste_pas"]))
                .await
                .unwrap_err()
                .contains("n'existe pas"),
            "faute de frappe refusée"
        );
        assert!(hub
            .set_selection("mcp-fake", custom(&["../evil"]))
            .await
            .unwrap_err()
            .contains("invalide"));
        assert!(
            hub.set_selection("mcp-fake", custom(&["echo*"]))
                .await
                .unwrap_err()
                .contains("invalide"),
            "pas de motifs : des noms réels"
        );
        // Un refus ne modifie rien.
        assert_eq!(hub.selection("mcp-fake"), ToolSelection::Essential);
        assert_eq!(
            names(&hub.list_tools("mcp-fake").await.unwrap()),
            ["echo", "boom"]
        );
        // Serveur inconnu.
        assert!(hub
            .set_selection("mcp-grist", ToolSelection::All)
            .await
            .unwrap_err()
            .contains("non pris en charge"));
    }

    #[tokio::test]
    async fn selection_is_persisted_and_survives_a_restart() {
        let py = need_python!();
        let fx = selection_fixture(py);
        fx.hub
            .set_selection(
                "mcp-fake",
                ToolSelection::Custom {
                    tools: vec!["pid".into()],
                },
            )
            .await
            .unwrap();
        let file = fx.dir.0.join(SELECTION_FILE);
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            on_disk["mcp-fake"],
            json!({"mode": "custom", "tools": ["pid"]})
        );
        assert!(
            !fx.dir.0.join(format!("{SELECTION_FILE}.tmp")).exists(),
            "pas de fichier temporaire résiduel"
        );

        // « Redémarrage » : un nouveau Hub relit le fichier.
        let hub2 = Hub::with_specs(vec![fake_spec("mcp-fake", &[], &[], &["echo"])]);
        hub2.set_dir(fx.dir.0.clone());
        assert_eq!(
            hub2.selection("mcp-fake"),
            ToolSelection::Custom {
                tools: vec!["pid".into()]
            }
        );
        assert_eq!(names(&hub2.list_tools("mcp-fake").await.unwrap()), ["pid"]);

        // « Tous » est aussi mémorisé ; « Essentiels » retire l'entrée.
        fx.hub
            .set_selection("mcp-fake", ToolSelection::All)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&std::fs::read_to_string(&file).unwrap()).unwrap()
                ["mcp-fake"],
            json!({"mode": "all"})
        );
        fx.hub
            .set_selection("mcp-fake", ToolSelection::Essential)
            .await
            .unwrap();
        assert!(
            serde_json::from_str::<Value>(&std::fs::read_to_string(&file).unwrap())
                .unwrap()
                .get("mcp-fake")
                .is_none()
        );
    }

    #[test]
    fn a_corrupt_selection_file_falls_back_to_defaults() {
        let d = TmpDir::new("selbad");
        std::fs::write(d.0.join(SELECTION_FILE), "{ pas du json").unwrap();
        let hub = Hub::new();
        hub.set_dir(d.0.clone());
        assert_eq!(hub.selection("mcp-legifrance"), ToolSelection::Essential);
        // Valeur inconnue : même repli (jamais d'erreur au démarrage).
        std::fs::write(
            d.0.join(SELECTION_FILE),
            r#"{"mcp-legifrance": {"mode": "inconnu"}}"#,
        )
        .unwrap();
        hub.set_dir(d.0.clone());
        assert_eq!(hub.selection("mcp-legifrance"), ToolSelection::Essential);
    }

    #[tokio::test]
    async fn catalog_reports_totals_costs_and_exposure() {
        let py = need_python!();
        let fx = selection_fixture(py);
        let hub = &fx.hub;

        // Avant d'avoir interrogé le serveur : total inconnu, exposés = taille de la liste.
        let st = &hub.status()[0];
        assert_eq!((st.tools_total, st.tools_exposed), (None, Some(2)));

        let cat = hub.catalog("mcp-fake").await.unwrap();
        assert_eq!(cat.total, 8);
        assert_eq!(cat.selection, ToolSelection::Essential);
        let exposed: Vec<&str> = cat
            .tools
            .iter()
            .filter(|t| t.exposed)
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(exposed, ["echo", "boom"]);
        let essential: Vec<&str> = cat
            .tools
            .iter()
            .filter(|t| t.essential)
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(essential, ["echo", "boom"]);
        assert!(cat.tools.iter().all(|t| t.tokens > 0));
        assert_eq!(
            cat.tokens_exposed,
            cat.tools
                .iter()
                .filter(|t| t.exposed)
                .map(|t| t.tokens)
                .sum::<usize>()
        );

        // Le statut connaît maintenant le total : « 2 sur 8 ».
        let st = &hub.status()[0];
        assert_eq!((st.tools_total, st.tools_exposed), (Some(8), Some(2)));

        hub.set_selection("mcp-fake", ToolSelection::All)
            .await
            .unwrap();
        let all = hub.catalog("mcp-fake").await.unwrap();
        assert!(all.tools.iter().all(|t| t.exposed));
        assert!(
            all.tokens_exposed > cat.tokens_exposed,
            "tout exposer coûte plus de jetons"
        );
        let st = &hub.status()[0];
        assert_eq!((st.tools_total, st.tools_exposed), (Some(8), Some(8)));
        // « Essentiels » restent identifiables même quand tout est exposé.
        assert_eq!(all.tools.iter().filter(|t| t.essential).count(), 2);
    }

    #[tokio::test]
    async fn allowlist_filters_the_catalog_and_blocks_calls() {
        let py = need_python!();
        let d = TmpDir::new("allow");
        for id in ["mcp-restreint", "mcp-ouvert", "mcp-faute"] {
            install_fake(&d.0, id, py, json!({}));
        }
        let hub = Hub::with_specs(vec![
            fake_spec("mcp-restreint", &[], &[], &["echo", "bo*"]),
            fake_spec("mcp-ouvert", &[], &[], &[]),
            fake_spec("mcp-faute", &[], &[], &["echoo"]),
        ]);
        hub.set_dir(d.0.clone());

        let names = |v: Vec<Value>| {
            v.iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(hub.list_tools("mcp-restreint").await.unwrap()),
            ["echo", "boom"]
        );
        assert_eq!(hub.list_tools("mcp-ouvert").await.unwrap().len(), 8);
        assert!(
            hub.list_tools("mcp-faute").await.unwrap().is_empty(),
            "outil renommé : rien d'exposé (et un warn)"
        );

        assert_eq!(
            text(
                &hub.call_tool("mcp-restreint", "echo", &json!({"text": "ok"}))
                    .await
                    .unwrap()
            ),
            "écho : ok"
        );
        for interdit in ["crash", "pid"] {
            let e = hub
                .call_tool("mcp-restreint", interdit, &json!({}))
                .await
                .unwrap_err();
            assert!(e.contains("non autorisé"), "{interdit} : {e}");
        }
        assert!(hub.call_tool("mcp-ouvert", "pid", &json!({})).await.is_ok());
    }

    #[tokio::test]
    async fn timeout_abandons_the_session() {
        let py = need_python!();
        let fx = fixture(py, &[]);
        let pid1 = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        // On borne l'attente à la main : T_CALL (60 s) est trop long pour un test.
        let slot = fx.hub.slot("mcp-fake");
        {
            let mut g = slot.lock().await;
            let s = g.as_mut().unwrap();
            let e = s
                .request(
                    "tools/call",
                    json!({"name": "slow", "arguments": {}}),
                    Duration::from_millis(300),
                )
                .await;
            assert!(
                matches!(e, Err(RpcError::Transport(ref m)) if m.contains("délai")),
                "{e:?}"
            );
            *g = None; // ce que fait with_session sur erreur de transport
        }
        let pid2 = text(
            &fx.hub
                .call_tool("mcp-fake", "pid", &json!({}))
                .await
                .unwrap(),
        );
        assert_ne!(pid1, pid2);
    }

    // ── Via mcp.rs (ce qu'utilise la boucle de chat) ──────────────────────────

    /// Passe par l'API publique de `mcp.rs` et par le HUB global. Seul test à
    /// toucher au global : pas de course entre tests.
    #[tokio::test]
    async fn mcp_module_dispatches_stdio_urls_end_to_end() {
        let py = need_python!();
        let d = TmpDir::new("global");
        install_fake(&d.0, "mcp-fake", py, json!({}));
        HUB.replace_specs(vec![fake_spec("mcp-fake", &[], &[], &[])]);
        HUB.set_dir(d.0.clone());
        let client = reqwest::Client::new();

        // Découverte : outils nommés mcp__<slug>__<outil>, slug sans le préfixe « mcp- ».
        let all = crate::mcp::collect_all_tools(&client, &["stdio:mcp-fake".to_string()]).await;
        assert_eq!(all.len(), 8);
        let (server, echo) = all.iter().find(|(_, t)| t["name"] == "echo").unwrap();
        let oa = crate::mcp::mcp_tool_to_openai(echo, server);
        assert_eq!(oa["function"]["name"], "mcp__fake__echo");
        assert_eq!(oa["type"], "function");

        let r =
            crate::mcp::call_tool(&client, "stdio:mcp-fake", "echo", &json!({"text": "ok"})).await;
        assert_eq!(r.text, "écho : ok");
        // isError : le LLM doit voir qu'il s'agit d'un échec.
        let r = crate::mcp::call_tool(&client, "stdio:mcp-fake", "boom", &json!({})).await;
        assert_eq!(r.text, "Erreur de l'outil boom : échec métier");
        // Erreur JSON-RPC : texte d'erreur, pas de panique.
        let r = crate::mcp::call_tool(&client, "stdio:mcp-fake", "nexiste_pas", &json!({})).await;
        assert!(
            r.text
                .starts_with("Erreur : impossible d'appeler le tool nexiste_pas"),
            "{}",
            r.text
        );
        // Serveur non géré : liste vide + ok=false (le panneau d'état affichera « erreur »).
        let (tools, ok) = crate::mcp::list_tools(&client, "stdio:mcp-grist").await;
        assert!(tools.is_empty() && !ok);
        // Ancienne forme tolérée.
        assert!(crate::mcp::list_tools(&client, "stdio://fake").await.1);

        // Routes d'API de l'écran des réglages : état puis arrêt.
        async fn body(r: axum::response::Response) -> Value {
            let bytes = axum::body::to_bytes(r.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }
        let st = body(crate::api::mcp_stdio_servers().await).await;
        assert_eq!(st["platform"], mcp_pack::platform_key().as_str());
        assert!(
            st["pack_version"].is_null(),
            "rien d'importé depuis une release dans ce test"
        );
        let srv = &st["servers"][0];
        assert_eq!(
            (
                srv["id"].as_str(),
                srv["installed"].as_bool(),
                srv["running"].as_bool()
            ),
            (Some("mcp-fake"), Some(true), Some(true))
        );
        assert!(
            srv.get("env").is_none() && srv.get("command").is_none(),
            "aucun détail de lancement ni secret exposé"
        );
        // Catalogue et sélection d'outils (écran des réglages).
        let cat = body(
            crate::api::mcp_stdio_tools(axum::extract::Query(crate::api::McpToolsQuery {
                id: "mcp-fake".into(),
            }))
            .await,
        )
        .await;
        assert_eq!(
            (cat["total"].as_u64(), cat["selection"]["mode"].as_str()),
            (Some(8), Some("essential"))
        );
        assert!(
            cat["tools"][0]["tokens"].as_u64().unwrap() > 0
                && cat["tokens_exposed"].as_u64().unwrap() > 0
        );
        let set = |sel: Value| {
            let req: crate::api::McpSelectionRequest = serde_json::from_value(sel).unwrap();
            crate::api::mcp_stdio_set_tools(axum::Json(req))
        };
        let r = set(json!({"id": "mcp-fake", "mode": "custom", "tools": ["echo", "pid"]})).await;
        assert_eq!(r.status(), 200);
        let b = body(r).await;
        assert_eq!(
            (
                b["selection"]["mode"].as_str(),
                b["selection"]["tools"].clone()
            ),
            (Some("custom"), json!(["echo", "pid"]))
        );
        assert_eq!(
            b["tools"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|t| t["exposed"] == true)
                .count(),
            2
        );
        // Refus → 400 avec un message ; la sélection précédente est conservée.
        let r = set(json!({"id": "mcp-fake", "mode": "custom", "tools": ["nexiste_pas"]})).await;
        assert_eq!(r.status(), 400);
        assert!(body(r).await["error"]
            .as_str()
            .unwrap()
            .contains("n'existe pas"));
        let r = set(json!({"id": "mcp-fake", "mode": "custom", "tools": []})).await;
        assert_eq!(r.status(), 400);
        assert_eq!(
            set(json!({"id": "mcp-fake", "mode": "essential"}))
                .await
                .status(),
            200
        );
        let st = body(crate::api::mcp_stdio_servers().await).await;
        assert_eq!(st["servers"][0]["selection"]["mode"], "essential");
        assert_eq!(
            (
                st["servers"][0]["tools_total"].as_u64(),
                st["servers"][0]["tools_exposed"].as_u64()
            ),
            (Some(8), Some(8)),
            "spec sans liste = tous"
        );

        let r = crate::api::mcp_stdio_stop(axum::Json(crate::api::McpStopRequest {
            id: "mcp-fake".into(),
        }))
        .await;
        assert_eq!(body(r).await["ok"], true);
        let st = body(crate::api::mcp_stdio_servers().await).await;
        assert_eq!(st["servers"][0]["running"], false);
    }

    #[test]
    fn stdio_slugs_are_short_distinct_and_safe_for_routing() {
        let slug = |n: &str| crate::mcp::server_slug(&format!("stdio:{n}"));
        assert_eq!(slug("mcp-legifrance"), "legifrance");
        assert_eq!(slug("mcp-judilibre"), "judilibre");
        assert_eq!(slug("legifrance"), "legifrance", "forme courte identique");

        // Jamais de « __ » (api.rs découpe sur ce séparateur), jamais > 15 caractères.
        for n in [
            "a__b",
            "mon-serveur.v2",
            "x___y___z",
            "__lead",
            "trail__",
            "un-nom-de-serveur-tres-long-a",
            "éèà",
        ] {
            let s = slug(n);
            assert!(!s.contains("__"), "{n} → {s}");
            assert!(s.len() <= 15, "{n} → {s} ({} car.)", s.len());
            assert!(
                !s.is_empty() && !s.starts_with('_') && !s.ends_with('_'),
                "{n} → {s}"
            );
            assert!(
                s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "{n} → {s}"
            );
        }
        assert_ne!(
            slug("un-nom-de-serveur-tres-long-a"),
            slug("un-nom-de-serveur-tres-long-b")
        );
        assert_ne!(
            slug("a-b"),
            slug("a_b"),
            "« a-b » et « a_b » nettoyés pareil : le hachage les distingue"
        );
        assert_eq!(
            slug("un-nom-de-serveur-tres-long-a"),
            slug("un-nom-de-serveur-tres-long-a"),
            "déterministe"
        );

        // Routage de api.rs : préfixe terminé par « __ » et nom d'outil récupéré par splitn(3, "__").
        let (a, b) = (slug("legifrance"), slug("legifrance_b"));
        let tool = format!("mcp__{b}__getArticle");
        assert!(
            !tool.starts_with(&format!("mcp__{a}__")),
            "« legifrance » ne doit pas capter « legifrance_b »"
        );
        let parts: Vec<&str> = tool.splitn(3, "__").collect();
        assert_eq!((parts[1], parts[2]), (b.as_str(), "getArticle"));
        let with_sep = format!("mcp__{a}__ma__fonction");
        let parts: Vec<&str> = with_sep.splitn(3, "__").collect();
        assert_eq!(parts[2], "ma__fonction");
    }

    // ── Bout en bout : import depuis GitHub puis lancement des VRAIS serveurs ──

    /// Ce que fera l'application : importer la release, puis lancer Légifrance et
    /// Judilibre. Ignoré par défaut (réseau + exécution des binaires publiés) :
    ///   cargo test --lib real_import_then_launch -- --ignored --nocapture
    /// Aucun identifiant PISTE valide n'est nécessaire (protocole seulement).
    #[tokio::test]
    #[ignore = "nécessite github.com et exécute les binaires publiés"]
    async fn real_import_then_launch() {
        let d = TmpDir::new("e2e");
        let hub = Hub::new();
        hub.set_dir(d.0.clone());
        // 1. Avant import : rien d'installé, message explicite.
        let e = hub.list_tools("mcp-legifrance").await.unwrap_err();
        assert!(e.contains("non installé"), "{e}");

        // 2. Import (Légifrance + Judilibre uniquement).
        let src = mcp_pack::PackSource {
            pinned: Some("v0.3.1".into()),
            ..mcp_pack::PackSource::default()
        };
        fn stop(_: &str) -> futures::future::BoxFuture<'_, ()> {
            Box::pin(async {}) // rien ne tourne encore à ce stade
        }
        let rep = mcp_pack::install(&src, &d.0, &hub.ids(), &stop)
            .await
            .unwrap();
        assert!(rep.servers.iter().all(|s| s.installed), "{rep:?}");

        // 3. Sans identifiants : pas de lancement.
        let e = hub.list_tools("mcp-legifrance").await.unwrap_err();
        assert!(e.contains("identifiants PISTE"), "{e}");

        // 4. Avec des identifiants factices : les vrais serveurs démarrent et répondent.
        hub.set_secret(SECRET_PISTE_ID, "x");
        hub.set_secret(SECRET_PISTE_SECRET, "y");
        let mut longest = 0usize;
        for spec in official_specs() {
            let tools = hub
                .list_tools(&spec.id)
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", spec.id));
            let mut got: Vec<String> = tools
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect();
            let mut want = spec.tools.clone();
            got.sort();
            want.sort();
            assert_eq!(
                got, want,
                "{} : l'allowlist doit désigner des outils qui existent",
                spec.id
            );
            let bytes: usize = tools.iter().map(|t| t.to_string().len()).sum();
            println!(
                "{} : {} outils exposés (≈ {} jetons)",
                spec.id,
                got.len(),
                bytes / 4
            );
            for n in &got {
                let full = format!(
                    "mcp__{}__{n}",
                    crate::mcp::server_slug(&format!("stdio:{}", spec.id))
                );
                longest = longest.max(full.len());
                assert!(full.len() <= 64, "nom d'outil > 64 caractères : {full}");
            }
        }
        println!("nom d'outil le plus long côté LLM : {longest} caractères");

        // 5. Appel sans accès à PISTE : échec propre signalé, session conservée.
        let r = hub
            .call_tool(
                "mcp-legifrance",
                "legifrance_article_par_numero",
                &json!({"code": "civil", "numero": "1103"}),
            )
            .await
            .unwrap();
        println!("appel → {}", text(&r).chars().take(100).collect::<String>());
        assert_eq!(r["isError"], true);
        assert!(hub.list_tools("mcp-legifrance").await.is_ok());

        // 5 bis. Le VRAI nombre d'outils des serveurs, et les trois modes de sélection.
        for (id, total_attendu, essentiels) in
            [("mcp-legifrance", 62usize, 6usize), ("mcp-judilibre", 8, 3)]
        {
            let cat = hub.catalog(id).await.unwrap();
            println!(
                "{id} : {} outils au catalogue, {} exposés (Essentiels) ≈ {} jetons",
                cat.total,
                cat.tools.iter().filter(|t| t.exposed).count(),
                cat.tokens_exposed
            );
            assert_eq!(cat.total, total_attendu, "{id} : total réel");
            assert_eq!(cat.tools.iter().filter(|t| t.exposed).count(), essentiels);
            let st = hub.status().into_iter().find(|s| s.id == id).unwrap();
            assert_eq!(
                (st.tools_total, st.tools_exposed),
                (Some(total_attendu), Some(essentiels))
            );

            hub.set_selection(id, ToolSelection::All).await.unwrap();
            let all = hub.list_tools(id).await.unwrap();
            assert_eq!(
                all.len(),
                total_attendu,
                "{id} : « Tous » expose le catalogue complet"
            );
            let all_cat = hub.catalog(id).await.unwrap();
            println!(
                "{id} : « Tous » = {} outils ≈ {} jetons par requête",
                all.len(),
                all_cat.tokens_exposed
            );
            for t in &all {
                let full = format!(
                    "mcp__{}__{}",
                    crate::mcp::server_slug(&format!("stdio:{id}")),
                    t["name"].as_str().unwrap()
                );
                assert!(
                    full.len() <= 64,
                    "nom d'outil > 64 caractères avec « Tous » : {full}"
                );
            }
            hub.set_selection(id, ToolSelection::Essential)
                .await
                .unwrap();
            assert_eq!(hub.list_tools(id).await.unwrap().len(), essentiels);
        }

        // 6. Un outil hors allowlist est refusé sans atteindre le serveur.
        let e = hub
            .call_tool(
                "mcp-legifrance",
                "legifrance_lister_questions_parlementaires",
                &json!({}),
            )
            .await
            .unwrap_err();
        assert!(e.contains("non autorisé"), "{e}");
    }
}
