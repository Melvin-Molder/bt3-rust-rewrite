//! The volley of shots, technique effect type 1 (`EftVolley_*`, src/battle/eft_emit.c), the blast objects it
//! fires (`EftBlastObj_*`, src/battle/eft_blast_obj.c: one flying shot, a task of layer 3) and their aim
//! (`EftVolleyAim_*`, src/battle/eft_detect.c): each event of the technique fires a shot from the firing node
//! (phase 1, the FIRE event, fires the definition's volley of live shots; the other phases' shots stay frozen and
//! harmless); the live shots are spread round the aim by the set's aim kind with the scene generator and steered
//! at the opponent every frame, and each publishes a technique hit record. The volley ends when its shots are gone
//! after the last firing, or on the END event; its end raises flag 0xA8.
//!
//! The volley drives its technique set's emitters but those of the phases its shots fire in; each shot drives the
//! emitters of its (phase, node slot) pairs, from a copy of the volley's node slots (`EftBlastObj_UpdateParts`).
//!
//! Not ported: the models (drawing only), and techniques 0x26E / 0x26F, whose shots wait for their model's
//! animation (their volley is not started).

use std::io;

use bt3_data::eft_pack::{EmitSet, SetHead};
use bt3_data::elf::Elf;

use crate::battle::{IDENTITY, apply};
use crate::btl_blast::{HitRec, Tag, World, home};
use crate::btl_chr::{SideData, add, dot, length, scale3, sub};
use crate::btl_tech_fx::{
    EV_ABORT, EV_END, EV_FIRE, EV_START, EmitNodes, T_HIT_CHAR, T_MULTI, TechSrc, aim_dir,
    aim_dir_keep, emit_state, parts_alive, stopped, update_parts, update_trail_width,
};
use crate::eft::{EmitState, Modules, is_part_deferred, rotate_xyz};

/// `gMathfSinCoef`: the coefficients of `Mathf_SinFast` (x^9, x^7, x^5, x^3).
const SIN_COEF: u32 = 0x2C_3450;
/// `gEftVolleyAimScripts`: the two steering scripts of the lobbed shots.
const AIM_SCRIPTS: u32 = 0x2C_46B0;

/// `EftBlastObjMgr_Init`: blast objects, both fighters together.
const SHOTS_MAX: usize = 60;
/// `EftVolleyWork.shot`.
const VOLLEY_SHOTS: usize = 30;

/// One step of a steering script (`EftVolleyAimStep`): applies while t0 <= frames since fired <= t1.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AimStep {
    op: i32,
    t0: u8,
    t1: u8,
    v0: u8,
    v1: u8,
}

// EFT_VOLLEY_OP_*.
const OP_TURN: i32 = 1;
const OP_SPEED: i32 = 2;
const OP_SCATTER: i32 = 3;
const OP_BEHIND: i32 = 4;
const OP_ABOVE: i32 = 5;
const OP_FRONT: i32 = 6;

/// The executable's tables the technique effects read.
#[derive(Clone, Debug, Default)]
pub(crate) struct FxTables {
    pub sin_coef: [f32; 4],
    pub aim_scripts: [Vec<AimStep>; 2],
    /// The vanish lines' body segments and column slices.
    pub shotfx: crate::eft_shotfx::Tables,
}

impl FxTables {
    pub(crate) fn read(elf: &Elf) -> io::Result<Self> {
        let c = elf.f32s(SIN_COEF, 4)?;
        let ptrs = elf.i32s(AIM_SCRIPTS, 2)?;
        let mut aim_scripts: [Vec<AimStep>; 2] = Default::default();
        for (script, &at) in aim_scripts.iter_mut().zip(&ptrs) {
            for k in 0..64 {
                let b = elf.read(at as u32 + k * 8, 8)?;
                let op = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                if op == 0 {
                    break;
                }
                script.push(AimStep {
                    op,
                    t0: b[4],
                    t1: b[5],
                    v0: b[6],
                    v1: b[7],
                });
            }
        }
        Ok(Self {
            sin_coef: [c[0], c[1], c[2], c[3]],
            aim_scripts,
            shotfx: crate::eft_shotfx::Tables::read(elf)?,
        })
    }
}

// ---- maths (src/sys/mathf.c, src/battle/eft_core.c) ----------------------------------------------------------

const PI_F: f32 = 3.141_592_7;

/// `Mathf_WrapAngle(angle, pi)`: pushed up to at least 2 pi, then brought down into (-pi, pi], one addition at a
/// time.
fn mathf_wrap(mut a: f32) -> f32 {
    let range = PI_F + PI_F;
    while a < range {
        a += range;
    }
    while PI_F < a {
        a -= range;
    }
    a
}

/// `Mathf_Sin`: `sinf` of the wrapped angle.
pub(crate) fn mathf_sin(angle: f32) -> f32 {
    mathf_wrap(angle).sin()
}

/// `Mathf_Cos`: `Mathf_Sin(angle + pi / 2)`.
pub(crate) fn mathf_cos(angle: f32) -> f32 {
    mathf_sin(angle + PI_F / 2.0)
}

/// `Mathf_SinFast`: the VU0 polynomial x + c3 x^3 + c2 x^5 + c1 x^7 + c0 x^9 of the wrapped angle, summed in
/// that order.
pub(crate) fn sin_fast(c: &[f32; 4], angle: f32) -> f32 {
    let x = mathf_wrap(angle);
    let x2 = x * x;
    let mut t = c.map(|k| k * x * x2);
    let mut r = x + t[3];
    for v in &mut t[..3] {
        *v *= x2;
    }
    r += t[2];
    for v in &mut t[..2] {
        *v *= x2;
    }
    r += t[1];
    t[0] *= x2;
    r + t[0]
}

