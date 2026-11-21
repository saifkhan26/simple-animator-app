# `src/app/autosave.rs` — autosave, crash recovery and the unsaved-changes prompt

> **Source:** [src/app/autosave.rs](../../../src/app/autosave.rs) · **Module:** `crate::app::autosave` · **Size:** 741 lines · Line links as of commit `84b26e8`

## What this file is for

This file keeps unsaved work from being lost. It does three jobs:

- **Timed autosave.** Every few minutes (Settings ▸ Files ▸ *Autosave every*, default 3, 0 = off) it copies
  the project to a separate autosave file on a worker thread — never over your own `.anim`. It writes only when
  something changed since the last save or autosave, and waits while a drag (or a pen stroke) is under way.
- **Recovery.** At launch it looks for autosaves an earlier run left behind — the app crashed, was killed, or
  Windows shut down without asking — and offers them in the *Recover unsaved work* dialog: Recover, Discard
  (two clicks), or Later.
- **The unsaved-changes prompt.** Closing the window (its X, Alt+F4, File ▸ Quit), File ▸ Open, New project ▸
  Create and Recover all first ask *"Save changes to … before …?"* when there is unsaved work.

**What "unsaved" means.** Two numbers are compared: the project's *content key* now
([`project_file::content_key`](../io/project_file.md#content_key)) and the one taken when the project was last
loaded, started or saved (`AppState::saved_key`). The key hashes what the file would hold, but uses each drawing's
`rev` (a number that changes on every pixel write) instead of its pixels, so it is cheap to take and never reads a
pixel. Undoing back to the saved state makes the two equal again, so the prompt doesn't ask.

A web analogy: a text editor in a browser tab that keeps a draft in `localStorage` every few minutes, offers it
back after the tab crashed, and shows *"Leave site? Changes you made may not be saved"* on `beforeunload`.

This file decides *when*; the folders and files themselves are [io/autosave.rs](../io/autosave.md)'s job. It is a
**child module of `src/app.rs`**, so its methods on `AppState` can use the app's private fields.

## How it connects

- [src/app.rs](../app.md): `AppState::update` runs `intercept_close` first thing each frame and `tick_autosave`
  after the save poll; `on_exit` calls `end_autosave`; `reset_with` and `load_project` call `mark_saved` and
  `clear_autosave`; `poll_save_job` calls `saved` or `save_failed`; the Open shortcut goes through `request`.
- [src/ui/shell.rs](../ui/shell.md): File ▸ Open and the New project dialog's Create go through `request`;
  `unsaved_dialog` and `recovery_dialog` draw the two prompts; the Settings window edits `Autosave::minutes`.
- Uses [src/io/autosave.rs](../io/autosave.md) (session folders, orphans) and
  [src/io/project_file.rs](../io/project_file.md) (`content_key`, `load_from`, `load_dialog`).

## Types

### `const DEFAULT_AUTOSAVE_MINUTES`
[src/app/autosave.rs:24](../../../src/app/autosave.rs#L24)

```rust
pub const DEFAULT_AUTOSAVE_MINUTES: u32 = 3;
```

Minutes between autosaves when the prefs don't say.

### `const RETRY`
[src/app/autosave.rs:26](../../../src/app/autosave.rs#L26)

```rust
const RETRY: Duration = Duration::from_secs(2);
```

How soon to try again when an autosave falls due mid-drag or mid-save.

### `struct Autosave`
[src/app/autosave.rs:29](../../../src/app/autosave.rs#L29)

Autosave's state on `AppState`.

| Field | Type | Meaning |
|---|---|---|
| `minutes` | `u32` | Minutes between autosaves; 0 = off. A preference. |
| `session` | `Option<Session>` | This run's folder. `None` under test, or when it couldn't be made. |
| `job` | `Option<(Receiver<Result<()>>, u64)>` | A write in flight, and the content key of what it writes. |
| `written` | `Option<u64>` | Content key of what the session's autosave file holds, if it has one. |
| `since` | `Instant` | When the interval last started. |
| `orphans` | `Vec<Orphan>` | Work earlier runs left behind, offered at launch. |
| `show_recovery` | `bool` | The recovery dialog is up. |
| `discard_armed` | `Option<PathBuf>` | The orphan whose Discard was clicked once; a second click deletes it. |
| `error` | `Option<String>` | Why the last recover or discard failed, shown in the dialog. |

### `enum Pending`
[src/app/autosave.rs:94](../../../src/app/autosave.rs#L94)

Something that would drop unsaved work, held while the user is asked.

| Variant | Meaning |
|---|---|
| `Quit` | Close the app: the window's X, Alt+F4, or File ▸ Quit. |
| `Open` | File ▸ Open project… (or its shortcut). |
| `New { width: u32, height: u32, fps: f32 }` | New project ▸ Create, with the size and frame rate picked. |
| `Recover(PathBuf)` | Recover the orphan in this folder. |

### `enum Unsaved`
[src/app/autosave.rs:116](../../../src/app/autosave.rs#L116)

The unsaved-changes prompt's buttons.

| Variant | Meaning |
|---|---|
| `Save` | Save first; carry on once the save lands (nothing happens if Save As is cancelled). |
| `Discard` | Carry on and drop the unsaved work. |
| `Cancel` | Do nothing and keep working (also Esc). |

## `impl Autosave`

### `Autosave::new`
[src/app/autosave.rs:51](../../../src/app/autosave.rs#L51)

```rust
pub fn new(minutes: u32) -> Self
```

The autosave state at startup: the interval from the prefs, no session yet (`start` makes one), nothing written. Tests get this and never call `start`, so they never touch the real folder.

### `Autosave::start`
[src/app/autosave.rs:67](../../../src/app/autosave.rs#L67)

```rust
pub fn start(&mut self)
```

Find what earlier runs left, then start this run's session. In that order, so this run's own folder is never among the orphans.

### `Autosave::start_in`
[src/app/autosave.rs:80](../../../src/app/autosave.rs#L80)

```rust
pub fn start_in(&mut self, root: &std::path::Path)
```

Test-only `start`, in a folder the test owns: lists that folder's orphans, then starts a session in it.

### `Autosave::session_dir`
[src/app/autosave.rs:87](../../../src/app/autosave.rs#L87)

```rust
pub fn session_dir(&self) -> Option<&std::path::Path>
```

Test-only: the session folder, to look at what was written.

## `impl Pending`

### `Pending::verb`
[src/app/autosave.rs:104](../../../src/app/autosave.rs#L104)

```rust
pub fn verb(&self) -> &'static str
```

Finishes "Save changes to … before …?".

## `impl AppState`

### `AppState::has_unsaved_changes`
[src/app/autosave.rs:125](../../../src/app/autosave.rs#L125)

```rust
pub fn has_unsaved_changes(&self) -> bool
```

Whether the project differs from what was last loaded, started or saved. Hashes the project, so ask when it matters, not every frame.

### `AppState::mark_saved`
[src/app/autosave.rs:130](../../../src/app/autosave.rs#L130)

```rust
pub(super) fn mark_saved(&mut self)
```

The project as it is now counts as saved: just loaded or started.

### `AppState::request`
[src/app/autosave.rs:135](../../../src/app/autosave.rs#L135)

```rust
pub fn request(&mut self, p: Pending)
```

Do `p` now, or first ask about the unsaved changes it would drop.

### `AppState::answer_unsaved`
[src/app/autosave.rs:144](../../../src/app/autosave.rs#L144)

```rust
pub fn answer_unsaved(&mut self, a: Unsaved)
```

The answer to the unsaved-changes prompt.

### `AppState::perform`
[src/app/autosave.rs:163](../../../src/app/autosave.rs#L163)

```rust
pub(super) fn perform(&mut self, p: Pending)
```

Carry out `p` without asking.

### `AppState::intercept_close`
[src/app/autosave.rs:181](../../../src/app/autosave.rs#L181)

```rust
pub(super) fn intercept_close(&mut self, ctx: &egui::Context)
```

The window's close button, Alt+F4 or File ▸ Quit: let it through, or hold it for the unsaved-changes prompt. Called first thing each frame.

### `AppState::saved`
[src/app/autosave.rs:205](../../../src/app/autosave.rs#L205)

```rust
pub(super) fn saved(&mut self, key: u64)
```

A manual save landed: the autosave is redundant, and whatever was waiting on the save can go ahead.

### `AppState::save_failed`
[src/app/autosave.rs:215](../../../src/app/autosave.rs#L215)

```rust
pub(super) fn save_failed(&mut self)
```

A manual save failed: what was waiting on it waits no more — the error dialog has the user's attention, and the work is still unsaved.

### `AppState::clear_autosave`
[src/app/autosave.rs:221](../../../src/app/autosave.rs#L221)

```rust
pub(super) fn clear_autosave(&mut self)
```

The project is about to be replaced: let an autosave of it land, then drop it. Its work was saved or deliberately thrown away by now.

### `AppState::tick_autosave`
[src/app/autosave.rs:235](../../../src/app/autosave.rs#L235)

```rust
pub(super) fn tick_autosave(&mut self, now: Instant, busy: bool) -> Option<Duration>
```

Autosave when the interval is up and there is something new to keep. `busy`: a drag is under way, so the cells are mid-edit.

Returns how long until it next wants a look, so an idle window still wakes for the autosave of the last thing drawn before the break.

### `AppState::poll_autosave_job`
[src/app/autosave.rs:275](../../../src/app/autosave.rs#L275)

```rust
fn poll_autosave_job(&mut self)
```

Each frame: pick up a finished autosave write without waiting (`try_recv`, like a `Promise` checked rather than awaited). Success records the key it wrote in `written`; a failure goes to `autosave_failed`.

### `AppState::finish_autosave_job`
[src/app/autosave.rs:288](../../../src/app/autosave.rs#L288)

```rust
fn finish_autosave_job(&mut self)
```

Wait for an autosave in flight.

### `AppState::autosave_failed`
[src/app/autosave.rs:300](../../../src/app/autosave.rs#L300)

```rust
fn autosave_failed(&mut self, why: String)
```

Said, not shown as a modal: the work is no less saved than it was, and the next interval tries again.

### `AppState::end_autosave`
[src/app/autosave.rs:308](../../../src/app/autosave.rs#L308)

```rust
pub(super) fn end_autosave(&mut self)
```

The process is ending. A clean end removes the session; one with unsaved work nobody chose to drop — a shutdown that never asked — writes it out and leaves it for the next launch to offer.

### `AppState::recover_now`
[src/app/autosave.rs:326](../../../src/app/autosave.rs#L326)

```rust
fn recover_now(&mut self, dir: &std::path::Path)
```

Load the orphan in `dir` in place of the current project.

### `AppState::discard_orphan`
[src/app/autosave.rs:354](../../../src/app/autosave.rs#L354)

```rust
pub fn discard_orphan(&mut self, dir: &std::path::Path)
```

Discard, twice: the first click arms it, the second deletes.

## Tests

### `mod tests`

Run with `cargo test app::autosave::tests` · 16 tests

| Test | What it checks |
|---|---|
| [`a_fresh_project_has_nothing_unsaved_and_an_edit_does`](../../../src/app/autosave.rs#L427) | A fresh project has nothing unsaved and an edit does. |
| [`a_save_marks_the_work_saved_and_clears_the_autosave`](../../../src/app/autosave.rs#L439) | A save marks the work saved and clears the autosave. |
| [`autosave_waits_for_the_interval_and_for_a_change`](../../../src/app/autosave.rs#L456) | Autosave waits for the interval and for a change. |
| [`autosave_off_writes_nothing`](../../../src/app/autosave.rs#L483) | Autosave off writes nothing. |
| [`undoing_back_to_the_saved_state_drops_the_autosave`](../../../src/app/autosave.rs#L495) | Undoing back to the saved state drops the autosave. |
| [`a_crash_leaves_work_the_next_run_recovers`](../../../src/app/autosave.rs#L511) | A crash leaves work the next run recovers. |
| [`recovering_over_unsaved_work_asks_first`](../../../src/app/autosave.rs#L537) | Recovering over unsaved work asks first. |
| [`discarding_an_orphan_takes_two_clicks`](../../../src/app/autosave.rs#L560) | Discarding an orphan takes two clicks. |
| [`a_clean_exit_leaves_nothing_and_an_unasked_one_keeps_the_work`](../../../src/app/autosave.rs#L580) | A clean exit leaves nothing and an unasked one keeps the work. |
| [`quitting_with_unsaved_work_waits_for_the_answer`](../../../src/app/autosave.rs#L601) | Quitting with unsaved work waits for the answer. |
| [`save_then_quit_quits_once_the_save_lands`](../../../src/app/autosave.rs#L616) | Save then quit quits once the save lands. |
| [`a_close_with_unsaved_work_is_held_for_the_prompt`](../../../src/app/autosave.rs#L655) | A close with unsaved work is held for the prompt. |
| [`a_close_with_nothing_unsaved_goes_straight_through`](../../../src/app/autosave.rs#L674) | A close with nothing unsaved goes straight through. |
| [`a_close_waits_for_a_save_in_flight_instead_of_asking`](../../../src/app/autosave.rs#L683) | A close waits for a save in flight instead of asking. |
| [`frames_and_scrubbing_are_not_edits`](../../../src/app/autosave.rs#L698) | What every frame does to the project without anyone editing — buffers following keys, cells packed and collected, the UI drawn — must not read as unsaved work, or every close would ask. |
| [`a_new_project_over_unsaved_work_asks_first`](../../../src/app/autosave.rs#L732) | A new project over unsaved work asks first. |

**Test helpers** (used only by the tests above):

- [`deref`](../../../src/app/autosave.rs#L388) — Deref.
- [`drop`](../../../src/app/autosave.rs#L394) — Drop.
- [`scratch`](../../../src/app/autosave.rs#L399) — Scratch.
- [`paint`](../../../src/app/autosave.rs#L407) — Paint one pixel of the current cell, as an edit would.
- [`autosave_now`](../../../src/app/autosave.rs#L415) — Run the autosave timer past its interval and wait for the write.
- [`autosaved`](../../../src/app/autosave.rs#L421) — Autosaved.
- [`wait_save`](../../../src/app/autosave.rs#L631) — Let the background save land through the same polling the frame loop does.
- [`close_input`](../../../src/app/autosave.rs#L644) — A frame in which the window was told to close (its X, Alt+F4, or the Close that File ▸ Quit sends).
- [`sent`](../../../src/app/autosave.rs#L650) — Sent.

**Test-only types and constants:** [`Scratch`](../../../src/app/autosave.rs#L384), [`Target`](../../../src/app/autosave.rs#L387)
