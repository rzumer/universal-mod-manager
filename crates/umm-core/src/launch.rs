//! Deploy → launch → wait for the game to exit → revert.

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
        // A Paradox game's descriptors and list of enabled mods.
        self.plan_paradox(&def, &profile, &mut plan)?;
        // Factorio's mod folders, named as the game wants.
        self.plan_factorio(&def, &mut plan)?;
        // Kenshi's and XCOM 2's mod folders, named after their mods' files.
        self.plan_mod_files(&def, &mut plan)?;
        // Bannerlord's module folders, named after their ids.
        self.plan_bannerlord(&def, &mut plan)?;
        // A Larian game's enabled modules.
        self.plan_larian(&def, &profile, &mut plan)?;
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
                        .filter(|f| crate::godotml::is_loader_file(f))
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
                    let (loader, rest): (Vec<_>, Vec<_>) =
                        inside.into_iter().partition(crate::godotml::is_loader_file);
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
        if let Some(loader) = extender_loader(&def_launch, &journal.game_root) {
            // A Steam game started outside Steam restarts itself through Steam (without the
            // extender) when Steam is not running: start Steam first.
            let via_steam = def_launch
                .launch
                .as_ref()
                .is_some_and(|l| l.steam_relaunch == SteamRelaunch::ViaSteam);
            let started_steam = via_steam && ensure_steam(find_steam_exe());
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
            ensure_steam(find_steam_exe());
        }
        let mut spec = build_spec(
            &def_launch,
            &journal.game_root,
            &game.runtime_dir(&profile.name),
            &profile,
            injection,
            find_steam_exe(),
        )?;
        spec.args.extend(self.paradox_mod_args(&def, &profile));
        spec.args
            .extend(self.bannerlord_args(&def, &profile, &journal));

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

/// A Windows executable's CPU architecture, from its PE header: `Some(true)` for 64-bit,
/// `Some(false)` for 32-bit, `None` when it cannot tell.
fn is_64_bit(path: &Path) -> Option<bool> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut b = [0u8; 4];
    f.seek(SeekFrom::Start(0x3c)).ok()?;
    f.read_exact(&mut b).ok()?;
    let pe = u32::from_le_bytes(b) as u64;
    let mut h = [0u8; 6];
    f.seek(SeekFrom::Start(pe)).ok()?;
    f.read_exact(&mut h).ok()?;
    if &h[..4] != b"PE\0\0" {
        return None;
    }
    match u16::from_le_bytes([h[4], h[5]]) {
        0x8664 | 0xaa64 => Some(true),
        0x14c => Some(false),
        _ => None,
    }
}

/// BepInEx packs known to suit a build of Unity game, by (64-bit, IL2CPP). Only 64-bit packs
/// are known; for a 32-bit game nothing is suggested.
fn known_bepinex_pack(game_64: bool, il2cpp: bool) -> Option<&'static str> {
    match (game_64, il2cpp) {
        (true, true) => Some("BepInEx-BepInExPack_IL2CPP"),
        (true, false) => Some("BepInEx-BepInExPack"),
        (false, _) => None,
    }
}

/// An installed BepInEx pack that cannot load in the game.
#[derive(Debug, Clone)]
pub struct LoaderMismatch {
    pub message: String,
    /// A known pack for the game's build (`Owner-Name`) to offer in its place.
    pub suggestion: Option<String>,
}

impl Session {
    /// A message when an installed mod loader pack (one recorded as a loader) ships a
    /// Doorstop proxy for the other architecture than the game's exe; `None` when it matches
    /// or cannot be told.
    pub(crate) fn loader_architecture_warning(&self, id: &str) -> Option<String> {
        self.loader_mismatch(id).map(|m| m.message)
    }

