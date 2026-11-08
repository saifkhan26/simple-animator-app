# `src/doc/clip.rs` — clipping masks and merged group images

> **Source:** [src/doc/clip.rs](../../../src/doc/clip.rs) · **Module:** `crate::doc::clip` · **Size:** 646 lines · Line links as of commit `1fa5845`

## What this file is for

A **clipped layer** shows only where the layer under it (its *base*) has paint — the standard way to
colour or shade inside line art without going over the edge (Photoshop "clipping mask", Clip Studio
"clip to layer below"). Several clipped layers in a row share one base.

This file computes the clipped result on the CPU: for each pixel of the clipped drawing, its alpha is
multiplied by the base drawing's alpha at the same spot. When both layers sit identically, that's a
pixel-for-pixel multiply; when the layers are moved/scaled differently, the base's alpha is sampled
bilinearly at the right place (`BaseMap`).

```text
clipped colour layer      base line layer        what shows
 ██████████████             ░░███░░               ░░███░░
 ██████████████     ×       ░█████░      =        ░█████░    (colour only where the base has ink)
```

It also renders **groups as one image** (for groups below full opacity, and for layers clipped to a
whole group), computes a hash describing what a group image is made from (to know when to rebuild it),
and **bakes** clipping into pixels for export to Krita.

## How it connects

- **Used by:** [app.rs](../app.md) (clip textures `sync_clip_textures`), [app/groups.rs](../app/groups.md)
  (group images), [io/composite.rs](../io/composite.md) (exports draw clipped layers via `shown`),
  [io/kra/mod.rs](../io/kra/mod.md) / [krita_link.rs](../krita_link.md) (`bake_clipping`).
- **Uses:** [doc/project.rs](project.md) (`clip_base`, layers), [doc/canvas.rs](canvas.md),
  [doc/transform.rs](transform.md) (`cell_to_doc`, `doc_to_cell`), [doc/groups.rs](groups.md) (group
  children).

## Rust concepts in this file

