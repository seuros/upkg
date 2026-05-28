use crate::error::UpkgError;

fn install_options(kind: crate::cli::PackageKind) -> crate::api::InstallOptions {
    crate::api::InstallOptions {
        package_kind: match kind {
            crate::cli::PackageKind::Auto => crate::api::PackageKindHint::Auto,
            crate::cli::PackageKind::App => crate::api::PackageKindHint::App,
        },
        ..crate::api::InstallOptions::default()
    }
}

pub fn install_native(packages: &[String], kind: crate::cli::PackageKind) -> Result<(), UpkgError> {
    let options = install_options(kind);
    crate::api::install(packages, &options).map_err(UpkgError::Native)
}

pub fn uninstall_native(
    packages: &[String],
    kind: crate::cli::PackageKind,
) -> Result<(), UpkgError> {
    let options = install_options(kind);
    crate::api::uninstall(packages, &options).map_err(UpkgError::Native)
}

pub fn upgrade_native(packages: &[String], kind: crate::cli::PackageKind) -> Result<(), UpkgError> {
    let options = install_options(kind);
    crate::api::upgrade(packages, &options).map_err(UpkgError::Native)
}

pub fn print_install_dry_run(
    packages: &[String],
    kind: crate::cli::PackageKind,
) -> Result<(), UpkgError> {
    print_dry_run("install", packages, kind)
}

pub fn print_uninstall_dry_run(
    packages: &[String],
    kind: crate::cli::PackageKind,
) -> Result<(), UpkgError> {
    print_dry_run("uninstall", packages, kind)
}

pub fn print_upgrade_dry_run(
    packages: &[String],
    kind: crate::cli::PackageKind,
) -> Result<(), UpkgError> {
    print_dry_run("upgrade", packages, kind)
}

fn print_dry_run(
    command: &str,
    packages: &[String],
    kind: crate::cli::PackageKind,
) -> Result<(), UpkgError> {
    let package_args = match kind {
        crate::cli::PackageKind::Auto => packages.to_vec(),
        crate::cli::PackageKind::App => packages
            .iter()
            .map(|package| crate::package_ref::normalize_app_name(package))
            .collect::<Result<Vec<_>, _>>()
            .map_err(UpkgError::Native)?,
    };

    let mut rendered = vec!["upkg".to_string(), command.to_string()];
    if kind == crate::cli::PackageKind::App {
        rendered.push("--app".to_string());
    }
    rendered.extend(package_args);

    println!("engine: built-in macOS Homebrew-compatible");
    println!("dry-run: {}", rendered.join(" "));
    Ok(())
}

pub fn search_native(
    query: &str,
    exact: bool,
    kind: crate::cli::PackageKind,
    refresh: bool,
) -> Result<(), UpkgError> {
    let options = crate::api::SearchOptions {
        package_kind: match kind {
            crate::cli::PackageKind::Auto => crate::api::PackageKindHint::Auto,
            crate::cli::PackageKind::App => crate::api::PackageKindHint::App,
        },
        exact,
        refresh,
        ..crate::api::SearchOptions::default()
    };

    let hits = crate::api::search(query, &options).map_err(UpkgError::Native)?;

    for hit in hits {
        let label = match hit.kind {
            crate::api::SearchKind::Formula => "formula",
            crate::api::SearchKind::Cask => "app",
        };
        let desc = hit.desc.as_deref().unwrap_or("");
        println!("{label}\t{}\t{}\t{}", hit.name, hit.version, desc);
    }

    Ok(())
}

pub fn list_native() -> Result<(), UpkgError> {
    let options = crate::api::InstallOptions::default();
    let installed = crate::api::list(&options).map_err(UpkgError::Native)?;

    if installed.is_empty() {
        return Ok(());
    }

    for package in installed {
        let kind = if crate::package_ref::is_cask_name(&package.name) {
            "app"
        } else {
            "formula"
        };
        println!("{kind}\t{}\t{}", package.name, package.version);
    }

    Ok(())
}
