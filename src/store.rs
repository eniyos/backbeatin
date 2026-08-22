use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::config::RepoConfig;
use crate::verify::{Manifest, VerificationStatus};

// ---------------------------------------------------------------------------
// Run record
// ---------------------------------------------------------------------------

/// A persisted record of a single verification run.
#[derive(Debug, Clone)]
pub struct VerificationRunRecord {
    pub id: i64,
    pub repo_id: i64,
    pub repo_name: String,
    pub snapshot_id: String,
    pub status: String,
    pub files_count: u64,
    pub bytes_restored: u64,
    pub message: String,
    pub started_at: i64,
    pub completed_at: i64,
    /// Hex-encoded Ed25519 signature of the run data, if signed.
    pub signature_hex: Option<String>,
    /// Hex-encoded Ed25519 public key that produced the signature, if signed.
    pub public_key_hex: Option<String>,
}

/// Data needed to insert a new verification run.
#[derive(Debug, Clone)]
pub struct NewVerificationRun {
    pub repo_name: String,
    pub repo_backend: String,
    pub repo_uri: String,
    pub snapshot_id: String,
    pub status: VerificationStatus,
    pub files_count: u64,
    pub bytes_restored: u64,
    pub message: String,
    pub manifest: Option<Manifest>,
    pub started_at: i64,
    pub completed_at: i64,
    /// Hex-encoded Ed25519 signature (optional, set after insertion).
    pub signature_hex: Option<String>,
    /// Hex-encoded public key corresponding to the signature.
    pub public_key_hex: Option<String>,
}

