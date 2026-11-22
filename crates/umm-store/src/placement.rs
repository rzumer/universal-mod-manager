//! Where a mod's files go in the game folder, for mods whose archive is not laid out like it.
//! The files stay as extracted; a placement picks the folder inside the mod whose content is
//! deployed (`root`), the game folder that content goes into (`target`), and files left out
//! (`skip`). It applies before the game's install rules, and changing it is a metadata edit.

use serde::{Deserialize, Serialize};
use umm_gamedef::RelPath;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// The folder inside the mod whose content is deployed; empty for the whole mod. Files
    /// outside it are left out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root: String,
    /// The game folder that content goes into; empty for the game folder itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// Files left out (readmes, checksums, ...), as paths inside the mod.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip: Vec<String>,
    /// UMM found nothing in the game folder to go by: the placement is a guess to check.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub guessed: bool,
    /// Chosen by the user; kept when the mod is updated.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub chosen: bool,
}

/// Why a file of a placed mod is not deployed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftOut {
    /// Outside the mod's root folder.
    OutsideRoot,
    /// In the placement's skip list.
    Skipped,
}

impl LeftOut {
    pub fn reason(self) -> &'static str {
        match self {
            LeftOut::OutsideRoot => "outside the folder this mod installs from",
            LeftOut::Skipped => "left out: documentation or similar",
        }
    }
}

impl Placement {
    /// The path a mod file has before the install rules apply.
    pub fn place(&self, file: &RelPath) -> Result<RelPath, LeftOut> {
        let key = file.key();
        if self
            .skip
            .iter()
            .any(|s| RelPath::parse(s).is_ok_and(|s| s.key() == key))
        {
            return Err(LeftOut::Skipped);
        }
        let root = RelPath::parse(&self.root).map_err(|_| LeftOut::OutsideRoot)?;
        let n = root.depth();
        let under = file.depth() > n
            && file
                .components()
                .zip(root.components())
                .all(|(a, b)| a.eq_ignore_ascii_case(b));
        if !under {
            return Err(LeftOut::OutsideRoot);
        }
        let target = RelPath::parse(&self.target).map_err(|_| LeftOut::OutsideRoot)?;
        Ok(target.join(&file.strip_components(n)))
    }

    /// Checks the paths against the mod's files: `root` must be one of its folders (or empty),
    /// `target` a plain relative path, and `skip` only names its files. Normalises separators.
    pub fn checked<'a>(
        mut self,
        files: impl Iterator<Item = &'a str> + Clone,
    ) -> Result<Self, String> {
        let norm = |s: &str, what: &str| -> Result<String, String> {
            RelPath::parse(s.trim())
                .map(String::from)
                .map_err(|e| format!("{what} `{s}`: {e}"))
        };
        self.root = norm(&self.root, "folder")?;
        self.target = norm(&self.target, "game folder")?;
        if !self.root.is_empty() {
            let prefix = format!("{}/", self.root.to_lowercase());
            if !files.clone().any(|f| f.to_lowercase().starts_with(&prefix)) {
                return Err(format!("the mod has no folder `{}`", self.root));
            }
        }
        let known: std::collections::HashSet<String> = files.map(|f| f.to_lowercase()).collect();
        let mut skip = Vec::new();
        for s in &self.skip {
            let s = norm(s, "file")?;
            if !known.contains(&s.to_lowercase()) {
                return Err(format!("the mod has no file `{s}`"));
            }
            if !skip.iter().any(|k: &String| k.eq_ignore_ascii_case(&s)) {
                skip.push(s);
            }
        }
        skip.sort();
        self.skip = skip;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(s: &str) -> RelPath {
        RelPath::parse(s).unwrap()
    }

    #[test]
    fn places_files_under_the_root_into_the_target() {
        let p = Placement {
            root: "SprintFix 1.0".into(),
            target: "Data".into(),
            skip: vec!["SprintFix 1.0/README.txt".into()],
            ..Default::default()
        };
        assert_eq!(p.place(&rel("SprintFix 1.0/x.dll")), Ok(rel("Data/x.dll")));
        assert_eq!(
            p.place(&rel("sprintfix 1.0/sub/y.pak")),
            Ok(rel("Data/sub/y.pak"))
        );
        assert_eq!(
            p.place(&rel("SprintFix 1.0/readme.TXT")),
            Err(LeftOut::Skipped)
        );
        assert_eq!(p.place(&rel("Other/x.dll")), Err(LeftOut::OutsideRoot));
        assert_eq!(p.place(&rel("SprintFix 1.0")), Err(LeftOut::OutsideRoot));
        assert_eq!(Placement::default().place(&rel("a/b")), Ok(rel("a/b")));
    }

    #[test]
    fn checks_paths_against_the_files() {
        let files = ["Mod/a.dll", "Mod/README.md"];
        let ok = Placement {
            root: "mod\\".into(),
            skip: vec!["Mod/README.md".into(), "mod/readme.md".into()],
            ..Default::default()
        }
        .checked(files.iter().copied())
        .unwrap();
        assert_eq!((ok.root.as_str(), ok.skip.len()), ("mod", 1));
        let bad = |p: Placement| p.checked(files.iter().copied()).is_err();
        assert!(bad(Placement {
            root: "Nope".into(),
            ..Default::default()
        }));
        assert!(bad(Placement {
            target: "../out".into(),
            ..Default::default()
        }));
        assert!(bad(Placement {
            skip: vec!["Mod/none.txt".into()],
            ..Default::default()
        }));
    }
}
