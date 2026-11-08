# `src/app/liquify.rs` — a Liquify session on AppState

> **Source:** [src/app/liquify.rs](../../../src/app/liquify.rs) · **Module:** `crate::app::liquify` · **Size:** 762 lines · Line links as of commit `67a7cb9`

## What this file is for

**Liquify** lets you push a drawing's pixels around with the pen, like Photoshop's Liquify filter — to
nudge a proportion or bend a line without redrawing. This file runs a *session* of it on `AppState`; the
maths (the warp field and resampling) lives in [tools/liquify.rs](../tools/liquify.md).

How a session works:
1. On the first pen-down the drawing as it is becomes the **original** (the stroke snapshot), kept for the
   whole session, and a **warp field** is created: a grid where each point says how far that spot has been
   pushed.
2. Each pen movement applies **dabs** that change the field under the brush.
3. Once per frame, the area the dabs reached is **re-sampled from the original** through the field. Always
   sampling from the original (never from an already-warped image) is why going back and forth doesn't blur
   the lines.
4. Ctrl+Z takes back the last stroke (from per-tile backups of the field); Enter, or changing tool, frame
   or layer, **commits** the warp as one undo step; Esc cancels it. With "All frames" on, the same warp is
   applied to every drawing of the layer.

## How it connects

- **Used by:** [app.rs](../app.md) (pointer handlers route Liquify presses here, `update` calls
  `sync_liquify`, `undo` calls `liquify_step_back`, `land_float` calls `commit_liquify`),
  [ui/shell.rs](../ui/shell.md) (Liquify options, mesh display, calls `flush_liquify` once a frame).
- **Uses:** [tools/liquify.rs](../tools/liquify.md) (`LiquifyField`, dabs, resampling),
  [tools/select_mask.rs](../tools/select_mask.md) (`Mask` for the selection edge),
  [undo.rs](../undo.md), [doc/clip.rs](../doc/clip.md) `Placement`.

## Rust concepts in this file

