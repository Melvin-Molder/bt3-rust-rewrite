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

use crate::btl_blast::{HitRec, blast_limit_a, can_fire, hits_of, ki_cost, ki_cost_of};
use crate::btl_chr::warp_in_front_of_opponent;
use crate::btl_chr::{
    Attack, Chr, Env, SideData, add, anim_flag, begin_rise_to_opponent, calc_approach_point,
    circle_opponent, deg, dot, is_blocked_by_opponent, kmh, length, normalize, opp_closing_time,
    opp_delta, opp_distance_xz, opp_gap_xz, opp_pitch_adjusted, opp_yaw, opp_yaw_from_facing,
    opp_yaw_from_its_facing, scale_speed_by_approach, scale3, snap_to_opponent, steer_at_opponent,
    sub, warp_ahead_of_opponent, warp_behind_opponent,
};
use crate::btl_clash::{ClashC, count_and_mark};
use crate::btl_input::{DIR_MASK, wrap_angle};
use crate::btl_obj::{mask_to_node, node_side};

/// `BTL_SUPER_TIMEOUT`: frames after which a technique stops waiting for its effect's flag.
const SUPER_TIMEOUT: i32 = 150;

/// The motion `n` of a class 2..4 technique: 0x22 per class from 0x105 (0 start, 1 charge loop, 2 fire, 3 firing
/// loop, 4 second stage, 5 its loop, 6 end; + 7 / + 14 aimed up / down).
fn super_anim(cls: i32, n: i32) -> i32 {
    cls * 0x22 + 0xC1 + n
}

/// `BtlAct_IsTechniqueId`: 0x105..0x132.
fn is_technique_id(id: i32) -> bool {
    (0x106..0x106 + 0x2D).contains(&id) || id == 0x105
}

/// `BtlAct_GetIdClass`.
fn id_class(id: i32) -> i32 {
    if id == 0x105 {
        4
    } else if (0xFD..0xFD + 6).contains(&id) {
        (id - 0xFD) % 2
    } else if (0x106..0x106 + 0x36).contains(&id) {
        (id - 0x106) % 3 + 2
    } else {
        -1
    }
}

/// `BtlAct_IsAttackId`: the attack actions of the record table (0x70..0xAD).
fn is_attack_id(id: i32) -> bool {
    (0x70..0xAE).contains(&id)
}

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
    /// The effect scene's ki blast records of the last frame (`EftHit_GetList`).
    pub blasts: &'a [HitRec],
    /// The clash C block (`gBtlChars->clashC`).
    pub clash: &'a ClashC,
}

