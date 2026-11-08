# `src/app/pen_slots.rs` — the pen slots (the row of pens in the Tools panel)

> **Source:** [src/app/pen_slots.rs](../../../src/app/pen_slots.rs) · **Module:** `crate::app::pen_slots` · **Size:** 503 lines · Line links as of commit `67a7cb9`

## What this file is for

The Tools panel's first row is a row of **pen slots**. Each slot is a remembered pen: a brush
(any brush from the library), with its own size memory, line smoothing, colour, name and icon.
There are one to six of them — six and the Eraser make a row of seven, as wide as the row of
tools under it. A fresh workspace has two, "Pencil" and "Ink" — the two pens the app always had.
The "+" at the end of the row adds another ("Brush", "Brush 2", …) and opens its menu; the same
menu opens on a right-click, to rename a slot, give it another icon or brush, copy it, or remove
it (the last one stays) ([`pen_slot_menu`](../ui/shell.md#pen_slot_menu) in ui/shell.rs).

In the code a slot is the tool [`ActiveTool::Pen(i)`](../tools/mod.md#enum-activetool), `i` being
its place in the row, and its data is `AppState::pens[i]`, a [`PenSlot`](#struct-penslot). This
file holds that type, the icons a slot can wear, and the `AppState` methods that pick, step
through, add and remove slots.

Slots are a workspace preference: they are saved in the app's prefs (`ui.ron`), never in the
`.anim` project file. A prefs file written before slots existed has no `pens` list; then
[`restore_pens`](#restore_pens) builds the first two slots from the pencil's and the ink's old
saved brush, smoothing and sizes. Going the other way, `save` still writes those old fields from
the first two slots ([`legacy_tool_prefs`](#appstatelegacy_tool_prefs)), so an older build of the
app finds its pencil and ink where it always did.

## How it connects

- **Used by:** [app.rs](../app.md) (`with_prefs` builds `pens`, `dispatch` routes the pen keys,
  `save` writes them), [app/brush_sizes.rs](brush_sizes.md) (each slot's sizes),
  [ui/shell.rs](../ui/shell.md) (the pen row and its right-click menu),
  [ui/brush_picker.rs](../ui/brush_picker.md) (`set_pen_brush`).
- **Uses:** [tools/mod.rs](../tools/mod.md) (`ActiveTool`, `BrushSettings`, smoothing types),
  [input/shortcuts.rs](../input/shortcuts.md) (`Action::PENS`, the pen keys),
  [app/brush_sizes.rs](brush_sizes.md) (`SizeMemory`).

## Rust concepts in this file

**Moving indices after an insert or remove.** A slot is referred to by its position, and several
fields hold one (`tool`, `last_pen`, `prev_pen`, the screen picker's return tool). When a slot is
inserted or removed, [`remap_pens`](#appstateremap_pens) takes a closure `f` that maps an old
position to the new one and applies it to all of them, like `ids.map(f)` in JS:
```rust
self.remap_pens(|j| if j >= at { j + 1 } else { j }); // a copy went in at `at`
```

**`#[serde(skip, default = "basic")]`.** A field that is never written to the prefs file, and
gets `basic()` when one is read. Used for `before_stabilizer`, which only matters for this run.

## Types

### `const MAX_PENS`
[src/app/pen_slots.rs:25](../../../src/app/pen_slots.rs#L25)

```rust
pub const MAX_PENS: usize = Action::PENS.len();
```

The most pen slots the row takes. Eight and the eraser still make a narrower Tools panel than the one row of every tool there was before.

### `const PEN_ICONS`
[src/app/pen_slots.rs:29](../../../src/app/pen_slots.rs#L29)

```rust
pub const PEN_ICONS: [(&str, &str, &str); 16] = [
    ("pencil", ic::PENCIL, "Pencil"),
    ("pencil-simple", ic::PENCIL_SIMPLE, "Plain pencil"),
    ("pencil-line", ic::PENCIL_LINE, "Pencil and line"),
    ("pen", ic::PEN, "Pen"),
    ("pen-nib", ic::PEN_NIB, "Nib"),
// …
```

The icons a slot's button can wear: the name the prefs keep it by, the glyph, and what the picker calls it.

### `struct PenSlot`
[src/app/pen_slots.rs:69](../../../src/app/pen_slots.rs#L69)

One pen slot. Its name and icon are only what the button shows: the brush is what draws.

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | As the artist named it. Left blank, it shows as "Pen n". |
| `icon` | `String` | A `PEN_ICONS` name. Kept as text, so an icon this build doesn't know can't fail the prefs and reset every other preference. |
| `brush` | `BrushSettings` | What the slot draws with: any brush from the library, at its size and colour. |
| `smoothing` | `SmoothingOptions` | The slot's own line smoothing (Basic, Weighted, Stabilizer…). |
| `sizes` | `SizeMemory` | The sizes this slot drew at lately, and the ones pinned. |
| `before_stabilizer` | `Smoothing` | The smoothing it had before the Stabilizer key turned it on, for the key to turn it back to. Session-only. |

## Free functions

### `icon_glyph`
[src/app/pen_slots.rs:50](../../../src/app/pen_slots.rs#L50)

```rust
pub fn icon_glyph(key: &str) -> &'static str
```

The glyph for an icon the prefs name. One this build doesn't know (from a newer one) gets the plain pencil rather than failing the prefs.

### `pen_action`
[src/app/pen_slots.rs:57](../../../src/app/pen_slots.rs#L57)

```rust
pub fn pen_action(i: usize) -> Option<Action>
```

The key action that picks slot `i`. The first two keep the names the pencil's and the ink's keys always had, so a `shortcuts.toml` binding them still binds the same slots.

### `basic`
[src/app/pen_slots.rs:61](../../../src/app/pen_slots.rs#L61)

```rust
fn basic() -> Smoothing
```

Returns `Smoothing::Basic`. Serde's `default = "…"` attribute needs a *function name*, not a value,
so this tiny function exists to give `PenSlot::before_stabilizer` its value when prefs are read.

## `impl Default for PenSlot`

### `PenSlot::default`
[src/app/pen_slots.rs:88](../../../src/app/pen_slots.rs#L88)

```rust
fn default() -> Self
```

A new slot: a pencil brush under a neutral name, ready for whatever brush goes in it.

## `impl PenSlot`

### `PenSlot::new`
[src/app/pen_slots.rs:94](../../../src/app/pen_slots.rs#L94)

```rust
pub fn new(name: &str, icon: &str, brush: BrushSettings) -> Self
```

A slot with this name, icon key and brush, the default smoothing and no remembered sizes.

### `PenSlot::pencil`
[src/app/pen_slots.rs:105](../../../src/app/pen_slots.rs#L105)

```rust
pub fn pencil() -> Self
```

The first slot of a fresh workspace: "Pencil", the pencil icon, `BrushSettings::default_pencil()`.

### `PenSlot::ink`
[src/app/pen_slots.rs:109](../../../src/app/pen_slots.rs#L109)

```rust
pub fn ink() -> Self
```

The second slot of a fresh workspace: "Ink", the nib icon, `BrushSettings::default_ink()`.

### `PenSlot::glyph`
[src/app/pen_slots.rs:113](../../../src/app/pen_slots.rs#L113)

```rust
pub fn glyph(&self) -> &'static str
```

The slot's icon as a font glyph, through [`icon_glyph`](#icon_glyph).

## Free functions

### `default_pens`
[src/app/pen_slots.rs:119](../../../src/app/pen_slots.rs#L119)

```rust
pub fn default_pens() -> Vec<PenSlot>
```

The slots a fresh workspace starts with.

### `restore_pens`
[src/app/pen_slots.rs:126](../../../src/app/pen_slots.rs#L126)

```rust
pub(super) fn restore_pens(prefs: &UiPrefs, legacy: &[BrushSettings; TOOL_COUNT]) -> Vec<PenSlot>
```

The slots the prefs keep, or from a prefs file written before there were slots, its pencil and ink: their brushes, smoothing and sizes as they were. `legacy` is that file's per-tool brushes, pencil and ink first.

## `impl AppState`

### `AppState::pen_name`
[src/app/pen_slots.rs:144](../../../src/app/pen_slots.rs#L144)

```rust
pub fn pen_name(&self, i: usize) -> String
```

Slot `i`'s name as its button and the Tools panel show it.

### `AppState::select_pen`
[src/app/pen_slots.rs:152](../../../src/app/pen_slots.rs#L152)

```rust
pub fn select_pen(&mut self, i: usize)
```

Make slot `i` the tool, if there is one.

### `AppState::step_pen`
[src/app/pen_slots.rs:160](../../../src/app/pen_slots.rs#L160)

```rust
pub fn step_pen(&mut self, forward: bool)
```

The next slot along the row, or the one before, round at the ends. From a tool that isn't a pen, back to the pen used last.

### `AppState::swap_pen`
[src/app/pen_slots.rs:172](../../../src/app/pen_slots.rs#L172)

```rust
pub fn swap_pen(&mut self)
```

Back to the pen used before this one, and pressed again, back here. From a tool that isn't a pen, back to the pen used last.

### `AppState::add_pen`
[src/app/pen_slots.rs:182](../../../src/app/pen_slots.rs#L182)

```rust
pub fn add_pen(&mut self) -> Option<usize>
```

A new slot at the end of the row, holding a pencil brush in the colour of the pen used last, made the tool. Its place, or `None` with the row full.

### `AppState::duplicate_pen`
[src/app/pen_slots.rs:201](../../../src/app/pen_slots.rs#L201)

```rust
pub fn duplicate_pen(&mut self, i: usize) -> Option<usize>
```

A copy of slot `i` right after it: its brush, sizes, smoothing and icon, under its name with a number after it. Made the tool. Its place, or `None` with the row full.

### `AppState::remove_pen`
[src/app/pen_slots.rs:218](../../../src/app/pen_slots.rs#L218)

```rust
pub fn remove_pen(&mut self, i: usize) -> bool
```

Take slot `i` out of the row, its brush with it. The last slot stays. A tool on it goes to the slot before it, or the one after for the first.

### `AppState::set_pen_brush`
[src/app/pen_slots.rs:249](../../../src/app/pen_slots.rs#L249)

```rust
pub fn set_pen_brush(&mut self, i: usize, brush: BrushSettings)
```

Put `brush` in slot `i`. The slot in hand takes it at once; so does Shape while it follows that slot's size.

### `AppState::remap_pens`
[src/app/pen_slots.rs:263](../../../src/app/pen_slots.rs#L263)

```rust
fn remap_pens(&mut self, f: impl Fn(usize) -> usize)
```

Every slot reference after slots moved: `f` takes an old slot to the one now in its place.

### `AppState::legacy_tool_prefs`
[src/app/pen_slots.rs:279](../../../src/app/pen_slots.rs#L279)

```rust
pub(super) fn legacy_tool_prefs(&self) -> (Vec<BrushSettings>, Vec<SmoothingOptions>, Vec<SizeMemory>)
```

The per-tool brushes, smoothing and sizes in the prefs' layout from before the slots, pencil and ink first: the first two slots in their places, so a build from before the slots still finds them.

### `AppState::unused_pen_name`
[src/app/pen_slots.rs:295](../../../src/app/pen_slots.rs#L295)

```rust
fn unused_pen_name(&self, base: &str) -> String
```

`base`, or with a number after it when a slot has that name already.

## Tests

### `mod tests`

Run with `cargo test app::pen_slots::tests` · 13 tests

| Test | What it checks |
|---|---|
| [`a_fresh_workspace_has_the_pencil_and_the_ink`](../../../src/app/pen_slots.rs#L320) | A fresh workspace has the pencil and the ink, and nothing else. |
| [`an_added_slot_goes_at_the_end_in_the_last_pens_colour`](../../../src/app/pen_slots.rs#L331) | A slot added goes at the end and becomes the tool, in the colour of the pen used last, under a name no other slot has. |
| [`a_copy_lands_after_its_slot_with_everything_it_had`](../../../src/app/pen_slots.rs#L348) | A copy lands right after its slot with that slot's brush, sizes and smoothing, numbered on from its name. |
| [`the_row_holds_one_to_six_slots`](../../../src/app/pen_slots.rs#L366) | No more than six slots, and the last one can't go. |
| [`removing_the_slot_in_hand_goes_to_the_one_before`](../../../src/app/pen_slots.rs#L379) | Removing the slot in hand goes to the one before it, with that slot's own brush, not the removed one's. |
| [`removing_an_earlier_slot_keeps_the_pen_in_hand`](../../../src/app/pen_slots.rs#L393) | Removing a slot before the one in hand keeps the same pen in hand. |
| [`shape_follows_the_slot_that_takes_a_removed_ones_place`](../../../src/app/pen_slots.rs#L408) | Shape following a removed slot follows the slot taking its place. |
| [`next_and_previous_go_round_the_row`](../../../src/app/pen_slots.rs#L423) | Next and previous go round the row, and from another tool, back to the pen used last. |
| [`the_swap_goes_between_the_last_two_pens`](../../../src/app/pen_slots.rs#L443) | The swap goes to the pen used before, and back, through other tools. |
| [`a_key_for_a_missing_slot_does_nothing`](../../../src/app/pen_slots.rs#L458) | A slot key with no slot behind it does nothing. |
| [`a_brush_goes_into_its_slot`](../../../src/app/pen_slots.rs#L470) | A brush put in a slot not in hand waits there; in the slot in hand, it draws at once. |
| [`every_slot_keeps_its_own_smoothing`](../../../src/app/pen_slots.rs#L482) | Each slot keeps its own smoothing, the new ones too. |
| [`an_unknown_icon_shows_as_the_plain_pencil`](../../../src/app/pen_slots.rs#L496) | An icon from a newer build shows as the plain pencil. |

**Test helpers** (used only by the tests above):

- [`dispatch_all`](../../../src/app/pen_slots.rs#L312) — Dispatch all.
