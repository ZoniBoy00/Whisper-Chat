//! SQLCipher-backed persistence for the desktop chat history.
//!
//! The store holds everything the relay client previously kept in JSON files
//! or only in memory: the decrypted message history, Double Ratchet sessions,
//! the contact list and the app settings. It lives next to the identity file
//! in the app data directory as `whisper-<peer_id>.db` and is keyed
//! deterministically from the identity pickle (see [`derive_db_key`]).
//!
//! SQLCIPHER / ENCRYPTION
//! ----------------------
//! The SQLCipher codec is detected at runtime via `PRAGMA cipher_version`
//! (SQLCipher answers with a version string; plain SQLite returns no rows).
//! When the codec is present, the database is keyed with
//! `PRAGMA key = "x'<hex>'"` — a raw 32-byte key, not a passphrase — so the
//! file is encrypted at rest and can only be opened with the matching
//! identity. When the codec is absent (the `bundled` SQLite fallback; see
//! Cargo.toml), the same code runs against an unencrypted database.
//!
//! CONCURRENCY
//! -----------
//! A single `rusqlite::Connection` is shared behind a `std::sync::Mutex`,
//! mirroring the relay server's `Store`. All operations are short single-row
//! upserts or ordered reads, so a shared connection keeps the design simple.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};

use sha2::{Digest, Sha256};

use crate::relay::{ReactionView, UIMessage};

