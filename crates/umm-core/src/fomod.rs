//! FOMOD installers in stored mods: finding one, running it for the options dialog, and
//! keeping the choices and the files they install (see `umm_store::FomodMeta`).
//!
//! A mod with an installer deploys nothing until its options are chosen. Its files then go
//! where the installer says, under the game folder its placement names (UMM suggests one from
//! what the game folder holds, as for any mod). On an update, the earlier choices are replayed
//! when the new installer still offers them all; otherwise the user is asked again.

use crate::placement::{suggest, GameIndex};
use crate::{CoreError, ModMeta, Result, Session};
use std::fs;
use umm_deploy::GameLock;
use umm_fomod::{Choice, Evaluated, FileState, Installer, PluginType};
use umm_gamedef::{CompiledDef, RelPath};
use umm_store::fsutil::IoCtx;
use umm_store::{FomodChoice, FomodFile, FomodMeta, Placement};

/// The installer, as the options dialog shows it with a set of choices.
#[derive(Debug, Clone)]
pub struct FomodView {
    pub name: String,
    /// Paths inside the mod.
    pub image: Option<String>,
    pub steps: Vec<FomodStepView>,
    /// Every shown group has a valid choice: the options can be saved.
    pub complete: bool,
    /// The mod says it needs something the game does not have.
    pub needs_unmet: bool,
    /// The files these choices install, as `(path inside the mod, path in the game folder)`.
    pub files: Vec<(String, String)>,
    /// Files the installer names that the archive lacks.
    pub missing: Vec<String>,
    /// The choices in effect, to send back.
    pub choices: Vec<Choice>,
    /// The game folder the files go under, and UMM's suggestion for it.
    pub target: String,
    pub suggested_target: String,
    pub game_folders: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FomodStepView {
    pub name: String,
    pub visible: bool,
    pub groups: Vec<FomodGroupView>,
}

#[derive(Debug, Clone)]
pub struct FomodGroupView {
    pub name: String,
    pub kind: umm_fomod::GroupKind,
    pub problem: Option<String>,
    pub options: Vec<FomodOptionView>,
}

#[derive(Debug, Clone)]
pub struct FomodOptionView {
    pub name: String,
    pub description: String,
    /// Path inside the mod.
    pub image: Option<String>,
    pub kind: PluginType,
    pub selected: bool,
}

/// The folder holding `fomod/ModuleConfig.xml` in a mod's files, the shallowest if several.
fn installer_root(meta: &ModMeta) -> Option<(String, String)> {
    meta.files
        .keys()
        .filter_map(|f| {
            let lower = f.to_lowercase();
            let cut = if lower == "fomod/moduleconfig.xml" {
                0
            } else {
                lower.strip_suffix("/fomod/moduleconfig.xml")?.len()
            };
            Some((f[..cut].to_string(), f.clone()))
        })
        .min_by_key(|(root, _)| root.matches('/').count() + usize::from(!root.is_empty()))
}

fn under<'a>(root: &str, file: &'a str) -> Option<&'a str> {
    if root.is_empty() {
        return Some(file);
    }
    let n = root.len();
    (file.len() > n + 1 && file[..n].eq_ignore_ascii_case(root) && file.as_bytes()[n] == b'/')
        .then(|| &file[n + 1..])
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a}/{b}"),
    }
}

fn to_choice(c: &FomodChoice) -> Choice {
    Choice {
        step: c.step.clone(),
        group: c.group.clone(),
        plugins: c.plugins.clone(),
    }
}

fn from_choice(c: &Choice) -> FomodChoice {
    FomodChoice {
        step: c.step.clone(),
        group: c.group.clone(),
        plugins: c.plugins.clone(),
    }
}

impl Session {
    /// The mod's installer, if it has one: its folder in the mod and the parsed file.
    fn installer(&self, game_id: &str, meta: &ModMeta) -> Result<Option<(String, Installer)>> {
        let Some((root, file)) = installer_root(meta) else {
            return Ok(None);
        };
        let p = RelPath::parse(&file)?.to_path(&self.data.game(game_id).mod_files_dir(&meta.id));
        let bytes = fs::read(&p).ctx(|| format!("reading {}", p.display()))?;
        let inst = Installer::parse(&bytes)
            .map_err(|e| CoreError::Invalid(format!("{}: {e}", meta.name)))?;
        Ok(Some((root, inst)))
    }

    /// How the game folder has a file a condition names, below the mod's game folder.
    fn file_states(&self, def: &CompiledDef, target: &str) -> impl Fn(&str) -> FileState {
        let base = self
            .install_path(&def.def)
            .ok()
            .flatten()
            .map(|(p, _)| p)
            .zip(RelPath::parse(target).ok())
            .map(|(root, t)| t.to_path(&root));
        move |f: &str| match (&base, RelPath::parse(f)) {
            (Some(b), Ok(rel)) if rel.to_path(b).exists() => FileState::Active,
            _ => FileState::Missing,
        }
    }

