use super::*;
use rstest::rstest;

fn parse(args: &[&str]) -> Result<Cli, UpkgError> {
    Cli::parse(args.iter().copied())
}

#[rstest]
#[case(&["i", "git"], "install")]
#[case(&["remove", "git"], "uninstall")]
#[case(&["rm", "git"], "uninstall")]
#[case(&["update"], "upgrade")]
#[case(&["reinstall", "git"], "reinstall")]
#[case(&["list"], "list")]
#[case(&["ls"], "list")]
#[case(&["check", "git"], "check")]
#[case(&["shaman"], "shaman")]
#[case(&["doctor"], "shaman")]
#[case(&["--self-upgrade", "--dry-run"], "self-upgrade")]
fn aliases(#[case] args: &[&str], #[case] expected: &str) {
    let cli = parse(args).unwrap();
    let name = match cli.command {
        CommandKind::Package { action, .. } => action.as_str(),
        CommandKind::List => "list",
        CommandKind::Check { .. } => "check",
        CommandKind::Shaman => "shaman",
        CommandKind::SelfUpgrade { dry_run: true } => "self-upgrade",
        _ => panic!("unexpected command"),
    };
    assert_eq!(name, expected);
}

#[rstest]
#[case(&[], "command")]
#[case(&["nonesuch"], "nonesuch")]
#[case(&["--unknown"], "--unknown")]
#[case(&["install", "--app"], "--app needs app names")]
#[case(&["install", "--manifest"], "--manifest")]
#[case(&["install", "--manifest", "ci/upkg.toml", "git"], "not both")]
#[case(&["check", "--manifest", "ci/upkg.toml", "git"], "not both")]
#[case(&["uninstall"], "<PACKAGES>")]
#[case(&["uninstall", "--dry-run"], "<PACKAGES>")]
#[case(&["reinstall"], "<PACKAGES>")]
#[case(&["search"], "<QUERY>")]
#[case(&["search", "--exact"], "<QUERY>")]
#[case(&["list", "extra"], "extra")]
#[case(&["shaman", "extra"], "extra")]
#[case(&["shaman", "--verbose"], "--verbose")]
#[case(&["self-upgrade", "--app"], "--app")]
#[case(&["--self-upgrade", "extra"], "extra")]
fn invalid_arguments(#[case] args: &[&str], #[case] expected: &str) {
    let error = parse(args).unwrap_err();
    assert!(error.to_string().contains(expected), "{error}");
}

#[rstest]
#[case(&["help"], "install")]
#[case(&["--help"], "install")]
#[case(&["-h"], "install")]
#[case(&["install", "--help"], "--dry-run")]
#[case(&["help", "install"], "--dry-run")]
#[case(&["i", "-h"], "--app")]
#[case(&["search", "--help"], "--refresh")]
#[case(&["check", "--help"], "missing")]
#[case(&["--self-upgrade", "--help"], "--dry-run")]
fn generated_help(#[case] args: &[&str], #[case] expected: &str) {
    let cli = parse(args).unwrap();
    let CommandKind::Print(text) = cli.command else {
        panic!("expected help");
    };
    assert!(text.contains(expected), "{text}");
    assert!(text.contains("upkg"), "{text}");
}

#[rstest]
#[case::install_with_dry_run(
    &["install", "--dry-run", "git"],
    PackageAction::Install, &["git"], true, PackageKind::Auto
)]
#[case::uninstall_with_dry_run(
    &["uninstall", "--dry-run", "git"],
    PackageAction::Uninstall, &["git"], true, PackageKind::Auto
)]
#[case::upgrade_allows_no_package(&["upgrade"], PackageAction::Upgrade, &[], false, PackageKind::Auto)]
#[case::install_app_hint(
    &["install", "--app", "ghostty"],
    PackageAction::Install, &["ghostty"], false, PackageKind::App
)]
#[case::options_can_follow_values_and_repeat(
    &["install", "git", "--dry-run", "curl", "--app", "--dry-run", "--app"],
    PackageAction::Install, &["git", "curl"], true, PackageKind::App
)]
fn package_command(
    #[case] args: &[&str],
    #[case] expected_action: PackageAction,
    #[case] expected_packages: &[&str],
    #[case] expected_dry_run: bool,
    #[case] expected_kind: PackageKind,
) {
    let CommandKind::Package {
        action,
        packages,
        manifest,
        dry_run,
        kind,
    } = parse(args).expect("parse should succeed").command
    else {
        panic!("expected package command");
    };
    assert_eq!(action, expected_action);
    assert_eq!(packages, expected_packages);
    assert_eq!(manifest, None);
    assert_eq!(dry_run, expected_dry_run);
    assert_eq!(kind, expected_kind);
}

