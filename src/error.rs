//! Errors for talking to the Blender add-on, with messages meant to be read by an MCP client (a model).

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error(
        "Blender is not running or the add-on is not started: open Blender, then N panel > Omnix > Start. \
         (If the add-on is not installed yet, run `omnix-blender install-addon`.)"
    )]
    NotRunning,
    #[error("cannot talk to Blender: {0}")]
    Io(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error(
        "the Blender add-on speaks protocol v{theirs} but this server needs v{ours}; \
         run `omnix-blender install-addon` to update the add-on"
    )]
    VersionMismatch { ours: u32, theirs: u64 },
    #[error("{message}")]
    Remote { code: String, message: String },
    #[error(
        "Blender did not answer within {0:?}; it may be busy (a render or a long script blocks it)"
    )]
    Timeout(Duration),
    #[error("{0}")]
    BadPath(String),
}

impl LinkError {
    /// Stable code to show alongside the message.
    pub fn code(&self) -> &str {
        match self {
            Self::NotRunning => "not_connected",
            Self::Io(_) => "io_error",
            Self::Protocol(_) => "protocol_error",
            Self::VersionMismatch { .. } => "version_mismatch",
            Self::Remote { code, .. } => code,
            Self::Timeout(_) => "timeout",
            Self::BadPath(_) => "bad_socket_path",
        }
    }
}
