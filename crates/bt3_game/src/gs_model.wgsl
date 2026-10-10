// A battle model batch as the GS draws it from VU1 program 0's packet (src/battle/obj_render.c):
// layer 0 is the model texture (TEX0_1, MODULATE with the vertex colour 0x80, alpha blending off),
// layer 1 the shading ramp (TEX0_2, DECAL) with ALPHA_2 0x62: Cd - Cs, clamped (COLCLAMP 1).
// The GS divides S / T by Q per pixel; the vertex carries (s, t) * q in uv and (u * q, 0, q) in its colour.
// Textures hold the raw GS colours, so the arithmetic is done on them and the result is handed on as linear.

#import bevy_sprite::{
    mesh2d_vertex_output::VertexOutput,
    mesh2d_view_bindings::view,
}

#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var shade_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var shade_sampler: sampler;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
#ifdef VERTEX_COLORS
#ifdef VERTEX_UVS
    let q = in.color.z;
    let base = textureSample(base_texture, base_sampler, in.uv / q).rgb;
    let shade = textureSample(shade_texture, shade_sampler, vec2<f32>(in.color.x / q, 0.5)).rgb;
    color = vec4<f32>(srgb_to_linear(clamp(base - shade, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
#endif
#endif
#ifdef TONEMAP_IN_SHADER
    color = tonemapping::tone_mapping(color, view.color_grading);
#endif
    return color;
}
