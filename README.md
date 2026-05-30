# Animator

Lightweight cross-platform raster animation app for cartoon / anime workflow.
Pure Rust (egui + wgpu + winit via eframe). No Electron, no Tauri webview, no
mocked browser DOM. Single executable under ~15 MB after release build.

## Why this exists

Built for frame-by-frame animators who want Krita / CSP / TVPaint mechanics
without the launch time or memory footprint. Designed to feel snappy on
modest hardware: dirty-rect uploads, lazy cell allocation, no background
threads except export.

## Features

- **Transparent or opaque window background**, slider-controlled at runtime.
  Drop the slider to 0 to see your desktop through unpainted areas — useful
  for tracing / rotoscoping.
- **Layers** with opacity, visibility, lock, and a *Reference* (light-table)
  flag that dims the layer and excludes it from export.
- **Onion skin** with separately tinted previous / next frames, configurable
  count, falloff, and max alpha.
- **X-sheet (exposure sheet)** — frames × layers grid. Click any slot to
  navigate. *Insert key* duplicates the resolved cell so you can break a hold;
  *Hold* deletes a key so the previous one persists ("on 2s/3s" workflow).
- **Tools** — Pencil, Ink, Eraser, Flood Fill. Each tool ships with a
  default pressure curve and brush settings.
- **Tablet pressure** on Windows via Wintab (Wacom, Huion, XP-Pen, Gaomon, …)
  — auto-detected. Falls back to mouse (constant pressure) when no driver
  found.
- **Undo / Redo** with bounded history (80 entries), stored as dirty-rect
  pixel snapshots so memory stays small.
- **Export** to PNG sequence, animated GIF (NeuQuant palette), or single PNG
  of the current flattened frame.
- **Floating, movable, collapsible panels** — Tools, Brush, Timeline, Onion
  skin, Layers, X-sheet. Drag titlebars to rearrange.

## Build & run

```bash
git clone <repo>
cd animator-app
cargo run --release
```

Requirements: Rust 1.80+. On Windows you also need a tablet driver if you
want pressure (the app still runs without one). On Linux you need a Vulkan
or OpenGL driver and a Wayland or compositing X11 session for true window
transparency.

Release binary lands at `target/release/animator-app(.exe)`.

## UI tour

```
┌──────────────────────────────────────────────────────────────────────┐
│ Edit  File   Animator   ··· (drag here to move)   — 🗖 ✕            │  ← custom title bar
├──────────────────────────────────────────────────────────────────────┤
│ ┌─────────────┐                                ┌──────────────────┐  │
│ │  Tools      │                                │  Layers          │  │
│ │  Pencil…    │                                │  + − ▲ ▼         │  │
│ │  Size  Flow │            CANVAS              │  ☑ Layer 2  α    │  │
│ │  Bg α       │                                │  ☑ Layer 1  α    │  │
│ │  ☐ Checker  │                                └──────────────────┘  │
│ ├─────────────┤                                                      │
│ │  Brush      │                                ┌──────────────────┐  │
│ │  Color      │                                │  X-sheet         │  │
│ │  Hardness…  │                                │  Fr  L1  L2      │  │
│ └─────────────┘                                │  ▶0   0   ·      │  │
│                                                │   1   ·   2      │  │
│        ┌────────────────────────────────┐      └──────────────────┘  │
│        │  Timeline   ▶ ⏮ ◀ ▶  +Frame…   │                            │
│        │  [████····················]    │                            │
│        └────────────────────────────────┘                            │
└──────────────────────────────────────────────────────────────────────┘
```

The title bar has no OS chrome because the OS only gives us a transparent
surface for borderless windows. Drag the dotted strip in the centre to move
the window; double-click it to toggle maximize.

## Drawing

1. Pick a tool in the **Tools** window.
2. Set color in the **Brush** window.
3. Drag on the canvas to draw on the currently-selected layer's currently-
   visible cell.

If the current (layer, frame) slot is empty, the first stroke allocates a
new cell and keys it there automatically.

### Tool reference

| Tool   | Behaviour                                                        |
|--------|------------------------------------------------------------------|
| Pencil | Soft round stamp. Pressure → size + flow.                        |
| Ink    | Harder, denser stamps for clean-up lines.                        |
| Eraser | Punches alpha to 0 (no white).                                   |
| Fill   | Scanline flood within tolerance bound. Click once to fill.       |

### Brush parameters

- **Size** — radius at pressure = 1.
- **Flow / Opacity** — per-stamp alpha at pressure = 1.
- **Hardness** — edge falloff exponent (1 = soft, 8 = hard).
- **Spacing** — distance between stamps as fraction of radius
  (smaller = denser).