    pub(crate) fn loader_mismatch(&self, id: &str) -> Option<LoaderMismatch> {
        let def = self.def(id).ok()?.def;
        let exe = RelPath::parse(&def.launch.as_ref()?.exe).ok()?;
        let (root, _) = self.install_path(&def).ok()??;
        let game_64 = is_64_bit(&exe.to_path(&root))?;
        let il2cpp = root.join("GameAssembly.dll").is_file();
        let game = self.data.game(id);
        // Only packs some profile uses: a replaced pack, turned off, is not a problem.
        let in_use: Vec<String> = umm_store::list_profiles(&game)
            .iter()
            .filter_map(|n| umm_store::load_profile(&game, n).ok())
            .flat_map(|p| p.mods.into_iter().filter(|e| e.enabled).map(|e| e.id))
            .collect();
        for m in umm_store::list_mods(&game).ok()? {
            if !m.thunderstore.as_ref().is_some_and(|t| t.loader) || !in_use.contains(&m.id) {
                continue;
            }
            for file in m.files.keys() {
                if !DOORSTOP_PROXIES
                    .iter()
                    .any(|p| file.eq_ignore_ascii_case(p))
                {
                    continue;
                }
                let path = RelPath::parse(file)
                    .ok()?
                    .to_path(&game.mod_files_dir(&m.id));
                if is_64_bit(&path).is_some_and(|p| p != game_64) {
                    let bits = |b: bool| if b { "64-bit" } else { "32-bit" };
                    let current = m.thunderstore.as_ref().map(|t| t.full_name.as_str());
                    let kind = def
                        .loader
                        .as_ref()
                        .map_or(BEPINEX_KIND, |l| l.kind.as_str());
                    let (name, what) = if kind == BEPINEX_KIND {
                        ("BepInEx", "BepInEx pack")
                    } else {
                        (loader_name(kind), "package")
                    };
                    return Some(LoaderMismatch {
                        message: format!(
                            "{} is {}, but {} is {}, so {name} will not load. You can try installing an unlisted {what} built for {}.",
                            m.name,
                            bits(!game_64),
                            def.name,
                            bits(game_64),
                            bits(game_64)
                        ),
                        suggestion: known_bepinex_pack(game_64, il2cpp)
                            .filter(|_| kind == BEPINEX_KIND)
                            .filter(|s| current.is_none_or(|c| !c.eq_ignore_ascii_case(s)))
                            .map(str::to_string),
                    });
                }
            }
        }
        None
    }
}

/// Doorstop's proxy DLLs: whichever the pack ships goes next to the game exe.
const DOORSTOP_PROXIES: &[&str] = &["winhttp.dll", "version.dll", "winmm.dll"];

/// Refuses a Doorstop proxy built for the other architecture than the game: Windows would
/// skip it without a word, the game would start, and no mod would load.
fn check_proxy_architecture(files: &[PlannedFile], game_exe: &Path, game: &str) -> Result<()> {
    let Some(game_64) = is_64_bit(game_exe) else {
        return Ok(());
    };
    for f in files {
        let name = f.target.as_str();
        if !DOORSTOP_PROXIES
            .iter()
            .any(|p| name.eq_ignore_ascii_case(p))
        {
            continue;
        }
        if let Some(proxy_64) = is_64_bit(&f.source) {
            if proxy_64 != game_64 {
                let bits = |b: bool| if b { "64-bit" } else { "32-bit" };
                return Err(CoreError::Invalid(format!(
                    "the mod loader pack's {name} (from {}) is {}, but {game} is {}: Windows would not load it and no mod would run. Use a {} pack",
                    f.owner,
                    bits(proxy_64),
                    bits(game_64),
                    bits(game_64)
                )));
            }
        }
    }
    Ok(())
}

/// How the profile's loader is started, as found when deploying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Injection {
    Doorstop(Doorstop),
    Shimloader,
    /// `regenerate`: the runtime folder has no generated game assemblies yet.
    MelonLoader {
        regenerate: bool,
    },
    Lovely,
    GDWeave,
    GodotML,
    ReturnOfModding,
    Northstar,
    Mio,
    Rivet,
    /// The renderer's BepInEx preloader, when the profile has it.
    BepisLoader {
        renderer: Option<&'static str>,
    },
}

/// Puts Return of Modding's proxy DLL next to the game's exe (Hades II keeps its own in
/// `Ship/`); errors when the profile has none.
fn place_rom_proxy(def: &GameDef, files: &mut [PlannedFile]) -> Result<()> {
    if place_by_exe(def, files, ROM_PROXIES) {
        Ok(())
    } else {
        Err(CoreError::Invalid(
            "the profile has no Return of Modding; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
        ))
    }
}

/// Moves the files named `names` at the game folder's root next to the game's exe, when the
/// exe is in a subfolder (a proxy DLL loads from there). Whether any was found.
fn place_by_exe(def: &GameDef, files: &mut [PlannedFile], names: &[&str]) -> bool {
    let exe_dir = def
        .launch
        .as_ref()
        .and_then(|l| RelPath::parse(&l.exe).ok())
        .and_then(|e| e.parent())
        .filter(|p| !p.is_root());
    let mut found = false;
    for f in files.iter_mut() {
        let name = f.target.as_str();
        if f.target.depth() != 1 || !names.iter().any(|p| name.eq_ignore_ascii_case(p)) {
            continue;
        }
        found = true;
        if let Some(dir) = &exe_dir {
            f.target = dir.join(&f.target);
        }
    }
    found
}

/// Whether `file` has `needle` in it, read a piece at a time.
fn file_contains(file: &Path, needle: &[u8]) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(file) else {
        return false;
    };
    let mut buf = vec![0u8; 1 << 20];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        carry.extend_from_slice(&buf[..n]);
        if carry.windows(needle.len()).any(|w| w == needle) {
            return true;
        }
        let keep = carry.len().saturating_sub(needle.len() - 1);
        carry.drain(..keep);
    }
}

