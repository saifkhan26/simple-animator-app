// The art stack: the canvas's artwork drawn off-screen, layer by layer, when
// a layer on show has a blend mode — then laid on the window in one go, so a
// blend mixes with the art alone and never with the paper under it.
//
// Appended to canvas_gpu.wgsl, whose vertex shader and gamma-space texture
// read it shares, after a line that sets MODE:
//     const MODE: u32 = <BlendMode::shader_index>u;
//
// The off-screen texture is Rgba8Unorm: premultiplied, gamma, as the CPU
// compositor works, so a blend comes out as an export of it does.

// A copy of the stack so far, under the quad being blended: what it mixes
// with. Read texel for texel at the fragment's own pixel.
@group(2) @binding(0) var r_backdrop: texture_2d<f32>;

// A Normal layer: premultiplied over, by the pipeline's blend state.
@fragment
fn fs_stack_normal(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color * sample_gamma(in.tex_coord);
}

fn unmultiplied(c: vec4<f32>) -> vec3<f32> {
    if c.a <= 0.0 {
        return vec3<f32>(0.0);
    }
    return clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0));
}

// `BlendMode::mix` in doc/blend.rs, case for case: one channel of the layer,
// cs, mixed with the same channel under it, cb, both unmultiplied.
fn mix_channel(cb: f32, cs: f32) -> f32 {
    switch MODE {
        // Multiply
        case 1u: {
            return cb * cs;
        }
        // Screen
        case 2u: {
            return cb + cs - cb * cs;
        }
        // Add
        case 3u: {
            return min(cb + cs, 1.0);
        }
        // Overlay: hard light with the layers swapped.
        case 4u: {
            if cb <= 0.5 {
                return 2.0 * cb * cs;
            }
            let b = 2.0 * cb - 1.0;
            return b + cs - b * cs;
        }
        default: {
            return cs;
        }
    }
}

// A layer in MODE over the stack so far, composited as the W3C spec (and
// doc/blend.rs) does it. Written, not blended: the backdrop is in the sum.
@fragment
fn fs_stack_blend(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = in.color * sample_gamma(in.tex_coord);
    let b = textureLoad(r_backdrop, vec2<i32>(in.position.xy), 0);
    let cs = unmultiplied(s);
    let cb = unmultiplied(b);
    let m = vec3<f32>(mix_channel(cb.r, cs.r), mix_channel(cb.g, cs.g), mix_channel(cb.b, cs.b));
    let rgb = s.rgb * (1.0 - b.a) + b.rgb * (1.0 - s.a) + s.a * b.a * m;
    let a = s.a + b.a * (1.0 - s.a);
    return vec4<f32>(rgb, a);
}

// The finished stack, laid on the window as egui lays a texel: dithered, and
// for an sRGB window turned to linear. The stack texture is bound where a
// drawing's would be, and read raw — it is gamma already.
fn stack_texel(in: VertexOutput) -> vec4<f32> {
    var c = textureLoad(r_tex_color, vec2<i32>(in.position.xy), 0);
    if r_locals.dithering == 1 {
        c = vec4<f32>(dither_interleaved(c.rgb, 256.0, in.position), c.a);
    }
    return c;
}

@fragment
fn fs_blit_gamma_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    return stack_texel(in);
}

@fragment
fn fs_blit_linear_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = stack_texel(in);
    return vec4<f32>(linear_from_gamma_rgb(c.rgb), c.a);
}
