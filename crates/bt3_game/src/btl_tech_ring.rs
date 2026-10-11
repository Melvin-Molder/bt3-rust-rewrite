//! The ring shot, technique effect type 4 (`EftRingShot_*`, src/battle/eft_ring_shot.c), by technique id:
//! 0 (0x26D) / 1 (0x18D): on FIRE the definition's shots are created at the firing node and slide to places on
//! two rings round the opponent (0) or the fighter itself (1), bob there held harmless, and once all have arrived
//! flag 0xA8 is raised; event 0x40 launches them at the opponent, the later the earlier made. 2 (0x156, 0x1DC,
//! 0x2EC): a carrier point flies a spline above the opponent in 20 frames with a hit record, raising 0xA8; event
//! 0x10 then fires a volley of shots from it at the opponent, two of three spread. 3 (0x197, 0x198, 0x200, 0x2A4):
//! a hit record at the opponent's node 3 from event 0x10. Its end raises flag 0xA9 (and 0xA8 when not yet).
//!
//! Not ported: the emitters (drawing).

use bt3_data::eft_pack::{EmitSet, SetHead};

use crate::btl_blast::{HitRec, Tag, World};
use crate::btl_chr::{add, length, scale3, sub};
use crate::btl_tech_fx::{
    EV_ABORT, EV_END, EV_FIRE, EV_START, EmitNodes, T_HIT_CHAR, T_MULTI, TechSrc, aim_dir,
    aim_dir_keep, stopped, update_trail_width,
};
use crate::btl_tech_volley::{Shots, scene_rand_f};

// EFT_RINGSHOT_*.
const R_COUNT: u32 = 0x1;
const R_KILL: u32 = 0x2;
const R_FAST_END: u32 = 0x4;
const R_ENDED: u32 = 0x8;
const R_FIRED: u32 = 0x10;
const R_HIT_MOVED: u32 = 0x20;
const R_STARTED: u32 = 0x40;
const R_SHOT: u32 = 0x100;
const R_LAUNCHED: u32 = 0x200;
const R_DONE: u32 = 0x400;
const R_PATH: u32 = 0x800;

/// `EFT_HIT_FLAG_NO_MODE1`: the carrier's record does not meet the fighters.
const H_NO_MODE1: u32 = 0x80;

const PI_F: f32 = 3.141_592_7;
const TWO_PI: f32 = 6.283_185_3;

/// `EftRingShotOne`.
#[derive(Clone, Copy, Debug, Default)]
struct RingShot {
    pos: [f32; 3],
    dir: [f32; 3],
    home: [f32; 3],
    from: [f32; 3],
    /// 1 in use, 2 arrived, 4 last of the volley.
    flags: u32,
    index: i32,
    phase: f32,
    time: f32,
    slide_time: f32,
    handle: Option<u32>,
}

