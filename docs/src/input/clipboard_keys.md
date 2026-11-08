# `src/input/clipboard_keys.rs` — making Ctrl+C / Ctrl+X / Ctrl+V usable as shortcuts

> **Source:** [src/input/clipboard_keys.rs](../../../src/input/clipboard_keys.rs) · **Module:** `crate::input::clipboard_keys` · **Size:** 148 lines · Line links as of commit `1fa5845`

## What this file is for

The app binds the standard clipboard keys to its **pixel selection**: Ctrl+C copies the
selected pixels, Ctrl+X cuts them, Ctrl+V pastes them (see the default bindings in
[shortcuts.md](shortcuts.md)). That clipboard lives inside the app, so what the *system*
clipboard holds does not matter.

The trouble is egui-winit (the layer that turns OS input into egui events). When it sees one of
the clipboard chords it emits a special event — `Event::Copy`, `Event::Cut` or `Event::Paste` —
and **returns before emitting the key press itself**. So a shortcut that waits for "C pressed
with Ctrl" never fires. Paste is worse: egui-winit only sends `Event::Paste` when the system
clipboard holds *text*, so with an image or nothing on the clipboard, Ctrl+V leaves no trace in
egui at all. On Windows the same applies to the old clipboard keys Shift+Delete, Ctrl+Insert
and Shift+Insert.

This file fixes that in two halves:

1. **Record** — on Windows, a message hook ([msg_hook.rs](msg_hook.md)) shows this file every
   raw key-down *before* egui-winit sees it. `on_msg` writes down any of the keys egui-winit
   might swallow (C, X, V, Insert, Delete) in a small shared list.
2. **Put back** — once per frame, just before egui reads its input, `inject` adds a normal
   "key pressed" event for each recorded key that did not arrive on its own.

On other platforms there is no message hook, so `inject` instead maps the Copy / Cut / Paste
events back to C / X / V. That version still misses a Ctrl+V with no text on the clipboard,
because then there is no event to map.

The clipboard events themselves are left in place, so text fields still copy and paste as usual.

## How it connects

- **Used by:**
  - [msg_hook.rs](msg_hook.md) calls `on_msg` for every raw Windows message.
  - [app.rs](../app.md) calls `inject` from `AppState::raw_input_hook` (eframe's per-frame hook
    that may edit the input before egui reads it).
- **Uses:** egui's input types (`RawInput`, `Event`, `Key`), the `windows` crate's message and
  virtual-key constants (`WM_KEYDOWN`, `VK_C`, …). Its test uses
  [shortcuts.rs](shortcuts.md) (`ShortcutMap::poll_actions`, `Action`).

```text
key-down (OS) ──► msg_hook ──► on_msg: push C into PRESSED
              └─► egui-winit: Ctrl+C → Event::Copy only (key dropped)
next frame:   raw_input_hook ──► inject: PRESSED has C, no Key{C} event → push Key{C, pressed}
              egui reads input ──► ShortcutMap::poll_actions sees Ctrl+C → Action::SelectionCopy
```

## Rust concepts in this file

