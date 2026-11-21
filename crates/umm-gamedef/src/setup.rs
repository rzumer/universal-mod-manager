//! The three ways UMM can set a game up for mods, and turning a definition into one of them.
//!
//! A game's own definition file is never rewritten; a setup chosen later in UMM is applied on
//! top of it each time the definition is loaded.

use crate::{GameDef, GameDefError, Inject, InstallRule, Loader, Thunderstore, THUNDERSTORE_SOURCE};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderSetup {
    /// BepInEx 5 through Doorstop, with the pack and mods installed from Thunderstore.
    Thunderstore,
    /// BepInEx 5 through Doorstop, with a pack the user imports as the mod `bepinex`.
    Bepinex,
    /// No mod loader: mod files are copied into the game folder as they are.
    Generic,
}

/// Mod id of the BepInEx pack in the "BepInEx with your own pack" setup.
pub const BEPINEX_PACK_MOD: &str = "bepinex";

impl LoaderSetup {
    pub fn label(self) -> &'static str {
        match self {
            LoaderSetup::Thunderstore => "BepInEx from Thunderstore",
            LoaderSetup::Bepinex => "BepInEx with your own pack",
            LoaderSetup::Generic => "Generic (no mod loader)",
        }
    }
}

fn rule(m: &str, t: &str) -> InstallRule {
    InstallRule {
        pattern: m.into(),
        target: t.into(),
    }
}

/// Install rules for BepInEx 5 mods that are not already laid out for the game.
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

const BEPINEX_CONFIG: &str = "BepInEx/config/**";

impl GameDef {
    /// The setup this definition describes. `None`: a loader none of the setups covers (an
    /// ASI loader, say), kept exactly as the definition says.
    pub fn setup(&self) -> Option<LoaderSetup> {
        match &self.loader {
            None => Some(LoaderSetup::Generic),
            Some(l) if l.from_thunderstore() => Some(LoaderSetup::Thunderstore),
            Some(l) if l.kind == "bepinex5" && l.inject == Inject::Doorstop => {
                Some(LoaderSetup::Bepinex)
            }
            Some(_) => None,
        }
    }

    /// The Thunderstore community, if the definition names one.
    pub fn community(&self) -> Option<&str> {
        self.thunderstore.as_ref().map(|t| t.community.as_str())
    }

    /// This definition with `setup` applied. Applying the setup it already has changes
    /// nothing, so its own install rules (Unreal's pak rules, say) are kept. `community` is
    /// needed for [`LoaderSetup::Thunderstore`] unless the definition already names one.
    pub fn with_setup(
        &self,
        setup: LoaderSetup,
        community: Option<&str>,
    ) -> Result<GameDef, GameDefError> {
        let same_community = community.is_none_or(|c| self.community() == Some(c));
        if self.setup() == Some(setup) && same_community {
            return Ok(self.clone());
        }
        let mut d = self.clone();
        match setup {
            LoaderSetup::Generic => {
                d.loader = None;
                d.install = vec![rule("**", "")];
                d.writable.paths.retain(|p| p != BEPINEX_CONFIG);
            }
            LoaderSetup::Bepinex | LoaderSetup::Thunderstore => {
                let source = if setup == LoaderSetup::Thunderstore {
                    let community = community
                        .or(self.community())
                        .ok_or_else(|| {
                            GameDefError::Unsupported(format!(
                                "{} is not on Thunderstore, so it cannot use BepInEx from Thunderstore",
                                self.name
                            ))
                        })?
                        .to_string();
                    d.thunderstore = Some(Thunderstore { community });
                    THUNDERSTORE_SOURCE.to_string()
                } else {
                    format!("mod:{BEPINEX_PACK_MOD}")
                };
                d.loader = Some(Loader {
                    kind: "bepinex5".into(),
                    source: Some(source),
                    inject: Inject::Doorstop,
                });
                d.install = bepinex_rules();
                if !d.writable.paths.iter().any(|p| p == BEPINEX_CONFIG) {
                    d.writable.paths.push(BEPINEX_CONFIG.into());
                }
            }
        }
        d.validate()?;
        Ok(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RelPath;
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

        let ts = d.with_setup(LoaderSetup::Thunderstore, Some("trip")).unwrap();
        assert_eq!(ts.setup(), Some(LoaderSetup::Thunderstore));
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
    fn thunderstore_source_needs_a_community() {
        let text = format!(
            "{UNREAL}\n[loader]\nkind = \"bepinex5\"\nsource = \"thunderstore\"\ninject = \"doorstop\"\n"
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
