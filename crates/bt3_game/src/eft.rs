//! The visual side of the effect scene: the emitter sets a projectile drives (`EftEmit_*`, src/battle/eft_emit.c
//! and eft_sweep.c), the particle modules they start, and the depth-sorted pass the modules draw into (the ordering
//! table of src/sys/gfx_ot.c, with the clipper and projection of src/sys/vu0_math.c).
//!
//! Ported modules: kind 14, the keyframed sprite animations (`EftAnimPart_*` of src/battle/eft_char_parts.c playing
//! `EftSprAnim_*` of src/battle/eft_sprite_anim.c). The other kinds start nothing yet.
//!
//! The draw pass: every module queues GS triangles into a slot of the ordering table (a depth slot 0..0xFFF from
//! the projected z, far ones first, or the first / last slot) and one of its two layers; `Ot_Draw` sends them in
//! slot order, layer 0 before layer 1, in queueing order within a chain. The layer and the GS context pick the
//! blend: layer 0 normal (0x44) / subtractive (0x42), layer 1 additive (0x48) / destination-doubling (0x49). The
//! pass tests depth (GEQUAL) without writing it and filters bilinearly.

use std::f32::consts::PI;

use bt3_data::eft_pack::{
    BillDef, BillDef2, EmitSet, PtclDef, PtclDef2, QuadDef, QuadDef2, RibbonAnim, RibbonPrm,
    SprPack,
};

use crate::battle::{IDENTITY, Mtx, apply, inverse_rt};
use crate::eft_bill::{B_DEAD, B_FRONT, BILLS, Bill};
use crate::eft_ptcl::{EMITTERS, Emitter, W_DEAD, W_DETACH, W_FRONT};
use crate::eft_quad::{E_DEAD, E_OWN_ORIGIN, E_VIEW_ONLY, Pool, QuadEmitter, Rng};
use crate::eft_ribbon::{R_DEAD, R_STOP, RIBBON_NODES, RIBBONS, Ribbon};
use crate::rng::{LibcRandom, Vu0Random};

pub(crate) type V3 = [f32; 3];
pub(crate) type V4 = [f32; 4];

/// `gEftEmitNodeSlot`: the node slot of each emitter node index (-1: the given position).
const NODE_SLOT: [i32; 8] = [0, 1, 3, 4, 5, -1, -1, 0];
/// `gEftSprGridUv*` and `gEftSprSubdivCorner` (read from the executable).
const GRID_2X2: [[f32; 4]; 4] = [
    [0.0, 0.0, 0.5, 0.5],
    [0.5, 0.0, 1.0, 0.5],
    [0.0, 0.5, 0.5, 1.0],
    [0.5, 0.5, 1.0, 1.0],
];
const GRID_4X2: [[f32; 4]; 8] = [
    [0.0, 0.0, 0.25, 0.5],
    [0.25, 0.0, 0.5, 0.5],
    [0.5, 0.0, 0.75, 0.5],
    [0.75, 0.0, 1.0, 0.5],
    [0.0, 0.5, 0.25, 1.0],
    [0.25, 0.5, 0.5, 1.0],
    [0.5, 0.5, 0.75, 1.0],
    [0.75, 0.5, 1.0, 1.0],
];
const GRID_3X3: [[f32; 4]; 9] = [
    [0.0, 0.0, 0.329_999_98, 0.329_999_98],
    [0.329_999_98, 0.0, 0.659_999_97, 0.329_999_98],
    [0.659_999_97, 0.0, 0.989_999_95, 0.329_999_98],
    [0.0, 0.329_999_98, 0.329_999_98, 0.659_999_97],
    [0.329_999_98, 0.329_999_98, 0.659_999_97, 0.659_999_97],
    [0.659_999_97, 0.329_999_98, 0.989_999_95, 0.659_999_97],
    [0.0, 0.659_999_97, 0.329_999_98, 0.989_999_95],
    [0.329_999_98, 0.659_999_97, 0.659_999_97, 0.989_999_95],
    [0.659_999_97, 0.659_999_97, 0.989_999_95, 0.989_999_95],
];
const GRID_4X4: [[f32; 4]; 16] = [
    [0.0, 0.0, 0.25, 0.25],
    [0.25, 0.0, 0.5, 0.25],
    [0.5, 0.0, 0.75, 0.25],
    [0.75, 0.0, 1.0, 0.25],
    [0.0, 0.25, 0.25, 0.5],
    [0.25, 0.25, 0.5, 0.5],
    [0.5, 0.25, 0.75, 0.5],
    [0.75, 0.25, 1.0, 0.5],
    [0.0, 0.5, 0.25, 0.75],
    [0.25, 0.5, 0.5, 0.75],
    [0.5, 0.5, 0.75, 0.75],
    [0.75, 0.5, 1.0, 0.75],
    [0.0, 0.75, 0.25, 1.0],
    [0.25, 0.75, 0.5, 1.0],
    [0.5, 0.75, 0.75, 1.0],
    [0.75, 0.75, 1.0, 1.0],
];
const SUBDIV_CORNER: [[usize; 4]; 4] = [[0, 1, 2, 3], [1, 0, 3, 2], [2, 3, 0, 1], [3, 2, 1, 0]];

/// The GS frame of the battle (`View_InitLayout`): XYOFFSET (2048 - 256, 2048 - 224).
pub(crate) const OFFSET_X: f32 = 2048.0 - 256.0;
pub(crate) const OFFSET_Y: f32 = 2048.0 - 224.0;
/// `view->screenDist`.
const SCREEN_DIST: f32 = 433.0;

// ---- vectors and matrices (the VU0 library) ------------------------------------------------------------------