/// An `EftRingShot` item.
#[derive(Clone, Debug)]
pub(crate) struct Ring {
    id: u32,
    src: TechSrc,
    pub(crate) tag: Tag,
    flags: u32,
    state: u8,
    kind: i32,
    speed: f32,
    size: f32,
    timer: f32,
    life: f32,
    path_time: f32,
    dir: [f32; 3],
    start: [f32; 3],
    pos: [f32; 3],
    prev: [f32; 3],
    vel: [f32; 3],
    path: [[f32; 3]; 4],
    nodes: EmitNodes,
    shots: [RingShot; 20],
    fired: i32,
    total: i32,
    head_set: SetHead,
    trail_width: f32,
    trail_time: f32,
    /// Flag 0xA8 raised this frame (`EftShot_SetHeldFlagA8`).
    raise_a8: bool,
    dead: bool,
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

/// Row vector times `Mtx_RotateZ`'s matrix.
fn rot_z(v: [f32; 3], a: f32) -> [f32; 3] {
    let (s, c) = a.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
}

/// `EftMath_WrapAngle`.
fn eft_wrap(a: f32) -> f32 {
    if a >= PI_F {
        let n = (((a - PI_F) / TWO_PI) as i32).abs();
        a - (n as f32 * TWO_PI + TWO_PI)
    } else if a <= -PI_F {
        let n = (((a + PI_F) / TWO_PI) as i32).abs();
        a + (n as f32 * TWO_PI + TWO_PI)
    } else {
        a
    }
}

/// `EftMath_Spline3`: the quadratic through p0 (t = 0), p1 (t = 1) and p2 (t = 2).
fn spline3(p: &[[f32; 3]], t: f32) -> [f32; 3] {
    let (a, b, c) = (t * t - 3.0 * t + 2.0, -2.0 * t * t + 4.0 * t, t * t - t);
    std::array::from_fn(|i| (a * p[0][i] + b * p[1][i] + c * p[2][i]) / 2.0)
}

impl Ring {
    /// `EftRingShot_Init`.
    pub(crate) fn new(id: u32, src: TechSrc, set: &EmitSet) -> Self {
        let def = src.def;
        let kind = match def.id {
            0x26D => 0,
            0x18D => 1,
            0x156 | 0x1DC | 0x2EC => 2,
            0x197 | 0x198 | 0x200 | 0x2A4 => 3,
            _ => 0,
        };
        Self {
            id,
            src,
            tag: Tag::default(),
            flags: 0,
            state: 0,
            kind,
            speed: def.speed,
            size: def.scale,
            timer: 0.0,
            life: f32::from(set.head.end_frames),
            path_time: 0.0,
            dir: [0.0; 3],
            start: [0.0; 3],
            pos: [0.0; 3],
            prev: [0.0; 3],
            vel: [0.0; 3],
            path: [[0.0; 3]; 4],
            nodes: EmitNodes::default(),
            shots: [RingShot::default(); 20],
            fired: 0,
            total: def.shots * def.volley,
            head_set: set.head,
            trail_width: set.head.width[0],
            trail_time: 0.0,
            raise_a8: false,
            dead: false,
        }
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

    /// Whether flag 0xA8 is still owed at the end (`EftRingShot_Term`).
    pub(crate) fn done(&self) -> bool {
        self.flags & R_DONE != 0
    }

    /// `EftRingShot_Update`. Returns the record; raises flag 0xA8 when the ring or the path is done.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        events: u32,
        cur_slot: i32,
        requests: &mut u32,
        shots: &mut Shots,
        next_id: &mut u32,
        rand: &mut u32,
    ) -> Option<HitRec> {
        let side = self.src.side;
        if stopped(w, side) {
            return None;
        }
        let def = self.src.def;
        let test = |b: u32| events & b != 0;
        self.nodes
            .update(&w.chars[side], w.data[side], &def, cur_slot, events);
        if self.flags & R_STARTED == 0 {
            if test(EV_START) {
                self.dir = aim_dir(w, side, self.nodes.pos(0));
                self.state = 0;
                self.flags |= R_STARTED;
            } else if test(EV_FIRE) {
                self.state = 0;
                self.flags |= R_STARTED;
            }
        }
        let mut advance = false;
        if self.flags & R_STARTED != 0 {
            match self.state {
                0 => {
                    if test(EV_FIRE) {
                        self.flags |= R_FIRED;
                        if self.kind != 3 {
                            self.start = if self.kind == 2 {
                                self.nodes.pos(1)
                            } else {
                                w.chars[side].obj.node_pos(0x11)
                            };
                            self.pos = self.start;
                            self.prev = w.chars[side].obj.node_pos(0x11);
                            self.dir = aim_dir_keep(w, side, self.pos);
                            self.vel = scale3(self.dir, self.speed);
                        }
                        self.state = 1;
                    }
                }
                2 => {
                    if self.flags & R_HIT_MOVED == 0 {
                        self.prev = self.pos;
                    }
                    if test(EV_END) {
                        self.flags |= R_ENDED | R_COUNT;
                        self.state = 3;
                    }
                }
                1 | 3 => advance = true,
                _ => {}
            }
        }
        let ts = w.time_stopped;
        match self.kind {
            0 | 1 => self.update_ring(w, events, requests, shots, next_id, rand, ts),
            2 => self.update_path(w, events, requests, shots, next_id, rand, ts),
            _ => self.update_fixed(w, events),
        }
        if std::mem::take(&mut self.raise_a8) {
            w.chars[side].flags.set_held(0xA8);
        }
        let alive = self.reap(shots);
        if !alive && self.flags & R_SHOT != 0 {
            self.flags |= R_COUNT;
        }
        if test(EV_ABORT) {
            if self.flags & R_ENDED == 0 {
                self.flags |= R_FAST_END;
            }
            self.flags |= R_COUNT;
        }
        // EftRingShot_Draw: drawing only.
        if advance {
            self.state += 1;
        }
        if self.flags & R_COUNT != 0 {
            self.timer += 1.0;
        }
        if !alive && self.flags & R_KILL != 0 {
            self.dead = true;
            return None;
        }
        if self.flags & R_COUNT != 0 {
            if self.flags & R_FAST_END != 0 || self.timer >= self.life {
                self.flags |= R_KILL;
            }
            return None;
        }
        if def.flags & 1 == 0 || self.flags & R_FIRED == 0 {
            return None;
        }
        update_trail_width(&mut self.trail_width, &mut self.trail_time, &self.head_set);
        match self.kind {
            2 if self.flags & R_DONE == 0 => self.add_hit(w, true),
            3 => self.add_hit(w, false),
            _ => None,
        }
    }

