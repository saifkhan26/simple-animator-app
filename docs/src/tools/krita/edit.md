# `src/tools/krita/edit.rs` — editing a Krita preset as a list of overrides

> **Source:** [src/tools/krita/edit.rs](../../../../src/tools/krita/edit.rs) · **Module:** `crate::tools::krita::edit` · **Size:** 152 lines · Line links as of commit `1fa5845`

## What this file is for

The app has a brush editor for Krita presets. It never rewrites the preset
file when you move a slider. Instead an **edit is a list of
`(param, value)` overrides** — pairs like `("FlowValue", "0.5")` — kept in
the brush settings and laid over the preset's own parameters when the brush
is built (`Preset::with_overrides` in [kpp.rs](kpp.md), called from
[brush_library/mod.rs](../../brush_library/mod.md)).

This file holds the small helpers the editor uses to produce those overrides:

- [`set`](#set) / [`set_all`](#set_all) add an override, but
  **drop it if it only repeats what the preset already says**. So an
  untouched preset has an empty list, and the editor's *Reset* button is just
  "clear the list".
- [`num`](#num) prints a number the way Krita writes one, so a value
  you did not really change compares equal to the preset's text.
- [`attr`](#attr) / [`set_attr`](#set_attr) read and change one XML
  attribute inside the preset's `brush_definition` (the tip's description,
  e.g. `<Brush type="auto_brush" spacing="0.1"> <MaskGenerator diameter="60" …/> </Brush>`).
- [`auto_tip_definition`](#auto_tip_definition) /
  [`bitmap_tip_definition`](#bitmap_tip_definition) write a fresh `brush_definition`
  when the editor switches tip kind.

A JS analogy: instead of mutating a config object, you keep a "diff" object
and drop keys whose value equals the original — like a form library that only
tracks *dirty* fields.

## How it connects

- **Used by:**
  - [ui/krita_editor.rs](../../ui/krita_editor.md) — every control in the
    Krita brush editor: `set`, `set_all`, `num`, `attr`, `set_attr`,
    `auto_tip_definition`, `bitmap_tip_definition`.
  - [brush_library/mod.rs](../../brush_library/mod.md) — `attr` to read the
    tip `filename` out of a `brush_definition`.
- **Uses:** [kpp.rs](kpp.md) — `Preset` and its `string(key)` getter, to
  compare an override against the preset's own value.

## Rust concepts in this file

**`&mut Vec<…>` — a function that edits the caller's list.** `set` takes
`overrides: &mut Vec<(String, String)>`: a *mutable borrow*. The function
changes the caller's vector in place; nothing is returned.
```rust
pub fn set(base: &Preset, overrides: &mut Vec<(String, String)>, key: &str, value: String) {
    overrides.retain(|(k, _)| k != key);   // like arr = arr.filter(...), but in place
    ...
}
```
```js
function set(base, overrides, key, value) {
  overrides.splice(0, overrides.length, ...overrides.filter(([k]) => k !== key));
}
```
More: [Ownership and borrowing](../../../rust-primer.md#ownership-and-borrowing)

**`Option` comparisons with `as_deref`.** `base.string(key)` returns
`Option<String>`. `.as_deref()` turns it into `Option<&str>` so it can be
compared to `Some(value.as_str())` without copying.
```rust
if base.string(key).as_deref() != Some(value.as_str()) { /* keep the override */ }
```
```js
if (base.string(key) !== value) { /* null !== "x" is true too */ }
```
More: [Option and Result](../../../rust-primer.md#option-and-result)

**Byte-index string slicing.** `&xml[s..e]` is a substring by **byte**
positions (like `xml.slice(s, e)`), and `str::find` returns byte positions,
so the two fit together. Slicing in the middle of a multi-byte UTF-8
character would panic, but every index here comes from `find` on ASCII
patterns, so it never does.
More: [Strings and slices](../../../rust-primer.md#strings-and-slices)

**`while let` and `let … else`.** `while let Some(i) = xml[from..].find(&open)`
loops as long as `find` keeps finding something.
`let Some((s, e)) = start_tag(...) else { return xml.to_owned() };` unpacks
the `Option` or bails out early — like `const r = f(); if (!r) return xml;`.
More: [Enums and match](../../../rust-primer.md#enums-and-match)

**`format!` with inline names.** `format!("<{element}")` builds a `String`,
like the JS template literal `` `<${element}` ``.
More: [Macros](../../../rust-primer.md#macros)

## Free functions

### `set`
[src/tools/krita/edit.rs:11](../../../../src/tools/krita/edit.rs#L11)

```rust
pub fn set(base: &Preset, overrides: &mut Vec<(String, String)>, key: &str, value: String)
```

**What it does.** Records "parameter `key` should be `value`" in the override
list — or removes the override, if the preset itself already has exactly that
value.

**Parameters.**
- `base` — the preset as loaded (without overrides).
- `overrides` — the edit list, changed in place.
- `key` — the preset parameter name, e.g. `"FlowValue"` or `"brush_definition"`.
- `value` — the new value as text, the way the preset stores it.

**How it works.**
1. Remove any existing override for `key` (so a key appears at most once).
2. If `base.string(key)` is not exactly `value`, push `(key, value)` on the end.

**Example.** Suppose the preset says `FlowValue = "1"`:
```rust
let mut o = Vec::new();
set(&base, &mut o, "FlowValue", "0.5".into()); // o == [("FlowValue", "0.5")]
set(&base, &mut o, "FlowValue", "1".into());   // back to the preset's value: o == []
```

**Used by:** [`set_all`](#set_all); [ui/krita_editor.rs](../../ui/krita_editor.md)
(painting mode, flow, tip definition, required tip file, texture settings).

### `set_all`
[src/tools/krita/edit.rs:19](../../../../src/tools/krita/edit.rs#L19)

```rust
pub fn set_all(base: &Preset, overrides: &mut Vec<(String, String)>, pairs: Vec<(String, String)>)
```

**What it does.** Calls [`set`](#set) for each `(key, value)` pair, in order.

**Used by:** [ui/krita_editor.rs](../../ui/krita_editor.md) — writing all the
keys of one curve option (its sensors, curve and strength) at once.

### `num`
[src/tools/krita/edit.rs:27](../../../../src/tools/krita/edit.rs#L27)

```rust
pub fn num(v: f64) -> String
```

**What it does.** Turns a number into text the way Krita writes numbers into
a preset (the doc comment names Qt's `QString::number`): the shortest form,
with no trailing zeros and no trailing dot.

**How it works.**
1. Replace negative zero with zero (`-0.0 == 0.0` is true, so `-0.0` becomes `0.0`; this avoids printing `"-0"`).
2. Format with Rust's default float display (`format!("{v}")`).
3. If the text has a `.`, strip trailing `0`s and then a trailing `.`.

**Example.**

| Input | Output |
|---|---|
| `1.0` | `"1"` |
| `0.25` | `"0.25"` |
| `156.25` | `"156.25"` |
| `-0.0` | `"0"` |

**Used by:** [`auto_tip_definition`](#auto_tip_definition); [ui/krita_editor.rs](../../ui/krita_editor.md)
(flow, ratio, fade, density, randomness, angle, spacing, texture scale).

### `start_tag`
[src/tools/krita/edit.rs:38](../../../../src/tools/krita/edit.rs#L38)

```rust
fn start_tag(xml: &str, element: &str) -> Option<(usize, usize)>
```

**What it does.** Finds the first start tag of `<element …>` in an XML string
and returns its byte range: from the `<` up to (not including) the closing
`>`. For a self-closing tag, the range therefore ends with the `/`.

**How it works.**
1. Search for `"<" + element` from position `from`.
2. Check the next character is whitespace, `>` or `/`, so that looking for
   `Brush` does not match `<BrushTip`. If not, keep searching after it.
3. On a match, find the next `>` and return `(start, position of '>')`.

**Returns.** `None` if there is no such tag (or the tag is cut off at the end
of the text).

**Used by:** [`attr`](#attr), [`set_attr`](#set_attr).

### `attr`
[src/tools/krita/edit.rs:54](../../../../src/tools/krita/edit.rs#L54)

```rust
pub fn attr(xml: &str, element: &str, attr: &str) -> Option<String>
```

**What it does.** Reads one attribute of the first `<element>` in `xml`, e.g.
`attr(def, "MaskGenerator", "id")` → `Some("gauss")`.

**How it works.** Finds the start tag, then looks inside it for
`␣attr="` (with a leading space, so `id` does not match `xid`), and returns
the text up to the next `"`. The value is returned as written: XML escapes
such as `&amp;` are not decoded.

**Returns.** `None` if the element or attribute is missing.

**Used by:** [ui/krita_editor.rs](../../ui/krita_editor.md) (reading every tip
setting), [brush_library/mod.rs](../../brush_library/mod.md) (the tip
`filename`).

### `set_attr`
[src/tools/krita/edit.rs:66](../../../../src/tools/krita/edit.rs#L66)

```rust
pub fn set_attr(xml: &str, element: &str, attr: &str, value: &str) -> String
```

**What it does.** Returns a copy of `xml` in which attribute `attr` of the
first `<element>` is set to `value` — replaced in place if present, added at
the end of the tag if not. Everything else in the text is left byte for byte.
The doc comment explains why such a simple editor is enough: Krita's tip
definitions are only one or two flat elements.

**How it works.**
1. Find the tag with [`start_tag`](#start_tag). If there is none, return `xml` unchanged.
2. Escape `&`, `"` and `<` in `value` (`&amp;`, `&quot;`, `&lt;`).
3. If the tag already has `␣attr="…"`, splice the new value between the quotes.
4. Otherwise append `␣attr="value"` — before the `/` if the tag is self-closing.
5. Glue the text before the tag, the new tag and the text after it back together.

**Example.** (From the tests.)
```text
<MaskGenerator diameter="60" ratio="1" id="gauss"/>
set_attr(…, "MaskGenerator", "spikes", "5")
→ <MaskGenerator diameter="60" ratio="1" id="gauss" spikes="5"/>
```

**Used by:** [ui/krita_editor.rs](../../ui/krita_editor.md) — every tip
slider (ratio, fade, density, randomness, angle, spacing…).

### `auto_tip_definition`
[src/tools/krita/edit.rs:90](../../../../src/tools/krita/edit.rs#L90)

```rust
pub fn auto_tip_definition(diameter: f64, spacing: &str, auto_spacing: &str, coeff: &str) -> String
```

**What it does.** Writes a brand-new `brush_definition` for a **generated
round tip** (Krita's `auto_brush`: a circle mask computed from numbers rather
than an image — see [tip/auto.rs](tip/auto.md)) of the given `diameter`,
with Krita's default settings for everything else (ratio 1, fades 0.5, two
spikes, antialiased edges, density 1, no randomness, angle 0). The spacing
settings of the previous tip are carried over through `spacing`,
`auto_spacing` (`useAutoSpacing`) and `coeff` (`autoSpacingCoeff`).

**Used by:** [ui/krita_editor.rs](../../ui/krita_editor.md) when the user
switches the tip to a generated one.

### `bitmap_tip_definition`
[src/tools/krita/edit.rs:98](../../../../src/tools/krita/edit.rs#L98)

```rust
pub fn bitmap_tip_definition(file: &str, spacing: &str, auto_spacing: &str, coeff: &str) -> String
```

**What it does.** Writes a new `brush_definition` for a **bitmap tip** read
from `file`, at scale 1 and angle 0, keeping the old spacing settings. The
`type` is `png_brush` when the file name ends in `.png` (any case) and
`gbr_brush` otherwise (e.g. `.gih` / `.gbr`).

**Used by:** [ui/krita_editor.rs](../../ui/krita_editor.md) when the user
picks a bitmap tip.

## Tests

Run with `cargo test tools::krita::edit`.

| Test | What it checks |
|---|---|
| [`attributes_read_and_write_in_place`](../../../../src/tools/krita/edit.rs#L112) | `attr` reads attributes of both elements; `set_attr` replaces an existing attribute without touching the other element, appends a missing one before `/>` on a self-closing tag and before `>` on an open tag, and the result still parses as XML (`roxmltree`). |
| [`numbers_print_like_qt`](../../../../src/tools/krita/edit.rs#L127) | `num` prints `1`, `0.25`, `156.25`, and `-0.0` as `0`. |
| [`overrides_that_restate_the_preset_are_dropped`](../../../../src/tools/krita/edit.rs#L135) | On the embedded Pencil-5 preset, setting `FlowValue` to `0.5` adds one override; setting it back to the preset's own value empties the list. |
| [`new_tip_definitions_parse`](../../../../src/tools/krita/edit.rs#L145) | Both generated definitions are valid XML, and a `.gih` file gets the `gbr_brush` type. |

**Test constant:** `DEF` — a two-element sample `brush_definition` (`<Brush …> <MaskGenerator …/> </Brush>`).

### `mod tests`

Run with `cargo test tools::krita::edit::tests` · 4 tests

| Test | What it checks |
|---|---|
| [`attributes_read_and_write_in_place`](../../../../src/tools/krita/edit.rs#L112) | Attributes read and write in place. |
| [`numbers_print_like_qt`](../../../../src/tools/krita/edit.rs#L127) | Numbers print like qt. |
| [`overrides_that_restate_the_preset_are_dropped`](../../../../src/tools/krita/edit.rs#L135) | Overrides that restate the preset are dropped. |
| [`new_tip_definitions_parse`](../../../../src/tools/krita/edit.rs#L145) | New tip definitions parse. |

**Test-only types and constants:** [`DEF`](../../../../src/tools/krita/edit.rs#L109)
