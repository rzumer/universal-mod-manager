//! The three ways UMM can set a game up for mods, and turning a definition into one of them.
//!
//! A game's own definition file is never rewritten; a setup chosen later in UMM is applied on
//! top of it each time the definition is loaded.

use crate::{
    loader_name, thunderstore_inject, GameDef, GameDefError, Inject, InstallRule, Loader,
    Thunderstore, BEPINEX_KIND, BEPISLOADER_KIND, DATA_ARCHIVE_EXTENSIONS, DATA_DIRS,
    EXTENDER_DIRS, GDWEAVE_KIND, GODOTML_KIND, LOVELY_KIND, MIO_KIND, NORTHSTAR_KIND,
    PLUGIN_EXTENSIONS, RECURSIVE_MELONLOADER_KIND, RIVET_KIND, ROM_KIND, SHIMLOADER_KIND,
    THUNDERSTORE_SOURCE, UMM_KIND,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderSetup {
    /// The game's mod loader on Thunderstore (BepInEx through Doorstop, or Shimloader), with
    /// the loader and the mods installed from Thunderstore.
    Thunderstore,
    /// BepInEx (5 or 6) through Doorstop, with a pack the user imports as the mod `bepinex`.
    Bepinex,
    /// No mod loader: mod files are copied into the game folder as they are.
    Generic,
}

/// Mod id of the BepInEx pack in the "BepInEx with your own pack" setup.
pub const BEPINEX_PACK_MOD: &str = "bepinex";

impl LoaderSetup {
    /// The setup's name. `ts_loader` is the Thunderstore loader kind it uses or would use;
    /// the Thunderstore setup is named after it ("Shimloader from Thunderstore").
    pub fn label(self, ts_loader: Option<&str>) -> String {
        match self {
            LoaderSetup::Thunderstore => format!(
                "{} from Thunderstore",
                loader_name(ts_loader.unwrap_or(BEPINEX_KIND))
            ),
            LoaderSetup::Bepinex => "BepInEx with your own pack".into(),
            LoaderSetup::Generic => "Generic (no mod loader)".into(),
        }
    }
}

/// A Thunderstore community and the mod loader its games use there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Community<'a> {
    pub label: &'a str,
    pub loader: &'a str,
}

fn rule(m: &str, t: &str) -> InstallRule {
    InstallRule {
        pattern: m.into(),
        target: t.into(),
    }
}

/// Install rules for a Bethesda game without a mod loader: plugins, archives and the data
/// folder's usual folders go to the data folder (`Data`); everything else, a mod laid out from
/// the game folder (`Data/…`) included, keeps its path.
pub fn bethesda_rules(data: &str) -> Vec<InstallRule> {
    let data = data.trim_matches('/');
    let mut v: Vec<InstallRule> = PLUGIN_EXTENSIONS
        .iter()
        .chain(DATA_ARCHIVE_EXTENSIONS)
        .map(|e| rule(&format!("*.{e}"), &format!("{data}/")))
        .collect();
    v.extend(
        DATA_DIRS
            .iter()
            .map(|d| rule(&format!("{d}/**"), &format!("{data}/{d}/"))),
    );
    v.extend(
        EXTENDER_DIRS
            .iter()
            .map(|d| rule(&format!("{d}/plugins/**"), &format!("{data}/{d}/plugins/"))),
    );
    v.push(rule("**", ""));
    v
}

/// Install rules for a Paradox game: each mod in a folder of its own in the user folder's
/// `mod/` (named `folder` in the definition's folders).
pub fn paradox_rules(folder: &str) -> Vec<InstallRule> {
    vec![rule("**", &format!("@{folder}/mod/{{mod}}/"))]
}

/// Install rules for a Larian game: `.pak` files to the user folder's `Mods/` (named
/// `folder` in the definition's folders), where the game finds them only at its top, and the
/// game folder's own folders (`bin/` for the Script Extender) where they are. Other files
/// (`info.json`, readmes) are left out.
pub fn larian_rules(folder: &str) -> Vec<InstallRule> {
    vec![
        rule("*.pak", &format!("@{folder}/Mods/")),
        rule("bin/**", "bin/"),
        rule("Data/**", "Data/"),
        // The Script Extender's loader.
        rule("DWrite.dll", "bin/"),
    ]
}

