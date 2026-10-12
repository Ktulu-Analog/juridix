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

use anyhow::{Context, Result};
use std::path::Path;
use tokio_rusqlite::Connection;
use tracing::info;

use crate::models::{
    CollectionMeta, CollectionVisibility, Conversation, ConversationSave, Message,
};

#[derive(Clone)]
pub struct Database {
    conn: Connection,
}

impl Database {
    /// `db_key_hex` : clé de chiffrement SQLCipher, 32 octets en hexadécimal
    /// (64 caractères) — voir `secrets::get_or_create_db_key`. Une base déjà
    /// chiffrée avec une autre clé, ou une base corrompue, fait échouer cet
    /// appel avec une erreur explicite plutôt que de renvoyer des lectures
    /// silencieusement incorrectes (comportement par défaut de SQLCipher tant
    /// qu'aucune requête n'a forcé le déchiffrement — voir plus bas).
    pub async fn new(path: &Path, db_key_hex: &str) -> Result<Self> {
        migrate_plaintext_to_encrypted(path, db_key_hex)
            .context("migration de la base existante vers un stockage chiffré")?;

        let conn = Connection::open(path).await?;

        let key_literal = format!("x'{db_key_hex}'");
        conn.call(move |db| {
            db.pragma_update(None, "key", &key_literal)?;
            // SQLCipher ne vérifie la clé qu'à la première vraie lecture des
            // pages : sans ce contrôle immédiat, une clé invalide ne se
            // manifesterait qu'au premier `SELECT` applicatif, avec un
            // message d'erreur SQLite générique ("file is not a database")
            // bien plus difficile à diagnostiquer pour l'utilisateur.
            db.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
            Ok(())
        })
        .await
        .context(
            "ouverture de la base chiffrée — clé invalide (trousseau système modifié \
             manuellement ?) ou fichier corrompu",
        )?;

        conn.call(|db| {
            db.execute_batch(
                "CREATE TABLE IF NOT EXISTS conversations (
                    id         TEXT PRIMARY KEY,
                    title      TEXT NOT NULL,
                    space_id   TEXT,
                    messages   TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS collection_meta (
                    collection_name TEXT PRIMARY KEY,
                    owner_id        TEXT NOT NULL,
                    visibility      TEXT NOT NULL DEFAULT 'private',
                    shared_with     TEXT NOT NULL DEFAULT '[]',
                    created_at      TEXT NOT NULL
                );",
            )?;
            Ok(())
        })
        .await?;

        // Migration idempotente : ajout de dossier_id sur les bases déjà créées
        // avant ce champ. SQLite ne supporte pas `ADD COLUMN IF NOT EXISTS`,
        // donc on tente l'ALTER et on ignore l'erreur "duplicate column".
        conn.call(|db| {
            match db.execute("ALTER TABLE conversations ADD COLUMN dossier_id TEXT", []) {
                Ok(_) => {}
                Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
                    if msg.contains("duplicate column") => {}
                Err(e) => return Err(tokio_rusqlite::Error::Rusqlite(e)),
            }
            Ok(())
        })
        .await?;

        // Autorisations des serveurs MCP (idempotent) : voir mcp_grants.rs.
        conn.call(|db| {
            db.execute_batch(crate::mcp_grants::SCHEMA_SQL)?;
            Ok(())
        })
        .await?;

        // Veille juridique : tables `veilles` et `veille_resultats` (idempotent).
        conn.call(|db| {
            db.execute_batch(crate::veille::SCHEMA_SQL)?;
            Ok(())
        })
        .await?;

        info!("SQLite initialised at {}", path.display());
        Ok(Self { conn })
    }

