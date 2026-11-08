# `src/color.rs` — HSL colours and colour harmonies

> **Source:** [src/color.rs](../../src/color.rs) · **Module:** `crate::color` · **Size:** 535 lines · Line links as of commit `67a7cb9`

## What this file is for

The UI talks about colours in **HSL** (hue, saturation, lightness) — the same `hsl(210, 50%, 40%)`
you know from CSS, computed directly on the sRGB byte values exactly as browsers do. Everything else
in the app (pixels, the palette, preferences, exports, Krita) stores plain 8-bit RGB bytes, because
that's what the GPU and file formats want; this file converts between the two.

It also builds **colour harmonies** for the Color panel: given a base colour, a scheme such as
*Complementary* or *Triadic* lays out related colours around the colour wheel, and you can paint with
any member.

And it parses colours typed or pasted as text — `hsl(…)`, `rgb(…)` or hex `#RRGGBB` — so a colour
copied from another app lands correctly.

## How it connects

- **Used by:** [ui/color_wheel.rs](ui/color_wheel.md) (the wheel and swatches),
  [ui/shell.rs](ui/shell.md) (colour fields, paste), [app.rs](app.md) (`AppState::harmony` holds a
  `HarmonySet`).
- **Uses:** only egui's `Color32` type.

## Rust concepts in this file

**Array destructuring and `map`.** `let [r, g, b] = rgb.map(|c| c as f32 / 255.0);` converts all three
bytes to 0–1 floats and unpacks them — like `const [r, g, b] = rgb.map(c => c / 255)`. Arrays have
`.map` in Rust too, returning a new fixed-size array.

**`rem_euclid` vs `%`.** In both JS and Rust `-30 % 360` is `-30`. `(-30.0).rem_euclid(360.0)` is
`330.0` — always a positive remainder, which is what wrapping an angle needs.

