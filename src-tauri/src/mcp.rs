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

// Client MCP — transport HTTP Streamable (spec 2025-03-26)
//
// Les serveurs locaux lancés comme processus (`stdio:<id>`) sont pris en
// charge par `mcp_stdio.rs` ; les trois points d'entrée publics de ce fichier
// (`list_tools`, `call_tool`, `collect_all_tools`) y aiguillent selon le schéma.
//
// Compatible avec mcp.data.gouv.fr et tout serveur FastMCP en mode stateless_http.
//
// Une seule requête POST par appel. Headers obligatoires :
//   Content-Type: application/json
//   Accept: application/json, text/event-stream   ← les DEUX ensemble, obligatoire
//   Origin: {scheme}://{host}                     ← obligatoire (anti-DNS-rebinding)

use futures::StreamExt;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{info, warn};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extrait l'origine (scheme + host) d'une URL pour le header Origin.
/// "https://mcp.data.gouv.fr/mcp" → "https://mcp.data.gouv.fr"
fn origin_from_url(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let after = &url[scheme_end + 3..];
        let host_end = after.find('/').unwrap_or(after.len());
        return format!("{}://{}", &url[..scheme_end], &after[..host_end]);
    }
    url.to_string()
}

/// Parse un stream SSE et retourne la première réponse JSON-RPC valide.
async fn parse_sse_stream(resp: reqwest::Response) -> Option<Value> {
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                warn!("MCP SSE read error: {}", e);
                break;
            }
        };
        let text = String::from_utf8_lossy(&bytes).to_string();
        buf.push_str(&text);

        // Les blocs SSE sont séparés par "\r\n\r\n" ou "\n\n"
        loop {
            let sep = buf
                .find("\r\n\r\n")
                .map(|i| (i, 4))
                .or_else(|| buf.find("\n\n").map(|i| (i, 2)));
            let (end, sep_len) = match sep {
                Some(s) => s,
                None => break,
            };

            let block = buf[..end].to_string();
            buf = buf[end + sep_len..].to_string();

            let mut data = String::new();
            for line in block.lines() {
                // On ignore "event:", "id:", "retry:" — seul "data:" nous intéresse
                if let Some(r) = line.strip_prefix("data:") {
                    data = r.trim().to_string();
                }
            }

            if data.is_empty() || data == "[DONE]" {
                continue;
            }

            match serde_json::from_str::<Value>(&data) {
                Ok(v) => {
                    if v.get("result").is_some() || v.get("error").is_some() {
                        return Some(v);
                    }
                }
                Err(e) => {
                    warn!("MCP SSE JSON parse error: {}", e);
                }
            }
        }
    }
    warn!(
        "MCP SSE stream ended — no result found (buf remaining: {}b)",
        buf.len()
    );
    None
}

/// Normalise une URL MCP : ajoute https:// si absent.
fn normalize_url(url: &str) -> String {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{}", url)
    }
}

// ── Appel JSON-RPC principal ──────────────────────────────────────────────────

/// Nombre maximal de redirections suivies (chacune est re-validée).
const MAX_MCP_REDIRECTS: u8 = 3;

/// Client HTTP dédié aux serveurs MCP. Il ne suit AUCUNE redirection de
/// lui-même : `rpc_post` les suit à la main en re-validant chaque saut, pour
/// qu'un serveur MCP public ne puisse pas rediriger vers une adresse
/// link-local / de métadonnées cloud (SSRF par redirection).
static MCP_HTTP: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Mozilla/5.0 (compatible; Juridix/1.0)")
        .build()
        .expect("client HTTP MCP")
});

