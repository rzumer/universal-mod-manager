//! The three ways UMM can set a game up for mods, and turning a definition into one of them.
//!
//! A game's own definition file is never rewritten; a setup chosen later in UMM is applied on
//! top of it each time the definition is loaded.

use crate::{
    loader_name, thunderstore_inject, GameDef, GameDefError, Inject, InstallRule, Loader,
    Thunderstore, BEPINEX_KIND, RECURSIVE_MELONLOADER_KIND, SHIMLOADER_KIND,
    THUNDERSTORE_SOURCE,
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

const BEPINEX_CONFIG: &str = "BepInEx/config/**";
const SHIMLOADER_CONFIG: &str = "shimloader/cfg/**";
const MELONLOADER_CONFIG: &str = "UserData/**";
const LOADER_CONFIGS: &[&str] = &[BEPINEX_CONFIG, SHIMLOADER_CONFIG, MELONLOADER_CONFIG];

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
        d.writable.paths.retain(|p| !LOADER_CONFIGS.contains(&p.as_str()));
        let (loader, rules, config) = match setup {
            LoaderSetup::Generic => (None, vec![rule("**", "")], None),
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
                let (rules, config) = if kind == SHIMLOADER_KIND {
                    (shimloader_rules(), SHIMLOADER_CONFIG)
                } else if crate::is_melonloader(kind) {
                    (
                        melonloader_rules(kind == RECURSIVE_MELONLOADER_KIND),
                        MELONLOADER_CONFIG,
                    )
                } else {
                    (bepinex_rules(), BEPINEX_CONFIG)
                };
                (
                    Some(Loader {
                        kind: kind.to_string(),
                        source: Some(THUNDERSTORE_SOURCE.into()),
                        inject,
                    }),
                    rules,
                    Some(config),
                )
            }
        };
        d.loader = loader;
        d.install = rules;
        if let Some(c) = config {
            d.writable.paths.push(c.into());
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
id = "trip"
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
        GameDef::parse(UNREAL, Path::new("t.toml")).unwrap()
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
        assert!(back.writable.paths.is_empty());
    }

    #[test]
    fn shimloader_from_thunderstore() {
        let shim = Community {
            label: "trip",
            loader: "shimloader",
        };
        let ts = def().with_setup(LoaderSetup::Thunderstore, Some(shim)).unwrap();
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
        assert_eq!(map("MyMod/Scripts/main.lua"), "shimloader/mod/MyMod/Scripts/main.lua");
        assert_eq!(map("shimloader/cfg/Engine.ini"), "shimloader/cfg/Engine.ini");
        assert!(c.is_writable(&RelPath::parse("shimloader/cfg/Engine.ini").unwrap()));

        // The same community with BepInEx changes the loader and its config folder.
        let bep = ts.with_setup(LoaderSetup::Thunderstore, Some(BEP)).unwrap();
        assert_eq!(bep.ts_loader(), Some("bepinex"));
        assert_eq!(bep.writable.paths, [BEPINEX_CONFIG]);

        let other = Community {
            label: "trip",
            loader: "northstar",
        };
        let err = def()
            .with_setup(LoaderSetup::Thunderstore, Some(other))
            .unwrap_err();
        assert!(err.to_string().contains("northstar"), "{err}");
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
            let ts = def().with_setup(LoaderSetup::Thunderstore, Some(c)).unwrap();
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
    fn thunderstore_source_needs_a_community() {
        let text = format!(
            "{UNREAL}\n[loader]\nkind = \"bepinex\"\nsource = \"thunderstore\"\ninject = \"doorstop\"\n"
        );
        assert!(matches!(
            GameDef::parse(&text, Path::new("t")),
            Err(GameDefError::LoaderSource(_))
        ));
        let ok = format!("{text}\n[thunderstore]\ncommunity = \"trip\"\n");
        let d = GameDef::parse(&ok, Path::new("t")).unwrap();
        assert_eq!(d.setup(), Some(LoaderSetup::Thunderstore));
    }
}
