//! The shots, technique effect type 2 (`EftShotTech_*`, src/battle/eft_shot_tech.c and src/battle/eft_obj_tech.c):
//! the effect of the rush techniques. It makes no hit record: on the owner's START / FIRE events (or event 0x10 for
//! a set with option 4) it takes a target point at the event's node and drives its emitters there; with set flag 1
//! the point moves (straight on or onto the opponent, in front of the opponent, or past it). A set with option 0x20
//! fires blast objects (`EftBlastObj`, without records) on the owner's events of the phases it has emitters for,
//! each stopped after the technique's shot life. Once the owner's rush connects (flag 0xA1) the point may sit on
//! the owner's node 0x36 and aim again, and option 1 takes the muzzle (node 0x36) on event 0x20 for the phase 4
//! emitters and shots. The END event, or ABORT, ends it once its shots are gone; its end raises flag 0xA8.
//!
//! Not ported: its model, the camera cut, and what the connected rush starts on the opponent's events (screen
//! flash, node sounds, stage blur, streaks, the aura orb and glow, the ground dust).

use bt3_data::eft_pack::SetHead;

use crate::btl_blast::World;
use crate::btl_chr::{add, scale3, sub};
use crate::btl_tech_fx::{
    EV_ABORT, EV_END, EV_FIRE, EV_START, EmitNodes, TechSrc, aim_dir, attr_kind, drive_parts_at,
    emit_state, parts_alive, stopped,
};
use crate::btl_tech_volley::Shots;
use crate::eft::{EmitState, Modules, is_part_deferred};

/// `EftJShotSlot`s.
const SLOTS: usize = 14;

/// A blast object the item fired (`EftJShotSlot`): its handle, flag 8 stopped, its age.
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    handle: Option<u32>,
    flags: u32,
    timer: f32,
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    crate::btl_blast::norm(v)
}

/// An `EftShotTech` item.
#[derive(Clone, Debug)]
pub(crate) struct ShotTech {
    src: TechSrc,
    flags: u32,
    state: u8,
    phase: u8,
    node: i32,
    speed: f32,
    scale: f32,
    dir: [f32; 3],
    start: [f32; 3],
    pos: [f32; 3],
    prev: [f32; 3],
    vel: [f32; 3],
    shot_dir: [f32; 3],
    shot_count: i32,
    muzzle_pos: [f32; 3],
    muzzle_dir: [f32; 3],
    shots: [Slot; SLOTS],
    /// This frame's opponent events and owner's technique slot (`EftShot_TestBits` of the opponent,
    /// `EftShot_GetCurSlot`).
    opp_events: u32,
    cur_slot: i32,
    nodes: EmitNodes,
    emit: Option<EmitState>,
    head_set: SetHead,
    dead: bool,
}

impl ShotTech {
    /// `EftShotTech_Init` (the model is not made).
    pub(crate) fn new(src: TechSrc, data: &crate::btl_chr::SideData, head_set: SetHead) -> Self {
        let mut s = Self {
            src,
            flags: 0,
            state: 0,
            phase: 0,
            node: 0,
            speed: src.def.item_speed,
            scale: src.def.scale,
            dir: [0.0; 3],
            start: [0.0; 3],
            pos: [0.0; 3],
            prev: [0.0; 3],
            vel: [0.0; 3],
            shot_dir: [0.0; 3],
            shot_count: 0,
            muzzle_pos: [0.0; 3],
            muzzle_dir: [0.0; 3],
            shots: [Slot::default(); SLOTS],
            opp_events: 0,
            cur_slot: -1,
            nodes: EmitNodes::default(),
            emit: emit_state(data, src.slot),
            head_set,
            dead: false,
        };
        if head_set.flags & 1 != 0 && head_set.move_kind == 2 {
            s.node = 0x15;
            s.flags |= 0x2000;
        }
        if head_set.options & 0x20 != 0 {
            s.flags |= 0x2_0000;
        }
        s
    }

    pub(crate) fn dead(&self) -> bool {
        self.dead
    }

