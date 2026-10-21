//! Canvas artwork drawn through a copy of egui-wgpu's own pipeline that mixes
//! texels in gamma space rather than linear light — `canvas_gpu.wgsl` says
//! why. `ui::shell::canvas_quad` draws through it, and through a plain egui
//! mesh wherever this can't: no GPU (the headless tests), or a texture egui
//! hasn't made yet.

use std::num::NonZeroU64;
use std::sync::Arc;

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor};
use eframe::wgpu::{self, util::DeviceExt};
use egui::epaint::Vertex;
use egui::{pos2, Color32, PaintCallbackInfo, Pos2, Rect, Shape, TextureId};

/// Dithering, for egui's pipeline and this one alike: `main` hands this to
/// eframe, so the two always agree.
pub const DITHERING: bool = true;

/// egui-wgpu 0.29's `UniformBuffer`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Locals {
    screen_size_in_points: [f32; 2],
    dithering: u32,
    _padding: u32,
}

/// The pipeline, and what every quad drawn through it shares.
pub(crate) struct Shared {
    pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    locals: wgpu::Buffer,
    locals_group: wgpu::BindGroup,
    dithering: bool,
}

impl Shared {
    /// egui-wgpu 0.29's `Renderer::new`, setting for setting, for a render
    /// pass with no depth buffer and no multisampling — which is what `main`
    /// asks eframe for. Only the texture binding differs: a texture with no
    /// sampler, which the shader reads texel by texel.
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, dithering: bool) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("canvas_quad"),
            source: wgpu::ShaderSource::Wgsl(include_str!("canvas_gpu.wgsl").into()),
        });
        let locals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("canvas_quad_locals"),
            contents: bytemuck::bytes_of(&Locals {
                screen_size_in_points: [0.0, 0.0],
                dithering: u32::from(dithering),
                _padding: 0,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let locals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas_quad_locals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<Locals>() as _),
                    ty: wgpu::BufferBindingType::Uniform,
                },
                count: None,
            }],
        });
        let locals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canvas_quad_locals"),
            layout: &locals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &locals,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas_quad_texture"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas_quad"),
            bind_group_layouts: &[&locals_layout, &texture_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("canvas_quad"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                entry_point: "vs_main",
                module: &module,
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 5 * 4,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    // 0: vec2 position
                    // 1: vec2 texture coordinates
                    // 2: uint color
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                unclipped_depth: false,
                conservative: false,
                cull_mode: None,
                front_face: wgpu::FrontFace::default(),
                polygon_mode: wgpu::PolygonMode::default(),
                strip_index_format: None,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                alpha_to_coverage_enabled: false,
                count: 1,
                mask: !0,
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: if format.is_srgb() {
                    "fs_main_linear_framebuffer"
                } else {
                    "fs_main_gamma_framebuffer"
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });
        Self {
            pipeline,
            texture_layout,
            locals,
            locals_group,
            dithering,
        }
    }

    /// A quad of egui's texture `tex`, or `None` while egui has no texture
    /// by that id — one made this frame only reaches the renderer after it.
    fn quad(
        self: &Arc<Self>,
        device: &wgpu::Device,
        renderer: &egui_wgpu::Renderer,
        tex: TextureId,
        corners: [Pos2; 4],
        tint: Color32,
    ) -> Option<Quad> {
        let texture = renderer.texture(&tex)?.texture.as_ref()?;
        // Only what egui makes of a `ColorImage`: the shader reads texels as
        // decoded sRGB.
        if texture.format() != wgpu::TextureFormat::Rgba8UnormSrgb {
            return None;
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let texture = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canvas_quad_texture"),
            layout: &self.texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("canvas_quad_vertices"),
            contents: bytemuck::cast_slice(&vertices(corners, tint)),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Some(Quad {
            shared: Arc::clone(self),
            texture,
            vertices,
        })
    }
}

