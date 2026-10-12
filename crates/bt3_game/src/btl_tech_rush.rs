//! The catching technique effects of src/battle/eft_obj_tech.c: the thrown object of type 8 (`ObjTech`, below)
//! and the rush shot of type 9.
//!
//! The rush shot, technique effect type 9 (`EftRushShot_*`, the end of src/battle/eft_obj_tech.c): the projectile
//! of a catching technique. The owner's START / FIRE events fire it from the muzzle (node slot 1) along the aim; it
//! flies at the technique's speed (homing by its turn rate, its speed following the set's second width when the
//! set has one) and adds a record each frame while the definition has flag 1. A hit stops its records; the record
//! that catches (reaction 0x22 / 0x1D / 0x1E) starts the rush, and once the owner's rush damage is set up (flag
//! 0xA1, `BtlCharApi_IsRushConnected`) it stops the owner's timeline and sits on the victim's node 3 (technique
//! 0x19D) or the owner's node 0x36 for the sequence. The END event ends it once the set's end frames are over,
//! ABORT at once; its end raises flag 0xA8 for a technique.
//!
//! Its emitters are driven at its head along its direction (`EftRushShot_Draw`; technique 0x19D's once attached
//! keep the parts that run by themselves going). The swarm of models of a definition with flag 0x20000 is not
//! drawn, but its libc `rand()` draws are kept (`EftRushShot_UpdateModels`).
//!
//! Not ported: the swarm's models, the camera cut, the stage tint, blur, streaks and flash, and the sparks' sounds
//! the victim's events start.

use bt3_data::eft_pack::{EmitSet, SetHead};

use crate::btl_blast::{H_LAST, H_NO_LAST, HitRec, Tag, World, home};
use crate::btl_chr::{add, scale3, sub};
use crate::btl_tech_fx::{
    EV_ABORT, EV_END, EV_FIRE, EV_START, EmitNodes, TechSrc, aim_dir, aim_dir_keep, drive_parts,
    emit_state, step_width, stopped, update_trail_width,
};
use crate::eft::{EmitState, Modules};
use crate::rng::LibcRandom;

// EFT_RUSHSHOT_*.
const COUNT: u32 = 0x1;
const END: u32 = 0x2;
const KILL: u32 = 0x4;
const FAST_END: u32 = 0x8;
const ENDED: u32 = 0x10;
const FIRED: u32 = 0x20;
const HIT_MOVED: u32 = 0x40;
const STARTED: u32 = 0x80;
const SCALED: u32 = 0x200;
const ATTACHED: u32 = 0x800;
const ATTACHED2: u32 = 0x1000;
const CONNECTED: u32 = 0x1_0000;
const SUB: u32 = 0x2_0000;
const FLIGHT: u32 = 0x4_0000;

/// `EftRushShot.count`: the swarm's models.
const SWARM: usize = 7;

/// A swarm model as far as its random draws go (`EftRushShotObj`).
#[derive(Clone, Copy, Debug, Default)]
struct Mote {
    /// `model.flags`: 1 placed, 2 released.
    flags: u32,
    delay: f32,
}

