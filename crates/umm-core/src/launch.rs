//! Deploy → launch → wait for the game to exit → revert.

use crate::runtime::{self, SyncReport};
use crate::{CoreError, Result, Session};
use std::path::{Path, PathBuf};
use umm_deploy::{
    apply, build_plan, prepare, Faults, GameLock, Journal, PlannedFile, RevertReport,
};
use umm_gamedef::{
    detect::steam_roots, GameDef, RelPath, SteamRelaunch, DOORSTOP_ENABLED_FLAG,
    DOORSTOP_TARGET_FLAG,
};
use umm_launch::{LaunchSpec, Tracking};
use umm_store::fsutil::{hash_file, write_atomic, IoCtx};
use umm_store::Profile;

#[derive(Debug, Clone)]
pub enum LaunchEvent {
    /// A deployment left over from a crash was reverted before launching.
    Recovered(RevertReport),
    Deployed {
        game_files: usize,
        runtime: Option<SyncReport>,
    },
    Started {
        pid: u32,
    },
    /// Launched through Steam; waiting for the game's process to appear.
    WaitingForGame,
    GameDetected,
    Exited {
        code: Option<i32>,
    },
    Reverted(RevertReport),
}

#[derive(Debug, Clone)]
pub struct LaunchOutcome {
    pub exit_code: Option<i32>,
    /// False when a Steam launch never produced a matching process.
    pub game_appeared: bool,
    /// The caller stopped waiting; mods are still deployed and the journal is kept.
    pub abandoned: bool,
    pub played: std::time::Duration,
    pub revert: Option<RevertReport>,
}

impl Session {
    /// Deploys the profile, splitting files between the game folder (journaled) and, for
    /// Doorstop loaders, the profile's runtime folder (synced).
    pub(crate) fn deploy_profile(
        &self,
        id: &str,
        profile_name: &str,
        faults: &mut Faults,
    ) -> Result<(Journal, Option<SyncReport>)> {
        let def = self.def(id)?;
        let game = self.data.game(id);
        game.ensure()?;
        let _lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(id.into()));
        }
        let root = self.require_install(&def.def)?;
        self.ensure_not_running(&def.def)?;
        let profile = umm_store::load_profile(&game, profile_name)?;
        let mut plan = build_plan(&game, &def, &profile)?;

        let runtime = match def.def.doorstop() {
            Some(ds) => {
                let (inside, outside): (Vec<PlannedFile>, Vec<PlannedFile>) =
                    plan.files.into_iter().partition(|f| {
                        f.target
                            .first()
                            .is_some_and(|c| c.eq_ignore_ascii_case(ds.runtime_dir))
                    });
                plan.files = outside;
                Some(runtime::sync(&game.runtime_dir(&profile.name), &inside)?)
            }
            None => None,
        };

        if let (Some(launch), Some(app_id)) = (&def.def.launch, def.def.detect.steam_app_id) {
            if launch.steam_relaunch == SteamRelaunch::AppidFile {
                let f = steam_appid_file(&game.generated_dir(), app_id)?;
                plan.files.retain(|p| p.target.key() != f.target.key());
                plan.files.push(f);
            }
        }

        let mut journal = prepare(&game, &plan, &root, false)?;
        apply(&game, &mut journal, faults)?;
        Ok((journal, runtime))
    }

    /// Deploys the profile, starts the game, waits until every game process has exited, then
    /// reverts. `stop` is polled while waiting; if it returns true, waiting ends and the mods
    /// stay deployed (the next launch or `recover` reverts them once the game has exited).
    pub fn launch(
        &self,
        id: &str,
        profile_name: &str,
        stop: &dyn Fn() -> bool,
        on_event: &mut dyn FnMut(LaunchEvent),
    ) -> Result<LaunchOutcome> {
        let def = self.def(id)?;
        if def.def.launch.is_none() {
            return Err(CoreError::NoLaunch(id.into()));
        }
        self.ensure_not_running(&def.def)?;
        if let Some(r) = self.revert(id, false, &mut Faults::none())? {
            on_event(LaunchEvent::Recovered(r));
        }

        let (journal, runtime) = self.deploy_profile(id, profile_name, &mut Faults::none())?;
        let game_files = journal
            .ops
            .iter()
            .filter(|o| !matches!(o, umm_deploy::Op::Mkdir { .. }))
            .count();
        on_event(LaunchEvent::Deployed {
            game_files,
            runtime,
        });

        let game = self.data.game(id);
        let profile = umm_store::load_profile(&game, profile_name)?;
        let spec = build_spec(
            &def.def,
            &journal.game_root,
            &game.runtime_dir(&profile.name),
            &profile,
            find_steam_exe(),
        )?;

        let outcome = umm_launch::run(&spec, stop, &mut |e| {
            on_event(match e {
                umm_launch::Event::Started { pid } => LaunchEvent::Started { pid },
                umm_launch::Event::WaitingForProcess => LaunchEvent::WaitingForGame,
                umm_launch::Event::ProcessDetected => LaunchEvent::GameDetected,
            })
        });
        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                // The game never started: put the folder back before reporting.
                self.revert(id, false, &mut Faults::none())?;
                return Err(e.into());
            }
        };
        if outcome.abandoned {
            return Ok(LaunchOutcome {
                exit_code: outcome.exit_code,
                game_appeared: outcome.appeared,
                abandoned: true,
                played: outcome.duration,
                revert: None,
            });
        }
        on_event(LaunchEvent::Exited {
            code: outcome.exit_code,
        });
        let report = self.revert(id, false, &mut Faults::none())?;
        if let Some(r) = &report {
            on_event(LaunchEvent::Reverted(r.clone()));
        }
        Ok(LaunchOutcome {
            exit_code: outcome.exit_code,
            game_appeared: outcome.appeared,
            abandoned: false,
            played: outcome.duration,
            revert: report,
        })
    }
}

