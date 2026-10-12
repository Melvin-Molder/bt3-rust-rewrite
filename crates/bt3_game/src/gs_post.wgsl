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
// A masked op (params.x 1) is blended by the alpha its mask gives the depth byte at the target pixel (the frame's
// alpha after a depth CLUT pass), 0x80 = 1.0; params.yz scale a target pixel to the plane's.
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var plane: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<uniform> params: vec4<f32>;
// The source texels read (x0, y0, x1, y1): the GS's region clamp.
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var<uniform> region: vec4<f32>;

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
    let lo = max(vec2<i32>(region.xy), vec2<i32>(0));
    let hi = min(vec2<i32>(region.zw), size) - 1;
    return linear_to_srgb(textureLoad(post_texture, clamp(p, lo, hi), 0).rgb);
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
    var alpha = in.color.a / 128.0;
    if params.x > 0.5 {
        let byte = textureLoad(plane, vec2<i32>(in.position.xy * params.yz), 0).r;
        alpha = textureLoad(mask, vec2<i32>(i32(round(byte * 255.0)), 0), 0).a * 255.0 / 128.0;
    }
    color = vec4<f32>(srgb_to_linear(rgb), min(alpha, 1.0));
#endif
#ifdef TONEMAP_IN_SHADER
    color = tonemapping::tone_mapping(color, view.color_grading);
#endif
    return color;
}
