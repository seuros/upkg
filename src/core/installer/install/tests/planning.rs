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

    mount_text(&mock_server, "/nobottle.json", formula_json).await;

    let installer = server_installer(&mock_server, &tmp);

    let plan = installer.plan(&["nobottle".to_string()]).await.unwrap();

    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].formula.name, "nobottle");
    assert_matches!(plan.items[0].method, crate::types::InstallMethod::Source(_));

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

    mount_text(&mock_server, "/hasboth.json", formula_json).await;

    let installer = server_installer(&mock_server, &tmp);

    let plan = installer.plan(&["hasboth".to_string()]).await.unwrap();

    assert_eq!(plan.items.len(), 1);
    assert_matches!(plan.items[0].method, crate::types::InstallMethod::Bottle(_));
}

#[tokio::test]
async fn plan_skips_already_installed_same_version() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();
    mount_bottled(&mock_server, "alreadythere", "1.0.0", &[]).await;

    let mut installer = server_installer(&mock_server, &tmp);

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

    mount_text(&mock_server, "/nothing.json", formula_json).await;

    let installer = server_installer(&mock_server, &tmp);

    let result = installer.plan(&["nothing".to_string()]).await;
    assert!(result.is_err());
    assert_matches!(
        result.unwrap_err(),
        crate::types::Error::UnsupportedBottle { name } if name == "nothing"
    );
}

#[rstest::rstest]
#[case(true, false)]
#[case(false, false)]
#[case(false, true)]
#[tokio::test]
async fn dependency_with_only_newer_bottles_is_not_poured(
    #[case] has_source: bool,
    #[case] installed_dependency: bool,
) {
    let current = get_test_bottle_tag();
    let newer_tag = if current.starts_with("arm64_") {
        "arm64_tahoe"
    } else {
        "tahoe"
    };
    // There is no known newer bottle to test on the newest macOS.
    if current == newer_tag {
        return;
    }

    let server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();
    if installed_dependency {
        let previous_server = MockServer::start().await;
        mount_bottled(&previous_server, "freetype", "1.0.0", &[]).await;
        let mut installer = server_installer(&previous_server, &tmp);
        installer
            .install(&["freetype".to_string()], true)
            .await
            .unwrap();
    }
    mount_bottled(&server, "vips", "1.0.0", &["freetype"]).await;
    let mut dependency = serde_json::json!({
        "name": "freetype",
        "versions": { "stable": "1.0.0" },
        "dependencies": [],
        "bottle": { "stable": { "files": {
            newer_tag: {
                "url": "https://example.com/freetype.bottle.tar.gz",
                "sha256": "abc123"
            }
        } } }
    });
    if has_source {
        dependency["urls"] = serde_json::json!({
            "stable": { "url": "https://example.com/freetype-1.0.0.tar.gz" }
        });
    }
    mount_json(&server, "/freetype.json", dependency).await;

    let installer = server_installer(&server, &tmp);
    let result = installer.plan(&["vips".to_string()]).await;
    if installed_dependency {
        let plan = result.unwrap();
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].install_name, "vips");
    } else if has_source {
        let plan = result.unwrap();
        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.items[0].install_name, "freetype");
        assert_matches!(plan.items[0].method, crate::types::InstallMethod::Source(_));
        assert_eq!(plan.items[1].install_name, "vips");
        assert_matches!(plan.items[1].method, crate::types::InstallMethod::Bottle(_));
    } else {
        assert_matches!(
            result,
            Err(Error::UnsupportedBottle { name }) if name == "freetype"
        );
    }
    assert!(installer.get_installed("vips").is_none());
    assert_eq!(
        installer.get_installed("freetype").is_some(),
        installed_dependency
    );
}
