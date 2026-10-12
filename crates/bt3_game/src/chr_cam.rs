//! The fighter camera's cuts, fixed camera, shake and swing (src/battle/btl_char_cam.c, the shake of btl_cam.c).
//!
//! A move takes the camera over with a cut: `ChrCam_RequestCut(table, index)` raises flag 0xDB and
//! `ChrCam_StartCut` sets the cut up from entry `index` of the character's cut table (model section 0x1C, table 0)
//! or of the common one (common file 2 word 5, table 1): an orbit point (a node of the fighter, the opponent or
//! the midpoint of both, sampled once or tracked), a target (the body sphere), a look-at node, and yaw / pitch /
//! distance, each a start and what is added over the cut's time. `ChrCam_CalcCut` evaluates it every frame;
//! its flags snap the smoothing, slow it, give this camera the screen (flag 0xD3) or hold the last frame.
//!
//! Not ported: the camera's stage trace (`BtlCam_TraceStage`; the camera stands at its eye, so the turn towards
//! the opponent when the stage is in the way never happens) and the distance presets of SELECT.

use std::f32::consts::{FRAC_PI_2, PI};

use bt3_data::chara_data::CamCutDef;

use crate::btl_chr::{Chr, Env, SideData, add, approach, length, scale3, sub};
use crate::btl_input::wrap_angle;
use crate::rng::LibcRandom;

// CHRCUT_F_*.
const F_SNAP: i32 = 0x1;
const F_SNAP_RUN: i32 = 0x2;
const F_SNAP_END: i32 = 0x4;
const F_RATE: i32 = 0x8;
const F_RATE_RUN: i32 = 0x10;
const F_RATE_END: i32 = 0x20;
const F_PRIORITY: i32 = 0x40;
const F_HOLD: i32 = 0x80;
const F_SET_SIDE: i32 = 0x100;

// CHRCUT_NODE_*.
const NODE_OPP: i32 = 0x4000_0000;
const NODE_MID: i32 = 0x2000_0000;
const NODE_MASK: i32 = 0x9FFF_FFFFu32 as i32;

// CHRCUTDEF_*.
const D_A_TRACK: u16 = 0x1;
const D_OPP: u16 = 0x2;
const D_C_TRACK: u16 = 0x4;
const D_C_OPP: u16 = 0x8;
const D_C2_TRACK: u16 = 0x10;
const D_C2_OPP: u16 = 0x20;
const D_BLEND: u16 = 0x40;
const D_MID: u16 = 0x80;
const D_C_MID: u16 = 0x100;
const D_C2_MID: u16 = 0x200;
const D_MIN_SCALE: u16 = 0x400;

/// `CamShake`: four slots of (time left, strength).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CamShake {
    time: [f32; 4],
    strength: [f32; 4],
}

impl CamShake {
    /// `CamShake_Add`: into the first slot it outdoes, unless one already outdoes it.
    pub(crate) fn add(&mut self, strength: f32, time: f32) {
        for i in 0..4 {
            if self.time[i] <= time && self.strength[i] <= strength {
                self.time[i] = time;
                self.strength[i] = strength;
                return;
            }
            if time <= self.time[i] && strength <= self.strength[i] {
                break;
            }
        }
    }

    /// `CamShake_Tick`.
    pub(crate) fn tick(&mut self) {
        for i in 0..4 {
            self.time[i] -= 0.033_333_335;
            if self.time[i] < 0.0 {
                self.time[i] = 0.0;
                self.strength[i] = 0.0;
            }
        }
    }

    /// `CamShake_Calc`: the position and rotation offsets of the strongest slot (five libc `rand()` draws while
    /// one runs).
    pub(crate) fn calc(&self, libc: &mut LibcRandom) -> ([f32; 3], [f32; 3]) {
        let (mut strength, mut time) = (0.0f32, 0.0f32);
        for i in 0..4 {
            if strength < self.strength[i] {
                time = self.time[i];
                strength = self.strength[i];
            }
        }
        let (mut pos, mut rot) = ([0.0; 3], [0.0; 3]);
        if strength > 0.0 {
            let amp = (time * 0.05 * strength).min(0.1);
            let mut r = || ((libc.rand() & 0xFF) - 0x7F) as f32;
            rot[0] = r() * 0.007_812_5 * amp;
            rot[1] = r() * 0.007_812_5 * amp;
            pos[0] = r() * 0.5 * amp;
            pos[1] = r() * 0.5 * amp;
            pos[2] = r() * 0.5 * amp;
        }
        (pos, rot)
    }
}

