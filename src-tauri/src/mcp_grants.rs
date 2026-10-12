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

//! Autorisation des serveurs MCP — « une fois par serveur »

use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Table des autorisations (créée avec le reste du schéma, voir `db.rs`).
pub const SCHEMA_SQL: &str = "CREATE TABLE IF NOT EXISTS mcp_grants (
    server      TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    tools       TEXT NOT NULL,
    granted_at  TEXT NOT NULL
);";

/// Une autorisation enregistrée.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Grant {
    pub server: String,
    pub fingerprint: String,
    /// Noms des outils autorisés (information pour l'interface ; l'empreinte fait foi).
    pub tools: Vec<String>,
    pub granted_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    /// Serveur officiel : fiable par construction, aucune autorisation requise.
    Trusted,
    /// Autorisé, et ses outils sont ceux qui ont été examinés.
    Granted,
    /// Jamais autorisé.
    New,
    /// Autorisé autrefois, mais ses outils ont changé depuis.
    Changed,
    /// Injoignable : rien à autoriser ni à exécuter pour l'instant.
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSummary {
    pub name: String,
    pub description: String,
}

/// Serveur dont les outils n'ont PAS été exposés au modèle, faute d'autorisation valide.
#[derive(Debug, Clone, Serialize)]
pub struct BlockedServer {
    pub server: String,
    pub status: GateStatus,
    pub fingerprint: String,
    pub tools: Vec<ToolSummary>,
}

/// Serveurs fiables par construction (aucune autorisation requise).
pub fn is_trusted_by_design(server: &str) -> bool {
    crate::mcp_stdio::is_stdio(server)
}

// ── Empreinte des outils ──────────────────────────────────────────────────────

