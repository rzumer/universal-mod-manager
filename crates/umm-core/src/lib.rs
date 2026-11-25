//! `Session`: the one API the CLI and the Tauri app both call.

mod config;
mod doctor;
mod downloads;
mod factorio;
mod fomod;
mod godotml;
mod icons;
mod launch;
mod link;
mod manual;
mod nexus;
mod paradox;
mod patcher;
mod placement;
mod plugins;
mod runtime;
mod setup;
mod share;
mod steam;
mod ts;
mod weidu;

pub use config::{ConfigChange, ConfigEntry, ConfigFile, ConfigSection, ConfigView};
pub use doctor::{Finding, Level};
pub use downloads::{downloads_dir, DownloadWatch};
pub use fomod::{FomodGroupView, FomodOptionView, FomodStepView, FomodView};
pub use godotml::{
    setup_cache as godotml_setup_cache, SetupRunner, SETUP_ARGS as GODOTML_SETUP_ARGS,
    SETUP_OWNER as GODOTML_SETUP_OWNER,
};
pub use launch::{LaunchEvent, LaunchOutcome};
pub use link::{
    parse_link, system_registry, InstallLink, LinkHandler, SchemeRegistry, LINK_SCHEME,
    LINK_SCHEMES, NXM_SCHEME,
};
pub use manual::FolderInfo;
pub use nexus::{
    CollectionFileState, ExpectedFile, NexusCollectionCheck, NexusGame, NexusInstallReport,
    NexusItem, NexusModFiles, NexusPage, NexusRequirement, NexusSearch, NexusSort, NexusUpdate,
    NexusUpdateState,
};
pub use patcher::{is_patcher_exe, patcher_of, PatchState, PatchedFile, PatcherStatus};
pub use placement::{PlacedFile, PlacementView};
pub use plugins::{IniPreview, PluginEntry, PluginList};
pub use runtime::SyncReport;
pub use setup::LoaderChoice;
pub use share::{ProfileExport, ProfileImport};
pub use steam::{Draft, Engine, Note, SteamApp, SteamCandidate, SteamList};
pub use ts::{
    package_mod_id, InstalledPackage, TsInstallReport, TsItem, TsLookup, TsPage, TsQuery, TsSort,
    TsVersionChoice,
};
pub use umm_deploy::{
    Conflict, DeployError, Faults, Journal, Method as DeployMethod, Op as DeployOp, Plan,
    RevertReport, Skipped,
};
pub use umm_fomod::{
    Choice as FomodChoice, GroupKind as FomodGroupKind, PluginType as FomodOptionKind,
};
pub use umm_gamedef::PluginListFormat;
pub use umm_gamedef::{CompiledDef, GameDef, GameDefError};
pub use umm_launch::catch_interrupts;
pub use umm_nexus::graph::{
    Game as NexusGameInfo, ModFile as NexusModFile, ModSummary as NexusModSummary,
};
pub use umm_nexus::{NexusError, NxmLink};
pub use umm_store::{
    name_archive, DataDir, GameDir, ImportOptions, Layout as ModLayout, ModMeta, Placement,
    Profile, ProfileEntry, StoreError, DEFAULT_PROFILE,
};
pub use umm_thunderstore::index::Package;
pub use weidu::{
    is_weidu_setup, weidu_setup_of, WeiduChoice, WeiduState, WeiduStatus, WEIDU_SCRATCH,
};

