//! Fighter hit resolution (src/battle/btl_char_hit.c, the strike test of eft_detect.c, the attack-record
//! accessors of btl_param.c): does a strike connect, is it avoided or guarded, which reaction it gives the
//! defender and how much health it takes.
//!
//! A hit is the attacker's current attack id (`BtlAtk_GetId`), the 0x30-byte record it selects in the character's
//! attack table, the strike volumes of the motion's open hit window touching the defender's body boxes, and the
//! reaction block `BtlHit_ApplyHit` fills on the defender. A rushing technique (flag 0x49) that touches its
//! opponent starts its sequence (`BtlHit_CheckRush`, `BtlColl_StartThrow`). Not ported: camera shake, pad
//! vibration, sounds and voices.

use std::f32::consts::PI;

use bt3_data::chara_data::{ATTACK_COUNT, AttackRecord};

use crate::btl_act::id_class;
use crate::btl_chr::{Chr, Env, SideData, Throw, kmh, opp_yaw, opp_yaw_from_facing};
use crate::btl_input::wrap_angle;
use crate::btl_obj::body_hit_test;

/// `BTL_ATK_CHARGE_MID` / `BTL_ATK_CHARGE_FULL`: the weak / medium / full tiers of a charged attack.
const CHARGE_MID: f32 = 0.3;
const CHARGE_FULL: f32 = 0.99;

/// `BtlAtk_GetId`: the attack record the fighter's current action / motion uses, -1 when it is not attacking.
/// The attack actions 0x70.. take theirs from the roster's table; then by action, then by motion, as the original.
pub(crate) fn attack_id(me: &Chr, data: &SideData, env: &Env) -> i32 {
    let action = me.act.current;
    if (0x70..0x70 + 0x3E).contains(&action) {
        return env
            .common
            .attack_actions
            .get((action - 0x70) as usize)
            .map_or(-1, |row| i32::from(row[3]));
    }
    let charge = me.charge;
    let tier = |a: i32, b: i32, c: i32| {
        if charge < CHARGE_MID {
            a
        } else if charge < CHARGE_FULL {
            b
        } else {
            c
        }
    };
    let alt = me.dash_count != 0;
    let frame = me.obj.frame;
    let last = me.obj.query_event(&data.obj, 1, 0, 2) as f32;
    match action {
        0x46 => {
            return match me.anim.cur {
                0x3C => 0x4E,
                0x3D => 0x4F,
                0x3E => 0x50,
                0x3F => 0x51,
                0x40 => 0x52,
                _ => -1,
            };
        }
        0xB4 | 0xB5 | 0xB7 | 0xBB => return 0x55,
        0xB6 | 0xB9 => return 0x56,
        0x60 => return tier(0xB, 0xC, 0xD),
        0x5A => return 0x43,
        0x5B => return 0x44,
        0x5C => return 0x45,
        0x5D => return 0x46,
        0x6B => return if alt { 0x3E } else { 0x34 },
        0x6C => return if alt { 0x3F } else { 0x35 },
        0x6D => return if alt { 0x40 } else { 0x36 },
        0x6E => return if alt { 0x41 } else { 0x37 },
        0x6F => return if alt { 0x42 } else { 0x38 },
        0x64 => return tier(0x59, 0x5B, 0x5D),
        0x65 => return tier(0x5A, 0x5C, 0x5E),
        0x61 => return tier(0x2E, 0x2F, 0x30),
        0x62 => return tier(0x31, 0x32, 0x33),
        0x37 | 0x42 => return 0x66,
        0x4F => return 0x67,
        0x50 => return 0x68,
        0x4D => return 0x69,
        0x4E => return 0x6A,
        0x51 | 0x52 => return 0x6B,
        _ => {}
    }
    // The smash's release motion: the instant (flag 0x84) record, else by charge.
    let smash = |instant: i32, a: i32, b: i32, c: i32| {
        if me.flags.test(0x84) {
            instant
        } else {
            tier(a, b, c)
        }
    };
    let charged = |alt_id: i32, low: i32, full: i32| {
        if alt {
            alt_id
        } else if charge < CHARGE_FULL {
            low
        } else {
            full
        }
    };
    let two = |early: i32, late: i32| if frame < last { early } else { late };
    match me.anim.cur {
        0x37 => {
            // The first rush hit is record 1 when its motion has a second window that has begun.
            let h = &me.obj.hit;
            if h.count >= 2 && h.index != 0 && h.index != -1 {
                1
            } else {
                0
            }
        }
        0x38..=0x40 => me.anim.cur - 0x36,
        0x93 => two(0x11, 0x12),
        0x58 => smash(0x67, 0x13, 0x18, 0x1D),
        0x5B => smash(0x68, 0x14, 0x19, 0x1E),
        0x52 => smash(0x69, 0x15, 0x1A, 0x1F),
        0x55 => smash(0x6A, 0x16, 0x1B, 0x20),
        0x4C | 0x4F => smash(0x6B, 0x17, 0x1C, 0x21),
        0x67 => charged(0x3E, 0x34, 0x39),
        0x6A => charged(0x3F, 0x35, 0x3A),
        0x61 => charged(0x40, 0x36, 0x3B),
        0x64 => charged(0x41, 0x37, 0x3C),
        0x5E => charged(0x42, 0x38, 0x3D),
        0x43 => tier(0x48, 0x49, 0x4A),
        0x46 => tier(0x4B, 0x4C, 0x4D),
        0x6D => {
            if charge < CHARGE_MID {
                two(0x22, 0x23)
            } else if charge < CHARGE_FULL {
                two(0x24, 0x25)
            } else {
                two(0x26, 0x27)
            }
        }
        0x70 => {
            if charge < CHARGE_MID {
                two(0x28, 0x29)
            } else if charge < CHARGE_FULL {
                two(0x2A, 0x2B)
            } else {
                two(0x2C, 0x2D)
            }
        }
        0xF8 => 0x57,
        0x19B => {
            let h = &me.obj.hit;
            if h.index == 0 {
                tier(0x7E, 0x81, 0x84)
            } else if h.index < h.count - 1 {
                tier(0x7F, 0x82, 0x85)
            } else {
                tier(0x80, 0x83, 0x86)
            }
        }
        _ => -1,
    }
}

/// `BtlAtk_GetRecord`: ids outside 0..0xA2 give record 0.
fn record<'a>(me: &Chr, data: &'a SideData, env: &Env) -> (&'a AttackRecord, usize) {
    let id = attack_id(me, data, env);
    let n = if (0..ATTACK_COUNT as i32).contains(&id) {
        id as usize
    } else {
        0
    };
    (&data.tables.attacks[n], n)
}