**`Cow` — borrow when possible, own when needed.** `shown` returns `Option<Cow<'_, Canvas>>`: an
unclipped layer's own canvas comes back *borrowed* (no copy), a clipped one comes back as a newly built
*owned* canvas. Callers use both the same way. More: [Smart pointers](../../rust-primer.md#smart-pointers)

**`&dyn Fn(usize) -> Transform`.** Several functions take "a function that gives each layer's
transform", as a trait object reference, so callers can pass different placement rules.
More: [Generics](../../rust-primer.md#generics)

**Integer maths for 0–255 fractions.** `mul(a, k) = (a·k + 127) / 255` multiplies two alpha values as if
they were 0–1 numbers, with rounding — e.g. `mul(128, 128) = 64`.

## Types

### `struct Placement`
[src/doc/clip.rs:23](../../../src/doc/clip.rs#L23)

How a clipped drawing and its base sit in a `pw`×`ph` document.

| Field | Type | Meaning |
|---|---|---|
| `xf` | `Transform` | The clipped layer's transform on the frame. |
| `base_xf` | `Transform` | The base layer's transform on the frame. |
| `pw` | `u32` | Document width. |
| `ph` | `u32` | Document height. |

### `struct BaseMap`
[src/doc/clip.rs:61](../../../src/doc/clip.rs#L61)

An affine map from a clipped-cell pixel to the base cell's sample position: `u = ux·x + uy·y + u0`,
`v = vx·x + vy·y + v0`. Both placements are similarity transforms, so the chain is affine and three points
determine it.

| Field | Type | Meaning |
|---|---|---|
| `ux` | `f32` | How far `u` moves per clip pixel in x. |
| `uy` | `f32` | How far `u` moves per clip pixel in y. |
| `u0` | `f32` | `u` at clip pixel (0, 0). |
| `vx` | `f32` | How far `v` moves per clip pixel in x. |
| `vy` | `f32` | How far `v` moves per clip pixel in y. |
| `v0` | `f32` | `v` at clip pixel (0, 0). |

## `impl Placement`

### `Placement::of`
[src/doc/clip.rs:32](../../../src/doc/clip.rs#L32)

```rust
pub fn of(p: &Project, li: usize, base: usize, frame: usize) -> Self
```

Where layer `li` and its base sit on `frame`.

## `impl Placement`

### `Placement::aligned`
[src/doc/clip.rs:46](../../../src/doc/clip.rs#L46)

```rust
pub fn aligned(&self, clip: &Canvas, base: &Canvas) -> bool
```

Whether the base pixel lies exactly under the clipped pixel with the same index (same transform and same
cell size) — the common case, where a pixel-by-pixel multiply works.

## Free functions

### `mul`
[src/doc/clip.rs:53](../../../src/doc/clip.rs#L53)

```rust
fn mul(a: u8, k: u8) -> u8
```

Multiplies two 0–255 fractions with rounding.

## `impl BaseMap`

### `BaseMap::new`
[src/doc/clip.rs:71](../../../src/doc/clip.rs#L71)

```rust
fn new(clip: &Canvas, base: &Canvas, at: &Placement) -> Self
```

Builds the affine map by sending three clip-cell points ((0,0), (1,0), (0,1)) through clip cell → document
→ base cell, and reading off the coefficients.

## Free functions

### `alpha_at`
[src/doc/clip.rs:94](../../../src/doc/clip.rs#L94)

```rust
fn alpha_at(b: &Canvas, u: f32, v: f32) -> u8
```

The base canvas's alpha at fractional position `(u, v)`, bilinearly interpolated from the four nearest
pixels; 0 outside the canvas.

### `inside`
[src/doc/clip.rs:108](../../../src/doc/clip.rs#L108)

```rust
fn inside(clip: &Canvas, rect: DirtyRect) -> (u32, u32, u32, u32)
```

Clamps a rectangle to the clipped canvas's bounds.

### `mask_row`
[src/doc/clip.rs:115](../../../src/doc/clip.rs#L115)

```rust
fn mask_row(
    out: &mut [u8],
    clip: &Canvas,
    base: Option<&Canvas>,
    map: Option<&BaseMap>,
    y: u32,
    x0: u32,
    x1: u32,
)
```

Computes one row of clipped output: each pixel of `clip` with its alpha multiplied by the base's alpha
(directly when aligned, via `BaseMap` sampling otherwise, or zero when there is no base drawing).

### `mask_into`
[src/doc/clip.rs:154](../../../src/doc/clip.rs#L154)

```rust
pub fn mask_into(
    out: &mut [u8],
    clip: &Canvas,
    base: Option<&Canvas>,
    at: &Placement,
    rect: DirtyRect,
)
```

Writes the clipped pixels for rectangle `rect` into `out` (a buffer the size of the clip cell), row by
row with `mask_row`; pixels outside `rect` are untouched.

### `mask_rect`
[src/doc/clip.rs:172](../../../src/doc/clip.rs#L172)

```rust
pub fn mask_rect(clip: &Canvas, base: Option<&Canvas>, at: &Placement, rect: DirtyRect) -> Vec<u8>
```

`mask_into` for just `rect`, packed into its own buffer — what a partial GPU texture upload needs.

### `masked`
[src/doc/clip.rs:186](../../../src/doc/clip.rs#L186)

```rust
pub fn masked(clip: &Canvas, base: Option<&Canvas>, at: &Placement) -> Canvas
```

The whole clipped drawing as a new canvas.

### `shown`
[src/doc/clip.rs:203](../../../src/doc/clip.rs#L203)

```rust
pub fn shown(p: &Project, li: usize, frame: usize) -> Option<std::borrow::Cow<'_, Canvas>>
```

The drawing layer `li` displays on `frame`: its own cell (borrowed) if unclipped, the clipped version if
clipped, or `None` when a clipped layer's base is hidden, a reference layer or empty on that frame.

### `group_mask`
[src/doc/clip.rs:231](../../../src/doc/clip.rs#L231)

```rust
pub fn group_mask(p: &Project, g: usize, frame: usize) -> Option<Canvas>
```

What group `g` covers on `frame`, as a mask for layers clipped to the whole group: the union of what its
visible, non-reference layers show (opacity ignored). The mask lives in "group space" — a canvas centred
on the document.

### `group_kids`
[src/doc/clip.rs:246](../../../src/doc/clip.rs#L246)

```rust
fn group_kids(p: &Project, g: usize, frame: usize) -> impl Iterator<Item = usize> + '_
```

The layers of group `g` that draw on `frame` (visible, not reference, with a drawing there). Returns an
iterator (`impl Iterator<Item = usize> + '_`), produced lazily.

### `group_space`
[src/doc/clip.rs:254](../../../src/doc/clip.rs#L254)

```rust
pub fn group_space(p: &Project, g: usize, frame: usize, xf: &dyn Fn(usize) -> Transform) -> (u32, u32)
```

The size of group `g`'s group-space canvas: the document frame, grown by whole pixels on each side as far
as any of its layers reaches, keeping the document's centre.

### `render_group`
[src/doc/clip.rs:278](../../../src/doc/clip.rs#L278)

```rust
pub fn render_group(
    p: &Project,
    g: usize,
    frame: usize,
    opacity: bool,
    xf: &dyn Fn(usize) -> Transform,
    out: &mut Canvas,
    rect: DirtyRect,
) -> bool
```

Draws group `g`'s layers merged into `rect` of `out` (clearing it first) — at each layer's opacity when
`opacity` is true (how the group shows), or at full strength (as a clip mask). Returns whether any layer
drew.

### `group_sig`
[src/doc/clip.rs:313](../../../src/doc/clip.rs#L313)

```rust
pub fn group_sig(p: &Project, g: usize, frame: usize, opacity: bool, xf: &dyn Fn(usize) -> Transform) -> u64
```

A hash of everything a group image is built from (size, each layer's drawing, placement, visibility,
opacity, clip base). Equal hashes mean the cached image is still right (pixel edits are tracked
separately per cell).

### `group_cells`
[src/doc/clip.rs:337](../../../src/doc/clip.rs#L337)

```rust
pub fn group_cells(p: &Project, g: usize, frame: usize, xf: &dyn Fn(usize) -> Transform) -> Vec<(CellId, bool)>
```

The cells a group image on `frame` is drawn from, each flagged with whether it maps pixel-for-pixel onto
the group canvas — so an edit to that cell only redraws the same rectangle.

## `impl Placement`

### `Placement::of_group`
[src/doc/clip.rs:359](../../../src/doc/clip.rs#L359)

```rust
pub fn of_group(p: &Project, li: usize, frame: usize) -> Self
```

Where layer `li` sits against its group's mask (which is in group space, i.e. identity transform).

## Free functions

### `bake_clipping`
[src/doc/clip.rs:376](../../../src/doc/clip.rs#L376)

```rust
pub fn bake_clipping(p: &mut Project) -> HashMap<CellId, CellId>
```

For exporting to Krita (which has no equivalent clipping here): bakes every clipped layer's mask into its
pixels and unclips it. A clipped drawing held while its base changes becomes one baked drawing per
pairing, keyed where the pairing changes. Returns a map from new baked cells to their source cells.

## Tests

### `mod tests`

Run with `cargo test doc::clip::tests` · 9 tests

| Test | What it checks |
|---|---|
| [`clipped_layers_share_the_first_unclipped_layer_below`](../../../src/doc/clip.rs#L467) | Clipped layers share the first unclipped layer below. |
| [`the_mask_is_the_bases_alpha`](../../../src/doc/clip.rs#L484) | The mask is the bases alpha. |
| [`packed_drawings_clip_the_same`](../../../src/doc/clip.rs#L497) | Clipping reads packed drawings in place and gets the same pixels. |
| [`a_moved_base_moves_the_mask`](../../../src/doc/clip.rs#L512) | A moved base moves the mask. |
| [`only_the_rect_is_touched`](../../../src/doc/clip.rs#L523) | Only the rect is touched. |
| [`a_packed_rect_matches_the_same_rect_in_place`](../../../src/doc/clip.rs#L539) | A packed rect matches the same rect in place. |
| [`a_hidden_base_hides_what_is_clipped_to_it`](../../../src/doc/clip.rs#L558) | A hidden base hides what is clipped to it. |
| [`baking_splits_a_held_drawing_where_its_base_changes`](../../../src/doc/clip.rs#L573) | Baking splits a held drawing where its base changes. |
| [`clip_timing`](../../../src/doc/clip.rs#L608) | Cost at a 1080p frame, for the record: `cargo test --release clip_timing -- --ignored --nocapture`. |

**Test helpers** (used only by the tests above):

- [`band`](../../../src/doc/clip.rs#L442) — An 8×8 cell painted `rgba` inside `x0..x1`, transparent elsewhere.
- [`alpha`](../../../src/doc/clip.rs#L453) — Alpha.
- [`still`](../../../src/doc/clip.rs#L457) — Still.