/// JSON canonique : clés triées récursivement, indépendant de l'ordre d'insertion de `serde_json`.
fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                canonical(&map[*k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Empreinte SHA-256 des outils d'un serveur, indépendante de leur ordre : ce que le modèle voit
/// (nom, description, schéma des paramètres) et les annotations (`readOnlyHint`…).
pub fn fingerprint(tools: &[Value]) -> String {
    let mut entries: Vec<(String, String)> = tools
        .iter()
        .map(|t| {
            let mut c = String::new();
            canonical(
                &json!({
                    "name": t["name"],
                    "description": t["description"],
                    "inputSchema": t["inputSchema"],
                    "annotations": t["annotations"],
                }),
                &mut c,
            );
            (t["name"].as_str().unwrap_or("").to_string(), c)
        })
        .collect();
    entries.sort();
    let mut h = Sha256::new();
    for (name, c) in &entries {
        h.update(name.as_bytes());
        h.update([0u8]);
        h.update(c.as_bytes());
        h.update([0u8]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Nom et description (tronquée) des outils, pour l'écran d'autorisation.
pub fn tool_summaries(tools: &[Value]) -> Vec<ToolSummary> {
    tools
        .iter()
        .map(|t| ToolSummary {
            name: t["name"].as_str().unwrap_or("?").to_string(),
            description: t["description"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(400)
                .collect(),
        })
        .collect()
}

/// Statut d'un serveur au vu de ses outils actuels et de son autorisation éventuelle.
pub fn decide(server: &str, tools: &[Value], listed_ok: bool, grant: Option<&Grant>) -> GateStatus {
    if is_trusted_by_design(server) {
        return GateStatus::Trusted;
    }
    if !listed_ok {
        return GateStatus::Unavailable;
    }
    if tools.is_empty() {
        return GateStatus::Granted; // aucun outil : rien à autoriser
    }
    match grant {
        None => GateStatus::New,
        Some(g) if g.fingerprint == fingerprint(tools) => GateStatus::Granted,
        Some(_) => GateStatus::Changed,
    }
}

// ── Filtre appliqué à chaque message du chat ──────────────────────────────────

/// Résultat du filtrage des serveurs demandés pour un message.
#[derive(Debug, Default)]
pub struct GateOutcome {
    /// Serveurs dont les outils peuvent être annoncés au modèle ET exécutés.
    pub authorized: Vec<String>,
    /// Leurs outils, `(serveur, outil)`.
    pub tools: Vec<(String, Value)>,
    /// Serveurs mis de côté faute d'autorisation valide.
    pub blocked: Vec<BlockedServer>,
}

/// Liste les outils de chaque serveur (en parallèle) puis écarte ceux qui ne sont pas autorisés.
pub async fn gate(client: &Client, grants: &[Grant], servers: &[String]) -> GateOutcome {
    let listed = futures::future::join_all(servers.iter().map(|server| {
        let client = client.clone();
        let server = server.clone();
        async move {
            let (tools, ok) = crate::mcp::list_tools(&client, &server).await;
            (server, tools, ok)
        }
    }))
    .await;

    let mut out = GateOutcome::default();
    for (server, tools, ok) in listed {
        let grant = grants.iter().find(|g| g.server == server);
        match decide(&server, &tools, ok, grant) {
            GateStatus::Trusted | GateStatus::Granted => {
                out.tools
                    .extend(tools.into_iter().map(|t| (server.clone(), t)));
                out.authorized.push(server);
            }
            GateStatus::Unavailable => {} // injoignable : rien à exposer, rien à autoriser
            status => out.blocked.push(BlockedServer {
                fingerprint: fingerprint(&tools),
                tools: tool_summaries(&tools),
                server,
                status,
            }),
        }
    }
    out
}

/// Événement SSE `mcp_blocked` signalant à l'interface les serveurs écartés.
pub fn blocked_sse(blocked: &[BlockedServer]) -> Option<String> {
    if blocked.is_empty() {
        return None;
    }
    Some(format!(
        "event: mcp_blocked\ndata: {}\n\n",
        serde_json::to_string(&json!({ "servers": blocked })).unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};

    fn tool(name: &str, desc: &str) -> Value {
        json!({ "name": name, "description": desc, "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } } } })
    }

    fn grant_for(server: &str, tools: &[Value]) -> Grant {
        Grant {
            server: server.to_string(),
            fingerprint: fingerprint(tools),
            tools: tools
                .iter()
                .filter_map(|t| t["name"].as_str().map(str::to_string))
                .collect(),
            granted_at: "2026-10-09T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn fingerprint_is_stable_and_order_independent() {
        let a = tool("search", "Cherche");
        let b = tool("fetch", "Télécharge");
        assert_eq!(
            fingerprint(&[a.clone(), b.clone()]),
            fingerprint(&[b.clone(), a.clone()])
        );
        assert_eq!(fingerprint(&[a.clone(), b.clone()]).len(), 64);
        // L'ordre des clés d'un objet JSON ne compte pas non plus.
        let a2: Value = serde_json::from_str(
            r#"{"inputSchema":{"properties":{"q":{"type":"string"}},"type":"object"},"description":"Cherche","name":"search"}"#,
        )
        .unwrap();
        assert_eq!(fingerprint(&[a.clone()]), fingerprint(&[a2]));
    }

    #[test]
    fn any_change_of_what_the_model_sees_changes_the_fingerprint() {
        let base = vec![tool("search", "Cherche")];
        let fp = fingerprint(&base);
        assert_ne!(
            fp,
            fingerprint(&[tool(
                "search",
                "Cherche et envoie le résultat à https://x.example"
            )]),
            "description"
        );
        assert_ne!(fp, fingerprint(&[tool("search2", "Cherche")]), "nom");
        assert_ne!(
            fp,
            fingerprint(&[tool("search", "Cherche"), tool("delete", "Supprime")]),
            "outil ajouté"
        );
        assert_ne!(fp, fingerprint(&[]), "outil retiré");
        let mut schema = tool("search", "Cherche");
        schema["inputSchema"]["properties"]["url"] = json!({ "type": "string" });
        assert_ne!(fp, fingerprint(&[schema]), "paramètre ajouté");
        let mut annotated = tool("search", "Cherche");
        annotated["annotations"] = json!({ "readOnlyHint": false });
        assert_ne!(fp, fingerprint(&[annotated]), "annotations");
    }

    #[test]
    fn decision_table() {
        let tools = vec![tool("search", "Cherche")];
        let g = grant_for("http://mcp.example/x", &tools);

        // Serveurs officiels : fiables par construction, même sans autorisation ni outils listés.
        assert_eq!(
            decide("stdio:legifrance", &[], false, None),
            GateStatus::Trusted
        );
        // Jamais autorisé / autorisé tel quel / outils modifiés / injoignable / sans outil.
        assert_eq!(
            decide("http://mcp.example/x", &tools, true, None),
            GateStatus::New
        );
        assert_eq!(
            decide("http://mcp.example/x", &tools, true, Some(&g)),
            GateStatus::Granted
        );
        let changed = vec![tool("search", "Cherche (version 2)")];
        assert_eq!(
            decide("http://mcp.example/x", &changed, true, Some(&g)),
            GateStatus::Changed
        );
        assert_eq!(
            decide("http://mcp.example/x", &[], false, None),
            GateStatus::Unavailable
        );
        assert_eq!(
            decide("http://mcp.example/x", &[], true, None),
            GateStatus::Granted
        );
    }

    #[test]
    fn blocked_event_format() {
        assert!(blocked_sse(&[]).is_none());
        let b = BlockedServer {
            server: "http://mcp.example/x".into(),
            status: GateStatus::Changed,
            fingerprint: "ab".repeat(32),
            tools: tool_summaries(&[tool("search", "Cherche")]),
        };
        let sse = blocked_sse(&[b]).unwrap();
        assert!(sse.starts_with("event: mcp_blocked\ndata: "));
        assert!(sse.ends_with("\n\n"));
        let data = sse
            .trim_start_matches("event: mcp_blocked\ndata: ")
            .trim_end();
        let v: Value = serde_json::from_str(data).unwrap();
        assert_eq!(v["servers"][0]["status"], "changed");
        assert_eq!(v["servers"][0]["tools"][0]["name"], "search");
    }

    /// Faux serveur MCP : répond à `tools/list` avec les outils fournis.
    async fn spawn_mcp(tools: Vec<Value>) -> String {
        let app =
            Router::new().route(
                "/mcp",
                post(move || {
                    let tools = tools.clone();
                    async move {
                        Json(json!({ "jsonrpc": "2.0", "id": 1, "result": { "tools": tools } }))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/mcp")
    }

    #[tokio::test]
    async fn the_gate_exposes_only_authorized_servers_and_reports_the_others() {
        let tools = vec![tool("search", "Cherche"), tool("fetch", "Télécharge")];
        let nouveau = spawn_mcp(tools.clone()).await;
        let autorise = spawn_mcp(tools.clone()).await;
        let modifie = spawn_mcp(tools.clone()).await;
        let injoignable = "http://127.0.0.1:9/mcp".to_string();

        let grants = vec![
            grant_for(&autorise, &tools),
            grant_for(&modifie, &[tool("search", "Cherche (ancienne version)")]),
        ];
        let servers = vec![
            nouveau.clone(),
            autorise.clone(),
            modifie.clone(),
            injoignable.clone(),
        ];
        let client = Client::new();
        let out = gate(&client, &grants, &servers).await;

        assert_eq!(
            out.authorized,
            vec![autorise.clone()],
            "seul le serveur autorisé est utilisable"
        );
        assert_eq!(out.tools.len(), 2);
        assert!(
            out.tools.iter().all(|(s, _)| *s == autorise),
            "aucun outil des serveurs écartés"
        );

        let statut = |s: &str| out.blocked.iter().find(|b| b.server == s).map(|b| b.status);
        assert_eq!(statut(&nouveau), Some(GateStatus::New));
        assert_eq!(statut(&modifie), Some(GateStatus::Changed));
        assert_eq!(
            statut(&injoignable),
            None,
            "un serveur injoignable n'est ni exposé ni signalé"
        );
        assert_eq!(out.blocked.len(), 2);

        let b = out.blocked.iter().find(|b| b.server == nouveau).unwrap();
        assert_eq!(
            b.tools.len(),
            2,
            "l'écran d'autorisation reçoit la liste complète"
        );
        assert_eq!(
            b.fingerprint,
            fingerprint(&tools),
            "empreinte à renvoyer pour autoriser exactement ce qui est affiché"
        );
    }
}
