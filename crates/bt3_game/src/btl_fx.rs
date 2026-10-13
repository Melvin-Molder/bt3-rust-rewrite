//! The fighters' effect requests that start impacts (src/battle/btl_char_fx_1.c, btl_char_fx_2.c): the hit sparks
//! of animation events and effect bits, and the clash flash.

use crate::btl_chr::{Chr, Env, SideData};
use crate::btl_obj::mask_to_node;
use crate::btl_tech_volley::{mathf_cos, mathf_sin};
use crate::eft::{V3, add, dot, scale3};
use crate::eft_impact::{ImpactArg, Impacts};

/// `BtlMember_HasAnyListedAbility`: one of the aura colour abilities.
pub(crate) fn has_any_listed_ability(me: &Chr) -> bool {
    [
        0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x74,
    ]
    .into_iter()
    .any(|a| me.has_ability(a))
}

/// `BtlParam_GetAuraKind`: the parameter's +3, replaced by 0..10 by abilities 0x4D..0x53, 0x74, 0x54..0x56 (the
/// first held in that order).
pub(crate) fn aura_kind(me: &Chr, data: &SideData) -> i32 {
    const BY_ABILITY: [(i32, i32); 11] = [
        (0x4D, 0),
        (0x4E, 1),
        (0x4F, 2),
        (0x50, 3),
        (0x51, 4),
        (0x52, 5),
        (0x53, 6),
        (0x74, 7),
        (0x54, 8),
        (0x55, 9),
        (0x56, 10),
    ];
    BY_ABILITY
        .into_iter()
        .find(|&(a, _)| me.has_ability(a))
        .map_or(i32::from(data.tables.params.u8_at(3) as i8), |(_, k)| k)
}

/// `BtlCharApi_GetAuraType`: 3 with object flag 0x40000, else the aura kind.
pub(crate) fn aura_type(me: &Chr, data: &SideData) -> i32 {
    if me.obj.flags & 0x4_0000 != 0 {
        3
    } else {
        aura_kind(me, data)
    }
}

/// `BtlCharApi_GetActionFxKind`: 3 rising (0x17, 0xF3), 4 falling (0x18, 0xBB), 5 the techniques' sequences
/// (0x12D..0x12F), else 1.
pub(crate) fn action_fx_kind(me: &Chr) -> i32 {
    match me.act.current {
        0xBB | 0x18 => 4,
        0x17 | 0xF3 => 3,
        0x12D..=0x12F => 5,
        _ => 1,
    }
}

/// What the aura reads of a fighter.
pub(crate) fn aura_chr<'a>(
    me: &Chr,
    data: &SideData,
    node_pos: &'a dyn Fn(u32) -> V3,
) -> crate::eft_aura::AuraChr<'a> {
    crate::eft_aura::AuraChr {
        side: me.player,
        height: data.height(),
        aura_type: aura_type(me, data),
        dir: me.pose.dir,
        vel: me.pose.vel,
        speed: me.pose.speed,
        fall_speed: me.pose.fall_speed,
        fx_kind: action_fx_kind(me),
        node_pos,
    }
}

/// `BtlFx_UpdateAura`: the aura's level is the ki ratio when at least half full (none with parameter flag 0x10
/// and no aura colour ability; full with object flag 0x40000 or request 0x13; request 0x14 kills it); with a level
/// it is asked for every frame, else stopped. Without parameter flag 0x10, request 7's start and end burst it and
/// end the burst, request 8 its peak.
pub(crate) fn update_aura(
    me: &Chr,
    data: &SideData,
    env: &Env,
    auras: &mut crate::eft_aura::Auras,
) {
    let Some(cfg) = env.aura else {
        return;
    };
    let node_pos = |n| me.obj.node_pos(n);
    let c = aura_chr(me, data, &node_pos);
    let g = &me.gauge;
    let mut level = if g.ki_max > 0 {
        let l = g.ki as f32 / g.ki_max as f32;
        if l < 0.5 { 0.0 } else { l }
    } else {
        0.0
    };
    let flag10 = data.tables.params.flags() & 0x10 != 0;
    if flag10 && !has_any_listed_ability(me) {
        level = 0.0;
    }
    if me.obj.flags & 0x4_0000 != 0 || me.test_fx(0x13) {
        level = 1.0;
    }
    if me.test_fx(0x14) {
        level = 0.0;
        auras.command(cfg, &c, 7);
    }
    if level > 0.0 {
        auras.command(cfg, &c, 0);
        auras.set_level(me.player, level);
    } else {
        auras.command(cfg, &c, 1);
    }
    if !flag10 {
        if me.fx_new(7) {
            auras.command(cfg, &c, 2);
        }
        if me.fx_ended(7) {
            auras.command(cfg, &c, 3);
        }
        if me.test_fx(8) {
            auras.command(cfg, &c, 4);
        }
    }
}

