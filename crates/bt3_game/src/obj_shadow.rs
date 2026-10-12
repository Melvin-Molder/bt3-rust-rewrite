//! The fighters' ground shadows (`ObjShadow_*`, src/battle/obj_shadow.c; VU1 program 6, src/vu1/prog6.vsm).
//!
//! Every update (not while paused) the stage triangles under the object's bounds (widened 2.3 times across, 1.1
//! for a body scale above 50 or in split screen, which also throw the shadow straight down instead of along the
//! light's half vector) are collected with the shadow's strength (`StgShadow_Collect`). The draw first renders the
//! model, flattened on the plane through the object's position along that direction, into a 256x256 page seen
//! from 1000 below (`ObjShadow_BeginRender`: white on black, a one-pixel border left black), then draws the
//! collected triangles, lifted 0.02 along their normals, in the stage's light colour (alpha times the strength)
//! textured with the page: s, t = (x * scale + 1) / 2, (y * scale + 1) / 2 of the position in the page camera's
//! frame. Black texels have alpha 0 (TEXA's AEM), so only the silhouette draws. The GS's destination alpha test
//! that keeps two shadows from darkening the same pixel twice is not reproduced.

use bt3_data::model::CharaModel;
use bt3_data::stage_col::{ColBox, ShadowTri, StageCollision};

use crate::battle::{IDENTITY, Mtx, View, apply, inverse_rt, mul};
use crate::eft::{EffectBlend, EffectTri, EffectVertex};

/// `OBJ_SHADOW_TRI_MAX`.
pub(crate) const TRI_MAX: usize = 0x80;
/// The page is 256 x 256.
pub(crate) const PAGE: usize = 256;
/// `BtlObj_UpdateAll`'s light colour without the stage's.
pub(crate) const DEFAULT_COLOR: [f32; 4] = [10.0, 10.0, 40.0, 64.0];

/// `ObjShadow`: this frame's triangles, strength and direction.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Shadow {
    pub tris: Vec<ShadowTri>,
    pub alpha: f32,
    /// `sh->light`: the direction the model is flattened along.
    pub light: [f32; 3],
}

/// `ObjShadow_IsEnabled`: object flag 0x10000000, and no stage loading.
pub(crate) fn enabled(obj_flags: u32, loading: bool) -> bool {
    !loading && obj_flags & 0x1000_0000 != 0
}

/// `BtlObjLight_SetDir`'s half vector: the light direction and (0, -1.6, 0), halved, normalised.
pub(crate) fn light_half(dir: [f32; 3]) -> [f32; 3] {
    let d = norm(dir);
    norm([d[0] * 0.5, (d[1] - 1.6) * 0.5, d[2] * 0.5])
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l == 0.0 { v } else { v.map(|c| c / l) }
}

/// `ObjShadow_Update` with `ObjShadow_GetBox`: `bounds` is the object's box, `height` its body scale (the model
/// height), `zone` its stage zone, `broken` the mask of the broken stage objects.
pub(crate) fn update(
    bounds: &ColBox,
    height: f32,
    split: bool,
    zone: i32,
    light_dir: [f32; 3],
    col: &StageCollision,
    broken: u64,
) -> Shadow {
    let center: [f32; 3] = std::array::from_fn(|i| (bounds.max[i] + bounds.min[i]) * 0.5);
    let mut half: [f32; 3] = std::array::from_fn(|i| (bounds.max[i] - bounds.min[i]) * 0.5);
    let down = split || height > 50.0;
    let k = if down { 1.1 } else { 2.3 };
    half[0] *= k;
    half[2] *= k;
    let b = ColBox::from_center_half(center, half);
    let light = if down {
        [0.0, -1.0, 0.0]
    } else {
        light_half(light_dir)
    };
    let (tris, alpha) = col.shadow_collect(zone, &b, TRI_MAX, broken);
    Shadow { tris, alpha, light }
}

