use sha2::{Digest, Sha256};

#[cfg(target_os = "macos")]
use crate::types::Error;

#[cfg(target_os = "macos")]
pub fn verify_sha256_bytes(bytes: &[u8], expected_sha256: Option<&str>) -> Result<(), Error> {
    let Some(expected_sha256) = expected_sha256 else {
        return Ok(());
    };

    let expected = normalize_sha256(expected_sha256)?;
    let actual = sha256_hex_bytes(bytes);

    if actual != expected {
        return Err(Error::ChecksumMismatch { expected, actual });
    }

    Ok(())
}

#[cfg(target_os = "macos")]
pub fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    finalize_sha256_hex(hasher)
}

pub fn finalize_sha256_hex(hasher: Sha256) -> String {
    let digest = hasher.finalize();
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut rendered, "{byte:02x}");
    }
    rendered
}

#[cfg(target_os = "macos")]
fn normalize_sha256(input: &str) -> Result<String, Error> {
    let normalized = input.trim().to_lowercase();

    if normalized.len() != 64 {
        return Err(Error::InvalidArgument {
            message: format!(
                "invalid sha256 checksum: expected 64 hex chars, got {}",
                normalized.len()
            ),
        });
    }

    if !normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::InvalidArgument {
            message: "invalid sha256 checksum: must contain only hex characters".to_string(),
        });
    }

    Ok(normalized)
}

#[cfg(all(test, target_os = "macos"))]
#[path = "checksum/tests.rs"]
mod tests;
