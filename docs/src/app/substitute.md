# `src/app/substitute.rs` — reusing drawings across keys

> **Source:** [src/app/substitute.rs](../../../src/app/substitute.rs) · **Module:** `crate::app::substitute` · **Size:** 429 lines · Line links as of commit `f210c6f`

## What this file is for

**Drawing substitution**: a key shows another of its layer's drawings, *shared* rather than copied. A blink or a
mouth shape drawn once can be reused anywhere on the layer, and a fix to it shows everywhere it does. This file holds
the commands behind the timeline's **Drawing** picker and **Make independent**, the numbers drawings go by, and the
picker's thumbnails.

- **Sharing:** two keys hold the same `CellId`. Nothing new in the `.anim` file: holds, saves, Krita export and the
  cell collector already allowed it (Duplicate frame has always made such keys). A drawing is never shared between
  layers; `Project::set_drawing` refuses one from another layer.
- **Numbers:** a layer's non-blank drawings, by id, which is the order they were made in (ids survive a save and
  load in order). The timeline numbers a layer's bars only when some drawing on it shows on more than one key.
- **Substituting** (`substitute_drawing`): the drawing goes on the layer's picked keys, or on a new key at the
  playhead when none are picked, or when one is and the playhead is partway through its hold — where a
  right-click on a held frame leaves them, so a mouth shape starts on the frame clicked. One undo step. A drawing that no longer shows anywhere gets a toast; undo brings
  it back, and until then the history keeps its pixels.
- **Make independent** (`unlink_track_selection`): each picked key that shares its drawing gets a copy (sharing
  pixels until drawn on), so drawing on it changes it alone.
- **Thumbnails:** made on the CPU from the part of the drawing the canvas frame shows, boxed down to 112 px and
  averaged by coverage, a few per frame (`THUMBS_PER_FRAME`). Cached by drawing with the `Canvas::rev` they were made
  from, updated in place when the drawing changes, and parked in `retired_textures` when a project is replaced.

A child module of `app` so it can work with `AppState`'s private state directly.

## How it connects

- **Used by:** [ui/tracks.rs](../ui/tracks.md) (`drawing_picker`, the context menu, bar numbers and hover text),
  [app.rs](../app.md) (`dispatch` for `Action::TrackUnlink`, the per-frame allowance, project load).
- **Uses:** [doc/project.rs](../doc/project.md) (`set_drawing`, `keys_showing`, `unlink_keys`,
  `layer_cell_ids`), [doc/canvas.rs](../doc/canvas.md) (`Canvas::held`).

## Rust concepts in this file

