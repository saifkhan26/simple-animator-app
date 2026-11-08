# `src/app/button_drag.rs` — what middle/right-button drags do on the canvas

> **Source:** [src/app/button_drag.rs](../../../src/app/button_drag.rs) · **Module:** `crate::app::button_drag` · **Size:** 519 lines · Line links as of commit `67a7cb9`

## What this file is for

A pen tablet's barrel buttons, and a mouse's middle and right buttons, can each be bound
(in Settings) to a *drag gesture* on the canvas. This file carries out four of those
gestures once a drag has started:

- **Scrub** — drag sideways to move the playhead one frame every 10 points. With Shift
  held it moves one *key drawing* at a time instead. The playhead stays where you let go.
- **Flip** — drag sideways to step from drawing to drawing (24 points per drawing), and
  when you let go it jumps back to the frame you started on. This is the animator's
  "flipping": riffling through drawings to check motion without losing your place.
- **Tool adjust** — drag sideways and up/down to change the active tool's two main
  values: size and opacity for a brush, gap and expand for the Fill bucket, size and
  strength for the Liquify warp brush. A small text readout follows the pen.
- **Swatch pie** — a ring of colour swatches appears around the pen; let go over one to
  pick that colour, or in the middle to keep the current one.

The other gestures (Pan, Zoom, Rotate) reuse the normal canvas navigation drag and never
come here. The gesture *types*, the dead zone and the pie geometry live in
[input/button_drag.rs](../input/button_drag.md); this file is the part that changes the
app state.

