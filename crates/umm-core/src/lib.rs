//! `Session`: the one API the CLI and the Tauri app both call.

mod doctor;
mod launch;
mod runtime;
mod steam;

pub use doctor::{Finding, Level};
pub use launch::{LaunchEvent, LaunchOutcome};
pub use runtime::SyncReport;
pub use steam::{Draft, Engine, Note, SteamApp, SteamCandidate};
pub use umm_deploy::{
    Conflict, DeployError, Faults, Journal, Method as DeployMethod, Op as DeployOp, Plan,
    RevertReport, Skipped,
};
pub use umm_gamedef::{CompiledDef, GameDef, GameDefError};
pub use umm_launch::catch_interrupts;
pub use umm_store::{
    DataDir, GameDir, ImportOptions, ModMeta, Profile, ProfileEntry, StoreError, DEFAULT_PROFILE,
};

use std::fs;
use std::path::{Path, PathBuf};
use umm_deploy::{build_plan, revert, GameLock};
use umm_gamedef::detect_install;
use umm_store::fsutil::IoCtx;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no game definition with id `{0}`")]
    UnknownGame(String),
    #[error(
        "`{0}` is not installed, or its folder was not found; set it with `umm game set-path`"
    )]
    NotInstalled(String),
    #[error("mods are deployed for `{0}`; revert first")]
    Deployed(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Def(#[from] GameDefError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Deploy(#[from] DeployError),
    #[error("`{0}` is running; close it first (UMM never changes files a running game uses)")]
    GameRunning(String),
    #[error("the game definition for `{0}` has no [launch] section")]
    NoLaunch(String),
    #[error(transparent)]
    Launch(#[from] umm_launch::LaunchError),
}

impl From<umm_gamedef::RelPathError> for CoreError {
    fn from(e: umm_gamedef::RelPathError) -> Self {
        CoreError::Store(e.into())
    }
}

impl From<umm_store::IoError> for CoreError {
    fn from(e: umm_store::IoError) -> Self {
        CoreError::Store(e.into())
    }
}

impl CoreError {
    pub fn is_injected_crash(&self) -> bool {
        matches!(self, CoreError::Deploy(DeployError::InjectedCrash(_)))
    }
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

#[derive(Debug, Clone)]
pub struct GameStatus {
    pub def: GameDef,
    pub install: Option<PathBuf>,
    /// "manual", "steam" or "registry".
    pub found_via: Option<&'static str>,
    pub deployed: bool,
    pub mod_count: usize,
}

pub struct Session {
    data: DataDir,
}

impl Session {
    pub fn open(root: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            data: DataDir::open(root.unwrap_or_else(DataDir::default_root))?,
        })
    }

    pub fn data(&self) -> &DataDir {
        &self.data
    }

    pub fn game_dir(&self, id: &str) -> GameDir {
        self.data.game(id)
    }

    /// Valid definitions, plus one error per invalid file.
    pub fn defs(&self) -> (Vec<GameDef>, Vec<GameDefError>) {
        GameDef::load_dir(&self.data.gamedefs_dir())
    }

    pub fn def(&self, id: &str) -> Result<CompiledDef> {
        let (defs, _) = self.defs();
        let def = defs
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| CoreError::UnknownGame(id.into()))?;
        Ok(def.compile()?)
    }

    /// The manual path if one is set, otherwise detection.
    pub fn install_path(&self, def: &GameDef) -> Result<Option<(PathBuf, &'static str)>> {
        if let Some(p) = self.data.game(&def.id).settings()?.install_path {
            return Ok(Some((p, "manual")));
        }
        Ok(detect_install(def).map(|d| (d.path, d.via)))
    }

    fn require_install(&self, def: &GameDef) -> Result<PathBuf> {
        match self.install_path(def)? {
            Some((p, _)) if p.is_dir() => Ok(p),
            _ => Err(CoreError::NotInstalled(def.id.clone())),
        }
    }

    pub fn status(&self) -> Result<Vec<GameStatus>> {
        let (defs, _) = self.defs();
        let mut out = Vec::new();
        for def in defs {
            let game = self.data.game(&def.id);
            let found = self.install_path(&def)?.filter(|(p, _)| p.is_dir());
            out.push(GameStatus {
                deployed: game.journal_path().exists(),
                mod_count: umm_store::list_mods(&game)?.len(),
                install: found.as_ref().map(|(p, _)| p.clone()),
                found_via: found.map(|(_, v)| v),
                def,
            });
        }
        Ok(out)
    }

    pub fn set_install_path(&self, id: &str, path: &Path) -> Result<()> {
        let def = self.def(id)?;
        if !path.is_dir() {
            return Err(CoreError::Invalid(format!(
                "{} is not a folder",
                path.display()
            )));
        }
        if let Some(marker) = &def.def.detect.marker {
            let rel = umm_gamedef::RelPath::parse(marker)
                .map_err(|e| CoreError::Invalid(e.to_string()))?;
            if !rel.to_path(path).exists() {
                return Err(CoreError::Invalid(format!(
                    "{} does not contain `{marker}`; is this the right folder?",
                    path.display()
                )));
            }
        }
        let game = self.data.game(id);
        game.ensure()?;
        let mut s = game.settings()?;
        s.install_path = Some(path.to_path_buf());
        game.save_settings(&s)?;
        Ok(())
    }

    /// Imports a mod and appends it, enabled, to `profile`.
    pub fn import(
        &self,
        id: &str,
        src: &Path,
        mut opts: ImportOptions,
        profile: &str,
    ) -> Result<ModMeta> {
        let def = self.def(id)?;
        let game = self.data.game(id);
        opts.keep_top_dirs.extend(def.content_top_dirs());
        if let Ok(Some((root, _))) = self.install_path(&def.def) {
            if let Ok(rd) = fs::read_dir(&root) {
                opts.keep_top_dirs.extend(
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                        .filter_map(|e| e.file_name().to_str().map(String::from)),
                );
            }
        }
        let meta = umm_store::import_mod(&game, src, &opts)?;
        let mut p = umm_store::load_profile(&game, profile)?;
        if p.position(&meta.id).is_none() {
            p.mods.push(ProfileEntry {
                id: meta.id.clone(),
                enabled: true,
            });
            umm_store::save_profile(&game, &p)?;
        }
        Ok(meta)
    }

    pub fn mods(&self, id: &str) -> Result<Vec<ModMeta>> {
        Ok(umm_store::list_mods(&self.data.game(id))?)
    }

    /// Deletes a mod from the store and from every profile. Explicit user action only.
    pub fn remove_mod(&self, id: &str, mod_id: &str) -> Result<()> {
        let game = self.data.game(id);
        let _lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(id.into()));
        }
        umm_store::remove_mod(&game, mod_id)?;
        for name in umm_store::list_profiles(&game) {
            let mut p = umm_store::load_profile(&game, &name)?;
            let before = p.mods.len();
            p.mods.retain(|m| m.id != mod_id);
            if p.mods.len() != before {
                umm_store::save_profile(&game, &p)?;
            }
        }
        Ok(())
    }

    pub fn profile(&self, id: &str, name: &str) -> Result<Profile> {
        Ok(umm_store::load_profile(&self.data.game(id), name)?)
    }

    pub fn save_profile(&self, id: &str, profile: &Profile) -> Result<()> {
        let game = self.data.game(id);
        game.ensure()?;
        Ok(umm_store::save_profile(&game, profile)?)
    }

    /// Profile names; `Default` is always present.
    pub fn profiles(&self, id: &str) -> Vec<String> {
        umm_store::list_profiles(&self.data.game(id))
    }

    /// Creates a profile, optionally copying another profile's mod list and launch arguments.
    pub fn create_profile(&self, id: &str, name: &str, copy_from: Option<&str>) -> Result<Profile> {
        if self
            .profiles(id)
            .iter()
            .any(|p| p.eq_ignore_ascii_case(name))
        {
            return Err(CoreError::Invalid(format!(
                "a profile named `{name}` already exists"
            )));
        }
        let mut p = match copy_from {
            Some(src) => self.profile(id, src)?,
            None => Profile::new(name),
        };
        p.name = name.to_string();
        self.save_profile(id, &p)?;
        Ok(p)
    }

    /// Deletes a profile and its runtime folder (configs the game wrote for that profile).
    /// Explicit user action only. The Default profile and mods in the store are kept.
    pub fn delete_profile(&self, id: &str, name: &str) -> Result<()> {
        if name == DEFAULT_PROFILE {
            return Err(CoreError::Invalid(
                "the Default profile cannot be deleted".into(),
            ));
        }
        let game = self.data.game(id);
        let _lock = GameLock::acquire(&game)?;
        if let Ok(Some(j)) = Journal::load(&game.journal_path()) {
            if j.profile == name {
                return Err(CoreError::Deployed(id.into()));
            }
        }
        // Validates the name and confirms the profile exists.
        self.profile(id, name)?;
        let file = game.profiles_dir().join(format!("{name}.toml"));
        fs::remove_file(&file).ctx(|| format!("removing {}", file.display()))?;
        let runtime = game.runtime_dir(name);
        if runtime.exists() {
            fs::remove_dir_all(&runtime).ctx(|| format!("removing {}", runtime.display()))?;
        }
        Ok(())
    }

    pub fn plan(&self, id: &str, profile: &str) -> Result<Plan> {
        let def = self.def(id)?;
        let game = self.data.game(id);
        let profile = umm_store::load_profile(&game, profile)?;
        Ok(build_plan(&game, &def, &profile)?)
    }

    /// Places the profile's mods in the game folder. Pair with [`Session::revert`].
    pub fn deploy(&self, id: &str, profile: &str, faults: &mut Faults) -> Result<Journal> {
        Ok(self.deploy_profile(id, profile, faults)?.0)
    }

    /// True if one of the game's processes is running.
    pub fn is_running(&self, def: &GameDef) -> Result<bool> {
        umm_launch::is_any_running(&def.process_names())
            .map_err(|e| CoreError::Invalid(format!("listing processes: {e}")))
    }

    fn ensure_not_running(&self, def: &GameDef) -> Result<()> {
        if self.is_running(def)? {
            return Err(CoreError::GameRunning(def.id.clone()));
        }
        Ok(())
    }

    /// Undoes a deployment (or an interrupted one). `Ok(None)` if nothing was deployed.
    ///
    /// If the game folder no longer exists (uninstalled, or on a drive that is not connected),
    /// nothing is touched unless `discard_if_game_missing` is set; then the journal and backups
    /// are dropped. The mod store and profiles are never touched by recovery.
    pub fn revert(
        &self,
        id: &str,
        discard_if_game_missing: bool,
        faults: &mut Faults,
    ) -> Result<Option<RevertReport>> {
        let game = self.data.game(id);
        if !game.journal_path().exists() {
            return Ok(None);
        }
        if let Ok(def) = self.def(id) {
            self.ensure_not_running(&def.def)?;
        }
        let _lock = GameLock::acquire(&game)?;
        let Some(mut journal) = Journal::load(&game.journal_path())? else {
            return Ok(None);
        };
        if !journal.game_root.is_dir() {
            if !discard_if_game_missing {
                return Err(DeployError::GameRootMissing(journal.game_root).into());
            }
            if !journal.backup_in_game && journal.backup_root.exists() {
                fs::remove_dir_all(&journal.backup_root)
                    .ctx(|| format!("removing {}", journal.backup_root.display()))?;
            }
            fs::remove_file(game.journal_path())
                .ctx(|| format!("removing {}", game.journal_path().display()))?;
            return Ok(Some(RevertReport::default()));
        }
        Ok(Some(revert(&game, &mut journal, faults)?))
    }

    /// Reverts every game with a leftover journal. Run at startup.
    pub fn recover_all(
        &self,
        discard_if_game_missing: bool,
    ) -> Vec<(String, Result<Option<RevertReport>>)> {
        self.data
            .games_with_data()
            .into_iter()
            .filter(|id| self.data.game(id).journal_path().exists())
            .map(|id| {
                let r = self.revert(&id, discard_if_game_missing, &mut Faults::none());
                (id, r)
            })
            .collect()
    }

    /// Removes a game from UMM: its mods, profiles and settings, then its definition file, so
    /// it leaves the games list. Explicit user action only; refused while mods are deployed.
    /// The game's own folder is never touched.
    pub fn delete_game_data(&self, id: &str) -> Result<()> {
        let game = self.data.game(id);
        if game.exists() {
            {
                let _lock = GameLock::acquire(&game)?;
                if game.journal_path().exists() {
                    return Err(CoreError::Deployed(id.into()));
                }
            }
            fs::remove_dir_all(game.root())
                .ctx(|| format!("removing {}", game.root().display()))?;
        }
        for path in self.def_files(id) {
            fs::remove_file(&path).ctx(|| format!("removing {}", path.display()))?;
        }
        Ok(())
    }

    /// The definition files in `gamedefs/` that define `id`.
    fn def_files(&self, id: &str) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.data.gamedefs_dir()) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension().is_some_and(|e| e.eq_ignore_ascii_case("toml"))
                    && GameDef::load_file(p).is_ok_and(|d| d.id == id)
            })
            .collect()
    }

    pub fn doctor(&self, id: &str) -> Result<Vec<Finding>> {
        doctor::run(self, id)
    }
}
