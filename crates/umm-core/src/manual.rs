//! Adding a game by hand: pick its folder, and UMM fills in the rest from what it finds there.

use crate::steam::{setups_for, TsFit};
use crate::{CoreError, Result, Session};
use std::path::{Path, PathBuf};
use umm_gamedef::steam::{
    draft_manual, inspect_folder, installed_games, to_toml, Draft, Engine, Note,
};
use umm_gamedef::LoaderSetup;

/// What UMM found in a folder, to fill in the "Add game manually" form.
#[derive(Debug, Clone)]
pub struct FolderInfo {
    pub folder: PathBuf,
    pub engine: Engine,
    /// A name to show, from the folder's name.
    pub name: String,
    /// Exes that could be the game (relative to the folder), the likely one first.
    pub exes: Vec<String>,
    /// The game's Thunderstore community, when its exe is a game there whose mod loader
    /// UMM supports.
    pub community: Option<String>,
    /// That community's mod loader kind.
    pub ts_loader: Option<String>,
    /// Thunderstore's mod loader for the game when UMM does not support it.
    pub other_loader: Option<String>,
    /// The setups it can be added with, the default first.
    pub setups: Vec<LoaderSetup>,
    /// The folder is a Steam game's install: `(app id, name)`. Add from Steam suits it better.
    pub steam_app: Option<(u32, String)>,
    /// The game's own mod support, which names the generic setup (`None`: files copied in).
    pub own_mods: Option<&'static str>,
}

impl Session {
    /// Looks at a game folder to suggest how to add it.
    pub fn inspect_game_folder(&self, folder: &Path) -> Result<FolderInfo> {
        if !folder.is_dir() {
            return Err(CoreError::Invalid(format!(
                "{} is not a folder",
                folder.display()
            )));
        }
        let found = inspect_folder(folder);
        let fit = self.community_for_exe(&found.exe);
        let same = |a: &Path, b: &Path| same_file::is_same_file(a, b).unwrap_or(false);
        let steam_app = installed_games()
            .into_iter()
            .find(|a| same(&a.path, folder))
            .map(|a| (a.app_id, a.name));
        Ok(FolderInfo {
            folder: folder.to_path_buf(),
            engine: found.engine,
            name: folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            own_mods: found.own_mods(),
            exes: found.exes,
            setups: setups_for(found.engine, fit.loader.as_deref()),
            community: fit.community,
            ts_loader: fit.loader,
            other_loader: fit.other_loader,
            steam_app,
        })
    }

    /// What Thunderstore says about the game with this exe name.
    fn community_for_exe(&self, exe: &str) -> TsFit {
        let name = exe.rsplit('/').next().unwrap_or(exe);
        match self.ecosystem(false) {
            Ok(eco) => TsFit::of(eco.by_exe_name(name)),
            Err(_) => TsFit::default(),
        }
    }

    /// Adds a game from its folder. `exe` is relative to the folder (empty: none);
    /// `setup` `None` takes the default for what the folder holds.
    pub fn add_manual_game(
        &self,
        folder: &Path,
        name: &str,
        exe: &str,
        setup: Option<LoaderSetup>,
    ) -> Result<Draft> {
        if name.trim().is_empty() {
            return Err(CoreError::Invalid("give the game a name".into()));
        }
        if !folder.is_dir() {
            return Err(CoreError::Invalid(format!(
                "{} is not a folder",
                folder.display()
            )));
        }
        let exe = exe.trim().replace('\\', "/");
        if !exe.is_empty() {
            let rel = umm_gamedef::RelPath::parse(&exe)
                .map_err(|e| CoreError::Invalid(format!("{exe}: {e}")))?;
            if !rel.to_path(folder).is_file() {
                return Err(CoreError::Invalid(format!(
                    "{exe} is not in {}",
                    folder.display()
                )));
            }
        }
        let mut found = inspect_folder(folder);
        let fit = self.community_for_exe(&exe);
        let setups = setups_for(found.engine, fit.loader.as_deref());
        let setup = setup.unwrap_or(setups[0]);
        if !setups.contains(&setup) {
            return Err(CoreError::Invalid(format!(
                "{name} cannot be set up as {}",
                setup.label(fit.loader.as_deref(), found.own_mods())
            )));
        }
        found.exe = exe.clone();
        let mut draft = draft_manual(folder, name, &exe, &found, setup, fit.as_community())?;
        // Its Windows folders (`%DOCUMENTS%`...) would be the Linux user's, not those of the
        // Wine prefix the game runs in, which UMM does not know of.
        if !cfg!(windows) && to_toml(&draft).contains('%') {
            return Err(CoreError::Invalid(format!(
                "{name} keeps files in Windows' user folders, which UMM finds on this system \
                 only for a game that Steam runs through Proton for now: add it from Steam"
            )));
        }
        if let Some(l) = &fit.other_loader {
            draft.notes.push(Note::new(
                "thunderstore-other-loader",
                format!("On Thunderstore this game uses {l}, which UMM does not support yet."),
            ));
        }
        let folder_name = folder
            .file_name()
            .and_then(|n| umm_gamedef::slug(&n.to_string_lossy()))
            .unwrap_or_else(|| "2".into());
        draft.def.id = self.free_game_id(&draft.def.id, &folder_name)?;
        self.create_game_file(&draft.def.id, &to_toml(&draft))?;
        Ok(draft)
    }
}
