//! Oxygen Not Included: each mod is a folder in the user folder's `mods/Local`, known to the
//! game by that folder's name, and enabled in `mods/mods.json` for each game version it is
//! enabled for (the base game, Spaced Out). UMM lists the mods it deploys there, enabled for
//! both (see [`crate::plugins`]); the user's own entries are kept as the game wrote them,
//! except Steam Workshop items a profile turns on or off, whose entries the game lists too.

use crate::plugins::PluginEntry;
use serde::Deserialize;
use serde_json::value::RawValue;

/// The game versions a mod is enabled for: the base game and Spaced Out.
const VERSIONS: [&str; 2] = ["", "EXPANSION1_ID"];

/// The game's mod platforms, in the order it numbers them.
const PLATFORMS: [&str; 5] = ["Local", "Steam", "Epic", "Rail", "Dev"];

#[derive(Deserialize)]
struct Db<'a> {
    #[serde(borrow, default)]
    mods: Vec<&'a RawValue>,
}

#[derive(Deserialize)]
struct Mod {
    label: Label,
    #[serde(default)]
    enabled: bool,
    #[serde(rename = "enabledForDlc")]
    enabled_for: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Label {
    #[serde(default)]
    distribution_platform: usize,
    id: String,
}

impl Mod {
    /// Its name in UMM's list: the folder of a local mod, `<id>.<platform>` of another (as
    /// the game names those by default).
    fn name(&self) -> String {
        match self.label.distribution_platform {
            0 => self.label.id.clone(),
            p => format!(
                "{}.{}",
                self.label.id,
                PLATFORMS.get(p).copied().unwrap_or("Unknown")
            ),
        }
    }

