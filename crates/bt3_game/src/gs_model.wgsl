// A battle model batch as the GS draws it from VU1 program 0's packet (src/battle/obj_render.c):
// layer 0 is the model texture (TEX0_1, MODULATE with the vertex colour, the view's; alpha blending off),
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

// The colour mode (ObjDraw_AddClutPass, ObjDraw_SetLight): the palettes lerp towards fade.rgb by fade.w (ALPHA
// 0x64), then get tint.rgb * tint.w and flash.rgb * flash.w added (ALPHA 0x68), each clamped as the GS does;
// flags.x draws the texture unshaded.
struct ModelLook {
    fade: vec4<f32>,
    tint: vec4<f32>,
    flash: vec4<f32>,
    flags: vec4<f32>,
    color: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<uniform> look: ModelLook;

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
    var base = textureSample(base_texture, base_sampler, in.uv / q).rgb;
    base = clamp(base + (look.fade.rgb - base) * look.fade.w, vec3<f32>(0.0), vec3<f32>(1.0));
    base = min(base + look.tint.rgb * look.tint.w, vec3<f32>(1.0));
    base = min(base + look.flash.rgb * look.flash.w, vec3<f32>(1.0));
    // MODULATE: the texel times the vertex colour (the view's) / 128, clamped.
    base = min(base * look.color.rgb, vec3<f32>(1.0));
    let shade = textureSample(shade_texture, shade_sampler, vec2<f32>(in.color.x / q, 0.5)).rgb;
    var lit = clamp(base - shade, vec3<f32>(0.0), vec3<f32>(1.0));
    if look.flags.x > 0.5 {
        lit = base;
    }
    color = vec4<f32>(srgb_to_linear(lit), 1.0);
#endif
#endif
#ifdef TONEMAP_IN_SHADER
    color = tonemapping::tone_mapping(color, view.color_grading);
#endif
    return color;
}
