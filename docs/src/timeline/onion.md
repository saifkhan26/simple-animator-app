# `src/timeline/onion.rs` — onion-skin settings and which ghosts to draw

> **Source:** [src/timeline/onion.rs](../../../src/timeline/onion.rs) · **Module:** `crate::timeline::onion` · **Size:** 747 lines · Line links as of commit `36405ad`

## What this file is for

**Onion skinning** shows neighbouring drawings faintly behind the one you're drawing, so you can see
the motion: earlier drawings tinted blue, later ones red. This file holds the settings
(`OnionConfig`) and works out, for a layer at a frame, *which* drawings to ghost and how strongly
(`OnionConfig::steps`). The actual ghost images are built and drawn elsewhere
([app.rs](../app.md) `ghost_image`, [ui/shell.rs](../ui/shell.md) `paint_canvas`).

Settings follow Krita's Onion Skins docker: a strip of offsets `−8 … +8`, each one lit or not and with
its own opacity bar. By default it steps by **drawings**, not frames (`by_key`), so animation "on
twos" (each drawing held for two frames) still shows the real previous drawings instead of the same
held drawing twice. The current drawing is never ghosted. **Pins** are specific frames of a layer kept
as ghosts wherever the playhead is.

## How it connects

- **Used by:** [app.rs](../app.md) (`AppState::onion`, ghost textures), [ui/shell.rs](../ui/shell.md)
  (the Onion panel, `onion_strip`, drawing ghosts), [doc/layer.rs](../doc/layer.md) (layers hold their
  `OnionPin`s).
- **Uses:** [doc/layer.rs](../doc/layer.md) (`Layer`, `CellId`, resolving which drawing is on a frame).

## Rust concepts in this file

**Bit masks in a `u8`.** `prev_shown: u8` stores eight on/off flags, one bit per offset: bit `k−1` set
means "ghost `k` steps back is lit". Same operators as JS:
```rust
let lit = self.prev_shown & (1 << (k - 1)) != 0;   // test bit
*mask ^= 1 << (k - 1);                             // flip bit (toggle)
```
`reach` finds the farthest lit offset with `u8::BITS - mask.leading_zeros()` (count of bits up to the
highest set one).

