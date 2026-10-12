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
//
// Stockage sécurisé des secrets — trousseau du système d'exploitation.
//
// Les identifiants sensibles (bearer LLM, clé API Qdrant, secret client
// PISTE, clé de chiffrement SQLCipher — voir db.rs) sont stockés dans le
// trousseau natif de l'OS (Keychain macOS, Credential Manager Windows,
// Secret Service Linux) via la crate `keyring`, plutôt qu'en clair dans le
// `localStorage` du frontend ou sur disque. Sur Linux, l'accès au Secret
// Service se fait en pur Rust (zbus) : aucune dépendance système
// supplémentaire (pas de libdbus-1-dev, pas de libsecret).
//
// `KeyStore` abstrait l'accès réel pour permettre de tester la logique de ce
// module (et celle de `db.rs`, qui en dépend) sans trousseau disponible —
// notamment en CI, où aucune session Secret Service/Keychain n'existe.
// ============================================================================

use anyhow::{Context, Result};
use rand::RngCore;
use std::collections::HashMap;
use std::sync::Mutex;

/// Nom de service utilisé pour toutes les entrées du trousseau. Reprend
/// l'identifiant Tauri (`tauri.conf.json` → `identifier`) pour éviter toute
/// collision avec une autre application installée sur la même machine.
const SERVICE: &str = "fr.juridix.app";

/// Nom de la clé de chiffrement de la base SQLite dans le trousseau — voir
/// `db.rs::migrate_plaintext_to_encrypted` et `get_or_create_db_key` ci-dessous.
pub const DB_KEY_NAME: &str = "db_encryption_key";

/// Clés que le frontend est autorisé à lire/écrire via les commandes Tauri
/// `store_secret`/`read_secret`/`delete_secret` (voir lib.rs). Volontairement
/// restrictif : `DB_KEY_NAME` n'y figure pas, pour qu'aucun appel venu du
/// frontend ne puisse jamais écraser ou lire la clé de chiffrement de la base.
pub const FRONTEND_ALLOWED_KEYS: &[&str] = &["bearer", "qdrant_api_key", "piste_client_secret"];

pub trait KeyStore: Send + Sync {
    fn set(&self, key: &str, value: &str) -> Result<()>;
    /// `Ok(None)` si l'entrée n'existe pas encore — distinct d'une erreur
    /// d'accès au trousseau (verrouillé, service indisponible…).
    fn get(&self, key: &str) -> Result<Option<String>>;
    /// Ne renvoie pas d'erreur si l'entrée n'existait déjà pas (idempotent).
    fn delete(&self, key: &str) -> Result<()>;
}

// ── Implémentation réelle (trousseau du système) ──────────────────────────────

struct SystemKeyStore;

impl KeyStore for SystemKeyStore {
    fn set(&self, key: &str, value: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, key).context("ouverture du trousseau")?;
        entry
            .set_password(value)
            .context("écriture dans le trousseau")?;
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, key).context("ouverture du trousseau")?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e).context("lecture du trousseau"),
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, key).context("ouverture du trousseau")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e).context("suppression dans le trousseau"),
        }
    }
}

// ── Implémentation de test (en mémoire, jamais utilisée en production) ────────

#[derive(Default)]
pub struct InMemoryKeyStore(Mutex<HashMap<String, String>>);

impl KeyStore for InMemoryKeyStore {
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

// ── API publique ────────────────────────────────────────────────────────────
// Fonctions bloquantes (I/O trousseau synchrone) : à appeler depuis un
// contexte synchrone (setup Tauri) ou via `tokio::task::spawn_blocking`
// depuis un contexte async (commandes Tauri — voir lib.rs).

pub fn store(key: &str, value: &str) -> Result<()> {
    SystemKeyStore.set(key, value)
}

pub fn read(key: &str) -> Result<Option<String>> {
    SystemKeyStore.get(key)
}

pub fn delete(key: &str) -> Result<()> {
    SystemKeyStore.delete(key)
}

/// Récupère la clé de chiffrement SQLCipher, ou en génère une nouvelle (256
/// bits, CSPRNG) et la stocke si c'est le premier démarrage. Retournée sous
/// forme hexadécimale (64 caractères) — le format que `db.rs` transmet
/// ensuite à SQLCipher via `PRAGMA key = "x'…'"`.
///
/// Si le trousseau est indisponible au tout premier démarrage, cette fonction
/// échoue : contrairement au reste de l'application (Winnow, MCP…), il n'y a
/// pas d'« échec ouvert » possible ici — sans clé stable, la base ne serait
/// plus lisible au redémarrage suivant. Mieux vaut un échec net et explicite
/// au démarrage qu'une base chiffrée avec une clé perdue.
pub fn get_or_create_db_key() -> Result<String> {
    get_or_create_db_key_with(&SystemKeyStore)
}

fn get_or_create_db_key_with(store: &dyn KeyStore) -> Result<String> {
    if let Some(existing) = store.get(DB_KEY_NAME)? {
        return Ok(existing);
    }
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let hex_key = raw.iter().map(|b| format!("{b:02x}")).collect::<String>();
    store
        .set(DB_KEY_NAME, &hex_key)
        .context("écriture de la nouvelle clé de chiffrement dans le trousseau")?;
    Ok(hex_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_or_create_is_idempotent_and_well_formed() {
        let store = InMemoryKeyStore::default();
        let k1 = get_or_create_db_key_with(&store).unwrap();
        let k2 = get_or_create_db_key_with(&store).unwrap();
        assert_eq!(k1, k2, "la clé ne doit pas changer entre deux démarrages");
        assert_eq!(k1.len(), 64, "32 octets en hexadécimal = 64 caractères");
        assert!(k1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn two_fresh_keys_differ() {
        let a = get_or_create_db_key_with(&InMemoryKeyStore::default()).unwrap();
        let b = get_or_create_db_key_with(&InMemoryKeyStore::default()).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn set_get_delete_roundtrip() {
        let store = InMemoryKeyStore::default();
        assert_eq!(store.get("x").unwrap(), None);
        store.set("x", "secret").unwrap();
        assert_eq!(store.get("x").unwrap(), Some("secret".to_string()));
        store.delete("x").unwrap();
        assert_eq!(store.get("x").unwrap(), None);
    }

    #[test]
    fn delete_is_idempotent_on_missing_entry() {
        let store = InMemoryKeyStore::default();
        assert!(store.delete("jamais-écrit").is_ok());
    }

    #[test]
    fn db_key_is_never_frontend_allowed() {
        assert!(
            !FRONTEND_ALLOWED_KEYS.contains(&DB_KEY_NAME),
            "la clé de chiffrement de la base ne doit jamais être accessible depuis le frontend"
        );
    }
}
