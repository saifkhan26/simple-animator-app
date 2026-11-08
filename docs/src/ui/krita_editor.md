# `src/ui/krita_editor.rs` — the Krita brush editor

> **Source:** [src/ui/krita_editor.rs](../../../src/ui/krita_editor.rs) · **Module:** `crate::ui::krita_editor` · **Size:** 489 lines · Line links as of commit `1fa5845`

## What this file is for

The **Krita brush editor** section of Brush settings, laid out after Krita's own brush editor but trimmed to what
this engine paints: painting mode (build-up or wash, wash flow), the tip (which tip; for a generated tip its
shape, ratio, fade, spikes; angle and spacing), the paper texture, and each **curve option** (Size, Opacity, Flow,
Rotation, Scatter, …) with its on/off switch, strength, the sensors it reads, how they combine, and the curve of the
picked sensor (drawn with [ui/curve_editor.rs](curve_editor.md)).

Edits are not written into the preset: they're kept as **overrides** — `(parameter, value)` pairs applied over the
preset (`BrushSettings::krita_overrides`), so **Reset** simply clears them, and **Save as new** writes a new preset.

## How it connects

- **Used by:** [ui/shell.rs](shell.md) (Brush settings window).
- **Uses:** [tools/krita/kpp.rs](../tools/krita/kpp.md) (`Preset`, `OptionData`, `with_overrides`),
  [tools/krita/edit.rs](../tools/krita/edit.md) (option specs), [ui/curve_editor.rs](curve_editor.md),
  [brush_library/mod.rs](../brush_library/mod.md) (saving as new), [app.rs](../app.md) (`AppState::brush`).

## Rust concepts in this file

**Diffing against a base.** Each control reads the current value from `cur` (preset with overrides) and, when the
user changes it, writes an override only if it differs from `base` (the preset itself) — so overrides stay minimal.

## Types

### `const BUILT_IN`
[src/ui/krita_editor.rs:18](../../../src/ui/krita_editor.rs#L18)

```rust
const BUILT_IN: &str = "krita/c)_Pencil-5_Tilted";
```

What a Krita brush without a library id paints: the built-in Pencil-5.

### `const SENSORS`
[src/ui/krita_editor.rs:21](../../../src/ui/krita_editor.rs#L21)

```rust
const SENSORS: [(&str, &str); 11] = [
    ("pressure", "Pressure"),
    ("speed", "Speed"),
    ("drawingangle", "Direction"),
    ("fuzzy", "Random"),
    ("fade", "Fade"),
// …
```

The sensors this engine reads, with the names the editor shows.

### `const MODES`
[src/ui/krita_editor.rs:36](../../../src/ui/krita_editor.rs#L36)

```rust
const MODES: [&str; 5] = ["Multiply", "Add", "Maximum", "Minimum", "Difference"];
```

The ways two sensors' values can combine (multiply, add, max, min, difference).

## Free functions

### `sensor_name`
[src/ui/krita_editor.rs:38](../../../src/ui/krita_editor.rs#L38)

```rust
fn sensor_name(id: &str) -> &str
```

The editor's display name for a sensor id.

### `r3`
[src/ui/krita_editor.rs:43](../../../src/ui/krita_editor.rs#L43)

```rust
fn r3(v: f64) -> f64
```

Round a slider's value so an override reads like Krita's own.

### `krita_editor`
[src/ui/krita_editor.rs:47](../../../src/ui/krita_editor.rs#L47)

```rust
pub fn krita_editor(state: &mut AppState, ui: &mut egui::Ui)
```

Draws the whole editor for the current Krita brush: works out the base preset and the current one with overrides,
then the save row, painting mode, tip, texture and each curve option; stores any changed overrides back on the
brush.

### `save_row`
[src/ui/krita_editor.rs:125](../../../src/ui/krita_editor.rs#L125)

```rust
fn save_row(ui: &mut egui::Ui, name: &str, o: &mut Vec<(String, String)>, save_as: &mut Option<String>)
```

Reset, and Save as new with its name field.

### `painting`
[src/ui/krita_editor.rs:169](../../../src/ui/krita_editor.rs#L169)

```rust
fn painting(ui: &mut egui::Ui, base: &Preset, cur: &Preset, o: &mut Vec<(String, String)>) -> bool
```

Build-up or wash, and wash's flow. Returns whether it washes.

### `tip`
[src/ui/krita_editor.rs:205](../../../src/ui/krita_editor.rs#L205)

```rust
fn tip(ui: &mut egui::Ui, base: &Preset, cur: &Preset, o: &mut Vec<(String, String)>, tips: &[String], diameter: f64) -> bool
```

The tip: which, its shape if generated, angle and spacing. Returns whether it is generated.

### `texture`
[src/ui/krita_editor.rs:334](../../../src/ui/krita_editor.rs#L334)

```rust
fn texture(ui: &mut egui::Ui, base: &Preset, cur: &Preset, o: &mut Vec<(String, String)>) -> bool
```

The paper texture, for a preset that has one. Returns whether it is on.

### `option`
[src/ui/krita_editor.rs:377](../../../src/ui/krita_editor.rs#L377)

```rust
fn option(ui: &mut egui::Ui, base: &Preset, cur: &Preset, o: &mut Vec<(String, String)>, spec: &OptionSpec, label: &str, strength: bool)
```

One curve option: on or off, strength, the sensors it reads, and the curve of the one picked.

### `option_body`
[src/ui/krita_editor.rs:412](../../../src/ui/krita_editor.rs#L412)

```rust
fn option_body(ui: &mut egui::Ui, d: &mut OptionData, spec: &OptionSpec, strength: bool)
```

The inside of one curve option: strength slider, "use same curve" and combine mode, a checkbox per sensor, and the
curve editor for the picked sensor.
