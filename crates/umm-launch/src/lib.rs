//! Starting a game and knowing when it has really exited.
//!
//! * [`Tracking::Job`]: the launcher starts the exe itself, suspended, inside a Windows job
//!   object, then resumes it. Every process it spawns joins the job, so the session ends only
//!   when the whole tree has exited (launcher-to-game hops, crash reporters). Elsewhere the
//!   exe starts in a process group of its own, followed through `/proc` the same way.
//! * [`Tracking::Names`]: for launches handed to Steam (`steam.exe -applaunch`), where the
//!   game is not our child: wait for a matching process to appear, then until none has been
//!   alive for a grace period.

#[cfg(not(windows))]
mod tree;
#[cfg(windows)]
mod win;

use std::io;
use std::path::PathBuf;
#[cfg(not(windows))]
use std::process::Child;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tracking {
    Job,
    Names {
        names: Vec<String>,
        appear_timeout: Duration,
        grace: Duration,
    },
}

impl Tracking {
    pub fn names(names: Vec<String>) -> Self {
        Tracking::Names {
            names,
            // Steam may have to start, update or ask about cloud saves first.
            appear_timeout: Duration::from_secs(180),
            grace: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub tracking: Tracking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Started { pid: u32 },
    WaitingForProcess,
    ProcessDetected,
}

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Exit code of the process the launcher started, when it is the game itself.
    pub exit_code: Option<i32>,
    /// False when [`Tracking::Names`] never saw the game start.
    pub appeared: bool,
    /// True when the caller asked to stop waiting while the game was still running.
    pub abandoned: bool,
    pub duration: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("starting {program}: {source}")]
    Spawn { program: PathBuf, source: io::Error },
    #[error("tracking the game's processes: {0}")]
    Track(io::Error),
}

const POLL: Duration = Duration::from_millis(250);

/// Starts the game and blocks until it has exited. `stop` is polled while waiting; when it
/// returns true the call returns at once with `abandoned` set and the game left running.
pub fn run(
    spec: &LaunchSpec,
    stop: &dyn Fn() -> bool,
    on_event: &mut dyn FnMut(Event),
) -> Result<Outcome, LaunchError> {
    let started = Instant::now();
    let mut out = match &spec.tracking {
        Tracking::Job => run_job(spec, stop, on_event)?,
        Tracking::Names {
            names,
            appear_timeout,
            grace,
        } => {
            let mut child = command(spec).spawn().map_err(|source| LaunchError::Spawn {
                program: spec.program.clone(),
                source,
            })?;
            on_event(Event::Started { pid: child.id() });
            let out = wait_names(names, *appear_timeout, *grace, stop, on_event)
                .map_err(LaunchError::Track)?;
            // `steam -applaunch` hands off to a running Steam and exits; reap it if so.
            let _ = child.try_wait();
            out
        }
    };
    out.duration = started.elapsed();
    Ok(out)
}

fn command(spec: &LaunchSpec) -> Command {
    let mut c = Command::new(&spec.program);
    c.args(&spec.args);
    for (k, v) in &spec.env {
        c.env(k, v);
    }
    if let Some(cwd) = &spec.cwd {
        c.current_dir(cwd);
    }
    c
}

#[cfg(windows)]
fn run_job(
    spec: &LaunchSpec,
    stop: &dyn Fn() -> bool,
    on_event: &mut dyn FnMut(Event),
) -> Result<Outcome, LaunchError> {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;

    let job = win::Job::new().map_err(LaunchError::Track)?;
    let mut cmd = command(spec);
    cmd.creation_flags(win::CREATE_SUSPENDED);
    let mut child = cmd.spawn().map_err(|source| LaunchError::Spawn {
        program: spec.program.clone(),
        source,
    })?;
    // Assign before resuming, so not even the first child process can escape the job.
    let setup = job
        .assign(child.as_raw_handle())
        .and_then(|()| win::resume_process(child.id()));
    if let Err(e) = setup {
        let _ = child.kill();
        return Err(LaunchError::Track(e));
    }
    on_event(Event::Started { pid: child.id() });

    let mut exit_code = None;
    loop {
        if exit_code.is_none() {
            if let Ok(Some(status)) = child.try_wait() {
                exit_code = status.code();
            }
        }
        if job.active_processes().map_err(LaunchError::Track)? == 0 {
            break;
        }
        if stop() {
            return Ok(Outcome {
                exit_code,
                appeared: true,
                abandoned: true,
                ..Default::default()
            });
        }
        sleep(POLL);
    }
    if exit_code.is_none() {
        exit_code = child.wait().ok().and_then(|s| s.code());
    }
    Ok(Outcome {
        exit_code,
        appeared: true,
        ..Default::default()
    })
}

