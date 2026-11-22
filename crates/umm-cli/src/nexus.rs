//! `umm nexus ...`: Nexus Mods from the command line.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;
use umm_core::{
    DownloadWatch, ExpectedFile, NexusFile, NexusInstallReport, NexusSearch, NexusSort,
    NexusUpdateState, Session, DEFAULT_PROFILE,
};

#[derive(Subcommand)]
pub enum NexusCmd {
    /// Show or set the game's Nexus Mods game (its name in nexusmods.com/<name>/mods/...)
    Game {
        game: String,
        domain: Option<String>,
        /// Remove the setting
        #[arg(long, conflicts_with = "domain")]
        clear: bool,
    },
    /// Find Nexus Mods games by name
    Games { name: Vec<String> },
    /// Search the game's mods on Nexus Mods
    Search {
        game: String,
        /// Words in the mod's name
        query: Vec<String>,
        /// endorsements, downloads, updated or name
        #[arg(long, default_value = "endorsements")]
        sort: String,
        #[arg(long, default_value_t = 25)]
        limit: u64,
        /// Only mods with a file installed
        #[arg(long)]
        installed: bool,
    },
    /// List a mod's files (MOD: its page link or number)
    Files { game: String, r#mod: String },
    /// Download and install a mod's file (its main file, or --file). Premium accounts
    /// download in UMM; otherwise UMM gives the file's page on the website and installs the
    /// archive once it appears in the Downloads folder.
    Download {
        game: String,
        r#mod: String,
        #[arg(long)]
        file: Option<u64>,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
        /// Where the browser saves downloads (default: your Downloads folder)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Install an archive downloaded from Nexus Mods, recognised by its hash
    Import {
        game: String,
        archive: PathBuf,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    /// Check installed Nexus mods for newer versions of their files
    Updates { game: String },
    /// Install newer versions of installed Nexus files (all of them if none are named).
    /// Premium accounts download in UMM; otherwise UMM gives each file's page.
    Update {
        game: String,
        mods: Vec<String>,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    /// Sign in with a Premium account's personal API key, to download in UMM. Without KEY,
    /// it is read from the terminal, so it stays out of your shell history.
    SignIn { key: Option<String> },
    /// Forget the API key
    SignOut,
    /// Show the signed-in account
    Status,
}

fn print_install(r: &NexusInstallReport) {
    let what = if r.replaced { "updated" } else { "installed" };
    let version = r
        .meta
        .version
        .as_deref()
        .map(|v| format!(" {v}"))
        .unwrap_or_default();
    println!("{what} {} ({}){version}", r.meta.name, r.meta.id);
    if r.domain_set {
        if let Some(n) = &r.meta.nexus {
            println!("this game is now set to the Nexus Mods game `{}`", n.domain);
        }
    }
}

/// A mod named by a page link or a number.
fn mod_id(m: &str) -> Result<u64> {
    if let Some((_, id)) = umm_nexus::parse_mod_url(m) {
        return Ok(id);
    }
    m.parse()
        .with_context(|| format!("`{m}` is not a Nexus Mods page link or mod number"))
}

fn premium(s: &Session) -> bool {
    s.nexus_account().is_some_and(|a| a.is_premium)
}

/// Waits for the file to be downloaded on the website, then installs it.
fn download_on_site(
    s: &Session,
    game: &str,
    want: &ExpectedFile,
    profile: &str,
    dir: Option<PathBuf>,
) -> Result<NexusInstallReport> {
    let dir = match dir.or_else(umm_core::downloads_dir) {
        Some(d) => d,
        None => bail!("cannot find your Downloads folder; give it with --dir"),
    };
    let watch = DownloadWatch::new(&dir).with_context(|| format!("reading {}", dir.display()))?;
    println!(
        "Download it on the website (click \"Slow download\" if asked):\n  {}",
        Session::nexus_download_page(&want.domain, want.mod_id, want.file_id)
    );
    println!("Waiting for it in {} (Ctrl+C to stop)...", dir.display());
    let path = loop {
        if let Some(p) = watch.poll() {
            break p;
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    println!("found {}", path.display());
    let progress = &mut |m: &str| eprintln!("{m}");
    match s.nexus_install_download(game, &path, Some(want), false, profile, progress) {
        Err(umm_core::CoreError::NexusUnrecognised) => {
            eprint!("Nexus Mods does not recognise this file. Install it as the file you downloaded? [y/N] ");
            std::io::stderr().flush().ok();
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            if !line.trim().eq_ignore_ascii_case("y") {
                bail!("not installed");
            }
            Ok(s.nexus_install_download(game, &path, Some(want), true, profile, progress)?)
        }
        r => Ok(r?),
    }
}

pub fn run(s: &Session, cmd: NexusCmd) -> Result<()> {
    let progress = &mut |m: &str| eprintln!("{m}");
    match cmd {
        NexusCmd::Game {
            game,
            domain,
            clear,
        } => {
            if clear {
                s.set_nexus_domain(&game, None)?;
                println!("{game}: no Nexus Mods game set");
            } else if let Some(d) = domain {
                s.set_nexus_domain(&game, Some(&d))?;
                println!(
                    "{game}: Nexus Mods game `{d}` ({})",
                    umm_nexus::game_url(&d)
                );
            } else {
                match s.nexus_game(&game)? {
                    Some(g) => {
                        let how = if g.guessed {
                            " (found by name; not saved yet)"
                        } else {
                            ""
                        };
                        let name = g.name.map(|n| format!(", {n}")).unwrap_or_default();
                        println!("{game}: Nexus Mods game `{}`{name}{how}", g.domain);
                    }
                    None => println!(
                        "{game}: no Nexus Mods game found; find one with `umm nexus games <name>`"
                    ),
                }
            }
        }
        NexusCmd::Games { name } => {
            for g in s.nexus_find_games(&name.join(" "))? {
                println!(
                    "{:<32} {} ({} mods)",
                    g.domain_name,
                    g.name,
                    g.mod_count.unwrap_or(0)
                );
            }
        }
        NexusCmd::Search {
            game,
            query,
            sort,
            limit,
            installed,
        } => {
            let sort = match sort.as_str() {
                "endorsements" => NexusSort::Endorsements,
                "downloads" => NexusSort::Downloads,
                "updated" => NexusSort::Updated,
                "name" => NexusSort::Name,
                other => bail!("unknown sort `{other}` (endorsements, downloads, updated, name)"),
            };
            let page = s.nexus_search(
                &game,
                &NexusSearch {
                    text: query.join(" "),
                    sort,
                    count: limit,
                    include_adult: true,
                    ..Default::default()
                },
                installed,
            )?;
            println!("{} mods on nexusmods.com/{}", page.total, page.game.domain);
            for it in page.items {
                let m = &it.summary;
                let mark = if it.installed.is_empty() {
                    ""
                } else {
                    " [installed]"
                };
                println!(
                    "{:>7}  {} {} by {}{mark}  ({} endorsements)",
                    m.mod_id,
                    m.name,
                    m.version,
                    m.author.as_deref().unwrap_or("?"),
                    m.endorsements
                );
            }
        }
        NexusCmd::Files { game, r#mod } => {
            let f = s.nexus_files(&game, mod_id(&r#mod)?)?;
            println!(
                "{} ({})",
                f.summary.name,
                umm_nexus::mod_url(&f.domain, f.summary.mod_id)
            );
            for file in &f.files {
                let mark = if f.installed.contains(&file.file_id) {
                    "  [installed]"
                } else {
                    ""
                };
                println!(
                    "{:>10}  {:<14} {:<40} {}{mark}",
                    file.file_id, file.category, file.name, file.version
                );
            }
            if !f.requirements.is_empty() {
                println!("needs:");
            }
            for r in &f.requirements {
                let state = match (r.installed, r.mod_id) {
                    (true, _) => "installed".to_string(),
                    (false, Some(id)) => format!("mod {id}: `umm nexus download {game} {id}`"),
                    (false, None) => "not on Nexus Mods for this game".to_string(),
                };
                let notes = r
                    .notes
                    .as_deref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default();
                let url = r
                    .url
                    .as_deref()
                    .map(|u| format!(" {u}"))
                    .unwrap_or_default();
                println!("  {}{notes}: {state}{url}", r.name);
            }
        }
        NexusCmd::Download {
            game,
            r#mod,
            file,
            profile,
            dir,
        } => {
            let f = s.nexus_files(&game, mod_id(&r#mod)?)?;
            let file_id = match file {
                Some(id) => id,
                None => {
                    let main: Vec<_> = f.files.iter().filter(|x| x.category == "MAIN").collect();
                    match main.as_slice() {
                        [one] => one.file_id,
                        [] => bail!("the mod has no main file; pick one with --file (see `umm nexus files`)"),
                        _ => bail!("the mod has several main files; pick one with --file (see `umm nexus files`)"),
                    }
                }
            };
            let r = if premium(s) {
                let nf = NexusFile {
                    domain: f.domain.clone(),
                    mod_id: f.summary.mod_id,
                    file_id,
                    credentials: None,
                };
                s.nexus_install(&game, &nf, &profile, progress)?
            } else {
                let want = ExpectedFile {
                    domain: f.domain.clone(),
                    mod_id: f.summary.mod_id,
                    file_id,
                };
                download_on_site(s, &game, &want, &profile, dir)?
            };
            print_install(&r);
        }
        NexusCmd::Import {
            game,
            archive,
            profile,
        } => {
            let r = s.nexus_install_download(&game, &archive, None, false, &profile, progress)?;
            print_install(&r);
        }
        NexusCmd::Updates { game } => {
            let list = s.nexus_updates(&game)?;
            if list.is_empty() {
                println!("no mods from Nexus Mods installed for {game}");
            }
            for u in list {
                let have = u.nexus.version.as_deref().unwrap_or("?");
                match u.state {
                    NexusUpdateState::Current => println!("{}: {have}, up to date", u.id),
                    NexusUpdateState::Newer { version, name, .. } => {
                        println!("{}: {have} -> {version} ({name})", u.id)
                    }
                    NexusUpdateState::Outdated => {
                        println!("{}: {have}, marked old by its author", u.id)
                    }
                    NexusUpdateState::Gone => {
                        println!("{}: {have}, no longer on Nexus Mods", u.id)
                    }
                }
            }
        }
        NexusCmd::Update {
            game,
            mods,
            profile,
        } => {
            let updates: Vec<_> = s
                .nexus_updates(&game)?
                .into_iter()
                .filter(|u| mods.is_empty() || mods.contains(&u.id))
                .filter(|u| matches!(u.state, NexusUpdateState::Newer { .. }))
                .collect();
            if updates.is_empty() {
                println!("everything is up to date");
            }
            for u in updates {
                let NexusUpdateState::Newer { file_id, .. } = u.state else {
                    continue;
                };
                let r = if premium(s) {
                    s.nexus_update(&game, &u.id, &profile, progress)?
                        .context("already up to date")?
                } else {
                    let want = ExpectedFile {
                        domain: u.nexus.domain.clone(),
                        mod_id: u.nexus.mod_id,
                        file_id,
                    };
                    download_on_site(s, &game, &want, &profile, None)?
                };
                print_install(&r);
            }
        }
        NexusCmd::SignIn { key } => {
            let key = match key {
                Some(k) => k,
                None => {
                    eprintln!("Create a personal API key at {}", umm_nexus::API_KEY_PAGE);
                    eprint!("Paste it here: ");
                    std::io::stderr().flush().ok();
                    let mut line = String::new();
                    std::io::stdin().lock().read_line(&mut line)?;
                    line
                }
            };
            let a = s.nexus_sign_in(&key)?;
            if a.is_premium {
                println!(
                    "signed in as {} (Premium): UMM downloads files itself",
                    a.name
                );
            } else {
                println!(
                    "signed in as {} (free account): downloads still go through the website",
                    a.name
                );
            }
        }
        NexusCmd::SignOut => {
            s.nexus_sign_out()?;
            println!("signed out of Nexus Mods");
        }
        NexusCmd::Status => match s.nexus_account() {
            Some(a) => {
                let kind = if a.is_premium { "Premium" } else { "free" };
                println!("signed in as {} ({kind} account)", a.name);
            }
            None => println!("not signed in; browsing and website downloads need no account"),
        },
    }
    Ok(())
}
