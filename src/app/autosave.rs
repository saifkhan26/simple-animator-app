//! Keeping unsaved work: the timed autosave, recovering what a crash left,
//! and asking before closing, opening or starting over on top of unsaved
//! changes.
//!
//! "Unsaved" means the project's [`content_key`] differs from the one taken
//! when it was last loaded, started or saved. The files themselves are
//! `crate::io::autosave`'s.
//!
//! A child of `app` so it can work on `AppState`'s private state directly;
//! split out only because `app.rs` is long enough already.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;

use super::AppState;
use crate::io::autosave::{self, Orphan, Session};
use crate::io::project_file::{self, content_key};

/// Minutes between autosaves when the prefs don't say.
pub const DEFAULT_AUTOSAVE_MINUTES: u32 = 3;
/// How soon to try again when an autosave falls due mid-drag or mid-save.
const RETRY: Duration = Duration::from_secs(2);

/// Autosave's state on `AppState`.
pub struct Autosave {
    /// Minutes between autosaves; 0 = off. A preference.
    pub minutes: u32,
    /// This run's folder. `None` under test, or when it couldn't be made.
    session: Option<Session>,
    /// A write in flight, and the content key of what it writes.
    job: Option<(Receiver<Result<()>>, u64)>,
    /// Content key of what the session's autosave file holds, if it has one.
    written: Option<u64>,
    /// When the interval last started.
    since: Instant,
    /// Work earlier runs left behind, offered at launch.
    pub orphans: Vec<Orphan>,
    /// The recovery dialog is up.
    pub show_recovery: bool,
    /// The orphan whose Discard was clicked once; a second click deletes it.
    pub discard_armed: Option<PathBuf>,
    /// Why the last recover or discard failed, shown in the dialog.
    pub error: Option<String>,
}

impl Autosave {
    pub fn new(minutes: u32) -> Self {
        Autosave {
            minutes,
            session: None,
            job: None,
            written: None,
            since: Instant::now(),
            orphans: Vec::new(),
            show_recovery: false,
            discard_armed: None,
            error: None,
        }
    }

    /// Find what earlier runs left, then start this run's session. In that
    /// order, so this run's own folder is never among the orphans.
    pub fn start(&mut self) {
        let Some(root) = autosave::root() else {
            return;
        };
        self.orphans = autosave::orphans(&root);
        self.show_recovery = !self.orphans.is_empty();
        match Session::start(&root) {
            Ok(s) => self.session = Some(s),
            Err(e) => log::warn!("Autosave is off this run: {e:#}"),
        }
    }

    #[cfg(test)]
    pub fn start_in(&mut self, root: &std::path::Path) {
        self.orphans = autosave::orphans(root);
        self.show_recovery = !self.orphans.is_empty();
        self.session = Some(Session::start(root).unwrap());
    }

    #[cfg(test)]
    pub fn session_dir(&self) -> Option<&std::path::Path> {
        self.session.as_ref().map(Session::dir)
    }
}

/// Something that would drop unsaved work, held while the user is asked.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    Quit,
    Open,
    New { width: u32, height: u32, fps: f32 },
    /// Recover the orphan in this folder.
    Recover(PathBuf),
}

impl Pending {
    /// Finishes "Save changes to … before …?".
    pub fn verb(&self) -> &'static str {
        match self {
            Pending::Quit => "closing",
            Pending::Open => "opening another project",
            Pending::New { .. } => "starting a new project",
            Pending::Recover(_) => "recovering the autosave",
        }
    }
}

/// The unsaved-changes prompt's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsaved {
    Save,
    Discard,
    Cancel,
}

impl AppState {
    /// Whether the project differs from what was last loaded, started or
    /// saved. Hashes the project, so ask when it matters, not every frame.
    pub fn has_unsaved_changes(&self) -> bool {
        self.saved_key != Some(content_key(&self.project))
    }

    /// The project as it is now counts as saved: just loaded or started.
    pub(super) fn mark_saved(&mut self) {
        self.saved_key = Some(content_key(&self.project));
    }

    /// Do `p` now, or first ask about the unsaved changes it would drop.
    pub fn request(&mut self, p: Pending) {
        if self.has_unsaved_changes() {
            self.unsaved_prompt = Some(p);
        } else {
            self.perform(p);
        }
    }

