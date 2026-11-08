# `src/io/composite.rs` — flatten all layers of a frame into one picture

> **Source:** [src/io/composite.rs](../../../src/io/composite.rs) · **Module:** `crate::io::composite` · **Size:** 987 lines · Line links as of commit `67a7cb9`

## What this file is for

On screen, Animator draws each layer as its own GPU texture and lets the graphics card stack
them. Exporting is different: a PNG, GIF, WebP or MP4 file wants **one flat image per frame**.
This file builds that flat image on the CPU. Given a project and a frame number, it walks the
layers from bottom to top and paints each visible one onto a fresh, empty canvas — a process
called *compositing* (see [glossary](../../glossary.md)).

While it stacks layers it also applies everything that changes where a layer lands: the
layer's own *transform* (move / scale / rotate, possibly keyed over time), the *camera*
(the rectangle that frames the shot, which can pan, zoom and roll), layer opacity, layer
groups, and *clipping masks* (a layer that only shows where the layer under it has paint).
Onion skins and reference layers are never part of an export, so they are skipped.

The core operation is **"source-over"** alpha blending — exactly what an HTML canvas does by
default (`ctx.globalCompositeOperation = "source-over"`): each new layer is laid *on top of*
what is already there, and its transparency decides how much of the layers below still shows
through. A worked numeric example is under [`composite_layer_in`](#composite_layer_in).

There is also an export-only quality trick: when a layer is drawn at **less than half** its
size (a camera pulled far back over a big canvas), the layer is first shrunk by averaging
blocks of pixels, so a one-pixel pencil line fades out evenly instead of flickering in and
out of existence as the camera drifts.

## How it connects

- **Used by:**
  - [gif_export.rs](gif_export.md), [png_seq.rs](png_seq.md), [mp4_export.rs](mp4_export.md),
    [webp_export.rs](webp_export.md) — call `flatten_through` once per exported frame.
  - [sprite_sheet.rs](sprite_sheet.md) — calls `flatten_frame` for each sheet cell.
  - [kra/mod.rs](kra/mod.md) — `write` uses `flatten_frame` / `flatten_doc` for the
    thumbnail and merged image inside a `.kra` file.
  - [app.rs](../app.md) — `merge_layer_down` and `fill_boundary` call `composite_layer`;
    `clipboard_frame` calls `flatten_through`; `visible_lines` calls `flatten_doc`; the free
    function `doc_to_cell` uses `sample_bilinear`.
  - [app/select.rs](../app/select.md) — `wand_down` (magic wand) reads `flatten_doc`.
  - [doc/clip.rs](../doc/clip.md) — `render_group` calls `composite_layer_in`.
  - [ui/shell.rs](../ui/shell.md) — the *Save PNG (current frame)…* menu item calls
    `flatten_frame`.
  - [project_file.rs](project_file.md) tests reuse the `pins::fnv` and `pins::scene` test helpers.
- **Uses:** [doc/canvas.rs](../doc/canvas.md) (`Canvas`, `DirtyRect`),
  [doc/project.rs](../doc/project.md) (`Project`, `output_size`, `resolve_camera`),
  [doc/camera.rs](../doc/camera.md) (`Camera::apply`), [doc/transform.rs](../doc/transform.md)
  (`Transform::doc_to_cell`), [doc/clip.rs](../doc/clip.md) (`clip::shown`),
  [doc/groups.rs](../doc/groups.md) (`shows`, `group_of`, `children`).

```text
exporter (png_seq / gif / webp / mp4 / sprite sheet / kra)
        │  one call per frame
        ▼
flatten_frame / flatten_through / flatten_doc
        │
        ▼
flatten ── for each layer, bottom → top ──┐
        │                                 │ group at <100% opacity:
        │                                 │ merge children into a temp canvas,
        ▼                                 │ then composite_over(temp, opacity)
composite_layer ─► composite_layer_in     ┘
        │  (identity transform + same size?)
        ├── yes ─► composite_over_in   (fast 1:1 row copy-blend)
        └── no  ─► per output pixel: doc_to_cell → sample_bilinear → blend
```

## Rust concepts in this file

**Closures that capture local variables.** `flatten` defines a little helper function inline
that can see `project`, `frame`, `cam`, `pw` and `ph` from the surrounding code.
```rust
let draw = |dst: &mut Canvas, li: usize| {
    let layer = &project.layers[li];
    // ...
};
draw(&mut out, li);
```
```js
const draw = (dst, li) => { const layer = project.layers[li]; /* ... */ };
draw(out, li);
```
More: [Closures](../../rust-primer.md#closures)

**`let … else` — unwrap or bail out.** `clip::shown` returns `Option<…>` (a value or
nothing). `let Some(x) = … else { return; }` takes the value out, or leaves early.
```rust
let Some(src) = crate::doc::clip::shown(project, li, frame) else {
    return;
};
```
```js
const src = clipShown(project, li, frame);
if (src == null) return;
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

**`Option::map_or` — apply if present, else use a default.**
```rust
let xform = cam.map_or(xform, |cam| cam.apply(&xform));
```
```js
const x = cam != null ? cam.apply(xform) : xform;
```
More: [Option and Result](../../rust-primer.md#option-and-result)

**Byte ↔ float casts with `as`.** Pixels are stored as `u8` (0–255). The maths happens in
`f32` (0.0–1.0), then is rounded, clamped and cast back. `as u8` would wrap or truncate badly
without the `clamp`, so the code always clamps first.
```rust
let sa = (s[3] as f32 / 255.0) * op;
d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
```
More: [Numbers and casts](../../rust-primer.md#numbers-and-casts)

**Mutable slices and `chunks_exact`.** A canvas is one long `Vec<u8>` of `R,G,B,A,R,G,B,A…`.
`chunks_exact(4)` walks it four bytes (one pixel) at a time; `&mut buf[i..i + 4]` borrows one
pixel so it can be written in place.
```rust
for (s, d) in span.chunks_exact(4).zip(out[at..at + span.len()].chunks_exact_mut(4)) {
    // s = source pixel (read), d = destination pixel (write)
}
```
```js
for (let i = 0; i < span.length; i += 4) { const s = span.subarray(i, i + 4); /* ... */ }
```
More: [Strings and slices](../../rust-primer.md#strings-and-slices),
[Iterators](../../rust-primer.md#iterators)

**`pub(crate)` and `#[cfg(test)]`.** `pub(crate)` means "public inside this program, but not
to outside crates". `#[cfg(test)]` modules (`tests`, `pins`, `camera_size_tests`,
`downscale_tests`) are only compiled by `cargo test`.
More: [Modules and visibility](../../rust-primer.md#modules-and-visibility),
[Attributes and cfg](../../rust-primer.md#attributes-and-cfg)

## Types

### `const AVERAGE_BELOW`
[src/io/composite.rs:409](../../../src/io/composite.rs#L409)

```rust
const AVERAGE_BELOW: f32 = 0.5;
```

The scale below which an exported layer is shrunk by averaging before it is sampled. A
bilinear sample reads the two source pixels either side of it, so down to half size every
source pixel still contributes to some output pixel. Smaller than that, whole rows and columns
fall *between* samples and a thin line "comes and goes" as the camera moves.

## Free functions

### `flatten_frame`
[src/io/composite.rs:21](../../../src/io/composite.rs#L21)

```rust
pub fn flatten_frame(project: &Project, frame: usize) -> Canvas
```

**What it does.** Flattens every visible, non-reference layer of `frame` into a new canvas,
viewed through the camera that is keyed for that frame. The result is the size of the
camera's frame (`Project::output_size` — the camera's own size, or the canvas size), in
straight (unmultiplied) RGBA8.

The camera is *folded into* each layer's transform instead of being applied as a second pass:
both are "similarity transforms" (move + uniform scale + rotate), and combining two of them
gives another one, so it costs nothing extra. An identity (untouched) camera folds to a no-op,
which keeps the fast 1:1 path alive for projects made before the camera existed.

**Parameters.**
- `project` — the whole document (layers, cells, camera keys).
- `frame` — 0-based frame index.

**Returns.** A new `Canvas` holding the flat picture.

**Used by:** [sprite_sheet.rs](sprite_sheet.md), [kra/mod.rs](kra/mod.md) (`write`),
[ui/shell.rs](../ui/shell.md) (*Save PNG (current frame)…*), `flatten_through` below, and many
tests across the crate.

### `flatten_through`
[src/io/composite.rs:29](../../../src/io/composite.rs#L29)

```rust
pub fn flatten_through(project: &Project, frame: usize, cam: Option<Camera>) -> Canvas
```

**What it does.** The frame "as an export writes it". When the export has worked out its own
camera for each output frame — for example a camera move stretched over several loops of a
short animation, so frame 3's drawings are seen through the camera of a later moment — that
camera is passed as `Some(cam)`. With `None` it behaves exactly like `flatten_frame`.

**Parameters.**
- `cam` — `Some(camera)` to force a camera, `None` to use the one keyed on `frame`.

**Returns.** The flat canvas.

**Used by:** [gif_export.rs](gif_export.md), [png_seq.rs](png_seq.md),
[mp4_export.rs](mp4_export.md), [webp_export.rs](webp_export.md),
[app.rs](../app.md) (`clipboard_frame`).

### `flatten_doc`
[src/io/composite.rs:38](../../../src/io/composite.rs#L38)

```rust
pub fn flatten_doc(project: &Project, frame: usize) -> Canvas
```

**What it does.** Like `flatten_frame`, but in *document space*: the canvas rectangle exactly
as the document has it, with no camera at all. Tools that need to "see what is drawn" — the
fill tool's boundaries, the magic wand — read this, because they work in canvas coordinates
whatever the camera frames.

**Used by:** [app.rs](../app.md) (`visible_lines`), [app/select.rs](../app/select.md)
(`wand_down`), [kra/mod.rs](kra/mod.md) (`write`, when the camera has its own size).

### `flatten`
[src/io/composite.rs:43](../../../src/io/composite.rs#L43)

```rust
fn flatten(project: &Project, frame: usize, cam: Option<Camera>) -> Canvas
```

**What it does.** The shared worker behind the three public `flatten_*` functions: draws
`frame`'s layers through `cam`, or in document space when `cam` is `None`.

**How it works.**
1. Picks the output size: the camera frame size (`output_size()`) when there is a camera,
   otherwise the canvas size. Creates an empty `Canvas` of that size.
2. Defines a closure `draw(dst, li)` that, for one layer index `li`:
   - skips it if it is hidden (`!project.shows(li)`, which also checks its group's eye), a
     reference layer, or a group header;
   - gets the layer's drawing for this frame via `clip::shown` (already cut to its base if
     the layer is a clipping mask; `None` means nothing to show);
   - resolves the layer's transform for the frame and folds the camera into it
     (`cam.apply(&xform)`);
   - if seen through a camera and drawn at less than `AVERAGE_BELOW` (half) size, shrinks the
     drawing first with `shrunk_for`; then calls `composite_layer`.
3. Walks layers bottom to top (`project.layers` is ordered bottom first):
   - a layer inside a group is skipped here — it is drawn when its group header is reached
     (the header sits directly above its children, so stacking order is unchanged);
   - a plain layer is drawn straight onto the output;
   - a visible group at full opacity draws its children straight onto the output. Source-over
     is *associative*, so "merge the group, then lay it down" equals "lay each child down";
   - a visible group at partial opacity first merges its children into a temporary canvas,
     then lays that down once with `composite_over` at the group's opacity. This stops the
     children from showing through one another (see the test
     `a_half_opaque_group_composites_first`);
   - a group at 0 opacity or hidden draws nothing.
4. Returns the output canvas.

**Example.** Why a half-opaque group must merge first. Blue at the bottom, opaque red on top
of it, both in a group at 50%:

```text
merge first:  red covers blue inside the group → (red, a=1) → laid at 50% → red at a≈0.5, no blue
each at 50%:  blue at 50%, then red at 50% over it → some blue still shows through
```

### `composite_layer`
[src/io/composite.rs:97](../../../src/io/composite.rs#L97)

```rust
pub fn composite_layer(dst: &mut Canvas, src: &Canvas, xform: &Transform, opacity: f32, pw: u32, ph: u32)
```

**What it does.** Paints one cell (`src`) onto `dst` with source-over blending, placed by the
layer transform `xform`, faded by `opacity`. It simply calls `composite_layer_in` with a
rectangle covering the whole `pw × ph` output.

**Parameters.**
- `dst` — the canvas being built; changed in place.
- `src` — the layer's drawing (its cell), which may be a different size than `dst`.
- `xform` — where the cell sits on `dst` (translate, scale, rotate about the centre).
- `opacity` — layer opacity, 0.0–1.0.
- `pw`, `ph` — width and height of `dst` (the "page").

**Used by:** `flatten` above; [app.rs](../app.md) (`merge_layer_down`, `fill_boundary`).

### `composite_layer_in`
[src/io/composite.rs:109](../../../src/io/composite.rs#L109)

```rust
pub fn composite_layer_in(
    dst: &mut Canvas,
    src: &Canvas,
    xform: &Transform,
    opacity: f32,
    pw: u32,
    ph: u32,
    rect: DirtyRect,
)
```

**What it does.** The real work of `composite_layer`, but only writing the pixels of `dst`
inside `rect`. Redrawing just part of a group's merged image needs this.

**Parameters.** As `composite_layer`, plus `rect` — the area of `dst` to write (clamped to
`pw × ph`).

**How it works.**
1. Clamps `rect` to the output size.
2. **Fast path:** if the transform is identity and `src` is exactly `pw × ph`, every source
   pixel lines up with a destination pixel, so it hands off to `composite_over_in` and returns.
3. Otherwise, clamps `opacity` to 0–1 and returns early if it is 0.
4. For every destination pixel `(x, y)` in `rect`, takes its centre `(x + 0.5, y + 0.5)` and
   *inverse-maps* it into the cell with `xform.doc_to_cell` — "which point of the drawing
   lands here?". (Mapping backwards from the output guarantees every output pixel gets
   exactly one sample, with no holes.)
5. Shifts by −0.5 to sample on the half-pixel grid; skips points outside the cell.
6. Reads the cell there with `sample_bilinear` (smooth blend of the four nearest pixels).
7. Blends that colour onto the destination pixel with straight-alpha source-over (below).

**The blend (source-over, straight alpha).** With all values as fractions 0.0–1.0, `sa` the
source alpha already multiplied by `opacity`, and `da` the destination alpha:

```text
out_a = sa + da × (1 − sa)
out_c = (src_c × sa + dst_c × da × (1 − sa)) / out_a      for c in R, G, B
```

In words: the new layer contributes its colour weighted by its own alpha; whatever was
already there contributes the rest, weighted by how much of it the new layer *lets through*
(`1 − sa`). Dividing by `out_a` turns the weighted sum back into a plain colour, because the
canvas stores colours *unmultiplied* (not scaled by alpha).

**Example.** Worked numbers. Destination pixel: opaque blue `(0, 0, 255, 255)`. Source pixel:
red at about half alpha `(255, 0, 0, 128)`, layer opacity 1.0.

```text
sa = 128/255 = 0.502        da = 255/255 = 1.0
out_a = 0.502 + 1.0 × (1 − 0.502)              = 1.0     → 255
R     = (1.0 × 0.502 + 0.0 × 1.0 × 0.498) / 1.0 = 0.502   → 128
G     = 0                                                 → 0
B     = (0.0 × 0.502 + 1.0 × 1.0 × 0.498) / 1.0 = 0.498   → 127
result: (128, 0, 127, 255)  — half red, half blue, fully opaque
```

Now with a see-through destination — blue at alpha 128 — and the same red at layer opacity 0.5:

```text
sa = 0.502 × 0.5 = 0.251     da = 0.502
out_a = 0.251 + 0.502 × 0.749            = 0.627  → 160
R     = (1.0 × 0.251) / 0.627            = 0.400  → 102
B     = (1.0 × 0.502 × 0.749) / 0.627    = 0.600  → 153
result: (102, 0, 153, 160)
```

The same rule in JavaScript, for one pixel:

```js
function sourceOver(dst, src, opacity) {        // dst, src: [r,g,b,a] 0..255
  const sa = (src[3] / 255) * opacity, da = dst[3] / 255;
  const outA = sa + da * (1 - sa);
  if (outA <= 0) return [0, 0, 0, 0];
  const out = [0, 1, 2].map(c =>
    Math.round(((src[c] / 255) * sa + (dst[c] / 255) * da * (1 - sa)) / outA * 255));
  return [...out, Math.round(outA * 255)];
}
sourceOver([0, 0, 255, 255], [255, 0, 0, 128], 1); // → [128, 0, 127, 255]
```

**Used by:** `composite_layer`; [doc/clip.rs](../doc/clip.md) (`render_group`).

### `shrunk_for`
[src/io/composite.rs:423](../../../src/io/composite.rs#L423)

```rust
fn shrunk_for(src: &Canvas, xform: &Transform) -> (Canvas, Transform)
```

**What it does.** For a layer drawn at under half size, returns a smaller copy of the drawing
plus a new transform that draws that smaller copy in exactly the place `xform` drew the
original. The copy is halved as many times as needed to leave it drawn at between half and
full size — where bilinear sampling misses nothing — and each of its pixels is the average of
the block it stands for. Export only: on screen and in document space layers are sampled
normally.

**Returns.** `(small_canvas, adjusted_transform)`.

**How it works.**
1. `halvings = floor(log2(1 / scale))`, clamped to 1..=7; the shrink factor is
   `k = 2^halvings` (2, 4, … 128).
2. Calls `shrunk(src, k)`.
3. `shrunk` pads the last row/column out to whole blocks, which moves the cell's centre (a
   cell is placed by its middle). The offset `(dx, dy)` is rotated by the layer angle,
   scaled, and added to `tx`/`ty` to put it back.
4. The new transform's `scale` is `scale × k`, so the small cell covers the same area.

**Example.** `scale = 0.25` → `1/0.25 = 4` → `log2 = 2` → `k = 4` → new scale 1.0.
`scale = 0.3` → `1/0.3 ≈ 3.33` → `log2 ≈ 1.74` → floor 1 → `k = 2` → new scale 0.6.

### `shrunk`
[src/io/composite.rs:446](../../../src/io/composite.rs#L446)

```rust
fn shrunk(src: &Canvas, k: u32) -> Canvas
```

**What it does.** Returns `src` at one `k`-th of its size: each output pixel is the average of
a `k × k` block, with the last column and row padded with clear pixels. The average is taken
*premultiplied* — colour weighted by alpha — so a clear pixel next to paint makes it fainter
without darkening it.

**How it works.**
1. Output size is `ceil(width / k) × ceil(height / k)` (at least 1×1).
2. For each row of blocks, sums `R·a`, `G·a`, `B·a` and `a` for every source pixel into a
   per-block accumulator (`row`), skipping clear pixels and reading only the stored span of
   a packed canvas (`row_span`).
3. For each block with any alpha: colour = `sum(c·a) / sum(a)`; alpha = `sum(a) / (k·k)`.
   Then resets the accumulator for the next block row.

**Example.** A 2×2 block with one opaque red pixel `(200, 0, 0, 255)` and three clear pixels:
colour = `200·255 / 255 = 200`, alpha = `255 / 4 ≈ 64` → `(200, 0, 0, 64)`. A plain average
would have given a dark `(50, 0, 0, 64)`; the premultiplied one keeps the red, just fainter.

### `sample_bilinear`
[src/io/composite.rs:492](../../../src/io/composite.rs#L492)

```rust
pub(crate) fn sample_bilinear(src: &Canvas, x: f32, y: f32) -> [u8; 4]
```

**What it does.** Reads the colour at a fractional position `(x, y)` in the source by blending
the four surrounding pixels (bilinear interpolation). Coordinates outside the image are
clamped to the nearest edge pixel. It reads pixels in place with `src.px` because a packed
canvas has no whole buffer to index.

**Returns.** The blended RGBA pixel.

**How it works.** `x0 = floor(x)`, `fx = x − x0` (likewise `y`). Blend the top pair by `fx`,
the bottom pair by `fx`, then blend those two results by `fy`.

**Example.** At `(0.25, 0.5)` between `p00 = 0`, `p10 = 100`, `p01 = 200`, `p11 = 40`:
`top = 0 + (100−0)·0.25 = 25`, `bot = 200 + (40−200)·0.25 = 160`,
`out = 25 + (160−25)·0.5 = 92.5 → 93`.

**Used by:** `composite_layer_in`; [app.rs](../app.md) (free function `doc_to_cell`).

### `composite_over`
[src/io/composite.rs:518](../../../src/io/composite.rs#L518)

```rust
pub(crate) fn composite_over(dst: &mut Canvas, src: &Canvas, opacity: f32)
```

**What it does.** Source-over blends `src` onto `dst` pixel for pixel (both must be the same
size), scaled by `opacity`. Calls `composite_over_in` over the whole canvas.

**Used by:** `flatten` (laying down a partly transparent group).

### `composite_over_in`
[src/io/composite.rs:529](../../../src/io/composite.rs#L529)

```rust
fn composite_over_in(dst: &mut Canvas, src: &Canvas, opacity: f32, rect: DirtyRect)
```

**What it does.** The same-size, no-transform blend, limited to `rect`. Uses the same
straight-alpha source-over formula as `composite_layer_in`.

**How it works.**
1. In debug builds, asserts the two canvases are the same size.
2. Row by row, asks the source for `row_span(y)` — the part of the row it actually stores.
   A packed canvas skips its blank margins, so empty areas cost nothing.
3. Intersects that span with `rect`, then zips source and destination pixels and blends each
   one (skipping fully transparent source pixels).

**Used by:** `composite_over`, and the fast path of `composite_layer_in`.

## Tests

Run with `cargo test io::composite`. The file has four test modules: `tests` (clipping,
groups, camera), `pins` (pinned output hashes), `camera_size_tests` (camera frame of its own
size) and `downscale_tests` (the averaging shrink).

| Test | What it checks |
|---|---|
| [`export_shows_a_clipped_layer_only_over_its_base`](../../../src/io/composite.rs#L174) | A clipped red layer shows only where the blue base has paint; unclipped it covers the frame. |
| [`a_half_opaque_group_composites_first`](../../../src/io/composite.rs#L237) | A group at 50% merges its layers first, so the blue under the red never shows through. |
| [`an_opaque_group_matches_direct_drawing`](../../../src/io/composite.rs#L254) | A group at full opacity exports byte-for-byte the same as its layers ungrouped. |
| [`a_hidden_group_exports_nothing`](../../../src/io/composite.rs#L265) | Hiding a group hides all its layers in the export. |
| [`a_layer_clipped_to_a_group_shows_over_all_of_it`](../../../src/io/composite.rs#L276) | A layer clipped to a group shows over everything the group covers, ignores child opacity, disappears when the group is hidden, and matches the Krita "baked" version. |
| [`identity_camera_exports_unchanged`](../../../src/io/composite.rs#L315) | An untouched camera exports exactly as before the camera existed. |
| [`panning_the_camera_shifts_the_frame`](../../../src/io/composite.rs#L325) | Panning the camera right by 2 moves the artwork 2 px left. |
| [`offscreen_layer_appears_only_once_the_camera_arrives`](../../../src/io/composite.rs#L339) | A layer parked a frame-width to the right is invisible until the camera pans over to it. |
| [`keyed_camera_pans_between_two_characters`](../../../src/io/composite.rs#L353) | A keyed camera move shows character A at frame 0 and B at frame 10, and ease-both barely moves at frame 1. |
| [`frames_through_a_keyed_camera_are_what_they_were`](../../../src/io/composite.rs#L675) | Pinned hashes of frames through a keyed camera have not changed. |
| [`frames_through_a_still_camera_are_what_they_were`](../../../src/io/composite.rs#L690) | Pinned hashes through a moved still camera and the resting camera have not changed. |
| [`frames_in_document_space_are_what_they_were`](../../../src/io/composite.rs#L703) | Pinned hashes of `flatten_doc` output have not changed. |
| [`a_smaller_frame_is_a_window_onto_the_canvas`](../../../src/io/composite.rs#L735) | A 12×8 camera on a 24×16 canvas gives exactly the middle crop, 1:1. |
| [`panning_moves_the_window_across_the_canvas`](../../../src/io/composite.rs#L747) | Panning a smaller camera frame crops a different area of the canvas. |
| [`a_portrait_frame_stands_on_end`](../../../src/io/composite.rs#L760) | A tall 8×12 camera frame gives the right crop and size. |
| [`a_frame_bigger_than_the_canvas_shows_nothing_past_its_edge`](../../../src/io/composite.rs#L770) | A camera frame larger than the canvas is clear outside the canvas area. |
| [`document_space_is_the_canvas_whatever_the_camera_s_size`](../../../src/io/composite.rs#L787) | `flatten_doc` ignores the camera's own size. |
| [`a_thin_line_is_never_skipped`](../../../src/io/composite.rs#L824) | At zoom 0.25 a 1-px line always becomes a quarter-alpha column, wherever it falls in the block. |
| [`thinning_a_line_keeps_its_colour`](../../../src/io/composite.rs#L840) | A shrunk red line stays pure red, just at lower alpha. |
| [`solid_paint_stays_solid`](../../../src/io/composite.rs#L847) | A canvas filled with one opaque colour still exports as that colour when shrunk. |
| [`down_to_a_half_frames_are_sampled_as_before`](../../../src/io/composite.rs#L871) | At zoom 1, 0.75, 0.5 nothing is averaged (output unchanged); at 0.25 it is. |
| [`a_shrunken_layer_is_averaged_through_a_resting_camera`](../../../src/io/composite.rs#L887) | A layer scaled to 0.25 is averaged even with the camera at rest. |
| [`an_odd_sized_cell_is_shrunk_in_place`](../../../src/io/composite.rs#L900) | A cell that is not a whole number of blocks is padded but still lands in the right place. |
| [`shrinking_averages_each_block`](../../../src/io/composite.rs#L923) | `shrunk` averages each 2×2 block premultiplied, including a half-off-edge block. |
| [`document_space_is_never_averaged`](../../../src/io/composite.rs#L940) | `flatten_doc` never uses the shrink, even for a small layer. |
| [`perf_export_downscale`](../../../src/io/composite.rs#L956) | `#[ignore]`d timing probe on an 8K canvas (run with `--ignored --nocapture`); not pass/fail. |

**Test helpers** (only compiled for tests):

### `mod tests`

Run with `cargo test io::composite::tests` · 9 tests

| Test | What it checks |
|---|---|
| [`export_shows_a_clipped_layer_only_over_its_base`](../../../src/io/composite.rs#L174) | Export shows a clipped layer only over its base. |
| [`a_half_opaque_group_composites_first`](../../../src/io/composite.rs#L237) | At half opacity the group's layers merge first: blue under red never shows through, only the merged red at half strength. |
| [`an_opaque_group_matches_direct_drawing`](../../../src/io/composite.rs#L254) | An opaque group matches direct drawing. |
| [`a_hidden_group_exports_nothing`](../../../src/io/composite.rs#L265) | A hidden group exports nothing. |
| [`a_layer_clipped_to_a_group_shows_over_all_of_it`](../../../src/io/composite.rs#L276) | Green clipped to the group shows only over what the group covers — the left half — and the mask ignores the layers' opacity. |
| [`identity_camera_exports_unchanged`](../../../src/io/composite.rs#L315) | An untouched project must export exactly as it did before the camera existed: the identity camera folds to a no-op. |
| [`panning_the_camera_shifts_the_frame`](../../../src/io/composite.rs#L325) | Panning the camera right moves the artwork left in the exported frame — which is how a character parked off to the side comes into shot. |
| [`offscreen_layer_appears_only_once_the_camera_arrives`](../../../src/io/composite.rs#L339) | A layer parked a full frame to the right is invisible until the camera keys pan over to it. |
| [`keyed_camera_pans_between_two_characters`](../../../src/io/composite.rs#L353) | The whole point, end to end: character A in frame, character B parked off to the right, keyed camera pans from one to the other. |

**Test helpers** (used only by the tests above):

- [`grouped`](../../../src/io/composite.rs#L206) — Three frame-sized layers on an 8×4 project, bottom first: blue on the left half, red on the left half too, green over everything. Each opaque. Layers 0 and 1 then go into a group.
- [`px`](../../../src/io/composite.rs#L230) — Px.
- [`dot_project`](../../../src/io/composite.rs#L298) — Paint one opaque red pixel at `(x, y)` of the project's only cell.
- [`red_at`](../../../src/io/composite.rs#L307) — Red at.

### `mod pins`

Run with `cargo test io::composite::pins` · 3 tests

| Test | What it checks |
|---|---|
| [`frames_through_a_keyed_camera_are_what_they_were`](../../../src/io/composite.rs#L675) | Frames through a keyed camera are what they were. |
| [`frames_through_a_still_camera_are_what_they_were`](../../../src/io/composite.rs#L690) | Frames through a still camera are what they were. |
| [`frames_in_document_space_are_what_they_were`](../../../src/io/composite.rs#L703) | Frames in document space are what they were. |

**Test helpers** (used only by the tests above):

- [`fnv`](../../../src/io/composite.rs#L583) — FNV-1a, 64 bit.
- [`speckle`](../../../src/io/composite.rs#L591) — Speckle a cell: clear, half and full alpha, in colours that depend on `seed`, so every layer and drawing is told apart.
- [`scene`](../../../src/io/composite.rs#L616) — Sixteen frames of everything an export goes through: a background with two drawings, a layer with a keyed pose, a half-opaque group of two, a clipped layer with an oversized cell on top, and a camera keyed through a pan, a push-in, a pull-out and a roll with every ease.
- [`hashes`](../../../src/io/composite.rs#L670) — Hashes.

### `mod camera_size_tests`

Run with `cargo test io::composite::camera_size_tests` · 5 tests

| Test | What it checks |
|---|---|
| [`a_smaller_frame_is_a_window_onto_the_canvas`](../../../src/io/composite.rs#L735) | A smaller frame is a window onto the canvas. |
| [`panning_moves_the_window_across_the_canvas`](../../../src/io/composite.rs#L747) | Panning moves the window across the canvas. |
| [`a_portrait_frame_stands_on_end`](../../../src/io/composite.rs#L760) | A portrait frame stands on end. |
| [`a_frame_bigger_than_the_canvas_shows_nothing_past_its_edge`](../../../src/io/composite.rs#L770) | A frame bigger than the canvas shows nothing past its edge. |
| [`document_space_is_the_canvas_whatever_the_camera_s_size`](../../../src/io/composite.rs#L787) | The fill tool's source is the canvas, whatever the camera frames. |

**Test helpers** (used only by the tests above):

- [`crop`](../../../src/io/composite.rs#L716) — `[x0, x0 + w) × [y0, y0 + h)` of `c`, as its own pixels.
- [`scene`](../../../src/io/composite.rs#L727) — The pinned scene on its 24×16 canvas, with the camera at rest.

### `mod downscale_tests`

Run with `cargo test io::composite::downscale_tests` · 9 tests

| Test | What it checks |
|---|---|
| [`a_thin_line_is_never_skipped`](../../../src/io/composite.rs#L824) | The point of it. A line one pixel wide used to show or vanish by where it fell among the samples; now it is a quarter of the exported pixel it lies in, wherever in it that is. |
| [`thinning_a_line_keeps_its_colour`](../../../src/io/composite.rs#L840) | Averaged premultiplied: a red line among clear pixels is fainter, not darker. |
| [`solid_paint_stays_solid`](../../../src/io/composite.rs#L847) | Solid paint stays solid. |
| [`down_to_a_half_frames_are_sampled_as_before`](../../../src/io/composite.rs#L871) | Down to a half, nothing is averaged: those frames are the bytes they were. Only past it does the export change. |
| [`a_shrunken_layer_is_averaged_through_a_resting_camera`](../../../src/io/composite.rs#L887) | A layer shrunk on the canvas is averaged in an export too — it is the size it is drawn at that counts, camera and layer together. |
| [`an_odd_sized_cell_is_shrunk_in_place`](../../../src/io/composite.rs#L900) | A cell that isn't a whole number of blocks is padded to shrink it — and still drawn where it was: its paint lands in the exported pixels it covers, not half a block over. |
| [`shrinking_averages_each_block`](../../../src/io/composite.rs#L923) | Shrinking averages each block. |
| [`document_space_is_never_averaged`](../../../src/io/composite.rs#L940) | Document space — what the fill tool reads its boundaries from — is not an export, and is sampled as it always was. |
| [`perf_export_downscale`](../../../src/io/composite.rs#L956) | What averaging costs, and that frames it doesn't touch cost what they did. Not a pass/fail test — run it for the numbers: `cargo test --release perf_export_downscale -- --ignored --nocapture` |

**Test helpers** (used only by the tests above):

- [`pulled_back`](../../../src/io/composite.rs#L804) — A 64×64 canvas seen whole through a 16×16 frame: a camera pulled back to a quarter, four canvas pixels to each exported one.
- [`column`](../../../src/io/composite.rs#L811) — Column.
- [`sampled`](../../../src/io/composite.rs#L859) — What `flatten_frame` gave before any of this: every layer through `composite_layer`.
