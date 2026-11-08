# `src/input/shortcuts.rs` — key bindings, the mouse-wheel map and `shortcuts.toml`

> **Source:** [src/input/shortcuts.rs](../../../src/input/shortcuts.rs) · **Module:** `crate::input::shortcuts` · **Size:** 2475 lines · Line links as of commit `36405ad`

## What this file is for

Every keyboard shortcut in the app goes through this file. It defines:

- **`Action`** — the full list of things a key can do ("Tool: Pencil", "Next frame", "Undo",
  "Selection: copy", …), about 120 of them.
- **`KeyCombo`** — one key plus Ctrl / Shift / Alt (or just modifiers with no key).
- **`ShortcutMap`** — which combo each action is bound to, plus what the mouse wheel does with
  each modifier. It answers "which actions fired this frame?" and "which drag gesture is held
  right now?".
- **Loading and saving** the map as `shortcuts.toml` in the user's config folder, so rebinds
  survive restarts. Saving edits the file *in place*, keeping the user's comments and spelling.
- **Clash detection** for the Settings window: two actions on one combo, or a combo the app also
  reads for something built in.

The default bindings sit on the **left side of a QWERTY keyboard** (Q W E R, A S D F, 1 2 3, …)
so someone holding a tablet pen in their right hand can reach them with the left without lifting
the pen.

There are two kinds of binding:

| Kind | Fires… | Example | Can be bare modifiers? |
|---|---|---|---|
| **Press** | once, on the frame the key goes down (auto-repeat repeats it) | `S` = Next frame | No |
| **Hold** | is *read* while a canvas drag starts (or, for two of them, partway through a drag) | hold `Shift` while dragging = pan | Yes (`Ctrl`, `Shift`, `Alt`, or any mix) |

Holds never fire as presses. They are the canvas gestures (zoom / pan / rotate, scrub, flip,
tool adjust, swatch pie — the same choices as [button_drag.rs](button_drag.md)), the Lasso's
selection modes, the Fill lasso's modes, and the two "mid-drag" holds that move or centre a
shape while it is being dragged.

### The `shortcuts.toml` file

The file lives at `<config dir>/animator-app/shortcuts.toml`, where the config dir comes from
the `dirs` crate (`%APPDATA%` — `C:\Users\<you>\AppData\Roaming` — on Windows). The same folder
also holds eframe's UI state (`ui.ron`) and the brush library. A fresh install has no file until
a binding changes, or until Settings' **Open shortcuts file** button writes one out.