    fn enabled(&self) -> bool {
        match &self.enabled_for {
            // Older lists say only whether it is enabled.
            None => self.enabled,
            Some(v) => v.iter().any(|d| VERSIONS.contains(&d.as_str())),
        }
    }
}

/// The list's entries as the game wrote them, each with its offset in `text`.
fn entries(text: &str) -> Vec<(usize, &str, Mod)> {
    let body = text.trim_start_matches('\u{feff}');
    let Ok(db) = serde_json::from_str::<Db>(body) else {
        return Vec::new();
    };
    db.mods
        .into_iter()
        .filter_map(|raw| {
            let m = serde_json::from_str::<Mod>(raw.get()).ok()?;
            let at = raw.get().as_ptr() as usize - text.as_ptr() as usize;
            Some((at, raw.get(), m))
        })
        .collect()
}

/// The list's entries.
pub fn read(text: &str) -> Vec<PluginEntry> {
    entries(text)
        .into_iter()
        .map(|(_, _, m)| PluginEntry {
            name: m.name(),
            enabled: m.enabled(),
            owner: None,
        })
        .collect()
}

/// `string.GetHashCode()` as the game's runtime (64-bit Mono) computes it, over UTF-16: the
/// version the game gives a local mod, from its folder's name. An entry of another version
/// makes the game reinstall the mod and ask for a restart.
fn mono_hash(s: &str) -> i32 {
    let (mut h1, mut h2) = (5381i32, 5381i32);
    let units: Vec<u16> = s.encode_utf16().collect();
    for pair in units.chunks(2) {
        h1 = (h1 << 5).wrapping_add(h1) ^ i32::from(pair[0]);
        if let Some(&c) = pair.get(1) {
            h2 = (h2 << 5).wrapping_add(h2) ^ i32::from(c);
        }
    }
    h1.wrapping_add(h2.wrapping_mul(1566083941))
}

/// A local mod's entry, enabled, as the game writes it (indented by four spaces).
fn local(folder: &str, nl: &str) -> String {
    let s = |v: &str| serde_json::Value::String(v.to_string()).to_string();
    let versions = VERSIONS
        .iter()
        .map(|v| format!("        {}", s(v)))
        .collect::<Vec<_>>()
        .join(&format!(",{nl}"));
    [
        "{".to_string(),
        "      \"label\": {".into(),
        "        \"distribution_platform\": 0,".into(),
        format!("        \"id\": {},", s(folder)),
        format!("        \"title\": {},", s(folder)),
        format!("        \"version\": {}", mono_hash(folder)),
        "      },".into(),
        "      \"status\": 1,".into(),
        "      \"enabled\": true,".into(),
        "      \"enabledForDlc\": [".into(),
        versions,
        "      ],".into(),
        "      \"crash_count\": 0,".into(),
        "      \"reinstall_path\": null,".into(),
        format!("      \"staticID\": {}", s(&format!("{folder}.Local"))),
        "    }".into(),
    ]
    .join(nl)
}

/// An entry as the game wrote it, enabled for both game versions or for none: only its
/// `enabled` and `enabledForDlc` values change.
fn set_enabled(raw: &str, on: bool) -> String {
    let mut out = raw.to_string();
    let value_at = |s: &str, key: &str| -> Option<usize> {
        let k = s.find(&format!("\"{key}\""))? + key.len() + 2;
        let colon = k + s[k..].find(':')? + 1;
        Some(colon + (s[colon..].len() - s[colon..].trim_start().len()))
    };
    if let Some(v) = value_at(&out, "enabled") {
        let old = if out[v..].starts_with("true") {
            4
        } else if out[v..].starts_with("false") {
            5
        } else {
            0
        };
        if old > 0 {
            out.replace_range(v..v + old, if on { "true" } else { "false" });
        }
    }
    if let Some(v) = value_at(&out, "enabledForDlc").filter(|&v| out[v..].starts_with('[')) {
        if let Some(len) = out[v..].find(']') {
            let list = if on {
                format!(
                    "[{}]",
                    VERSIONS
                        .iter()
                        .map(|d| format!("\"{d}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                "[]".into()
            };
            out.replace_range(v..v + len + 1, &list);
        }
    }
    out
}

/// The list with these entries. The user's entries are kept as the game wrote them, and the
/// rest of the file with them; UMM's mods are listed enabled.
pub fn render(original: &str, list: &[PluginEntry]) -> String {
    let nl = match original.contains('\n') && !original.contains("\r\n") {
        true => "\n",
        false => "\r\n",
    };
    let theirs = entries(original);
    let items: Vec<String> = list
        .iter()
        .map(|e| {
            let kept = theirs
                .iter()
                .find(|(_, _, m)| e.owner.is_none() && m.name() == e.name);
            match kept {
                Some((_, raw, m)) if m.enabled() == e.enabled => raw.to_string(),
                Some((_, raw, _)) => set_enabled(raw, e.enabled),
                None => local(&e.name, nl),
            }
        })
        .collect();
    if let (Some(first), Some(last)) = (theirs.first(), theirs.last()) {
        // Between entries, what the game puts there.
        let sep = match theirs.get(1) {
            Some((at, ..)) => &original[first.0 + first.1.len()..*at],
            None => ",\r\n    ",
        };
        let sep = match sep.trim_start().starts_with(',') {
            true => sep.replace("\r\n", nl),
            false => format!(",{nl}    "),
        };
        return format!(
            "{}{}{}",
            &original[..first.0],
            items.join(&sep),
            &original[last.0 + last.1.len()..]
        );
    }
    let mods = match items.is_empty() {
        true => "[]".to_string(),
        false => format!("[{nl}    {}{nl}  ]", items.join(&format!(",{nl}    "))),
    };
    format!("{{{nl}  \"version\": 1,{nl}  \"mods\": {mods}{nl}}}")
}

/// The folder in the mods folder (`prefix`, ending in `/`) holding `target`, as spelled.
pub fn mod_folder<'a>(target: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = target.get(prefix.len()..).filter(|_| {
        target
            .get(..prefix.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    })?;
    let (dir, _) = rest.split_once('/')?;
    (!dir.is_empty()).then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_names_as_the_game_does() {
        // Versions the game gave these folders.
        assert_eq!(mono_hash("PerfProbe"), -2037174645);
        assert_eq!(mono_hash("E2eDriver"), 1399985550);
    }

    #[test]
    fn keeps_the_users_entries_as_the_game_wrote_them() {
        let original = "{\r\n  \"version\": 1,\r\n  \"mods\": [\r\n    {\r\n      \"label\": {\r\n        \"distribution_platform\": 1,\r\n        \"id\": \"1838445101\",\r\n        \"title\": \"Theirs\",\r\n        \"version\": 1700000000\r\n      },\r\n      \"status\": 1,\r\n      \"enabled\": true,\r\n      \"enabledForDlc\": [\r\n        \"EXPANSION1_ID\"\r\n      ],\r\n      \"crash_count\": 0,\r\n      \"reinstall_path\": null,\r\n      \"staticID\": \"Theirs\"\r\n    },\r\n    {\r\n      \"label\": {\r\n        \"distribution_platform\": 0,\r\n        \"id\": \"Old\",\r\n        \"title\": \"Old\",\r\n        \"version\": 1\r\n      },\r\n      \"status\": 1,\r\n      \"enabled\": false,\r\n      \"enabledForDlc\": [],\r\n      \"crash_count\": 0,\r\n      \"reinstall_path\": null\r\n    }\r\n  ],\r\n  \"mod_load_in_progress\": false\r\n}";
        let e = read(original);
        assert_eq!(
            e.iter()
                .map(|e| (e.name.as_str(), e.enabled))
                .collect::<Vec<_>>(),
            [("1838445101.Steam", true), ("Old", false)]
        );
        assert_eq!(render(original, &e), original, "written back as it was");

        let mut more = vec![e[0].clone()];
        more.push(PluginEntry {
            name: "Old".into(),
            enabled: true,
            owner: Some("old".into()),
        });
        let out = render(original, &more);
        let back = read(&out);
        assert_eq!(
            back.iter()
                .map(|e| (e.name.as_str(), e.enabled))
                .collect::<Vec<_>>(),
            [("1838445101.Steam", true), ("Old", true)]
        );
        assert!(out.ends_with("\r\n  ],\r\n  \"mod_load_in_progress\": false\r\n}"));
        assert!(out.contains("\r\n    },\r\n    {\r\n      \"label\": {\r\n        \"distribution_platform\": 0,\r\n        \"id\": \"Old\",\r\n        \"title\": \"Old\",\r\n        \"version\": "));
        assert!(!out.contains("\n\n") && !out.replace("\r\n", "").contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mods"][1]["enabledForDlc"][1], "EXPANSION1_ID");
    }

    #[test]
    fn turns_a_workshop_item_on_or_off_in_its_entry() {
        let original = "{\n  \"mods\": [\n    {\n      \"label\": {\n        \"distribution_platform\": 1,\n        \"id\": \"1838445101\"\n      },\n      \"enabled\": false,\n      \"enabledForDlc\": [],\n      \"staticID\": \"Theirs\"\n    }\n  ]\n}";
        let mut e = read(original);
        assert_eq!(
            (e[0].name.as_str(), e[0].enabled),
            ("1838445101.Steam", false)
        );
        e[0].enabled = true;
        let on = render(original, &e);
        assert_eq!(
            on,
            original
                .replace("\"enabled\": false", "\"enabled\": true")
                .replace(
                    "\"enabledForDlc\": []",
                    "\"enabledForDlc\": [\"\", \"EXPANSION1_ID\"]"
                )
        );
        assert!(read(&on)[0].enabled);
        e[0].enabled = false;
        assert_eq!(render(&on, &e), original);
    }

    #[test]
    fn writes_a_list_of_its_own_when_there_is_none() {
        let ours = PluginEntry {
            name: "My \"Mod\"".into(),
            enabled: true,
            owner: Some("m".into()),
        };
        for original in ["", "not json", "{\"version\":1,\"mods\":[]}"] {
            let out = render(original, std::slice::from_ref(&ours));
            let v: serde_json::Value = serde_json::from_str(&out).unwrap();
            assert_eq!(v["version"], 1);
            assert_eq!(v["mods"][0]["label"]["id"], "My \"Mod\"");
            assert_eq!(v["mods"][0]["status"], 1);
            assert_eq!(read(&out).len(), 1);
        }
        assert_eq!(
            render("", &[]),
            "{\r\n  \"version\": 1,\r\n  \"mods\": []\r\n}"
        );
        assert_eq!(
            mod_folder("@user/mods/Local/k/x/y.dll", "@user/mods/local/"),
            Some("k")
        );
        assert_eq!(mod_folder("@user/mods/Local/k", "@user/mods/local/"), None);
    }
}
