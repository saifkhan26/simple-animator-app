# Glossary

Animation, graphics and Krita words used in the code and these docs, in plain language. Rust terms are in the
[Rust primer](rust-primer.md).

## Animation terms

**Frame** — one moment of the animation timeline (at 24 fps, one frame is 1/24 s). Frame numbers are 0-based in the
code and shown 1-based in places in the UI.

**Cell / drawing** — one drawing: a pixel image (`Canvas`). The code calls them *cells* (as in traditional cel
animation); the UI calls them drawings. Identified by a `CellId`, an index into `Project::cells`.

**Layer** — a row of the animation that shows one drawing per frame, stacked bottom to top. See
[doc/layer.rs](src/doc/layer.md).

**Exposure** — which drawing a layer shows on a frame. A layer's `exposures` list has one entry per frame.

**Key** — a frame where a layer starts showing a (new) drawing: `Some(cell)` in `exposures`. Also used for
**transform keys**, **camera keys** and **grid keys**: values set on a frame and interpolated in between.

**Hold** — a frame that keeps showing the previous key's drawing: `None` in `exposures`.

**On twos / on threes** — each drawing held for 2 (or 3) frames; common in hand-drawn animation. "Timing on N" in
the timeline sets every selected drawing's hold to N frames.

**X-sheet (exposure sheet)** — the traditional frames × layers table showing which drawing is exposed when.

**Timeline tracks** — the Timeline panel's rows, where each drawing is a bar you can move and stretch
([ui/tracks.rs](src/ui/tracks.md)).

**Onion skin** — faint, tinted copies ("ghosts") of earlier (blue) and later (red) drawings behind the current one,
so you can see the motion. **Pins** keep chosen frames visible as ghosts.

**Loop range** — the frames playback repeats over.

**Camera** — the rectangle of the artwork that exports show; can pan, zoom and roll, keyed over time
([doc/camera.rs](src/doc/camera.md)). **Look through camera** locks the view to it.

**Reference layer** — a layer shown dimmed for tracing and left out of exports.

**Cleanup layer** — a new layer over a rough one, for redrawing it cleanly; its key jumps follow the rough's timing.

**Tracker** — a tool that places a tracking point per frame for stabilising.

## Pixels and colour

**RGBA** — four bytes per pixel: red, green, blue, alpha (opacity), each 0–255. Pixel `(x, y)` of a `w`-wide image starts
at byte `(y·w + x)·4`, as in a browser's `ImageData`.

**Straight (unpremultiplied) alpha** — colour and alpha stored independently: half-transparent red is `(255, 0, 0, 128)`.
How `Canvas` stores pixels.

**Premultiplied alpha** — colour already multiplied by alpha: half-transparent red is `(128, 0, 0, 128)`. GPUs blend
this way. Converting: `c' = round(c × a / 255)`. Mixing straight-alpha pixels directly drags the colour of
transparent pixels (black) into edges, which is why sampling is done premultiplied.

**sRGB / gamma vs linear** — stored colour values are "gamma-encoded" (perceptual); "linear light" values are
proportional to physical brightness. Blending in one space or the other gives slightly different results; the canvas
blends in gamma space on purpose ([ui/canvas_gpu.rs](src/ui/canvas_gpu.md)).

**HSL** — hue (0–360°), saturation, lightness: the colour model the UI shows, computed like CSS `hsl()`
([color.rs](src/color.md)).

**Source-over** — the normal "paint on top" blend: `out = src + dst × (1 − src_alpha)` (premultiplied).

**Dirty rectangle** — the bounding box of what an edit changed, so only that part is re-uploaded or stored for undo.

**Texture** — an image in GPU memory. Each visible drawing has one; edits upload changed patches.

**Quad** — a rectangle made of two triangles, used to draw a texture at any position, scale and rotation.

## Brushes and strokes

**Stroke** — one pen-down-to-pen-up line.

**Pen sample / packet** — one reading from the tablet: position, pressure, tilt, time, barrel rotation.

**Pressure / tilt** — how hard the pen presses (0–1) and how far it leans (degrees in x and y).

**Smoothing / stabilizer** — filtering the pen's path so lines are cleaner; the stabilizer makes the line trail the
pen and average its motion ([tools/stabilizer.rs](src/tools/stabilizer.md)).

**Spine** — the centre line of a stroke as a list of nodes with radius and flow.

**Ribbon** — a stroke drawn as one continuous band of capsules combined with `max()` (ink: no darkening where it
overlaps itself) ([tools/ribbon.rs](src/tools/ribbon.md)).

