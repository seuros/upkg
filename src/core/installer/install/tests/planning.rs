use super::*;

#[tokio::test]
async fn plan_falls_back_to_source_when_no_bottle() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let formula_json = r#"{
            "name": "nobottle",
            "versions": { "stable": "1.0.0" },
            "dependencies": [],
            "build_dependencies": ["pkgconf"],
            "urls": {
                "stable": {
                    "url": "https://example.com/nobottle-1.0.0.tar.gz",
                    "checksum": "abc123"
                }
            },
            "ruby_source_path": "Formula/n/nobottle.rb",
            "bottle": { "stable": { "files": {} } }
        }"#;

    Mock::given(method("GET"))
        .and(path("/nobottle.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(formula_json))
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root.clone();
    let prefix = ctx.prefix.clone();
    let installer = ctx.installer;

    let plan = installer.plan(&["nobottle".to_string()]).await.unwrap();

    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].formula.name, "nobottle");
    assert!(matches!(
        plan.items[0].method,
        crate::types::InstallMethod::Source(_)
    ));

    if let crate::types::InstallMethod::Source(ref bp) = plan.items[0].method {
        assert_eq!(bp.source_url, "https://example.com/nobottle-1.0.0.tar.gz");
        assert_eq!(bp.formula_name, "nobottle");
        assert_eq!(bp.build_dependencies, vec!["pkgconf"]);
    }
}

#[tokio::test]
async fn plan_prefers_bottle_over_source() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let tag = get_test_bottle_tag();
    let formula_json = format!(
        r#"{{
                "name": "hasboth",
                "versions": {{ "stable": "2.0.0" }},
                "dependencies": [],
                "urls": {{
                    "stable": {{
                        "url": "https://example.com/hasboth-2.0.0.tar.gz",
                        "checksum": "def456"
                    }}
                }},
                "ruby_source_path": "Formula/h/hasboth.rb",
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "https://example.com/hasboth.bottle.tar.gz",
                                "sha256": "aabbccdd"
                            }}
                        }}
                    }}
                }}
            }}"#,
        tag
    );

    Mock::given(method("GET"))
        .and(path("/hasboth.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root.clone();
    let prefix = ctx.prefix.clone();
    let installer = ctx.installer;

    let plan = installer.plan(&["hasboth".to_string()]).await.unwrap();

    assert_eq!(plan.items.len(), 1);
    assert!(matches!(
        plan.items[0].method,
        crate::types::InstallMethod::Bottle(_)
    ));
}

#[tokio::test]
async fn plan_skips_already_installed_same_version() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();
    let tag = get_test_bottle_tag();
    let bottle = create_bottle_tarball("alreadythere");
    let bottle_sha = crate::core::checksum::sha256_hex_bytes(&bottle);

    let formula_json = format!(
        r#"{{
                "name": "alreadythere",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/alreadythere.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        tag,
        mock_server.uri(),
        bottle_sha
    );

    Mock::given(method("GET"))
        .and(path("/alreadythere.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/bottles/alreadythere.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle))
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root.clone();
    let prefix = ctx.prefix.clone();
    let mut installer = ctx.installer;

    installer
        .install(&["alreadythere".to_string()], true)
        .await
        .unwrap();

    let plan = installer.plan(&["alreadythere".to_string()]).await.unwrap();
    assert!(plan.items.is_empty());
}

#[tokio::test]
async fn plan_errors_when_no_bottle_and_no_source() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let formula_json = r#"{
            "name": "nothing",
            "versions": { "stable": "1.0.0" },
            "dependencies": [],
            "bottle": { "stable": { "files": {} } }
        }"#;

    Mock::given(method("GET"))
        .and(path("/nothing.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(formula_json))
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root.clone();
    let prefix = ctx.prefix.clone();
    let installer = ctx.installer;

    let result = installer.plan(&["nothing".to_string()]).await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::types::Error::MissingFormula { .. }
    ));
}