**Type alias for a tuple.** `type TileAt = (usize, usize);` names a (column, row) pair. More:
[Structs](../../rust-primer.md#structs)

**Destructuring in parameters.** `fn save(&mut self, field: &LiquifyField, (i0, i1, j0, j1): (usize, usize, usize, usize))`
unpacks a 4-tuple argument right in the signature, like JS `function f({a, b})`.

**Bit shifts for sizes.** `256 << 20` is 256 × 2²⁰ = 256 MiB.

## Types

### `const BACKUP_BYTES`
[src/app/liquify.rs:33](../../../src/app/liquify.rs#L33)

```rust
const BACKUP_BYTES: usize = 256 << 20;
```

What the per-stroke copies of the field, kept for Ctrl+Z, may take between them; the oldest go first.

### `const TILE`
[src/app/liquify.rs:37](../../../src/app/liquify.rs#L37)

```rust
const TILE: usize = 32;
```

Grid points per side of one tile of the warp field (32 points = 128 pixels): the unit a stroke backup
saves.

### `type TileAt`
[src/app/liquify.rs:40](../../../src/app/liquify.rs#L40)

```rust
type TileAt = (usize, usize);
```

A tile's (column, row) among the field's tiles.

### `struct StrokeBackup`
[src/app/liquify.rs:46](../../../src/app/liquify.rs#L46)

The field as it was before one stroke, kept only where the stroke wrote: each tile it reached, saved the first time a dab did. A copy of the whole field at every pen-down was 16 MB on an 8K drawing.

| Field | Type | Meaning |
|---|---|---|
| `tiles` | `Vec<(TileAt, Vec<[f32; 2]>)>` | Each tile kept, and the values its points had. |
| `saved` | `std::collections::HashSet<TileAt>` | Which tiles have already been saved for this stroke (saved only the first time a dab touches them). |

### `struct LiquifySession`
[src/app/liquify.rs:89](../../../src/app/liquify.rs#L89)

A warp under way on one drawing.

| Field | Type | Meaning |
|---|---|---|
| `cell` | `CellId` | The drawing being warped. |
| `field` | `LiquifyField` | The warp field: how far each grid point has been pushed. |
| `edge` | `Vec<f32>` | How far each grid point may move — the selection's soft edge. |
| `edge_width` | `f32` | The edge falloff `edge` was built for, to rebuild it when it changes. |
| `clip` | `Option<Arc<Mask>>` | The selection as this drawing sees it; `None` warps the whole drawing. |
| `alpha_lock` | `bool` | Whether the layer is alpha-locked (the warp then keeps the drawing's original coverage). |
| `placement` | `Placement` | Where the drawing sat in the document, for warping its frame-mates. |
| `touched` | `Option<DirtyRect>` | Cell pixels the warp has written since the session began. |
| `last` | `Option<(f32, f32)>` | Where the last dab landed, while the pen is down. |
| `aim` | `Option<((f32, f32), f32)>` | Where the pen has got to since, and its pressure there: dabbed toward once a frame (see `AppState::flush_liquify`). |
| `pending` | `Option<DirtyRect>` | Cell pixels the dabs have changed the field under since the drawing was last resampled, and where those dabs landed. |
| `dabs` | `Vec<(f32, f32)>` | Where this frame's dabs landed, for resampling only what they reached. |
| `backups` | `Vec<StrokeBackup>` | The field before each stroke, newest last: what Ctrl+Z goes back to. |

## `impl StrokeBackup`

### `StrokeBackup::save`
[src/app/liquify.rs:55](../../../src/app/liquify.rs#L55)

```rust
fn save(&mut self, field: &LiquifyField, (i0, i1, j0, j1): (usize, usize, usize, usize))
```

Before a dab writes to grid points `(i0..=i1, j0..=j1)`, saves each tile there that isn't saved yet — so
undoing a stroke only needs the tiles it touched, not a copy of the whole field (16 MB on an 8K drawing).

### `StrokeBackup::restore`
[src/app/liquify.rs:73](../../../src/app/liquify.rs#L73)

```rust
fn restore(&self, field: &mut LiquifyField)
```

Copies every saved tile back into the field (undoing one stroke).

### `StrokeBackup::bytes`
[src/app/liquify.rs:83](../../../src/app/liquify.rs#L83)

```rust
fn bytes(&self) -> usize
```

Memory used by this backup.

## `impl AppState`

### `AppState::liquify_down`
[src/app/liquify.rs:120](../../../src/app/liquify.rs#L120)

```rust
pub(super) fn liquify_down(&mut self, s: PointerSample)
```

Pen down with Liquify: continues this drawing's warp session or starts one (needs an existing drawing on
an editable layer).

### `AppState::liquify_move`
[src/app/liquify.rs:157](../../../src/app/liquify.rs#L157)

```rust
pub(super) fn liquify_move(&mut self, s: PointerSample)
```

Pen moved: a Push notes where the pen is and dabs toward it once per frame (`flush_liquify`); a Smooth dabs
at every sample.

### `AppState::liquify_up`
[src/app/liquify.rs:171](../../../src/app/liquify.rs#L171)

```rust
pub(super) fn liquify_up(&mut self)
```

Pen up: catches the warp up with the pen and forgets the last dab position.

### `AppState::flush_liquify`
[src/app/liquify.rs:185](../../../src/app/liquify.rs#L185)

```rust
pub fn flush_liquify(&mut self)
```

Once per frame: dabs from where the last dab landed to where the pen is now, then resamples what all of
those dabs changed. Batching per frame instead of per pen sample is what keeps a big brush fast.

### `AppState::start_liquify`
[src/app/liquify.rs:202](../../../src/app/liquify.rs#L202)

```rust
fn start_liquify(&mut self) -> bool
```

Starts a session on the current drawing: takes the original-pixels snapshot (usually prepared already
while the pen rested, by `warm_pre`), builds a warp field covering the selection's box (plus a margin) so a
small selection on a huge drawing stays cheap, and the selection's soft edge. Returns `false` if there's
nothing to warp.

### `AppState::liquify_dab`
[src/app/liquify.rs:262](../../../src/app/liquify.rs#L262)

```rust
fn liquify_dab(&mut self, from: (f32, f32), to: (f32, f32), pressure: f32)
```

Dabs from `from` to `to`, spaced a quarter of the brush apart — close enough that a push never carries a
point past the next dab, which stops the warp folding over itself. Backs up tiles before writing.

### `AppState::liquify_render`
[src/app/liquify.rs:315](../../../src/app/liquify.rs#L315)

```rust
fn liquify_render(&mut self, rect: DirtyRect, dabs: Option<(&[(f32, f32)], f32)>)
```

Resamples rectangle `rect` of the drawing from the original through the warp field — all of it, or only
what the given dabs reached — and marks it dirty for texture upload.

### `AppState::commit_liquify`
[src/app/liquify.rs:351](../../../src/app/liquify.rs#L351)

```rust
pub fn commit_liquify(&mut self)
```

Lands the warp as one undo step (including other drawings of the layer with "All frames").

### `AppState::liquify_other_drawings`
[src/app/liquify.rs:379](../../../src/app/liquify.rs#L379)

```rust
fn liquify_other_drawings(&mut self, l: &LiquifySession, touched: DirtyRect) -> Vec<undo::Command>
```

Applies the same warp to every other drawing of the layer. A drawing placed the same way takes the field as
is; one placed differently has each pixel mapped into the active drawing's space, warped, and mapped back.
Returns their undo patches.

### `AppState::cancel_liquify`
[src/app/liquify.rs:470](../../../src/app/liquify.rs#L470)

```rust
pub fn cancel_liquify(&mut self) -> bool
```

Esc: restores the drawing as the session found it and forgets the warp. `true` if there was a session.

### `AppState::liquify_step_back`
[src/app/liquify.rs:492](../../../src/app/liquify.rs#L492)

```rust
pub(super) fn liquify_step_back(&mut self) -> bool
```

Ctrl+Z during a session: undoes the last stroke, or with none left, the whole session.

### `AppState::sync_liquify`
[src/app/liquify.rs:512](../../../src/app/liquify.rs#L512)

```rust
pub(super) fn sync_liquify(&mut self)
```

Per frame: commits the warp once its drawing is no longer the one in hand (another frame/layer/tool).

## Tests

### `mod tests`

Run with `cargo test app::liquify::tests` · 10 tests

| Test | What it checks |
|---|---|
| [`a_liquify_session_commits_as_one_undo_step`](../../../src/app/liquify.rs#L582) | A liquify session commits as one undo step. |
| [`ctrl_z_restores_the_field_exactly_from_its_tiles`](../../../src/app/liquify.rs#L606) | Ctrl+Z puts the field back exactly as it was before each stroke, from the tiles the stroke kept — at the drawing's edge too, where the tiles are cut short. |
| [`esc_restores_exactly`](../../../src/app/liquify.rs#L621) | Esc restores exactly. |
| [`switching_tool_or_frame_commits`](../../../src/app/liquify.rs#L634) | Switching tool or frame commits. |
| [`ctrl_z_steps_back_one_stroke_in_the_session`](../../../src/app/liquify.rs#L659) | Ctrl z steps back one stroke in the session. |
| [`all_frames_lands_the_warp_on_every_drawing`](../../../src/app/liquify.rs#L681) | With "All frames" on, every drawing of the layer takes the same warp, in one undo step — including one drawn on a bigger canvas, where the same place in the document is elsewhere in the cell. |
| [`a_float_is_landed_before_liquify`](../../../src/app/liquify.rs#L708) | A float is landed before liquify. |
| [`a_locked_or_reference_layer_is_refused`](../../../src/app/liquify.rs#L724) | A locked or reference layer is refused. |
| [`nothing_to_warp_on_an_empty_frame`](../../../src/app/liquify.rs#L737) | Nothing to warp on an empty frame. |
| [`tool_adjust_sets_liquify_size_and_strength`](../../../src/app/liquify.rs#L748) | Tool adjust sets liquify size and strength. |

**Test helpers** (used only by the tests above):

- [`dot`](../../../src/app/liquify.rs#L533) — Paint an 8×8 dot with its corner at `(x, y)` in `cell`, outside the history.
- [`centre`](../../../src/app/liquify.rs#L544) — Alpha-weighted centre of `cell`'s pixels.
- [`pixels`](../../../src/app/liquify.rs#L556) — Pixels.
- [`liquify`](../../../src/app/liquify.rs#L561) — A drawing with a dot at (100, 100), Liquify up with a big brush.
- [`push`](../../../src/app/liquify.rs#L572) — A push from `(x, y)` by `dx` to the right, in a few moves.

**Test-only types and constants:** [`RED`](../../../src/app/liquify.rs#L529)