    /// Runs the installer and works out its files.
    fn run_installer(
        &self,
        def: &CompiledDef,
        meta: &ModMeta,
        root: &str,
        inst: &Installer,
        choices: &[Choice],
        target: &str,
    ) -> (Evaluated, Vec<(String, String)>, Vec<String>) {
        let e = inst.evaluate(choices, &self.file_states(def, target));
        let archive: Vec<&str> = meta.files.keys().filter_map(|f| under(root, f)).collect();
        let (placed, missing) = umm_fomod::resolve(&e.files, &archive);
        let files = placed
            .into_iter()
            .map(|p| (join(root, &p.source), p.destination))
            .collect();
        (e, files, missing)
    }

    /// The game folder UMM suggests for files going to these destinations.
    fn fomod_target(
        &self,
        def: &CompiledDef,
        files: &[(String, String)],
        ix: &GameIndex,
    ) -> String {
        let dests: Vec<&str> = files.iter().map(|(_, d)| d.as_str()).collect();
        suggest(&dests, ix, def)
            .filter(|p| p.root.is_empty())
            .map(|p| p.target)
            .unwrap_or_default()
    }

    /// Gives a newly imported mod with an installer its record: the earlier version's
    /// choices when they still fit, else nothing chosen yet. Mods without one are unchanged.
    pub(crate) fn fomod_imported(
        &self,
        def: &CompiledDef,
        meta: ModMeta,
        previous: Option<(Option<FomodMeta>, Option<Placement>)>,
    ) -> Result<ModMeta> {
        let game_id = def.def.id.clone();
        let game = self.data.game(&game_id);
        let Some((root, inst)) = self.installer(&game_id, &meta).unwrap_or(None) else {
            return Ok(meta);
        };
        let (old, old_place) = previous.unwrap_or((None, None));
        let old_target = old_place.map(|p| p.target).unwrap_or_default();
        if let Some(old) = old.filter(|o| o.done) {
            let choices: Vec<Choice> = old.choices.iter().map(to_choice).collect();
            let (e, files, missing) =
                self.run_installer(def, &meta, &root, &inst, &choices, &old_target);
            if e.complete() && !e.defaulted() && missing.is_empty() && !files.is_empty() {
                let rec = FomodMeta {
                    root,
                    done: true,
                    choices: e.choices.iter().map(from_choice).collect(),
                    files: files
                        .into_iter()
                        .map(|(source, destination)| FomodFile {
                            source,
                            destination,
                        })
                        .collect(),
                };
                umm_store::set_placement(
                    &game,
                    &meta.id,
                    Some(Placement {
                        target: old_target,
                        chosen: true,
                        ..Default::default()
                    }),
                )?;
                return Ok(umm_store::set_fomod(&game, &meta.id, Some(rec))?);
            }
        }
        umm_store::set_placement(&game, &meta.id, None)?;
        Ok(umm_store::set_fomod(
            &game,
            &meta.id,
            Some(FomodMeta {
                root,
                ..Default::default()
            }),
        )?)
    }