/// `BtlAtk_GetLaunchAngleBOf`: record `id`'s +0x16 (degrees), the launch pitch; ids past the table give record 0.
pub(crate) fn launch_angle_b_of(data: &SideData, id: usize) -> f32 {
    let n = if id < ATTACK_COUNT { id } else { 0 };
    f32::from(data.tables.attacks[n].s8_at(0x16)) * PI / 180.0
}

/// `BtlAtk_GetLaunchAngleA`: record +0x15 (degrees), the launch yaw.
pub(crate) fn launch_angle_a(me: &Chr, data: &SideData, env: &Env) -> f32 {
    let (r, _) = record(me, data, env);
    f32::from(r.s8_at(0x15)) * PI / 180.0
}

/// `BtlAtk_GetFlags`.
pub(crate) fn attack_flags(me: &Chr, data: &SideData, env: &Env) -> u32 {
    record(me, data, env).0.flags()
}

/// `BtlAtk_GetThrowParamC` / `E`: record +0x0C / +0x0E, a throw's health and ki drains.
pub(crate) fn throw_drains(me: &Chr, data: &SideData, env: &Env) -> (i32, i32) {
    let (r, _) = record(me, data, env);
    (i32::from(r.u16_at(0xC)), i32::from(r.u16_at(0xE)))
}

/// `BtlAtk_GetLaunchSpeed`: record +0x14 in 10 km/h, times curve row 8.
pub(crate) fn launch_speed(me: &Chr, data: &SideData, env: &Env) -> f32 {
    let (r, _) = record(me, data, env);
    f32::from(r.u8_at(0x14)) * 10.0 * kmh(1.0) * me.curve(env, 8)
}

/// The number of hit windows of the motion (`BtlObjAnim_QueryEvent(obj, 1, 0, 3)`).
fn hit_windows(me: &Chr, data: &SideData) -> i32 {
    me.obj.query_event(&data.obj, 1, 0, 3)
}

/// `BtlAtk_ApplyAbilities`: attacks 0..9 +10 % (ability 4) / -10 % (5), 0x13..0x21 +10 % (6) / -10 % (7), the
/// throws 0x55 / 0x56 doubled (8) / halved (9).
fn apply_atk_abilities(me: &Chr, mut v: i32, id: usize) -> i32 {
    let mut step = |up: i32, down: i32| {
        if me.has_ability(up) {
            v += v / 10;
        }
        if me.has_ability(down) {
            v -= v / 10;
        }
    };
    if id < 10 {
        step(4, 5);
    }
    if (0x13..0x22).contains(&id) {
        step(6, 7);
    }
    if (0x55..0x57).contains(&id) {
        if me.has_ability(8) {
            v *= 2;
        }
        if me.has_ability(9) {
            v -= v / 2;
        }
    }
    v
}

/// `BtlAtk_GetDamage`: record damage times curve row 4, the abilities' changes, divided by the motion's hits.
pub(crate) fn damage(me: &Chr, data: &SideData, env: &Env) -> i32 {
    let (r, id) = record(me, data, env);
    let v = (i32::from(r.u16_at(4)) as f32 * me.curve(env, 4)) as i32;
    let mut v = apply_atk_abilities(me, v, id);
    let hits = hit_windows(me, data);
    if hits >= 2 {
        v /= hits;
    }
    v
}

/// `BtlAtk_GetGuardDamage`: the same on record +6, 30 % less for attacks 0..10 under powered skill 0x10.
fn guard_damage(me: &Chr, data: &SideData, env: &Env) -> i32 {
    let (r, id) = record(me, data, env);
    let mut v = (i32::from(r.u16_at(6)) as f32 * me.curve(env, 4)) as i32;
    // BtlAct_TestPoweredSkill(0x10).
    if me.flags.test(6) && data.tables.params.flags3() & 0x10 != 0 && id < 0xB {
        v -= v * 3 / 10;
    }
    let mut v = apply_atk_abilities(me, v, id);
    let hits = hit_windows(me, data);
    if hits >= 2 {
        v /= hits;
    }
    v
}

/// `BtlAtk_GetGuardKiCost`: record +8 times curve row 5, doubled (ability 0x44) or +50 % (0x6D).
fn guard_ki_cost(me: &Chr, data: &SideData, env: &Env) -> i32 {
    let (r, _) = record(me, data, env);
    let v = (i32::from(r.u16_at(8)) as f32 * me.curve(env, 5)) as i32;
    if me.has_ability(0x44) {
        v * 2
    } else if me.has_ability(0x6D) {
        v + v / 2
    } else {
        v
    }
}

/// `BtlAtk_GetGuardKindA` (record +0x29) / `B` (+0x27) / `C` (+0x28): result 1 becomes 0 when the defender has
/// ability 0x40 (for B and C only on attacks 0x13..0x21).
fn guard_result(rec: &AttackRecord, id: usize, def: &Chr, o: usize) -> i8 {
    let v = rec.s8_at(o);
    if v != 1 || !def.has_ability(0x40) {
        return v;
    }
    if o == 0x29 {
        0
    } else {
        i8::from(!(0x13..0x22).contains(&id))
    }
}

/// `BtlAtk_GetKiGain`: +50 % (ability 0x6E) or +25 % (0x20), divided by the motion's hits.
fn ki_gain(me: &Chr, data: &SideData, env: &Env) -> i32 {
    let (r, _) = record(me, data, env);
    let mut v = i32::from(r.u16_at(0xA));
    if me.has_ability(0x6E) {
        v += (v as u32 / 2) as i32;
    } else if me.has_ability(0x20) {
        v += v / 4;
    }
    let hits = hit_windows(me, data);
    if hits > 0 {
        v /= hits;
    }
    v
}

/// `BtlAtk_GetChargeGaugeGain`.
fn charge_gauge_gain(me: &Chr, data: &SideData, env: &Env) -> i32 {
    let (r, _) = record(me, data, env);
    let mut v = i32::from(r.u16_at(0x10));
    let hits = hit_windows(me, data);
    if hits > 0 {
        v /= hits;
    }
    v
}

/// A reaction id with its fallback to record +0x1B when 0.
fn reaction_or_base(r: &AttackRecord, o: usize) -> i32 {
    match r.s8_at(o) {
        0 => i32::from(r.s8_at(0x1B)),
        v => i32::from(v),
    }
}

/// `BtlColl_IsGuarding`: guard kinds 1..4.
fn is_guarding(me: &Chr) -> bool {
    (1..=4).contains(&guard_kind(me))
}

