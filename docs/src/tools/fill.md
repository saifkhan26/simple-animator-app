# `src/tools/fill.rs` — flood fill, gap closing and fill painting

> **Source:** [src/tools/fill.rs](../../../src/tools/fill.rs) · **Module:** `crate::tools::fill` · **Size:** 3253 lines · Line links as of commit `67a7cb9`

## What this file is for

The **bucket fill**: click inside an area and it fills with the brush colour up to the lines around it. On
top of the classic algorithm it handles what animation line art needs:

- **Tolerance** — how different a pixel's colour may be and still be filled.
- **Lines on another layer** — the walls can come from a line-art layer (`boundary`) while paint goes on the
  colour layer.
- **Gap closing** — small breaks in the lines (up to `gap` pixels) count as closed, so the fill doesn't leak
  out through them.
- **Soft edges** — on bare canvas the fill reaches into the anti-aliased (and grainy) edges of lines so no
  white rim is left between line and colour.
- **Expand**, **opacity**, **alpha lock**, **behind** (fill under existing paint), **paper grain** in the fill,
  and staying inside a **selection**.
- **Enclose & fill** for the lasso mode: fill every closed shape inside a lasso.
- A press can be **dragged to retune** gap and expand live (`FillSession`).

### Flood fill in a tiny grid

```text
# = line, . = empty, click at C
# # # # # #          # # # # # #
# . . # . #          # F F # . #
# . C # . #   ──►    # F F # . #
# . . . . #          # F F F F #     (the gap at row 3 lets it flow right)
# # # # # #          # # # # # #
```

The same idea in JavaScript (breadth-first):
```js
function flood(grid, sx, sy, isWall) {
  const q = [[sx, sy]], seen = new Set([sx + "," + sy]);
  while (q.length) {
    const [x, y] = q.shift();
    grid[y][x] = "F";
    for (const [nx, ny] of [[x+1,y],[x-1,y],[x,y+1],[x,y-1]]) {
      const k = nx + "," + ny;
      if (!seen.has(k) && inBounds(nx, ny) && !isWall(nx, ny)) { seen.add(k); q.push([nx, ny]); }
    }
  }
}
```
The Rust version (`span_flood`) is a **scanline** flood: it fills whole horizontal runs at once and only queues
the runs above and below — much faster than one pixel at a time.

### Gap closing: distance field + watershed

To treat a 3-pixel break as closed, the fill first computes, for every pixel, the **distance to the nearest
wall** (`wall_distance`, an exact Euclidean distance transform). Pixels farther than half the gap from every
wall are "open space"; a gap narrower than the gap setting has no open space in it, so open regions separated
only by narrow gaps fall into separate pieces. Then a **watershed** (`gap_flood`) grows the clicked piece and
its neighbours outward at once, by increasing distance, until they meet — each in-between pixel goes to
whichever piece reaches it first. The fill keeps only what the clicked piece got.

```text
lines with a 2-px gap          distance > gap/2 (open)       watershed result
###########                    ...........                   ###########
#....  ...#                    .oooo  ooo.                   #FFFF  ...#
#....  ...#            ──►     .oooo  ooo.          ──►      #FFFF| ...#   ← split at the gap
###########                    ...........                   ###########
```

## How it connects

- **Used by:** [app.rs](../app.md) (Fill press/drag: `FillSession`, `fill_source`), [app/select.rs](../app/select.md)
  (`fill_selection` → `fill_masked`), [app/lasso_fill.rs](../app/lasso_fill.md) (`fill_masked`, `enclose_fill`),
  [tools/wand.rs](wand.md) (shares `wall_distance`, `gap_flood`, `soft_pixels`, ladders, rules).
- **Uses:** [tools/scan.rs](scan.md) (word scans), [tools/noise.rs](noise.md) (`Periodic` paper fields),
  [tools/lasso.rs](lasso.md) (`Mask`), [doc/canvas.rs](../doc/canvas.md).

## Rust concepts in this file

