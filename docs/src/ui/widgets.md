# `src/ui/widgets.rs` — custom widgets

> **Source:** [src/ui/widgets.rs](../../../src/ui/widgets.rs) · **Module:** `crate::ui::widgets` · **Size:** 72 lines · Line links as of commit `36405ad`

## What this file is for

Widgets egui doesn't provide. Currently one: a **range slider** with two knobs selecting an inclusive integer
range (used to pick which frames to export or import).

```text
 min ├────●━━━━━━━━━●────┤ max
          start      end
```

## How it connects

- **Used by:** [ui/shell.rs](shell.md) (export and import range dialogs).

## Rust concepts in this file

**Mutable references as "two-way binding".** `range_slider(ui, &mut start, &mut end, min, max)` edits the caller's
variables in place — like passing a ref/setter pair in React. The returned `Response` says whether they changed.
More: [Ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)

**Custom egui widgets.** A widget is just a function: allocate space (`ui.allocate_exact_size`), read pointer
interaction from the `Response`, paint with `ui.painter()`.

## Free functions

### `range_slider`
[src/ui/widgets.rs:10](../../../src/ui/widgets.rs#L10)

```rust
pub fn range_slider(
    ui: &mut Ui,
    start: &mut usize,
    end: &mut usize,
    min: usize,
    max: usize,
) -> Response
```

Draws a horizontal track with two draggable knobs for `start` and `end` within `min..=max`. Dragging a knob (or
clicking near one) moves it, never past the other; the response is marked changed when either moves.
