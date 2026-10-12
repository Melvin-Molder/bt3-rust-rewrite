//! The stage's animated surfaces (`EftSurf_*`, src/battle/eft_stage_2.c and eft_surface_out.c; stage effect kind
//! 1, on a stage whose pack has entry 0xD): meshes of the surface file with a texture each, scrolling (or
//! wobbling, or pulsing) every update, drawn each frame after a visibility test of their box: the clipped ones
//! straight to the GS while the scene draws (before the ordering table), the others into the table at the depth
//! of their first vertex less their bias, in layer 0, 1 (no depth test: additive) or 2.
//!
//! Not ported yet: the reflecting meshes (flag 0x800, stages 2, 11, 12 and 14), lit through palettes every frame.

use bt3_data::stage_surf::SurfMesh;

use crate::battle::apply;
use crate::eft::{
    ClipVtx, DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, V3, V4,
    clip_plane, depth_slot, point, sub,
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
}

impl Surf {
    /// `EftSurf_Init`.
    pub(crate) fn new(meshes: Vec<SurfMesh>, textures: Vec<Option<usize>>, level: f32) -> Self {
        Self {
            meshes,
            textures,
            level,
        }
    }

    /// `EftSurf_Update` (`EftSurf_Animate`, not while paused or loading): every mesh's scroll steps; a pulsing
    /// one's v bounces between 0.95 and 1 while u goes round, a wobbling one's u goes round.
    pub(crate) fn update(&mut self, paused: bool) {
        if paused {
            return;
        }
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
                // EftSurf_StepAnim: the reflecting surface's animation (not ported).
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
    pub(crate) fn draw(&self, ot: &mut OrderTable, direct: &mut Vec<EffectTri>, ctx: &DrawCtx) {
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
                    // EftStage_GetTintScale: 1 without a stage tint.
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
        s.draw(&mut ot, &mut direct, &ctx);
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
