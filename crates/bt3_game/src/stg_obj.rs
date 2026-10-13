//! The stage's destructible objects (src/battle/stg_parts.c): their hit points, breaking (with the objects that
//! name the broken one as parent), the shakes a break gives the cameras near it, and their pieces' fall: a piece
//! with keys follows them, a piece with one key is a rigid body thrown away from the hit (`stg_rigid`).
//!
//! A break plays its sound; the falling pieces raise dust (`EftGndDust_SpawnDebris`) and, of an object of type
//! 0x80, splash where they fall through y = 0 (`EftWater_AddSplashAt`).
//!
//! A broken object of type bit 0x20000000 may hold story mode's hidden item (`btl_chr::HiddenItem`). Not ported:
//! the way-point graph's object flags (nothing reads them).

use bt3_data::stage::StageObjectDef;
use bt3_data::stage_model::{StageModel, StageNode};

use crate::rng::{LibcRandom, Vu0Random};
use crate::stg_rigid::StgRigid;

/// `STG_OBJ_BROKEN` / `STG_OBJ_FALLING`.
const BROKEN: u32 = 1;
const FALLING: u32 = 2;
/// `STG_OBJ_SPLASH`: the pieces splash where they fall through y = 0.
const SPLASH: i32 = 0x80;

/// `STG_OBJ_SHAKE_S` / `M` / `L`.
const SHAKE_S: i32 = 0x10;
const SHAKE_M: i32 = 0x20;
const SHAKE_L: i32 = 0x40;

/// `StgObj`.
#[derive(Clone, Debug, Default)]
struct StgObj {
    state: u32,
    hp: i32,
    kind: i32,
    parent: i32,
    /// Advances by 2 a frame while the pieces fall, up to `anim_end`.
    frame: i32,
    anim_end: i32,
    hit_pos: [f32; 3],
    /// Part 0 (the whole model's first part): its centre and radius, `None` without one.
    part: Option<([f32; 3], f32)>,
    /// The debris pieces: their part, node and radius.
    pieces: Vec<Piece>,
}

/// A debris piece (`StgPart` with a node).
#[derive(Clone, Copy, Debug)]
struct Piece {
    /// The piece's place among all of the object's pieces (those without a node too).
    index: usize,
    part: usize,
    node: usize,
    radius: f32,
}

/// A camera shake a break asks for (`BtlCharApi_ShakeCamsNear`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shake {
    pub pos: [f32; 3],
    pub near: f32,
    pub far: f32,
    pub strength: f32,
    pub time: f32,
    /// `BtlCharApi_RumbleNear` with it: the far distance, power and time (the near one is the shake's).
    pub rumble: Option<(f32, f32, f32)>,
}

/// `gBtlStage->objs` with the definitions they reload from.
#[derive(Clone, Debug, Default)]
pub(crate) struct StageObjs {
    objs: Vec<StgObj>,
    defs: Vec<StageObjectDef>,
    /// The shakes asked for since the cameras last took them.
    pub shakes: Vec<Shake>,
    /// The broken set changed since the model last took it.
    dirty: bool,
    /// The pieces' nodes (`StgNode`): their keys, their current matrices and rigid bodies (< 0: none).
    nodes: Vec<StageNode>,
    node_mtx: Vec<[[f32; 4]; 4]>,
    node_body: Vec<i32>,
    rigid: StgRigid,
    /// The animated objects' animations and frames.
    anims: crate::stage_anim::StageAnims,
    /// `StgModel_ResetAnims` passes owed since the stage was bound (they draw VU0 random numbers, which the
    /// battle supplies on its next step).
    pending_resets: u8,
}

/// A falling piece as `StgModel_FadeDebris` reads it: its part, its node and the fade of its object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FallingPiece {
    pub part: usize,
    pub node: usize,
    /// `Stg_FadeRatio(frame, animEnd)`.
    pub ratio: f32,
}

