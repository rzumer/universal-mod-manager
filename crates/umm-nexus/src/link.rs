//! `nxm://` links, which the website's Mod Manager Download button opens:
//! `nxm://<game>/mods/<mod id>/files/<file id>?key=<key>&expires=<unix time>&user_id=<id>`.
//! The key and expiry let a free account download that one file.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NxmLink {
    /// The game's domain name on Nexus (`stardewvalley`).
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub key: Option<String>,
    /// Unix time the key stops working.
    pub expires: Option<u64>,
    pub user_id: Option<u64>,
}

pub const LINK_SCHEME: &str = "nxm";

/// Game domain names are lower-case letters, digits and a few separators.
pub(crate) fn domain_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Keys from the website are short tokens; anything else is refused rather than sent on.
fn key_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '='))
}

impl NxmLink {
    /// Reads an `nxm://` file link. Collection links and anything malformed give `None`.
    pub fn parse(url: &str) -> Option<NxmLink> {
        let url = url.trim();
        let rest = url
            .get(..6)
            .filter(|s| s.eq_ignore_ascii_case("nxm://"))
            .map(|_| &url[6..])?;
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let [game, "mods", mod_id, "files", file_id] = parts.as_slice() else {
            return None;
        };
        let domain = game.to_ascii_lowercase();
        if !domain_ok(&domain) {
            return None;
        }
        let mut link = NxmLink {
            domain,
            mod_id: mod_id.parse().ok()?,
            file_id: file_id.parse().ok()?,
            key: None,
            expires: None,
            user_id: None,
        };
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            match k {
                "key" if key_ok(v) => link.key = Some(v.to_string()),
                "key" => return None,
                "expires" => link.expires = Some(v.parse().ok()?),
                "user_id" => link.user_id = v.parse().ok(),
                _ => {}
            }
        }
        Some(link)
    }

    /// The key and expiry, when the link has both.
    pub fn credentials(&self) -> Option<(&str, u64)> {
        Some((self.key.as_deref()?, self.expires?))
    }

    /// The key has expired, as of `now` (Unix time).
    pub fn expired(&self, now: u64) -> bool {
        self.expires.is_some_and(|e| e <= now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_file_links() {
        let l = NxmLink::parse(
            "nxm://stardewvalley/mods/2400/files/9566?key=AbC-12_x&expires=1700000000&user_id=42",
        )
        .unwrap();
        assert_eq!(
            l,
            NxmLink {
                domain: "stardewvalley".into(),
                mod_id: 2400,
                file_id: 9566,
                key: Some("AbC-12_x".into()),
                expires: Some(1700000000),
                user_id: Some(42),
            }
        );
        assert_eq!(l.credentials(), Some(("AbC-12_x", 1700000000)));
        assert!(l.expired(1700000000) && !l.expired(1699999999));

        // Premium links may come without a key; the scheme's case does not matter.
        let p = NxmLink::parse("NXM://SkyrimSpecialEdition/mods/266/files/1000").unwrap();
        assert_eq!(
            (p.domain.as_str(), p.credentials()),
            ("skyrimspecialedition", None)
        );
    }

    #[test]
    fn refuses_other_links() {
        for bad in [
            "nxm://skyrimspecialedition/collections/abc/revisions/3",
            "nxm://stardewvalley/mods/x/files/1",
            "nxm://../mods/1/files/2",
            "nxm://stardewvalley/mods/1/files/2?key=a%2Fb",
            "nxm://stardewvalley/mods/1/files/2?expires=soon",
            "https://www.nexusmods.com/stardewvalley/mods/1",
        ] {
            assert!(NxmLink::parse(bad).is_none(), "{bad}");
        }
    }
}
