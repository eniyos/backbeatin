use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use anyhow::Context;
use sha2::{Digest, Sha256};

use crate::repo::RestoreOutcome;

// ---------------------------------------------------------------------------
// Manifest types
// ---------------------------------------------------------------------------

/// A single entry in a restore manifest.
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    /// Relative path of the file within the restored tree.
    pub relative_path: String,
    /// Hex-encoded SHA-256 digest of the file contents.
    pub sha256: String,
    /// File size in bytes.
    pub size: u64,
}

/// A manifest describing every file under a restored directory tree.
#[derive(Debug, Clone)]
pub struct Manifest {
    /// All entries, indexed by relative path for convenient lookup.
    pub entries: BTreeMap<String, ManifestEntry>,
    /// Total number of files discovered.
    pub total_files: u64,
    /// Total size of all files in bytes.
    pub total_bytes: u64,
}

impl Manifest {
    /// Return the list of entries (sorted by path).
    pub fn sorted_entries(&self) -> Vec<&ManifestEntry> {
        self.entries.values().collect()
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// The outcome of a verification check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationStatus {
    Pass,
    Fail,
}

/// The complete result of a verification run.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub status: VerificationStatus,
    pub message: String,
    pub manifest: Manifest,
}

// ---------------------------------------------------------------------------
// Manifest computation
// ---------------------------------------------------------------------------

/// Walk every file under `dir` and produce a [`Manifest`] recording the
/// relative path, SHA-256 digest, and size of each file.
///
/// Directories and symlinks are recorded in the manifest but skipped for
/// hashing (only regular files are hashed).  Returns an error if a file
/// cannot be read.
pub fn compute_manifest(dir: &Path) -> anyhow::Result<Manifest> {
    let mut entries = BTreeMap::new();
    let mut total_files: u64 = 0;
    let mut total_bytes: u64 = 0;

    for entry in walkdir::WalkDir::new(dir).min_depth(1).into_iter() {
        let entry = entry.context("Failed to walk restored directory tree")?;

        let relative_path = entry
            .path()
            .strip_prefix(dir)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();

        if entry.file_type().is_dir() {
            // Record directories with a trailing slash so they're
            // distinguishable from files, but skip hashing.
            entries.insert(
                relative_path.clone(),
                ManifestEntry {
                    relative_path: relative_path.clone(),
                    sha256: String::new(),
                    size: 0,
                },
            );
            continue;
        }

        let metadata = entry.metadata().context("Failed to read file metadata")?;
        let size = metadata.len();

        let sha256 = if entry.file_type().is_file() {
            let mut file = std::fs::File::open(entry.path())
                .context(format!("Failed to open file: {}", entry.path().display()))?;
            let mut hasher = Sha256::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = file.read(&mut buf).context("Failed to read file")?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            hex::encode(hasher.finalize())
        } else {
            // Symlinks and other non-regular files — record but don't hash.
            String::new()
        };

        total_files += 1;
        total_bytes += size;

        entries.insert(
            relative_path.clone(),
            ManifestEntry {
                relative_path,
                sha256,
                size,
            },
        );
    }

    Ok(Manifest {
        entries,
        total_files,
        total_bytes,
    })
}

// ---------------------------------------------------------------------------
// Verification logic
// ---------------------------------------------------------------------------