#[rstest]
#[case("install")]
#[case("uninstall")]
#[case("upgrade")]
#[case("reinstall")]
fn package_double_dash(#[case] command: &str) {
    let cli = parse(&[command, "--", "--app", "--help"]).unwrap();
    let CommandKind::Package { packages, kind, .. } = cli.command else {
        panic!("expected package command");
    };
    assert_eq!(packages, ["--app", "--help"]);
    assert_eq!(kind, PackageKind::Auto);
}

#[rstest]
#[case("install")]
#[case("uninstall")]
#[case("upgrade")]
#[case("reinstall")]
#[case("check")]
#[case("search")]
fn unknown_flags_remain_positional_values(#[case] command: &str) {
    let cli = parse(&[command, "--unknown"]).unwrap();
    match cli.command {
        CommandKind::Package { packages, .. } => assert_eq!(packages, ["--unknown"]),
        CommandKind::Check { packages, .. } => assert_eq!(packages, ["--unknown"]),
        CommandKind::Search { query, .. } => assert_eq!(query, "--unknown"),
        _ => panic!("unexpected command"),
    }
}

#[rstest]
#[case::simple(&["search", "ripgrep"], "ripgrep", false, false, PackageKind::Auto)]
#[case::joins_multi_word_query(
    &["search", "visual", "studio", "code"],
    "visual studio code", false, false, PackageKind::Auto
)]
#[case::alias_and_flags(
    &["s", "--app", "--exact", "--refresh", "ghostty"],
    "ghostty", true, true, PackageKind::App
)]
#[case::double_dash_passes_through_flag_like_queries(
    &["search", "--", "--exact"],
    "--exact", false, false, PackageKind::Auto
)]
#[case::short_exact_flag(&["search", "-e", "git"], "git", true, false, PackageKind::Auto)]
fn search_command(
    #[case] args: &[&str],
    #[case] expected_query: &str,
    #[case] expected_exact: bool,
    #[case] expected_refresh: bool,
    #[case] expected_kind: PackageKind,
) {
    let CommandKind::Search {
        query,
        exact,
        kind,
        refresh,
    } = parse(args).expect("parse ok").command
    else {
        panic!("expected search command");
    };
    assert_eq!(query, expected_query);
    assert_eq!(exact, expected_exact);
    assert_eq!(refresh, expected_refresh);
    assert_eq!(kind, expected_kind);
}

#[rstest]
#[case(&["--self-upgrade"], false)]
#[case(&["self-upgrade", "--dry-run"], true)]
fn self_upgrade_command(#[case] args: &[&str], #[case] expected_dry_run: bool) {
    let CommandKind::SelfUpgrade { dry_run } = parse(args).expect("parse should succeed").command
    else {
        panic!("expected self-upgrade command");
    };
    assert_eq!(dry_run, expected_dry_run);
}

#[test]
fn check_keeps_package_order() {
    let CommandKind::Check { packages, .. } =
        parse(&["check", "git", "libpq-dev"]).unwrap().command
    else {
        panic!("expected check command");
    };
    assert_eq!(packages, ["git", "libpq-dev"]);
}

#[rstest]
#[case(&["refresh"], upkg::host::Task::Refresh, false)]
#[case(&["refresh", "--dry-run"], upkg::host::Task::Refresh, true)]
#[case(&["cleanup", "--dry-run"], upkg::host::Task::Cleanup, true)]
#[case(&["upgrade", "--full"], upkg::host::Task::FullUpgrade, false)]
#[case(&["upgrade", "--full", "--dry-run"], upkg::host::Task::FullUpgrade, true)]
#[case(&["update", "--dry-run", "--full"], upkg::host::Task::FullUpgrade, true)]
fn maintenance_commands(
    #[case] args: &[&str],
    #[case] expected_task: upkg::host::Task,
    #[case] expected_dry_run: bool,
) {
    let CommandKind::Maintain { task, dry_run } = parse(args).expect("parse ok").command else {
        panic!("expected a maintenance command");
    };
    assert_eq!(task, expected_task);
    assert_eq!(dry_run, expected_dry_run);
}

