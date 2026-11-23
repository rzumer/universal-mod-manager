//! Serializable shapes sent to the webview. Kept here so the engine crates stay free of
//! UI concerns.

use serde::{Deserialize, Serialize};
use umm_core::{
    Finding, GameStatus, LaunchEvent, Level, LoaderChoice, LoaderSetup, ModMeta, Plan,
    RevertReport, SyncReport,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameDto {
    pub id: String,
    pub name: String,
    pub install: Option<String>,
    pub found_via: Option<&'static str>,
    pub deployed: bool,
    pub running: bool,
    pub mod_count: usize,
    pub can_launch: bool,
    pub loader: Option<String>,
    pub inject: Option<String>,
    /// Label of the mod loader setup in effect.
    pub setup: String,
    pub community: Option<String>,
    /// Set up with its mod loader from Thunderstore, so the Thunderstore tab applies.
    pub thunderstore: bool,
}

impl GameDto {
    pub fn new(s: GameStatus, running: bool) -> Self {
        let loader = s.def.loader.as_ref();
        GameDto {
            setup: match s.def.setup() {
                Some(_) => s.def.setup_label(),
                None => "Loader from the game definition".into(),
            },
            community: s.def.community().map(str::to_string),
            thunderstore: loader.is_some_and(|l| l.from_thunderstore()),
            install: s.install.map(|p| p.display().to_string()),
            found_via: s.found_via,
            deployed: s.deployed,
            running,
            mod_count: s.mod_count,
            can_launch: s.def.launch.is_some(),
            loader: loader.map(|l| l.kind.clone()),
            inject: loader.map(|l| format!("{:?}", l.inject).to_lowercase()),
            id: s.def.id,
            name: s.def.name,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModDto {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub source: Option<String>,
    pub files: usize,
    pub size: u64,
    pub imported_unix: u64,
    /// `Owner-Name`, for packages installed from Thunderstore.
    pub thunderstore: Option<String>,
    /// The mod's page, for files installed from Nexus Mods.
    pub nexus_url: Option<String>,
    /// Where its files go can be changed (not a package laid out for the game).
    pub placeable: bool,
    pub placement: Option<PlacementDto>,
    /// A FOMOD installer: `waiting` until its options are chosen, then `done`.
    pub installer: Option<&'static str>,
    /// Its patcher program (`Patcher.exe`), when the mod is a patcher of the game's files.
    pub patcher: Option<String>,
}

/// Where a mod's files go; see `umm_store::Placement`.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlacementDto {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub skip: Vec<String>,
    #[serde(default)]
    pub guessed: bool,
    #[serde(default)]
    pub chosen: bool,
}

impl From<umm_core::Placement> for PlacementDto {
    fn from(p: umm_core::Placement) -> Self {
        PlacementDto {
            root: p.root,
            target: p.target,
            skip: p.skip,
            guessed: p.guessed,
            chosen: p.chosen,
        }
    }
}

impl From<PlacementDto> for umm_core::Placement {
    fn from(p: PlacementDto) -> Self {
        umm_core::Placement {
            root: p.root,
            target: p.target,
            skip: p.skip,
            guessed: p.guessed,
            chosen: p.chosen,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacedFileDto {
    pub file: String,
    pub target: Option<String>,
    pub note: Option<String>,
    pub skipped: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementViewDto {
    pub placement: PlacementDto,
    pub suggested: PlacementDto,
    pub folders: Vec<String>,
    pub game_folders: Vec<String>,
    pub files: Vec<PlacedFileDto>,
}

impl From<umm_core::PlacementView> for PlacementViewDto {
    fn from(v: umm_core::PlacementView) -> Self {
        PlacementViewDto {
            placement: v.placement.into(),
            suggested: v.suggested.into(),
            folders: v.folders,
            game_folders: v.game_folders,
            files: v
                .files
                .into_iter()
                .map(|f| PlacedFileDto {
                    file: f.file,
                    target: f.target,
                    note: f.note,
                    skipped: f.skipped,
                })
                .collect(),
        }
    }
}

impl From<ModMeta> for ModDto {
    fn from(m: ModMeta) -> Self {
        let patcher = umm_core::patcher_of(&m);
        ModDto {
            patcher,
            placeable: m.layout == umm_core::ModLayout::Raw && m.thunderstore.is_none(),
            installer: m
                .fomod
                .as_ref()
                .map(|f| if f.done { "done" } else { "waiting" }),
            placement: m.placement.clone().map(Into::into),
            thunderstore: m.thunderstore.as_ref().map(|t| t.full_name.clone()),
            nexus_url: m
                .nexus
                .as_ref()
                .map(|n| umm_nexus::mod_url(&n.domain, n.mod_id)),
            files: m.files.len(),
            size: m.files.values().map(|f| f.size).sum(),
            id: m.id,
            name: m.name,
            version: m.version,
            source: m.source,
            imported_unix: m.imported_unix,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameDetails {
    pub game: GameDto,
    pub profiles: Vec<String>,
    pub mods: Vec<ModDto>,
    pub data_dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    pub files: Vec<PlanFile>,
    pub conflicts: Vec<ConflictDto>,
    pub skipped: Vec<SkippedDto>,
    /// The game's plugin list while deployed, for games that keep one.
    pub plugins: Option<PluginListDto>,
    /// Ini settings set while deployed.
    pub settings: Vec<IniSettingDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IniSettingDto {
    pub path: String,
    pub section: String,
    pub key: String,
    pub value: String,
    pub current: Option<String>,
    pub exists: bool,
    pub create: bool,
}

impl From<umm_core::IniPreview> for IniSettingDto {
    fn from(s: umm_core::IniPreview) -> Self {
        IniSettingDto {
            path: s.path.display().to_string(),
            section: s.section,
            key: s.key,
            value: s.value,
            current: s.current,
            exists: s.exists,
            create: s.create,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginListDto {
    pub path: String,
    /// `plugins`; `mods` when it lists mods (RimWorld); `dated` when the game loads its
    /// plugins in the order of their file dates instead (Morrowind).
    pub kind: &'static str,
    pub entries: Vec<PluginEntryDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntryDto {
    pub name: String,
    pub enabled: bool,
    /// The mod providing it; absent for the user's own entries.
    pub owner: Option<String>,
}

impl From<umm_core::PluginList> for PluginListDto {
    fn from(l: umm_core::PluginList) -> Self {
        PluginListDto {
            path: l.path.display().to_string(),
            kind: match l.format {
                umm_core::PluginListFormat::RimworldXml => "mods",
                umm_core::PluginListFormat::MorrowindIni => "dated",
                _ => "plugins",
            },
            entries: l
                .entries
                .into_iter()
                .map(|e| PluginEntryDto {
                    name: e.name,
                    enabled: e.enabled,
                    owner: e.owner,
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFile {
    pub target: String,
    pub owner: String,
    pub writable: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictDto {
    pub target: String,
    pub winner: String,
    pub overridden: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedDto {
    pub owner: String,
    pub file: String,
    pub reason: String,
}

impl From<Plan> for PlanDto {
    fn from(p: Plan) -> Self {
        PlanDto {
            files: p
                .files
                .into_iter()
                .map(|f| PlanFile {
                    target: f.target.to_string(),
                    owner: f.owner,
                    writable: f.writable,
                })
                .collect(),
            conflicts: p
                .conflicts
                .into_iter()
                .map(|c| ConflictDto {
                    target: c.target.to_string(),
                    winner: c.winner,
                    overridden: c.overridden,
                })
                .collect(),
            skipped: p
                .skipped
                .into_iter()
                .map(|s| SkippedDto {
                    owner: s.owner,
                    file: s.file,
                    reason: s.reason,
                })
                .collect(),
            plugins: None,
            settings: Vec::new(),
        }
    }
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportDto {
    pub restored: usize,
    pub removed: usize,
    pub quarantined: Vec<String>,
    pub kept_game_versions: Vec<String>,
    pub dirs_left: Vec<String>,
    pub errors: Vec<String>,
}

impl From<RevertReport> for ReportDto {
    fn from(r: RevertReport) -> Self {
        ReportDto {
            restored: r.restored,
            removed: r.removed,
            quarantined: r.quarantined,
            kept_game_versions: r.kept_game_versions,
            dirs_left: r.dirs_left,
            errors: r.errors,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncDto {
    pub placed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub kept_edited: Vec<String>,
}

impl From<SyncReport> for SyncDto {
    fn from(s: SyncReport) -> Self {
        SyncDto {
            placed: s.placed,
            unchanged: s.unchanged,
            removed: s.removed,
            kept_edited: s.kept_edited,
        }
    }
}

/// Payload of the `session` event.
#[derive(Serialize, Clone)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionEvent {
    Recovered {
        game: String,
        report: ReportDto,
    },
    Deployed {
        game: String,
        game_files: usize,
        runtime: Option<SyncDto>,
    },
    Started {
        game: String,
        pid: u32,
    },
    Patched {
        game: String,
        files: Vec<String>,
    },
    ExtenderLoader {
        game: String,
        loader: String,
        started_steam: bool,
    },
    WaitingForGame {
        game: String,
    },
    GameDetected {
        game: String,
    },
    Exited {
        game: String,
        code: Option<i32>,
    },
    Reverted {
        game: String,
        report: ReportDto,
    },
    Finished {
        game: String,
        abandoned: bool,
        appeared: bool,
        seconds: u64,
    },
    Failed {
        game: String,
        message: String,
    },
}

impl SessionEvent {
    pub fn from_launch(game: &str, e: LaunchEvent) -> Self {
        let game = game.to_string();
        match e {
            LaunchEvent::Recovered(r) => SessionEvent::Recovered {
                game,
                report: r.into(),
            },
            LaunchEvent::Deployed {
                game_files,
                runtime,
            } => SessionEvent::Deployed {
                game,
                game_files,
                runtime: runtime.map(Into::into),
            },
            LaunchEvent::Started { pid } => SessionEvent::Started { game, pid },
            LaunchEvent::Patched { files } => SessionEvent::Patched { game, files },
            LaunchEvent::ExtenderLoader {
                loader,
                started_steam,
            } => SessionEvent::ExtenderLoader {
                game,
                loader,
                started_steam,
            },
            LaunchEvent::WaitingForGame => SessionEvent::WaitingForGame { game },
            LaunchEvent::GameDetected => SessionEvent::GameDetected { game },
            LaunchEvent::Exited { code } => SessionEvent::Exited { game, code },
            LaunchEvent::Reverted(r) => SessionEvent::Reverted {
                game,
                report: r.into(),
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingDto {
    pub level: &'static str,
    pub message: String,
}

impl From<Finding> for FindingDto {
    fn from(f: Finding) -> Self {
        FindingDto {
            level: match f.level {
                Level::Ok => "ok",
                Level::Info => "info",
                Level::Warn => "warn",
                Level::Error => "error",
            },
            message: f.message,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryDto {
    pub game: String,
    pub ok: bool,
    pub message: String,
    pub report: Option<ReportDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupDto {
    pub data_dir: String,
    pub recovery: Vec<RecoveryDto>,
    pub def_errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupOption {
    pub value: LoaderSetup,
    pub label: String,
}

/// `ts_loader`: the Thunderstore loader kind the Thunderstore setup would use.
fn options(setups: &[LoaderSetup], ts_loader: Option<&str>) -> Vec<SetupOption> {
    setups
        .iter()
        .map(|&value| SetupOption {
            value,
            label: value.label(ts_loader),
        })
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamGameDto {
    pub app_id: u32,
    pub name: String,
    pub path: String,
    pub engine: &'static str,
    pub defined_as: Option<String>,
    pub community: Option<String>,
    pub other_loader: Option<String>,
    /// Default first.
    pub setups: Vec<SetupOption>,
}

impl From<umm_core::SteamCandidate> for SteamGameDto {
    fn from(c: umm_core::SteamCandidate) -> Self {
        SteamGameDto {
            app_id: c.app.app_id,
            name: c.app.name,
            path: c.app.path.display().to_string(),
            engine: c.engine.label(),
            defined_as: c.defined_as,
            community: c.community,
            other_loader: c.other_loader,
            setups: options(&c.setups, c.ts_loader.as_deref()),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamListDto {
    pub games: Vec<SteamGameDto>,
    pub thunderstore_error: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamPick {
    pub app_id: u32,
    pub setup: Option<LoaderSetup>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInfoDto {
    pub folder: String,
    pub engine: &'static str,
    pub name: String,
    pub exes: Vec<String>,
    pub community: Option<String>,
    pub other_loader: Option<String>,
    /// Default first.
    pub setups: Vec<SetupOption>,
    /// Set when the folder is a Steam game's install.
    pub steam_app_id: Option<u32>,
    pub steam_name: Option<String>,
}

impl From<umm_core::FolderInfo> for FolderInfoDto {
    fn from(f: umm_core::FolderInfo) -> Self {
        FolderInfoDto {
            folder: f.folder.display().to_string(),
            engine: f.engine.label(),
            name: f.name,
            exes: f.exes,
            community: f.community,
            other_loader: f.other_loader,
            setups: options(&f.setups, f.ts_loader.as_deref()),
            steam_app_id: f.steam_app.as_ref().map(|s| s.0),
            steam_name: f.steam_app.map(|s| s.1),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderChoiceDto {
    /// `None`: the definition's own loader, which no setup describes.
    pub current: Option<LoaderSetup>,
    pub current_label: String,
    pub available: Vec<SetupOption>,
    pub community: Option<String>,
    pub other_loader: Option<String>,
}

impl From<LoaderChoice> for LoaderChoiceDto {
    fn from(c: LoaderChoice) -> Self {
        LoaderChoiceDto {
            current: c.current,
            current_label: c
                .current
                .map(|s| s.label(c.ts_loader.as_deref()))
                .unwrap_or_else(|| "Set by hand in game.toml".into()),
            available: options(&c.available, c.ts_loader.as_deref()),
            community: c.community,
            other_loader: c.other_loader,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedGameDto {
    pub app_id: u32,
    pub ok: bool,
    pub id: Option<String>,
    /// The game's display name, once added.
    pub name: Option<String>,
    pub message: String,
    pub notes: Vec<umm_core::Note>,
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TsQueryDto {
    pub text: String,
    pub category: Option<String>,
    /// "rating", "downloads", "updated" or "name".
    pub sort: Option<String>,
    pub include_deprecated: bool,
    pub include_nsfw: bool,
    pub installed_only: bool,
    pub offset: usize,
    pub limit: usize,
}

impl From<TsQueryDto> for umm_core::TsQuery {
    fn from(q: TsQueryDto) -> Self {
        use umm_core::TsSort;
        umm_core::TsQuery {
            text: q.text,
            category: q.category.filter(|c| !c.is_empty()),
            sort: match q.sort.as_deref() {
                Some("downloads") => TsSort::Downloads,
                Some("updated") => TsSort::Updated,
                Some("name") => TsSort::Name,
                _ => TsSort::Rating,
            },
            include_deprecated: q.include_deprecated,
            include_nsfw: q.include_nsfw,
            installed_only: q.installed_only,
            offset: q.offset,
            limit: q.limit,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsPackageDto {
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub website: String,
    pub categories: Vec<String>,
    pub rating: i64,
    pub downloads: u64,
    pub deprecated: bool,
    pub latest: Option<String>,
    pub size: u64,
    pub updated: String,
    pub installed: Option<String>,
    pub update: bool,
    pub mod_id: String,
}

impl From<umm_core::TsItem> for TsPackageDto {
    fn from(i: umm_core::TsItem) -> Self {
        let p = i.package;
        let latest = p.latest().cloned();
        TsPackageDto {
            mod_id: umm_core::package_mod_id(&p.full_name),
            latest: latest.as_ref().map(|v| v.version.clone()),
            size: latest.map(|v| v.size).unwrap_or(0),
            full_name: p.full_name,
            owner: p.owner,
            name: p.name,
            description: p.description,
            icon: p.icon,
            website: p.website,
            categories: p.categories,
            rating: p.rating,
            downloads: p.downloads,
            deprecated: p.deprecated,
            updated: p.updated,
            installed: i.installed,
            update: i.update,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsPageDto {
    pub total: usize,
    pub items: Vec<TsPackageDto>,
    pub categories: Vec<String>,
    pub stale: bool,
    /// The game's mod loader, by name ("BepInEx", "Shimloader").
    pub loader_name: String,
    pub loader_package: Option<String>,
    pub loader_installed: bool,
    /// The installed BepInEx pack cannot load in this game, and why.
    pub loader_warning: Option<String>,
    /// A known pack for the game's build to pre-fill when offering an unlisted install.
    pub loader_suggestion: Option<String>,
    /// Installed packages with a newer version: (Owner-Name, installed, latest).
    pub updates: Vec<(String, String, String)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsInstalledDto {
    pub full_name: String,
    pub version: String,
    pub replaced: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsReportDto {
    pub installed: Vec<TsInstalledDto>,
    pub missing: Vec<String>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

/// A version of a package, offered to change to.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsVersionDto {
    pub version: String,
    pub size: u64,
    pub installed: bool,
    /// Installed packages needing a newer version than this one.
    pub needed_by: Vec<TsNeedDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsNeedDto {
    pub name: String,
    pub version: String,
}

impl From<umm_core::TsVersionChoice> for TsVersionDto {
    fn from(c: umm_core::TsVersionChoice) -> Self {
        TsVersionDto {
            version: c.version,
            size: c.size,
            installed: c.installed,
            needed_by: c
                .needed_by
                .into_iter()
                .map(|(name, version)| TsNeedDto { name, version })
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsLookupDto {
    pub package: TsPackageDto,
    pub listed: bool,
    pub review_status: Option<String>,
}

impl From<umm_core::TsLookup> for TsLookupDto {
    fn from(l: umm_core::TsLookup) -> Self {
        TsLookupDto {
            package: umm_core::TsItem {
                package: l.package,
                installed: None,
                update: false,
            }
            .into(),
            listed: l.listed,
            review_status: l.review_status,
        }
    }
}

impl From<umm_core::TsInstallReport> for TsReportDto {
    fn from(r: umm_core::TsInstallReport) -> Self {
        TsReportDto {
            installed: r
                .installed
                .into_iter()
                .map(|(full_name, version, replaced)| TsInstalledDto {
                    full_name,
                    version,
                    replaced,
                })
                .collect(),
            missing: r.missing,
            warnings: r.warnings,
            notes: r.notes,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreviewDto {
    pub mods: usize,
    pub files: Vec<String>,
    pub left_out: Vec<String>,
    pub size: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDto {
    pub profile: String,
    pub installed: TsReportDto,
    pub notes: Vec<String>,
    pub files: usize,
    pub skipped: Vec<String>,
}

impl From<umm_core::ProfileImport> for ImportDto {
    fn from(i: umm_core::ProfileImport) -> Self {
        ImportDto {
            profile: i.profile,
            installed: i.installed.into(),
            notes: i.notes,
            files: i.files,
            skipped: i.skipped,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkHandlerDto {
    /// Link handling can be set up on this system.
    pub supported: bool,
    /// UMM handles every kind of link.
    pub ours: bool,
    /// Commands of other programs that handle some of them.
    pub others: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkGameDto {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkDto {
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub version: Option<String>,
    pub games: Vec<LinkGameDto>,
}

/// Payload of a store's install progress events (`ts-progress`, `nexus-progress`).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StoreProgressDto {
    pub game: String,
    pub message: String,
}

#[cfg(test)]
mod session_event_tests {
    use super::*;

    #[test]
    fn fields_are_camel_case_like_the_interface_reads_them() {
        let e = SessionEvent::Deployed {
            game: "g".into(),
            game_files: 3,
            runtime: None,
        };
        let v = serde_json::to_value(e).unwrap();
        assert_eq!(v["kind"], "deployed");
        assert_eq!(v["gameFiles"], 3);
    }
}