    /// The answer to the unsaved-changes prompt.
    pub fn answer_unsaved(&mut self, a: Unsaved) {
        let Some(p) = self.unsaved_prompt.take() else {
            return;
        };
        match a {
            Unsaved::Save => {
                self.save_project();
                // Carried out once the write lands. No job: Save As was
                // cancelled, and so is `p`.
                if self.save_job.is_some() {
                    self.after_save = Some(p);
                }
            }
            Unsaved::Discard => self.perform(p),
            Unsaved::Cancel => {}
        }
    }

    /// Carry out `p` without asking.
    pub(super) fn perform(&mut self, p: Pending) {
        match p {
            Pending::Quit => {
                self.quit_ok = true;
                self.close_now = true;
            }
            Pending::Open => match project_file::load_dialog() {
                Ok(Some((project, path))) => self.load_project(project, Some(path)),
                Ok(None) => {}
                Err(e) => log::error!("Open project failed: {e:#}"),
            },
            Pending::New { width, height, fps } => self.reset_with(width, height, fps),
            Pending::Recover(dir) => self.recover_now(&dir),
        }
    }

    /// The window's close button, Alt+F4 or File ▸ Quit: let it through, or
    /// hold it for the unsaved-changes prompt. Called first thing each frame.
    pub(super) fn intercept_close(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.close_now) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.quit_ok || !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        // A save in flight would count as unsaved until it lands. Closing
        // waits for it anyway (`on_exit`), so wait here and ask about what
        // it really leaves.
        if self.after_save.is_none() {
            self.finish_pending_save();
        }
        if self.unsaved_prompt.is_some() || self.after_save.is_some() || self.has_unsaved_changes() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.unsaved_prompt.is_none() && self.after_save.is_none() {
                self.unsaved_prompt = Some(Pending::Quit);
            }
        }
    }

    /// A manual save landed: the autosave is redundant, and whatever was
    /// waiting on the save can go ahead.
    pub(super) fn saved(&mut self, key: u64) {
        self.saved_key = Some(key);
        self.clear_autosave();
        if let Some(p) = self.after_save.take() {
            self.perform(p);
        }
    }

    /// A manual save failed: what was waiting on it waits no more — the
    /// error dialog has the user's attention, and the work is still unsaved.
    pub(super) fn save_failed(&mut self) {
        self.after_save = None;
    }

    /// The project is about to be replaced: let an autosave of it land, then
    /// drop it. Its work was saved or deliberately thrown away by now.
    pub(super) fn clear_autosave(&mut self) {
        self.finish_autosave_job();
        if let Some(s) = &self.autosave.session {
            s.clear();
        }
        self.autosave.written = None;
        self.autosave.since = Instant::now();
    }

    /// Autosave when the interval is up and there is something new to keep.
    /// `busy`: a drag is under way, so the cells are mid-edit.
    ///
    /// Returns how long until it next wants a look, so an idle window still
    /// wakes for the autosave of the last thing drawn before the break.
    pub(super) fn tick_autosave(&mut self, now: Instant, busy: bool) -> Option<Duration> {
        self.poll_autosave_job();
        if self.autosave.session.is_none() || self.autosave.minutes == 0 {
            return None;
        }
        let every = Duration::from_secs(u64::from(self.autosave.minutes) * 60);
        let due = self.autosave.since + every;
        if now < due {
            return Some(due - now);
        }
        if busy || self.autosave.job.is_some() || self.save_job.is_some() {
            return Some(RETRY);
        }
        self.autosave.since = now;
        let key = content_key(&self.project);
        if Some(key) == self.saved_key {
            // Back to what is in the file, undo by undo: nothing to keep.
            if self.autosave.written.take().is_some() {
                if let Some(s) = &self.autosave.session {
                    s.clear();
                }
            }
            return Some(every);
        }
        if Some(key) == self.autosave.written {
            return Some(every);
        }
        let dir = self.autosave.session.as_ref().map(|s| s.dir().to_path_buf())?;
        // Cells are shared, so the copy is a reference per cell; drawing
        // goes on while the worker writes.
        let project = self.project.clone();
        let source = self.project_path.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(autosave::write(&dir, &project, source.as_deref()));
        });
        self.autosave.job = Some((rx, key));
        Some(every)
    }

    fn poll_autosave_job(&mut self) {
        let Some((rx, key)) = self.autosave.job.take() else {
            return;
        };
        match rx.try_recv() {
            Err(TryRecvError::Empty) => self.autosave.job = Some((rx, key)),
            Ok(Ok(())) => self.autosave.written = Some(key),
            Ok(Err(e)) => self.autosave_failed(format!("{e:#}")),
            Err(TryRecvError::Disconnected) => self.autosave_failed("the autosave worker stopped".into()),
        }
    }

    /// Wait for an autosave in flight.
    fn finish_autosave_job(&mut self) {
        if let Some((rx, key)) = self.autosave.job.take() {
            match rx.recv() {
                Ok(Ok(())) => self.autosave.written = Some(key),
                Ok(Err(e)) => self.autosave_failed(format!("{e:#}")),
                Err(_) => self.autosave_failed("the autosave worker stopped".into()),
            }
        }
    }

    /// Said, not shown as a modal: the work is no less saved than it was,
    /// and the next interval tries again.
    fn autosave_failed(&mut self, why: String) {
        log::error!("Autosave failed: {why}");
        self.save_toast = Some((format!("Autosave failed: {why}"), Instant::now() + Self::TOAST_TTL * 3));
    }

    /// The process is ending. A clean end removes the session; one with
    /// unsaved work nobody chose to drop — a shutdown that never asked —
    /// writes it out and leaves it for the next launch to offer.
    pub(super) fn end_autosave(&mut self) {
        self.finish_autosave_job();
        let Some(session) = self.autosave.session.take() else {
            return;
        };
        if !self.quit_ok && self.has_unsaved_changes() {
            if Some(content_key(&self.project)) != self.autosave.written {
                if let Err(e) = autosave::write(session.dir(), &self.project, self.project_path.as_deref()) {
                    log::error!("Last autosave failed: {e:#}");
                }
            }
            // Dropping it lets the lock go and keeps the folder.
            return;
        }
        session.end();
    }

    /// Load the orphan in `dir` in place of the current project.
    fn recover_now(&mut self, dir: &std::path::Path) {
        let Some(i) = self.autosave.orphans.iter().position(|o| o.dir == dir) else {
            return;
        };
        let orphan = self.autosave.orphans[i].clone();
        match project_file::load_from(&orphan.project()) {
            Ok(project) => {
                self.load_project(project, orphan.source.clone());
                // Not what is in the source file: that's why it was kept.
                self.saved_key = None;
                if let Some(s) = &self.autosave.session {
                    match autosave::adopt(&orphan, s) {
                        Ok(()) => self.autosave.written = Some(content_key(&self.project)),
                        Err(e) => log::warn!("Couldn't take over the recovered autosave: {e:#}"),
                    }
                }
                self.autosave.orphans.remove(i);
                self.autosave.show_recovery = !self.autosave.orphans.is_empty();
                self.save_toast = Some((format!("Recovered {}", orphan.name()), Instant::now() + Self::TOAST_TTL));
            }
            Err(e) => {
                self.autosave.error = Some(format!("Couldn't read the autosave of {}: {e:#}", orphan.name()));
                self.autosave.show_recovery = true;
            }
        }
    }

    /// Discard, twice: the first click arms it, the second deletes.
    pub fn discard_orphan(&mut self, dir: &std::path::Path) {
        if self.autosave.discard_armed.as_deref() != Some(dir) {
            self.autosave.discard_armed = Some(dir.to_path_buf());
            return;
        }
        self.autosave.discard_armed = None;
        let Some(i) = self.autosave.orphans.iter().position(|o| o.dir == dir) else {
            return;
        };
        match autosave::discard(&self.autosave.orphans[i]) {
            Ok(()) => {
                self.autosave.orphans.remove(i);
                self.autosave.show_recovery = !self.autosave.orphans.is_empty();
                self.autosave.error = None;
            }
            Err(e) => self.autosave.error = Some(format!("Couldn't delete it: {e:#}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::project::Project;
    use std::path::Path;
    use std::sync::Arc;

    /// A temp folder, removed when the test ends. Declared before the
    /// states that use it, so it is dropped after them: a live state holds
    /// its session's lock, and the folder can't go while it does.
    struct Scratch(PathBuf);

    impl std::ops::Deref for Scratch {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("animator-app-autosave-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    /// Paint one pixel of the current cell, as an edit would.
    fn paint(st: &mut AppState) {
        let id = st.project.layers[0].resolve(st.project.current_frame).unwrap();
        let px = Arc::make_mut(&mut st.project.cells[id]).pixels_mut();
        px[0] = px[0].wrapping_add(1);
        px[3] = 255;
    }

    /// Run the autosave timer past its interval and wait for the write.
    fn autosave_now(st: &mut AppState) {
        let later = st.autosave.since + Duration::from_secs(u64::from(st.autosave.minutes) * 60 + 1);
        st.tick_autosave(later, false);
        st.finish_autosave_job();
    }

    fn autosaved(st: &AppState) -> Option<Project> {
        let p = st.autosave.session_dir()?.join("project.anim");
        p.exists().then(|| project_file::load_from(&p).unwrap())
    }

    #[test]
    fn a_fresh_project_has_nothing_unsaved_and_an_edit_does() {
        let mut st = AppState::for_test();
        assert!(!st.has_unsaved_changes());
        paint(&mut st);
        assert!(st.has_unsaved_changes());
        st.reset_with(64, 64, 24.0);
        assert!(!st.has_unsaved_changes(), "a new project starts saved");
        st.load_project(Project::new(32, 32, 12.0), None);
        assert!(!st.has_unsaved_changes(), "a loaded project starts saved");
    }

    #[test]
    fn a_save_marks_the_work_saved_and_clears_the_autosave() {
        let root = scratch("save");
        let mut st = AppState::for_test();
        st.autosave.start_in(&root);
        paint(&mut st);
        autosave_now(&mut st);
        assert!(autosaved(&st).is_some());

        let file = root.join("saved.anim");
        st.write_project(file.clone());
        wait_save(&mut st);
        assert!(!st.has_unsaved_changes());
        assert!(autosaved(&st).is_none(), "the autosave outlived the save");
        assert_eq!(st.project_path.as_deref(), Some(file.as_path()));
    }

    #[test]
    fn autosave_waits_for_the_interval_and_for_a_change() {
        let root = scratch("timer");
        let mut st = AppState::for_test();
        st.autosave.start_in(&root);
        // Nothing unsaved: nothing written, however long it has been.
        autosave_now(&mut st);
        assert!(autosaved(&st).is_none());

        paint(&mut st);
        let soon = st.autosave.since + Duration::from_secs(5);
        let wait = st.tick_autosave(soon, false).unwrap();
        st.finish_autosave_job();
        assert!(autosaved(&st).is_none(), "wrote before the interval was up");
        assert!(wait <= Duration::from_secs(u64::from(st.autosave.minutes) * 60));

        // Due, but mid-drag: put off.
        let late = st.autosave.since + Duration::from_secs(10_000);
        assert_eq!(st.tick_autosave(late, true), Some(RETRY));
        st.finish_autosave_job();
        assert!(autosaved(&st).is_none(), "wrote in the middle of a drag");

        autosave_now(&mut st);
        let back = autosaved(&st).expect("autosave");
        assert_eq!(back.cells.len(), st.project.cells.len());
    }

    #[test]
    fn autosave_off_writes_nothing() {
        let root = scratch("off");
        let mut st = AppState::for_test();
        st.autosave.start_in(&root);
        st.autosave.minutes = 0;
        paint(&mut st);
        assert_eq!(st.tick_autosave(Instant::now() + Duration::from_secs(100_000), false), None);
        st.finish_autosave_job();
        assert!(autosaved(&st).is_none());
    }

    #[test]
    fn undoing_back_to_the_saved_state_drops_the_autosave() {
        let root = scratch("undo");
        let mut st = AppState::for_test();
        st.autosave.start_in(&root);
        let id = st.project.layers[0].resolve(0).unwrap();
        let saved_cell = st.project.cells[id].clone();
        paint(&mut st);
        autosave_now(&mut st);
        assert!(autosaved(&st).is_some());
        // The pixels as they were: what an undo puts back.
        st.project.cells[id] = saved_cell;
        autosave_now(&mut st);
        assert!(autosaved(&st).is_none());
    }

    #[test]
    fn a_crash_leaves_work_the_next_run_recovers() {
        let root = scratch("crash");
        let mut st = AppState::for_test();
        st.autosave.start_in(&root);
        st.project.layers[0].name = "Before the crash".into();
        let source = root.join("scene.anim");
        st.project_path = Some(source.clone());
        autosave_now(&mut st);
        // A crash: no end_autosave, the lock simply goes.
        drop(st);

        let mut next = AppState::for_test();
        next.autosave.start_in(&root);
        assert!(next.autosave.show_recovery);
        let dir = next.autosave.orphans[0].dir.clone();
        next.request(Pending::Recover(dir.clone()));
        assert_eq!(next.project.layers[0].name, "Before the crash");
        assert_eq!(next.project_path.as_deref(), Some(source.as_path()));
        assert!(next.has_unsaved_changes(), "recovered work is not in its file");
        assert!(!next.autosave.show_recovery);
        assert!(!dir.exists(), "the orphan folder was left");
        // Taken over: this run's autosave holds it until a save.
        assert_eq!(autosaved(&next).unwrap().layers[0].name, "Before the crash");
    }

    #[test]
    fn recovering_over_unsaved_work_asks_first() {
        let root = scratch("ask");
        let mut first = AppState::for_test();
        first.autosave.start_in(&root);
        paint(&mut first);
        autosave_now(&mut first);
        drop(first);

        let mut st = AppState::for_test();
        st.autosave.start_in(&root);
        paint(&mut st);
        let dir = st.autosave.orphans[0].dir.clone();
        st.request(Pending::Recover(dir.clone()));
        assert_eq!(st.unsaved_prompt, Some(Pending::Recover(dir.clone())));
        st.answer_unsaved(Unsaved::Cancel);
        assert!(st.unsaved_prompt.is_none());
        assert!(dir.exists(), "cancel recovered anyway");
        st.request(Pending::Recover(dir.clone()));
        st.answer_unsaved(Unsaved::Discard);
        assert!(!dir.exists());
    }

    #[test]
    fn discarding_an_orphan_takes_two_clicks() {
        let root = scratch("discard");
        let mut first = AppState::for_test();
        first.autosave.start_in(&root);
        paint(&mut first);
        autosave_now(&mut first);
        drop(first);

        let mut st = AppState::for_test();
        st.autosave.start_in(&root);
        let dir = st.autosave.orphans[0].dir.clone();
        st.discard_orphan(&dir);
        assert!(dir.exists(), "one click deleted it");
        st.discard_orphan(&dir);
        assert!(!dir.exists());
        assert!(st.autosave.orphans.is_empty());
        assert!(!st.autosave.show_recovery);
    }

    #[test]
    fn a_clean_exit_leaves_nothing_and_an_unasked_one_keeps_the_work() {
        let root = scratch("exit");
        let mut clean = AppState::for_test();
        clean.autosave.start_in(&root);
        paint(&mut clean);
        clean.request(Pending::Quit);
        clean.answer_unsaved(Unsaved::Discard);
        assert!(clean.quit_ok);
        clean.end_autosave();
        assert!(autosave::orphans(&root).is_empty());

        let mut unasked = AppState::for_test();
        unasked.autosave.start_in(&root);
        paint(&mut unasked);
        unasked.end_autosave();
        drop(unasked);
        let found = autosave::orphans(&root);
        assert_eq!(found.len(), 1, "unsaved work vanished on an exit nobody confirmed");
    }

    #[test]
    fn quitting_with_unsaved_work_waits_for_the_answer() {
        let mut st = AppState::for_test();
        st.request(Pending::Quit);
        assert!(st.quit_ok, "nothing unsaved: quit at once");

        let mut st = AppState::for_test();
        paint(&mut st);
        st.request(Pending::Quit);
        assert!(!st.quit_ok && !st.close_now);
        assert_eq!(st.unsaved_prompt, Some(Pending::Quit));
        st.answer_unsaved(Unsaved::Cancel);
        assert!(!st.quit_ok);
    }

    #[test]
    fn save_then_quit_quits_once_the_save_lands() {
        let root = scratch("save-quit");
        let mut st = AppState::for_test();
        paint(&mut st);
        st.project_path = Some(root.join("q.anim"));
        st.request(Pending::Quit);
        st.answer_unsaved(Unsaved::Save);
        assert!(!st.close_now, "closed before the save landed");
        wait_save(&mut st);
        assert!(st.close_now && st.quit_ok);
        assert!(Path::new(&root.join("q.anim")).exists());
    }

    /// Let the background save land through the same polling the frame loop
    /// does.
    fn wait_save(st: &mut AppState) {
        for _ in 0..2000 {
            st.poll_save_job();
            if st.save_job.is_none() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("the save never landed");
    }

    /// A frame in which the window was told to close (its X, Alt+F4, or
    /// the Close that File ▸ Quit sends).
    fn close_input() -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        raw
    }

    fn sent(out: &egui::FullOutput, cmd: egui::ViewportCommand) -> bool {
        out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&cmd))
    }

    #[test]
    fn a_close_with_unsaved_work_is_held_for_the_prompt() {
        let mut st = AppState::for_test();
        paint(&mut st);
        let ctx = egui::Context::default();
        let out = ctx.run(close_input(), |ctx| st.intercept_close(ctx));
        assert!(sent(&out, egui::ViewportCommand::CancelClose), "the window closed over unsaved work");
        assert_eq!(st.unsaved_prompt, Some(Pending::Quit));
        // Closing again while it asks: still held, still one prompt.
        let out = ctx.run(close_input(), |ctx| st.intercept_close(ctx));
        assert!(sent(&out, egui::ViewportCommand::CancelClose));

        st.answer_unsaved(Unsaved::Discard);
        let out = ctx.run(egui::RawInput::default(), |ctx| st.intercept_close(ctx));
        assert!(sent(&out, egui::ViewportCommand::Close), "Don't save didn't close");
        let out = ctx.run(close_input(), |ctx| st.intercept_close(ctx));
        assert!(!sent(&out, egui::ViewportCommand::CancelClose), "the confirmed close was held");
    }

    #[test]
    fn a_close_with_nothing_unsaved_goes_straight_through() {
        let mut st = AppState::for_test();
        let ctx = egui::Context::default();
        let out = ctx.run(close_input(), |ctx| st.intercept_close(ctx));
        assert!(!sent(&out, egui::ViewportCommand::CancelClose));
        assert!(st.unsaved_prompt.is_none());
    }

    #[test]
    fn a_close_waits_for_a_save_in_flight_instead_of_asking() {
        let root = scratch("close-saving");
        let mut st = AppState::for_test();
        paint(&mut st);
        st.write_project(root.join("c.anim"));
        let ctx = egui::Context::default();
        let out = ctx.run(close_input(), |ctx| st.intercept_close(ctx));
        assert!(!sent(&out, egui::ViewportCommand::CancelClose), "asked about work being saved");
        assert!(st.save_job.is_none());
    }

    /// What every frame does to the project without anyone editing —
    /// buffers following keys, cells packed and collected, the UI drawn —
    /// must not read as unsaved work, or every close would ask.
    #[test]
    fn frames_and_scrubbing_are_not_edits() {
        use crate::doc::camera::Camera;
        use crate::doc::transform::Transform;
        let mut p = Project::new(320, 240, 24.0);
        p.insert_frames_before(5);
        p.layers[0].set_transform_key(0, Transform { tx: 3.0, ..Transform::default() });
        p.layers[0].set_transform_key(5, Transform { tx: 30.0, rot: 0.4, ..Transform::default() });
        p.set_camera_key(0, Camera::default());
        p.set_camera_key(5, Camera { tx: 10.0, rot: 0.2, ..Camera::default() });
        let mut st = AppState::for_test();
        st.load_project(p, None);
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        for f in [0, 3, 5, 2, 4, 1] {
            st.project.current_frame = f;
            st.track_layer_change();
            st.sync_active_transform_buffer();
            st.sync_selection();
            st.sync_liquify();
            st.sync_camera_buffer();
            st.hold_camera_inside();
            st.sync_grid_buffers();
            st.collect_cells();
            st.pack_idle_cells();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| crate::ui::shell::draw(&mut st, ctx));
            assert!(!st.has_unsaved_changes(), "frame {f} read as an edit");
        }
    }

    #[test]
    fn a_new_project_over_unsaved_work_asks_first() {
        let mut st = AppState::for_test();
        paint(&mut st);
        st.request(Pending::New { width: 100, height: 50, fps: 12.0 });
        assert_eq!(st.project.width, 1280, "replaced without asking");
        st.answer_unsaved(Unsaved::Discard);
        assert_eq!((st.project.width, st.project.height), (100, 50));
    }
}
