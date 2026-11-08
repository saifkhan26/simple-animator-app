# `src/io/kra/mod.rs` — reading and writing Krita .kra files

> **Source:** [src/io/kra/mod.rs](../../../../src/io/kra/mod.rs) · **Module:** `crate::io::kra` · **Size:** 1314 lines · Line links as of commit `67a7cb9`

## What this file is for

Krita's native file format `.kra` is a **zip archive** of XML descriptions plus binary pixel data. This
module writes the whole project as an *animated* Krita document and reads one back — used by
File → Export .kra, File → Import .kra, and the live "Edit in Krita" link.

```text
shot.kra  (a zip)
├── mimetype                         "application/x-krita" — stored uncompressed, must be first
├── maindoc.xml                      image size + the layer stack (names, opacity, visibility, groups…)
├── documentinfo.xml
├── mergedimage.png, preview.png     flattened picture / thumbnail
└── <image>/
    ├── animation/index.xml          fps, playback range, current time
    └── layers/
        ├── anim1.keyframes.xml      which drawing file shows from which frame
        ├── anim1.f0, anim1.f1 …     one tiled pixel file per drawing (see tiles.rs)
        ├── anim1.defaultpixel       the colour outside the tiles
        └── anim1.icc                colour profile (sRGB)
```

**Writing** (`write`) turns each layer into a `<layer>` element in `maindoc.xml`, each drawing into a
tile file, and each layer's keys into a keyframes file. **Reading** (`read_filtered`) walks
`maindoc.xml`'s layer tree, decodes each paint layer's drawings and keyframes into `KraDoc`/`KraLayer`
values, folds nested groups into one level, and collects warnings for things it can't carry (other layer
types, blend modes, masks).

