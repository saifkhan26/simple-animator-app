# `src/krita_link.rs` — the two-way live link with Krita

> **Source:** [src/krita_link.rs](../../src/krita_link.rs) · **Module:** `crate::krita_link` · **Size:** 1879 lines · Line links as of commit `36405ad`

## What this file is for

This file is the brain of **File ▸ Edit in Krita**. Krita is a free painting program. The
link lets you send the whole animation (or one layer) to Krita, paint there, press Save in
Krita, and see the changes appear back in Animator — without closing either app.

It works through an ordinary file on disk:

1. **Send.** Animator writes the project as an animated `.kra` (Krita's file format) into a
   temp folder named `animator-krita` and starts Krita on it. While writing, it records a
   **baseline**: a snapshot of exactly what Krita now holds (each layer's name, opacity,
   keys, and a content hash of every drawing).
2. **Watch.** Every frame the app checks the file's modification time and size. When Krita
   saves, the change must "settle" (stay still for half a second) before it is read.
3. **Pull.** A worker thread reads the `.kra` and *matches* every Krita drawing against the
   baseline by content hash. Then the UI thread makes a **plan** — the minimal set of changes
   Krita actually made — and applies it as one undoable edit.

The clever part is the three-way comparison (Animator now, Krita now, baseline = what both
agreed on last time). It is like a `git merge` with the baseline as the common ancestor:

* **Drawings** Krita did not touch map back to the local cell they came from, so edits you
  made *in Animator* in the meantime survive. A drawing Krita changed becomes a **new** cell
  ("Krita wins" when both apps changed the same drawing). Existing cells are never modified,
  which is what makes the cheap timeline-snapshot undo correct.
* **Timing** (which drawing shows on which frame) is rebuilt only if Krita's key list
  differs from the baseline.
* **Layer attributes** (name, opacity, visibility, lock) are copied field by field, only
  where Krita's value moved.
* **Layers** added / deleted / reordered in Krita are added / deleted / reordered here.
  Layers you added in Animator since the send stay put above the linked layer they sat on.

Some layers never travel: **reference** (light-table) layers and any layer whose name ends
in `-x` stay in the app they are in (see [`stays_put`](#stays_put)). Things
Krita cannot hold — layer transforms, the camera, tracker points — are never sent, so they
are never disturbed. The link lives only for the session; nothing about it is saved in the
`.anim` file.

The second half of the file (from line 929) is the **session** machinery: the temp file
path, starting Krita, polling the file, and the state of a "Send to Krita again" round
that talks to the optional Krita helper plugin (see [krita_helper.rs](krita_helper.md)).

Words used here: a **cell** is one drawing (one image); a **key** (exposure) is a frame where
a layer switches to a cell; the layer then *holds* that cell until the next key. See the
[glossary](../glossary.md).

## How it connects

- **Used by:**
  - [app.rs](app.md) — owns the `KritaLink` (`AppState::krita`), and drives everything:
    `edit_in_krita` / `begin_send` / `start_send` call [`sendable`](#sendable),
    [`new_uuid`](#new_uuid) and [`send`](#send) on a worker;
    `poll_krita` calls [`KritaLink::poll`](#kritalinkpoll), [`KritaLink::start_pull`](#kritalinkstart_pull)
    and [`KritaLink::launch`](#kritalinklaunch); `apply_krita_pull` calls [`plan`](#plan),
    [`PullPlan::apply`](#pullplanapply) and [`Report::summary`](#reportsummary); `import_kra`
    uses [`comes_back`](#comes_back) as its layer filter; Export .kra uses
    `new_uuid`.
  - [ui/shell.rs](ui/shell.md) — the layer panel calls [`stays_here_at`](#stays_here_at)
    to mark layers that never go to Krita.
  - [krita_stress.rs](krita_stress.md) — the randomized stress test drives `send`, `prepare`,
    `plan` and `apply` exactly like the app.
  - [brush_library/import.rs](brush_library/import.md) — an ignored survey test calls
    [`find_krita`](#find_krita).
- **Uses:**
  - [io/kra/mod.rs](io/kra/mod.md) — the `.kra` reader and writer (`kra::write`,
    `kra::read_filtered`, `kra::carry`, `cell_origin`, `opacity_u8`, `KraDoc`, `WriteOpts`).
  - [doc/project.rs](doc/project.md), [doc/layer.rs](doc/layer.md),
    [doc/canvas.rs](doc/canvas.md) — the document model (`Project`, `Layer`, `CellId`, `Canvas`).
  - [doc/groups.rs](doc/groups.md) — `group_of`, `normalize_groups`.
  - [doc/clip.rs](doc/clip.md) — `bake_clipping`, because Krita has no clipping masks like ours.
  - [krita_helper.rs](krita_helper.md) is *not* called from here; [app.rs](app.md) combines
    the two. This file only defines the `Round` / `Stage` state the app moves through.

```text
 Animator (UI thread)                worker threads                 Krita (separate process)
 ───────────────────                 ──────────────                 ───────────────────────
 edit_in_krita ─► start_send ──────► send(): bake clipping,
                                     write <temp>/animator-krita/
                                     NAME-PID.kra, Baseline::capture
 poll_krita ◄── Sent{baseline} ◄──── (channel)
   └─ KritaLink::launch ──────────────────────────────────────────► krita.exe NAME-PID.kra
                                                                    … user paints, Ctrl+S …
 poll_krita ─► KritaLink::poll (stat file, wait SETTLE)
   └─ start_pull ──────────────────► pull(): read_filtered + prepare()
 poll_krita ◄── Pulled ◄──────────── (channel)
   └─ apply_krita_pull ─► plan() ─► PullPlan::apply (one undo step)
```

## Rust concepts in this file

**`Option<T>` and the `?` operator.** A function that returns `Option` can use `?` to bail out
with `None` as soon as any step is missing — like optional chaining that also returns early.
```rust
pub fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;   // file missing → return None
    Some((m.modified().ok()?, m.len()))
}
```
```js
function stamp(path) {
  const m = tryStat(path); if (m == null) return null;
  return m.mtime == null ? null : [m.mtime, m.size];
}
```
More: [Option and Result](../rust-primer.md#option-and-result)

**`Result` with `anyhow`: `bail!` and `.context(...)`.** `Result<T>` is "a value or an error".
`bail!(...)` returns an error right away (like `throw new Error(...)`), and `.context("...")`
wraps an error with a human-readable reason, the way you might re-throw with a better message.
```rust
std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
```
More: [Option and Result](../rust-primer.md#option-and-result)

**Worker threads and channels.** Reading a big `.kra` takes time, so it runs on a thread. The
result comes back through an `mpsc` channel: `tx` sends, `rx` receives. The UI checks `rx`
each frame without blocking. This is the same idea as a Web Worker plus `postMessage`.
```rust
let (tx, rx) = std::sync::mpsc::channel();
std::thread::spawn(move || {
    let _ = tx.send(pull(&path, &base, &present));
});
self.pending = Some((at, rx));
```
`move ||` means the closure takes ownership of `path`, `base` and `present`, so the thread
has its own copies. More: [Threads and channels](../rust-primer.md#threads-and-channels)

**Struct update syntax `..other`.** Build a new struct by listing the fields that change and
copying the rest from another value — like `{ ...b, name: pl.name }` in JS.
```rust
LayerBase { name: pl.name.clone(), opacity: pl.opacity, ..b.clone() }
```
More: [Structs](../rust-primer.md#structs)

**Closures passed as parameters (`impl Fn`).** `LayerEdit::apply` takes a function that turns
a `CellRef` into a real cell id. `impl Fn(CellRef) -> CellId` means "any callable with that
shape".
```rust
fn apply(self, l: &mut Layer, fc: usize, resolve: impl Fn(CellRef) -> CellId) { … }
```
```js
function apply(edit, layer, fc, resolve /* (ref) => cellId */) { … }
```
More: [Closures](../rust-primer.md#closures)

**Conditional compilation (`#[cfg(...)]`, `cfg!`).** Code can exist only on some platforms or
only in tests. `#[cfg(windows)]` removes the loop below from macOS/Linux builds entirely;
`cfg!(windows)` is an ordinary `true`/`false` expression.
```rust
#[cfg(windows)]
for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)", "LOCALAPPDATA"] { … }
let exe = if cfg!(windows) { "krita.exe" } else { "krita" };
```
More: [Attributes and cfg](../rust-primer.md#attributes-and-cfg)

## impl Baseline

## impl PulledLayer

## impl LayerEdit

## impl Report

## impl PullPlan

## impl KritaLink

### Session constants
[src/krita_link.rs:934](../../src/krita_link.rs#L934)

| Const | Value | Meaning |
|---|---|---|
| `SETTLE` (line 934) | 500 ms | Minimum quiet time after the file changes before it is read, so a save still being written isn't read half-done. |
| `POLL` (line 936) | 1 s | How often the file is checked (`stat`). |
| `READY_TIMEOUT` (line 939) | 4 s | How long the helper plugin gets to say "ready" (it ticks every 400 ms and may have a save to make first). |
| `RELOAD_TIMEOUT` (line 942) | 30 s | How long it gets to reload — reopening a big animated document takes a while. |

## Types

### `const EVERYTHING_DELETED`
[src/krita_link.rs:60](../../src/krita_link.rs#L60)

```rust
pub const EVERYTHING_DELETED: &str = "Krita's save deletes every layer here";
```

`&str` = `"Krita's save deletes every layer here"`. The start of the error message
[`plan`](#plan) returns when a pull would leave the project with no layers at
all. The stress test checks error text starts with it.

### `struct LayerBase`
[src/krita_link.rs:118](../../src/krita_link.rs#L118)

What Krita holds for **one layer**, as of the last sync. Derives `Clone`.

| Field | Type | Meaning |
|---|---|---|
| `uuid` | `String` | The layer's Krita id, like `{1b2c…}`. This is how layers are matched across the round trip. |
| `name` | `String` | Layer name Krita had. |
| `opacity` | `u8` | Opacity 0–255 (Krita's scale). |
| `visible` | `bool` | Visibility flag. |
| `locked` | `bool` | Lock flag. |
| `keys` | `Vec<(usize, u64)>` | One `(frame, content hash)` per key. Leading blank keys are dropped, because Krita needs a blank key where Animator simply has nothing yet. |
| `sizes` | `Vec<(u32, u32)>` | Drawing size per key, same order as `keys`. |
| `by_hash` | `HashMap<u64, CellId>` | Krita-side content hash → the local cell that stands for it. The heart of the matching. |
| `cell_size` | `(u32, u32)` | Size to use for a drawing Krita adds when there is no neighbouring key to copy a size from. |
| `in_group` | `bool` | The layer is inside a group. |
| `collapsed` | `bool` | (For a group) it is folded in the layer list. |

### `struct Baseline`
[src/krita_link.rs:140](../../src/krita_link.rs#L140)

What Krita holds as of the last sync — the send, or the last pull. The "common ancestor" of
the three-way merge. Derives `Clone`.

| Field | Type | Meaning |
|---|---|---|
| `pw` | `u32` | Frame size Krita's canvas was made from. |
| `ph` | `u32` | Project height at the send (see `pw`). |
| `range_end` | `usize` | Last frame of Krita's animation range. |
| `layers` | `Vec<LayerBase>` | One per Krita layer, **bottom first** (Krita's stack order). |
| `ignored` | `HashSet<String>` | Krita uuids whose link was ended from this side (the layer here became reference or `-x`). They are never brought back, until the next send rewrites Krita's file without them. |

### `struct PulledDrawing`
[src/krita_link.rs:205](../../src/krita_link.rs#L205)

One Krita drawing after the worker has rasterized it into cell space and looked it up in the
baseline.

| Field | Type | Meaning |
|---|---|---|
| `hash` | `u64` | Content hash ([`cell_hash`](#cell_hash)). |
| `size` | `(u32, u32)` | Cell size it was placed at. |
| `matched` | `Option<CellId>` | The local cell whose Krita-side content this is. `None` = Krita changed or added it. |
| `canvas` | `Option<Canvas>` | The pixels — kept only when unmatched (or when the whole layer must come back). |
| `blank` | `bool` | Fully transparent. |

### `struct PulledLayer`
[src/krita_link.rs:216](../../src/krita_link.rs#L216)

One Krita layer, decoded and matched.

| Field | Type | Meaning |
|---|---|---|
| `uuid` | `String` | Krita layer id (a fresh one is made if Krita gave none). |
| `name` | `String` | The layer's name in Krita. |
| `opacity` | `u8` | Its Krita opacity, 0–255. |
| `visible` | `bool` | Whether it is visible in Krita. |
| `locked` | `bool` | Whether it is locked in Krita. |
| `drawings` | `Vec<PulledDrawing>` | Every distinct drawing in the layer. |
| `keys` | `Vec<(usize, usize)>` | `(frame, index into drawings)`, leading blank keys dropped. |
| `group` | `bool` | This Krita layer is a group. |
| `in_group` | `bool` | Inside a group. |
| `collapsed` | `bool` | Folded group. |

### `struct Pulled`
[src/krita_link.rs:238](../../src/krita_link.rs#L238)

A Krita save, decoded and matched against the baseline — the heavy part of a pull, done off
the UI thread by [`prepare`](#prepare).

| Field | Type | Meaning |
|---|---|---|
| `layers` | `Vec<PulledLayer>` | Krita's layers, bottom first. |
| `range_end` | `Option<usize>` | Krita's last animation frame, if the file says. |
| `warnings` | `Vec<String>` | Messages for the user (size mismatch, clipped paint, reader warnings). |
| `held` | `Vec<(String, String)>` | `(uuid, name)` of Krita layers that stay in Krita (named `-x`, see [`comes_back`](#comes_back)). |

### `enum CellRef`
[src/krita_link.rs:353](../../src/krita_link.rs#L353)

Which cell a planned key will point at. Derives `Clone, Copy, Debug, PartialEq, Eq`.

| Variant | Meaning |
|---|---|
| `Old(CellId)` | An existing cell of the project. |
| `New(usize)` | Index into the plan's `new_cells`; becomes a real id at apply time. |

### `struct LayerEdit`
[src/krita_link.rs:360](../../src/krita_link.rs#L360)

Field-by-field changes to one layer. Every field is an `Option`; `None` means "leave alone".
Derives `Default` (all `None`).

| Field | Type | Meaning |
|---|---|---|
| `name` | `Option<String>` | New name. |
| `opacity` | `Option<u8>` | New opacity 0–255. |
| `visible` | `Option<bool>` | New visibility, or `None` to leave it. |
| `locked` | `Option<bool>` | New lock state, or `None`. |
| `keys` | `Option<Vec<(usize, CellRef)>>` | A complete new key list (frame, cell). |
| `group` | `Option<bool>` | Only for a layer Krita added: whether it is a group. |
| `in_group` | `Option<bool>` | New group membership, or `None`. |
| `collapsed` | `Option<bool>` | New folded state for a group, or `None`. |

### `enum Entry`
[src/krita_link.rs:417](../../src/krita_link.rs#L417)

One row of the planned layer stack.

| Variant | Meaning |
|---|---|
| `Local { index: usize, edit: Option<LayerEdit> }` | An existing layer (its index in the project now), with an optional `LayerEdit`. |
| `New { uuid: String, edit: LayerEdit }` | A layer Krita added (or one deleted here that Krita then changed): created at apply time and linked to `uuid`. |

### `struct Report`
[src/krita_link.rs:426](../../src/krita_link.rs#L426)

What a pull did, for the toast message. Derives `Default, Debug, PartialEq, Eq`.

| Field | Type | Meaning |
|---|---|---|
| `drawings` | `usize` | Drawings Krita changed or added on kept layers. |
| `retimed` | `usize` | Layers whose key times changed. |
| `added` | `usize` | Layers added from Krita. |
| `removed` | `usize` | Linked layers Krita deleted. |
| `renamed_or_attrs` | `usize` | Kept layers whose name/opacity/visibility/lock/group state changed. |
| `reordered` | `bool` | Krita reordered the linked layers. |
| `grew_to` | `Option<usize>` | New frame count when the timeline grew. |

### `struct PullPlan`
[src/krita_link.rs:466](../../src/krita_link.rs#L466)

A pull, planned against the current project and ready to apply. Made by
[`plan`](#plan); nothing in the project changes until
[`PullPlan::apply`](#pullplanapply).

| Field | Type | Meaning |
|---|---|---|
| `new_cells` | `Vec<Canvas>` | Drawings to add as new cells. |
| `cell_base` | `usize` | `project.cells.len()` when planned; new cells get ids from here. |
| `stack` | `Vec<Entry>` | The new layer stack, bottom first. |
| `frame_count` | `usize` | Frame count after the pull. |
| `noop` | `bool` | Nothing would change. |
| `next` | `Baseline` | The baseline to keep once this plan is applied (or skipped as a no-op). |
| `report` | `Report` | Counts for the toast. |
| `warnings` | `Vec<String>` | Messages for the user. |
| `unlink` | `Vec<u64>` | [`Layer::uid`](doc/layer.md)s whose link ends with this pull (renamed to stay in Krita there, or made reference / `-x` here). Their layers stay as they are; only the link goes. Applies even to a no-op plan. |

### `enum Slot`
[src/krita_link.rs:594](../../src/krita_link.rs#L594)

A local enum inside the pull planner: for each Krita layer, in Krita order, what the plan does with it — keep it on its linked layer, add it as new, or skip it (deleted here, untouched in Krita, so it stays deleted).

| Variant | Meaning |
|---|---|
| `Keep(usize, LayerEdit)` | A Krita layer linked to app layer `i`: keep it there, applying these edits. |
| `Add(LayerEdit)` | A layer Krita added: insert it as a new layer with these settings. |
| `Skip` | Deleted here, untouched in Krita: stays deleted. |

### `const SETTLE`
[src/krita_link.rs:934](../../src/krita_link.rs#L934)

```rust
const SETTLE: Duration = Duration::from_millis(500);
```

Minimum quiet time after the file changes before it is read, so a save still being written isn't read half-done.

### `const POLL`
[src/krita_link.rs:936](../../src/krita_link.rs#L936)

```rust
const POLL: Duration = Duration::from_secs(1);
```

How often the file is stat'ed.

### `const READY_TIMEOUT`
[src/krita_link.rs:939](../../src/krita_link.rs#L939)

```rust
const READY_TIMEOUT: Duration = Duration::from_secs(4);
```

How long the helper plugin gets to say it's ready (it ticks every 400 ms, and may have a save to make first).

### `const RELOAD_TIMEOUT`
[src/krita_link.rs:942](../../src/krita_link.rs#L942)

```rust
const RELOAD_TIMEOUT: Duration = Duration::from_secs(30);
```

How long it gets to reload — reopening a big animated document takes a while.

### `type Stamp`
[src/krita_link.rs:945](../../src/krita_link.rs#L945)

```rust
pub type Stamp = (SystemTime, u64);
```

`(SystemTime, u64)` — a file's modification time and length. Enough to notice a save: if
either moves, Krita (or someone) wrote the file.

### `struct Round`
[src/krita_link.rs:954](../../src/krita_link.rs#L954)

One **"Send to Krita again"**: get Krita's latest onto disk and into Animator, write the new
file, and — with the helper plugin — have Krita reload it. [app.rs](app.md)'s
`drive_round` moves it through the [`Stage`](#enum-stage)s.

| Field | Type | Meaning |
|---|---|---|
| `seq` | `u64` | Mailbox sequence number; 0 when the helper isn't in play. |
| `stage` | `Stage` | Where the round is. |
| `selected` | `usize` | The layer Krita should open on. |
| `only` | `Option<u64>` | Send only this layer (by `Layer::uid`, because the pull before the send can move it); `None` = whole project. |
| `launch` | `Option<PathBuf>` | Krita executable to start if it has to be (no helper answered, Krita closed). |
| `helper_ready` | `bool` | The helper said "ready", so it will reload the file itself. |
| `helper_silent` | `bool` | The helper is installed but didn't answer — worth a hint in the toast. |
| `note` | `String` | Extra toast text (layers kept out here, Krita-only layers kept there), shown once Krita confirms the reload. |

### `enum Stage`
[src/krita_link.rs:975](../../src/krita_link.rs#L975)

Steps of a [`Round`](#struct-round). Derives `Clone, Copy, Debug, PartialEq, Eq`.

| Variant | Meaning |
|---|---|
| `AwaitReady { until: Instant }` | Asked the helper to get ready (saving Krita's edits if it has any); give up at `until`. |
| `Sync` | Bring in any Krita save not pulled yet, then send. |
| `Sending` | The send worker is writing the file. |
| `AwaitReloaded { until: Instant }` | Told the helper the file is there; waiting for it to reload. |

### `struct KritaLink`
[src/krita_link.rs:986](../../src/krita_link.rs#L986)

The live link itself. One exists while a link is active (`AppState::krita` is
`Option<KritaLink>`).

| Field | Type | Meaning |
|---|---|---|
| `path` | `PathBuf` | The temp `.kra` file both apps share. |
| `uuids` | `HashMap<u64, String>` | `Layer::uid` → the Krita uuid it travels as. |
| `baseline` | `Option<Baseline>` | `None` until the send worker reports back. |
| `last_seen` | `Option<Stamp>` | The file as last read or written by us. A different stamp means a save. |
| `changed` | `Option<(Instant, Stamp)>` | A change noticed, and when; read once it has held still for `settle`. |
| `last_poll` | `Option<Instant>` | When the file was last checked. |
| `child` | `Option<Child>` | Krita as we launched it. Still running = don't launch again. |
| `pending` | `Option<(Stamp, Receiver<Result<Pulled>>)>` | Pull in flight: the stamp being read, and the worker's answer channel. |
| `poll_every` | `Duration` | How often the file is stat'ed, and how long a change must hold still before it is read. Fields so tests can run them at zero. |
| `settle` | `Duration` | How long a changed file must hold still before it is read (see `poll_every`). |
| `round` | `Option<Round>` | The re-send in progress, if any. |
| `helper_seq` | `u64` | Last mailbox sequence number used. |
| `use_helper` | `Option<bool>` | Whether to go through the helper plugin. `None` = "if it's installed"; tests pin it. |
| `ready_timeout` | `Duration` | How long to wait for the helper to answer, and to reload. |
| `reload_timeout` | `Duration` | How long to wait for the helper plugin to reload (see `ready_timeout`). |

### `struct Sent`
[src/krita_link.rs:1145](../../src/krita_link.rs#L1145)

What a [`send`](#send) wrote.

| Field | Type | Meaning |
|---|---|---|
| `baseline` | `Baseline` | What Krita now holds. |
| `stamp` | `Option<Stamp>` | The written file's stamp, so the app doesn't mistake its own write for a Krita save. |
| `kept` | `Vec<String>` | Names of Krita-only (`-x`) layers carried over from the file it replaced. |

## Free functions

### `cell_hash`
[src/krita_link.rs:44](../../src/krita_link.rs#L44)

```rust
pub fn cell_hash(c: &Canvas) -> u64
```

**What it does.** A content fingerprint of one drawing: it feeds the width, the height and
every pixel byte into Rust's standard `DefaultHasher` and returns the 64-bit result. Both
directions of the round trip hash through this one function, so a drawing Krita didn't touch
hashes to the same number when it comes back.

**Example.** The JS idea:
```js
const cellHash = c => hash64([c.width, c.height, ...c.pixels]); // same pixels → same number
```

**Used by:** `Baseline::capture`, `prepare`, `plan` (to check a cell still holds the content),
and [krita_stress.rs](krita_stress.md) (`shows`).

### `packed`
[src/krita_link.rs:54](../../src/krita_link.rs#L54)

```rust
fn packed(mut c: Canvas) -> Canvas
```

Calls `Canvas::pack` (keep only the rectangle that has paint, see
[doc/canvas.rs](doc/canvas.md)) and returns the canvas. Done on the worker so a pull's new
drawings arrive in the project already packed. **Used by:** `prepare`.

### `stays_put`
[src/krita_link.rs:65](../../src/krita_link.rs#L65)

```rust
pub fn stays_put(name: &str) -> bool
```

**What it does.** The `-x` naming rule: `true` when a layer name ends in `-x`, ignoring
upper/lower case and trailing spaces (`"sketch-x"`, `"BG -X "`). The name must be longer than
just `-x`. The same rule applies both ways: such a layer in Animator is never sent, and such a
layer in Krita is never brought back.

**How it works.** Trim trailing whitespace; require length > 2; check the cut point is a
character boundary (so a multi-byte UTF-8 character isn't split); compare the last two bytes
with `"-x"` case-insensitively.

**Used by:** `stays_here`, `stays_here_at`, `comes_back`, and
[krita_stress.rs](krita_stress.md).

### `stays_here`
[src/krita_link.rs:72](../../src/krita_link.rs#L72)

```rust
pub fn stays_here(l: &Layer) -> bool
```

A layer of ours that never goes to Krita: a reference (light-table) layer, or one whose name
passes [`stays_put`](#stays_put). **Used by:** `stays_here_at`.

### `stays_here_at`
[src/krita_link.rs:78](../../src/krita_link.rs#L78)

```rust
pub fn stays_here_at(p: &Project, li: usize) -> bool
```

[`stays_here`](#stays_here) for layer index `li` of project `p`, plus: the layer
also stays when the **group** it is in is named `-x`. `is_some_and` is `Option`'s "is there
a value and does it pass this test".

**Used by:** `sendable`, `plan`, [app.rs](app.md) (`edit_in_krita`, `start_send`),
[ui/shell.rs](ui/shell.md) (layer panel markers), [krita_stress.rs](krita_stress.md).

### `sendable`
[src/krita_link.rs:83](../../src/krita_link.rs#L83)

```rust
pub fn sendable(p: &Project) -> Vec<bool>
```

One `bool` per layer of `p`: `true` if a send takes it to Krita (the opposite of
`stays_here_at`). **Used by:** [app.rs](app.md) `start_send`, [krita_stress.rs](krita_stress.md),
tests.

### `comes_back`
[src/krita_link.rs:89](../../src/krita_link.rs#L89)

```rust
pub fn comes_back(name: &str, groups: &[String]) -> bool
```

**What it does.** The Krita-side filter: a Krita layer comes back here only if its own name
is not `-x` and none of the groups it sits inside (`groups`, their names) is `-x`. It is passed
as a function value to `kra::read_filtered` and `kra::carry` (see [io/kra/mod.rs](io/kra/mod.md)).

**Used by:** `send`, `pull`, [app.rs](app.md) `import_kra`, [krita_stress.rs](krita_stress.md).

### `is_blank`
[src/krita_link.rs:93](../../src/krita_link.rs#L93)

```rust
fn is_blank(c: &Canvas) -> bool
```

`true` when every pixel byte is zero (fully transparent). **Used by:** `Baseline::capture`,
`prepare`.

### `new_uuid`
[src/krita_link.rs:99](../../src/krita_link.rs#L99)

```rust
pub fn new_uuid() -> String
```

**What it does.** Makes a fresh layer id in Krita's format: `{8-4-4-4-12}` hex digits with
braces, 38 characters, a version-4 style id.

**How it works.**
1. A `static N: AtomicU64` counter (a global number safe to bump from any thread) gives a
   different `n` on every call.
2. `RandomState::new()` is the standard library's randomly-seeded hasher factory (seeded from
   the OS), so values differ per process.
3. Hash `(n, now)` into `a`, then `(a, n)` into `b`, and format the bits as the id — the third
   group starts with `4` and the fourth with `8`–`b`, as uuids do.

**Used by:** `prepare` (Krita layer without an id), [app.rs](app.md) (`start_send`, Export
.kra), [krita_stress.rs](krita_stress.md), tests.

## `impl Baseline`

### `Baseline::empty`
[src/krita_link.rs:156](../../src/krita_link.rs#L156)

```rust
pub fn empty(pw: u32, ph: u32) -> Self
```

A baseline with nothing sent yet — no layers, `range_end` 0. Compiled only for tests
(`#[cfg(test)]`). **Used by:** [krita_stress.rs](krita_stress.md) (sessions start from here).

### `Baseline::capture`
[src/krita_link.rs:161](../../src/krita_link.rs#L161)

```rust
pub fn capture(project: &Project, uuids: &[String]) -> Self
```

**What it does.** Records `project` as sent: for each layer (paired with `uuids[i]`) a
[`LayerBase`](#struct-layerbase) with its attributes, its keys as `(frame, hash)`, the size of
each keyed drawing and the hash → cell map.

**Parameters.**
- `project` — the project exactly as written to the `.kra` (already filtered to sent layers).
- `uuids` — one Krita uuid per layer of `project`, bottom first.

**Returns.** The new `Baseline`, with `range_end = frame_count - 1` and no ignored uuids.

**How it works.**
1. Walk every layer zipped with its uuid.
2. For each frame up to `frame_count` with a key (`Some(id)`), skip missing cells, and skip
   **leading** blank keys (Krita needs a blank key where we have nothing).
3. Hash each cell once (a `hashes` cache by `CellId`), push `(frame, hash)` and the size, and
   remember the first cell seen for each hash in `by_hash`.

**Used by:** `send`, tests.

### `Baseline::layer`
[src/krita_link.rs:199](../../src/krita_link.rs#L199)

```rust
fn layer(&self, uuid: &str) -> Option<&LayerBase>
```

Finds the baseline layer with that Krita uuid, or `None`. **Used by:** `prepare`, `plan`, tests.

## `impl PulledLayer`

### `PulledLayer::key_hashes`
[src/krita_link.rs:231](../../src/krita_link.rs#L231)

```rust
fn key_hashes(&self) -> Vec<(usize, u64)>
```

This layer's keys as `(frame, content hash)` — the same shape as `LayerBase::keys`, so the two
can be compared with `!=`. **Used by:** `plan`.

## Free functions

### `present`
[src/krita_link.rs:247](../../src/krita_link.rs#L247)

```rust
pub fn present(project: &Project, links: &HashMap<u64, String>) -> HashSet<String>
```

The Krita uuids of the layers *currently in the project* that are linked (looked up through
`links`, uid → uuid). Passed to [`prepare`](#prepare) so it knows which Krita
layers have a local partner. **Used by:** [app.rs](app.md) (`poll_krita`, `drive_round`),
[krita_stress.rs](krita_stress.md), tests.

### `prepare`
[src/krita_link.rs:255](../../src/krita_link.rs#L255)

```rust
pub fn prepare(doc: KraDoc, base: &Baseline, present: &HashSet<String>) -> Pulled
```

**What it does.** The worker-side half of a pull: turns every drawing of a decoded Krita file
into a cell-sized canvas, hashes it and looks it up in the baseline. A Krita layer whose uuid
is **not** in `present` (deleted here, or new) must come back whole, exactly as Krita has it,
so its pixels are kept even when they match — the matching local cell may have been edited
or deleted since.

**Parameters.**
- `doc` — the decoded `.kra` ([io/kra/mod.rs](io/kra/mod.md) `KraDoc`).
- `base` — the current baseline.
- `present` — uuids linked to a layer here ([`present`](#present)).

**Returns.** A [`Pulled`](#struct-pulled).

**How it works.**
1. Start from the reader's warnings; add one if Krita's canvas size differs from what was
   sent ("drawings kept centered").
2. Work out the offset between Krita's canvas and our frame with `cell_origin` — the two are
   aligned by their centres.
3. For each Krita layer: gather every drawing size the layer had at the last sync (plus its
   `cell_size`), or the frame size for a layer the baseline doesn't know.
4. For each drawing, try placing it at each candidate size and hash the result. The first
   size whose hash is in the baseline's `by_hash` is a **match**: return a `PulledDrawing`
   with `matched: Some(id)` (pixels kept only when the layer comes back whole).
5. No match = new content. Use the size of the key it replaces: find the drawing's first key
   time, take the size of the last baseline key at or before that time (fallback: the first
   candidate size). Reuse the already-placed canvas of that size if there is one. Count it if
   paint fell outside the cell and was clipped.
6. Drop the layer's leading blank keys; give it a fresh uuid if Krita's was empty.
7. Add a "N drawing(s) had paint outside the layer's drawable area (clipped)" warning if any.

**Used by:** `pull`, [krita_stress.rs](krita_stress.md), tests.

## `impl LayerEdit`

### `LayerEdit::is_empty`
[src/krita_link.rs:373](../../src/krita_link.rs#L373)

```rust
fn is_empty(&self) -> bool
```

`true` when every field is `None` — the edit would change nothing. **Used by:** `plan`.

### `LayerEdit::apply`
[src/krita_link.rs:384](../../src/krita_link.rs#L384)

```rust
fn apply(self, l: &mut Layer, fc: usize, resolve: impl Fn(CellRef) -> CellId)
```

**What it does.** Writes each `Some` field into layer `l`: name; `group == Some(true)` turns
the layer into a group (`LayerKind::Group`); `in_group`, `collapsed`; opacity converted from
0–255 to `0.0..=1.0`; visible; locked. If `keys` is set, the layer's `exposures` are rebuilt
from scratch: `fc` empty frames, then each key `(t, r)` with `t < fc` becomes
`Some(resolve(r))`.

**Parameters.**
- `self` — taken by value (consumed), so the `String`s can be moved in without copying.
- `l` — the layer to change (`&mut` = may be modified).
- `fc` — the project's frame count after the pull.
- `resolve` — turns a [`CellRef`](#enum-cellref) into a real cell id.

**Used by:** `PullPlan::apply`.

## `impl Report`

### `Report::summary`
[src/krita_link.rs:437](../../src/krita_link.rs#L437)

```rust
pub fn summary(&self) -> String
```

**What it does.** A short English summary for the toast, e.g.
`"1 drawing, timing on 2 layers, 1 layer added, layer order, 10 frames"`. Only non-zero
parts appear, joined by `", "`. A small closure `n(k, "layer", "layers")` picks singular or
plural.

**Used by:** [app.rs](app.md) `apply_krita_pull` (`"Updated from Krita: …"`).

## `impl PullPlan`

### `PullPlan::is_noop`
[src/krita_link.rs:484](../../src/krita_link.rs#L484)

```rust
pub fn is_noop(&self) -> bool
```

`true` when applying would change nothing (the save matched what Animator already has).
**Used by:** [app.rs](app.md) `apply_krita_pull`, [krita_stress.rs](krita_stress.md), tests.

### `PullPlan::apply`
[src/krita_link.rs:491](../../src/krita_link.rs#L491)

```rust
pub fn apply(self, project: &mut Project) -> Vec<(u64, String)>
```

**What it does.** Carries out the plan on the project it was made against. It must run in
the same frame as `plan`, because new cell ids were numbered from the project's cell count
at planning time — an `assert_eq!` panics with "project changed between plan and apply" if
that count moved.

**Returns.** The `(uid, uuid)` link of every layer it created, so the caller can add them to
`KritaLink::uuids`.

**How it works.**
1. Grow the timeline to the planned frame count; append the new cells (each wrapped in an
   `Arc`, the shared pointer the project stores cells in — see
   [Smart pointers](../rust-primer.md#smart-pointers)).
2. `resolve`: `Old(id)` → `id`, `New(i)` → `cell_base + i`.
3. Take the old layers out of the project (`std::mem::take` leaves an empty `Vec` behind)
   into a list of `Option<Layer>`, so each can be taken once.
4. Walk the planned `stack`: `Local` takes the old layer, applies its edit and records where
   it moved; `New` creates a `Layer::new(name, fc)`, applies its edit and records its link.
5. Fix layer indices that point at other layers: each `lines_from` (the line-art layer a fill
   reads, see [doc/layer.rs](doc/layer.md)) and `current_layer` are remapped through the
   "moved" table.
6. Call `normalize_groups` ([doc/groups.rs](doc/groups.md)): Krita and this side may each have
   moved layers in or out of groups, so put the group structure back in shape.

**Used by:** [app.rs](app.md) `apply_krita_pull` (inside `structural_edit`, which makes it one
undo step), [krita_stress.rs](krita_stress.md), tests.

## Free functions

### `plan`
[src/krita_link.rs:539](../../src/krita_link.rs#L539)

```rust
pub fn plan(project: &Project, base: &Baseline, links: &HashMap<u64, String>, pulled: Pulled) -> Result<PullPlan>
```

**What it does.** The three-way merge. Compares Krita's save (`pulled`) against the baseline
and the project as it is now, and decides, layer by layer, what to keep, change, add, remove
and reorder. It is **pure**: it only reads `project`; nothing changes until
[`PullPlan::apply`](#pullplanapply).

**Parameters.**
- `project` — the project now.
- `base` — what Krita held at the last sync.
- `links` — `Layer::uid` → Krita uuid it was sent as.
- `pulled` — the worker's [`Pulled`](#struct-pulled) result.

**Returns.** `Ok(PullPlan)`, or an error starting with
[`EVERYTHING_DELETED`](#const-everything_deleted) if the result would have no layers, or
"unmatched drawing without pixels" in a case that should not happen.

**How it works.**
1. **Links that end.** A Krita layer renamed to `-x` (`pulled.held`) or a uuid already in
   `base.ignored` is ignored. For each linked local layer: if it now stays here (reference or
   `-x`), its uuid joins `ignored`, its uid goes into `unlink`, with a warning
   `"… stays here now, no longer synced"`; if Krita renamed its partner to `-x`, it is unlinked
   with the warning `"… now stays in Krita; … kept here as it was"`. Neither side's layer is
   touched.
2. **`local_of`**: for each linked uuid in the baseline, the index of the first local layer
   carrying it.
3. **Frame count** grows to Krita's `range_end + 1` if Krita changed its range.
4. **Decide every Krita layer** (bottom first) into a local `enum Slot` defined inside the
   function: `Keep(index, edit)`, `Add(edit)`, or `Skip` (deleted here, untouched in Krita: it
   stays deleted).
   - Build a `LayerEdit` holding only the attributes that differ from the baseline (all of
     them for a layer the baseline doesn't know).
   - `keep` = baseline knows it **and** a local layer has it. `add` = not kept, and Krita
     added it or changed it. `rebuild` keys when kept-and-keys-changed, or added.
   - When rebuilding, each drawing becomes a `CellRef` (once per drawing): a match on a kept
     layer reuses the old cell (so local edits survive where Krita didn't change the
     drawing); a match on a returning layer reuses the old cell only if it still holds that
     exact content; otherwise the drawing's pixels become a new cell. Keys past the end grow
     the frame count. For a kept layer the report counts retimes and new drawings.
   - Record the layer's **next baseline**: unchanged keys keep the old record with fresh
     attributes (`..b.clone()`); rebuilt keys get a new record whose `by_hash` *keeps* old
     entries too, so if Krita later undoes back to an earlier drawing it maps onto the cell
     that already holds it.
   - A re-added layer gets every attribute filled in, not just the changed ones.
5. **Orphans.** A layer inside a Krita group whose group header isn't coming here is placed
   top-level (`in_group = false`) — otherwise it would join whatever group sits next to it.
6. **Removed**: linked layers whose uuid Krita no longer has.
7. **Order.** If the kept layers' order differs between baseline and Krita, Krita's order wins.
   Otherwise the local order is kept (so a reorder done here survives), and each layer Krita
   added is slotted in just above its nearest Krita neighbour below.
8. **Local-only layers** (added here since the send) are "anchored" to the kept linked layer
   beneath them. The final stack is built so that what Krita added just above a layer stays
   right above it (a group Krita made around that layer must sit right on top of it), and the
   local-only layers anchored to that layer go on top of those.
9. If the stack is empty, `bail!` — wiping the whole project from one save is the wrong
   default; the message tells the user to send again or delete the layers themselves.
10. Build the next `Baseline` and the `PullPlan`; `noop` is true when nothing was added,
    removed, reordered, edited or grown.

**Example.** The core rule for one drawing, in JS terms:
```js
// baseline.byHash: Krita-side hash -> local cell id
function cellFor(drawing, layerIsKeptHere) {
  if (drawing.matched != null && layerIsKeptHere) return { old: drawing.matched };
  if (drawing.matched != null && hash(project.cell(drawing.matched)) === drawing.hash)
    return { old: drawing.matched };          // returning layer, cell unchanged
  newCells.push(drawing.canvas);               // Krita changed or added it
  return { new: newCells.length - 1 };
}
```

**Used by:** [app.rs](app.md) `apply_krita_pull`, [krita_stress.rs](krita_stress.md), tests.

### `stamp`
[src/krita_link.rs:947](../../src/krita_link.rs#L947)

```rust
pub fn stamp(path: &Path) -> Option<Stamp>
```

The file's `(modified time, length)`, or `None` if the file or its time can't be read.
**Used by:** `KritaLink::unseen_change`, `KritaLink::poll`, `send`.

## `impl KritaLink`

### `KritaLink::new`
[src/krita_link.rs:1018](../../src/krita_link.rs#L1018)

```rust
pub fn new(path: PathBuf) -> Self
```

A link on temp file `path` with nothing sent yet: empty uuid map, no baseline, no Krita
process, default timings (`POLL`, `SETTLE`, `READY_TIMEOUT`, `RELOAD_TIMEOUT`) and
`use_helper: None`. **Used by:** [app.rs](app.md) `start_send`, app tests.

### `KritaLink::unseen_change`
[src/krita_link.rs:1040](../../src/krita_link.rs#L1040)

```rust
pub fn unseen_change(&self) -> Option<Stamp>
```

The file's stamp if it changed since we last read or wrote it — a Krita save not pulled yet.
Unlike `poll` it doesn't wait for the change to settle. **Used by:** [app.rs](app.md)
`drive_round` (the `Sync` stage pulls it before re-sending).

### `KritaLink::start_pull`
[src/krita_link.rs:1046](../../src/krita_link.rs#L1046)

```rust
pub fn start_pull(&mut self, at: Stamp, present: HashSet<String>) -> bool
```

**What it does.** Starts reading the file on a new thread, against a copy of the current
baseline, and stores `(at, receiver)` in `pending`. Returns `false` (and does nothing) when
there is no baseline yet — a send is in flight.

**Parameters.**
- `at` — the stamp being read; marked as seen when the result lands.
- `present` — the linked uuids ([`present`](#present)).

**Used by:** [app.rs](app.md) `poll_krita`, `drive_round`.

### `KritaLink::temp_path`
[src/krita_link.rs:1060](../../src/krita_link.rs#L1060)

```rust
pub fn temp_path(project_path: Option<&Path>) -> PathBuf
```

The temp file for this app instance: `<system temp>/animator-krita/<stem>-<process id>.kra`,
where `<stem>` is the project's file name without extension, or `untitled`. The process id
keeps two running Animators apart. The folder name `animator-krita` is also what the Krita
plugin looks for ([mailbox.py](../krita-plugin/animator_link/mailbox.md)). **Used by:**
[app.rs](app.md) `start_send`.

### `KritaLink::mark_seen`
[src/krita_link.rs:1071](../../src/krita_link.rs#L1071)

```rust
pub fn mark_seen(&mut self, s: Option<Stamp>)
```

Records that the file now on disk is ours (just written or just pulled), and forgets any
change being watched. **Used by:** [app.rs](app.md) `poll_krita`, app tests.

### `KritaLink::poll`
[src/krita_link.rs:1078](../../src/krita_link.rs#L1078)

```rust
pub fn poll(&mut self, now: Instant) -> Option<Stamp>
```

**What it does.** Called every frame. Returns the stamp to read once a Krita save has
**settled** and no read is already running; otherwise `None`.

**How it works.**
1. Do nothing while a pull is pending or there is no baseline.
2. Rate-limit: only check the file every `poll_every`.
3. If the stamp equals `last_seen`, nothing changed (clear `changed`).
4. If the same new stamp has been seen for at least `settle`, return it.
5. Same new stamp but not long enough: wait. A different stamp: remember it with the time
   (the "settle" timer restarts — Krita may still be writing).

**Example.** The JS equivalent of the debounce:
```js
if (stamp !== lastSeen) {
  if (changed && changed.stamp === stamp && now - changed.at >= settle) return stamp;
  if (!changed || changed.stamp !== stamp) changed = { at: now, stamp };
}
return null;
```

**Used by:** [app.rs](app.md) `poll_krita`.

### `KritaLink::krita_running`
[src/krita_link.rs:1102](../../src/krita_link.rs#L1102)

```rust
pub fn krita_running(&mut self) -> bool
```

Whether the Krita process **we** launched is still open: `Child::try_wait` returns
`Ok(None)` while the process hasn't exited. **Used by:** [app.rs](app.md) `edit_in_krita`
(don't launch a second Krita).

### `KritaLink::launch`
[src/krita_link.rs:1106](../../src/krita_link.rs#L1106)

```rust
pub fn launch(&mut self, krita: &Path) -> Result<()>
```

Starts the Krita executable `krita` with the temp file as its argument
(`Command::new(krita).arg(&self.path).spawn()`, like Node's `child_process.spawn`) and keeps
the child handle. Errors read "starting <path>". **Used by:** [app.rs](app.md) `poll_krita`.

## Free functions

### `find_krita`
[src/krita_link.rs:1118](../../src/krita_link.rs#L1118)

```rust
pub fn find_krita(pref: Option<&Path>) -> Option<PathBuf>
```

**What it does.** Finds Krita's executable. `None` means "ask the user" (the app then opens a
file dialog).

**How it works.**
1. The saved preference, if it is an existing file.
2. On Windows: `Krita (x64)\bin\krita.exe` and `Programs\Krita (x64)\bin\krita.exe` under
   `%ProgramFiles%`, `%ProgramW6432%`, `%ProgramFiles(x86)%` and `%LOCALAPPDATA%`. On macOS:
   `/Applications/krita.app/Contents/MacOS/krita`.
3. Every folder on `PATH`, looking for `krita.exe` (Windows) or `krita`.

**Used by:** [app.rs](app.md) `resolve_krita` and `open_krita_import` (Import Krita brushes),
[brush_library/import.rs](brush_library/import.md) (ignored survey test).

### `send`
[src/krita_link.rs:1165](../../src/krita_link.rs#L1165)

```rust
pub fn send(
    mut project: Project,
    keep: &[bool],
    uuids: &[String],
    selected: Option<usize>,
    path: &Path,
) -> Result<Sent>
```

**What it does.** The worker side of a send: writes the `.kra` and records the baseline. It
takes the worker's own copy of the project (`mut project: Project`, owned), so dropping layers
from it clones no cells.

**Parameters.**
- `project` — a clone of the project to write.
- `keep` — one `bool` per layer: which layers go.
- `uuids`, `selected` — count only the kept layers: the Krita uuid of each, and which kept
  layer Krita should open with selected.
- `path` — the temp `.kra`.

**Returns.** [`Sent`](#struct-sent): baseline, the written file's stamp, and names of carried
Krita-only layers.

**How it works.**
1. `bake_clipping` ([doc/clip.rs](doc/clip.md)): Krita has no clipping like ours, so clipped
   layers are sent cut to their base. It returns, for each baked cell, the clipped cell it
   came from.
2. Drop the layers `keep` doesn't mark (`Vec::retain` with a counter), then
   `normalize_groups` — a group whose header stayed here sends its layers as plain ones.
3. If a previous file exists, lift out Krita's own `-x` layers with `kra::carry` so they go
   back in untouched, in their place in the stack. If that fails it only logs a warning —
   better a send that loses Krita-only layers than no send at all.
4. `kra::write` the project, create the folder, write the file.
5. `Baseline::capture`, then point every `by_hash` entry of a baked cell back at the original
   clipped cell. The baseline records the cut pixels (what Krita holds) but each stands for
   the whole clipped drawing here, so an untouched round trip changes nothing and the layer
   stays clipped.

**Used by:** [app.rs](app.md) `start_send` (on a thread), app tests,
[krita_stress.rs](krita_stress.md), tests.

### `pull`
[src/krita_link.rs:1213](../../src/krita_link.rs#L1213)

```rust
pub fn pull(path: &Path, base: &Baseline, present: &HashSet<String>) -> Result<Pulled>
```

The worker side of a pull: read the file, decode it with `kra::read_filtered` (dropping
layers that fail [`comes_back`](#comes_back)), then
[`prepare`](#prepare). **Used by:** `KritaLink::start_pull`.

## Tests

Run with `cargo test krita_link`. Most tests play "Krita" with a second `Project`: they write
it through our own `.kra` writer, read it back, and plan the pull. One test uses a real file
re-saved by Krita 5.2.9 (`src/io/kra/testdata/krita-5.2.9-resaved.kra`).

| Test | What it checks |
|---|---|
| [`real_krita_resave_of_what_we_sent_is_a_noop`](../../src/krita_link.rs#L1284) | What we sent, opened and re-saved untouched by real Krita 5.2.9, pulls back as no change and no warnings. |
| [`uuids_are_krita_shaped_and_distinct`](../../src/krita_link.rs#L1296) | `new_uuid` gives distinct, 38-character, braced ids with the version `4` in place. |
| [`untouched_krita_file_is_a_noop`](../../src/krita_link.rs#L1305) | Pulling exactly what was sent plans nothing. |
| [`only_the_drawing_krita_changed_is_replaced`](../../src/krita_link.rs#L1313) | One painted drawing becomes one new cell; other keys, the old cell's pixels and the other layer are untouched. |
| [`local_edit_krita_did_not_touch_survives`](../../src/krita_link.rs#L1339) | An edit made here to drawing A survives a pull where Krita edited drawing B. |
| [`both_edited_krita_wins`](../../src/krita_link.rs#L1355) | When both apps edited the same drawing, Krita's pixels win. |
| [`krita_retime_reuses_cells`](../../src/krita_link.rs#L1368) | Moving a key in Krita retimes the layer without making new cells. |
| [`local_retime_on_a_layer_krita_left_alone_stays`](../../src/krita_link.rs#L1383) | A retime here survives when Krita only changed another layer. |
| [`link_survives_undo_to_before_the_send`](../../src/krita_link.rs#L1399) | Restoring layer snapshots from before the send (what undo does) keeps the link, because it keys on `uid`. |
| [`layers_added_removed_and_reordered_in_krita`](../../src/krita_link.rs#L1412) | Krita's added layer arrives, a local-only layer keeps its place, then a Krita delete and swap land correctly. |
| [`krita_key_past_the_end_grows_the_timeline`](../../src/krita_link.rs#L1450) | A Krita key past the last frame grows every layer's timeline. |
| [`second_pull_after_first_is_a_noop`](../../src/krita_link.rs#L1466) | After applying a pull, the same Krita file again changes nothing. |
| [`a_clipped_layer_goes_cut_and_an_untouched_round_trip_is_a_noop`](../../src/krita_link.rs#L1483) | A clipped layer goes cut, one drawing per base drawing, each standing for the whole clipped cell; the round trip is a no-op. |
| [`leading_blank_keys_do_not_count_as_changes`](../../src/krita_link.rs#L1521) | The blank lead-in key the writer adds is not seen as a change. |
| [`oversized_layer_cells_keep_their_size`](../../src/krita_link.rs#L1531) | A layer with cells bigger than the frame gets new cells at the same size. |
| [`stays_put_is_a_trailing_dash_x`](../../src/krita_link.rs#L1548) | The `-x` rule on many names, and `comes_back` with `-x` groups. |
| [`an_untouched_round_trip_with_groups_is_a_noop`](../../src/krita_link.rs#L1617) | Groups survive a no-change round trip, including through a real `send`. |
| [`a_group_made_in_krita_comes_back`](../../src/krita_link.rs#L1630) | A group created in Krita arrives with its opacity, is linked, and a second pull is quiet. |
| [`a_layer_moved_into_a_group_in_krita_moves_here`](../../src/krita_link.rs#L1650) | A layer moved into a group (and the group renamed) in Krita does the same here. |
| [`a_group_made_in_krita_takes_only_its_own_layers`](../../src/krita_link.rs#L1669) | A new Krita group sits right on its layer; a layer added here stays on top. |
| [`a_layer_moved_into_a_group_that_stays_here_stays_out`](../../src/krita_link.rs#L1688) | A layer Krita moves into a group that is now `-x` here stays synced and out of the group; the next pull is quiet. |
| [`a_dash_x_group_stays_here_with_its_layers`](../../src/krita_link.rs#L1714) | A `-x` group and its children are not sent. |
| [`a_layer_clipped_to_a_group_goes_cut`](../../src/krita_link.rs#L1727) | A layer clipped to a group is sent cut to the group, and the round trip is a no-op. |
| [`reference_and_dash_x_layers_are_not_sent`](../../src/krita_link.rs#L1744) | Reference and `-x` layers are left out; pulling the untouched file changes nothing. |
| [`a_dash_x_layer_made_in_krita_stays_there`](../../src/krita_link.rs#L1759) | A `-x` layer added in Krita never comes here. |
| [`renamed_dash_x_in_krita_stops_syncing_and_keeps_ours`](../../src/krita_link.rs#L1774) | Renaming a layer to `-x` in Krita unlinks it with a warning, keeps ours as it was, and later pulls are quiet. |
| [`odd_layer_names_survive_the_round_trip`](../../src/krita_link.rs#L1808) | Quotes, markup characters and non-Latin names go out and come back unchanged. |
| [`resend_keeps_krita_only_layers_in_place`](../../src/krita_link.rs#L1826) | A re-send carries Krita's `sketch-x` layer over untouched and in its place; the baseline holds only our layers. |
| [`a_layer_made_reference_here_after_the_send_unlinks`](../../src/krita_link.rs#L1862) | A layer made reference here after the send is unlinked and left exactly as it was, even though Krita edited it. |

**Test helpers:**
- [`cell`](../../src/krita_link.rs#L1222) — a small canvas with six seeded pixels.
- [`project`](../../src/krita_link.rs#L1233) — two layers over 6 frames: "Ink" keys 0, 2, 4 (4 repeats 0), "Color" key 0.
- `type Links` — `HashMap<u64, String>` (uid → uuid).
- [`link`](../../src/krita_link.rs#L1253) — gives each layer a uuid and returns links plus the baseline.
- [`base_of`](../../src/krita_link.rs#L1259) — a baseline of the project's linked layers as they stand.
- [`pull_from`](../../src/krita_link.rs#L1268) — "Krita saves" a project and the pull is planned; new Krita layers get fresh uuids.
- [`paint`](../../src/krita_link.rs#L1276) — overwrites the first pixel of a cell.
- [`project_with_stay_here_layers`](../../src/krita_link.rs#L1562) — `project()` with Color as reference plus a `notes-x` layer: only Ink goes.
- [`send_linked`](../../src/krita_link.rs#L1573) — does a real `send` to a temp folder and returns links, baseline and the file bytes.
- [`grouped`](../../src/krita_link.rs#L1593) — `project()` plus a "Top" layer, with Ink and Color grouped as "Char".
- [`rows`](../../src/krita_link.rs#L1605) — the stack as strings (`[Group]`, ` child`, `plain`) for easy asserts.

### `mod tests`

Run with `cargo test krita_link::tests` · 29 tests

| Test | What it checks |
|---|---|
| [`real_krita_resave_of_what_we_sent_is_a_noop`](../../src/krita_link.rs#L1284) | The whole premise, against real Krita output: what we sent, loaded and re-saved by Krita 5.2.9 untouched, must pull back as no change at all. |
| [`uuids_are_krita_shaped_and_distinct`](../../src/krita_link.rs#L1296) | Uuids are krita shaped and distinct. |
| [`untouched_krita_file_is_a_noop`](../../src/krita_link.rs#L1305) | Untouched krita file is a noop. |
| [`only_the_drawing_krita_changed_is_replaced`](../../src/krita_link.rs#L1313) | Only the drawing krita changed is replaced. |
| [`local_edit_krita_did_not_touch_survives`](../../src/krita_link.rs#L1339) | Local edit krita did not touch survives. |
| [`both_edited_krita_wins`](../../src/krita_link.rs#L1355) | Both edited krita wins. |
| [`krita_retime_reuses_cells`](../../src/krita_link.rs#L1368) | Krita retime reuses cells. |
| [`local_retime_on_a_layer_krita_left_alone_stays`](../../src/krita_link.rs#L1383) | Local retime on a layer krita left alone stays. |
| [`link_survives_undo_to_before_the_send`](../../src/krita_link.rs#L1399) | Undo restores layer snapshots taken before the send; the link must survive that, which is why it keys on `uid` rather than a field set at send time. |
| [`layers_added_removed_and_reordered_in_krita`](../../src/krita_link.rs#L1412) | Layers added removed and reordered in krita. |
| [`krita_key_past_the_end_grows_the_timeline`](../../src/krita_link.rs#L1450) | Krita key past the end grows the timeline. |
| [`second_pull_after_first_is_a_noop`](../../src/krita_link.rs#L1466) | Second pull after first is a noop. |
| [`a_clipped_layer_goes_cut_and_an_untouched_round_trip_is_a_noop`](../../src/krita_link.rs#L1483) | A clipped layer goes to Krita cut to its base — one drawing per base drawing under it — and an untouched round trip brings nothing back. |
| [`leading_blank_keys_do_not_count_as_changes`](../../src/krita_link.rs#L1521) | Leading blank keys do not count as changes. |
| [`oversized_layer_cells_keep_their_size`](../../src/krita_link.rs#L1531) | Oversized layer cells keep their size. |
| [`stays_put_is_a_trailing_dash_x`](../../src/krita_link.rs#L1548) | Stays put is a trailing dash x. |
| [`an_untouched_round_trip_with_groups_is_a_noop`](../../src/krita_link.rs#L1617) | An untouched round trip with groups is a noop. |
| [`a_group_made_in_krita_comes_back`](../../src/krita_link.rs#L1630) | A group made in krita comes back. |
| [`a_layer_moved_into_a_group_in_krita_moves_here`](../../src/krita_link.rs#L1650) | A layer moved into a group in krita moves here. |
| [`a_group_made_in_krita_takes_only_its_own_layers`](../../src/krita_link.rs#L1669) | Krita puts Top in a new group while a layer is added above Top here: the new group lands right on Top, the layer here on top of the group. |
| [`a_layer_moved_into_a_group_that_stays_here_stays_out`](../../src/krita_link.rs#L1688) | The group stays here now (`-x`), and Krita moves Top, just under it, into its copy of it: Top stays synced and out of the group here, and the next pull is quiet. |
| [`a_dash_x_group_stays_here_with_its_layers`](../../src/krita_link.rs#L1714) | A dash x group stays here with its layers. |
| [`a_layer_clipped_to_a_group_goes_cut`](../../src/krita_link.rs#L1727) | Top clipped to the group goes to Krita cut to it, and the untouched round trip still changes nothing. |
| [`reference_and_dash_x_layers_are_not_sent`](../../src/krita_link.rs#L1744) | Reference and dash x layers are not sent. |
| [`a_dash_x_layer_made_in_krita_stays_there`](../../src/krita_link.rs#L1759) | A dash x layer made in krita stays there. |
| [`renamed_dash_x_in_krita_stops_syncing_and_keeps_ours`](../../src/krita_link.rs#L1774) | Renamed dash x in krita stops syncing and keeps ours. |
| [`odd_layer_names_survive_the_round_trip`](../../src/krita_link.rs#L1808) | Quotes, markup characters and non-Latin scripts in layer names go out and come back unchanged, both ways. |
| [`resend_keeps_krita_only_layers_in_place`](../../src/krita_link.rs#L1826) | Krita adds `sketch-x` between two synced layers and saves; the next send from here rewrites the file but leaves the sketch in its place. |
| [`a_layer_made_reference_here_after_the_send_unlinks`](../../src/krita_link.rs#L1862) | A layer made reference here after the send unlinks. |

**Test helpers** (used only by the tests above):

- [`cell`](../../src/krita_link.rs#L1222) — Cell.
- [`project`](../../src/krita_link.rs#L1233) — Two layers over 6 frames: "Ink" keys 0,2,4 (4 repeats 0), "Color" key 0.
- [`link`](../../src/krita_link.rs#L1253) — Link `p`: a uuid per layer, and the baseline of what Krita now holds.
- [`base_of`](../../src/krita_link.rs#L1259) — A baseline of `p`'s linked layers as they stand.
- [`pull_from`](../../src/krita_link.rs#L1268) — "Krita saves" `krita` and we plan the pull. `krita`'s layers are named through `links`; one Krita made up gets a fresh uuid, as Krita would.
- [`paint`](../../src/krita_link.rs#L1276) — Paint.
- [`project_with_stay_here_layers`](../../src/krita_link.rs#L1562) — A project whose bottom layer is a reference and top layer is named -x: only "Ink" goes.
- [`send_linked`](../../src/krita_link.rs#L1573) — Send linked.
- [`grouped`](../../src/krita_link.rs#L1593) — `project()` plus a "Top" layer, with Ink and Color grouped under it.
- [`rows`](../../src/krita_link.rs#L1605) — Rows.

**Test-only types and constants:** [`Links`](../../src/krita_link.rs#L1250)
