//! Importing mods into the store. A stored mod is never modified after import:
//! `mods/<id>/files/` holds its content and `mods/<id>/mod.toml` its metadata and file hashes.

use crate::fsutil::{self, clear_readonly, hash_file, IoCtx};
use crate::placement::Placement;
use crate::{read_toml, write_toml, GameDir, Result, StoreError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use umm_gamedef::{is_valid_id, RelPath};
use walkdir::WalkDir;

pub const MOD_META_FILE: &str = "mod.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModMeta {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub imported_unix: u64,
    /// How the files are laid out; see [`Layout`].
    #[serde(default, skip_serializing_if = "Layout::is_raw")]
    pub layout: Layout,
    /// Set for packages installed from Thunderstore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thunderstore: Option<ThunderstoreMeta>,
    /// Set for files installed from Nexus Mods.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus: Option<NexusMeta>,
    /// Where the files go, for a mod not laid out like the game folder. Applies before the
    /// install rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<Placement>,
    /// For a mod with a FOMOD installer: the choices made and the files they install, which
    /// replace the placement's folder and skip list (its game folder still applies).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fomod: Option<FomodMeta>,
    /// Path inside the mod (`/`-separated) → size and hash.
    pub files: BTreeMap<String, FileRecord>,
}

/// Whether a mod's files still go through the game definition's install rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// As imported: the install rules decide where each file goes.
    #[default]
    Raw,
    /// Already laid out relative to the game folder (Thunderstore packages).
    Game,
}

