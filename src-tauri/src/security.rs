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

//! Sécurité de l'API HTTP locale (axum).
//!
//! Le serveur écoute sur 127.0.0.1, mais « localhost » n'est pas une frontière
//! de confiance : n'importe quelle page web ouverte dans le navigateur de
//! l'utilisateur peut émettre des requêtes vers `http://localhost:<port>`
//! (directement, ou via DNS rebinding). Ce module met en place trois barrières :
//!
//! 1. **Host** : seules les requêtes adressées à `localhost` / `127.0.0.1` /
//!    `[::1]` sur le bon port sont acceptées (anti DNS-rebinding).
//! 2. **Origin** : si le navigateur envoie un en-tête `Origin`, il doit
//!    appartenir à la liste des origines de l'application (CORS restreint).
//! 3. **Jeton** : un secret aléatoire, généré à chaque lancement et transmis au
//!    frontend par IPC Tauri (`get_api_token`), est exigé dans l'en-tête
//!    `X-Juridix-Token`. Un site tiers ne peut pas le connaître.
//!
//! Il fournit aussi la validation des URL des serveurs MCP HTTP
//! (`validate_mcp_url`), qui refuse les schémas non HTTP et les adresses
//! link-local / métadonnées cloud (anti-SSRF).

use std::net::{IpAddr, Ipv4Addr};

use axum::{
    extract::{Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

/// Nom de l'en-tête portant le jeton d'API.
pub const TOKEN_HEADER: &str = "x-juridix-token";

/// Origines légitimes du frontend (WebView Tauri selon la plateforme + dev Vite).
const ALLOWED_ORIGINS: &[&str] = &[
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
    "http://localhost:3910",
    "http://127.0.0.1:3910",
];

// ── Jeton ─────────────────────────────────────────────────────────────────────

/// Génère un jeton aléatoire de 256 bits (hex).
///
/// En build de debug uniquement, la variable d'environnement
/// `JURIDIX_API_TOKEN` permet de fixer le jeton (utile pour développer le
/// frontend dans un navigateur, hors Tauri). Elle est ignorée en release.
pub fn generate_token() -> String {
    #[cfg(debug_assertions)]
    if let Ok(t) = std::env::var("JURIDIX_API_TOKEN") {
        if t.trim().len() >= 16 {
            return t.trim().to_string();
        }
    }
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Comparaison en temps constant (évite les attaques par chronométrage).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

// ── Garde (middleware) ────────────────────────────────────────────────────────

/// État du middleware : jeton attendu + port d'écoute.
#[derive(Clone)]
pub struct ApiGuard {
    token: String,
    port: u16,
}

impl ApiGuard {
    pub fn new(token: String, port: u16) -> Self {
        Self { token, port }
    }

    fn host_allowed(&self, host: &str) -> bool {
        let h = host.trim().to_ascii_lowercase();
        h == format!("localhost:{}", self.port)
            || h == format!("127.0.0.1:{}", self.port)
            || h == format!("[::1]:{}", self.port)
    }
}

fn is_allowed_origin(origin: &str) -> bool {
    ALLOWED_ORIGINS
        .iter()
        .any(|o| o.eq_ignore_ascii_case(origin))
}

fn deny(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "detail": msg }))).into_response()
}

/// Middleware : Host + Origin + jeton. Doit être placé *à l'intérieur* de la
/// couche CORS (les pré-vols `OPTIONS` sont traités par `CorsLayer`, qui est
/// plus externe, et n'atteignent donc pas ce middleware).
pub async fn guard(State(g): State<ApiGuard>, req: Request, next: Next) -> Response {
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }

    // 1. Host (anti DNS-rebinding)
    let host_ok = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| g.host_allowed(h))
        .unwrap_or(false);
    if !host_ok {
        return deny(StatusCode::FORBIDDEN, "Host non autorisé");
    }

    // 2. Origin (si présent : requêtes émises par un navigateur)
    if let Some(origin) = req.headers().get(header::ORIGIN) {
        let ok = origin.to_str().map(is_allowed_origin).unwrap_or(false);
        if !ok {
            return deny(StatusCode::FORBIDDEN, "Origine non autorisée");
        }
    }

    // 3. Jeton, exigé dans l'en-tête `X-Juridix-Token` pour TOUTES les routes
    //    (jamais en paramètre d'URL). Seul `GET /health` est exempté : il ne
    //    révèle rien.
    let path = req.uri().path();
    if req.method() == Method::GET && (path == "/health" || path == "/api-proxy/health") {
        return next.run(req).await;
    }

    let token_ok = req
        .headers()
        .get(TOKEN_HEADER)
        .map(|v| ct_eq(v.as_bytes(), g.token.as_bytes()))
        .unwrap_or(false);

    if token_ok {
        next.run(req).await
    } else {
        deny(StatusCode::UNAUTHORIZED, "Jeton d'API manquant ou invalide")
    }
}

