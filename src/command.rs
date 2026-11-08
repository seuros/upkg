//! Commands upkg hands to the native package managers, and how they get root.

use std::env;
use std::path::Path;
use std::process::Command;

/// One native command: a program, its arguments and any environment it needs.
///
/// Builders write privileged commands as `sudo <tool> ...`; [`CommandSpec::escalate`]
/// rewrites that prefix for the caller's [`Privilege`] right before running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>, args: impl Into<Vec<String>>) -> Self {
        Self {
            program: program.into(),
            args: args.into(),
            env: Vec::new(),
        }
    }

    pub fn with_packages(program: impl Into<String>, args: &[&str], packages: &[String]) -> Self {
        let words = args.iter().map(ToString::to_string);
        Self::new(
            program,
            words.chain(packages.iter().cloned()).collect::<Vec<_>>(),
        )
    }

    /// Sets `key=value` for the tool itself, surviving sudo/doas (see [`Self::argv`]).
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Appends `extra` to the arguments.
    #[must_use]
    pub fn arguments(mut self, extra: impl IntoIterator<Item = String>) -> Self {
        self.args.extend(extra);
        self
    }

    /// The program as built, before environment is folded in: `sudo` for
    /// privileged commands until [`Self::escalate`] runs.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The arguments as built, without environment assignments.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The environment the tool runs with, in the order it was set.
    #[must_use]
    pub fn environment(&self) -> &[(String, String)] {
        &self.env
    }

    /// The process to start and its arguments, with environment folded in
    /// where the process would otherwise drop it.
    ///
    /// sudo resets the environment, so assignments go on its command line
    /// (`sudo KEY=value tool`), which sudoers allows for `ALL` or a `SETENV`
    /// rule. doas has no such syntax and goes through `env(1)`. Any other
    /// program gets them through the process environment (see
    /// [`Self::process_env`]).
    #[must_use]
    pub fn argv(&self) -> (&str, Vec<String>) {
        let assignments = self.env.iter().map(|(key, value)| format!("{key}={value}"));
        let args = match self.program.as_str() {
            "sudo" => assignments.chain(self.args.iter().cloned()).collect(),
            "doas" if !self.env.is_empty() => std::iter::once("env".to_string())
                .chain(assignments)
                .chain(self.args.iter().cloned())
                .collect(),
            _ => self.args.clone(),
        };
        (&self.program, args)
    }

    /// Environment to set on the process itself: everything, unless sudo or
    /// doas carries it on the command line instead.
    #[must_use]
    pub fn process_env(&self) -> &[(String, String)] {
        match self.program.as_str() {
            "sudo" | "doas" => &[],
            _ => &self.env,
        }
    }

    /// The command as a shell would show it.
    #[must_use]
    pub fn render(&self) -> String {
        let (program, args) = self.argv();
        let words: Vec<String> = self
            .process_env()
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .chain([program.to_string()])
            .chain(args)
            .collect();
        words.join(" ").trim().to_string()
    }

    /// Rewrites a `sudo` prefix: dropped for root, swapped for doas.
    #[must_use]
    pub fn escalate(self, privilege: Privilege) -> Self {
        if self.program != "sudo" {
            return self;
        }
        let Self { args, env, .. } = self;
        let mut spec = match privilege.resolve() {
            Privilege::Sudo | Privilege::Auto => Self::new("sudo", args),
            Privilege::Doas => Self::new("doas", args),
            Privilege::AlreadyRoot => {
                let mut args = args.into_iter();
                args.next().map_or_else(
                    || Self::new("sudo", Vec::<String>::new()),
                    |program| Self::new(program, args.collect::<Vec<_>>()),
                )
            }
        };
        spec.env = env;
        spec
    }

    /// The command as the current user can run it.
    #[must_use]
    pub fn for_current_user(self) -> Self {
        self.escalate(Privilege::Auto)
    }

    /// A [`Command`] ready to spawn, environment included.
    #[must_use]
    pub fn into_command(self) -> Command {
        let (program, args) = self.argv();
        let mut command = Command::new(program);
        command.args(args);
        command.envs(self.process_env().iter().map(|(key, value)| (key, value)));
        command
    }
}

/// How a command that needs root is run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Privilege {
    /// Root runs commands as is; anyone else gets sudo, or doas when only
    /// doas exists (Alpine, the BSDs).
    #[default]
    Auto,
    /// The caller is root (a daemon, a container, CI): never prefix anything.
    AlreadyRoot,
    Sudo,
    Doas,
}

impl Privilege {
    /// The concrete mode: `Auto` becomes `AlreadyRoot`, `Doas` or `Sudo`.
    ///
    /// Falls back to sudo when neither tool exists, so the error names the
    /// missing tool.
    #[must_use]
    pub fn resolve(self) -> Self {
        match self {
            Self::Auto if is_root() => Self::AlreadyRoot,
            Self::Auto if !command_exists("sudo") && command_exists("doas") => Self::Doas,
            Self::Auto => Self::Sudo,
            other => other,
        }
    }
}

/// Whether the effective user is root, asked of `id -u`: no libc, no unsafe.
#[must_use]
pub fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|output| output.stdout.trim_ascii() == b"0")
}

/// Whether `name` resolves to a file on `PATH` (or is a path to a file).
#[must_use]
pub fn command_exists(name: &str) -> bool {
    if name.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(name).is_file();
    }

    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|path| {
        let candidate = path.join(name);
        if candidate.is_file() {
            return true;
        }

        #[cfg(windows)]
        {
            ["exe", "cmd", "bat"]
                .iter()
                .any(|ext| path.join(format!("{name}.{ext}")).is_file())
        }

        #[cfg(not(windows))]
        {
            false
        }
    })
}

#[cfg(test)]
mod tests;
