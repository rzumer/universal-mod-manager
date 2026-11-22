//! Install buttons on mod sites open links in a mod manager: Thunderstore's "Install with Mod
//! Manager" opens `ror2mm://v1/install/thunderstore.io/<owner>/<name>/<version>/`, and Nexus
//! Mods' "Mod Manager Download" (shown for games Vortex supports) opens `nxm://...`. One
//! switch handles them all: UMM registers itself for every scheme for the current user,
//! replacing any other handler (r2modman's, Vortex's). Turning it off removes UMM's
//! registrations.

use crate::{Result, Session};
use std::path::Path;

pub const LINK_SCHEME: &str = "ror2mm";
/// Nexus Mods' download links.
pub const NXM_SCHEME: &str = umm_nexus::link::LINK_SCHEME;
/// The link schemes UMM registers itself for, all at once.
pub const LINK_SCHEMES: &[&str] = &[LINK_SCHEME, NXM_SCHEME];

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

/// Where link handlers are registered: each URL scheme's open command.
pub trait SchemeRegistry: Send + Sync {
    fn command(&self, scheme: &str) -> Option<String>;
    fn register(&self, scheme: &str, command: &str) -> std::io::Result<()>;
    fn unregister(&self, scheme: &str) -> std::io::Result<()>;
}

/// `HKCU\Software\Classes\<scheme>`: per user, no administrator rights needed.
#[cfg(windows)]
pub struct WindowsRegistry;

#[cfg(windows)]
impl SchemeRegistry for WindowsRegistry {
    fn command(&self, scheme: &str) -> Option<String> {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(format!(r"Software\Classes\{scheme}\shell\open\command"))
            .ok()?
            .get_value::<String, _>("")
            .ok()
    }

    fn register(&self, scheme: &str, command: &str) -> std::io::Result<()> {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (root, _) = hkcu.create_subkey(format!(r"Software\Classes\{scheme}"))?;
        root.set_value("", &format!("URL:{scheme} Protocol"))?;
        root.set_value("URL Protocol", &"")?;
        let (cmd, _) = root.create_subkey(r"shell\open\command")?;
        cmd.set_value("", &command.to_string())
    }

    fn unregister(&self, scheme: &str) -> std::io::Result<()> {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        match RegKey::predef(HKEY_CURRENT_USER)
            .delete_subkey_all(format!(r"Software\Classes\{scheme}"))
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
    /// UMM handles every kind of link.
    pub ours: bool,
    /// Commands of other programs that handle some of them.
    pub others: Vec<String>,
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
    /// Whether UMM handles every kind of link in [`LINK_SCHEMES`], and which other programs
    /// handle some of them.
    pub fn link_handler(&self, reg: &dyn SchemeRegistry, exe: &Path) -> LinkHandler {
        let mut ours = true;
        let mut others: Vec<String> = Vec::new();
        for scheme in LINK_SCHEMES {
            match reg.command(scheme) {
                Some(c) if is_ours(&c, exe) => {}
                Some(c) => {
                    ours = false;
                    if !others.contains(&c) {
                        others.push(c);
                    }
                }
                None => ours = false,
            }
        }
        LinkHandler { ours, others }
    }

    /// Turns link handling on or off for this user, for every kind of link at once. Turning
    /// it on replaces whatever handled them; turning it off removes UMM's registrations (and
    /// leaves other programs' alone).
    pub fn set_link_handler(
        &self,
        reg: &dyn SchemeRegistry,
        exe: &Path,
        on: bool,
    ) -> Result<LinkHandler> {
        let io = |e: std::io::Error| {
            crate::CoreError::Invalid(format!("updating the link handler: {e}"))
        };
        for scheme in LINK_SCHEMES {
            if on {
                reg.register(scheme, &our_command(exe)).map_err(io)?;
            } else if reg.command(scheme).is_some_and(|c| is_ours(&c, exe)) {
                reg.unregister(scheme).map_err(io)?;
            }
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
    struct FakeRegistry(Mutex<std::collections::HashMap<String, String>>);

    impl SchemeRegistry for FakeRegistry {
        fn command(&self, scheme: &str) -> Option<String> {
            self.0.lock().unwrap().get(scheme).cloned()
        }
        fn register(&self, scheme: &str, command: &str) -> std::io::Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(scheme.to_string(), command.to_string());
            Ok(())
        }
        fn unregister(&self, scheme: &str) -> std::io::Result<()> {
            self.0.lock().unwrap().remove(scheme);
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
        reg.register("ror2mm", r#""C:\r2modman\r2modman.exe" "%1""#)
            .unwrap();
        reg.register("nxm", r#""C:\Vortex\Vortex.exe" "-d" "%1""#)
            .unwrap();

        let h = s.link_handler(&reg, exe);
        assert!(!h.ours);
        assert_eq!(h.others.len(), 2);
        assert!(h.others[0].contains("r2modman") && h.others[1].contains("Vortex"));
        // Off while other programs have the links: theirs are left alone.
        s.set_link_handler(&reg, exe, false).unwrap();
        assert!(reg.command("ror2mm").unwrap().contains("r2modman"));

        // One switch takes every kind of link.
        assert!(s.set_link_handler(&reg, exe, true).unwrap().ours);
        for scheme in LINK_SCHEMES {
            assert_eq!(
                reg.command(scheme).unwrap(),
                r#""C:\Apps\UMM\umm-app.exe" "%1""#
            );
        }

        // Taken back by one program: no longer all UMM's, and that program is named.
        reg.register("nxm", r#""C:\Vortex\Vortex.exe" "-d" "%1""#)
            .unwrap();
        let h = s.link_handler(&reg, exe);
        assert!(!h.ours && h.others[0].contains("Vortex"));

        // Off removes UMM's registrations only; nothing is restored.
        let h = s.set_link_handler(&reg, exe, false).unwrap();
        assert!(!h.ours);
        assert!(reg.command("ror2mm").is_none());
        assert!(reg.command("nxm").unwrap().contains("Vortex"));
    }
}
