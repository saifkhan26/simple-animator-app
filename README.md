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
- **Krita's Pencil-5 Tilted**, the real preset on a port of Krita's pixel
  brush: its bar tip turns with the pen's lean and widens as it tilts,
  pressure sets how dark, over Krita's own paper texture, built up in 8 bits
  the way Krita paints. *Brush settings → Preset → Pencil-5*.
- **Selections, Krita-style** — lasso, box, ellipse or polygon; add to,
  remove from or intersect them; every stroke and fill stays inside. Move,
  cut, copy and paste the selected pixels between frames and layers.
- **Flip the view** horizontally or vertically as a drawing check. Never
  touches the document.
- **Pinned colour swatches**, remembered across runs.
- **Colour wheel with harmonies** — an HSL wheel in the Color panel that lays
  Complementary, Monochromatic, Analogous, Triadic or Tetradic colours around
  the one you pick. Every colour in the app reads and pastes as `hsl(…)`.
- **Tablet pressure** on Windows via Wintab (Wacom, Huion, XP-Pen, Gaomon, …)
  — auto-detected. Falls back to mouse (constant pressure) when no driver
  found.
- **Undo / Redo** with bounded history (80 entries), stored as dirty-rect
  pixel snapshots so memory stays small.
- **Export** to PNG sequence, animated GIF (NeuQuant palette), MP4, sprite
  sheet, or a single PNG of the current flattened frame — each over a chosen
  frame range.
- **Edit in Krita** — opens the whole animation in Krita and brings each
  Krita save back as one undo step, changing only what Krita changed. Plain
  `.kra` import and export too.
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
2. Set color in the **Brush** window, or on the wheel in the **Color** window.
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

## Selection

The Lasso tool (`Y`) makes a selection that works like Krita's: it stays up
until you deselect, and **every stroke, erase, shape and fill stays inside
it** — on any tool, any layer and any frame. It belongs to the canvas, not to
a drawing, so you can scrub to the next frame or switch to the colour layer
and keep painting inside the same shape. On a moved, scaled or rotated layer
it clips where the marching ants are on screen.

**Making one.** Pick a shape in the Lasso panel: freehand, rectangle, ellipse
or polygon. A polygon takes a corner per click; `Enter`, a double-click or a
click back on the first corner closes it, `Backspace` takes back the last
corner and `Esc` abandons it.

**Adding and removing.** Hold a modifier as the drag starts, or set the mode
with the Lasso panel's buttons:

| Held while dragging | Mode |
|---|---|
| (nothing) | Replace |
| `Ctrl+Shift` | Add to the selection |
| `Ctrl+Alt` | Remove from the selection |
| `Shift+Alt` | Intersect with the selection |

Shift and Alt on their own still pan and rotate the canvas. All three are
rebindable in **Settings → Shortcuts**, and a selection mode bound to the same
keys as a canvas gesture wins while the Lasso is active. On Windows with more
than one keyboard layout installed, left `Alt+Shift` (and optionally
`Ctrl+Shift`) may also switch layout when released; rebind them if that gets
in the way.

A plain click outside the selection deselects.

**Moving pixels.** Drag inside the selection to move what's under it, or
nudge with the arrow keys. Drag a handle on the box around it to scale:
corners take both axes, edges take one, and `Shift` keeps it uniform. Drag
just outside a corner to rotate, with `Shift` snapping to 15°.

- Moves are pixel-snapped, so a move alone never resamples. A scaled or
  rotated selection is resampled once, when it lands, from the pixels as they
  were lifted — scaling out and back is no softer than doing it once.
- `Enter` puts the pixels down and keeps the selection where they landed.
  Changing frame, layer or tool, or starting a stroke, puts them down too.
- One `Ctrl+Z` while pixels are floating puts them back where they were
  picked up. Once they have landed, the move — pixels and selection
  together — is a single undo step.

**Commands.**

| Shortcut | Action |
|---|---|
| `Ctrl+A` | Select all |
| `Ctrl+Shift+I` | Invert the selection |
| `Esc` / `Ctrl+D` | Deselect |
| `Delete` | Erase inside the selection (the selection stays) |
| `Backspace` | Clear the drawing — only inside the selection when there is one |
| `Shift+Backspace` | Fill the selection with the brush colour |
| `Ctrl+X` / `Ctrl+C` / `Ctrl+V` | Cut, copy and paste — a paste lands on whatever cell is active, so it crosses frames and layers |