/// `BtlFx_UpdateStageFxRequest`: request 0x10 raises 0x12 (in a technique only when its flag word A has bit 31
/// clear); request 0x11 raises it once the animation's attribute 0x200 was seen during it (flag 0x12E, dropped
/// when 0x11 ends).
pub(crate) fn stage_fx_request(me: &mut Chr, data: &SideData) {
    if me.test_fx(0x10) {
        let cur = me.act.current;
        let class = crate::btl_act::id_class(cur);
        if !crate::btl_act::is_technique_id(cur) || data.tables.tech().flags_a(class) as i32 >= 0 {
            me.set_fx(0x12);
        }
    }
    if me.test_fx(0x11) {
        if me.anim_test_attr(0x200) {
            me.flags.set_held(0x12E);
        }
        if me.flags.test(0x12E) {
            me.set_fx(0x12);
        }
    } else {
        me.flags.clear(0x12E);
    }
}

/// `BtlFx_UpdateScreenFilter`: request 9 raises the fighter's filter by a sixth a frame (and aims its view's blur
/// along its facing), 0xA sets it full, else it falls by a third; its view's blur is that strong (alphas 8, 32,
/// 128 at full), three passes.
pub(crate) fn screen_filter(me: &mut Chr, blurs: &mut crate::stg_blur::Blurs) {
    if me.test_fx(9) {
        me.filter = (me.filter + 0.166_666_68).min(1.0);
    } else if me.test_fx(0xA) {
        me.filter = 1.0;
    } else {
        me.filter = (me.filter - 0.333_333_35).max(0.0);
    }
    let a = (me.filter * 8.0) as u8;
    let b = (me.filter * 32.0) as u8;
    let c = (me.filter * 128.0) as u8;
    let view = me.player;
    blurs.set_passes(view, 3);
    blurs.set_color(view, 3, [0x80, 0x80, 0x80, 0x40]);
    blurs.set_color(view, 0, [0x80, 0x80, 0x80, a]);
    blurs.set_color(view, 1, [0x80, 0x80, 0x80, b]);
    blurs.set_color(view, 2, [0x80, 0x80, 0x80, c]);
    if me.test_fx(9) {
        blurs.set_center(view, me.pose.dir, false);
    }
}

/// `BtlFx_TriggerObjReq1D`: request 0x1D flashes the model yellow.
pub(crate) fn trigger_obj_1d(me: &mut Chr) {
    if me.test_fx(0x1D) {
        me.obj.flash.start(0);
    }
}

/// `BtlFx_TriggerObjReq1E` / `1F`: while held, the model flashes white (strong white) every fourth frame.
pub(crate) fn trigger_obj_1e_1f(me: &mut Chr) {
    if me.test_fx(0x1E) {
        if me.trigger_1e == 0 {
            me.obj.flash.start(1);
        }
        me.trigger_1e = (me.trigger_1e + 1) % 4;
    } else {
        me.trigger_1e = 0;
    }
    if me.test_fx(0x1F) {
        if me.trigger_1f == 0 {
            me.obj.flash.start(2);
        }
        me.trigger_1f = (me.trigger_1f + 1) % 4;
    } else {
        me.trigger_1f = 0;
    }
}

/// `BtlFx_SetObjMaskOnEvent400`: animation event 0x400 in skill 0x18 turns colour mode 2 on.
pub(crate) fn obj_mask_event_400(me: &mut Chr, data: &SideData) {
    let cur = me.act.current;
    if (0xFD..0x103).contains(&cur)
        && data.tables.skill().id(crate::btl_act::id_class(cur)) == 0x18
        && me.anim_test_attr(0x400)
    {
        me.obj.set_color_mode(2, true);
    }
}

