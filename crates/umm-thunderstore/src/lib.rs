//! Thunderstore integration: what UMM needs from thunderstore.io, without an account.
//!
//! - [`ecosystem`]: which games Thunderstore covers, by Steam app id, and how their packages
//!   are laid out.
//! - [`http`]: the only place that talks to the network, behind a trait so tests never do.
//! - [`cache`]: JSON kept under the data folder, refreshed when old, used as is when offline.

pub mod cache;
pub mod ecosystem;
pub mod http;
pub mod index;
pub mod layout;
pub mod profile;
pub mod resolve;

pub use ecosystem::{Ecosystem, EcosystemGame, InstallRule, TrackingMethod};
pub use http::{Http, UreqHttp};

pub const BASE_URL: &str = "https://thunderstore.io";

#[derive(Debug, thiserror::Error)]
pub enum TsError {
    #[error("could not reach Thunderstore ({url}): {message}")]
    Network { url: String, message: String },
    #[error("unexpected data from Thunderstore ({what}): {message}")]
    Data { what: String, message: String },
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = TsError> = std::result::Result<T, E>;

pub(crate) fn io_err(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> TsError {
    let context = context.into();
    move |source| TsError::Io { context, source }
}
