//! Install buttons on mod sites open links in a mod manager: Thunderstore's "Install with Mod
//! Manager" opens `ror2mm://v1/install/thunderstore.io/<owner>/<name>/<version>/`, and Nexus
//! Mods' "Mod Manager Download" (shown for games Vortex supports) opens `nxm://...`. One
//! switch handles them all: UMM registers itself for every scheme for the current user,
//! replacing any other handler (r2modman's, Vortex's). Turning it off removes UMM's
//! registrations.

use crate::{Result, Session};
use std::path::{Path, PathBuf};

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

/// Linux and the like (freedesktop.org): a scheme's handler is the desktop entry that
/// `mimeapps.list` makes the default for `x-scheme-handler/<scheme>`. UMM writes its own
/// entry, `umm-<scheme>.desktop`, and makes it the user's default; [`Self::command`] is the
/// default entry's `Exec` line.
pub struct XdgRegistry {
    /// `$XDG_CONFIG_HOME`, where the user's `mimeapps.list` is.
    pub config_home: PathBuf,
    /// `$XDG_DATA_HOME`, whose `applications` holds the user's desktop entries.
    pub data_home: PathBuf,
    /// `$XDG_CONFIG_DIRS`, the system's `mimeapps.list` folders.
    pub config_dirs: Vec<PathBuf>,
    /// `$XDG_DATA_DIRS`, whose `applications` hold the system's desktop entries.
    pub data_dirs: Vec<PathBuf>,
    /// `$XDG_CURRENT_DESKTOP`, lower-cased: each has its own `<desktop>-mimeapps.list` first.
    pub desktops: Vec<String>,
}

const DEFAULTS: &str = "[Default Applications]";

impl XdgRegistry {
    /// The folders from the environment, with the specification's defaults.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn from_env() -> Option<Self> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let var = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
        let dir = |n: &str, d: PathBuf| var(n).map(PathBuf::from).unwrap_or(d);
        let dirs = |n: &str, d: &str| -> Vec<PathBuf> {
            var(n)
                .unwrap_or_else(|| d.to_string())
                .split(':')
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .collect()
        };
        Some(XdgRegistry {
            config_home: dir("XDG_CONFIG_HOME", home.join(".config")),
            data_home: dir("XDG_DATA_HOME", home.join(".local/share")),
            config_dirs: dirs("XDG_CONFIG_DIRS", "/etc/xdg"),
            data_dirs: dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share"),
            desktops: var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .filter(|d| !d.is_empty())
                .map(str::to_lowercase)
                .collect(),
        })
    }

    fn mime(scheme: &str) -> String {
        format!("x-scheme-handler/{scheme}")
    }

    fn entry_name(scheme: &str) -> String {
        format!("umm-{scheme}.desktop")
    }

    fn user_list(&self) -> PathBuf {
        self.config_home.join("mimeapps.list")
    }

    /// `mimeapps.list` files, the one that wins first.
    fn lists(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let config = std::iter::once(&self.config_home).chain(&self.config_dirs);
        let apps = std::iter::once(&self.data_home)
            .chain(&self.data_dirs)
            .map(|d| d.join("applications"));
        for dir in config.cloned().chain(apps) {
            for d in &self.desktops {
                out.push(dir.join(format!("{d}-mimeapps.list")));
            }
            out.push(dir.join("mimeapps.list"));
        }
        out
    }

    /// The desktop entry a list makes the default for `mime`: the first one listed.
    fn default_in(text: &str, mime: &str) -> Option<String> {
        let mut in_defaults = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_defaults = line == DEFAULTS;
            } else if in_defaults {
                if let Some((k, v)) = line.split_once('=') {
                    if k.trim() == mime {
                        return v
                            .split(';')
                            .map(str::trim)
                            .find(|e| !e.is_empty())
                            .map(String::from);
                    }
                }
            }
        }
        None
    }

    /// A desktop entry's `Exec` line, found where entries are kept (`a-b.desktop` may be
    /// `a/b.desktop`).
    fn exec_of(&self, entry: &str) -> Option<String> {
        let dirs = std::iter::once(&self.data_home)
            .chain(&self.data_dirs)
            .map(|d| d.join("applications"));
        let nested = entry.replace('-', "/");
        for dir in dirs {
            for name in [entry, nested.as_str()] {
                let Ok(text) = std::fs::read_to_string(dir.join(name)) else {
                    continue;
                };
                let mut in_entry = false;
                for line in text.lines() {
                    let line = line.trim();
                    if line.starts_with('[') {
                        in_entry = line == "[Desktop Entry]";
                    } else if in_entry {
                        if let Some(v) = line.strip_prefix("Exec=") {
                            return Some(v.trim().to_string());
                        }
                    }
                }
            }
        }
        None
    }

    /// Sets (`Some`) or removes (`None`) the user's default for `mime`, leaving the rest of
    /// their list as it was.
    fn set_default(&self, mime: &str, entry: Option<&str>) -> std::io::Result<()> {
        let path = self.user_list();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut out: Vec<String> = Vec::new();
        let mut in_defaults = false;
        let mut done = false;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                if in_defaults && !done {
                    if let Some(e) = entry {
                        out.push(format!("{mime}={e};"));
                    }
                    done = true;
                }
                in_defaults = t == DEFAULTS;
            } else if in_defaults && t.split_once('=').is_some_and(|(k, _)| k.trim() == mime) {
                if let (Some(e), false) = (entry, done) {
                    out.push(format!("{mime}={e};"));
                }
                done = true;
                continue;
            }
            out.push(line.to_string());
        }
        if !done {
            if let Some(e) = entry {
                if !in_defaults {
                    if out.last().is_some_and(|l| !l.trim().is_empty()) {
                        out.push(String::new());
                    }
                    out.push(DEFAULTS.to_string());
                }
                out.push(format!("{mime}={e};"));
            }
        }
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut new = out.join("\n");
        new.push('\n');
        std::fs::write(&path, new)
    }
}