/// Whether action `id` has a handler here.
pub(crate) fn ported(id: i32) -> bool {
    matches!(id,
        1..=3 | 0xB..=0x36 | 0x37..=0x6F | 0x70..=0xBC | 0xBE | 0xBF
        | 0xBD | 0xC0..=0xD9 | 0xDA..=0xDF | 0xE0..=0xE9 | 0xFA..=0xFC | 0x105..=0x11A)
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
        self.me
            .anim_play_any(self.data, self.opp_data, self.env, motion, blend);
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
        if mask & 0x10_0000 != 0 {
            let n = self.check_technique_input();
            if n != 0 {
                return n;
            }
        }
        // Mask 0x800000 (skills): not ported.
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
        if mask & 0x100_0000 != 0 && self.flags2() & 0x20 != 0 {
            if self.data.tables.params.flags() & 2 != 0 {
                if self.blast_incoming(true, true, true, 90.0) && self.test(0x2B) {
                    self.set_queue(0, 0x3D);
                    return 1;
                }
            } else if self.blast_incoming(true, true, false, 90.0) && self.test(0x2B) {
                self.set_queue(0, 0x3B);
                return 1;
            }
        }
        if mask & 0x200_0000 != 0
            && self.flags2() & 0x20 != 0
            && self.blast_incoming(false, true, false, 5.0)
            && self.test(0x27)
        {
            self.set_queue(0, 0x3C);
            return 1;
        }
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
        if mask & 0x10 != 0 && self.flags2() & 0x800 != 0 {
            self.add_action_bit(0x5E);
            if self.test(0x4D) {
                self.set_queue(0, 0x5E);
                return 1;
            }
        }
        // Masks 0x800000 / 0x1000000: the finishers 0x5E / 0x5F, after the feint 0x40 by the parameter's choice.
        for (bit, flag2, input, choice, action) in [
            (0x80_0000, 0x800, 0x4D, 1, 0x5E),
            (0x20, 0x1000, 0x4E, -1, 0x5F),
            (0x100_0000, 0x1000, 0x4E, 2, 0x5F),
        ] {
            if mask & bit == 0 || self.flags2() & flag2 == 0 {
                continue;
            }
            self.add_action_bit(action);
            if !self.test(input) {
                continue;
            }
            let list: &[i32] = match choice {
                -1 => &[action],
                c => match self.finisher_choice(c as u32) {
                    0 => &[action],
                    1 => &[0x40, action],
                    _ => &[],
                },
            };
            if !list.is_empty() {
                for (i, &a) in list.iter().enumerate() {
                    self.set_queue(i, a);
                }
                return list.len() as u32;
            }
        }
        if mask & 0x20_0000 != 0 {
            let step = ((self.me.rush_step + 4) % 5) as u32;
            let act = match self.rush_finisher(step) {
                0 => 0x60,
                1 => 0x61,
                2 => 0x62,
                3 => 0x63,
                4 => 0x66,
                5 => self.alternating_finisher(),
                _ => 0,
            };
            self.add_action_bit(act);
            if self.test(0x42) {
                self.set_queue(0, act);
                return 1;
            }
        }
        if mask & 0xA40_0000 != 0 {
            let (kind, input) = if mask & 0x40_0000 != 0 {
                (0, 0x42)
            } else if mask & 0x200_0000 != 0 {
                (3, 0x42)
            } else if blast_limit_a(self.data) <= 0 {
                (4, 0x58)
            } else {
                (-1, -1)
            };
            if kind >= 0 {
                let choice = self.finisher_choice(kind as u32);
                let act = match choice / 2 {
                    0 => 0x60,
                    1 => 0x61,
                    2 => 0x62,
                    3 => 0x63,
                    4 => 0x66,
                    5 => self.alternating_finisher(),
                    _ => 0,
                };
                let list: Vec<i32> = match (choice < 12, choice % 2) {
                    (false, _) => Vec::new(),
                    (true, 0) => vec![act],
                    (true, _) => vec![0x40, act],
                };
                if let Some(&last) = list.last() {
                    self.add_action_bit(last);
                    if self.test(input as u32) {
                        for (i, &a) in list.iter().enumerate() {
                            self.set_queue(i, a);
                        }
                        return list.len() as u32;
                    }
                }
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
        // Past the vanish limit the powered combo finisher (BtlAct_QueueComboFinish) would be offered instead,
        // which is not ported.
        if mask & 0x10_0000 != 0 {
            let n = self.queue_attack(0x9B);
            if n > 0 {
                return n as u32;
            }
        }
        if mask & 0x10_0000 != 0
            && self.me.vanish_count < self.me.vanish_limit
            && self.flags2() & 0x2000 != 0
        {
            self.add_action_bit(0x5C);
            for (input, action) in [(0x44, 0x5C), (0x45, 0x5D), (0x46, 0x5A), (0x47, 0x5B)] {
                if self.test(input) {
                    self.set_queue(0, action);
                    return 1;
                }
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
        if mask & 0x800 != 0 && self.test(0x60) {
            let a = if self.chain_kind(1) == 5 { 0x40 } else { 0x69 };
            self.set_queue(0, a);
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
            0x2B | 0x2C => self.vanish_attack(phase),
            0x2D | 0x2E => self.warp_attack(phase),
            0x2F | 0x30 => self.slide_attack(phase),
            0x31 => self.rush_attack(phase),
            0x32 => self.hop_back_attack(phase),
            0x33 => self.circle_dash(phase),
            0x34 => self.rush_dash(phase),
            0x35 => self.vanish_dash(phase),
            0x36 => self.search(phase),
            0x92 | 0x9B | 0xAB => self.attack_dash(phase),
            0x99 => self.attack_volley_dash(phase),
            0x95 => self.ki_volley(phase),
            0x70..=0x94 | 0x96..=0xAD => self.attack(phase),
            0xB2 => self.falling_blast(phase),
            0xB3 => self.charged_falling_blast(phase),
            0xB4..=0xB6 => self.grab(phase),
            0xB7 | 0xB9 => self.throw(phase),
            0xB8 | 0xBA => self.thrown(phase),
            0xBB => self.slam_throw(phase),
            0xBC => self.slam_thrown(phase),
            0xBE => self.drag_down(phase),
            0xBF => self.dragged_down(phase),
            0x47..=0x4C => self.smash_charge(phase),
            0x4D..=0x52 => self.smash_full(phase),
            0x53..=0x57 => self.dash_smash(phase),
            0x58 => self.flying_kick(phase),
            0x59 => self.lift_strike(phase),
            0x5A..=0x5D => self.smash_vanish(phase),
            0x5E..=0x62 | 0x64..=0x66 => self.rush_finish(phase),
            0x63 => self.rush_finish_plain(phase),
            0x40 => self.feint(phase),
            0x39 => self.evade(phase),
            0x3B => self.swat(phase),
            0x3C => self.duck_blast(phase),
            0x3D => self.block_blasts(phase),
            0x3E => self.air_step(phase),
            0x3F => self.ground_step(phase),
            0x41 => self.rush_guard(phase),
            0x42 => self.health_burst(phase),
            0x43 => self.idle_talk(phase),
            0x45 => self.rush_auto(phase),
            0x46 => self.rush_rapid(phase),
            0x67..=0x69 => self.ki_pose(phase),
            0x6A => self.lunge(phase),
            0x6B..=0x6F => self.homing_strike(phase),
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
            0xE7 => self.wall_slam(phase),
            0xE8 | 0xE9 => self.wall_recover(phase),
            0x105 => self.ultimate_start(phase),
            0x106..=0x108 | 0x10C..=0x10E => self.super_beam(phase),
            0x109..=0x10B => self.super_warp_beam(phase),
            0x10F..=0x111 => self.super_charge(phase),
            0x112..=0x114 => self.super_repeat(phase),
            0x115..=0x11A => self.super_quick_beam(phase),
            0xFA => self.clash_blows(phase),
            0xFB => self.clash_vanish(phase),
            0xFC => self.clash_exchange(phase),
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
            self.me
                .anim_flush_requests(self.data, self.opp_data, self.env);
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
            if self.flag(0x77) {
                self.queue_attack(if self.env.stage_4_or_27 { 0x7A } else { 0x79 });
                let q = self.queued();
                self.request(q);
            }
        }
        if self.flag(0x7C) {
            let id = self.evasion_attack();
            self.queue_attack(id);
            let q = self.queued();
            self.request(q);
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
        if self.flag(0x6B) {
            let a = if self.data.tables.params.flags() & 2 != 0 {
                0x3D
            } else {
                0x3B
            };
            self.request(a);
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
        // Flags 0xAA / 0xAB (the techniques) are not ported.
        for (flag, action) in [(0x7D, 0x41), (0x7E, 0x45)] {
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
            if self.env.mode != 1
                && self.flag(3)
                && self.me.idle_talk_count <= 0
                && !self.flag(0xB1)
            {
                self.me.work[2] += 1;
                if self.me.work[2] > 90 {
                    self.request(0x43);
                }
            }
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
                    for id in [0x8F, 0x8D, 0x8E] {
                        self.queue_attack(id);
                    }
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

    // ---- the dash attacks (btl_act_1.c) --------------------------------------------------------------------------

    /// `BtlMoveParam_GetAngle58`: table +0x58 (degrees).
    fn angle58(&self) -> f32 {
        self.data.tables.moves.angle58 * PI / 180.0
    }

    /// The horizontal gap to the opponent as a 0..1 steering scale (`BtlOpp_GetGapXZ * 0.01`, at most 1).
    fn near_scale(&self) -> f32 {
        (opp_gap_xz(self.me, self.opp, self.data, self.opp_data) * 0.01).min(1.0)
    }

    /// Actions 0x53..0x57 (`BtlAct_DashSmashHandler`): the charged smash out of a dash, motions wind-up -> hold ->
    /// release (0x65.., 0x68.., 0x5F.., 0x62.., 0x5C..), homing on the opponent at dash speed while it charges and
    /// spending the dash's ki. Letting go of the button (input 0x3F) or losing the lock-on releases it. The sounds,
    /// the voice and the vibration are not ported.
    fn dash_smash(&mut self, phase: u32) {
        if phase == ENTER {
            let anim = match self.current() {
                0x53 => 0x65,
                0x54 => 0x68,
                0x55 => 0x5F,
                0x56 => 0x62,
                _ => 0x5C,
            };
            self.play(anim, 0.15);
            self.held(0xE);
        }
        if phase == RUN {
            let mut speed = self.me.move_speed(self.data, self.env, 6);
            let close_speed = speed;
            let cost = self.move_ki_cost(0);
            let mut accel = 10000.0;
            let mut steer = true;
            let id = self.anim_id();
            match id {
                0x5C | 0x5F | 0x62 | 0x65 | 0x68 => {
                    self.me.anim_advance_then(id + 1, 0.0, 0);
                    self.me.set_fx(4);
                    self.me.set_fx(9);
                    self.me.spend_ki(cost, false);
                    self.set(0x47);
                }
                0x5D | 0x60 | 0x63 | 0x66 | 0x69 => {
                    self.set_charge_anim_duration();
                    self.me.anim_advance_then(id + 1, 0.0, 0);
                    if self.me.dash_count > 0 {
                        self.step_charge();
                        self.me.set_fx(0x1E);
                    } else if self.step_charge() {
                        self.me.charge_full_frames += 1;
                    }
                    self.me.set_fx(4);
                    self.me.set_fx(9);
                    self.me.spend_ki(cost, false);
                    self.set(0x47);
                }
                0x5E | 0x61 | 0x64 | 0x67 | 0x6A => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    self.me.obj.rate = 2.0;
                    if self.flag(0x5B) {
                        speed = 0.0;
                        self.me.pose.speed = speed;
                    }
                    if self.flag(0x6C) {
                        self.me.work[0] |= 1;
                    }
                    if self.me.work[0] & 1 != 0 {
                        speed = 0.0;
                        self.me.pose.speed = speed;
                    }
                    if self.flag(0x5C) || self.flag(0x5D) {
                        speed = 0.0;
                        accel = kmh(100.0);
                        steer = false;
                        if self.blocked() {
                            self.me.pose.speed = speed;
                        }
                    }
                }
                _ => {}
            }
            if self.flag(5) && steer {
                let (yaw_accel, pitch_accel) = (self.turn_accel(0), self.turn_accel(1));
                let (yaw_max, pitch_max) = (self.turn_max(0), self.turn_max(1));
                let scale = self.near_scale();
                steer_at_opponent(
                    self.me,
                    self.opp,
                    self.data,
                    self.opp_data,
                    close_speed,
                    yaw_accel,
                    pitch_accel,
                    yaw_max * scale,
                    pitch_max * scale,
                    PI,
                );
                self.me.turn_model_yaw(deg(36.0), 1.0);
                self.me.set_direction(3);
                self.me.advance(speed, accel);
            } else {
                self.step(6, 6, 3, speed, accel);
            }
            self.me.apply_gravity();
            self.set(0x89);
            self.set(9);
            self.set(0xD5);
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            let id = self.anim_id();
            match id {
                0x5C | 0x5F | 0x62 | 0x65 | 0x68 => {
                    if self.test(0x3F) {
                        self.me.work[0] |= 2;
                    }
                }
                0x5D | 0x60 | 0x63 | 0x66 | 0x69 => {
                    if self.test(0x3F) {
                        self.me.work[0] |= 2;
                    }
                    if !self.flag(5) {
                        self.me.work[0] |= 2;
                    }
                    if self.me.work[0] & 2 != 0 {
                        self.request_anim(id + 1, 0.0);
                    }
                }
                0x5E | 0x61 | 0x64 | 0x67 | 0x6A => {
                    if self.flag(0x5B) {
                        self.set(0x3D);
                        match self.opp_seen_action() {
                            0xD5 => {
                                if !self.has_queued() {
                                    self.decide_attack(0x18_0000);
                                }
                                if 0.7 < self.me.anim_progress() {
                                    let q = self.queued();
                                    self.request(q);
                                }
                            }
                            0xC6 => {
                                if !self.has_queued() {
                                    self.decide_main(0x8000);
                                    self.decide_attack(1);
                                }
                                if 0.6 < self.me.anim_progress() {
                                    let q = self.queued();
                                    self.request(q);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Action 0x58 (`BtlAct_FlyingKickHandler`): the kick out of a dash, motions 0x6B (wind-up) -> 0x6C (hold, the
    /// charge follows its progress) -> 0x6D (strike, on letting go: input 0x41). Within the parameter's angle of the
    /// opponent it homes on it. The follow-up attack records 0x87 / 0x77 (`BtlDecide_QueueAttack`), the sounds, the
    /// voice and the vibration are not ported.
    fn flying_kick(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x6B, 0.15);
            self.held(0xE);
            if self.flag(5) && opp_yaw_from_facing(self.me, self.opp).abs() < self.angle58() {
                self.me.work[0] |= 1;
            }
        }
        if phase == RUN {
            let mut lean = 0.0;
            let n = if self.flag(0x11) { 5 } else { 4 };
            let mut speed = self.me.move_speed(self.data, self.env, n);
            let close_speed = speed;
            let mut steer = self.me.work[0] & 1 != 0;
            match self.anim_id() {
                0x6B => {
                    self.me.anim_advance_then(0x6C, 0.0, 0);
                    self.me.set_fx(0xB);
                    lean = 1.0;
                }
                0x6C => {
                    self.me.anim_advance_then(0x6D, 0.0, 0);
                    self.me.charge = self.me.anim_progress();
                    self.me.set_fx(0xB);
                    lean = 1.0;
                }
                0x6D => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    if self.flag(0x5B) {
                        speed = 0.0;
                        self.me.pose.speed = speed;
                    } else if self.flag(0x5C) || self.flag(0x5D) {
                        speed = 0.0;
                        steer = false;
                        if self.blocked() {
                            self.me.pose.speed = 0.0;
                        }
                    } else {
                        self.me.set_fx(0xB);
                    }
                    lean = 1.0 - self.me.anim_progress();
                }
                _ => {}
            }
            if steer && self.flag(5) {
                let yaw_accel = if self.me.action_frame == 0 {
                    PI
                } else {
                    deg(9.0)
                };
                let scale = self.near_scale();
                steer_at_opponent(
                    self.me,
                    self.opp,
                    self.data,
                    self.opp_data,
                    close_speed,
                    yaw_accel,
                    0.0,
                    scale * deg(54.0),
                    0.0,
                    PI,
                );
                self.me.turn_model_yaw(deg(36.0), 0.5);
                self.me.set_direction(3);
                self.me.advance(speed, kmh(100.0));
            } else {
                self.step(6, 6, 3, speed, kmh(100.0));
            }
            self.me.apply_gravity();
            self.set(0xD5);
            let pitch = self.me.pose.pitch;
            self.me.set_lean_x(pitch * lean);
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            match self.anim_id() {
                0x6C => {
                    if self.test(0x41) {
                        self.request_anim(0x6D, 0.0);
                    }
                }
                0x6D => {
                    if self.flag(0x5B) {
                        for id in [0x87, 0x77] {
                            if self.queue_attack(id) != 0 {
                                self.set_work_f(0, 0.4);
                                break;
                            }
                        }
                        if self.decide_attack(1) != 0 {
                            self.set_work_f(0, 0.6);
                        }
                        if self.has_queued() && self.work_f(0) < self.me.anim_progress() {
                            let q = self.queued();
                            self.request(q);
                        }
                    }
                    self.cancel_on_opp_c9(0.6);
                }
                _ => {}
            }
        }
    }

    /// Action 0x59 (`BtlAct_LiftStrikeHandler`): the rising strike, motions 0x6E (wind-up) -> 0x6F (hold) -> 0x70
    /// (strike, on letting go: input 0x41), leaving flight. Within the parameter's angle of the opponent it heads
    /// for it, pitched up 27..81 degrees; otherwise straight up at 36 degrees. Under flags2 0x400000 a blast press on
    /// a hit queues the finisher 0x5F. The sounds, the voice and the vibration are not ported.
    fn lift_strike(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x6E, 0.15);
            self.clear(0xE);
            self.me.pose.fall_speed = 0.0;
            if self.flag(5) && opp_yaw_from_facing(self.me, self.opp).abs() < self.angle58() {
                self.me.turn_yaw(self.opp, 2, PI);
                self.me
                    .turn_pitch(self.data, self.opp, self.opp_data, 1, PI);
                self.me.pose.pitch = self.me.pose.pitch.clamp(deg(-81.0), deg(-27.0));
                self.me.work[0] |= 1;
            } else {
                let yaw = self.me.pose.yaw;
                self.me.set_heading(yaw, deg(-36.0));
            }
        }
        if phase == RUN {
            let mut speed = self.me.move_speed(self.data, self.env, 4);
            let mut falling = false;
            let mut steer = true;
            match self.anim_id() {
                0x6E => {
                    self.me.anim_advance_then(0x6F, 0.0, 0);
                }
                0x6F => {
                    self.me.anim_advance_then(0x70, 0.0, 0);
                    self.me.charge = self.me.anim_progress();
                }
                0x70 => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    speed = 0.0;
                    falling = true;
                    if self.flag(0x5B) {
                        self.me.pose.speed = speed;
                    } else if self.flag(0x5C) || self.flag(0x5D) {
                        steer = false;
                        if self.blocked() {
                            self.me.pose.speed = speed;
                        }
                    }
                }
                _ => {}
            }
            let yaw = if self.me.work[0] & 1 != 0 && steer {
                2
            } else {
                6
            };
            self.step(yaw, 6, 3, speed, kmh(100.0));
            if falling && !self.flag(0xE) {
                self.me.fall();
            } else {
                self.me.brake_vertical();
            }
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            match self.anim_id() {
                0x6F => {
                    if self.test(0x41) {
                        self.request_anim(0x70, 0.0);
                    }
                }
                0x70 => {
                    if self.flag(0x5B) {
                        if self.flags2() & 0x40_0000 != 0 {
                            self.add_action_bit(0x5F);
                            if self.test(0x50) {
                                self.set_queue(0, 0x5F);
                                self.set_work_f(0, 0.3);
                            }
                        }
                        if self.decide_attack(1) != 0 {
                            self.set_work_f(0, 0.6);
                        }
                        if self.has_queued() && self.work_f(0) < self.me.anim_progress() {
                            let q = self.queued();
                            self.request(q);
                        }
                    }
                    self.cancel_on_opp_c9(0.6);
                }
                _ => {}
            }
        }
    }

    /// Actions 0x5A..0x5D (`BtlAct_SmashVanishHandler`): the vanishing smash. Eight frames out of sight while the
    /// strike's lead is measured from its motions on the sub layer (work[2] the step, work[3] the frame count,
    /// work[4] the motion, workF[0] the lead); then onto the opponent's body, then ahead of it on its path facing
    /// it, striking with a full charge (motions 0x65 / 0x68 / 0x5F / 0x62 and their releases at +2). The voices,
    /// the sound and the vibration are not ported.
    fn smash_vanish(&mut self, phase: u32) {
        if phase == ENTER {
            self.me.work[4] = match self.current() {
                0x5A => 0x65,
                0x5B => 0x68,
                0x5C => 0x5F,
                _ => 0x62,
            };
            self.me.set_fx(0xC);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            self.held(0xE);
            self.me.vanish_count += 1;
            self.me.attack_id = self.current();
        }
        if phase == RUN {
            let anim = self.me.work[4];
            match self.me.work[2] {
                0 => {
                    self.me.anim_advance(0);
                    self.me.work[3] += 1;
                    let count = self.me.work[3];
                    if count == 1 {
                        self.me.anim_play_sub(self.data, anim);
                        let lead = self.work_f(0) + self.me.obj.layers[1].length * 30.0 / 60.0;
                        self.set_work_f(0, lead);
                    }
                    if count == 2 {
                        self.me.anim_play_sub(self.data, anim + 2);
                        let frames = self.query_event(1, 1, 0) + self.query_event(2, 1, 0);
                        let lead = self.work_f(0) + frames as f32 * 0.5 * 30.0 / 60.0;
                        self.set_work_f(0, (lead + 0.5).floor());
                    }
                    if count >= 4 {
                        self.set(0xB);
                    }
                    if count >= 8 {
                        self.me.work[3] = 0;
                        self.me.work[2] += 1;
                    }
                }
                1 => {
                    snap_to_opponent(self.me, self.opp);
                    self.set(0xB);
                    self.set(0xCC);
                    self.me.work[2] += 1;
                }
                2 => {
                    let lead = self.work_f(0);
                    warp_ahead_of_opponent(
                        self.me,
                        self.data,
                        self.opp,
                        self.opp_data,
                        self.env,
                        lead,
                    );
                    self.me.turn_yaw(self.opp, 2, PI);
                    self.me
                        .turn_pitch(self.data, self.opp, self.opp_data, 1, PI);
                    self.me.turn_model_yaw(PI, 1.0);
                    self.me.set_direction(3);
                    self.set(0x3F);
                    self.set(0xB);
                    self.request_anim(anim, 0.0);
                    self.me.set_fx(0xD);
                    self.set(0xCC);
                    self.set(0x24);
                    self.me.work[2] += 1;
                }
                3 => {
                    let mut moving = true;
                    let id = self.anim_id();
                    match id {
                        0x5F | 0x62 | 0x65 | 0x68 => {
                            self.me.anim_advance_then(id + 2, 0.0, 0);
                            self.me.charge = 1.0;
                            if self.me.anim_progress() < 0.7 {
                                self.set(0xB);
                            }
                            if self.me.anim_is_new() {
                                self.set(0xCD);
                            }
                        }
                        0x61 | 0x64 | 0x67 | 0x6A => {
                            if self.me.anim_advance(0) {
                                let next = match self.opp_seen_action() {
                                    0x89..=0x8C => 0xC,
                                    _ => 0xB,
                                };
                                self.request(next);
                            }
                            self.me.obj.rate = 3.0;
                            if self.flag(0x5B) {
                                self.me.work[0] |= 1;
                            }
                            moving = !self.flag(0x5C);
                        }
                        _ => {}
                    }
                    if moving {
                        self.step(2, 1, 5, 0.0, kmh(100.0));
                    } else {
                        self.step(6, 6, 5, 0.0, kmh(100.0));
                    }
                }
                _ => {}
            }
            self.me.set_fx(0x3A);
            self.set(0x89);
        }
        if phase == DECIDE && self.me.work[0] & 1 != 0 && self.flag(0x5B) {
            self.set(0x3D);
            if self.opp_seen_action() == 0xD5 {
                if !self.has_queued() {
                    self.decide_attack(0x18_0000);
                }
                if 0.7 < self.me.anim_progress() {
                    let q = self.queued();
                    self.request(q);
                }
            }
        }
    }

    // ---- the attack records (btl_char_action.c, btl_act_change.c, btl_act_1.c) -----------------------------------

    /// `BtlOpp_GetSeenActionFrame`: the opponent's action frame as this frame sees it.
    fn opp_seen_action_frame(&self) -> i32 {
        if self.me.flags.stage() < 6 || self.opp.flags.stage() < 6 {
            self.opp.prev_action_frame
        } else {
            self.opp.action_frame
        }
    }

    /// `BtlAct_TestAttackSkill`: the skill bit record `index` needs (flags2 bits 0..31, flags3 from 32), or none.
    /// The original reads the word before the table for index -1; in the game's data the follow-up chosen is -1
    /// either way.
    fn test_attack_skill(&self, index: i32) -> bool {
        let Some(rec) = usize::try_from(index)
            .ok()
            .and_then(|i| self.env.common.attack_rec(i))
        else {
            return false;
        };
        let bit = i32::from(rec.need_skill);
        if bit < 0 {
            return true;
        }
        let p = &self.data.tables.params;
        if bit >= 0x20 {
            p.flags3() & 1u32.wrapping_shl((bit - 0x20) as u32) != 0
        } else {
            p.flags2() & 1u32.wrapping_shl(bit as u32) != 0
        }
    }

    /// `BtlAct_PrepareAttack`: the pending attack parameters from attack action `id`'s record.
    fn prepare_attack(&mut self, id: i32) {
        if !is_attack_id(id) {
            return;
        }
        let Some(rec) = self.env.common.attack_rec((id - 0x70) as usize) else {
            return;
        };
        let parts = i32::from(rec.parts);
        let motion = i32::from(rec.motion as i16);
        let cut = if rec.cut_alt < 0 {
            rec.cut
        } else if rec.flags & 0x20 != 0 {
            // The camera's side against the lead time just zeroed.
            if self.me.cam.side < 0.0 {
                rec.cut
            } else {
                rec.cut_alt
            }
        } else if self.frame_mod(2) != 0 {
            rec.cut
        } else {
            rec.cut_alt
        };
        let powered = self.flag(6) && !self.me.gauge.body_changed;
        let mut follow = [0; 2];
        for (i, f) in follow.iter_mut().enumerate() {
            let alt = i32::from(rec.follow_alt[i]);
            *f = if powered && self.test_attack_skill(alt) {
                alt
            } else {
                i32::from(rec.follow[i])
            };
        }
        self.me.next_attack = Attack {
            flags: i32::from(rec.flags),
            motion: if parts >= 2 {
                [motion, motion + 2]
            } else {
                [motion, -1]
            },
            parts: parts.max(1),
            lead_in: i32::from(rec.lead_in),
            cut: i32::from(cut),
            follow,
            rate: if rec.rate > 0 {
                f32::from(rec.rate) * 0.1
            } else {
                1.0
            },
            cancel_at: rec.cancel_at,
            lead_time: 0.0,
            speed: f32::from(rec.speed) * 10.0 * 0.009_259_259,
            chained: false,
        };
        self.me.attack_id = id;
    }

    /// `BtlDecide_QueueAttack`: attack action `id` when its skill bit is there, the opponent is seen in the state its
    /// kind needs (not on the frame the hit landed) and its input is made: its lead-in actions, then the attack,
    /// prepared. Returns how many actions were queued.
    fn queue_attack(&mut self, id: i32) -> i32 {
        if !is_attack_id(id) {
            return 0;
        }
        let idx = id - 0x70;
        let Some(rec) = self.env.common.attack_rec(idx as usize) else {
            return 0;
        };
        if !self.test_attack_skill(idx) {
            return 0;
        }
        let needs: &[i32] = match rec.kind {
            0 => &[],
            1 => &[0xD4],
            2 => &[0xD5],
            3 => &[0xD0],
            4 => &[0xCD],
            5 => &[0xCB],
            6 => &[0xC6],
            7 => &[0xC7, 0xC8],
            8 => &[0xCE],
            _ => &[-2],
        };
        if !needs.is_empty() && self.me.flags.raised(0x5B) {
            return 0;
        }
        let ok = rec.kind == 0 || needs.contains(&self.opp_seen_action());
        if !ok {
            return 0;
        }
        self.add_action_bit(id);
        if (1..=9).contains(&rec.input_kind) && !self.test(0x4E + rec.input_kind as u32) {
            return 0;
        }
        let lead: &[i32] = match rec.lead_in {
            1 => &[0x2B],
            2 => &[0x2B, 0x2D],
            3 => &[0x2B, 0x2E],
            4 => {
                if self.frame_mod(2) != 0 {
                    &[0x21]
                } else {
                    &[0x22]
                }
            }
            5 => &[0x2B, 0x2F],
            6 => &[0x31],
            7 => &[0x32],
            8 => &[0x2B, 0x30],
            9 => &[0x2C, 0x2D],
            _ => &[],
        };
        let mut queued = 0;
        for &a in lead {
            self.set_queue(queued, a);
            queued += 1;
        }
        self.set_queue(queued, id);
        self.prepare_attack(id);
        queued as i32 + 1
    }

    /// `BtlAct_DecideAttackFollow`: once the attack connects (flag 0x5B), its follow-up records and the inputs its
    /// flags allow, started past `cancelAt` unless it clashed (0x5D). The combo finishers of flags 0x100 / 0x200
    /// (`BtlAct_QueueComboFinish`) are not ported.
    fn decide_attack_follow(&mut self) {
        if !self.flag(0x5B) {
            return;
        }
        let atk = self.me.attack;
        for f in atk.follow {
            if f >= 0 && self.queue_attack(f + 0x70) != 0 {
                self.me.attack.chained = true;
            }
        }
        for (bit, mask) in [(1, 0x8_0000), (2, 0x10_0000), (4, 1)] {
            if atk.flags & bit != 0 && self.decide_attack(mask) != 0 {
                self.me.attack.chained = false;
            }
        }
        if atk.flags & 8 != 0 && self.test(0x34) {
            self.set_queue(0, 0x46);
            self.me.attack.chained = false;
        }
        if !self.flag(0x5D) && atk.cancel_at < self.me.anim_progress() {
            let q = self.queued();
            self.request(q);
            if self.me.attack.chained && atk.flags & 0x10 != 0 {
                self.held(0xDD);
            }
        }
    }

    /// `BtlAct_RequestAttackEnd`: 0xB, or 0xC when guarded (flag 0x5C) with attack flag 0x400.
    fn request_attack_end(&mut self) {
        let id = if self.flag(0x5C) && self.me.attack.flags & 0x400 != 0 {
            0xC
        } else {
            0xB
        };
        self.request(id);
    }

    /// The start of an attack action: latched, the flag and effect of the record (the camera cut,
    /// `ChrCam_RequestCut`, and the voice are not ported).
    fn begin_attack(&mut self) {
        self.me.attack = self.me.next_attack;
        if self.me.attack.flags & 0x80 != 0 {
            self.set(0xCD);
        }
        if (1..=3).contains(&self.me.attack.lead_in) {
            self.me.set_fx(0xD);
        }
    }

    /// Actions 0x70..0xAD (`BtlAct_AttackHandler`): the attack of the record `BtlAct_PrepareAttack` set up. Its
    /// parts play one after another (the earlier ones at twice the record's rate); with a record speed it homes on
    /// the opponent until it connects or is stopped. Of two attackers only the earlier gets its camera cut (flag
    /// 0x27; the cut itself is not ported).
    fn attack(&mut self, phase: u32) {
        if phase == ENTER {
            self.begin_attack();
            let m = self.me.attack.motion[0];
            self.play(m, 0.15);
            begin_rise_to_opponent(self.me, self.opp, self.data, self.opp_data);
            self.me.pose.speed = crate::btl_hit::launch_speed(self.me, self.data, self.env);
            if self.me.attack.cut >= 0
                && (!is_attack_id(self.opp_seen_action())
                    || self.me.action_frame < self.opp_seen_action_frame())
            {
                self.set(0x27);
            }
        }
        if phase == RUN {
            let atk = self.me.attack;
            self.me.charge = 1.0;
            let part = self.me.work[2];
            if part < atk.parts - 1 {
                self.me.anim_set_step(atk.rate * 2.0);
                let next = atk.motion.get((part + 1) as usize).copied().unwrap_or(-1);
                if self.me.anim_advance_then(next, 0.0, 0) {
                    self.me.work[2] += 1;
                }
            } else if self.me.anim_advance(0) {
                self.request_attack_end();
            }
            let (mut speed, mut accel, mut homing) = if atk.speed < 0.0001 {
                (0.0, kmh(100.0), false)
            } else {
                (atk.speed, 10000.0, true)
            };
            if self.flag(0x5B) {
                speed = 0.0;
                self.me.pose.speed = speed;
            }
            if self.flag(0x5C) || self.flag(0x5D) {
                speed = 0.0;
                accel = kmh(100.0);
                homing = false;
                if self.blocked() {
                    self.me.pose.speed = speed;
                }
            }
            if homing {
                self.step(2, 1, 3, speed, accel);
            } else {
                self.step(6, 6, 3, speed, accel);
            }
            self.me.apply_gravity();
            self.me.obj.rate = 2.0;
            self.set(0xD5);
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            self.decide_attack_follow();
        }
    }

    /// Actions 0x92, 0x9B, 0xAB (`BtlAct_AttackDashHandler`): a dash at the record's speed (motion 0x197, held
    /// flag 0x4F) for up to 21 frames or until contact, then the strike 0x198 in 0.3 s; the lean follows the pitch
    /// to the opponent within 45 degrees. The camera cut, the voice and the sound are not ported.
    fn attack_dash(&mut self, phase: u32) {
        if phase == ENTER {
            self.begin_attack();
            self.play(0x197, 0.0);
            self.held(0xE);
            self.held(0x4F);
        }
        if phase == RUN {
            let atk = self.me.attack;
            let (mut lean, mut accel, mut speed) = (0.0, kmh(100.0), 0.0);
            match self.anim_id() {
                0x197 => {
                    self.me.anim_advance_loop(0);
                    speed = atk.speed;
                    lean = 1.0;
                    accel = 10000.0;
                    self.me.set_fx(9);
                }
                0x198 => {
                    self.me.anim_set_duration(0.3);
                    if self.me.anim_advance(0) {
                        self.request_attack_end();
                    }
                    self.set(0x3F);
                    lean = 1.0 - self.me.anim_progress();
                }
                _ => {}
            }
            self.step(6, 6, 3, speed, accel);
            self.me.apply_gravity();
            self.set(0x1A);
            self.set(0xD5);
            let pitch = self.me.pose.pitch;
            let aim = opp_pitch_adjusted(self.me, self.opp, self.data, self.opp_data)
                .clamp(pitch - deg(45.0), pitch + deg(45.0));
            self.me.set_lean_x(aim * lean);
            self.set(0x28);
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            match self.anim_id() {
                0x197 => {
                    let mut end = false;
                    self.me.work[2] += 1;
                    if self.me.work[2] > 20 {
                        end = true;
                        self.me.pose.speed *= 0.5;
                    }
                    if self.flag(0x5B) || self.flag(0x5D) {
                        end = true;
                        self.me.pose.speed = 0.0;
                    }
                    if end {
                        self.request_anim(0x198, 0.0);
                        self.clear(0x4F);
                    }
                }
                0x198 => self.decide_attack_follow(),
                _ => {}
            }
        }
        if phase == LEAVE {
            self.clear(0x4F);
        }
    }

    /// Action 0x99 (`BtlAct_AttackVolleyDashHandler`): motion 0x2D into the dash loop 0x19 at the record's speed
    /// (held flag 0x4F from the loop's start, raised again two frames after it is found down, three times), then
    /// the strike 0x1A in 0.3 s after the fourth time, 24 frames or a guard. The camera cut, the voice, the sound
    /// and the vibration are not ported.
    fn attack_volley_dash(&mut self, phase: u32) {
        if phase == ENTER {
            self.begin_attack();
            self.play(0x2D, 0.15);
            self.held(0xE);
        }
        if phase == RUN {
            let atk = self.me.attack;
            let (mut speed, mut accel, mut lean) = (0.0, kmh(100.0), 0.0);
            match self.anim_id() {
                0x2D => {
                    self.me.anim_advance_then(0x19, 0.1, 0);
                }
                0x19 => {
                    self.me.anim_advance_loop(0);
                    speed = atk.speed;
                    accel = 10000.0;
                    lean = 1.0;
                    if self.me.anim_is_new() {
                        self.held(0x4F);
                    }
                    self.me.set_fx(9);
                    self.me.set_fx(4);
                }
                0x1A => {
                    self.me.anim_set_duration(0.3);
                    if self.me.anim_advance(0) {
                        self.request_attack_end();
                    }
                    lean = 1.0 - self.me.anim_progress();
                    self.clear(0x4F);
                    if self.blocked() {
                        self.me.pose.speed = speed;
                    }
                }
                _ => {}
            }
            self.step(2, 1, 3, speed, accel);
            self.me.apply_gravity();
            self.set(0x1A);
            self.set(0xD5);
            let pitch = self.me.pose.pitch;
            self.me.set_lean_x(pitch * lean);
        }
        if phase == DECIDE {
            match self.anim_id() {
                0x19 => {
                    let mut end = false;
                    if !self.flag(0x4F) {
                        if self.me.work[3] > 0 {
                            self.me.work[3] -= 1;
                            if self.me.work[3] <= 0 {
                                self.held(0x4F);
                            }
                        } else {
                            self.me.work[3] = 2;
                            self.me.work[4] += 1;
                            end = self.me.work[4] >= 4;
                        }
                    }
                    self.me.work[2] += 1;
                    if self.me.work[2] > 23 || self.flag(0x5C) {
                        end = true;
                    }
                    if end {
                        self.request_anim(0x1A, 0.15);
                    }
                }
                0x1A => self.decide_attack_follow(),
                _ => {}
            }
        }
        if phase == LEAVE {
            self.clear(0x4F);
        }
    }

    // ---- the lead-ins of the attack records (btl_act_2.c) ------------------------------------------------------

    /// `BtlAct_PlayAttackPart`: plays part `frame` of the pending attack on the sub layer and adds to the lead
    /// time: an earlier part its length (in 60ths) over the rate, the last half of its two first event frames,
    /// rounded.
    fn play_attack_part(&mut self, frame: i32, time: f32) -> f32 {
        let atk = self.me.next_attack;
        let motion = |i: i32| atk.motion.get(i as usize).copied().unwrap_or(-1);
        let mut time = time;
        if frame < atk.parts - 1 {
            self.me.anim_play_sub(self.data, motion(frame));
            time += self.me.obj.layers[1].length * 30.0 / 60.0 / atk.rate;
        } else if frame == atk.parts - 1 {
            self.me.anim_play_sub(self.data, motion(frame));
            let n = self.query_event(1, 1, 0) + self.query_event(2, 1, 0);
            time += n as f32 * 0.5 * 30.0 / 60.0;
            time = (time + 0.5).floor();
        }
        time
    }

    /// The queued action, or 0xB.
    fn request_queued_or_idle(&mut self) {
        if self.has_queued() {
            let q = self.queued();
            self.request(q);
        } else {
            self.request(0xB);
        }
    }

    /// Actions 0x2B, 0x2C (`BtlAct_VanishAttackHandler` / `...QuickHandler`): vanish (motion 0x26) for six frames
    /// (one for 0x2C) while the attack's lead time is measured, then onto the opponent's body and into the queued
    /// attack. The sound and the vibration are not ported.
    fn vanish_attack(&mut self, phase: u32) {
        if phase == ENTER {
            self.me.set_fx(0xC);
            self.held(0xE);
        }
        if phase == RUN {
            self.me.anim_advance_then(0x26, 0.15, 0);
            let t = self.play_attack_part(self.me.action_frame, self.me.next_attack.lead_time);
            self.me.next_attack.lead_time = t;
            match self.me.work[2] {
                0 => {
                    if self.current() == 0x2B {
                        self.me.work[3] += 1;
                        if self.me.work[3] >= 4 {
                            self.set(0xB);
                        }
                        if self.me.work[3] >= 6 {
                            self.me.work[3] = 0;
                            self.me.work[2] += 1;
                        }
                    } else {
                        self.set(0xB);
                        self.me.work[2] += 1;
                    }
                }
                1 => {
                    snap_to_opponent(self.me, self.opp);
                    self.set(0xB);
                    self.set(0xCC);
                    self.request_queued_or_idle();
                }
                _ => {}
            }
        }
        if phase == LEAVE {
            self.held(0xDD);
        }
    }

    /// Actions 0x2D, 0x2E (`BtlAct_WarpBehindAttackHandler` / `WarpAheadAttackHandler`): on the first frame behind
    /// the opponent, or ahead of it on its path by the attack's lead time, facing it; then the queued attack.
    fn warp_attack(&mut self, phase: u32) {
        if phase == RUN {
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            if !self.flag(0xBA) {
                if self.current() == 0x2D {
                    warp_behind_opponent(self.me, self.data, self.opp, self.opp_data, 0.0);
                } else {
                    let lead = self.me.next_attack.lead_time;
                    warp_ahead_of_opponent(
                        self.me,
                        self.data,
                        self.opp,
                        self.opp_data,
                        self.env,
                        lead,
                    );
                }
                self.me.turn_yaw(self.opp, 2, PI);
                self.me
                    .turn_pitch(self.data, self.opp, self.opp_data, 1, PI);
                self.me.turn_model_yaw(PI, 1.0);
                self.me.set_direction(3);
                for f in [0x3F, 0xB, 0xCC, 0x24] {
                    self.set(f);
                }
            }
            self.request_queued_or_idle();
        }
        if phase == LEAVE {
            self.held(0xDD);
        }
    }

    /// Actions 0x2F, 0x30 (`BtlAct_SlideToAttackHandler`): four frames sliding to a point beside where the opponent
    /// will be (35 degrees below at 11, or 70 at 8 times the attack's speed), then facing that point.
    fn slide_attack(&mut self, phase: u32) {
        if phase == ENTER {
            let (angle, dist, extra, frames) = if self.current() == 0x2F {
                (-35.0 * PI / 180.0, 11.0, 1.0, 4.0)
            } else {
                (-70.0 * PI / 180.0, 8.0, 0.0, 4.0)
            };
            let speed = self.me.next_attack.speed;
            let (point, target) = calc_approach_point(
                self.me,
                self.opp,
                self.data,
                self.opp_data,
                self.env,
                speed,
                angle,
                dist,
                dist + frames + extra,
            );
            let step = scale3(sub(point, self.me.pose.pos), 0.25);
            for k in 0..3 {
                self.set_work_f(6 + k, step[k]);
                self.set_work_f(10 + k, target[k]);
            }
        }
        if phase == RUN {
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            let step = [self.work_f(6), self.work_f(7), self.work_f(8)];
            self.me.pose.pos = add(self.me.pose.pos, step);
            self.me.work[2] += 1;
            if self.me.work[2] == 4 {
                self.request_queued_or_idle();
            }
            for f in [0x3F, 0xB, 0xCC, 0x24] {
                self.set(f);
            }
        }
        if phase == LEAVE {
            let face = [self.work_f(10), self.work_f(11), self.work_f(12)];
            self.me.turn_to_point(face, PI, PI);
            self.held(0xDD);
        }
    }

    /// Action 0x31 (`BtlAct_RushToAttackHandler`): dashing at the opponent (motion 0xD) at the attack's speed until
    /// the closing time is below the attack's lead time, it is blocked or the lock-on is lost: then the queued
    /// attack; stuck for eight frames, the stop motion 0xE. The sound and the vibration are not ported.
    fn rush_attack(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xD, 0.15);
            self.held(0xE);
            self.set(0xD1);
        }
        if phase == RUN {
            let mut speed = 0.0;
            let lead = self.play_attack_part(self.me.action_frame, self.work_f(0));
            self.set_work_f(0, lead);
            let mut accel = kmh(100.0);
            match self.anim_id() {
                0xD => {
                    self.me.anim_advance_loop(0);
                    speed = self.me.next_attack.speed;
                    accel = 10000.0;
                    self.me.set_fx(4);
                    if self.flag(0xF) {
                        self.me.set_fx(0x30);
                    }
                    self.set(9);
                    self.set(0x50);
                    self.set(0x51);
                    self.me.set_fx(9);
                }
                0xE => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                }
                _ => {}
            }
            self.step(2, 1, 3, speed, accel);
            self.me.apply_gravity();
        }
        if phase == DECIDE && self.anim_id() == 0xD {
            let mut reach = opp_closing_time(self.me, self.opp) < self.work_f(0);
            if self.blocked() {
                reach = true;
            }
            let mut stuck = false;
            if length(self.me.pose.moved) < kmh(100.0) {
                self.me.work[2] += 1;
                if self.me.work[2] >= 8 {
                    stuck = true;
                }
            } else {
                self.me.work[2] = 0;
            }
            if !self.flag(5) {
                reach = true;
            }
            if reach || stuck {
                self.me.pose.speed = 0.0;
                if self.has_queued() && !stuck {
                    let q = self.queued();
                    self.request(q);
                } else {
                    self.request_anim(0xE, 0.0);
                }
            }
        }
        if phase == LEAVE {
            self.held(0xDD);
        }
    }

    /// Action 0x32 (`BtlAct_HopBackAttackHandler`): a hop back (motion 0x28), the queued attack half way through.
    /// The sound is not ported.
    fn hop_back_attack(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x28, 0.15);
            self.held(0xE);
        }
        if phase == RUN {
            let speed = self.me.move_speed(self.data, self.env, 0xB);
            self.me.anim_advance(0);
            if self.me.anim_progress() > 0.5 {
                self.request_queued_or_idle();
            }
            self.step(2, 5, 4, speed, kmh(100.0));
            self.me.apply_gravity();
        }
        if phase == LEAVE {
            self.held(0xDD);
        }
    }

    // ---- the rush finishers (btl_act_1.c, btl_act_2.c) ----------------------------------------------------------

    /// `BtlParam_GetRushFinisher` / `GetChainKind` / `GetFinisherChoice`: parameter bytes +0x84 (4), +0x88 (2),
    /// +0x8A (5); an index past the end reads entry 0.
    fn rush_finisher(&self, n: u32) -> u8 {
        let n = if n >= 4 { 0 } else { n };
        self.data.tables.params.u8_at(0x84 + n as usize)
    }

    fn chain_kind(&self, n: u32) -> u8 {
        let n = if n >= 2 { 0 } else { n };
        self.data.tables.params.u8_at(0x88 + n as usize)
    }

    fn finisher_choice(&self, n: u32) -> u8 {
        let n = if n >= 5 { 0 } else { n };
        self.data.tables.params.u8_at(0x8A + n as usize)
    }

    /// The finisher 0x64 / 0x65 by held flag 0x8F (each one toggles it).
    fn alternating_finisher(&self) -> i32 {
        if self.flag(0x8F) { 0x64 } else { 0x65 }
    }

    /// `BtlAct_GetChainAction`: after 0x67..0x69, kind 0 the lunge 0x6A, 1..4 the attack 0x9E..0xA1 when it can be
    /// queued (its first queued action), else 0x6A.
    fn chain_action(&mut self, kind: u8) -> i32 {
        let id = match kind {
            1 => 0x9E,
            2 => 0x9F,
            3 => 0xA0,
            4 => 0xA1,
            _ => return 0x6A,
        };
        if self.queue_attack(id) != 0 {
            self.queued()
        } else {
            0x6A
        }
    }

    /// `BtlAct_StartSub19B`: motion 0x19B on the sub layer, and the burst node by the side of the first node its
    /// fire events name (0x2D, 0x1F, else 0x15).
    fn start_sub_19b(&mut self) -> i32 {
        self.me.anim_play_sub(self.data, 0x19B);
        let mask = self.query_event(1, 1, 6) as u32;
        for i in 0..32 {
            if mask & (1 << i) != 0 {
                match node_side(mask_to_node(1 << i)) {
                    0 => return 0x2D,
                    1 => return 0x1F,
                    _ => {}
                }
            }
        }
        0x15
    }

    /// Actions 0x5E..0x62, 0x64..0x66 (`BtlAct_RushFinishHandler`): the chargeable rush finishers, motions wind-up
    /// -> hold (the charge follows its progress; letting go, input 0x43, releases) -> release (0x41.., 0x44..,
    /// 0x47.., 0x56.., 0x59.., 0x90.., and 0x199.. for 0x66, whose release is followed by 0x19C). On a hit each
    /// offers its attack records and the rush. The sounds, the voice and the vibration are not ported.
    fn rush_finish(&mut self, phase: u32) {
        if phase == ENTER {
            let anim = match self.current() {
                0x5E => 0x41,
                0x5F => 0x44,
                0x60 => 0x47,
                0x61 | 0x62 => 0x90,
                0x66 => 0x199,
                0x64 => {
                    self.me.flags.toggle_held(0x8F);
                    0x56
                }
                0x65 => {
                    self.me.flags.toggle_held(0x8F);
                    0x59
                }
                _ => 0,
            };
            self.play(anim, 0.1);
            begin_rise_to_opponent(self.me, self.opp, self.data, self.opp_data);
            if self.current() == 0x66 {
                self.me.burst_node = self.start_sub_19b();
            }
        }
        if phase == RUN {
            let (mut speed, mut mode, mut turn) = (0.0, 6, 0.0);
            if self.flag(0x81) {
                speed = kmh(50.0);
                mode = 2;
                turn = deg(1.8);
            }
            let id = self.anim_id();
            match id {
                0x41 | 0x44 | 0x47 | 0x56 | 0x59 | 0x90 | 0x199 => {
                    self.set_charge_anim_step();
                    self.me.anim_advance_then(id + 1, 0.0, 0);
                }
                0x42 | 0x45 | 0x48 | 0x57 | 0x5A | 0x91 | 0x19A => {
                    self.set_charge_anim_step();
                    self.me.anim_advance_then(id + 1, 0.0, 0);
                    self.me.charge = self.me.anim_progress();
                    if self.current() == 0x66 && self.me.anim_is_new() {
                        self.me.set_fx(0x28);
                    }
                }
                0x43 | 0x46 | 0x49 | 0x58 | 0x5B | 0x92 | 0x19B => {
                    speed = 0.0;
                    turn = 0.0;
                    mode = 6;
                    if self.current() == 0x66 {
                        if self.me.anim_is_new() {
                            self.me.set_fx(0x29);
                        }
                        self.me.anim_advance_then(id + 1, 0.0, 0);
                        speed = crate::btl_hit::launch_speed(self.me, self.data, self.env);
                        if self.flag(0x13) {
                            self.set(0xDC);
                        }
                    } else {
                        if self.me.anim_is_new() {
                            self.me.pose.speed =
                                crate::btl_hit::launch_speed(self.me, self.data, self.env);
                        }
                        if self.me.anim_advance(0) {
                            self.request(0xB);
                        }
                    }
                    if self.blocked() && (self.flag(0x5B) || self.flag(0x5D)) {
                        turn = PI;
                        mode = 2;
                    }
                }
                0x19C => {
                    if self.me.anim_is_new() {
                        self.me.set_fx(0x2A);
                    }
                    self.me.anim_set_duration(0.5);
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    speed = 0.0;
                    mode = 6;
                    turn = 0.0;
                }
                _ => {}
            }
            self.me.turn_yaw(self.opp, mode, turn);
            self.me
                .turn_pitch(self.data, self.opp, self.opp_data, 5, PI);
            self.me.turn_model_yaw(deg(36.0), 0.3);
            self.me.set_direction(2);
            self.me.advance(speed, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE {
            let id = self.anim_id();
            match id {
                0x42 | 0x45 | 0x48 | 0x57 | 0x5A | 0x91 | 0x19A => {
                    if self.test(0x43) {
                        self.request_anim(id + 1, 0.0);
                    }
                }
                0x43 => {
                    if self.flag(0x5B) {
                        if self.opp_seen_action() == 0xCE {
                            self.add_action_bit(0x34);
                            if self.test(0x2A) {
                                let first = self.query_event(1, 0, 0) as f32;
                                let at = (first + 20.0) / self.me.obj.layers[0].length;
                                self.set_work_f(0, at.clamp(0.2, 0.8));
                                self.set_queue(0, 0x34);
                            }
                        }
                        if self.has_queued() && self.work_f(0) < self.me.anim_progress() {
                            let q = self.queued();
                            self.request(q);
                        }
                    }
                    self.cancel_on_opp_c9(0.6);
                }
                0x46 | 0x49 | 0x58 | 0x5B | 0x92 => {
                    if self.flag(0x5B) {
                        let follow: &[i32] = match self.current() {
                            0x5F => &[0x80, 0x70],
                            0x60 => &[0x7E, 0x74],
                            0x61 if 0.99 < self.me.charge => &[0x73, 0x96],
                            0x62 => &[0x82],
                            0x64 => &[0x99, 0x85],
                            0x65 => &[0x99, 0x84],
                            _ => &[],
                        };
                        for &f in follow {
                            if self.queue_attack(f) != 0 {
                                self.set_work_f(0, 0.4);
                                break;
                            }
                        }
                        if self.decide_attack(3) != 0 {
                            self.set_work_f(0, 0.6);
                        }
                        if self.has_queued() && self.work_f(0) < self.me.anim_progress() {
                            let q = self.queued();
                            self.request(q);
                        }
                    }
                    self.cancel_on_opp_c9(0.6);
                }
                0x19C => {
                    if self.flag(0x5B) {
                        let low = crate::btl_hit::launch_angle_b_of(self.data, 0x86) < deg(10.0);
                        if self.queue_attack(if low { 0xA3 } else { 0x93 }) != 0 {
                            self.set_work_f(0, 0.9);
                        }
                        if self.opp_seen_action() != 0xD5 && self.decide_attack(3) != 0 {
                            self.set_work_f(0, 0.6);
                        }
                        if self.has_queued() && self.work_f(0) < self.me.anim_progress() {
                            let q = self.queued();
                            self.request(q);
                        }
                    }
                    self.cancel_on_opp_c9(0.6);
                }
                _ => {}
            }
        }
        if phase == LEAVE && self.current() == 0x66 {
            self.me.set_fx(0x2A);
        }
    }

    /// Action 0x63 (`BtlAct_Action63`): the finisher that is not chargeable, motion 0x93; the rush and the smash
    /// are read all through and start past 90%. The voice is not ported.
    fn rush_finish_plain(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x93, 0.1);
            begin_rise_to_opponent(self.me, self.opp, self.data, self.opp_data);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            self.step(6, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE {
            self.decide_attack(3);
            if self.me.anim_progress() > 0.9 {
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// Action 0x40 (`BtlAct_Action40`): motion 0xDE untouchable (flag 0x42), a queued action past 60%. The sound
    /// is not ported.
    fn feint(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xDE, 0.15);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            self.step(6, 5, 3, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            self.set(0x42);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE && 0.6 < self.me.anim_progress() && self.has_queued() {
            let q = self.queued();
            self.request(q);
        }
    }

    /// Actions 0x67..0x69 (`BtlAct_Action67to69`): the ki-spending pose 0xF7 (parameter +0x4C a second; ability
    /// 0x28 is not ported); out of ki it guards, without the guard input it idles; flag 0x70 chains into
    /// `BtlAct_GetChainAction` by the parameter's chain kind. The voice and the vibration are not ported.
    fn ki_pose(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xF7, 0.15);
        }
        if phase == RUN {
            let drain = self.data.tables.params.s32_at(0x4C) / 30;
            self.me.spend_ki(drain, false);
            if self.me.anim_advance(0) {
                if self.me.is_ki_empty() {
                    self.request(0x38);
                }
                if self.me.test(0x22, false) {
                    self.request(0xB);
                }
            }
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.current() == 0x67 && self.me.action_frame < 10 {
                self.set(0x4E);
            }
        }
        if phase == DECIDE && self.flag(0x70) {
            let kind = self.chain_kind(if self.current() == 0x69 { 1 } else { 0 });
            let next = self.chain_action(kind);
            self.request(next);
        }
    }

    /// Action 0x6A (`BtlAct_Action6A`): motion 0xF8, a lunge from 1000 km/h braking; on a hit attack 0xA2 past 60%.
    fn lunge(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xF8, 0.15);
            self.me.pose.speed = kmh(1000.0);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            self.me.charge = 1.0;
            self.step(2, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE && self.flag(0x5B) {
            self.queue_attack(0xA2);
            if self.me.anim_progress() > 0.6 {
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// Actions 0x6B..0x6F (`BtlAct_Action6Bto6F`): one strike each (motions 0x67, 0x6A, 0x61, 0x64, 0x5E) homing at
    /// 1500 km/h; on a hit the follow-ups of the opponent's state. The camera cut after a dash and the vibration
    /// are not ported.
    fn homing_strike(&mut self, phase: u32) {
        if phase == ENTER {
            let anim = match self.current() {
                0x6B => 0x67,
                0x6C => 0x6A,
                0x6D => 0x61,
                0x6E => 0x64,
                _ => 0x5E,
            };
            self.play(anim, 0.1);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            let (mut speed, mut accel, mut homing) = (kmh(1500.0), 10000.0, true);
            if self.flag(0x5B) {
                speed = 0.0;
                self.me.pose.speed = speed;
            }
            if self.flag(0x5C) || self.flag(0x5D) {
                speed = 0.0;
                accel = kmh(100.0);
                homing = false;
                if self.blocked() {
                    self.me.pose.speed = speed;
                }
            }
            if homing {
                self.step(2, 1, 3, speed, accel);
            } else {
                self.step(6, 6, 3, speed, accel);
            }
            self.me.apply_gravity();
            self.me.obj.rate = 2.0;
            self.set(0x89);
            self.set(9);
            self.set(0xD5);
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE && self.flag(0x5B) {
            self.set(0x3D);
            match self.opp_seen_action() {
                0xD5 => {
                    if !self.has_queued() {
                        self.decide_attack(0x18_0000);
                    }
                    if self.me.anim_progress() > 0.7 {
                        let q = self.queued();
                        self.request(q);
                    }
                }
                0xC6 => {
                    if !self.has_queued() {
                        self.decide_main(0x8000);
                        self.decide_attack(1);
                    }
                    if self.me.anim_progress() > 0.6 {
                        let q = self.queued();
                        self.request(q);
                    }
                }
                _ => {}
            }
        }
    }

    // ---- the circle, rush and vanish dashes and the search (btl_act_2.c) ----------------------------------------

    /// Action 0x33 (`BtlAct_CircleDashHandler`): round the opponent (motions 0x2D -> 0x19 loop -> 0x1A) toward the
    /// point opposite where the fighter faced at the start, on the side the opponent is turned from, spending the
    /// ki of kind 3; close and far enough round, attack record 0x90 or a homing strike. The voice, the sound, the
    /// vibration and the camera cut are not ported.
    fn circle_dash(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x2D, 0.15);
            self.held(0xE);
            self.me.set_fx(6);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            if 0.0 < opp_yaw_from_its_facing(self.me, self.opp) {
                self.me.work[0] |= 1;
            }
            let base = wrap_angle(self.me.pose.rot[1] + PI);
            self.set_work_f(0, base);
        }
        if phase == RUN {
            let (mut circling, mut lean) = (false, 0.0);
            match self.anim_id() {
                0x2D => {
                    self.me.anim_advance_then(0x19, 0.15, 0);
                    lean = 1.0;
                }
                0x19 => {
                    self.me.anim_advance_loop(0);
                    lean = 1.0;
                    circling = true;
                }
                0x1A => {
                    self.me.anim_set_duration(0.3);
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    lean = 1.0 - self.me.anim_progress();
                }
                _ => {}
            }
            if circling {
                let speed = self.me.move_speed(self.data, self.env, 0x13);
                let side = self.me.work[0] & 1 != 0;
                let lead = self.opp_seen_action() == 0xD5;
                let base = self.work_f(0);
                if circle_opponent(
                    self.me,
                    self.opp,
                    self.data,
                    self.opp_data,
                    self.env,
                    side,
                    lead,
                    base,
                    speed,
                ) {
                    self.me.work[0] |= 2;
                }
                self.set(9);
                self.set(0x50);
                self.set(0x51);
                self.me.set_fx(4);
                self.set(0x47);
            } else {
                let yaw = if self.me.work[0] & 2 != 0 { 2 } else { 6 };
                self.step(yaw, 6, 7, 0.0, kmh(100.0));
                self.me.apply_gravity();
            }
            let pitch = self.me.pose.pitch;
            self.me.set_lean_x(pitch * lean);
            self.set(0x89);
        }
        if phase == DECIDE {
            let mut stop = false;
            match self.anim_id() {
                0x2D => {
                    if self.blocked() {
                        stop = true;
                        self.me.pose.speed = 0.0;
                    }
                }
                0x19 => {
                    if self.blocked() {
                        stop = true;
                        self.me.pose.speed = 0.0;
                    }
                    if self.me.work[0] & 2 != 0 {
                        stop = true;
                        self.me.pose.speed = 0.0;
                    }
                    let cost = self.move_ki_cost(3);
                    if self.me.spend_ki(cost, false) {
                        stop = true;
                        self.me.pose.speed *= 0.2;
                    }
                    if !self.flag(5) {
                        stop = true;
                    }
                    if length(self.me.pose.moved) < kmh(1000.0) {
                        self.me.work[2] += 1;
                        if self.me.work[2] >= 6 {
                            stop = true;
                            self.me.pose.speed = 0.0;
                        }
                    } else {
                        self.me.work[2] = 0;
                    }
                }
                _ => {}
            }
            if stop {
                self.request_anim(0x1A, 0.15);
            }
            if self.flag(0x13) {
                let limit = if self.anim_id() == 0x1A {
                    PI / 2.0
                } else {
                    PI / 4.0
                };
                let round = wrap_angle(opp_yaw(self.me, self.opp) - self.work_f(0));
                if round.abs() < limit {
                    if self.queue_attack(0x90) != 0 {
                        let q = self.queued();
                        self.request(q);
                    }
                    if self.decide_attack(0x400_0000) != 0 {
                        let q = self.queued();
                        self.request(q);
                    }
                }
            }
        }
    }

    /// Action 0x34 (`BtlAct_RushDashHandler`): a dash at the opponent (motions 0x23 -> 0x24 loop -> 0x25 stop) at
    /// 1800 km/h, or vanished and untouchable at 2200 km/h with parameter flag bit 20; attack record 0x9C, the
    /// rush 0x46 (input 0x34) or 0xB5 (input 0x5D) take over. The sounds and the vibration are not ported.
    fn rush_dash(&mut self, phase: u32) {
        let fast = (self.data.tables.params.flags() >> 20) & 1 != 0;
        if phase == ENTER {
            self.play(0x23, 0.15);
            self.held(0xE);
            if fast {
                self.me.set_fx(0xE);
            }
        }
        if phase == RUN {
            let (mut yaw, mut accel, mut pitch) = (2, 10000.0, 1);
            let mut speed = if fast {
                for f in [0xB, 0x42, 0x43, 0x44, 0x45, 0x46] {
                    self.set(f);
                }
                if self.me.action_frame != 0 {
                    kmh(2200.0)
                } else {
                    0.0
                }
            } else {
                kmh(1800.0)
            };
            match self.anim_id() {
                0x23 => {
                    self.me.anim_advance_then(0x24, 0.15, 0);
                }
                0x24 => {
                    self.me.anim_advance_loop(0);
                }
                0x25 => {
                    self.me.anim_set_duration(0.1);
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                        self.me.pose.speed = 0.0;
                        self.me.pose.fall_speed = 0.0;
                    }
                    speed = 0.0;
                    yaw = 6;
                    accel = kmh(100.0);
                    pitch = 6;
                }
                _ => {}
            }
            self.step(yaw, pitch, 3, speed, accel);
            self.me.apply_gravity();
            self.me.set_fx(9);
            self.me.set_fx(0x3A);
        }
        if phase == DECIDE {
            if self.anim_id() == 0x24 && self.blocked() {
                self.request_anim(0x25, 0.0);
            }
            if self.queue_attack(0x9C) != 0 {
                let q = self.queued();
                self.request(q);
            }
            if self.test(0x34) {
                self.request(0x46);
            }
            if self.test(0x5D) {
                self.request(0xB5);
            }
        }
        if phase == LEAVE && fast {
            self.me.set_fx(0xD);
        }
    }

    /// Action 0x35 (`BtlAct_VanishDashHandler`): vanished and untouchable at 2000 km/h the way the stick points
    /// (motions 0xC -> 0xD loop -> 0xE), for 13 frames or until it meets the opponent head on; from the seventh
    /// frame the main inputs. The sound is not ported.
    fn vanish_dash(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xC, 0.15);
            self.me.set_fx(0xE);
            self.held(0xE);
            self.me.turn_yaw(self.opp, 0, PI);
            self.me
                .turn_pitch(self.data, self.opp, self.opp_data, 2, PI);
            self.me.turn_model_yaw(PI, 1.0);
        }
        if phase == RUN {
            let mut moving = true;
            match self.anim_id() {
                0xC => {
                    self.me.anim_set_duration(0.05);
                    self.me.anim_advance_then(0xD, 0.15, 0);
                    if self.me.anim_is_new() {
                        moving = false;
                    }
                }
                0xD => {
                    self.me.anim_advance_loop(0);
                    self.set(0xB);
                }
                0xE => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    if self.me.anim_is_new() {
                        self.me.set_fx(0xD);
                        self.me.pose.speed = kmh(1000.0);
                    }
                    if self.blocked() {
                        self.me.pose.speed = 0.0;
                    }
                    moving = false;
                }
                _ => {}
            }
            if moving {
                self.step(6, 6, 3, kmh(2000.0), 10000.0);
                for f in [9, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47] {
                    self.set(f);
                }
            } else {
                self.step(6, 6, 3, 0.0, kmh(100.0));
            }
            self.me.apply_gravity();
            self.me.request_orbit(50.0, 100.0);
            self.set(0xBC);
        }
        if phase == DECIDE {
            match self.anim_id() {
                0xC | 0xD => {
                    let mut stop = self.me.action_frame >= 0xD;
                    if self.blocked() && opp_yaw_from_facing(self.me, self.opp).abs() < PI / 4.0 {
                        stop = true;
                    }
                    if stop {
                        self.request_anim(0xE, 0.15);
                    }
                }
                0xE => {
                    self.decide_attack(3);
                    let q = self.queued();
                    self.request(q);
                }
                _ => {}
            }
            if self.me.action_frame >= 7 {
                self.decide_main(0x2000_0000);
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// `BtlParam_GetType`: the search type of the costume (parameter +0x58; 2..4 become 7 / 8 in a variant).
    fn search_type(&self) -> usize {
        let costume = if (0..4).contains(&self.data.costume) {
            self.data.costume as usize
        } else {
            0
        };
        let t = i32::from(self.data.tables.params.u8_at(0x58 + costume) as i8);
        let t = match t {
            2..=4 if self.me.gauge.variant != 0 => {
                if t == 4 {
                    8
                } else {
                    7
                }
            }
            _ => t,
        };
        t.max(0) as usize
    }

    /// `BtlAct_GrowSearchWindow`: the window widens by the search type's angle and range (60% against an opponent
    /// with parameter word bit 0x80; ability 0x6B is not ported), up to pi and 1000000; true when the angle passes
    /// `limit`.
    fn grow_search_window(&mut self, limit: f32) -> bool {
        const ANGLE: [f32; 10] = [
            0.022_577_715,
            0.027_516_59,
            0.062_895_06,
            0.110_066_36,
            0.293_510_3,
            0.022_013_27,
            0.030_363_134,
            0.016_933_287,
            0.019_567_352,
            0.025_158_024,
        ];
        const RANGE: [f32; 10] = [
            3000.0, 3500.0, 20000.0, 30000.0, 40000.0, 3000.0, 4000.0, 2000.0, 2500.0, 3000.0,
        ];
        let t = self.search_type();
        let slow = (self.opp_data.tables.params.chara_flags() >> 7) & 1 != 0;
        let mut d_angle = ANGLE.get(t).copied().unwrap_or(0.0);
        let mut d_range = RANGE.get(t).map_or(0.0, |&v| kmh(v));
        if slow {
            d_angle *= 0.6;
            d_range *= 0.6;
        }
        let old = self.me.search_angle;
        self.me.search_angle = (old + d_angle).min(PI);
        self.me.search_range = (self.me.search_range + d_range).min(1_000_000.0);
        old < limit && limit <= self.me.search_angle
    }

    /// `BtlAct_IsOppInSearchWindow`: the opponent within the window's angle of the facing and its range (narrower
    /// to the sides), unless it is hidden (flag 0xBA) or cannot be targeted (0xB2). Ability 0x6C is not ported.
    fn opp_in_search_window(&self) -> bool {
        if self.flag(0xB2) {
            return false;
        }
        let dist = opp_distance_xz(self.me, self.opp);
        let yaw = opp_yaw_from_facing(self.me, self.opp).abs();
        yaw <= self.me.search_angle
            && dist < self.me.search_range * ((1.0 - yaw / PI) * 0.85 + 0.15)
            && !self.flag(0xBA)
    }

    /// Action 0x36 (`BtlAct_SearchHandler`): looking for an opponent lost from sight (motion 0x17F for search
    /// types 2..4, then the loop 0x17E) while the search window grows; found, the lock-on comes back and 0xB
    /// follows. Leaving resets the window. The voices and the sound are not ported.
    fn search(&mut self, phase: u32) {
        if phase == ENTER {
            let looks = matches!(self.search_type(), 2..=4);
            self.play(if looks { 0x17F } else { 0x17E }, 0.15);
        }
        if phase == RUN {
            let mut grow = false;
            match self.anim_id() {
                0x17F => {
                    self.me.anim_advance_then(0x17E, 0.15, 0);
                    if 0.5 < self.me.anim_progress() {
                        grow = true;
                    }
                }
                0x17E => {
                    self.me.anim_advance_loop(0);
                    grow = true;
                }
                _ => {}
            }
            if grow {
                self.grow_search_window(PI / 2.0);
            }
            if self.opp_in_search_window() {
                self.held(5);
                self.me.turn_yaw(self.opp, 2, PI);
                if !self.flag(0xBA) {
                    self.me.set_fx(0x15);
                }
            }
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xC9);
            self.set(0x1B);
            self.set(0x1C);
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.flag(0xF) {
                self.clear(0xE);
            }
            if self.flag(5) {
                self.request(0xB);
            }
        }
        if phase == DECIDE {
            self.decide_main(0x48_1FEB);
            self.decide_attack(3);
            self.decide_common(6);
            let q = self.queued();
            self.request(q);
        }
        if phase == LEAVE {
            self.me.search_angle = 0.5;
            self.me.search_range = 500.0;
        }
    }

    // ---- the dodges, the poses and the rapid rush (btl_act_1.c, btl_act_2.c) -----------------------------------

    /// `BtlMove_IsBlastIncoming`: a live ki blast record (of the uncharged kinds 0 / 4 / 8, or the others) heading
    /// for the fighter that reaches it within `limit` frames. Technique records are not ported, so `techniques`
    /// finds nothing.
    fn blast_incoming(&self, uncharged: bool, others: bool, _techniques: bool, limit: f32) -> bool {
        self.blasts.iter().any(|rec| {
            let plain = matches!(rec.atk.kind, 0 | 4 | 8);
            if (plain && !uncharged) || (!plain && !others) {
                return false;
            }
            let step = sub(rec.pos, rec.prev);
            let d = sub(self.me.pose.pos, rec.pos);
            if dot(d, step) < 0.0 {
                return false;
            }
            let speed = length(step);
            speed >= 0.001 && length(d) / speed <= limit
        })
    }

    /// `BtlAct_GetEvasionAttack`: by both fighters' evasion counts, record 0x89, 0x8A, or 0x8B / 0x8C by height.
    fn evasion_attack(&self) -> i32 {
        match (self.me.evasion_count + self.opp.evasion_count).rem_euclid(3) {
            1 => 0x89,
            2 => 0x8A,
            _ => {
                if self.me.height_ratio(self.data, self.env) > 0.5 {
                    0x8C
                } else {
                    0x8B
                }
            }
        }
    }

    /// Action 0x39 (`BtlAct_Action39`, forced by flag 0x7B): motion 0xEC, then the evasion attack.
    fn evade(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xEC, 0.0);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                let id = self.evasion_attack();
                self.queue_attack(id);
                let q = self.queued();
                self.request(q);
            }
            self.step(6, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xC9);
            self.set(0xD5);
            if self.flag(0xF) && kmh(200.0) < length(self.me.pose.moved) {
                self.me.set_fx(0x32);
            }
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
    }

    /// Action 0x3B (`BtlAct_Action3B`, forced by flag 0x6B or before a blast): the swatting pose 0xF9 / 0xFA; four
    /// swats (refilled by input 0x2B, emptied when the guard is let go), one off at each motion's end.
    fn swat(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xF9, 0.15);
            self.me.work[2] = 4;
        }
        if phase == RUN {
            self.me.anim_advance(0);
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0x1C);
            self.set(0x50);
            self.set(0x51);
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.flag(0xF) {
                self.clear(0xE);
            }
        }
        if phase == DECIDE {
            if self.test(0x2B) {
                self.me.work[2] = 4;
            }
            if self.me.test(0x22, false) {
                self.me.work[2] = 0;
            }
            if self.flag(anim_flag::END) {
                self.me.work[2] -= 1;
                if self.me.work[2] <= 0 {
                    self.request(0xB);
                } else {
                    let next = if self.anim_id() == 0xF9 { 0xFA } else { 0xF9 };
                    self.request_anim(next, 0.0);
                }
            }
            self.decide_common(6);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0x3C (`BtlAct_Action3C`): motion 0xFD with flag 0x52 (a blast about to hit is let pass), then 0xB.
    /// The voice is not ported.
    fn duck_blast(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xFD, 0.15);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0x1C);
            self.set(0x52);
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.flag(0xF) {
                self.clear(0xE);
            }
        }
        if phase == DECIDE {
            self.decide_main(0x1C6A);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0x3D (`BtlAct_Action3D`, for characters with parameter flag 2): 0xF9 into the loop 0xFA with flag
    /// 0x54 until the guard is let go. The sound is not ported.
    fn block_blasts(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xF9, 0.15);
        }
        if phase == RUN {
            match self.anim_id() {
                0xF9 => {
                    self.me.anim_advance_then(0xFA, 0.0, 0);
                }
                0xFA => self.me.anim_advance_loop(0),
                _ => {}
            }
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0x1C);
            self.set(0x54);
            if self.flag(0x11) {
                self.held(0xE);
            }
            if self.flag(0xF) {
                self.clear(0xE);
            }
        }
        if phase == DECIDE {
            if self.me.test(0x22, false) {
                self.request(0xB);
            }
            self.decide_common(6);
            let q = self.queued();
            self.request(q);
        }
    }

    /// Action 0x3E (`BtlAct_Action3E`): a step in the air (motion 0xFB) at dash speed round the opponent, then the
    /// dash 0xF; input 0x2C past 40% queues another, taken at 70%; input 0x40 the flying kick. The voice is not
    /// ported.
    fn air_step(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xFB, 0.15);
            self.held(0xE);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xF);
            }
            let n = if self.flag(0x11) { 5 } else { 4 };
            let speed = self.me.move_speed(self.data, self.env, n);
            let rate = self.me.turn_rate(self.data, 0);
            self.me.turn_yaw(self.opp, 0, rate);
            self.me
                .turn_pitch(self.data, self.opp, self.opp_data, 2, deg(9.0));
            self.me.turn_model_yaw(deg(36.0), 0.3);
            self.me.set_direction(3);
            self.me.advance(speed, 10000.0);
            self.me.apply_gravity();
            if self.flag(5) && !self.me.input.is_held(DIR_MASK) {
                self.set(0x1A);
            }
            let c = opp_yaw_from_facing(self.me, self.opp).cos();
            if 0.0 < c {
                let pitch = self.me.pose.pitch;
                self.me.set_lean_x(pitch * c);
            }
            self.set(0xCA);
            self.me.request_orbit(50.0, 100.0);
            self.set(0x50);
            self.set(0x51);
            self.me.set_fx(0xB);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE {
            if 0.4 < self.me.anim_progress() && self.test(0x2C) {
                self.set_queue(0, 0x3E);
            }
            if self.me.anim_passed_ratio(0.7) {
                let q = self.queued();
                self.request(q);
            }
            if self.test(0x40) {
                self.request(0x58);
            }
            if self.decide_attack(0xC000) != 0 {
                let q = self.queued();
                self.request(q);
            }
            if self.decide_common(6) {
                let q = self.queued();
                self.request(q);
            }
        }
    }

    /// Action 0x3F (`BtlAct_Action3F`): the ground version, motion 0xFC, then the jump 0x11; input 0x40 the lift
    /// strike; landing ends it. The voice is not ported.
    fn ground_step(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xFC, 0.15);
            self.clear(0xE);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0x11);
            }
            self.step(2, 5, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            self.set(0x50);
            self.set(0x51);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE {
            if 0.4 < self.me.anim_progress() && self.test(0x2C) {
                self.set_queue(0, 0x3F);
            }
            if self.me.anim_passed_ratio(0.7) {
                let q = self.queued();
                self.request(q);
            }
            if self.test(0x40) {
                self.request(0x59);
            }
            if self.decide_attack(0x6_0000) != 0 {
                let q = self.queued();
                self.request(q);
            }
            if self.flag(0xF) {
                self.request(0xB);
            }
        }
    }

    /// Action 0x41 (`BtlAct_Action41`, forced by flag 0x7D): motion 0x193 (the counter of 0x193..0x195 is reset on
    /// every entry, `BtlAct_GetPrev` never being 0x41), then the idle until flag 0xB5 or the opponent leaves the
    /// auto rush 0x45: the lunge 0x6A. The sound and the camera cut are not ported.
    fn rush_guard(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x193, 0.15);
        }
        if phase == RUN {
            let idle = if self.flag(0xE) { 0x26 } else { 0 };
            match self.anim_id() {
                0x193..=0x195 => {
                    self.me.anim_advance_then(idle, 0.15, 0);
                }
                0 | 0x26 => self.me.anim_advance_loop(0),
                _ => {}
            }
            self.step(2, 5, 3, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            self.set(0x4E);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == DECIDE
            && matches!(self.anim_id(), 0 | 0x26)
            && (self.flag(0xB5) || self.opp_seen_action() != 0x45)
        {
            self.request(0x6A);
        }
    }

    /// Action 0x42 (`BtlAct_Action42`): the burst out of a combo, motion 0x192 over 25 frames for 5000 health,
    /// untouchable (held flag 0x4B) for its first half. The sound, the voice and the vibration are not ported.
    fn health_burst(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x192, 0.15);
            self.held(0xE);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            self.me.damage(self.env, self.data, 5000, 0x42B);
            self.held(0x4B);
            self.me.set_fx(0x2B);
        }
        if phase == RUN {
            self.me.anim_set_duration(25.0 / 30.0);
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            if self.me.anim_passed_ratio(0.5) {
                self.clear(0x4B);
            }
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xC9);
            self.set(0xD5);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
        if phase == LEAVE {
            self.clear(0x4B);
        }
    }

    /// Action 0x43 (`BtlAct_Action43`): the idle talk, motion 0x185, counted on the member. The voice is not
    /// ported.
    fn idle_talk(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x185, 0.15);
            self.me.idle_talk_count += 1;
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            self.step(6, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
    }

    /// Action 0x45 (`BtlAct_RushAutoHandler`, forced by flag 0x7E): the whole rush chain at triple speed from
    /// 0x37 (`BtlAct_GetPrev` is never 0x45); after the last hit window of 0x40, flag 0x90. The camera cut is not
    /// ported.
    fn rush_auto(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x37, 0.1);
        }
        if phase == RUN {
            let id = self.anim_id();
            match id {
                0x37..=0x39 | 0x3C..=0x3F => {
                    self.me.anim_set_step(3.0);
                    self.me.anim_advance_then(id + 1, 0.0, 0);
                }
                0x3A => {
                    self.me.anim_set_step(3.0);
                    self.me.anim_advance_then(0x3C, 0.0, 0);
                }
                0x40 => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    let h = self.me.obj.hit;
                    if !self.me.anim_is_new() && h.index == !h.count {
                        self.set(0x90);
                    }
                }
                _ => {}
            }
            self.step(2, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            self.set(0x88);
            if self.flag(0x11) {
                self.held(0xE);
            }
        }
    }

    /// Action 0x46 (`BtlAct_RushRapidHandler`): the rapid rush, motion 0x3C at triple speed (its step counter is
    /// reset on every entry, `BtlAct_GetPrev` never being 0x46), repeated by input 0x34; on a hit attack record
    /// 0x9D (0xAC powered up). The sound, the voice and the camera cut are not ported.
    fn rush_rapid(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x3C, 0.1);
            begin_rise_to_opponent(self.me, self.opp, self.data, self.opp_data);
            self.held(0xE);
            self.me.pose.speed = crate::btl_hit::launch_speed(self.me, self.data, self.env);
        }
        if phase == RUN {
            let next = if self.flag(0xE) { 0x26 } else { 0 };
            match self.anim_id() {
                0x3C..=0x3F => {
                    self.me.anim_set_step(3.0);
                    self.me.obj.rate = 2.0;
                    self.me.anim_advance_then(next, 0.3, 0);
                    self.me.set_fx(9);
                }
                0x40 => {
                    if self.me.obj.frame < self.query_event(1, 0, 0) as f32 {
                        self.me.anim_set_step(3.0);
                        self.me.set_fx(9);
                    }
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    if self.flag(0x5B) {
                        self.me.pose.speed = 0.0;
                    }
                }
                0 | 0x26 => {
                    self.me.anim_advance(1);
                    if !self.flag(0x34) {
                        self.request(0xB);
                    }
                }
                _ => {}
            }
            self.step(2, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
        }
        if phase == DECIDE {
            let (can, go) = match self.anim_id() {
                0x3C..=0x3F => (true, self.flag(anim_flag::END)),
                0 | 0x26 => (true, true),
                _ => (false, false),
            };
            if can {
                if self.flag(6) && self.test_attack_skill(0x3C) && !self.me.gauge.body_changed {
                    self.queue_attack(0xAC);
                } else {
                    self.queue_attack(0x9D);
                }
                if self.test(0x34) {
                    self.set_queue(0, 0x46);
                }
                if go && self.flag(0x5B) {
                    let q = self.queued();
                    self.request(q);
                }
            }
        }
    }

    // ---- the throws (btl_act_change.c, btl_act_super.c) --------------------------------------------------------

    /// `BtlActThrow_SetupDamage`: the queue of the current attack record's damage, spread over the motion's damage
    /// events (+ 1 with `extra`) as total / (hits + 5), rounded up to a multiple of 10 while that stays within the
    /// total; unless the opponent's parameter word has bit 0x80, the record's health / ki drains between its
    /// 0x20000 and 0x40000 events.
    fn setup_throw_damage(&mut self, extra: bool) {
        let mut hits = self.query_event(0x2_0000_0000, 0, 3);
        if extra {
            hits += 1;
        }
        let damage = crate::btl_hit::damage(self.me, self.data, self.env);
        let (drain_health, drain_ki) = crate::btl_hit::throw_drains(self.me, self.data, self.env);
        let immune = self.opp_data.tables.params.chara_flags() & 0x80 != 0;
        let (from, to) = (
            self.query_event(0x2_0000, 0, 0),
            self.query_event(0x4_0000, 0, 2),
        );
        let q = &mut self.me.queue;
        *q = crate::btl_chr::DamageQueue {
            hits,
            total_start: damage,
            total: damage,
            per_hit: damage / (hits + 5),
            flags: if damage <= 0 {
                0x1000_0200
            } else {
                0x1000_0000
            },
            drain_from: -1,
            drain_to: -1,
            ..Default::default()
        };
        let rem = q.per_hit % 10;
        if rem != 0 {
            let up = q.per_hit - rem + 10;
            if up * q.hits < q.total_start {
                q.per_hit = up;
            }
        }
        if !immune && (drain_health > 0 || drain_ki > 0) {
            let span = ((to - from) / 2).max(1);
            q.drain_health_start = drain_health;
            q.drain_health = drain_health;
            q.drain_health_step = drain_health / span + 1;
            q.drain_ki_start = drain_ki;
            q.drain_ki = drain_ki;
            q.drain_ki_step = drain_ki / span + 1;
            q.drain_from = from;
            q.drain_to = to;
        }
    }

    /// `BtlActThrow_TurnByRootYaw`: the heading less the yaw of the first key of the motion's root rotation.
    fn turn_by_root_yaw(&mut self) {
        if let Some(yaw) = self.me.obj.root_key_yaw(&self.data.obj) {
            let h = wrap_angle(self.me.pose.yaw - yaw);
            self.me.set_heading(h, 0.0);
        }
    }

    /// `BtlActThrow_SetReleaseHeading`: the thrown fighter leaves along the hit's launch angles (mirrored when
    /// grabbed from behind).
    fn set_release_heading(&mut self) {
        let r = self.me.react;
        let p = &self.me.pose;
        let (yaw, pitch) = if r.back {
            (wrap_angle(wrap_angle(p.yaw + r.launch_a) + PI), r.launch_b)
        } else {
            (wrap_angle(p.yaw + r.launch_a), -r.launch_b)
        };
        let pitch = pitch * p.root_rot[1].cos();
        self.me.set_heading(yaw, pitch);
    }

    /// The blend a throw's motions start with: none for a throw of parameter flag bit 19.
    fn throw_blend(&self) -> f32 {
        if self.me.react.no_blend { 0.0 } else { 0.3 }
    }

    /// Actions 0xB4..0xB6 (`BtlAct_GrabDash`): the grab, motions 0x94 -> 0x95 (0xB6: 0x186 -> 0x187 at double
    /// rate); missed (flag 0x7A) 0x9D, caught (0x5B) the throw 0xB7 (0xBB for parameter flag 0x200), 0xB6 0xB9.
    fn grab(&mut self, phase: u32) {
        if phase == ENTER {
            let anim = if self.current() == 0xB6 { 0x186 } else { 0x94 };
            self.play(anim, 0.15);
            begin_rise_to_opponent(self.me, self.opp, self.data, self.opp_data);
        }
        if phase == RUN {
            let yaw = if self.current() == 0xB5 { 2 } else { 6 };
            match self.anim_id() {
                0x94 => {
                    self.me.anim_advance_then(0x95, 0.0, 0);
                }
                0x186 => {
                    self.me.anim_advance_then(0x187, 0.0, 0);
                }
                0x95 | 0x9D | 0x187 => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                }
                _ => {}
            }
            self.step(yaw, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.set(0xD5);
            self.set(0x92);
            if self.current() == 0xB6 {
                self.me.obj.rate = 2.0;
            }
        }
        if phase == DECIDE {
            if self.flag(0x7A) {
                self.request_anim(0x9D, 0.0);
            }
            if self.flag(0x5B) {
                let next = match self.current() {
                    0xB6 => 0xB9,
                    _ if self.data.tables.params.flags() & 0x200 != 0 => 0xBB,
                    _ => 0xB7,
                };
                self.request(next);
            }
        }
    }

    /// Actions 0xB7, 0xB9 (`BtlAct_Throw`): the thrower, motions 0x96 -> 0x98 (0xB9: 0x188 -> 0x18A) holding the
    /// victim (flag 0x8B); near the stage's edge facing out it turns round. The drain runs between the queue's
    /// frames. The voice, the sounds and the camera cut are not ported.
    fn throw(&mut self, phase: u32) {
        let alt = self.current() == 0xB9;
        if phase == ENTER {
            let blend = self.throw_blend();
            self.play(if alt { 0x188 } else { 0x96 }, blend);
            self.held(0xE);
            self.setup_throw_damage(false);
            if crate::btl_hit::launch_angle_a(self.me, self.data, self.env).abs() < deg(90.0) {
                let radius = self.env.inner_radius();
                let p = self.me.pose.pos;
                if radius - 200.0 < p[0].hypot(p[2]) {
                    self.me.turn_yaw(self.opp, if alt { 8 } else { 7 }, PI);
                }
            }
            if self.data.chara == 0x6E && self.opp_data.tables.params.chara_flags() & 0x80 == 0 {
                self.me.boost_stock = (self.me.boost_stock + 1).clamp(0, 10);
            }
        }
        if phase == RUN {
            match self.anim_id() {
                0x96 => {
                    self.me.anim_advance_then(0x98, 0.0, 1);
                    self.set(0x8B);
                }
                0x188 => {
                    self.me.anim_advance_then(0x18A, 0.0, 1);
                    self.set(0x8B);
                }
                0x98 | 0x18A => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                }
                _ => {}
            }
            self.step(6, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            for f in [0xBB, 0x1A, 0x3F, 0x44, 0x45, 0x46] {
                self.set(f);
            }
            let aflags = crate::btl_hit::attack_flags(self.me, self.data, self.env);
            if aflags & 0x400 != 0 {
                self.set(0x13B);
            }
            let q = self.me.queue;
            if q.drain_from >= 0 && q.drain_to >= 0 {
                if self
                    .me
                    .anim_in_frame_range(q.drain_from as f32, q.drain_to as f32)
                {
                    self.me.set_fx(0x38);
                }
                if aflags & 0x2000_0000 != 0 {
                    self.set(0x13C);
                }
            }
        }
        if phase == LEAVE {
            self.set(0x8C);
            self.set(0x33);
        }
    }

    /// Actions 0xB8, 0xBA (`BtlAct_Thrown`): the victim, motion 0x97 (0xBA: 0x189, both the thrower's) held by the
    /// thrower; at its end the hit's second reaction. Dropped by the hold (flag 0x23), it leaves along the launch
    /// angles. The voice is not ported.
    fn thrown(&mut self, phase: u32) {
        if phase == ENTER {
            let blend = self.throw_blend();
            self.play(if self.current() == 0xBA { 0x189 } else { 0x97 }, blend);
            self.held(0xE);
            self.turn_by_root_yaw();
        }
        if phase == RUN {
            if self.me.anim_advance(1) {
                if self.me.react.back {
                    self.set(0x35);
                }
                let sub = self.me.react.reaction_sub;
                self.queue_reaction(sub);
                let q = self.queued();
                self.request(q);
            }
            self.step(9, 6, 3, kmh(1000.0), kmh(100.0));
            self.me.apply_gravity();
            for f in [0x97, 0x95, 0xBB, 0xA, 0x1A, 0x3F, 0x44, 0x45, 0x46] {
                self.set(f);
            }
            if !self.me.react.silent {
                self.set(0x96);
            }
            self.set(0x1B);
            self.me.set_fx(0x3B);
            if self.me.flags.dropped(0x23) {
                self.set_release_heading();
                self.me.work[0] |= 1;
            }
            if self.me.work[0] & 1 != 0 {
                let pitch = self.me.pose.pitch;
                self.me.set_lean_x(-pitch);
            }
        }
        if phase == LEAVE {
            self.set(0x33);
            if self.me.work[0] & 1 == 0 {
                self.set_release_heading();
            }
        }
    }

    /// Action 0xBB (`BtlAct_SlamThrow`): the thrower dives with the victim, motion 0x96, the loop 0x99 at 1500 km/h
    /// down, then 0x98 on the ground, at the stage's bottom, when the victim is gone, after 91 frames, or (parameter
    /// flag 0x400) close above the water. The voices, the sound, the camera cuts and the vibration are not ported.
    fn slam_throw(&mut self, phase: u32) {
        if phase == ENTER {
            let blend = self.throw_blend();
            self.play(0x96, blend);
            self.setup_throw_damage(true);
            self.held(0xE);
        }
        if phase == RUN {
            let mut rise = false;
            match self.anim_id() {
                0x96 => {
                    self.me.anim_advance_then(0x99, 0.0, 1);
                    if self.me.anim_progress() > 0.9 {
                        rise = true;
                    }
                    self.set(0x8B);
                }
                0x99 => {
                    self.me.anim_advance_loop(0);
                    self.set(0x8B);
                    self.me.set_fx(4);
                    rise = true;
                }
                0x98 => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                }
                _ => {}
            }
            self.step(6, 5, 2, 0.0, kmh(100.0));
            if rise {
                self.me.move_vertical(kmh(1500.0), kmh(100.0));
            } else {
                self.me.brake_vertical();
            }
            for f in [0xBB, 0x1A, 0x3F, 0x44, 0x45, 0x46] {
                self.set(f);
            }
        }
        if phase == DECIDE && self.anim_id() == 0x99 {
            let mut end = self.flag(0xF) || self.flag(0x16);
            if self.opp_seen_action() != 0xBC {
                end = true;
            }
            if self.env.stage_4_or_27 && self.flag(0x17) {
                end = true;
            }
            self.me.work[2] += 1;
            if self.me.work[2] > 10 {
                if self.me.work[2] > 90 {
                    end = true;
                }
                if self.data.tables.params.flags() & 0x400 != 0
                    && let Some(top) = self.env.stage.water_level
                    && top - self.me.pose.pos[1] < self.data.height() * 3.0
                {
                    end = true;
                }
            }
            if end {
                self.request_anim(0x98, 0.0);
                self.me.pose.fall_speed = 0.0;
            }
        }
    }

    /// Action 0xBC (`BtlAct_SlamThrown`): its victim, motion 0x97, the loop 0x9A diving with the thrower, 0x9B on
    /// the ground once let go, then down (0xDE); out of the stage or without the thrower 0xE6. The camera shake and
    /// the rumble are not ported.
    fn slam_thrown(&mut self, phase: u32) {
        if phase == ENTER {
            let blend = self.throw_blend();
            self.play(0x97, blend);
            self.held(0xE);
            self.turn_by_root_yaw();
        }
        if phase == RUN {
            let mut rise = false;
            match self.anim_id() {
                0x97 => {
                    self.me.anim_advance_then(0x9A, 0.0, 1);
                    self.set(0x97);
                }
                0x9A => {
                    self.me.anim_advance_loop(0);
                    self.set(0x97);
                    self.set(0xA);
                    rise = true;
                }
                0x9B => {
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                }
                _ => {}
            }
            self.step(9, 6, 3, 0.0, kmh(100.0));
            if rise {
                self.me.move_vertical(kmh(1500.0), 10000.0);
            } else {
                self.me.apply_gravity();
            }
            for f in [0x95, 0xBB, 0x1A, 0x3F, 0x44, 0x45, 0x46, 0x96, 0x1B] {
                self.set(f);
            }
            self.me.set_fx(0x3B);
        }
        if phase == DECIDE {
            if self.anim_id() == 0x9A {
                if self.flag(0x16)
                    || (self.env.stage_4_or_27 && self.flag(0x17))
                    || self.opp_seen_action() != 0xBB
                {
                    self.request(0xE6);
                }
                if !self.flag(0x23) && self.flag(0xF) {
                    self.request_anim(0x9B, 0.0);
                    self.opp.flags.set(0x8C);
                }
            }
            if self.me.work[1] != 0 {
                if self.me.react.back {
                    self.set(0x35);
                }
                self.request(0xDE);
            }
        }
        if phase == LEAVE {
            self.set(0x33);
        }
    }

    /// Action 0xBE (`BtlAct_DragDown`, forced by flag 0x7F): drags the opponent down head first at 2000 km/h
    /// (motion 0x197), then 0x198 and the hop 0x28 once it is let go. The camera cut is not ported.
    fn drag_down(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x197, 0.0);
        }
        if phase == RUN {
            let (mut lean, mut speed) = (0.0, 0.0);
            match self.anim_id() {
                0x197 => {
                    self.me.anim_advance_loop(0);
                    lean = 1.0;
                    self.me.move_vertical(kmh(2000.0), 10000.0);
                }
                0x198 => {
                    self.me.anim_advance_then(0x28, 0.15, 0);
                    lean = 1.0 - self.me.anim_progress();
                    self.me.move_vertical(0.0, 10000.0);
                }
                0x28 => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                    if self.me.anim_progress() < 0.6 {
                        speed = self.me.move_speed(self.data, self.env, 0xB);
                    }
                    self.me.move_vertical(0.0, 10000.0);
                }
                _ => {}
            }
            self.step(6, 5, 4, speed, kmh(100.0));
            for f in [0xBB, 0x1A, 0x3F, 0x44, 0x45, 0x46] {
                self.set(f);
            }
            self.me.set_lean_x(lean * deg(-90.0));
        }
        if phase == DECIDE
            && self.anim_id() == 0x197
            && self.me.action_frame > 0
            && self.opp_seen_action() != 0xBF
        {
            self.request_anim(0x198, 0.0);
        }
    }

    /// Action 0xBF (`BtlAct_DraggedDown`, reaction 0x31): dragged down under the opponent in 0xBE (its position
    /// plus its movement, 0.3 of its height lower, by this model's node 0 less node 0x10), motion 0xC3 / 0xC4
    /// (`BtlAct_GetPrev` is never 0xD5); on the ground 500 damage and 0xDE, out of the stage 0xE6 (0xD9 dead). The
    /// camera shake and the rumble are not ported.
    fn dragged_down(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, false);
            self.play(if air { 0xC4 } else { 0xC3 }, 0.0);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
        }
        if phase == RUN {
            let air = self.me.is_air_motion(self.env, false);
            self.me.anim_advance_loop(0);
            self.step(6, 6, 7, 0.0, kmh(100.0));
            self.me.move_vertical(kmh(2000.0), 10000.0);
            if self.opp_seen_action() == 0xBE || self.me.action_frame == 0 {
                let mut pos = add(self.opp.snap_pos(6), self.opp.pose.vel);
                pos[1] += self.opp_data.height() * 0.3;
                let offset = sub(self.me.obj.node_pos(0), self.me.obj.node_pos(0x10));
                self.me.pose.pos = add(pos, offset);
            }
            for f in [0xBB, 0xA, 0x1A, 0x3F, 0x44, 0x45, 0x46, 0x95, 0x96, 0x1B] {
                self.set(f);
            }
            self.me.set_fx(0x3B);
            self.me.set_lean_x(if air { deg(-90.0) } else { deg(90.0) });
        }
        if phase == DECIDE {
            if self.flag(0x1F) || self.flag(0xF) {
                self.request(0xDE);
                self.me.damage(self.env, self.data, 500, 0);
            }
            let out = |s: &Self| if s.me.is_dead() { 0xD9 } else { 0xE6 };
            if self.flag(0x16) {
                self.request(0xE6);
            }
            if self.flag(0xB6) {
                let a = out(self);
                self.request(a);
            }
            if self.env.stage_4_or_27 {
                if self.flag(0x17) {
                    self.request(0xE6);
                }
                if self.flag(0xB7) {
                    let a = out(self);
                    self.request(a);
                }
            }
        }
    }

    // ---- the clashes (btl_act_2.c) -----------------------------------------------------------------------------

    /// Action 0xFA (`BtlAct_ClashBlowsHandler`, forced by flag 0x61): clash B, trading blows (motion 0x173 / 0x174
    /// by player) while spinning round the opponent; every direction input counts, and one in four frames counts by
    /// itself (abilities 0x12..0x14 and 0x68..0x6A are not ported; the technique bonus needs `BtlAct_GetPrev`, which
    /// is never a technique). Won (flag 0xBF) motion 0x175 then 0xB, lost (0xC0) 0x176 with the damage of the
    /// winner's count, then blown away. The vibration is not ported.
    fn clash_blows(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(if self.me.player == 0 { 0x173 } else { 0x174 }, 0.15);
            self.me.clash_count_b = 0;
            self.me.flags.clear_range(0xBF, 0xC3);
        }
        if phase == RUN {
            match self.anim_id() {
                0x173 | 0x174 => {
                    self.me.anim_advance_loop(0);
                    self.set(0xE6);
                    self.set(0xED);
                    if self.test(0x33) || self.me.action_frame % 4 == 0 {
                        count_and_mark(self.me, self.opp);
                    }
                    self.me.combo.timer = 30;
                    if self.flag(0xBF) {
                        self.request_anim(0x175, 0.15);
                        count_and_mark(self.me, self.opp);
                        self.me.combo.timer = 0;
                    }
                    if self.flag(0xC0) {
                        self.request_anim(0x176, 0.15);
                    }
                }
                0x175 => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                }
                0x176 => {
                    if self.me.anim_advance(0) {
                        self.request(0xD5);
                    }
                    if self.me.anim_passed_ratio(0.5) {
                        let dmg = self.opp.clash_count_b * 60;
                        self.me.damage(self.env, self.data, dmg, 8);
                    }
                    self.set(0x95);
                    self.set(0x96);
                }
                _ => {}
            }
            steer_at_opponent(
                self.me,
                self.opp,
                self.data,
                self.opp_data,
                kmh(100.0),
                0.0,
                0.0,
                0.0,
                0.0,
                PI / 4.0,
            );
            self.me.turn_model_yaw(deg(36.0), 0.3);
            self.me.pose.yaw = wrap_angle(self.me.pose.yaw + 0.5);
            self.me.set_direction(3);
            self.me.advance(kmh(100.0), 10000.0);
            self.me.apply_gravity();
            for f in [
                0xBB, 0x128, 0xD2, 0x134, 0xEE, 0x13B, 0x42, 0x43, 0x44, 0x45, 0x46,
            ] {
                self.set(f);
            }
        }
        if phase == LEAVE {
            self.me.clash_count_b = 0;
            self.me.flags.clear_range(0xBF, 0xC3);
        }
    }

    /// Action 0xFB (`BtlAct_ClashVanishHandler`, forced by flag 0x62): the start of clash C, vanishing (motion
    /// 0xF6), then the exchange 0xFC seven frames after the motion ends. The sound is not ported.
    fn clash_vanish(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0xF6, 0.1);
            self.me.clash_count_b = 0;
            self.me.set_fx(0xC);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.set(0xB);
                self.me.work[2] += 1;
                if self.me.work[2] >= 7 {
                    self.request(0xFC);
                }
            }
            self.step(2, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            for f in [0xBB, 0x128, 0x42, 0x43, 0x44, 0x45, 0x46] {
                self.set(f);
            }
        }
    }

    /// Action 0xFC (`BtlAct_ClashExchangeHandler`): one exchange of clash C, placed on the clash's stage path
    /// (flag 0xF8). After a wait by the clash level the fighter strikes at 2000 km/h (motion 0x4C: the exchange
    /// counter that would turn it to 0x3B / 0x4F / 0x40 is reset on every entry, `BtlAct_GetPrev` never being
    /// 0xFC); the frame the asked button is pressed is the answer (-2 for a wrong one). The sequence answers with
    /// 0xC5 (again), 0xC6 (over), 0xC7 (won: 0x175) and 0xC8 (lost: 0x176, damage by the winner's count). The
    /// sounds and the vibration are not ported.
    fn clash_exchange(&mut self, phase: u32) {
        let c = *self.clash;
        if phase == ENTER {
            self.play(0x4C, 0.15);
            // BtlAnim_EnableHandle: the motion's root translation zeroed.
            self.me.obj.zero_root(&self.data.obj);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            self.held(0xE);
            self.set(0xF8);
            self.me.clash_answer_frame = -1;
        }
        if phase == RUN {
            let (duration, wait) = match c.level {
                0 => (0.5, 18),
                2 => (0.3, 12),
                _ => (0.4, 15),
            };
            let mut speed = 0.0;
            match self.anim_id() {
                0x3B | 0x40 | 0x4C | 0x4F => {
                    if self.me.work[2] < wait {
                        self.me.work[2] += 1;
                        self.set(0xB);
                    } else {
                        self.me.anim_set_duration(duration);
                        self.me.anim_advance(0);
                        if self.me.anim_progress() < 0.1 {
                            self.set(0xB);
                        } else {
                            speed = kmh(2000.0);
                        }
                        if self.me.anim_passed_ratio(0.1) {
                            self.me.set_fx(0xC);
                            if self.me.player == 0 {
                                self.me.set_fx(0x2D);
                            }
                        }
                        if self.me.anim_progress() > 0.7 {
                            self.set(0xB);
                            self.set(0xC4);
                        }
                    }
                    if self.me.action_frame > 0 {
                        self.set((c.pick + 0xE9) as u32);
                        if self.me.clash_answer_frame == -1 && self.me.input.is_pressed(0x7800_0000)
                        {
                            if self.me.input.is_pressed(0x800_0000 << c.pick) {
                                self.me.clash_answer_frame = self.me.action_frame;
                            } else if c.prev_pick < 0
                                || !self.me.input.is_pressed(0x800_0000 << c.prev_pick)
                            {
                                self.me.clash_answer_frame = -2;
                            }
                        }
                    }
                    self.set(0x18);
                    self.me.combo.timer = 30;
                }
                0x175 => {
                    if self.me.anim_advance(0) {
                        self.request(0xB);
                    }
                }
                0x176 => {
                    if self.me.anim_advance(0) {
                        self.request(0xD5);
                    }
                    if self.me.anim_passed_ratio(0.5) {
                        let dmg = self.opp.clash_count_b * 600;
                        self.me.damage(self.env, self.data, dmg, 8);
                    }
                    self.set(0x95);
                    self.set(0x96);
                }
                _ => {}
            }
            self.step(2, 5, 2, speed, 10000.0);
            self.me.apply_gravity();
            for f in [0xBB, 0x128, 0x134, 0xEE, 0x42, 0x43, 0x44, 0x45, 0x46, 0x91] {
                self.set(f);
            }
        }
        if phase == DECIDE {
            if self.flag(0xC5) {
                self.request(0xFC);
            }
            if self.flag(0xC6) {
                self.request(0xB);
            }
            if self.flag(0xC7) {
                self.request_anim(0x175, 0.0);
                self.me.combo.timer = 0;
            }
            if self.flag(0xC8) {
                self.request_anim(0x176, 0.0);
            }
        }
        if phase == LEAVE && self.requested() != 0xFC {
            self.me.clash_count_b = 0;
        }
    }

    // ---- the techniques: Blast 2 and Ultimate Blast (btl_act_super.c, btl_act_decide.c) -------------------------

    /// The technique table.
    fn tech(&self) -> bt3_data::chara_data::TechTable<'_> {
        self.data.tables.tech()
    }

    /// `BtlAct_GetCurrentClass` (`BtlAct_GetIdClass`): 0 / 1 for the skills 0xFD..0x102, 2..4 for the technique
    /// actions 0x106..0x13B (their place in the triple), 4 for 0x105, else -1.
    fn current_class(&self) -> i32 {
        id_class(self.current())
    }

    /// `BtlAct_CanUseTechnique`: lock-on, the powered-up mode for slot 4, the ki; a technique with flag 0x2000 is
    /// refused while its lock counter runs (except in modes 5 and 6).
    fn can_use_technique(&self, slot: i32) -> bool {
        if !(2..5).contains(&slot) || self.me.gauge.body_changed || !self.flag(5) {
            return false;
        }
        if slot == 4 && !self.flag(6) {
            return false;
        }
        // BtlSuper_GetKiCost: ability 0x2B is not ported.
        if !self.me.has_ki(self.tech().ki(slot)) {
            return false;
        }
        if self.tech().flags(slot) & 0x2000 != 0 {
            if matches!(self.env.mode, 5 | 6) {
                return true;
            }
            let check = self.env.mode != 1 || !self.flag(0x11F);
            if check && self.me.gauge.fired[slot as usize] > 0 {
                return false;
            }
        }
        true
    }

    /// `BtlAct_GetTechniqueAction`: the technique's kind picks a block of three actions.
    fn technique_action(&self, slot: i32) -> i32 {
        let n = slot - 2;
        match self.tech().kind(slot) {
            0 | 7 => n + 0x106,
            1 => n + 0x10C,
            2 => n + 0x10F,
            3 => n + 0x112,
            6 => n + 0x11E,
            4 | 5 => n + 0x11B,
            9 => n + 0x109,
            8 => n + 0x124,
            10 => n + 0x121,
            _ => slot + 0x104,
        }
    }

    /// `BtlAct_CheckTechniqueInput`: the charge button with blast (inputs 0x6C / 0x6D / 0x6E: neutral, up, down)
    /// picks slot 2 / 3 / 4; when it can be used by a free fighter: a step back 0x1B first when close (unless the
    /// technique has flag 0x200 or is of kind 10), 0x105 before slot 4, then the technique's action.
    fn check_technique_input(&mut self) -> u32 {
        if self.me.tech_delay > 0 {
            return 0;
        }
        let mut slot = -1;
        for (input, s) in [(0x6C, 2), (0x6D, 3), (0x6E, 4)] {
            if self.test(input) {
                slot = s;
            }
        }
        if slot < 0 || !self.can_use_technique(slot) || !self.me.is_free() {
            return 0;
        }
        let mut n = 0;
        if self.flag(0x13) && self.tech().flags(slot) & 0x200 == 0 && self.tech().kind(slot) != 10 {
            self.set_queue(0, 0x1B);
            n = 1;
        }
        if slot == 4 {
            self.set_queue(n, 0x105);
            n += 1;
        }
        let a = self.technique_action(slot);
        self.set_queue(n, a);
        n as u32 + 1
    }

    /// `BtlSuper_Begin`: speed and fall stopped, the technique flags cleared, the use counted, in flight; class 4
    /// only when forced. The colour mode of flag 0x8000 is not ported.
    fn super_begin(&mut self, cls: i32, force: bool) {
        if !force && cls == 4 {
            return;
        }
        self.me.pose.speed = 0.0;
        self.me.pose.fall_speed = 0.0;
        self.me.flags.clear_range(0x9B, 0xA0);
        self.me.flags.clear_range(0xA7, 0xAA);
        self.set(0x29);
        self.me.charge = 0.0;
        self.me.charge_full_frames = 0;
        if let Some(u) = self.me.gauge.used.get_mut(cls as usize) {
            *u += 1;
        }
        if !self.flag(0xF) {
            self.held(0xE);
        }
    }

    /// `BtlSuper_SetStartFx`: effect bit 0x3E / 0x3F / 0x40 by class.
    fn super_start_fx(&mut self, cls: i32) {
        let bit = match cls {
            3 => 0x3F,
            4 => 0x40,
            _ => 0x3E,
        };
        self.me.set_fx(bit);
    }

    /// `BtlSuper_SetClassFlag`: flag 0xE0 / 0xE1 / 0xE2 by class; class 4 only when forced.
    fn super_class_flag(&mut self, cls: i32, force: bool) {
        if !force && cls == 4 {
            return;
        }
        let flag = match cls {
            3 => 0xE1,
            4 => 0xE2,
            _ => 0xE0,
        };
        self.set(flag);
    }

    /// `BtlSuper_RequestHitStop`: level 2 (0x125) with the full flag set, or level 1 (0x126).
    fn super_hit_stop(&mut self, light: bool) {
        let flags: &[u32] = if light {
            &[0x44, 0x45, 0x126]
        } else {
            &[0x42, 0x43, 0x44, 0x45, 0x46, 0x125, 0x12A]
        };
        for &f in flags {
            self.set(f);
        }
        self.set(0x128);
    }

    /// `BtlSuper_SetFiringFlags`.
    fn super_firing_flags(&mut self) {
        self.set(0xD5);
        self.set(0xBB);
        self.set(0x127);
        if !self.flag(0xE) && !self.flag(0xF) {
            self.held(0xE);
        }
    }

    /// `BtlSuper_SetClass`: the technique's button command is read next frame.
    fn super_set_class(&mut self, cls: i32) {
        self.me.input.tech_class = cls;
    }

    /// `BtlSuper_FitAnimToEvent`: the motion stretched so that its 0x400 event comes `seconds` from now (0.1 s
    /// for a motion without one).
    fn super_fit_to_event(&mut self, seconds: f32) {
        let event = self.query_event(0x400, 0, 0) as f32;
        if event < 0.0 {
            self.me.anim_set_duration(0.1);
            return;
        }
        let frame = self.me.obj.frame;
        if event - 2.0 < frame {
            return;
        }
        let step = if seconds < 0.033_333_335 {
            event - frame
        } else {
            (event / (seconds * 30.0)).min(event - frame)
        };
        self.me.anim_set_step(step);
    }

    /// `BtlSuper_Leave`: unless another technique action follows, the cooldown starts and the effect requests and
    /// the input lock are dropped.
    fn super_leave(&mut self, cls: i32) {
        if is_technique_id(self.requested()) {
            return;
        }
        self.me.tech_delay = self.tech().cooldown_frames(cls);
        self.me.flags.clear_range(0xA7, 0xAA);
        self.clear(0x11F);
        match self.tech().id(cls) {
            0x268 => self.me.skill_count3 = 0,
            0x2CD => self.me.boost_stock = 0,
            _ => {}
        }
    }

    /// `BtlSuper_Finish`: back to 0xB, or falling (0xD9) for a technique with flag 0x4000.
    fn super_finish(&mut self, cls: i32) {
        if self.tech().flags(cls) & 0x4000 != 0 {
            self.held(0x1D);
            self.held(0x1E);
            self.set(0x35);
            self.request(0xD9);
        } else {
            self.request(0xB);
        }
    }

    /// The common start of the beam techniques: the start motion, the begin, facing the opponent, the start flags
    /// and the ki. The camera cut, the sound and the head look are not ported.
    fn super_start(&mut self, cls: i32, blend: f32) {
        self.play(super_anim(cls, 0), blend);
        self.super_begin(cls, false);
        self.me.turn_yaw(self.opp, 2, PI);
        self.me
            .turn_pitch(self.data, self.opp, self.opp_data, 5, PI);
        self.me.turn_model_yaw(PI, 1.0);
        self.super_start_fx(cls);
        self.super_class_flag(cls, false);
        let ki = self.tech().ki(cls);
        self.me.spend_ki(ki, true);
    }

    /// The button command during a technique (input 0x6F): the held flags 0x9C..0x9E (more damage). The sounds
    /// are not ported.
    fn super_boost_input(&mut self) {
        if self.test(0x6F) {
            self.held(0x9E);
            self.held(0x9D);
            self.held(0x9C);
        }
    }

    /// The common end of a run phase: the pitch motions, the step (facing the opponent while aiming), the effect
    /// bit 0x10.
    fn super_move(&mut self, cls: i32, aim: bool) {
        if self.tech().flags(cls) & 0x1000 == 0 {
            let a = self.anim_id();
            self.set_pitch_motion_with(a + 7, a + 0xE, aim);
        }
        self.step(if aim { 2 } else { 6 }, 5, 2, 0.0, kmh(100.0));
        self.me.apply_gravity();
        self.me.set_fx(0x10);
    }

    /// A wait on an effect's flag, or `BTL_SUPER_TIMEOUT` frames: then motion `next`.
    fn super_wait_flag(&mut self, flag: u32, cls: i32, next: i32) {
        self.me.work[2] += 1;
        if self.me.work[2] > SUPER_TIMEOUT {
            self.held(flag);
        }
        if self.flag(flag) {
            self.request_anim(super_anim(cls, next), 0.0);
            self.me.work[2] = 0;
        }
    }

    /// Actions 0x106..0x108 (`BtlAct_SuperBeamHandler`) and the long beam 0x10C..0x10E
    /// (`BtlAct_SuperLongBeamHandler`): the start motion (level 2 hit-stop), the charge loop until the effect
    /// raises flag 0xA7, the fire, the firing loop until 0xA8 (the long beam: a second stage until 0xA9), the end.
    /// The effects that raise the flags are not ported yet: each wait ends after 150 frames, as the original's own
    /// timeout. Not ported either: the self-destruct change of flag 0x2000 (a character change), the shakes,
    /// vibration, sounds and cuts, and the beam struggles 0x12A.. (flags 0x72 / 0x73).
    fn super_beam(&mut self, phase: u32) {
        let cls = self.current_class();
        let long = (0x10C..=0x10E).contains(&self.current());
        if phase == ENTER {
            self.super_start(cls, 0.0);
        }
        if phase == RUN {
            let mut aim = self.tech().flags_a(cls) & 0x800 != 0;
            match self.anim_id() - super_anim(cls, 0) {
                0 => {
                    aim = true;
                    self.me.anim_advance_then(super_anim(cls, 1), 0.0, 0);
                    self.super_hit_stop(false);
                    self.super_set_class(cls);
                }
                1 => {
                    aim = true;
                    self.me.anim_advance_loop(0);
                    self.super_hit_stop(false);
                    self.super_set_class(cls);
                    self.super_wait_flag(0xA7, cls, 2);
                }
                2 => {
                    self.me.anim_advance_then(super_anim(cls, 3), 0.0, 0);
                    if long {
                        self.super_hit_stop(true);
                        self.super_set_class(cls);
                    } else {
                        self.super_set_class(cls);
                        if self.me.obj.frame < self.query_event(0x400, 0, 0) as f32 {
                            aim = true;
                            self.super_hit_stop(true);
                        }
                        if self.me.anim_is_new() && self.tech().flags(cls) & 0x2000 != 0 {
                            let h = self.me.gauge.health - 1;
                            self.me.damage(self.env, self.data, h, 0x5B);
                            if let Some(f) = self.me.gauge.fired.get_mut(cls as usize) {
                                *f += 1;
                            }
                        }
                    }
                }
                3 => {
                    self.me.anim_advance_loop(0);
                    if long {
                        self.super_hit_stop(true);
                        self.super_wait_flag(0xA8, cls, 4);
                    } else {
                        self.super_wait_flag(0xA8, cls, 6);
                    }
                }
                4 if long => {
                    self.me.anim_advance_then(super_anim(cls, 5), 0.0, 0);
                }
                5 if long => {
                    self.me.anim_advance_loop(0);
                    self.super_wait_flag(0xA9, cls, 6);
                }
                6 => {
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                }
                _ => {}
            }
            self.super_move(cls, aim);
            self.super_firing_flags();
            self.super_boost_input();
        }
        if phase == DECIDE {
            if !long {
                if self.flag(0x73) {
                    self.request(cls + 0x128);
                }
                if self.flag(0x72) {
                    self.request(cls + 0x12B);
                }
            }
            if self.me.work[1] != 0 {
                self.super_finish(cls);
            }
        }
        if phase == LEAVE {
            self.super_leave(cls);
        }
    }

    /// Actions 0x109..0x10B (`BtlAct_SuperWarpBeamHandler`): the beam fired after a teleport. Once the charge is
    /// done (flag 0xA7, or 150 frames) five stages: vanish, onto the opponent, 40 in front of it and 20 up facing
    /// it, reappear, fire.
    fn super_warp_beam(&mut self, phase: u32) {
        let cls = self.current_class();
        if phase == ENTER {
            self.super_start(cls, 0.0);
        }
        if phase == RUN {
            let mut aim = self.tech().flags_a(cls) & 0x800 != 0;
            let mut warped = false;
            match self.anim_id() - super_anim(cls, 0) {
                0 => {
                    aim = true;
                    self.me.anim_advance_then(super_anim(cls, 1), 0.0, 0);
                    self.super_hit_stop(false);
                    self.super_set_class(cls);
                }
                1 => {
                    self.me.anim_advance_loop(0);
                    aim = true;
                    match self.me.work[4] {
                        0 => {
                            self.super_hit_stop(false);
                            self.super_set_class(cls);
                            self.me.work[2] += 1;
                            if self.me.work[2] > SUPER_TIMEOUT {
                                self.held(0xA7);
                            }
                            if self.flag(0xA7) {
                                self.me.work[4] += 1;
                                self.me.work[3] = 0;
                                self.me.work[2] = 0;
                                self.me.set_fx(0xC);
                            }
                        }
                        1 => {
                            self.super_hit_stop(false);
                            self.me.work[3] += 1;
                            if self.me.work[3] > 3 {
                                self.set(0xB);
                            }
                            if self.me.work[3] >= 7 {
                                self.me.work[4] += 1;
                                self.me.work[3] = 0;
                            }
                        }
                        2 => {
                            self.super_hit_stop(false);
                            snap_to_opponent(self.me, self.opp);
                            self.set(0xB);
                            warped = true;
                            self.set(0xCC);
                            self.me.work[4] += 1;
                            self.me.work[3] = 0;
                        }
                        3 => {
                            self.super_hit_stop(false);
                            warp_in_front_of_opponent(
                                self.me,
                                self.data,
                                self.opp,
                                self.opp_data,
                                40.0,
                            );
                            warped = true;
                            self.me.pose.pos[1] += 20.0;
                            self.me.turn_yaw(self.opp, 2, PI);
                            self.me
                                .turn_pitch(self.data, self.opp, self.opp_data, 1, PI);
                            self.me.turn_model_yaw(PI, 1.0);
                            self.me.set_direction(3);
                            for f in [0x3F, 0xB] {
                                self.set(f);
                            }
                            self.me.set_fx(0xD);
                            for f in [0xCC, 0xCE, 0x24] {
                                self.set(f);
                            }
                            self.me.work[4] += 1;
                            self.me.work[3] = 0;
                        }
                        4 => {
                            self.super_hit_stop(true);
                            self.me.work[3] += 1;
                            if self.me.work[3] < 3 {
                                self.set(0xB);
                            }
                            if self.me.work[3] >= 7 {
                                self.request_anim(super_anim(cls, 2), 0.0);
                            }
                        }
                        _ => {}
                    }
                }
                2 => {
                    self.me.anim_advance_then(super_anim(cls, 3), 0.0, 0);
                    let event = self.query_event(0x400, 0, 0) as f32;
                    let frame = self.me.obj.frame;
                    if frame < event - 4.0 {
                        self.super_hit_stop(true);
                    }
                    if frame < event {
                        aim = true;
                    }
                }
                3 => {
                    self.me.anim_advance_loop(0);
                    self.super_wait_flag(0xA8, cls, 6);
                }
                6 => {
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                }
                _ => {}
            }
            if self.tech().flags(cls) & 0x1000 == 0 {
                let a = self.anim_id();
                self.set_pitch_motion_with(a + 7, a + 0xE, aim);
            }
            if !warped {
                self.step(if aim { 2 } else { 6 }, 5, 2, 0.0, kmh(100.0));
                self.me.apply_gravity();
            }
            self.me.set_fx(0x10);
            self.super_firing_flags();
            self.super_boost_input();
        }
        if phase == DECIDE && self.me.work[1] != 0 {
            self.super_finish(cls);
        }
        if phase == LEAVE {
            self.super_leave(cls);
        }
    }

    /// Actions 0x10F..0x111 (`BtlAct_SuperChargeHandler`): the chargeable technique. The charge loop lasts until
    /// the button command (letting go of the blast button, flag 0xA2) or the charge time; the charge is full after
    /// half of it (ability 0x4A is not ported).
    fn super_charge(&mut self, phase: u32) {
        let cls = self.current_class();
        if phase == ENTER {
            self.play(super_anim(cls, 0), 0.15);
            self.super_begin(cls, false);
            self.super_start_fx(cls);
            self.super_class_flag(cls, false);
            let ki = self.tech().ki(cls);
            self.me.spend_ki(ki, true);
        }
        if phase == RUN {
            let limit = self.tech().charge_limit_frames(cls);
            let mut aim = self.tech().flags_a(cls) & 0x800 != 0;
            match self.anim_id() - super_anim(cls, 0) {
                0 => {
                    aim = true;
                    self.me.anim_advance_then(super_anim(cls, 1), 0.0, 0);
                    self.super_set_class(cls);
                    self.set(0x53);
                }
                1 => {
                    self.me.anim_advance_loop(0);
                    aim = true;
                    self.super_set_class(cls);
                    self.set(0x53);
                    self.me.work[2] += 1;
                    if self.me.work[2] >= limit {
                        self.me.work[0] |= 1;
                    }
                    self.me.charge = self.me.work[2] as f32 * 2.0 / limit as f32;
                    if self.me.charge >= 1.0 {
                        self.me.charge = 1.0;
                        self.me.set_fx(0x1E);
                        self.me.charge_full_frames += 1;
                    }
                    if self.me.work[0] & 1 != 0 {
                        self.request_anim(super_anim(cls, 2), 0.0);
                        self.me.work[2] = 0;
                    }
                }
                2 => {
                    self.me.anim_advance_then(super_anim(cls, 3), 0.0, 0);
                    self.set(0x53);
                    if self.me.obj.frame < self.query_event(0x400, 0, 0) as f32 {
                        aim = true;
                    }
                }
                3 => {
                    self.me.anim_advance_loop(0);
                    self.super_wait_flag(0xA8, cls, 6);
                }
                6 => {
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                }
                _ => {}
            }
            self.super_move(cls, aim);
            self.super_firing_flags();
            self.set(0xA2);
        }
        if phase == DECIDE {
            if self.anim_id() == super_anim(cls, 1) && self.test(0x6F) {
                self.held(0x9E);
                self.held(0x9D);
                self.me.work[0] |= 1;
                if self.me.charge >= 1.0 {
                    self.held(0x9C);
                }
            }
            if self.me.work[1] != 0 {
                self.super_finish(cls);
            }
        }
        if phase == LEAVE {
            self.super_leave(cls);
        }
    }

    /// Actions 0x112..0x114 (`BtlAct_SuperRepeatHandler`): shots fired again and again, each paid with health;
    /// in the fire loop the button command (pressing the blast button, flag 0xA3) fires the next until the shot
    /// limit, the time limit or 1 health. Own technique records are not ported, so none is ever waited for.
    fn super_repeat(&mut self, phase: u32) {
        let cls = self.current_class();
        if phase == ENTER {
            self.super_start(cls, 0.0);
        }
        if phase == RUN {
            let mut aim = self.tech().flags_a(cls) & 0x800 != 0;
            match self.anim_id() - super_anim(cls, 0) {
                0 => {
                    self.me.anim_advance_then(super_anim(cls, 1), 0.0, 0);
                    self.super_hit_stop(false);
                    self.super_set_class(cls);
                    self.super_boost_input();
                    aim = true;
                }
                1 => {
                    self.me.anim_advance_loop(0);
                    self.super_hit_stop(false);
                    self.super_set_class(cls);
                    self.super_boost_input();
                    aim = true;
                    self.super_wait_flag(0xA7, cls, 2);
                }
                2 => {
                    self.me.anim_advance_then(super_anim(cls, 3), 0.0, 0);
                    if self.me.obj.frame < self.query_event(0x400, 0, 0) as f32 {
                        aim = true;
                        self.super_hit_stop(true);
                    }
                    if self.me.anim_is_new() {
                        let recoil = self.tech().recoil(cls);
                        self.me.damage(self.env, self.data, recoil, 0x42B);
                        self.me.work[3] += 1;
                        self.me.work[2] = 0;
                        if self.me.work[3] == 1 {
                            self.clear(0x9E);
                            self.clear(0x9D);
                        }
                    }
                }
                3 => {
                    self.me.anim_advance_loop(0);
                }
                6 => {
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                }
                _ => {}
            }
            self.super_move(cls, aim);
            self.super_firing_flags();
            self.set(0xA3);
        }
        if phase == DECIDE {
            let shots = self.me.work[3];
            let last = self.me.gauge.health == 1 || shots >= self.tech().shots(cls);
            match self.anim_id() - super_anim(cls, 0) {
                2 => {
                    if !last {
                        self.super_set_class(cls);
                    }
                }
                3 => {
                    self.me.work[2] += 1;
                    let stop =
                        self.me.work[2] >= (self.tech().shot_time(cls) * 30.0) as i32 - 1 || last;
                    if stop {
                        // BtlMove_CountOwnBeams: no technique records yet.
                        self.request_anim(super_anim(cls, 6), 0.0);
                        self.me.work[2] = 0;
                    } else {
                        self.super_set_class(cls);
                        if self.me.work[2] >= 16 && self.test(0x6F) {
                            self.request_anim(super_anim(cls, 2), 0.0);
                        }
                    }
                }
                _ => {}
            }
            if self.me.work[1] != 0 {
                self.super_finish(cls);
            }
        }
        if phase == LEAVE {
            self.super_leave(cls);
        }
    }

    /// Actions 0x115..0x117 (`BtlAct_SuperQuickBeamHandler`) and 0x118..0x11A (`...QuickLongBeamHandler`): the
    /// quick forms (flag 0xA4: 30% damage) without the ki, the hit-stop or the charge: the start cut to 0.1 s, the
    /// fire (the long form's second stage until flag 0xA9), the end. `BtlAct_GetPrev` is never 0x2E.
    fn super_quick_beam(&mut self, phase: u32) {
        let cls = self.current_class();
        let long = (0x118..=0x11A).contains(&self.current());
        if phase == ENTER {
            self.play(super_anim(cls, 0), 0.0);
            self.super_begin(cls, !long);
            self.super_start_fx(cls);
            self.held(0xE);
        }
        if phase == RUN {
            let mut aim = self.tech().flags_a(cls) & 0x800 != 0;
            match self.anim_id() - super_anim(cls, 0) {
                0 => {
                    self.me.anim_set_duration(0.1);
                    aim = true;
                    self.me.anim_advance_then(super_anim(cls, 2), 0.0, 0);
                    self.me.charge = 0.0;
                }
                2 => {
                    self.super_fit_to_event(0.1);
                    let next = if long { 4 } else { 3 };
                    self.me.anim_advance_then(super_anim(cls, next), 0.0, 0);
                    if !long && self.me.obj.frame < self.query_event(0x400, 0, 0) as f32 {
                        aim = true;
                    }
                }
                3 if !long => {
                    self.me.anim_advance_loop(0);
                    self.super_wait_flag(0xA8, cls, 6);
                }
                4 if long => {
                    self.super_fit_to_event(0.1);
                    self.me.anim_advance_then(super_anim(cls, 5), 0.0, 0);
                }
                5 if long => {
                    self.me.anim_advance_loop(0);
                    self.super_wait_flag(0xA9, cls, 6);
                }
                6 => {
                    if self.me.anim_advance(0) {
                        self.me.work[1] = 1;
                    }
                }
                _ => {}
            }
            self.super_move(cls, aim);
            self.set(0xD5);
            self.set(0xA4);
        }
        if phase == DECIDE && self.me.work[1] != 0 {
            self.super_finish(cls);
        }
        if phase == LEAVE {
            self.super_leave(cls);
        }
    }

    /// Action 0x105 (`BtlAct_UltimateStartHandler`): before an Ultimate Blast, motion 0x16B under level 2 hit-stop
    /// (the battle frozen round the pose); its end starts the queued technique. The camera cut, the sound and the
    /// head look are not ported.
    fn ultimate_start(&mut self, phase: u32) {
        let slot = self.current_class();
        if phase == ENTER {
            self.play(0x16B, 0.0);
            self.super_begin(slot, true);
            self.me.turn_yaw(self.opp, 2, PI);
            self.me
                .turn_pitch(self.data, self.opp, self.opp_data, 5, PI);
            self.me.turn_model_yaw(PI, 1.0);
            self.super_class_flag(slot, true);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
        }
        if phase == RUN {
            self.me.anim_advance(0);
            self.super_set_class(slot);
            self.step(6, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.me.set_fx(0x10);
            self.super_hit_stop(false);
            if self.queued() == 0x114 {
                self.set(0xA3);
            }
            self.super_boost_input();
        }
        if phase == DECIDE && self.flag(anim_flag::END) {
            let q = self.queued();
            self.request(q);
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

    /// Action 0xE7 (`BtlAct_ActionE7`): slammed into a wall, motion 0xC8 (0xC9 in the air) without flight; then
    /// 0xDB on the ground, the down action when grounded but not standing, else the fall 0xD9. The voice and the
    /// sound are not ported.
    fn wall_slam(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, false);
            self.play(if air { 0xC9 } else { 0xC8 }, 0.0);
            self.clear(0xE);
            self.me.pose.speed = 0.0;
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                if self.flag(0xF) {
                    self.request(0xDB);
                } else if self.flag(0x11) {
                    let d = self.down_action();
                    self.request(d);
                } else {
                    self.request(0xD9);
                }
            }
            self.step(6, 5, 6, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.reaction_flags();
        }
    }

    /// Actions 0xE8, 0xE9 (`BtlAct_ActionE8E9`): caught before a wall (0xE8: turned to face the stage centre and
    /// sent off at 700 km/h) or the opponent (0xE9: stopped), motion 0xD0 (0xD1 in the air), then 0xB. The voice
    /// and the sound are not ported.
    fn wall_recover(&mut self, phase: u32) {
        if phase == ENTER {
            let air = self.me.is_air_motion(self.env, false);
            self.play(if air { 0xD1 } else { 0xD0 }, 0.0);
            if self.current() == 0xE8 {
                self.me.turn_yaw(self.opp, if air { 8 } else { 7 }, PI);
                self.me.set_direction(2);
                self.me.pose.speed = kmh(700.0);
            } else {
                self.me.pose.speed = 0.0;
            }
            self.held(0xE);
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(0xB);
            }
            self.step(6, 5, 2, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.reaction_flags();
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
        self.set_pitch_motion_with(up, down, true);
    }

    /// `BtlAct_SetPitchMotion` with `recalc`: without it the last measured pitch is kept.
    fn set_pitch_motion_with(&mut self, up: i32, down: i32, recalc: bool) {
        if !self.flag(5) {
            self.me.aim_pitch = 0.0;
            return;
        }
        if recalc {
            let own = self.me.obj.node_pos(0x11);
            let mut dir = sub(self.opp.obj.node_pos(0x11), own);
            let len = length(dir);
            if 0.0001 < len {
                dir = dir.map(|v| v * (1.0 / len));
            }
            self.me.aim_pitch = -dir[1].clamp(-1.0, 1.0).asin();
        }
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

    /// The falling blasts' aim: under lock-on at the opponent's node 0x11 from the own, 18..45 degrees below the
    /// horizon; otherwise 45 degrees down along the heading.
    fn aim_falling_blast(&mut self) {
        self.me.aim_dir = if self.flag(5) {
            let from = self.me.obj.node_pos(0x11);
            self.calc_aim_dir(0x11, from, deg(18.0), deg(45.0))
        } else {
            let r = 0.7071;
            let yaw = self.me.pose.yaw;
            [yaw.sin() * r, r, yaw.cos() * r]
        };
    }

    /// Action 0xB2 (`BtlAct_KiBlastB2`): the falling ki blast, motion 0x7A, paid on entry; then 0xB in flight or
    /// the jump 0x11. The voice is not ported.
    fn falling_blast(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x7A, 0.15);
            let cost = ki_cost(self.me, self.data);
            self.me.spend_ki(cost, false);
            self.me.pose.speed = 0.0;
            self.me.pose.fall_speed = 0.0;
            self.me.blast_shots += self.query_event(4, 0, 3);
            self.me.blast_shots_timer = 0x1E;
        }
        if phase == RUN {
            if self.me.anim_advance(0) {
                self.request(if self.flag(0xE) { 0xB } else { 0x11 });
            }
            self.step(2, 5, 7, 0.0, kmh(100.0));
            self.me.brake_vertical();
            self.set(0xD5);
            self.aim_falling_blast();
        }
    }

    /// Action 0xB3 (`BtlAct_KiBlastChargeB3`): the charged falling blast, motions 0x8D wind-up, 0x8E hold (its
    /// progress the charge; left when the blast button is let go, input 0x5B), 0x8F release, paid when it starts;
    /// flag 0x8E until the release fires. The sound and the voice are not ported.
    fn charged_falling_blast(&mut self, phase: u32) {
        if phase == ENTER {
            self.play(0x8D, 0.15);
            self.me.blast_node = self.play_sub_anim(0x8F);
            self.me.set_fx(0x17);
            self.me.blast_shots += 1;
            self.me.blast_shots_timer = 0x1E;
        }
        if phase == RUN {
            let id = self.anim_id();
            match id {
                0x8D => {
                    self.me.anim_advance_then(id + 1, 0.0, 0);
                    self.set(0x8E);
                }
                0x8E => {
                    self.me.anim_advance_then(id + 1, 0.0, 0);
                    self.set(0x8E);
                    self.me.blast_charge = self.me.anim_progress();
                }
                0x8F => {
                    if self.me.anim_advance(0) {
                        self.request(if self.flag(0xE) { 0xB } else { 0x11 });
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
            self.step(2, 5, 7, 0.0, kmh(100.0));
            self.me.brake_vertical();
            self.set(0xD5);
            self.aim_falling_blast();
        }
        if phase == DECIDE && self.anim_id() == 0x8E && self.test(0x5B) {
            let id = self.anim_id();
            self.request_anim(id + 1, 0.0);
        }
    }

    /// Action 0x95 (`BtlAct_KiVolley95`, attack record 0x95): ten ki blasts, motions 0x71 / 0x72 of 0.1 s each
    /// paid as it starts, alternating (always 0x72 with parameter flag 0x100 or a changed body), with their pitch
    /// variants. The voice and the camera cut are not ported.
    fn ki_volley(&mut self, phase: u32) {
        if phase == ENTER {
            self.me.attack = self.me.next_attack;
            self.play(0x71, 0.1);
            self.held(0xE);
            let atk = self.me.attack;
            if atk.flags & 0x80 != 0 {
                self.set(0xCD);
            }
            if (1..4).contains(&atk.lead_in) {
                self.me.set_fx(0xD);
            }
        }
        if phase == RUN {
            let id = self.anim_id();
            if matches!(id, 0x71 | 0x72) {
                if self.me.anim_is_new() {
                    self.me.work[2] += 1;
                    let cost = ki_cost(self.me, self.data);
                    self.me.spend_ki(cost, false);
                }
                self.me.anim_set_duration(0.1);
                if self.me.anim_advance(0) {
                    if self.me.work[2] >= 10 {
                        self.request(0xB);
                    } else {
                        let next = if self.data.tables.params.flags() & 0x100 != 0
                            || self.me.gauge.body_changed
                            || id == 0x71
                        {
                            0x72
                        } else {
                            0x71
                        };
                        self.request_anim(next, 0.0);
                    }
                }
                let a = self.anim_id();
                self.set_pitch_motion(a + 2, a + 4);
            }
            self.step(2, 5, 7, 0.0, kmh(100.0));
            self.me.apply_gravity();
            self.aim_blast(0x11);
        }
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
                    if self.flag(0x5B) {
                        if 0.4 < self.me.anim_progress() {
                            let id = if self.me.height_ratio(self.data, self.env) < 0.5 {
                                0x7B
                            } else {
                                0x7C
                            };
                            if self.queue_attack(id) != 0 {
                                self.set_work_f(0, 0.6);
                            }
                        }
                        if self.flags2() & 2 != 0 && self.test(0x2A) {
                            self.set_queue(0, 0x1F);
                            self.set_work_f(0, 0.5);
                        }
                        if self.work_f(0) < self.me.anim_progress() {
                            let q = self.queued();
                            self.request(q);
                        }
                    }
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
            0xB, 0xD, 0xF, 0x38, 0x44, 0x60, 0x7B, 0xAE, 0xB1, 0xC0, 0xC6, 0xCF, 0xD2, 0xD8, 0xDB,
            0xE1,
        ] {
            assert!(ported(id), "0x{id:X}");
        }
        for id in [0xEC, 0xFD, 0xF6] {
            assert!(!ported(id), "0x{id:X}");
        }
    }
}
