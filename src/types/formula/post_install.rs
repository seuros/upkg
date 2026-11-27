use serde::{Deserialize, Deserializer, Serialize};

/// The step types upkg applies.
///
/// Bump it whenever a variant learns to do something new: receipts stamped
/// with an older schema recorded steps that upkg skipped then, so a reinstall
/// refetches the formula's steps instead of replaying them.
pub const STEPS_SCHEMA: u32 = 1;

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
    /// One file link, `target` -> `source`: `openssl@3`'s `cert.pem` from
    /// `ca-certificates`. `force` replaces what is already at `target`.
    Symlink {
        source: PostInstallPath,
        target: PostInstallPath,
        #[serde(default)]
        force: bool,
        /// `source` is a shell glob; upkg skips such steps.
        #[serde(default)]
        source_glob: bool,
    },
    /// A step upkg does not run, by its Homebrew type (`run`, `touch`...).
    /// Receipts written before kinds were kept hold none.
    Unsupported {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
    },
}

impl PostInstallStep {
    /// What upkg skips of this step, named for a warning; `None` when applied.
    #[must_use]
    pub fn skipped(&self) -> Option<&str> {
        match self {
            Self::Unsupported { kind } => Some(kind.as_deref().unwrap_or("unknown")),
            Self::Symlink {
                source_glob: true, ..
            } => Some("symlink (glob)"),
            _ => None,
        }
    }

    /// The Homebrew types parsed into a variant; anything else is kept as
    /// [`PostInstallStep::Unsupported`].
    const KINDS: &[&str] = &[
        "mkdir_p",
        "link_dir",
        "link_children",
        "symlink",
        "unsupported",
    ];
}

/// Steps as Homebrew publishes them, unknown types kept as
/// [`PostInstallStep::Unsupported`] by their type.
///
/// # Errors
///
/// When the input is not a list, or a step of a known type is malformed.
pub fn deserialize_steps<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<PostInstallStep>, D::Error> {
    Vec::<serde_json::Value>::deserialize(deserializer)?
        .into_iter()
        .map(|value| {
            let kind = value.get("type").and_then(|kind| kind.as_str());
            match kind {
                Some(kind) if !PostInstallStep::KINDS.contains(&kind) => {
                    Ok(PostInstallStep::Unsupported {
                        kind: Some(kind.to_owned()),
                    })
                }
                _ => serde_json::from_value(value).map_err(serde::de::Error::custom),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PostInstallPath {
    /// An omitted base is valid only with a recognized absolute prefix template.
    #[serde(default)]
    pub base: String,
    pub path: String,
}
