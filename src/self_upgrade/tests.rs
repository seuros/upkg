use super::*;

#[test]
fn release_asset_name_uses_upkg_release_format() {
    let target = ReleaseTarget {
        triple: "aarch64-apple-darwin",
        archive: ArchiveKind::TarGz,
    };

    assert_eq!(
        release_asset_name("upkg-v1.2.3", target),
        "upkg-v1.2.3-aarch64-apple-darwin.tar.gz"
    );
}

#[test]
fn version_from_tag_handles_component_tags() {
    assert_eq!(version_from_tag("upkg-v1.2.3"), "1.2.3");
    assert_eq!(version_from_tag("v1.2.3"), "1.2.3");
    assert_eq!(version_from_tag("1.2.3"), "1.2.3");
}

#[test]
fn release_target_is_supported_for_current_platform_or_cleanly_missing() {
    if let Some(target) = release_target() {
        assert!(target.triple.contains(std::env::consts::ARCH));
    }
}

#[test]
fn checksum_for_asset_finds_matching_line() {
    let body = "abc  other.tar.gz\ndef  upkg-v1.2.3-aarch64-apple-darwin.tar.gz\n";
    assert_eq!(
        checksum_for_asset(body, "upkg-v1.2.3-aarch64-apple-darwin.tar.gz").unwrap(),
        "def"
    );
}
