//! DELTAMOD packages: GameMaker mods (DELTARUNE, UNDERTALE, ...) made for the Deltamod and
//! G3M managers. Beside its files, a package has a `modding.xml` listing what each does,
//! `<patch type="xdelta" patch="./ch2.xdelta" to="./chapter2_windows/data.win" />`, and its
//! details in `meta.toml` (or `meta.json`, or the older `_deltamodInfo.json`): name, version,
//! the game version it was made for (`deltaruneTargetVersion`).
//!
//! UMM follows the list: an `xdelta` patch goes to its `to` file first (still tried, see
//! [`crate::patching::delta`]); an `override` or `copy` file is deployed at its `to` path; the package's
//! own files stay out of the game folder. `csx` scripts and `g3mpatch` files go to their `to`
//! file, applied with G3MTool (see [`crate::patching::delta`]).
//!
//! Also here, for GameMaker games with several data files (a DELTARUNE chapter each): a mod's
//! replacement data file goes where the game's data file with the same name is (its `GEN8`
//! display name, "DELTARUNE Chapter 2"), whatever folder the mod has it in.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use umm_deploy::{Plan, Skipped};
use umm_gamedef::gamemaker;
use umm_gamedef::RelPath;

/// The package's own files, never deployed.
const PACKAGE_FILES: &[&str] = &[
    "modding.xml",
    "meta.toml",
    "meta.json",
    "_deltamodinfo.json",
    "__deltaid.json",
    "__variant",
    "_icon.png",
    "icon.png",
];

/// What a mod's `modding.xml` says, for the deploy plan.
#[derive(Debug, Clone, Default)]
pub(crate) struct Manifest {
    /// Each delta patch (by its key in the plan) and the file it is for.
    pub targets: HashMap<String, RelPath>,
    /// The game version the package says it was made for.
    pub game_version: Option<String>,
}

/// A path as the manifest writes it (`./chapter1_windows/data.win`, `.\x.xdelta`), relative.
fn clean(p: &str) -> Option<RelPath> {
    let mut s = p.trim().replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    RelPath::parse(s.trim_start_matches('/')).ok()
}

/// The `key="value"` attributes of a tag.
fn attributes(tag: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = tag;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq]
            .rsplit(|c: char| c.is_whitespace() || c == '<')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let after = rest[eq + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let body = &after[1..];
        let Some(end) = body.find(quote) else {
            break;
        };
        out.insert(key, xml_unescape(&body[..end]));
        rest = &body[end + 1..];
    }
    out
}

fn xml_unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The `<patch>` entries of a `modding.xml`: `(type, patch, to)`, in order. The file is a
/// list of tags, with or without an XML root around them.
pub(crate) fn parse_modding_xml(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.to_ascii_lowercase().find("<patch") {
        let tail = &rest[i..];
        // Not `<patches>`.
        if tail[6..].starts_with(|c: char| c.is_ascii_alphanumeric()) {
            rest = &tail[6..];
            continue;
        }
        let end = tail.find('>').unwrap_or(tail.len());
        let a = attributes(&tail[6..end]);
        if let (Some(t), Some(p), Some(to)) = (a.get("type"), a.get("patch"), a.get("to")) {
            out.push((t.trim().to_ascii_lowercase(), p.clone(), to.clone()));
        }
        rest = &tail[end.min(tail.len())..];
        if rest.is_empty() {
            break;
        }
    }
    out
}

/// The game version a package's details say it was made for.
fn game_version(dir: &[(String, std::path::PathBuf)]) -> Option<String> {
    for (name, source) in dir {
        let text = fs::read_to_string(source).ok()?;
        let version = match name.as_str() {
            "meta.toml" => text
                .parse::<toml::Table>()
                .ok()
                .and_then(|t| t.get("deltaruneTargetVersion")?.as_str().map(String::from)),
            "meta.json" | "_deltamodinfo.json" => serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("deltaruneTargetVersion")?.as_str().map(String::from)),
            _ => None,
        };
        if let Some(v) = version.filter(|v| !v.trim().is_empty()) {
            return Some(v);
        }
    }
    None
}

