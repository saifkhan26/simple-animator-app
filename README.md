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
- **Layer groups** — fold, hide, lock, fade, move and clip to a whole group;
  a group at 50% fades as one picture, as in Krita, and goes to Krita and back
  as a real group.
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
- **Selections, Krita-style** — lasso (drag freehand or tap corners), box, ellipse or magic
  select (click to pick the pixels joined to it); add to,
  remove from or intersect them; every stroke and fill stays inside. Move,
  cut, copy and paste the selected pixels between frames and layers.
- **Flip the view** horizontally or vertically as a drawing check. Never
  touches the document.
- **Pinned colour swatches**, remembered across runs.
- **Lasso fill** — the Fill tool's second mode: drag round an area or tap it
  out as a polygon, and it fills; optionally only what its lines shut in, or
  behind the line art.
- **Colour wheel with harmonies** — an HSL wheel in the Brush panel that lays
  Complementary, Monochromatic, Analogous, Triadic or Tetradic colours around
  the one you pick. Every colour in the app reads and pastes as `hsl(…)`.
- **Tablet pressure** on Windows via Wintab (Wacom, Huion, XP-Pen, Gaomon, …)
  — auto-detected. Falls back to mouse (constant pressure) when no driver
  found.
- **Undo / Redo** with bounded history (80 entries), stored as dirty-rect
  pixel snapshots so memory stays small.
- **Export** to PNG sequence, animated GIF (NeuQuant palette), animated WebP
  (lossless), MP4, sprite sheet, or a single PNG of the current flattened frame — each over a chosen
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
│ │             │                                │  ☑ Layer 1  α    │  │
│ │             │                                └──────────────────┘  │
│ ├─────────────┤                                                      │
│ │  Brush      │                                ┌──────────────────┐  │
│ │  Wheel      │                                │  X-sheet         │  │
│ │  Backdrop α │                                │  Fr  L1  L2      │  │
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
2. Set color on the wheel in the **Brush** window.
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
| Fill   | Bucket: scanline flood within tolerance, click once to fill. Lasso: drag or tap round what to fill. |
| Liquify | Push the drawing's pixels with the pen, inside the selection.   |

**Liquify.** Drag on a drawing to push its pixels along (Push), or to even
out a warp's kinks (Smooth). With a selection, only what's inside moves, and
the warp eases out over the **Edge falloff** so it never tears at the edge;
without one the whole drawing warps. Pen pressure sets how far the pixels
follow, up to **Strength**, and the Tool-adjust drag (or its hold key) sizes
the brush sideways and sets the strength upward. Every pass is resampled from
the drawing as the session found it, so going back and forth doesn't soften
the lines. `Ctrl+Z` takes back a stroke; `Enter`, or changing tool, frame or
layer, applies the warp as one undo step; `Esc` throws it away. With **All
frames** on, the same warp lands on every drawing of the layer — fixing a
proportion through a whole cycle. **Show mesh** draws the warp as a grid.

**Lasso fill.** Press the Fill key again (or pick **Lasso** in the Tools
panel) to switch the Fill tool from the bucket to the lasso; the tool button
shows which is on. Drag round an area and let go to fill it, or tap it out
corner by corner and tap the first corner, double-click or press `Enter` to
fill; a drag between corners adds a freehand stretch, as on the selection
lasso. `Backspace` or `Ctrl+Z` takes back a corner, `Esc` drops the shape. It
fills with the brush colour at the Fill opacity, inside the selection and
under alpha lock.

- **Enclose & fill** — fill only the areas the lines inside the lasso shut
  in, not the space between them and the lasso. It reads lines the way the
  bucket does (*lines from*, gap closing, expand), and the canvas edge closes
  an area as a line does: two strokes across a corner make a shape with it
  (drag the lasso past the edge to take it in). Hold `Ctrl+Shift` as a shape
  lands for the other way.
- **Behind existing paint** — the fill goes under what is already there:
  line art keeps its colour and its soft edges.
- Hold `Ctrl+Alt` to erase the shape instead, and `Shift+Alt` to turn
  polygon edges in 15° steps. All three holds are rebindable.

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

