//! The stage's scrolling sheet (`EftStageScroll_*`, src/battle/eft_stage_1.c; stage effect kind 0, on a stage whose
//! pack has entry 7): an effect mesh (the sky's clouds) with the textures of pack entry 6, its texture scrolling
//! along a direction (random when the file's angle is negative), drawn by the stage between its back groups 0 and
//! 1 (`StgModel_Draw`: depth not written, ALPHA 0x44, the textures repeating). On most stages its vertices'
//! alpha is 0.5, which the GS takes as 0: only stages 5, 8, 17 and 38 show it.

use bt3_data::eft_mesh::StageScroll;

use crate::battle::{View, apply};
use crate::btl_tech_volley::eft_wrap;
use crate::eft::{ClipVtx, DrawCtx, OFFSET_X, OFFSET_Y, clip_plane, point};
use crate::rng::LibcRandom;
use crate::stage_draw::{StageBatch, StageScreenVertex};

/// The stage batches' order of the sheet's runs: after every material of group 0.
const ORDER: usize = 1000;

/// `gEftStageScroll`.
#[derive(Clone, Debug)]
pub(crate) struct Scroll {
    sheet: StageScroll,
    /// The first of its textures among the stage textures.
    tex_base: usize,
    size: f32,
    speed: f32,
    dir: [f32; 2],
    off: [f32; 2],
}

impl Scroll {
    /// `EftStageScroll_Init` before its load (`StageFx::run_pending`).
    pub(crate) fn new(sheet: StageScroll, tex_base: usize) -> Self {
        let size = sheet.size as f32;
        Self {
            speed: sheet.speed as f32 * 0.00001 * size,
            size,
            sheet,
            tex_base,
            dir: [0.0; 2],
            off: [0.0; 2],
        }
    }

    /// Whether a load draws a random number.
    #[cfg(test)]
    pub(crate) fn random(&self) -> bool {
        self.sheet.angle < 0
    }

    /// `EftStageScroll_Load` (the init and the reset): the direction from the file's angle, or a random one.
    pub(crate) fn load(&mut self, libc: &mut LibcRandom) {
        let deg = if self.sheet.angle < 0 {
            libc.rand() as f32 / 2_147_483_647.0 * 360.0
        } else {
            self.sheet.angle as f32
        };
        let a = eft_wrap(deg * 3.141_592_7 / 180.0);
        self.dir = [a.cos(), a.sin()];
    }

    /// `EftStageScroll_Update` (not while paused or loading): the offset moves and wraps into -size..size.
    pub(crate) fn update(&mut self, stopped: bool) {
        if stopped {
            return;
        }
        for k in 0..2 {
            self.off[k] += self.dir[k] * self.speed;
            if self.off[k] > self.size {
                self.off[k] -= self.size;
            } else if self.off[k] < -self.size {
                self.off[k] += self.size;
            }
        }
    }

