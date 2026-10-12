//! The animated stage objects (`StgModel_*Anims`, src/battle/stg_model_anim.c, stg_model_draw.c; `Vu1Node_Animate`,
//! `Vu1Node_Draw` and VU1 program 8, docs/systems/vu1/prog2_prog7_prog8.md). An object whose definition names an
//! animation plays it in a loop, 2 frames an update (not while paused), while it is whole, or always with type bit
//! 0x40000000; each start, reset and stage swap puts it at a random frame (VU0 `Rand_FloatRange`). The draw poses
//! the animation's node hierarchy at the object's frame (keys interpolated linearly; a node keeps its last pose
//! where no pair of keys holds the frame), placed at the object's position and rotation, and skins each vertex
//! between its node's matrix and its parent's, right after the stage's main group with that group's alpha test.

use bt3_data::stage::StageObjectDef;
use bt3_data::stage_anim::StageAnim;

use crate::battle::{IDENTITY, Mtx, View, apply, mul};
use crate::rng::Vu0Random;
use crate::stage_draw::{StageBatch, StageScreenVertex};

/// The type bit that keeps an object's animation playing once it is broken.
const ANIM_WHEN_BROKEN: i32 = 0x4000_0000;
/// Where the batches sort among the main group's materials: after them all.
const ORDER: usize = 9000;

/// The stage's animations, their textures among the stage textures, and the nodes' poses.
#[derive(Clone, Debug, Default)]
pub(crate) struct StageAnims {
    anims: Vec<StageAnim>,
    /// Per animation, node and batch: the stage texture of its TEX0.
    textures: Vec<Vec<Vec<Option<usize>>>>,
    /// Per animation and node: position and rotation as last posed (the file's at first).
    poses: Vec<Vec<([f32; 4], [f32; 4])>>,
    /// `StgObjDef.frame` by object.
    pub frames: Vec<f32>,
}

/// `Vu0Cur_RotateZ` / `X` / `Y` of the current matrix (`cur = cur * R`).
fn rot_z(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[0] = [c, s, 0.0, 0.0];
    m[1] = [-s, c, 0.0, 0.0];
    m
}

fn rot_x(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[1] = [0.0, c, s, 0.0];
    m[2] = [0.0, -s, c, 0.0];
    m
}

fn rot_y(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[0] = [c, 0.0, -s, 0.0];
    m[2] = [s, 0.0, c, 0.0];
    m
}

/// `Vu0Cur_RotateYXZ`'s three rotations (each through the current matrix: `cur = R * cur`), in their own sign
/// convention.
fn rot_yxz(r: [f32; 4]) -> Mtx {
    let (sy, cy) = r[1].sin_cos();
    let (sx, cx) = r[0].sin_cos();
    let (sz, cz) = r[2].sin_cos();
    let mut ry = IDENTITY;
    ry[0] = [cy, 0.0, -sy, 0.0];
    ry[2] = [sy, 0.0, cy, 0.0];
    let mut rx = IDENTITY;
    rx[1] = [0.0, cx, -sx, 0.0];
    rx[2] = [0.0, sx, cx, 0.0];
    let mut rz = IDENTITY;
    rz[0] = [cz, -sz, 0.0, 0.0];
    rz[1] = [sz, cz, 0.0, 0.0];
    mul(&rz, &mul(&rx, &ry))
}

impl StageAnims {
    /// `StgModel_BindAnims`: the animations of a stage file, their textures appended to `textures`.
    pub(crate) fn load(
        file: &[u8],
        textures: &mut Vec<bt3_data::gs::RgbaTexture>,
        object_count: usize,
    ) -> std::io::Result<Self> {
        let anims = bt3_data::stage_anim::parse_stage_anims(file)?;
        let mut tex_of = Vec::new();
        for a in &anims {
            let res =
                bt3_data::tex::TextureResource::parse(file.get(a.textures..).unwrap_or_default())?;
            // TexFile_UploadPacked at 0x3480 / 0x3700: a batch's TEX0 names its texture by those offsets.
            let mut by_block = Vec::new();
            for e in &res.entries {
                by_block.push((e.tbp_offset as u32, e.cbp_offset as u32, textures.len()));
                textures.push(bt3_data::gs::decode_sprite_texture(e)?);
            }
            tex_of.push(
                a.nodes
                    .iter()
                    .map(|n| {
                        n.batches
                            .iter()
                            .map(|b| {
                                let key = (
                                    bt3_data::model::tex0_tbp(b.tex0),
                                    bt3_data::model::tex0_cbp(b.tex0),
                                );
                                by_block
                                    .iter()
                                    .find(|&&(t, c, _)| (t, c) == key)
                                    .map(|&(_, _, i)| i)
                            })
                            .collect()
                    })
                    .collect(),
            );
        }
        let poses = anims
            .iter()
            .map(|a| a.nodes.iter().map(|n| (n.pos, n.rot)).collect())
            .collect();
        Ok(Self {
            anims,
            textures: tex_of,
            poses,
            frames: vec![0.0; object_count],
        })
    }

