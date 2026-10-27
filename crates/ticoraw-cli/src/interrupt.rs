//! Interruption (Ctrl-C, SIGTERM, SIGHUP): the temporary files of outputs still being
//! written are removed before the process exits, so an interrupted `convert` or `decode`
//! leaves neither a partial output nor a partial temporary file behind.
//!
//! A process killed outright (SIGKILL, a crash, a power cut) runs no handler; `convert`
//! removes such leftovers from the folders it writes to once they are a day old
//! ([`remove_stale_temps`]).

use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, Once, PoisonError};
use std::time::{Duration, SystemTime};

/// Exit status after an interruption (128 + SIGINT, as shells report Ctrl-C).
const EXIT_INTERRUPTED: i32 = 130;

/// A leftover temporary file older than this is no longer being written by anyone.
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// The temporary files being written right now.
static LIVE: Live = Live::new();

/// Installs the interruption handler (once per process). If it cannot be installed, the
/// default action applies: the process ends at once, possibly leaving a temporary file.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Best effort by design: without a handler, Ctrl-C still ends the process.
        let _ = ctrlc::set_handler(|| {
            LIVE.remove_all();
            if io::stderr().is_terminal() {
                // Clear a progress line first.
                eprint!("\r\x1b[2K");
            }
            eprintln!("ticoraw: interrupted");
            std::process::exit(EXIT_INTERRUPTED);
        });
    });
}

/// Creates the temporary file `path` (which must not exist yet) and tracks it until
/// [`forget`], so that an interruption removes it.
pub fn create_temp(path: &Path) -> io::Result<File> {
    LIVE.create(path)
}

/// Stops tracking `path` (renamed into place or removed).
pub fn forget(path: &Path) {
    LIVE.forget(path);
}

#[derive(Debug)]
struct Live(Mutex<State>);

#[derive(Debug)]
struct State {
    paths: Vec<PathBuf>,
    /// Set once the files were removed: the process is exiting, nothing new is created.
    closed: bool,
}

impl Live {
    const fn new() -> Self {
        Self(Mutex::new(State {
            paths: Vec::new(),
            closed: false,
        }))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Creating the file under the lock means an interruption sees either no file or a
    /// tracked one, never an untracked one.
    fn create(&self, path: &Path) -> io::Result<File> {
        let mut s = self.state();
        if s.closed {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "interrupted"));
        }
        let f = OpenOptions::new().write(true).create_new(true).open(path)?;
        s.paths.push(path.to_path_buf());
        Ok(f)
    }

    fn forget(&self, path: &Path) {
        self.state().paths.retain(|p| p != path);
    }

    /// Removes every tracked file and refuses new ones; returns how many were removed.
    fn remove_all(&self) -> usize {
        let mut s = self.state();
        s.closed = true;
        s.paths
            .drain(..)
            .filter(|p| std::fs::remove_file(p).is_ok())
            .count()
    }
}

/// The temporary file name `write_output` uses for `name` in this process.
pub fn temp_name(name: &str) -> String {
    format!(".{name}.ticoraw-{}.tmp", std::process::id())
}

/// Whether `name` has the shape of [`temp_name`]'s output for some process.
fn is_temp_name(name: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|n| n.strip_suffix(".tmp"))
        .and_then(|n| n.rsplit_once(".ticoraw-"))
        .is_some_and(|(stem, pid)| {
            !stem.is_empty() && !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit())
        })
}

/// Removes the regular files in `dir` named like [`temp_name`] that were last modified
/// more than `older_than` ago; returns how many were removed. A directory that cannot be
/// listed (often one that does not exist yet) has none.
pub fn remove_stale_temps(dir: &Path, older_than: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_str().is_some_and(is_temp_name))
        .filter(|e| {
            // `DirEntry::metadata` does not follow symlinks: only real files are removed.
            e.metadata().is_ok_and(|m| {
                m.is_file()
                    && m.modified()
                        .ok()
                        .and_then(|t| now.duration_since(t).ok())
                        .is_some_and(|age| age > older_than)
            })
        })
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ticoraw-interrupt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn removes_tracked_files_and_then_refuses_new_ones() {
        let dir = scratch("live");
        let live = Live::new();
        let (a, b) = (dir.join("a.tmp"), dir.join("b.tmp"));
        drop(live.create(&a).unwrap());
        drop(live.create(&b).unwrap());
        live.forget(&b);
        assert!(live.create(&a).is_err(), "create_new");
        assert_eq!(live.remove_all(), 1);
        assert!(!a.exists());
        assert!(b.exists(), "forgotten files are left alone");
        let c = dir.join("c.tmp");
        assert_eq!(
            live.create(&c).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(!c.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recognises_temp_names() {
        assert!(is_temp_name(&temp_name("DSC_0001.dng")));
        assert!(is_temp_name(".a.b.dng.ticoraw-123.tmp"));
        for name in [
            "DSC_0001.dng",
            ".a.dng.ticoraw-.tmp",
            ".a.dng.ticoraw-12x.tmp",
            "a.dng.ticoraw-12.tmp",
            "..ticoraw-12.tmp",
            ".a.dng.ticoraw-12.tmp.keep",
        ] {
            assert!(!is_temp_name(name), "{name}");
        }
    }

    #[test]
    fn removes_only_old_temp_files() {
        let dir = scratch("stale");
        let temp = dir.join(".a.dng.ticoraw-1.tmp");
        let other = dir.join("a.dng");
        std::fs::write(&temp, b"partial").unwrap();
        std::fs::write(&other, b"keep").unwrap();
        assert_eq!(remove_stale_temps(&dir, STALE_AFTER), 0, "fresh");
        assert!(temp.exists());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(remove_stale_temps(&dir, Duration::from_millis(1)), 1);
        assert!(!temp.exists());
        assert!(other.exists());
        assert_eq!(remove_stale_temps(&dir.join("missing"), Duration::ZERO), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
