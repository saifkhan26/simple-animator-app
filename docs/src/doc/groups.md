# `src/doc/groups.rs` — layer groups in the flat layer list

> **Source:** [src/doc/groups.rs](../../../src/doc/groups.rs) · **Module:** `crate::doc::groups` · **Size:** 647 lines · Line links as of commit `67a7cb9`

## What this file is for

Layer **groups** (folders of layers) without a tree: the layer stack stays one flat `Vec<Layer>`, bottom
first. A group is a *header* row (`LayerKind::Group`, no drawings), and its members are the run of layers
marked `in_group` directly **below** the header. Groups contain layers, never other groups (one level
only). This is also the order Krita lists a group's contents in.

```text
index  row                       in_group
  4    Background                  false
  3    [Group "Character"]         false   ← header
  2      Line                      true    ┐ members: the in_group run
  1      Colour                    true    ┘ directly below the header
  0    Paper                       false
```

This file adds `impl Project` methods (Rust allows a type's methods to be spread across files) for asking
about groups (`group_of`, `children`, `shows`, `locked_eff`), keeping the rules intact
(`normalize_groups`), and editing: grouping, ungrouping, deleting, moving a layer one step, and
drag-and-drop in the Layers panel.

## How it connects

- **Used by:** [app.rs](../app.md) (group/ungroup/move commands), [app/groups.rs](../app/groups.md),
  [ui/shell.rs](../ui/shell.md) (Layers panel rows, drag and drop), [doc/clip.rs](clip.md),
  [io/composite.rs](../io/composite.md), [io/kra/mod.rs](../io/kra/mod.md) (Krita groups).
- **Uses:** [doc/project.rs](project.md), [doc/layer.rs](layer.md).

## Rust concepts in this file

**Methods for one type in several files.** This file starts with `impl Project { … }` even though
`Project` is defined in `project.rs`. More: [Impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

**Ranges as return values.** `children(g)` returns `start..g`, a `Range<usize>` the caller can loop over
(`for li in p.children(g)`) — an empty range `g..g` for non-groups.

**Iterator chains with `rev` and `take_while`.**
```rust
let start = (0..g).rev().take_while(|&j| self.layers[j].in_group).last().unwrap_or(g);
```
Walk down from the header while layers are `in_group`; the last one reached is the group's bottom.
More: [Iterators](../../rust-primer.md#iterators)

## `impl Project`

### `Project::is_group`
[src/doc/groups.rs:27](../../../src/doc/groups.rs#L27)

```rust
pub fn is_group(&self, li: usize) -> bool
```

Whether row `li` is a group header (`false` for an out-of-range index).

### `Project::group_of`
[src/doc/groups.rs:32](../../../src/doc/groups.rs#L32)

```rust
pub fn group_of(&self, li: usize) -> Option<usize>
```

The header index of the group layer `li` belongs to, or `None`: walks up past the `in_group` run and
checks the next row is a header.

### `Project::children`
[src/doc/groups.rs:42](../../../src/doc/groups.rs#L42)

```rust
pub fn children(&self, g: usize) -> Range<usize>
```

Group `g`'s member layers, bottom first, as an index range (empty for non-headers).

### `Project::block`
[src/doc/groups.rs:52](../../../src/doc/groups.rs#L52)

```rust
pub fn block(&self, li: usize) -> Range<usize>
```

The rows that move together with `li`: a group's members plus header, or just the one layer.

### `Project::panel_rows`
[src/doc/groups.rs:62](../../../src/doc/groups.rs#L62)

```rust
pub fn panel_rows(&self) -> Vec<usize>
```

The rows the Layers panel lists, top first: every layer except members of a folded (collapsed) group.

### `Project::panel_row_of`
[src/doc/groups.rs:71](../../../src/doc/groups.rs#L71)

```rust
pub fn panel_row_of(&self, li: usize) -> usize
```

Which panel row layer `li` appears on: its own, or its folded group's header.

### `Project::shows`
[src/doc/groups.rs:79](../../../src/doc/groups.rs#L79)

```rust
pub fn shows(&self, li: usize) -> bool
```

Whether layer `li` is visible: its own eye and its group's eye.

### `Project::locked_eff`
[src/doc/groups.rs:85](../../../src/doc/groups.rs#L85)

```rust
pub fn locked_eff(&self, li: usize) -> bool
```

Whether layer `li` is locked, by itself or by its group ("effectively locked").

### `Project::paintable`
[src/doc/groups.rs:92](../../../src/doc/groups.rs#L92)

```rust
pub fn paintable(&self, li: usize) -> bool
```

Whether layer `li`'s drawings may be edited: a drawing layer, not a reference layer, not locked (itself or
by its group).

### `Project::normalize_groups`
[src/doc/groups.rs:102](../../../src/doc/groups.rs#L102)

```rust
pub fn normalize_groups(&mut self)
```

Restores the group rules whatever state arrived (from a file, Krita, an edit): headers have no drawings,
transform, clip or reference flag and aren't in a group; every `in_group` run has a header directly above
it (otherwise its layers leave the group); no fill or key-jump link points at a header.

### `Project::apply_order`
[src/doc/groups.rs:150](../../../src/doc/groups.rs#L150)

```rust
pub fn apply_order(&mut self, order: &[usize], in_group: &[bool])
```

Rearranges the whole stack in one go: `order[i]` is the old index of the layer that ends up at `i`, and
`in_group[i]` its new group flag. Every reorder goes through here so fill links (`lines_from`) and the
active layer are carried along.

### `Project::groups_ok`
[src/doc/groups.rs:171](../../../src/doc/groups.rs#L171)

```rust
pub fn groups_ok(&self) -> bool
```

Checks the group rules hold (by normalising a clone and comparing). For assertions and tests.

### `Project::group_layers`
[src/doc/groups.rs:181](../../../src/doc/groups.rs#L181)

```rust
pub fn group_layers(&mut self, sel: &[usize]) -> Option<usize>
```

Puts the selected layers (or whole groups) into a new group whose header goes where the topmost of them
was. A selected group gives its layers to the new one and disappears (no nesting). Returns the new
header's index, which becomes active.

### `Project::ungroup`
[src/doc/groups.rs:246](../../../src/doc/groups.rs#L246)

```rust
pub fn ungroup(&mut self, g: usize)
```

Dissolves group `g`: its members stay where they are, just no longer in a group.

### `Project::delete_block`
[src/doc/groups.rs:264](../../../src/doc/groups.rs#L264)

```rust
pub fn delete_block(&mut self, li: usize)
```

Deletes layer `li`, or a group together with all its layers — refused if it would leave no layers.

### `Project::move_item`
[src/doc/groups.rs:288](../../../src/doc/groups.rs#L288)

```rust
pub fn move_item(&mut self, li: usize, up: bool) -> bool
```

Moves layer `li` one step up or down as the panel reads: a group moves as a whole; a single layer steps
into a group it meets, through it one place at a time, and out the other side. Returns `false` if it
can't move.

### `Project::drop_item`
[src/doc/groups.rs:357](../../../src/doc/groups.rs#L357)

```rust
pub fn drop_item(&mut self, from: usize, target: usize, above: bool) -> bool
```

Drag and drop in the Layers panel: drops layer (or group) `from` just above or below row `target`. Dropping
on a member puts it in that group; on a header's top edge, above the group (outside); on its bottom edge,
as the group's top member; anywhere else, outside any group. A dragged group can't land inside a group.

## Tests

### `mod tests`

Run with `cargo test doc::groups::tests` · 12 tests

| Test | What it checks |
|---|---|
| [`children_are_the_run_under_the_header`](../../../src/doc/groups.rs#L437) | Children are the run under the header. |
| [`normalize_repairs_orphans_and_strips_header_cells`](../../../src/doc/groups.rs#L452) | Normalize repairs orphans and strips header cells. |
| [`grouping_a_scattered_selection_keeps_its_order`](../../../src/doc/groups.rs#L479) | Grouping a scattered selection keeps its order. |
| [`ungroup_and_delete_group`](../../../src/doc/groups.rs#L500) | Ungroup and delete group. |
| [`moves_enter_and_leave_one_step_at_a_time`](../../../src/doc/groups.rs#L515) | Moves enter and leave one step at a time. |
| [`a_header_moves_its_block`](../../../src/doc/groups.rs#L540) | A header moves its block. |
| [`drops_follow_the_rows`](../../../src/doc/groups.rs#L559) | Drops follow the rows. |
| [`a_group_dropped_into_a_group_lands_beside_it`](../../../src/doc/groups.rs#L584) | A group dropped into a group lands beside it. |
| [`apply_order_relinks_lines_from_and_current`](../../../src/doc/groups.rs#L598) | Apply order relinks lines from and current. |
| [`clip_base_stays_in_the_group`](../../../src/doc/groups.rs#L609) | Clip base stays in the group. |
| [`a_layer_above_a_group_clips_to_it`](../../../src/doc/groups.rs#L622) | A layer above a group clips to it. |
| [`hidden_or_locked_groups_cascade`](../../../src/doc/groups.rs#L633) | Hidden or locked groups cascade. |

**Test helpers** (used only by the tests above):

- [`stack`](../../../src/doc/groups.rs#L408) — Layers named `names`, bottom first, none grouped; the top one active.
- [`rows`](../../../src/doc/groups.rs#L421) — The stack bottom first: a header in brackets, a group's layer indented.
- [`by_name`](../../../src/doc/groups.rs#L432) — By name.
