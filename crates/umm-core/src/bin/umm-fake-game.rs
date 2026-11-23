//! Test helper that stands in for a game in UMM's launch tests. Not useful on its own.
//!
//! `--out <dir>`      where to write what it saw (required)
//! `--sleep-ms <n>`   stay running this long
//! `--child-ms <n>`   start a copy of itself that runs for `n` ms after this one exits,
//!                    like a launcher handing off to the real game
//!
//! Copied as `Patcher.exe`, it stands in for a patcher instead: it appends `+patched` to the
//! file `patch.txt` beside it names (see `main`) and prints so.
//!
//! It records its arguments (`args.txt`), the files in its working folder while running
//! (`<main|child>-seen.txt`) and `steam_appid.txt`'s contents (`appid.txt`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use std::{env, fs, thread};

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out);
        } else {
            out.push(
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

// The child is deliberately not waited on: it imitates a launcher handing off to the game.
#[allow(clippy::zombie_processes)]
fn main() {
    let me = env::current_exe().unwrap_or_default();
    let name = me
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if name.contains("patcher") {
        // `go`: Game.exe here; `esm`: Data/Game.esm here; `real:<dir>`: <dir>/Game.exe, like
        // a patcher that finds the game folder by itself; `add:<dir>`: it also adds
        // <dir>/Data/New.esp and changes <dir>/Data/meshes/rock.nif.
        let go = fs::read_to_string("patch.txt").unwrap_or_default();
        let go = go.trim();
        if let Some(d) = go.strip_prefix("add:") {
            let d = Path::new(d);
            fs::write(d.join("Data/New.esp"), "new").unwrap();
            fs::write(d.join("Data/meshes/rock.nif"), "changed").unwrap();
            let mut bytes = fs::read("Game.exe").unwrap();
            bytes.extend_from_slice(b"+patched");
            fs::write("Game.exe", bytes).unwrap();
            println!("Patched Game.exe, and more");
            return;
        }
        let target = match go {
            "go" => Some(PathBuf::from("Game.exe")),
            "esm" => Some(PathBuf::from("Data/Game.esm")),
            g => g
                .strip_prefix("real:")
                .map(|d| Path::new(d).join("Game.exe")),
        };
        match target {
            Some(t) => {
                let mut bytes = fs::read(&t).expect("the file to patch");
                bytes.extend_from_slice(b"+patched");
                fs::write(&t, bytes).unwrap();
                println!("Patched {}", t.display());
            }
            None => println!("Invalid executable"),
        }
        return;
    }
    let args: Vec<String> = env::args().collect();
    let (mut out, mut sleep_ms, mut child_ms, mut as_child) = (None, 0u64, None, false);
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                out = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--sleep-ms" => {
                sleep_ms = args[i + 1].parse().unwrap();
                i += 1;
            }
            "--child-ms" => {
                child_ms = Some(args[i + 1].clone());
                i += 1;
            }
            "--as-child" => as_child = true,
            _ => {}
        }
        i += 1;
    }
    let out = out.expect("--out is required");
    fs::create_dir_all(&out).unwrap();
    let tag = if as_child { "child" } else { "main" };
    if !as_child {
        fs::write(out.join("args.txt"), args[1..].join("\n")).unwrap();
        if let Ok(id) = fs::read_to_string("steam_appid.txt") {
            fs::write(out.join("appid.txt"), id).unwrap();
        }
    }
    thread::sleep(Duration::from_millis(sleep_ms));
    let cwd = env::current_dir().unwrap();
    let mut seen = Vec::new();
    walk(&cwd, &cwd, &mut seen);
    seen.sort();
    fs::write(out.join(format!("{tag}-seen.txt")), seen.join("\n")).unwrap();
    if let Some(ms) = child_ms {
        Command::new(env::current_exe().unwrap())
            .args(["--as-child", "--sleep-ms", &ms, "--out"])
            .arg(&out)
            .spawn()
            .unwrap();
    }
    if as_child {
        fs::write(out.join("child-done.txt"), "done").unwrap();
    }
}