/// The data folder of a definition's Bethesda plugin list (not RimWorld's or Factorio's mod
/// list).
pub fn bethesda_data_folder(def: &GameDef) -> Option<&str> {
    let p = def.plugins.as_ref()?;
    let folder = p.folder.trim_matches('/');
    (!p.format.lists_mod_folders() && !folder.is_empty()).then_some(folder)
}

/// Install rules for a game listing mod folders (RimWorld, Factorio): each mod in its own
/// folder in `folder`.
pub fn mod_folder_rules(folder: &str) -> Vec<InstallRule> {
    vec![rule(
        "**",
        &format!("{}/{{mod}}/", folder.trim_matches('/')),
    )]
}

/// Install rules for BepInEx mods that are not already laid out for the game.
pub fn bepinex_rules() -> Vec<InstallRule> {
    vec![
        rule("BepInEx/**", "BepInEx/"),
        rule("plugins/**", "BepInEx/plugins/{mod}/"),
        rule("patchers/**", "BepInEx/patchers/{mod}/"),
        rule("config/**", "BepInEx/config/"),
        rule("*.dll", "BepInEx/plugins/{mod}/"),
        rule("**", ""),
    ]
}

/// Install rules for Shimloader mods imported from files rather than Thunderstore: paks go
/// to the pak folder, anything else to the mod folder, where UE4SS wants one folder per mod.
pub fn shimloader_rules() -> Vec<InstallRule> {
    vec![
        rule("shimloader/**", "shimloader/"),
        rule("*.pak", "shimloader/pak/{mod}/"),
        rule("*.utoc", "shimloader/pak/{mod}/"),
        rule("*.ucas", "shimloader/pak/{mod}/"),
        rule("**", "shimloader/mod/"),
    ]
}

/// Install rules for MelonLoader mods imported from files rather than Thunderstore:
/// MelonLoader's own folders keep their place, a loose DLL is a mod. With `recursive`
/// (MelonLoader 0.7 and later), each mod gets its own folder under `Mods/`.
pub fn melonloader_rules(recursive: bool) -> Vec<InstallRule> {
    let mods = if recursive { "Mods/{mod}/" } else { "Mods/" };
    vec![
        rule("Mods/**", mods),
        rule("Plugins/**", "Plugins/"),
        rule("UserData/**", "UserData/"),
        rule("UserLibs/**", "UserLibs/"),
        rule("MelonLoader/**", "MelonLoader/"),
        rule("*.dll", mods),
        rule("**", ""),
    ]
}

/// Install rules for Lovely mods imported from files rather than Thunderstore: Lovely's own
/// proxy stays at the game folder's root, its `lovely/` folder goes to `mods/lovely/`, and
/// anything else is a mod in its own folder under `mods/` (Lovely requires that).
pub fn lovely_rules() -> Vec<InstallRule> {
    vec![
        rule("version.dll", ""),
        rule("winmm.dll", ""),
        rule("lovely/**", "mods/lovely/"),
        rule("mods/**", "mods/"),
        rule("**", "mods/{mod}/"),
    ]
}

/// Install rules for GDWeave mods imported from files rather than Thunderstore: GDWeave's
/// proxy stays at the game folder's root and its own folder keeps its place; anything else
/// is a mod in its own folder under `GDWeave/mods/` (GDWeave reads one folder per mod).
pub fn gdweave_rules() -> Vec<InstallRule> {
    vec![
        rule("winmm.dll", ""),
        rule("GDWeave/**", "GDWeave/"),
        rule("mods/**", "GDWeave/mods/"),
        rule("**", "GDWeave/mods/{mod}/"),
    ]
}

/// Install rules for Godot Mod Loader mods imported from files rather than Thunderstore: the
/// loader reads mods as zips, so zips go to `mods/`. Anything else keeps its path.
pub fn godotml_rules() -> Vec<InstallRule> {
    vec![
        rule("mods/**", "mods/"),
        rule("*.zip", "mods/"),
        rule("**", ""),
    ]
}

