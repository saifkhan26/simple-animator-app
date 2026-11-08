# `src/tools/krita/kpp.rs` — reading Krita brush presets (.kpp)

> **Source:** [src/tools/krita/kpp.rs](../../../../src/tools/krita/kpp.rs) · **Module:** `crate::tools::krita::kpp` · **Size:** 813 lines · Line links as of commit `67a7cb9`

## What this file is for

A Krita brush preset file (`.kpp`) is a **PNG image** — the preset's thumbnail — with the settings hidden in a
PNG **text chunk** named `preset`, holding XML: a flat list of `<param type="…" name="…">value</param>`
entries (Krita's `KisPropertiesConfiguration`). Krita 5 presets may also carry their tip images inside
(`<resources>`, base64).

```xml
<Preset paintopid="paintbrush" name="Pencil-5_Tilted">
  <param type="string" name="PressureSize"><![CDATA[<!DOCTYPE params>…]]></param>
  <param type="internal" name="Texture/Pattern/Enabled">true</param>
  …
</Preset>
```

This file reads a `.kpp` into a `Preset` (params in their original order and type, so it can be written back
identically), and turns a preset's sensor options into `OptionData`/`CurveOption` — exactly as Krita's
`KisKritaSensorPack::read` does, legacy quirks included.

## How it connects

- **Used by:** [tools/krita/mod.rs](mod.md) (`PixelBrush::load`/`from_preset`), [tools/krita/kpp_write.rs](kpp_write.md),
  [tools/krita/edit.rs](edit.md) (overrides), [brush_library/](../../brush_library/mod.md) (import),
  [ui/krita_editor.rs](../../ui/krita_editor.md).
- **Uses:** [tools/krita/sensors.rs](sensors.md), [tools/krita/curve.rs](curve.md), `roxmltree`, `flate2`.

## Rust concepts in this file

**Reading PNG chunks by hand.** A PNG is a signature followed by chunks: 4-byte length, 4-byte type, data, CRC.
`png_text` walks them to find `tEXt`/`zTXt`/`iTXt` with keyword `preset` (decompressing `zTXt` with `inflate`).

**`HashMap` plus an order `Vec`.** `params: HashMap<String, Param>` gives fast lookup; `order: Vec<String>`
remembers the original order for writing back — like a JS `Map`, which keeps insertion order by itself.

## Types

### `const DEFAULT_CURVE`
[src/tools/krita/kpp.rs:20](../../../../src/tools/krita/kpp.rs#L20)

```rust
pub const DEFAULT_CURVE: &str = "0,0;1,1;";
```

Krita's identity curve string `"0,0;1,1;"`.

### `struct Param`
[src/tools/krita/kpp.rs:25](../../../../src/tools/krita/kpp.rs#L25)

One `<param>`: its value, and its declared type (`string`, `bytearray` …), kept so a preset can be written back the way Krita wrote it.

| Field | Type | Meaning |
|---|---|---|
| `kind` | `String` | The declared type (`string`, `internal`, `bytearray`…), kept for writing back. |
| `value` | `Vec<u8>` | The raw value bytes. |

### `struct EmbeddedResource`
[src/tools/krita/kpp.rs:32](../../../../src/tools/krita/kpp.rs#L32)

A file a version-5.0 preset carries inside itself.

| Field | Type | Meaning |
|---|---|---|
| `kind` | `String` | `brushes`, `patterns`, … |
| `name` | `String` | The resource's name. |
| `filename` | `String` | Its file name. |
| `md5sum` | `String` | The MD5 Krita identifies it by. |
| `data` | `Vec<u8>` | The file's bytes. |

### `struct Preset`
[src/tools/krita/kpp.rs:43](../../../../src/tools/krita/kpp.rs#L43)

The flat settings of one preset.

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | The preset's name. |
| `paintop` | `String` | The paint operation id (`paintbrush` for the pixel brush). |
| `version` | `String` | The `version` text chunk; `5.0` for a Krita 5 preset. Only the reader acts on it (embedded resources), so the app never reads it back. |
| `params` | `HashMap<String, Param>` | Every parameter by name. |
| `order` | `Vec<String>` | Parameter names in the order the preset lists them. |
| `resources` | `Vec<EmbeddedResource>` | Files embedded in a Krita 5 preset. |

### `struct Slot`
[src/tools/krita/kpp.rs:166](../../../../src/tools/krita/kpp.rs#L166)

One sensor slot of an option, as `KisSensorData` keeps it.

| Field | Type | Meaning |
|---|---|---|
| `id` | `&'static str` | The sensor's id (`pressure`, `xtilt`, `speed`, …). |
| `active` | `bool` | Whether the sensor is on. |
| `curve` | `String` | Its curve string. |
| `fan_corners_enabled` | `bool` | Drawing-angle sensor: fan dabs round sharp corners. |
| `fan_corners_step` | `i32` | Fan step in degrees. |
| `angle_offset` | `i32` | Drawing-angle sensor's offset. |
| `locked_angle_mode` | `bool` | Drawing-angle sensor's locked mode. |
| `length` | `i32` | Fade/distance/time sensors' length. |
| `periodic` | `bool` | Whether that length repeats. |

### `struct OptionData`
[src/tools/krita/kpp.rs:183](../../../../src/tools/krita/kpp.rs#L183)

One curve option as `KisCurveOptionData` holds it: what a preset says, before it becomes sensors. The editor changes this and writes it back.

| Field | Type | Meaning |
|---|---|---|
| `checked` | `bool` | Whether the option is enabled. |
| `use_curve` | `bool` | Whether its sensors are used. |
| `use_same_curve` | `bool` | All sensors share one curve. |
| `strength` | `f64` | The option's strength (0–1). |
| `curve_mode` | `i32` | How several sensors combine (multiply, add, max, min, difference). |
| `common_curve` | `String` | The shared curve when `use_same_curve`. |
| `slots` | `Vec<Slot>` | Every sensor Krita knows, in its order; the active ones apply. |

### `const SLOTS`
[src/tools/krita/kpp.rs:195](../../../../src/tools/krita/kpp.rs#L195)

```rust
pub const SLOTS: [&str; 16] = [
    "pressure",
    "pressurein",
    "tangentialpressure",
    "drawingangle",
    "xtilt",
// …
```

Every sensor Krita knows, in Krita's order.

## Free functions

### `with_xml`
[src/tools/krita/kpp.rs:59](../../../../src/tools/krita/kpp.rs#L59)

```rust
pub fn with_xml(xml: &str) -> Vec<u8>
```

A `.kpp` holding `xml`: just the text chunk, which is all the reader looks at.

### `png_text`
[src/tools/krita/kpp.rs:71](../../../../src/tools/krita/kpp.rs#L71)

```rust
fn png_text(png: &[u8], key: &str) -> Result<Vec<u8>>
```

The value of a PNG text chunk (`tEXt`, `zTXt` or `iTXt`) by keyword.

### `parse_xml`
[src/tools/krita/kpp.rs:113](../../../../src/tools/krita/kpp.rs#L113)

```rust
fn parse_xml(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error>
```

Krita's XML carries a `<!DOCTYPE>`, which roxmltree refuses unless asked.

### `inflate`
[src/tools/krita/kpp.rs:121](../../../../src/tools/krita/kpp.rs#L121)

```rust
fn inflate(data: &[u8]) -> Result<Vec<u8>>
```

Decompresses zlib data (for `zTXt` chunks).

### `base64`
[src/tools/krita/kpp.rs:130](../../../../src/tools/krita/kpp.rs#L130)

```rust
pub(crate) fn base64(text: &[u8]) -> Vec<u8>
```

`QByteArray::fromBase64`: forgiving of whitespace and missing padding.

### `preset_xml`
[src/tools/krita/kpp.rs:159](../../../../src/tools/krita/kpp.rs#L159)

```rust
pub fn preset_xml(kpp: &[u8]) -> Result<String>
```

The settings XML of a `.kpp`.

## `impl Slot`

### `Slot::new`
[src/tools/krita/kpp.rs:216](../../../../src/tools/krita/kpp.rs#L216)

```rust
pub fn new(id: &'static str) -> Self
```

`KisSensorData(id)` / `reset()`.

### `Slot::read`
[src/tools/krita/kpp.rs:231](../../../../src/tools/krita/kpp.rs#L231)

```rust
fn read(&mut self, e: roxmltree::Node)
```

`KisSensorData::read` and its subclasses' (which reset first).

### `Slot::write`
[src/tools/krita/kpp.rs:265](../../../../src/tools/krita/kpp.rs#L265)

```rust
fn write(&self, tag: &str) -> String
```

`KisSensorData::write` and its subclasses', into `<tag …>…</tag>` with QDom's line breaks folded to spaces, as Krita's presets hold it.

### `Slot::kind`
[src/tools/krita/kpp.rs:284](../../../../src/tools/krita/kpp.rs#L284)

```rust
pub fn kind(&self, option: &str) -> Result<SensorKind>
```

The sensor this slot paints with, or why it can't.

## `impl Preset`

### `Preset::read`
[src/tools/krita/kpp.rs:304](../../../../src/tools/krita/kpp.rs#L304)

```rust
pub fn read(kpp: &[u8]) -> Result<Self>
```

Reads a `.kpp`: extracts the `preset` text chunk, parses the XML, collects each `<param>` (name, type, value —
CDATA or base64), the paint op and name, and any embedded resources.

### `Preset::has`
[src/tools/krita/kpp.rs:359](../../../../src/tools/krita/kpp.rs#L359)

```rust
pub fn has(&self, key: &str) -> bool
```

Whether a parameter exists.

### `Preset::param`
[src/tools/krita/kpp.rs:364](../../../../src/tools/krita/kpp.rs#L364)

```rust
pub fn param(&self, key: &str) -> Option<&Param>
```

A parameter by name.

### `Preset::string`
[src/tools/krita/kpp.rs:368](../../../../src/tools/krita/kpp.rs#L368)

```rust
pub fn string(&self, key: &str) -> Option<String>
```

A parameter as text.

### `Preset::bytes`
[src/tools/krita/kpp.rs:372](../../../../src/tools/krita/kpp.rs#L372)

```rust
pub fn bytes(&self, key: &str) -> Option<&[u8]>
```

A parameter's raw bytes.

### `Preset::bool`
[src/tools/krita/kpp.rs:377](../../../../src/tools/krita/kpp.rs#L377)

```rust
pub fn bool(&self, key: &str, default: bool) -> bool
```

`getBool`: `QVariant(QString).toBool()`.

### `Preset::double`
[src/tools/krita/kpp.rs:387](../../../../src/tools/krita/kpp.rs#L387)

```rust
pub fn double(&self, key: &str, default: f64) -> f64
```

A parameter as a number, or `default`.

### `Preset::int`
[src/tools/krita/kpp.rs:393](../../../../src/tools/krita/kpp.rs#L393)

```rust
pub fn int(&self, key: &str, default: i32) -> i32
```

A parameter as an integer, or `default`.

### `Preset::curve_option`
[src/tools/krita/kpp.rs:403](../../../../src/tools/krita/kpp.rs#L403)

```rust
pub fn curve_option(&self, spec: &OptionSpec) -> Result<CurveOption>
```

A curve option, read the way `KisKritaSensorPack::read` reads it, down to its legacy quirks: a sensor the preset does not mention is reset, except that the old `Custom<id>` curve format keeps every sensor's state, which leaves pressure on beside whatever was read.

### `Preset::option_data`
[src/tools/krita/kpp.rs:409](../../../../src/tools/krita/kpp.rs#L409)

```rust
pub fn option_data(&self, spec: &OptionSpec) -> Result<OptionData>
```

An option's data, without judging its sensors: see `Preset::curve_option`.

### `Preset::set`
[src/tools/krita/kpp.rs:495](../../../../src/tools/krita/kpp.rs#L495)

```rust
pub fn set(&mut self, key: &str, value: &str)
```

Set one parameter, keeping its declared type (a new one is typed as Krita would: numbers and booleans `internal`, text `string`).

### `Preset::with_overrides`
[src/tools/krita/kpp.rs:513](../../../../src/tools/krita/kpp.rs#L513)

```rust
pub fn with_overrides(&self, overrides: &[(String, String)]) -> Preset
```

This preset with `overrides` (key, value) set over it.

### `Preset::to_xml`
[src/tools/krita/kpp.rs:524](../../../../src/tools/krita/kpp.rs#L524)

```rust
pub fn to_xml(&self) -> String
```

The `preset` XML, as `KisPaintOpPreset` saves it: each param in order, text in CDATA, byte arrays in base64, and (for a version 5.0 preset) the files it carries.

#### `Preset::to_xml::esc`
[src/tools/krita/kpp.rs:525](../../../../src/tools/krita/kpp.rs#L525)

```rust
fn esc(s: &str) -> String
```

Escapes text for XML.

## `impl OptionData`

### `OptionData::curve_option`
[src/tools/krita/kpp.rs:559](../../../../src/tools/krita/kpp.rs#L559)

```rust
pub fn curve_option(&self, spec: &OptionSpec) -> Result<CurveOption>
```

The option's sensors, or why one can't paint.

### `OptionData::write`
[src/tools/krita/kpp.rs:585](../../../../src/tools/krita/kpp.rs#L585)

```rust
pub fn write(&self, spec: &OptionSpec) -> Vec<(String, String)>
```

`KisKritaSensorPack::write`: the params that say this.

## Free functions

### `base64_encode`
[src/tools/krita/kpp.rs:611](../../../../src/tools/krita/kpp.rs#L611)

```rust
pub(crate) fn base64_encode(data: &[u8]) -> String
```

`QByteArray::toBase64`.

## Tests

### `mod tests`

Run with `cargo test tools::krita::kpp::tests` · 9 tests

| Test | What it checks |
|---|---|
| [`base64_is_lenient`](../../../../src/tools/krita/kpp.rs#L636) | Base64 is lenient. |
| [`options_write_back_as_they_read`](../../../../src/tools/krita/kpp.rs#L648) | Writing an option back and reading it again changes nothing, for every option of every shipped preset: the editor's edits round-trip. |
| [`the_preset_xml_reads_back`](../../../../src/tools/krita/kpp.rs#L675) | The XML writer's output reads back as the same preset. |
| [`reads_the_embedded_pencil`](../../../../src/tools/krita/kpp.rs#L706) | Reads the embedded pencil. |
| [`a_sensor_list_reads_every_child_and_the_common_curve`](../../../../src/tools/krita/kpp.rs#L747) | A sensor list reads every child and the common curve. |
| [`scatter_defaults_to_five_and_honours_the_old_amount`](../../../../src/tools/krita/kpp.rs#L765) | Scatter defaults to five and honours the old amount. |
| [`the_drawing_angle_brings_its_offset_and_fan_corners`](../../../../src/tools/krita/kpp.rs#L773) | The drawing angle brings its offset and fan corners. |
| [`the_old_custom_curve_format_leaves_pressure_on`](../../../../src/tools/krita/kpp.rs#L794) | The old custom curve format leaves pressure on. |
| [`unsupported_sensors_are_refused_by_name`](../../../../src/tools/krita/kpp.rs#L807) | Unsupported sensors are refused by name. |

**Test helpers** (used only by the tests above):

- [`preset`](../../../../src/tools/krita/kpp.rs#L721) — A preset holding just `params`, for exercising the option reader.
- [`kinds`](../../../../src/tools/krita/kpp.rs#L742) — Kinds.
