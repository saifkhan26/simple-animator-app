# `src/ui/canvas_gpu.rs` — drawing the canvas on the GPU

> **Source:** [src/ui/canvas_gpu.rs](../../../src/ui/canvas_gpu.rs) · **Module:** `crate::ui::canvas_gpu` · **Size:** 792 lines · Line links as of commit `1fa5845`

## What this file is for

Every drawing on the canvas is a **GPU texture** drawn as a **quad** (a rectangle made of two triangles,
placed by its four screen corners so it can be moved, scaled and rotated with the view and the layer). egui can
draw textured quads itself, but its sampler blends neighbouring texels in **linear light**, while this app's
pixels are premultiplied in **gamma (sRGB) space** — mixing them in linear light makes soft edges brighter than
they should be: a faint **white rim** around light strokes. So this file builds a **copy of egui-wgpu's own render
pipeline**, identical in every setting except the texture read, which loads the four nearest texels and mixes
them in gamma space (see `src/ui/canvas_gpu.wgsl`).

### GPU concepts, for a WebGL/WebGPU-curious JS developer

| Term | Meaning here | Web equivalent |
|---|---|---|
| **texture** | the drawing's pixels living in GPU memory | `GPUTexture` / `gl.createTexture()` |
| **vertex** | one corner of a triangle: position, texture coordinate (UV), tint colour | a vertex buffer entry |
| **shader** | small programs run on the GPU: the *vertex shader* places corners on screen, the *fragment shader* computes each pixel's colour | WGSL / GLSL shaders |
| **render pipeline** | the fixed combination of shaders, vertex layout, blending and output format | `GPURenderPipeline` |
| **bind group** | the resources (texture, uniform buffer) a draw reads | `GPUBindGroup` |
| **uniform buffer** | small constant data for all draws (screen size, dithering on/off) | uniform buffer |

```text
CPU Canvas (RGBA bytes) ──upload──► GPU texture ──bind group──┐
four screen corners ──► 6 vertices (2 triangles) ─────────────┼──► render pass ──► window
                                       pipeline (shaders) ────┘
```

### How it plugs into egui

egui lets an app insert its own GPU drawing into egui's render pass with a **paint callback**
(`egui_wgpu::CallbackTrait`): `Quad::prepare` uploads the vertices before the pass, `Quad::paint` binds the pipeline
and draws inside it. Because it draws inside egui's pass, the pipeline must match the pass exactly — no MSAA, no depth
buffer, same dithering — which is why [main.rs](../main.md) sets those options and a test there checks them.

Without a GPU (tests), `CanvasGpu::recording()` just records what it would draw.

## How it connects

- **Used by:** [ui/shell.rs](shell.md) (`canvas_quad` draws drawing textures through it), [app.rs](../app.md)
  (`AppState::canvas_gpu`, created in `AppState::new`), [main.rs](../main.md) (`DITHERING`).
- **Uses:** `egui_wgpu`, `wgpu`, `bytemuck` (casting vertex structs to bytes), `pollster` (waiting on GPU error
  scopes), the WGSL shader `canvas_gpu.wgsl` (embedded with `include_str!`).

## Rust concepts in this file

