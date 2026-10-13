// `ObjOutline_Draw` (src/sys/gfx_post.c) over the objects' ids (the depth buffer's spare byte after
// `GfxPost_CopyAlphaToDepth`). The ids go through the outline's CLUT (`ObjOutline_BuildClut`: id << 3, 0x80 for
// 0, nothing for 0xFF) into a 16-bit work page, whose 5-bit channels keep `k`: the id's low five bits, 16 for those
// that are 0, 0 for no object. Two shifted copies are subtracted from it (ALPHA 0x62, no clamp): the half-pixel
// vertical shift samples the same texel (the GS samples at the pixel's corner, point sampled), so only the copy one
// pixel to the left, which reads the next texel on, leaves anything: a pixel whose right neighbour's `k` differs
// is an edge. The edges are drawn into the frame's alpha (TEXA: 0 where nothing is left) three times: two pixels
// up and in place at 0x30, then one up at 0x80; a grey 0x64 is then taken from the frame by that alpha
// (ALPHA 0x52, `Cd - Cs * Ad`), which the blend does here.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var plane: texture_2d<f32>;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn k(p: vec2<i32>) -> i32 {
    let size = vec2<i32>(textureDimensions(plane));
    let id = i32(round(textureLoad(plane, clamp(p, vec2<i32>(0), size - 1), 0).r * 255.0));
    if id == 255 {
        return 0;
    }
    if ((id << 3u) & 0xFF) == 0 {
        return 16;
    }
    return id & 31;
}

fn edge(p: vec2<i32>) -> bool {
    return k(p) != k(p + vec2<i32>(1, 0));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.position.xy);
    var alpha = 0.0;
    if edge(p + vec2<i32>(0, 1)) {
        alpha = 128.0;
    } else if edge(p + vec2<i32>(0, 2)) || edge(p) {
        alpha = 48.0;
    }
    return vec4<f32>(srgb_to_linear(vec3<f32>(100.0 / 255.0)), alpha / 128.0);
}
