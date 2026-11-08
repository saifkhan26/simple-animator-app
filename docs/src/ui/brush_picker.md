# `src/ui/brush_picker.rs` — the brush picker grid

> **Source:** [src/ui/brush_picker.rs](../../../src/ui/brush_picker.rs) · **Module:** `crate::ui::brush_picker` · **Size:** 207 lines · Line links as of commit `1fa5845`

## What this file is for

The brush picker inside the Brush settings window: a **searchable grid of thumbnails** in the brush library's
three groups (the app's own presets, the Krita presets the app ships, and imported Krita presets), plus a short
plain-language summary of the Krita preset in use ("tip: gradient bar, size from pressure, opacity from tilt…").
Brushes this engine can't paint are shown faded.

## How it connects

- **Used by:** [ui/shell.rs](shell.md) (Brush settings window).
- **Uses:** [brush_library/mod.rs](../brush_library/mod.md) (entries, thumbnails), [tools/krita/mod.rs](../tools/krita/mod.md)
  (`PixelBrush`), [tools/krita/sensors.rs](../tools/krita/sensors.md) (`SensorKind`), [app.rs](../app.md)
  (`AppState::brush`, `brush_thumbs`, `brush_search`).

## Rust concepts in this file

**Out-parameter for a deferred action.** `groups(state, ui, needle, &mut picked)` records the clicked brush id in
`picked` instead of acting immediately — because `state` is only borrowed *immutably* while the grid is drawn;
after drawing, `pick(state, id)` runs with a mutable borrow. A common pattern for satisfying the borrow checker
in immediate-mode UI. More: [Ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)

## Types

### `const TILE`
[src/ui/brush_picker.rs:16](../../../src/ui/brush_picker.rs#L16)

```rust
const TILE: f32 = 48.0;
```

Thumbnail size on screen.

### `const TILE_W`
[src/ui/brush_picker.rs:18](../../../src/ui/brush_picker.rs#L18)

```rust
const TILE_W: f32 = TILE + 16.0;
```

A tile's footprint: the thumbnail with a margin, and two lines of name.

### `const TILE_H`
[src/ui/brush_picker.rs:19](../../../src/ui/brush_picker.rs#L19)

```rust
const TILE_H: f32 = TILE + 32.0;
```

A tile's height (thumbnail plus two lines of name); see `TILE_W`.

### `const PAPER`
[src/ui/brush_picker.rs:22](../../../src/ui/brush_picker.rs#L22)

```rust
pub(crate) const PAPER: egui::Color32 = egui::Color32::from_rgb(226, 226, 222);
```

What thumbnails sit on: a paper colour, so dark strokes read on the dark theme.

## Free functions

### `tile`
[src/ui/brush_picker.rs:26](../../../src/ui/brush_picker.rs#L26)

```rust
fn tile(ui: &mut egui::Ui, thumb: Option<&egui::TextureHandle>, name: &str, selected: bool, enabled: bool) -> egui::Response
```

One brush in the grid: a fixed-size tile, so the grid lines up however long the names are. A brush that cannot paint is drawn faded.

### `sync_thumbs`
[src/ui/brush_picker.rs:61](../../../src/ui/brush_picker.rs#L61)

```rust
fn sync_thumbs(state: &mut AppState, ctx: &egui::Context)
```

Upload the thumbnails the library has decoded and the picker has not.

### `pick`
[src/ui/brush_picker.rs:79](../../../src/ui/brush_picker.rs#L79)

```rust
pub fn pick(state: &mut AppState, id: &str)
```

Pick `id`: an app preset replaces the brush, a Krita preset sets the Krita engine on it at the preset's own size. The colour stays.

### `brush_grid`
[src/ui/brush_picker.rs:97](../../../src/ui/brush_picker.rs#L97)

```rust
pub fn brush_grid(state: &mut AppState, ui: &mut egui::Ui)
```

Draws the search box and the grid of brush groups, then applies a click (`pick`) after drawing.

### `groups`
[src/ui/brush_picker.rs:116](../../../src/ui/brush_picker.rs#L116)

```rust
fn groups(state: &AppState, ui: &mut egui::Ui, needle: &str, picked: &mut Option<String>)
```

Draws each library group whose brushes match the search text, as rows of tiles; records a clicked id in `picked`.

### `sensor_name`
[src/ui/brush_picker.rs:154](../../../src/ui/brush_picker.rs#L154)

```rust
fn sensor_name(k: SensorKind) -> &'static str
```

A sensor's human name for the summary ("pressure", "tilt direction", …).

### `describe`
[src/ui/brush_picker.rs:171](../../../src/ui/brush_picker.rs#L171)

```rust
pub fn describe(b: &PixelBrush) -> Vec<String>
```

What a library preset does, in a few lines.
