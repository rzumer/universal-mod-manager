//! A game's processes on Linux, from `/proc`: those in the process group it was started in,
//! and those below it (a child that started a group of its own included), as far as they
//! were seen while their parent was alive.

use std::collections::HashSet;
use std::fs;
use std::io;

pub(crate) struct Tree {
    group: u32,
    /// The game and the processes seen below it, still alive at the last look.
    known: HashSet<u32>,
}

/// `(pid, ppid, pgrp)` of each live process (zombies left out).
fn processes() -> io::Result<Vec<(u32, u32, u32)>> {
    let mut out = Vec::new();
    for e in fs::read_dir("/proc")?.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        // Gone in the meantime, or not ours to read.
        let Ok(stat) = fs::read_to_string(e.path().join("stat")) else {
            continue;
        };
        if let Some(p) = parse_stat(pid, &stat) {
            out.push(p);
        }
    }
    Ok(out)
}

/// `pid (comm) state ppid pgrp …`; the name may hold spaces and parentheses, so the fields
/// are read after its last `)`.
fn parse_stat(pid: u32, stat: &str) -> Option<(u32, u32, u32)> {
    let mut f = stat.get(stat.rfind(')')? + 1..)?.split_whitespace();
    let state = f.next()?;
    if matches!(state, "Z" | "X" | "x") {
        return None;
    }
    let ppid = f.next()?.parse().ok()?;
    let pgrp = f.next()?.parse().ok()?;
    Some((pid, ppid, pgrp))
}

impl Tree {
    /// The tree of the process `leader`, started as the leader of a group of its own.
    pub(crate) fn new(leader: u32) -> Self {
        Tree {
            group: leader,
            known: HashSet::from([leader]),
        }
    }

    /// Whether any of its processes is alive, noting new ones below those known.
    pub(crate) fn alive(&mut self) -> io::Result<bool> {
        let procs = processes()?;
        // Those gone are forgotten, so that their ids, used again, are not taken for ours.
        let live: HashSet<u32> = procs.iter().map(|p| p.0).collect();
        self.known.retain(|p| live.contains(p));
        loop {
            let before = self.known.len();
            for &(pid, ppid, _) in &procs {
                if self.known.contains(&ppid) {
                    self.known.insert(pid);
                }
            }
            if self.known.len() == before {
                break;
            }
        }
        Ok(!self.known.is_empty() || procs.iter().any(|p| p.2 == self.group))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_fields_after_the_name() {
        assert_eq!(
            parse_stat(7, "7 (Game (x) 1.exe) S 3 7 7 0 -1"),
            Some((7, 3, 7))
        );
        assert_eq!(parse_stat(7, "7 (game) Z 3 7 7 0 -1"), None);
    }

    #[test]
    fn sees_a_child_and_its_end() {
        use std::os::unix::process::CommandExt;
        let mut c = std::process::Command::new("sh")
            .args(["-c", "sleep 0.5 & wait"])
            .process_group(0)
            .spawn()
            .unwrap();
        let mut t = Tree::new(c.id());
        assert!(t.alive().unwrap());
        c.wait().unwrap();
        assert!(!t.alive().unwrap());
    }
}