/// `rand() / 2147483647.0f` (the draw's value is only kept where it matters).
fn rand01(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// An `EftRushShot` item.
#[derive(Clone, Debug)]
pub(crate) struct RushShot {
    id: u32,
    src: TechSrc,
    pub(crate) tag: Tag,
    flags: u32,
    state: u8,
    /// 1 for technique 0x19D (it sits on the victim).
    kind: i32,
    speed: f32,
    draw_size: f32,
    hit_size: f32,
    timer: f32,
    life: f32,
    dir: [f32; 3],
    start: [f32; 3],
    pos: [f32; 3],
    prev: [f32; 3],
    vel: [f32; 3],
    nodes: EmitNodes,
    emit: Option<EmitState>,
    head_set: SetHead,
    trail_width: f32,
    trail_time: f32,
    width2: f32,
    time2: f32,
    /// The swarm, when the definition has flag 0x20000 (`mgr->sub[0]`).
    swarm: Option<[Mote; SWARM]>,
    dead: bool,
}

impl RushShot {
    /// `EftRushShot_Init` (the swarm's models are not made).
    pub(crate) fn new(
        id: u32,
        src: TechSrc,
        data: &crate::btl_chr::SideData,
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
            kind: i32::from(def.id == 0x19D),
            speed: def.speed,
            draw_size: def.scale,
            hit_size: def.scale,
            timer: 0.0,
            life: f32::from(head_set.end_frames),
            dir: [0.0; 3],
            start: [0.0; 3],
            pos: [0.0; 3],
            prev: [0.0; 3],
            vel: [0.0; 3],
            nodes: EmitNodes::default(),
            emit: emit_state(data, src.slot),
            head_set,
            trail_width: head_set.width[0],
            trail_time: 0.0,
            width2: head_set.width_b[0],
            time2: 0.0,
            swarm: (def.flags & 0x2_0000 != 0).then_some([Mote::default(); SWARM]),
            dead: false,
        };
        if head_set.flags & 4 != 0 {
            s.speed = s.width2 * def.scale;
            s.flags |= SCALED;
        }
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

    /// `EftRushShot_Reset`: its emitters' objects die.
    pub(crate) fn kill_parts(&mut self, fx: &mut Modules) {
        if let Some(st) = &mut self.emit {
            st.kill_all(fx);
        }
    }

    /// `EftRushShot_UpdateModels` of technique 0x19D (mode 1): each model's placement draws from libc `rand()` the
    /// first frame, and again when it follows the released shot after its delay.
    fn update_swarm(&mut self, libc: &mut LibcRandom) {
        let released = self.flags & FLIGHT != 0;
        let Some(swarm) = &mut self.swarm else {
            return;
        };
        for (i, o) in swarm.iter_mut().enumerate() {
            if !released {
                if o.flags & 1 == 0 {
                    // The offset (2), the rotation (2), the spin (4).
                    for _ in 0..8 {
                        libc.rand();
                    }
                    if i & 3 != 0 {
                        libc.rand();
                        o.delay = rand01(libc) * 3.0;
                    } else {
                        o.delay = 0.0;
                    }
                    // The scale, the bob's amplitude, phase and speed, the sinking.
                    for _ in 0..5 {
                        libc.rand();
                    }
                    o.flags |= 1;
                }
            } else if o.delay <= 0.0 {
                if o.flags & 2 == 0 {
                    // The spin's two factors, the bob's amplitude and speed.
                    for _ in 0..4 {
                        libc.rand();
                    }
                    o.flags |= 2;
                }
            } else {
                o.delay -= 1.0;
            }
        }
    }

    /// `EftRushShot_Update`. Returns the record and whether the owner's timeline is to stop
    /// (`EftTechEvt_RequestStop`, as the rush connects).
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        events: u32,
        cur_slot: i32,
    ) -> (Option<HitRec>, bool) {
        let side = self.src.side;
        if stopped(w, side) {
            return (None, false);
        }
        let def = self.src.def;
        let test = |b: u32| events & b != 0;
        let mut stop = false;
        if w.chars[side].flags.test(0xA1) && self.flags & CONNECTED == 0 {
            if let Some((set, _)) = crate::btl_tech_fx::slot_set(w.data[side], self.src.slot)
                && let Some(st) = &mut self.emit
            {
                st.mark_kind6(set);
            }
            stop = true;
            self.flags = (self.flags & !FIRED) | ATTACHED | CONNECTED;
        }
        self.nodes
            .update(&w.chars[side], w.data[side], &def, cur_slot, events);
        if self.flags & STARTED == 0 {
            if test(EV_START) {
                // EftAim_GetDir into the start node's slot (overwritten before it is read).
                let _ = aim_dir(&*w, side, self.nodes.pos(0));
                self.state = 0;
                self.flags |= STARTED;
                if self.swarm.is_some() {
                    self.flags |= SUB;
                }
            } else if test(EV_FIRE) {
                self.state = 0;
                self.flags |= STARTED;
            }
        }
        let mut advance = false;
        if self.flags & STARTED != 0 {
            match self.state {
                0 => {
                    if test(EV_FIRE) {
                        self.flags |= FIRED;
                        self.start = self.nodes.pos(1);
                        self.pos = self.start;
                        self.prev = w.chars[side].obj.node_pos(0x11);
                        self.dir = aim_dir_keep(&*w, side, self.pos);
                        self.vel = scale3(self.dir, self.speed);
                        self.state = 1;
                        self.flags |= FLIGHT;
                    }
                }
                2 => {
                    if self.flags & HIT_MOVED == 0 {
                        self.prev = self.pos;
                    }
                    if def.flags & 2 == 0 {
                        if self.flags & HIT_MOVED == 0 {
                            if def.homing > 0.0 {
                                self.dir = home(
                                    &*w.chars,
                                    self.pos,
                                    &mut self.dir,
                                    side,
                                    self.speed,
                                    def.homing,
                                );
                                self.vel = scale3(self.dir, self.speed);
                            }
                            self.pos = add(self.pos, self.vel);
                        } else {
                            self.pos = self.tag.pos;
                        }
                    }
                    if test(EV_END) {
                        self.flags |= ENDED | END | COUNT;
                        self.state = 3;
                    }
                }
                1 | 3 => advance = true,
                _ => {}
            }
        }
        // EftRushShot_UpdateAttached: the swarm, then the node it sits on (the victim's events' blur, streaks,
        // flash and sparks are not ported).
        if self.kind == 1 {
            if self.flags & SUB != 0 {
                self.update_swarm(w.libc);
            }
            if self.flags & ATTACHED != 0 {
                self.pos = w.chars[side ^ 1].obj.node_pos(3);
                self.dir = aim_dir(&*w, side, self.nodes.pos(1));
            }
        } else if self.flags & ATTACHED != 0 {
            self.pos = w.chars[side].obj.node_pos(0x36);
            self.dir = aim_dir(&*w, side, self.nodes.pos(1));
        }
        if test(EV_ABORT) {
            if self.flags & ENDED == 0 {
                self.flags |= FAST_END;
            }
            self.flags |= END;
        }
        self.draw(w, fx, events);
        if self.flags & FIRED != 0 && self.flags & SCALED != 0 {
            let h = self.head_set;
            step_width(
                &mut self.width2,
                &mut self.time2,
                h.width_b,
                h.width_b_time,
                h.width_b_split,
            );
            self.speed = self.width2 * def.scale;
            self.vel = scale3(self.dir, self.speed);
        }
        if advance {
            self.state += 1;
        }
        if self.flags & COUNT != 0 {
            self.timer += 1.0;
        }
        if self.flags & KILL != 0 {
            self.dead = true;
            return (None, stop);
        }
        if self.flags & END != 0 {
            if self.flags & FAST_END != 0 || self.timer >= self.life {
                self.flags |= KILL;
            }
            return (None, stop);
        }
        if def.flags & 1 == 0 || self.flags & FIRED == 0 {
            return (None, stop);
        }
        update_trail_width(&mut self.trail_width, &mut self.trail_time, &self.head_set);
        if w.paused {
            return (None, stop);
        }
        // EftRushShot_AddHitRecord: its body, capsules from the muzzle for shape 1.
        let rec = HitRec::technique(
            self.src,
            self.id,
            self.pos,
            self.prev,
            self.vel,
            self.nodes.pos(1),
            self.hit_size * self.trail_width,
        )
        .map(|mut r| {
            r.origin = self.start;
            r
        });
        (rec, stop)
    }

    /// `EftRushShot_Draw`: every emitter from the owner's events at the head, along the direction. Technique
    /// 0x19D ends its parts only by the life timer until it is attached; then (from the second frame) by the end,
    /// except the parts that stop by themselves (no end phase, a running time).
    fn draw(&mut self, w: &mut World, fx: &mut Modules, events: u32) {
        let Some(st) = &mut self.emit else {
            return;
        };
        let mut fast = self.flags & FAST_END != 0;
        let mut end = self.flags & END != 0;
        let mut alt = false;
        if self.kind == 1 {
            if self.flags & ATTACHED == 0 {
                end = self.flags & COUNT != 0;
            } else if self.flags & ATTACHED2 == 0 {
                end = self.flags & COUNT != 0;
                self.flags |= ATTACHED2;
            } else {
                alt = true;
            }
        }
        let end_alt = end;
        drive_parts(
            fx,
            w,
            &self.src,
            st,
            &self.nodes,
            self.pos,
            self.dir,
            self.draw_size,
            |_, _, _| false,
            |st, set, kind, n| {
                // The C's `fast`, once cleared by such a part, stays cleared for the parts after it.
                if alt {
                    end = end_alt;
                    let p = &set.parts[n];
                    if p.end_phase() == 0 && p.rate() > 0.0 {
                        end = false;
                        fast = false;
                    }
                }
                st.flags_from_req(set, kind, n, end, fast, events)
            },
        );
    }

    /// `EftRushShot_PostUpdate`: the record moved by the pass (result bit 0) moves the head and the tail; a hit
    /// stops the records, a guard or the stage ends it; result bit 2 starts its life timer.
    pub(crate) fn post_update(&mut self, w: &World, fx: &mut Modules) {
        if stopped(w, self.src.side) {
            return;
        }
        if let Some(st) = &mut self.emit {
            crate::btl_tech_fx::parts_alive(fx, w.data[self.src.side], &self.src, st, false);
        }
        if self.tag.result & 1 != 0 {
            let d = sub(self.tag.pos, self.pos);
            self.pos = self.tag.pos;
            self.prev = add(self.prev, d);
            self.flags |= HIT_MOVED;
        }
        let t = self.tag.flags;
        if t & 1 != 0 {
            if t & 2 != 0 {
                self.flags = (self.flags | END) & !FIRED;
            } else {
                self.flags &= !FIRED;
            }
        } else if t & 2 != 0 || t & 4 != 0 {
            self.flags = (self.flags | END) & !FIRED;
        }
        if self.tag.result & 4 != 0 {
            self.flags = (self.flags | COUNT) & !FIRED;
        }
    }
}

