# `src/ui/krita_import.rs` — the Import Krita brushes window

> **Source:** [src/ui/krita_import.rs](../../../src/ui/krita_import.rs) · **Module:** `crate::ui::krita_import` · **Size:** 214 lines · Line links as of commit `36405ad`

## What this file is for

Draws the **Import Krita brushes** window: the presets found in the installed Krita (and any `.kpp`/`.bundle`
files you add), grouped by folder or bundle, each row with a thumbnail, its name and whether this app can paint
it. Paintable ones have an **Import** button (or **Use** once imported); a group has **Import all**. The scanning
itself happens in the background ([brush_library/import.rs](../brush_library/import.md)).

## How it connects

- **Used by:** [ui/shell.rs](shell.md) (draws it while `AppState::krita_import` is open).
- **Uses:** [brush_library/import.rs](../brush_library/import.md) (`KritaImport`, `Candidate`), [app.rs](../app.md)
  (`rescan_brushes`, `close_krita_import`), [ui/brush_picker.rs](brush_picker.md) (`pick`).

## Rust concepts in this file

**A deferred action enum.** Clicks are recorded as a `Do` value while drawing (when `state` is partly borrowed) and
carried out after the window is drawn.

## Types

### `const THUMB`
[src/ui/krita_import.rs:17](../../../src/ui/krita_import.rs#L17)

```rust
const THUMB: f32 = 28.0;
```

A row's thumbnail size.

### `enum Do`
[src/ui/krita_import.rs:20](../../../src/ui/krita_import.rs#L20)

What a click in the window asked for, done once the window is drawn.

| Variant | Meaning |
|---|---|
| `Import(usize)` | Import candidate `i`. |
| `ImportAll(usize)` | Import every paintable candidate of group `i`. |
| `Use(String, String)` | Switch the brush to an already-imported preset (library id, name). |
| `AddFiles` | Open a file dialog to add `.kpp`/`.bundle` files. |

## Free functions

### `krita_import_window`
[src/ui/krita_import.rs:27](../../../src/ui/krita_import.rs#L27)

```rust
pub fn krita_import_window(state: &mut AppState, ctx: &egui::Context)
```

Draws the window (title, status while scanning, the groups via `body`, Close), polls the background scan, and then
performs the recorded `Do` action.

### `importable`
[src/ui/krita_import.rs:84](../../../src/ui/krita_import.rs#L84)

```rust
fn importable(c: &Candidate) -> bool
```

Whether a candidate can be imported (the engine paints it and it isn't already in the library).

### `import`
[src/ui/krita_import.rs:89](../../../src/ui/krita_import.rs#L89)

```rust
fn import(state: &mut AppState, which: &[usize])
```

Import candidates `which`, then show the library what arrived.

### `body`
[src/ui/krita_import.rs:113](../../../src/ui/krita_import.rs#L113)

```rust
fn body(imp: &mut KritaImport, ui: &mut egui::Ui, shortfall: f32, act: &mut Option<Do>)
```

The scrollable list: each group with its header and Import all, then its visible candidate rows.

### `row`
[src/ui/krita_import.rs:179](../../../src/ui/krita_import.rs#L179)

```rust
fn row(ui: &mut egui::Ui, c: &Candidate, thumb: Option<&egui::TextureHandle>, i: usize, act: &mut Option<Do>)
```

One candidate's row: thumbnail, name, why it can't paint (if so), and an Import or Use button.