/// `STG_ANGLE`: degrees into -pi..pi (`Stg_WrapRange`).
fn wrap_angle_deg(deg: f32) -> f32 {
    use std::f32::consts::PI;
    let mut v = deg * PI / 180.0;
    if v < -PI {
        while -PI > v {
            v += 2.0 * PI;
        }
    } else {
        while PI < v {
            v -= 2.0 * PI;
        }
    }
    v
}

/// `STG_NODE_BUILD`: identity, then the X, Y and Z rotations (`StgVu_RotateX` .. `Z`, each a row-vector rotation
/// multiplied on the right), then the position added to row 3.
fn node_matrix(pos: [f32; 3], rot: [f32; 3]) -> [[f32; 4]; 4] {
    let (sx, cx) = rot[0].sin_cos();
    let (sy, cy) = rot[1].sin_cos();
    let (sz, cz) = rot[2].sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, -sy], [0.0, 1.0, 0.0], [sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    let mul = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| -> [[f32; 3]; 3] {
        std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
    };
    let m = mul(mul(rx, ry), rz);
    [
        [m[0][0], m[0][1], m[0][2], 0.0],
        [m[1][0], m[1][1], m[1][2], 0.0],
        [m[2][0], m[2][1], m[2][2], 0.0],
        [pos[0], pos[1], pos[2], 1.0],
    ]
}

/// `StgPart_Animate`: the node's matrix at `frame` of its keys (the last key from the end frame on; between two
/// keys the position linear and the angles through +3600 so that they turn the short way); unchanged when the
/// frame falls between no pair of keys.
fn animate(node: &StageNode, frame: f32) -> Option<[[f32; 4]; 4]> {
    let keys = &node.keys;
    let at = |k: &bt3_data::stage_model::StageKey| node_matrix(k.pos, k.rot.map(wrap_angle_deg));
    if node.end_frame as f32 <= frame {
        return keys.last().map(at);
    }
    for i in 1..keys.len() {
        let (a, b) = (&keys[i - 1], &keys[i]);
        if a.frame as f32 == frame {
            return Some(at(a));
        }
        if (a.frame as f32) < frame && frame < b.frame as f32 {
            let t = (frame - a.frame as f32) / (b.frame as f32 - a.frame as f32);
            let rot = std::array::from_fn(|k| {
                wrap_angle_deg(((b.rot[k] + 3600.0) - (a.rot[k] + 3600.0)) * t + a.rot[k])
            });
            let pos = std::array::from_fn(|k| (b.pos[k] - a.pos[k]) * t + a.pos[k]);
            return Some(node_matrix(pos, rot));
        }
        if frame == b.frame as f32 {
            return Some(at(b));
        }
    }
    None
}

impl StageObjs {
    /// The model's objects (`BtlStage.objCount`) with their definitions, whole (`BtlStage_Reset`).
    pub(crate) fn new(defs: &[StageObjectDef], model: &StageModel) -> Self {
        let objs = model
            .objects
            .iter()
            .map(|o| StgObj {
                anim_end: o.anim_end,
                part: o.parts[0].map(|p| (model.parts[p].center, model.parts[p].radius)),
                pieces: o
                    .pieces
                    .iter()
                    .enumerate()
                    .filter_map(|(index, &p)| {
                        let part = &model.parts[p];
                        part.node.map(|node| Piece {
                            index,
                            part: p,
                            node,
                            radius: part.radius,
                        })
                    })
                    .collect(),
                ..StgObj::default()
            })
            .collect();
        let mut s = Self {
            objs,
            defs: defs.to_vec(),
            shakes: Vec::new(),
            dirty: false,
            nodes: model.nodes.clone(),
            node_mtx: model.nodes.iter().map(|n| n.mtx).collect(),
            node_body: vec![0; model.nodes.len()],
            rigid: StgRigid::default(),
            anims: crate::stage_anim::StageAnims::default(),
            pending_resets: 0,
        };
        s.reset(false);
        s
    }