/// `BtlFx_UpdateObjCmdFlag137`: fighter flag 0x137 forces eye frame 1 (the tired idle's shut eyes); once it
/// clears, the eyes go back to the blink.
pub(crate) fn update_eyes_flag137(me: &mut Chr) {
    if me.flags.test(0x137) {
        me.obj.set_eye_frame(1);
    } else if me.obj.eye_frame() == 1 {
        me.obj.set_eye_frame(9);
    }
}

/// `BtlFx_UpdateSubStateFlag96`: fighter flag 0x96 (the low-health idle) holds mouth shape 1 (mode 5) from rest or
/// a single talk; clearing it returns the mouth to rest.
pub(crate) fn update_mouth_flag96(me: &mut Chr) {
    let mode = me.obj.mouth_mode();
    if me.flags.test(0x96) {
        if mode == 0 || mode == 4 {
            me.obj.set_mouth(5);
        }
    } else if mode == 5 {
        me.obj.set_mouth(0);
    }
}

/// `BtlFxObj_HandleSubStateEvents` (`BtlFx_UpdateObjEvents`, the motion pass): motion events 1 << 56..58 hold
/// mouth shapes 5..7 (modes 9..11), 1 << 59 forces eye frame 2, 1 << 60 ends a held shape and the forced eyes.
pub(crate) fn face_events(me: &mut Chr) {
    obj_face_events(&mut me.obj);
}

/// `BtlFxObj_HandleSubStateEvents` of any object (the partner's too).
pub(crate) fn obj_face_events(obj: &mut crate::btl_obj::Obj) {
    for (bit, mode) in [(56, 9), (57, 0xA), (58, 0xB)] {
        if obj.test_event(1 << bit) {
            obj.set_mouth(mode);
        }
    }
    if obj.test_event(1 << 59) {
        obj.set_eye_frame(2);
    }
    if obj.test_event(1 << 60) {
        if (9..=12).contains(&obj.mouth_mode()) {
            obj.set_mouth(0);
        }
        obj.set_eye_frame(9);
    }
}

/// `BtlFx_ShakeCamOnEvent` and `BtlFx_VibrateOnEvent`: motion event bit 46 shakes the fighter's camera (3.0 for
/// 0.3 s), bit 45 vibrates its pad (1.0 for 0.2 s).
pub(crate) fn shake_and_vibrate_on_events(me: &mut Chr) {
    if me.anim_test_attr(0x4000_0000_0000) {
        crate::chr_cam::add_shake(me, 3.0, 0.3);
    }
    if me.anim_test_attr(0x2000_0000_0000) {
        me.vibrate(1.0, 0.2);
    }
}

/// `BtlFx_PushBackOnEvent400`: in a rush sequence of a technique without flag 0x1000000, animation event 0x400
/// raises flag 0x9F, while which the chains are pushed straight back (the partner object's too).
pub(crate) fn push_back_on_event_400(me: &mut Chr, data: &SideData) {
    let cur = me.act.current;
    if crate::btl_act::is_technique_id(cur)
        && crate::btl_tech_fx::in_rush_sequence(me)
        && data.tables.tech().flags_a(crate::btl_act::id_class(cur)) & 0x100_0000 == 0
    {
        if me.anim_test_attr(0x400) {
            me.flags.set_held(0x9F);
        }
        if me.flags.test(0x9F) {
            let a = crate::btl_input::wrap_angle(me.pose.rot[1] + std::f32::consts::PI);
            let v = [
                crate::btl_tech_volley::mathf_sin(a),
                0.0,
                crate::btl_tech_volley::mathf_cos(a),
            ];
            me.obj.chains.add_push(v, 5.0);
            // BtlPartner_PushAngle.
            if let Some(p) = me.partner.as_deref_mut() {
                p.obj.chains.add_push(v, 5.0);
            }
        }
    }
}

/// `BtlFx_UpdateObjFlag80`: with parameter flag 0x200000, ability 0x79 shows model part 4 (object flag 0x80, the
/// geometry on node 0x6F); without either it is cleared, but only while the object is drawn (flag 2).
pub(crate) fn update_obj_flag80(me: &mut Chr, data: &SideData) {
    let on = data.tables.params.flags() & 0x20_0000 != 0 && me.has_ability(0x79);
    let flags = &mut me.obj.flags;
    if on {
        if *flags & 0x82 != 0x82 {
            *flags |= 0x80;
        }
    } else if *flags & 0x82 == 0x82 {
        *flags &= !0x80;
    }
}

