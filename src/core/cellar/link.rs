use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::types::{ConflictedLink, Error};

const LINK_DIRS: &[&str] = &[
    "bin",
    "sbin",
    "lib",
    "libexec",
    "cli-plugins",
    "include",
    "share",
    "etc",
];

pub struct Linker {
    prefix: PathBuf,
    opt_dir: PathBuf,
}

fn keg_name_from_path(path: &Path) -> Option<String> {
    let components: Vec<_> = path.components().collect();
    for (i, c) in components.iter().enumerate() {
        if let Component::Normal(s) = c
            && s.eq_ignore_ascii_case("cellar")
            && let Some(Component::Normal(name)) = components.get(i + 1)
        {
            return name.to_str().map(String::from);
        }
    }
    None
}

fn read_link_resolved(link: &Path) -> io::Result<PathBuf> {
    let target = fs::read_link(link)?;
    Ok(if target.is_relative() {
        link.parent().unwrap_or(Path::new("")).join(target)
    } else {
        target
    })
}

fn same_path(left: &Path, right: &Path) -> bool {
    fs::canonicalize(left).ok() == fs::canonicalize(right).ok()
}

fn keg_formula_name(keg_path: &Path) -> Option<&str> {
    keg_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
}

fn store_err(e: io::Error) -> Error {
    Error::StoreCorruption {
        message: e.to_string(),
    }
}

fn keg_name_from_symlink(dst: &Path) -> Option<String> {
    let resolved = read_link_resolved(dst).ok()?;
    let canonical = fs::canonicalize(&resolved).ok()?;
    keg_name_from_path(&canonical)
}

fn same_cellar_formula(left: &Path, right: &Path) -> bool {
    let left_name = fs::canonicalize(left)
        .ok()
        .and_then(|path| keg_name_from_path(&path));
    let right_name = fs::canonicalize(right)
        .ok()
        .and_then(|path| keg_name_from_path(&path));

    left_name.is_some() && left_name == right_name
}

impl Linker {
    pub fn new(prefix: &Path) -> io::Result<Self> {
        let bin_dir = prefix.join("bin");
        let opt_dir = prefix.join("opt");
        fs::create_dir_all(&bin_dir)?;
        fs::create_dir_all(&opt_dir)?;

        for dir in LINK_DIRS {
            if *dir != "bin" {
                fs::create_dir_all(prefix.join(dir))?;
            }
        }

        Ok(Self {
            prefix: prefix.to_path_buf(),
            opt_dir,
        })
    }

    pub fn check_conflicts(&self, keg_path: &Path) -> Result<(), Error> {
        let mut conflicts = Vec::new();
        for dir_name in LINK_DIRS {
            let src_dir = keg_path.join(dir_name);
            let dst_dir = self.prefix.join(dir_name);
            if src_dir.exists() {
                Self::collect_conflicts(&src_dir, &dst_dir, &mut conflicts);
            }
        }
        if conflicts.is_empty() {
            Ok(())
        } else {
            Err(Error::LinkConflict { conflicts })
        }
    }

    fn collect_conflicts(src: &Path, dst: &Path, conflicts: &mut Vec<ConflictedLink>) {
        let Ok(entries) = fs::read_dir(src) else {
            return;
        };
        for entry in entries.flatten() {
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() {
                if dst_path.symlink_metadata().is_ok()
                    && dst_path.is_symlink()
                    && let Ok(resolved) = read_link_resolved(&dst_path)
                {
                    Self::collect_conflicts_merged(&src_path, &resolved, &dst_path, conflicts);
                    continue;
                }
                Self::collect_conflicts(&src_path, &dst_path, conflicts);
                continue;
            }

            if dst_path.symlink_metadata().is_ok() {
                if let Ok(resolved) = read_link_resolved(&dst_path) {
                    if same_path(&resolved, &src_path) {
                        continue;
                    }
                    if same_cellar_formula(&resolved, &src_path) {
                        continue;
                    }
                }
                conflicts.push(ConflictedLink {
                    path: dst_path.clone(),
                    owned_by: keg_name_from_symlink(&dst_path),
                });
            } else if dst_path.exists() {
                conflicts.push(ConflictedLink {
                    path: dst_path,
                    owned_by: None,
                });
            }
        }
    }

    fn collect_conflicts_merged(
        src: &Path,
        old_target: &Path,
        dst: &Path,
        conflicts: &mut Vec<ConflictedLink>,
    ) {
        let new_entries = match fs::read_dir(src) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in new_entries.flatten() {
            let src_path = entry.path();
            let matching_old = old_target.join(entry.file_name());
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() {
                if matching_old.exists() {
                    Self::collect_conflicts_merged(&src_path, &matching_old, &dst_path, conflicts);
                } else {
                    Self::collect_conflicts(&src_path, &dst_path, conflicts);
                }
                continue;
            }

            if matching_old.exists()
                && fs::canonicalize(&matching_old).ok() != fs::canonicalize(&src_path).ok()
                && !same_cellar_formula(&matching_old, &src_path)
            {
                conflicts.push(ConflictedLink {
                    path: dst_path,
                    owned_by: keg_name_from_symlink(dst).or_else(|| keg_name_from_path(old_target)),
                });
            }
        }
    }

