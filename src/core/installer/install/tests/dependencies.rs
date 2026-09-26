use super::*;

#[tokio::test]
async fn install_with_dependencies() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let dep_bottle = create_bottle_tarball("deplib");
    let dep_sha = sha256_hex(&dep_bottle);

    let main_bottle = create_bottle_tarball("mainpkg");
    let main_sha = sha256_hex(&main_bottle);

    let tag = get_test_bottle_tag();
    let dep_json = format!(
        r#"{{
                "name": "deplib",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/deplib-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        tag,
        mock_server.uri(),
        tag,
        dep_sha
    );

    let main_json = format!(
        r#"{{
                "name": "mainpkg",
                "versions": {{ "stable": "2.0.0" }},
                "dependencies": ["deplib"],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/mainpkg-2.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        tag,
        mock_server.uri(),
        tag,
        main_sha
    );

    Mock::given(method("GET"))
        .and(path("/deplib.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&dep_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/mainpkg.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&main_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/bottles/deplib-1.0.0.{}.bottle.tar.gz", tag)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(dep_bottle))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/bottles/mainpkg-2.0.0.{}.bottle.tar.gz",
            tag
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(main_bottle))
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let _root = ctx.root.clone();
    let _prefix = ctx.prefix.clone();
    let mut installer = ctx.installer;

    installer
        .install(&["mainpkg".to_string()], true)
        .await
        .unwrap();

    assert!(installer.get_installed("mainpkg").is_some());
    assert!(installer.get_installed("deplib").is_some());
}

