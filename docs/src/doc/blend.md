# `src/doc/blend.rs` — layer blend modes

> **Source:** [src/doc/blend.rs](../../../src/doc/blend.rs) · **Module:** `crate::doc::blend` · **Size:** 219 lines · Line links as of commit `84b26e8`

## What this file is for

**Layer blend modes**: how a layer's colours mix with what is under it — Normal, Multiply (shadows), Screen and
Add (light, glows) and Overlay (tone). This file is the one place that defines them: the enum stored on each
[`Layer`](layer.md), each mode's name and hint for the Layers panel, its Krita `compositeop` name, and its formula.

Every mode is *separable*: red, green and blue mix one at a time through `BlendMode::mix(cb, cs)` — `cb` the
backdrop's channel, `cs` the layer's, both unmultiplied 0–1 — and the result is composited the way the W3C
Compositing spec, Krita and Photoshop do it:

```text
αo = αs + αb·(1 − αs)
co = (αs·(1 − αb)·cs + αb·(1 − αs)·cb + αs·αb·B(cb, cs)) / αo
```

So a layer over nothing shows as itself, and its transparent pixels leave the backdrop alone. `blend_px` is that
formula for one RGBA8 pixel, used by the CPU compositor ([io/composite.rs](../io/composite.md)) for export, merged
group images and merge down. The canvas runs the same formula on the GPU in `ui/canvas_blend.wgsl`, which keeps a
copy of `mix` in step with this one ([ui/canvas_gpu.rs](../ui/canvas_gpu.md)).

- **Stored as:** the variant's index in the `.anim` (postcard). New modes go at the end, never in between; a test
  pins the numbers.
- **Groups:** always Normal. A blend mode inside a group mixes with the group's own layers only.
- **To add a mode:** a variant at the end, a row in each `match`, a case in the shader's `mix_channel`, an icon in
  `ui::shell::blend_icon`, and the tests' expected values.

## How it connects

- **Used by:** [doc/layer.rs](layer.md) (`Layer::blend`), [io/composite.rs](../io/composite.md),
  [ui/canvas_gpu.rs](../ui/canvas_gpu.md), [io/kra/mod.rs](../io/kra/mod.md), [krita_link.rs](../krita_link.md),
  [ui/shell.rs](../ui/shell.md) (the Layers panel's menu and row marks), [app.rs](../app.md) (`set_selected_blend`).

## Rust concepts in this file

**`#[derive(Default)]` with `#[default]`.** Marks which enum variant `BlendMode::default()` returns — Normal — so a
layer read from an older file, where the field is missing, gets it.
More: [Traits and derive](../../rust-primer.md#traits-and-derive)

## Types

### `enum BlendMode`
[src/doc/blend.rs:31](../../../src/doc/blend.rs#L31)

How a layer mixes with what is under it. Groups are always Normal.

The `.anim` file stores the variant's index, so new modes go at the end, never in between, and none is ever removed.

| Variant | Meaning |
|---|---|
| `Normal` | Covers what is under it. |
| `Multiply` | Darkens: `cb × cs`. White vanishes. For shadows. |
| `Screen` | Lightens: `cb + cs − cb × cs`. Black vanishes. For light. |
| `Add` | Lightens harder: `cb + cs`, clamped to white. For glows. |
| `Overlay` | Contrast: multiply where the backdrop is dark, screen where it is light; mid grey vanishes. |

### `const BlendMode::ALL`
[src/doc/blend.rs:42](../../../src/doc/blend.rs#L42)

```rust
pub const ALL: [BlendMode; 5] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Add,
    BlendMode::Overlay,
// …
```

Every mode, in the order the menu lists them.

## `impl BlendMode`

### `BlendMode::label`
[src/doc/blend.rs:50](../../../src/doc/blend.rs#L50)

```rust
pub fn label(self) -> &'static str
```

The mode's name, as the Layers panel's menu shows it.

### `BlendMode::hint`
[src/doc/blend.rs:61](../../../src/doc/blend.rs#L61)

```rust
pub fn hint(self) -> &'static str
```

What the mode is for, in a sentence: the menu's hover text.

### `BlendMode::krita_id`
[src/doc/blend.rs:72](../../../src/doc/blend.rs#L72)

```rust
pub fn krita_id(self) -> &'static str
```

Krita's name for the mode, its `compositeop` in a `.kra`.

### `BlendMode::from_krita`
[src/doc/blend.rs:84](../../../src/doc/blend.rs#L84)

```rust
pub fn from_krita(id: &str) -> Option<BlendMode>
```

The mode Krita's `compositeop` names, or `None` for one this app doesn't have. Krita's Linear Dodge is the same sum as its Addition.

### `BlendMode::shader_index`
[src/doc/blend.rs:92](../../../src/doc/blend.rs#L92)

```rust
pub fn shader_index(self) -> u32
```

The number the canvas shader knows the mode by.

### `BlendMode::mix`
[src/doc/blend.rs:98](../../../src/doc/blend.rs#L98)

```rust
pub fn mix(self, cb: f32, cs: f32) -> f32
```

`B(cb, cs)`: one channel of the layer, `cs`, mixed with the same channel under it, `cb`, both unmultiplied in 0..=1.

## Free functions

### `blend_px`
[src/doc/blend.rs:121](../../../src/doc/blend.rs#L121)

```rust
pub fn blend_px(mode: BlendMode, d: &mut [u8], s: &[u8], sa: f32)
```

Lay one pixel `s` over one pixel `d` in `mode`, with `s`'s coverage taken as `sa` (its alpha times the layer's opacity, 0..=1). Both unmultiplied RGBA8, as canvases hold them; `d` is written. Rounds as the plain "over" in `io::composite` does.

## Tests

### `mod tests`

Run with `cargo test doc::blend::tests` · 6 tests

| Test | What it checks |
|---|---|
| [`the_file_s_numbers_never_move`](../../../src/doc/blend.rs#L152) | The file s numbers never move. |
| [`each_mode_mixes_as_its_formula_says`](../../../src/doc/blend.rs#L167) | Each mode mixes as its formula says. |
| [`over_nothing_a_layer_shows_as_itself_and_transparent_leaves_the_backdrop`](../../../src/doc/blend.rs#L183) | Over nothing a layer shows as itself and transparent leaves the backdrop. |
| [`opaque_over_opaque_is_the_mix`](../../../src/doc/blend.rs#L191) | Opaque over opaque is the mix. |
| [`opacity_fades_toward_the_backdrop`](../../../src/doc/blend.rs#L202) | Opacity fades toward the backdrop. |
| [`krita_names_round_trip`](../../../src/doc/blend.rs#L211) | Krita names round trip. |

**Test helpers** (used only by the tests above):

- [`over`](../../../src/doc/blend.rs#L145) — Over.