/// CORS restreint aux origines de l'application (remplace `allow_origin(Any)`).
pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _parts| {
            origin.to_str().map(is_allowed_origin).unwrap_or(false)
        }))
        .allow_methods(Any)
        .allow_headers(Any)
}

// ── Serveurs MCP HTTP (anti-SSRF) ─────────────────────────────────────────────

/// `true` pour les adresses qu'un serveur MCP n'a aucune raison légitime
/// d'utiliser : link-local (dont les métadonnées cloud 169.254.169.254),
/// non spécifiée, multicast, broadcast. Le loopback et les réseaux privés restent
/// autorisés : les serveurs MCP locaux (ex. `http://localhost:6503/mcp`) en dépendent.
fn is_blocked_for_mcp(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || *v4 == Ipv4Addr::new(100, 100, 100, 200) // métadonnées Alibaba Cloud
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_for_mcp(&IpAddr::V4(v4));
            }
            v6.is_unspecified() || v6.is_multicast() || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Valide l'URL d'un serveur MCP HTTP avant tout appel sortant : schéma
/// http(s) uniquement, pas d'adresse link-local / métadonnées cloud (y compris
/// quand un nom d'hôte y résout). Une panne DNS n'est pas bloquante ici : la
/// requête elle-même échouera.
pub async fn validate_mcp_url(raw: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(raw).map_err(|e| format!("URL invalide : {}", e))?;
    match url.scheme() {
        "http" | "https" => {}
        s => return Err(format!("schéma « {} » non autorisé", s)),
    }
    let host = url.host_str().ok_or("hôte manquant")?.to_ascii_lowercase();
    if matches!(
        host.as_str(),
        "metadata.google.internal" | "metadata" | "instance-data"
    ) {
        return Err("hôte de métadonnées cloud refusé".into());
    }
    let port = url.port_or_known_default().ok_or("port inconnu")?;
    let ips: Vec<IpAddr> = match host
        .trim_matches(|c| c == '[' || c == ']')
        .parse::<IpAddr>()
    {
        Ok(ip) => vec![ip],
        Err(_) => tokio::net::lookup_host((host.as_str(), port))
            .await
            .map(|it| it.map(|a| a.ip()).collect())
            .unwrap_or_default(),
    };
    if ips.iter().any(is_blocked_for_mcp) {
        return Err("adresse link-local / métadonnées refusée".into());
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_basics() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(!ct_eq(b"", b"a"));
    }

    #[test]
    fn token_is_long_and_random() {
        let a = generate_token();
        let b = generate_token();
        assert!(a.len() >= 64);
        assert_ne!(a, b);
    }

    #[test]
    fn host_checks() {
        let g = ApiGuard::new("t".into(), 51873);
        assert!(g.host_allowed("localhost:51873"));
        assert!(g.host_allowed("127.0.0.1:51873"));
        assert!(g.host_allowed("[::1]:51873"));
        assert!(g.host_allowed("LOCALHOST:51873"));
        assert!(!g.host_allowed("evil.example:51873"));
        assert!(!g.host_allowed("localhost:80"));
        assert!(!g.host_allowed("localhost"));
        assert!(!g.host_allowed("localhost:51873.evil.example"));
    }

    #[test]
    fn origin_checks() {
        assert!(is_allowed_origin("tauri://localhost"));
        assert!(is_allowed_origin("http://localhost:3910"));
        assert!(!is_allowed_origin("https://evil.example"));
        assert!(!is_allowed_origin("null"));
        assert!(!is_allowed_origin("http://localhost:51873"));
    }

    #[test]
    fn mcp_ip_filter() {
        for s in [
            "169.254.169.254",
            "169.254.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "fe80::1",
            "::ffff:169.254.169.254",
            "100.100.100.200",
        ] {
            let ip: IpAddr = s.parse().unwrap();
            assert!(is_blocked_for_mcp(&ip), "{} devrait être bloquée", s);
        }
        for s in ["127.0.0.1", "192.168.1.10", "10.0.0.5", "::1", "8.8.8.8"] {
            let ip: IpAddr = s.parse().unwrap();
            assert!(!is_blocked_for_mcp(&ip), "{} devrait être autorisée", s);
        }
    }

    #[tokio::test]
    async fn mcp_url_validation() {
        assert!(validate_mcp_url("http://localhost:6503/mcp").await.is_ok());
        assert!(validate_mcp_url("http://127.0.0.1:3001/sse").await.is_ok());
        assert!(validate_mcp_url("http://169.254.169.254/latest/meta-data")
            .await
            .is_err());
        assert!(validate_mcp_url("http://metadata.google.internal/")
            .await
            .is_err());
        assert!(validate_mcp_url("file:///etc/passwd").await.is_err());
        assert!(validate_mcp_url("ftp://example.org/").await.is_err());
        assert!(validate_mcp_url("pas une url").await.is_err());
    }
}