    /// `StgModel_BindAnims` and `BtlStage_Init`'s `BtlStage_Reset`: the stage's animations, each object's frame
    /// to be drawn twice.
    pub(crate) fn with_anims(mut self, anims: crate::stage_anim::StageAnims) -> Self {
        self.anims = anims;
        self.pending_resets = 2;
        self
    }

    /// One more `BtlStage_Reset`'s animation pass owed (`Battle_Restart` at the first start).
    pub(crate) fn owe_reset(&mut self) {
        self.pending_resets += 1;
    }

    /// The owed `StgModel_ResetAnims` passes.
    pub(crate) fn run_pending_resets(&mut self, vu0: &mut Vu0Random) {
        for _ in 0..std::mem::take(&mut self.pending_resets) {
            self.reset_anims(vu0);
        }
    }

    fn reset_anims(&mut self, vu0: &mut Vu0Random) {
        let objs = &self.objs;
        let broken = |i: usize| objs.get(i).is_some_and(|o| o.state & BROKEN != 0);
        self.anims.reset(&self.defs, broken, vu0);
    }

    /// `BtlStage_Reset`: every object whole (`BtlStage_ResetObjs(0)`), the animations at random frames
    /// (`StgModel_ResetAnims`) and the rigid bodies forgotten (`StgRigid_Reset`).
    pub(crate) fn reset_stage(&mut self, vu0: &mut Vu0Random) {
        self.reset(false);
        self.reset_anims(vu0);
        self.rigid.reset();
    }

    /// `StgModel_DrawAnims`.
    pub(crate) fn draw_anims(
        &mut self,
        view: &crate::battle::View,
    ) -> Vec<crate::stage_draw::StageBatch> {
        let objs = &self.objs;
        let broken = |i: usize| objs.get(i).is_some_and(|o| o.state & BROKEN != 0);
        self.anims.draw(&self.defs, broken, view)
    }

    #[cfg(test)]
    pub(crate) fn anim_frames(&self) -> &[f32] {
        &self.anims.frames
    }

    /// `BtlStage_ResetObjs`: every object whole (hit points, type and parent reloaded, `StgObj_LoadDef`), or with
    /// `broken` all broken without a fall.
    pub(crate) fn reset(&mut self, broken: bool) {
        for (i, o) in self.objs.iter_mut().enumerate() {
            o.state = if broken { BROKEN } else { 0 };
            o.frame = 0;
            if broken {
                o.hp = 0;
            } else if let Some(d) = self.defs.get(i) {
                o.hp = d.hp;
                o.kind = d.kind;
                o.parent = d.parent;
            }
        }
        self.dirty = true;
    }

    /// The broken objects as the collision queries take them.
    pub(crate) fn broken_mask(&self) -> u64 {
        self.objs
            .iter()
            .take(64)
            .enumerate()
            .filter(|(_, o)| o.state & BROKEN != 0)
            .fold(0, |m, (i, _)| m | 1 << i)
    }

    /// `BtlStage_IsObjBroken`.
    pub(crate) fn is_broken(&self, i: usize) -> bool {
        self.objs.get(i).is_some_and(|o| o.state & BROKEN != 0)
    }

    /// `BtlStage_GetObjType`: 0 once broken.
    pub(crate) fn kind(&self, i: usize) -> i32 {
        match self.objs.get(i) {
            Some(o) if o.state & BROKEN == 0 => o.kind,
            _ => 0,
        }
    }

    /// `BtlStage_DamageObj`: true when the object has no hit points left (the caller breaks it); never once broken.
    pub(crate) fn damage(&mut self, i: usize, amount: i32) -> bool {
        match self.objs.get_mut(i) {
            Some(o) if o.state & BROKEN == 0 => {
                o.hp -= amount;
                o.hp <= 0
            }
            _ => false,
        }
    }

