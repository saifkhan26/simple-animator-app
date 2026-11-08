# `src/ui/theme.rs` — colour schemes, fonts and icon buttons

> **Source:** [src/ui/theme.rs](../../../src/ui/theme.rs) · **Module:** `crate::ui::theme` · **Size:** 601 lines · Line links as of commit `67a7cb9`

## What this file is for

The look of the app's own chrome (not the artwork): colour **palettes**, egui styling, the Phosphor icon font,
and a few styled helpers (icon buttons, section headers).

- A `Palette` lists every UI colour: layered backgrounds (base → panel → surface → hover), text, one accent
  colour, timeline colours, warning colour.
- Built-in schemes: **Graphite** (default dark indigo), **Midnight** (blue-black, cyan accent), **Paper** (light).
  A **Custom** scheme is derived from five colours the user picks (`Palette::derive`).
- The current palette is kept in a thread-local cell, and the `tokens!` macro generates one function per colour
  (`theme::text()`, `theme::accent()`, `theme::bg_panel()`…) so UI code can ask for a colour by role.
- `premul` and `white_alpha` make translucent colours correctly (premultiplied in gamma space) — egui's own helpers
  make them too bright, so `clippy.toml` forbids those.

## How it connects

- **Used by:** almost every UI file ([ui/shell.rs](shell.md), [ui/tracks.rs](tracks.md),
  [ui/color_wheel.rs](color_wheel.md)…), [app.rs](../app.md) (`AppState::new` calls `install`, `apply_scheme` calls
  `apply`; `color_scheme`/`custom_colors` are saved in prefs).

## Rust concepts in this file

**A `macro_rules!` that writes functions.**
```rust
macro_rules! tokens {
    ($($name:ident),* $(,)?) => {
        $( pub fn $name() -> Color32 { palette().$name } )*
    };
}
tokens!(bg_base, bg_panel, text, accent, /* … */);
```
For each name in the list, the macro emits a function returning that palette field. `$( … ),*` means "repeat for
each comma-separated item". More: [Macros](../../rust-primer.md#macros)