    /// `EftStageScroll_Draw` (`EftMesh_DrawNowClip`): every triangle clipped by the view's planes, its uv times the
    /// size plus the offset; a fan triangle is dropped when the fan's first three points are all off the GS area
    /// (the original tests those three, not the triangle's) or its three alphas are below 0.1. Runs of one
    /// texture make one batch each.
    pub(crate) fn draw(&self, view: &View) -> Vec<StageBatch> {
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mesh = &self.sheet.mesh;
        let mut batches: Vec<StageBatch> = Vec::new();
        for tri in &mesh.tris {
            let mut poly: Vec<ClipVtx> = tri
                .v
                .iter()
                .map(|&i| {
                    let v = &mesh.verts[i];
                    ClipVtx {
                        pos: v.pos,
                        uv: [
                            v.uv[0] * self.size + self.off[0],
                            v.uv[1] * self.size + self.off[1],
                            1.0,
                            0.0,
                        ],
                        col: v.color,
                    }
                })
                .collect();
            for plane in ctx.planes() {
                clip_plane(&mut poly, plane);
            }
            if poly.is_empty() {
                continue;
            }
            let mut scr: Vec<(i32, i32, i32, f32)> = poly
                .iter()
                .map(|v| {
                    let p = apply(&view.world2screen, point([v.pos[0], v.pos[1], v.pos[2]]));
                    let q = 1.0 / p[3];
                    (
                        (p[0] * q * 16.0) as i32,
                        (p[1] * q * 16.0) as i32,
                        (p[2] * q) as i32,
                        q,
                    )
                })
                .collect();
            let off = |s: &(i32, i32, i32, f32)| {
                s.2 <= 0 || s.0 > 0xFFEF || s.0 <= 0 || s.1 > 0xFFEF || s.1 <= 0
            };
            let texture = self.tex_base + tri.tex.max(0) as usize;
            for i in 2..poly.len() {
                for k in [0, i - 1, i] {
                    scr[k].2 = scr[k].2.min(0xFF_FFFF);
                }
                if off(&scr[0]) && off(&scr[1]) && off(&scr[2]) {
                    continue;
                }
                if [0, i - 1, i].iter().all(|&k| poly[k].col[3] < 0.1) {
                    continue;
                }
                let vert = |k: usize| {
                    let (s, v) = (scr[k], &poly[k]);
                    StageScreenVertex {
                        position: [
                            s.0 as f32 / 16.0 - OFFSET_X,
                            s.1 as f32 / 16.0 - OFFSET_Y,
                            s.2 as f32,
                        ],
                        stq: [v.uv[0] * s.3, v.uv[1] * s.3, s.3],
                        rgba: v.col.map(|c| f32::from(c as i32 as u8)),
                    }
                };
                match batches.last_mut() {
                    Some(b) if b.texture == texture => {
                        b.vertices.extend([vert(0), vert(i - 1), vert(i)]);
                    }
                    _ => batches.push(StageBatch {
                        group: 0,
                        order: ORDER + batches.len(),
                        texture,
                        vertices: vec![vert(0), vert(i - 1), vert(i)],
                    }),
                }
            }
        }
        batches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bt3_data::eft_mesh::{EffectMesh, MeshTri, MeshVertex};

    fn sheet(angle: i32, alpha: f32) -> StageScroll {
        // A quad far ahead of a camera at the origin looking along +z.
        let v = |x: f32, y: f32, u: f32, t: f32| MeshVertex {
            pos: [x, y, 1000.0, 1.0],
            color: [128.0, 128.0, 128.0, alpha],
            uv: [u, t, 1.0, 1.0],
        };
        StageScroll {
            size: 4,
            speed: 50,
            angle,
            mesh: EffectMesh {
                tris: vec![
                    MeshTri {
                        v: [0, 1, 2],
                        tex: 0,
                    },
                    MeshTri {
                        v: [2, 1, 3],
                        tex: 1,
                    },
                ],
                verts: vec![
                    v(-100.0, -100.0, 0.0, 0.0),
                    v(100.0, -100.0, 1.0, 0.0),
                    v(-100.0, 100.0, 0.0, 1.0),
                    v(100.0, 100.0, 1.0, 1.0),
                ],
            },
        }
    }

    #[test]
    fn the_offset_scrolls_along_the_direction_and_wraps() {
        let mut s = Scroll::new(sheet(90, 128.0), 0);
        let mut libc = LibcRandom::new(1);
        s.load(&mut libc);
        // 90 degrees: (cos, sin) = (0, 1); speed 50 * 0.00001 * 4 = 0.002 a frame.
        assert!(s.dir[0].abs() < 1e-6 && (s.dir[1] - 1.0).abs() < 1e-6);
        for _ in 0..100 {
            s.update(false);
        }
        assert!((s.off[1] - 0.2).abs() < 1e-4);
        s.update(true);
        assert!((s.off[1] - 0.2).abs() < 1e-4);
        s.off[1] = 3.999;
        s.update(false);
        assert!((s.off[1] - 0.001).abs() < 1e-3);
        // A negative angle draws its direction at each load.
        let mut r = Scroll::new(sheet(-1, 128.0), 0);
        let mut a = LibcRandom::new(5);
        r.load(&mut a);
        r.load(&mut a);
        let mut b = LibcRandom::new(5);
        b.rand();
        b.rand();
        assert_eq!(a.rand(), b.rand());
        assert!(r.random() && !s.random());
    }

    #[test]
    fn the_sheet_draws_into_group_0_after_its_materials_unless_its_alpha_is_below_one() {
        let view = View::new([0.0; 3], [0.0; 3]);
        let s = Scroll::new(sheet(0, 128.0), 7);
        let batches = s.draw(&view);
        // Two runs, textures 7 and 8 of the stage's.
        assert_eq!(batches.len(), 2);
        assert_eq!(
            batches
                .iter()
                .map(|b| (b.group, b.order, b.texture))
                .collect::<Vec<_>>(),
            [(0, 1000, 7), (0, 1001, 8)]
        );
        assert!(batches.iter().all(|b| b.vertices.len() == 3));
        assert_eq!(batches[0].vertices[0].rgba, [128.0; 4]);
        // Alpha 0.5 is drawn (above 0.1) as a 0 byte.
        let s = Scroll::new(sheet(0, 0.5), 0);
        let batches = s.draw(&view);
        assert_eq!(batches[0].vertices[0].rgba[3], 0.0);
        // Below 0.1: nothing.
        let s = Scroll::new(sheet(0, 0.05), 0);
        assert!(s.draw(&view).is_empty());
    }
}