**Struct update with `..b`.** `Hsl { l, ..b }` makes a copy of colour `b` with a different lightness —
like `{ ...b, l }` in JS. More: [Structs](../rust-primer.md#structs)

**Ranges as values.** `pub const MONO_STEP: RangeInclusive<f32> = 0.02..=0.3;` stores a range in a
constant; `(0.0..=100.0).contains(&v)` tests membership. More:
[Strings and slices](../rust-primer.md#strings-and-slices)

**Lifetimes in `func_args<'a>`.** The returned `Vec<&'a str>` points into the input text `t: &'a str`,
so no strings are copied; the `'a` tells the compiler the results live as long as the input.
More: [Lifetimes](../rust-primer.md#lifetimes)

## Types

### `struct Hsl`
[src/color.rs:14](../../src/color.rs#L14)

A colour as hue (degrees, `0..360`), saturation and lightness (`0..=1`).

| Field | Type | Meaning |
|---|---|---|
| `h` | `f32` | Hue in degrees, 0 up to (not including) 360: 0 red, 120 green, 240 blue. |
| `s` | `f32` | Saturation 0–1 (0 is grey). |
| `l` | `f32` | Lightness 0–1 (0 black, 0.5 the pure colour, 1 white). |

### `enum Harmony`
[src/color.rs:182](../../src/color.rs#L182)

Which colours the Color panel offers alongside the base.

| Variant | Meaning |
|---|---|
| `Complementary` | The base and the hue opposite it. |
| `Monochromatic` | Five lightnesses of the base hue, darkest first. |
| `Analogous` | The base flanked by its neighbours on the wheel. |
| `Triadic` | Three hues a third of the wheel apart. |
| `Tetradic` | Two complementary pairs: the base, a hue beside it, and their opposites — a rectangle on the wheel. |

### `const Harmony::ALL`
[src/color.rs:199](../../src/color.rs#L199)

```rust
pub const ALL: [Harmony; 5] = [
    Harmony::Complementary,
    Harmony::Monochromatic,
    Harmony::Analogous,
    Harmony::Triadic,
    Harmony::Tetradic,
// …
```

Every harmony scheme, in the order the scheme picker lists them.

### `const ANALOG_SPREAD`
[src/color.rs:239](../../src/color.rs#L239)

```rust
pub const ANALOG_SPREAD: std::ops::RangeInclusive<f32> = 5.0..=90.0;
```

Spread limits, in degrees for the hue schemes and lightness for the monochrome step. The lower ends keep members from collapsing onto the base.

### `const TETRA_SPREAD`
[src/color.rs:240](../../src/color.rs#L240)

```rust
pub const TETRA_SPREAD: std::ops::RangeInclusive<f32> = 15.0..=165.0;
```

The allowed range of `tetra_spread`, in degrees.

### `const MONO_STEP`
[src/color.rs:241](../../src/color.rs#L241)

```rust
pub const MONO_STEP: std::ops::RangeInclusive<f32> = 0.02..=0.3;
```

The allowed range of `mono_step`, in lightness units.

### `const MONO_L`
[src/color.rs:244](../../src/color.rs#L244)

```rust
const MONO_L: std::ops::RangeInclusive<f32> = 0.03..=0.97;
```

How far from black and white the monochrome steps may go, so the ends of a set stay colours rather than all turning into the same black.

### `struct HarmonySet`
[src/color.rs:253](../../src/color.rs#L253)

A harmony laid around a base colour, and which member the brush is on.

The set is its own thing, not a view of the brush: picking a member sets the brush but leaves the set where it is, so you can hop between its colours while painting. It moves when you drag on the wheel, or when the brush colour changes from somewhere else (`HarmonySet::rebase`).

| Field | Type | Meaning |
|---|---|---|
| `scheme` | `Harmony` | Which harmony this set uses. |
| `base` | `Hsl` | The colour the set is built around. |
| `active` | `usize` | The member the brush is painting with. |
| `analog_spread` | `f32` | Degrees between the base and each neighbour in the Analogous scheme (5–90). |
| `tetra_spread` | `f32` | Degrees between the base and the hue beside it in the Tetradic scheme (15–165). |
| `mono_step` | `f32` | Lightness step between neighbours in the Monochromatic scheme (0.02–0.3). |

## `impl Hsl`

### `Hsl::new`
[src/color.rs:21](../../src/color.rs#L21)

```rust
pub fn new(h: f32, s: f32, l: f32) -> Self
```

Makes an HSL colour, wrapping the hue into 0–360 and clamping saturation and lightness into 0–1.

### `Hsl::from_rgb`
[src/color.rs:25](../../src/color.rs#L25)

```rust
pub fn from_rgb(rgb: [u8; 3]) -> Self
```

Converts 8-bit RGB to HSL with the standard (CSS) formulas: lightness is the average of the largest
and smallest channel; saturation is their difference relative to lightness; hue depends on which
channel is largest. Greys (all channels equal) get hue 0 and saturation 0.

**Example.** `[255, 0, 0]` (red) → max 1, min 0 → `l = 0.5`, `s = 1`, hue `60 × ((0−0)/1 mod 6) = 0°`.

### `Hsl::from_rgb_keeping`
[src/color.rs:50](../../src/color.rs#L50)

```rust
pub fn from_rgb_keeping(rgb: [u8; 3], prev: Hsl) -> Self
```

Converts RGB to HSL but keeps what RGB can't express from the previous value `prev`: the hue of a
grey, and the hue and saturation of black and white. Without this the colour picker would jump to red
whenever you dragged to the centre or an end of the lightness bar. If `prev` already converts to the
same bytes it is returned unchanged, so reading the colour back every frame never drifts.

### `Hsl::to_rgb`
[src/color.rs:66](../../src/color.rs#L66)

```rust
pub fn to_rgb(self) -> [u8; 3]
```

Converts HSL back to 8-bit RGB with the standard formula: chroma `c = (1 − |2l − 1|) × s`, an
intermediate `x` based on the hue's position within its 60° sector, the sector picking which channels
get `c` and `x`, then adding `m = l − c/2` and scaling to 0–255.

### `Hsl::to_color32`
[src/color.rs:84](../../src/color.rs#L84)

```rust
pub fn to_color32(self) -> Color32
```

The colour as egui's `Color32` (opaque), for drawing UI.

### `Hsl::fmt`
[src/color.rs:90](../../src/color.rs#L90)

```rust
pub fn fmt(self) -> String
```

Formats the colour as text like `hsl(210, 50%, 40%)` — the form `parse_color` reads back.

## Free functions

### `fmt_rgb`
[src/color.rs:99](../../src/color.rs#L99)

```rust
pub fn fmt_rgb(rgb: [u8; 3]) -> String
```

Formats RGB bytes as `hsl(…)` text.

### `wrap_hue`
[src/color.rs:104](../../src/color.rs#L104)

```rust
pub fn wrap_hue(h: f32) -> f32
```

Folds any angle into 0–360 (e.g. −30 → 330, 400 → 40). The special case handles a floating-point
quirk where a tiny negative number wraps to exactly 360.

### `signed_hue`
[src/color.rs:115](../../src/color.rs#L115)

```rust
fn signed_hue(d: f32) -> f32
```

Folds an angle difference into −180…180: the shortest way round from one hue to another.

### `parse_color`
[src/color.rs:128](../../src/color.rs#L128)

```rust
pub fn parse_color(text: &str) -> Option<[u8; 3]>
```

Reads a colour from text: `hsl(h, s%, l%)`, `rgb(r, g, b)` or hex (`#RGB`, `#RRGGBB`, the `#`
optional). Commas or spaces separate parts, as in CSS, and an alpha part is ignored. Returns `None`
for anything else.

**Example.**
```rust
parse_color("#ff8000")          // Some([255, 128, 0])
parse_color("rgb(255 128 0)")   // Some([255, 128, 0])
parse_color("hsl(30, 100%, 50%)") // Some([255, 128, 0])
parse_color("orange")           // None
```

### `func_args`
[src/color.rs:161](../../src/color.rs#L161)

```rust
fn func_args<'a>(t: &'a str, names: &[&str]) -> Option<Vec<&'a str>>
```

For text like `rgb(1, 2, 3)` with a name from `names`, returns the arguments `["1", "2", "3"]` (split on
commas, spaces and the alpha `/`), as slices of the original string.

### `percent`
[src/color.rs:175](../../src/color.rs#L175)

```rust
fn percent(p: &str) -> Option<f32>
```

Reads `40%` (or a bare `40`) as `0.4`; values outside 0–100 give `None`. `bool.then_some(x)` returns
`Some(x)` only when the condition holds.

## `impl Harmony`

### `Harmony::label`
[src/color.rs:207](../../src/color.rs#L207)

```rust
pub fn label(self) -> &'static str
```

The scheme's display name.

### `Harmony::base_index`
[src/color.rs:219](../../src/color.rs#L219)

```rust
pub fn base_index(self) -> usize
```

Which member of the scheme is the base colour. Analogous and Monochromatic put it in the middle, so the
swatch row reads left to right around it.

### `Harmony::len`
[src/color.rs:227](../../src/color.rs#L227)

```rust
pub fn len(self) -> usize
```

How many colours the scheme has (2, 3, 4 or 5).

## `impl HarmonySet`

### `HarmonySet::new`
[src/color.rs:264](../../src/color.rs#L264)

```rust
pub fn new(scheme: Harmony, rgb: [u8; 3]) -> Self
```

A new set around RGB colour `rgb` with default spreads (30° analogous, 60° tetradic, 0.15 monochrome
step), painting with the base.

### `HarmonySet::len`
[src/color.rs:275](../../src/color.rs#L275)

```rust
pub fn len(&self) -> usize
```

How many members the set has.

### `HarmonySet::set_scheme`
[src/color.rs:280](../../src/color.rs#L280)

```rust
pub fn set_scheme(&mut self, scheme: Harmony)
```

Switches to another scheme around the same base; the brush goes back to the base member.

### `HarmonySet::offset`
[src/color.rs:286](../../src/color.rs#L286)

```rust
fn offset(&self, i: usize) -> f32
```

The hue offset of member `i` from the base: e.g. Complementary `[0, 180]`, Triadic `[0, 120, 240]`,
Analogous `[−spread, 0, +spread]`, Tetradic a rectangle `[0, spread, 180, 180+spread]`.

### `HarmonySet::members`
[src/color.rs:302](../../src/color.rs#L302)

```rust
pub fn members(&self) -> Vec<Hsl>
```

Every colour of the set. For Monochromatic: five lightnesses of the base hue (base unclamped so a black
base still matches the brush; the others kept between 3% and 97%). For the others: the base with each
member's hue offset applied.

### `HarmonySet::member`
[src/color.rs:323](../../src/color.rs#L323)

```rust
pub fn member(&self, i: usize) -> Hsl
```

Member `i` (clamped to the last member).

### `HarmonySet::active_rgb`
[src/color.rs:327](../../src/color.rs#L327)

```rust
pub fn active_rgb(&self) -> [u8; 3]
```

The RGB colour of the member the brush is painting with.

### `HarmonySet::pick`
[src/color.rs:332](../../src/color.rs#L332)

```rust
pub fn pick(&mut self, i: usize)
```

Paints with member `i`; the set itself doesn't move.

### `HarmonySet::move_member`
[src/color.rs:340](../../src/color.rs#L340)

```rust
pub fn move_member(&mut self, i: usize, h: f32, s: f32)
```

Member `i` dragged on the wheel to hue `h`, saturation `s`. Fixed shapes rotate as a whole; dragging an
Analogous or Tetradic flank changes the spread instead, keeping the base. Saturation is shared, so it
moves every member.

### `HarmonySet::set_member_lightness`
[src/color.rs:362](../../src/color.rs#L362)

```rust
pub fn set_member_lightness(&mut self, i: usize, l: f32)
```

Sets member `i`'s lightness. Hue schemes share one lightness; for a monochrome step other than the base
it changes the step size instead.

### `HarmonySet::rebase`
[src/color.rs:376](../../src/color.rs#L376)

```rust
pub fn rebase(&mut self, rgb: [u8; 3])
```

The brush colour changed from outside the panel (eyedropper, swatch, paste, tool switch): rebuilds the
set around the new colour (keeping hue/saturation where RGB can't say them) and selects the base.

## Tests

### `mod tests`

Run with `cargo test color::tests` · 12 tests

| Test | What it checks |
|---|---|
| [`every_8_bit_colour_survives_the_round_trip`](../../src/color.rs#L391) | Every 8 bit colour survives the round trip. |
| [`primaries_land_where_css_puts_them`](../../src/color.rs#L404) | Primaries land where css puts them. |
| [`greys_black_and_white_keep_the_previous_hue`](../../src/color.rs#L414) | Greys black and white keep the previous hue. |
| [`a_value_that_already_rounds_to_the_bytes_does_not_drift`](../../src/color.rs#L426) | A value that already rounds to the bytes does not drift. |
| [`pasted_colours_in_every_form`](../../src/color.rs#L432) | Pasted colours in every form. |
| [`each_scheme_lays_its_hues_around_the_base`](../../src/color.rs#L452) | Each scheme lays its hues around the base. |
| [`picking_a_member_leaves_the_set_in_place`](../../src/color.rs#L470) | Picking a member leaves the set in place. |
| [`dragging_a_fixed_member_turns_the_whole_set`](../../src/color.rs#L479) | Dragging a fixed member turns the whole set. |
| [`dragging_a_flank_opens_the_spread_and_holds_the_base`](../../src/color.rs#L489) | Dragging a flank opens the spread and holds the base. |
| [`a_monochrome_tick_sets_the_step`](../../src/color.rs#L507) | A monochrome tick sets the step. |
| [`a_black_monochrome_base_still_matches_the_brush`](../../src/color.rs#L519) | A black monochrome base still matches the brush. |
| [`rebase_moves_the_set_onto_the_new_colour`](../../src/color.rs#L526) | Rebase moves the set onto the new colour. |

**Test helpers** (used only by the tests above):

- [`close`](../../src/color.rs#L386) — Close.
- [`hues`](../../src/color.rs#L447) — Hues.