/// `Mathf_CosFast`.
pub(crate) fn cos_fast(c: &[f32; 4], angle: f32) -> f32 {
    sin_fast(c, angle + PI_F / 2.0)
}

/// `EftMath_WrapAngle`: whole turns removed by their count.
pub(crate) fn eft_wrap(a: f32) -> f32 {
    const TWO_PI: f32 = 6.283_185_3;
    if a >= PI_F {
        let n = (((a - PI_F) / TWO_PI) as i32).abs();
        a - (n as f32 * TWO_PI + TWO_PI)
    } else if a <= -PI_F {
        let n = (((a + PI_F) / TWO_PI) as i32).abs();
        a + (n as f32 * TWO_PI + TWO_PI)
    } else {
        a
    }
}

/// `Mathf_Asin` / `Mathf_Acos`: the argument clamped.
fn asin_c(x: f32) -> f32 {
    x.clamp(-1.0, 1.0).asin()
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    crate::btl_blast::norm(v)
}

/// The scene's generator (`BtlScene_Rand`, `gBtlScene->randState`), kept while time is stopped.
fn scene_rand(state: &mut u32, time_stopped: bool) -> u32 {
    let next = ((u64::from(*state) * 714_025 + 0x1000) % 150_889) as u32;
    if !time_stopped {
        *state = next;
    }
    next
}

/// `BtlScene_RandF`.
pub(crate) fn scene_rand_f(state: &mut u32, ts: bool) -> f32 {
    scene_rand(state, ts) as f32 / 150_889.0
}

/// `BtlScene_RandRangeF`.
fn rand_range_f(state: &mut u32, ts: bool, a: f32, b: f32) -> f32 {
    if b == a {
        return b;
    }
    let (a, b) = if b < a { (b, a) } else { (a, b) };
    a + scene_rand(state, ts) as f32 / 150_889.0 * (b - a)
}

/// `BtlScene_RandRange`.
fn rand_range(state: &mut u32, ts: bool, a: i32, b: i32) -> i32 {
    if b == a {
        return a;
    }
    let (a, b) = if b < a { (b, a) } else { (a, b) };
    a + (scene_rand(state, ts) % (b - a + 1) as u32) as i32
}

// ---- the aim (eft_detect.c) ----------------------------------------------------------------------------------

/// `EftVolleyAim_GetNodeSide`: 0 for the nodes 0x12..0x1F and 0x36, 1 for 0x20..0x2D.
fn node_side(node: i32) -> i32 {
    i32::from((0x20..=0x2D).contains(&node))
}

/// The randomness the spread and the steering draw from.
struct Rng<'a> {
    state: &'a mut u32,
    ts: bool,
}

impl Rng<'_> {
    fn f(&mut self, a: f32, b: f32) -> f32 {
        rand_range_f(self.state, self.ts, a, b)
    }
}

