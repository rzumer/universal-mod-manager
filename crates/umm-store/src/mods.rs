//! Importing mods into the store. A stored mod is never modified after import:
//! `mods/<id>/files/` holds its content and `mods/<id>/mod.toml` its metadata and file hashes.

use crate::fsutil::{self, clear_readonly, hash_file, IoCtx};
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
    /// Path inside the mod (`/`-separated) → size and hash.
    pub files: BTreeMap<String, FileRecord>,
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
    if dest.exists() {
        return Err(StoreError::ModExists(id));
    }

    let staging = game.staging_dir().join(fsutil::unique_suffix());
    fs::create_dir_all(&staging).ctx(|| format!("creating {}", staging.display()))?;
    let result = (|| {
        let content = staging.join("content");
        if src.is_dir() {
            copy_tree(src, &content)?;
        } else if src
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
        {
            extract_zip(src, &content)?;
        } else {
            return Err(StoreError::Unsupported(src.to_path_buf()));
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
        fs::create_dir_all(&dest).ctx(|| format!("creating {}", dest.display()))?;
        let files_dir = game.mod_files_dir(&id);
        fs::rename(&root, &files_dir).ctx(|| format!("moving mod into {}", files_dir.display()))?;
        let meta = ModMeta {
            id: id.clone(),
            name,
            version: opts.version.clone(),
            source: opts
                .source
                .clone()
                .or_else(|| Some(src.display().to_string())),
            imported_unix: fsutil::unix_now(),
            files,
        };
        write_toml(&dest.join(MOD_META_FILE), &meta)?;
        Ok(meta)
    })();
    let _ = fs::remove_dir_all(&staging);
    if result.is_err() && dest.exists() && !dest.join(MOD_META_FILE).exists() {
        let _ = fs::remove_dir_all(&dest);
    }
    result
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
