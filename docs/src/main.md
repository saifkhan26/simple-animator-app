# `src/main.rs` — where the program starts

> **Source:** [src/main.rs](../../src/main.rs) · **Module:** `crate` · **Size:** 101 lines · Line links as of commit `67a7cb9`

## What this file is for

This is the **entry point**: when you run `animator-app.exe`, the operating system starts the
program at `fn main()` in this file — like `node index.js` running the top of `index.js`. It is
short and does three things:

1. **Declares the module tree.** Every `mod something;` line tells the compiler "there is a module
   called `something` — load it from `src/something.rs` (or `src/something/mod.rs`)". Files that are
   not reachable from these lines are not compiled at all. This is how the whole crate is assembled.
2. **Starts logging** (`env_logger`), so `log::info!` / `log::warn!` messages appear in the terminal.
3. **Opens the window and hands control to eframe** (`eframe::run_native`), passing a closure that
   builds the app's state, [`AppState`](app.md). From then on eframe runs the event loop and calls
   `AppState::update` every frame; `main` only returns when the window closes.

## How it connects

- **Uses:** [app.rs](app.md) (`AppState::new`), [ui/canvas_gpu.rs](ui/canvas_gpu.md) (its
  `DITHERING` constant must match the window settings), [input/shortcuts.rs](input/shortcuts.md)
  (`config_dir`, for where `ui.ron` is stored), [input/msg_hook.rs](input/msg_hook.md) (a hook on
  raw Windows messages).
- **Used by:** nothing — the OS calls `main`.

### The module tree

The `mod` lines here (and inside each `mod.rs`) build this tree. Each box links to its docs.

```text
crate (src/main.rs)
├── app            src/app.rs            AppState — the whole app's state + behaviour
│   └── brush_sizes, button_drag, grids, groups, lasso_fill, liquify, orient, select   (src/app/*.rs)
├── brush_library  src/brush_library/    stored brush presets, Krita preset import
├── color          src/color.rs          colour maths (HSL, hex)
├── doc            src/doc/              document model: Project, Layer, Canvas, camera, groups…
├── input          src/input/            shortcuts, pen tablet, button drags, OS hooks
├── io             src/io/               load/save: .anim, .kra, PNG/GIF/WebP/MP4, imports
│   └── kra        src/io/kra/           Krita .kra reading/writing
├── krita_helper   src/krita_helper.rs   finding / launching Krita
├── krita_link     src/krita_link.rs     "Edit in Krita" round trip
├── krita_stress   src/krita_stress.rs   (tests only) stress harness for the Krita link
├── platform       src/platform.rs       (Windows only) window corners/border
├── timeline       src/timeline/         playback clock, onion skin
├── tools          src/tools/            drawing tools, fill, selection, perspective
│   └── krita      src/tools/krita/      port of Krita's pixel brush engine
│       └── tip    src/tools/krita/tip/  brush tip shapes
├── ui             src/ui/               egui UI: shell, timeline tracks, theme, GPU canvas…
└── undo           src/undo.rs           undo / redo history
```

Folder index pages: [app](app.md) · [brush_library](brush_library/mod.md) · [doc](doc/mod.md) ·
[input](input/mod.md) · [io](io/mod.md) · [timeline](timeline/mod.md) · [tools](tools/mod.md) ·
[tools/krita](tools/krita/mod.md) · [ui](ui/mod.md).

## Rust concepts in this file

