//! Deploy → launch → wait for the game to exit → revert.

mod extender;
mod loaders;
mod proton;
mod spec;
mod steam;
#[cfg(test)]
mod tests;

pub(crate) use extender::*;
pub(crate) use loaders::*;
use proton::*;
pub(crate) use spec::*;
use steam::*;

use crate::runtime::{self, SyncReport};
use crate::{CoreError, Result, Session};
use std::path::{Path, PathBuf};
use umm_deploy::{
    apply, build_plan, prepare, Faults, GameLock, Journal, PlannedFile, RevertReport,
};
use umm_gamedef::{
    detect::steam_roots, loader_name, DoorstopLoader, DoorstopTarget, GameDef, RelPath,
    RuntimeLoader, SteamRelaunch, BEPINEX_KIND, BEPISLOADER_ENABLE_FLAG, BEPISLOADER_HOOK,
    BEPISLOADER_RENDERER_PRELOADERS, BEPISLOADER_TARGET_FLAG, CORECLR_PATH, CORLIB_DIR,
    DOORSTOP3_ENABLED_FLAG, DOORSTOP3_TARGET_FLAG, DOORSTOP_CORECLR_FLAG, DOORSTOP_CORLIB_FLAG,
    DOORSTOP_ENABLED_FLAG, DOORSTOP_TARGET_FLAG, GDWEAVE_DIR, GDWEAVE_FOLDER_FLAG, GDWEAVE_PROXY,
    GODOTML_AUTOLOAD, GODOTML_ENABLE_FLAG, GODOTML_MODS_DIR, GODOTML_MODS_FLAG, LOVELY_DIR,
    LOVELY_MOD_DIR_FLAG, LOVELY_PROXIES, MELONLOADER_BASEDIR_FLAG, MELONLOADER_GENERATED,
    MELONLOADER_REGENERATE_FLAG, MIO_CONFIG_DIR, MIO_CONFIG_FLAG, MIO_LOADER, MIO_MODS_DIR,
    MIO_MODS_FLAG, MIO_PROXY, NORTHSTAR_DIR, NORTHSTAR_DLL, NORTHSTAR_FLAG, NORTHSTAR_PROFILE_FLAG,
    NORTHSTAR_PROXY, RIVET_FLAGS, RIVET_LOADER, RIVET_LOG, RIVET_MODS_DIR, RIVET_PROXY,
    ROM_PROXIES, ROM_ROOT_FLAG, SHIMLOADER_DIRS,
};
use umm_launch::{LaunchSpec, Tracking};
use umm_store::fsutil::{hash_file, write_atomic, IoCtx};
use umm_store::Profile;

