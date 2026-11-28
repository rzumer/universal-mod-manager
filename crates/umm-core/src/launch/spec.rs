//! What to run and how to track it: the game's command, from its definition, its profile
//! and its mod loader.

use super::*;

/// A path as the game's loader expects it: backslashes only on Windows, so a data folder
/// given with `/` does not produce a mixed-separator argument.
pub(super) fn native_path(p: &Path) -> String {
    let s = p.display().to_string();
    if cfg!(windows) {
        s.replace('/', "\\")
    } else {
        s
    }
}

/// Pure: turns a game definition and profile into what to run and how to track it.
#[cfg(test)]
pub(crate) fn build_spec(
    def: &GameDef,
    game_root: &Path,
    runtime_dir: &Path,
    profile: &Profile,
    injection: Option<Injection>,
    steam_exe: Option<PathBuf>,
) -> Result<LaunchSpec> {
    build_spec_on(
        def,
        game_root,
        runtime_dir,
        profile,
        injection,
        steam_exe.map(SteamCmd::from),
        false,
    )
}

/// [`build_spec`] with Steam's command, for a game run through Proton (`proton`) or not.
/// Under Proton every game starts through Steam, which picks its Proton, and paths given to
/// it are Wine's.
pub(crate) fn build_spec_on(
    def: &GameDef,
    game_root: &Path,
    runtime_dir: &Path,
    profile: &Profile,
    injection: Option<Injection>,
    steam: Option<SteamCmd>,
    proton: bool,
) -> Result<LaunchSpec> {
    let launch = def
        .launch
        .as_ref()
        .ok_or_else(|| CoreError::NoLaunch(def.id.clone()))?;
    let exe_rel = RelPath::parse(&launch.exe).map_err(|e| CoreError::Invalid(e.to_string()))?;
    let native_path = |p: &Path| {
        if proton {
            wine_path(p)
        } else {
            native_path(p)
        }
    };

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
    if let Some(loader) = extender_loader(def, game_root).filter(|_| !proton) {
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

    if launch.steam_relaunch == SteamRelaunch::ViaSteam || proton {
        let app_id = def.detect.steam_app_id.ok_or_else(|| {
            CoreError::Invalid("steam_relaunch = \"via_steam\" needs steam_app_id".into())
        })?;
        let steam = steam.ok_or_else(|| CoreError::Invalid("Steam was not found".into()))?;
        let mut args = steam.args;
        args.extend(["-applaunch".to_string(), app_id.to_string()]);
        args.extend(game_args);
        return Ok(LaunchSpec {
            program: steam.program,
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