**Making one.** Pick a shape in the Lasso panel: lasso, rectangle or ellipse.
The lasso takes a drag or taps: drag round an area and let go for a freehand
selection, or tap to put down corners for a polygon. Between corners, a drag
adds a freehand stretch, so one shape can have straight sides and curves. A
tap back on the first corner, a double-click or `Enter` closes it,
`Backspace` takes back the last corner (with any stretch drawn from it) and
`Esc` abandons it. A quick click no longer deselects with the lasso; `Esc` or
`Ctrl+D` does, and a click still does with the rectangle and ellipse.

**Magic select.** The fourth shape, the wand, picks by what is drawn: click a
line to select that line, inside a shape to select the area up to its lines,
or bare canvas to select the bare canvas joined to it. Keep the pen down after
the click and drag sideways to change the **Tolerance** (how far a colour may
stray from the one clicked), or up and down to change the **Gap**: breaks in
the lines up to that wide count as shut, as with the Fill tool, so the pick
doesn't leak out. The outline follows as you drag and the sliders keep what
you land on; the whole press is one undo step, and `Esc` before letting go
puts the old selection back. **Expand** grows each pick so a flat colour's
selection reaches under its anti-aliased lines. **Any paint** makes a click
on paint take every painted pixel touching it, whatever its colour — a
character's lines and fills as one piece, ready to drag away. Untick
**Contiguous** to pick every pixel like the one clicked, anywhere on the
drawing. It reads the active drawing; **Read all visible layers** picks by
what every visible layer shows together, so on an empty colour layer a click
selects the area the line layer walls in. A click inside the selection picks
afresh there; a drag from inside it moves the pixels, as with the other
shapes. The held Add / Remove / Intersect keys work on picks too.

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

A plain click outside the selection deselects (with the magic select, it picks).

**Flipping and turning the art.** The flip and rotate buttons in the Lasso
panel (also **Edit → Flip / rotate**, and unbound shortcuts in Settings)
mirror or turn the pixels themselves — the `F` view flip only changes how the
canvas is shown. With a selection they act on the selected pixels, about the
selection's centre, and leave them floating so they can still be moved. With
nothing selected (or Select All) they act on the current drawing about the
canvas centre, or on every drawing of the layer with **All frames** on; a
layer's right-click menu does the whole layer. Pixels only move, never
resample, so flipping back gives the drawing back exactly. A quarter turn of a
non-square drawing swaps its width and height rather than cropping it.

**Moving pixels.** Drag inside the selection to move what's under it, or
nudge with the arrow keys. Drag a handle on the box around it to scale:
corners take both axes, edges take one. `Shift` on any handle keeps the box's
proportions — the ones it had, so a stretched box stays stretched — and `Alt`
scales from the centre, so both sides move; either can be pressed or let go
mid-drag. Over the box, Shift and Alt mean this rather than pan and rotate,
even when held before the press. Drag just outside a corner to rotate, with
`Shift` snapping to 15°.

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

- **▶ / ❚❚** — play / pause the loop range. Stopping goes back to the frame
  playback started on, as does a run with the loop off reaching its end;
  `Shift+Space` or a Shift-click pauses where it is instead. A scrub while
  playing stops where you put it.
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

- `+` / `−` — add / remove layer. `▲` / `▼` — reorder.
- **Second toolbar** — flip horizontally / vertically and turn every drawing
  of the active layer and every layer picked with it; **clip to layer below**
  and **lock alpha** for the same layers.
- **Opacity** — the slider under the toolbars sets the active layer's
  opacity, or every picked layer's at once; one undo step per drag, a
  double-click puts it back to 100%. A layer under 100% shows its opacity on
  its row.
- On each row: eye, **🔒** lock (refuses strokes), **Ref** light-table
  layer (dimmed on every frame, left out of export), the name (double-click to
  rename, drag to reorder), and **⋮** for the rest (rename, group, Edit in
  Krita; for a group, ungroup / merge / delete). Right-click the name for the
  same menu.
