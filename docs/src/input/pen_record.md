# `src/input/pen_record.rs` — record a few seconds of pen vs. cursor positions to a CSV file

> **Source:** [src/input/pen_record.rs](../../../src/input/pen_record.rs) · **Module:** `crate::input::pen_record` · **Size:** 97 lines · Line links as of commit `67a7cb9`

## What this file is for

A debugging tool for pen-tablet problems. When you press **Record pen (6 s)** in the
tablet diagnostics panel, the app writes, once per frame for six seconds, where three
different things think the pointer is:

- the **pen** (the newest tablet packet, from [tablet.rs](tablet.md)),
- the **OS cursor** (from [screen_pixel.rs](screen_pixel.md)),
- **egui's pointer** (the position the UI library received).

…plus pressure, button state, any touch event, and the tablet mapping details. At the
end it saves everything as `animator-pen-record.csv` in the system temp folder.

Why a file instead of just looking at the on-screen readout? Some tablet problems only
happen while Animator is the *active* window. Taking a screenshot of the readout makes
the screenshot tool active instead, so the readout can never show those problems. A
recording runs while you keep using the pen in the app, and you read the file
afterwards.

A JS analogy: like attaching a `pointermove` listener that pushes
`[performance.now(), e.clientX, e.clientY, e.pressure]` into an array for six seconds,
then downloading it as CSV.

## How it connects

- **Used by:**
  - [ui/shell.rs](../ui/shell.md) — `tablet_diagnostics` has the
    "Record pen (6 s)" button that calls `PenRecord::start` and stores the recorder in
    `AppState::pen_record`, and shows the saved path or error afterwards.
  - [app.rs](../app.md) — `AppState::update` calls `frame` every frame while a
    recording exists (and requests a repaint so quiet frames are recorded too), then
    `finish` once `frame` reports the time is up; the result goes into
    `AppState::pen_record_saved`.
- **Uses:**
  - [tablet.rs](tablet.md) — `PenInput::diagnostics()` and `PenInput::packets()`.
  - [screen_pixel.rs](screen_pixel.md) — `cursor_pos()`.
  - egui's `Context::input` for focus, pointer position, scale factor, buttons and
    touch events.

## Rust concepts in this file

**Building a string with `writeln!`.** `writeln!(text, "...", a, b)` formats like
`format!` and appends the result plus a newline to `text`. It needs the
`std::fmt::Write` trait in scope; `use std::fmt::Write as _;` imports the trait's
methods without giving it a name (avoiding a clash with `std::io::Write`).
```rust
let _ = writeln!(text, "# monitors {:?}", d.display.monitors);
```
```js
text += `# monitors ${JSON.stringify(d.display.monitors)}\n`;
```
`{:?}` prints a value with its `Debug` format; `{:.2}` prints a float with 2
decimals. Writing to a `String` cannot really fail, so the `Result` is dropped with
`let _ =`. More: [macros](../../rust-primer.md#macros)

**`Option::map_or` for "value or empty".** Many columns may be missing (no pen yet,
cursor unknown). `map_or(default, f)` returns `default` for `None`, or `f(value)`.
```rust
cursor.map_or(String::new(), |c| c.0.to_string())
```
```js
cursor == null ? "" : String(cursor[0])
```
The file even defines a tiny closure `opt` that does this for `Option<f32>` with two
decimals. More: [Option and Result](../../rust-primer.md#option-and-result),
[closures](../../rust-primer.md#closures)

**Methods that consume `self`.** `finish(self)` takes the recorder *by value*: after
calling it, the `PenRecord` is gone (moved into the function), so nobody can keep
adding rows to a finished recording. The caller first takes it out of its
`Option` with `.take()`.
```rust
let rec = self.pen_record.take().expect("recording");
let saved = rec.finish();   // `rec` cannot be used after this line
```
More: [ownership and borrowing](../../rust-primer.md#ownership-and-borrowing),
[impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

**`Instant` and `Duration`.** `Instant::now()` is a monotonic clock reading (like
`performance.now()`), `started.elapsed()` the `Duration` since then, and
`Duration::from_secs(6)` a fixed length that can be compared with `>=`.

## impl PenRecord

## Types

### `const LENGTH`
[src/input/pen_record.rs:16](../../../src/input/pen_record.rs#L16)

```rust
pub const LENGTH: Duration = Duration::from_secs(6);
```

`Duration` of 6 seconds: how long a recording runs — time to click the button, then
sweep the pen across the tablet edge to edge and back.

### `struct PenRecord`
[src/input/pen_record.rs:18](../../../src/input/pen_record.rs#L18)

A recording in progress.

| Field | Type | Meaning |
|---|---|---|
| `started` | `Instant` | When the recording began; row times are measured from here. |
| `text` | `String` | The CSV text so far: `#` header lines, the column line, then one row per frame. |