- **Pres → size** — how strongly pressure scales radius (0 = constant).
- **Pres → flow** — how strongly pressure scales flow.
- **Tolerance** (Fill only) — per-channel match window in 0..=255.

## Frames, layers, X-sheet

### Timeline window

- **▶ / ❚❚** — play / pause the loop range.
- **⏮ ◀ ▶** — jump to loop start / step prev / step next.
- **+ Frame** — insert an empty exposure after the current frame on every
  layer (effectively a hold).
- **Duplicate** — insert a copy of each layer's resolved cell at the new
  position (cells are duplicated cell-by-cell; modifying the copy doesn't
  affect the original).
- **Delete** — remove the current frame.
- **fps slider** — set playback rate (also drives GIF export delay).
- **Frame strip** — click to seek. Blue = current. Dark = inside loop range.

### Layers window

- `+` / `−` — add / remove layer.
- `▲` / `▼` — reorder.
- **α** — layer opacity in composite.
- **🔒** — lock (refuses strokes).
- **Ref** — light-table reference layer. Renders dimmed across all frames;
  excluded from PNG / GIF export.

Layers paint bottom-up. The list shows them top-down (top of list = on top).

### X-sheet window

Grid view: rows = frames, columns = layers. Each cell shows the **CellId**
keyed at that slot, or `·` for a hold.

- Click any cell → moves the editing cursor to (frame, layer).
- **+ Key (blank)** — insert an empty key at the active slot. Breaks any
  hold and starts with a blank cell. Use when you want to draw fresh.
- **+ Key (copy)** — insert a key cloned from the currently-resolved cell.
  Breaks the hold but keeps the existing drawing as a starting point for
  tweaks (anime "modify slightly" workflow).
- **Hold** — delete the active slot's key so the previous cell persists.

## Onion skin

Open the **Onion skin** window:

- **Enabled** — master toggle.
- **Prev / Next** — how many frames in each direction (0..=8).
- **Max α** — alpha of the nearest ghost frame.
- **Falloff** — exponent on the distance-to-current weight.
- **Prev tint / Next tint** — multiplicative tints. Default: blue past, red
  future, which matches Krita / TVPaint convention.

Onion skin only applies to the *active layer*. Other layers stay solid.

## Tablet pressure (Windows)

If `Wintab32.dll` is present (Wacom / Huion / XP-Pen / Gaomon drivers all
install one), the app polls pressure each frame and feeds it into the brush.
On startup you'll see `Wintab opened: pressure_max = …` in the log.

No tablet driver installed → no error, just no pressure. Mouse keeps working
with constant pressure = 1.

Linux / macOS tablet support is not wired in yet (see Roadmap).

## Window background

The **Bg α** slider in the Tools window drives the window clear-color
alpha. Drop it to 0 to make the entire app background transparent. The
floating windows stay semi-translucent so they remain readable.

Toggle **Checker backdrop** to render a Krita-style checker behind the
canvas image — useful when working in transparent mode.

## Export

| Action                            | Where                                |
|-----------------------------------|--------------------------------------|
| Save current frame as PNG         | Edit-bar **File → Save PNG…**        |
| Export every frame as PNG sequence| **File → Export PNG sequence…**      |
| Export animated GIF               | **File → Export animated GIF…**      |

Export flattens visible non-reference layers per frame. The X-sheet's
holds are resolved, so a single keyed cell held over 3 frames produces 3
identical PNGs (or GIF frames) — matching what playback shows.

## Keyboard shortcuts

| Shortcut             | Action     |
|----------------------|------------|
| Ctrl+Z               | Undo       |
| Ctrl+Y / Ctrl+Shift+Z| Redo       |

(Tool / frame shortcuts are not yet bound — coming soon.)

## Files

The app does not yet have a native save format. Use **File → Export PNG
sequence** for now. A binary `.anim` format is planned for the next
milestone.

## Roadmap

Done:
- A — boot, mouse stroke, PNG save
- B — timeline, playback, onion skin
- C — layers, X-sheet, light table
- D — flood fill, Windows tablet pressure
- E — undo / redo, GIF + PNG-sequence export, transparent window

Next:
- F — `.ora` (Krita) read support
- GPU compute brush stamping (replace CPU stamp loop for big brushes)
- Native `.anim` save format
- Linux / macOS tablet backends (`libinput`, NSEvent)
- Keyboard shortcuts for tool swap and frame nav
- Brush presets save / load

## License

MIT OR Apache-2.0.
# simple-animator-app