/// What the power-up glow reads of a fighter.
pub(crate) fn glow_chr<'a>(
    me: &Chr,
    data: &SideData,
    node_pos: &'a dyn Fn(u32) -> V3,
) -> crate::eft_glow::GlowChr<'a> {
    crate::eft_glow::GlowChr {
        side: me.player,
        height: data.height(),
        aura_type: aura_type(me, data),
        param_flags: data.tables.params.chara_flags(),
        fx_kind: action_fx_kind(me),
        dir: me.pose.dir,
        rot: me.pose.rot,
        fall_speed: me.pose.fall_speed,
        moved: me.pose.moved,
        node_pos,
    }
}

/// `BtlFx_UpdatePowerUpLook` (without parameter flag 0x10, or with an aura colour ability): request 5 removes
/// the glow, request 4 starts it (object colour mode 6 on) and its end fades it (mode 6 off).
pub(crate) fn power_up_look(
    me: &mut Chr,
    data: &SideData,
    env: &Env,
    glows: &mut crate::eft_glow::Glows,
    auras: &mut crate::eft_aura::Auras,
    libc: &mut crate::rng::LibcRandom,
) {
    if data.tables.params.flags() & 0x10 != 0 && !has_any_listed_ability(me) {
        return;
    }
    // The aura fades out while the glow runs (5 in, 6 out).
    let mut aura_cmds = [None, None];
    let cmd = if me.test_fx(5) {
        me.obj.set_color_mode(6, false);
        aura_cmds[0] = Some(5);
        [Some(2), None]
    } else {
        let mut cmds = [None, None];
        if me.fx_new(4) {
            me.obj.set_color_mode(6, true);
            aura_cmds[0] = Some(6);
            cmds[0] = Some(0);
        }
        if me.fx_ended(4) {
            me.obj.set_color_mode(6, false);
            aura_cmds[1] = Some(5);
            cmds[1] = Some(1);
        }
        cmds
    };
    if let Some(cfg) = env.aura {
        let node_pos = |n| me.obj.node_pos(n);
        let c = aura_chr(me, data, &node_pos);
        for a in aura_cmds.into_iter().flatten() {
            auras.command(cfg, &c, a);
        }
    }
    let Some(cfg) = env.glow else {
        return;
    };
    for c in cmd.into_iter().flatten() {
        let node_pos = |n| me.obj.node_pos(n);
        let gc = glow_chr(me, data, &node_pos);
        glows.request(cfg, &gc, c, libc);
    }
}

/// `BtlScene_GetCharScale`.
pub(crate) fn char_scale(data: &SideData) -> f32 {
    data.height() / 19.35
}

/// The direction the model faces (its yaw `rot.y`).
fn facing(me: &Chr) -> V3 {
    let y = me.pose.rot[1];
    [mathf_sin(y), 0.0, mathf_cos(y)]
}

fn spark(impacts: &mut Impacts, env: &Env, data: &SideData, me: &Chr, id: i32, pos: V3, dir: V3) {
    let arg = ImpactArg {
        pos,
        dir,
        id,
        chr: me.player,
    };
    impacts.spawn_hit(env.impact, &arg, char_scale(data));
}

