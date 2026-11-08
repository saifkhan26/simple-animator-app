# `src/tools/krita/random.rs` — Krita's random number source

> **Source:** [src/tools/krita/random.rs](../../../../src/tools/krita/random.rs) · **Module:** `crate::tools::krita::random` · **Size:** 157 lines · Line links as of commit `1fa5845`

## What this file is for

Scatter, "fuzzy" sensors, a pipe tip's cell choice and an auto tip's density all draw random numbers. To
paint like Krita (and to make tests reproducible), this file reimplements Krita's generator exactly: boost's
**taus88** — three small linear-feedback shift registers ("Tausworthe" generators) whose outputs are XORed
together. Seeded, it always produces the same sequence; each stroke gets a different seed unless a test fixes
it.

## How it connects

- **Used by:** [tools/krita/sensors.rs](sensors.md) (fuzzy sensors), [tools/krita/mod.rs](mod.md) (scatter,
  `seed`), [tools/krita/tip/auto.rs](tip/auto.md) (density).

## Rust concepts in this file

**Const generics.** `struct Lfsr<const K: u32, const Q: u32, const S: u32>` takes numbers as type parameters, so
`Lfsr<31, 13, 12>` and `Lfsr<29, 2, 4>` are different types with the shift amounts baked in at compile time —
like a template in C++. More: [Generics](../../../rust-primer.md#generics)

## Types

### `struct Lfsr`
[src/tools/krita/random.rs:14](../../../../src/tools/krita/random.rs#L14)

`linear_feedback_shift_engine<uint32_t, 32, K, Q, S>`.

| Field | Type | Meaning |
|---|---|---|
| `value` | `u32` | The register's current state. |

### `const Lfsr::W`
[src/tools/krita/random.rs:19](../../../../src/tools/krita/random.rs#L19)

```rust
const W: u32 = 32;
```

Word size in bits (32).

### `struct Taus88`
[src/tools/krita/random.rs:41](../../../../src/tools/krita/random.rs#L41)

boost's `taus88`: three Tausworthe components, xor-combined.

| Field | Type | Meaning |
|---|---|---|
| `a` | `Lfsr<31, 13, 12>` | First Tausworthe component. |
| `b` | `Lfsr<29, 2, 4>` | Second component. |
| `c` | `Lfsr<28, 3, 17>` | Third component. |

### `struct RandomSource`
[src/tools/krita/random.rs:66](../../../../src/tools/krita/random.rs#L66)

`KisRandomSource`.

| Field | Type | Meaning |
|---|---|---|
| `rng` | `Taus88` | The underlying taus88 generator. |

## `impl<const K: u32, const Q: u32, const S: u32> Lfsr<K, Q, S>`

### `Lfsr::seeded`
[src/tools/krita/random.rs:21](../../../../src/tools/krita/random.rs#L21)

```rust
fn seeded(s0: u32) -> Self
```

A register seeded with `s0` (adjusted the way boost does so it's never stuck at zero).

### `Lfsr::next`
[src/tools/krita/random.rs:30](../../../../src/tools/krita/random.rs#L30)

```rust
fn next(&mut self) -> u32
```

Advances the register one step (shift, XOR, mask) and returns the new state.

## `impl Taus88`

### `Taus88::new`
[src/tools/krita/random.rs:49](../../../../src/tools/krita/random.rs#L49)

```rust
pub fn new(seed: u32) -> Self
```

`taus88(v)`: every component seeded with the same value.

### `Taus88::next_u32`
[src/tools/krita/random.rs:58](../../../../src/tools/krita/random.rs#L58)

```rust
pub fn next_u32(&mut self) -> u32
```

`operator()`: `xor_combine` evaluates its first engine first.

## `impl RandomSource`

### `RandomSource::new`
[src/tools/krita/random.rs:71](../../../../src/tools/krita/random.rs#L71)

```rust
pub fn new(seed: u32) -> Self
```

A source seeded with `seed`.

### `RandomSource::for_stroke`
[src/tools/krita/random.rs:77](../../../../src/tools/krita/random.rs#L77)

```rust
pub fn for_stroke() -> Self
```

A stroke's source, seeded the way Krita seeds one (`qrand()`): from whatever is at hand, different every stroke.

### `RandomSource::generate`
[src/tools/krita/random.rs:92](../../../../src/tools/krita/random.rs#L92)

```rust
pub fn generate(&mut self) -> u32
```

`generate()`.

### `RandomSource::generate_range`
[src/tools/krita/random.rs:97](../../../../src/tools/krita/random.rs#L97)

```rust
pub fn generate_range(&mut self, min: i32, max: i32) -> i32
```

`generate(min, max)`: `boost::uniform_smallint`, a plain modulo.

### `RandomSource::generate_normalized`
[src/tools/krita/random.rs:108](../../../../src/tools/krita/random.rs#L108)

```rust
pub fn generate_normalized(&mut self) -> f64
```

`generateNormalized()`: 0..=1.

## Tests

### `mod tests`

Run with `cargo test tools::krita::random::tests` · 2 tests

| Test | What it checks |
|---|---|
| [`taus88_steps_like_boost`](../../../../src/tools/krita/random.rs#L121) | The first outputs of boost's `taus88` for seed 1, worked through by hand from the three shift registers' definitions: each component is lifted to its floor (2, 8, 16) by the seeding rule, then stepped. |
| [`a_seed_repeats_and_ranges_hold`](../../../../src/tools/krita/random.rs#L139) | A seed repeats and ranges hold. |
