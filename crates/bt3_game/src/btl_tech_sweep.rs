//! The sweep, technique effect type 3 (`EftSweep_*`, src/battle/eft_sweep.c): from the FIRE event the beam's end
//! goes round a circle about the firing node, in the plane of the aim, of the fighters' distance plus 50 (at most
//! 800); each frame the record runs from the firing node to the beam's end (it never meets the stage), and where
//! the stage cuts the beam a mark is left for ten frames. A hit or a guard stops its records; the END event, or
//! ABORT at once, ends it once its marks are over; its end raises flag 0xA8.
//!
//! Its emitters are driven at the beam's end along the aim (`EftSweep_Emit`).
//!
//! The follow of type 7 (`EftFollow_*`, the tail of the same module) is here too.
//!
//! A mark pulls the beam's end back onto the stage, breaks the stage object the beam met and, ten frames later,
//! starts the technique's stage impact there.

use bt3_data::eft_pack::{EmitSet, SetHead};

use crate::btl_blast::{HitRec, Tag, World};
use crate::btl_chr::{Chr, SideData, add, length, scale3, sub};
use crate::btl_tech_fx::{
    EV_ABORT, EV_END, EV_FIRE, EV_START, EmitNodes, T_HIT_CHAR, T_MULTI, TechSrc, aim_dir,
    aim_dir_keep, emit_state, parts_alive, stopped, update_parts, update_trail_width,
};
use crate::eft::{EmitState, Modules};

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
    emit: Option<EmitState>,
    head_set: SetHead,
    trail_width: f32,
    trail_time: f32,
    /// The stage marks' timers and positions (`EftSweepMark`), `None` free.
    marks: [Option<(f32, [f32; 3])>; 15],
    /// The blast impacts the marks started this update.
    pub(crate) mark_impacts: Vec<(crate::eft_impact::ImpactArg, f32)>,
    dead: bool,
}

/// Where a mark lies: `half` from the hit back towards the beam's origin.
fn mark_at(hit: [f32; 3], origin: [f32; 3], half: f32) -> [f32; 3] {
    add(hit, scale3(crate::btl_blast::norm(sub(origin, hit)), half))
}

