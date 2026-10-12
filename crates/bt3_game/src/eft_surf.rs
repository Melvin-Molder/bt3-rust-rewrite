//! The stage's animated surfaces (`EftSurf_*`, src/battle/eft_stage_2.c and eft_surface_out.c; stage effect kind
//! 1, on a stage whose pack has entry 0xD): meshes of the surface file with a texture each, scrolling (or
//! wobbling, or pulsing) every update, drawn each frame after a visibility test of their box: the clipped ones
//! straight to the GS while the scene draws (before the ordering table), the others into the table at the depth
//! of their first vertex less their bias, in layer 0, 1 (no depth test: additive) or 2.
//!
//! The reflecting meshes (flag 0x800: stages 2, 11, 12 and 14) take the frames of a normal-map animation (stage
//! pack entry 0xA, but its last four textures): an 8-bit texture whose palette holds normals. Every frame the
//! palette of the current one is lit twice (`EftSurf_RenderPalettes`), diffusely by the light turned into the
//! surface's frame and specularly as seen from the camera, giving two textures; a reflecting triangle draws with
//! the first, blended, and again with the second, added, its alpha faded with the distance.

use std::sync::Arc;

use bt3_data::gs::{IndexedTexture, RgbaTexture};
use bt3_data::stage::StageSurfRec;
use bt3_data::stage_surf::{SurfMesh, SurfParam};

use crate::battle::{Mtx, apply, inverse_rt};
use crate::eft::{
    ClipVtx, DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, V3, V4,
    clip_plane, cross, depth_slot, dot, norm, point, scale3, sub, xyz,
};

/// `EFT_SURF_*`.
const NO_DEPTH_TEST: u32 = 0x8;
const LAYER2: u32 = 0x10;
const HIDDEN: u32 = 0x60;
const WAVE: u32 = 0x80;
const PULSE: u32 = 0x100;
const NEAR_ONLY: u32 = 0x200;
const CLIP: u32 = 0x400;
const REFLECT: u32 = 0x800;

const TWO_PI: f32 = 6.283_185_3;

/// `gEftSurf`: the meshes with their scroll, the file's textures among the effect textures.
#[derive(Clone, Debug)]
pub(crate) struct Surf {
    meshes: Vec<SurfMesh>,
    textures: Vec<Option<usize>>,
    /// `BtlStage_GetWaterLevel` (0 without water).
    level: f32,
    /// `EftSurfRt`, when a mesh reflects and the normal maps are there.
    reflect: Option<Reflect>,
}

/// `EftSurfRt`: the reflecting surface's normal maps, parameters, frame and animation, and the two textures
/// its lit palettes give among the effect textures.
#[derive(Clone, Debug)]
pub(crate) struct Reflect {
    frames: Arc<Vec<IndexedTexture>>,
    param: SurfParam,
    frame: Mtx,
    light: V3,
    anim_frame: usize,
    anim_acc: f32,
    out: [usize; 2],
}

impl Reflect {
    /// The renderer's setup in `EftSurf_Init`: the file's parameters with the stage's record over them (the fade
    /// distance 1000 when 0).
    pub(crate) fn new(
        frames: Arc<Vec<IndexedTexture>>,
        file: SurfParam,
        rec: Option<&StageSurfRec>,
        out: [usize; 2],
    ) -> Self {
        let mut param = file;
        if let Some(r) = rec {
            param.color_a = r.color_a.map(f32::from);
            param.color_b = r.color_b.map(f32::from);
            param.light_dir = [r.light_dir[0], r.light_dir[1], r.light_dir[2], 1.0];
            param.specular = r.specular;
            param.anim_speed = i32::from(r.anim_speed);
            param.fade_dist = r.fade_dist;
        }
        if param.fade_dist == 0.0 {
            param.fade_dist = 1000.0;
        }
        Self {
            frames,
            param,
            frame: tangent_frame(),
            light: [0.0; 3],
            anim_frame: 0,
            anim_acc: 0.0,
            out,
        }
    }

