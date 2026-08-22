use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use sha2::{Digest, Sha256};

/// Directory name (under `$HOME`) for storing signing keys.
const KEY_DIR: &str = ".backbeatin";
const PRIVATE_KEY_FILE: &str = "signing.key";
const PUBLIC_KEY_FILE: &str = "signing.pub";

// ---------------------------------------------------------------------------
// Signer
// ---------------------------------------------------------------------------

/// Manages an Ed25519 signing keypair and produces signatures.
pub struct Signer {
    keypair: ed25519_dalek::SigningKey,
}

impl Signer {
    /// Load or generate a signing keypair from `~/.backbeatin/`.
    ///
    /// If the key files do not exist they are created with restricted
    /// permissions (Unix: 0o600).
    pub fn auto_load_or_generate() -> anyhow::Result<Self> {
        let key_dir = home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
            .join(KEY_DIR);

        Self::load_or_generate(&key_dir)
    }

    /// Load or generate a signing keypair from `key_dir`.
    pub fn load_or_generate(key_dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(key_dir)
            .with_context(|| format!("Failed to create key directory {:?}", key_dir))?;

        let priv_path = key_dir.join(PRIVATE_KEY_FILE);
        let pub_path = key_dir.join(PUBLIC_KEY_FILE);

        if priv_path.exists() {
            Self::load(&priv_path)
        } else {
            let signer = Self::generate();
            signer.save(&priv_path, &pub_path)?;
            tracing::warn!(
                "Generated new Ed25519 signing key at {:?}. The private key is stored \
                 unencrypted on disk (permissions 0o600). Keep this directory secure.",
                priv_path,
            );
            Ok(signer)
        }
    }

    /// Generate a new random signing keypair.
    pub fn generate() -> Self {
        let mut rng = rand::rngs::OsRng;
        let keypair = ed25519_dalek::SigningKey::generate(&mut rng);
        Self { keypair }
    }

    /// Load an existing keypair from `priv_path`.
    fn load(priv_path: &Path) -> anyhow::Result<Self> {
        let priv_bytes = fs::read(priv_path)
            .with_context(|| format!("Failed to read private key {:?}", priv_path))?;
        let keypair = ed25519_dalek::SigningKey::from_keypair_bytes(
            &priv_bytes.try_into().map_err(|_| {
                anyhow::anyhow!("Invalid private key file (expected 64 bytes)")
            })?,
        )?;

        Ok(Self { keypair })
    }

    /// Persist the keypair to disk.
    ///
    /// The private key file is created with 0o600 permissions on Unix.
    fn save(&self, priv_path: &Path, pub_path: &Path) -> anyhow::Result<()> {
        // Write private key.
        let priv_bytes = self.keypair.to_keypair_bytes();
        fs::write(priv_path, priv_bytes)
            .with_context(|| format!("Failed to write private key {:?}", priv_path))?;

        // Restrict permissions (best-effort, Unix only).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = fs::set_permissions(priv_path, fs::Permissions::from_mode(0o600)) {
                tracing::warn!("Failed to set private key permissions: {}", e);
            }
        }

        let pub_bytes = self.keypair.verifying_key().to_bytes();
        fs::write(pub_path, pub_bytes)
            .with_context(|| format!("Failed to write public key {:?}", pub_path))?;

        Ok(())
    }

    /// Sign `data` and return the hex-encoded signature (128 hex chars).
    pub fn sign(&self, data: &[u8]) -> String {
        use ed25519_dalek::Signer as _;
        let signature = self.keypair.sign(data);
        hex::encode(signature.to_bytes())
    }

    /// Return the hex-encoded public key.
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.keypair.verifying_key().to_bytes())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

#[cfg(not(unix))]
fn home_dir() -> Option<PathBuf> {
    std::env::var("USERPROFILE").ok().map(PathBuf::from)
}