/// `EftVolleyAim_Spread`: turns `dir` by a yaw / pitch offset of the aim kind, the shot's index and the scene
/// generator (closer fighters spread less); a shot low over the ground does not dip much. Returns the steering
/// script of the lobbed kinds 7 / 8, else -1.
#[allow(clippy::too_many_arguments)]
fn spread(
    w: &World,
    c: &[f32; 4],
    rng: &mut Rng,
    side: usize,
    dir: &mut [f32; 3],
    kind: i32,
    node_side: i32,
    index: i32,
    count: i32,
) -> i32 {
    let deg = |x: f32| x / 180.0 * PI_F;
    let (mut jit_v, mut jit_h) = (deg(1.0), deg(1.0));
    let mut script = -1;
    let d = sub(w.chars[0].pose.pos, w.chars[1].pose.pos);
    let dist_sq = dot(d, d);
    let (mut spr_h, mut spr_v) = (deg(12.5), deg(10.0));
    if kind == 9 {
        return -1;
    }
    match kind {
        7 | 8 => {
            spr_h = deg(12.5) * 0.5;
            jit_h = deg(1.0) * 0.5;
        }
        10 => {
            spr_h = deg(12.5) * 0.02;
            jit_h = deg(1.0) * 0.02;
            spr_v = deg(10.0) * 0.05;
            jit_v = deg(1.0) * 0.05;
        }
        12 => {
            jit_h = deg(1.0) * 0.05;
            spr_h = deg(12.5) * 0.05;
            spr_v = deg(10.0) * 0.05;
            jit_v = deg(1.0) * 0.05;
        }
        _ if index == 0 => {
            spr_h = deg(12.5) * 0.1;
            spr_v = deg(10.0) * 0.1;
        }
        _ if dist_sq < 250_000.0 => {
            let t = if 10_000.0 < dist_sq {
                0.5 + (1.0 - 0.5) * ((dist_sq - 10_000.0) / 240_000.0)
            } else {
                0.5
            };
            spr_h *= t;
            spr_v *= t;
        }
        _ => {}
    }
    let (yaw, mut pitch);
    match kind {
        0 => {
            yaw = rng.f(-spr_h - jit_h, spr_h + jit_h);
            pitch = rng.f(-spr_v - jit_v, spr_v + jit_v);
        }
        1 => {
            yaw = spr_h * 2.0 / count as f32 * index as f32 - spr_h + rng.f(-jit_h, jit_h);
            pitch = rng.f((-spr_v - jit_v) * 0.5, (spr_v + jit_v) * 0.5);
        }
        2 => {
            if index >= 4 {
                let t = 0.5f32.powf((index / 4) as f32);
                spr_h *= t;
                spr_v *= t;
            }
            let y = spr_h + rng.f(-jit_h, jit_h);
            yaw = if index % 2 != node_side { -y } else { y };
            pitch = rng.f(-spr_v - jit_v, spr_v + jit_v);
        }
        3 => {
            let a = rng.f(-PI_F, PI_F);
            yaw = spr_h * sin_fast(c, a) + rng.f(-jit_h, jit_h);
            pitch = spr_v * cos_fast(c, a) + rng.f(-jit_v, jit_v);
        }
        4 => {
            let mut a = PI_F * 2.0 / (count + 2) as f32;
            let half = index / 2;
            a = if index % 2 == node_side {
                a * (half + 1) as f32 + -PI_F / 2.0
            } else {
                eft_wrap(-PI_F / 2.0 - a * (half + 1) as f32)
            };
            yaw = spr_h * cos_fast(c, a) + rng.f(-jit_h, jit_h);
            pitch = spr_v * sin_fast(c, a) + rng.f(-jit_v, jit_v);
        }
        5 | 6 => {
            let mut a = PI_F / (count - 1) as f32;
            let half = index / 2;
            a = if index % 2 == node_side {
                a * half as f32
            } else {
                PI_F - a * half as f32
            };
            if kind == 6 && half % 2 == 1 {
                spr_h *= 0.5;
                spr_v *= 0.5;
            }
            yaw = spr_h * cos_fast(c, a) + rng.f(-jit_h, jit_h);
            pitch = spr_v * sin_fast(c, a) + rng.f(-jit_v, jit_v);
        }
        7 | 8 => {
            pitch = if kind == 7 { PI_F / 2.0 } else { PI_F / 4.0 };
            script = kind - 7;
            yaw = rng.f(-spr_h - jit_h, spr_h + jit_h);
            dir[1] = 0.0;
        }
        10 => {
            let y = spr_h + jit_h;
            pitch = rng.f(-spr_v - jit_v, spr_v + jit_v);
            yaw = if index & 1 != 0 { -y } else { y };
        }
        11 => {
            yaw = spr_h - spr_h * 2.0 / count as f32 * index as f32 + rng.f(-jit_h, jit_h);
            pitch = rng.f((-spr_v - jit_v) * 0.5, (spr_v + jit_v) * 0.5);
        }
        12 => {
            yaw = if index == 0 {
                0.0
            } else if index & 1 != 0 {
                (index / 2) as f32 * spr_h + spr_h + rng.f(0.0, jit_h)
            } else {
                -((index / 2) as f32 * spr_h + spr_h + rng.f(0.0, jit_h))
            };
            pitch = rng.f(-spr_v - jit_v, spr_v + jit_v);
        }
        _ => {
            yaw = 0.0;
            pitch = 0.0;
        }
    }
    if !(kind == 7 || kind == 8) {
        let me = &w.chars[side];
        // BtlCharApi_GetAltitude.
        if me.ground_y() - (me.pose.pos[1] + me.pose.disp[1]) < 30.0 && pitch < 0.0 {
            pitch *= 0.4;
        }
    }
    let d = dir.map(|v| v.clamp(-1.0, 1.0));
    let rx = eft_wrap(asin_c(-d[1]) + pitch);
    let ry = eft_wrap(d[0].atan2(d[2]) + yaw);
    // Mtx_RotateX, Mtx_RotateY, then (0, 0, 1) through the matrix.
    *dir = [rx.cos() * ry.sin(), -rx.sin(), rx.cos() * ry.cos()];
    script
}

/// `EftVolleyAimShot`: a live shot's steering.
#[derive(Clone, Copy, Debug, Default)]
struct AimShot {
    pos: [f32; 3],
    dir: [f32; 3],
    offset: [f32; 3],
    script: i32,
    speed: f32,
    max_turn: f32,
}