    /// `EftRingShot_AddHit`: from the firing node (shape 1) or about `pos`; the carrier's never meets a fighter.
    fn add_hit(&mut self, w: &World, carrier: bool) -> Option<HitRec> {
        if w.paused {
            return None;
        }
        let mut rec = HitRec::technique(
            self.src,
            self.id,
            self.pos,
            self.prev,
            self.vel,
            self.nodes.pos(1),
            self.size * self.trail_width,
        )?;
        if carrier {
            rec.flags |= H_NO_MODE1;
        }
        if self.src.def.max_hits > 0 {
            self.tag.flags |= T_MULTI;
            self.tag.flags &= !T_HIT_CHAR;
        }
        Some(rec)
    }

    /// `EftRingShot_Fire`: one shot, or the definition's volley; each waits at its place (ring kinds) or flies at
    /// the opponent from the carrier (kind 2: three frames apart, two of three spread, timed by the definition).
    #[allow(clippy::too_many_arguments)]
    fn fire(
        &mut self,
        w: &World,
        volley: bool,
        shots: &mut Shots,
        next_id: &mut u32,
        rand: &mut u32,
        ts: bool,
    ) {
        let side = self.src.side;
        let def = self.src.def;
        let count = if volley { def.volley.max(1) } else { 1 };
        for i in 0..count {
            // EftRingShot_AllocShot: the first free entry.
            let Some(k) = self.shots.iter().position(|s| s.handle.is_none()) else {
                continue;
            };
            self.shots[k] = RingShot::default();
            let delay = def.life as f32 / 30.0;
            let mut life = 0.0;
            let s = &mut self.shots[k];
            s.index = self.fired;
            s.dir = self.dir;
            match self.kind {
                0 | 1 => {
                    s.pos = w.chars[side].obj.node_pos(self.nodes.node(1) as u32);
                    if self.fired == 0 {
                        s.flags |= 4;
                    }
                }
                2 => {
                    life = delay;
                    s.pos = self.pos;
                    if i + 1 >= count {
                        s.flags |= 4;
                    }
                }
                _ => {}
            }
            match self.kind {
                0 | 1 => {
                    let ring0 = self.kind == 0;
                    let half = self.total / 2;
                    let inner = ((half as f32 * 0.4) as i32).max(1);
                    let outer = half - inner;
                    let (owner, h_mid, r_out) = if ring0 {
                        (side ^ 1, 0.7, 70.0)
                    } else {
                        (side, 1.0, 30.0)
                    };
                    let center = w.chars[owner].obj.node_pos(0x11);
                    let scale = (w.data[owner].height() / 19.35).max(0.7);
                    let (mut h, mut r) = (1.0f32, 1.0f32);
                    let n = self.fired;
                    let a;
                    if n < half {
                        if n < inner {
                            h = 2.0;
                            r = if ring0 || inner >= 2 { 0.5 } else { 0.0 };
                            a = n as f32 / inner as f32 * TWO_PI;
                        } else {
                            h = if ring0 { h_mid } else { h };
                            a = (n - inner) as f32 / outer as f32 * TWO_PI;
                        }
                    } else if n - half < inner {
                        h = -2.0;
                        r = if ring0 || inner >= 2 { 0.5 } else { 0.0 };
                        a = (n - half + 1) as f32 / inner as f32 * TWO_PI;
                    } else {
                        h = -h_mid;
                        a = (n - (half + inner) + 1) as f32 / outer as f32 * TWO_PI;
                    }
                    let mut v = rot_y([0.0, 0.0, -1.0], eft_wrap(a));
                    let d = sub(center, self.start);
                    v = rot_y(v, d[0].atan2(d[2]));
                    let y_unit = if ring0 { 35.0 } else { 15.0 };
                    v = [
                        v[0] * scale * r_out * r,
                        v[1] - scale * y_unit * h,
                        v[2] * scale * r_out * r,
                    ];
                    s.home = add(center, v);
                    let mut p = s.home;
                    s.phase = scene_rand_f(rand, ts) * PI_F;
                    p[1] += s.phase.sin() * 10.0;
                    s.from = sub(p, self.start);
                    let min = if ring0 { 10.0 } else { 1.0 };
                    s.slide_time = length(s.from) / def.speed;
                    if s.slide_time < min {
                        s.slide_time = min;
                    }
                    s.time = 0.0;
                }
                2 => {
                    let h = scene_rand_f(rand, ts) * 0.35 + 0.15;
                    let target = w.chars[side ^ 1].obj.node_pos(0x11);
                    s.dir = crate::btl_blast::norm(sub(target, s.pos));
                    if i + 1 < count && i % 3 != 0 {
                        let a = eft_wrap(i as f32 / count as f32 * TWO_PI);
                        let v = rot_z([0.0, h, 1.0 - h], a);
                        let d = s.dir.map(|x| x.clamp(-1.0, 1.0));
                        let pitch = (-d[1]).asin();
                        let yaw = d[0].atan2(d[2]);
                        s.dir = crate::btl_blast::norm(rot_y(rot_x(v, pitch), yaw));
                    }
                }
                _ => {}
            }
            let (pos, dir) = (s.pos, s.dir);
            let handle = shots.create(
                next_id,
                self.src,
                &w.chars[side],
                &self.head_set,
                pos,
                dir,
                life,
                self.size,
                self.speed,
            );
            let s = &mut self.shots[k];
            s.handle = handle;
            s.flags |= 1;
            match self.kind {
                0 | 1 => shots.set_held(handle, true),
                2 => {
                    shots.set_delay(handle, i as f32 * 3.0);
                    shots.set_prev(handle, pos);
                }
                _ => {}
            }
            if s.flags & 4 != 0 {
                shots.mark_last(handle);
            }
            self.flags |= R_SHOT;
            self.fired += 1;
        }
    }

