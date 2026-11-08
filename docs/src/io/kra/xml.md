# `src/io/kra/xml.rs` — the XML parts of a .kra

> **Source:** [src/io/kra/xml.rs](../../../../src/io/kra/xml.rs) · **Module:** `crate::io::kra::xml` · **Size:** 203 lines · Line links as of commit `67a7cb9`

## What this file is for

Builds the XML files inside a `.kra` — `maindoc.xml` (image size and layer stack), each layer's
`*.keyframes.xml`, `animation/index.xml` (fps, range, playhead) and `documentinfo.xml` — and parses Krita's
XML back. Writing uses plain `format!` templates (the shapes are fixed and small, so no XML library is
needed for output); reading uses the `roxmltree` crate.

A snippet of what `maindoc` produces:

```xml
<layer name="Ink" uuid="{…}" filename="anim1" nodetype="paintlayer" opacity="255" visible="1" locked="0" …/>
```

## How it connects

- **Used by:** [io/kra/mod.rs](mod.md).
- **Uses:** `roxmltree`.

## Rust concepts in this file

**Enums with borrowed data and lifetimes.** `enum Item<'a> { Layer(LayerXml<'a>), Group(..), Raw(&'a str) }`
— each variant holds references into strings owned elsewhere, so building the stack copies nothing.
More: [Lifetimes](../../../rust-primer.md#lifetimes)

**Multi-line string literals.** A `\` at the end of a line inside `"…"` continues the string without the
newline — used for the XML templates. `format!("…{fps}…")` interpolates variables like a JS template
literal.

**Recursion over a tree.** `write_item` calls itself for a group's children.

## Types

### `enum Item`
[src/io/kra/xml.rs:9](../../../../src/io/kra/xml.rs#L9)

One entry of the layer stack being written: one of our paint layers, one of our groups (with children), or
a Krita layer copied verbatim.

| Variant | Meaning |
|---|---|
| `Layer(LayerXml<'a>)` | A paint layer of ours. |
| `Group(LayerXml<'a>, bool, Vec<Item<'a>>)` | A group of ours: its own `<layer>` attributes, whether it's folded, and what's in it, top first. |
| `Raw(&'a str)` | A `<layer>` element copied verbatim from a Krita file (see `super::Carried`). |

### `struct LayerXml`
[src/io/kra/xml.rs:23](../../../../src/io/kra/xml.rs#L23)

The attributes of one `<layer>` element this app writes.

| Field | Type | Meaning |
|---|---|---|
| `filename` | `&'a str` | Its file name under `layers/` (`anim1`, …). |
| `name` | `&'a str` | Layer name (escaped when written). |
| `uuid` | `&'a str` | Krita uuid. |
| `opacity` | `u8` | 0–255. |
| `visible` | `bool` | Visible flag. |
| `locked` | `bool` | Locked flag. |
| `selected` | `bool` | Whether Krita opens with it selected. |

### `const PROFILE`
[src/io/kra/xml.rs:59](../../../../src/io/kra/xml.rs#L59)

```rust
pub const PROFILE: &str = "sRGB built-in";
```

The image profile's name. Krita registers LittleCMS's sRGB under this name at startup, and the bytes `super::SRGB_ICC` carries are that profile.

### `const KRITA_VERSION`
[src/io/kra/xml.rs:63](../../../../src/io/kra/xml.rs#L63)

```rust
const KRITA_VERSION: &str = "5.0.0";
```

The kritaVersion we claim. Old enough that any Krita 5 reads it without a "newer version" warning, new enough to promise cloned keyframes.

## Free functions

### `parse`
[src/io/kra/xml.rs:35](../../../../src/io/kra/xml.rs#L35)

```rust
pub fn parse(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error>
```

Parses XML as Krita writes it — every file has a `<!DOCTYPE>`, which roxmltree rejects unless
`allow_dtd: true` is set.

### `attr`
[src/io/kra/xml.rs:41](../../../../src/io/kra/xml.rs#L41)

```rust
pub fn attr(s: &str) -> String
```

Escapes text for use inside a double-quoted XML attribute (`&` → `&amp;`, `"` → `&quot;`, `<`, `>`, and
control characters).

### `maindoc`
[src/io/kra/xml.rs:66](../../../../src/io/kra/xml.rs#L66)

```rust
pub fn maindoc(image: &str, width: u32, height: u32, layers: &[Item]) -> String
```

Builds `maindoc.xml`: the `<DOC>`/`<IMAGE>` header with size, colour profile and animation flags, then the
layer stack (top of the stack first, as Krita lists it).

### `write_item`
[src/io/kra/xml.rs:90](../../../../src/io/kra/xml.rs#L90)

```rust
fn write_item(s: &mut String, item: &Item, depth: usize)
```

Writes one stack entry indented `depth` spaces: a `<layer>` for a paint layer, a group `<layer>` with its own
nested `<layers>` (recursing), or a carried Krita element verbatim.

### `keyframes`
[src/io/kra/xml.rs:136](../../../../src/io/kra/xml.rs#L136)

```rust
pub fn keyframes(keys: &[(usize, String)]) -> String
```

A raster layer's keyframes file: one keyframe per `(time, frame file)`. The same file named twice is a
*cloned* keyframe (Krita 5) — the same drawing exposed again.

### `animation_index`
[src/io/kra/xml.rs:152](../../../../src/io/kra/xml.rs#L152)

```rust
pub fn animation_index(fps: f32, last_frame: usize, current: usize) -> String
```

`animation/index.xml`: frame rate (Krita stores an integer, so fps is rounded), the playback range
`0..last_frame`, and the current time.

### `documentinfo`
[src/io/kra/xml.rs:166](../../../../src/io/kra/xml.rs#L166)

```rust
pub fn documentinfo() -> &'static str
```

A minimal `documentinfo.xml` (empty title and author), which Krita expects to exist.

## Tests

### `mod tests`

Run with `cargo test io::kra::xml::tests` · 2 tests

| Test | What it checks |
|---|---|
| [`attr_escapes_quotes_and_markup`](../../../../src/io/kra/xml.rs#L180) | Attr escapes quotes and markup. |
| [`maindoc_parses_and_marks_selection`](../../../../src/io/kra/xml.rs#L185) | Maindoc parses and marks selection. |
