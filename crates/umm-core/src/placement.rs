//! Suggesting where a mod's files go, for archives not laid out like the game folder (most
//! Nexus and hand-made archives). UMM tries each folder of the mod as its root and each game
//! folder as its target, and keeps the pair whose files line up best with what the game
//! already has. Readmes, checksums and the like are left out.

use crate::{CoreError, ModMeta, Result, Session};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use umm_deploy::GameLock;
use umm_gamedef::setup::bethesda_data_folder;
use umm_gamedef::{
    CompiledDef, RelPath, DATA_ARCHIVE_EXTENSIONS, DATA_DIRS, EXTENDER_DIRS, IN_GAME_BACKUP_DIR,
    PLUGIN_EXTENSIONS,
};
use umm_store::{LeftOut, Placement};
use walkdir::WalkDir;

/// How deep into the game folder UMM looks, and how many entries at most.
const SCAN_DEPTH: usize = 5;
const SCAN_LIMIT: usize = 200_000;
/// How deep inside a mod a root folder may be.
const ROOT_DEPTH: usize = 4;

/// The game folder's files and folders, lower-cased, relative to it.
#[derive(Debug, Default)]
pub struct GameIndex {
    files: HashSet<String>,
    dirs: HashSet<String>,
    /// Entry name → the folders holding an entry of that name ("" for the game folder).
    parents: HashMap<String, Vec<String>>,
    /// `(folder, extension)` pairs: the kinds of files each folder holds.
    exts: HashSet<(String, String)>,
    /// Lower-cased folder → as spelled on disk.
    spelled: HashMap<String, String>,
}

impl GameIndex {
    pub fn scan(root: &Path) -> Self {
        let mut ix = GameIndex::default();
        let walk = WalkDir::new(root)
            .min_depth(1)
            .max_depth(SCAN_DEPTH)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !e.file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(IN_GAME_BACKUP_DIR)
            });
        for e in walk.filter_map(|e| e.ok()).take(SCAN_LIMIT) {
            let Ok(rel) = e.path().strip_prefix(root) else {
                continue;
            };
            let Ok(rel) = RelPath::from_path(rel) else {
                continue;
            };
            ix.add(&rel, e.file_type().is_dir());
        }
        ix
    }

    fn add(&mut self, rel: &RelPath, dir: bool) {
        let path = rel.as_str().to_lowercase();
        let parent = rel
            .parent()
            .map(|p| p.as_str().to_lowercase())
            .unwrap_or_default();
        let name = rel.file_name().unwrap_or_default().to_lowercase();
        if dir {
            self.spelled.insert(path.clone(), rel.as_str().to_string());
            self.dirs.insert(path);
        } else {
            if let Some(ext) = ext(&name) {
                self.exts.insert((parent.clone(), ext.to_string()));
            }
            self.files.insert(path);
        }
        self.parents.entry(name).or_default().push(parent);
    }

    /// The game's folders, shallowest first, to offer as targets.
    pub fn folders(&self, max_depth: usize) -> Vec<String> {
        let mut v: Vec<&String> = self
            .dirs
            .iter()
            .filter(|d| d.split('/').count() <= max_depth)
            .collect();
        v.sort_by(|a, b| (a.split('/').count(), *a).cmp(&(b.split('/').count(), *b)));
        v.into_iter().map(|d| self.spelling(d)).collect()
    }

    /// A lower-cased game folder as spelled on disk.
    fn spelling(&self, dir: &str) -> String {
        self.spelled
            .get(dir)
            .cloned()
            .unwrap_or_else(|| dir.to_string())
    }

    #[cfg(test)]
    pub fn of(entries: &[&str]) -> Self {
        let mut ix = GameIndex::default();
        for e in entries {
            let dir = e.ends_with('/');
            ix.add(&RelPath::parse(e.trim_end_matches('/')).unwrap(), dir);
        }
        ix
    }
}

fn ext(name: &str) -> Option<&str> {
    name.rsplit_once('.')
        .map(|(_, e)| e)
        .filter(|e| !e.is_empty())
}

/// The game's data folder (lower-cased), where plugins and their files go, for games with a
/// Bethesda plugin list.
fn data_folder(def: &CompiledDef) -> Option<String> {
    bethesda_data_folder(&def.def).map(str::to_lowercase)
}

