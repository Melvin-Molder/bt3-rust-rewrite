//! The fighter action state machine (src/battle/btl_char_action.c, the handlers of btl_act_1.c / btl_act_2.c and
//! the decision tables of btl_act_change.c / btl_act_decide.c).
//!
//! An action is a number 0..0x13B with a handler called in four phases: enter (once, when it becomes current),
//! run (every frame), decide (every frame, before the forced-action check; it requests the next action) and leave.
//! `BtlAct_Update` (stage 4 / 5 of the frame) runs decide, the forced check, the switch, then run, the gauges and
//! the timers.
//!
//! Ported so far: the entrance pose (1), neutral (0xB, 0xC), walking (0xD, 0xE), falling (0x11 / 0x13), the turn
//! (0x3A), one rush step (0x44), the hit reactions that stay in place (0xBD, 0xC0..0xCA), the push-backs
//! (0xCF..0xD1), the knock-out fall (0xD2), stun (0xD3, 0xE0), lying down (0xD8), the helpless fall (0xD9), the
//! ground hits (0xDA..0xDE), getting up (0xE1..0xE6). An input or reaction that asks for any other action is
//! recognised as the original recognises it, but the action is not queued (`ported`).

use std::f32::consts::PI;

use crate::btl_chr::{
    Chr, Env, SideData, anim_flag, begin_rise_to_opponent, deg, is_blocked_by_opponent, kmh,
    opp_yaw_from_facing, scale_speed_by_approach,
};
use crate::btl_input::{DIR_MASK, wrap_angle};

const ENTER: u32 = 0;
const RUN: u32 = 1;
const DECIDE: u32 = 2;
const LEAVE: u32 = 3;

/// What a handler sees: the fighter, its opponent, both sides' data and the battle.
pub(crate) struct Ctx<'a, 'd> {
    pub me: &'a mut Chr,
    pub opp: &'a mut Chr,
    pub data: &'a SideData<'d>,
    pub opp_data: &'a SideData<'d>,
    pub env: &'a Env<'d>,
    /// `gBtlChars->frame` (`BtlChar_FrameMod`).
    pub frame: u32,
    /// `BtlChars_IsTimeStopped`.
    pub time_stopped: bool,
}

/// Whether action `id` has a handler here.
pub(crate) fn ported(id: i32) -> bool {
    matches!(id,
        1..=3 | 0xB..=0x18 | 0x37 | 0x38 | 0x3A | 0x44
        | 0xBD | 0xC0..=0xCA | 0xCF..=0xD3 | 0xD8 | 0xD9 | 0xDA..=0xDE | 0xE0..=0xE6)
}

impl Ctx<'_, '_> {
    // ---- bookkeeping (btl_char_action.c) ------------------------------------------------------------------------

    /// `BtlAct_Request` (-1 is ignored).
    fn request(&mut self, id: i32) {
        if id != -1 && ported(id) {
            self.me.act.request = id;
        }
    }

    /// `BtlAct_SetQueue`: the slot, and the slots after it emptied.
    fn set_queue(&mut self, slot: usize, id: i32) {
        if slot < 4 && id != -1 && ported(id) {
            self.me.act.queue[slot] = id;
            for q in &mut self.me.act.queue[slot + 1..] {
                *q = -1;
            }
        }
    }

    fn queued(&self) -> i32 {
        self.me.act.queue[0]
    }

    fn has_queued(&self) -> bool {
        self.me.act.queue[0] != -1
    }

    fn pop_queue(&mut self) {
        let q = &mut self.me.act.queue;
        q.copy_within(1..4, 0);
        q[3] = -1;
    }

    fn current(&self) -> i32 {
        self.me.act.current
    }

    fn requested(&self) -> i32 {
        self.me.act.request
    }

    fn test(&mut self, id: u32) -> bool {
        self.me.test(id, true)
    }

    fn flag(&self, n: u32) -> bool {
        self.me.flags.test(n)
    }

    fn set(&mut self, n: u32) {
        self.me.flags.set(n);
    }

    fn held(&mut self, n: u32) {
        self.me.flags.set_held(n);
    }

    fn clear(&mut self, n: u32) {
        self.me.flags.clear(n);
    }

    fn frame_mod(&self, n: u32) -> u32 {
        self.frame % n.max(1)
    }

    /// `BtlOpp_GetSeenAction`: the opponent's action as this frame sees it.
    fn opp_seen_action(&self) -> i32 {
        if self.me.flags.stage() < 6 || self.opp.flags.stage() < 6 {
            self.opp.prev_action
        } else {
            self.opp.act.current
        }
    }

    fn anim_flags(&self, motion: i32) -> u32 {
        self.env.anim_flags(motion)
    }

    fn anim_id(&self) -> i32 {
        self.me.anim.cur
    }

    fn play(&mut self, motion: i32, blend: f32) {
        self.me.anim_play(self.data, motion, blend);
    }

    fn request_anim(&mut self, motion: i32, blend: f32) {
        self.me.anim_request(motion, blend);
    }

    fn step(&mut self, yaw: u32, pitch: u32, dir: u32, speed: f32, accel: f32) {
        self.me.step(
            self.data,
            self.opp,
            self.opp_data,
            yaw,
            pitch,
            dir,
            speed,
            accel,
        );
    }

    fn blocked(&self) -> bool {
        is_blocked_by_opponent(self.me, self.opp, self.data, self.opp_data)
    }

    fn query_event(&self, mask: u64, layer: usize, what: u32) -> i32 {
        self.me.obj.query_event(&self.data.obj, mask, layer, what)
    }

    /// `BtlParam_GetFlags2`.
    fn flags2(&self) -> u32 {
        self.data.tables.params.flags2()
    }

    /// `BtlAct_TestPoweredSkill`.
    fn powered_skill(&self, mask: u32) -> bool {
        self.flag(6) && self.data.tables.params.flags3() & mask != 0
    }

    /// `BtlColl_AddActionBit`: the action classes offered this frame (read by the CPU).
    fn add_action_bit(&mut self, action: i32) {
        let bit: u64 = match action {
            0x70 => 1,
            0x73 => 2,
            0x74 => 4,
            0x77 => 8,
            0x7E => 0x10,
            0x80 => 0x20,
            0x82 => 0x40,
            0x7B | 0x7C => 0x800_0000,
            0x19 => 0x80,
            0x5A..=0x5D => 0x100,
            0x44 => 0x200,
            0x47..=0x4C => 0x400,
            0x53..=0x57 => 0x800,
            0x6B..=0x6F => 0x20_0000_0000,
            0xAE => 0x1000,
            0xAF => 0x2000,
            0xB0 => 0x4000,
            0xB1 => 0x8000,
            0xB2 => 0x1_0000,
            0xB3 => 0x2_0000,
            0x5E => 0x4_0000,
            0x5F => 0x8_0000,
            0x60 => 0x10_0000,
            0x61 => 0x20_0000,
            0x62 => 0x40_0000,
            0x63 => 0x80_0000,
            0x66 => 0x100_0000,
            0x64 | 0x65 => 0x200_0000,
            0x84 | 0x85 => 0x8000_0000,
            0xB4 => 0x400_0000,
            0xB6 => 0x4000_0000,
            0x90 => 0x2000_0000,
            0x9B => 0x1000_0000,
            0x99 => 0x1_0000_0000,
            0x93 => 0x2_0000_0000,
            0x96 => 0x4_0000_0000,
            0x34 => 0x8_0000_0000,
            0xA2 => 0x10_0000_0000,
            0x25 => 0x200_0000_0000,
            0x26 => 0x400_0000_0000,
            0x28 | 0x2A => 0x800_0000_0000,
            0xD7 => 0x1000_0000_0000,
            0x24 => 0x2000_0000_0000,
            0x42 => 0x4000_0000_0000,
            _ => 0,
        };
        self.me.act_bits |= bit;
    }

    // ---- decisions (btl_act_change.c) ----------------------------------------------------------------------------

    /// `BtlDecide_Common`: stun -> 0xD3 (bit 3), flag 0xBE -> 0xEA (bit 2), lost footing -> 0x11 (bit 0),
    /// dead -> 0xD2 (bit 1).
    fn decide_common(&mut self, mask: u32) -> bool {
        if mask & 8 != 0 && self.me.stun_timer > 0 {
            self.set_queue(0, 0xD3);
            return true;
        }
        if mask & 4 != 0 && self.flag(0xBE) {
            self.set_queue(0, 0xEA);
            return true;
        }
        if mask & 1 != 0 && !self.flag(0xE) && !self.flag(0xF) && !self.me.flags.test_prev(0xF) {
            self.set_queue(0, 0x11);
            return true;
        }
        if mask & 2 != 0 && self.me.is_dead() {
            self.set_queue(0, 0xD2);
            return true;
        }
        false
    }