    /// `EftShotTech_CalcDir`: the direction by the set's aim kind, and the point's velocity along it.
    fn calc_dir(&mut self, w: &World) -> [f32; 3] {
        let side = self.src.side;
        let me = &w.chars[side].obj;
        let dir = match self.head_set.dir_kind {
            0 => norm(sub(w.chars[side ^ 1].obj.node_pos(3), self.pos)),
            1 => norm(sub(me.node_pos(3), me.node_pos(0x2E))),
            2 => aim_dir(w, side, self.pos),
            3 => norm(sub(me.node_pos(0x23), me.node_pos(0x22))),
            4 => norm(sub(me.node_pos(0x15), me.node_pos(0x14))),
            5 => muzzle(w, side).1,
            _ => [0.0; 3],
        };
        self.vel = scale3(dir, self.speed);
        dir
    }

    /// `EftShotTech_Start`: the point at the event's node (node 0x15 for move kind 2, which event 0x10 also moves on).
    fn start(&mut self, w: &World, events: u32, evt: u32) {
        let side = self.src.side;
        let chr = &w.chars[side];
        if self.flags & 0x2000 != 0 {
            if events & 0x10 != 0 {
                self.flags |= 0x4000;
            }
            self.node = 0x15;
        } else {
            self.node = attr_kind(chr, w.data[side], self.cur_slot, evt);
        }
        self.start = chr.obj.node_pos(self.node as u32);
        self.pos = self.start;
        self.prev = self.pos;
        self.dir = self.calc_dir(w);
    }

    /// `EftShotTech_MoveStraight` / `MoveFromTarget` / `MoveAimed`.
    fn move_point(&mut self, w: &World, events: u32) {
        let side = self.src.side;
        let me = &w.chars[side].obj;
        let opp = &w.chars[side ^ 1].obj;
        let opp_events = self.opp_events;
        let options = self.head_set.options;
        match self.head_set.move_kind {
            0 => {
                if self.phase == 0 {
                    self.start = me.node_pos(self.node as u32);
                    self.prev = self.pos;
                    if options & 2 == 0 {
                        self.pos = add(self.pos, self.vel);
                    } else {
                        self.pos = opp.node_pos(3);
                    }
                    if opp_events & 0x40 != 0 {
                        self.phase = 1;
                    }
                }
            }
            1 => {
                let gap = self.head_set.gap;
                let in_front = || {
                    let a = opp.node_pos(0x11);
                    let d = scale3(norm(sub(me.node_pos(0x11), a)), gap);
                    add(a, d)
                };
                match self.phase {
                    0 => {
                        self.start = me.node_pos(self.node as u32);
                        self.prev = self.pos;
                        self.pos = add(self.pos, self.vel);
                        if options & 8 == 0 || events & 0x10 != 0 {
                            self.pos = in_front();
                            self.phase = 1;
                        }
                    }
                    1 => {
                        self.pos = in_front();
                        if events & 0x20 != 0 {
                            self.flags |= 0x100;
                        }
                        if self.flags & 0x100 != 0 {
                            self.scale = (self.scale + 0.05).min(2.0);
                        }
                        if opp_events & 0x40 != 0 {
                            self.phase = 2;
                        }
                    }
                    _ => {}
                }
            }
            2 => {
                if self.phase == 0 {
                    if events & 0x10 != 0 {
                        self.flags |= 0x4000;
                    }
                    self.start = me.node_pos(self.node as u32);
                    self.prev = self.pos;
                    let d;
                    if self.flags & 0x4000 == 0 {
                        let to = opp.node_pos(3);
                        let v = sub(to, self.start);
                        let dist = crate::btl_chr::length(v) + 30.0;
                        d = norm(v);
                        self.pos = add(self.start, scale3(d, dist));
                    } else {
                        let dist = if self.src.def.id == 0x2B4 {
                            30.0 * 0.6
                        } else {
                            30.0
                        };
                        d = norm(sub(me.node_pos(0x1F), self.start));
                        self.pos = add(self.start, scale3(d, dist));
                    }
                    self.dir = d;
                }
            }
            _ => {}
        }
    }

