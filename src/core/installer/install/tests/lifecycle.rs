use super::*;

/// Serves bottled formula `name` 1.0.0 and returns a fresh context plus the
/// bottle checksum. The server must outlive the install.
async fn bottled_context(name: &str) -> (MockServer, TempDir, TestInstallContext, String) {
    let server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();
    let sha = mount_bottled(&server, name, "1.0.0", &[]).await;
    let ctx = server_context(&server, &tmp);
    (server, tmp, ctx, sha)
}

#[tokio::test]
async fn install_completes_successfully() {
    let (_server, _tmp, ctx, _) = bottled_context("testpkg").await;
    let mut installer = ctx.installer;

    installer
        .install(&["testpkg".to_string()], true)
        .await
        .unwrap();

    assert!(ctx.root.join("Cellar/testpkg/1.0.0").exists());

    assert!(ctx.prefix.join("bin/testpkg").exists());

    let installed = installer.get_installed("testpkg");
    assert!(installed.is_some());
    assert_eq!(installed.unwrap().version, "1.0.0");
}

#[tokio::test]
async fn uninstall_cleans_everything() {
    let (_server, _tmp, ctx, _) = bottled_context("uninstallme").await;
    let (root, prefix) = (ctx.root, ctx.prefix);
    let mut installer = ctx.installer;

    installer
        .install(&["uninstallme".to_string()], true)
        .await
        .unwrap();

    assert!(installer.is_installed("uninstallme"));
    assert!(root.join("Cellar/uninstallme/1.0.0").exists());
    assert!(prefix.join("bin/uninstallme").exists());

    installer.uninstall("uninstallme").unwrap();

    assert!(!installer.is_installed("uninstallme"));
    assert!(!root.join("Cellar/uninstallme/1.0.0").exists());
    assert!(!prefix.join("bin/uninstallme").exists());
}

#[tokio::test]
async fn gc_removes_unreferenced_store_entries() {
    let (_server, _tmp, ctx, bottle_sha) = bottled_context("gctest").await;
    let store_entry = ctx.root.join("store").join(&bottle_sha);
    let mut installer = ctx.installer;

    installer
        .install(&["gctest".to_string()], true)
        .await
        .unwrap();

    assert!(store_entry.exists());

    installer.uninstall("gctest").unwrap();

    assert!(store_entry.exists());

    let removed = installer.gc().unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0], bottle_sha);

    assert!(!store_entry.exists());
    assert!(installer.gc().unwrap().is_empty());
}

#[tokio::test]
async fn gc_does_not_remove_referenced_store_entries() {
    let (_server, _tmp, ctx, bottle_sha) = bottled_context("keepme").await;
    let store_entry = ctx.root.join("store").join(&bottle_sha);
    let mut installer = ctx.installer;

    installer
        .install(&["keepme".to_string()], true)
        .await
        .unwrap();

    assert!(store_entry.exists());

    let removed = installer.gc().unwrap();
    assert!(removed.is_empty());

    assert!(store_entry.exists());
}
