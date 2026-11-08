# `src/doc/transform.rs` — Where a layer's drawings sit, and ease curves

> **Source:** [src/doc/transform.rs](../../../src/doc/transform.rs) · **Module:** `crate::doc::transform` · **Size:** 168 lines · Line links as of commit `36405ad`

## What this file is for

Every layer can be moved, scaled and rotated as a whole — think of sliding a sheet of
acetate around on top of the background, without redrawing anything. This file defines that
**layer transform** (`Transform`: an offset, one uniform scale and a rotation), the
**keyframe** that stores a transform on a given frame (`TransformKey`), and the **ease curve**
(`Ease`) that shapes how motion speeds up and slows down between two keyframes. `Ease` is also
used by camera keyframes ([camera.rs](camera.md)), which is why it lives here.

It is part of the **document model** ([doc/mod.md](mod.md)). A transform maps a cell's own
pixel grid ("cell space", `(u, v)`) into the canvas's pixel grid ("document space",
`(x, y)`). The identity transform puts the cell's centre on the canvas's centre at its natural
size, so a cell the same size as the canvas lines up pixel for pixel — exactly how the app
behaved before transforms existed. In CSS terms it is like
`transform: translate(tx, ty) rotate(rot) scale(scale)` with `transform-origin: center`.

Which transform shows on a given frame (interpolating between keys) is worked out by
[`Layer::resolve_transform`](layer.md#layerresolve_transform) in `layer.rs`.

## How it connects

- **Used by:**
  - [layer.rs](layer.md) — each `Layer` holds a `transform` and a `Vec<TransformKey>`, and
    `resolve_transform` calls [`Ease::apply`](#easeapply) and [`Transform::lerp`](#transformlerp).
  - [camera.rs](camera.md) — re-exports `Ease` (`pub use crate::doc::transform::Ease`) for
    camera keys and calls `Ease::apply` when resolving the camera.
  - [project.rs](project.md) — uses `Ease::default()` (through the camera re-export) for new
    camera keys.
  - [app.rs](../app.md), [app/select.rs](../app/select.md), [app/liquify.rs](../app/liquify.md),
    [doc/clip.rs](clip.md), [tools/perspective.rs](../tools/perspective.md),
    [tools/select_mask.rs](../tools/select_mask.md), [ui/shell.rs](../ui/shell.md),
    [io/composite.rs](../io/composite.md) — convert points between cell space and document
    space with `cell_to_doc` / `doc_to_cell`.
  - [ui/shell.rs](../ui/shell.md) — builds the ease drop-down menus from `Ease::ALL` and
    `Ease::label`.
  - [io/project_file.rs](../io/project_file.md) — saves and loads these types (via `serde`).
- **Uses:** nothing else in the crate — only `f32` maths and `serde`.

## Rust concepts in this file

**Enums with `#[derive]`.** `Ease` is a plain list of choices, like a TypeScript string union.
The `#[derive(...)]` line asks the compiler to generate standard behaviour: copy it freely
(`Clone, Copy`), print it for debugging (`Debug`), compare with `==` (`PartialEq, Eq`), give it a
default (`Default`, with `#[default]` marking which variant), and save/load it (`serde`).
```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Ease { #[default] Linear, In, Out, Both }
```
```ts
type Ease = "Linear" | "In" | "Out" | "Both";
const defaultEase: Ease = "Linear";
```
More: [Enums and match](../../rust-primer.md#enums-and-match),
[Traits and derive](../../rust-primer.md#traits-and-derive)

**`match` must cover every case.** `Ease::apply` and `Ease::label` use `match self { … }`. Like a
`switch` with no fall-through — and if a new variant were added, the compiler would refuse to
build until every `match` handles it.
```rust
match self {
    Ease::Linear => t,
    Ease::In => t * t,
    // …
}
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

**Implementing a trait by hand: `impl Default for Transform`.** `Transform` cannot use
`#[derive(Default)]` because that would make `scale` zero (every number defaults to `0`). So the
file writes the `default()` function itself, with `scale: 1.0`.
```rust
impl Default for Transform {
    fn default() -> Self { Self { tx: 0.0, ty: 0.0, scale: 1.0, rot: 0.0 } }
}
```
Callers then write `Transform { tx: 100.0, ..Default::default() }` — "all defaults except
`tx`", like `{ ...defaults, tx: 100 }` in JS.
More: [Structs](../../rust-primer.md#structs), [Traits and derive](../../rust-primer.md#traits-and-derive)

**Associated constants and functions.** `Ease::ALL` is a constant attached to the type (like a
`static` class field), and `Transform::lerp(a, b, t)` is a function attached to the type that
takes no `self` (like a `static` method). Methods that do take `&self` (`is_identity`,
`cell_to_doc`) are called on a value: `xf.cell_to_doc(...)`.
More: [Impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

**Closures.** Inside `lerp`, `let l = |x: f32, y: f32| x + (y - x) * t;` is a small inline
function, the same as `const l = (x, y) => x + (y - x) * t;`. It *captures* `t` from the
surrounding function.
More: [Closures](../../rust-primer.md#closures)

**`#[serde(default)]` on a field.** `.anim` files are written with *postcard*, a format that
stores fields by position, not by name. Older files have no `ease` value at the end of each
key; `#[serde(default)]` means "if it is missing, use `Ease::default()`". That only works for
fields at the end, which is why the source insists `ease` stays the last field.
More: [Serde and file formats](../../rust-primer.md#serde-and-file-formats)

## impl Ease

## impl Default for Transform

## impl Transform

## Types

### `enum Ease`
[src/doc/transform.rs:13](../../../src/doc/transform.rs#L13)

The timing curve for the stretch of animation that **starts** at a key and runs to the next
one. It remaps "how far through the segment are we" (`t`, from 0 to 1) into "how far along the
motion should be". Shared by layer transform keys and camera keys.

| Variant | Meaning |
|---|---|
| `Linear` | `t`: constant speed, no easing (the default). |
| `In` | `t²` |
| `Out` | `t·(2 − t)` |
| `Both` | `t²·(3 − 2t)` |

### `const Ease::ALL`
[src/doc/transform.rs:25](../../../src/doc/transform.rs#L25)

```rust
pub const ALL: [Ease; 4] = [Ease::Linear, Ease::In, Ease::Out, Ease::Both];
```

`[Ease; 4]` — all four variants in menu order (`Linear, In, Out, Both`). The UI loops over it
to build ease drop-downs; a test loops over it to check every curve.

### `struct Transform`
[src/doc/transform.rs:49](../../../src/doc/transform.rs#L49)

One pose of a layer: where its cells sit on the canvas. `Copy`, so it is passed around by value
like a number.

Default ([`Transform::default`](#transformdefault)): `tx = ty = 0`, `scale = 1`, `rot = 0` — the identity.

| Field | Type | Meaning |
|---|---|---|
| `tx` | `f32` | horizontal offset of the cell's centre from the canvas's centre, in document pixels |
| `ty` | `f32` | vertical offset of the same (y grows downwards) |
| `scale` | `f32` | uniform scale; `1.0` = natural size |
| `rot` | `f32` | rotation in radians (positive turns clockwise on screen, because y points down) |

### `struct TransformKey`
[src/doc/transform.rs:117](../../../src/doc/transform.rs#L117)

A keyframe for a layer's transform: "on this frame, the layer is posed like this".

| Field | Type | Meaning |
|---|---|---|
| `frame` | `usize` | the frame the key sits on (0-based) |
| `transform` | `Transform` | the pose on that frame |
| `ease` | `Ease` | how the motion from this key to the *next* one is timed; ignored on the last key. Must stay the last field (postcard is positional), and defaults to `Linear` when an older file lacks it |

## `impl Ease`

### `Ease::label`
[src/doc/transform.rs:27](../../../src/doc/transform.rs#L27)

```rust
pub fn label(self) -> &'static str
```

**What it does.** The human-readable name for the menu: `"Linear"`, `"Ease in"`, `"Ease out"`
or `"Ease both"`. `&'static str` means a string baked into the program that lives forever —
like a string literal in JS.

**Used by:** [ui/shell.rs](../ui/shell.md) (the ease drop-downs for layer transform and camera keys).

### `Ease::apply`
[src/doc/transform.rs:37](../../../src/doc/transform.rs#L37)

```rust
pub fn apply(self, t: f32) -> f32
```

**What it does.** Remaps normalised segment time. `t` is first clamped to `0..=1`; then the
formula from the [`Ease`](#enum-ease) table is applied. Every curve gives `0` at `t = 0` and `1` at
`t = 1`, so keys are always hit exactly; only the in-between frames move.

**Parameters.**
- `t` — how far through the segment between two keys, `0.0` (on the first key) to `1.0` (on
  the next key).

**Returns.** The eased fraction, also in `0..=1`.

**Example.** Halfway between two keys (`t = 0.5`): `Linear` gives `0.5`, `In` gives `0.25`
(still lagging behind), `Out` gives `0.75` (already most of the way), `Both` gives `0.5` but
with a slow start and finish. The same idea in JS:
```js
const ease = { Linear: t => t, In: t => t*t, Out: t => t*(2-t), Both: t => t*t*(3-2*t) };
const clamp01 = t => Math.min(1, Math.max(0, t));
ease.In(clamp01(0.5)); // 0.25
```

**Used by:** [`Layer::resolve_transform`](layer.md#layerresolve_transform) and the camera's
`Camera::resolve` / `Camera::resolve_at` in [camera.rs](camera.md).

## `impl Default for Transform`

### `Transform::default`
[src/doc/transform.rs:60](../../../src/doc/transform.rs#L60)

```rust
fn default() -> Self
```

**What it does.** The identity transform: no offset, scale `1.0`, no rotation. Written by hand
because a derived default would set `scale` to `0.0`. Used whenever a layer is created
(`Layer::new`) and by `..Default::default()` in many places.

## `impl Transform`

### `Transform::is_identity`
[src/doc/transform.rs:71](../../../src/doc/transform.rs#L71)

```rust
pub fn is_identity(&self) -> bool
```

**What it does.** `true` when the transform does nothing: zero offset, zero rotation, and a
scale within `0.000001` of `1.0` (floating-point scale is compared with a tolerance; the other
fields exactly). Callers use it to take fast paths — e.g. copy pixels straight across instead of
resampling.

**Used by:** [app.rs](../app.md), [doc/clip.rs](clip.md), [io/composite.rs](../io/composite.md),
[ui/shell.rs](../ui/shell.md).

### `Transform::lerp`
[src/doc/transform.rs:79](../../../src/doc/transform.rs#L79)

```rust
pub fn lerp(a: Transform, b: Transform, t: f32) -> Transform
```

**What it does.** Blends two poses: `t = 0` gives `a`, `t = 1` gives `b`, values between give
the in-between pose. ("lerp" = **l**inear int**erp**olation.)

**Parameters.**
- `a`, `b` — the two poses (usually two keyframes).
- `t` — blend amount, normally `0..=1` (already eased by the caller).

**Returns.** The blended `Transform`.

**How it works.**
1. `tx`, `ty` and `rot` move in a straight line: `x + (y − x)·t`.
2. `scale` moves **geometrically**: `sa · (sb / sa)^t`. Both scales are first raised to at
   least `0.000001` so the division never hits zero.

**Example.** Why geometric? Growing from scale 1 to 4 over 10 frames: linearly, the midpoint
would be 2.5 and the zoom would seem to race at first and crawl at the end. Geometrically, the
midpoint is 2 (1 → 2 → 4: each half doubles), which reads as a steady zoom. The same rule is
used by `Camera::lerp` for zoom.
```js
const lerpScale = (sa, sb, t) => sa * Math.pow(sb / sa, t);
lerpScale(1, 4, 0.5); // 2
```

**Used by:** [`Layer::resolve_transform`](layer.md#layerresolve_transform).

### `Transform::cell_to_doc`
[src/doc/transform.rs:92](../../../src/doc/transform.rs#L92)

```rust
pub fn cell_to_doc(&self, u: f32, v: f32, cw: f32, ch: f32, pw: f32, ph: f32) -> (f32, f32)
```

**What it does.** Converts a pixel position inside a cell into a position on the canvas.

**Parameters.**
- `u`, `v` — the point in cell pixels.
- `cw`, `ch` — the cell's width and height.
- `pw`, `ph` — the canvas (project) width and height.

**Returns.** `(x, y)` in document pixels — a *tuple*, Rust's way of returning two values
(like returning `[x, y]` in JS).

**How it works.**
1. Move the origin to the cell's centre: `(u − cw/2, v − ch/2)`, then multiply by `scale`.
2. Rotate by `rot` with the usual 2D rotation (`x·cos − y·sin`, `x·sin + y·cos`).
3. Add the canvas centre `(pw/2, ph/2)` and the offset `(tx, ty)`.

**Example.** Identity transform, cell 100×100, canvas 100×100: `(10, 20)` stays `(10, 20)`.
With `tx = 5` it becomes `(15, 20)`. With `scale = 2` it becomes
`(50 + (10 − 50)·2, 50 + (20 − 50)·2) = (−30, −10)` — the point moves away from the centre.

**Used by:** [app.rs](../app.md), [app/select.rs](../app/select.md),
[app/liquify.rs](../app/liquify.md), [doc/clip.rs](clip.md),
[tools/perspective.rs](../tools/perspective.md), [tools/select_mask.rs](../tools/select_mask.md),
[ui/shell.rs](../ui/shell.md).

### `Transform::doc_to_cell`
[src/doc/transform.rs:102](../../../src/doc/transform.rs#L102)

```rust
pub fn doc_to_cell(&self, x: f32, y: f32, cw: f32, ch: f32, pw: f32, ph: f32) -> (f32, f32)
```

**What it does.** The exact inverse of [`cell_to_doc`](#transformcell_to_doc): converts a canvas
position (say, where the pen touched) into the cell pixel it lands on. This is how a stroke on
a moved, scaled or rotated layer ends up in the right place in the drawing.

**Parameters.** Same as `cell_to_doc`, but `x`, `y` are document pixels.

**Returns.** `(u, v)` in cell pixels (may be outside the cell).

**How it works.**
1. Subtract the canvas centre and the offset.
2. Rotate by `−rot` (the sine terms flip sign).
3. Divide by `scale` — but if `scale` is practically zero (under `1e-9`), divide by `1e-9`
   instead, so the maths never divides by zero.
4. Add the cell centre `(cw/2, ch/2)`.

**Used by:** [app.rs](../app.md) (including its own `doc_to_cell` helper), [app/select.rs](../app/select.md),
[app/liquify.rs](../app/liquify.md), [doc/clip.rs](clip.md), [io/composite.rs](../io/composite.md),
[tools/perspective.rs](../tools/perspective.md), [tools/select_mask.rs](../tools/select_mask.md),
[ui/shell.rs](../ui/shell.md).

## Tests

Run with `cargo test doc::transform`.

| Test | What it checks |
|---|---|
| [`lerp_scales_geometrically`](../../../src/doc/transform.rs#L132) | Halfway between scale 1 and 4 is 2 (geometric), and the endpoints are exact. |
| [`lerp_translates_linearly`](../../../src/doc/transform.rs#L148) | Translation stays linear: a quarter of the way from `tx = 0` to `100` is `25`, so a scrolling background moves at an even rate. |
| [`ease_endpoints_are_fixed`](../../../src/doc/transform.rs#L158) | Every `Ease` gives 0 at `t = 0` and 1 at `t = 1`; ease-in is below 0.5 at the midpoint and ease-out above it. |

### `mod tests`

Run with `cargo test doc::transform::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`lerp_scales_geometrically`](../../../src/doc/transform.rs#L132) | Scale interpolates geometrically, so the midpoint of a 1 → 4 blow-up is 2 (a constant rate) rather than the linear 2.5. |
| [`lerp_translates_linearly`](../../../src/doc/transform.rs#L148) | Translation stays linear — a road scrolling on X must move at an even rate between keys. |
| [`ease_endpoints_are_fixed`](../../../src/doc/transform.rs#L158) | Ease endpoints are fixed. |
