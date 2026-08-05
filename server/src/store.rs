//! SQLite-backed persistence for the relay: the offline envelope queue and
//! the per-peer pre-key bundle directory.
//!
//! SECURITY MODEL
//! --------------
//! - Only opaque, client-encrypted `Envelope`s are persisted. The server
//!   never stores or inspects plaintext, keys or message content.
//! - Pre-key bundles are public directory data: the relay stores exactly the
//!   JSON a peer published so other peers can fetch it for the X3DH
//!   handshake. No secret material is ever involved.
//! - Rows live under `server/data/relay.db` (gitignored) at runtime and in a
//!   purely in-memory database during unit tests.
//!
//! CONCURRENCY
//! -----------
//! A single `rusqlite::Connection` is shared behind a `std::sync::Mutex`.
//! All operations are short (indexed lookups, bounded inserts) and SQLite is
//! WAL-free by default; this is deliberately simple for a single-process
//! relay. The bounds (500 blobs/peer, 7-day TTL) keep the DB tiny.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::OptionalExtension;
use rusqlite::Result as SqlResult;
use rusqlite::{params, Connection};

use crate::relay::Envelope;

/// Max number of offline ciphertext blobs buffered per peer.
pub const MAX_OFFLINE_BLOBS: usize = 500;

/// Envelopes older than this are purged from the queue.
pub const ENVELOPE_TTL_SECS: i64 = 7 * 24 * 60 * 60; // 7 days

