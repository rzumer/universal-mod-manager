use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use umm_core::{
    DeployOp, Faults, ImportOptions, LaunchEvent, Level, RevertReport, Session, DEFAULT_PROFILE,
};

fn launch(s: &Session, game: &str, profile: &str) -> Result<()> {
    use std::cell::Cell;
    use std::sync::atomic::Ordering;
    // First Ctrl+C: keep waiting so the revert still happens. Second: stop waiting and leave
    // the mods deployed; `umm recover` (or the next launch) reverts after the game exits.
    let interrupts = umm_core::catch_interrupts();
    let warned = Cell::new(false);
    let stop = || {
        let n = interrupts.load(Ordering::SeqCst);
        if n == 1 && !warned.replace(true) {
            eprintln!(
                "\nThe game is still running. UMM will restore the game folder when it exits.\n\
                 Press Ctrl+C again to stop waiting and leave the mods in place \
                 (then run `umm recover` after closing the game)."
            );
        }
        n >= 2
    };
    let outcome = s.launch(game, profile, &stop, &mut |e| match e {
        LaunchEvent::Recovered(r) => {
            println!("reverted a deployment left over from an earlier session");
            print_report(&r);
        }
        LaunchEvent::Deployed {
            game_files,
            runtime,
        } => {
            println!("deployed {game_files} file(s) to the game folder");
            if let Some(r) = runtime {
                println!(
                    "runtime folder: {} placed, {} unchanged, {} removed",
                    r.placed, r.unchanged, r.removed
                );
                for k in r.kept_edited {
                    println!("  kept your edited {k}");
                }
            }
        }
        LaunchEvent::Started { pid } => {
            println!("started (pid {pid}); waiting for the game to exit")
        }
        LaunchEvent::WaitingForGame => {
            println!("launched through Steam; waiting for the game to start")
        }
        LaunchEvent::GameDetected => println!("game is running"),
        LaunchEvent::Exited { code } => match code {
            Some(c) => println!("game exited (code {c})"),
            None => println!("game exited"),
        },
        LaunchEvent::Reverted(r) => print_report(&r),
    })?;
    if outcome.abandoned {
        println!(
            "stopped waiting; mods are still deployed. Run `umm recover` after the game exits."
        );
    } else if !outcome.game_appeared {
        println!("the game never appeared; the game folder was restored");
    } else {
        println!(
            "played for {}s; game folder restored",
            outcome.played.as_secs()
        );
    }
    Ok(())
}

