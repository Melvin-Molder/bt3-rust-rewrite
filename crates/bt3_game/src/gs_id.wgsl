// What a battle model batch leaves in the frame's alpha (src/battle/obj_render.c, src/battle/obj_shadow.c): the
// model texture's alpha, which is its CLUT entry's (TEX0_1 MODULATE with the vertex alpha 0x80, ALPHA_1 0x64 and
// FBA 0 write it as it is). `BtlObjMdl_InitAlphaSlots` made 16 entries of each material texture's CLUT the slot's
// id (the slot + the object's row * 15); the id texture holds each texel's alpha in r and, in g, whether its entry
// is one of those (r then the slot). The texels are filtered as the colour is (bilinear, wrapping), the result
// truncated. A dimmed draw (`ObjDraw_AddClutPass`) gives every texel one alpha. The byte goes out in r.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var ids: texture_2d<f32>;
// x: the object's row * 15; y: the dimmed alpha; z: 1 for a dimmed draw.
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> params: vec4<f32>;

fn texel_id(p: vec2<i32>, size: vec2<i32>) -> f32 {
    let q = ((p % size) + size) % size;
    let t = textureLoad(ids, q, 0);
    var id = round(t.r * 255.0);
    if t.g > 0.5 {
        id = id + params.x;
    }
    return id;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if params.z > 0.5 {
        return vec4<f32>(params.y / 255.0, 0.0, 0.0, 1.0);
    }
    var id = 255.0;
#ifdef VERTEX_COLORS
#ifdef VERTEX_UVS
    let size = vec2<i32>(textureDimensions(ids));
    let t = in.uv / in.color.z * vec2<f32>(size) - 0.5;
    let i0 = floor(t);
    let f = t - i0;
    let p = vec2<i32>(i0);
    let top = mix(texel_id(p, size), texel_id(p + vec2<i32>(1, 0), size), f.x);
    let bottom = mix(texel_id(p + vec2<i32>(0, 1), size), texel_id(p + vec2<i32>(1, 1), size), f.x);
    // (the epsilon keeps equal texels exact through the float weights)
    id = floor(mix(top, bottom, f.y) + 0.001);
#endif
#endif
    return vec4<f32>(id / 255.0, 0.0, 0.0, 1.0);
}
