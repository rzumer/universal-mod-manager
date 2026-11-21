//! Serializable shapes sent to the webview. Kept here so the engine crates stay free of
//! UI concerns.

use serde::Serialize;
use umm_core::{Finding, GameStatus, LaunchEvent, Level, ModMeta, Plan, RevertReport, SyncReport};

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
}

impl GameDto {
    pub fn new(s: GameStatus, running: bool) -> Self {
        let loader = s.def.loader.as_ref();
        GameDto {
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
}

impl From<ModMeta> for ModDto {
    fn from(m: ModMeta) -> Self {
        ModDto {
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
#[serde(tag = "kind", rename_all = "camelCase")]
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
    pub gamedefs_dir: String,
    pub recovery: Vec<RecoveryDto>,
    pub def_errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamGameDto {
    pub app_id: u32,
    pub name: String,
    pub path: String,
    pub engine: &'static str,
    pub defined_as: Option<String>,
}

impl From<umm_core::SteamCandidate> for SteamGameDto {
    fn from(c: umm_core::SteamCandidate) -> Self {
        SteamGameDto {
            app_id: c.app.app_id,
            name: c.app.name,
            path: c.app.path.display().to_string(),
            engine: c.engine.label(),
            defined_as: c.defined_as,
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
