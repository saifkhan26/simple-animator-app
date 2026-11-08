# Getting started

How to build, run and test Animator, what every folder in the repository is for, and what
each dependency does — explained for someone coming from JavaScript.

## 1. Install Rust

Rust is installed with **rustup** (like `nvm` for Node): it installs the compiler (`rustc`) and
the build tool / package manager (`cargo`, like `npm`).

- Windows: download and run `rustup-init.exe` from <https://rustup.rs>. It may ask for the
  "Visual Studio C++ Build Tools" — install them; Rust uses Microsoft's linker on Windows.
- macOS / Linux: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

The project needs Rust **1.80 or newer** (`rust-version = "1.80"` in [Cargo.toml](../Cargo.toml)).
Check with:

```bash
rustc --version
```

Recommended editor setup: VS Code with the **rust-analyzer** extension (type hints, go to
definition, inline errors). Ctrl+click on any function name jumps to its definition — the same
gesture works on the links in these docs.

## 2. Build and run

| npm habit | cargo equivalent | What it does |
|---|---|---|
| `npm install` | *(automatic)* | dependencies download on first build |
| `npm run dev` | `cargo run` | debug build + start the app |
| `npm run build` | `cargo build --release` | optimised build |
| `npm test` | `cargo test` | run every test |
| `npm run lint` | `cargo clippy` | lints |
| `prettier --write` | `cargo fmt` | formats code |

From the repository root:

```bash
cargo run --release
```

The first build takes several minutes (it compiles every dependency); later builds only
recompile what changed. The executable lands in `target/release/animator-app.exe`
(`target/debug/` for debug builds). `target/` is like `node_modules` + `dist` together — safe to
delete, it gets rebuilt.

**Debug vs release.** The two "profiles" are configured at the bottom of
[Cargo.toml](../Cargo.toml):

- `[profile.dev]` (`cargo run`): `opt-level = 1` — a little optimisation so drawing isn't
  painfully slow, plus debug info and incremental compilation for fast rebuilds.
- `[profile.release]` (`--release`): maximum optimisation, `lto = "fat"` (optimise across all
  crates at link time), `codegen-units = 1`, symbols stripped, and `panic = "abort"` (a crash
  ends the process immediately instead of unwinding). Slow to build, fast to run.

In debug builds a console window shows log output. In release builds on Windows there is no
console (see `windows_subsystem` in [src/main.rs](../src/main.rs) and
[docs/src/main.md](src/main.md)).

**Logging.** The app uses the `log` + `env_logger` crates. By default it shows `info` messages
from the app and `warn` from libraries. Change it with the `RUST_LOG` environment variable, e.g.
in PowerShell:

```powershell
$env:RUST_LOG = "animator_app=debug"; cargo run
```

## 3. Run the tests

```bash
cargo test
```

Run only part of the suite by giving a filter — any test whose full path contains the text runs:

```bash
cargo test tools::fill
```

```bash
cargo test app::select
```

Things to know:

- Tests live at the bottom of each source file in `#[cfg(test)] mod tests { … }` blocks (see the
  [primer](rust-primer.md#tests)). Each file's doc page has a table of its tests.
- Some heavy tests are much faster with `cargo test --release`.
- Tests never read or write your real settings: the config path is `None` under test (see
  `config_path` in [src/input/shortcuts.rs](../src/input/shortcuts.rs#L1461)).
- `cargo test --features exact-rcp` turns on the `exact-rcp` feature, which makes the Krita
  brush golden tests produce identical hashes on every CPU
  ([docs/src/tools/golden_tests.md](src/tools/golden_tests.md)). Never use it for a real build.
- On a Windows machine with **App Control / Smart App Control**, `cargo test` can fail with
  `os error 4551`: Windows is blocking the freshly built test executable, not a code problem.

## 4. Lints

[clippy.toml](../clippy.toml) forbids two egui colour helpers
(`Color32::from_rgba_unmultiplied`, `Color32::from_white_alpha`) because egui 0.29 makes
translucent colours too bright with them; the project's own `ui::theme::premul` and
`ui::theme::white_alpha` are used instead (see [docs/src/ui/theme.md](src/ui/theme.md)).

```bash
cargo clippy --all-targets
```

## 5. Where the app keeps its files

| File | Location (Windows) | What it is |
|---|---|---|
| `shortcuts.toml` | `%APPDATA%\animator-app\shortcuts.toml` | key bindings, human-editable TOML |
| `ui.ron` | `%APPDATA%\animator-app\ui.ron` | panel layout + UI preferences (eframe persistence) |
| `*.anim` | wherever you save | your projects ([docs/src/io/project_file.md](src/io/project_file.md)) |

`%APPDATA%` is `C:\Users\<you>\AppData\Roaming`. On macOS/Linux the folder is the platform's
config directory (from the `dirs` crate). Deleting `ui.ron` resets the panel layout.

## 6. What is in the repository

```text
animator-app/
├── Cargo.toml          ← package.json: name, dependencies, build profiles, features
├── Cargo.lock          ← package-lock.json: exact dependency versions
├── clippy.toml         ← lint configuration
├── README.md           ← feature list and user guide
├── NOTICE              ← licence notices
├── docs/               ← these docs
├── krita-plugin/       ← a small Python plugin for Krita used by "Edit in Krita"
│   ├── animator_link/  ←   the plugin itself (extension.py, mailbox.py)
│   └── tests/          ←   its Python tests
├── src/                ← all Rust source code (one crate)
│   ├── main.rs         ← program entry point: declares modules, opens the window
│   ├── app.rs          ← AppState: the whole app's state and behaviour
│   ├── app/            ←   more AppState methods, split by feature (select, liquify, grids…)
│   ├── doc/            ← the document model: Project, Layer, Canvas (pixels), camera, groups
│   ├── tools/          ← drawing tools: strokes, fill, selection, perspective, Krita brush engine
│   ├── ui/             ← egui drawing code: window shell, timeline tracks, colour wheel, GPU canvas
│   ├── input/          ← keyboard shortcuts, pen tablet, mouse button drags, OS message hooks
│   ├── io/             ← loading and saving: .anim, .kra, PNG/GIF/WebP/MP4, imports
│   ├── timeline/       ← playback timing and onion skin
│   ├── brush_library/  ← stored brush presets and importing Krita presets
│   ├── color.rs        ← colour conversions (HSL, hex…)
│   ├── undo.rs         ← undo / redo history
│   ├── krita_link.rs   ← "Edit in Krita" round trip
│   ├── krita_helper.rs ← helpers for finding/launching Krita
│   ├── krita_stress.rs ← test-only stress harness for the Krita link
│   └── platform.rs     ← Windows-only window styling
└── target/             ← build output (generated, not committed)
```

Every `.rs` file has its own page under [docs/src/](src/) — the full list is in
[README.md](README.md#every-source-file).

## 7. Dependencies (crates) and what they are for

Rust packages are called **crates** and come from <https://crates.io> (the npm registry of
Rust). They are listed in `[dependencies]` in [Cargo.toml](../Cargo.toml). `features = [...]`
switches optional parts of a crate on; `default-features = false` turns its defaults off to keep
the binary small.

| Crate | Role in this app | Closest JS-world analogy |
|---|---|---|
| `eframe` | opens the native window, runs the frame loop, hosts egui on wgpu | Electron's app shell (but tiny) |
| `egui` | immediate-mode GUI: panels, buttons, sliders, painting | React + a canvas UI kit |
| `egui_extras` | SVG image support for icons | |
| `egui-phosphor` | the Phosphor icon font | an icon font package |
| `wgpu` (via eframe) | GPU API used for rendering | WebGPU |
| `winit` | window + OS events (named for a Windows message hook) | |
| `image` | decode/encode PNG, JPEG, BMP, WebP | `sharp` |
| `image-webp` | animated WebP container writing/reading | |
| `gif`, `color_quant` | animated GIF writing and palette reduction (NeuQuant) | `gif.js` |
| `flate2` | deflate compression for `.anim` payloads | `zlib` |
| `zip` | `.kra` files are zip archives | `jszip` |
| `roxmltree` | read XML inside `.kra` / `.kpp` | `DOMParser` |
| `rfd` | native open/save file dialogs | `<input type="file">` / Electron dialog |
| `arboard` | system clipboard (copy/paste images) | Clipboard API |
| `serde` | (de)serialisation framework, `#[derive(Serialize, Deserialize)]` | `JSON.stringify/parse` |
| `postcard` | compact binary serde format used in `.anim` | MessagePack |
| `toml`, `toml_edit` | read/write `shortcuts.toml`, keeping comments | `@iarna/toml` |
| `bytemuck` | safely view `&[u8]` as other plain types (and vice versa) | `new Uint32Array(buffer)` |
| `glam` | small vector/matrix maths library | `gl-matrix` |
| `pollster` | block on a GPU async call | `await` at top level |
| `anyhow` | easy error type for `Result` | `throw new Error(...)` |
| `log`, `env_logger` | logging macros + terminal output | `debug` / `pino` |
| `dirs`, `directories` | find the user's config folder | `os.homedir()` / `app.getPath` |
| `wintab_lite`, `libloading` *(Windows)* | pen-tablet driver (Wintab) loaded at run time | WebHID / PointerEvent |
| `windows` *(Windows)* | Microsoft's official Win32 API bindings | Node native addons |
| `raw-window-handle` *(Windows)* | get the OS window handle from winit | |

Lines under `[target.'cfg(target_os = "windows")'.dependencies]` are only compiled on Windows.

## 8. Where to go next

1. [architecture.md](architecture.md) — how the pieces connect, from `main()` to pixels on screen.
2. [walkthroughs.md](walkthroughs.md) — follow one action (drawing a line, saving, playing) through the code.
3. [docs/src/main.md](src/main.md) — the entry point, explained line by line.
4. [rust-primer.md](rust-primer.md) — whenever a Rust construct is unfamiliar.
