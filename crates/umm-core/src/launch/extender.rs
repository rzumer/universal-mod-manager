//! Script extenders (SKSE, F4SE…): their loader starts the game when it is there.

use super::*;

/// The script extender loader beside the game's exe, unless the definition says to launch
/// the game as usual.
pub(crate) fn extender_loader(def: &GameDef, game_root: &Path) -> Option<RelPath> {
    let launch = def.launch.as_ref()?;
    if launch.ignore_extender_loader {
        return None;
    }
    let exe = RelPath::parse(&launch.exe).ok()?;
    let dir = exe.parent().unwrap_or_else(RelPath::root);
    umm_gamedef::EXTENDER_LOADERS.iter().find_map(|name| {
        let rel = dir.join(&RelPath::parse(name).ok()?);
        rel.to_path(game_root).is_file().then_some(rel)
    })
}

/// The game's process names, and its own launcher's beside its exe (`Fallout3Launcher.exe`),
/// which Steam may start first: the game counts as running while either is, so a launcher
/// closed without playing ends the session instead of a wait for the game.
pub(super) fn with_launchers(def: &GameDef, game_root: &Path) -> Vec<String> {
    let mut names = def.process_names();
    let dir = def
        .launch
        .as_ref()
        .and_then(|l| RelPath::parse(&l.exe).ok())
        .and_then(|e| e.parent())
        .unwrap_or_else(RelPath::root)
        .to_path(game_root);
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            if lower.ends_with(".exe")
                && lower.contains("launcher")
                && !names.iter().any(|n| n.eq_ignore_ascii_case(&name))
            {
                names.push(name);
            }
        }
    }
    names
}
