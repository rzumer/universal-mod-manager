//! Steam at launch: finding and starting it, and `steam_appid.txt` for games started
//! directly.

use super::*;

pub(super) fn steam_appid_file(dir: &Path, app_id: u32) -> Result<PlannedFile> {
    let p = dir.join("steam_appid.txt");
    write_atomic(&p, app_id.to_string().as_bytes()).ctx(|| format!("writing {}", p.display()))?;
    let size = std::fs::metadata(&p)
        .ctx(|| format!("reading {}", p.display()))?
        .len();
    Ok(PlannedFile {
        target: RelPath::parse("steam_appid.txt").expect("valid path"),
        hash: hash_file(&p).ctx(|| format!("hashing {}", p.display()))?,
        size,
        source: p,
        owner: "umm".into(),
        writable: true,
    })
}

/// How long to wait for Steam to start and sign in before launching a game outside it.
pub(super) const STEAM_START_WAIT: std::time::Duration = std::time::Duration::from_secs(45);
/// Steam's process is up some time before it has signed in and can answer games.
pub(super) const STEAM_SETTLE: std::time::Duration = std::time::Duration::from_secs(8);

/// Starts Steam (minimised to the tray) if it is not running, and waits for it. Returns
/// whether it had to be started.
pub(super) fn ensure_steam(steam: Option<SteamCmd>) -> bool {
    let exe = |n: &str| {
        if cfg!(windows) {
            format!("{n}.exe")
        } else {
            n.to_string()
        }
    };
    if umm_launch::is_any_running(&[exe("steam")]).unwrap_or(true) {
        return false;
    }
    let Some(steam) = steam else { return false };
    if std::process::Command::new(&steam.program)
        .args(&steam.args)
        .arg("-silent")
        .spawn()
        .is_err()
    {
        return false;
    }
    let start = std::time::Instant::now();
    while start.elapsed() < STEAM_START_WAIT {
        std::thread::sleep(std::time::Duration::from_millis(500));
        // Its web helper starts once Steam is past its own updates and signing in.
        if umm_launch::is_any_running(&[exe("steamwebhelper")]).unwrap_or(false) {
            std::thread::sleep(STEAM_SETTLE);
            break;
        }
    }
    true
}

/// How to start Steam: its program, and the arguments before Steam's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SteamCmd {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl From<PathBuf> for SteamCmd {
    fn from(program: PathBuf) -> Self {
        SteamCmd {
            program,
            args: Vec::new(),
        }
    }
}

/// Steam's exe on Windows; elsewhere `steam` on the PATH, else Steam's Flatpak.
pub(super) fn find_steam() -> Option<SteamCmd> {
    if cfg!(windows) {
        return steam_roots()
            .into_iter()
            .map(|r| r.join("steam.exe"))
            .find(|p| p.is_file())
            .map(SteamCmd::from);
    }
    let on_path = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("steam").is_file()));
    let flatpak = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .is_some_and(|h| h.join(".var/app/com.valvesoftware.Steam").is_dir())
    };
    Some(if !on_path && flatpak() {
        SteamCmd {
            program: "flatpak".into(),
            args: vec!["run".into(), "com.valvesoftware.Steam".into()],
        }
    } else {
        PathBuf::from("steam").into()
    })
}