## `impl PenRecord`

### `PenRecord::start`
[src/input/pen_record.rs:26](../../../src/input/pen_record.rs#L26)

```rust
pub fn start(pen: &PenInput) -> Self
```

**What it does.** Begins a recording. Writes header lines describing what the tablet
backend was opened against, then the CSV column names, and starts the clock.

**Parameters.**
- `pen` — the app's tablet input; its `diagnostics()` snapshot supplies the header.

**Returns.** A new `PenRecord`.

**How it works.** Writes these `#` comment lines from `PenDiagnostics` (see
[tablet.rs](tablet.md)):
1. the context's screen area, the window handle it is bound to and the one found by
   window title;
2. the monitor list;
3. the desktop bounding box and the driver's own reported area;
4. the driver-reported packet-to-screen mapping per axis (`map_x`, `map_y`);
5. the cursor-learned mapping per axis (`fit_x`, `fit_y`) and how many pairs it uses;
6. how many times the context was reopened, the last reason, and whether a backend
   exists.

Then the column line:
`t_ms,focused,cursor_x,cursor_y,egui_x,egui_y,client_x,client_y,ppp,raw_x,raw_y,pen_x,pen_y,packets,pressure,down,touch_x,touch_y`.

**Used by:** `tablet_diagnostics` in [ui/shell.rs](../ui/shell.md).

### `PenRecord::frame`
[src/input/pen_record.rs:52](../../../src/input/pen_record.rs#L52)

```rust
pub fn frame(&mut self, ctx: &egui::Context, pen: &PenInput) -> bool
```

**What it does.** Appends one CSV row for the current frame.

**Parameters.**
- `ctx` — egui context, read for window focus, the pointer's latest position,
  `pixels_per_point` (UI scale), whether any pointer button is down, and the newest
  `Touch` event this frame.
- `pen` — tablet input, read for its diagnostics and this frame's packets.

**Returns.** `true` once the recording has run for `LENGTH` (6 s) — the signal to call
`finish`.

**How it works.** One row holds, in column order:
1. `t_ms` — milliseconds since `start`.
2. `focused` — 1 if the app window has focus, else 0.
3. `cursor_x`, `cursor_y` — OS cursor in physical desktop pixels (`cursor_pos()`).
4. `egui_x`, `egui_y` — egui's pointer position (in points).
5. `client_x`, `client_y` — the window's client-area top-left on the desktop.
6. `ppp` — pixels per point (3 decimals).
7. `raw_x`, `raw_y` — the last raw packet coordinates from the driver.
8. `pen_x`, `pen_y` — the last packet of this frame, mapped to desktop pixels.
9. `packets` — how many packets arrived this frame.
10. `pressure` — that packet's pressure.
11. `down` — 1 if any pointer button is down.
12. `touch_x`, `touch_y` — position of the newest touch event, if any.

Missing values become empty cells; floats are written with 2 decimals.

**Used by:** `AppState::update` in [app.rs](../app.md).

### `PenRecord::finish`
[src/input/pen_record.rs:91](../../../src/input/pen_record.rs#L91)

```rust
pub fn finish(self) -> std::io::Result<PathBuf>
```

**What it does.** Writes the recording to `animator-pen-record.csv` in the OS temp
folder (`std::env::temp_dir()`), overwriting any earlier one.

**Returns.** `Ok(path)` with where it was written, or the I/O error (`?` passes it up).

**Used by:** `AppState::update` in [app.rs](../app.md), which stores the result (the
error as a `String`) for the diagnostics panel to show.