impl Layout {
    pub fn is_raw(&self) -> bool {
        *self == Layout::Raw
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThunderstoreMeta {
    pub community: String,
    /// `Owner-Name`.
    pub full_name: String,
    pub version: String,
    /// `Owner-Name-1.2.3`, as the package's manifest lists them.
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    /// The package is a mod loader (the BepInEx pack).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub loader: bool,
}

/// Where a mod from Nexus Mods came from: one file of one mod there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NexusMeta {
    /// The game's domain name on Nexus (`stardewvalley`).
    pub domain: String,
    /// The game's number on Nexus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<u64>,
    pub mod_id: u64,
    pub file_id: u64,
    /// The file's name on Nexus ("Main File", "Glow Extras").
    pub file_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `MAIN`, `OPTIONAL`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// A mod's FOMOD installer, as it was run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FomodMeta {
    /// The folder inside the mod holding `fomod/`; empty for the mod's top.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root: String,
    /// False until the user has chosen its options: until then nothing of the mod deploys.
    #[serde(default)]
    pub done: bool,
    /// The options chosen, by step, group and option name; replayed on an update.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<FomodChoice>,
    /// The files those choices install: path inside the mod → path in the mod's game folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FomodFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FomodChoice {
    pub step: String,
    pub group: String,
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FomodFile {
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    /// Unwrap a single top-level folder (`MyMod-1.2/…`) unless its name is in `keep_top_dirs`.
    pub unwrap: bool,
    /// Folder names that are real content, not wrappers (e.g. `BepInEx`). Case-insensitive.
    pub keep_top_dirs: Vec<String>,
    pub layout: Layout,
    pub thunderstore: Option<ThunderstoreMeta>,
    pub nexus: Option<NexusMeta>,
    /// Replace an existing mod with the same id (an update). The old one is removed only once
    /// the new one is fully stored.
    pub replace: bool,
    /// Files kept beside the mod in `package/`, by plain file name (a Thunderstore package's
    /// manifest, icon and readme). Never deployed; names with a path in them are ignored.
    pub package_files: std::collections::BTreeMap<String, Vec<u8>>,
}

/// Writes `opts.package_files` into the staged mod's `package/` folder.
fn stage_package_files(new_dir: &Path, opts: &ImportOptions) -> Result<()> {
    let files: Vec<_> = opts
        .package_files
        .iter()
        .filter(|(n, _)| {
            !n.contains(['/', '\\']) && umm_gamedef::RelPath::parse(n).is_ok_and(|r| r.depth() == 1)
        })
        .collect();
    if files.is_empty() {
        return Ok(());
    }
    let dir = new_dir.join("package");
    fs::create_dir_all(&dir).ctx(|| format!("creating {}", dir.display()))?;
    for (name, body) in files {
        let p = dir.join(name);
        fs::write(&p, body).ctx(|| format!("writing {}", p.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileIssue {
    Missing(String),
    Changed(String),
    Extra(String),
}

impl std::fmt::Display for FileIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileIssue::Missing(p) => write!(f, "missing: {p}"),
            FileIssue::Changed(p) => write!(f, "changed: {p}"),
            FileIssue::Extra(p) => write!(f, "unexpected file: {p}"),
        }
    }
}

pub fn slugify(s: &str) -> String {
    umm_gamedef::slug(s).unwrap_or_else(|| "mod".into())
}

pub fn import_mod(game: &GameDir, src: &Path, opts: &ImportOptions) -> Result<ModMeta> {
    game.ensure()?;
    // A folder's whole name ("Cool Mod 1.0"); an archive's name without its extension.
    let base = if src.is_dir() {
        src.file_name()
    } else {
        src.file_stem()
    };
    let stem = base
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = opts.name.clone().unwrap_or_else(|| stem.clone());
    let id = opts.id.clone().unwrap_or_else(|| slugify(&name));
    if !is_valid_id(&id) {
        return Err(StoreError::InvalidId(id));
    }
    let dest = game.mod_dir(&id);
    if dest.exists() && !opts.replace {
        return Err(StoreError::ModExists(id));
    }

    let staging = game.staging_dir().join(fsutil::unique_suffix());
    fs::create_dir_all(&staging).ctx(|| format!("creating {}", staging.display()))?;
    if dest.exists() {
        // An update: build the new mod beside the old one, then swap them.
        let result = (|| -> Result<ModMeta> {
            let new_dir = staging.join("new");
            let meta = stage_mod(src, &staging, &new_dir, &id, name, opts)?;
            stage_package_files(&new_dir, opts)?;
            let old = staging.join("old");
            fs::rename(&dest, &old).ctx(|| format!("moving {} aside", dest.display()))?;
            if let Err(e) = fs::rename(&new_dir, &dest) {
                let _ = fs::rename(&old, &dest);
                let failed: io::Result<()> = Err(e);
                failed.ctx(|| format!("moving the update into {}", dest.display()))?;
            }
            Ok(meta)
        })();
        let _ = remove_tree(&staging);
        return result;
    }
    let result = (|| -> Result<ModMeta> {
        let new_dir = staging.join("new");
        let meta = stage_mod(src, &staging, &new_dir, &id, name, opts)?;
        stage_package_files(&new_dir, opts)?;
        fs::create_dir_all(game.mods_dir()).ctx(|| "creating the mods folder".to_string())?;
        fs::rename(&new_dir, &dest).ctx(|| format!("moving mod into {}", dest.display()))?;
        Ok(meta)
    })();
    let _ = remove_tree(&staging);
    result
}

fn remove_tree(p: &Path) -> io::Result<()> {
    fs::remove_dir_all(p)
}

/// Builds a complete mod folder (`files/` + `mod.toml`) at `out`, using `staging` for scratch.
fn stage_mod(
    src: &Path,
    staging: &Path,
    out: &Path,
    id: &str,
    name: String,
    opts: &ImportOptions,
) -> Result<ModMeta> {
    let content = staging.join("content");
    if src.is_dir() {
        copy_tree(src, &content)?;
    } else {
        match archive_kind(src) {
            Some(ArchiveKind::Zip) => extract_zip(src, &content)?,
            Some(ArchiveKind::SevenZip) => extract_7z(src, &content)?,
            Some(ArchiveKind::Rar) => extract_rar(src, &content)?,
            None => return Err(StoreError::Unsupported(src.to_path_buf())),
        }
    }
    let root = if opts.unwrap {
        unwrap_root(&content, &opts.keep_top_dirs)?
    } else {
        content
    };
    let files = index_files(&root)?;
    if files.is_empty() {
        return Err(StoreError::EmptyMod);
    }
    fs::create_dir_all(out).ctx(|| format!("creating {}", out.display()))?;
    let files_dir = out.join("files");
    fs::rename(&root, &files_dir).ctx(|| format!("moving mod into {}", files_dir.display()))?;
    let meta = ModMeta {
        id: id.to_string(),
        name,
        version: opts.version.clone(),
        // Only a remote origin is worth recording: an imported file or folder's content is
        // copied into the store, so its original path says nothing useful.
        source: opts.source.clone(),
        imported_unix: fsutil::unix_now(),
        layout: opts.layout,
        thunderstore: opts.thunderstore.clone(),
        nexus: opts.nexus.clone(),
        placement: None,
        fomod: None,
        files,
    };
    write_toml(&out.join(MOD_META_FILE), &meta)?;
    Ok(meta)
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    for entry in WalkDir::new(src).follow_links(false) {
        let entry = entry.map_err(|e| {
            let p = e.path().map(Path::to_path_buf).unwrap_or_default();
            StoreError::Io(crate::IoError {
                context: format!("reading {}", p.display()),
                source: e
                    .into_io_error()
                    .unwrap_or_else(|| io::Error::other("walk error")),
            })
        })?;
        let ft = entry.file_type();
        if ft.is_symlink() {
            return Err(StoreError::Symlink(entry.path().to_path_buf()));
        }
        let rel = entry.path().strip_prefix(src).unwrap_or(entry.path());
        let rel = RelPath::from_path(rel)?;
        let out = rel.to_path(dst);
        if ft.is_dir() {
            fs::create_dir_all(&out).ctx(|| format!("creating {}", out.display()))?;
        } else {
            fs::copy(entry.path(), &out).ctx(|| format!("copying {}", entry.path().display()))?;
            clear_readonly(&out).ctx(|| format!("updating {}", out.display()))?;
        }
    }
    Ok(())
}

/// Archive formats mods come in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    SevenZip,
    Rar,
}

/// The archive format of a file, by its extension.
pub fn archive_kind(p: &Path) -> Option<ArchiveKind> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "zip" => Some(ArchiveKind::Zip),
        "7z" => Some(ArchiveKind::SevenZip),
        "rar" => Some(ArchiveKind::Rar),
        _ => None,
    }
}

