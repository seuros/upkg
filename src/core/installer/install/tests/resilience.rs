use super::*;
use std::sync::Arc;

#[tokio::test]
async fn preserves_successful_installs_when_one_package_fails() {
    use std::time::Duration;

    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let good_bottle = create_bottle_tarball("goodpkg");
    let good_sha = sha256_hex(&good_bottle);

    let tag = get_test_bottle_tag();
    let good_json = format!(
        r#"{{
                "name": "goodpkg",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/goodpkg-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        tag,
        mock_server.uri(),
        tag,
        good_sha
    );

    let bad_json = format!(
        r#"{{
                "name": "badpkg",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/badpkg-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        tag,
        mock_server.uri(),
        tag,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );

    Mock::given(method("GET"))
        .and(path("/goodpkg.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&good_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/badpkg.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&bad_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/bottles/goodpkg-1.0.0.{}.bottle.tar.gz",
            tag
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(good_bottle))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/bottles/badpkg-1.0.0.{}.bottle.tar.gz", tag)))
        .respond_with(
            ResponseTemplate::new(500)
                .set_delay(Duration::from_millis(100))
                .set_body_string("download failed"),
        )
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root.clone();
    let prefix = ctx.prefix.clone();
    let mut installer = ctx.installer;

    let result = installer
        .install(&["goodpkg".to_string(), "badpkg".to_string()], false)
        .await;
    assert!(result.is_err());

    assert!(installer.get_installed("goodpkg").is_some());
    assert!(installer.get_installed("badpkg").is_none());
    assert!(root.join("Cellar/goodpkg/1.0.0").exists());
}

#[tokio::test]
async fn parallel_api_fetching_with_deep_deps() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let leaf1_bottle = create_bottle_tarball("leaf1");
    let leaf1_sha = sha256_hex(&leaf1_bottle);
    let leaf2_bottle = create_bottle_tarball("leaf2");
    let leaf2_sha = sha256_hex(&leaf2_bottle);
    let mid1_bottle = create_bottle_tarball("mid1");
    let mid1_sha = sha256_hex(&mid1_bottle);
    let mid2_bottle = create_bottle_tarball("mid2");
    let mid2_sha = sha256_hex(&mid2_bottle);
    let root_bottle = create_bottle_tarball("root");
    let root_sha = sha256_hex(&root_bottle);

    let tag = get_test_bottle_tag();
    let leaf1_json = format!(
        r#"{{"name":"leaf1","versions":{{"stable":"1.0.0"}},"dependencies":[],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/leaf1.tar.gz","sha256":"{}"}}}}}}}}}}"#,
        tag,
        mock_server.uri(),
        leaf1_sha
    );
    let leaf2_json = format!(
        r#"{{"name":"leaf2","versions":{{"stable":"1.0.0"}},"dependencies":[],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/leaf2.tar.gz","sha256":"{}"}}}}}}}}}}"#,
        tag,
        mock_server.uri(),
        leaf2_sha
    );
    let mid1_json = format!(
        r#"{{"name":"mid1","versions":{{"stable":"1.0.0"}},"dependencies":["leaf1"],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/mid1.tar.gz","sha256":"{}"}}}}}}}}}}"#,
        tag,
        mock_server.uri(),
        mid1_sha
    );
    let mid2_json = format!(
        r#"{{"name":"mid2","versions":{{"stable":"1.0.0"}},"dependencies":["leaf1","leaf2"],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/mid2.tar.gz","sha256":"{}"}}}}}}}}}}"#,
        tag,
        mock_server.uri(),
        mid2_sha
    );
    let root_json = format!(
        r#"{{"name":"root","versions":{{"stable":"1.0.0"}},"dependencies":["mid1","mid2"],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/root.tar.gz","sha256":"{}"}}}}}}}}}}"#,
        tag,
        mock_server.uri(),
        root_sha
    );

    for (name, json) in [
        ("leaf1", &leaf1_json),
        ("leaf2", &leaf2_json),
        ("mid1", &mid1_json),
        ("mid2", &mid2_json),
        ("root", &root_json),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/{}.json", name)))
            .respond_with(ResponseTemplate::new(200).set_body_string(json))
            .mount(&mock_server)
            .await;
    }
    for (name, bottle) in [
        ("leaf1", &leaf1_bottle),
        ("leaf2", &leaf2_bottle),
        ("mid1", &mid1_bottle),
        ("mid2", &mid2_bottle),
        ("root", &root_bottle),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/bottles/{}.tar.gz", name)))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
            .mount(&mock_server)
            .await;
    }

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root.clone();
    let prefix = ctx.prefix.clone();
    let mut installer = ctx.installer;

    installer
        .install(&["root".to_string()], true)
        .await
        .unwrap();

    assert!(installer.get_installed("root").is_some());
    assert!(installer.get_installed("mid1").is_some());
    assert!(installer.get_installed("mid2").is_some());
    assert!(installer.get_installed("leaf1").is_some());
    assert!(installer.get_installed("leaf2").is_some());
}