/// Whether the game has the Godot Mod Loader built in: its project settings, in its pack
/// (`<exe>.pck`, or inside the exe), load it. Otherwise UMM sets it up for each session.
fn godotml_built_in(def: &GameDef, root: &Path) -> bool {
    let autoload = format!("*{GODOTML_AUTOLOAD}");
    let exe = def
        .launch
        .as_ref()
        .and_then(|l| RelPath::parse(&l.exe).ok())
        .map(|e| e.to_path(root));
    let Some(exe) = exe else {
        return false;
    };
    [exe.with_extension("pck"), exe]
        .iter()
        .any(|f| file_contains(f, autoload.as_bytes()))
}

/// Errors when the profile has no GDWeave: its proxy DLL at the game folder's root, and its
/// core in its folder.
fn check_gdweave(files: &[PlannedFile]) -> Result<()> {
    let proxy = files
        .iter()
        .any(|f| f.target.as_str().eq_ignore_ascii_case(GDWEAVE_PROXY));
    let core = format!("{}/core/", GDWEAVE_DIR.to_lowercase());
    let core = files
        .iter()
        .any(|f| f.target.as_str().to_lowercase().starts_with(&core));
    if proxy && core {
        Ok(())
    } else {
        Err(CoreError::Invalid(
            "the profile has no GDWeave; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
        ))
    }
}

/// Errors when the profile has no Lovely (its proxy DLL at the game folder's root).
fn check_lovely(files: &[PlannedFile]) -> Result<()> {
    let found = files.iter().any(|f| {
        f.target.depth() == 1
            && LOVELY_PROXIES
                .iter()
                .any(|p| f.target.as_str().eq_ignore_ascii_case(p))
    });
    if found {
        Ok(())
    } else {
        Err(CoreError::Invalid(
            "the profile has no Lovely; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
        ))
    }
}

/// Errors when the profile has no MelonLoader (its `MelonLoader.dll`, under `MelonLoader/`).
fn check_melonloader(files: &[PlannedFile]) -> Result<()> {
    let found = files.iter().any(|f| {
        f.target
            .first()
            .is_some_and(|c| c.eq_ignore_ascii_case("MelonLoader"))
            && f.target
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("MelonLoader.dll"))
    });
    if found {
        Ok(())
    } else {
        Err(CoreError::Invalid(
            "the profile has no MelonLoader; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
        ))
    }
}

/// Finds Shimloader's UE4SS among the files for the game folder and returns its folder (the
/// game's real exe's folder). Errors when the profile has none.
fn check_shimloader(files: &[PlannedFile]) -> Result<RelPath> {
    files
        .iter()
        .filter(|f| f.target.file_name().is_some_and(|n| n.eq_ignore_ascii_case("ue4ss.dll")))
        .find_map(|f| f.target.parent())
        .ok_or_else(|| {
            CoreError::Invalid(
                "the profile has no Shimloader; install it from the Thunderstore tab (it also comes with any mod that needs it)".into(),
            )
        })
}

/// The BepInEx build a profile has and the Doorstop that starts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Doorstop {
    pub build: DoorstopTarget,
    /// 3 or 4: the two spell their arguments differently.
    pub major: u8,
}