/// Whether `c` (relative to the mod's root) is something a data folder holds: a plugin or
/// archive at its top, or a file in one of its usual folders.
fn data_content(c: &RelPath) -> bool {
    let first = c.first().unwrap_or_default().to_lowercase();
    if c.depth() == 1 {
        return ext(&first).is_some_and(|e| {
            PLUGIN_EXTENSIONS.contains(&e) || DATA_ARCHIVE_EXTENSIONS.contains(&e)
        });
    }
    let second = c.components().nth(1).unwrap_or_default().to_lowercase();
    DATA_DIRS.contains(&first.as_str())
        || (c.depth() > 2 && EXTENDER_DIRS.contains(&first.as_str()) && second == "plugins")
}

fn join(t: &str, rel: &str) -> String {
    if t.is_empty() {
        rel.to_string()
    } else {
        format!("{t}/{rel}")
    }
}

/// Whether a file (relative to the mod's root folder) is documentation or clutter rather
/// than something the game uses.
pub fn is_doc(rel: &RelPath) -> bool {
    let name = rel.file_name().unwrap_or_default().to_lowercase();
    let e = ext(&name).unwrap_or("");
    // Anywhere: links, checksums, release notes, build projects and OS clutter.
    if matches!(
        e,
        "url"
            | "nfo"
            | "sfv"
            | "md5"
            | "sha1"
            | "sha256"
            | "sha512"
            | "sln"
            | "vcproj"
            | "vcxproj"
            | "filters"
    ) || matches!(name.as_str(), "thumbs.db" | "desktop.ini" | ".ds_store")
    {
        return true;
    }
    let depth = rel.depth();
    if depth > 1 {
        let top = rel.first().unwrap_or_default().to_lowercase();
        return matches!(
            top.as_str(),
            "fomod"
                | "screenshots"
                | "screenshot"
                | "docs"
                | "documentation"
                | "__macosx"
                // Source code some tools ship beside their build (a script extender's `src/`).
                | "src"
                | "source"
                | "sources"
        );
    }
    // At the root: documents, and readme-like text files.
    if matches!(
        e,
        "md" | "pdf" | "rtf" | "doc" | "docx" | "odt" | "htm" | "html"
    ) {
        return true;
    }
    let stem = name.split('.').next().unwrap_or_default();
    let stem: String = stem.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let texty = matches!(e, "" | "txt");
    // Named anywhere in the name ("StreetLights Wasteland Readme.txt", "fose_whatsnew.txt").
    if texty
        && ["readme", "changelog", "whatsnew", "versionhistory"]
            .iter()
            .any(|w| stem.contains(w))
    {
        return true;
    }
    texty
        && [
            "readme",
            "changelog",
            "changes",
            "license",
            "licence",
            "credits",
            "install",
            "installation",
            "instructions",
            "sha256sums",
            "md5sums",
            "checksums",
            "versionhistory",
            "copying",
            "authors",
        ]
        .iter()
        .any(|w| stem.starts_with(w))
}

struct Candidate {
    root: String,
    target: String,
    score: usize,
    depth: usize,
}