/// Where an archive entry named `name` goes under `dst`; refused if it would leave `dst`.
/// `None` for the archive's root itself.
fn entry_path(dst: &Path, name: &str) -> Result<Option<PathBuf>> {
    let rel = RelPath::parse(name).map_err(|_| StoreError::UnsafeEntry(name.to_string()))?;
    Ok((!rel.is_root()).then(|| rel.to_path(dst)))
}

fn archive_err(src: &Path, e: impl std::fmt::Display) -> StoreError {
    StoreError::Archive {
        path: src.to_path_buf(),
        message: e.to_string(),
    }
}

fn extract_7z(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).ctx(|| format!("creating {}", dst.display()))?;
    let mut reader = sevenz_rust2::ArchiveReader::open(src, sevenz_rust2::Password::empty())
        .map_err(|e| archive_err(src, e))?;
    // Errors writing files are kept here: the reader's own error type cannot carry them.
    let mut failed: Option<StoreError> = None;
    let result = reader.for_each_entries(|entry, data| {
        let out = match entry_path(dst, entry.name()) {
            Ok(Some(out)) => out,
            Ok(None) => return Ok(true),
            Err(e) => {
                failed = Some(e);
                return Ok(false);
            }
        };
        let mut write = || -> Result<()> {
            if entry.is_directory() {
                fs::create_dir_all(&out).ctx(|| format!("creating {}", out.display()))?;
                return Ok(());
            }
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
            }
            let mut o = File::create(&out).ctx(|| format!("creating {}", out.display()))?;
            io::copy(data, &mut o).ctx(|| format!("extracting {}", entry.name()))?;
            Ok(())
        };
        match write() {
            Ok(()) => Ok(true),
            Err(e) => {
                failed = Some(e);
                Ok(false)
            }
        }
    });
    if let Some(e) = failed {
        return Err(e);
    }
    result.map_err(|e| archive_err(src, e))
}

