//! Autosave files: where they live, and finding the ones a crash left behind.
//!
//! Each running app owns one *session* folder, `<config>/autosave/<id>/`:
//!
//! * `lock` — held open for as long as the app runs, shared with nobody
//!   (Windows). Another instance trying to open it gets a sharing violation,
//!   which is how it tells a live session from a dead one. The OS closes it
//!   however the app ends, a crash included.
//! * `project.anim` — the latest autosave, an ordinary project file.
//! * `source.txt` — the file that project was opened from or saved to; empty
//!   for one never saved.
//!
//! An *orphan* is a session folder nobody holds the lock of that still has a
//! `project.anim`: the app that wrote it ended without cleaning up (a crash,
//! a kill, a power cut, a shutdown that never asked). A clean exit removes its
//! folder, and a save clears the autosave in it, so an orphan always holds
//! work that is not in its source file.
//!
//! The timing — when to write, and what counts as unsaved — is
//! `crate::app::autosave`'s. This file only knows the folders.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::doc::project::Project;
use crate::io::project_file;

const LOCK: &str = "lock";
const PROJECT: &str = "project.anim";
const SOURCE: &str = "source.txt";

/// The folder every session folder goes in. `None` under test, as for every
/// file the app keeps between runs: a test must never write into, or recover
/// from, the folder of the person running it.
pub fn root() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::input::shortcuts::config_dir().map(|d| d.join("autosave"))
}

/// This run's folder, and the lock that marks it as alive.
pub struct Session {
    dir: PathBuf,
    /// Never read: holding it open is the whole point.
    _lock: File,
}

