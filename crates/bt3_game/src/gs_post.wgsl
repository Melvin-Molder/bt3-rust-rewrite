// A post-pass triangle (src/battle/screen_fx.c, src/battle/stg_ambient.c): the source buffer (the frame or a work
// buffer) under TEX0 TCC 0 MODULATE, bilinear (TEX1 0x60), clamped: Cf = Ct * Cv / 128 and Af = Av / 128. The
// buffers hold raw GS colours sRGB-encoded (a load decodes them), vertex colours are raw 0..255 values. The texel is
// filtered here from four loads, so the buffers keep their own samplers.

#import bevy_sprite::{
    mesh2d_vertex_output::VertexOutput,
    mesh2d_view_bindings::view,
}

#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var post_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var post_sampler: sampler;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

fn texel(p: vec2<i32>, size: vec2<i32>) -> vec3<f32> {
    return linear_to_srgb(textureLoad(post_texture, clamp(p, vec2<i32>(0), size - 1), 0).rgb);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
#ifdef VERTEX_COLORS
    let size = vec2<i32>(textureDimensions(post_texture));
    let pos = in.uv * vec2<f32>(size) - 0.5;
    let base = floor(pos);
    let f = pos - base;
    let p = vec2<i32>(base);
    let top = mix(texel(p, size), texel(p + vec2<i32>(1, 0), size), f.x);
    let bottom = mix(texel(p + vec2<i32>(0, 1), size), texel(p + vec2<i32>(1, 1), size), f.x);
    let raw = mix(top, bottom, f.y);
    let rgb = min(raw * in.color.rgb / 128.0, vec3<f32>(1.0));
    color = vec4<f32>(srgb_to_linear(rgb), min(in.color.a / 128.0, 1.0));
#endif
#ifdef TONEMAP_IN_SHADER
    color = tonemapping::tone_mapping(color, view.color_grading);
#endif
    return color;
}