/// Envoie un message JSON-RPC au serveur MCP et retourne la réponse.
/// Gère les réponses JSON directes et SSE.
///
/// Le client passé par l'appelant n'est volontairement pas utilisé : il suit les
/// redirections automatiquement (voir `MCP_HTTP`).
async fn rpc_post(
    _client: &Client,
    server_url: &str,
    body: &Value,
    timeout_secs: u64,
) -> Option<Value> {
    let mut current = normalize_url(server_url);
    let mut hops: u8 = 0;

    let resp = loop {
        // Anti-SSRF : refuse les schémas non HTTP et les adresses link-local /
        // métadonnées cloud (le loopback et le réseau privé restent permis :
        // serveurs MCP locaux). Revalidé à chaque redirection.
        if let Err(e) = crate::security::validate_mcp_url(&current).await {
            warn!("MCP: URL refusée ({}) — {}", e, current);
            return None;
        }
        let origin = origin_from_url(&current);

        let r = match MCP_HTTP
            .post(current.as_str())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Origin", &origin)
            .header("MCP-Protocol-Version", "2025-03-26")
            .header("User-Agent", "Mozilla/5.0 (compatible; Juridix/1.0)")
            .json(body)
            .timeout(Duration::from_secs(timeout_secs))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "MCP: send() failed — is_timeout={} is_connect={} is_builder={} — {}",
                    e.is_timeout(),
                    e.is_connect(),
                    e.is_builder(),
                    e
                );
                if let Some(url) = e.url() {
                    warn!("MCP: failed URL = {}", url);
                }
                return None;
            }
        };

        if r.status().is_redirection() {
            hops += 1;
            if hops > MAX_MCP_REDIRECTS {
                warn!("MCP: trop de redirections — {}", server_url);
                return None;
            }
            let next = r
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|loc| reqwest::Url::parse(&current).ok()?.join(loc).ok());
            let Some(next) = next else {
                warn!("MCP: redirection invalide depuis {}", current);
                return None;
            };
            // Pas de rétrogradation https → http.
            if current.starts_with("https://") && next.scheme() != "https" {
                warn!("MCP: redirection https → http refusée ({})", next);
                return None;
            }
            info!("MCP: redirection {} → {}", current, next);
            current = next.to_string();
            continue;
        }
        break r;
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let deny_reason = resp
            .headers()
            .get("x-deny-reason")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-")
            .to_string();
        let body_text = resp.text().await.unwrap_or_default();
        warn!(
            "MCP HTTP {} (x-deny-reason: {}) — {}",
            status, deny_reason, body_text
        );
        return None;
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if content_type.contains("application/json") {
        // Réponse JSON directe (mode json_response activé)
        resp.json::<Value>().await.ok()
    } else if content_type.contains("text/event-stream") {
        // Réponse SSE (mode par défaut de FastMCP)
        parse_sse_stream(resp).await
    } else {
        warn!("MCP: Content-Type inattendu: {}", content_type);
        None
    }
}

// ── API publique ──────────────────────────────────────────────────────────────

/// Récupère la liste des tools depuis un serveur MCP.
///
/// Le serveur est en mode stateless → pas besoin d'initialize préalable.
/// Un seul POST suffit.
pub async fn list_tools(client: &Client, server_url: &str) -> (Vec<Value>, bool) {
    if crate::mcp_stdio::is_stdio(server_url) {
        let name = crate::mcp_stdio::server_name(server_url);
        return match crate::mcp_stdio::HUB.list_tools(name).await {
            Ok(tools) => {
                info!("MCP: {} tools depuis {}", tools.len(), server_url);
                (tools, true)
            }
            Err(e) => {
                warn!("MCP: serveur STDIO {} indisponible — {}", server_url, e);
                (vec![], false)
            }
        };
    }
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    match rpc_post(client, server_url, &body, 10).await {
        Some(data) => {
            if let Some(tools) = data["result"]["tools"].as_array() {
                info!("MCP: {} tools depuis {}", tools.len(), server_url);
                return (tools.clone(), true);
            }
            if let Some(err) = data.get("error") {
                warn!("MCP tools/list error depuis {}: {}", server_url, err);
            } else {
                warn!("MCP: réponse inattendue de {}: {:?}", server_url, data);
            }
            (vec![], false)
        }
        None => {
            warn!("MCP: serveur injoignable — {}", server_url);
            (vec![], false)
        }
    }
}