    /// The mod's installer run with `choices` (default: the ones saved, else the installer's
    /// defaults), and its files under `target` (default: the saved game folder, else UMM's
    /// suggestion).
    pub fn fomod_view(
        &self,
        game_id: &str,
        mod_id: &str,
        choices: Option<Vec<Choice>>,
        target: Option<String>,
    ) -> Result<FomodView> {
        let def = self.def(game_id)?;
        let meta = umm_store::load_mod(&self.data.game(game_id), mod_id)?;
        let (root, inst) = self
            .installer(game_id, &meta)?
            .ok_or_else(|| CoreError::Invalid(format!("{} has no FOMOD installer", meta.name)))?;
        let saved = meta.fomod.as_ref().filter(|f| f.done);
        let choices = choices.unwrap_or_else(|| {
            saved
                .map(|f| f.choices.iter().map(to_choice).collect())
                .unwrap_or_default()
        });
        let ix = self.game_index(&def);
        // Conditions on game files need a game folder: the saved or given one, else none yet.
        let first_target = target
            .clone()
            .or_else(|| saved.and(meta.placement.as_ref()).map(|p| p.target.clone()))
            .unwrap_or_default();
        let (e, files, missing) =
            self.run_installer(&def, &meta, &root, &inst, &choices, &first_target);
        let suggested_target = self.fomod_target(&def, &files, &ix);
        let target = target
            .or_else(|| saved.and(meta.placement.as_ref()).map(|p| p.target.clone()))
            .unwrap_or_else(|| suggested_target.clone());
        let in_mod = |p: &Option<String>| p.as_ref().map(|p| join(&root, p));
        Ok(FomodView {
            name: if inst.name.is_empty() {
                meta.name.clone()
            } else {
                inst.name.clone()
            },
            image: in_mod(&inst.image),
            steps: inst
                .steps
                .iter()
                .zip(&e.steps)
                .map(|(s, st)| FomodStepView {
                    name: s.name.clone(),
                    visible: st.visible,
                    groups: s
                        .groups
                        .iter()
                        .zip(&st.groups)
                        .map(|(g, gs)| FomodGroupView {
                            name: g.name.clone(),
                            kind: g.kind,
                            problem: gs.problem.clone(),
                            options: g
                                .plugins
                                .iter()
                                .zip(&gs.plugins)
                                .map(|(p, ps)| FomodOptionView {
                                    name: p.name.clone(),
                                    description: p.description.clone(),
                                    image: in_mod(&p.image),
                                    kind: ps.kind,
                                    selected: ps.selected,
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
            complete: e.complete(),
            needs_unmet: e.needs_unmet,
            files,
            missing,
            choices: e.choices,
            target,
            suggested_target,
            game_folders: ix.folders(3),
        })
    }

    /// Saves the installer's options and the game folder its files go under. Refused while
    /// mods are deployed, or when a shown group has no valid choice.
    pub fn fomod_apply(
        &self,
        game_id: &str,
        mod_id: &str,
        choices: Vec<Choice>,
        target: String,
    ) -> Result<ModMeta> {
        let def = self.def(game_id)?;
        let game = self.data.game(game_id);
        let _lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(game_id.into()));
        }
        let meta = umm_store::load_mod(&game, mod_id)?;
        let (root, inst) = self
            .installer(game_id, &meta)?
            .ok_or_else(|| CoreError::Invalid(format!("{} has no FOMOD installer", meta.name)))?;
        let (e, files, _) = self.run_installer(&def, &meta, &root, &inst, &choices, &target);
        if !e.complete() {
            return Err(CoreError::Invalid(
                "every shown group needs its options chosen first".into(),
            ));
        }
        umm_store::set_placement(
            &game,
            mod_id,
            Some(Placement {
                target,
                chosen: true,
                ..Default::default()
            }),
        )?;
        Ok(umm_store::set_fomod(
            &game,
            mod_id,
            Some(FomodMeta {
                root,
                done: true,
                choices: e.choices.iter().map(from_choice).collect(),
                files: files
                    .into_iter()
                    .map(|(source, destination)| FomodFile {
                        source,
                        destination,
                    })
                    .collect(),
            }),
        )?)
    }

    /// An image in a mod (an installer option's picture) as a `data:` URL. Only common image
    /// types, at most 8 MB.
    pub fn mod_image(&self, game_id: &str, mod_id: &str, path: &str) -> Result<String> {
        use base64::Engine;
        let meta = umm_store::load_mod(&self.data.game(game_id), mod_id)?;
        let key = meta
            .files
            .keys()
            .find(|f| f.eq_ignore_ascii_case(path))
            .ok_or_else(|| CoreError::Invalid(format!("{} has no file {path}", meta.name)))?;
        let ext = key
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        let mime = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            _ => {
                return Err(CoreError::Invalid(format!(
                    "{path} is not an image UMM shows"
                )))
            }
        };
        if meta.files[key].size > 8 << 20 {
            return Err(CoreError::Invalid(format!("{path} is too large to show")));
        }
        let p = RelPath::parse(key)?.to_path(&self.data.game(game_id).mod_files_dir(mod_id));
        let bytes = fs::read(&p).ctx(|| format!("reading {}", p.display()))?;
        Ok(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_shallowest_installer() {
        let mut meta: ModMeta =
            serde_json::from_str(r#"{"id": "m", "name": "M", "imported_unix": 0, "files": {}}"#)
                .unwrap();
        let rec = umm_store::FileRecord {
            size: 1,
            hash: "h".into(),
        };
        for f in [
            "Wrap/FOMOD/ModuleConfig.xml",
            "Wrap/Deep/fomod/ModuleConfig.xml",
            "Wrap/a.esp",
        ] {
            meta.files.insert(f.into(), rec.clone());
        }
        assert_eq!(installer_root(&meta).unwrap().0, "Wrap");
        assert_eq!(under("Wrap", "wrap/a.esp"), Some("a.esp"));
        assert_eq!(under("Wrap", "Wrapper/a.esp"), None);
    }
}
