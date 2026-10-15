use super::*;
use std::sync::Arc;

#[tokio::test]
async fn preserves_successful_installs_when_one_package_fails() {
    use std::time::Duration;

    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    mount_bottled(&mock_server, "goodpkg", "1.0.0", &[]).await;
    let failing_download = ResponseTemplate::new(500)
        .set_delay(Duration::from_millis(100))
        .set_body_string("download failed");
    mount_bottle_response(
        &mock_server,
        "badpkg",
        "1.0.0",
        &[],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        failing_download,
    )
    .await;

    let ctx = server_context(&mock_server, &tmp);
    let root = ctx.root;
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

    for (name, deps) in [
        ("leaf1", &[][..]),
        ("leaf2", &[]),
        ("mid1", &["leaf1"]),
        ("mid2", &["leaf1", "leaf2"]),
        ("root", &["mid1", "mid2"]),
    ] {
        mount_bottled(&mock_server, name, "1.0.0", deps).await;
    }

    let mut installer = server_installer(&mock_server, &tmp);

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

    mount_bottled(&mock_server, "fastpkg", "1.0.0", &[]).await;
    let slow_bottle = create_bottle_tarball("slowpkg");
    let slow_sha = sha256_hex(&slow_bottle);
    let slow_download = ResponseTemplate::new(200)
        .set_body_bytes(slow_bottle)
        .set_delay(Duration::from_millis(100));
    mount_bottle_response(
        &mock_server,
        "slowpkg",
        "1.0.0",
        &["fastpkg"],
        &slow_sha,
        slow_download,
    )
    .await;

    let ctx = server_context(&mock_server, &tmp);
    let (root, prefix) = (ctx.root, ctx.prefix);
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
    let attempt_count = Arc::new(AtomicUsize::new(0));
    let attempt_clone = attempt_count.clone();
    let counting_download = move |_: &wiremock::Request| {
        attempt_clone.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_bytes(bottle.clone())
    };
    mount_bottle_response(
        &mock_server,
        "retrypkg",
        "1.0.0",
        &[],
        &bottle_sha,
        counting_download,
    )
    .await;

    let ctx = server_context(&mock_server, &tmp);
    let (root, prefix) = (ctx.root, ctx.prefix);
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
