# `src/io/image_import.rs` — importing a still image (file or clipboard)

> **Source:** [src/io/image_import.rs](../../../src/io/image_import.rs) · **Module:** `crate::io::image_import` · **Size:** 202 lines · Line links as of commit `1fa5845`

## What this file is for

Brings a single still image into the project — from a file picked in a dialog, or from the system
clipboard (paste) — as a canvas at its native resolution; the caller puts it on a new layer shown on every
frame, and the layer transform handles placement and scaling. It also watches the clipboard so the New
project dialog can offer "use the clipboard image's size".

## How it connects

- **Used by:** [app.rs](../app.md) (`PasteImage` action → `paste_image_as_background`, New project dialog),
  [ui/shell.rs](../ui/shell.md) (shows `ClipboardWatch::size`).
- **Uses:** the `image` crate (decoding), `arboard` (clipboard), `rfd` (file dialog), Windows API for the
  clipboard sequence number.

## Rust concepts in this file

**`Result<Option<T>>` — three outcomes.** `pick()` returns `Ok(Some(canvas))` (got one), `Ok(None)` (user
cancelled — not an error) or `Err(e)` (something failed). In JS you'd use `null` for cancel and `throw` for
errors. More: [Option and Result](../../rust-primer.md#option-and-result)

**`cfg!(test)` inside code.** `if cfg!(test) { return; }` makes `poll` do nothing under `cargo test`, so
tests never read the real clipboard. More: [Attributes and cfg](../../rust-primer.md#attributes-and-cfg)

**Dependency injection with a closure.** `poll_with(now, seq, read)` takes the clock, sequence number and
reader as arguments, so tests can feed fake values; `poll` passes the real ones.

## Types

### `const CLIPBOARD_POLL`
[src/io/image_import.rs:75](../../../src/io/image_import.rs#L75)

```rust
pub const CLIPBOARD_POLL: Duration = Duration::from_millis(250);
```

How often a window showing `ClipboardWatch::size` should redraw. A copy in another app is not an input event, so nothing else wakes the UI.

### `const CLIPBOARD_SETTLE`
[src/io/image_import.rs:81](../../../src/io/image_import.rs#L81)

```rust
const CLIPBOARD_SETTLE: Duration = Duration::from_millis(1500);
```

How long after the clipboard changes a read that found no image is tried again. The app that copied may still be filling the clipboard, and watchers like Windows' clipboard history open it to read the new image, so a read just after the change can find it busy or not yet holding one.

### `struct ClipboardWatch`
[src/io/image_import.rs:88](../../../src/io/image_import.rs#L88)

The size of the image on the system clipboard, kept current by `poll` for as long as something shows it.

`poll`: ClipboardWatch::poll

| Field | Type | Meaning |
|---|---|---|
| `size` | `Option<(u32, u32)>` | `None` when the clipboard holds no image, or it couldn't be read. |
| `seq` | `Option<u32>` | The clipboard's sequence number at the last look; `None` before it. |
| `changed_at` | `Option<Instant>` | When the clipboard's sequence number last changed (for the settle-and-retry delay). |
| `read_at` | `Option<Instant>` | When the image size was last actually read. |

## Free functions

### `pick`
[src/io/image_import.rs:13](../../../src/io/image_import.rs#L13)

```rust
pub fn pick() -> Result<Option<Canvas>>
```

Shows an open-file dialog for images and decodes the chosen file into a canvas at its own resolution.
`Ok(None)` if cancelled.

### `from_clipboard`
[src/io/image_import.rs:35](../../../src/io/image_import.rs#L35)

```rust
pub fn from_clipboard() -> Result<Option<Canvas>>
```

Reads an image from the system clipboard into a canvas (`Ok(None)` when the clipboard holds no image).

### `clipboard_image_size`
[src/io/image_import.rs:54](../../../src/io/image_import.rs#L54)

```rust
fn clipboard_image_size() -> Result<Option<(u32, u32)>>
```

The size of the clipboard image, or `None` if there is none. Decodes the whole image, so it's only called
when the clipboard changed.

### `clipboard_seq`
[src/io/image_import.rs:65](../../../src/io/image_import.rs#L65)

```rust
fn clipboard_seq() -> u32
```

A number Windows increments whenever the clipboard changes (cheap to read every frame); always 0 on other
systems. The call is `unsafe` because it is a raw Windows API function.

## `impl ClipboardWatch`

### `ClipboardWatch::poll`
[src/io/image_import.rs:100](../../../src/io/image_import.rs#L100)

```rust
pub fn poll(&mut self)
```

Checks the clipboard — cheaply, unless its sequence number changed. Does nothing under tests.

### `ClipboardWatch::poll_with`
[src/io/image_import.rs:107](../../../src/io/image_import.rs#L107)

```rust
fn poll_with(
    &mut self,
    now: Instant,
    seq: u32,
    read: impl FnOnce() -> Result<Option<(u32, u32)>>,
)
```

The logic of `poll` with its inputs passed in: if the sequence number changed, read the image size; if a
read just after a change found nothing, try again after `CLIPBOARD_SETTLE` (the copying app or Windows'
clipboard history may still be busy with it). Once an image is found it isn't decoded again until the next
change.

## Tests

### `mod tests`

Run with `cargo test io::image_import::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`reads_once_per_change_when_the_image_is_there`](../../../src/io/image_import.rs#L148) | Reads once per change when the image is there. |
| [`a_busy_or_half_filled_clipboard_is_read_again_once_it_settles`](../../../src/io/image_import.rs#L170) | A busy or half filled clipboard is read again once it settles. |
| [`a_copy_that_is_not_an_image_stops_being_read_after_settling`](../../../src/io/image_import.rs#L186) | A copy that is not an image stops being read after settling. |

**Test-only types and constants:** [`MS`](../../../src/io/image_import.rs#L145)
