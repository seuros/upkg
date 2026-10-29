use std::borrow::Cow;

use super::*;
use crate::types::formula::{PostInstallPath, PostInstallStep};

enum Operation {
    Directory(PathBuf),
    Tree(PathBuf, PathBuf),
    Link(PathBuf, PathBuf),
}

/// Template values derived once per keg (`Cellar/<formula>/<version>`).
struct KegVars<'a> {
    keg: &'a Path,
    formula_name: &'a str,
    version_major: &'a str,
}

impl<'a> KegVars<'a> {
    fn new(keg: &'a Path) -> Result<Self, Error> {
        let formula_name = keg_formula_name(keg).ok_or_else(|| invalid_step("invalid keg path"))?;
        let version = keg
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| invalid_step("invalid keg version"))?;
        let version_major = version.split('.').next().unwrap_or(version);
        Ok(Self {
            keg,
            formula_name,
            version_major,
        })
    }

    /// Single pass; borrows `value` when it holds no template.
    fn expand<'v>(&self, value: &'v str) -> Result<Cow<'v, str>, Error> {
        let unsupported = || invalid_step("unsupported path template");
        let Some(first) = value.find("{{") else {
            return if value.contains("}}") {
                Err(unsupported())
            } else {
                Ok(Cow::Borrowed(value))
            };
        };
        let mut out = String::with_capacity(value.len() + self.formula_name.len());
        let mut rest = value;
        let mut start = first;
        loop {
            let literal = &rest[..start];
            if literal.contains("}}") {
                return Err(unsupported());
            }
            out.push_str(literal);
            let after = &rest[start + 2..];
            let end = after.find("}}").ok_or_else(unsupported)?;
            out.push_str(match &after[..end] {
                "formula_name" => self.formula_name,
                "version.major" => self.version_major,
                _ => return Err(unsupported()),
            });
            rest = &after[end + 2..];
            match rest.find("{{") {
                Some(next) => start = next,
                None if rest.contains("}}") => return Err(unsupported()),
                None => break,
            }
        }
        out.push_str(rest);
        Ok(Cow::Owned(out))
    }

    fn suffix<'v>(&self, suffix: &'v str) -> Result<Cow<'v, str>, Error> {
        let suffix = self.expand(suffix)?;
        if suffix.contains(['/', '\\']) {
            return Err(invalid_step(
                "link suffix must not contain a path separator",
            ));
        }
        Ok(suffix)
    }
}

impl Linker {
    /// These runtime links are required even when ordinary linking is disabled.
    /// Unlike Ruby post_install hooks, this never executes package code.
    pub fn apply_post_install(&self, keg: &Path, steps: &[PostInstallStep]) -> Result<(), Error> {
        let operations = self.post_install_operations(keg, steps)?;
        // Check the complete plan before writing any links.
        let mut conflicts = Vec::new();
        for op in &operations {
            match op {
                Operation::Directory(dst) => self.check_post_install_destination(dst)?,
                Operation::Tree(src, dst) => {
                    self.check_post_install_destination(dst)?;
                    Self::collect_conflicts(src, dst, &mut conflicts);
                }
                // The parent is the preceding `Directory` operation's path.
                Operation::Link(src, dst) => {
                    if dst.symlink_metadata().is_ok()
                        && !read_link_resolved(dst)
                            .is_ok_and(|old| same_path(&old, src) || same_cellar_formula(&old, src))
                    {
                        conflicts.push(ConflictedLink {
                            path: dst.clone(),
                            owned_by: keg_name_from_symlink(dst),
                        });
                    }
                }
            }
        }
        if !conflicts.is_empty() {
            return Err(Error::LinkConflict { conflicts });
        }
        for op in operations {
            match op {
                Operation::Directory(dst) => fs::create_dir_all(dst).map_err(store_err)?,
                Operation::Tree(src, dst) => Self::link_recursive(&src, &dst)?,
                Operation::Link(src, dst) => {
                    if dst.symlink_metadata().is_ok() {
                        fs::remove_file(&dst).map_err(store_err)?;
                    }
                    std::os::unix::fs::symlink(src, dst).map_err(store_err)?;
                }
            }
        }
        Ok(())
    }

    /// Remove only links belonging to this keg; retain data and extensions.
    pub fn remove_post_install(&self, keg: &Path, steps: &[PostInstallStep]) -> Result<(), Error> {
        if steps.is_empty() {
            return Ok(());
        }
        let vars = KegVars::new(keg)?;
        // Read destinations, not sources: uninstall must also work for a
        // damaged keg whose runtime files or directories have disappeared.
        for step in steps {
            match step {
                PostInstallStep::LinkDir { source, target } => {
                    let (src, dst) = self.post_install_link(&vars, source, target)?;
                    remove_runtime_tree(&src, &dst)?;
                }
                PostInstallStep::LinkChildren {
                    source,
                    target,
                    suffix,
                } => {
                    let (src, dst) = self.post_install_link(&vars, source, target)?;
                    let suffix = vars.suffix(suffix)?;
                    if !dst.exists() {
                        continue;
                    }
                    for entry in fs::read_dir(dst).map_err(store_err)? {
                        let entry = entry.map_err(store_err)?;
                        let path = entry.path();
                        if let Ok(resolved) = read_link_resolved(&path)
                            && resolved
                                .parent()
                                .is_some_and(|parent| same_path(parent, &src))
                            && let Some(stem) = path
                                .file_name()
                                .and_then(|n| n.as_encoded_bytes().strip_suffix(suffix.as_bytes()))
                            && resolved
                                .file_name()
                                .is_some_and(|n| n.as_encoded_bytes() == stem)
                        {
                            fs::remove_file(path).map_err(store_err)?;
                        }
                    }
                }
                PostInstallStep::MkdirP { .. } | PostInstallStep::Unsupported => {}
            }
        }
        Ok(())
    }