/// Install rules for Return of Modding mods imported from files rather than Thunderstore: its
/// proxy stays at the game folder's root (moved next to the exe at deploy), its folders keep
/// their place, and anything else is a mod in its own folder under `plugins/`.
pub fn rom_rules() -> Vec<InstallRule> {
    vec![
        rule("version.dll", ""),
        rule("d3d12.dll", ""),
        rule("ReturnOfModding/**", "ReturnOfModding/"),
        rule("plugins/**", "ReturnOfModding/plugins/"),
        rule("plugins_data/**", "ReturnOfModding/plugins_data/"),
        rule("config/**", "ReturnOfModding/config/"),
        rule("**", "ReturnOfModding/plugins/{mod}/"),
    ]
}

/// Install rules for Unity Mod Manager mods imported from files rather than Thunderstore:
/// its own folders keep their place, and anything else is a mod in its own folder under
/// `UMM/Mods/` (Unity Mod Manager reads one folder per mod, with its `Info.json`).
pub fn umm_rules() -> Vec<InstallRule> {
    vec![
        rule("UMM/**", "UMM/"),
        rule("Mods/**", "UMM/Mods/"),
        rule("**", "UMM/Mods/{mod}/"),
    ]
}

/// Install rules for Northstar mods imported from files rather than Thunderstore: its profile
/// folder keeps its place, a `mods/` folder goes into it, and anything else is a mod in its
/// own folder under `R2Northstar/mods/` (Northstar reads one folder per mod, with its
/// `mod.json`).
pub fn northstar_rules() -> Vec<InstallRule> {
    vec![
        rule("R2Northstar/**", "R2Northstar/"),
        rule("mods/**", "R2Northstar/mods/"),
        rule("**", "R2Northstar/mods/{mod}/"),
    ]
}

/// Install rules for MIO Mod Loader mods imported from files rather than Thunderstore: the
/// loader's proxy and folder keep their place at the game folder's root, a `mods/` folder
/// stays one, and anything else is a mod in its own folder under `mods/` (the loader reads
/// one folder per mod, with its `mod.json`).
pub fn mio_rules() -> Vec<InstallRule> {
    vec![
        rule("winhttp.dll", ""),
        rule("mio-mod-loader/**", "mio-mod-loader/"),
        rule("mods/**", "mods/"),
        rule("**", "mods/{mod}/"),
    ]
}

/// Install rules for Rivet mods imported from files rather than Thunderstore: its proxy and
/// `Rivet.ini` keep their place at the game folder's root, its folder keeps its place, a
/// `Mods/` folder goes into it, and anything else is a mod in its own folder there.
pub fn rivet_rules() -> Vec<InstallRule> {
    vec![
        rule("version.dll", ""),
        rule("Rivet.ini", ""),
        rule("Rivet/**", "Rivet/"),
        rule("Mods/**", "Rivet/Mods/"),
        rule("**", "Rivet/Mods/{mod}/"),
    ]
}

const BEPINEX_CONFIG: &str = "BepInEx/config/**";
const SHIMLOADER_CONFIG: &str = "shimloader/cfg/**";
const MELONLOADER_CONFIG: &str = "UserData/**";
const LOVELY_CONFIG: &str = "mods/lovely/**";
const GDWEAVE_CONFIG: &str = "GDWeave/configs/**";
const ROM_CONFIG: &str = "ReturnOfModding/config/**";
/// Unity Mod Manager's settings, and each mod's `Settings.xml`.
const UMM_CONFIG: &str = "UMM/**/*.xml";
const MIO_CONFIG: &str = "modconfig/**";
const LOADER_CONFIGS: &[&str] = &[
    UMM_CONFIG,
    MIO_CONFIG,
    ROM_CONFIG,
    BEPINEX_CONFIG,
    SHIMLOADER_CONFIG,
    MELONLOADER_CONFIG,
    LOVELY_CONFIG,
    GDWEAVE_CONFIG,
];

