# `src/io/png_seq.rs` — PNG sequence export

> **Source:** [src/io/png_seq.rs](../../../src/io/png_seq.rs) · **Module:** `crate::io::png_seq` · **Size:** 155 lines · Line links as of commit `1fa5845`

## What this file is for

Exports the animation as one PNG per frame (`frame_0000.png`, `frame_0001.png`, …) into a chosen folder.
Each frame is flattened through the camera exactly like every other export. For a plain frame range the
file number is the **timeline frame number**, so re-exporting frames 10–20 overwrites exactly
`frame_0010.png`…`frame_0020.png` from an earlier full export. If the export order shows frames more than
once (a loop or ping-pong), files are numbered by position instead.

## How it connects

- **Used by:** [app.rs](../app.md) (the export job, `ExportKind::PngSequence`).
- **Uses:** [io/composite.rs](composite.md) (`flatten_frame`), [io/frame_order.rs](frame_order.md).

## Rust concepts in this file

**`HashSet::insert` returns a bool.** `seen.insert(f)` is `false` if `f` was already present, so
`!order.iter().all(|f| seen.insert(*f))` means "some frame appears twice" — a compact duplicate check.
More: [Collections](../../rust-primer.md#collections)

## Free functions

### `repeats`
[src/io/png_seq.rs:16](../../../src/io/png_seq.rs#L16)

```rust
pub fn repeats(order: &[usize]) -> bool
```

Whether `order` contains some timeline frame more than once (then files are numbered by position).

### `export_to`
[src/io/png_seq.rs:33](../../../src/io/png_seq.rs#L33)

```rust
pub fn export_to(project: &Project, dir: &PathBuf, order: &[usize], cams: Option<&[Camera]>) -> Result<()>
```

Writes the frames in `order` as PNGs into `dir`, each flattened (through the per-frame camera from `cams`
when the camera move is stretched), named by timeline frame or by position (see `repeats`). Blocking; runs
on the export worker thread.

## Tests

### `mod tests`

Run with `cargo test io::png_seq::tests` · 4 tests

| Test | What it checks |
|---|---|
| [`plain_range_keeps_absolute_names`](../../../src/io/png_seq.rs#L89) | Plain range keeps absolute names. |
| [`frames_come_out_at_the_camera_s_size`](../../../src/io/png_seq.rs#L98) | Frames come out at the camera s size. |
| [`frames_under_their_own_cameras_are_all_their_own_pictures`](../../../src/io/png_seq.rs#L111) | A camera a frame: the same drawing coming round again is a different picture, written out in full rather than copied. |
| [`looped_order_numbers_sequentially_and_copies_repeats`](../../../src/io/png_seq.rs#L137) | Looped order numbers sequentially and copies repeats. |

**Test helpers** (used only by the tests above):

- [`project`](../../../src/io/png_seq.rs#L60) — Project.
- [`names`](../../../src/io/png_seq.rs#L73) — Names.
- [`tmp`](../../../src/io/png_seq.rs#L82) — Tmp.
