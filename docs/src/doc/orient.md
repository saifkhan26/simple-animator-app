# `src/doc/orient.rs` — lossless flips and quarter turns

> **Source:** [src/doc/orient.rs](../../../src/doc/orient.rs) · **Module:** `crate::doc::orient` · **Size:** 192 lines · Line links as of commit `36405ad`

## What this file is for

There are exactly eight ways to flip or rotate a picture by multiples of 90° — mirror or not, times 0, 1,
2 or 3 clockwise quarter turns. Each only *moves* pixels to other pixel positions (no resampling, no blur),
so doing one and then its opposite gives back every byte. `Orient` represents one of those eight; it can
map a pixel position, a rectangle, or a whole pixel buffer.

Used by the "Flip art horizontally/vertically" and "Rotate art 90°/180°" commands, which turn the
selection or whole drawings.

```text
FLIP_H        CW (quarter = 1)      HALF (quarter = 2)
 AB → BA       AB      CA            AB → DC
 CD   DC       CD  →   DB            CD   BA
```

## How it connects

- **Used by:** [app/orient.rs](../app/orient.md) (applying turns to drawings and selections),
  [app.rs](../app.md) (`dispatch` art flip/rotate actions), [tools/selection.rs](../tools/selection.md).
- **Uses:** nothing else in the crate.

## Rust concepts in this file

**Associated constants.** `Orient::FLIP_H`, `Orient::CW` are constants defined inside `impl Orient` —
like static readonly fields on a JS class. More: [Constants and statics](../../rust-primer.md#constants-and-statics)

**`Copy` value type with chaining.** `Orient::CW.then(Orient::FLIP_H)` combines two operations into one
`Orient` value without changing either.

## Types

### `struct Orient`
[src/doc/orient.rs:10](../../../src/doc/orient.rs#L10)

One of the eight lossless orientations: an optional horizontal mirror, then `quarter` clockwise quarter
turns.

| Field | Type | Meaning |
|---|---|---|
| `quarter` | `u8` | Number of clockwise quarter turns (0–3). |
| `mirror` | `bool` | Whether a left–right mirror is applied first. |

### `const Orient::IDENTITY`
[src/doc/orient.rs:16](../../../src/doc/orient.rs#L16)

```rust
pub const IDENTITY: Orient = Orient {
    quarter: 0,
    mirror: false,
};
```

No change.

### `const Orient::FLIP_H`
[src/doc/orient.rs:21](../../../src/doc/orient.rs#L21)

```rust
pub const FLIP_H: Orient = Orient {
    quarter: 0,
    mirror: true,
};
```

Left and right swap.

### `const Orient::FLIP_V`
[src/doc/orient.rs:26](../../../src/doc/orient.rs#L26)

```rust
pub const FLIP_V: Orient = Orient {
    quarter: 2,
    mirror: true,
};
```

Top and bottom swap: a left-right mirror turned half way round.

### `const Orient::CW`
[src/doc/orient.rs:30](../../../src/doc/orient.rs#L30)

```rust
pub const CW: Orient = Orient {
    quarter: 1,
    mirror: false,
};
```

A quarter turn clockwise.

### `const Orient::CCW`
[src/doc/orient.rs:34](../../../src/doc/orient.rs#L34)

```rust
pub const CCW: Orient = Orient {
    quarter: 3,
    mirror: false,
};
```

A quarter turn anticlockwise (three clockwise).

### `const Orient::HALF`
[src/doc/orient.rs:38](../../../src/doc/orient.rs#L38)

```rust
pub const HALF: Orient = Orient {
    quarter: 2,
    mirror: false,
};
```

A half turn (180°).

## `impl Orient`

### `Orient::is_identity`
[src/doc/orient.rs:43](../../../src/doc/orient.rs#L43)

```rust
pub fn is_identity(self) -> bool
```

Whether it changes nothing.

### `Orient::then`
[src/doc/orient.rs:50](../../../src/doc/orient.rs#L50)

```rust
pub fn then(self, next: Orient) -> Orient
```

The single `Orient` equal to doing `self` and then `next`. A mirror reverses the direction of turns done
before it, which is all the arithmetic needed.

### `Orient::swaps_axes`
[src/doc/orient.rs:65](../../../src/doc/orient.rs#L65)

```rust
pub fn swaps_axes(self) -> bool
```

Whether width and height trade places (an odd number of quarter turns).

### `Orient::dims`
[src/doc/orient.rs:70](../../../src/doc/orient.rs#L70)

```rust
pub fn dims(self, w: u32, h: u32) -> (u32, u32)
```

The output size of a `w`×`h` image.

### `Orient::map`
[src/doc/orient.rs:79](../../../src/doc/orient.rs#L79)

```rust
pub fn map(self, x: u32, y: u32, w: u32, h: u32) -> (u32, u32)
```

Where pixel `(x, y)` of a `w`×`h` image ends up. For example a clockwise turn sends `(x, y)` to
`(h − 1 − y, x)`.

### `Orient::map_rect`
[src/doc/orient.rs:93](../../../src/doc/orient.rs#L93)

```rust
pub fn map_rect(self, x: u32, y: u32, w: u32, h: u32, cw: u32, ch: u32) -> (u32, u32)
```

Where a rectangle inside an image lands (its new top-left), by mapping two opposite corners.

### `Orient::permute`
[src/doc/orient.rs:100](../../../src/doc/orient.rs#L100)

```rust
pub fn permute(self, src: &[u8], w: u32, h: u32, bpp: usize) -> Vec<u8>
```

Turns a whole buffer of `w`×`h` pixels with `bpp` bytes each, returning the new buffer. Works for RGBA
(4 bytes) and single-channel masks (1 byte) alike.

## Tests

### `mod tests`

Run with `cargo test doc::orient::tests` · 5 tests

| Test | What it checks |
|---|---|
| [`four_quarters_and_two_flips_are_identity`](../../../src/doc/orient.rs#L138) | Four quarters and two flips are identity. |
| [`the_named_ones_do_what_they_say`](../../../src/doc/orient.rs#L152) | The named ones do what they say. |
| [`then_composes_like_applying_twice`](../../../src/doc/orient.rs#L163) | Then composes like applying twice. |
| [`half_turn_is_both_flips`](../../../src/doc/orient.rs#L174) | Half turn is both flips. |
| [`a_rect_lands_where_its_pixels_do`](../../../src/doc/orient.rs#L180) | A rect lands where its pixels do. |

**Test helpers** (used only by the tests above):

- [`all`](../../../src/doc/orient.rs#L126) — All.
- [`apply`](../../../src/doc/orient.rs#L132) — Apply.

**Test-only types and constants:** [`IMG`](../../../src/doc/orient.rs#L124)
