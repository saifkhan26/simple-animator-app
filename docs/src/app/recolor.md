# `src/app/recolor.rs` — the Replace color command

> **Source:** [src/app/recolor.rs](../../../src/app/recolor.rs) · **Module:** `crate::app::recolor` · **Size:** 375 lines · Line links as of commit `7901e50`

## What this file is for

The **Replace color** command (Edit menu, or the unbound `ReplaceColor` shortcut): the window's
settings, which drawings it goes over, and the one undo step it makes. The pixel work is
[tools/recolor.rs](../tools/recolor.md).

- **Which drawings** ([`RecolorSpan`](#enum-recolorspan)): this frame, the drawings picked in the
  timeline, a range of frames, or all frames. Every span except the timeline pick uses the selected
  layers (a picked group stands for its layers), less the locked, reference and hidden ones. A drawing
  held over several frames is done once.
- **Selection:** when a bounded selection exists, each drawing is held to it, cut through the layer's
  placement on the frame the drawing shows on.
- **Copies:** drawings that share pixels (repeats, a swing's way back) are recoloured once and go on
  sharing.
- **Undo:** one `structural_edit` with whole-drawing before/after copies (`capture_cells`). The
  timeline selection is kept across it.
- **Screen pick:** each colour well has an eyedropper that starts the app's screen pick.
  `commit_screen_pick` sees `picking` set and fills that well instead of changing the brush.

The settings live for the session only, not in `ui.ron` or the `.anim` file.

A child module of `app` so it can work with `AppState`'s private state directly.

## How it connects

- **Used by:** [app.rs](../app.md) (`dispatch` for `Action::ReplaceColor`, `commit_screen_pick`,
  `end_screen_pick`), [ui/shell.rs](../ui/shell.md) (`recolor_window`, the Edit menu).
- **Uses:** [tools/recolor.rs](../tools/recolor.md) (`recolored`), [tools/select_mask.rs](../tools/select_mask.md)
  (`SelectionMask::to_cell`), [undo.rs](../undo.md) through `structural_edit`.

## Rust concepts in this file

**Adding methods to `AppState` from another file.** See [app/select.rs](select.md) for the same pattern.
More: [Impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

**A `HashMap` keyed by pointer.** `replace_color` keys finished drawings by `Arc::as_ptr`, the address of
the shared pixels, so two cells pointing at the same pixels are done once and end up sharing the new ones.

## Types

### `enum RecolorSpan`
[src/app/recolor.rs:20](../../../src/app/recolor.rs#L20)

Which drawings a colour replace goes over.

| Variant | Meaning |
|---|---|
| `Here` | The drawing each selected layer shows on this frame. |
| `Picked` | The drawings selected in the timeline, whichever layers they are on. |
| `Range` | Every drawing the selected layers show from `start` to `end`. |
| `All` | Every drawing on the selected layers. |

### `enum Well`
[src/app/recolor.rs:34](../../../src/app/recolor.rs#L34)

Which colour well a screen pick fills.

| Variant | Meaning |
|---|---|
| `From` | The colour to replace. |
| `To` | The colour to put in its place. |

### `struct RecolorCfg`
[src/app/recolor.rs:42](../../../src/app/recolor.rs#L42)

The Replace color window and what it was last set to. Kept for the session, not in the prefs: the colours belong to the picture at hand.

| Field | Type | Meaning |
|---|---|---|
| `show` | `bool` | Whether the window is open. |
| `from` | `[u8; 3]` | The colour to replace, as RGB. |
| `to` | `[u8; 3]` | The colour to put in its place. |
| `tolerance` | `u8` | 0..=`recolor::MAX_TOLERANCE` per channel. |
| `span` | `RecolorSpan` | Which drawings it goes over. |
| `start` | `usize` | The Range span's first and last frames, 0-based, both included. |
| `end` | `usize` | See `start`. |
| `picking` | `Option<Well>` | The well a screen pick is up for, while one is. |
| `opened` | `bool` | Whether the window has been opened yet: the first opening takes the brush colour as the one to replace. |

## `impl Default for RecolorCfg`

### `RecolorCfg::default`
[src/app/recolor.rs:60](../../../src/app/recolor.rs#L60)

```rust
fn default() -> Self
```

Closed, black to black, a tolerance of 16, this frame only.

## `impl AppState`

### `AppState::open_recolor`
[src/app/recolor.rs:80](../../../src/app/recolor.rs#L80)

```rust
pub fn open_recolor(&mut self)
```

Open the Replace color window. The first time, the colour to replace is the brush's — pick the old colour, then open it — and the range is the playback loop. Drawings picked in the timeline are what it goes over, when there are some.

### `AppState::pick_recolor_well`
[src/app/recolor.rs:103](../../../src/app/recolor.rs#L103)

```rust
pub fn pick_recolor_well(&mut self, well: Well)
```

Start a screen pick that fills one of the window's colour wells instead of the brush.

### `AppState::recolor_layers`
[src/app/recolor.rs:111](../../../src/app/recolor.rs#L111)

```rust
pub fn recolor_layers(&self) -> Vec<usize>
```

The layers a colour replace may change: the selected ones (a picked group stands for its layers) that show and may be edited. Bottom first.

### `AppState::recolor_targets`
[src/app/recolor.rs:120](../../../src/app/recolor.rs#L120)

```rust
pub fn recolor_targets(&self) -> Vec<(usize, usize, CellId)>
```

The drawings a colour replace goes over, each once, with its layer and the first frame it shows on.

### `AppState::replace_color`
[src/app/recolor.rs:155](../../../src/app/recolor.rs#L155)

```rust
pub fn replace_color(&mut self)
```

Replace the old colour with the new on every drawing the window says, held to the selection when there is one. One undo step; the drawings picked in the timeline stay picked. Says how it went.

## Tests

### `mod tests`

Run with `cargo test app::recolor::tests` · 9 tests

| Test | What it checks |
|---|---|
| [`here_is_the_drawing_on_this_frame_of_the_selected_layer`](../../../src/app/recolor.rs#L255) | Here is the drawing on this frame of the selected layer. |
| [`all_frames_on_every_selected_layer_in_one_undo_step`](../../../src/app/recolor.rs#L265) | All frames on every selected layer in one undo step. |
| [`a_range_takes_the_drawings_it_shows`](../../../src/app/recolor.rs#L280) | A range takes the drawings it shows. |
| [`picked_drawings_go_whatever_layer_they_are_on_and_stay_picked`](../../../src/app/recolor.rs#L290) | Picked drawings go whatever layer they are on and stay picked. |
| [`locked_and_hidden_layers_are_left_alone`](../../../src/app/recolor.rs#L304) | Locked and hidden layers are left alone. |
| [`a_copy_recoloured_goes_on_sharing_its_pixels`](../../../src/app/recolor.rs#L318) | A copy recoloured goes on sharing its pixels. |
| [`the_selection_holds_it_on_every_frame`](../../../src/app/recolor.rs#L330) | The selection holds it on every frame. |
| [`a_colour_that_isnt_there_says_so_and_records_nothing`](../../../src/app/recolor.rs#L350) | A colour that isnt there says so and records nothing. |
| [`a_screen_pick_for_a_well_fills_it_and_leaves_the_brush`](../../../src/app/recolor.rs#L360) | A screen pick for a well fills it and leaves the brush. |

**Test helpers** (used only by the tests above):

- [`scene`](../../../src/app/recolor.rs#L218) — Two layers over 6 frames, drawings on 0 and 3 of each, every one a red square, the window set to turn red blue.
- [`color`](../../../src/app/recolor.rs#L245) — Color.
- [`is_blue`](../../../src/app/recolor.rs#L250) — Is blue.

**Test-only types and constants:** [`RED`](../../../src/app/recolor.rs#L213), [`BLUE`](../../../src/app/recolor.rs#L214)