    /// `EftShotTech_FireShots`: one blast object (the burst of the FIRE event for kind 1) at the event's node, or
    /// at the muzzle for a phase 4 shot with option 1 (held still there); without records.
    #[allow(clippy::too_many_arguments)]
    fn fire_shots(
        &mut self,
        w: &World,
        shots: &mut Shots,
        next_id: &mut u32,
        evt: u32,
        kind: u8,
        node_slot: u8,
        scale: f32,
        speed: f32,
    ) {
        let side = self.src.side;
        let def = self.src.def;
        let mut n = 1;
        if kind == 1 {
            n = def.volley;
            if n <= 0 {
                n = 1;
            }
        }
        for _ in 0..n {
            let Some(k) = self.shots.iter().position(|s| s.handle.is_none()) else {
                continue;
            };
            self.shots[k] = Slot::default();
            let alt = self.head_set.options & 1 != 0 && kind == 4;
            let chr = &w.chars[side];
            let node = attr_kind(chr, w.data[side], self.cur_slot, evt);
            let (pos, dir) = if alt {
                (self.muzzle_pos, self.muzzle_dir)
            } else {
                (chr.obj.node_pos(node as u32), self.shot_dir)
            };
            let handle = shots.create(
                next_id,
                self.src,
                w,
                &self.head_set,
                &self.nodes,
                (kind, node_slot),
                pos,
                dir,
                0.0,
                scale,
                speed,
            );
            self.shots[k].handle = handle;
            self.shots[k].flags |= 1;
            shots.set_no_hit(handle, true);
            if alt {
                shots.set_frozen(handle, true);
            }
            self.flags |= 0x4_0000;
            self.shot_count += 1;
        }
    }

    /// `EftShotTech_ReapShots`: the slots of the blast objects that are gone are freed; whether any is left.
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

    /// `EftShotTech_UpdateShots`: fires on the owner's events of the phases the set has emitters for, ages the
    /// blast objects (placed on the point with option 0x10) and stops them at the shot life or the item's end;
    /// whether any is left.
    fn update_shots(
        &mut self,
        w: &World,
        shots: &mut Shots,
        next_id: &mut u32,
        events: u32,
    ) -> bool {
        let test = |b: u32| events & b != 0;
        let def = self.src.def;
        let (mut scale, mut speed) = (self.scale, self.speed);
        let fire = if test(EV_START) {
            Some((2, 0, 0))
        } else if test(EV_FIRE) {
            if test(0x40) {
                scale *= 1.2;
                speed *= 0.8;
            }
            Some((4, 1, 1))
        } else if test(0x10) {
            Some((0x10, 3, 2))
        } else if test(0x20) {
            Some((0x20, 4, 3))
        } else if test(0x40) {
            Some((0x40, 5, 4))
        } else {
            None
        };
        if let Some((evt, kind, node_slot)) = fire {
            self.shot_dir = self.calc_dir(w);
            if (self.head_set.phase_mask >> kind) & 1 != 0 {
                self.fire_shots(w, shots, next_id, evt, kind, node_slot, scale, speed);
            }
        }
        for i in 0..SLOTS {
            let h = self.shots[i].handle;
            if !shots.alive(h) {
                continue;
            }
            let s = &mut self.shots[i];
            if s.flags & 8 == 0 {
                if self.head_set.options & 0x10 != 0 {
                    shots.set_target(h, Some(self.pos));
                }
                if s.timer >= def.shot_life {
                    s.flags |= 8;
                    shots.stop(h);
                } else {
                    s.timer += 1.0;
                }
            }
            if self.flags & 1 != 0 {
                shots.stop(h);
            }
        }
        self.reap(shots)
    }

