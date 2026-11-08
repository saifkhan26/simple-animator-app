# `src/doc/layer.rs` — one layer of the animation

> **Source:** [src/doc/layer.rs](../../../src/doc/layer.rs) · **Module:** `crate::doc::layer` · **Size:** 406 lines · Line links as of commit `67a7cb9`

## What this file is for

A **layer** is one row of the animation, like a layer in Photoshop but stretched across time. Its key
field is `exposures`: one slot per frame saying which **drawing** (a *cell*, identified by a
`CellId` — an index into `Project::cells`) shows on that frame.

```text
frame:       0      1      2      3      4      5
exposures: Some(7) None   None   Some(9) None   Some(7)
showing:     7      7      7      9      9      7
```

- `Some(id)` is a **key**: drawing `id` starts showing here.
- `None` is a **hold**: keep showing whatever the last key put up.

So "animating on twos" (each drawing held for 2 frames) is just a key every other frame. The same
drawing can be keyed again later (frame 5 above re-uses drawing 7). Drawing on a held frame edits the
shared drawing — the usual animator convention (unless "auto-key on draw" breaks the hold first).

A layer also has a name, opacity, visibility/lock flags, a **transform** (position/scale/rotation, with
optional keyframes that are interpolated between), flags for clipping and alpha lock, group membership
(`kind`, `in_group`, `collapsed`), and some session-only extras (onion pins, fill-boundary link,
tracker points).

## How it connects

- **Used by:** [doc/project.rs](project.md) (`Project::layers: Vec<Layer>`), and through it nearly
  everything: [app.rs](../app.md), [ui/shell.rs](../ui/shell.md), [ui/tracks.rs](../ui/tracks.md),
  [timeline/onion.rs](../timeline/onion.md), [io/project_file.rs](../io/project_file.md),
  [io/composite.rs](../io/composite.md), [undo.rs](../undo.md) (snapshots clone layers).
- **Uses:** [doc/transform.rs](transform.md) (`Transform`, `TransformKey`, `Ease`),
  [timeline/onion.rs](../timeline/onion.md) (`OnionPin`).

## Rust concepts in this file