/// The marks count down; one that ran out starts the stage impact along `dir`. Whether any is left.
fn step_marks(
    marks: &mut [Option<(f32, [f32; 3])>; 15],
    src: &TechSrc,
    dir: [f32; 3],
    out: &mut Vec<(crate::eft_impact::ImpactArg, f32)>,
) -> bool {
    let mut any = false;
    let def = &src.def;
    for m in marks.iter_mut() {
        if let Some((t, at)) = m {
            if *t <= 0.0 {
                if def.ground_fx >= 0 {
                    let arg = crate::eft_impact::ImpactArg {
                        pos: *at,
                        dir,
                        id: def.ground_fx,
                        chr: src.side,
                    };
                    out.push((arg, def.ground_scale));
                }
                *m = None;
            } else {
                any = true;
                *t -= 1.0;
            }
        }
    }
    any
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
    pub(crate) fn new(
        id: u32,
        src: TechSrc,
        me: &Chr,
        opp: &Chr,
        data: &SideData,
        set: &EmitSet,
    ) -> Self {
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
            emit: emit_state(data, src.slot),
            head_set,
            trail_width: head_set.width[0],
            trail_time: 0.0,
            marks: [None; 15],
            mark_impacts: Vec::new(),
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

    /// `EftSweep_Reset`: its emitters' objects die.
    pub(crate) fn kill_parts(&mut self, fx: &mut Modules) {
        if let Some(st) = &mut self.emit {
            st.kill_all(fx);
        }
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

    /// `EftSweep_AddMark`: a free mark for ten frames, half the beam's width back from the hit towards the origin;
    /// the beam's end is pulled back to it, and a standing stage object the trace hit breaks, away from the
    /// origin. Nothing without a free mark.
    fn add_mark(&mut self, w: &World, hit: bt3_data::stage_col::SegHit) {
        let width = self.width * self.trail_width * 0.5;
        let Some(m) = self.marks.iter_mut().find(|m| m.is_none()) else {
            return;
        };
        let at = mark_at(hit.pos, self.pos, width);
        *m = Some((10.0, at));
        self.cur = at;
        if let Ok(obj) = usize::try_from(hit.obj) {
            let dir = crate::btl_blast::norm(sub(self.cur, self.pos));
            w.env.objs().destroy(obj, dir, &mut w.env.snd());
        }
    }

    /// `EftSweep_UpdateMarks`: the marks count down; whether any is left. A mark that ran out starts the
    /// technique's stage impact (`markEffect` / `markSize`: the definition's `groundFx` and `groundScale`), along
    /// the beam.
    fn update_marks(&mut self) -> bool {
        let dir = crate::btl_blast::norm(sub(self.cur, self.pos));
        step_marks(&mut self.marks, &self.src, dir, &mut self.mark_impacts)
    }

    /// `EftSweep_Update`. Returns the record.
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        events: u32,
        cur_slot: i32,
    ) -> Option<HitRec> {
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
                    if let Some(hit) = w.env.col.trace_segment(self.pos, self.cur, w.env.broken()) {
                        self.add_mark(w, hit);
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
        if let Some(st) = &mut self.emit {
            let (ending, now) = (self.flags & S_ENDING != 0, self.flags & S_NOW != 0);
            update_parts(
                fx,
                w,
                &self.src,
                st,
                &self.nodes,
                events,
                ending,
                now,
                self.cur,
                self.dir,
                self.width,
                |_, _, _| false,
            );
        }
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
    pub(crate) fn post_update(&mut self, w: &World, fx: &mut Modules) {
        if stopped(w, self.src.side) {
            return;
        }
        if let Some(st) = &mut self.emit {
            parts_alive(fx, w.data[self.src.side], &self.src, st, false);
        }
        if self.tag.flags & 3 != 0 {
            self.flags &= !S_TRAIL;
        }
    }
}

// EFT_FOLLOW_*.
const F_ENDING: u32 = 0x1;
const F_DEAD: u32 = 0x2;
const F_NOW: u32 = 0x4;
const F_STOP_SEEN: u32 = 0x8;
const F_TRAIL: u32 = 0x10;
const F_STARTED: u32 = 0x40;

/// An `EftFollow` item (technique effect type 7, the steered dashes): from the FIRE event a record of two spheres
/// at the fighter's node 3 now and last frame, as wide as the technique's scale times the set's trail width; the
/// stop (END) or ABORT events end it once the set's end frames are over, and its end raises flag 0xA8, which ends
/// the dash. The stage light (parameter flag 0x200) is not ported.
#[derive(Clone, Debug)]
pub(crate) struct Follow {
    id: u32,
    src: TechSrc,
    pub(crate) tag: Tag,
    flags: u32,
    state: u8,
    width: f32,
    time: f32,
    life: f32,
    /// From node 0x2E to node 3.
    dir: [f32; 3],
    start: [f32; 3],
    cur: [f32; 3],
    prev: [f32; 3],
    nodes: EmitNodes,
    emit: Option<EmitState>,
    head_set: SetHead,
    trail_width: f32,
    trail_time: f32,
    dead: bool,
}

impl Follow {
    /// `EftFollow_Init` (the speed of a set with its own width is never read).
    pub(crate) fn new(id: u32, src: TechSrc, data: &SideData, set: &EmitSet) -> Self {
        let head_set = set.head;
        Self {
            id,
            src,
            tag: Tag::default(),
            flags: 0,
            state: 0,
            width: src.def.scale,
            time: 0.0,
            life: f32::from(head_set.end_frames),
            dir: [0.0; 3],
            start: [0.0; 3],
            cur: [0.0; 3],
            prev: [0.0; 3],
            nodes: EmitNodes::default(),
            emit: emit_state(data, src.slot),
            head_set,
            trail_width: head_set.width[0],
            trail_time: 0.0,
            dead: false,
        }
    }

    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    pub(crate) fn dead(&self) -> bool {
        self.dead
    }

    /// `EftFollow_Reset`: its emitters' objects die.
    pub(crate) fn kill_parts(&mut self, fx: &mut Modules) {
        if let Some(st) = &mut self.emit {
            st.kill_all(fx);
        }
    }

    /// `EftFollow_Update`. Returns the record.
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        events: u32,
        cur_slot: i32,
    ) -> Option<HitRec> {
        let side = self.src.side;
        if stopped(w, side) {
            return None;
        }
        let def = self.src.def;
        let test = |b: u32| events & b != 0;
        self.nodes
            .update(&w.chars[side], w.data[side], &def, cur_slot, events);
        if self.flags & F_STARTED == 0 && (test(EV_START) || test(EV_FIRE)) {
            self.state = 0;
            self.flags |= F_STARTED;
        }
        let mut next = false;
        if self.flags & F_STARTED != 0 {
            match self.state {
                0 => {
                    if test(EV_FIRE) {
                        self.flags |= F_TRAIL;
                        let obj = &w.chars[side].obj;
                        self.start = obj.node_pos(3);
                        self.cur = self.start;
                        self.prev = obj.node_pos(0x11);
                        self.state = 1;
                    }
                }
                2 => {
                    if test(EV_END) {
                        self.flags |= F_STOP_SEEN | F_ENDING;
                        self.state = 3;
                    }
                }
                1 | 3 => next = true,
                _ => {}
            }
        }
        if test(EV_ABORT) {
            if self.flags & F_STOP_SEEN == 0 {
                self.flags |= F_NOW;
            }
            self.flags |= F_ENDING;
        }
        let obj = &w.chars[side].obj;
        let d = sub(obj.node_pos(3), obj.node_pos(0x2E));
        let len = length(d);
        self.dir = if len > 0.0 { scale3(d, 1.0 / len) } else { d };
        self.prev = self.cur;
        self.cur = obj.node_pos(3);
        if let Some(st) = &mut self.emit {
            let (ending, now) = (self.flags & F_ENDING != 0, self.flags & F_NOW != 0);
            update_parts(
                fx,
                w,
                &self.src,
                st,
                &self.nodes,
                events,
                ending,
                now,
                self.cur,
                self.dir,
                self.width,
                |_, _, _| false,
            );
        }
        if next {
            self.state += 1;
        }
        if self.flags & F_ENDING != 0 {
            self.time += 1.0;
        }
        if self.flags & F_DEAD != 0 {
            self.dead = true;
            return None;
        }
        if self.flags & F_ENDING != 0 {
            if self.flags & F_NOW != 0 || self.time >= self.life {
                self.flags |= F_DEAD;
            }
            return None;
        }
        if self.flags & F_TRAIL == 0 {
            return None;
        }
        update_trail_width(&mut self.trail_width, &mut self.trail_time, &self.head_set);
        if w.paused {
            return None;
        }
        // EftFollow_AddHit: two spheres, never meeting the stage.
        let mut rec = HitRec::technique_shape(
            self.src,
            self.id,
            self.cur,
            self.prev,
            [0.0; 3],
            self.start,
            self.width * self.trail_width,
            0,
        )?;
        rec.flags |= H_NEVER;
        Some(rec)
    }

    /// `EftFollow_PostUpdate`: a hit (task flag 1, marking the phase 6 emitters) or a guard (2) stops its records.
    pub(crate) fn post_update(&mut self, w: &World, fx: &mut Modules) {
        if stopped(w, self.src.side) {
            return;
        }
        let hit = self.tag.flags & 1 != 0;
        if let Some(st) = &mut self.emit {
            parts_alive(fx, w.data[self.src.side], &self.src, st, hit);
        }
        if self.tag.flags & 3 != 0 {
            self.flags &= !F_TRAIL;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btl_chr::dot;

    #[test]
    fn a_mark_starts_the_stage_impact_ten_frames_on() {
        // Half the beam's width back from the hit towards the origin.
        assert_eq!(mark_at([10.0, 0.0, 0.0], [0.0; 3], 2.0), [8.0, 0.0, 0.0]);
        let mut src = TechSrc {
            side: 1,
            slot: 2,
            def: crate::btl_tech_fx::TechDef::default(),
        };
        src.def.ground_fx = 3;
        src.def.ground_scale = 1.5;
        let mut marks = [None; 15];
        marks[4] = Some((10.0, [1.0, 2.0, 3.0]));
        let mut out = Vec::new();
        let dir = [0.0, 0.0, 1.0];
        for _ in 0..10 {
            assert!(step_marks(&mut marks, &src, dir, &mut out));
            assert!(out.is_empty());
        }
        assert!(!step_marks(&mut marks, &src, dir, &mut out));
        assert_eq!(out.len(), 1);
        let (arg, scale) = &out[0];
        assert_eq!(
            (arg.pos, arg.dir, arg.id, arg.chr, *scale),
            ([1.0, 2.0, 3.0], dir, 3, 1, 1.5)
        );
        // Without a stage impact the mark only ends.
        src.def.ground_fx = -1;
        marks[0] = Some((0.0, [0.0; 3]));
        out.clear();
        assert!(!step_marks(&mut marks, &src, dir, &mut out));
        assert!(out.is_empty() && marks.iter().all(Option::is_none));
    }

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
