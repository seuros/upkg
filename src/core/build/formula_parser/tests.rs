use super::*;

#[test]
fn parses_mv_into_prefix() {
    let plan = parse_supported_install_plan(
        r#"
class Foo < Formula
  def install
    mv "themes", prefix
  end
end
"#,
    )
    .unwrap()
    .unwrap();

    assert_eq!(
        plan,
        InstallPlan {
            actions: vec![InstallAction::Move {
                sources: vec!["themes".to_string()],
                destination: InstallTarget::Prefix,
            }],
        }
    );
}

#[test]
fn parses_install_into_named_target() {
    let plan = parse_supported_install_plan(
        r#"
class Foo < Formula
  def install
    bin.install "foo"
    prefix.install "README.md", "LICENSE"
  end
end
"#,
    )
    .unwrap()
    .unwrap();

    assert_eq!(
        plan,
        InstallPlan {
            actions: vec![
                InstallAction::Install {
                    destination: InstallTarget::Bin,
                    sources: vec![InstallSource {
                        source: "foo".to_string(),
                        target_name: None,
                    }],
                },
                InstallAction::Install {
                    destination: InstallTarget::Prefix,
                    sources: vec![
                        InstallSource {
                            source: "README.md".to_string(),
                            target_name: None,
                        },
                        InstallSource {
                            source: "LICENSE".to_string(),
                            target_name: None,
                        },
                    ],
                },
            ],
        }
    );
}

#[test]
fn parses_local_variable_ternary_and_renamed_install() {
    let plan = parse_supported_install_plan(
        r#"
class AgentSafehouse < Formula
  def install
    artifact_path = build.head? ? "dist/safehouse.sh" : "safehouse.sh"
    bin.install artifact_path => "safehouse"
  end
end
"#,
    )
    .unwrap()
    .unwrap();

    assert_eq!(
        plan,
        InstallPlan {
            actions: vec![InstallAction::Install {
                destination: InstallTarget::Bin,
                sources: vec![InstallSource {
                    source: "safehouse.sh".to_string(),
                    target_name: Some("safehouse".to_string()),
                }],
            }],
        }
    );
}

#[cfg(target_os = "macos")]
#[test]
fn skips_macos_odie_guard() {
    let plan = parse_supported_install_plan(
        r#"
class AgentSafehouse < Formula
  def install
    odie "Agent Safehouse requires macOS" unless OS.mac?
    bin.install "safehouse.sh" => "safehouse"
  end
end
"#,
    )
    .unwrap()
    .unwrap();

    assert_eq!(
        plan,
        InstallPlan {
            actions: vec![InstallAction::Install {
                destination: InstallTarget::Bin,
                sources: vec![InstallSource {
                    source: "safehouse.sh".to_string(),
                    target_name: Some("safehouse".to_string()),
                }],
            }],
        }
    );
}

#[test]
fn returns_none_for_unsupported_system_calls() {
    let plan = parse_supported_install_plan(
        r#"
class Foo < Formula
  def install
    system "sh", "-c", "echo hi"
  end
end
"#,
    )
    .unwrap();

    assert!(plan.is_none());
}
