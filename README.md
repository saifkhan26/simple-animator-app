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
- **Onion skin** drawn as colored silhouettes — blue past, red future — that
  step by *drawing* rather than by frame, so holds don't waste ghosts.
- **X-sheet (exposure sheet)** — frames × layers grid. Click any slot to
  navigate. *Insert key* duplicates the resolved cell so you can break a hold;
  *Hold* deletes a key so the previous one persists ("on 2s/3s" workflow).
- **Tools** — Pencil, Ink, Eraser, Flood Fill, Shape, Lasso select, Tracker.
  Each tool ships with a default pressure curve and brush settings.
- **Lasso selection** — move, cut, copy and paste a region of pixels between
  frames and layers.
- **Flip the view** horizontally or vertically as a drawing check. Never
  touches the document.
- **Pinned colour swatches**, remembered across runs.
- **Tablet pressure** on Windows via Wintab (Wacom, Huion, XP-Pen, Gaomon, …)
  — auto-detected. Falls back to mouse (constant pressure) when no driver
  found.
- **Undo / Redo** with bounded history (80 entries), stored as dirty-rect
  pixel snapshots so memory stays small.
- **Export** to PNG sequence, animated GIF (NeuQuant palette), MP4, sprite
  sheet, or a single PNG of the current flattened frame — each over a chosen
  frame range.
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

## Flipping the view

`F` mirrors the canvas horizontally, `Shift+F` vertically — the standard check
for a pose that has gone lopsided. A chip in the top-left corner says `FLIPPED`
while it is on, and *Reset rotation* (`J`) clears both flips along with the
roll.

This is a view transform only. Strokes land under the cursor as usual, and
exports are never mirrored.

## Lasso selection

Draw a loop with the Lasso tool (`Y`) to select; the path closes itself on
release. Then:

- **Drag inside it** to move the pixels, or nudge a pixel at a time with the
  arrow keys. Moves are pixel-snapped, so a move alone never resamples.
- **Drag a handle** on the box around the selection to scale it — corners take
  both axes, edges take one, and `Shift` keeps it uniform. **Drag just outside
  a corner** to rotate, with `Shift` snapping to 15°.
- The lifted pixels are only resampled once, when the selection commits.
  Scaling out and back, or rotating twice, is no softer than doing it once,
  because every pose is resolved from the pixels as they were lifted. A
  selection left square and on whole pixels still takes the exact blit.
- **`Delete`** erases the selected pixels.
- **`Ctrl+X` / `Ctrl+C` / `Ctrl+V`** cut, copy and paste — a paste lands on
  whatever cell is active, so it crosses frames and layers.
- **`Ctrl+D`** drops the selection in place.

Changing frame, changing layer, switching tools or starting a new lasso all
commit a floating selection first. Lifting and committing are two undo steps:
one undo puts the pixels back where you picked them up, a second restores the
hole.

Pasting an image from the system clipboard as a new layer moved to
`Ctrl+Shift+V`, since `Ctrl+V` now pastes a selection. An existing
`shortcuts.toml` is migrated automatically.

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
- **Frame number** — drag to scrub, or click and type. It takes arithmetic:
  `22/2`, `8+12`, `(4+8)*2`. Starting with an operator is relative to the
  current frame, which is the quick form since clicking selects what's already
  there: `+12` jumps twelve ahead, `-3` back, `/2` to the halfway frame. Enter
  applies it; anything that isn't a valid expression leaves the frame alone.
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
- **Cut / Copy / Paste drawing** (`Alt+X` / `Alt+C` / `Alt+V`) — move one
  drawing between frames or layers. Cut leaves the frame blank; paste keys the
  drawing at the active slot, re-centring it if that layer's cells are a
  different size. Other layers and the frame count are never touched, so
  cut-then-paste is how a drawing gets retimed.

## Colour swatches

The Brush panel keeps a strip of pinned colours under the picker: `+` pins the
current colour, a click selects one, right-click removes it. Up to 32, saved
with your preferences, so a palette follows you between projects.

## Onion skin

Open the **Onion skin** window:

- **Enabled** — master toggle (`O`).
- **Step by drawings** — count distinct drawings instead of frames, so on 2s
  and 3s `Prev = 2` reaches the two previous *drawings* rather than two frames
  of the same held one. Off: literal frame stepping, which simply shows fewer
  ghosts across a hold.
- **Prev / Next** — how many in each direction (0..=8).
- **Max α** — alpha of the nearest ghost.
- **Falloff** — exponent on the distance-to-current weight. The farthest ghost
  keeps a floor of the max alpha, so it never fades to nothing.
