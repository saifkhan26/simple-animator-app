# `src/io/project_file.rs` — saving and opening `.anim` projects

> **Source:** [src/io/project_file.rs](../../../src/io/project_file.rs) · **Module:** `crate::io::project_file` · **Size:** 851 lines · Line links as of commit `36405ad`

## What this file is for

This is Animator's native **Save** and **Open**. A whole project — every cell (drawing),
every layer with its exposures (which drawing shows on which frame), the timeline length,
the camera and its keyframes — is turned into bytes and written to one `.anim` file, and
read back again later.

The recipe has three layers, like an envelope inside an envelope:

1. **A tiny header** — the 4 bytes `ANIM` (a "magic number" so a random file is rejected
   straight away) and a 4-byte version number, so files written by older builds can still
   be recognised and opened.
2. **zlib compression** (the [`flate2`](https://docs.rs/flate2) crate). Cells are RGBA8
   pixels and line art is mostly transparent — four zero bytes per empty pixel — so the
   data is long runs of zeros and shrinks by one to two orders of magnitude. zlib is
   lossless, so pixels come back bit-identical.
3. **postcard** — a compact binary format for [serde](../../rust-primer.md#serde-and-file-formats).
   serde is Rust's standard "turn a struct into data and back" system; postcard is one
   output format for it. If you know `JSON.stringify` / `JSON.parse`, it is the same idea,
   but binary and without field names (see the byte sketch below).

Every older format version (2 to 7) can still be opened. Versions 2–5 were not
compressed; 6 and 7 were compressed but had fewer layer fields. The old layouts live in
[legacy.rs](legacy.md) as frozen "mirror" structs, and this file picks the right one
from the version number.

Loading **streams**: the decompressor feeds postcard directly, and pixel data goes
through one reused scratch buffer, so the app never holds the whole uncompressed file
and the finished project in memory at the same time.

## How it connects

- **Used by:**
  - [app.rs](../app.md) — `save_project` / `save_project_as` / `write_project` call
    [`ask_save_path`](#ask_save_path) and run [`save_to`](#save_to)
    on a background thread; the `Action::OpenProject` shortcut calls
    [`load_dialog`](#load_dialog) and hands the result to `load_project`;
    the `perf_big_canvas_stroke` probe test calls [`load_from`](#load_from).
  - [ui/shell.rs](../ui/shell.md) — the File menu's "Open project…" item calls
    [`load_dialog`](#load_dialog).
  - [webp_export.rs](webp_export.md) — a test loads a real project with
    [`load_from`](#load_from).
- **Uses:**
  - [doc/project.rs](../doc/project.md) — the `Project` struct that is saved, and
    `normalize_groups` (from [doc/groups.rs](../doc/groups.md)) run after loading.
  - [doc/canvas.rs](../doc/canvas.md) — `Canvas` has a hand-written serde
    implementation that writes each cell's pixels as one byte string.
  - [legacy.rs](legacy.md) — `ProjectV2` … `ProjectV7`, the old layouts.
  - [composite.rs](composite.md) — only in tests (`pins::scene`, `pins::fnv`).
  - External crates: `flate2` (zlib), `postcard` + `serde` (encoding), `rfd` (native file
    dialogs), `anyhow` (errors), `log`.

```text
SAVE                                             OPEN
app.rs save_project / save_project_as            app.rs Action::OpenProject, ui/shell.rs File menu
  └ ask_save_path() ─ rfd "Save" dialog            └ load_dialog() ─ rfd "Open" dialog
  └ write_project(): clone Project,                    └ load_from(path)
      spawn a thread ──► save_to(&project, path)            └ File ─► BufReader ─► decode_from()
                          └ encode()                             ├ check "ANIM", read version
                             header ─► ZlibEncoder               ├ v8:    stream::<Project>
                                ◄─ BufWriter                     ├ v7/v6: stream::<ProjectV7/V6>().into()
                                ◄─ postcard::to_io               ├ v2–v5: decode_uncompressed()
                          └ std::fs::write, log ratio            └ normalize_groups()
  poll_save_job(): remember path + toast,          └ AppState::load_project(project, Some(path))
      or show the error
```

## The `.anim` file, byte by byte

```text
offset  size   content
------  -----  ---------------------------------------------------------------
0       4      41 4E 49 4D            "ANIM"  (MAGIC)
4       4      08 00 00 00            version 8 as a little-endian u32 (VERSION)
8       rest   zlib stream, level 1 (versions 6, 7, 8)
               ├ 2-byte zlib header
               ├ deflate-compressed data   ── inflates to the postcard body ↓
               └ 4-byte Adler-32 checksum
               (versions 2–5: the postcard body sits here uncompressed)
```

The postcard body is the `Project` struct's fields **in the order they are declared in
the Rust source**, with no names and no type tags. Integers bigger than one byte
(`u32`, `usize`) are *varints* — 7 bits per byte, high bit set means "another byte
follows" — so small numbers take one byte. `f32` is 4 little-endian bytes, `bool` is one
byte `00`/`01`, a `String` or `Vec` is a varint length followed by its items, an
`Option` is `00` (None) or `01` followed by the value, and an enum is a varint variant
index.

```text
Project (format v8)
  width                     varint            e.g. 1920 → 80 0F
  height                    varint
  fps                       f32               e.g. 12.0 → 00 00 40 41
  cells                     varint count, then per cell (Canvas):
      width, height           varint, varint
      pixels                  varint length (= width*height*4), then that many raw RGBA8 bytes
  layers                    varint count, then per layer (Layer):
      name                    varint length + UTF-8 bytes
      opacity                 f32
      visible, locked, reference   1 byte each
      exposures               varint count; each: 00 (no drawing) | 01 varint cell id
      transform               4 × f32 (tx, ty, scale, rot)
      transform_keys          varint count; each: varint frame, 4 × f32, varint ease
      track_points            varint count; each: two Option<[f32; 2]>
      cell_w, cell_h          varint, varint                   (v4+)
      clip, alpha_lock        1 byte each                      (v7+)
      kind, in_group, collapsed   varint enum, 1 byte, 1 byte  (v8+)
  frame_count, current_frame, current_layer, loop_start, loop_end   varint each
  camera                    4 × f32 (tx, ty, zoom, rot)         (v4+)
  camera_keys               varint count; each: varint frame, 4 × f32, varint ease
```

Fields marked `#[serde(skip)]` in the source are not in the file at all: on `Layer`
that is `lines_from`, `onion_pins`, `onion`, `keys_from` and `uid`; on `Project` it is
`camera_opts`. They are "session-only" and come back as defaults after a load. A cell's
pixels are always written as the full `width × height × 4` bytes even when the cell is
stored packed (cropped) in memory; the zeros outside the drawing are what zlib squeezes
away. A freed cell ("tombstone") is 0×0 and writes no pixel bytes.

**Why the version number matters so much.** Because the body has no field names, the
reader must know the exact field list the writer used. Adding a field to `Layer` shifts
every byte after it. So each time a field is appended, `VERSION` goes up and the old
layout is frozen in [legacy.rs](legacy.md). The version history is written in a comment
above `VERSION`:

| Version | What changed |
|---|---|
| 1 | Original layout. No longer readable ("rejected cleanly"). |
| 2 | Per-layer `transform` + `transform_keys`. |
| 3 | Per-layer stabilization `track_points`. |
| 4 | Project `camera` + `camera_keys`; per-layer `cell_w` / `cell_h`. |
| 5 | `ease` on each layer transform key. |
| 6 | Same postcard layout as v5, but the body is now zlib-compressed. |
| 7 | `clip` and `alpha_lock` on each layer. |
| 8 | `kind`, `in_group`, `collapsed` on each layer (layer groups). Current. |

## Rust concepts in this file

**serde + postcard instead of `JSON.stringify` / `JSON.parse`.** A struct that says
`#[derive(serde::Serialize, serde::Deserialize)]` (like `Project`) can be written to and
read from any serde format. postcard is the binary one used here.
```rust
let bytes: Vec<u8> = postcard::to_stdvec(&project)?;          // like JSON.stringify
let back: Project  = postcard::from_bytes(&bytes)?;           // like JSON.parse
```
```js
const text = JSON.stringify(project);   // {"width":4,"height":3,...} - names included
const back = JSON.parse(text);          // any shape; no check it matches
```
Two big differences: postcard writes no field names (so it is small, but the layout is
fixed by field order), and `from_bytes::<Project>` checks the data really is a `Project`
— a missing byte or wrong length is an error, not a half-filled object.
More: [Serde and file formats](../../rust-primer.md#serde-and-file-formats)

**`Result`, `?` and `anyhow` context.** Every step that can fail returns
`Result<T>`. `?` means "if this is an error, return it from this function now";
`.context("…")` wraps the error with a human sentence, the way you would re-throw with a
better message in JS.
```rust
let file = std::fs::File::open(path).with_context(|| format!("reading {path:?}"))?;
```
```js
let file;
try { file = fs.openSync(path); } catch (e) { throw new Error(`reading ${path}`, { cause: e }); }
```
`bail!("…")` is `return Err(anyhow!("…"))` — like `throw new Error("…")`.
More: [Option and Result](../../rust-primer.md#option-and-result)

**Byte strings and little-endian numbers.** `b"ANIM"` is a byte array (`&[u8; 4]`), not
a text string. `VERSION.to_le_bytes()` turns a `u32` into its 4 bytes, lowest byte first.
```rust
const MAGIC: &[u8; 4] = b"ANIM";
out.extend_from_slice(&VERSION.to_le_bytes());     // 8 → [8, 0, 0, 0]
```
```js
const MAGIC = new TextEncoder().encode("ANIM");
const v = new Uint8Array(4); new DataView(v.buffer).setUint32(0, 8, true); // true = little-endian
```
More: [Numbers and casts](../../rust-primer.md#numbers-and-casts)

**`impl Read` arguments: "anything you can read bytes from".** `decode_from(input: impl
Read)` accepts a file, a byte slice, a decompressor… anything that implements the
`Read` trait. It is like a JS function that accepts any object with a `read()` method
(duck typing), except the compiler checks it.
```rust
fn decode_from(mut input: impl Read) -> Result<Project> { /* … */ }
decode_from(&bytes[..])?;                          // a slice is Read
decode_from(BufReader::new(file))?;                // so is a buffered file
```
More: [Generics](../../rust-primer.md#generics)

**Implementing someone else's trait for your own type.** postcard reads input through a
trait called `Flavor`. This file writes `impl Flavor for Stream<R>` so postcard can pull
bytes from a stream. It is like handing a library an object that fulfils an interface
(`{ pop(), takeN(), finalize() }`).
```rust
impl<'de, R: Read + 'de> postcard::de_flavors::Flavor<'de> for Stream<R> {
    fn pop(&mut self) -> postcard::Result<u8> { /* read one byte */ }
}
```
The `'de` and `'a` names are *lifetimes* — compile-time labels for "how long a borrowed
slice stays valid". Here they say the slice handed back by `try_take_n_temp` only lives
until the next call.
More: [Traits and derive](../../rust-primer.md#traits-and-derive),
[Lifetimes](../../rust-primer.md#lifetimes)

**`let … else` for an early return.** In `load_dialog`, `let Some(path) = … else { return
Ok(None); };` means "if the dialog gave a path, bind it; otherwise leave the function".
```js
const path = pickFile();
if (path == null) return null;
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

## impl `Stream<R>`

## impl `postcard::de_flavors::Flavor<'de> for Stream<R>`

These four methods are the interface postcard calls to get bytes. The associated types
`Remainder` and `Source` are both `R`: when parsing ends, the reader is what is left over.

## Types

### `const MAGIC`
[src/io/project_file.rs:22](../../../src/io/project_file.rs#L22)

```rust
const MAGIC: &[u8; 4] = b"ANIM";
```

`&[u8; 4] = b"ANIM"`. The first four bytes of every `.anim` file. A file that does not
start with them is refused with "not an Animator project file".

### `const VERSION`
[src/io/project_file.rs:34](../../../src/io/project_file.rs#L34)

```rust
const VERSION: u32 = 8;
```

`u32 = 8`. The format version this build writes. The comment above it is the version
history (see the table above). A test in `pins` fails if it changes, so a format bump is
always deliberate.

### `const EXT`
[src/io/project_file.rs:35](../../../src/io/project_file.rs#L35)

```rust
const EXT: &str = "anim";
```

`&str = "anim"`. The file extension used in the Save / Open dialog filters.

### `const BUF`
[src/io/project_file.rs:49](../../../src/io/project_file.rs#L49)

```rust
const BUF: usize = 256 * 1024;
```

`usize = 256 * 1024` (256 KiB). Size of the in-memory buffer placed between postcard
and the compressor (on save) and between the file / decompressor and postcard (on
load). The comment says it is "generous on purpose": one allocation per save, and it is
the difference between a save you notice and one you do not.

### `struct Stream`
[src/io/project_file.rs:133](../../../src/io/project_file.rs#L133)

A postcard *flavor* (input adapter) that reads from any `Read` stream instead of from a
byte slice already in memory. The `<R>` means it works for any reader type.

| Field | Type | Meaning |
|---|---|---|
| `input` | `R` | Where bytes come from — on load, a buffered zlib decompressor. |
| `scratch` | `Vec<u8>` | One reusable buffer. Each cell's pixel bytes are read into it, then turned into a `Canvas`; the next cell reuses it. So memory holds at most the project being built plus the largest single cell. |

### `const MAX_BYTES`
[src/io/project_file.rs:150](../../../src/io/project_file.rs#L150)

```rust
const MAX_BYTES: usize = 16384 * 16384 * 4;
```

`usize = 16384 * 16384 * 4` (1 GiB). The longest byte string a file may claim. A
damaged length field larger than this is treated as an error rather than an attempt to
allocate that much memory. 16384² is the largest texture a GPU takes.

### `type Stream::Remainder`
[src/io/project_file.rs:153](../../../src/io/project_file.rs#L153)

```rust
type Remainder = R;
```

postcard's `Flavor` trait asks what is left over when decoding ends; for `Stream` it is the reader `R` itself, handed back.

### `type Stream::Source`
[src/io/project_file.rs:154](../../../src/io/project_file.rs#L154)

```rust
type Source = R;
```

The other associated type of postcard's `Flavor`: where bytes come from — the same reader `R`.

## Free functions

### `level`
[src/io/project_file.rs:42](../../../src/io/project_file.rs#L42)

```rust
fn level() -> Compression
```

**What it does.** Returns the zlib compression level used for saving:
`Compression::fast()`, which is level 1 (the zlib default is 6). The comment explains
why: the data is mostly runs of zeros, where even the fastest setting finds long
matches immediately, so level 1 gets nearly all of level 6's shrinkage for a fraction of
the CPU time. Saving a few hundred MB must not stall on the compressor.

**Used by:** [`encode`](#encode); the v6 / v7 tests that hand-build
compressed files.

### `encode`
[src/io/project_file.rs:56](../../../src/io/project_file.rs#L56)

```rust
fn encode(project: &Project) -> Result<(Vec<u8>, u64)>
```

**What it does.** Turns a `Project` into the exact bytes of a `.anim` file:
`MAGIC | version (little-endian) | zlib(postcard body)`.

**Parameters.**
- `project` — the project to save, borrowed read-only (`&`).

**Returns.** `Ok((bytes, raw_len))`: the whole file as a byte vector, and how long the
postcard body was *before* compression (so the caller can log the ratio). An error if
serialising, flushing or compressing failed.

**How it works.**
1. Make an empty `Vec<u8>` and push the 4 magic bytes and the 4 version bytes into it.
2. Wrap that vector in a `ZlibEncoder` (the compressor). The compressor appends its
   output after the header, so nothing has to be copied afterwards.
3. Wrap the compressor in a 256 KiB `BufWriter` and let `postcard::to_io` write the
   project into it. Two design choices, both explained in comments:
   - `to_io` (stream out) rather than `to_stdvec` (build a full `Vec` first): building the
     uncompressed body first would need hundreds of MB extra on exactly the big files
     compression is for.
   - The `BufWriter` is "not optional": postcard writes one byte at a time and the
     compressor has no input buffer of its own, so without it saving was measured at
     3 MiB/s, about thirty times slower.
4. `w.into_inner()` flushes the buffer into the compressor and hands the compressor back;
   a write error shows up here instead of being lost when the buffer is dropped.
5. Read `enc.total_in()` (bytes fed in = raw body length), then `enc.finish()` to end
   the zlib stream and get the finished vector back.

**Example.** The same pipeline in Node.js terms:
```js
const header = Buffer.concat([Buffer.from("ANIM"), u32le(8)]);
const body = binaryEncode(project);                 // postcard
const file = Buffer.concat([header, zlib.deflateSync(body, { level: 1 })]);
// (the Rust version streams instead of building `body` in memory first)
```

**Used by:** [`save_to`](#save_to); many tests.

### `decode`
[src/io/project_file.rs:88](../../../src/io/project_file.rs#L88)

```rust
fn decode(bytes: &[u8]) -> Result<Project>
```

**What it does.** Decodes a `.anim` file that is already in memory. It just calls
[`decode_from`](#decode_from) with the byte slice (a slice implements
`Read`). It only exists when compiling tests (`#[cfg(test)]`), so tests can round-trip
without touching the disk.

**Used by:** the tests in this file.

### `decode_from`
[src/io/project_file.rs:96](../../../src/io/project_file.rs#L96)

```rust
fn decode_from(mut input: impl Read) -> Result<Project>
```

**What it does.** Reads a whole project from a byte stream, whatever format version it
was written in, and returns the current-version `Project`.

**Parameters.**
- `input` — any reader positioned at the start of the file (on load: a buffered file).

**Returns.** The project, or an error: "not an Animator project file" (bad magic or fewer
than 8 bytes), "unsupported project version …", or a parse error wrapped with which
version's body failed.

**How it works.**
1. Read exactly 8 bytes. If that fails or the first 4 are not `ANIM`, bail.
2. Rebuild the version number from bytes 4–7 (`u32::from_le_bytes`).
3. `match` on the version:
   - `8` → [`stream`](#stream)`::<Project>` — inflate and parse at the same
     time, straight into the current struct.
   - `7` → `stream::<ProjectV7>` then `.into()` (the `From` conversion in
     [legacy.rs](legacy.md)) to turn the old mirror into a `Project`.
   - `6` → the same with `ProjectV6`.
   - anything else → read the rest of the input into memory and hand it to
     [`decode_uncompressed`](#decode_uncompressed), which handles 2–5 and
     rejects the rest.
4. Call `project.normalize_groups()`: a file from another build, or a damaged one, could
   break the layer-group rules the rest of the app relies on, so they are repaired here.

The comment explains the streaming: inflating the whole body first would hold the raw
postcard (every pixel of every cell) *and* the project built from it — twice the
project's size at once.

**Used by:** [`load_from`](#load_from), [`decode`](#tests).

### `stream`
[src/io/project_file.rs:123](../../../src/io/project_file.rs#L123)

```rust
fn stream<T: serde::de::DeserializeOwned>(body: impl Read) -> Result<T>
```

**What it does.** Parses a zlib-compressed postcard body *as it is being decompressed*,
into any type `T` that serde can build without borrowing from the input
(`DeserializeOwned`). `T` is `Project`, `ProjectV7` or `ProjectV6`.

**How it works.** `ZlibDecoder::new(body)` decompresses; a 256 KiB `BufReader` sits on top
so postcard's one-byte reads are cheap; that is wrapped in a [`Stream`](#struct-stream)
flavor and a `postcard::Deserializer`; then `T::deserialize(&mut de)` builds the value.

**Example.**
```rust
let p: Project = stream(file_after_header)?;                 // v8
let old: ProjectV7 = stream(file_after_header)?;             // v7, then old.into()
```

**Used by:** [`decode_from`](#decode_from).

## `impl<R> Stream<R>`

### `Stream::new`
[src/io/project_file.rs:139](../../../src/io/project_file.rs#L139)

```rust
fn new(input: R) -> Self
```

Wraps a reader in a `Stream` with an empty scratch buffer.

**Used by:** [`stream`](#stream).

## `impl<'de, R: Read + 'de> postcard::de_flavors::Flavor<'de> for Stream<R>`

### `Stream::pop`
[src/io/project_file.rs:156](../../../src/io/project_file.rs#L156)

```rust
fn pop(&mut self) -> postcard::Result<u8>
```

Reads and returns the next single byte. Any read failure (including end of input) is
reported to postcard as `DeserializeUnexpectedEnd`, which is how a truncated file becomes
an error.

### `Stream::try_take_n`
[src/io/project_file.rs:166](../../../src/io/project_file.rs#L166)

```rust
fn try_take_n(&mut self, _ct: usize) -> postcard::Result<&'de [u8]>
```

postcard calls this when a value wants to *borrow* bytes straight from the input for the
whole parse. This flavor always says no (`DeserializeUnexpectedEnd`): nothing in a project
borrows from its file, and a stream has no long-lived buffer to lend.

### `Stream::try_take_n_temp`
[src/io/project_file.rs:170](../../../src/io/project_file.rs#L170)

```rust
fn try_take_n_temp<'a>(&'a mut self, ct: usize) -> postcard::Result<&'a [u8]>
where
    'de: 'a,
```

**What it does.** Reads the next `ct` bytes into the reusable scratch buffer and lends
them out only until the next call. This is the path a cell's pixel byte string takes.

**How it works.**
1. If `ct` is larger than [`MAX_BYTES`](#const-max_bytes), fail — a corrupt length must
   not trigger a giant allocation.
2. Grow `scratch` to `ct` bytes if it is smaller (it never shrinks, so after the biggest
   cell no more allocation happens).
3. `read_exact` into the first `ct` bytes; failure → `DeserializeUnexpectedEnd`.
4. Return that slice.

The `where 'de: 'a` line says "the input lifetime outlives this temporary borrow".

### `Stream::finalize`
[src/io/project_file.rs:187](../../../src/io/project_file.rs#L187)

```rust
fn finalize(self) -> postcard::Result<R>
```

Called when parsing is done; gives back the underlying reader.

## Free functions

### `decode_uncompressed`
[src/io/project_file.rs:193](../../../src/io/project_file.rs#L193)

```rust
fn decode_uncompressed(version: u32, body: &[u8]) -> Result<Project>
```

**What it does.** Decodes the body of a file from before compression existed (versions
2–5), already fully in memory.

**Parameters.**
- `version` — the number from the header.
- `body` — every byte after the 8-byte header.

**Returns.** The migrated `Project`, or `unsupported project version {v} (this build reads
2–8)` for anything else (version 1, or a version from a newer build).

**How it works.** `match version`: 5 → `postcard::from_bytes::<ProjectV6>` (v5 has the
same layout as v6, it just was not compressed); 4 → `ProjectV4`; 3 → `ProjectV3`;
2 → `ProjectV2`; each followed by `.into()` to convert the mirror into a current
`Project` (see [legacy.rs](legacy.md)).

**Used by:** [`decode_from`](#decode_from).

### `save_to`
[src/io/project_file.rs:762](../../../src/io/project_file.rs#L762)

```rust
pub fn save_to(project: &Project, path: &Path) -> Result<()>
```

**What it does.** Writes `project` to `path`, overwriting any file there. Kept separate
from [`ask_save_path`](#ask_save_path) so the caller can remember the path
and save to it again without a dialog.

**Parameters.**
- `project` — what to save.
- `path` — the destination file.

**Returns.** `Ok(())` on success, or an error such as `writing "C:\\…\\x.anim"` with the
OS error inside.

**How it works.**
1. [`encode`](#encode) the project.
2. `std::fs::write` the bytes in one go.
3. Log a line like `Saved project → path (3.2 MiB from 180.0 MiB, 56x)`. The comment says
   why: a format change that quietly stopped compressing would otherwise only be noticed
   as a surprise in the file manager. (`packed.max(1)` avoids dividing by zero.)

**Used by:** [app.rs](../app.md) `write_project`, on a background thread with a cloned
project so drawing can continue; the result is sent back over a channel and collected by
`poll_save_job`. Also the `saves_and_loads_a_real_file_losslessly` test.

### `ask_save_path`
[src/io/project_file.rs:782](../../../src/io/project_file.rs#L782)

```rust
pub fn ask_save_path(current: Option<&Path>) -> Option<PathBuf>
```

**What it does.** Shows the operating system's "Save" dialog (via the `rfd` crate),
filtered to `.anim` files.

**Parameters.**
- `current` — the file the project was last saved to or opened from, if any. When set,
  the dialog opens in that file's folder with its name filled in, so "Save As" starts
  where the project already lives. When `None`, the name is pre-filled as
  `project.anim`.

**Returns.** `Some(path)` the user chose, or `None` if they cancelled.

**Used by:** [app.rs](../app.md) `save_project_as` (and `save_project` through it when no
path is remembered yet).

### `load_from`
[src/io/project_file.rs:803](../../../src/io/project_file.rs#L803)

```rust
pub fn load_from(path: &Path) -> Result<Project>
```

**What it does.** Opens the file at `path`, wraps it in a 256 KiB `BufReader`, decodes it
with [`decode_from`](#decode_from), and logs `Loaded project ← path`. It is
the read half of [`save_to`](#save_to), split out of
[`load_dialog`](#load_dialog) so tests can do a real on-disk round trip
without a file picker.

**Returns.** The project, or an error (`reading "…"` if the file cannot be opened, or any
decode error).

**Used by:** [`load_dialog`](#load_dialog); [app.rs](../app.md)
`perf_big_canvas_stroke` (reads the project named by `ANIM_PROBE`);
[webp_export.rs](webp_export.md) tests; this file's tests.

### `load_dialog`
[src/io/project_file.rs:812](../../../src/io/project_file.rs#L812)

```rust
pub fn load_dialog() -> Result<Option<(Project, PathBuf)>>
```

**What it does.** Shows the OS "Open" dialog filtered to `.anim`, then loads the chosen
file with [`load_from`](#load_from).

**Returns.** Three outcomes, nested like an onion:
- `Ok(Some((project, path)))` — loaded; `path` is returned so the caller can remember it
  for the next Save.
- `Ok(None)` — the user cancelled the dialog.
- `Err(e)` — the file could not be read or decoded.

```rust
match project_file::load_dialog() {
    Ok(Some((p, path))) => state.load_project(p, Some(path)),
    Ok(None) => {}                                   // cancelled
    Err(e) => log::error!("Open project failed: {e:#}"),
}
```

**Used by:** [app.rs](../app.md) (`Action::OpenProject`, the keyboard shortcut) and
[ui/shell.rs](../ui/shell.md) (File ▸ Open project…), both exactly as in the snippet.

## Tests

Run with `cargo test io::project_file`. There are two test modules: `tests` (format and
migration) and `pins` (byte-exact "nothing changed by accident" checks).

| Test | What it checks |
|---|---|
| [`decodes_v2_project`](../../../src/io/project_file.rs#L222) | A hand-built v2 file (no `track_points`) decodes and migrates: frame count, layer name and exposures kept, `track_points` empty. |
| [`decodes_v3_project`](../../../src/io/project_file.rs#L261) | A v3 file keeps its track points and gets an identity camera, no camera keys and a 0×0 (use project size) cell size. |
| [`decodes_v4_project`](../../../src/io/project_file.rs#L303) | A v4 file keeps its camera and cell size, and its transform keys come back with `Ease::Linear` (same playback as before `ease` existed). |
| [`saves_a_project_with_freed_cells`](../../../src/io/project_file.rs#L366) | A project with a freed (0×0 tombstone) cell saves and loads; this once panicked and crashed the app during a background save. |
| [`round_trips_current_project`](../../../src/io/project_file.rs#L382) | `encode` stamps `VERSION`, and track points, cell size, camera, camera keys and key eases all survive a round trip. |
| [`pixels_survive_the_round_trip_exactly`](../../../src/io/project_file.rs#L427) | A cell filled with a pattern that has no long runs comes back byte-identical (guards against a lossy or truncating codec). |
| [`decodes_v5_project`](../../../src/io/project_file.rs#L450) | An uncompressed v5 file opens with name, cell size, camera keys and fps intact and nothing clipped. |
| [`decodes_v6_project`](../../../src/io/project_file.rs#L512) | A compressed v6 file opens with both layers and no clip / alpha lock. |
| [`clipping_and_alpha_lock_survive_a_save`](../../../src/io/project_file.rs#L532) | `clip` and `alpha_lock` flags come back on the right layers. |
| [`groups_survive_a_save`](../../../src/io/project_file.rs#L546) | A collapsed, half-opaque group keeps its header, members, `collapsed` flag and opacity. |
| [`a_v7_file_opens_without_groups`](../../../src/io/project_file.rs#L562) | A v7 file keeps its clipping and opens with every layer ungrouped. |
| [`compression_actually_shrinks_sparse_cells`](../../../src/io/project_file.rs#L616) | Five blank 256×256 cells pack more than 100× smaller than the raw postcard body, and the reported raw size is the true body size. |
| [`rejects_a_truncated_body`](../../../src/io/project_file.rs#L641) | A file cut short, or a header with no body, is an error, not a panic or a half-built project. |
| [`measure_ratio_and_time`](../../../src/io/project_file.rs#L652) | `#[ignore]` measurement harness: 12 cells of 1080p fake line art; prints raw / packed size and encode / decode times. Run with `-- --ignored --nocapture`. |
| [`saves_and_loads_a_real_file_losslessly`](../../../src/io/project_file.rs#L704) | `save_to` + `load_from` through a real temp file: pixels byte-identical and the file smaller than the raw body. |
| [`rejects_unsupported_version`](../../../src/io/project_file.rs#L738) | A version newer than `VERSION` gives "unsupported project version"; a file starting `NOPE` is refused. |
| [`a_saved_project_is_byte_for_byte_what_it_was`](../../../src/io/project_file.rs#L830) | (`pins`) The postcard body of the shared test scene (`composite::pins::scene`) still has the same length and FNV hash — any layout change fails here. |
| [`the_format_version_has_not_moved`](../../../src/io/project_file.rs#L836) | (`pins`) `VERSION` is still 8. |
| [`the_camera_s_session_options_stay_out_of_the_file`](../../../src/io/project_file.rs#L841) | (`pins`) Setting a session-only camera size does not change the saved bytes, and a load brings back default `camera_opts`. |

**Test helpers:** `v6_mirror` ([line 476](../../../src/io/project_file.rs#L476)) —
copies a `Project` into the v5/v6 mirror `ProjectV6`, field by field, the way a file of
that era was written; used by `decodes_v5_project` and `decodes_v6_project`.

### `mod tests`

Run with `cargo test io::project_file::tests` · 16 tests

| Test | What it checks |
|---|---|
| [`decodes_v2_project`](../../../src/io/project_file.rs#L222) | A v2-era file (no `track_points`) must decode and migrate cleanly. |
| [`decodes_v3_project`](../../../src/io/project_file.rs#L261) | A v3-era file (no camera, no per-layer cell size) must decode and migrate to an identity camera, which composites exactly as v3 did. |
| [`decodes_v4_project`](../../../src/io/project_file.rs#L303) | A v4-era file (transform keys without `ease`) must decode with its keys intact and default to Linear, which reproduces v4 playback exactly. |
| [`saves_a_project_with_freed_cells`](../../../src/io/project_file.rs#L366) | A freed cell is 0×0 and holds nothing; saving one must still work. Saves panicked here once, which on a background save took the whole app down: reading a 0-wide canvas row by row asked for 0-byte rows. |
| [`round_trips_current_project`](../../../src/io/project_file.rs#L382) | Current-version round trip through encode/decode. |
| [`pixels_survive_the_round_trip_exactly`](../../../src/io/project_file.rs#L427) | Fill a cell with a pattern no compressor can fake its way through, and demand every byte back. Compression is lossless, so a drawing that survives a save is bit-identical — this is the test that keeps it that way if the codec is ever swapped. |
| [`decodes_v5_project`](../../../src/io/project_file.rs#L450) | An uncompressed v5 file — everything written before compression landed — must still open. Hand-built rather than produced by `encode`, because `encode` only writes the current version now. |
| [`decodes_v6_project`](../../../src/io/project_file.rs#L512) | A v6 file — deflated, no clipping yet — opens with nothing clipped. |
| [`clipping_and_alpha_lock_survive_a_save`](../../../src/io/project_file.rs#L532) | Clipping and alpha lock survive a save. |
| [`groups_survive_a_save`](../../../src/io/project_file.rs#L546) | Groups survive a save. |
| [`a_v7_file_opens_without_groups`](../../../src/io/project_file.rs#L562) | A v7 file — clipping, no groups — opens with every layer ungrouped. |
| [`compression_actually_shrinks_sparse_cells`](../../../src/io/project_file.rs#L616) | The point of the whole format bump. Blank cells are runs of zero bytes, so they must collapse to a tiny fraction of the raw postcard — if this ever fails, something has stopped compressing. |
| [`rejects_a_truncated_body`](../../../src/io/project_file.rs#L641) | A half-copied file must fail, not panic or hand back a half-built project. Truncation is the realistic corruption for a save interrupted partway through. |
| [`measure_ratio_and_time`](../../../src/io/project_file.rs#L652) | Measure ratio and time. |
| [`saves_and_loads_a_real_file_losslessly`](../../../src/io/project_file.rs#L704) | The whole thing through a real file: write it, read it back, and demand the pixels byte for byte. `encode`/`decode` alone would not catch a framing or truncation bug that only shows up once bytes hit the disk. |
| [`rejects_unsupported_version`](../../../src/io/project_file.rs#L738) | A file from a future build is refused with the supported range, rather than being fed to a parser that would read it as garbage. |

**Test helpers** (used only by the tests above):

- [`v6_mirror`](../../../src/io/project_file.rs#L476) — `p` in the v5/v6 layout, as a file of that era wrote it.

### `mod pins`

Run with `cargo test io::project_file::pins` · 3 tests

| Test | What it checks |
|---|---|
| [`a_saved_project_is_byte_for_byte_what_it_was`](../../../src/io/project_file.rs#L830) | A saved project is byte for byte what it was. |
| [`the_format_version_has_not_moved`](../../../src/io/project_file.rs#L836) | The format version has not moved. |
| [`the_camera_s_session_options_stay_out_of_the_file`](../../../src/io/project_file.rs#L841) | The camera s session options stay out of the file. |
