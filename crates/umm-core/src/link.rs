//! Thunderstore's "Install with Mod Manager" button opens
//! `ror2mm://v1/install/thunderstore.io/<owner>/<name>/<version>/`. UMM handles these links only
//! after the user turns it on; it then registers itself for the current user, replacing any
//! other handler (r2modman's, say). Turning it off removes UMM's registration.

use crate::{Result, Session};
use std::path::Path;

pub const LINK_SCHEME: &str = "ror2mm";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallLink {
    pub owner: String,
    pub name: String,
    pub version: Option<String>,
}

impl InstallLink {
    pub fn full_name(&self) -> String {
        format!("{}-{}", self.owner, self.name)
    }
}

/// Reads `ror2mm://v1/install/thunderstore.io/<owner>/<name>/<version>/`.
pub fn parse_link(url: &str) -> Option<InstallLink> {
    let rest = url.trim().strip_prefix("ror2mm://")?;
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    // Thunderstore names use letters, digits and `_`; versions digits and dots.
    let name_ok =
        |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let version_ok = |s: &str| {
        s.split('.').count() == 3
            && s.split('.')
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    };
    match parts.as_slice() {
        ["v1", "install", host, owner, name, rest @ ..]
            if (*host == "thunderstore.io" || host.ends_with(".thunderstore.io"))
                && name_ok(owner)
                && name_ok(name)
                && rest.len() <= 1 =>
        {
            Some(InstallLink {
                owner: owner.to_string(),
                name: name.to_string(),
                version: rest
                    .first()
                    .filter(|v| version_ok(v))
                    .map(|v| v.to_string()),
            })
        }
        _ => None,
    }
}

/// Where the link handler is registered: the `ror2mm` URL scheme's open command.
pub trait SchemeRegistry: Send + Sync {
    fn command(&self) -> Option<String>;
    fn register(&self, command: &str) -> std::io::Result<()>;
    fn unregister(&self) -> std::io::Result<()>;
}

/// `HKCU\Software\Classes\ror2mm`: per user, no administrator rights needed.
#[cfg(windows)]
pub struct WindowsRegistry;

#[cfg(windows)]
impl SchemeRegistry for WindowsRegistry {
    fn command(&self) -> Option<String> {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(format!(
                r"Software\Classes\{LINK_SCHEME}\shell\open\command"
            ))
            .ok()?
            .get_value::<String, _>("")
            .ok()
    }

    fn register(&self, command: &str) -> std::io::Result<()> {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (root, _) = hkcu.create_subkey(format!(r"Software\Classes\{LINK_SCHEME}"))?;
        root.set_value("", &format!("URL:{LINK_SCHEME} Protocol"))?;
        root.set_value("URL Protocol", &"")?;
        let (cmd, _) = root.create_subkey(r"shell\open\command")?;
        cmd.set_value("", &command.to_string())
    }

    fn unregister(&self) -> std::io::Result<()> {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        match RegKey::predef(HKEY_CURRENT_USER)
            .delete_subkey_all(format!(r"Software\Classes\{LINK_SCHEME}"))
        {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}

/// The platform's registry, where UMM can register itself.
pub fn system_registry() -> Option<Box<dyn SchemeRegistry>> {
    #[cfg(windows)]
    {
        Some(Box::new(WindowsRegistry))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkHandler {
    /// UMM handles the links.
    pub ours: bool,
    /// The command of another program that handles them, if any.
    pub other: Option<String>,
}

fn our_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

fn is_ours(command: &str, exe: &Path) -> bool {
    command
        .to_lowercase()
        .contains(&exe.display().to_string().to_lowercase())
}

impl Session {
    pub fn link_handler(&self, reg: &dyn SchemeRegistry, exe: &Path) -> LinkHandler {
        match reg.command() {
            Some(c) if is_ours(&c, exe) => LinkHandler {
                ours: true,
                other: None,
            },
            other => LinkHandler { ours: false, other },
        }
    }

    /// Turns link handling on or off for this user. Turning it on replaces whatever handled
    /// the links; turning it off removes UMM's registration (and leaves another program's
    /// alone).
    pub fn set_link_handler(
        &self,
        reg: &dyn SchemeRegistry,
        exe: &Path,
        on: bool,
    ) -> Result<LinkHandler> {
        let io = |e: std::io::Error| {
            crate::CoreError::Invalid(format!("updating the link handler: {e}"))
        };
        if on {
            reg.register(&our_command(exe)).map_err(io)?;
        } else if reg.command().is_some_and(|c| is_ours(&c, exe)) {
            reg.unregister().map_err(io)?;
        }
        Ok(self.link_handler(reg, exe))
    }

    /// Games set up for BepInEx from Thunderstore whose community has the package:
    /// `(game id, game name)`. Downloads package lists that are not cached yet.
    pub fn link_games(&self, link: &InstallLink) -> Result<Vec<(String, String)>> {
        let full = link.full_name();
        let mut out = Vec::new();
        for st in self.status()? {
            if !st
                .def
                .loader
                .as_ref()
                .is_some_and(|l| l.from_thunderstore())
            {
                continue;
            }
            if let Ok((pkgs, _)) = self.ts_packages(&st.def.id, false) {
                if pkgs.iter().any(|p| p.full_name.eq_ignore_ascii_case(&full)) {
                    out.push((st.def.id.clone(), st.def.name.clone()));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRegistry(Mutex<Option<String>>);

    impl SchemeRegistry for FakeRegistry {
        fn command(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
        fn register(&self, command: &str) -> std::io::Result<()> {
            *self.0.lock().unwrap() = Some(command.to_string());
            Ok(())
        }
        fn unregister(&self) -> std::io::Result<()> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[test]
    fn parses_install_links() {
        assert_eq!(
            parse_link("ror2mm://v1/install/thunderstore.io/Ann/Lantern_Tweaks/1.2.0/"),
            Some(InstallLink {
                owner: "Ann".into(),
                name: "Lantern_Tweaks".into(),
                version: Some("1.2.0".into())
            })
        );
        assert_eq!(
            parse_link("ror2mm://v1/install/thunderstore.io/Ann/Lantern_Tweaks")
                .unwrap()
                .version,
            None
        );
        assert!(parse_link("ror2mm://v1/install/evil.example/Ann/X/1.0.0/").is_none());
        assert!(parse_link("ror2mm://v1/install/thunderstore.io/../X/1.0.0/").is_none());
        assert!(parse_link("https://thunderstore.io/").is_none());
    }

    #[test]
    fn turning_on_and_off() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Session::open(Some(tmp.path().to_path_buf())).unwrap();
        let exe = Path::new(r"C:\Apps\UMM\umm-app.exe");
        let reg = FakeRegistry::default();
        reg.register(r#""C:\r2modman\r2modman.exe" "%1""#).unwrap();

        let h = s.link_handler(&reg, exe);
        assert!(!h.ours);
        assert!(h.other.unwrap().contains("r2modman"));
        // Off while another program has the links: theirs is left alone.
        s.set_link_handler(&reg, exe, false).unwrap();
        assert!(reg.command().unwrap().contains("r2modman"));

        assert!(s.set_link_handler(&reg, exe, true).unwrap().ours);
        assert_eq!(reg.command().unwrap(), r#""C:\Apps\UMM\umm-app.exe" "%1""#);

        // Off removes UMM's registration; nothing is restored.
        let h = s.set_link_handler(&reg, exe, false).unwrap();
        assert_eq!(
            h,
            LinkHandler {
                ours: false,
                other: None
            }
        );
        assert!(reg.command().is_none());
    }
}
