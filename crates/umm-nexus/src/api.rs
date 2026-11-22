//! The v1 REST API (`https://api.nexusmods.com/v1/...`), for what needs an account: checking
//! the user's API key, and download links. Browsing goes through [`crate::graph`] instead.

use crate::{NexusError, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use umm_net::{Http, NetError, Progress};

pub const API_URL: &str = "https://api.nexusmods.com/v1";

/// The name Nexus sees in every request's `Application-Name` header.
pub const APP_NAME: &str = "Universal Mod Manager";

/// The signed-in account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub user_id: u64,
    pub name: String,
    #[serde(default)]
    pub is_premium: bool,
    #[serde(default)]
    pub is_supporter: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadLink {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "URI")]
    pub uri: String,
}

/// The API, as the user with `key`.
pub struct Client<'a> {
    http: &'a dyn Http,
    key: &'a str,
    version: &'a str,
}

impl<'a> Client<'a> {
    /// `version`: UMM's version, sent as `Application-Version`.
    pub fn new(http: &'a dyn Http, key: &'a str, version: &'a str) -> Self {
        Client { http, key, version }
    }

    fn get<T: DeserializeOwned>(&self, path: &str, what: &str) -> Result<T> {
        let url = format!("{API_URL}{path}");
        let body = self
            .http
            .get_with(
                &url,
                &[
                    ("apikey", self.key),
                    ("Application-Name", APP_NAME),
                    ("Application-Version", self.version),
                    ("Accept", "application/json"),
                ],
            )
            .map_err(|e| api_err(e, what))?;
        serde_json::from_slice(&body).map_err(|e| NexusError::Data {
            what: what.to_string(),
            message: e.to_string(),
        })
    }

    /// Checks the key and says whose it is.
    pub fn validate(&self) -> Result<User> {
        self.get("/users/validate.json", "account")
    }

    /// Where to download a file. Free accounts need the key and expiry from an `nxm://`
    /// link; without them only Premium accounts get links.
    pub fn download_links(
        &self,
        domain: &str,
        mod_id: u64,
        file_id: u64,
        credentials: Option<(&str, u64)>,
    ) -> Result<Vec<DownloadLink>> {
        let mut path = format!("/games/{domain}/mods/{mod_id}/files/{file_id}/download_link.json");
        if let Some((key, expires)) = credentials {
            path.push_str(&format!("?key={key}&expires={expires}"));
        }
        let what = format!("file {file_id}");
        match self.get::<Vec<DownloadLink>>(&path, &what) {
            // Without the website's key, refusal means the account is not Premium; with it,
            // that the key is no longer good.
            Err(NexusError::PremiumOnly) if credentials.is_some() => Err(NexusError::LinkExpired),
            Ok(links) if links.is_empty() => {
                Err(NexusError::NotFound(format!("download for file {file_id}")))
            }
            r => r,
        }
    }

    /// Downloads a file to `dest` from the first of its links.
    pub fn download(
        &self,
        links: &[DownloadLink],
        dest: &std::path::Path,
        progress: Progress,
    ) -> Result<()> {
        let link = links
            .first()
            .ok_or_else(|| NexusError::NotFound("download link".into()))?;
        self.http
            .download(&link.uri, dest, progress)
            .map_err(|e| NexusError::Network {
                url: link.uri.clone(),
                message: e.message,
            })
    }
}

fn api_err(e: NetError, what: &str) -> NexusError {
    match e.status {
        Some(401) => NexusError::BadKey,
        Some(403) => NexusError::PremiumOnly,
        Some(410) => NexusError::LinkExpired,
        Some(404) => NexusError::NotFound(what.to_string()),
        Some(429) => NexusError::RateLimited,
        _ => NexusError::Network {
            url: e.url,
            message: e.message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use umm_net::FakeHttp;

    #[test]
    fn sends_the_key_and_reads_answers() {
        let http = FakeHttp::default().with(
            "https://api.nexusmods.com/v1/users/validate.json",
            r#"{"user_id": 42, "key": "k", "name": "Ann", "is_premium": false, "is_supporter": false, "email": "a@b"}"#,
        );
        let c = Client::new(&http, "secret", "0.2.0");
        let u = c.validate().unwrap();
        assert_eq!(
            (u.user_id, u.name.as_str(), u.is_premium),
            (42, "Ann", false)
        );
        let headers = http.headers.lock().unwrap()[0].1.clone();
        assert!(headers.contains(&("apikey".into(), "secret".into())));
        assert!(headers.contains(&("Application-Name".into(), APP_NAME.into())));
        assert!(headers.contains(&("Application-Version".into(), "0.2.0".into())));
    }

    #[test]
    fn errors_say_what_to_do() {
        let base = "https://api.nexusmods.com/v1/games/g/mods/1/files/2/download_link.json";
        let http = FakeHttp::default()
            .with_error(
                "https://api.nexusmods.com/v1/users/validate.json",
                401,
                "{}",
            )
            .with_error(base, 403, r#"{"message":"premium only"}"#)
            .with_error(&format!("{base}?key=k&expires=9"), 403, "{}")
            .with(
                "https://api.nexusmods.com/v1/games/g/mods/1/files/3/download_link.json",
                "[]",
            );
        let c = Client::new(&http, "bad", "0.2.0");
        assert!(matches!(c.validate(), Err(NexusError::BadKey)));
        assert!(matches!(
            c.download_links("g", 1, 2, None),
            Err(NexusError::PremiumOnly)
        ));
        assert!(matches!(
            c.download_links("g", 1, 2, Some(("k", 9))),
            Err(NexusError::LinkExpired)
        ));
        assert!(matches!(
            c.download_links("g", 1, 3, None),
            Err(NexusError::NotFound(_))
        ));
    }
}
