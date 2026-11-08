# `src/tools/krita/options.rs` — the list of curve options a Krita brush reads

> **Source:** [src/tools/krita/options.rs](../../../../src/tools/krita/options.rs) · **Module:** `crate::tools::krita::options` · **Size:** 32 lines · Line links as of commit `36405ad`

## What this file is for

A Krita brush preset (a `.kpp` file) stores its settings as a flat list of
named parameters. Many of those settings are **curve options**: "Size",
"Opacity", "Rotation" and so on, each of which can be driven by pen
*sensors* (pressure, tilt, speed…) through a curve, and scaled by a
*strength* slider. (A *sensor* is one pen reading fed into a curve; see
[sensors.rs](sensors.md) and the [glossary](../../../glossary.md).)

This tiny file is a **table of declarations** for those options, copied from
Krita's C++ headers (`KisStandardOptionData.h`, `KisSizeOptionData.cpp`,
`KisScatterOptionData.cpp`). For each option it records three things:

1. the **id** — the prefix of that option's keys inside the preset
   (`SizeSensor`, `SizeValue`, `PressureSize`, …);
2. whether the option is **checkable** — can the user switch it off? An
   option that is not checkable (Opacity, Flow) always applies;
3. the **range** of its strength slider (`min`..`max`).

The range matters more than it looks: when a preset leaves the strength out,
the reader in [kpp.rs](kpp.md) uses the **top of the range** as the default
(`self.double(&format!("{id}Value"), spec.max)`). So Scatter, whose range is
0..5, defaults to 5 — a dab thrown up to five diameters away — not to 1.

Think of it as a small JS config object: `const SIZE = { id: "Size", checkable: true, min: 0, max: 1 }`.

## How it connects

- **Used by:**
  - [kpp.rs](kpp.md) — `Preset::curve_option`, `Preset::option_data`,
    `OptionData::curve_option` and `OptionData::write` take an `&OptionSpec`
    to know which keys to read or write and what the default strength is.
  - [mod.rs](mod.md) — `PixelBrush::from_preset` reads all eight options
    (`p.curve_option(&options::SIZE)?` …).
  - [ui/krita_editor.rs](../../ui/krita_editor.md) — the brush editor lists
    the options, shows a checkbox only for checkable ones, and sizes each
    strength slider from `spec.min..=spec.max`.
- **Uses:** nothing.

## Rust concepts in this file