/// Derive the database key from the identity file contents.
///
/// The key is the lowercase hex encoding of the SHA-256 digest of the identity
/// pickle bytes, which yields a deterministic 256-bit raw key per identity.
/// The database therefore only opens on the machine that holds the matching
/// identity file. This is deliberately a stopgap KDF: a dedicated Argon2 (or
/// similar) derivation is planned as a hardening follow-up, at which point
/// existing databases must be re-keyed.
pub fn derive_db_key(identity_json: &str) -> String {
    let digest = Sha256::digest(identity_json.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Errors surfaced by the [`ChatStore`].
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// A low-level SQLite error (including a wrong SQLCipher key, which
    /// surfaces as "file is not a database").
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    /// A filesystem error while creating directories or removing the database.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A JSON (de)serialization error while persisting a quote snapshot.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// A shared connection mutex was poisoned by a panicking task.
    #[error("store connection was poisoned by a panic")]
    Poisoned,
}

/// A persisted contact row. `username` and `avatar_url` are reserved for the
/// public directory feature and are currently always `None`.
#[derive(Debug, Clone)]
pub struct ContactRow {
    pub peer_id: String,
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub last_seen: Option<i64>,
    /// The peer's public X25519 identity key (base64), when we have learned
    /// it (pre-key bundle, handshake or profile). Used to compute safety
    /// numbers without a live relay round-trip.
    pub curve25519_key: Option<String>,
    /// Whether we have verified this peer's safety number ("verified contact").
    pub verified: bool,
}

/// A persisted inbound group session: `(group_id, sender_peer_id)` maps to the
/// group name plus the Megolm pickle. Multi-sender Megolm keeps one inbound
/// session per group sender, so the key carries both.
pub type StoredGroupInbound = HashMap<(String, String), (String, String)>;

/// Thread-safe handle to the shared SQLite store.
pub struct ChatStore {
    conn: Mutex<Connection>,
}

impl ChatStore {
    /// Lock the shared connection, mapping poison onto a typed error.
    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StoreError> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }

    /// Open (or create) the on-disk database at `path`, initializing the
    /// schema. When the SQLCipher codec is present the database is keyed with
    /// `key_hex` before the schema is touched.
    pub fn open(path: impl AsRef<Path>, key_hex: &str) -> Result<Self, StoreError> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.set_key(key_hex)?;
        store.init_schema()?;
        Ok(store)
    }

    /// Open a purely in-memory database (unit tests only).
    #[cfg(test)]
    pub fn open_in_memory(key_hex: &str) -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.set_key(key_hex)?;
        store.init_schema()?;
        Ok(store)
    }

    /// Whether the linked SQLite library ships the SQLCipher codec.
    ///
    /// SQLCipher answers `PRAGMA cipher_version` with a version string; plain
    /// SQLite has no such pragma and returns no rows. Used so a single code
    /// path handles both encrypted (SQLCipher) and unencrypted (`bundled`)
    /// builds.
    fn has_codec(&self) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        Ok(conn
            .query_row("PRAGMA cipher_version", [], |row| row.get::<_, String>(0))
            .is_ok())
    }

    /// Apply the SQLCipher key, but only when the codec is actually compiled
    /// in. The key is supplied as a raw 32-byte hex literal so SQLCipher uses
    /// it directly instead of running a passphrase KDF.
    fn set_key(&self, key_hex: &str) -> Result<(), StoreError> {
        if self.has_codec()? {
            let conn = self.conn()?;
            conn.execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))?;
        }
        Ok(())
    }

    /// Create the tables and indexes on first open (idempotent).
    fn init_schema(&self) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id        TEXT PRIMARY KEY,
                peer_id   TEXT NOT NULL,
                text      TEXT NOT NULL,
                outgoing  INTEGER NOT NULL,
                timestamp INTEGER NOT NULL,
                status    TEXT NOT NULL,
                client_id TEXT,
                quote_json TEXT,
                system_json TEXT,
                expires_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_messages_peer_timestamp
                ON messages (peer_id, timestamp);
            CREATE TABLE IF NOT EXISTS reactions (
                peer_id    TEXT NOT NULL,
                message_id TEXT NOT NULL,
                sender     TEXT NOT NULL,
                emoji      TEXT NOT NULL,
                PRIMARY KEY (peer_id, message_id, sender)
            );
            CREATE TABLE IF NOT EXISTS sessions (
                peer_id      TEXT PRIMARY KEY,
                session_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS contacts (
                peer_id      TEXT PRIMARY KEY,
                display_name TEXT,
                username     TEXT,
                avatar_url   TEXT,
                last_seen    INTEGER,
                curve25519_key TEXT,
                verified     INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS group_outbound (
                group_id TEXT PRIMARY KEY,
                name     TEXT NOT NULL,
                pickle   TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS group_inbound (
                group_id TEXT NOT NULL,
                sender   TEXT NOT NULL,
                name     TEXT NOT NULL,
                pickle   TEXT NOT NULL,
                PRIMARY KEY (group_id, sender)
            );
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS chat_settings (
                peer_id          TEXT PRIMARY KEY,
                expire_seconds   INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS group_meta (
                group_id    TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                avatar_hash TEXT
            );",
        )?;

        // Migration: `group_inbound` gained a `sender` column for multi-sender
        // Megolm (one inbound session per group sender). SQLite has no
        // `ADD COLUMN ... PRIMARY KEY`, so detect the old single-sender shape
        // and rebuild the table, carrying legacy rows over with an empty
        // sender (the recipient then falls back to that session during
        // decrypt until the senders re-share their keys).
        let inbound_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(group_inbound)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        if !inbound_columns.iter().any(|c| c.as_str() == "sender") {
            conn.execute_batch(
                "ALTER TABLE group_inbound RENAME TO group_inbound_legacy;
                 CREATE TABLE group_inbound (
                     group_id TEXT NOT NULL,
                     sender   TEXT NOT NULL,
                     name     TEXT NOT NULL,
                     pickle   TEXT NOT NULL,
                     PRIMARY KEY (group_id, sender)
                 );
                 INSERT INTO group_inbound (group_id, sender, name, pickle)
                     SELECT group_id, '', name, pickle FROM group_inbound_legacy;
                 DROP TABLE group_inbound_legacy;",
            )?;
        }

        // Migration: `messages` gained a nullable `quote_json` column for
        // quoted replies. Older rows simply have no quote (NULL).
        let message_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(messages)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        if !message_columns.iter().any(|c| c.as_str() == "quote_json") {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN quote_json TEXT;")?;
        }
        if !message_columns.iter().any(|c| c.as_str() == "system_json") {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN system_json TEXT;")?;
        }
        // Migration: `messages` gained a nullable `expires_at` column for
        // disappearing messages. Older rows simply never expire (NULL). The
        // index is created here — after the column is guaranteed to exist —
        // because an existing database from before this feature lacks the
        // column, and CREATE INDEX would fail against it.
        if !message_columns.iter().any(|c| c.as_str() == "expires_at") {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN expires_at INTEGER;")?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_expires_at
             ON messages (expires_at);",
        )?;

        // Migration: `contacts` gained `curve25519_key` (for safety numbers)
        // and `verified` (contact verification state).
        let contact_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(contacts)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        if !contact_columns
            .iter()
            .any(|c| c.as_str() == "curve25519_key")
        {
            conn.execute_batch("ALTER TABLE contacts ADD COLUMN curve25519_key TEXT;")?;
        }
        if !contact_columns.iter().any(|c| c.as_str() == "verified") {
            conn.execute_batch(
                "ALTER TABLE contacts ADD COLUMN verified INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        Ok(())
    }

    /// Replace the whole outbound group-session map (group_id -> (name,
    /// Megolm pickle)). A full rewrite keeps it simple: groups are few and the
    /// session state must stay in lockstep with the in-memory map anyway.
    pub fn replace_group_outbound(
        &self,
        groups: &HashMap<String, (String, String)>,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM group_outbound", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO group_outbound (group_id, name, pickle) VALUES (?1, ?2, ?3)",
            )?;
            for (group_id, (name, pickle)) in groups {
                stmt.execute(params![group_id, name, pickle])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Load every persisted outbound group session as a
    /// `group_id -> (name, pickle)` map.
    pub fn load_group_outbound(&self) -> Result<HashMap<String, (String, String)>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT group_id, name, pickle FROM group_outbound")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, (row.get(1)?, row.get(2)?)))
        })?;
        let mut groups = HashMap::new();
        for row in rows {
            let (id, rest) = row?;
            groups.insert(id, rest);
        }
        Ok(groups)
    }

    /// Replace the whole inbound group-session map (`(group_id, sender) ->
    /// (name, Megolm pickle)`). A full rewrite keeps it simple, mirroring
    /// [`ChatStore::replace_sessions`]. Multi-sender Megolm keeps one inbound
    /// session per group sender, so the key carries both the group id and the
    /// sender's peer id.
    pub fn replace_group_inbound(&self, groups: &StoredGroupInbound) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM group_inbound", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO group_inbound (group_id, sender, name, pickle) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for ((group_id, sender), (name, pickle)) in groups {
                stmt.execute(params![group_id, sender, name, pickle])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Load every persisted inbound group session as a
    /// `(group_id, sender) -> (name, pickle)` map.
    pub fn load_group_inbound(&self) -> Result<StoredGroupInbound, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT group_id, sender, name, pickle FROM group_inbound")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                (row.get(2)?, row.get(3)?),
            ))
        })?;
        let mut groups = HashMap::new();
        for row in rows {
            let (key, rest) = row?;
            groups.insert(key, rest);
        }
        Ok(groups)
    }

    /// Persist a group's public metadata (name, avatar path) so it survives a
    /// restart. The Megolm pickles carry the session; this row carries the
    /// display data the chat list needs immediately after startup.
    pub fn set_group_meta(
        &self,
        group_id: &str,
        name: &str,
        avatar_url: Option<&str>,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO group_meta (group_id, name, avatar_hash)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(group_id) DO UPDATE SET
                 name = excluded.name,
                 avatar_hash = excluded.avatar_hash",
            params![group_id, name, avatar_url],
        )?;
        Ok(())
    }

    /// Every persisted group's display metadata: group_id -> (name, avatar).
    pub fn load_group_meta(&self) -> Result<HashMap<String, (String, Option<String>)>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT group_id, name, avatar_hash FROM group_meta")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?),
            ))
        })?;
        let mut meta = HashMap::new();
        for row in rows {
            let (group_id, rest) = row?;
            meta.insert(group_id, rest);
        }
        Ok(meta)
    }

    /// Insert or update one message row. The `id` is the primary key, so
    /// re-recording an optimistic message (same client id) replaces the row
    /// instead of duplicating it.
    pub fn upsert_message(
        &self,
        peer_id: &str,
        message: &UIMessage,
        client_id: Option<&str>,
    ) -> Result<(), StoreError> {
        let quote_json = match &message.quote {
            Some(quote) => Some(serde_json::to_string(quote)?),
            None => None,
        };
        let system_json = match &message.system {
            Some(system) => Some(serde_json::to_string(system)?),
            None => None,
        };
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO messages
                 (id, peer_id, text, outgoing, timestamp, status, client_id, quote_json, system_json, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                 peer_id   = excluded.peer_id,
                 text      = excluded.text,
                 outgoing  = excluded.outgoing,
                 timestamp = excluded.timestamp,
                 status    = excluded.status,
                 client_id = excluded.client_id,
                 quote_json = excluded.quote_json,
                 system_json = excluded.system_json,
                 expires_at = excluded.expires_at",
            params![
                message.id,
                peer_id,
                message.text,
                message.outgoing as i64,
                message.timestamp as i64,
                message.status,
                client_id,
                quote_json,
                system_json,
                message.expires_at.map(|millis| millis as i64)
            ],
        )?;
        Ok(())
    }

    /// Delete every message whose disappearing deadline has passed, along with
    /// their reactions. Returns the (peer_id, message_id) pairs removed so the
    /// caller can drop them from the in-memory state and notify the UI.
    pub fn delete_expired_messages(&self, now: u64) -> Result<Vec<(String, String)>, StoreError> {
        let mut conn = self.conn()?;
        let expired: Vec<(String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, peer_id FROM messages
                 WHERE expires_at IS NOT NULL AND expires_at <= ?1",
            )?;
            let rows = stmt.query_map(params![now as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let collected: Vec<(String, String)> =
                rows.collect::<rusqlite::Result<Vec<(String, String)>>>()?;
            collected
        };
        if expired.is_empty() {
            return Ok(Vec::new());
        }
        let tx = conn.transaction()?;
        for (message_id, _) in &expired {
            tx.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
            tx.execute(
                "DELETE FROM reactions WHERE message_id = ?1",
                params![message_id],
            )?;
        }
        tx.commit()?;
        Ok(expired)
    }

    /// Every per-chat disappearing-message timer currently configured.
    pub fn all_chat_expirations(&self) -> Result<HashMap<String, u64>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT peer_id, expire_seconds FROM chat_settings")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (peer_id, seconds) = row?;
            map.insert(peer_id, seconds);
        }
        Ok(map)
    }

    /// Set (or clear, with `0`) the disappearing-message timer for a peer or
    /// group. The setting is local — it only affects messages THIS identity
    /// sends; recipients apply the per-message deadline they receive.
    pub fn set_chat_expire_seconds(&self, peer_id: &str, seconds: u64) -> Result<(), StoreError> {
        let conn = self.conn()?;
        if seconds == 0 {
            conn.execute(
                "DELETE FROM chat_settings WHERE peer_id = ?1",
                params![peer_id],
            )?;
        } else {
            conn.execute(
                "INSERT INTO chat_settings (peer_id, expire_seconds) VALUES (?1, ?2)
                 ON CONFLICT(peer_id) DO UPDATE SET expire_seconds = excluded.expire_seconds",
                params![peer_id, seconds as i64],
            )?;
        }
        Ok(())
    }

    /// Remove a message row (used to roll back an optimistic record when a
    /// send fails before the envelope leaves the client). Reactions attached
    /// to the message are removed with it.
    pub fn delete_message(&self, id: &str) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        tx.execute("DELETE FROM reactions WHERE message_id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    /// Persist the absolute state of one `(peer_id, message_id, sender)`
    /// reaction. The in-memory message map is the source of truth; this only
    /// mirrors it on disk so the state survives restarts. `active = true`
    /// upserts the emoji, `false` removes the row.
    pub fn set_reaction_state(
        &self,
        peer_id: &str,
        message_id: &str,
        sender: &str,
        emoji: &str,
        active: bool,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        if active {
            conn.execute(
                "INSERT INTO reactions (peer_id, message_id, sender, emoji)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(peer_id, message_id, sender)
                 DO UPDATE SET emoji = excluded.emoji",
                params![peer_id, message_id, sender, emoji],
            )?;
        } else {
            conn.execute(
                "DELETE FROM reactions
                 WHERE peer_id = ?1 AND message_id = ?2 AND sender = ?3",
                params![peer_id, message_id, sender],
            )?;
        }
        Ok(())
    }

    /// Every stored reaction for one conversation, keyed by message id and
    /// ordered by insertion (rowid) so the UI renders pills deterministically.
    pub fn reactions_for(
        &self,
        peer_id: &str,
    ) -> Result<HashMap<String, Vec<ReactionView>>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT message_id, sender, emoji FROM reactions
             WHERE peer_id = ?1 ORDER BY rowid",
        )?;
        let rows = stmt.query_map(params![peer_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut by_message: HashMap<String, Vec<ReactionView>> = HashMap::new();
        for row in rows {
            let (message_id, sender, emoji) = row?;
            by_message
                .entry(message_id)
                .or_default()
                .push(ReactionView { sender, emoji });
        }
        Ok(by_message)
    }

    /// The previously stored messages for `peer_id`, oldest first. Quoted-reply
    /// snapshots and emoji reactions are loaded back with their messages.
    ///
    /// The reaction map is fetched *before* the connection lock is taken:
    /// `reactions_for` takes the same shared lock, so calling it while this
    /// method still holds the connection would deadlock.
    pub fn messages_for(&self, peer_id: &str) -> Result<Vec<UIMessage>, StoreError> {
        let reactions = self.reactions_for(peer_id)?;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, text, outgoing, timestamp, status, quote_json, system_json, expires_at
             FROM messages WHERE peer_id = ?1
             ORDER BY timestamp, id",
        )?;
        let rows = stmt.query_map(params![peer_id], |row| {
            let quote_json: Option<String> = row.get(5)?;
            let quote = match quote_json {
                Some(json) => serde_json::from_str(&json).unwrap_or(None),
                None => None,
            };
            let system_json: Option<String> = row.get(6)?;
            let system = match system_json {
                Some(json) => serde_json::from_str(&json).unwrap_or(None),
                None => None,
            };
            Ok(UIMessage {
                id: row.get(0)?,
                text: row.get(1)?,
                outgoing: row.get(2)?,
                timestamp: row.get::<_, i64>(3)? as u64,
                status: row.get(4)?,
                quote,
                reactions: Vec::new(), // hydrated below
                system,
                read_by: Vec::new(),
                expires_at: row.get::<_, Option<i64>>(7)?.map(|millis| millis as u64),
            })
        })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        // Attach every stored reaction to its message in thread order.
        for message in &mut messages {
            if let Some(reacts) = reactions.get(&message.id) {
                message.reactions = reacts.clone();
            }
        }
        Ok(messages)
    }

    /// Every stored message grouped by peer, oldest first per peer. Used to
    /// hydrate the in-memory history on startup.
    pub fn all_messages(&self) -> Result<HashMap<String, Vec<UIMessage>>, StoreError> {
        let peer_ids: Vec<String> = {
            let conn = self.conn()?;
            let mut stmt = conn.prepare("SELECT DISTINCT peer_id FROM messages")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut all = HashMap::new();
        for peer_id in peer_ids {
            all.insert(peer_id.clone(), self.messages_for(&peer_id)?);
        }
        Ok(all)
    }

    /// Replace the whole session map, dropping rows for sessions that no
    /// longer exist (sessions are few, so a full rewrite keeps it simple).
    pub fn replace_sessions(&self, sessions: &HashMap<String, String>) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM sessions", [])?;
        {
            let mut stmt =
                tx.prepare("INSERT INTO sessions (peer_id, session_json) VALUES (?1, ?2)")?;
            for (peer, json) in sessions {
                stmt.execute(params![peer, json])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Load every persisted session as a peer_id -> JSON map.
    pub fn load_sessions(&self) -> Result<HashMap<String, String>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT peer_id, session_json FROM sessions")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut sessions = HashMap::new();
        for row in rows {
            let (peer, json) = row?;
            sessions.insert(peer, json);
        }
        Ok(sessions)
    }

    /// Insert or update a contact. `COALESCE` on update keeps existing
    /// display name, username and avatar when a partial update only carries a
    /// new `last_seen` (presence) or only a new name. The `curve25519_key`
    /// and `verified` flags follow the same keep-existing-when-null rule.
    pub fn upsert_contact(&self, contact: &ContactRow) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO contacts
                 (peer_id, display_name, username, avatar_url, last_seen, curve25519_key, verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(peer_id) DO UPDATE SET
                 display_name   = COALESCE(excluded.display_name, contacts.display_name),
                 username       = COALESCE(excluded.username, contacts.username),
                 avatar_url     = COALESCE(excluded.avatar_url, contacts.avatar_url),
                 last_seen      = COALESCE(excluded.last_seen, contacts.last_seen),
                 curve25519_key = COALESCE(excluded.curve25519_key, contacts.curve25519_key),
                 verified       = excluded.verified",
            params![
                contact.peer_id,
                contact.display_name,
                contact.username,
                contact.avatar_url,
                contact.last_seen,
                contact.curve25519_key,
                contact.verified as i64
            ],
        )?;
        Ok(())
    }

    /// Every known contact row.
    pub fn contacts(&self) -> Result<Vec<ContactRow>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT peer_id, display_name, username, avatar_url, last_seen,
                    curve25519_key, verified
             FROM contacts ORDER BY peer_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ContactRow {
                peer_id: row.get(0)?,
                display_name: row.get(1)?,
                username: row.get(2)?,
                avatar_url: row.get(3)?,
                last_seen: row.get(4)?,
                curve25519_key: row.get(5)?,
                verified: row.get::<_, i64>(6)? != 0,
            })
        })?;
        let mut contacts = Vec::new();
        for row in rows {
            contacts.push(row?);
        }
        Ok(contacts)
    }

    /// A single contact row, if it exists.
    pub fn get_contact(&self, peer_id: &str) -> Result<Option<ContactRow>, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT peer_id, display_name, username, avatar_url, last_seen,
                    curve25519_key, verified
             FROM contacts WHERE peer_id = ?1",
            params![peer_id],
            |row| {
                Ok(ContactRow {
                    peer_id: row.get(0)?,
                    display_name: row.get(1)?,
                    username: row.get(2)?,
                    avatar_url: row.get(3)?,
                    last_seen: row.get(4)?,
                    curve25519_key: row.get(5)?,
                    verified: row.get::<_, i64>(6)? != 0,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// Remove a contact row (and any learned display name / presence) from the
    /// store. Used by the client-local "remove contact" action.
    pub fn delete_contact(&self, peer_id: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM contacts WHERE peer_id = ?1", params![peer_id])?;
        Ok(())
    }

    /// Remove every stored message row for a peer. Used by the client-local
    /// "remove contact" action, which also drops the session so history and
    /// keys for that peer are cleared on this device.
    pub fn delete_messages_for(&self, peer_id: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM messages WHERE peer_id = ?1", params![peer_id])?;
        Ok(())
    }

    /// Remove every message row across all peers. Used by the "clear chat
    /// history" action, which wipes all message history but keeps contacts,
    /// sessions and settings intact.
    pub fn clear_messages(&self) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM messages", [])?;
        Ok(())
    }

    /// Persist one settings value.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Read one settings value, if it is set.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// Remove one settings value (clearing a preference).
    pub fn delete_setting(&self, key: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stable, valid 64-char hex key for tests (32 bytes of 0xaa).
    const TEST_KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    /// Deterministic peer id + message helper.
    fn sample_message(id: &str, text: &str, outgoing: bool, status: &str) -> UIMessage {
        UIMessage {
            id: id.to_string(),
            text: text.to_string(),
            outgoing,
            timestamp: 0,
            status: status.to_string(),
            quote: None,
            reactions: Vec::new(),
            system: None,
            read_by: Vec::new(),
            expires_at: None,
        }
    }

    #[test]
    fn delete_expired_messages_removes_only_overdue_ones() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        let now = 1_000_000;
        let mut overdue = sample_message("m-1", "gone", true, "delivered");
        overdue.expires_at = Some(now - 1); // deadline passed
        let mut on_time = sample_message("m-2", "stays", true, "delivered");
        on_time.expires_at = Some(now + 60_000); // still alive
        let persistent = sample_message("m-3", "forever", true, "delivered"); // no expiry
        for message in [&overdue, &on_time, &persistent] {
            store
                .upsert_message("peer-1", message, None)
                .expect("persist");
        }

        let removed = store.delete_expired_messages(now).expect("purge expired");
        assert_eq!(removed, vec![("m-1".to_string(), "peer-1".to_string())]);

        let remaining = store.messages_for("peer-1").expect("load messages");
        let ids: Vec<&str> = remaining.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["m-2", "m-3"]);
    }

    #[test]
    fn chat_expiration_settings_roundtrip_and_clear() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        assert_eq!(store.all_chat_expirations().expect("empty").len(), 0);

        store.set_chat_expire_seconds("peer-1", 30).expect("set");
        store.set_chat_expire_seconds("peer-2", 3600).expect("set");
        let map = store.all_chat_expirations().expect("read back");
        assert_eq!(map.get("peer-1"), Some(&30));
        assert_eq!(map.get("peer-2"), Some(&3600));

        // Clearing (0) removes the row entirely.
        store.set_chat_expire_seconds("peer-1", 0).expect("clear");
        let map = store.all_chat_expirations().expect("read back");
        assert!(!map.contains_key("peer-1"));
        assert_eq!(map.get("peer-2"), Some(&3600));
    }

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("whisper-desktop-{name}-{}.db", std::process::id()))
    }

    /// Whether the built SQLite library has the SQLCipher codec compiled in.
    /// The key-related assertions below only apply on such builds; on the
    /// plain `bundled` fallback the codec is absent and they are skipped.
    fn codec_present() -> bool {
        ChatStore::open_in_memory(TEST_KEY)
            .and_then(|store| store.has_codec())
            .unwrap_or(false)
    }

    #[test]
    fn derive_db_key_is_deterministic_and_sensitive_to_the_input() {
        let key_a = derive_db_key(r#"{"account":"pickle-a"}"#);
        let key_b = derive_db_key(r#"{"account":"pickle-a"}"#);
        let key_c = derive_db_key(r#"{"account":"pickle-b"}"#);
        assert_eq!(key_a, key_b, "the same identity must yield the same key");
        assert_eq!(key_a.len(), 64, "a 32-byte SHA-256 digest is 64 hex chars");
        assert_ne!(
            key_a, key_c,
            "a different identity must yield a different key"
        );
    }

    #[test]
    fn messages_roundtrip_through_the_store_oldest_first() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        let first = sample_message("m-1", "hello", false, "delivered");
        let second = sample_message("m-2", "hello again", false, "delivered");
        store
            .upsert_message("peer-1", &first, None)
            .expect("store first message");
        store
            .upsert_message("peer-1", &second, None)
            .expect("store second message");

        let loaded = store.messages_for("peer-1").expect("load messages");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "m-1", "messages must come back oldest first");
        assert_eq!(loaded[1].text, "hello again");
    }

    #[test]
    fn messages_are_isolated_per_peer() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_message(
                "peer-1",
                &sample_message("m-1", "for peer 1", false, "delivered"),
                None,
            )
            .expect("store for peer 1");
        store
            .upsert_message(
                "peer-2",
                &sample_message("m-2", "for peer 2", false, "delivered"),
                None,
            )
            .expect("store for peer 2");

        assert_eq!(
            store.messages_for("peer-1").expect("peer 1 messages").len(),
            1
        );
        assert!(store.messages_for("peer-1").unwrap()[0]
            .text
            .contains("peer 1"));
        let all = store.all_messages().expect("all messages");
        assert_eq!(all.len(), 2, "both peers must be grouped");
        assert_eq!(all["peer-2"][0].text, "for peer 2");
    }

    #[test]
    fn reupserting_the_same_message_id_replaces_the_row() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_message(
                "peer-1",
                &sample_message("client-1", "optimistic", true, "sent"),
                Some("client-1"),
            )
            .expect("first upsert");
        // A status flip upserts the same id again with a new status.
        store
            .upsert_message(
                "peer-1",
                &sample_message("client-1", "optimistic", true, "delivered"),
                Some("client-1"),
            )
            .expect("second upsert");

        let loaded = store.messages_for("peer-1").expect("load messages");
        assert_eq!(
            loaded.len(),
            1,
            "an id collision must update, not duplicate"
        );
        assert_eq!(loaded[0].status, "delivered");
    }

    #[test]
    fn delete_message_removes_the_row() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_message(
                "peer-1",
                &sample_message("m-1", "hello", false, "delivered"),
                None,
            )
            .expect("store message");
        store.delete_message("m-1").expect("delete message");
        assert!(store.messages_for("peer-1").expect("load").is_empty());
    }

    #[test]
    fn reactions_roundtrip_and_state_toggle() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .set_reaction_state("peer-1", "m-1", "alice", "👍", true)
            .expect("add reaction");
        store
            .set_reaction_state("peer-1", "m-1", "bob", "🔥", true)
            .expect("add second reaction");

        let reactions = store.reactions_for("peer-1").expect("load reactions");
        assert_eq!(reactions["m-1"].len(), 2);
        assert_eq!(reactions["m-1"][0].emoji, "👍");
        assert_eq!(reactions["m-1"][1].emoji, "🔥");

        // Turning the state off removes the row.
        store
            .set_reaction_state("peer-1", "m-1", "alice", "👍", false)
            .expect("remove reaction");
        let reactions = store.reactions_for("peer-1").expect("load reactions");
        assert_eq!(reactions["m-1"].len(), 1);
        assert_eq!(reactions["m-1"][0].sender, "bob");
    }

    #[test]
    fn messages_for_loads_quotes_and_reactions() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        let mut message = sample_message("m-1", "my reply", false, "delivered");
        message.quote = Some(e2ee_core::Quote::new(
            "m-0",
            "original text",
            "bob",
            Some("Bob".to_string()),
        ));
        store
            .upsert_message("peer-1", &message, None)
            .expect("store quoted message");
        store
            .set_reaction_state("peer-1", "m-1", "bob", "👍", true)
            .expect("store reaction");

        let loaded = store.messages_for("peer-1").expect("load messages");
        assert_eq!(loaded.len(), 1);
        let quote = loaded[0].quote.as_ref().expect("quote must load");
        assert_eq!(quote.message_id, "m-0");
        assert_eq!(quote.text, "original text");
        assert_eq!(quote.sender_name.as_deref(), Some("Bob"));
        assert_eq!(loaded[0].reactions.len(), 1);
        assert_eq!(loaded[0].reactions[0].emoji, "👍");
    }

    #[test]
    fn delete_message_removes_attached_reactions() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_message(
                "peer-1",
                &sample_message("m-1", "hello", false, "delivered"),
                None,
            )
            .expect("store message");
        store
            .set_reaction_state("peer-1", "m-1", "bob", "🔥", true)
            .expect("store reaction");

        store.delete_message("m-1").expect("delete message");
        let reactions = store.reactions_for("peer-1").expect("load reactions");
        assert!(
            !reactions.contains_key("m-1"),
            "reactions must be removed with their message"
        );
    }

    #[test]
    fn sessions_roundtrip_as_a_peer_json_map() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        let mut sessions = HashMap::new();
        sessions.insert("peer-1".to_string(), r#"{"ratchet":1}"#.to_string());
        sessions.insert("peer-2".to_string(), r#"{"ratchet":2}"#.to_string());
        store.replace_sessions(&sessions).expect("store sessions");

        let loaded = store.load_sessions().expect("load sessions");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded["peer-1"], r#"{"ratchet":1}"#);
    }

    #[test]
    fn replace_sessions_drops_stale_peers() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .replace_sessions(&HashMap::from([("peer-1".to_string(), "one".to_string())]))
            .expect("first batch");
        store
            .replace_sessions(&HashMap::from([("peer-2".to_string(), "two".to_string())]))
            .expect("second batch");

        let loaded = store.load_sessions().expect("load sessions");
        assert_eq!(loaded.len(), 1, "the old peer session must be replaced");
        assert!(loaded.contains_key("peer-2"));
    }

    #[test]
    fn group_outbound_sessions_roundtrip_and_replace() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .replace_group_outbound(&HashMap::from([(
                "g1".to_string(),
                ("Squad".to_string(), r#"{"ratchet":1}"#.to_string()),
            )]))
            .expect("store outbound");
        let loaded = store.load_group_outbound().expect("load outbound");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["g1"].0, "Squad");
        assert_eq!(loaded["g1"].1, r#"{"ratchet":1}"#);

        // A fresh rewrite drops stale rows.
        store
            .replace_group_outbound(&HashMap::from([(
                "g2".to_string(),
                ("New".to_string(), r#"{"ratchet":2}"#.to_string()),
            )]))
            .expect("rewrite outbound");
        let loaded = store.load_group_outbound().expect("reload outbound");
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key("g2"));
    }

    #[test]
    fn group_inbound_sessions_roundtrip_and_replace() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .replace_group_inbound(&HashMap::from([(
                ("g1".to_string(), "alice".to_string()),
                ("Squad".to_string(), r#"{"ratchet":1}"#.to_string()),
            )]))
            .expect("store inbound");
        let loaded = store.load_group_inbound().expect("load inbound");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[&("g1".to_string(), "alice".to_string())].0, "Squad");
        assert_eq!(
            loaded[&("g1".to_string(), "alice".to_string())].1,
            r#"{"ratchet":1}"#
        );

        // A second sender's session for the same group coexists.
        store
            .replace_group_inbound(&HashMap::from([
                (
                    ("g1".to_string(), "alice".to_string()),
                    ("Squad".to_string(), r#"{"ratchet":1}"#.to_string()),
                ),
                (
                    ("g1".to_string(), "bob".to_string()),
                    ("Squad".to_string(), r#"{"ratchet":2}"#.to_string()),
                ),
            ]))
            .expect("store both senders");
        let loaded = store.load_group_inbound().expect("reload inbound");
        assert_eq!(loaded.len(), 2, "each sender keeps its own inbound session");
        assert!(loaded.contains_key(&("g1".to_string(), "bob".to_string())));

        // A fresh rewrite drops stale rows.
        store
            .replace_group_inbound(&HashMap::from([(
                ("g2".to_string(), "alice".to_string()),
                ("New".to_string(), r#"{"ratchet":3}"#.to_string()),
            )]))
            .expect("rewrite inbound");
        let loaded = store.load_group_inbound().expect("reload inbound");
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key(&("g2".to_string(), "alice".to_string())));
    }

    #[test]
    fn settings_roundtrip_set_get_and_delete() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        assert_eq!(store.get_setting("theme").expect("missing key"), None);

        store.set_setting("theme", "dark").expect("set theme");
        assert_eq!(
            store.get_setting("theme").expect("read theme").as_deref(),
            Some("dark")
        );
        store.set_setting("theme", "light").expect("update theme");
        assert_eq!(
            store.get_setting("theme").expect("read theme").as_deref(),
            Some("light")
        );

        store.delete_setting("theme").expect("delete theme");
        assert_eq!(
            store
                .get_setting("theme")
                .expect("missing key after delete"),
            None
        );
    }

    #[test]
    fn contact_upsert_and_list_roundtrip() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_contact(&ContactRow {
                peer_id: "peer-1".into(),
                display_name: Some("Alice".into()),
                username: None,
                avatar_url: None,
                last_seen: Some(1_700_000_000),
                curve25519_key: None,
                verified: false,
            })
            .expect("upsert contact");

        let contacts = store.contacts().expect("list contacts");
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name.as_deref(), Some("Alice"));
        assert_eq!(contacts[0].last_seen, Some(1_700_000_000));

        let fetched = store.get_contact("peer-1").expect("get contact");
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().peer_id, "peer-1");
        assert!(store.get_contact("ghost").expect("get ghost").is_none());
    }

    #[test]
    fn partial_contact_update_preserves_existing_fields() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_contact(&ContactRow {
                peer_id: "peer-1".into(),
                display_name: Some("Alice".into()),
                username: None,
                avatar_url: None,
                last_seen: None,
                curve25519_key: None,
                verified: false,
            })
            .expect("upsert with a name");
        // A presence update only carries a new last_seen; the name must stay.
        store
            .upsert_contact(&ContactRow {
                peer_id: "peer-1".into(),
                display_name: None,
                username: None,
                avatar_url: None,
                last_seen: Some(123),
                curve25519_key: None,
                verified: false,
            })
            .expect("upsert last_seen");

        let contact = store.get_contact("peer-1").expect("get contact").unwrap();
        assert_eq!(contact.display_name.as_deref(), Some("Alice"));
        assert_eq!(contact.last_seen, Some(123));
    }

    #[test]
    fn contact_curve_key_and_verified_flag_roundtrip() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_contact(&ContactRow {
                peer_id: "peer-1".into(),
                display_name: None,
                username: None,
                avatar_url: None,
                last_seen: None,
                curve25519_key: Some("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=".into()),
                verified: false,
            })
            .expect("upsert with key");

        let contact = store.get_contact("peer-1").expect("get contact").unwrap();
        assert!(contact.curve25519_key.is_some());
        assert!(!contact.verified);

        // Flip the verified flag through a fresh upsert.
        store
            .upsert_contact(&ContactRow {
                peer_id: "peer-1".into(),
                display_name: None,
                username: None,
                avatar_url: None,
                last_seen: None,
                curve25519_key: None, // keep-existing: must not wipe the key
                verified: true,
            })
            .expect("mark verified");

        let contact = store.get_contact("peer-1").expect("get contact").unwrap();
        assert!(contact.verified, "verified flag must persist");
        assert!(
            contact.curve25519_key.is_some(),
            "a None key update must keep the stored key"
        );
    }

    #[test]
    fn delete_contact_removes_the_row() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_contact(&ContactRow {
                peer_id: "peer-1".into(),
                display_name: Some("Alice".into()),
                username: None,
                avatar_url: None,
                last_seen: None,
                curve25519_key: None,
                verified: false,
            })
            .expect("upsert contact");
        store.delete_contact("peer-1").expect("delete contact");
        assert!(store.get_contact("peer-1").expect("get contact").is_none());
        // Deleting an unknown contact is a no-op, not an error.
        store
            .delete_contact("ghost")
            .expect("delete unknown contact");
    }

    #[test]
    fn delete_messages_for_removes_only_that_peers_rows() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_message(
                "peer-1",
                &sample_message("m-1", "for peer 1", false, "delivered"),
                None,
            )
            .expect("store for peer 1");
        store
            .upsert_message(
                "peer-2",
                &sample_message("m-2", "for peer 2", false, "delivered"),
                None,
            )
            .expect("store for peer 2");

        store
            .delete_messages_for("peer-1")
            .expect("delete peer 1 messages");
        assert!(store.messages_for("peer-1").expect("load").is_empty());
        assert_eq!(
            store.messages_for("peer-2").expect("load").len(),
            1,
            "another peer's history must survive"
        );
    }

    #[test]
    fn clear_messages_wipes_every_peers_history_but_keeps_contacts() {
        let store = ChatStore::open_in_memory(TEST_KEY).expect("open in-memory store");
        store
            .upsert_message(
                "peer-1",
                &sample_message("m-1", "for peer 1", false, "delivered"),
                None,
            )
            .expect("store for peer 1");
        store
            .upsert_message(
                "peer-2",
                &sample_message("m-2", "for peer 2", false, "delivered"),
                None,
            )
            .expect("store for peer 2");
        store
            .upsert_contact(&ContactRow {
                peer_id: "peer-1".into(),
                display_name: Some("Alice".into()),
                username: None,
                avatar_url: None,
                last_seen: None,
                curve25519_key: None,
                verified: false,
            })
            .expect("store a contact");

        store.clear_messages().expect("clear message history");

        assert!(store.all_messages().expect("load all").is_empty());
        assert_eq!(
            store.contacts().expect("contacts must survive").len(),
            1,
            "clear history must not remove contacts or sessions"
        );
        assert!(store
            .get_setting("theme")
            .expect("settings must survive")
            .is_none());
    }

    #[test]
    fn legacy_group_inbound_table_is_migrated_to_multi_sender_shape() {
        let path = temp_db_path("group-inbound-migration");
        let _ = std::fs::remove_file(&path);
        // Simulate a database created before multi-sender Megolm: one inbound
        // session per group, no sender column.
        {
            let conn = Connection::open(&path).expect("open legacy db");
            conn.execute_batch(
                "CREATE TABLE group_inbound (
                     group_id TEXT PRIMARY KEY,
                     name     TEXT NOT NULL,
                     pickle   TEXT NOT NULL
                 );
                 INSERT INTO group_inbound (group_id, name, pickle)
                     VALUES ('g1', 'Squad', '{\"ratchet\":1}');",
            )
            .expect("create legacy schema");
        }
        let store = ChatStore::open(&path, TEST_KEY).expect("migrated db must open");
        // The legacy row survives with an empty sender (the defensive fallback
        // key used while senders re-share their keys).
        let loaded = store.load_group_inbound().expect("load inbound");
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key(&("g1".to_string(), String::new())));
        // The multi-sender shape works on the migrated schema.
        store
            .replace_group_inbound(&HashMap::from([(
                ("g1".to_string(), "alice".to_string()),
                ("Squad".to_string(), r#"{"ratchet":1}"#.to_string()),
            )]))
            .expect("write multi-sender inbound");
        let loaded = store.load_group_inbound().expect("reload inbound");
        assert!(loaded.contains_key(&("g1".to_string(), "alice".to_string())));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn legacy_messages_table_gains_the_expires_at_column() {
        let path = temp_db_path("expires-migration");
        let _ = std::fs::remove_file(&path);
        // Simulate a database created before disappearing messages: a
        // `messages` table WITHOUT the expires_at column (which the initial
        // CREATE INDEX used to reference, breaking the open).
        {
            let conn = Connection::open(&path).expect("open legacy db");
            conn.execute_batch(
                "CREATE TABLE messages (
                     id        TEXT PRIMARY KEY,
                     peer_id   TEXT NOT NULL,
                     text      TEXT NOT NULL,
                     outgoing  INTEGER NOT NULL,
                     timestamp INTEGER NOT NULL,
                     status    TEXT NOT NULL,
                     client_id TEXT,
                     quote_json TEXT,
                     system_json TEXT
                 );
                 INSERT INTO messages (id, peer_id, text, outgoing, timestamp, status)
                     VALUES ('m-1', 'peer-1', 'old', 0, 1, 'delivered');",
            )
            .expect("create legacy schema");
        }
        let store = ChatStore::open(&path, TEST_KEY).expect("migrated db must open");
        // The legacy row loads, and a disappearing message persists fine now.
        let loaded = store.messages_for("peer-1").expect("load legacy row");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].text, "old");
        assert_eq!(loaded[0].expires_at, None);

        let mut disappearing = sample_message("m-2", "secret", true, "sent");
        disappearing.expires_at = Some(9_999_999);
        store
            .upsert_message("peer-1", &disappearing, None)
            .expect("persist with expires_at");
        let loaded = store.messages_for("peer-1").expect("reload");
        let expired = loaded
            .iter()
            .find(|m| m.id == "m-2")
            .expect("m-2 must exist");
        assert_eq!(expired.expires_at, Some(9_999_999));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn persisted_database_reopens_with_the_same_key_and_reads_back() {
        let path = temp_db_path("reopen");
        let _ = std::fs::remove_file(&path);
        {
            let store = ChatStore::open(&path, TEST_KEY).expect("open on disk");
            store
                .upsert_message(
                    "peer-1",
                    &sample_message("m-1", "survives a restart", false, "delivered"),
                    None,
                )
                .expect("store message");
            store
                .upsert_contact(&ContactRow {
                    peer_id: "peer-1".into(),
                    display_name: Some("Alice".into()),
                    username: None,
                    avatar_url: None,
                    last_seen: None,
                    curve25519_key: None,
                    verified: false,
                })
                .expect("store contact");
            store.set_setting("theme", "dark").expect("store setting");
            store
                .set_setting("my_username", "alice_42")
                .expect("store username");
            store
                .set_setting("my_avatar_url", "/media/abc123")
                .expect("store avatar");
        }
        // A brand-new store instance (as after an app restart) must read the
        // same rows back with the same key.
        {
            let store = ChatStore::open(&path, TEST_KEY).expect("reopen with the same key");
            let messages = store.messages_for("peer-1").expect("read messages");
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].text, "survives a restart");
            assert_eq!(store.contacts().expect("read contacts").len(), 1);
            assert_eq!(
                store.get_setting("theme").expect("read setting").as_deref(),
                Some("dark")
            );
            assert_eq!(
                store
                    .get_setting("my_username")
                    .expect("read username")
                    .as_deref(),
                Some("alice_42")
            );
            assert_eq!(
                store
                    .get_setting("my_avatar_url")
                    .expect("read avatar")
                    .as_deref(),
                Some("/media/abc123")
            );
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn opening_with_the_wrong_key_fails_when_the_codec_is_present() {
        if !codec_present() {
            // Plain `bundled` build (see Cargo.toml): no codec, so no key to
            // mismatch. The assertion is only meaningful on SQLCipher builds.
            return;
        }
        let path = temp_db_path("wrong-key");
        let _ = std::fs::remove_file(&path);
        ChatStore::open(&path, TEST_KEY).expect("create the database with key a");

        let wrong_key = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        match ChatStore::open(&path, wrong_key) {
            Err(StoreError::Sql(_)) => {}
            Err(other) => panic!("wrong key must surface as an sqlite error, got: {other:?}"),
            Ok(_) => panic!("a wrong key must not open the database"),
        }
        std::fs::remove_file(&path).ok();
    }
}
