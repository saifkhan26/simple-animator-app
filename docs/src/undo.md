# `src/undo.rs` — undo and redo history

> **Source:** [src/undo.rs](../../src/undo.rs) · **Module:** `crate::undo` · **Size:** 462 lines · Line links as of commit `67a7cb9`

## What this file is for

Every change you can undo is recorded here as a **`Command`**: a small description of what changed,
with enough "before" and "after" data to go either way. `History` keeps two stacks — undo and redo —
like a browser's back/forward history:

```text
 do A, do B, do C          undo            undo            redo
 undo: [A, B, C]      undo: [A, B]     undo: [A]       undo: [A, B]
 redo: []             redo: [C]        redo: [C, B]    redo: [C]
```

Doing something new after an undo clears the redo stack.

The trick that keeps memory small is that a paint stroke doesn't store the whole drawing: a
`PixelPatch` stores only the **dirty rectangle** the stroke touched, before and after. Structural
edits (adding frames, moving keys, adding layers) store a `TimelineState` — the layer list and
cursors, which are cheap to clone because the heavy pixel buffers are shared `Arc`s and are not part
of it. The history is capped at 80 steps and 2 GB of pixel patches, dropping the oldest first.

## How it connects

- **Used by:** [app.rs](app.md) — `AppState` owns a `History` (`history` field); every edit pushes a
  command (`self.history.push(...)`), and `AppState::undo` / `redo` call `History::undo` / `redo`,
  then refresh textures from the returned `Touched`. Also [app/select.rs](app/select.md),
  [app/liquify.rs](app/liquify.md), [app/grids.rs](app/grids.md) and others push commands.
- **Uses:** [doc/project.rs](doc/project.md) (`Project`, which commands restore),
  [doc/canvas.rs](doc/canvas.md) (`Canvas` pixels), [doc/layer.rs](doc/layer.md) (`Layer`, `CellId`),
  [doc/camera.rs](doc/camera.md), [tools/selection.rs](tools/selection.md) (`SelectionMask`,
  `PackedMask`), [tools/perspective.rs](tools/perspective.md) (`PerspectiveGrid`).
- The cell garbage collector in `AppState::collect_cells` asks `History::for_each_cell` which drawings
  the history still refers to, so it never frees one an undo could bring back.

## Rust concepts in this file