/// The sparks of `BtlFx_UpdateAll`, in its order. `BtlFx_HandleImpactEvents`: a light (0x100) or heavy (0x80)
/// impact event at its model part while the model is drawn (not with event 0x8000000): kinds 0 / 3 in actions
/// without a class, 0xF / 0x10 in the others, 9 / 0xA for a shown weapon outside a rush sequence.
/// `BtlFx_SpawnFlashReq2B`: request 0x2B, kind 0xD at node 0x10. `BtlFx_SpawnPartFlashes`: event 0x1000000, kind
/// 0xC at every model part its value names, and common sound 0x4B.
pub(crate) fn update_all(me: &Chr, data: &SideData, env: &Env, impacts: &mut Impacts) {
    let classless = crate::btl_act::id_class(me.act.current) < 0;
    let mut event = None;
    if me.anim_test_attr(0x100) {
        event = Some((if classless { 0 } else { 0xF }, 9, 0x100));
    }
    if me.anim_test_attr(0x80) {
        event = Some((if classless { 3 } else { 0x10 }, 0xA, 0x80));
    }
    if let Some((mut id, weapon, mask)) = event {
        if !crate::btl_tech_fx::in_rush_sequence(me) && crate::btl_snd::has_weapon_out(me, data) {
            id = weapon;
        }
        if me.obj.flags & 2 != 0 && !me.anim_test_attr(0x800_0000) {
            let node = mask_to_node(me.obj.event_arg(mask)) as u32;
            spark(
                impacts,
                env,
                data,
                me,
                id,
                me.obj.node_pos(node),
                facing(me),
            );
        }
    }
    if me.test_fx(0x2B) {
        spark(
            impacts,
            env,
            data,
            me,
            0xD,
            me.obj.node_pos(0x10),
            me.pose.dir,
        );
    }
    if me.anim_test_attr(0x100_0000) {
        let bits = me.obj.event_arg(0x100_0000);
        if bits != 0 {
            let dir = facing(me);
            for i in 0..32 {
                let bit = bits & (1 << i);
                if bit != 0 {
                    let pos = me.obj.node_pos(mask_to_node(bit) as u32);
                    spark(impacts, env, data, me, 0xC, pos, dir);
                }
            }
            me.play_common(&mut env.snd(), 0x4B);
        }
    }
}

/// `BtlFx_UpdateObjFlagReq3`: request 3 (the change's flash) sets the object's colour flag to the black mode
/// (0x80000) as it starts and to the stage mode (0x10000) as it ends, past the colour-mode requesters.
pub(crate) fn update_obj_flag_req3(me: &mut Chr) {
    if me.fx_new(3) {
        me.obj.flags = (me.obj.flags & !0xFF_0000) | 0x8_0000;
    }
    if me.fx_ended(3) {
        me.obj.flags = (me.obj.flags & !0xFF_0000) | 0x1_0000;
    }
}

/// `BtlFx_HandleEvent1`: an attack with flag 0x80000 raises the landed blow's spark (request 0x1B) on its motion's
/// first hit event (the frame's event 1, with one of the motion's hit events reached), whether it hits or not.
pub(crate) fn handle_event1(me: &mut Chr, data: &SideData, env: &Env) {
    if me.anim_test_attr(1)
        && crate::btl_hit::attack_id(me, data, env) != -1
        && crate::btl_hit::attack_flags(me, data, env) & 0x80000 != 0
        && me.obj.query_event(&data.obj, 1, 0, 5) == 1
    {
        me.set_fx(0x1B);
    }
}