/// `BtlSuper_GetThrowChara`: the character a rush brings in: the table's, or its alternate when the user is that
/// character or (characters of 0x100 and above with technique flag 0x40000000) wears costume 1.
fn throw_chara(data: &SideData, slot: i32) -> i32 {
    let t = data.tables.tech();
    let (chara, alt) = (t.throw_chara(slot), t.throw_chara_alt(slot));
    if data.chara == chara {
        return alt;
    }
    if chara >= 0x100 && t.flags(slot) & 0x4000_0000 != 0 && data.costume == 1 {
        return alt;
    }
    chara
}

/// `BtlColl_StartThrow`: a rush technique of `slot` catches: the attacker's sequence block is filled (flag 0x72)
/// and copied to the victim (reaction 0x22), both facing the attacker's heading and out of reach (flags
/// 0x44..0x46). Refused while the attacker is hit or either is down. The battle events are not ported.
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_throw(
    atk: &mut Chr,
    def: &mut Chr,
    atk_data: &SideData,
    env: &Env,
    slot: i32,
    tech_flags: u32,
    frame: u32,
) -> bool {
    if atk.react.reaction >= 3 || atk.is_dead() || def.is_dead() || !atk.flags.test(3) {
        return false;
    }
    let t = atk_data.tables.tech();
    atk.flags.set(0x72);
    // BtlSuper_GetThrowCostume / GetThrowGauge20: the user's own for a character with flag 0x40000000.
    let own = t.throw_chara(slot) < 0x100 && t.flags(slot) & 0x4000_0000 != 0;
    let costume = if t.throw_chara(slot) >= 0x100 {
        0
    } else if own {
        atk_data.costume
    } else if t.id(slot) == 0x27F {
        1
    } else {
        0
    };
    let angles = tech_flags & 0x40_0000 != 0;
    atk.thr = Throw {
        tech: t.id(slot),
        atk_side: atk.player as i32,
        def_side: def.player as i32,
        slot,
        step_count: t.steps(slot),
        partner_chara: throw_chara(atk_data, slot),
        partner_costume: costume,
        partner_variant: if own { atk.gauge.variant } else { 0 },
        partner_slot: t.throw_object_slot(slot),
        partner_step: t.throw_partner_step(slot),
        last_step: t.last_step(slot),
        failed: tech_flags & 0x400 != 0 && frame % 2 != 0,
        landing_kind: t.landing_kind(slot),
        turn_victim: angles,
        turn_yaw: if angles { t.throw_angle_a(slot) } else { 0.0 },
        turn_pitch: if angles { t.throw_angle_b(slot) } else { 0.0 },
        ko_skip_landing: t.flags_a(slot) & 0x200_0000 != 0,
        caught_loop: false,
        atk_reload: tech_flags & 0x2000 != 0 && atk.gauge.variant == 0,
        def_reload: tech_flags & 0x8_0000 != 0 && def.gauge.variant == 0,
        place_first_step: tech_flags & 0x800 != 0,
        unk54: tech_flags & 0x1_0000 != 0,
        half_damage: env.anim_flags(def.anim.cur) & 0x800 != 0,
        raise_at_end: tech_flags & 0x400_0000 != 0,
        ..atk.thr
    };
    atk.flags.set(0x94);
    atk.react.turn_yaw = atk.pose.yaw;
    atk.react.turn_pitch = 0.0;
    def.react.reaction = 0x22;
    def.thr = Throw {
        partner_chara: -1,
        partner_costume: 0,
        partner_variant: 0,
        partner_slot: 1,
        hold_victim: def.thr.hold_victim,
        face_attacker: def.thr.face_attacker,
        ..atk.thr
    };
    def.flags.set(0x94);
    def.react.turn_yaw = atk.react.turn_yaw;
    def.react.turn_pitch = atk.react.turn_pitch;
    def.react.yaw = atk.react.turn_yaw;
    def.react.back = tech_flags & 0x1000_0000 != 0;
    for f in [0x44, 0x45, 0x46] {
        atk.flags.set(f);
        def.flags.set(f);
    }
    raise_first_clash(atk, def);
    atk.flags.set(0x128);
    true
}

/// `BtlColl_StartRushCatch`: a technique's catch reaction (0x1D, 0x1E looping) lands: the attacker's block (flag
/// 0x73) without a sequence, copied to the victim. The hit sound is not ported.
pub(crate) fn start_rush_catch(
    def: &mut Chr,
    atk: &mut Chr,
    atk_data: &SideData,
    env: &Env,
    slot: i32,
    react: i32,
    yaw: f32,
) -> bool {
    if atk.react.reaction >= 3 || atk.is_dead() || def.is_dead() || !atk.flags.test(3) {
        return false;
    }
    let t = atk_data.tables.tech();
    let tf = t.flags(slot);
    def.react.reaction = react;
    def.react.scale = 0.0;
    def.react.turn_yaw = 0.0;
    def.react.turn_pitch = 0.0;
    let angles = tf & 0x40_0000 != 0;
    atk.thr = Throw {
        tech: t.id(slot),
        atk_side: atk.player as i32,
        def_side: def.player as i32,
        slot,
        landing_kind: t.landing_kind(slot),
        turn_victim: angles,
        turn_yaw: if angles { t.throw_angle_a(slot) } else { 0.0 },
        turn_pitch: if angles { t.throw_angle_b(slot) } else { 0.0 },
        hold_victim: tf & 0x10_0000 != 0,
        face_attacker: tf & 0x80_0000 != 0,
        caught_loop: react == 0x1E,
        unk54: tf & 0x1_0000 != 0,
        half_damage: env.anim_flags(def.anim.cur) & 0x800 != 0,
        atk_reload: false,
        partner_step: -1,
        last_step: -1,
        def_reload: false,
        place_first_step: false,
        raise_at_end: false,
        ..atk.thr
    };
    atk.flags.set(0x73);
    let a = atk.thr;
    let d = &mut def.thr;
    d.tech = a.tech;
    d.atk_side = a.atk_side;
    d.def_side = a.def_side;
    d.slot = a.slot;
    d.landing_kind = a.landing_kind;
    d.turn_victim = a.turn_victim;
    d.hold_victim = a.hold_victim;
    d.face_attacker = a.face_attacker;
    d.turn_yaw = a.turn_yaw;
    d.turn_pitch = a.turn_pitch;
    d.caught_loop = a.caught_loop;
    d.atk_reload = a.atk_reload;
    d.def_reload = a.def_reload;
    d.partner_step = a.partner_step;
    d.last_step = a.last_step;
    d.place_first_step = a.place_first_step;
    d.unk54 = a.unk54;
    d.half_damage = a.half_damage;
    d.raise_at_end = a.raise_at_end;
    def.react.yaw = yaw;
    def.react.back = tf & 0x1000_0000 != 0;
    true
}