/// `ui::shell::image_quad`'s mesh as a plain triangle list: the same corners,
/// UVs, tint and triangles, so the same pixels are covered.
fn vertices(corners: [Pos2; 4], tint: Color32) -> [Vertex; 6] {
    let uv = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    let v = |i: usize| Vertex {
        pos: corners[i],
        uv: uv[i],
        color: tint,
    };
    [v(0), v(1), v(2), v(0), v(2), v(3)]
}

/// One quad on its way to the GPU: its texture, bound, and its vertices. Made
/// fresh every frame, so it never outlives the texture it binds.
struct Quad {
    shared: Arc<Shared>,
    texture: wgpu::BindGroup,
    vertices: wgpu::Buffer,
}

impl CallbackTrait for Quad {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        _resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // What egui writes into its own uniforms each frame.
        let ppp = screen.pixels_per_point;
        let locals = Locals {
            screen_size_in_points: [
                screen.size_in_pixels[0] as f32 / ppp,
                screen.size_in_pixels[1] as f32 / ppp,
            ],
            dithering: u32::from(self.shared.dithering),
            _padding: 0,
        };
        queue.write_buffer(&self.shared.locals, 0, bytemuck::bytes_of(&locals));
        Vec::new()
    }

    fn paint(&self, info: PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, _resources: &CallbackResources) {
        // egui narrows the viewport to the callback's rect. The vertices are
        // in screen points, like egui's own, so draw against the whole screen;
        // the scissor stays egui's clip rect.
        let [w, h] = info.screen_size_px;
        pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(&self.shared.pipeline);
        pass.set_bind_group(0, &self.shared.locals_group, &[]);
        pass.set_bind_group(1, &self.texture, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..6, 0..1);
    }
}

/// Draws canvas textures in gamma space. `None` in `AppState` without a GPU.
pub struct CanvasGpu(Kind);

enum Kind {
    Gpu { rs: RenderState, shared: Arc<Shared> },
    /// Records what it would draw instead, for headless tests.
    #[cfg(test)]
    Recording,
}

impl CanvasGpu {
    /// The pipeline for eframe's window, or `None` if the GPU refuses it —
    /// then the canvas draws through egui as it always has. Never a panic:
    /// a release build aborts on one.
    pub fn new(rs: &RenderState) -> Option<Self> {
        let device = &rs.device;
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shared = Shared::new(device, rs.target_format, DITHERING);
        if let Some(err) = pollster::block_on(device.pop_error_scope()) {
            log::warn!("canvas pipeline disabled, drawing through egui instead: {err}");
            return None;
        }
        Some(Self(Kind::Gpu {
            rs: rs.clone(),
            shared: Arc::new(shared),
        }))
    }

    #[cfg(test)]
    pub fn recording() -> Self {
        Self(Kind::Recording)
    }

    /// `tex` over `corners` at `tint`, as `image_quad` paints it, clipped to
    /// `clip`. `None` when egui has no such texture yet.
    pub fn quad(&self, clip: Rect, tex: TextureId, corners: [Pos2; 4], tint: Color32) -> Option<Shape> {
        match &self.0 {
            Kind::Gpu { rs, shared } => {
                let quad = shared.quad(&rs.device, &rs.renderer.read(), tex, corners, tint)?;
                Some(Shape::Callback(egui_wgpu::Callback::new_paint_callback(clip, quad)))
            }
            #[cfg(test)]
            Kind::Recording => Some(Shape::Callback(egui::PaintCallback {
                rect: clip,
                callback: Arc::new(Recorded { tex, corners, tint }),
            })),
        }
    }
}

/// What a recording `CanvasGpu` was asked to draw.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recorded {
    pub tex: TextureId,
    pub corners: [Pos2; 4],
    pub tint: Color32,
}

