//! Tauri commands for Nexus Mods.

use crate::commands::{blocking, open_url, Res};
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime, State};
use umm_core::{
    CoreError, DownloadWatch, ExpectedFile, NexusAccount, NexusFile, NexusInstallReport,
    NexusModFile, NexusSearch, NexusSort, NexusUpdateState, Session,
};

/// A download the user is making on the website, which UMM waits for.
pub struct PendingDownload {
    pub game: String,
    pub expected: ExpectedFile,
    pub watch: DownloadWatch,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusAccountDto {
    pub name: String,
    pub premium: bool,
}

impl From<NexusAccount> for NexusAccountDto {
    fn from(a: NexusAccount) -> Self {
        NexusAccountDto {
            name: a.name,
            premium: a.is_premium,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusGameDto {
    /// `None`: UMM does not know the game's Nexus game.
    pub domain: Option<String>,
    pub name: Option<String>,
    pub mod_count: Option<u64>,
    /// Found by name, not saved in `game.toml` yet.
    pub guessed: bool,
    pub url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusGameChoiceDto {
    pub domain: String,
    pub name: String,
    pub mod_count: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusItemDto {
    pub mod_id: u64,
    pub name: String,
    pub summary: String,
    pub author: Option<String>,
    pub version: String,
    pub endorsements: u64,
    pub downloads: u64,
    pub updated_at: String,
    pub category: String,
    pub thumbnail: Option<String>,
    pub url: String,
    /// Versions of its files that are installed.
    pub installed: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusPageDto {
    pub total: u64,
    pub items: Vec<NexusItemDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusQueryDto {
    pub text: String,
    /// `endorsements`, `downloads`, `updated` or `name`.
    pub sort: String,
    pub offset: u64,
    pub count: u64,
    /// Only mods with a file installed.
    pub installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusFileDto {
    pub file_id: u64,
    pub name: String,
    pub version: String,
    pub category: String,
    pub size: Option<u64>,
    pub date: u64,
    pub description: Option<String>,
    pub old: bool,
    pub installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusRequirementDto {
    pub name: String,
    /// On Nexus for this game: UMM can get it.
    pub mod_id: Option<u64>,
    pub notes: Option<String>,
    pub url: Option<String>,
    pub installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusFilesDto {
    pub domain: String,
    pub mod_id: u64,
    pub name: String,
    pub files: Vec<NexusFileDto>,
    pub requirements: Vec<NexusRequirementDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusInstallDto {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub replaced: bool,
}

impl From<NexusInstallReport> for NexusInstallDto {
    fn from(r: NexusInstallReport) -> Self {
        NexusInstallDto {
            id: r.meta.id,
            name: r.meta.name,
            version: r.meta.version,
            replaced: r.replaced,
        }
    }
}

/// The outcome of installing a downloaded archive: installed, or not recognised by Nexus
/// (the interface then asks whether it is the file the user downloaded).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusDownloadResultDto {
    pub installed: Option<NexusInstallDto>,
    pub unrecognised: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusUpdateDto {
    pub id: String,
    pub name: String,
    pub mod_id: u64,
    pub version: Option<String>,
    /// `current`, `newer`, `outdated` or `gone`.
    pub state: &'static str,
    pub new_file_id: Option<u64>,
    pub new_version: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusLinkGameDto {
    pub id: String,
    pub name: String,
    /// The game is the link's Nexus game (set, or by name).
    pub matches: bool,
}

/// What an `nxm://` link names, and the games it can go into.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusLinkDto {
    pub domain: String,
    pub mod_name: Option<String>,
    pub file_name: Option<String>,
    pub version: Option<String>,
    pub expired: bool,
    pub signed_in: bool,
    pub games: Vec<NexusLinkGameDto>,
}

/// Payload of the `nexus-progress` event.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NexusProgressDto {
    pub game: String,
    pub message: String,
}

fn progress_to<R: Runtime>(app: AppHandle<R>, game: String) -> impl FnMut(&str) {
    move |m: &str| {
        let _ = app.emit(
            "nexus-progress",
            NexusProgressDto {
                game: game.clone(),
                message: m.to_string(),
            },
        );
    }
}

fn file_dto(f: &NexusModFile, installed: &[u64]) -> NexusFileDto {
    NexusFileDto {
        file_id: f.file_id,
        name: f.name.clone(),
        version: f.version.clone(),
        category: f.category.clone(),
        size: f.size_in_bytes,
        date: f.date,
        description: f.description.clone(),
        old: f.is_old(),
        installed: installed.contains(&f.file_id),
    }
}

#[tauri::command]
pub async fn nexus_account(state: State<'_, AppState>) -> Res<Option<NexusAccountDto>> {
    blocking(&state, |s| Ok(s.nexus_account().map(Into::into))).await
}

#[tauri::command]
pub async fn nexus_sign_in(state: State<'_, AppState>, key: String) -> Res<NexusAccountDto> {
    blocking(&state, move |s| Ok(s.nexus_sign_in(&key)?.into())).await
}

#[tauri::command]
pub async fn nexus_sign_out(state: State<'_, AppState>) -> Res<()> {
    blocking(&state, |s| s.nexus_sign_out()).await
}

fn game_dto(s: &Session, game: &str) -> Result<NexusGameDto, CoreError> {
    let g = s.nexus_game(game)?;
    Ok(NexusGameDto {
        url: g.as_ref().map(|g| umm_nexus::game_url(&g.domain)),
        guessed: g.as_ref().is_some_and(|g| g.guessed),
        name: g.as_ref().and_then(|g| g.name.clone()),
        mod_count: g.as_ref().and_then(|g| g.mod_count),
        domain: g.map(|g| g.domain),
    })
}

#[tauri::command]
pub async fn nexus_game(state: State<'_, AppState>, game: String) -> Res<NexusGameDto> {
    blocking(&state, move |s| game_dto(s, &game)).await
}

#[tauri::command]
pub async fn nexus_find_games(
    state: State<'_, AppState>,
    name: String,
) -> Res<Vec<NexusGameChoiceDto>> {
    blocking(&state, move |s| {
        Ok(s.nexus_find_games(&name)?
            .into_iter()
            .map(|g| NexusGameChoiceDto {
                domain: g.domain_name,
                name: g.name,
                mod_count: g.mod_count,
            })
            .collect())
    })
    .await
}

/// Sets the game's Nexus game; an empty `domain` removes it.
#[tauri::command]
pub async fn set_nexus_domain(
    state: State<'_, AppState>,
    game: String,
    domain: String,
) -> Res<NexusGameDto> {
    blocking(&state, move |s| {
        let d = domain.trim().to_lowercase();
        s.set_nexus_domain(&game, Some(d.as_str()).filter(|d| !d.is_empty()))?;
        game_dto(s, &game)
    })
    .await
}

#[tauri::command]
pub async fn nexus_search(
    state: State<'_, AppState>,
    game: String,
    query: NexusQueryDto,
) -> Res<NexusPageDto> {
    blocking(&state, move |s| {
        let sort = match query.sort.as_str() {
            "downloads" => NexusSort::Downloads,
            "updated" => NexusSort::Updated,
            "name" => NexusSort::Name,
            _ => NexusSort::Endorsements,
        };
        let page = s.nexus_search(
            &game,
            &NexusSearch {
                text: query.text,
                sort,
                offset: query.offset,
                count: query.count,
                include_adult: true,
            },
            query.installed,
        )?;
        let domain = page.game.domain.clone();
        Ok(NexusPageDto {
            total: page.total,
            items: page
                .items
                .into_iter()
                .map(|it| {
                    let m = it.summary;
                    NexusItemDto {
                        url: umm_nexus::mod_url(&domain, m.mod_id),
                        mod_id: m.mod_id,
                        name: m.name,
                        summary: m.summary,
                        author: m.author,
                        version: m.version,
                        endorsements: m.endorsements,
                        downloads: m.downloads,
                        updated_at: m.updated_at,
                        category: m.category,
                        thumbnail: m.thumbnail_url,
                        installed: it
                            .installed
                            .into_iter()
                            .map(|(id, v)| v.unwrap_or(id))
                            .collect(),
                    }
                })
                .collect(),
        })
    })
    .await
}

#[tauri::command]
pub async fn nexus_files(
    state: State<'_, AppState>,
    game: String,
    mod_id: u64,
) -> Res<NexusFilesDto> {
    blocking(&state, move |s| {
        let f = s.nexus_files(&game, mod_id)?;
        Ok(NexusFilesDto {
            files: f.files.iter().map(|x| file_dto(x, &f.installed)).collect(),
            requirements: f
                .requirements
                .into_iter()
                .map(|r| NexusRequirementDto {
                    name: r.name,
                    mod_id: r.mod_id,
                    notes: r.notes,
                    url: r.url,
                    installed: r.installed,
                })
                .collect(),
            domain: f.domain,
            mod_id,
            name: f.summary.name,
        })
    })
    .await
}

/// Starts a download on the website: opens the file's page in the browser and starts
/// watching the Downloads folder for it. Returns the folder watched.
#[tauri::command]
pub async fn nexus_download_start(
    state: State<'_, AppState>,
    game: String,
    domain: String,
    mod_id: u64,
    file_id: u64,
) -> Res<String> {
    let dir = umm_core::downloads_dir().ok_or("cannot find your Downloads folder")?;
    let watch = DownloadWatch::new(&dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
    let url = Session::nexus_download_page(&domain, mod_id, file_id);
    *state.nexus_download.lock().map_err(|e| e.to_string())? = Some(PendingDownload {
        game,
        expected: ExpectedFile {
            domain,
            mod_id,
            file_id,
        },
        watch,
    });
    open_url(&url)?;
    Ok(dir.display().to_string())
}

/// The archive that has appeared for the pending download, if any.
#[tauri::command]
pub fn nexus_download_poll(state: State<'_, AppState>) -> Res<Option<String>> {
    let slot = state.nexus_download.lock().map_err(|e| e.to_string())?;
    Ok(slot
        .as_ref()
        .and_then(|p| p.watch.poll())
        .map(|p| p.display().to_string()))
}

#[tauri::command]
pub fn nexus_download_cancel(state: State<'_, AppState>) -> Res<()> {
    *state.nexus_download.lock().map_err(|e| e.to_string())? = None;
    Ok(())
}

/// Installs the pending download's archive at `path` (found by the watch, or picked by the
/// user). `trust`: the user confirmed it is the file they downloaded, though Nexus does not
/// know its hash.
#[tauri::command]
pub async fn nexus_download_install<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    profile: String,
    path: String,
    trust: bool,
) -> Res<NexusDownloadResultDto> {
    let (game, expected) = {
        let slot = state.nexus_download.lock().map_err(|e| e.to_string())?;
        let p = slot.as_ref().ok_or("no download is waiting")?;
        (p.game.clone(), p.expected.clone())
    };
    let pending = Arc::clone(&state.nexus_download);
    blocking(&state, move |s| {
        let mut progress = progress_to(app, game.clone());
        let path = PathBuf::from(path);
        match s.nexus_install_download(
            &game,
            &path,
            Some(&expected),
            trust,
            &profile,
            &mut progress,
        ) {
            Err(CoreError::NexusUnrecognised) => Ok(NexusDownloadResultDto {
                installed: None,
                unrecognised: true,
            }),
            r => {
                let r = r?;
                if let Ok(mut slot) = pending.lock() {
                    *slot = None;
                }
                Ok(NexusDownloadResultDto {
                    installed: Some(r.into()),
                    unrecognised: false,
                })
            }
        }
    })
    .await
}

#[tauri::command]
pub async fn nexus_resolve_link(state: State<'_, AppState>, url: String) -> Res<NexusLinkDto> {
    blocking(&state, move |s| {
        let l = umm_core::NxmLink::parse(&url).ok_or_else(|| {
            CoreError::Invalid("not a Nexus Mods download link UMM can use".into())
        })?;
        // Names are a nicety: the link still installs if they cannot be looked up here.
        let (mod_name, file) = match s.nexus_link_info(&l) {
            Ok((n, f)) => (Some(n), f),
            Err(_) => (None, None),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        Ok(NexusLinkDto {
            games: s
                .nexus_link_games(&l.domain)?
                .into_iter()
                .map(|(id, name, matches)| NexusLinkGameDto { id, name, matches })
                .collect(),
            mod_name,
            file_name: file.as_ref().map(|f| f.name.clone()),
            version: file.map(|f| f.version),
            expired: l.expired(now),
            signed_in: s.nexus_account().is_some(),
            domain: l.domain,
        })
    })
    .await
}

/// Installs the file an `nxm://` link names. Progress arrives as `nexus-progress` events.
#[tauri::command]
pub async fn nexus_install_link<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    profile: String,
    url: String,
) -> Res<NexusInstallDto> {
    blocking(&state, move |s| {
        let mut progress = progress_to(app, game.clone());
        Ok(s.nexus_install_link(&game, &url, &profile, &mut progress)?
            .into())
    })
    .await
}

/// Installs a file straight from Nexus (Premium accounts).
#[tauri::command]
pub async fn nexus_install_file<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    profile: String,
    domain: String,
    mod_id: u64,
    file_id: u64,
) -> Res<NexusInstallDto> {
    blocking(&state, move |s| {
        let mut progress = progress_to(app, game.clone());
        let f = NexusFile {
            domain,
            mod_id,
            file_id,
            credentials: None,
        };
        Ok(s.nexus_install(&game, &f, &profile, &mut progress)?.into())
    })
    .await
}

#[tauri::command]
pub async fn nexus_updates(state: State<'_, AppState>, game: String) -> Res<Vec<NexusUpdateDto>> {
    blocking(&state, move |s| {
        Ok(s.nexus_updates(&game)?
            .into_iter()
            .map(|u| {
                let (state, new_file_id, new_version) = match u.state {
                    NexusUpdateState::Current => ("current", None, None),
                    NexusUpdateState::Newer {
                        file_id, version, ..
                    } => ("newer", Some(file_id), Some(version)),
                    NexusUpdateState::Outdated => ("outdated", None, None),
                    NexusUpdateState::Gone => ("gone", None, None),
                };
                NexusUpdateDto {
                    id: u.id,
                    name: u.name,
                    mod_id: u.nexus.mod_id,
                    version: u.nexus.version,
                    state,
                    new_file_id,
                    new_version,
                }
            })
            .collect())
    })
    .await
}

/// Installs the newest version of an installed Nexus file (Premium accounts).
#[tauri::command]
pub async fn nexus_update<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    profile: String,
    mod_id: String,
) -> Res<Option<NexusInstallDto>> {
    blocking(&state, move |s| {
        let mut progress = progress_to(app, game.clone());
        Ok(s.nexus_update(&game, &mod_id, &profile, &mut progress)?
            .map(Into::into))
    })
    .await
}

/// Opens the web page of a requirement hosted outside Nexus. Only http(s) addresses, with
/// nothing a command line could split.
#[tauri::command]
pub fn open_web_page(url: String) -> Res<()> {
    let ok = (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() < 2048
        && !url.contains(['"', ' ', '\n', '\r', '\\', '<', '>', '^', '|', '`']);
    if !ok {
        return Err(format!("not a web page UMM can open: {url}"));
    }
    open_url(&url)
}

/// Opens a page on nexusmods.com in the browser. Other sites are refused.
#[tauri::command]
pub fn open_nexus_page(url: String) -> Res<()> {
    let ok = url.starts_with(&format!("{}/", umm_nexus::SITE_URL))
        && !url.contains(['"', ' ', '\n', '\r', '\\']);
    if !ok {
        return Err(format!("not a Nexus Mods page: {url}"));
    }
    open_url(&url)
}