**Generic structs with a closure type parameter.** `struct Edges<'a, F>` stores a closure `theirs: F` and
borrowed slices; `impl<F: Fn(usize) -> bool> Edges<'_, F>` gives it methods for any such closure.
More: [Generics](../../rust-primer.md#generics)

**`Option<Option<Region>>`.** `edged` is "not computed yet" (`None`) vs "computed: there is none"
(`Some(None)`) vs "computed: here it is" (`Some(Some(r))`) — caching a result that may itself be absent.

**`VecDeque` buckets for a priority flood.** The watershed keeps one queue per distance level and always
takes from the lowest non-empty one. More: [Collections](../../rust-primer.md#collections)

**`f64` distance maths.** The distance transform works in squared distances and uses a large finite `FAR`
instead of infinity so `∞ − ∞` never appears.

## Types

### `const MAX_EXPAND`
[src/tools/fill.rs:48](../../../src/tools/fill.rs#L48)

```rust
pub const MAX_EXPAND: u8 = 32;
```

The largest expand radius (matches the UI slider).

### `const MAX_GAP`
[src/tools/fill.rs:50](../../../src/tools/fill.rs#L50)

```rust
pub const MAX_GAP: u8 = 255;
```

The largest gap setting (matches the UI slider).

### `const GRAIN_SIZES`
[src/tools/fill.rs:52](../../../src/tools/fill.rs#L52)

```rust
pub const GRAIN_SIZES: RangeInclusive<f32> = 0.25..=4.0;
```

The fill-grain sizes the UI offers (1 = Pencil-5 Tilted's own paper scale).

### `const GRAIN_CONTRASTS`
[src/tools/fill.rs:54](../../../src/tools/fill.rs#L54)

```rust
pub const GRAIN_CONTRASTS: RangeInclusive<f32> = 0.25..=4.0;
```

The fill-grain contrasts the UI offers (1 = the paper as scanned).

### `const GAP_LADDER`
[src/tools/fill.rs:59](../../../src/tools/fill.rs#L59)

```rust
const GAP_LADDER: [u8; 37] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 28, 32, 36, 40, 48, 56,
    64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 255,
];
```

The gap values a press-drag steps through (every pixel while small, wider strides as they grow).

### `struct FillOptions`
[src/tools/fill.rs:65](../../../src/tools/fill.rs#L65)

Everything a fill press needs to know: tolerance, colour, expand, gap, alpha lock, opacity, grain.

| Field | Type | Meaning |
|---|---|---|
| `tolerance` | `u8` | 0..=255 per-channel tolerance. |
| `color` | `[u8; 4]` | Fill colour (RGBA8 unmultiplied). |
| `expand` | `u8` | Grow the filled region by this many pixels after the flood so the colour tucks under anti-aliased lines. 0 = off. |
| `gap` | `u8` | Treat breaks in the lines up to this many pixels wide as shut. 0 = off. |
| `alpha_lock` | `bool` | Recolour only pixels that already have paint, keeping their alpha. |
| `opacity` | `f32` | 0..=1. Below 1 the fill goes on over what is there, the way a stroke at that opacity does, instead of replacing it. |
| `grain` | `Grain` | Paper showing through the fill. `Grain::NONE` for flat colour. |

### `struct Grain`
[src/tools/fill.rs:93](../../../src/tools/fill.rs#L93)

Paper grain on a fill: the paper scan Krita's Pencil-5 Tilted draws on thins the fill where the paper dips, the way a pencil's tooth does, so whatever is under the fill shows through the specks.

The paper is anchored to the cell's pixels, as the brushes' grain is: it stays put whatever shape is filled, every fill of a drawing shares one sheet, and at size 1 with no offset its specks fall where a Pencil-5 stroke's do.

| Field | Type | Meaning |
|---|---|---|
| `amount` | `f32` | 0..=1. 0 is flat colour; 1 leaves the paper's deepest valleys bare. |
| `size` | `f32` | Cell pixels per texel of the paper at Pencil-5's own scale. Larger magnifies the tooth. |
| `contrast` | `f32` | Krita's texture contrast, pivoting on mid-grey: 1 is the paper as scanned, above it the tooth goes harder, below it softer. |
| `offset` | `(i32, i32)` | Where the sheet sits, in cell pixels. |

### `const Grain::NONE`
[src/tools/fill.rs:108](../../../src/tools/fill.rs#L108)

```rust
pub const NONE: Self = Self {
    amount: 0.0,
    size: 1.0,
    contrast: 1.0,
    offset: (0, 0),
};
```

No grain: flat colour.

### `struct Tooth`
[src/tools/fill.rs:187](../../../src/tools/fill.rs#L187)

A `Grain` baked: one period of coverage factors (see `Grain::bake`), laid on the cell at `offset`.

| Field | Type | Meaning |
|---|---|---|
| `factors` | `Periodic` | One period of per-pixel coverage multipliers. |
| `offset` | `(i32, i32)` | Where the sheet sits on the cell. |

### `type Bbox`
[src/tools/fill.rs:200](../../../src/tools/fill.rs#L200)

```rust
pub(super) type Bbox = (i32, i32, i32, i32);
```

Inclusive pixel bounds `(min_x, min_y, max_x, max_y)`.

### `struct Region`
[src/tools/fill.rs:226](../../../src/tools/fill.rs#L226)

The flood's reach at one gap setting: a `w * h` bitmap and its bounds.

| Field | Type | Meaning |
|---|---|---|
| `gap` | `u8` | The gap setting this region was computed for. |
| `mask` | `Vec<bool>` | One bool per cell pixel: in the fill or not. |
| `bbox` | `Bbox` | The region's bounds. |

### `struct Field`
[src/tools/fill.rs:237](../../../src/tools/fill.rs#L237)

Squared distance to the nearest wall over a window of the cell, 0 on the walls. The window is the plain flood's bounds grown by `reach`, which puts every wall close enough to matter for a gap up to `2 * (reach - 1)` inside it; a gap fill never reaches past the plain flood, so nothing outside is needed.

| Field | Type | Meaning |
|---|---|---|
| `x0` | `i32` | Window left, in cell pixels. |
| `y0` | `i32` | Window top. |
| `w` | `i32` | Window width. |
| `h` | `i32` | Window height. |
| `reach` | `i32` | How far the window extends past the plain flood (bounds the largest gap it serves). |
| `whole` | `bool` | The window is the whole cell, so it serves any gap. |
| `dist` | `Vec<f32>` | Squared distance to the nearest wall per window pixel. |

### `struct FillSession`
[src/tools/fill.rs:251](../../../src/tools/fill.rs#L251)

One press of the bucket. Built at pen-down; `apply` paints it, and may be called again as a drag changes the gap or the expand — each call puts back what the last one painted first.

| Field | Type | Meaning |
|---|---|---|
| `w` | `i32` | Cell width. |
| `h` | `i32` | Cell height. |
| `seed` | `(i32, i32)` | The pressed pixel. |
| `target` | `[u8; 4]` | The colour under the press (what "similar" is measured from). |
| `tolerance` | `u8` | Per-channel tolerance. |
| `color` | `[u8; 4]` | Fill colour. |
| `alpha_lock` | `bool` | Only recolour existing paint. |
| `opacity` | `f32` | Fill opacity. |
| `grain` | `Grain` | Paper grain settings. |
| `boundary` | `Option<Vec<u8>>` | Lines from another layer, already in the cell's pixels — every pixel, since the flood reads them one by one all press long. `None` reads the cell's own pixels as they were before the press. |
| `clip` | `Option<Arc<Mask>>` | The selection mask, if any. |
| `inert` | `bool` | A same-layer refill with the colour already there: nothing to do. |
| `plain` | `Option<Region>` | The plain flood, gap 0. Every gap region lies inside it. Over bare canvas it holds only clear pixels, short of the soft edges of its lines. |
| `edged` | `Option<Option<Region>>` | Over bare canvas, once worked out, `plain` with the soft edges of its lines: what a press with no gap paints. `None` inside when there are none. |
| `field` | `Option<Field>` | Built the first time a gap is asked for, then kept for the press. |
| `region` | `Option<Region>` | The last gap region worked out, for a gap above 0. |
| `written` | `Option<Bbox>` | Bounds of what the last `apply` painted. |
| `touched` | `Option<DirtyRect>` | Everything any `apply` of this press changed, for the undo record. |

### `struct Ink`
[src/tools/fill.rs:547](../../../src/tools/fill.rs#L547)

What a fill puts down, and how.

| Field | Type | Meaning |
|---|---|---|
| `color` | `[u8; 4]` | Fill colour (straight RGBA). |
| `opacity` | `f32` | 0..=1, scaling every pixel's coverage. |
| `alpha_lock` | `bool` | Only change pixels that already have paint. |
| `behind` | `bool` | Go under the paint already there, as if on a layer below: bare pixels fill, line art keeps its colour, and its soft edges get the fill behind them. Nothing at all under an alpha lock, which forbids the only thing behind does — adding paint where there is none. |
| `tooth` | `Option<Tooth>` | Paper grain thinning the coverage, when there is any. |

### `const FREE`
[src/tools/fill.rs:914](../../../src/tools/fill.rs#L914)

```rust
const FREE: u8 = 0;
```

Who a pixel belongs to while `gap_flood` runs.

### `const OURS`
[src/tools/fill.rs:915](../../../src/tools/fill.rs#L915)

```rust
const OURS: u8 = 1;
```

`gap_flood` ownership: claimed by the clicked piece.

### `const THEIRS`
[src/tools/fill.rs:916](../../../src/tools/fill.rs#L916)

```rust
const THEIRS: u8 = 2;
```

`gap_flood` ownership: claimed by some other piece.

### `const OUT`
[src/tools/fill.rs:918](../../../src/tools/fill.rs#L918)

```rust
const OUT: u8 = 3;
```

A wall, or a pixel the plain flood never reached: nobody's.

### `const SOFT_EDGE`
[src/tools/fill.rs:1117](../../../src/tools/fill.rs#L1117)

```rust
pub(super) const SOFT_EDGE: u8 = 128;
```

How covered a pixel over bare canvas can be and still be the soft edge of a line rather than its body: half. A fill over bare canvas reaches this far into its lines, so it meets them without a gap.

### `const SPECK`
[src/tools/fill.rs:1122](../../../src/tools/fill.rs#L1122)

```rust
const SPECK: usize = 16;
```

The most pixels a clear patch walled in by a line's soft edge can have and still be a speck of its grain or noise rather than a place of its own. A speck goes with the edge round it.

### `const RIDGE_BLUR`
[src/tools/fill.rs:1126](../../../src/tools/fill.rs#L1126)

```rust
const RIDGE_BLUR: i32 = 4;
```

How far round each soft pixel `soft_edges` averages the coverage it floods by.

### `struct Cover`
[src/tools/fill.rs:1130](../../../src/tools/fill.rs#L1130)

How covered each pixel of a cell is: a byte every `stride` of `px`.

| Field | Type | Meaning |
|---|---|---|
| `px` | `&'a [u8]` | The pixel bytes. |
| `stride` | `usize` | Bytes per pixel (4 for RGBA alpha, 1 for a plain byte map). |

### `const UNSEEN`
[src/tools/fill.rs:1325](../../../src/tools/fill.rs#L1325)

```rust
const UNSEEN: u8 = 0;
```

`soft_edges` label: not looked at yet.

### `const SOFT`
[src/tools/fill.rs:1327](../../../src/tools/fill.rs#L1327)

```rust
const SOFT: u8 = 1;
```

Soft edge, or a speck in it, not yet shared out.

### `const OTHER`
[src/tools/fill.rs:1329](../../../src/tools/fill.rs#L1329)

```rust
const OTHER: u8 = 2;
```

Clear and someone else's, or soft and gone to them.

### `const MINE`
[src/tools/fill.rs:1331](../../../src/tools/fill.rs#L1331)

```rust
const MINE: u8 = 3;
```

Soft and gone to the fill.

### `const BODY`
[src/tools/fill.rs:1333](../../../src/tools/fill.rs#L1333)

```rust
const BODY: u8 = 4;
```

Too covered to be an edge.

### `const PATCH`
[src/tools/fill.rs:1335](../../../src/tools/fill.rs#L1335)

```rust
const PATCH: u8 = 5;
```

Clear, in a patch still being sized up.

### `struct Edges`
[src/tools/fill.rs:1338](../../../src/tools/fill.rs#L1338)

`soft_edges` at work.

| Field | Type | Meaning |
|---|---|---|
| `cover` | `Cover<'a>` | How covered each pixel is. |
| `w` | `i32` | Cell width. |
| `h` | `i32` | Cell height. |
| `lo` | `u8` | The most coverage a pixel may have and still be "clear". |
| `ours` | `&'a [u8]` | The fill: its pixels hold `mine`. |
| `mine` | `u8` | The value marking fill pixels in `ours`. |
| `theirs` | `F` | Whether a clear pixel belongs to some other area. |
| `label` | `Vec<u8>` | Per-pixel label (`UNSEEN`, `SOFT`, `MINE`, …). |
| `soft` | `Vec<u32>` | Every pixel labelled `SOFT`, in the order found. |
| `todo` | `Vec<u32>` | Soft pixels whose neighbours are still to look at. |
| `patch` | `Vec<u32>` | The clear patch being sized up. |
| `shared` | `bool` | Some soft pixel touches a clear place that isn't the fill's. |

### `const FAR`
[src/tools/fill.rs:1485](../../../src/tools/fill.rs#L1485)

```rust
const FAR: f64 = 1e30;
```

Squared distance of a parabola that never touches down — a line with no wall on it. Finite, so the arithmetic below never meets `inf - inf`.

### `const TURN`
[src/tools/fill.rs:1577](../../../src/tools/fill.rs#L1577)

```rust
const TURN: usize = 16;
```

Rows or columns turned at a time by `wall_distance`.

### `struct EdtScratch`
[src/tools/fill.rs:1580](../../../src/tools/fill.rs#L1580)

Reusable buffers for one line of the distance transform.

| Field | Type | Meaning |
|---|---|---|
| `f` | `Vec<f64>` | Input: squared distances along the line before this pass. |
| `d` | `Vec<f64>` | Output distances. |
| `v` | `Vec<usize>` | Indexes of parabolas in the lower envelope. |
| `z` | `Vec<f64>` | Boundaries between neighbouring envelope parabolas. |

### `struct Grown`
[src/tools/fill.rs:1641](../../../src/tools/fill.rs#L1641)

A grown region: a window of the canvas and its bits.

| Field | Type | Meaning |
|---|---|---|
| `x0` | `i32` | Window left. |
| `y0` | `i32` | Window top. |
| `w` | `i32` | Window width. |
| `h` | `i32` | Window height. |
| `bits` | `Vec<bool>` | In-region flag per window pixel. |

## `impl Grain`

### `Grain::boiled`
[src/tools/fill.rs:119](../../../src/tools/fill.rs#L119)

```rust
pub fn boiled(self, id: usize) -> Self
```

The same grain on drawing `id`'s own patch of paper: every fill of that drawing lands on the same patch, and each drawing on a different one, the way each drawing of a hand-drawn scene gets its own sheet.

### `Grain::tooth`
[src/tools/fill.rs:134](../../../src/tools/fill.rs#L134)

```rust
fn tooth(self) -> Option<Tooth>
```

The grain baked for the per-pixel loops, or `None` when it adds nothing. The last grain baked is kept, so a drag that re-applies a fill, or the next fill at the same settings, doesn't bake again.

### `Grain::bake`
[src/tools/fill.rs:163](../../../src/tools/fill.rs#L163)

```rust
fn bake(self) -> Periodic
```

One period of the paper at this size, as what each pixel's coverage is multiplied by: 1 where the paper peaks, down to `1 - amount` in its valleys.

Size 1 reads the texels as they are, the way Pencil-5's dabs do; any other size is `Periodic::scaled`.

## `impl Tooth`

### `Tooth::row`
[src/tools/fill.rs:194](../../../src/tools/fill.rs#L194)

```rust
fn row(&self, x: i32, y: i32) -> PeriodicRow<'_>
```

The factors along row `y`, starting at column `x`.

## Free functions

### `step_gap`
[src/tools/fill.rs:205](../../../src/tools/fill.rs#L205)

```rust
pub fn step_gap(base: u8, steps: i32) -> u8
```

The gap `steps` rungs of `GAP_LADDER` away from `base`. No steps keeps `base` as it is, even off the ladder; the first step lands on the next rung in that direction.

### `step_ladder`
[src/tools/fill.rs:211](../../../src/tools/fill.rs#L211)

```rust
pub(super) fn step_ladder(ladder: &[u8], base: u8, steps: i32) -> u8
```

The value `steps` rungs of `ladder` away from `base`, as `step_gap` counts them.

## `impl FillSession`

### `FillSession::new`
[src/tools/fill.rs:296](../../../src/tools/fill.rs#L296)

```rust
pub fn new(
    pre: &[u8],
    size: (u32, u32),
    seed: (i32, i32),
    opts: &FillOptions,
    boundary: Option<Canvas>,
    clip: Option<Arc<Mask>>,
) -> Option<Self>
```

Starts a bucket press at `seed`: records the target colour and options, reads walls from `boundary` (lines on
another layer, converted to this cell's pixels) or from the cell itself, notes the selection, and marks the
press inert if it would just refill a same-colour area. `None` off the cell. Regions are computed lazily.

### `FillSession::apply`
[src/tools/fill.rs:356](../../../src/tools/fill.rs#L356)

```rust
pub fn apply(
    &mut self,
    canvas: &mut Canvas,
    pre: &[u8],
    gap: u8,
    expand: u8,
) -> Option<DirtyRect>
```

Paints the fill for `gap` and `expand`: first restores what the previous `apply` painted (from `pre`), then
computes (or reuses) the region — plain flood for gap 0, gap region otherwise, plus soft edges on bare canvas —
grows it by `expand`, and paints it with the session's ink. Returns the changed pixels (old and new bounds).
Called again each time a drag changes the gap or expand.

### `FillSession::revert`
[src/tools/fill.rs:404](../../../src/tools/fill.rs#L404)

```rust
pub fn revert(&mut self, canvas: &mut Canvas, pre: &[u8]) -> Option<DirtyRect>
```

Put back what the last `apply` painted, as if the press never happened. Returns the pixels restored, also marked dirty.

### `FillSession::touched`
[src/tools/fill.rs:416](../../../src/tools/fill.rs#L416)

```rust
pub fn touched(&self) -> Option<DirtyRect>
```

Everything this press has changed so far, over every `apply`.

### `FillSession::fits`
[src/tools/fill.rs:420](../../../src/tools/fill.rs#L420)

```rust
fn fits(&self, canvas: &Canvas, pre: &[u8]) -> bool
```

Whether `canvas` and `pre` still have the size this session was built for (a safety check).

### `FillSession::ensure_regions`
[src/tools/fill.rs:428](../../../src/tools/fill.rs#L428)

```rust
fn ensure_regions(&mut self, pre: &[u8], gap: u8)
```

Computes the plain flood (and soft edges, and the gap region for `gap > 0` via `wall_distance` and
`gap_flood`) unless they are already cached for this press.

## Free functions

### `flood`
[src/tools/fill.rs:481](../../../src/tools/fill.rs#L481)

```rust
pub fn flood(canvas: &mut Canvas, boundary: Option<&Canvas>, x: i32, y: i32, opts: FillOptions)
```

A one-shot bucket fill at `(x, y)` (no drag): builds a session and applies it once.

### `flood_clipped`
[src/tools/fill.rs:489](../../../src/tools/fill.rs#L489)

```rust
pub fn flood_clipped(
    canvas: &mut Canvas,
    boundary: Option<&Canvas>,
    x: i32,
    y: i32,
    opts: FillOptions,
    clip: Option<&Mask>,
)
```

`flood` limited to a selection: only covered pixels change, partly covered ones blend by coverage, a click
outside the selection does nothing.

### `fill_masked`
[src/tools/fill.rs:512](../../../src/tools/fill.rs#L512)

```rust
pub fn fill_masked(
    canvas: &mut Canvas,
    mask: &Mask,
    color: [u8; 4],
    opacity: f32,
    alpha_lock: bool,
    behind: bool,
    grain: Grain,
) -> bool
```

Fill everything `mask` covers with `color` at `opacity` — Fill selection, and the lasso fill. Fully covered pixels are replaced at full opacity, as the bucket does; a feathered edge or a lower opacity blends. With `alpha_lock`, only pixels that already have paint change, and keep their alpha. With `behind`, the fill goes under what is there instead (see `Ink::behind`). `grain` thins it where the paper dips. Returns whether anything was inside the canvas to fill.

### `paint`
[src/tools/fill.rs:563](../../../src/tools/fill.rs#L563)

```rust
fn paint(
    canvas: &mut Canvas,
    bbox: Bbox,
    inside: impl Fn(i32, i32) -> bool,
    ink: &Ink,
    clip: Option<&Mask>,
)
```

Write `ink` over every pixel of `bbox` that `inside` says belongs to the fill, held to `clip` when there is one.

### `put`
[src/tools/fill.rs:587](../../../src/tools/fill.rs#L587)

```rust
fn put(canvas: &mut Canvas, x: i32, y: i32, ink: &Ink, k: u8, grain: f32)
```

Fill one pixel with `ink` at coverage `k`, scaled by its opacity and by `grain`, the paper's factor there (1 for none): replaced when fully covered at full opacity, otherwise blended toward the colour — over what is there, as a stroke at that opacity lands. With `alpha_lock` only the colour moves: the pixel's alpha stays, and a bare pixel stays bare.

### `put_behind`
[src/tools/fill.rs:615](../../../src/tools/fill.rs#L615)

```rust
fn put_behind(canvas: &mut Canvas, x: i32, y: i32, color: [u8; 4], t: f32)
```

`color` at strength `t` slid under pixel `(x, y)`: what is there composited over it.

### `enclose_fill`
[src/tools/fill.rs:651](../../../src/tools/fill.rs#L651)

```rust
pub fn enclose_fill(
    canvas: &mut Canvas,
    pre: &[u8],
    boundary: Option<&[u8]>,
    lasso: &Mask,
    opts: &FillOptions,
    behind: bool,
    clip: Option<&Mask>,
) -> Option<DirtyRect>
```

"Enclose & fill": fills every area inside the lasso that lines completely shut in, and nothing the lasso merely
cuts across — lasso round a drawing and each of its closed shapes fills while the space between drawing and
lasso stays bare. It floods the outside from the lasso's border inward; whatever stays unreached and is not line
is enclosed, and gets filled (with soft edges).

### `restore`
[src/tools/fill.rs:769](../../../src/tools/fill.rs#L769)

```rust
fn restore(canvas: &mut Canvas, pre: &[u8], b: Bbox)
```

Copy `bbox` of `pre` back over `canvas`, row by row.

### `mark`
[src/tools/fill.rs:779](../../../src/tools/fill.rs#L779)

```rust
fn mark(canvas: &mut Canvas, r: DirtyRect)
```

Marks rectangle `r` dirty on the canvas (so its texture is re-uploaded).

### `to_rect`
[src/tools/fill.rs:783](../../../src/tools/fill.rs#L783)

```rust
fn to_rect(b: Bbox) -> DirtyRect
```

Converts an inclusive `Bbox` to an exclusive-end `DirtyRect`.

### `join`
[src/tools/fill.rs:792](../../../src/tools/fill.rs#L792)

```rust
fn join(a: Bbox, b: Bbox) -> Bbox
```

The bounding box of two boxes.

### `blend_toward`
[src/tools/fill.rs:799](../../../src/tools/fill.rs#L799)

```rust
fn blend_toward(canvas: &mut Canvas, x: i32, y: i32, color: [u8; 4], t: f32)
```

Move a pixel `t` of the way toward `color`, lerping premultiplied so a half-covered pixel over transparency keeps the fill's own colour rather than going muddy.

### `span_flood`
[src/tools/fill.rs:839](../../../src/tools/fill.rs#L839)

```rust
pub(super) fn span_flood(
    w: i32,
    h: i32,
    (x, y): (i32, i32),
    pass: impl Fn(i32, i32) -> bool,
) -> (Vec<bool>, Bbox)
```

The scanline flood: from `(x, y)`, fills each horizontal run of pixels `pass` accepts, then scans the rows
above and below that run for new runs to push. Returns the reached bitmap and its bounds.

### `neighbours`
[src/tools/fill.rs:907](../../../src/tools/fill.rs#L907)

```rust
fn neighbours(x: i32, y: i32, w: i32, h: i32) -> impl Iterator<Item = (i32, i32)>
```

The in-bounds 4-neighbours of `(x, y)`.

### `gap_flood`
[src/tools/fill.rs:932](../../../src/tools/fill.rs#L932)

```rust
pub(super) fn gap_flood(
    field: &Field,
    plain: &[bool],
    cw: i32,
    ch: i32,
    seed: (i32, i32),
    gap: u8,
) -> (Vec<bool>, Bbox)
```

The gap-closing watershed with markers (see the intro): open space beyond half the gap from every wall splits
into pieces; the clicked piece and all the others grow outward at once by distance level; each contested pixel
goes to whoever reaches it first. Returns the clicked piece's final region.

## `impl<'a> Cover<'a>`

### `Cover::rgba`
[src/tools/fill.rs:1137](../../../src/tools/fill.rs#L1137)

```rust
pub(super) fn rgba(src: &'a [u8]) -> Self
```

The alpha of RGBA8 pixels.

### `Cover::bytes`
[src/tools/fill.rs:1142](../../../src/tools/fill.rs#L1142)

```rust
pub(super) fn bytes(px: &'a [u8]) -> Self
```

A byte a pixel.

### `Cover::at`
[src/tools/fill.rs:1146](../../../src/tools/fill.rs#L1146)

```rust
fn at(self, i: usize) -> u8
```

The coverage byte of pixel `i`.

### `Cover::sum`
[src/tools/fill.rs:1151](../../../src/tools/fill.rs#L1151)

```rust
fn sum(self, i0: usize, i1: usize) -> u32
```

The coverage of pixels `i0..=i1`, added up.

## Free functions

### `soft_edges`
[src/tools/fill.rs:1176](../../../src/tools/fill.rs#L1176)

```rust
fn soft_edges(
    src: &[u8],
    w: i32,
    h: i32,
    mut ours: Vec<bool>,
    bbox: Bbox,
    lo: u8,
    theirs: impl Fn(usize) -> bool,
) -> (Vec<bool>, Bbox)
```

Grows a fill over bare canvas into the soft (partly covered) edges of its lines, so colour meets the line with no
gap. Grainy pencil lines are full of under-half-covered pixels, so the flood itself only takes clear pixels; this
step then shares the soft pixels between the fill and neighbouring areas by a watershed on blurred coverage (the
line's ridge), and lets tiny clear specks inside a line go with the edge around them.

### `soft_pixels`
[src/tools/fill.rs:1193](../../../src/tools/fill.rs#L1193)

```rust
pub(super) fn soft_pixels(
    cover: Cover,
    w: i32,
    h: i32,
    ours: &[u8],
    mine: u8,
    bbox: Bbox,
    lo: u8,
    theirs: impl Fn(usize) -> bool,
) -> Vec<u32>
```

The soft pixels `soft_edges` gives a fill. Here the fill is the pixels of `ours` that hold `mine`, and `cover` says how covered each pixel is.

### `take`
[src/tools/fill.rs:1239](../../../src/tools/fill.rs#L1239)

```rust
pub(super) fn take(mask: &mut [bool], w: i32, bbox: Bbox, px: &[u32]) -> Bbox
```

Add pixels `px` to `mask`, a `w` wide cell's, whose bounds were `bbox`. Returns the new bounds.

### `rim`
[src/tools/fill.rs:1253](../../../src/tools/fill.rs#L1253)

```rust
fn rim(map: &[u8], mine: u8, w: i32, h: i32, bbox: Bbox) -> Vec<u32>
```

The pixels just outside a region (beside one of its pixels), found a row at a time with word compares so the
inside of a big fill costs little.

### `only_in`
[src/tools/fill.rs:1287](../../../src/tools/fill.rs#L1287)

```rust
fn only_in(a: &[u8], b: &[u8], v: u8, mut hit: impl FnMut(usize))
```

Calls `hit(k)` for each index where `a[k] == v` and `b[k] != v`, skipping equal stretches eight bytes at a time.

### `bytes_eq`
[src/tools/fill.rs:1316](../../../src/tools/fill.rs#L1316)

```rust
fn bytes_eq(x: u64, v: u8) -> u64
```

The top bit of each byte of `x` that equals `v` (a word trick: XOR with `v` repeated, then detect zero bytes).

## `impl<F: Fn(usize) -> bool> Edges<'_, F>`

### `Edges::ours`
[src/tools/fill.rs:1359](../../../src/tools/fill.rs#L1359)

```rust
fn ours(&self, i: usize) -> bool
```

Whether pixel `i` belongs to the fill.

### `Edges::level`
[src/tools/fill.rs:1366](../../../src/tools/fill.rs#L1366)

```rust
fn level(&self, i: usize) -> usize
```

How covered the neighbourhood of `i` is, on average: what the watershed floods by. A grainy line's own pixels jump about too much to find its ridge by, and the flood would race along every dip.

### `Edges::look`
[src/tools/fill.rs:1377](../../../src/tools/fill.rs#L1377)

```rust
fn look(&mut self, x: i32, y: i32)
```

Sort out the pixel at `(x, y)`, beside the fill or a soft pixel.

### `Edges::share`
[src/tools/fill.rs:1432](../../../src/tools/fill.rs#L1432)

```rust
fn share(&mut self)
```

The watershed: hand every soft pixel to the fill or to the clear places beside it, flooding up from them all at once by coverage.

### `Edges::claim`
[src/tools/fill.rs:1468](../../../src/tools/fill.rs#L1468)

```rust
fn claim(&mut self, buckets: &mut [VecDeque<u32>], i: u32, from: u8) -> usize
```

Give the soft neighbours of `i` to `from`, queued by their `level`. Returns the lowest level queued.

## Free functions

### `wall_distance`
[src/tools/fill.rs:1497](../../../src/tools/fill.rs#L1497)

```rust
pub(super) fn wall_distance(
    w: i32,
    h: i32,
    bbox: Bbox,
    reach: i32,
    pass: impl Fn(i32, i32) -> bool + Sync,
) -> Field
```

Builds the `Field` for a plain flood: exact squared Euclidean distance from each pixel to the nearest wall
(and the cell edge counts as a wall), over a window of the flood's bounds grown by `reach`. Uses the separable
distance transform (`edt_line` on every column, then every row) on several threads.

## `impl EdtScratch`

### `EdtScratch::new`
[src/tools/fill.rs:1588](../../../src/tools/fill.rs#L1588)

```rust
fn new(n: usize) -> Self
```

Scratch buffers for lines of length `n`.

### `EdtScratch::run`
[src/tools/fill.rs:1597](../../../src/tools/fill.rs#L1597)

```rust
fn run(&mut self)
```

Runs `edt_line` on the buffers.

## Free functions

### `edt_line`
[src/tools/fill.rs:1605](../../../src/tools/fill.rs#L1605)

```rust
fn edt_line(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64])
```

One line of the exact squared distance transform (Felzenszwalb–Huttenlocher): `d[q] = min over p of
(q − p)² + f[p]`, computed in linear time from the lower envelope of the parabolas `(q − p)² + f[p]`.

## `impl Grown`

### `Grown::at`
[src/tools/fill.rs:1650](../../../src/tools/fill.rs#L1650)

```rust
pub(super) fn at(&self, x: i32, y: i32) -> bool
```

Whether pixel `(x, y)` is in the grown region (false outside the window).

### `Grown::bbox`
[src/tools/fill.rs:1654](../../../src/tools/fill.rs#L1654)

```rust
pub(super) fn bbox(&self) -> Bbox
```

The grown region's window as a `Bbox`.

## Free functions

### `dilate_window`
[src/tools/fill.rs:1663](../../../src/tools/fill.rs#L1663)

```rust
pub(super) fn dilate_window(mask: &[bool], w: i32, h: i32, bbox: Bbox, r: i32) -> Grown
```

`mask` grown by `r` pixels with a square kernel, clamped to the canvas — the same pixels as a plain separable dilate of the whole canvas, but worked over the region's bounds grown by `r` only, with running counts, so the cost depends on neither `r` nor the canvas size.

### `read_px`
[src/tools/fill.rs:1706](../../../src/tools/fill.rs#L1706)

```rust
fn read_px(canvas: &Canvas, x: i32, y: i32) -> [u8; 4]
```

Reads pixel `(x, y)` of a canvas as RGBA.

### `src_px`
[src/tools/fill.rs:1712](../../../src/tools/fill.rs#L1712)

```rust
pub(super) fn src_px(src: &[u8], w: i32, x: i32, y: i32) -> [u8; 4]
```

Pixel `(x, y)` of an RGBA8 buffer `w` pixels wide.

### `write_px`
[src/tools/fill.rs:1718](../../../src/tools/fill.rs#L1718)

```rust
fn write_px(canvas: &mut Canvas, x: i32, y: i32, p: [u8; 4])
```

Writes pixel `(x, y)` of a canvas.

### `over_bare`
[src/tools/fill.rs:1726](../../../src/tools/fill.rs#L1726)

```rust
fn over_bare(target: [u8; 4]) -> bool
```

Whether a fill started on `target` is over bare canvas, where only how covered a pixel is matters, not its colour.

### `is_clear`
[src/tools/fill.rs:1735](../../../src/tools/fill.rs#L1735)

```rust
fn is_clear(src: &[u8], w: i32, x: i32, y: i32, target: [u8; 4], tol: u8) -> bool
```

Whether a fill started on `target` floods into the pixel at `(x, y)`: as `matches_target`, but over bare canvas only a pixel no more than `tol` more covered than `target` is clear. Paint past that is a line, and the fill reaches into its soft edge through `soft_edges` instead.

### `matches_target`
[src/tools/fill.rs:1746](../../../src/tools/fill.rs#L1746)

```rust
pub(super) fn matches_target(src: &[u8], w: i32, x: i32, y: i32, target: [u8; 4], tol: u8) -> bool
```

Whether the pixel at `(x, y)` is within `tol` of `target` on every channel, or over bare canvas, at most half covered whatever its colour — the rule the wand picks by. The bucket floods by `is_clear`.

### `px_eq`
[src/tools/fill.rs:1761](../../../src/tools/fill.rs#L1761)

```rust
fn px_eq(a: [u8; 4], b: [u8; 4]) -> bool
```

Whether two RGBA pixels are identical.

## Tests

### `mod tests`

Run with `cargo test tools::fill::tests` · 50 tests

| Test | What it checks |
|---|---|
| [`the_fast_gap_matches_the_first_one_exactly`](../../../src/tools/fill.rs#L1996) | The threaded wall distance and the band-side watershed against the first ones, over random broken line art, presses and gaps — including fields big enough to be split between threads. |
| [`enclose_fills_only_what_the_lines_shut_in`](../../../src/tools/fill.rs#L2068) | Enclose fills only what the lines shut in. |
| [`enclose_counts_the_canvas_edge_as_a_line`](../../../src/tools/fill.rs#L2089) | Two strokes across a corner make a shape with the canvas's edges. |
| [`enclose_leaves_a_shape_the_lasso_cuts_through`](../../../src/tools/fill.rs#L2111) | Enclose leaves a shape the lasso cuts through. |
| [`enclose_reads_another_layer_s_lines_and_keeps_to_the_selection`](../../../src/tools/fill.rs#L2121) | Enclose reads another layer s lines and keeps to the selection. |
| [`a_fill_behind_keeps_the_lines_and_fills_round_them`](../../../src/tools/fill.rs#L2134) | A fill behind keeps the lines and fills round them. |
| [`masks_intersect_by_coverage`](../../../src/tools/fill.rs#L2155) | Masks intersect by coverage. |
| [`grain_lets_the_paper_show_through_the_fill`](../../../src/tools/fill.rs#L2185) | Grain lets the paper show through the fill. |
| [`grain_at_size_one_lines_up_with_pencil_5s_paper`](../../../src/tools/fill.rs#L2206) | At size 1 with no offset a fill's specks are the Pencil-5 paper's texels, where that pencil's dabs read them, so the two line up. |
| [`every_fill_lands_on_the_same_paper`](../../../src/tools/fill.rs#L2222) | The paper belongs to the canvas, not to the gesture: every way of filling the same pixels gives the same specks. |
| [`grain_under_an_alpha_lock_mottles_the_colour_and_keeps_the_alpha`](../../../src/tools/fill.rs#L2252) | Grain under an alpha lock mottles the colour and keeps the alpha. |
| [`grain_size_and_contrast_shape_the_tooth`](../../../src/tools/fill.rs#L2265) | Grain size and contrast shape the tooth. |
| [`boiling_gives_each_drawing_its_own_patch_of_paper`](../../../src/tools/fill.rs#L2281) | Boiling gives each drawing its own patch of paper. |
| [`no_grain_is_flat_colour`](../../../src/tools/fill.rs#L2295) | No grain is flat colour. |
| [`boundary_layer_walls_the_fill`](../../../src/tools/fill.rs#L2328) | Boundary layer walls the fill. |
| [`no_boundary_floods_everything`](../../../src/tools/fill.rs#L2347) | No boundary floods everything. |
| [`boundary_dimensions_must_match`](../../../src/tools/fill.rs#L2358) | Boundary dimensions must match. |
| [`expand_grows_region_under_the_wall`](../../../src/tools/fill.rs#L2366) | Expand grows region under the wall. |
| [`expand_clamps_to_canvas_edges`](../../../src/tools/fill.rs#L2382) | Expand clamps to canvas edges. |
| [`expand_zero_matches_unexpanded`](../../../src/tools/fill.rs#L2395) | Expand zero matches unexpanded. |
| [`same_colour_refill_is_a_noop_without_boundary`](../../../src/tools/fill.rs#L2405) | Same colour refill is a noop without boundary. |
| [`an_alpha_locked_fill_recolours_only_what_is_painted`](../../../src/tools/fill.rs#L2429) | An alpha locked fill recolours only what is painted. |
| [`a_selection_holds_the_fill_inside_it`](../../../src/tools/fill.rs#L2455) | A selection holds the fill inside it. |
| [`a_click_outside_the_selection_fills_nothing`](../../../src/tools/fill.rs#L2464) | A click outside the selection fills nothing. |
| [`a_feathered_selection_blends_the_fill`](../../../src/tools/fill.rs#L2473) | A feathered selection blends the fill. |
| [`fill_selection_paints_only_the_mask`](../../../src/tools/fill.rs#L2482) | Fill selection paints only the mask. |
| [`a_half_opacity_fill_goes_on_over_what_is_there`](../../../src/tools/fill.rs#L2513) | A half opacity fill goes on over what is there. |
| [`an_alpha_locked_half_fill_moves_the_colour_half_way`](../../../src/tools/fill.rs#L2527) | An alpha locked half fill moves the colour half way. |
| [`a_half_opacity_drag_never_builds_up`](../../../src/tools/fill.rs#L2545) | A half opacity drag never builds up. |
| [`a_zero_opacity_fill_does_nothing`](../../../src/tools/fill.rs#L2563) | A zero opacity fill does nothing. |
| [`fill_selection_takes_the_opacity_too`](../../../src/tools/fill.rs#L2577) | Fill selection takes the opacity too. |
| [`a_grainy_line_holds_the_fill`](../../../src/tools/fill.rs#L2700) | A grainy line holds the fill. |
| [`a_grainy_line_holds_a_fill_on_its_own_layer_too`](../../../src/tools/fill.rs#L2719) | A grainy line holds a fill on its own layer too. |
| [`a_line_with_no_holes_fills_as_it_always_did`](../../../src/tools/fill.rs#L2738) | Where a flood through everything at most half covered stayed inside its lines, a fill takes exactly what it took. |
| [`the_rim_is_every_pixel_beside_the_fill`](../../../src/tools/fill.rs#L2764) | The rim is every pixel beside the fill. |
| [`enclose_fills_inside_a_grainy_line`](../../../src/tools/fill.rs#L2798) | Enclose fills inside a grainy line. |
| [`closed_shapes_fill_the_same_at_any_gap`](../../../src/tools/fill.rs#L2807) | Closed shapes fill the same at any gap. |
| [`a_gap_up_to_the_setting_holds_the_fill`](../../../src/tools/fill.rs#L2857) | A gap up to the setting holds the fill. |
| [`a_line_ending_short_of_another_does_not_leak_along_it`](../../../src/tools/fill.rs#L2886) | A line ending short of another does not leak along it. |
| [`a_click_beside_a_line_still_shuts_the_gap`](../../../src/tools/fill.rs#L2901) | A click beside a line still shuts the gap. |
| [`a_shape_smaller_than_the_gap_still_fills`](../../../src/tools/fill.rs#L2909) | A shape smaller than the gap still fills. |
| [`a_line_stopping_short_of_the_edge_counts_as_shut`](../../../src/tools/fill.rs#L2918) | A line stopping short of the edge counts as shut. |
| [`a_session_redoes_the_fill_as_the_values_change`](../../../src/tools/fill.rs#L2933) | A session redoes the fill as the values change. |
| [`a_same_layer_session_reads_its_lines_from_before_the_press`](../../../src/tools/fill.rs#L2954) | A same layer session reads its lines from before the press. |
| [`an_inert_press_never_paints`](../../../src/tools/fill.rs#L2971) | An inert press never paints. |
| [`the_window_dilate_matches_the_plain_one`](../../../src/tools/fill.rs#L2981) | The window dilate matches the plain one. |
| [`a_wide_gap_holds_and_the_field_grows_to_serve_it`](../../../src/tools/fill.rs#L3034) | A wide gap holds and the field grows to serve it. |
| [`gap_drags_climb_a_ladder_that_spreads_out`](../../../src/tools/fill.rs#L3058) | Gap drags climb a ladder that spreads out. |
| [`fill_timing`](../../../src/tools/fill.rs#L3083) | Cost at a 1080p frame, for the record: `cargo test --release fill_timing -- --ignored --nocapture`. |
| [`grain_render`](../../../src/tools/fill.rs#L3192) | Writes grainy fills, under line art on its own layer, over grey paper, as `fill_grain_render.png` into `SNAPSHOT_DIR`: flat; grain 0.6; grain 0.6 at size 2.5 and contrast 1.8; a lasso Enclose & fill, Behind, boiled. `SNAPSHOT_DIR=... cargo test -- --ignored grain_render` |

**Test helpers** (used only by the tests above):

- [`old_wall_distance`](../../../src/tools/fill.rs#L1772) — Old wall distance.
- [`old_gap_flood`](../../../src/tools/fill.rs#L1823) — Old gap flood.
- [`next`](../../../src/tools/fill.rs#L1980) — Next.
- [`below`](../../../src/tools/fill.rs#L1987) — Below.
- [`two_boxes`](../../../src/tools/fill.rs#L2044) — A 40x40 canvas with two black squares drawn as 1px outlines: one closed, x 5..=15, and one with a 2px break in its top edge, x 22..=32.
- [`lasso_box`](../../../src/tools/fill.rs#L2062) — A lasso box from `(x0, y0)` to `(x1, y1)`, fully covered.
- [`grain`](../../../src/tools/fill.rs#L2164) — Grain.
- [`alphas`](../../../src/tools/fill.rs#L2168) — Alphas.
- [`roughness`](../../../src/tools/fill.rs#L2173) — Mean step between horizontal neighbours: low for a coarse tooth.
- [`spread`](../../../src/tools/fill.rs#L2179) — Spread.
- [`opts`](../../../src/tools/fill.rs#L2302) — Opts.
- [`walled`](../../../src/tools/fill.rs#L2315) — 16x16 transparent canvas with an opaque black wall down column 5.
- [`filled_at`](../../../src/tools/fill.rs#L2323) — Filled at.
- [`left_half`](../../../src/tools/fill.rs#L2418) — Left half of a 16x16 canvas selected at coverage `k`.
- [`half`](../../../src/tools/fill.rs#L2493) — Half.
- [`solid`](../../../src/tools/fill.rs#L2500) — Solid.
- [`near`](../../../src/tools/fill.rs#L2508) — Near.
- [`lines`](../../../src/tools/fill.rs#L2597) — A `w`×`h` line layer with opaque ink wherever `wall` says.
- [`fill_on`](../../../src/tools/fill.rs#L2610) — A fresh paint layer filled from `(x, y)` against `boundary`.
- [`count_filled`](../../../src/tools/fill.rs#L2620) — Count filled.
- [`box_outline`](../../../src/tools/fill.rs#L2626) — 1-px outline of the box `x0..=x1` × `y0..=y1`, its top edge broken by a hole `hole` pixels wide starting at `hole_x`.
- [`seg_dist`](../../../src/tools/fill.rs#L2642) — Seg dist.
- [`soft_ring`](../../../src/tools/fill.rs#L2650) — Anti-aliased ring of radius 20 round (32, 32): the core is solid and the rim fades, so the walls the flood sees are ragged.
- [`grainy_lines`](../../../src/tools/fill.rs#L2669) — A `w`×`h` line layer of grainy pencil, `offset` being each pixel's signed distance from the middle of a line: about 80% covered at most, fading over 4 px each side, and thinned by a hash of the pixel to as little as a third — so pixels under half covered run right through it, as through a grainy pencil line.
- [`grainy_ring`](../../../src/tools/fill.rs#L2688) — A grainy ring of radius 20 round (32, 32).
- [`half_covered_flood`](../../../src/tools/fill.rs#L2694) — What a press at `seed` on `c` itself would have filled when anything at most half covered let it through.
- [`reference`](../../../src/tools/fill.rs#L2983) — The whole-canvas dilate this module used to run.

**Test-only types and constants:** [`Rng`](../../../src/tools/fill.rs#L1977), [`RED`](../../../src/tools/fill.rs#L2040), [`BLUE`](../../../src/tools/fill.rs#L2491), [`INK`](../../../src/tools/fill.rs#L2594)