/// `ChrCamCut` (fighter + 0x4C0): (start, delta) pairs blended over `total` frames.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CamCut {
    pub req_table: i32,
    pub req_index: i32,
    vec_a: [f32; 3],
    vec_a_delta: [f32; 3],
    vec_b: [f32; 3],
    vec_b_delta: [f32; 3],
    vec_c: [f32; 3],
    vec_c_delta: [f32; 3],
    val_a: f32,
    val_a_delta: f32,
    val_b: f32,
    val_b_delta: f32,
    val_c: f32,
    val_c_delta: f32,
    node_a: i32,
    node_a2: i32,
    node_c: i32,
    node_c2: i32,
    pub timer: i32,
    total: i32,
    pub flags: i32,
}

/// A cut's values (`ChrCam_SetCut`'s arguments).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CutSpec {
    pub vec_a: [f32; 3],
    pub vec_a_delta: [f32; 3],
    pub vec_b: [f32; 3],
    pub vec_b_delta: [f32; 3],
    pub vec_c: [f32; 3],
    pub vec_c_delta: [f32; 3],
    pub node_a: i32,
    pub val_a: f32,
    pub val_a_delta: f32,
    pub val_b: f32,
    pub val_b_delta: f32,
    pub val_c: f32,
    pub val_c_delta: f32,
    pub node_a2: i32,
    pub node_c: i32,
    pub node_c2: i32,
    pub time: i32,
    pub flags: i32,
}

/// `ChrCam_RequestCut`: cut `index` of table `table` (0 the character's, 1 the common one) at the next
/// `ChrCam_StartCut`.
pub(crate) fn request_cut(me: &mut Chr, table: i32, index: i32) {
    me.cut.req_table = table;
    me.cut.req_index = index;
    me.flags.set(0xDB);
}

/// `ChrCam_EndCut`: the end flags the cut asks for.
pub(crate) fn end_cut(me: &mut Chr) {
    if me.cut.flags & F_SNAP_END != 0 {
        me.flags.set(0xCD);
    }
    if me.cut.flags & F_RATE_END != 0 {
        me.flags.set(0xD1);
    }
    me.cut.timer = 0;
    me.cut.flags = 0;
}

/// `ChrCam_IsCutActive`: running, held, or just requested.
pub(crate) fn is_cut_active(me: &Chr) -> bool {
    me.cut.timer > 0 || me.cut.flags & F_HOLD != 0 || me.flags.test(0xDB)
}

/// `ChrCam_AddShake` (while the side's shake option is on).
pub(crate) fn add_shake(me: &mut Chr, strength: f32, time: f32) {
    if me.cam.shake_on {
        me.cam.shake.add(strength, time);
    }
}

/// `ChrCam_SetCut`.
pub(crate) fn set_cut(me: &mut Chr, s: &CutSpec) {
    let c = &mut me.cut;
    c.vec_a = s.vec_a;
    c.vec_a_delta = s.vec_a_delta;
    c.vec_b = s.vec_b;
    c.vec_b_delta = s.vec_b_delta;
    c.vec_c = s.vec_c;
    c.vec_c_delta = s.vec_c_delta;
    c.node_a = s.node_a;
    c.node_a2 = s.node_a2;
    c.node_c = s.node_c;
    c.node_c2 = s.node_c2;
    c.val_a = s.val_a;
    c.val_a_delta = s.val_a_delta;
    c.val_b = s.val_b;
    c.val_b_delta = s.val_b_delta;
    c.val_c = s.val_c;
    c.val_c_delta = s.val_c_delta;
    c.timer = s.time;
    c.total = s.time;
    c.flags = s.flags;
    if s.flags & F_SNAP != 0 {
        me.flags.set(0xCD);
    }
    if s.flags & F_RATE != 0 {
        me.flags.set(0xD1);
    }
}

