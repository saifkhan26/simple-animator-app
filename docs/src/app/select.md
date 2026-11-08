# `src/app/select.rs` — the selection: mask, gestures and floating pixels

> **Source:** [src/app/select.rs](../../../src/app/select.rs) · **Module:** `crate::app::select` · **Size:** 3102 lines · Line links as of commit `36405ad`

## What this file is for

This file holds everything the app does with a **selection** — the marching-ants
area you draw with the Lasso tool (freehand, polygon, rectangle, ellipse or magic
select) and then paint inside, fill, delete, copy, paste, move, scale, rotate or flip.
About the first 1750 lines are the real code; the rest is a large test module.

Two things live here, and they are easy to mix up (the file's own header warns about this):

- **The mask** (`AppState::sel_mask`) says *which part of the canvas is selected*. It
  lives in **document space** (the coordinates of the whole picture, not of one
  drawing), so it belongs to no drawing and survives changing frame or layer. Every
  paint edit is clipped to it. Each pixel has a coverage value 0–255 (255 = fully
  selected), so edges can be soft.
- **The float** (`AppState::selection`) is a set of *pixels lifted out of one cell*
  (one drawing) through the mask, moving under a **pose** (offset, scale, rotation).
  It exists only while pixels are being moved. Before any other edit happens it
  **lands** — its pixels are stamped back into the cell, and the mask moves with them.

When several layers are picked in the Layers panel, the float gets **followers**
(`AppState::sel_followers`): the same selection lifted out of each other picked
layer's drawing. The float leads (its transform box is the one you drag) and every
follower is posed so it moves the same way on screen.

A web analogy: think of the mask as a CSS `clip-path` stored on the page, and the
float as a `<canvas>` snapshot you cut out and drag around with a CSS transform; only
when you let go is the snapshot painted back onto the page.

This file is a **child module of `src/app.rs`**. It exists only because `app.rs` was
already very long; almost everything in it is a method on `AppState` (the one big
struct that holds the whole running app). Being a child module lets it read and
write `AppState`'s private fields directly. See the first Rust concept below for how
that works.

Words used on this page (see also the [glossary](../../glossary.md)):

- **Cell** — one drawing (a raster image). A layer shows a cell on each frame via the
  **x-sheet** (`exposures`: frame → cell). One cell can be held over several frames.
- **Cell space / document space** — pixel coordinates inside one cell vs. coordinates
  of the whole picture. A layer can have a transform (move/rotate/scale), and a cell
  can be a different size from the picture, so the two differ. A
  [`Placement`](../tools/selection.md) bundles what is needed to convert between them.
- **Lift** — cut the pixels out of the cell (erase them there) and keep them in the float.
- **Stamp / land** — paint the float's pixels back into a cell and record undo.
- **Lossless / pixel-aligned** — a move by whole pixels with no rotation or scale; the
  pixels are copied byte-for-byte instead of being resampled (blurred).
- **Texture** — an image uploaded to the GPU so egui can draw it.

## How it connects

- **Used by:**
  - [app.rs](../app.md) — declares `mod select;`, stores the selection fields on
    `AppState`, re-exports `SelGesture`, and calls in from key handling (`handle_keys`,
    `dispatch`, `undo`, `redo`), pointer handling (`pointer_down`, `pointer_up`,
    `active_cell_clip`), per-frame upkeep (`update` → `sync_selection`,
    `sync_textures` → `sync_tint`), structural edits (`structural_command` →
    `land_float`) and project reset/load (`clear_selection_state`).
  - [ui/shell.rs](../ui/shell.md) — the canvas input code (`draw`) feeds presses,
    drags and releases into `select_down` / `select_move` / `select_up` and the wand
    drag; `paint_canvas` draws the marching ants, transform box and drag preview
    (`sel_drag_path`, `cached_cell_mask`, `lead_placement`); `selection_options` is the
    Selection panel (Select All, Invert, Grow, Fill selection…).
  - [app/lasso_fill.rs](lasso_fill.md) — the Fill tool's lasso uses `set_mask`,
    `active_cell_clip`, `fill_opacity_mut`, `fill_grain`, `land_float`,
    `doc_to_active_cell`, `cancel_gesture`.
  - [app/liquify.rs](liquify.md) — `active_placement`, `active_cell_clip`,
    `land_float_only`, `layer_drawings`.
  - [app/orient.rs](orient.md) — `orient_selection`, `layer_drawings`.
- **Uses:**
  - [tools/select_mask.rs](../tools/select_mask.md) — `SelectionMask` (the
    document-space mask), `SelOp` (Replace/Add/Remove/Intersect), `SelShape`, path
    builders, combine/grow/shrink/feather, conversion to and from cell space.
  - [tools/selection.rs](../tools/selection.md) — `Selection` (the float), `Pose`,
    `Placement`, `Grab`, `HANDLE_PX`, `DragMods`, `grab_at`.
  - [tools/lasso.rs](../tools/lasso.md) — `Mask` (a cell-space coverage mask) and
    `erase_masked`.
  - [tools/tap_lasso.rs](../tools/tap_lasso.md) — `TapLasso`, the freehand-or-polygon
    lasso gesture.
  - [tools/wand.rs](../tools/wand.md) — `WandPick` and `step_tolerance` for magic select.
  - [tools/fill.rs](../tools/fill.md) — `fill_masked`, `Grain`, `step_gap`.
  - [tools/axis_lock.rs](../tools/axis_lock.md) — `square`, `mirror`, `lock_hv`
    (Shift / centre-key behaviour).
  - [doc/canvas.rs](../doc/canvas.md) — `Canvas` (a cell's pixels) and `DirtyRect`.
  - [io/composite.rs](../io/composite.md) — `flatten_doc` for "all visible layers" magic select.
  - [input/button_drag.rs](../input/button_drag.md) — `drag_steps` for the wand's live retune.
  - [undo.rs](../undo.md) — `undo::Command` (`Selection`, `Compound`, pixel patches).
  - Many `AppState` helpers defined in [app.rs](../app.md): `display_transform`,
    `snapshot_pre`, `take_patch`, `mark_dirty`, `pre_replaced`, `edit_cell_on`,
    `painted`, `axis_lock_on`, `move_on`, `centre_on`, `lock_screen_drag`,
    `lock_basis_doc`, `stash_brush`, `load_brush`; `commit_liquify` from
    [app/liquify.rs](liquify.md); `ensure_perspective_grid` from [app/grids.rs](grids.md).

The life of a moved selection:

```text
 draw a shape ──► select_down / select_move / select_up
                     └─► apply_op ──► set_mask  (sel_mask changes, one undo step)

 plain press inside the selection ──► try_grab ──► make_float_from
        (a Selection + followers are created, but nothing is erased yet: lifted = false)

 first drag / nudge / flip ──► touch_selection
        ├─► lift_selection_source   (erase under the float, keep the old cell in float_pre)
        └─► lift_followers          (same for each follower, keeps a cropped copy in pre)

 more drags ──► drag_selection (pose changes; GPU draws the pixels posed, cell untouched)

 then one of:
   land_float  ──► land_float_only: stamp pixels, record lift+stamp+mask move as ONE undo step
   Ctrl+Z      ──► cancel_float: copy the old pixels back, nothing recorded
   Delete      ──► delete_selection: record the lift only (pixels gone)
```

## Rust concepts in this file

**One type's methods split across files.** In JavaScript a class body must be written
in one place. In Rust, data (`struct`) and behaviour (`impl` blocks) are separate, and a
type can have *many* `impl` blocks, even in different files of the same crate. `app.rs`
declares this file as a child module, and this file adds methods to `AppState`:
```rust
// src/app.rs
mod select;                       // load src/app/select.rs as module `app::select`
pub use select::SelGesture;       // re-export one of its types

// src/app/select.rs
use super::{AppState, SelDrag, SelTint};   // `super` = the parent module, app
impl AppState {
    pub fn select_all(&mut self) { /* ... */ }
}
```
```js
// Closest JS idea: patching a prototype from another file
import { AppState } from "./app.js";
AppState.prototype.selectAll = function () { /* ... */ };
```
Unlike a JS prototype patch, this is checked at compile time and the methods look exactly
like ones written in `app.rs`. Because `select` is a *child* of `app`, it can touch
`AppState`'s private fields (`self.clip_cache`, `self.float_pre`…). Visibility words you
will see: `pub` (anyone), `pub(crate)` (anywhere in this program), `pub(super)` (only
the parent module, `app`), and nothing (only this file).
More: [Modules and visibility](../../rust-primer.md#modules-and-visibility),
[impl blocks](../../rust-primer.md#impl-blocks-and-methods)

**`Option`, `.take()` and `let … else`.** "Maybe a value" is `Option<T>` (`Some(x)` or
`None`), never `null`. `.take()` moves the value out and leaves `None` behind in one step,
and `let Some(x) = … else { return; }` unwraps or bails out early:
```rust
let Some(sel) = self.selection.take() else {
    return;                 // nothing was floating
};
// here `sel` is the Selection and self.selection is now None
```
```js
const sel = this.selection; this.selection = null;
if (sel == null) return;
```
`std::mem::take(&mut self.sel_followers)` does the same for any value with a default (a
`Vec` becomes empty), and `std::mem::swap(&mut a, &mut b)` exchanges two buffers without
copying their contents.
More: [Option and Result](../../rust-primer.md#option-and-result)

**Enums that carry data, and `match`.** `SelGesture` is either a `Drag` (with a start,
end…) or a `Lasso` (with a `TapLasso`). `match` must handle every case. You can also
match on a tuple of values at once, as `combined` does:
```rust
match (op, shape) {
    (SelOp::Replace, m) => m,
    (SelOp::Intersect, None) => None,
    (_, None) => before.cloned(),
    (op, Some(b)) => SelectionMask::combine(before, &b, op),
}
```
```js
// JS has no pattern matching; you'd write a chain of ifs:
if (op === "Replace") return shape;
if (op === "Intersect" && shape == null) return null;
if (shape == null) return before;
return combine(before, shape, op);
```
`matches!(value, Pattern if condition)` is a yes/no `match`, used by `polygon_active`.
More: [Enums and match](../../rust-primer.md#enums-and-match)

**`Arc` for a cheap shared copy.** The selection as one cell sees it is an
`Arc<Mask>` — a reference-counted pointer. `c.clone()` copies the pointer, not the
(possibly megabytes of) mask bytes, so the cache can hand the same mask to many callers.
```rust
let c = Arc::new(m.to_cell(&p.xf, p.cw, p.ch, p.pw, p.ph));
self.clip_cache = Some((key, c.clone()));   // cache keeps one handle
Some(c)                                      // caller gets another
```
In JS every object is already shared by reference, so this is what JS does for free;
Rust makes you ask for it.
More: [Smart pointers](../../rust-primer.md#smart-pointers)

**Closures as arguments.** `reshape_selection` takes "some function from a mask and a
number to a new mask" and Grow/Shrink/Feather each pass a different one:
```rust
fn reshape_selection(&mut self, f: impl FnOnce(&SelectionMask, u32) -> Option<SelectionMask>) { … }
pub fn grow_selection(&mut self) { self.reshape_selection(|m, n| m.grow(n)); }
```
```js
reshapeSelection(f) { … } ; growSelection() { this.reshapeSelection((m, n) => m.grow(n)); }
```
`FnOnce` means the closure is called at most once.
More: [Closures](../../rust-primer.md#closures)

**Returning a mutable reference.** `fill_opacity_mut` returns `&mut f32` — a pointer
to *one of two* fields the caller can then read or write (`*st.fill_opacity_mut() = 0.5`).
JS cannot hand out a reference to a number; you would return an object and a key
instead. The `*` dereferences (reads/writes through) the reference.
More: [Ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)

## Types

### `enum SelGesture`
[src/app/select.rs:41](../../../src/app/select.rs#L41)

A selection gesture still in progress (the pen is down, or a polygon is being tapped
out), in document coordinates. Derives `Clone` and `Debug`. Stored in
`AppState::sel_gesture` and re-exported from `app` as `app::SelGesture`, because
[ui/shell.rs](../ui/shell.md) reads it to draw the preview.

| Variant | Meaning |
|---|---|
| `Drag { op, shape, start, end }` | A rectangle or ellipse: one press, drag, release. `max_dist` is the furthest the pointer got from `start`; it tells a click from a drag. |
| `Lasso { op: SelOp, lasso: TapLasso }` | The lasso: dragged freehand, or tapped out as a polygon with freehand stretches between corners. |

### `struct WandDrag`
[src/app/select.rs:58](../../../src/app/select.rs#L58)

A magic select (wand) press that is still under the pen. It remembers what the press
reads and which selection it started from, so dragging can retune it live and Esc can
put the old selection back. Stored in `AppState::wand_drag`. All fields are private.

| Field | Type | Meaning |
|---|---|---|
| `pick` | `WandPick` | The worked-out pick from [tools/wand.rs](../tools/wand.md); can be re-run at new settings. |
| `at` | `Option<Placement>` | Where the pixels it reads sit: the active cell, or `None` when it reads the flattened document (all visible layers). |
| `op` | `SelOp` | How the pick combines with the old selection. |
| `before` | `Option<SelectionMask>` | The selection before the press. |
| `anchor` | `Option<egui::Pos2>` | The screen point of the press; drags are measured from here. |
| `base_tolerance` | `u8` | The wand's tolerance at the press. |
| `base_gap` | `u8` | The wand's gap setting at the press. |

### `struct Follower`
[src/app/select.rs:74](../../../src/app/select.rs#L74)

Another picked layer's pixels, lifted through the selection together with the float
and moving under the float's box. Stored in `AppState::sel_followers`.

| Field | Type | Meaning |
|---|---|---|
| `li` | `usize` | Its layer index. Stays valid while it floats, because every structural edit lands the float first. |
| `sel` | `Selection` | Its own floating pixels, mask, pose and placement. |
| `tex` | `Option<TextureHandle>` | Its pixels on the GPU; rebuilt only when they change (by `sync_textures` in [app.rs](../app.md)). |
| `tex_stale` | `bool` | `true` when `tex` must be rebuilt. |
| `pre` | `Vec<u8>` | The drawing's pixels under the mask's box *before* the lift, rows of `pre_rect` packed together. An undo while floating puts these back, and landing uses them as the "before" for undo. Empty until lifted, and for pasted pixels. |
| `pre_rect` | `Option<DirtyRect>` | Which rectangle `pre` covers. `None` until lifted, and for pasted pixels. |

### `struct ClipPart`
[src/app/select.rs:134](../../../src/app/select.rs#L134)

What Copy took from one layer. `AppState::pixel_clip` (the selection clipboard) is a
`Vec` of these, the lead layer's first. Derives `Clone`.

| Field | Type | Meaning |
|---|---|---|
| `layer` | `u64` | The uid (stable id) of the layer it was copied from, so a multi-layer paste can put it back there. |
| `mask` | `Mask` | The coverage mask (in the source cell's pixels). |
| `pixels` | `Vec<u8>` | The copied RGBA pixels, `mask.w * mask.h * 4` bytes. |

### `struct ClipKey`
[src/app/select.rs:159](../../../src/app/select.rs#L159)

The key for the one-slot cell-mask cache (`AppState::clip_cache`): what the cached mask
was built for. Derives `Clone`, `Copy`, `PartialEq` (so two keys can be compared with `==`).

| Field | Type | Meaning |
|---|---|---|
| `ver` | `u64` | The selection's version (`AppState::sel_ver`) when it was built. |
| `at` | `Placement` | Where the cell sat (transform, cell size, picture size). |

### `const CLICK_PX`
[src/app/select.rs:167](../../../src/app/select.rs#L167)

```rust
const CLICK_PX: f32 = 3.0;
```

`3.0` screen pixels. A press that travels less than this is a *click*, not a drag: on a
rectangle or ellipse in Replace mode it deselects; on the lasso it places a polygon corner.

## `impl Follower`

### `Follower::new`
[src/app/select.rs:90](../../../src/app/select.rs#L90)

```rust
fn new(li: usize, sel: Selection) -> Self
```

**What it does.** Builds a follower for layer `li` around the floating `sel`, with no
texture yet (`tex_stale: true`, so one gets built) and no saved "before" pixels
(`pre` empty, `pre_rect: None`) — those are filled in by `lift_followers` when it lifts.

**Used by:** `AppState::paste_selection`, `AppState::make_float_from` (this file).

## Free functions

### `crop_rows`
[src/app/select.rs:103](../../../src/app/select.rs#L103)

```rust
fn crop_rows(pixels: &[u8], width: u32, r: DirtyRect) -> Vec<u8>
```

**What it does.** Copies the rectangle `r` out of an RGBA buffer that is `width` pixels
wide, packing its rows one after another into a new `Vec<u8>`.

**Parameters.**
- `pixels` — the whole image, 4 bytes (R, G, B, A) per pixel, row after row.
- `width` — the image width in pixels (each row is `width * 4` bytes, the "stride").
- `r` — the rectangle; `min_x..max_x` by `min_y..max_y` (max is exclusive).

**Returns.** `(max_x − min_x) * 4 * (max_y − min_y)` bytes.

**Example.** The same idea in JavaScript:
```js
function cropRows(pixels, width, r) {
  const stride = width * 4, x0 = r.minX * 4, x1 = r.maxX * 4;
  const out = [];
  for (let y = r.minY; y < r.maxY; y++)
    out.push(...pixels.subarray(y * stride + x0, y * stride + x1));
  return Uint8Array.from(out);
}
```

**Used by:** `AppState::lift_followers` (keeps a follower's pixels before its lift).

### `uncrop_rows`
[src/app/select.rs:113](../../../src/app/select.rs#L113)

```rust
fn uncrop_rows(pixels: &mut [u8], width: u32, r: DirtyRect, rows: &[u8])
```

**What it does.** The reverse of `crop_rows`: writes packed `rows` back into rectangle `r`
of the `width`-wide buffer `pixels`, in place. `rows.chunks_exact(n)` walks the packed
data one row (`n` bytes) at a time, and `.zip` pairs each row with its `y`.

**Used by:** `AppState::land_follower` (rebuilds the follower's "before" image for undo)
and `AppState::cancel_float` (puts a follower's pixels back).

### `mask_rect`
[src/app/select.rs:121](../../../src/app/select.rs#L121)

```rust
fn mask_rect(mask: &Mask, canvas: &Canvas) -> Option<DirtyRect>
```

**What it does.** The mask's bounding box clipped to the canvas, or `None` when the two do
not overlap. `cond.then_some(r)` is Rust shorthand for `cond ? r : null`.

**Used by:** `AppState::lift_followers`.

### `combined`
[src/app/select.rs:143](../../../src/app/select.rs#L143)

```rust
fn combined(
    op: SelOp,
    before: Option<&SelectionMask>,
    shape: Option<SelectionMask>,
) -> Option<SelectionMask>
```

**What it does.** Combines a newly drawn `shape` into the selection `before` using mode
`op`. A `shape` of `None` means "a shape that covered nothing".

**Returns.** The new selection (`None` = nothing selected):

**Used by:** `AppState::apply_op`, `AppState::wand_show`.

### `dist`
[src/app/select.rs:169](../../../src/app/select.rs#L169)

```rust
fn dist(a: (f32, f32), b: (f32, f32)) -> f32
```

**What it does.** Straight-line distance between two points (`hypot` = √(dx² + dy²),
like `Math.hypot`).

**Used by:** `AppState::select_move` (tracks `max_dist` of a rectangle/ellipse drag).

### `pose_for`
[src/app/select.rs:184](../../../src/app/select.rs#L184)

```rust
fn pose_for(sel: &Selection, pk: &Placement, mk: &Mask) -> Pose
```

**What it does.** Works out the pose that moves *another* drawing — one cut through mask
`mk` and placed at `pk` — the same way the float `sel` has moved **on screen**. This is
how followers and "All frames" drawings keep up with the float even when their layer
has a different transform on that frame or their canvas is a different size.

**Parameters.**
- `sel` — the lead float (its placement, mask and current pose).
- `pk` — where the other drawing sits.
- `mk` — the other drawing's view of the selection mask.

**Returns.** A `Pose` with the float's scale and rotation and a recomputed `offset`.

**How it works.**
1. If the other drawing sits exactly where the float's did (same placement, same mask
   box), the float's pose is returned unchanged — a plain move stays a lossless
   whole-pixel copy.
2. Otherwise take the centre of the other mask's box (`ck`), convert it from the other
   cell's space to document space, then into the float's cell space (`c0`).
3. Send that point through the float's motion (`sel.path_point`), back to document
   space, and into the other cell's space (`target`).
4. The offset is `target − ck`. If the float's pose is pixel-aligned and both
   placements share rotation and scale, round the offset to whole pixels so it stays
   on the lossless path.
5. Return the float's pose with this new offset (`Pose { offset, ..sel.pose }` copies
   every other field from `sel.pose`, like JS `{ ...sel.pose, offset }`).

The source notes that rotation and uniform scale carry over exactly; a non-uniform
scale is only exact while both frames share a layer rotation, because a pose can only
scale along the drawing's own axes.

**Used by:** `AppState::land_on_other_drawings`, `AppState::pose_followers`.

## `impl AppState`

### `AppState::active_placement`
[src/app/select.rs:210](../../../src/app/select.rs#L210)

```rust
pub fn active_placement(&self) -> Placement
```

**What it does.** Where the active layer's cell sits in the document right now: the
layer's live transform and the size of the cell a stroke here would paint into.
Shortcut for `placement_of(current_layer)`.

**Used by:** [app/liquify.rs](liquify.md) (`start_liquify`); in this file
`doc_to_active_cell`, `lead_placement`, `active_cell_mask`, `wand_down`.

### `AppState::placement_of`
[src/app/select.rs:216](../../../src/app/select.rs#L216)

```rust
pub fn placement_of(&self, li: usize) -> Placement
```

**What it does.** Where layer `li`'s drawing on the current frame sits: its displayed
transform (`display_transform`, which uses the live edit buffer for the active layer),
its cell size (`draw_cell_size`), and the picture size.

**Used by:** this file only (`active_placement`, `lead_placement`, `layer_cell_mask`,
`paste_selection`, `make_float_from`).

### `AppState::doc_to_active_cell`
[src/app/select.rs:229](../../../src/app/select.rs#L229)

```rust
pub fn doc_to_active_cell(&self, doc: (f32, f32)) -> (f32, f32)
```

**What it does.** Converts a document point into the active cell's pixel coordinates.

**Used by:** [app.rs](../app.md) (`slide_shape`, `lock_basis_cell`),
[app/lasso_fill.rs](lasso_fill.md) (`land_lasso_fill`), [ui/shell.rs](../ui/shell.md)
(`draw`); `wand_down` here.

### `AppState::sel_layers`
[src/app/select.rs:239](../../../src/app/select.rs#L239)

```rust
pub fn sel_layers(&self) -> Vec<usize>
```

**What it does.** The layers a selection edit (fill, clear, delete, move…) works on,
bottom first: the active layer whenever it may be edited, plus every other layer picked
in the Layers panel (a picked group counts as its layers) that is visible and editable.
Nothing hidden moves.

**How it works.** Starts from `selected_drawing_layers()` (in [app.rs](../app.md)) and
keeps `li` when `paintable(li)` and it is either the active layer or `shows(li)`.

**Used by:** [ui/shell.rs](../ui/shell.md) (`selection_options`); here `lift_layers`,
`fill_selection`, `erase_sel_layers`.

### `AppState::lift_layers`
[src/app/select.rs:252](../../../src/app/select.rs#L252)

```rust
pub(super) fn lift_layers(&self) -> Vec<usize>
```

**What it does.** The `sel_layers` that actually have a drawing on this frame — the ones a
lift takes pixels from — with the **lead** moved to the front. The lead is the active
layer; if that one cannot lead (a group, locked, nothing drawn here) it is the topmost
of the rest. Empty when there is nothing to lift.

**How it works.** Filters `sel_layers()` by `layers[li].resolve(frame).is_some()` (the
x-sheet has a cell there), finds the active layer's position or else the last index
(`out.len().checked_sub(1)` gives `None` for an empty list instead of underflowing), and
moves that entry to index 0.

**Used by:** this file (`lead_placement`, `mask_is_grabbable`, `copy_selection`,
`make_float`, `make_float_from`).

### `AppState::lead_placement`
[src/app/select.rs:272](../../../src/app/select.rs#L272)

```rust
pub fn lead_placement(&self) -> Placement
```

**What it does.** Where the drawing that owns the transform box sits: the float's
placement if something is floating; otherwise the first of `lift_layers`; otherwise the
active cell. Every press, drag and handle on the box is measured in that drawing's pixels.

**Used by:** [ui/shell.rs](../ui/shell.md) (`paint_canvas`); here `doc_to_lead_cell`,
`lead_view_scale`, `lead_cell_mask`, `cached_cell_mask`.

### `AppState::doc_to_lead_cell`
[src/app/select.rs:283](../../../src/app/select.rs#L283)

```rust
pub fn doc_to_lead_cell(&self, doc: (f32, f32)) -> (f32, f32)
```

**What it does.** Converts a document point into the lead drawing's pixel coordinates.

**Used by:** this file (`lock_basis_lead`, `select_move`, `transform_grab_at`, `try_grab`).

### `AppState::lead_view_scale`
[src/app/select.rs:291](../../../src/app/select.rs#L291)

```rust
fn lead_view_scale(&self) -> f32
```

**What it does.** How many screen pixels one lead-drawing pixel covers: the view zoom
times the size of the layer's scale, never below `1e-6` (so dividing by it is safe). It
mirrors `cell_view_scale` in [app.rs](../app.md), which does this for the active cell.

**Used by:** `transform_grab_at`, `try_grab` (to turn `HANDLE_PX` screen pixels into a
grab tolerance in cell pixels).

### `AppState::lock_basis_lead`
[src/app/select.rs:296](../../../src/app/select.rs#L296)

```rust
fn lock_basis_lead(&self) -> f32
```

**What it does.** The Shift axis-lock direction (`lock_basis_doc`, an angle in document
space) expressed as an angle in the lead drawing's pixels. It converts two document
points — the origin and a point 100 units along the angle — into lead-cell space and
takes the angle between them with `atan2`.

**Used by:** `drag_selection` (Shift-locked moves).

---

**Section: the mask** (`// --- The mask ---`)

### `AppState::set_mask`
[src/app/select.rs:307](../../../src/app/select.rs#L307)

```rust
pub(crate) fn set_mask(&mut self, m: Option<SelectionMask>, record: bool)
```

**What it does.** Replaces the selection with `m`. Does nothing if it is unchanged.

**Parameters.**
- `m` — the new mask (`None` = select nothing).
- `record` — `true` pushes an undo step (`undo::Command::Selection` with the old and new
  masks, both `pack`ed = run-length compressed). Undo and redo themselves pass `false`.

**Used by:** [app.rs](../app.md) (`apply_touched`, which replays undo/redo);
[app/lasso_fill.rs](lasso_fill.md) tests; here `apply_op`, `select_invert`, `deselect`,
`reshape_selection`.

### `AppState::mask_changed`
[src/app/select.rs:322](../../../src/app/select.rs#L322)

```rust
fn mask_changed(&mut self)
```

**What it does.** Invalidates everything derived from `sel_mask`: bumps `sel_ver`
(`wrapping_add` wraps around instead of panicking on overflow), rebuilds the
marching-ants outline (`contours`, or an empty list when there is no mask), and drops
the cell-mask cache.

**Used by:** `set_mask`, `cancel_gesture`, `wand_show`, `land_float_only`,
`clear_selection_state`.

### `AppState::active_cell_mask`
[src/app/select.rs:336](../../../src/app/select.rs#L336)

```rust
pub fn active_cell_mask(&mut self) -> Option<Arc<Mask>>
```

**What it does.** The selection as the active cell sees it (in that cell's pixels), or
`None` when nothing is selected. Select All comes back as the whole cell. It is cached
until the selection, the layer's placement or the cell size changes, because a stroke
asks for it at every pen-down.

**Used by:** [ui/shell.rs](../ui/shell.md) tests; here `active_cell_clip`.

### `AppState::lead_cell_mask`
[src/app/select.rs:343](../../../src/app/select.rs#L343)

```rust
pub fn lead_cell_mask(&mut self) -> Option<Arc<Mask>>
```

**What it does.** The selection as the lead drawing (the one the transform box is drawn
in) sees it. Same as `active_cell_mask` unless another picked layer leads.

**Used by:** `sync_selection`, `transform_grab_at`, `try_grab`, `make_float`.

### `AppState::cell_mask_at`
[src/app/select.rs:350](../../../src/app/select.rs#L350)

```rust
fn cell_mask_at(&mut self, at: Placement) -> Option<Arc<Mask>>
```

**What it does.** The selection as a drawing placed `at` sees it, through the one-slot
cache `clip_cache`.

**How it works.**
1. No mask → `None` (the `?` after `self.sel_mask.as_ref()` returns early).
2. Build the key `ClipKey { ver: sel_ver, at }`. If the cache holds that key, return
   a clone of its `Arc` (cheap).
3. Otherwise convert the document mask into cell space (`SelectionMask::to_cell`), store
   it in the cache, and return it.

**Example.** A one-slot memo in JS:
```js
cellMaskAt(at) {
  if (!this.selMask) return null;
  const key = { ver: this.selVer, at };
  if (this.clipCache && equal(this.clipCache.key, key)) return this.clipCache.mask;
  const mask = this.selMask.toCell(at);
  this.clipCache = { key, mask };
  return mask;
}
```

**Used by:** `active_cell_mask`, `lead_cell_mask`, `layer_cell_mask`.

### `AppState::layer_cell_mask`
[src/app/select.rs:367](../../../src/app/select.rs#L367)

```rust
fn layer_cell_mask(&mut self, li: usize) -> Option<Arc<Mask>>
```

**What it does.** The selection as layer `li`'s drawing on this frame sees it. For the
active layer it goes through the cache. For another layer it reuses the cache only if it
happens to match; otherwise it cuts a fresh mask **without** storing it, leaving the
cache for the active or lead drawing, which ask every frame.

**Used by:** `fill_selection`, `erase_sel_layers`, `copy_selection`, `make_float_from`.

### `AppState::active_cell_clip`
[src/app/select.rs:383](../../../src/app/select.rs#L383)

```rust
pub fn active_cell_clip(&mut self) -> Option<Arc<Mask>>
```

**What it does.** The clip a paint edit on the active cell is held to. `None` means "paint
freely": either nothing is selected or everything is (`is_all`), so there is no point
clipping.

**Used by:** [app.rs](../app.md) (`pointer_down`, `pointer_up` — strokes and fills),
[app/lasso_fill.rs](lasso_fill.md) (`land_lasso_fill`), [app/liquify.rs](liquify.md)
(`start_liquify`).

### `AppState::cached_cell_mask`
[src/app/select.rs:394](../../../src/app/select.rs#L394)

```rust
pub fn cached_cell_mask(&self) -> Option<&Mask>
```

**What it does.** The lead drawing's cell mask, but only if the cache is current for it.
It takes `&self` (read-only), so it can be called while drawing the transform box, where
there is no `&mut` access to build a new one. `sync_selection` keeps the cache warm while
the Lasso tool needs it.

**Used by:** [ui/shell.rs](../ui/shell.md) (`paint_canvas`).

### `AppState::mask_is_grabbable`
[src/app/select.rs:407](../../../src/app/select.rs#L407)

```rust
pub fn mask_is_grabbable(&self) -> bool
```

**What it does.** Whether a plain press on the selection should lift its pixels. Only
when the selection is bounded (`outside == 0`, meaning everything outside its box is
unselected) and there is an editable drawing to lift from. After Select All or Invert
every drag would otherwise become a move, leaving no way to draw a new selection.
`is_some_and(|m| …)` is "is `Some` and the test passes".

**Used by:** [ui/shell.rs](../ui/shell.md) (`paint_canvas`); here `sync_selection`,
`transform_grab_at`, `try_grab`.

### `AppState::sync_selection`
[src/app/select.rs:413](../../../src/app/select.rs#L413)

```rust
pub(super) fn sync_selection(&mut self)
```

**What it does.** Per-frame upkeep: lands a float whose cell is no longer the one showing
(`land_float_if_orphaned`), and while the Lasso tool is active with a grabbable,
unlifted selection, asks for `lead_cell_mask()` so the cache is warm for
`paint_canvas`. (`let _ = …` throws the result away on purpose.)

**Used by:** [app.rs](../app.md) (`update`, every frame).

### `AppState::selection_universe`
[src/app/select.rs:424](../../../src/app/select.rs#L424)

```rust
pub(super) fn selection_universe(&self) -> (i32, i32, i32, i32)
```

**What it does.** The document box `(x0, y0, x1, y1)` every new selection is clipped to:
the picture frame plus every layer's cell on this frame, wherever its transform puts it,
padded by 2 pixels. Nothing outside can be painted anyway, and it stops a lasso drawn
while zoomed far out from asking for a gigapixel mask.

**How it works.** Starts with the picture rectangle, sends the four corners of each
layer's cell through `cell_to_doc`, grows the box to include them, then floors/ceils
and pads.

**Used by:** [app/lasso_fill.rs](lasso_fill.md) tests; here `select_up`, `apply_lasso`.

### `AppState::apply_op`
[src/app/select.rs:450](../../../src/app/select.rs#L450)

```rust
pub fn apply_op(&mut self, op: SelOp, shape: Option<SelectionMask>)
```

**What it does.** Combines a freshly drawn shape into the selection (via `combined`) as
one undo step. Lands any float first. `None` is a shape that covered nothing.

**Used by:** `select_up`, `apply_lasso`, `wand_down`, `select_all`.

---

**Section: gestures** (`// --- Gestures ---`)

The canvas code in [ui/shell.rs](../ui/shell.md) turns pen/mouse events into three calls,
much like `pointerdown` / `pointermove` / `pointerup` handlers in the browser:
`select_down` → `select_move` (many times) → `select_up`.

### `AppState::select_down`
[src/app/select.rs:460](../../../src/app/select.rs#L460)

```rust
pub fn select_down(&mut self, doc: (f32, f32), held: Option<SelOp>)
```

**What it does.** A selection press at document point `doc`. Decides whether this press
adds a polygon corner, grabs the existing selection to move it, starts a magic select,
or starts a new rectangle/ellipse/lasso.

**Parameters.**
- `doc` — the press point in document space.
- `held` — the mode a held modifier key asks for (e.g. Add), which beats the sticky
  `sel_op` for this one drag. `None` when no mode key is held.

**How it works.**
1. Stops playback.
2. If a polygon is being tapped out: a press near the first corner (within
   1.5 × `HANDLE_PX` screen pixels) closes it (`polygon_finish`); anywhere else
   `TapLasso::press` adds a corner. Return.
3. `op` = `held` or the sticky `sel_op`.
4. With no held modifier and Replace mode, try to grab the selection (`try_grab`). If
   that took the press, remember the point in `wand_repick` when the shape is the wand
   (so a press that never moves picks there instead), and return. A held modifier
   always starts a new shape, even inside the selection — that is how you add to it
   from within.
5. Magic select: `wand_down(doc, op)` and return.
6. Otherwise land any float and start a new gesture: `SelGesture::Lasso` with
   `TapLasso::start(doc)` for the freehand shape, or `SelGesture::Drag` for a rectangle
   or ellipse.

**Used by:** [ui/shell.rs](../ui/shell.md) (`draw`, the canvas input).

### `AppState::select_move`
[src/app/select.rs:506](../../../src/app/select.rs#L506)

```rust
pub fn select_move(&mut self, doc: (f32, f32))
```

**What it does.** The pointer, still pressed, is now at document point `doc`.

**How it works.**
1. If the transform box is being dragged (`sel_drag`), convert `doc` into lead-cell
   space and hand it to `drag_selection`. Return.
2. Work out the lasso sample `step` (finer than one document pixel when zoomed in, one
   pixel when zoomed out) and the tap distance (`CLICK_PX` in screen pixels, turned into
   document units), and whether the move key (Space by default) is held (`move_on`).
3. For a `Drag` gesture: if the move key is held, slide `start` along with the pointer
   (so the whole box moves); set `end = doc`; update `max_dist`.
4. For a `Lasso` gesture: `lasso.drag(doc, tap, step)`.

`match &mut self.sel_gesture { Some(SelGesture::Drag { start, end, .. }) => … }` borrows
the fields *inside* the enum so they can be changed in place (`*end = doc`). `..` means
"ignore the other fields".

**Used by:** [ui/shell.rs](../ui/shell.md) (`draw`).

### `AppState::select_up`
[src/app/select.rs:538](../../../src/app/select.rs#L538)

```rust
pub fn select_up(&mut self)
```

**What it does.** The pointer was released: finish whatever the press started.

**How it works.**
1. Take `wand_repick`.
2. If a transform-box drag was in progress: if the float was never actually lifted (the
   press never moved), throw the candidate float and its followers away so nothing
   changes — and with magic select, pick afresh at the press point (`wand_down` +
   `wand_up` in Replace mode). Return.
3. If a magic select press was in progress, `wand_up` lands it. Return.
4. A `Drag` gesture: if it moved less than `CLICK_PX` screen pixels it is a click — in
   Replace mode that deselects; otherwise nothing. If it was a real drag, build the
   outline with `sel_drag_path`, rasterise it with `SelectionMask::from_path` (clipped to
   `selection_universe`), and `apply_op`.
5. A `Lasso` gesture: if `lasso.release()` says a freehand outline is finished,
   `apply_lasso`; otherwise (a tap) put the gesture back — the user is now tapping out a
   polygon.

**Used by:** [ui/shell.rs](../ui/shell.md) (`finish_canvas_drag`).

### `AppState::sel_drag_path`
[src/app/select.rs:592](../../../src/app/select.rs#L592)

```rust
pub fn sel_drag_path(&self, shape: SelShape, start: (f32, f32), end: (f32, f32)) -> Vec<(f32, f32)>
```

**What it does.** The outline (a list of document points) a rectangle or ellipse
selection dragged from `start` to `end` makes. While Shift's axis lock is on it becomes a
square or circle (`axis_lock::square`); while the centre key (Alt) holds, `start` is the
middle and the shape is mirrored round it (`axis_lock::mirror`). Both the on-screen
preview and the final selection use this, so they always match.

**Returns.** `select_mask::ellipse_path(...)` for `Ellipse`, `rect_path(...)` otherwise.

**Used by:** [ui/shell.rs](../ui/shell.md) (`paint_canvas`, the preview); `select_up` here.

### `AppState::apply_lasso`
[src/app/select.rs:611](../../../src/app/select.rs#L611)

```rust
fn apply_lasso(&mut self, op: SelOp, lasso: TapLasso)
```

**What it does.** Combines a finished lasso outline into the selection. `lasso.finish()`
returns `None` for fewer than three distinct points, in which case nothing changes;
otherwise the outline becomes a mask (`SelectionMask::from_path`) and goes through
`apply_op`.

**Used by:** `select_up`, `polygon_finish`.

### `AppState::polygon_active`
[src/app/select.rs:619](../../../src/app/select.rs#L619)

```rust
pub fn polygon_active(&self) -> bool
```

**What it does.** `true` while a lasso is being tapped out as a polygon (corner by
corner). Written with `matches!(value, Pattern if guard)`.

**Used by:** [app.rs](../app.md) (`handle_keys` — Enter closes, other keys behave
differently), [ui/shell.rs](../ui/shell.md) (`draw`); here `select_down`,
`polygon_finish`, `transform_grab_at`.

### `AppState::polygon_finish`
[src/app/select.rs:624](../../../src/app/select.rs#L624)

```rust
pub fn polygon_finish(&mut self)
```

**What it does.** Closes the polygon in progress and applies it (`apply_lasso`). Does
nothing if no polygon is in progress.

**Used by:** [app.rs](../app.md) (`handle_keys`); here `select_down`,
`polygon_double_click`.

### `AppState::polygon_double_click`
[src/app/select.rs:635](../../../src/app/select.rs#L635)

```rust
pub fn polygon_double_click(&mut self)
```

**What it does.** Called on a double-click on the canvas: closes the polygon, but only
when both presses landed on the same spot (`TapLasso::doubled`, within `HANDLE_PX`
screen pixels). egui reports any two quick clicks as a double-click, so corners tapped
fast in different places must not close it.

**Used by:** [ui/shell.rs](../ui/shell.md) (`draw`).

### `AppState::polygon_back`
[src/app/select.rs:645](../../../src/app/select.rs#L645)

```rust
pub fn polygon_back(&mut self) -> bool
```

**What it does.** Takes back the newest polygon corner, together with any freehand
stretch drawn from it. If that leaves nothing, the gesture is dropped.

**Returns.** `false` when no polygon is in progress, so the key (Backspace, or Ctrl+Z)
can do what it usually does.

**Used by:** [app.rs](../app.md) (`undo`, `handle_keys`).

### `AppState::cancel_gesture`
[src/app/select.rs:660](../../../src/app/select.rs#L660)

```rust
pub fn cancel_gesture(&mut self) -> bool
```

**What it does.** Abandons a selection gesture in progress — also the Fill tool's lasso
(`fill_lasso`), which ends the same ways. Clears `wand_repick`. A magic select press in
progress puts back the selection it started from (without an undo step).

**Returns.** `true` if anything was in progress.

**Used by:** [app.rs](../app.md) (`begin_screen_pick`, `undo`, `redo`, `handle_keys` —
Esc), [ui/shell.rs](../ui/shell.md) (`selection_options`); here `select_all`,
`select_invert`, `deselect`, `fill_selection`, `clear_active`, `paste_selection`,
`set_tool`.

---

**Section: magic select** (`// --- Magic select ---`)

Magic select (the "wand") picks the pixels joined to the one you press on, like the
magic wand in Photoshop. Pressing and *dragging* retunes it live: sideways changes the
**tolerance** (how different a colour may be and still count), up/down changes the
**gap** (line breaks up to this many pixels wide are treated as closed).

### `AppState::wand_down`
[src/app/select.rs:682](../../../src/app/select.rs#L682)

```rust
fn wand_down(&mut self, doc: (f32, f32), op: SelOp)
```

**What it does.** A magic select press at `doc`: picks what joins the pixel there, shows
it at once combined by `op`, and stores a `WandDrag` so the pen can stay down to retune it
(`wand_drag_to`). `wand_up` later records it as one undo step.

**How it works.**
1. Land any float.
2. If the wand reads all visible layers (`wand.all_visible`): flatten the document for
   this frame (`io::composite::flatten_doc`) and seed the pick at the document pixel;
   `at = None`.
3. Otherwise read the active cell: convert `doc` to cell space for the seed, and use the
   cell's pixels if it has the expected size, or an all-transparent buffer when there is
   no drawing here yet; `at = Some(active placement)`.
4. `WandPick::new` returns `None` when the seed is off the pixels — then
   `apply_op(op, None)` (a shape that covered nothing) and return.
5. Build the `WandDrag` (remembering the current mask, tolerance and gap), call
   `wand_show` to display the pick, and store it in `wand_drag`.

**Used by:** `select_down`, `select_up` (the re-pick).

### `AppState::wand_show`
[src/app/select.rs:728](../../../src/app/select.rs#L728)

```rust
fn wand_show(&mut self, drag: &mut WandDrag)
```

**What it does.** Re-runs `drag`'s pick at the wand's current settings, converts it to a
document-space mask, combines it into the selection the press started from, and makes
that the live `sel_mask` (bumping caches via `mask_changed` if it differs). It records
nothing — `wand_up` records once.

**How it works.** The pick (`p`) is a box plus 0/255 coverage in the source pixels.
- Read from a cell whose placement is a whole-pixel offset (`integer_offset` returns
  `Some((ox, oy))`): the pick is just shifted into document space, no copying or
  resampling.
- Read from a transformed cell: `SelectionMask::from_cell` resamples it into document
  space.
- Read from the flattened document (`at` is `None`): it already is document space.
`.normalize()` tidies the mask (it returns `None` when it covers nothing).

**Used by:** `wand_down`, `wand_drag_to`.

### `AppState::wand_anchor`
[src/app/select.rs:766](../../../src/app/select.rs#L766)

```rust
pub fn wand_anchor(&mut self, at: egui::Pos2)
```

**What it does.** Remembers the screen point where a magic select press landed; the drag
is measured from here. `get_or_insert` sets it only the first time.

**Used by:** [ui/shell.rs](../ui/shell.md) (`draw`).

### `AppState::wand_drag_to`
[src/app/select.rs:777](../../../src/app/select.rs#L777)

```rust
pub fn wand_drag_to(&mut self, at: egui::Pos2)
```

**What it does.** The pen, still down on a magic select press, is at screen point `at`.
Moving sideways steps the tolerance and moving up/down steps the gap, each counted from
its value at the press (the same way a Fill press steps its gap). If either changed, the
pick is re-run so the outline follows. The new values stay in the wand's sliders.

**How it works.**
1. Take the `WandDrag` out of `self` (so `self` can be borrowed mutably meanwhile).
2. With an anchor: apply Shift's partway axis lock (`lock_screen_drag`), turn the x
   distance into tolerance steps (`wand::step_tolerance` + `drag_steps`), and the y
   distance into gap steps (`fill::step_gap`; screen y grows downward, and up means more,
   so it uses `anchor.y − at.y`).
3. If `(tolerance, gap)` changed, store them in `self.wand` and call `wand_show`.
4. Put the `WandDrag` back.

The line `use crate::input::button_drag::drag_steps;` inside the function body imports a
name just for this function — Rust allows `use` in any block.

**Used by:** [ui/shell.rs](../ui/shell.md) (`draw`).

### `AppState::wand_dragging`
[src/app/select.rs:798](../../../src/app/select.rs#L798)

```rust
pub fn wand_dragging(&self) -> bool
```

**What it does.** Whether a magic select press is still under the pen.

**Used by:** [ui/shell.rs](../ui/shell.md) (`draw_tool_cursor`).

### `AppState::wand_up`
[src/app/select.rs:804](../../../src/app/select.rs#L804)

```rust
fn wand_up(&mut self) -> bool
```

**What it does.** Ends a magic select press, pushing one `undo::Command::Selection` step
from the selection before the press to the one now showing (only if they differ).

**Returns.** `true` if there was a press to end.

**Used by:** `select_up`.

### `AppState::transform_grab_at`
[src/app/select.rs:824](../../../src/app/select.rs#L824)

```rust
pub fn transform_grab_at(&mut self, doc: (f32, f32)) -> Option<Grab>
```

**What it does.** Tells what a plain press at `doc` *would* take hold of on the transform
box, without taking it (used to pick the mouse cursor and hints). On a float, any part of
it (`Grab::Move`, a `Scale` handle, or the `Rotate` ring). On a selection not lifted yet,
only its handles and ring — not `Move`, because inside an unlifted selection a held
selection mode still starts a new shape. `None` while a polygon is in progress, when the
sticky mode is not Replace, or when the selection cannot be grabbed.

(The source's doc comment above this function starts with a paragraph that describes
`try_grab`; the second paragraph is this function's.)

**Used by:** [ui/shell.rs](../ui/shell.md) (`draw`).

### `AppState::try_grab`
[src/app/select.rs:840](../../../src/app/select.rs#L840)

```rust
fn try_grab(&mut self, doc: (f32, f32)) -> bool
```

**What it does.** A press on the float's transform box — or on the selection itself when
nothing floats yet, which creates a float. Starts a `sel_drag` remembering what was
grabbed, the press point in lead-cell space, and the pose at the press.

**Returns.** `true` when the press was taken.

**How it works.**
1. Convert `doc` to lead-cell space; the grab tolerance is `HANDLE_PX` screen pixels.
2. If a float exists: `sel.grab_at` decides the part; none → `false`.
3. Otherwise, if the mask is grabbable, get the lead cell's mask and test it with
   `selection::grab_at` (at the default, un-moved pose).
4. `make_float_from(&clip)` creates the float and followers (nothing is erased yet).
5. Start `sel_drag` with `Pose::default()`.

**Used by:** `select_down`.

### `AppState::select_all`
[src/app/select.rs:876](../../../src/app/select.rs#L876)

```rust
pub fn select_all(&mut self)
```

Select → All: cancels any selection gesture in progress and replaces the selection with a mask that
selects everything (`SelectionMask::all()`).

### `AppState::select_invert`
[src/app/select.rs:881](../../../src/app/select.rs#L881)

```rust
pub fn select_invert(&mut self)
```

Select → Invert: lands any floating pixels, then flips the selection — nothing selected becomes
everything, otherwise the mask's `inverted()` (which may be `None` if that leaves nothing selected).

### `AppState::deselect`
[src/app/select.rs:892](../../../src/app/select.rs#L892)

```rust
pub fn deselect(&mut self)
```

Select → Deselect: lands any floating pixels where they are, then selects nothing.

### `AppState::grow_selection`
[src/app/select.rs:898](../../../src/app/select.rs#L898)

```rust
pub fn grow_selection(&mut self)
```

Grows the selection outward by `sel_amount` pixels, via `reshape_selection`.

### `AppState::shrink_selection`
[src/app/select.rs:902](../../../src/app/select.rs#L902)

```rust
pub fn shrink_selection(&mut self)
```

Shrinks the selection inward by `sel_amount` pixels, via `reshape_selection`.

### `AppState::feather_selection`
[src/app/select.rs:906](../../../src/app/select.rs#L906)

```rust
pub fn feather_selection(&mut self)
```

Softens the selection's edge over `sel_amount` pixels (partial selection values fading out), via
`reshape_selection`.

### `AppState::reshape_selection`
[src/app/select.rs:910](../../../src/app/select.rs#L910)

```rust
fn reshape_selection(&mut self, f: impl FnOnce(&SelectionMask, u32) -> Option<SelectionMask>)
```

Shared body of Grow/Shrink/Feather. It lands floating pixels, then applies `f` — a closure such as
`|m, n| m.grow(n)` — to the current mask with the amount (at least 1) and installs the result.
Passing the operation in as a closure is how one function serves three menu items; in JS you would
pass `(m, n) => m.grow(n)` the same way.

### `AppState::fill_opacity_mut`
[src/app/select.rs:921](../../../src/app/select.rs#L921)

```rust
pub fn fill_opacity_mut(&mut self) -> &mut f32
```

Returns a *mutable reference* (`&mut f32`) to the Fill tool's opacity, which Fill selection shares:
the live brush's when Fill is the active tool, otherwise the stored copy in `tool_brushes`. Returning
`&mut` lets the caller (a slider) edit whichever one is right without knowing which it is.

### `AppState::fill_grain`
[src/app/select.rs:932](../../../src/app/select.rs#L932)

```rust
pub fn fill_grain(&self, cell: CellId) -> Grain
```

The paper-grain settings a fill on drawing `cell` should use (shared with the Fill tool). "Boil" (a
grain pattern that changes per drawing) gives each drawing its own patch of paper texture.

### `AppState::fill_selection`
[src/app/select.rs:955](../../../src/app/select.rs#L955)

```rust
pub fn fill_selection(&mut self)
```

Edit → Fill selection: fills the selected area with the brush colour at the Fill tool's opacity, on
the current drawing of every selected layer. One undo step, including any keys auto-key creates.

### `AppState::clear_active`
[src/app/select.rs:990](../../../src/app/select.rs#L990)

```rust
pub fn clear_active(&mut self)
```

Clears the current drawing on every selected layer — only inside the selection when there is one.
One undo step.

### `AppState::delete_selection`
[src/app/select.rs:999](../../../src/app/select.rs#L999)

```rust
pub fn delete_selection(&mut self)
```

Deletes what is selected on every selected layer. Floating pixels are simply dropped (they already
left their drawings when lifted). The selection outline itself stays, as in Krita.

### `AppState::erase_sel_layers`
[src/app/select.rs:1035](../../../src/app/select.rs#L1035)

```rust
fn erase_sel_layers(&mut self, whole: bool)
```

The shared eraser behind clear/delete: erases the current drawing of every selected layer through
the selection — or, with `whole` and no selection, entirely. Collects one undo patch per drawing and
records them as one step (`push_parts`).

### `AppState::push_parts`
[src/app/select.rs:1071](../../../src/app/select.rs#L1071)

```rust
fn push_parts(&mut self, mut parts: Vec<undo::Command>)
```

Records a list of undo commands as one undo step: nothing for an empty list, the command itself for
one, a `Command::Compound` wrapping them for several.

### `AppState::copy_selection`
[src/app/select.rs:1086](../../../src/app/select.rs#L1086)

```rust
pub fn copy_selection(&mut self)
```

Copies the selected pixels to the selection clipboard, separately for each layer they come from (the
lead layer first). A scaled or rotated float is "baked" first so you paste what you see; an unmoved
one keeps its original pixels, so a plain copy loses nothing.

### `AppState::paste_selection`
[src/app/select.rs:1135](../../../src/app/select.rs#L1135)

```rust
pub fn paste_selection(&mut self)
```

Pastes the clipboard as floating pixels so you can position them before they land. Pixels copied from
one layer go onto the active drawing (any frame or layer). Pixels copied from several layers go back
onto those layers' current drawings, moving together.

### `AppState::set_tool`
[src/app/select.rs:1188](../../../src/app/select.rs#L1188)

```rust
pub fn set_tool(&mut self, t: ActiveTool)
```

Switches the active tool. First lands floating pixels and cancels any selection gesture (the
transform box and half-drawn polygons belong to the Lasso), stores the old tool's brush and loads the
new one's.

### `AppState::make_float`
[src/app/select.rs:1204](../../../src/app/select.rs#L1204)

```rust
pub fn make_float(&mut self) -> bool
```

Lifts the selection into floating pixels if it isn't already — used by the arrow keys, which move
pixels without a press. Returns whether there is a float now.

### `AppState::make_float_from`
[src/app/select.rs:1220](../../../src/app/select.rs#L1220)

```rust
fn make_float_from(&mut self, clip: &Mask) -> bool
```

Cuts a float through `clip` (the lead drawing's view of the selection), plus a "follower" float on
each other selected layer. Nothing is erased from any drawing until the float first moves.

### `AppState::orient_selection`
[src/app/select.rs:1268](../../../src/app/select.rs#L1268)

```rust
pub(super) fn orient_selection(&mut self, o: crate::doc::orient::Orient) -> bool
```

Mirrors or rotates the selected pixels in place, lifting them first if needed. The float stays up so
it can still be moved, and the whole thing lands (or Ctrl+Z restores it) as one step. Returns `false`
if there was nothing selected.

### `AppState::land_float`
[src/app/select.rs:1290](../../../src/app/select.rs#L1290)

```rust
pub fn land_float(&mut self)
```

"Settle what's in hand" before any edit: lands an open Liquify warp, then any floating pixels. Safe to
call when nothing is in hand, so many edit functions call it first.

### `AppState::land_float_only`
[src/app/select.rs:1299](../../../src/app/select.rs#L1299)

```rust
pub(super) fn land_float_only(&mut self)
```

Puts floating pixels down: stamps them into their drawing (and followers into theirs), moves the
selection to where they landed, and records the lift + stamp + selection change as **one** undo step,
so one Ctrl+Z puts everything back.

### `AppState::land_follower`
[src/app/select.rs:1374](../../../src/app/select.rs#L1374)

```rust
fn land_follower(&mut self, fl: &Follower, stamp: bool) -> Option<undo::Command>
```

For one follower float: records its lift and, if `stamp`, paints its pixels where they float. Returns
its undo patch for the caller's single step.

### `AppState::layer_drawings`
[src/app/select.rs:1400](../../../src/app/select.rs#L1400)

```rust
pub fn layer_drawings(&self, li: usize) -> Vec<(crate::doc::layer::CellId, usize)>
```

Every distinct drawing on layer `li` with the first frame it appears on. A drawing held over several
frames appears once, so an edit applied "to each drawing" happens once per drawing.

### `AppState::land_on_other_drawings`
[src/app/select.rs:1417](../../../src/app/select.rs#L1417)

```rust
fn land_on_other_drawings(&mut self, sel: &Selection) -> Vec<undo::Command>
```

For "apply to all frames": repeats the float's move on every other drawing of its layer — each is cut
through the same selection and moved the same way. Returns their undo patches.

### `AppState::cancel_float`
[src/app/select.rs:1469](../../../src/app/select.rs#L1469)

```rust
pub(super) fn cancel_float(&mut self) -> bool
```

What Ctrl+Z does while pixels float: puts them back where they were lifted from and forgets the
float. Nothing had been recorded yet, so the history is untouched. Returns `true` if there was a float.

### `AppState::nudge_selection`
[src/app/select.rs:1514](../../../src/app/select.rs#L1514)

```rust
pub fn nudge_selection(&mut self, dx: i32, dy: i32)
```

Arrow keys: moves the float by whole pixels. Whole-pixel moves never need resampling, so nudging is
lossless.

### `AppState::drag_selection`
[src/app/select.rs:1529](../../../src/app/select.rs#L1529)

```rust
pub(super) fn drag_selection(&mut self, drag: SelDrag, x: f32, y: f32)
```

Applies a transform-box drag (move, scale or rotate handle) to the float. Each update is computed
from the pose recorded at the press, so the result depends only on where the pointer is now. Moves go
in whole pixels; Shift pressed part-way locks to horizontal/vertical.

### `AppState::touch_selection`
[src/app/select.rs:1571](../../../src/app/select.rs#L1571)

```rust
fn touch_selection(&mut self)
```

Called after any change of pose: the first time, erases the source pixels behind the float (and its
followers), then moves the followers along. Shared by every gesture so a first rotate lifts exactly
like a first move.

### `AppState::lift_followers`
[src/app/select.rs:1586](../../../src/app/select.rs#L1586)

```rust
fn lift_followers(&mut self)
```

Erases each follower's source pixels once, keeping a copy of what was there (only under the mask's
bounding box) so it can be restored or recorded.

### `AppState::pose_followers`
[src/app/select.rs:1609](../../../src/app/select.rs#L1609)

```rust
fn pose_followers(&mut self)
```

Gives every follower the same on-screen movement as the lead float.

### `AppState::drop_followers`
[src/app/select.rs:1620](../../../src/app/select.rs#L1620)

```rust
fn drop_followers(&mut self)
```

Forgets the followers without landing them (for a float that never lifted). `std::mem::take` moves
the `Vec` out and leaves an empty one in its place — needed because the method then calls another
`&mut self` method with it.

### `AppState::retire_follower_texs`
[src/app/select.rs:1625](../../../src/app/select.rs#L1625)

```rust
fn retire_follower_texs(&mut self, followers: Vec<Follower>)
```

Parks the followers' textures in `retired_textures` (freed next frame rather than now, since they may
have been drawn this frame). `filter_map(|fl| fl.tex)` keeps only the followers that had a texture.

### `AppState::lift_selection_source`
[src/app/select.rs:1632](../../../src/app/select.rs#L1632)

```rust
fn lift_selection_source(&mut self, cell: crate::doc::layer::CellId)
```

Erases the source region behind the float, once. Not recorded yet: the drawing as it was is kept in
`float_pre`, and landing records lift and stamp together.

### `AppState::land_float_if_orphaned`
[src/app/select.rs:1660](../../../src/app/select.rs#L1660)

```rust
fn land_float_if_orphaned(&mut self)
```

Lands a float whose drawing is no longer the one showing (you changed frame or layer), so pixels never
hover over a drawing they don't belong to.

### `AppState::retire_float_tex`
[src/app/select.rs:1675](../../../src/app/select.rs#L1675)

```rust
fn retire_float_tex(&mut self)
```

Parks the floating pixels' texture handle in `retired_textures`.

### `AppState::clear_selection_state`
[src/app/select.rs:1683](../../../src/app/select.rs#L1683)

```rust
pub(super) fn clear_selection_state(&mut self)
```

Forgets the selection, float and all, without writing anything back — for File → New/Open, where the
drawings it pointed at are gone.

### `AppState::sync_tint`
[src/app/select.rs:1698](../../../src/app/select.rs#L1698)

```rust
pub(super) fn sync_tint(&mut self, ctx: &egui::Context)
```

Rebuilds the "dim outside the selection" texture when the selection changes. Downsampled so its long
side is at most 2048 texels — it's a dimming wash, not artwork.

## Tests

### `mod tests`

Run with `cargo test app::select::tests` · 59 tests

| Test | What it checks |
|---|---|
| [`a_selection_is_one_undo_step`](../../../src/app/select.rs#L1815) | A selection is one undo step. |
| [`strokes_stay_inside_the_selection`](../../../src/app/select.rs#L1827) | Strokes stay inside the selection. |
| [`a_moved_layer_is_clipped_where_the_selection_is_on_screen`](../../../src/app/select.rs#L1839) | The selection lives in document space: on a layer shifted 10 px right, the clip lands 10 px left in the layer's own pixels. |
| [`dragging_inside_moves_the_pixels_and_the_selection_follows`](../../../src/app/select.rs#L1852) | Dragging inside moves the pixels and the selection follows. |
| [`shift_partway_holds_a_move_to_an_axis`](../../../src/app/select.rs#L1876) | Shift pressed partway through a move holds the float to the line through the press, whichever way it has gone further; let go, it follows the pointer again. Every step stays in whole pixels. |
| [`shift_partway_squares_a_rectangle_selection`](../../../src/app/select.rs#L1898) | Shift partway squares a rectangle selection. |
| [`the_move_key_partway_slides_a_rectangle_selection`](../../../src/app/select.rs#L1910) | The move key partway slides a rectangle selection. |
| [`the_centre_key_partway_grows_an_ellipse_selection_from_the_press`](../../../src/app/select.rs#L1924) | The centre key partway grows an ellipse selection from the press. |
| [`a_held_mode_with_shift_in_it_doesnt_square`](../../../src/app/select.rs#L1941) | A selection mode held as the drag starts has Shift in it: that Shift chose the mode, so the box isn't squared. |
| [`the_stroke_snapshot_stays_the_cell_through_every_kind_of_edit`](../../../src/app/select.rs#L1973) | The stroke snapshot stays the cell through every kind of edit. |
| [`all_frames_moves_every_drawing_once_in_one_undo`](../../../src/app/select.rs#L2058) | All frames moves every drawing once in one undo. |
| [`flipping_a_selection_lands_as_one_undo`](../../../src/app/select.rs#L2093) | Flipping a selection lands as one undo. |
| [`alt_toggled_mid_drag_switches_pivot_without_drift`](../../../src/app/select.rs#L2113) | Alt pressed and let go mid-drag switches the pivot back and forth, and every frame is solved from the press, so nothing creeps. |
| [`undo_while_floating_puts_the_flip_back`](../../../src/app/select.rs#L2142) | Undo while floating puts the flip back. |
| [`all_frames_lands_the_flip_on_every_drawing`](../../../src/app/select.rs#L2157) | All frames lands the flip on every drawing. |
| [`without_all_frames_only_this_drawing_moves`](../../../src/app/select.rs#L2186) | Without all frames only this drawing moves. |
| [`pasted_pixels_land_on_one_drawing_even_with_all_frames`](../../../src/app/select.rs#L2199) | Pasted pixels land on one drawing even with all frames. |
| [`a_keyed_layer_moves_the_same_on_screen`](../../../src/app/select.rs#L2216) | Frame 1's layer is keyed 15 px right, so its drawing's square sits at 85..125 in its own pixels to show up where the others do. The move is the same on screen: 30 px right in every drawing. |
| [`a_rotation_lands_identically_on_every_drawing`](../../../src/app/select.rs#L2239) | A rotation lands identically on every drawing. |
| [`undo_while_floating_puts_the_pixels_back_untouched`](../../../src/app/select.rs#L2274) | Undo while floating puts the pixels back untouched. |
| [`the_selection_survives_frame_and_layer_changes`](../../../src/app/select.rs#L2290) | The selection survives frame and layer changes. |
| [`clear_empties_only_the_selection_and_can_be_undone`](../../../src/app/select.rs#L2301) | Clear empties only the selection and can be undone. |
| [`switching_tool_by_key_lands_the_float`](../../../src/app/select.rs#L2314) | Switching tool by key lands the float. |
| [`a_structural_edit_lands_the_float_first`](../../../src/app/select.rs#L2325) | A structural edit lands the float first. |
| [`a_new_project_forgets_the_selection`](../../../src/app/select.rs#L2336) | A new project forgets the selection. |
| [`held_modes_add_remove_and_intersect`](../../../src/app/select.rs#L2344) | Held modes add remove and intersect. |
| [`a_polygon_takes_a_corner_per_click`](../../../src/app/select.rs#L2363) | On the lasso a tap starts a polygon, a corner per tap after. |
| [`the_lasso_draws_freehand_and_mixes_stretches_into_a_polygon`](../../../src/app/select.rs#L2384) | A drag on the lasso is freehand; between tapped corners a drag adds a freehand stretch, and the shape keeps going until it is closed. |
| [`a_click_deselects_in_replace_mode_only`](../../../src/app/select.rs#L2417) | A click deselects in replace mode only. |
| [`grow_and_fill_act_on_the_selection`](../../../src/app/select.rs#L2427) | Grow and fill act on the selection. |
| [`fill_selection_paints_at_the_fill_tools_opacity`](../../../src/app/select.rs#L2442) | Fill selection paints at the fill tools opacity. |
| [`fill_selection_takes_the_fill_tools_grain`](../../../src/app/select.rs#L2465) | Fill selection takes the fill tools grain. |
| [`delete_erases_inside_and_keeps_the_selection`](../../../src/app/select.rs#L2487) | Delete erases inside and keeps the selection. |
| [`modifiers_on_the_canvas_pick_the_selection_mode`](../../../src/app/select.rs#L2500) | The same, driven through the real canvas: a Ctrl+Shift drag adds, and Ctrl alone still zooms while the Lasso is up. |
| [`a_wand_click_picks_what_joins_it_as_one_undo_step`](../../../src/app/select.rs#L2596) | A wand click picks what joins it as one undo step. |
| [`held_modes_combine_wand_picks`](../../../src/app/select.rs#L2611) | Held modes combine wand picks. |
| [`any_paint_takes_the_lines_and_the_colour_inside_together`](../../../src/app/select.rs#L2625) | Any paint takes the lines and the colour inside together. |
| [`a_click_inside_the_selection_picks_again_and_a_drag_moves_it`](../../../src/app/select.rs#L2636) | A click inside the selection picks again and a drag moves it. |
| [`dragging_a_wand_press_retunes_it_live_and_lands_once`](../../../src/app/select.rs#L2657) | Dragging a wand press retunes it live and lands once. |
| [`esc_mid_press_puts_the_old_selection_back`](../../../src/app/select.rs#L2681) | Esc mid press puts the old selection back. |
| [`a_wand_click_off_the_drawing_picks_nothing`](../../../src/app/select.rs#L2696) | A wand click off the drawing picks nothing. |
| [`a_wand_pick_lands_where_a_moved_layer_shows_it`](../../../src/app/select.rs#L2707) | A wand pick lands where a moved layer shows it. |
| [`all_visible_picks_by_the_lines_on_another_layer`](../../../src/app/select.rs#L2720) | All visible picks by the lines on another layer. |
| [`the_canvas_drags_a_wand_press_in_screen_points`](../../../src/app/select.rs#L2735) | The canvas hands a press and its drag to the wand in screen points. |
| [`a_move_takes_every_picked_layer_along_in_one_undo`](../../../src/app/select.rs#L2811) | A move takes every picked layer along in one undo. |
| [`undo_while_floating_puts_every_layer_back`](../../../src/app/select.rs#L2834) | Undo while floating puts every layer back. |
| [`a_press_that_never_moves_lifts_nothing_anywhere`](../../../src/app/select.rs#L2846) | A press that never moves lifts nothing anywhere. |
| [`hidden_locked_and_unpicked_layers_stay_put`](../../../src/app/select.rs#L2856) | Hidden locked and unpicked layers stay put. |
| [`a_picked_group_moves_its_layers`](../../../src/app/select.rs#L2880) | A picked group moves its layers. |
| [`a_moved_picked_layer_moves_the_same_on_screen`](../../../src/app/select.rs#L2896) | A moved picked layer moves the same on screen. |
| [`all_frames_reaches_every_drawing_of_every_picked_layer`](../../../src/app/select.rs#L2915) | All frames reaches every drawing of every picked layer. |
| [`a_flip_turns_every_picked_layer`](../../../src/app/select.rs#L2937) | A flip turns every picked layer. |
| [`delete_and_clear_work_on_every_picked_layer`](../../../src/app/select.rs#L2955) | Delete and clear work on every picked layer. |
| [`fill_selection_fills_every_picked_layer_in_one_undo`](../../../src/app/select.rs#L2978) | Fill selection fills every picked layer in one undo. |
| [`a_fill_with_auto_key_keys_every_picked_layer`](../../../src/app/select.rs#L2992) | A fill with auto key keys every picked layer. |
| [`pasted_pixels_go_back_on_the_layers_they_came_from`](../../../src/app/select.rs#L3008) | Pasted pixels go back on the layers they came from. |
| [`pixels_copied_off_one_layer_paste_onto_the_active_one`](../../../src/app/select.rs#L3024) | Pixels copied off one layer paste onto the active one. |
| [`with_none_of_its_layers_left_a_paste_lands_on_the_active_one`](../../../src/app/select.rs#L3040) | With none of its layers left a paste lands on the active one. |
| [`wand_timing`](../../../src/app/select.rs#L3062) | What a magic select press costs, end to end, on a 1080p and an 8K frame: a click on the bare background, one inside a ring, and drag steps, each with the frame after it. `cargo test --release wand_timing -- --ignored --nocapture`. |

**Test helpers** (used only by the tests above):

- [`lasso`](../../../src/app/select.rs#L1760) — A fresh state with the Lasso up, drawing rectangles.
- [`paint`](../../../src/app/select.rs#L1770) — Fill `x0..x1, y0..y1` of the active cell with `rgba`, outside the history — the drawing a test starts from.
- [`px`](../../../src/app/select.rs#L1781) — Px.
- [`drag`](../../../src/app/select.rs#L1789) — Drag the current selection shape from `a` to `b` in document space.
- [`at`](../../../src/app/select.rs#L1798) — At.
- [`ink_line`](../../../src/app/select.rs#L1803) — An Ink stroke along `y`, from `x0` to `x1`, in cell space.
- [`assert_pre_mirrors`](../../../src/app/select.rs#L1952) — Once an edit lands, the stroke snapshot either is the cell it names, byte for byte, or names none: the next edit trusts it uncopied.
- [`stroke_with`](../../../src/app/select.rs#L1962) — Stroke with.
- [`px_on`](../../../src/app/select.rs#L2027) — Pixel of layer 0's drawing on frame `f`, in that drawing's own pixels.
- [`new_drawing`](../../../src/app/select.rs#L2035) — Next frame, with a drawing of its own.
- [`animated`](../../../src/app/select.rs#L2044) — Three drawings on frames 0-2 with a red square at 100..140 in each, frame 3 holding the third; the playhead back on frame 0.
- [`two_stripes`](../../../src/app/select.rs#L2087) — Red on the left, blue on the right, inside a selection around both: flipped, they trade sides about the selection's centre.
- [`wand`](../../../src/app/select.rs#L2572) — A fresh state with the Lasso up as the magic select.
- [`ring`](../../../src/app/select.rs#L2579) — A one-pixel square outline from `(a, a)` to `(b, b)` on the active cell.
- [`click`](../../../src/app/select.rs#L2586) — Click.
- [`area`](../../../src/app/select.rs#L2591) — Area.
- [`px_at`](../../../src/app/select.rs#L2784) — Layer `li`'s pixel at `(x, y)` on frame `f`.
- [`paint_on`](../../../src/app/select.rs#L2792) — Paint on layer `li` instead of the active one.
- [`two_layers`](../../../src/app/select.rs#L2800) — A red square on layer 0 and a blue one on layer 1, both at 100..140; layer 1 active, layer 0 picked with it, a selection around the squares.

**Test-only types and constants:** [`RED`](../../../src/app/select.rs#L2024), [`BLUE`](../../../src/app/select.rs#L2083)