/// The sparks of `BtlFx_UpdateAfterHits`. `BtlChar_SpawnHitFx`: a landed blow (0x1B) by the attack's hit effect
/// kind (9 / 0xA for a shown weapon unless attack flag 0x2000), a guarded one (0x1C), 0x21 and 0x24 kind 6, 0x22
/// kind 4 or 5; launched along the attack's angles from the opponent's side (0x25, 0x26 half a height up, 0x27 a
/// height) or at the first striking part the hit window names, kept at least 5 in front of the fighter.
/// `BtlFx_SpawnFlashReq20`: request 0x20, kind 0xE at node 0x2E. `BtlFx_SpawnClashFlash`: requests 0x2C / 0x2D,
/// kinds 6 and 0xD of size 3 / 6 halfway to the opponent; returns where the big one shakes the cameras.
pub(crate) fn after_hits(
    me: &Chr,
    opp: &Chr,
    data: &SideData,
    opp_data: &SideData,
    env: &Env,
    impacts: &mut Impacts,
) -> Option<V3> {
    let mut id = -1;
    if me.test_fx(0x1B) {
        id = match crate::btl_hit::hit_fx_kind(me, data, env) {
            1 => 0,
            2 | 3 | 5 => 2,
            4 => 1,
            6 => 3,
            7 => 0xC,
            8 => 7,
            9 => 8,
            10 => 0xD,
            _ => -1,
        };
        if crate::btl_snd::has_weapon_out(me, data)
            && crate::btl_hit::attack_flags(me, data, env) & 0x2000 == 0
        {
            id = if id < 3 { 9 } else { 10 };
        }
    }
    if me.test_fx(0x1C) || me.test_fx(0x21) || me.test_fx(0x24) {
        id = 6;
    }
    if me.test_fx(0x22) {
        id = if crate::btl_hit::hit_fx_kind(me, data, env) < 4 {
            4
        } else {
            5
        };
    }
    if id >= 0 {
        let pose = &me.pose;
        let yaw =
            crate::btl_input::wrap_angle(pose.yaw + crate::btl_hit::launch_angle_a(me, data, env));
        let pitch = crate::btl_hit::launch_angle_b(me, data, env);
        let (sy, cy, sp, cp) = (
            mathf_sin(yaw),
            mathf_cos(yaw),
            mathf_sin(pitch),
            mathf_cos(pitch),
        );
        let dir = [cp * sy, -sp, cp * cy];
        let beside = |drop: f32| {
            let target = opp.snap_pos(6);
            let r = opp_data.radius();
            [
                target[0] - mathf_sin(pose.yaw) * r,
                pose.pos[1] - drop,
                target[2] - mathf_cos(pose.yaw) * r,
            ]
        };
        let pos = if me.test_fx(0x25) {
            Some(beside(0.0))
        } else if me.test_fx(0x26) {
            Some(beside(data.height() * 0.5))
        } else if me.test_fx(0x27) {
            Some(beside(data.height()))
        } else {
            striking_part(me, data)
        };
        if let Some(pos) = pos {
            spark(impacts, env, data, me, id, pos, dir);
        }
    }
    if me.test_fx(0x20) {
        spark(
            impacts,
            env,
            data,
            me,
            0xE,
            me.obj.node_pos(0x2E),
            me.pose.dir,
        );
    }
    let mut size = -1.0;
    let mut shake = false;
    if me.test_fx(0x2C) {
        size = 3.0;
    }
    if me.test_fx(0x2D) {
        size = 6.0;
        shake = true;
    }
    if size > 0.0 {
        let (a, b) = (me.obj.node_pos(3), opp.obj.node_pos(3));
        let pos = add(a, scale3(crate::eft::sub(b, a), 0.5));
        for id in [6, 0xD] {
            let arg = ImpactArg {
                pos,
                dir: me.pose.dir,
                id,
                chr: me.player,
            };
            impacts.spawn_hit_scaled(env.impact, &arg, size);
        }
        if shake {
            return Some(pos);
        }
    }
    None
}

/// The striking part of `BtlChar_SpawnHitFx`: of the hit window's nodes (or the motion's first window's), the
/// first of a preferred order, else the lowest; pushed out to the plane 5 in front of the fighter.
fn striking_part(me: &Chr, data: &SideData) -> Option<V3> {
    const ORDER: [u32; 9] = [0x4_0000, 0x800, 0x100, 0x400, 0x80, 0x20, 8, 0x10, 4];
    let mut bits = me.obj.hit.node_mask;
    if bits == 0 {
        bits = me.obj.query_event(&data.obj, 1, 0, 6) as u32;
    }
    let hit = ORDER
        .iter()
        .map(|m| bits & m)
        .find(|&h| h != 0)
        .or_else(|| (0..0x13).map(|i| bits & (1 << i)).find(|&h| h != 0))?;
    let mut pos = me.obj.node_pos(mask_to_node(hit) as u32);
    let fwd = [mathf_sin(me.pose.rot[1]), 0.0, mathf_cos(me.pose.rot[1])];
    let point = add(me.pose.pos, scale3(fwd, 5.0));
    let d = dot(fwd, pos) - dot(fwd, point);
    if d < 0.0 {
        pos = add(pos, scale3(fwd, -d));
    }
    Some(pos)
}

/// `BtlFx_SpawnFlashReq2E` (after the effect scene): request 0x2E, kind 0xB at node 0x11 along the deflection.
pub(crate) fn post_scene(me: &Chr, data: &SideData, env: &Env, impacts: &mut Impacts) {
    if me.test_fx(0x2E) {
        spark(
            impacts,
            env,
            data,
            me,
            0xB,
            me.obj.node_pos(0x11),
            me.deflect_dir,
        );
    }
}
