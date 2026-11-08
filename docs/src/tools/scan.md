# `src/tools/scan.rs` — fast byte scans, eight bytes at a time

> **Source:** [src/tools/scan.rs](../../../src/tools/scan.rs) · **Module:** `crate::tools::scan` · **Size:** 150 lines · Line links as of commit `1fa5845`

## What this file is for

Selection masks and flood-fill maps are big arrays of bytes, mostly long runs of the same value (all 0 or
all 255). Checking them one byte at a time is slow on an 8K canvas (33 million bytes). These helpers read
**eight bytes at once as one 64-bit number** and compare whole words, skipping uniform runs eight bytes
per step. `flood` is a scanline flood fill built on them.

### The word trick, worked through

To test "are these 8 bytes all equal to `v`?", build a pattern with `v` in every byte and XOR:

```text
v = 0xFF           pat  = 0xFFFF_FFFF_FFFF_FFFF   (ONES * v: 0x0101…01 × 0xFF)
bytes = FF FF FF 00 FF FF FF FF   (memory order)
word  = 0xFFFF_FFFF_00FF_FFFF   (little-endian: first byte is the lowest)
word ^ pat = 0x0000_0000_FF00_0000
                                 ↑ nonzero → not all equal
trailing_zeros = 24 → 24 / 8 = byte 3 is the first different one
```

So one XOR and one `trailing_zeros` (a single CPU instruction) find the first byte that differs among
eight.

## How it connects

- **Used by:** [tools/select_mask.rs](select_mask.md), [tools/selection.rs](selection.md),
  [tools/wand.rs](wand.md), [tools/fill.rs](fill.md).

## Rust concepts in this file

**Bit operations on `u64`.** `^` (XOR), `trailing_zeros()`, `leading_zeros()` — same operators as JS, but
on true 64-bit integers (JS needs `BigInt` for that).

**`chunks_exact(8)` and `remainder()`.** Walk a slice in 8-byte chunks; the leftover (<8) bytes are handled
separately. **`try_into()`** converts an 8-byte slice to a `[u8; 8]` array (failing if the length is wrong).
More: [Iterators](../../rust-primer.md#iterators)

## Types

### `const ONES`
[src/tools/scan.rs:5](../../../src/tools/scan.rs#L5)

```rust
const ONES: u64 = 0x0101_0101_0101_0101;
```

`0x0101…01`: multiplying a byte by it repeats the byte in all eight positions.

### `const HIGHS`
[src/tools/scan.rs:6](../../../src/tools/scan.rs#L6)

```rust
const HIGHS: u64 = 0x8080_8080_8080_8080;
```

`0x8080…80`: the top bit of every byte, used in "has a zero byte" tests.

### `const SHUT`
[src/tools/scan.rs:67](../../../src/tools/scan.rs#L67)

```rust
pub const SHUT: u8 = 0;
```

A pixel of a `flood` map that the flood may not enter.

### `const OPEN`
[src/tools/scan.rs:69](../../../src/tools/scan.rs#L69)

```rust
pub const OPEN: u8 = 1;
```

`flood` map value: may be entered, not reached yet.

### `const TAKEN`
[src/tools/scan.rs:71](../../../src/tools/scan.rs#L71)

```rust
pub const TAKEN: u8 = 2;
```

`flood` map value: reached by the flood.

## Free functions

### `word`
[src/tools/scan.rs:9](../../../src/tools/scan.rs#L9)

```rust
fn word(c: &[u8]) -> u64
```

Reads 8 bytes as a little-endian `u64`.

### `run_of`
[src/tools/scan.rs:15](../../../src/tools/scan.rs#L15)

```rust
pub fn run_of(s: &[u8], v: u8) -> usize
```

How many bytes at the start of `s` equal `v`: XOR each 8-byte word with the repeated pattern; the first
nonzero result's `trailing_zeros() / 8` locates the first differing byte; the tail shorter than 8 is checked
byte by byte.

### `run_of_back`
[src/tools/scan.rs:32](../../../src/tools/scan.rs#L32)

```rust
pub fn run_of_back(s: &[u8], v: u8) -> usize
```

The same from the end of `s` (uses `leading_zeros`, since the last byte in memory is the highest).

### `find`
[src/tools/scan.rs:48](../../../src/tools/scan.rs#L48)

```rust
pub fn find(s: &[u8], v: u8) -> Option<usize>
```

The index of the first `v` in `s`, using a word-at-a-time "does this word contain byte v" test.

### `flood`
[src/tools/scan.rs:76](../../../src/tools/scan.rs#L76)

```rust
pub fn flood(map: &mut [u8], w: usize, seed: (i32, i32)) -> (i32, i32, i32, i32)
```

A scanline flood fill over a byte map `w` wide, from `seed`: every `OPEN` pixel connected (side by side) to
the seed becomes `TAKEN`. It fills whole horizontal runs at a time (found with `run_of`), then looks at the
rows above and below for new runs to visit. Returns the bounding box of what was taken.

## Tests

### `mod tests`

Run with `cargo test tools::scan::tests` · 1 tests

| Test | What it checks |
|---|---|
| [`scans_match_the_plain_loops`](../../../src/tools/scan.rs#L116) | Every length and position up to a few words, against the plain loops. |
