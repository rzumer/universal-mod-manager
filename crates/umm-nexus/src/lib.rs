//! Nexus Mods: the public GraphQL API for browsing (search, files, updates, recognising an
//! archive by its hash), the v1 REST API for what needs an account (the API key, Premium
//! downloads), and `nxm://` links from the website.
//!
//! Every request carries UMM's name and version, as Nexus asks of applications; only the v1
//! requests carry the user's API key.

pub mod api;
pub mod graph;
pub mod link;

pub use api::{Client, DownloadLink, User};
pub use link::NxmLink;

/// The website's host, for links shown to the user.
pub const SITE_URL: &str = "https://www.nexusmods.com";
/// Where a user creates a personal API key.
pub const API_KEY_PAGE: &str = "https://www.nexusmods.com/users/myaccount?tab=api";

#[derive(Debug, thiserror::Error)]
pub enum NexusError {
    /// The API key was refused (HTTP 401).
    #[error("Nexus Mods did not accept the API key; check it, or create a new one")]
    BadKey,
    /// Downloading needs a Premium account, or a link from the website (HTTP 403).
    #[error("Nexus Mods only lets Premium accounts download from inside an app. Use the Mod Manager Download button on the mod's Files page instead")]
    PremiumOnly,
    /// The download link from the website has expired.
    #[error("the download link from Nexus Mods has expired; click Mod Manager Download again")]
    LinkExpired,
    #[error("Nexus Mods has no {0}")]
    NotFound(String),
    /// Too many requests: Nexus allows a few thousand a day.
    #[error("Nexus Mods says UMM made too many requests; try again in an hour")]
    RateLimited,
    #[error("could not reach Nexus Mods ({url}): {message}")]
    Network { url: String, message: String },
    #[error("unexpected data from Nexus Mods ({what}): {message}")]
    Data { what: String, message: String },
}

pub type Result<T, E = NexusError> = std::result::Result<T, E>;

/// The game's page on the website.
pub fn game_url(domain: &str) -> String {
    format!("{SITE_URL}/{domain}")
}

/// A mod's page on the website.
pub fn mod_url(domain: &str, mod_id: u64) -> String {
    format!("{SITE_URL}/{domain}/mods/{mod_id}")
}

/// A mod's Files tab, where the Mod Manager Download buttons are.
pub fn files_url(domain: &str, mod_id: u64) -> String {
    format!("{SITE_URL}/{domain}/mods/{mod_id}?tab=files")
}

/// Reads a mod page link (`https://www.nexusmods.com/<game>/mods/<id>`, with or without
/// `www.`, a query or a fragment): the game's domain name and the mod id.
pub fn parse_mod_url(url: &str) -> Option<(String, u64)> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))?;
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = rest.strip_prefix("nexusmods.com/")?;
    let path = rest.split(['?', '#']).next()?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [game, "mods", id, ..] if link::domain_ok(game) => {
            Some((game.to_string(), id.parse().ok()?))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_mod_page_links() {
        assert_eq!(
            parse_mod_url("https://www.nexusmods.com/stardewvalley/mods/2400?tab=files"),
            Some(("stardewvalley".into(), 2400))
        );
        assert_eq!(
            parse_mod_url("nexusmods.com/stardewvalley/mods/2400"),
            None,
            "a scheme is needed"
        );
        assert_eq!(
            parse_mod_url("https://nexusmods.com/skyrimspecialedition/mods/266/"),
            Some(("skyrimspecialedition".into(), 266))
        );
        assert!(parse_mod_url("https://www.nexusmods.com/stardewvalley/mods/abc").is_none());
        assert!(parse_mod_url("https://evil.example/stardewvalley/mods/1").is_none());
    }
}