The Lasso panel also has **Grow**, **Shrink** and **Feather** by a number of
pixels, and **Tint outside selection**, which dims everything that isn't
selected. Every selection change is an undo step. The selection isn't saved
with the project.

While painting, the short live tip of a stroke and a shape being dragged are
previews and can show past the selection edge; what lands in the drawing is
always clipped.

Pasting an image from the system clipboard as a new layer is
`Ctrl+Shift+V`, since `Ctrl+V` pastes a selection.

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

## Colour wheel

The **Color** panel is an HSL wheel: hue goes round the disc, saturation runs
out from the centre, and the bar beside it is lightness. Pick a scheme and the
wheel shows every colour of the set as a dot:

- **Complementary** — the opposite hue.
- **Monochromatic** — five lightnesses of one hue. They share a dot on the
  disc; their ticks on the lightness bar set the step.
- **Analogous** — the neighbours either side.
- **Triadic** — three hues a third of the wheel apart.
- **Tetradic** — two complementary pairs, a rectangle on the wheel.

Click a swatch under the wheel to paint with it; the set stays where it is.
Drag the base dot (or press bare disc) to move the whole set; drag a side dot
of Analogous or Tetradic to widen or narrow it. H / S / L fields set the
current colour exactly, and **Pin set** adds the whole set to your swatches.
The set rebuilds around the brush colour whenever it changes elsewhere
(eyedropper, a swatch, Paste). The scheme is remembered; the set is not.

Every colour button in the app opens the same wheel. Colours show as
`hsl(h, s%, l%)`, and **Paste** takes `hsl(…)`, `rgb(…)` or `#hex`.

## Onion skin

Open the **Onion skin** window:

- **Enabled** — master toggle (`O`).
- **Step by drawings** — count distinct drawings instead of frames, so on 2s
  and 3s `Prev = 2` reaches the two previous *drawings* rather than two frames
  of the same held one. Off: literal frame stepping, which simply shows fewer
  ghosts across a hold.
- **Prev / Next** — how many in each direction (0..=8).
- **Offset chips** — one chip per ghost under the sliders (`−2 −1 • +1 +2`).
  Click one to hide just that offset: hiding −1 leaves −2 where it was, at
  its own fade.
- **Max α** — alpha of the nearest ghost.
- **Falloff** — exponent on the distance-to-current weight. The farthest ghost
  keeps a floor of the max alpha, so it never fades to nothing.
- **Prev tint / Next tint** — silhouette colors. Default: blue past, red
  future, which matches Krita / TVPaint convention.

Ghosts are drawn from silhouette textures baked in the tint color, not by
multiplying a tint over the artwork — a multiply leaves black line art black,
which is what made ghosts read as a grey smudge.

**Pinned frames** keep one particular frame of the active layer on screen as
a ghost wherever the playhead is — a key pose to check the in-betweens
against. Type a frame number and **+**, or pin the current frame. Each pin
has its own color, a show/hide toggle, and a delete button. Pins follow their
drawing when frames are inserted or deleted before them, show only while
their layer is active, and are not saved with the project.

All ghosts draw under the current drawing, nearest on top, so they never
cover the lines being drawn. A drawing already showing on the current frame
is never ghosted (on a hold it would land exactly on top), and ghosts are
hidden during playback.

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

A **Playback** section shapes how the range plays:

- **Ping-pong** (MP4, PNG sequence, GIF) — forward, then back, without
  doubling the end frames, so the wrap is seamless.
- **Loop to N s** (MP4, PNG sequence) — repeats the range to fill that many
  seconds, rounded *up* to whole loops so the file ends on a loop boundary and
  a player that repeats it repeats it seamlessly. 21 frames at 24 fps looped
  to 10 s gives 12 loops, 10.5 s; the dialog shows the real length before you
  export. GIFs repeat forever on their own, so they don't need it.

These last until you quit, like the other export settings.

PNG-sequence filenames keep the absolute frame index, so a range starting at 10
writes `frame_0010.png` — a partial re-export still lines up with files from an
earlier full one. With a loop or ping-pong a frame appears more than once, so
the files are numbered in playing order from `frame_0000.png` instead (repeats
are file copies, not re-renders).

Sprite-sheet cells are laid out row-major at the project frame size; `columns:
0` picks a near-square grid, and padding is a transparent gutter *between*
cells only, so the cell stride stays a round number.

