use super::*;
use crate::formula::types::*;
use std::collections::BTreeMap;

fn test_formula(name: &str, source_url: &str, build_deps: &[&str]) -> Formula {
    let mut files = BTreeMap::new();
    files.insert(
        "arm64_sonoma".to_string(),
        BottleFile {
            url: format!("https://example.com/{name}.tar.gz"),
            sha256: "deadbeef".repeat(8),
        },
    );

    Formula {
        name: name.to_string(),
        versions: Versions {
            stable: "1.0.0".to_string(),
        },
        dependencies: vec!["libfoo".to_string()],
        bottle: Bottle {
            stable: BottleStable { files, rebuild: 0 },
        },
        revision: 0,
        keg_only: KegOnly::default(),
        build_dependencies: build_deps.iter().map(|s| s.to_string()).collect(),
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
        ruby_source_checksum: None,
        uses_from_macos: Vec::new(),
        requirements: Vec::new(),
        variations: None,
    }
}

#[test]
fn detects_cmake_from_build_deps() {
    let f = test_formula("libheif", "https://example.com/src.tar.gz", &["cmake"]);
    let prefix = PathBuf::from("/opt/upkg");
    let plan = BuildPlan::from_formula(&f, &prefix).unwrap();
    assert_eq!(plan.detected_system, BuildSystem::Cmake);
}

#[test]
fn detects_meson_from_build_deps() {
    let f = test_formula("glib", "https://example.com/src.tar.xz", &["meson"]);
    let prefix = PathBuf::from("/opt/upkg");
    let plan = BuildPlan::from_formula(&f, &prefix).unwrap();
    assert_eq!(plan.detected_system, BuildSystem::Meson);
}

#[test]
fn detects_autoconf_from_tarball_url() {
    let f = test_formula("wget", "https://ftp.gnu.org/wget-1.25.tar.gz", &["pkgconf"]);
    let prefix = PathBuf::from("/opt/upkg");
    let plan = BuildPlan::from_formula(&f, &prefix).unwrap();
    assert_eq!(plan.detected_system, BuildSystem::Autoconf);
}

#[test]
fn returns_none_without_source_url() {
    let mut f = test_formula("wget", "https://example.com/src.tar.gz", &[]);
    f.urls = None;
    let prefix = PathBuf::from("/opt/upkg");
    assert!(BuildPlan::from_formula(&f, &prefix).is_none());
}

#[test]
fn cellar_path_includes_version() {
    let f = test_formula("wget", "https://example.com/src.tar.gz", &[]);
    let prefix = PathBuf::from("/opt/upkg");
    let plan = BuildPlan::from_formula(&f, &prefix).unwrap();
    assert_eq!(
        plan.cellar_path,
        PathBuf::from("/opt/upkg/Cellar/wget/1.0.0")
    );
}
