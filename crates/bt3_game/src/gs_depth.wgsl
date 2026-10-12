// The spare byte of the GS depth buffer as `GfxDepthFog_Draw` leaves it (src/sys/gfx_screen.c): a stage triangle
// that writes depth (with the stage pass's alpha test) stores 255 - (Z >> 8) where its depth is below 0xFFA8, else
// 0; the target is cleared to 255 (no depth: the sky). The vertex position's z is the 2D pass's mapping of the GS Z
// (-950 + Z / 2^24 * 900).

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var stage_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var stage_sampler: sampler;
// x: the alpha test reference as a fraction of 0x80 (0 = no test).
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> params: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var byte = 0.0;
#ifdef VERTEX_COLORS
#ifdef VERTEX_TANGENTS
    let stq = in.world_tangent.xyz;
    let texel = textureSample(stage_texture, stage_sampler, stq.xy / stq.z);
    let alpha = texel.a * in.color.a / 128.0;
    if alpha < params.x {
        discard;
    }
#endif
#endif
    let z = (in.world_position.z + 950.0) / 900.0 * 16777216.0;
    if z < 65448.0 {
        byte = 255.0 - floor(z / 256.0);
    }
    return vec4<f32>(byte / 255.0, 0.0, 0.0, 1.0);
}