/// The running cut's progress 0..1.
fn progress(c: &CamCut) -> f32 {
    if c.total > 0 {
        1.0 - c.timer as f32 / c.total as f32
    } else {
        0.0
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// `ChrCam_BlendToCut`: a cut that starts where the camera is now (the running cut's current values, or the live
/// camera's pose turned into an orbit).
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_to_cut(
    me: &mut Chr,
    vec_a_delta: [f32; 3],
    vec_b_delta: [f32; 3],
    vec_c_delta: [f32; 3],
    node_a2: i32,
    node_c2: i32,
    val_a_delta: f32,
    val_b_delta: f32,
    val_c_delta: f32,
    time: i32,
    flags: i32,
) {
    let c = me.cut;
    let mut s = CutSpec {
        vec_a_delta,
        vec_b_delta,
        vec_c_delta,
        node_a2,
        node_c2,
        val_a_delta,
        val_b_delta,
        val_c_delta,
        time,
        flags: flags & !9,
        ..CutSpec::default()
    };
    if c.timer > 0 || c.flags & F_HOLD != 0 {
        let t = progress(&c);
        s.vec_a = add(c.vec_a, scale3(c.vec_a_delta, t));
        s.vec_b = add(c.vec_b, scale3(c.vec_b_delta, t));
        s.vec_c = add(c.vec_c, scale3(c.vec_c_delta, t));
        s.node_a = c.node_a;
        s.val_a = c.val_a + c.val_a_delta * t;
        s.val_b = c.val_b + c.val_b_delta * t;
        s.node_c = c.node_c;
        s.val_c = c.val_c + c.val_c_delta * t;
    } else {
        let cam = me.cam;
        s.node_c = -1;
        s.node_a = -1;
        s.val_b = cam.rot[0];
        s.val_a = cam.rot[1];
        s.vec_b = cam.target;
        let pitch = -s.val_b;
        s.val_c = length(sub(cam.target, cam.eye));
        let dir = [
            pitch.cos() * s.val_a.sin(),
            pitch.sin(),
            pitch.cos() * s.val_a.cos(),
        ];
        s.vec_a = add(cam.eye, scale3(dir, s.val_c));
        s.vec_c = s.vec_a;
    }
    set_cut(me, &s);
}

/// A node of the fighter (`opp` false) or the opponent, or the midpoint of both.
fn node_of(me: &Chr, opp: &Chr, which: u8, node: u8) -> [f32; 3] {
    let n = u32::from(node);
    match which {
        0 => me.obj.node_pos(n),
        1 => opp.obj.node_pos(n),
        _ => lerp3(me.obj.node_pos(n), opp.obj.node_pos(n), 0.5),
    }
}

/// `*(obj + 0xFA0)`: the object's current body sphere centre.
fn body_pos(c: &Chr) -> [f32; 3] {
    c.obj.body.sphere[c.obj.body.cur].0
}

/// `ChrCam_StartCut`: the cut asked for with `request_cut`, set up from its table entry.
pub(crate) fn start_cut(me: &mut Chr, opp: &Chr, data: &SideData, opp_data: &SideData, env: &Env) {
    if !me.flags.test(0xDB) {
        return;
    }
    let def: Option<CamCutDef> = match me.cut.req_table {
        0 => data.tables.cam_cuts.get(me.cut.req_index as usize).copied(),
        1 => env.common.cam_cuts.get(me.cut.req_index as usize).copied(),
        _ => None,
    };
    let Some(def) = def else {
        return;
    };
    let flags = def.flags;
    let (mut vec_a, mut vec_a_delta);
    let (mut node_a, mut node_a2) = (-1, -1);
    let vec_b;
    let yaw;
    let mut scale;
    if flags & D_OPP != 0 {
        yaw = opp.snap_rot(6)[1];
        scale = opp_data.height();
        if flags & D_A_TRACK != 0 {
            vec_a = [0.0; 3];
            vec_a_delta = [0.0; 3];
            node_a = i32::from(def.node_a) | NODE_OPP;
            node_a2 = i32::from(def.node_a2) | NODE_OPP;
        } else {
            vec_a = node_of(me, opp, 1, def.node_a);
            vec_a_delta = sub(node_of(me, opp, 1, def.node_a2), vec_a);
        }
        vec_b = body_pos(opp);
    } else if flags & D_MID != 0 {
        yaw = me.pose.yaw;
        scale = data.height().max(opp_data.height());
        vec_a = node_of(me, opp, 2, def.node_a);
        let mid = node_of(me, opp, 2, def.node_a2);
        vec_a_delta = sub(mid, vec_a);
        vec_b = vec_a;
        if flags & D_A_TRACK != 0 {
            vec_a = [0.0; 3];
            vec_a_delta = [0.0; 3];
            node_a = i32::from(def.node_a) | NODE_MID;
            node_a2 = i32::from(def.node_a2) | NODE_MID;
        }
    } else {
        yaw = me.pose.yaw;
        scale = data.height();
        if flags & D_A_TRACK != 0 {
            vec_a = [0.0; 3];
            vec_a_delta = [0.0; 3];
            node_a = i32::from(def.node_a);
            node_a2 = i32::from(def.node_a2);
        } else {
            vec_a = node_of(me, opp, 0, def.node_a);
            vec_a_delta = sub(node_of(me, opp, 0, def.node_a2), vec_a);
        }
        vec_b = body_pos(me);
    }
    let mid3 = || lerp3(me.obj.node_pos(3), opp.obj.node_pos(3), 0.5);
    let (vec_c, node_c) = if flags & D_C_OPP != 0 {
        if flags & D_C_TRACK != 0 {
            ([0.0; 3], i32::from(def.node_c) | NODE_OPP)
        } else {
            (node_of(me, opp, 1, def.node_c), -1)
        }
    } else if flags & D_C_MID != 0 {
        if flags & D_C_TRACK != 0 {
            ([0.0; 3], i32::from(def.node_c) | NODE_MID)
        } else {
            (mid3(), -1)
        }
    } else if flags & D_C_TRACK != 0 {
        ([0.0; 3], i32::from(def.node_c))
    } else {
        (node_of(me, opp, 0, def.node_c), -1)
    };
    let (vec_c_delta, node_c2) = if flags & D_C2_OPP != 0 {
        if flags & D_C2_TRACK != 0 {
            ([0.0; 3], i32::from(def.node_c2) | NODE_OPP)
        } else {
            (sub(node_of(me, opp, 1, def.node_c2), vec_c), -1)
        }
    } else if flags & D_C2_MID != 0 {
        if flags & D_C2_TRACK != 0 {
            ([0.0; 3], i32::from(def.node_c2) | NODE_MID)
        } else {
            (sub(mid3(), vec_c), -1)
        }
    } else if flags & D_C2_TRACK != 0 {
        ([0.0; 3], i32::from(def.node_c2))
    } else {
        (sub(node_of(me, opp, 0, def.node_c2), vec_c), -1)
    };
    if flags & D_MIN_SCALE != 0 {
        scale = scale.max(15.0);
    }
    let val_a = wrap_angle(yaw + def.yaw * PI / 180.0);
    let val_a_delta = def.yaw_delta * PI / 180.0;
    let val_b = def.pitch * PI / 180.0;
    let val_b_delta = def.pitch_delta * PI / 180.0;
    let val_c = def.dist * scale;
    let val_c_delta = def.dist_delta * scale;
    let time = ((def.seconds * 30.0) as i32).max(1);
    let cut_flags = i32::from(def.cut_flags);
    if flags & D_BLEND != 0 {
        blend_to_cut(
            me,
            vec_a_delta,
            [0.0; 3],
            vec_c_delta,
            node_a2,
            node_c2,
            val_a_delta,
            val_b_delta,
            val_c_delta,
            time,
            cut_flags,
        );
    } else {
        set_cut(
            me,
            &CutSpec {
                vec_a,
                vec_a_delta,
                vec_b,
                vec_b_delta: [0.0; 3],
                vec_c,
                vec_c_delta,
                node_a,
                val_a,
                val_a_delta,
                val_b,
                val_b_delta,
                val_c,
                val_c_delta,
                node_a2,
                node_c,
                node_c2,
                time,
                flags: cut_flags,
            },
        );
    }
}

/// The position of a cut node (`CHRCUT_NODE_*`): `None` when the opponent's is not refreshed (flag 0xB4).
fn cut_node(me: &Chr, opp: &Chr, node: i32, frozen: bool) -> Option<[f32; 3]> {
    let n = (node & NODE_MASK) as u32;
    if node & NODE_OPP != 0 {
        (!frozen).then(|| opp.obj.node_pos(n))
    } else if node & NODE_MID != 0 {
        Some(lerp3(me.obj.node_pos(n), opp.obj.node_pos(n), 0.5))
    } else {
        Some(me.obj.node_pos(n))
    }
}

/// `ChrCam_GetSideLimit`.
fn side_limit(data: &SideData) -> f32 {
    data.cam_side_slope().atan().max(0.785_398_2)
}

/// `ChrCam_CalcCut`: the running cut's eye, rotation and target; raises its flags and counts its timer down.
pub(crate) fn calc_cut(me: &mut Chr, opp: &Chr, data: &SideData) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let frozen = me.flags.test(0xB4);
    let t = progress(&me.cut);
    let (mut refreshed, mut used_opp, mut used_mid) = (false, false, false);
    let mut c = me.cut;
    if c.node_a >= 0 {
        used_opp |= c.node_a & NODE_OPP != 0;
        used_mid |= c.node_a & NODE_OPP == 0 && c.node_a & NODE_MID != 0;
        if let Some(p) = cut_node(me, opp, c.node_a, frozen) {
            c.vec_a = p;
        }
        refreshed = true;
    }
    if c.node_a2 >= 0 {
        if c.node_a == c.node_a2 {
            c.vec_a_delta = [0.0; 3];
        } else {
            used_opp |= c.node_a2 & NODE_OPP != 0;
            used_mid |= c.node_a2 & NODE_OPP == 0 && c.node_a2 & NODE_MID != 0;
            // An opponent's node that is not refreshed leaves the original's temporary unset; the cut keeps
            // still here.
            let p = cut_node(me, opp, c.node_a2, frozen).unwrap_or(c.vec_a);
            c.vec_a_delta = sub(p, c.vec_a);
        }
        refreshed = true;
    }
    if c.node_c >= 0
        && let Some(p) = cut_node(me, opp, c.node_c, frozen)
    {
        c.vec_c = p;
    }
    if c.node_c2 >= 0 {
        if c.node_c == c.node_c2 {
            c.vec_c_delta = [0.0; 3];
        } else {
            let p = cut_node(me, opp, c.node_c2, frozen).unwrap_or(c.vec_c);
            c.vec_c_delta = sub(p, c.vec_c);
        }
    }
    if refreshed {
        c.vec_b = if used_opp {
            body_pos(opp)
        } else if used_mid {
            lerp3(body_pos(opp), body_pos(me), 0.5)
        } else {
            body_pos(me)
        };
        c.vec_b_delta = [0.0; 3];
    }
    let mut eye = add(c.vec_a, scale3(c.vec_a_delta, t));
    let target = add(c.vec_b, scale3(c.vec_b_delta, t));
    let look = add(c.vec_c, scale3(c.vec_c_delta, t));
    let yaw = c.val_a + c.val_a_delta * t;
    let pitch = -(c.val_b + c.val_b_delta * t);
    let dist = c.val_c + c.val_c_delta * t;
    let dir = [
        pitch.cos() * yaw.sin(),
        pitch.sin(),
        pitch.cos() * yaw.cos(),
    ];
    eye = sub(eye, scale3(dir, dist));
    let mut rot = [0.0; 3];
    let d = sub(look, eye);
    let len = length(d);
    if 0.001 < len {
        let d = scale3(d, 1.0 / len).map(|v| v.clamp(-1.0, 1.0));
        rot[0] = (-d[1]).asin();
        rot[1] = if d[0].abs() < 0.001 && d[2].abs() < 0.001 {
            0.0
        } else {
            d[0].atan2(d[2])
        };
    }
    me.cut = c;
    if c.flags & F_SNAP_RUN != 0 {
        me.flags.set(0xCD);
    }
    if c.flags & F_RATE_RUN != 0 {
        me.flags.set(0xD1);
    }
    if c.flags & F_PRIORITY != 0 {
        me.flags.set(0xD3);
    }
    if me.cut.timer > 0 {
        me.cut.timer -= 1;
        if me.cut.timer == 0 && me.cut.flags & F_HOLD == 0 {
            if me.cut.flags & F_SNAP_END != 0 {
                me.flags.set(0xCE);
            }
            end_cut(me);
        }
    }
    if c.flags & F_SET_SIDE != 0 {
        let d = sub(me.pose.pos, me.cam.eye);
        if 0.001 < d[0].abs() || 0.001 < d[2].abs() {
            let limit = side_limit(data);
            let a = d[0].atan2(d[2]);
            me.cam.side = (-wrap_angle(a - me.pose.yaw)).clamp(-limit, limit);
        }
    }
    me.flags.set(0xD4);
    (eye, rot, target)
}

/// `ChrCam_GetBob`: the vertical swing while airborne (flag 0xE), settling back on the ground; none while time
/// is stopped or the shake option is off.
pub(crate) fn bob(me: &mut Chr, time_stopped: bool) -> f32 {
    let cam = &mut me.cam;
    if time_stopped || !cam.shake_on {
        cam.bob = 0.0;
        return cam.bob;
    }
    if me.flags.test(0xE) {
        cam.bob = wrap_angle(cam.bob + PI / 75.0);
    } else {
        if FRAC_PI_2 < cam.bob {
            cam.bob = PI - cam.bob;
        }
        if cam.bob < -FRAC_PI_2 {
            cam.bob = -PI - cam.bob;
        }
        cam.bob = approach(cam.bob, 0.0, PI / 30.0);
    }
    cam.bob.sin() * 1.5
}

/// `ChrCam_CalcFixed` (flags 0xDC / 0xB8): beside the fighter, angled by the opponent's height.
pub(crate) fn calc_fixed(
    me: &Chr,
    opp: &Chr,
    data: &SideData,
    opp_data: &SideData,
) -> ([f32; 3], [f32; 3], [f32; 3], f32) {
    let scale = data.height();
    let pos = me.pose.pos;
    let mut to_opp = sub(opp.snap_pos(6), pos);
    to_opp[1] += data.height() * 0.5 - opp_data.height() * 0.5;
    let len = length(to_opp);
    let mut dir = if len < 0.01 {
        [0.0, 0.0, 1.0]
    } else {
        scale3(to_opp, 1.0 / len)
    };
    let max_pitch = PI * 0.35;
    let k = 0.4f32;
    let mut base = -0.1f32;
    let mut target = [0.0, -(scale * 0.7), 0.0];
    let mut ofs = sub([0.0, -data.height() * k - 10.0, -35.0], target);
    let cam_yaw = me.cam.yaw;
    dir[1] = dir[1].clamp(-1.0, 1.0);
    let mut a = (-dir[1].asin() / max_pitch).clamp(-1.0, 1.0);
    let mut b = a.max(0.0);
    b = b * 2.0 - 1.0;
    a = -a * a.abs() + a * 2.0;
    b = -b * b.abs() + b * 2.0;
    let pitch = a * max_pitch * k;
    b = b * 0.5 + 0.5;
    ofs = crate::battle::rotate_x(ofs, pitch);
    ofs = scale3(ofs, 1.0 - b * 0.3);
    base = pitch - b * base + base;
    let horiz = to_opp[0].hypot(to_opp[2]);
    let sign = if me.cam.side < 0.0 { -1.0 } else { 1.0 };
    ofs = crate::battle::rotate_y(ofs, sign * (PI * 0.4));
    let t = (-ofs[0]).atan2(horiz * 0.5 - ofs[2]);
    let yaw = wrap_angle(cam_yaw - t);
    target = add(target, pos);
    let (s, c) = cam_yaw.sin_cos();
    let out = [-c * ofs[0] + s * ofs[2], ofs[1], s * ofs[0] + c * ofs[2]];
    (add(target, out), [base, yaw, 0.0], target, t)
}