/// `BtlHit_IsFromBehind`: the attacker's heading within 90 degrees of the defender's model yaw.
fn is_from_behind(atk: &Chr, def: &Chr) -> bool {
    let ang = wrap_angle(def.pose.rot[1] + def.pose.root_rot[1]);
    wrap_angle(ang - atk.pose.yaw).abs() < 1.570_796_3
}

/// `BtlHit_GetHitNo`: the motion's hit counter while the strike touches, else the contact frames, else -1.
fn hit_no(me: &Chr) -> i32 {
    if me.obj.work.hit_flags & 0xF00_0000 != 0 {
        i32::from(me.obj.hit.index)
    } else if me.flags.test(0x65) {
        me.status.contact
    } else {
        -1
    }
}

/// `BtlHit_GetVoiceKind`: the voice a hit reaction plays (`BtlChar_PlayVoice`; the hit voices are not ported yet).
#[allow(dead_code)]
pub(crate) fn voice_kind(reaction: i32) -> i32 {
    match reaction {
        5..=10 | 29 | 30 | 35 | 36 | 42 | 47 | 48 => 1,
        15..=17 | 19 => 2,
        18 | 40 | 41 => 3,
        12 | 13 | 23..=28 | 38 | 39 => 4,
        _ => 0,
    }
}

/// `BtlChar_RaiseFirstClash`: the first hit of the battle (flag 0xE3; battle event 0x3C is not ported).
pub(crate) fn raise_first_clash(atk: &mut Chr, def: &Chr) {
    if !atk.first_hit && !def.first_hit {
        atk.first_hit = true;
        atk.flags.set(0xE3);
    }
}

/// `BtlColl_GetGuardKind`: the guard motions 0xEC..0xF7.
pub(crate) fn guard_kind(me: &Chr) -> i32 {
    match me.anim.cur {
        0xEF | 0xF0 => 1,
        0xF1 | 0xF2 => 2,
        0xF3 | 0xF4 => 3,
        0xEC | 0xF5 => 4,
        0xF7 => 5,
        _ => 0,
    }
}

/// Both fighters with their data, indexed by player.
pub(crate) struct Pair<'a, 'd> {
    pub chars: &'a mut [Chr; 2],
    pub data: [&'a SideData<'d>; 2],
    pub env: &'a Env<'d>,
    pub frame: u32,
}

impl Pair<'_, '_> {
    /// `BtlBodyHit_Update`: the strike of side 0 against the body of side 1, then the reverse.
    pub(crate) fn body_hit_update(&mut self) {
        let [a, b] = self.chars;
        body_hit_test(&mut a.obj, 0, &mut b.obj, 1);
        body_hit_test(&mut b.obj, 1, &mut a.obj, 0);
    }

    /// `BtlColl_Update`: begin frame, clash, rush, proximity, per fighter the hit test, the trades, the hits.
    pub(crate) fn update(&mut self) {
        for chr in self.chars.iter_mut() {
            begin_frame(chr);
        }
        if self.check_clash() || self.check_rush() {
            return;
        }
        for a in 0..2 {
            let (data, opp_data) = (self.data[a], self.data[a ^ 1]);
            let (me, opp) = split(self.chars, a);
            check_near(me, opp, data);
            check_grab(me, opp, data, opp_data, self.env);
            check_touch(me, opp, data, opp_data);
        }
        for a in 0..2 {
            self.chars[a].status.target = -1;
            self.test_hit(a);
        }
        self.resolve_trades();
        for a in 0..2 {
            self.apply_hit(a);
        }
    }

    /// `BtlHit_CheckClash`: two touching fighters both in a clashing motion (flags 0x47 / 0x48).
    /// `BtlHit_CheckRush`: a rushing technique (flag 0x49) touching its opponent misses (0x63) one that cannot be
    /// caught, bounces (0x64) off a giant, is broken or dodged, is guarded from the front (0x6F; 20000 ki), or
    /// starts its sequence, which ends the collision step (a rush break fills the blast stocks by one with
    /// ability 0x22). Sounds and voices are not ported.
    fn check_rush(&mut self) -> bool {
        let (env, frame) = (self.env, self.frame);
        for a in 0..2 {
            let (data, opp_data) = (self.data[a], self.data[a ^ 1]);
            let (me, opp) = split(self.chars, a);
            if !me.flags.test(0x49)
                || !crate::btl_chr::is_blocked_by_opponent(me, opp, data, opp_data)
            {
                continue;
            }
            let anim = env.anim_flags(opp.anim.cur);
            let slot = id_class(me.act.current);
            let tf = data.tables.tech().flags(slot);
            if opp.flags.test(0x42)
                || anim & 0x800 != 0
                || (opp.flags.test(0x49) && opp.flags.test(0xA5) && !me.flags.test(0xA5))
            {
                me.flags.set(0x63);
                continue;
            }
            if (opp_data.tables.params.size_class() == 4 && tf & 0x800_0000 == 0)
                || opp.flags.test(0x59)
            {
                me.flags.set(0x64);
                me.flags.set(0x129);
                continue;
            }
            if opp.is_free() && opp.rush_break_window > 0 {
                if me.flags.test(0xA5) {
                    if anim & 0x31 != 0 && !is_from_behind(me, opp) {
                        me.flags.set(0x7A);
                        opp.flags.set(0x79);
                        continue;
                    }
                } else if anim & 0xB1 != 0 && !opp.flags.test(0x5A) {
                    me.flags.set(0x63);
                    opp.flags.set(if frame % 2 != 0 { 0x74 } else { 0x75 });
                    if opp.has_ability(0x22) {
                        opp.add_blast(100_000);
                    }
                    continue;
                }
            }
            if opp.is_free() && !opp.flags.test(0x5A) {
                let (use_a, use_b) = (opp.dodges > 0, opp.dodges_b > 0);
                if (use_a || use_b) && anim & 0xB1 != 0 {
                    if use_a {
                        opp.dodges -= 1;
                    }
                    if use_b {
                        opp.dodges_b -= 1;
                    }
                    me.flags.set(0x63);
                    if use_a {
                        opp.flags.set(if frame % 2 != 0 { 0x74 } else { 0x75 });
                    }
                    if use_b {
                        opp.flags.set(0x77);
                    }
                    match opp.skill_slot {
                        0 => opp.flags.set(0xDE),
                        1 => opp.flags.set(0xDF),
                        _ => {}
                    }
                    continue;
                }
            }
            if is_guarding(opp) && !is_from_behind(me, opp) && tf & 1 == 0 && !me.flags.test(0xA5) {
                me.flags.set(0x6F);
                me.flags.set(0x129);
                opp.set_fx(0x20);
                opp.flags.set(0x6D);
                opp.flags.set(0x94);
                opp.react.turn_yaw = wrap_angle(me.pose.yaw + PI);
                opp.react.turn_pitch = 0.0;
                opp.spend_ki(20000, false);
                if opp.is_ki_empty() {
                    opp.flags.set_held(0xBE);
                }
                continue;
            }
            if !me.flags.test(3) {
                me.flags.set(0x63);
                continue;
            }
            if start_throw(me, opp, data, env, slot, tf, frame) {
                return true;
            }
        }
        false
    }