#[test]
fn plain_upgrade_keeps_its_behavior() {
    let CommandKind::Package {
        action, packages, ..
    } = parse(&["upgrade", "git"]).unwrap().command
    else {
        panic!("expected package command");
    };
    assert_eq!(action, PackageAction::Upgrade);
    assert_eq!(packages, ["git"]);
}

#[rstest]
#[case(&["reboot-required"], false)]
#[case(&["reboot-required", "--json"], true)]
fn reboot_required_command(#[case] args: &[&str], #[case] expected_json: bool) {
    let CommandKind::RebootRequired { json } = parse(args).unwrap().command else {
        panic!("expected reboot-required");
    };
    assert_eq!(json, expected_json);
}

#[rstest]
#[case(&["upgrade", "--full", "git"], "takes no package names")]
#[case(&["upgrade", "--full", "--app"], "--app")]
#[case(&["refresh", "git"], "git")]
#[case(&["cleanup", "--full"], "--full")]
#[case(&["reboot-required", "--dry-run"], "--dry-run")]
fn maintenance_rejects_extras(#[case] args: &[&str], #[case] expected: &str) {
    let error = parse(args).unwrap_err();
    assert!(error.to_string().contains(expected), "{error}");
    assert!(
        matches!(error, UpkgError::Usage { code: 2, .. }),
        "{error:?}"
    );
}

#[rstest]
#[case(&["--help"], "reboot-required")]
#[case(&["--help"], "cleanup")]
#[case(&["upgrade", "--help"], "--full")]
#[case(&["refresh", "--help"], "--dry-run")]
#[case(&["reboot-required", "--help"], "--json")]
fn maintenance_help(#[case] args: &[&str], #[case] expected: &str) {
    let CommandKind::Print(text) = parse(args).unwrap().command else {
        panic!("expected help");
    };
    assert!(text.contains(expected), "{text}");
}

#[rstest]
#[case(&["install"], Some(ManifestSource::WorkingDir))]
#[case(&["install", "--dry-run"], Some(ManifestSource::WorkingDir))]
#[case(
    &["install", "--manifest", "ci/upkg.toml"],
    Some(ManifestSource::Path("ci/upkg.toml".into()))
)]
#[case(&["install", "git"], None)]
// After `--`, `--manifest` is a package name like any other.
#[case(&["install", "--", "--manifest"], None)]
fn install_reads_a_manifest_only_without_names(
    #[case] args: &[&str],
    #[case] expected: Option<ManifestSource>,
) {
    let CommandKind::Package {
        action: PackageAction::Install,
        packages,
        manifest,
        ..
    } = parse(args).unwrap().command
    else {
        panic!("expected install");
    };
    assert_eq!(packages.is_empty(), expected.is_some(), "{packages:?}");
    assert_eq!(manifest, expected);
}

#[rstest]
#[case(&["check"], Some(ManifestSource::WorkingDir))]
#[case(
    &["check", "--manifest", "/srv/app/upkg.toml"],
    Some(ManifestSource::Path("/srv/app/upkg.toml".into()))
)]
#[case(&["check", "git"], None)]
fn check_reads_a_manifest_only_without_names(
    #[case] args: &[&str],
    #[case] expected: Option<ManifestSource>,
) {
    let CommandKind::Check { manifest, .. } = parse(args).unwrap().command else {
        panic!("expected check");
    };
    assert_eq!(manifest, expected);
}

#[test]
fn other_package_commands_never_read_a_manifest() {
    for args in [
        &["upgrade"][..],
        &["uninstall", "git"],
        &["reinstall", "git"],
    ] {
        let CommandKind::Package { manifest, .. } = parse(args).unwrap().command else {
            panic!("expected a package command for {args:?}");
        };
        assert_eq!(manifest, None, "{args:?}");
    }
    // Unknown flags are package names there, as they always were.
    let CommandKind::Package { packages, .. } = parse(&["uninstall", "--manifest", "upkg.toml"])
        .unwrap()
        .command
    else {
        panic!("expected uninstall");
    };
    assert_eq!(packages, ["--manifest", "upkg.toml"]);
}