/// Compare the backend-reported restore outcome against the local manifest
/// and return a [`VerificationResult`].
///
/// For Phase 1 this checks:
///   * The manifest is non-empty (files were actually restored).
///   * The file count from the manifest roughly matches what the backend
///     reported (allow a small delta for metadata entries).
pub fn verify_restore(outcome: &RestoreOutcome, manifest: &Manifest) -> VerificationResult {
    let non_dir_entries = manifest
        .entries
        .values()
        .filter(|e| e.size > 0 || !e.sha256.is_empty())
        .count() as u64;

    // --- Check 1: restore actually produced files ---
    if manifest.entries.is_empty() {
        return VerificationResult {
            status: VerificationStatus::Fail,
            message: "Restore produced an empty directory — no files were restored".to_string(),
            manifest: manifest.clone(),
        };
    }

    // --- Check 2: file count plausibility ---
    // Allow up to 5% discrepancy: the backend may count differently from
    // what we discover on disk (e.g. metadata files, directories).
    let backend_count = outcome.files_count;
    let actual_count = non_dir_entries;

    if backend_count > 0 {
        let diff = if backend_count > actual_count {
            backend_count - actual_count
        } else {
            actual_count - backend_count
        };
        let threshold = (backend_count.max(actual_count) as f64 * 0.05).ceil() as u64;
        if diff > threshold.max(1) {
            let msg = format!(
                "File count mismatch: backend reported {} files, but restore produced {} \
                 files on disk (difference {} exceeds {} threshold)",
                backend_count, actual_count, diff, threshold
            );
            return VerificationResult {
                status: VerificationStatus::Fail,
                message: msg,
                manifest: manifest.clone(),
            };
        }
    }

    // --- Check 3: no zero-byte files that shouldn't be zero ---
    // (This is a soft check — some legit files may be empty.  We warn but
    // don't fail on this in Phase 1.)
    let zero_byte_files: Vec<&ManifestEntry> = manifest
        .entries
        .values()
        .filter(|e| e.size == 0 && !e.sha256.is_empty())
        .collect();

    let pass_msg = if zero_byte_files.is_empty() {
        format!(
            "Restore verified successfully: {} files, {} bytes restored from snapshot {}",
            non_dir_entries, manifest.total_bytes, outcome.snapshot_id
        )
    } else {
        format!(
            "Restore verified successfully: {} files, {} bytes restored from snapshot {} \
             ({} zero-byte files found)",
            non_dir_entries, manifest.total_bytes, outcome.snapshot_id, zero_byte_files.len()
        )
    };

    VerificationResult {
        status: VerificationStatus::Pass,
        message: pass_msg,
        manifest: manifest.clone(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_manifest_fails() {
        let manifest = Manifest {
            entries: BTreeMap::new(),
            total_files: 0,
            total_bytes: 0,
        };
        let outcome = RestoreOutcome {
            snapshot_id: "test".into(),
            files_count: 10,
            bytes_restored: 1000,
        };

        let result = verify_restore(&outcome, &manifest);
        assert_eq!(result.status, VerificationStatus::Fail);
    }

    #[test]
    fn test_file_count_tolerance() {
        let mut entries = BTreeMap::new();
        for i in 0u64..100 {
            entries.insert(
                format!("file_{}", i),
                ManifestEntry {
                    relative_path: format!("file_{}", i),
                    sha256: "abc".into(),
                    size: 100,
                },
            );
        }
        let manifest = Manifest {
            total_files: 100,
            total_bytes: 10_000,
            entries,
        };
        let outcome = RestoreOutcome {
            snapshot_id: "test".into(),
            files_count: 105, // 5% diff — should still pass
            bytes_restored: 10_500,
        };

        let result = verify_restore(&outcome, &manifest);
        assert_eq!(result.status, VerificationStatus::Pass);
    }

    #[test]
    fn test_large_file_count_mismatch_fails() {
        let mut entries = BTreeMap::new();
        entries.insert(
            "file1".into(),
            ManifestEntry {
                relative_path: "file1".into(),
                sha256: "abc".into(),
                size: 100,
            },
        );
        let manifest = Manifest {
            total_files: 1,
            total_bytes: 100,
            entries,
        };
        let outcome = RestoreOutcome {
            snapshot_id: "test".into(),
            files_count: 500, // wildly different — should fail
            bytes_restored: 50_000,
        };

        let result = verify_restore(&outcome, &manifest);
        assert_eq!(result.status, VerificationStatus::Fail);
    }
}