**Dab** — one stamp of the brush tip; dab brushes stamp many along the stroke, and overlaps build up like graphite.

**Spacing** — distance between dabs, as a fraction of the dab size.

**Flow / opacity** — flow: how much one dab lays down; opacity: the whole stroke's strength.

**Hardness / softness** — how much of the brush radius is solid versus fading out.

**Grain / paper tooth** — a paper texture that thins paint where the paper "dips", so strokes look like pencil on
paper ([tools/paper.rs](src/tools/paper.md)).

**Midtone noise** — speckle in a stroke's soft edge only ([tools/noise.rs](src/tools/noise.md)).

**Alpha lock** — painting only changes pixels that already have paint.

**Clipping mask (clip to layer below)** — a layer shows only where the layer under it has paint
([doc/clip.rs](src/doc/clip.md)).

## Selection and fill

**Selection mask** — per-pixel 0–255 "how selected" values, like an alpha channel ([tools/select_mask.rs](src/tools/select_mask.md)).

**Marching ants** — the animated dashed outline of a selection.

**Float / floating pixels** — selected pixels lifted off a drawing to be moved, scaled or rotated before they land.

**Feather** — softening a selection's edge.

**Flood fill / bucket** — filling the connected area of similar pixels around a click ([tools/fill.rs](src/tools/fill.md)).

**Tolerance** — how different a pixel may be and still be filled or selected.

**Gap closing** — treating small breaks in line art as closed so a fill doesn't leak.

**Distance field / distance transform** — for every pixel, the distance to the nearest wall pixel.

**Watershed** — growing several regions outward at once by a "height" (here: distance from walls) until they meet;
used to decide where a gap is.

**Magic wand** — selecting the connected pixels like the clicked one ([tools/wand.rs](src/tools/wand.md)).

**Lasso** — a hand-drawn (or tapped polygon) outline used to select or fill.

**Liquify** — pushing pixels around with a brush via a displacement field ([tools/liquify.rs](src/tools/liquify.md)).

## Perspective

**Vanishing point** — where lines that are parallel in the world meet in the picture.

**Horizon** — the line through the vanishing points of a flat floor: the eye level.

**Homography** — the projective map that takes a square to any four-cornered shape; how a grid's corners define
its whole perspective ([tools/perspective.rs](src/tools/perspective.md)).

**Snapping** — holding a stroke to the direction of a vanishing point.

## Krita

**Krita** — a free, open-source painting program (C++/Qt). Animator can open and save its files and round-trip with it.

**`.kra`** — Krita's document format: a zip of XML plus tiled pixel files ([io/kra/mod.rs](src/io/kra/mod.md)).

**`.kpp` (preset)** — a Krita brush preset: a PNG thumbnail with the settings XML in a text chunk
([tools/krita/kpp.rs](src/tools/krita/kpp.md)).

**Bundle** — a zip of Krita resources (presets, tips, textures).

**Paint op / pixel brush** — Krita's brush engine type; Animator ports the pixel brush (`paintbrush`).

**Brush tip** — the stamp image a Krita brush uses: a bitmap, a generated "auto" tip, or a GIMP `.gbr`/`.gih`.

**Sensor** — a pen input (pressure, tilt, speed, drawing angle, fade, random) driving a brush option through a curve
([tools/krita/sensors.rs](src/tools/krita/sensors.md)).

**Curve option** — a brush setting (Size, Opacity, Rotation…) controlled by sensors.

**Build-up vs wash** — Krita painting modes: build-up stamps each dab onto the layer; wash builds the stroke in a
separate buffer, then merges it, so a stroke doesn't darken over itself.

**`-x` layer** — a Krita layer named ending in `-x`: kept only in Krita by the Edit in Krita link.

**Golden test** — a test comparing an image's hash with a stored value, to catch any change in output.

## Files and app infrastructure

**`.anim`** — Animator's project file: a small header, then the project serialised with postcard and compressed with
zlib ([io/project_file.rs](src/io/project_file.md)).

**postcard** — a compact binary serde format (no field names; field order matters).

**`ui.ron`, `shortcuts.toml`** — preference files in the app's config folder ([getting-started.md](getting-started.md#5-where-the-app-keeps-its-files)).

**egui / eframe / wgpu** — the UI library, its window/app framework, and the GPU API underneath
([egui and immediate mode UI](rust-primer.md#egui-and-immediate-mode-ui)).

**Wintab** — the Windows tablet driver API used for pen input ([input/tablet.rs](src/input/tablet.md)).

**ffmpeg** — an external video tool used for MP4 export and video import.
