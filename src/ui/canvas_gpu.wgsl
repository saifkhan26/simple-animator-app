// Canvas artwork quads. Everything except the texture read is egui-wgpu
// 0.29's own egui.wgsl, verbatim, so a quad lands on the same pixels with the
// same tint, blend and dither an egui mesh would.
//
// The texture read is the point of this pipeline. egui keeps every texture as
// Rgba8UnormSrgb, so its sampler mixes neighbouring texels in linear light.
// Our texels are premultiplied in gamma space, because that is where the
// framebuffer blends, and mixing those in linear light gives a colour
// brighter than its alpha allows wherever a sample falls between two texels:
// a white rim on light strokes. Here the four texels are loaded, turned back
// into the gamma values they were uploaded as, and mixed in gamma.

struct VertexOutput {
    @location(0) tex_coord: vec2<f32>,
    @location(1) color: vec4<f32>, // gamma 0-1
    @builtin(position) position: vec4<f32>,
};

struct Locals {
    screen_size: vec2<f32>,
    dithering: u32, // 1 if dithering is enabled, 0 otherwise
    // Uniform buffers need to be at least 16 bytes in WebGL.
    // See https://github.com/gfx-rs/wgpu/issues/2072
    _padding: u32,
};
@group(0) @binding(0) var<uniform> r_locals: Locals;

fn interleaved_gradient_noise(n: vec2<f32>) -> f32 {
    let f = 0.06711056 * n.x + 0.00583715 * n.y;
    return fract(52.9829189 * fract(f));
}

fn dither_interleaved(rgb: vec3<f32>, levels: f32, frag_coord: vec4<f32>) -> vec3<f32> {
    var noise = interleaved_gradient_noise(frag_coord.xy);
    // scale down the noise slightly to ensure flat colors aren't getting dithered
    noise = (noise - 0.5) * 0.95;
    return rgb + noise / (levels - 1.0);
}

// 0-1 linear  from  0-1 sRGB gamma
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

// 0-1 sRGB gamma  from  0-1 linear
fn gamma_from_linear_rgb(rgb: vec3<f32>) -> vec3<f32> {
    let cutoff = rgb < vec3<f32>(0.0031308);
    let lower = rgb * vec3<f32>(12.92);
    let higher = vec3<f32>(1.055) * pow(rgb, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(higher, lower, cutoff);
}

// [u8; 4] SRGB as u32 -> [r, g, b, a] in 0.-1
fn unpack_color(color: u32) -> vec4<f32> {
    return vec4<f32>(
        f32(color & 255u),
        f32((color >> 8u) & 255u),
        f32((color >> 16u) & 255u),
        f32((color >> 24u) & 255u),
    ) / 255.0;
}

fn position_from_screen(screen_pos: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(
        2.0 * screen_pos.x / r_locals.screen_size.x - 1.0,
        1.0 - 2.0 * screen_pos.y / r_locals.screen_size.y,
        0.0,
        1.0,
    );
}

@vertex
fn vs_main(
    @location(0) a_pos: vec2<f32>,
    @location(1) a_tex_coord: vec2<f32>,
    @location(2) a_color: u32,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coord = a_tex_coord;
    out.color = unpack_color(a_color);
    out.position = position_from_screen(a_pos);
    return out;
}

@group(1) @binding(0) var r_tex_color: texture_2d<f32>;

// One texel as uploaded: premultiplied, gamma. Out-of-range coordinates
// clamp to the edge, as egui's ClampToEdge sampler does.
fn texel_gamma(p: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    let c = textureLoad(r_tex_color, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0);
    return vec4<f32>(gamma_from_linear_rgb(c.rgb), c.a);
}

// What egui's linear sampler returns, re-encoded to gamma — except that the
// four texels are mixed in gamma space. At a texel centre the two agree.
fn sample_gamma(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(r_tex_color));
    let p = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let base = floor(p);
    let f = p - base;
    let i = vec2<i32>(base);
    let top = mix(texel_gamma(i, size), texel_gamma(i + vec2<i32>(1, 0), size), f.x);
    let bottom = mix(texel_gamma(i + vec2<i32>(0, 1), size), texel_gamma(i + vec2<i32>(1, 1), size), f.x);
    return mix(top, bottom, f.y);
}

@fragment
fn fs_main_linear_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_gamma = sample_gamma(in.tex_coord);
    var out_color_gamma = in.color * tex_gamma;
    if r_locals.dithering == 1 {
        let out_color_gamma_rgb = dither_interleaved(out_color_gamma.rgb, 256.0, in.position);
        out_color_gamma = vec4<f32>(out_color_gamma_rgb, out_color_gamma.a);
    }
    let out_color_linear = linear_from_gamma_rgb(out_color_gamma.rgb);
    return vec4<f32>(out_color_linear, out_color_gamma.a);
}

@fragment
fn fs_main_gamma_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_gamma = sample_gamma(in.tex_coord);
    var out_color_gamma = in.color * tex_gamma;
    if r_locals.dithering == 1 {
        let out_color_gamma_rgb = dither_interleaved(out_color_gamma.rgb, 256.0, in.position);
        out_color_gamma = vec4<f32>(out_color_gamma_rgb, out_color_gamma.a);
    }
    return out_color_gamma;
}