- **lines from** — the layer whose lines a fill on this one stops at.
- **keys with** — key jumps on this layer also stop at that layer's drawings,
  and the button beside it adds a cleanup layer over this one, already linked.

Layers paint bottom-up. The list shows them top-down (top of list = on top).

#### Groups

Ctrl-click layer names to pick several (Shift-click picks a range), then
**Group** (`Ctrl+G` on a new install) puts them in a group. Groups are one
level deep: a group holds layers, not other groups.

- **Group row** — the chevron folds it; its eye, lock and opacity cover every
  layer inside. Opacity is a true group composite: the layers are mixed first
  and the result faded, so overlapping lines inside don't show through each
  other — the same picture Krita draws.
- **Drag** a layer onto a layer inside a group to put it in; drag it out to
  take it out. Dragging the group row moves the whole group.
- **The group row's ⋮** — *Ungroup*, *Merge group* (one layer holding
  exactly what the group showed, at the group's opacity; hidden and reference
  layers inside are kept as layers), *Delete group*. With the group active,
  the flip and turn buttons work on every drawing inside.
- **Clip to the layer below** on a layer sitting just above a group clips it
  to everything the group shows. Inside a group, a layer clips to the layer
  below it in the same group.
- **Move a whole group** — pick the group row and use the layer transform
  (move, turn, scale): every layer inside moves together, as one undo step.
- The **timeline tracks** and the **X-sheet** show a group row with its
  layers' keys under it, and indent the layers inside; folding hides them.

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

The Brush panel keeps a strip of pinned colours under the wheel: `+` pins the
current colour, a click selects one, right-click removes it. Up to 32, saved
with your preferences, so a palette follows you between projects.

## Colour wheel

The top of the **Brush** panel is an HSL wheel: hue goes round the disc, saturation runs
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
current colour exactly, and the **+** at the end of the set adds the whole set
to your swatches.
The set rebuilds around the brush colour whenever it changes elsewhere
(eyedropper, a swatch, Paste). The scheme is remembered; the set is not.

**Pin the wheel** with the pin beside the scheme (or bind *Pin the colour
wheel* in Settings). While it's lit, every tool paints in one colour: pick it
on the Pencil and the Ink, Fill and Shape have it too. Normally each tool keeps
its own. The wheel's set also leads the swatch pie under the pen, on the inner
ring by itself, with your pinned swatches on a ring round it. The set stays put
while pinned: only the wheel's own dots, bar and fields move it. Picking one of
its colours from the pie lights that member; any other colour (a swatch, the
eyedropper, Paste) paints without touching the set. Unpinning lets it follow
the brush colour again, and leaves every tool on the colour it had; it starts
unpinned each launch.

Every colour button in the app opens the same wheel. Colours show as
`hsl(h, s%, l%)` (hover a swatch), and **Paste** — the clipboard button at the
end of the H / S / L row — takes `hsl(…)`, `rgb(…)` or `#hex`.

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

**Backdrop**, the last row of the Brush panel, sets the colour behind the
canvas (its swatch) and its opacity (the percentage: drag it sideways or click
and type), which drives the window clear-color alpha. Drop it to 0% to make
the entire app background transparent. The floating windows stay
semi-translucent so they remain readable.

## Export

| Action                     | Where                            |
|----------------------------|----------------------------------|
| Current frame as PNG       | **File → Save PNG…**             |
| PNG sequence               | **File → Export PNG sequence…**  |
| Animated GIF               | **File → Export animated GIF…**  |
| Animated WebP              | **File → Export animated WebP…** |
| MP4 (needs ffmpeg on PATH) | **File → Export MP4…**           |
| Sprite sheet               | **File → Export sprite sheet…**  |

Every export except the single PNG opens one dialog with a **frame range**
(dual-knob slider, exact inputs, and a *Use loop range* button that fills it
from the loop bars on the frame strip), plus the options that format needs —
CRF and preset for MP4, columns and padding for the sheet. It reports the frame
count, and for a sheet the pixel dimensions, before you commit to it.

A **Playback** section shapes how the range plays:

