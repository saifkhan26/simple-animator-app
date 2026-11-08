# `src/io/kra/tiles.rs` — Krita's tiled pixel files

> **Source:** [src/io/kra/tiles.rs](../../../../src/io/kra/tiles.rs) · **Module:** `crate::io::kra::tiles` · **Size:** 373 lines · Line links as of commit `67a7cb9`

## What this file is for

Inside a `.kra`, each drawing's pixels are stored in Krita's **tile file** format: the image is cut into
64×64-pixel tiles, fully transparent tiles are left out, and each stored tile is compressed with **LZF** (a
very fast, simple compression). This file encodes and decodes that format.

```text
VERSION 2
TILEWIDTH 64
TILEHEIGHT 64
PIXELSIZE 4
DATA <count>
<x>,<y>,LZF,<len>      ← one header line per tile: its top-left and compressed length
<len bytes>            ← 1 flag byte (0 raw / 1 LZF) + the data
…repeated <count> times
```

Two Krita quirks: pixels are **BGRA**, not RGBA; and inside a tile the bytes are **planar** — all pixels'
byte 0, then all byte 1, etc. — which compresses better than interleaved pixels.

```text
interleaved (RGBA):  B G R A  B G R A  B G R A …
planar (Krita):      B B B …  G G G …  R R R …  A A A …
```

## How it connects

- **Used by:** [io/kra/mod.rs](mod.md) (`write` encodes each drawing, `Reader::drawing` decodes).
- **Uses:** nothing else in the crate.

## Rust concepts in this file

**Working with raw bytes.** `&[u8]` slices and index arithmetic (`out[i * PIXEL + c] = planar[c * n + i]`)
— like working with a `Uint8Array` in JS.

**Byte literals.** `b'\n'` is the byte value of a newline (10), comparable to `"\n".charCodeAt(0)`.

**`std::str::from_utf8`** turns bytes into a `&str` only if they are valid UTF-8, returning a `Result`.
More: [Strings and slices](../../../rust-primer.md#strings-and-slices)

## Types

### `const TILE`
[src/io/kra/tiles.rs:24](../../../../src/io/kra/tiles.rs#L24)

```rust
pub const TILE: i32 = 64;
```

Tile side in pixels (64).

### `const PIXEL`
[src/io/kra/tiles.rs:25](../../../../src/io/kra/tiles.rs#L25)

```rust
const PIXEL: usize = 4;
```

Bytes per pixel (4).

### `const TILE_BYTES`
[src/io/kra/tiles.rs:26](../../../../src/io/kra/tiles.rs#L26)

```rust
const TILE_BYTES: usize = (TILE * TILE) as usize * PIXEL;
```

Bytes in one uncompressed tile: 64 × 64 × 4 = 16384.

### `const FLAG_RAW`
[src/io/kra/tiles.rs:28](../../../../src/io/kra/tiles.rs#L28)

```rust
const FLAG_RAW: u8 = 0;
```

Tile data flag: stored uncompressed.

### `const FLAG_LZF`
[src/io/kra/tiles.rs:29](../../../../src/io/kra/tiles.rs#L29)

```rust
const FLAG_LZF: u8 = 1;
```

Tile data flag: LZF-compressed.

### `struct Decoded`
[src/io/kra/tiles.rs:34](../../../../src/io/kra/tiles.rs#L34)

Pixels decoded from a tile file: the bounding box of every tile present, in device coordinates, as unpremultiplied RGBA8. Space no tile covers holds the device's default pixel.

| Field | Type | Meaning |
|---|---|---|
| `x` | `i32` | Left of the tile bounding box, in Krita device coordinates. |
| `y` | `i32` | Top of the box. |
| `w` | `u32` | Box width. |
| `h` | `u32` | Box height. |
| `pixels` | `Vec<u8>` | RGBA8 pixels of the box (gaps filled with the default pixel). |

## Free functions

### `encode`
[src/io/kra/tiles.rs:44](../../../../src/io/kra/tiles.rs#L44)

```rust
pub fn encode(pixels: &[u8], w: u32, h: u32, ox: i32, oy: i32) -> Vec<u8>
```

Encodes an RGBA buffer (top-left at device `(ox, oy)`) as a tile file: walks the 64×64 tile grid covering
it, skips fully transparent tiles, converts each tile to planar BGRA, compresses it with LZF (or stores it
raw if that's not smaller), and writes the header lines and data.

### `decode`
[src/io/kra/tiles.rs:99](../../../../src/io/kra/tiles.rs#L99)

```rust
pub fn decode(bytes: &[u8], default: [u8; 4]) -> Result<Option<Decoded>>
```

Decodes a tile file: parses the text header, reads each tile (decompressing LZF, undoing the planar
layout, BGRA → RGBA), and assembles them into one bounding-box image, filling gaps with `default`. Returns
`Ok(None)` if the file contains no tiles.

### `next_line`
[src/io/kra/tiles.rs:186](../../../../src/io/kra/tiles.rs#L186)

```rust
fn next_line<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a str>
```

Reads the text line starting at byte `*pos` and moves `*pos` past its newline. The lifetime `'a` says the
returned `&str` points into `bytes`.

### `delinearize`
[src/io/kra/tiles.rs:195](../../../../src/io/kra/tiles.rs#L195)

```rust
fn delinearize(planar: &[u8], out: &mut [u8])
```

Converts Krita's planar layout back to interleaved pixels: byte `c` of pixel `i` comes from
`planar[c * n + i]`.

### `lzf_decompress`
[src/io/kra/tiles.rs:206](../../../../src/io/kra/tiles.rs#L206)

```rust
fn lzf_decompress(input: &[u8], out: &mut [u8]) -> Result<usize>
```

Decompresses plain LZF data (liblzf's format). LZF is a stream of instructions: a control byte below 32
means "copy the next N+1 literal bytes"; otherwise it encodes a back-reference "copy L bytes from D bytes
back in the output". Returns the number of bytes produced, with errors on malformed input.

## Tests

### `mod tests`

Run with `cargo test io::kra::tiles::tests` · 7 tests

| Test | What it checks |
|---|---|
| [`lzf_back_reference_repeats_overlapping_run`](../../../../src/io/kra/tiles.rs#L260) | Lzf back reference repeats overlapping run. |
| [`lzf_extended_length`](../../../../src/io/kra/tiles.rs#L270) | Lzf extended length. |
| [`lzf_rejects_reference_before_start`](../../../../src/io/kra/tiles.rs#L280) | Lzf rejects reference before start. |
| [`raw_round_trip_with_negative_unaligned_origin`](../../../../src/io/kra/tiles.rs#L316) | Raw round trip with negative unaligned origin. |
| [`empty_buffer_writes_no_tiles`](../../../../src/io/kra/tiles.rs#L327) | Empty buffer writes no tiles. |
| [`decodes_compressed_planar_tile`](../../../../src/io/kra/tiles.rs#L334) | Decodes compressed planar tile. |
| [`gaps_between_tiles_take_the_default_pixel`](../../../../src/io/kra/tiles.rs#L360) | Gaps between tiles take the default pixel. |

**Test helpers** (used only by the tests above):

- [`lzf_literal_only`](../../../../src/io/kra/tiles.rs#L250) — A tiny LZF *compressor* for tests only — enough to produce the planar, compressed tiles Krita writes, so the decoder is exercised on the real code path rather than only on raw tiles.
- [`sample`](../../../../src/io/kra/tiles.rs#L285) — Sample.
- [`crop`](../../../../src/io/kra/tiles.rs#L299) — Crop a decoded device back to a `w`×`h` window at `(ox, oy)`.
