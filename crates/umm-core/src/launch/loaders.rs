//! The mod loaders' parts of a launch: checking the profile has the loader, its proxy's
//! architecture, where its files go, and how it is injected.

use super::*;

/// A Windows executable's CPU architecture, from its PE header: `Some(true)` for 64-bit,
/// `Some(false)` for 32-bit, `None` when it cannot tell.
pub(super) fn is_64_bit(path: &Path) -> Option<bool> {
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
pub(super) fn known_bepinex_pack(game_64: bool, il2cpp: bool) -> Option<&'static str> {
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
pub(super) const DOORSTOP_PROXIES: &[&str] = &["winhttp.dll", "version.dll", "winmm.dll"];

/// Refuses a Doorstop proxy built for the other architecture than the game: Windows would
/// skip it without a word, the game would start, and no mod would load.
pub(super) fn check_proxy_architecture(
    files: &[PlannedFile],
    game_exe: &Path,
    game: &str,
) -> Result<()> {
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
pub(super) fn place_rom_proxy(def: &GameDef, files: &mut [PlannedFile]) -> Result<()> {
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
pub(super) fn place_by_exe(def: &GameDef, files: &mut [PlannedFile], names: &[&str]) -> bool {
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
pub(super) fn file_contains(file: &Path, needle: &[u8]) -> bool {
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
pub(super) fn godotml_built_in(def: &GameDef, root: &Path) -> bool {
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
pub(super) fn check_gdweave(files: &[PlannedFile]) -> Result<()> {
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
pub(super) fn check_lovely(files: &[PlannedFile]) -> Result<()> {
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
pub(super) fn check_melonloader(files: &[PlannedFile]) -> Result<()> {
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
pub(super) fn check_shimloader(files: &[PlannedFile]) -> Result<RelPath> {
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
pub(super) fn check_bepinex(
    def: &GameDef,
    files: &[PlannedFile],
    ds: &DoorstopLoader,
) -> Result<Doorstop> {
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