**Implementing a library's trait.** `impl CallbackTrait for Quad { fn prepare(..); fn paint(..) }` — egui calls these
at the right moment, like implementing an interface a framework calls back.
More: [Traits and derive](../../rust-primer.md#traits-and-derive)

**`Arc<Shared>`** — the pipeline is created once and shared by every quad each frame. `self: &Arc<Self>` as a method
receiver lets `quad` clone the `Arc` into the new `Quad`.

**`#[repr(C)]` + `bytemuck`** — `Locals` has C layout and derives `Pod` so it can be copied to the GPU as raw bytes,
matching the shader's struct byte-for-byte (including padding).

## Types

### `const DITHERING`
[src/ui/canvas_gpu.rs:17](../../../src/ui/canvas_gpu.rs#L17)

```rust
pub const DITHERING: bool = true;
```

Whether dithering is on — shared by egui's pipeline and this one (and passed to eframe in `main`), so they agree.

### `struct Locals`
[src/ui/canvas_gpu.rs:22](../../../src/ui/canvas_gpu.rs#L22)

The uniform buffer contents, matching egui-wgpu's and the shader's `Locals` struct.

| Field | Type | Meaning |
|---|---|---|
| `screen_size_in_points` | `[f32; 2]` | Screen size in egui points (for converting positions to clip space). |
| `dithering` | `u32` | 1 if dithering is on. |
| `_padding` | `u32` | Padding to 16 bytes (uniform buffers need it). |

### `struct Shared`
[src/ui/canvas_gpu.rs:29](../../../src/ui/canvas_gpu.rs#L29)

The pipeline and the bind group layout/uniforms every quad shares.

| Field | Type | Meaning |
|---|---|---|
| `pipeline` | `wgpu::RenderPipeline` | The render pipeline. |
| `texture_layout` | `wgpu::BindGroupLayout` | Layout of a texture bind group. |
| `locals` | `wgpu::Buffer` | The uniform buffer. |
| `locals_group` | `wgpu::BindGroup` | Bind group for the uniforms. |
| `dithering` | `bool` | Whether dithering is on. |

### `struct Quad`
[src/ui/canvas_gpu.rs:219](../../../src/ui/canvas_gpu.rs#L219)

One quad on its way to the GPU: its texture, bound, and its vertices. Made fresh every frame, so it never outlives the texture it binds.

| Field | Type | Meaning |
|---|---|---|
| `shared` | `Arc<Shared>` | The shared pipeline. |
| `texture` | `wgpu::BindGroup` | The bind group of this quad's texture. |
| `vertices` | `wgpu::Buffer` | The quad's vertex buffer (6 vertices). |

### `struct CanvasGpu`
[src/ui/canvas_gpu.rs:263](../../../src/ui/canvas_gpu.rs#L263)

The canvas renderer: the real GPU pipeline, or a recorder in tests.

| Field | Type | Meaning |
|---|---|---|
| `0` | `Kind` |  |

### `enum Kind`
[src/ui/canvas_gpu.rs:265](../../../src/ui/canvas_gpu.rs#L265)

What a `CanvasGpu` really is: the GPU pipeline, or a recorder for tests.

| Variant | Meaning |
|---|---|
| `Gpu { rs: RenderState, shared: Arc<Shared> }` | The real pipeline with eframe's render state. |
| `Recording` | Records what it would draw instead, for headless tests. |

### `struct Recorded`
[src/ui/canvas_gpu.rs:315](../../../src/ui/canvas_gpu.rs#L315)

What a recording `CanvasGpu` was asked to draw.

| Field | Type | Meaning |
|---|---|---|
| `tex` | `TextureId` | The texture asked for. |
| `corners` | `[Pos2; 4]` | Its screen corners. |
| `tint` | `Color32` | Its tint. |

## `impl Shared`

### `Shared::new`
[src/ui/canvas_gpu.rs:42](../../../src/ui/canvas_gpu.rs#L42)

```rust
pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, dithering: bool) -> Self
```

egui-wgpu 0.29's `Renderer::new`, setting for setting, for a render pass with no depth buffer and no multisampling — which is what `main` asks eframe for. Only the texture binding differs: a texture with no sampler, which the shader reads texel by texel.

### `Shared::quad`
[src/ui/canvas_gpu.rs:169](../../../src/ui/canvas_gpu.rs#L169)

```rust
fn quad(
    self: &Arc<Self>,
    device: &wgpu::Device,
    renderer: &egui_wgpu::Renderer,
    tex: TextureId,
    corners: [Pos2; 4],
    tint: Color32,
) -> Option<Quad>
```

A quad of egui's texture `tex`, or `None` while egui has no texture by that id — one made this frame only reaches the renderer after it.

## Free functions

### `vertices`
[src/ui/canvas_gpu.rs:207](../../../src/ui/canvas_gpu.rs#L207)

```rust
fn vertices(corners: [Pos2; 4], tint: Color32) -> [Vertex; 6]
```

`ui::shell::image_quad`'s mesh as a plain triangle list: the same corners, UVs, tint and triangles, so the same pixels are covered.

## `impl CallbackTrait for Quad`

### `Quad::prepare`
[src/ui/canvas_gpu.rs:226](../../../src/ui/canvas_gpu.rs#L226)

```rust
fn prepare(
    &self,
    _device: &wgpu::Device,
    queue: &wgpu::Queue,
    screen: &ScreenDescriptor,
    _encoder: &mut wgpu::CommandEncoder,
    _resources: &mut CallbackResources,
) -> Vec<wgpu::CommandBuffer>
```

Before the render pass: writes the screen size and dithering flag into the uniform buffer.

### `Quad::paint`
[src/ui/canvas_gpu.rs:248](../../../src/ui/canvas_gpu.rs#L248)

```rust
fn paint(&self, info: PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, _resources: &CallbackResources)
```

Inside egui's render pass: sets the pipeline, binds uniforms and the texture, the vertex buffer, and draws the 6
vertices.

## `impl CanvasGpu`

### `CanvasGpu::new`
[src/ui/canvas_gpu.rs:276](../../../src/ui/canvas_gpu.rs#L276)

```rust
pub fn new(rs: &RenderState) -> Option<Self>
```

The pipeline for eframe's window, or `None` if the GPU refuses it — then the canvas draws through egui as it always has. Never a panic: a release build aborts on one.

### `CanvasGpu::recording`
[src/ui/canvas_gpu.rs:291](../../../src/ui/canvas_gpu.rs#L291)

```rust
pub fn recording() -> Self
```

A recorder for headless tests (records quads instead of drawing).

### `CanvasGpu::quad`
[src/ui/canvas_gpu.rs:297](../../../src/ui/canvas_gpu.rs#L297)

```rust
pub fn quad(&self, clip: Rect, tex: TextureId, corners: [Pos2; 4], tint: Color32) -> Option<Shape>
```

`tex` over `corners` at `tint`, as `image_quad` paints it, clipped to `clip`. `None` when egui has no such texture yet.

## Tests

### `mod tests`

Run with `cargo test ui::canvas_gpu::tests` · 6 tests

| Test | What it checks |
|---|---|
| [`perf_cell_upload`](../../../src/ui/canvas_gpu.rs#L649) | What a cell upload costs on this machine's GPU, through egui's own renderer, until the GPU has it: a whole 2160×3840 drawing made new (a live stroke outgrowing its texture), the same re-sent into its handle, and a stroke-sized patch. Not a pass/fail test — run it for the numbers: `cargo test --release perf_cell_upload -- --ignored --nocapture` |
| [`matches_egui_at_texel_centres`](../../../src/ui/canvas_gpu.rs#L686) | At 1:1 on whole pixels every sample is a texel centre, where the two pipelines must agree: the canvas at 100% looks exactly as it did. |
| [`dark_ink_matches_egui_when_zoomed`](../../../src/ui/canvas_gpu.rs#L707) | Dark ink barely showed the fault, so zoomed and turned it must look as it did: on the reference, and no further from egui's old pixels than egui's own error on this ink (3.5 levels at most, measured on an RTX 2060) plus one for rounding. |
| [`light_ink_has_no_rim_at_any_zoom`](../../../src/ui/canvas_gpu.rs#L730) | The report: light ink over paper, between texels. egui's path comes out well above what the texels allow — the white rim — and ours sits on the gamma-space reference at every zoom and turn. |
| [`clip_rect_is_kept`](../../../src/ui/canvas_gpu.rs#L751) | Nothing lands outside egui's clip rect, though the viewport is reset. |
| [`keeps_paint_order_with_meshes`](../../../src/ui/canvas_gpu.rs#L776) | Callbacks keep their place among egui's meshes: one painted after a quad covers it, and the quad covers one painted before. |

**Test helpers** (used only by the tests above):

- [`gpu`](../../../src/ui/canvas_gpu.rs#L350) — Gpu.
- [`upload`](../../../src/ui/canvas_gpu.rs#L376) — A straight-alpha image uploaded the way the app uploads a cell.
- [`image`](../../../src/ui/canvas_gpu.rs#L385) — Image.
- [`band`](../../../src/ui/canvas_gpu.rs#L395) — A 16×8 band of `ink`: transparent above and below and at both ends, a partly covered row on each side, solid in the middle — hard and soft edges, both ways.
- [`speckle`](../../../src/ui/canvas_gpu.rs#L408) — 16×8 of seeded noise: a third clear, a third solid, a third between.
- [`placed`](../../../src/ui/canvas_gpu.rs#L431) — The corners of `size` texels scaled by `zoom` and turned `deg` about the centre of the screen.
- [`at`](../../../src/ui/canvas_gpu.rs#L439) — Axis-aligned at 1:1, on whole pixels: every sample a texel centre.
- [`full`](../../../src/ui/canvas_gpu.rs#L459) — Full.
- [`draw`](../../../src/ui/canvas_gpu.rs#L463) — Draw.
- [`render`](../../../src/ui/canvas_gpu.rs#L474) — The scene drawn over the paper, read back, RGBA per pixel.
- [`max_diff`](../../../src/ui/canvas_gpu.rs#L581) — Largest per-channel difference between two renders.
- [`changed`](../../../src/ui/canvas_gpu.rs#L589) — Changed.
- [`reference`](../../../src/ui/canvas_gpu.rs#L597) — What pixel (x, y) should be with `tex` drawn over the paper at `corners`: the premultiplied texels mixed in gamma space (clamped at the edges), tinted, and blended in gamma — or `None` for a pixel near the quad's edge, where the rasterizer's rules decide coverage.
- [`against_reference`](../../../src/ui/canvas_gpu.rs#L623) — The worst amount `px` sits above and away from the reference, over the pixels the reference covers, and how many it covers.

**Test-only types and constants:** [`W`](../../../src/ui/canvas_gpu.rs#L333), [`H`](../../../src/ui/canvas_gpu.rs#L334), [`FORMAT`](../../../src/ui/canvas_gpu.rs#L335), [`PAPER`](../../../src/ui/canvas_gpu.rs#L337), [`LIGHT`](../../../src/ui/canvas_gpu.rs#L338), [`DARK`](../../../src/ui/canvas_gpu.rs#L339), [`Gpu`](../../../src/ui/canvas_gpu.rs#L341), [`Via`](../../../src/ui/canvas_gpu.rs#L445), [`Draw`](../../../src/ui/canvas_gpu.rs#L451)