/// Applies a mod's `modding.xml`, if it has one, to the plan: its package files left out,
/// `override` and `copy` files moved to where it says, and its delta patches' files returned.
pub(crate) fn apply_manifest(plan: &mut Plan, owner: &str) -> Option<Manifest> {
    let xml = plan.files.iter().find(|f| {
        f.owner == owner
            && f.target
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("modding.xml"))
    })?;
    let base = xml.target.parent();
    let base_key = base.as_ref().map(|b| b.key());
    let entries = parse_modding_xml(&fs::read_to_string(&xml.source).ok()?);
    let in_base = |t: &RelPath| t.parent().as_ref().map(|p| p.key()) == base_key;
    let package: Vec<(String, std::path::PathBuf)> = plan
        .files
        .iter()
        .filter(|f| f.owner == owner && in_base(&f.target))
        .filter_map(|f| {
            let n = f.target.file_name()?.to_ascii_lowercase();
            PACKAGE_FILES
                .contains(&n.as_str())
                .then(|| (n, f.source.clone()))
        })
        .collect();
    let mut manifest = Manifest {
        game_version: game_version(&package),
        ..Default::default()
    };
    let skip = |plan: &mut Plan, file: &str, reason: &str| {
        plan.skipped.push(Skipped {
            owner: owner.to_string(),
            file: file.to_string(),
            reason: reason.to_string(),
        })
    };
    // The package's own files.
    let mut kept = Vec::new();
    for f in std::mem::take(&mut plan.files) {
        let own = f.owner == owner
            && in_base(&f.target)
            && f.target
                .file_name()
                .is_some_and(|n| PACKAGE_FILES.contains(&n.to_ascii_lowercase().as_str()));
        if own {
            skip(
                plan,
                f.target.as_str(),
                "the mod package's information (DELTAMOD)",
            );
        } else {
            kept.push(f);
        }
    }
    plan.files = kept;

    for (kind, patch, to) in entries {
        let (Some(patch), Some(to)) = (clean(&patch), clean(&to)) else {
            continue;
        };
        let patch = match &base {
            Some(b) => b.join(&patch),
            None => patch,
        };
        let full_file = patch.as_str().to_ascii_lowercase().ends_with(".win");
        match kind.as_str() {
            "xdelta" | "csx" | "g3mpatch" if !full_file => {
                manifest.targets.insert(patch.key(), to);
            }
            "override" | "copy" | "xdelta" => {
                let Some(i) = plan
                    .files
                    .iter()
                    .position(|f| f.owner == owner && f.target.key() == patch.key())
                else {
                    skip(
                        plan,
                        patch.as_str(),
                        "listed in its modding.xml, but not in the mod",
                    );
                    continue;
                };
                // In the profile's order: this mod's file wins over earlier ones there.
                let mut f = plan.files.remove(i);
                plan.files.retain(|o| o.target.key() != to.key());
                f.target = to;
                plan.files.push(f);
            }
            _ => {}
        }
    }
    plan.files.sort_by_key(|f| f.target.key());
    Some(manifest)
}

/// For a GameMaker game with several data files: moves a mod's replacement data file to the
/// game's data file with the same display name (`DELTARUNE Chapter 2`), when the mod has it
/// somewhere else (a lone `data.win` at its top).
pub(crate) fn place_data_files(root: &Path, plan: &mut Plan) {
    let ext_ok = |t: &RelPath| {
        t.as_str().rsplit_once('.').is_some_and(|(_, e)| {
            gamemaker::DATA_EXTENSIONS
                .iter()
                .any(|x| e.eq_ignore_ascii_case(x))
        })
    };
    if !plan.files.iter().any(|f| ext_ok(&f.target)) {
        return;
    }
    let game_files = gamemaker::data_files(root);
    if game_files.len() < 2 {
        return;
    }
    let names: Vec<(String, Option<String>)> = game_files
        .iter()
        .map(|p| {
            let info = gamemaker::data_info(&root.join(p)).unwrap_or_default();
            (p.clone(), info.display_name.or(info.name))
        })
        .collect();
    let mut moves: Vec<(usize, RelPath)> = Vec::new();
    for (i, f) in plan.files.iter().enumerate() {
        if !ext_ok(&f.target) || !gamemaker::is_data_file(&f.source) {
            continue;
        }
        let Some(mine) = gamemaker::data_info(&f.source).and_then(|i| i.display_name.or(i.name))
        else {
            continue;
        };
        let here = names
            .iter()
            .find(|(p, _)| p.eq_ignore_ascii_case(f.target.as_str()))
            .and_then(|(_, n)| n.clone());
        if here.as_deref() == Some(mine.as_str()) {
            continue;
        }
        let matching: Vec<&String> = names
            .iter()
            .filter(|(_, n)| n.as_deref() == Some(mine.as_str()))
            .map(|(p, _)| p)
            .collect();
        if let [to] = matching.as_slice() {
            if let Ok(to) = RelPath::parse(to) {
                moves.push((i, to));
            }
        }
    }
    if moves.is_empty() {
        return;
    }
    // Each moved file replaces whatever else went there.
    let moved: Vec<_> = moves
        .into_iter()
        .map(|(i, to)| {
            let mut f = plan.files[i].clone();
            f.target = to;
            (i, f)
        })
        .collect();
    let mut i = 0;
    plan.files.retain(|o| {
        let gone = moved
            .iter()
            .any(|(m, f)| *m == i || f.target.key() == o.target.key());
        i += 1;
        !gone
    });
    plan.files.extend(moved.into_iter().map(|(_, f)| f));
    plan.files.sort_by_key(|f| f.target.key());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_modding_xml() {
        let xml = "<patch type=\"xdelta\" patch=\"./chapter1.xdelta\" to=\"./chapter1_windows/data.win\" />\n\
                   <patches><patch type='override' to=\"./mus/x.ogg\" patch=\"music\\x.ogg\"/></patches>";
        assert_eq!(
            parse_modding_xml(xml),
            [
                (
                    "xdelta".to_string(),
                    "./chapter1.xdelta".to_string(),
                    "./chapter1_windows/data.win".to_string()
                ),
                (
                    "override".to_string(),
                    "music\\x.ogg".to_string(),
                    "./mus/x.ogg".to_string()
                ),
            ]
        );
        assert_eq!(
            clean("./chapter1_windows/data.win").unwrap().as_str(),
            "chapter1_windows/data.win"
        );
        assert_eq!(clean(".\\mus\\x.ogg").unwrap().as_str(), "mus/x.ogg");
        assert!(clean("../evil").is_none());
    }
}