pub(crate) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale3(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub(crate) fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `Vec3_Normalize`: a zero vector gives zero.
pub(crate) fn norm(v: V3) -> V3 {
    let len = dot(v, v).sqrt();
    if len == 0.0 {
        [0.0; 3]
    } else {
        scale3(v, 1.0 / len)
    }
}

pub(crate) fn xyz(v: V4) -> V3 {
    [v[0], v[1], v[2]]
}

pub(crate) fn point(v: V3) -> V4 {
    [v[0], v[1], v[2], 1.0]
}

/// `Mtx_Mul(dst, a, b)`: b's rows through a (b is applied first).
pub(crate) fn mtx_mul(a: &Mtx, b: &Mtx) -> Mtx {
    std::array::from_fn(|i| apply(a, b[i]))
}

/// `Mtx_ScaleDiag`: only the three diagonal elements are multiplied.
fn scale_diag(m: &Mtx, s: V3) -> Mtx {
    let mut out = *m;
    for i in 0..3 {
        out[i][i] *= s[i];
    }
    out
}

/// `Mtx_RotateXYZ`: the rows of `m` through the rotations about X, then Y, then Z.
fn rotate_xyz(m: &Mtx, r: V3) -> Mtx {
    let (sx, cx) = r[0].sin_cos();
    let (sy, cy) = r[1].sin_cos();
    let (sz, cz) = r[2].sin_cos();
    let rx: Mtx = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, cx, sx, 0.0],
        [0.0, -sx, cx, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let ry: Mtx = [
        [cy, 0.0, -sy, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [sy, 0.0, cy, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let rz: Mtx = [
        [cz, sz, 0.0, 0.0],
        [-sz, cz, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mut out = *m;
    for r in [&rx, &ry, &rz] {
        out = std::array::from_fn(|i| apply(r, out[i]));
    }
    out
}

/// `EftMath_WrapAngle`: into [-pi, pi).
pub(crate) fn wrap_angle(a: f32) -> f32 {
    let mut a = a;
    while a >= PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}

// ---- the draw pass --------------------------------------------------------------------------------------------

/// The blend equation of a primitive: its ordering-table layer and GS context (`Ot_SetDefaultAlpha`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EffectBlend {
    /// Layer 0, context 1: ALPHA 0x44, (Cs - Cd) * As + Cd.
    Alpha,
    /// Layer 0, context 2: ALPHA 0x42, (0 - Cs) * As + Cd.
    Subtract,
    /// Layer 1, context 1: ALPHA 0x48, Cs * As + Cd.
    Add,
    /// Layer 1, context 2: ALPHA 0x49, Cd * As + Cd.
    AddDest,
}

impl EffectBlend {
    pub(crate) fn of(layer: usize, ctx2: bool) -> Self {
        match (layer, ctx2) {
            (0, false) => Self::Alpha,
            (0, true) => Self::Subtract,
            (_, false) => Self::Add,
            (_, true) => Self::AddDest,
        }
    }
}

/// A vertex as the GS gets it: screen position in pixels of the 512 x 448 frame and GS Z, (s, t, q) with s and t
/// already multiplied by q, and the vertex colour (128 = 1.0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectVertex {
    pub position: [f32; 3],
    pub stq: [f32; 3],
    pub rgba: [u8; 4],
}

/// One textured gouraud triangle of the sorted pass (TEX0 MODULATE with its alpha).
#[derive(Clone, Debug, PartialEq)]
pub struct EffectTri {
    pub texture: Option<usize>,
    pub blend: EffectBlend,
    /// CLAMP 5 (clamped) rather than the pass's wrapping.
    pub clamp: bool,
    pub vertices: [EffectVertex; 3],
}

/// `gOt`: the chains of this frame by slot (first slot, 0x1000 depth slots, last slot) and layer.
pub(crate) struct OrderTable {
    chains: Vec<Vec<EffectTri>>,
}

/// The slot index of depth slot `z` (`gOtZ[z]`; `gOtFirst` is 0, `gOtLast` 0x1001).
pub(crate) fn depth_slot(z: i32) -> usize {
    z.clamp(0, 0xFFF) as usize + 1
}

impl OrderTable {
    pub(crate) fn new() -> Self {
        Self {
            chains: vec![Vec::new(); 0x1002 * 2],
        }
    }

    pub(crate) fn add(&mut self, slot: usize, layer: usize, tri: EffectTri) {
        self.chains[slot * 2 + layer].push(tri);
    }

    /// `Ot_Draw`: every chain in table order.
    pub(crate) fn into_list(self) -> Vec<EffectTri> {
        self.chains.into_iter().flatten().collect()
    }
}

/// What a module needs to draw: the view's matrices and the five clip planes.
pub(crate) struct DrawCtx {
    pub world2screen: Mtx,
    /// `view->view`: world to view.
    pub world2view: Mtx,
    /// `view->pos`: the camera position.
    pub eye: V3,
    planes: [V4; 5],
}

impl DrawCtx {
    /// `EftGfx_UpdateClipPlanes`: top, bottom, left, right and near, from the camera.
    pub(crate) fn new(world2screen: Mtx, world2view: Mtx) -> Self {
        let angle = 30.0 * PI / 180.0;
        let aspect = 0.75f32;
        let (s, c) = angle.sin_cos();
        let mut planes = [
            [0.0, c, s, 1.0],
            [0.0, -c, s, 1.0],
            [0.0; 4],
            [0.0; 4],
            [0.0, 0.0, 1.0, -1.0],
        ];
        let angle2 = (s / (c * aspect)).atan();
        let (s, c) = angle2.sin_cos();
        planes[2] = [-c, 0.0, s, 1.0];
        planes[3] = [c, 0.0, s, 1.0];
        let mut m = inverse_rt(&world2view);
        let eye = xyz(m[3]);
        m[3] = [0.0, 0.0, 0.0, m[3][3]];
        for p in &mut planes {
            let r = apply(&m, *p);
            *p = [r[0], r[1], r[2], -dot(xyz(r), eye)];
        }
        planes[4][3] -= 0.3;
        Self {
            world2screen,
            world2view,
            eye,
            planes,
        }
    }

    /// `Vu0Cur_ProjectPoint` with the world-to-screen matrix: GS x, y and the integer z.
    pub(crate) fn project(&self, p: V3) -> (f32, f32, i32) {
        let v = apply(&self.world2screen, point(p));
        let q = 1.0 / v[3];
        (v[0] * q, v[1] * q, (v[2] * q) as i32)
    }
}

/// A vertex of the clipper (`ClipVtx`): position, uv, colour.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClipVtx {
    pub pos: V4,
    pub uv: V4,
    pub col: V4,
}

/// `ClipVtx_Lerp`: a * t + b * (1 - t) on all three quadwords.
fn clip_lerp(a: &ClipVtx, b: &ClipVtx, t: f32) -> ClipVtx {
    let l = |x: V4, y: V4| std::array::from_fn(|i| x[i] * t + y[i] * (1.0 - t));
    ClipVtx {
        pos: l(a.pos, b.pos),
        uv: l(a.uv, b.uv),
        col: l(a.col, b.col),
    }
}

/// `ClipPoly_ClipPlane`: Sutherland-Hodgman against one plane (outside when the distance is below 0).
fn clip_plane(poly: &mut Vec<ClipVtx>, plane: V4) {
    let n = poly.len();
    if n == 0 {
        return;
    }
    let dist: Vec<f32> = poly
        .iter()
        .map(|v| dot(xyz(plane), xyz(v.pos)) + plane[3])
        .collect();
    let out: Vec<bool> = dist.iter().map(|&d| d < 0.0).collect();
    let n_out = out.iter().filter(|&&o| o).count();
    if n_out == 0 {
        return;
    }
    if n_out == n {
        poly.clear();
        return;
    }
    let mut buf = Vec::with_capacity(9);
    for i in 0..n {
        let j = (i + 1) % n;
        let (cur, next) = (&poly[i], &poly[j]);
        // ClipPlane_EdgeParam: dist(cur) / dot(plane, cur - next).
        let edge = || dist[i] / dot(xyz(plane), sub(xyz(cur.pos), xyz(next.pos)));
        if !out[i] {
            buf.push(*cur);
            if out[j] {
                buf.push(clip_lerp(next, cur, edge()));
            }
        } else if !out[j] {
            buf.push(clip_lerp(next, cur, edge()));
        }
    }
    *poly = buf;
}

/// How a primitive picks its depth slot: a given z (`EftGfx_DrawPolyFixedZ`, the sprite animations), or the
/// average of the fan triangle's projected depths >> 8 times a factor (`EftGfx_DrawPolyScaledZ`).
#[derive(Clone, Copy, Debug)]
pub(crate) enum ZSort {
    Fixed(i32),
    Average(f32),
}

/// A primitive's GS state: texture, ordering-table layer (>= 2: the second context), the depth slot rule, vertices
/// in front of everything, a clamped texture (`CLAMP 5` around the sprite animations' triangles; the sorted pass
/// wraps otherwise) and the alpha a triangle needs on some vertex to be drawn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Prim {
    pub texture: Option<usize>,
    pub ot_layer: u8,
    pub z: ZSort,
    pub front: bool,
    pub clamp: bool,
    pub alpha_min: f32,
}

/// Clips a triangle against the five planes, projects it (`ClipPoly_ProjectCur`) and queues its fan; fan
/// triangles whose three alphas are at most `alpha_min` are dropped.
pub(crate) fn queue_tri_clip(ot: &mut OrderTable, ctx: &DrawCtx, tri: [ClipVtx; 3], prim: Prim) {
    let mut poly = tri.to_vec();
    for plane in ctx.planes {
        clip_plane(&mut poly, plane);
    }
    if poly.is_empty() {
        return;
    }
    let verts: Vec<(EffectVertex, f32, i64)> = poly
        .iter()
        .map(|v| {
            let p = apply(&ctx.world2screen, v.pos);
            let q = 1.0 / p[3];
            let z = (p[2] * q) as i64;
            // The colours are converted to bytes as the MIPS does it: through an int, keeping the low byte.
            let rgba = v.col.map(|c| c as i32 as u8);
            (
                EffectVertex {
                    position: [
                        p[0] * q - OFFSET_X,
                        p[1] * q - OFFSET_Y,
                        if prim.front {
                            0xFF_FFFF as f32
                        } else {
                            z.min(0xFF_FFFF) as f32
                        },
                    ],
                    stq: [v.uv[0] * q, v.uv[1] * q, q],
                    rgba,
                },
                v.col[3],
                z,
            )
        })
        .collect();
    let layer = if prim.ot_layer >= 2 {
        usize::from(prim.ot_layer - 2)
    } else {
        usize::from(prim.ot_layer)
    };
    let blend = EffectBlend::of(layer, prim.ot_layer >= 2);
    for i in 2..verts.len() {
        let (a, b, c) = (verts[0], verts[i - 1], verts[i]);
        if a.1 <= prim.alpha_min && b.1 <= prim.alpha_min && c.1 <= prim.alpha_min {
            continue;
        }
        let z = match prim.z {
            ZSort::Fixed(z) => z,
            ZSort::Average(k) => ((((a.2 + b.2 + c.2) / 3) >> 8) as f32 * k) as i32,
        };
        ot.add(
            depth_slot(z),
            layer,
            EffectTri {
                texture: prim.texture,
                blend,
                clamp: prim.clamp,
                vertices: [a.0, b.0, c.0],
            },
        );
    }
}

/// `EftGfx_DrawSprite`: a camera-facing quad of half size (w, h) world units at `pos` (in pixels, (s32)w *
/// screen distance / depth, then taken as 1/16 pixel offsets as the original adds them to the 12.4 position),
/// rotated by `rot`; nothing below 2 or when a corner is off screen. Queued at the centre's depth slot.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_sprite(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    pos: V3,
    color: V4,
    size: [f32; 2],
    uv: [f32; 4],
    rot: f32,
    layer: u8,
    front: bool,
    texture: Option<usize>,
) {
    let v = apply(&ctx.world2screen, point(pos));
    let q = 1.0 / v[3];
    let (sx, sy, sz, sw) = (
        (v[0] * q * 16.0) as i32,
        (v[1] * q * 16.0) as i32,
        (v[2] * q) as i32,
        v[3] as i32,
    );
    if sw == 0 {
        return;
    }
    let scale = (SCREEN_DIST * 4096.0) / sw as f32;
    let w = ((size[0] as i32).wrapping_mul(scale as i32)) >> 12;
    let h = ((size[1] as i32).wrapping_mul(scale as i32)) >> 12;
    if w < 2 || h < 2 {
        return;
    }
    let (s, c) = rot.sin_cos();
    let turn = |x: f32, y: f32| [x * c - y * s, (x * s + y * c) * 1.166_666_7];
    let a = turn(w as f32, h as f32).map(|v| v as i32);
    let b = turn(-(w as f32), h as f32).map(|v| v as i32);
    let corners = [
        (sx - a[0], sy - a[1]),
        (sx + b[0], sy + b[1]),
        (sx - b[0], sy - b[1]),
        (sx + a[0], sy + a[1]),
    ];
    let off = |x: i32, y: i32| sz <= 0 || x > 0xFFEF || x <= 0 || y > 0xFFEF || y <= 0;
    if corners.iter().any(|&(x, y)| off(x, y)) {
        return;
    }
    let z = if front { 0xFF_FFFF } else { sz };
    let rgba = color.map(|c| c as i32 as u8);
    let st = [
        [uv[0], uv[1]],
        [uv[0], uv[3]],
        [uv[2], uv[1]],
        [uv[2], uv[3]],
    ];
    let vert = |k: usize| EffectVertex {
        position: [
            corners[k].0 as f32 / 16.0 - OFFSET_X,
            corners[k].1 as f32 / 16.0 - OFFSET_Y,
            z.min(0xFF_FFFF) as f32,
        ],
        stq: [st[k][0], st[k][1], 1.0],
        rgba,
    };
    let l = if layer >= 2 { layer - 2 } else { layer };
    let blend = EffectBlend::of(usize::from(l), layer >= 2);
    // A triangle strip: (0, 1, 2) and (1, 2, 3).
    for k in 0..2 {
        ot.add(
            depth_slot(sz >> 8),
            usize::from(l),
            EffectTri {
                texture,
                blend,
                clamp: false,
                vertices: [vert(k), vert(k + 1), vert(k + 2)],
            },
        );
    }
}

/// `Vu0Cur_ProjectPoints` of four points as a strip packet: dropped unless every corner is on the GS area; the
/// depth slot is the sum of the four depths >> 10. `front`: Z forced to the nearest; `front_slot`: forced before
/// the slot is taken (`EftLine_DrawSprite`) rather than after it (`EftPtcl_DrawAxisQuads`, `EftQuad_DrawSprite`).
/// `float_q`: Q is 1 / w of the projection (`Vu0Cur_ProjectPointsStq`) rather than 1 / the integer w.
#[allow(clippy::too_many_arguments)]
pub(crate) fn queue_strip(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    quad: &[V3; 4],
    st: [[f32; 2]; 4],
    color: V4,
    layer: u8,
    texture: Option<usize>,
    front: bool,
    front_slot: bool,
    float_q: bool,
) {
    // (x, y in 1/16 pixel, z, w).
    let mut scr = [(0i32, 0i32, 0i32, 0i32); 4];
    let mut qs = [0.0f32; 4];
    for (k, c) in quad.iter().enumerate() {
        // Vu0Cur_ProjectPoints: stops at the first point outside 0 < x, y < 4096 or behind (w <= 0).
        let v = apply(&ctx.world2screen, point(*c));
        let q = 1.0 / v[3];
        let (x, y) = (v[0] * q, v[1] * q);
        if !(0.0 < x && x < 4096.0 && 0.0 < y && y < 4096.0 && v[3] > 0.0) {
            return;
        }
        scr[k] = (
            (x * 16.0) as i32,
            (y * 16.0) as i32,
            (v[2] * q) as i32,
            v[3] as i32,
        );
        qs[k] = if float_q { q } else { 1.0 / scr[k].3 as f32 };
    }
    let sum = |scr: &[(i32, i32, i32, i32); 4]| {
        scr[0]
            .2
            .wrapping_add(scr[1].2)
            .wrapping_add(scr[2].2)
            .wrapping_add(scr[3].2)
            >> 10
    };
    if front {
        let z = sum(&scr);
        for v in &mut scr {
            v.2 = 0xFF_FFFF;
        }
        if !front_slot {
            return queue_strip_at(ot, &scr, qs, st, color, layer, texture, z);
        }
    }
    let z = sum(&scr);
    queue_strip_at(ot, &scr, qs, st, color, layer, texture, z);
}

#[allow(clippy::too_many_arguments)]
fn queue_strip_at(
    ot: &mut OrderTable,
    scr: &[(i32, i32, i32, i32); 4],
    qs: [f32; 4],
    st: [[f32; 2]; 4],
    color: V4,
    layer: u8,
    texture: Option<usize>,
    z: i32,
) {
    let col = color.map(|c| c as i32 as u8);
    let vert = |k: usize| {
        let q = qs[k];
        EffectVertex {
            position: [
                scr[k].0 as f32 / 16.0 - OFFSET_X,
                scr[k].1 as f32 / 16.0 - OFFSET_Y,
                (scr[k].2 & 0xFF_FFFF) as f32,
            ],
            stq: [st[k][0] * q, st[k][1] * q, q],
            rgba: col,
        }
    };
    let l = if layer >= 2 { layer - 2 } else { layer };
    let blend = EffectBlend::of(usize::from(l), layer >= 2);
    // A triangle strip: (0, 1, 2) and (1, 2, 3).
    for k in 0..2 {
        ot.add(
            depth_slot(z),
            usize::from(l),
            EffectTri {
                texture,
                blend,
                clamp: false,
                vertices: [vert(k), vert(k + 1), vert(k + 2)],
            },
        );
    }
}

/// `EftPrim_DrawQuadDepth`: a w x h quad (half sizes) facing the camera, rotated about the view axis, as two
/// clipped triangles in the depth slot of its centre.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quad_depth(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    pos: V3,
    color: V4,
    layer: u8,
    front: bool,
    texture: Option<usize>,
    size: [f32; 2],
    uv: [f32; 4],
    rot: f32,
) {
    let (w, h) = (size[0], size[1]);
    let mut inv = inverse_rt(&ctx.world2view);
    inv[3] = [0.0, 0.0, 0.0, inv[3][3]];
    // Mtx_Mul(m, inv, Rz(rot)): the rotation first.
    let m = mtx_mul(&inv, &rotate_xyz(&IDENTITY, [0.0, 0.0, rot]));
    let corners = [[-w, -h, 0.0], [w, -h, 0.0], [-w, h, 0.0], [w, h, 0.0]]
        .map(|c| add(xyz(apply(&m, point(c))), pos));
    let (_, _, sz) = ctx.project(pos);
    if sz < 0 {
        return;
    }
    let st: [V4; 4] = [
        [uv[0], uv[1], 1.0, 0.0],
        [uv[2], uv[1], 1.0, 0.0],
        [uv[0], uv[3], 1.0, 0.0],
        [uv[2], uv[3], 1.0, 0.0],
    ];
    for i in 0..2 {
        let tri = std::array::from_fn(|k| ClipVtx {
            pos: point(corners[i + k]),
            uv: st[i + k],
            col: color,
        });
        let prim = Prim {
            texture,
            ot_layer: layer,
            z: ZSort::Fixed(sz >> 8),
            front,
            clamp: false,
            alpha_min: 0.01,
        };
        queue_tri_clip(ot, ctx, tri, prim);
    }
}

