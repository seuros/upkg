use super::*;
use crate::formula::test_support::{names, test_formula};
use crate::formula::types::{FormulaUrls, SourceUrl};
use rstest::rstest;

const PREFIX: &str = "/opt/upkg";

/// Buildable formula with a stable source tarball at `source_url`.
fn source_formula(name: &str, source_url: &str, build_deps: &[&str]) -> Formula {
    Formula {
        dependencies: names(&["libfoo"]),
        build_dependencies: names(build_deps),
        urls: Some(FormulaUrls {
            stable: Some(SourceUrl {
                url: source_url.to_string(),
                checksum: Some("abc123".to_string()),
                tag: None,
                revision: None,
            }),
            head: None,
        }),
        ruby_source_path: Some(format!("Formula/{}/{name}.rb", &name[..1])),
        ..test_formula(name)
    }
}

fn plan_for(formula: &Formula) -> Option<BuildPlan> {
    BuildPlan::from_formula(formula, &PathBuf::from(PREFIX))
}

#[rstest]
#[case::cmake_from_build_deps("libheif", "https://example.com/src.tar.gz", &["cmake"], BuildSystem::Cmake)]
#[case::meson_from_build_deps("glib", "https://example.com/src.tar.xz", &["meson"], BuildSystem::Meson)]
#[case::autoconf_from_tarball_url(
    "wget",
    "https://ftp.gnu.org/wget-1.25.tar.gz",
    &["pkgconf"],
    BuildSystem::Autoconf
)]
fn detects_build_system(
    #[case] name: &str,
    #[case] source_url: &str,
    #[case] build_deps: &[&str],
    #[case] expected: BuildSystem,
) {
    let plan = plan_for(&source_formula(name, source_url, build_deps)).unwrap();
    assert_eq!(plan.detected_system, expected);
}

#[test]
fn returns_none_without_source_url() {
    let f = Formula {
        urls: None,
        ..source_formula("wget", "https://example.com/src.tar.gz", &[])
    };
    assert!(plan_for(&f).is_none());
}

#[test]
fn cellar_path_includes_version() {
    let f = source_formula("wget", "https://example.com/src.tar.gz", &[]);
    let plan = plan_for(&f).unwrap();
    assert_eq!(
        plan.cellar_path,
        PathBuf::from("/opt/upkg/Cellar/wget/1.0.0")
    );
}