It is a flat [TOML](https://toml.io) file: one `name = "value"` line per action, plus four wheel
lines. A short example:

```toml
# my keys — comments survive a save made from Settings
tool_pencil = "Q"
undo = "ctrl+z"          # any case; the app keeps your spelling
redo = "Ctrl+Shift+Z"
tool_ink = ""            # "" (or "none") = unbound on purpose; stays unbound
canvas_pan = "Shift"     # a hold binding may be bare modifiers
hold_scrub = "N"         # …or a key: hold N while dragging to scrub
layer_pick1 = "Alt+1"
wheel = "frames"         # the wheel with nothing held
shift_wheel = "drawings"
ctrl_wheel = "zoom"
alt_wheel = "pan_timeline"
```

Rules, all from the code below:

- **Names** are the `Action` variant in `snake_case`: `ToolPencil` → `tool_pencil`,
  `LayerPick1` → `layer_pick1`, `ArtFlipH` → `art_flip_h` (the full list is in the
  [`Action` table](#enum-action)).
- **Combos** are parts joined by `+`. `Ctrl` (also `Control`, `Cmd`, `Command`, `Super`,
  `Meta`), `Shift`, `Alt` (also `Option`), and at most one key name, in any case — `Space`,
  `Delete`, `esc`, `F5`, `OpenBracket`… (egui's key names). `Ctrl++` means the `+` key.
  The app writes them as `Ctrl+Shift+Alt+Key`, in that order.
- An action **missing** from the file gets its default. An action written as `""` or `none`
  stays unbound.
- A line that **doesn't read** — an unknown action name, a combo that doesn't parse, a value
  that isn't text — costs only that line: its action keeps its default, the line is listed in
  Settings, and saving leaves the line exactly as written until that action is rebound in the
  app.
- A file that isn't valid TOML at all is reported, the defaults are used, and the app **never
  saves over it** (it is "a hand edit half done").

### How a key press becomes an action

```text
OS key-down ─► winit ─► egui-winit ─► egui Event::Key { key, pressed, repeat, modifiers }
                                         ▲
             clipboard_keys::inject ─────┘ (puts back Ctrl+C/X/V that egui-winit swallowed)

every frame, AppState::handle_keys (app.rs):
  ├─ Settings is waiting for a new combo?  → capture it (CAPTURE_KEYS / HoldCapture::step),
  │                                          ShortcutMap::set, shortcuts::save
  ├─ no widget has keyboard focus?         → ShortcutMap::poll_actions(ctx) → Vec<Action>
  │       for each binding: skip holds; KeyCombo::matches (key pressed this frame AND
  │       Ctrl/Shift/Alt exactly equal); hold keys only on a fresh press, not auto-repeat;
  │       plus Ctrl+Shift+Z always means Redo
  │    → a few app rules (Delete: tracks vs selection, Backspace in a polygon, mid-drag keys)
  │    → AppState::dispatch(action)   ← one big `match` that does the work
  └─ a widget has focus?                   → ShortcutMap::poll_focused (only Tab and `)

when a canvas drag starts (ui/shell.rs):  ShortcutMap::held_drag(i) → Action::hold_drag()
                                          → a ButtonDrag gesture (pan, zoom, scrub, …)
the mouse wheel (ui/shell.rs, ui/tracks.rs): ShortcutMap::wheel_action(modifiers) → WheelAction
```

The matching rule in JavaScript terms:

```js
// What KeyCombo::matches does, for one binding, on one frame:
function matches(combo, input) {
  if (combo.key == null) return false;              // bare-modifier combos are holds only
  const m = input.modifiers;
  if (m.ctrl !== combo.ctrl || m.shift !== combo.shift || m.alt !== combo.alt) return false;
  return input.keyPressedThisFrame(combo.key);       // so Shift+S never also fires S
}
```

## How it connects

- **Used by:**
  - [app.rs](../app.md) — `AppState` owns a `ShortcutMap` (`shortcuts` field), a
    `HoldCapture` and a `hold_latch: Option<KeyCombo>`. `with_prefs` calls `load`;
    `handle_keys` runs rebind capture (`CAPTURE_KEYS`, `HoldCapture::step`, `set`, `save`) and
    dispatch (`poll_actions`, `poll_focused`, `mid_drag_keys`, `hold_keys`, `get`);
    `dispatch` matches on `Action` (and `layer_pick`); `read_input` uses `still_held`;
    `reload_shortcuts_file` uses `read_file`; `new` uses `config_dir` for the brushes folder.
  - [main.rs](../main.md) — `config_dir` decides where eframe keeps `ui.ron`.
  - [ui/shell.rs](../ui/shell.md) — the Settings shortcut list (`Action::ALL`, `label`,
    `is_hold`, `clashes`, `Clash`, `clear`, `save`, `ensure_file`, `open_file`, the wheel
    pickers via `WheelSlot` / `WheelAction` / `set_wheel`); tooltips that show a binding
    (`get`, `display`); the canvas drag start (`held_drag`, `hold_drag`, `KeyCombo::still_held`);
    held selection / Fill-lasso modes (`KeyCombo::held`); the wheel (`wheel_action`); the title
    menu's clash count (`conflict_count`).
  - [ui/tracks.rs](../ui/tracks.md) — the Timeline's own wheel handling (`wheel_action`,
    `wheel_slot_of`, `WheelAction`).
  - [clipboard_keys.rs](clipboard_keys.md) — its test checks the restored Ctrl+C / Ctrl+V reach
    `poll_actions`.
  - Many tests elsewhere (`app/*.rs`, `tools/golden_tests.rs`) name `Action`s to drive
    `AppState::dispatch`.
- **Uses:** [button_drag.rs](button_drag.md) (`ButtonDrag`, returned by `Action::hold_drag`);
  egui's input types (`Key`, `Modifiers`, `InputState`, `Context`, `Event`); the `serde`,
  `toml` and `toml_edit` crates for the file; the `dirs` crate for the config folder;
  `std::process::Command` to open the file in the system's editor.

## Rust concepts in this file

**Enums that serde turns into names.** `Action` is a plain list of variants. `#[derive(Serialize,
Deserialize)]` plus `#[serde(rename_all = "snake_case")]` makes serde write `ToolPencil` as the
string `"tool_pencil"` and read it back — that is where the file's line names come from.
```rust
#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action { ToolPencil, ToolInk, /* … */ }
```
```ts
type Action = "tool_pencil" | "tool_ink" /* | … */;
```
`Ord, PartialOrd` give the variants an order (the order they are written in), which is what lets
them be keys of a sorted `BTreeMap`.
More: [Traits and derive](../../rust-primer.md#traits-and-derive),
[Serde and file formats](../../rust-primer.md#serde-and-file-formats)

**`Option<T>` for "maybe nothing".** A `KeyCombo` with `key: None` is a modifier-only combo;
`ShortcutMap::get` returns `None` for an unbound action. `Some(x)` / `None` replace
`x` / `null`, and the compiler makes you handle both.
```rust
pub fn get(&self, action: Action) -> Option<KeyCombo> {
    self.bindings.get(&action).copied()
}
```
```js
get(action) { return this.bindings.get(action) ?? null; }
```
More: [Option and Result](../../rust-primer.md#option-and-result)

**`let … else` and `?` for early exits.** `let Some(key) = self.key else { return false; };`
unpacks an `Option` or leaves the function. `?` does the same in one character: on `None` (or an
`Err`) it returns it straight away.
```rust
let Some(key) = self.key else { return false; };   // matches()
let text = std::fs::read_to_string(config_path()?).ok()?;   // read_file()
```
```js
if (this.key == null) return false;
const p = configPath(); if (p == null) return null;
```
More: [Enums and match](../../rust-primer.md#enums-and-match),
[Option and Result](../../rust-primer.md#option-and-result)

**Sorted maps and sets.** `BTreeMap<Action, KeyCombo>` is a map kept sorted by key (here, by
`Action` order); `BTreeSet<Action>` is a sorted set. They behave like JS `Map` / `Set` but
iterate in key order, so saved files and clash lists come out in a stable order.
More: [Collections](../../rust-primer.md#collections)

**Writing serde's traits by hand.** Most types just `#[derive(Serialize)]`, but `KeyCombo` and
`ShortcutMap` implement `Serialize` / `Deserialize` themselves, so a combo is stored as one
string (`"Ctrl+Shift+Z"`) and one bad line can't spoil the whole file. These impls are
*generic* (`<S: serde::Serializer>` — "works for any serializer") and the deserializer carries a
lifetime `'de` (the input text it borrows from). You can read them as
`toJSON()` / a custom `reviver` in JavaScript.
```rust
impl Serialize for KeyCombo {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.display())   // write "Ctrl+Z"
    }
}
```
```js
class KeyCombo { toJSON() { return this.display(); } }
```
More: [Generics](../../rust-primer.md#generics), [Lifetimes](../../rust-primer.md#lifetimes)

**Closures passed to egui.** egui hands out its input state through a callback:
`ctx.input(|i| …)` locks the state, runs your closure with `i`, and returns what it returns.
```rust
ctx.input(|i| self.bindings.iter().filter(|(_, c)| c.matches(i)).map(|(&a, _)| a).collect())
```
```js
ctx.input(i => [...this.bindings].filter(([, c]) => c.matches(i)).map(([a]) => a));
```
More: [Closures](../../rust-primer.md#closures), [Iterators](../../rust-primer.md#iterators),
[egui and immediate-mode UI](../../rust-primer.md#egui-and-immediate-mode-ui)

**Test-only code: `#[cfg(test)]` and `cfg!(test)`.** `#[cfg(test)]` removes an item from normal
builds (`load_from_str`, the `tests` module); `cfg!(test)` is a `true`/`false` you can test in
ordinary code. `config_path` uses it so `cargo test` can never touch the real
`shortcuts.toml`.
More: [Attributes and cfg](../../rust-primer.md#attributes-and-cfg),
[Tests](../../rust-primer.md#tests)

## Types

### `enum Action`
[src/input/shortcuts.rs:18](../../../src/input/shortcuts.rs#L18)

Everything a shortcut can do. The table lists every variant in source order with its
`shortcuts.toml` name, the label Settings shows (from `Action::label`), and its default combo
(from `ShortcutMap::default`; "—" = unbound by default). **Hold** marks the actions read while
held rather than pressed (`Action::is_hold`).

Two pairs of defaults clash today and are listed as "known" in the `defaults_have_no_conflicts`
test: `FrameDuplicate` / `FlipHorizontal` (both `F`) and `FrameDelete` / `FlipVertical` (both
`Shift+F`). Two other pairs share a combo **on purpose** and are only shown as notes:
`SelectionDelete` / `TrackClear` on `Delete`, and the mid-drag holds with `PlayPause` (`Space`)
and `CanvasRotate` (`Alt`) — see `shared_on_purpose`.

| Variant | Meaning |
|---|---|
| `ToolPencil` | `Q` |
| `ToolInk` | `W` |
| `ToolPen3` | Pick pen slot 3. Unbound by default, like slots 4 to 6. |
| `ToolPen4` | Pick pen slot 4. Unbound by default. |
| `ToolPen5` | Pick pen slot 5. Unbound by default. |
| `ToolPen6` | Pick pen slot 6. Unbound by default. |
| `PenNext` | `Shift+W` — the next pen slot along the row, round at the end; from another tool, back to the pen used last. |
| `PenPrev` | `Shift+Q` — the pen slot before, round at the start; from another tool, back to the pen used last. |
| `PenSwap` | Back to the pen slot used before this one. Unbound by default. |
| `ToolEraser` | `E` |
| `ToolFill` | `R` |
| `ToolColorPicker` | — |
| `PickScreenColor` | `C` |
| `ToolShape` | `G` |
| `ToolTracker` | `X` |
| `ToolLasso` | `Y` |
| `ToolPerspective` | `Shift+G` |
| `ToolLiquify` | — |
| `TogglePerspectiveGrid` | `Shift+Alt+G` |
| `TogglePerspectiveSnap` | `Ctrl+Shift+G` |
| `PerspectiveNextGrid` | `Alt+G` |
| `PlayPause` | `Space` |
| `PlayPauseHere` | `Shift+Space` |
| `FramePrev` | `A` |
| `FrameNext` | `S` |
| `KeyJumpPrev` | `Shift+A` |
| `KeyJumpNext` | `Shift+S` |
| `FrameAdd` | `D` |
| `FrameDuplicate` | `F` |
| `FrameDelete` | `Shift+F` |
| `FrameInsertBefore` | — |
| `OnionToggle` | `O` |
| `LayerAdd` | `T` |
| `LayerDelete` | `Shift+T` |
| `LayerGroup` | `Ctrl+G` |
| `LayerUngroup` | — |
| `LayerToggleVisible` | `V` |
| `LayerLast` | `Z` |
| `LayerSelectAbove` | `Alt+W` |
| `LayerSelectBelow` | `Alt+S` |
| `LayerSelectTop` | `Shift+Alt+W` |
| `LayerSelectBottom` | `Shift+Alt+S` |
| `LayerMoveUp` | `Ctrl+Alt+W` |
| `LayerMoveDown` | `Ctrl+Alt+S` |
| `LayerPick1` | Select the Nth layer counting down from the top of the Layers panel. |
| `LayerPick2` | The "Select layer 2 (from top)" command. |
| `LayerPick3` | The "Select layer 3 (from top)" command. |
| `LayerPick4` | The "Select layer 4 (from top)" command. |
| `LayerPick5` | The "Select layer 5 (from top)" command. |
| `LayerPick6` | The "Select layer 6 (from top)" command. |
| `LayerPick7` | The "Select layer 7 (from top)" command. |
| `LayerPick8` | The "Select layer 8 (from top)" command. |
| `LayerPick9` | The "Select layer 9 (from top)" command. |
| `FadeOthersToggle` | `Shift+Z` |
| `WheelPinToggle` | — |
| `LayerClipToggle` | `Ctrl+Alt+G` |
| `LayerAlphaLockToggle` | `/` (`Slash`) |
| `LayerKeyLinkBelow` | — |
| `KeyBlank` | `1` |
| `KeyCopy` | `2` |
| `Hold` | `3` |
| `SizeDown` | `[` (`OpenBracket`) |
| `SizeUp` | `]` (`CloseBracket`) |
| `SizeSwap` | — |
| `StabilizerToggle` | `U` |
| `SmoothingCycle` | `Shift+U` |
| `Undo` | `Ctrl+Z` |
| `Redo` | `Ctrl+Y` |
| `ClearCell` | `Backspace` |
| `PasteImage` | `Ctrl+Shift+V` |
| `ToggleCheckerBg` | — |
| `TogglePanels` | `Tab` |
| `ToggleMiniTimeline` | `M` |
| `ToggleLockAxes` | — |
| `ZoomReset` | `0` |
| `PanReset` | `H` |
| `RotateReset` | `J` |
| `FlipHorizontal` | `F` |
| `FlipVertical` | `Shift+F` |
| `ArtFlipH` | — |
| `ArtFlipV` | — |
| `ArtRotateCw` | — |
| `ArtRotateCcw` | — |
| `ArtRotate180` | — |
| `CellCut` | `Alt+X` |
| `CellCopy` | `Alt+C` |
| `CellPaste` | `Alt+V` |
| `SelectionCut` | `Ctrl+X` |
| `SelectionCopy` | `Ctrl+C` |
| `SelectionPaste` | `Ctrl+V` |
| `SelectionDelete` | `Delete` |
| `SelectionDeselect` | `Ctrl+D` |
| `SelectAll` | `Ctrl+A` |
| `SelectInvert` | `Ctrl+Shift+I` |
| `SelectionFill` | `Shift+Backspace` |
| `SelModeAdd` | `Ctrl+Shift` |
| `SelModeSubtract` | `Ctrl+Alt` |
| `SelModeIntersect` | `Shift+Alt` |
| `FillLassoErase` | `Ctrl+Alt` |
| `FillLassoEnclose` | `Ctrl+Shift` |
| `FillPolygonSnap` | `Shift+Alt` |
| `ShapeMove` | `Space` |
| `ShapeCentre` | `Alt` |
| `TrackClear` | `Delete` |
| `TrackCloseGap` | `Ctrl+Delete` |
| `SaveProject` | `Ctrl+S` |
| `SaveProjectAs` | `Ctrl+Shift+S` |
| `OpenProject` | `Ctrl+O` |
| `CopyFrame` | — |
| `CanvasZoom` | `Ctrl` |
| `CanvasPan` | `Shift` |
| `CanvasRotate` | `Alt` |
| `HoldToolAdjust` | — |
| `HoldScrub` | — |
| `HoldFlip` | — |
| `HoldSwatchPie` | — |
| `LayerTransformToggle` | `B` |
| `TransformKeyAdd` | `K` |
| `TransformKeyDelete` | `Shift+K` |
| `CameraEditToggle` | `Shift+B` |
| `CameraBoxToggle` | — |
| `CameraKeyAdd` | `Ctrl+K` |
| `CameraKeyDelete` | `Ctrl+Shift+K` |
| `CameraLookThrough` | `Shift+V` |
| `CameraKeyPrev` | — |
| `CameraKeyNext` | — |

### `const Action::ALL`
[src/input/shortcuts.rs:196](../../../src/input/shortcuts.rs#L196)

```rust
pub const ALL: &'static [Action] = &[
    Action::ToolPencil,
    Action::ToolInk,
    Action::ToolPen3,
    Action::ToolPen4,
    Action::ToolPen5,
// …
```

`&'static [Action]` — every action Settings offers, in the order it lists them. It is every
variant **except** the two retired ones (`ToolColorPicker`, `ToggleCheckerBg`).

### `const Action::PENS`
[src/input/shortcuts.rs:455](../../../src/input/shortcuts.rs#L455)

```rust
pub const PENS: [Action; 6] = [
    Action::ToolPencil,
    Action::ToolInk,
    Action::ToolPen3,
    Action::ToolPen4,
    Action::ToolPen5,
// …
```

The actions that pick each pen slot, slot 1 first.

### `const HOLD_DRAGS`
[src/input/shortcuts.rs:529](../../../src/input/shortcuts.rs#L529)

```rust
pub const HOLD_DRAGS: [Action; 7] = [
    Action::CanvasZoom,
    Action::CanvasRotate,
    Action::CanvasPan,
    Action::HoldToolAdjust,
    Action::HoldScrub,
// …
```

`[Action; 7]` — the seven drag-gesture holds in the order they win when several are held at once
with equally specific combos: `CanvasZoom`, `CanvasRotate`, `CanvasPan`, `HoldToolAdjust`,
`HoldScrub`, `HoldFlip`, `HoldSwatchPie`. Read by `ShortcutMap::held_drag`.

### `struct KeyCombo`
[src/input/shortcuts.rs:540](../../../src/input/shortcuts.rs#L540)

One key combination. `Copy`, so it is passed around by value like a number. Written to the file as
a single string such as `"Ctrl+Shift+Z"` (see its `Serialize` impl).

| Field | Type | Meaning |
|---|---|---|
| `key` | `Option<egui::Key>` | The key, or `None` for a modifier-only combo (only valid for hold bindings, and only with at least one modifier). |
| `ctrl` | `bool` | Ctrl must be down. |
| `shift` | `bool` | Shift must be down. |
| `alt` | `bool` | Alt must be down. |

### `struct HoldCapture`
[src/input/shortcuts.rs:685](../../../src/input/shortcuts.rs#L685)

The little state machine Settings uses while waiting for a new **hold** binding. Unlike a press
binding, a hold may be bare modifiers — but egui reports no key event for Shift, Ctrl or Alt
alone, so the combo is taken as "the modifiers held at their peak", when the last of them comes
back up.

| Field | Type | Meaning |
|---|---|---|
| `armed` | `bool` | Everything held when capture began has been let go. Until then, modifiers still down from whatever started the capture (e.g. the click) are ignored. |
| `peak` | `(bool, bool, bool)` | Ctrl, Shift, Alt — each `true` if it has been down at any point since arming. |

### `enum Clash`
[src/input/shortcuts.rs:784](../../../src/input/shortcuts.rs#L784)

Something else sitting on a binding's combo, as the Settings list flags it.

| Variant | Meaning |
|---|---|
| `Action(Action)` | Another action on the same combo. On a press both fire; as holds, only one of them can ever start a drag. A real conflict. |
| `Builtin(&'static str)` | A key the app reads outside this map (arrow keys, Ctrl+Shift+Z, Shift mid-drag); the text says what it does. A real conflict. |
| `Shared(Action, &'static str)` | Another action on the same combo **by design**, kept apart by the rule in the text. Shown as a note, not a conflict. |

### `const FOCUS_PASS_KEYS`
[src/input/shortcuts.rs:858](../../../src/input/shortcuts.rs#L858)

```rust
pub const FOCUS_PASS_KEYS: [egui::Key; 2] = [egui::Key::Tab, egui::Key::Backtick];
```

`[egui::Key; 2]` = `Tab` and `` ` `` (backtick). Keys that still reach the shortcut map while a
widget has keyboard focus. Without this, Tab would only move focus from field to field, which
could leave every shortcut dead after the panels were hidden and shown again; backtick is a
character no name or number field needs. Whatever the map binds to them fires (matched exactly
like any other press).

### `const CAPTURE_KEYS`
[src/input/shortcuts.rs:861](../../../src/input/shortcuts.rs#L861)

```rust
pub const CAPTURE_KEYS: [egui::Key; 65] = {
    use egui::Key::*;
    [
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, Num0, Num1,
        Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, OpenBracket, CloseBracket, Semicolon, Comma,
        Period, Slash, Backslash, Minus, Equals, Space, Tab, Backspace, Backtick, F1, F2, F3, F4,
// …
```

`[egui::Key; 65]` — every key Settings accepts as a new binding: `A`–`Z`, `0`–`9`, `[ ] ; , . / \ - =`,
Space, Tab, Backspace, backtick, `F1`–`F12` and the four arrows. Built inside a `{ }` block with
`use egui::Key::*;` so the names can be written bare.

### `enum WheelSlot`
[src/input/shortcuts.rs:876](../../../src/input/shortcuts.rs#L876)

Which modifier the mouse wheel is turned with. Each slot holds one `WheelAction`, so the wheel
can't clash with itself; turning it with two or more modifiers does nothing.

`WheelSlot::ALL` ([line 884](../../../src/input/shortcuts.rs#L884)) lists the four in that order.

| Variant | Meaning |
|---|---|
| `Plain` | Wheel |
| `Shift` | Shift+Wheel |
| `Ctrl` | Ctrl+Wheel |
| `Alt` | Alt+Wheel |

### `const WheelSlot::ALL`
[src/input/shortcuts.rs:884](../../../src/input/shortcuts.rs#L884)

```rust
pub const ALL: [WheelSlot; 4] = [WheelSlot::Plain, WheelSlot::Shift, WheelSlot::Ctrl, WheelSlot::Alt];
```

`&'static [Action]` — every action Settings offers, in the order it lists them. It is every
variant **except** the two retired ones (`ToolColorPicker`, `ToggleCheckerBg`).

### `enum WheelAction`
[src/input/shortcuts.rs:926](../../../src/input/shortcuts.rs#L926)

What one notch of the wheel does, in whichever slot it sits. It applies over the canvas, the
Timeline and the mini timeline alike — except `PanTimeline`, which only the Timeline's tracks
have.

`WheelAction::ALL` ([line 942](../../../src/input/shortcuts.rs#L942)) lists all seven in that
order.

| Variant | Meaning |
|---|---|
| `Frames` | Plain |
| `Drawings` | Shift |
| `Zoom` | Ctrl |
| `BrushSize` | — |
| `Layers` | — |
| `PanTimeline` | Alt |
| `Nothing` | — |

### `const WheelAction::ALL`
[src/input/shortcuts.rs:942](../../../src/input/shortcuts.rs#L942)

```rust
pub const ALL: [WheelAction; 7] = [
    WheelAction::Frames,
    WheelAction::Drawings,
    WheelAction::Zoom,
    WheelAction::BrushSize,
    WheelAction::Layers,
// …
```

`&'static [Action]` — every action Settings offers, in the order it lists them. It is every
variant **except** the two retired ones (`ToolColorPicker`, `ToggleCheckerBg`).

### `struct ShortcutMap`
[src/input/shortcuts.rs:1031](../../../src/input/shortcuts.rs#L1031)

The user's whole keymap. Saved as `action = "Combo"` lines plus the four wheel lines. An action
the user unbound is saved as `action = ""` and kept in `unbound`, so loading doesn't hand it its
default back. An action absent from both `bindings` and `unbound` is also unbound for now, but
takes its default on the next load.

| Field | Type | Meaning |
|---|---|---|
| `bindings` | `BTreeMap<Action, KeyCombo>` | Each bound action's combo. |
| `unbound` | `BTreeSet<Action>` | Actions the user cleared on purpose. |
| `unread` | `Vec<(String, String)>` | File lines that didn't read, as `(name, value)`: an action this build doesn't know, or a combo that doesn't parse. The action keeps its default meanwhile, and saving leaves the line as written until the action is bound here — "a typo in a hand edit is the user's to fix, not the app's to overwrite". |
| `wheel` | `BTreeMap<WheelSlot, WheelAction>` | What the wheel does in each slot. Always holds all four slots. |

### `enum Raw`
[src/input/shortcuts.rs:1072](../../../src/input/shortcuts.rs#L1072)

A helper enum declared inside `ShortcutMap::deserialize`. With `#[serde(untagged)]`, serde tries each
variant in turn: a value that is text becomes `Raw::Text`, anything else TOML allows (a number, a
table…) becomes `Raw::Other` and is ignored. This lets one bad line in `shortcuts.toml` cost only that
line instead of failing the whole file.

| Variant | Meaning |
|---|---|
| `Text(String)` | The line's value was a string, e.g. `"Ctrl+Z"`. |
| `Other(serde::de::IgnoredAny)` | The line's value was some other TOML type; it is skipped (kept as an "unread" line). |

## `impl Action`

### `Action::label`
[src/input/shortcuts.rs:323](../../../src/input/shortcuts.rs#L323)

```rust
pub fn label(self) -> &'static str
```

The human-readable name of an action, shown in the Settings shortcut list and in tooltips — e.g.
`Action::ToolPencil` → `"Tool: Pencil"`. One `match` arm per action returning a string literal.

### `Action::pen_slot`
[src/input/shortcuts.rs:465](../../../src/input/shortcuts.rs#L465)

```rust
pub fn pen_slot(self) -> Option<usize>
```

The pen slot this action picks, from 0, if it picks one.

### `Action::layer_pick`
[src/input/shortcuts.rs:471](../../../src/input/shortcuts.rs#L471)

```rust
pub fn layer_pick(self) -> Option<usize>
```

For the "select layer n" actions (`LayerPick1`…`LayerPick9`) returns `Some(n)`, counting rows from
the top of the Layers panel starting at 1; `None` for every other action.

### `Action::is_hold`
[src/input/shortcuts.rs:489](../../../src/input/shortcuts.rs#L489)

```rust
pub fn is_hold(self) -> bool
```

True for actions that are *held* while a canvas drag starts rather than pressed — canvas
pan/zoom/rotate, tool-adjust, scrub, flip, swatch pie, the selection modes and the lasso-fill holds.
Their combos may be a key, bare modifiers, or both, and they never fire as a press.

### `Action::is_mid_drag`
[src/input/shortcuts.rs:506](../../../src/input/shortcuts.rs#L506)

```rust
pub fn is_mid_drag(self) -> bool
```

True for the holds read only once a drag is already under way (ShapeMove, ShapeCentre). `matches!`
returns a bool for "does this value match this pattern".

### `Action::hold_drag`
[src/input/shortcuts.rs:513](../../../src/input/shortcuts.rs#L513)

```rust
pub fn hold_drag(self) -> Option<ButtonDrag>
```

For a hold action that *starts its own gesture* (pan, zoom, rotate, tool-adjust, scrub, flip, swatch
pie) returns which `ButtonDrag` gesture it is; `None` for everything else, including selection modes
(those modify what a Lasso drag does instead of replacing it).

## `impl KeyCombo`

### `KeyCombo::plain`
[src/input/shortcuts.rs:549](../../../src/input/shortcuts.rs#L549)

```rust
pub fn plain(key: egui::Key) -> Self
```

Builds a combo of `key` with no modifiers. The next few functions (`ctrl`, `shift`, `alt`,
`ctrl_shift`, `ctrl_alt`, `shift_alt`) are the same constructor with the named modifiers set. They
exist so the default map reads cleanly: `KeyCombo::ctrl(K::Z)` for Ctrl+Z.

### `KeyCombo::ctrl`
[src/input/shortcuts.rs:557](../../../src/input/shortcuts.rs#L557)

```rust
pub fn ctrl(key: egui::Key) -> Self
```

A combo of Ctrl + `key`.

### `KeyCombo::shift`
[src/input/shortcuts.rs:565](../../../src/input/shortcuts.rs#L565)

```rust
pub fn shift(key: egui::Key) -> Self
```

A combo of Shift + `key`.

### `KeyCombo::alt`
[src/input/shortcuts.rs:573](../../../src/input/shortcuts.rs#L573)

```rust
pub fn alt(key: egui::Key) -> Self
```

A combo of Alt + `key`.

### `KeyCombo::ctrl_shift`
[src/input/shortcuts.rs:582](../../../src/input/shortcuts.rs#L582)

```rust
pub fn ctrl_shift(key: egui::Key) -> Self
```

A combo of Ctrl + Shift + `key`.

### `KeyCombo::ctrl_alt`
[src/input/shortcuts.rs:591](../../../src/input/shortcuts.rs#L591)

```rust
pub fn ctrl_alt(key: egui::Key) -> Self
```

A combo of Ctrl + Alt + `key`.

### `KeyCombo::shift_alt`
[src/input/shortcuts.rs:600](../../../src/input/shortcuts.rs#L600)

```rust
pub fn shift_alt(key: egui::Key) -> Self
```

A combo of Shift + Alt + `key`.

### `KeyCombo::modifier_only`
[src/input/shortcuts.rs:610](../../../src/input/shortcuts.rs#L610)

```rust
pub fn modifier_only(ctrl: bool, shift: bool, alt: bool) -> Self
```

A combo with no key, only modifiers (e.g. "Ctrl" alone) — used for drag gestures like Ctrl-drag to
zoom.

### `KeyCombo::display`
[src/input/shortcuts.rs:619](../../../src/input/shortcuts.rs#L619)

```rust
pub fn display(self) -> String
```

Turns a combo into its text form, like `"Ctrl+Shift+Z"` or `"Alt"` for a modifier-only combo. This is
also exactly how combos are written in `shortcuts.toml`.

### `KeyCombo::matches`
[src/input/shortcuts.rs:645](../../../src/input/shortcuts.rs#L645)

```rust
pub fn matches(self, i: &egui::InputState) -> bool
```

True if this combo was *pressed this frame* with exactly these modifiers (Caps Lock and the
Windows/Super key ignored). Modifier-only combos never match here — they are drag gestures, checked
with `held`.

### `KeyCombo::held`
[src/input/shortcuts.rs:659](../../../src/input/shortcuts.rs#L659)

```rust
pub fn held(self, i: &egui::InputState) -> bool
```

True while the combo is being held down: its key (if any) is down with exactly its modifiers. A
key-less combo needs at least one modifier. Used to choose a drag gesture when a drag starts.

### `KeyCombo::still_held`
[src/input/shortcuts.rs:671](../../../src/input/shortcuts.rs#L671)

```rust
pub fn still_held(self, i: &egui::InputState) -> bool
```

True while every part of the combo is still down, *allowing extra modifiers*. Looser than `held`, so
a gesture started with Space survives Shift being pressed on top mid-drag. `self.key.map_or(true, |k|
…)` means "if there is no key, true; otherwise test the key".

## `impl HoldCapture`

### `HoldCapture::step`
[src/input/shortcuts.rs:695](../../../src/input/shortcuts.rs#L695)

```rust
pub fn step(&mut self, mods: egui::Modifiers, pressed: Option<egui::Key>) -> Option<KeyCombo>
```

One frame of capturing a new hold binding in Settings. Bare modifiers are only known once they are
released again (you can't tell "Ctrl" from "Ctrl+something" until then), so this tracks what was held
and returns the captured combo once it is complete.

## `impl Serialize for KeyCombo`

### `KeyCombo::serialize`
[src/input/shortcuts.rs:723](../../../src/input/shortcuts.rs#L723)

```rust
fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error>
```

Tells serde how to write a combo: as its display string (`"Ctrl+Z"`). Implementing serde's
`Serialize` by hand like this replaces the default "write every field" behaviour.

## `impl<'de> Deserialize<'de> for KeyCombo`

### `KeyCombo::deserialize`
[src/input/shortcuts.rs:729](../../../src/input/shortcuts.rs#L729)

```rust
fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error>
```

Tells serde how to read a combo: read a string, then `parse_combo` it; a string that doesn't parse
becomes a serde error mentioning the bad text.

## Free functions

### `parse_combo`
[src/input/shortcuts.rs:735](../../../src/input/shortcuts.rs#L735)

```rust
fn parse_combo(s: &str) -> Option<KeyCombo>
```

Parses text like `"ctrl+shift+z"` into a `KeyCombo`. It splits on `+`, treats ctrl/control/cmd/
command/super/meta as Ctrl, shift as Shift, alt/option as Alt, and the remaining part as the key name,
matched case-insensitively (so `space`, `Esc`, `del` all work). `Ctrl++` means the `+` key. A
modifier-only combo is valid only if at least one modifier is set; an unknown key name gives `None`.

**Example.**
```rust
parse_combo("Ctrl+Shift+Z") // Some(KeyCombo { key: Some(Z), ctrl: true, shift: true, alt: false })
parse_combo("alt")          // Some(KeyCombo { key: None, ctrl: false, shift: false, alt: true })
parse_combo("banana")       // None
```

## `impl Clash`

### `Clash::is_conflict`
[src/input/shortcuts.rs:797](../../../src/input/shortcuts.rs#L797)

```rust
pub fn is_conflict(self) -> bool
```

True for a real conflict between two bindings, false for a deliberate shared combo (`Clash::Shared`,
shown only as a note).

## Free functions

### `shared_on_purpose`
[src/input/shortcuts.rs:804](../../../src/input/shortcuts.rs#L804)

```rust
fn shared_on_purpose(a: Action, b: Action) -> Option<&'static str>
```

Some pairs of actions are allowed to share a combo because `handle_keys` lets exactly one of them
through depending on context; for such a pair this returns the reason (shown in Settings), else
`None`.

### `builtin_clash`
[src/input/shortcuts.rs:829](../../../src/input/shortcuts.rs#L829)

```rust
fn builtin_clash(action: Action, c: KeyCombo) -> Option<&'static str>
```

Whether a combo collides with keys the app reads directly instead of through the map (e.g. Escape,
Enter, the Ctrl+Shift+Z redo alias), returning what it collides with.

### `fresh_press`
[src/input/shortcuts.rs:847](../../../src/input/shortcuts.rs#L847)

```rust
fn fresh_press(i: &egui::InputState, combo: &KeyCombo) -> bool
```

True if this frame contains a real key-down for the combo's key — not an auto-repeat from holding it.
It scans egui's event list with `iter().any(...)` and a `matches!` pattern with a guard
(`if Some(*key) == combo.key`).

## `impl WheelSlot`

### `WheelSlot::key`
[src/input/shortcuts.rs:887](../../../src/input/shortcuts.rs#L887)

```rust
pub fn key(self) -> &'static str
```

The name a wheel slot has in `shortcuts.toml` (`wheel`, `shift_wheel`, `ctrl_wheel`, `alt_wheel`).

### `WheelSlot::label`
[src/input/shortcuts.rs:896](../../../src/input/shortcuts.rs#L896)

```rust
pub fn label(self) -> &'static str
```

The slot's display name in Settings ("Wheel", "Shift+Wheel", …).

### `WheelSlot::named`
[src/input/shortcuts.rs:906](../../../src/input/shortcuts.rs#L906)

```rust
fn named(name: &str) -> Option<Self>
```

Finds the slot whose file name is `name`, if any.

### `WheelSlot::of`
[src/input/shortcuts.rs:911](../../../src/input/shortcuts.rs#L911)

```rust
pub fn of(mods: egui::Modifiers) -> Option<Self>
```

Which slot the mouse wheel is in given the held modifiers; `None` when two or more are held. The
`match` on a tuple of three bools is a compact way to test combinations.

## `impl WheelAction`

### `WheelAction::label`
[src/input/shortcuts.rs:952](../../../src/input/shortcuts.rs#L952)

```rust
pub fn label(self) -> &'static str
```

The display name of a wheel action ("Step frames", "Zoom", …).

### `WheelAction::hint`
[src/input/shortcuts.rs:964](../../../src/input/shortcuts.rs#L964)

```rust
pub fn hint(self) -> &'static str
```

The longer explanation of a wheel action shown as a tooltip in Settings.

### `WheelAction::name`
[src/input/shortcuts.rs:988](../../../src/input/shortcuts.rs#L988)

```rust
pub fn name(self) -> &'static str
```

The value a wheel action is written as in `shortcuts.toml`.

### `WheelAction::named`
[src/input/shortcuts.rs:1002](../../../src/input/shortcuts.rs#L1002)

```rust
fn named(s: &str) -> Option<Self>
```

Reads a wheel action from its file value, ignoring case; an empty value or `none` means Nothing.

## Free functions

### `default_wheel`
[src/input/shortcuts.rs:1014](../../../src/input/shortcuts.rs#L1014)

```rust
fn default_wheel() -> BTreeMap<WheelSlot, WheelAction>
```

The default wheel mapping: plain wheel steps frames, Shift jumps between drawings, Ctrl zooms, Alt
pans the timeline. `BTreeMap::from([...])` builds a sorted map from an array of pairs.

## `impl Serialize for ShortcutMap`

### `ShortcutMap::serialize`
[src/input/shortcuts.rs:1046](../../../src/input/shortcuts.rs#L1046)

```rust
fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error>
```

How the whole map is written to TOML: one `action = "combo"` line per binding (an empty string for
actions the user deliberately unbound), then one line per wheel slot. `BTreeMap` keeps the lines
sorted so the file is stable between saves.

## `impl<'de> Deserialize<'de> for ShortcutMap`

### `ShortcutMap::deserialize`
[src/input/shortcuts.rs:1067](../../../src/input/shortcuts.rs#L1067)

```rust
fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error>
```

How a `shortcuts.toml` is read. Each line is read as `Raw` (text or something else); text lines
naming an action are parsed as combos, wheel lines as wheel actions, and anything unreadable is kept
in `unread` so saving later doesn't destroy the user's line. Missing actions get their defaults.

## `impl Default for ShortcutMap`

### `ShortcutMap::default`
[src/input/shortcuts.rs:1121](../../../src/input/shortcuts.rs#L1121)

```rust
fn default() -> Self
```

The factory key bindings: one `b.insert(Action::…, KeyCombo::…)` per action — Q/W/E/R for
Pencil/Ink/Eraser/Fill, C for the colour picker, Ctrl+Z for undo, and so on — plus the default wheel
map. `use egui::Key as K;` is a local alias so the long list stays readable.

## Free functions

### `action_named`
[src/input/shortcuts.rs:1316](../../../src/input/shortcuts.rs#L1316)

```rust
fn action_named(name: &str) -> Option<Action>
```

Finds the action a line in the file names (e.g. `tool_pencil`), using serde to convert the string
into the enum the same way loading does.

### `key_of`
[src/input/shortcuts.rs:1323](../../../src/input/shortcuts.rs#L1323)

```rust
fn key_of(action: Action) -> String
```

The name an action is written under in the file — the reverse of `action_named`.

## `impl ShortcutMap`

### `ShortcutMap::get`
[src/input/shortcuts.rs:1331](../../../src/input/shortcuts.rs#L1331)

```rust
pub fn get(&self, action: Action) -> Option<KeyCombo>
```

The combo bound to `action`, if any.

### `ShortcutMap::set`
[src/input/shortcuts.rs:1334](../../../src/input/shortcuts.rs#L1334)

```rust
pub fn set(&mut self, action: Action, combo: KeyCombo)
```

Binds `action` to `combo`, removing it from the unbound set and forgetting any unreadable file line
for it.

### `ShortcutMap::forget_unread`
[src/input/shortcuts.rs:1341](../../../src/input/shortcuts.rs#L1341)

```rust
fn forget_unread(&mut self, action: Action)
```

Drops the stored unreadable line for `action` from the file text, since it now has a real binding.

### `ShortcutMap::wheel_action`
[src/input/shortcuts.rs:1347](../../../src/input/shortcuts.rs#L1347)

```rust
pub fn wheel_action(&self, mods: egui::Modifiers) -> WheelAction
```

What the wheel does with these modifiers held (Nothing for two or more modifiers).

### `ShortcutMap::wheel_slot_of`
[src/input/shortcuts.rs:1354](../../../src/input/shortcuts.rs#L1354)

```rust
pub fn wheel_slot_of(&self, action: WheelAction) -> Option<WheelSlot>
```

The first wheel slot that holds `action`, if any.

### `ShortcutMap::set_wheel`
[src/input/shortcuts.rs:1360](../../../src/input/shortcuts.rs#L1360)

```rust
pub fn set_wheel(&mut self, slot: WheelSlot, action: WheelAction)
```

Puts `action` into wheel `slot`.

### `ShortcutMap::clear`
[src/input/shortcuts.rs:1367](../../../src/input/shortcuts.rs#L1367)

```rust
pub fn clear(&mut self, action: Action)
```

Unbinds `action` permanently — it stays unbound across save/load rather than getting its default
back.

### `ShortcutMap::held_drag`
[src/input/shortcuts.rs:1376](../../../src/input/shortcuts.rs#L1376)

```rust
pub fn held_drag(&self, i: &egui::InputState) -> Option<(Action, KeyCombo)>
```

The drag gesture being held right now and the combo holding it. A combo with a key beats a bare
modifier one (Ctrl+Space beats Ctrl); otherwise the order in `HOLD_DRAGS` decides.

### `ShortcutMap::mid_drag_keys`
[src/input/shortcuts.rs:1392](../../../src/input/shortcuts.rs#L1392)

```rust
pub fn mid_drag_keys(&self) -> Vec<egui::Key>
```

The keys bound to the mid-drag holds (move/centre a shape). While a shape is being dragged, those
keys belong to the hold, not to any press shortcut.

### `ShortcutMap::still_held`
[src/input/shortcuts.rs:1402](../../../src/input/shortcuts.rs#L1402)

```rust
pub fn still_held(&self, action: Action, i: &egui::InputState) -> bool
```

Whether `action`'s hold is still down, extra modifiers allowed.

### `ShortcutMap::hold_keys`
[src/input/shortcuts.rs:1408](../../../src/input/shortcuts.rs#L1408)

```rust
pub(crate) fn hold_keys(&self) -> Vec<egui::Key>
```

The keys used by hold bindings; a press shortcut on one of them fires only on its first key-down,
never on auto-repeat.

### `ShortcutMap::clashes`
[src/input/shortcuts.rs:1418](../../../src/input/shortcuts.rs#L1418)

```rust
pub fn clashes(&self) -> BTreeMap<Action, Vec<Clash>>
```

Every binding that shares its combo with another (or with a built-in key), as a map from action to
the list of clashes, for the Settings window. Bindings with no clash are absent.

### `ShortcutMap::conflict_count`
[src/input/shortcuts.rs:1438](../../../src/input/shortcuts.rs#L1438)

```rust
pub fn conflict_count(&self) -> usize
```

How many bindings have a real conflict (notes don't count) — shown as a badge in Settings.

### `ShortcutMap::poll_actions`
[src/input/shortcuts.rs:1448](../../../src/input/shortcuts.rs#L1448)

```rust
pub fn poll_actions(&self, ctx: &egui::Context) -> Vec<Action>
```

**How a key press becomes an action.** Called once per frame (from `AppState::handle_keys`): looks
at egui's input for this frame and returns every action whose combo was pressed. Hold actions are
skipped (they are read when a drag starts), auto-repeat on hold keys is ignored, and Ctrl+Shift+Z is
also accepted as Redo. `AppState::dispatch` then runs each action.

```text
key press ─► egui InputState ─► ShortcutMap::poll_actions ─► [Action, …] ─► AppState::dispatch
```

### `ShortcutMap::poll_focused`
[src/input/shortcuts.rs:1482](../../../src/input/shortcuts.rs#L1482)

```rust
pub fn poll_focused(&self, ctx: &egui::Context) -> Vec<Action>
```

The same as `poll_actions` for a frame where a text field or other widget has keyboard focus: only
actions bound to the keys allowed through focus (`FOCUS_PASS_KEYS`, e.g. Tab) fire.

## Free functions

### `config_dir`
[src/input/shortcuts.rs:1496](../../../src/input/shortcuts.rs#L1496)

```rust
pub fn config_dir() -> Option<PathBuf>
```

The app's settings folder: the OS config directory plus `animator-app` (on Windows
`%APPDATA%\animator-app`). Holds `shortcuts.toml`, `ui.ron` and imported brushes.

### `config_path`
[src/input/shortcuts.rs:1507](../../../src/input/shortcuts.rs#L1507)

```rust
fn config_path() -> Option<PathBuf>
```

The path of `shortcuts.toml` — but `None` when compiled for tests (`cfg!(test)`), so `cargo test`
can never overwrite the real user's key bindings (this happened once).

### `load`
[src/input/shortcuts.rs:1516](../../../src/input/shortcuts.rs#L1516)

```rust
pub fn load() -> (ShortcutMap, Option<String>)
```

Loads the user's shortcuts at startup: the parsed file, or defaults plus the error message if the
file is broken (and the broken file is not overwritten), or defaults if there is no file.

### `load_from_str`
[src/input/shortcuts.rs:1530](../../../src/input/shortcuts.rs#L1530)

```rust
pub fn load_from_str(text: &str) -> ShortcutMap
```

Parses shortcuts from text, falling back to all defaults on error (used by tests).

### `parse`
[src/input/shortcuts.rs:1538](../../../src/input/shortcuts.rs#L1538)

```rust
pub fn parse(text: &str) -> Result<ShortcutMap, String>
```

Parses `shortcuts.toml` text with the `toml` crate into a `ShortcutMap` (migrating old action names
and filling defaults for anything missing), or returns the error as a string.

### `read_file`
[src/input/shortcuts.rs:1594](../../../src/input/shortcuts.rs#L1594)

```rust
pub fn read_file() -> Option<Result<ShortcutMap, String>>
```

Reads the shortcuts file as it is on disk now (for Settings → Reload); `None` if there is no file.
The `?` operators bail out with `None` if there is no config path or the read fails.

### `ensure_file`
[src/input/shortcuts.rs:1602](../../../src/input/shortcuts.rs#L1602)

```rust
pub fn ensure_file(map: &ShortcutMap) -> std::io::Result<PathBuf>
```

Returns the shortcuts file path, writing the file out first if it doesn't exist yet (a fresh install
has none), so "Open shortcuts file" opens an editor showing every binding.

### `ensure_at`
[src/input/shortcuts.rs:1610](../../../src/input/shortcuts.rs#L1610)

```rust
fn ensure_at(path: &std::path::Path, map: &ShortcutMap) -> std::io::Result<()>
```

`ensure_file` for a given path (tests pass their own temporary path).

### `open_file`
[src/input/shortcuts.rs:1623](../../../src/input/shortcuts.rs#L1623)

```rust
pub fn open_file(path: &std::path::Path) -> std::io::Result<()>
```

Opens a file with the system's default program (`explorer` on Windows, `open` on macOS, `xdg-open`
on Linux). The three `#[cfg(...)]` lines pick one at compile time. It doesn't wait for the program.

### `save`
[src/input/shortcuts.rs:1634](../../../src/input/shortcuts.rs#L1634)

```rust
pub fn save(map: &ShortcutMap)
```

Saves the map to the user's `shortcuts.toml` (if there is a config folder), logging the result.

### `save_at`
[src/input/shortcuts.rs:1648](../../../src/input/shortcuts.rs#L1648)

```rust
fn save_at(path: &std::path::Path, map: &ShortcutMap) -> std::io::Result<()>
```

Writes the map to `path`, editing an existing file in place (`save_text`) so comments survive. A file
that isn't valid TOML is left alone — it is probably a half-finished hand edit.

### `save_text`
[src/input/shortcuts.rs:1669](../../../src/input/shortcuts.rs#L1669)

```rust
fn save_text(old: &str, map: &ShortcutMap) -> Option<String>
```

Produces the new file text from the old text plus `map`, using `toml_edit` (a TOML library that keeps
formatting): only lines whose binding changed are rewritten, so the user's comments, ordering,
spelling (`ctrl+z`) and lines this version can't read all stay. `None` if the old text isn't TOML.

## Tests

### `mod tests`

Run with `cargo test input::shortcuts::tests` · 33 tests

| Test | What it checks |
|---|---|
| [`tests_never_reach_the_users_shortcuts_file`](../../../src/input/shortcuts.rs#L1716) | The test suite has no shortcuts file: nothing of the user's to read, and nowhere for a rebind in a test to be saved over it. |
| [`the_shortcuts_file_is_written_only_when_missing`](../../../src/input/shortcuts.rs#L1728) | Opening the file on a fresh install writes it out first, with every binding in it; an existing file — the user's edits — is left as it is. |
| [`a_broken_shortcuts_file_is_an_error_not_the_defaults`](../../../src/input/shortcuts.rs#L1748) | A file that doesn't read says why, rather than quietly turning into the defaults — which a reload would then save over the user's file. |
| [`a_save_edits_the_file_in_place`](../../../src/input/shortcuts.rs#L1767) | A hand-edited file keeps everything the user wrote through a save made here: comments, their spelling of a combo, a line naming an action this build doesn't have, and a line that doesn't parse. Only the binding changed here is rewritten. |
| [`key_names_read_in_any_case`](../../../src/input/shortcuts.rs#L1801) | Key names read in any case. |
| [`paste_on_ctrl_v_is_kept_in_a_current_file`](../../../src/input/shortcuts.rs#L1812) | Ctrl+V for paste_image in a file saved since selections came in is the user's choice: the old migration only moves it in older files. |
| [`selection_defaults_clash_with_nothing`](../../../src/input/shortcuts.rs#L1823) | The selection's bindings must not collide with anything else's — a shared press fires both actions, and a shared held combo makes a selection drag also pan or zoom. |
| [`next_grid_is_alt_g_and_clashes_with_nothing`](../../../src/input/shortcuts.rs#L1844) | Next grid is alt g and clashes with nothing. |
| [`layer_nav_defaults_clash_with_nothing`](../../../src/input/shortcuts.rs#L1856) | Layer nav defaults clash with nothing. |
| [`alt_digit_picks_a_layer_and_nothing_else`](../../../src/input/shortcuts.rs#L1891) | The plain digits are x-sheet keys; Alt+digit must pick a layer without also inserting a key. |
| [`defaults_have_no_conflicts`](../../../src/input/shortcuts.rs#L1919) | No two defaults may share a combo, so a fresh install opens Settings with nothing flagged. Delete is shared on purpose and only noted. |
| [`the_mid_drag_keys_share_the_press_holds_on_purpose`](../../../src/input/shortcuts.rs#L1939) | The mid drag keys share the press holds on purpose. |
| [`clashes_name_the_other_action_and_note_the_delete_pair`](../../../src/input/shortcuts.rs#L1959) | Clashes name the other action and note the delete pair. |
| [`keys_read_outside_the_map_are_flagged`](../../../src/input/shortcuts.rs#L1991) | Keys read outside the map are flagged. |
| [`a_saved_checker_toggle_is_dropped_quietly`](../../../src/input/shortcuts.rs#L2019) | The reported map: the panels on M, the mini timeline on Shift+M and the last layer on Tab. Settings flagged Tab, because it was read as the panel toggle whenever a field had focus, whatever the map said. The checker backdrop is gone: a file that binds its toggle still reads, the binding dropped, with no line left over to warn about. |
| [`tab_and_backtick_are_free_to_bind`](../../../src/input/shortcuts.rs#L2028) | Tab and backtick are free to bind. |
| [`poll_focused_sees_only_the_pass_keys_exactly`](../../../src/input/shortcuts.rs#L2078) | Poll focused sees only the pass keys exactly. |
| [`an_unbound_binding_survives_a_save_over_its_default`](../../../src/input/shortcuts.rs#L2157) | An unbound binding survives a save over its default. |
| [`old_shortcut_files_still_load`](../../../src/input/shortcuts.rs#L2175) | Files written before holds could be keys: the gestures keep their bare modifiers, and the new holds come up unbound. |
| [`ctrl_g_groups_unless_an_old_file_has_the_grid_there`](../../../src/input/shortcuts.rs#L2199) | A fresh install groups on Ctrl+G and finds the perspective grid on Shift+Alt+G; a file from before keeps its grid on Ctrl+G and gets grouping unbound, rather than a clash. |
| [`a_bad_entry_drops_only_itself`](../../../src/input/shortcuts.rs#L2217) | A bad entry drops only itself. |
| [`a_key_hold_matches_only_while_down_with_exact_modifiers`](../../../src/input/shortcuts.rs#L2230) | A key hold matches only while down with exact modifiers. |
| [`hold_actions_never_fire_as_presses`](../../../src/input/shortcuts.rs#L2253) | Hold actions never fire as presses. |
| [`holding_a_hold_key_does_not_repeat_presses_on_it`](../../../src/input/shortcuts.rs#L2265) | Space held to pan, then Ctrl pressed: Space's auto-repeat now arrives as Ctrl+Space, which must not play and pause on every repeat. |
| [`the_more_specific_hold_wins`](../../../src/input/shortcuts.rs#L2302) | The more specific hold wins. |
| [`hold_and_press_on_one_combo_clash`](../../../src/input/shortcuts.rs#L2317) | Hold and press on one combo clash. |
| [`capturing_shift_alone_binds_it_on_release`](../../../src/input/shortcuts.rs#L2333) | Capturing shift alone binds it on release. |
| [`selection_modes_are_holds_and_never_fire_as_presses`](../../../src/input/shortcuts.rs#L2364) | Selection modes are holds and never fire as presses. |
| [`fill_selection_does_not_also_clear`](../../../src/input/shortcuts.rs#L2376) | Shift+Backspace fills; it must not also clear the drawing it just filled, which a loose modifier match would do. |
| [`the_wheel_defaults_and_its_modifiers`](../../../src/input/shortcuts.rs#L2414) | Out of the box: frames, Shift for drawings, Ctrl zooms, Alt pans the tracks. Two modifiers do nothing. |
| [`wheel_lines_read_from_the_file`](../../../src/input/shortcuts.rs#L2428) | The wheel lines read in any case, `none` empties a slot, and a slot the file doesn't name keeps its default. |
| [`a_bad_wheel_line_is_kept_as_written`](../../../src/input/shortcuts.rs#L2441) | A wheel line that doesn't read costs that slot only: its default stands in, and the line stays as written through a save until the slot is set here. |
| [`a_save_writes_the_wheel_in_place`](../../../src/input/shortcuts.rs#L2457) | A save writes the wheel lines into the file in place: comments and the user's own spelling stay, and a changed slot is rewritten. |

**Test helpers** (used only by the tests above):

- [`press`](../../../src/input/shortcuts.rs#L2048) — What `poll` returns for one frame with `key` pressed under `mods`.
- [`with_input`](../../../src/input/shortcuts.rs#L2107) — Run one frame of `events` with `mods` held on `ctx`, and read the input.
- [`key`](../../../src/input/shortcuts.rs#L2130) — Key.
- [`mods`](../../../src/input/shortcuts.rs#L2401) — Mods.

**Test-only types and constants:** [`NONE`](../../../src/input/shortcuts.rs#L2140), [`CTRL`](../../../src/input/shortcuts.rs#L2141), [`SHIFT`](../../../src/input/shortcuts.rs#L2148)
