# `src/brush_library/mod.rs` — the brush library (every preset the picker offers)

> **Source:** [src/brush_library/mod.rs](../../../src/brush_library/mod.rs) · **Module:** `crate::brush_library` · **Size:** 617 lines · Line links as of commit `36405ad`

## What this file is for

This file is the app's **brush library**: the list of every brush the brush picker in
*Brush settings* shows, plus the code that loads those brushes and draws their little
preview pictures (thumbnails). The brushes come in three groups:

| Group | What it is | Entry id looks like |
|---|---|---|
| **App** | The app's own presets, `BrushSettings::PRESETS` in [tools/mod.rs](../tools/mod.md). | `app/Pencil` |
| **Krita** | The Krita presets the app ships inside its own binary (see [tools/krita/defaults.rs](../tools/krita/defaults.md)). | `krita/c)_Pencil-5_Tilted` |
| **Imported** | Krita presets the user imported from their own Krita install. | `user/<folder name>` |

A **preset** (a `.kpp` file) is Krita's saved brush: a PNG picture of the brush with the
brush's settings stored inside the PNG as XML. A preset usually refers to a **tip** — the
small image (or generated shape) stamped along the stroke — by file name. See the
[glossary](../../glossary.md).

**Where the files live on disk.** Imported presets are not read from Krita. Each one is
copied (by [brush_library/import.rs](import.md)) into its own folder under the app's
**brushes folder**: `<config>/brushes/<name>/`, holding the `.kpp`, any tip files it uses and
a `source.toml` saying where it came from. The app passes that folder in as `root` when it
builds the library ([app.rs](../app.md) uses `shortcuts::config_dir().join("brushes")`, and
`config_dir()` in [input/shortcuts.rs](../input/shortcuts.md) is the OS config folder plus
`animator-app` — on Windows that is `%APPDATA%\animator-app\brushes\`). Tests pass `None`, which
means "no imported presets and nowhere to save".

**Threads.** Turning a Krita preset into a paintable brush means decoding its tip and paper
texture, which is too slow to do for two dozen presets in one frame. So the library starts a
**background thread** (named `brush-library`) that loads every Krita entry and draws every
thumbnail, and sends each result back over a channel. Every frame the app calls
[`BrushLibrary::poll`](#brushlibrarypoll) to pick up whatever has arrived, and the picker shows
the tiles as they come in. If a stroke needs a preset before the thread got to it,
[`BrushLibrary::brush`](#brushlibrarybrush) loads it on the spot.

The library also backs the **Krita brush editor** ([ui/krita_editor.rs](../ui/krita_editor.md)):
an edit is a list of `(param, value)` *overrides* laid over a preset, and the library builds
and caches the edited brush, and can save an edit as a new imported preset.

## How it connects

- **Used by:**
  - [app.rs](../app.md) — `AppState` owns one `brush_library`. `with_prefs` creates it,
    `update` calls `poll`/`loading` every frame, `krita_brush` asks it for the brush to paint
    with, `open_krita_import` reads its `root`, `rescan_brushes` calls `rescan`.
  - [ui/brush_picker.rs](../ui/brush_picker.md) — draws `entries` as a grid of tiles, uploads
    `thumbnail`s, and calls `entry`/`brush` when a tile is picked.
  - [ui/krita_editor.rs](../ui/krita_editor.md) — `preset`, `edited_preset`, `brush_with`,
    `tips_for`, `entry`, `save_as_new`.
  - [brush_library/import.rs](import.md) — a child module; reuses `display_name`,
    `kpp_thumbnail` and `FolderTips`. In the other direction, `save_as_new` here calls its
    `install`, `folder_name`, `hash` and `SourceInfo`.
- **Uses:**
  - [tools/krita/mod.rs](../tools/krita/mod.md) — `PixelBrush` (a loaded Krita brush) and the
    `Resources` trait (where tips come from).
  - [tools/krita/defaults.rs](../tools/krita/defaults.md) — the shipped presets and tips
    (`PRESETS`, `TIPS`, `kpp`, `load`, `DefaultTips`).
  - [tools/krita/kpp.rs](../tools/krita/kpp.md) — `Preset` (a preset's settings read from the
    file) and `with_overrides`.
  - [tools/krita/edit.rs](../tools/krita/edit.md) — `attr`, to read the tip file name.
  - [tools/krita/kpp_write.rs](../tools/krita/kpp_write.md) — `embed_tip` and `write`, to save
    a new `.kpp`.
  - [tools/mod.rs](../tools/mod.md) — `BrushSettings`, `BrushMode`, `ActiveTool`,
    `SmoothingOptions`.
  - [tools/stroke.rs](../tools/stroke.md), [tools/ribbon.rs](../tools/ribbon.md),
    [doc/canvas.rs](../doc/canvas.md), [input/pointer.rs](../input/pointer.md) — to paint the
    S-shaped preview strokes.
  - [ui/theme.rs](../ui/theme.md) — `premul`, to premultiply thumbnail pixels.

```text
 BrushLibrary::new(root)
   ├─ list App entries (Ready), Krita entries (Loading), scan root/ for Imported (Loading)
   └─ start_loading() ──spawn──▶ thread "brush-library"
                                   for each entry: load brush + draw thumbnail
                                   tx.send(Loaded{id, brush, thumb})
                                          │ mpsc channel
 every frame: AppState::update ─▶ poll() ◀┘  (try_recv: never blocks)
                                   entry.status = Ready / Refused(why)
                                   brushes[id] = brush, thumbs[id] = picture
```

## Rust concepts in this file

**Enums that carry data.** A Rust `enum` variant can hold values, like a TypeScript tagged
union. `match` then unpacks them.
```rust
pub enum Source {
    App(usize),                 // index into BrushSettings::PRESETS
    Embedded(&'static str),     // a shipped preset's file name
    Folder(PathBuf, String),    // an imported preset's folder and .kpp name
}
match source {
    Source::Folder(dir, kpp) => std::fs::read(dir.join(kpp)).ok(),
    _ => None,
}
```
```js
// { kind: "app", index } | { kind: "embedded", name } | { kind: "folder", dir, kpp }
if (source.kind === "folder") return fs.readFileSync(path.join(source.dir, source.kpp));
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

**A thread and a channel.** The loader runs on its own OS thread and talks back through an
`mpsc` channel ("multi-producer, single-consumer" queue). `move ||` hands the job list to the
thread. `try_recv` checks the queue without waiting.
```rust
let (tx, rx) = mpsc::channel();
std::thread::Builder::new().name("brush-library".into()).spawn(move || {
    for (id, source) in jobs { tx.send(Loaded { id, brush, thumb }); }
});
while let Ok(l) = rx.try_recv() { /* use l */ }
```
```js
const worker = new Worker("loader.js");           // like the spawned thread
worker.onmessage = (e) => queue.push(e.data);      // like tx.send(...)
// each frame: while (queue.length) use(queue.shift());   // like try_recv
```
More: [Threads and channels](../../rust-primer.md#threads-and-channels)

**`RefCell`: changing a cache from a `&self` method.** Rust normally forbids changing a value
through a shared (`&self`) reference. `RefCell` moves that check to run time, so a "read"
method like `brush(&self, id)` can still fill a cache.
```rust
brushes: RefCell<HashMap<String, Arc<PixelBrush>>>,
// inside fn brush(&self, ...):
self.brushes.borrow_mut().insert(id.to_owned(), b.clone());
```
In JS every object is mutable, so a getter can always write to a cache; Rust needs the
`RefCell` wrapper to allow it. More: [Smart pointers](../../rust-primer.md#smart-pointers)

**`Arc`: one brush, many owners.** A loaded `PixelBrush` is wrapped in `Arc` (an
atomically reference-counted pointer). Cloning an `Arc` copies a pointer, not the brush, and it
is safe to send between threads — the loader thread builds brushes and hands them to the UI
thread this way. It is the nearest thing to a JS object reference.
More: [Smart pointers](../../rust-primer.md#smart-pointers)

**Traits and borrowed structs.** `Resources` is a trait (an interface) with one method,
`tip(filename)`. Small structs here implement it. `FolderTips<'a>(&'a Path)` holds a *borrowed*
path; the `'a` lifetime says "this struct cannot outlive the path it points at".
```rust
struct FolderTips<'a>(&'a Path);
impl Resources for FolderTips<'_> {
    fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>> { /* read <dir>/<filename> */ }
}
```
```js
const folderTips = (dir) => ({ tip: (name) => readFileOrNull(path.join(dir, name)) });
```
More: [Traits and derive](../../rust-primer.md#traits-and-derive),
[Lifetimes](../../rust-primer.md#lifetimes)

**`let … else` and errors as strings.** `let Some(x) = … else { return … };` unpacks an
`Option` or leaves early. Many loaders here return `Result<_, String>`, turning an `anyhow`
error into text with `format!("{e:#}")` — the `#` prints the whole chain of causes
("outer: inner: innermost").
More: [Option and Result](../../rust-primer.md#option-and-result)

## impl Group

## impl Resources for FolderTips

## impl Resources for Chain

## impl BrushLibrary

## Types

### `const THUMB`
[src/brush_library/mod.rs:23](../../../src/brush_library/mod.rs#L23)

```rust
pub const THUMB: u32 = 64;
```

`u32 = 64`. Thumbnails are this many pixels on their long side.

### `enum Group`
[src/brush_library/mod.rs:26](../../../src/brush_library/mod.rs#L26)

Which of the three picker groups an entry belongs to. Derives `Copy` and `PartialEq`, so it can
be compared with `==` and passed by value.

| Variant | Meaning |
|---|---|
| `App` | One of the app's own presets. |
| `Krita` | A Krita preset shipped inside the app. |
| `Imported` | A Krita preset the user imported into the brushes folder. |

### `enum Source`
[src/brush_library/mod.rs:44](../../../src/brush_library/mod.rs#L44)

Where an entry's brush comes from.

| Variant | Meaning |
|---|---|
| `App(usize)` | Index into `BrushSettings::PRESETS`. |
| `Embedded(&'static str)` | A shipped Krita preset, by its file name (e.g. `c)_Pencil-5_Tilted.kpp`). |
| `Folder(PathBuf, String)` | An imported preset: its folder and the `.kpp` file name inside it. |

### `enum Status`
[src/brush_library/mod.rs:55](../../../src/brush_library/mod.rs#L55)

Whether a Krita entry can paint yet.

| Variant | Meaning |
|---|---|
| `Loading` | The loader thread has not reported on it yet. |
| `Ready` | It loaded and can paint. |
| `Refused(String)` | It cannot paint; the text is the loader's reason (e.g. the preset uses an engine this app does not have). |

### `struct Entry`
[src/brush_library/mod.rs:63](../../../src/brush_library/mod.rs#L63)

One brush in the picker.

| Field | Type | Meaning |
|---|---|---|
| `id` | `String` | Stable id: `app/<label>`, `krita/<file stem>` or `user/<folder>`. Saved in preferences as `BrushSettings::krita_id`. |
| `name` | `String` | Name shown under the tile (see [`display_name`](#display_name)). |
| `group` | `Group` | Which picker group. |
| `source` | `Source` | Where the brush is loaded from. |
| `status` | `Status` | Loading / ready / refused. App entries start `Ready`. |
| `tooltip` | `String` | Hover text for the tile. |

### `struct Loaded`
[src/brush_library/mod.rs:73](../../../src/brush_library/mod.rs#L73)

What the loader thread sends back for one entry.

| Field | Type | Meaning |
|---|---|---|
| `id` | `String` | Which entry this is for. |
| `brush` | `Result<Arc<PixelBrush>, String>` | The loaded brush, or why it would not load. App entries send `Err("")` (they are not Krita brushes; only their thumbnail matters). |
| `thumb` | `Option<egui::ColorImage>` | The thumbnail picture, if one could be made. |

### `type Edited`
[src/brush_library/mod.rs:80](../../../src/brush_library/mod.rs#L80)

```rust
type Edited = Vec<(String, Vec<(String, String)>, Result<Arc<PixelBrush>, String>)>;
```

`Vec<(String, Vec<(String, String)>, Result<Arc<PixelBrush>, String>)>` — the brushes built
lately from edited presets, newest last: `(entry id, overrides, brush or error)`. A small cache
(at most 4) so a slider drag does not rebuild the same brush every frame.

### `type EditedPreset`
[src/brush_library/mod.rs:82](../../../src/brush_library/mod.rs#L82)

```rust
type EditedPreset = Option<(String, Vec<(String, String)>, Arc<Preset>)>;
```

`Option<(String, Vec<(String, String)>, Arc<Preset>)>` — the last edited preset asked for:
`(entry id, overrides, preset with overrides applied)`.

### `struct BrushLibrary`
[src/brush_library/mod.rs:84](../../../src/brush_library/mod.rs#L84)

The library itself. One lives in `AppState::brush_library`.

| Field | Type | Meaning |
|---|---|---|
| `root` | `Option<PathBuf>` | The brushes folder imported presets live in; `None` in tests. |
| `entries` | `Vec<Entry>` | Every brush, App group first, then Krita, then Imported (sorted by name). |
| `brushes` | `RefCell<HashMap<String, Arc<PixelBrush>>>` | Loaded Krita brushes by entry id. |
| `presets` | `RefCell<HashMap<String, Arc<Preset>>>` | Parsed `.kpp` settings by entry id (for the editor). |
| `edited` | `RefCell<Edited>` | Cache of brushes built from edits (see `Edited`). |
| `edited_preset` | `RefCell<EditedPreset>` | Cache of the last edited preset. |
| `thumbs` | `HashMap<String, egui::ColorImage>` | Decoded thumbnail pictures by entry id (CPU images; the picker uploads them as textures). |
| `incoming` | `Option<Receiver<Loaded>>` | The receiving end of the loader thread's channel; `None` if the thread could not start. |

### `struct FolderTips`
[src/brush_library/mod.rs:106](../../../src/brush_library/mod.rs#L106)

A tuple struct holding `&'a Path`: tips for a preset imported into its own folder are the files
beside it. Implements `Resources`.

| Field | Type | Meaning |
|---|---|---|
| `0` | `&'a Path` |  |

### `struct Chain`
[src/brush_library/mod.rs:117](../../../src/brush_library/mod.rs#L117)

A tuple struct of two `&dyn Resources`: look up a tip in the first, and if it is not there, in
the second. `dyn Resources` means "any type that implements `Resources`", chosen at run time.

| Field | Type | Meaning |
|---|---|---|
| `0` | `&'a dyn Resources` |  |
| `1` | `&'a dyn Resources` |  |

### `const TIP_EXTENSIONS`
[src/brush_library/mod.rs:184](../../../src/brush_library/mod.rs#L184)

```rust
const TIP_EXTENSIONS: [&str; 3] = ["png", "gbr", "gih"];
```

`["png", "gbr", "gih"]` — file extensions that count as tip files beside an imported preset
(PNG images, GIMP brushes, and GIMP image hoses, i.e. multi-image tips).

## `impl Group`

### `Group::label`
[src/brush_library/mod.rs:33](../../../src/brush_library/mod.rs#L33)

```rust
pub fn label(self) -> &'static str
```

**What it does.** The group's heading text: `"App"`, `"Krita"` or `"Imported"`. The
`&'static str` return type means a string literal baked into the program.

**Used by:** `groups` in [ui/brush_picker.rs](../ui/brush_picker.md) (the collapsing header
title and its id).

## Free functions

### `display_name`
[src/brush_library/mod.rs:96](../../../src/brush_library/mod.rs#L96)

```rust
pub fn display_name(file: &str) -> String
```

**What it does.** Turns a preset file name into the name shown in the picker, the way Krita's
own chooser reads: `c)_Pencil-5_Tilted.kpp` → `Pencil-5 Tilted`.

**How it works.**
1. Drop a trailing `.kpp`.
2. If the name has a `)_` and what comes before it is at most 2 characters (Krita's sort
   prefixes like `c)`), keep only what comes after.
3. Replace `_` with spaces.

**Used by:** `BrushLibrary::new`, `scan_imported`, and `scan` in
[brush_library/import.rs](import.md).

## `impl Resources for FolderTips<'_>`

### `FolderTips::tip`
[src/brush_library/mod.rs:109](../../../src/brush_library/mod.rs#L109)

```rust
fn tip(&self, filename: &str) -> Option<std::borrow::Cow<'_, [u8]>>
```

**What it does.** Reads the tip file `filename` from the folder this `FolderTips` points at.
Only the file-name part of `filename` is used, so a preset that names a path can never make the
app read outside the folder. Returns `None` if the file is missing.

**Returns.** `Cow::Owned(bytes)`. A `Cow` ("clone on write") can hold either borrowed or owned
data; here it always owns the bytes it just read.

**Used by:** `load_source`, `build_edited` and `source_tip` (through `Chain`), and `install` in
[brush_library/import.rs](import.md).

## `impl Resources for Chain<'_>`

### `Chain::tip`
[src/brush_library/mod.rs:120](../../../src/brush_library/mod.rs#L120)

```rust
fn tip(&self, filename: &str) -> Option<std::borrow::Cow<'_, [u8]>>
```

**What it does.** Asks the first source for the tip, and if it has none, the second
(`or_else` is like JS `a ?? b()`, evaluated only when needed).

**Used by:** `build_edited`, `source_tip`.

## Free functions

### `build_edited`
[src/brush_library/mod.rs:127](../../../src/brush_library/mod.rs#L127)

```rust
fn build_edited(source: &Source, preset: &Preset) -> Result<PixelBrush, String>
```

**What it does.** Builds a paintable brush from an (edited) `preset`. Tips are looked up beside
an imported preset first and then among the shipped tips — the shipped ones are always
reachable because the editor lets you switch the tip to one of them.

**Parameters.**
- `source` — where the original entry came from; only `Source::Folder` adds its folder.
- `preset` — the preset settings, usually already with overrides applied.

**Returns.** The brush, or the build error as text.

**Used by:** `BrushLibrary::brush_with`, `BrushLibrary::save_as_new`.

### `source_tip`
[src/brush_library/mod.rs:136](../../../src/brush_library/mod.rs#L136)

```rust
fn source_tip(source: &Source, file: &str) -> Option<Vec<u8>>
```

**What it does.** The bytes of tip `file`, from beside the imported preset if `source` is a
folder, else from the shipped tips. `into_owned()` turns the `Cow` into a plain `Vec<u8>`.

**Used by:** `BrushLibrary::save_as_new` (to embed the tip in a saved preset).

### `krita_picture`
[src/brush_library/mod.rs:145](../../../src/brush_library/mod.rs#L145)

```rust
fn krita_picture(brush: Arc<PixelBrush>, size: u32) -> image::RgbaImage
```

**What it does.** Paints an S-shaped stroke with a Krita brush on a square of paper and returns
the image. This becomes the picture of a preset saved with "Save as new" (a `.kpp` file *is* a
PNG, so it needs a picture).

**Parameters.**
- `brush` — the Krita brush to paint with.
- `size` — width and height of the picture in pixels (`save_as_new` uses 128).

**Returns.** An opaque RGBA image.

**How it works.**
1. Make a blank `size × size` [`Canvas`](../doc/canvas.md) and brush settings in
   `BrushMode::Krita`, near-black, with a radius of half the brush's own diameter but at most
   `size / 8` so it fits.
2. Start a stroke: a `StrokeWorkspace` (scratch buffers) and a `StrokeBuilder` created with
   `new_with_krita` so it paints through the Krita engine.
3. Feed 61 fake pen samples. For `t` from 0 to 1: x goes from 12% to 88% of the width; y wobbles
   around the middle by `sin(6t) × 22%` (the S-shape); pressure is `sin(πt)^0.6`, clamped to
   `0.05..=1` (light at the ends, firm in the middle); the pen is tilted 35° and samples are
   8 ms apart. After each sample, `flush` paints what is ready; `finish` ends the stroke.
4. Composite the transparent stroke over a paper colour (RGB 226, 226, 222), so the picture
   reads the same in Krita's chooser as in this app's picker.

**Example.** The "over paper" step for one colour channel, in JS:
```js
const over = (ch, paper, a) => Math.floor((ch * a + paper * (255 - a) + 127) / 255);
// a = stroke alpha 0..255; the +127 rounds to nearest
```

**Used by:** `BrushLibrary::save_as_new`.

### `load_source`
[src/brush_library/mod.rs:186](../../../src/brush_library/mod.rs#L186)

```rust
fn load_source(source: &Source) -> Result<PixelBrush, String>
```

**What it does.** Loads the Krita brush an entry points at: a shipped preset through
`defaults::load`, or an imported one by reading `<folder>/<file>.kpp` and loading it with the
tips beside it. An `App` source is an error ("an app preset is not a Krita preset").

**Returns.** The brush, or the reason it would not load (for example the preset uses a Krita
engine other than the pixel brush).

**Used by:** the loader thread in `BrushLibrary::start_loading`, and `BrushLibrary::brush`.

### `kpp_bytes`
[src/brush_library/mod.rs:197](../../../src/brush_library/mod.rs#L197)

```rust
fn kpp_bytes(source: &Source) -> Option<Vec<u8>>
```

**What it does.** The raw `.kpp` file bytes for an entry: from the shipped set or from its
folder. `None` for an app preset or an unreadable file.

**Used by:** `BrushLibrary::start_loading` (for thumbnails), `BrushLibrary::preset`.

### `color_image`
[src/brush_library/mod.rs:207](../../../src/brush_library/mod.rs#L207)

```rust
fn color_image(size: [usize; 2], rgba: &[u8]) -> egui::ColorImage
```

**What it does.** Converts a flat RGBA byte buffer into the `egui::ColorImage` egui wants,
premultiplying each pixel with `theme::premul`. *Premultiplied* means each colour channel is
already multiplied by alpha. The app does this in gamma (sRGB) space, matching its theme, so
soft edges do not come out too bright.

**How it works.** `chunks_exact(4)` walks the buffer four bytes (one pixel) at a time, like
looping `i += 4` in JS.

**Used by:** `kpp_thumbnail`, `app_thumbnail`.

### `kpp_thumbnail`
[src/brush_library/mod.rs:215](../../../src/brush_library/mod.rs#L215)

```rust
fn kpp_thumbnail(kpp: &[u8]) -> Option<egui::ColorImage>
```

**What it does.** A `.kpp` file is a PNG, so its thumbnail is simply its own picture: decode it
as PNG, shrink it to fit in `THUMB × THUMB` (triangle filter), and convert with `color_image`.
`None` if it does not decode.

**Used by:** the loader thread in `BrushLibrary::start_loading`, and `scan` in
[brush_library/import.rs](import.md).

### `app_thumbnail`
[src/brush_library/mod.rs:223](../../../src/brush_library/mod.rs#L223)

```rust
fn app_thumbnail(brush: BrushSettings) -> egui::ColorImage
```

**What it does.** Draws an app preset's thumbnail: a short S-stroke, dark on a transparent
background, on a `64 × 48` canvas. The colour is forced to near-black and the radius capped at 7
pixels so every preset fits.

**How it works.** Same idea as `krita_picture`, but with the app's normal `StrokeBuilder::new`
(whatever mode the preset uses), 41 samples, x from 6 px to `width - 6`, y wobbling by
`sin(6t) × 25%` of the height, the same pressure curve and 35° tilt, and no paper underneath.

**Used by:** the loader thread in `BrushLibrary::start_loading`.

### `scan_imported`
[src/brush_library/mod.rs:251](../../../src/brush_library/mod.rs#L251)

```rust
fn scan_imported(root: &Path) -> Vec<Entry>
```

**What it does.** Lists the imported presets: one `Entry` per sub-folder of `root` that holds a
`.kpp` file, sorted by name (case-insensitive).

**Returns.** Entries with id `user/<folder>`, group `Imported`, source
`Source::Folder(folder, kpp)`, status `Loading`. Empty if `root` cannot be read.

**How it works.**
1. Read `root`'s directory listing; give up quietly (empty list) if that fails.
2. Skip anything that is not a folder, and folders whose name starts with `.` — a
   `.import-<pid>` folder is an import still being written (see `install` in
   [import.rs](import.md)).
3. In each folder, find the first file whose name ends in `.kpp` (any case); skip the folder if
   there is none.
4. Build the entry; its name comes from the `.kpp` file name via `display_name`.

**Used by:** `BrushLibrary::new`, `BrushLibrary::rescan`.

## `impl BrushLibrary`

### `BrushLibrary::new`
[src/brush_library/mod.rs:285](../../../src/brush_library/mod.rs#L285)

```rust
pub fn new(root: Option<PathBuf>) -> Self
```

**What it does.** Builds the library and starts the loader thread.

**Parameters.**
- `root` — the brushes folder to read imported presets from; `None` means no imported presets
  (and nowhere to save edits).

**Returns.** A library whose App entries are `Ready` and whose Krita and Imported entries are
`Loading` until the thread reports.

**How it works.**
1. One `App` entry per `BrushSettings::PRESETS` item `(label, tooltip, builder)`, id
   `app/<label>`.
2. One `Krita` entry per shipped file in `defaults::PRESETS`, id `krita/<file without .kpp>`,
   tooltip "Krita's own …, from its default brushes."
3. If there is a `root`, append `scan_imported(root)`.
4. Empty caches, then `start_loading()`.

**Used by:** `AppState::with_prefs` in [app.rs](../app.md) (with
`config_dir()/brushes`); UI tests in [ui/shell.rs](../ui/shell.md).

### `BrushLibrary::start_loading`
[src/brush_library/mod.rs:324](../../../src/brush_library/mod.rs#L324)

```rust
fn start_loading(&mut self)
```

**What it does.** Starts a background thread that loads every entry still `Loading` and draws
every thumbnail not made yet.

**How it works.**
1. Collect the jobs: `(id, source)` for each entry that is `Loading` or has no thumbnail. They
   are cloned, because the thread must own its data (it may outlive this call).
2. Make a new channel and store its receiver in `incoming`. This replaces any earlier
   receiver; an older thread's next `send` then fails and that thread stops.
3. Spawn a thread named `brush-library`. For each job:
   - `Source::App(i)`: no brush (`Err("")`), thumbnail from `app_thumbnail` of that preset's
     builder function.
   - any Krita source: `load_source` (wrapped in `Arc`) and `kpp_thumbnail` of its `.kpp` bytes.
   - Send a `Loaded`; if the receiver is gone, return.
4. If the thread could not be spawned, set `incoming` to `None`.

**Example.** The same pattern in JS with a worker:
```js
const jobs = entries.filter(e => e.status === "loading" || !thumbs.has(e.id))
                    .map(e => [e.id, structuredClone(e.source)]);
const worker = new Worker("brush-loader.js");
worker.postMessage(jobs);                 // the thread owns its copy
worker.onmessage = (m) => inbox.push(m.data);   // read later by poll()
```

**Used by:** `BrushLibrary::new`, `BrushLibrary::rescan`.

### `BrushLibrary::poll`
[src/brush_library/mod.rs:356](../../../src/brush_library/mod.rs#L356)

```rust
pub fn poll(&mut self) -> bool
```

**What it does.** Takes in everything the loader thread has finished so far, without waiting.

**Returns.** `true` if anything arrived, so the caller can repaint.

**How it works.**
1. No receiver: return `false`.
2. `while let Ok(l) = rx.try_recv()` — loop while a message is waiting.
3. Store the thumbnail if there is one.
4. Find the entry by id. App entries stop here (they only wanted a thumbnail).
5. A brush: mark the entry `Ready` and add the brush to the cache — but only if no brush is
   cached under that id yet (`entry(..).or_insert`), so a brush already loaded on demand by
   `brush()` is kept. An error: mark it `Refused(why)`.

**Used by:** `AppState::update` in [app.rs](../app.md), every frame; tests.

### `BrushLibrary::loading`
[src/brush_library/mod.rs:380](../../../src/brush_library/mod.rs#L380)

```rust
pub fn loading(&self) -> bool
```

**What it does.** `true` while any entry is still `Loading`. The app keeps asking for repaints
(every 50 ms) while this is true so tiles appear as they load.

**Used by:** `AppState::update` in [app.rs](../app.md); the `settle` test helpers here and in
[ui/shell.rs](../ui/shell.md).

### `BrushLibrary::entry`
[src/brush_library/mod.rs:384](../../../src/brush_library/mod.rs#L384)

```rust
pub fn entry(&self, id: &str) -> Option<&Entry>
```

**What it does.** Finds the entry with this id (like `entries.find(e => e.id === id)`), or
`None`.

**Used by:** `brush`, `preset`, `brush_with`, `save_as_new`, `tips_for`; `pick` in
[ui/brush_picker.rs](../ui/brush_picker.md); `krita_editor` in
[ui/krita_editor.rs](../ui/krita_editor.md).

### `BrushLibrary::brush`
[src/brush_library/mod.rs:389](../../../src/brush_library/mod.rs#L389)

```rust
pub fn brush(&self, id: &str) -> Option<Arc<PixelBrush>>
```

**What it does.** The Krita brush for entry `id`. If the thread has not loaded it yet, it is
loaded right now (on the calling thread) and cached, so a stroke never has to wait for the
background loader.

**Returns.** A shared pointer to the brush, or `None` for an unknown id, an app preset, or a
preset that does not load.

**Used by:** `brush_with` (no overrides), `AppState::krita_brush` in [app.rs](../app.md),
`pick` in [ui/brush_picker.rs](../ui/brush_picker.md).

### `BrushLibrary::preset`
[src/brush_library/mod.rs:400](../../../src/brush_library/mod.rs#L400)

```rust
pub fn preset(&self, id: &str) -> Option<Arc<Preset>>
```

**What it does.** The parsed settings of entry `id`'s `.kpp` file, exactly as the file says
them (no overrides). Parsed once with `Preset::read`, then cached.

**Used by:** `brush_with`, `edited_preset`, `save_as_new`; `krita_editor` in
[ui/krita_editor.rs](../ui/krita_editor.md) (as the "base" preset that overrides are compared
against).

### `BrushLibrary::brush_with`
[src/brush_library/mod.rs:412](../../../src/brush_library/mod.rs#L412)

```rust
pub fn brush_with(&self, id: &str, overrides: &[(String, String)]) -> Result<Arc<PixelBrush>, String>
```

**What it does.** Entry `id`'s brush with the editor's overrides laid over its preset. An
*override* is a `(param name, new value)` pair; see [tools/krita/edit.rs](../tools/krita/edit.md).

**Parameters.**
- `id` — the library entry.
- `overrides` — the edits; empty means the preset as it came.

**Returns.** The built brush, or why the edited preset does not build (for example an invalid
tip definition). Errors are cached too.

**How it works.**
1. No overrides: return `brush(id)` (error text "no brush `id`" if it is missing).
2. Look in the `edited` cache for the same id and the same overrides; return that result.
3. Otherwise build: `preset(id).with_overrides(overrides)`, then `build_edited`.
4. Push the result onto `edited`; if it now holds more than 4, drop the oldest. A slider drag
   builds one brush per frame, and only the last few matter.

**Used by:** `AppState::krita_brush` in [app.rs](../app.md) (falls back to `brush(id)` if the
edit does not build); `krita_editor` in [ui/krita_editor.rs](../ui/krita_editor.md).

### `BrushLibrary::edited_preset`
[src/brush_library/mod.rs:434](../../../src/brush_library/mod.rs#L434)

```rust
pub fn edited_preset(&self, id: &str, overrides: &[(String, String)]) -> Option<Arc<Preset>>
```

**What it does.** Entry `id`'s preset with `overrides` applied: what the editor's controls
show. Only the last one asked for is cached, and it is rebuilt only when the id or overrides
change.

**Used by:** `krita_editor` in [ui/krita_editor.rs](../ui/krita_editor.md).

### `BrushLibrary::save_as_new`
[src/brush_library/mod.rs:447](../../../src/brush_library/mod.rs#L447)

```rust
pub fn save_as_new(&mut self, id: &str, overrides: &[(String, String)], name: &str) -> anyhow::Result<String>
```

**What it does.** "Save as new" in the Krita editor: writes entry `id` with its overrides as a
brand-new imported preset called `name`, carrying its tip inside the file, and adds it to the
library.

**Parameters.**
- `id` — the entry being edited.
- `overrides` — the edits to bake in.
- `name` — the new preset's name (trimmed); also used for its folder and file name.

**Returns.** The new entry's id (`user/<folder>`), or an error: no brushes folder, unknown id,
unreadable preset, missing tip, a brush that does not build, or a failed write.

**How it works.**
1. Needs a `root`, the entry's `source`, and its preset with overrides applied; set the
   preset's `name`.
2. Read the tip file name from the `brush_definition` param (its `<Brush filename="…">`
   attribute). If there is one and the preset does not already carry that tip as an embedded
   resource, fetch the tip bytes with `source_tip` and embed them (`kpp_write::embed_tip`), so
   the saved file does not depend on any other file.
3. Build the brush with `build_edited` (proves it paints) and paint its 128 px picture with
   `krita_picture`.
4. `kpp_write::write` produces the new `.kpp` bytes (PNG picture + XML settings).
5. The file name is `import::folder_name(name)` + `.kpp`; a `SourceInfo` records origin
   `edited from <id>` and the file's FNV hash.
6. `import::install` writes the folder (built aside, then renamed into place).
7. `rescan()` so the new entry appears, and return its id.

**Used by:** `krita_editor` in [ui/krita_editor.rs](../ui/krita_editor.md).

### `BrushLibrary::tips_for`
[src/brush_library/mod.rs:475](../../../src/brush_library/mod.rs#L475)

```rust
pub fn tips_for(&self, id: &str) -> Vec<String>
```

**What it does.** The tip file names an edit of entry `id` may switch to: every shipped tip
(`defaults::TIPS`), plus, for an imported preset, every `.png`/`.gbr`/`.gih` file in its folder
that is not already in the list.

**Used by:** `krita_editor` in [ui/krita_editor.rs](../ui/krita_editor.md) (the Tip
drop-down).

### `BrushLibrary::thumbnail`
[src/brush_library/mod.rs:489](../../../src/brush_library/mod.rs#L489)

```rust
pub fn thumbnail(&self, id: &str) -> Option<&egui::ColorImage>
```

**What it does.** The decoded thumbnail picture for entry `id`, if the loader has made it.

**Used by:** `sync_thumbs` in [ui/brush_picker.rs](../ui/brush_picker.md).

### `BrushLibrary::rescan`
[src/brush_library/mod.rs:494](../../../src/brush_library/mod.rs#L494)

```rust
pub fn rescan(&mut self)
```

**What it does.** Re-reads the brushes folder after an import or a Save as new.

**How it works.**
1. Remove every `Imported` entry, and every cached brush, preset, edited brush and thumbnail
   whose id starts with `user/`; clear the edited-preset cache. (A folder name can be reused, so
   old cached data must not survive.)
2. If there is a `root`, append a fresh `scan_imported(root)`.
3. `start_loading()` for the new entries.

**Used by:** `AppState::rescan_brushes` in [app.rs](../app.md), `save_as_new`.

### `BrushLibrary::root`
[src/brush_library/mod.rs:508](../../../src/brush_library/mod.rs#L508)

```rust
pub fn root(&self) -> Option<&Path>
```

**What it does.** Where imported presets live. `as_deref()` turns the stored
`Option<PathBuf>` into an `Option<&Path>` (a borrowed view), like returning a read-only
reference instead of a copy.

**Used by:** `AppState::open_krita_import` in [app.rs](../app.md) (imports go into the same
folder).

## Tests

Run with `cargo test brush_library::tests`.

| Test | What it checks |
|---|---|
| [`names_read_like_krita_s_picker`](../../../src/brush_library/mod.rs#L527) | `display_name` drops the `c)_` prefix and `.kpp`, and turns `_` into spaces. |
| [`the_library_lists_three_groups_and_loads_the_krita_ones`](../../../src/brush_library/mod.rs#L534) | All app presets and the 24 shipped Krita presets are listed; after loading, every Krita entry is `Ready` and app and Krita entries have thumbnails. |
| [`the_library_s_pencil5_is_the_built_in_one`](../../../src/brush_library/mod.rs#L557) | The shipped Pencil-5 preset loads to the same brush (diameter, wash) as the app's built-in `KritaPreset::Pencil5Tilted`; an unknown id gives `None`. |
| [`edits_build_and_save_as_new_presets`](../../../src/brush_library/mod.rs#L569) | Overrides change the brush without touching the unedited one, are built once (same `Arc`), show in `edited_preset`; a bad tip definition errors; Save as new writes `user/Soft wash` with its tip embedded (no loose `gradient.png`), and it loads as the edited brush with a thumbnail. |
| [`imported_presets_are_read_from_their_folders`](../../../src/brush_library/mod.rs#L597) | A preset copied into `<root>/Basic/` is listed as `user/Basic`, loads `Ready`; a hairy-brush preset is still listed after `rescan` but `Refused` with a reason naming `hairybrush`. |

**Test helpers:**
- `settle` ([line 517](../../../src/brush_library/mod.rs#L517)) — polls the library every 5 ms
  until nothing is `Loading` (at most 30 s).
- `write_hairy` ([line 547](../../../src/brush_library/mod.rs#L547), `pub(crate)`) — writes a
  `Hairy/Hairy.kpp` preset that uses Krita's hairy-brush engine, which this app refuses. Also
  used by tests in [ui/shell.rs](../ui/shell.md).

### `mod tests`

Run with `cargo test brush_library::tests` · 5 tests

| Test | What it checks |
|---|---|
| [`names_read_like_krita_s_picker`](../../../src/brush_library/mod.rs#L527) | Names read like krita s picker. |
| [`the_library_lists_three_groups_and_loads_the_krita_ones`](../../../src/brush_library/mod.rs#L534) | The library lists three groups and loads the krita ones. |
| [`the_library_s_pencil5_is_the_built_in_one`](../../../src/brush_library/mod.rs#L557) | The shipped Pencil-5 is the same preset as the app's built-in one, so its strokes hash the same. |
| [`edits_build_and_save_as_new_presets`](../../../src/brush_library/mod.rs#L569) | An edit paints differently, and Save as new keeps it: the saved preset carries its tip and loads as the edited brush. |
| [`imported_presets_are_read_from_their_folders`](../../../src/brush_library/mod.rs#L597) | Imported presets are read from their folders. |

**Test helpers** (used only by the tests above):

- [`settle`](../../../src/brush_library/mod.rs#L517) — Settle.
- [`write_hairy`](../../../src/brush_library/mod.rs#L547) — An imported preset this engine cannot paint: a Krita hairy brush.