/// Suggests a placement for a mod with these files (paths inside the mod). `None` means the
/// mod needs none: its files are already laid out like the game folder.
pub fn suggest(files: &[&str], game: &GameIndex, def: &CompiledDef) -> Option<Placement> {
    let rels: Vec<RelPath> = files
        .iter()
        .filter_map(|f| RelPath::parse(f).ok())
        .collect();
    // Games whose mods each have their own folder: its root found by what marks it.
    let format = def.def.plugins.as_ref().map(|p| p.format);
    let own_folder = match format {
        Some(umm_gamedef::PluginListFormat::RimworldXml) => Some(rimworld_root(&rels)),
        Some(umm_gamedef::PluginListFormat::FactorioJson) => Some(factorio_root(&rels)),
        _ if def.def.paradox.is_some() => Some(paradox_root(&rels, def)),
        _ => None,
    };
    if let Some(p) = own_folder {
        let mut p = p.unwrap_or_default();
        p.skip = doc_skips(&rels, &p.root, &p.target, game);
        if format == Some(umm_gamedef::PluginListFormat::FactorioJson) {
            // Factorio shows a mod's changelog in its mod manager.
            let keep = join(&p.root, "changelog.txt");
            p.skip.retain(|s| !s.eq_ignore_ascii_case(&keep));
        }
        return (p != Placement::default()).then_some(p);
    }
    let mut roots: Vec<String> = vec![String::new()];
    let mut seen: HashSet<String> = HashSet::new();
    for r in &rels {
        let comps: Vec<&str> = r.components().collect();
        for n in 1..comps.len().min(ROOT_DEPTH + 1) {
            let root = comps[..n].join("/");
            if seen.insert(root.to_lowercase()) {
                roots.push(root);
            }
        }
    }

    // Install-rule folders (`plugins/**`) take any path, so they say nothing about which
    // folder is the root: try them only below the wrapper folders.
    let wrapper = wrapper_chain(&rels, def);
    let mut best: Option<Candidate> = None;
    // Another root folder fits just as well (a main and an optional version side by side).
    let mut tied = false;
    for root in &roots {
        let content: Vec<RelPath> = rels
            .iter()
            .filter_map(|r| under(r, root))
            .filter(|c| !is_doc(c))
            .collect();
        if content.is_empty() {
            continue;
        }
        for target in targets(&content, game, def) {
            if !target.is_empty()
                && !game.dirs.contains(&target.to_lowercase())
                && !root.eq_ignore_ascii_case(&wrapper)
            {
                continue;
            }
            let score: usize = content
                .iter()
                .map(|c| evidence(c, &target, game, def))
                .sum();
            let depth = root.split('/').filter(|s| !s.is_empty()).count()
                + target.split('/').filter(|s| !s.is_empty()).count();
            let better = best
                .as_ref()
                .is_none_or(|b| score > b.score || (score == b.score && depth < b.depth));
            if score > 0
                && best.as_ref().is_some_and(|b| {
                    score == b.score && depth == b.depth && !b.root.eq_ignore_ascii_case(root)
                })
            {
                tied = true;
            }
            if score > 0 && better {
                tied = false;
                best = Some(Candidate {
                    root: root.clone(),
                    target,
                    score,
                    depth,
                });
            }
        }
    }

    let (root, target, guessed) = match best {
        Some(c) => (c.root, game.spelling(&c.target), tied),
        None => {
            // Nothing to go by: drop wrapper folders, and say it is a guess unless the
            // install rules know where the files go.
            let root = wrapper_chain(&rels, def);
            let specific = rels
                .iter()
                .filter_map(|r| under(r, &root))
                .any(|c| def.has_specific_rule(&c));
            (root, String::new(), !specific)
        }
    };
    let skip = doc_skips(&rels, &root, &target, game);
    let p = Placement {
        root,
        target,
        skip,
        guessed,
        chosen: false,
    };
    (p != Placement::default()).then_some(p)
}

/// The mod's documents under `root` (readmes, checksums...) to leave out, unless the game
/// already has a file where one would go.
fn doc_skips(rels: &[RelPath], root: &str, target: &str, game: &GameIndex) -> Vec<String> {
    rels.iter()
        .filter(|r| {
            under(r, root).is_some_and(|c| {
                is_doc(&c)
                    && !game
                        .files
                        .contains(&join(target, c.as_str()).to_lowercase())
            })
        })
        .map(|r| r.as_str().to_string())
        .collect()
}

/// A RimWorld mod's folder is the one holding `About/About.xml`: whatever wraps it is
/// dropped (the install rules give each mod its own folder in `Mods/`). Several at the same
/// depth (a pack of mods) is a guess.
fn rimworld_root(rels: &[RelPath]) -> Option<Placement> {
    let mut roots: Vec<(usize, String)> = rels
        .iter()
        .filter(|r| {
            let c: Vec<&str> = r.components().collect();
            c.len() >= 2
                && c[c.len() - 1].eq_ignore_ascii_case("About.xml")
                && c[c.len() - 2].eq_ignore_ascii_case("About")
        })
        .map(|r| {
            let root = r
                .parent()
                .and_then(|p| p.parent())
                .unwrap_or_else(RelPath::root);
            (root.depth(), root.as_str().to_string())
        })
        .collect();
    roots.sort();
    let (depth, root) = roots.first()?.clone();
    let guessed = roots.get(1).is_some_and(|(d, _)| *d == depth);
    let p = Placement {
        root,
        guessed,
        ..Default::default()
    };
    (p != Placement::default()).then_some(p)
}

