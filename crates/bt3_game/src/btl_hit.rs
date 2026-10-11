//! Fighter hit resolution (src/battle/btl_char_hit.c, the strike test of eft_detect.c, the attack-record
//! accessors of btl_param.c): does a strike connect, is it avoided or guarded, which reaction it gives the
//! defender and how much health it takes.
//!
//! A hit is the attacker's current attack id (`BtlAtk_GetId`), the 0x30-byte record it selects in the character's
//! attack table, the strike volumes of the motion's open hit window touching the defender's body boxes, and the
//! reaction block `BtlHit_ApplyHit` fills on the defender. Not ported: throws and rushing techniques (flags 0x49,
//! 0x4C / 0x4D come only from actions that are not ported), camera shake and pad vibration.

use std::f32::consts::PI;

use bt3_data::chara_data::{ATTACK_COUNT, AttackRecord};

use crate::btl_chr::{Chr, Env, SideData, kmh, opp_yaw, opp_yaw_from_facing};
use crate::btl_input::wrap_angle;
use crate::btl_obj::body_hit_test;

/// `BtlAtk_GetId`: the attack record the fighter's current action / motion uses, -1 when it is not attacking.
/// The attack actions 0x70.. take theirs from the roster's table; the other cases are the original's, of which
/// only the rush steps can be reached so far.
pub(crate) fn attack_id(me: &Chr, env: &Env) -> i32 {
    let action = me.act.current;
    if (0x70..0x70 + 0x3E).contains(&action) {
        return env
            .common
            .attack_actions
            .get((action - 0x70) as usize)
            .map_or(-1, |row| i32::from(row[3]));
    }
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
        _ => -1,
    }
}

/// `BtlAtk_GetRecord`: ids outside 0..0xA2 give record 0.
fn record<'a>(me: &Chr, data: &'a SideData, env: &Env) -> (&'a AttackRecord, usize) {
    let id = attack_id(me, env);
    let n = if (0..ATTACK_COUNT as i32).contains(&id) {
        id as usize
    } else {
        0
    };
    (&data.tables.attacks[n], n)
}

/// `BtlAtk_GetLaunchSpeed`: record +0x14 in 10 km/h, times curve row 8.
pub(crate) fn launch_speed(me: &Chr, data: &SideData, env: &Env) -> f32 {
    let (r, _) = record(me, data, env);
    f32::from(r.u8_at(0x14)) * 10.0 * kmh(1.0) * env.curve_base(8)
}

/// The number of hit windows of the motion (`BtlObjAnim_QueryEvent(obj, 1, 0, 3)`).
fn hit_windows(me: &Chr, data: &SideData) -> i32 {
    me.obj.query_event(&data.obj, 1, 0, 3)
}

/// `BtlAtk_GetDamage`: record damage times curve row 4, divided by the motion's hits.
fn damage(me: &Chr, data: &SideData, env: &Env) -> i32 {
    let (r, _) = record(me, data, env);
    let mut v = (i32::from(r.u16_at(4)) as f32 * env.curve_base(4)) as i32;
    let hits = hit_windows(me, data);
    if hits >= 2 {
        v /= hits;
    }
    v
}