Exports run on a worker thread with a progress overlay, and report success as a
toast and failure in a dialog.

Export flattens visible non-reference layers per frame. The X-sheet's holds are
resolved, so a single keyed cell held over 3 frames produces 3 identical PNGs
(or GIF frames) — matching what playback shows. A flipped view is never
exported.

## Krita

**File → Edit in Krita** (or right-click a layer name → *Edit in Krita*, which
opens with that layer selected) writes the project as an animated `.kra` to a
temp folder and opens it in Krita. Keys, holds, layer names, opacity,
visibility and lock all travel; a drawing reused at several frames arrives as
a Krita cloned frame.

Every time you save in Krita, the app notices within a couple of seconds and
folds the save back in as **one undo step**, touching only what Krita changed:

- **Drawings** are matched by content. One you didn't touch in Krita keeps
  whatever you have drawn on it here since; one you changed in Krita replaces
  it (if both sides changed it, Krita wins — Ctrl+Z brings yours back).
- **Timing** on a layer is only rebuilt if its keys changed in Krita.
- **Layers** added, deleted or reordered in Krita are added, deleted or
  reordered here. Layers you added here meanwhile stay where they are.

Layer transforms, the camera, tracker points and the light-table flag stay
here — Krita edits the drawings, this app keeps the staging.

**Layers that stay on one side.** A name ending in **`-x`** (any case,
`sketch-x`, `BG -X`) keeps a layer in the app it's in:

- Here, `-x` layers and **reference** (lightbulb) layers are never sent — a
  video background stays out of Krita once you click its lightbulb or rename
  it `bg-x`. The send toast lists what was kept out.
- In Krita, an `-x` layer — or any layer inside a group named `…-x` — never
  comes back. Use it for Krita-side sketches and guides.
- Renaming a synced layer to `…-x` in Krita, or making one reference / `-x`
  here, just ends its link: both copies stay as they are and nothing is
  deleted on either side.
- **Send to Krita again** only updates the layers that come from this app.
  Krita's `-x` layers and groups are carried into the new file untouched —
  copied as Krita saved them, so any layer type survives — and go back in the
  same place in the stack, just above the layer they sat on.

The link lasts until you quit, start a new project, open another, or choose
**Stop Krita link**. While linked, the menu item becomes **Send to Krita
again**. Any Krita save the app hasn't read yet comes in first, then the file
is rewritten. Krita is found under Program Files, in `/Applications`, or on
`PATH`; otherwise the app asks once and remembers.

### Krita helper (reload in place)

Krita doesn't reload a file it has open and has no Reload command, so without
help you'd close the file there and reopen it after every **Send to Krita
again**. **File → Install Krita helper…** puts a small plugin, *Animator
Link*, in Krita's plugin folder (`%APPDATA%\krita\pykrita` on Windows). Enable
it once — Krita: *Settings → Configure Krita → Python Plugin Manager*, tick
*Animator Link*, restart Krita — and from then on Send to Krita again:

1. asks Krita to get ready; if the Krita document has unsaved edits, the
   helper saves them and they come in here first, as one undo step;
2. writes the new file;
3. has Krita swap it into the same window — same frame, active layer, zoom,
   rotation and mirror (the view recenters: Krita's scripting can't set the
   pan) — and show *Updated from Animator*.

If the helper doesn't answer within a few seconds (not enabled, or Krita is
closed), the send goes ahead as before and the toast says what to check. The
two sides talk through two small text files next to the linked `.kra`
(`.to-krita`, `.to-app`); the protocol is in
`krita-plugin/animator_link/mailbox.py`, tested with
`python -m unittest discover -s krita-plugin/tests`.

What doesn't come back, with a note saying so: vector, filter, fill, clone
and file layers, masks, blend modes other than Normal, animated opacity.
Groups are flattened into plain layers. The file must stay 8-bit RGBA.

**File → Import .kra…** adds a Krita document's paint layers under the
active layer (skipping `-x` ones, as the link would); **File → Export .kra…**
writes the whole project, every layer, as an animated `.kra` without linking. Import is also how to recover a Krita save made after the
app closed — the temp file stays on disk.

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
| Ctrl+A / Ctrl+Shift+I     | Select all / invert selection |
| Esc / Ctrl+D              | Deselect                      |
| Ctrl+Shift / Ctrl+Alt / Shift+Alt (held) | Add to / remove from / intersect the selection |
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