/// `EftVolleyAim_Steer`: aims at the opponent's node 0x11 (plus the offset) led by its frame movement over half
/// the frames to reach it (at most 15), turns by at most the shot's turn rate (only toward a target in front
/// with `front_only`), then moves the shot.
fn steer(w: &World, side: usize, shot: &mut AimShot, front_only: bool) {
    let opp = &w.chars[side ^ 1];
    let mut target = add(opp.obj.node_pos(0x11), shot.offset);
    let d = sub(target, shot.pos);
    let t = (length(d) / shot.speed).min(15.0);
    target = add(target, scale3(opp.pose.moved, t * 0.5));
    let d = norm(sub(target, shot.pos));
    shot.dir = norm(shot.dir);
    let dt = dot(d, shot.dir);
    if !front_only || 0.0 < dt {
        let a = dt.clamp(0.0, 1.0).acos().min(shot.max_turn);
        let axis = norm(cross(shot.dir, d));
        shot.dir = norm(rotate_axis(shot.dir, axis, a));
    }
    shot.pos = add(shot.pos, scale3(shot.dir, shot.speed));
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `Vec3_RotateAxis` (Rodrigues).
fn rotate_axis(v: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    let k = dot(axis, v) * (1.0 - c);
    add(
        add(scale3(v, c), scale3(cross(axis, v), s)),
        scale3(axis, k),
    )
}

/// `EftVolleyAim_Update`: the lobbed shots' script for the frames since they were fired (turn rate, speed, a new
/// target offset round, behind or above the opponent, front only), then the steering.
#[allow(clippy::too_many_arguments)]
fn aim_update(
    w: &World,
    tables: &FxTables,
    rng: &mut Rng,
    side: usize,
    shot: &mut AimShot,
    exact: bool,
    time: f32,
    speed: f32,
    max_turn: f32,
) {
    let mut front_only = false;
    let to_opp = |w: &World| sub(w.chars[side ^ 1].pose.pos, w.chars[side].pose.pos);
    let script = usize::try_from(shot.script)
        .ok()
        .and_then(|s| tables.aim_scripts.get(s));
    for p in script.into_iter().flatten() {
        let (t0, t1) = (f32::from(p.t0), f32::from(p.t1));
        if !(t0 <= time && time <= t1) {
            continue;
        }
        let lerp = |base: f32| {
            let a = base * f32::from(p.v0) * 0.1;
            let b = base * f32::from(p.v1) * 0.1;
            let t = if time != t0 {
                (time - t0) / f32::from(p.t1 - p.t0)
            } else {
                1.0
            };
            a + (b - a) * t
        };
        match p.op {
            OP_TURN => shot.max_turn = lerp(max_turn),
            OP_SPEED => shot.speed = lerp(speed),
            OP_SCATTER => {
                let r;
                if rand_range(rng.state, rng.ts, 0, i32::from(p.v1)) == 0 || exact {
                    shot.offset = [0.0; 3];
                    r = f32::from(p.v0) * 0.04;
                } else if rand_range(rng.state, rng.ts, 0, i32::from(p.v1 >> 1)) == 0 {
                    shot.offset = [0.0; 3];
                    r = f32::from(p.v0) * 0.4;
                } else {
                    let o = to_opp(w);
                    let len_sq = dot(o, o);
                    let v0 = i32::from(p.v0);
                    let k = if len_sq < (v0 * v0) as f32 {
                        len_sq.sqrt()
                    } else {
                        f32::from(p.v0)
                    } * -0.5;
                    shot.offset = scale3(norm(o), k);
                    r = f32::from(p.v0);
                }
                shot.offset[0] += rng.f(-r, r);
                shot.offset[1] = 0.0;
                shot.offset[2] += rng.f(-r, r);
            }
            OP_BEHIND | OP_ABOVE => {
                let mut o = to_opp(w);
                o[1] = 0.0;
                o = scale3(norm(o), -1000.0);
                if p.op == OP_ABOVE {
                    let mut t = norm(shot.pos);
                    t[1] = 0.0;
                    o = add(o, scale3(t, -400.0));
                    o[1] -= 1000.0;
                }
                shot.offset = o;
            }
            OP_FRONT => {
                if p.v0 != 0 {
                    front_only = true;
                }
            }
            _ => {}
        }
    }
    steer(w, side, shot, front_only);
}

// ---- the blast object (eft_blast_obj.c) ---------------------------------------------------------------------

// EFT_BLASTOBJ_*.
const B_ALIVE: u32 = 0x1;
const B_STOP: u32 = 0x2;
const B_DYING: u32 = 0x4;
const B_STUCK: u32 = 0x20;
const B_FROZEN: u32 = 0x40;
const B_NO_HIT: u32 = 0x80;
const B_HELD: u32 = 0x100;
const B_PLACED: u32 = 0x200;
const B_TIMED: u32 = 0x400;
const B_LAST: u32 = 0x800;

/// An `EftBlastObj`: one flying shot.
#[derive(Clone, Debug)]
pub(crate) struct Shot {
    id: u32,
    src: TechSrc,
    tag: Tag,
    flags: u32,
    life_left: f32,
    scale: f32,
    speed: f32,
    delay: f32,
    dir: [f32; 3],
    target: [f32; 3],
    start: [f32; 3],
    pos: [f32; 3],
    prev: [f32; 3],
    vel: [f32; 3],
    head_set: SetHead,
    trail_width: f32,
    trail_time: f32,
    /// The creator's node slots, its own refreshed.
    nodes: EmitNodes,
    /// The (phase, node slot) pairs of the set's emitters it drives (`sel`).
    sel: [(u8, u8); 2],
    sel_count: usize,
    emit: Option<EmitState>,
}

impl Shot {
    /// `EftBlastObj_Init`: at `pos` along `dir` (as given), its last position the owner's node 0x11, held for
    /// the frame it is created in; timed when given a life (seconds).
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: u32,
        src: TechSrc,
        w: &World,
        cur_slot: i32,
        set: &SetHead,
        nodes: &EmitNodes,
        sel: (u8, u8),
        pos: [f32; 3],
        dir: [f32; 3],
        life: f32,
        scale: f32,
        speed: f32,
    ) -> Self {
        let mut flags = B_ALIVE;
        let mut life_left = 0.0;
        if life > 0.0 {
            flags |= B_TIMED;
            life_left = life * 30.0;
        }
        let owner = &w.chars[src.side];
        let data = w.data[src.side];
        // The event slot of the selected node: 0, 1, 3, 4, 5 for 0..4.
        let mut nodes = *nodes;
        if let Some(&slot) = [0, 1, 3, 4, 5].get(usize::from(sel.1)) {
            nodes.set_attr_slot(owner, data, cur_slot, slot);
        }
        Self {
            id,
            src,
            tag: Tag::default(),
            flags,
            life_left,
            scale,
            speed,
            delay: 1.0,
            dir,
            target: [0.0; 3],
            start: pos,
            pos,
            prev: owner.obj.node_pos(0x11),
            vel: scale3(dir, speed),
            head_set: *set,
            trail_width: set.width[0],
            trail_time: 0.0,
            nodes,
            sel: [sel, (0, 0)],
            sel_count: 1,
            emit: emit_state(data, src.slot),
        }
    }

    /// `EftBlastObj_Update`: moves (to the owner's target when placed, else homing and by its velocity) unless
    /// held, stuck or frozen; the life and ABORT stop it; a stopped shot publishes no record and dies on the next
    /// update. Returns the record.
    fn update(
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
        self.nodes
            .update(&w.chars[side], w.data[side], &def, cur_slot, events);
        if self.delay <= 0.0 && def.flags & 2 == 0 && self.flags & (B_STUCK | B_FROZEN) == 0 {
            self.prev = self.pos;
            if self.flags & B_PLACED != 0 {
                self.pos = self.target;
            } else {
                if 0.0 < def.homing {
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
            }
        }
        if 0.0 < self.delay {
            self.delay -= 1.0;
        }
        if self.flags & B_TIMED != 0 {
            self.life_left -= 1.0;
            if self.life_left <= 0.0 {
                self.flags |= B_STOP;
            }
        }
        if events & EV_ABORT != 0 {
            self.flags |= B_STOP;
        }
        // EftBlastObj_UpdateParts: the emitters of its (phase, node slot) pairs, or of its phases on slot 5 (the
        // model is not ported).
        if let Some(st) = &mut self.emit {
            let sel = &self.sel[..self.sel_count];
            update_parts(
                fx,
                w,
                &self.src,
                st,
                &self.nodes,
                events,
                self.flags & B_STOP != 0,
                false,
                self.pos,
                self.dir,
                self.scale,
                |set, kind, sub| {
                    let Some(g) = set.groups[usize::from(kind)] else {
                        return true;
                    };
                    let p = &set.parts[g.first + sub];
                    !sel.iter()
                        .any(|&(k, n)| p.phase() == k && (p.node() == n || p.node() == 5))
                },
            );
        }
        if self.flags & B_DYING != 0 {
            self.flags &= !B_ALIVE;
            return None;
        }
        if self.flags & B_STOP != 0 {
            self.flags |= B_DYING;
            return None;
        }
        if self.flags & B_NO_HIT != 0 {
            return None;
        }
        update_trail_width(&mut self.trail_width, &mut self.trail_time, &self.head_set);
        // EftBlastObj_AddHit and EftHit_Add (nothing while time is stopped).
        if w.paused {
            return None;
        }
        let mut rec = HitRec::technique(
            self.src,
            self.id,
            self.pos,
            self.prev,
            self.vel,
            self.start,
            self.scale * self.trail_width,
        )?;
        if self.flags & B_HELD != 0 {
            rec.flags |= 0x80;
        }
        if self.flags & B_LAST != 0 {
            rec.flags |= 0x10;
        }
        // EftHit_InitMultiHit.
        if def.max_hits > 0 {
            self.tag.flags |= T_MULTI;
            self.tag.flags &= !T_HIT_CHAR;
        }
        Some(rec)
    }

    /// `EftBlastObj_PostUpdate`: result bit 0 puts it where the pass stopped it, for good; bit 2 stops it.
    fn post_update(&mut self, w: &World, fx: &mut Modules) {
        if stopped(w, self.src.side) {
            return;
        }
        if let Some(st) = &mut self.emit {
            parts_alive(fx, w.data[self.src.side], &self.src, st, false);
        }
        if self.tag.result & 1 != 0 {
            self.pos = self.tag.pos;
            self.flags |= B_STUCK;
        }
        if self.tag.result & 4 != 0 {
            self.flags |= B_STOP;
        }
    }
}

/// The blast objects of the scene (layer 3), in creation order.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shots {
    list: Vec<Shot>,
    /// The fighters' current technique slots (`EftShot_GetCurSlot`), for the shots' nodes.
    pub cur_slot: [i32; 2],
}