**Carrying** (`carry`): Krita-only layers (names ending in `-x`, by the link's convention) are lifted out
of a Krita save *undecoded* — their XML and files byte for byte — so a re-send can put them back exactly
as Krita wrote them.

## How it connects

- **Used by:** [krita_link.rs](../../krita_link.md) (send/pull), [app.rs](../../app.md) (`import_kra`,
  `export_kra`, `on_kra_imported`), [krita_stress.rs](../../krita_stress.md).
- **Uses:** [io/kra/tiles.rs](tiles.md) (Krita's tile compression), [io/kra/xml.rs](xml.md) (XML
  templates/escaping), [doc/project.rs](../../doc/project.md), [doc/canvas.rs](../../doc/canvas.md),
  the `zip`, `roxmltree` and `image` crates.

## Rust concepts in this file

**Structs that hold references need lifetimes.** `WriteOpts<'a>` holds `&'a [String]` slices borrowed from
the caller instead of owning copies: "these borrowed slices live at least as long as the options value".
More: [Lifetimes](../../../rust-primer.md#lifetimes)

**Trait objects as filter callbacks.** `keep: &dyn Fn(&str, &[String]) -> bool` — "any function from (layer
name, enclosing group names) to bool". The caller decides which layers come back.
More: [Generics](../../../rust-primer.md#generics)

**Nested helper functions.** `carry` and `read_filtered` each contain an inner `fn walk(...)` that recurses
through the XML tree, plus a small `struct Walk` for the state it carries — a local recursive helper with
a context object.

**Reading a zip from memory.** `ZipArchive::new(Cursor::new(bytes))` — `Cursor` makes a `Vec<u8>` behave like
a file, similar to wrapping a buffer in a stream.

## Types

### `const IMAGE`
[src/io/kra/mod.rs:36](../../../../src/io/kra/mod.rs#L36)

```rust
const IMAGE: &str = "image";
```

The folder name used inside the zip for this document's data (Krita reads it from the image's `name`).

### `const MIMETYPE`
[src/io/kra/mod.rs:37](../../../../src/io/kra/mod.rs#L37)

```rust
const MIMETYPE: &[u8] = b"application/x-krita";
```

The text of the `mimetype` file: `application/x-krita`.

### `const SRGB_ICC`
[src/io/kra/mod.rs:43](../../../../src/io/kra/mod.rs#L43)

```rust
const SRGB_ICC: &[u8] = include_bytes!("srgb-built-in.icc");
```

The sRGB colour profile bytes embedded with every layer and the image (byte-for-byte what Krita embeds for
"sRGB built-in"), since this app's pixels are sRGB.

### `struct WriteOpts`
[src/io/kra/mod.rs:45](../../../../src/io/kra/mod.rs#L45)

Options for `write`: Krita uuids to give the layers, which layer Krita opens selected, and carried layers
to put back.

| Field | Type | Meaning |
|---|---|---|
| `layer_uuids` | `&'a [String]` | One `{uuid}` per project layer, bottom first. |
| `selected_layer` | `usize` | The layer Krita opens with selected. |
| `carried` | `&'a [Carried]` | Layers of the file being replaced that go back in untouched (see `carry`). |

### `const OUR_PREFIX`
[src/io/kra/mod.rs:58](../../../../src/io/kra/mod.rs#L58)

```rust
const OUR_PREFIX: &str = "anim";
```

Prefix for the layer file names this app writes (`anim1`, `anim2`, …), different from Krita's `layer<N>`, so
carried Krita layers can't clash.

### `struct Carried`
[src/io/kra/mod.rs:64](../../../../src/io/kra/mod.rs#L64)

A layer — or a whole group — of a Krita file, carried untouched into a rewrite of that file: its `<layer>` element exactly as Krita wrote it, and every file under `layers/` it names (pixels, frames, keyframes, profile, vector shapes, masks…). Nothing is decoded, so any layer type survives.

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | The layer's (or group's) name. |
| `xml` | `String` | Its `<layer>` element exactly as Krita wrote it. |
| `files` | `Vec<(String, Vec<u8>)>` | `(path under layers/, bytes)`. |
| `below` | `Vec<String>` | Uuids of the layers and groups under it in Krita's stack, nearest first (a group before what's in it): it goes back in just above the first of them the rewrite still has beside it. |
| `group` | `Option<String>` | The uuid of the group it sat in — the outermost one, as groups fold on reading — so it goes back inside it while the rewrite has it. |

### `enum Seen`
[src/io/kra/mod.rs:95](../../../../src/io/kra/mod.rs#L95)

Inside `carry`: for each layer met during the walk, whether it was carried (index into the output) or kept
(its uuid).

| Variant | Meaning |
|---|---|
| `Carried(usize)` | Lifted out; index into the carried list. |
| `Kept(String)` | Stays with the app; its uuid. |

### `struct Walk`
[src/io/kra/mod.rs:99](../../../../src/io/kra/mod.rs#L99)

State carried through the recursive XML walk (one version in `carry`, one in `read_filtered`): the filter,
the stack of enclosing group names, and what has been found so far.

| Field | Type | Meaning |
|---|---|---|
| `keep` | `&'a dyn Fn(&str, &[String]) -> bool` | The filter deciding which layers the app keeps. |
| `src` | `&'a str` | The `maindoc.xml` text (for slicing out a layer's exact XML). |
| `groups` | `Vec<String>` | Names of the groups the walk is inside, outermost first. |
| `group_uuids` | `Vec<String>` | The uuid of each group in `groups`. |
| `seen` | `Vec<Seen>` | Per layer met: carried or kept. |
| `out` | `Vec<(String, String, Vec<String>, Option<String>)>` | Carried layers found: (name, xml, files below, group uuid). |

### `struct Drawing`
[src/io/kra/mod.rs:383](../../../../src/io/kra/mod.rs#L383)

One Krita drawing decoded: the bounding box of its tiles in Krita canvas coordinates, its RGBA pixels, and
the default pixel used outside the box.

| Field | Type | Meaning |
|---|---|---|
| `x` | `i32` | Left of the tile box, in Krita canvas pixels. |
| `y` | `i32` | Top of the tile box. |
| `w` | `u32` | Box width. |
| `h` | `u32` | Box height. |
| `pixels` | `Vec<u8>` | RGBA8 pixels of the box. |
| `default` | `[u8; 4]` | The colour everywhere outside the box (usually transparent; a Krita "fill layer" background is a non-transparent default). |

### `struct KraLayer`
[src/io/kra/mod.rs:429](../../../../src/io/kra/mod.rs#L429)

One layer read from a `.kra`: identity, attributes, its drawings and keys, group flags.

| Field | Type | Meaning |
|---|---|---|
| `uuid` | `String` | Krita's layer uuid. |
| `name` | `String` | Layer name. |
| `opacity` | `u8` | Krita's 0–255 opacity. |
| `visible` | `bool` | Visibility. |
| `locked` | `bool` | Lock flag. |
| `drawings` | `Vec<Drawing>` | Its decoded drawings. |
| `keys` | `Vec<(usize, usize)>` | `(time, index into drawings)`, sorted by time. |
| `group` | `bool` | A group's header — no drawings — with its layers listed just below it in `KraDoc::layers`. |
| `in_group` | `bool` | One of a group's layers. Groups inside groups are folded into the outermost one. |
| `collapsed` | `bool` | A group shown folded in Krita. |

### `struct KraDoc`
[src/io/kra/mod.rs:449](../../../../src/io/kra/mod.rs#L449)

A decoded `.kra`: size, fps, playback range end, layers (bottom first), warnings and held-back layers.

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | Image width. |
| `height` | `u32` | Image height. |
| `fps` | `Option<f32>` | Frame rate, if the file has animation info. |
| `range_end` | `Option<usize>` | Last frame of Krita's playback range. |
| `layers` | `Vec<KraLayer>` | Layers, bottom first: paint layers, and a group's header just above its layers. |
| `warnings` | `Vec<String>` | Things read but not carried over — unsupported layer types, blend modes, masks — worded for a toast. |
| `held` | `Vec<(String, String)>` | `(uuid, name)` of paint layers the caller's filter kept out (see `read_filtered`). Not decoded. |

### `struct Reader`
[src/io/kra/mod.rs:466](../../../../src/io/kra/mod.rs#L466)

A small wrapper around the open zip with helpers for reading entries.

| Field | Type | Meaning |
|---|---|---|
| `zip` | `ZipArchive<Cursor<Vec<u8>>>` | The zip archive, read from memory. |

### `struct Found`
[src/io/kra/mod.rs:549](../../../../src/io/kra/mod.rs#L549)

Inside `read_filtered`: one paint layer or group found in `maindoc.xml`, before its pixels are decoded.

| Field | Type | Meaning |
|---|---|---|
| `uuid` | `String` | Krita uuid. |
| `name` | `String` | Name. |
| `filename` | `String` | Its file name under `layers/`. |
| `opacity` | `u8` | 0–255, combined with folded parent groups. |
| `visible` | `bool` | Visibility, combined with parents. |
| `locked` | `bool` | Lock flag. |
| `x` | `i32` | Layer offset x in Krita. |
| `y` | `i32` | Layer offset y in Krita. |
| `keyframes` | `Option<String>` | Its keyframes file, if animated. |
| `group` | `bool` | Is a group header. |
| `in_group` | `bool` | Is inside a group. |
| `collapsed` | `bool` | Group shown folded. |

### `struct Walk`
[src/io/kra/mod.rs:563](../../../../src/io/kra/mod.rs#L563)

State carried through the recursive XML walk (one version in `carry`, one in `read_filtered`): the filter,
the stack of enclosing group names, and what has been found so far.

| Field | Type | Meaning |
|---|---|---|
| `keep` | `&'a dyn Fn(&str, &[String]) -> bool` | The filter deciding which layers the app keeps. |
| `groups` | `Vec<String>` | Names of the groups the walk is inside, outermost first. |
| `found` | `Vec<Found>` | Layers to decode. |
| `warnings` | `Vec<String>` | Messages about unsupported content. |
| `held` | `Vec<(String, String)>` | Layers rejected by the filter: (uuid, name). |

## Free functions

### `carry`
[src/io/kra/mod.rs:81](../../../../src/io/kra/mod.rs#L81)

```rust
pub fn carry(bytes: Vec<u8>, keep: &dyn Fn(&str, &[String]) -> bool) -> Result<Vec<Carried>>
```

Lifts the layers that `keep` rejects (Krita-only `-x` layers) out of a Krita file whole: their `<layer>` XML
and every file under `layers/` they reference, undecoded, plus where in the stack and which group they sat
in, so a rewrite can put them back exactly.

#### `carry::walk`
[src/io/kra/mod.rs:108](../../../../src/io/kra/mod.rs#L108)

```rust
fn walk(layers: roxmltree::Node, w: &mut Walk)
```

Recursive walk over `maindoc.xml`'s `<layers>` element for `carry`, descending into groups and recording each
layer as carried or kept.

### `cell_origin`
[src/io/kra/mod.rs:176](../../../../src/io/kra/mod.rs#L176)

```rust
pub fn cell_origin(pw: u32, ph: u32, cw: u32, ch: u32) -> (i32, i32)
```

The top-left (in frame coordinates) of a `cw`×`ch` cell centred on a `pw`×`ph` frame — floored so an odd size
difference lands on the same pixel both directions of the round trip.

### `write`
[src/io/kra/mod.rs:184](../../../../src/io/kra/mod.rs#L184)

```rust
pub fn write(project: &Project, opts: &WriteOpts) -> Result<Vec<u8>>
```

Serialises the project as an animated `.kra` (bytes of a zip):
1. writes `mimetype` uncompressed first (Krita requires it);
2. builds `maindoc.xml` with one `<layer>` per layer (paint layers and group headers, names, opacity,
   visibility, lock, uuids from `opts`), inserting carried Krita layers back where they belong;
3. per paint layer: each distinct drawing as a tile file (via [tiles.rs](tiles.md)), a keyframes XML
   mapping frames to drawing files, the default pixel and the sRGB profile;
4. `animation/index.xml` with fps, range and current frame; a flattened `mergedimage.png` and preview.

### `png_bytes`
[src/io/kra/mod.rs:370](../../../../src/io/kra/mod.rs#L370)

```rust
fn png_bytes(img: &image::RgbaImage) -> Result<Vec<u8>>
```

Encodes an RGBA image as PNG bytes (for the merged image and preview inside the zip).

### `opacity_u8`
[src/io/kra/mod.rs:376](../../../../src/io/kra/mod.rs#L376)

```rust
pub fn opacity_u8(o: f32) -> u8
```

Converts a 0–1 opacity to Krita's 0–255.

## `impl Drawing`

### `Drawing::place`
[src/io/kra/mod.rs:396](../../../../src/io/kra/mod.rs#L396)

```rust
pub fn place(&self, cx: i32, cy: i32, cw: u32, ch: u32) -> (Canvas, bool)
```

Rasterises the drawing into a `cw`×`ch` cell whose top-left sits at Krita canvas `(cx, cy)`. Returns the
canvas and whether any visible paint fell outside the cell and was cut off (reported as a warning).

## `impl Reader`

### `Reader::bytes`
[src/io/kra/mod.rs:471](../../../../src/io/kra/mod.rs#L471)

```rust
fn bytes(&mut self, name: &str) -> Result<Vec<u8>>
```

Reads a zip entry fully; an error if it's missing.

### `Reader::opt_bytes`
[src/io/kra/mod.rs:478](../../../../src/io/kra/mod.rs#L478)

```rust
fn opt_bytes(&mut self, name: &str) -> Result<Option<Vec<u8>>>
```

Reads a zip entry if present (`Ok(None)` when absent).

### `Reader::text`
[src/io/kra/mod.rs:485](../../../../src/io/kra/mod.rs#L485)

```rust
fn text(&mut self, name: &str) -> Result<String>
```

Reads a zip entry as UTF-8 text.

### `Reader::default_pixel`
[src/io/kra/mod.rs:490](../../../../src/io/kra/mod.rs#L490)

```rust
fn default_pixel(&mut self, name: &str) -> Result<[u8; 4]>
```

Reads a layer's default-pixel file (stored as BGRA) and returns it as RGBA; transparent if absent.

### `Reader::drawing`
[src/io/kra/mod.rs:497](../../../../src/io/kra/mod.rs#L497)

```rust
fn drawing(&mut self, file: &str, fallback_default: [u8; 4], dx: i32, dy: i32) -> Result<Drawing>
```

Reads and decodes one drawing's tile file into a `Drawing`, offset by the layer position `(dx, dy)`.

## Free functions

### `attr_i32`
[src/io/kra/mod.rs:510](../../../../src/io/kra/mod.rs#L510)

```rust
fn attr_i32(n: roxmltree::Node, name: &str) -> i32
```

An integer XML attribute, 0 if missing or invalid.

### `read`
[src/io/kra/mod.rs:517](../../../../src/io/kra/mod.rs#L517)

```rust
pub fn read(bytes: Vec<u8>) -> Result<KraDoc>
```

Reads a `.kra` with every layer kept (tests use this; the app always filters).

### `read_filtered`
[src/io/kra/mod.rs:525](../../../../src/io/kra/mod.rs#L525)

```rust
pub fn read_filtered(bytes: Vec<u8>, keep: &dyn Fn(&str, &[String]) -> bool) -> Result<KraDoc>
```

Reads a `.kra`, keeping only layers `keep(name, groups)` accepts (a rejected group takes its contents along).
Walks `maindoc.xml`, decodes each kept paint layer's drawings and keyframes into `KraLayer`s, reads fps and
range from `animation/index.xml`, folds nested groups into one level (passing visibility and opacity down),
lists rejected layers in `held` without decoding them, and collects warnings for unsupported content.

#### `read_filtered::walk`
[src/io/kra/mod.rs:573](../../../../src/io/kra/mod.rs#L573)

```rust
fn walk(
    layers: roxmltree::Node,
    visible: bool,
    opacity: f32,
    holding: bool,
    dx: i32,
    dy: i32,
    w: &mut Walk,
) -> Result<()>
```

The recursive walk for `read_filtered`: visits each `<layer>`, decides keep/hold, records paint layers and
group headers as `Found`, and adds warnings for unsupported layer types, blend modes and masks.

## Tests

### `mod tests`

Run with `cargo test io::kra::tests` · 16 tests

| Test | What it checks |
|---|---|
| [`write_then_read_keeps_keys_holds_repeats_and_pixels`](../../../../src/io/kra/mod.rs#L819) | Write then read keeps keys holds repeats and pixels. |
| [`reads_krita_written_file_byte_identical`](../../../../src/io/kra/mod.rs#L889) | The reader against a file Krita itself wrote: `fixture_project` went through our writer, then `krita --export resaved.kra` (Krita 5.2.9). Regenerate with `KRA_FIXTURE_DIR=… cargo test kra_fixture -- --ignored` and that export. Krita re-encodes every tile (LZF, channel-planar), renames the frame files and keeps a cloned keyframe — every drawing must still come back byte-identical, or the link would see changes that were never made. |
| [`carries_a_krita_written_layer_into_a_rewrite`](../../../../src/io/kra/mod.rs#L920) | Carry a layer out of a file Krita itself wrote — Krita's folder name, Krita's `layerN` file names, LZF tiles — into a rewrite that doesn't have it: it goes back in the same place, drawing for drawing. |
| [`carries_a_whole_group_with_its_files`](../../../../src/io/kra/mod.rs#L954) | A rejected group comes out whole — any layer type inside — with the files its layers name, and only those (`layer9…`, not `layer90`). |
| [`kra_carry_src`](../../../../src/io/kra/mod.rs#L1013) | Not a unit test: step 1 of the carry check — writes `carry_src.kra` (the project with `sketch-x`) into `$KRA_FIXTURE_DIR`. Then `krita --export --export-filename carry_krita.kra carry_src.kra`. |
| [`a_group_round_trips_through_kra`](../../../../src/io/kra/mod.rs#L1038) | A group round trips through kra. |
| [`carried_layers_keep_their_side_of_a_group`](../../../../src/io/kra/mod.rs#L1067) | Krita-only layers beside a group go back on the same side of its edge: one just above the group stays out of it, one at the bottom of it stays in. |
| [`a_nested_krita_group_folds_into_the_outer`](../../../../src/io/kra/mod.rs#L1109) | Krita groups can nest; ours can't. A group inside a group folds into the outer one, its own visibility and opacity going into its layers. |
| [`kra_group_src`](../../../../src/io/kra/mod.rs#L1147) | Not a unit test: writes `group_src.kra` (see `grouped_project`) into `$KRA_FIXTURE_DIR`, for checking against Krita: `krita --export-sequence --export-filename seq.png group_src.kra` should match our own flatten, and `krita --export --export-filename group_krita.kra group_src.kra` should read back with its group. |
| [`kra_group_back`](../../../../src/io/kra/mod.rs#L1173) | Not a unit test: reads Krita's re-save of `group_src.kra` and checks the group came through. |
| [`kra_carry_out`](../../../../src/io/kra/mod.rs#L1214) | Not a unit test: step 2 — carries `sketch-x` out of Krita's `carry_krita.kra` into a rewrite of the project *without* it, `carry_out.kra`. Krita should render that exactly like `carry_src.kra`. |
| [`kra_fixture`](../../../../src/io/kra/mod.rs#L1229) | Not a unit test: writes `ours.kra` plus our own flattened frames (`ours_NNNN.png`) into `$KRA_FIXTURE_DIR`, to check the writer against a real Krita (`krita --export-sequence`, `krita --export`). |
| [`first_drawing_lives_in_the_main_file_next_to_a_profile`](../../../../src/io/kra/mod.rs#L1248) | Krita loads a single-keyframe layer from the layer's main file alone, and reports a layer without its own `.icc` as a load problem — both found the hard way in the Krita GUI, which the CLI export stays quiet about. |
| [`place_reports_paint_outside_the_cell`](../../../../src/io/kra/mod.rs#L1269) | Place reports paint outside the cell. |
| [`a_rejected_group_holds_its_layers_undecoded`](../../../../src/io/kra/mod.rs#L1279) | A group the filter rejects takes its layers along: they come back as `held`, undecoded — this file has no pixel data for them at all. |
| [`rejects_non_rgba_colour_space`](../../../../src/io/kra/mod.rs#L1304) | Rejects non rgba colour space. |

**Test helpers** (used only by the tests above):

- [`cell`](../../../../src/io/kra/mod.rs#L771) — A cell with a few coloured pixels, distinct per `seed`.
- [`uuids`](../../../../src/io/kra/mod.rs#L781) — Uuids.
- [`sample_project`](../../../../src/io/kra/mod.rs#L787) — Two layers: one with a key, a hold and a repeated cell; one with its first key late and a bigger-than-frame cell.
- [`read_cell`](../../../../src/io/kra/mod.rs#L811) — The frame-space placement read back into a cell of the original size.
- [`fixture_project`](../../../../src/io/kra/mod.rs#L851) — `sample_project` with drawings big enough to see — the project behind `testdata/krita-5.2.9-resaved.kra`.
- [`with_sketch`](../../../../src/io/kra/mod.rs#L990) — `fixture_project` with a Krita-only `sketch-x` layer between Ink and Big, and the uuids to write it with (`uuids(3)` for the fixture's own layers, in order, so a rewrite without the sketch lines up).
- [`grouped_project`](../../../../src/io/kra/mod.rs#L1022) — `sample_project` with its two layers in a half-opaque, folded group under a third layer.
- [`with_maindoc`](../../../../src/io/kra/mod.rs#L1089) — Rewrite `bytes`' `maindoc.xml` with `f`, every other file as it was.

**Test-only types and constants:** [`SKETCH_UUID`](../../../../src/io/kra/mod.rs#L985)
