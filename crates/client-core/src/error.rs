use proto::ErrorCode;

#[derive(Debug, Clone, thiserror::Error)]
pub enum ClientError {
    /// The server answered with an API error.
    #[error("{message}")]
    Api { code: ErrorCode, message: String },
    /// Could not reach the server (offline, DNS, TLS failure, timeout).
    #[error("network error: {0}")]
    Network(String),
    /// Local problem (disk, cache, bad input).
    #[error("{0}")]
    Local(String),
    #[error("not signed in")]
    NotSignedIn,
}

pub type Result<T> = std::result::Result<T, ClientError>;

impl ClientError {
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            ClientError::Api { code, .. } => Some(*code),
            _ => None,
        }
    }
    /// Worth retrying later (network failure, server overloaded or restarting).
    pub fn is_retryable(&self) -> bool {
        match self {
            ClientError::Network(_) => true,
            ClientError::Api { code, .. } => matches!(
                code,
                ErrorCode::RateLimited | ErrorCode::Internal | ErrorCode::StorageError | ErrorCode::TokenExpired
            ),
            _ => false,
        }
    }
    pub fn requires_wipe(&self) -> bool {
        self.code().map(|c| c.requires_wipe()).unwrap_or(false)
    }
}

impl From<rusqlite::Error> for ClientError {
    fn from(e: rusqlite::Error) -> Self {
        ClientError::Local(format!("cache: {e}"))
    }
}
impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        ClientError::Local(format!("io: {e}"))
    }
}
impl From<serde_json::Error> for ClientError {
    fn from(e: serde_json::Error) -> Self {
        ClientError::Local(format!("json: {e}"))
    }
}