/// Canonical serialization of verification-run data for signing.
///
/// This produces a deterministic JSON string so the same run always results
/// in the same signed bytes (assuming the field values are identical).
#[allow(clippy::too_many_arguments)]
pub fn run_signing_message(
    run_id: i64,
    repo_name: &str,
    snapshot_id: &str,
    status: &str,
    files_count: u64,
    bytes_restored: u64,
    message: &str,
    manifest_hash: &str,
    started_at: i64,
    completed_at: i64,
) -> anyhow::Result<Vec<u8>> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct RunData<'a> {
        run_id: i64,
        repo_name: &'a str,
        snapshot_id: &'a str,
        status: &'a str,
        files_count: u64,
        bytes_restored: u64,
        message: &'a str,
        manifest_hash: &'a str,
        started_at: i64,
        completed_at: i64,
    }

    let data = RunData {
        run_id,
        repo_name,
        snapshot_id,
        status,
        files_count,
        bytes_restored,
        message,
        manifest_hash,
        started_at,
        completed_at,
    };

    serde_json::to_vec(&data).context("Failed to serialize run data for signing")
}

/// Compute the SHA-256 hash of a JSON-serialized manifest.
///
/// Returns a hex-encoded 64-character string.  If serialization fails
/// (shouldn't happen in practice), returns an empty string.
pub fn manifest_sha256(manifest: &crate::verify::Manifest) -> String {
    if let Ok(json) = serde_json::to_string(manifest) {
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        hex::encode(hasher.finalize())
    } else {
        String::new()
    }
}

/// Parse a hex-encoded Ed25519 public key (64 hex chars → 32 bytes).
pub fn public_key_from_hex(hex_str: &str) -> anyhow::Result<ed25519_dalek::VerifyingKey> {
    let bytes = hex::decode(hex_str)
        .context("Failed to decode hex public key")?;
    let len = bytes.len();
    let arr: [u8; 32] = bytes.try_into().map_err(|_| {
        anyhow::anyhow!("Invalid public key: expected 32 bytes, got {}", len)
    })?;
    Ok(ed25519_dalek::VerifyingKey::from_bytes(&arr)?)
}

/// Verify an Ed25519 signature against a public key and the original data.
///
/// Returns `Ok(true)` if the signature is valid, `Ok(false)` if it is
/// structurally valid but does not match, or `Err` on decode / key errors.
pub fn verify_signature(
    data: &[u8],
    signature_hex: &str,
    public_key: &ed25519_dalek::VerifyingKey,
) -> anyhow::Result<bool> {
    use ed25519_dalek::Verifier;

    let sig_bytes = hex::decode(signature_hex)
        .context("Failed to decode hex signature")?;
    let sig_arr: [u8; 64] = sig_bytes.try_into().map_err(|_| {
        anyhow::anyhow!("Invalid signature: expected 64 bytes")
    })?;
    let signature = ed25519_dalek::Signature::from_bytes(&sig_arr);

    Ok(public_key.verify(data, &signature).is_ok())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let signer = Signer::generate();
        let data = b"hello world";
        let sig = signer.sign(data);

        assert_eq!(sig.len(), 128); // 64 bytes → 128 hex chars

        let pk_hex = signer.public_key_hex();
        let pk = public_key_from_hex(&pk_hex).expect("public key should parse");

        let valid = verify_signature(data, &sig, &pk).expect("verify should not error");
        assert!(valid);

        // Tampered data should fail.
        let valid = verify_signature(b"tampered", &sig, &pk).expect("verify should not error");
        assert!(!valid);
    }

    #[test]
    fn test_run_signing_message_deterministic() {
        let msg1 = run_signing_message(1, "repo", "snap", "pass", 100, 5000, "ok", "abc123", 1000, 1001)
            .expect("should serialize");
        let msg2 = run_signing_message(1, "repo", "snap", "pass", 100, 5000, "ok", "abc123", 1000, 1001)
            .expect("should serialize");
        assert_eq!(msg1, msg2);
    }

    #[test]
    fn test_invalid_key_hex() {
        let err = public_key_from_hex("too-short").unwrap_err();
        assert!(err.to_string().contains("32 bytes") || err.to_string().contains("decode"));
    }
}