- **Ping-pong** (MP4, PNG sequence, GIF, WebP) — forward, then back, without
  doubling the end frames, so the wrap is seamless.
- **Loop to N s** (MP4, PNG sequence) — repeats the range to fill that many
  seconds, rounded *up* to whole loops so the file ends on a loop boundary and
  a player that repeats it repeats it seamlessly. 21 frames at 24 fps looped
  to 10 s gives 12 loops, 10.5 s; the dialog shows the real length before you
  export. GIFs and WebPs repeat on their own, so they don't need it.
- **Stop after N plays** (WebP) — off, it repeats forever like a GIF.

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

An animated WebP is lossless: exact colours and 8-bit alpha, where a GIF has a
256-colour palette and on/off transparency. Each frame stores only the
rectangle that changed, and a held drawing is stored once and shown for longer,
so line art on twos comes out much smaller than the GIF. Full-colour painting
at 4K can come out bigger than the GIF, which gets its size by dropping
colours. **File → Import WebP…** brings one back in through the same
frame-range dialog as *Import GIF…*.

## Krita

**File → Edit in Krita** (or right-click a layer name → *Edit in Krita*, which
opens with that layer selected) writes the project as an animated `.kra` to a
temp folder and opens it in Krita. Keys, holds, layer names, opacity,
visibility and lock all travel; a drawing reused at several frames arrives as
a Krita cloned frame. Groups arrive as Krita group layers, folded, faded and
hidden as they are here, and come back the same way: a group made in Krita,
or a layer moved in or out of one there, shows up here. Krita groups nested
inside groups fold into the outermost one, since groups here are one level
deep.

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
lists the current binding for each one. **Open shortcuts file** there opens
`shortcuts.toml` (in `%APPDATA%\animator-app`) in your editor, writing it out
first on a fresh install; **Reload** loads your saved edits without a restart. A
file with a mistake in it is reported and left unloaded, key names read in any
case (`ctrl+space`), and a line that doesn't read is listed there and kept in the
file. Changes made in Settings go into the file in place, so your comments and
lines stay as you wrote them. A few of the defaults:

| Shortcut                  | Action                        |
|---------------------------|-------------------------------|
| Q / W / E / R / G / Y     | Pencil / Ink / Eraser / Fill / Shape / Lasso |
| A / S                     | Previous / next frame         |
| Space / Shift+Space       | Play / pause (back to the start frame / where it stops) |
| O                         | Toggle onion skin             |
| F / Shift+F               | Flip view H / V               |
| Ctrl+X / C / V            | Selection cut / copy / paste  |
| Ctrl+A / Ctrl+Shift+I     | Select all / invert selection |
| Esc / Ctrl+D              | Deselect                      |
| Ctrl+Shift / Ctrl+Alt / Shift+Alt (held) | Add to / remove from / intersect the selection |
| Ctrl+Shift / Ctrl+Alt / Shift+Alt (held) | Lasso fill: Enclose & fill the other way / erase / 15° polygon edges |
| Alt+X / C / V             | Drawing cut / copy / paste    |
| Ctrl+Z / Ctrl+Y           | Undo / redo                   |
| Ctrl+G                    | Group the picked layers (new installs; Shift+Alt+G is the perspective grid) |
| Ctrl+S / Ctrl+Shift+S     | Save / Save As                |
| Tab                       | Hide / show panels            |

**Held while dragging.** Pan, zoom and rotate (`Shift` / `Ctrl` / `Alt` by
default), and brush size and opacity, scrub, flip and the swatch pie (unbound
by default) can each be held on a key, a combo or bare modifiers — `Space` to
pan, or `Shift` to size the brush as in Krita. Bind them under **Held while
dragging** in Settings; tap Shift, Ctrl or Alt there to bind a modifier on
its own. Letting go of the key ends the gesture, and the rest of that drag
does nothing. An unbound action (the × beside it) stays unbound.

## Files

Projects save as a binary `.anim` file (**File → Save**, `Ctrl+S`) — postcard
over a magic + version header, currently version 8, with migrations for every
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
