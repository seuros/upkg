use super::*;

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

    Mock::given(method("GET"))
        .and(path("/formula/ripgrep.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(formula_json))
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(format!("{}/formula", mock_server.uri()))
        .with_cask_base_url(format!("{}/cask", mock_server.uri()));
    let installer = new_test_installer(api_client, &tmp);

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

    Mock::given(method("GET"))
        .and(path("/formula/ghostty.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/cask/ghostty.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(cask_json))
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(format!("{}/formula", mock_server.uri()))
        .with_cask_base_url(format!("{}/cask", mock_server.uri()));
    let installer = new_test_installer(api_client, &tmp);

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
async fn auto_targets_report_original_missing_formula_when_cask_is_missing() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    Mock::given(method("GET"))
        .and(path("/formula/ghostty.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/cask/ghostty.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(format!("{}/formula", mock_server.uri()))
        .with_cask_base_url(format!("{}/cask", mock_server.uri()));
    let installer = new_test_installer(api_client, &tmp);

    let err = installer
        .resolve_auto_install_targets(&[("ghostty".to_string(), "ghostty".to_string())])
        .await
        .unwrap_err();

    assert!(matches!(err, Error::MissingFormula { name } if name == "ghostty"));
}

#[tokio::test]
async fn auto_targets_do_not_probe_tap_formulae_as_casks() {
    let tmp = TempDir::new().unwrap();
    let api_client = ApiClient::with_base_url("http://127.0.0.1:1".to_string());
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