    /// Autorisations enregistrées des serveurs MCP.
    pub async fn list_mcp_grants(&self) -> Result<Vec<crate::mcp_grants::Grant>> {
        let rows = self
            .conn
            .call(|db| {
                let mut stmt =
                    db.prepare("SELECT server, fingerprint, tools, granted_at FROM mcp_grants ORDER BY server")?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        Ok(rows
            .into_iter()
            .map(|(server, fingerprint, tools, granted_at)| crate::mcp_grants::Grant {
                server,
                fingerprint,
                tools: serde_json::from_str(&tools).unwrap_or_default(),
                granted_at,
            })
            .collect())
    }

    /// Enregistre (ou remplace) l'autorisation d'un serveur MCP.
    pub async fn upsert_mcp_grant(&self, g: crate::mcp_grants::Grant) -> Result<()> {
        self.conn
            .call(move |db| {
                db.execute(
                    "INSERT INTO mcp_grants (server, fingerprint, tools, granted_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(server) DO UPDATE SET
                         fingerprint = excluded.fingerprint,
                         tools       = excluded.tools,
                         granted_at  = excluded.granted_at",
                    rusqlite::params![
                        g.server,
                        g.fingerprint,
                        serde_json::to_string(&g.tools).unwrap_or_else(|_| "[]".into()),
                        g.granted_at
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Retire l'autorisation d'un serveur MCP (sans erreur si elle n'existe pas).
    pub async fn delete_mcp_grant(&self, server: String) -> Result<()> {
        self.conn
            .call(move |db| {
                db.execute("DELETE FROM mcp_grants WHERE server = ?1", rusqlite::params![server])?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Contenu intégral de la base (toutes les tables, découvertes via `sqlite_master`
    /// pour couvrir aussi les tables ajoutées plus tard) : export RGPD, voir `rgpd.rs`.
    pub async fn dump_all_tables(&self) -> Result<Vec<(String, Vec<serde_json::Value>)>> {
        let out = self
            .conn
            .call(|db| {
                let mut result = Vec::new();
                for table in list_user_tables(db)? {
                    let rows = dump_table(db, &table)?;
                    result.push((table, rows));
                }
                Ok(result)
            })
            .await?;
        Ok(out)
    }

    /// Vide toutes les tables, écrase les pages libérées (`secure_delete`) et compacte le
    /// fichier : premier temps de l'effacement RGPD (le fichier lui-même est détruit au
    /// redémarrage suivant, voir `rgpd.rs`).
    pub async fn purge_all(&self) -> Result<()> {
        self.conn
            .call(|db| {
                let mut batch = String::from("PRAGMA secure_delete = ON;\nBEGIN;\n");
                for t in list_user_tables(db)? {
                    batch.push_str(&format!("DELETE FROM {};\n", quote_ident(&t)));
                }
                batch.push_str("COMMIT;\n");
                db.execute_batch(&batch)?;
                db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);\nVACUUM;")?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Connexion sous-jacente, pour les modules qui ajoutent leurs propres requêtes
    /// (ex. `veille.rs`) plutôt que d'alourdir ce fichier.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    // ── collection_meta ───────────────────────────────────────────────────────

    pub async fn upsert_collection_meta(&self, meta: CollectionMeta) -> Result<()> {
        let shared = serde_json::to_string(&meta.shared_with).unwrap_or_else(|_| "[]".into());
        let vis = meta.visibility.as_str().to_string();
        self.conn
            .call(move |db| {
                db.execute(
                    "INSERT INTO collection_meta (collection_name, owner_id, visibility, shared_with, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(collection_name) DO UPDATE SET
                         visibility  = excluded.visibility,
                         shared_with = excluded.shared_with",
                    rusqlite::params![meta.collection_name, meta.owner_id, vis, shared, meta.created_at],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn get_collection_meta(&self, name: String) -> Result<Option<CollectionMeta>> {
        let row = self
            .conn
            .call(move |db| {
                let mut stmt = db.prepare(
                    "SELECT collection_name, owner_id, visibility, shared_with, created_at
                     FROM collection_meta WHERE collection_name = ?1",
                )?;
                let mut rows = stmt.query_map(rusqlite::params![name], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?;
                Ok(rows.next().transpose()?)
            })
            .await?;

        Ok(
            row.map(|(n, owner, vis, shared, created_at)| CollectionMeta {
                collection_name: n,
                owner_id: owner,
                visibility: CollectionVisibility::from_str(&vis),
                shared_with: serde_json::from_str(&shared).unwrap_or_default(),
                created_at,
            }),
        )
    }

    pub async fn list_collection_meta(&self) -> Result<Vec<CollectionMeta>> {
        let rows = self
            .conn
            .call(|db| {
                let mut stmt = db.prepare(
                    "SELECT collection_name, owner_id, visibility, shared_with, created_at
                     FROM collection_meta",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;

        Ok(rows
            .into_iter()
            .map(|(n, owner, vis, shared, created_at)| CollectionMeta {
                collection_name: n,
                owner_id: owner,
                visibility: CollectionVisibility::from_str(&vis),
                shared_with: serde_json::from_str(&shared).unwrap_or_default(),
                created_at,
            })
            .collect())
    }

    pub async fn delete_collection_meta(&self, name: String) -> Result<()> {
        self.conn
            .call(move |db| {
                db.execute(
                    "DELETE FROM collection_meta WHERE collection_name = ?1",
                    rusqlite::params![name],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_conversations(&self) -> Result<Vec<Conversation>> {
        let rows = self
            .conn
            .call(|db| {
                let mut stmt = db.prepare(
                    "SELECT id, title, space_id, messages, created_at, updated_at, dossier_id
                     FROM conversations ORDER BY updated_at DESC",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;

        let mut conversations = Vec::new();
        for (id, title, space_id, messages_json, created_at, updated_at, dossier_id) in rows {
            let messages: Vec<Message> = serde_json::from_str(&messages_json).unwrap_or_default();
            conversations.push(Conversation {
                id,
                title,
                space_id,
                messages,
                created_at,
                updated_at,
                dossier_id,
            });
        }
        Ok(conversations)
    }

    pub async fn save_conversation(&self, conv: ConversationSave) -> Result<()> {
        let messages_json =
            serde_json::to_string(&conv.messages).unwrap_or_else(|_| "[]".to_string());
        self.conn
            .call(move |db| {
                db.execute(
                    "INSERT INTO conversations (id, title, space_id, messages, created_at, updated_at, dossier_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET
                         title      = excluded.title,
                         space_id   = excluded.space_id,
                         messages   = excluded.messages,
                         updated_at = excluded.updated_at,
                         dossier_id = excluded.dossier_id",
                    rusqlite::params![
                        conv.id,
                        conv.title,
                        conv.space_id,
                        messages_json,
                        conv.created_at,
                        conv.updated_at,
                        conv.dossier_id,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn delete_conversation(&self, id: String) -> Result<bool> {
        let count = self
            .conn
            .call(move |db| {
                let n = db.execute(
                    "DELETE FROM conversations WHERE id = ?1",
                    rusqlite::params![id],
                )?;
                Ok(n)
            })
            .await?;
        Ok(count > 0)
    }
}

// ============================================================================
// Migration : base SQLite en clair (installations antérieures à ce chantier)
// → base chiffrée SQLCipher, en place, au démarrage.
// ============================================================================

// ── Export / effacement RGPD : utilitaires ───────────────────────────────────

/// Tables applicatives (hors tables internes `sqlite_*`).
fn list_user_tables(db: &rusqlite::Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = db.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// Identifiant SQL entre guillemets (les noms viennent de `sqlite_master`, jamais de l'utilisateur).
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Toutes les lignes d'une table en JSON ; les colonnes texte contenant du JSON
/// (messages, critères…) sont dépliées pour un export lisible.
fn dump_table(db: &rusqlite::Connection, table: &str) -> rusqlite::Result<Vec<serde_json::Value>> {
    use rusqlite::types::ValueRef;
    use serde_json::{json, Value};

    let mut stmt = db.prepare(&format!("SELECT * FROM {}", quote_ident(table)))?;
    let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let rows = stmt.query_map([], |row| {
        let mut obj = serde_json::Map::new();
        for (i, c) in cols.iter().enumerate() {
            let v = match row.get_ref(i)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => json!(n),
                ValueRef::Real(f) => json!(f),
                ValueRef::Text(t) => {
                    let s = String::from_utf8_lossy(t).into_owned();
                    if s.starts_with('[') || s.starts_with('{') {
                        serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s))
                    } else {
                        Value::String(s)
                    }
                }
                ValueRef::Blob(b) => json!(format!("<{} octets>", b.len())),
            };
            obj.insert(c.clone(), v);
        }
        Ok(Value::Object(obj))
    })?;
    rows.collect()
}

/// Convertit en place une base SQLite en clair préexistante en base chiffrée
/// SQLCipher, via la fonction `sqlcipher_export()` fournie par SQLCipher
/// (copie intégrale schéma + données vers une base attachée chiffrée). Ne
/// fait rien si le fichier n'existe pas encore (premier démarrage — la base
/// sera créée chiffrée directement) ou s'il est déjà chiffré.
///
/// Une sauvegarde de l'ancienne base (`<fichier>.pre-encryption.bak`) est
/// systématiquement conservée à côté — jamais supprimée automatiquement, y
/// compris en cas de succès, pour qu'une migration défaillante reste
/// récupérable. Comme elle contient les données EN CLAIR, elle est créée en 0600
/// (Unix), signalée au démarrage, et l'utilisateur peut la supprimer (écrasement
/// puis suppression) dans Réglages > Données et confidentialité ; l'effacement
/// RGPD la détruit aussi — voir `rgpd.rs`.
fn migrate_plaintext_to_encrypted(path: &Path, db_key_hex: &str) -> Result<()> {
    if !is_plaintext_sqlite(path)? {
        return Ok(());
    }
    info!(
        "Base SQLite non chiffrée détectée ({}) — migration vers un stockage chiffré…",
        path.display()
    );

    let backup_path = sibling_path(path, ".pre-encryption.bak");
    std::fs::copy(path, &backup_path).with_context(|| {
        format!(
            "sauvegarde de la base avant migration (vers {})",
            backup_path.display()
        )
    })?;

    // La sauvegarde est EN CLAIR : lisible par son seul propriétaire (Unix). Sa suppression
    // est proposée à l'utilisateur (Réglages > Données et confidentialité) — voir `rgpd.rs`.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&backup_path, std::fs::Permissions::from_mode(0o600));
    }

    let tmp_path = sibling_path(path, ".encrypting.tmp");
    let _ = std::fs::remove_file(&tmp_path); // résidu d'une tentative précédente interrompue

    {
        // Connexion synchrone dédiée à la migration : opération one-shot au
        // démarrage, avant que `tokio_rusqlite::Connection` ne prenne la main
        // sur le fichier — pas besoin (et pas souhaitable) de la faire
        // transiter par le pool async.
        let conn = rusqlite::Connection::open(path).context("ouverture de l'ancienne base en clair")?;
        let tmp_str = tmp_path
            .to_str()
            .context("le chemin de la base contient des caractères non UTF-8")?;
        let key_literal = format!("x'{db_key_hex}'");

        conn.execute(
            "ATTACH DATABASE ?1 AS encrypted KEY ?2",
            rusqlite::params![tmp_str, key_literal],
        )
        .context("ATTACH de la nouvelle base chiffrée")?;
        conn.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()))
            .context("sqlcipher_export — échec du rechiffrement des données")?;
        conn.execute("DETACH DATABASE encrypted", [])
            .context("DETACH après migration")?;
        // `conn` est fermée ici (drop) avant de toucher aux fichiers : sur
        // Windows notamment, un fichier encore ouvert ne peut pas être renommé.
    }

    std::fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "remplacement de {} par la version chiffrée (sauvegarde intacte : {})",
            path.display(),
            backup_path.display()
        )
    })?;

    info!(
        "Migration terminée. Sauvegarde de l'ancienne base (non chiffrée) conservée : {}",
        backup_path.display()
    );
    Ok(())
}

/// Vrai si `path` existe et commence par l'en-tête SQLite en clair
/// (`"SQLite format 3\0"`, 16 octets) — cette signature est absente d'une
/// base SQLCipher, dont les 16 premiers octets sont le sel aléatoire utilisé
/// pour dériver la clé de page.
fn is_plaintext_sqlite(path: &Path) -> Result<bool> {
    use std::io::Read;
    if !path.exists() {
        return Ok(false);
    }
    let mut f = std::fs::File::open(path).context("lecture de l'en-tête de la base existante")?;
    let mut header = [0u8; 16];
    let n = f.read(&mut header)?;
    Ok(n == 16 && &header == b"SQLite format 3\0")
}

/// `path` avec `suffix` ajouté au nom de fichier complet (pas seulement à
/// l'extension) — ex. `conversations.db` + `.pre-encryption.bak` →
/// `conversations.db.pre-encryption.bak`.
fn sibling_path(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    std::path::PathBuf::from(name)
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use rusqlite::Connection;

    fn key_literal(hex: &str) -> String {
        format!("x'{hex}'")
    }

    fn tmp_dir() -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let d = std::env::temp_dir().join(format!(
            "juridix-db-test-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn detects_plaintext_and_encrypted_correctly() {
        let dir = tmp_dir();

        let plain = dir.join("plain.db");
        Connection::open(&plain).unwrap().execute("CREATE TABLE t (id INTEGER)", []).unwrap();
        assert!(is_plaintext_sqlite(&plain).unwrap());

        let enc = dir.join("enc.db");
        {
            let conn = Connection::open(&enc).unwrap();
            conn.pragma_update(None, "key", &key_literal(&"11".repeat(32))).unwrap();
            conn.execute("CREATE TABLE t (id INTEGER)", []).unwrap();
        }
        assert!(!is_plaintext_sqlite(&enc).unwrap());

        assert!(!is_plaintext_sqlite(&dir.join("nexiste-pas.db")).unwrap());
    }

    #[test]
    fn migrates_plaintext_db_preserving_data_and_keeping_backup() {
        let dir = tmp_dir();
        let path = dir.join("conversations.db");

        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE conversations (id INTEGER PRIMARY KEY, title TEXT);
                 INSERT INTO conversations (title) VALUES ('Dossier X');
                 INSERT INTO conversations (title) VALUES ('Dossier Y');",
            )
            .unwrap();
        }

        let key = "ab".repeat(32);
        migrate_plaintext_to_encrypted(&path, &key).unwrap();

        // Le fichier est désormais chiffré...
        assert!(!is_plaintext_sqlite(&path).unwrap());
        // ...la sauvegarde en clair existe et contient bien les données d'origine...
        let backup = sibling_path(&path, ".pre-encryption.bak");
        assert!(is_plaintext_sqlite(&backup).unwrap());
        // ...et les données migrées sont intactes avec la nouvelle clé.
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "key", &key_literal(&key)).unwrap();
        let mut stmt = conn.prepare("SELECT title FROM conversations ORDER BY id").unwrap();
        let titles: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(titles, vec!["Dossier X".to_string(), "Dossier Y".to_string()]);
    }

    #[test]
    fn is_noop_on_already_encrypted_db() {
        let dir = tmp_dir();
        let path = dir.join("enc.db");
        let key = "cd".repeat(32);
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "key", &key_literal(&key)).unwrap();
            conn.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('inchangé');").unwrap();
        }
        let before = std::fs::read(&path).unwrap();

        migrate_plaintext_to_encrypted(&path, &key).unwrap();

        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "aucune ré-écriture sur une base déjà chiffrée");
        assert!(!sibling_path(&path, ".pre-encryption.bak").exists(), "pas de sauvegarde créée inutilement");
    }

    #[test]
    fn is_noop_when_file_does_not_exist_yet() {
        let dir = tmp_dir();
        let path = dir.join("nouvelle.db");
        migrate_plaintext_to_encrypted(&path, &"00".repeat(32)).unwrap();
        assert!(!path.exists(), "la migration ne doit pas créer le fichier elle-même");
    }
}

#[cfg(test)]
mod rgpd_db_tests {
    use super::*;

    const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn tmp_dir() -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let d = std::env::temp_dir().join(format!(
            "juridix-db-rgpd-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn dump_all_tables_then_purge_all() {
        let dir = tmp_dir();
        let db = Database::new(&dir.join("conversations.db"), KEY).await.unwrap();
        db.conn()
            .call(|c| {
                c.execute(
                    "INSERT INTO conversations (id, title, space_id, messages, created_at, updated_at) \
                     VALUES ('c1', 'Titre', NULL, ?1, '2026-10-01', '2026-10-01')",
                    [r#"[{"role":"user","content":"Bonjour"}]"#],
                )?;
                c.execute(
                    "INSERT INTO collection_meta (collection_name, owner_id, created_at) VALUES ('col', 'u1', '2026-10-01')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let dump = db.dump_all_tables().await.unwrap();
        let noms: Vec<&str> = dump.iter().map(|(n, _)| n.as_str()).collect();
        for t in ["conversations", "collection_meta", "veilles", "veille_resultats"] {
            assert!(noms.contains(&t), "{t} absent de {noms:?}");
        }
        let conv = &dump.iter().find(|(n, _)| n == "conversations").unwrap().1;
        assert_eq!(conv.len(), 1);
        assert_eq!(conv[0]["messages"][0]["content"], "Bonjour", "la colonne JSON doit être dépliée");

        db.purge_all().await.unwrap();
        let apres = db.dump_all_tables().await.unwrap();
        assert!(apres.iter().all(|(_, rows)| rows.is_empty()), "toutes les tables doivent être vides");
        assert!(apres.iter().any(|(n, _)| n == "conversations"), "le schéma est conservé");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn mcp_grants_roundtrip_and_replace() {
        use crate::mcp_grants::Grant;
        let dir = tmp_dir();
        let db = Database::new(&dir.join("conversations.db"), KEY).await.unwrap();
        assert!(db.list_mcp_grants().await.unwrap().is_empty());

        let g = Grant {
            server: "http://mcp.example/x".into(),
            fingerprint: "aa".repeat(32),
            tools: vec!["search".into(), "fetch".into()],
            granted_at: "2026-10-09T10:00:00Z".into(),
        };
        db.upsert_mcp_grant(g.clone()).await.unwrap();
        assert_eq!(db.list_mcp_grants().await.unwrap(), vec![g.clone()]);

        // Réautorisation après changement des outils : remplacement, pas doublon.
        let g2 = Grant { fingerprint: "bb".repeat(32), tools: vec!["search".into()], ..g.clone() };
        db.upsert_mcp_grant(g2.clone()).await.unwrap();
        assert_eq!(db.list_mcp_grants().await.unwrap(), vec![g2]);

        // Visible dans l'export RGPD, détruit par le vidage.
        let dump = db.dump_all_tables().await.unwrap();
        assert_eq!(dump.iter().find(|(n, _)| n == "mcp_grants").map(|(_, r)| r.len()), Some(1));
        db.purge_all().await.unwrap();
        assert!(db.list_mcp_grants().await.unwrap().is_empty());

        db.upsert_mcp_grant(g).await.unwrap();
        db.delete_mcp_grant("http://mcp.example/x".into()).await.unwrap();
        db.delete_mcp_grant("absent".into()).await.unwrap(); // sans erreur
        assert!(db.list_mcp_grants().await.unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
