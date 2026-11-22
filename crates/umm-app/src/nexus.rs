//! Tauri commands for Nexus Mods.

use crate::commands::{blocking, open_url, Res};
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime, State};
use umm_core::{
    CoreError, DownloadWatch, ExpectedFile, NexusInstallReport, NexusModFile, NexusSearch,
    NexusSort, NexusUpdateState, Session,
};

/// A download the user is making on the website, which UMM waits for: in its own window
/// (caught there), or in their browser (picked up from the Downloads folder).
pub struct PendingDownload {
    pub game: String,
    pub expected: ExpectedFile,
    pub watch: DownloadWatch,
    /// Where the window saves downloads; deleted once the download is done with.
    pub temp: PathBuf,
    /// The archive the window downloaded.
    pub caught: Option<PathBuf>,
    /// The window's download failed.
    pub failed: bool,
    /// The user closed the window.
    pub closed: bool,
}

/// The label of the window showing nexusmods.com.
const DOWNLOAD_WINDOW: &str = "nexus-download";

/// What has happened to the pending download.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPollDto {
    /// An archive to install: caught in the window, or found in the Downloads folder.
    pub path: Option<String>,
    pub failed: bool,
    pub closed: bool,
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
    /// UMM could not tell where the files go: the interface asks.
    pub placement_guessed: bool,
}