/// Convertit un tool MCP au format OpenAI function tool.
pub fn mcp_tool_to_openai(tool: &Value, server_url: &str) -> Value {
    let full = format!(
        "mcp__{}__{}",
        slugify(server_url),
        tool["name"].as_str().unwrap_or("unknown")
    );
    if full.len() > 64 {
        // Pas de filtrage (certains fournisseurs, dont Mistral, acceptent davantage) ;
        // mais l'utilisateur doit savoir pourquoi un fournisseur strict répondrait 400.
        warn!(
            "MCP: nom d'outil de {} caractères (> 64, limite de certains fournisseurs) : {}",
            full.len(),
            full
        );
    }
    json!({
        "type": "function",
        "function": {
            "name": format!("mcp__{}__{}", slugify(server_url), tool["name"].as_str().unwrap_or("unknown")),
            "description": tool["description"].as_str().unwrap_or(""),
            "parameters": tool.get("inputSchema").cloned().unwrap_or(json!({"type":"object","properties":{}}))
        }
    })
}

/// Résultat d'un appel tool MCP.
/// Retourne le texte, et optionnellement une image (b64 + mime).
pub struct McpToolResult {
    pub text: String,
    pub image_b64: Option<String>,
    pub image_mime: Option<String>,
}

/// Exécute un tool MCP et retourne le résultat structuré.
pub async fn call_tool(
    client: &Client,
    server_url: &str,
    tool_name: &str,
    arguments: &Value,
) -> McpToolResult {
    if crate::mcp_stdio::is_stdio(server_url) {
        let name = crate::mcp_stdio::server_name(server_url);
        return match crate::mcp_stdio::HUB
            .call_tool(name, tool_name, arguments)
            .await
        {
            Ok(result) => {
                let mut r = extract_content(&result);
                // `isError` : l'outil a échoué côté métier. On le dit au LLM pour
                // qu'il ne prenne pas un message d'erreur pour un résultat.
                if result["isError"].as_bool().unwrap_or(false) {
                    r.text = format!("Erreur de l'outil {} : {}", tool_name, r.text);
                }
                r
            }
            Err(e) => {
                warn!(
                    "MCP: échec d'appel du tool {} sur {} — {}",
                    tool_name, server_url, e
                );
                McpToolResult {
                    text: format!(
                        "Erreur : impossible d'appeler le tool {} ({})",
                        tool_name, e
                    ),
                    image_b64: None,
                    image_mime: None,
                }
            }
        };
    }
    let body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": tool_name, "arguments": arguments }
    });

    match rpc_post(client, server_url, &body, 30).await {
        Some(data) => {
            if let Some(err) = data.get("error") {
                warn!("MCP tools/call error: {}", err);
                return McpToolResult {
                    text: format!("Erreur MCP : {}", err),
                    image_b64: None,
                    image_mime: None,
                };
            }
            extract_content(&data["result"])
        }
        None => {
            warn!(
                "MCP: échec d'appel du tool {} sur {}",
                tool_name, server_url
            );
            McpToolResult {
                text: format!(
                    "Erreur : impossible d'appeler le tool {} sur {}",
                    tool_name, server_url
                ),
                image_b64: None,
                image_mime: None,
            }
        }
    }
}

/// Collecte tous les tools de tous les serveurs MCP configurés, SANS contrôle d'autorisation.
/// Réservé aux tests : le chat passe par `mcp_grants::gate`, qui n'expose que les serveurs autorisés.
#[cfg(test)]
pub async fn collect_all_tools(client: &Client, servers: &[String]) -> Vec<(String, Value)> {
    let futures: Vec<_> = servers
        .iter()
        .map(|server| {
            let client = client.clone();
            let server = server.clone();
            async move {
                let (tools, _ok) = list_tools(&client, &server).await;
                info!("MCP: {} tools depuis {}", tools.len(), server);
                tools
                    .into_iter()
                    .map(move |t| (server.clone(), t))
                    .collect::<Vec<_>>()
            }
        })
        .collect();
    futures::future::join_all(futures)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// Extrait le contenu d'une réponse MCP (texte + image optionnelle).
fn extract_content(data: &Value) -> McpToolResult {
    if let Some(content) = data["content"].as_array() {
        let mut texts: Vec<&str> = Vec::new();
        let mut image_b64: Option<String> = None;
        let mut image_mime: Option<String> = None;

        for block in content {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(t) = block["text"].as_str() {
                        texts.push(t);
                    }
                }
                Some("image") => {
                    image_b64 = block["data"].as_str().map(|s| s.to_string());
                    image_mime = block["mimeType"].as_str().map(|s| s.to_string());
                }
                _ => {}
            }
        }

        return McpToolResult {
            text: texts.join("\n"),
            image_b64,
            image_mime,
        };
    }
    if let Some(s) = data.as_str() {
        return McpToolResult {
            text: s.to_string(),
            image_b64: None,
            image_mime: None,
        };
    }
    McpToolResult {
        text: data.to_string(),
        image_b64: None,
        image_mime: None,
    }
}