fn extract_rar(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).ctx(|| format!("creating {}", dst.display()))?;
    let mut archive = unrar::Archive::new(src)
        .open_for_processing()
        .map_err(|e| archive_err(src, e))?;
    while let Some(header) = archive.read_header().map_err(|e| archive_err(src, e))? {
        let name = header.entry().filename.to_string_lossy().into_owned();
        let out = entry_path(dst, &name)?;
        archive = match out {
            Some(out) if header.entry().is_file() => {
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
                }
                header.extract_to(&out).map_err(|e| archive_err(src, e))?
            }
            Some(out) => {
                fs::create_dir_all(&out).ctx(|| format!("creating {}", out.display()))?;
                header.skip().map_err(|e| archive_err(src, e))?
            }
            None => header.skip().map_err(|e| archive_err(src, e))?,
        };
    }
    Ok(())
}

fn extract_zip(src: &Path, dst: &Path) -> Result<()> {
    let f = File::open(src).ctx(|| format!("opening {}", src.display()))?;
    let mut zip = zip::ZipArchive::new(f)?;
    fs::create_dir_all(dst).ctx(|| format!("creating {}", dst.display()))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(name) = entry.enclosed_name() else {
            return Err(StoreError::UnsafeEntry(entry.name().to_string()));
        };
        let rel = RelPath::from_path(&name)?;
        if rel.is_root() {
            continue;
        }
        let out = rel.to_path(dst);
        if entry.is_dir() {
            fs::create_dir_all(&out).ctx(|| format!("creating {}", out.display()))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
        }
        let mut o = File::create(&out).ctx(|| format!("creating {}", out.display()))?;
        io::copy(&mut entry, &mut o).ctx(|| format!("extracting {}", rel))?;
    }
    Ok(())
}

fn unwrap_root(content: &Path, keep: &[String]) -> Result<PathBuf> {
    let entries: Vec<fs::DirEntry> = fs::read_dir(content)
        .ctx(|| format!("reading {}", content.display()))?
        .filter_map(|e| e.ok())
        .collect();
    if let [only] = entries.as_slice() {
        let name = only.file_name().to_string_lossy().into_owned();
        let is_dir = only.file_type().is_ok_and(|t| t.is_dir());
        if is_dir && !keep.iter().any(|k| k.eq_ignore_ascii_case(&name)) {
            return Ok(only.path());
        }
    }
    Ok(content.to_path_buf())
}

fn index_files(root: &Path) -> Result<BTreeMap<String, FileRecord>> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|e| {
            StoreError::Io(crate::IoError {
                context: format!("indexing {}", root.display()),
                source: e
                    .into_io_error()
                    .unwrap_or_else(|| io::Error::other("walk error")),
            })
        })?;
        if entry.file_type().is_symlink() {
            return Err(StoreError::Symlink(entry.path().to_path_buf()));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = RelPath::from_path(entry.path().strip_prefix(root).unwrap_or(entry.path()))?;
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let hash = hash_file(entry.path()).ctx(|| format!("hashing {}", entry.path().display()))?;
        files.insert(String::from(rel), FileRecord { size, hash });
    }
    Ok(files)
}

