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
// Juridix — Pré-filtre Winnow — client.rs
// ============================================================================
// Client de l'API native d'Ollaya (https://ollaya.dev/docs/api) :
//   POST /api/decide   — réponses typées + probabilités calibrées
//   GET  /api/version  — sonde de disponibilité
//   GET  /api/tags     — modèles présents sur la machine
//   GET  /api/ps       — modèles chargés en mémoire
// ============================================================================

use once_cell::sync::Lazy;
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

use crate::config;

// Client partagé (pool de connexions) ; les timeouts sont fixés par requête.
static HTTP: Lazy<Client> = Lazy::new(|| Client::builder().build().unwrap_or_default());

#[derive(Debug)]
pub enum WinnowError {
    /// Serveur Ollaya injoignable (non lancé, mauvais port…).
    Unreachable(String),
    /// Le modèle demandé n'est pas téléchargé (`ollaya pull …`).
    ModelMissing(String),
    Unauthorized,
    Timeout,
    /// Erreur renvoyée par Ollaya (code machine + message).
    Server {
        code: String,
        msg: String,
    },
    Malformed(String),
}

impl WinnowError {
    /// Erreurs après lesquelles inutile de réessayer dans la même requête.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            WinnowError::Unreachable(_) | WinnowError::ModelMissing(_) | WinnowError::Unauthorized
        )
    }
}

impl fmt::Display for WinnowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WinnowError::Unreachable(e) => write!(f, "serveur Ollaya injoignable ({e})"),
            WinnowError::ModelMissing(m) => {
                write!(f, "modèle « {m} » absent — lancer `ollaya pull {m}`")
            }
            WinnowError::Unauthorized => write!(f, "clé OLLAYA_API_KEY refusée"),
            WinnowError::Timeout => write!(f, "délai dépassé"),
            WinnowError::Server { code, msg } => write!(f, "Ollaya {code} : {msg}"),
            WinnowError::Malformed(e) => write!(f, "réponse Ollaya inattendue ({e})"),
        }
    }
}

fn with_auth(rb: RequestBuilder) -> RequestBuilder {
    match config::ollaya_api_key() {
        Some(k) => rb.bearer_auth(k),
        None => rb,
    }
}

fn map_send_err(e: reqwest::Error) -> WinnowError {
    if e.is_timeout() {
        WinnowError::Timeout
    } else {
        WinnowError::Unreachable(e.without_url().to_string())
    }
}

/// Convertit une réponse HTTP non-2xx en `WinnowError` (corps d'erreur Ollaya : {error, code}).
async fn error_from_response(resp: reqwest::Response, model: &str) -> WinnowError {
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let code = body["code"].as_str().unwrap_or("").to_string();
    let msg = body["error"].as_str().unwrap_or("").to_string();
    match code.as_str() {
        "MODEL_NOT_FOUND" => WinnowError::ModelMissing(model.to_string()),
        "UNAUTHORIZED" => WinnowError::Unauthorized,
        _ => WinnowError::Server {
            code: if code.is_empty() {
                status.as_u16().to_string()
            } else {
                code
            },
            msg,
        },
    }
}

pub async fn version(timeout: Duration) -> Result<String, WinnowError> {
    let resp = with_auth(HTTP.get(format!("{}/api/version", config::ollaya_url())))
        .timeout(timeout)
        .send()
        .await
        .map_err(map_send_err)?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp, "").await);
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| WinnowError::Malformed(e.to_string()))?;
    Ok(v["version"].as_str().unwrap_or("?").to_string())
}

/// Liste JSON brute de `GET <path>` (`/api/tags` ou `/api/ps`).
pub async fn list(path: &str, timeout: Duration) -> Result<Value, WinnowError> {
    let resp = with_auth(HTTP.get(format!("{}{}", config::ollaya_url(), path)))
        .timeout(timeout)
        .send()
        .await
        .map_err(map_send_err)?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp, "").await);
    }
    resp.json()
        .await
        .map_err(|e| WinnowError::Malformed(e.to_string()))
}

/// Charge le modèle en mémoire sans rien décider (requête sans `state`).
pub async fn preload() -> Result<(), WinnowError> {
    let model = config::winnow_model();
    let resp = with_auth(HTTP.post(format!("{}/api/decide", config::ollaya_url())))
        .timeout(config::winnow_load_timeout())
        .json(&json!({ "model": model, "keep_alive": config::winnow_keep_alive() }))
        .send()
        .await
        .map_err(map_send_err)?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp, &model).await);
    }
    Ok(())
}

/// Résultat utile d'une décision sur un passage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scores {
    /// P(niveau 0 = « sans rapport ») — absente si la question de pertinence n'a pas été posée.
    pub p_irrelevant: Option<f64>,
    /// Rôle le plus probable et sa probabilité.
    pub role: Option<(String, f64)>,
    /// Le passage a été tronqué par Ollaya pour tenir dans le contexte : décision non fiable.
    pub truncated: bool,
}

/// Extrait `Scores` d'une réponse `/api/decide`. Pure : testée sans réseau.
pub fn parse_scores(v: &Value) -> Result<Scores, WinnowError> {
    let answers = v
        .get("answers")
        .and_then(|a| a.as_object())
        .ok_or_else(|| WinnowError::Malformed("champ `answers` absent".into()))?;

    let p_irrelevant = answers
        .get("relevance")
        .and_then(|a| a["probabilities"]["0"].as_f64());

    let role = answers.get("role").and_then(|a| {
        let label = a["choice"].as_str()?;
        let p = a["probabilities"][label].as_f64()?;
        Some((label.to_string(), p))
    });

    if p_irrelevant.is_none() && role.is_none() {
        return Err(WinnowError::Malformed("aucune réponse exploitable".into()));
    }
    Ok(Scores {
        p_irrelevant,
        role,
        truncated: v["state_truncated"].as_bool().unwrap_or(false),
    })
}

/// Pose les `questions` sur `state` (un passage) — une seule requête, un seul passage avant.
pub async fn decide(state: &Value, questions: &Value) -> Result<Scores, WinnowError> {
    let model = config::winnow_model();
    let body = json!({
        "model": model,
        "state": state,
        "questions": questions,
        "keep_alive": config::winnow_keep_alive(),
    });
    let resp = with_auth(HTTP.post(format!("{}/api/decide", config::ollaya_url())))
        .timeout(config::winnow_request_timeout())
        .json(&body)
        .send()
        .await
        .map_err(map_send_err)?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp, &model).await);
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| WinnowError::Malformed(e.to_string()))?;
    parse_scores(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_answers() {
        let v = json!({
          "answers": {
            "relevance": {"type":"score","score":0.2,"probabilities":{"0":0.91,"1":0.05,"2":0.03,"3":0.01}},
            "role": {"type":"choice","choice":"mentions","confidence":0.8,
                     "probabilities":{"regle":0.02,"mentions":0.93,"contexte":0.05}}
          },
          "state_truncated": false
        });
        let s = parse_scores(&v).unwrap();
        assert_eq!(s.p_irrelevant, Some(0.91));
        assert_eq!(s.role, Some(("mentions".to_string(), 0.93)));
        assert!(!s.truncated);
    }

    #[test]
    fn parses_role_only_and_truncation_flag() {
        let v = json!({
          "answers": { "role": {"type":"choice","choice":"regle","probabilities":{"regle":0.7}} },
          "state_truncated": true
        });
        let s = parse_scores(&v).unwrap();
        assert_eq!(s.p_irrelevant, None);
        assert!(s.truncated);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_scores(&json!({"foo": 1})).is_err());
        assert!(parse_scores(&json!({"answers": {}})).is_err());
    }
}