impl From<NexusInstallReport> for NexusInstallDto {
    fn from(r: NexusInstallReport) -> Self {
        NexusInstallDto {
            placement_guessed: r.meta.placement.as_ref().is_some_and(|p| p.guessed),
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
    pub mod_id: u64,
    pub file_id: u64,
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

/// Starts a download on the website: opens the file's page in a UMM window, which catches
/// the download (or the mod-manager link) the page starts. The Downloads folder is watched
/// too, for when the user switches to their own browser. Returns the folder watched.
#[tauri::command]
pub async fn nexus_download_start<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    domain: String,
    mod_id: u64,
    file_id: u64,
) -> Res<String> {
    let dir = umm_core::downloads_dir().ok_or("cannot find your Downloads folder")?;
    let watch = DownloadWatch::new(&dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
    let url = Session::nexus_download_page(&domain, mod_id, file_id);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let temp = std::env::temp_dir()
        .join("umm-downloads")
        .join(format!("{}-{stamp}", std::process::id()));
    let previous = state
        .nexus_download
        .lock()
        .map_err(|e| e.to_string())?
        .replace(PendingDownload {
            game,
            expected: ExpectedFile {
                domain,
                mod_id,
                file_id,
            },
            watch,
            temp: temp.clone(),
            caught: None,
            failed: false,
            closed: false,
        });
    if let Some(p) = previous {
        let _ = std::fs::remove_dir_all(&p.temp);
    }
    open_download_window(&app, &url, Arc::clone(&state.nexus_download))?;
    Ok(dir.display().to_string())
}

/// Shows a page of nexusmods.com in UMM's download window. The page has no access to UMM:
/// only the main window may call its commands. UMM watches what it does instead: a download
/// is saved to the pending download's temporary folder and noted, and a mod-manager link is
/// noted rather than followed. The hooks read the pending download when they run, so a
/// reused window serves the next file.
fn open_download_window<R: Runtime>(
    app: &AppHandle<R>,
    url: &str,
    pending: Arc<std::sync::Mutex<Option<PendingDownload>>>,
) -> Res<()> {
    use tauri::webview::{DownloadEvent, NewWindowResponse};
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
    let parsed: tauri::Url = url.parse().map_err(|e| format!("{url}: {e}"))?;
    if let Some(w) = app.get_webview_window(DOWNLOAD_WINDOW) {
        // One window at a time: reuse it for the new file.
        w.navigate(parsed).map_err(|e| e.to_string())?;
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    let note = Arc::clone(&pending);
    let plain = app.clone();
    let on_close = Arc::clone(&pending);
    let opener = app.clone();
    let window = WebviewWindowBuilder::new(app, DOWNLOAD_WINDOW, WebviewUrl::External(parsed))
        .title("Download from Nexus Mods – UMM")
        .inner_size(1180.0, 860.0)
        .min_inner_size(640.0, 480.0)
        // "Mod Manager Download" makes Slow download produce an `nxm://` link, which only
        // an app signed in to Nexus could use: go to the file's plain download page instead,
        // where Slow download gives the file itself.
        .on_navigation(move |u| match u.scheme() {
            "nxm" => {
                if let Some(l) = umm_core::NxmLink::parse(u.as_str()) {
                    let page = Session::nexus_download_page(&l.domain, l.mod_id, l.file_id);
                    if let (Some(w), Ok(page)) =
                        (plain.get_webview_window(DOWNLOAD_WINDOW), page.parse())
                    {
                        let _ = w.navigate(page);
                    }
                }
                false
            }
            "https" | "about" | "blob" | "data" => true,
            _ => false,
        })
        // Links that open a new window (log-in pop-ups, some buttons) open in this one.
        .on_new_window(move |u, _| {
            if u.scheme() == "https" {
                if let Some(w) = opener.get_webview_window(DOWNLOAD_WINDOW) {
                    let _ = w.navigate(u);
                }
            }
            NewWindowResponse::Deny
        })
        .on_download(move |_, event| match event {
            DownloadEvent::Requested { url, destination } => {
                let Ok(slot) = note.lock() else { return false };
                let Some(p) = slot.as_ref() else { return false };
                let name = destination
                    .file_name()
                    .map(|n| n.to_owned())
                    .or_else(|| {
                        url.path_segments()
                            .and_then(|mut s| s.next_back().map(std::ffi::OsString::from))
                    })
                    .unwrap_or_else(|| "download".into());
                if std::fs::create_dir_all(&p.temp).is_err() {
                    return false;
                }
                *destination = p.temp.join(name);
                true
            }
            DownloadEvent::Finished { path, success, .. } => {
                if let Ok(mut slot) = note.lock() {
                    if let Some(p) = slot.as_mut() {
                        match path.filter(|_| success) {
                            Some(path) => p.caught = Some(path),
                            None => p.failed = true,
                        }
                    }
                }
                true
            }
            _ => true,
        })
        .build()
        .map_err(|e| format!("opening the Nexus Mods window: {e}"))?;
    window.on_window_event(move |e| {
        if let tauri::WindowEvent::Destroyed = e {
            if let Ok(mut slot) = on_close.lock() {
                if let Some(p) = slot.as_mut() {
                    p.closed = true;
                }
            }
        }
    });
    Ok(())
}

fn close_download_window<R: Runtime>(app: &AppHandle<R>) {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window(DOWNLOAD_WINDOW) {
        let _ = w.close();
    }
}

/// What has happened to the pending download.
#[tauri::command]
pub fn nexus_download_poll(state: State<'_, AppState>) -> Res<DownloadPollDto> {
    let mut slot = state.nexus_download.lock().map_err(|e| e.to_string())?;
    let Some(p) = slot.as_mut() else {
        return Ok(DownloadPollDto::default());
    };
    let path = p.caught.clone().or_else(|| p.watch.poll());
    Ok(DownloadPollDto {
        path: path.map(|p| p.display().to_string()),
        failed: std::mem::take(&mut p.failed),
        closed: p.closed,
    })
}

/// Opens the Nexus Mods window again for the pending download, after the user closed it.
/// Async like every command that creates a window: on Windows, a window created from a
/// synchronous command (run on the main thread) deadlocks the webview.
#[tauri::command]
pub async fn nexus_download_reopen<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Res<()> {
    let url = {
        let mut slot = state.nexus_download.lock().map_err(|e| e.to_string())?;
        let p = slot.as_mut().ok_or("no download is waiting")?;
        p.closed = false;
        Session::nexus_download_page(&p.expected.domain, p.expected.mod_id, p.expected.file_id)
    };
    open_download_window(&app, &url, Arc::clone(&state.nexus_download))
}

/// Opens the pending download's page in the user's own browser instead; the Downloads folder
/// is watched for it.
#[tauri::command]
pub async fn nexus_download_in_browser<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Res<()> {
    let url = {
        let slot = state.nexus_download.lock().map_err(|e| e.to_string())?;
        let p = slot.as_ref().ok_or("no download is waiting")?;
        Session::nexus_download_page(&p.expected.domain, p.expected.mod_id, p.expected.file_id)
    };
    close_download_window(&app);
    open_url(&url)
}

#[tauri::command]
pub async fn nexus_download_cancel<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Res<()> {
    if let Some(p) = state
        .nexus_download
        .lock()
        .map_err(|e| e.to_string())?
        .take()
    {
        let _ = std::fs::remove_dir_all(&p.temp);
    }
    close_download_window(&app);
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
    let (game, expected, temp) = {
        let slot = state.nexus_download.lock().map_err(|e| e.to_string())?;
        let p = slot.as_ref().ok_or("no download is waiting")?;
        (p.game.clone(), p.expected.clone(), p.temp.clone())
    };
    let pending = Arc::clone(&state.nexus_download);
    let window = app.clone();
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
                // A copy the window downloaded is UMM's to delete; one in the user's
                // Downloads folder is theirs.
                let _ = std::fs::remove_dir_all(&temp);
                close_download_window(&window);
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
        // Names are a nicety: the file can still be downloaded if they cannot be looked up.
        let (mod_name, file) = match s.nexus_link_info(&l) {
            Ok((n, f)) => (Some(n), f),
            Err(_) => (None, None),
        };
        Ok(NexusLinkDto {
            mod_id: l.mod_id,
            file_id: l.file_id,
            games: s
                .nexus_link_games(&l.domain)?
                .into_iter()
                .map(|(id, name, matches)| NexusLinkGameDto { id, name, matches })
                .collect(),
            mod_name,
            file_name: file.as_ref().map(|f| f.name.clone()),
            version: file.map(|f| f.version),
            domain: l.domain,
        })
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
