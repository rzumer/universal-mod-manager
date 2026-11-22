//! Suggesting where a mod's files go, for archives not laid out like the game folder (most
//! Nexus and hand-made archives). UMM tries each folder of the mod as its root and each game
//! folder as its target, and keeps the pair whose files line up best with what the game
//! already has. Readmes, checksums and the like are left out.

use crate::{CoreError, ModMeta, Result, Session};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use umm_deploy::GameLock;
use umm_gamedef::{CompiledDef, RelPath, IN_GAME_BACKUP_DIR};
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
    // Anywhere: links, checksums, release notes and OS clutter.
    if matches!(
        e,
        "url" | "nfo" | "sfv" | "md5" | "sha1" | "sha256" | "sha512"
    ) || matches!(name.as_str(), "thumbs.db" | "desktop.ini" | ".ds_store")
    {
        return true;
    }
    let depth = rel.depth();
    if depth > 1 {
        let top = rel.first().unwrap_or_default().to_lowercase();
        return matches!(
            top.as_str(),
            "fomod" | "screenshots" | "screenshot" | "docs" | "documentation" | "__macosx"
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
    let skip: Vec<String> = rels
        .iter()
        .filter(|r| {
            under(r, &root).is_some_and(|c| {
                is_doc(&c)
                    && !game
                        .files
                        .contains(&join(&target, c.as_str()).to_lowercase())
            })
        })
        .map(|r| r.as_str().to_string())
        .collect();
    let p = Placement {
        root,
        target,
        skip,
        guessed,
        chosen: false,
    };
    (p != Placement::default()).then_some(p)
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
                    Some(t) if def.is_protected(&t) => (
                        None,
                        Some(format!("{t} is protected by the game definition")),
                        false,
                    ),
                    Some(t) => (Some(t.to_string()), None, false),
                },
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
