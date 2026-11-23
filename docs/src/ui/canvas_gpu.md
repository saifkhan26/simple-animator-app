# `src/ui/canvas_gpu.rs` — drawing the canvas on the GPU

> **Source:** [src/ui/canvas_gpu.rs](../../../src/ui/canvas_gpu.rs) · **Module:** `crate::ui::canvas_gpu` · **Size:** 1337 lines · Line links as of commit `76a196a`

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

### The art stack (blend modes)

Quads blend straight onto the window with "premultiplied over", which is all Normal layers need. A layer in another
[blend mode](../doc/blend.md) has to mix with the *art* under it, not with the paper (the window's background,
which is dark by default) — so while a layer on show has one, `ui::shell`'s `Art` gathers every quad of the
artwork into one [`Stack`](#struct-stack) instead:

1. In `prepare`, before egui's pass, each item is drawn into an off-screen `Rgba8Unorm` texture the size of the
   window (premultiplied gamma, as the CPU compositor works): Normal ones blended over, others by copying the pixels
   under them into a second texture and writing what `canvas_blend.wgsl`'s `fs_stack_blend` works out from both.
2. In `paint`, the finished texture is laid on the window like one quad.

With every layer Normal none of this runs: the quads draw exactly as before. The stack's pipelines are made in
their own GPU error scope, so a GPU that refuses them only loses blend modes (shown as Normal). GPU tests check the
stack against `doc::blend` pixel by pixel.

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
[src/ui/canvas_gpu.rs:19](../../../src/ui/canvas_gpu.rs#L19)

```rust
pub const DITHERING: bool = true;
```

Whether dithering is on — shared by egui's pipeline and this one (and passed to eframe in `main`), so they agree.

### `struct Locals`
[src/ui/canvas_gpu.rs:24](../../../src/ui/canvas_gpu.rs#L24)

The uniform buffer contents, matching egui-wgpu's and the shader's `Locals` struct.

| Field | Type | Meaning |
|---|---|---|
| `screen_size_in_points` | `[f32; 2]` | Screen size in egui points (for converting positions to clip space). |
| `dithering` | `u32` | 1 if dithering is on. |
| `_padding` | `u32` | Padding to 16 bytes (uniform buffers need it). |

### `struct Shared`
[src/ui/canvas_gpu.rs:31](../../../src/ui/canvas_gpu.rs#L31)

The pipeline and the bind group layout/uniforms every quad shares.

| Field | Type | Meaning |
|---|---|---|
| `pipeline` | `wgpu::RenderPipeline` | The render pipeline. |
| `texture_layout` | `wgpu::BindGroupLayout` | Layout of a texture bind group. |
| `locals_layout` | `wgpu::BindGroupLayout` | Kept for the art stack's pipelines, whose group 0 is this one. |
| `locals` | `wgpu::Buffer` | The uniform buffer. |
| `locals_group` | `wgpu::BindGroup` | Bind group for the uniforms. |
| `dithering` | `bool` | Whether dithering is on. |

### `struct Quad`
[src/ui/canvas_gpu.rs:224](../../../src/ui/canvas_gpu.rs#L224)

One quad on its way to the GPU: its texture, bound, and its vertices. Made fresh every frame, so it never outlives the texture it binds.

| Field | Type | Meaning |
|---|---|---|
| `shared` | `Arc<Shared>` | The shared pipeline. |
| `texture` | `wgpu::BindGroup` | The bind group of this quad's texture. |
| `vertices` | `wgpu::Buffer` | The quad's vertex buffer (6 vertices). |

### `const ART`
[src/ui/canvas_gpu.rs:269](../../../src/ui/canvas_gpu.rs#L269)

```rust
const ART: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
```

The art stack's own texture format: premultiplied gamma, as the CPU compositor works, whatever the window's format.

### `struct StackItem`
[src/ui/canvas_gpu.rs:274](../../../src/ui/canvas_gpu.rs#L274)

One draw in the art stack: a drawing's texture over `corners` at `tint`, in `blend`.

| Field | Type | Meaning |
|---|---|---|
| `tex` | `TextureId` | The drawing's texture. |
| `corners` | `[Pos2; 4]` | Where it goes on screen, in points: TL, TR, BR, BL. |
| `tint` | `Color32` | Premultiplied tint: the layer's opacity, as a quad's. |
| `blend` | `BlendMode` | How it mixes with the art under it. |

### `struct Stack`
[src/ui/canvas_gpu.rs:286](../../../src/ui/canvas_gpu.rs#L286)

The art stack: the canvas's artwork drawn into a texture of its own, layer by layer, each in its blend mode, then laid on the window in one go. Only used while a layer on show has a blend mode — so a blend mixes with the art alone, never with the paper — and the plain quads draw everything else exactly as before.

| Field | Type | Meaning |
|---|---|---|
| `shared` | `Arc<Shared>` | The quads' pipeline resources: the uniform buffer and texture layouts it reuses. |
| `normal` | `wgpu::RenderPipeline` | A Normal layer, premultiplied over, into the stack. |
| `blends` | `Vec<(BlendMode, wgpu::RenderPipeline)>` | Every other mode: written, not blended, from the layer and a copy of what is under it. |
| `blit` | `wgpu::RenderPipeline` | The finished stack onto the window, as a quad is laid on it. |
| `target` | `Mutex<Option<Target>>` | The textures, made at the window's size on first use. |

### `struct Target`
[src/ui/canvas_gpu.rs:301](../../../src/ui/canvas_gpu.rs#L301)

The stack's two screen-sized textures: the art so far, and a copy of the part of it under the layer being blended.

| Field | Type | Meaning |
|---|---|---|
| `size` | `[u32; 2]` | The window's size in pixels, which both textures are made at. |
| `art` | `wgpu::Texture` | The art so far: premultiplied gamma `Rgba8Unorm`. |
| `art_view` | `wgpu::TextureView` | A view of `art`, to render into. |
| `backdrop` | `wgpu::Texture` | A copy of the part of `art` under the layer being blended — what the blend shader mixes with. |
| `art_group` | `wgpu::BindGroup` | `art` bound where a drawing would be, for the blit. |
| `backdrop_group` | `wgpu::BindGroup` | `backdrop` bound for the blend shader. |

### `struct StackDraw`
[src/ui/canvas_gpu.rs:495](../../../src/ui/canvas_gpu.rs#L495)

One frame's art stack on its way to the GPU: its draws, each bound, and the quad that lays the result on the window.

| Field | Type | Meaning |
|---|---|---|
| `stack` | `Arc<Stack>` | The pipelines and textures. |
| `draws` | `Vec<(BlendMode, [Pos2; 4], Quad)>` | Each item's mode, corners and bound quad, bottom first. |
| `blit` | `wgpu::Buffer` | Vertices of the quad that lays the finished stack on the window: the canvas's clip rect. |

### `struct CanvasGpu`
[src/ui/canvas_gpu.rs:638](../../../src/ui/canvas_gpu.rs#L638)

The canvas renderer: the real GPU pipeline, or a recorder in tests.

| Field | Type | Meaning |
|---|---|---|
| `0` | `Kind` |  |

### `enum Kind`
[src/ui/canvas_gpu.rs:640](../../../src/ui/canvas_gpu.rs#L640)

What a `CanvasGpu` really is: the GPU pipeline, or a recorder for tests.

| Variant | Meaning |
|---|---|
| `Gpu { rs, shared }` | The real pipeline with eframe's render state. |
| `Recording` | Records what it would draw instead, for headless tests. |

### `struct RecordedStack`
[src/ui/canvas_gpu.rs:728](../../../src/ui/canvas_gpu.rs#L728)

What a recording `CanvasGpu` was asked to draw as an art stack.

| Field | Type | Meaning |
|---|---|---|
| `items` | `Vec<StackItem>` | The items, bottom first. |

### `struct Recorded`
[src/ui/canvas_gpu.rs:736](../../../src/ui/canvas_gpu.rs#L736)

What a recording `CanvasGpu` was asked to draw.

| Field | Type | Meaning |
|---|---|---|
| `tex` | `TextureId` | The texture asked for. |
| `corners` | `[Pos2; 4]` | Its screen corners. |
| `tint` | `Color32` | Its tint. |

## `impl Shared`

### `Shared::new`
[src/ui/canvas_gpu.rs:46](../../../src/ui/canvas_gpu.rs#L46)

```rust
pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, dithering: bool) -> Self
```

egui-wgpu 0.29's `Renderer::new`, setting for setting, for a render pass with no depth buffer and no multisampling — which is what `main` asks eframe for. Only the texture binding differs: a texture with no sampler, which the shader reads texel by texel.

### `Shared::quad`
[src/ui/canvas_gpu.rs:174](../../../src/ui/canvas_gpu.rs#L174)

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
[src/ui/canvas_gpu.rs:212](../../../src/ui/canvas_gpu.rs#L212)

```rust
fn vertices(corners: [Pos2; 4], tint: Color32) -> [Vertex; 6]
```

`ui::shell::image_quad`'s mesh as a plain triangle list: the same corners, UVs, tint and triangles, so the same pixels are covered.

## `impl CallbackTrait for Quad`

### `Quad::prepare`
[src/ui/canvas_gpu.rs:231](../../../src/ui/canvas_gpu.rs#L231)

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
[src/ui/canvas_gpu.rs:253](../../../src/ui/canvas_gpu.rs#L253)

```rust
fn paint(&self, info: PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, _resources: &CallbackResources)
```

Inside egui's render pass: sets the pipeline, binds uniforms and the texture, the vertex buffer, and draws the 6
vertices.

## `impl Stack`

### `Stack::new`
[src/ui/canvas_gpu.rs:316](../../../src/ui/canvas_gpu.rs#L316)

```rust
pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, shared: &Arc<Shared>) -> Self
```

The pipelines, for a window of `format`. The shader is canvas_gpu.wgsl with canvas_blend.wgsl after it, one module per blend mode, each told its mode in a constant.

### `Stack::draw`
[src/ui/canvas_gpu.rs:420](../../../src/ui/canvas_gpu.rs#L420)

```rust
fn draw(
    self: &Arc<Self>,
    device: &wgpu::Device,
    renderer: &egui_wgpu::Renderer,
    clip: Rect,
    items: &[StackItem],
) -> StackDraw
```

`items`, bottom first, as one shape clipped to `clip`. An item whose texture egui doesn't have is left out, as a quad would be.

### `Stack::target`
[src/ui/canvas_gpu.rs:445](../../../src/ui/canvas_gpu.rs#L445)

```rust
fn target<'a>(&self, device: &wgpu::Device, slot: &'a mut Option<Target>, size: [u32; 2]) -> &'a Target
```

The textures at `size`, made or remade to fit.

## Free functions

### `reach`
[src/ui/canvas_gpu.rs:503](../../../src/ui/canvas_gpu.rs#L503)

```rust
fn reach(corners: &[Pos2; 4], ppp: f32, size: [u32; 2]) -> Option<([u32; 2], [u32; 2])>
```

Pixels of a `size` screen that a quad at `corners` (in points) can touch, with a pixel to spare, as an origin and an extent. `None` when off screen.

### `art_pass`
[src/ui/canvas_gpu.rs:518](../../../src/ui/canvas_gpu.rs#L518)

```rust
fn art_pass(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, clear: bool) -> wgpu::RenderPass<'static>
```

A pass drawing into the art texture, cleared first or carrying on.

## `impl CallbackTrait for StackDraw`

### `StackDraw::prepare`
[src/ui/canvas_gpu.rs:542](../../../src/ui/canvas_gpu.rs#L542)

```rust
fn prepare(
    &self,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    screen: &ScreenDescriptor,
    encoder: &mut wgpu::CommandEncoder,
    _resources: &mut CallbackResources,
) -> Vec<wgpu::CommandBuffer>
```

Records the stack before egui's pass: writes the uniforms, makes the textures at the window's size if needed, clears the art texture and draws each item into it — a Normal one with premultiplied over, any other by copying the pixels under it into the backdrop texture and writing the blend shader's result.

### `StackDraw::paint`
[src/ui/canvas_gpu.rs:621](../../../src/ui/canvas_gpu.rs#L621)

```rust
fn paint(&self, info: PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, _resources: &CallbackResources)
```

Lays the finished art texture on the window over the canvas's clip rect, as egui lays a texel: premultiplied over, dithered.

## `impl CanvasGpu`

### `CanvasGpu::new`
[src/ui/canvas_gpu.rs:657](../../../src/ui/canvas_gpu.rs#L657)

```rust
pub fn new(rs: &RenderState) -> Option<Self>
```

The pipeline for eframe's window, or `None` if the GPU refuses it — then the canvas draws through egui as it always has. Never a panic: a release build aborts on one.

### `CanvasGpu::recording`
[src/ui/canvas_gpu.rs:685](../../../src/ui/canvas_gpu.rs#L685)

```rust
pub fn recording() -> Self
```

A recorder for headless tests (records quads instead of drawing).

### `CanvasGpu::quad`
[src/ui/canvas_gpu.rs:691](../../../src/ui/canvas_gpu.rs#L691)

```rust
pub fn quad(&self, clip: Rect, tex: TextureId, corners: [Pos2; 4], tint: Color32) -> Option<Shape>
```

`tex` over `corners` at `tint`, as `image_quad` paints it, clipped to `clip`. `None` when egui has no such texture yet.

## `impl CanvasGpu`

### `CanvasGpu::stack`
[src/ui/canvas_gpu.rs:710](../../../src/ui/canvas_gpu.rs#L710)

```rust
pub fn stack(&self, clip: Rect, items: &[StackItem]) -> Option<Shape>
```

`items` drawn as the art stack, clipped to `clip`: see `Stack`. `None` when the GPU has no stack, for the caller to draw the items as plain quads instead.

## Tests

### `mod tests`

Run with `cargo test ui::canvas_gpu::tests` · 10 tests

| Test | What it checks |
|---|---|
| [`a_stack_of_normal_layers_matches_the_quads`](../../../src/ui/canvas_gpu.rs#L1049) | Normal layers through the stack look as they do drawn straight onto the window, to within the dithering's last level. |
| [`each_blend_mode_matches_the_cpu`](../../../src/ui/canvas_gpu.rs#L1067) | Every mode on the GPU comes out as the CPU — export, merged groups — works it out, over solid and soft pixels alike. |
| [`the_paper_takes_no_part_in_a_blend`](../../../src/ui/canvas_gpu.rs#L1094) | A blend mixes with the art, never the paper: over bare paper a Multiply layer looks just as a Normal one does. |
| [`a_blend_leaves_the_art_beside_it_alone`](../../../src/ui/canvas_gpu.rs#L1108) | A blended layer only touches the pixels under it: art beside it stays as it was. |
| [`perf_cell_upload`](../../../src/ui/canvas_gpu.rs#L1194) | What a cell upload costs on this machine's GPU, through egui's own renderer, until the GPU has it: a whole 2160×3840 drawing made new (a live stroke outgrowing its texture), the same re-sent into its handle, and a stroke-sized patch. Not a pass/fail test — run it for the numbers: `cargo test --release perf_cell_upload -- --ignored --nocapture` |
| [`matches_egui_at_texel_centres`](../../../src/ui/canvas_gpu.rs#L1231) | At 1:1 on whole pixels every sample is a texel centre, where the two pipelines must agree: the canvas at 100% looks exactly as it did. |
| [`dark_ink_matches_egui_when_zoomed`](../../../src/ui/canvas_gpu.rs#L1252) | Dark ink barely showed the fault, so zoomed and turned it must look as it did: on the reference, and no further from egui's old pixels than egui's own error on this ink (3.5 levels at most, measured on an RTX 2060) plus one for rounding. |
| [`light_ink_has_no_rim_at_any_zoom`](../../../src/ui/canvas_gpu.rs#L1275) | The report: light ink over paper, between texels. egui's path comes out well above what the texels allow — the white rim — and ours sits on the gamma-space reference at every zoom and turn. |
| [`clip_rect_is_kept`](../../../src/ui/canvas_gpu.rs#L1296) | Nothing lands outside egui's clip rect, though the viewport is reset. |
| [`keeps_paint_order_with_meshes`](../../../src/ui/canvas_gpu.rs#L1321) | Callbacks keep their place among egui's meshes: one painted after a quad covers it, and the quad covers one painted before. |

**Test helpers** (used only by the tests above):

- [`gpu`](../../../src/ui/canvas_gpu.rs#L772) — Gpu.
- [`upload`](../../../src/ui/canvas_gpu.rs#L800) — A straight-alpha image uploaded the way the app uploads a cell.
- [`image`](../../../src/ui/canvas_gpu.rs#L809) — Image.
- [`band`](../../../src/ui/canvas_gpu.rs#L819) — A 16×8 band of `ink`: transparent above and below and at both ends, a partly covered row on each side, solid in the middle — hard and soft edges, both ways.
- [`speckle`](../../../src/ui/canvas_gpu.rs#L832) — 16×8 of seeded noise: a third clear, a third solid, a third between.
- [`placed`](../../../src/ui/canvas_gpu.rs#L855) — The corners of `size` texels scaled by `zoom` and turned `deg` about the centre of the screen.
- [`at`](../../../src/ui/canvas_gpu.rs#L863) — Axis-aligned at 1:1, on whole pixels: every sample a texel centre.
- [`full`](../../../src/ui/canvas_gpu.rs#L883) — Full.
- [`draw`](../../../src/ui/canvas_gpu.rs#L887) — Draw.
- [`render`](../../../src/ui/canvas_gpu.rs#L898) — The scene drawn over the paper, read back, RGBA per pixel.
- [`render_jobs`](../../../src/ui/canvas_gpu.rs#L928) — `jobs` rendered over the paper, read back, RGBA per pixel.
- [`render_stack`](../../../src/ui/canvas_gpu.rs#L1010) — `items` drawn as one art stack over the paper, read back.
- [`item`](../../../src/ui/canvas_gpu.rs#L1020) — Item.
- [`gradient`](../../../src/ui/canvas_gpu.rs#L1025) — 16×8, opaque, every pixel its own colour: something to blend onto.
- [`expected`](../../../src/ui/canvas_gpu.rs#L1037) — What the stack should come to at texel (x, y) of two 16×8 images laid 1:1 — `under` Normal, `over` in `mode` at `opacity` — over the paper, worked out by `doc::blend` as the CPU compositor does.
- [`max_diff`](../../../src/ui/canvas_gpu.rs#L1126) — Largest per-channel difference between two renders.
- [`changed`](../../../src/ui/canvas_gpu.rs#L1134) — Changed.
- [`reference`](../../../src/ui/canvas_gpu.rs#L1142) — What pixel (x, y) should be with `tex` drawn over the paper at `corners`: the premultiplied texels mixed in gamma space (clamped at the edges), tinted, and blended in gamma — or `None` for a pixel near the quad's edge, where the rasterizer's rules decide coverage.
- [`against_reference`](../../../src/ui/canvas_gpu.rs#L1168) — The worst amount `px` sits above and away from the reference, over the pixels the reference covers, and how many it covers.

**Test-only types and constants:** [`W`](../../../src/ui/canvas_gpu.rs#L754), [`H`](../../../src/ui/canvas_gpu.rs#L755), [`FORMAT`](../../../src/ui/canvas_gpu.rs#L756), [`PAPER`](../../../src/ui/canvas_gpu.rs#L758), [`LIGHT`](../../../src/ui/canvas_gpu.rs#L759), [`DARK`](../../../src/ui/canvas_gpu.rs#L760), [`Gpu`](../../../src/ui/canvas_gpu.rs#L762), [`Via`](../../../src/ui/canvas_gpu.rs#L869), [`Draw`](../../../src/ui/canvas_gpu.rs#L875)