/// These draw through egui-wgpu's real `Renderer` on whatever GPU the
/// machine has: egui's own mesh path next to this one, read back and compared
/// pixel by pixel. They need a GPU; `ANIMATOR_SKIP_GPU_TESTS=1` skips them on
/// a machine without one, and only there.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::premultiplied_image;
    use crate::ui::theme::white_alpha;
    use egui::epaint::{ClippedPrimitive, ImageDelta, Mesh, Primitive};
    use egui::{vec2, ColorImage, TextureOptions};

    const W: u32 = 128;
    const H: u32 = 96;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    /// The paper and the two inks from the report.
    const PAPER: [u8; 3] = [220, 204, 185];
    const LIGHT: [u8; 3] = [246, 240, 226];
    const DARK: [u8; 3] = [16, 25, 17];

    struct Gpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderer: egui_wgpu::Renderer,
        shared: Arc<Shared>,
        /// Uploaded so far, by id, as egui holds them.
        images: Vec<ColorImage>,
    }

    fn gpu() -> Option<Gpu> {
        if std::env::var_os("ANIMATOR_SKIP_GPU_TESTS").is_some() {
            eprintln!("SKIPPED: ANIMATOR_SKIP_GPU_TESTS is set");
            return None;
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("no GPU adapter: these tests need one (ANIMATOR_SKIP_GPU_TESTS=1 skips them)");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .expect("GPU device");
        eprintln!("ran on {}", adapter.get_info().name);
        let renderer = egui_wgpu::Renderer::new(&device, FORMAT, None, 1, DITHERING);
        let shared = Arc::new(Shared::new(&device, FORMAT, DITHERING));
        Some(Gpu {
            device,
            queue,
            renderer,
            shared,
            images: Vec::new(),
        })
    }

    /// A straight-alpha image uploaded the way the app uploads a cell.
    fn upload(g: &mut Gpu, size: [usize; 2], straight: &[u8]) -> TextureId {
        let image = premultiplied_image(size, straight);
        let tex = TextureId::Managed(g.images.len() as u64 + 1);
        let delta = ImageDelta::full(image.clone(), TextureOptions::LINEAR);
        g.renderer.update_texture(&g.device, &g.queue, tex, &delta);
        g.images.push(image);
        tex
    }

    fn image(g: &Gpu, tex: TextureId) -> &ColorImage {
        match tex {
            TextureId::Managed(i) => &g.images[i as usize - 1],
            TextureId::User(_) => unreachable!(),
        }
    }

    /// A 16×8 band of `ink`: transparent above and below and at both ends, a
    /// partly covered row on each side, solid in the middle — hard and soft
    /// edges, both ways.
    fn band(ink: [u8; 3]) -> Vec<u8> {
        let alpha = [0, 0, 90, 255, 255, 255, 140, 0];
        let mut px = Vec::new();
        for a in alpha {
            for x in 0..16 {
                let a = if (2..14).contains(&x) { a } else { 0 };
                px.extend_from_slice(&[ink[0], ink[1], ink[2], a]);
            }
        }
        px
    }

    /// 16×8 of seeded noise: a third clear, a third solid, a third between.
    fn speckle() -> Vec<u8> {
        let mut s = 0x9e37_79b9_u32;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        (0..16 * 8)
            .flat_map(|_| {
                let v = next();
                let a = match v % 3 {
                    0 => 0,
                    1 => 255,
                    _ => (v >> 24) as u8,
                };
                [(v >> 8) as u8, (v >> 16) as u8, (v >> 4) as u8, a]
            })
            .collect()
    }

    /// The corners of `size` texels scaled by `zoom` and turned `deg` about
    /// the centre of the screen.
    fn placed(size: [usize; 2], zoom: f32, deg: f32) -> [Pos2; 4] {
        let c = pos2(W as f32 / 2.0, H as f32 / 2.0);
        let (hw, hh) = (size[0] as f32 * zoom / 2.0, size[1] as f32 * zoom / 2.0);
        let (s, k) = deg.to_radians().sin_cos();
        [vec2(-hw, -hh), vec2(hw, -hh), vec2(hw, hh), vec2(-hw, hh)].map(|v| c + vec2(v.x * k - v.y * s, v.x * s + v.y * k))
    }

    /// Axis-aligned at 1:1, on whole pixels: every sample a texel centre.
    fn at(x: f32, y: f32, size: [usize; 2]) -> [Pos2; 4] {
        let (w, h) = (size[0] as f32, size[1] as f32);
        [pos2(x, y), pos2(x + w, y), pos2(x + w, y + h), pos2(x, y + h)]
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Via {
        Egui,
        Ours,
    }

    #[derive(Clone, Copy)]
    struct Draw {
        tex: TextureId,
        corners: [Pos2; 4],
        tint: Color32,
        clip: Rect,
        via: Via,
    }

    fn full() -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(W as f32, H as f32))
    }

    fn draw(tex: TextureId, corners: [Pos2; 4], tint: Color32, via: Via) -> Draw {
        Draw {
            tex,
            corners,
            tint,
            clip: full(),
            via,
        }
    }

    /// The scene drawn over the paper, read back, RGBA per pixel.
    fn render(g: &mut Gpu, draws: &[Draw]) -> Vec<[u8; 4]> {
        let jobs: Vec<ClippedPrimitive> = draws
            .iter()
            .map(|d| ClippedPrimitive {
                clip_rect: d.clip,
                primitive: match d.via {
                    Via::Egui => {
                        // `ui::shell::image_quad`'s mesh.
                        let mut mesh = Mesh::with_texture(d.tex);
                        let uv = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
                        for (pos, uv) in d.corners.into_iter().zip(uv) {
                            mesh.vertices.push(Vertex { pos, uv, color: d.tint });
                        }
                        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
                        Primitive::Mesh(mesh)
                    }
                    Via::Ours => {
                        let quad = g
                            .shared
                            .quad(&g.device, &g.renderer, d.tex, d.corners, d.tint)
                            .expect("uploaded");
                        Primitive::Callback(egui_wgpu::Callback::new_paint_callback(d.clip, quad))
                    }
                },
            })
            .collect();
        let screen = ScreenDescriptor {
            size_in_pixels: [W, H],
            pixels_per_point: 1.0,
        };
        let size = wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        };
        let target = g.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = g.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut commands = g.renderer.update_buffers(&g.device, &g.queue, &mut encoder, &jobs, &screen);
        {
            let paper = |c: u8| c as f64 / 255.0;
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: paper(PAPER[0]),
                                g: paper(PAPER[1]),
                                b: paper(PAPER[2]),
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            g.renderer.render(&mut pass, &jobs, &screen);
        }
        let row = (W * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = g.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(H),
                },
            },
            size,
        );
        commands.push(encoder.finish());
        g.queue.submit(commands);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("read back"));
        g.device.poll(wgpu::Maintain::Wait);
        let bytes = slice.get_mapped_range();
        (0..H as usize)
            .flat_map(|y| {
                let line = &bytes[y * row as usize..][..W as usize * 4];
                line.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect::<Vec<_>>()
            })
            .collect()
    }

    /// Largest per-channel difference between two renders.
    fn max_diff(a: &[[u8; 4]], b: &[[u8; 4]]) -> u8 {
        a.iter()
            .zip(b)
            .flat_map(|(p, q)| (0..4).map(move |c| p[c].abs_diff(q[c])))
            .max()
            .unwrap_or(0)
    }

    fn changed(px: &[[u8; 4]]) -> usize {
        px.iter().filter(|p| p[..3] != PAPER).count()
    }

    /// What pixel (x, y) should be with `tex` drawn over the paper at
    /// `corners`: the premultiplied texels mixed in gamma space (clamped at
    /// the edges), tinted, and blended in gamma — or `None` for a pixel near
    /// the quad's edge, where the rasterizer's rules decide coverage.
    fn reference(img: &ColorImage, corners: [Pos2; 4], tint: Color32, x: u32, y: u32) -> Option<[f32; 3]> {
        let p = pos2(x as f32 + 0.5, y as f32 + 0.5) - corners[0];
        let (ex, ey) = (corners[1] - corners[0], corners[3] - corners[0]);
        let det = ex.x * ey.y - ex.y * ey.x;
        let u = (p.x * ey.y - p.y * ey.x) / det;
        let v = (ex.x * p.y - ex.y * p.x) / det;
        let [w, h] = img.size;
        let (tx, ty) = (u * w as f32 - 0.5, v * h as f32 - 0.5);
        if !(0.0..=w as f32 - 1.0).contains(&tx) || !(0.0..=h as f32 - 1.0).contains(&ty) {
            return None;
        }
        let texel = |i: i32, j: i32| -> [f32; 4] {
            let c = img.pixels[j.clamp(0, h as i32 - 1) as usize * w + i.clamp(0, w as i32 - 1) as usize];
            c.to_array().map(|v| v as f32 / 255.0)
        };
        let (i, j) = (tx.floor() as i32, ty.floor() as i32);
        let (fx, fy) = (tx - tx.floor(), ty - ty.floor());
        let mix = |a: [f32; 4], b: [f32; 4], t: f32| -> [f32; 4] { std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t) };
        let s = mix(mix(texel(i, j), texel(i + 1, j), fx), mix(texel(i, j + 1), texel(i + 1, j + 1), fx), fy);
        let t = tint.to_array().map(|v| v as f32 / 255.0);
        let s: [f32; 4] = std::array::from_fn(|c| s[c] * t[c]);
        Some(std::array::from_fn(|c| (s[c] + PAPER[c] as f32 / 255.0 * (1.0 - s[3])) * 255.0))
    }

    /// The worst amount `px` sits above and away from the reference, over the
    /// pixels the reference covers, and how many it covers.
    fn against_reference(g: &Gpu, px: &[[u8; 4]], tex: TextureId, corners: [Pos2; 4], tint: Color32) -> (f32, f32, usize) {
        let (mut over, mut off, mut n) = (0.0f32, 0.0f32, 0);
        for y in 0..H {
            for x in 0..W {
                let Some(want) = reference(image(g, tex), corners, tint, x, y) else {
                    continue;
                };
                n += 1;
                let got = px[(y * W + x) as usize];
                for c in 0..3 {
                    let d = got[c] as f32 - want[c];
                    over = over.max(d);
                    off = off.max(d.abs());
                }
            }
        }
        (over, off, n)
    }

    /// At 1:1 on whole pixels every sample is a texel centre, where the two
    /// pipelines must agree: the canvas at 100% looks exactly as it did.
    #[test]
    fn matches_egui_at_texel_centres() {
        let Some(mut g) = gpu() else { return };
        let textures = [band(LIGHT), band(DARK), speckle()].map(|px| upload(&mut g, [16, 8], &px));
        for tex in textures {
            for tint in [white_alpha(255), white_alpha(128), white_alpha(31)] {
                for (x, y) in [(10.0, 7.0), (61.0, 40.0)] {
                    let corners = at(x, y, [16, 8]);
                    let egui = render(&mut g, &[draw(tex, corners, tint, Via::Egui)]);
                    let ours = render(&mut g, &[draw(tex, corners, tint, Via::Ours)]);
                    assert!(changed(&egui) > 0, "drew nothing");
                    assert!(max_diff(&egui, &ours) <= 1, "{tex:?} {tint:?} at ({x}, {y}): off by {}", max_diff(&egui, &ours));
                }
            }
        }
    }

    /// Dark ink barely showed the fault, so zoomed and turned it must look
    /// as it did: on the reference, and no further from egui's old pixels
    /// than egui's own error on this ink (3.5 levels at most, measured on an
    /// RTX 2060) plus one for rounding.
    #[test]
    fn dark_ink_matches_egui_when_zoomed() {
        let Some(mut g) = gpu() else { return };
        let tex = upload(&mut g, [16, 8], &band(DARK));
        for (zoom, deg) in [(3.0, 0.0), (1.5, 0.0), (0.5, 0.0), (3.0, 30.0)] {
            let corners = placed([16, 8], zoom, deg);
            let egui = render(&mut g, &[draw(tex, corners, white_alpha(255), Via::Egui)]);
            let ours = render(&mut g, &[draw(tex, corners, white_alpha(255), Via::Ours)]);
            assert!(changed(&ours) > 0, "drew nothing");
            let (_, off, n) = against_reference(&g, &ours, tex, corners, white_alpha(255));
            assert!(n > 0 && off <= 2.0, "{zoom}× {deg}°: {off:.1} off the reference");
            let (_, egui_off, _) = against_reference(&g, &egui, tex, corners, white_alpha(255));
            let moved = max_diff(&egui, &ours);
            assert!(
                moved as f32 <= egui_off.ceil() + 1.0 && moved <= 5,
                "{zoom}× {deg}°: moved {moved} from egui, whose own error is {egui_off:.1}"
            );
        }
    }

    /// The report: light ink over paper, between texels. egui's path comes
    /// out well above what the texels allow — the white rim — and ours sits
    /// on the gamma-space reference at every zoom and turn.
    #[test]
    fn light_ink_has_no_rim_at_any_zoom() {
        let Some(mut g) = gpu() else { return };
        let tex = upload(&mut g, [16, 8], &band(LIGHT));
        for (zoom, deg) in [(3.0, 0.0), (1.5, 0.0), (0.5, 0.0), (3.0, 30.0), (1.5, -12.0)] {
            let corners = placed([16, 8], zoom, deg);
            for tint in [white_alpha(255), white_alpha(128)] {
                let ours = render(&mut g, &[draw(tex, corners, tint, Via::Ours)]);
                let (_, off, n) = against_reference(&g, &ours, tex, corners, tint);
                assert!(n > 0, "{zoom}× {deg}°: no pixel checked");
                assert!(off <= 2.0, "{zoom}× {deg}° {tint:?}: {off:.1} off the reference");
            }
            if deg == 0.0 && zoom > 1.0 {
                let egui = render(&mut g, &[draw(tex, corners, white_alpha(255), Via::Egui)]);
                let (over, _, _) = against_reference(&g, &egui, tex, corners, white_alpha(255));
                assert!(over >= 15.0, "{zoom}×: egui's path shows no rim ({over:.1}), so this test can't see one");
            }
        }
    }

    /// Nothing lands outside egui's clip rect, though the viewport is reset.
    #[test]
    fn clip_rect_is_kept() {
        let Some(mut g) = gpu() else { return };
        let tex = upload(&mut g, [16, 8], &[255; 16 * 8 * 4]);
        let clip = Rect::from_min_max(pos2(30.0, 20.0), pos2(64.0, 60.0));
        let quad = Draw {
            clip,
            ..draw(tex, at(10.0, 10.0, [100, 70]), white_alpha(255), Via::Ours)
        };
        let px = render(&mut g, &[quad]);
        for y in 0..H {
            for x in 0..W {
                let inside = clip.contains(pos2(x as f32 + 0.5, y as f32 + 0.5));
                let p = px[(y * W + x) as usize];
                if inside {
                    assert_eq!(p[..3], [255, 255, 255], "({x}, {y}) inside the clip");
                } else {
                    assert_eq!(p[..3], PAPER, "({x}, {y}) outside the clip");
                }
            }
        }
    }

    /// Callbacks keep their place among egui's meshes: one painted after a
    /// quad covers it, and the quad covers one painted before.
    #[test]
    fn keeps_paint_order_with_meshes() {
        let Some(mut g) = gpu() else { return };
        let white = upload(&mut g, [1, 1], &[255; 4]);
        let ink = upload(&mut g, [16, 8], &band(LIGHT));
        let red = Color32::from_rgb(200, 0, 0);
        // The band spans x 40..88, y 36..60 at 3×; (64, 48) is in its solid
        // middle.
        let area = at(40.0, 30.0, [48, 30]);
        let quad = draw(ink, placed([16, 8], 3.0, 0.0), white_alpha(255), Via::Ours);
        let over = render(&mut g, &[quad, draw(white, area, red, Via::Egui)]);
        let under = render(&mut g, &[draw(white, area, red, Via::Egui), quad]);
        let p = (48 * W + 64) as usize;
        assert_eq!(over[p][..3], [200, 0, 0], "mesh painted after the quad");
        assert_ne!(under[p][..3], [200, 0, 0], "quad painted after the mesh");
    }
}
