use super::*;
use crate::core::extraction::test_support::{gzip, tar_gz};
use rstest::rstest;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use tar::Builder;
use tempfile::TempDir;

type Extract = fn(&Path, &Path) -> Result<(), Error>;

/// Fresh temp dir with an empty `extracted/` destination inside it.
fn dest_dir() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("extracted");
    fs::create_dir(&dest).unwrap();
    (tmp, dest)
}

/// Writes `archive` as `<tmp>/<file_name>` and runs `extract` into `<tmp>/extracted`.
fn unpack(
    archive: &[u8],
    file_name: &str,
    extract: Extract,
) -> (TempDir, PathBuf, Result<(), Error>) {
    let (tmp, dest) = dest_dir();
    let archive_path = tmp.path().join(file_name);
    fs::write(&archive_path, archive).unwrap();
    let result = extract(&archive_path, &dest);
    (tmp, dest, result)
}

fn create_tarball_with_symlink(name: &str, target: &str) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());

    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_path(name).unwrap();
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();

    builder.append_link(&mut header, name, target).unwrap();

    gzip(&builder.into_inner().unwrap())
}

fn create_test_zip(entries: Vec<(&str, &[u8])>) -> Vec<u8> {
    use zip::write::SimpleFileOptions;

    let cursor = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(cursor);

    for (path, content) in entries {
        zip.start_file(path, SimpleFileOptions::default()).unwrap();
        zip.write_all(content).unwrap();
    }

    zip.finish().unwrap().into_inner()
}

#[test]
fn extracts_file_with_content() {
    let tarball = tar_gz(&[("hello.txt", b"Hello, World!", 0o644)]);

    let (_tmp, dest, result) = unpack(&tarball, "test.tar.gz", extract_tarball);
    result.unwrap();

    let content = fs::read_to_string(dest.join("hello.txt")).unwrap();
    assert_eq!(content, "Hello, World!");
}

#[test]
fn extracts_zip_file_with_content() {
    let zip_data = create_test_zip(vec![("op", b"#!/bin/sh\necho op")]);

    let (_tmp, dest, result) = unpack(&zip_data, "test.zip", extract_archive);
    result.unwrap();

    let content = fs::read_to_string(dest.join("op")).unwrap();
    assert_eq!(content, "#!/bin/sh\necho op");
}

#[test]
fn preserves_executable_bit() {
    let tarball = tar_gz(&[("script.sh", b"#!/bin/sh\necho hi", 0o755)]);

    let (_tmp, dest, result) = unpack(&tarball, "test.tar.gz", extract_tarball);
    result.unwrap();

    let metadata = fs::metadata(dest.join("script.sh")).unwrap();
    let mode = metadata.permissions().mode();
    assert!(
        mode & 0o111 != 0,
        "executable bit not preserved: {:o}",
        mode
    );
}

#[test]
fn preserves_symlink() {
    let tarball = create_tarball_with_symlink("link", "target.txt");

    let (_tmp, dest, result) = unpack(&tarball, "test.tar.gz", extract_tarball);
    result.unwrap();

    let link_path = dest.join("link");
    assert!(
        link_path
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(&link_path).unwrap(),
        PathBuf::from("target.txt")
    );
}

fn create_malicious_tarball(path: &[u8]) -> Vec<u8> {
    let mut tar_data = vec![0u8; 512 + 512]; // header + one block of data

    let path_len = path.len().min(100);
    tar_data[..path_len].copy_from_slice(&path[..path_len]);

    tar_data[100..108].copy_from_slice(b"0000644\0");

    tar_data[108..116].copy_from_slice(b"0000000\0");

    tar_data[116..124].copy_from_slice(b"0000000\0");

    tar_data[124..136].copy_from_slice(b"00000000004\0");

    tar_data[136..148].copy_from_slice(b"00000000000\0");

    tar_data[156] = b'0';

    tar_data[148..156].copy_from_slice(b"        ");

    let checksum: u32 = tar_data[..512].iter().map(|&b| b as u32).sum();
    let checksum_str = format!("{:06o}\0 ", checksum);
    tar_data[148..156].copy_from_slice(checksum_str.as_bytes());

    tar_data[512..516].copy_from_slice(b"evil");

    gzip(&tar_data)
}

#[rstest]
#[case::path_traversal(b"../evil.txt", "evil.tar.gz", "path traversal")]
#[case::absolute_path(b"/etc/passwd", "absolute.tar.gz", "absolute path")]
fn rejects_unsafe_entry(
    #[case] entry: &[u8],
    #[case] file_name: &str,
    #[case] expected_error: &str,
) {
    let tarball = create_malicious_tarball(entry);

    let (_tmp, _dest, result) = unpack(&tarball, file_name, extract_tarball);

    let err = result.unwrap_err();
    assert!(err.to_string().contains(expected_error), "{err}");
}

#[rstest]
#[case("/foo/./bar/./baz", "/foo/bar/baz", "removes dot components")]
#[case("/foo/bar/../baz", "/foo/baz", "resolves parent dirs")]
#[case("/foo/bar/qux/../../baz", "/foo/baz", "handles multiple parent dirs")]
#[case(
    "../foo/bar",
    "../foo/bar",
    "preserves leading parent dirs in relative paths"
)]
#[case(
    "foo/./bar/../baz/./qux",
    "foo/baz/qux",
    "handles complex relative path"
)]
#[case("/foo/../../etc/passwd", "/etc/passwd", "cannot escape root")]
#[case(
    "/../../../../etc/passwd",
    "/etc/passwd",
    "multiple attempts to escape root"
)]
#[case("/../..", "/", "root with only parent dirs")]
fn path_normalization(#[case] input: &str, #[case] expected: &str, #[case] _description: &str) {
    let normalized = normalize_path(&PathBuf::from(input));
    assert_eq!(normalized, PathBuf::from(expected));
}

#[rstest]
#[case::rejects_normalized_escape("foo/../../etc/passwd", false)]
#[case::accepts_safe_nested_paths("foo/bar/baz.txt", true)]
#[case::accepts_paths_with_dots_in_names("foo/file.tar.gz", true)]
fn validate_path_cases(#[case] path: &str, #[case] accepted: bool) {
    let (_tmp, dest) = dest_dir();

    let result = validate_path(&PathBuf::from(path), &dest);
    assert_eq!(result.is_ok(), accepted, "{path}: {result:?}");
}
