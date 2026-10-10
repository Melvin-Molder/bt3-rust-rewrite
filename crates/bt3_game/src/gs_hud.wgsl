// A HUD sprite as HudSprite_DrawAt sends it (src/battle/hud_sprite.c): TEX0 MODULATE + TCC with bilinear filtering
// (Cf = Ct * Cv / 128, Af = At * Av / 128), or the vertex colour alone when untextured; colour clamp on. The
// textures hold raw GS colours with alpha doubled (0x80 -> 255); vertex colours are raw 0..255 values.
// With a mask (ALPHA 0x58, Cs * Ad + Cd) the colour is weighted by the alpha the mask texture wrote, sampled at the
// tangent's xy, times params.z, and blended additively.

#import bevy_sprite::{
    mesh2d_vertex_output::VertexOutput,
    mesh2d_view_bindings::view,
}

#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var hud_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var hud_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var mask_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var mask_sampler: sampler;
// x: textured, y: masked, z: the mask's vertex alpha (1.0 = 0x80), w: the CPU-drawn layer (premultiplied: colour
// P, alpha 1 - K of P + K * background).
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<uniform> params: vec4<f32>;

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
        let texel = textureSample(hud_texture, hud_sampler, in.uv);
        rgb = min(texel.rgb * in.color.rgb / 128.0, vec3<f32>(1.0));
        alpha = texel.a * in.color.a / 128.0;
    }
    if params.w > 0.5 {
        let texel = textureSample(hud_texture, hud_sampler, in.uv);
        color = vec4<f32>(srgb_to_linear(texel.rgb), texel.a);
    } else if params.y > 0.5 {
        let weight = textureSample(mask_texture, mask_sampler, in.world_tangent.xy).a * params.z;
        color = vec4<f32>(srgb_to_linear(min(rgb * weight, vec3<f32>(1.0))), 0.0);
    } else {
        color = vec4<f32>(srgb_to_linear(rgb), min(alpha, 1.0));
    }
#endif
#endif
#ifdef TONEMAP_IN_SHADER
    color = tonemapping::tone_mapping(color, view.color_grading);
#endif
    return color;
}