    /// `EftSurf_StepAnim`: the light in the surface's frame; the normal map advances by `animSpeed / count` of a
    /// frame.
    fn step(&mut self) {
        let dir = xyz(self.param.light_dir);
        self.light = [0, 1, 2].map(|k| dot(dir, xyz(self.frame[k])));
        let count = self.frames.len();
        self.anim_acc += self.param.anim_speed as f32 / count as f32;
        if self.anim_acc >= 1.0 {
            self.anim_acc = 0.0;
            self.anim_frame += 1;
        }
        if self.anim_frame > count - 1 {
            self.anim_frame = 0;
        }
    }

    /// `EftSurf_RenderPalettes`: the current normal map through its diffuse palette (colour B, alpha from N.L)
    /// and its specular one (the light reflected about the normal, seen along the view), for the two textures.
    pub(crate) fn render(&self, world2view: &Mtx) -> [(usize, RgbaTexture); 2] {
        let src = &self.frames[self.anim_frame];
        let normals: Vec<V3> = src
            .palette
            .iter()
            .map(|p| norm([0, 1, 2].map(|k| f32::from(p[k]) - 128.0)))
            .collect();
        // EftGfx_LightClutDiffuse.
        let b = self.param.color_b.map(|c| c as u32 as u8);
        let diffuse: Vec<[u8; 4]> = normals
            .iter()
            .map(|n| {
                let v = ((dot(self.light, *n) * 128.0) as i32).max(0);
                [b[0], b[1], b[2], (v / 2 + 0x20).min(0x80) as u8]
            })
            .collect();
        // EftGfx_LightClutSpecular: the eye is the light; the camera's axis in the surface's frame.
        let mut inv = inverse_rt(world2view);
        let z = scale3(xyz(inv[2]), -1.0);
        inv[3] = [0.0, 0.0, 0.0, inv[3][3]];
        let c = [0, 1, 2].map(|k| dot(z, xyz(self.frame[k])));
        let eye = self.light;
        let power = self.param.specular * 255.0;
        let specular: Vec<[u8; 4]> = normals
            .iter()
            .map(|n| {
                let d = dot(eye, *n) * 2.0;
                let r = sub(scale3(*n, d), eye);
                let h = xyz(apply(&inv, [r[0], r[1], r[2], 1.0]));
                let v = ((dot(c, h) * power) as i32).clamp(0, 255) as u8;
                [v, v, v, 0x80]
            })
            .collect();
        [
            (self.out[0], src.with_palette(&diffuse)),
            (self.out[1], src.with_palette(&specular)),
        ]
    }
}

