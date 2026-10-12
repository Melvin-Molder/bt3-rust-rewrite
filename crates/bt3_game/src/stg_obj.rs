//! The stage's destructible objects (src/battle/stg_parts.c): their hit points, breaking (with the objects that
//! name the broken one as parent), the shakes a break gives the cameras near it, and the time their pieces fall.
//!
//! Not ported: the debris pieces' animation and rigid bodies, their dust and splashes, the break sound, the pads'
//! rumble, the hidden item of mode 1 and the way-point graph's object flags (nothing reads them).

use bt3_data::stage::StageObjectDef;
use bt3_data::stage_model::StageModel;

/// `STG_OBJ_BROKEN` / `STG_OBJ_FALLING`.
const BROKEN: u32 = 1;
const FALLING: u32 = 2;

/// `STG_OBJ_SHAKE_S` / `M` / `L`.
const SHAKE_S: i32 = 0x10;
const SHAKE_M: i32 = 0x20;
const SHAKE_L: i32 = 0x40;

/// `StgObj`.
#[derive(Clone, Copy, Debug, Default)]
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
}

/// A camera shake a break asks for (`BtlCharApi_ShakeCamsNear`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shake {
    pub pos: [f32; 3],
    pub near: f32,
    pub far: f32,
    pub strength: f32,
    pub time: f32,
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
                ..StgObj::default()
            })
            .collect();
        let mut s = Self {
            objs,
            defs: defs.to_vec(),
            shakes: Vec::new(),
            dirty: false,
        };
        s.reset(false);
        s
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

    /// `BtlStage_DestroyObj`: breaks object `i` and the objects under it. The break sound and the hidden item are
    /// not ported.
    pub(crate) fn destroy(&mut self, i: usize, hit_pos: [f32; 3]) {
        self.break_obj(i, hit_pos);
    }

    /// `BtlStage_BreakObj`: the broken model instead of the whole one, the pieces start falling, the cameras near
    /// part 0 shake by the type's size (large, medium, small: the last one listed wins); then every object that
    /// names it as parent. False when it was already broken.
    fn break_obj(&mut self, i: usize, hit_pos: [f32; 3]) -> bool {
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
        if let Some((pos, r)) = o.part
            && o.kind & (SHAKE_S | SHAKE_M | SHAKE_L) != 0
        {
            let mut shake = None;
            if o.kind & SHAKE_L != 0 {
                shake = Some((3.0, 0.7, 300.0));
            }
            if o.kind & SHAKE_M != 0 {
                shake = Some((2.0, 0.5, 100.0));
            }
            if o.kind & SHAKE_S != 0 {
                shake = Some((1.0, 0.3, 100.0));
            }
            if let Some((strength, time, near)) = shake {
                self.shakes.push(Shake {
                    pos,
                    near: near + r,
                    far: 1500.0 + r,
                    strength,
                    time,
                });
            }
        }
        for j in 0..self.objs.len() {
            if self.objs[j].parent == i as i32 {
                self.break_obj(j, hit_pos);
            }
        }
        true
    }

    /// `BtlStage_UpdateObjs` (not while paused): the falling objects' frame advances by 2 until their animation's
    /// end (0x94 when 0), which ends the fall.
    pub(crate) fn update(&mut self) {
        for o in &mut self.objs {
            if o.state & FALLING == 0 {
                continue;
            }
            o.frame += 2;
            if o.anim_end == 0 {
                o.anim_end = 0x94;
            }
            if o.anim_end <= o.frame {
                o.frame = 0;
                o.state ^= FALLING;
            }
        }
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
        s.destroy(0, [0.0; 3]);
        assert_eq!(s.broken_mask(), 0b0111);
        assert_eq!(s.kind(0), 0);
        assert!(!s.damage(1, 10));
        assert_eq!(s.shakes.len(), 3);
        assert_eq!((s.shakes[0].strength, s.shakes[0].near), (1.0, 150.0));
        // The pieces fall for 0x94 frames, two a frame.
        for _ in 0..0x4A {
            s.update();
        }
        assert!(s.objs.iter().all(|o| o.state & FALLING == 0));
        s.reset(false);
        assert_eq!(s.broken_mask(), 0);
    }
}