    /// `BtlDecide_Main`: the main input table of a free fighter; the first satisfied input queues its action.
    /// The techniques, skills, transformations, fusions and member switches are not ported: their inputs are not
    /// recognised.
    fn decide_main(&mut self, mask: u32) -> u32 {
        if !self.me.is_free() {
            return 0;
        }
        if mask & 0x1000_0000 != 0 && self.me.has_health(5000) {
            self.add_action_bit(0x42);
            if self.test(0x2E) {
                self.set_queue(0, 0x42);
                return 1;
            }
        }
        if mask & 0x400 != 0 && self.me.has_ki(1000) {
            if self.test(0x15) {
                self.set_queue(0, 0x1A);
                return 1;
            }
            if self.flag(5) && self.test(0x14) {
                self.set_queue(0, 0x19);
                return 1;
            }
        }
        if mask & 0x800_0000 != 0 && self.me.has_ki(10000) && self.flag(5) {
            let ok = !self.flag(0x13) || self.opp_seen_action() == 0xD5;
            if ok && self.test(0x14) {
                self.set_queue(0, 0x33);
                return 1;
            }
        }
        if mask & 0x800 != 0 && self.test(0xF) {
            self.set_queue(0, 0x17);
            return 1;
        }
        if mask & 0x1000 != 0 && !self.flag(0xF) && self.test(0x11) {
            self.set_queue(0, 0x18);
            return 1;
        }
        if mask & 6 != 0 && self.flag(0xF) && !self.flag(0x11) && !self.flag(0xE) && self.test(8) {
            self.set_queue(0, if mask & 2 != 0 { 0x10 } else { 0x12 });
            return 1;
        }
        if mask & 0x18 != 0 && (!self.flag(0x13) || mask & 0x10 != 0) && self.test(3) {
            self.set_queue(0, 0xF);
            return 1;
        }
        if mask & 0x6000 != 0 && self.flag(5) && (self.flag(0x13) || mask & 0x4000 != 0) {
            for (input, action) in [(0x19, 0x1B), (0x1A, 0x1C), (0x1B, 0x1D)] {
                if self.test(input) {
                    self.set_queue(0, action);
                    return 1;
                }
            }
        }
        if mask & 0x18000 != 0
            && self.flags2() & 1 != 0
            && self.flag(5)
            && (self.flag(0x13) || mask & 0x10000 != 0)
            && self.test(0x1C)
        {
            self.set_queue(0, 0x1E);
            return 1;
        }
        // BtlParam_GetAmountA: 10000 without abilities.
        if mask & 0x20000 != 0 && self.flags2() & 8 != 0 && self.flag(5) && self.me.has_ki(10000) {
            for (input, action) in [(0x20, 0x23), (0x1D, 0x20), (0x1E, 0x21), (0x1F, 0x22)] {
                if self.test(input) {
                    self.set_queue(0, action);
                    return 1;
                }
            }
        }
        // BtlParam_GetAmountB: 20000.
        if mask & 0x40000 != 0
            && self.flag(5)
            && self.flag(0x13)
            && self.me.has_ki(20000)
            && self.flag(0xB3)
            && self.me.unk1080 > 0
        {
            self.add_action_bit(0x24);
            if self.test(0x21) {
                self.set_queue(0, 0x24);
                return 1;
            }
        }
        if mask & 0x2000_0000 != 0 && self.powered_skill(8) && self.test(0x31) {
            self.set_queue(0, 0x35);
            return 1;
        }
        // Mask 0x100000 (techniques) and 0x800000 (skills): not ported.
        if mask & 0x80 != 0 && !self.flag(6) && !self.flag(0x98) {
            if self.data.tables.params.chara_flags() & 0x80 != 0 {
                if self.me.gauge.ki >= self.me.gauge.ki_max
                    && self.me.gauge.blast >= 100_000
                    && self.test(0x13)
                {
                    self.set_queue(0, 0x37);
                    return 1;
                }
            } else if self.test(0x13) {
                self.set_queue(0, 0x37);
                return 1;
            }
        }
        // Masks 0x1000000 / 0x2000000 need an incoming blast: there are none.
        if mask & 0x80000 != 0 && self.test(0x22) {
            if self.flag(5) && opp_yaw_from_facing(self.me, self.opp).abs() > 1.7 {
                self.set_queue(0, 0x3A);
                self.set_queue(1, 0x38);
                return 2;
            }
            self.set_queue(0, 0x38);
            return 1;
        }
        if mask & 0x20 != 0 && (self.flag(0xE) || self.flag(0x11)) && self.test(0xA) {
            let id = if self.data.can_fly() { 0x14 } else { 0x16 };
            self.set_queue(0, id);
            return 1;
        }
        if mask & 0x40 != 0 && (self.flag(0xE) || self.flag(0x11)) && !self.flag(0xF) {
            if !self.data.can_fly() && !self.flag(0x11) {
                if self.test(0xE) {
                    self.set_queue(0, 0x11);
                    return 1;
                }
            } else if self.test(0xC) {
                self.set_queue(0, 0x15);
                return 1;
            }
        }
        // Masks 0x400000 (switch), 0x100 (transformation), 0x200 (fusion): not ported.
        if mask & 1 != 0 && self.test(1) {
            if self.flag(0x13) {
                if self.current() != 0xE {
                    self.set_queue(0, 0xE);
                    return 1;
                }
            } else if self.current() != 0xD {
                self.set_queue(0, 0xD);
                return 1;
            }
        }
        if mask & 0x20_0000 != 0 && !self.flag(5) && !self.flag(0x93) && self.flag(3) {
            self.set_queue(0, 0x36);
            return 1;
        }
        0
    }

    /// `BtlDecide_Attack`: rush attacks, smashes, throws, ki blasts. The ki blasts, the finishers and the other
    /// attack records need tables that are not ported; their inputs are not recognised.
    fn decide_attack(&mut self, mask: u32) -> u32 {
        if !self.me.is_free() {
            return 0;
        }
        if mask & 1 != 0 {
            self.add_action_bit(0x44);
            if self.test(0x34) {
                if self.flag(5) && opp_yaw_from_facing(self.me, self.opp).abs() > 1.7 {
                    self.set_queue(0, 0x3A);
                    self.set_queue(1, 0x44);
                    return 2;
                }
                self.set_queue(0, 0x44);
                return 1;
            }
        }
        if mask & 2 != 0 {
            self.add_action_bit(0x47);
            for (input, action) in [(0x35, 0x47), (0x36, 0x48), (0x37, 0x49), (0x38, 0x4A)] {
                if self.test(input) {
                    self.set_queue(0, action);
                    return 1;
                }
            }
            if self.test(0x39) {
                self.set_queue(0, if self.me.rush_step < 5 { 0x4B } else { 0x4C });
                return 1;
            }
        }
        if mask & 0x2000 != 0 {
            self.add_action_bit(0x55);
            for (input, action) in [
                (0x3A, 0x55),
                (0x3B, 0x56),
                (0x3C, 0x53),
                (0x3D, 0x54),
                (0x3E, 0x57),
            ] {
                if self.test(input) {
                    self.set_queue(0, action);
                    return 1;
                }
            }
        }
        if mask & 0x400_0000 != 0 && self.flags2() & 0x1_0000 != 0 {
            self.add_action_bit(0x6D);
            for (input, action) in [
                (0x4A, 0x6D),
                (0x4B, 0x6E),
                (0x48, 0x6B),
                (0x49, 0x6C),
                (0x4C, 0x6E),
            ] {
                if self.test(input) {
                    self.set_queue(0, action);
                    return 1;
                }
            }
        }
        if mask & 0x1_0000 != 0 && self.test(0x40) {
            self.set_queue(0, 0x59);
            return 1;
        }
        if mask & 0xC0 != 0 && (self.flag(0x13) || mask & 0x80 != 0) {
            self.add_action_bit(0xB4);
            if self.test(0x5C) {
                self.set_queue(0, 0xB4);
                return 1;
            }
        }
        if mask & 0x300 != 0
            && (self.flag(0x13) || mask & 0x200 != 0)
            && self.flags2() & 0x8_0000 != 0
        {
            self.add_action_bit(0xB6);
            if self.test(0x5E) {
                self.set_queue(0, 0xB6);
                return 1;
            }
        }
        if mask & 0x8_0000 != 0 && self.me.dash_count < self.me.dash_limit && self.me.has_ki(5000) {
            self.add_action_bit(0x19);
            if self.test(0x2A) {
                self.set_queue(0, 0x19);
                return 1;
            }
        }
        if mask & 0x400 != 0 && self.test(0x5F) {
            self.set_queue(0, 0x67);
            return 1;
        }
        if mask & 0x1000 != 0 && self.test(0x60) {
            self.set_queue(0, 0x68);
            return 1;
        }
        0
    }

    // ---- the dispatcher -----------------------------------------------------------------------------------------

    fn handler(&mut self, phase: u32) {
        match self.current() {
            1 => self.entrance(phase),
            2 | 3 => self.result_pose(phase),
            0xB => self.neutral(phase),
            0xC => self.idle_once(phase),
            0xD => self.walk(phase),
            0xE => self.close_walk(phase),
            0x11 | 0x13 => self.jump_air(phase),
            0x3A => self.turn(phase),
            0xF => self.dash(phase),
            0x10 | 0x12 => self.jump_start(phase),
            0x14 => self.ascend(phase),
            0x15 => self.descend(phase),
            0x16 => self.hop(phase),
            0x17 | 0x18 => self.fast_vertical(phase),
            0x37 => self.charge(phase),
            0x38 => self.guard(phase),
            0x44 => self.rush(phase),
            0xBD | 0xC0..=0xCA => self.stagger(phase),
            0xCF..=0xD1 => self.pushed_back(phase),
            0xD2 => self.knocked_out(phase),
            0xD3 => self.stun(phase),
            0xD8 => self.lie_down(phase),
            0xD9 => self.fall(phase),
            0xDA..=0xDE => self.down(phase),
            0xE0 => self.air_stun(phase),
            0xE1..=0xE5 => self.get_up(phase),
            0xE6 => self.recover(phase),
            _ => {}
        }
    }

    /// `BtlAct_Update`: stage 4 decide, the forced check, the switch (or the motion requests); stage 5 run,
    /// gauges, timers.
    pub(crate) fn update(&mut self) {
        self.me.flags.set_stage(4);
        self.handler(DECIDE);
        self.check_forced();
        if self.me.act.request != -1 {
            self.handler(LEAVE);
            self.me.apply_root_yaw();
            self.me.act.current = self.me.act.request;
            self.me.act.request = -1;
            self.me.work = [0; 0x14];
            self.me.action_frame = 0;
            self.me.charge = 0.0;
            self.me.charge_timer = 0;
            self.me.charge_full_frames = 0;
            self.me.blast_charge = 0;
            self.pop_queue();
            self.me.flags.clear_range(0x81, 0x84);
            self.me.flags.clear_range(0x5B, 0x5D);
            self.me.flags.clear_range(0xDD, 0xDD);
            self.me.status.contact = 0;
            self.set(8);
            if self.flag(0x94) {
                self.me
                    .set_heading(self.me.react.turn_yaw, self.me.react.turn_pitch);
            }
            self.handler(ENTER);
        } else {
            self.me.anim_flush_requests(self.data);
            self.me.apply_root_yaw();
        }
        self.me.flags.set_stage(5);
        self.me.hold_action = false;
        self.handler(RUN);
        self.update_gauges();
        self.update_timers();
    }

    /// `BtlAct_CheckForced`: fighter flags raised elsewhere turned into requests, lowest priority first.
    fn check_forced(&mut self) {
        if self.me.hold_action {
            return;
        }
        if self.current() == 0 {
            self.request(0xB);
        }
        if self.me.is_free() {
            for (flag, action) in [(0x74, 0x21), (0x75, 0x22), (0x76, 0x20)] {
                if self.flag(flag) {
                    self.request(action);
                }
            }
        }
        for (flag, action) in [(0x6D, 0xC9), (0x6F, 0xCA)] {
            if self.flag(flag) {
                self.request(action);
            }
        }
        if self.flag(0x68) || self.flag(0x69) || self.flag(0x6A) {
            self.request(0x38);
        }
        if self.flag(0x7B) {
            self.request(0x39);
        }
        if self.flag(0x79) {
            self.request(0xBD);
        }
        if self.queue_reaction(self.me.react.reaction) {
            let q = self.queued();
            self.request(q);
        }
        for (flag, action) in [(0x7F, 0xBE), (0x61, 0xFA), (0x62, 0xFB)] {
            if self.flag(flag) {
                self.request(action);
            }
        }
        if self.flag(0x139) {
            self.clear(0x139);
            self.request(0xB);
        }
        if self.flag(0x13A) {
            self.clear(0x13A);
            self.request(0xD8);
        }
        for (flag, action) in [(0xEF, 1), (0xF1, 2), (0xF2, 3)] {
            if self.flag(flag) {
                self.request(action);
            }
        }
    }