/// A Factorio mod's folder is the one holding its `info.json` (the shallowest).
fn factorio_root(rels: &[RelPath]) -> Option<Placement> {
    let mut roots: Vec<(usize, String)> = rels
        .iter()
        .filter(|r| {
            r.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(crate::factorio::INFO))
        })
        .map(|r| {
            let root = r.parent().unwrap_or_else(RelPath::root);
            (root.depth(), root.as_str().to_string())
        })
        .collect();
    roots.sort();
    let (depth, root) = roots.first()?.clone();
    let guessed = roots.get(1).is_some_and(|(d, _)| *d == depth);
    let p = Placement {
        root,
        guessed,
        ..Default::default()
    };
    (p != Placement::default()).then_some(p)
}

/// A Paradox mod's folder is the one holding its `descriptor.mod` (an archive often has the
/// launcher's `<mod>.mod` beside it, left out); without one, the folder beside the launcher's
/// `<mod>.mod` (Crusader Kings II's mods), else wrapper folders are dropped.
/// The install rules give it a folder of its own in the user folder's `mod/`.
fn paradox_root(rels: &[RelPath], def: &CompiledDef) -> Option<Placement> {
    let root = rels
        .iter()
        .filter(|r| {
            r.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(crate::paradox::DESCRIPTOR))
        })
        .map(|r| r.parent().unwrap_or_else(RelPath::root))
        .min_by_key(|r| r.depth())
        .map(|r| r.as_str().to_string())
        .or_else(|| launcher_folder(rels))
        .unwrap_or_else(|| wrapper_chain(rels, def));
    let p = Placement {
        root,
        ..Default::default()
    };
    (p != Placement::default()).then_some(p)
}

/// The folder beside a Paradox mod's launcher `<mod>.mod` (the shallowest): the one named like
/// it, or the only one there.
fn launcher_folder(rels: &[RelPath]) -> Option<String> {
    let m = rels
        .iter()
        .filter(|r| {
            r.file_name()
                .is_some_and(|n| n.to_ascii_lowercase().ends_with(".mod"))
        })
        .min_by_key(|r| r.depth())?;
    let name = m.file_name()?;
    let stem = &name[..name.len() - 4];
    let parent = m.parent().unwrap_or_else(RelPath::root);
    let mut dirs: Vec<String> = rels
        .iter()
        .filter_map(|r| under(r, parent.as_str()))
        .filter(|c| c.depth() > 1)
        .map(|c| c.first().unwrap_or_default().to_string())
        .collect();
    dirs.sort();
    dirs.dedup();
    let pick = dirs
        .iter()
        .find(|d| d.eq_ignore_ascii_case(stem))
        .or_else(|| dirs.first().filter(|_| dirs.len() == 1))?;
    Some(join(parent.as_str(), pick))
}

/// `rel` relative to `root`, if it is inside it.
fn under(rel: &RelPath, root: &str) -> Option<RelPath> {
    let n = root.split('/').filter(|s| !s.is_empty()).count();
    if rel.depth() <= n {
        return None;
    }
    let inside = rel
        .components()
        .zip(root.split('/').filter(|s| !s.is_empty()))
        .all(|(a, b)| a.eq_ignore_ascii_case(b));
    inside.then(|| rel.strip_components(n))
}

/// Game folders worth trying as a target: the game folder itself, and every folder that
/// already holds an entry named like one of the content's top-level entries.
fn targets(content: &[RelPath], game: &GameIndex, def: &CompiledDef) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut seen: HashSet<String> = HashSet::from([String::new()]);
    let mut add = |p: &String| {
        if seen.insert(p.to_lowercase()) {
            out.push(p.clone());
        }
    };
    // The folders where the install rules give each mod its own (`plugins/**` →
    // BepInEx/plugins/<mod>/).
    def.per_mod_top_dirs().iter().for_each(&mut add);
    if let Some(d) = data_folder(def) {
        add(&game.spelling(&d));
    }
    let mut loose: HashSet<String> = HashSet::new();
    for c in content {
        let first = c.first().unwrap_or_default().to_lowercase();
        game.parents
            .get(&first)
            .into_iter()
            .flatten()
            .for_each(&mut add);
        if c.depth() == 1 {
            loose.extend(ext(&first).map(str::to_string));
        }
    }
    // Loose files: the folders holding files of their kind, when only a few do.
    for e in loose {
        let dirs: Vec<&String> = game
            .exts
            .iter()
            .filter(|(_, x)| *x == e)
            .map(|(d, _)| d)
            .collect();
        if dirs.len() <= 3 {
            dirs.into_iter().for_each(&mut add);
        }
    }
    out
}

