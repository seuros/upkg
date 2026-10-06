use super::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn detects_archive_urls() {
    assert!(looks_like_archive("https://example.com/foo.tar.gz"));
    assert!(looks_like_archive("https://example.com/foo.zip?download=1"));
    assert!(!looks_like_archive("https://example.com/safehouse.sh"));
}

#[test]
fn extracts_source_filename_from_url() {
    assert_eq!(
        source_filename("https://example.com/releases/download/v1/safehouse.sh").unwrap(),
        "safehouse.sh"
    );
}

#[tokio::test]
async fn stages_single_file_sources_without_extracting() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/safehouse.sh"))
        .respond_with(ResponseTemplate::new(200).set_body_string("#!/bin/sh\n"))
        .mount(&server)
        .await;

    let work_dir = tempfile::tempdir().unwrap();
    let source_root = download_and_extract_source(
        &format!("{}/safehouse.sh", server.uri()),
        None,
        work_dir.path(),
    )
    .await
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(source_root.join("safehouse.sh")).unwrap(),
        "#!/bin/sh\n"
    );
}
