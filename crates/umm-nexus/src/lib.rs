//! Nexus Mods: the public GraphQL API for browsing (search, files, updates, recognising an
//! archive by its hash), and `nxm://` links from the website. Files are downloaded on the
//! website; downloading through the API needs Nexus's single sign-on, which comes once Nexus
//! registers UMM.
//!
//! Every request carries UMM's name and version, as Nexus asks of applications.

pub mod graph;
pub mod link;

pub use link::NxmLink;

/// UMM's name in the `Application-Name` header.
pub const APP_NAME: &str = "Universal Mod Manager";

/// The website's host, for links shown to the user.
pub const SITE_URL: &str = "https://www.nexusmods.com";

#[derive(Debug, thiserror::Error)]
pub enum NexusError {
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