/// The two hit shapes of a thrown object (`EftKShape`): its flight, and where it is held or pinned.
#[derive(Clone, Copy, Debug, Default)]
struct Body {
    start: [f32; 3],
    pos: [f32; 3],
    prev: [f32; 3],
    vel: [f32; 3],
}

/// An `EftObjTech` item (technique effect type 8, the thrown object; `EftObjTech_*`, src/battle/eft_obj_tech.c):
/// fired from the muzzle on FIRE, it flies like a blast (homing, the set's second width as its speed) with a
/// record each frame until it hits. A fighter hit stops the owner's timeline and pins it on the victim; from then
/// on its record is the held shape marked as the last hit (flag 0x10: the technique's second reaction), while the
/// flight's record of mode 0 is never the last hit (0x800). By technique: 0x282 / 0x1F3 (mode 0) take it in hand
/// on event 0x10 at node slot 3's node and throw it again on 0x20; 0x2A8 (1) sits on the victim's node 3; 0x162 (2)
/// follows the hand's node 0x1F from FIRE; 0x167 / 0x168 (3) sit on the victim's node 0x11 with the rings. END
/// ends it once the set's end frames are over, ABORT at once; its end raises flag 0xA8 for a technique. Its models
/// and rings are not ported.
#[derive(Clone, Debug)]
pub(crate) struct ObjTech {
    id: u32,
    src: TechSrc,
    pub(crate) tag: Tag,
    flags: u32,
    state: u8,
    mode: i32,
    speed: f32,
    scale: f32,
    hit_scale: f32,
    timer: f32,
    life: f32,
    dir: [f32; 3],
    body: Body,
    held: Body,
    nodes: EmitNodes,
    emit: Option<EmitState>,
    head_set: SetHead,
    trail_width: f32,
    trail_time: f32,
    width2: f32,
    time2: f32,
    dead: bool,
}

