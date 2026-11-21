//! Changes UMM makes to a game's `game.toml`, done in place so the rest of the file (your
//! comments, layout and other keys) is kept.

use crate::{GameDef, GameDefError};
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item};

fn parse(text: &str, origin: &Path) -> Result<DocumentMut, GameDefError> {
    text.parse::<DocumentMut>().map_err(|e| GameDefError::Unsupported(format!(
        "{} cannot be edited: {e}",
        origin.display()
    )))
}

/// Sets `[detect] path`, or removes it with `None`.
pub fn set_install_path(text: &str, origin: &Path, path: Option<&Path>) -> Result<String, GameDefError> {
    let mut doc = parse(text, origin)?;
    match path {
        Some(p) => {
            let detect = doc
                .entry("detect")
                .or_insert(toml_edit::table())
                .as_table_mut()
                .ok_or_else(|| GameDefError::Unsupported("[detect] is not a table".into()))?;
            detect.insert("path", value(p.display().to_string()));
        }
        None => {
            if let Some(t) = doc.get_mut("detect").and_then(Item::as_table_mut) {
                t.remove("path");
            }
        }
    }
    Ok(doc.to_string())
}

/// The sections a mod loader setup decides.
const SETUP_KEYS: &[&str] = &["loader", "thunderstore", "install", "writable"];

/// Rewrites the sections a mod loader setup decides (`[loader]`, `[thunderstore]`,
/// `[[install]]`, `[writable]`) to match `def`, leaving everything else as written.
pub fn set_setup(text: &str, origin: &Path, def: &GameDef) -> Result<String, GameDefError> {
    let mut doc = parse(text, origin)?;
    let fresh = toml::to_string(def)
        .map_err(|e| GameDefError::Unsupported(format!("writing the setup: {e}")))?;
    let fresh = parse(&fresh, Path::new("setup"))?;
    for key in SETUP_KEYS {
        match fresh.get(key) {
            Some(item) => {
                doc.insert(key, item.clone());
            }
            None => {
                doc.remove(key);
            }
        }
    }
    Ok(doc.to_string())
}

/// The text of a new `game.toml` for `def`, under a header of comment lines.
pub fn new_file(def: &GameDef, header: &[String]) -> String {
    let mut d = def.clone();
    d.id = String::new(); // the folder name is the id
    let mut out = String::new();
    for line in header {
        out.push_str(&format!("# {line}\n"));
    }
    if !header.is_empty() {
        out.push('\n');
    }
    out.push_str(&toml::to_string_pretty(&d).expect("game definitions serialize"));
    out
}

/// Where a game's file lives.
pub fn game_file(games_dir: &Path, id: &str) -> PathBuf {
    games_dir.join(id).join(crate::GAME_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LoaderSetup;

    const TEXT: &str = "# My notes about this game\nschema = 1\nname = \"Trip\" # the real name\n\n[detect]\nsteam_app_id = 5\n\n[[install]]\nmatch = \"*.pak\"\ntarget = \"Trip/Content/Paks/~mods/\"\n\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n";

    #[test]
    fn edits_keep_comments_and_other_keys() {
        let p = Path::new("game.toml");
        let t = set_install_path(TEXT, p, Some(Path::new(r"D:\Games\Trip"))).unwrap();
        assert!(t.contains("# My notes about this game"));
        assert!(t.contains("# the real name"));
        let def = GameDef::parse_game(&t, p, "trip").unwrap();
        assert_eq!(def.detect.path.as_deref(), Some(Path::new(r"D:\Games\Trip")));
        assert_eq!(def.detect.steam_app_id, Some(5));

        let own = def.with_setup(LoaderSetup::Bepinex, None).unwrap();
        let t2 = set_setup(&t, p, &own).unwrap();
        assert!(t2.contains("# My notes about this game"));
        let back = GameDef::parse_game(&t2, p, "trip").unwrap();
        assert_eq!(back.setup(), Some(LoaderSetup::Bepinex));
        assert_eq!(back.detect.path, def.detect.path);
        assert_eq!(back.name, "Trip");

        let generic = back.with_setup(LoaderSetup::Generic, None).unwrap();
        let t3 = set_setup(&t2, p, &generic).unwrap();
        let g = GameDef::parse_game(&t3, p, "trip").unwrap();
        assert!(g.loader.is_none());
        assert!(!t3.contains("[loader]"));

        let t4 = set_install_path(&t3, p, None).unwrap();
        assert!(GameDef::parse_game(&t4, p, "trip").unwrap().detect.path.is_none());
    }

    #[test]
    fn the_folder_is_the_id() {
        let p = Path::new("game.toml");
        assert!(GameDef::parse_game(TEXT, p, "trip").is_ok());
        let wrong = format!("id = \"other\"\n{TEXT}");
        assert!(matches!(
            GameDef::parse_game(&wrong, p, "trip"),
            Err(GameDefError::IdMismatch { .. })
        ));
        let def = GameDef::parse_game(TEXT, p, "trip").unwrap();
        assert!(!new_file(&def, &[]).lines().any(|l| l.starts_with("id =")));
    }
}
