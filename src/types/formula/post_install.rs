use serde::{Deserialize, Deserializer, Serialize};

/// The step types upkg applies.
///
/// Bump it whenever a variant learns to do something new: receipts stamped
/// with an older schema recorded steps that upkg skipped then, so a reinstall
/// refetches the formula's steps instead of replaying them.
pub const STEPS_SCHEMA: u32 = 2;

/// Formulas whose `run` step upkg executes.
///
/// Each script was read, and does only what Homebrew's own `post_install`
/// would. `ca-certificates` builds `etc/ca-certificates/cert.pem` from the
/// system trust store and Mozilla's bundle, the policy Homebrew maintains.
pub const TRUSTED_SCRIPTS: &[&str] = &["ca-certificates"];

/// Homebrew's declarative filesystem operations. Stateful hooks (such as
/// database initialization) and arbitrary Ruby `post_install` methods are not
/// run, nor are `run` steps outside [`TRUSTED_SCRIPTS`].
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
    /// `libexec/<command>` from the keg, each argument a path template:
    /// `ca-certificates`' bundle builder. Steps with guards, an environment,
    /// a working directory or redirections are kept as unsupported.
    Run {
        command: PostInstallPath,
        #[serde(default)]
        args: Vec<String>,
    },
    /// A step upkg does not run, by its Homebrew type (`run`, `touch`...).
    /// Receipts written before kinds were kept hold none.
    Unsupported {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
    },
}

impl PostInstallStep {
    /// What upkg skips of this step for `formula`, named for a warning;
    /// `None` when applied.
    #[must_use]
    pub fn skipped(&self, formula: &str) -> Option<&str> {
        match self {
            Self::Unsupported { kind } => Some(kind.as_deref().unwrap_or("unknown")),
            Self::Symlink {
                source_glob: true, ..
            } => Some("symlink (glob)"),
            Self::Run { .. } if !TRUSTED_SCRIPTS.contains(&formula) => Some("run"),
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
        "run",
        "unsupported",
    ];

    /// The only fields a [`PostInstallStep::Run`] may carry: anything else
    /// (guards, env, chdir, stdin/stdout paths) changes what running it means.
    const RUN_FIELDS: &[&str] = &["type", "command", "args"];
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
            let unsupported = match kind {
                Some("run") => value.as_object().is_some_and(|fields| {
                    fields
                        .keys()
                        .any(|field| !PostInstallStep::RUN_FIELDS.contains(&field.as_str()))
                }),
                Some(kind) => !PostInstallStep::KINDS.contains(&kind),
                None => false,
            };
            if unsupported {
                return Ok(PostInstallStep::Unsupported {
                    kind: kind.map(str::to_owned),
                });
            }
            serde_json::from_value(value).map_err(serde::de::Error::custom)
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