    fn post_install_operations(
        &self,
        keg: &Path,
        steps: &[PostInstallStep],
    ) -> Result<Vec<Operation>, Error> {
        if steps.is_empty() {
            return Ok(Vec::new());
        }
        let vars = KegVars::new(keg)?;
        let mut operations = Vec::new();
        for step in steps {
            match step {
                PostInstallStep::MkdirP { path } => {
                    operations.push(Operation::Directory(
                        self.post_install_path(&vars, path, false)?,
                    ));
                }
                PostInstallStep::LinkDir { source, target } => {
                    let (src, dst) = self.post_install_link(&vars, source, target)?;
                    check_post_install_source(keg, &src)?;
                    operations.push(Operation::Tree(src, dst));
                }
                PostInstallStep::LinkChildren {
                    source,
                    target,
                    suffix,
                } => {
                    let (src, dst) = self.post_install_link(&vars, source, target)?;
                    check_post_install_source(keg, &src)?;
                    let suffix = vars.suffix(suffix)?;
                    let dir_index = operations.len();
                    for entry in fs::read_dir(src).map_err(store_err)? {
                        let entry = entry.map_err(store_err)?;
                        let mut name = entry.file_name();
                        name.push(&*suffix);
                        operations.push(Operation::Link(entry.path(), dst.join(name)));
                    }
                    // Checked and created once, before its children are linked.
                    operations.insert(dir_index, Operation::Directory(dst));
                }
                // Do not initialize databases or start services during installation.
                PostInstallStep::Unsupported => {}
            }
        }
        Ok(operations)
    }

    /// Resolve a link step's keg source and prefix destination.
    fn post_install_link(
        &self,
        vars: &KegVars<'_>,
        source: &PostInstallPath,
        target: &PostInstallPath,
    ) -> Result<(PathBuf, PathBuf), Error> {
        let src = self.post_install_path(vars, source, true)?;
        let dst = self.post_install_path(vars, target, false)?;
        self.check_post_install_destination(&dst)?;
        Ok((src, dst))
    }

    fn post_install_path(
        &self,
        vars: &KegVars<'_>,
        path: &PostInstallPath,
        source: bool,
    ) -> Result<PathBuf, Error> {
        // Homebrew also emits absolute prefix templates without a `base`.
        // Strip the exact marker, then apply the same relative-path validation
        // as explicit bases; never expand arbitrary absolute filesystem paths.
        let (path_base, value) = if path.base.is_empty() {
            let relative = path
                .path
                .strip_prefix("{{HOMEBREW_PREFIX}}/")
                .ok_or_else(|| invalid_step("unsupported absolute path template"))?;
            ("homebrew_prefix", relative)
        } else {
            (path.base.as_str(), path.path.as_str())
        };
        let (base, sub) = match (source, path_base) {
            (true, "prefix") => (vars.keg, None),
            (false, "homebrew_prefix") => (self.prefix.as_path(), None),
            (false, "var") => (self.prefix.as_path(), Some("var")),
            (false, "etc") => (self.prefix.as_path(), Some("etc")),
            _ => return Err(invalid_step("unsupported path base")),
        };
        let expanded = vars.expand(value)?;
        let relative = Path::new(&*expanded);
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(invalid_step(
                "paths must be nonempty relative paths without traversal",
            ));
        }
        let mut out = PathBuf::with_capacity(
            base.as_os_str().len()
                + sub.map_or(0, |s| s.len() + 1)
                + relative.as_os_str().len()
                + 1,
        );
        out.push(base);
        if let Some(sub) = sub {
            out.push(sub);
        }
        out.push(relative);
        Ok(out)
    }

    fn check_post_install_destination(&self, dst: &Path) -> Result<(), Error> {
        let relative = dst
            .strip_prefix(&self.prefix)
            .map_err(|_| invalid_step("destination must be inside the installation prefix"))?;
        let mut ancestor = self.prefix.clone();
        for component in relative.components() {
            ancestor.push(component);
            if ancestor.is_symlink() {
                return Err(invalid_step(
                    "destination must not traverse directory symlinks",
                ));
            }
            if ancestor.exists() && !ancestor.is_dir() {
                return Err(invalid_step("destination parent is not a directory"));
            }
        }
        Ok(())
    }
}

fn check_post_install_source(keg: &Path, src: &Path) -> Result<(), Error> {
    if !src.is_dir()
        || !fs::canonicalize(src)
            .map_err(store_err)?
            .starts_with(fs::canonicalize(keg).map_err(store_err)?)
    {
        return Err(invalid_step(
            "link source must be a directory inside the keg",
        ));
    }
    Ok(())
}

fn remove_runtime_tree(src: &Path, dst: &Path) -> Result<(), Error> {
    if !dst.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dst).map_err(store_err)? {
        let entry = entry.map_err(store_err)?;
        let src_path = src.join(entry.file_name());
        let dst_path = entry.path();
        if dst_path.is_symlink() {
            if read_link_resolved(&dst_path).is_ok_and(|old| same_path(&old, &src_path)) {
                fs::remove_file(dst_path).map_err(store_err)?;
            }
        } else if dst_path.is_dir() {
            remove_runtime_tree(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn invalid_step(reason: &str) -> Error {
    Error::StoreCorruption {
        message: format!("invalid declarative post-install step: {reason}"),
    }
}

#[cfg(test)]
#[path = "post_install/tests.rs"]
mod tests;