    /// `BtlAct_QueueReaction`: the damage action of a pending hit reaction.
    fn queue_reaction(&mut self, reaction: i32) -> bool {
        if reaction < 2 || (reaction == 2 && !self.me.is_dead()) {
            return false;
        }
        let mut reaction = reaction.clamp(0, 0x31) as usize;
        if self.me.react_count[reaction] < 100 {
            self.me.react_count[reaction] += 1;
        }
        match reaction {
            0x23 => self.me.react_count[0x24] = self.me.react_count[0x24].saturating_add(1),
            0x24 => self.me.react_count[0x23] = self.me.react_count[0x23].saturating_add(1),
            _ => {}
        }
        let turn = |me: &mut Chr| {
            me.react.turn_yaw = if me.react.back {
                me.react.yaw
            } else {
                wrap_angle(me.react.yaw + PI)
            };
            me.react.turn_pitch = 0.0;
        };
        if matches!(reaction, 5..=13 | 35 | 36 | 42 | 45..=47) && self.me.react_count[reaction] >= 2
        {
            self.set(0x94);
            reaction = 0x10;
            turn(self.me);
        }
        if self.me.is_dead() && matches!(reaction, 2..=9 | 23..=25 | 42..=48) {
            self.set(0x94);
            reaction = 0xF;
            turn(self.me);
        }
        self.me.react_timer[reaction] = 0;
        let stun = |me: &mut Chr, frames: i32| {
            if me.stun_timer <= 0 {
                me.stun_timer = frames;
            }
        };
        let action = match reaction {
            4 => 0xC1,
            5 => 0xC2,
            6 => 0xC3,
            7 | 43 | 46 => 0xC4,
            8 | 44 => 0xC5,
            9 | 42 | 45 | 47 | 48 => 0xC6,
            10 => 0xCE,
            11 => 0xCD,
            14 => 0xDA,
            15 => 0xCF,
            16 => 0xD0,
            38 => 0xD1,
            17 => 0xD5,
            18 => 0xD6,
            19 => 0xD4,
            20 => 0xB8,
            37 => 0xBC,
            22 => 0xBA,
            49 => 0xBF,
            12 | 39 => 0xCB,
            13 => 0xCC,
            31 => 0xDB,
            32 => 0xDC,
            33 => 0xDD,
            35 => 0xC7,
            36 => 0xC8,
            40 => 0xDF,
            23..=25 => {
                stun(self.me, [15, 30, 45][reaction - 23]);
                0xD3
            }
            26..=28 => {
                stun(self.me, [15, 30, 45][reaction - 26]);
                0xE0
            }
            // The throw-victim actions 0x131.. / 0x134.. / 0x137..: not ported.
            29 | 30 | 34 | 41 => -1,
            _ => 0xC0,
        };
        self.set(0x129);
        self.set_queue(0, action);
        true
    }

    /// `BtlAct_UpdateGauges`: ki regeneration in a motion with flag 0x10000, blast gain with 0x20000, the charge
    /// gauge's decay. Abilities, skills and the powered-up mode are not ported.
    fn update_gauges(&mut self) {
        if self.time_stopped
            || !self.flag(3)
            || self.flag(0x125)
            || self.flag(0x126)
            || self.flag(0x128)
        {
            return;
        }
        let mflags = self.anim_flags(self.anim_id());
        if self.me.is_dead() {
            return;
        }
        self.me.sec_timer += 1;
        if self.me.sec_timer >= 30 {
            self.me.sec_timer = 0;
        }
        let p = &self.data.tables.params;
        if mflags & 0x1_0000 != 0 {
            if self.flag(0xBE) {
                // BtlParam_GetKiRecoverRate.
                let mut amount =
                    (p.s32_at(0x3C) / 30 + (self.env.curve_base(2) / 30.0) as i32).max(10);
                self.set(0xE7);
                if self.me.input.is_pressed(0x10_0000) {
                    amount *= 4;
                }
                self.me.add_ki(amount);
                if self.me.has_ki(p.s32_at(0x2C)) {
                    self.clear(0xBE);
                }
            } else if !self.flag(0x98) {
                // BtlParam_GetKiRegenLimit / BtlParam_GetKiRegenRate.
                if !self.me.has_ki(p.s32_at(0x28).min(100_000)) {
                    let rate =
                        (p.s32_at(0x38) / 30 + (self.env.curve_base(1) / 30.0) as i32).max(0);
                    self.me.add_ki(rate);
                }
                if matches!(self.env.mode, 5 | 6) {
                    self.me.add_ki(1000);
                }
            }
        }
        if mflags & 0x2_0000 != 0 {
            // BtlParam_GetBlastGainRate.
            let rate = (p.s32_at(0x44) / 30 + (self.env.curve_base(3) / 30.0) as i32).max(20);
            self.me.add_blast(rate);
            if matches!(self.env.mode, 5 | 6) {
                self.me.add_blast(0xD05);
            }
        }
        self.me.charge_gauge = (self.me.charge_gauge - 400).max(0);
        if matches!(self.env.mode, 5 | 6) && !self.flag(0x95) {
            self.me.add_health(200);
        }
    }

    /// `BtlAct_UpdateTimers`: the impulse, the display offset, the defence windows, the flag-held counters, the
    /// stun and status countdowns, the action frame counter.
    fn update_timers(&mut self) {
        let mflags = self.anim_flags(self.anim_id());
        self.me.apply_impulse(self.time_stopped);
        self.me.update_hover_offset(self.time_stopped);
        self.update_defense_timers();
        if self.time_stopped {
            return;
        }
        if !self.flag(0x82) {
            self.clear(0x8A);
        }
        if !self.flag(0x88) {
            self.me.rush_step = 0;
            if mflags & 2 == 0 {
                self.clear(0x86);
            }
        }
        if !self.flag(0x89) {
            self.me.dash_count = 0;
            self.me.vanish_count = 0;
        }
        if !self.flag(0x95) {
            self.me.combo.damage = 0;
            self.me.combo.hits = 0;
            if self.me.combo.timer > 0 {
                self.me.combo.timer -= 1;
            }
            if self.me.flags.test_prev(0x95) {
                self.me.react_count = [0; 0x32];
            }
        }
        for t in &mut self.me.react_timer {
            if *t < 100 {
                *t += 1;
            }
        }
        if self.me.stun_timer > 0 {
            self.me.set_fx(0x1A);
            if mflags & 0x1_0000 != 0 {
                self.me.stun_timer -= 1;
                self.set(0xE7);
                if self.me.input.is_pressed(0x10_0000) {
                    self.me.stun_timer = (self.me.stun_timer - 3).max(0);
                }
            }
            if self.me.is_dead() {
                self.me.stun_timer = 0;
            }
        }
        if self.me.blind_timer > 0 {
            self.me.blind_timer -= 1;
            self.set(0x93);
            self.set(0x137);
            self.set(0x96);
        }
        let target = self.me.blind_timer * 2;
        let level = self.me.blind_level;
        self.me.blind_level = if level < target {
            (level + 15).min(target)
        } else {
            (level - 15).max(target)
        };
        if self.me.shake_timer > 0 {
            self.me.shake_timer -= 1;
            if self.me.is_dead() {
                self.me.shake_timer = 0;
            }
        }
        if self.me.unk_fe4 > 0 {
            self.me.unk_fe4 -= 1;
            self.set(0x96);
        }
        if self.me.blast_shots_timer > 0 {
            if mflags & 4 == 0 {
                self.me.blast_shots_timer -= 1;
            }
        } else {
            self.me.blast_shots = 0;
        }
        if self.me.tech_delay > 0 {
            self.me.tech_delay -= 1;
        }
        if self.me.unk_ff0 > 0 {
            self.me.unk_ff0 -= 1;
            if !self.flag(0x13) {
                self.me.unk_ff0 = 0;
            }
        }
        match self.current() {
            0xF | 0x3E | 0x58 | 0xB0 | 0xB1 => {
                self.me.pose.dash_boost = self.me.pose.dash_boost.max(1.0)
            }
            _ => self.me.pose.dash_boost = 1.0,
        }
        if mflags & 0x810 == 0 {
            self.clear(0x1D);
            self.clear(0x1E);
        }
        match self.current() {
            0x89..=0x8C => {
                if self.flag(8) {
                    self.me.evasion_count += 1;
                }
            }
            0x2B | 0x2D | 0x2E => {}
            _ => self.me.evasion_count = 0,
        }
        // An opponent of size class 4 doubles / triples the animation rate.
        if self.opp_data.tables.params.size_class() == 4 {
            let k = if self.data.tables.params.size_class() == 4 {
                2.0
            } else {
                3.0
            };
            self.me.obj.rate *= k;
        }
        let mut dir = -1;
        if !self.me.input.is_held(0x1_0000) {
            if self.me.input.is_held(0x4_0000) {
                dir = 2;
            } else if self.me.input.is_held(0x8_0000) {
                dir = 3;
            } else if self.me.input.is_held(0x2_0000) {
                dir = 4;
            }
        }
        self.me.dir_held = dir;
        if self.anim_flags(self.anim_id()) & 0x800 != 0 {
            if !self.me.gauge.low_health_idle {
                self.me.recover_presses = 0;
            } else if self.anim_flags(self.me.anim.prev) & 0x800 == 0 {
                self.me.recover_presses = 4;
            }
        } else {
            self.me.recover_presses = 0;
        }
        if self.env.mode == 1 && self.me.input_off_timer > 0 {
            self.me.input_off_timer -= 1;
            self.set(0x11E);
        }
        self.me.action_frame += 1;
    }

    /// `BtlMove_UpdateDefenseTimers`: the defence windows, each counting down to a floor and reopened by its input.
    fn update_defense_timers(&mut self) {
        let mut on = self.test(0x27);
        let hp = self.me.gauge.health as f32 / self.me.gauge.health_max as f32;
        if !self.flag(5) {
            on = false;
        }
        let me = &mut *self.me;
        if me.flags.test(0x12C) {
            me.unk1068 = -30;
            me.dodge_window = -30;
            me.rush_break_window = -30;
        }
        me.unk1068 -= 1;
        if me.unk1068 < -30 {
            me.unk1068 = -30;
            if on {
                me.unk1068 = if hp < 0.3 {
                    2
                } else if hp < 0.7 {
                    3
                } else {
                    4
                };
            }
        } else {
            if on {
                me.unk1068 = -1;
            }
            if me.react.reaction != 1 {
                me.unk1068 = -30;
            }
        }
        let wide = me.flags.test(0x138);
        me.dodge_window -= 1;
        if me.dodge_window < -30 {
            me.dodge_window = -30;
            if on {
                me.dodge_window = if wide { 4 } else { 2 };
            }
        } else if on {
            me.dodge_window = -1;
        }
        let counter = me.test(0x26, true);
        me.counter_window -= 1;
        if me.counter_window < -30 {
            me.counter_window = -30;
            if counter {
                me.counter_window = 1;
            }
        } else if counter {
            me.counter_window = -1;
        }
        me.unk1074 -= 1;
        if me.unk1074 < -15 {
            me.unk1074 = -15;
            let deflect = me.test(0x2B, true);
            if deflect && self.data.tables.params.flags2() & 0x20 != 0 {
                me.unk1074 = 5;
            }
        }
        me.throw_break_window -= 1;
        if me.throw_break_window < -30 {
            me.throw_break_window = -30;
            if on {
                me.throw_break_window = 5;
            }
        }
        me.rush_break_window -= 1;
        if me.rush_break_window < -30 {
            me.rush_break_window = -30;
            if on {
                me.rush_break_window = if wide { 4 } else { 2 };
            }
        } else if on {
            me.rush_break_window = -1;
        }
        if self.data.tables.params.flags2() & 8 != 0 {
            me.unk1080 += 1;
            if me.unk1080 >= 2 {
                me.unk1080 = 1;
            }
            if on {
                me.unk1080 = -15;
            }
        } else {
            me.unk1080 = -1;
        }
    }