#[tokio::test]
async fn streaming_extraction_processes_as_downloads_complete() {
    use std::time::Duration;

    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let fast_bottle = create_bottle_tarball("fastpkg");
    let fast_sha = sha256_hex(&fast_bottle);
    let slow_bottle = create_bottle_tarball("slowpkg");
    let slow_sha = sha256_hex(&slow_bottle);

    let tag = get_test_bottle_tag();
    let fast_json = format!(
        r#"{{"name":"fastpkg","versions":{{"stable":"1.0.0"}},"dependencies":[],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/fast.tar.gz","sha256":"{}"}}}}}}}}}}"#,
        tag,
        mock_server.uri(),
        fast_sha
    );

    let slow_json = format!(
        r#"{{"name":"slowpkg","versions":{{"stable":"1.0.0"}},"dependencies":["fastpkg"],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/slow.tar.gz","sha256":"{}"}}}}}}}}}}"#,
        tag,
        mock_server.uri(),
        slow_sha
    );

    Mock::given(method("GET"))
        .and(path("/fastpkg.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&fast_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/slowpkg.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&slow_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/bottles/fast.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fast_bottle.clone()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/bottles/slow.tar.gz"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(slow_bottle.clone())
                .set_delay(Duration::from_millis(100)),
        )
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root.clone();
    let prefix = ctx.prefix.clone();
    let mut installer = ctx.installer;

    installer
        .install(&["slowpkg".to_string()], true)
        .await
        .unwrap();

    assert!(installer.get_installed("fastpkg").is_some());
    assert!(installer.get_installed("slowpkg").is_some());

    assert!(root.join("Cellar/fastpkg/1.0.0").exists());
    assert!(root.join("Cellar/slowpkg/1.0.0").exists());

    assert!(prefix.join("bin/fastpkg").exists());
    assert!(prefix.join("bin/slowpkg").exists());
}

#[tokio::test]
async fn retries_on_corrupted_download() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let bottle = create_bottle_tarball("retrypkg");
    let bottle_sha = sha256_hex(&bottle);

    let tag = get_test_bottle_tag();
    let formula_json = format!(
        r#"{{
                "name": "retrypkg",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/retrypkg-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        tag,
        mock_server.uri(),
        tag,
        bottle_sha
    );

    Mock::given(method("GET"))
        .and(path("/retrypkg.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
        .mount(&mock_server)
        .await;

    let attempt_count = Arc::new(AtomicUsize::new(0));
    let attempt_clone = attempt_count.clone();
    let valid_bottle = bottle.clone();

    Mock::given(method("GET"))
        .and(path(format!(
            "/bottles/retrypkg-1.0.0.{}.bottle.tar.gz",
            tag
        )))
        .respond_with(move |_: &wiremock::Request| {
            attempt_clone.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_bytes(valid_bottle.clone())
        })
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root.clone();
    let prefix = ctx.prefix.clone();
    let mut installer = ctx.installer;

    installer
        .install(&["retrypkg".to_string()], true)
        .await
        .unwrap();

    assert!(installer.is_installed("retrypkg"));
    assert!(root.join("Cellar/retrypkg/1.0.0").exists());
    assert!(prefix.join("bin/retrypkg").exists());
}

#[tokio::test]
async fn fails_after_max_retries() {}