/// `BtlAtk_GetKiGain`.
fn ki_gain(me: &Chr, data: &SideData, env: &Env) -> i32 {
    let (r, _) = record(me, data, env);
    let mut v = i32::from(r.u16_at(0xA));
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

/// `BtlHit_GetVoiceKind`.
pub(crate) fn voice_kind(reaction: i32) -> i32 {
    match reaction {
        5..=10 | 29 | 30 | 35 | 36 | 42 | 47 | 48 => 1,
        15..=17 | 19 => 2,
        18 | 40 | 41 => 3,
        12 | 13 | 23..=28 | 38 | 39 => 4,
        _ => 0,
    }
}

/// `BtlColl_GetGuardKind`: the guard motions 0xEC..0xF7.
fn guard_kind(me: &Chr) -> i32 {
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
        if self.check_clash() {
            return;
        }
        // BtlHit_CheckRush: flag 0x49 comes only from the rushing techniques, which are not ported.
        for a in 0..2 {
            let (data, opp_data) = (self.data[a], self.data[a ^ 1]);
            let (me, opp) = split(self.chars, a);
            check_near(me, opp, data);
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
        if me.flags.test(0x60) || attack_id(me, env) == -1 {
            return;
        }
        if me.obj.work.hit_flags & 0xF00_0000 != 0 || me.flags.test(0x65) {
            let flags = record(me, data, env).0.flags();
            let ok = opp_yaw_from_facing(me, opp).abs() < 1.570_796_3 || flags & 8 != 0;
            if ok {
                me.flags.set_held(0x60);
                if is_void(me, opp, env)
                    || check_dodge(me, opp, data, env, frame)
                    || check_armor(me, opp, data, env)
                    || check_repel(me, opp)
                {
                    me.flags.set_held(0x5C);
                } else if let Some(full) = check_guard(me, opp, data, env) {
                    if full {
                        let mut guard = (i32::from(record(me, data, env).0.u16_at(6)) as f32
                            * env.curve_base(4)) as i32;
                        let hits = hit_windows(me, data);
                        if hits >= 2 {
                            guard /= hits;
                        }
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
            if def.skill_timer_c > 0 {
                armor += 1;
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
                def.react.back = (atk_flags >> 12) & 1 != 0;
                def.react.yaw = yaw;
                def.react.launch_a = angle(0x15);
                def.react.launch_b = angle(0x16);
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
        // BtlChar_RaiseFirstClash; BtlChar_AddStageTimer concerns stages 4 / 27 only.
        if !atk.first_hit && !def.first_hit {
            atk.first_hit = true;
            atk.flags.set(0xE3);
        }
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
fn is_void(atk: &Chr, def: &Chr, env: &Env) -> bool {
    if atk.flags.test(0xBA) {
        return true;
    }
    let id = attack_id(atk, env);
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
        if window > 0 && flags & 1 != 0 && hit_no(atk) == 0 {
            def.flags.set(0x12C);
            done = true;
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
    let rec = *record(atk, data, env).0;
    if atk.obj.hit.index == 0 && env.anim_flags(def.anim.cur) & 0x31 != 0 && def.counter_window > 0
    {
        apply_guard(atk, def, &rec, 8);
        return Some(false);
    }
    if is_from_behind(atk, def) {
        return None;
    }
    if def.flags.test(0x40) && (attack_id(atk, env) as u32) < 0xB {
        apply_guard(atk, def, &rec, 1);
        return Some(true);
    }
    if def.flags.test(0x41) && rec.flags() & 0x10 != 0 && def.input.is_cmd_held(0x10) {
        let (kind, result) = if def.input.is_held(0x10) {
            (rec.s8_at(0x27), 2)
        } else if def.input.is_held(0x20) {
            (rec.s8_at(0x28), 3)
        } else {
            (rec.s8_at(0x29), 4)
        };
        if matches!(kind, 0 | 2) {
            apply_guard(atk, def, &rec, result);
            return Some(true);
        }
    }
    if guard == 0 {
        return None;
    }
    let res = match guard {
        1 => rec.s8_at(0x29),
        2 => rec.s8_at(0x27),
        3 => rec.s8_at(0x28),
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
    apply_guard(atk, def, &rec, result);
    Some(true)
}

/// `BtlHit_ApplyGuard`: the effects of a guarded hit on both fighters (no health is touched).
fn apply_guard(atk: &mut Chr, def: &mut Chr, rec: &AttackRecord, result: u32) {
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
            // BtlAtk_GetGuardKiCost: record +8 times curve row 5 (1 at level 0).
            def.spend_ki(i32::from(rec.u16_at(8)), false);
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