impl ObjTech {
    /// `EftObjTech_Init` (the model and the rings are not made).
    pub(crate) fn new(
        id: u32,
        src: TechSrc,
        data: &crate::btl_chr::SideData,
        set: &EmitSet,
    ) -> Self {
        let def = src.def;
        let head_set = set.head;
        let mode = match def.id {
            0x2A8 => 1,
            0x162 => 2,
            0x167 | 0x168 => 3,
            _ => 0,
        };
        let mut o = Self {
            id,
            src,
            tag: Tag::default(),
            flags: if def.id == 0x167 { 0x8000 } else { 0 },
            state: 0,
            mode,
            speed: def.speed,
            scale: def.scale,
            hit_scale: def.scale,
            timer: 0.0,
            life: f32::from(head_set.end_frames),
            dir: [0.0; 3],
            body: Body::default(),
            held: Body::default(),
            nodes: EmitNodes::default(),
            emit: emit_state(data, src.slot),
            head_set,
            trail_width: head_set.width[0],
            trail_time: 0.0,
            width2: head_set.width_b[0],
            time2: 0.0,
            dead: false,
        };
        if head_set.flags & 4 != 0 {
            o.speed = o.width2 * def.scale;
            o.flags |= 0x200;
        }
        o
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

    /// `EftObjTech_Reset`: its emitters' objects die.
    pub(crate) fn kill_parts(&mut self, fx: &mut Modules) {
        if let Some(st) = &mut self.emit {
            st.kill_all(fx);
        }
    }

    /// `EftObjTech_UpdateMotion`: where the object is held or pinned, by mode (the poses of the model and the
    /// rings are not ported).
    fn update_motion(&mut self, w: &World, events: u32) {
        let side = self.src.side;
        let test = |b: u32| events & b != 0;
        let me = &w.chars[side].obj;
        let opp = &w.chars[side ^ 1].obj;
        match self.mode {
            0 => {
                let node = self.nodes.node(3) as u32;
                if test(0x10) {
                    self.held.start = me.node_pos(node);
                    self.held.pos = self.held.start;
                    self.dir = aim_dir(w, side, self.held.pos);
                    self.held.vel = scale3(self.dir, self.speed);
                    self.flags |= 0x1000;
                }
                if test(0x20) {
                    self.speed = self.src.def.speed;
                    self.dir = aim_dir(w, side, self.held.pos);
                    self.held.vel = scale3(self.dir, self.speed);
                    self.flags |= 0x20;
                    self.tag.flags &= !0x205;
                    self.flags |= 0x2000;
                }
                self.held.prev = self.held.pos;
                if self.flags & 0x2000 != 0 {
                    self.held.pos = add(self.held.pos, self.held.vel);
                } else if self.flags & 0x1000 != 0 {
                    self.held.pos = me.node_pos(node);
                }
            }
            1 => {
                if self.flags & 0x400 != 0 {
                    self.held.pos = opp.node_pos(3);
                }
            }
            2 => {
                if test(EV_FIRE) {
                    self.flags |= 0x4000;
                }
                if self.flags & 0x4000 != 0 {
                    if self.flags & 0x400 == 0 {
                        self.body.pos = me.node_pos(0x1F);
                    } else {
                        self.held.pos = opp.node_pos(0x1F);
                    }
                }
            }
            3 => {
                if self.flags & 0x400 != 0 {
                    self.held.pos = opp.node_pos(0x11);
                    self.dir = [0.0, -1.0, 0.0];
                }
                if test(EV_START) && self.flags & 0x8000 != 0 {
                    self.flags |= 0x4000;
                }
                if test(EV_FIRE) {
                    self.flags |= 0x1_4000;
                }
                if test(0x10) {
                    self.flags |= 0x2_0000;
                }
            }
            _ => {}
        }
    }

    /// `EftObjTech_Update`. Returns the record.
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
        if self.flags & 0x80 == 0 {
            if test(EV_START) {
                self.dir = aim_dir(&*w, side, self.nodes.pos(0));
                self.state = 0;
                self.flags |= 0x80;
            } else if test(EV_FIRE) {
                self.state = 0;
                self.flags |= 0x80;
            }
        }
        let mut advance = false;
        if self.flags & 0x80 != 0 {
            match self.state {
                0 => {
                    if test(EV_FIRE) {
                        self.flags |= 0x20;
                        self.body.start = self.nodes.pos(1);
                        self.body.pos = self.body.start;
                        self.body.prev = w.chars[side].obj.node_pos(0x11);
                        self.dir = aim_dir_keep(&*w, side, self.body.pos);
                        self.body.vel = scale3(self.dir, self.speed);
                        self.state = 1;
                    }
                }
                2 => {
                    if self.flags & 0x400 == 0 {
                        if self.flags & 0x40 == 0 {
                            self.body.prev = self.body.pos;
                        }
                        if def.flags & 2 == 0 {
                            if self.flags & 0x40 == 0 {
                                if def.homing > 0.0 {
                                    self.dir = home(
                                        &*w.chars,
                                        self.body.pos,
                                        &mut self.dir,
                                        side,
                                        self.speed,
                                        def.homing,
                                    );
                                    self.body.vel = scale3(self.dir, self.speed);
                                }
                                self.body.pos = add(self.body.pos, self.body.vel);
                            } else {
                                self.body.pos = self.tag.pos;
                            }
                        }
                    }
                    if test(EV_END) {
                        self.flags |= 0x13;
                        self.state = 3;
                    }
                }
                1 | 3 => advance = true,
                _ => {}
            }
        }
        self.update_motion(&*w, events);
        if test(EV_ABORT) {
            if self.flags & 0x10 == 0 {
                self.flags |= 8;
            }
            self.flags |= 2;
        }
        self.update_emitters(w, fx, events);
        if self.flags & 0x20 != 0 && self.flags & 0x200 != 0 {
            let h = self.head_set;
            step_width(
                &mut self.width2,
                &mut self.time2,
                h.width_b,
                h.width_b_time,
                h.width_b_split,
            );
            self.speed = self.width2 * def.scale;
            self.body.vel = scale3(self.dir, self.speed);
        }
        if advance {
            self.state += 1;
        }
        if self.flags & 1 != 0 {
            self.timer += 1.0;
        }
        if self.flags & 4 != 0 {
            self.dead = true;
            return None;
        }
        if self.flags & 2 != 0 {
            if self.flags & 8 != 0 || self.timer >= self.life {
                self.flags |= 4;
            }
            return None;
        }
        if def.flags & 1 == 0 || self.flags & 0x20 == 0 {
            return None;
        }
        update_trail_width(&mut self.trail_width, &mut self.trail_time, &self.head_set);
        if w.paused {
            return None;
        }
        // EftObjTech_AddHitRecord: the flight, or once pinned the held shape as the last hit.
        let (shape, flag) = if self.flags & 0x800 == 0 {
            let flag = if self.mode == 0 { H_NO_LAST } else { 0 };
            (self.body, flag)
        } else {
            (self.held, H_LAST)
        };
        let mut rec = HitRec::technique(
            self.src,
            self.id,
            shape.pos,
            shape.prev,
            shape.vel,
            self.nodes.pos(1),
            self.hit_scale * self.trail_width,
        )?;
        rec.origin = shape.start;
        rec.flags |= flag;
        Some(rec)
    }

