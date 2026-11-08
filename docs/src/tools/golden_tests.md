# `src/tools/golden_tests.rs` — golden-image tests for every brush (test-only)

> **Source:** [src/tools/golden_tests.rs](../../../src/tools/golden_tests.rs) · **Module:** `crate::tools::golden_tests` · **Size:** 845 lines · Line links as of commit `36405ad`

## What this file is for

A test-only module (`mod golden_tests;` in [tools/mod.rs](mod.md) is only used by tests) that guards every
shipped brush against accidental changes. It draws each brush through the **real stroke code** over a fixed
matrix of inputs — gestures (straight, S-curve, taps…), tools, what is already on the layer (`Ground`), and
what holds the stroke in (`Hold`: free, a soft selection, alpha lock) — hashes the resulting pixels, and
compares with hashes stored in `src/tools/testdata/golden_strokes.tsv`.

This is the Rust cousin of **Jest snapshot tests**: the "snapshot" is a 64-bit hash of the image instead of a
serialized object. Recording only ever *appends* rows, so a changed hash always fails; on mismatch the stroke is
written to `target/golden-diff/<case>.png` to look at.

```bash
cargo test golden
```

```bash
cargo test golden --features exact-rcp
```

```bash
GOLDEN_RECORD=1 cargo test golden
```

**Why `exact-rcp`?** The ported Krita engine divides with the CPU instruction `_mm_rcp_ss` (a fast approximate
reciprocal), exactly as Krita does — its results differ slightly between Intel and AMD CPUs. So Krita-engine
hashes are stored per CPU "rcp signature". Building with the `exact-rcp` Cargo feature (declared in
`Cargo.toml`) replaces that instruction with a true division, giving one CPU-independent hash; it is for tests
only, never for a real build.

## How it connects

- **Uses:** [tools/stroke.rs](stroke.md), [tools/ribbon.rs](ribbon.md), [tools/mod.rs](mod.md) (brushes),
  [tools/krita/qt.rs](krita/qt.md) (`rcp_signature`), [tools/fill.rs](fill.md), [app.rs](../app.md)
  (`AppState::for_test` for app-level strokes).
- **Used by:** nothing — tests only.

## Rust concepts in this file

**Compile-time paths.** `concat!(env!("CARGO_MANIFEST_DIR"), "/src/tools/testdata/golden_strokes.tsv")` builds
the table path at compile time from the project folder (`env!` reads a compile-time environment variable).