// ---- textures --------------------------------------------------------------------------------------------------

/// The decoded textures of one emitter set: per resource category (sets of 32 / 16 / 8 / 4) and resource, the
/// battle's effect texture index of each texture; and the sprite packs of its emitters.
#[derive(Clone, Debug, Default)]
pub(crate) struct SetAssets {
    pub tex: [Vec<Vec<usize>>; 4],
    pub spr: Vec<Option<SprPack>>,
    /// The ribbon blocks of kind 17 emitters.
    pub ribbon: Vec<Option<(RibbonPrm, RibbonAnim)>>,
    /// The definitions of kind 5 emitters.
    pub ptcl: Vec<Option<(PtclDef, PtclDef2)>>,
    /// The definitions of kind 16 sprites.
    pub bill: Vec<Option<(BillDef, BillDef2)>>,
    /// The definitions of kind 9 quad emitters.
    pub quad: Vec<Option<(QuadDef, QuadDef2)>>,
}

impl SetAssets {
    /// Decodes the set's textures into `textures` and parses its sprite packs (kind 14 emitters).
    pub(crate) fn load(
        set: &EmitSet,
        textures: &mut Vec<bt3_data::gs::RgbaTexture>,
    ) -> std::io::Result<Self> {
        let mut tex: [Vec<Vec<usize>>; 4] = Default::default();
        for (cat, list) in set.textures.iter().enumerate() {
            for &o in list {
                let res = bt3_data::tex::TextureResource::parse(set.at(o))?;
                let mut ids = Vec::new();
                for e in &res.entries {
                    ids.push(textures.len());
                    textures.push(bt3_data::gs::decode_sprite_texture(e)?);
                }
                tex[cat].push(ids);
            }
        }
        let mut spr = vec![None; set.pairs.len()];
        if let Some(g) = set.groups[14] {
            for j in 0..g.count {
                let (a, _) = set.pairs[g.first_pair + j];
                spr[g.first_pair + j] = SprPack::parse(set.at(a)).ok();
            }
        }
        let mut ribbon = vec![None; set.pairs.len()];
        if let Some(g) = set.groups[17] {
            for j in 0..g.count {
                if let (a, Some(b)) = set.pairs[g.first_pair + j] {
                    ribbon[g.first_pair + j] = RibbonPrm::parse(set.at(a))
                        .ok()
                        .zip(RibbonAnim::parse(set.at(b)).ok());
                }
            }
        }
        let mut ptcl = vec![None; set.pairs.len()];
        if let Some(g) = set.groups[5] {
            for j in 0..g.count {
                if let (a, Some(b)) = set.pairs[g.first_pair + j] {
                    ptcl[g.first_pair + j] = PtclDef::parse(set.at(a))
                        .ok()
                        .zip(PtclDef2::parse(set.at(b)).ok());
                }
            }
        }
        let mut bill = vec![None; set.pairs.len()];
        if let Some(g) = set.groups[16] {
            for j in 0..g.count {
                if let (a, Some(b)) = set.pairs[g.first_pair + j] {
                    bill[g.first_pair + j] = BillDef::parse(set.at(a))
                        .ok()
                        .zip(BillDef2::parse(set.at(b)).ok());
                }
            }
        }
        let mut quad = vec![None; set.pairs.len()];
        if let Some(g) = set.groups[9] {
            for j in 0..g.count {
                if let (a, Some(b)) = set.pairs[g.first_pair + j] {
                    quad[g.first_pair + j] = QuadDef::parse(set.at(a))
                        .ok()
                        .zip(QuadDef2::parse(set.at(b)).ok());
                }
            }
        }
        Ok(Self {
            tex,
            spr,
            ribbon,
            ptcl,
            bill,
            quad,
        })
    }
}

// ---- kind 14: keyframed sprite animations ----------------------------------------------------------------------

// EFT_ANIMPART_*.
const AP_VALID: u32 = 0x1;
const AP_TIMED: u32 = 0x2;
const AP_STARTED: u32 = 0x4;
const AP_ENDING: u32 = 0x8;
const AP_DEAD: u32 = 0x10;

// EFT_SPR_LAYER_*.
const L_LOOP: u32 = 0x1;
const L_SLOT_MODE: u32 = 0x2;
const L_BILLBOARD: u32 = 0x4;
const L_SUBDIV: u32 = 0x8;
const L_GRID: u32 = 0x10;
const L_GRID_RAND: u32 = 0x20;
const L_SPIN: u32 = 0x40;
const L_BLINK: u32 = 0x80;
const L_JITTER_XY: u32 = 0x100;
const L_JITTER_X: u32 = 0x200;
const L_JITTER_Y: u32 = 0x400;

// EFT_SPR_DRAW_*.
const D_BILLBOARD: u32 = 0x1;
const D_ORIENTED: u32 = 0x2;
const D_MODE1: u32 = 0x4;
const D_MODE0: u32 = 0x8;
const D_MODE2: u32 = 0x10;

/// `EftSprLayer`: the play state of one layer.
#[derive(Clone, Copy, Debug, Default)]
struct SprLayerState {
    loop_start: f32,
    frame: f32,
    time: f32,
    last_frame: f32,
    fade_len: f32,
    fade_left: f32,
    speed: f32,
    loops: i32,
    rand1: i32,
    rand2: i32,
    finished: bool,
    alpha: f32,
    rot: V3,
    spin: V3,
}

/// Where a part's resources are: the owner side, its blast list and the emitter's pair / texture resource.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SetRef {
    pub side: usize,
    pub list: usize,
}

/// An `EftAnimPart` task with its `EftSprAnim` object.
#[derive(Clone, Debug)]
struct AnimPart {
    id: u32,
    chr: usize,
    mode: u8,
    set: SetRef,
    pair: usize,
    res: usize,
    flags: u32,
    life: f32,
    delay: f32,
    hold: f32,
    fade: f32,
    fade_time: f32,
    kind: i32,
    updated: bool,
    pos: V3,
    dir: V3,
    scale: V3,
    color: V4,
    layers: Vec<SprLayerState>,
}

/// `EftSprAnim_Start` / `EftSprAnim_Restart` (the restart keeps the time and the spin).
fn start_layers(pack: &SprPack, layers: &mut Vec<SprLayerState>, restart: bool) {
    if !restart {
        layers.clear();
        layers.resize(pack.layers.len(), SprLayerState::default());
    }
    for (st, def) in layers.iter_mut().zip(&pack.layers) {
        st.loop_start = 0.0;
        st.last_frame = f32::from(def.length - 1);
        st.fade_len = def.fade as f32;
        st.fade_left = def.fade as f32;
        st.frame = 0.0;
        st.alpha = 1.0;
        if !restart {
            st.time = 0.0;
        }
        st.speed = f32::from(def.speed);
        st.finished = false;
        st.loops = 0;
        if def.flags & L_LOOP != 0 {
            st.loop_start = key_frame(def, def.loop_start_key) as f32;
            st.last_frame = key_frame(def, def.loop_end_key) as f32;
            st.loops = i32::from(def.loop_count);
        }
        if !restart && def.flags & L_SPIN != 0 {
            st.rot = [0.0; 3];
            st.spin = def.keys.first().map_or([0.0; 3], |k| k.rot);
        }
    }
}