/// `ObjShadow_SetTarget`: the shadow area's half size and the texture scale by the body scale.
fn target_size(height: f32) -> (f32, f32) {
    if height <= 23.0 {
        (18.0, 0.055)
    } else if height <= 50.0 {
        (24.399_9, 0.042)
    } else if height <= 70.0 {
        (50.0, 0.021)
    } else {
        (85.0, 0.012)
    }
}

/// `ObjShadow_MakePlaneMtx`: flattens geometry on the plane (a, b, c, d + bias) along `light`.
fn plane_mtx(plane: [f32; 4], light: [f32; 3], bias: f32) -> Mtx {
    let [lx, ly, lz] = light.map(|c| -c);
    let [a, b, c, w] = plane;
    let d = -(w + bias);
    [
        [b * ly + c * lz, -a * ly, -a * lz, 0.0],
        [-b * lx, a * lx + c * lz, -b * lz, 0.0],
        [-c * lx, -c * ly, a * lx + b * ly, 0.0],
        [d * lx, d * ly, d * lz, a * lx + b * ly + c * lz],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The page's cameras (`ObjShadow_MakeLightMtx`, `ObjShadow_MakeProjMtx`) for an object at `target`: the
/// world-to-light-camera matrix the ground pass reads its texture coordinates with, and the world-to-page
/// projection, with the texture scale.
pub(crate) struct PageCams {
    pub view: Mtx,
    pub proj: Mtx,
    pub scale: f32,
}

pub(crate) fn page_cams(target: [f32; 3], height: f32) -> PageCams {
    let (size, scale) = target_size(height);
    let eye = [target[0], target[1] + 1000.0, target[2]];
    // Vu0Cur_RotateZXY(pi / 2, -pi / 2, 0) of the identity, at the eye: x along +z, y along -x, z along -y.
    let mut m = IDENTITY;
    m[0] = [0.0, 0.0, 1.0, 0.0];
    m[1] = [-1.0, 0.0, 0.0, 0.0];
    m[2] = [0.0, -1.0, 0.0, 0.0];
    m[3] = [eye[0], eye[1], eye[2], 1.0];
    let view = inverse_rt(&m);
    // Mtx_MakeViewScreen with ObjShadow_CalcScreenDist, centre 2048, and Mtx_MakeCamera(eye, dir, dir x z).
    let d = sub(target, eye);
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let s = size / dist;
    let scrz = dist * (1.0 - s * s).sqrt() * 254.0 / (size + size);
    let (z_min, z_max, near, far) = (1.0f32, 10_877_000.0f32, 1.0f32, 10_877_000.0f32);
    let cz = (-z_max * near + z_min * far) / (-near + far);
    let az = far * near * (-z_min + z_max) / (-near + far);
    let mut mp = IDENTITY;
    mp[0][0] = scrz;
    mp[1][1] = scrz;
    mp[2][2] = 0.0;
    mp[3][3] = 0.0;
    mp[3][2] = 1.0;
    mp[2][3] = 1.0;
    let mut mt = IDENTITY;
    mt[2][2] = az;
    mt[3] = [2048.0, 2048.0, cz, 1.0];
    let screen = mul(&mp, &mt);
    let dir = norm(d);
    let side = norm(cross(dir, [0.0, 0.0, 1.0]));
    let x = norm(cross(side, dir));
    let y = cross(dir, x);
    let cam_m: Mtx = [
        [x[0], x[1], x[2], 0.0],
        [y[0], y[1], y[2], 0.0],
        [dir[0], dir[1], dir[2], 0.0],
        [eye[0], eye[1], eye[2], 1.0],
    ];
    let cam = inverse_rt(&cam_m);
    PageCams {
        view,
        proj: mul(&cam, &screen),
        scale,
    }
}

/// The world triangles of a model's parts as `draw` would draw them (each enabled part whose node is shown, the
/// strips' triangles skinned between the node and its parent).
pub(crate) fn model_tris(
    model: &CharaModel,
    nodes: &[crate::btl_obj::Node],
    shown: impl Fn(u16) -> bool,
    mut out: impl FnMut([[f32; 3]; 3]),
) {
    for (part, node) in model.parts.iter().zip(nodes) {
        if !part.enabled || !shown(part.node) {
            continue;
        }
        for batch in &part.batches {
            let mut prev: [Option<[f32; 3]>; 2] = [None, None];
            for v in &batch.vertices {
                let [x, y, z, w] = v.position;
                let at = |m: &Mtx, o: [f32; 4]| apply(m, [x - o[0], y - o[1], z - o[2], 1.0]);
                let pa = at(&node.mtx, part.offset_a);
                let pb = at(&node.parent, part.offset_b);
                let p = std::array::from_fn(|i| pb[i] + (pa[i] - pb[i]) * w);
                if let [Some(a), Some(b)] = prev {
                    out([a, b, p]);
                }
                prev = [prev[1], Some(p)];
            }
        }
    }
}

/// `ObjShadow_BeginRender`'s page: the triangles flattened along `light` on the plane through `target` and
/// projected with `cams`, drawn white into a cleared page inside the one-pixel border (SCISSOR 1..254, XYOFFSET
/// 1920). Raw GS colours with alpha doubled (the TEXA's 0x80 for every non-black texel): 0x80 grey at 255, black at
/// 0.
pub(crate) fn render_page(
    cams: &PageCams,
    target: [f32; 3],
    light: [f32; 3],
    tris: impl IntoIterator<Item = [[f32; 3]; 3]>,
) -> Vec<u8> {
    let flat = plane_mtx([0.0, -1.0, 0.0, target[1]], light, -0.01);
    let m = mul(&flat, &cams.proj);
    let mut cover = vec![false; PAGE * PAGE];
    for tri in tris {
        let p = tri.map(|v| {
            let s = apply(&m, [v[0], v[1], v[2], 1.0]);
            let q = 1.0 / s[3];
            [
                (s[0] * q * 16.0).trunc() / 16.0 - 1920.0,
                (s[1] * q * 16.0).trunc() / 16.0 - 1920.0,
            ]
        });
        fill_tri(&mut cover, p);
    }
    let mut pixels = vec![0u8; PAGE * PAGE * 4];
    for (px, &c) in pixels.chunks_exact_mut(4).zip(&cover) {
        if c {
            px.copy_from_slice(&[0x80, 0x80, 0x80, 0xFF]);
        }
    }
    pixels
}

/// The pixels whose sampling point (the integer coordinate) the triangle covers, top-left rule, within the
/// scissor 1..254.
fn fill_tri(cover: &mut [bool], p: [[f32; 2]; 3]) {
    let area =
        (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
    if area == 0.0 || !area.is_finite() {
        return;
    }
    // Counter-clockwise order for the edge tests.
    let p = if area < 0.0 { [p[0], p[2], p[1]] } else { p };
    let min_x = p
        .iter()
        .map(|v| v[0])
        .fold(f32::MAX, f32::min)
        .ceil()
        .max(1.0) as i32;
    let max_x = p
        .iter()
        .map(|v| v[0])
        .fold(f32::MIN, f32::max)
        .floor()
        .min(254.0) as i32;
    let min_y = p
        .iter()
        .map(|v| v[1])
        .fold(f32::MAX, f32::min)
        .ceil()
        .max(1.0) as i32;
    let max_y = p
        .iter()
        .map(|v| v[1])
        .fold(f32::MIN, f32::max)
        .floor()
        .min(254.0) as i32;
    let edge = |a: [f32; 2], b: [f32; 2], x: f32, y: f32| {
        let e = (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
        // Top-left rule: points on a top or left edge are inside, on a bottom or right edge outside.
        let top_left = (b[1] - a[1]) < 0.0 || ((b[1] - a[1]) == 0.0 && (b[0] - a[0]) > 0.0);
        e > 0.0 || (e == 0.0 && top_left)
    };
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (fx, fy) = (x as f32, y as f32);
            if edge(p[0], p[1], fx, fy) && edge(p[1], p[2], fx, fy) && edge(p[2], p[0], fx, fy) {
                cover[y as usize * PAGE + x as usize] = true;
            }
        }
    }
}

/// `ObjShadow_Draw`: the collected triangles, lifted 0.02 along their normals, in `color` (alpha times the
/// strength), with program 6's texture coordinates, transformed and clipped as the stage is.
pub(crate) fn ground_tris(
    shadow: &Shadow,
    cams: &PageCams,
    color: [f32; 4],
    view: &View,
    texture: usize,
) -> Vec<EffectTri> {
    let rgba = [color[0], color[1], color[2], color[3] * shadow.alpha];
    let tris: Vec<_> = shadow
        .tris
        .iter()
        .map(|t| {
            t.v.map(|v| {
                let p = std::array::from_fn(|k| v[k] + t.nrm[k] * 0.02);
                let c = apply(&cams.view, [p[0], p[1], p[2], 1.0]);
                let st = [
                    (c[0] * cams.scale + 1.0) * 0.5,
                    (c[1] * cams.scale + 1.0) * 0.5,
                    1.0,
                ];
                (p, rgba, st)
            })
        })
        .collect();
    let screen = crate::stage_draw::draw_triangles(&tris, view);
    screen
        .chunks_exact(3)
        .map(|t| EffectTri {
            texture: Some(texture),
            blend: EffectBlend::Alpha,
            clamp: true,
            zwrite: false,
            alpha_ref: 0,
            vertices: [0, 1, 2].map(|i| EffectVertex {
                position: t[i].position,
                stq: t[i].stq,
                rgba: t[i].rgba.map(|c| c as i32 as u8),
            }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_and_ground_lookup_agree() {
        // A point offset from the target lands on the page where the ground pass looks it up.
        let target = [100.0, -20.0, 50.0];
        let cams = page_cams(target, 20.0);
        for d in [[5.0, 0.0, 0.0], [0.0, 0.0, 7.0], [-3.0, 0.0, 4.0]] {
            let p = [target[0] + d[0], target[1], target[2] + d[2]];
            let s = apply(&cams.proj, [p[0], p[1], p[2], 1.0]);
            let page = [s[0] / s[3] - 1920.0, s[1] / s[3] - 1920.0];
            let c = apply(&cams.view, [p[0], p[1], p[2], 1.0]);
            let st = [
                (c[0] * cams.scale + 1.0) * 0.5,
                (c[1] * cams.scale + 1.0) * 0.5,
            ];
            for k in 0..2 {
                assert!(
                    (page[k] - st[k] * 256.0).abs() < 0.2,
                    "{d:?}: {page:?} {st:?}"
                );
            }
        }
        // Straight down a point keeps its x / z; along a slanted light it moves.
        let flat = plane_mtx([0.0, -1.0, 0.0, target[1]], [0.0, -1.0, 0.0], -0.01);
        let q = apply(&flat, [101.0, -40.0, 52.0, 1.0]);
        assert!((q[0] / q[3] - 101.0).abs() < 1e-3 && (q[2] / q[3] - 52.0).abs() < 1e-3);
        assert!((q[1] / q[3] - (target[1] - 0.01)).abs() < 1e-3);
    }

    #[test]
    fn pages_fill_inside_the_border() {
        let mut cover = vec![false; PAGE * PAGE];
        fill_tri(&mut cover, [[-10.0, -10.0], [300.0, -10.0], [-10.0, 300.0]]);
        assert!(!cover[0] && cover[PAGE + 1] && !cover[255] && !cover[PAGE * 255]);
        let mut cover = vec![false; PAGE * PAGE];
        fill_tri(&mut cover, [[10.0, 10.0], [20.0, 10.0], [10.0, 20.0]]);
        assert_eq!(cover.iter().filter(|&&c| c).count(), 55);
    }

    #[test]
    fn strength_and_size_follow_the_height() {
        assert_eq!(bt3_data::stage_col::shadow_alpha(10.0), 1.0);
        assert_eq!(bt3_data::stage_col::shadow_alpha(35.0), 0.5);
        assert_eq!(bt3_data::stage_col::shadow_alpha(60.0), 0.0);
        assert_eq!(target_size(20.0), (18.0, 0.055));
        assert_eq!(target_size(80.0), (85.0, 0.012));
    }
}
