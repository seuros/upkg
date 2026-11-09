use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Why a manifest could not be loaded.
///
/// No variant quotes the file: a parse error says where (line and column)
/// and what (a key name at most), so a manifest from an untrusted checkout
/// can't smuggle its content into a log through one.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The file could not be read.
    Read { path: PathBuf, source: io::Error },
    /// The file is not one upkg reads: a symlink, a directory or a device
    /// where [`Manifest::load_from_dir`](super::Manifest::load_from_dir)
    /// wants a regular file, or larger than [`MAX_LEN`](super::MAX_LEN).
    Refused { path: PathBuf, reason: &'static str },
    /// Not TOML, an unknown key, a wrong type, or an unusable package name.
    Invalid {
        /// The file, when the manifest came from one.
        path: Option<PathBuf>,
        message: String,
    },
}

impl Error {
    /// Whether there was no file at all: [`Manifest::load`](super::Manifest::load)
    /// on a path that doesn't exist.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Read { source, .. } if source.kind() == io::ErrorKind::NotFound)
    }

    /// What went wrong, without the file's path: for a log that names the
    /// file itself, and has no business showing where it sits on the host.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::Read { source, .. } => source.to_string(),
            Self::Refused { reason, .. } => (*reason).to_string(),
            Self::Invalid { message, .. } => message.clone(),
        }
    }

    pub(super) fn at(self, file: &Path) -> Self {
        match self {
            Self::Invalid {
                path: None,
                message,
            } => Self::Invalid {
                path: Some(file.to_path_buf()),
                message,
            },
            other => other,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, .. }
            | Self::Refused { path, .. }
            | Self::Invalid {
                path: Some(path), ..
            } => write!(f, "{}: {}", path.display(), self.detail()),
            Self::Invalid { path: None, .. } => f.write_str(&self.detail()),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Refused { .. } | Self::Invalid { .. } => None,
        }
    }
}
