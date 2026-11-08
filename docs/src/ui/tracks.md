# `src/ui/tracks.rs` — the timeline's layer tracks

> **Source:** [src/ui/tracks.rs](../../../src/ui/tracks.rs) · **Module:** `crate::ui::tracks` · **Size:** 1886 lines · Line links as of commit `67a7cb9`

## What this file is for

Draws and handles the **Timeline panel's tracks**: a frame ruler on top, one row per layer, and every drawing
shown as a **bar** from its key to the next key. The bars are where timing is edited:

- click a bar to select it (Shift for a rectangle of bars, Ctrl to toggle);
- drag a bar's body to **move** it (Ctrl at release to copy) — onto other frames or layers, with the "magnetic"
  rules of `Project::move_blocks` (nothing overwritten);
- drag a bar's **right edge** to change how long that drawing holds;
- right-click for a context menu (clear, close gap, timing "on N"…);
- the ruler: scrub, drag camera keys, see the loop range and the playhead;
- the mouse wheel zooms frames, pans, or scrubs (per the wheel map in Settings).

```text
       0    1    2    3    4    5    6    7     ← ruler (playhead pill)
Ink   [  A     ][  B     ][  C          ]       ← bars: drawing A holds 2 frames, …
Color [  X                    ][  Y     ]
```

While dragging, the result is **previewed** (a retime is run on a copy of the layer) and a label beside the pointer
says what letting go will do.

## How it connects

- **Used by:** [ui/shell.rs](shell.md) (the Timeline panel calls `show`), [app.rs](../app.md) (`AppState::track_sel`,
  `track_drag`, `track_scroll`, `track_frame_w` hold the state).
- **Uses:** [doc/project.rs](../doc/project.md) (`Block`, `blocks`, `move_blocks`, `retime_block`, `layer_retime`,
  `set_timing`, `clear_blocks`, `close_blocks`), [ui/theme.rs](theme.md), [input/shortcuts.rs](../input/shortcuts.md)
  (wheel map).

## Rust concepts in this file

