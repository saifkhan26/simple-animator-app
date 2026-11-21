# `src/app/grids.rs` — perspective grids on the app side (per-file store, follow, keys, drags)

> **Source:** [src/app/grids.rs](../../../src/app/grids.rs) · **Module:** `crate::app::grids` · **Size:** 912 lines · Line links as of commit `84b26e8`

## What this file is for

A **perspective grid** is a drawing guide laid over the canvas: a four-cornered shape
(a *quad*) whose edges point at *vanishing points* — the spots where parallel lines seem to
meet in a perspective drawing. Strokes can snap to its lines. There are also flat (square)
and isometric grids, and **walls**: grids that stand up on one edge of another grid (their
*floor*, or parent) and are rebuilt from it. The grid geometry itself lives in
[tools/perspective.rs](../tools/perspective.md); this file is the *app-side* glue:

1. **Which file's grids are live.** Grids are *never* saved inside the `.anim` project file.
   Instead the app remembers one set of grids per project file, keyed by the file's path,
   in the workspace preferences (`GridFile` entries). Opening a file brings its grids
   back, a new unsaved project starts with one default grid, and Save As copies the grids
   to the new path.
2. **What a grid follows.** A grid can stay on the document, ride along with the camera,
   or ride along with a layer's transform (its move/rotate/scale). This file works out where
   the grid lands on a given frame (`grid_space`) and re-links grids to their layer.
3. **Keys.** A grid can be animated: *keys* are saved poses on particular frames, and
   frames between keys are *tweened* (interpolated). This file adds, deletes and eases keys
   as undoable steps.
4. **Canvas drags.** With the Perspective tool, pressing and dragging on a grid moves it,
   reshapes a corner, drags a vanishing point, and so on (`perspective_down`,
   `perspective_move`, `finish_grid_drag`), with the whole drag recorded as one undo step.