    fn check_clash(&mut self) -> bool {
        for a in 0..2 {
            let (data, opp_data) = (self.data[a], self.data[a ^ 1]);
            let (me, opp) = split(self.chars, a);
            if !me.flags.test(3) {
                continue;
            }
            let same = me.flags.test(0x47) && opp.flags.test(0x47);
            let mixed = (me.flags.test(0x48) && opp.flags.test(0x48))
                || (me.flags.test(0x47) && opp.flags.test(0x48))
                || (me.flags.test(0x48) && opp.flags.test(0x47));
            let touching = crate::btl_chr::is_blocked_by_opponent(me, opp, data, opp_data)
                || crate::btl_chr::is_blocked_by_opponent(opp, me, opp_data, data);
            if (same || mixed) && touching {
                let (flag, len) = if same { (0x61, 6) } else { (0x62, 2) };
                for c in [&mut *me, &mut *opp] {
                    c.flags.set(flag);
                    c.freeze_next = len;
                    c.freeze_delay = 0;
                    c.flags.set(0x44);
                    c.flags.set(0x45);
                    c.flags.set(0x46);
                }
                if !same {
                    me.set_fx(0x2C);
                }
                return true;
            }
        }
        false
    }

    /// `BtlHit_TestHit`: an attack id, a touching strike (or contact) and the opponent within 90 degrees of the
    /// facing: the avoid / guard tests, else the opponent becomes the pending target.
    fn test_hit(&mut self, a: usize) {
        let (env, frame) = (self.env, self.frame);
        let (data, opp_data) = (self.data[a], self.data[a ^ 1]);
        let (me, opp) = split(self.chars, a);
        if me.flags.test(0x60) || attack_id(me, data, env) == -1 {
            return;
        }
        if me.obj.work.hit_flags & 0xF00_0000 != 0 || me.flags.test(0x65) {
            let flags = record(me, data, env).0.flags();
            let ok = opp_yaw_from_facing(me, opp).abs() < 1.570_796_3 || flags & 8 != 0;
            if ok {
                me.flags.set_held(0x60);
                if is_void(me, data, opp, env)
                    || check_dodge(me, opp, data, env, frame)
                    || check_armor(me, opp, data, env)
                    || check_repel(me, opp)
                {
                    me.flags.set_held(0x5C);
                } else if let Some(full) = check_guard(me, opp, data, env) {
                    if full {
                        let guard = guard_damage(me, data, env);
                        opp.damage(env, opp_data, guard, 0xC);
                    }
                } else {
                    // BtlHit_CheckThrowBreak concerns attack 0x55 (a throw) only.
                    me.status.target = (a ^ 1) as i32;
                }
            }
        }
        if me.flags.test(0x65) {
            me.status.contact += 1;
        }
    }

    /// `BtlHit_ResolveTrades`: two fighters hitting each other: the higher priority wins, equal ones cancel.
    fn resolve_trades(&mut self) {
        let env = self.env;
        let [a, b] = &mut *self.chars;
        if a.status.target == 1 && b.status.target == 0 {
            let pa = record(a, self.data[0], env).0.s8_at(0x26);
            let pb = record(b, self.data[1], env).0.s8_at(0x26);
            if pa > pb {
                b.status.target = -1;
            } else if pa < pb {
                a.status.target = -1;
            } else {
                a.status.target = -1;
                b.status.target = -1;
                for c in [a, b] {
                    c.set_fx(0x22);
                    c.flags.set_held(0x5D);
                }
            }
        }
    }

