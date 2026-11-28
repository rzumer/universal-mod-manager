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
    let bad =
        "schema = 1\nname = \"G\"\n[launch]\nexe = \"Game.exe\"\nsteam_relaunch = \"via_steam\"\n";
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

#[test]
fn under_proton_every_game_starts_through_steam_with_wines_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("skse64_loader.exe"), b"loader").unwrap();
    let d = def("[launch]\nexe = \"Game.exe\"\nargs = [\"-windowed\"]\n\
         [loader]\nkind = \"bepinex\"\ninject = \"doorstop\"\n");
    let flatpak = SteamCmd {
        program: "flatpak".into(),
        args: vec!["run".into(), "com.valvesoftware.Steam".into()],
    };
    let ds = Some(Injection::Doorstop(Doorstop {
        build: DoorstopTarget::V5,
        major: 4,
    }));
    let spec = build_spec_on(
        &d,
        root,
        Path::new("/rt"),
        &Profile::new("Default"),
        ds,
        Some(flatpak),
        true,
    )
    .unwrap();
    // Not the script extender's loader, which Steam cannot start.
    assert_eq!(spec.program, Path::new("flatpak"));
    assert_eq!(
        spec.args,
        [
            "run",
            "com.valvesoftware.Steam",
            "-applaunch",
            "480",
            "-windowed",
            "--doorstop-enabled",
            "true",
            "--doorstop-target-assembly",
            "Z:\\rt\\BepInEx\\core\\BepInEx.Preloader.dll",
        ]
    );
    assert!(matches!(spec.tracking, Tracking::Names { .. }));
    assert_eq!(ds.as_ref().and_then(proton_override), Some("winhttp"));
}