/// A Windows-style open command (`"<program>" "%1"`) as a desktop entry's `Exec` line: the
/// program quoted as the specification asks, and `%u` for the link. Quoted only when the path
/// needs it: `xdg-open` without a desktop environment reads `Exec` word by word.
fn exec_line(command: &str) -> String {
    let program = command
        .trim()
        .strip_suffix("\"%1\"")
        .unwrap_or(command)
        .trim()
        .trim_matches('"');
    let reserved = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\''
                    | '\\'
                    | '>'
                    | '<'
                    | '~'
                    | '|'
                    | '&'
                    | ';'
                    | '$'
                    | '*'
                    | '?'
                    | '#'
                    | '('
                    | ')'
                    | '`'
                    | '%'
                    | '='
            )
    };
    if !program.chars().any(reserved) {
        return format!("{program} %u");
    }
    let mut quoted = String::from("\"");
    for c in program.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
        // Not a field code (`%u`): a percent sign of the path.
        if c == '%' {
            quoted.push('%');
        }
    }
    quoted.push('"');
    // Backslashes are escaped once more as a desktop entry's string value.
    format!("{} %u", quoted.replace('\\', "\\\\"))
}

impl SchemeRegistry for XdgRegistry {
    fn command(&self, scheme: &str) -> Option<String> {
        let mime = Self::mime(scheme);
        let entry = self
            .lists()
            .iter()
            .filter_map(|l| std::fs::read_to_string(l).ok())
            .find_map(|t| Self::default_in(&t, &mime))?;
        self.exec_of(&entry)
    }

    fn register(&self, scheme: &str, command: &str) -> std::io::Result<()> {
        let apps = self.data_home.join("applications");
        std::fs::create_dir_all(&apps)?;
        let entry = Self::entry_name(scheme);
        std::fs::write(
            apps.join(&entry),
            format!(
                "[Desktop Entry]\nType=Application\nName=Universal Mod Manager ({scheme} links)\n\
                 Exec={}\nMimeType={};\nNoDisplay=true\nTerminal=false\n",
                exec_line(command),
                Self::mime(scheme)
            ),
        )?;
        self.set_default(&Self::mime(scheme), Some(&entry))
    }