/// Without job objects: the game starts in a process group of its own, and the session ends
/// once no process is left in it or below the game (see [`tree::Tree`]).
#[cfg(not(windows))]
fn run_job(
    spec: &LaunchSpec,
    stop: &dyn Fn() -> bool,
    on_event: &mut dyn FnMut(Event),
) -> Result<Outcome, LaunchError> {
    use std::os::unix::process::CommandExt;
    let mut child: Child =
        command(spec)
            .process_group(0)
            .spawn()
            .map_err(|source| LaunchError::Spawn {
                program: spec.program.clone(),
                source,
            })?;
    on_event(Event::Started { pid: child.id() });
    let mut tree = tree::Tree::new(child.id());
    let mut exit_code = None;
    loop {
        if exit_code.is_none() {
            if let Some(status) = child.try_wait().map_err(LaunchError::Track)? {
                exit_code = Some(status.code());
            }
        }
        // The game has exited, and nothing it started is left.
        if let Some(code) = exit_code {
            if !tree.alive().map_err(LaunchError::Track)? {
                return Ok(Outcome {
                    exit_code: code,
                    appeared: true,
                    ..Default::default()
                });
            }
        } else {
            // Note its children while it runs, should they leave its group.
            tree.alive().map_err(LaunchError::Track)?;
        }
        if stop() {
            return Ok(Outcome {
                appeared: true,
                abandoned: true,
                ..Default::default()
            });
        }
        sleep(POLL);
    }
}

fn wait_names(
    names: &[String],
    appear_timeout: Duration,
    grace: Duration,
    stop: &dyn Fn() -> bool,
    on_event: &mut dyn FnMut(Event),
) -> io::Result<Outcome> {
    on_event(Event::WaitingForProcess);
    let t0 = Instant::now();
    while !is_any_running(names)? {
        if t0.elapsed() >= appear_timeout {
            return Ok(Outcome::default());
        }
        if stop() {
            return Ok(Outcome {
                abandoned: true,
                ..Default::default()
            });
        }
        sleep(POLL);
    }
    on_event(Event::ProcessDetected);
    let mut last_seen = Instant::now();
    while last_seen.elapsed() < grace {
        if is_any_running(names)? {
            last_seen = Instant::now();
        }
        if stop() {
            return Ok(Outcome {
                appeared: true,
                abandoned: true,
                ..Default::default()
            });
        }
        sleep(POLL);
    }
    Ok(Outcome {
        appeared: true,
        ..Default::default()
    })
}

/// Executable names of every running process (e.g. `Game.exe`).
pub fn running_processes() -> io::Result<Vec<String>> {
    #[cfg(windows)]
    {
        win::process_names()
    }
    #[cfg(not(windows))]
    {
        let mut out = Vec::new();
        for e in std::fs::read_dir("/proc")?.filter_map(|e| e.ok()) {
            let p = e.path();
            if let Ok(cmd) = std::fs::read(p.join("cmdline")) {
                if let Some(first) = cmd.split(|b| *b == 0).next() {
                    let s = String::from_utf8_lossy(first);
                    // Wine processes show Windows paths.
                    if let Some(name) = s.rsplit(['/', '\\']).next() {
                        if !name.is_empty() {
                            out.push(name.to_string());
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

/// True if a process with one of these executable names is running (case-insensitive).
pub fn is_any_running(names: &[String]) -> io::Result<bool> {
    if names.is_empty() {
        return Ok(false);
    }
    Ok(running_processes()?
        .iter()
        .any(|p| names.iter().any(|n| n.eq_ignore_ascii_case(p))))
}

/// Makes Ctrl+C (and Ctrl+Break) increment a counter instead of killing the process, so a
/// launcher can keep waiting and still revert. Returns the counter. Windows only; elsewhere
/// the counter never moves and Ctrl+C keeps its default behaviour (recovery runs next time).
pub fn catch_interrupts() -> &'static std::sync::atomic::AtomicUsize {
    #[cfg(windows)]
    {
        win::catch_interrupts()
    }
    #[cfg(not(windows))]
    {
        static NEVER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        &NEVER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_own_process() {
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().into_owned();
        assert!(is_any_running(&[name.to_uppercase()]).unwrap());
        assert!(!is_any_running(&["definitely-not-running-umm.exe".into()]).unwrap());
        assert!(!is_any_running(&[]).unwrap());
    }
}
