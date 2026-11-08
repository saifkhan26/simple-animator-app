# `src/io/legacy.rs` — frozen layouts of old `.anim` versions

> **Source:** [src/io/legacy.rs](../../../src/io/legacy.rs) · **Module:** `crate::io::legacy` · **Size:** 466 lines · Line links as of commit `1fa5845`

## What this file is for

Animator saves projects with **postcard**, a binary serde format that writes struct fields
in declaration order with no names (see the byte sketch on
[project_file.md](project_file.md#the-anim-file-byte-by-byte)). That makes files small,
but it means a file can only be read by a struct with *exactly* the field list that wrote
it. Every time a field was appended to `Layer` or `Project`, the format version went up,
and files from before the change would no longer fit the live structs.

This file keeps a **frozen copy ("mirror") of each old layout**: `ProjectV2`, `LayerV2`,
`ProjectV3`, … `ProjectV7`, `LayerV7`. The loader in [project_file.rs](project_file.md)
reads an old file into the matching mirror, then converts it to today's `Project` with
`.into()`. The conversions fill every field the old file did not have with a value that
reproduces how that old version behaved (for example, transform keys get `Ease::Linear`,
which plays back exactly as pre-v5 files did).

The rule in the file's header comment: each mirror must reproduce its version's field
order byte for byte and **never change again**.

Which mirror reads which file version (from `decode_from` / `decode_uncompressed` in
[project_file.rs](project_file.md)):

| File version | Compressed? | Mirror used | Layer mirror |
|---|---|---|---|
| 2 | no | `ProjectV2` | `LayerV2` |
| 3 | no | `ProjectV3` | `LayerV3` |
| 4 | no | `ProjectV4` | `LayerV4` |
| 5 | no | `ProjectV6` (same layout as 6) | `LayerV6` |
| 6 | yes | `ProjectV6` | `LayerV6` |
| 7 | yes | `ProjectV7` | `LayerV7` |
| 8 | yes | the live `Project` (no mirror) | the live `Layer` |

## How it connects

- **Used by:** [project_file.rs](project_file.md) — `decode_from` (v6, v7) and
  `decode_uncompressed` (v2–v5) parse into these mirrors and call `.into()`; its tests
  build mirrors by hand to fake old files.
- **Uses:** the live document types it converts into and borrows field types from:
  [doc/project.rs](../doc/project.md) (`Project`), [doc/layer.rs](../doc/layer.md)
  (`Layer`, `TrackSample`, `next_uid`), [doc/camera.rs](../doc/camera.md)
  (`Camera`, `CameraKey`), [doc/canvas.rs](../doc/canvas.md) (`Canvas`),
  [doc/transform.rs](../doc/transform.md) (`Transform`, `TransformKey`, `Ease`).

```text
old .anim body ──postcard──► ProjectV4 { layers: Vec<LayerV4>, … }
                                   │  impl From<ProjectV4> for Project
                                   ▼
                              Project { layers: Vec<Layer>, camera_opts: default, … }
                                   ▲  each LayerV4 → Layer via impl From<LayerV4> for Layer
```

## Rust concepts in this file

**`impl From<A> for B` and `.into()` — a typed converter.** Implementing the standard
`From` trait teaches Rust how to build a `B` out of an `A`. Once it exists, `a.into()`
works anywhere a `B` is expected.
```rust
impl From<LayerV2> for Layer {
    fn from(l: LayerV2) -> Self { Layer { name: l.name, /* … */ } }
}
let layer: Layer = old_layer.into();
```
```js
function layerFromV2(l) { return { name: l.name, /* … */ }; }
const layer = layerFromV2(oldLayer);
```
More: [Traits and derive](../../rust-primer.md#traits-and-derive)

**`#[derive(serde::Serialize, serde::Deserialize)]`.** One line generates the code to
write and read the struct in any serde format. With postcard the generated code reads the
fields in the order they appear, which is why these structs must never be reordered.
Several fields carry `#[serde(default)]` (copied from the live structs); with postcard
every field is read in order and none can be marked absent, so it is the *version number*
that chooses the right mirror, not these defaults.
More: [Serde and file formats](../../rust-primer.md#serde-and-file-formats)

**Moving vs. converting a whole `Vec`.** `p.layers.into_iter().map(Into::into).collect()`
takes ownership of the old vector, converts each item with its `From` impl, and gathers
the results into a new `Vec`. `Arc::new` is used the same way to wrap each cell.
```rust
layers: p.layers.into_iter().map(Into::into).collect(),
cells:  p.cells.into_iter().map(Arc::new).collect(),
```
```js
layers: p.layers.map(layerFromV4),
cells:  p.cells.map(c => shared(c)),
```
More: [Iterators](../../rust-primer.md#iterators), [Smart pointers](../../rust-primer.md#smart-pointers)

**Struct update syntax `..other`.** "Use these fields, take the rest from `other`." The
v7 layer conversion builds a `LayerV6`, converts it, and overrides just two fields:
```rust
Layer { clip: l.clip, alpha_lock: l.alpha_lock, ..LayerV6 { /* … */ }.into() }
```
```js
({ ...layerFromV6({ /* … */ }), clip: l.clip, alpha_lock: l.alpha_lock })
```
More: [Structs](../../rust-primer.md#structs)

## impl `From<TransformKeyV4> for TransformKey`

## impl `From<LayerV2> for Layer`

## impl `From<ProjectV2> for Project`

## impl `From<LayerV3> for Layer`

## impl `From<LayerV4> for Layer`

## impl `From<ProjectV4> for Project`

## impl `From<ProjectV3> for Project`

## impl `From<LayerV6> for Layer`

## impl `From<ProjectV6> for Project`

## impl `From<LayerV7> for Layer`

## impl `From<ProjectV7> for Project`

## Types

### `struct TransformKeyV4`
[src/io/legacy.rs:22](../../../src/io/legacy.rs#L22)

A transform keyframe as files v2–v4 wrote it, before `ease` was appended. Derives
`Clone, Copy, Serialize, Deserialize`.

Its doc comment records a lesson: mirrors must own frozen copies of every nested struct
that can still change. When `ease` was added to the real `TransformKey`, the v2/v3 mirrors
were still using it and would have started expecting a field those files never wrote —
so this frozen copy was made. (The other nested types listed above are still shared with
the live code.)

| Field | Type | Meaning |
|---|---|---|
| `frame` | `usize` | Frame the key sits on. |
| `transform` | `Transform` | The layer pose (offset, scale, rotation) at that frame. |

### `struct LayerV2`
[src/io/legacy.rs:44](../../../src/io/legacy.rs#L44)

`Layer` as format v2 wrote it — before `track_points`.

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | Layer name. |
| `opacity` | `f32` | 0.0–1.0. |
| `visible` | `bool` | Shown or hidden. |
| `locked` | `bool` | Editing blocked. |
| `reference` | `bool` | Light-table layer: drawn dim, left out of exports. |
| `exposures` | `Vec<Option<usize>>` | Per frame: which cell (drawing) starts showing there, or `None` (hold / nothing). |
| `transform` | `Transform` | Static layer transform. |
| `transform_keys` | `Vec<TransformKeyV4>` | Transform keyframes (no ease). |

### `struct ProjectV2`
[src/io/legacy.rs:59](../../../src/io/legacy.rs#L59)

`Project` as format v2 wrote it.

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | As `Project::width` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `height` | `u32` | As `Project::height` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `fps` | `f32` | Frames per second. |
| `cells` | `Vec<Canvas>` | The pool of drawings (plain, not yet `Arc`-shared). |
| `layers` | `Vec<LayerV2>` | Bottom to top. |
| `frame_count` | `usize` | Timeline length. |
| `current_frame` | `usize` | As `Project::current_frame` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `current_layer` | `usize` | As `Project::current_layer` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_start` | `usize` | As `Project::loop_start` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_end` | `usize` | As `Project::loop_end` in the current format ([project.md](../doc/project.md)), at this version's position. |

### `struct LayerV3`
[src/io/legacy.rs:75](../../../src/io/legacy.rs#L75)

`Layer` as format v3 wrote it — before the per-layer cell size. Same fields as
[`LayerV2`](#struct-layerv2) plus:

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | As `Layer::name` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `opacity` | `f32` | As `Layer::opacity` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `visible` | `bool` | As `Layer::visible` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `locked` | `bool` | As `Layer::locked` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `reference` | `bool` | As `Layer::reference` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `exposures` | `Vec<Option<usize>>` | As `Layer::exposures` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `transform` | `Transform` | As `Layer::transform` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `transform_keys` | `Vec<TransformKeyV4>` | As `Layer::transform_keys` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `track_points` | `Vec<TrackSample>` | Per-frame stabilization tracking samples. |

### `struct ProjectV3`
[src/io/legacy.rs:92](../../../src/io/legacy.rs#L92)

`Project` as format v3 wrote it — before the camera. Same fields as
[`ProjectV2`](#struct-projectv2) but `layers: Vec<LayerV3>`.

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | As `Project::width` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `height` | `u32` | As `Project::height` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `fps` | `f32` | As `Project::fps` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `cells` | `Vec<Canvas>` | As `Project::cells` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `layers` | `Vec<LayerV3>` | As `Project::layers` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `frame_count` | `usize` | As `Project::frame_count` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `current_frame` | `usize` | As `Project::current_frame` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `current_layer` | `usize` | As `Project::current_layer` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_start` | `usize` | As `Project::loop_start` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_end` | `usize` | As `Project::loop_end` in the current format ([project.md](../doc/project.md)), at this version's position. |

### `struct LayerV4`
[src/io/legacy.rs:107](../../../src/io/legacy.rs#L107)

`Layer` as format v4 wrote it — before transform keys gained `ease`. Same fields as
[`LayerV3`](#struct-layerv3) plus:

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | As `Layer::name` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `opacity` | `f32` | As `Layer::opacity` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `visible` | `bool` | As `Layer::visible` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `locked` | `bool` | As `Layer::locked` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `reference` | `bool` | As `Layer::reference` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `exposures` | `Vec<Option<usize>>` | As `Layer::exposures` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `transform` | `Transform` | As `Layer::transform` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `transform_keys` | `Vec<TransformKeyV4>` | As `Layer::transform_keys` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `track_points` | `Vec<TrackSample>` | As `Layer::track_points` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `cell_w` | `u32` | As `Layer::cell_w` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `cell_h` | `u32` | As `Layer::cell_h` in the current format ([layer.md](../doc/layer.md)), at this version's position. |

### `struct ProjectV4`
[src/io/legacy.rs:129](../../../src/io/legacy.rs#L129)

`Project` as format v4 wrote it. The camera arrived in v4, so unlike v2/v3 this mirror
carries it through instead of defaulting it. Same fields as `ProjectV3` with
`layers: Vec<LayerV4>`, plus:

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | As `Project::width` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `height` | `u32` | As `Project::height` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `fps` | `f32` | As `Project::fps` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `cells` | `Vec<Canvas>` | As `Project::cells` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `layers` | `Vec<LayerV4>` | As `Project::layers` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `frame_count` | `usize` | As `Project::frame_count` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `current_frame` | `usize` | As `Project::current_frame` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `current_layer` | `usize` | As `Project::current_layer` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_start` | `usize` | As `Project::loop_start` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_end` | `usize` | As `Project::loop_end` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `camera` | `Camera` | Static / working camera (offset, zoom, roll). |
| `camera_keys` | `Vec<CameraKey>` | Camera keyframes. |

### `struct LayerV6`
[src/io/legacy.rs:293](../../../src/io/legacy.rs#L293)

`Layer` as formats v5 and v6 wrote it — before `clip` and `alpha_lock`. Same fields as
[`LayerV4`](#struct-layerv4) except `transform_keys` is the live `Vec<TransformKey>`
(which has `ease`).

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | As `Layer::name` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `opacity` | `f32` | As `Layer::opacity` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `visible` | `bool` | As `Layer::visible` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `locked` | `bool` | As `Layer::locked` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `reference` | `bool` | As `Layer::reference` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `exposures` | `Vec<Option<usize>>` | As `Layer::exposures` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `transform` | `Transform` | As `Layer::transform` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `transform_keys` | `Vec<TransformKey>` | As `Layer::transform_keys` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `track_points` | `Vec<TrackSample>` | As `Layer::track_points` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `cell_w` | `u32` | As `Layer::cell_w` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `cell_h` | `u32` | As `Layer::cell_h` in the current format ([layer.md](../doc/layer.md)), at this version's position. |

### `struct ProjectV6`
[src/io/legacy.rs:314](../../../src/io/legacy.rs#L314)

`Project` as formats v5 and v6 wrote it. Same fields as `ProjectV4` with
`layers: Vec<LayerV6>`. (v5 and v6 differ only in that v6's body is compressed.)

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | As `Project::width` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `height` | `u32` | As `Project::height` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `fps` | `f32` | As `Project::fps` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `cells` | `Vec<Canvas>` | As `Project::cells` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `layers` | `Vec<LayerV6>` | As `Project::layers` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `frame_count` | `usize` | As `Project::frame_count` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `current_frame` | `usize` | As `Project::current_frame` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `current_layer` | `usize` | As `Project::current_layer` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_start` | `usize` | As `Project::loop_start` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_end` | `usize` | As `Project::loop_end` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `camera` | `Camera` | As `Project::camera` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `camera_keys` | `Vec<CameraKey>` | As `Project::camera_keys` in the current format ([project.md](../doc/project.md)), at this version's position. |

### `struct LayerV7`
[src/io/legacy.rs:382](../../../src/io/legacy.rs#L382)

`Layer` as format v7 wrote it — before `kind`, `in_group` and `collapsed` (layer groups).
Same fields as [`LayerV6`](#struct-layerv6) plus:

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | As `Layer::name` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `opacity` | `f32` | As `Layer::opacity` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `visible` | `bool` | As `Layer::visible` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `locked` | `bool` | As `Layer::locked` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `reference` | `bool` | As `Layer::reference` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `exposures` | `Vec<Option<usize>>` | As `Layer::exposures` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `transform` | `Transform` | As `Layer::transform` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `transform_keys` | `Vec<TransformKey>` | As `Layer::transform_keys` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `track_points` | `Vec<TrackSample>` | As `Layer::track_points` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `cell_w` | `u32` | As `Layer::cell_w` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `cell_h` | `u32` | As `Layer::cell_h` in the current format ([layer.md](../doc/layer.md)), at this version's position. |
| `clip` | `bool` | Clipped to the layer below (only shows where its base has pixels). |
| `alpha_lock` | `bool` | Painting only recolours existing paint. |

### `struct ProjectV7`
[src/io/legacy.rs:407](../../../src/io/legacy.rs#L407)

`Project` as format v7 wrote it. Same fields as `ProjectV6` with `layers: Vec<LayerV7>`.

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | As `Project::width` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `height` | `u32` | As `Project::height` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `fps` | `f32` | As `Project::fps` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `cells` | `Vec<Canvas>` | As `Project::cells` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `layers` | `Vec<LayerV7>` | As `Project::layers` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `frame_count` | `usize` | As `Project::frame_count` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `current_frame` | `usize` | As `Project::current_frame` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `current_layer` | `usize` | As `Project::current_layer` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_start` | `usize` | As `Project::loop_start` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `loop_end` | `usize` | As `Project::loop_end` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `camera` | `Camera` | As `Project::camera` in the current format ([project.md](../doc/project.md)), at this version's position. |
| `camera_keys` | `Vec<CameraKey>` | As `Project::camera_keys` in the current format ([project.md](../doc/project.md)), at this version's position. |

## `impl From<TransformKeyV4> for TransformKey`

### `TransformKey::from`
[src/io/legacy.rs:28](../../../src/io/legacy.rs#L28)

```rust
fn from(k: TransformKeyV4) -> Self
```

Builds a live `TransformKey` from an old one: same `frame` and `transform`, and
`ease: Ease::Linear`. The comment says Linear "reproduces pre-v5 playback exactly".

**Used by:** [`keys`](#keys) (through `Into::into`).

## Free functions

### `keys`
[src/io/legacy.rs:38](../../../src/io/legacy.rs#L38)

```rust
fn keys(old: Vec<TransformKeyV4>) -> Vec<TransformKey>
```

Converts a whole list of old transform keys with the `From` impl above
(`old.into_iter().map(Into::into).collect()`).

**Used by:** the `LayerV2`, `LayerV3` and `LayerV4` conversions.

## `impl From<LayerV2> for Layer`

### `Layer::from`
[src/io/legacy.rs:147](../../../src/io/legacy.rs#L147)

```rust
fn from(l: LayerV2) -> Self
```

Builds a live `Layer` from a v2 layer: copies `name`, `opacity`, `visible`, `locked`,
`reference`, `exposures`, `transform`; converts `transform_keys` with
[`keys`](#keys); fills everything else as in the table above (no track
points, project-sized cells, fresh `uid`).

**Used by:** [`Project::from(ProjectV2)`](#impl-fromprojectv2-for-project).

## `impl From<ProjectV2> for Project`

### `Project::from`
[src/io/legacy.rs:175](../../../src/io/legacy.rs#L175)

```rust
fn from(p: ProjectV2) -> Self
```

Builds a live `Project` from a v2 file: copies size, fps, frame count, cursor and loop
range; wraps each cell in `Arc::new` (the live project shares cells copy-on-write);
converts each layer with [`Layer::from(LayerV2)`](#impl-fromlayerv2-for-layer); sets an identity
camera (`Camera::default()`), no camera keys and default `camera_opts`.

**Used by:** [project_file.rs](project_file.md) `decode_uncompressed` (version 2).

## `impl From<LayerV3> for Layer`

### `Layer::from`
[src/io/legacy.rs:195](../../../src/io/legacy.rs#L195)

```rust
fn from(l: LayerV3) -> Self
```

Like the v2 conversion, but keeps the file's `track_points`.

**Used by:** [`Project::from(ProjectV3)`](#impl-fromprojectv3-for-project).

## `impl From<LayerV4> for Layer`

### `Layer::from`
[src/io/legacy.rs:223](../../../src/io/legacy.rs#L223)

```rust
fn from(l: LayerV4) -> Self
```

Like the v3 conversion, but also keeps `cell_w` / `cell_h`. Transform keys still go
through [`keys`](#keys) because v4 keys have no `ease`.

**Used by:** [`Project::from(ProjectV4)`](#impl-fromprojectv4-for-project).

## `impl From<ProjectV4> for Project`

### `Project::from`
[src/io/legacy.rs:251](../../../src/io/legacy.rs#L251)

```rust
fn from(p: ProjectV4) -> Self
```

Like the v2 project conversion, but carries the file's `camera` and `camera_keys` (they
were added in v4). `camera_opts` is default.

**Used by:** [project_file.rs](project_file.md) `decode_uncompressed` (version 4).

## `impl From<ProjectV3> for Project`

### `Project::from`
[src/io/legacy.rs:271](../../../src/io/legacy.rs#L271)

```rust
fn from(p: ProjectV3) -> Self
```

Same as the v2 project conversion (identity camera, no camera keys), with layers
converted by [`Layer::from(LayerV3)`](#impl-fromlayerv3-for-layer).

**Used by:** [project_file.rs](project_file.md) `decode_uncompressed` (version 3).

## `impl From<LayerV6> for Layer`

### `Layer::from`
[src/io/legacy.rs:332](../../../src/io/legacy.rs#L332)

```rust
fn from(l: LayerV6) -> Self
```

Copies every field the v5/v6 layer has — including `transform_keys` directly, since they
already carry `ease` — and defaults the rest (`clip` and `alpha_lock` false, not in a
group, fresh `uid`).

**Used by:** [`Project::from(ProjectV6)`](#impl-fromprojectv6-for-project) and
[`Layer::from(LayerV7)`](#impl-fromlayerv7-for-layer).

## `impl From<ProjectV6> for Project`

### `Project::from`
[src/io/legacy.rs:360](../../../src/io/legacy.rs#L360)

```rust
fn from(p: ProjectV6) -> Self
```

Same shape as the v4 project conversion (camera carried), with layers converted by
[`Layer::from(LayerV6)`](#impl-fromlayerv6-for-layer).

**Used by:** [project_file.rs](project_file.md) `decode_from` (version 6) and
`decode_uncompressed` (version 5).

## `impl From<LayerV7> for Layer`

### `Layer::from`
[src/io/legacy.rs:425](../../../src/io/legacy.rs#L425)

```rust
fn from(l: LayerV7) -> Self
```

**What it does.** Converts a v7 layer by reusing the v6 conversion: it builds a
`LayerV6` from the shared fields, turns that into a `Layer` with `.into()`, and then sets
`clip` and `alpha_lock` from the v7 file using struct update syntax
(`Layer { clip, alpha_lock, ..v6_layer.into() }`). Group fields stay at their defaults,
so every v7 layer opens ungrouped.

**Used by:** [`Project::from(ProjectV7)`](#impl-fromprojectv7-for-project).

## `impl From<ProjectV7> for Project`

### `Project::from`
[src/io/legacy.rs:448](../../../src/io/legacy.rs#L448)

```rust
fn from(p: ProjectV7) -> Self
```

Same shape as the v6 project conversion, with layers converted by
[`Layer::from(LayerV7)`](#impl-fromlayerv7-for-layer).

**Used by:** [project_file.rs](project_file.md) `decode_from` (version 7).