/// How strongly the game folder agrees with `c` going to `target/c`.
fn evidence(c: &RelPath, target: &str, game: &GameIndex, def: &CompiledDef) -> usize {
    let path = join(target, &c.as_str().to_lowercase());
    if game.files.contains(&path) {
        return 3; // replaces a game file
    }
    if data_folder(def).is_some_and(|d| d == target.to_lowercase()) && data_content(c) {
        return 2; // a plugin, an archive or a loose asset, in the data folder
    }
    let parent = c
        .parent()
        .filter(|p| !p.is_root())
        .map(|p| join(target, &p.as_str().to_lowercase()));
    if let Some(p) = &parent {
        if game.dirs.contains(p) {
            return 2; // goes into a folder the game has
        }
    }
    if RelPath::parse(&join(target, c.as_str())).is_ok_and(|p| def.has_specific_rule(&p)) {
        return 2; // the game's install rules know where this goes
    }
    let dir = parent.unwrap_or_else(|| target.to_lowercase());
    let name = c.file_name().unwrap_or_default().to_lowercase();
    if ext(&name).is_some_and(|e| game.exts.contains(&(dir, e.to_string()))) {
        return 1; // the folder holds such files
    }
    // Its top folder is there, if not the rest of the way (`~mods` not created yet).
    let first = c.first().unwrap_or_default().to_lowercase();
    usize::from(c.depth() > 1 && game.dirs.contains(&join(target, &first)))
}

/// Single folders wrapping the whole mod ("My Mod 1.2/…", maybe beside its readme), unless the\n/// install rules name them.
fn wrapper_chain(rels: &[RelPath], def: &CompiledDef) -> String {
    let keep: Vec<String> = def
        .content_top_dirs()
        .into_iter()
        .map(|d| d.to_lowercase())
        .collect();
    let mut root = String::new();
    for _ in 0..ROOT_DEPTH {
        // Readmes beside a wrapper folder do not stop it being one.
        let content: Vec<RelPath> = rels
            .iter()
            .filter_map(|r| under(r, &root))
            .filter(|c| !is_doc(c))
            .collect();
        let first = |c: &RelPath| c.first().unwrap_or_default().to_lowercase();
        let Some(top) = content.first().filter(|c| c.depth() > 1).map(first) else {
            break;
        };
        if keep.contains(&top) || !content.iter().all(|c| c.depth() > 1 && first(c) == top) {
            break;
        }
        let name = content[0].first().unwrap_or_default().to_string();
        root = join(&root, &name);
    }
    root
}

/// A mod's placement, with what it does to each file, for the "Where files go" dialog.
#[derive(Debug, Clone)]
pub struct PlacementView {
    /// The placement shown: the one asked about, else the mod's own.
    pub placement: Placement,
    /// What UMM would suggest now.
    pub suggested: Placement,
    /// Folders inside the mod, to pick a root from.
    pub folders: Vec<String>,
    /// Folders in the game, to pick a target from.
    pub game_folders: Vec<String>,
    pub files: Vec<PlacedFile>,
}

#[derive(Debug, Clone)]
pub struct PlacedFile {
    /// Path inside the mod.
    pub file: String,
    /// Where it goes in the game folder; `None` when it is left out.
    pub target: Option<String>,
    /// Why it is left out.
    pub note: Option<String>,
    pub skipped: bool,
}

impl Session {
    pub(crate) fn game_index(&self, def: &CompiledDef) -> GameIndex {
        match self.install_path(&def.def) {
            Ok(Some((root, _))) => GameIndex::scan(&root),
            _ => GameIndex::default(),
        }
    }

    /// Gives a newly imported mod its placement: the one chosen for the version it replaces,
    /// when that still fits, else a suggestion.
    pub(crate) fn place_imported(
        &self,
        def: &CompiledDef,
        meta: ModMeta,
        previous: Option<Placement>,
    ) -> Result<ModMeta> {
        let game = self.data.game(&def.def.id);
        let files: Vec<&str> = meta.files.keys().map(String::as_str).collect();
        let kept = previous
            .filter(|p| p.chosen)
            .map(|mut p| {
                // Files left out that the new version no longer has are dropped.
                p.skip
                    .retain(|s| files.iter().any(|f| f.eq_ignore_ascii_case(s)));
                p
            })
            .and_then(|p| p.checked(files.iter().copied()).ok());
        let placement = kept.or_else(|| suggest(&files, &self.game_index(def), def));
        if placement.is_none() {
            return Ok(meta);
        }
        Ok(umm_store::set_placement(&game, &meta.id, placement)?)
    }