pub fn load_mod(game: &GameDir, id: &str) -> Result<ModMeta> {
    let p = game.mod_dir(id).join(MOD_META_FILE);
    if !is_valid_id(id) || !p.exists() {
        return Err(StoreError::ModNotFound(id.to_string()));
    }
    read_toml(&p)
}

/// Sets or clears a mod's placement, after checking it against the mod's files. Only the
/// metadata changes; the files stay as they are.
pub fn set_placement(game: &GameDir, id: &str, placement: Option<Placement>) -> Result<ModMeta> {
    let mut meta = load_mod(game, id)?;
    meta.placement = match placement {
        Some(p) => Some(
            p.checked(meta.files.keys().map(String::as_str))
                .map_err(StoreError::Placement)?,
        ),
        None => None,
    };
    write_toml(&game.mod_dir(id).join(MOD_META_FILE), &meta)?;
    Ok(meta)
}

/// Sets or clears a mod's FOMOD installer record. Every file it names must be one of the
/// mod's own.
pub fn set_fomod(game: &GameDir, id: &str, fomod: Option<FomodMeta>) -> Result<ModMeta> {
    let mut meta = load_mod(game, id)?;
    if let Some(f) = &fomod {
        for file in &f.files {
            if !meta.files.contains_key(&file.source) {
                return Err(StoreError::Placement(format!(
                    "the installer names `{}`, which the mod does not have",
                    file.source
                )));
            }
            RelPath::parse(&file.destination)?;
        }
    }
    meta.fomod = fomod;
    write_toml(&game.mod_dir(id).join(MOD_META_FILE), &meta)?;
    Ok(meta)
}