    fn playing(def: &StageObjectDef, broken: bool) -> bool {
        def.anim > 0 && (def.kind & ANIM_WHEN_BROKEN != 0 || !broken)
    }

    fn length(&self, anim: i32) -> f32 {
        self.anims
            .get(anim as usize - 1)
            .map_or(0.0, |a| f32::from(a.length))
    }

    /// `StgModel_ResetAnims`: every playing object at a random frame of its animation.
    pub(crate) fn reset(
        &mut self,
        defs: &[StageObjectDef],
        broken: impl Fn(usize) -> bool,
        vu0: &mut Vu0Random,
    ) {
        if self.anims.is_empty() {
            return;
        }
        for (i, def) in defs.iter().enumerate() {
            if Self::playing(def, broken(i)) {
                let len = self.length(def.anim);
                if let Some(f) = self.frames.get_mut(i) {
                    *f = vu0.float_range(0.0, 1.0) * len;
                }
            }
        }
    }

    /// `StgModel_UpdateAnims`: 2 frames on, back to 0 past the end.
    pub(crate) fn update(&mut self, defs: &[StageObjectDef], broken: impl Fn(usize) -> bool) {
        if self.anims.is_empty() {
            return;
        }
        for (i, def) in defs.iter().enumerate() {
            if Self::playing(def, broken(i)) {
                let len = self.length(def.anim);
                if let Some(f) = self.frames.get_mut(i) {
                    *f += 2.0;
                    if len < *f {
                        *f = 0.0;
                    }
                }
            }
        }
    }

    /// `StgModel_DrawAnims`: per animation, every object playing it, posed and drawn.
    pub(crate) fn draw(
        &mut self,
        defs: &[StageObjectDef],
        broken: impl Fn(usize) -> bool,
        view: &View,
    ) -> Vec<StageBatch> {
        let mut batches: Vec<StageBatch> = Vec::new();
        for a in 0..self.anims.len() {
            for (j, def) in defs.iter().enumerate() {
                if def.anim != a as i32 + 1 || !Self::playing(def, broken(j)) {
                    continue;
                }
                let deg = |d: f32| d * 3.141_592_7 / 180.0;
                let mut place = mul(
                    &rot_z(deg(def.rotation[2])),
                    &mul(&rot_x(deg(def.rotation[0])), &rot_y(deg(def.rotation[1]))),
                );
                place[3] = [def.position[0], def.position[1], def.position[2], 1.0];
                let frame = self.frames.get(j).copied().unwrap_or(0.0);
                let posed = self.animate(a, &place, frame);
                for (n, (parent, world)) in posed.iter().enumerate() {
                    let node = &self.anims[a].nodes[n];
                    for (b, batch) in node.batches.iter().enumerate() {
                        let Some(texture) = self.textures[a][n][b] else {
                            continue;
                        };
                        let mut out = Vec::new();
                        draw_batch(
                            batch,
                            world,
                            parent,
                            node.pivot,
                            node.parent_pivot,
                            view,
                            &mut out,
                        );
                        match batches.iter_mut().find(|s| s.texture == texture) {
                            Some(s) => s.vertices.extend(out),
                            None => batches.push(StageBatch {
                                group: 2,
                                order: ORDER + batches.len(),
                                texture,
                                vertices: out,
                            }),
                        }
                    }
                }
            }
        }
        batches
    }

