//! Sampling support for partial backup restores.
//!
//! Full restores on every scheduled run are impractical for multi-TB repos.
//! Sampling restores a deterministic subset of files instead, so runs stay
//! cheap while still exercising the real restore path end to end.  Full
//! restores run on a separate cadence (see `full_schedule` in the config).
//!
//! A sample spec is either:
//! - a **percentage** (`"10"` or `"10%"`) — a stable, hash-based subset of
//!   all files in the snapshot, or
//! - a **path glob** (`"*/logs/*.log"`) — every file whose path matches.
//!
//! Percent selection is deterministic: each path is hashed with SHA-256 and
//! kept if the hash falls below the threshold, so the *same snapshot and
//! same spec always select the same files* — which keeps drift detection
//! between sampled runs meaningful.

use sha2::{Digest, Sha256};

use crate::repo::ListedFile;

/// A parsed sample specification.
#[derive(Debug, Clone, PartialEq)]
pub enum SampleSpec {
    /// Keep `percent` (0 < p <= 100) of the files, chosen by path hash.
    Percent(f64),
    /// Keep every file whose stored path matches this glob.
    Glob(String),
}

/// Parse a user-supplied sample spec.
///
/// Anything that parses as a number (optionally with a trailing `%`) is a
/// percentage; everything else is treated as a glob pattern.
///
/// # Errors
///
/// Returns an error if a percentage is out of range (0, 100] or a glob
/// pattern is syntactically invalid.
pub fn parse_sample_spec(spec: &str) -> anyhow::Result<SampleSpec> {
    let trimmed = spec.trim();
    let numeric = trimmed.strip_suffix('%').unwrap_or(trimmed);
    if let Ok(pct) = numeric.parse::<f64>() {
        if !(pct > 0.0 && pct <= 100.0) {
            anyhow::bail!("Sample percentage must be in (0, 100], got {pct}");
        }
        return Ok(SampleSpec::Percent(pct));
    }

    // Validate the glob so typos fail fast instead of selecting nothing.
    glob::Pattern::new(trimmed)
        .map_err(|e| anyhow::anyhow!("Invalid sample glob '{trimmed}': {e}"))?;
    Ok(SampleSpec::Glob(trimmed.to_string()))
}

/// Deterministic bucket for a path in `[0, 10_000)`, derived from SHA-256.
///
/// Using the path hash (rather than list position) makes the selection
/// independent of listing order and stable across runs and tool versions.
///
/// # Panics
///
/// Cannot panic: SHA-256 digests are always 32 bytes, so the first 8 bytes
/// always exist.
#[must_use]
pub fn path_bucket(path: &str) -> u64 {
    let digest = Sha256::digest(path.as_bytes());
    let bytes: [u8; 8] = digest[..8].try_into().expect("sha256 digest is >= 8 bytes");
    u64::from_be_bytes(bytes) % 10_000
}

/// Select the subset of `files` matching `spec`, preserving input order.
#[must_use]
pub fn select_files(files: &[ListedFile], spec: &SampleSpec) -> Vec<ListedFile> {
    match spec {
        SampleSpec::Percent(pct) => {
            // percent * 100 gives a threshold out of 10_000; the f64→u64
            // cast is safe because pct is validated to (0, 100].
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let threshold = (pct * 100.0) as u64;
            files
                .iter()
                .filter(|f| path_bucket(&f.path) < threshold)
                .cloned()
                .collect()
        }
        SampleSpec::Glob(pattern) => {
            let Ok(pat) = glob::Pattern::new(pattern) else {
                return Vec::new();
            };
            files
                .iter()
                .filter(|f| pat.matches(&f.path))
                .cloned()
                .collect()
        }
    }
}

/// The scope label persisted with sampled runs (`None` for full restores).
///
/// Drift detection only compares runs with the *same scope*, so sampled
/// and full manifests of one snapshot never collide.
#[must_use]
pub fn scope_label(spec: Option<&str>) -> Option<String> {
    spec.map(|s| format!("sample:{s}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> ListedFile {
        ListedFile {
            path: path.to_string(),
            size,
        }
    }

    #[test]
    fn test_parse_percent_specs() {
        assert_eq!(parse_sample_spec("10").unwrap(), SampleSpec::Percent(10.0));
        assert_eq!(parse_sample_spec("10%").unwrap(), SampleSpec::Percent(10.0));
        assert_eq!(parse_sample_spec("2.5").unwrap(), SampleSpec::Percent(2.5));
        assert_eq!(
            parse_sample_spec("100").unwrap(),
            SampleSpec::Percent(100.0)
        );
    }

    #[test]
    fn test_parse_percent_bounds() {
        assert!(parse_sample_spec("0").is_err());
        assert!(parse_sample_spec("-5").is_err());
        assert!(parse_sample_spec("101").is_err());
    }

    #[test]
    fn test_parse_glob_specs() {
        assert_eq!(
            parse_sample_spec("*/logs/*.log").unwrap(),
            SampleSpec::Glob("*/logs/*.log".into())
        );
        assert!(parse_sample_spec("[").is_err(), "invalid glob must fail");
    }

    #[test]
    fn test_percent_selection_is_deterministic_and_stable() {
        let files: Vec<ListedFile> = (0..1000)
            .map(|i| file(&format!("/data/file_{i:04}.bin"), 100))
            .collect();

        let spec = SampleSpec::Percent(25.0);
        let first = select_files(&files, &spec);
        let second = select_files(&files, &spec);
        assert_eq!(first, second, "selection must be deterministic");

        // 25% of 1000 hash-distributed paths should be near 250.
        assert!((200..300).contains(&first.len()), "got {}", first.len());
    }

    #[test]
    fn test_percent_selection_independent_of_order() {
        let mut files: Vec<ListedFile> =
            (0..500).map(|i| file(&format!("/data/f{i}"), i)).collect();
        let spec = SampleSpec::Percent(30.0);
        let forward: Vec<String> = select_files(&files, &spec)
            .iter()
            .map(|f| f.path.clone())
            .collect();

        files.reverse();
        let mut backward: Vec<String> = select_files(&files, &spec)
            .iter()
            .map(|f| f.path.clone())
            .collect();
        backward.sort();
        let mut forward_sorted = forward;
        forward_sorted.sort();
        assert_eq!(forward_sorted, backward);
    }

    #[test]
    fn test_full_percent_selects_everything() {
        let files: Vec<ListedFile> = (0..50).map(|i| file(&format!("/data/f{i}"), 1)).collect();
        let selected = select_files(&files, &SampleSpec::Percent(100.0));
        assert_eq!(selected.len(), files.len());
    }

    #[test]
    fn test_glob_selection() {
        let files = vec![
            file("/data/logs/a.log", 1),
            file("/data/logs/b.log", 2),
            file("/data/db/main.sqlite", 3),
        ];
        let selected = select_files(&files, &SampleSpec::Glob("/data/logs/*.log".into()));
        assert_eq!(selected.len(), 2);
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        for f in &selected {
            assert!(f.path.ends_with(".log"));
        }
    }

    #[test]
    fn test_scope_label() {
        assert_eq!(scope_label(None), None);
        assert_eq!(scope_label(Some("10")), Some("sample:10".into()));
        assert_eq!(scope_label(Some("*/x/*")), Some("sample:*/x/*".into()));
    }
}
