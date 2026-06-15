use std::path::PathBuf;

/// Result alias used throughout the engine.
pub type Result<T> = std::result::Result<T, Error>;

/// All error cases the engine can surface. Kept coarse on purpose — callers
/// (CLI / GUI / MCP) only need to distinguish a handful of situations
/// (bad password, unsupported format, IO, corrupt data).
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported or unrecognized archive format{}", .0.as_ref().map(|p| format!(" for {}", p.display())).unwrap_or_default())]
    UnsupportedFormat(Option<PathBuf>),

    #[error("this archive is encrypted — a password is required")]
    PasswordRequired,

    #[error("incorrect password")]
    BadPassword,

    #[error("archive is corrupt or truncated: {0}")]
    Corrupt(String),

    #[error("refusing to extract entry outside destination (path traversal): {0}")]
    PathTraversal(String),

    #[error("creating this format is not supported (it is proprietary or read-only): {0}")]
    CreateUnsupported(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
    pub fn corrupt(msg: impl Into<String>) -> Self {
        Error::Corrupt(msg.into())
    }
}