    /// `Vu1Node_Animate`: each node's pose from its keys at `frame`, then its matrices through the stack (the
    /// node's position, then its rotation, under the matrix on entry; `pop` levels dropped after it). Per node:
    /// (matrix on entry, matrix after the pose).
    fn animate(&mut self, a: usize, place: &Mtx, frame: f32) -> Vec<(Mtx, Mtx)> {
        let anim = &self.anims[a];
        let poses = &mut self.poses[a];
        let mut cur = *place;
        let mut stack: Vec<Mtx> = Vec::new();
        let mut out = Vec::with_capacity(anim.nodes.len());
        for (i, node) in anim.nodes.iter().enumerate() {
            stack.push(cur);
            let pose = &mut poses[i];
            if let Some(keys) = anim.tracks.get(i) {
                for k in 1..keys.len() {
                    let (p, q) = (&keys[k - 1], &keys[k]);
                    let (fp, fq) = (p.frame as f32, q.frame as f32);
                    if fp == frame {
                        *pose = (p.pos, p.rot);
                        break;
                    }
                    if fp < frame && frame < fq {
                        let t = (frame - fp) / (q.frame - p.frame) as f32;
                        for c in 0..3 {
                            pose.0[c] = (q.pos[c] - p.pos[c]) * t + p.pos[c];
                            pose.1[c] = (q.rot[c] - p.rot[c]) * t + p.rot[c];
                        }
                        break;
                    }
                    if fq == frame {
                        *pose = (q.pos, q.rot);
                        break;
                    }
                }
            }
            pose.0[3] = 1.0;
            pose.1[3] = 1.0;
            let parent = cur;
            // Vu0Cur_TranslateLocal, then Vu0Cur_RotateYXZ: both through the current matrix.
            let mut t = IDENTITY;
            t[3] = pose.0;
            cur = mul(&rot_yxz(pose.1), &mul(&t, &cur));
            out.push((parent, cur));
            for _ in 0..node.pop {
                if let Some(m) = stack.pop() {
                    cur = m;
                }
            }
        }
        out
    }
}

/// Program 8's vertex loop: each vertex between the node's matrix (weight w, less its pivot) and its parent's (less
/// the parent's pivot), to the screen; a strip triangle is drawn when none of its three vertices is outside the
/// clip volume.
fn draw_batch(
    batch: &bt3_data::stage_anim::AnimBatch,
    world: &Mtx,
    parent: &Mtx,
    pivot: [f32; 4],
    parent_pivot: [f32; 4],
    view: &View,
    out: &mut Vec<StageScreenVertex>,
) {
    const OFFSET_X: f32 = 2048.0 - 256.0;
    const OFFSET_Y: f32 = 2048.0 - 224.0;
    let mut last: [Option<(StageScreenVertex, bool)>; 2] = [None, None];
    for v in &batch.vertices {
        let [x, y, z, w] = v.position;
        let pa = apply(world, [x - pivot[0], y - pivot[1], z - pivot[2], 1.0]);
        let pb = apply(
            parent,
            [
                x - parent_pivot[0],
                y - parent_pivot[1],
                z - parent_pivot[2],
                1.0,
            ],
        );
        let p: [f32; 4] = std::array::from_fn(|i| pb[i] + (pa[i] - pb[i]) * w);
        let p = [p[0], p[1], p[2], 1.0];
        let s = apply(&view.world2screen, p);
        let c = apply(&view.world2clip, p);
        let q = 1.0 / s[3];
        let cw = c[3].abs();
        let outside = c[0].abs() > cw || c[1].abs() > cw || c[2].abs() > cw;
        let vertex = StageScreenVertex {
            position: [
                (s[0] * q * 16.0).trunc() / 16.0 - OFFSET_X,
                (s[1] * q * 16.0).trunc() / 16.0 - OFFSET_Y,
                (s[2] * q).trunc(),
            ],
            stq: v.stq.map(|t| t * q),
            rgba: v.color.map(f32::trunc),
        };
        if let [Some(a), Some(b)] = last
            && !(outside || a.1 || b.1)
        {
            out.extend([a.0, b.0, vertex]);
        }
        last = [last[1], Some((vertex, outside))];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotations_follow_the_vu0_conventions() {
        // RotateYXZ by y alone turns +z towards +x the way Vu0Cur_RotateY's rows do through cur = R * cur.
        let m = rot_yxz([0.0, std::f32::consts::FRAC_PI_2, 0.0, 1.0]);
        let v = apply(&m, [0.0, 0.0, 1.0, 0.0]);
        assert!((v[0] - 1.0).abs() < 1e-6 && v[2].abs() < 1e-6, "{v:?}");
        // Its x rotation has the opposite sign of Vu0Cur_RotateX's.
        let a = 0.3;
        let r = rot_yxz([a, 0.0, 0.0, 1.0]);
        let x = rot_x(a);
        assert!((r[1][2] + x[1][2]).abs() < 1e-6);
    }
}