Like the other files in `src/app/`, this is a child module of [src/app.rs](../app.md),
split out only to keep `app.rs` shorter. Its methods are added to `AppState` with an
`impl AppState` block (see the explanation on the [brush_sizes page](brush_sizes.md#rust-concepts-in-this-file)).

Words used here: a **key drawing** (or *key*) is a frame where a layer's drawing changes;
a **hold** is a frame that keeps showing the previous drawing (see the
[glossary](../../glossary.md)). The **dead zone** is a few points around the press where the
drag does nothing yet, so a tiny wobble of the pen does not change anything.

## How it connects

- **Used by:**
  - [src/ui/shell.rs](../ui/shell.md) — starts the drag (`begin_button_drag` for a button,
    `begin_held_drag` for a held key combo), feeds it pen positions (`button_drag_to`),
    ends it in `finish_canvas_drag` (`end_button_drag`), draws the readout
    (`button_drag_readout`) and draws the swatch pie from `ButtonDragState`.
  - [src/app.rs](../app.md) — re-exports `ButtonDragState`, stores the drag in
    `AppState::button_drag: Option<ButtonDragState>`, and calls `cancel_button_drag` when Esc
    is pressed (`handle_keys`).
- **Uses:**
  - [input/button_drag.rs](../input/button_drag.md) — the `ButtonDrag` enum, `drag_steps`,
    `past_dead_zone`, `pie_slot`, `FLIP_PX_PER_KEY`, `SCRUB_PX_PER_FRAME`.
  - [app/brush_sizes.rs](brush_sizes.md) — `size_with_detents`, `SIZE_DETENT_PX`, and the
    `size_detents` / `size_note` methods.
  - [tools/fill.rs](../tools/fill.md) — `step_gap`, `MAX_EXPAND` for the bucket's values.
  - [tools/liquify.rs](../tools/liquify.md) — `MIN_RADIUS` for the warp brush.
  - [doc/project.rs](../doc/project.md) — `Project::goto`, `Project::jumps_at`.
  - [src/app.rs](../app.md) — `lock_screen_drag`, `pie_colors`, `set_brush_color`,
    `playback.stop()`.

```text
 pen button down ─► ui/shell.rs ─► begin_button_drag / begin_held_drag
                                         │ stores ButtonDragState in AppState::button_drag
 pen moves      ─► ui/shell.rs ─► button_drag_to ─► Scrub/Flip: project.goto(frame)
                                                 ├► ToolAdjust: adjust_tool (brush values)
                                                 └► SwatchPie: pie_hover = pie_slot(…)
 button up      ─► finish_canvas_drag ─► end_button_drag (Flip goes home, pie picks colour)
 Esc            ─► handle_keys ─► cancel_button_drag (undo so far, swallow the rest)
```

## Rust concepts in this file

**`let … else` for early return.** Many methods start by pulling the drag out of
`self.button_drag` (an `Option`) and returning if there is none.
```rust
let Some(mut d) = self.button_drag else {
    return;
};
```
```js
const d = this.buttonDrag;
if (d == null) return;
```
More: [enums and match](../../rust-primer.md#enums-and-match)

**`Copy` structs are copied, not shared.** `ButtonDragState` derives `Copy`, so
`let Some(mut d) = self.button_drag` makes a *copy* of the drag. `button_drag_to` edits
that copy and writes it back at the end with `self.button_drag = Some(d)`. In JS the object
would be shared and edits would show up immediately; here the write-back is what saves them.
More: [ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)

**Struct update syntax `..d`.** Builds a new struct with a few fields changed and the rest
copied from `d` — like object spread in JS, but the spread goes *last*.
```rust
self.button_drag = Some(ButtonDragState { kind: ButtonDrag::None, pie_hover: None, ..d });
```
```js
this.buttonDrag = { ...d, kind: "None", pieHover: null };
```
More: [structs](../../rust-primer.md#structs)

**`match` arms with a binding and a guard.** `t if adjusts_brush(t) => …` matches *any*
tool, names it `t`, and only takes the arm when the condition is true. `matches!(value,
A | B)` is a shorthand that returns `true` when the value fits one of the patterns.
```rust
match self.tool {
    ActiveTool::Fill => { /* gap and expand */ }
    t if adjusts_brush(t) => { /* size and opacity */ }
    ActiveTool::Liquify => { /* size and strength */ }
    _ => {}
}
```
More: [enums and match](../../rust-primer.md#enums-and-match), [macros](../../rust-primer.md#macros)

**`?` on an `Option`.** Inside a function that returns `Option`, `expr?` returns `None`
straight away if `expr` is `None`. `button_drag_readout` uses it:
`let d = self.button_drag.as_ref()?;` — "no drag, no readout". JS's nearest is optional
chaining (`this.buttonDrag?.kind`), but `?` returns from the whole function.
More: [Option and Result](../../rust-primer.md#option-and-result)

**Casts with `as` and `trunc()`.** Rust never converts numbers silently.
`(dx / SCRUB_PX_PER_FRAME).trunc() as i64` drops the fraction (like `Math.trunc`) and then
converts the float to a 64-bit integer. More: [numbers and casts](../../rust-primer.md#numbers-and-casts)

## impl AppState

## Types

### `const SIZE_PX_PER_DOUBLING`
[src/app/button_drag.rs:18](../../../src/app/button_drag.rs#L18)

```rust
const SIZE_PX_PER_DOUBLING: f32 = 150.0;
```

`f32 = 150.0`. Screen points of sideways drag that double (or halve) the brush size.

### `const OPACITY_PX`
[src/app/button_drag.rs:20](../../../src/app/button_drag.rs#L20)

```rust
const OPACITY_PX: f32 = 300.0;
```

`f32 = 300.0`. Screen points of upward drag that take opacity (or Liquify strength) from
nothing to full.

### `const MIN_RADIUS`
[src/app/button_drag.rs:22](../../../src/app/button_drag.rs#L22)

```rust
const MIN_RADIUS: f32 = 0.5;
```

`0.5` and `256.0` (lines 22–23). The brush radius range a size drag can reach — the same as
the size keys'.

### `const MAX_RADIUS`
[src/app/button_drag.rs:23](../../../src/app/button_drag.rs#L23)

```rust
const MAX_RADIUS: f32 = 256.0;
```

`0.5` and `256.0` (lines 22–23). The brush radius range a size drag can reach — the same as
the size keys'.

### `const MIN_OPACITY`
[src/app/button_drag.rs:25](../../../src/app/button_drag.rs#L25)

```rust
const MIN_OPACITY: f32 = 0.01;
```

`f32 = 0.01`. A drag never takes opacity (or strength) all the way to zero, so the brush
never becomes invisible.

### `const MAX_LIQUIFY_RADIUS`
[src/app/button_drag.rs:27](../../../src/app/button_drag.rs#L27)

```rust
const MAX_LIQUIFY_RADIUS: f32 = 1000.0;
```

`f32 = 1000.0`. The largest warp brush a size drag can reach, matching the Liquify slider.

### `struct ButtonDragState`
[src/app/button_drag.rs:31](../../../src/app/button_drag.rs#L31)

One middle- or right-button drag that is under way on the canvas. Stored in
`AppState::button_drag` while the button is held. Derives `Clone, Copy, Debug`.

| Field | Type | Meaning |
|---|---|---|
| `kind` | `ButtonDrag` | Which gesture this is (Scrub, Flip, ToolAdjust, SwatchPie, or `None` to swallow the drag). Public. |
| `anchor` | `egui::Pos2` | Where the press landed, in screen points. Tool adjust keeps the brush ring here; the swatch pie is centred here. Public. |
| `from` | `egui::Pos2` | Where a scrub counts from: the press, or wherever Shift last went down or up (switching between frames and keys restarts the count there). |
| `from_frame` | `usize` | The frame the scrub counts from (paired with `from`). |
| `start_frame` | `usize` | The frame the drag started on. Flip and Esc go back to it. |
| `keys_only` | `bool` | Whether the scrub steps key drawings rather than frames. |
| `shift_locked` | `bool` | The drag was started by a held key combo that includes Shift, so Shift belongs to that combo and must not also mean "keys only". |
| `radius` | `f32` | Brush radius at the press (tool adjust changes it relative to this; Esc restores it). |
| `opacity` | `f32` | Brush opacity at the press. |
| `strength` | `f32` | Liquify strength at the press. |
| `gap` | `u8` | Fill gap-closing size at the press. |
| `expand` | `u8` | Fill expand amount at the press. |
| `pie_hover` | `Option<usize>` | Which swatch of the pie is under the pen, if any. Public. |

## `impl AppState`

### `AppState::begin_button_drag`
[src/app/button_drag.rs:61](../../../src/app/button_drag.rs#L61)

```rust
pub fn begin_button_drag(&mut self, kind: ButtonDrag, at: egui::Pos2)
```

**What it does.** Starts the drag that the pressed button is bound to, at screen point
`at`. Just calls `begin_held_drag(kind, at, false)`. Note that `ButtonDrag::None` also
starts a drag: it does nothing, but it "swallows" the drag so the active tool never sees
it (so the button cannot accidentally paint).

**Used by:** the canvas input code in [ui/shell.rs](../ui/shell.md); tests here and in
[app/liquify.rs](liquify.md).

### `AppState::begin_held_drag`
[src/app/button_drag.rs:67](../../../src/app/button_drag.rs#L67)

```rust
pub fn begin_held_drag(&mut self, kind: ButtonDrag, at: egui::Pos2, shift_locked: bool)
```

**What it does.** Starts a button drag that was triggered by holding a keyboard combo
rather than a pen/mouse button. For Scrub and Flip it first stops playback (playback
rewrites the frame every tick, which would hide the scrub). Then it records a new
`ButtonDragState` with the current frame and the current brush values.

**Parameters.**
- `kind` — the gesture.
- `at` — the press position in screen points.
- `shift_locked` — `true` when the held combo contains Shift. Then Shift does not switch a
  scrub to key-by-key mode.

`keys_only` starts as `true` for Flip, or for Scrub when Shift is held and not
`shift_locked`.

**Used by:** `begin_button_drag`; [ui/shell.rs](../ui/shell.md) for held combos.

### `AppState::button_drag_to`
[src/app/button_drag.rs:93](../../../src/app/button_drag.rs#L93)

```rust
pub fn button_drag_to(&mut self, at: egui::Pos2)
```

**What it does.** Called every frame while the button stays down, with the pen's screen
position. Does nothing if there is no drag. What happens depends on the gesture.

**How it works.**
1. Copy the drag out of `self.button_drag` (or return).
2. **Scrub / Flip:** decide whether to step by keys (always for Flip, for Scrub only when
   Shift is held and not locked). If that changed since last time, restart counting from
   here (`from = at`, `from_frame = current frame`). Then the sideways distance `dx` gives
   the frame: by keys, `key_steps(project, from_frame, dx / FLIP_PX_PER_KEY)`; by frames,
   `from_frame + dx / SCRUB_PX_PER_FRAME`, clamped to the timeline. Go to that frame.
3. **ToolAdjust:** pass the point through `lock_screen_drag` (if Shift was pressed during
   the drag it locks the motion to horizontal or vertical, so only one of the two values
   changes), measure `dx` right and `up` from the anchor, and call `adjust_tool`.
4. **SwatchPie:** find the pie slot under the pen offset with `pie_slot` and store it in
   `pie_hover`.
5. Other kinds: nothing. Write the (possibly changed) copy back to `self.button_drag`.

**Used by:** [ui/shell.rs](../ui/shell.md) while `state.button_drag` is set; tests.

### `AppState::end_button_drag`
[src/app/button_drag.rs:132](../../../src/app/button_drag.rs#L132)

```rust
pub fn end_button_drag(&mut self)
```

**What it does.** The button came up. Removes the drag (`Option::take()` empties the field
and hands back what was in it). A Flip jumps back to the frame it started on; a swatch pie
sets the brush colour to the hovered swatch, if any (let go in the middle and nothing
changes). Scrub and tool adjust just keep their result.

**Used by:** `finish_canvas_drag` in [ui/shell.rs](../ui/shell.md); tests.

### `AppState::cancel_button_drag`
[src/app/button_drag.rs:149](../../../src/app/button_drag.rs#L149)

```rust
pub fn cancel_button_drag(&mut self) -> bool
```

**What it does.** Esc during a button drag: undo what the drag has done so far, and ignore
the rest of it until the button comes up. Scrub and Flip return to the start frame; tool
adjust restores the radius, opacity, strength, gap and expand saved at the press. The drag
is then replaced by one of kind `ButtonDrag::None`, so further pen motion is swallowed.

**Returns.** `true` if there was a drag to cancel, `false` otherwise — so the Esc handler
knows whether to try the next thing.

**Used by:** `AppState::handle_keys` in [src/app.rs](../app.md) (the Esc chain); tests here
and in [app/liquify.rs](liquify.md).

### `AppState::button_drag_readout`
[src/app/button_drag.rs:173](../../../src/app/button_drag.rs#L173)

```rust
pub fn button_drag_readout(&self) -> Option<String>
```

**What it does.** The text to show beside the pen while a button drag runs, if any.

**Returns.**
- Scrub/Flip: `"5 / 12"` (frame number, counting from 1, out of the total), with
  `" · keys"` added when stepping by keys.
- Tool adjust with Fill: `"Gap 14 · Expand 2"`.
- Tool adjust with Pencil/Ink/Eraser/Shape: `"Size 8.0 (pinned) · Opacity 70%"` — the note
  comes from `size_note`.
- Tool adjust with Liquify: `"Size 120 · Strength 50%"`.
- Anything else: `None`.

**Used by:** [ui/shell.rs](../ui/shell.md) (draws it with `draw_readout`); tests here and
in [app/liquify.rs](liquify.md).

### `AppState::adjust_tool`
[src/app/button_drag.rs:209](../../../src/app/button_drag.rs#L209)

```rust
fn adjust_tool(&mut self, d: &ButtonDragState, dx: f32, up: f32)
```

**What it does.** The heart of tool adjust. `dx` is the sideways drag and `up` the upward
drag, in screen points from the press. Values are always computed from the values *at the
press* (stored in `d`), not from the previous frame, so moving back undoes the change.

**Parameters.**
- `d` — the drag state, holding the starting values.
- `dx` — sideways distance (right is positive).
- `up` — upward distance (up is positive).

**How it works.**
- **Fill:** the gap steps along the gap ladder by `drag_steps(dx)` (`step_gap`), and expand
  goes up one per step of `up`, clamped to `0..=MAX_EXPAND` — the same as the bucket's own
  press-drag.
- **Pen slots, Eraser, Shape:** the size scales by powers of two with sideways drag
  (`SIZE_PX_PER_DOUBLING` points per doubling), holding briefly on each remembered size
  (`size_detents` + `size_with_detents`), clamped to `MIN_RADIUS..MAX_RADIUS`. Opacity rises
  by `up / OPACITY_PX`, clamped to `MIN_OPACITY..1`.
- **Liquify:** size scales the same way but without detents and up to
  `MAX_LIQUIFY_RADIUS`; strength takes the place of opacity.
- Any other tool: nothing.

Both distances go through `past_dead_zone` first, so nothing moves until the pen leaves
the dead zone.

**Used by:** `button_drag_to`.

## Free functions

### `adjusts_brush`
[src/app/button_drag.rs:241](../../../src/app/button_drag.rs#L241)

```rust
fn adjusts_brush(t: ActiveTool) -> bool
```

**What it does.** `true` for the tools whose size and opacity tool adjust changes: Pencil,
Ink, Eraser and Shape.

**Used by:** `button_drag_readout`, `adjust_tool`.

### `key_steps`
[src/app/button_drag.rs:252](../../../src/app/button_drag.rs#L252)

```rust
fn key_steps(p: &Project, from: usize, steps: i32) -> usize
```

**What it does.** Finds the frame of the drawing that is `steps` key drawings away from the
one showing on frame `from`, on the active layer. "Key" is decided by `Project::jumps_at`,
the same test the key-jump shortcuts use, so a cleanup layer linked to a rough layer steps
through the rough's drawings too. It stops at the first and last drawing; if there is no
drawing in that direction, it stays where it is.

**Parameters.**
- `p` — the project.
- `from` — the frame to count from.
- `steps` — how many drawings to move; negative goes back.

**Returns.** The target frame index.

**How it works.**
1. If the active layer does not exist, return `from`.
2. `is_key` is a small closure: `|f| p.jumps_at(li, f)`.
3. **Forward** (`steps > 0`): list every key frame after `from`; take the `steps`-th one,
   or the last one if there are fewer, or `from` if there are none.
4. **Backward** (`steps < 0`): first find the drawing actually *showing* on `from` (the
   nearest key at or before it — on a held frame the drawing started earlier). Then list
   keys before that one, newest first, and take the `-steps`-th (or the earliest, or stay
   on the showing drawing).
5. `steps == 0`: return `from`.

**Example.** Keys on frames 0, 3, 6, 9. From frame 7 (holding frame 6's drawing):
`key_steps(p, 7, -1)` → 3 (one back from 6, not from 7); `key_steps(p, 7, 1)` → 9;
`key_steps(p, 7, 5)` → 9 (clamped to the last drawing).

**Used by:** `button_drag_to` (Scrub with keys, and Flip).

## Tests

Run with `cargo test app::button_drag`.

| Test | What it checks |
|---|---|
| [`scrubbing_moves_a_frame_every_ten_points_and_stays`](../../../src/app/button_drag.rs#L312) | Scrub moves one frame per 10 points, ignores up/down, clamps at both ends, and the playhead stays where the drag ends. |
| [`shift_scrubs_key_to_key_from_where_it_went_down`](../../../src/app/button_drag.rs#L330) | Pressing Shift mid-scrub restarts the count from that point and steps one key drawing per 24 points. |
| [`a_scrub_started_by_a_shift_hold_ignores_shift`](../../../src/app/button_drag.rs#L349) | A scrub started by a combo containing Shift keeps stepping frames; one started without it still treats Shift as "keys". |
| [`flipping_goes_drawing_to_drawing_and_lets_go_back_home`](../../../src/app/button_drag.rs#L364) | Flip steps drawing to drawing (counting back from the drawing showing on a held frame), shows `"10 / 12 · keys"`, and returns to the start frame on release. |
| [`flipping_a_linked_layer_follows_the_linked_drawings`](../../../src/app/button_drag.rs#L381) | On an empty cleanup layer linked to the rough (`keys_from`), Flip steps through the rough's drawings. |
| [`tool_adjust_sizes_and_fades_a_brush`](../../../src/app/button_drag.rs#L397) | One doubling-width right doubles the size, 60 points up adds 20% opacity, both clamp at their minimums, and the new size stays after release. |
| [`tool_adjust_holds_on_a_pinned_size_then_goes_on_past`](../../../src/app/button_drag.rs#L415) | The size drag sticks on a pinned size (readout says "(pinned)") and then moves just past it. |
| [`tool_adjust_steps_the_buckets_gap_and_expand_without_filling`](../../../src/app/button_drag.rs#L430) | With Fill active, tool adjust steps gap and expand, shows them in the readout, and paints nothing / adds no undo step. |
| [`the_pie_picks_the_swatch_it_is_let_go_over`](../../../src/app/button_drag.rs#L449) | Letting go over a pie swatch picks its colour; letting go in the centre hole keeps the colour. |
| [`a_pinned_wheel_s_set_leads_the_pie`](../../../src/app/button_drag.rs#L470) | With the colour wheel pinned, its harmony set fills the inner ring and the pinned swatches move to the outer ring. |
| [`esc_puts_everything_back_and_swallows_the_rest`](../../../src/app/button_drag.rs#L500) | Esc returns a scrub to its start frame and ignores further motion; Esc during tool adjust restores the size. |

**Test helpers:** `timeline` ([line 291](../../../src/app/button_drag.rs#L291)) — a test
`AppState` with 12 frames and drawings on frames 0, 3, 6 and 9 (holds between).
`AT` (line 309) — the press point `(400, 300)` used by every test.

### `mod tests`

Run with `cargo test app::button_drag::tests` · 11 tests

| Test | What it checks |
|---|---|
| [`scrubbing_moves_a_frame_every_ten_points_and_stays`](../../../src/app/button_drag.rs#L312) | Scrubbing moves a frame every ten points and stays. |
| [`shift_scrubs_key_to_key_from_where_it_went_down`](../../../src/app/button_drag.rs#L330) | Shift scrubs key to key from where it went down. |
| [`a_scrub_started_by_a_shift_hold_ignores_shift`](../../../src/app/button_drag.rs#L349) | A scrub held on Shift: Shift is the gesture's own key there, so it keeps scrubbing frames rather than jumping key to key. |
| [`flipping_goes_drawing_to_drawing_and_lets_go_back_home`](../../../src/app/button_drag.rs#L364) | Flipping goes drawing to drawing and lets go back home. |
| [`flipping_a_linked_layer_follows_the_linked_drawings`](../../../src/app/button_drag.rs#L381) | Flipping on an empty cleanup layer linked to the rough flips through the rough's drawings, as Shift+A / Shift+S do. |
| [`tool_adjust_sizes_and_fades_a_brush`](../../../src/app/button_drag.rs#L397) | Tool adjust sizes and fades a brush. |
| [`tool_adjust_holds_on_a_pinned_size_then_goes_on_past`](../../../src/app/button_drag.rs#L415) | The size drag holds a moment on a pinned size, says so, then goes on past it: a hair over the pin is in reach. |
| [`tool_adjust_steps_the_buckets_gap_and_expand_without_filling`](../../../src/app/button_drag.rs#L430) | Tool adjust steps the buckets gap and expand without filling. |
| [`the_pie_picks_the_swatch_it_is_let_go_over`](../../../src/app/button_drag.rs#L449) | The pie picks the swatch it is let go over. |
| [`a_pinned_wheel_s_set_leads_the_pie`](../../../src/app/button_drag.rs#L470) | With the wheel pinned its set has the inner ring to itself, and the pinned swatches move out round it. |
| [`esc_puts_everything_back_and_swallows_the_rest`](../../../src/app/button_drag.rs#L500) | Esc puts everything back and swallows the rest. |

**Test helpers** (used only by the tests above):

- [`timeline`](../../../src/app/button_drag.rs#L291) — Twelve frames, with drawings on frames 0, 3, 6 and 9 of the active layer and holds between.

**Test-only types and constants:** [`AT`](../../../src/app/button_drag.rs#L309)
