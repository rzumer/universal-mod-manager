//! Tauri commands for FOMOD installers.

use crate::commands::{blocking, Res};
use crate::dto::ModDto;
use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;
use umm_core::{FomodChoice, FomodGroupKind, FomodOptionKind, FomodView};

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceDto {
    pub step: String,
    pub group: String,
    pub plugins: Vec<String>,
}

impl From<ChoiceDto> for FomodChoice {
    fn from(c: ChoiceDto) -> Self {
        FomodChoice {
            step: c.step,
            group: c.group,
            plugins: c.plugins,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionDto {
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    /// required, recommended, optional, not_usable, could_be_usable
    pub kind: &'static str,
    pub selected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDto {
    pub name: String,
    /// exactly_one, at_most_one, at_least_one, all, any
    pub kind: &'static str,
    pub problem: Option<String>,
    pub options: Vec<OptionDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepDto {
    pub name: String,
    pub visible: bool,
    pub groups: Vec<GroupDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDto {
    pub source: String,
    pub destination: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FomodViewDto {
    pub name: String,
    pub image: Option<String>,
    pub steps: Vec<StepDto>,
    pub complete: bool,
    pub needs_unmet: bool,
    pub files: Vec<FileDto>,
    pub missing: Vec<String>,
    pub choices: Vec<ChoiceDto>,
    pub target: String,
    pub suggested_target: String,
    pub game_folders: Vec<String>,
}

fn group_kind(k: FomodGroupKind) -> &'static str {
    match k {
        FomodGroupKind::ExactlyOne => "exactly_one",
        FomodGroupKind::AtMostOne => "at_most_one",
        FomodGroupKind::AtLeastOne => "at_least_one",
        FomodGroupKind::All => "all",
        FomodGroupKind::Any => "any",
    }
}

fn option_kind(k: FomodOptionKind) -> &'static str {
    match k {
        FomodOptionKind::Required => "required",
        FomodOptionKind::Recommended => "recommended",
        FomodOptionKind::Optional => "optional",
        FomodOptionKind::NotUsable => "not_usable",
        FomodOptionKind::CouldBeUsable => "could_be_usable",
    }
}

impl From<FomodView> for FomodViewDto {
    fn from(v: FomodView) -> Self {
        FomodViewDto {
            name: v.name,
            image: v.image,
            steps: v
                .steps
                .into_iter()
                .map(|s| StepDto {
                    name: s.name,
                    visible: s.visible,
                    groups: s
                        .groups
                        .into_iter()
                        .map(|g| GroupDto {
                            name: g.name,
                            kind: group_kind(g.kind),
                            problem: g.problem,
                            options: g
                                .options
                                .into_iter()
                                .map(|o| OptionDto {
                                    name: o.name,
                                    description: o.description,
                                    image: o.image,
                                    kind: option_kind(o.kind),
                                    selected: o.selected,
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
            complete: v.complete,
            needs_unmet: v.needs_unmet,
            files: v
                .files
                .into_iter()
                .map(|(source, destination)| FileDto {
                    source,
                    destination,
                })
                .collect(),
            missing: v.missing,
            choices: v
                .choices
                .into_iter()
                .map(|c| ChoiceDto {
                    step: c.step,
                    group: c.group,
                    plugins: c.plugins,
                })
                .collect(),
            target: v.target,
            suggested_target: v.suggested_target,
            game_folders: v.game_folders,
        }
    }
}

/// The mod's installer with `choices` (default: the saved ones, else its defaults).
#[tauri::command]
pub async fn fomod_view(
    state: State<'_, AppState>,
    game: String,
    id: String,
    choices: Option<Vec<ChoiceDto>>,
    target: Option<String>,
) -> Res<FomodViewDto> {
    blocking(&state, move |s| {
        let choices = choices.map(|c| c.into_iter().map(Into::into).collect());
        Ok(s.fomod_view(&game, &id, choices, target)?.into())
    })
    .await
}

/// Saves the installer's options and the game folder its files go under.
#[tauri::command]
pub async fn fomod_apply(
    state: State<'_, AppState>,
    game: String,
    id: String,
    choices: Vec<ChoiceDto>,
    target: String,
) -> Res<ModDto> {
    blocking(&state, move |s| {
        let choices = choices.into_iter().map(Into::into).collect();
        Ok(s.fomod_apply(&game, &id, choices, target)?.into())
    })
    .await
}

/// An image inside a mod, as a `data:` URL.
#[tauri::command]
pub async fn mod_image(
    state: State<'_, AppState>,
    game: String,
    id: String,
    path: String,
) -> Res<String> {
    blocking(&state, move |s| s.mod_image(&game, &id, &path)).await
}
