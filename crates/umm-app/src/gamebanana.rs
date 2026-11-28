//! Tauri commands for GameBanana.

use crate::commands::{blocking, open_url, store_progress, Res};
use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State};
use umm_core::{GbGame, GbInstallReport, GbSearch, GbSort, GbUpdateState, Session};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GbGameDto {
    /// `None`: UMM does not know the game's GameBanana game.
    pub game_id: Option<u64>,
    pub name: Option<String>,
    pub mod_count: Option<u64>,
    /// Found by name, not saved in `game.toml` yet.
    pub guessed: bool,
    pub url: Option<String>,
}

impl From<Option<GbGame>> for GbGameDto {
    fn from(g: Option<GbGame>) -> Self {
        match g {
            Some(g) => GbGameDto {
                url: Some(umm_sources::gamebanana::game_url(g.game_id)),
                game_id: Some(g.game_id),
                name: g.name,
                mod_count: g.mod_count,
                guessed: g.guessed,
            },
            None => GbGameDto {
                game_id: None,
                name: None,
                mod_count: None,
                guessed: false,
                url: None,
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GbGameChoiceDto {
    pub game_id: u64,
    pub name: String,
    pub icon: Option<String>,
    pub url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GbQueryDto {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub sort: String,
    #[serde(default = "first_page")]
    pub page: u32,
    #[serde(default)]
    pub installed: bool,
}

fn first_page() -> u32 {
    1
}

fn gb_sort(s: &str) -> GbSort {
    match s {
        "likes" => GbSort::Likes,
        "updated" => GbSort::Updated,
        "newest" => GbSort::Newest,
        "name" => GbSort::Name,
        _ => GbSort::Downloads,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GbItemDto {
    pub mod_id: u64,
    pub name: String,
    pub author: Option<String>,
    pub version: String,
    pub category: String,
    pub thumbnail: Option<String>,
    pub likes: u64,
    pub views: u64,
    /// Unix time.
    pub updated: u64,
    pub rated: bool,
    pub obsolete: bool,
    pub has_files: bool,
    pub url: String,
    /// Versions of its files that are installed.
    pub installed: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GbPageDto {
    pub total: u64,
    /// No further pages.
    pub complete: bool,
    pub items: Vec<GbItemDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GbFileDto {
    pub file_id: u64,
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub size: u64,
    /// Unix time.
    pub added: u64,
    pub downloads: u64,
    pub clean: Option<bool>,
    pub contains_exe: Vec<String>,
    /// The platform it is for, when not Windows (`Mac`, `Linux`, ...).
    pub platform: Option<String>,
    /// The UMM mod holding this file, when it is installed.
    pub installed: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GbFilesDto {
    pub mod_id: u64,
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub text: String,
    pub downloads: u64,
    pub url: String,
    pub files: Vec<GbFileDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GbInstallDto {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub replaced: bool,
    /// The game's GameBanana game was saved by this install.
    pub game_set: bool,
    /// UMM could not tell where the files go: the interface asks.
    pub placement_guessed: bool,
    /// Its FOMOD installer's options need choosing.
    pub installer_waiting: bool,
}

impl From<GbInstallReport> for GbInstallDto {
    fn from(r: GbInstallReport) -> Self {
        GbInstallDto {
            placement_guessed: r.meta.placement.as_ref().is_some_and(|p| p.guessed),
            installer_waiting: r.meta.fomod.as_ref().is_some_and(|f| !f.done),
            id: r.meta.id,
            name: r.meta.name,
            version: r.meta.version,
            replaced: r.replaced,
            game_set: r.game_set,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GbUpdateDto {
    pub id: String,
    pub name: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub version: Option<String>,
    /// `current`, `newer` or `gone`.
    pub state: &'static str,
    pub new_file_id: Option<u64>,
    pub new_version: Option<String>,
}

fn game_dto(s: &Session, game: &str) -> Result<GbGameDto, umm_core::CoreError> {
    Ok(s.gamebanana_game(game)?.into())
}

/// The game's GameBanana game, set or found by name.
#[tauri::command]
pub async fn gb_game(state: State<'_, AppState>, game: String) -> Res<GbGameDto> {
    blocking(&state, move |s| game_dto(s, &game)).await
}

#[tauri::command]
pub async fn gb_find_games(state: State<'_, AppState>, name: String) -> Res<Vec<GbGameChoiceDto>> {
    blocking(&state, move |s| {
        Ok(s.gamebanana_find_games(&name)?
            .into_iter()
            .map(|g| GbGameChoiceDto {
                url: umm_sources::gamebanana::game_url(g.id),
                game_id: g.id,
                name: g.name,
                icon: g.icon_url,
            })
            .collect())
    })
    .await
}

/// Saves the game's GameBanana game (`None` forgets it).
#[tauri::command]
pub async fn set_gb_game(
    state: State<'_, AppState>,
    game: String,
    game_id: Option<u64>,
) -> Res<GbGameDto> {
    blocking(&state, move |s| {
        s.set_gamebanana_game(&game, game_id)?;
        game_dto(s, &game)
    })
    .await
}

#[tauri::command]
pub async fn gb_search(
    state: State<'_, AppState>,
    game: String,
    query: GbQueryDto,
) -> Res<GbPageDto> {
    blocking(&state, move |s| {
        let page = s.gamebanana_search(
            &game,
            &GbSearch {
                text: query.text,
                sort: gb_sort(&query.sort),
                page: query.page.max(1),
                per_page: 20,
            },
            query.installed,
        )?;
        Ok(GbPageDto {
            total: page.total,
            complete: page.complete,
            items: page
                .items
                .into_iter()
                .map(|it| {
                    let m = it.summary;
                    GbItemDto {
                        url: umm_sources::gamebanana::mod_url(m.mod_id),
                        mod_id: m.mod_id,
                        name: m.name,
                        author: m.author,
                        version: m.version,
                        category: m.category,
                        thumbnail: m.thumbnail_url,
                        likes: m.likes,
                        views: m.views,
                        updated: m.updated,
                        rated: m.rated,
                        obsolete: m.obsolete,
                        has_files: m.has_files,
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

/// A mod's description and files, with which are installed.
#[tauri::command]
pub async fn gb_files(state: State<'_, AppState>, game: String, mod_id: u64) -> Res<GbFilesDto> {
    blocking(&state, move |s| {
        let f = s.gamebanana_files(&game, mod_id)?;
        let d = f.details;
        Ok(GbFilesDto {
            url: umm_sources::gamebanana::mod_url(d.summary.mod_id),
            mod_id: d.summary.mod_id,
            name: d.summary.name,
            version: d.summary.version,
            author: d.summary.author,
            text: d.text,
            downloads: d.downloads,
            files: d
                .files
                .into_iter()
                .map(|x| GbFileDto {
                    installed: f
                        .installed
                        .iter()
                        .find(|(fid, _)| *fid == x.file_id)
                        .map(|(_, id)| id.clone()),
                    file_id: x.file_id,
                    name: x.name,
                    description: x.description,
                    version: x.version,
                    size: x.size,
                    added: x.added,
                    downloads: x.downloads,
                    clean: x.clean,
                    contains_exe: x.contains_exe,
                    platform: x.platform.map(String::from),
                })
                .collect(),
        })
    })
    .await
}

/// Downloads one file of a mod and installs it into the profile. Progress arrives as
/// `gb-progress` events.
#[tauri::command]
pub async fn gb_install<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    mod_id: u64,
    file_id: u64,
    profile: String,
) -> Res<GbInstallDto> {
    blocking(&state, move |s| {
        let mut progress = store_progress(app, "gb-progress", game.clone());
        Ok(
            s.gamebanana_install(&game, mod_id, file_id, &profile, &mut progress)?
                .into(),
        )
    })
    .await
}

/// Each installed GameBanana file, and whether a newer upload of it is out.
#[tauri::command]
pub async fn gb_updates(state: State<'_, AppState>, game: String) -> Res<Vec<GbUpdateDto>> {
    blocking(&state, move |s| {
        Ok(s.gamebanana_updates(&game)?
            .into_iter()
            .map(|u| {
                let (state, new_file_id, new_version) = match u.state {
                    GbUpdateState::Current => ("current", None, None),
                    GbUpdateState::Newer {
                        file_id, version, ..
                    } => ("newer", Some(file_id), Some(version)),
                    GbUpdateState::Gone => ("gone", None, None),
                };
                GbUpdateDto {
                    id: u.id,
                    name: u.name,
                    mod_id: u.mod_id,
                    file_id: u.file_id,
                    version: u.version,
                    state,
                    new_file_id,
                    new_version,
                }
            })
            .collect())
    })
    .await
}

/// Opens a page on gamebanana.com in the browser. Other sites are refused.
#[tauri::command]
pub fn open_gb_page(url: String) -> Res<()> {
    let ok = url.starts_with(&format!("{}/", umm_sources::gamebanana::SITE_URL))
        && url.len() < 2048
        && !url.contains(['"', ' ', '\n', '\r', '\\']);
    if !ok {
        return Err(format!("not a GameBanana page: {url}"));
    }
    open_url(&url)
}