fn key_frame(def: &bt3_data::eft_pack::SprLayer, key: u8) -> i32 {
    def.keys.get(usize::from(key)).map_or(0, |k| k.frame)
}

/// `EftSprAnim_WrapDegrees`.
fn wrap_degrees(v: &mut V3) {
    for c in v {
        if *c > 360.0 {
            *c -= 360.0;
        }
        if *c < 0.0 {
            *c += 360.0;
        }
    }
}

impl AnimPart {
    /// `EftSprAnim_IsPlaying`.
    fn playing(&self) -> bool {
        self.layers.iter().any(|l| !l.finished)
    }

    /// `EftSprAnim_Step`: every layer advances speed * 2 frames (two libc rand() draws each), loops or fades out;
    /// a looping pack restarts when all layers are done.
    fn step(&mut self, pack: &SprPack, libc: &mut LibcRandom) {
        for st in &mut self.layers {
            st.frame += st.speed + st.speed;
            st.time += st.speed + st.speed;
            st.rand1 = libc.rand();
            st.rand2 = libc.rand();
            st.rot = add(st.rot, st.spin);
            wrap_degrees(&mut st.rot);
            if st.frame >= st.last_frame {
                if st.loops > 0 || st.loops == -1 {
                    st.frame = st.loop_start;
                    if st.loops != -1 {
                        st.loops -= 1;
                    }
                } else {
                    st.frame = st.last_frame;
                    if st.fade_left > 0.0 {
                        st.fade_left -= 1.0;
                        st.alpha = st.fade_left / st.fade_len;
                        if st.fade_left <= 0.0 {
                            st.alpha = 0.0;
                        }
                    }
                    if st.fade_left <= 0.0 {
                        st.finished = true;
                    }
                }
            }
        }
        if pack.looped && !self.playing() {
            start_layers(pack, &mut self.layers, true);
        }
    }

    /// `EftAnimPart_Update`.
    fn update(&mut self, pack: &SprPack, stopped: bool, libc: &mut LibcRandom) {
        if !stopped {
            if self.delay <= 0.0 {
                self.step(pack, libc);
                self.flags |= AP_STARTED;
                if self.flags & AP_TIMED != 0 {
                    self.life -= 1.0;
                    if self.life <= 0.0 {
                        self.flags |= AP_ENDING;
                    }
                }
                if !self.playing() {
                    self.flags &= !AP_STARTED;
                    self.flags |= AP_ENDING;
                }
            } else {
                self.delay -= 1.0;
            }
            if self.flags & AP_ENDING != 0 {
                if self.hold > 0.0 {
                    self.hold -= 1.0;
                } else {
                    let mut a = 0.0;
                    if self.fade_time > 0.0 {
                        a = self.fade / self.fade_time;
                        self.fade -= 1.0;
                    }
                    if self.fade <= 0.0 {
                        self.flags |= AP_DEAD;
                        self.flags &= !AP_VALID;
                        a = 0.0;
                    }
                    self.color = [1.0, 1.0, 1.0, a];
                }
            }
        }
        self.updated = true;
    }

    /// `EftSprAnim_BuildMatrices`: the frame oriented along the direction and the one facing the camera, both
    /// scaled.
    fn matrices(&self, ctx: &DrawCtx) -> (Mtx, Mtx) {
        let mut dir_m = IDENTITY;
        let d = self.dir;
        let mut up = [0.0, -1.0, 0.0];
        if d == [0.0, -1.0, 0.0] {
            up = [0.0, 0.0, 1.0];
        }
        let fwd = norm(d);
        if fwd != [0.0; 3] {
            let right = norm(cross(up, fwd));
            let up = norm(cross(right, fwd));
            dir_m[0] = point(right);
            dir_m[1] = point(up);
            dir_m[2] = point(fwd);
            for r in &mut dir_m[..3] {
                r[3] = 0.0;
            }
        }
        let scale_m = scale_diag(&IDENTITY, self.scale);
        let mut oriented = mtx_mul(&dir_m, &scale_m);
        oriented[3] = [0.0, 0.0, 0.0, oriented[3][3]];
        let mut billboard = inverse_rt(&ctx.world2view);
        billboard[3] = [0.0, 0.0, 0.0, billboard[3][3]];
        let billboard = mtx_mul(&billboard, &scale_m);
        (oriented, billboard)
    }

    /// `EftSprAnim_Draw` with the draw flags `EftAnimPart_Draw` set (`EFT_SPR_DRAW_*`).
    fn draw(&self, flags: u32, pack: &SprPack, tex: &[usize], ot: &mut OrderTable, ctx: &DrawCtx) {
        let (oriented, billboard) = self.matrices(ctx);
        for (st, def) in self.layers.iter().zip(&pack.layers) {
            let mut m = &oriented;
            let mut mode = 0;
            if def.flags & L_BILLBOARD != 0 {
                m = &billboard;
            }
            if def.flags & L_SLOT_MODE != 0 {
                // The slot's flag is 1 from creation (EftSprAnim_SetSlotFlag has no caller).
                mode = 1;
            }
            let f = flags;
            if f & D_BILLBOARD != 0 {
                m = &billboard;
            }
            if f & D_ORIENTED != 0 {
                m = &oriented;
            }
            if f & D_MODE1 != 0 {
                mode = 1;
            }
            if f & D_MODE0 != 0 {
                mode = 0;
            }
            if f & D_MODE2 != 0 {
                mode = 2;
            }
            let mut color = self.color;
            color[3] *= st.alpha;
            let texture = tex.get(usize::from(def.tex)).copied();
            draw_layer(
                def, st, texture, m, &oriented, self.pos, color, mode, ot, ctx,
            );
        }
    }
}

/// `EftSprPack_FindKey`.
fn find_key(def: &bt3_data::eft_pack::SprLayer, frame: i32) -> Option<usize> {
    (0..def.keys.len().saturating_sub(1))
        .find(|&i| frame >= def.keys[i].frame && frame <= def.keys[i + 1].frame)
}

/// `EftSprAnim_DrawLayerAt`: the layer's 10 x 10 quad at its interpolated key, in the frame `m`, around the key
/// position rotated by the oriented frame; flip-book cell or blink; queued at the projected centre's depth (the far
/// end in mode 2), every vertex in front in modes 1 and 2.
#[allow(clippy::too_many_arguments)]
fn draw_layer(
    def: &bt3_data::eft_pack::SprLayer,
    st: &SprLayerState,
    texture: Option<usize>,
    m: &Mtx,
    oriented: &Mtx,
    obj_pos: V3,
    color: V4,
    mode: i32,
    ot: &mut OrderTable,
    ctx: &DrawCtx,
) {
    let frame = st.frame as i32;
    let time = st.time as i32;
    let Some(key) = find_key(def, frame) else {
        return;
    };
    let (k, kn) = (&def.keys[key], &def.keys[key + 1]);
    let span = (kn.frame - k.frame) as f32;
    let t = if span != 0.0 {
        (frame - k.frame) as f32 / span
    } else {
        1.0
    };
    // Vec3_Lerp(out, a, b, t) = b + (a - b) * t with a the next key; Vec4_Lerp(out, a, b, t) = a * t + b * (1 - t).
    let lerp3 = |a: V3, b: V3| std::array::from_fn(|i| b[i] + (a[i] - b[i]) * t);
    let pos = lerp3(kn.pos, k.pos);
    let mut rot = lerp3(kn.rot, k.rot);
    let mut scale = lerp3(kn.scale, k.scale);
    let mut col: V4 =
        std::array::from_fn(|i| f32::from(kn.color[i]) * t + f32::from(k.color[i]) * (1.0 - t));
    for i in 0..4 {
        col[i] *= color[i];
    }
    if def.flags & (L_JITTER_XY | L_JITTER_X | L_JITTER_Y) != 0 {
        jitter(&mut scale, def, frame, st.rand1, st.rand2);
    }
    if def.flags & L_SPIN != 0 {
        rot = st.rot;
    }
    let rot = rot.map(|r| wrap_angle(r * PI / 180.0));
    let local = rotate_xyz(&scale_diag(&IDENTITY, scale), rot);
    let world = mtx_mul(m, &local);
    let center = add(xyz(apply(oriented, point(pos))), obj_pos);
    let mut corner = [
        [-5.0, -5.0, 0.0],
        [5.0, -5.0, 0.0],
        [-5.0, 5.0, 0.0],
        [5.0, 5.0, 0.0],
    ];
    let mut uv: [V4; 4] = [
        [0.0, 0.0, 1.0, 1.0],
        [1.0, 0.0, 1.0, 1.0],
        [0.0, 1.0, 1.0, 1.0],
        [1.0, 1.0, 1.0, 1.0],
    ];
    for c in &mut corner {
        *c = add(xyz(apply(&world, point(*c))), center);
    }
    if def.flags & L_GRID != 0 {
        let (on, off, step) = (
            i32::from(def.blink_on),
            i32::from(def.blink_off),
            i32::from(def.grid_step),
        );
        if on + off != 0 && (time >> 1) % (on + off) >= on {
            return;
        }
        let n = match def.grid {
            0 => 4,
            1 => 8,
            2 => 16,
            3 => 9,
            _ => 0,
        };
        let mut cell = 0;
        if def.flags & L_GRID_RAND != 0 {
            cell = st.rand1 % n;
        } else if step != 0 {
            cell = (time >> 1) / step % n;
        }
        set_grid_uv(&mut uv, def.grid, cell as usize);
    } else if def.flags & L_BLINK != 0 {
        let step = i32::from(def.grid_step);
        if step > 0 {
            let on = i32::from(def.blink_on) + st.rand1 % step;
            let off = i32::from(def.blink_off) + st.rand2 % step;
            if on + off != 0 && (frame >> 1) % (on + off) >= on {
                return;
            }
        }
    }
    let mut z = ctx.project(center).2 >> 8;
    if mode == 2 {
        z = 0xFFF;
    }
    // The CLAMP reset packet queued at z carries no primitive.
    let front = mode != 0;
    let quad = |ot: &mut OrderTable, c: [V3; 4], u: [V4; 4]| {
        for j in 0..2 {
            let tri = std::array::from_fn(|i| ClipVtx {
                pos: point(c[j + i]),
                uv: u[j + i],
                col,
            });
            let prim = Prim {
                texture,
                ot_layer: def.ot_layer,
                z: ZSort::Fixed(z),
                front,
                clamp: true,
                alpha_min: 0.0,
            };
            queue_tri_clip(ot, ctx, tri, prim);
        }
    };
    if def.flags & L_SUBDIV != 0 {
        // EftSprAnim_DrawQuadSubdiv: four quarters from corner 0, each with the whole texture mirrored to join.
        let half = |i: usize| scale3(add(corner[0], corner[i]), 0.5);
        let quarter = [corner[0], half(1), half(2), half(3)];
        let off = [
            [0.0; 3],
            sub(quarter[1], corner[0]),
            sub(quarter[2], corner[0]),
            sub(quarter[3], corner[0]),
        ];
        for q in 0..4 {
            let c = std::array::from_fn(|k| add(quarter[k], off[q]));
            let u = std::array::from_fn(|k| {
                let s = uv[SUBDIV_CORNER[q][k]];
                [s[0], s[1], 1.0, 1.0]
            });
            quad(ot, c, u);
        }
    } else {
        quad(ot, corner, uv);
    }
}