/// Create a stable unix-seconds timestamp for a store operation.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Handle to the shared SQLite store. Internally synchronized; a Relay owns
/// exactly one Store.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (or create) the on-disk database at `path`, initializing the
    /// schema and purging expired rows on startup.
    pub fn open(path: impl AsRef<Path>) -> SqlResult<Self> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            }
        }
        let conn = Connection::open(path)?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        store.purge_expired(unix_now())?;
        Ok(store)
    }

    /// Open a purely in-memory database (unit tests only).
    #[cfg(test)]
    pub fn open_in_memory() -> SqlResult<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS envelopes (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                sender     TEXT NOT NULL,
                recipient  TEXT NOT NULL,
                payload    TEXT NOT NULL,
                seq        INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_envelopes_recipient_id
                ON envelopes (recipient, id);
            CREATE TABLE IF NOT EXISTS users (
                peer_id         TEXT PRIMARY KEY,
                curve25519_key  TEXT,
                ed25519_key     TEXT,
                display_name    TEXT,
                first_seen      INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS prekeys (
                peer_id     TEXT PRIMARY KEY,
                bundle_json TEXT NOT NULL,
                updated_at  INTEGER NOT NULL
            );",
        )?;

        // Migration: databases created before the signed-hello binding lack
        // the public-key columns, and databases created before the profile
        // feature lack the display-name column. SQLite has no
        // `ADD COLUMN IF NOT EXISTS`, so inspect the live schema and alter it
        // in place when needed.
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(users)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        for name in ["curve25519_key", "ed25519_key", "display_name"] {
            if !columns.iter().any(|c| c.as_str() == name) {
                conn.execute(&format!("ALTER TABLE users ADD COLUMN {name} TEXT"), [])?;
            }
        }
        Ok(())
    }

    /// Remember a peer's first sighting (INSERT OR IGNORE keeps the original).
    /// Kept for test/introspection use; the relay registers peers with keys
    /// via [`Store::register_user_with_keys`].
    #[cfg(test)]
    pub fn register_user(&self, peer_id: &str, first_seen: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users (peer_id, first_seen) VALUES (?1, ?2)
             ON CONFLICT(peer_id) DO NOTHING",
            params![peer_id, first_seen],
        )?;
        Ok(())
    }

    /// Register a peer with the public keys bound by its signed hello.
    ///
    /// "First-seen wins": an existing row's keys are never overwritten, but
    /// legacy rows with NULL keys are back-filled so returning peers become
    /// bound to their keys on their next signed hello.
    pub fn register_user_with_keys(
        &self,
        peer_id: &str,
        curve_b64: &str,
        ed_b64: &str,
        first_seen: i64,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users (peer_id, curve25519_key, ed25519_key, first_seen)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(peer_id) DO UPDATE SET
                 curve25519_key = COALESCE(users.curve25519_key, excluded.curve25519_key),
                 ed25519_key = COALESCE(users.ed25519_key, excluded.ed25519_key)",
            params![peer_id, curve_b64, ed_b64, first_seen],
        )?;
        Ok(())
    }

    /// The public keys bound to a peer, if registered: `(curve25519, ed25519)`.
    pub fn get_user_keys(&self, peer_id: &str) -> Option<(String, String)> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT curve25519_key, ed25519_key FROM users WHERE peer_id = ?1",
            params![peer_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap_or(None)
    }

    /// Set (or update) a peer's public display name — Signal-style profile
    /// data that any other peer can see in a pre-key lookup. Creating or
    /// updating an existing row is the same operation, so this works for both
    /// a brand-new peer and a returning one. The original `first_seen` is
    /// preserved on update; a brand-new row records the current time.
    pub fn set_display_name(&self, peer_id: &str, display_name: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users (peer_id, display_name, first_seen) VALUES (?1, ?2, ?3)
             ON CONFLICT(peer_id) DO UPDATE SET display_name = excluded.display_name",
            params![peer_id, display_name, unix_now()],
        )?;
        Ok(())
    }

    /// The peer's public display name, if one was set (NULL rows yield None).
    pub fn get_display_name(&self, peer_id: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT display_name FROM users WHERE peer_id = ?1",
            params![peer_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .unwrap_or(None)
        .flatten()
    }

    /// Store a peer's current pre-key bundle, replacing any previous bundle
    /// (INSERT OR REPLACE). The relay persists bundles as opaque JSON — it
    /// never inspects the key material.
    pub fn set_prekeys(&self, peer_id: &str, bundle_json: &str, now: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO prekeys (peer_id, bundle_json, updated_at)
             VALUES (?1, ?2, ?3)",
            params![peer_id, bundle_json, now],
        )?;
        Ok(())
    }

    /// The most recently published pre-key bundle JSON for a peer, if any.
    pub fn get_prekeys(&self, peer_id: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT bundle_json FROM prekeys WHERE peer_id = ?1",
            params![peer_id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None)
    }

    /// Append an offline envelope, enforcing the per-peer cap by evicting the
    /// oldest rows once `MAX_OFFLINE_BLOBS` is exceeded.
    pub fn enqueue(&self, envelope: &Envelope, created_at: i64) -> SqlResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO envelopes (sender, recipient, payload, seq, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                envelope.sender,
                envelope.recipient,
                envelope.payload,
                envelope.seq as i64,
                created_at
            ],
        )?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM envelopes WHERE recipient = ?1",
            params![envelope.recipient],
            |r| r.get(0),
        )?;
        if count > MAX_OFFLINE_BLOBS as i64 {
            let overflow = count - MAX_OFFLINE_BLOBS as i64;
            tx.execute(
                "DELETE FROM envelopes
                 WHERE id IN (
                     SELECT id FROM envelopes WHERE recipient = ?1 ORDER BY id LIMIT ?2
                 )",
                params![envelope.recipient, overflow],
            )?;
        }
        tx.commit()
    }

    /// List every queued envelope for a recipient (does NOT delete), oldest
    /// first. Expired rows are purged before the read.
    pub fn list_for(&self, recipient: &str, now: i64) -> Vec<Envelope> {
        self.purge_expired(now).ok();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT sender, recipient, payload, seq FROM envelopes WHERE recipient = ?1 ORDER BY id")
            .expect("valid statement");
        let rows = stmt
            .query_map(params![recipient], |row| {
                Ok(Envelope {
                    sender: row.get(0)?,
                    recipient: row.get(1)?,
                    payload: row.get(2)?,
                    seq: row.get::<_, i64>(3)? as u64,
                })
            })
            .expect("query ok");
        rows.filter_map(|r| r.ok()).collect()
    }

    /// Fetch and REMOVE all queued envelopes for a recipient whose `seq` is
    /// greater than `since`, oldest first. This is the fetch_since sync
    /// mechanism: a successful fetch acknowledges delivery. Expired rows are
    /// purged before the read.
    pub fn drain_since(&self, recipient: &str, since: u64, now: i64) -> Vec<Envelope> {
        self.purge_expired(now).ok();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().expect("tx ok");
        let rows: Vec<(i64, Envelope)> = {
            let mut stmt = tx
                .prepare(
                    "SELECT id, sender, recipient, payload, seq FROM envelopes
                     WHERE recipient = ?1 AND seq > ?2 ORDER BY id",
                )
                .expect("valid statement");
            let rows = stmt
                .query_map(params![recipient, since as i64], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        Envelope {
                            sender: row.get(1)?,
                            recipient: row.get(2)?,
                            payload: row.get(3)?,
                            seq: row.get::<_, i64>(4)? as u64,
                        },
                    ))
                })
                .expect("query ok")
                .filter_map(|r| r.ok())
                .collect();
            rows
        };
        // Deleting one row at a time avoids dynamic SQL with a
        // variable-length IN clause; bounded by MAX_OFFLINE_BLOBS.
        for (id, _) in &rows {
            tx.execute("DELETE FROM envelopes WHERE id = ?1", params![id])
                .ok();
        }
        tx.commit().ok();
        rows.into_iter().map(|(_, env)| env).collect()
    }

    /// Delete every envelope older than `ENVELOPE_TTL_SECS`. Returns the
    /// number of purged rows.
    pub fn purge_expired(&self, now: i64) -> SqlResult<usize> {
        let conn = self.conn.lock().unwrap();
        let cutoff = now - ENVELOPE_TTL_SECS;
        conn.execute(
            "DELETE FROM envelopes WHERE created_at < ?1",
            params![cutoff],
        )
    }

    /// Number of queued envelopes for a recipient (tests/introspection).
    #[cfg(test)]
    pub fn count_for(&self, recipient: &str) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM envelopes WHERE recipient = ?1",
            params![recipient],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c as usize)
        .unwrap_or(0)
    }

    /// First-seen timestamp for a peer, if registered (tests/introspection).
    #[cfg(test)]
    pub fn first_seen_for(&self, peer_id: &str) -> Option<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT first_seen FROM users WHERE peer_id = ?1",
            params![peer_id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(sender: &str, recipient: &str, seq: u64) -> Envelope {
        Envelope {
            sender: sender.into(),
            recipient: recipient.into(),
            payload: format!("blob-{seq}"),
            seq,
        }
    }

    #[test]
    fn enqueue_and_list_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        store.enqueue(&env("a", "b", 1), now).unwrap();
        store.enqueue(&env("a", "b", 2), now).unwrap();
        assert_eq!(store.count_for("b"), 2);
        let listed = store.list_for("b", now);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].seq, 1);
        assert_eq!(listed[1].seq, 2);
    }

    #[test]
    fn cap_evicts_oldest_envelopes() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        for seq in 0..(MAX_OFFLINE_BLOBS as u64 + 5) {
            store.enqueue(&env("a", "b", seq), now).unwrap();
        }
        assert_eq!(store.count_for("b"), MAX_OFFLINE_BLOBS);
        let listed = store.list_for("b", now);
        assert_eq!(listed[0].seq, 5, "oldest must be evicted");
        assert_eq!(listed.last().unwrap().seq, MAX_OFFLINE_BLOBS as u64 + 4);
    }

    #[test]
    fn ttl_purges_expired_envelopes() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        store
            .enqueue(&env("a", "b", 1), now - ENVELOPE_TTL_SECS - 1)
            .unwrap();
        store.enqueue(&env("a", "b", 2), now).unwrap();
        assert_eq!(store.count_for("b"), 2);
        let purged = store.purge_expired(now).unwrap();
        assert_eq!(purged, 1);
        let listed = store.list_for("b", now);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].seq, 2);
    }

    #[test]
    fn fetch_since_filters_and_drains() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        for seq in 1..=5 {
            store.enqueue(&env("a", "b", seq), now).unwrap();
        }
        let fetched = store.drain_since("b", 3, now);
        assert_eq!(fetched.len(), 2);
        assert_eq!(fetched[0].seq, 4);
        assert_eq!(fetched[1].seq, 5);
        assert_eq!(store.count_for("b"), 3, "older rows must survive the drain");
        // A second fetch above the same cursor is now empty.
        assert!(store.drain_since("b", 3, now).is_empty());
    }

    #[test]
    fn fetch_since_purges_expired_first() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        store
            .enqueue(&env("a", "b", 1), now - ENVELOPE_TTL_SECS - 1)
            .unwrap();
        store.enqueue(&env("a", "b", 2), now).unwrap();
        let fetched = store.drain_since("b", 0, now);
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].seq, 2, "expired envelope must not be fetched");
    }

    #[test]
    fn register_user_records_first_seen() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        store.register_user("peer-x", now).unwrap();
        assert_eq!(store.first_seen_for("peer-x"), Some(now));
        // Re-registration must not overwrite first_seen.
        store.register_user("peer-x", now + 100).unwrap();
        assert_eq!(store.first_seen_for("peer-x"), Some(now));
        assert_eq!(store.first_seen_for("peer-ghost"), None);
    }

    #[test]
    fn register_user_with_keys_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        store
            .register_user_with_keys("peer-x", "curve-a", "ed-a", now)
            .unwrap();
        let keys = store.get_user_keys("peer-x").expect("keys must be stored");
        assert_eq!(keys, ("curve-a".to_string(), "ed-a".to_string()));
        assert_eq!(store.first_seen_for("peer-x"), Some(now));
    }

    #[test]
    fn register_user_with_keys_keeps_existing_keys() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        store
            .register_user_with_keys("peer-x", "curve-a", "ed-a", now)
            .unwrap();
        // A second registration with different keys must not overwrite.
        store
            .register_user_with_keys("peer-x", "curve-b", "ed-b", now + 10)
            .unwrap();
        let keys = store.get_user_keys("peer-x").expect("keys must be stored");
        assert_eq!(keys, ("curve-a".to_string(), "ed-a".to_string()));
        // First-seen must also be preserved.
        assert_eq!(store.first_seen_for("peer-x"), Some(now));
    }

    #[test]
    fn register_user_with_keys_backfills_legacy_null_keys() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        // A legacy row registered without keys (pre-migration).
        store.register_user("peer-x", now).unwrap();
        assert_eq!(store.get_user_keys("peer-x"), None);
        // The signed hello then binds the keys without touching first_seen.
        store
            .register_user_with_keys("peer-x", "curve-a", "ed-a", now + 10)
            .unwrap();
        let keys = store.get_user_keys("peer-x").expect("keys must be stored");
        assert_eq!(keys, ("curve-a".to_string(), "ed-a".to_string()));
        assert_eq!(store.first_seen_for("peer-x"), Some(now));
    }

    #[test]
    fn set_display_name_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        store.set_display_name("peer-x", "Test Alice").unwrap();
        assert_eq!(
            store.get_display_name("peer-x").as_deref(),
            Some("Test Alice")
        );
    }

    #[test]
    fn set_display_name_updates_existing_name() {
        let store = Store::open_in_memory().unwrap();
        store.set_display_name("peer-x", "First").unwrap();
        store.set_display_name("peer-x", "Second").unwrap();
        assert_eq!(store.get_display_name("peer-x").as_deref(), Some("Second"));
    }

    #[test]
    fn get_display_name_returns_none_for_unknown_peer() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_display_name("peer-ghost"), None);
    }

    #[test]
    fn get_display_name_returns_none_for_unset_column() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        store
            .register_user_with_keys("peer-x", "curve-a", "ed-a", now)
            .unwrap();
        assert_eq!(
            store.get_display_name("peer-x"),
            None,
            "a user registered without a name must yield None"
        );
    }

    #[test]
    fn set_prekeys_roundtrip_returns_stored_json() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        let json = r#"{"version":1,"identity_key":"AA=="}"#;
        store.set_prekeys("peer-x", json, now).unwrap();
        assert_eq!(store.get_prekeys("peer-x").as_deref(), Some(json));
    }

    #[test]
    fn set_prekeys_replaces_previous_bundle() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        store.set_prekeys("peer-x", "first", now).unwrap();
        store.set_prekeys("peer-x", "second", now + 1).unwrap();
        assert_eq!(store.get_prekeys("peer-x").as_deref(), Some("second"));
    }

    #[test]
    fn get_prekeys_returns_none_for_unknown_peer() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_prekeys("peer-ghost"), None);
    }

    #[test]
    fn migration_adds_key_columns_to_legacy_users_table() {
        let path = std::env::temp_dir().join(format!(
            "whisper-relay-migration-{}.db",
            uuid::Uuid::new_v4()
        ));
        // Simulate a database created before the signed-hello binding.
        {
            let conn = Connection::open(&path).expect("open legacy db");
            conn.execute_batch(
                "CREATE TABLE users (
                     peer_id    TEXT PRIMARY KEY,
                     first_seen INTEGER NOT NULL
                 );
                 INSERT INTO users (peer_id, first_seen) VALUES ('peer-old', 1);",
            )
            .expect("create legacy schema");
        }
        let store = Store::open(&path).expect("migrated db must open");
        // The key columns must work and the legacy row must survive.
        store
            .register_user_with_keys("peer-old", "curve-a", "ed-a", 999)
            .unwrap();
        let keys = store
            .get_user_keys("peer-old")
            .expect("keys must be stored");
        assert_eq!(keys, ("curve-a".to_string(), "ed-a".to_string()));
        assert_eq!(store.first_seen_for("peer-old"), Some(1));
        // The display-name column must be added by the same migration.
        store.set_display_name("peer-old", "Old One").unwrap();
        assert_eq!(
            store.get_display_name("peer-old").as_deref(),
            Some("Old One")
        );
        store
            .register_user_with_keys("peer-new", "curve-b", "ed-b", 2)
            .unwrap();
        let keys = store
            .get_user_keys("peer-new")
            .expect("keys must be stored");
        assert_eq!(keys, ("curve-b".to_string(), "ed-b".to_string()));
        std::fs::remove_file(&path).ok();
    }
}