    /// `BtlStage_DestroyObj`: breaks object `i` and the objects under it, with the break sound (stage sound 8 at
    /// the part 0 of the last object broken, heard from its radius + 200 to + 1500). Whether it broke (the hidden
    /// item of mode 1 is the caller's, `fighter_break_obj`).
    pub(crate) fn destroy(
        &mut self,
        i: usize,
        hit_pos: [f32; 3],
        snd: &mut crate::btl_snd::CharSound,
    ) -> bool {
        let mut info = None;
        let broke = self.break_obj(i, hit_pos, &mut info);
        if let Some((pos, r)) = info {
            snd.request(
                pos,
                -1,
                crate::btl_snd::KIND_BANK8,
                r + 200.0,
                r + 1500.0,
                8,
            );
        }
        broke
    }

    /// `StgObj_GetItemPos`'s test: an object of type bit 0x20000000 may hold mode 1's hidden item.
    pub(crate) fn holds_item(&self, i: usize) -> bool {
        self.objs.get(i).is_some_and(|o| o.kind & 0x2000_0000 != 0)
    }

    /// `BtlStage_BreakObj`: the broken model instead of the whole one, the pieces start falling, the cameras near
    /// part 0 shake by the type's size (large, medium, small: the last one listed wins); then every object that
    /// names it as parent. False when it was already broken.
    fn break_obj(
        &mut self,
        i: usize,
        hit_pos: [f32; 3],
        info: &mut Option<([f32; 3], f32)>,
    ) -> bool {
        let Some(o) = self.objs.get_mut(i) else {
            return false;
        };
        if o.state & BROKEN != 0 {
            return false;
        }
        o.hp = 0;
        o.state = BROKEN | FALLING;
        o.frame = 0;
        o.hit_pos = hit_pos;
        self.dirty = true;
        if o.part.is_some() {
            *info = o.part;
        }
        if let Some((pos, r)) = o.part
            && o.kind & (SHAKE_S | SHAKE_M | SHAKE_L) != 0
        {
            let mut shake = None;
            if o.kind & SHAKE_L != 0 {
                shake = Some((3.0, 0.7, 300.0, 1.0, 0.3));
            }
            if o.kind & SHAKE_M != 0 {
                shake = Some((2.0, 0.5, 100.0, 0.8, 0.2));
            }
            if o.kind & SHAKE_S != 0 {
                shake = Some((1.0, 0.3, 100.0, 0.5, 0.1));
            }
            if let Some((strength, time, near, power, rumble_time)) = shake {
                self.shakes.push(Shake {
                    pos,
                    near: near + r,
                    far: 1500.0 + r,
                    strength,
                    time,
                    rumble: Some((1000.0 + r, power, rumble_time)),
                });
            }
        }
        for j in 0..self.objs.len() {
            if self.objs[j].parent == i as i32 {
                self.break_obj(j, hit_pos, info);
            }
        }
        true
    }