fn steam_appid_file(dir: &Path, app_id: u32) -> Result<PlannedFile> {
    let p = dir.join("steam_appid.txt");
    write_atomic(&p, app_id.to_string().as_bytes()).ctx(|| format!("writing {}", p.display()))?;
    let size = std::fs::metadata(&p)
        .ctx(|| format!("reading {}", p.display()))?
        .len();
    Ok(PlannedFile {
        target: RelPath::parse("steam_appid.txt").expect("valid path"),
        hash: hash_file(&p).ctx(|| format!("hashing {}", p.display()))?,
        size,
        source: p,
        owner: "umm".into(),
        writable: true,
    })
}

fn find_steam_exe() -> Option<PathBuf> {
    if cfg!(windows) {
        steam_roots()
            .into_iter()
            .map(|r| r.join("steam.exe"))
            .find(|p| p.is_file())
    } else {
        Some(PathBuf::from("steam"))
    }
}

/// A path as the game's loader expects it: backslashes only on Windows, so a data folder
/// given with `/` does not produce a mixed-separator argument.
fn native_path(p: &Path) -> String {
    let s = p.display().to_string();
    if cfg!(windows) {
        s.replace('/', "\\")
    } else {
        s
    }
}

/// Pure: turns a game definition and profile into what to run and how to track it.
pub(crate) fn build_spec(
    def: &GameDef,
    game_root: &Path,
    runtime_dir: &Path,
    profile: &Profile,
    steam_exe: Option<PathBuf>,
) -> Result<LaunchSpec> {
    let launch = def
        .launch
        .as_ref()
        .ok_or_else(|| CoreError::NoLaunch(def.id.clone()))?;
    let exe_rel = RelPath::parse(&launch.exe).map_err(|e| CoreError::Invalid(e.to_string()))?;

    let mut game_args = launch.args.clone();
    if let Some(ds) = def.doorstop() {
        let target = RelPath::parse(ds.target_assembly)
            .expect("valid path")
            .to_path(runtime_dir);
        game_args.extend([
            DOORSTOP_ENABLED_FLAG.to_string(),
            "true".to_string(),
            DOORSTOP_TARGET_FLAG.to_string(),
            native_path(&target),
        ]);
    }
    game_args.extend(profile.launch_args.iter().cloned());

    if launch.steam_relaunch == SteamRelaunch::ViaSteam {
        let app_id = def.detect.steam_app_id.ok_or_else(|| {
            CoreError::Invalid("steam_relaunch = \"via_steam\" needs steam_app_id".into())
        })?;
        let steam = steam_exe.ok_or_else(|| CoreError::Invalid("Steam was not found".into()))?;
        let mut args = vec!["-applaunch".to_string(), app_id.to_string()];
        args.extend(game_args);
        return Ok(LaunchSpec {
            program: steam,
            args,
            env: Vec::new(),
            cwd: None,
            tracking: Tracking::names(def.process_names()),
        });
    }
    Ok(LaunchSpec {
        program: exe_rel.to_path(game_root),
        args: game_args,
        env: Vec::new(),
        cwd: Some(game_root.to_path_buf()),
        tracking: Tracking::Job,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(extra: &str) -> GameDef {
        let text =
            format!("schema = 1\nid = \"g\"\nname = \"G\"\n[detect]\nsteam_app_id = 480\n{extra}");
        GameDef::parse(&text, Path::new("t")).unwrap()
    }

    #[test]
    fn owned_launch_with_doorstop_args() {
        let d = def("[launch]\nexe = \"bin/Game.exe\"\nargs = [\"-windowed\"]\n\
             [loader]\nkind = \"bepinex5\"\ninject = \"doorstop\"\n");
        let mut p = Profile::new("Default");
        p.launch_args = vec!["-skipintro".into()];
        let spec = build_spec(&d, Path::new("/game"), Path::new("/rt"), &p, None).unwrap();
        assert_eq!(spec.tracking, Tracking::Job);
        assert_eq!(
            spec.program,
            Path::new("/game").join("bin").join("Game.exe")
        );
        let target = Path::new("/rt")
            .join("BepInEx")
            .join("core")
            .join("BepInEx.Preloader.dll");
        assert_eq!(
            spec.args,
            [
                "-windowed",
                "--doorstop-enabled",
                "true",
                "--doorstop-target-assembly",
                &native_path(&target),
                "-skipintro"
            ]
        );
    }

    #[test]
    fn via_steam_tracks_by_name() {
        let d = def("[launch]\nexe = \"Game.exe\"\nsteam_relaunch = \"via_steam\"\n");
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &Profile::new("Default"),
            Some(PathBuf::from("steam.exe")),
        )
        .unwrap();
        assert_eq!(spec.program, PathBuf::from("steam.exe"));
        assert_eq!(spec.args, ["-applaunch", "480"]);
        assert_eq!(spec.tracking, Tracking::names(vec!["Game.exe".into()]));
    }

    #[test]
    fn rejects_unsupported_definitions() {
        let bad = "schema = 1\nid = \"g\"\nname = \"G\"\n[launch]\nexe = \"Game.exe\"\nsteam_relaunch = \"via_steam\"\n";
        assert!(
            GameDef::parse(bad, Path::new("t")).is_err(),
            "via_steam without app id"
        );
        let bad = "schema = 1\nid = \"g\"\nname = \"G\"\n[loader]\nkind = \"asi\"\ninject = \"doorstop\"\n";
        assert!(
            GameDef::parse(bad, Path::new("t")).is_err(),
            "doorstop for a non-Unity loader"
        );
    }
}
