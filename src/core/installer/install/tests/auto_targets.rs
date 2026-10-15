use super::*;

/// Installer resolving formulae under `/formula` and casks under `/cask` on
/// `server`, with no local taps.
fn formula_cask_installer(server: &MockServer, tmp: &TempDir) -> Installer {
    let api_client = ApiClient::with_base_url(format!("{}/formula", server.uri()))
        .with_cask_base_url(format!("{}/cask", server.uri()))
        .with_tap_roots(Vec::new());
    new_test_installer(api_client, tmp)
}

#[tokio::test]
async fn auto_targets_keep_existing_formulas() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let formula_json = r#"{
            "name": "ripgrep",
            "versions": { "stable": "14.1.1" },
            "dependencies": [],
            "bottle": { "stable": { "files": {} } }
        }"#;
    mount_text(&mock_server, "/formula/ripgrep.json", formula_json).await;

    let installer = formula_cask_installer(&mock_server, &tmp);

    let targets = installer
        .resolve_auto_install_targets(&[("ripgrep".to_string(), "ripgrep".to_string())])
        .await
        .unwrap();

    assert_eq!(
        targets.formulas,
        vec![("ripgrep".to_string(), "ripgrep".to_string())]
    );
    assert!(targets.casks.is_empty());
}

#[tokio::test]
async fn auto_targets_fall_back_to_cask_when_formula_is_missing() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let cask_json = r#"{
            "token": "ghostty",
            "version": "1.3.1",
            "url": "https://example.com/ghostty.zip",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [{"app":["Ghostty.app"]}]
        }"#;
    mount_status(&mock_server, "/formula/ghostty.json", 404).await;
    mount_text(&mock_server, "/cask/ghostty.json", cask_json).await;

    let installer = formula_cask_installer(&mock_server, &tmp);

    let targets = installer
        .resolve_auto_install_targets(&[("ghostty".to_string(), "ghostty".to_string())])
        .await
        .unwrap();

    assert!(targets.formulas.is_empty());
    assert_eq!(
        targets.casks,
        vec![("ghostty".to_string(), "cask:ghostty".to_string())]
    );
}

#[tokio::test]
async fn auto_targets_resolve_old_cask_tokens() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();
    let cask_json = r#"{
        "token": "docker-desktop",
        "old_tokens": ["docker"],
        "version": "4.88.1,237512",
        "url": "https://example.com/Docker.dmg",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts": [{"app":["Docker.app"]}]
    }"#;
    mount_status(&mock_server, "/formula/docker.json", 404).await;
    mount_status(&mock_server, "/cask/docker.json", 404).await;
    mount_text(&mock_server, "/cask.json", format!("[{cask_json}]")).await;
    mount_text(&mock_server, "/cask/docker-desktop.json", cask_json).await;

    let installer = formula_cask_installer(&mock_server, &tmp);

    let targets = installer
        .resolve_auto_install_targets(&[("docker".to_string(), "docker".to_string())])
        .await
        .unwrap();

    assert!(targets.formulas.is_empty());
    assert_eq!(
        targets.casks,
        vec![("docker".to_string(), "cask:docker-desktop".to_string())]
    );
}

#[tokio::test]
async fn auto_targets_report_original_missing_formula_when_cask_is_missing() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();
    mount_status(&mock_server, "/formula/ghostty.json", 404).await;
    mount_status(&mock_server, "/cask/ghostty.json", 404).await;

    let installer = formula_cask_installer(&mock_server, &tmp);

    let err = installer
        .resolve_auto_install_targets(&[("ghostty".to_string(), "ghostty".to_string())])
        .await
        .unwrap_err();

    assert_matches!(err, Error::MissingFormula { name } if name == "ghostty");
}

#[tokio::test]
async fn auto_targets_do_not_probe_tap_formulae_as_casks() {
    let tmp = TempDir::new().unwrap();
    let api_client =
        ApiClient::with_base_url("http://127.0.0.1:1".to_string()).with_tap_roots(Vec::new());
    let installer = new_test_installer(api_client, &tmp);

    let targets = installer
        .resolve_auto_install_targets(&[(
            "hashicorp/tap/terraform".to_string(),
            "hashicorp/tap/terraform".to_string(),
        )])
        .await
        .unwrap();

    assert_eq!(
        targets.formulas,
        vec![(
            "hashicorp/tap/terraform".to_string(),
            "hashicorp/tap/terraform".to_string()
        )]
    );
    assert!(targets.casks.is_empty());
}