    fn unregister(&self, scheme: &str) -> std::io::Result<()> {
        let entry = Self::entry_name(scheme);
        let mime = Self::mime(scheme);
        let list = std::fs::read_to_string(self.user_list()).unwrap_or_default();
        if Self::default_in(&list, &mime).as_deref() == Some(entry.as_str()) {
            self.set_default(&mime, None)?;
        }
        match std::fs::remove_file(self.data_home.join("applications").join(&entry)) {
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
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        XdgRegistry::from_env().map(|r| Box::new(r) as Box<dyn SchemeRegistry>)
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
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
    let exe = exe.display().to_string().to_lowercase();
    command.to_lowercase().contains(&exe) || unescape_exec(command).to_lowercase().contains(&exe)
}

/// A desktop entry's `Exec` value as the program sees it: backslashes unescaped as a string
/// value, then the escapes inside its quotes (`\"`, `` \` ``, `\$`, `\\`).
fn unescape_exec(exec: &str) -> String {
    let value = exec.replace("\\\\", "\\");
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some('"' | '`' | '$' | '\\')) => out.push(chars.next().unwrap_or(c)),
            _ => out.push(c),
        }
    }
    out
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

    #[test]
    fn exec_lines_quote_only_paths_that_need_it() {
        assert_eq!(
            exec_line(r#""/home/me/Apps/UMM.AppImage" "%1""#),
            "/home/me/Apps/UMM.AppImage %u"
        );
        assert_eq!(
            exec_line(r#""/opt/My UMM/50%/umm-app" "%1""#),
            r#""/opt/My UMM/50%%/umm-app" %u"#
        );
        assert!(is_ours(
            r#""/opt/a\\\\b \\$x/umm-app" %u"#,
            Path::new(r"/opt/a\b $x/umm-app")
        ));
    }

    fn xdg(root: &Path) -> XdgRegistry {
        XdgRegistry {
            config_home: root.join("home/.config"),
            data_home: root.join("home/.local/share"),
            config_dirs: vec![root.join("etc/xdg")],
            data_dirs: vec![root.join("usr/share")],
            desktops: vec!["kde".into()],
        }
    }

    #[test]
    fn on_linux_the_default_desktop_entry_handles_a_scheme() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let reg = xdg(root);
        let w = |p: &str, t: &str| {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        // r2modman, the system's default for its links; the user's own list has other things.
        w(
            "usr/share/applications/r2modman.desktop",
            "[Desktop Entry]\nName=r2modman\nExec=/opt/r2modman/r2modman %U\n\
             [Desktop Action New]\nExec=/opt/r2modman/r2modman --new\n",
        );
        w(
            "etc/xdg/mimeapps.list",
            "[Default Applications]\nx-scheme-handler/ror2mm=r2modman.desktop;\n",
        );
        let mine = "[Added Associations]\ntext/plain=gedit.desktop;\n\n\
                    [Default Applications]\nx-scheme-handler/https=firefox.desktop;\n";
        w("home/.config/mimeapps.list", mine);
        assert_eq!(
            reg.command("ror2mm").as_deref(),
            Some("/opt/r2modman/r2modman %U")
        );
        assert_eq!(reg.command("nxm"), None);

        let s = Session::open(Some(root.join("data"))).unwrap();
        let exe = Path::new("/home/me/Apps/UMM $1/umm-app");
        assert!(s.set_link_handler(&reg, exe, true).unwrap().ours);
        let entry =
            std::fs::read_to_string(root.join("home/.local/share/applications/umm-nxm.desktop"))
                .unwrap();
        assert!(
            entry.contains("Exec=\"/home/me/Apps/UMM \\\\$1/umm-app\" %u\n"),
            "{entry}"
        );
        assert!(entry.contains("MimeType=x-scheme-handler/nxm;\n"));
        let list = std::fs::read_to_string(root.join("home/.config/mimeapps.list")).unwrap();
        assert!(list.starts_with(mine), "{list}");
        assert!(list.contains("x-scheme-handler/ror2mm=umm-ror2mm.desktop;\n"));

        // A desktop's own list wins over the general one.
        w(
            "home/.config/kde-mimeapps.list",
            "[Default Applications]\nx-scheme-handler/nxm=r2modman.desktop\n",
        );
        let h = s.link_handler(&reg, exe);
        assert!(!h.ours && h.others == ["/opt/r2modman/r2modman %U"]);
        std::fs::remove_file(root.join("home/.config/kde-mimeapps.list")).unwrap();

        // Off: UMM's entries and defaults go; the system's default shows again.
        let h = s.set_link_handler(&reg, exe, false).unwrap();
        assert!(!h.ours);
        assert!(!root
            .join("home/.local/share/applications/umm-ror2mm.desktop")
            .exists());
        let list = std::fs::read_to_string(root.join("home/.config/mimeapps.list")).unwrap();
        assert!(!list.contains("umm-"), "{list}");
        assert!(list.contains("x-scheme-handler/https=firefox.desktop;"));
        assert_eq!(
            reg.command("ror2mm").as_deref(),
            Some("/opt/r2modman/r2modman %U")
        );
    }
}
