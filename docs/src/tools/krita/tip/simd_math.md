# `src/tools/krita/tip/simd_math.rs` — Krita's exact floating-point maths

> **Source:** [src/tools/krita/tip/simd_math.rs](../../../../../src/tools/krita/tip/simd_math.rs) · **Module:** `crate::tools::krita::tip::simd_math` · **Size:** 404 lines · Line links as of commit `36405ad`

## What this file is for

Krita computes its auto tips with fast vectorised maths libraries — **xsimd** for `exp`, **Vc** for `erf`, boost
for double-precision `erf`, and a table-based fast `atan2`. Those don't give quite the same results as Rust's
standard `f32::exp` or `atan2` (they use different polynomial approximations), and an 8-bit mask is sensitive
enough that the difference shows. So this file reproduces each routine **operation for operation**.

**SIMD for a JS developer.** "Single Instruction, Multiple Data": the CPU has registers holding 4 or 8 numbers and
instructions that add/multiply all of them at once — like `array.map` done in one CPU step. Krita processes 8
mask pixels per instruction (AVX2). This port computes one "lane" at a time but with exactly the same sequence
of operations, so each lane's rounding matches.

```text
scalar:  a0+b0, a1+b1, a2+b2, … (8 instructions)
SIMD:    [a0..a7] + [b0..b7]    (1 instruction)
```

## How it connects

- **Used by:** [tools/krita/tip/auto.rs](auto.md) (gauss profile `erf`, `exp`, spike/rotation angles).

## Rust concepts in this file

**Bit-level float tricks.** `f32::from_bits` / `to_bits` reinterpret a float's 32 bits as an integer and back —
used to build `2^n` quickly in `exp`. JS equivalent: a `Float32Array` and `Uint32Array` sharing one buffer.

**`OnceLock` lookup table.** `atan_table` is built once and shared. More:
[Constants and statics](../../../../rust-primer.md#constants-and-statics)

## Free functions

### `xsimd_exp`
[src/tools/krita/tip/simd_math.rs:25](../../../../../src/tools/krita/tip/simd_math.rs#L25)

```rust
pub fn xsimd_exp(a: f32) -> f32
```

xsimd `exp(batch<float, fma3<avx2>>)`, one lane.

### `vc_erf`
[src/tools/krita/tip/simd_math.rs:61](../../../../../src/tools/krita/tip/simd_math.rs#L61)

```rust
pub fn vc_erf(x: f32) -> f32
```

`VcExtraMath::erf`, one lane: precise to about 1e-5, which is plenty for an 8-bit mask.

### `poly`
[src/tools/krita/tip/simd_math.rs:84](../../../../../src/tools/krita/tip/simd_math.rs#L84)

```rust
fn poly(a: &[f64], x: f64) -> f64
```

boost's `evaluate_polynomial`, method 3 (two interleaved Horner chains).

### `exp_minus_z_squared`
[src/tools/krita/tip/simd_math.rs:123](../../../../../src/tools/krita/tip/simd_math.rs#L123)

```rust
fn exp_minus_z_squared(z: f64) -> f64
```

`exp(-z²)` with the error of squaring `z` folded back in, as boost does above 1.5.

### `frexp`
[src/tools/krita/tip/simd_math.rs:134](../../../../../src/tools/krita/tip/simd_math.rs#L134)

```rust
fn frexp(x: f64) -> (f64, i32)
```

`frexp` for finite, non-zero `x`: `x = m · 2^e` with `0.5 <= |m| < 1`.

### `erf_imp`
[src/tools/krita/tip/simd_math.rs:146](../../../../../src/tools/krita/tip/simd_math.rs#L146)

```rust
fn erf_imp(z: f64, invert: bool) -> f64
```

boost's `erf_imp` for 53-bit precision, `invert == false`.

### `boost_erf`
[src/tools/krita/tip/simd_math.rs:283](../../../../../src/tools/krita/tip/simd_math.rs#L283)

```rust
pub fn boost_erf(z: f64) -> f64
```

`boost::math::erf`, double precision.

### `atan_table`
[src/tools/krita/tip/simd_math.rs:288](../../../../../src/tools/krita/tip/simd_math.rs#L288)

```rust
fn atan_table() -> &'static [f64]
```

`KisATanTable`: atan sampled across 0..=1 for the fast arctangent.

### `calc_angle`
[src/tools/krita/tip/simd_math.rs:305](../../../../../src/tools/krita/tip/simd_math.rs#L305)

```rust
fn calc_angle(x: f64, y: f64) -> f64
```

Krita's table-based arctangent for one octant, used by `fast_atan2`.

### `fast_atan2`
[src/tools/krita/tip/simd_math.rs:318](../../../../../src/tools/krita/tip/simd_math.rs#L318)

```rust
pub fn fast_atan2(y: f64, x: f64) -> f64
```

`KisFastMath::atan2`.

## Tests

### `mod tests`

Run with `cargo test tools::krita::tip::simd_math::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`xsimd_exp_is_close_to_exp_and_exact_at_zero`](../../../../../src/tools/krita/tip/simd_math.rs#L364) | Xsimd exp is close to exp and exact at zero. |
| [`the_erfs_agree_with_each_other`](../../../../../src/tools/krita/tip/simd_math.rs#L377) | The erfs agree with each other. |
| [`fast_atan2_stays_within_its_error_bound`](../../../../../src/tools/krita/tip/simd_math.rs#L393) | Fast atan2 stays within its error bound. |
