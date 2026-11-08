# `src/input/tablet.rs` — pen tablet input through the Wintab driver

> **Source:** [src/input/tablet.rs](../../../src/input/tablet.rs) · **Module:** `crate::input::tablet` · **Size:** 2100 lines · Line links as of commit `67a7cb9`

## What this file is for

This file reads a **pen tablet** (Wacom, XP-Pen, Huion…) directly from its driver, so
strokes get every sample the pen produces — with pressure, tilt, barrel rotation and
the driver's own timestamps — instead of the one mouse position per frame that the
GUI library sees.

**Why not just use the mouse position?** The tablet driver also moves the normal OS
cursor, and egui (the GUI library) turns that into ordinary mouse events. But the app
only sees *one* cursor position per redraw. A pen reports 133–266 times a second, so
most samples are thrown away and the survivors are 2–4 device samples apart, rounded to
whole screen pixels. Zoomed out, those gaps are several canvas pixels wide and the line
becomes a visible staircase. Reading *every* queued pen packet removes it. The browser
has the same problem and the same fix:
[`PointerEvent.getCoalescedEvents()`](https://developer.mozilla.org/docs/Web/API/PointerEvent/getCoalescedEvents)
gives you the extra pen samples that were merged into one `pointermove`.

The strategy copies Krita (really Qt's `qwindowstabletsupport.cpp`): the driver's normal
cursor emulation keeps driving egui's *press and release*, but stroke *positions* come
from the tablet's packet queue.

**Wintab.** On Windows the file talks to the tablet through **Wintab**, an old C API
that tablet drivers install as `Wintab32.dll`. The app opens a *context* (a
subscription: "send me pen data for this window, in these coordinates, with these
fields") and the driver puts *packets* (one pen sample each) into a queue that the app
drains. The DLL is loaded at run time with `libloading`, so the app still starts on a
machine with no tablet driver. Linux and macOS have no backend yet: no packets are
produced and drawing falls back to the egui pointer at pressure 1.0. (Windows Ink is
not used.)

A pen packet compared with a browser `PointerEvent`:

| `PenPacket` field | `PointerEvent` | Note |
|---|---|---|
| `x`, `y` | `screenX`, `screenY` | here in *physical* virtual-desktop pixels, with sub-pixel precision |
| `pressure` | `pressure` | 0..1 |
| `tilt_x`, `tilt_y` | `tiltX`, `tiltY` | degrees −90..90, x toward screen-right, y toward screen-down |
| `rotation` | `twist` | degrees, but −180..180 |
| `tangential` | `tangentialPressure` | airbrush finger wheel |
| `time_ms` | `timeStamp` | driver's clock; only differences matter |

**What makes this file long** is that drivers do not always do what they say. The
comments record several real bugs, and the code defends against each:

- **Packet size varies.** The driver may grant fewer fields than asked for, and each
  packet then gets shorter. Reading packets through a fixed-size struct worked for the
  first packet of a batch and read every later one at the wrong offset.
  `PacketLayout` computes the real offsets from the granted field mask.
- **Sub-pixel positions.** The app asks the driver to report positions on an area 32
  times bigger than the screen (`SUBPIXEL`), then divides back down, so positions land
  between screen pixels.
- **Y axis flip.** A tablet's Y axis points up, a screen's points down. Getting this
  wrong mirrored every position about the middle of the screen. `axis_map` carries the
  sign through.
- **Lying output areas.** One driver reported back an area it did not actually use, so
  every position was a screen-height off. `CursorFit` therefore *learns* the
  packet-to-screen mapping from where the driver puts the OS cursor (which it moves
  from the same packets), with a least-squares straight-line fit per axis.
- **Desktop changes.** Plugging in a monitor or changing the tablet's work area makes
  the old context wrong. `DisplayWatch` notices and reopens the context (never in the
  middle of a stroke).
- **Driver missing / refusing.** `Retry` backs off between attempts to open the
  context.
- **Invisible failures.** A release build has no console, so `PenDiagnostics`
  snapshots everything for an on-screen readout.

## How it connects

- **Used by:**
  - [app.rs](../app.md) — `AppState` owns one `PenInput` (`state.pen`). `AppState::update`
    hands it the window handle (`set_hwnd`) and calls `poll` once per frame;
    `make_sample` and `make_pen_sample` turn packets into
    [`PointerSample`](pointer.md)s.
  - [ui/shell.rs](../ui/shell.md) — the canvas drag code (`pen_stroke_start`,
    `pen_stroke_points`, `pen_agrees_with_cursor`, `pen_to_points`) reads
    `packets()`, `last_packet()`, `trail()`, `client_origin()` and `pen_active()`;
    `tablet_status` shows a pen/mouse label from `is_active()`/`pen_active()`;
    `tablet_diagnostics` / `display_diagnostics` show `diagnostics()` and use
    `monitors_spanned`, `reach_fits_one_monitor`, `ScreenRect` and
    `ReopenReason::label`; `draw_tool_cursor` and `draw_krita_outline` read pressure
    and the last packet for the brush cursor.
  - [pen_record.rs](pen_record.md) — records `diagnostics()` and `packets()` to CSV.
  - [msg_hook.rs](msg_hook.md) — forwards every raw window message to
    `tablet::on_msg` so driver change notices reach this file.
- **Uses:**
  - [screen_pixel.rs](screen_pixel.md) — `cursor_pos()`, the OS cursor, for the cursor
    fit.
  - External crates: `wintab_lite` (Wintab types and function signatures), `libloading`
    (load the DLL), `windows` (Win32 calls: `FindWindowA`, `GetClientRect`,
    `ClientToScreen`, `EnumDisplayMonitors`, `GetSystemMetrics`…), `anyhow` (errors),
    `log`.

```text
pen ─► tablet driver ─► Wintab32.dll packet queue (up to 128 packets)
                                    │
 every frame: AppState::update ─► PenInput::poll(pointer_down)
                                    ├─ DisplayWatch::tick / Retry ─► Wintab::try_init ─► open
                                    └─ Wintab::poll ─► poll_once (WTPacketsGet → bytes
                                    │                   → PacketLayout offsets → PenPacket)
                                    │              └─► CursorFit::observe(raw, cursor_pos())
                                    ▼
             packets() / last_packet() / trail() / client_origin()
                                    │
 ui/shell.rs canvas drag ─► pen_stroke_start / pen_stroke_points (screen points, sanity checks)
                                    │
                          stroke_sample ─► AppState::make_pen_sample ─► PointerSample
                                    │
                          AppState::pointer_down / pointer_move ─► stroke builder

 msg_hook.rs ─► tablet::on_msg (driver "something changed" message) ─► NUDGE ─► next poll
```

## Rust concepts in this file

**`unsafe`, FFI and loading a DLL at run time.** *FFI* (foreign function interface)
means calling code written in another language — here C functions inside
`Wintab32.dll`. `libloading` opens the DLL and looks functions up by name; each
lookup gives a `Symbol`, a typed function pointer. Both steps are `unsafe`: Rust cannot
check that the DLL's function really has the signature you claim.
```rust
let lib = unsafe { Library::new("Wintab32.dll") }?;            // fails if no driver
let wt_open: WTOpen<'static> = unsafe { lib.get(c"WTOpenA".to_bytes())? };
let ctx_handle = unsafe { wt_open(hwnd, &mut log_context, 1) };
```
```js
// Node + a native FFI library, roughly:
const wintab = ffi.Library("Wintab32.dll", { WTOpenA: ["pointer", ["pointer", "pointer", "int"]] });
const ctx = wintab.WTOpenA(hwnd, logContextBuffer, 1);
```
`c"WTOpenA"` is a C-string literal (zero-terminated bytes), which is what a DLL
symbol name is. The `'static` in `WTOpen<'static>` says the function pointer may live
forever — true because the library itself is kept in a global for the whole process
(see `LIB`). More: [unsafe and FFI](../../rust-primer.md#unsafe-and-ffi),
[lifetimes](../../rust-primer.md#lifetimes)

**`#[repr(C)]` structs and "fill this in for me" pointers.** Wintab exchanges data
through C structs such as `LOGCONTEXT` (the context settings) and `AXIS` (a value
range). The `wintab_lite` crate declares them with `#[repr(C)]`, which tells Rust to lay
the fields out in memory exactly as a C compiler would (normally Rust may reorder
fields). The code then passes a raw pointer to such a struct and the driver writes into
it. `cast_void!` is a small `wintab_lite` macro for `&mut x as *mut _ as *mut c_void`.
```rust
let mut log_context = LOGCONTEXT::default();
let r = unsafe { wt_info(WTI::DEFSYSCTX, 0, cast_void!(log_context)) }; // driver fills it
if r == 0 { return Err(anyhow!("WTInfo(DEFSYSCTX) failed")); }
```
In JS terms: like handing a native function an `ArrayBuffer` whose byte layout you and
the native side agreed on in advance. More:
[unsafe and FFI](../../rust-primer.md#unsafe-and-ffi)

**Raw pointers and reading bytes by hand.** The context handle is a raw pointer
(`*mut HCTX`); "no context" is a null pointer, checked with `.is_null()`. Packets arrive
as raw bytes in a `Vec<u8>`, and fields are decoded at computed offsets:
```rust
fn read_i32(buf: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}
```
```js
const readI32 = (view, at) => view.getInt32(at, /* littleEndian */ true);
```
More: [numbers and casts](../../rust-primer.md#numbers-and-casts),
[strings and slices](../../rust-primer.md#strings-and-slices)

**`impl Drop` — cleanup that always runs.** `Drop::drop` is called automatically when a
value goes away (goes out of scope, is replaced, or is `take()`n out of an `Option` and
discarded). `Wintab` uses it to close the driver context, so reopening a context is just
`self.backend.take()` followed by opening a new one.
```rust
impl Drop for Wintab {
    fn drop(&mut self) { let _ = unsafe { (self.wt_close)(self.ctx_handle) }; }
}
```
```js
// closest JS: explicit resource management
class Wintab { [Symbol.dispose]() { WTClose(this.ctx); } }
using w = openWintab();   // disposed at end of scope
```
More: [traits and derive](../../rust-primer.md#traits-and-derive)

**Globals shared with the message hook: `static`, `OnceLock`, atomics.** The window
message hook (in [msg_hook.rs](msg_hook.md)) runs outside `PenInput`, so it talks to it
through global `static`s. `OnceLock<Library>` is "initialise once, then read forever";
`AtomicBool` / `AtomicU32` are numbers that are safe to read and write from anywhere
without a lock.
```rust
static NUDGE: AtomicBool = AtomicBool::new(false);
NUDGE.store(true, Ordering::Relaxed);               // in on_msg
let nudged = NUDGE.swap(false, Ordering::Relaxed);  // in take_nudge: read and reset
```
`Ordering::Relaxed` means "no ordering guarantees with other memory", which is fine for
a simple hint flag. More: [constants and statics](../../rust-primer.md#constants-and-statics),
[threads and channels](../../rust-primer.md#threads-and-channels)

**`#[cfg]` on fields and code blocks.** `PenInput` has a `backend` field whose *type*
depends on the OS — `Option<Wintab>` on Windows, `()` (the empty "unit" type, like
`undefined`) elsewhere — and methods contain `#[cfg(target_os = "windows")] { … }`
blocks. The whole `windows_backend` module only exists on Windows.
```rust
#[cfg(target_os = "windows")]
backend: Option<windows_backend::Wintab>,
#[cfg(not(target_os = "windows"))]
backend: (),
```
More: [attributes and cfg](../../rust-primer.md#attributes-and-cfg)

## Types

### `struct PenPacket`
[src/input/tablet.rs:29](../../../src/input/tablet.rs#L29)

One tablet sample. `Clone + Copy + Debug`. Position is in *virtual-desktop pixels*
(the coordinate space spanning all monitors, the same one the OS cursor uses), with
sub-pixel precision.

| Field | Type | Meaning |
|---|---|---|
| `x` | `f32` | Pen position, horizontal, in virtual-desktop pixels (fractions allowed — the driver is asked for sub-pixel precision). |
| `y` | `f32` | Pen position, vertical, in virtual-desktop pixels. |
| `pressure` | `f32` | Normalised 0..=1. |
| `tilt_x` | `f32` | Tilt in degrees, -90..=90, x toward screen-right and y toward screen-down. Zero when the device reports no orientation. |
| `tilt_y` | `f32` | Tilt toward screen-down in degrees (see `tilt_x`). |
| `time_ms` | `f64` | The packet's own timestamp in ms on the driver's clock (0 when none). Only differences between packets mean anything. |
| `rotation` | `f32` | Barrel rotation (twist), degrees −180..=180, mapped the way Krita maps it. |
| `tangential` | `f32` | Tangential pressure 0..=1 — an airbrush's finger wheel; 0 for every other pen. |

### `struct PacketLayout`
[src/input/tablet.rs:64](../../../src/input/tablet.rs#L64)

Where each field this module reads sits inside one packet (byte offset), and how long a
packet is. Derived from the field mask the driver *actually granted*: `WTOpen` rewrites
the requested mask (`lcPktData`) with what it is willing to report, granted fields
appear in the fixed order of the `WTPKT` bits, and nothing else is in the packet. So the
packet size depends on the device. `Default + PartialEq + Eq + Copy`.

| Field | Type | Meaning |
|---|---|---|
| `mask` | `u32` | The granted field mask this layout was built from. |
| `size` | `usize` | Stride between packets, in bytes. |
| `status` | `Option<usize>` | Offset of the STATUS field (`None` if not granted). |
| `x` | `Option<usize>` | Byte offset of the X coordinate inside one packet, if the driver granted it. |
| `y` | `Option<usize>` | Byte offset of the Y coordinate inside one packet, if granted. |
| `pressure` | `Option<usize>` | Offset of NORMAL_PRESSURE (tip pressure). |
| `orientation` | `Option<usize>` | Offset of the ORIENTATION triple: azimuth, altitude, twist (3 × 4 bytes). |
| `time` | `Option<usize>` | Offset of TIME. |
| `cursor` | `Option<usize>` | Offset of CURSOR (which stylus / end of stylus). |
| `tangent` | `Option<usize>` | Offset of TANGENT_PRESSURE. |

### `const PACKET_FIELDS`
[src/input/tablet.rs:85](../../../src/input/tablet.rs#L85)

```rust
const PACKET_FIELDS: [(u32, usize); 14] = [
    (1 << 0, std::mem::size_of::<usize>()), // CONTEXT
    (1 << 1, 4),                            // STATUS
    (1 << 2, 4),                            // TIME
    (1 << 3, 4),                            // CHANGED
    (1 << 4, 4),                            // SERIAL_NUMBER
// …
```

`[(u32, usize); 14]` — every Wintab packet field as `(bit, size in bytes)`, in `WTPKT`
bit order. This order *is* the layout. `CONTEXT` (bit 0) is a handle, so pointer-sized
(`size_of::<usize>()`, 8 bytes on 64-bit); `ORIENTATION` and `ROTATION` are 12-byte
triples; every other field is 4 bytes. The bits are: 0 CONTEXT, 1 STATUS, 2 TIME,
3 CHANGED, 4 SERIAL_NUMBER, 5 CURSOR, 6 BUTTONS, 7 X, 8 Y, 9 Z, 10 NORMAL_PRESSURE,
11 TANGENT_PRESSURE, 12 ORIENTATION, 13 ROTATION.

### `struct AxisFit`
[src/input/tablet.rs:172](../../../src/input/tablet.rs#L172)

One axis of a *fitted* packet → screen mapping, applied as `origin + raw * scale`.
`Copy + PartialEq`.

| Field | Type | Meaning |
|---|---|---|
| `origin` | `f64` | Screen position of raw value 0. |
| `scale` | `f64` | Screen pixels per raw packet unit (negative for a flipped axis). |
| `rms` | `f64` | Root-mean-square distance of the fitted pairs from the line, in screen pixels — how well the line fits. Whole-pixel cursor rounding alone gives about 0.3. |

### `const FIT_WINDOW`
[src/input/tablet.rs:189](../../../src/input/tablet.rs#L189)

```rust
const FIT_WINDOW: usize = 256;
```

Pairs pulled into a fit. Enough to average whole-pixel rounding down to a small fraction of a pixel, few enough to refit every frame.

### `const FIT_MIN_PAIRS`
[src/input/tablet.rs:191](../../../src/input/tablet.rs#L191)

```rust
const FIT_MIN_PAIRS: usize = 16;
```

Fewest pairs a fit is trusted on.

### `const FIT_MIN_SPAN`
[src/input/tablet.rs:194](../../../src/input/tablet.rs#L194)

```rust
const FIT_MIN_SPAN: f64 = 48.0;
```

Cursor travel, in screen pixels, the pairs must span on an axis before that axis's slope means anything.

### `const FIT_MAX_RMS`
[src/input/tablet.rs:197](../../../src/input/tablet.rs#L197)

```rust
const FIT_MAX_RMS: f64 = 1.5;
```

Scatter above which the pairs are not describing a straight line — a mouse moved alongside the pen, say — and the fit is not used.

### `const FIT_MAX_STEP`
[src/input/tablet.rs:200](../../../src/input/tablet.rs#L200)

```rust
const FIT_MAX_STEP: i32 = 8;
```

Cursor travel in one frame above which a pair is left out: moving that fast, the gap between reading the queue and reading the cursor matters.

### `const FIT_MISS_PX`
[src/input/tablet.rs:205](../../../src/input/tablet.rs#L205)

```rust
const FIT_MISS_PX: f64 = 6.0;
```

A new pair this far off a trusted fit is a miss. A miss is left out of the fit — it is usually the cursor lagging the pen for a moment — but a long enough run of them means the mapping itself has changed (a monitor rearranged, the tablet remapped in its driver), and the fit starts over.

### `const FIT_MISSES_TO_RESET`
[src/input/tablet.rs:206](../../../src/input/tablet.rs#L206)

```rust
const FIT_MISSES_TO_RESET: u32 = 24;
```

How many packets in a row may disagree with the learned packet→cursor mapping before the mapping is
thrown away and learned again (24). Guards against a stale fit after the driver's settings change.

### `struct CursorFit`
[src/input/tablet.rs:224](../../../src/input/tablet.rs#L224)

The packet → screen mapping *learned from where the driver puts the cursor*. The output
area a driver reports back from `WTOpen` cannot be trusted (one driver reported a
flipped area and delivered top-down coordinates anyway, putting every position a
screen-height off). The cursor is always right, because the driver moves it from the
same packets; it is only rounded to whole pixels, and a straight-line fit over many
pairs averages that rounding away. So the fit recovers flip, offset and scale to a
fraction of a pixel. `Clone + Debug + Default`.

| Field | Type | Meaning |
|---|---|---|
| `pairs` | `std::collections::VecDeque<((i32, i32), (i32, i32))>` | Recent `(raw packet x/y, cursor x/y)` pairs, oldest first, at most `FIT_WINDOW`. A `VecDeque` is a double-ended queue (cheap push at the back, pop at the front). |
| `last_cursor` | `Option<(i32, i32)>` | Cursor at the previous `observe`, for the per-frame step check; `None` after `pause`. |
| `misses` | `u32` | Consecutive pairs that were far from the trusted fit. |
| `x` | `Option<AxisFit>` | The learned mapping for the horizontal axis, once there are enough points to trust it. |
| `y` | `Option<AxisFit>` | The learned mapping for the vertical axis. |

### `struct ScreenRect`
[src/input/tablet.rs:345](../../../src/input/tablet.rs#L345)

A rectangle in virtual-desktop pixels. Its width/height may be negative (Wintab
rectangles can run "backwards"). Derives `Copy`, `Default`, equality and ordering
(`PartialOrd + Ord`, so a list of them can be sorted).

| Field | Type | Meaning |
|---|---|---|
| `x` | `i32` | Left edge in desktop pixels. |
| `y` | `i32` | Top edge in desktop pixels. |
| `w` | `i32` | Width (may be negative in Wintab's own rectangles; see `normalized`). |
| `h` | `i32` | Height (may be negative). |

### `struct DriverArea`
[src/input/tablet.rs:372](../../../src/input/tablet.rs#L372)

Where the tablet driver *says* it puts the pen.

| Field | Type | Meaning |
|---|---|---|
| `screen` | `ScreenRect` | Screen area its system context moves the cursor over (`lcSysOrgXY` / `lcSysExtXY`). |
| `tablet` | `(i32, i32)` | Size of the tablet area it reads (`lcInExtXYZ`), in tablet counts. |

### `struct DisplaySig`
[src/input/tablet.rs:379](../../../src/input/tablet.rs#L379)

A "signature" of the desktop a context was opened against; two of them are compared to
spot changes. `Clone + Default + PartialEq`.

| Field | Type | Meaning |
|---|---|---|
| `desktop` | `ScreenRect` | The virtual desktop's bounding box. |
| `monitors` | `Vec<(ScreenRect, bool)>` | Every monitor, `true` for the primary one. Sorted, so a different enumeration order never looks like a change. |
| `driver` | `Option<DriverArea>` | The driver's area; `None` when there is no driver to ask. |

### `enum ReopenReason`
[src/input/tablet.rs:391](../../../src/input/tablet.rs#L391)

Why the context was last reopened.

| Variant | Meaning |
|---|---|
| `Monitors` | The desktop box or the monitor list changed. |
| `Driver` | The driver's reported area changed. |
| `Requested` | The user pressed **Reconnect tablet**. |

### `const DISPLAY_CHECK`
[src/input/tablet.rs:410](../../../src/input/tablet.rs#L410)

```rust
const DISPLAY_CHECK: std::time::Duration = std::time::Duration::from_millis(500);
```

`Duration` of 500 ms: how often the desktop layout is re-read. Measured in time, not
frames, because the app only redraws when something happens — the first frame after a
quiet spell is the one that should notice.

### `struct DisplayWatch`
[src/input/tablet.rs:415](../../../src/input/tablet.rs#L415)

Watches the desktop and the driver's area for changes since the context opened.
`Clone + Debug + Default`.

| Field | Type | Meaning |
|---|---|---|
| `at_open` | `Option<DisplaySig>` | What the current context was built against. |
| `last_check` | `Option<std::time::Instant>` | When the layout was last read. |
| `pending` | `Option<ReopenReason>` | A change seen mid-stroke, held until the pen lifts (reopening under a stroke would remap the rest of it). |

### `enum InitFail`
[src/input/tablet.rs:471](../../../src/input/tablet.rs#L471)

Why opening the context failed; this decides when to try again.

| Variant | Meaning |
|---|---|
| `NoWindow` | The app window is not up yet. Cheap to retry, so every frame. |
| `NoDriver` | No `Wintab32.dll`. Nothing changes until the user installs a driver and presses Reconnect. |
| `Context` | The driver exists but refused the context. Retried with growing waits, so a tablet plugged in later is still picked up. |

### `struct Retry`
[src/input/tablet.rs:484](../../../src/input/tablet.rs#L484)

Backoff state for opening the context. `Copy + Default`.

| Field | Type | Meaning |
|---|---|---|
| `failures` | `u32` | Refused-context failures so far (sets the wait length). |
| `next_at` | `Option<std::time::Instant>` | Earliest time of the next attempt; `None` = now. |
| `parked` | `bool` | Waiting for Reconnect rather than for a time (no driver). |

### `const RETRY_FIRST`
[src/input/tablet.rs:492](../../../src/input/tablet.rs#L492)

```rust
const RETRY_FIRST: f32 = 0.5;
```

`0.5` and `10.0` seconds: the first and the longest wait after a refused context. The
wait doubles each failure: 0.5, 1, 2, 4, 8, 10, 10…

### `const RETRY_MAX`
[src/input/tablet.rs:493](../../../src/input/tablet.rs#L493)

```rust
const RETRY_MAX: f32 = 10.0;
```

`0.5` and `10.0` seconds: the first and the longest wait after a refused context. The
wait doubles each failure: 0.5, 1, 2, 4, 8, 10, 10…

### `const REACH_SLACK`
[src/input/tablet.rs:548](../../../src/input/tablet.rs#L548)

```rust
const REACH_SLACK: f32 = 8.0;
```

`8.0` pixels a measured pen reach may run past a monitor's size and still count as
fitting on it (the packet range and the screen can disagree by one at each edge).

### `struct PenDiagnostics`
[src/input/tablet.rs:568](../../../src/input/tablet.rs#L568)

A snapshot of what the tablet backend is seeing, for the on-screen readout and
[pen_record.rs](pen_record.md). It exists because the interesting failures are
invisible: a release build has no console, so log lines go nowhere, and a mis-mapped
axis draws confidently in the wrong place instead of reporting an error. Everything is
read off the last poll; nothing is computed on the drawing path. `Clone + Debug + Default`.

| Field | Type | Meaning |
|---|---|---|
| `backend` | `bool` | A Wintab context is open. |
| `packets_this_frame` | `usize` | Packets drained by the last poll. |
| `total_packets` | `u64` | Packets since the context opened. |
| `last_raw` | `Option<(i32, i32)>` | Raw packet coordinates exactly as the driver sent them. |
| `last_mapped` | `Option<(f32, f32)>` | The same point mapped to virtual-desktop pixels. |
| `client_origin` | `Option<(f32, f32)>` | Top-left of the window's client area on the desktop. |
| `map_x` | `(f32, f32, f32)` | `(origin, packet_origin, scale)` per axis — see `axis_map`. |
| `map_y` | `(f32, f32, f32)` | The vertical packet→screen mapping in use, `(origin, packet_origin, scale)`. |
| `pressure` | `f32` | Last pressure. |
| `tilt` | `(f32, f32)` | Tilt of the last packet this frame. |
| `queue_overflowed` | `bool` | The driver flagged a queue overflow (samples were dropped). |
| `max_packets_per_frame` | `usize` | Most packets in one poll. A stride (packet size) error only shows when this is above 1, so it says whether that case was even exercised. |
| `drained_full` | `u32` | Polls that filled the read buffer, meaning the queue had backed up. |
| `layout` | `PacketLayout` | The granted packet layout. A device granting every field gives 76 bytes on a 64-bit build. |
| `fit_x` | `Option<AxisFit>` | The mapping learned from the cursor, per axis, once trusted, and how many pairs it is drawn from. See `CursorFit`. |
| `fit_y` | `Option<AxisFit>` | The learned vertical cursor fit, if trusted. |
| `fit_pairs` | `usize` | How many pairs the fit is drawn from. |
| `reach` | `Option<(f32, f32)>` | Desktop pixels the whole tablet covers, as the cursor shows it (`pen_reach`). |
| `context_area` | `Option<ScreenRect>` | Screen area the context was opened with (`lcSys*`). |
| `hwnd` | `Option<isize>` | The window handle the context is bound to. |
| `hwnd_by_title` | `Option<isize>` | The window `FindWindowA` finds by the title "Animator" (how the context was bound before the app handed its own handle over); recorded to tell the two apart. |
| `display` | `DisplaySig` | Desktop and driver area, read when the snapshot is taken. |
| `reopens` | `u32` | Times the context was reopened. |
| `last_reopen` | `Option<ReopenReason>` | Latest reason. |
| `init_error` | `Option<String>` | Why the last open attempt failed, while there is no context. |
| `retry_in` | `Option<std::time::Duration>` | Time until the next attempt; `None` when due now or waiting for Reconnect. |
| `waiting_for_reconnect` | `bool` | Parked until the user presses Reconnect. |

### `const PEN_IDLE_FRAMES`
[src/input/tablet.rs:621](../../../src/input/tablet.rs#L621)

```rust
const PEN_IDLE_FRAMES: u32 = 12;
```

`12` frames without a packet, with nothing pressed, after which the pen counts as gone
and pressure falls back to 1.0 (the mouse took over). Never applied while a button is
down, so a stroke that pauses mid-line keeps its pressure.

### `const TRAIL_LEN`
[src/input/tablet.rs:625](../../../src/input/tablet.rs#L625)

```rust
const TRAIL_LEN: usize = 32;
```

`32` packets kept in `PenInput::trail` — about 130 ms of a 240 Hz pen, compared with a
cursor that trails the newest packet by a few milliseconds.

### `struct PenInput`
[src/input/tablet.rs:627](../../../src/input/tablet.rs#L627)

The app-facing tablet object; `AppState` owns exactly one. It owns the Wintab backend
(when there is one), this frame's packets, and the reopen / retry bookkeeping.

| Field | Type | Meaning |
|---|---|---|
| `backend` | `Option<windows_backend::Wintab>` | The open driver context, if any. |
| `backend` | `()` | The open driver context, if any. |
| `packets` | `Vec<PenPacket>` | This frame's packets, oldest first. Cleared by every `poll`. |
| `last_packet` | `Option<PenPacket>` | Newest packet ever drained. Survives an empty poll — needed because the frame that sees the press has often drained its packet a frame earlier. |
| `trail` | `Vec<PenPacket>` | Newest packets across polls, oldest first, ending at `last_packet` (at most `TRAIL_LEN`). |
| `last_pressure` | `f32` | Pressure of the newest packet, or 1.0 once idle. |
| `idle_frames` | `u32` | Frames (with nothing pressed) since the last packet. Starts at `u32::MAX` = "never seen". |
| `hwnd` | `Option<isize>` | The app window, once known; until then the window is found by title. |
| `retry` | `Retry` | Backoff for opening the context. |
| `watch` | `DisplayWatch` | Desktop change detection. |
| `reconnect` | `bool` | Reconnect was pressed; honoured on the next poll with the pen up. |
| `reopens` | `u32` | Reopen count (for diagnostics). |
| `last_reopen` | `Option<ReopenReason>` | Latest reopen reason. |
| `init_error` | `Option<String>` | Last open failure message. |

### `const windows_backend::SILENCE_WARN_POLLS`
[src/input/tablet.rs:930](../../../src/input/tablet.rs#L930)

```rust
const SILENCE_WARN_POLLS: u32 = 300;
```

`300` polls: if a context has been open this long and never delivered a packet, a
warning is logged (roughly a few seconds of drawing).

### `const windows_backend::SUBPIXEL`
[src/input/tablet.rs:935](../../../src/input/tablet.rs#L935)

```rust
const SUBPIXEL: i32 = 32;
```

`32`: how much finer than a screen pixel positions are asked for. Deliberately past the
tablet's own resolution, so the remaining coarseness is the hardware's, not the app's.

### `const windows_backend::MAX_PACKET_BYTES`
[src/input/tablet.rs:940](../../../src/input/tablet.rs#L940)

```rust
const MAX_PACKET_BYTES: usize = 256;
```

`256`: upper bound on one packet, for sizing the read buffer. The documented fields total
76 bytes; the rest is room for a driver that reports more than was asked.

### `const windows_backend::QUEUE_SIZE`
[src/input/tablet.rs:945](../../../src/input/tablet.rs#L945)

```rust
const QUEUE_SIZE: i32 = 128;
```

`128` packets of queue depth, matching Qt's `TabletPacketQSize`. The driver default is
often 8, which a 200 Hz pen overflows in 40 ms — well within one 60 fps frame.

### `type windows_backend::WtQueueSizeGet`
[src/input/tablet.rs:949](../../../src/input/tablet.rs#L949)

```rust
type WtQueueSizeGet<'a> = Symbol<'a, unsafe extern "C" fn(*mut HCTX) -> i32>;
```

Type aliases for the `WTQueueSizeGet(ctx) -> i32` and `WTQueueSizeSet(ctx, n) -> i32`
function pointers, looked up by name because `wintab_lite` has no binding for them.
`unsafe extern "C" fn(...)` is the type of a C function pointer.

### `type windows_backend::WtQueueSizeSet`
[src/input/tablet.rs:950](../../../src/input/tablet.rs#L950)

```rust
type WtQueueSizeSet<'a> = Symbol<'a, unsafe extern "C" fn(*mut HCTX, i32) -> i32>;
```

Type aliases for the `WTQueueSizeGet(ctx) -> i32` and `WTQueueSizeSet(ctx, n) -> i32`
function pointers, looked up by name because `wintab_lite` has no binding for them.
`unsafe extern "C" fn(...)` is the type of a C function pointer.

### `struct Wintab`
[src/input/tablet.rs:952](../../../src/input/tablet.rs#L952)

One open Wintab context plus everything needed to decode and map its packets. Closing
happens in its `Drop`.

| Field | Type | Meaning |
|---|---|---|
| `wt_close` | `WTClose<'static>` | `WTClose` function pointer (used by `drop`). |
| `wt_packets_get` | `WTPacketsGet` | `WTPacketsGet` function pointer (drains packets). |
| `ctx_handle` | `*mut HCTX` | Raw handle of the open context. |
| `hwnd` | `HWND` | The window the context is bound to. |
| `pressure_min` | `f32` | Lowest raw pressure value of the device. |
| `pressure_span` | `f32` | `max - min` raw pressure, for normalising to 0..1. |
| `layout` | `PacketLayout` | Field offsets and packet size from the granted mask. |
| `buf` | `Vec<u8>` | Reused scratch buffer for one poll's packets (`QUEUE_SIZE * MAX_PACKET_BYTES` bytes). |
| `last_raw` | `Option<(i32, i32)>` | Raw coordinates of the newest packet. |
| `total_packets` | `u64` | Packets since the context opened. |
| `max_packets_per_frame` | `usize` | Most packets in one `poll_once`. |
| `drained_full` | `u32` | `poll_once` calls that came back with a completely full buffer. |
| `map_x` | `(f32, f32, f32)` | Affine packet → virtual-desktop mapping, per axis: the desktop origin, the packet-space origin, and desktop units per packet unit. |
| `map_y` | `(f32, f32, f32)` | Vertical packet→screen mapping from the driver's reported area: `(origin, packet_origin, scale)`. |
| `out_span` | `(i32, i32)` | Size of the packet range the whole tablet reports over (granted output extent, made positive). |
| `sys_area` | `ScreenRect` | Screen area the context was opened with (`lcSys*`). |
| `tilt_support` | `bool` | Device reports azimuth / altitude. |
| `tangent_span` | `f32` | Range of the tangential-pressure axis; 0 without one. |
| `cursor_types` | `std::collections::HashMap<u32, u32>` | Cache of each cursor index's `CSR_TYPE` (only an airbrush reports tangential pressure). |
| `wt_info_raw` | `libloading::Symbol<'static, unsafe extern "C" fn(u32, u32, *mut std::ffi::c_void) -> u32>` | `WTInfoA` with a plain-number category, for the cursor categories that sit outside `wintab_lite`'s enum. |
| `client_origin` | `Option<(f32, f32)>` | Window client-area top-left on the desktop, refreshed every poll. |
| `fit` | `super::CursorFit` | The cursor-learned mapping, preferred per axis once trusted. |
| `queue_err_logged` | `bool` | A queue-overflow warning has been logged (only once). |
| `polls` | `u32` | `poll_once` calls since opening. |
| `seen_packets` | `bool` | Any packet has ever arrived (a context that opens but never delivers looks like no tablet at all, so it is reported). |

### `static windows_backend::LIB`
[src/input/tablet.rs:1009](../../../src/input/tablet.rs#L1009)

```rust
static LIB: OnceLock<Library> = OnceLock::new();
```

`OnceLock<Library>`: `Wintab32.dll`, loaded once for the life of the process. Every
function pointer borrows from it, which is what allows `Wintab` to store them as
`'static`; reopening a context must not load (and leak) another copy.

### `static windows_backend::MSG_BASE`
[src/input/tablet.rs:1022](../../../src/input/tablet.rs#L1022)

```rust
static MSG_BASE: AtomicU32 = AtomicU32::new(0x7FF0);
```

`AtomicU32`, initially `0x7FF0`: the window-message number base the open context was
given (`lcMsgBase`). Wintab's own messages are numbered from it.

### `static windows_backend::NUDGE`
[src/input/tablet.rs:1024](../../../src/input/tablet.rs#L1024)

```rust
static NUDGE: AtomicBool = AtomicBool::new(false);
```

`AtomicBool`: set by `on_msg` when the driver says something changed; taken (read and
cleared) by the next poll.

## `impl PacketLayout`

### `PacketLayout::from_mask`
[src/input/tablet.rs:104](../../../src/input/tablet.rs#L104)

```rust
pub fn from_mask(granted: u32) -> Self
```

Works out where each field sits inside a raw Wintab packet. The driver says which fields it granted
as a bit mask; fields are packed in a fixed order, so walking the mask and adding each granted
field's size gives every field's byte offset and the total packet size.

### `PacketLayout::usable`
[src/input/tablet.rs:132](../../../src/input/tablet.rs#L132)

```rust
pub fn usable(&self) -> bool
```

True if the granted fields are enough to place a stroke (a non-empty packet with X and Y).

## Free functions

### `read_i32`
[src/input/tablet.rs:138](../../../src/input/tablet.rs#L138)

```rust
fn read_i32(buf: &[u8], at: usize) -> i32
```

Reads a little-endian 32-bit signed integer from 4 bytes of `buf` at offset `at`. Packets arrive as
raw bytes, so fields are decoded by hand — like `new DataView(buf).getInt32(at, true)` in JS.

### `read_u32`
[src/input/tablet.rs:143](../../../src/input/tablet.rs#L143)

```rust
fn read_u32(buf: &[u8], at: usize) -> u32
```

The unsigned version of `read_i32` (`DataView.getUint32(at, true)`).

### `axis_map`
[src/input/tablet.rs:162](../../../src/input/tablet.rs#L162)

```rust
fn axis_map(base_org: i32, base_ext: i32, granted_org: i32, granted_ext: i32) -> (f32, f32, f32)
```

One axis of the packet → screen mapping, as `(origin, packet_origin, scale)`, applied as
`origin + (v − packet_origin) × scale`. It compares the area the driver defines for itself with the
finer one it granted, so the code works whether or not the request for sub-pixel precision was
honoured.

## `impl AxisFit`

### `AxisFit::map`
[src/input/tablet.rs:182](../../../src/input/tablet.rs#L182)

```rust
pub fn map(&self, raw: i32) -> f32
```

Applies a learned fit to one raw coordinate: `origin + raw × scale`.

## `impl CursorFit`

### `CursorFit::observe`
[src/input/tablet.rs:235](../../../src/input/tablet.rs#L235)

```rust
pub fn observe(&mut self, raw: (i32, i32), cursor: (i32, i32))
```

Records the newest packet of a poll together with the OS cursor position read right after it; enough
such pairs let `fit_axis` learn the exact packet→screen mapping (drivers sometimes report the wrong
area, e.g. on multi-monitor setups).

### `CursorFit::pause`
[src/input/tablet.rs:284](../../../src/input/tablet.rs#L284)

```rust
pub fn pause(&mut self)
```

Stops learning until the next `observe`. Called for every poll made with the pen down: a stroke uses
one mapping from start to end, and drawing is when the cursor lags the pen most.

### `CursorFit::pairs`
[src/input/tablet.rs:288](../../../src/input/tablet.rs#L288)

```rust
pub fn pairs(&self) -> usize
```

How many (packet, cursor) pairs have been collected.

## Free functions

### `fit_axis`
[src/input/tablet.rs:295](../../../src/input/tablet.rs#L295)

```rust
fn fit_axis(pairs: impl Iterator<Item = (i32, i32)> + Clone) -> Option<AxisFit>
```

Fits a straight line (least squares) through `(raw, cursor)` pairs for one axis, giving an `AxisFit`
— or `None` when there are too few points, they span too little, or they don't lie on a line.

**Example.** If raw X values 0, 1000, 2000 map to cursor X 100, 600, 1100, the fit is
`origin = 100, scale = 0.5`.

## `impl ScreenRect`

### `ScreenRect::normalized`
[src/input/tablet.rs:355](../../../src/input/tablet.rs#L355)

```rust
fn normalized(self) -> Self
```

The same rectangle with a non-negative width and height (Wintab rectangles can have negative
extents, meaning "runs the other way").

### `ScreenRect::overlaps`
[src/input/tablet.rs:362](../../../src/input/tablet.rs#L362)

```rust
fn overlaps(self, o: Self) -> bool
```

Whether two rectangles share any area (touching edges don't count).

## `impl ReopenReason`

### `ReopenReason::label`
[src/input/tablet.rs:398](../../../src/input/tablet.rs#L398)

```rust
pub fn label(self) -> &'static str
```

A short text for why the tablet context was reopened ("monitors changed", …), shown in the
diagnostics.

## `impl DisplayWatch`

### `DisplayWatch::opened`
[src/input/tablet.rs:425](../../../src/input/tablet.rs#L425)

```rust
pub fn opened(&mut self, sig: DisplaySig, now: std::time::Instant)
```

Records the display layout a freshly opened tablet context was built against, to detect changes
later.

### `DisplayWatch::tick`
[src/input/tablet.rs:436](../../../src/input/tablet.rs#L436)

```rust
pub fn tick(
    &mut self,
    now: std::time::Instant,
    read: impl FnOnce() -> DisplaySig,
    pointer_down: bool,
    nudged: bool,
) -> Option<ReopenReason>
```

Decides whether the tablet context should be reopened now because monitors or the driver's area
changed. The (slow) `read` closure is only called when a check is due, or immediately when the
driver posted a change message (`nudged`); reopening waits while the pen is down.

## `impl Retry`

### `Retry::due`
[src/input/tablet.rs:496](../../../src/input/tablet.rs#L496)

```rust
pub fn due(&self, now: std::time::Instant) -> bool
```

Whether it's time to try opening the tablet again: not parked, and the next attempt time has passed
(or none is set).

### `Retry::failed`
[src/input/tablet.rs:500](../../../src/input/tablet.rs#L500)

```rust
pub fn failed(&mut self, kind: InitFail, now: std::time::Instant)
```

Records a failed attempt to open the tablet. No window yet → retry soon; no driver → stop trying
("parked"); a context error → wait with exponential back-off (`RETRY_FIRST × 2^(failures−1)`, capped
at `RETRY_MAX`).

### `Retry::wait`
[src/input/tablet.rs:513](../../../src/input/tablet.rs#L513)

```rust
pub fn wait(&self, now: std::time::Instant) -> Option<std::time::Duration>
```

How long until the next attempt, or `None` if it is due now or parked.

### `Retry::parked`
[src/input/tablet.rs:520](../../../src/input/tablet.rs#L520)

```rust
pub fn parked(&self) -> bool
```

Whether retrying has stopped (no tablet driver installed).

## Free functions

### `monitors_spanned`
[src/input/tablet.rs:527](../../../src/input/tablet.rs#L527)

```rust
pub fn monitors_spanned(area: ScreenRect, monitors: &[(ScreenRect, bool)]) -> usize
```

How many monitors the tablet's screen area reaches onto. More than one means the tablet is spread
over several screens, so each gets only part of it.

### `pen_reach`
[src/input/tablet.rs:540](../../../src/input/tablet.rs#L540)

```rust
pub fn pen_reach(span: (i32, i32), fit_x: Option<AxisFit>, fit_y: Option<AxisFit>) -> Option<(f32, f32)>
```

How much screen (in pixels) the whole tablet surface covers, measured from the cursor fit rather than
the driver's report (which can be wrong). `None` until both axes are trusted. The `?` on each fit
returns `None` early.

### `reach_fits_one_monitor`
[src/input/tablet.rs:553](../../../src/input/tablet.rs#L553)

```rust
pub fn reach_fits_one_monitor(reach: (f32, f32), monitors: &[(ScreenRect, bool)]) -> bool
```

Whether a tablet covering `reach` pixels fits on any one monitor. If not, the driver maps the tablet
over several monitors (a common "pen only reaches half the screen" confusion).

## `impl PenInput`

### `PenInput::new`
[src/input/tablet.rs:656](../../../src/input/tablet.rs#L656)

```rust
pub fn new() -> Self
```

Creates the pen input in its "nothing connected yet" state. The `#[cfg(...)]` attributes on the
`backend` field pick a real backend slot on Windows and an empty `()` placeholder elsewhere — one
struct definition, two platform versions.

### `PenInput::set_hwnd`
[src/input/tablet.rs:680](../../../src/input/tablet.rs#L680)

```rust
pub fn set_hwnd(&mut self, hwnd: isize)
```

Gives the pen input the app window's handle, so the tablet context binds to this window (not just the
first window titled "Animator").

### `PenInput::has_hwnd`
[src/input/tablet.rs:684](../../../src/input/tablet.rs#L684)

```rust
pub fn has_hwnd(&self) -> bool
```

Whether the window handle has been given yet.

### `PenInput::reconnect`
[src/input/tablet.rs:690](../../../src/input/tablet.rs#L690)

```rust
pub fn reconnect(&mut self)
```

Asks for the tablet context to be closed and reopened on the next pen-up poll (the "Reconnect" button).

### `PenInput::poll`
[src/input/tablet.rs:702](../../../src/input/tablet.rs#L702)

```rust
pub fn poll(&mut self, pointer_down: bool)
```

Called once per frame from `AppState::update`. Opens the tablet context lazily (once the window
exists), reopens it if the desktop or driver area changed, and drains the whole packet queue into
`packets`. While a pointer is down it freezes the idle counter (a pen held still mid-stroke sends no
packets, and treating that as "pen gone" would make the mouse path stamp a full-pressure blob).

### `PenInput::track_idle`
[src/input/tablet.rs:761](../../../src/input/tablet.rs#L761)

```rust
fn track_idle(&mut self, pointer_down: bool)
```

Updates the idle counter after a poll: reset when packets arrived, counted up otherwise — but only
while nothing is pressed (see `poll`).

### `PenInput::packets`
[src/input/tablet.rs:781](../../../src/input/tablet.rs#L781)

```rust
pub fn packets(&self) -> &[PenPacket]
```

This frame's tablet packets, oldest first.

### `PenInput::last_packet`
[src/input/tablet.rs:787](../../../src/input/tablet.rs#L787)

```rust
pub fn last_packet(&self) -> Option<PenPacket>
```

The most recent packet ever received, or `None` before the pen has reported.

### `PenInput::trail`
[src/input/tablet.rs:794](../../../src/input/tablet.rs#L794)

```rust
pub fn trail(&self) -> &[PenPacket]
```

The last few dozen packets across frames, oldest first — used to check fast flicks against the
cursor.

### `PenInput::diagnostics`
[src/input/tablet.rs:799](../../../src/input/tablet.rs#L799)

```rust
pub fn diagnostics(&self) -> PenDiagnostics
```

A snapshot of the tablet's state (mapping, fits, monitors, errors, counts) for the Tablet readout in
Settings.

### `PenInput::pen_active`
[src/input/tablet.rs:847](../../../src/input/tablet.rs#L847)

```rust
pub fn pen_active(&self) -> bool
```

True while the pen is the live input device: a backend exists and packets arrived recently. False for
mouse input, so callers know to ignore pressure and packet positions.

### `PenInput::recently_seen`
[src/input/tablet.rs:852](../../../src/input/tablet.rs#L852)

```rust
fn recently_seen(&self) -> bool
```

The platform-independent half of `pen_active`: packets arrived within the last `PEN_IDLE_FRAMES`
frames.

### `PenInput::current_pressure`
[src/input/tablet.rs:857](../../../src/input/tablet.rs#L857)

```rust
pub fn current_pressure(&self) -> Option<f32>
```

The latest pen pressure (0–1) while the pen is active, else `None`.

### `PenInput::client_origin`
[src/input/tablet.rs:868](../../../src/input/tablet.rs#L868)

```rust
pub fn client_origin(&self) -> Option<(f32, f32)>
```

The top-left of the window's drawing area in desktop pixels (to turn packet positions into window
coordinates); `None` without a backend. Two `#[cfg]` blocks give the Windows and non-Windows bodies.

### `PenInput::is_active`
[src/input/tablet.rs:881](../../../src/input/tablet.rs#L881)

```rust
pub fn is_active(&self) -> bool
```

Whether a tablet backend exists at all (drives the pen/mouse status label).

## `mod windows_backend`

### `windows_backend::map_twist`
[src/input/tablet.rs:920](../../../src/input/tablet.rs#L920)

```rust
pub(super) fn map_twist(twist: i32) -> f32
```

Converts Wintab's barrel twist (tenths of a degree, clockwise) to Krita's convention:
`360 − twist/10`, folded into −180…180.

### `windows_backend::library`
[src/input/tablet.rs:1011](../../../src/input/tablet.rs#L1011)

```rust
fn library() -> Result<&'static Library>
```

Loads `Wintab32.dll` (the tablet driver's library) the first time it's needed and keeps it in a
`OnceLock` global so later calls reuse it. Loading a DLL is `unsafe` because Rust can't check the
foreign code. If the DLL isn't there, there is no tablet driver — an error, not a crash.

### `windows_backend::on_msg`
[src/input/tablet.rs:1031](../../../src/input/tablet.rs#L1031)

```rust
pub fn on_msg(msg: &MSG)
```

Called for every window message the event loop reads (installed by
[input/msg_hook.rs](msg_hook.md)). If it is one of Wintab's "context updated" / "info changed"
messages, sets an atomic flag (`NUDGE`) so the next poll checks the layout immediately.

### `windows_backend::take_nudge`
[src/input/tablet.rs:1038](../../../src/input/tablet.rs#L1038)

```rust
pub fn take_nudge() -> bool
```

Reads and clears the "driver changed something" flag in one atomic step (`swap(false)`).

### `windows_backend::window_by_title`
[src/input/tablet.rs:1044](../../../src/input/tablet.rs#L1044)

```rust
pub fn window_by_title() -> Option<isize>
```

Finds the window titled "Animator" through the Win32 `FindWindowA` call — the fallback way to bind
the tablet before the app hands over its own handle. `c"Animator"` is a C string literal
(null-terminated) for the C API.

### `windows_backend::display_sig`
[src/input/tablet.rs:1051](../../../src/input/tablet.rs#L1051)

```rust
pub fn display_sig(w: Option<&Wintab>) -> super::DisplaySig
```

A "signature" of the current display setup — every monitor's rectangle (via `EnumDisplayMonitors`)
plus the driver's area when a context exists — so changes can be detected by comparing signatures.

#### `windows_backend::display_sig::each`
[src/input/tablet.rs:1059](../../../src/input/tablet.rs#L1059)

```rust
unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL
```

The callback `EnumDisplayMonitors` calls once per monitor. It is an `unsafe extern "system" fn` — a
function with the Windows calling convention, callable from C. The `out` parameter smuggles a pointer
to a Rust `Vec`, which it casts back (`&mut *(out.0 as *mut Vec<…>)`) and pushes the monitor's
rectangle and "is primary" flag onto. Returning `true` asks Windows to continue enumerating.

## `impl Wintab`

### `Wintab::try_init`
[src/input/tablet.rs:1113](../../../src/input/tablet.rs#L1113)

```rust
pub fn try_init(hwnd: Option<isize>) -> std::result::Result<Self, (InitFail, String)>
```

Tries to open a tablet context on the given window (or the one found by title). On failure it returns
which kind of failure it was (no window, no driver, context error), which decides when to retry.

### `Wintab::open`
[src/input/tablet.rs:1125](../../../src/input/tablet.rs#L1125)

```rust
fn open(hwnd: HWND, lib: &'static Library) -> Result<Self>
```

Opens the Wintab context. Long because of the driver details it has to get right:
1. Looks up the Wintab functions in the DLL (`WTInfoA`, `WTOpenA`, `WTPacketsGet`, …).
2. Reads the driver's default system context and its output area, kept as reported (changing it moved
   the cursor off the work area on some drivers).
3. Sets the output area to screen coordinates with a **negative Y extent**, because tablets count Y
   upward and screens downward (the same trick Qt uses) — without it strokes appear mirrored.
4. Asks only for the packet fields it reads (position, pressure, orientation, time, cursor, twist,
   tangential pressure), so the packet size is predictable, and builds the `PacketLayout` from what
   was granted.
5. Requests a finer coordinate scale so positions arrive with sub-pixel precision, and builds the
   axis mappings (`axis_map`).

### `Wintab::map_point`
[src/input/tablet.rs:1395](../../../src/input/tablet.rs#L1395)

```rust
pub fn map_point(&self, x: i32, y: i32) -> (f32, f32)
```

Converts a packet coordinate to desktop pixels: through the learned cursor fit once trusted,
otherwise through the area the driver reported. `map_or(default, f)` picks between the two per axis.

### `Wintab::is_airbrush`
[src/input/tablet.rs:1406](../../../src/input/tablet.rs#L1406)

```rust
fn is_airbrush(&mut self, index: u32) -> bool
```

Whether cursor (tool) `index` is an airbrush pen, by testing Wintab's cursor-type bits the way Qt
does (cached per cursor).

### `Wintab::map_tilt`
[src/input/tablet.rs:1425](../../../src/input/tablet.rs#L1425)

```rust
fn map_tilt(&self, azimuth: i32, altitude: i32) -> (f32, f32)
```

Converts the pen's azimuth/altitude angles (tenths of a degree) into X/Y tilt in degrees, using the
same trigonometry as Qt: `X = sin(az)·cos(alt)`, `Z = sin(alt)`, `xTilt = atan(X/Z)`.

### `Wintab::poll`
[src/input/tablet.rs:1449](../../../src/input/tablet.rs#L1449)

```rust
pub fn poll(&mut self, out: &mut Vec<PenPacket>, pointer_down: bool)
```

Drains the whole packet queue into `out`, oldest first, calling `poll_once` repeatedly until the queue
is empty (one call returns at most a bufferful — without looping, a backed-up queue would keep
returning older and older positions). Also refreshes the cached window origin and feeds the cursor
fit when the pen is up.

### `Wintab::poll_once`
[src/input/tablet.rs:1474](../../../src/input/tablet.rs#L1474)

```rust
fn poll_once(&mut self, out: &mut Vec<PenPacket>)
```

Reads one bufferful of packets with `WTPacketsGet` and decodes each from raw bytes using the
`PacketLayout` offsets: position (through `map_point`), pressure normalised to 0–1 (full strength if
the device reports none), tilt, timestamp, twist and airbrush wheel. Packets with the queue-error
flag are skipped; stray far-away coordinates are caught later by a distance check in
[ui/shell.rs](../ui/shell.md).

## `mod windows_backend`

### `windows_backend::client_origin`
[src/input/tablet.rs:1582](../../../src/input/tablet.rs#L1582)

```rust
fn client_origin(hwnd: HWND) -> Option<(f32, f32)>
```

The top-left of window `hwnd`'s client area in desktop pixels (Win32 `ClientToScreen`).

## `impl Drop for Wintab`

### `Wintab::drop`
[src/input/tablet.rs:1598](../../../src/input/tablet.rs#L1598)

```rust
fn drop(&mut self)
```

Rust's destructor (`Drop` trait): runs automatically when the `Wintab` value goes away and closes the
tablet context, like a `finally` that can't be forgotten.

## Tests

### `mod tests`

Run with `cargo test input::tablet::tests` · 27 tests

| Test | What it checks |
|---|---|
| [`an_ignored_scale_request_maps_one_to_one`](../../../src/input/tablet.rs#L1619) | A driver that ignores the finer-scale request leaves packets in its own output space, and the mapping has to be the identity. |
| [`a_granted_scale_request_divides_back_down`](../../../src/input/tablet.rs#L1628) | A granted request divides back down, and lands between whole pixels. |
| [`a_flipped_axis_stays_flipped`](../../../src/input/tablet.rs#L1645) | The regression this function exists for.  A tablet's Y axis points up, so the driver's output extent for it is negative. Rebuilding the map from the screen rectangle instead — which is positive — mirrored every position about the middle of the screen. Near that line the mirrored point is close enough to the real one to pass a sanity check and be drawn, which is why the artefact appeared as a band across the middle of the window rather than as everything being upside down. |
| [`the_full_mask_matches_the_reference_packet_struct`](../../../src/input/tablet.rs#L1662) | The field table has to agree with the layout `wintab_lite`'s own `Packet` struct describes, or the offsets computed from it are fiction. Checking the full mask against that struct's size is what pins the two together. |
| [`a_missing_trailing_field_only_shortens_the_packet`](../../../src/input/tablet.rs#L1676) | The case that was breaking: a pen with no rotation. Every field before the missing one keeps its offset, and the packet is shorter — so a fixed-stride read finds the first packet and loses every one after it. |
| [`a_missing_middle_field_shifts_what_follows`](../../../src/input/tablet.rs#L1689) | A field dropped from the middle shifts everything after it, which is the case a size check alone would not catch. |
| [`time_cursor_and_tangent_have_their_offsets`](../../../src/input/tablet.rs#L1703) | The fields asked for now: time, cursor and tangential pressure land at their places in the bit order, around the old ones. |
| [`twist_reads_like_krita`](../../../src/input/tablet.rs#L1723) | Twist reads like krita. |
| [`position_alone_is_usable_and_nothing_is_not`](../../../src/input/tablet.rs#L1733) | The minimum a tablet has to give us to be worth listening to. |
| [`a_flipped_axis_scaled_up_lands_on_screen_coordinates`](../../../src/input/tablet.rs#L1748) | The live configuration: a Y axis that must come out top-down, on an output area scaled up for sub-pixel resolution. Both at once, because each was got wrong in the presence of the other. |
| [`a_refused_flip_is_corrected_rather_than_believed`](../../../src/input/tablet.rs#L1764) | A driver that ignores both the scale and the flip hands back its own bottom-up, screen-scaled area. The map has to correct both, or every position comes out mirrored about the middle of the screen — which is the failure this whole arrangement exists to make impossible. |
| [`a_pen_paused_mid_stroke_keeps_its_pressure`](../../../src/input/tablet.rs#L1788) | The blob regression. A pen held still mid-stroke sends no packets while the stroke keeps repainting; it must stay the live device, at its own pressure, however long the pause. |
| [`the_trail_keeps_the_newest_packets_across_polls`](../../../src/input/tablet.rs#L1803) | The trail runs across polls, oldest first, ends at the newest packet and keeps only the last `TRAIL_LEN`. |
| [`a_driver_that_misreports_its_area_is_fitted_from_the_cursor`](../../../src/input/tablet.rs#L1855) | The driver seen in the field: accepted a Y-flipped output area, reported it back flipped, and delivered top-down coordinates offset by the flipped origin anyway. The fit must land on the cursor to a fraction of a pixel regardless. |
| [`a_flipped_axis_at_any_scale_is_fitted`](../../../src/input/tablet.rs#L1866) | And one that really does flip, at a scale nobody asked for. |
| [`too_little_travel_is_not_trusted`](../../../src/input/tablet.rs#L1884) | Until the pen has moved enough to pin a slope down, there is no fit, and the driver's own mapping stays in charge. |
| [`a_still_pen_adds_one_pair`](../../../src/input/tablet.rs#L1895) | A pen resting in place must not flood the window with one point. |
| [`a_changed_mapping_is_relearned`](../../../src/input/tablet.rs#L1906) | Rearrange the monitors and the old fit is simply wrong. It must be dropped and relearned, not averaged into the new one. |
| [`a_lagging_cursor_neither_bends_nor_drops_the_fit`](../../../src/input/tablet.rs#L1922) | The regression from the field: a stroke fast enough that the cursor trails the pen used to push the fit out of trust mid-stroke, and the stroke fell back to the cursor. A run of lagging pairs must neither move a trusted fit nor lose it. |
| [`the_last_packet_survives_an_empty_poll`](../../../src/input/tablet.rs#L1945) | A stroke's first sample comes from the newest packet, and the press is often seen a frame after that packet was drained. An empty poll must not forget it. |
| [`an_idle_pen_hands_over_to_the_mouse`](../../../src/input/tablet.rs#L1957) | And once lifted and left alone, the mouse takes over at full pressure. |
| [`an_offset_output_origin_is_carried_through`](../../../src/input/tablet.rs#L1972) | A driver free to grant a different origin as well as a different extent must still land correctly. |
| [`the_watch_reopens_only_on_a_real_change`](../../../src/input/tablet.rs#L1996) | The watch reopens only on a real change. |
| [`a_change_during_a_stroke_waits_for_pen_up`](../../../src/input/tablet.rs#L2029) | A change during a stroke waits for pen up. |
| [`failed_inits_back_off_and_a_missing_driver_waits_for_reconnect`](../../../src/input/tablet.rs#L2046) | Failed inits back off and a missing driver waits for reconnect. |
| [`your_desktop_spans_two_monitors`](../../../src/input/tablet.rs#L2070) | Your desktop spans two monitors. |
| [`the_cursor_measures_what_the_pen_reaches`](../../../src/input/tablet.rs#L2085) | The user's readout after mapping the tablet to the main monitor: the driver still says 4159×1439, but the cursor puts the tablet's width on 2558 pixels — one monitor. |

**Test helpers** (used only by the tests above):

- [`apply`](../../../src/input/tablet.rs#L1612) — Apply what `axis_map` returns, the way `Wintab::map_point` does.
- [`packet`](../../../src/input/tablet.rs#L1771) — Packet.
- [`hover_path`](../../../src/input/tablet.rs#L1823) — A pen hovering around the screen, as `(true x, true y)` in screen pixels, a few pixels per step.
- [`learn`](../../../src/input/tablet.rs#L1834) — Feed `path` through a driver whose packets are `raw(true position)`, with the cursor on the whole pixel nearest the truth.
- [`worst_error`](../../../src/input/tablet.rs#L1840) — Worst error.
- [`rect`](../../../src/input/tablet.rs#L1978) — Rect.
- [`reported`](../../../src/input/tablet.rs#L1984) — The reported setup: a 2560×1440 primary with a 1600×900 display to its right, and a Deco Pro M (16:9) mapped over both.