    /// `BtlStage_UpdateObjs` then `StgRigid_Update` (not while paused): the falling objects' frame advances by 2
    /// until their animation's end (0x94 when 0), which ends the fall and releases their bodies; on the first frame
    /// a rigid piece gets a body at its first key, thrown by the hit's direction and the object's material; a keyed
    /// piece follows its keys, a rigid one its body.
    pub(crate) fn update(
        &mut self,
        inner_radius: f32,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
        dust: &mut crate::eft_gdust::GndDust,
        mut water: Option<&mut crate::eft_water::Water>,
    ) {
        for i in 0..self.objs.len() {
            let o = &mut self.objs[i];
            if o.state & FALLING == 0 {
                continue;
            }
            let first = o.frame == 0;
            o.frame += 2;
            if o.anim_end == 0 {
                o.anim_end = 0x94;
            }
            if o.anim_end <= o.frame {
                o.frame = 0;
                o.state ^= FALLING;
                for p in &o.pieces {
                    if self.nodes[p.node].is_rigid() && self.node_body[p.node] >= 0 {
                        self.rigid.release(self.node_body[p.node]);
                    }
                }
                continue;
            }
            let (frame, hit_pos, material) = (o.frame, o.hit_pos, material(o.kind));
            let dust_bright = dust_scale(o.kind);
            if first {
                // The pieces at frame 0 (their centre is worked out and never used).
                for p in &o.pieces {
                    if let Some(m) = animate(&self.nodes[p.node], 0.0) {
                        self.node_mtx[p.node] = m;
                    }
                }
            }
            for k in 0..self.objs[i].pieces.len() {
                let p = self.objs[i].pieces[k];
                let mut pos = [0.0; 3];
                let prev = self.node_mtx[p.node][3];
                if self.nodes[p.node].is_rigid() {
                    if first {
                        pos = animate(&self.nodes[p.node], 0.0).map_or([0.0; 3], |m| {
                            self.node_mtx[p.node] = m;
                            [m[3][0], m[3][1], m[3][2]]
                        });
                        let body = self.rigid.create(pos, p.radius);
                        self.node_body[p.node] = body;
                        if body >= 0 {
                            self.rigid.launch(body, hit_pos, material, vu0, libc);
                        }
                    }
                    if let Some(m) = self.rigid.matrix(self.node_body[p.node]) {
                        self.node_mtx[p.node] = m;
                    }
                } else {
                    self.node_body[p.node] = 0;
                    if let Some(m) = animate(&self.nodes[p.node], frame as f32) {
                        self.node_mtx[p.node] = m;
                        pos = [m[3][0], m[3][1], m[3][2]];
                    }
                }
                // The dust of the break, at every fourth piece (`EftGndDust_SpawnDebris`).
                if first
                    && p.index & 3 == 0
                    && let Some(bright) = dust_bright
                {
                    dust.spawn_debris(pos, bright, vu0);
                }
                if first {
                    continue;
                }
                let now = self.node_mtx[p.node][3];
                if self.objs[i].kind & SPLASH != 0
                    && prev[1] < 0.0
                    && 0.0 < now[1]
                    && let Some(w) = water.as_deref_mut()
                {
                    w.add_splash_at([prev[0], prev[1], prev[2]], 3.0, libc);
                }
            }
        }
        // StgModel_UpdateAnims, between StgAmb_Update and StgRigid_Update.
        {
            let objs = &self.objs;
            let broken = |i: usize| objs.get(i).is_some_and(|o| o.state & BROKEN != 0);
            self.anims.update(&self.defs, broken);
        }
        self.rigid.update(inner_radius);
    }

    /// The pieces of the falling objects that are shown (`StgModel_FadeDebris`: those with a body or keys).
    pub(crate) fn falling_pieces(&self) -> Vec<FallingPiece> {
        let mut out = Vec::new();
        for o in self.objs.iter().filter(|o| o.state & FALLING != 0) {
            // Stg_FadeRatio: fading over the last 30 frames.
            let d = (o.anim_end - o.frame) as f32;
            let ratio = if d < 30.0 { d / 30.0 } else { 1.0 };
            for p in o.pieces.iter().filter(|p| self.node_body[p.node] >= 0) {
                out.push(FallingPiece {
                    part: p.part,
                    node: p.node,
                    ratio,
                });
            }
        }
        out
    }

    /// The pieces' current matrices, by node.
    pub(crate) fn node_matrices(&self) -> &[[[f32; 4]; 4]] {
        &self.node_mtx
    }

    /// Hands the broken set to the stage model when it changed (`StgModel_Cull` reads the parts' flags).
    pub(crate) fn sync_model(&mut self, model: &mut StageModel) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        for i in 0..self.objs.len() {
            model.set_broken(i, self.is_broken(i));
        }
        model.refresh_drawable();
    }
}