    /// `BtlHit_ApplyHit`: the reaction by the defender's motion flags (or "no flinch" from armour), the reaction
    /// block, the push, the damage, the attacker's ki and gauge gains.
    fn apply_hit(&mut self, a: usize) {
        let target = self.chars[a].status.target;
        if target < 0 {
            return;
        }
        let env = self.env;
        let (data, opp_data) = (self.data[a], self.data[a ^ 1]);
        let (atk, def) = split(self.chars, a);
        let (rec, atk_id) = record(atk, data, env);
        let atk_flags = rec.flags();
        let anim = def.anim.cur;
        let anim_flags = env.anim_flags(anim);
        let yaw = if atk_flags & 0x1000_0000 != 0 {
            opp_yaw(atk, def)
        } else {
            atk.pose.yaw
        };
        let back = is_from_behind(atk, def);
        let mut drain = false;
        let mut react = if anim_flags & 0x1000 != 0 {
            reaction_or_base(rec, 0x1D)
        } else if anim_flags & 0x2000 != 0 {
            reaction_or_base(rec, 0x1E)
        } else if anim_flags & 0x4000 != 0 || def.act.current == 0xDF {
            reaction_or_base(rec, 0x1F)
        } else if ((anim - 0xAE) as u32) < 2 || anim_flags & 0x200 != 0 {
            reaction_or_base(rec, 0x22)
        } else {
            let def_flags = opp_data.tables.params.flags();
            let atk_param = data.tables.params.flags();
            let mut armor = i32::from(def_flags & 4 != 0);
            if def_flags & 8 != 0 {
                armor -= 1;
            }
            if atk_param & 4 != 0 {
                armor -= 1;
            }
            if atk_param & 8 != 0 {
                armor += 1;
            }
            // BtlAct_TestPoweredSkill(def, 0x40) / (atk, 0x100), ability 0x46 of the defender, 0x47 (or 0x70 in
            // the powered-up mode) of the attacker.
            if def.flags.test(6) && opp_data.tables.params.flags3() & 0x40 != 0 {
                armor += 1;
            }
            if def.skill_timer_c > 0 {
                armor += 1;
            }
            if def.has_ability(0x46) {
                armor += 1;
            }
            if atk.flags.test(6) && data.tables.params.flags3() & 0x100 != 0 {
                armor -= 1;
            }
            if atk.has_ability(0x47) || (atk.has_ability(0x70) && atk.flags.test(6)) {
                armor -= 1;
            }
            armor -= i32::from(rec.s8_at(0x2C));
            if armor > 0 && atk_flags & 4 != 0 {
                if def_flags & 4 != 0 {
                    drain = true;
                }
                2
            } else if anim_flags & 0x800 != 0 {
                reaction_or_base(rec, 0x1C)
            } else if anim_flags & 0x100 != 0 {
                let last = def.obj.query_event(&opp_data.obj, 1, 0, 3) != 0
                    && i32::from(def.obj.hit.index) == !i32::from(def.obj.hit.count);
                if back || last {
                    i32::from(rec.s8_at(0x1B))
                } else {
                    reaction_or_base(rec, 0x21)
                }
            } else {
                i32::from(rec.s8_at(0x1B))
            }
        };
        if react == 1 {
            atk.flags.set_held(0x5C);
            return;
        }
        if react == 0x14 && data.tables.params.flags() & 0x200 != 0 {
            react = 0x25;
        }
        let launch = matches!(react, 0x14 | 0x15 | 0x16 | 0x25 | 0x31);
        if !launch && def.no_flinch > 0 {
            react = 2;
        }
        def.react.reaction = react;
        if react != 2 {
            def.react.reaction_sub = i32::from(rec.s8_at(0x20));
            if atk.gauge.body_changed {
                def.react.reaction_sub = 0x13;
            }
            def.react.silent = (atk_flags >> 10) & 1 != 0;
            def.react.scale = if react == 0x14 {
                1.0
            } else if atk_flags & 0x200_0000 != 0 {
                atk.charge
            } else {
                atk.charge * opp_data.tables.params.f32_at(0x60)
            };
            if atk_flags & 0x40 != 0 {
                def.flags.set_held(0xE);
            }
            if atk_flags & 0x40_0000 != 0 {
                def.flags.set_held(0x1E);
            }
            // The hurt voice (BtlChar_PlayVoice with BtlHit_GetVoiceKind) is not ported.
        } else {
            def.shake_timer = 3;
            if drain {
                def.spend_ki(2000, false);
            }
        }
        if rec.s8_at(0x23) != 0 {
            def.set_fx(0x1D);
        }
        if atk_flags & 0x8_0000 == 0 {
            atk.set_fx(0x1B);
        }
        for (bit, fx) in [(0x1_0000, 0x25), (0x2_0000, 0x26), (0x4_0000, 0x27)] {
            if atk_flags & bit != 0 {
                atk.set_fx(fx);
            }
        }
        atk.flags.clear(0x8A);
        atk.flags.set_held(0x5B);
        if anim_flags & 2 != 0 {
            atk.flags.set(0x71);
        }
        if atk_flags & 0x200 != 0 {
            atk.flags.set_held(0x86);
        }
        if atk_flags & 0x20 != 0 {
            crate::btl_chr::aim_vertical_at_opponent(atk, def, data, opp_data, false);
        }
        if react == 0x31 {
            atk.flags.set(0x7F);
        }
        let angle = |o: usize| f32::from(rec.s8_at(o)) * PI / 180.0;
        if react != 2 {
            if launch {
                let (mut launch_a, mut launch_b) = (angle(0x15), angle(0x16));
                def.react.back = (atk_flags >> 12) & 1 != 0;
                if atk.gauge.body_changed && atk_id == 0x55 {
                    def.react.back = false;
                    launch_a = 0.0;
                    launch_b = -0.785_398_07;
                }
                def.react.yaw = yaw;
                def.react.launch_a = launch_a;
                def.react.launch_b = launch_b;
                def.react.turn_yaw = 0.0;
                def.react.turn_pitch = 0.0;
                for f in [0x44, 0x45, 0x46] {
                    atk.flags.set(f);
                    def.flags.set(f);
                }
            } else {
                def.react.back = back;
                def.react.yaw = yaw;
                let push = f32::from(rec.u8_at(0x12)) * 10.0 * kmh(1.0);
                def.set_impulse_yaw(yaw, push);
                if atk_flags & 0x100 != 0 {
                    def.flags.set(0x94);
                    let (mut a_ang, mut b_ang) = (angle(0x15), angle(0x16));
                    if atk_flags & 0x20_0000 != 0 {
                        a_ang = 0.0;
                        b_ang = atk.pose.pitch;
                    }
                    if def.react.back {
                        def.react.turn_yaw = wrap_angle(yaw + a_ang);
                        def.react.turn_pitch = b_ang;
                    } else {
                        def.react.turn_yaw = wrap_angle(wrap_angle(yaw + a_ang) + PI);
                        def.react.turn_pitch = -b_ang;
                    }
                }
                def.react.launch_a = 0.0;
                def.react.launch_b = 0.0;
            }
        }
        let no_blend = atk_id == 0x55 && (data.tables.params.flags() >> 19) & 1 != 0;
        atk.react.no_blend = no_blend;
        def.react.no_blend = no_blend;
        match react {
            0x14 | 0x16 | 0x25 => def.react.damage = damage(atk, data, env),
            _ => {
                let mut dmg = damage(atk, data, env);
                match atk_id {
                    0x75..=0x78 => {
                        dmg += dmg * (atk.evasion_count + def.evasion_count - 1).clamp(0, 5) / 5;
                    }
                    _ => {
                        if def.react.back && atk_flags & 0x800_0000 == 0 && anim_flags & 0x800 == 0
                        {
                            dmg += dmg / 5;
                        }
                    }
                }
                let flags = match atk_id {
                    0x43..=0x47 => 0x800_0000,
                    0x3E..=0x42 => 0x400_0000,
                    0x61 | 0x62 => 0x20_0000 + atk.skill_slot as u32,
                    _ => 0,
                };
                def.damage(env, opp_data, dmg, flags);
            }
        }
        if !atk.flags.test(0x98) {
            let gain = ki_gain(atk, data, env);
            atk.add_ki(gain);
        }
        atk.charge_gauge = (atk.charge_gauge + charge_gauge_gain(atk, data, env)).min(100_000);
        // BtlChar_AddStageTimer concerns stages 4 / 27 only.
        raise_first_clash(atk, def);
    }
}

fn split(chars: &mut [Chr; 2], a: usize) -> (&mut Chr, &mut Chr) {
    let (lo, hi) = chars.split_at_mut(1);
    if a == 0 {
        (&mut lo[0], &mut hi[0])
    } else {
        (&mut hi[0], &mut lo[0])
    }
}

