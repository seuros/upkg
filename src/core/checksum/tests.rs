use super::*;
use std::assert_matches;

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
    assert_matches!(err, Error::InvalidArgument { .. });
}

#[test]
fn rejects_non_hex() {
    let bad = format!("{}{}", "a".repeat(63), "z");
    let err = verify_sha256_bytes(b"hello", Some(&bad)).unwrap_err();
    assert_matches!(err, Error::InvalidArgument { .. });
}

#[test]
fn rejects_mismatch() {
    let err = verify_sha256_bytes(b"hello", Some(&"0".repeat(64))).unwrap_err();
    assert_matches!(err, Error::ChecksumMismatch { .. });
}