/// Transforme une URL en slug pour les noms de tools.
pub fn server_slug(url: &str) -> String {
    slugify(url)
}

fn slugify(url: &str) -> String {
    // STDIO : le nom du serveur suffit. Contraintes, toutes vérifiées par des tests :
    //  - jamais de « __ » (api.rs découpe `mcp__<slug>__<outil>` sur ce séparateur) ;
    //  - ≤ 15 caractères : `mcp__` + slug + `__` + outil doit rester ≤ 64 (limite de
    //    nombreux fournisseurs compatibles OpenAI, qui rejettent TOUTE la requête sinon) ;
    //    avec le plus long outil du pack Légifrance (42 car.) : 5 + 15 + 2 + 42 = 64 ;
    //  - deux noms distincts donnent deux slugs distincts (suffixe de hachage au-delà de 15).
    if crate::mcp_stdio::is_stdio(url) {
        let full = crate::mcp_stdio::server_name(url);
        // `mcp-legifrance` → `legifrance` : le préfixe est le même pour tous les serveurs du pack.
        let name = full.strip_prefix("mcp-").unwrap_or(full);
        let mut clean = String::new();
        for c in name.chars() {
            let c = if c.is_ascii_alphanumeric() { c } else { '_' };
            if !(c == '_' && clean.ends_with('_')) {
                clean.push(c);
            }
        }
        let clean = clean.trim_matches('_').to_string();
        if clean.len() <= 15 && clean == name {
            return clean;
        }
        // FNV-1a 32 bits sur le nom complet : stable d'une exécution à l'autre.
        let h = name.bytes().fold(0x811c_9dc5_u32, |h, b| {
            (h ^ b as u32).wrapping_mul(0x0100_0193)
        });
        let head: String = clean.chars().take(10).collect();
        let head = head.trim_end_matches('_');
        return format!(
            "{}_{:04x}",
            if head.is_empty() { "s" } else { head },
            h & 0xffff
        );
    }
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
        .chars()
        .take(20)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::{header, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };

    async fn spawn(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn redirect_vers_metadonnees_refusee() {
        let app = Router::new().route(
            "/mcp",
            post(|| async {
                (
                    StatusCode::FOUND,
                    [(header::LOCATION, "http://169.254.169.254/latest/meta-data")],
                )
                    .into_response()
            }),
        );
        let base = spawn(app).await;
        let r = rpc_post(&Client::new(), &format!("{base}/mcp"), &json!({}), 5).await;
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn redirect_legitime_suivie() {
        let app = Router::new()
            .route(
                "/ancien",
                post(|| async {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [(header::LOCATION, "/nouveau")],
                    )
                        .into_response()
                }),
            )
            .route(
                "/nouveau",
                post(|| async { Json(json!({"ok": true})).into_response() }),
            );
        let base = spawn(app).await;
        let r = rpc_post(&Client::new(), &format!("{base}/ancien"), &json!({}), 5).await;
        assert_eq!(r, Some(json!({"ok": true})));
    }

    #[tokio::test]
    async fn boucle_de_redirections_plafonnee() {
        let app = Router::new().route(
            "/boucle",
            post(|| async {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, "/boucle")],
                )
                    .into_response()
            }),
        );
        let base = spawn(app).await;
        let r = rpc_post(&Client::new(), &format!("{base}/boucle"), &json!({}), 5).await;
        assert!(r.is_none());
    }
}