/// Finds which BepInEx build the profile carries (by the assembly Doorstop would load) and
/// its Doorstop version. Errors when there is none, or when the pack cannot work.
fn check_bepinex(def: &GameDef, files: &[PlannedFile], ds: &DoorstopLoader) -> Result<Doorstop> {
    let find = |p: &str| {
        files
            .iter()
            .find(|f| f.target.as_str().eq_ignore_ascii_case(p))
    };
    let Some(build) = ds
        .builds
        .iter()
        .copied()
        .find(|b| find(b.target()).is_some())
    else {
        let how = if def.loader.as_ref().is_some_and(|l| l.from_thunderstore()) {
            format!(
                "install the {} from the Thunderstore tab (it also comes with any mod that needs it)",
                ds.name
            )
        } else {
            format!("import a {} as the mod `bepinex`", ds.name)
        };
        return Err(CoreError::Invalid(format!(
            "the profile has no {}; {how}",
            ds.name
        )));
    };
    if build == DoorstopTarget::V6Il2cpp && find(CORECLR_PATH).is_none() {
        return Err(CoreError::Invalid(format!(
            "the {} pack has no {CORECLR_PATH}; reinstall the pack",
            build.label()
        )));
    }
    let read = |p: &str| {
        find(p)
            .and_then(|f| std::fs::read(&f.source).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    };
    let major = match read(".doorstop_version")
        .and_then(|v| v.trim().chars().next().and_then(|c| c.to_digit(10)))
    {
        Some(m) => m as u8,
        // Doorstop 3 configs have an [UnityDoorstop] section; Doorstop 4 ones [General].
        None => match read("doorstop_config.ini") {
            Some(ini) if ini.contains("[UnityDoorstop]") => 3,
            _ => 4,
        },
    };
    if build == DoorstopTarget::V6Il2cpp && major < 4 {
        return Err(CoreError::Invalid(
            "this BepInEx 6 IL2CPP pack ships Doorstop 3, which cannot start IL2CPP games; use a newer pack".into(),
        ));
    }
    Ok(Doorstop { build, major })
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
fn with_launchers(def: &GameDef, game_root: &Path) -> Vec<String> {
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

/// How long to wait for Steam to start and sign in before launching a game outside it.
const STEAM_START_WAIT: std::time::Duration = std::time::Duration::from_secs(45);
/// Steam's process is up some time before it has signed in and can answer games.
const STEAM_SETTLE: std::time::Duration = std::time::Duration::from_secs(8);

/// Starts Steam (minimised to the tray) if it is not running, and waits for it. Returns
/// whether it had to be started.
fn ensure_steam(steam: Option<PathBuf>) -> bool {
    let names = ["steam.exe".to_string()];
    if umm_launch::is_any_running(&names).unwrap_or(true) {
        return false;
    }
    let Some(steam) = steam else { return false };
    if std::process::Command::new(&steam)
        .arg("-silent")
        .spawn()
        .is_err()
    {
        return false;
    }
    let start = std::time::Instant::now();
    while start.elapsed() < STEAM_START_WAIT {
        std::thread::sleep(std::time::Duration::from_millis(500));
        // Its web helper starts once Steam is past its own updates and signing in.
        if umm_launch::is_any_running(&["steamwebhelper.exe".to_string()]).unwrap_or(false) {
            std::thread::sleep(STEAM_SETTLE);
            break;
        }
    }
    true
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
    injection: Option<Injection>,
    steam_exe: Option<PathBuf>,
) -> Result<LaunchSpec> {
    let launch = def
        .launch
        .as_ref()
        .ok_or_else(|| CoreError::NoLaunch(def.id.clone()))?;
    let exe_rel = RelPath::parse(&launch.exe).map_err(|e| CoreError::Invalid(e.to_string()))?;

    let mut game_args = launch.args.clone();
    let in_runtime = |rel: &str| {
        native_path(
            &RelPath::parse(rel)
                .expect("valid path")
                .to_path(runtime_dir),
        )
    };
    if injection == Some(Injection::Shimloader) {
        for (flag, rel) in SHIMLOADER_DIRS {
            game_args.extend([flag.to_string(), in_runtime(rel)]);
        }
    }
    if injection == Some(Injection::Lovely) {
        game_args.extend([LOVELY_MOD_DIR_FLAG.to_string(), in_runtime(LOVELY_DIR)]);
    }
    if let Some(Injection::BepisLoader { renderer }) = injection {
        game_args.extend([
            BEPISLOADER_ENABLE_FLAG.to_string(),
            BEPISLOADER_TARGET_FLAG.to_string(),
            in_runtime("BepInEx"),
        ]);
        // The renderer, a Unity process the game starts, gets Doorstop's arguments.
        if let Some(preloader) = renderer {
            game_args.extend([
                DOORSTOP_ENABLED_FLAG.to_string(),
                "true".to_string(),
                DOORSTOP_TARGET_FLAG.to_string(),
                in_runtime(preloader),
            ]);
        }
    }
    if injection == Some(Injection::Rivet) {
        let [enable, target, directory, log] = RIVET_FLAGS;
        game_args.extend([
            enable.to_string(),
            "true".to_string(),
            target.to_string(),
            in_runtime(RIVET_LOADER),
            directory.to_string(),
            in_runtime(RIVET_MODS_DIR),
            // Its log in the runtime folder, not left in the game folder.
            log.to_string(),
            in_runtime(RIVET_LOG),
        ]);
    }
    if injection == Some(Injection::Mio) {
        game_args.extend([
            MIO_MODS_FLAG.to_string(),
            in_runtime(MIO_MODS_DIR),
            MIO_CONFIG_FLAG.to_string(),
            in_runtime(MIO_CONFIG_DIR),
        ]);
    }
    if injection == Some(Injection::Northstar) {
        // Northstar reads its profile up to the first space, unless quoted in a way Windows
        // argument quoting does not produce.
        let dir = in_runtime(NORTHSTAR_DIR);
        if dir.contains([' ', '"']) {
            return Err(CoreError::Invalid(format!(
                "Northstar cannot use a profile folder whose path has a space in it ({dir}); move UMM's data folder"
            )));
        }
        game_args.extend([
            NORTHSTAR_FLAG.to_string(),
            format!("{NORTHSTAR_PROFILE_FLAG}{dir}"),
        ]);
    }
    if injection == Some(Injection::ReturnOfModding) {
        // The loader adds `ReturnOfModding/` to the folder it is given.
        game_args.extend([ROM_ROOT_FLAG.to_string(), native_path(runtime_dir)]);
    }
    if injection == Some(Injection::GodotML) {
        game_args.extend([
            GODOTML_ENABLE_FLAG.to_string(),
            GODOTML_MODS_FLAG.to_string(),
            in_runtime(GODOTML_MODS_DIR),
        ]);
    }
    if injection == Some(Injection::GDWeave) {
        // GDWeave keeps only what comes before a second `=`.
        let dir = in_runtime(GDWEAVE_DIR);
        if dir.contains('=') {
            return Err(CoreError::Invalid(format!(
                "GDWeave cannot use a folder whose path has `=` in it ({dir}); move UMM's data folder"
            )));
        }
        game_args.push(format!("{GDWEAVE_FOLDER_FLAG}{dir}"));
    }
    if let Some(Injection::MelonLoader { regenerate }) = injection {
        game_args.extend([
            MELONLOADER_BASEDIR_FLAG.to_string(),
            native_path(runtime_dir),
        ]);
        if regenerate {
            game_args.push(MELONLOADER_REGENERATE_FLAG.to_string());
        }
    }
    if let Some(Injection::Doorstop(d)) = injection {
        let (enabled, target_flag) = if d.major == 3 {
            (DOORSTOP3_ENABLED_FLAG, DOORSTOP3_TARGET_FLAG)
        } else {
            (DOORSTOP_ENABLED_FLAG, DOORSTOP_TARGET_FLAG)
        };
        game_args.extend([
            enabled.to_string(),
            "true".to_string(),
            target_flag.to_string(),
            in_runtime(d.build.target()),
        ]);
        if d.build == DoorstopTarget::V6Il2cpp {
            // The pack's .NET runtime lives in the runtime folder, not the game folder.
            game_args.extend([
                DOORSTOP_CORECLR_FLAG.to_string(),
                in_runtime(CORECLR_PATH),
                DOORSTOP_CORLIB_FLAG.to_string(),
                in_runtime(CORLIB_DIR),
            ]);
        }
    }
    game_args.extend(profile.launch_args.iter().cloned());

    // A script extender's loader starts the game itself, with the extender loaded; a Steam
    // game started so still talks to the Steam client (overlay, playtime, achievements).
    // The loader exits once the game is up: the game is followed by its process names.
    if let Some(loader) = extender_loader(def, game_root) {
        let path = loader.to_path(game_root);
        // The loader too: one that fails (a game version it does not support) shows its error
        // and exits before the game starts, which ends the session instead of waiting.
        let mut names = def.process_names();
        if let Some(n) = loader.file_name() {
            names.push(n.to_string());
        }
        return Ok(LaunchSpec {
            cwd: path.parent().map(Path::to_path_buf),
            program: path,
            args: game_args,
            env: Vec::new(),
            tracking: Tracking::names(names),
        });
    }

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
            tracking: Tracking::names(with_launchers(def, game_root)),
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
        let text = format!("schema = 1\nname = \"G\"\n[detect]\nsteam_app_id = 480\n{extra}");
        GameDef::parse_game(&text, Path::new("game.toml"), "g").unwrap()
    }

    #[test]
    fn owned_launch_with_doorstop_args() {
        let d = def("[launch]\nexe = \"bin/Game.exe\"\nargs = [\"-windowed\"]\n\
             [loader]\nkind = \"bepinex\"\ninject = \"doorstop\"\n");
        let mut p = Profile::new("Default");
        p.launch_args = vec!["-skipintro".into()];
        let ds = |build, major| Some(Injection::Doorstop(Doorstop { build, major }));
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &p,
            ds(DoorstopTarget::V5, 4),
            None,
        )
        .unwrap();
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

        // Older BepInEx 5 packs ship Doorstop 3, which spells its flags differently.
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &p,
            ds(DoorstopTarget::V5, 3),
            None,
        )
        .unwrap();
        assert_eq!(spec.args[1], "--doorstop-enable");
        assert_eq!(spec.args[3], "--doorstop-target");

        // BepInEx 6 for IL2CPP loads its own assembly and the pack's .NET runtime.
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &p,
            ds(DoorstopTarget::V6Il2cpp, 4),
            None,
        )
        .unwrap();
        let rt = |parts: &[&str]| {
            native_path(
                &parts
                    .iter()
                    .fold(Path::new("/rt").to_path_buf(), |p, c| p.join(c)),
            )
        };
        assert_eq!(
            spec.args,
            [
                "-windowed",
                "--doorstop-enabled",
                "true",
                "--doorstop-target-assembly",
                &rt(&["BepInEx", "core", "BepInEx.Unity.IL2CPP.dll"]),
                "--doorstop-clr-runtime-coreclr-path",
                &rt(&["dotnet", "coreclr.dll"]),
                "--doorstop-clr-corlib-dir",
                &rt(&["dotnet"]),
                "-skipintro"
            ]
        );
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &p,
            ds(DoorstopTarget::V6Mono, 4),
            None,
        )
        .unwrap();
        assert_eq!(
            spec.args[4],
            rt(&["BepInEx", "core", "BepInEx.Unity.Mono.Preloader.dll"])
        );
        assert_eq!(spec.args.len(), 6, "no runtime paths for Mono");
        // Unity Mod Manager, through the same Doorstop.
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &p,
            ds(DoorstopTarget::UnityModManager, 4),
            None,
        )
        .unwrap();
        assert_eq!(spec.args[4], rt(&["UMM", "Core", "UnityModManager.dll"]));
        assert_eq!(spec.args.len(), 6);
    }

    #[test]
    fn shimloader_gets_its_folders_as_arguments() {
        let d = def("[launch]\nexe = \"Palworld.exe\"\n\
             [loader]\nkind = \"shimloader\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"palworld\"\n");
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &Profile::new("Default"),
            Some(Injection::Shimloader),
            None,
        )
        .unwrap();
        let rt = |parts: &[&str]| {
            native_path(
                &parts
                    .iter()
                    .fold(Path::new("/rt").to_path_buf(), |p, c| p.join(c)),
            )
        };
        assert_eq!(
            spec.args,
            [
                "--mod-dir",
                &rt(&["shimloader", "mod"]),
                "--pak-dir",
                &rt(&["shimloader", "pak"]),
                "--cfg-dir",
                &rt(&["shimloader", "cfg"]),
                "--overlay-dir",
                &rt(&["shimloader", "overlay"]),
            ]
        );
    }

    #[test]
    fn lovely_gets_its_mod_folder() {
        let d = def("[launch]\nexe = \"Balatro.exe\"\n\
             [loader]\nkind = \"lovely\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"balatro\"\n");
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &Profile::new("Default"),
            Some(Injection::Lovely),
            None,
        )
        .unwrap();
        assert_eq!(
            spec.args,
            [
                "--mod-dir",
                native_path(&Path::new("/rt").join("mods")).as_str()
            ]
        );
    }

    #[test]
    fn bepisloader_gets_each_process_bepinex() {
        let d = def("[launch]\nexe = \"Resonite.exe\"\n\
             [loader]\nkind = \"bepisloader\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"resonite\"\n");
        let spec = |renderer| {
            build_spec(
                &d,
                Path::new("/game"),
                Path::new("/rt"),
                &Profile::new("Default"),
                Some(Injection::BepisLoader { renderer }),
                None,
            )
            .unwrap()
            .args
        };
        let rt = |parts: &[&str]| {
            native_path(
                &parts
                    .iter()
                    .fold(Path::new("/rt").to_path_buf(), |p, c| p.join(c)),
            )
        };
        assert_eq!(
            spec(Some(BEPISLOADER_RENDERER_PRELOADERS[0])),
            [
                "--hookfxr-enable",
                "--bepinex-target",
                &rt(&["BepInEx"]),
                "--doorstop-enabled",
                "true",
                "--doorstop-target-assembly",
                &rt(&[
                    "Renderer",
                    "BepInEx",
                    "core",
                    "BepInEx.Unity.Mono.Preloader.dll"
                ])
            ]
        );
        assert_eq!(
            spec(None).len(),
            3,
            "no renderer BepInEx: the main process only"
        );
    }

    #[test]
    fn rivet_gets_its_loader_mods_and_log() {
        let d = def("[launch]\nexe = \"Release/ScrapMechanic.exe\"\n\
             [loader]\nkind = \"rivet\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"scrap-mechanic\"\n");
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &Profile::new("Default"),
            Some(Injection::Rivet),
            None,
        )
        .unwrap();
        let rt = |parts: &[&str]| {
            native_path(
                &parts
                    .iter()
                    .fold(Path::new("/rt").to_path_buf(), |p, c| p.join(c)),
            )
        };
        assert_eq!(
            spec.args,
            [
                "-rivetEnable",
                "true",
                "-rivetTarget",
                &rt(&["Rivet", "Loader.dll"]),
                "-rivetDirectory",
                &rt(&["Rivet", "Mods"]),
                "-rivetLog",
                &rt(&["Rivet", "rivet.log"])
            ]
        );
    }

    #[test]
    fn mio_gets_its_mod_and_settings_folders() {
        let d = def("[launch]\nexe = \"mio.exe\"\n\
             [loader]\nkind = \"mio\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"mio\"\n");
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &Profile::new("Default"),
            Some(Injection::Mio),
            None,
        )
        .unwrap();
        assert_eq!(
            spec.args,
            [
                "--mods-path",
                native_path(&Path::new("/rt").join("mods")).as_str(),
                "--mods-config-path",
                native_path(&Path::new("/rt").join("modconfig")).as_str()
            ]
        );
    }

    #[test]
    fn northstar_gets_its_profile_folder() {
        let d = def("[launch]\nexe = \"Titanfall2.exe\"\n\
             [loader]\nkind = \"northstar\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"northstar\"\n");
        let spec = |rt: &str| {
            build_spec(
                &d,
                Path::new("/game"),
                Path::new(rt),
                &Profile::new("Default"),
                Some(Injection::Northstar),
                None,
            )
        };
        assert_eq!(
            spec("/rt").unwrap().args,
            [
                "-northstar".to_string(),
                format!(
                    "-profile={}",
                    native_path(&Path::new("/rt").join("R2Northstar"))
                )
            ]
        );
        let err = spec("/my data/rt").unwrap_err().to_string();
        assert!(err.contains("space"), "{err}");
    }

    #[test]
    fn return_of_modding_gets_its_root_folder_and_its_proxy_goes_by_the_exe() {
        let d = def("[launch]\nexe = \"Ship/Hades2.exe\"\n\
             [loader]\nkind = \"return-of-modding\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"hades-ii\"\n");
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &Profile::new("Default"),
            Some(Injection::ReturnOfModding),
            None,
        )
        .unwrap();
        assert_eq!(
            spec.args,
            [
                "--rom_modding_root_folder",
                native_path(Path::new("/rt")).as_str()
            ]
        );
        let file = |t: &str| PlannedFile {
            target: RelPath::parse(t).unwrap(),
            source: PathBuf::new(),
            owner: "Hell2Modding-Hell2Modding".into(),
            hash: String::new(),
            size: 0,
            writable: false,
        };
        let mut files = vec![file("d3d12.dll"), file("Content/x.txt")];
        place_rom_proxy(&d, &mut files).unwrap();
        assert_eq!(files[0].target.as_str(), "Ship/d3d12.dll");
        assert_eq!(files[1].target.as_str(), "Content/x.txt");
        assert!(place_rom_proxy(&d, &mut [file("Content/x.txt")]).is_err());
    }

    #[test]
    fn godotml_gets_its_mod_folder() {
        let d = def("[launch]\nexe = \"Brotato.exe\"\n\
             [loader]\nkind = \"godotml\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"brotato\"\n");
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &Profile::new("Default"),
            Some(Injection::GodotML),
            None,
        )
        .unwrap();
        assert_eq!(
            spec.args,
            [
                "--enable-mods",
                "--mods-path",
                native_path(&Path::new("/rt").join("mods")).as_str()
            ]
        );
    }

    #[test]
    fn godotml_must_be_built_in() {
        let tmp = tempfile::tempdir().unwrap();
        let d = def("[launch]\nexe = \"Game.exe\"\n\
             [loader]\nkind = \"godotml\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"g\"\n");
        let pck = tmp.path().join("Game.pck");
        std::fs::write(&pck, b"GDPC res://addons/mod_loader/mod_loader.gd").unwrap();
        assert!(
            !godotml_built_in(&d, tmp.path()),
            "its files alone are not enough"
        );
        // Found across a read boundary, as the project settings' autoload.
        let mut big = vec![0u8; (1 << 20) - 10];
        big.extend_from_slice(b"autoload/ModLoader\0*res://addons/mod_loader/mod_loader.gd");
        std::fs::write(&pck, &big).unwrap();
        assert!(godotml_built_in(&d, tmp.path()));
        std::fs::remove_file(&pck).unwrap();
        // Or in the exe, when the pack is embedded.
        std::fs::write(tmp.path().join("Game.exe"), &big).unwrap();
        assert!(godotml_built_in(&d, tmp.path()));
    }

    #[test]
    fn gdweave_gets_its_folder() {
        let d = def("[launch]\nexe = \"webfishing.exe\"\n\
             [loader]\nkind = \"gdweave\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"webfishing\"\n");
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &Profile::new("Default"),
            Some(Injection::GDWeave),
            None,
        )
        .unwrap();
        assert_eq!(
            spec.args,
            [format!(
                "--gdweave-folder-override={}",
                native_path(&Path::new("/rt").join("GDWeave"))
            )]
        );
    }

    #[test]
    fn melonloader_gets_its_base_folder() {
        let d = def("[launch]\nexe = \"Game.exe\"\n\
             [loader]\nkind = \"recursive-melonloader\"\nsource = \"thunderstore\"\ninject = \"args\"\n\
             [thunderstore]\ncommunity = \"g\"\n");
        let spec = |regenerate| {
            build_spec(
                &d,
                Path::new("/game"),
                Path::new("/rt"),
                &Profile::new("Default"),
                Some(Injection::MelonLoader { regenerate }),
                None,
            )
            .unwrap()
            .args
        };
        let rt = native_path(Path::new("/rt"));
        assert_eq!(
            spec(true),
            [
                "--melonloader.basedir",
                rt.as_str(),
                "--melonloader.agfregenerate"
            ]
        );
        assert_eq!(spec(false), ["--melonloader.basedir", rt.as_str()]);
    }

    /// The smallest file `is_64_bit` reads: a PE header offset, then the header's machine.
    fn fake_pe(dir: &Path, name: &str, machine: u16) -> PathBuf {
        let mut b = vec![0u8; 0x40];
        b[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        b.extend_from_slice(b"PE\0\0");
        b.extend_from_slice(&machine.to_le_bytes());
        let p = dir.join(name);
        std::fs::write(&p, b).unwrap();
        p
    }

    #[test]
    fn refuses_a_proxy_for_the_other_architecture() {
        let tmp = tempfile::tempdir().unwrap();
        let game = fake_pe(tmp.path(), "Game.exe", 0x8664);
        let planned = |source: PathBuf| PlannedFile {
            target: RelPath::parse("winhttp.dll").unwrap(),
            source,
            owner: "bepinex-bepinexpack-amongus".into(),
            hash: String::new(),
            size: 0,
            writable: false,
        };
        let x86 = planned(fake_pe(tmp.path(), "proxy32.dll", 0x14c));
        let err = check_proxy_architecture(&[x86], &game, "Among Us").unwrap_err();
        assert!(
            err.to_string()
                .contains("is 32-bit, but Among Us is 64-bit"),
            "{err}"
        );
        let x64 = planned(fake_pe(tmp.path(), "proxy64.dll", 0x8664));
        assert!(check_proxy_architecture(&[x64], &game, "Among Us").is_ok());
        // A file UMM cannot read as an executable is not a reason to refuse.
        std::fs::write(tmp.path().join("odd.dll"), "not a dll").unwrap();
        let odd = planned(tmp.path().join("odd.dll"));
        assert!(check_proxy_architecture(&[odd], &game, "Among Us").is_ok());
    }

    #[test]
    fn via_steam_tracks_by_name() {
        let d = def("[launch]\nexe = \"Game.exe\"\nsteam_relaunch = \"via_steam\"\n");
        let spec = build_spec(
            &d,
            Path::new("/game"),
            Path::new("/rt"),
            &Profile::new("Default"),
            None,
            Some(PathBuf::from("steam.exe")),
        )
        .unwrap();
        assert_eq!(spec.program, PathBuf::from("steam.exe"));
        assert_eq!(spec.args, ["-applaunch", "480"]);
        assert_eq!(spec.tracking, Tracking::names(vec!["Game.exe".into()]));
    }

    #[test]
    fn a_script_extenders_loader_starts_the_game() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("Fallout3.exe"), "exe").unwrap();
        std::fs::write(root.join("fose_loader.exe"), "loader").unwrap();
        let launch = "[launch]\nexe = \"Fallout3.exe\"\nsteam_relaunch = \"via_steam\"\n";
        let spec = |d: &GameDef| {
            build_spec(
                d,
                root,
                Path::new("/rt"),
                &Profile::new("Default"),
                None,
                Some(PathBuf::from("steam.exe")),
            )
            .unwrap()
        };
        // Run in Steam's place, followed by its own process name and the game's.
        let s = spec(&def(launch));
        assert_eq!(s.program, root.join("fose_loader.exe"));
        assert_eq!(s.cwd.as_deref(), Some(root));
        assert!(s.args.is_empty());
        assert_eq!(
            s.tracking,
            Tracking::names(vec!["Fallout3.exe".into(), "fose_loader.exe".into()])
        );
        // Set up in Steam (or a patched exe): launched through Steam as usual.
        let s = spec(&def(&format!("{launch}ignore_extender_loader = true\n")));
        assert_eq!(s.program, PathBuf::from("steam.exe"));
        // No loader deployed (a vanilla profile): through Steam, which starts the game's
        // launcher first: followed too.
        std::fs::remove_file(root.join("fose_loader.exe")).unwrap();
        std::fs::write(root.join("Fallout3Launcher.exe"), "launcher").unwrap();
        let s = spec(&def(launch));
        assert_eq!(s.program, PathBuf::from("steam.exe"));
        assert_eq!(
            s.tracking,
            Tracking::names(vec!["Fallout3.exe".into(), "Fallout3Launcher.exe".into()])
        );
    }

    #[test]
    fn rejects_unsupported_definitions() {
        let p = Path::new("game.toml");
        let bad = "schema = 1\nname = \"G\"\n[launch]\nexe = \"Game.exe\"\nsteam_relaunch = \"via_steam\"\n";
        assert!(
            matches!(
                GameDef::parse_game(bad, p, "g"),
                Err(umm_gamedef::GameDefError::Unsupported(_))
            ),
            "via_steam without app id"
        );
        let bad = "schema = 1\nname = \"G\"\n[loader]\nkind = \"asi\"\ninject = \"doorstop\"\n";
        assert!(
            matches!(
                GameDef::parse_game(bad, p, "g"),
                Err(umm_gamedef::GameDefError::Unsupported(_))
            ),
            "doorstop for a non-Unity loader"
        );
    }
}