    /// `EftRingShot_BobShot`: 10 up and down about its place, 10.8 degrees a frame.
    fn bob(s: &mut RingShot, shots: &mut Shots) {
        let mut p = s.home;
        p[1] += s.phase.sin() * 10.0;
        s.phase += 10.8 / 180.0 * PI_F;
        if s.phase >= PI_F {
            s.phase -= TWO_PI;
        }
        shots.set_target(s.handle, Some(p));
    }

    /// `EftRingShot_UpdateRing`.
    #[allow(clippy::too_many_arguments)]
    fn update_ring(
        &mut self,
        w: &World,
        events: u32,
        requests: &mut u32,
        shots: &mut Shots,
        next_id: &mut u32,
        rand: &mut u32,
        ts: bool,
    ) {
        if self.flags & R_DONE == 0 {
            if events & EV_FIRE != 0 && (self.head_set.phase_mask >> 1) & 1 != 0 {
                self.fire(w, true, shots, next_id, rand, ts);
                restart(events, requests);
            }
            for s in &mut self.shots {
                if shots.alive(s.handle) {
                    if s.time > s.slide_time {
                        s.flags |= 2;
                    }
                    if s.flags & 2 == 0 {
                        let t = (s.time / s.slide_time).min(1.0);
                        shots.set_target(s.handle, Some(add(scale3(s.from, t), self.start)));
                    } else {
                        Self::bob(s, shots);
                    }
                    s.time += 1.0;
                } else if s.flags & 1 != 0 {
                    s.flags |= 2;
                }
            }
            if self.fired >= self.total
                && self
                    .shots
                    .iter()
                    .all(|s| s.handle.is_none() || s.flags & 2 != 0)
            {
                for s in &self.shots {
                    shots.set_held(s.handle, false);
                }
                self.raise_a8 = true;
                self.flags |= R_DONE;
            }
        } else {
            if self.flags & R_LAUNCHED == 0 {
                for s in &mut self.shots {
                    if s.handle.is_some() {
                        Self::bob(s, shots);
                    }
                }
            }
            if events & 0x40 != 0 {
                let target = w.chars[self.src.side ^ 1].obj.node_pos(0x11);
                for s in &self.shots {
                    if s.handle.is_some() {
                        shots.set_target(s.handle, None);
                        shots.set_dir(s.handle, crate::btl_blast::norm(sub(target, s.home)));
                        shots.set_delay(s.handle, (20 - s.index) as f32 * 0.75);
                    }
                }
                self.flags |= R_LAUNCHED;
            }
        }
    }

