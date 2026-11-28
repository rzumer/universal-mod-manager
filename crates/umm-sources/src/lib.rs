//! Where mods come from: the sites UMM installs mods from, and the network access they share.
//!
//! - [`thunderstore`]: which games Thunderstore covers, its package lists, dependencies,
//!   package layouts and r2modman profile codes.
//! - [`nexus`]: Nexus Mods' public GraphQL API and `nxm://` links.
//! - [`gamebanana`]: GameBanana's public API.
//! - [`workshop`]: the Steam Workshop: Steam's records on this machine, its public item
//!   details and its browse page.
//! - [`net`]: the only place that talks to the network, behind a trait so tests never do.
//!
//! None of them needs an account. They share one error type, [`SourceError`], which names the
//! site it comes from.

pub mod gamebanana;
pub mod net;
pub mod nexus;
pub mod thunderstore;
pub mod workshop;

use std::fmt;

/// A site UMM gets mods from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Site {
    Thunderstore,
    Nexus,
    GameBanana,
    SteamWorkshop,
}

impl fmt::Display for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Site::Thunderstore => "Thunderstore",
            Site::Nexus => "Nexus Mods",
            Site::GameBanana => "GameBanana",
            Site::SteamWorkshop => "the Steam Workshop",
        })
    }
}

impl Site {
    /// How long its limit on requests lasts: Nexus allows a few thousand a day.
    fn retry_after(self) -> &'static str {
        match self {
            Site::Nexus => "in an hour",
            Site::Thunderstore | Site::GameBanana => "in a few minutes",
            Site::SteamWorkshop => "in a minute",
        }
    }

    pub fn not_found(self, what: impl Into<String>) -> SourceError {
        SourceError::NotFound {
            site: self,
            what: what.into(),
        }
    }

    pub fn rate_limited(self) -> SourceError {
        SourceError::RateLimited { site: self }
    }

    pub fn network(self, url: impl Into<String>, message: impl Into<String>) -> SourceError {
        SourceError::Network {
            site: self,
            url: url.into(),
            message: message.into(),
        }
    }

    pub fn data(self, what: impl Into<String>, message: impl Into<String>) -> SourceError {
        SourceError::Data {
            site: self,
            what: what.into(),
            message: message.into(),
        }
    }

    /// A request to the site that failed.
    pub fn net(self, e: net::NetError) -> SourceError {
        self.network(e.url, e.message)
    }
}

/// What went wrong getting mods or their details from a site.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("{site} has no {what}")]
    NotFound { site: Site, what: String },
    #[error("{site} says UMM made too many requests; try again {}", .site.retry_after())]
    RateLimited { site: Site },
    #[error("could not reach {site} ({url}): {message}")]
    Network {
        site: Site,
        url: String,
        message: String,
    },
    #[error("unexpected data from {site} ({what}): {message}")]
    Data {
        site: Site,
        what: String,
        message: String,
    },
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = SourceError> = std::result::Result<T, E>;

/// The path of a link to one of `host`'s pages, `http` or `https`, with or without `www.` or
/// one of `subdomains`, as its parts without the query or fragment:
/// `https://www.nexusmods.com/skyrim/mods/1?tab=files` → `skyrim`, `mods`, `1`.
pub(crate) fn page_path<'a>(url: &'a str, host: &str, subdomains: &[&str]) -> Option<Vec<&'a str>> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let rest = ["www."]
        .iter()
        .chain(subdomains)
        .find_map(|s| rest.strip_prefix(s))
        .unwrap_or(rest);
    let rest = rest.strip_prefix(host)?.strip_prefix('/')?;
    Some(path_parts(rest))
}

/// A path's parts, without its query or fragment, or empty parts (`a//b/?q` → `a`, `b`).
pub(crate) fn path_parts(path: &str) -> Vec<&str> {
    let path = path.split(['?', '#']).next().unwrap_or("");
    path.split('/').filter(|p| !p.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_name_their_site() {
        assert_eq!(
            Site::Nexus.rate_limited().to_string(),
            "Nexus Mods says UMM made too many requests; try again in an hour"
        );
        assert_eq!(
            Site::GameBanana.not_found("mod 7").to_string(),
            "GameBanana has no mod 7"
        );
    }

    #[test]
    fn page_paths_drop_scheme_subdomain_query_and_fragment() {
        let nexus = |u| page_path(u, "nexusmods.com", &["next."]);
        assert_eq!(
            nexus(" https://www.nexusmods.com/skyrim/mods/1?tab=files#x "),
            Some(vec!["skyrim", "mods", "1"])
        );
        assert_eq!(nexus("http://next.nexusmods.com/a/"), Some(vec!["a"]));
        assert_eq!(nexus("https://nexusmods.com.evil.example/a"), None);
        assert_eq!(nexus("https://evil.example/nexusmods.com/a"), None);
        assert_eq!(nexus("ftp://nexusmods.com/a"), None);
    }
}
