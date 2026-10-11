//! The sweep, technique effect type 3 (`EftSweep_*`, src/battle/eft_sweep.c): from the FIRE event the beam's end
//! goes round a circle about the firing node, in the plane of the aim, of the fighters' distance plus 50 (at most
//! 800); each frame the record runs from the firing node to the beam's end (it never meets the stage), and where
//! the stage cuts the beam a mark is left for ten frames. A hit or a guard stops its records; the END event, or
//! ABORT at once, ends it once its marks are over; its end raises flag 0xA8.
//!
//! Not ported: the marks' impact effects and the stage objects they break, and the emitters (drawing).

use bt3_data::eft_pack::{EmitSet, SetHead};

use crate::btl_blast::{HitRec, Tag, World};
use crate::btl_chr::{Chr, add, length, scale3, sub};
use crate::btl_tech_fx::{
    EV_ABORT, EV_END, EV_FIRE, EV_START, EmitNodes, T_HIT_CHAR, T_MULTI, TechSrc, aim_dir,
    aim_dir_keep, stopped, update_trail_width,
};

// EFT_SWEEP_*.
const S_ENDING: u32 = 0x1;
const S_DEAD: u32 = 0x2;
const S_NOW: u32 = 0x4;
const S_STOP_SEEN: u32 = 0x8;
const S_TRAIL: u32 = 0x10;
const S_STARTED: u32 = 0x20;
const S_PREV_VALID: u32 = 0x40;

/// `EFT_SWEEP_STEP` / `EFT_SWEEP_START`.
const STEP: f32 = 6.283_185_3 * 0.003;
const START: f32 = 6.283_185_3 * 0.003 * -11.0;

/// `EFT_HIT_FLAG_NEVER`: the record never meets the stage.
const H_NEVER: u32 = 0x40;

/// An `EftSweep` item.
#[derive(Clone, Debug)]
pub(crate) struct Sweep {
    id: u32,
    src: TechSrc,
    pub(crate) tag: Tag,
    flags: u32,
    state: u8,
    speed: f32,
    width: f32,
    time: f32,
    life: f32,
    radius: f32,
    angle: f32,
    /// The beam's origin and the circle's centre (node slot 1).
    pos: [f32; 3],
    dir: [f32; 3],
    cur: [f32; 3],
    prev: [f32; 3],
    vel: [f32; 3],
    nodes: EmitNodes,
    head_set: SetHead,
    trail_width: f32,
    trail_time: f32,
    /// The stage marks' timers (`EftSweepMark`), `None` free.
    marks: [Option<f32>; 15],
    dead: bool,
}

/// The distance between the fighters' node 3, plus 50, at most 800.
fn path_radius(me: &Chr, opp: &Chr) -> f32 {
    (length(sub(opp.obj.node_pos(3), me.obj.node_pos(3))) + 50.0).min(800.0)
}

