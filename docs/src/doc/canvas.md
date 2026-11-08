# `src/doc/canvas.rs` — Pixel storage for one drawing (a cell)

> **Source:** [src/doc/canvas.rs](../../../src/doc/canvas.rs) · **Module:** `crate::doc::canvas` · **Size:** 831 lines · Line links as of commit `67a7cb9`

## What this file is for

A `Canvas` is the pixels of one **cell** — one drawing that the x-sheet shows on one or
more frames of a layer (see [glossary](../../glossary.md)). Every brush stroke, fill,
selection move and import ends up writing bytes into a `Canvas`. It is part of the
**document model** (`src/doc/`): the data of an animation, with no UI in it.

A canvas can hold its pixels two ways:

- **Full** — a buffer with every pixel. A cell someone is drawing on needs this, because any
  pixel may be written next.
- **Packed** — only the smallest rectangle that has any non-zero byte in it; everything
  outside that rectangle is known to be zero. Drawings rarely fill their whole cell, so a
  full buffer is mostly zeros. The source comment gives real numbers: one 2160×2880
  project's cells took 3.5 GB full and about 340 MB packed.

Reading works on either form. Writing (through [`Canvas::pixels_mut`](#canvaspixels_mut))
unpacks first. The app packs the cells nobody is drawing on
(`pack_idle_cells` in [app.rs](../app.md)).

Two more pieces of bookkeeping ride along with the pixels:

- **`rev`** — a number that is new for every canvas created and every time something gets the
  chance to write. Two canvases with the same `rev` hold the same pixels, so a copy can be
  trusted without comparing bytes.
- **`dirty`** — the rectangle changed since the GPU texture was last updated, so the UI only
  re-uploads that part. It is not saved to disk.

The file also has the `.anim` reading/writing code for a canvas (written by hand so the bytes
match what older versions wrote), a few fast scanning helpers, and
[`par_rows`](#par_rows), a small helper that splits work on a big buffer across threads.

### The pixel layout (same as a browser's `ImageData.data`)

Pixels are **RGBA8, row-major, straight (unmultiplied) alpha** — exactly the layout of
`ImageData.data` from a browser `<canvas>`:

- 4 bytes per pixel, in the order red, green, blue, alpha; each byte 0–255.
- Row 0 first (top of the image), left to right, then row 1, and so on.
- **Straight alpha** means the colour bytes are *not* multiplied by alpha. A pixel
  `[200, 0, 0, 0]` is "red, but fully transparent": invisible, yet its bytes are not zero.
  (Premultiplied alpha, which this file does *not* use, would store that pixel as
  `[0, 0, 0, 0]`.) This is why the file has both [`is_blank`](#canvasis_blank) (no pixel has
  any alpha) and [`is_all_zero`](#canvasis_all_zero) (every byte is zero, hidden colour
  included).

**Worked example — full canvas.** A 40×30 canvas; read the pixel at `x = 10, y = 5`:

```text
index = (y * width + x) * 4 = (5 * 40 + 10) * 4 = 840
bytes 840, 841, 842, 843  =  R, G, B, A
```

```rust
let w = canvas.width as usize;               // 40
let i = (5 * w + 10) * 4;                     // 840
let px = canvas.pixels();                     // every byte, see `pixels()`
let (r, g, b, a) = (px[i], px[i + 1], px[i + 2], px[i + 3]);
// Better: canvas.px(10, 5) returns [r, g, b, a] without building a buffer.
```

```js
// The same thing in a browser:
const img = ctx.getImageData(0, 0, 40, 30);
const i = (5 * img.width + 10) * 4;          // 840
const [r, g, b, a] = img.data.slice(i, i + 4);
```

**Worked example — packed canvas.** Say the drawing only covers the rectangle
`min_x = 10, min_y = 5, max_x = 31, max_y = 21` (max values are *exclusive*). The packed
buffer is then a little 21×16 image of its own. Pixel `(13, 8)` sits at:

```text
rect width rw = 31 - 10 = 21
index = ((y - min_y) * rw + (x - min_x)) * 4 = ((8 - 5) * 21 + (13 - 10)) * 4 = 264
```

Pixel `(0, 0)` is outside the rectangle, so it is `[0, 0, 0, 0]` without reading anything.
[`Canvas::px`](#canvaspx) does exactly these two cases.

**Pixels as 32-bit words.** Some scans read four bytes at once as a little-endian `u32`.
Byte order R, G, B, A becomes the number `A·2²⁴ + B·2¹⁶ + G·2⁸ + R`, so the alpha byte is the
top 8 bits — the constant `ALPHA = 0xFF00_0000`. `word & ALPHA != 0` asks "does this pixel
have any alpha?" in one operation.

## How it connects

- **Used by:**
  - [project.md](project.md) — a `Project` keeps every cell as `Arc<Canvas>` in
    `cells`; uses [`tombstone`](#canvastombstone) for freed cells and
    [`recentered`](#canvasrecentered) when the canvas size changes.
  - [../app.md](../app.md) — packs idle cells ([`pack`](#canvaspack)), frees cells into
    tombstones, reads `dirty` to know which part of a texture to re-upload, uses
    [`par_rows`](#par_rows), [`rev`](#canvasrev), [`held_rect`](#canvasheld_rect),
    [`unpack`](#canvasunpack), [`row_span`](#canvasrow_span) and
    [`is_blank`](#canvasis_blank).
  - [../app/orient.md](../app/orient.md) — flips/rotates cells with
    [`oriented`](#canvasoriented).
  - [../app/select.md](../app/select.md), [../app/groups.md](../app/groups.md),
    [../app/liquify.md](../app/liquify.md), [../app/lasso_fill.md](../app/lasso_fill.md),
    [../app/button_drag.md](../app/button_drag.md) — edit cells and mark them dirty.
  - [clip.md](clip.md) — reads rows with [`read_row_into`](#canvasread_row_into) to cut
    clipping-mask textures.
  - Tools that paint: [../tools/stroke.md](../tools/stroke.md),
    [../tools/krita/mod.md](../tools/krita/mod.md), [../tools/fill.md](../tools/fill.md),
    [../tools/shape.md](../tools/shape.md), [../tools/lasso.md](../tools/lasso.md),
    [../tools/selection.md](../tools/selection.md), [../tools/ribbon.md](../tools/ribbon.md);
    [../tools/fill.md](../tools/fill.md), [../tools/wand.md](../tools/wand.md) and
    [../tools/liquify.md](../tools/liquify.md) also borrow [`par_rows`](#par_rows).
  - File input/output: [../io/project_file.md](../io/project_file.md) (`.anim`, via the serde
    code here), [../io/composite.md](../io/composite.md) (flattening frames, reads
    [`row_span`](#canvasrow_span)), [../io/kra/mod.md](../io/kra/mod.md),
    [../io/png_save.md](../io/png_save.md) and the importers
    ([../io/png_import.md](../io/png_import.md), [../io/gif_import.md](../io/gif_import.md),
    [../io/webp_import.md](../io/webp_import.md), [../io/image_import.md](../io/image_import.md),
    [../io/video_import.md](../io/video_import.md)) that build canvases with
    [`from_pixels`](#canvasfrom_pixels).
  - [../krita_link.md](../krita_link.md), [../krita_stress.md](../krita_stress.md),
    [../undo.md](../undo.md), [../ui/shell.md](../ui/shell.md).
  - The GPU drawing of canvases ([../ui/canvas_gpu.md](../ui/canvas_gpu.md)) is fed by
    textures that [app.rs](../app.md) builds from these pixels.
- **Uses:** [orient.md](orient.md) (`Orient` for [`oriented`](#canvasoriented)); outside
  crates `serde` (save/load), `bytemuck` (reading a byte slice as `u32`/`u64` words) and,
  in tests, `postcard` (the binary format inside `.anim`).

```text
 pen / fill / import ──► Canvas::pixels_mut()  ──► Store::Full (every pixel)
                                                        │   app packs idle cells
                                                        ▼
 GPU upload, export ◄── pixels() / row_span() ◄── Store::Packed { rect, pixels }
                                                        │
 .anim file ◄──────── serialize (always every pixel) ◄─┘   deserialize ──► packed
```

## Rust concepts in this file

**Enums that carry data.** `Store` is either `Full(buffer)` or `Packed { rect, pixels }`.
Code `match`es on it to handle each case; the compiler refuses to build if a case is missed.
```rust
match &self.store {
    Store::Full(p) => Cow::Borrowed(p),
    Store::Packed { .. } => Cow::Owned(self.unpacked()),
}
```
```js
// Closest JS: a tagged union and a switch
switch (store.kind) { case "full": ...; case "packed": ...; }
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

**Slices: `&[u8]` and `&mut [u8]`.** A slice is a view into part of a buffer (pointer +
length), like a `Uint8Array` made with `subarray()` — no copying. `&mut` means "I may write
through this view". `buf.chunks_exact(4)` walks a byte slice four bytes (one pixel) at a time.
```rust
let row = &p[y * w..(y + 1) * w];   // one row of a full canvas, no copy
```
```js
const row = data.subarray(y * w, (y + 1) * w);
```
More: [Strings and slices](../../rust-primer.md#strings-and-slices)

**`Cow` — "borrowed or owned".** [`pixels()`](#canvaspixels) returns `Cow<'_, [u8]>`: for a
full canvas it lends the existing buffer (free); for a packed one it builds a new full buffer
and hands that over. The caller reads it the same way in both cases.
```rust
let px = canvas.pixels();      // Cow: maybe borrowed, maybe a fresh Vec
let first_alpha = px[3];       // reads the same either way
```
There is no JS equivalent; think "a getter that sometimes returns the internal array and
sometimes a temporary copy". More: [Smart pointers](../../rust-primer.md#smart-pointers)

**A global atomic counter.** `static NEXT_REV: AtomicU64` is one number shared by the whole
program that threads can bump safely. `fetch_add(1, …)` returns the old value and adds one in
a single step.
```rust
static NEXT_REV: AtomicU64 = AtomicU64::new(1);
NEXT_REV.fetch_add(1, Ordering::Relaxed)
```
```js
let nextRev = 1;           // JS is single-threaded, so a plain counter would do
const rev = nextRev++;
```
More: [Constants and statics](../../rust-primer.md#constants-and-statics),
[Threads and channels](../../rust-primer.md#threads-and-channels)

**Scoped threads.** [`par_rows`](#par_rows) uses `std::thread::scope` to start a few threads
that each get their own non-overlapping `&mut` piece of one buffer; the scope waits for all of
them before returning, so borrowing the buffer is safe.
```rust
std::thread::scope(|s| {
    for (k, band) in buf.chunks_mut(per * row).enumerate() {
        s.spawn(move || f(k * per, band));
    }
}); // every thread has finished here
```
JS comparison: like splitting an array between Web Workers and `await Promise.all(...)`, except
the threads write straight into the same memory. More:
[Threads and channels](../../rust-primer.md#threads-and-channels)

**Struct update syntax `..other`.** Build a value by naming a few fields and copying the rest.
```rust
Self { dirty: None, ..Self::new(0, 0) }
```
```js
({ ...newCanvas(0, 0), dirty: null })
```
More: [Structs](../../rust-primer.md#structs)

**Hand-written `Serialize` / `Deserialize`.** Most types get save/load code from
`#[derive(Serialize, Deserialize)]`. `Canvas` writes its own (`impl serde::Serialize for
Canvas`) so it can write a packed canvas as if it were full and pack while loading. serde's
reading side uses the *visitor* pattern: you hand the decoder an object whose methods
(`visit_seq`, `visit_bytes`, …) it calls with whatever it finds.
More: [Serde and file formats](../../rust-primer.md#serde-and-file-formats),
[Traits and derive](../../rust-primer.md#traits-and-derive)

## impl serde::Serialize for Canvas

## impl serde::Deserialize for Canvas

## impl DirtyRect

## impl Canvas

## Types

### `static NEXT_REV`
[src/doc/canvas.rs:17](../../../src/doc/canvas.rs#L17)

```rust
static NEXT_REV: AtomicU64 = AtomicU64::new(1);
```

`AtomicU64` starting at 1. The source of every [`rev`](#canvasrev) value; because it is one
counter for the whole program, revs are unique across all canvases.

### `const PAR_MIN`
[src/doc/canvas.rs:25](../../../src/doc/canvas.rs#L25)

```rust
const PAR_MIN: usize = 2 << 20;
```

`2 << 20` = 2 MiB (2,097,152 bytes). Below this size [`par_rows`](#par_rows) stays on one
thread, because starting threads would cost more than they save.

### `struct Canvas`
[src/doc/canvas.rs:59](../../../src/doc/canvas.rs#L59)

The pixels of one cell, plus bookkeeping. Derives `Clone` (a clone copies the pixel buffer
and keeps the same `rev`, since it holds the same pixels). Its save format is hand-written
(see [`serialize`](#canvasserialize)) so `.anim` files stay byte-for-byte what the old
derived code wrote.

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | Width in pixels. `0` (with `height`) marks a freed cell, see [`tombstone`](#canvastombstone). |
| `height` | `u32` | Height in pixels. |
| `store` | `Store` | The pixels themselves, full or packed. |
| `dirty` | `Option<DirtyRect>` | Region changed since the GPU last copied it (min inclusive, max exclusive). `None` = clean. Render bookkeeping only; never saved. |
| `rev` | `u64` | "Which pixels these are." New for every canvas and every chance to write; unchanged by packing/unpacking; kept by `clone()`. Equal revs mean equal pixels. |
| `unwritten` | `Option<(u64, DirtyRect, Vec<u8>)>` | After [`unpack`](#canvasunpack): the rev at the time, the packed rect and the packed bytes. Kept until something writes, so packing again before that is free. |

### `enum Store`
[src/doc/canvas.rs:78](../../../src/doc/canvas.rs#L78)

How a canvas holds its pixels (private to this file).

| Variant | Meaning |
|---|---|
| `Full(Vec<u8>)` | Every pixel, row-major RGBA8, unmultiplied (straight) alpha. Length is `width × height × 4`. |
| `Packed { rect: DirtyRect, pixels: Vec<u8> }` | Only `rect`, row-major. `rect` is the bounds of every byte that is not zero, so nothing is lost; everything outside is zero. An empty `rect` is a blank canvas and holds no buffer at all. |

### `type Pixels::Value`
[src/doc/canvas.rs:118](../../../src/doc/canvas.rs#L118)

```rust
type Value = Canvas;
```

What the visitor `V` produces: a `Canvas`.

### `type Pixels::Value`
[src/doc/canvas.rs:124](../../../src/doc/canvas.rs#L124)

```rust
type Value = Canvas;
```

What the visitor `V` produces: a `Canvas`.

### `type V::Value`
[src/doc/canvas.rs:144](../../../src/doc/canvas.rs#L144)

```rust
type Value = Canvas;
```

What the visitor `V` produces: a `Canvas`.

### `struct DirtyRect`
[src/doc/canvas.rs:160](../../../src/doc/canvas.rs#L160)

A pixel rectangle. Despite the name it is used for any rectangle in this file (dirty regions,
packed bounds, content bounds). Derives `Clone, Copy, Debug, PartialEq, Eq` — `Copy` means it
is copied on assignment like a number.

| Field | Type | Meaning |
|---|---|---|
| `min_x` | `u32` | Left edge, inclusive. |
| `min_y` | `u32` | Top edge, inclusive. |
| `max_x` | `u32` | Right edge, **exclusive** (one past the last column). |
| `max_y` | `u32` | Bottom edge, exclusive. |

### `const EMPTY`
[src/doc/canvas.rs:177](../../../src/doc/canvas.rs#L177)

```rust
const EMPTY: DirtyRect = DirtyRect {
    min_x: 0,
    min_y: 0,
    max_x: 0,
    max_y: 0,
};
```

A `DirtyRect` of all zeros: the empty rectangle a blank packed canvas holds.

### `const ALPHA`
[src/doc/canvas.rs:185](../../../src/doc/canvas.rs#L185)

```rust
const ALPHA: u32 = u32::from_le_bytes([0, 0, 0, 255]);
```

`u32::from_le_bytes([0, 0, 0, 255])` = `0xFF00_0000`: the alpha byte of a pixel read as a
little-endian `u32` (see [Pixels as 32-bit words](#the-pixel-layout-same-as-a-browsers-imagedatadata)).
Used as the mask for "has any alpha".

## Free functions

### `next_rev`
[src/doc/canvas.rs:19](../../../src/doc/canvas.rs#L19)

```rust
fn next_rev() -> u64
```

**What it does.** Takes the next number from [`NEXT_REV`](#static-next_rev) and returns it.
Every call returns a different value, from any thread. Called whenever a canvas is made or
gets a chance to be written ([`new`](#canvasnew), [`pixels_mut`](#canvaspixels_mut),
[`clear`](#canvasclear)).

### `par_rows`
[src/doc/canvas.rs:32](../../../src/doc/canvas.rs#L32)

```rust
pub fn par_rows<T: Send>(buf: &mut [T], row: usize, f: impl Fn(usize, &mut [T]) + Sync)
```

**What it does.** Runs `f` over a buffer in **bands of whole rows**, one band per thread once
the buffer is big enough to be worth it. The source comment explains why: copying or clearing
a big buffer is limited by how fast one CPU core can move memory, which is only a fraction of
what the memory itself can do, so an 8K cell clears and fills about twice as fast on several
cores.

**Parameters.**
- `buf` — the whole buffer, written in place. `T` can be any element type that may be sent to
  another thread (`T: Send`), e.g. `u8` pixel bytes or `u32` labels.
- `row` — how many items make one row (for RGBA8 pixels: `width * 4`).
- `f` — called once per band with *(index of the band's first row, the band's items)*. It must
  be `Sync` (safe to share between threads) because every thread calls the same `f`.

**How it works.**
1. If `row` is 0 or the buffer is empty (a freed 0×0 canvas), return — there is nothing to do,
   and a band cannot be cut into 0-wide rows.
2. `rows = buf.len() / row`.
3. Pick a thread count: 1 if the buffer is under [`PAR_MIN`](#const-par_min) bytes or has
   fewer than 2 rows; otherwise the machine's available parallelism, capped at 8.
4. One thread: call `f(0, buf)` and return.
5. Otherwise `per = ceil(rows / threads)` rows per band. Inside `std::thread::scope`, split
   the buffer with `chunks_mut(per * row)` and spawn a thread per chunk calling
   `f(k * per, band)`. The scope waits for every thread before `par_rows` returns.

**Example.** Clearing the right half of every row of a 1000-row RGBA image:
```rust
let w = width as usize * 4;                       // bytes per row
par_rows(&mut pixels, w, |first_row, band| {
    for (y, row) in (first_row..).zip(band.chunks_exact_mut(w)) {
        row[w / 2..].fill(0);                      // y is the real row number
    }
});
```

**Used by:** [`Canvas::unpacked`](#canvasunpacked) here; [../app.md](../app.md),
[../tools/fill.md](../tools/fill.md), [../tools/liquify.md](../tools/liquify.md),
[../tools/wand.md](../tools/wand.md).

## `impl serde::Serialize for Canvas`

### `Canvas::serialize`
[src/doc/canvas.rs:92](../../../src/doc/canvas.rs#L92)

```rust
fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error>
```

**What it does.** Writes a canvas for saving: a struct named `"Canvas"` with three fields —
`width`, `height`, and `pixels` (every pixel, full size, as one byte string). A packed canvas is
expanded with [`pixels()`](#canvaspixels) first, so the file holds exactly what the old
derived code wrote. In postcard (the binary format inside `.anim`) a byte string and a list of
bytes are written the same way (length, then raw bytes); the byte-string form is just written
in one go instead of one call per byte.

**Parameters.** `s` — the serde serializer (postcard when saving `.anim`).

**Returns.** Whatever the serializer returns; `Err` if writing fails (the `?` operator passes
the error up).

**Used by:** serde, when [../io/project_file.md](../io/project_file.md) saves a project.

## `impl serde::Serialize for Bytes<'_>`

#### `Bytes::serialize`
[src/doc/canvas.rs:96](../../../src/doc/canvas.rs#L96)

```rust
fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error>
```

`Bytes<'a>(&'a [u8])` is a tiny wrapper type defined inside `Canvas::serialize`. Its only job is this method, which calls `s.serialize_bytes(self.0)` so the pixels go out as one byte string. (The `'a` is a lifetime: the wrapper borrows the pixel slice and cannot outlive it — see [Lifetimes](../../rust-primer.md#lifetimes).)

## `impl<'de> serde::Deserialize<'de> for Canvas`

### `Canvas::deserialize`
[src/doc/canvas.rs:111](../../../src/doc/canvas.rs#L111)

```rust
fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error>
```

**What it does.** Reads a canvas back from a saved file. The pixels arrive as one byte slice and
are **packed as they arrive**, so a whole cell's full buffer is never allocated while loading.
The loaded canvas is clean (`dirty = None`).

**Returns.** The packed `Canvas`, or an error if fields are missing or the pixel byte count does
not equal `width × height × 4`.

**How it works.** It defines two helper types inside itself and asks the decoder to read a
struct named `"Canvas"` with fields `width`, `height`, `pixels`, using visitor `V`:
1. `V::visit_seq` reads `width`, then `height`.
2. It then reads the third element with the seed `Pixels(width, height)` — a "seed" is a
   visitor that already carries some data (here, the size it should expect).
3. `Pixels` asks for a byte buffer; when the bytes arrive, `visit_bytes` checks the length and
   calls [`packed_from`](#canvaspacked_from).

Only the *sequence* form of a struct is handled (`visit_seq`), which is how postcard encodes
structs.

**Used by:** serde, when [../io/project_file.md](../io/project_file.md) loads a project.

## `impl<'de> DeserializeSeed<'de> for Pixels`

#### `Pixels::deserialize`
[src/doc/canvas.rs:119](../../../src/doc/canvas.rs#L119)

```rust
fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Canvas, D::Error>
```

`Pixels(u32, u32)` holds the expected width and height. As a `DeserializeSeed` it tells the decoder "read a byte buffer and give it to me" (`d.deserialize_byte_buf(self)`).

## `impl<'de> Visitor<'de> for Pixels`

#### `Pixels::expecting`
[src/doc/canvas.rs:125](../../../src/doc/canvas.rs#L125)

```rust
fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result
```

Writes `"RGBA8 pixels"` — the phrase serde puts in an error message when the data is the wrong kind.

#### `Pixels::visit_bytes`
[src/doc/canvas.rs:128](../../../src/doc/canvas.rs#L128)

```rust
fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<Canvas, E>
```

Called with the pixel bytes. If `v.len()` is not `w × h × 4` (computed in `u64` so a huge size
cannot overflow) it returns an error like `"4×4 canvas with 10 bytes of pixels"`. Otherwise it
builds the canvas with [`Canvas::packed_from`](#canvaspacked_from), marks it clean
(`dirty = None`) and returns it.

#### `Pixels::visit_byte_buf`
[src/doc/canvas.rs:137](../../../src/doc/canvas.rs#L137)

```rust
fn visit_byte_buf<E: Error>(self, v: Vec<u8>) -> Result<Canvas, E>
```

The same as `visit_bytes`, for decoders that hand over an owned `Vec<u8>` instead of a borrowed
slice: it just calls `visit_bytes(&v)`.

## `impl<'de> Visitor<'de> for V`

#### `V::expecting`
[src/doc/canvas.rs:145](../../../src/doc/canvas.rs#L145)

```rust
fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result
```

Writes `"a canvas"` for error messages.

#### `V::visit_seq`
[src/doc/canvas.rs:148](../../../src/doc/canvas.rs#L148)

```rust
fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Canvas, A::Error>
```

Reads the three fields in order: `width: u32`, `height: u32`, then the pixels through the
`Pixels(width, height)` seed. Any missing element becomes an "invalid length" error naming
which one (`0`, `1` or `2`) was absent.

## `impl DirtyRect`

### `DirtyRect::is_empty`
[src/doc/canvas.rs:168](../../../src/doc/canvas.rs#L168)

```rust
fn is_empty(&self) -> bool
```

True when the rectangle has no area: `max_x <= min_x` or `max_y <= min_y`.

### `DirtyRect::contains`
[src/doc/canvas.rs:172](../../../src/doc/canvas.rs#L172)

```rust
fn contains(&self, x: u32, y: u32) -> bool
```

True when pixel `(x, y)` is inside: `min <= coordinate < max` on both axes (max is exclusive).

## Free functions

### `scan_bounds`
[src/doc/canvas.rs:191](../../../src/doc/canvas.rs#L191)

```rust
fn scan_bounds(buf: &[u8], w: usize, mask: u32) -> Option<DirtyRect>
```

**What it does.** Finds the smallest rectangle around every pixel that has at least one bit of
`mask` set, in a `w`-pixel-wide RGBA8 buffer.

**Parameters.**
- `buf` — RGBA8 bytes, whole rows.
- `w` — width in pixels.
- `mask` — which bits count: [`ALPHA`](#const-alpha) means "anything drawn" (alpha > 0); `!0`
  (all 32 bits set) means "any non-zero byte at all".

**Returns.** `Some(rect)` (max edges exclusive), or `None` if no pixel matches or `w` is 0.

**How it works.**
1. `word(px)` reads 4 bytes as a little-endian `u32`; `hit(px)` is `word & mask != 0`.
2. `blank(row)` decides if a whole row has no hit. If the row's bytes line up as `u32`s
   (`bytemuck::try_cast_slice`), it ORs every word together with no early exit — the source
   notes this lets the compiler vectorise it, about 40% faster on the mostly-empty rows a big
   cell is made of. Otherwise it falls back to checking pixel by pixel.
3. Cut the buffer into rows. `top` = first non-blank row, `bottom` = one past the last
   non-blank row (the `?` returns `None` if every row is blank).
4. Start with `x0 = w` (left edge not found yet) and `x1 = 0`. For each row from `top` to
   `bottom`, look for a hit only in `row[..x0]` (could it be further left than we know?) and in
   `row[x1..]` (further right?). Each row therefore only scans the part not yet known to be
   inside the rectangle.
5. Return `min_x = x0, min_y = top, max_x = x1, max_y = bottom`.

**Used by:** [`packed_from`](#canvaspacked_from), [`is_all_zero`](#canvasis_all_zero),
[`recentered`](#canvasrecentered), [`content_bounds`](#canvascontent_bounds).

### `no_alpha`
[src/doc/canvas.rs:227](../../../src/doc/canvas.rs#L227)

```rust
pub fn no_alpha(px: &[u8]) -> bool
```

**What it does.** True if no pixel in a straight-alpha RGBA8 buffer has any coverage (every
alpha byte is 0). Built to be fast: a blank 100 MB cell is checked in one pass at memory speed
instead of 25 million separate if-checks.

**How it works.**
1. `bytemuck::pod_align_to` splits the bytes into an unaligned head, a run of `u64` words
   (8 bytes = 2 pixels each) and a tail.
2. If the head is not empty, the words would not line up with pixels, so it simply checks
   `p[3] == 0` for every pixel. (The comment notes big allocations are aligned in practice, so
   this is rare.)
3. Otherwise a `u64` mask with `0xff` at byte positions 3 and 7 picks out both pixels' alpha
   bytes. The words are ORed together in blocks of 4096; `.all(...)` stops at the first block
   containing any alpha, so a drawing exits early at its first stroke.
4. The leftover tail pixels are checked one by one.

**Used by:** [`Canvas::is_blank`](#canvasis_blank); tests in [../app.md](../app.md).

### `crop`
[src/doc/canvas.rs:242](../../../src/doc/canvas.rs#L242)

```rust
fn crop(buf: &[u8], w: u32, rect: DirtyRect) -> Vec<u8>
```

Copies the pixels inside `rect` out of a `w`-pixel-wide buffer, row by row, into a new
tightly-packed buffer of `rect`'s size. **Used by:** [`packed_from`](#canvaspacked_from).

## `impl Canvas`

### `Canvas::new`
[src/doc/canvas.rs:253](../../../src/doc/canvas.rs#L253)

```rust
pub fn new(width: u32, height: u32) -> Self
```

**What it does.** Makes a blank, transparent canvas of the given size. It holds **no pixel
buffer at all** (packed with an empty rect) until something is drawn on it. It starts dirty over
its whole area (so the GPU texture gets built) and with a fresh [`rev`](#canvasrev).

**Used by:** almost everything that creates a cell or a scratch image — e.g.
[project.md](project.md), [../app.md](../app.md), the tools in `src/tools/`.

### `Canvas::from_pixels`
[src/doc/canvas.rs:273](../../../src/doc/canvas.rs#L273)

```rust
pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Self
```

**What it does.** Wraps an existing full RGBA8 buffer as a canvas (full storage, dirty
everywhere, fresh rev). The buffer is moved in, not copied.

**Parameters.** `pixels` must be exactly `width × height × 4` bytes; otherwise the program
stops with a panic (`assert_eq!`) whose message names the size.

**Used by:** [../app.md](../app.md) and the image loaders —
[../io/png_import.md](../io/png_import.md), [../io/gif_import.md](../io/gif_import.md),
[../io/webp_import.md](../io/webp_import.md), [../io/image_import.md](../io/image_import.md),
[../io/video_import.md](../io/video_import.md), [../io/png_save.md](../io/png_save.md).

### `Canvas::packed_from`
[src/doc/canvas.rs:292](../../../src/doc/canvas.rs#L292)

```rust
pub fn packed_from(width: u32, height: u32, full: &[u8]) -> Self
```

**What it does.** Like [`from_pixels`](#canvasfrom_pixels) but produces a **packed** canvas,
and only *borrows* the full buffer — it never keeps a copy of the whole thing.

**How it works.** Finds the bounds of every non-zero byte with
[`scan_bounds`](#scan_bounds)`(…, !0)` (an empty rect if there are none), copies just that
rectangle with [`crop`](#crop), and fills the rest of the fields from
[`Canvas::new`](#canvasnew) using `..Self::new(width, height)`. Panics if `full` is the wrong
length.

**Used by:** [`pack`](#canvaspack) and [`Pixels::visit_bytes`](#pixelsvisit_bytes) here.

### `Canvas::tombstone`
[src/doc/canvas.rs:304](../../../src/doc/canvas.rs#L304)

```rust
pub fn tombstone() -> Self
```

**What it does.** Makes what a **freed** cell becomes: a 0×0 canvas with no pixels and no dirty
rect. The cell keeps its slot (its id) in the project's list so no other cell's id shifts; the
next load drops it for good (`Project::compact_cells` in [project.md](project.md)).

**Used by:** [../app.md](../app.md) (freeing cells nothing uses),
[project.md](project.md) and [../io/project_file.md](../io/project_file.md) tests.

### `Canvas::is_tombstone`
[src/doc/canvas.rs:311](../../../src/doc/canvas.rs#L311)

```rust
pub fn is_tombstone(&self) -> bool
```

True when width or height is 0 — a freed cell. **Used by:** [../app.md](../app.md),
[../io/project_file.md](../io/project_file.md).

### `Canvas::byte_len`
[src/doc/canvas.rs:316](../../../src/doc/canvas.rs#L316)

```rust
pub fn byte_len(&self) -> usize
```

The size the full buffer *would* have, `width × height × 4`, however many bytes are actually
held. **Used by:** [../app.md](../app.md), [../app/groups.md](../app/groups.md),
[../app/select.md](../app/select.md), [../tools/fill.md](../tools/fill.md).

### `Canvas::is_all_zero`
[src/doc/canvas.rs:322](../../../src/doc/canvas.rs#L322)

```rust
pub fn is_all_zero(&self) -> bool
```

**What it does.** True when every byte is zero — including colour bytes under zero alpha, which
[`is_blank`](#canvasis_blank) ignores. A full canvas is scanned with
[`scan_bounds`](#scan_bounds)`(…, !0)`; a packed canvas just checks whether its rect is empty
(packing keeps every non-zero byte, so an empty rect means all zeros).

**Used by:** [../app.md](../app.md), [../krita_link.md](../krita_link.md),
[../krita_stress.md](../krita_stress.md).

### `Canvas::pixels`
[src/doc/canvas.rs:332](../../../src/doc/canvas.rs#L332)

```rust
pub fn pixels(&self) -> Cow<'_, [u8]>
```

**What it does.** Gives every pixel (full size, row-major RGBA8) for **reading**. For a full
canvas it lends the existing buffer (`Cow::Borrowed`, free). For a packed canvas it builds a new
full buffer (`Cow::Owned`), which costs a whole image's worth of memory.

**Rule from the source:** call it once per use, **never once per pixel**. To read single pixels
or rows in place, use [`px`](#canvaspx) or [`row_span`](#canvasrow_span).

```rust
// Wrong: builds a full copy of a packed canvas for every pixel.
for y in 0..h { for x in 0..w { let a = canvas.pixels()[((y*w + x)*4 + 3) as usize]; } }
// Right:
let px = canvas.pixels();          // once
for i in (3..px.len()).step_by(4) { let a = px[i]; }
```

**Used by:** widely — serializing here, exports, compositing, tests.

### `Canvas::pixels_mut`
[src/doc/canvas.rs:341](../../../src/doc/canvas.rs#L341)

```rust
pub fn pixels_mut(&mut self) -> &mut [u8]
```

**What it does.** Gives every pixel for **writing**. Because the caller may now change pixels, it
first takes a new [`rev`](#canvasrev) and forgets any kept packed copy (`unwritten = None`). A
packed canvas is unpacked to full storage and stays full until it is packed again. It does
**not** touch `dirty`; callers mark what they changed with [`mark_dirty`](#canvasmark_dirty)
or by setting `dirty` themselves.

**Returns.** A mutable slice over the full buffer.

**Used by:** every painting tool and pixel edit (e.g. [../tools/stroke.md](../tools/stroke.md),
[../tools/fill.md](../tools/fill.md), [../app/select.md](../app/select.md)).

### `Canvas::into_pixels`
[src/doc/canvas.rs:354](../../../src/doc/canvas.rs#L354)

```rust
pub fn into_pixels(self) -> Vec<u8>
```

Consumes the canvas and returns its full pixel buffer by value (a packed canvas is expanded
first). Taking `self` (not `&self`) means the canvas is used up — see
[Ownership](../../rust-primer.md#ownership-and-borrowing). **Used by:** [../app.md](../app.md),
[../app/lasso_fill.md](../app/lasso_fill.md), [../io/kra/mod.md](../io/kra/mod.md),
[../tools/fill.md](../tools/fill.md), [../tools/ribbon.md](../tools/ribbon.md).

### `Canvas::unpacked`
[src/doc/canvas.rs:361](../../../src/doc/canvas.rs#L361)

```rust
fn unpacked(&self) -> Vec<u8>
```

**What it does.** Builds a full-size copy of the pixels. A full canvas just clones its buffer.

**How it works (packed).**
1. Allocate a zeroed buffer of [`byte_len`](#canvasbyte_len) bytes. (The comment notes the
   memory pages only really get touched as the drawing is copied in, on as many threads as do
   the copying.)
2. With [`par_rows`](#par_rows), for each row `y` inside `rect.min_y..rect.max_y`, copy that
   row's packed bytes into columns `min_x..max_x` of the full row. Rows outside stay zero.

**Used by:** [`pixels`](#canvaspixels), [`pixels_mut`](#canvaspixels_mut),
[`into_pixels`](#canvasinto_pixels), [`unpack`](#canvasunpack).

### `Canvas::is_packed`
[src/doc/canvas.rs:382](../../../src/doc/canvas.rs#L382)

```rust
pub fn is_packed(&self) -> bool
```

True when the store is `Packed` (uses the `matches!` macro, a one-line `match` that returns a
bool). **Used by:** [../app.md](../app.md) (skips already-packed cells when packing idle ones).

### `Canvas::unpack`
[src/doc/canvas.rs:390](../../../src/doc/canvas.rs#L390)

```rust
pub fn unpack(&mut self)
```

**What it does.** Expands a packed canvas to full storage **ahead of** a write that is about to
happen, so the write itself does not pay for unpacking. The pixels and the `rev` stay the same.
The packed bytes are kept in `unwritten` (with the current rev) until something writes, so
packing again before then costs nothing. Does nothing for a full canvas.

**How it works.** Builds the full store, swaps it in with `std::mem::replace` (which returns the
old value), and moves the old packed rect + bytes into `unwritten`.

**Used by:** [../app.md](../app.md).

### `Canvas::unwritten`
[src/doc/canvas.rs:401](../../../src/doc/canvas.rs#L401)

```rust
fn unwritten(&self) -> Option<(DirtyRect, &[u8])>
```

Returns the kept packed rect and bytes **only if** the canvas is full and its rev still equals
the rev saved at [`unpack`](#canvasunpack) time (nothing has written since). Otherwise `None`.
**Used by:** [`held_rect`](#canvasheld_rect), [`pack`](#canvaspack),
[`content_bounds`](#canvascontent_bounds).

### `Canvas::rev`
[src/doc/canvas.rs:409](../../../src/doc/canvas.rs#L409)

```rust
pub fn rev(&self) -> u64
```

Returns the `rev` field: two canvases with the same rev hold the same pixels. **Used by:**
[../app.md](../app.md) (e.g. to know a saved snapshot of a cell is still current),
[../app/select.md](../app/select.md).

### `Canvas::held_rect`
[src/doc/canvas.rs:415](../../../src/doc/canvas.rs#L415)

```rust
pub fn held_rect(&self) -> Option<DirtyRect>
```

**What it does.** The rectangle this canvas actually holds pixels for; every byte outside it is
zero. That is the kept packed rect for an unpacked-but-unwritten canvas, the packed rect for a
packed canvas, or the whole canvas when full. Returns `None` when the canvas is blank (empty
rect). Uses `bool::then_some`, which turns `true`/`false` into `Some(value)`/`None`.

**Used by:** [`oriented`](#canvasoriented); [../app.md](../app.md).

### `Canvas::pack`
[src/doc/canvas.rs:431](../../../src/doc/canvas.rs#L431)

```rust
pub fn pack(&mut self) -> bool
```

**What it does.** Keeps only what is drawn: switches a full canvas to packed storage.

**Returns.** `true` if the canvas was full before (it changed), `false` if it was already packed.

**How it works.**
1. If [`unwritten`](#canvasunwritten) still holds the packed copy from an
   [`unpack`](#canvasunpack), move it back into the store — no scanning needed.
2. Otherwise, if the store is not `Full`, return `false` (`let … else` exits early).
3. Otherwise build a packed store with [`packed_from`](#canvaspacked_from) and replace the full
   one. `rev` is unchanged: the pixels are the same.

**Used by:** [../app.md](../app.md) (`pack_idle_cells`), [clip.md](clip.md) tests,
[../io/project_file.md](../io/project_file.md), [../krita_link.md](../krita_link.md).

### `Canvas::heap_bytes`
[src/doc/canvas.rs:447](../../../src/doc/canvas.rs#L447)

```rust
pub fn heap_bytes(&self) -> usize
```

**Test-only** (`#[cfg(test)]` means it is compiled only for `cargo test`). Returns how many bytes
of buffer this canvas owns on the heap: the store's buffer capacity plus any kept `unwritten`
bytes. Tests use it to prove packing really saves memory.

### `Canvas::px`
[src/doc/canvas.rs:455](../../../src/doc/canvas.rs#L455)

```rust
pub fn px(&self, x: u32, y: u32) -> [u8; 4]
```

**What it does.** Returns the pixel at `(x, y)` as `[r, g, b, a]`, reading in place. `(x, y)` must
be on the canvas (checked only in debug builds by `debug_assert!`).

**How it works.** Full: index `(y × width + x) × 4`. Packed: if the point is outside the rect,
return `[0; 4]` (four zeros); otherwise index `((y − min_y) × rect_width + (x − min_x)) × 4` into
the packed bytes. See the [worked example](#the-pixel-layout-same-as-a-browsers-imagedatadata).

**Used by:** [clip.md](clip.md), [../app/orient.md](../app/orient.md),
[../io/composite.md](../io/composite.md), [../tools/fill.md](../tools/fill.md),
[../ui/shell.md](../ui/shell.md).

### `Canvas::row_span`
[src/doc/canvas.rs:473](../../../src/doc/canvas.rs#L473)

```rust
pub fn row_span(&self, y: u32) -> (u32, &[u8])
```

**What it does.** Row `y` as it is stored, without copying.

**Returns.** `(x of the first stored pixel, the stored bytes from there)`. Every pixel of the row
outside that span is zero. A full canvas returns `(0, whole row)`; a packed canvas returns
`(rect.min_x, the packed row)`, or `(0, empty slice)` for rows outside the rect.

**Example.**
```rust
let (x0, span) = canvas.row_span(y);
for (i, px) in span.chunks_exact(4).enumerate() {
    let x = x0 as usize + i;      // real x of this pixel
    // px = [r, g, b, a]; pixels before x0 or after the span are all zero
}
```

**Used by:** [`read_row_into`](#canvasread_row_into); [../app.md](../app.md),
[../io/composite.md](../io/composite.md).

### `Canvas::read_rect`
[src/doc/canvas.rs:492](../../../src/doc/canvas.rs#L492)

```rust
pub fn read_rect(&self, rect: DirtyRect) -> Vec<u8>
```

**Test-only.** Copies `rect`'s pixels (which must be on the canvas) into a new buffer, one row at
a time via [`read_row_into`](#canvasread_row_into). A zero-width rect gives an all-zero (empty)
buffer.

### `Canvas::read_row_into`
[src/doc/canvas.rs:506](../../../src/doc/canvas.rs#L506)

```rust
pub fn read_row_into(&self, y: u32, x: u32, out: &mut [u8])
```

**What it does.** Copies part of row `y`, starting at pixel `x`, into `out` — as many pixels as
`out` has room for (the range must stay on the canvas). Works on full or packed canvases.

**How it works.**
1. Get the stored span with [`row_span`](#canvasrow_span): it covers pixels `x0..x1`.
2. The wanted range is `x..end`. Their overlap is `a..b` (`a = max(x0, x)`, `b = min(x1, end)`).
3. No overlap: fill `out` with zeros. Otherwise zero the part of `out` before `a`, copy the
   overlap from the span, and zero the part after `b`.

**Used by:** [`read_rect`](#canvasread_rect), [`recentered`](#canvasrecentered),
[`oriented`](#canvasoriented); [clip.md](clip.md), [../app.md](../app.md).

### `Canvas::recentered`
[src/doc/canvas.rs:524](../../../src/doc/canvas.rs#L524)

```rust
pub fn recentered(&self, new_w: u32, new_h: u32) -> Canvas
```

**What it does.** Returns a `new_w`×`new_h` copy with the drawing kept **centred**. Growing adds a
clear margin; shrinking crops the edges. The result is always packed, whatever this canvas was,
so resizing a layer's canvas never holds its cells at full size.

**How it works.**
1. `ox, oy` = where the old top-left lands in the new canvas: `(new − old) / 2`, negative when
   shrinking (done in `i64` so negatives are possible).
2. Find the rect that holds anything: the packed rect, or [`scan_bounds`](#scan_bounds) of a
   full buffer.
3. Shift that rect by `(ox, oy)` and clamp it to the new canvas. If nothing is left, return the
   blank new canvas.
4. Allocate just that rect and fill each row with [`read_row_into`](#canvasread_row_into) from
   the matching old row, then store it as `Packed`.

**Used by:** `recenter` in [project.md](project.md).

### `Canvas::oriented`
[src/doc/canvas.rs:557](../../../src/doc/canvas.rs#L557)

```rust
pub fn oriented(&self, o: crate::doc::orient::Orient) -> Canvas
```

**What it does.** Returns this canvas mirrored and/or turned by quarter turns (an
[`Orient`](orient.md)). This never resamples — pixels only move to other pixel positions. The
result is packed, and only the held part is read, so (as the comment puts it) an 8K drawing with
a doodle in one corner costs only the doodle. A quarter turn swaps width and height.

**How it works.**
1. New size = `o.dims(width, height)`; start from a blank canvas of that size.
2. If [`held_rect`](#canvasheld_rect) is `None`, return the blank canvas.
3. Copy the held rect out row by row (with [`read_row_into`](#canvasread_row_into)).
4. Work out where that rect lands in the turned canvas (`o.map_rect`) and its new size
   (`o.dims`), and store `o.permute(...)` of the small buffer as the packed pixels there.

**Used by:** [../app/orient.md](../app/orient.md).

### `Canvas::content_bounds`
[src/doc/canvas.rs:584](../../../src/doc/canvas.rs#L584)

```rust
pub fn content_bounds(&self) -> Option<DirtyRect>
```

**What it does.** The smallest rectangle containing every pixel with **any alpha** (colour under
zero alpha does not count), or `None` for a blank canvas. A packed (or unpacked-but-unwritten)
canvas only scans the bytes it holds, then shifts the result by the held rect's top-left so it is
in canvas coordinates. A full canvas scans everything with [`scan_bounds`](#scan_bounds) and
the [`ALPHA`](#const-alpha) mask.

**Used by:** [../app.md](../app.md).

### `Canvas::is_blank`
[src/doc/canvas.rs:600](../../../src/doc/canvas.rs#L600)

```rust
pub fn is_blank(&self) -> bool
```

Nothing drawn: no pixel has any alpha. Runs [`no_alpha`](#no_alpha) on whichever buffer is held
(an empty packed buffer is trivially blank). **Used by:** [../app.md](../app.md)
(`cell_is_blank`).

### `Canvas::clear`
[src/doc/canvas.rs:607](../../../src/doc/canvas.rs#L607)

```rust
pub fn clear(&mut self)
```

Clears to transparent. Since a blank packed canvas holds nothing, it simply replaces the store
with an empty packed one (freeing the buffer), takes a new rev, drops `unwritten`, and marks the
whole canvas dirty.

### `Canvas::mark_dirty`
[src/doc/canvas.rs:622](../../../src/doc/canvas.rs#L622)

```rust
pub fn mark_dirty(&mut self, x: u32, y: u32, w: u32, h: u32)
```

**What it does.** Records that the `w`×`h` area at `(x, y)` changed, by growing `dirty` to cover
it (the union of the old dirty rect and the new one). The right and bottom edges are clipped to
the canvas size.

**Parameters.** `x, y` — top-left of the changed area; `w, h` — its size in pixels.

**Used by:** painting code throughout, e.g. [../tools/stroke.md](../tools/stroke.md),
[../tools/fill.md](../tools/fill.md), [../tools/shape.md](../tools/shape.md),
[../tools/krita/mod.md](../tools/krita/mod.md), [../app/select.md](../app/select.md),
[../app/liquify.md](../app/liquify.md), [../ui/shell.md](../ui/shell.md).

## Tests

Run with `cargo test doc::canvas`.

| Test | What it checks |
|---|---|
| [`orienting_a_packed_cell_matches_the_unpacked_one`](../../../src/doc/canvas.rs#L664) | For all 8 orientations, `oriented` on a full and on a packed canvas gives the same size and pixels as permuting the whole buffer, with a new rev; a blank canvas stays blank. |
| [`writes_the_bytes_the_derive_wrote`](../../../src/doc/canvas.rs#L701) | The hand-written `serialize` produces the same postcard bytes as the old `#[derive]`d struct, also through an `Arc`, and a packed canvas writes the same bytes as its full form. |
| [`reads_what_the_derive_wrote_packed`](../../../src/doc/canvas.rs#L720) | Loading old-format bytes gives a packed canvas with the same pixels, holding only the 21×16 bounding rect, and clean (`dirty` is `None`). |
| [`packing_loses_nothing_and_reads_the_same`](../../../src/doc/canvas.rs#L737) | A packed copy matches the full one for `pixels`, `content_bounds`, `px`, `read_rect` and `row_span`; packing twice returns `false`; writing unpacks and keeps the drawing. |
| [`unpacking_ahead_of_a_write_packs_back_for_free`](../../../src/doc/canvas.rs#L765) | `unpack` keeps the rev and held rect and repacks to just the drawing; after a write, `held_rect` is the whole canvas and packing still keeps everything. |
| [`a_blank_canvas_holds_nothing`](../../../src/doc/canvas.rs#L789) | A new 2160×2880 canvas uses 0 heap bytes, is blank, reads zeros; `clear` frees the buffer. |
| [`pixels_that_dont_fit_the_size_are_an_error`](../../../src/doc/canvas.rs#L800) | Loading a 4×4 canvas with 10 pixel bytes fails instead of producing a bad canvas. |
| [`content_bounds_wraps_every_inked_pixel`](../../../src/doc/canvas.rs#L810) | `content_bounds` ignores colour with zero alpha, wraps every pixel with alpha ≥ 1, and gives the same answer after packing. |

**Test helpers:**
- `Derived` ([line 647](../../../src/doc/canvas.rs#L647)) — a struct with `width`, `height`,
  `pixels: Vec<u8>` and `#[derive(Serialize, Deserialize)]`: the exact old on-disk shape of a
  canvas, used to prove the format did not change.
- `inked` ([line 653](../../../src/doc/canvas.rs#L653)) — a `w`×`h` canvas with every byte set to
  a varying non-zero-ish pattern (`i * 7 % 251`).
- `sparse` ([line 687](../../../src/doc/canvas.rs#L687)) — a 40×30 canvas with a 4×4 opaque square
  at (10..14, 5..9) and one "red but transparent" pixel at (30, 20), so packed bounds
  (by any byte) and content bounds (by alpha) differ.

### `mod tests`

Run with `cargo test doc::canvas::tests` · 8 tests

| Test | What it checks |
|---|---|
| [`orienting_a_packed_cell_matches_the_unpacked_one`](../../../src/doc/canvas.rs#L664) | Orienting reads only what a packed canvas holds, and must come out the same as turning the whole buffer over. |
| [`writes_the_bytes_the_derive_wrote`](../../../src/doc/canvas.rs#L701) | Writes the bytes the derive wrote. |
| [`reads_what_the_derive_wrote_packed`](../../../src/doc/canvas.rs#L720) | Reads what the derive wrote packed. |
| [`packing_loses_nothing_and_reads_the_same`](../../../src/doc/canvas.rs#L737) | Packing loses nothing and reads the same. |
| [`unpacking_ahead_of_a_write_packs_back_for_free`](../../../src/doc/canvas.rs#L765) | Unpacked ahead of a write, a canvas reads the same, keeps its rev, and packs back for free; once written, it packs as ever. |
| [`a_blank_canvas_holds_nothing`](../../../src/doc/canvas.rs#L789) | A blank canvas holds nothing. |
| [`pixels_that_dont_fit_the_size_are_an_error`](../../../src/doc/canvas.rs#L800) | Pixels that dont fit the size are an error. |
| [`content_bounds_wraps_every_inked_pixel`](../../../src/doc/canvas.rs#L810) | Content bounds wraps every inked pixel. |

**Test helpers** (used only by the tests above):

- [`inked`](../../../src/doc/canvas.rs#L653) — Inked.
- [`sparse`](../../../src/doc/canvas.rs#L687) — A drawing in one corner, with colour under zero alpha around it.

**Test-only types and constants:** [`Derived`](../../../src/doc/canvas.rs#L647)