**Test cases as boxed closures.** `struct Case { run: Box<dyn Fn() -> Canvas>, … }` — each case is a stored
function that produces its image, so the test loop can run them uniformly. More:
[Closures](../../rust-primer.md#closures)

**`#[ignore]` performance probes.** The perf test only runs with `cargo test --release perf_ -- --ignored`.

## Types

### `const W`
[src/tools/golden_tests.rs:38](../../../src/tools/golden_tests.rs#L38)

```rust
const W: u32 = 256;
```

Golden canvas width (256).

### `const H`
[src/tools/golden_tests.rs:39](../../../src/tools/golden_tests.rs#L39)

```rust
const H: u32 = 160;
```

Golden canvas height (160).

### `const TABLE`
[src/tools/golden_tests.rs:40](../../../src/tools/golden_tests.rs#L40)

```rust
const TABLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tools/testdata/golden_strokes.tsv");
```

Path of the stored hash table (`src/tools/testdata/golden_strokes.tsv`).

### `enum Ground`
[src/tools/golden_tests.rs:54](../../../src/tools/golden_tests.rs#L54)

What is on the layer before the stroke.

| Variant | Meaning |
|---|---|
| `Clear` | A transparent layer. |
| `Grey` | An opaque grey layer. |
| `Stripes` | Translucent red stripes over half the canvas, so the composite has partial alpha to work against. |
| `Ink` | Opaque dark ink everywhere: something for an eraser to take away. |

### `enum Hold`
[src/tools/golden_tests.rs:66](../../../src/tools/golden_tests.rs#L66)

What holds the stroke in.

| Variant | Meaning |
|---|---|
| `Free` | Nothing holds the stroke in. |
| `Selection` | A soft-edged selection over the middle of the canvas. |
| `AlphaLock` | The layer's alpha is locked. |

### `struct Case`
[src/tools/golden_tests.rs:183](../../../src/tools/golden_tests.rs#L183)

One golden case: its name, whether its hash depends on the CPU's `rcp` instruction, and the function that draws it.

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | The case's unique name (row key in the table). |
| `rcp` | `bool` | Paints through `_mm_rcp_ss`, so its hash depends on the CPU. |
| `run` | `Box<dyn Fn() -> Canvas>` | Draws the case and returns the resulting canvas. |

### `const PERF_TABLE`
[src/tools/golden_tests.rs:653](../../../src/tools/golden_tests.rs#L653)

```rust
const PERF_TABLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tools/testdata/perf_baseline.txt");
```

Path of the recorded performance baseline for the perf probe.

## Free functions

### `fnv`
[src/tools/golden_tests.rs:43](../../../src/tools/golden_tests.rs#L43)

```rust
fn fnv(bytes: &[u8]) -> u64
```

FNV-1a 64-bit hash of the pixel bytes.

### `canvas`
[src/tools/golden_tests.rs:73](../../../src/tools/golden_tests.rs#L73)

```rust
fn canvas(ground: Ground) -> Canvas
```

A fresh canvas with the given `Ground` painted on it.

### `selection`
[src/tools/golden_tests.rs:92](../../../src/tools/golden_tests.rs#L92)

```rust
fn selection() -> Arc<Mask>
```

A selection over x 60..200, y 30..130 whose coverage ramps up over its first 40 columns, so both hard and partial coverage are exercised.

### `sample`
[src/tools/golden_tests.rs:103](../../../src/tools/golden_tests.rs#L103)

```rust
fn sample(x: f32, y: f32, pressure: f32, tilt: (f32, f32)) -> PointerSample
```

A pointer sample at `(x, y)` with pressure and tilt.

### `gesture`
[src/tools/golden_tests.rs:116](../../../src/tools/golden_tests.rs#L116)

```rust
fn gesture(name: &str) -> Vec<PointerSample>
```

The named input gestures (lines, curves, taps…) as sample lists.

### `stroke_on`
[src/tools/golden_tests.rs:152](../../../src/tools/golden_tests.rs#L152)

```rust
fn stroke_on(
    c: &mut Canvas,
    ws: &mut StrokeWorkspace,
    brush: &BrushSettings,
    tool: ActiveTool,
    smoothing: SmoothingOptions,
    view_scale: f32,
    orient: (f64, (bool, bool)),
    hold: Hold,
    input: &[PointerSample],
)
```

Draws one freehand stroke the way the app does: `begin`, set the hold (selection/alpha lock), push each sample
with a `flush` after it, and `finish` at pen-up.

### `brushes`
[src/tools/golden_tests.rs:191](../../../src/tools/golden_tests.rs#L191)

```rust
fn brushes() -> Vec<(&'static str, BrushSettings, ActiveTool)>
```

Every shipped brush preset with the tool it's used on.

### `freehand_case`
[src/tools/golden_tests.rs:205](../../../src/tools/golden_tests.rs#L205)

```rust
fn freehand_case(
    name: String,
    brush: BrushSettings,
    tool: ActiveTool,
    ground: Ground,
    hold: Hold,
    input: &'static str,
    smoothing: SmoothingOptions,
    view_scale: f32,
    orient: (f64, (bool, bool)),
) -> Case
```

Builds a `Case` drawing one freehand stroke with the given brush, tool, ground, hold, gesture and smoothing.

### `cases`
[src/tools/golden_tests.rs:229](../../../src/tools/golden_tests.rs#L229)

```rust
fn cases() -> Vec<Case>
```

The full matrix of golden cases (brushes × gestures × grounds × holds × smoothing × zoom/orientation, plus shape
and fill cases).

### `tool_name`
[src/tools/golden_tests.rs:476](../../../src/tools/golden_tests.rs#L476)

```rust
fn tool_name(t: ActiveTool) -> &'static str
```

A tool's short name for case names.

### `app_stroke`
[src/tools/golden_tests.rs:487](../../../src/tools/golden_tests.rs#L487)

```rust
fn app_stroke(tool: Action, brush: BrushSettings) -> Canvas
```

A stroke through a whole `AppState` (as the app runs it), returning the drawing as painted on top of the drawing
after undo — so undo is checked too.

### `variant`
[src/tools/golden_tests.rs:526](../../../src/tools/golden_tests.rs#L526)

```rust
fn variant(rcp: bool) -> String
```

Which hash column a case uses: `portable`, `exact` (with `exact-rcp`) or `rcp-<cpu signature>`.

### `load_table`
[src/tools/golden_tests.rs:536](../../../src/tools/golden_tests.rs#L536)

```rust
fn load_table() -> HashMap<(String, String), u64>
```

Reads the TSV hash table into a map from (case, variant) to hash, skipping blank and `#` lines.

### `dump`
[src/tools/golden_tests.rs:548](../../../src/tools/golden_tests.rs#L548)

```rust
fn dump(name: &str, c: &Canvas)
```

Writes a canvas as a PNG into `target/golden-diff/` for inspecting a mismatch.

## Tests

### Tests in this file

Run with `cargo test tools::golden_tests` · 4 tests

| Test | What it checks |
|---|---|
| [`golden_strokes`](../../../src/tools/golden_tests.rs#L558) | Golden strokes. |
| [`noise_render`](../../../src/tools/golden_tests.rs#L613) | Writes airbrush strokes with and without midtone noise, three passes of the S each, over white, as `brush_noise_render.png` into `SNAPSHOT_DIR`: Airbrush; Grainy airbrush; noise size 2.5; noise from the paper; stipple; and the grainy airbrush as an eraser on black. `SNAPSHOT_DIR=... cargo test -- --ignored noise_render` |
| [`perf_brushes_1080p`](../../../src/tools/golden_tests.rs#L666) | Stroke timings on a 1080p cell, against this machine's recorded baseline. Release only means anything:  ```text cargo test --release perf_ -- --ignored --nocapture     # report PERF_RECORD=1 cargo test --release perf_ -- --ignored   # add missing rows PERF_RERECORD=1 cargo test --release perf_ -- --ignored # re-baseline this machine PERF_STRICT=1 cargo test --release perf_ -- --ignored   # fail beyond +15% ``` |
| [`a_grainy_pencil_ring_holds_the_bucket`](../../../src/tools/golden_tests.rs#L808) | Paper grain and noise riddle a pencil line with pixels under half covered. The bucket pressed inside a ring drawn with one fills the ring, and not the page round it. |
