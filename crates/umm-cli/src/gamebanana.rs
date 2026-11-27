//! `umm gamebanana ...`: GameBanana from the command line.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use umm_core::{GbInstallReport, GbSearch, GbSort, GbUpdateState, Session, DEFAULT_PROFILE};

#[derive(Subcommand)]
pub enum GbCmd {
    /// Show or set the game's GameBanana game (its number in gamebanana.com/games/<number>)
    Game {
        game: String,
        number: Option<u64>,
        /// Remove the setting
        #[arg(long, conflicts_with = "number")]
        clear: bool,
    },
    /// Find GameBanana games by name
    Games { name: Vec<String> },
    /// Search the game's mods on GameBanana
    Search {
        game: String,
        /// Words to search for
        query: Vec<String>,
        /// downloads, likes, updated, newest or name (searching words: downloads or name)
        #[arg(long, default_value = "downloads")]
        sort: String,
        /// The page of results, from 1
        #[arg(long, default_value_t = 1)]
        page: u32,
        /// Only mods with a file installed
        #[arg(long)]
        installed: bool,
    },
    /// List a mod's files (MOD: its page link or number)
    Files { game: String, r#mod: String },
    /// Download and install a mod's file (its newest, or --file)
    Install {
        game: String,
        r#mod: String,
        #[arg(long)]
        file: Option<u64>,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
    /// Check installed GameBanana files for newer uploads
    Updates { game: String },
    /// Install newer uploads of installed GameBanana files (all of them if none are named)
    Update {
        game: String,
        mods: Vec<String>,
        #[arg(short, long, default_value = DEFAULT_PROFILE)]
        profile: String,
    },
}

fn print_install(r: &GbInstallReport) {
    let what = if r.replaced { "updated" } else { "installed" };
    let version = r
        .meta
        .version
        .as_deref()
        .map(|v| format!(" {v}"))
        .unwrap_or_default();
    println!("{what} {} ({}){version}", r.meta.name, r.meta.id);
    if r.game_set {
        if let Some(g) = &r.meta.gamebanana {
            println!("this game is now set to GameBanana game {}", g.game_id);
        }
    }
}

/// A mod named by a page link or a number.
fn mod_id(m: &str) -> Result<u64> {
    if let Some(id) = umm_gamebanana::parse_mod_url(m) {
        return Ok(id);
    }
    m.parse()
        .with_context(|| format!("`{m}` is not a GameBanana page link or mod number"))
}

fn date(unix: u64) -> String {
    // Days since 1970 to a calendar date (proleptic Gregorian), for listing files.
    let days = (unix / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn run(s: &Session, cmd: GbCmd) -> Result<()> {
    let progress = &mut |m: &str| eprintln!("{m}");
    match cmd {
        GbCmd::Game {
            game,
            number,
            clear,
        } => {
            if clear {
                s.set_gamebanana_game(&game, None)?;
                println!("{game}: no GameBanana game set");
            } else if let Some(n) = number {
                s.set_gamebanana_game(&game, Some(n))?;
                println!(
                    "{game}: GameBanana game {n} ({})",
                    umm_gamebanana::game_url(n)
                );
            } else {
                match s.gamebanana_game(&game)? {
                    Some(g) => {
                        let how = if g.guessed {
                            " (found by name; not saved yet)"
                        } else {
                            ""
                        };
                        let name = g.name.map(|n| format!(", {n}")).unwrap_or_default();
                        println!("{game}: GameBanana game {}{name}{how}", g.game_id);
                    }
                    None => println!(
                        "{game}: no GameBanana game found; find one with `umm gamebanana games <name>`"
                    ),
                }
            }
        }
        GbCmd::Games { name } => {
            for g in s.gamebanana_find_games(&name.join(" "))? {
                println!("{:>7}  {}", g.id, g.name);
            }
        }
        GbCmd::Search {
            game,
            query,
            sort,
            page,
            installed,
        } => {
            let sort = match sort.as_str() {
                "downloads" => GbSort::Downloads,
                "likes" => GbSort::Likes,
                "updated" => GbSort::Updated,
                "newest" => GbSort::Newest,
                "name" => GbSort::Name,
                other => bail!("unknown sort `{other}` (downloads, likes, updated, newest, name)"),
            };
            let p = s.gamebanana_search(
                &game,
                &GbSearch {
                    text: query.join(" "),
                    sort,
                    page,
                    per_page: 25,
                },
                installed,
            )?;
            println!(
                "{} mods on {}",
                p.total,
                umm_gamebanana::game_url(p.game.game_id)
            );
            for it in p.items {
                let m = &it.summary;
                let mut marks = String::new();
                if !it.installed.is_empty() {
                    marks.push_str(" [installed]");
                }
                if m.obsolete {
                    marks.push_str(" [obsolete]");
                }
                println!(
                    "{:>7}  {} {} by {}{marks}  ({} likes)",
                    m.mod_id,
                    m.name,
                    m.version,
                    m.author.as_deref().unwrap_or("?"),
                    m.likes
                );
            }
            if !p.complete {
                println!("more: --page {}", page + 1);
            }
        }
        GbCmd::Files { game, r#mod } => {
            let f = s.gamebanana_files(&game, mod_id(&r#mod)?)?;
            let d = &f.details;
            println!(
                "{} {} ({})",
                d.summary.name,
                d.summary.version,
                umm_gamebanana::mod_url(d.summary.mod_id)
            );
            for x in &d.files {
                let installed = f
                    .installed
                    .iter()
                    .find(|(id, _)| *id == x.file_id)
                    .map(|(_, m)| format!(" [installed as {m}]"))
                    .unwrap_or_default();
                let about = if x.description.is_empty() {
                    String::new()
                } else {
                    format!("  {}", x.description)
                };
                println!(
                    "{:>9}  {}  {} KB  {}{about}{installed}",
                    x.file_id,
                    x.name,
                    x.size / 1024,
                    date(x.added)
                );
                if x.clean == Some(false) {
                    println!("           GameBanana's virus scan flagged this file");
                }
            }
        }
        GbCmd::Install {
            game,
            r#mod,
            file,
            profile,
        } => {
            let id = mod_id(&r#mod)?;
            let file = match file {
                Some(f) => f,
                None => {
                    let f = s.gamebanana_files(&game, id)?;
                    f.details
                        .files
                        .first()
                        .map(|x| x.file_id)
                        .with_context(|| format!("{} has no files", f.details.summary.name))?
                }
            };
            print_install(&s.gamebanana_install(&game, id, file, &profile, progress)?);
        }
        GbCmd::Updates { game } => {
            for u in s.gamebanana_updates(&game)? {
                let v = u.version.as_deref().unwrap_or("?");
                match u.state {
                    GbUpdateState::Current => println!("{}: {v}, current", u.id),
                    GbUpdateState::Newer {
                        version, file_name, ..
                    } => println!("{}: {v} → {version} ({file_name})", u.id),
                    GbUpdateState::Gone => println!("{}: {v}, no longer on GameBanana", u.id),
                }
            }
        }
        GbCmd::Update {
            game,
            mods,
            profile,
        } => {
            let mut any = false;
            for u in s.gamebanana_updates(&game)? {
                if !mods.is_empty() && !mods.contains(&u.id) {
                    continue;
                }
                if let GbUpdateState::Newer { file_id, .. } = u.state {
                    any = true;
                    print_install(
                        &s.gamebanana_install(&game, u.mod_id, file_id, &profile, progress)?,
                    );
                }
            }
            if !any {
                println!("nothing to update");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        assert_eq!(super::date(0), "1970-01-01");
        assert_eq!(super::date(1678916481), "2023-03-15");
    }
}
