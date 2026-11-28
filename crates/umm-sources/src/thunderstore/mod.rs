//! Thunderstore integration: what UMM needs from thunderstore.io, without an account.
//!
//! - [`ecosystem`]: which games Thunderstore covers, by Steam app id, and how their packages
//!   are laid out.
//! - [`cache`]: JSON kept under the data folder, refreshed when old, used as is when offline.
//!
//! Network access is [`crate::net`]'s.

pub mod cache;
pub mod ecosystem;
pub mod index;
pub mod layout;
pub mod profile;
pub mod resolve;

pub use ecosystem::{Ecosystem, EcosystemGame, InstallRule, TrackingMethod};

pub const BASE_URL: &str = "https://thunderstore.io";

use crate::Site;
pub use crate::{Result, SourceError};

/// A failed request to Thunderstore.
pub(crate) fn net(e: crate::net::NetError) -> SourceError {
    Site::Thunderstore.net(e)
}

pub(crate) fn io_err(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> SourceError {
    let context = context.into();
    move |source| SourceError::Io { context, source }
}