impl Shots {
    fn get(&mut self, id: Option<u32>) -> Option<&mut Shot> {
        let id = id?;
        self.list
            .iter_mut()
            .find(|s| s.id == id && s.flags & B_ALIVE != 0)
    }

    /// `EftBlastObj_Create` for another module (`EftBlastObj_Init`); `None` when the list is full.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        &mut self,
        next_id: &mut u32,
        src: TechSrc,
        w: &World,
        set: &SetHead,
        nodes: &EmitNodes,
        sel: (u8, u8),
        pos: [f32; 3],
        dir: [f32; 3],
        life: f32,
        scale: f32,
        speed: f32,
    ) -> Option<u32> {
        if self.list.len() >= SHOTS_MAX {
            return None;
        }
        *next_id += 1;
        let cur_slot = self.cur_slot[src.side];
        self.list.push(Shot::new(
            *next_id, src, w, cur_slot, set, nodes, sel, pos, dir, life, scale, speed,
        ));
        Some(*next_id)
    }

    /// `EftBlastObj_SetTarget`: placed at `pos` each update that moves it, or flying again.
    pub(crate) fn set_target(&mut self, id: Option<u32>, pos: Option<[f32; 3]>) {
        if let Some(s) = self.get(id) {
            match pos {
                Some(p) => {
                    s.flags |= B_PLACED;
                    s.target = p;
                }
                None => s.flags &= !B_PLACED,
            }
        }
    }

    /// `EftBlastObj_SetDir`: the direction only (the velocity follows only while homing).
    pub(crate) fn set_dir(&mut self, id: Option<u32>, dir: [f32; 3]) {
        if let Some(s) = self.get(id) {
            s.dir = dir;
        }
    }

    /// `EftBlastObj_SetDelay`.
    pub(crate) fn set_delay(&mut self, id: Option<u32>, frames: f32) {
        if let Some(s) = self.get(id) {
            s.delay = frames;
        }
    }

    /// `EftBlastObj_SetHeld`: its records do not meet the fighters.
    pub(crate) fn set_held(&mut self, id: Option<u32>, on: bool) {
        if let Some(s) = self.get(id) {
            if on {
                s.flags |= B_HELD;
            } else {
                s.flags &= !B_HELD;
            }
        }
    }

    /// `EftBlastObj_SetNoHit`: it publishes no record.
    pub(crate) fn set_no_hit(&mut self, id: Option<u32>, on: bool) {
        if let Some(s) = self.get(id) {
            if on {
                s.flags |= B_NO_HIT;
            } else {
                s.flags &= !B_NO_HIT;
            }
        }
    }

    /// `EftBlastObj_SetFrozen`: it does not move.
    pub(crate) fn set_frozen(&mut self, id: Option<u32>, on: bool) {
        if let Some(s) = self.get(id) {
            if on {
                s.flags |= B_FROZEN;
            } else {
                s.flags &= !B_FROZEN;
            }
        }
    }

    /// `EftBlastObj_Stop`: no more records; it dies on the update after the next.
    pub(crate) fn stop(&mut self, id: Option<u32>) {
        if let Some(s) = self.get(id) {
            s.flags |= B_STOP;
        }
    }

    /// `EftBlastObj_SetPrevPos`.
    pub(crate) fn set_prev(&mut self, id: Option<u32>, pos: [f32; 3]) {
        if let Some(s) = self.get(id) {
            s.prev = pos;
        }
    }

    /// `EftBlastObj_MarkLast`.
    pub(crate) fn mark_last(&mut self, id: Option<u32>) {
        if let Some(s) = self.get(id) {
            s.flags |= B_LAST;
        }
    }

    /// A fighter's blast objects reset (`EftBlastObj_Reset`).
    pub(crate) fn reset_side(&mut self, fx: &mut Modules, side: usize) {
        self.list.retain_mut(|s| {
            if s.src.side != side {
                return true;
            }
            if let Some(st) = &mut s.emit {
                st.kill_all(fx);
            }
            false
        });
    }

    /// `EftBlastObj_IsAlive`.
    pub(crate) fn alive(&self, id: Option<u32>) -> bool {
        id.is_some_and(|id| {
            self.list
                .iter()
                .any(|s| s.id == id && s.flags & B_ALIVE != 0)
        })
    }

    pub(crate) fn tag_mut(&mut self, id: u32) -> Option<&mut Tag> {
        self.list
            .iter_mut()
            .find(|s| s.id == id)
            .map(|s| &mut s.tag)
    }

    /// The blast objects' updates: each one's record; a dead one is removed.
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        events: [u32; 2],
        records: &mut Vec<HitRec>,
        max: usize,
    ) {
        let mut i = 0;
        while i < self.list.len() {
            let s = &mut self.list[i];
            let side = s.src.side;
            if let Some(rec) = s.update(w, fx, events[side], self.cur_slot[side])
                && records.len() < max
            {
                records.push(rec);
            }
            if s.flags & B_ALIVE == 0 {
                self.list.remove(i);
            } else {
                i += 1;
            }
        }
    }

    pub(crate) fn post_update(&mut self, w: &World, fx: &mut Modules) {
        for s in &mut self.list {
            s.post_update(w, fx);
        }
    }

    /// The live shots' positions of a fighter, for tests.
    #[cfg(test)]
    pub(crate) fn of(&self, side: usize) -> Vec<[f32; 3]> {
        self.list
            .iter()
            .filter(|s| s.src.side == side)
            .map(|s| s.pos)
            .collect()
    }
}

