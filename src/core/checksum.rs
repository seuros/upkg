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

#[cfg(any(target_os = "macos", test))]
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
mod tests {
    use super::*;

    #[test]
    fn skips_verification_when_none() {
        assert!(verify_sha256_bytes(b"anything", None).is_ok());
    }

    #[test]
    fn accepts_valid_checksum() {
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_sha256_bytes(b"hello", Some(expected)).is_ok());
    }

    #[test]
    fn accepts_uppercase_and_whitespace() {
        let expected = " 2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824 ";
        assert!(verify_sha256_bytes(b"hello", Some(expected)).is_ok());
    }

    #[test]
    fn rejects_invalid_length() {
        let err = verify_sha256_bytes(b"hello", Some("abc")).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument { .. }));
    }

    #[test]
    fn rejects_non_hex() {
        let bad = format!("{}{}", "a".repeat(63), "z");
        let err = verify_sha256_bytes(b"hello", Some(&bad)).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument { .. }));
    }

    #[test]
    fn rejects_mismatch() {
        let err = verify_sha256_bytes(b"hello", Some(&"0".repeat(64))).unwrap_err();
        assert!(matches!(err, Error::ChecksumMismatch { .. }));
    }
}