/// `EftSprAnim_Jitter`.
fn jitter(scale: &mut V3, def: &bt3_data::eft_pack::SprLayer, frame: i32, rand1: i32, rand2: i32) {
    let on = def.jitter_period > 0 && frame % i32::from(def.jitter_period) == 0;
    let (jx, jy) = (i32::from(def.jitter[0]), i32::from(def.jitter[1]));
    if def.flags & L_JITTER_XY != 0 && jx > 0 && on {
        let d = (rand1 % jx) as f32 * 0.001;
        scale[0] += d;
        scale[1] += d;
    }
    if def.flags & L_JITTER_X != 0 && jx > 0 && on {
        scale[0] += (rand1 % jx) as f32 * 0.001;
    }
    if def.flags & L_JITTER_Y != 0 && jy > 0 && on {
        scale[1] += (rand2 % jy) as f32 * 0.001;
    }
}

/// `EftSprAnim_SetGridUv`.
fn set_grid_uv(uv: &mut [V4; 4], grid: i8, cell: usize) {
    let c = match grid {
        0 => GRID_2X2.get(cell),
        1 => GRID_4X2.get(cell),
        2 => GRID_4X4.get(cell),
        3 => GRID_3X3.get(cell),
        _ => None,
    };
    if let Some(c) = c {
        uv[0][0] = c[0];
        uv[0][1] = c[1];
        uv[1][0] = c[2];
        uv[1][1] = c[1];
        uv[2][0] = c[0];
        uv[2][1] = c[3];
        uv[3][0] = c[2];
        uv[3][1] = c[3];
    }
}

// ---- the modules -----------------------------------------------------------------------------------------------

/// A spawned module object (the task handle an emitter keeps).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handle {
    Anim(u32),
    Ribbon(u32),
    Ptcl(u32),
    Bill(u32),
    Quad(u32),
}

