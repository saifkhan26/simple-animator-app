# `src/app/orient.rs` — flipping and turning the artwork

> **Source:** [src/app/orient.rs](../../../src/app/orient.rs) · **Module:** `crate::app::orient` · **Size:** 207 lines · Line links as of commit `1fa5845`

## What this file is for

The **Flip art** and **Rotate art** commands (horizontal/vertical mirror, 90° either way, 180°) change
the pixels themselves — unlike the view flip, which only changes how the canvas is displayed. They work
on whatever the selection says: floating pixels or a bounded selection turn about their own centre;
otherwise the current drawing (or every drawing on the layer, with the Lasso's "All frames" on) turns
about the cell's centre. All of it is lossless (see [doc/orient.rs](../doc/orient.md)) and one undo step.

A child module of `app` so it can work with `AppState`'s private state directly.

## How it connects

- **Used by:** [app.rs](../app.md) `dispatch` (`ArtFlipH`, `ArtRotateCw`…, `orient_selected_layers`),
  [ui/shell.rs](../ui/shell.md) (layer menu).
- **Uses:** [doc/orient.rs](../doc/orient.md) (`Orient`), [app/select.rs](select.md)
  (`orient_selection`), [undo.rs](../undo.md).

## Rust concepts in this file

**Adding methods to `AppState` from another file.** See [app/select.rs](select.md) for the same pattern.
More: [Impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

## `impl AppState`

### `AppState::orient_art`
[src/app/orient.rs:19](../../../src/app/orient.rs#L19)

```rust
pub fn orient_art(&mut self, o: Orient)
```

Entry point for the art flip/rotate commands: turns the floating or bounded selection if there is one,
otherwise the current drawing (or all of the layer's drawings with "All frames").

### `AppState::orient_drawings`
[src/app/orient.rs:35](../../../src/app/orient.rs#L35)

```rust
pub fn orient_drawings(&mut self, li: usize, o: Orient, every: bool)
```

Turns the current drawing of layer `li` — or with `every`, each of its drawings — about the cell's centre
(which is the canvas centre for an unmoved layer, so a flipped walk cycle stays registered). A quarter
turn of a non-square drawing swaps its width and height instead of cropping. One undo step.

### `AppState::orient_layers`
[src/app/orient.rs:53](../../../src/app/orient.rs#L53)

```rust
pub fn orient_layers(&mut self, layers: &[usize], o: Orient)
```

Turns every drawing on each of `layers` (e.g. a group's members) as `orient_drawings` does, in one undo
step; locked and reference layers are skipped.

### `AppState::orient_cells`
[src/app/orient.rs:72](../../../src/app/orient.rs#L72)

```rust
fn orient_cells(&mut self, cells: Vec<crate::doc::layer::CellId>, resized: &[usize], o: Orient)
```

The shared worker: turns each cell in `cells`, swaps the cell size of `resized` layers for quarter turns,
and records one undo step.

## Tests

### `mod tests`

Run with `cargo test app::orient::tests` · 5 tests

| Test | What it checks |
|---|---|
| [`flipping_the_drawing_is_one_undo_step`](../../../src/app/orient.rs#L125) | Flipping the drawing is one undo step. |
| [`rotating_a_non_square_drawing_swaps_its_size_and_back_exactly`](../../../src/app/orient.rs#L139) | Rotating a non square drawing swaps its size and back exactly. |
| [`flipping_the_layer_keeps_registration`](../../../src/app/orient.rs#L157) | Every drawing on the layer turns about the same centre, so things that lined up still do. |
| [`a_locked_or_reference_layer_is_not_flipped`](../../../src/app/orient.rs#L179) | A locked or reference layer is not flipped. |
| [`rotating_the_whole_layer_resizes_new_keys`](../../../src/app/orient.rs#L195) | Turning the whole layer a quarter turns its new drawings too, and turning it back makes them frame-sized again. |

**Test helpers** (used only by the tests above):

- [`paint`](../../../src/app/orient.rs#L102) — Paint `x0..x1, y0..y1` of `cell` outside the history.
- [`alpha`](../../../src/app/orient.rs#L112) — Alpha.
- [`blob`](../../../src/app/orient.rs#L117) — A blob near the top-left of the current drawing.

**Test-only types and constants:** [`RED`](../../../src/app/orient.rs#L99)
