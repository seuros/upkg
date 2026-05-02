use std::path::PathBuf;

use crate::core::installer::install::create_installer;
use crate::types::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKindHint {
    Auto,
    App,
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub root: Option<PathBuf>,
    pub prefix: Option<PathBuf>,
    pub concurrency: usize,
    pub no_link: bool,
    pub build_from_source: bool,
    pub package_kind: PackageKindHint,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            root: None,
            prefix: None,
            concurrency: 20,
            no_link: false,
            build_from_source: false,
            package_kind: PackageKindHint::Auto,
        }
    }
}

fn resolve_root_and_prefix(options: &InstallOptions) -> (PathBuf, PathBuf) {
    let default_prefix = default_prefix();
    let root = options.root.clone().unwrap_or_else(default_root);
    let prefix = options.prefix.clone().unwrap_or(default_prefix);
    (root, prefix)
}

fn build_runtime() -> Result<tokio::runtime::Runtime, Error> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to initialize runtime: {e}"),
        })
}

pub fn install(formulas: &[String], options: &InstallOptions) -> Result<(), Error> {
    if formulas.is_empty() {
        return Err(Error::InvalidArgument {
            message: "install requires at least one formula".to_string(),
        });
    }

    let (root, prefix) = resolve_root_and_prefix(options);

    crate::init::ensure_init(&root, &prefix, true)?;

    let runtime = build_runtime()?;

    runtime.block_on(async {
        let mut installer = create_installer(&root, &prefix, options.concurrency)?;

        crate::native_cli::commands::install::execute(
            &mut installer,
            formulas.to_vec(),
            options.no_link,
            options.build_from_source,
            options.package_kind,
        )
        .await
    })
}

pub fn uninstall(formulas: &[String], options: &InstallOptions) -> Result<(), Error> {
    if formulas.is_empty() {
        return Err(Error::InvalidArgument {
            message: "uninstall requires at least one formula".to_string(),
        });
    }

    let (root, prefix) = resolve_root_and_prefix(options);
    let mut installer = create_installer(&root, &prefix, options.concurrency)?;

    for formula in package_requests(formulas, options.package_kind)? {
        installer.uninstall(&formula)?;
    }
    let _ = installer.gc()?;

    Ok(())
}

pub fn upgrade(formulas: &[String], options: &InstallOptions) -> Result<(), Error> {
    if formulas.is_empty() && options.package_kind == PackageKindHint::App {
        return Err(Error::InvalidArgument {
            message: "upgrade --app requires at least one app for now".to_string(),
        });
    }

    let (root, prefix) = resolve_root_and_prefix(options);
    crate::init::ensure_init(&root, &prefix, true)?;

    let runtime = build_runtime()?;

    runtime.block_on(async {
        let mut installer = create_installer(&root, &prefix, options.concurrency)?;
        let targets = if formulas.is_empty() {
            let mut installed: Vec<String> = installer
                .list_installed()?
                .into_iter()
                .map(|keg| keg.name)
                .collect();
            installed.sort();
            installed.dedup();
            installed
        } else {
            formulas.to_vec()
        };

        if targets.is_empty() {
            return Ok(());
        }

        crate::native_cli::commands::install::execute(
            &mut installer,
            targets,
            options.no_link,
            options.build_from_source,
            options.package_kind,
        )
        .await
    })
}

fn default_prefix() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if cfg!(target_arch = "aarch64") {
            PathBuf::from("/opt/homebrew")
        } else {
            PathBuf::from("/usr/local")
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".upkg")
            .join("prefix")
    }
}

fn default_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        default_prefix()
    }

    #[cfg(not(target_os = "macos"))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".upkg")
    }
}

fn package_requests(
    packages: &[String],
    kind: PackageKindHint,
) -> Result<Vec<String>, crate::types::Error> {
    match kind {
        PackageKindHint::Auto => Ok(packages.to_vec()),
        PackageKindHint::App => packages
            .iter()
            .map(|package| {
                let trimmed = package.trim();
                if let Some(token) = trimmed.strip_prefix("cask:") {
                    if token.is_empty() {
                        return Err(Error::InvalidArgument {
                            message: "cask token cannot be empty".to_string(),
                        });
                    }
                    return Ok(trimmed.to_string());
                }
                if let Some((tap, token)) = trimmed.rsplit_once('/') {
                    if token.is_empty() {
                        return Err(Error::MissingFormula {
                            name: trimmed.to_string(),
                        });
                    }
                    if tap != "homebrew/cask" {
                        return Err(Error::InvalidArgument {
                            message: format!("'{package}' is not a supported app reference"),
                        });
                    }
                    return Ok(format!("cask:{token}"));
                }
                Ok(format!("cask:{trimmed}"))
            })
            .collect(),
    }
}