**`const fn` — a function the compiler can run at compile time.** `spec` is
a `const fn`, so it can be called to build `pub const` values. The values are
baked into the program; nothing runs at start-up.
```rust
const fn spec(id: &'static str, checkable: bool, min: f64, max: f64) -> OptionSpec {
    OptionSpec { id, checkable, min, max }
}
pub const SIZE: OptionSpec = spec("Size", true, 0.0, 1.0);
```
```js
const spec = (id, checkable, min, max) => Object.freeze({ id, checkable, min, max });
export const SIZE = spec("Size", true, 0, 1);
```
More: [Constants and statics](../../../rust-primer.md#constants-and-statics)

**`&'static str` — a string that lives for the whole program.** A string
literal like `"Size"` is stored inside the executable, so a reference to it is
valid forever; `'static` is the *lifetime* that says so. That is why a
`const` struct can hold it without owning a `String`.
More: [Lifetimes](../../../rust-primer.md#lifetimes), [Strings and slices](../../../rust-primer.md#strings-and-slices)

**`#[derive(Clone, Copy, Debug, PartialEq)]`.** Asks the compiler to write
the boilerplate: `Copy` means an `OptionSpec` is copied (like a JS number)
instead of moved when assigned; `PartialEq` gives `==`; `Debug` lets
`{:?}` print it.
More: [Traits and derive](../../../rust-primer.md#traits-and-derive)

**Field init shorthand.** `OptionSpec { id, checkable, min, max }` is the
same as `{ id: id, checkable: checkable, … }`, exactly like JS object
shorthand.
More: [Structs](../../../rust-primer.md#structs)

### The eight option constants
All are `pub const …: OptionSpec`, built with [`spec`](#spec).

| Constant | Line | `id` | Checkable | Range | Notes |
|---|---|---|---|---|---|
| `SIZE` | [23](../../../../src/tools/krita/options.rs#L23) | `"Size"` | yes | 0..1 | Dab size multiplier. |
| `RATIO` | [24](../../../../src/tools/krita/options.rs#L24) | `"Ratio"` | yes | 0..1 | Height-to-width squash of the dab. |
| `ROTATION` | [25](../../../../src/tools/krita/options.rs#L25) | `"Rotation"` | yes | 0..1 | Dab rotation. |
| `OPACITY` | [26](../../../../src/tools/krita/options.rs#L26) | `"Opacity"` | **no** | 0..1 | Always on. |
| `FLOW` | [27](../../../../src/tools/krita/options.rs#L27) | `"Flow"` | **no** | 0..1 | Always on; used by wash mode. |
| `SOFTNESS` | [28](../../../../src/tools/krita/options.rs#L28) | `"Softness"` | yes | **0.1**..1 | Fades a generated (auto) tip's edge. |
| `SCATTER` | [30](../../../../src/tools/krita/options.rs#L30) | `"Scatter"` | yes | 0..**5** | Strength is a multiple of the dab's diameter, up to five. |
| `TEXTURE_STRENGTH` | [31](../../../../src/tools/krita/options.rs#L31) | `"Texture/Strength/"` | yes | 0..1 | How strongly the paper texture bites. Its id is a path-like prefix, so its keys look like `Texture/Strength/Value`. |

## Types

### `struct OptionSpec`
[src/tools/krita/options.rs:9](../../../../src/tools/krita/options.rs#L9)

The declaration of one curve option: how to find it in a preset and how its
strength slider is bounded.

| Field | Type | Meaning |
|---|---|---|
| `id` | `&'static str` | The key prefix in the preset: the option's keys are `<id>Sensor`, `<id>Value`, `Pressure<id>`, … |
| `checkable` | `bool` | `true`: the option is off unless the preset's `Pressure<id>` key says `true`. `false`: the option always applies. |
| `min` | `f64` | Lowest value of the strength slider. |
| `max` | `f64` | Highest value of the strength slider; also the strength used when the preset does not give one. |

### `const SIZE`
[src/tools/krita/options.rs:23](../../../../src/tools/krita/options.rs#L23)

```rust
pub const SIZE: OptionSpec = spec("Size", true, 0.0, 1.0);
```

The **Size** option (`"Size"`): a dab-size multiplier driven by sensors, 0–1. See the table above.

### `const RATIO`
[src/tools/krita/options.rs:24](../../../../src/tools/krita/options.rs#L24)

```rust
pub const RATIO: OptionSpec = spec("Ratio", true, 0.0, 1.0);
```

The **Ratio** option (`"Ratio"`): squashes the dab's height against its width, 0–1.

### `const ROTATION`
[src/tools/krita/options.rs:25](../../../../src/tools/krita/options.rs#L25)

```rust
pub const ROTATION: OptionSpec = spec("Rotation", true, 0.0, 1.0);
```

The **Rotation** option (`"Rotation"`): turns the dab, from sensors such as drawing angle or tilt direction.

### `const OPACITY`
[src/tools/krita/options.rs:26](../../../../src/tools/krita/options.rs#L26)

```rust
pub const OPACITY: OptionSpec = spec("Opacity", false, 0.0, 1.0);
```

The **Opacity** option (`"Opacity"`): each dab's opacity, 0–1.

### `const FLOW`
[src/tools/krita/options.rs:27](../../../../src/tools/krita/options.rs#L27)

```rust
pub const FLOW: OptionSpec = spec("Flow", false, 0.0, 1.0);
```

The **Flow** option (`"Flow"`): how much a wash-mode dab adds, 0–1.

### `const SOFTNESS`
[src/tools/krita/options.rs:28](../../../../src/tools/krita/options.rs#L28)

```rust
pub const SOFTNESS: OptionSpec = spec("Softness", true, 0.1, 1.0);
```

The **Softness** option (`"Softness"`): an auto tip's edge softness, 0–1.

### `const SCATTER`
[src/tools/krita/options.rs:30](../../../../src/tools/krita/options.rs#L30)

```rust
pub const SCATTER: OptionSpec = spec("Scatter", true, 0.0, 5.0);
```

Scatter's strength is a multiple of the dab's diameter, up to five.

### `const TEXTURE_STRENGTH`
[src/tools/krita/options.rs:31](../../../../src/tools/krita/options.rs#L31)

```rust
pub const TEXTURE_STRENGTH: OptionSpec = spec("Texture/Strength/", true, 0.0, 1.0);
```

The texture **Strength** option: how strongly the paper texture shapes each dab, 0–1.

## Free functions

### `spec`
[src/tools/krita/options.rs:19](../../../../src/tools/krita/options.rs#L19)

```rust
const fn spec(id: &'static str, checkable: bool, min: f64, max: f64) -> OptionSpec
```

**What it does.** Builds an `OptionSpec` from its four fields. It exists only
to make the constant table below it short; being a `const fn`, it runs at
compile time.

**Used by:** the eight constants in this file.
