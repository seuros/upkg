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
