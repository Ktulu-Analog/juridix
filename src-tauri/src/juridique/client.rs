// ============================================================================
// Juridix — Module Juridique — client.rs
// ============================================================================
// PisteClient : OAuth2 client_credentials (PISTE), gestion du token,
// refresh automatique avant expiration.
// Utilisé par legifrance.rs et judilibre.rs.
// ============================================================================

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// ── Constantes PISTE ──────────────────────────────────────────────────────────

const TOKEN_URL: &str = "https://oauth.piste.gouv.fr/api/oauth/token";
/// Base de l'API Légifrance (utilisée par legifrance.rs via `legifrance_base()`).
pub(crate) const LEGIFRANCE_BASE: &str = "https://api.piste.gouv.fr/dila/legifrance/lf-engine-app";
/// Base de l'API Judilibre (utilisée par judilibre.rs via `judilibre_base()`).
pub(crate) const JUDILIBRE_BASE: &str = "https://api.piste.gouv.fr/cassation/judilibre/v1.0";
/// On renouvelle 60 s avant expiration pour avoir une marge confortable.
const REFRESH_MARGIN_SECS: i64 = 60;

// ── Token interne ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Token {
    access_token: String,
    expires_at: DateTime<Utc>,
}

impl Token {
    fn is_valid(&self) -> bool {
        Utc::now() < self.expires_at - Duration::seconds(REFRESH_MARGIN_SECS)
    }
}

// ── Réponse OAuth2 PISTE ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64, // secondes
}

// ── PisteClient ───────────────────────────────────────────────────────────────

/// Client HTTP partagé pour les API PISTE (Légifrance + JUDILIBRE).
/// Thread-safe : clone du `Arc` pour partager entre handlers Axum.
#[derive(Clone)]
pub struct PisteClient {
    inner: Arc<PisteClientInner>,
}

struct PisteClientInner {
    http: Client,
    client_id: String,
    client_secret: String,
    token: RwLock<Option<Token>>,
    /// URL OAuth2 — toujours `TOKEN_URL` en production, surchargée uniquement
    /// par `new_for_test` (mock Légifrance local dans les tests).
    token_url: String,
    /// Base Légifrance — `None` en production (= `LEGIFRANCE_BASE`).
    legifrance_base: Option<String>,
    /// Base Judilibre — `None` en production (= `JUDILIBRE_BASE`).
    judilibre_base: Option<String>,
}

impl PisteClient {
    /// Crée un nouveau PisteClient.
    /// Retourne une erreur si client_id ou client_secret sont vides.
    pub fn new(client_id: String, client_secret: String) -> Result<Self> {
        if client_id.trim().is_empty() || client_secret.trim().is_empty() {
            return Err(anyhow!(
                "PISTE client_id et client_secret sont requis pour l'API juridique"
            ));
        }
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Impossible de créer le client HTTP PISTE")?;

        Ok(Self {
            inner: Arc::new(PisteClientInner {
                http,
                client_id,
                client_secret,
                token: RwLock::new(None),
                token_url: TOKEN_URL.to_string(),
                legifrance_base: None,
                judilibre_base: None,
            }),
        })
    }

    /// Base de l'API Judilibre à utiliser pour ce client.
    pub(crate) fn judilibre_base(&self) -> &str {
        self.inner
            .judilibre_base
            .as_deref()
            .unwrap_or(JUDILIBRE_BASE)
    }

    /// Base de l'API Légifrance à utiliser pour ce client.
    pub(crate) fn legifrance_base(&self) -> &str {
        self.inner
            .legifrance_base
            .as_deref()
            .unwrap_or(LEGIFRANCE_BASE)
    }

    /// Client pointant vers un serveur mock local (tests d'intégration) :
    /// `<base>/oauth/token` pour le token, `<base>/legifrance` et `<base>/judilibre` pour les API.
    #[cfg(test)]
    pub(crate) fn new_for_test(base: &str) -> Self {
        Self {
            inner: Arc::new(PisteClientInner {
                http: Client::new(),
                client_id: "test-id".into(),
                client_secret: "test-secret".into(),
                token: RwLock::new(None),
                token_url: format!("{base}/oauth/token"),
                legifrance_base: Some(format!("{base}/legifrance")),
                judilibre_base: Some(format!("{base}/judilibre")),
            }),
        }
    }

    /// Retourne un token valide, en obtenant ou renouvelant si nécessaire.
    pub async fn bearer_token(&self) -> Result<String> {
        // Fast path : token existant encore valide
        {
            let guard = self.inner.token.read().await;
            if let Some(tok) = guard.as_ref() {
                if tok.is_valid() {
                    debug!("[PISTE] token cache hit, expires_at={}", tok.expires_at);
                    return Ok(tok.access_token.clone());
                }
            }
        }

        // Slow path : acquisition/renouvellement
        let mut guard = self.inner.token.write().await;
        // Double-check après le write lock
        if let Some(tok) = guard.as_ref() {
            if tok.is_valid() {
                return Ok(tok.access_token.clone());
            }
        }

        info!("[PISTE] Renouvellement du token OAuth2");
        let tok = self.fetch_token().await?;
        let access = tok.access_token.clone();
        *guard = Some(tok);
        Ok(access)
    }

    /// Appel HTTP POST vers l'endpoint OAuth2 PISTE.
    async fn fetch_token(&self) -> Result<Token> {
        let resp = self
            .inner
            .http
            .post(&self.inner.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.inner.client_id),
                ("client_secret", &self.inner.client_secret),
                ("scope", "openid"),
            ])
            .send()
            .await
            .context("Erreur réseau lors de l'obtention du token PISTE")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Échec OAuth2 PISTE — HTTP {} : {}", status, body));
        }

        let tr: TokenResponse = resp
            .json()
            .await
            .context("Réponse OAuth2 PISTE illisible")?;

        let expires_at = Utc::now() + Duration::seconds(tr.expires_in as i64);
        info!(
            "[PISTE] Token obtenu, expire dans {}s ({})",
            tr.expires_in, expires_at
        );

        Ok(Token {
            access_token: tr.access_token,
            expires_at,
        })
    }

    /// Expose le reqwest::Client sous-jacent pour les appels API métier.
    pub fn http(&self) -> &Client {
        &self.inner.http
    }

    /// Effectue un GET authentifié et retourne le body JSON.
    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let token = self.bearer_token().await?;
        let resp = self
            .inner
            .http
            .get(url)
            .bearer_auth(&token)
            .header("Accept", "application/json")
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("PISTE GET {url} — HTTP {status}: {body}"));
        }

        resp.json::<T>()
            .await
            .with_context(|| format!("Parsing JSON GET {url}"))
    }

    /// Effectue un POST authentifié avec body JSON et retourne le body JSON.
    pub async fn post_json<B, T>(&self, url: &str, body: &B) -> Result<T>
    where
        B: serde::Serialize,
        T: serde::de::DeserializeOwned,
    {
        let token = self.bearer_token().await?;
        let resp = self
            .inner
            .http
            .post(url)
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            warn!("[PISTE] POST {url} — HTTP {status}: {body_text}");
            return Err(anyhow!("PISTE POST {url} — HTTP {status}: {body_text}"));
        }

        resp.json::<T>()
            .await
            .with_context(|| format!("Parsing JSON POST {url}"))
    }
}