**Adding methods to `AppState` from another file.** See [app/select.rs](select.md) for the same pattern.
More: [Impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

## Types

### `const THUMB_PX`
[src/app/substitute.rs:21](../../../src/app/substitute.rs#L21)

```rust
const THUMB_PX: usize = 112;
```

A thumbnail's longer side, in pixels: sharp at twice the size it is shown at.

### `const THUMBS_PER_FRAME`
[src/app/substitute.rs:25](../../../src/app/substitute.rs#L25)

```rust
const THUMBS_PER_FRAME: u32 = 3;
```

Thumbnails made in one frame. A big drawing takes a few milliseconds; the rest come on the frames after.

### `struct Thumbs`
[src/app/substitute.rs:29](../../../src/app/substitute.rs#L29)

The picker's thumbnails, by drawing, with the pixels they were made from.

| Field | Type | Meaning |
|---|---|---|
| `made` | `HashMap<CellId, (u64, TextureHandle)>` | Each thumbnail made, with the drawing's rev it was made from. |
| `left` | `u32` | Thumbnails still allowed this frame. |

## `impl AppState`

### `AppState::numbered_drawings`
[src/app/substitute.rs:38](../../../src/app/substitute.rs#L38)

```rust
pub fn numbered_drawings(&self, layer: usize) -> Vec<CellId>
```

Layer `layer`'s drawings in the order they are numbered: by when they were made, blanks left out. A drawing's number is its place here, from 1.

### `AppState::drawing_number`
[src/app/substitute.rs:47](../../../src/app/substitute.rs#L47)

```rust
pub fn drawing_number(&self, layer: usize, id: CellId) -> Option<usize>
```

Drawing `id`'s number on layer `layer`, or `None` for a blank.

### `AppState::layer_reuses`
[src/app/substitute.rs:53](../../../src/app/substitute.rs#L53)

```rust
pub fn layer_reuses(&self, layer: usize) -> bool
```

Whether some drawing on layer `layer` shows on more than one key — when its bars are worth numbering.

### `AppState::substitution_keys`
[src/app/substitute.rs:67](../../../src/app/substitute.rs#L67)

```rust
fn substitution_keys(&self) -> (Vec<usize>, bool)
```

The keys a substitution on the active layer goes to: the ones picked on it in the timeline, or the playhead when none are.

### `AppState::substitution_shows`
[src/app/substitute.rs:83](../../../src/app/substitute.rs#L83)

```rust
pub fn substitution_shows(&self) -> Vec<CellId>
```

The drawings a substitution would replace: what those keys show now.

### `AppState::substitute_drawing`
[src/app/substitute.rs:91](../../../src/app/substitute.rs#L91)

```rust
pub fn substitute_drawing(&mut self, id: CellId)
```

Show drawing `id` on the active layer's picked keys, or from the playhead when none are picked, sharing it. One undo step. A drawing that no longer shows anywhere is said so: undo brings it back.

### `AppState::picked_drawings_shared`
[src/app/substitute.rs:132](../../../src/app/substitute.rs#L132)

```rust
pub fn picked_drawings_shared(&self) -> bool
```

Whether any drawing picked in the timeline shares its drawing with another key — what "Make independent" would change.

### `AppState::unlink_track_selection`
[src/app/substitute.rs:145](../../../src/app/substitute.rs#L145)

```rust
pub fn unlink_track_selection(&mut self)
```

Give each drawing picked in the timeline a drawing of its own where it shares one, so drawing on it changes it alone. One undo step.

### `AppState::drawing_thumb`
[src/app/substitute.rs:156](../../../src/app/substitute.rs#L156)

```rust
pub fn drawing_thumb(&mut self, ctx: &egui::Context, id: CellId) -> Option<egui::load::SizedTexture>
```

Drawing `id`'s thumbnail, made now when the frame's allowance has some left, else on a later frame (`None` until then).

### `AppState::refill_thumbs`
[src/app/substitute.rs:184](../../../src/app/substitute.rs#L184)

```rust
pub(super) fn refill_thumbs(&mut self)
```

A new frame's allowance of thumbnails.

### `AppState::retire_drawing_thumbs`
[src/app/substitute.rs:190](../../../src/app/substitute.rs#L190)

```rust
pub(super) fn retire_drawing_thumbs(&mut self)
```

Drop every thumbnail, parked for a frame — a new project's drawings reuse the old ones' ids. See `retired_textures`.

## Free functions

### `thumb_image`
[src/app/substitute.rs:199](../../../src/app/substitute.rs#L199)

```rust
fn thumb_image(c: &Canvas, frame_w: u32, frame_h: u32) -> ColorImage
```

A drawing's thumbnail: the part of it the canvas frame shows when the layer hasn't moved (a layer's drawings can be bigger than the frame, and sit centred on it), boxed down to fit `THUMB_PX`. Averaged by coverage, so a soft edge keeps its colour rather than darkening.

## Tests

### `mod tests`

Run with `cargo test app::substitute::tests` · 9 tests

| Test | What it checks |
|---|---|
| [`drawings_are_numbered_by_when_they_were_made`](../../../src/app/substitute.rs#L278) | Drawings are numbered by when they were made. |
| [`picking_a_drawing_shares_it_on_the_picked_keys_in_one_step`](../../../src/app/substitute.rs#L287) | Picking a drawing shares it on the picked keys in one step. |
| [`with_nothing_picked_it_keys_the_playhead`](../../../src/app/substitute.rs#L308) | With nothing picked it keys the playhead. |
| [`partway_through_the_picked_key_s_hold_it_keys_from_the_playhead`](../../../src/app/substitute.rs#L320) | Partway through the picked key s hold it keys from the playhead. |
| [`several_picked_keys_take_it_wherever_the_playhead_is`](../../../src/app/substitute.rs#L343) | Several picked keys take it wherever the playhead is. |
| [`a_locked_layer_takes_no_substitute`](../../../src/app/substitute.rs#L358) | A locked layer takes no substitute. |
| [`making_a_shared_drawing_independent_copies_it_for_the_picked_key`](../../../src/app/substitute.rs#L368) | Making a shared drawing independent copies it for the picked key. |
| [`a_thumbnail_keeps_its_colour_and_fits_the_frame`](../../../src/app/substitute.rs#L386) | A thumbnail keeps its colour and fits the frame. |
| [`thumbnails_are_made_a_few_a_frame_and_kept_until_drawn_on`](../../../src/app/substitute.rs#L409) | Thumbnails are made a few a frame and kept until drawn on. |

**Test helpers** (used only by the tests above):

- [`keyed`](../../../src/app/substitute.rs#L257) — One layer over 8 frames, drawings keyed on 0, 2 and 4, each with a dot of its own colour.
- [`shows`](../../../src/app/substitute.rs#L273) — Shows.
- [`key_count`](../../../src/app/substitute.rs#L353) — Key count.