**`Vec<Option<CellId>>` — a list of "maybe a drawing".** Rust has no `null`, so an empty slot is
`None`. Walking back through holds:
```rust
pub fn resolve(&self, frame: usize) -> Option<CellId> {
    // from `frame` backwards, return the first slot that is Some(id)
}
```
```js
function resolve(exposures, frame) {
  for (let f = Math.min(frame, exposures.length - 1); f >= 0; f--)
    if (exposures[f] !== null) return exposures[f];
  return null;
}
```
More: [Option and Result](../../rust-primer.md#option-and-result)

**Type alias.** `pub type CellId = usize;` — `CellId` *is* `usize`, just named for meaning.
More: [Structs](../../rust-primer.md#structs)

**`impl Into<String>` parameters.** `Layer::new(name: impl Into<String>, …)` accepts `"Ink"` (a `&str`)
or a `String`; `.into()` converts. More: [Generics](../../rust-primer.md#generics)

**A global counter with an atomic.** `next_uid` keeps a `static NEXT: AtomicU64` and does
`fetch_add(1)` — a thread-safe `counter++` that returns the old value.
More: [Threads and channels](../../rust-primer.md#threads-and-channels)

**Field order matters for the file format.** The `.anim` format uses *postcard*, which writes fields by
position, not by name, so new fields are only ever appended at the end (`kind`, `in_group`,
`collapsed` "must stay the LAST fields"). More: [Serde and file formats](../../rust-primer.md#serde-and-file-formats)

## Types

### `type CellId`
[src/doc/layer.rs:11](../../../src/doc/layer.rs#L11)

```rust
pub type CellId = usize;
```

The id of a drawing: an index into `Project::cells`.

### `struct TrackSample`
[src/doc/layer.rs:28](../../../src/doc/layer.rs#L28)

One frame's stabilization tracking points, in document space. `a` is the primary point (translation); `b` is optional and enables rotation/scale correction when present on both the reference frame and the tracked frame.

| Field | Type | Meaning |
|---|---|---|
| `a` | `Option<[f32; 2]>` | The primary tracking point on this frame (document pixels), used to correct position shake. |
| `b` | `Option<[f32; 2]>` | An optional second point; with both, rotation and zoom shake can be corrected too. |

### `struct Layer`
[src/doc/layer.rs:34](../../../src/doc/layer.rs#L34)

One row of the layer stack: a paint layer with drawings exposed across the timeline, or a group header.

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | The name shown in the Layers panel. |
| `opacity` | `f32` | 0–1; how strongly the layer shows. |
| `visible` | `bool` | Whether the layer is drawn. |
| `locked` | `bool` | Whether editing is blocked. |
| `reference` | `bool` | If true, this layer renders at a fixed dim opacity and is excluded from final export — used as a light-table / reference layer. |
| `exposures` | `Vec<Option<CellId>>` | One slot per frame: `Some(cell)` = a key showing that drawing, `None` = hold the previous one. |
| `transform` | `Transform` | Live / static layer transform. Used directly when `transform_keys` is empty; otherwise it is the working buffer for the current frame (synced from the keys when scrubbing, edited live when dragging). |
| `transform_keys` | `Vec<TransformKey>` | Sorted keyframes for the layer transform. Empty = static `transform`. |
| `lines_from` | `Option<usize>` | Index of the layer whose strokes bound this layer's flood fills — the line-art layer for a paint layer. `None` = fill reads this layer's own pixels (plain bucket behaviour). Session-only: not written to `.anim`, so it resets on load. |
| `onion_pins` | `Vec<OnionPin>` | Frames of this layer pinned as onion ghosts in the Onion panel. Session-only like `lines_from`. Frame edits shift them the way they shift `track_points`, so a pin stays on its drawing. |
| `onion` | `bool` | Onion skin kept on for this layer while another one is active. The active layer ghosts either way. Session-only like `onion_pins`. |
| `keys_from` | `Option<u64>` | `Layer::uid` of the layer whose drawings this one's key jumps also stop at — the rough under a cleanup layer. By uid, not index, so it rides through reorders, undo and the Krita reload untouched; a deleted target simply stops resolving. Session-only like `lines_from`. See `crate::doc::project::Project::jumps_at`. |
| `uid` | `u64` | Session-unique identity. Undo snapshots clone it, so a layer keeps it through undo/redo; a load hands out fresh ones. What the Krita link (`crate::krita_link`) keys its layers by — an index shifts with every insert, and a field set at send time would vanish on undoing past it. |
| `track_points` | `Vec<TrackSample>` | Per-frame stabilization tracking samples, parallel to `exposures`. Empty vec = tracker unused on this layer. Project frame edits keep the indices aligned with `exposures`. |
| `cell_w` | `u32` | Cell buffer size for this layer's drawings. `0` = use the project frame size. Larger than the frame lets a single layer's artwork extend past what the camera sees. |
| `cell_h` | `u32` | Height of this layer's drawings (`0` = the project height). See `cell_w`. |
| `clip` | `bool` | Clipped to the layer below: it only shows where the first layer under it that isn't clipped itself — its base — has pixels. See `crate::doc::clip`. |
| `alpha_lock` | `bool` | Painting only changes pixels that already have paint: strokes and fills recolour, never add coverage, and the eraser does nothing. |
| `kind` | `LayerKind` | A drawing layer, or a group's header row. See `crate::doc::groups`.  These three must stay the LAST fields: the `.anim` format (postcard) is positional, and v8 appended them. |
| `in_group` | `bool` | One of the layers of the group whose header sits directly above this one's run. Groups hold layers, never groups. |
| `collapsed` | `bool` | A group header shown folded, its layers out of sight in the panel and the tracks. |

### `enum LayerKind`
[src/doc/layer.rs:118](../../../src/doc/layer.rs#L118)

What a row of the layer stack is.

| Variant | Meaning |
|---|---|
| `Paint` | Drawings on the timeline. |
| `Group` | A group's header: no drawings of its own. Its layers are the run of `in_group` layers directly below it. |

## Free functions

### `next_uid`
[src/doc/layer.rs:14](../../../src/doc/layer.rs#L14)

```rust
pub fn next_uid() -> u64
```

Returns a new unique layer id each call (1, 2, 3, …) from a global atomic counter, safe from any
thread.

## `impl Layer`

### `Layer::new`
[src/doc/layer.rs:128](../../../src/doc/layer.rs#L128)

```rust
pub fn new(name: impl Into<String>, frames: usize) -> Self
```

A new, empty paint layer named `name` with `frames` empty slots (at least one), full opacity, visible,
unlocked, identity transform, the project's cell size, and a fresh `uid`. `vec![None; n]` makes a
`Vec` of `n` `None`s.

### `Layer::group`
[src/doc/layer.rs:155](../../../src/doc/layer.rs#L155)

```rust
pub fn group(name: impl Into<String>, frames: usize) -> Self
```

A new group header row named `name`. Uses struct update syntax: everything from `Layer::new`, except
`kind: LayerKind::Group`.

### `Layer::is_group`
[src/doc/layer.rs:162](../../../src/doc/layer.rs#L162)

```rust
pub fn is_group(&self) -> bool
```

Whether this row is a group header.

### `Layer::cell_size`
[src/doc/layer.rs:168](../../../src/doc/layer.rs#L168)

```rust
pub fn cell_size(&self, pw: u32, ph: u32) -> (u32, u32)
```

The pixel size of this layer's drawings: its own `cell_w`/`cell_h`, or the project size `(pw, ph)` when
those are 0.

### `Layer::track_insert_frame`
[src/doc/layer.rs:177](../../../src/doc/layer.rs#L177)

```rust
pub fn track_insert_frame(&mut self, at: usize)
```

Keeps the tracker points list aligned with `exposures` after a frame is inserted at `at` (inserts an
empty sample); does nothing when the tracker isn't used.

### `Layer::track_remove_frame`
[src/doc/layer.rs:190](../../../src/doc/layer.rs#L190)

```rust
pub fn track_remove_frame(&mut self, at: usize)
```

Keeps tracker points aligned after frame `at` is removed.

### `Layer::pins_insert_frames`
[src/doc/layer.rs:198](../../../src/doc/layer.rs#L198)

```rust
pub fn pins_insert_frames(&mut self, at: usize, n: usize)
```

After `n` frames are inserted at `at`, moves onion pins at or after `at` along by `n`, so each pin
stays on its drawing.

### `Layer::pins_remove_frames`
[src/doc/layer.rs:208](../../../src/doc/layer.rs#L208)

```rust
pub fn pins_remove_frames(&mut self, at: usize, n: usize)
```

After frames `at..at+n` are removed, drops pins on those frames and moves later pins back by `n`.
`retain` keeps only items passing the test (an in-place `filter`).

### `Layer::transform_keys_insert_frames`
[src/doc/layer.rs:219](../../../src/doc/layer.rs#L219)

```rust
pub fn transform_keys_insert_frames(&mut self, at: usize, n: usize)
```

Moves transform keys at or after `at` along by `n` frames after an insert.

### `Layer::transform_keys_remove_frames`
[src/doc/layer.rs:229](../../../src/doc/layer.rs#L229)

```rust
pub fn transform_keys_remove_frames(&mut self, at: usize, n: usize)
```

Drops transform keys on removed frames and moves later ones back by `n`.

### `Layer::resolve_transform`
[src/doc/layer.rs:241](../../../src/doc/layer.rs#L241)

```rust
pub fn resolve_transform(&self, frame: usize) -> Transform
```

The layer's transform on `frame`: the static `transform` if there are no keys; otherwise interpolated
between the keys around `frame` using the left key's easing curve, and held flat before the first key
and after the last. Works the same way as `Camera::resolve`.

**Example.** Keys at frame 0 (x = 0) and frame 10 (x = 100), linear: frame 5 → x = 50; frame 12 → x = 100.

### `Layer::has_transform_key`
[src/doc/layer.rs:265](../../../src/doc/layer.rs#L265)

```rust
pub fn has_transform_key(&self, frame: usize) -> bool
```

Whether there's a transform key exactly on `frame`.

### `Layer::set_transform_key`
[src/doc/layer.rs:271](../../../src/doc/layer.rs#L271)

```rust
pub fn set_transform_key(&mut self, frame: usize, transform: Transform)
```

Adds a transform key on `frame` (or replaces the existing one, keeping its easing), keeping keys
sorted by frame.

### `Layer::set_transform_key_ease`
[src/doc/layer.rs:287](../../../src/doc/layer.rs#L287)

```rust
pub fn set_transform_key_ease(&mut self, frame: usize, ease: Ease)
```

Sets the easing of the key on `frame`, if any.

### `Layer::delete_transform_key`
[src/doc/layer.rs:293](../../../src/doc/layer.rs#L293)

```rust
pub fn delete_transform_key(&mut self, frame: usize)
```

Removes the transform key on `frame`.

### `Layer::resolve`
[src/doc/layer.rs:299](../../../src/doc/layer.rs#L299)

```rust
pub fn resolve(&self, frame: usize) -> Option<CellId>
```

Which drawing shows on `frame`: walks back from `frame` to the nearest key. `None` if no key exists
at or before it (an empty start of the layer).

### `Layer::is_key`
[src/doc/layer.rs:315](../../../src/doc/layer.rs#L315)

```rust
pub fn is_key(&self, frame: usize) -> bool
```

True if `frame` has its own key (rather than holding an earlier one). `is_some_and(Option::is_some)`
reads "the slot exists and it holds `Some`".

### `Layer::set_key`
[src/doc/layer.rs:319](../../../src/doc/layer.rs#L319)

```rust
pub fn set_key(&mut self, frame: usize, cell: CellId)
```

Puts drawing `cell` as a key on `frame` (ignored if `frame` is past the end).

### `Layer::hold`
[src/doc/layer.rs:325](../../../src/doc/layer.rs#L325)

```rust
pub fn hold(&mut self, frame: usize)
```

Turns `frame` into a hold (`None`), so the previous drawing continues.

## Tests

### `mod tests`

Run with `cargo test doc::layer::tests` · 4 tests

| Test | What it checks |
|---|---|
| [`resolve_transform_interpolates_linearly_by_default`](../../../src/doc/layer.rs#L346) | The scroll case: two keys, linear ease, even movement in between. |
| [`resolve_transform_applies_ease_of_the_left_key`](../../../src/doc/layer.rs#L360) | Ease shapes the segment running *out of* the key it sits on. |
| [`set_transform_key_preserves_ease`](../../../src/doc/layer.rs#L377) | Replacing a key keeps the ease already set on it — re-posing a keyframe shouldn't silently reset its timing. |
| [`is_key_distinguishes_keys_from_holds`](../../../src/doc/layer.rs#L393) | Is key distinguishes keys from holds. |

**Test helpers** (used only by the tests above):

- [`keyed`](../../../src/doc/layer.rs#L336) — Keyed.
