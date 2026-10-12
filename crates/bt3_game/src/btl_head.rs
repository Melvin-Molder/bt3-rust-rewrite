//! Head tracking (`BtlChar_*Look*`, `BtlChar_UpdateHead`, src/battle/btl_head_tracking.c): a fighter turns its head
//! (node 0x2E) and neck (node 0x2F) towards the other fighter's head. The look offset eases a fifth a frame towards
//! the head's offset from the body; the direction (the opponent's target position less the own, plus the own
//! offset, less the opponent's) eases 30 % a frame into the look direction. Taken into the head's frame (turned by
//! the neck pitch), tracking starts when the target is within 1.5 of yaw and 0.6 of pitch and less than 90 degrees
//! off the heading, and keeps within 1.8 / 1; the direction is then clamped to the character's limits (parameter
//! bytes +0x350..+0x352) and half the rotation from forward goes to both nodes, whose weights ease 30 % a frame
//! towards 1 while tracking with flag 5 and the look enabled (by the guard, neutral and move actions each frame),
//! towards 0 otherwise. Character 0x99 uses plain yaw / pitch clamps. Parameter flag 0x20 turns it off. The limits
//! are the five bytes after the chain parameters of character file section 0x17 (`obj->param` is that section).

use crate::battle::{Mtx, apply, inverse_rt, mul};
use crate::btl_chr::{Chr, SideData};
use crate::btl_obj_chain::{q_axis_angle, q_from_vectors, rotate_axis};

/// `BtlCtlLook`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Look {
    /// Eased towards the head's offset from the body.
    pub offset: [f32; 3],
    pub dir: [f32; 3],
    pub height: f32,
    /// Set by the actions that look, every frame (cleared as the frame begins).
    pub enabled: bool,
    pub tracking: bool,
}

const PI_F: f32 = 3.141_592_7;

fn deg(d: f32) -> f32 {
    d * PI_F / 180.0
}

fn norm4(v: [f32; 4]) -> [f32; 4] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l == 0.0 {
        [0.0, 0.0, 0.0, v[3]]
    } else {
        [v[0] / l, v[1] / l, v[2] / l, v[3]]
    }
}

/// `BtlChar_ResetLook`: no extra rotation, weights 0, the height offset at 0.7 times the height.
pub(crate) fn reset_look(me: &mut Chr, data: &SideData) {
    me.obj.head_rot = [0.0, 0.0, 0.0, 1.0];
    me.obj.neck_rot = [0.0, 0.0, 0.0, 1.0];
    me.obj.head_weight = 0.0;
    me.obj.neck_weight = 0.0;
    me.look.height = data.height() * 0.7;
}

/// `BtlChar_UpdateLookOffset` (the end of the motion pass): the offset from node 0x2F to the object's position
/// eased a fifth a frame; reset while time is stopped.
pub(crate) fn update_look_offset(me: &mut Chr, data: &SideData, time_stopped: bool) {
    if time_stopped {
        reset_look(me, data);
        return;
    }
    let Some(node) = me.obj.node(0x2F) else {
        return;
    };
    let w = [node.mtx[3][0], node.mtx[3][1], node.mtx[3][2]];
    let out = me.obj.root_pos;
    let look = &mut me.look;
    look.height += (out[1] - w[1] - look.height) * 0.2;
    for i in 0..3 {
        let offset = out[i] - w[i];
        look.offset[i] += (offset - look.offset[i]) * 0.2;
    }
}

/// `BtlChar_UpdateHead`: `BtlChar_UpdateLook`, or `BtlChar_UpdateLookAlt` for character 0x99; nothing with
/// parameter flag 0x20.
pub(crate) fn update_head(me: &mut Chr, opp: &Chr, data: &SideData) {
    if data.tables.params.flags() & 0x20 != 0 {
        return;
    }
    let chara = me.members.get(me.active).map_or(-1, |m| m.chara);
    update_look(me, opp, data, chara == 0x99);
}