    /// A stored mod that placements apply to: not the loader pack, nor a package laid out
    /// for the game already.
    fn placeable(&self, game_id: &str, mod_id: &str) -> Result<(CompiledDef, ModMeta)> {
        let def = self.def(game_id)?;
        let meta = umm_store::load_mod(&self.data.game(game_id), mod_id)?;
        let loader = def.def.loader.as_ref().and_then(|l| l.source_mod());
        if meta.layout != umm_store::Layout::Raw || meta.thunderstore.is_some() {
            return Err(CoreError::Invalid(format!(
                "{} is laid out by its package; its files cannot be moved",
                meta.name
            )));
        }
        if loader == Some(mod_id) {
            return Err(CoreError::Invalid(format!(
                "{} is the game's mod loader; its files go where the loader needs them",
                meta.name
            )));
        }
        if meta.fomod.is_some() {
            return Err(CoreError::Invalid(format!(
                "{} has a FOMOD installer, which decides where its files go; change its options instead",
                meta.name
            )));
        }
        Ok((def, meta))
    }

    /// Where a mod's files go under `placement` (default: its own), and what UMM suggests.
    pub fn placement_view(
        &self,
        game_id: &str,
        mod_id: &str,
        placement: Option<Placement>,
    ) -> Result<PlacementView> {
        let (def, meta) = self.placeable(game_id, mod_id)?;
        let files: Vec<&str> = meta.files.keys().map(String::as_str).collect();
        let ix = self.game_index(&def);
        let root = self.install_path(&def.def)?.map(|(p, _)| p);
        let suggested = suggest(&files, &ix, &def).unwrap_or_default();
        let placement = match placement {
            Some(p) => p
                .checked(files.iter().copied())
                .map_err(CoreError::Invalid)?,
            None => meta.placement.clone().unwrap_or_default(),
        };
        let mut folders: Vec<String> = Vec::new();
        for f in &files {
            let comps: Vec<&str> = f.split('/').collect();
            for n in 1..comps.len() {
                let d = comps[..n].join("/");
                if !folders.iter().any(|x| x.eq_ignore_ascii_case(&d)) {
                    folders.push(d);
                }
            }
        }
        folders.sort_by_key(|d| d.to_lowercase());
        let mut placed = Vec::new();
        for f in &files {
            let rel = RelPath::parse(f)?;
            let (target, note, skipped) = match placement.place(&rel) {
                Err(why) => (
                    None,
                    Some(why.reason().to_string()),
                    why == LeftOut::Skipped,
                ),
                Ok(p) => match def.map_mod_file(mod_id, &p)? {
                    None => (None, Some("no install rule matches".to_string()), false),
                    Some(t)
                        if def.is_protected(&t)
                            && root.as_ref().is_none_or(|r| t.to_path(r).exists()) =>
                    {
                        (
                            None,
                            Some(format!(
                                "{t} is a game file protected by the game definition"
                            )),
                            false,
                        )
                    }
                    Some(t) => (Some(t.to_string()), None, false),
                },
            };
            // A Paradox mod's launcher `.mod`, beside its folder: UMM writes the game its own.
            let launcher = def.def.paradox.is_some()
                && f.to_ascii_lowercase().ends_with(".mod")
                && !f.eq_ignore_ascii_case(crate::paradox::DESCRIPTOR);
            let note = match note {
                Some(_) if launcher && target.is_none() && !skipped => Some(
                    "the mod's descriptor: UMM writes the game one from it, pointing at where the mod goes"
                        .to_string(),
                ),
                n => n,
            };
            placed.push(PlacedFile {
                file: f.to_string(),
                target,
                note,
                skipped,
            });
        }
        Ok(PlacementView {
            placement,
            suggested,
            folders,
            game_folders: ix.folders(3),
            files: placed,
        })
    }