/// Every complete mod in the store. Folders without `mod.toml` (an interrupted import) are
/// skipped; `umm doctor` reports them.
pub fn list_mods(game: &GameDir) -> Result<Vec<ModMeta>> {
    let Ok(rd) = fs::read_dir(game.mods_dir()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for e in rd.filter_map(|e| e.ok()) {
        if e.path().join(MOD_META_FILE).is_file() {
            out.push(read_toml(&e.path().join(MOD_META_FILE))?);
        }
    }
    out.sort_by(|a: &ModMeta, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Compares a stored mod with its recorded hashes.
pub fn verify_mod(game: &GameDir, meta: &ModMeta) -> Result<Vec<FileIssue>> {
    let root = game.mod_files_dir(&meta.id);
    let mut issues = Vec::new();
    let on_disk = if root.is_dir() {
        index_files(&root)?
    } else {
        BTreeMap::new()
    };
    for (path, rec) in &meta.files {
        match on_disk.get(path) {
            None => issues.push(FileIssue::Missing(path.clone())),
            Some(r) if r != rec => issues.push(FileIssue::Changed(path.clone())),
            _ => {}
        }
    }
    for path in on_disk.keys() {
        if !meta.files.contains_key(path) {
            issues.push(FileIssue::Extra(path.clone()));
        }
    }
    Ok(issues)
}

/// Deletes a mod from the store. Only called on an explicit user request.
pub fn remove_mod(game: &GameDir, id: &str) -> Result<()> {
    if !is_valid_id(id) || !game.mod_dir(id).exists() {
        return Err(StoreError::ModNotFound(id.to_string()));
    }
    let p = game.mod_dir(id);
    fs::remove_dir_all(&p).ctx(|| format!("removing {}", p.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataDir;
    use std::io::Write;

    fn game() -> (tempfile::TempDir, GameDir) {
        let tmp = tempfile::tempdir().unwrap();
        let data = DataDir::open(tmp.path().join("data")).unwrap();
        let g = data.game("g");
        (tmp, g)
    }

    fn seven_zip(path: &Path, entries: &[(&str, &str)]) {
        let mut w = sevenz_rust2::ArchiveWriter::create(path).unwrap();
        for (name, body) in entries {
            w.push_archive_entry(
                sevenz_rust2::ArchiveEntry::new_file(name),
                Some(body.as_bytes()),
            )
            .unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn imports_7z_and_rar_archives() {
        let (tmp, g) = game();
        let src = tmp.path().join("Glow 1.2.7z");
        seven_zip(
            &src,
            &[("Glow/plugins/glow.dll", "dll"), ("Glow/readme.txt", "hi")],
        );
        let meta = import_mod(
            &g,
            &src,
            &ImportOptions {
                unwrap: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(meta.id, "glow-1-2");
        assert_eq!(
            meta.files.keys().collect::<Vec<_>>(),
            ["plugins/glow.dll", "readme.txt"]
        );
        assert!(verify_mod(&g, &meta).unwrap().is_empty());

        // A small archive from the unrar crate's own tests: one file, VERSION.
        let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/version.rar");
        let meta = import_mod(&g, &rar, &ImportOptions::default()).unwrap();
        assert_eq!(meta.files.keys().collect::<Vec<_>>(), ["VERSION"]);
    }

    #[test]
    fn archive_entries_outside_the_mod_are_refused() {
        let (tmp, g) = game();
        let src = tmp.path().join("evil.7z");
        seven_zip(&src, &[("../evil.dll", "x")]);
        assert!(matches!(
            import_mod(&g, &src, &ImportOptions::default()),
            Err(StoreError::UnsafeEntry(_))
        ));
        assert!(!tmp.path().join("evil.dll").exists());
        assert!(list_mods(&g).unwrap().is_empty());
    }

    #[test]
    fn imports_folder_and_unwraps_wrapper() {
        let (tmp, g) = game();
        let src = tmp.path().join("Cool Mod 1.0");
        fs::create_dir_all(src.join("Cool Mod 1.0/plugins")).unwrap();
        fs::write(src.join("Cool Mod 1.0/plugins/a.dll"), b"a").unwrap();
        let meta = import_mod(
            &g,
            &src,
            &ImportOptions {
                unwrap: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(meta.id, "cool-mod-1-0");
        assert_eq!(meta.files.keys().collect::<Vec<_>>(), ["plugins/a.dll"]);
        assert!(verify_mod(&g, &meta).unwrap().is_empty());
        assert!(matches!(
            import_mod(&g, &src, &ImportOptions::default()),
            Err(StoreError::ModExists(_))
        ));
    }

    #[test]
    fn keeps_content_folder_and_detects_changes() {
        let (tmp, g) = game();
        let src = tmp.path().join("pack");
        fs::create_dir_all(src.join("BepInEx/core")).unwrap();
        fs::write(src.join("BepInEx/core/x.dll"), b"x").unwrap();
        let opts = ImportOptions {
            unwrap: true,
            keep_top_dirs: vec!["bepinex".into()],
            ..Default::default()
        };
        let meta = import_mod(&g, &src, &opts).unwrap();
        assert!(meta.files.contains_key("BepInEx/core/x.dll"));
        fs::write(
            g.mod_files_dir("pack").join("BepInEx/core/x.dll"),
            b"tampered",
        )
        .unwrap();
        assert_eq!(
            verify_mod(&g, &meta).unwrap(),
            [FileIssue::Changed("BepInEx/core/x.dll".into())]
        );
    }

    #[test]
    fn imports_zip_and_rejects_empty() {
        let (tmp, g) = game();
        let zpath = tmp.path().join("z.zip");
        {
            let mut z = zip::ZipWriter::new(File::create(&zpath).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("data/readme.txt", o).unwrap();
            z.write_all(b"hi").unwrap();
            z.finish().unwrap();
        }
        let meta = import_mod(&g, &zpath, &ImportOptions::default()).unwrap();
        assert_eq!(meta.files.keys().collect::<Vec<_>>(), ["data/readme.txt"]);

        let empty = tmp.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(matches!(
            import_mod(&g, &empty, &ImportOptions::default()),
            Err(StoreError::EmptyMod)
        ));
        assert!(!g.mod_dir("empty").exists());
    }
}