**`thread_local!` + `Cell`.** `static PALETTE: Cell<Palette>` is a per-thread global you can replace
(`set`) and read (`get`) without `&mut` — the UI thread's current palette.
More: [Constants and statics](../../rust-primer.md#constants-and-statics)

**Associated constants as presets.** `Palette::GRAPHITE`, `Palette::MIDNIGHT`, `Palette::PAPER`.

## Types

### `struct Palette`
[src/ui/theme.rs:17](../../../src/ui/theme.rs#L17)

Every colour the app's chrome uses, by role.

| Field | Type | Meaning |
|---|---|---|
| `bg_base` | `Color32` | Deepest level — window background behind floating panels. |
| `bg_panel` | `Color32` | Floating panel fill (slightly translucent so the canvas glows through). |
| `bg_surface` | `Color32` | Elevated control surface (inputs, headers). |
| `bg_inactive` | `Color32` | Resting widget fill. |
| `bg_hover` | `Color32` | Hovered widget fill. |
| `faint_bg` | `Color32` | Alternate rows in striped lists. |
| `extreme_bg` | `Color32` | Text edits and other sunken fields. |
| `text` | `Color32` | Normal text colour. |
| `text_muted` | `Color32` | Secondary, dimmer text. |
| `text_hover` | `Color32` | Icons and text on a hovered widget. |
| `accent_text` | `Color32` | Text/icon drawn on top of an `accent` fill. |
| `stroke_thin` | `Color32` | Thin outlines and separators. |
| `accent` | `Color32` | The single accent colour for active/selected/playing things. |
| `accent_hover` | `Color32` | The accent when hovered. |
| `accent_dim` | `Color32` | Subtle accent tint for selection backgrounds. |
| `well` | `Color32` | The timeline's inset well, a step below the panel. |
| `row_line` | `Color32` | Hairline between timeline rows — quieter than the well's own edges. |
| `bar` | `Color32` | A drawing's bar on the timeline. |
| `bar_hover` | `Color32` | A timeline bar when hovered. |
| `warn` | `Color32` | Something out of range or clashing. |
| `key_layer` | `Color32` | Timeline markers for layer transform keys and camera keys. |
| `key_camera` | `Color32` | Timeline markers for camera keys (see `key_layer`). |
| `sheen` | `Color32` | What a faint highlight lightens or darkens toward: white over dark surfaces, black over light ones. See `tint`. |
| `light` | `bool` | Surfaces are light, so egui starts from its light visuals. |

### `const Palette::GRAPHITE`
[src/ui/theme.rs:64](../../../src/ui/theme.rs#L64)

```rust
pub const GRAPHITE: Palette = Palette {
    bg_base: Color32::from_rgb(14, 15, 18),
    bg_panel: Color32::from_rgba_premultiplied(22, 24, 28, 240),
    bg_surface: Color32::from_rgb(31, 34, 40),
    bg_inactive: Color32::from_rgb(27, 29, 34),
    bg_hover: Color32::from_rgb(42, 46, 55),
// …
```

The default cool-indigo dark palette.

### `const Palette::MIDNIGHT`
[src/ui/theme.rs:94](../../../src/ui/theme.rs#L94)

```rust
pub const MIDNIGHT: Palette = Palette {
    bg_base: Color32::from_rgb(8, 12, 20),
    bg_panel: Color32::from_rgba_premultiplied(13, 18, 28, 240),
    bg_surface: Color32::from_rgb(22, 30, 44),
    bg_inactive: Color32::from_rgb(18, 25, 37),
    bg_hover: Color32::from_rgb(32, 43, 62),
// …
```

Blue-black with a cyan accent.

### `const Palette::PAPER`
[src/ui/theme.rs:123](../../../src/ui/theme.rs#L123)

```rust
pub const PAPER: Palette = Palette {
    bg_base: Color32::from_rgb(200, 202, 207),
    // premul(236, 237, 240, 245): still a touch see-through.
    bg_panel: Color32::from_rgba_premultiplied(227, 228, 231, 245),
    bg_surface: Color32::from_rgb(252, 252, 253),
    bg_inactive: Color32::from_rgb(226, 228, 233),
// …
```

Light grey surfaces and dark text.

### `enum Scheme`
[src/ui/theme.rs:219](../../../src/ui/theme.rs#L219)

Which colours the UI is drawn in. Stored with the app's preferences, never in a project.

| Variant | Meaning |
|---|---|
| `Graphite` | The default dark scheme. |
| `Midnight` | Blue-black. |
| `Paper` | Light. |
| `Custom` | Worked out from the `BaseColors` kept beside the scheme. |

### `const Scheme::ALL`
[src/ui/theme.rs:229](../../../src/ui/theme.rs#L229)

```rust
pub const ALL: [Scheme; 4] = [Scheme::Graphite, Scheme::Midnight, Scheme::Paper, Scheme::Custom];
```

Every scheme, in menu order.

### `const Scheme::PRESETS`
[src/ui/theme.rs:230](../../../src/ui/theme.rs#L230)

```rust
pub const PRESETS: [Scheme; 3] = [Scheme::Graphite, Scheme::Midnight, Scheme::Paper];
```

The built-in schemes (all but Custom).

### `struct BaseColors`
[src/ui/theme.rs:254](../../../src/ui/theme.rs#L254)

The five colours a custom scheme is picked as.

| Field | Type | Meaning |
|---|---|---|
| `bg` | `[u8; 3]` | Behind the floating panels. |
| `panel` | `[u8; 3]` | Floating panel colour. |
| `widget` | `[u8; 3]` | Buttons, fields and sliders. |
| `text` | `[u8; 3]` | Text colour. |
| `accent` | `[u8; 3]` | Accent colour. |

### `macro_rules! tokens`
[src/ui/theme.rs:301](../../../src/ui/theme.rs#L301)

Generates `pub fn <name>() -> Color32` for each listed palette field, reading the current palette.

## `impl Palette`

### `Palette::derive`
[src/ui/theme.rs:154](../../../src/ui/theme.rs#L154)

```rust
pub fn derive(b: BaseColors) -> Palette
```

A whole palette worked out from five colours. Everything else is a mix of those, the way the presets' in-between shades relate to them, so a custom scheme hangs together however the five are picked.

### `Palette::base`
[src/ui/theme.rs:199](../../../src/ui/theme.rs#L199)

```rust
pub fn base(&self) -> BaseColors
```

The five colours a custom scheme is made from, as this palette has them — where "Start from" begins.

## `impl Scheme`

### `Scheme::label`
[src/ui/theme.rs:232](../../../src/ui/theme.rs#L232)

```rust
pub fn label(self) -> &'static str
```

The scheme's display name.

### `Scheme::palette`
[src/ui/theme.rs:242](../../../src/ui/theme.rs#L242)

```rust
pub fn palette(self, custom: BaseColors) -> Palette
```

The palette this scheme draws in; `custom` is only read by Custom.

## `impl Default for BaseColors`

### `BaseColors::default`
[src/ui/theme.rs:265](../../../src/ui/theme.rs#L265)

```rust
fn default() -> Self
```

Graphite's five base colours.

## Free functions

### `mix`
[src/ui/theme.rs:271](../../../src/ui/theme.rs#L271)

```rust
fn mix(a: Color32, b: Color32, t: f32) -> Color32
```

`a` moved fraction `t` of the way to `b`, in sRGB.

### `luminance`
[src/ui/theme.rs:277](../../../src/ui/theme.rs#L277)

```rust
pub fn luminance(c: Color32) -> f32
```

WCAG relative luminance, 0 (black) to 1 (white).

### `contrast`
[src/ui/theme.rs:286](../../../src/ui/theme.rs#L286)

```rust
pub fn contrast(a: Color32, b: Color32) -> f32
```

WCAG contrast ratio between two colours, 1 to 21.

### `palette`
[src/ui/theme.rs:297](../../../src/ui/theme.rs#L297)

```rust
pub fn palette() -> Palette
```

The palette the UI is drawn in now.

### `tint`
[src/ui/theme.rs:319](../../../src/ui/theme.rs#L319)

```rust
pub fn tint(a: u8) -> Color32
```

A faint highlight at alpha `a`, toward white on a dark palette and black on a light one — what a picked row or a hovered bar is lifted by.

### `window_shadow`
[src/ui/theme.rs:326](../../../src/ui/theme.rs#L326)

```rust
pub fn window_shadow() -> Shadow
```

The shadow under floating windows: soft on light surfaces, where a dark one reads as a smudge.

### `premul`
[src/ui/theme.rs:346](../../../src/ui/theme.rs#L346)

```rust
pub fn premul(r: u8, g: u8, b: u8, a: u8) -> Color32
```

Straight-alpha sRGB colour, premultiplied in gamma space.

### `white_alpha`
[src/ui/theme.rs:352](../../../src/ui/theme.rs#L352)

```rust
pub const fn white_alpha(a: u8) -> Color32
```

White at alpha `a` — also the tint that draws a texture at opacity `a`.

### `install`
[src/ui/theme.rs:356](../../../src/ui/theme.rs#L356)

```rust
pub fn install(ctx: &egui::Context)
```

Called once at startup: adds the Phosphor icon font to egui's fonts, sets text sizes (slightly smaller body text
for a denser look), animation timing, and the initial visuals for both light and dark OS themes.

### `apply`
[src/ui/theme.rs:395](../../../src/ui/theme.rs#L395)

```rust
pub fn apply(ctx: &egui::Context, p: Palette)
```

Draw the UI in `p` from the next frame on.

### `visuals`
[src/ui/theme.rs:404](../../../src/ui/theme.rs#L404)

```rust
fn visuals() -> Visuals
```

Builds egui's `Visuals` (widget fills, strokes, selection colour, window rounding, shadows…) from the current
palette.

### `icon_button`
[src/ui/theme.rs:461](../../../src/ui/theme.rs#L461)

```rust
pub fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response
```

Tooltip-decorated icon-only button. 26×22 minimum.

### `icon_toggle`
[src/ui/theme.rs:475](../../../src/ui/theme.rs#L475)

```rust
pub fn icon_toggle(ui: &mut egui::Ui, icon: &str, tooltip: &str, selected: bool) -> egui::Response
```

Selectable icon button used in the tool palette. When selected it shows the accent fill via the active widget visuals; an accent underline reinforces it.

### `icon_text`
[src/ui/theme.rs:494](../../../src/ui/theme.rs#L494)

```rust
pub fn icon_text(icon: &str, label: &str) -> String
```

An icon glyph followed by a label, as one string.

### `section_header`
[src/ui/theme.rs:498](../../../src/ui/theme.rs#L498)

```rust
pub fn section_header(ui: &mut egui::Ui, icon: &str, title: &str)
```

Draws a panel section header: an icon and a title in the muted header style.

## Tests

### `mod tests`

Run with `cargo test ui::theme::tests` · 5 tests

| Test | What it checks |
|---|---|
| [`every_preset_reads`](../../../src/ui/theme.rs#L540) | Every preset reads. |
| [`a_custom_scheme_made_from_a_preset_reads_too`](../../../src/ui/theme.rs#L547) | A custom scheme made from a preset reads too. |
| [`a_custom_scheme_keeps_its_five_colours`](../../../src/ui/theme.rs#L556) | A custom scheme keeps its five colours. |
| [`applying_a_scheme_restyles_both_slots`](../../../src/ui/theme.rs#L573) | The scheme lands in both of egui's style slots, and the accessors follow it. |
| [`premultiplying_stays_in_gamma_space`](../../../src/ui/theme.rs#L589) | Premultiplying stays in gamma space. |

**Test helpers** (used only by the tests above):

- [`over`](../../../src/ui/theme.rs#L513) — Opaque colour a translucent panel fill shows as over `under`.
- [`readable`](../../../src/ui/theme.rs#L521) — Body text, muted text and the accent read on the surfaces they sit on: WCAG AA for text (4.5), and 3 for muted labels and the accent.