    /// Sets where a mod's files go, as the user chose; `None` goes back to UMM's suggestion.
    /// Refused while mods are deployed.
    pub fn set_placement(
        &self,
        game_id: &str,
        mod_id: &str,
        placement: Option<Placement>,
    ) -> Result<ModMeta> {
        let (def, meta) = self.placeable(game_id, mod_id)?;
        let game = self.data.game(game_id);
        let _lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(game_id.into()));
        }
        let placement = match placement {
            Some(p) => Some(Placement {
                guessed: false,
                chosen: true,
                ..p
            }),
            None => {
                let files: Vec<&str> = meta.files.keys().map(String::as_str).collect();
                suggest(&files, &self.game_index(&def), &def)
            }
        };
        Ok(umm_store::set_placement(&game, mod_id, placement)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use umm_gamedef::GameDef;

    fn def(rules: &str) -> CompiledDef {
        let text = format!(
            "schema = 1\nname = \"G\"\n{rules}\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n"
        );
        GameDef::parse_game(&text, Path::new("game.toml"), "g")
            .unwrap()
            .compile()
            .unwrap()
    }

    fn game() -> GameIndex {
        GameIndex::of(&[
            "DeadIslandGame.exe",
            "game_x86_rwdi.dll",
            "DI/",
            "DI/Data/",
            "DI/Data/textures/",
            "DI/Data/textures/rock.dds",
            "DI/Data/sound.fsb",
            "steam_appid.txt",
        ])
    }

    #[test]
    fn a_replacement_dll_with_docs() {
        let p = suggest(
            &["game_x86_rwdi.dll", "README.txt", "SHA256SUMS.txt"],
            &game(),
            &def(""),
        )
        .unwrap();
        assert_eq!((p.root.as_str(), p.target.as_str()), ("", ""));
        assert_eq!(p.skip, ["README.txt", "SHA256SUMS.txt"]);
        assert!(!p.guessed);
    }

    #[test]
    fn finds_the_folder_the_content_belongs_in() {
        // Wrapped, and laid out from the game's data folder rather than its root.
        let p = suggest(
            &[
                "Better Rocks v2/textures/rock.dds",
                "Better Rocks v2/textures/moss.dds",
                "Better Rocks v2/Better Rocks.url",
                "Better Rocks v2/screenshots/a.png",
            ],
            &game(),
            &def(""),
        )
        .unwrap();
        assert_eq!(
            (p.root.as_str(), p.target.as_str()),
            ("Better Rocks v2", "DI/Data")
        );
        assert_eq!(
            p.skip,
            [
                "Better Rocks v2/Better Rocks.url",
                "Better Rocks v2/screenshots/a.png"
            ]
        );

        // Already laid out from the game folder, under a wrapper.
        let p = suggest(
            &["Wrap/DI/Data/sound.fsb", "Wrap/readme.md"],
            &game(),
            &def(""),
        )
        .unwrap();
        assert_eq!((p.root.as_str(), p.target.as_str()), ("Wrap", ""));

        // Loose files of a kind a folder holds.
        let p = suggest(&["louder.fsb"], &game(), &def("")).unwrap();
        assert_eq!(p.target, "DI/Data");
    }

    #[test]
    fn plugin_folders_go_where_the_rules_put_plugins() {
        // As Thunderstore BepInEx games are set up.
        let bep = def(
            "[[install]]\nmatch = \"BepInEx/**\"\ntarget = \"BepInEx/\"\n\
             [[install]]\nmatch = \"plugins/**\"\ntarget = \"BepInEx/plugins/{mod}/\"\n\
             [[install]]\nmatch = \"config/**\"\ntarget = \"BepInEx/config/\"\n\
             [[install]]\nmatch = \"*.dll\"\ntarget = \"BepInEx/plugins/{mod}/\"",
        );
        let p = suggest(
            &[
                "Cool-1.2/Cool/Cool.dll",
                "Cool-1.2/Cool/assets/x.bundle",
                "Cool-1.2/README.md",
            ],
            &GameIndex::default(),
            &bep,
        )
        .unwrap();
        assert_eq!(
            (p.root.as_str(), p.target.as_str()),
            ("Cool-1.2/Cool", "plugins")
        );
        assert!(!p.guessed);
    }

    #[test]
    fn two_versions_side_by_side_are_a_guess() {
        let p = suggest(
            &[
                "Fix/Main/game_x86_rwdi.dll",
                "Fix/Optional - No Blur/game_x86_rwdi.dll",
            ],
            &game(),
            &def(""),
        )
        .unwrap();
        assert_eq!(p.root, "Fix/Main", "the first, until the user picks");
        assert!(p.guessed);
    }

    #[test]
    fn nothing_to_go_by_is_a_guess() {
        let p = suggest(&["Cool Mod/stuff/thing.bin"], &game(), &def("")).unwrap();
        assert_eq!(p.root, "Cool Mod/stuff", "wrappers dropped");
        assert!(p.guessed);
        // A game whose install rules know the file needs no guess.
        let bep = def("[[install]]\nmatch = \"*.dll\"\ntarget = \"BepInEx/plugins/{mod}/\"");
        let p = suggest(&["Cool Mod/Cool.dll", "Cool Mod/README.txt"], &game(), &bep).unwrap();
        assert_eq!(p.root, "Cool Mod");
        assert!(!p.guessed);
        // A mod laid out like the game folder needs no placement at all.
        assert!(suggest(&["DI/Data/sound.fsb"], &game(), &def("")).is_none());
    }

    #[test]
    fn bethesda_mods_go_in_the_data_folder() {
        let fo3 =
            def("[mod_list]\nlist = \"%LOCALAPPDATA%/Fallout3/plugins.txt\"\nfolder = \"Data\"");
        // Plain Fallout 3: its plugins are .esm, its assets in archives.
        let game = GameIndex::of(&[
            "Fallout3.exe",
            "Data/",
            "Data/Fallout3.esm",
            "Data/Fallout - Meshes.bsa",
        ]);
        let p = suggest(&["Mod.esp", "Read Me Mod.txt"], &game, &fo3).unwrap();
        assert_eq!(
            (p.root.as_str(), p.target.as_str(), p.guessed),
            ("", "Data", false)
        );
        assert_eq!(p.skip, ["Read Me Mod.txt"]);
        let p = suggest(&["Mod 1.0/meshes/a.nif", "Mod 1.0/Mod.esp"], &game, &fo3).unwrap();
        assert_eq!((p.root.as_str(), p.target.as_str()), ("Mod 1.0", "Data"));
        let p = suggest(&["fose/plugins/x.dll"], &game, &fo3).unwrap();
        assert_eq!(p.target, "Data");
        // Already laid out from the game folder: left as it is.
        assert!(suggest(&["Data/Mod.esp"], &game, &fo3).is_none());
        // The script extender's loader goes beside the game, and its source code nowhere: the
        // archive's top stays its root.
        let p = suggest(
            &["fose_loader.exe", "fose_1_7.dll", "fose_readme.txt"],
            &game,
            &fo3,
        );
        assert!(p.is_none_or(|p| p.target.is_empty()));
        let mut fose = vec![
            "fose_loader.exe",
            "fose_1_7.dll",
            "fose_1_7ng.dll",
            "fose_readme.txt",
        ];
        let src: Vec<String> = (0..98)
            .map(|i| format!("src/fose/f{i}.cpp"))
            .chain((0..72).map(|i| format!("src/common/c{i}.h")))
            .collect();
        fose.extend(src.iter().map(String::as_str));
        let p = suggest(&fose, &game, &fo3);
        assert!(
            p.as_ref()
                .is_none_or(|p| p.root.is_empty() && p.target.is_empty()),
            "{p:?}"
        );
    }

    #[test]
    fn recognises_docs() {
        let doc = |p: &str| is_doc(&RelPath::parse(p).unwrap());
        for p in [
            "README.txt",
            "Read Me.txt",
            "read_me",
            "CHANGELOG.md",
            "License",
            "x.url",
            "a/b.nfo",
            "fomod/info.xml",
            "Screenshots/1.jpg",
            "guide.pdf",
            "Read Me NCR Outcast.txt",
            "src/fose/GameAPI.cpp",
            "loader.vcxproj",
            "StreetLights Wasteland Readme.txt",
            "fellout-readme-quick.txt",
            "fose_whatsnew.txt",
        ] {
            assert!(doc(p), "{p}");
        }
        for p in [
            "steam_appid.txt",
            "config.txt",
            "data/readme.txt",
            "x.dll",
            "images/a.png",
        ] {
            assert!(!doc(p), "{p}");
        }
    }
}
