//! SuperChat client core: everything except the UI.

pub mod admin;
pub mod api;
pub mod applock;
pub mod client;
pub mod error;
pub mod logic;
pub mod prefs;
pub mod secrets;
pub mod store;
pub mod tls;

pub use api::{normalize_server_url, Session};
pub use client::{guess_content_type, Client, ClientConfig, ClientEvent, ConnState, SendOptions, UploadState};
pub use error::{ClientError, Result};
pub use prefs::Prefs;
pub use store::{SendState, TimelineItem};

/// Version reported to the server (`X-Client-Version`).
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