impl Session {
    /// A new, empty session folder under `root`, locked.
    pub fn start(root: &Path) -> Result<Session> {
        // The count keeps two sessions one process starts in the same
        // millisecond apart (tests do).
        static STARTED: AtomicU32 = AtomicU32::new(0);
        let n = STARTED.fetch_add(1, Ordering::Relaxed);
        let millis = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let dir = root.join(format!("{millis}-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).with_context(|| format!("creating {dir:?}"))?;
        let lock = hold_lock(&dir.join(LOCK)).with_context(|| format!("locking {dir:?}"))?;
        Ok(Session { dir, _lock: lock })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Drop the autosave, keeping the session: the work it held is now in
    /// its file, or was thrown away on purpose.
    pub fn clear(&self) {
        for name in [PROJECT, SOURCE] {
            let _ = fs::remove_file(self.dir.join(name));
        }
    }

    /// The clean end of a run: the folder goes, so nothing is offered for
    /// recovery next time.
    pub fn end(self) {
        let Session { dir, _lock } = self;
        drop(_lock);
        if let Err(e) = fs::remove_dir_all(&dir) {
            log::warn!("Couldn't remove autosave folder {dir:?}: {e}");
        }
    }
}

/// Autosave `project` into the session folder `dir`, noting `source` beside
/// it. Takes the folder rather than the [`Session`] so a worker thread can
/// run it while the session stays with the app.
///
/// Both files are replaced whole (see [`project_file::write_replacing`]), so
/// a crash mid-autosave leaves the previous autosave readable.
pub fn write(dir: &Path, project: &Project, source: Option<&Path>) -> Result<()> {
    let text = source.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    project_file::write_replacing(&dir.join(SOURCE), text.as_bytes())?;
    project_file::save_to(project, &dir.join(PROJECT))
}

/// Work an earlier run left behind.
#[derive(Clone, Debug)]
pub struct Orphan {
    pub dir: PathBuf,
    /// The file it came from; `None` for a project never saved.
    pub source: Option<PathBuf>,
    /// When it was autosaved.
    pub saved: SystemTime,
    pub bytes: u64,
}

impl Orphan {
    /// The autosaved project file.
    pub fn project(&self) -> PathBuf {
        self.dir.join(PROJECT)
    }

    /// What to call it: the source file's name, or "Untitled".
    pub fn name(&self) -> String {
        self.source
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "Untitled".to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// When the source file was last written, if it still exists.
    pub fn source_saved(&self) -> Option<SystemTime> {
        self.source.as_deref().and_then(|p| fs::metadata(p).ok()?.modified().ok())
    }
}

/// Every orphan under `root`, newest first. Dead session folders with no
/// autosave in them (a run that ended before its first one) are removed on
/// the way.
pub fn orphans(root: &Path) -> Vec<Orphan> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        if is_live(&dir) {
            continue;
        }
        let project = dir.join(PROJECT);
        let Ok(meta) = fs::metadata(&project) else {
            let _ = fs::remove_dir_all(&dir);
            continue;
        };
        let source = fs::read_to_string(dir.join(SOURCE))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        out.push(Orphan {
            source,
            saved: meta.modified().unwrap_or(UNIX_EPOCH),
            bytes: meta.len(),
            dir,
        });
    }
    out.sort_by(|a, b| b.saved.cmp(&a.saved));
    out
}

/// Take over an orphan the app has just loaded: its autosave becomes this
/// session's, so the recovered work stays safe until the next autosave or
/// save, and its folder goes. Called after the load, which clears the
/// session — in the other order the clear would delete what was adopted.
pub fn adopt(orphan: &Orphan, session: &Session) -> Result<()> {
    for name in [PROJECT, SOURCE] {
        let from = orphan.dir.join(name);
        if from.exists() {
            fs::rename(&from, session.dir.join(name)).with_context(|| format!("moving {from:?}"))?;
        }
    }
    discard(orphan)
}

/// Delete an orphan for good.
pub fn discard(orphan: &Orphan) -> Result<()> {
    fs::remove_dir_all(&orphan.dir).with_context(|| format!("removing {:?}", orphan.dir))
}

/// Open the lock file so that no one else can open it while we hold it.
#[cfg(windows)]
fn hold_lock(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new().write(true).create(true).truncate(true).share_mode(0).open(path)
}

#[cfg(not(windows))]
fn hold_lock(path: &Path) -> std::io::Result<File> {
    File::create(path)
}

/// Whether the app that made session folder `dir` is still running.
#[cfg(windows)]
fn is_live(dir: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    match fs::OpenOptions::new().write(true).share_mode(0).open(dir.join(LOCK)) {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

/// Elsewhere there is no unshared open: go by the process id in the folder's
/// name, where `/proc` can answer for it. Without `/proc` every other folder
/// counts as live — offering a running app's work for recovery would be worse
/// than offering none.
#[cfg(not(windows))]
fn is_live(dir: &Path) -> bool {
    let pid = dir.file_name().and_then(|n| n.to_str()).and_then(|n| n.split('-').nth(1));
    match pid {
        Some(pid) if Path::new("/proc").is_dir() => Path::new("/proc").join(pid).exists(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("animator-autosave-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn named(name: &str) -> Project {
        let mut p = Project::new(8, 8, 12.0);
        p.layers[0].name = name.into();
        p
    }

    #[test]
    fn tests_never_find_the_real_autosave_folder() {
        assert_eq!(root(), None);
    }

    #[test]
    fn a_live_session_is_not_an_orphan_and_a_dead_one_is() {
        let root = scratch("live");
        let live = Session::start(&root).unwrap();
        write(live.dir(), &named("live"), None).unwrap();
        assert!(orphans(&root).is_empty(), "a running session was offered for recovery");

        // What a crash leaves: the folder, with the lock let go.
        let dead_dir = live.dir().to_path_buf();
        drop(live);
        let found = orphans(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].dir, dead_dir);
        assert_eq!(found[0].name(), "Untitled");
        let back = project_file::load_from(&found[0].project()).unwrap();
        assert_eq!(back.layers[0].name, "live");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_source_file_comes_back_with_the_orphan() {
        let root = scratch("source");
        let s = Session::start(&root).unwrap();
        let src = root.join("walk cycle.anim");
        write(s.dir(), &named("a"), Some(&src)).unwrap();
        drop(s);
        let found = orphans(&root);
        assert_eq!(found[0].source.as_deref(), Some(src.as_path()));
        assert_eq!(found[0].name(), "walk cycle.anim");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dead_session_with_nothing_saved_is_tidied_away() {
        let root = scratch("empty");
        let s = Session::start(&root).unwrap();
        let dir = s.dir().to_path_buf();
        drop(s);
        assert!(orphans(&root).is_empty());
        assert!(!dir.exists(), "the empty folder was left");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_clean_end_and_a_clear_leave_nothing_to_recover() {
        let root = scratch("end");
        let s = Session::start(&root).unwrap();
        write(s.dir(), &named("a"), None).unwrap();
        s.clear();
        assert!(!s.dir().join(PROJECT).exists());
        let dir = s.dir().to_path_buf();
        s.end();
        assert!(!dir.exists());
        assert!(orphans(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn adopting_moves_the_autosave_into_this_session() {
        let root = scratch("adopt");
        let old = Session::start(&root).unwrap();
        write(old.dir(), &named("crashed"), None).unwrap();
        drop(old);
        let orphan = orphans(&root).remove(0);
        let now = Session::start(&root).unwrap();
        adopt(&orphan, &now).unwrap();
        assert!(!orphan.dir.exists());
        let back = project_file::load_from(&now.dir().join(PROJECT)).unwrap();
        assert_eq!(back.layers[0].name, "crashed");
        // Still this run's: nothing to offer while it is alive.
        assert!(orphans(&root).is_empty());
        now.end();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn orphans_come_newest_first() {
        let root = scratch("order");
        for name in ["older", "newer"] {
            let s = Session::start(&root).unwrap();
            write(s.dir(), &named(name), None).unwrap();
            drop(s);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let found = orphans(&root);
        let names: Vec<String> = found
            .iter()
            .map(|o| project_file::load_from(&o.project()).unwrap().layers[0].name.clone())
            .collect();
        assert_eq!(names, ["newer", "older"]);
        let _ = fs::remove_dir_all(&root);
    }
}
