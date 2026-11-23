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

/// A collection's page on the website.
pub fn collection_url(domain: &str, slug: &str) -> String {
    format!("{SITE_URL}/games/{domain}/collections/{slug}")
}

/// Reads a collection link (`https://www.nexusmods.com/games/<game>/collections/<slug>`,
/// the older `next.nexusmods.com/<game>/collections/<slug>`, or the website's install button,
/// `nxm://<game>/collections/<slug>/revisions/<n>`): the game's domain name, the
/// collection's slug and the revision, if it names one.
pub fn parse_collection_url(url: &str) -> Option<(String, String, Option<u64>)> {
    let url = url.trim();
    let rest = match url.get(..6) {
        Some(s) if s.eq_ignore_ascii_case("nxm://") => &url[6..],
        _ => {
            let rest = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))?;
            let rest = rest
                .strip_prefix("www.")
                .or_else(|| rest.strip_prefix("next."))
                .unwrap_or(rest);
            rest.strip_prefix("nexusmods.com/")?
        }
    };
    let path = rest.split(['?', '#']).next()?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let parts = match parts.as_slice() {
        ["games", rest @ ..] => rest,
        all => all,
    };
    let slug_ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    match parts {
        [game, "collections", slug, more @ ..] if link::domain_ok(game) && slug_ok(slug) => {
            let revision = match more {
                ["revisions", n, ..] => n.parse().ok(),
                _ => None,
            };
            Some((game.to_string(), slug.to_string(), revision))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_collection_links() {
        assert_eq!(
            parse_collection_url(
                "https://www.nexusmods.com/games/stardewvalley/collections/g14kxi"
            ),
            Some(("stardewvalley".into(), "g14kxi".into(), None))
        );
        assert_eq!(
            parse_collection_url(
                "https://next.nexusmods.com/skyrimspecialedition/collections/qdurkx/revisions/12?tab=mods"
            ),
            Some(("skyrimspecialedition".into(), "qdurkx".into(), Some(12)))
        );
        assert_eq!(
            parse_collection_url("nxm://stardewvalley/collections/g14kxi/revisions/70"),
            Some(("stardewvalley".into(), "g14kxi".into(), Some(70)))
        );
        assert_eq!(
            parse_collection_url("https://www.nexusmods.com/stardewvalley/mods/2400"),
            None
        );
        assert_eq!(
            parse_collection_url("nxm://stardewvalley/mods/1/files/2"),
            None
        );
        assert_eq!(
            collection_url("stardewvalley", "g14kxi"),
            "https://www.nexusmods.com/games/stardewvalley/collections/g14kxi"
        );
    }

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
