// An effect triangle of the sorted pass (src/sys/gfx_ot.c): TEX0 MODULATE with the texture's alpha (Cf = Ct * Cv /
// 128, Af = At * Av / 128) or the vertex colour alone when untextured, bilinear, clamped; colour clamp on. The
// texture holds raw GS colours with alpha doubled (0x80 -> 255); vertex colours are raw 0..255 values (128 = 1.0 when
// modulating, the colour itself when untextured).
// The vertex carries (s, t, q) in its tangent; the GS divides S and T by Q per pixel.
// params.x: textured; params.y: 1 for ALPHA 0x49 (Cd * As + Cd), where the blend multiplies the destination by
// what this outputs, so the output is As itself; params.z: 1 for ALPHA 0x48 / 0x42 (Cd +/- Cs * As), where the
// colour is multiplied by As here, as the GS does, alpha above 0x80 included. params.w: the alpha test's AREF (TEST
// ATST GREATER; 0 none): a pixel whose alpha (x 128) is not above it is dropped.

#import bevy_sprite::{
    mesh2d_vertex_output::VertexOutput,
    mesh2d_view_bindings::view,
}

#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var effect_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var effect_sampler: sampler;
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
    var rgb = in.color.rgb / 255.0;
    var alpha = in.color.a / 128.0;
    if params.x > 0.5 {
        let stq = in.world_tangent.xyz;
        let texel = textureSample(effect_texture, effect_sampler, stq.xy / stq.z);
        rgb = min(texel.rgb * in.color.rgb / 128.0, vec3<f32>(1.0));
        alpha = texel.a * in.color.a / 128.0;
    }
    if params.w > 0.0 && alpha * 128.0 <= params.w {
        discard;
    }
    if params.y > 0.5 {
        color = vec4<f32>(vec3<f32>(alpha), 0.0);
    } else if params.z > 0.5 {
        color = vec4<f32>(srgb_to_linear(min(rgb * alpha, vec3<f32>(1.0))), 0.0);
    } else {
        color = vec4<f32>(srgb_to_linear(rgb), min(alpha, 1.0));
    }
#endif
#endif
#ifdef TONEMAP_IN_SHADER
    if params.y < 0.5 && params.z < 0.5 {
        color = tonemapping::tone_mapping(color, view.color_grading);
    }
#endif
    return color;
}