#[derive(Debug, Clone)]
pub enum LaunchEvent {
    /// A deployment left over from a crash was reverted before launching.
    Recovered(RevertReport),
    /// What the deploy is doing before files are placed: applying or merging mods' patches,
    /// downloading a tool for it.
    Preparing(String),
    /// These game files were replaced by a patcher's versions for the session.
    Patched {
        files: Vec<String>,
    },
    /// A script extender's loader starts the game, so that the extender loads.
    /// `started_steam`: Steam was not running, and was started first (a Steam game).
    ExtenderLoader {
        loader: String,
        started_steam: bool,
    },
    Deployed {
        game_files: usize,
        runtime: Option<SyncReport>,
    },
    /// A Windows game run through Proton: started through Steam, which picks its Proton.
    /// `notes`: what the user has to set in Steam themselves for its mods to load.
    Proton {
        notes: Vec<String>,
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

/// Splits planned files into those that live in the runtime folder and those for the game.
fn split_runtime(
    files: Vec<PlannedFile>,
    rl: &RuntimeLoader,
) -> (Vec<PlannedFile>, Vec<PlannedFile>) {
    let dirs = rl.runtime_dirs();
    files.into_iter().partition(|f| {
        f.target
            .first()
            .is_some_and(|c| dirs.iter().any(|d| c.eq_ignore_ascii_case(d)))
    })
}

impl Session {
    /// Brings every profile's existing runtime folder in line with its mod list now, so a
    /// deleted mod's files leave at once rather than at the next launch. Files the game wrote
    /// or edited there are kept, as at launch. Callers hold the game lock and have checked
    /// nothing is deployed.
    pub(crate) fn resync_runtimes(&self, id: &str) -> Result<()> {
        let def = self.def(id)?;
        let Some(rl) = def.def.runtime_loader() else {
            return Ok(());
        };
        let game = self.data.game(id);
        for name in umm_store::list_profiles(&game) {
            let dir = game.runtime_dir(&name);
            if !dir.is_dir() {
                continue;
            }
            let profile = umm_store::load_profile(&game, &name)?;
            let root = self.install_path(&def.def)?.map(|(p, _)| p);
            let plan = build_plan(&game, &def, &profile, root.as_deref())?;
            let (inside, _) = split_runtime(plan.files, &rl);
            runtime::sync(&dir, &inside)?;
        }
        Ok(())
    }

    /// Deploys the profile, splitting files between the game folder (journaled) and, for
    /// loaders kept outside the game (Doorstop, Shimloader), the profile's runtime folder
    /// (synced).
    pub(crate) fn deploy_profile(
        &self,
        id: &str,
        profile_name: &str,
        faults: &mut Faults,
        progress: &mut dyn FnMut(&str),
    ) -> Result<(Journal, Option<SyncReport>, Option<Injection>)> {
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
        let mut plan = build_plan(&game, &def, &profile, Some(&root))?;
        // Folders outside the game folder that mods go to (a Paradox game's user folder).
        plan.folders = self.resolve_folders(&def)?;
        // Patchers' results replace the game's files (a patcher out of date runs first).
        self.add_patches(&def, &profile, &root, &mut plan, true, progress)?;
        // WeiDU mods' installed result (installed again first, when out of date).
        self.add_weidu(&def, &profile, &root, &mut plan, true)?;
        // What the game needs beyond its mods' files: folders named as it wants, its
        // descriptors and lists.
        if let Some(g) = crate::games::integration(&def.def) {
            g.plan(self, &def, &profile, &mut plan)?;
        }
        let rl = def.def.runtime_loader();
        // The Godot Mod Loader's own files go to the game folder when UMM sets it up.
        let mut godotml_in_game = false;
        let (injection, shim_bin) = match rl {
            Some(RuntimeLoader::Doorstop(ds)) => (
                Some(Injection::Doorstop(check_bepinex(
                    &def.def,
                    &plan.files,
                    &ds,
                )?)),
                None,
            ),
            Some(RuntimeLoader::Shimloader) => (
                Some(Injection::Shimloader),
                Some(check_shimloader(&plan.files)?),
            ),
            Some(RuntimeLoader::Lovely) => {
                check_lovely(&plan.files)?;
                (Some(Injection::Lovely), None)
            }
            Some(RuntimeLoader::GDWeave) => {
                check_gdweave(&plan.files)?;
                (Some(Injection::GDWeave), None)
            }
            Some(RuntimeLoader::BepisLoader) => {
                let find = |p: &str| {
                    plan.files
                        .iter()
                        .any(|f| f.target.as_str().eq_ignore_ascii_case(p))
                };
                if !find(BEPISLOADER_HOOK) {
                    return Err(CoreError::Invalid(
                        "the profile has no BepisLoader; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
                    ));
                }
                // The renderer's BepInEx is a package of its own; without it, only the
                // main process is modded.
                let renderer = BEPISLOADER_RENDERER_PRELOADERS
                    .iter()
                    .copied()
                    .find(|p| find(p));
                (Some(Injection::BepisLoader { renderer }), None)
            }
            Some(RuntimeLoader::Rivet) => {
                let has = |p: &str| {
                    plan.files
                        .iter()
                        .any(|f| f.target.as_str().eq_ignore_ascii_case(p))
                };
                if !has(RIVET_PROXY) || !has(RIVET_LOADER) {
                    return Err(CoreError::Invalid(
                        "the profile has no Rivet; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
                    ));
                }
                // Its proxy, and the ini it reads beside it, go next to the game's exe
                // (Scrap Mechanic's is in `Release/`), as r2modman puts the proxy.
                place_by_exe(&def.def, &mut plan.files, &[RIVET_PROXY, "Rivet.ini"]);
                (Some(Injection::Rivet), None)
            }
            Some(RuntimeLoader::Mio) => {
                let has = |p: &str| {
                    plan.files
                        .iter()
                        .any(|f| f.target.as_str().eq_ignore_ascii_case(p))
                };
                if !has(MIO_PROXY) || !has(MIO_LOADER) {
                    return Err(CoreError::Invalid(
                        "the profile has no MIO Mod Loader; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
                    ));
                }
                (Some(Injection::Mio), None)
            }
            Some(RuntimeLoader::Northstar) => {
                let has = |p: &str| {
                    plan.files
                        .iter()
                        .any(|f| f.target.as_str().eq_ignore_ascii_case(p))
                };
                if !has(NORTHSTAR_PROXY) || !has(NORTHSTAR_DLL) {
                    return Err(CoreError::Invalid(
                        "the profile has no Northstar; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
                    ));
                }
                (Some(Injection::Northstar), None)
            }
            Some(RuntimeLoader::ReturnOfModding) => {
                place_rom_proxy(&def.def, &mut plan.files)?;
                (Some(Injection::ReturnOfModding), None)
            }
            Some(RuntimeLoader::GodotML) => {
                if !godotml_built_in(&def.def, &root) {
                    // Set up for the session: the loader's files and what its setup makes
                    // go to the game folder, journaled.
                    let loader: Vec<PlannedFile> = plan
                        .files
                        .iter()
                        .filter(|f| crate::patching::godotml::is_loader_file(f))
                        .cloned()
                        .collect();
                    if !loader.iter().any(|f| {
                        f.target
                            .as_str()
                            .eq_ignore_ascii_case("addons/mod_loader/mod_loader_setup.gd")
                    }) {
                        return Err(CoreError::Invalid(
                            "the profile has no Godot Mod Loader; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
                        ));
                    }
                    let made = self.godotml_setup_files(&def.def, &root, &loader)?;
                    plan.files
                        .retain(|f| !made.iter().any(|m| m.target.key() == f.target.key()));
                    plan.files.extend(made);
                    godotml_in_game = true;
                }
                (Some(Injection::GodotML), None)
            }
            Some(RuntimeLoader::MelonLoader) => {
                check_melonloader(&plan.files)?;
                // Whether assemblies need generating is known once the folder is synced.
                (Some(Injection::MelonLoader { regenerate: false }), None)
            }
            None => (None, None),
        };
        let proxied = matches!(
            injection,
            Some(
                Injection::Doorstop(_)
                    | Injection::MelonLoader { .. }
                    | Injection::Lovely
                    | Injection::GDWeave
                    | Injection::ReturnOfModding
                    | Injection::Mio
                    | Injection::Rivet
            )
        );
        if let (true, Some(launch)) = (proxied, &def.def.launch) {
            if let Ok(exe) = RelPath::parse(&launch.exe) {
                check_proxy_architecture(&plan.files, &exe.to_path(&root), &def.def.name)?;
            }
        }

        let runtime = match rl {
            Some(rl) => {
                let (mut inside, mut outside) = split_runtime(plan.files, &rl);
                if godotml_in_game {
                    let (loader, rest): (Vec<_>, Vec<_>) = inside
                        .into_iter()
                        .partition(crate::patching::godotml::is_loader_file);
                    outside.extend(loader);
                    inside = rest;
                }
                plan.files = outside;
                let dir = game.runtime_dir(&profile.name);
                let report = runtime::sync(&dir, &inside)?;
                if rl == RuntimeLoader::Shimloader {
                    // Shimloader wants every folder it is given to exist.
                    for (_, rel) in SHIMLOADER_DIRS {
                        let p = RelPath::parse(rel).expect("valid path").to_path(&dir);
                        std::fs::create_dir_all(&p).ctx(|| format!("creating {}", p.display()))?;
                    }
                }
                Some(report)
            }
            None => None,
        };
        let mut injection = injection;
        if let Some(Injection::MelonLoader { regenerate }) = &mut injection {
            let dir = game.runtime_dir(&profile.name);
            *regenerate = !MELONLOADER_GENERATED.iter().any(|rel| {
                RelPath::parse(rel)
                    .expect("valid path")
                    .to_path(&dir)
                    .is_file()
            });
        }

        if let (Some(launch), Some(app_id)) = (&def.def.launch, def.def.detect.steam_app_id) {
            if launch.steam_relaunch == SteamRelaunch::AppidFile {
                let f = steam_appid_file(&game.generated_dir(), app_id)?;
                // An Unreal game's real exe, next to Shimloader, looks for it in its own
                // folder: without it the game restarts through Steam, losing Shimloader's
                // arguments.
                let extra = shim_bin.map(|bin| PlannedFile {
                    target: bin.join(&f.target),
                    ..f.clone()
                });
                // Beside an exe in a folder of its own too (Paradox games' `binaries/`), where
                // the game looks for it when started directly.
                let by_exe = RelPath::parse(&launch.exe)
                    .ok()
                    .and_then(|e| e.parent())
                    .filter(|d| !d.is_root())
                    .map(|d| PlannedFile {
                        target: d.join(&f.target),
                        ..f.clone()
                    });
                for f in std::iter::once(f).chain(extra).chain(by_exe) {
                    plan.files.retain(|p| p.target.key() != f.target.key());
                    plan.files.push(f);
                }
            }
        }

        self.plan_plugin_list(&def, &profile, &mut plan, &root)?;
        plan.ini = self.plan_ini(&def)?;
        let mut journal = prepare(&game, &plan, &root, false)?;
        apply(&game, &mut journal, faults)?;
        if rl == Some(RuntimeLoader::GodotML) {
            self.place_godotml_hooks(&game, &root, &profile.name);
        }
        Ok((journal, runtime, injection))
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
        let proton = self.path_vars(&def.def).prefix_user.is_some();
        let windows_exe = || {
            let root = self.install_path(&def.def).ok().flatten().map(|(r, _)| r);
            let exe = def.def.launch.as_ref().map(|l| l.exe.as_str());
            match (root, exe.and_then(|e| RelPath::parse(e).ok())) {
                (Some(root), Some(exe)) => crate::is_windows_program(&exe.to_path(&root)),
                _ => false,
            }
        };
        if !cfg!(windows) && !proton && windows_exe() {
            return Err(crate::not_through_proton(&def.def.name));
        }
        if let Some(r) = self.revert(id, false, &mut Faults::none())? {
            on_event(LaunchEvent::Recovered(r));
        }

        let (journal, runtime, injection) =
            self.deploy_profile(id, profile_name, &mut Faults::none(), &mut |m| {
                on_event(LaunchEvent::Preparing(m.to_string()))
            })?;
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
        // A patched game exe starts its script extender itself (the Fallout Anniversary
        // Patcher's loads FOSE): the extender's own loader would fail on it.
        let patched = self.patched_in(id, &journal);
        let mut def_launch = def.def.clone();
        if let Some(l) = def_launch.launch.as_mut() {
            if patched.iter().any(|p| p.eq_ignore_ascii_case(&l.exe)) {
                l.ignore_extender_loader = true;
            }
        }
        if !patched.is_empty() {
            on_event(LaunchEvent::Patched { files: patched });
        }
        if proton {
            let mut notes = Vec::new();
            if let Some(loader) = extender_loader(&def_launch, &journal.game_root) {
                notes.push(format!(
                    "{} cannot be started through Steam, so the game starts without its script \
                     extender. To use it under Proton, have Steam start it instead of the game.",
                    loader.file_name().unwrap_or_default()
                ));
                if let Some(l) = def_launch.launch.as_mut() {
                    l.ignore_extender_loader = true;
                }
            }
            if let Some(dll) = injection.as_ref().and_then(proton_override) {
                notes.push(format!(
                    "Under Proton the mod loader loads only with this in the game's launch \
                     options in Steam: WINEDLLOVERRIDES=\"{dll}=n,b\" %command%"
                ));
            }
            on_event(LaunchEvent::Proton { notes });
        } else if let Some(loader) = extender_loader(&def_launch, &journal.game_root) {
            // A Steam game started outside Steam restarts itself through Steam (without the
            // extender) when Steam is not running: start Steam first.
            let via_steam = def_launch
                .launch
                .as_ref()
                .is_some_and(|l| l.steam_relaunch == SteamRelaunch::ViaSteam);
            let started_steam = via_steam && ensure_steam(find_steam());
            on_event(LaunchEvent::ExtenderLoader {
                loader: loader.file_name().unwrap_or_default().to_string(),
                started_steam,
            });
        } else if def_launch
            .launch
            .as_ref()
            .is_some_and(|l| l.steam_relaunch == SteamRelaunch::AppidFile)
            && def_launch.detect.steam_app_id.is_some()
        {
            // Started directly (a Paradox game, past its launcher): the game needs Steam
            // running to start, else it restarts itself through Steam.
            ensure_steam(find_steam());
        }
        let mut spec = build_spec_on(
            &def_launch,
            &journal.game_root,
            &game.runtime_dir(&profile.name),
            &profile,
            injection,
            find_steam(),
            proton,
        )?;
        if let Some(g) = crate::games::integration(&def.def) {
            spec.args
                .extend(g.launch_args(self, &def, &profile, &journal));
        }

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
