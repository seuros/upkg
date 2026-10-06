use super::*;
use crate::types::BuildSystem;
use std::assert_matches;

fn test_build_plan(prefix: &Path) -> BuildPlan {
    let cellar_path = prefix.join("Cellar").join("foo").join("1.0.0");
    BuildPlan {
        formula_name: "foo".to_string(),
        version: "1.0.0".to_string(),
        source_url: "https://example.com/foo-1.0.0.tar.gz".to_string(),
        source_checksum: None,
        ruby_source_path: None,
        build_dependencies: Vec::new(),
        runtime_dependencies: Vec::new(),
        detected_system: BuildSystem::RubyFormula,
        prefix: prefix.to_path_buf(),
        cellar_path,
    }
}

#[tokio::test]
async fn unsupported_formula_fails_before_touching_the_cellar() {
    let tmp = tempfile::tempdir().unwrap();
    let prefix = tmp.path().join("prefix");
    let plan = test_build_plan(&prefix);
    let formula_rb = tmp.path().join("foo.rb");
    std::fs::write(
        &formula_rb,
        "class Foo < Formula\n  def install\n    system \"./configure\", \"--prefix=#{prefix}\"\n    system \"make\", \"install\"\n  end\nend\n",
    )
    .unwrap();

    let executor = BuildExecutor::new(prefix.clone(), tmp.path().to_path_buf());
    let err = executor
        .execute(&plan, &formula_rb, &HashMap::new())
        .await
        .unwrap_err();

    assert_matches!(err, Error::UnsupportedFormula { .. }, "{err:?}");
    assert!(!plan.cellar_path.exists());
    assert!(!tmp.path().join("cache/build/foo").exists());
}

#[tokio::test]
async fn native_install_plan_moves_supported_targets() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let source_root = tmp.path().join("source");
    std::fs::create_dir_all(source_root.join("themes")).unwrap();
    std::fs::create_dir_all(source_root.join("build")).unwrap();
    std::fs::write(source_root.join("themes/default.omp.json"), "{}").unwrap();
    std::fs::write(source_root.join("build/foo"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(
        source_root.join("build/foo"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    std::fs::write(source_root.join("README.md"), "readme").unwrap();

    let prefix = tmp.path().join("prefix");
    let plan = test_build_plan(&prefix);
    let native_plan = InstallPlan {
        actions: vec![
            InstallAction::Move {
                sources: vec!["themes".to_string()],
                destination: InstallTarget::Prefix,
            },
            InstallAction::Install {
                destination: InstallTarget::Bin,
                sources: vec![InstallSource {
                    source: "build/foo".to_string(),
                    target_name: None,
                }],
            },
            InstallAction::Install {
                destination: InstallTarget::Doc,
                sources: vec![InstallSource {
                    source: "README.md".to_string(),
                    target_name: None,
                }],
            },
        ],
    };

    execute_native_install_plan(&plan, &source_root, &native_plan)
        .await
        .unwrap();

    assert!(
        plan.cellar_path
            .join("themes")
            .join("default.omp.json")
            .exists()
    );
    assert!(plan.cellar_path.join("bin").join("foo").exists());
    let mode = std::fs::metadata(plan.cellar_path.join("bin").join("foo"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
    assert!(
        plan.cellar_path
            .join("share")
            .join("doc")
            .join("foo")
            .join("README.md")
            .exists()
    );
}

#[tokio::test]
async fn native_install_plan_renames_and_marks_bin_install_executable() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let source_root = tmp.path().join("source");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(source_root.join("foo.sh"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(
        source_root.join("foo.sh"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let prefix = tmp.path().join("prefix");
    let plan = test_build_plan(&prefix);
    let native_plan = InstallPlan {
        actions: vec![InstallAction::Install {
            destination: InstallTarget::Bin,
            sources: vec![InstallSource {
                source: "foo.sh".to_string(),
                target_name: Some("foo".to_string()),
            }],
        }],
    };

    execute_native_install_plan(&plan, &source_root, &native_plan)
        .await
        .unwrap();

    let installed = plan.cellar_path.join("bin").join("foo");
    assert!(installed.exists());
    assert!(!plan.cellar_path.join("bin").join("foo.sh").exists());
    let mode = std::fs::metadata(installed).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111);
}