**A global, thread-safe list: `static` + `Mutex`.** `PRESSED` is one list shared by the whole
program. The message hook writes to it and the frame code reads from it, so it is wrapped in a
`Mutex` (a lock): you must `lock()` it to get at the `Vec` inside, and the lock is released
automatically when the guard goes out of scope.
```rust
static PRESSED: std::sync::Mutex<Vec<egui::Key>> = std::sync::Mutex::new(Vec::new());

if let Ok(mut pressed) = PRESSED.lock() {
    pressed.push(key);
} // lock released here
```
```js
// JS is single-threaded, so the closest thing is a module-level array:
const PRESSED = [];
PRESSED.push(key);
```
`lock()` returns a `Result` because a lock can be "poisoned" if a thread panicked while holding
it; this code simply skips the work in that case.
More: [Constants and statics](../../rust-primer.md#constants-and-statics),
[Smart pointers (Mutex)](../../rust-primer.md#smart-pointers)

**`match` with guards.** `_ if vk == VK_C.0 => …` means "any value, but only if this condition
holds". It is used here because the Windows key codes are constants inside wrapper structs, which
cannot be written directly as match patterns.
```rust
let key = match vk {
    _ if vk == VK_C.0 => egui::Key::C,
    _ if vk == VK_X.0 => egui::Key::X,
    _ => return, // not a key we care about: leave the function
};
```
```js
let key;
if (vk === VK_C) key = "C"; else if (vk === VK_X) key = "X"; else return;
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

**Tuple-struct fields and `as` casts.** `msg.wParam.0 as u16` reads the first (and only) field
of the `WPARAM` wrapper struct (`.0`), then converts the number to a 16-bit unsigned integer
with `as`, keeping only the low 16 bits.
More: [Numbers and casts](../../rust-primer.md#numbers-and-casts),
[Structs](../../rust-primer.md#structs)

**`std::mem::take`.** Swaps a value out for its empty default and hands you the old one — "give
me the list and leave an empty one behind" in one step.
```rust
let keys = std::mem::take(&mut *p); // p now holds an empty Vec
```
```js
const keys = PRESSED.splice(0); // empties the array, returns what was in it
```
More: [Ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)

**`matches!` with a pattern and guard.** `matches!(value, Pattern if cond)` is a yes/no test of
whether `value` fits a pattern; `..` means "ignore the other fields".
```rust
matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key)
```
```js
e.type === "Key" && e.pressed === true && e.key === key
```
More: [Enums and match](../../rust-primer.md#enums-and-match),
[Macros](../../rust-primer.md#macros)

## Types

### `static PRESSED`
[src/input/clipboard_keys.rs:18](../../../src/input/clipboard_keys.rs#L18)

```rust
static PRESSED: std::sync::Mutex<Vec<egui::Key>> = std::sync::Mutex::new(Vec::new());
```

Windows only. The keys the message hook saw go down since the last frame. Only the handful of
keys egui-winit can swallow (C, X, V, Insert, Delete) are ever recorded, each at most once, so
the list never grows past five entries. Filled by `on_msg`, emptied by `take_pressed` each frame.

## Free functions

### `on_msg`
[src/input/clipboard_keys.rs:23](../../../src/input/clipboard_keys.rs#L23)

```rust
pub fn on_msg(msg: &windows::Win32::UI::WindowsAndMessaging::MSG)
```

**What it does.** Looks at one raw Windows message. If it is a key-down for C, X, V, Insert or
Delete, it adds that key to `PRESSED` (unless it is already there). Everything else is ignored.

**Parameters.**
- `msg` — the raw Windows message the event loop just read, borrowed.

**How it works.**
1. Returns early unless the message is `WM_KEYDOWN` or `WM_SYSKEYDOWN`. The "SYS" variant is
   included because holding Alt turns Ctrl+Alt+V into a system key-down.
2. Reads the virtual-key code from `wParam` (the message's first argument) as a `u16`.
3. Maps it to an egui key: `VK_C`→`C`, `VK_X`→`X`, `VK_V`→`V`, `VK_INSERT`→`Insert`,
   `VK_DELETE`→`Delete`; any other key returns early.
4. Locks `PRESSED` and pushes the key if it is not already in the list.

Note that it records the key whatever modifiers are held — a plain C is recorded too. `inject`
deals with that by skipping keys that arrived normally.

**Used by:** [msg_hook.rs](msg_hook.md) (`install`'s hook closure), and the test below writes
to `PRESSED` directly to fake it.

### `take_pressed`
[src/input/clipboard_keys.rs:50](../../../src/input/clipboard_keys.rs#L50)

```rust
fn take_pressed(_raw: &egui::RawInput) -> Vec<egui::Key>
```

**What it does.** Empties `PRESSED` and returns what was in it. If the lock cannot be taken it
returns an empty list (`unwrap_or_default`). The `_raw` argument is unused on Windows; it is
there so both platform versions have the same signature.

**Returns.** The keys recorded since the previous frame.

**Used by:** `inject`.

### `take_pressed`
[src/input/clipboard_keys.rs:58](../../../src/input/clipboard_keys.rs#L58)

```rust
fn take_pressed(raw: &egui::RawInput) -> Vec<egui::Key>
```

**What it does.** Without a message hook, works backwards from this frame's events:
`Event::Cut` → `X`, `Event::Copy` → `C`, `Event::Paste(_)` → `V`. Other events are dropped
(`filter_map` keeps only the `Some` results).

**Parameters.**
- `raw` — this frame's raw egui input, read only.

**Returns.** One key per clipboard event in the frame.

**Used by:** `inject`.

### `inject`
[src/input/clipboard_keys.rs:77](../../../src/input/clipboard_keys.rs#L77)

```rust
pub fn inject(raw: &mut egui::RawInput)
```

**What it does.** For every clipboard key egui-winit swallowed this frame, adds an ordinary
"key pressed" event to the frame's input, so the shortcut code ([shortcuts.md](shortcuts.md))
sees Ctrl+C / Ctrl+X / Ctrl+V like any other combo. It runs before egui reads the input.

**Parameters.**
- `raw` — the frame's raw input, borrowed mutably so events can be appended.

**How it works.**
1. Gets the candidate keys from `take_pressed` (which also clears the record).
2. For each key, checks whether a pressed `Event::Key` for it is already in `raw.events`. The
   record includes keys egui-winit passed through untouched (a plain C, say), and this check
   stops those being doubled.
3. If it did not arrive, pushes `Event::Key { key, physical_key: Some(key), pressed: true,
   repeat: false, modifiers: raw.modifiers }` — using the frame's current modifiers, so Ctrl is
   attached.
4. Leaves the `Copy` / `Cut` / `Paste` events alone, so text fields keep working.

**Example.** A frame where the user pressed Ctrl+C:
```text
before inject: events = [Copy]                   modifiers = Ctrl   PRESSED = [C]
after  inject: events = [Copy, Key{C, pressed}]  modifiers = Ctrl   PRESSED = []
```

**Used by:** [app.rs](../app.md) — `AppState::raw_input_hook`.

## Tests

Run with `cargo test input::clipboard_keys`. The test module only compiles on Windows
(`#[cfg(all(test, target_os = "windows"))]`).

| Test | What it checks |
|---|---|
| [`swallowed_clipboard_keys_reach_the_shortcuts`](../../../src/input/clipboard_keys.rs#L109) | Three cases in one test (one test, because `PRESSED` is a process-wide static and tests run in parallel): a Ctrl+V with no event at all becomes `Action::SelectionPaste` and `PRESSED` is drained; a Ctrl+C that only produced `Event::Copy` keeps the `Copy` event and also fires `Action::SelectionCopy`; a plain C that already arrived as a key is not added a second time. |

**Test helpers:** `actions_for` ([line 99](../../../src/input/clipboard_keys.rs#L99)) — runs one
egui frame with the given raw input and returns the actions the default `ShortcutMap`'s
`poll_actions` reports.

### `mod tests`

Run with `cargo test input::clipboard_keys::tests` · 1 tests

| Test | What it checks |
|---|---|
| [`swallowed_clipboard_keys_reach_the_shortcuts`](../../../src/input/clipboard_keys.rs#L109) | Swallowed clipboard keys reach the shortcuts. |

**Test helpers** (used only by the tests above):

- [`actions_for`](../../../src/input/clipboard_keys.rs#L99) — Actions for.
