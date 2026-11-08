# `src/doc/project.rs` — the animation document (Project)

> **Source:** [src/doc/project.rs](../../../src/doc/project.rs) · **Module:** `crate::doc::project` · **Size:** 2387 lines · Line links as of commit `1fa5845`

## What this file is for

`Project` is **the animation document** — everything that gets saved in a `.anim` file: canvas size,
frame rate, every drawing's pixels, the layers and which drawing each shows on each frame, the playhead,
the loop range and the camera. Almost every edit you make in the app is a method call on `Project`.

### The data model in one picture

```text
Project
├── width × height, fps
├── cells: Vec<Arc<Canvas>>        ← the "pool" of drawings; a CellId is an index into this
│      [0] Canvas (pixels)  [1] Canvas  [2] Canvas  …
├── layers: Vec<Layer>             ← bottom to top
│      Layer "Ink":   exposures [Some(0), None, Some(2), None]   → frames show 0,0,2,2
│      Layer "Color": exposures [Some(1), None, None,    None]   → frames show 1,1,1,1
├── frame_count = 4, current_frame, current_layer
├── loop_start .. loop_end         ← playback range
└── camera (+ camera_keys)          ← what the export frame sees
```

- **Cells** are the drawings (pixel buffers). They live in one flat pool so layers can refer to them by
  number. A cell is never shared between two layers or two separate keys; holds are what make a drawing
  show on several frames.
- **Exposures** say which cell each layer shows on each frame (`Some` = key, `None` = hold) — see
  [doc/layer.rs](layer.md).
- Cells are wrapped in `Arc` (a shared pointer) so **cloning the whole project is cheap**: a save or
  export copies pointers, not pixels. The first write to a cell that is still shared makes a private copy
  (`Arc::make_mut`, called by `cell_mut`) — "copy-on-write".

The file also contains the **timeline editing** used by the timeline tracks: "blocks" (a drawing plus
the frames it holds for), retiming, clearing, closing gaps, and dragging drawings between layers
(`ripple_move`, which never overwrites anything — later drawings slide out of the way).

## How it connects

- **Used by:** almost everything — [app.rs](../app.md) (`AppState::project`), [ui/shell.rs](../ui/shell.md),
  [ui/tracks.rs](../ui/tracks.md) (blocks, retime, move), [undo.rs](../undo.md) (snapshots),
  [io/project_file.rs](../io/project_file.md) (save/load), [io/composite.rs](../io/composite.md) and every
  exporter, [io/kra/mod.rs](../io/kra/mod.md), [krita_link.rs](../krita_link.md),
  [timeline/playback.rs](../timeline/playback.md).
- **Uses:** [doc/layer.rs](layer.md), [doc/canvas.rs](canvas.md), [doc/camera.rs](camera.md),
  [doc/transform.rs](transform.md), [doc/groups.rs](groups.md) and [doc/clip.rs](clip.md) (which add more
  `impl Project` methods in their own files).

## Rust concepts in this file

