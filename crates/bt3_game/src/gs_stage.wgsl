// A stage triangle as the GS draws it from VU1 program 4's packet (src/battle/stg_model_draw.c): the stage texture
// with TEX0 MODULATE + TCC, the vertex colour (128 = 1.0), ALPHA_1 0x44 (blended by the source alpha, 128 = 1.0)
// and TEST_1's alpha test (AFAIL keep: nothing written). The vertex carries (s, t, q) / w in its tangent; the GS
// divides S and T by Q per pixel. The texture holds raw GS colours with alpha doubled (0x80 -> 255).

#import bevy_sprite::{
    mesh2d_vertex_output::VertexOutput,
    mesh2d_view_bindings::view,
}

#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var stage_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var stage_sampler: sampler;
// x: the alpha test reference as a fraction of 0x80 (0 = no test).
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> params: vec4<f32>;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
#ifdef VERTEX_COLORS
#ifdef VERTEX_TANGENTS
    let stq = in.world_tangent.xyz;
    let texel = textureSample(stage_texture, stage_sampler, stq.xy / stq.z);
    let rgb = min(texel.rgb * in.color.rgb / 128.0, vec3<f32>(1.0));
    let alpha = texel.a * in.color.a / 128.0;
    if alpha < params.x {
        discard;
    }
    color = vec4<f32>(srgb_to_linear(rgb), min(alpha, 1.0));
#endif
#endif
#ifdef TONEMAP_IN_SHADER
    color = tonemapping::tone_mapping(color, view.color_grading);
#endif
    return color;
}
