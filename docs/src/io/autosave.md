# `src/io/autosave.rs` — the autosave folders, and finding what a crash left

> **Source:** [src/io/autosave.rs](../../../src/io/autosave.rs) · **Module:** `crate::io::autosave` · **Size:** 336 lines · Line links as of commit `bdc0f67`

## What this file is for

This file knows where autosaves live on disk and how to tell work a crashed run left behind from the
autosave of an app that is still running. *When* to autosave is decided in [app/autosave.rs](../app/autosave.md).

Each running app owns one **session folder**, `%APPDATA%\animator-app\autosave\<millis>-<pid>-<n>\`:

| File | What it is |
|---|---|
| `lock` | Held open for as long as the app runs, shared with nobody. |
| `project.anim` | The latest autosave: an ordinary project file. |
| `source.txt` | The path of the project's own `.anim`; empty for one never saved. |

**Is it alive?** On Windows a file can be opened *unshared* (`share_mode(0)`): while one process holds it, any
other open fails with a *sharing violation* (OS error 32). Each app holds its `lock` that way, and the OS closes
the handle however the app ends — a crash included. So another app opening the same `lock` learns at once whether
its owner is still running (`is_live`).

An **orphan** is a session folder whose lock nobody holds and that still has a `project.anim`. A clean exit
removes its folder and a save clears the autosave inside, so an orphan always holds work that is in no file.

Files are written with [`project_file::write_replacing`](project_file.md#write_replacing) — to a temporary name,
flushed, then renamed over the old one — so a crash mid-autosave leaves the previous autosave readable.

A web analogy: one folder per open browser tab, each with a heartbeat; a folder whose heartbeat stopped is a tab
that crashed, and its draft is offered back.

## How it connects

- [src/app/autosave.rs](../app/autosave.md) starts the session, writes autosaves through `write` on a worker
  thread, lists `orphans` at launch, and calls `adopt` / `discard` from the recovery dialog.
- Uses [src/io/project_file.rs](project_file.md) (`save_to`, `write_replacing`) and
  [src/input/shortcuts.rs](../input/shortcuts.md) (`config_dir`, the app's settings folder).

## Types

### `const LOCK`
[src/io/autosave.rs:32](../../../src/io/autosave.rs#L32)

```rust
const LOCK: &str = "lock";
```

Name of the lock file each running app holds open in its session folder.

### `const PROJECT`
[src/io/autosave.rs:33](../../../src/io/autosave.rs#L33)

```rust
const PROJECT: &str = "project.anim";
```

Name of the autosaved project in a session folder: an ordinary `.anim`.

### `const SOURCE`
[src/io/autosave.rs:34](../../../src/io/autosave.rs#L34)

```rust
const SOURCE: &str = "source.txt";
```

Name of the text file holding the path of the project's own `.anim`; empty when it has none.

### `struct Session`
[src/io/autosave.rs:47](../../../src/io/autosave.rs#L47)

This run's folder, and the lock that marks it as alive.

| Field | Type | Meaning |
|---|---|---|
| `dir` | `PathBuf` | This run's folder: `<config>/autosave/<millis>-<pid>-<n>`. |
| `_lock` | `File` | Never read: holding it open is the whole point. |

### `struct Orphan`
[src/io/autosave.rs:104](../../../src/io/autosave.rs#L104)

Work an earlier run left behind.

| Field | Type | Meaning |
|---|---|---|
| `dir` | `PathBuf` | The left-behind session folder. |
| `source` | `Option<PathBuf>` | The file it came from; `None` for a project never saved. |
| `saved` | `SystemTime` | When it was autosaved. |
| `bytes` | `u64` | Size of the autosaved project, shown in the dialog. |

## Free functions

### `root`
[src/io/autosave.rs:39](../../../src/io/autosave.rs#L39)

```rust
pub fn root() -> Option<PathBuf>
```

The folder every session folder goes in. `None` under test, as for every file the app keeps between runs: a test must never write into, or recover from, the folder of the person running it.

## `impl Session`

### `Session::start`
[src/io/autosave.rs:55](../../../src/io/autosave.rs#L55)

```rust
pub fn start(root: &Path) -> Result<Session>
```

A new, empty session folder under `root`, locked.

### `Session::dir`
[src/io/autosave.rs:67](../../../src/io/autosave.rs#L67)

```rust
pub fn dir(&self) -> &Path
```

The folder: for `write` on a worker thread, and for tests.

### `Session::clear`
[src/io/autosave.rs:73](../../../src/io/autosave.rs#L73)

```rust
pub fn clear(&self)
```

Drop the autosave, keeping the session: the work it held is now in its file, or was thrown away on purpose.

### `Session::end`
[src/io/autosave.rs:81](../../../src/io/autosave.rs#L81)

```rust
pub fn end(self)
```

The clean end of a run: the folder goes, so nothing is offered for recovery next time.

## Free functions

### `write`
[src/io/autosave.rs:96](../../../src/io/autosave.rs#L96)

```rust
pub fn write(dir: &Path, project: &Project, source: Option<&Path>) -> Result<()>
```

Autosave `project` into the session folder `dir`, noting `source` beside it. Takes the folder rather than the `Session` so a worker thread can run it while the session stays with the app.

Both files are replaced whole (see `project_file::write_replacing`), so a crash mid-autosave leaves the previous autosave readable.

## `impl Orphan`

### `Orphan::project`
[src/io/autosave.rs:115](../../../src/io/autosave.rs#L115)

```rust
pub fn project(&self) -> PathBuf
```

The autosaved project file.

### `Orphan::name`
[src/io/autosave.rs:120](../../../src/io/autosave.rs#L120)

```rust
pub fn name(&self) -> String
```

What to call it: the source file's name, or "Untitled".

### `Orphan::source_saved`
[src/io/autosave.rs:128](../../../src/io/autosave.rs#L128)

```rust
pub fn source_saved(&self) -> Option<SystemTime>
```

When the source file was last written, if it still exists.

## Free functions

### `orphans`
[src/io/autosave.rs:136](../../../src/io/autosave.rs#L136)

```rust
pub fn orphans(root: &Path) -> Vec<Orphan>
```

Every orphan under `root`, newest first. Dead session folders with no autosave in them (a run that ended before its first one) are removed on the way.

### `adopt`
[src/io/autosave.rs:170](../../../src/io/autosave.rs#L170)

```rust
pub fn adopt(orphan: &Orphan, session: &Session) -> Result<()>
```

Take over an orphan the app has just loaded: its autosave becomes this session's, so the recovered work stays safe until the next autosave or save, and its folder goes. Called after the load, which clears the session — in the other order the clear would delete what was adopted.

### `discard`
[src/io/autosave.rs:181](../../../src/io/autosave.rs#L181)

```rust
pub fn discard(orphan: &Orphan) -> Result<()>
```

Delete an orphan for good.

### `hold_lock`
[src/io/autosave.rs:187](../../../src/io/autosave.rs#L187)

```rust
fn hold_lock(path: &Path) -> std::io::Result<File>
```

Open the lock file so that no one else can open it while we hold it.

### `hold_lock`
[src/io/autosave.rs:193](../../../src/io/autosave.rs#L193)

```rust
fn hold_lock(path: &Path) -> std::io::Result<File>
```

Elsewhere: just create the file. There is no unshared open, so `is_live` goes by the process id in the folder's name instead.

### `is_live`
[src/io/autosave.rs:199](../../../src/io/autosave.rs#L199)

```rust
fn is_live(dir: &Path) -> bool
```

Whether the app that made session folder `dir` is still running.

### `is_live`
[src/io/autosave.rs:213](../../../src/io/autosave.rs#L213)

```rust
fn is_live(dir: &Path) -> bool
```

Elsewhere there is no unshared open: go by the process id in the folder's name, where `/proc` can answer for it. Without `/proc` every other folder counts as live — offering a running app's work for recovery would be worse than offering none.

## Tests

### `mod tests`

Run with `cargo test io::autosave::tests` · 7 tests

| Test | What it checks |
|---|---|
| [`tests_never_find_the_real_autosave_folder`](../../../src/io/autosave.rs#L239) | Tests never find the real autosave folder. |
| [`a_live_session_is_not_an_orphan_and_a_dead_one_is`](../../../src/io/autosave.rs#L244) | A live session is not an orphan and a dead one is. |
| [`the_source_file_comes_back_with_the_orphan`](../../../src/io/autosave.rs#L263) | The source file comes back with the orphan. |
| [`a_dead_session_with_nothing_saved_is_tidied_away`](../../../src/io/autosave.rs#L276) | A dead session with nothing saved is tidied away. |
| [`a_clean_end_and_a_clear_leave_nothing_to_recover`](../../../src/io/autosave.rs#L287) | A clean end and a clear leave nothing to recover. |
| [`adopting_moves_the_autosave_into_this_session`](../../../src/io/autosave.rs#L301) | Adopting moves the autosave into this session. |
| [`orphans_come_newest_first`](../../../src/io/autosave.rs#L319) | Orphans come newest first. |

**Test helpers** (used only by the tests above):

- [`scratch`](../../../src/io/autosave.rs#L225) — Scratch.
- [`named`](../../../src/io/autosave.rs#L232) — Named.