    /// `EftObjTech_UpdateEmitters`: every emitter from the owner's events at the flight's head; once pinned (from
    /// the frame after) at the held position, the parts that stop by themselves left to run.
    fn update_emitters(&mut self, w: &mut World, fx: &mut Modules, events: u32) {
        let Some(st) = &mut self.emit else {
            return;
        };
        let mut pos = self.body.pos;
        let mut fast = self.flags & 8 != 0;
        let mut end;
        let mut pinned = false;
        if self.flags & 0x400 == 0 {
            end = self.flags & 1 != 0;
        } else if self.flags & 0x800 == 0 {
            end = self.flags & 1 != 0;
            self.flags |= 0x800;
        } else {
            pos = self.held.pos;
            end = self.flags & 2 != 0;
            pinned = true;
        }
        let end_pinned = end;
        drive_parts(
            fx,
            w,
            &self.src,
            st,
            &self.nodes,
            pos,
            self.dir,
            self.scale,
            |_, _, _| false,
            |st, set, kind, n| {
                // As in the original, a part that clears `fast` clears it for the parts after it.
                if pinned {
                    end = end_pinned;
                    let p = &set.parts[n];
                    if p.end_phase() == 0 && p.rate() > 0.0 {
                        end = false;
                        fast = false;
                    }
                }
                st.flags_from_req(set, kind, n, end, fast, events)
            },
        );
    }