    /// `EftRingShot_UpdatePath`: on FIRE the carrier's spline from the start over the middle (raised by 10 times
    /// the opponent's scale plus 40) to above the opponent (25 times plus 145); 9 frames a half; at 20 frames flag
    /// 0xA8; then event 0x10 fires the volley.
    #[allow(clippy::too_many_arguments)]
    fn update_path(
        &mut self,
        w: &World,
        events: u32,
        requests: &mut u32,
        shots: &mut Shots,
        next_id: &mut u32,
        rand: &mut u32,
        ts: bool,
    ) {
        let side = self.src.side;
        if self.flags & R_DONE == 0 {
            if events & EV_FIRE != 0 {
                let opp = side ^ 1;
                let scale = (w.data[opp].height() / 19.35).max(0.6);
                let p0 = self.start;
                let mut p2 = w.chars[opp].obj.node_pos(0x11);
                let mut p1 = add(scale3(sub(p2, p0), 0.5), p0);
                p1[1] -= scale * 10.0 + 40.0;
                p2[1] -= scale * 25.0 + 145.0;
                self.path = [p0, p1, p2, p2];
                self.flags |= R_PATH;
            }
            if self.flags & R_PATH != 0 {
                let t = self.path_time.min(18.0);
                self.pos = if t < 9.0 {
                    spline3(&self.path[0..3], t / 9.0)
                } else {
                    spline3(&self.path[1..4], ((t - 9.0) / 9.0).min(1.0))
                };
                self.path_time += 1.0;
                if self.path_time >= 20.0 {
                    self.raise_a8 = true;
                    self.flags |= R_DONE;
                }
            }
        } else if self.flags & R_SHOT == 0 && events & 0x10 != 0 {
            self.fire(w, true, shots, next_id, rand, ts);
            restart(events, requests);
        }
    }

    /// `EftRingShot_UpdateFixed`: event 0x10 puts the record at the opponent's node 3 and raises flag 0xA8.
    fn update_fixed(&mut self, w: &World, events: u32) {
        if self.flags & R_DONE == 0 && events & 0x10 != 0 {
            let p = w.chars[self.src.side ^ 1].obj.node_pos(3);
            self.pos = p;
            self.prev = p;
            self.raise_a8 = true;
            self.flags |= R_DONE;
        }
    }

    /// `EftRingShot_ReapShots`.
    fn reap(&mut self, shots: &Shots) -> bool {
        let mut alive = false;
        for s in &mut self.shots {
            if shots.alive(s.handle) {
                alive = true;
            } else {
                s.handle = None;
            }
        }
        alive
    }

    /// `EftRingShot_PostUpdate`: the shots are reaped; result bit 0 moves the record's volume where the pass put
    /// it, bit 2 starts the end.
    pub(crate) fn post_update(&mut self, w: &World, shots: &Shots) {
        if stopped(w, self.src.side) {
            return;
        }
        self.reap(shots);
        if self.tag.result & 1 != 0 {
            let d = sub(self.tag.pos, self.pos);
            self.pos = self.tag.pos;
            self.prev = add(self.prev, d);
            self.flags |= R_HIT_MOVED;
        } else {
            self.flags &= !R_HIT_MOVED;
        }
        if self.tag.result & 4 != 0 {
            self.flags |= R_COUNT;
        }
    }
}

/// `EftTechEvt_RequestRestart`.
fn restart(events: u32, requests: &mut u32) {
    if events & EV_END == 0 {
        *requests |= 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btl_chr::dot;

    #[test]
    fn the_spline_passes_its_three_points() {
        let p = [[0.0, 0.0, 0.0], [1.0, 2.0, 3.0], [4.0, 0.0, -2.0]];
        assert_eq!(spline3(&p, 0.0), p[0]);
        assert_eq!(spline3(&p, 1.0), p[1]);
        assert_eq!(spline3(&p, 2.0), p[2]);
        let v = rot_z([1.0, 0.0, 0.0], PI_F / 2.0);
        assert!((v[1] - 1.0).abs() < 1e-6 && dot(v, v) > 0.99);
    }
}