fn update_look(me: &mut Chr, opp: &Chr, data: &SideData, alt: bool) {
    let lim = data.tables.chain.as_ref();
    let byte = |f: fn(&bt3_data::chara_data::ChainParams) -> f32| lim.map_or(0.0, f);
    let neck_pitch = deg(byte(|c| f32::from(c.neck_pitch)));
    let head_pitch = deg(byte(|c| f32::from(c.head_pitch)));
    let (mut full, mut active) = (true, true);
    if !me.flags.test(5) {
        (full, active) = (false, false);
    }
    if !me.look.enabled {
        (full, active) = (false, true);
    }
    if me.flags.test(0xAC) {
        (full, active) = (false, false);
    }
    let (Some(_), Some(_)) = (me.obj.node(0x2E), me.obj.node(0x2F)) else {
        return;
    };
    let head_parent = me.obj.node(0x2E).map(|n| n.parent).unwrap_or_default();
    let m: Mtx = if alt {
        head_parent
    } else {
        mul(
            &crate::eft_ptcl::rot_x(&crate::battle::IDENTITY, neck_pitch),
            &head_parent,
        )
    };
    let inv = inverse_rt(&m);
    let delta = crate::btl_chr::opp_delta(me, opp);
    let target = norm4([
        delta[0] + me.look.offset[0] - opp.look.offset[0],
        delta[1] + me.look.offset[1] - opp.look.offset[1],
        delta[2] + me.look.offset[2] - opp.look.offset[2],
        0.0,
    ]);
    // Vec4_Lerp(dir, target, dir, 0.3) (look.snap is never set).
    for i in 0..3 {
        me.look.dir[i] = target[i] * 0.3 + me.look.dir[i] * 0.7;
    }
    let mut blend = 0.0f32;
    if active {
        let d = me.look.dir;
        let mut local = norm4(apply(&inv, [d[0], d[1], d[2], 0.0]));
        let (up_max, down_max, yaw_max) = match (alt, me.look.tracking) {
            (false, true) => (1.0, -1.0, 1.8),
            (false, false) => (0.6, -0.6, 1.5),
            (true, true) => (1.3, -1.3, 2.5),
            (true, false) => (0.8, -0.8, 1.8),
        };
        let mut yaw = (-local[0]).atan2(-local[2]);
        let mut pitch = -local[1].asin() + yaw.abs() * 0.2;
        if -yaw_max < yaw && yaw < yaw_max && down_max < pitch && pitch < up_max {
            if !me.look.tracking {
                let a = delta[0].atan2(delta[2]);
                if crate::eft::wrap_angle(a - me.pose.yaw).abs() < 1.570_796_3 {
                    me.look.tracking = true;
                }
            }
        } else {
            me.look.tracking = false;
        }
        if me.look.tracking {
            let up = deg(byte(|c| f32::from(c.head_up_max)));
            let down = -deg(byte(|c| f32::from(c.head_down_max)));
            let side = deg(byte(|c| f32::from(c.head_yaw_max)));
            if alt {
                pitch = pitch.min(up).max(down);
                yaw = yaw.max(-side).min(side);
                me.obj.head_rot = q_axis_angle(0.0, 1.0, 0.0, yaw);
                me.obj.neck_rot = q_axis_angle(1.0, 0.0, 0.0, head_pitch - pitch);
            } else {
                let axis = norm4([-local[2], 0.0, local[0], 0.0]);
                let angle = -local[1].asin() - (-local[0]).atan2(-local[2]).abs() * local[1] * 0.2;
                if up < angle {
                    local = rotate_axis(local, axis, angle - up);
                }
                if angle < down {
                    local = rotate_axis(local, axis, angle - down);
                }
                for axis in [inv[1], [0.0, 1.0, 0.0, 0.0]] {
                    let a = (-local[0]).atan2(-local[2]);
                    if a < -side {
                        local = rotate_axis(local, axis, -side - a);
                    }
                    if side < a {
                        local = rotate_axis(local, axis, side - a);
                    }
                }
                let fwd = [0.0, 0.0, -1.0, 0.0];
                let mut q = q_from_vectors(fwd, local, 0.5);
                q = crate::btl_obj::quat_mul(q, q_axis_angle(1.0, 0.0, 0.0, head_pitch * 0.5));
                q = crate::btl_obj::quat_mul(q_axis_angle(1.0, 0.0, 0.0, neck_pitch * 0.5), q);
                me.obj.head_rot = q;
                me.obj.neck_rot = q;
            }
            blend = if full { 1.0 } else { 0.0 };
        }
    } else {
        me.look.tracking = false;
    }
    me.obj.head_weight += (blend - me.obj.head_weight) * 0.3;
    me.obj.neck_weight += (blend - me.obj.neck_weight) * 0.3;
}
