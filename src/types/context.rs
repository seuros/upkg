use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub root: PathBuf,
    pub store: PathBuf,
    pub cellar: PathBuf,
    pub cache: PathBuf,
    pub locks: PathBuf,
}

impl Paths {
    pub fn from_root(root: PathBuf) -> Self {
        let store = root.join("store");
        let cellar = root.join("Cellar");
        let cache = root.join("cache");
        let locks = root.join("locks");

        Self {
            root,
            store,
            cellar,
            cache,
            locks,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConcurrencyLimits {
    pub download: usize,
    pub unpack: usize,
    pub materialize: usize,
}

impl Default for ConcurrencyLimits {
    fn default() -> Self {
        Self {
            download: 20,
            unpack: 4,
            materialize: 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoggerHandle {
    pub level: LogLevel,
}

impl Default for LoggerHandle {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub paths: Paths,
    pub concurrency: ConcurrencyLimits,
    pub logger: LoggerHandle,
}

impl Context {
    pub fn from_defaults() -> Self {
        Self {
            paths: Paths::from_root(default_root()),
            concurrency: ConcurrencyLimits::default(),
            logger: LoggerHandle::default(),
        }
    }
}

fn default_root() -> PathBuf {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        PathBuf::from("/opt/homebrew")
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        PathBuf::from("/usr/local")
    }

    #[cfg(not(target_os = "macos"))]
    {
        PathBuf::from("/opt/homebrew")
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn from_defaults_sets_expected_paths() {
        let context = Context::from_defaults();

        #[cfg(target_arch = "aarch64")]
        let root = PathBuf::from("/opt/homebrew");
        #[cfg(target_arch = "x86_64")]
        let root = PathBuf::from("/usr/local");

        assert_eq!(context.paths.root, root.clone());
        assert_eq!(context.paths.store, root.join("store"));
        assert_eq!(context.paths.cellar, root.join("Cellar"));
        assert_eq!(context.paths.cache, root.join("cache"));
        assert_eq!(context.paths.locks, root.join("locks"));
    }
}