**Enums carrying data = a typed command list.** Each kind of undo step is a variant with its own
fields, and one `match` in `apply` handles all of them:
```rust
match cmd {
    Command::PixelPatch { cell, x, y, w, h, before, after } => { /* copy bytes back */ }
    Command::Structural { before, after, .. } => { /* restore layers */ }
    Command::Compound(cmds) => { /* apply each, in order or reversed */ }
    // ...
}
```
In JS you'd write objects with a `type` field and a `switch (cmd.type)`; Rust's compiler checks every
variant is handled. More: [Enums and match](../rust-primer.md#enums-and-match)

**`?` on `Option` inside a method.** `let cmd = self.undo_stack.pop()?;` — `pop` returns `None` on an
empty stack, and `?` makes `undo` return `None` right there. More:
[Option and Result](../rust-primer.md#option-and-result)

**`Arc` sharing instead of copying.** `CellPixelDelta` holds `Arc<Canvas>` before/after: "pointers"
to the very canvases the project used, so keeping them costs no extra pixel memory.
More: [Smart pointers](../rust-primer.md#smart-pointers)

**`Box<T>` to keep enum variants small.** `Grid { before: Box<PerspectiveGrid>, … }` stores the big
grid on the heap so every `Command` value stays small. More: [Smart pointers](../rust-primer.md#smart-pointers)

## Types

### `const MAX_HISTORY`
[src/undo.rs:16](../../src/undo.rs#L16)

```rust
const MAX_HISTORY: usize = 80;
```

Bounded undo capacity — prevents memory blow-up on long sessions.

### `const MAX_PATCH_BYTES`
[src/undo.rs:21](../../src/undo.rs#L21)

```rust
const MAX_PATCH_BYTES: usize = 2 << 30;
```

Bytes of paint patches the undo history may hold before its oldest steps go, however few steps that leaves. A stroke across an 8K cell keeps ~70 MB of before and after, so 80 of them would be several GB.

### `enum Touched`
[src/undo.rs:24](../../src/undo.rs#L24)

What an undo/redo touched, so the caller knows what to re-upload.

| Variant | Meaning |
|---|---|
| `Cell(CellId)` | One cell's pixels changed; its `Canvas::dirty` says where. |
| `Cells(Vec<CellId>)` | These cells' pixels changed wholesale. |
| `Structure` | Only the timeline structure changed. Every pixel is where it was, so every texture is still good — on a big layer, re-uploading them all is what made undo stall. |
| `Selection(Option<SelectionMask>)` | The selection became this. |
| `Grid(Box<PerspectiveGrid>)` | A perspective grid's pose, keys or wall height went back to this. |
| `Many(Vec<Touched>)` | Several of the above, from a `Command::Compound`. |

### `struct TimelineState`
[src/undo.rs:45](../../src/undo.rs#L45)

Snapshot of the timeline structure (exposures + cursors), excluding the heavy pixel buffers. Cheap to clone, so structural edits stay undoable without blowing up history memory.

| Field | Type | Meaning |
|---|---|---|
| `layers` | `Vec<Layer>` | Every layer, including its exposures (which drawing shows on which frame), transform keys and flags — but not pixels. |
| `frame_count` | `usize` | Number of frames in the timeline. |
| `current_frame` | `usize` | The playhead. |
| `current_layer` | `usize` | The active layer. |
| `loop_start` | `usize` | First frame of the playback loop. |
| `loop_end` | `usize` | Last frame of the playback loop. |
| `camera` | `Camera` | The live camera. |
| `camera_keys` | `Vec<CameraKey>` | The camera's keyframes. |

### `struct CellPixelDelta`
[src/undo.rs:85](../../src/undo.rs#L85)

Before/after for a cell whose pixels a structural edit wiped (only the "delete the last remaining frame" path clears pixels). The buffers themselves, shared: holding them costs nothing the pool didn't hold.

| Field | Type | Meaning |
|---|---|---|
| `cell` | `CellId` | Which drawing. |
| `before` | `Arc<Canvas>` | Its canvas before the edit (shared, not copied). |
| `after` | `Arc<Canvas>` | Its canvas after the edit. |

### `enum Command`
[src/undo.rs:91](../../src/undo.rs#L91)

One undoable step. See the variants below; `Compound` groups several into one step.

| Variant | Meaning |
|---|---|
| `PixelPatch { cell, x, y, w, h, before, after }` | A paint operation on a single cell within a sub-rect. |
| `Structural { before, after, cell_pixels }` | A timeline/layer edit (add/duplicate/delete frame, insert/hold key, add/delete layer). Restores the structure wholesale; `cell_pixels` carries pixel buffers only when the edit also wiped cell contents. |
| `LayerCanvasResize { layer, before_size, after_size, before }` | A layer's drawable buffer was resized, re-padding every cell it owns. No existing command covers this — `Structural` restores layers but not `project.cells`.  Only the originals are stored — the very buffers the resize replaced, shared rather than copied — and redo re-runs the deterministic re-pad rather than keeping a second full copy of every cell. |
| `Selection { before: Option<PackedMask>, after: Option<PackedMask>, }` | The selection changed. Only the selection: the pixels it covers are untouched, so this restores no cell. |
| `Grid { before: Box<PerspectiveGrid>, after: Box<PerspectiveGrid>, }` | A perspective grid moved, or its keys changed. Grids live on the app, not in the project, so this restores nothing here: the caller puts the snapshot back by the grid's id. |
| `Compound(Vec<Command>)` | Several commands that undo and redo as one step — a float landing is its pixels *and* the selection that moved with them. Applied in order on redo, in reverse on undo. |

### `struct History`
[src/undo.rs:155](../../src/undo.rs#L155)

The undo/redo history: two stacks of `Command`s plus a revision counter. One per `AppState`.

| Field | Type | Meaning |
|---|---|---|
| `undo_stack` | `Vec<Command>` | Commands that can be undone, oldest first; the last is undone next. |
| `redo_stack` | `Vec<Command>` | Commands that were undone and can be redone; the last is redone next. |
| `revision` | `u64` | Bumped by every push, undo and redo — whatever can change which cells the history still reaches. See `History::for_each_cell`. |

## `impl TimelineState`

### `TimelineState::capture`
[src/undo.rs:57](../../src/undo.rs#L57)

```rust
pub fn capture(project: &Project) -> Self
```

Takes a snapshot of the project's timeline structure (layers, frame count, cursors, loop, camera and
camera keys) — everything except pixels. Called before and after structural edits.

### `TimelineState::restore`
[src/undo.rs:70](../../src/undo.rs#L70)

```rust
fn restore(&self, project: &mut Project)
```

Writes a snapshot back into the project, replacing the layers and cursors.

## `impl Command`

### `Command::patch_bytes`
[src/undo.rs:145](../../src/undo.rs#L145)

```rust
fn patch_bytes(&self) -> usize
```

How many bytes of pixel data this step holds of its own (for the 2 GB cap). Only `PixelPatch` and
`Compound` hold any; shared canvases don't count.

## `impl History`

### `History::push`
[src/undo.rs:164](../../src/undo.rs#L164)

```rust
pub fn push(&mut self, cmd: Command)
```

Records a new step (with the default 2 GB patch cap). Clears the redo stack.

### `History::push_within`
[src/undo.rs:170](../../src/undo.rs#L170)

```rust
fn push_within(&mut self, cmd: Command, max_bytes: usize)
```

`push` with an explicit byte cap: pushes `cmd`, clears the redo stack, bumps the revision, then drops
the oldest steps until at most `MAX_HISTORY` (80) remain and their patches total at most `max_bytes`.
The newest step always stays, even if it alone is over the cap.

### `History::revision`
[src/undo.rs:185](../../src/undo.rs#L185)

```rust
pub fn revision(&self) -> u64
```

A counter bumped by every push/undo/redo. Other code compares it with a stored value to know whether
the history changed (e.g. before re-running the drawing garbage collector).

### `History::patch_bytes`
[src/undo.rs:191](../../src/undo.rs#L191)

```rust
pub fn patch_bytes(&self) -> usize
```

Total bytes of pixel patches held by the undo stack.

### `History::for_each_cell`
[src/undo.rs:197](../../src/undo.rs#L197)

```rust
pub fn for_each_cell(&self, f: &mut impl FnMut(CellId))
```

Calls `f` for every drawing (cell id) any undo or redo step refers to, so those drawings are kept
alive. `f: &mut impl FnMut(CellId)` is "a callback that may modify its own captured state", e.g.
inserting into a set.

#### `History::for_each_cell::walk`
[src/undo.rs:198](../../src/undo.rs#L198)

```rust
fn walk(cmd: &Command, f: &mut impl FnMut(CellId))
```

The recursive helper: matches on a command's kind and reports its cell ids — a patch's cell, every
exposure in a structural snapshot (`iter().flatten()` skips the empty frames, which are `None`), the
resized layer's cells, or each part of a compound.

### `History::can_undo`
[src/undo.rs:221](../../src/undo.rs#L221)

```rust
pub fn can_undo(&self) -> bool
```

Whether there is anything to undo (drives the Undo button's enabled state).

### `History::can_redo`
[src/undo.rs:224](../../src/undo.rs#L224)

```rust
pub fn can_redo(&self) -> bool
```

Whether there is anything to redo.

### `History::undo_len`
[src/undo.rs:229](../../src/undo.rs#L229)

```rust
pub fn undo_len(&self) -> usize
```

Number of steps on the undo stack.

### `History::undo`
[src/undo.rs:235](../../src/undo.rs#L235)

```rust
pub fn undo(&mut self, project: &mut Project) -> Option<Touched>
```

Pops the newest command, applies it *backwards* to the project (`apply(project, &cmd, false)`), moves
it to the redo stack, bumps the revision and returns what was touched — or `None` if there was
nothing to undo.

### `History::redo`
[src/undo.rs:243](../../src/undo.rs#L243)

```rust
pub fn redo(&mut self, project: &mut Project) -> Option<Touched>
```

The mirror of `undo`: pops from the redo stack, applies forwards, pushes back onto the undo stack.

## Free functions

### `apply`
[src/undo.rs:253](../../src/undo.rs#L253)

```rust
fn apply(project: &mut Project, cmd: &Command, forward: bool) -> Touched
```

Applies one command forwards (`forward = true`, redo) or backwards (undo) and returns what it touched:
- `PixelPatch` — copies the `after` (or `before`) bytes into the cell's rectangle with `blit_subrect`
  and sets the cell's dirty rect to exactly that patch → `Touched::Cell`.
- `Structural` — restores the timeline snapshot and puts back any wiped cell canvases →
  `Touched::Structure` (no textures need re-uploading) or `Touched::Cells`.
- `LayerCanvasResize` — redo re-runs the resize; undo puts the original canvases and size back.
- `Selection` / `Grid` — returns the snapshot for the caller to install (they live on `AppState`, not
  the project).
- `Compound` — applies each part, in order for redo and in reverse for undo → `Touched::Many`.

### `blit_subrect`
[src/undo.rs:344](../../src/undo.rs#L344)

```rust
fn blit_subrect(canvas: &mut Canvas, x: u32, y: u32, w: u32, h: u32, bytes: &[u8])
```

Copies a `w`×`h` block of RGBA bytes into the canvas at `(x, y)`, one row at a time. Row `r` of the
patch goes to byte offset `((y + r) × width + x) × 4` in the canvas — the same indexing as a browser
`ImageData` buffer. `copy_from_slice` is a fast memory copy (like `TypedArray.set`).

## Tests

### `mod tests`

Run with `cargo test undo::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`the_history_keeps_its_patches_within_a_byte_cap`](../../src/undo.rs#L394) | Big patches push the oldest out before the step count would; the newest stays even when it alone is over. |
| [`a_selection_step_undoes_and_redoes`](../../src/undo.rs#L411) | A selection step undoes and redoes. |
| [`a_compound_undoes_in_reverse`](../../src/undo.rs#L425) | A compound undoes its parts last-first, so a later part that depends on an earlier one is always taken back before it. |

**Test helpers** (used only by the tests above):

- [`rect`](../../src/undo.rs#L360) — Rect.
- [`selection_of`](../../src/undo.rs#L371) — Selection of.
- [`patch`](../../src/undo.rs#L379) — A paint step holding `n` bytes of before and `n` of after.
