use crate::DeployError;
use std::collections::BTreeMap;
use std::path::PathBuf;
use umm_gamedef::{CompiledDef, RelPath, IN_GAME_BACKUP_DIR};
use umm_store::{load_mod, GameDir, Layout, Profile, StoreError};

/// Which stored file goes to which game path. Pure: reads the store, never the game folder.
#[derive(Debug, Clone)]
pub struct Plan {
    pub game: String,
    pub profile: String,
    /// Sorted by target path.
    pub files: Vec<PlannedFile>,
    pub conflicts: Vec<Conflict>,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone)]
pub struct PlannedFile {
    pub target: RelPath,
    pub source: PathBuf,
    pub owner: String,
    pub hash: String,
    pub size: u64,
    /// Always copied, never hardlinked, because the game writes to it.
    pub writable: bool,
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub target: RelPath,
    pub winner: String,
    /// Lower-priority mods whose copy of this file is not used, lowest first.
    pub overridden: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Skipped {
    pub owner: String,
    pub file: String,
    pub reason: String,
}

struct Entry {
    file: PlannedFile,
    overridden: Vec<String>,
}

/// Priority, lowest first: the loader, then the profile's enabled mods in order.
/// For each target path, the highest-priority source wins.
pub fn build_plan(
    game: &GameDir,
    def: &CompiledDef,
    profile: &Profile,
) -> Result<Plan, DeployError> {
    let loader = def
        .def
        .loader
        .as_ref()
        .and_then(|l| l.source_mod())
        .map(str::to_string);
    let mut sources: Vec<(String, bool)> = Vec::new();
    if let Some(l) = &loader {
        // The loader pack is laid out like the game folder, so it skips the install rules.
        sources.push((l.clone(), true));
    }
    for id in profile.enabled_ids() {
        if Some(id) != loader.as_deref() {
            sources.push((id.to_string(), false));
        }
    }

    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut skipped = Vec::new();
    for (id, identity) in &sources {
        let meta = match load_mod(game, id) {
            Ok(m) => m,
            Err(StoreError::ModNotFound(_)) => return Err(DeployError::MissingMod(id.clone())),
            Err(e) => return Err(e.into()),
        };
        let files_root = game.mod_files_dir(id);
        if meta.thunderstore.is_some()
            && !def
                .def
                .loader
                .as_ref()
                .is_some_and(|l| l.from_thunderstore())
        {
            // Laid out for BepInEx from Thunderstore; meaningless in any other setup.
            skipped.push(Skipped {
                owner: id.clone(),
                file: "(all files)".into(),
                reason: "installed from Thunderstore; needs the BepInEx from Thunderstore setup"
                    .into(),
            });
            continue;
        }
        let identity = *identity || meta.layout == Layout::Game;
        // Each stored file to deploy, and its path before the install rules: from the mod's
        // FOMOD choices, its placement, or as stored.
        let mut items: Vec<(&String, RelPath)> = Vec::new();
        match &meta.fomod {
            Some(f) if !identity && !f.done => {
                skipped.push(Skipped {
                    owner: id.clone(),
                    file: "(all files)".into(),
                    reason: "its installer's options have not been chosen yet".into(),
                });
                continue;
            }
            Some(f) if !identity => {
                let base = meta
                    .placement
                    .as_ref()
                    .map(|p| RelPath::parse(&p.target))
                    .transpose()?
                    .unwrap_or_else(|| RelPath::parse("").expect("empty path"));
                for m in &f.files {
                    let Some((file, _)) = meta.files.get_key_value(&m.source) else {
                        return Err(DeployError::MissingMod(format!("{id} ({})", m.source)));
                    };
                    items.push((file, base.join(&RelPath::parse(&m.destination)?)));
                }
            }
            _ => {
                for file in meta.files.keys() {
                    let rel = RelPath::parse(file)?;
                    let placed = match meta.placement.as_ref().filter(|_| !identity) {
                        Some(p) => match p.place(&rel) {
                            Ok(placed) => placed,
                            Err(why) => {
                                skipped.push(Skipped {
                                    owner: id.clone(),
                                    file: file.clone(),
                                    reason: why.reason().to_string(),
                                });
                                continue;
                            }
                        },
                        None => rel,
                    };
                    items.push((file, placed));
                }
            }
        }
        for (file, placed) in items {
            let rec = &meta.files[file];
            let rel = RelPath::parse(file)?;
            let skip = |reason: &str| Skipped {
                owner: id.clone(),
                file: file.clone(),
                reason: reason.to_string(),
            };
            let target = if identity {
                Some(placed)
            } else {
                def.map_mod_file(id, &placed)?
            };
            let Some(target) = target else {
                skipped.push(skip("no install rule matches"));
                continue;
            };
            if target.is_root() {
                skipped.push(skip("maps to the game folder itself"));
                continue;
            }
            if target
                .first()
                .is_some_and(|c| c.eq_ignore_ascii_case(IN_GAME_BACKUP_DIR))
            {
                skipped.push(skip("reserved for UMM backups"));
                continue;
            }
            if def.is_protected(&target) {
                skipped.push(skip(&format!(
                    "{target} is protected by the game definition"
                )));
                continue;
            }
            let planned = PlannedFile {
                writable: def.is_writable(&target),
                source: rel.to_path(&files_root),
                owner: id.clone(),
                hash: rec.hash.clone(),
                size: rec.size,
                target,
            };
            match entries.get_mut(&planned.target.key()) {
                Some(e) => {
                    let loser = std::mem::replace(&mut e.file, planned);
                    e.overridden.push(loser.owner);
                }
                None => {
                    entries.insert(
                        planned.target.key(),
                        Entry {
                            file: planned,
                            overridden: vec![],
                        },
                    );
                }
            }
        }
    }

    // A file and a folder with the same path cannot both be deployed.
    let keys: Vec<String> = entries.keys().cloned().collect();
    for k in &keys {
        let prefix = format!("{k}/");
        if keys.iter().any(|other| other.starts_with(&prefix)) {
            let e = &entries[k];
            return Err(DeployError::TargetIsFile(e.file.target.to_string()));
        }
    }

    let mut files = Vec::new();
    let mut conflicts = Vec::new();
    for (_, e) in entries {
        if !e.overridden.is_empty() {
            conflicts.push(Conflict {
                target: e.file.target.clone(),
                winner: e.file.owner.clone(),
                overridden: e.overridden,
            });
        }
        files.push(e.file);
    }
    Ok(Plan {
        game: game.id.clone(),
        profile: profile.name.clone(),
        files,
        conflicts,
        skipped,
    })
}
