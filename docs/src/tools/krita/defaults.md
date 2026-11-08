# `src/tools/krita/defaults.rs` — the 24 Krita brush presets built into the app

> **Source:** [src/tools/krita/defaults.rs](../../../../src/tools/krita/defaults.rs) · **Module:** `crate::tools::krita::defaults` · **Size:** 160 lines · Line links as of commit `67a7cb9`

## What this file is for

The app ships with a set of real Krita brush presets so you can paint with
Krita brushes without having Krita installed. They are the eraser, basic,
pencil, ink and marker **pixel-brush presets** from Krita's
`Krita_4_Default_Resources.bundle` (released as CC-0, i.e. public domain),
plus the three **brush tip** images those presets name.

All of those files are packed into one zip, `src/tools/krita/assets/defaults.zip`,
and that zip is **compiled into the executable** (`include_bytes!`). This file
is the little "file system" on top of it:

- `PRESETS` and `TIPS` list what is inside, in the order the brush picker shows them;
- [`file`](#file) pulls one file out of the zip by its path;
- [`kpp`](#kpp) and [`load`](#load) get a preset's bytes or a ready-to-paint
  [`PixelBrush`](mod.md#struct-pixelbrush);
- `DefaultTips` lets the preset loader find tip images inside the zip;
- a test-only cache ([`brush`](#brush)) and a maintenance "test" that
  rebuilds the zip from an installed Krita.

A *preset* (`.kpp`) is a PNG file whose text chunks hold the brush settings as
XML; a *tip* is the stamp image each dab is made from (`.png`, `.gbr`, or a
multi-image `.gih` "image pipe"). See [kpp.rs](kpp.md), [tip/mod.rs](tip/mod.md) and the
[glossary](../../../glossary.md).

## How it connects

- **Used by:**
  - [brush_library/mod.rs](../../brush_library/mod.md) — lists `PRESETS` as the
    built-in `krita/…` entries, loads them with `load`/`kpp`, uses
    `DefaultTips` as the fallback tip source (also behind a user folder), and
    lists `TIPS`.
  - [brush_library/import.rs](../../brush_library/import.md) — hashes every
    shipped preset (`kpp`) to recognise a built-in when importing, and its
    tests read tips with `file`.
  - [golden_tests.rs](../golden_tests.md) and [stroke.rs](../stroke.md) tests —
    paint every shipped preset through `brush`.
  - Inside this folder: [mod.rs](mod.md) tests (`brush`),
    [kpp.rs](kpp.md) and [kpp_write.rs](kpp_write.md) tests (`PRESETS`,
    `kpp`, `file`, `load`), [tip/gbr.rs](tip/mod.md) tests (`file`).
- **Uses:**
  - [mod.rs](mod.md) — `PixelBrush::load` and the `Resources` trait.
  - The `zip` crate to read the archive; `anyhow` for errors.

## Rust concepts in this file

**`include_bytes!` — embed a file in the program.** The zip is read by the
compiler and stored inside the `.exe` as a byte array; at run time it is just
memory.
```rust
const ZIP: &[u8] = include_bytes!("assets/defaults.zip");
```
```js
// Like a bundler inlining an asset: import zipBytes from "./defaults.zip?arraybuffer";
```
More: [Macros](../../../rust-primer.md#macros)

**`OnceLock` + `Mutex` — a lazily built, shared, thread-safe global.** The zip
reader is created the first time someone asks and then reused. `OnceLock`
guarantees it is built only once; the `Mutex` makes sure only one thread reads
from it at a time (reading moves an internal cursor).
```rust
fn archive() -> &'static Archive {
    static A: OnceLock<Archive> = OnceLock::new();
    A.get_or_init(|| Mutex::new(ZipArchive::new(Cursor::new(ZIP)).expect("embedded defaults.zip")))
}
```
```js
let _archive; // module-level
function archive() { return _archive ??= new ZipReader(ZIP); }
```
More: [Constants and statics](../../../rust-primer.md#constants-and-statics), [Smart pointers](../../../rust-primer.md#smart-pointers)

**`?` on an `Option` — return `None` early.** In `file`, every step that can
fail ends in `.ok()?`: `.ok()` turns a `Result` into an `Option`, and `?`
returns `None` from the function if it is empty.
```rust
let mut f = a.by_name(path).ok()?;
```
```js
const f = a.byName(path); if (f == null) return null;
```
More: [Option and Result](../../../rust-primer.md#option-and-result)

**`Cow` — "borrowed or owned".** `Resources::tip` returns `Cow<[u8]>`, which
can either point at bytes someone else owns (`Cow::Borrowed`) or carry its
own (`Cow::Owned`). The zip must decompress into a fresh `Vec`, so
`DefaultTips` always returns `Cow::Owned`.
More: [Smart pointers](../../../rust-primer.md#smart-pointers)

**Implementing a trait for a type.** `impl Resources for DefaultTips { … }`
is like making a class satisfy a TypeScript interface. `DefaultTips` is a
*unit struct* (no fields): it exists only to carry that behaviour.
More: [Traits and derive](../../../rust-primer.md#traits-and-derive)

**`#[cfg(test)]` and `#[ignore]`.** `brush` (and the `Arc` import it needs)
only exist in test builds. `regen_default_brushes` is marked `#[ignore]`, so a
normal `cargo test` skips it; you run it on purpose with `-- --ignored`.
More: [Attributes and cfg](../../../rust-primer.md#attributes-and-cfg), [Tests](../../../rust-primer.md#tests)

## impl Resources for DefaultTips

## Types

### `const ZIP`
[src/tools/krita/defaults.rs:17](../../../../src/tools/krita/defaults.rs#L17)

```rust
const ZIP: &[u8] = include_bytes!("assets/defaults.zip");
```

`&[u8]` — the whole `assets/defaults.zip`, embedded at compile time.

### `const PRESETS`
[src/tools/krita/defaults.rs:21](../../../../src/tools/krita/defaults.rs#L21)

```rust
pub const PRESETS: [&str; 24] = [
    "a)_Eraser_Circle.kpp",
    "a)_Eraser_Small.kpp",
    "a)_Eraser_Soft.kpp",
    "b)_Airbrush_Soft.kpp",
    "b)_Basic-2_Opacity.kpp",
// …
```

`[&str; 24]` — the file names of the 24 shipped presets inside the bundle, in
the order the brush picker lists them. The letter prefix groups them:
`a)` erasers (Circle, Small, Soft), `b)` airbrush and basic brushes,
`c)` pencils (Pencil-1 Hard … Pencil-6 Quick Shade), `d)` inks, `e)` markers.

### `const TIPS`
[src/tools/krita/defaults.rs:49](../../../../src/tools/krita/defaults.rs#L49)

```rust
pub const TIPS: [&str; 3] = ["gradient.png", "plain_rake.png", "graphite_grain.gih"];
```

`[&str; 3]` — the tip files those presets name: `gradient.png`,
`plain_rake.png` and `graphite_grain.gih`.

### `type Archive`
[src/tools/krita/defaults.rs:51](../../../../src/tools/krita/defaults.rs#L51)

```rust
type Archive = Mutex<ZipArchive<Cursor<&'static [u8]>>>;
```

`Mutex<ZipArchive<Cursor<&'static [u8]>>>` — a zip reader over the embedded
bytes (`Cursor` makes a byte slice look like a seekable file), behind a lock.

### `struct DefaultTips`
[src/tools/krita/defaults.rs:68](../../../../src/tools/krita/defaults.rs#L68)

A field-less marker type: "look for tips among the shipped files". It
implements [`Resources`](mod.md#trait-resources), the trait the preset loader
asks for tips it does not carry itself.

## Free functions

### `archive`
[src/tools/krita/defaults.rs:53](../../../../src/tools/krita/defaults.rs#L53)

```rust
fn archive() -> &'static Archive
```

**What it does.** Returns the one shared zip reader, opening the embedded zip
the first time it is called. It panics (`expect`) only if the embedded zip is
corrupt, which would be a broken build.

**Used by:** [`file`](#file).

### `file`
[src/tools/krita/defaults.rs:59](../../../../src/tools/krita/defaults.rs#L59)

```rust
pub fn file(path: &str) -> Option<Vec<u8>>
```

**What it does.** Reads one file out of the embedded zip and returns its
bytes.

**Parameters.**
- `path` — the path inside the zip: `paintoppresets/<name>` for a preset or
  `brushes/<name>` for a tip.

**Returns.** `Some(bytes)`, or `None` if the lock is poisoned, the file is not
in the zip, or reading fails.

**How it works.** Locks the archive, finds the entry by name, makes a `Vec`
sized to the entry's uncompressed size, and decompresses into it.

**Used by:** [`kpp`](#kpp), `DefaultTips::tip`, and tests in
[brush_library/import.rs](../../brush_library/import.md),
[kpp_write.rs](kpp_write.md) and `tip/gbr.rs`.

## `impl Resources for DefaultTips`

### `DefaultTips::tip`
[src/tools/krita/defaults.rs:71](../../../../src/tools/krita/defaults.rs#L71)

```rust
fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>>
```

**What it does.** Answers the preset loader's "where is tip `filename`?" by
reading `brushes/<filename>` from the zip, wrapped as an owned `Cow`.

**Used by:** `PixelBrush::load` / `PixelBrush::from_preset` in
[mod.rs](mod.md) when [`load`](#load) passes `&DefaultTips`; the
brush library passes it directly or chained behind a folder.

## Free functions

### `kpp`
[src/tools/krita/defaults.rs:77](../../../../src/tools/krita/defaults.rs#L77)

```rust
pub fn kpp(name: &str) -> Option<Vec<u8>>
```

**What it does.** Returns the raw `.kpp` bytes of a shipped preset, by its
file name from `PRESETS` (it reads `paintoppresets/<name>`).

**Used by:** [brush_library/mod.rs](../../brush_library/mod.md) (an embedded
source's bytes), [brush_library/import.rs](../../brush_library/import.md)
(hashing built-ins), [`load`](#load), and tests in
[kpp.rs](kpp.md) and [kpp_write.rs](kpp_write.md).

### `load`
[src/tools/krita/defaults.rs:82](../../../../src/tools/krita/defaults.rs#L82)

```rust
pub fn load(name: &str) -> Result<PixelBrush>
```

**What it does.** Loads a shipped preset into a ready-to-paint
[`PixelBrush`](mod.md#struct-pixelbrush), taking any tip it needs from the zip.

**Returns.** `Ok(brush)`, or an error: "no default preset `name`" if the name
is not in the zip, otherwise the loader's own error with the preset name
added as context (so the message reads like `c)_Pencil-2.kpp: …`).

**Used by:** [brush_library/mod.rs](../../brush_library/mod.md) (`Source::Embedded`),
[`brush`](#brush), and tests.

### `brush`
[src/tools/krita/defaults.rs:89](../../../../src/tools/krita/defaults.rs#L89)

```rust
pub fn brush(name: &str) -> Result<Arc<PixelBrush>>
```

**What it does.** Like [`load`](#load), but each preset is
loaded only once per test run and then shared as an `Arc` (a reference-counted
pointer — cloning it just bumps a counter). Many tests paint the same presets,
so this saves re-parsing them.

**How it works.** A `static` `OnceLock` holds a `Mutex<Vec<(name, Arc<PixelBrush>)>>`.
It looks the name up; on a hit it returns a clone of the `Arc`; on a miss it
loads, pushes, and returns it. The lock is released between the lookup and the
load, so two threads could both load the same preset once — harmless.

**Used by:** tests in [mod.rs](mod.md), [golden_tests.rs](../golden_tests.md)
and [stroke.rs](../stroke.md).

## Tests

Run with `cargo test tools::krita::defaults`.

| Test | What it checks |
|---|---|
| [`the_shipped_presets_load_or_say_why_not`](../../../../src/tools/krita/defaults.rs#L107) | Every name in `PRESETS` loads into a `PixelBrush`; any that is refused is printed with the reason, and the test asserts that all 24 load. |
| [`regen_default_brushes`](../../../../src/tools/krita/defaults.rs#L131) | **Not a real test — a maintenance tool, `#[ignore]`d.** Rebuilds `assets/defaults.zip` from an installed Krita's bundle (path from the `KRITA_BUNDLE` environment variable, defaulting to the usual Windows install path). It copies each preset and tip, deflate-compressed with a fixed timestamp so rebuilding gives the same zip. Run with `cargo test regen_default_brushes -- --ignored`. It only reads the bundle. |

### `mod tests`

Run with `cargo test tools::krita::defaults::tests` · 2 tests

| Test | What it checks |
|---|---|
| [`the_shipped_presets_load_or_say_why_not`](../../../../src/tools/krita/defaults.rs#L107) | The engine paints every shipped preset. |
| [`regen_default_brushes`](../../../../src/tools/krita/defaults.rs#L131) | Rebuild `assets/defaults.zip` from an installed Krita's CC-0 bundle. Reads the bundle only.  ```text KRITA_BUNDLE="C:/Program Files/Krita (x64)/share/krita/bundles/Krita_4_Default_Resources.bundle" \     cargo test regen_default_brushes -- --ignored ``` |