    // ---- handlers ------------------------------------------------------------------------------------------------

    /// `BtlAct_GetIdleMotion`: 0 / 1 on the ground, 0x26 / 0x27 in flight (0x18B for a non-flyer out of water).
    fn idle_motion(&self) -> i32 {
        let low = i32::from(self.me.gauge.low_health_idle);
        if self.flag(0xE) {
            if !self.data.can_fly() && !self.flag(0x11) {
                0x18B
            } else {
                0x26 + low
            }
        } else {
            low
        }
    }

    /// `BtlAct_GetIdleFollowUp`: 0xB unless an input (main mask 0x80000) or a status queues something else.
    fn idle_follow_up(&mut self) -> i32 {
        self.set_queue(0, 0xB);
        self.decide_main(0x8_0000);
        self.decide_common(0xF);
        self.queued()
    }

    /// `BtlAct_GetDownAction`.
    fn down_action(&self) -> i32 {
        if self.me.stun_timer > 0 { 0xE0 } else { 0xD8 }
    }

    /// `BtlActB_SetReactionFlags`.
    fn reaction_flags(&mut self) {
        self.set(0xD5);
        self.set(0x95);
        self.set(0x96);
        if self.flag(0x11) {
            self.held(0xE);
        }
    }

    /// `BtlActB_NextVariant`.
    fn next_variant(&mut self, n: i32) -> i32 {
        if n < 2 {
            return 0;
        }
        self.me.anim_cycle = (self.me.anim_cycle + 1) % n;
        self.me.anim_cycle
    }