/// What the modules read of the battle while they update or draw.
pub(crate) struct FxWorld<'a> {
    /// Per side: the set assets of its two blast lists.
    pub sets: [[Option<&'a SetAssets>; 2]; 2],
    /// `BtlScene_IsAnyCharStopped`: a fighter in hit-stop.
    pub any_frozen: bool,
}

/// The particle modules of the effect scene (layer 3's managers that the ki blasts use).
#[derive(Clone, Debug)]
pub(crate) struct Modules {
    anim: Vec<AnimPart>,
    ribbons: Vec<Ribbon>,
    /// `gEftRibbonMgr`'s pool: nodes not in use.
    ribbon_nodes_free: usize,
    ptcls: Vec<Emitter>,
    /// `gEftPtcl->used`: particles of the pool in use.
    ptcl_used: usize,
    bills: Vec<Bill>,
    quads: Vec<QuadEmitter>,
    /// `gEftQuadMgr`'s quads.
    quad_pool: Pool,
    next_id: u32,
}

impl Default for Modules {
    fn default() -> Self {
        Self {
            anim: Vec::new(),
            ribbons: Vec::new(),
            ribbon_nodes_free: RIBBON_NODES,
            ptcls: Vec::new(),
            ptcl_used: 0,
            bills: Vec::new(),
            quads: Vec::new(),
            quad_pool: Pool::default(),
            next_id: 0,
        }
    }
}

/// `EftAnimPartMgr_Init`: 32 parts (and `gEftSprAnimPool`'s 32 slots).
const ANIM_PARTS: usize = 32;

impl Modules {
    fn anim_mut(&mut self, h: u32) -> Option<&mut AnimPart> {
        // EftAnimPart_IsValid: a live part with the id it was created with.
        self.anim
            .iter_mut()
            .find(|p| p.id == h && p.flags & AP_VALID != 0)
    }

    /// `EftAnimPart_IsAlive`.
    fn anim_alive(&self, h: u32) -> bool {
        self.anim
            .iter()
            .any(|p| p.id == h && p.flags & AP_VALID != 0)
    }

    /// `EftAnimPart_Create` with `EftAnimPart_Init`; `None` when the list is full.
    #[allow(clippy::too_many_arguments)]
    fn anim_create(
        &mut self,
        chr: usize,
        mode: u8,
        rate: f32,
        pos: V3,
        dir: V3,
        size: f32,
        set: SetRef,
        pair: usize,
        res: usize,
        pack: &SprPack,
    ) -> Option<u32> {
        if self.anim.len() >= ANIM_PARTS {
            return None;
        }
        self.next_id += 1;
        let mut p = AnimPart {
            id: self.next_id,
            chr,
            mode,
            set,
            pair,
            res,
            flags: AP_VALID,
            life: 0.0,
            delay: 0.0,
            hold: 0.0,
            fade: 0.0,
            fade_time: 0.0,
            kind: 0,
            updated: false,
            pos,
            dir,
            scale: [size; 3],
            color: [1.0; 4],
            layers: Vec::new(),
        };
        start_layers(pack, &mut p.layers, false);
        if rate > 0.0 {
            p.life = rate * 30.0;
            p.flags |= AP_TIMED;
        }
        self.anim.push(p);
        Some(self.next_id)
    }

    fn ribbon_mut(&mut self, h: u32) -> Option<&mut Ribbon> {
        self.ribbons.iter_mut().find(|r| r.id == h)
    }

    /// Quads of the pool in use and live quad emitters (kind 9).
    #[cfg(test)]
    pub(crate) fn quads_used(&self) -> (usize, usize) {
        (self.quad_pool.used(), self.quads.len())
    }

    /// Live sprites (kind 16).
    #[cfg(test)]
    pub(crate) fn bills_alive(&self) -> usize {
        self.bills.len()
    }

    /// Particles of the pool in use (`gEftPtcl->used`).
    #[cfg(test)]
    pub(crate) fn particles_used(&self) -> usize {
        self.ptcl_used
    }

    fn quad_mut(&mut self, h: u32) -> Option<&mut QuadEmitter> {
        self.quads.iter_mut().find(|e| e.id == h && e.alive())
    }

    fn bill_mut(&mut self, h: u32) -> Option<&mut Bill> {
        self.bills.iter_mut().find(|b| b.id == h && b.alive())
    }

    fn ptcl_mut(&mut self, h: u32) -> Option<&mut Emitter> {
        self.ptcls.iter_mut().find(|e| e.id == h && e.alive())
    }

    /// Whether a handle names a live object (`EftEmit_UpdateAlive`'s tests).
    fn alive(&self, h: Handle) -> bool {
        match h {
            Handle::Anim(id) => self.anim_alive(id),
            Handle::Ribbon(id) => self.ribbons.iter().any(|r| r.id == id),
            Handle::Ptcl(id) => self.ptcls.iter().any(|e| e.id == id && e.alive()),
            Handle::Bill(id) => self.bills.iter().any(|b| b.id == id && b.alive()),
            Handle::Quad(id) => self.quads.iter().any(|e| e.id == id && e.alive()),
        }
    }

    /// `EftEmit_KillAll`'s calls.
    fn kill(&mut self, h: Handle) {
        match h {
            Handle::Anim(id) => {
                if let Some(p) = self.anim_mut(id) {
                    p.flags |= AP_DEAD;
                }
            }
            Handle::Ribbon(id) => {
                if let Some(r) = self.ribbon_mut(id) {
                    r.flags |= R_DEAD;
                }
            }
            Handle::Ptcl(id) => {
                if let Some(e) = self.ptcl_mut(id) {
                    e.flags |= W_DEAD;
                }
            }
            Handle::Bill(id) => {
                if let Some(b) = self.bill_mut(id) {
                    b.flags |= B_DEAD;
                }
            }
            Handle::Quad(id) => {
                if let Some(e) = self.quad_mut(id) {
                    e.kill();
                }
            }
        }
    }

    /// The modules' turn in `BtlScene_Update`, in layer 3's order.
    pub(crate) fn update(&mut self, w: &FxWorld, libc: &mut LibcRandom, vu0: &mut Vu0Random) {
        let mut i = 0;
        while i < self.anim.len() {
            let p = &mut self.anim[i];
            let pack = w.sets[p.set.side][p.set.list].and_then(|s| s.spr[p.pair].as_ref());
            if let Some(pack) = pack {
                // BtlScene_IsEffectStopped(chr, kind): kind 0 is "any fighter stopped".
                p.update(pack, w.any_frozen, libc);
            } else {
                p.flags |= AP_DEAD;
            }
            if p.flags & AP_DEAD != 0 {
                self.anim.remove(i);
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.ribbons.len() {
            let r = &mut self.ribbons[i];
            match w.sets[r.set.side][r.set.list].and_then(|s| s.ribbon[r.pair].as_ref()) {
                Some((prm, anim)) => r.update(prm, anim, w.any_frozen, &mut self.ribbon_nodes_free),
                None => r.flags |= R_DEAD,
            }
            if r.flags & R_DEAD != 0 {
                r.free_nodes(&mut self.ribbon_nodes_free);
                self.ribbons.remove(i);
            } else {
                i += 1;
            }
        }
        // EftPtcl_Update; a dead emitter's task ends (EftPtcl_Term) after its update.
        let mut i = 0;
        while i < self.ptcls.len() {
            let e = &mut self.ptcls[i];
            match w.sets[e.set.side][e.set.list].and_then(|s| s.ptcl[e.pair].as_ref()) {
                Some((def, def2)) => e.update(def, def2, w.any_frozen, &mut self.ptcl_used, libc),
                None => e.flags |= W_DEAD,
            }
            if e.flags & W_DEAD != 0 {
                e.free(&mut self.ptcl_used);
                self.ptcls.remove(i);
            } else {
                i += 1;
            }
        }
        // EftQuad_Update (entry 22 of the module table); a dead emitter's task ends (EftQuad_Term).
        let mut i = 0;
        while i < self.quads.len() {
            let e = &mut self.quads[i];
            match w.sets[e.set.side][e.set.list].and_then(|s| s.quad[e.pair].as_ref()) {
                Some((def, def2)) => {
                    let mut rng = Rng {
                        vu0: &mut *vu0,
                        libc: &mut *libc,
                    };
                    e.update(def, def2, w.any_frozen, &mut self.quad_pool, &mut rng);
                }
                None => e.flags |= E_DEAD,
            }
            if e.flags & E_DEAD != 0 {
                e.free(&mut self.quad_pool);
                self.quads.remove(i);
            } else {
                i += 1;
            }
        }
        // EftBill_Update (entry 25 of the module table).
        let mut i = 0;
        while i < self.bills.len() {
            let b = &mut self.bills[i];
            match w.sets[b.set.side][b.set.list].and_then(|s| s.bill[b.pair].as_ref()) {
                Some((def, def2)) => b.update(def, def2, w.any_frozen),
                None => b.flags |= B_DEAD,
            }
            if b.flags & B_DEAD != 0 {
                self.bills.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// `BtlScene_Draw`: every module's objects that have been updated, in layer 3's order. `cam_shown(chr)`:
    /// `BtlScene_IsCharInView`.
    pub(crate) fn draw(
        &self,
        w: &FxWorld,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
        cam_shown: impl Fn(usize) -> bool,
    ) {
        for p in self.anim.iter().filter(|p| p.updated) {
            let Some(assets) = w.sets[p.set.side][p.set.list] else {
                continue;
            };
            let Some(pack) = assets.spr[p.pair].as_ref() else {
                continue;
            };
            // EftAnimPart_Draw: the draw flags by mode (BtlScene_IsEffectHidden: no fighter hides).
            let other = !cam_shown(p.chr);
            let flags = match p.mode {
                1 | 3 => {
                    if other {
                        continue;
                    }
                    5
                }
                4 => 10,
                5 => 6,
                6 if !other => 5,
                0 | 2 | 6 => 9,
                7 => 0,
                8 => {
                    if other {
                        continue;
                    }
                    0x11
                }
                9 => 0x12,
                _ => 0,
            };
            if p.flags & AP_STARTED != 0 {
                let tex = assets.tex[0].get(p.res).map_or(&[][..], Vec::as_slice);
                p.draw(flags, pack, tex, ot, ctx);
            }
        }
        // EftRibbon_Draw (BtlScene_IsEffectHidden: no fighter hides).
        for r in self.ribbons.iter().filter(|r| r.updated) {
            let _ = r.chr();
            r.draw(ot, ctx);
        }
        // EftPtcl_Draw: a front emitter only while its fighter is in view.
        for e in &self.ptcls {
            if e.flags & W_FRONT != 0 && !cam_shown(e.chr()) {
                continue;
            }
            if let Some((def, _)) =
                w.sets[e.set.side][e.set.list].and_then(|s| s.ptcl[e.pair].as_ref())
            {
                e.draw(def, ot, ctx);
            }
        }
        // EftQuad_Draw: a view-only emitter only while its fighter is in view.
        for e in &self.quads {
            if e.flags & E_VIEW_ONLY != 0 && !cam_shown(e.chr()) {
                continue;
            }
            let Some(assets) = w.sets[e.set.side][e.set.list] else {
                continue;
            };
            if let (Some((def, _)), Some(table)) = (
                assets.quad[e.pair].as_ref(),
                e.table.and_then(|t| assets.tex[2].get(t)),
            ) {
                e.draw(def, &self.quad_pool, table, ot, ctx);
            }
        }
        // EftBill_Draw: a front sprite only while its fighter is in view.
        for b in &self.bills {
            if b.flags & B_FRONT != 0 && !cam_shown(b.chr()) {
                continue;
            }
            if let Some((def, _)) =
                w.sets[b.set.side][b.set.list].and_then(|s| s.bill[b.pair].as_ref())
            {
                b.draw(def, ot, ctx);
            }
        }
    }
}

// ---- the emitter set of an instance (EftEmit_*) ---------------------------------------------------------------

// EFT_SPAWN_*.
pub(crate) const SPAWN_START: u32 = 0x1;
pub(crate) const SPAWN_STOP: u32 = 0x2;
const SPAWN_FADE: u32 = 0x4;
const SPAWN_KILL: u32 = 0x8;
const SPAWN_MOVE: u32 = 0x10;
const SPAWN_WARP: u32 = 0x20;
const SPAWN_SCALE: u32 = 0x40;
const SPAWN_DIR: u32 = 0x80;
const SPAWN_RESTART: u32 = 0x100;
const SPAWN_DIR2: u32 = 0x200;

// EFT_EMIT_*.
const E_ONESHOT: u8 = 0x1;
const E_STARTED: u8 = 0x2;
const E_STOPPED: u8 = 0x4;
const E_ALTNODE: u8 = 0x20;
const E_KIND6: u8 = 0x40;
const E_RESTART: u8 = 0x80;

/// `EftEmitNodes`: positions the emitters are attached to.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Nodes {
    /// Bit 2i: slot i resolved; bit 2i + 1: slot i is a pair.
    flags: u32,
    fixed: u32,
    n: [[(V3, u32); 2]; 6],
    c: [V3; 6],
}

impl Nodes {
    /// `EftEmit_SetNode` with a fighter node.
    pub(crate) fn set_node(&mut self, slot: usize, node: u32) {
        self.flags |= 1 << (slot * 2);
        self.fixed |= 1 << (slot * 2);
        self.n[slot][0].1 = node;
    }

    /// `EftEmit_RefreshFixedNodes`.
    pub(crate) fn refresh(&mut self, node_pos: impl Fn(u32) -> V3) {
        for i in 0..6 {
            let bit = 1 << (i * 2);
            if self.flags & bit != 0 && self.fixed & bit != 0 {
                self.n[i][0].0 = node_pos(self.n[i][0].1);
            }
        }
    }
}

/// `EftSetState`: the per-instance state of a set's emitters.
#[derive(Clone, Debug)]
pub(crate) struct EmitState {
    flags: [u8; 40],
    scale: [f32; 40],
    time: [f32; 40],
    handles: [[Option<Handle>; 40]; 2],
}

impl EmitState {
    /// `EftEmit_InitState`.
    pub(crate) fn new(set: &EmitSet) -> Self {
        let mut scale = [0.0; 40];
        for (s, p) in scale.iter_mut().zip(&set.parts) {
            *s = p.scale(0);
        }
        Self {
            flags: [0; 40],
            scale,
            time: [0.0; 40],
            handles: [[None; 40]; 2],
        }
    }

    /// `EftEmit_GetFlags`: what emitter `n` of group `kind` gets this frame. `req`: bits 2, 4, 0x10, 0x20, 0x40 for
    /// phases 0, 1, 3, 4, 5; `ending`: the instance is ending; `kill`: it stops at once.
    fn flags_for(
        &mut self,
        set: &EmitSet,
        kind: u8,
        n: usize,
        ending: bool,
        kill: bool,
        req: u32,
    ) -> u32 {
        let part = &set.parts[n];
        let mut flags = 0;
        if !kill {
            let pairs = [
                (2, 0, 1),
                (4, 1, 2),
                (0x10, 3, 4),
                (0x20, 4, 5),
                (0x40, 5, 6),
            ];
            for (bit, phase, end) in pairs {
                if req & bit != 0 {
                    if part.phase() == phase {
                        flags |= SPAWN_START;
                    }
                    if part.end_phase() == end {
                        flags |= SPAWN_STOP;
                    }
                }
            }
            if self.flags[n] & E_KIND6 != 0 {
                flags |= SPAWN_START;
            }
            if self.flags[n] & E_RESTART != 0 && kind == 0x11 {
                flags |= SPAWN_RESTART;
                self.flags[n] &= 0x7F;
            }
        } else {
            flags = SPAWN_FADE;
        }
        if kill {
            flags |= SPAWN_STOP;
        } else if ending {
            if part.phase() == 2 {
                flags |= SPAWN_START;
            } else if part.phase() != 6 {
                flags |= SPAWN_STOP;
            }
        }
        let f = self.flags[n];
        let started = f & E_STARTED != 0;
        if started {
            flags &= !SPAWN_START;
        }
        if f & E_STOPPED != 0 {
            flags &= !SPAWN_STOP;
        }
        let pf = part.flags();
        if pf & 2 != 0 {
            flags &= !SPAWN_MOVE;
            if kind == 0x11 || kind == 0xF {
                flags |= SPAWN_DIR2;
            }
        } else {
            if !started {
                return flags;
            }
            if part.phase() != 2 {
                flags |= SPAWN_MOVE;
            }
        }
        let f2 = self.flags[n];
        if f2 & E_STARTED != 0 && f2 & E_STOPPED == 0 {
            flags |= SPAWN_SCALE;
            let a = if f2 & E_ALTNODE != 0 {
                part.alt_node()
            } else {
                part.node()
            };
            if a == 5 || pf & 0x80 != 0 {
                flags |= SPAWN_DIR;
            }
        }
        flags
    }

    /// `EftEmit_GetFlagsFromMask`: from a mask of phases 0..5 happening now.
    pub(crate) fn flags_from_mask(
        &mut self,
        set: &EmitSet,
        kind: u8,
        n: usize,
        ending: bool,
        kill: bool,
        mask: u32,
    ) -> u32 {
        let mut req = (mask & 1) << 1;
        for (m, r) in [(2, 4), (4, 8), (8, 0x10), (0x10, 0x20), (0x20, 0x40)] {
            if mask & m != 0 {
                req |= r;
            }
        }
        self.flags_for(set, kind, n, ending, kill, req)
    }

    /// `EftEmit_GetResetFlags`: a started, not stopped emitter moves and warps (kind 17 always, others when they
    /// follow the direction).
    pub(crate) fn reset_flags(&self, set: &EmitSet, kind: u8, n: usize) -> u32 {
        let f = self.flags[n];
        if f & E_STARTED == 0 || f & E_STOPPED != 0 {
            return 0;
        }
        let part = &set.parts[n];
        let a = if f & E_ALTNODE != 0 {
            part.alt_node()
        } else {
            part.node()
        };
        if kind != 0x11 && a != 5 { 0 } else { 0x30 }
    }

    /// `EftEmit_UpdateAlive`: forgets the objects that ended; the groups that still have a live object of a
    /// non-one-shot emitter.
    pub(crate) fn update_alive(&mut self, set: &EmitSet, fx: &Modules) -> u32 {
        let mut result = 0;
        for half in 0..2 {
            for g in set.groups.iter().flatten() {
                if set.head.mask & (1 << g.kind) == 0 {
                    continue;
                }
                for n in g.first..g.first + g.count {
                    match self.handles[half][n] {
                        Some(h) if fx.alive(h) => {
                            if set.parts[n].phase() != 2 {
                                result |= 1 << g.kind;
                            }
                        }
                        _ => self.handles[half][n] = None,
                    }
                }
            }
        }
        result
    }

    /// `EftKiBlast_EndTrail`: bit 0x80 on every emitter of group 17.
    pub(crate) fn end_trail(&mut self, set: &EmitSet) {
        if set.head.mask & 0x2_0000 != 0
            && let Some(g) = set.groups[17]
        {
            for n in g.first..g.first + g.count {
                self.flags[n] |= E_RESTART;
            }
        }
    }

    /// `EftEmit_KillAll`.
    pub(crate) fn kill_all(&mut self, fx: &mut Modules) {
        for half in &mut self.handles {
            for h in half.iter_mut() {
                if let Some(x) = h.take() {
                    fx.kill(x);
                }
            }
        }
    }
}

/// What `EftEmit_Spawn` reads of the owner.
pub(crate) struct SpawnCtx<'a> {
    pub chr: usize,
    pub set: &'a EmitSet,
    pub assets: &'a SetAssets,
    pub set_ref: SetRef,
    /// `BtlScene_GetCharScale` of the owner and of its opponent.
    pub char_scale: [f32; 2],
    /// `BtlCharApi_ObjGetParamFlags0` of the opponent.
    pub opp_chara_flags: u16,
    /// The opponent's node 3 (emitter node 6).
    pub opp_node3: V3,
}

/// `EftEmit_Spawn`: creates, moves, scales, turns, stops or kills the object of emitter `idx` of group `kind`, and
/// steps the emitter's scale animation. `pos` / `dir`: the instance's position and direction; `scale`: what the
/// offsets and sizes are multiplied by.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    fx: &mut Modules,
    state: &mut EmitState,
    nodes: &Nodes,
    c: &SpawnCtx,
    pos: V3,
    dir: V3,
    kind: u8,
    idx: usize,
    flags: u32,
    scale: f32,
    libc: &mut LibcRandom,
) {
    let Some(g) = c.set.groups[usize::from(kind)] else {
        return;
    };
    let n = g.first + idx;
    let def = &c.set.parts[n];
    let mut flags = flags;
    if def.flags2() & 1 != 0 {
        let has = c.opp_chara_flags & 0x80 != 0;
        if def.flags2() & 0x10 != 0 {
            if !has {
                return;
            }
        } else if has {
            return;
        }
    }
    let sel = if state.flags[n] & E_ALTNODE != 0 {
        def.alt_node()
    } else {
        def.node()
    };
    let slot = NODE_SLOT.get(usize::from(sel)).copied().unwrap_or(-1);
    if flags & SPAWN_START != 0 && state.flags[n] & E_STARTED == 0 {
        if matches!(def.phase(), 2 | 6) {
            if def.end_phase() != 0 || def.rate() <= 0.0 {
                return;
            }
            state.flags[n] |= E_ONESHOT;
        }
        if def.end_phase() == 0 && def.rate() <= 0.0 {
            return;
        }
    }
    let rate = def.rate();
    if flags & SPAWN_STOP != 0 && def.phase() == 2 {
        flags &= !SPAWN_STOP;
    }
    let size = state.scale[n]
        * match def.scale_mode() {
            1 => c.char_scale[0],
            2 => c.char_scale[1],
            _ => scale,
        };
    let d = match def.dir_mode() {
        0 => dir,
        1 => scale3(dir, -1.0),
        2 | 3 => {
            let up = if dir[1] == -1.0 {
                [0.0, 0.0, 1.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            let d = cross(dir, cross(dir, up));
            if def.dir_mode() == 3 {
                scale3(d, -1.0)
            } else {
                d
            }
        }
        4 => [0.0, -1.0, 0.0],
        5 => [0.0, 1.0, 0.0],
        _ => [0.0; 3],
    };
    let halves = if slot >= 0 && nodes.flags & (1 << (slot * 2 + 1)) != 0 {
        2
    } else {
        1
    };
    for half in 0..halves {
        let mut p = if slot >= 0 {
            let s = slot as usize;
            if def.flags() & 8 != 0 {
                nodes.c[s]
            } else {
                nodes.n[s][half].0
            }
        } else if sel == 6 {
            c.opp_node3
        } else {
            pos
        };
        if flags & SPAWN_START != 0 {
            let spread = def.spread();
            if 0.0 < spread {
                let mut angle = libc.rand() as f32 / 2_147_483_647.0 * 6.283_185_3;
                if PI <= angle {
                    angle -= 6.283_185_3;
                }
                let r = spread * (libc.rand() as f32 / 2_147_483_647.0);
                let (sa, ca) = angle.sin_cos();
                // (0, r, 0) about Z, then about X by the pitch and Y by the yaw of the direction.
                let off = [-r * sa, r * ca, 0.0];
                let pitch = (-dir[1]).asin();
                let yaw = dir[0].atan2(dir[2]);
                let m = rotate_xyz(&IDENTITY, [pitch, 0.0, 0.0]);
                let m = rotate_xyz(&m, [0.0, yaw, 0.0]);
                p = add(p, xyz(apply(&m, [off[0], off[1], off[2], 1.0])));
            }
        }
        let handles = &mut state.handles[half];
        match kind {
            5 => spawn_type5(fx, handles, flags, c, n, idx, size, scale, rate, p, d, libc),
            16 => spawn_type16(fx, handles, flags, c, n, idx, size, scale, rate, p, d, libc),
            9 => spawn_type9(fx, handles, flags, c, n, idx, size, scale, rate, p, d),
            14 => spawn_type14(fx, handles, flags, c, n, idx, size, scale, rate, p, d),
            17 => spawn_type17(
                fx, handles, flags, c, n, idx, size, scale, rate, p, pos, d, libc,
            ),
            _ => {}
        }
    }
    if flags & SPAWN_START != 0 {
        state.flags[n] = (state.flags[n] | E_STARTED) & !E_STOPPED;
    }
    if flags & SPAWN_STOP != 0 {
        state.flags[n] |= E_STOPPED;
    }
    if state.flags[n] & E_STARTED != 0 && flags & SPAWN_SCALE != 0 {
        let total = def.scale_time() * 30.0;
        if 0.0 < def.scale_time() {
            let first = total * def.scale_split();
            let t = state.time[n] + 1.0;
            state.time[n] = t;
            state.scale[n] = if t < first {
                (def.scale(0) + (def.scale(1) - def.scale(0)) * (t / first)).max(0.0)
            } else if t < total {
                (def.scale(1) + (def.scale(2) - def.scale(1)) * ((t - first) / (total - first)))
                    .max(0.0)
            } else {
                def.scale(2)
            };
        } else {
            state.scale[n] += def.scale_step();
            let s2 = def.scale(2);
            if s2 != 0.0 {
                if 0.0 < def.scale_step() {
                    if s2 < state.scale[n] {
                        state.scale[n] = s2;
                    }
                } else if def.scale_step() < 0.0 {
                    if state.scale[n] < s2 {
                        state.scale[n] = s2;
                    }
                    if state.scale[n] < 0.0 {
                        state.scale[n] = 0.0;
                    }
                }
            }
        }
    }
}

/// `EftEmit_SpawnType9`: a quad emitter at the node, offset along the direction.
#[allow(clippy::too_many_arguments)]
fn spawn_type9(
    fx: &mut Modules,
    handles: &mut [Option<Handle>; 40],
    flags: u32,
    c: &SpawnCtx,
    n: usize,
    idx: usize,
    size: f32,
    scale: f32,
    rate: f32,
    pos: V3,
    dir: V3,
) {
    let Some(g) = c.set.groups[9] else {
        return;
    };
    let part = &c.set.parts[n];
    let p = add(scale3(dir, part.offset() * scale), pos);
    let pair = g.first_pair + idx;
    // The texture table: resource `res` of the set's packs of 8.
    let table = g.cat_first + part.res();
    if flags & SPAWN_START != 0
        && handles[n].is_none()
        && fx.quads.len() < crate::eft_quad::EMITTERS
        && let Some((def, def2)) = c.assets.quad.get(pair).and_then(Option::as_ref)
    {
        let at = if part.flags() & 0x40 != 0 { pos } else { p };
        fx.next_id += 1;
        let table = (table < c.assets.tex[2].len()).then_some(table);
        let mut e = QuadEmitter::new(
            fx.next_id, c.chr, c.set_ref, pair, table, at, dir, size, rate, def, def2,
        );
        e.delay = f32::from(part.delay());
        e.stop_delay = f32::from(part.hold());
        e.set_fade(f32::from(part.fade()));
        e.set_tex(table, part.tex(), part.tex());
        if part.flags() & 0x20 != 0 {
            e.flags |= E_VIEW_ONLY;
        }
        if part.flags2() & 8 != 0 {
            e.flags |= E_OWN_ORIGIN;
        }
        fx.quads.push(e);
        handles[n] = Some(Handle::Quad(fx.next_id));
    }
    let Some(Handle::Quad(id)) = handles[n] else {
        return;
    };
    let Some(def) = c
        .assets
        .quad
        .get(pair)
        .and_then(Option::as_ref)
        .map(|d| d.0)
    else {
        return;
    };
    let Modules {
        quads, quad_pool, ..
    } = fx;
    let Some(e) = quads.iter_mut().find(|e| e.id == id && e.alive()) else {
        return;
    };
    if flags & SPAWN_MOVE != 0 {
        if flags & SPAWN_WARP != 0 {
            e.warp(&def, quad_pool, p);
        } else {
            e.set_pos(p);
        }
    }
    if flags & SPAWN_SCALE != 0 {
        e.set_size(size);
    }
    if flags & SPAWN_DIR != 0 {
        e.set_dir(dir);
    }
    if flags & SPAWN_KILL != 0 {
        e.kill();
        handles[n] = None;
    } else if flags & SPAWN_STOP != 0 {
        if flags & SPAWN_FADE != 0 {
            e.stop_delay = 0.0;
        }
        e.stop();
    }
}

/// `EftEmit_SpawnType16`: a sprite at the node, offset along the direction.
#[allow(clippy::too_many_arguments)]
fn spawn_type16(
    fx: &mut Modules,
    handles: &mut [Option<Handle>; 40],
    flags: u32,
    c: &SpawnCtx,
    n: usize,
    idx: usize,
    size: f32,
    scale: f32,
    rate: f32,
    pos: V3,
    dir: V3,
    libc: &mut LibcRandom,
) {
    let Some(g) = c.set.groups[16] else {
        return;
    };
    let part = &c.set.parts[n];
    let p = add(scale3(dir, part.offset() * scale), pos);
    let pair = g.first_pair + idx;
    if flags & SPAWN_START != 0
        && handles[n].is_none()
        && fx.bills.len() < BILLS
        && let Some((def, def2)) = c.assets.bill.get(pair).and_then(Option::as_ref)
    {
        // EftLine_SetTex: image texIdx of the set's resource.
        let texture = c.assets.tex[1]
            .get(g.cat_first + part.res())
            .and_then(|t| t.get(usize::from(part.tex())))
            .copied();
        let at = if part.flags() & 0x40 != 0 { pos } else { p };
        fx.next_id += 1;
        let mut b = Bill::new(
            fx.next_id, c.chr, c.set_ref, pair, texture, at, dir, rate, size, def, def2, libc,
        );
        b.delay = f32::from(part.delay());
        b.end_delay = f32::from(part.hold());
        if part.flags() & 0x20 != 0 {
            b.flags |= B_FRONT;
        }
        fx.bills.push(b);
        handles[n] = Some(Handle::Bill(fx.next_id));
    }
    let Some(Handle::Bill(id)) = handles[n] else {
        return;
    };
    let Some(b) = fx.bill_mut(id) else {
        return;
    };
    if flags & SPAWN_MOVE != 0 {
        b.set_pos(p);
    }
    if flags & SPAWN_SCALE != 0 {
        b.set_size(size);
    }
    if flags & SPAWN_DIR != 0 {
        b.set_dir(dir);
    }
    if flags & SPAWN_KILL != 0 {
        b.flags |= B_DEAD;
        handles[n] = None;
    } else if flags & SPAWN_STOP != 0 {
        if flags & SPAWN_FADE != 0 {
            b.end_delay = 0.0;
        }
        b.stop();
    }
}

/// `EftEmit_SpawnType5`: a particle emitter at the node, offset along the direction.
#[allow(clippy::too_many_arguments)]
fn spawn_type5(
    fx: &mut Modules,
    handles: &mut [Option<Handle>; 40],
    flags: u32,
    c: &SpawnCtx,
    n: usize,
    idx: usize,
    size: f32,
    scale: f32,
    rate: f32,
    pos: V3,
    dir: V3,
    libc: &mut LibcRandom,
) {
    let Some(g) = c.set.groups[5] else {
        return;
    };
    let part = &c.set.parts[n];
    let p = add(scale3(dir, part.offset() * scale), pos);
    let pair = g.first_pair + idx;
    if flags & SPAWN_START != 0
        && handles[n].is_none()
        && fx.ptcls.len() < EMITTERS
        && let Some((def, def2)) = c.assets.ptcl.get(pair).and_then(Option::as_ref)
    {
        // EftPtcl_PickTexture: image texIdx of the set's resource.
        let texture = c.assets.tex[1]
            .get(g.cat_first + part.res())
            .and_then(|t| t.get(usize::from(part.tex())))
            .copied();
        let at = if part.flags() & 0x40 != 0 { pos } else { p };
        fx.next_id += 1;
        let mut e = Emitter::new(
            fx.next_id, c.chr, c.set_ref, pair, texture, at, dir, rate, size, def, def2, libc,
        );
        e.start_delay = f32::from(part.delay());
        e.stop_delay = f32::from(part.hold());
        e.linger = f32::from(part.fade());
        e.linger_init = e.linger;
        if part.flags() & 0x20 != 0 {
            e.flags |= W_FRONT;
        }
        if part.flags2() & 8 != 0 {
            e.flags |= W_DETACH;
        }
        fx.ptcls.push(e);
        handles[n] = Some(Handle::Ptcl(fx.next_id));
    }
    let Some(Handle::Ptcl(id)) = handles[n] else {
        return;
    };
    let Some(e) = fx.ptcl_mut(id) else {
        return;
    };
    if flags & SPAWN_MOVE != 0 {
        if flags & SPAWN_WARP != 0 {
            e.warp(p);
        } else {
            e.set_pos(p);
        }
    }
    if flags & SPAWN_SCALE != 0 {
        e.set_size(size);
    }
    if flags & SPAWN_DIR != 0 {
        e.set_dir(dir);
    }
    if flags & SPAWN_KILL != 0 {
        e.flags |= W_DEAD;
        handles[n] = None;
    } else if flags & SPAWN_STOP != 0 {
        if flags & SPAWN_FADE != 0 {
            e.stop_delay = 0.0;
        }
        e.stop();
    }
}

/// `EftEmit_SpawnType17`: a ribbon from the node (offset along the direction) to the instance position (offset by
/// offset2); a restart request replaces it.
#[allow(clippy::too_many_arguments)]
fn spawn_type17(
    fx: &mut Modules,
    handles: &mut [Option<Handle>; 40],
    flags: u32,
    c: &SpawnCtx,
    n: usize,
    idx: usize,
    size: f32,
    scale: f32,
    rate: f32,
    pos: V3,
    pos2: V3,
    dir: V3,
    libc: &mut LibcRandom,
) {
    let Some(g) = c.set.groups[17] else {
        return;
    };
    let part = &c.set.parts[n];
    let p = add(scale3(dir, part.offset() * scale), pos);
    let p2 = add(scale3(dir, part.offset2() * scale), pos2);
    let mut create = false;
    if flags & SPAWN_START != 0 && handles[n].is_none() {
        create = true;
    } else if flags & SPAWN_RESTART != 0
        && let Some(Handle::Ribbon(id)) = handles[n].take()
    {
        if let Some(r) = fx.ribbon_mut(id) {
            r.flags |= R_DEAD;
        }
        create = true;
    }
    let pair = g.first_pair + idx;
    if create
        && fx.ribbons.len() < RIBBONS
        && let Some((prm, anim)) = c.assets.ribbon.get(pair).and_then(Option::as_ref)
    {
        let res = g.cat_first + part.res();
        let tex = c.assets.tex[1]
            .get(res)
            .and_then(|t| t.get(usize::from(part.tex())..))
            .unwrap_or_default();
        let at = if part.flags() & 0x40 != 0 { pos } else { p };
        fx.next_id += 1;
        let mut r = Ribbon::new(
            fx.next_id, c.chr, c.set_ref, pair, tex, at, rate, size, prm, anim, libc,
        );
        r.delay = f32::from(part.delay());
        r.fade_delay = f32::from(part.hold());
        r.set_fade_time(f32::from(part.fade()));
        if part.flags2() & 2 != 0 && part.count() >= 3 {
            r.max_nodes = i32::from(part.count());
        }
        fx.ribbons.push(r);
        handles[n] = Some(Handle::Ribbon(fx.next_id));
    }
    let Some(Handle::Ribbon(id)) = handles[n] else {
        return;
    };
    let Some(r) = fx.ribbon_mut(id) else {
        return;
    };
    if flags & SPAWN_MOVE != 0 {
        let warp = flags & SPAWN_WARP != 0;
        if !warp {
            r.set_pos(p);
        }
        r.set_end(p2, warp);
    } else if flags & SPAWN_DIR2 != 0 {
        r.set_end(p2, false);
    }
    if flags & SPAWN_SCALE != 0 {
        r.set_size(size);
    }
    if flags & SPAWN_KILL != 0 {
        r.flags |= R_DEAD;
        handles[n] = None;
    } else if flags & SPAWN_STOP != 0 {
        r.flags |= R_STOP;
    }
}

/// `EftEmit_SpawnType14`: a sprite animation at the node, offset along the direction.
#[allow(clippy::too_many_arguments)]
fn spawn_type14(
    fx: &mut Modules,
    handles: &mut [Option<Handle>; 40],
    flags: u32,
    c: &SpawnCtx,
    n: usize,
    idx: usize,
    size: f32,
    scale: f32,
    rate: f32,
    pos: V3,
    dir: V3,
) {
    let Some(g) = c.set.groups[14] else {
        return;
    };
    let part = &c.set.parts[n];
    let p = add(scale3(dir, part.offset() * scale), pos);
    let pair = g.first_pair + idx;
    if flags & SPAWN_START != 0
        && handles[n].is_none()
        && let Some(pack) = c.assets.spr.get(pair).and_then(Option::as_ref)
    {
        let at = if part.flags() & 0x40 != 0 { pos } else { p };
        let res = g.cat_first + part.res();
        if let Some(id) = fx.anim_create(
            c.chr,
            part.mode(),
            rate,
            at,
            dir,
            size,
            c.set_ref,
            pair,
            res,
            pack,
        ) {
            let a = fx.anim_mut(id).unwrap();
            a.delay = f32::from(part.delay());
            a.hold = f32::from(part.hold());
            a.fade = f32::from(part.fade());
            a.fade_time = f32::from(part.fade());
            a.kind = 0;
            handles[n] = Some(Handle::Anim(id));
        }
    }
    let Some(Handle::Anim(id)) = handles[n] else {
        return;
    };
    let Some(a) = fx.anim_mut(id) else {
        return;
    };
    if flags & SPAWN_MOVE != 0 {
        a.pos = p;
    }
    if flags & SPAWN_SCALE != 0 {
        a.scale = [size; 3];
    }
    if flags & SPAWN_DIR != 0 {
        a.dir = dir;
    }
    if flags & SPAWN_KILL != 0 {
        a.flags |= AP_DEAD;
        handles[n] = None;
    } else if flags & SPAWN_STOP != 0 {
        if flags & SPAWN_FADE != 0 {
            a.hold = 0.0;
        }
        a.flags |= AP_ENDING;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_keeps_the_inside_and_cuts_the_crossing_edges() {
        let v = |x: f32| ClipVtx {
            pos: [x, 0.0, 0.0, 1.0],
            ..ClipVtx::default()
        };
        // Plane x >= 0.
        let plane = [1.0, 0.0, 0.0, 0.0];
        let mut poly = vec![v(-1.0), v(1.0), v(2.0)];
        clip_plane(&mut poly, plane);
        // One vertex out: both crossing edges get a vertex.
        assert_eq!(poly.len(), 4);
        // The crossing vertices land on the plane up to rounding.
        assert!(poly.iter().all(|p| p.pos[0] >= -1e-6));
        let mut poly = vec![v(-1.0), v(-2.0), v(-3.0)];
        clip_plane(&mut poly, plane);
        assert!(poly.is_empty());
    }

    #[test]
    fn the_order_table_draws_far_slots_first_and_layer_0_before_layer_1() {
        let tri = |b| EffectTri {
            texture: None,
            blend: b,
            clamp: false,
            vertices: [EffectVertex {
                position: [0.0; 3],
                stq: [0.0; 3],
                rgba: [0; 4],
            }; 3],
        };
        let mut ot = OrderTable::new();
        ot.add(depth_slot(5), 1, tri(EffectBlend::Add));
        ot.add(depth_slot(5), 0, tri(EffectBlend::Alpha));
        ot.add(depth_slot(1), 1, tri(EffectBlend::AddDest));
        let list = ot.into_list();
        let order: Vec<_> = list.iter().map(|t| t.blend).collect();
        assert_eq!(
            order,
            [EffectBlend::AddDest, EffectBlend::Alpha, EffectBlend::Add]
        );
    }
}