/// `EftMath_CalcTangentFrame` of the surface's two fixed triples (positions (0, 0, 0), (10, 0, 0), (0, 0, 10)
/// against texture points (0, 0), (10, 0), (0, 10)).
fn tangent_frame() -> Mtx {
    let pos: [V3; 3] = [[0.0; 3], [10.0, 0.0, 0.0], [0.0, 0.0, 10.0]];
    let uv: [[f32; 2]; 3] = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]];
    let mut row0 = [0.0; 3];
    let mut row1 = [0.0; 3];
    for k in 0..3 {
        let e1 = [
            pos[1][k] - pos[0][k],
            uv[1][0] - uv[0][0],
            uv[1][1] - uv[0][1],
        ];
        let e2 = [
            pos[2][k] - pos[0][k],
            uv[2][0] - uv[0][0],
            uv[2][1] - uv[0][1],
        ];
        let c = cross(e1, e2);
        row0[k] = -c[1] / c[0];
        row1[k] = -c[2] / c[0];
    }
    let n = cross(row0, row1);
    let row1 = cross(n, row0);
    let [r0, r1, r2] = [norm(row0), norm(row1), norm(n)];
    [
        [r0[0], r0[1], r0[2], 1.0],
        [r1[0], r1[1], r1[2], 1.0],
        [r2[0], r2[1], r2[2], 1.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

impl Surf {
    /// `EftSurf_Init`.
    pub(crate) fn new(
        meshes: Vec<SurfMesh>,
        textures: Vec<Option<usize>>,
        level: f32,
        reflect: Option<Reflect>,
    ) -> Self {
        let reflect = reflect.filter(|_| meshes.iter().any(|m| m.flags & REFLECT != 0));
        Self {
            meshes,
            textures,
            level,
            reflect,
        }
    }

    /// `EftSurf_BuildPalettes` (every frame, before the views are drawn): the two lit textures.
    pub(crate) fn palettes(&self, world2view: &Mtx) -> Option<[(usize, RgbaTexture); 2]> {
        self.reflect.as_ref().map(|r| r.render(world2view))
    }

    /// `EftSurf_Update` (`EftSurf_Animate`, not while paused or loading): every mesh's scroll steps; a pulsing
    /// one's v bounces between 0.95 and 1 while u goes round, a wobbling one's u goes round.
    pub(crate) fn update(&mut self, paused: bool) {
        if paused {
            return;
        }
        let mut stepped = false;
        for m in &mut self.meshes {
            if m.flags & PULSE != 0 {
                m.v += m.dv;
                if m.dv == 0.0 {
                    m.dv = -0.001;
                    m.v = 1.0;
                }
                if m.v > 1.0 {
                    m.v = 1.0;
                    m.dv = -m.dv;
                }
                if m.v < 0.95 {
                    m.v = 0.95;
                    m.dv = -m.dv;
                }
                m.u += m.du;
                if m.u < 0.0 {
                    m.u = 0.0;
                }
                if m.u > TWO_PI {
                    m.u = -TWO_PI;
                }
            } else if m.flags & WAVE != 0 {
                m.v += m.dv;
                m.u += m.du;
                if m.v > 1.0 {
                    m.v -= 1.0;
                }
                if m.u > TWO_PI {
                    m.u = -TWO_PI;
                }
            } else if m.flags & REFLECT != 0 {
                if !stepped && let Some(r) = &mut self.reflect {
                    r.step();
                    stepped = true;
                }
            } else {
                m.u += m.du;
                m.v += m.dv;
                if m.u > 1.0 {
                    m.u -= 1.0;
                }
                if m.v > 1.0 {
                    m.v -= 1.0;
                }
            }
        }
    }

    /// A mesh's scroll.
    #[cfg(test)]
    pub(crate) fn scroll(&self, i: usize) -> (f32, f32) {
        (self.meshes[i].u, self.meshes[i].v)
    }

    /// `EftSurf_DrawMeshes`: each shown mesh whose box may be in view (one near the camera only within 400 of
    /// it, fading from 200), its triangles scrolled or wobbled, through the route its flags pick. `direct`
    /// takes what goes straight to the GS (`EftSurf_BeginDraw`: blended, depth tested, not written).
    /// `bright`: `EftStage_GetTintScale` (the reflecting colour's RGB); `tinted`: `StgTint_IsOn(0)`, which drops a
    /// reflecting triangle's table pass when it has no specular one.
    pub(crate) fn draw(
        &self,
        ot: &mut OrderTable,
        direct: &mut Vec<EffectTri>,
        ctx: &DrawCtx,
        bright: f32,
        tinted: bool,
    ) {
        // The camera at the water's height (BtlStage_GetWaterLevel).
        let cam = [ctx.eye[0], self.level, ctx.eye[2]];
        for m in &self.meshes {
            if m.flags & HIDDEN != 0 || !box_visible(ctx, m.box_min, m.box_max) {
                continue;
            }
            let mut fade = 1.0;
            if m.flags & NEAR_ONLY != 0 {
                let d = sub(m.center, cam);
                let d = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                if d > 160_000.0 {
                    continue;
                }
                if d > 40_000.0 {
                    fade -= (d - 40_000.0) / 120_000.0;
                }
            }
            if m.flags & REFLECT != 0 {
                self.draw_reflect(ot, direct, ctx, m, cam, bright, tinted);
                continue;
            }
            let layer: u8 = if m.flags & NO_DEPTH_TEST != 0 {
                1
            } else if m.flags & LAYER2 != 0 {
                2
            } else {
                0
            };
            let texture = self.textures.get(m.tex).copied().flatten();
            for tri in &m.tris {
                let out: [ClipVtx; 3] = std::array::from_fn(|k| {
                    let v = &tri[k];
                    let mut st = v.st;
                    let mut col = v.color;
                    if m.flags & WAVE != 0 {
                        let w = m.u.sin() * 0.05;
                        st[0] += if st[1] < 0.5 { w } else { -w };
                        st[1] += m.v;
                    } else {
                        st[0] += m.u;
                        st[1] += m.v;
                        col[3] *= fade;
                    }
                    ClipVtx {
                        pos: point(v.pos),
                        uv: [st[0], st[1], 1.0, 1.0],
                        col,
                    }
                });
                if m.flags & CLIP != 0 {
                    clipped(ctx, out, |v| {
                        direct.push(tri_of(texture, EffectBlend::Alpha, v))
                    });
                } else {
                    queue_clipped(ot, ctx, out, layer, texture, m.z_bias);
                }
            }
        }
    }
}

impl Surf {
    /// A reflecting mesh in `EftSurf_DrawMeshes`: colour A over every vertex (its alpha times the vertex's), a
    /// fade by the squared distance to the camera (at the water's height) up to the fade distance; a clipped
    /// mesh draws straight to the GS (clipped when the triangle is within 450 of the camera), the others into
    /// the table.
    #[allow(clippy::too_many_arguments)]
    fn draw_reflect(
        &self,
        ot: &mut OrderTable,
        direct: &mut Vec<EffectTri>,
        ctx: &DrawCtx,
        m: &SurfMesh,
        cam: V3,
        bright: f32,
        tinted: bool,
    ) {
        let Some(r) = &self.reflect else {
            return;
        };
        let p = &r.param;
        let dist_sq = p.fade_dist * p.fade_dist;
        let light = [
            p.color_a[0] * bright,
            p.color_a[1] * bright,
            p.color_a[2] * bright,
            p.color_a[3] * (1.0 / 128.0),
        ];
        let tex = [Some(r.out[0]), Some(r.out[1])];
        for tri in &m.tris {
            let mut near = true;
            let mut fog = [0.0f32; 3];
            let out: [ClipVtx; 3] = std::array::from_fn(|k| {
                let v = &tri[k];
                let d = sub(v.pos, cam);
                let d = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).min(dist_sq);
                if d > 202_500.0 {
                    near = false;
                }
                fog[k] = (1.0 - d / dist_sq).clamp(0.0, 1.0);
                let mut col = light;
                col[3] *= v.color[3];
                ClipVtx {
                    pos: point(v.pos),
                    uv: [v.st[0], v.st[1], 1.0, 1.0],
                    col,
                }
            });
            if m.flags & CLIP != 0 {
                let mut send = |v: [Proj; 3]| direct_pair(direct, &v, tex, fog);
                if near {
                    let p = project_clipped(ctx, out);
                    for i in 2..p.len() {
                        send([p[0].clone(), p[i - 1].clone(), p[i].clone()]);
                    }
                } else {
                    let p = project_plain(ctx, &out);
                    if !p.iter().all(off_view) {
                        send(p);
                    }
                }
            } else {
                // EftSurf_DrawPolyOtClipped.
                let p = project_clipped(ctx, out);
                for i in 2..p.len() {
                    let z = (p[0].z >> 8) - m.z_bias;
                    queue_pair(ot, [&p[0], &p[i - 1], &p[i]], tex, fog, z, tinted);
                }
            }
        }
    }
}

/// `ClipPoly_ProjectCur` of the triangle as it is.
fn project_plain(ctx: &DrawCtx, tri: &[ClipVtx; 3]) -> [Proj; 3] {
    std::array::from_fn(|k| {
        let v = &tri[k];
        let p = apply(&ctx.world2screen, v.pos);
        let q = 1.0 / p[3];
        Proj {
            x: (p[0] * q * 16.0) as i32,
            y: (p[1] * q * 16.0) as i32,
            z: (p[2] * q) as i32,
            stq: [v.uv[0] * q, v.uv[1] * q, q],
            col: v.col,
        }
    })
}

/// `EftSurf_IsOffScreen`: behind the camera or outside the view's rectangle (2048 +- 272 across, +- 240 down).
fn off_view(p: &Proj) -> bool {
    !(p.z > 0 && p.x < 0x910 << 4 && p.x > 0x6F0 << 4 && p.y < 0x8F0 << 4 && p.y > 0x710 << 4)
}

/// `EftSurf_DrawTriDirect`: the triangle blended with the diffuse texture, then, unless the three fades are
/// at most 0.01, added with the specular one, its alpha times the fades; nothing when the three alphas are 0
/// or less.
fn direct_pair(direct: &mut Vec<EffectTri>, v: &[Proj; 3], tex: [Option<usize>; 2], fog: [f32; 3]) {
    if v.iter().all(|p| p.col[3] <= 0.0) {
        return;
    }
    let count = if fog.iter().all(|&f| f <= 0.01) { 1 } else { 2 };
    for pass in 0..count {
        let vert = |k: usize| {
            let mut c = v[k].col.map(|c| c as i32);
            if pass == 1 {
                c[3] = (c[3] as f32 * fog[k]) as i32;
            }
            EffectVertex {
                position: vertex(&v[k]).position,
                stq: v[k].stq,
                rgba: c.map(|c| c as u8),
            }
        };
        let blend = if pass == 0 {
            EffectBlend::Alpha
        } else {
            EffectBlend::Add
        };
        direct.push(tri_of(tex[pass], blend, [vert(0), vert(1), vert(2)]));
    }
}

/// `EftSurf_QueueTri` of the reflecting surface: the same two passes into layers 0 and 1 of the depth slot; with
/// only the first and the stage tint's slot 0 on, nothing.
fn queue_pair(
    ot: &mut OrderTable,
    v: [&Proj; 3],
    tex: [Option<usize>; 2],
    fog: [f32; 3],
    z: i32,
    tinted: bool,
) {
    if v.iter().all(|p| p.col[3] <= 0.0) {
        return;
    }
    let count = if fog.iter().all(|&f| f <= 0.01) { 1 } else { 2 };
    for layer in 0..count {
        if count == 1 && tinted {
            continue;
        }
        let vert = |k: usize| {
            let mut c = v[k].col;
            if layer == 1 {
                c[3] *= fog[k];
            }
            EffectVertex {
                position: vertex(v[k]).position,
                stq: v[k].stq,
                rgba: c.map(|c| c as i32 as u8),
            }
        };
        ot.add(
            depth_slot(z),
            layer,
            tri_of(
                tex[layer],
                EffectBlend::of(layer, false),
                [vert(0), vert(1), vert(2)],
            ),
        );
    }
}

/// `EftSurf_IsBoxVisible`: the box's corners projected; in view when one is on the screen, when they lie on
/// two sides of it or more, or some are behind the camera and some in front.
fn box_visible(ctx: &DrawCtx, min: V3, max: V3) -> bool {
    let mut all = 0;
    for i in 0..8 {
        let c = [
            if i & 1 != 0 { max[0] } else { min[0] },
            if i & 2 != 0 { max[1] } else { min[1] },
            if i & 4 != 0 { max[2] } else { min[2] },
        ];
        let v = apply(&ctx.world2screen, point(c));
        let q = 1.0 / v[3];
        let (x, y, z) = (
            (v[0] * q) as i32 - 0x700,
            (v[1] * q) as i32 - 0x720,
            (v[2] * q) as i32,
        );
        let code = if z < 0 {
            0x10
        } else {
            let mut code = 0x20;
            // The view's scissor: 0..511 x 0..447.
            if x < 0 {
                code |= 1;
            }
            if 511 + 1 < x {
                code |= 2;
            }
            if y < 0 {
                code |= 4;
            }
            if 447 + 1 < y {
                code |= 8;
            }
            code
        };
        all |= code;
        if code & !0x20 == 0 {
            return true;
        }
    }
    let sides = (all & 1) + ((all & 2) >> 1) + ((all & 4) >> 2) + ((all & 8) >> 3);
    sides >= 2 || all & 0x30 == 0x30
}

/// A projected clip vertex: GS x, y in 1/16 pixel, z, and its (s, t, q) and colour.
#[derive(Clone)]
struct Proj {
    x: i32,
    y: i32,
    z: i32,
    stq: [f32; 3],
    col: V4,
}

/// The triangle clipped by the view's five planes and projected (`ClipPoly_ProjectCur`); its fan's corners.
fn project_clipped(ctx: &DrawCtx, tri: [ClipVtx; 3]) -> Vec<Proj> {
    let mut poly = tri.to_vec();
    for plane in ctx.planes() {
        clip_plane(&mut poly, plane);
    }
    poly.iter()
        .map(|v| {
            let p = apply(&ctx.world2screen, v.pos);
            let q = 1.0 / p[3];
            Proj {
                x: (p[0] * q * 16.0) as i32,
                y: (p[1] * q * 16.0) as i32,
                z: (p[2] * q) as i32,
                stq: [v.uv[0] * q, v.uv[1] * q, q],
                col: v.col,
            }
        })
        .collect()
}

fn vertex(p: &Proj) -> EffectVertex {
    EffectVertex {
        position: [
            p.x as f32 / 16.0 - OFFSET_X,
            p.y as f32 / 16.0 - OFFSET_Y,
            p.z.min(0xFF_FFFF) as f32,
        ],
        stq: p.stq,
        rgba: p.col.map(|c| c as i32 as u8),
    }
}

fn tri_of(texture: Option<usize>, blend: EffectBlend, v: [EffectVertex; 3]) -> EffectTri {
    EffectTri {
        texture,
        blend,
        clamp: false,
        zwrite: false,
        alpha_ref: 0,
        vertices: v,
    }
}

/// `EftSurf_DrawTriClipped`: the fan of the clipped triangle, every triangle of it.
fn clipped(ctx: &DrawCtx, tri: [ClipVtx; 3], mut out: impl FnMut([EffectVertex; 3])) {
    let p = project_clipped(ctx, tri);
    for i in 2..p.len() {
        out([vertex(&p[0]), vertex(&p[i - 1]), vertex(&p[i])]);
    }
}

/// `EftSurf_DrawTriOtClipped`: the fan queued at the depth slot of its first corner less the bias; a fan
/// triangle whose three alphas are below 0.1 is dropped.
fn queue_clipped(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    tri: [ClipVtx; 3],
    layer: u8,
    texture: Option<usize>,
    z_bias: i32,
) {
    let p = project_clipped(ctx, tri);
    let l = if layer >= 2 { layer - 2 } else { layer };
    let blend = EffectBlend::of(usize::from(l), layer >= 2);
    for i in 2..p.len() {
        let (a, b, c) = (&p[0], &p[i - 1], &p[i]);
        if a.col[3] < 0.1 && b.col[3] < 0.1 && c.col[3] < 0.1 {
            continue;
        }
        let z = (a.z >> 8) - z_bias;
        ot.add(
            depth_slot(z),
            usize::from(l),
            tri_of(texture, blend, [vertex(a), vertex(b), vertex(c)]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;
    use bt3_data::stage_surf::SurfVertex;

    fn mesh(flags: u32) -> SurfMesh {
        let v = |x: f32, z: f32, s: f32, t: f32| SurfVertex {
            pos: [x, 20.0, z],
            st: [s, t],
            color: [128.0, 128.0, 128.0, 100.0],
        };
        SurfMesh {
            tex: 0,
            tris: vec![[
                v(-100.0, 200.0, 0.0, 0.0),
                v(100.0, 200.0, 1.0, 0.0),
                v(0.0, 400.0, 0.5, 1.0),
            ]],
            du: 0.01,
            dv: 0.02,
            u: 0.0,
            v: 0.0,
            z_bias: 0,
            flags,
            box_min: [-100.0, 20.0, 200.0],
            box_max: [100.0, 20.0, 400.0],
            center: [0.0, 20.0, 300.0],
        }
    }

    #[test]
    fn meshes_scroll_and_draw_by_their_flags() {
        let mut s = Surf::new(
            vec![mesh(0x7), mesh(0xF), mesh(0x407), mesh(0x67), mesh(0x100)],
            vec![Some(9)],
            0.0,
            None,
        );
        for _ in 0..60 {
            s.update(false);
        }
        // A plain scroll wraps at 1.
        let (u, v) = s.scroll(0);
        assert!((u - 0.6).abs() < 1e-4 && (v - 0.2).abs() < 1e-4, "{u} {v}");
        // A pulse: v bounces within 0.95..1.
        assert!((0.95..=1.0).contains(&s.scroll(4).1));
        s.update(true);
        assert!((s.scroll(0).0 - 0.6).abs() < 1e-4);
        let view = View::new([0.0, 0.0, 0.0], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let (mut ot, mut direct) = (OrderTable::new(), Vec::new());
        s.draw(&mut ot, &mut direct, &ctx, 1.0, false);
        let tris = ot.into_list();
        // The clipped mesh straight to the GS; the plain ones in layer 0 (alpha) and 1 (additive, no depth
        // test); the hidden one not at all.
        assert!(!direct.is_empty());
        assert!(
            direct
                .iter()
                .all(|t| t.blend == EffectBlend::Alpha && t.texture == Some(9))
        );
        assert!(tris.iter().any(|t| t.blend == EffectBlend::Alpha));
        assert!(tris.iter().any(|t| t.blend == EffectBlend::Add));
        // The scroll moved the texture coordinates: (u, v) + the vertex's.
        let first = direct[0].vertices[0];
        assert!(first.stq[2] > 0.0);
    }

    #[test]
    fn the_reflecting_surface_lights_its_normal_map_and_draws_twice() {
        // A 2 x 1 normal map: an entry along the surface's normal (+z of its frame) and one sideways.
        let frame = IndexedTexture {
            width: 2,
            height: 1,
            indices: vec![0, 1],
            palette: vec![[128, 128, 255, 0x80], [255, 128, 128, 0x80]],
        };
        let param = SurfParam {
            color_a: [128.0, 128.0, 128.0, 128.0],
            color_b: [10.0, 164.0, 164.0, 128.0],
            light_dir: [0.0, -1.0, 0.0, 1.0],
            specular: 1.0,
            anim_speed: 15,
            fade_dist: 0.0,
        };
        let frames = Arc::new(vec![frame.clone(), frame]);
        let r = Reflect::new(frames, param, None, [40, 41]);
        assert_eq!(r.param.fade_dist, 1000.0);
        let mut m = mesh(0xC07);
        m.tris[0].iter_mut().for_each(|v| v.color[3] = 128.0);
        let mut s = Surf::new(vec![m], vec![Some(9)], 0.0, Some(r));
        // The frame turns the light into the surface's: the step sets it; the animation advances by 15 / 2 a
        // frame, one frame each update.
        s.update(false);
        let r = s.reflect.as_ref().unwrap();
        assert_eq!(r.anim_frame, 1);
        let view = View::new([0.0, -50.0, 0.0], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let [(a, diffuse), (b, spec)] = s.palettes(&ctx.world2view).unwrap();
        assert_eq!((a, b), (40, 41));
        // Diffuse: colour B, alpha N.L / 2 + 0x20 (0x60 for the normal facing the light, 0x20 sideways),
        // doubled as the effect textures hold it.
        assert_eq!(diffuse.pixels[..4], [10, 164, 164, 0xC0]);
        assert_eq!(diffuse.pixels[4..8], [10, 164, 164, 0x40]);
        assert_eq!(spec.pixels[3], 0xFF);
        // Clipped (0x400) and near: straight to the GS, blended with the first texture, then added with the
        // second.
        let (mut ot, mut direct) = (OrderTable::new(), Vec::new());
        s.draw(&mut ot, &mut direct, &ctx, 1.0, false);
        assert!(!direct.is_empty());
        assert_eq!(direct[0].texture, Some(40));
        assert_eq!(direct[0].blend, EffectBlend::Alpha);
        assert!(
            direct
                .iter()
                .any(|t| t.texture == Some(41) && t.blend == EffectBlend::Add)
        );
    }

    #[test]
    fn a_box_behind_the_camera_is_not_drawn() {
        let view = View::new([0.0; 3], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        assert!(box_visible(
            &ctx,
            [-10.0, -10.0, 100.0],
            [10.0, 10.0, 120.0]
        ));
        assert!(!box_visible(
            &ctx,
            [-10.0, -10.0, -120.0],
            [10.0, 10.0, -100.0]
        ));
    }
}