#[tokio::test]
#[ignore = "flaky mock channel close for dependent core formula fetch"]
async fn plans_tapped_formula_with_core_dependency() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let dep_bottle = create_bottle_tarball("go");
    let dep_sha = sha256_hex(&dep_bottle);
    let tag = get_test_bottle_tag();
    let dep_json = format!(
        r#"{{
                "name": "go",
                "versions": {{ "stable": "1.24.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/go-1.24.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        tag,
        mock_server.uri(),
        tag,
        dep_sha
    );

    Mock::given(method("GET"))
        .and(path("/go.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&dep_json))
        .mount(&mock_server)
        .await;

    let tap_formula_rb = format!(
        r#"
class Terraform < Formula
  version "1.10.0"
  depends_on "go"
  bottle do
    root_url "{}/ghcr/hashicorp/tap"
    sha256 {}: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#,
        mock_server.uri(),
        tag
    );

    Mock::given(method("GET"))
        .and(path("/hashicorp/homebrew-tap/main/Formula/terraform.rb"))
        .respond_with(ResponseTemplate::new(200).set_body_string(tap_formula_rb))
        .mount(&mock_server)
        .await;

    let api_client =
        ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let _root = ctx.root.clone();
    let _prefix = ctx.prefix.clone();
    let installer = ctx.installer;
    let plan = installer
        .plan(&["hashicorp/tap/terraform".to_string()])
        .await
        .unwrap();

    let planned_names: Vec<String> = plan
        .items
        .iter()
        .map(|item| item.formula.name.clone())
        .collect();
    assert!(planned_names.contains(&"terraform".to_string()));
    assert!(planned_names.contains(&"go".to_string()));
}

#[tokio::test]
async fn uninstall_accepts_full_tap_reference_after_install() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let bottle = create_bottle_tarball("terraform");
    let sha = sha256_hex(&bottle);
    let tag = get_test_bottle_tag();

    let tap_formula_rb = format!(
        r#"
class Terraform < Formula
  version "1.10.0"
  bottle do
    root_url "{}/v2/hashicorp/tap"
    sha256 {}: "{}"
  end
end
"#,
        mock_server.uri(),
        tag,
        sha
    );

    Mock::given(method("GET"))
        .and(path("/hashicorp/homebrew-tap/main/Formula/terraform.rb"))
        .respond_with(ResponseTemplate::new(200).set_body_string(tap_formula_rb))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/v2/hashicorp/tap/terraform/blobs/sha256:{sha}"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle))
        .mount(&mock_server)
        .await;

    let api_client =
        ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root.clone();
    let _prefix = ctx.prefix.clone();
    let mut installer = ctx.installer;

    installer
        .install(&["hashicorp/tap/terraform".to_string()], true)
        .await
        .unwrap();

    assert!(installer.is_installed("hashicorp/tap/terraform"));
    assert!(!installer.is_installed("terraform"));
    assert!(root.join("Cellar/terraform/1.10.0").exists());
    installer.uninstall("hashicorp/tap/terraform").unwrap();
    assert!(!installer.is_installed("hashicorp/tap/terraform"));
    assert!(!root.join("Cellar/terraform/1.10.0").exists());
}

#[tokio::test]
async fn uninstalling_non_installed_tap_ref_does_not_remove_core_formula() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();

    let bottle = create_bottle_tarball("terraform");
    let sha = sha256_hex(&bottle);
    let tag = get_test_bottle_tag();
    let core_json = format!(
        r#"{{
                "name": "terraform",
                "versions": {{ "stable": "1.10.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/terraform-1.10.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        tag,
        mock_server.uri(),
        tag,
        sha
    );

    Mock::given(method("GET"))
        .and(path("/terraform.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(core_json))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/bottles/terraform-1.10.0.{}.bottle.tar.gz",
            tag
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle))
        .mount(&mock_server)
        .await;

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let _root = ctx.root.clone();
    let _prefix = ctx.prefix.clone();
    let mut installer = ctx.installer;
    installer
        .install(&["terraform".to_string()], true)
        .await
        .unwrap();
    assert!(installer.is_installed("terraform"));

    let err = installer.uninstall("hashicorp/tap/terraform").unwrap_err();
    assert!(matches!(err, Error::NotInstalled { .. }));
    assert!(installer.is_installed("terraform"));
}

fn bottled_formula_json(
    server: &MockServer,
    name: &str,
    version: &str,
    deps: &str,
    sha: &str,
) -> String {
    let tag = get_test_bottle_tag();
    format!(
        r#"{{
                "name": "{name}",
                "versions": {{ "stable": "{version}" }},
                "dependencies": [{deps}],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{tag}": {{
                                "url": "{uri}/bottles/{name}-{version}.{tag}.bottle.tar.gz",
                                "sha256": "{sha}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        uri = server.uri(),
    )
}

async fn mount_bottled(server: &MockServer, name: &str, version: &str, deps: &str) {
    let tag = get_test_bottle_tag();
    let bottle = create_bottle_tarball(name);
    let json = bottled_formula_json(server, name, version, deps, &sha256_hex(&bottle));
    Mock::given(method("GET"))
        .and(path(format!("/{name}.json")))
        .respond_with(ResponseTemplate::new(200).set_body_string(json))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/bottles/{name}-{version}.{tag}.bottle.tar.gz"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle))
        .mount(server)
        .await;
}

async fn install_deplib_1_0_0(tmp: &TempDir) {
    let server = MockServer::start().await;
    mount_bottled(&server, "deplib", "1.0.0", "").await;
    let mut installer = new_test_context(ApiClient::with_base_url(server.uri()), tmp).installer;
    installer
        .install(&["deplib".to_string()], true)
        .await
        .unwrap();
}

#[tokio::test]
async fn installed_dependency_without_a_bottle_upgrade_is_kept() {
    let tmp = TempDir::new().unwrap();
    install_deplib_1_0_0(&tmp).await;

    let server = MockServer::start().await;
    mount_bottled(&server, "mainpkg", "2.0.0", r#""deplib""#).await;
    let source_only = r#"{
            "name": "deplib",
            "versions": { "stable": "1.1.0" },
            "dependencies": [],
            "urls": { "stable": { "url": "https://example.com/deplib-1.1.0.tar.gz", "checksum": "abc123" } },
            "ruby_source_path": "Formula/d/deplib.rb",
            "bottle": { "stable": { "files": {} } }
        }"#;
    Mock::given(method("GET"))
        .and(path("/deplib.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(source_only))
        .mount(&server)
        .await;

    let mut installer = new_test_context(ApiClient::with_base_url(server.uri()), &tmp).installer;
    let plan = installer.plan(&["mainpkg".to_string()]).await.unwrap();
    let planned: Vec<_> = plan.items.iter().map(|i| i.install_name.as_str()).collect();
    assert_eq!(planned, ["mainpkg"]);

    installer
        .install(&["mainpkg".to_string()], true)
        .await
        .unwrap();
    assert_eq!(installer.get_installed("deplib").unwrap().version, "1.0.0");
    assert!(installer.get_installed("mainpkg").is_some());
}

#[tokio::test]
async fn requested_formula_without_a_bottle_is_still_planned_from_source() {
    let tmp = TempDir::new().unwrap();
    install_deplib_1_0_0(&tmp).await;

    let server = MockServer::start().await;
    let source_only = r#"{
            "name": "deplib",
            "versions": { "stable": "1.1.0" },
            "dependencies": [],
            "urls": { "stable": { "url": "https://example.com/deplib-1.1.0.tar.gz", "checksum": "abc123" } },
            "ruby_source_path": "Formula/d/deplib.rb",
            "bottle": { "stable": { "files": {} } }
        }"#;
    Mock::given(method("GET"))
        .and(path("/deplib.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(source_only))
        .mount(&server)
        .await;

    let installer = new_test_context(ApiClient::with_base_url(server.uri()), &tmp).installer;
    let plan = installer.plan(&["deplib".to_string()]).await.unwrap();
    assert_eq!(plan.items.len(), 1);
    assert!(matches!(
        plan.items[0].method,
        crate::types::InstallMethod::Source(_)
    ));
}

#[tokio::test]
async fn installed_dependency_with_an_unreadable_formula_is_kept() {
    let tmp = TempDir::new().unwrap();
    install_deplib_1_0_0(&tmp).await;

    let server = MockServer::start().await;
    mount_bottled(&server, "mainpkg", "2.0.0", r#""deplib""#).await;
    Mock::given(method("GET"))
        .and(path("/deplib.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let installer = new_test_context(ApiClient::with_base_url(server.uri()), &tmp).installer;
    let plan = installer.plan(&["mainpkg".to_string()]).await.unwrap();
    let planned: Vec<_> = plan.items.iter().map(|i| i.install_name.as_str()).collect();
    assert_eq!(planned, ["mainpkg"]);
}
