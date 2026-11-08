# `src/tools/krita/tip/gbr.rs` — GIMP brush tips (.gbr and .gih)

> **Source:** [src/tools/krita/tip/gbr.rs](../../../../../src/tools/krita/tip/gbr.rs) · **Module:** `crate::tools::krita::tip::gbr` · **Size:** 318 lines · Line links as of commit `36405ad`

## What this file is for

Many Krita presets use **GIMP brush** files as tips:

- **`.gbr`** — one grayscale brush image with a small binary header (big-endian sizes, name, spacing).
- **`.gih`** — an *image pipe*: several `.gbr` cells plus a text header ("parasite") saying how a dab picks its
  cell — constant, incremental, random, or by angle, pressure, tilt or speed, possibly in several dimensions.

This file parses both and walks a pipe's cells dab by dab (`PipeState::prepare`) as Krita's
`KisImagePipeBrush` does.

```text
.gih header:  "ncells:4 cellwidth:64 … dim:1 rank0:4 sel0:random"
cells:        [gbr][gbr][gbr][gbr]   → each dab picks one at random
```

## How it connects

- **Used by:** [tools/krita/mod.rs](../mod.md) (`Tip::Pipe`, cell per dab), [tools/krita/tip/mod.rs](mod.md)
  (cells become bitmap tips).

## Rust concepts in this file

**Parsing binary headers.** `be32(d, at)` reads a big-endian `u32` — `DataView.getUint32(at, false)` in JS — and
returns a `Result` so truncated files are errors, not crashes.

**Callbacks for randomness.** `prepare(…, random: &mut impl FnMut(i32, i32) -> i32)` takes the random generator as a
closure, so tests can pass a deterministic one.

## Types

### `struct GbrCell`
[src/tools/krita/tip/gbr.rs:16](../../../../../src/tools/krita/tip/gbr.rs#L16)

A grayscale cell as Krita holds it: `255 - byte`, so the mask a dab takes from it (`255 - gray`) is the file's own byte.

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | Cell width. |
| `height` | `u32` | Cell height. |
| `gray` | `Vec<u8>` | Pixel values as Krita holds them (`255 − byte`). |

### `enum Selection`
[src/tools/krita/tip/gbr.rs:63](../../../../../src/tools/krita/tip/gbr.rs#L63)

`KisParasite::SelectionMode`: how a pipe dimension picks its index.

| Variant | Meaning |
|---|---|
| `Constant` | Always the same index. |
| `Incremental` | The next index each dab. |
| `Angular` | By drawing direction. |
| `Random` | A random index each dab. |
| `Pressure` | By pen pressure. |
| `TiltX` | By tilt along x. |
| `TiltY` | By tilt along y. |
| `Velocity` | By speed. |

### `const MAX_DIM`
[src/tools/krita/tip/gbr.rs:74](../../../../../src/tools/krita/tip/gbr.rs#L74)

```rust
const MAX_DIM: usize = 4;
```

The most dimensions a pipe may have (4).

### `struct Parasite`
[src/tools/krita/tip/gbr.rs:78](../../../../../src/tools/krita/tip/gbr.rs#L78)

`KisPipeBrushParasite`, parsed from the `.gih` header.

| Field | Type | Meaning |
|---|---|---|
| `dim` | `usize` | Number of dimensions. |
| `ncells` | `i32` | Total cells. |
| `rank` | `[i32; MAX_DIM]` | Cells along each dimension. |
| `selection` | `[Selection; MAX_DIM]` | How each dimension picks its index. |
| `brushes_count` | `[i32; MAX_DIM]` | Cells per step of each dimension. |

### `struct PipeInput`
[src/tools/krita/tip/gbr.rs:175](../../../../../src/tools/krita/tip/gbr.rs#L175)

What a pipe's selection reads from the dab.

| Field | Type | Meaning |
|---|---|---|
| `pressure` | `f64` | Pen pressure. |
| `drawing_angle` | `f64` | Direction of travel. |
| `x_tilt` | `f64` | Tilt x. |
| `y_tilt` | `f64` | Tilt y. |
| `speed` | `f64` | Speed. |

### `struct PipeState`
[src/tools/krita/tip/gbr.rs:185](../../../../../src/tools/krita/tip/gbr.rs#L185)

One stroke's walk through a pipe's cells.

| Field | Type | Meaning |
|---|---|---|
| `index` | `[i32; MAX_DIM]` | Current index along each dimension. |
| `current` | `usize` | The cell picked for the current dab. |

## Free functions

### `be32`
[src/tools/krita/tip/gbr.rs:22](../../../../../src/tools/krita/tip/gbr.rs#L22)

```rust
fn be32(d: &[u8], at: usize) -> Result<u32>
```

Reads a big-endian 32-bit number at `at`, or an error if the data is too short.

### `parse_gbr`
[src/tools/krita/tip/gbr.rs:28](../../../../../src/tools/krita/tip/gbr.rs#L28)

```rust
pub fn parse_gbr(data: &[u8]) -> Result<(GbrCell, usize)>
```

`KisGbrBrush::init` on `data`: the cell, and how many bytes it took.

## `impl Parasite`

### `Parasite::parse`
[src/tools/krita/tip/gbr.rs:88](../../../../../src/tools/krita/tip/gbr.rs#L88)

```rust
pub fn parse(source: &str) -> Self
```

`KisPipeBrushParasite(const QString&)` then `sanitize()`.

## Free functions

### `parse_gih`
[src/tools/krita/tip/gbr.rs:149](../../../../../src/tools/krita/tip/gbr.rs#L149)

```rust
pub fn parse_gih(data: &[u8]) -> Result<(Parasite, Vec<GbrCell>)>
```

A `.gih`: the header's parasite and its cells.

## `impl PipeState`

### `PipeState::new`
[src/tools/krita/tip/gbr.rs:191](../../../../../src/tools/krita/tip/gbr.rs#L191)

```rust
pub fn new() -> Self
```

A fresh walk (all indices 0).

### `PipeState::prepare`
[src/tools/krita/tip/gbr.rs:200](../../../../../src/tools/krita/tip/gbr.rs#L200)

```rust
pub fn prepare(&mut self, p: &Parasite, cells: usize, input: &PipeInput, seq_no: i32, random: &mut impl FnMut(i32, i32) -> i32) -> usize
```

`prepareForSeqNo`: advance the indices for dab `seq_no` (drawing from `random` for a random dimension), then pick the cell.

## Free functions

### `qround`
[src/tools/krita/tip/gbr.rs:236](../../../../../src/tools/krita/tip/gbr.rs#L236)

```rust
fn qround(d: f64) -> i32
```

`qRound` (halves round up).

## Tests

### `mod tests`

Run with `cargo test tools::krita::tip::gbr::tests` · 4 tests

| Test | What it checks |
|---|---|
| [`a_gray_gbr_is_its_inverse`](../../../../../src/tools/krita/tip/gbr.rs#L261) | A gray gbr is its inverse. |
| [`the_shipped_pipe_parses_as_seven_random_cells`](../../../../../src/tools/krita/tip/gbr.rs#L269) | The shipped pipe parses as seven random cells. |
| [`krita_s_test_brushes_read_as_krita_reads_them`](../../../../../src/tools/krita/tip/gbr.rs#L280) | Krita's own test brushes (libs/brush/tests/data). |
| [`selection_walks_like_krita`](../../../../../src/tools/krita/tip/gbr.rs#L297) | Selection walks like krita. |

**Test helpers** (used only by the tests above):

- [`gbr`](../../../../../src/tools/krita/tip/gbr.rs#L248) — Gbr.
