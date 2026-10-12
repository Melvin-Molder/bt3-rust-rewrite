// `GfxPost_DrawDepthClut`: the depth buffer's spare byte (the plane of gs_depth.wgsl) through a 256-entry CLUT over
// the screen, pixel for pixel. The CLUT holds raw GS colours (alpha 0x80 = 1.0 stored as 0x80 / 255); the blend
// is (Cs - Cd) * As + Cd or, additive, Cs * As + Cd.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var plane: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var clut: texture_2d<f32>;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let byte = textureLoad(plane, vec2<i32>(in.position.xy), 0).r;
    let index = i32(round(byte * 255.0));
    let entry = textureLoad(clut, vec2<i32>(index, 0), 0);
    let alpha = entry.a * 255.0 / 128.0;
    return vec4<f32>(srgb_to_linear(entry.rgb), min(alpha, 1.0));
}
