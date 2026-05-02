use super::*;

#[tokio::test]
async fn install_completes_successfully() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let bottle = create_bottle_tarball("testpkg");
    let bottle_sha = sha256_hex(&bottle);

    let tag = get_test_bottle_tag();
    let formula_json = format!(
        r#"{{
                "name": "testpkg",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/testpkg-1.0.0.{}.bottle.tar.gz",
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
        .and(path("/testpkg.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/bottles/testpkg-1.0.0.{}.bottle.tar.gz",
            tag
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
        .mount(&mock_server)
        .await;

    let root = tmp.path().join("upkg");
    let prefix = tmp.path().join("homebrew");

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
    let store = Store::new(&root).unwrap();
    let cellar = Cellar::new(&root).unwrap();
    let linker = Linker::new(&prefix).unwrap();

    let mut installer = Installer::new(
        api_client,
        blob_cache,
        store,
        cellar,
        linker,
        prefix.clone(),
    );

    installer
        .install(&["testpkg".to_string()], true)
        .await
        .unwrap();

    assert!(root.join("Cellar/testpkg/1.0.0").exists());

    assert!(prefix.join("bin/testpkg").exists());

    let installed = installer.get_installed("testpkg");
    assert!(installed.is_some());
    assert_eq!(installed.unwrap().version, "1.0.0");
}

#[tokio::test]
async fn uninstall_cleans_everything() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let bottle = create_bottle_tarball("uninstallme");
    let bottle_sha = sha256_hex(&bottle);

    let tag = get_test_bottle_tag();
    let formula_json = format!(
        r#"{{
                "name": "uninstallme",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/uninstallme-1.0.0.{}.bottle.tar.gz",
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
        .and(path("/uninstallme.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/bottles/uninstallme-1.0.0.{}.bottle.tar.gz",
            tag
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
        .mount(&mock_server)
        .await;

    let root = tmp.path().join("upkg");
    let prefix = tmp.path().join("homebrew");

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
    let store = Store::new(&root).unwrap();
    let cellar = Cellar::new(&root).unwrap();
    let linker = Linker::new(&prefix).unwrap();

    let mut installer = Installer::new(
        api_client,
        blob_cache,
        store,
        cellar,
        linker,
        prefix.clone(),
    );

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
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let bottle = create_bottle_tarball("gctest");
    let bottle_sha = sha256_hex(&bottle);

    let tag = get_test_bottle_tag();
    let formula_json = format!(
        r#"{{
                "name": "gctest",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/gctest-1.0.0.{}.bottle.tar.gz",
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
        .and(path("/gctest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/bottles/gctest-1.0.0.{}.bottle.tar.gz", tag)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
        .mount(&mock_server)
        .await;

    let root = tmp.path().join("upkg");
    let prefix = tmp.path().join("homebrew");

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
    let store = Store::new(&root).unwrap();
    let cellar = Cellar::new(&root).unwrap();
    let linker = Linker::new(&prefix).unwrap();

    let mut installer = Installer::new(
        api_client,
        blob_cache,
        store,
        cellar,
        linker,
        prefix.clone(),
    );

    installer
        .install(&["gctest".to_string()], true)
        .await
        .unwrap();

    assert!(root.join("store").join(&bottle_sha).exists());

    installer.uninstall("gctest").unwrap();

    assert!(root.join("store").join(&bottle_sha).exists());

    let removed = installer.gc().unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0], bottle_sha);

    assert!(!root.join("store").join(&bottle_sha).exists());
    assert!(installer.gc().unwrap().is_empty());
}

#[tokio::test]
async fn gc_does_not_remove_referenced_store_entries() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let bottle = create_bottle_tarball("keepme");
    let bottle_sha = sha256_hex(&bottle);

    let tag = get_test_bottle_tag();
    let formula_json = format!(
        r#"{{
                "name": "keepme",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/keepme-1.0.0.{}.bottle.tar.gz",
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
        .and(path("/keepme.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/bottles/keepme-1.0.0.{}.bottle.tar.gz", tag)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
        .mount(&mock_server)
        .await;

    let root = tmp.path().join("upkg");
    let prefix = tmp.path().join("homebrew");

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
    let store = Store::new(&root).unwrap();
    let cellar = Cellar::new(&root).unwrap();
    let linker = Linker::new(&prefix).unwrap();

    let mut installer = Installer::new(
        api_client,
        blob_cache,
        store,
        cellar,
        linker,
        prefix.clone(),
    );

    installer
        .install(&["keepme".to_string()], true)
        .await
        .unwrap();

    assert!(root.join("store").join(&bottle_sha).exists());

    let removed = installer.gc().unwrap();
    assert!(removed.is_empty());

    assert!(root.join("store").join(&bottle_sha).exists());
}