**`mod` declarations build the program.** In JS each file imports what it needs and the bundler
follows imports. In Rust the *parent* declares its children; `use` only creates shortcuts to names.
```rust
mod app;            // compile src/app.rs as crate::app
use app::AppState;  // shortcut: write `AppState` instead of `app::AppState`
```
More: [Modules and visibility](../rust-primer.md#modules-and-visibility)

**Conditional compilation.** `#[cfg(test)] mod krita_stress;` compiles that module only for
`cargo test`; `#[cfg(target_os = "windows")] mod platform;` only on Windows. The file-wide
`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` (note the `!`: it applies to the
whole crate) says "in release builds, mark the program as a GUI app" so Windows doesn't open a console
window next to it. More: [Attributes and cfg](../rust-primer.md#attributes-and-cfg)

**Closures and `Box`.** eframe needs "a function that creates the app", which it calls once the
window exists:
```rust
Box::new(|cc| Ok(Box::new(AppState::new(cc))))
```
Read from inside out: `AppState::new(cc)` builds the state; `Box::new(...)` puts it on the heap as a
`Box<dyn eframe::App>` (eframe only knows it is "some type implementing the `App` trait");
`Ok(...)` wraps it in a `Result` meaning "creation succeeded"; `|cc| ...` is the closure taking the
creation context; the outer `Box::new` boxes the closure itself. The JS shape is roughly
`runNative("Animator", options, (cc) => new AppState(cc))`.
More: [Closures](../rust-primer.md#closures), [Smart pointers](../rust-primer.md#smart-pointers)

**Struct update syntax.** `eframe::NativeOptions { renderer: …, multisampling: 0, ..Default::default() }`
sets the listed fields and takes every other field from the default options — like
`{ ...defaults, renderer, multisampling: 0 }` in JS.
More: [Structs](../rust-primer.md#structs)

**`Option::map`.** `config_dir().map(|d| d.join("ui.ron"))`: if there is a config folder, turn it into
the path of `ui.ron`; if not, stay `None` (then eframe doesn't persist state).
More: [Option and Result](../rust-primer.md#option-and-result)

## What happens next

`eframe::run_native` opens the window, sets up the GPU (wgpu) and egui, and calls the closure →
[`AppState::new`](app.md#appstatenew) builds the state (loads preferences, shortcuts, an empty
1280×720 project). Then, every frame, eframe calls [`AppState::update`](app.md#appstateupdate), which
handles input, playback and background jobs and finally draws the UI with
[`ui::shell::draw`](ui/shell.md). See [architecture.md](../architecture.md) for the full picture.

## Free functions

### `main`
[src/main.rs:28](../../src/main.rs#L28)

```rust
fn main() -> eframe::Result<()>
```

The program's first function. It:
1. Initialises logging with the default filter `animator_app=info,warn` — `info` and above from this
   app, only warnings from libraries — which the `RUST_LOG` environment variable can override.
2. Calls `eframe::run_native("Animator", native_options(), Box::new(|cc| Ok(Box::new(AppState::new(cc)))))`:
   window title, window/renderer options, and the closure that creates the app once the window
   exists.

It returns `eframe::Result<()>` — `Ok(())` when the window closes normally, or an error if, say, no
graphics driver could be initialised. Returning a `Result` from `main` makes Rust print the error and
exit with a failure code.

### `native_options`
[src/main.rs:41](../../src/main.rs#L41)

```rust
fn native_options() -> eframe::NativeOptions
```

Builds the window and renderer settings passed to eframe:

| Setting | Value | Why |
|---|---|---|
| `renderer` | `Wgpu` | GPU rendering through wgpu (the WebGPU API), which [ui/canvas_gpu.rs](ui/canvas_gpu.md) draws into |
| `present_mode` | `Mailbox` | show the newest frame without waiting for vsync to tear |
| `multisampling`, `depth_buffer` | `0` | the custom canvas pipeline is built for no MSAA and no depth buffer — they must match |
| `dithering` | `canvas_gpu::DITHERING` | same reason |
| `persist_window` | `false` | always open maximised instead of restoring the last window rectangle |
| `persistence_path` | `<config dir>/ui.ron` | where egui panel layout and `UiPrefs` are saved |
| `with_decorations(false)` | — | frameless window (the app draws its own title bar) |
| `with_transparent(true)` | — | so lowering the backdrop opacity shows the desktop through |
| `with_maximized(true)`, `with_inner_size`, `with_min_inner_size` | — | start maximised; 1280×800 when restored; never below 640×480 |
| `event_loop_builder` | `input::msg_hook::install` | hook raw Windows messages (Ctrl+C/X/V, tablet driver notices) |

The comments in the source explain that `with_maximized` alone doesn't stick on a frameless window,
so `AppState::update` also sends a maximise command on the first frame.

## Tests

These tests compile only under `cargo test`. The one test here checks that the window options match
what the custom GPU canvas pipeline was built for — if they didn't, wgpu would reject the first draw
(a silent crash in release builds).

### `mod tests`

Run with `cargo test tests` · 1 tests

| Test | What it checks |
|---|---|
| [`the_window_is_what_the_canvas_pipeline_is_built_for`](../../src/main.rs#L93) | `ui::canvas_gpu` draws inside egui's render pass, so its pipeline has to match the pass eframe opens. A pipeline that doesn't is a wgpu validation error at the first draw — a silent abort in release — so changing any of these means changing `canvas_gpu` first. |
