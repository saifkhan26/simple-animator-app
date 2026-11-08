# `src/ui/color_wheel.rs` — the HSL colour wheel and Color panel

> **Source:** [src/ui/color_wheel.rs](../../../src/ui/color_wheel.rs) · **Module:** `crate::ui::color_wheel` · **Size:** 831 lines · Line links as of commit `36405ad`

## What this file is for

The app's colour picker: an **HSL disc** (hue around the circle, saturation from the centre outwards) beside a
vertical **lightness bar**, plus H°/S%/L% number fields. It appears in two forms: the **Color panel** (with a
harmony set — several related colours shown as dots on the disc and ticks on the bar, see [color.rs](../color.md)),
and a **compact picker** behind every other colour button in the app (replacing egui's own colour button).

**HSL ↔ RGB, worked example.** Hue 210°, saturation 50%, lightness 40%:
`c = (1 − |2·0.4 − 1|)·0.5 = 0.4`, hue sector 3 (180–240°) gives `(r,g,b) = (0, x, c)` with `x = 0.2`, add
`m = 0.4 − 0.2 = 0.2` → `(0.2, 0.4, 0.6)` → RGB `(51, 102, 153)`, i.e. `#336699`.

On the disc, hue 0 (red) is at the top and runs clockwise; the disc itself is drawn as a mesh of coloured
vertices (`SEGMENTS` × `RINGS`) that the GPU blends between.

## How it connects

- **Used by:** [ui/shell.rs](shell.md) (Brush/Color panel, settings colour buttons).
- **Uses:** [color.rs](../color.md) (`Hsl`, `HarmonySet`), [app.rs](../app.md) (`AppState::harmony`, brush colour,
  palette).

## Rust concepts in this file

**Keeping widget state between frames in egui.** `hsl_edit_button` stores the picker's `Hsl` in egui's temporary
memory (`ui.data_mut(...)`) keyed by the widget id, so a grey keeps its hue between frames — immediate-mode widgets
have no instance fields, so persistent bits go in egui's memory or in `AppState`.
More: [egui and immediate mode UI](../../rust-primer.md#egui-and-immediate-mode-ui)

**Returning `Option` for "the user changed something".** `disc(...) -> Option<DiscDrag>` is `None` when nothing
was pressed or dragged this frame.

## Types

### `const SEGMENTS`
[src/ui/color_wheel.rs:14](../../../src/ui/color_wheel.rs#L14)

```rust
const SEGMENTS: usize = 72;
```

Disc mesh resolution around (segments of hue).

### `const RINGS`
[src/ui/color_wheel.rs:15](../../../src/ui/color_wheel.rs#L15)

```rust
const RINGS: usize = 8;
```

Disc mesh resolution outward (rings of saturation).

### `const BAR_W`
[src/ui/color_wheel.rs:17](../../../src/ui/color_wheel.rs#L17)

```rust
pub const BAR_W: f32 = 16.0;
```

Width of the lightness bar.

### `const SET_ADD_W`
[src/ui/color_wheel.rs:19](../../../src/ui/color_wheel.rs#L19)

```rust
const SET_ADD_W: f32 = 22.0;
```

The + that adds the set to the pinned swatches, at the end of its row.

### `const DISC_MAX`
[src/ui/color_wheel.rs:23](../../../src/ui/color_wheel.rs#L23)

```rust
const DISC_MAX: f32 = 159.0;
```

The Brush panel's disc at its largest: what its 200-point opening width leaves for it.

### `const GRAB_R`
[src/ui/color_wheel.rs:25](../../../src/ui/color_wheel.rs#L25)

```rust
const GRAB_R: f32 = 10.0;
```

How near a dot or tick a press has to land to grab it.

### `const POPUP_DISC`
[src/ui/color_wheel.rs:27](../../../src/ui/color_wheel.rs#L27)

```rust
const POPUP_DISC: f32 = 168.0;
```

Disc size in the compact picker.

### `struct Marker`
[src/ui/color_wheel.rs:31](../../../src/ui/color_wheel.rs#L31)

A dot on the disc: one harmony member.

| Field | Type | Meaning |
|---|---|---|
| `index` | `usize` | Which member. |
| `hsl` | `Hsl` | Its colour. |
| `active` | `bool` | Whether the brush paints with it. |

### `struct DiscDrag`
[src/ui/color_wheel.rs:39](../../../src/ui/color_wheel.rs#L39)

A press or drag on the disc: the dot it grabbed (`None` for bare disc) and where that dot should now sit.

| Field | Type | Meaning |
|---|---|---|
| `index` | `Option<usize>` | The grabbed dot, or `None` for bare disc. |
| `h` | `f32` | Where it should now sit: hue. |
| `s` | `f32` | Saturation. |

### `struct Grab`
[src/ui/color_wheel.rs:49](../../../src/ui/color_wheel.rs#L49)

What a press grabbed, held for the rest of the drag. The offset is from the pointer to the grabbed dot's centre, so taking hold of a dot a few pixels off-centre doesn't make it jump.

| Field | Type | Meaning |
|---|---|---|
| `index` | `Option<usize>` | The dot grabbed (`None` for bare disc). |
| `offset` | `Vec2` | Pointer-to-dot offset, so grabbing a dot off-centre doesn't make it jump. |

### `struct Tick`
[src/ui/color_wheel.rs:158](../../../src/ui/color_wheel.rs#L158)

A tick on the lightness bar: one member's lightness.

| Field | Type | Meaning |
|---|---|---|
| `index` | `usize` | Which member. |
| `l` | `f32` | Its lightness. |
| `active` | `bool` | Whether the brush paints with it. |

## Free functions

### `disc_pos`
[src/ui/color_wheel.rs:56](../../../src/ui/color_wheel.rs#L56)

```rust
fn disc_pos(center: Pos2, r: f32, h: f32, s: f32) -> Pos2
```

Where `(h, s)` sits on a disc at `center` with radius `r`. Hue 0 (red) is at the top and runs clockwise.

### `disc_hs`
[src/ui/color_wheel.rs:62](../../../src/ui/color_wheel.rs#L62)

```rust
fn disc_hs(center: Pos2, r: f32, p: Pos2) -> (f32, f32)
```

The `(h, s)` under `p`; past the rim reads as the rim.

### `contrast`
[src/ui/color_wheel.rs:70](../../../src/ui/color_wheel.rs#L70)

```rust
fn contrast(c: Hsl) -> Color32
```

Black or white, whichever reads against `c`.

### `disc`
[src/ui/color_wheel.rs:81](../../../src/ui/color_wheel.rs#L81)

```rust
pub fn disc(ui: &mut Ui, diameter: f32, l: f32, markers: &[Marker]) -> Option<DiscDrag>
```

Paint the hue/saturation disc at lightness `l` with `markers` on it, and report a press or drag. A press within reach of a dot grabs that dot, preferring the active one when they overlap; anywhere else is bare disc.

### `l_bar`
[src/ui/color_wheel.rs:168](../../../src/ui/color_wheel.rs#L168)

```rust
pub fn l_bar(ui: &mut Ui, height: f32, h: f32, s: f32, ticks: &[Tick]) -> Option<(usize, f32)>
```

A vertical lightness bar for hue `h`, saturation `s` — white at the top, black at the bottom — with `ticks` on it. A press takes the nearest tick (the active one when stacked) and drags it; one beyond reach jumps it to the pointer. Returns the tick's index and its new lightness.

### `hsl_fields`
[src/ui/color_wheel.rs:231](../../../src/ui/color_wheel.rs#L231)

```rust
pub fn hsl_fields(ui: &mut Ui, c: Hsl) -> Option<Hsl>
```

H°, S% and L% as drag fields sharing the row's width. Returns the edited colour when one moved. The H, S and L letters go when the row is too narrow for them; the tooltips still say which is which.

### `swatch_button`
[src/ui/color_wheel.rs:274](../../../src/ui/color_wheel.rs#L274)

```rust
fn swatch_button(ui: &mut Ui, rgb: [u8; 3], open: bool) -> Response
```

A flat colour swatch that reads as a button.

### `compact_picker`
[src/ui/color_wheel.rs:290](../../../src/ui/color_wheel.rs#L290)

```rust
fn compact_picker(ui: &mut Ui, c: &mut Hsl) -> bool
```

The compact picker: disc, lightness bar and fields, no harmonies.

### `hsl_edit_button`
[src/ui/color_wheel.rs:322](../../../src/ui/color_wheel.rs#L322)

```rust
pub fn hsl_edit_button(ui: &mut Ui, rgb: &mut [u8; 3]) -> Response
```

A colour button that opens the HSL picker — the app's replacement for egui's own `color_edit_button_*`. The picker keeps its HSL between frames, so a grey keeps its hue and nothing drifts through the 8-bit round trip; it only re-reads the bytes when they change from outside.

### `hsl_edit_button_f32`
[src/ui/color_wheel.rs:359](../../../src/ui/color_wheel.rs#L359)

```rust
pub fn hsl_edit_button_f32(ui: &mut Ui, rgb: &mut [f32; 3]) -> Response
```

`hsl_edit_button` for a colour held as `0..=1` floats.

### `color_panel`
[src/ui/color_wheel.rs:370](../../../src/ui/color_wheel.rs#L370)

```rust
pub fn color_panel(state: &mut AppState, ui: &mut Ui)
```

The Color panel: the harmony scheme, the wheel with every member on it, the members as swatches, and exact H/S/L for the one being painted with.

## Tests

### `mod tests`

Run with `cargo test ui::color_wheel::tests` · 10 tests

| Test | What it checks |
|---|---|
| [`a_press_on_bare_disc_paints_with_what_is_under_it`](../../../src/ui/color_wheel.rs#L666) | A press on bare disc paints with what is under it. |
| [`a_press_on_a_dot_paints_with_it_and_leaves_the_set`](../../../src/ui/color_wheel.rs#L678) | A press on a dot paints with it and leaves the set. |
| [`dragging_a_flank_dot_opens_the_spread`](../../../src/ui/color_wheel.rs#L691) | Dragging a flank dot opens the spread. |
| [`dragging_the_base_dot_turns_every_member`](../../../src/ui/color_wheel.rs#L706) | Dragging the base dot turns every member. |
| [`the_set_rebuilds_when_the_brush_changes_elsewhere`](../../../src/ui/color_wheel.rs#L719) | The set rebuilds when the brush changes elsewhere. |
| [`pin_set_pins_every_member_once`](../../../src/ui/color_wheel.rs#L731) | Pin set pins every member once. |
| [`a_pinned_wheel_keeps_its_set_whatever_picks_a_colour`](../../../src/ui/color_wheel.rs#L748) | Pinned, colours picked elsewhere (the swatch pie, a swatch) never move the set: one of its own selects that member, any other leaves it be. Unpinned, it follows the brush again. |
| [`the_pin_pins_the_wheel_and_lets_it_go`](../../../src/ui/color_wheel.rs#L773) | The pin pins the wheel and lets it go. |
| [`the_colour_button_opens_an_hsl_picker`](../../../src/ui/color_wheel.rs#L787) | The colour button opens an hsl picker. |
| [`disc_positions_round_trip`](../../../src/ui/color_wheel.rs#L821) | Disc positions round trip. |

**Test helpers** (used only by the tests above):

- [`run`](../../../src/ui/color_wheel.rs#L578) — Run one frame of `ui_fn` across a narrow panel-sized screen.
- [`disc_geom`](../../../src/ui/color_wheel.rs#L591) — Centre and radius of the disc painted in `out`, found by its mesh.
- [`text_pos`](../../../src/ui/color_wheel.rs#L606) — Where the text `label` was painted in `out`.
- [`press`](../../../src/ui/color_wheel.rs#L616) — Press.
- [`release`](../../../src/ui/color_wheel.rs#L628) — Release.
- [`brush_rgb`](../../../src/ui/color_wheel.rs#L637) — Brush rgb.
- [`state_on`](../../../src/ui/color_wheel.rs#L643) — A state whose brush and harmony set start on `rgb` under `scheme`.
- [`drag_panel`](../../../src/ui/color_wheel.rs#L651) — Press at `from`, drag through to `to`, let go; returns the disc.
