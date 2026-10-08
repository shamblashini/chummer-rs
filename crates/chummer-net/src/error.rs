use crate::frame::FrameError;

/// Errors from the networking layer.
///
/// iroh's own error types are flattened to strings so they do not leak into
/// the public API (they are not covered by our semver).
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("could not start the network endpoint: {0}")]
    Bind(String),
    #[error("could not connect: {0}")]
    Connect(String),
    #[error("connection lost: {0}")]
    Connection(String),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("the GM's app refused to let us join: {0}")]
    Denied(crate::campaign::DenyReason),
    #[error("the other side reported an error: {0}")]
    Remote(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error(transparent)]
    Seal(#[from] crate::seal::SealError),
    #[error(transparent)]
    Mailbox(#[from] crate::mailbox::MailboxError),
}

impl NetError {
    pub(crate) fn connection(e: impl std::fmt::Display) -> Self {
        NetError::Connection(e.to_string())
    }
}