**A geometry helper struct with borrowed slices.** `Geo<'a>` bundles the frame's screen mapping and borrows
`rows: &'a [usize]` from the caller, so converting between frames/layers and screen positions doesn't copy
anything. More: [Lifetimes](../../rust-primer.md#lifetimes)

**Enums describing "what would happen".** `Target::Body { dl, df, moved, ok }` / `Target::Edge { … }` are computed
every frame of a drag and used both for the preview and the actual edit on release.

## Types

### `const DEFAULT_FRAME_W`
[src/ui/tracks.rs:27](../../../src/ui/tracks.rs#L27)

```rust
pub const DEFAULT_FRAME_W: f32 = 16.0;
```

Frame column width for a fresh install.

### `const MIN_FRAME_W`
[src/ui/tracks.rs:28](../../../src/ui/tracks.rs#L28)

```rust
const MIN_FRAME_W: f32 = 6.0;
```

Narrowest frame column (zoomed out).

### `const MAX_FRAME_W`
[src/ui/tracks.rs:29](../../../src/ui/tracks.rs#L29)

```rust
const MAX_FRAME_W: f32 = 48.0;
```

Widest frame column (zoomed in).

### `const ROW_H`
[src/ui/tracks.rs:30](../../../src/ui/tracks.rs#L30)

```rust
const ROW_H: f32 = 24.0;
```

Height of one layer row.

### `const RULER_H`
[src/ui/tracks.rs:31](../../../src/ui/tracks.rs#L31)

```rust
const RULER_H: f32 = 20.0;
```

Height of the frame ruler.

### `const HEADER_W`
[src/ui/tracks.rs:32](../../../src/ui/tracks.rs#L32)

```rust
const HEADER_W: f32 = 132.0;
```

Width of the layer-name column.

### `const BAR_INSET`
[src/ui/tracks.rs:34](../../../src/ui/tracks.rs#L34)

```rust
const BAR_INSET: f32 = 5.0;
```

Space between a bar and the top and bottom of its row.

### `const EDGE_GRAB`
[src/ui/tracks.rs:36](../../../src/ui/tracks.rs#L36)

```rust
const EDGE_GRAB: f32 = 5.0;
```

Width of the grab zone at a bar's right end, in points.

### `const AUTOSCROLL_ZONE`
[src/ui/tracks.rs:38](../../../src/ui/tracks.rs#L38)

```rust
const AUTOSCROLL_ZONE: f32 = 20.0;
```

How close to an edge a drag gets before the tracks scroll that way.

### `const FOLLOW_MARGIN`
[src/ui/tracks.rs:40](../../../src/ui/tracks.rs#L40)

```rust
const FOLLOW_MARGIN: f32 = 3.0;
```

Frames kept between the playhead and the edge when following it.

### `const END_SLACK`
[src/ui/tracks.rs:43](../../../src/ui/tracks.rs#L43)

```rust
const END_SLACK: f32 = 4.0;
```

Frames of room past the end, so the last drawing's edge can be dragged out beyond it.

### `const PREVIEW_BLANK`
[src/ui/tracks.rs:45](../../../src/ui/tracks.rs#L45)

```rust
const PREVIEW_BLANK: CellId = CellId::MAX;
```

Stand-in id for the blank cell a previewed retime would allocate.

### `const HOVER_DELAY`
[src/ui/tracks.rs:47](../../../src/ui/tracks.rs#L47)

```rust
const HOVER_DELAY: f32 = 0.5;
```

How long the pointer rests on a bar before its details show, in seconds.

### `const HEADER_RECT_KEY`
[src/ui/tracks.rs:49](../../../src/ui/tracks.rs#L49)

```rust
const HEADER_RECT_KEY: &str = "tracks_header_rect";
```

Where `show` leaves this frame's header rect for the wheel scrubber.

### `const WELL_ROUNDING`
[src/ui/tracks.rs:54](../../../src/ui/tracks.rs#L54)

```rust
const WELL_ROUNDING: f32 = 6.0;
```

Corner rounding of the tracks well.

### `const BAR_ROUNDING`
[src/ui/tracks.rs:55](../../../src/ui/tracks.rs#L55)

```rust
const BAR_ROUNDING: f32 = 4.0;
```

Corner rounding of drawing bars.

### `const THUMB`
[src/ui/tracks.rs:57](../../../src/ui/tracks.rs#L57)

```rust
const THUMB: f32 = 4.0;
```

Thickness of the scroll thumbs along the bottom and right edges.

### `enum TrackDrag`
[src/ui/tracks.rs:66](../../../src/ui/tracks.rs#L66)

A drag in progress in the tracks (kept in `AppState::track_drag`).

| Variant | Meaning |
|---|---|
| `Body { grab: (usize, usize) }` | Moving the selection — or copying it, if Ctrl is down at release — picked up at this (layer, frame). |
| `Edge { block: Block }` | Stretching one drawing's hold by its right edge. |

### `struct Geo`
[src/ui/tracks.rs:83](../../../src/ui/tracks.rs#L83)

Screen mapping for this frame.

| Field | Type | Meaning |
|---|---|---|
| `tracks` | `Rect` | The tracks area on screen. |
| `fw` | `f32` | Frame column width in points. |
| `scroll` | `egui::Vec2` | Scroll offset. |
| `n_layers` | `usize` | Number of layers. |
| `frame_count` | `usize` | Timeline length. |
| `rows` | `&'a [usize]` | The layers that get a row, top first, as the Layers panel lists them: a folded group's layers have none. |
| `row_of` | `&'a [usize]` | Each layer's row: its own, or for a folded group's layer, the group's. |

### `struct Hit`
[src/ui/tracks.rs:153](../../../src/ui/tracks.rs#L153)

What the pointer is over in the tracks.

| Field | Type | Meaning |
|---|---|---|
| `layer` | `usize` | Layer under the pointer. |
| `frame` | `usize` | Frame under the pointer. |
| `block` | `Option<Block>` | The drawing (bar) under it, if any. |
| `edge` | `bool` | On the grab zone at the block's right end. |

### `enum Target`
[src/ui/tracks.rs:187](../../../src/ui/tracks.rs#L187)

What releasing the current drag would do.

| Variant | Meaning |
|---|---|
| `Body { dl, df, moved, ok }` | Move (or copy) the selection by `dl` layers and `df` frames; `moved` lists where they'd land, `ok` whether allowed. |
| `Edge { block: Block, old_len: usize, new_len: usize, }` | Retime `block` from `old_len` to `new_len` frames. |

### `const KEY_GRAB_H`
[src/ui/tracks.rs:640](../../../src/ui/tracks.rs#L640)

```rust
const KEY_GRAB_H: f32 = 8.0;
```

Height of the strip along the ruler's bottom edge where a camera key's dot is picked up, in points. The dots sit 3 up from the edge.

### `struct BarPaint`
[src/ui/tracks.rs:1023](../../../src/ui/tracks.rs#L1023)

One bar ready to paint.

| Field | Type | Meaning |
|---|---|---|
| `layer` | `usize` | Its layer. |
| `start` | `usize` | First frame. |
| `end` | `usize` | One past the last frame. |
| `cell` | `CellId` | The drawing. |
| `preview` | `bool` | Drawn from a previewed retime rather than the real sheet. |

## Free functions

### `active_tint`
[src/ui/tracks.rs:60](../../../src/ui/tracks.rs#L60)

```rust
fn active_tint() -> Color32
```

The active layer's row, lifted just enough to find.

### `header_rect`
[src/ui/tracks.rs:76](../../../src/ui/tracks.rs#L76)

```rust
pub fn header_rect(ctx: &egui::Context) -> Option<Rect>
```

This frame's header column, if the tracks drew one. The wheel scrolls the rows there instead of scrubbing.

## `impl Geo<'_>`

### `Geo::x`
[src/ui/tracks.rs:97](../../../src/ui/tracks.rs#L97)

```rust
fn x(&self, frame: f32) -> f32
```

Screen x of frame position `frame`.

### `Geo::frame_at`
[src/ui/tracks.rs:102](../../../src/ui/tracks.rs#L102)

```rust
fn frame_at(&self, x: f32) -> isize
```

Frame under screen `x`, unclamped.

### `Geo::frame_on_sheet`
[src/ui/tracks.rs:107](../../../src/ui/tracks.rs#L107)

```rust
fn frame_on_sheet(&self, x: f32) -> usize
```

Frame under screen `x`, clamped onto the timeline.

### `Geo::boundary_at`
[src/ui/tracks.rs:112](../../../src/ui/tracks.rs#L112)

```rust
fn boundary_at(&self, x: f32) -> isize
```

Frame boundary nearest screen `x`.

### `Geo::row_y`
[src/ui/tracks.rs:118](../../../src/ui/tracks.rs#L118)

```rust
fn row_y(&self, layer: usize) -> f32
```

Top of layer `layer`'s row. The top layer is drawn first, matching the Layers panel.

### `Geo::row`
[src/ui/tracks.rs:124](../../../src/ui/tracks.rs#L124)

```rust
fn row(&self, layer: usize) -> Rect
```

The screen rectangle of a layer's row.

### `Geo::layer_at`
[src/ui/tracks.rs:130](../../../src/ui/tracks.rs#L130)

```rust
fn layer_at(&self, y: f32) -> Option<usize>
```

Layer under screen `y`, or `None` off the rows.

### `Geo::layer_on_sheet`
[src/ui/tracks.rs:136](../../../src/ui/tracks.rs#L136)

```rust
fn layer_on_sheet(&self, y: f32) -> usize
```

Layer under screen `y`, clamped onto the sheet.

### `Geo::bar`
[src/ui/tracks.rs:142](../../../src/ui/tracks.rs#L142)

```rust
fn bar(&self, layer: usize, start: usize, end: usize) -> Rect
```

Bar for frames `start..end` on `layer`.

## Free functions

### `hit_test`
[src/ui/tracks.rs:161](../../../src/ui/tracks.rs#L161)

```rust
fn hit_test(state: &AppState, g: &Geo, p: Pos2) -> Option<Hit>
```

What the pointer at `p` is over: layer, frame, the bar there, and whether it's on the bar's right-edge grab zone.

### `drag_target`
[src/ui/tracks.rs:203](../../../src/ui/tracks.rs#L203)

```rust
fn drag_target(state: &AppState, g: &Geo, drag: TrackDrag, p: Pos2, copy: bool) -> Option<Target>
```

Works out the `Target` for a drag at pointer `p`: for a body drag, the layer/frame offset and whether
`can_move_blocks` allows it; for an edge drag, the new hold length.

### `bars`
[src/ui/tracks.rs:255](../../../src/ui/tracks.rs#L255)

```rust
fn bars(exposures: &[Option<CellId>]) -> Vec<(usize, usize, CellId)>
```

(start, end, cell) for every drawing in one layer's exposures.

### `show`
[src/ui/tracks.rs:269](../../../src/ui/tracks.rs#L269)

```rust
pub fn show(state: &mut AppState, ui: &mut egui::Ui)
```

The tracks: ruler, layer headers and drawing bars. Fills the width it is given.

### `click`
[src/ui/tracks.rs:443](../../../src/ui/tracks.rs#L443)

```rust
fn click(state: &mut AppState, h: Hit, mods: egui::Modifiers)
```

A plain click selects the drawing under it, Shift-click selects every drawing in the rectangle back to the last plain click, Ctrl-click toggles one. All three move the cursor there.

### `wheel`
[src/ui/tracks.rs:477](../../../src/ui/tracks.rs#L477)

```rust
fn wheel(state: &mut AppState, ui: &egui::Ui, frames: Rect, header: Rect)
```

The wheel over the tracks, where the wheel map gives it to them: Zoom widens the frames about the pointer, Pan timeline scrolls them sideways, as a sideways trackpad swipe does too. Any wheel over the layer names scrolls the rows. The rest over the frames is left alone — that is `shell::wheel_map`'s.

### `follow_cursor`
[src/ui/tracks.rs:529](../../../src/ui/tracks.rs#L529)

```rust
fn follow_cursor(state: &mut AppState, ui: &egui::Ui, tracks: Rect, id: Id)
```

Bring the playhead and the active layer into view when either changes — only then, so the tracks can still be scrolled away from them freely.

### `clamp_scroll`
[src/ui/tracks.rs:559](../../../src/ui/tracks.rs#L559)

```rust
fn clamp_scroll(state: &mut AppState, tracks: Rect, fc: usize, n_layers: usize)
```

Keeps the scroll offset within the content.

### `autoscroll`
[src/ui/tracks.rs:569](../../../src/ui/tracks.rs#L569)

```rust
fn autoscroll(state: &mut AppState, ctx: &egui::Context, tracks: Rect, p: Pos2, rows: bool)
```

Scroll toward whichever edge a drag is pressing against. Rows only for a move: a retime stays on its own layer.

### `corner_ui`
[src/ui/tracks.rs:593](../../../src/ui/tracks.rs#L593)

```rust
fn corner_ui(state: &mut AppState, ui: &mut egui::Ui, corner: Rect, id: Id)
```

Above the layer names: a label, and zoom buttons — the wheel can do the same, but a gesture nobody can see is one nobody finds.

### `ruler_ui`
[src/ui/tracks.rs:646](../../../src/ui/tracks.rs#L646)

```rust
fn ruler_ui(state: &mut AppState, ui: &mut egui::Ui, g: &Geo, ruler: Rect, id: Id)
```

Frame numbers, camera keys, a loop range that isn't the whole timeline, and the playhead as an accent pill carrying its frame number. Click or drag to scrub — except a drag that starts on a camera key's dot, which takes the key along the timeline instead (Alt leaves a copy behind).

### `header_ui`
[src/ui/tracks.rs:802](../../../src/ui/tracks.rs#L802)

```rust
fn header_ui(state: &mut AppState, ui: &mut egui::Ui, g: &Geo, header: Rect, id: Id)
```

One row per layer: visibility, lock and name, the active layer marked by an accent edge the way the active tool is underlined. Clicking the name selects the layer; the Layers panel keeps everything else.

### `well_lines`
[src/ui/tracks.rs:958](../../../src/ui/tracks.rs#L958)

```rust
fn well_lines(ui: &egui::Ui, outer: Rect, header: Rect, ruler: Rect)
```

The well's edges, drawn over everything inside it: its outline, and the hairlines under the ruler and beside the names.

### `scrollbars`
[src/ui/tracks.rs:969](../../../src/ui/tracks.rs#L969)

```rust
fn scrollbars(state: &mut AppState, ui: &mut egui::Ui, g: &Geo, id: Id)
```

Thin scroll thumbs along the bottom and right edges, only while there is more to see. The plain wheel scrubs here, so these are the one way to scroll the tracks without a modifier key.

### `paint`
[src/ui/tracks.rs:1032](../../../src/ui/tracks.rs#L1032)

```rust
fn paint(
    state: &mut AppState,
    ui: &egui::Ui,
    g: &Geo,
    target: Option<Target>,
    copy: bool,
    hover: Option<Hit>,
)
```

Paints the tracks: row backgrounds, every bar (from the real sheet or a previewed retime/move), selection
highlight, hover, drop target, the playhead line, and onion/grid key marks.

### `drag_label`
[src/ui/tracks.rs:1317](../../../src/ui/tracks.rs#L1317)

```rust
fn drag_label(state: &AppState, t: Target, copy: bool) -> Option<String>
```

The text shown beside the pointer during a drag ("Move 2 drawings +3 frames", "Hold 4 frames", "Can't drop on a
locked layer"…).

### `pointer_label`
[src/ui/tracks.rs:1351](../../../src/ui/tracks.rs#L1351)

```rust
fn pointer_label(ctx: &egui::Context, id: Id, p: Pos2, text: String)
```

A small label beside the pointer. Painted straight onto a tooltip layer rather than laid out as widgets: widgets are hit-tested, and one sitting under the pointer for even a frame would take the press meant for a bar.

### `hover_text`
[src/ui/tracks.rs:1369](../../../src/ui/tracks.rs#L1369)

```rust
fn hover_text(state: &AppState, b: Block) -> String
```

The tooltip for a bar: which drawing, its frames and hold length.

### `context_menu`
[src/ui/tracks.rs:1385](../../../src/ui/tracks.rs#L1385)

```rust
fn context_menu(state: &mut AppState, ui: &mut egui::Ui)
```

The right-click menu on the tracks: delete drawings (leave blank / close gap), set timing "on N", select and similar
actions.

## Tests

### `mod tests`

Run with `cargo test ui::tracks::tests` · 6 tests

| Test | What it checks |
|---|---|
| [`a_plain_drag_moves_the_drawing`](../../../src/ui/tracks.rs#L1558) | A plain drag moves the drawing. |
| [`a_group_row_takes_no_drawings_and_folding_hides_its_layers`](../../../src/ui/tracks.rs#L1568) | A group's row takes no drawings, and a folded group's layers have no rows at all. |
| [`shift_partway_keeps_a_drag_on_its_layer`](../../../src/ui/tracks.rs#L1589) | Shift pressed partway through a drag that has gone further along than across keeps the drawing on its own layer. |
| [`a_ctrl_drag_copies_the_drawing`](../../../src/ui/tracks.rs#L1627) | A ctrl drag copies the drawing. |
| [`no_plain_drag_ever_duplicates`](../../../src/ui/tracks.rs#L1643) | Every plain drag of every drawing, grabbed at every one of its frames, to every frame of both layers: a move never leaves the drawing showing on more frames than it had. |
| [`a_drag_after_resting_on_a_bar_still_moves_it`](../../../src/ui/tracks.rs#L1698) | Resting on a bar until its details show, then dragging it, still drags. egui's at-pointer tooltip used to appear over the pointer and take the press. |

**Test helpers** (used only by the tests above):

- [`sheet`](../../../src/ui/tracks.rs#L1494) — One layer, a distinct drawing keyed on each of `keys`, marked with its key frame + 1 in its first byte (alpha set, so it is not blank).
- [`row`](../../../src/ui/tracks.rs#L1508) — The mark of the drawing showing on every frame of `layer`.
- [`frame`](../../../src/ui/tracks.rs#L1515) — Run one frame of the tracks, laid out from the top-left corner.
- [`at`](../../../src/ui/tracks.rs#L1530) — Centre of `frame` on the top row, at the default zoom with no scroll.
- [`drag`](../../../src/ui/tracks.rs#L1539) — Press at `from`, drag through to `to`, release — each step its own frame, as a real drag arrives.
- [`frames_showing`](../../../src/ui/tracks.rs#L1635) — How many frames of `layer` show the drawing marked `mark`.

### `mod camera_key_drag_tests`

Run with `cargo test ui::tracks::camera_key_drag_tests` · 6 tests

| Test | What it checks |
|---|---|
| [`dragging_a_key_s_dot_retimes_the_key_and_leaves_the_playhead`](../../../src/ui/tracks.rs#L1811) | Dragging a key s dot retimes the key and leaves the playhead. |
| [`alt_dragging_leaves_a_copy_behind`](../../../src/ui/tracks.rs#L1824) | Alt dragging leaves a copy behind. |
| [`a_key_dropped_on_another_takes_its_place`](../../../src/ui/tracks.rs#L1831) | A key dropped on another takes its place. |
| [`a_drag_from_anywhere_else_on_the_ruler_still_scrubs`](../../../src/ui/tracks.rs#L1840) | The pins for the ruler as it was: everything that isn't a drag from a dot still scrubs, and moves no key. |
| [`a_click_on_a_dot_still_goes_to_its_frame`](../../../src/ui/tracks.rs#L1856) | A click on a dot still goes to its frame. |
| [`a_key_dragged_back_to_where_it_was_changes_nothing`](../../../src/ui/tracks.rs#L1870) | A key dragged back to where it was changes nothing. |

**Test helpers** (used only by the tests above):

- [`keyed`](../../../src/ui/tracks.rs#L1744) — Twelve frames, camera keys on 2 (ease in) and 8.
- [`keys`](../../../src/ui/tracks.rs#L1761) — Keys.
- [`frame`](../../../src/ui/tracks.rs#L1765) — Frame.
- [`dot`](../../../src/ui/tracks.rs#L1780) — The dot of a key on `frame`, at the default zoom with no scroll.
- [`number`](../../../src/ui/tracks.rs#L1785) — The same frame's number, up in the ruler clear of the dots.
- [`button`](../../../src/ui/tracks.rs#L1789) — Button.
- [`drag`](../../../src/ui/tracks.rs#L1798) — Drag.