// ---- the volley (eft_emit.c) --------------------------------------------------------------------------------

// EFT_VOLLEY_*.
const V_ENDING: u32 = 0x1;
const V_DEAD: u32 = 0x2;
const V_4: u32 = 0x4;
const V_8: u32 = 0x8;
const V_AIMED2: u32 = 0x10;
const V_AIMED: u32 = 0x40;
const V_FIRED: u32 = 0x100;
const V_ID1C6: u32 = 0x200;

/// `EftVolleyShot`: a fired shot's entry.
#[derive(Clone, Copy, Debug, Default)]
struct VolleyShot {
    aim: AimShot,
    /// 1 fired, 2 the last of its motion, 4 steered.
    flags: u32,
    time: f32,
    handle: Option<u32>,
}

/// An `EftVolley` item.
#[derive(Clone, Debug)]
pub(crate) struct Volley {
    src: TechSrc,
    flags: u32,
    state: u8,
    speed: f32,
    homing: f32,
    scale: f32,
    end_time: f32,
    end_limit: f32,
    aim: [f32; 3],
    /// The emitters' direction: the aim, straight up (aim kind 7) or lifted 45 degrees (aim kind 8).
    dir: [f32; 3],
    pos: [f32; 3],
    nodes: EmitNodes,
    emit: Option<EmitState>,
    shots: [VolleyShot; VOLLEY_SHOTS],
    fired: i32,
    volleys: i32,
    node_side: i32,
    head_set: SetHead,
    /// Killed: the item is removed (`BtlTask_SetDead`).
    dead: bool,
}

impl Volley {
    /// `EftVolley_Init`; `None` for techniques 0x26E / 0x26F (their shots follow a model animation).
    pub(crate) fn new(src: TechSrc, data: &SideData, set: &EmitSet) -> Option<Self> {
        let def = src.def;
        if matches!(def.id, 0x26E | 0x26F) {
            return None;
        }
        Some(Self {
            src,
            flags: if def.id == 0x1C6 { V_ID1C6 } else { 0 },
            state: 0,
            speed: def.speed,
            homing: def.homing,
            scale: def.scale,
            end_time: 0.0,
            end_limit: f32::from(set.head.end_frames),
            aim: [0.0; 3],
            dir: [0.0; 3],
            pos: [0.0; 3],
            nodes: EmitNodes::default(),
            emit: emit_state(data, src.slot),
            shots: [VolleyShot::default(); VOLLEY_SHOTS],
            fired: 0,
            volleys: 0,
            node_side: 0,
            head_set: set.head,
            dead: false,
        })
    }

    pub(crate) fn src(&self) -> &TechSrc {
        &self.src
    }

    /// Whether the item is dead (its end raises flag 0xA8).
    pub(crate) fn dead(&self) -> bool {
        self.dead
    }

    /// `EftVolley_Reset`: its emitters' objects die.
    pub(crate) fn kill_parts(&mut self, fx: &mut Modules) {
        if let Some(st) = &mut self.emit {
            st.kill_all(fx);
        }
    }