pub use umm_gamedef::LoaderSetup;
pub use umm_thunderstore::{Ecosystem, Http, TsError, UreqHttp};

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
    #[error(transparent)]
    Thunderstore(#[from] TsError),
    #[error(transparent)]
    Nexus(#[from] NexusError),
    /// A downloaded archive Nexus does not know by its hash (re-packed, or not from there).
    #[error("Nexus Mods does not recognise this file, so UMM cannot tell which download it is")]
    NexusUnrecognised,
    /// A downloaded archive Nexus knows as a file of another mod than the one awaited (another
    /// download that landed in the Downloads folder meanwhile).
    #[error("this file is {0} on Nexus Mods, not the one being downloaded")]
    NexusOtherMod(String),
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
    http: Arc<dyn Http>,
    ecosystem: Mutex<Option<Arc<Ecosystem>>>,
    ts: Mutex<ts::TsCache>,
    /// Nexus games looked up by domain name.
    nexus_games: Mutex<std::collections::HashMap<String, umm_nexus::graph::Game>>,
    /// Runs the Godot Mod Loader's setup; `None` for the real thing.
    godotml_runner: Mutex<Option<godotml::SetupRunner>>,
}

impl Session {
    /// Opens the data folder (the default one with `None`), with real network access.
    pub fn open(root: Option<PathBuf>) -> Result<Self> {
        Self::open_with(root, Arc::new(UreqHttp::default()))
    }

    /// Like [`Session::open`], with the given network access (tests pass a fake).
    pub fn open_with(root: Option<PathBuf>, http: Arc<dyn Http>) -> Result<Self> {
        Ok(Self {
            data: DataDir::open(root.unwrap_or_else(DataDir::default_root))?,
            http,
            ecosystem: Mutex::new(None),
            ts: Mutex::new(ts::TsCache::new()),
            nexus_games: Mutex::new(Default::default()),
            godotml_runner: Mutex::new(None),
        })
    }

    pub(crate) fn http(&self) -> &dyn Http {
        self.http.as_ref()
    }

    /// Folder for downloaded Thunderstore data.
    pub fn thunderstore_dir(&self) -> PathBuf {
        self.data.root().join("thunderstore")
    }

    /// Thunderstore's list of games and loaders: cached for a week, downloaded when missing or
    /// when `refresh` is set, and the cached copy used when the network fails.
    pub fn ecosystem(&self, refresh: bool) -> Result<Arc<Ecosystem>> {
        let mut slot = self.ecosystem.lock().unwrap_or_else(|e| e.into_inner());
        if let (Some(e), false) = (slot.as_ref(), refresh) {
            return Ok(e.clone());
        }
        let file = self.thunderstore_dir().join("ecosystem.json");
        let (eco, _) = umm_thunderstore::ecosystem::load(self.http(), &file, refresh)?;
        let eco = Arc::new(eco);
        *slot = Some(eco.clone());
        Ok(eco)
    }

    pub fn data(&self) -> &DataDir {
        &self.data
    }

    pub fn game_dir(&self, id: &str) -> GameDir {
        self.data.game(id)
    }

    /// Every game's definition (`games/<id>/game.toml`), plus one error per invalid file.
    pub fn defs(&self) -> (Vec<GameDef>, Vec<GameDefError>) {
        GameDef::load_games(&self.data.games_dir())
    }

    /// The game's definition, as its `game.toml` says.
    pub fn game_def(&self, id: &str) -> Result<GameDef> {
        let game = self.data.game(id);
        if !umm_gamedef::is_valid_id(id) || !game.def_path().is_file() {
            return Err(CoreError::UnknownGame(id.into()));
        }
        Ok(GameDef::load_game(game.root())?)
    }

    pub fn def(&self, id: &str) -> Result<CompiledDef> {
        Ok(self.game_def(id)?.compile()?)
    }

    /// Writes a new game's `game.toml`. Refuses to replace one.
    pub(crate) fn create_game_file(&self, id: &str, text: &str) -> Result<PathBuf> {
        use std::io::Write;
        let game = self.data.game(id);
        fs::create_dir_all(game.root()).ctx(|| format!("creating {}", game.root().display()))?;
        let path = game.def_path();
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ctx(|| format!("creating {}", path.display()))?;
        f.write_all(text.as_bytes())
            .ctx(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    /// Rewrites the game's `game.toml` with `change`, refusing a result that no longer loads.
    pub(crate) fn edit_game_file(
        &self,
        id: &str,
        change: impl FnOnce(&str, &Path) -> Result<String, GameDefError>,
    ) -> Result<GameDef> {
        let path = self.data.game(id).def_path();
        let text = fs::read_to_string(&path).ctx(|| format!("reading {}", path.display()))?;
        let new = change(&text, &path)?;
        let def = GameDef::parse_game(&new, &path, id)?;
        umm_store::fsutil::write_atomic(&path, new.as_bytes())
            .ctx(|| format!("writing {}", path.display()))?;
        Ok(def)
    }

    /// `[detect] path` if set and still there, otherwise detection.
    pub fn install_path(&self, def: &GameDef) -> Result<Option<(PathBuf, &'static str)>> {
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
        self.edit_game_file(id, |text, origin| {
            umm_gamedef::edit::set_install_path(text, origin, Some(path))
        })?;
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
        // A mod (not the loader pack, whose paths UMM reads) gets a placement instead of
        // having its wrapper folders stripped: its files stay as extracted.
        let is_loader = def
            .def
            .loader
            .as_ref()
            .and_then(|l| l.source_mod())
            .is_some_and(|l| opts.id.as_deref() == Some(l));
        let place = opts.unwrap && opts.layout == umm_store::Layout::Raw && !is_loader;
        let previous_meta = opts
            .id
            .as_deref()
            .and_then(|m| umm_store::load_mod(&game, m).ok());
        let previous = previous_meta.as_ref().and_then(|m| m.placement.clone());
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
        let meta = match self.thunderstore_layout(&def.def, src)? {
            // A Thunderstore package zip, for a game set up that way: lay it out like one.
            Some((files, manifest, package_files)) => {
                let staging = game.staging_dir().join(umm_store::fsutil::unique_suffix());
                let result = (|| -> Result<ModMeta> {
                    umm_thunderstore::layout::write(&files, &staging)?;
                    opts.package_files = package_files;
                    opts.name = opts.name.or(Some(manifest.name.replace('_', " ")));
                    if !manifest.version_number.is_empty() {
                        opts.version = opts.version.or(Some(manifest.version_number.clone()));
                    }
                    opts.layout = umm_store::Layout::Game;
                    opts.unwrap = false;
                    Ok(umm_store::import_mod(&game, &staging, &opts)?)
                })();
                let _ = fs::remove_dir_all(&staging);
                result?
            }
            None if place => {
                opts.unwrap = false;
                let meta = umm_store::import_mod(&game, src, &opts)?;
                let meta = self.place_imported(&def, meta, previous)?;
                self.fomod_imported(&def, meta, previous_meta.map(|m| (m.fomod, m.placement)))?
            }
            None => umm_store::import_mod(&game, src, &opts)?,
        };
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

    /// For a game set up for BepInEx from Thunderstore, a `.zip` with a `manifest.json` is
    /// laid out the way Thunderstore packages are. `None` for anything else.
    #[allow(clippy::type_complexity)]
    fn thunderstore_layout(
        &self,
        def: &GameDef,
        src: &Path,
    ) -> Result<
        Option<(
            std::collections::BTreeMap<String, Vec<u8>>,
            umm_thunderstore::layout::Manifest,
            std::collections::BTreeMap<String, Vec<u8>>,
        )>,
    > {
        let ts = def.loader.as_ref().is_some_and(|l| l.from_thunderstore());
        let zip = src
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        let (true, true, Some(community)) = (ts, zip, def.community()) else {
            return Ok(None);
        };
        let bytes = fs::read(src).ctx(|| format!("reading {}", src.display()))?;
        let Some(manifest) = umm_thunderstore::layout::manifest(&bytes)? else {
            return Ok(None);
        };
        let eco = self.ecosystem(false)?;
        let Some(m) = eco.community(community) else {
            return Ok(None);
        };
        let name = if manifest.name.is_empty() {
            "Local-Mod".to_string()
        } else {
            manifest.name.clone()
        };
        let files = umm_thunderstore::layout::plan(&bytes, &name, m.r2, None)?;
        let package = umm_thunderstore::layout::package_files(&bytes)?;
        Ok(Some((files, manifest, package)))
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
        // What UMM made from it goes too: imported again, it starts over.
        patcher::forget_mod(&game, mod_id);
        weidu::forget_mod(&game, mod_id);
        for name in umm_store::list_profiles(&game) {
            let mut p = umm_store::load_profile(&game, &name)?;
            let before = p.mods.len();
            p.mods.retain(|m| m.id != mod_id);
            if p.mods.len() != before {
                umm_store::save_profile(&game, &p)?;
            }
        }
        // Its files also leave the profiles' runtime folders now, not at the next launch.
        self.resync_runtimes(id)?;
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

    /// Deletes a profile, its runtime folder (configs the game wrote for that profile) and its
    /// WeiDU install. Explicit user action only. The Default profile and mods in the store
    /// are kept.
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
        weidu::forget_profile(&game, name).ctx(|| format!("removing {name}'s WeiDU install"))?;
        Ok(())
    }

    pub fn plan(&self, id: &str, profile: &str) -> Result<Plan> {
        let def = self.def(id)?;
        let game = self.data.game(id);
        let profile = umm_store::load_profile(&game, profile)?;
        let root = self.install_path(&def.def)?.map(|(p, _)| p);
        let mut plan = build_plan(&game, &def, &profile, root.as_deref())?;
        plan.folders = self.resolve_folders(&def)?;
        if let Some(r) = &root {
            self.add_patches(&def, &profile, r, &mut plan, false)?;
            self.add_weidu(&def, &profile, r, &mut plan, false)?;
        }
        Ok(plan)
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
            let _ = fs::remove_file(game.generated_dir().join("godotml-hooks.owned"));
            if !journal.backup_in_game && journal.backup_root.exists() {
                fs::remove_dir_all(&journal.backup_root)
                    .ctx(|| format!("removing {}", journal.backup_root.display()))?;
            }
            fs::remove_file(game.journal_path())
                .ctx(|| format!("removing {}", game.journal_path().display()))?;
            return Ok(Some(RevertReport::default()));
        }
        // The Godot Mod Loader's hook pack goes back to the profile. A failure leaves it, and
        // the mark, for the next revert: the mods still go.
        let _ = self.take_godotml_hooks(&game, &journal.game_root);
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

    /// Removes a game from UMM: its folder under `games/`, with its definition, mods,
    /// profiles and settings. Explicit user action only; refused while mods are deployed.
    /// The game's own install folder is never touched.
    pub fn delete_game_data(&self, id: &str) -> Result<()> {
        if !umm_gamedef::is_valid_id(id) {
            return Err(CoreError::UnknownGame(id.into()));
        }
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
        Ok(())
    }

    pub fn doctor(&self, id: &str) -> Result<Vec<Finding>> {
        doctor::run(self, id)
    }
}
