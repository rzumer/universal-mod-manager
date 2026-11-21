use crate::{Result, Session};
use std::fs;
use umm_deploy::Journal;
use umm_gamedef::IN_GAME_BACKUP_DIR;
use umm_store::fsutil::{count_files, dir_is_empty};
use umm_store::MOD_META_FILE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub level: Level,
    pub message: String,
}

fn f(level: Level, message: impl Into<String>) -> Finding {
    Finding {
        level,
        message: message.into(),
    }
}

pub(crate) fn run(s: &Session, id: &str) -> Result<Vec<Finding>> {
    let mut out = Vec::new();
    let def = s.def(id)?;
    let game = s.game_dir(id);

    let install = s.install_path(&def.def)?;
    // A folder set in game.toml that is gone: say which, rather than just "not found".
    let missing = def.def.detect.path.as_ref().filter(|p| !p.is_dir());
    match (&install, missing) {
        (Some((p, via)), None) if p.is_dir() => out.push(f(Level::Ok, format!("installed at {} (found via {via})", p.display()))),
        (_, Some(p)) | (Some((p, _)), None) => out.push(f(Level::Warn, format!(
            "install folder {} does not exist; the game may be uninstalled. Mods and profiles are kept.",
            p.display()
        ))),
        (None, None) => out.push(f(Level::Warn, "game not found; mods and profiles are kept. Set the folder with `umm game set-path`.")),
    }

    match Journal::load(&game.journal_path()) {
        Ok(Some(j)) => out.push(f(
            Level::Warn,
            format!(
            "mods are deployed (profile `{}`, phase {:?}, {} operations); run `umm revert {id}`",
            j.profile, j.phase, j.ops.len()
        ),
        )),
        Ok(None) => {
            out.push(f(Level::Ok, "no deployment in progress"));
            if !dir_is_empty(&game.backups_dir()).unwrap_or(false) {
                out.push(f(
                    Level::Error,
                    format!(
                        "stray backups with no journal in {}; these may be original game files",
                        game.backups_dir().display()
                    ),
                ));
            }
            if let Some((root, _)) = &install {
                let b = root.join(IN_GAME_BACKUP_DIR);
                if b.exists() {
                    out.push(f(
                        Level::Error,
                        format!("stray backup folder with no journal: {}", b.display()),
                    ));
                }
            }
        }
        Err(e) => out.push(f(Level::Error, format!("journal cannot be read: {e}"))),
    }

    if game.quarantine_dir().is_dir() {
        let n = count_files(&game.quarantine_dir());
        if n > 0 {
            out.push(f(Level::Info, format!(
                "{n} quarantined file(s) in {} (files changed while deployed); review or delete them",
                game.quarantine_dir().display()
            )));
        }
    }
    if game.staging_dir().is_dir() && !dir_is_empty(&game.staging_dir()).unwrap_or(true) {
        out.push(f(
            Level::Info,
            format!(
                "leftover import staging in {}; safe to delete",
                game.staging_dir().display()
            ),
        ));
    }

    if let Ok(rd) = fs::read_dir(game.mods_dir()) {
        for e in rd.filter_map(|e| e.ok()) {
            if e.path().is_dir() && !e.path().join(MOD_META_FILE).exists() {
                out.push(f(
                    Level::Warn,
                    format!(
                        "{} has no {MOD_META_FILE} (interrupted import?); re-import or delete it",
                        e.path().display()
                    ),
                ));
            }
        }
    }
    let mods = umm_store::list_mods(&game)?;
    let mut clean = 0;
    for m in &mods {
        let issues = umm_store::verify_mod(&game, m)?;
        if issues.is_empty() {
            clean += 1;
            continue;
        }
        let shown: Vec<String> = issues.iter().take(5).map(|i| i.to_string()).collect();
        let more = issues.len().saturating_sub(5);
        out.push(f(
            Level::Warn,
            format!(
                "mod `{}` differs from what was imported: {}{}",
                m.id,
                shown.join(", "),
                if more > 0 {
                    format!(" (+{more} more)")
                } else {
                    String::new()
                }
            ),
        ));
    }
    out.push(f(
        Level::Ok,
        format!(
            "{clean} of {} installed mod(s) match their recorded hashes",
            mods.len()
        ),
    ));

    for name in umm_store::list_profiles(&game) {
        let p = umm_store::load_profile(&game, &name)?;
        for m in &p.mods {
            if !mods.iter().any(|x| x.id == m.id) {
                out.push(f(
                    Level::Warn,
                    format!(
                        "profile `{name}` lists `{}`, which is not installed",
                        m.id
                    ),
                ));
            }
        }
    }
    if let Some(loader) = def.def.loader.as_ref().and_then(|l| l.source_mod()) {
        if !mods.iter().any(|m| m.id == loader) {
            out.push(f(Level::Warn, format!(
                "the game definition expects loader mod `{loader}`; import it with `umm import {id} <path> --id {loader}`"
            )));
        }
    }
    Ok(out)
}
