# `src/tools/paper.rs` — paper grain texture

> **Source:** [src/tools/paper.rs](../../../src/tools/paper.rs) · **Module:** `crate::tools::paper` · **Size:** 198 lines · Line links as of commit `1fa5845`

## What this file is for

Real paper has a "tooth" — small bumps that catch graphite unevenly. This file builds a **paper grain
field**: a 256×256 tileable texture of multi-octave *value noise* (smooth random bumps at four scales,
weighted so the fine detail dominates). Brushes sample it by canvas position to eat into their coverage
(`grain` setting), so strokes look like pencil on paper rather than flat colour. It's built once per run
and shared.

```text
octave 1 (4 cells)    octave 2 (16)    octave 3 (32)    octave 4 (64)
 large soft blobs  +   medium        +   finer        +   finest     = paper tooth
 weight 0.12           0.18              0.25             0.45
```

## How it connects

- **Used by:** [tools/ribbon.rs](ribbon.md) (grain on app brushes), [tools/noise.rs](noise.md).
- Not the same as Krita's paper texture (see [tools/krita/texture.rs](krita/texture.md)) or the fill grain
  ([tools/fill.rs](fill.md)).

## Rust concepts in this file

**Lazy global with `OnceLock`.**
```rust
static PAPER: OnceLock<Paper> = OnceLock::new();
PAPER.get_or_init(Paper::new)
```
Built the first time `paper()` is called, then shared (`&'static Paper`) — a thread-safe lazy singleton.
More: [Constants and statics](../../rust-primer.md#constants-and-statics)

**Wrapping integer hashes.** `wrapping_mul` lets multiplication overflow on purpose — the basis of fast
integer hash functions.

## Types

### `const TILE`
[src/tools/paper.rs:18](../../../src/tools/paper.rs#L18)

```rust
const TILE: usize = 256;
```

Tile edge in texels. The field wraps at this period, so every octave's lattice count has to divide it.

### `const OCTAVES`
[src/tools/paper.rs:23](../../../src/tools/paper.rs#L23)

```rust
const OCTAVES: [(usize, f32); 4] = [(4, 0.12), (16, 0.18), (32, 0.25), (64, 0.45)];
```

(lattice cells across the tile, weight). Wide-to-fine, weighted so the finest octave dominates — pencil tooth is mostly high frequency, with just enough coarse structure to keep it from looking like TV static.

### `struct Paper`
[src/tools/paper.rs:25](../../../src/tools/paper.rs#L25)

The baked grain tile.

| Field | Type | Meaning |
|---|---|---|
| `tile` | `Vec<f32>` | 256 × 256 grain values (0–1), row by row. |

## Free functions

### `paper`
[src/tools/paper.rs:31](../../../src/tools/paper.rs#L31)

```rust
pub fn paper() -> &'static Paper
```

The single shared grain field, built on first use (a few hundred microseconds, 256 KB).

## `impl Paper`

### `Paper::new`
[src/tools/paper.rs:37](../../../src/tools/paper.rs#L37)

```rust
fn new() -> Self
```

Bakes the tile: for each texel, sums the four octaves of `value_noise` with their weights and normalises
the result to 0–1.

### `Paper::sample`
[src/tools/paper.rs:72](../../../src/tools/paper.rs#L72)

```rust
pub fn sample(&self, u: f32, v: f32) -> f32
```

Bilinearly samples the field at texel coordinates `(u, v)`, wrapping around (so any coordinate works and the
texture repeats seamlessly).

## Free functions

### `wrap`
[src/tools/paper.rs:93](../../../src/tools/paper.rs#L93)

```rust
fn wrap(i: i64) -> usize
```

Wraps an integer coordinate into `0..256` (`rem_euclid` keeps negatives positive).

### `value_noise`
[src/tools/paper.rs:100](../../../src/tools/paper.rs#L100)

```rust
fn value_noise(u: f32, v: f32, cells: usize) -> f32
```

One octave of value noise: random values at the corners of a `cells × cells` lattice (from `hash01`),
smoothly interpolated with `smoothstep`. Lattice indices wrap at `cells`, which makes the tile seamless.

### `smoothstep`
[src/tools/paper.rs:121](../../../src/tools/paper.rs#L121)

```rust
fn smoothstep(t: f32) -> f32
```

The S-curve `3t² − 2t³`: eases interpolation in and out so the noise has no visible grid lines.

### `hash01`
[src/tools/paper.rs:128](../../../src/tools/paper.rs#L128)

```rust
fn hash01(x: u32, y: u32, salt: u32) -> f32
```

A fast integer hash of `(x, y, salt)` to a pseudo-random value in 0–1 (same input, same output). The salt
gives each octave its own random values.

## Tests

### `mod tests`

Run with `cargo test tools::paper::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`the_field_is_seamless`](../../../src/tools/paper.rs#L144) | The field is seamless. |
| [`sampling_is_deterministic_and_in_range`](../../../src/tools/paper.rs#L162) | Sampling is deterministic and in range. |
| [`the_field_has_structure_at_more_than_one_scale`](../../../src/tools/paper.rs#L177) | The whole point of going multi-octave: a single-texel hash has no correlation between neighbours, so it reads as speckle. Real tooth has neighbouring texels agreeing more often than chance. |