impl GameDef {
    /// The setup this definition describes. `None`: a loader none of the setups covers (an
    /// ASI loader, say), kept exactly as the definition says.
    pub fn setup(&self) -> Option<LoaderSetup> {
        match &self.loader {
            None => Some(LoaderSetup::Generic),
            Some(l) if l.from_thunderstore() => Some(LoaderSetup::Thunderstore),
            Some(l) if l.kind == BEPINEX_KIND && l.inject == Inject::Doorstop => {
                Some(LoaderSetup::Bepinex)
            }
            Some(_) => None,
        }
    }

    /// The Thunderstore community, if the definition names one.
    pub fn community(&self) -> Option<&str> {
        self.thunderstore.as_ref().map(|t| t.community.as_str())
    }

    /// The loader kind, when the loader comes from Thunderstore.
    pub fn ts_loader(&self) -> Option<&str> {
        self.loader
            .as_ref()
            .filter(|l| l.from_thunderstore())
            .map(|l| l.kind.as_str())
    }

    /// The name of the setup in effect, or of the loader when no setup describes it.
    pub fn setup_label(&self) -> String {
        match (self.setup(), &self.loader) {
            (Some(s), _) => s.label(self.ts_loader()),
            (None, Some(l)) => loader_name(&l.kind).to_string(),
            (None, None) => LoaderSetup::Generic.label(None),
        }
    }

