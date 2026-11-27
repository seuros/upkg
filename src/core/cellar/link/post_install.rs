use std::borrow::Cow;

use super::*;
use crate::types::formula::{PostInstallPath, PostInstallStep, TRUSTED_SCRIPTS};

/// A trusted `run` step, resolved: the keg's script and its path arguments.
/// The linker validates it; the installer runs it.
#[derive(Debug, PartialEq, Eq)]
pub struct PostInstallScript {
    pub path: PathBuf,
    pub args: Vec<PathBuf>,
}

enum Operation {
    Directory(PathBuf),
    Tree(PathBuf, PathBuf),
    Link(PathBuf, PathBuf),
    /// A `force`d link: replaces a file or link already at the destination,
    /// never a directory.
    Replace(PathBuf, PathBuf),
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
                    Self::collect_conflicts(src, dst, &mut conflicts, false);
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
                Operation::Replace(_, dst) => {
                    if dst.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
                        conflicts.push(ConflictedLink {
                            path: dst.clone(),
                            owned_by: None,
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
                Operation::Tree(src, dst) => Self::link_recursive(&src, &dst, false)?,
                Operation::Link(src, dst) | Operation::Replace(src, dst) => {
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
                PostInstallStep::Symlink {
                    source,
                    target,
                    source_glob: false,
                    ..
                } => {
                    let (src, dst) = self.symlink_paths(&vars, source, target)?;
                    if read_link_resolved(&dst).is_ok_and(|old| same_path(&old, &src)) {
                        fs::remove_file(dst).map_err(store_err)?;
                    }
                }
                // What a script wrote (a CA bundle) stays, as with Homebrew.
                PostInstallStep::MkdirP { .. }
                | PostInstallStep::Symlink { .. }
                | PostInstallStep::Run { .. }
                | PostInstallStep::Unsupported { .. } => {}
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
                PostInstallStep::Symlink {
                    source,
                    target,
                    force,
                    source_glob: false,
                } => {
                    let (src, dst) = self.symlink_paths(&vars, source, target)?;
                    let parent = dst
                        .parent()
                        .ok_or_else(|| invalid_step("symlink target has no parent"))?;
                    operations.push(Operation::Directory(parent.to_path_buf()));
                    operations.push(if *force {
                        Operation::Replace(src, dst)
                    } else {
                        Operation::Link(src, dst)
                    });
                }
                // Glob sources, databases, services, package scripts: reported
                // or run by the installer, never here.
                PostInstallStep::Symlink { .. }
                | PostInstallStep::Run { .. }
                | PostInstallStep::Unsupported { .. } => {}
            }
        }
        Ok(operations)
    }

    /// The trusted `run` steps of `steps`, resolved and checked: the script is
    /// a file in the keg's `libexec`, every argument a path template whose
    /// parent stays inside the prefix. Steps of untrusted formulas are left
    /// out (the installer reports them).
    pub fn post_install_scripts(
        &self,
        keg: &Path,
        steps: &[PostInstallStep],
    ) -> Result<Vec<PostInstallScript>, Error> {
        let vars = KegVars::new(keg)?;
        if !TRUSTED_SCRIPTS.contains(&vars.formula_name) {
            return Ok(Vec::new());
        }
        let mut scripts = Vec::new();
        for step in steps {
            let PostInstallStep::Run { command, args } = step else {
                continue;
            };
            if command.base != "libexec" {
                return Err(invalid_step("a script must live in the keg's libexec"));
            }
            let path = join_relative(&keg.join("libexec"), &vars, &command.path)?;
            if !path.is_file()
                || !fs::canonicalize(&path)
                    .map_err(store_err)?
                    .starts_with(fs::canonicalize(keg).map_err(store_err)?)
            {
                return Err(invalid_step("a script must be a file inside the keg"));
            }
            let args = args
                .iter()
                .map(|arg| {
                    let path = self.helper_path(
                        &vars,
                        &PostInstallPath {
                            base: String::new(),
                            path: arg.clone(),
                        },
                    )?;
                    self.check_helper_parent(keg, &path)?;
                    Ok(path)
                })
                .collect::<Result<_, Error>>()?;
            scripts.push(PostInstallScript { path, args });
        }
        Ok(scripts)
    }

    /// A symlink step's link text and link path. Either may name any of
    /// Homebrew's path helpers; the link's parent must stay inside the prefix,
    /// and the link itself may already exist (it is replaced or conflicts).
    fn symlink_paths(
        &self,
        vars: &KegVars<'_>,
        source: &PostInstallPath,
        target: &PostInstallPath,
    ) -> Result<(PathBuf, PathBuf), Error> {
        let src = self.helper_path(vars, source)?;
        let dst = self.helper_path(vars, target)?;
        self.check_helper_parent(vars.keg, &dst)?;
        Ok((src, dst))
    }

    /// A helper path lands in the keg (which may live outside the prefix) or
    /// in the prefix; its parent must stay there without crossing a
    /// directory symlink.
    fn check_helper_parent(&self, keg: &Path, path: &Path) -> Result<(), Error> {
        let parent = path
            .parent()
            .ok_or_else(|| invalid_step("path has no parent"))?;
        match parent.strip_prefix(keg) {
            Ok(relative) => {
                let mut ancestor = keg.to_path_buf();
                for component in relative.components() {
                    ancestor.push(component);
                    if ancestor.is_symlink() {
                        return Err(invalid_step(
                            "destination must not traverse directory symlinks",
                        ));
                    }
                }
                Ok(())
            }
            Err(_) => self.check_post_install_destination(parent),
        }
    }

    /// `{{helper}}/relative`, or an explicit `base` naming the helper.
    fn helper_path(&self, vars: &KegVars<'_>, path: &PostInstallPath) -> Result<PathBuf, Error> {
        let (helper, relative) = if path.base.is_empty() {
            path.path
                .strip_prefix("{{")
                .and_then(|rest| rest.split_once("}}/"))
                .ok_or_else(|| invalid_step("unsupported absolute path template"))?
        } else {
            (path.base.as_str(), path.path.as_str())
        };
        let root = self
            .helper_root(vars, helper)
            .ok_or_else(|| invalid_step("unsupported path base"))?;
        join_relative(&root, vars, relative)
    }

    /// Homebrew's `Formula` path helpers: the keg's directories, then the
    /// prefix's.
    fn helper_root(&self, vars: &KegVars<'_>, helper: &str) -> Option<PathBuf> {
        let keg = |relative: &str| Some(vars.keg.join(relative));
        let prefix = |relative: &str| Some(self.prefix.join(relative));
        match helper {
            "prefix" => Some(vars.keg.to_path_buf()),
            "bin" | "sbin" | "lib" | "libexec" | "include" | "share" => keg(helper),
            "pkgshare" => Some(vars.keg.join("share").join(vars.formula_name)),
            "bash_completion" => keg("etc/bash_completion.d"),
            "zsh_completion" => keg("share/zsh/site-functions"),
            "fish_completion" => keg("share/fish/vendor_completions.d"),
            "pwsh_completion" => keg("share/pwsh/completions"),
            "HOMEBREW_PREFIX" | "homebrew_prefix" => Some(self.prefix.clone()),
            "etc" | "var" => prefix(helper),
            "pkgetc" => Some(self.prefix.join("etc").join(vars.formula_name)),
            _ => None,
        }
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
        let base = sub.map_or_else(|| base.to_path_buf(), |sub| base.join(sub));
        join_relative(&base, vars, value)
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

/// `base` joined with `value` once templates expand: a nonempty relative path
/// without traversal, never an arbitrary absolute one.
fn join_relative(base: &Path, vars: &KegVars<'_>, value: &str) -> Result<PathBuf, Error> {
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
    Ok(base.join(relative))
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