/// `BtlHit_BeginFrame`: reaction back to 1; "handled" (0x60) drops when the motion's hit counter changes; a window
/// that ends unhandled raises held flag 0x5C.
fn begin_frame(chr: &mut Chr) {
    chr.react.reaction = 1;
    if chr.flags.test(crate::btl_chr::anim_flag::NEW) {
        chr.flags.clear(0x60);
    }
    let (index, count) = (i32::from(chr.obj.hit.index), i32::from(chr.obj.hit.count));
    if index >= 0 {
        if chr.status.last_hit_no != index {
            chr.flags.clear(0x60);
        }
    } else {
        if chr.status.last_hit_no >= 0
            && !chr.flags.test(0x60)
            && !chr.flags.test(crate::btl_chr::anim_flag::NEW)
            && index == !count
        {
            chr.flags.set_held(0x5C);
        }
        chr.flags.clear(0x60);
    }
    chr.status.last_hit_no = index;
}

/// `BtlHit_CheckNear`: attacker flags 0x4A / 0x4B raise contact (0x65) near the opponent's node 3.
fn check_near(a: &mut Chr, b: &Chr, data: &SideData) {
    if b.flags.test(0x42) {
        return;
    }
    let range = if a.flags.test(0x4A) {
        data.height() * 5.0
    } else if a.flags.test(0x4B) {
        data.height() * 2.0
    } else {
        return;
    };
    let range = range.clamp(20.0, 150.0);
    let d = crate::btl_chr::sub(a.obj.node_pos(3), b.obj.node_pos(3));
    if crate::btl_chr::length(d) < range {
        a.flags.set(0x65);
        if a.flags.test(0x4B) {
            a.flags.clear(0x4B);
        }
    }
}

/// `BtlHit_CheckGrab`: attacker flag 0x4C / 0x4D (the blinding skill of slot 0 / 1) catches the opponent within 100
/// of its node 3, unless it is untouchable (0x42), its form is immune (parameter bit 0x1000000 << form), it has flag
/// 0x137, or the attacker is behind it (lying down: with motion flag 0x8000): reaction 9 (0xE lying down), blinded
/// for the skill's frames, lock-on lost. The voice is not ported.
fn check_grab(a: &mut Chr, b: &mut Chr, data: &SideData, opp_data: &SideData, env: &Env) -> bool {
    if b.flags.test(0x42) {
        return false;
    }
    let slot = if a.flags.test(0x4D) {
        1
    } else if a.flags.test(0x4C) {
        0
    } else {
        return false;
    };
    a.flags.clear(0x4C);
    a.flags.clear(0x4D);
    // BtlHit_FormIndex: the costume twice, plus one for a damaged variant.
    let costume = b
        .members
        .get(b.active)
        .map_or(opp_data.costume, |m| m.costume);
    let form = costume * 2 + i32::from(b.gauge.variant != 0);
    let immune = u32::try_from(form)
        .ok()
        .and_then(|f| 0x100_0000u32.checked_shl(f))
        .is_some_and(|bit| opp_data.tables.params.flags() & bit != 0);
    if immune || b.flags.test(0x137) {
        return false;
    }
    if 100.0 < crate::btl_chr::length(crate::btl_chr::sub(a.obj.node_pos(3), b.obj.node_pos(3))) {
        return false;
    }
    let flags = env.anim_flags(b.anim.cur);
    let lying = flags & 0x800 != 0;
    let behind = if lying {
        flags & 0x8000 != 0
    } else {
        is_from_behind(a, b)
    };
    if behind {
        return false;
    }
    b.react.back = false;
    b.blind_timer = data.tables.skill().frames(slot);
    b.react.yaw = a.pose.yaw;
    b.react.reaction = if lying { 0xE } else { 9 };
    b.flags.clear(5);
    true
}

/// `BtlHit_CheckTouch`: attacker flag 0x4F raises contact when the bodies touch.
fn check_touch(a: &mut Chr, b: &Chr, data: &SideData, opp_data: &SideData) {
    if b.flags.test(0x42) || !a.flags.test(0x4F) {
        return;
    }
    let k = 1.2;
    let mut hit = crate::btl_chr::is_blocked_by_opponent(a, b, data, opp_data);
    let d = crate::btl_chr::sub(b.pose.pos, a.pose.pos);
    let dist = d[0].hypot(d[2]) - data.radius() * k - opp_data.radius() * k;
    if dist < 0.0 && -data.height() * k < d[1] && d[1] < opp_data.height() * k {
        hit = true;
    }
    if hit {
        a.flags.set(0x65);
        a.flags.clear(0x4F);
    }
}

/// `BtlHit_IsVoid`: attacker flag 0xBA, defender flag 0x42 (0x43 for a throw), a grab from behind.
fn is_void(atk: &Chr, data: &SideData, def: &Chr, env: &Env) -> bool {
    if atk.flags.test(0xBA) {
        return true;
    }
    let id = attack_id(atk, data, env);
    if def.flags.test(if id == 0x55 { 0x43 } else { 0x42 }) {
        return true;
    }
    if id == 0x56 {
        let flags = env.anim_flags(def.anim.cur);
        if (flags & 0x800 != 0 && flags & 0x8000 != 0) || is_from_behind(atk, def) {
            return true;
        }
    }
    false
}

/// `BtlHit_CheckDodge`: the automatic evasions of a free defender in a guardable motion.
fn check_dodge(atk: &Chr, def: &mut Chr, data: &SideData, env: &Env, frame: u32) -> bool {
    if !def.is_free() || def.flags.test(0x5A) {
        return false;
    }
    let anim_flags = env.anim_flags(def.anim.cur);
    let mut on = anim_flags & 0xB1 != 0;
    if anim_flags & 0x20_0000 != 0
        && (i32::from(def.obj.hit.index) ^ !i32::from(def.obj.hit.count) == 0
            || def.flags.test(0x60))
    {
        on = true;
    }
    if on {
        let flags = record(atk, data, env).0.flags();
        let (mut done, mut strike, mut counted) = (false, false, false);
        let window = def.dodge_window - i32::from(flags & 0x400_0000 != 0);
        let mut free = false;
        if window > 0 && flags & 1 != 0 && hit_no(atk) == 0 {
            def.flags.set(0x12C);
            done = true;
            free = true;
            // A paid evasion (attack flag 0x100000) needs 20000 ki: its attack (flag 0x7C) is not ported.
        }
        if !done {
            if def.dodges > 0 {
                def.dodges -= 1;
                done = true;
                counted = true;
            }
            if def.dodges_b > 0 {
                def.dodges_b -= 1;
                done = true;
                strike = true;
                counted = true;
            }
            if def.flags.test(0x56) {
                done = true;
                counted = true;
            }
            if def.flags.test(0x57) {
                done = true;
            }
        }
        if done {
            if strike {
                def.flags.set(0x77);
            } else {
                let mut dir = if flags & 0x80_0000 != 0 {
                    frame % 2
                } else {
                    frame % 3
                };
                if def.input.is_held(0x40) {
                    dir = 0;
                }
                if def.input.is_held(0x80) {
                    dir = 1;
                }
                if flags & 0x80_0000 == 0 && def.input.is_held(0x20) {
                    dir = 2;
                }
                def.flags.set([0x74, 0x75, 0x76][dir as usize]);
                if flags & 0x100_0000 != 0 {
                    def.flags.set(0x78);
                }
            }
            // A timed evasion fills the blast stocks by one with ability 0x21.
            if free && def.has_ability(0x21) {
                def.add_blast(100_000);
            }
            if counted {
                def.flags.set(if def.skill_slot == 0 { 0xDE } else { 0xDF });
            }
            return true;
        }
    }
    false
}