    /// `EftObjTech_PostUpdate`: the record moved by the pass moves the head; a fighter hit pins it on the victim
    /// (flag 0x400, the phase 6 emitters started) and stops the owner's timeline, a guard or the stage ends it;
    /// result bit 2 starts its life timer. Returns whether the timeline is to stop.
    pub(crate) fn post_update(&mut self, w: &World, fx: &mut Modules) -> bool {
        if stopped(w, self.src.side) {
            return false;
        }
        let mut stop = false;
        if let Some(st) = &mut self.emit {
            crate::btl_tech_fx::parts_alive(fx, w.data[self.src.side], &self.src, st, false);
        }
        if self.tag.result & 1 != 0 {
            let d = sub(self.tag.pos, self.body.pos);
            self.body.pos = self.tag.pos;
            self.body.prev = add(self.body.prev, d);
            self.flags |= 0x40;
        }
        let t = self.tag.flags;
        if t & 1 != 0 {
            if t & 2 == 0 {
                stop = true;
                self.flags = (self.flags & !0x20) | 0x400;
                if let Some((set, _)) =
                    crate::btl_tech_fx::slot_set(w.data[self.src.side], self.src.slot)
                    && let Some(st) = &mut self.emit
                {
                    st.mark_kind6(set);
                }
            } else {
                self.flags = (self.flags | 2) & !0x20;
            }
        } else if t & 2 != 0 || t & 4 != 0 {
            self.flags = (self.flags | 2) & !0x20;
        }
        if self.tag.result & 4 != 0 {
            self.flags = (self.flags | 1) & !0x20;
        }
        stop
    }
}
