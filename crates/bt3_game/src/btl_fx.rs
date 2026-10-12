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
    libc: &mut crate::rng::LibcRandom,
) {
    if data.tables.params.flags() & 0x10 != 0 && !has_any_listed_ability(me) {
        return;
    }
    let cmd = if me.test_fx(5) {
        me.obj.set_color_mode(6, false);
        [Some(2), None]
    } else {
        let mut cmds = [None, None];
        if me.fx_new(4) {
            me.obj.set_color_mode(6, true);
            cmds[0] = Some(0);
        }
        if me.fx_ended(4) {
            me.obj.set_color_mode(6, false);
            cmds[1] = Some(1);
        }
        cmds
    };
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
fn char_scale(data: &SideData) -> f32 {
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
/// 0xC at every model part its value names.
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
        }
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