    pub fn link_keg(&self, keg_path: &Path) -> Result<(), Error> {
        self.check_conflicts(keg_path)?;
        self.link_opt(keg_path)?;
        for dir_name in LINK_DIRS {
            let src_dir = keg_path.join(dir_name);
            let dst_dir = self.prefix.join(dir_name);
            if src_dir.exists() {
                Self::link_recursive(&src_dir, &dst_dir)?;
            }
        }
        Ok(())
    }

    fn link_recursive(src: &Path, dst: &Path) -> Result<(), Error> {
        if !dst.exists() {
            fs::create_dir_all(dst).map_err(store_err)?;
        }

        for entry in fs::read_dir(src).map_err(store_err)? {
            let entry = entry.map_err(store_err)?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() {
                if dst_path.symlink_metadata().is_ok() && dst_path.is_symlink() {
                    let resolved_target = read_link_resolved(&dst_path).map_err(store_err)?;
                    let _ = fs::remove_file(&dst_path);
                    Self::link_recursive(&resolved_target, &dst_path)?;
                }
                Self::link_recursive(&src_path, &dst_path)?;
                continue;
            }

            if dst_path.symlink_metadata().is_ok() {
                if let Ok(resolved) = read_link_resolved(&dst_path) {
                    if same_path(&resolved, &src_path) {
                        if resolved.exists() {
                            continue;
                        } else {
                            let _ = fs::remove_file(&dst_path);
                        }
                    } else if same_cellar_formula(&resolved, &src_path) {
                        let _ = fs::remove_file(&dst_path);
                    } else {
                        return Err(Error::LinkConflict {
                            conflicts: vec![ConflictedLink {
                                path: dst_path.clone(),
                                owned_by: keg_name_from_symlink(&dst_path),
                            }],
                        });
                    }
                } else {
                    return Err(Error::LinkConflict {
                        conflicts: vec![ConflictedLink {
                            path: dst_path,
                            owned_by: None,
                        }],
                    });
                }
            } else if dst_path.exists() {
                return Err(Error::LinkConflict {
                    conflicts: vec![ConflictedLink {
                        path: dst_path,
                        owned_by: None,
                    }],
                });
            }

            #[cfg(unix)]
            std::os::unix::fs::symlink(&src_path, &dst_path).map_err(store_err)?;
        }
        Ok(())
    }

    pub fn unlink_keg(&self, keg_path: &Path) -> Result<Vec<PathBuf>, Error> {
        self.unlink_opt(keg_path)?;
        let mut unlinked = Vec::new();
        for dir_name in LINK_DIRS {
            let src_dir = keg_path.join(dir_name);
            let dst_dir = self.prefix.join(dir_name);
            if src_dir.exists() {
                unlinked.extend(Self::unlink_recursive(&src_dir, &dst_dir)?);
            }
        }
        Ok(unlinked)
    }

    fn unlink_recursive(src: &Path, dst: &Path) -> Result<Vec<PathBuf>, Error> {
        let mut unlinked = Vec::new();
        if !src.exists() || !dst.exists() {
            return Ok(unlinked);
        }
        for entry in fs::read_dir(src).map_err(store_err)? {
            let entry = entry.map_err(store_err)?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() && dst_path.is_dir() && !dst_path.is_symlink() {
                unlinked.extend(Self::unlink_recursive(&src_path, &dst_path)?);
                if let Ok(mut entries) = fs::read_dir(&dst_path)
                    && entries.next().is_none()
                {
                    let _ = fs::remove_dir(&dst_path);
                }
                continue;
            }

            if let Ok(resolved) = read_link_resolved(&dst_path)
                && same_path(&resolved, &src_path)
            {
                let _ = fs::remove_file(&dst_path);
                unlinked.push(dst_path);
            }
        }
        Ok(unlinked)
    }

    fn unlink_opt(&self, keg_path: &Path) -> Result<(), Error> {
        if let Some(name) = keg_formula_name(keg_path) {
            let opt_link = self.opt_dir.join(name);
            if let Ok(resolved) = read_link_resolved(&opt_link)
                && same_path(&resolved, keg_path)
            {
                let _ = fs::remove_file(&opt_link);
            }
        }
        Ok(())
    }

    pub fn link_opt(&self, keg_path: &Path) -> Result<(), Error> {
        let name = keg_formula_name(keg_path).ok_or_else(|| Error::StoreCorruption {
            message: "invalid keg path".into(),
        })?;
        let opt_link = self.opt_dir.join(name);
        if opt_link.symlink_metadata().is_ok() {
            if let Ok(resolved) = read_link_resolved(&opt_link)
                && same_path(&resolved, keg_path)
            {
                return Ok(());
            }
            let _ = fs::remove_file(&opt_link);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(keg_path, &opt_link).map_err(store_err)?;
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "link/tests.rs"]
mod tests;