**`Arc<Canvas>` and copy-on-write.**
```rust
pub fn cell_mut(&mut self, id: CellId) -> Option<&mut Canvas> {
    self.cells.get_mut(id).map(Arc::make_mut)   // copy the canvas first if a clone still shares it
}
```
`Arc::make_mut` gives a writable reference, cloning the canvas only if another `Arc` points at it. There
is no direct JS equivalent; think of an immutable-data library (Immer) doing structural sharing for you.
More: [Smart pointers](../../rust-primer.md#smart-pointers)

**`&mut self` methods mutate in place.** `project.add_frame()` changes `project` itself; there is no
return value to reassign. More: [Impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

**Closures passed in to supply things.** `ripple_move(…, mut cell: impl FnMut(CellReq) -> CellId)` asks
its caller for cells through a callback instead of touching `self.cells` directly — so it can work on a
plain slice of layers and be previewed on a copy. More: [Closures](../../rust-primer.md#closures)

**`Range<usize>`.** `Block::span` returns `start..end`, a value you can loop over or test with
`.contains(&f)`. More: [Strings and slices](../../rust-primer.md#strings-and-slices)

**`saturating_sub`.** `frame_count.saturating_sub(1)` is `frame_count − 1` but never below 0 (an
unsigned number can't be negative). More: [Numbers and casts](../../rust-primer.md#numbers-and-casts)

## Types

### `struct Project`
[src/doc/project.rs:17](../../../src/doc/project.rs#L17)

The whole animation document. See the picture above.

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | Canvas size: the document rect, and the size of a layer's cells unless it has its own. The camera's frame is this size too unless it is given one — see `Project::output_size`. Layers may sit outside it — see `crate::doc::camera`. |
| `height` | `u32` | Canvas height in pixels (see `width`). |
| `fps` | `f32` | Frames per second for playback and export. |
| `cells` | `Vec<Arc<Canvas>>` | Shared, copy-on-write: cloning the project (a save, an export, a Krita send) costs a reference per cell, and only a cell edited while such a copy is alive gets duplicated — on its first write. |
| `layers` | `Vec<Layer>` | The layer stack, bottom (index 0) to top. Includes group header rows. |
| `frame_count` | `usize` | Timeline length; every layer's `exposures` has this many slots. |
| `current_frame` | `usize` | The playhead (0-based). |
| `current_layer` | `usize` | The active layer's index. |
| `loop_start` | `usize` | First frame of the playback loop. |
| `loop_end` | `usize` | One past the last frame of the playback loop. |
| `camera` | `Camera` | Live / static camera. Used directly when `camera_keys` is empty; otherwise it is the working buffer for the current frame, mirroring how `Layer::transform` relates to `Layer::transform_keys`. |
| `camera_keys` | `Vec<CameraKey>` | Sorted camera keyframes. Empty = static `camera`. |
| `camera_opts` | `CameraOpts` | Session-only, like `Layer::lines_from`: skipped, so the `.anim` layout is what it was and a load brings back the defaults. |

### `enum CellReq`
[src/doc/project.rs:1101](../../../src/doc/project.rs#L1101)

What `ripple_move` needs a cell for.

| Variant | Meaning |
|---|---|
| `Blank { layer: usize }` | A fresh blank for `layer`, to stop a drawing that landed where nothing was showing — otherwise it would hold on to the end. |
| `Moved { id: CellId, src: usize, dst: usize, must_clone: bool, }` | Drawing `id`, from layer `src`, landing on layer `dst`. `must_clone` when it has to be a buffer of its own: a copy, or a move to another layer while the source still shows it somewhere else. |

### `struct Pick`
[src/doc/project.rs:1156](../../../src/doc/project.rs#L1156)

A helper struct inside `ripple_move`: the drawings picked up from one layer.

| Field | Type | Meaning |
|---|---|---|
| `src` | `usize` | The layer the picked drawings come from. |
| `first` | `usize` | The first picked drawing's start frame. |
| `spans` | `Vec<Range<usize>>` | Frame ranges of each picked drawing. |
| `ids` | `Vec<CellId>` | The drawings' cell ids. |

### `struct Block`
[src/doc/project.rs:1254](../../../src/doc/project.rs#L1254)

A drawing and the frames it holds for, on one layer: the key at `start` through to the next key.

| Field | Type | Meaning |
|---|---|---|
| `layer` | `usize` | The layer. |
| `start` | `usize` | The frame the drawing's key is on. |
| `len` | `Option<usize>` | Frames held. `None` for the layer's last drawing, which holds to the end of the timeline however long that is. |

## `impl Project`

### `Project::new`
[src/doc/project.rs:54](../../../src/doc/project.rs#L54)

```rust
pub fn new(width: u32, height: u32, fps: f32) -> Self
```

A new project of the given size and frame rate, seeded with one blank cell, one layer ("Layer 1") keyed
to it on frame 0, a one-frame timeline, a default camera.

### `Project::output_size`
[src/doc/project.rs:78](../../../src/doc/project.rs#L78)

```rust
pub fn output_size(&self) -> (u32, u32)
```

The camera frame's size — the size every exported frame has: the camera's own size if set, else the
canvas size.

### `Project::resolve_camera`
[src/doc/project.rs:83](../../../src/doc/project.rs#L83)

```rust
pub fn resolve_camera(&self, frame: usize) -> Camera
```

The camera on `frame`: the static camera when there are no keys, else interpolated between camera keys.

### `Project::resolve_camera_at`
[src/doc/project.rs:95](../../../src/doc/project.rs#L95)

```rust
pub fn resolve_camera_at(&self, t: f32) -> Camera
```

The camera at a *fractional* time `t` in frames (needed when the camera's move is stretched over several
loops and falls between frames).

### `Project::camera_loops`
[src/doc/project.rs:106](../../../src/doc/project.rs#L106)

```rust
pub fn camera_loops(&self) -> u32
```

How many passes of the animation the camera's move lasts — more than 1 only when that option is set and
there is a move (two keys) to stretch.

### `Project::held_inside`
[src/doc/project.rs:116](../../../src/doc/project.rs#L116)

```rust
pub fn held_inside(&self, cam: Camera) -> Camera
```

`cam`, pushed back inside the canvas when "keep camera inside canvas" is on.

### `Project::camera_leaves_canvas`
[src/doc/project.rs:126](../../../src/doc/project.rs#L126)

```rust
pub fn camera_leaves_canvas(&self) -> bool
```

Whether the keyed camera shows past the canvas edge on any frame (or at rest).

### `Project::keep_inside_by_default`
[src/doc/project.rs:143](../../../src/doc/project.rs#L143)

```rust
pub fn keep_inside_by_default(&mut self) -> bool
```

Turns "keep inside canvas" on for a new/opened project — unless its camera already goes past the edge on
purpose, in which case it stays off and this returns `false` (the app then shows a toast).

### `Project::has_camera_key`
[src/doc/project.rs:149](../../../src/doc/project.rs#L149)

```rust
pub fn has_camera_key(&self, frame: usize) -> bool
```

Whether there is a camera key exactly on `frame`.

### `Project::set_camera_key`
[src/doc/project.rs:155](../../../src/doc/project.rs#L155)

```rust
pub fn set_camera_key(&mut self, frame: usize, camera: Camera)
```

Adds or replaces the camera key on `frame` (keeping its easing), keeping keys sorted.

### `Project::delete_camera_key`
[src/doc/project.rs:169](../../../src/doc/project.rs#L169)

```rust
pub fn delete_camera_key(&mut self, frame: usize)
```

Removes the camera key on `frame`.

### `Project::prev_camera_key`
[src/doc/project.rs:176](../../../src/doc/project.rs#L176)

```rust
pub fn prev_camera_key(&self) -> Option<usize>
```

The frame of the nearest camera key before the current frame (keys past the end of the timeline ignored).

### `Project::next_camera_key`
[src/doc/project.rs:183](../../../src/doc/project.rs#L183)

```rust
pub fn next_camera_key(&self) -> Option<usize>
```

The frame of the nearest camera key after the current frame.

### `Project::move_camera_key`
[src/doc/project.rs:195](../../../src/doc/project.rs#L195)

```rust
pub fn move_camera_key(&mut self, from: usize, to: usize, copy: bool) -> bool
```

Moves (or with `copy`, copies) the camera key on `from` to `to`, replacing any key there. Returns
`false` and changes nothing if there's no key on `from` or `from == to`.

### `Project::cell`
[src/doc/project.rs:208](../../../src/doc/project.rs#L208)

```rust
pub fn cell(&self, id: CellId) -> Option<&Canvas>
```

Read access to drawing `id`, or `None` if the id is out of range. `.map(|c| &**c)`-style code turns the
`&Arc<Canvas>` into a plain `&Canvas`.

### `Project::cell_mut`
[src/doc/project.rs:213](../../../src/doc/project.rs#L213)

```rust
pub fn cell_mut(&mut self, id: CellId) -> Option<&mut Canvas>
```

Write access to drawing `id` — copying it first if a clone of the project (a background save, say) still
shares it.

### `Project::compact_cells`
[src/doc/project.rs:222](../../../src/doc/project.rs#L222)

```rust
pub fn compact_cells(&mut self)
```

Drops every cell no layer shows (deleted drawings, leftovers in older files) and renumbers the rest.
Only safe right after opening a file, when nothing else (no undo history, no texture cache) holds cell ids.

### `Project::alloc_cell`
[src/doc/project.rs:249](../../../src/doc/project.rs#L249)

```rust
pub fn alloc_cell(&mut self) -> CellId
```

Adds a new blank cell sized for the active layer and returns its id.

### `Project::alloc_cell_for`
[src/doc/project.rs:255](../../../src/doc/project.rs#L255)

```rust
pub fn alloc_cell_for(&mut self, layer: usize) -> CellId
```

Adds a new blank cell sized for `layer` (layers with an enlarged canvas get bigger cells).

### `Project::draw_cell_size`
[src/doc/project.rs:271](../../../src/doc/project.rs#L271)

```rust
pub fn draw_cell_size(&self, layer: usize, frame: usize) -> (u32, u32)
```

The pixel size of the buffer a stroke on `(layer, frame)` will paint into — the existing cell's, or the
size `alloc_cell_for` would create. Pen input mapping uses this, so it must agree with allocation.

### `Project::resolved_current`
[src/doc/project.rs:282](../../../src/doc/project.rs#L282)

```rust
pub fn resolved_current(&self) -> Option<CellId>
```

The drawing showing at the active layer and frame, if any.

### `Project::ensure_active_cell`
[src/doc/project.rs:298](../../../src/doc/project.rs#L298)

```rust
pub fn ensure_active_cell(&mut self) -> CellId
```

Makes sure the active slot has a drawing to paint into and returns its id: an existing key's cell, a
held cell (painting into the hold edits the shared drawing), or a newly allocated blank key if the layer
shows nothing yet.

### `Project::ensure_cell`
[src/doc/project.rs:303](../../../src/doc/project.rs#L303)

```rust
pub fn ensure_cell(&mut self, layer: usize, frame: usize) -> CellId
```

`ensure_active_cell` for any `(layer, frame)`.

### `Project::copy_active_cell`
[src/doc/project.rs:319](../../../src/doc/project.rs#L319)

```rust
pub fn copy_active_cell(&self) -> Option<Canvas>
```

A deep copy of the drawing at the active slot (for Copy drawing). A copy rather than an id, because two
slots must never share a buffer.

### `Project::cut_active_cell`
[src/doc/project.rs:330](../../../src/doc/project.rs#L330)

```rust
pub fn cut_active_cell(&mut self) -> Option<Canvas>
```

Takes the drawing at the active slot and leaves the frame **blank**: the slot is keyed to a new empty cell
rather than turned into a hold (which would make the previous drawing appear there, as if the cut hit
the wrong frame).

### `Project::paste_cell_here`
[src/doc/project.rs:341](../../../src/doc/project.rs#L341)

```rust
pub fn paste_cell_here(&mut self, src: &Canvas) -> CellId
```

Keys a copy of `src` at the active slot, replacing what was there. If the layer's cells are a different
size, the drawing is re-centred (cropped or padded) rather than stretched.

### `Project::insert_blank_key_here`
[src/doc/project.rs:357](../../../src/doc/project.rs#L357)

```rust
pub fn insert_blank_key_here(&mut self) -> CellId
```

Puts a new empty key on the active slot, breaking any hold.

### `Project::insert_duplicate_key_here`
[src/doc/project.rs:368](../../../src/doc/project.rs#L368)

```rust
pub fn insert_duplicate_key_here(&mut self) -> CellId
```

Puts a new key on the active slot that is a copy of the drawing that was showing — to break a hold but
keep the drawing as a starting point.

### `Project::layer_cell_ids`
[src/doc/project.rs:389](../../../src/doc/project.rs#L389)

```rust
pub fn layer_cell_ids(&self, layer: usize) -> Vec<CellId>
```

Every distinct cell id a layer shows.

### `Project::expand_layer_canvas`
[src/doc/project.rs:405](../../../src/doc/project.rs#L405)

```rust
pub fn expand_layer_canvas(&mut self, layer: usize, new_w: u32, new_h: u32)
```

Resizes a layer's drawing buffers to `new_w`×`new_h`, re-padding every one of its cells with the old
pixels centred. Centring keeps the artwork exactly where it was, since layers are positioned by their
centre.

### `Project::hold_here`
[src/doc/project.rs:421](../../../src/doc/project.rs#L421)

```rust
pub fn hold_here(&mut self)
```

Deletes the key on the active slot so the previous drawing holds through it.

### `Project::add_frame`
[src/doc/project.rs:429](../../../src/doc/project.rs#L429)

```rust
pub fn add_frame(&mut self)
```

Inserts one empty frame (a hold) after the current frame on every layer, shifting keys, pins and tracker
points after it, and moves the playhead onto the new frame. The loop end follows the timeline end.

### `Project::duplicate_frame`
[src/doc/project.rs:446](../../../src/doc/project.rs#L446)

```rust
pub fn duplicate_frame(&mut self)
```

Inserts a frame after the current one on every layer that shows the same drawing (each layer's resolved
cell is keyed again), and moves the playhead onto it.

### `Project::delete_frame`
[src/doc/project.rs:461](../../../src/doc/project.rs#L461)

```rust
pub fn delete_frame(&mut self)
```

Deletes the current frame on every layer (shifting later keys back). If it is the only frame, clears it
instead: every slot becomes empty and every cell is replaced by a fresh blank canvas (new `Arc`s rather
than clearing in place, so an undo snapshot sharing the old ones stays valid).

### `Project::insert_frames_before`
[src/doc/project.rs:496](../../../src/doc/project.rs#L496)

```rust
pub fn insert_frames_before(&mut self, n: usize)
```

Inserts `n` frames before the playhead on every layer (holds of what shows before, or blank at the start).
Everything from the playhead on — drawings, camera and transform keys, pins, tracker points, the loop —
moves `n` later, and the playhead moves with its drawing.

### `Project::keys_insert_frames`
[src/doc/project.rs:530](../../../src/doc/project.rs#L530)

```rust
fn keys_insert_frames(&mut self, at: usize, n: usize)
```

Shifts camera keys and every layer's transform keys after an insert of `n` frames at `at`.

### `Project::keys_remove_frames`
[src/doc/project.rs:543](../../../src/doc/project.rs#L543)

```rust
fn keys_remove_frames(&mut self, at: usize, n: usize)
```

Same after removing frames `at..at+n` (keys on removed frames are dropped).

### `Project::goto`
[src/doc/project.rs:555](../../../src/doc/project.rs#L555)

```rust
pub fn goto(&mut self, frame: usize)
```

Moves the playhead to `frame`, clamped to the timeline.

### `Project::step`
[src/doc/project.rs:565](../../../src/doc/project.rs#L565)

```rust
pub fn step(&mut self, delta: isize, wrap: bool)
```

Moves the playhead by `delta` frames — wrapping around with `wrap`, otherwise stopping at the ends.

### `Project::clip_base`
[src/doc/project.rs:587](../../../src/doc/project.rs#L587)

```rust
pub fn clip_base(&self, li: usize) -> Option<usize>
```

The base layer that layer `li` is clipped to: the nearest layer below that isn't clipped itself (staying
within its group). `None` if `li` isn't clipped or has no possible base.

### `Project::key_link`
[src/doc/project.rs:605](../../../src/doc/project.rs#L605)

```rust
pub fn key_link(&self, li: usize) -> Option<usize>
```

The layer `li`'s key jumps also stop at (its `keys_from` link), if it still resolves to a valid drawing
layer.

### `Project::jumps_at`
[src/doc/project.rs:615](../../../src/doc/project.rs#L615)

```rust
pub fn jumps_at(&self, li: usize, f: usize) -> bool
```

Whether a "next/previous drawing" jump on layer `li` stops at frame `f`: one of its own drawings starts
there, or one of its linked layer's does (so a cleanup layer can follow the rough's timing).

### `Project::toggle_key_link_below`
[src/doc/project.rs:623](../../../src/doc/project.rs#L623)

```rust
pub fn toggle_key_link_below(&mut self, li: usize) -> bool
```

Links layer `li`'s key jumps to the nearest drawing layer below it, or unlinks if already linked there.
`false` if there's nothing to link.

### `Project::add_cleanup_layer`
[src/doc/project.rs:638](../../../src/doc/project.rs#L638)

```rust
pub fn add_cleanup_layer(&mut self, of: usize) -> Option<usize>
```

Adds a layer directly above `of` for redrawing it cleanly: named after it, key jumps linked to it, and
made active. `None` for a group header.

### `Project::prev_key_frame`
[src/doc/project.rs:656](../../../src/doc/project.rs#L656)

```rust
pub fn prev_key_frame(&self) -> Option<usize>
```

The nearest drawing key on the active layer before the current frame (holds skipped; linked layer's keys
count). Used by the "previous drawing" shortcut.

### `Project::next_key_frame`
[src/doc/project.rs:667](../../../src/doc/project.rs#L667)

```rust
pub fn next_key_frame(&self) -> Option<usize>
```

The nearest drawing key on the active layer after the current frame.

### `Project::relink_after_insert`
[src/doc/project.rs:677](../../../src/doc/project.rs#L677)

```rust
pub(crate) fn relink_after_insert(&mut self, at: usize)
```

After inserting a layer at `at`, shifts every `lines_from` link pointing at `at` or above up by one, so
links keep pointing at the same layers.

### `Project::relink_after_remove`
[src/doc/project.rs:689](../../../src/doc/project.rs#L689)

```rust
pub fn relink_after_remove(&mut self, at: usize)
```

After removing the layer at `at`, drops links to it and shifts links above it down by one.

### `Project::relink`
[src/doc/project.rs:701](../../../src/doc/project.rs#L701)

```rust
pub(crate) fn relink(&mut self, to_new: impl Fn(usize) -> usize)
```

After any reorder, remaps every `lines_from` link through `to_new(old_index)`.

### `Project::add_layer`
[src/doc/project.rs:709](../../../src/doc/project.rs#L709)

```rust
pub fn add_layer(&mut self)
```

Adds a new full-length layer named "Layer N" at the top of the stack and selects it.

### `Project::add_layer_above_active`
[src/doc/project.rs:719](../../../src/doc/project.rs#L719)

```rust
pub fn add_layer_above_active(&mut self) -> usize
```

Adds a new full-length layer directly above the active one, selects it, and returns its index.

### `Project::move_layer`
[src/doc/project.rs:735](../../../src/doc/project.rs#L735)

```rust
pub fn move_layer(&mut self, from: usize, to: usize)
```

Moves layer `from` to index `to` (the others close up), keeping links valid, and selects it.

### `Project::ensure_frame_count`
[src/doc/project.rs:760](../../../src/doc/project.rs#L760)

```rust
pub fn ensure_frame_count(&mut self, n: usize)
```

Grows the timeline to at least `n` frames, padding every layer with holds.

### `Project::add_layer_below_active`
[src/doc/project.rs:777](../../../src/doc/project.rs#L777)

```rust
pub fn add_layer_below_active(&mut self, name: impl Into<String>) -> usize
```

Adds a new layer directly below the active one; the active layer stays selected (its index shifts up).

### `Project::add_background_layer`
[src/doc/project.rs:795](../../../src/doc/project.rs#L795)

```rust
pub fn add_background_layer(&mut self, name: impl Into<String>) -> usize
```

Adds a new layer at the very bottom (index 0) — a background; the active layer stays selected.

### `Project::delete_layer`
[src/doc/project.rs:804](../../../src/doc/project.rs#L804)

```rust
pub fn delete_layer(&mut self)
```

Deletes the active layer (never the last one), fixes links, and keeps the selection in range.

### `Project::block_at`
[src/doc/project.rs:830](../../../src/doc/project.rs#L830)

```rust
pub fn block_at(&self, layer: usize, frame: usize) -> Option<Block>
```

The drawing showing on `frame` on `layer`, with the frames it holds for, as a `Block`; `None` before the
layer's first key.

### `Project::blocks`
[src/doc/project.rs:843](../../../src/doc/project.rs#L843)

```rust
pub fn blocks(&self, layer: usize) -> Vec<Block>
```

Every drawing (block) on `layer`, in timeline order.

### `Project::settle_length`
[src/doc/project.rs:861](../../../src/doc/project.rs#L861)

```rust
fn settle_length(&mut self, need: usize)
```

Grows the timeline to `need` frames, padding only layers shorter than that.

### `Project::insert_layer_frames`
[src/doc/project.rs:877](../../../src/doc/project.rs#L877)

```rust
pub fn insert_layer_frames(&mut self, layer: usize, after: usize, n: usize)
```

Inserts `n` held frames on one layer after `after`: that drawing holds longer and later ones slide right.

### `Project::remove_layer_frames`
[src/doc/project.rs:890](../../../src/doc/project.rs#L890)

```rust
pub fn remove_layer_frames(&mut self, layer: usize, at: usize, n: usize)
```

Removes `n` frames from one layer at `at`: later drawings slide left and the last drawing holds longer to
fill the end.

### `Project::retime_block`
[src/doc/project.rs:897](../../../src/doc/project.rs#L897)

```rust
pub fn retime_block(&mut self, b: Block, new_len: usize)
```

Changes how long drawing `b` holds to `new_len` frames (at least 1), sliding later drawings.

### `Project::set_timing`
[src/doc/project.rs:918](../../../src/doc/project.rs#L918)

```rust
pub fn set_timing(&mut self, starts: &[(usize, usize)], n: usize) -> Vec<(usize, usize)>
```

Gives every drawing keyed at `starts` a hold of `n` frames ("on twos", "on threes"…). Works left to right
with a running shift, because retiming one drawing moves every later one. Returns the new starts.

### `Project::clear_blocks`
[src/doc/project.rs:949](../../../src/doc/project.rs#L949)

```rust
pub fn clear_blocks(&mut self, starts: &[(usize, usize)]) -> Vec<(usize, usize)>
```

Deletes the drawings keyed at `starts`, leaving blank frames (each run of adjacent ones becomes one new
blank key); nothing else moves. Returns the blank runs' starts.

### `Project::close_blocks`
[src/doc/project.rs:984](../../../src/doc/project.rs#L984)

```rust
pub fn close_blocks(&mut self, starts: &[(usize, usize)])
```

Deletes the drawings keyed at `starts` and closes the gap: later drawings slide left.

### `Project::drop_allowed`
[src/doc/project.rs:1010](../../../src/doc/project.rs#L1010)

```rust
pub fn drop_allowed(&self, src: usize, dst: usize, copy: bool) -> bool
```

Whether a timeline drag from layer `src` to `dst` may land: never onto (or out of, for a move) a locked or
reference layer.

### `Project::blocks_keyed_at`
[src/doc/project.rs:1016](../../../src/doc/project.rs#L1016)

```rust
pub fn blocks_keyed_at(&self, starts: &[(usize, usize)]) -> Vec<Block>
```

The blocks whose keys are exactly at `starts`.

### `Project::can_move_blocks`
[src/doc/project.rs:1027](../../../src/doc/project.rs#L1027)

```rust
pub fn can_move_blocks(&self, starts: &[(usize, usize)], dl: isize, df: isize, copy: bool) -> bool
```

Whether `move_blocks` would do anything: the drag goes somewhere, the layers involved exist and may be
edited, and everything lands within the timeline (or just past its end, which extends it).

### `Project::move_blocks`
[src/doc/project.rs:1052](../../../src/doc/project.rs#L1052)

```rust
pub fn move_blocks(
    &mut self,
    starts: &[(usize, usize)],
    dl: isize,
    df: isize,
    copy: bool,
) -> Option<Vec<(usize, usize)>>
```

Moves (or copies) every drawing keyed at `starts` by `dl` layers and `df` frames, using `ripple_move`'s
"nothing overwritten" rules. Returns where they landed, or `None` (nothing changed) if not allowed.

## Free functions

### `block_keyed_on`
[src/doc/project.rs:1117](../../../src/doc/project.rs#L1117)

```rust
fn block_keyed_on(l: &Layer, layer: usize, fc: usize, start: usize) -> Option<Block>
```

The block keyed exactly on frame `start` of a layer, if there is one.

### `ripple_move`
[src/doc/project.rs:1147](../../../src/doc/project.rs#L1147)

```rust
pub fn ripple_move(
    layers: &mut [Layer],
    fc: usize,
    starts: &[(usize, usize)],
    dl: isize,
    df: isize,
    copy: bool,
    mut cell: impl FnMut(CellReq) -> CellId,
) -> (usize, Vec<(usize, usize)>)
```

The magnetic timeline drag. For each layer: the selected drawings are lifted out and (unless copying)
the gaps close up; they then land on layer `+dl`, back to back in their order, the first at its old start
`+df`, and the drawings already there slide right to make room. Nothing is overwritten or lost. It asks
its caller for cells through `cell` (a blank to stop a drawing that landed on empty frames, or a moved
drawing that must be cloned). Returns the timeline length needed and where the drawings landed.

**Example.**
```text
before:  A A B B C C        drag B one frame right
after:   A A A B B C C      B lands at frame 3; A holds one more frame; C slides right
```

## `impl Block`

### `Block::span`
[src/doc/project.rs:1264](../../../src/doc/project.rs#L1264)

```rust
pub fn span(&self, frame_count: usize) -> Range<usize>
```

The frame range a block covers (`start..start+len`, or to the end of the timeline for the last drawing).

## Free functions

### `distinct_layers`
[src/doc/project.rs:1270](../../../src/doc/project.rs#L1270)

```rust
fn distinct_layers(starts: &[(usize, usize)]) -> Vec<usize>
```

Each layer index mentioned in `starts`, once, in order.

### `layer_insert`
[src/doc/project.rs:1280](../../../src/doc/project.rs#L1280)

```rust
pub fn layer_insert(l: &mut Layer, after: usize, n: usize)
```

Inserts `n` holds after frame `after` on one layer (tracker points kept in step).

### `layer_insert_at`
[src/doc/project.rs:1286](../../../src/doc/project.rs#L1286)

```rust
pub fn layer_insert_at(l: &mut Layer, at: usize, n: usize)
```

Inserts `n` holds at frame `at` (before whatever is there) — the version that can reach frame 0.

### `layer_remove`
[src/doc/project.rs:1298](../../../src/doc/project.rs#L1298)

```rust
pub fn layer_remove(l: &mut Layer, at: usize, n: usize)
```

Removes frames `at..at+n` from one layer and pads the end with holds. If the cut removed the key of a
drawing that continues past it, that drawing is re-keyed at `at` so the next frame shows what it showed
before.

### `layer_fit`
[src/doc/project.rs:1323](../../../src/doc/project.rs#L1323)

```rust
pub fn layer_fit(l: &mut Layer, min_len: usize) -> usize
```

Trims or pads a layer to the timeline length (or longer, if a key was pushed past the end). Returns the
timeline length required.

### `layer_retime`
[src/doc/project.rs:1341](../../../src/doc/project.rs#L1341)

```rust
pub fn layer_retime(
    l: &mut Layer,
    frame_count: usize,
    b: Block,
    new_len: usize,
    blank: impl FnOnce() -> CellId,
) -> usize
```

`retime_block` for one layer, with `blank` supplying a fresh blank cell when a last drawing is cut short.
Public so the timeline can preview a retime on a copy of the layer.

### `recenter`
[src/doc/project.rs:1369](../../../src/doc/project.rs#L1369)

```rust
pub fn recenter(src: &Canvas, new_w: u32, new_h: u32) -> Canvas
```

Copies `src` into a new `new_w`×`new_h` canvas with its content centred; content that doesn't fit is
cropped.

## Tests

### `mod tests`

Run with `cargo test doc::project::tests` · 56 tests

| Test | What it checks |
|---|---|
| [`a_clone_shares_cells_until_one_is_written`](../../../src/doc/project.rs#L1378) | A clone shares cells until one is written. |
| [`a_duplicated_key_shares_its_drawing_until_drawn_on`](../../../src/doc/project.rs#L1388) | A duplicated key shares its drawing until drawn on. |
| [`compacting_drops_unshown_cells_and_renumbers_the_rest`](../../../src/doc/project.rs#L1399) | Compacting drops unshown cells and renumbers the rest. |
| [`step_wraps_at_both_ends_when_looping`](../../../src/doc/project.rs#L1422) | Step wraps at both ends when looping. |
| [`step_clamps_at_both_ends_when_not_looping`](../../../src/doc/project.rs#L1435) | Step clamps at both ends when not looping. |
| [`a_camera_key_moves_with_its_ease_and_takes_the_place_of_one_it_lands_on`](../../../src/doc/project.rs#L1449) | A camera key moves with its ease and takes the place of one it lands on. |
| [`the_smooth_path_is_only_taken_when_asked_for`](../../../src/doc/project.rs#L1488) | The smooth path is an option of the session: off, every frame is what the keys have always given; on, the frames between three keys curve. |
| [`a_held_camera_stays_on_the_canvas_on_every_frame`](../../../src/doc/project.rs#L1516) | With the camera held on the canvas, a key that left it — and every frame tweened towards one — comes back inside; with it off, they are exactly what the keys say. |
| [`keep_inside_starts_on_unless_the_camera_already_leaves_the_canvas`](../../../src/doc/project.rs#L1541) | Keep inside canvas starts on for a camera that stays on the canvas, and off for one keyed past its edge — on a key, or only between two. |
| [`recenter_grows_symmetrically`](../../../src/doc/project.rs#L1574) | A symmetric grow leaves the old pixels centered, which is what keeps a layer visually still (its transform is defined about the cell center). |
| [`a_new_layer_goes_just_above_the_selected_one`](../../../src/doc/project.rs#L1607) | A new layer goes just above the selected one. |
| [`moving_a_layer_closes_up_behind_it_and_keeps_links`](../../../src/doc/project.rs#L1624) | Moving a layer closes up behind it and keeps links. |
| [`draw_cell_size_matches_what_alloc_will_create`](../../../src/doc/project.rs#L1647) | Input mapping sizes an unkeyed slot by the layer's own cell size, not the frame size — otherwise the first stroke on a fresh oversized layer is mapped for one buffer and painted into a bigger one. |
| [`recenter_shrinks_by_cropping`](../../../src/doc/project.rs#L1667) | Shrinking crops around the center rather than panicking on the copy. |
| [`expanded_layer_allocates_bigger_cells`](../../../src/doc/project.rs#L1682) | New cells on an expanded layer come out at the layer's size, not the project's — otherwise the next key would silently shrink back. |
| [`cut_blanks_the_slot_without_disturbing_the_grid`](../../../src/doc/project.rs#L1694) | Cut must not shorten this layer or touch any other: every layer's `exposures` stays `frame_count` long, which the whole timeline assumes. |
| [`pasted_drawings_are_independent_copies`](../../../src/doc/project.rs#L1716) | Pasted drawings are independent copies. |
| [`pasting_into_a_differently_sized_layer_recentres`](../../../src/doc/project.rs#L1734) | Pasting into a differently sized layer recentres. |
| [`key_jump_skips_holds`](../../../src/doc/project.rs#L1762) | The point of the feature: from a held frame the jump reaches the bracketing drawings, not the neighbouring repeats. |
| [`key_jump_is_strict_about_the_current_frame`](../../../src/doc/project.rs#L1772) | Strictly before / after: sitting on a key must still move off it, otherwise the button would be a no-op wherever it matters most. |
| [`key_jump_clamps_at_both_ends`](../../../src/doc/project.rs#L1781) | Clamping, not wrapping — `None` is what greys the button out. |
| [`camera_key_jump_is_strict_and_clamps`](../../../src/doc/project.rs#L1804) | Camera key jump is strict and clamps. |
| [`camera_key_jump_ignores_keys_past_the_end`](../../../src/doc/project.rs#L1817) | A key past the last frame isn't somewhere the jump can land, so the button greys out instead of doing nothing. |
| [`key_jump_on_an_untouched_layer_finds_nothing`](../../../src/doc/project.rs#L1828) | The jump reads the *active* layer only. A fresh `Project` keys its first layer on frame 0, so this needs a second, untouched one. |
| [`a_linked_layer_jumps_through_its_own_and_the_linked_drawings`](../../../src/doc/project.rs#L1844) | The cleanup case: a layer linked to the rough under it stops at the rough's drawings and at its own, and nowhere else. |
| [`a_broken_key_link_is_ignored`](../../../src/doc/project.rs#L1870) | A link that can't resolve is no link: its target deleted, itself, or a group header. |
| [`a_block_is_a_key_and_its_hold`](../../../src/doc/project.rs#L1935) | A block is a key and its hold. |
| [`inserting_frames_slides_only_that_layer`](../../../src/doc/project.rs#L1948) | The end of the timeline stays put: the last drawing's hold absorbs it. |
| [`a_key_pushed_off_the_end_grows_every_layer`](../../../src/doc/project.rs#L1960) | A key pushed off the end grows every layer. |
| [`removing_frames_keeps_what_showed_after_the_cut`](../../../src/doc/project.rs#L1970) | Removing frames keeps what showed after the cut. |
| [`retiming_a_drawing_slides_the_ones_after_it`](../../../src/doc/project.rs#L1987) | Retiming a drawing slides the ones after it. |
| [`retiming_the_last_drawing_stops_it_or_grows_the_timeline`](../../../src/doc/project.rs#L1998) | Retiming the last drawing stops it or grows the timeline. |
| [`set_timing_puts_a_run_on_twos`](../../../src/doc/project.rs#L2013) | Set timing puts a run on twos. |
| [`clearing_leaves_blank_frames_and_moves_nothing`](../../../src/doc/project.rs#L2024) | Clearing leaves blank frames and moves nothing. |
| [`closing_the_gap_slides_later_drawings_up`](../../../src/doc/project.rs#L2039) | Closing the gap slides later drawings up. |
| [`moving_within_a_layer_closes_the_gap_and_leaves_no_blank`](../../../src/doc/project.rs#L2064) | Moving within a layer closes the gap and leaves no blank. |
| [`dragging_onto_the_next_drawing_swaps_them`](../../../src/doc/project.rs#L2076) | Dragging onto the next drawing swaps them. |
| [`landing_in_a_hold_splits_it`](../../../src/doc/project.rs#L2087) | Landing in a hold splits it. |
| [`a_drawing_moved_into_an_earlier_hold_splits_it`](../../../src/doc/project.rs#L2102) | A drawing moved into an earlier hold splits it. |
| [`moving_to_an_empty_layer_hands_the_buffer_over_and_stops_it`](../../../src/doc/project.rs#L2112) | Moving to an empty layer hands the buffer over and stops it. |
| [`a_cross_layer_move_copies_when_it_has_to`](../../../src/doc/project.rs#L2125) | A cross layer move copies when it has to. |
| [`a_last_drawing_moved_to_another_layer_keeps_its_length`](../../../src/doc/project.rs#L2146) | A last drawing moved to another layer keeps its length. |
| [`a_last_drawing_moved_left_splits_the_hold_it_lands_in`](../../../src/doc/project.rs#L2157) | A last drawing moved left splits the hold it lands in. |
| [`the_last_drawings_hold_does_not_pick_up_past_the_end`](../../../src/doc/project.rs#L2167) | Dropping into the last drawing's hold near the end: that hold is only filler to the end, so it doesn't pick up again past it. |
| [`moving_a_run_lands_it_back_to_back_in_either_direction`](../../../src/doc/project.rs#L2176) | Moving a run lands it back to back in either direction. |
| [`scattered_drawings_land_together_in_order`](../../../src/doc/project.rs#L2190) | Scattered drawings land together in order. |
| [`a_drawing_on_the_last_step_is_pushed_along_never_erased`](../../../src/doc/project.rs#L2202) | The reported bug: with a drawing on the last step, copying and moving drawings in front of it erased it instead of pushing it along. |
| [`dropping_just_past_the_end_appends`](../../../src/doc/project.rs#L2227) | Dropping just past the end appends. |
| [`moving_a_rectangle_down_a_layer_moves_the_lower_row_first`](../../../src/doc/project.rs#L2236) | Moving a rectangle down a layer moves the lower row first. |
| [`locked_layers_are_never_edited_by_a_drag`](../../../src/doc/project.rs#L2253) | Locked layers are never edited by a drag. |
| [`pins_follow_their_drawings_through_frame_inserts`](../../../src/doc/project.rs#L2283) | Pins follow their drawings through frame inserts. |
| [`a_pin_on_a_removed_frame_goes_with_it`](../../../src/doc/project.rs#L2299) | A pin on a removed frame goes with it. |
| [`inserting_before_the_first_frame_slides_every_layer_and_key`](../../../src/doc/project.rs#L2324) | Frames inserted before the first one go in on every layer, blank: every drawing slides, every key with it, and the playhead stays on the drawing it was on. |
| [`inserting_before_mid_timeline_holds_what_showed_before`](../../../src/doc/project.rs#L2348) | Mid-timeline, what showed before the playhead holds over the new frames, and a shorter loop stays on its drawings. |
| [`frame_ops_move_camera_and_transform_keys_too`](../../../src/doc/project.rs#L2363) | Adding, duplicating and deleting frames on every layer move the camera's and the layers' transform keys with the drawings; a key on a deleted frame goes with it. |
| [`padding_the_timeline_leaves_pins_alone`](../../../src/doc/project.rs#L2381) | Padding the timeline leaves pins alone. |

**Test helpers** (used only by the tests above):

- [`names`](../../../src/doc/project.rs#L1587) — Names.
- [`abc`](../../../src/doc/project.rs#L1592) — Three layers named A (bottom), B, C (top); C's fills read B's lines.
- [`keyed_on_fours`](../../../src/doc/project.rs#L1749) — Keys on 0 / 4 / 8 of an 11-frame layer — the shape the jump buttons navigate.
- [`camera_keyed`](../../../src/doc/project.rs#L1794) — Camera keys on 2 and 7 of a 10-frame timeline, plus one left past the end the way a frame delete leaves it.
- [`key`](../../../src/doc/project.rs#L1888) — Key a fresh drawing on (`layer`, `f`) whose first byte is `mark`, so a test can tell which drawing shows where after it has moved.
- [`sheet`](../../../src/doc/project.rs#L1897) — One layer, `frames` long, with a drawing keyed on each of `keys`, marked with its key frame + 1.
- [`key_frames`](../../../src/doc/project.rs#L1907) — Key frames.
- [`row`](../../../src/doc/project.rs#L1912) — The mark of the drawing showing on every frame; 0 for nothing or blank.
- [`assert_sound`](../../../src/doc/project.rs#L1920) — What the whole timeline assumes: every layer exactly as long as the timeline, and no cell shown on two layers.
- [`mv`](../../../src/doc/project.rs#L2057) — Move (or copy) the drawing keyed on `from` so it starts on (`to_layer`, `to_frame`).
- [`pin_frames`](../../../src/doc/project.rs#L2264) — Pin frames.
- [`pinned`](../../../src/doc/project.rs#L2268) — Pinned.
- [`cam_key`](../../../src/doc/project.rs#L2308) — Cam key.
- [`cam_frames`](../../../src/doc/project.rs#L2316) — Cam frames.