    /// `EftShotTech_Update`.
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        shots: &mut Shots,
        next_id: &mut u32,
        events: u32,
        opp_events: u32,
        cur_slot: i32,
    ) {
        let side = self.src.side;
        self.opp_events = opp_events;
        self.cur_slot = cur_slot;
        if stopped(w, side) {
            return;
        }
        let test = |b: u32| events & b != 0;
        if w.chars[side].flags.test(0xA1) {
            self.flags |= 0x800;
        }
        let options = self.head_set.options;
        let def = self.src.def;
        self.nodes
            .update(&w.chars[side], w.data[side], &def, cur_slot, events);
        if self.flags & 0x1000 == 0 {
            if test(EV_START) {
                self.start(w, events, EV_START);
                self.state = 0;
                self.flags |= 0x1000;
            } else if test(EV_FIRE) {
                self.start(w, events, EV_FIRE);
                self.state = 2;
                self.flags |= 0x1000;
            } else if options & 4 != 0 && test(0x10) {
                self.start(w, events, 0x10);
                self.state = 2;
                self.flags |= 0x1000;
            }
        }
        let mut advance = false;
        match self.state {
            0 => {
                if test(EV_FIRE) {
                    self.start(w, events, EV_FIRE);
                    self.state = 1;
                }
            }
            1 | 3 => advance = true,
            2 => {
                if self.head_set.flags & 1 != 0 {
                    self.move_point(w, events);
                }
                if test(EV_END) {
                    self.flags |= 9;
                    self.state = 3;
                }
            }
            _ => {}
        }
        if self.flags & 0x800 != 0 {
            if options & 0x10 != 0 {
                self.pos = w.chars[side].obj.node_pos(0x36);
            }
            if options & 0x40 != 0 {
                self.dir = self.calc_dir(w);
            }
            // EftShotTech_UpdateConnected: of its helpers only the muzzle is ported (the others draw).
            if options & 1 != 0 && test(0x20) {
                (self.muzzle_pos, self.muzzle_dir) = muzzle(w, side);
            }
        }
        let mut busy = false;
        if self.flags & 0x2_0000 != 0 {
            busy = self.update_shots(w, shots, next_id, events);
        }
        if test(EV_ABORT) {
            if self.flags & 8 == 0 {
                self.flags |= 4;
            }
            self.flags |= 1;
        }
        self.update_parts(w, fx, events);
        if advance {
            self.state += 1;
        }
        if !busy && self.flags & 2 != 0 {
            self.dead = true;
            return;
        }
        if self.flags & 1 != 0 {
            self.flags |= 2;
        }
    }

    /// `EftShotTech_UpdateParts`: every emitter from the owner's events (but those the shots drive, with option
    /// 0x20) at the point, the phase 4 ones at the muzzle with option 1.
    fn update_parts(&mut self, w: &mut World, fx: &mut Modules, events: u32) {
        let Some(st) = &mut self.emit else {
            return;
        };
        let deferred = self.flags & 0x2_0000 != 0;
        let (ending, kill) = (self.flags & 1 != 0, self.flags & 4 != 0);
        let muzzle = self.head_set.options & 1 != 0;
        let (pos, dir, mpos, mdir) = (self.pos, self.dir, self.muzzle_pos, self.muzzle_dir);
        drive_parts_at(
            fx,
            w,
            &self.src,
            st,
            &self.nodes,
            self.scale,
            |set, kind, sub| deferred && is_part_deferred(set, kind, sub),
            |st, set, kind, n| st.flags_from_req(set, kind, n, ending, kill, events),
            |set, n| {
                if muzzle && set.parts[n].phase() == 4 {
                    (mpos, mdir)
                } else {
                    (pos, dir)
                }
            },
        );
    }

    /// `EftShotTech_PostUpdate`.
    pub(crate) fn post_update(&mut self, w: &World, fx: &mut Modules, shots: &Shots) {
        if stopped(w, self.src.side) {
            return;
        }
        if let Some(st) = &mut self.emit {
            parts_alive(fx, w.data[self.src.side], &self.src, st, false);
        }
        self.reap(shots);
    }
}

/// `EftShotTech_UpdateMuzzle` / aim kind 5: node 0x36's position and its -z axis.
fn muzzle(w: &World, side: usize) -> ([f32; 3], [f32; 3]) {
    let obj = &w.chars[side].obj;
    match obj.node(0x36) {
        Some(n) => {
            let m = n.mtx;
            (
                [m[3][0], m[3][1], m[3][2]],
                norm([-m[2][0], -m[2][1], -m[2][2]]),
            )
        }
        None => (obj.pos, [0.0, 0.0, -1.0]),
    }
}
