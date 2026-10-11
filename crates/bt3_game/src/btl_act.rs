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

use crate::btl_blast::{blast_limit_a, can_fire, hits_of, ki_cost, ki_cost_of};
use crate::btl_chr::{
    Chr, Env, SideData, anim_flag, begin_rise_to_opponent, deg, is_blocked_by_opponent, kmh,
    length, normalize, opp_delta, opp_gap_xz, opp_yaw, opp_yaw_from_facing,
    scale_speed_by_approach, steer_at_opponent, sub, warp_behind_opponent,
};
use crate::btl_input::{DIR_MASK, wrap_angle};
use crate::btl_obj::mask_to_node;

const ENTER: u32 = 0;
const RUN: u32 = 1;
const DECIDE: u32 = 2;
const LEAVE: u32 = 3;

/// `BtlParam_GetAmountA` / `BtlParam_GetAmountB`: the ki of a vanishing step and of vanishing behind the
/// opponent (abilities 0x23 / 0x24 lower them; abilities are not ported).
const VANISH_KI: i32 = 10000;
const VANISH_BEHIND_KI: i32 = 20000;

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
        1..=3 | 0xB..=0x2A | 0x37 | 0x38 | 0x3A | 0x44 | 0x47..=0x52 | 0xAE..=0xB1
        | 0xBD | 0xC0..=0xD9 | 0xDA..=0xDF | 0xE0..=0xE6)
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
        if mask & 0x20000 != 0
            && self.flags2() & 8 != 0
            && self.flag(5)
            && self.me.has_ki(VANISH_KI)
        {
            for (input, action) in [(0x20, 0x23), (0x1D, 0x20), (0x1E, 0x21), (0x1F, 0x22)] {
                if self.test(input) {
                    self.set_queue(0, action);
                    return 1;
                }
            }
        }
        if mask & 0x40000 != 0
            && self.flag(5)
            && self.flag(0x13)
            && self.me.has_ki(VANISH_BEHIND_KI)
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
        // The ki blasts: ki for the kind, a shot left (blastShots) and room for one more live blast of the class.
        let limit = blast_limit_a(self.data);
        let room = |s: &Self, kind: i32, class: usize| {
            s.me.has_ki(ki_cost_of(s.data, kind))
                && s.me.blast_shots < limit
                && can_fire(s.me, s.data, class)
        };
        if mask & 4 != 0 && room(self, 0, 0) {
            self.add_action_bit(0xAE);
            if self.test(0x59) {
                if self.flag(5) && opp_yaw_from_facing(self.me, self.opp).abs() > 1.7 {
                    self.set_queue(0, 0x3A);
                    self.set_queue(1, 0xAE);
                    return 2;
                }
                self.set_queue(0, 0xAE);
                return 1;
            }
        }
        if mask & 8 != 0 && room(self, 3, 1) {
            self.add_action_bit(0xAF);
            if self.test(0x5A) {
                self.set_queue(0, 0xAF);
                return 1;
            }
        }
        let dash_blasts = self.flags2() & 0x10 != 0;
        if mask & 0x4000 != 0 && dash_blasts && room(self, 4, 0) {
            self.add_action_bit(0xB0);
            if self.test(0x59) {
                self.set_queue(0, 0xB0);
                return 1;
            }
        }
        if mask & 0x8000 != 0 && dash_blasts && room(self, 7, 1) {
            self.add_action_bit(0xB1);
            if self.test(0x5A) {
                self.set_queue(0, 0xB1);
                return 1;
            }
        }
        if mask & 0x1_0000 != 0 && self.test(0x40) {
            self.set_queue(0, 0x59);
            return 1;
        }
        if mask & 0x2_0000 != 0 && dash_blasts && room(self, 8, 0) {
            self.add_action_bit(0xB2);
            if self.test(0x59) {
                self.set_queue(0, 0xB2);
                return 1;
            }
        }
        if mask & 0x4_0000 != 0 && dash_blasts && room(self, 0xB, 1) {
            self.add_action_bit(0xB3);
            if self.test(0x5A) {
                self.set_queue(0, 0xB3);
                return 1;
            }
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
            0x19 => self.homing_dash(phase),
            0x1A => self.dragon_dash(phase),
            0x1B..=0x1D => self.side_step(phase),
            0x1E | 0x1F => self.step_in(phase),
            0x20..=0x23 => self.vanish_step(phase),
            0x24 => self.vanish_behind(phase),
            0x25..=0x2A => self.recover_air(phase),
            0x47..=0x4C => self.smash_charge(phase),
            0x4D..=0x52 => self.smash_full(phase),
            0xCB | 0xCC => self.reel(phase),
            0xCD => self.ground_bounce(phase),
            0xCE => self.launched(phase),
            0xD4 => self.knock_back(phase),
            0xD5 | 0xD6 | 0xDF => self.blow_away(phase),
            0xD7 => self.blow_recover(phase),
            0x37 => self.charge(phase),
            0x38 => self.guard(phase),
            0xAE => self.ki_blast(phase),
            0xAF => self.charged_ki_blast(phase),
            0xB0 => self.dash_ki_blast(phase),
            0xB1 => self.dash_charged_ki_blast(phase),
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
            self.me.charge_timer = 0.0;
            self.me.charge_full_frames = 0;
            self.me.blast_charge = 0.0;
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

    /// `BtlMoveParam_GetTurnAccel` / `GetTurnMax`: tables +0x80 (degrees per second) and +0x90 (degrees).
    fn turn_accel(&self, n: usize) -> f32 {
        self.data.tables.moves.turn_accel[n] * PI / 180.0 * (1.0 / 30.0)
    }

    fn turn_max(&self, n: usize) -> f32 {
        self.data.tables.moves.turn_max[n] * PI / 180.0
    }

    /// Action 0x19 (`BtlAct_HomingDashHandler`): the ki-draining dash that homes on the opponent, motions 0x18 /
    /// 0x2D (start) -> 0x19 (loop) -> 0x1A or 0x15 (stop). work[2] counts slow frames. The voices, sounds and
    /// vibration are not ported.
    fn homing_dash(&mut self, phase: u32) {
        if phase == ENTER {
            let prev = self.me.act.prev;
            let motion = if (0x70..0x70 + 0x3E).contains(&prev) {
                self.me.dash_count += 1;
                0x19
            } else {
                match prev {
                    0x47..=0x57 | 0x5A..=0x5D | 0x6B..=0x6F => {
                        self.me.dash_count += 1;
                        0x19
                    }
                    0x1A | 0x28 | 0x2A | 0xD7 => 0x19,
                    0x17 | 0x18 => 0x2D,
                    _ => 0x18,
                }
            };
            self.play(motion, 0.15);
            self.held(0xE);
        }
        if phase == RUN {
            let (mut lean, mut moving, mut face_opp, mut first) = (0.0, true, true, false);
            match self.anim_id() {
                0x18 | 0x2D => {
                    self.me.anim_advance_then(0x19, 0.2, 0);
                    lean = self.me.anim_progress();
                    moving = false;
                    if 0.5 < self.me.anim_progress() {
                        self.set(0x47);
                    }
                }
                0x19 => {
                    self.me.anim_advance_loop(0);
                    if self.me.anim_is_new() {
                        first = true;
                        self.set(0xD1);
                        if self.me.height_above_ground() < 20.0 {
                            self.me.set_fx(0x35);
                        }
                    }
                    self.set(0x47);
                    lean = 1.0;
                }
                0x15 | 0x1A => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                        self.me.pose.speed = 0.0;
                    }
                    moving = false;
                    face_opp = false;
                    lean = 1.0 - self.me.anim_progress();
                }
                _ => {}
            }
            if moving {
                let speed = self.me.move_speed(self.data, self.env, 6);
                let (mut yaw_accel, mut pitch_accel) = (self.turn_accel(0), self.turn_accel(1));
                let (yaw_max, pitch_max) = (self.turn_max(0), self.turn_max(1));
                let mut model_turn = deg(36.0);
                if first {
                    pitch_accel = PI;
                    yaw_accel = PI;
                    model_turn = PI;
                }
                let near =
                    (opp_gap_xz(self.me, self.opp, self.data, self.opp_data) * 0.01).min(1.0);
                steer_at_opponent(
                    self.me,
                    self.opp,
                    self.data,
                    self.opp_data,
                    speed,
                    yaw_accel,
                    pitch_accel,
                    yaw_max * near,
                    pitch_max * near,
                    PI,
                );
                self.me.turn_model_yaw(model_turn, 0.5);
                self.me.set_direction(3);
                self.me.advance(speed, 10000.0);
                self.me.apply_gravity();
                self.set(9);
                self.set(0x50);
                self.set(0x51);
                self.me.set_fx(4);
                self.me.set_fx(9);
            } else {
                if self.blocked() {
                    self.me.pose.speed = 0.0;
                }
                self.step(if face_opp { 2 } else { 6 }, 6, 7, 0.0, kmh(100.0));
                self.me.apply_gravity();
            }
            self.set(0x89);
            let pitch = self.me.pose.pitch;
            self.me.set_lean_x(pitch * lean);
            self.me.request_orbit(50.0, 100.0);
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            match self.anim_id() {
                0x19 => {
                    let mut stop = self.blocked() && self.opp_seen_action() != 0xD5;
                    if crate::btl_chr::length(self.me.pose.moved) < kmh(100.0) {
                        self.me.work[2] += 1;
                        if self.me.work[2] >= 16 {
                            stop = true;
                        }
                    } else {
                        self.me.work[2] = 0;
                    }
                    if !self.flag(5) {
                        stop = true;
                    }
                    let cost = self.move_ki_cost(0);
                    stop |= self.me.spend_ki(cost, false);
                    if self.me.test(0x16, true) {
                        self.request_anim(0x15, 0.0);
                    }
                    if self.me.test(0x15, true) {
                        self.request_anim(0x15, 0.0);
                        self.me.work[0] |= 1;
                    }
                    if self.decide_main(0x800_0000) != 0 {
                        let q = self.queued();
                        self.request(q);
                    }
                    if stop {
                        self.request_anim(0x1A, 0.0);
                        self.me.pose.speed *= 0.5;
                    }
                }
                0x1A => {
                    if self.blocked() {
                        self.me.pose.speed = 0.0;
                    }
                }
                0x15 => {
                    if self.me.test(0x18, true) {
                        self.me.work[0] |= 1;
                    }
                    if 0.5 < self.me.anim_progress() && self.me.work[0] & 1 != 0 {
                        self.request(0x1A);
                    }
                    if self.decide_main(0x800_0000) != 0 {
                        let q = self.queued();
                        self.request(q);
                    }
                }
                _ => {}
            }
            self.decide_main(0x1800);
            match self.anim_id() {
                0x19 => {
                    self.decide_attack(0x2000);
                }
                0x1A => {
                    self.decide_attack(3);
                }
                _ => {}
            }
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0x1A (`BtlAct_DragonDashHandler`): the dash in the direction held, motions 0x18 / 0x2D (start) ->
    /// 0x19 (loop) -> 0x1A or 0x15 (stop); ASCEND / DESCEND move up and down. It stops when blocked, slower than
    /// 100 km/h for 16 frames (work[2]) or out of ki. The sounds and vibration are not ported.
    fn dragon_dash(&mut self, phase: u32) {
        if phase == ENTER {
            let mut anim = if matches!(self.me.act.prev, 0x19 | 0x1A) {
                0x19
            } else {
                0x2D
            };
            if self.me.unk_ff0 > 0 {
                anim = 0x18;
            }
            self.play(anim, 0.15);
            self.held(0xE);
            self.me.turn_yaw(self.opp, 0, PI);
        }
        if phase == RUN {
            let mut moving = true;
            match self.anim_id() {
                0x18 | 0x2D => {
                    self.me.anim_advance_then(0x19, 0.2, 0);
                    moving = false;
                }
                0x19 => self.me.anim_advance_loop(0),
                0x15 | 0x1A => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                        self.me.pose.speed = 0.0;
                    }
                    moving = false;
                }
                _ => {}
            }
            if moving {
                let speed = self.me.move_speed(self.data, self.env, 7);
                let turn = self.me.turn_rate(self.data, 2);
                let vspeed = self.me.move_speed(self.data, self.env, 8);
                self.me.turn_yaw(self.opp, 0, turn);
                self.me
                    .turn_pitch(self.data, self.opp, self.opp_data, 5, PI);
                self.me.turn_model_yaw(deg(36.0), 0.3);
                self.me.set_direction(3);
                self.me.advance(speed, 10000.0);
                if self.me.input.is_held(crate::btl_input::ASCEND) {
                    self.me.move_vertical(-vspeed, kmh(100.0));
                } else if self.me.input.is_held(crate::btl_input::DESCEND) {
                    self.me.move_vertical(vspeed, kmh(100.0));
                } else {
                    self.me.apply_gravity();
                }
                self.set(9);
                self.set(0x50);
                self.set(0x51);
                self.me.set_fx(4);
            } else {
                self.step(6, 5, 3, 0.0, kmh(100.0));
                self.me.apply_gravity();
            }
            self.me.request_orbit(50.0, 100.0);
            self.set(0x1A);
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            match self.anim_id() {
                0x19 => {
                    let mut stop = self.blocked();
                    if crate::btl_chr::length(self.me.pose.moved) < kmh(100.0) {
                        self.me.work[2] += 1;
                        if self.me.work[2] >= 0x10 {
                            stop = true;
                        }
                    } else {
                        self.me.work[2] = 0;
                    }
                    let cost = self.move_ki_cost(1);
                    stop |= self.me.spend_ki(cost, false);
                    if self.me.test(0x16, true) {
                        self.request_anim(0x15, 0.0);
                    }
                    if self.me.test(0x14, true) {
                        self.request_anim(0x15, 0.0);
                        self.me.work[0] |= 1;
                    }
                    if self.me.test(0x15, true) {
                        self.request_anim(0x15, 0.0);
                        self.me.work[0] |= 2;
                    }
                    if stop {
                        self.request_anim(0x1A, 0.0);
                        self.me.pose.speed *= 0.5;
                    }
                }
                0x1A => {
                    if self.blocked() {
                        self.me.pose.speed = 0.0;
                    }
                }
                0x15 => {
                    if self.me.test(0x17, true) {
                        self.me.work[0] |= 1;
                    }
                    if self.me.test(0x18, true) {
                        self.me.work[0] |= 2;
                    }
                    if self.me.anim_progress() > 0.5 {
                        if self.me.work[0] & 1 != 0 && self.flag(5) {
                            self.request(0x19);
                        }
                        if self.me.work[0] & 2 != 0 {
                            self.request(0x1A);
                        }
                    }
                }
                _ => {}
            }
            self.decide_main(0x1800);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Actions 0x1B (back), 0x1C / 0x1D (left / right of the line to the opponent) (`BtlAct_StepHandler`): a step,
    /// motions 0x1C..0x1E on the ground, 0x28..0x2A in the air. It moves between 10 % and 60 % of the motion. The
    /// sounds and voices are not ported.
    fn side_step(&mut self, phase: u32) {
        if phase == ENTER {
            if self.me.height_above_ground() < 5.0 {
                self.clear(0xE);
            }
            let air = self.flag(0xE);
            let mut yaw = opp_yaw(self.me, self.opp);
            let anim = match self.current() {
                0x1B => {
                    yaw = wrap_angle(yaw + PI);
                    if air { 0x28 } else { 0x1C }
                }
                0x1C => {
                    yaw = wrap_angle(yaw - PI / 2.0);
                    if air { 0x29 } else { 0x1D }
                }
                _ => {
                    yaw = wrap_angle(yaw + PI / 2.0);
                    if air { 0x2A } else { 0x1E }
                }
            };
            self.play(anim, 0.15);
            self.me.pose.dir = [yaw.sin(), 0.0, yaw.cos()];
        }
        if phase == RUN {
            let progress = self.me.anim_progress();
            if self.me.anim_advance(0) {
                self.me.work[1] = 1;
            }
            if self.me.anim_progress() < 0.2 {
                self.set(0x42);
            }
            let window = 0.1 < progress && progress < 0.6;
            let speed = match self.anim_id() {
                0x1C | 0x28 if window => self.me.move_speed(self.data, self.env, 0xB),
                0x1D | 0x1E | 0x29 | 0x2A if window => {
                    let speed = self.me.move_speed(self.data, self.env, 0xC);
                    let frames = self.me.obj.layers[0].length / self.me.obj.step;
                    scale_speed_by_dist(self.me, self.opp, speed, PI / 2.0, frames * (0.6 - 0.1))
                }
                _ => 0.0,
            };
            self.step(2, 5, 6, speed, kmh(100.0));
            self.me.apply_gravity();
            self.set(0x88);
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
            self.me.request_orbit(5.0, 10.0);
        }
        if phase == DECIDE {
            // BtlParam_GetBackStepCancelRatio (+0x6C) / BtlParam_GetStepCancelRatio (+0x68).
            let p = &self.data.tables.params;
            let cancel = if self.anim_id() == 0x1C {
                p.f32_at(0x6C)
            } else {
                p.f32_at(0x68)
            };
            if self.me.work[1] != 0 {
                self.request(0xB);
                let mask = if self.me.input.is_held(crate::btl_input::DOWN) {
                    0x11
                } else {
                    1
                };
                self.decide_main(mask);
                let q = self.queued();
                self.request(q);
            }
            if !self.has_queued() && self.me.anim_progress() > 0.4 {
                self.decide_main(0x1_4000);
                self.decide_attack(0x83);
            }
            if cancel < self.me.anim_progress() {
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// Actions 0x1E, 0x1F (`BtlAct_StepInHandler`): the short forward dash, motion 0x1B, fast until 70% of it.
    /// 0x1E reads attack inputs; 0x1F queues the attack records 0x8F, 0x8D, 0x8E (the records are not ported).
    fn step_in(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x1B, 0.15);
            self.held(0xE);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            if self.me.anim_progress() < 0.3 {
                self.set(0x42);
            }
            let (speed, accel) = if self.me.anim_progress() < 0.7 {
                (self.me.move_speed(self.data, self.env, 0xA), 10000.0)
            } else {
                (0.0, kmh(100.0))
            };
            self.step(2, 1, 3, speed, accel);
            self.me.apply_gravity();
            self.set(0x88);
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.me.gauge.low_health_idle {
                self.set(0x96);
            }
        }
        if phase == DECIDE {
            match self.current() {
                0x1E => {
                    self.decide_attack(0x1C0_0803);
                    if self.me.anim_progress() > 0.5 {
                        let q = self.queued();
                        self.request(q);
                    }
                    if self.decide_attack(0x40) != 0 {
                        let q = self.queued();
                        self.request(q);
                    }
                }
                _ => {
                    self.decide_attack(0x200_0030);
                    // BtlDecide_QueueAttack(0x8F / 0x8D / 0x8E): the attack records are not ported.
                    if self.me.anim_progress() > 0.3 {
                        let q = self.queued();
                        self.request(q);
                    }
                }
            }
        }
    }

    /// Actions 0x20..0x23 (`BtlAct_VanishStepHandler`): the vanishing step, motion 0xF6, then seven frames of fast
    /// movement back (0x20), left (0x21), right (0x22) or along the line to the opponent (0x23), untouchable
    /// (flags 0x42..0x46). Raised by flags 0x74..0x76 (a counter), it starts at once. The sound, the voice, battle
    /// event 0x41, frame bit 0x8000 and the stage timer of stages 4 / 27 are not ported.
    fn vanish_step(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xF6, 0.15);
            if self.me.act.prev == 0x44 {
                self.held(0x83);
                self.me.spend_ki(VANISH_KI, false);
            }
            if self.flag(0x74) || self.flag(0x75) || self.flag(0x76) {
                self.me.work[0] |= 1;
                self.set(0x12C);
            }
            self.me.work[2] = 7;
            self.me.set_fx(0xC);
            self.held(0xE);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            if self.flag(0x78) {
                self.me.work[0] |= 2;
            }
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.me.work[0] |= 1;
            }
            if self.me.work[0] & 1 == 0 {
                self.step(2, 5, 7, 0.0, kmh(100.0));
                self.me.apply_gravity();
            } else {
                if self.me.work[2] == 7 {
                    let delta = normalize(opp_delta(self.me, self.opp));
                    let yaw = -opp_yaw(self.me, self.opp);
                    let mut dir = [0.0f32; 3];
                    match self.current() {
                        0x21 => dir[0] = -1.0,
                        0x22 => dir[0] = 1.0,
                        0x23 => {
                            dir[1] = delta[1];
                            dir[2] = (1.0f32 - delta[1] * delta[1]).sqrt();
                        }
                        _ => dir[2] = -1.0,
                    }
                    let dir = normalize(dir);
                    let (s, c) = yaw.sin_cos();
                    self.me.pose.dir = [dir[0] * c - dir[2] * s, dir[1], dir[0] * s + dir[2] * c];
                }
                self.me.work[2] -= 1;
                let counter = self.me.work[2];
                let speed = if counter >= 2 {
                    let speed = self.me.move_speed(self.data, self.env, 0x12);
                    scale_speed_by_dist(self.me, self.opp, speed, PI, 6.0)
                } else {
                    if counter == 1 {
                        self.me.set_fx(0xD);
                    }
                    0.0
                };
                self.step(2, 5, 7, speed, 10000.0);
                self.me.apply_gravity();
                // Unless flag 0x78 was up at the start, the step keeps up with the opponent.
                if self.me.work[0] & 2 == 0 && self.flag(5) {
                    let moved = self.opp.pose.moved;
                    for (p, m) in self.me.pose.pos.iter_mut().zip(moved) {
                        *p += m;
                    }
                }
                self.set(0xB);
            }
            self.me.request_orbit(5.0, 10.0);
            self.set(0x88);
            self.me.set_fx(0x3A);
            for f in 0x42..=0x46 {
                self.set(f);
            }
            if self.flag(0x83) {
                self.set(0x85);
            }
        }
        if phase == DECIDE {
            if !self.has_queued() {
                self.decide_attack(3);
            }
            if self.me.work[2] <= 0 {
                self.request(0xB);
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// Action 0x24 (`BtlAct_VanishBehindHandler`): vanish (motion 0xF6), then reappear behind the opponent facing
    /// it, for 20000 ki. The sound, the voice and the vibration are not ported.
    fn vanish_behind(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xF6, 0.15);
            self.me.spend_ki(VANISH_BEHIND_KI, false);
            self.me.set_fx(0xC);
            self.held(0xE);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            self.me.work[2] = 7;
        }
        if phase == RUN {
            if !self.me.anim_advance(0) {
                self.step(2, 5, 7, 0.0, kmh(100.0));
                self.me.apply_gravity();
            } else {
                self.me.work[2] -= 1;
                let counter = self.me.work[2];
                if counter >= 2 {
                    self.me.pose.speed = 0.0;
                    self.me.pose.fall_speed = 0.0;
                } else if counter == 1 {
                    self.me.set_fx(0xD);
                    if !self.flag(0xBA) {
                        warp_behind_opponent(self.me, self.data, self.opp, self.opp_data, 0.0);
                        self.me.turn_yaw(self.opp, 2, PI);
                        self.me
                            .turn_pitch(self.data, self.opp, self.opp_data, 1, PI);
                        self.me.turn_model_yaw(PI, 1.0);
                        self.me.set_direction(3);
                        for f in [0x3F, 0xB, 0xCC, 0x24] {
                            self.set(f);
                        }
                    }
                }
                self.set(0xB);
            }
            for f in 0x42..=0x46 {
                self.set(f);
            }
        }
        if phase == DECIDE {
            if !self.has_queued() {
                self.decide_attack(3);
            }
            if self.me.work[2] <= 0 {
                self.request(0xB);
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// Actions 0x25..0x2A (`BtlAct_RecoverHandler`): recovering in the air (motions 0xFE..0x102) for the ki of the
    /// parameter's +0x48. 0x29 and 0x2A turn around; 0x26 goes on to the dash 0xF, 0x28 and 0x2A to the homing
    /// dash 0x19, the others to 0xB. The sound is not ported.
    fn recover_air(&mut self, phase: u32) {
        if phase == ENTER {
            if self.me.is_air_motion(self.env, false) {
                self.me.pose.yaw = wrap_angle(self.me.pose.yaw + PI);
            }
            let anim = match self.current() {
                0x27 => 0xFF,
                0x29 => 0x100,
                0x28 => 0x101,
                0x2A => 0x102,
                _ => 0xFE,
            };
            if matches!(self.current(), 0x29 | 0x2A) {
                self.me.pose.speed = 0.0;
                self.me.pose.yaw = wrap_angle(self.me.pose.yaw + PI);
                self.me.pose.rot[1] = self.me.pose.yaw;
            }
            self.play(anim, 0.0);
            self.held(0xE);
            let cost = self.data.tables.params.s32_at(0x48);
            self.me.spend_ki(cost, false);
        }
        if phase == RUN {
            match self.current() {
                0x26 => {
                    self.me.anim_advance(0);
                    if self.me.anim_progress() > 0.4 {
                        self.request(0xF);
                    }
                }
                0x28 | 0x2A => {
                    if self.me.anim_advance(0) {
                        self.request(0x19);
                    }
                }
                _ => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                }
            }
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == LEAVE {
            self.set(0x33);
        }
    }

    // ---- the smashes (btl_act_1.c) ---------------------------------------------------------------------------

    /// `workF[k]`: the float scratch shares its words with `work[6 + k]`.
    fn work_f(&self, k: usize) -> f32 {
        f32::from_bits(self.me.work[6 + k] as u32)
    }

    fn set_work_f(&mut self, k: usize, v: f32) {
        self.me.work[6 + k] = v.to_bits() as i32;
    }

    /// The character's two charge rates (parameters +0x74 / +0x78) blended by the gauge at +0xD80.
    fn blended_charge_rate(&self) -> f32 {
        let t = self.me.charge_gauge as f32 / 100_000.0;
        let p = &self.data.tables.params;
        (1.0 - t) * p.f32_at(0x74) + t * p.f32_at(0x78)
    }

    /// `BtlAct_GetChargeRate`: at least 0.1 (ability 0x4A is not ported).
    fn charge_rate(&self) -> f32 {
        self.blended_charge_rate().max(0.1)
    }

    /// `BtlAct_StepCharge`: one frame of the charge timer; true once the charge level reached 1.
    fn step_charge(&mut self) -> bool {
        let rate = self.charge_rate();
        self.me.charge_timer = (self.me.charge_timer + rate * (1.0 / 22.5)).min(1.0);
        self.me.charge = if self.me.charge_timer < 1.0 {
            self.me.charge_timer / rate
        } else {
            1.0
        };
        1.0 <= self.me.charge
    }

    /// `BtlAct_SetChargeAnimDuration`: 1.5 s at rate 1, at least 0.75 s.
    fn set_charge_anim_duration(&mut self) {
        let d = (1.5 / self.charge_rate()).max(0.75);
        self.me.anim_set_duration(d);
    }

    /// `BtlAct_SetChargeAnimStep`: twice the blended rate.
    fn set_charge_anim_step(&mut self) {
        let step = self.blended_charge_rate() * 2.0;
        self.me.anim_set_step(step);
    }

    /// `BtlAct_CancelOnOppC9`: under flag 0x5D with the opponent seen in 0xC9, a follow-up started past `after`.
    fn cancel_on_opp_c9(&mut self, after: f32) {
        if self.flag(0x5D) && self.opp_seen_action() == 0xC9 {
            if !self.has_queued() {
                self.decide_attack(1);
            }
            if after < self.me.anim_progress() {
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// The release motion's follow-up after a connecting smash (flag 0x5B): attack inputs (or, against a
    /// knock-back, main input 8), started past `cancelAt`.
    fn smash_follow_up(&mut self, knock_back_too: bool) {
        if !self.flag(0x5B) {
            return;
        }
        self.set(0x3D);
        let seen = self.opp_seen_action();
        if !self.has_queued() {
            if seen == 0xD5 {
                if self.decide_attack(0x18_0000) != 0 {
                    self.set_work_f(0, 0.7);
                }
            } else if knock_back_too {
                if seen == 0xD4 {
                    if self.decide_main(8) != 0 {
                        self.set_work_f(0, 0.7);
                    }
                } else if self.decide_attack(1) != 0 {
                    self.set_work_f(0, 0.6);
                }
            }
        }
        if self.has_queued() && self.work_f(0) < self.me.anim_progress() {
            let q = self.queued();
            self.request(q);
        }
    }

    /// Actions 0x47..0x4C (`BtlAct_SmashChargeHandler`): the charged smash, motions wind-up -> hold -> release
    /// (0x50.., 0x53.., 0x56.., 0x59.., 0x4A.., 0x4D..). Holding charges it (the rate of the character, faster with
    /// the gauge); a full charge held one frame becomes the instant smash (flag 0x84: seven untouchable frames at
    /// 3500 km/h), held longer the full smash 0x4D..0x52 under powered skill 0x200. The sounds, the voice and the
    /// vibration are not ported.
    fn smash_charge(&mut self, phase: u32) {
        if phase == ENTER {
            let anim = match self.current() {
                0x47 => 0x50,
                0x48 => 0x53,
                0x49 => 0x56,
                0x4A => 0x59,
                0x4B => 0x4A,
                _ => 0x4D,
            };
            self.play(anim, 0.15);
            begin_rise_to_opponent(self.me, self.opp, self.data, self.opp_data);
        }
        if phase == RUN {
            let mut speed = 0.0;
            let mut accel = kmh(100.0);
            let mut turn = speed;
            let mut yaw_mode = 6;
            let mut pitch_mode = 5;
            let mut charging = false;
            if self.flag(0x81) {
                speed = kmh(50.0);
                yaw_mode = 2;
                turn = deg(1.8);
            }
            let id = self.anim_id();
            match id {
                0x4A | 0x4D | 0x50 | 0x53 | 0x56 | 0x59 => {
                    self.set_charge_anim_step();
                    if !self.flag(0x84) {
                        self.me.anim_advance_then(id + 1, 0.0, 0);
                    } else {
                        self.me.anim_advance(0);
                    }
                    charging = true;
                }
                0x4B | 0x4E | 0x51 | 0x54 | 0x57 | 0x5A => {
                    self.set_charge_anim_duration();
                    if !self.flag(0x84) {
                        if self.me.anim_advance(0) {
                            self.me.work[0] |= 1;
                        }
                    } else {
                        self.me.anim_advance(0);
                    }
                    charging = true;
                }
                0x4C | 0x4F | 0x52 | 0x55 | 0x58 | 0x5B => {
                    if self.me.anim_is_new() {
                        self.me.pose.speed =
                            crate::btl_hit::launch_speed(self.me, self.data, self.env);
                    }
                    self.me.obj.rate = self.me.charge + 1.0;
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    speed = 0.0;
                    yaw_mode = 6;
                    turn = speed;
                    if self.blocked() && (self.flag(0x5B) || self.flag(0x5D)) {
                        turn = PI;
                        yaw_mode = 2;
                    }
                }
                _ => {}
            }
            if charging {
                if !self.flag(0x84) {
                    if self.step_charge() {
                        self.me.set_fx(0x1E);
                        self.me.charge_full_frames += 1;
                    }
                } else {
                    let frames = self.me.work[2];
                    if frames == 0 {
                        self.me.set_fx(0xC);
                    }
                    if frames >= 2 {
                        self.set(0xB);
                        speed = kmh(3500.0);
                        yaw_mode = 2;
                        accel = 10000.0;
                        pitch_mode = 4;
                        turn = PI;
                    }
                    if frames >= 6 {
                        self.request_anim(id + 1, 0.0);
                    }
                    self.me.work[2] += 1;
                    for f in 0x42..=0x46 {
                        self.set(f);
                    }
                }
            }
            self.me.turn_yaw(self.opp, yaw_mode, turn);
            self.me
                .turn_pitch(self.data, self.opp, self.opp_data, pitch_mode, PI);
            self.me.turn_model_yaw(deg(36.0), 0.3);
            self.me.set_direction(3);
            self.me.advance(speed, accel);
            self.me.apply_gravity();
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE {
            let id = self.anim_id();
            match id {
                0x4A | 0x4B | 0x4D | 0x4E | 0x50 | 0x51 | 0x53 | 0x54 | 0x56 | 0x57 | 0x59
                | 0x5A => {
                    if !self.flag(0x84) {
                        // Input 0x3F: the smash button let go.
                        if self.test(0x3F) {
                            self.me.work[0] |= 1;
                        }
                        if self.me.work[0] & 1 != 0 {
                            let full = self.me.charge_full_frames;
                            if full == 1 {
                                self.held(0x84);
                            } else if full > 0 && self.powered_skill(0x200) && self.flag(0x13) {
                                let next = match self.current() {
                                    0x49 => 0x4F,
                                    0x4A => 0x50,
                                    0x47 => 0x4D,
                                    0x48 => 0x4E,
                                    0x4C => 0x52,
                                    _ => 0x51,
                                };
                                self.request(next);
                            } else {
                                self.me.work[0] |= 2;
                            }
                        }
                        if self.me.work[0] & 2 != 0 {
                            self.request_anim(id + 1, 0.0);
                        }
                    }
                }
                0x4C | 0x4F | 0x52 | 0x55 | 0x58 | 0x5B => {
                    self.smash_follow_up(true);
                    self.cancel_on_opp_c9(0.6);
                }
                _ => {}
            }
        }
    }

    /// Actions 0x4D..0x52 (`BtlAct_SmashFullHandler`): the fully charged smash, the wind-up and hold played fast
    /// with hit-stop flag 0x125. The fighter camera cut (`ChrCam_RequestCut`), the voice, the sound and the
    /// vibration are not ported.
    fn smash_full(&mut self, phase: u32) {
        if phase == ENTER {
            let anim = match self.current() {
                0x4D => 0x50,
                0x4E => 0x53,
                0x4F => 0x56,
                0x50 => 0x59,
                0x51 => 0x4A,
                _ => 0x4D,
            };
            self.play(anim, 0.0);
            begin_rise_to_opponent(self.me, self.opp, self.data, self.opp_data);
        }
        if phase == RUN {
            let mut mode = 2;
            let id = self.anim_id();
            match id {
                0x4A | 0x4B | 0x4D | 0x4E | 0x50 | 0x51 | 0x53 | 0x54 | 0x56 | 0x57 | 0x59
                | 0x5A => {
                    if matches!(id, 0x4B | 0x4E | 0x51 | 0x54 | 0x57 | 0x5A) {
                        self.me.anim_set_duration(0.2);
                    }
                    self.me.anim_advance_then(id + 1, 0.0, 0);
                    self.set(0x125);
                    self.set(0x12A);
                    self.me.charge = 1.0;
                }
                0x4C | 0x4F | 0x52 | 0x55 | 0x58 | 0x5B => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    if self.me.anim_is_new() {
                        self.me.pose.speed =
                            crate::btl_hit::launch_speed(self.me, self.data, self.env);
                    }
                    if self.flag(0x5C) {
                        mode = 6;
                    }
                }
                _ => {}
            }
            self.step(mode, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE && matches!(self.anim_id(), 0x4C | 0x4F | 0x52 | 0x55 | 0x58 | 0x5B) {
            self.smash_follow_up(false);
        }
    }

    // ---- the reactions a smash gives (btl_act_1.c) ---------------------------------------------------------------

    /// `BtlActB_UpdateMashRate`: each press of an action button (`ANY_FACE`) adds to the motion's step, kept in
    /// 1..4 and drifting to twice the addend by 4 a second; scaled, past 20% the addend is 0.5 + 0.5 * (1 -
    /// reaction scale). Flag 0xE7 while free.
    fn update_mash_rate(&mut self, scaled: bool) {
        let mut add = 1.0;
        if scaled && self.me.anim_progress() > 0.2 {
            add = (1.0 - self.me.react.scale) * 0.5 + 0.5;
        }
        let target = add * 2.0;
        let mut rate = self.work_f(0);
        if self.me.is_free() {
            self.set(0xE7);
            if self.me.input.is_pressed(crate::btl_input::ANY_FACE) {
                rate += add;
            }
        }
        rate = rate.clamp(1.0, 4.0);
        self.me.anim_set_step(rate);
        let step = 4.0 / 30.0;
        rate = if rate < target {
            (rate + step).min(target)
        } else {
            (rate - step).max(target)
        };
        self.set_work_f(0, rate);
    }

    /// `BtlAct_QueueFlag21Action`: flags 0x21 / 0x22 (the air recovery inputs) queue 0x25 / 0x27, or 0x28 under
    /// lock-on for 0x22.
    fn queue_flag21_action(&mut self) -> bool {
        let fly = if self.data.can_fly() { 0x27 } else { 0x25 };
        if self.flag(0x21) {
            self.set_queue(0, fly);
            return true;
        }
        if self.flag(0x22) {
            self.set_queue(0, if self.flag(5) { 0x28 } else { fly });
            return true;
        }
        false
    }

    /// `BtlAct_QueueFlag21ActionB`: the same against a wall, 0x29 / 0x2A.
    fn queue_flag21_action_b(&mut self) -> bool {
        if self.flag(0x21) {
            self.set_queue(0, 0x29);
            return true;
        }
        if self.flag(0x22) {
            self.set_queue(0, if self.flag(5) { 0x2A } else { 0x29 });
            return true;
        }
        false
    }

    /// Actions 0xCB, 0xCC (`BtlAct_ActionCBtoCC`): motion 0xAE (air 0xAF) at the mash rate; 0xCC leans back and
    /// straightens. Then the idle follow-up; main inputs 0x10000000 / 0x800000 cut it short.
    fn reel(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, true);
            self.play(if air { 0xAF } else { 0xAE }, 0.0);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            self.set_work_f(0, 2.0);
        }
        if phase == RUN {
            let air = self.me.is_air_motion(self.env, false);
            let cb = self.current() == 0xCB;
            self.update_mash_rate(cb);
            if self.me.anim_advance(0) {
                let next = self.idle_follow_up();
                self.request(next);
            }
            self.step(6, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.reaction_flags();
            self.set(0x41);
            if !cb {
                let t = 1.0 - self.me.anim_progress();
                let lean = if air { t * deg(45.0) } else { t * -deg(90.0) };
                self.me.set_lean_x(lean);
            }
        }
        if phase == DECIDE {
            for mask in [0x1000_0000, 0x80_0000] {
                if self.decide_main(mask) != 0 {
                    let q = self.queued();
                    self.request(q);
                }
            }
        }
        if phase == LEAVE {
            self.me.unk_ff0 = 15;
        }
    }

    /// Action 0xCD (`BtlAct_GroundBounceHandler`): hits the ground, motions 0xDC then 0xDD at the mash rate,
    /// sliding on at 50 km/h; then the down action 0xDB, in flight 0xE6 / 0xD9, or with a direction held a
    /// recovery (0x25 up / down, 0xE2 left, 0xE3 right; an air set turns round first).
    fn ground_bounce(&mut self, phase: u32) {
        if phase == ENTER {
            if self.me.is_air_motion(self.env, true) {
                self.me.work[0] |= 1;
            }
            self.play(0xDC, 0.0);
            self.set_work_f(0, 2.0);
        }
        if phase == RUN {
            self.update_mash_rate(true);
            match self.anim_id() {
                0xDC => {
                    self.me.anim_advance_then(0xDD, 0.0, 0);
                }
                0xDD => {
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                }
                _ => {}
            }
            let dir = if self.me.work[0] & 1 != 0 { 2 } else { 4 };
            self.step(6, 5, dir, kmh(50.0), kmh(100.0));
            self.me.apply_gravity();
            self.reaction_flags();
            self.set(0x41);
            if self.env.stage_4_or_27 && self.flag(0x17) {
                self.held(0xE);
            }
        }
        if phase == DECIDE {
            if self.decide_main(0x1000_0000) != 0 {
                let q = self.queued();
                self.request(q);
            }
            if self.me.work[1] != 0 {
                let mut next = 0xDB;
                if self.flag(0xE) {
                    next = if self.me.is_free() { 0xE6 } else { 0xD9 };
                } else if self.me.is_free() && !self.flag(0x1E) {
                    let held = |s: &Self, m: u32| s.me.input.is_held(m);
                    let mut id = -1;
                    if held(self, 0x20) || held(self, 0x10) {
                        id = 0x25;
                    }
                    if held(self, 0x40) {
                        id = 0xE2;
                    }
                    if held(self, 0x80) {
                        id = 0xE3;
                    }
                    if id >= 0 {
                        if self.me.work[0] & 1 != 0 {
                            self.me.pose.yaw = wrap_angle(self.me.pose.yaw + PI);
                        }
                        next = id;
                    }
                }
                self.request(next);
            }
        }
    }

    /// An air recovery with a direction held by a free fighter (0x25, or 0x27 when it can fly).
    fn request_air_recovery(&mut self) {
        if self.me.is_free() && !self.flag(0x1E) && self.me.input.is_held(DIR_MASK) {
            self.request(if self.data.can_fly() { 0x27 } else { 0x25 });
        }
    }

    /// Action 0xCE (`BtlAct_LaunchedHandler`): knocked upward (up to 1200 km/h, 70% plus 30% of the reaction
    /// scale), motion 0xDA rising and 0xDB falling at 1000 km/h. Falling it lands into 0xDC (or an air recovery),
    /// meets a wall (0xE7 / 0xE8 / 0xDC), lands in water or on the stage limits; from the ninth frame a free
    /// fighter recovers with input 0x2F (0xE6) or 0x30 (0x68).
    fn launched(&mut self, phase: u32) {
        if phase == ENTER {
            if self.me.is_air_motion(self.env, true) {
                self.me.work[0] |= 1;
            }
            self.play(0xDA, 0.0);
            self.me.pose.fall_speed = (self.me.react.scale * 0.3 + 0.7) * -kmh(1200.0);
            self.clear(0xE);
        }
        if phase == RUN {
            self.me.fall();
            match self.anim_id() {
                0xDA => {
                    self.me.anim_advance_loop(0);
                    if self.me.pose.fall_speed > 0.0 {
                        self.request_anim(0xDB, 0.4);
                    }
                }
                0xDB => self.me.anim_advance_loop(0),
                _ => {}
            }
            let dir = if self.me.work[0] & 1 != 0 { 2 } else { 4 };
            self.step(6, 5, dir, kmh(1000.0), kmh(100.0));
            self.reaction_flags();
            self.me.set_fx(0x3B);
        }
        if phase == DECIDE {
            if self.decide_main(0x1000_0000) != 0 {
                let q = self.queued();
                self.request(q);
            }
            if self.anim_id() == 0xDB {
                if self.me.frames_to_ground() < 2.0 || self.me.is_heading_into_floor(-6.283_185_3) {
                    if !self.env.stage_4_or_27 || self.me.ground_y() < -10.0 {
                        self.request(0xDC);
                        self.request_air_recovery();
                    } else if self.flag(7) {
                        self.request(0xDC);
                    }
                }
                if self.me.is_heading_into_wall(-6.283_185_3) {
                    if self.me.obj.work.contact_flags & 0x40 != 0 {
                        self.request(if self.me.is_free() { 0xE8 } else { 0xE7 });
                    } else if self.me.work[0] & 1 != 0 {
                        self.request(0xDC);
                        self.request_air_recovery();
                    } else {
                        self.request(0xE7);
                        if self.me.is_free() && !self.flag(0x1E) && self.me.input.is_held(DIR_MASK)
                        {
                            self.request(0x29);
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
                if self.env.stage_4_or_27 && self.flag(0x17) && self.me.is_free() {
                    self.request(0xE6);
                }
            }
            if self.me.action_frame > 8 && self.me.is_free() {
                if self.test(0x2F) {
                    self.request(0xE6);
                }
                if self.test(0x30) {
                    self.held(0xE);
                    self.request(0x68);
                }
            }
        }
    }

    /// Action 0xD4 (`BtlAct_KnockBackHandler`): knocked back at 700 km/h (on stages 4 / 27 scaled by the health
    /// lost), motions 0xBD start, 0xBE loop, 0xBF end (0xC0.. for an air set). The loop lasts the reaction scale
    /// times 15 frames and deflects at the stage limit; a free fighter holding a direction then recovers (0xE6).
    /// The end gives the down action, or 0xD9 higher than the body size past 60%. The sound is not ported.
    fn knock_back(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, true);
            self.play(if air { 0xC0 } else { 0xBD }, 0.0);
            let mut timer = (self.me.react.scale * 15.0) as i32;
            if self.env.stage_4_or_27 {
                timer = (timer as f32 * (1.0 - self.me.health_ratio())) as i32;
            }
            self.me.work[2] = timer;
            self.me.pose.fall_speed = 0.0;
            self.clear(0xE);
            // BtlAct_GetPrev reads a word nobody writes: never a throw's 0xB8 / 0xBA.
        }
        if phase == RUN {
            let mut speed = kmh(700.0);
            if self.env.stage_4_or_27 {
                speed *= (1.0 - self.me.health_ratio()) * 0.5 + 0.5;
            }
            let id = self.anim_id();
            match id {
                0xBD | 0xC0 => {
                    self.me.anim_advance_then(id + 1, 0.0, 0);
                }
                0xBE | 0xC1 => {
                    if self.me.work[2] > 0 {
                        self.me.anim_advance_loop(0);
                        self.me.work[2] -= 1;
                    } else if self.me.anim_advance_then(id + 1, 0.0, 0)
                        && self.me.is_free()
                        && !self.flag(0x1D)
                        && (self.me.input.is_held(DIR_MASK) || self.env.stage_4_or_27)
                    {
                        self.request(0xE6);
                    }
                    if self.me.action_frame > 0 {
                        let back = self.me.is_air_motion(self.env, false);
                        self.me.deflect_at_stage_limit(back);
                    }
                }
                0xBF | 0xC2 => {
                    if self.me.anim_advance(0) {
                        let d = self.down_action();
                        self.request(d);
                    }
                    if self.data.height() < self.me.height_above_ground()
                        && self.me.anim_progress() > 0.6
                    {
                        self.request(0xD9);
                    }
                }
                _ => {}
            }
            let dir = if self.me.is_air_motion(self.env, false) {
                3
            } else {
                5
            };
            self.step(6, 6, dir, speed, 10000.0);
            self.me.brake_vertical();
            self.set(0x1B);
            self.reaction_flags();
            self.me.set_fx(0x3B);
        }
        if phase == DECIDE && self.decide_main(0x1000_0000) != 0 {
            let q = self.queued();
            self.request(q);
        }
    }

    /// Actions 0xD5, 0xD6, 0xDF (`BtlAct_BlowAwayHandler`): blown away in flight for 50 / 15 / 32 frames at
    /// 1000 / 2000 / 2000 km/h (1000 on stages 4 / 27), motion 0xC3 (0xC6 for 0xDF, the next for an air set);
    /// 0xD5 leans with the pitch, 0xD6 spins. A free fighter recovers in the air (inputs 0x28 / 0x29 raise flags
    /// 0x21 / 0x22, with the ki for it) or, with parameter bit 0x40, mashes out (0xD7). Into the floor: the air
    /// recovery or 500 damage and 0xDD; into a wall: 0xE8 / 0xE7 or the wall recovery; at the stage limit 0xE8 /
    /// 0xE7; into the opponent 0xE9 / 0xE7. The sound, camera shake and rumble are not ported.
    fn blow_away(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, true);
            let (timer, anim, bits) = match self.current() {
                0xD5 => (50, if air { 0xC4 } else { 0xC3 }, 3),
                0xD6 => (15, if air { 0xC4 } else { 0xC3 }, 6),
                _ => (32, if air { 0xC7 } else { 0xC6 }, 8),
            };
            self.me.work[0] |= bits;
            let mut timer = timer;
            if self.env.stage_4_or_27 {
                timer = (timer as f32 * (1.0 - self.me.health_ratio())) as i32;
            }
            self.me.work[2] = timer;
            self.play(anim, 0.0);
            self.held(0xE);
            self.me.pose.fall_speed = 0.0;
            // Bit 1 also marks a throw (previous action 0xB8 / 0xBA), which BtlAct_GetPrev never reports.
        }
        if phase == RUN {
            let air = self.me.is_air_motion(self.env, false);
            self.me.anim_advance_loop(0);
            self.me.work[2] -= 1;
            if self.me.work[2] < 0 {
                if self.env.stage_4_or_27 && self.me.is_free() {
                    self.request(0xE6);
                } else {
                    self.request(0xD9);
                }
            }
            if self.me.action_frame > 0 {
                let back = self.me.is_air_motion(self.env, false);
                self.me.deflect_at_stage_limit(back);
            }
            let speed = match (self.current(), self.env.stage_4_or_27) {
                (0xD5, _) | (_, true) => kmh(1000.0),
                _ => kmh(2000.0),
            };
            self.step(6, 6, if air { 3 } else { 5 }, speed, 10000.0);
            self.me.apply_gravity();
            for f in [0x1B, 0xA, 0x2A] {
                self.set(f);
            }
            self.reaction_flags();
            self.me.set_fx(0x3B);
            if self.me.work[0] & 2 != 0 {
                let pitch = self.me.pose.pitch;
                self.me.set_lean_x(pitch);
            }
            if self.me.work[0] & 4 != 0 {
                let spin = wrap_angle(self.work_f(0) + deg(48.0));
                self.set_work_f(0, spin);
                self.me.set_lean_z(spin);
            }
            if self.me.work[0] & 1 != 0 {
                self.set(0x138);
            }
        }
        if phase == DECIDE {
            if self.me.is_free() {
                if self.decide_main(0x1000_0000) != 0 {
                    let q = self.queued();
                    self.request(q);
                }
                let cost = self.data.tables.params.s32_at(0x48);
                if !self.flag(0x1E) && self.me.action_frame > 8 && self.me.has_ki(cost) {
                    self.add_action_bit(0x25);
                    self.add_action_bit(0x2A);
                    if self.test(0x28) {
                        self.set(0x21);
                    }
                    if self.test(0x29) {
                        self.set(0x22);
                    }
                }
                if !self.flag(0x1D) && self.flags2() & 0x40 != 0 {
                    self.add_action_bit(0xD7);
                    self.set(0xE7);
                    if self.me.input.is_pressed(crate::btl_input::ANY_FACE) {
                        self.me.work[3] += 1;
                        let need = ((1.0 - self.me.health_ratio()) * 12.0) as i32 + 4;
                        if self.me.work[3] >= need {
                            self.request(0xD7);
                        }
                    }
                }
            }
            if self.me.is_heading_into_floor(PI * 70.0 / 180.0) {
                if self.queue_flag21_action() {
                    let q = self.queued();
                    self.request(q);
                } else {
                    self.me.damage(self.env, self.data, 500, 0);
                    if self.me.work[0] & 8 != 0 {
                        self.me.pose.speed = 0.0;
                    }
                    self.request(0xDD);
                }
            } else if self.me.is_heading_into_wall(PI * 70.0 / 180.0) {
                if self.me.obj.work.contact_flags & 0x40 != 0 {
                    self.request(if self.me.is_free() { 0xE8 } else { 0xE7 });
                } else if self.queue_flag21_action_b() {
                    let q = self.queued();
                    self.request(q);
                } else {
                    self.me.damage(self.env, self.data, 500, 0);
                    self.request(0xE7);
                }
            } else if self.flag(0x14) {
                let a = normalize(self.me.pose.pos);
                let b = normalize(self.me.pose.vel);
                if a[0] * b[0] + a[1] * b[1] + a[2] * b[2] > 0.7 {
                    self.request(if self.me.is_free() { 0xE8 } else { 0xE7 });
                }
            } else if self.blocked() {
                let air = self.me.is_air_motion(self.env, false);
                let ahead = opp_yaw_from_facing(self.me, self.opp).abs() < deg(90.0);
                if ahead == air {
                    self.request(if self.me.is_free() { 0xE9 } else { 0xE7 });
                }
            }
        }
    }

    /// Action 0xD7 (`BtlAct_BlowRecoverHandler`): stops being blown away, motion 0x18D / 0x18E at half the speed,
    /// braking by 50 km/h a frame while the lean follows the pitch; then 0x19 with up held, else 0xB. The sound is
    /// not ported.
    fn blow_recover(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, false);
            self.play(if air { 0x18E } else { 0x18D }, 0.15);
            self.me.pose.speed *= 0.5;
            self.held(0xE);
        }
        if phase == RUN {
            let air = self.me.is_air_motion(self.env, false);
            if self.me.anim_advance(0) {
                self.request(if self.me.input.is_held(0x10) {
                    0x19
                } else {
                    0xB
                });
            }
            self.step(6, 6, if air { 3 } else { 5 }, 0.0, kmh(50.0));
            self.me.apply_gravity();
            let t = 1.0 - self.me.anim_progress();
            let pitch = self.me.pose.pitch;
            self.me.set_lean_x(pitch * t);
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

    /// `BtlAct_PlaySubAnim`: the sub layer's motion, and the node of its first fire event.
    fn play_sub_anim(&mut self, motion: i32) -> i32 {
        self.me.anim_play_sub(self.data, motion);
        mask_to_node(self.query_event(4, 1, 6) as u32)
    }

    /// `BtlAct_SetPitchMotion` (measuring again): blends in an up or down motion by the pitch toward the opponent,
    /// under lock-on only.
    fn set_pitch_motion(&mut self, up: i32, down: i32) {
        if !self.flag(5) {
            self.me.aim_pitch = 0.0;
            return;
        }
        let own = self.me.obj.node_pos(0x11);
        let mut dir = sub(self.opp.obj.node_pos(0x11), own);
        let len = length(dir);
        if 0.0001 < len {
            dir = dir.map(|v| v * (1.0 / len));
        }
        self.me.aim_pitch = -dir[1].clamp(-1.0, 1.0).asin();
        let t = (self.me.aim_pitch / 0.785_398_16).clamp(-1.0, 1.0);
        if t < 0.0 {
            self.me.anim_set_sub_mix(-t);
            self.me.anim_play_sub(self.data, down);
        } else {
            self.me.anim_set_sub_mix(t);
            self.me.anim_play_sub(self.data, up);
        }
    }

    /// `BtlCharApi_CalcAimDir`: the unit direction from `from` to the opponent's node `node`, its pitch kept in
    /// min..max (no technique is ported); a direction away from the opponent is mirrored.
    fn calc_aim_dir(&self, node: u32, from: [f32; 3], min: f32, max: f32) -> [f32; 3] {
        let yaw = self.me.pose.rot[1];
        let mut out = sub(self.opp.obj.node_pos(node), from);
        let len = length(out);
        if len < 0.0001 {
            return [yaw.sin(), 0.0, yaw.cos()];
        }
        out = out.map(|v| v * (1.0 / len));
        let delta = opp_delta(self.me, self.opp);
        if out[0] * delta[0] + out[2] * delta[2] < 0.0 {
            out[0] = -out[0];
            out[2] = -out[2];
        }
        let (lo, hi) = (min.sin(), max.sin());
        for limit in [lo, hi] {
            let past = if limit == lo {
                out[1] < lo
            } else {
                hi < out[1]
            };
            if !past {
                continue;
            }
            let xz = (1.0 - limit * limit).sqrt();
            if out[1].abs() < 0.999 {
                let k = xz / (1.0 - out[1] * out[1]).sqrt();
                out = [out[0] * k, limit, out[2] * k];
            } else {
                out = [xz * yaw.sin(), limit, xz * yaw.cos()];
            }
        }
        out
    }

    /// The aim of the standing ki blasts: at the opponent's node `node` from the own node 0x11 within 36 degrees
    /// under lock-on, else along the heading.
    fn aim_blast(&mut self, node: u32) {
        self.me.aim_dir = if self.flag(5) {
            let from = self.me.obj.node_pos(0x11);
            self.calc_aim_dir(node, from, -deg(36.0), deg(36.0))
        } else {
            let yaw = self.me.pose.yaw;
            [yaw.sin(), 0.0, yaw.cos()]
        };
    }

    /// Action 0xAE (`BtlAct_KiBlastHandler`): motion 0x71 (0x72 for the second hand, which nothing ever picks:
    /// the previous action is never stored), the ki cost, the shot count and its 30-frame timer; the shot leaves
    /// on the motion's fire event. The voice is not ported.
    fn ki_blast(&mut self, phase: u32) {
        if phase == ENTER {
            if self.me.act.prev != 0xAE {
                self.me.blast_alt_hand = 0;
            }
            if self.me.blast_alt_hand == 0 {
                self.play(0x71, 0.1);
                self.me.blast_alt_hand = 1;
            } else {
                self.play(0x72, 0.1);
                if !self.me.gauge.body_changed && self.data.tables.params.flags() & 0x100 == 0 {
                    self.me.blast_alt_hand = 0;
                }
            }
            let cost = ki_cost(self.me, self.data);
            self.me.spend_ki(cost, false);
            self.me.blast_shots += hits_of(self.data, 0);
            self.me.blast_shots_timer = 0x1E;
        }
        if phase == RUN {
            let idle = if self.flag(0xE) { 0x26 } else { 0 };
            match self.anim_id() {
                0x71 | 0x72 => {
                    self.me.anim_advance_then(idle, 0.2, 0);
                    let a = self.anim_id();
                    self.set_pitch_motion(a + 2, a + 4);
                }
                0 | 0x26 => {
                    self.me.anim_advance(1);
                    if !self.flag(0x34) {
                        self.request(0xB);
                    }
                    self.set(0x87);
                }
                _ => {}
            }
            self.step(2, 5, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
            self.aim_blast(0x11);
        }
        if phase == DECIDE {
            self.decide_attack(0xC);
            match self.anim_id() {
                0x71 | 0x72 => {
                    if self.flag(anim_flag::END) {
                        let q = self.queued();
                        self.request(q);
                    }
                }
                0 | 0x26 => {
                    self.decide_main(0x81CEA);
                    let q = self.queued();
                    self.request(q);
                }
                _ => {}
            }
        }
    }

    /// Action 0xAF (`BtlAct_ChargedKiBlastHandler`): wind-up 0x7B, hold 0x7C until the blast button is released
    /// (input 91; the hold's progress is the charge), release 0x7D (ki cost on its first frame). The sounds and the
    /// voice are not ported.
    fn charged_ki_blast(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x7B, 0.15);
            self.me.blast_node = self.play_sub_anim(0x7D);
            self.me.set_fx(0x17);
            self.me.blast_shots += 1;
            self.me.blast_shots_timer = 0x1E;
        }
        if phase == RUN {
            match self.anim_id() {
                0x7B => {
                    self.me.anim_advance_then(0x7C, 0.0, 0);
                    self.set(0x8E);
                }
                0x7C => {
                    self.me.anim_advance_then(0x7D, 0.0, 0);
                    self.set(0x8E);
                    self.me.blast_charge = self.me.anim_progress();
                }
                0x7D => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    if self.me.obj.frame < self.query_event(4, 0, 0) as f32 {
                        self.set(0x8E);
                    }
                    if self.me.anim_is_new() {
                        let cost = ki_cost(self.me, self.data);
                        self.me.spend_ki(cost, false);
                    }
                }
                _ => {}
            }
            self.step(2, 5, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
            let a = self.anim_id();
            self.set_pitch_motion(a + 3, a + 6);
            // BtlCharApi_CalcAimDir is given the blast's node as the opponent's node to aim at.
            let node = self.me.blast_node as u32;
            self.aim_blast(node);
        }
        if phase == DECIDE && self.anim_id() == 0x7C && self.test(0x5B) {
            self.request_anim(0x7D, 0.0);
        }
    }

    /// The motion of a dash ki blast by the facing relative to the camera (`BtlUtil_AngleToSector`): forward under
    /// lock-on steers at the opponent (work[0] bit 0).
    fn dash_blast_motion(&mut self, forward: i32, left: i32, right: i32) -> i32 {
        let a = self.me.facing_rel_cam();
        if a < -2.356_194_5 {
            forward
        } else if a < -0.785_398_16 {
            left
        } else if a < 0.785_398_16 {
            if self.flag(5) {
                self.me.work[0] |= 1;
                self.held(0xE);
            }
            forward
        } else if a < 2.356_194_5 {
            right
        } else {
            forward
        }
    }

    /// The movement of the dash ki blasts: the dash, steering at the opponent when locked on forward.
    fn dash_blast_move(&mut self) {
        let speed = self
            .me
            .move_speed(self.data, self.env, 4 + usize::from(self.flag(0x11)));
        if self.me.work[0] & 1 != 0 && self.flag(5) {
            let max = deg(54.0);
            let yaw_accel = if self.me.action_frame == 0 {
                PI
            } else {
                deg(9.0)
            };
            let near = (opp_gap_xz(self.me, self.opp, self.data, self.opp_data) * 0.01).min(1.0);
            steer_at_opponent(
                self.me,
                self.opp,
                self.data,
                self.opp_data,
                speed,
                yaw_accel,
                0.0,
                near * max,
                0.0,
                max,
            );
            self.me.turn_model_yaw(deg(36.0), 0.5);
            self.me.set_direction(3);
            self.me.advance(speed, kmh(200.0));
        } else {
            let turn = self.me.turn_rate(self.data, 0);
            self.me.turn_yaw(self.opp, 0, turn);
            self.me
                .turn_pitch(self.data, self.opp, self.opp_data, 5, deg(9.0));
            self.me.turn_model_yaw(deg(36.0), 0.3);
            self.me.set_direction(3);
            self.me.advance(speed, 10000.0);
        }
        self.me.apply_gravity();
        self.me.request_orbit(20.0, 50.0);
        self.set(0xCA);
        self.me.set_fx(0xB);
        if self.flag(0x11) {
            self.held(0xE);
        }
        if self.flag(5) && self.me.work[0] & 1 != 0 {
            let pitch = self.me.pose.pitch;
            self.me.set_lean_x(pitch);
        }
    }

    /// The aim of the dash ki blasts: the direction of travel, turned 90 degrees for the sideways motions.
    fn aim_sideways(&mut self, left: i32, right: i32) {
        let d = self.me.pose.dir;
        let a = self.anim_id();
        self.me.aim_dir = if a == left {
            [d[2], d[1], -d[0]]
        } else if a == right {
            [-d[2], d[1], d[0]]
        } else {
            d
        };
    }

    /// Action 0xB0 (`BtlAct_DashKiBlastHandler`): a ki blast while dashing (0x77 forward, 0x78 / 0x79 sideways,
    /// fired across the direction of travel), then the dash 0xF. The voice is not ported.
    fn dash_ki_blast(&mut self, phase: u32) {
        if phase == ENTER {
            let anim = self.dash_blast_motion(0x77, 0x78, 0x79);
            self.play(anim, 0.15);
            let cost = ki_cost(self.me, self.data);
            self.me.spend_ki(cost, false);
            self.me.blast_shots += self.query_event(4, 0, 3);
            self.me.blast_shots_timer = 0x1E;
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xF);
            }
            self.aim_sideways(0x78, 0x79);
            self.dash_blast_move();
        }
        if phase == DECIDE && 0.5 < self.me.anim_progress() {
            self.decide_attack(0xC000);
            if self.test(0x2C) {
                self.set_queue(0, 0x3E);
            }
            if self.flag(anim_flag::END) {
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// Action 0xB1 (`BtlAct_DashChargedKiBlastHandler`): the charged ki blast while dashing: wind-up 0x84 / 0x87 /
    /// 0x8A, hold +1 until the button is released, release +2 (only the sideways releases aim across), then 0xF.
    fn dash_charged_ki_blast(&mut self, phase: u32) {
        if phase == ENTER {
            let anim = self.dash_blast_motion(0x84, 0x87, 0x8A);
            self.play(anim, 0.15);
            self.me.blast_node = self.play_sub_anim(anim + 2);
            self.me.set_fx(0x17);
            self.me.blast_shots += 1;
            self.me.blast_shots_timer = 0x1E;
        }
        if phase == RUN {
            match self.anim_id() {
                0x84 | 0x87 | 0x8A => {
                    let next = self.anim_id() + 1;
                    self.me.anim_advance_then(next, 0.0, 0);
                    self.set(0x8E);
                }
                0x85 | 0x88 | 0x8B => {
                    let next = self.anim_id() + 1;
                    self.me.anim_advance_then(next, 0.0, 0);
                    self.set(0x8E);
                    self.me.blast_charge = self.me.anim_progress();
                }
                0x86 | 0x89 | 0x8C => {
                    if self.me.anim_advance(0) {
                        self.request(0xF);
                    }
                    if self.me.anim_is_new() {
                        let cost = ki_cost(self.me, self.data);
                        self.me.spend_ki(cost, false);
                    }
                    if self.me.obj.frame < self.query_event(4, 0, 0) as f32 {
                        self.set(0x8E);
                    }
                }
                _ => {}
            }
            self.aim_sideways(0x89, 0x8C);
            self.dash_blast_move();
        }
        if phase == DECIDE && matches!(self.anim_id(), 0x85 | 0x88 | 0x8B) && self.test(0x5B) {
            let next = self.anim_id() + 1;
            self.request_anim(next, 0.0);
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

/// `BtlAct_ScaleSpeedByDist`: under lock-on, `frames` frames of the speed travel at most `range` times the
/// horizontal distance to the opponent.
fn scale_speed_by_dist(me: &Chr, opp: &Chr, speed: f32, range: f32, frames: f32) -> f32 {
    if !me.flags.test(5) {
        return speed;
    }
    let travel = speed * frames;
    let limit = range * crate::btl_chr::opp_distance_xz(me, opp);
    if limit < travel && 0.01 < travel {
        speed * (limit / travel)
    } else {
        speed
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
            0xB, 0xD, 0xF, 0x38, 0x44, 0xAE, 0xB1, 0xC0, 0xC6, 0xCF, 0xD2, 0xD8, 0xDB, 0xE1,
        ] {
            assert!(ported(id), "0x{id:X}");
        }
        for id in [0xB2, 0x60, 0xB8, 0xF6] {
            assert!(!ported(id), "0x{id:X}");
        }
    }
}
