# `src/ui/mod.rs` — the ui folder

> **Source:** [src/ui/mod.rs](../../../src/ui/mod.rs) · **Module:** `crate::ui` · **Size:** 14 lines · Line links as of commit `1fa5845`

## What this file is for

`src/ui/mod.rs` declares the modules of the **user interface** folder — all the egui code that draws the
window. It contains nothing else.

### Map of the ui folder

| Page | What it draws or does |
|---|---|
| [shell.md](shell.md) | **The whole window**: top bar and menus, every floating panel (Tools, Brush, Layers, Onion, Camera, X-sheet, Timeline), dialogs, the canvas and everything over it, canvas pen/mouse input. Entry point `ui::shell::draw(state, ctx)`, called every frame from `AppState::update`. |
| [tracks.md](tracks.md) | The Timeline panel's layer tracks: drawings as bars you can select, drag, retime, clear. |
| [canvas_gpu.md](canvas_gpu.md) | A custom GPU (wgpu) pipeline that draws the drawings' textures on the canvas with correct gamma. |
| [theme.md](theme.md) | Colour schemes (Graphite, Midnight, Paper, Custom), fonts, icon buttons. |
| [color_wheel.md](color_wheel.md) | The HSL colour wheel and harmony swatches. |
| [curve_editor.md](curve_editor.md) | The little curve editor for pressure/sensor curves. |
| [expr.md](expr.md) | Typing arithmetic into number fields (`12*2`). |
| [widgets.md](widgets.md) | Custom widgets egui doesn't have (a two-knob range slider). |
| [brush_picker.md](brush_picker.md) | The brush preset picker grid. |
| [krita_editor.md](krita_editor.md) | The Krita brush settings editor. |
| [krita_import.md](krita_import.md) | The "Import Krita brushes" window. |
| [sweep_tests.md](sweep_tests.md) | Test-only: a headless sweep over every window looking for layout bugs (declared inside `shell.rs`). |

### How immediate-mode UI shapes this folder

There are no component objects or event listeners. Every frame, `shell::draw` runs top to bottom, calling
functions that both draw widgets and read what the user did, then change `AppState` directly. See
[egui and immediate mode UI](../../rust-primer.md#egui-and-immediate-mode-ui).

## How it connects

- **Used by:** [main.rs](../main.md) (`mod ui;`), [app.rs](../app.md) (calls `ui::shell::draw`, uses
  `ui::canvas_gpu`, `ui::theme`).