**`From` / `Into` conversions for saving.** `OnionRepr` is the on-disk shape; `impl From<OnionConfig>
for OnionRepr` and the reverse convert between them, so loading can migrate old settings.
`..OnionConfig::default().into()` uses that conversion inside struct-update syntax.
More: [Traits and derive](../../rust-primer.md#traits-and-derive)

**`std::array::from_fn`.** Builds a fixed-size array by calling a closure for each index — like
`Array.from({length: 8}, (_, i) => f(i))`.

## Types

### `const ONION_RANGE`
[src/timeline/onion.rs:19](../../../src/timeline/onion.rs#L19)

```rust
pub const ONION_RANGE: u8 = 8;
```

How many offsets the strip covers each way.

### `const ALPHA_FLOOR`
[src/timeline/onion.rs:23](../../../src/timeline/onion.rs#L23)

```rust
const ALPHA_FLOOR: f32 = 0.18;
```

Alpha floor for the outermost ghost under the old sliders, as a fraction of their `max_alpha`. Only used to carry those settings over.

### `struct OnionConfig`
[src/timeline/onion.rs:27](../../../src/timeline/onion.rs#L27)

All onion-skin settings. A workspace preference saved in `ui.ron` (through `OnionRepr`).

| Field | Type | Meaning |
|---|---|---|
| `enabled` | `bool` | Onion skinning on or off (the 0 in the strip). |
| `prev_tint` | `[u8; 3]` | Silhouette color for previous drawings (unmultiplied RGB). |
| `next_tint` | `[u8; 3]` | Silhouette color for next drawings (unmultiplied RGB). |
| `by_key` | `bool` | Count distinct *drawings* rather than frames when stepping outward, so animation on twos/threes still shows the real drawings back instead of the same held cell repeated. |
| `prev_shown` | `u8` | Lit offsets: bit `k - 1` shows the ghost `k` steps back. The walk reaches as far as the farthest lit one. An unlit offset keeps its slot, so turning off −1 leaves −2 where it was and at its opacity. |
| `next_shown` | `u8` | Same as `prev_shown`, for the ghosts ahead. |
| `prev_opacity` | `[f32; ONION_RANGE as usize]` | Opacity (0..=1) of the ghost `k` steps back, at index `k - 1`. Kept for unlit offsets too, so lighting one again brings its bar back. |
| `next_opacity` | `[f32; ONION_RANGE as usize]` | Same as `prev_opacity`, for the ghosts ahead. |
| `solo` | `Option<Solo>` | What an Alt-click solo put aside. Not saved. |

### `struct Solo`
[src/timeline/onion.rs:54](../../../src/timeline/onion.rs#L54)

One offset shown alone, and the lit offsets to put back.

| Field | Type | Meaning |
|---|---|---|
| `k` | `u8` | Which offset is soloed (1–8). |
| `dir` | `OnionDirection` | Which side (past or future). |
| `saved` | `(u8, u8)` | The lit masks `(prev_shown, next_shown)` to restore when the solo ends. |

### `const PIN_TINTS`
[src/timeline/onion.rs:73](../../../src/timeline/onion.rs#L73)

```rust
pub const PIN_TINTS: [[u8; 3]; 4] = [
    [70, 200, 110],
    [255, 165, 40],
    [185, 110, 255],
    [235, 215, 60],
];
```

Starting colours for new pins, handed out in turn. Kept clear of the default blue/red so a pin never reads as an ordinary prev/next ghost.

### `struct OnionPin`
[src/timeline/onion.rs:84](../../../src/timeline/onion.rs#L84)

A frame of one layer kept on screen as a ghost wherever the playhead is, in its own colour. Session-only: pins live on the `Layer` but aren't written to `.anim`.

| Field | Type | Meaning |
|---|---|---|
| `frame` | `usize` | The pinned frame. |
| `tint` | `[u8; 3]` | Its ghost colour. |
| `visible` | `bool` | Whether it is shown. |
| `opacity` | `f32` | How strongly its ghost shows, 0..=1. |

### `const DEFAULT_SHOWN`
[src/timeline/onion.rs:100](../../../src/timeline/onion.rs#L100)

```rust
const DEFAULT_SHOWN: u8 = 0b111;
```

Offsets lit by default and by Reset: three each way.

### `struct OnionStep`
[src/timeline/onion.rs:131](../../../src/timeline/onion.rs#L131)

One onion ghost to draw: how many steps away it is, which cell to draw, and the frame its layer transform resolves from.

| Field | Type | Meaning |
|---|---|---|
| `k` | `u8` | How many steps away this ghost is (its slot in the strip). |
| `cell` | `CellId` | Which drawing to draw. |
| `frame` | `usize` | The frame whose layer transform positions it. |

### `struct OnionRepr`
[src/timeline/onion.rs:328](../../../src/timeline/onion.rs#L328)

How the onion settings are written to the prefs, and read back — also from the build before the strip, whose Prev / Next / Max α / Falloff sliders seed it.

| Field | Type | Meaning |
|---|---|---|
| `enabled` | `bool` | As `OnionConfig::enabled`. |
| `prev_tint` | `[u8; 3]` | As `OnionConfig::prev_tint`. |
| `next_tint` | `[u8; 3]` | As `OnionConfig::next_tint`. |
| `by_key` | `bool` | As `OnionConfig::by_key`. |
| `strip` | `u8` | 1 once written by the strip. Missing — 0 — means the slider build wrote it, and the strip comes from the fields below. |
| `prev_shown` | `u8` | As `OnionConfig::prev_shown`. |
| `next_shown` | `u8` | As `OnionConfig::next_shown`. |
| `prev_opacity` | `[f32; ONION_RANGE as usize]` | As `OnionConfig::prev_opacity`. |
| `next_opacity` | `[f32; ONION_RANGE as usize]` | As `OnionConfig::next_opacity`. |
| `prev` | `u8` | Old (pre-strip) setting: how many ghosts back. |
| `next` | `u8` | Old setting: how many ghosts ahead. |
| `falloff` | `f32` | Old setting: how fast ghosts faded outward. |
| `max_alpha` | `f32` | Old setting: opacity of the nearest ghost. |
| `prev_hidden` | `u8` | Old setting: bit mask of hidden past ghosts. |
| `next_hidden` | `u8` | Old setting: bit mask of hidden future ghosts. |

### `enum OnionDirection`
[src/timeline/onion.rs:459](../../../src/timeline/onion.rs#L459)

Which side of the current frame a ghost is on.

| Variant | Meaning |
|---|---|
| `Prev` | Earlier drawings (shown blue by default). |
| `Next` | Later drawings (shown red by default). |

## `impl Solo`

### `Solo::masks`
[src/timeline/onion.rs:62](../../../src/timeline/onion.rs#L62)

```rust
fn masks(&self) -> (u8, u8)
```

The lit masks while this solo is active: only bit `k−1` on the soloed side.

## `impl OnionPin`

### `OnionPin::alpha`
[src/timeline/onion.rs:94](../../../src/timeline/onion.rs#L94)

```rust
pub fn alpha(&self) -> u8
```

The pin's opacity as a 0–255 alpha value.

## Free functions

### `default_opacity`
[src/timeline/onion.rs:104](../../../src/timeline/onion.rs#L104)

```rust
fn default_opacity(k: u8) -> f32
```

Default opacity of the ghost `k` steps away: 75% next to the current drawing, 8 points less each step
out (75%, 67%, 59%…).

### `default_opacities`
[src/timeline/onion.rs:108](../../../src/timeline/onion.rs#L108)

```rust
fn default_opacities() -> [f32; ONION_RANGE as usize]
```

The default opacity for all eight offsets, as an array.

## `impl Default for OnionConfig`

### `OnionConfig::default`
[src/timeline/onion.rs:113](../../../src/timeline/onion.rs#L113)

```rust
fn default() -> Self
```

Default settings: off, blue past / red future tints, stepping by drawings, three offsets lit each way
with the default fade.

## `impl OnionConfig`

### `OnionConfig::tint_rgb`
[src/timeline/onion.rs:139](../../../src/timeline/onion.rs#L139)

```rust
pub fn tint_rgb(&self, direction: OnionDirection) -> [u8; 3]
```

The tint colour for a side.

### `OnionConfig::shown_mask`
[src/timeline/onion.rs:146](../../../src/timeline/onion.rs#L146)

```rust
fn shown_mask(&self, direction: OnionDirection) -> u8
```

The lit-offset bit mask for a side.

### `OnionConfig::is_shown`
[src/timeline/onion.rs:154](../../../src/timeline/onion.rs#L154)

```rust
pub fn is_shown(&self, k: u8, direction: OnionDirection) -> bool
```

Whether offset `k` on a side is lit. `matches!(k, 1..=ONION_RANGE)` rejects out-of-range offsets
first.

### `OnionConfig::toggle_shown`
[src/timeline/onion.rs:159](../../../src/timeline/onion.rs#L159)

```rust
pub fn toggle_shown(&mut self, k: u8, direction: OnionDirection)
```

Lights or unlights offset `k` by flipping its bit with XOR. `let mask = match … { … => &mut self.prev_shown, … }`
picks *which field* to modify, then `*mask ^= …` writes through the reference.

### `OnionConfig::solo`
[src/timeline/onion.rs:174](../../../src/timeline/onion.rs#L174)

```rust
pub fn solo(&mut self, k: u8, dir: OnionDirection)
```

Alt-click: shows only ghost `k` on that side; doing it again restores what was lit before. Soloing a
different offset moves the solo but still remembers the original set.

### `OnionConfig::is_soloed`
[src/timeline/onion.rs:195](../../../src/timeline/onion.rs#L195)

```rust
pub fn is_soloed(&self, k: u8, dir: OnionDirection) -> bool
```

Whether offset `k` on a side is the soloed one (and the solo is still in effect).

### `OnionConfig::opacity`
[src/timeline/onion.rs:202](../../../src/timeline/onion.rs#L202)

```rust
pub fn opacity(&self, k: u8, direction: OnionDirection) -> f32
```

The opacity (0–1) of ghost `k` on a side; 0 for out-of-range offsets.

### `OnionConfig::set_opacity`
[src/timeline/onion.rs:213](../../../src/timeline/onion.rs#L213)

```rust
pub fn set_opacity(&mut self, k: u8, direction: OnionDirection, v: f32)
```

Sets ghost `k`'s opacity on a side, clamped to 0–1.

### `OnionConfig::reach`
[src/timeline/onion.rs:226](../../../src/timeline/onion.rs#L226)

```rust
pub fn reach(&self, direction: OnionDirection) -> u8
```

The farthest lit offset on a side (0 if none) — how far `steps` needs to walk.

### `OnionConfig::light`
[src/timeline/onion.rs:232](../../../src/timeline/onion.rs#L232)

```rust
pub fn light(&mut self, prev: u8, next: u8)
```

Lights exactly the nearest `prev` offsets back and `next` ahead: `(1 << n) − 1` is a mask of the
lowest `n` bits (n = 3 → `0b111`).

### `OnionConfig::reset_bars`
[src/timeline/onion.rs:239](../../../src/timeline/onion.rs#L239)

```rust
pub fn reset_bars(&mut self)
```

The panel's Reset: three lit each way at the default fade; colours, on/off and stepping are kept.

### `OnionConfig::alpha_for`
[src/timeline/onion.rs:248](../../../src/timeline/onion.rs#L248)

```rust
pub fn alpha_for(&self, k: u8, direction: OnionDirection) -> u8
```

The 0–255 alpha for ghost `k` on a side (its bar's opacity).

### `OnionConfig::pin_opacity`
[src/timeline/onion.rs:254](../../../src/timeline/onion.rs#L254)

```rust
pub fn pin_opacity(&self) -> f32
```

The opacity a new pin starts at: as strong as the nearest bar on either side.

### `OnionConfig::steps`
[src/timeline/onion.rs:267](../../../src/timeline/onion.rs#L267)

```rust
pub fn steps(
    &self,
    layer: &Layer,
    frame: usize,
    frame_count: usize,
    direction: OnionDirection,
) -> Vec<OnionStep>
```

Lists the ghosts to draw for `layer` around `frame` on one side, nearest first.

**How it works.**
1. Walks outward from `frame` (backwards for Prev, forwards for Next) up to `reach` steps, staying
   inside the timeline.
2. With `by_key` on, each step is the next *different* drawing (holds are skipped); otherwise each step
   is one frame.
3. Skips the drawing currently shown (on a hold it would sit exactly on top), skips unlit offsets
   (which keep their slot), and records `OnionStep { k, cell, frame }` for lit ones.

**Example.** A layer on twos (drawings A A B B C C D D), current frame 6 (drawing D), stepping by
drawings with −1 and −2 lit: ghosts are C (k=1) and B (k=2). Stepping by frames instead: frame 5 is C
(k=1), frame 4 is C again — the same drawing, shown once.

## `impl Default for OnionRepr`

### `OnionRepr::default`
[src/timeline/onion.rs:360](../../../src/timeline/onion.rs#L360)

```rust
fn default() -> Self
```

What a field missing from the saved prefs reads as: the current defaults, but with `strip = 0` so a
table without the marker is treated as coming from the older slider-based version.

## `impl From<OnionConfig> for OnionRepr`

### `OnionRepr::from`
[src/timeline/onion.rs:369](../../../src/timeline/onion.rs#L369)

```rust
fn from(c: OnionConfig) -> Self
```

Converts the live settings to their saved form (with `strip = 1` and the old sliders' defaults filled
in). This is the `From` trait — called through `.into()`.

## `impl From<OnionRepr> for OnionConfig`

### `OnionConfig::from`
[src/timeline/onion.rs:392](../../../src/timeline/onion.rs#L392)

```rust
fn from(r: OnionRepr) -> Self
```

Converts saved settings back to live ones. If they came from the older slider version (`strip == 0`),
rebuilds the strip from the old sliders with `from_sliders`.

## Free functions

### `from_sliders`
[src/timeline/onion.rs:415](../../../src/timeline/onion.rs#L415)

```rust
fn from_sliders(
    count: u8,
    hidden: u8,
    max_alpha: f32,
    falloff: f32,
) -> (u8, [f32; ONION_RANGE as usize])
```

Migrates one side of the old onion settings (count, hidden mask, max opacity, falloff) to the strip:
the first `count` offsets lit minus the hidden ones, each bar at the opacity the old fade curve gave it.

### `pin_ghosts`
[src/timeline/onion.rs:441](../../../src/timeline/onion.rs#L441)

```rust
pub fn pin_ghosts(layer: &Layer, frame: usize, frame_count: usize) -> Vec<(OnionPin, CellId)>
```

The pinned frames of `layer` to ghost at `frame`, each with the drawing it shows — skipping pins on the
current drawing and showing a drawing shared by two pins only once (in the first pin's colour).

## Tests

### `mod tests`

Run with `cargo test timeline::onion::tests` · 18 tests

| Test | What it checks |
|---|---|
| [`default_bars_fade_out_and_stay_visible`](../../../src/timeline/onion.rs#L489) | Default bars fade out and stay visible. |
| [`a_bar_sets_its_ghosts_alpha_alone`](../../../src/timeline/onion.rs#L500) | A bar sets its ghosts alpha alone. |
| [`reach_is_the_farthest_lit_offset`](../../../src/timeline/onion.rs#L514) | Reach is the farthest lit offset. |
| [`by_key_steps_over_holds_to_distinct_drawings`](../../../src/timeline/onion.rs#L525) | By key steps over holds to distinct drawings. |
| [`current_drawing_is_never_ghosted`](../../../src/timeline/onion.rs#L537) | Current drawing is never ghosted. |
| [`steps_stop_at_the_timeline_ends`](../../../src/timeline/onion.rs#L549) | Steps stop at the timeline ends. |
| [`disabled_yields_no_steps`](../../../src/timeline/onion.rs#L559) | Disabled yields no steps. |
| [`unlighting_the_nearest_offset_keeps_the_next_one_in_its_slot`](../../../src/timeline/onion.rs#L566) | Unlighting the nearest offset keeps the next one in its slot. |
| [`a_lone_far_offset_walks_past_unlit_ones`](../../../src/timeline/onion.rs#L581) | A lone far offset walks past unlit ones. |
| [`toggling_twice_lights_the_offset_again`](../../../src/timeline/onion.rs#L594) | Toggling twice lights the offset again. |
| [`frame_stepping_slots_are_frame_distances`](../../../src/timeline/onion.rs#L609) | Frame stepping slots are frame distances. |
| [`solo_shows_one_offset_and_the_same_again_restores`](../../../src/timeline/onion.rs#L624) | Solo shows one offset and the same again restores. |
| [`changing_what_is_lit_ends_a_solo`](../../../src/timeline/onion.rs#L640) | Changing what is lit ends a solo. |
| [`reset_brings_back_the_default_bars_only`](../../../src/timeline/onion.rs#L654) | Reset brings back the default bars only. |
| [`slider_settings_carry_over_to_the_strip`](../../../src/timeline/onion.rs#L672) | Slider settings carry over to the strip. |
| [`the_strip_round_trips_and_a_bare_table_takes_the_slider_defaults`](../../../src/timeline/onion.rs#L690) | The strip round trips and a bare table takes the slider defaults. |
| [`a_new_pin_starts_as_strong_as_the_nearest_bar`](../../../src/timeline/onion.rs#L716) | A new pin starts as strong as the nearest bar. |
| [`pins_skip_the_current_drawing_and_what_they_cannot_show`](../../../src/timeline/onion.rs#L727) | Pins skip the current drawing and what they cannot show. |

**Test helpers** (used only by the tests above):

- [`on_twos`](../../../src/timeline/onion.rs#L469) — Layer keyed every `every` frames over `frames` frames, cells 0.. in order.
- [`lit`](../../../src/timeline/onion.rs#L478) — `n` offsets lit each way.
- [`pin`](../../../src/timeline/onion.rs#L706) — Pin.