/// Row vector times `Mtx_RotateY`'s matrix.
fn rot_y(v: [f32; 3], a: f32) -> [f32; 3] {
    let (s, c) = a.sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

/// Row vector times `Mtx_RotateX`'s matrix.
fn rot_x(v: [f32; 3], a: f32) -> [f32; 3] {
    let (s, c) = a.sin_cos();
    [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
}

impl Sweep {
    /// `EftSweep_Init` with `EftSweep_InitPath`.
    pub(crate) fn new(id: u32, src: TechSrc, me: &Chr, opp: &Chr, set: &EmitSet) -> Self {
        let def = src.def;
        let head_set = set.head;
        let mut s = Self {
            id,
            src,
            tag: Tag::default(),
            flags: 0,
            state: 0,
            speed: def.speed,
            width: def.scale,
            time: 0.0,
            life: f32::from(head_set.end_frames),
            radius: 0.0,
            angle: START,
            pos: [0.0; 3],
            dir: [0.0; 3],
            cur: [0.0; 3],
            prev: [0.0; 3],
            vel: [0.0; 3],
            nodes: EmitNodes::default(),
            head_set,
            trail_width: head_set.width[0],
            trail_time: 0.0,
            marks: [None; 15],
            dead: false,
        };
        if head_set.flags & 4 != 0 {
            s.speed = head_set.width_b[0];
        }
        s.radius = path_radius(me, opp);
        s
    }

    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    pub(crate) fn src(&self) -> &TechSrc {
        &self.src
    }

    pub(crate) fn dead(&self) -> bool {
        self.dead
    }

    /// `EftSweep_Move`: the end one step further round its circle about `pos`, in the plane of the aim (the
    /// circle's matrix is built on the step's, so the step's angle counts twice).
    fn step(&mut self, w: &World) {
        let side = self.src.side;
        self.radius = path_radius(&w.chars[side], &w.chars[side ^ 1]);
        let mut cur = rot_y([0.0, 0.0, self.radius], self.angle);
        let pitch = (-self.dir[1]).asin();
        let yaw = self.dir[0].atan2(self.dir[2]);
        cur = rot_y(rot_x(rot_y(cur, self.angle), pitch), yaw);
        if self.flags & S_PREV_VALID == 0 {
            self.prev = add(cur, self.pos);
            self.flags |= S_PREV_VALID;
        }
        self.cur = add(cur, self.pos);
        self.angle += STEP;
        if 3.141_592_7 < self.angle {
            self.angle -= 6.283_185_3;
        }
    }

    /// `EftSweep_AddMark`: a free mark for ten frames (its position only places the impact effect).
    fn add_mark(&mut self) {
        if let Some(m) = self.marks.iter_mut().find(|m| m.is_none()) {
            *m = Some(10.0);
        }
    }

    /// `EftSweep_UpdateMarks`: the marks count down; whether any is left.
    fn update_marks(&mut self) -> bool {
        let mut any = false;
        for m in &mut self.marks {
            if let Some(t) = m {
                if *t <= 0.0 {
                    *m = None;
                } else {
                    any = true;
                    *t -= 1.0;
                }
            }
        }
        any
    }

    /// `EftSweep_Update`. Returns the record.
    pub(crate) fn update(&mut self, w: &World, events: u32, cur_slot: i32) -> Option<HitRec> {
        let side = self.src.side;
        if stopped(w, side) {
            return None;
        }
        let def = self.src.def;
        let test = |b: u32| events & b != 0;
        self.nodes
            .update(&w.chars[side], w.data[side], &def, cur_slot, events);
        if self.flags & S_STARTED == 0 {
            if test(EV_START) {
                self.dir = aim_dir(w, side, self.nodes.pos(0));
                self.state = 0;
                self.flags |= S_STARTED;
            } else if test(EV_FIRE) {
                self.state = 0;
                self.flags |= S_STARTED;
            }
        }
        let mut next = false;
        if self.flags & S_STARTED != 0 {
            match self.state {
                0 => {
                    if test(EV_FIRE) {
                        self.flags |= S_TRAIL;
                        self.pos = self.nodes.pos(1);
                        self.cur = self.pos;
                        self.prev = w.chars[side].obj.node_pos(0x11);
                        self.dir = aim_dir_keep(w, side, self.cur);
                        self.vel = scale3(self.dir, self.speed);
                        self.state = 1;
                    }
                }
                2 => {
                    self.prev = self.cur;
                    self.pos = self.nodes.pos(1);
                    self.step(w);
                    if w.env.col.trace_segment(self.pos, self.cur).is_some() {
                        self.add_mark();
                    }
                    if test(EV_END) {
                        self.flags |= S_STOP_SEEN | S_ENDING;
                        self.state = 3;
                    }
                }
                1 | 3 => next = true,
                _ => {}
            }
        }
        if test(EV_ABORT) {
            if self.flags & S_STOP_SEEN == 0 {
                self.flags |= S_NOW;
            }
            self.flags |= S_ENDING;
        }
        // EftSweep_Emit: drawing only.
        if next {
            self.state += 1;
        }
        let any = self.update_marks();
        if self.flags & S_ENDING != 0 {
            self.time += 1.0;
        }
        if !any && self.flags & S_DEAD != 0 {
            self.dead = true;
            return None;
        }
        if self.flags & S_ENDING != 0 {
            if self.flags & S_NOW != 0 || self.time >= self.life {
                self.flags |= S_DEAD;
            }
            return None;
        }
        if def.flags & 1 == 0 || self.flags & S_TRAIL == 0 {
            return None;
        }
        update_trail_width(&mut self.trail_width, &mut self.trail_time, &self.head_set);
        if w.paused {
            return None;
        }
        // EftSweep_AddHit: from the firing node (shape 1) or about the end.
        let mut rec = HitRec::technique(
            self.src,
            self.id,
            self.cur,
            self.prev,
            self.vel,
            self.nodes.pos(1),
            self.width * self.trail_width,
        )?;
        rec.flags |= H_NEVER;
        if def.max_hits > 0 {
            self.tag.flags |= T_MULTI;
            self.tag.flags &= !T_HIT_CHAR;
        }
        Some(rec)
    }

    /// `EftSweep_PostUpdate`: a hit or a guard (task flags 1 / 2) stops its records.
    pub(crate) fn post_update(&mut self, w: &World) {
        if stopped(w, self.src.side) {
            return;
        }
        if self.tag.flags & 3 != 0 {
            self.flags &= !S_TRAIL;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btl_chr::dot;

    #[test]
    fn row_rotations_follow_the_matrix_rows() {
        // (0, 0, 1) through Mtx_RotateY(a) is (sin a, 0, cos a); through Mtx_RotateX(p) it is (0, -sin p, cos p).
        let v = rot_y([0.0, 0.0, 1.0], 0.5);
        assert!((v[0] - 0.5f32.sin()).abs() < 1e-6 && (v[2] - 0.5f32.cos()).abs() < 1e-6);
        let v = rot_x([0.0, 0.0, 1.0], 0.5);
        assert!((v[1] + 0.5f32.sin()).abs() < 1e-6 && (v[2] - 0.5f32.cos()).abs() < 1e-6);
        assert!((dot(v, v) - 1.0).abs() < 1e-6);
    }
}