Like the other files in `src/app/`, it is a child module of [src/app.rs](../app.md) that adds
methods to `AppState` through its own `impl AppState` block (see the explanation on the
[brush_sizes page](brush_sizes.md#rust-concepts-in-this-file)).

## How it connects

- **Used by:**
  - [src/app.rs](../app.md) — stores the grid state (`perspective`, `grid_files`,
    `grid_seed`, `grid_drag`, …); calls `stash_grids` / `unstash_grids` when a project is
    reset or loaded (`reset_with`, `load_project`), `grids_saved_as` after a save
    (`poll_save_job`), `grid_files_for_prefs` when saving preferences (`save`),
    `sync_grid_buffers` every frame (`update`), `restore_grid` on undo/redo
    (`apply_touched`), `finish_grid_drag` in `pointer_up`, `cycle_perspective_grid` for
    the `PerspectiveNextGrid` shortcut, and `grid_space` for shape snapping
    (`shape_plane`, `slide_shape`, `snap_doc`). Most grid tests also live in `app.rs`.
  - [src/ui/shell.rs](../ui/shell.md) — the canvas input calls `perspective_down` /
    `perspective_move`; the grid settings panel (`grid_settings`) calls `set_grid_follow`,
    `follow_layer`, `grid_pose_edited`, `add_grid_key`, `delete_grid_key`,
    `set_grid_key_ease`; grid drawing (`draw_perspective_grids`, `draw_cursor_guides`) calls
    `grid_space`.
  - [src/app/select.rs](select.md) — `set_tool` calls `ensure_perspective_grid` when the
    Perspective tool is picked.
- **Uses:**
  - [tools/perspective.rs](../tools/perspective.md) — `PerspectiveGrid`, `PerspectiveConfig`,
    `Follow`, `Space`, `GridGrab`, `GridKind`, and the geometry helpers `grab_at`,
    `dragged`, `carry`, `place_extra`, `wall_height_at`, `Plane`.
  - [tools/axis_lock.rs](../tools/axis_lock.md) — `lock_hv` for Shift axis locking.
  - [doc/transform.rs](../doc/transform.md) — `Transform`; [doc/camera.rs](../doc/camera.md) — `Ease`.
  - [undo.rs](../undo.md) — `undo::Command::Grid` undo steps.
  - [src/app.rs](../app.md) — `GridDrag`, `HORIZON_ATTACH_PX`, `HORIZON_DETACH_PX`,
    `display_camera`, `display_transform`, `frame_size`, `axis_lock_on`, `lock_basis_doc`.

```text
 open file ──► load_project ──► stash_grids (old path) ──► unstash_grids (new path)
 save as   ──► poll_save_job ──► grids_saved_as(new path)
 quit/save prefs ──► grid_files_for_prefs ──► Vec<GridFile> in ui prefs

 every frame ──► sync_grid_buffers: re-find followed layers, tween keyed grids, rebuild walls
 pen down/move/up (Perspective tool) ──► perspective_down / perspective_move / finish_grid_drag
```

## Rust concepts in this file

**Compile-time platform checks with `cfg!`.** `cfg!(windows)` is `true` when the program is
built for Windows. Windows file names are case-insensitive, so `same_path` compares
lower-cased paths there.
```rust
if cfg!(windows) {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
} else { a == b }
```
Like `process.platform === "win32"` in Node, but decided when compiling.
More: [attributes and cfg](../../rust-primer.md#attributes-and-cfg)

**Matching on a tuple.** `unstash_grids` decides between three cases by matching two
`Option`s at once. `_` means "don't care".
```rust
let (grids, active) = match (kept, path) {
    (Some(f), _)    => (f.grids.clone(), f.active),
    (None, Some(_)) => (self.grid_seed.clone(), 0),
    (None, None)    => (PerspectiveConfig::default().grids, 0),
};
```
More: [enums and match](../../rust-primer.md#enums-and-match)

**Taking a closure as a parameter: `impl FnOnce(...)`.** `grid_edit` takes any function that
edits a grid, runs it once, and records an undo step if anything changed. `FnOnce` means
"may be called once". Callers pass a closure like `|g, f| g.set_key_ease(f, ease)`.
```rust
pub fn grid_edit(&mut self, i: usize, edit: impl FnOnce(&mut PerspectiveGrid, usize)) { … }
self.grid_edit(i, |g, f| g.set_key_ease(f, ease));
```
```js
gridEdit(i, (g, f) => g.setKeyEase(f, ease));
```
More: [closures](../../rust-primer.md#closures), [generics](../../rust-primer.md#generics)

**Enum variants with named fields, and `let … else`.** `Follow::Layer { name, index, uid }`
is an enum variant carrying data. `let Follow::Layer { uid, .. } = f else { return None; }`
means "if `f` is a `Layer`, pull out its `uid` (ignore the rest with `..`); otherwise
return". `&mut g.follow` in the pattern gives *mutable* access to the fields so they can be
updated in place.
More: [enums and match](../../rust-primer.md#enums-and-match)

**Patterns with `@` and `|`.** `GridGrab::Corner(c @ (0 | 1))` matches a corner grab whose
number is 0 or 1 *and* binds that number to `c`.
More: [enums and match](../../rust-primer.md#enums-and-match)

**Sorting newest first with `Reverse`.** `sort_by_key(|f| std::cmp::Reverse(f.used))` sorts
by `used` in *descending* order — like `arr.sort((a, b) => b.used - a.used)`.
More: [collections](../../rust-primer.md#collections)

## impl AppState

### Which file's grids

## Types

### `struct GridFile`
[src/app/grids.rs:32](../../../src/app/grids.rs#L32)

One project file's grids, as the app keeps them between sessions. A list of these
(`AppState::grid_files`) is saved in the workspace preferences. Derives
`Serialize`/`Deserialize` with `#[serde(default)]`, so missing fields load as defaults.

| Field | Type | Meaning |
|---|---|---|
| `path` | `PathBuf` | The project file these grids belong to. |
| `grids` | `Vec<PerspectiveGrid>` | That file's grids. |
| `active` | `usize` | Which of them was the active grid. |
| `used` | `u64` | When the file was last open, in seconds since 1 Jan 1970 (the Unix epoch). When more than `MAX_GRID_FILES` are kept, the least recently used are forgotten first. |

### `struct GridFileRepr`
[src/app/grids.rs:48](../../../src/app/grids.rs#L48)

`GridFile` as written. `grids` and `active` keep their old shape and hold only planes, so an older build reads them as it always did; the other guides go in `guides`, each with its place in the whole list, where an older build doesn't look.

| Field | Type | Meaning |
|---|---|---|
| `path` | `PathBuf` | As in `GridFile`. |
| `grids` | `Vec<PerspectiveGrid>` | The planes only (perspective, flat, isometric), in list order — all an older build looks at. |
| `active` | `usize` | Into `grids` — what an older build makes active. |
| `used` | `u64` | As in `GridFile`. |
| `guides` | `Vec<(usize, PerspectiveGrid)>` | Every other guide, each with its place in the whole list. An older build skips this field. |
| `active_all` | `usize` | The active guide's place in the whole list. Only meant when `split` is set: a file from before has none, and `active` is it. |
| `split` | `u8` | 1 once written this way; 0 (missing) in an older file. |

### `const MAX_GRID_FILES`
[src/app/grids.rs:118](../../../src/app/grids.rs#L118)

```rust
const MAX_GRID_FILES: usize = 256;
```

`usize = 256`. The most project files whose grids the app remembers.

### `struct GuidesFile`
[src/app/grids.rs:124](../../../src/app/grids.rs#L124)

A `.guides` file: guides exported from one project, to bring into another. Text, so it can be read and mailed about.

| Field | Type | Meaning |
|---|---|---|
| `version` | `u32` | 1 for now; a later layout bumps it. |
| `guides` | `Vec<PerspectiveGrid>` | The exported guides, as the list had them. |

## `impl From<GridFile> for GridFileRepr`

### `GridFileRepr::from`
[src/app/grids.rs:63](../../../src/app/grids.rs#L63)

```rust
fn from(f: GridFile) -> Self
```

Splits a `GridFile` for writing: planes into `grids`, every other guide into `guides` with its index.
The active guide's index goes in `active_all`; `active` points at the last plane at or before it, so
an older build opens on something sensible. `split` is set to 1. serde calls this through the
`#[serde(into = "GridFileRepr")]` attribute on `GridFile`.

## `impl From<GridFileRepr> for GridFile`

### `GridFile::from`
[src/app/grids.rs:90](../../../src/app/grids.rs#L90)

```rust
fn from(r: GridFileRepr) -> Self
```

Puts a written `GridFileRepr` back together: each guide goes back to its index, and the planes fill
the gaps in order (anything out of range goes on the end). With `split` 0 — a file an older build
wrote — `active` is the active index as it always was; otherwise `active_all` is. Called by serde
through `#[serde(from = "GridFileRepr")]`.

## Free functions

### `write_guides`
[src/app/grids.rs:131](../../../src/app/grids.rs#L131)

```rust
pub fn write_guides(path: &Path, guides: &[PerspectiveGrid]) -> anyhow::Result<()>
```

Write `guides` to a `.guides` file at `path`.

### `read_guides`
[src/app/grids.rs:142](../../../src/app/grids.rs#L142)

```rust
pub fn read_guides(path: &Path) -> anyhow::Result<Vec<PerspectiveGrid>>
```

The guides in the `.guides` file at `path`.

### `now_secs`
[src/app/grids.rs:148](../../../src/app/grids.rs#L148)

```rust
fn now_secs() -> u64
```

**What it does.** The current time in whole seconds since the Unix epoch (like
`Math.floor(Date.now() / 1000)`), or `0` if the system clock is before 1970. Used to stamp
`GridFile::used`.

**Used by:** `AppState::store_grids_as`.

### `same_path`
[src/app/grids.rs:157](../../../src/app/grids.rs#L157)

```rust
fn same_path(a: &Path, b: &Path) -> bool
```

**What it does.** Says whether two paths name the same file for the grid store. On Windows
it ignores letter case (because the Windows file system does); elsewhere it compares exactly.

**Used by:** `store_grids_as`, `unstash_grids`.

## `impl AppState`

### `AppState::stash_grids`
[src/app/grids.rs:169](../../../src/app/grids.rs#L169)

```rust
pub(super) fn stash_grids(&mut self)
```

**What it does.** Files the live grids under the open project's path in the store, if the
project has a path (an unsaved project has none, and nothing happens).

**Used by:** `grids_saved_as`, `grid_files_for_prefs` (this file); `reset_with` and
`load_project` in [src/app.rs](../app.md), before the project changes.

### `AppState::store_grids_as`
[src/app/grids.rs:175](../../../src/app/grids.rs#L175)

```rust
fn store_grids_as(&mut self, path: &Path)
```

**What it does.** Saves the live grids (and which is active) into the store under `path`,
stamped with the current time. If an entry for that path already exists it is replaced;
otherwise a new one is added. If the store then holds more than `MAX_GRID_FILES` entries,
it is sorted newest first and the oldest are dropped.

**Used by:** `stash_grids`, `grids_saved_as`.

### `AppState::unstash_grids`
[src/app/grids.rs:200](../../../src/app/grids.rs#L200)

```rust
pub(super) fn unstash_grids(&mut self, path: Option<&Path>)
```

**What it does.** Makes `path`'s grids the live ones.

**Parameters.**
- `path` — the project file now open, or `None` for a new, unsaved project.

**How it works.**
1. Look `path` up in the store.
2. Found: use its grids and active index.
3. Not found but there is a path: use `grid_seed` — the grids from before the app kept
   grids per file (an older preferences format), so nothing set up back then goes missing.
4. No path at all: start with the default grid set.
5. Clamp the active index to the list, make sure every grid has a unique id
   (`ensure_ids`), and clear any drag or frame-sync state left over from the previous file.

**Used by:** `reset_with` (with `None`) and `load_project` in [src/app.rs](../app.md).

### `AppState::grids_saved_as`
[src/app/grids.rs:217](../../../src/app/grids.rs#L217)

```rust
pub(super) fn grids_saved_as(&mut self, path: &Path)
```

**What it does.** A save just finished at `path`. First files the live grids under the
*old* path (so the old file keeps its grids), then under the new one — so Save As takes the
grids along to the new file.

**Used by:** `poll_save_job` in [src/app.rs](../app.md); tests in `app.rs`.

### `AppState::grid_files_for_prefs`
[src/app/grids.rs:224](../../../src/app/grids.rs#L224)

```rust
pub(super) fn grid_files_for_prefs(&mut self) -> Vec<GridFile>
```

**What it does.** Returns a copy of the whole grid store for the preferences to save, after
first filing the live grids so the latest changes are included.

**Used by:** `save` (the preferences save) in [src/app.rs](../app.md); tests.

### `AppState::export_guides`
[src/app/grids.rs:233](../../../src/app/grids.rs#L233)

```rust
pub fn export_guides(&mut self)
```

Export every guide of the open file to a `.guides` file the artist picks.

### `AppState::import_guides`
[src/app/grids.rs:251](../../../src/app/grids.rs#L251)

```rust
pub fn import_guides(&mut self)
```

Add the guides in a `.guides` file the artist picks to the open file's own.

### `AppState::merge_guides`
[src/app/grids.rs:271](../../../src/app/grids.rs#L271)

```rust
pub(super) fn merge_guides(&mut self, incoming: Vec<PerspectiveGrid>) -> usize
```

Append `incoming` to the live guides under fresh ids, keeping each wall on its own floor — or freeing it, if its floor didn't come too. The last one added becomes active. How many came in.

### `AppState::followed_layer`
[src/app/grids.rs:297](../../../src/app/grids.rs#L297)

```rust
fn followed_layer(&self, f: &Follow) -> Option<usize>
```

**What it does.** If `f` is a `Follow::Layer`, returns the index of the layer with that
session id (`uid`) if it still exists. For `Document` or `Camera`, or a deleted layer,
returns `None`.

**Used by:** `grid_space`.

### `AppState::refresh_follow_links`
[src/app/grids.rs:307](../../../src/app/grids.rs#L307)

```rust
fn refresh_follow_links(&mut self)
```

**What it does.** Re-finds the layer each layer-following grid follows. Layer `uid`s only
last one session (they are not saved), so a grid fresh from the preferences store has
`uid == 0` and must find its layer another way.

**How it works.** For each grid that follows a layer, try in order:
1. the layer whose `uid` matches (if the uid is not 0);
2. the layer at the saved `index`, if its name still matches;
3. the first layer with the saved `name`.

If one is found, the grid's `uid`, `index` and `name` are updated to that layer's current
values, so a rename or reorder keeps the link and the next session finds it too.

**Used by:** `sync_grid_buffers`.

### `AppState::grid_space`
[src/app/grids.rs:334](../../../src/app/grids.rs#L334)

```rust
pub fn grid_space(&self, g: &PerspectiveGrid, frame: usize) -> Space
```

**What it does.** Works out where grid `g`'s coordinates land in the document on `frame`.
Grid corners are stored *relative* to whatever the grid follows; the returned `Space` holds
the frame size and the transform to apply:
- `Follow::Document` — no transform (identity).
- `Follow::Camera` — the camera's frame on that frame (`display_camera(frame)`), so the
  grid stays put in the shot as the camera pans, zooms and rolls.
- `Follow::Layer` — that layer's transform on that frame (`display_transform`). If the
  layer is gone, the grid stays on the document.

**Returns.** A `Space`; `space.to_doc(p)` turns a grid point into document coordinates and
`space.to_rel(p)` goes back.

**Used by:** `set_grid_follow`, `perspective_down` (this file); `shape_plane`, `slide_shape`,
`snap_doc` in [src/app.rs](../app.md); `grid_settings`, `draw_perspective_grids`,
`draw_cursor_guides` in [ui/shell.rs](../ui/shell.md).

### `AppState::follow_layer`
[src/app/grids.rs:348](../../../src/app/grids.rs#L348)

```rust
pub fn follow_layer(&self, i: usize) -> Option<Follow>
```

**What it does.** Builds the `Follow::Layer` link (name, index, uid) for layer `i`, ready to
pass to `set_grid_follow`. `None` if there is no layer `i` (the `?` returns early).

**Used by:** `grid_settings` in [ui/shell.rs](../ui/shell.md); tests in `app.rs`.

### `AppState::set_grid_follow`
[src/app/grids.rs:360](../../../src/app/grids.rs#L360)

```rust
pub fn set_grid_follow(&mut self, i: usize, follow: Follow)
```

**What it does.** Points grid `i` at a new parent (document, camera or a layer) *without
moving it on screen* on the current frame. Its live corners, extra vanishing points, and
every key's pose are converted from the old space into the new one. Does nothing if the
follow is unchanged, or if the grid is a wall (a wall follows whatever its floor follows).

**How it works.**
1. Compute the old `Space` and the new `Space` (a clone with the new `follow`), both on the
   current frame.
2. `carry` is a closure: old-relative point → document point (`old.to_doc`) → new-relative
   point (`new.to_rel`).
3. Apply `carry` to the four corners (`corners.map(carry)`), every extra vanishing point,
   and every key's corners and extras.
4. Store the moved grid and call `relink()` so walls standing on it are rebuilt.

**Used by:** `grid_settings` in [ui/shell.rs](../ui/shell.md); tests.

### `AppState::sync_grid_buffers`
[src/app/grids.rs:390](../../../src/app/grids.rs#L390)

```rust
pub(super) fn sync_grid_buffers(&mut self)
```

**What it does.** Runs once per UI frame. Re-finds followed layers; then, if the current
frame changed since the last sync *and no grid drag is under way*, sets every keyed grid's
live pose to the tweened pose for this frame (`resolve` then `set_pose`); finally rebuilds
walls from their floors (`relink`). Skipping the update mid-drag is the same rule the camera
follows, so a drag is never overwritten under the pen.

**Used by:** `update` in [src/app.rs](../app.md) (every frame); tests.

### `AppState::grid_edit`
[src/app/grids.rs:409](../../../src/app/grids.rs#L409)

```rust
pub fn grid_edit(&mut self, i: usize, edit: impl FnOnce(&mut PerspectiveGrid, usize))
```

**What it does.** Runs `edit` on grid `i` as a single undo step. `edit` receives the grid
and the current frame. A copy of the grid is taken before; if the grid is different
afterwards, an `undo::Command::Grid { before, after }` is pushed onto the history (both
copies boxed — put on the heap — to keep the command small). If nothing changed, no undo
step is added. Walls are rebuilt at the end (`relink`).

**Parameters.**
- `i` — the grid index (out of range: nothing happens).
- `edit` — the change to make, as a closure.

**Used by:** `add_grid_key`, `delete_grid_key`, `set_grid_key_ease`.

### `AppState::add_grid_key`
[src/app/grids.rs:428](../../../src/app/grids.rs#L428)

```rust
pub fn add_grid_key(&mut self)
```

**What it does.** Keys the active grid's current (live) pose on the current frame. Walls are
skipped — they take their motion from their floor and have no keys. Undoable.

**Used by:** `grid_settings` in [ui/shell.rs](../ui/shell.md); tests.

### `AppState::delete_grid_key`
[src/app/grids.rs:439](../../../src/app/grids.rs#L439)

```rust
pub fn delete_grid_key(&mut self)
```

**What it does.** Deletes the active grid's key on the current frame (if it has one), then
shows the pose the remaining keys give this frame. Undoable.

**Used by:** `grid_settings` in [ui/shell.rs](../ui/shell.md).

### `AppState::set_grid_key_ease`
[src/app/grids.rs:452](../../../src/app/grids.rs#L452)

```rust
pub fn set_grid_key_ease(&mut self, ease: Ease)
```

**What it does.** Sets the *ease* out of the active grid's key on the current frame — the
speed curve of the tween towards the next key (e.g. linear, or slow-in/slow-out). Undoable.

**Used by:** `grid_settings` in [ui/shell.rs](../ui/shell.md).

### `AppState::grid_pose_edited`
[src/app/grids.rs:460](../../../src/app/grids.rs#L460)

```rust
pub fn grid_pose_edited(&mut self, i: usize)
```

**What it does.** Call this after grid `i`'s pose was changed by hand. With *auto-key* on, a
grid that already has keys gets a key on the current frame, so the change survives moving
to another frame. A grid with no keys (or a wall) just stays moved. With auto-key off,
nothing happens.

**Used by:** `finish_grid_drag` (this file); `grid_settings` in [ui/shell.rs](../ui/shell.md).

### `AppState::restore_grid`
[src/app/grids.rs:474](../../../src/app/grids.rs#L474)

```rust
pub(super) fn restore_grid(&mut self, snap: &PerspectiveGrid)
```

**What it does.** Puts back what an undo or redo of a grid step recorded. Finds the grid by
its id, restores its motion (pose, keys, wall height — but not its look or switches) from
`snap` via `restore_motion`, and if it has keys shows their pose for the current frame.
Clears any grid drag and rebuilds walls.

**Used by:** `apply_touched` in [src/app.rs](../app.md) (undo/redo).

### `AppState::ensure_perspective_grid`
[src/app/grids.rs:494](../../../src/app/grids.rs#L494)

```rust
pub fn ensure_perspective_grid(&mut self)
```

**What it does.** Makes sure the Perspective tool has something to edit: if every grid was
deleted, adds a default one. Also clamps the active index into range.

**Used by:** `AppState::set_tool` in [app/select.rs](select.md).

### `AppState::cycle_perspective_grid`
[src/app/grids.rs:507](../../../src/app/grids.rs#L507)

```rust
pub fn cycle_perspective_grid(&mut self)
```

**What it does.** Makes the next *visible* grid after the active one active, wrapping
around to the start. Hidden grids are skipped (nothing can snap to them). This is how to
change which grid strokes snap to without opening the grid list. With no other visible
grid, nothing changes.

**How it works.** Tries indices `active+1, active+2, …` modulo the grid count
(`(cfg.active + k) % n` for `k` in `1..n`) and picks the first visible one.

**Used by:** `dispatch` in [src/app.rs](../app.md) (`Action::PerspectiveNextGrid`).

### `AppState::perspective_down`
[src/app/grids.rs:522](../../../src/app/grids.rs#L522)

```rust
pub fn perspective_down(&mut self, p: P)
```

**What it does.** The Perspective tool was pressed at document point `p`. Decides which grid
and which part of it (a corner, a vanishing point, the horizon, the rotate ring, the inside
to move…) was hit, makes that grid active, and starts a drag on it.

**Parameters.**
- `p` — the press point in document coordinates (`P` is `[f32; 2]`, an `[x, y]` pair).

**How it works.**
1. Clear any old drag. The hit tolerance is the handle size in screen points
   (`HANDLE_PX`) divided by the view zoom, so handles are the same size on screen at any
   zoom.
2. Try the active grid first, then every other grid
   (`std::iter::once(active).chain(…)`).
3. Skip hidden grids. For each, get its corners and extra vanishing points in document
   space (via `grid_space`) and ask what the press hits: `perspective::grab_at` for a plane,
   `guides::grab_at` for a ruler, vanishing point or ellipse. The horizon handles count only
   for a perspective grid (not a wall) that shows its horizon.
4. On the first hit, make that grid active. If it can be grabbed — not *locked*, and if
   it is a wall only by its top corners (0 or 1; the rest of a wall belongs to its floor) —
   save a copy for undo (`grid_drag_before`) and store a `GridDrag` with the grab, the
   press point, the corners, the extra points and the `Space` *as they are at the press*.
5. Return after the first hit (a locked grid is selected but not dragged).

**Used by:** the canvas input in [ui/shell.rs](../ui/shell.md) (`draw`); many tests in
`app.rs`.

### `AppState::perspective_move`
[src/app/grids.rs:579](../../../src/app/grids.rs#L579)

```rust
pub fn perspective_move(&mut self, p: P)
```

**What it does.** The Perspective tool was dragged to document point `p`. Updates the
grabbed grid. The new shape is always solved from the shape *at the press* (stored in the
`GridDrag`), not accumulated frame to frame.

**How it works.**
1. No drag: return. Work in the `Space` saved at the press (the camera or layer the grid
   follows does not move mid-drag).
2. Decide whether this grab *slides* something (moving the grid, a vanishing point, an
   extra point, or a corner of a non-rigid grid). If Shift axis lock is on and it slides,
   hold `p` to the horizontal or vertical line through the press (`axis_lock::lock_hv`).
   A ruler's end and a helper line's end don't slide: Shift turns them in 15° steps
   instead (inside `guides::dragged`).
   `moved(from)` is a closure that shifts a point by the drag offset.
3. **Wall:** only its top corners do anything — they set how tall the wall stands
   (`wall_height_at` on its floor). Then rebuild walls and return.
4. **Extra vanishing point:** place it with `place_extra`: one sitting on the grid's horizon
   slides along it until pulled more than `HORIZON_DETACH_PX` screen points off; a free one
   snaps back on within `HORIZON_ATTACH_PX`. Store its new position and whether it is on the
   horizon, then return.
5. **Anything else:** with the centre key (Alt) held on a perspective grid's vanishing
   point, move both points evenly (`mirror_vps`). Otherwise ask `perspective::dragged` (a
   plane: corners, vanishing points, horizon, tilt, rotate, move, the 15° snapping, rigid
   grids) or `guides::dragged` (a ruler, vanishing point or ellipse) for the new corners. If
   it returns `None` — the drag would fold the quad, or leave a guide meaningless — leave
   the grid as it was.
6. If the whole grid moved or turned (or a rigid grid's corner was dragged, which turns
   and scales it), carry the extra vanishing points along with `perspective::carry`.
   Reshaping leaves them where they are.
7. Store the new corners (converted back to frame-relative) and rebuild walls.

**Used by:** the canvas input in [ui/shell.rs](../ui/shell.md) (`draw`); tests in `app.rs`.

### `AppState::grid_drag_readout`
[src/app/grids.rs:680](../../../src/app/grids.rs#L680)

```rust
pub fn grid_drag_readout(&self) -> Option<String>
```

What the grid drag in hand is turning, in degrees, for beside the pen: a ruler's or a helper line's heading, a grid turned whole, a horizon tilted. Up the screen counts positive, as on a protractor.

### `AppState::finish_grid_drag`
[src/app/grids.rs:712](../../../src/app/grids.rs#L712)

```rust
pub(super) fn finish_grid_drag(&mut self)
```

**What it does.** The pen lifted off a grid drag. Takes the drag and the saved "before"
copy (doing nothing if either is missing), applies auto-key (`grid_pose_edited`), and if
the grid changed pushes one `undo::Command::Grid` for the whole drag.

**Used by:** `pointer_up` in [src/app.rs](../app.md).

## Free functions

### `mirror_vps`
[src/app/grids.rs:734](../../../src/app/grids.rs#L734)

```rust
fn mirror_vps(c: &[P; 4], which: u8, delta: P) -> Option<[P; 4]>
```

A grid's corners re-aimed with its vanishing point `which` moved by `delta` and the other moved the opposite way, so both stay as far from the middle between them. `None` unless both are finite, or if the quad would fold.

## Tests

### `mod tests`

Run with `cargo test app::grids::tests` · 8 tests

| Test | What it checks |
|---|---|
| [`exported_guides_come_back_in_another_file_with_their_walls`](../../../src/app/grids.rs#L801) | Exported guides come back in another file with their walls. |
| [`a_wall_without_its_floor_comes_in_free`](../../../src/app/grids.rs#L828) | A wall without its floor comes in free. |
| [`the_centre_key_moves_both_vanishing_points_evenly`](../../../src/app/grids.rs#L840) | The centre key moves both vanishing points evenly. |
| [`the_store_keeps_every_guide_kind_in_order`](../../../src/app/grids.rs#L853) | The store keeps every guide kind in order. |
| [`an_older_build_reads_only_the_planes`](../../../src/app/grids.rs#L861) | An older build reads only the planes. |
| [`curved_guides_go_where_an_older_build_does_not_look`](../../../src/app/grids.rs#L873) | Curved guides go where an older build does not look. |
| [`curved_guides_export_and_come_back_with_their_shape`](../../../src/app/grids.rs#L884) | Curved guides export and come back with their shape. |
| [`a_store_an_older_build_wrote_still_loads`](../../../src/app/grids.rs#L894) | A store an older build wrote still loads. |

**Test helpers** (used only by the tests above):

- [`file`](../../../src/app/grids.rs#L752) — File.

**Test-only types and constants:** [`KEY`](../../../src/app/grids.rs#L750), [`OldGridFile`](../../../src/app/grids.rs#L778), [`OldGrid`](../../../src/app/grids.rs#L787), [`OldKind`](../../../src/app/grids.rs#L793)
