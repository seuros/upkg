use super::*;
use crate::core::storage::receipt::read_receipt;
use std::path::Path;

async fn mount_testpkg(server: &MockServer, bottle: &[u8], bottle_status: u16) {
    let tag = get_test_bottle_tag();
    let formula_json = format!(
        r#"{{
                "name": "testpkg",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{tag}": {{
                                "url": "{uri}/bottles/testpkg-1.0.0.{tag}.bottle.tar.gz",
                                "sha256": "{sha}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        uri = server.uri(),
        sha = sha256_hex(bottle),
    );

    Mock::given(method("GET"))
        .and(path("/testpkg.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/bottles/testpkg-1.0.0.{tag}.bottle.tar.gz")))
        .respond_with(ResponseTemplate::new(bottle_status).set_body_bytes(bottle.to_vec()))
        .mount(server)
        .await;
}

fn keg_entries(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.join("Cellar/testpkg"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Installs testpkg, then overwrites its binary. Returns the pristine content.
async fn install_and_break(tmp: &TempDir) -> (PathBuf, PathBuf, String) {
    let server = MockServer::start().await;
    mount_testpkg(&server, &create_bottle_tarball("testpkg"), 200).await;
    let ctx = new_test_context(ApiClient::with_base_url(server.uri()), tmp);
    let mut installer = ctx.installer;
    installer
        .install(&["testpkg".to_string()], true)
        .await
        .unwrap();

    let bin = ctx.root.join("Cellar/testpkg/1.0.0/bin/testpkg");
    let pristine = fs::read_to_string(&bin).unwrap();
    fs::remove_file(&bin).unwrap();
    fs::write(&bin, "broken").unwrap();
    (ctx.root, ctx.prefix, pristine)
}

fn store_entry(root: &Path) -> PathBuf {
    let receipt = read_receipt(&root.join("Cellar/testpkg/1.0.0")).unwrap();
    root.join("store").join(receipt.store_key)
}

#[tokio::test]
async fn reinstall_rebuilds_the_keg_from_the_store_without_network() {
    let tmp = TempDir::new().unwrap();
    let (root, prefix, pristine) = install_and_break(&tmp).await;

    // Nothing mounted: any request would fail the reinstall.
    let server = MockServer::start().await;
    let mut installer = new_test_installer(ApiClient::with_base_url(server.uri()), &tmp);

    let plan = installer
        .plan_reinstall(&["testpkg".to_string()], false)
        .await
        .unwrap();
    assert_eq!(plan.from_store.len(), 1);
    assert!(plan.fetch.items.is_empty());

    let result = installer
        .reinstall_with_progress(plan, true, None)
        .await
        .unwrap();

    assert_eq!(result.installed, 1);
    let bin = root.join("Cellar/testpkg/1.0.0/bin/testpkg");
    assert_eq!(fs::read_to_string(&bin).unwrap(), pristine);
    assert!(prefix.join("bin/testpkg").exists());
    assert!(read_receipt(&root.join("Cellar/testpkg/1.0.0")).is_some());
    assert_eq!(keg_entries(&root), vec!["1.0.0"]);
}

#[tokio::test]
async fn failed_store_reinstall_restores_the_previous_keg() {
    let tmp = TempDir::new().unwrap();
    let (root, prefix, _) = install_and_break(&tmp).await;

    // Entry still present, but its bottle content is gone.
    let entry = store_entry(&root);
    fs::remove_dir_all(&entry).unwrap();
    fs::create_dir_all(&entry).unwrap();

    let server = MockServer::start().await;
    let mut installer = new_test_installer(ApiClient::with_base_url(server.uri()), &tmp);
    let plan = installer
        .plan_reinstall(&["testpkg".to_string()], false)
        .await
        .unwrap();
    assert_eq!(plan.from_store.len(), 1);

    assert!(
        installer
            .reinstall_with_progress(plan, true, None)
            .await
            .is_err()
    );

    let bin = root.join("Cellar/testpkg/1.0.0/bin/testpkg");
    assert_eq!(fs::read_to_string(&bin).unwrap(), "broken");
    assert!(prefix.join("bin/testpkg").exists());
    assert!(prefix.join("opt/testpkg").exists());
    assert_eq!(keg_entries(&root), vec!["1.0.0"]);
}

#[tokio::test]
async fn reinstall_fetches_when_the_store_entry_is_gone() {
    let tmp = TempDir::new().unwrap();
    let (root, prefix, pristine) = install_and_break(&tmp).await;
    fs::remove_dir_all(store_entry(&root)).unwrap();

    let server = MockServer::start().await;
    mount_testpkg(&server, &create_bottle_tarball("testpkg"), 200).await;
    let mut installer = new_test_installer(ApiClient::with_base_url(server.uri()), &tmp);

    let plan = installer
        .plan_reinstall(&["testpkg".to_string()], false)
        .await
        .unwrap();
    assert!(plan.from_store.is_empty());
    assert_eq!(plan.fetch.items.len(), 1);

    installer
        .reinstall_with_progress(plan, true, None)
        .await
        .unwrap();

    let bin = root.join("Cellar/testpkg/1.0.0/bin/testpkg");
    assert_eq!(fs::read_to_string(&bin).unwrap(), pristine);
    assert!(prefix.join("bin/testpkg").exists());
    assert_eq!(keg_entries(&root), vec!["1.0.0"]);
}

#[tokio::test]
async fn failed_fetch_reinstall_restores_the_previous_keg() {
    let tmp = TempDir::new().unwrap();
    let (root, prefix, _) = install_and_break(&tmp).await;
    fs::remove_dir_all(store_entry(&root)).unwrap();

    // A different bottle that cannot be downloaded.
    let server = MockServer::start().await;
    mount_testpkg(&server, b"not the same bottle", 500).await;
    let mut installer = new_test_installer(ApiClient::with_base_url(server.uri()), &tmp);

    let plan = installer
        .plan_reinstall(&["testpkg".to_string()], false)
        .await
        .unwrap();
    assert_eq!(plan.fetch.items.len(), 1);

    assert!(
        installer
            .reinstall_with_progress(plan, true, None)
            .await
            .is_err()
    );

    let bin = root.join("Cellar/testpkg/1.0.0/bin/testpkg");
    assert_eq!(fs::read_to_string(&bin).unwrap(), "broken");
    assert!(prefix.join("bin/testpkg").exists());
    assert!(prefix.join("opt/testpkg").exists());
    assert_eq!(keg_entries(&root), vec!["1.0.0"]);
    assert!(installer.is_installed("testpkg"));
}

#[tokio::test]
async fn reinstall_requires_an_installed_package() {
    let tmp = TempDir::new().unwrap();
    let server = MockServer::start().await;
    mount_testpkg(&server, &create_bottle_tarball("testpkg"), 200).await;
    let installer = new_test_installer(ApiClient::with_base_url(server.uri()), &tmp);

    let err = installer
        .plan_reinstall(&["testpkg".to_string()], false)
        .await
        .unwrap_err();

    assert_matches!(err, Error::NotInstalled { ref name } if name == "testpkg");
    assert!(!tmp.path().join("upkg/Cellar/testpkg").exists());
}
