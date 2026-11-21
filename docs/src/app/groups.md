# `src/app/groups.rs` — drawing layer groups on screen

> **Source:** [src/app/groups.rs](../../../src/app/groups.rs) · **Module:** `crate::app::groups` · **Size:** 553 lines · Line links as of commit `31ab01e`

## What this file is for

A group at **full opacity** needs nothing special: its layers are simply drawn one after another. A group
**below full opacity** must look like one picture faded as a whole (as in Krita) — otherwise its layers
would show through one another. So its layers are first merged into one image, which is then drawn at the
group's opacity. Layers can also be clipped to a whole group, which needs the group's shape as a mask.

This file (a child module of `app`) manages those **group images** on `AppState`: building them (via
[doc/clip.rs](../doc/clip.md)'s `render_group`), caching them keyed by a hash of what they're made from,
updating just the changed rectangle when a member drawing is painted on, uploading the merged ones to the
GPU, and dropping them when no longer needed.

## How it connects

- **Used by:** [app.rs](../app.md) (texture sync calls `sync_group_images`, `mark_region` calls
  `mark_group_region`), [ui/shell.rs](../ui/shell.md) `paint_canvas` (`group_texture`,
  `group_drawn_whole`).
- **Uses:** [doc/clip.rs](../doc/clip.md) (`render_group`, `group_sig`, `group_cells`, `group_space`),
  [doc/groups.rs](../doc/groups.md), [doc/canvas.rs](../doc/canvas.md).

## Rust concepts in this file

**One type's methods across files.** `impl AppState { … }` here adds methods to the struct defined in
`src/app.rs`. Being a child module (`mod groups;` inside `app.rs`) also lets it read `AppState`'s private
fields. More: [Impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

**Structs as `HashMap` keys.** `GroupKey { uid, sig, merged }` is used as a key in
`AppState::group_images: HashMap<GroupKey, GroupImage>`; deriving `Hash` and `Eq` is what allows that
(JS `Map` keys would compare objects by identity, Rust compares the values). More:
[Collections](../../rust-primer.md#collections)

## Types

### `struct GroupKey`
[src/app/groups.rs:27](../../../src/app/groups.rs#L27)

Which image of which group: its layer's uid, a hash of what the image is made from (`clip::group_sig`), and whether it is the merged image (drawn on screen) or the mask (read by clip textures).

| Field | Type | Meaning |
|---|---|---|
| `uid` | `u64` | The group header layer's unique id. |
| `sig` | `u64` | Hash of what the image is built from (`clip::group_sig`). |
| `merged` | `bool` | `true` for the on-screen merged image, `false` for the clip mask. |

### `enum BaseKey`
[src/app/groups.rs:35](../../../src/app/groups.rs#L35)

What a clipped layer is cut by: one drawing, or a whole group.

| Variant | Meaning |
|---|---|
| `Cell(CellId)` | Cut by one drawing (a plain base layer's cell). |
| `Group(GroupKey)` | Cut by a whole group's mask. |

### `struct GroupImage`
[src/app/groups.rs:41](../../../src/app/groups.rs#L41)

A group drawn as one image.

| Field | Type | Meaning |
|---|---|---|
| `canvas` | `Canvas` | The group drawn into one canvas (in group space). |
| `cells` | `Vec<(CellId, bool)>` | The cells it is drawn from, and whether each lies pixel for pixel on it — an edit to one of those redoes the same rect here. |
| `pending` | `Dirty` | What changed in those cells since it was drawn. |
| `tex` | `Option<TextureHandle>` | The merged image on the GPU; masks stay on the CPU. |

## `impl GroupImage`

### `GroupImage::bytes`
[src/app/groups.rs:53](../../../src/app/groups.rs#L53)

```rust
pub fn bytes(&self) -> usize
```

Memory this image uses: its CPU canvas plus its GPU texture (width × height × 4 bytes), for the texture
budget.

## `impl AppState`

### `AppState::group_key`
[src/app/groups.rs:61](../../../src/app/groups.rs#L61)

```rust
pub(crate) fn group_key(&self, g: usize, merged: bool) -> GroupKey
```

The cache key for group `g`'s image (merged or mask) on the current frame.

### `AppState::group_drawn_whole`
[src/app/groups.rs:75](../../../src/app/groups.rs#L75)

```rust
pub(crate) fn group_drawn_whole(&self, g: usize) -> bool
```

Whether group `g` draws as one merged image: visible and below full opacity — except while "fade other
layers" is fading around one of its layers (then layers draw individually so the active one stands out).

### `AppState::art_blends`
[src/app/groups.rs:93](../../../src/app/groups.rs#L93)

```rust
pub(crate) fn art_blends(&self) -> bool
```

Whether a layer the canvas draws by itself has a blend mode: then the art goes through the GPU's art stack (`ui::canvas_gpu::Stack`). A layer inside a merged group image is blended there, on the CPU.

### `AppState::in_group_image`
[src/app/groups.rs:106](../../../src/app/groups.rs#L106)

```rust
pub(crate) fn in_group_image(&self, li: usize) -> bool
```

Whether layer `li` only appears as part of its group's merged image.

### `AppState::feeds_group_image`
[src/app/groups.rs:113](../../../src/app/groups.rs#L113)

```rust
pub(crate) fn feeds_group_image(&self, li: usize) -> bool
```

Whether what shows of layer `li` comes from a group image (its group is merged, or something is clipped to
its group). Strokes on such a layer can't use the fast "stream into its own texture" path.

### `AppState::group_texture`
[src/app/groups.rs:122](../../../src/app/groups.rs#L122)

```rust
pub(crate) fn group_texture(&self, g: usize) -> Option<(&TextureHandle, (u32, u32))>
```

Group `g`'s merged GPU texture on the current frame and its size, if built.

### `AppState::sync_group_images`
[src/app/groups.rs:129](../../../src/app/groups.rs#L129)

```rust
pub(super) fn sync_group_images(&mut self, ctx: &egui::Context, now: u64, merged: &[usize], clipped: &[ClipPair])
```

Called during texture sync: builds or updates the group images this frame needs — merged images for
`merged` groups (uploaded to the GPU) and masks for the `clipped` pairs (kept on the CPU) — redrawing only
pending dirty rectangles when possible.

### `AppState::mark_group_region`
[src/app/groups.rs:182](../../../src/app/groups.rs#L182)

```rust
pub(super) fn mark_group_region(&mut self, id: CellId, region: Dirty)
```

Records that `region` of cell `id` changed, so group images drawn from it (and clip textures cut by
those) update that area next sync.

### `AppState::retire_group_images`
[src/app/groups.rs:215](../../../src/app/groups.rs#L215)

```rust
pub(super) fn retire_group_images(&mut self, gone_cell: Option<CellId>)
```

Drops group images that no longer belong to any group or whose cells went away (`gone_cell`).

### `AppState::drop_group_image`
[src/app/groups.rs:228](../../../src/app/groups.rs#L228)

```rust
pub(super) fn drop_group_image(&mut self, k: GroupKey)
```

Removes one group image and every clip texture cut by it, parking their GPU textures in
`retired_textures` (freed next frame, never mid-frame).

## Free functions

### `full`
[src/app/groups.rs:243](../../../src/app/groups.rs#L243)

```rust
fn full(c: &Canvas) -> DirtyRect
```

The rectangle covering the whole canvas `c`.

## Tests

### `mod tests`

Run with `cargo test app::groups::tests` · 15 tests

| Test | What it checks |
|---|---|
| [`a_half_opaque_group_draws_one_merged_texture`](../../../src/app/groups.rs#L313) | A half opaque group draws one merged texture. |
| [`an_opaque_group_draws_its_layers_directly`](../../../src/app/groups.rs#L326) | An opaque group draws its layers directly. |
| [`painting_a_layer_of_the_group_patches_just_that_rect`](../../../src/app/groups.rs#L338) | Painting a layer of the group patches just that rect. |
| [`a_layer_above_a_group_is_cut_to_it_on_screen`](../../../src/app/groups.rs#L363) | A layer above a group is cut to it on screen. |
| [`merge_group_matches_the_group_at_its_opacity`](../../../src/app/groups.rs#L382) | Merging a half-opaque group gives one layer that exports the same. |
| [`merge_group_keeps_its_hidden_layers`](../../../src/app/groups.rs#L406) | Merge group keeps its hidden layers. |
| [`grouping_the_picked_layers_is_one_undo`](../../../src/app/groups.rs#L415) | Grouping the picked layers is one undo. |
| [`a_header_refuses_drawing_and_key_edits`](../../../src/app/groups.rs#L431) | A header refuses drawing and key edits. |
| [`the_layer_keys_walk_the_rows_the_panel_shows`](../../../src/app/groups.rs#L450) | The layer keys walk the rows the panel shows. |
| [`dragging_a_group_moves_every_layer_in_one_undo`](../../../src/app/groups.rs#L475) | Dragging a group moves every layer in one undo. |
| [`with_auto_key_each_layer_gets_a_key_here`](../../../src/app/groups.rs#L493) | With auto key each layer gets a key here. |
| [`a_keyed_layer_has_every_key_moved`](../../../src/app/groups.rs#L503) | A keyed layer has every key moved. |
| [`rotating_a_group_turns_its_layers_about_their_middle`](../../../src/app/groups.rs#L516) | Rotating a group turns its layers about their middle. |
| [`a_locked_group_stays_put`](../../../src/app/groups.rs#L532) | A locked group stays put. |
| [`group_images_go_with_their_group`](../../../src/app/groups.rs#L542) | Group images go with their group. |

**Test helpers** (used only by the tests above):

- [`sync`](../../../src/app/groups.rs#L259) — Run one texture sync and hand back what egui was asked to upload.
- [`alpha`](../../../src/app/groups.rs#L267) — Alpha.
- [`grouped`](../../../src/app/groups.rs#L277) — Two layers on the left half of the frame, red over blue, in a group; a third layer, green all over, above it.
- [`ungroup_active_of`](../../../src/app/groups.rs#L306) — Ungroup active of.
- [`pan_group`](../../../src/app/groups.rs#L467) — A pan of the group as transform mode drives it: begin, move, commit.