/// `StgObj_GetMaterial`: 0, 1 or 2 by type bits 1, 2, 4.
/// `StgObj_GetDust`: an object of a material (type bits 0..2) raises dust, of brightness 0.7 with type bit 0x200,
/// else 1; `None` without a material.
fn dust_scale(kind: i32) -> Option<f32> {
    if kind & 7 == 0 {
        None
    } else if kind & 0x200 != 0 {
        Some(0.7)
    } else {
        Some(1.0)
    }
}

fn material(kind: i32) -> i32 {
    if kind & 1 != 0 {
        0
    } else if kind & 2 != 0 {
        1
    } else if kind & 4 != 0 {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objs(parents: &[i32]) -> StageObjs {
        StageObjs {
            objs: parents
                .iter()
                .map(|&parent| StgObj {
                    hp: 3,
                    kind: SHAKE_M | SHAKE_S,
                    parent,
                    part: Some(([1.0, 2.0, 3.0], 50.0)),
                    ..StgObj::default()
                })
                .collect(),
            ..StageObjs::default()
        }
    }

    #[test]
    fn breaking_an_object_breaks_its_children_and_shakes_by_the_smallest_size() {
        let mut s = objs(&[-1, 0, 1, -1]);
        assert!(!s.damage(0, 2));
        assert!(s.damage(0, 1));
        let mut snd = crate::btl_snd::CharSound::default();
        s.destroy(0, [0.0; 3], &mut snd);
        assert_eq!(s.broken_mask(), 0b0111);
        // The break sound of the last object broken.
        assert_eq!(snd.requests(), [(-1, crate::btl_snd::KIND_BANK8, 8)]);
        assert_eq!(s.kind(0), 0);
        assert!(!s.damage(1, 10));
        assert_eq!(s.shakes.len(), 3);
        assert_eq!((s.shakes[0].strength, s.shakes[0].near), (1.0, 150.0));
        // The pieces fall for 0x94 frames, two a frame.
        let (mut vu0, mut libc) = (Vu0Random::default(), LibcRandom::new(1));
        for _ in 0..0x4A {
            s.update(
                1000.0,
                &mut vu0,
                &mut libc,
                &mut crate::eft_gdust::GndDust::default(),
                None,
            );
        }
        assert!(s.objs.iter().all(|o| o.state & FALLING == 0));
        s.reset(false);
        assert_eq!(s.broken_mask(), 0);
    }

    #[test]
    fn a_piece_falling_through_the_water_line_splashes_once() {
        use bt3_data::stage_model::StageKey;
        let key = |frame, y| StageKey {
            pos: [5.0, y, 7.0],
            frame,
            rot: [0.0; 3],
        };
        let node = StageNode {
            mtx: crate::battle::IDENTITY,
            keys: vec![key(0, -19.0), key(40, 21.0)],
            end_frame: 40,
        };
        let piece = Piece {
            index: 0,
            part: 0,
            node: 0,
            radius: 1.0,
        };
        let mut s = StageObjs {
            objs: vec![StgObj {
                state: FALLING,
                kind: SPLASH,
                pieces: vec![piece],
                ..StgObj::default()
            }],
            nodes: vec![node],
            node_mtx: vec![crate::battle::IDENTITY],
            node_body: vec![-1],
            ..StageObjs::default()
        };
        let mut water = crate::eft_water::Water::new(30.0, 0, [0.0; 4], vec![0; 5]);
        let (mut vu0, mut libc) = (Vu0Random::default(), LibcRandom::new(1));
        let mut dust = crate::eft_gdust::GndDust::default();
        let mut splashes = Vec::new();
        for _ in 0..30 {
            s.update(1000.0, &mut vu0, &mut libc, &mut dust, Some(&mut water));
            splashes.push(water.groups().0);
        }
        // y goes from -19 by 1 a frame, two frames an update: from -1 to 1 on the tenth.
        assert_eq!(splashes.iter().position(|&n| n == 1), Some(9));
        assert_eq!(splashes.last(), Some(&1));
    }
}
