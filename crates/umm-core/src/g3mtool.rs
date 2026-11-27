//! G3MTool (github.com/y114git/G3MTool, GPL-3.0): a command-line tool for GameMaker data
//! files, which merges several mods' changes to the same data file resource by resource. The
//! G3M and Deltamod managers use it the same way. UMM downloads one pinned release the first
//! time it needs it, checks it against the checksum below, and keeps it in its `tools`
//! folder; it runs as a separate program.

use crate::{CoreError, Result, Session};
use sha2::Digest;
use std::fs;
use std::path::{Path, PathBuf};
use umm_store::fsutil::IoCtx;

/// The release UMM uses, and its command-line build's version.
const RELEASE: &str = "2026.11.25";
const VERSION: &str = "1.3.7";

/// This platform's archive of the command-line build, and its SHA-256.
#[cfg(all(windows, target_arch = "x86_64"))]
const ASSET: Option<(&str, &str)> = Some((
    "G3MTool-CLI-Windows-x64-1.3.7.zip",
    "a02a75d29be8a07d78f3b398b082f649eba77865bb5e290dd9fcc518c6003b57",
));
#[cfg(all(windows, target_arch = "aarch64"))]
const ASSET: Option<(&str, &str)> = Some((
    "G3MTool-CLI-Windows-arm64-1.3.7.zip",
    "258c998e3f0c9e6849fcdf0b69622b1e638ed6c22a0b5c56677076cb895c247a",
));
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const ASSET: Option<(&str, &str)> = Some((
    "G3MTool-CLI-Linux-x64-1.3.7.zip",
    "517e889e54c8f971f5149b341f0e72cb13d3de70921f2d02f23ae03744aac5dc",
));
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const ASSET: Option<(&str, &str)> = Some((
    "G3MTool-CLI-Linux-arm64-1.3.7.zip",
    "c5d4091c43c4f4be4a89e79004413603096d2731e2972582be8177ade2a3c136",
));
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const ASSET: Option<(&str, &str)> = Some((
    "G3MTool-CLI-macOS-arm64-1.3.7.zip",
    "4cd232c38c6604d647c548381b07dbf3b697f6a377f885c98177476bd5bfde4b",
));
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const ASSET: Option<(&str, &str)> = Some((
    "G3MTool-CLI-macOS-x64-1.3.7.zip",
    "e72f398a0109468a86134b1dd7d40ca992ad21f98307d231dc63720d8bc3132c",
));
#[cfg(not(any(
    all(windows, any(target_arch = "x86_64", target_arch = "aarch64")),
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "macos",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
)))]
const ASSET: Option<(&str, &str)> = None;

const EXE: &str = if cfg!(windows) {
    "G3MTool.exe"
} else {
    "G3MTool"
};

/// Where UMM keeps this version of G3MTool.
fn tool_dir(data_root: &Path) -> PathBuf {
    data_root.join("tools").join("g3mtool").join(VERSION)
}

/// The program in an unpacked release, wherever the archive put it.
fn find_exe(dir: &Path) -> Option<PathBuf> {
    walkdir::WalkDir::new(dir)
        .max_depth(3)
        .into_iter()
        .flatten()
        .find(|e| e.file_type().is_file() && e.file_name().to_string_lossy() == EXE)
        .map(|e| e.into_path())
}

fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = fs::File::open(path).ctx(|| format!("opening {}", path.display()))?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f
            .read(&mut buf)
            .ctx(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    }))
}

/// The last lines of a program's output, for an error message.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    all[all.len().saturating_sub(lines)..].join(" / ")
}