/// Return the current Unix epoch timestamp (seconds).
#[must_use]
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .cast_signed()
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Persistent storage for verification history.
///
/// Uses `SQLite` via `rusqlite` (bundled) so there is no system dependency.
/// The store is created from a file path or in-memory (for tests).
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (or create) the database at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the `SQLite` file cannot be opened or the
    /// schema cannot be created.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.ensure_schema()?;
        Ok(store)
    }

    /// Open an in-memory database (useful for tests).
    ///
    /// # Errors
    ///
    /// Returns an error if the in-memory `SQLite` database cannot be
    /// created or the schema cannot be initialised.
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.ensure_schema()?;
        Ok(store)
    }

    // ------------------------------------------------------------------
    // Schema
    // ------------------------------------------------------------------

    fn ensure_schema(&self) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS repos (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL UNIQUE,
                backend     TEXT NOT NULL,
                uri         TEXT NOT NULL,
                created_at  INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            CREATE TABLE IF NOT EXISTS verification_runs (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_id         INTEGER NOT NULL REFERENCES repos(id),
                snapshot_id     TEXT NOT NULL,
                status          TEXT NOT NULL,
                files_count     INTEGER NOT NULL,
                bytes_restored  INTEGER NOT NULL,
                manifest_json   TEXT,
                message         TEXT,
                signature_hex   TEXT,
                public_key_hex  TEXT,
                started_at      INTEGER NOT NULL,
                completed_at    INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS notifications_sent (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id          INTEGER NOT NULL REFERENCES verification_runs(id),
                webhook_url     TEXT NOT NULL,
                success         INTEGER NOT NULL,
                response_code   INTEGER,
                sent_at         INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );",
        )?;

        // Migration: add signing columns to existing databases.
        let _ = conn.execute_batch(
            "ALTER TABLE verification_runs ADD COLUMN signature_hex TEXT;
             ALTER TABLE verification_runs ADD COLUMN public_key_hex TEXT;",
        );

        Ok(())
    }

    // ------------------------------------------------------------------
    // Repos
    // ------------------------------------------------------------------

    /// Look up a repo by name, creating a row if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query or insert fails.
    pub fn get_or_create_repo(&self, config: &RepoConfig) -> anyhow::Result<i64> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Try to find an existing repo row.
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM repos WHERE name = ?1",
                [&config.name],
                |row| row.get(0),
            )
            .ok();

        if let Some(id) = existing {
            // Optionally update URI / backend if they've changed.
            conn.execute(
                "UPDATE repos SET backend = ?1, uri = ?2 WHERE id = ?3",
                rusqlite::params![
                    format!("{:?}", config.backend).to_lowercase(),
                    config.uri,
                    id,
                ],
            )?;
            return Ok(id);
        }

        // Insert a new repo row.
        conn.execute(
            "INSERT INTO repos (name, backend, uri) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                config.name,
                format!("{:?}", config.backend).to_lowercase(),
                config.uri,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Internal helper: look up a repo by raw name/backend/URI.
    fn get_or_create_repo_raw(&self, name: &str, backend: &str, uri: &str) -> anyhow::Result<i64> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let existing: Option<i64> = conn
            .query_row("SELECT id FROM repos WHERE name = ?1", [name], |row| {
                row.get(0)
            })
            .ok();

        if let Some(id) = existing {
            conn.execute(
                "UPDATE repos SET backend = ?1, uri = ?2 WHERE id = ?3",
                rusqlite::params![backend, uri, id],
            )?;
            return Ok(id);
        }

        conn.execute(
            "INSERT INTO repos (name, backend, uri) VALUES (?1, ?2, ?3)",
            rusqlite::params![name, backend, uri],
        )?;
        Ok(conn.last_insert_rowid())
    }

    // ------------------------------------------------------------------
    // Verification runs
    // ------------------------------------------------------------------

    /// Insert a new verification run record and return its row ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying repository row cannot be
    /// resolved, the manifest cannot be serialized, or the insert fails.
    pub fn insert_verification_run(&self, run: &NewVerificationRun) -> anyhow::Result<i64> {
        let repo_id =
            self.get_or_create_repo_raw(&run.repo_name, &run.repo_backend, &run.repo_uri)?;

        let manifest_json = run
            .manifest
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        let status_str = match run.status {
            VerificationStatus::Pass => "pass",
            VerificationStatus::Fail => "fail",
        };

        let conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        conn.execute(
            "INSERT INTO verification_runs
                (repo_id, snapshot_id, status, files_count, bytes_restored,
                 manifest_json, message, signature_hex, public_key_hex,
                 started_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                repo_id,
                run.snapshot_id,
                status_str,
                run.files_count,
                run.bytes_restored,
                manifest_json,
                run.message,
                run.signature_hex,
                run.public_key_hex,
                run.started_at,
                run.completed_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Update a verification run with its Ed25519 signature after insertion.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn update_run_signature(
        &self,
        run_id: i64,
        signature_hex: &str,
        public_key_hex: &str,
    ) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        conn.execute(
            "UPDATE verification_runs
             SET signature_hex = ?1, public_key_hex = ?2
             WHERE id = ?3",
            rusqlite::params![signature_hex, public_key_hex, run_id],
        )?;
        Ok(())
    }

    /// Return the most recent verification runs for a given repo.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn recent_runs(
        &self,
        repo_name: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<VerificationRunRecord>> {
        let mut records = Vec::new();
        {
            let conn = self
                .conn
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut stmt = conn.prepare(
                "SELECT r.id, r.repo_id, r.snapshot_id, r.status,
                        r.files_count, r.bytes_restored, r.message,
                        r.signature_hex, r.public_key_hex,
                        r.started_at, r.completed_at, p.name
                 FROM verification_runs r
                 JOIN repos p ON p.id = r.repo_id
                 WHERE p.name = ?1
                 ORDER BY r.completed_at DESC
                 LIMIT ?2",
            )?;

            let rows = stmt.query_map(rusqlite::params![repo_name, limit], |row| {
                Ok(VerificationRunRecord {
                    id: row.get(0)?,
                    repo_id: row.get(1)?,
                    snapshot_id: row.get(2)?,
                    status: row.get(3)?,
                    files_count: row.get::<_, i64>(4)?.cast_unsigned(),
                    bytes_restored: row.get::<_, i64>(5)?.cast_unsigned(),
                    message: row.get(6)?,
                    signature_hex: row.get(7)?,
                    public_key_hex: row.get(8)?,
                    started_at: row.get(9)?,
                    completed_at: row.get(10)?,
                    repo_name: row.get(11)?,
                })
            })?;

            for row in rows {
                records.push(row?);
            }
            // conn is dropped here when the scope ends
        }
        Ok(records)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::VerificationStatus;
    use std::collections::BTreeMap;

    #[test]
    fn test_create_in_memory_store() {
        let store = Store::open_in_memory().expect("should open in-memory DB");
        // Schema should be created automatically.
        let conn = store.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM repos", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_get_or_create_repo() {
        let store = Store::open_in_memory().unwrap();
        let config = RepoConfig {
            name: "test-repo".into(),
            backend: crate::config::BackendType::Restic,
            uri: "s3:bucket/path".into(),
            credential_env_vars: std::collections::HashMap::new(),
            snapshot_tag: None,
            schedule: "0 0 * * * *".to_string(),
        };

        let id1 = store.get_or_create_repo(&config).unwrap();
        let id2 = store.get_or_create_repo(&config).unwrap();
        assert_eq!(id1, id2, "calling twice should return the same row");
    }

    #[test]
    fn test_insert_and_query_run() {
        let store = Store::open_in_memory().unwrap();

        let manifest = Manifest {
            entries: BTreeMap::new(),
            total_files: 0,
            total_bytes: 0,
        };

        let run = NewVerificationRun {
            repo_name: "test-repo".into(),
            repo_backend: "restic".into(),
            repo_uri: "s3:bucket/test".into(),
            snapshot_id: "abc123".into(),
            status: VerificationStatus::Pass,
            files_count: 100,
            bytes_restored: 50000,
            message: "All good".into(),
            manifest: Some(manifest),
            signature_hex: None,
            public_key_hex: None,
            started_at: 1_705_318_800,
            completed_at: 1_705_318_860,
        };

        let run_id = store.insert_verification_run(&run).unwrap();
        assert!(run_id > 0);

        let records = store.recent_runs("test-repo", 10).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].snapshot_id, "abc123");
        assert_eq!(records[0].status, "pass");
        assert_eq!(records[0].files_count, 100);
    }

    #[test]
    fn test_recent_runs_empty_for_unknown_repo() {
        let store = Store::open_in_memory().unwrap();
        let records = store.recent_runs("nonexistent", 10).unwrap();
        assert!(records.is_empty());
    }
}
