//! Validated, normalised paths relative to some root (a game folder or a mod's files).
//!
//! Every path the engine writes to goes through [`RelPath`], so a mod or a game definition can
//! never point outside the folder it is applied to.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RelPathError {
    #[error("path `{0}` is absolute")]
    Absolute(String),
    #[error("path `{0}` contains `..`")]
    ParentDir(String),
    #[error("path `{0}` has an invalid component `{1}`")]
    InvalidComponent(String, String),
    #[error("path is not valid UTF-8")]
    NotUtf8,
}

/// A relative path with `/` separators, no `.`/`..`, no empty components and no characters
/// Windows would reject or reinterpret (drive letters, alternate data streams, device names).
/// The empty path is the root itself.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelPath(String);

const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

pub(crate) fn valid_component(c: &str) -> bool {
    if c.is_empty() || c.ends_with('.') || c.ends_with(' ') {
        return false;
    }
    if c.chars()
        .any(|ch| (ch as u32) < 0x20 || "<>:\"|?*\\".contains(ch))
    {
        return false;
    }
    let stem = c.split('.').next().unwrap_or("").to_ascii_lowercase();
    !RESERVED.contains(&stem.as_str())
}

impl RelPath {
    pub fn root() -> Self {
        RelPath(String::new())
    }

    pub fn parse(s: &str) -> Result<Self, RelPathError> {
        let norm = s.replace('\\', "/");
        if norm.starts_with('/') {
            return Err(RelPathError::Absolute(s.to_string()));
        }
        let mut parts: Vec<&str> = Vec::new();
        for c in norm.split('/') {
            match c {
                "" | "." => continue,
                ".." => return Err(RelPathError::ParentDir(s.to_string())),
                c if !valid_component(c) => {
                    return Err(RelPathError::InvalidComponent(s.to_string(), c.to_string()))
                }
                c => parts.push(c),
            }
        }
        Ok(RelPath(parts.join("/")))
    }

    pub fn from_path(p: &Path) -> Result<Self, RelPathError> {
        let mut parts = Vec::new();
        for comp in p.components() {
            match comp {
                Component::Normal(os) => parts.push(os.to_str().ok_or(RelPathError::NotUtf8)?),
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(RelPathError::ParentDir(p.display().to_string()))
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(RelPathError::Absolute(p.display().to_string()))
                }
            }
        }
        Self::parse(&parts.join("/"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Key used to detect two paths that name the same file: lower-case, as mods are made for
    /// Windows, whose file systems ignore case (and so does Wine, for games under Proton).
    pub fn key(&self) -> String {
        self.0.to_lowercase()
    }

    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|c| !c.is_empty())
    }

    pub fn depth(&self) -> usize {
        self.components().count()
    }

    pub fn first(&self) -> Option<&str> {
        self.components().next()
    }

    pub fn file_name(&self) -> Option<&str> {
        self.components().last()
    }

    pub fn parent(&self) -> Option<RelPath> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rsplit_once('/') {
            Some((head, _)) => RelPath(head.to_string()),
            None => RelPath::root(),
        })
    }

    /// Every proper ancestor except the root, shallowest first: `a/b/c` gives `a`, `a/b`.
    pub fn ancestors(&self) -> Vec<RelPath> {
        let parts: Vec<&str> = self.components().collect();
        (1..parts.len())
            .map(|n| RelPath(parts[..n].join("/")))
            .collect()
    }

    pub fn join(&self, other: &RelPath) -> RelPath {
        match (self.is_root(), other.is_root()) {
            (true, _) => other.clone(),
            (_, true) => self.clone(),
            _ => RelPath(format!("{}/{}", self.0, other.0)),
        }
    }

    pub fn strip_components(&self, n: usize) -> RelPath {
        RelPath(self.components().skip(n).collect::<Vec<_>>().join("/"))
    }

    pub fn to_path(&self, root: &Path) -> PathBuf {
        let mut p = root.to_path_buf();
        for c in self.components() {
            p.push(c);
        }
        p
    }
}

impl TryFrom<String> for RelPath {
    type Error = RelPathError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        RelPath::parse(&s)
    }
}

impl From<RelPath> for String {
    fn from(p: RelPath) -> String {
        p.0
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_root() {
            f.write_str(".")
        } else {
            f.write_str(&self.0)
        }
    }
}

impl fmt::Debug for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RelPath({:?})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises() {
        assert_eq!(RelPath::parse("a\\b/./c/").unwrap().as_str(), "a/b/c");
        assert!(RelPath::parse("").unwrap().is_root());
    }

    #[test]
    fn rejects_escapes_and_windows_traps() {
        for bad in [
            "../x",
            "a/../b",
            "/abs",
            "C:/x",
            "C:",
            "a/b:stream",
            "con.txt",
            "dir/NUL",
            "trail.",
            "trail ",
            "a*b",
        ] {
            assert!(RelPath::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn ancestors_and_parent() {
        let p = RelPath::parse("a/b/c.txt").unwrap();
        let anc: Vec<String> = p.ancestors().into_iter().map(String::from).collect();
        assert_eq!(anc, ["a", "a/b"]);
        assert_eq!(p.parent().unwrap().as_str(), "a/b");
        assert!(RelPath::parse("x").unwrap().parent().unwrap().is_root());
    }
}