impl Session {
    /// G3MTool, downloaded the first time and checked against its pinned checksum.
    pub(crate) fn g3mtool(&self, progress: &mut dyn FnMut(&str)) -> Result<PathBuf> {
        let dir = tool_dir(self.data.root());
        if let Some(exe) = find_exe(&dir) {
            return Ok(exe);
        }
        let (asset, sha) = ASSET.ok_or_else(|| {
            CoreError::Invalid("G3MTool has no build for this kind of computer".into())
        })?;
        let parent = dir.parent().expect("under tools");
        fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
        let zip = parent.join(format!("{asset}.part"));
        let url = format!("https://github.com/y114git/G3MTool/releases/download/{RELEASE}/{asset}");
        progress("Downloading G3MTool, which merges mods' changes to GameMaker data files");
        self.http()
            .download(&url, &zip, &mut |_, _| {})
            .map_err(|e| {
                CoreError::Invalid(format!("could not download G3MTool ({url}): {}", e.message))
            })?;
        let got = sha256_file(&zip)?;
        if got != sha {
            let _ = fs::remove_file(&zip);
            return Err(CoreError::Invalid(format!(
                "the G3MTool download does not match its checksum (got {got}, expected {sha}); try again"
            )));
        }
        let staging = parent.join(format!("{VERSION}.part"));
        let _ = fs::remove_dir_all(&staging);
        let unpacked = umm_store::extract_zip(&zip, &staging);
        let _ = fs::remove_file(&zip);
        unpacked?;
        let Some(exe) = find_exe(&staging) else {
            let _ = fs::remove_dir_all(&staging);
            return Err(CoreError::Invalid(format!(
                "the G3MTool download has no {EXE}"
            )));
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&exe, fs::Permissions::from_mode(0o755));
        }
        #[cfg(not(unix))]
        let _ = exe;
        let _ = fs::remove_dir_all(&dir);
        fs::rename(&staging, &dir).ctx(|| format!("moving G3MTool into {}", dir.display()))?;
        find_exe(&dir).ok_or_else(|| CoreError::Invalid(format!("G3MTool's {EXE} went missing")))
    }

    /// Merges `inputs` (xdelta patches or data files, each made from `original`; later ones
    /// win where they change the same thing) into the data file `out`, with G3MTool.
    /// Returns what it printed.
    pub(crate) fn g3m_merge(
        &self,
        original: &Path,
        inputs: &[PathBuf],
        out: &Path,
        progress: &mut dyn FnMut(&str),
    ) -> Result<String> {
        let mut args: Vec<std::ffi::OsString> =
            vec!["patch".into(), "merge".into(), original.into()];
        args.extend(inputs.iter().map(|i| i.as_os_str().to_owned()));
        args.extend([
            "--properties".into(),
            "--apply".into(),
            out.as_os_str().to_owned(),
        ]);
        self.run_g3mtool(&args, out, "merge them", progress)
    }

    /// Applies a G3M patch (`.g3mpatch`) or an UndertaleModTool script (`.csx`) to the data
    /// file `data`, writing the result to `out`, with G3MTool. Returns what it printed.
    pub(crate) fn g3m_apply(
        &self,
        data: &Path,
        patch: &Path,
        out: &Path,
        progress: &mut dyn FnMut(&str),
    ) -> Result<String> {
        let args: Vec<std::ffi::OsString> = vec![
            "patch".into(),
            "apply".into(),
            data.into(),
            patch.into(),
            out.into(),
        ];
        self.run_g3mtool(&args, out, "apply it", progress)
    }

    /// Runs G3MTool with `args`, which write the data file `out`; `what` names the job for
    /// an error ("merge them").
    fn run_g3mtool(
        &self,
        args: &[std::ffi::OsString],
        out: &Path,
        what: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<String> {
        let exe = self.g3mtool(progress)?;
        if let Some(p) = out.parent() {
            fs::create_dir_all(p).ctx(|| format!("creating {}", p.display()))?;
        }
        let _ = fs::remove_file(out);
        let mut cmd = std::process::Command::new(&exe);
        cmd.args(args)
            .current_dir(exe.parent().unwrap_or(Path::new(".")))
            .stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window from the app.
            cmd.creation_flags(0x0800_0000);
        }
        let output = cmd
            .output()
            .map_err(|e| CoreError::Invalid(format!("could not run G3MTool: {e}")))?;
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() || !out.is_file() {
            let why = if text.contains("normally this indicates that the source file is incorrect")
            {
                "a patch was not made for the game's own file".to_string()
            } else {
                tail(&text, 3)
            };
            return Err(CoreError::Invalid(format!(
                "G3MTool could not {what}: {why}"
            )));
        }
        Ok(text)
    }
}
