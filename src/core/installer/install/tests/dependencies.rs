use super::*;

/// deplib 1.1.0 published without any bottle, only a source tarball.
const SOURCE_ONLY_DEPLIB: &str = r#"{
        "name": "deplib",
        "versions": { "stable": "1.1.0" },
        "dependencies": [],
        "urls": { "stable": { "url": "https://example.com/deplib-1.1.0.tar.gz", "checksum": "abc123" } },
        "ruby_source_path": "Formula/d/deplib.rb",
        "bottle": { "stable": { "files": {} } }
    }"#;

#[tokio::test]
async fn install_with_dependencies() {
    let mock_server = MockServer::start().await;
    let tmp = TempDir::new().unwrap();
    mount_bottled(&mock_server, "deplib", "1.0.0", &[]).await;
    mount_bottled(&mock_server, "mainpkg", "2.0.0", &["deplib"]).await;

    let mut installer = server_installer(&mock_server, &tmp);

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
    mount_bottled(&mock_server, "go", "1.24.0", &[]).await;

    let tag = get_test_bottle_tag();
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
    mount_text(
        &mock_server,
        "/hashicorp/homebrew-tap/main/Formula/terraform.rb",
        tap_formula_rb,
    )
    .await;

    let api_client =
        ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
    let installer = new_test_installer(api_client, &tmp);
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
    let sha = sha256_hex_bytes(&bottle);
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
    mount_text(
        &mock_server,
        "/hashicorp/homebrew-tap/main/Formula/terraform.rb",
        tap_formula_rb,
    )
    .await;
    mount_bytes(
        &mock_server,
        &format!("/v2/hashicorp/tap/terraform/blobs/sha256:{sha}"),
        bottle,
    )
    .await;

    let api_client =
        ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
    let ctx = new_test_context(api_client, &tmp);
    let root = ctx.root;
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
    mount_bottled(&mock_server, "terraform", "1.10.0", &[]).await;

    let mut installer = server_installer(&mock_server, &tmp);
    installer
        .install(&["terraform".to_string()], true)
        .await
        .unwrap();
    assert!(installer.is_installed("terraform"));

    let err = installer.uninstall("hashicorp/tap/terraform").unwrap_err();
    assert_matches!(err, Error::NotInstalled { .. });
    assert!(installer.is_installed("terraform"));
}

async fn install_deplib_1_0_0(tmp: &TempDir) {
    let server = MockServer::start().await;
    mount_bottled(&server, "deplib", "1.0.0", &[]).await;
    let mut installer = server_installer(&server, tmp);
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
    mount_bottled(&server, "mainpkg", "2.0.0", &["deplib"]).await;
    mount_text(&server, "/deplib.json", SOURCE_ONLY_DEPLIB).await;

    let mut installer = server_installer(&server, &tmp);
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
    mount_text(&server, "/deplib.json", SOURCE_ONLY_DEPLIB).await;

    let installer = server_installer(&server, &tmp);
    let plan = installer.plan(&["deplib".to_string()]).await.unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_matches!(plan.items[0].method, crate::types::InstallMethod::Source(_));
}

#[tokio::test]
async fn installed_dependency_with_an_unreadable_formula_is_kept() {
    let tmp = TempDir::new().unwrap();
    install_deplib_1_0_0(&tmp).await;

    let server = MockServer::start().await;
    mount_bottled(&server, "mainpkg", "2.0.0", &["deplib"]).await;
    mount_status(&server, "/deplib.json", 404).await;

    let installer = server_installer(&server, &tmp);
    let plan = installer.plan(&["mainpkg".to_string()]).await.unwrap();
    let planned: Vec<_> = plan.items.iter().map(|i| i.install_name.as_str()).collect();
    assert_eq!(planned, ["mainpkg"]);
}