/// `BtlHit_CheckArmor`: a defender motion with flag 0x200 ignores attacks with flag 2.
fn check_armor(atk: &Chr, def: &Chr, data: &SideData, env: &Env) -> bool {
    env.anim_flags(def.anim.cur) & 0x200 != 0 && record(atk, data, env).0.flags() & 2 != 0
}

/// `BtlHit_CheckRepel`: defender flag 0x59 or 0x53 pushes the attacker back.
fn check_repel(atk: &mut Chr, def: &Chr) -> bool {
    if def.flags.test(0x59) || def.flags.test(0x53) {
        atk.flags.set(0x6F);
        return true;
    }
    false
}

/// `BtlHit_CheckGuard`: `Some(full)` when the hit was guarded (the guard effects applied; `full` = chip damage).
fn check_guard(atk: &mut Chr, def: &mut Chr, data: &SideData, env: &Env) -> Option<bool> {
    if !def.is_free() {
        return None;
    }
    let guard = guard_kind(def);
    let (rec, id) = record(atk, data, env);
    let rec = *rec;
    let ki_cost = guard_ki_cost(atk, data, env);
    if atk.obj.hit.index == 0 && env.anim_flags(def.anim.cur) & 0x31 != 0 && def.counter_window > 0
    {
        apply_guard(atk, def, &rec, 8, ki_cost);
        return Some(false);
    }
    if is_from_behind(atk, def) {
        return None;
    }
    if def.flags.test(0x40) && (attack_id(atk, data, env) as u32) < 0xB {
        apply_guard(atk, def, &rec, 1, ki_cost);
        return Some(true);
    }
    if def.flags.test(0x41) && rec.flags() & 0x10 != 0 && def.input.is_cmd_held(0x10) {
        let (kind, result) = if def.input.is_held(0x10) {
            (guard_result(&rec, id, def, 0x27), 2)
        } else if def.input.is_held(0x20) {
            (guard_result(&rec, id, def, 0x28), 3)
        } else {
            (guard_result(&rec, id, def, 0x29), 4)
        };
        if matches!(kind, 0 | 2) {
            apply_guard(atk, def, &rec, result, ki_cost);
            return Some(true);
        }
    }
    if guard == 0 {
        return None;
    }
    let res = match guard {
        1 => guard_result(&rec, id, def, 0x29),
        2 => guard_result(&rec, id, def, 0x27),
        3 => guard_result(&rec, id, def, 0x28),
        5 => rec.s8_at(0x2A),
        // BtlAtk_GetBestGuardKind mixes the kinds: guard kind 4 comes from a guard action, not ported.
        _ => 0,
    };
    if res == 3 {
        return None;
    }
    let result = match res {
        1 => 5,
        2 => 6,
        4 => 7,
        _ => 0,
    };
    apply_guard(atk, def, &rec, result, ki_cost);
    Some(true)
}

/// `BtlHit_ApplyGuard`: the effects of a guarded hit on both fighters (no health is touched).
fn apply_guard(atk: &mut Chr, def: &mut Chr, rec: &AttackRecord, result: u32, ki_cost: i32) {
    let flags = rec.flags();
    for (bit, fx) in [(0x1_0000, 0x25), (0x2_0000, 0x26), (0x4_0000, 0x27)] {
        if flags & bit != 0 {
            atk.set_fx(fx);
        }
    }
    atk.flags.clear(0x8A);
    atk.flags.set_held(0x5D);
    match result {
        1 => atk.set_fx(0x22),
        0 | 2..=4 => atk.set_fx(0x1C),
        5 => {
            atk.flags.set(0x6C);
            atk.flags.set_held(0x86);
            atk.charge_gauge = (atk.charge_gauge + 50_000).min(100_000);
        }
        6 => {
            atk.set_fx(0x21);
            atk.flags.set(0x6F);
        }
        7 | 8 => {
            atk.set_fx(if result == 7 { 0x24 } else { 0x21 });
            atk.flags.set(0x6F);
            atk.freeze_next = 2;
            atk.freeze_delay = 1;
        }
        _ => {}
    }
    let mut scale = 1.0;
    match result {
        2 => def.flags.set(0x68),
        3 => def.flags.set(0x69),
        4 => def.flags.set(0x6A),
        5 => {
            def.set_fx(0x20);
            def.flags.set(0x6D);
            def.spend_ki(ki_cost, false);
            if def.is_ki_empty() {
                def.flags.set_held(0xBE);
            }
            scale = 0.5;
        }
        6 => {
            def.flags.set(0x6E);
            def.add_blast(50_000);
            scale = 0.5;
        }
        7 | 8 => {
            def.flags.set(if result == 7 { 0x70 } else { 0x7B });
            def.freeze_next = 3;
            def.freeze_delay = 0;
        }
        _ => {}
    }
    let push = f32::from(rec.u8_at(0x13)) * 10.0 * kmh(1.0);
    def.set_impulse_yaw(atk.pose.yaw, push * scale);
    match rec.s8_at(0x2B) {
        0 => def.flags.set(0x66),
        1 => def.flags.set(0x67),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_kinds_follow_the_reaction_groups() {
        assert_eq!(voice_kind(3), 0);
        assert_eq!(voice_kind(42), 1);
        assert_eq!(voice_kind(17), 2);
        assert_eq!(voice_kind(18), 3);
        assert_eq!(voice_kind(25), 4);
    }
}
