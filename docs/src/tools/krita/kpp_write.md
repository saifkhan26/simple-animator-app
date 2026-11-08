# `src/tools/krita/kpp_write.rs` — writing Krita brush presets (.kpp)

> **Source:** [src/tools/krita/kpp_write.rs](../../../../src/tools/krita/kpp_write.rs) · **Module:** `crate::tools::krita::kpp_write` · **Size:** 136 lines · Line links as of commit `1fa5845`

## What this file is for

Writes a `.kpp` file the way Krita's `KisPaintOpPreset::saveToDevice` does: a PNG of the preset's picture with
two text chunks added — `preset` (the settings XML from `Preset::to_xml`) and `version` = `5.0`, which tells
Krita to read the tip files embedded in the preset. Used when the brush library saves imported or edited
presets, so they open in Krita too.

```text
PNG signature | IHDR | … | tEXt "version\0" "5.0" | tEXt "preset\0" <xml> | IDAT … | IEND
```

## How it connects

- **Used by:** [brush_library/import.rs](../../brush_library/import.md),
  [brush_library/mod.rs](../../brush_library/mod.md), [ui/krita_editor.rs](../../ui/krita_editor.md).
- **Uses:** [tools/krita/kpp.rs](kpp.md) (`Preset`), the `image` crate (PNG encoding).

## Rust concepts in this file

**Building binary chunks.** `chunk(kind, body)` concatenates big-endian length, type, body and CRC32 into a
`Vec<u8>` — like assembling an `ArrayBuffer` with a `DataView`.

**A small MD5 implementation.** `md5_hex` is RFC 1321 written out (Krita names embedded resources by their MD5).

## Free functions

### `embed_tip`
[src/tools/krita/kpp_write.rs:13](../../../../src/tools/krita/kpp_write.rs#L13)

```rust
pub fn embed_tip(preset: &mut Preset, filename: &str, data: Vec<u8>)
```

Carry `data` as the tip `filename`, replacing a copy already carried.

### `write`
[src/tools/krita/kpp_write.rs:26](../../../../src/tools/krita/kpp_write.rs#L26)

```rust
pub fn write(preset: &Preset, picture: &image::RgbaImage) -> Result<Vec<u8>>
```

The `.kpp` bytes: `picture` with the preset's chunks.

### `chunk`
[src/tools/krita/kpp_write.rs:49](../../../../src/tools/krita/kpp_write.rs#L49)

```rust
fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8>
```

One PNG chunk: big-endian length, the 4-byte type, the body, and its CRC32.

### `md5_hex`
[src/tools/krita/kpp_write.rs:62](../../../../src/tools/krita/kpp_write.rs#L62)

```rust
pub fn md5_hex(data: &[u8]) -> String
```

MD5 (RFC 1321), as hex: what Krita names an embedded resource's contents by.

## Tests

### `mod tests`

Run with `cargo test tools::krita::kpp_write::tests` · 2 tests

| Test | What it checks |
|---|---|
| [`md5_matches_rfc_1321`](../../../../src/tools/krita/kpp_write.rs#L104) | Md5 matches rfc 1321. |
| [`a_written_preset_loads_on_its_own`](../../../../src/tools/krita/kpp_write.rs#L116) | A written preset carries its tip, so it loads with nothing else supplied, and it is still a PNG an image reader opens. |