    /// This definition with `setup` applied. Applying the setup it already has changes
    /// nothing, so its own install rules (Unreal's pak rules, say) are kept. `community` is
    /// needed for [`LoaderSetup::Thunderstore`] unless the definition already uses it.
    pub fn with_setup(
        &self,
        setup: LoaderSetup,
        community: Option<Community>,
    ) -> Result<GameDef, GameDefError> {
        let same_community = community.is_none_or(|c| {
            self.community() == Some(c.label) && self.ts_loader() == Some(c.loader)
        });
        if self.setup() == Some(setup) && same_community {
            return Ok(self.clone());
        }
        let mut d = self.clone();
        d.writable.retain(|p| !LOADER_CONFIGS.contains(&p.as_str()));
        let (loader, rules, config) = match setup {
            LoaderSetup::Generic => (
                None,
                match (&self.paradox, &self.larian, &self.plugins) {
                    (Some(p), _, _) => paradox_rules(&p.folder),
                    (None, Some(l), _) => larian_rules(&l.folder),
                    (None, None, Some(p)) if p.format.lists_mod_folders() => {
                        mod_folder_rules(&p.folder)
                    }
                    (None, None, _) => bethesda_data_folder(self)
                        .map_or_else(|| vec![rule("**", "")], bethesda_rules),
                },
                None,
            ),
            LoaderSetup::Bepinex => (
                Some(Loader {
                    kind: BEPINEX_KIND.into(),
                    source: Some(format!("mod:{BEPINEX_PACK_MOD}")),
                    inject: Inject::Doorstop,
                }),
                bepinex_rules(),
                Some(BEPINEX_CONFIG),
            ),
            LoaderSetup::Thunderstore => {
                let current = self.community().zip(self.ts_loader());
                let (label, kind) = community
                    .map(|c| (c.label, c.loader))
                    .or(current)
                    .ok_or_else(|| {
                        GameDefError::Unsupported(format!(
                            "{} is not on Thunderstore, so it cannot use a mod loader from there",
                            self.name
                        ))
                    })?;
                let inject = thunderstore_inject(kind).ok_or_else(|| {
                    GameDefError::Unsupported(format!(
                        "{} uses {} on Thunderstore, which UMM does not support yet",
                        self.name,
                        loader_name(kind)
                    ))
                })?;
                d.thunderstore = Some(Thunderstore {
                    community: label.to_string(),
                });
                // GodotML keeps its configs in the game's user folder, outside UMM's.
                let (rules, config) = if kind == SHIMLOADER_KIND {
                    (shimloader_rules(), Some(SHIMLOADER_CONFIG))
                } else if kind == LOVELY_KIND {
                    (lovely_rules(), Some(LOVELY_CONFIG))
                } else if kind == GDWEAVE_KIND {
                    (gdweave_rules(), Some(GDWEAVE_CONFIG))
                } else if kind == GODOTML_KIND {
                    (godotml_rules(), None)
                } else if kind == ROM_KIND {
                    (rom_rules(), Some(ROM_CONFIG))
                } else if kind == UMM_KIND {
                    (umm_rules(), Some(UMM_CONFIG))
                } else if kind == BEPISLOADER_KIND {
                    // BepInEx plugins, for the main process.
                    (bepinex_rules(), Some(BEPINEX_CONFIG))
                } else if kind == RIVET_KIND {
                    // `Rivet.ini` is an ini, which the definition already copies.
                    (rivet_rules(), None)
                } else if kind == MIO_KIND {
                    (mio_rules(), Some(MIO_CONFIG))
                } else if kind == NORTHSTAR_KIND {
                    // Its saves and settings are written in the runtime folder.
                    (northstar_rules(), None)
                } else if crate::is_melonloader(kind) {
                    (
                        melonloader_rules(kind == RECURSIVE_MELONLOADER_KIND),
                        Some(MELONLOADER_CONFIG),
                    )
                } else {
                    (bepinex_rules(), Some(BEPINEX_CONFIG))
                };
                (
                    Some(Loader {
                        kind: kind.to_string(),
                        source: Some(THUNDERSTORE_SOURCE.into()),
                        inject,
                    }),
                    rules,
                    config,
                )
            }
        };
        d.loader = loader;
        d.install = rules;
        if let Some(c) = config {
            d.writable.push(c.into());
        }
        d.validate()?;
        Ok(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RelPath, RuntimeLoader};
    use std::path::Path;

    const UNREAL: &str = r#"
schema = 1
name = "Unreal Trip"

[[install]]
match = "*.pak"
target = "Trip/Content/Paks/~mods/"

[[install]]
match = "**"
target = ""
"#;

    const BEP: Community = Community {
        label: "trip",
        loader: "bepinex",
    };

    fn def() -> GameDef {
        GameDef::parse_game(UNREAL, Path::new("game.toml"), "trip").unwrap()
    }

    #[test]
    fn applying_the_same_setup_keeps_the_definition() {
        let d = def();
        assert_eq!(d.setup(), Some(LoaderSetup::Generic));
        let same = d.with_setup(LoaderSetup::Generic, None).unwrap();
        assert_eq!(same.install.len(), 2, "pak rule kept");
    }

    #[test]
    fn switching_setups() {
        let d = def();
        assert!(d.with_setup(LoaderSetup::Thunderstore, None).is_err());

        let ts = d.with_setup(LoaderSetup::Thunderstore, Some(BEP)).unwrap();
        assert_eq!(ts.setup(), Some(LoaderSetup::Thunderstore));
        assert_eq!(ts.setup_label(), "BepInEx from Thunderstore");
        assert_eq!(ts.community(), Some("trip"));
        assert!(ts.doorstop().is_some());
        assert!(ts.loader.as_ref().unwrap().source_mod().is_none());

        let own = ts.with_setup(LoaderSetup::Bepinex, None).unwrap();
        assert_eq!(
            own.loader.as_ref().unwrap().source_mod(),
            Some(BEPINEX_PACK_MOD)
        );
        let c = own.compile().unwrap();
        let mapped = c
            .map_mod_file("m", &RelPath::parse("plugins/A.dll").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(mapped.as_str(), "BepInEx/plugins/m/A.dll");
        assert!(c.is_writable(&RelPath::parse("BepInEx/config/x.cfg").unwrap()));

        let back = own.with_setup(LoaderSetup::Generic, None).unwrap();
        assert!(back.loader.is_none());
        assert_eq!(back.install.len(), 1);
        assert!(back.writable.is_empty());
    }

    #[test]
    fn shimloader_from_thunderstore() {
        let shim = Community {
            label: "trip",
            loader: "shimloader",
        };
        let ts = def()
            .with_setup(LoaderSetup::Thunderstore, Some(shim))
            .unwrap();
        assert_eq!(ts.setup(), Some(LoaderSetup::Thunderstore));
        assert_eq!(ts.setup_label(), "Shimloader from Thunderstore");
        assert_eq!(ts.runtime_loader(), Some(RuntimeLoader::Shimloader));
        assert!(ts.doorstop().is_none());
        let c = ts.compile().unwrap();
        let map = |p: &str| {
            c.map_mod_file("m", &RelPath::parse(p).unwrap())
                .unwrap()
                .unwrap()
                .as_str()
                .to_string()
        };
        assert_eq!(map("Blue.pak"), "shimloader/pak/m/Blue.pak");
        assert_eq!(
            map("MyMod/Scripts/main.lua"),
            "shimloader/mod/MyMod/Scripts/main.lua"
        );
        assert_eq!(
            map("shimloader/cfg/Engine.ini"),
            "shimloader/cfg/Engine.ini"
        );
        assert!(c.is_writable(&RelPath::parse("shimloader/cfg/Engine.ini").unwrap()));

        // The same community with BepInEx changes the loader and its config folder.
        let bep = ts.with_setup(LoaderSetup::Thunderstore, Some(BEP)).unwrap();
        assert_eq!(bep.ts_loader(), Some("bepinex"));
        assert_eq!(bep.writable, [BEPINEX_CONFIG]);

        let other = Community {
            label: "trip",
            loader: "gdpatch",
        };
        let err = def()
            .with_setup(LoaderSetup::Thunderstore, Some(other))
            .unwrap_err();
        assert!(err.to_string().contains("gdpatch"), "{err}");
    }

    #[test]
    fn melonloader_from_thunderstore() {
        for (loader, mods) in [
            ("melonloader", "Mods/A.dll"),
            ("recursive-melonloader", "Mods/m/A.dll"),
        ] {
            let c = Community {
                label: "trip",
                loader,
            };
            let ts = def()
                .with_setup(LoaderSetup::Thunderstore, Some(c))
                .unwrap();
            assert_eq!(ts.setup_label(), "MelonLoader from Thunderstore");
            assert_eq!(ts.runtime_loader(), Some(RuntimeLoader::MelonLoader));
            let c = ts.compile().unwrap();
            let map = |p: &str| {
                c.map_mod_file("m", &RelPath::parse(p).unwrap())
                    .unwrap()
                    .unwrap()
                    .as_str()
                    .to_string()
            };
            assert_eq!(map("A.dll"), mods);
            assert_eq!(map("UserData/a.cfg"), "UserData/a.cfg");
            assert!(c.is_writable(&RelPath::parse("UserData/MelonPreferences.cfg").unwrap()));
        }
    }

    #[test]
    fn lovely_from_thunderstore() {
        let c = Community {
            label: "balatro",
            loader: "lovely",
        };
        let ts = def()
            .with_setup(LoaderSetup::Thunderstore, Some(c))
            .unwrap();
        assert_eq!(ts.setup_label(), "Lovely from Thunderstore");
        assert_eq!(ts.runtime_loader(), Some(RuntimeLoader::Lovely));
        let c = ts.compile().unwrap();
        let map = |p: &str| {
            c.map_mod_file("m", &RelPath::parse(p).unwrap())
                .unwrap()
                .unwrap()
                .as_str()
                .to_string()
        };
        assert_eq!(map("winmm.dll"), "winmm.dll");
        assert_eq!(map("lovely/config.toml"), "mods/lovely/config.toml");
        assert_eq!(map("lovely.toml"), "mods/m/lovely.toml");
        assert_eq!(map("src/main.lua"), "mods/m/src/main.lua");
    }

    #[test]
    fn thunderstore_source_needs_a_community() {
        let text = format!(
            "{UNREAL}\n[loader]\nkind = \"bepinex\"\nsource = \"thunderstore\"\ninject = \"doorstop\"\n"
        );
        assert!(matches!(
            GameDef::parse_game(&text, Path::new("game.toml"), "trip"),
            Err(GameDefError::LoaderSource(_))
        ));
        let ok = format!("{text}\n[thunderstore]\ncommunity = \"trip\"\n");
        let d = GameDef::parse_game(&ok, Path::new("game.toml"), "trip").unwrap();
        assert_eq!(d.setup(), Some(LoaderSetup::Thunderstore));
    }
}