#[derive(Parser)]
#[command(
    name = "umm",
    version,
    about = "Universal Mod Manager: deploy mods into game folders and put them back"
)]
struct Cli {
    /// Data folder (default: UMM_DATA, or %APPDATA%\umm)
    #[arg(long, global = true)]
    data: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List game definitions and whether each game was found
    Games,
    /// Manage a game's settings and data
    #[command(subcommand)]
    Game(GameCmd),
    /// Import a mod (folder or .zip) into the store and add it to a profile
    Import {
        game: String,
        path: PathBuf,
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long = "mod-version")]
        version: Option<String>,
        /// Keep a single top-level folder instead of unwrapping it
        #[arg(long)]
        no_unwrap: bool,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    /// List stored mods
    Mods { game: String },
    /// Delete a mod from the store and all profiles
    RemoveMod {
        game: String,
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Show or edit a profile
    #[command(subcommand)]
    Profile(ProfileCmd),
    /// Show what a deploy would do, without changing anything
    Plan {
        game: String,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    /// Place the profile's mods in the game folder (run `umm revert` when done playing)
    Deploy {
        game: String,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    /// Deploy, start the game, wait for it to exit, then revert
    Launch {
        game: String,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    /// Put the game folder back exactly as it was
    Revert {
        game: String,
        /// If the game folder is gone, drop the deployment record instead of failing
        #[arg(long)]
        discard_if_missing: bool,
    },
    /// Revert every interrupted deployment (run at startup)
    Recover {
        #[arg(long)]
        discard_if_missing: bool,
    },
    /// Check a game's install, deployment state and stored mods
    Doctor { game: String },
    /// Add game definitions for games installed through Steam
    #[command(subcommand)]
    Steam(SteamCmd),
}

#[derive(Subcommand)]
enum SteamCmd {
    /// List installed Steam games and whether UMM already has each one
    List,
    /// Draft a game definition for installed Steam games (by app id, or --all)
    Add {
        app_ids: Vec<u32>,
        /// Every installed game UMM does not have yet
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
enum GameCmd {
    /// Set the install folder manually (overrides detection)
    SetPath { game: String, path: PathBuf },
    /// Remove a game from UMM: its definition, mods, profiles and settings
    #[command(name = "delete", alias = "delete-data")]
    DeleteData {
        game: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ProfileCmd {
    Show {
        game: String,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    Enable {
        game: String,
        id: String,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    Disable {
        game: String,
        id: String,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    /// Move a mod to a 1-based position; later positions win conflicts
    Move {
        game: String,
        id: String,
        position: usize,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn print_report(r: &RevertReport) {
    println!(
        "restored {} original file(s), removed {} mod file(s)",
        r.restored, r.removed
    );
    for p in &r.quarantined {
        println!("  quarantined (changed while deployed): {p}");
    }
    for p in &r.kept_game_versions {
        println!("  kept the game's newer version: {p}");
    }
    for p in &r.dirs_left {
        println!("  left folder with new files in it: {p}");
    }
}

fn run(cli: Cli) -> Result<()> {
    let s = Session::open(cli.data).context("opening the data folder")?;
    match cli.cmd {
        Cmd::Games => {
            let (_, errors) = s.defs();
            for e in errors {
                eprintln!("warning: {e}");
            }
            let games = s.status()?;
            if games.is_empty() {
                println!(
                    "no game definitions in {}",
                    s.data().gamedefs_dir().display()
                );
            }
            for g in games {
                let place = match (&g.install, g.found_via) {
                    (Some(p), Some(via)) => format!("{} ({via})", p.display()),
                    _ => "not found".into(),
                };
                let state = if g.deployed { " [DEPLOYED]" } else { "" };
                println!(
                    "{:<24} {:<28} {} mod(s)  {place}{state}",
                    g.def.id, g.def.name, g.mod_count
                );
            }
        }
        Cmd::Game(GameCmd::SetPath { game, path }) => {
            s.set_install_path(&game, &path)?;
            println!("{game}: install folder set to {}", path.display());
        }
        Cmd::Game(GameCmd::DeleteData { game, yes }) => {
            if !yes {
                bail!("this removes `{game}` from UMM and permanently deletes its definition and every mod, profile and setting stored for it; rerun with --yes");
            }
            s.delete_game_data(&game)?;
            println!("removed {game} and all its UMM data (the game's own folder is untouched)");
        }
        Cmd::Import {
            game,
            path,
            id,
            name,
            version,
            no_unwrap,
            profile,
        } => {
            let opts = ImportOptions {
                id,
                name,
                version,
                unwrap: !no_unwrap,
                ..Default::default()
            };
            let m = s.import(&game, &path, opts, &profile)?;
            println!(
                "imported `{}` ({} file(s)) and added it to profile `{profile}`",
                m.id,
                m.files.len()
            );
        }
        Cmd::Mods { game } => {
            for m in s.mods(&game)? {
                let v = m.version.as_deref().unwrap_or("-");
                println!(
                    "{:<32} {:<10} {} file(s)  {}",
                    m.id,
                    v,
                    m.files.len(),
                    m.name
                );
            }
        }
        Cmd::RemoveMod { game, id, yes } => {
            if !yes {
                bail!("this permanently deletes `{id}` from the store; rerun with --yes");
            }
            s.remove_mod(&game, &id)?;
            println!("removed `{id}`");
        }
        Cmd::Profile(cmd) => profile_cmd(&s, cmd)?,
        Cmd::Plan { game, profile } => {
            let plan = s.plan(&game, &profile)?;
            println!(
                "{} file(s) to deploy for profile `{profile}`",
                plan.files.len()
            );
            for f in &plan.files {
                let w = if f.writable { " (copy)" } else { "" };
                println!("  {:<60} <- {}{w}", f.target.to_string(), f.owner);
            }
            if !plan.conflicts.is_empty() {
                println!("conflicts (last wins):");
                for c in &plan.conflicts {
                    println!(
                        "  {}: {} over {}",
                        c.target,
                        c.winner,
                        c.overridden.join(", ")
                    );
                }
            }
            if !plan.skipped.is_empty() {
                println!("skipped:");
                for k in &plan.skipped {
                    println!("  {}:{} ({})", k.owner, k.file, k.reason);
                }
            }
        }
        Cmd::Deploy { game, profile } => {
            let j = s.deploy(&game, &profile, &mut Faults::none())?;
            let files = j
                .ops
                .iter()
                .filter(|o| !matches!(o, DeployOp::Mkdir { .. }))
                .count();
            println!("deployed {files} file(s) to {}", j.game_root.display());
            println!("run `umm revert {game}` when you are done playing");
        }
        Cmd::Launch { game, profile } => launch(&s, &game, &profile)?,
        Cmd::Revert {
            game,
            discard_if_missing,
        } => match s.revert(&game, discard_if_missing, &mut Faults::none())? {
            None => println!("{game}: nothing deployed"),
            Some(r) => print_report(&r),
        },
        Cmd::Recover { discard_if_missing } => {
            let results = s.recover_all(discard_if_missing);
            if results.is_empty() {
                println!("nothing to recover");
            }
            let mut failed = false;
            for (id, r) in results {
                match r {
                    Ok(Some(r)) => {
                        println!("{id}: recovered");
                        print_report(&r);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        failed = true;
                        eprintln!("{id}: {e}");
                    }
                }
            }
            if failed {
                bail!("some games could not be recovered");
            }
        }
        Cmd::Steam(SteamCmd::List) => {
            let games = s.steam_games();
            if games.is_empty() {
                println!("no installed Steam games found");
            }
            for g in games {
                let state = match &g.defined_as {
                    Some(id) => format!("added as `{id}`"),
                    None => "-".into(),
                };
                println!(
                    "{:>8}  {:<40} {:<16} {state}",
                    g.app.app_id,
                    g.app.name,
                    g.engine.label()
                );
            }
        }
        Cmd::Steam(SteamCmd::Add { app_ids, all }) => {
            let ids: Vec<u32> = if all {
                s.steam_games()
                    .into_iter()
                    .filter(|g| g.defined_as.is_none())
                    .map(|g| g.app.app_id)
                    .collect()
            } else {
                app_ids
            };
            if ids.is_empty() {
                bail!("give one or more app ids (see `umm steam list`), or --all");
            }
            let mut failed = false;
            for id in ids {
                match s.add_steam_game(id) {
                    Ok(d) => {
                        println!(
                            "added `{}` ({}) -> {}",
                            d.def.id,
                            d.engine.label(),
                            s.data()
                                .gamedefs_dir()
                                .join(format!("{}.toml", d.def.id))
                                .display()
                        );
                        for n in d.notes {
                            println!("  note: {n}");
                        }
                    }
                    Err(e) => {
                        failed = true;
                        eprintln!("{id}: {e}");
                    }
                }
            }
            if failed {
                bail!("some games were not added");
            }
        }
        Cmd::Doctor { game } => {
            let mut worst = Level::Ok;
            for f in s.doctor(&game)? {
                let tag = match f.level {
                    Level::Ok => "ok   ",
                    Level::Info => "info ",
                    Level::Warn => "WARN ",
                    Level::Error => "ERROR",
                };
                println!("{tag} {}", f.message);
                worst = worst.max(f.level);
            }
            if worst == Level::Error {
                std::process::exit(2);
            }
        }
    }
    Ok(())
}

fn profile_cmd(s: &Session, cmd: ProfileCmd) -> Result<()> {
    match cmd {
        ProfileCmd::Show { game, profile } => {
            let p = s.profile(&game, &profile)?;
            println!("profile `{}` (later entries win conflicts)", p.name);
            for (i, m) in p.mods.iter().enumerate() {
                println!(
                    "  {:>3}. [{}] {}",
                    i + 1,
                    if m.enabled { "x" } else { " " },
                    m.id
                );
            }
        }
        ProfileCmd::Enable { game, id, profile } => set_enabled(s, &game, &id, &profile, true)?,
        ProfileCmd::Disable { game, id, profile } => set_enabled(s, &game, &id, &profile, false)?,
        ProfileCmd::Move {
            game,
            id,
            position,
            profile,
        } => {
            let mut p = s.profile(&game, &profile)?;
            let Some(i) = p.position(&id) else {
                bail!("`{id}` is not in profile `{profile}`")
            };
            let entry = p.mods.remove(i);
            let to = position.clamp(1, p.mods.len() + 1) - 1;
            p.mods.insert(to, entry);
            s.save_profile(&game, &p)?;
            println!("moved `{id}` to position {}", to + 1);
        }
    }
    Ok(())
}

fn set_enabled(s: &Session, game: &str, id: &str, profile: &str, on: bool) -> Result<()> {
    if !s.mods(game)?.iter().any(|m| m.id == id) {
        bail!("mod `{id}` is not in the store");
    }
    {
        let mut p = s.profile(game, profile)?;
        match p.position(id) {
            Some(i) => p.mods[i].enabled = on,
            None => p.mods.push(umm_core::ProfileEntry {
                id: id.to_string(),
                enabled: on,
            }),
        }
        s.save_profile(game, &p)?;
        println!(
            "{} `{id}` in profile `{profile}`",
            if on { "enabled" } else { "disabled" }
        );
    }
    Ok(())
}
