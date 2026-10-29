use serde::{Deserialize, Serialize};

/// Homebrew's declarative filesystem operations. Stateful hooks (such as
/// database initialization) and arbitrary Ruby post_install methods are not run.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PostInstallStep {
    MkdirP {
        path: PostInstallPath,
    },
    LinkDir {
        source: PostInstallPath,
        target: PostInstallPath,
    },
    LinkChildren {
        source: PostInstallPath,
        target: PostInstallPath,
        #[serde(default)]
        suffix: String,
    },
    #[serde(other)]
    Unsupported,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PostInstallPath {
    /// An omitted base is valid only with a recognized absolute prefix template.
    #[serde(default)]
    pub base: String,
    pub path: String,
}