    /// Action 1 (flag 0xEF): motion 0x180, then 0x181 looping; flag 0xF0 ends it.
    fn entrance(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x180, 0.0);
        }
        if phase == RUN {
            match self.anim_id() {
                0x180 => {
                    self.me.anim_advance_then(0x181, 0.0, 0);
                }
                0x181 => self.me.anim_advance_loop(0),
                _ => {}
            }
            self.step(6, 6, 7, 0.0, 0.925_925_8);
            self.me.apply_gravity();
        }
        if phase == DECIDE && self.flag(0xF0) {
            self.request(0xB);
        }
    }

    /// Actions 2 (flag 0xF1, the win pose: motions 0x182 -> 0x183) and 3 (flag 0xF2, the lose pose: motion 0x184),
    /// with the hit-stop and no-hit flags held every frame.
    fn result_pose(&mut self, phase: u32) {
        let win = self.current() == 2;
        if phase == ENTER {
            self.play(if win { 0x182 } else { 0x184 }, 0.15);
            self.set(0xF3);
            self.set(0x24);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            if win {
                self.me.stun_timer = 0;
            } else {
                self.me.blind_timer = 0;
                self.me.blind_level = 0;
            }
        }
        if phase == RUN {
            if win {
                match self.anim_id() {
                    0x182 => {
                        self.me.anim_advance_then(0x183, 0.0, 0);
                    }
                    0x183 => self.me.anim_advance_loop(0),
                    _ => {}
                }
            } else {
                self.me.anim_advance(0);
            }
            self.step(6, 6, 7, 0.0, 0.925_925_8);
            self.me.apply_gravity();
            for f in [0x42, 0x43, 0x44, 0x45, 0x46, 0x125, 0xBB, 0x3F, 0x12A] {
                self.set(f);
            }
            if !win {
                self.me.set_fx(0x14);
            }
            self.me.set_fx(0x3B);
        }
    }

    /// Action 0xB: the neutral state.
    fn neutral(&mut self, phase: u32) {
        if phase == ENTER {
            if self.me.height_above_ground() < kmh(50.0) {
                self.clear(0xE);
            }
            let m = self.idle_motion();
            self.play(m, 0.15);
        }
        if phase == RUN {
            if self.flag(0xE) {
                if self.flag(0xF) {
                    self.clear(0xE);
                }
            } else if !self.flag(0xF) && self.flag(0x11) {
                self.held(0xE);
            }
            self.me.anim_advance_loop(0);
            if self.blocked() {
                self.me.pose.speed = 0.0;
            }
            let speed = kmh(100.0);
            self.step(6, 6, 7, 0.0, speed);
            if self.anim_id() == 0x18B {
                self.me.move_vertical(speed, speed);
            } else {
                self.me.apply_gravity();
            }
            self.set(0xC9);
            self.set(0x1C);
            self.set(0x25);
            if self.me.gauge.low_health_idle {
                self.set(0x96);
                if self.data.tables.params.flags() & 0x1_0000 != 0 {
                    self.set(0x137);
                }
            }
            let m = self.idle_motion();
            if self.anim_id() != m {
                self.request_anim(m, 0.15);
            }
        }
        if phase == DECIDE {
            // The idle talk (0x43) of modes other than 1 is not ported.
            self.decide_main(0x22FC_BFEB);
            self.decide_attack(0x0800_054F);
            self.decide_common(0xF);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0x37 (`BtlAct_ChargeHandler`): ki charge, motions 0x34 -> 0x35 (loop) -> 0x36 once the max power
    /// gauge fills. The loop adds ki; at full ki with a blast stock it fills the max power gauge, and a full gauge
    /// spends the stock and holds flag 6. Releasing the charge input (action input 19) ends it outside 0x36. The
    /// sounds, voices, vibration and the chains' push are not ported.
    fn charge(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x34, 0.15);
        }
        if phase == RUN {
            match self.anim_id() {
                0x34 => {
                    self.me.anim_advance_then(0x35, 0.15, 1);
                }
                0x35 => {
                    self.me.anim_advance_loop(0);
                    self.me.set_fx(7);
                    self.set(0xC);
                    if self.flag(0xF) {
                        self.me.set_fx(0x36);
                    }
                    // BtlParam_GetRateA / B: per second / 30 plus the curve's row 0, at least 200.
                    let p = &self.data.tables.params;
                    let per_second = if self.flag(0x11) {
                        p.s32_at(0x34)
                    } else {
                        p.s32_at(0x30)
                    };
                    let amount =
                        (per_second / 30 + (self.env.curve_base(0) / 30.0) as i32).max(200);
                    self.me.add_ki(amount);
                    if self.me.gauge.ki >= self.me.gauge.ki_max {
                        if self.me.gauge.blast >= 100_000 {
                            // BtlParam_GetStepA: the whole gauge over parameter +0x50 seconds / curve row 11.
                            let t = p.f32_at(0x50) / self.env.curve_base(11);
                            let frames = (t * 30.0) as i32;
                            let step = if frames != 0 { 30000 / frames } else { 0 };
                            self.me.gauge.max_power = (self.me.gauge.max_power + step).min(30000);
                            self.set(0xBB);
                        }
                    }
                    if self.me.gauge.max_power >= 30000 {
                        self.request_anim(0x36, 0.15);
                        self.me.set_fx(8);
                        self.set(0xD);
                        self.me.gauge.blast = (self.me.gauge.blast - 100_000).max(0);
                        self.held(6);
                    }
                }
                0x36 => {
                    if self.me.anim_advance(0) {
                        let next = self.idle_follow_up();
                        self.request(next);
                    }
                    if self.data.tables.params.flags() & 0x10 == 0
                        && self.me.anim_passed_ratio(0.25)
                    {
                        self.set(0x4A);
                    }
                    self.set(0xBB);
                    self.set(0x59);
                }
                _ => {}
            }
            let accel = kmh(100.0);
            self.step(6, 6, 7, 0.0, accel);
            if !self.data.can_fly() && self.flag(0xE) && !self.flag(0x11) {
                self.me.move_vertical(accel, accel);
            } else {
                self.me.apply_gravity();
            }
            self.set(0xC9);
            self.set(0x1C);
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.flag(0xF) {
                self.clear(0xE);
            }
        }
        if phase == DECIDE && self.anim_id() != 0x36 && self.me.anim.next != 0x36 {
            if self.me.test(0x13, false) {
                self.request(0xB);
            }
            self.decide_main(0xD0_1C00);
            self.decide_common(6);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0xF (`BtlAct_DashMoveHandler`): the dash, motions 0xC (start) -> 0xD (loop) -> 0xE (stop), or 0x15
    /// (turn-around), with the side layers 0xF..0x14, 0x16, 0x17. work[0] bit 0: a turn was asked for during the
    /// start; work[1]: finished. Entered from 0x3E, 0xB0 or 0xB1 it resumes in the loop. The sounds are not ported.
    fn dash(&mut self, phase: u32) {
        if phase == ENTER {
            if matches!(self.me.act.prev, 0x3E | 0xB0 | 0xB1) {
                self.play(0xD, 0.25);
            } else {
                self.play(0xC, 0.15);
                self.set(0xD0);
                self.me.turn_yaw(self.opp, 0, PI);
            }
        }
        if phase == RUN {
            let (sub_neg, sub_pos, ratio, moving) = match self.anim_id() {
                0xC => {
                    self.me.anim_advance_then(0xD, 0.15, 0);
                    (0xF, 0x12, self.me.anim_progress(), true)
                }
                0xD => {
                    self.me.anim_advance_loop(0);
                    (0x10, 0x13, 1.0, true)
                }
                0xE => {
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                    (0x11, 0x14, 1.0 - self.me.anim_progress(), false)
                }
                0x15 => {
                    if self.me.anim_is_new() && self.flag(0xF) {
                        self.me.set_fx(0x31);
                    }
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                    self.me.work[0] &= !1;
                    (0x16, 0x17, 1.0 - self.me.anim_progress(), false)
                }
                _ => (0, 0, 0.0, true),
            };
            if self.data.tables.params.flags() & 0x2_0000 == 0 {
                let side = self.me.rot_y_rel_cam().sin();
                if 0.0 < side {
                    self.me.anim_set_sub_mix(side);
                    self.me.anim_play_sub(self.data, sub_pos);
                } else {
                    self.me.anim_set_sub_mix(-side);
                    self.me.anim_play_sub(self.data, sub_neg);
                }
            }
            if self.flag(5) && moving && (!self.flag(0xE) || !self.me.input.is_held(DIR_MASK)) {
                self.set(0x1A);
            }
            let pitch_mode = if self.flag(0xE) && self.flag(5) { 2 } else { 5 };
            if moving {
                let speed =
                    self.me
                        .move_speed(self.data, self.env, 4 + usize::from(self.flag(0x11)));
                let turn = self.me.turn_rate(self.data, 0);
                self.me.turn_yaw(self.opp, 0, turn);
                self.me
                    .turn_pitch(self.data, self.opp, self.opp_data, pitch_mode, deg(9.0));
                self.me.turn_model_yaw(deg(36.0), 0.3);
                self.me.set_direction(3);
                self.me.advance(speed, 10000.0);
                self.me.apply_gravity();
                self.me.set_fx(0xB);
                if self.flag(0xF) {
                    self.me.set_fx(0x30);
                }
            } else {
                self.step(6, 5, 3, 0.0, kmh(100.0));
                self.me.apply_gravity();
            }
            let toward = opp_yaw_from_facing(self.me, self.opp).cos();
            if 0.0 < toward {
                let pitch = self.me.pose.pitch;
                self.me.set_lean_x(pitch * toward * ratio);
            }
            self.set(0xCA);
            self.set(0x25);
            self.me.request_orbit(50.0, 100.0);
            self.me.set_fx(0x3A);
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.env.stage_4_or_27 && self.flag(0x17) {
                self.held(0xE);
            }
        }
        if phase == DECIDE {
            if self.me.work[1] != 0 {
                self.request(0xB);
                if !self.has_queued() {
                    self.decide_main(1);
                    self.decide_common(1);
                }
                let q = self.queued();
                self.request(q);
            }
            match self.anim_id() {
                0xC => {
                    if self.me.test(4, true) {
                        self.me.work[0] |= 1;
                    }
                }
                0xD => {
                    if self.me.test(6, true) {
                        self.request_anim(0xE, 0.15);
                    }
                    if self.blocked() {
                        self.request_anim(0xE, 0.15);
                        self.me.pose.speed = 0.0;
                    }
                    if self.me.test(4, true) || self.me.work[0] & 1 != 0 {
                        self.request_anim(0x15, 0.15);
                    }
                }
                0xE => {
                    if self.blocked() {
                        self.me.pose.speed = 0.0;
                    }
                    self.decide_main(0x300);
                }
                0x15 => {
                    if self.me.anim_passed_ratio(0.5) && self.me.test(5, true) {
                        self.request(0xF);
                        self.me.pose.dash_boost = (self.me.pose.dash_boost + 0.1).min(1.5);
                    }
                }
                _ => {}
            }
            let mut queued = self.decide_main(0x2000_1C64) != 0;
            if self.anim_id() != 0xE {
                if self.me.test(0x40, true) {
                    self.request(0x58);
                }
                queued |= self.decide_attack(0xC000) != 0;
                if self.me.test(0x2C, true) {
                    self.request(0x3E);
                }
            }
            queued |= self.decide_common(0xE);
            if queued {
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// `BtlInput_GetStickLength`.
    fn stick_length(&self) -> f32 {
        let s = self.me.input.stick();
        (s[0] * s[0] + s[1] * s[1]).sqrt()
    }

    /// `BtlMoveParam_GetAccel`: table +0x50 in 10 km/h per second, per frame per frame.
    fn move_accel(&self, n: usize) -> f32 {
        self.data.tables.moves.accel[n] * 10.0 * (1000.0 / 3600.0) * (1.0 / 30.0 / 30.0)
    }

    /// `BtlMoveParam_GetKiCost` without abilities: table +0x64 per second, per frame.
    fn move_ki_cost(&self, n: usize) -> i32 {
        self.data.tables.moves.ki_cost[n] / 30
    }

    /// `BtlAct_GetAction14or16`.
    fn ascend_action(&self) -> i32 {
        if self.data.can_fly() { 0x14 } else { 0x16 }
    }

    /// `BtlAct_GetAction11or15`.
    fn descend_action(&self) -> i32 {
        if !self.data.can_fly() && !self.flag(0x11) {
            0x11
        } else {
            0x15
        }
    }

    /// Actions 0x10 and 0x12 (`BtlAct_JumpStartHandler`): the take-off, motion 0x1F, then 0x11 or 0x13. The
    /// sound is not ported.
    fn jump_start(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x1F, 0.1);
            self.clear(0xE);
            self.me.unk_ff0 = 0;
            if self.current() == 0x12 {
                self.me.work[0] |= 1;
            }
        }
        if phase == RUN {
            let high = (self.me.work[0] & 1) as usize;
            if self.me.anim_advance(1) {
                self.request(if high != 0 { 0x13 } else { 0x11 });
            }
            if self.me.anim_passed_ratio(0.3) {
                // BtlMoveParam_GetHeight and BtlMove_CalcJumpSpeed.
                self.me.pose.fall_speed = jump_speed(self.data.tables.moves.height[high] * 10.0);
            }
            let speed = self
                .me
                .move_speed(self.data, self.env, if high != 0 { 4 } else { 0xD });
            let stick = self.stick_length();
            self.step(0, 5, 3, speed * stick, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xC9);
            self.me.request_orbit(20.0, 50.0);
            self.set(0x1A);
            self.me.set_fx(0x3A);
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
        }
        if phase == DECIDE {
            if self.me.test(9, true) {
                let id = self.ascend_action();
                self.request(id);
            }
            if self.data.can_fly() && self.me.test(0xC, true) {
                let id = self.descend_action();
                self.request(id);
            }
            if self.me.test(7, true) {
                self.request(0xF);
                self.held(0xE);
            }
            self.decide_main(0x1C00);
            let q = self.queued();
            self.request(q);
            if self.me.test(0x2C, true) {
                self.request(0x3F);
            }
            if self.decide_attack(0x7_0000) != 0 {
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// Action 0x14 (`BtlAct_AscendHandler`): rising, motion 0x2B (through 0x2E after a jump). The sounds are not
    /// ported.
    fn ascend(&mut self, phase: u32) {
        if phase == ENTER {
            let motion = if self.me.unk_ff0 > 0 { 0x2E } else { 0x2B };
            self.play(motion, 0.15);
            self.held(0xE);
        }
        if phase == RUN {
            match self.anim_id() {
                0x2E => {
                    self.me.anim_set_duration(0.35);
                    self.me.anim_advance_then(0x2B, 0.15, 0);
                    self.step(6, 5, 3, 0.0, kmh(50.0));
                    self.me.apply_gravity();
                }
                0x2B => {
                    self.me.anim_advance_loop(0);
                    let speed =
                        self.me
                            .move_speed(self.data, self.env, usize::from(self.flag(0x11)))
                            * self.stick_length();
                    let vspeed = -self.me.move_speed(self.data, self.env, 0x10);
                    let vaccel = self.move_accel(0);
                    self.step(0, 5, 3, speed, kmh(50.0));
                    self.me.move_vertical(vspeed, vaccel);
                }
                _ => {}
            }
            self.set(0xC9);
            self.me.request_orbit(20.0, 50.0);
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            if self.me.test(0xB, true) {
                self.request(0xB);
                self.decide_main(0x41);
            }
            self.decide_main(0x1800);
            self.decide_attack(0x7_0000);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0x15 (`BtlAct_DescendHandler`): sinking, motion 0x2C (through 0x1F after a jump), landing with
    /// 0x2D. The sounds and the landing effect are not ported.
    fn descend(&mut self, phase: u32) {
        if phase == ENTER {
            let motion = if self.me.unk_ff0 > 0 { 0x1F } else { 0x2C };
            self.play(motion, 0.15);
            self.held(0xE);
        }
        if phase == RUN {
            let speed = self
                .me
                .move_speed(self.data, self.env, usize::from(self.flag(0x11)))
                * self.stick_length();
            let vspeed = self.me.move_speed(self.data, self.env, 0x11);
            let vaccel = self.move_accel(1);
            match self.anim_id() {
                0x1F => {
                    self.me.anim_set_duration(0.35);
                    self.me.anim_advance_then(0x2C, 0.15, 0);
                }
                0x2C => {
                    self.me.anim_advance_loop(0);
                    if self.me.frames_to_ground() < 1.0
                        && (!self.env.stage_4_or_27
                            || self.flag(7)
                            || self.me.pose.ground_pos[1] < -10.0)
                        && self.me.pose.ground_flags & 0x40 == 0
                    {
                        self.request_anim(0x2D, 0.0);
                    }
                    self.step(0, 5, 3, speed, kmh(50.0));
                    self.me.move_vertical(vspeed, vaccel);
                    self.set(0xC9);
                }
                0x2D => {
                    self.clear(0xE);
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                    self.step(0, 5, 3, 0.0, kmh(100.0));
                    self.me.apply_gravity();
                }
                _ => {}
            }
            self.me.request_orbit(20.0, 50.0);
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            match self.anim_id() {
                0x1F | 0x2C => {
                    if self.me.test(0xD, true) {
                        self.request(0xB);
                        self.decide_main(0x21);
                        let q = self.queued();
                        self.request(q);
                    }
                }
                0x2D => {
                    if self.me.work[1] != 0 {
                        let id = if self.me.test(1, true) { 0xD } else { 0xB };
                        self.request(id);
                    }
                }
                _ => {}
            }
            self.decide_main(0x1800);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0x16 (`BtlAct_HopHandler`): a short rise for a fighter that cannot fly, fading out over motion
    /// 0x18C, then 0xB.
    fn hop(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x18C, 0.15);
            self.held(0xE);
            self.me.pose.fall_speed = 0.0;
        }
        if phase == RUN {
            let fade = 1.0 - self.me.anim_progress();
            let speed = self
                .me
                .move_speed(self.data, self.env, usize::from(self.flag(0x11)))
                * self.stick_length()
                * fade;
            let vspeed = self.me.move_speed(self.data, self.env, 0x10) * fade;
            let vaccel = self.move_accel(0) * 0.5;
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            self.step(0, 5, 2, speed, kmh(50.0));
            self.me.move_vertical(-vspeed, vaccel);
            self.set(0xC9);
            self.me.request_orbit(20.0, 50.0);
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            self.decide_main(0x1800);
            self.decide_attack(0x7_0000);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Actions 0x17 / 0x18 (`BtlAct_FastAscendHandler` / `BtlAct_FastDescendHandler`): the fast rise (motions
    /// 0x2E, 0x2F, 0x30) or fall (0x31, 0x32, 0x33), draining ki while it lasts. The sounds and the vibration are
    /// not ported.
    fn fast_vertical(&mut self, phase: u32) {
        let up = self.current() == 0x17;
        let first = if up { 0x2E } else { 0x31 };
        if phase == ENTER {
            self.play(first, 0.15);
            self.held(0xE);
            if self.me.unk_ff0 > 0 {
                self.me.work[0] |= 1;
            }
        }
        if phase == RUN {
            let speed = self.me.move_speed(self.data, self.env, 9);
            let vspeed = if up { -speed } else { speed };
            match self.anim_id() - first {
                0 => {
                    if self.me.work[0] & 1 != 0 {
                        self.me.anim_set_duration(0.5);
                    }
                    self.me.anim_advance_then(first + 1, 0.15, 0);
                    self.me.move_vertical(0.0, kmh(100.0));
                }
                1 => {
                    self.me.anim_advance_loop(0);
                    self.me.move_vertical(vspeed, 10000.0);
                    self.me.set_fx(4);
                    self.set(9);
                }
                2 => {
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                    self.me.move_vertical(0.0, kmh(100.0));
                }
                _ => {}
            }
            self.step(6, 5, 3, 0.0, kmh(100.0));
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            match self.anim_id() - first {
                1 => {
                    let mut stop = false;
                    if up {
                        stop |= self.me.test(0x10, true);
                        let cost = self.move_ki_cost(2);
                        stop |= self.me.spend_ki(cost, false);
                        stop |= self.flag(0x15);
                    } else {
                        stop |= self.me.test(0x12, true);
                        let flags = self.me.pose.ground_flags;
                        if (flags & 0x4000_0000 == 0 || flags & 0x0100_0000 != 0)
                            && self.me.frames_to_ground() < 0.3 * 30.0
                        {
                            stop = true;
                        }
                        stop |= self.flag(0x16);
                        let cost = self.move_ki_cost(2);
                        stop |= self.me.spend_ki(cost, false);
                    }
                    if stop {
                        self.request_anim(first + 2, 0.15);
                    }
                }
                2 => {
                    if self.me.work[1] != 0 {
                        self.request(0xB);
                        self.decide_main(1);
                        let q = self.queued();
                        self.request(q);
                    }
                }
                _ => {}
            }
            self.decide_main(if up { 0x1400 } else { 0xC00 });
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0x38 (`BtlAct_GuardHandler`): guard poses 0xEF (middle), 0xF1 (action input 36) and 0xF3 (input 37),
    /// each with its recoil at +1, and 0xF5 after flag 0x67. Forced by flags 0x68 / 0x69 / 0x6A when a hit is
    /// guarded (it opens on the recoil 0xF2 / 0xF4 / 0xF0). A pose change needs three frames of the new input
    /// (work[2]); releasing the guard (input 34) ends the action. The head look and the voice are not ported.
    fn guard(&mut self, phase: u32) {
        if phase == ENTER {
            let (anim, blend) = if self.flag(0x68) {
                (0xF2, 0.0)
            } else if self.flag(0x69) {
                (0xF4, 0.0)
            } else if self.flag(0x6A) {
                (0xF0, 0.0)
            } else if self.me.test(0x24, true) {
                (0xF1, 0.15)
            } else if self.me.test(0x25, true) {
                (0xF3, 0.15)
            } else {
                (0xEF, 0.15)
            };
            self.play(anim, blend);
            self.me.turn_yaw(self.opp, 2, PI);
        }
        if phase == RUN {
            match self.anim_id() {
                0xEF | 0xF1 | 0xF3 => {
                    self.me.anim_advance_loop(0);
                }
                0xF0 | 0xF2 | 0xF4 => {
                    let back = self.anim_id() - 1;
                    self.me.anim_advance_then(back, 0.0, 0);
                    self.set(0x95);
                    self.set(0x41);
                }
                0xF5 => {
                    self.me.anim_advance_then(0xEF, 0.0, 0);
                    self.set(0x95);
                    self.set(0x41);
                }
                _ => {}
            }
            let accel = kmh(100.0);
            self.step(6, 5, 2, 0.0, accel);
            if !self.data.can_fly() && self.flag(0xE) && !self.flag(0x11) {
                self.me.move_vertical(accel, accel);
            } else {
                self.me.apply_gravity();
            }
            self.set(0xC9);
            self.set(0xD5);
            self.set(0x1C);
            if self.flag(0xF) && kmh(200.0) < crate::btl_chr::length(self.me.pose.moved) {
                self.me.set_fx(0x32);
            }
            if self.flag(0x11) && !self.flag(0xF) {
                self.held(0xE);
            }
            if self.flag(0xF) {
                self.clear(0xE);
            }
        }
        if phase == DECIDE {
            let anim = self.anim_id();
            if self.flag(0x66) {
                match anim {
                    0xEF | 0xF1 | 0xF3 => self.request_anim(anim + 1, 0.0),
                    0xF0 | 0xF2 | 0xF4 => self.request_anim(anim, 0.0),
                    0xF5 => self.request_anim(0xF0, 0.0),
                    _ => {}
                }
            } else if self.flag(0x67) {
                self.request_anim(0xF5, 0.0);
            } else {
                if matches!(anim, 0xEF | 0xF1 | 0xF3) {
                    if self.me.test(0x22, false) {
                        self.request(0xB);
                    }
                    let mut want = 0xEF;
                    if self.me.test(0x24, true) {
                        want = 0xF1;
                    }
                    if self.me.test(0x25, true) {
                        want = 0xF3;
                    }
                    if anim == want {
                        self.me.work[2] = 0;
                    } else {
                        self.me.work[2] += 1;
                    }
                    if self.me.work[2] >= 3 {
                        self.request_anim(want, 0.15);
                        self.me.work[2] = 0;
                    }
                }
                self.decide_main(0x100_0000);
                self.decide_attack(0x400);
                self.decide_common(6);
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// Action 0xC: the idle motion once over a second, then 0xB.
    fn idle_once(&mut self, phase: u32) {
        if phase == ENTER {
            let m = self.idle_motion();
            self.play(m, 0.15);
        }
        if phase == RUN {
            self.me.anim_set_duration(1.0);
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0x1C);
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
        }
    }

    /// Action 0xD: free movement, motions 2 -> 3 with the side layers 4..7 by the heading relative to the camera.
    fn walk(&mut self, phase: u32) {
        if phase == ENTER {
            let mut motion = 2;
            if !self.data.can_fly() && self.flag(0xE) && !self.flag(0x11) {
                motion = 0x18B;
            }
            self.play(motion, 0.15);
        }
        if phase == RUN {
            let speed = self
                .me
                .move_speed(self.data, self.env, usize::from(self.flag(0x11)));
            let (sub_neg, sub_pos) = match self.anim_id() {
                2 => {
                    self.me.anim_advance_then(3, 0.0, 0);
                    (4, 6)
                }
                3 => {
                    self.me.anim_advance_loop(0);
                    (5, 7)
                }
                0x18B => {
                    self.me.anim_advance_loop(0);
                    (-1, -1)
                }
                _ => (0, 0),
            };
            if sub_neg >= 0 && sub_pos >= 0 {
                let side = self.me.rot_y_rel_cam().sin();
                if 0.0 < side {
                    self.me.anim_set_sub_mix(side);
                    self.me.anim_play_sub(self.data, sub_pos);
                } else {
                    self.me.anim_set_sub_mix(-side);
                    self.me.anim_play_sub(self.data, sub_neg);
                }
            }
            let pitch_mode = if self.flag(0xE) && self.flag(5) { 2 } else { 5 };
            self.step(0, pitch_mode, 3, speed, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xC9);
            self.set(0x1C);
            self.set(0x25);
            self.me.request_orbit(20.0, 50.0);
            if self.flag(0xE) && self.flag(0xF) {
                self.clear(0xE);
            }
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
            if self.anim_id() == 0x18B {
                if self.flag(0x11) {
                    self.request_anim(3, 0.15);
                }
            } else if !self.data.can_fly() && !self.flag(0x11) && !self.flag(0xF) {
                self.request_anim(0x18B, 0.15);
            }
        }
        if phase == DECIDE {
            if self.me.test(1, false) {
                self.request(0xB);
            }
            self.decide_main(0x2298_1FEB);
            self.decide_attack(0x0800_040F);
            self.decide_common(0xF);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0xE: movement close to the opponent (flag 0x13), facing it: motions 8 / 9 by the direction of
    /// travel, side layers 0xA / 0xB; speed kind 2 scaled down on approach.
    fn close_walk(&mut self, phase: u32) {
        if phase == ENTER {
            let mut motion = if self.me.input.stick()[1] < 0.0 { 8 } else { 9 };
            if !self.data.can_fly() && self.flag(0xE) && !self.flag(0x11) {
                motion = 0x18B;
            }
            self.play(motion, 0.15);
            self.me.set_direction(0);
            self.me.work[6] = self.me.vel_dir_rel_cam().sin().to_bits() as i32;
        }
        if phase == RUN {
            if self.flag(0xE) && self.flag(0xF) {
                self.clear(0xE);
            }
            if self.anim_id() != 0x18B {
                let dir = self.me.vel_dir_rel_cam();
                if -deg(90.0) < dir && dir < deg(90.0) {
                    if self.anim_id() == 9 {
                        self.me.anim_request_keep(8, 0.15);
                    }
                } else if self.anim_id() == 8 {
                    self.me.anim_request_keep(9, 0.15);
                }
                let lean = crate::btl_chr::approach(
                    f32::from_bits(self.me.work[6] as u32),
                    dir.sin(),
                    8.0 / 30.0,
                );
                self.me.work[6] = lean.to_bits() as i32;
                if 0.0 < lean {
                    self.me.anim_set_sub_mix(lean);
                    self.me.anim_play_sub(self.data, 0xB);
                } else {
                    self.me.anim_set_sub_mix(-lean);
                    self.me.anim_play_sub(self.data, 0xA);
                }
            }
            self.me.anim_advance_loop(0);
            let base = self
                .me
                .move_speed(self.data, self.env, usize::from(self.flag(0x11)) + 2);
            let speed = scale_speed_by_approach(self.me, self.opp, base, 30.0);
            let dir_mode = u32::from(self.flag(0xE));
            self.step(2, 2, dir_mode, speed, kmh(100.0));
            self.me.apply_gravity();
            self.set(0x1C);
            self.me.request_orbit(10.0, 20.0);
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
            if self.anim_id() == 0x18B {
                if self.flag(0x11) {
                    let m = if self.me.input.stick()[1] < 0.0 { 8 } else { 9 };
                    self.request_anim(m, 0.15);
                }
            } else if !self.data.can_fly() && !self.flag(0x11) && !self.flag(0xF) {
                self.request_anim(0x18B, 0.15);
            }
        }
        if phase == DECIDE {
            if self.test(2) {
                self.request(0xB);
            }
            self.decide_main(0x229C_BFE3);
            self.decide_attack(0x0800_054F);
            self.decide_common(0xF);
            let q = self.queued();
            self.request(q);
        }
        if phase == LEAVE && self.requested() == 0x11 {
            self.me.turn_yaw(self.opp, 0, PI);
        }
    }

    /// Actions 0x11 / 0x13: in the air (motions 0x20 rising, 0x21 falling, 0x22 landing), then 0xB.
    fn jump_air(&mut self, phase: u32) {
        if phase == ENTER {
            let m = if 0.0 <= self.me.pose.fall_speed {
                0x21
            } else {
                0x20
            };
            self.play(m, 0.15);
            self.clear(0xE);
        }
        if phase == RUN {
            let mut speed = 0.0;
            let mut accel = kmh(50.0);
            let turn = self.me.turn_rate(self.data, 1);
            match self.current() {
                0x11 => speed = self.me.move_speed(self.data, self.env, 0xD),
                0x13 => speed = self.me.move_speed(self.data, self.env, 4),
                _ => {}
            }
            match self.anim_id() {
                0x20 => {
                    self.me.anim_advance_loop(0);
                    if 0.0 <= self.me.pose.fall_speed {
                        self.request_anim(0x21, 0.15);
                    }
                }
                0x21 => {
                    self.me.anim_advance_loop(0);
                    if self.me.frames_to_ground() < 3.0
                        && (!self.env.stage_4_or_27 || self.flag(7) || self.me.ground_y() < -10.0)
                    {
                        self.request_anim(0x22, 0.1);
                    }
                }
                0x22 => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    speed = 0.0;
                    accel = kmh(100.0);
                }
                _ => {}
            }
            let [x, y] = self.me.input.stick();
            speed *= (x * x + y * y).sqrt();
            self.me.turn_yaw(self.opp, 0, turn);
            self.me
                .turn_pitch(self.data, self.opp, self.opp_data, 5, PI);
            self.me.turn_model_yaw(deg(36.0), 0.3);
            self.me.set_direction(3);
            self.me.advance(speed, accel);
            self.me.apply_gravity();
            self.me.request_orbit(20.0, 50.0);
            self.set(0x1A);
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
        }
        if phase == DECIDE {
            if self.flag(0x16) || (self.env.stage_4_or_27 && self.flag(0x17)) || self.flag(0x11) {
                self.held(0xE);
                self.request(0xB);
            }
            let ascend = self.test(0xA);
            if ascend {
                let id = if self.data.can_fly() { 0x14 } else { 0x16 };
                self.request(id);
            }
            if self.data.can_fly() && self.test(0xC) {
                self.request(0x15);
            }
            let mask = if self.anim_id() != 0x22 {
                0x1C00
            } else {
                0x1C02
            };
            self.decide_main(mask);
            let q = self.queued();
            self.request(q);
            if self.test(7) {
                self.request(0xF);
                self.held(0xE);
            }
            if self.anim_id() != 0x22 {
                if self.test(0x2C) {
                    self.request(0x3F);
                }
                if self.decide_attack(0x7_0000) != 0 {
                    let q = self.queued();
                    self.request(q);
                }
            }
        }
    }

    /// Action 0x3A: motion 0xEE (a turn), then 0xB or the queue; leaving raises flag 0x33.
    fn turn(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xEE, 0.15);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xB);
                let q = self.queued();
                self.request(q);
            }
            self.step(3, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == LEAVE {
            self.set(0x33);
        }
    }

    /// Action 0x44: one rush step (motion 0x37 + step); the chain continues on the next input.
    fn rush(&mut self, phase: u32) {
        if phase == ENTER {
            if self.powered_skill(0x10) {
                if self.me.rush_step == 4 {
                    self.me.rush_step = 5;
                }
                if self.me.rush_step == 9 {
                    self.me.rush_step = 0;
                }
            }
            if self.flag(0x86) {
                if self.me.rush_step < 5 {
                    self.me.rush_step = 5;
                }
                self.clear(0x86);
            }
            let anim = self.me.rush_step + 0x37;
            self.me.rush_step = (self.me.rush_step + 1) % 10;
            self.play(anim, 0.1);
            begin_rise_to_opponent(self.me, self.opp, self.data, self.opp_data);
            self.me.pose.speed = crate::btl_hit::launch_speed(self.me, self.data, self.env);
        }
        if phase == RUN {
            let next = if self.flag(0xE) { 0x26 } else { 0 };
            match self.anim_id() {
                0x37..=0x3A | 0x3C..=0x3F => {
                    self.me.anim_advance_then(next, 0.3, 0);
                    self.set(0x40);
                }
                0x3B | 0x40 => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    if self.me.obj.frame < self.query_event(2, 0, 2) as f32 {
                        self.set(0x40);
                    }
                }
                0 | 0x26 => {
                    self.me.anim_advance(1);
                    if !self.flag(anim_flag::BLENDING) {
                        self.request(0xB);
                    }
                    self.set(0x87);
                }
                _ => {}
            }
            let mode = if self.flag(0x5B) || self.flag(0x5D) {
                2
            } else {
                6
            };
            self.step(mode, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            self.set(0x88);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE {
            match self.anim_id() {
                0x37..=0x3A | 0x3C..=0x3F => {
                    self.decide_main(0x2_2000);
                    self.decide_attack(0x20_1073);
                    if self.flag(anim_flag::END) {
                        let q = self.queued();
                        self.request(q);
                    }
                }
                0x3B => {
                    // The follow-ups of the fifth hit (attack records 0x7B / 0x7C, input 0x2A) are not ported.
                }
                0 | 0x26 => {
                    self.decide_main(0x2_2000);
                    self.decide_attack(0x20_1073);
                    let q = self.queued();
                    self.request(q);
                }
                _ => {}
            }
        }
    }

    /// Actions 0xBD, 0xC0..0xCA: hit reactions in place, one motion each (variants cycled).
    fn stagger(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, true);
            let anim = match self.current() {
                0xC0 => self.next_variant(4) + if air { 0xA2 } else { 0x9E },
                0xC1 => self.next_variant(4) + if air { 0xAA } else { 0xA6 },
                0xC2 => {
                    if air {
                        0xB1
                    } else {
                        0xB3
                    }
                }
                0xC3 => {
                    if air {
                        0xB1
                    } else {
                        0xB4
                    }
                }
                0xC4 => {
                    if air {
                        0xB1
                    } else {
                        0xB0
                    }
                }
                0xC5 => {
                    if air {
                        0xB1
                    } else {
                        0xB2
                    }
                }
                0xC6 => {
                    if air {
                        0xB1
                    } else {
                        self.next_variant(2) + 0xB5
                    }
                }
                0xC7 => 0xE0,
                0xC8 => 0xE1,
                0xC9 => 0xB8,
                0xCA => 0xDF,
                0xBD => 0x9C,
                _ => 0,
            };
            self.play(anim, 0.0);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                let next = self.idle_follow_up();
                self.request(next);
            }
            self.step(6, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.reaction_flags();
            self.set(0x41);
        }
        if phase == DECIDE {
            if self.flag(anim_flag::END) {
                let q = self.queued();
                self.request(q);
            }
            if self.flag(0x13) && self.decide_main(0x1000_0000) != 0 {
                let q = self.queued();
                self.request(q);
            }
            if self.decide_main(0x84_0000) != 0 {
                let q = self.queued();
                self.request(q);
            }
        }
        if phase == LEAVE {
            self.set(0x33);
            self.me.unk_ff0 = 15;
        }
    }

    /// Actions 0xCF..0xD1: pushed back at 500 km/h (0xD1 in place), then 0xDB on the ground or 0xD9 in the air.
    fn pushed_back(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, true);
            let mut anim = 0xC5;
            match self.current() {
                0xCF => {
                    anim = if air { 0xBC } else { 0xBB };
                    self.clear(0xE);
                }
                0xD0 => {
                    if air {
                        let p = &mut self.me.pose;
                        p.rot[1] = wrap_angle(p.rot[1] + PI);
                        p.yaw = wrap_angle(p.yaw + PI);
                        p.pitch = -p.pitch;
                    }
                    self.clear(0xE);
                }
                0xD1 => anim = if air { 0x17D } else { 0x17C },
                _ => {}
            }
            self.play(anim, 0.0);
        }
        if phase == RUN {
            self.me.anim_advance(0);
            let speed = match self.current() {
                0xCF | 0xD0 => kmh(500.0),
                _ => 0.0,
            };
            let dir = if self.me.is_air_motion(self.env, false) {
                3
            } else {
                5
            };
            self.step(6, 6, dir, speed, kmh(100.0));
            if self.me.height_above_ground() > 10.0 {
                self.me.brake_vertical();
            } else {
                self.me.fall();
            }
            self.reaction_flags();
        }
        if phase == DECIDE {
            if self.me.is_free() {
                if !self.flag(0x1E) {
                    self.add_action_bit(0x25);
                    self.add_action_bit(0x26);
                }
                if self.decide_main(0x1000_0000) != 0 {
                    let q = self.queued();
                    self.request(q);
                }
            }
            let end =
                self.flag(anim_flag::END) || (self.me.anim_progress() > 0.7 && !self.flag(0xF));
            if end {
                if self.flag(0xF) {
                    self.request(0xDB);
                } else {
                    self.request(0xD9);
                    if self.env.stage_4_or_27 && self.me.is_free() {
                        self.request(0xE6);
                    }
                }
                if self.me.is_free() && !self.flag(0x1E) {
                    if self.me.input.is_held(0x10) {
                        self.held(0xE);
                        self.request(0x26);
                    } else if self.me.input.is_held(0x20) {
                        let id = if self.flag(0xF) { 0x25 } else { 0xE6 };
                        self.request(id);
                    }
                }
            }
        }
    }

    /// Action 0xD2: motion 0xD9 in place, then the down action; off the ground past 40% it falls (0xD9).
    fn knocked_out(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xD9, 0.0);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.clear(0xE);
                let d = self.down_action();
                self.request(d);
            }
            if !self.flag(0xF) && self.me.anim_progress() > 0.4 {
                self.request(0xD9);
            }
            self.step(6, 6, 6, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.reaction_flags();
        }
    }

    /// Action 0xD3: motion 0xB7 looping until the stun countdown runs out.
    fn stun(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xB7, 0.0);
        }
        if phase == RUN {
            self.me.anim_advance_loop(0);
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.reaction_flags();
        }
        if phase == DECIDE && self.me.stun_timer <= 0 {
            let next = self.idle_follow_up();
            self.request(next);
        }
    }

    /// Action 0xE0: the same in the air (0xD2 / 0xD3); ends in the down action.
    fn air_stun(&mut self, phase: u32) {
        if phase == ENTER {
            let m = if self.me.is_air_motion(self.env, false) {
                0xD3
            } else {
                0xD2
            };
            self.play(m, 0.0);
        }
        if phase == RUN {
            self.me.anim_advance_loop(0);
            let accel = kmh(100.0);
            self.step(6, 6, 7, 0.0, accel);
            if self.flag(0x11) {
                self.me.move_vertical(kmh(30.0), accel);
            } else {
                self.me.fall();
            }
            self.reaction_flags();
            self.set(0x2A);
        }
        if phase == DECIDE && self.me.stun_timer <= 0 {
            let d = self.down_action();
            self.request(d);
        }
    }

    /// `BtlActB_PickDownAnim`.
    fn pick_down_anim(&self) -> i32 {
        let air = self.me.is_air_motion(self.env, false);
        if self.flag(0x11) {
            if air { 0xD8 } else { 0xD7 }
        } else if crate::btl_chr::length(self.me.pose.moved) > kmh(300.0) {
            if air { 0xD3 } else { 0xD2 }
        } else if air {
            0xD6
        } else {
            0xD5
        }
    }

    /// `BtlActB_TickMemberChange`: a knocked-out fighter with a member left alive is replaced after 30 frames.
    fn tick_member_change(&mut self) -> bool {
        if self.env.mode == 1 && !self.flag(0x10E) {
            return false;
        }
        // One member per side: nobody is left to take over.
        false
    }

    /// Action 0xD8: lying down; getting up after 3 s or on an input; a knocked-out member is replaced.
    fn lie_down(&mut self, phase: u32) {
        if phase == ENTER {
            let m = self.pick_down_anim();
            self.play(m, 0.15);
            if self.flag(0x11) {
                self.held(0xE);
            } else {
                self.clear(0xE);
            }
        }
        if phase == RUN {
            let mut accel = kmh(100.0);
            let mut down = false;
            self.me.anim_advance_loop(0);
            if !self.flag(anim_flag::BLENDING) {
                let next = self.pick_down_anim();
                if self.anim_id() != next {
                    self.request_anim(next, 0.15);
                }
            }
            match self.anim_id() {
                0xD5 | 0xD6 => {
                    self.me.fall();
                    down = true;
                }
                0xD2 | 0xD3 => {
                    accel = kmh(500.0) * (1.0 / 30.0);
                    self.me.fall();
                }
                0xD7 | 0xD8 => {
                    self.me.move_vertical(kmh(30.0), kmh(100.0));
                    down = true;
                }
                _ => {}
            }
            self.step(6, 5, 6, 0.0, accel);
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.me.gauge.low_health_idle {
                self.set(0x96);
                if self.me.is_free() {
                    self.set(0xE7);
                    if self.me.input.is_pressed(0x10_0000) {
                        self.me.mash_left -= 1;
                    }
                }
            }
            if self.me.is_dead() {
                self.set(0x137);
            }
            if down && self.tick_member_change() {
                self.request(0xF6);
            }
        }
        if phase == DECIDE {
            if !self.flag(0xF)
                && !self.me.flags.test_prev(0xF)
                && !self.flag(0x1F)
                && !self.flag(0x11)
                && self.data.height() * 0.3 < self.me.height_above_ground()
            {
                self.request(0xD9);
            }
            if self.me.is_free() {
                if self.me.action_frame > 90 {
                    self.request(if self.flag(0x11) { 0xE6 } else { 0xE1 });
                }
                if (!self.me.gauge.low_health_idle || self.me.mash_left <= 0)
                    && self.recovery_input(false)
                {
                    let q = self.queued();
                    self.request(q);
                }
                if self.env.stage_4_or_27 && self.flag(0x17) {
                    self.request(0xE6);
                }
            }
            if self.me.stun_timer > 0 {
                self.request(0xE0);
            }
        }
    }

    /// Action 0xD9: falling helplessly (0xC6 / 0xC7) until landing (0xDC).
    fn fall(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, true);
            let blend = if self.me.act.prev == 0xD5 { 0.3 } else { 0.15 };
            self.play(if air { 0xC7 } else { 0xC6 }, blend);
            self.clear(0xE);
        }
        if phase == RUN {
            self.me.anim_advance_loop(0);
            self.step(6, 5, 7, 0.0, kmh(500.0) * (1.0 / 30.0));
            self.me.fall();
            self.reaction_flags();
            if self.me.gauge.low_health_idle {
                self.set(0x96);
                if self.me.is_free() && !self.flag(0x1D) {
                    self.set(0xE7);
                    if self.me.input.is_pressed(0x10_0000) {
                        self.me.mash_left -= 1;
                    }
                }
            }
        }
        if phase == DECIDE {
            let land = if self.env.stage_4_or_27 {
                if self.flag(0xF) {
                    true
                } else {
                    if self.flag(0x17) && self.me.is_free() {
                        self.request(0xE6);
                    }
                    false
                }
            } else {
                self.me.frames_to_ground() < 2.0 || self.flag(0xF) || self.flag(0x1F)
            };
            if land {
                self.request(0xDC);
                if self.me.is_free() && !self.flag(0x1E) {
                    self.add_action_bit(0x25);
                    if self.me.input.is_held(DIR_MASK) {
                        self.request(if self.data.can_fly() { 0x27 } else { 0x25 });
                    }
                }
            }
            if self.flag(0x11) {
                let d = self.down_action();
                self.request(d);
            }
            if self.flag(0x16) && self.me.is_free() {
                self.request(0xE6);
            }
            if self.me.input.is_pressed(0x10_0000)
                && self.me.is_free()
                && !self.flag(0x1D)
                && (!self.me.gauge.low_health_idle || self.me.mash_left <= 0)
            {
                self.request(0xE6);
            }
        }
    }

    /// Actions 0xDA..0xDE: lying / hitting the ground after a knock-down.
    fn down(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, false);
            let mut clear = true;
            let anim = match self.current() {
                0xDA => {
                    clear = false;
                    if air { 0xBA } else { 0xB9 }
                }
                0xDB | 0xDE => {
                    if air {
                        0xCB
                    } else {
                        0xCA
                    }
                }
                0xDC => {
                    if air {
                        0xCD
                    } else {
                        0xCC
                    }
                }
                _ => {
                    if air {
                        0xCF
                    } else {
                        0xCE
                    }
                }
            };
            self.play(anim, 0.0);
            if clear {
                self.clear(0xE);
            }
            match self.current() {
                0xDB | 0xDC => self.me.set_fx(0x34),
                0xDD | 0xDE => self.me.set_fx(0x35),
                _ => {}
            }
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                match self.current() {
                    0xDA | 0xDB | 0xDE => {
                        let d = self.down_action();
                        self.request(d);
                    }
                    _ => self.request(0xDB),
                }
                if self.env.stage_4_or_27 && self.me.is_free() && self.flag(0x17) {
                    self.request(0xE6);
                }
            }
            self.step(6, 5, 6, 0.0, kmh(50.0) / 3.0);
            self.me.apply_gravity();
            self.reaction_flags();
            if self.me.gauge.low_health_idle && self.me.is_free() {
                self.set(0xE7);
                if self.me.input.is_pressed(0x10_0000) {
                    self.me.mash_left -= 1;
                }
            }
        }
        if phase == DECIDE
            && self.me.is_free()
            && (!self.me.gauge.low_health_idle || self.me.mash_left <= 0)
            && self.me.combo.hits >= 10
            && self.recovery_input(false)
        {
            let q = self.queued();
            self.request(q);
        }
    }

    /// Actions 0xE1..0xE5: getting up, then the idle follow-up.
    fn get_up(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, true);
            let anim = match self.current() {
                0xE1 => {
                    if air {
                        0xE3
                    } else {
                        0xE2
                    }
                }
                0xE2 => {
                    if air {
                        0xE5
                    } else {
                        0xE4
                    }
                }
                0xE3 => {
                    if air {
                        0xE7
                    } else {
                        0xE6
                    }
                }
                0xE5 => {
                    self.me.pose.yaw = wrap_angle(self.me.pose.yaw + PI);
                    0xFE
                }
                _ => 0xFE,
            };
            self.play(anim, 0.15);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                let next = self.idle_follow_up();
                self.request(next);
            }
            self.step(6, 5, 6, 0.0, kmh(50.0));
            self.me.apply_gravity();
            self.set(0x41);
            self.me.request_orbit(20.0, 50.0);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
    }

    /// Action 0xE6: motion 0xEA / 0xEB, turning to the opponent; past 40% the decisions run.
    fn recover(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, true);
            let blend = if self.me.act.prev == 0xD4 { 0.0 } else { 0.15 };
            let anim = if air { 0xEB } else { 0xEA };
            self.play(anim, blend);
            self.held(0xE);
            if self.flag(5) {
                let mode = if self.anim_flags(anim) & 0x8000 != 0 {
                    3
                } else {
                    2
                };
                self.me.turn_yaw(self.opp, mode, 2.0 * PI);
            }
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                let next = self.idle_follow_up();
                self.request(next);
            }
            self.step(6, 5, 7, 0.0, kmh(50.0));
            self.me.apply_gravity();
            self.set(0x41);
            self.set(0x95);
        }
        if phase == DECIDE && 0.4 < self.me.anim_progress() {
            if !self.has_queued() {
                self.decide_main(0x8_1C68);
                self.decide_attack(0xF);
                self.decide_common(6);
            }
            let q = self.queued();
            self.request(q);
        }
        if phase == LEAVE {
            self.set(0x33);
        }
    }

    /// `BtlAct_CheckRecoveryInput`: a direction or a face button gets a downed fighter up.
    fn recovery_input(&mut self, held: bool) -> bool {
        if !self.me.is_free() {
            return false;
        }
        let input = &self.me.input;
        let test = |held: bool, mask: u32| {
            if held {
                input.is_held(mask)
            } else {
                input.is_pressed(mask)
            }
        };
        let id = if self.flag(0x11) {
            (test(held, 0xF0) || test(held, 0x10_0000)).then_some(0xE6)
        } else {
            let button = input.is_pressed(0x10_0000);
            let held = held || button;
            let front = wrap_angle(self.me.pose.rot[1] - self.me.cam.yaw).abs() < 1.570_796_3;
            if test(held, 0x40) {
                Some(if front { 0xE2 } else { 0xE3 })
            } else if test(held, 0x80) {
                Some(if front { 0xE3 } else { 0xE2 })
            } else if test(held, 0x10) {
                Some(if front { 0xE5 } else { 0xE4 })
            } else if test(held, 0x20) {
                Some(if front { 0xE4 } else { 0xE5 })
            } else {
                button.then_some(0xE1)
            }
        };
        match id {
            Some(id) => {
                self.set_queue(0, id);
                true
            }
            None => false,
        }
    }
}

/// `BtlMove_CalcJumpSpeed`: the upward (negative) launch speed that rises `height` against 50 km/h per frame of
/// gravity.
fn jump_speed(mut height: f32) -> f32 {
    let mut v = 0.0f32;
    loop {
        height -= v;
        v += kmh(50.0);
        if height < 0.0 {
            v += (1.0 - -height / v) * kmh(50.0) - kmh(50.0);
            break;
        }
    }
    -v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slice_s_actions_are_ported_and_others_are_not() {
        for id in [
            0xB, 0xD, 0xF, 0x38, 0x44, 0xC0, 0xC6, 0xCF, 0xD2, 0xD8, 0xDB, 0xE1,
        ] {
            assert!(ported(id), "0x{id:X}");
        }
        for id in [0xAE, 0x60, 0xB8, 0xF6] {
            assert!(!ported(id), "0x{id:X}");
        }
    }
}