- **Prev tint / Next tint** — silhouette colors. Default: blue past, red
  future, which matches Krita / TVPaint convention.

Ghosts are drawn from silhouette textures baked in the tint color, not by
multiplying a tint over the artwork — a multiply leaves black line art black,
which is what made ghosts read as a grey smudge.

A drawing already showing on the current frame is never ghosted (on a hold it
would land exactly on top), and ghosts are hidden during playback.

Onion skin only applies to the *active layer*. Other layers stay solid.
Settings persist across runs and survive **File → New**.

## Auto-key

Two opt-in toggles, both off by default and both remembered across runs:

- **Auto-key transform** (Layers panel) — moving, scaling or rotating the
  active layer writes a transform key on the current frame instead of shifting
  the layer on every frame. Drag and key land in one undo entry.
- **Auto-key drawing** (X-sheet panel) — drawing on a held frame starts a new
  *blank* key on that frame first, so you can draw frame after frame without
  inserting keys by hand. The previous drawing stays visible through onion
  skin. Undoing such a stroke takes two undos: the stroke, then the key.

## New project

**File → New** asks for width, height and fps.

- **Landscape / Portrait** — the orientation is read from the numbers
  themselves (a square canvas counts as landscape), and switching swaps them.
- **Presets** — `HD 720p` through `8K`, applied in whichever orientation is
  selected, so picking one doesn't undo a portrait choice.

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

| Action                     | Where                            |
|----------------------------|----------------------------------|
| Current frame as PNG       | **File → Save PNG…**             |
| PNG sequence               | **File → Export PNG sequence…**  |
| Animated GIF               | **File → Export animated GIF…**  |
| MP4 (needs ffmpeg on PATH) | **File → Export MP4…**           |
| Sprite sheet               | **File → Export sprite sheet…**  |

Every export except the single PNG opens one dialog with a **frame range**
(dual-knob slider, exact inputs, and a *Use loop range* button that fills it
from the loop bars on the frame strip), plus the options that format needs —
CRF and preset for MP4, columns and padding for the sheet. It reports the frame
count, and for a sheet the pixel dimensions, before you commit to it.

PNG-sequence filenames keep the absolute frame index, so a range starting at 10
writes `frame_0010.png` — a partial re-export still lines up with files from an
earlier full one.

Sprite-sheet cells are laid out row-major at the project frame size; `columns:
0` picks a near-square grid, and padding is a transparent gutter *between*
cells only, so the cell stride stays a round number.

Exports run on a worker thread with a progress overlay, and report success as a
toast and failure in a dialog.

Export flattens visible non-reference layers per frame. The X-sheet's holds are
resolved, so a single keyed cell held over 3 frames produces 3 identical PNGs
(or GIF frames) — matching what playback shows. A flipped view is never
exported.

## Keyboard shortcuts

Every action is bound and rebindable in **Settings → Shortcuts**, which also
lists the current binding for each one. A few of the defaults:

| Shortcut                  | Action                        |
|---------------------------|-------------------------------|
| Q / W / E / R / G / Y     | Pencil / Ink / Eraser / Fill / Shape / Lasso |
| A / S                     | Previous / next frame         |
| Space                     | Play / pause                  |
| O                         | Toggle onion skin             |
| F / Shift+F               | Flip view H / V               |
| Ctrl+X / C / V            | Selection cut / copy / paste  |
| Alt+X / C / V             | Drawing cut / copy / paste    |
| Ctrl+Z / Ctrl+Y           | Undo / redo                   |
| Ctrl+S / Ctrl+Shift+S     | Save / Save As                |
| Tab                       | Hide / show panels            |

## Files

Projects save as a binary `.anim` file (**File → Save**, `Ctrl+S`) — postcard
over a magic + version header, currently version 5, with migrations for every
older version the format has had.

## Roadmap

Done:
- A — boot, mouse stroke, PNG save
- B — timeline, playback, onion skin
- C — layers, X-sheet, light table
- D — flood fill, Windows tablet pressure
- E — undo / redo, GIF + PNG-sequence export, transparent window

Next:
- Layer blend modes (multiply / screen / add)
- `.ora` (Krita) read support
- GPU compute brush stamping (replace CPU stamp loop for big brushes)
- Linux / macOS tablet backends (`libinput`, NSEvent)
- Brush presets save / load
- Audio track for lip sync and timing

## License

MIT OR Apache-2.0.
# simple-animator-app