    /// `EftVolley_Aim`: the aim from `from` (keeping the facing with `by_slot`), and the emitters' direction by
    /// the set's aim kind.
    fn set_aim(&mut self, w: &World, from: [f32; 3], by_slot: bool) {
        let side = self.src.side;
        self.aim = if by_slot {
            aim_dir_keep(w, side, from)
        } else {
            aim_dir(w, side, from)
        };
        self.dir = self.aim;
        match self.head_set.aim_kind {
            7 => self.dir = [0.0, -1.0, 0.0],
            8 => {
                let d = self.dir.map(|v| v.clamp(-1.0, 1.0));
                let pitch = eft_wrap(asin_c(-d[1]) + PI_F / 4.0);
                let yaw = eft_wrap(d[0].atan2(d[2]));
                let m = rotate_xyz(&IDENTITY, [pitch, 0.0, 0.0]);
                let m = rotate_xyz(&m, [0.0, yaw, 0.0]);
                let v = apply(&m, [0.0, 0.0, 1.0, 1.0]);
                self.dir = norm([v[0], v[1], v[2]]);
            }
            _ => {}
        }
    }

    /// `EftVolley_Update`: the aim on START (or FIRE), the firing position on FIRE, then the shots; ends when its
    /// shots are gone after the last firing, on the END, or at once on ABORT; dies once ended and empty.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        events: u32,
        cur_slot: i32,
        requests: &mut u32,
        shots: &mut Shots,
        next_id: &mut u32,
        rand: &mut u32,
    ) {
        let side = self.src.side;
        if stopped(w, side) {
            return;
        }
        let tables = w.env.fx;
        let def = self.src.def;
        let test = |b: u32| events & b != 0;
        self.nodes
            .update(&w.chars[side], w.data[side], &def, cur_slot, events);
        if self.flags & V_AIMED == 0 {
            if test(EV_START) {
                self.set_aim(w, self.nodes.pos(0), false);
                self.state = 0;
                self.flags |= V_AIMED;
            } else if test(EV_FIRE) {
                self.state = 0;
                self.flags |= V_AIMED;
            }
        }
        let mut next = false;
        if self.flags & V_AIMED != 0 {
            match self.state {
                0 => {
                    if test(EV_FIRE) {
                        self.flags |= V_AIMED2;
                        self.pos = self.nodes.pos(1);
                        self.set_aim(w, self.pos, true);
                        self.state = 1;
                    }
                }
                1 | 3 => next = true,
                2 => {
                    if test(EV_FIRE) && def.flags & 0x800 != 0 {
                        self.pos = self.nodes.pos(1);
                        self.set_aim(w, self.pos, true);
                    }
                    if test(EV_END) {
                        self.flags |= V_ENDING | V_8;
                        self.state = 3;
                    }
                }
                _ => {}
            }
        }
        let alive = self.update_shots(w, tables, events, requests, shots, next_id, rand);
        if !alive && self.flags & V_FIRED != 0 && self.volleys >= def.shots {
            self.flags |= V_ENDING;
        }
        if test(EV_ABORT) {
            if self.flags & V_8 == 0 {
                self.flags |= V_4;
            }
            self.flags |= V_ENDING;
        }
        // EftVolley_UpdateParts: the shots' emitters are theirs.
        if let Some(st) = &mut self.emit {
            let (ending, kill) = (self.flags & V_ENDING != 0, self.flags & V_4 != 0);
            update_parts(
                fx,
                w,
                &self.src,
                st,
                &self.nodes,
                events,
                ending,
                kill,
                self.pos,
                self.dir,
                self.scale,
                is_part_deferred,
            );
        }
        if next {
            self.state += 1;
        }
        if self.flags & V_ENDING != 0 {
            self.end_time += 1.0;
        }
        if !alive && self.flags & V_DEAD != 0 {
            self.dead = true;
        } else if self.flags & V_ENDING != 0
            && (self.flags & V_4 != 0 || self.end_time >= self.end_limit)
        {
            self.flags |= V_DEAD;
        }
        // The trail width of its own state (definition flag 1) is drawing only.
    }

    /// `EftVolley_UpdateShots`: this frame's event fires (START phase 0, FIRE phase 1 restarting the end timer,
    /// 0x10 / 0x20 / 0x40 phases 3..5) when the set has emitters of the phase; then the live shots are steered
    /// and placed, and stopped once the volley ends. Whether any shot is alive.
    #[allow(clippy::too_many_arguments)]
    fn update_shots(
        &mut self,
        w: &World,
        tables: &FxTables,
        events: u32,
        requests: &mut u32,
        shots: &mut Shots,
        next_id: &mut u32,
        rand: &mut u32,
    ) -> bool {
        let side = self.src.side;
        // The phase and the event slot whose emitters the shot drives.
        let (mut phase, mut slot) = (-1, 0);
        if events & EV_START != 0 {
            phase = 0;
        } else if events & EV_FIRE != 0 {
            (phase, slot) = (1, 1);
            // EftTechEvt_RequestRestart.
            if events & EV_END == 0 {
                *requests |= 4;
            }
        } else if events & 0x10 != 0 {
            (phase, slot) = (3, 2);
        } else if events & 0x20 != 0 {
            (phase, slot) = (4, 3);
        } else if events & 0x40 != 0 {
            (phase, slot) = (5, 4);
        }
        let mut rng = Rng {
            state: rand,
            ts: w.time_stopped,
        };
        if phase >= 0 && (self.head_set.phase_mask >> phase) & 1 != 0 {
            self.fire(w, tables, &mut rng, phase, slot, shots, next_id);
        }
        for i in 0..VOLLEY_SHOTS {
            let shot = &mut self.shots[i];
            if !shots.alive(shot.handle) {
                continue;
            }
            let exact = shot.flags & 2 != 0;
            if shot.flags & 4 != 0 {
                let (time, speed, homing) = (shot.time, self.speed, self.homing);
                aim_update(
                    w,
                    tables,
                    &mut rng,
                    side,
                    &mut shot.aim,
                    exact,
                    time,
                    speed,
                    homing,
                );
                // EftBlastObj_SetTarget.
                if let Some(s) = shots.get(shot.handle) {
                    s.flags |= B_PLACED;
                    s.target = shot.aim.pos;
                }
            }
            if self.flags & V_ENDING != 0
                && let Some(s) = shots.get(shot.handle)
            {
                s.flags |= B_STOP;
            }
            shot.time += 1.0;
        }
        self.prune(shots)
    }

    /// `EftVolley_PruneShots`: the entries of dead shots are freed; whether any is alive.
    fn prune(&mut self, shots: &Shots) -> bool {
        let mut alive = false;
        for shot in &mut self.shots {
            if shots.alive(shot.handle) {
                alive = true;
            } else {
                shot.handle = None;
            }
        }
        alive
    }

    /// `EftVolley_PostUpdate`.
    pub(crate) fn post_update(&mut self, w: &World, fx: &mut Modules, shots: &Shots) {
        if stopped(w, self.src.side) {
            return;
        }
        if let Some(st) = &mut self.emit {
            parts_alive(fx, w.data[self.src.side], &self.src, st, false);
        }
        self.prune(shots);
    }

    /// `EftVolley_Fire`: one shot (the definition's volley of them in phase 1) at the firing node (technique 0x1C6:
    /// nodes 0x1F and 0x2D) along the aim; phase 1's shots live the definition's time, spread round the aim and
    /// steered (the last of the motion marked as the technique's last hit); the other phases' shots are frozen
    /// and harmless.
    #[allow(clippy::too_many_arguments)]
    fn fire(
        &mut self,
        w: &World,
        tables: &FxTables,
        rng: &mut Rng,
        phase: i32,
        slot: u8,
        shots: &mut Shots,
        next_id: &mut u32,
    ) {
        let side = self.src.side;
        let def = self.src.def;
        let chr = &w.chars[side];
        let n = if phase == 1 { def.volley.max(1) } else { 1 };
        for i in 0..n {
            // EftVolley_AllocShot: the last free entry.
            let Some(k) = (0..VOLLEY_SHOTS)
                .rev()
                .find(|&k| self.shots[k].handle.is_none())
            else {
                continue;
            };
            self.shots[k] = VolleyShot::default();
            let count = def.shots;
            let node = if self.flags & V_ID1C6 != 0 {
                self.shots[k].flags |= 2;
                if i == 0 { 0x1F } else { 0x2D }
            } else {
                if chr.obj.query_event(&w.data[side].obj, 0x400, 0, 4) <= 0 {
                    self.shots[k].flags |= 2;
                }
                self.nodes.node(1)
            };
            self.shots[k].aim.pos = chr.obj.node_pos(node as u32);
            self.shots[k].aim.dir = self.aim;
            let mut life = 0.0;
            if phase == 1 {
                life = def.life as f32 / 30.0;
                if self.flags & V_ID1C6 == 0 {
                    if self.shots[k].flags & 4 == 0 {
                        self.node_side = node_side(self.nodes.node(1));
                    }
                    self.shots[k].flags |= 4;
                    // EftVolleyAim_InitShot.
                    let a = &mut self.shots[k].aim;
                    a.script = spread(
                        w,
                        &tables.sin_coef,
                        rng,
                        side,
                        &mut a.dir,
                        i32::from(self.head_set.aim_kind),
                        self.node_side,
                        self.fired,
                        count,
                    );
                    a.speed = self.speed;
                    a.max_turn = self.homing;
                }
            }
            // EftBlastObj_Create: nothing when the list is full.
            if shots.list.len() >= SHOTS_MAX {
                continue;
            }
            *next_id += 1;
            let a = self.shots[k].aim;
            // Technique 0x1C6's shots leave from their own node (`shotPose`).
            let mut nodes = self.nodes;
            if self.flags & V_ID1C6 != 0 {
                nodes.set_slot_node(chr, 1, node);
            }
            let mut shot = Shot::new(
                *next_id,
                self.src,
                w,
                shots.cur_slot[side],
                &self.head_set,
                &nodes,
                (phase as u8, slot),
                a.pos,
                a.dir,
                life,
                self.scale,
                self.speed,
            );
            self.shots[k].handle = Some(shot.id);
            self.shots[k].flags |= 1;
            if phase == 1 {
                if self.shots[k].flags & 2 != 0 {
                    shot.flags |= B_LAST;
                }
            } else {
                shot.flags |= B_NO_HIT | B_FROZEN;
            }
            shots.list.push(shot);
            self.flags |= V_FIRED;
            self.fired += 1;
            if phase == 1 {
                self.volleys += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_the_fast_sine_follow_the_original() {
        assert!((eft_wrap(3.5) - (3.5 - 6.283_185_3)).abs() < 1e-6);
        assert!((eft_wrap(-10.0) - (-10.0 + 2.0 * 6.283_185_3)).abs() < 1e-5);
        assert_eq!(eft_wrap(1.0), 1.0);
        assert!((mathf_wrap(4.0) - (4.0 - 2.0 * PI_F)).abs() < 1e-5);
        // The plain Taylor coefficients give the sine closely.
        let c = [1.0 / 362_880.0, -1.0 / 5040.0, 1.0 / 120.0, -1.0 / 6.0];
        for a in [-3.0f32, -1.0, 0.0, 0.5, 2.0, 7.0] {
            assert!((sin_fast(&c, a) - a.sin()).abs() < 0.01, "{a}");
        }
    }

    #[test]
    fn the_scene_generator_steps_like_the_fighters_one() {
        let mut s = 0;
        assert_eq!(scene_rand(&mut s, false), 4096);
        assert_eq!(s, 4096);
        scene_rand(&mut s, true);
        assert_eq!(s, 4096);
        let v = rand_range(&mut s, false, 3, 3);
        assert_eq!(v, 3);
        assert_eq!(s, 4096);
    }
}
