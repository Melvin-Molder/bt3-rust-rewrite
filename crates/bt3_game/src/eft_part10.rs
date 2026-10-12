//! Effect pack part kind 10, the ring emitter (`EftPart10_*`, src/battle/eft_link_2.c and eft_part10.c): it keeps
//! rings of particles filled round its position (a ring per angle slot between two fractions of a sphere), each
//! particle flying out from its start distance, spinning, sized, coloured and faded over its life, drawn as a
//! camera-facing sprite or a turned quad. Particles and rings come from pools of 150 shared by every emitter; the
//! particles draw from the VU0 generator (and libc `rand()` for coin flips and the sheet frame).

use bt3_data::eft_pack::{Part10Def, Part10Def2};

use crate::battle::{IDENTITY, Mtx, apply, inverse_rt};
use crate::eft::{
    ClipVtx, DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, Prim,
    SCREEN_DIST, SetRef, V3, V4, ZSort, add, depth_slot, mtx_mul, norm, point, queue_strip,
    queue_tri_clip, rotate_xyz, scale3, xyz,
};
use crate::rng::{LibcRandom, Vu0Random};

/// `EftPart10Mgr_Init`: twelve emitters.
pub(crate) const EMITTERS: usize = 12;
/// `EFT_PART10_MAX` / `EFT_PART10_GRPS`: the particle and ring pools.
const POOL: usize = 150;

const PI_F: f32 = 3.141_592_7;
const TWO_PI: f32 = 6.283_185_3;

// EFT_PART10_*.
const F_ALIVE: u32 = 0x1;
const F_DONE: u32 = 0x2;
const F_NOEMIT: u32 = 0x4;
const F_HOLD: u32 = 0x8;
const F_FADE: u32 = 0x10;
pub(crate) const F_KILL: u32 = 0x20;
const F_TEXGRID: u32 = 0x100;
pub(crate) const F_NODEPTH: u32 = 0x400;
const F_KEY2: u32 = 0x2_0000;
const F_KEYANIM: u32 = 0x4_0000;
const F_STOPPED: u32 = 0x8_0000;

// EFT_PART10_P_*.
const P_ALIVE: u32 = 0x1;
const P_DRAW: u32 = 0x40;
const P_COLOR_RAMP: u32 = 0x80;
const P_MIRROR: u32 = 0x200;
const P_PULSE_DOWN: u32 = 0x1000;
const P_MUL: [u32; 3] = [0x2000, 0x4000, 0x8000];
const P_MUL_DOWN: u32 = 0x1_0000;

/// A ring refilled this frame.
const G_FILLED: u32 = 0x800;

/// `EftMath_WrapAngle`.
fn wrap(a: f32) -> f32 {
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

/// `Mathf_Asin`: the argument clamped.
fn asin_c(x: f32) -> f32 {
    x.clamp(-1.0, 1.0).asin()
}

fn clamp01(x: f32) -> f32 {
    if x < 0.0 {
        0.0
    } else if 1.0 < x {
        1.0
    } else {
        x
    }
}

/// The shared pools (`gEftPart10Mgr`): rings and particles in use, and where each search starts.
#[derive(Clone, Debug)]
pub(crate) struct Pools {
    grp_used: Vec<bool>,
    grp_next: usize,
    ptcl_used: Vec<bool>,
    ptcl_next: usize,
}

impl Default for Pools {
    fn default() -> Self {
        Self {
            grp_used: vec![false; POOL],
            grp_next: 0,
            ptcl_used: vec![false; POOL],
            ptcl_next: 0,
        }
    }
}

impl Pools {
    /// Particles in use.
    #[cfg(test)]
    pub(crate) fn particles(&self) -> usize {
        self.ptcl_used.iter().filter(|&&u| u).count()
    }
}

// The one-value tracks of the definitions, in the emitter's order.
const T_ANGLE: usize = 0;
const T_ANGLE_R: usize = 1;
const T_SPIN_X: usize = 2;
const T_SPIN_X_R: usize = 3;
const T_SPIN_Y: usize = 4;
const T_SPIN_Y_R: usize = 5;
const T_SPIN: usize = 6;
const T_SPIN_R: usize = 7;
const T_ANG_A: usize = 8;
const T_ANG_A_R: usize = 9;
const T_ANG_B: usize = 10;
const T_ANG_B_R: usize = 11;
const T_DIST: usize = 12;
const T_DIST_R: usize = 13;
const T_DIST_BASE: usize = 14;
const T_DIST_VEL: usize = 15;
const T_DIST_VEL_R: usize = 16;
const T_SCALE_X: usize = 17;
const T_SCALE_Y: usize = 18;
const T_LIFE: usize = 19;
const T_LIFE_R: usize = 20;
const T_WAIT: usize = 21;
const T_WAIT_R: usize = 22;
const T_PULSE_TIME: usize = 23;
const T_MUL_TIME: usize = 24;
const T_FADE0: usize = 25;
const T_FADE1: usize = 26;
/// The six size values (birth / middle / death, minimum and range), the pulse pair and the three colour
/// multiplier pairs are tracks too.
const T_SIZE: usize = 27;
const T_SIZE_R: usize = 30;
const T_PULSE: usize = 33;
const T_MUL: usize = 35;
const TRACKS: usize = 41;

/// The three keys of track `t`.
fn track(def: &Part10Def, def2: &Part10Def2, t: usize) -> [f32; 3] {
    match t {
        T_ANGLE => def.angle,
        T_ANGLE_R => def.angle_range,
        T_SPIN_X => def.spin_x,
        T_SPIN_X_R => def.spin_x_range,
        T_SPIN_Y => def.spin_y,
        T_SPIN_Y_R => def.spin_y_range,
        T_SPIN => def.spin,
        T_SPIN_R => def.spin_range,
        T_ANG_A => def.ang_a,
        T_ANG_A_R => def.ang_a_range,
        T_ANG_B => def.ang_b,
        T_ANG_B_R => def.ang_b_range,
        T_DIST => def.dist,
        T_DIST_R => def.dist_range,
        T_DIST_BASE => def.dist_base,
        T_DIST_VEL => def.dist_vel,
        T_DIST_VEL_R => def.dist_vel_range,
        T_SCALE_X => def.scale_x,
        T_SCALE_Y => def.scale_y,
        T_LIFE => def.life,
        T_LIFE_R => def.life_range,
        T_WAIT => def.wait,
        T_WAIT_R => def.wait_range,
        T_PULSE_TIME => def.pulse_time,
        T_MUL_TIME => def2.mul_time,
        T_FADE0 => def2.fade0,
        T_FADE1 => def2.fade1,
        t if (T_SIZE..T_SIZE + 3).contains(&t) => std::array::from_fn(|k| def.size[k][t - T_SIZE]),
        t if (T_SIZE_R..T_SIZE_R + 3).contains(&t) => {
            std::array::from_fn(|k| def.size_range[k][t - T_SIZE_R])
        }
        t if (T_PULSE..T_PULSE + 2).contains(&t) => {
            std::array::from_fn(|k| def.pulse[k][t - T_PULSE])
        }
        t => {
            let (c, i) = ((t - T_MUL) / 2, (t - T_MUL) % 2);
            std::array::from_fn(|k| def2.mul[c][k][i])
        }
    }
}

/// The four colour tracks.
fn color_track(def2: &Part10Def2, c: usize) -> &[[f32; 4]; 3] {
    match c {
        0 => &def2.color,
        1 => &def2.color_range,
        2 => &def2.end_color,
        _ => &def2.end_color_range,
    }
}

/// `EftPart10Ptcl`.
#[derive(Clone, Copy, Debug, Default)]
struct Ptcl {
    slot: usize,
    pos: V3,
    base: V3,
    /// The particle's offset (`axisScale`, x and y).
    offset: [f32; 2],
    col0: V4,
    col_delta: V3,
    col: V4,
    corner: [V3; 4],
    uv0: V4,
    uv1: V4,
    dist: f32,
    dist_vel: f32,
    ang: V3,
    ang_vel: V3,
    size: f32,
    size_vel0: f32,
    size_vel1: f32,
    fade_in: f32,
    fade_out: f32,
    /// The size pulse's factor.
    scale: f32,
    pulse0: f32,
    pulse_t: f32,
    pulse_time: f32,
    mul_d: [f32; 3],
    mul0: [f32; 3],
    mul_t: f32,
    mul_time: f32,
    fade_t: f32,
    fade_time: f32,
    pulse_d: f32,
    tex_frame: f32,
    delay: f32,
    life: f32,
    age: f32,
    flags: u32,
}

/// `EftPart10Grp`: one ring's particles.
#[derive(Clone, Debug, Default)]
struct Group {
    slot: usize,
    flags: u32,
    ptcls: Vec<Ptcl>,
}

/// An emitter task (`EftPart10`).
#[derive(Clone, Debug)]
pub(crate) struct Part10 {
    pub id: u32,
    chr: usize,
    pub set: SetRef,
    pub pair: usize,
    texture: Option<usize>,
    pos: V3,
    dir: V3,
    size: f32,
    /// Seconds it runs before it stops by itself; <= 0 until stopped.
    life: f32,
    /// The tracks' current values and their change over the key segment.
    v: [f32; TRACKS],
    d: [f32; TRACKS],
    color: [V4; 4],
    d_color: [V4; 4],
    key_time: f32,
    key_mid: f32,
    spin_a: f32,
    spin_b: f32,
    rot_a: f32,
    rot_b: f32,
    scale: f32,
    count: f32,
    d_spin_a: [f32; 2],
    d_spin_b: [f32; 2],
    d_scale: [f32; 2],
    d_count: [f32; 2],
    twist: f32,
    yaw: f32,
    pitch: f32,
    uv: [[f32; 4]; 16],
    tex_frames: f32,
    frame: f32,
    pub delay: f32,
    hold: f32,
    fade: f32,
    fade_time: f32,
    spin_time: f32,
    spin_mid: f32,
    pub flags: u32,
    groups: Vec<Group>,
}

impl Part10 {
    /// `EftPart10_Init`: the emitter spin (flag 0x40, VU0 draws), the key animation (flag 0x200) or key 2, the
    /// sheet's cells.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u32,
        chr: usize,
        set: SetRef,
        pair: usize,
        texture: Option<usize>,
        pos: V3,
        dir: V3,
        size: f32,
        life: f32,
        def: &Part10Def,
        def2: &Part10Def2,
        vu0: &mut Vu0Random,
    ) -> Self {
        let mut w = Self {
            id,
            chr,
            set,
            pair,
            texture,
            pos,
            dir,
            size,
            life,
            v: [0.0; TRACKS],
            d: [0.0; TRACKS],
            color: [[0.0; 4]; 4],
            d_color: [[0.0; 4]; 4],
            key_time: 0.0,
            key_mid: 0.0,
            spin_a: 0.0,
            spin_b: 0.0,
            rot_a: 0.0,
            rot_b: 0.0,
            scale: 0.0,
            count: 0.0,
            d_spin_a: [0.0; 2],
            d_spin_b: [0.0; 2],
            d_scale: [0.0; 2],
            d_count: [0.0; 2],
            twist: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            uv: [[0.0; 4]; 16],
            tex_frames: 0.0,
            frame: 0.0,
            delay: 0.0,
            hold: 0.0,
            fade: 0.0,
            fade_time: 0.0,
            spin_time: 0.0,
            spin_mid: 0.0,
            flags: 0,
            groups: Vec::new(),
        };
        if def.flags & 0x40 != 0 {
            w.init_spin(def, vu0);
        } else {
            w.spin_a = 0.0;
            w.scale = 1.0;
            w.spin_b = 0.0;
            w.count = def.count[0];
        }
        if def.flags & 0x200 != 0 {
            w.start_keys(def, def2);
            w.flags |= F_KEYANIM;
        } else {
            w.set_key(def, def2, 2);
        }
        if def.cols >= 2 || def.rows >= 2 {
            w.tex_frames = f32::from(def.cols) * f32::from(def.rows);
            let step = [1.0 / f32::from(def.cols), 1.0 / f32::from(def.rows)];
            let mut n = 0;
            for j in 0..def.rows {
                for i in 0..def.cols {
                    // The sheet has room for 16 cells.
                    if let Some(c) = w.uv.get_mut(n) {
                        let (fi, fj) = (f32::from(i), f32::from(j));
                        *c = [
                            step[0] * fi,
                            step[1] * fj,
                            step[0] * fi + step[0],
                            step[1] * fj + step[1],
                        ];
                    }
                    n += 1;
                }
            }
            w.flags |= F_TEXGRID;
        }
        w.pitch = wrap(def.pitch * PI_F);
        w.yaw = wrap(def.yaw * PI_F);
        w.flags |= 0x41;
        w
    }

    pub(crate) fn alive(&self) -> bool {
        self.flags & F_ALIVE != 0
    }

    pub(crate) fn chr(&self) -> usize {
        self.chr
    }

    /// `EftPart10_Stop`: it keeps emitting for the hold, fades for the fade, then stops emitting.
    pub(crate) fn stop(&mut self) {
        if self.flags & F_STOPPED != 0 || self.flags & F_ALIVE == 0 {
            return;
        }
        let mut now = true;
        if 0.0 < self.hold {
            self.flags |= F_HOLD;
            now = false;
        }
        if 0.0 < self.fade_time {
            self.flags |= F_FADE;
            now = false;
        }
        if now {
            self.flags |= F_NOEMIT;
        }
        self.flags |= F_STOPPED;
    }

    /// `EftPart10_SetFade`.
    pub(crate) fn set_fade(&mut self, frames: f32) {
        if self.flags & F_STOPPED == 0 && self.alive() {
            self.fade_time = frames;
            self.fade = frames;
        }
    }

    /// `EftPart10_SetHold`.
    pub(crate) fn set_hold(&mut self, frames: f32) {
        if self.alive() {
            self.hold = frames;
        }
    }

    /// `EftPart10_SetPos`: particles already out stay where they are.
    pub(crate) fn set_pos(&mut self, pos: V3) {
        if self.alive() {
            self.pos = pos;
        }
    }

    /// `EftPart10_Warp`: the emitter, and every particle's corners moved by the new position.
    pub(crate) fn warp(&mut self, pos: V3) {
        if !self.alive() {
            return;
        }
        self.pos = pos;
        for g in &mut self.groups {
            for p in &mut g.ptcls {
                for c in &mut p.corner {
                    *c = add(*c, pos);
                }
            }
        }
    }

    /// `EftPart10_SetDir`.
    pub(crate) fn set_dir(&mut self, dir: V3) {
        if self.alive() {
            self.dir = dir;
        }
    }

    /// `EftPart10_SetSize`.
    pub(crate) fn set_size(&mut self, size: f32) {
        if self.alive() {
            self.size = size;
        }
    }

    /// `EftPart10_Term`: its particles and rings back to the pools.
    pub(crate) fn free(&mut self, pools: &mut Pools) {
        for g in self.groups.drain(..) {
            for p in &g.ptcls {
                pools.ptcl_used[p.slot] = false;
            }
            pools.grp_used[g.slot] = false;
        }
        self.flags = 0;
    }

    /// `EftPart10_InitSpin`: one random spin and scale (flag 0x20), or three random keys of each turned into steps
    /// before and after the middle.
    fn init_spin(&mut self, def: &Part10Def, vu0: &mut Vu0Random) {
        let mut r = |a: f32, b: f32| vu0.float_range(a, a + b);
        if def.flags & 0x20 != 0 {
            self.spin_a = r(def.spin_a[0], def.spin_a_range[0]) * TWO_PI;
            self.spin_b = r(def.spin_b[0], def.spin_b_range[0]) * TWO_PI;
            self.scale = r(def.scale[0], def.scale_range[0]);
            self.count = def.count[0];
            return;
        }
        self.spin_time = def.spin_time * 30.0;
        self.spin_mid = self.spin_time * def.spin_mid;
        let (mid, rest) = (self.spin_mid, self.spin_time - self.spin_mid);
        let k: [f32; 3] = std::array::from_fn(|i| r(def.spin_a[i], def.spin_a_range[i]) * TWO_PI);
        self.d_spin_a = [(k[1] - k[0]) / mid, (k[2] - k[1]) / rest];
        self.spin_a = k[0];
        let k: [f32; 3] = std::array::from_fn(|i| r(def.spin_b[i], def.spin_b_range[i]) * TWO_PI);
        self.d_spin_b = [(k[1] - k[0]) / mid, (k[2] - k[1]) / rest];
        self.spin_b = k[0];
        let k: [f32; 3] = std::array::from_fn(|i| r(def.scale[i], def.scale_range[i]));
        self.d_scale = [(k[1] - k[0]) / mid, (k[2] - k[1]) / rest];
        self.scale = k[0];
        self.d_count = [
            (def.count[1] - def.count[0]) / mid,
            (def.count[2] - def.count[1]) / rest,
        ];
        self.count = def.count[0];
    }

    /// `EftPart10_UpdateSpin`: the spin, scale and count along their keys; the emitter turns by the spin.
    fn update_spin(&mut self, def: &Part10Def) {
        if def.flags & 0x20 == 0 {
            let seg = if self.frame < self.spin_mid {
                Some(0)
            } else if self.frame < self.spin_time {
                Some(1)
            } else {
                None
            };
            if let Some(s) = seg {
                self.spin_a += self.d_spin_a[s];
                self.spin_b += self.d_spin_b[s];
                self.scale += self.d_scale[s];
                self.count += self.d_count[s];
            }
        }
        self.rot_a = wrap(self.rot_a + self.spin_a);
        self.rot_b = wrap(self.rot_b + self.spin_b);
    }

    /// `EftPart10_StartKeys`: at age 0 the first segment (keys 0 to 1), once the age reaches the middle the second
    /// (keys 1 to 2); every track's change over it.
    fn start_keys(&mut self, def: &Part10Def, def2: &Part10Def2) {
        let mut seg = 0;
        if self.frame <= 0.0 {
            seg = 1;
            self.key_time = def.key_time * 30.0;
            self.key_mid = self.key_time * def.key_mid;
        }
        if self.key_mid <= self.frame && self.flags & F_KEY2 == 0 {
            seg = 2;
            self.flags |= F_KEY2;
        }
        if seg > 0 {
            for t in 0..TRACKS {
                let k = track(def, def2, t);
                self.d[t] = k[seg] - k[seg - 1];
            }
            for c in 0..4 {
                let k = color_track(def2, c);
                self.d_color[c] = std::array::from_fn(|i| k[seg][i] - k[seg - 1][i]);
            }
        }
    }

    /// `EftPart10_UpdateKeys`: every track at its segment's start key plus its change times the progress (the
    /// colours' alpha stays at the key's).
    fn update_keys(&mut self, def: &Part10Def, def2: &Part10Def2) {
        let (seg, t) = if self.flags & F_KEY2 == 0 {
            (0, self.frame / self.key_mid)
        } else {
            (
                1,
                (self.frame - self.key_mid) / (self.key_time - self.key_mid),
            )
        };
        let t = clamp01(t);
        for i in 0..TRACKS {
            self.v[i] = track(def, def2, i)[seg] + self.d[i] * t;
        }
        for c in 0..4 {
            let k = color_track(def2, c)[seg];
            let d = self.d_color[c];
            self.color[c] = [k[0] + d[0] * t, k[1] + d[1] * t, k[2] + d[2] * t, k[3]];
        }
    }

    /// `EftPart10_SetKey`: every track at key `idx`.
    fn set_key(&mut self, def: &Part10Def, def2: &Part10Def2, idx: usize) {
        for t in 0..TRACKS {
            self.v[t] = track(def, def2, t)[idx];
        }
        for c in 0..4 {
            self.color[c] = color_track(def2, c)[idx];
        }
    }

    /// `EftPart10_Update`: nothing while stopped or delayed; else the fade after a stop, the keys, the spin, the
    /// rings (until it stops emitting), the particles, the life and the hold. Returns whether the task ends.
    pub(crate) fn update(
        &mut self,
        def: &Part10Def,
        def2: &Part10Def2,
        stopped: bool,
        pools: &mut Pools,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
    ) -> bool {
        if stopped {
            return false;
        }
        if 0.0 < self.delay {
            self.delay -= 1.0;
            return false;
        }
        let mut alpha = 1.0;
        if self.flags & F_FADE != 0 && self.flags & F_HOLD == 0 {
            alpha = self.fade / self.fade_time;
            if self.fade >= 0.0 {
                self.fade -= 1.0;
            } else {
                self.flags = (self.flags & !F_ALIVE) | F_DONE;
            }
        }
        if self.flags & F_KEYANIM != 0 {
            self.update_keys(def, def2);
        }
        if def.flags & 0x40 != 0 {
            self.update_spin(def);
        }
        if self.flags & F_NOEMIT == 0 {
            self.emit_rings(def, pools, vu0, libc);
        }
        self.step(def, alpha, pools);
        self.frame += 1.0;
        if self.flags & F_KEYANIM != 0 {
            self.start_keys(def, def2);
            if self.frame >= self.key_time {
                self.set_key(def, def2, 2);
                self.flags &= !F_KEYANIM;
            }
        }
        if self.life * 30.0 <= self.frame && 0.0 < self.life {
            if 0.0 < self.fade_time {
                self.flags |= F_STOPPED | F_FADE;
            } else {
                self.flags |= F_NOEMIT;
            }
        }
        if self.flags & F_HOLD != 0 {
            if 0.0 < self.hold {
                self.hold -= 1.0;
            } else {
                self.flags &= !F_HOLD;
                if self.flags & F_FADE == 0 {
                    self.flags |= F_NOEMIT;
                }
            }
        }
        if self.groups.is_empty() && self.flags & F_NOEMIT != 0 {
            self.flags |= F_DONE;
        }
        if self.flags & (F_KILL | F_DONE) != 0 {
            self.flags &= !F_ALIVE;
            return true;
        }
        false
    }

    /// `EftPart10_EmitRings`: (count + 1) x count ring slots, those between the definition's fractions kept: a
    /// missing ring is taken from the pool, an empty one gets a particle at the slot's angles plus random offsets
    /// (two VU0 draws).
    fn emit_rings(
        &mut self,
        def: &Part10Def,
        pools: &mut Pools,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
    ) {
        let step = 1.0 / self.count * TWO_PI;
        let lim = [
            (self.count * def.ring_from) as i32,
            (self.count * def.ring_to) as i32,
        ];
        self.twist = 0.0;
        let n = self.count as i32;
        let mut gi = 0;
        for i in 0..n + 1 {
            if i < lim[0] || i > lim[1] {
                continue;
            }
            for j in 0..n {
                if gi >= self.groups.len() && !self.new_group(pools) {
                    continue;
                }
                if self.groups[gi].ptcls.is_empty() {
                    self.twist += def.twist_step * PI_F;
                    let (a0, ar) = (self.v[T_ANG_A], self.v[T_ANG_A_R]);
                    let (b0, br) = (self.v[T_ANG_B], self.v[T_ANG_B_R]);
                    let a = wrap(step * i as f32 + vu0.float_range(a0, a0 + ar) * PI_F);
                    let b = wrap(step * j as f32 + vu0.float_range(b0, b0 + br) * PI_F);
                    self.emit(gi, a, b, def, pools, vu0, libc);
                }
                self.groups[gi].flags |= G_FILLED;
                gi += 1;
            }
        }
    }

    /// `EftPart10_NewGroup`: a free ring of the pool (round robin), appended.
    fn new_group(&mut self, pools: &mut Pools) -> bool {
        if pools.grp_next >= POOL {
            pools.grp_next = 0;
        }
        let start = pools.grp_next;
        let mut i = start;
        loop {
            let g = i;
            i = (i + 1) % POOL;
            if !pools.grp_used[g] {
                pools.grp_used[g] = true;
                self.groups.push(Group {
                    slot: g,
                    flags: 1,
                    ptcls: Vec::new(),
                });
                pools.grp_next = i;
                return true;
            }
            if i == start {
                return false;
            }
        }
    }

    /// `EftPart10_Emit`: the first free particle of the pool that starts (one whose life comes out as 0 does not,
    /// and the next free one is tried) goes on the unit sphere at the two angles, into ring `gi`.
    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        gi: usize,
        yaw: f32,
        pitch: f32,
        def: &Part10Def,
        pools: &mut Pools,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
    ) {
        if pools.ptcl_next >= POOL {
            pools.ptcl_next = 0;
        }
        let start = pools.ptcl_next;
        let mut i = start;
        loop {
            let slot = i;
            i = (i + 1) % POOL;
            if !pools.ptcl_used[slot]
                && let Some(mut p) = self.init_ptcl(slot, def, vu0, libc)
            {
                let c = yaw.cos();
                p.base = [pitch.sin() * c, pitch.cos() * c, yaw.sin()];
                p.pos = p.base;
                p.flags |= P_ALIVE;
                pools.ptcl_used[slot] = true;
                self.groups[gi].ptcls.push(p);
                pools.ptcl_next = i;
                return;
            }
            if i == start {
                return;
            }
        }
    }

    /// `EftPart10_InitPtcl`: a particle from the emitter's current track values, each drawn between the value and
    /// value + range from the VU0 generator (libc for the coin flips and the sheet frame); `None` when its life
    /// comes out as 0 (three draws made).
    fn init_ptcl(
        &self,
        slot: usize,
        def: &Part10Def,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
    ) -> Option<Ptcl> {
        let v = &self.v;
        let mut r = |t: usize, tr: usize| vu0.float_range(v[t], v[t] + v[tr]);
        let mut p = Ptcl {
            slot,
            offset: [v[T_SCALE_X], v[T_SCALE_Y]],
            ..Ptcl::default()
        };
        p.dist = v[T_DIST_BASE] + r(T_DIST, T_DIST_R);
        p.dist_vel = r(T_DIST_VEL, T_DIST_VEL_R) * self.size;
        let life = r(T_LIFE, T_LIFE_R) * 30.0;
        p.life = life;
        if life <= 0.0 {
            return None;
        }
        p.delay = r(T_WAIT, T_WAIT_R) * 30.0;
        p.ang[2] = (r(T_ANGLE, T_ANGLE_R) + self.twist) * TWO_PI;
        p.ang_vel = [
            r(T_SPIN_X, T_SPIN_X_R) * PI_F,
            r(T_SPIN_Y, T_SPIN_Y_R) * PI_F,
            r(T_SPIN, T_SPIN_R) * PI_F,
        ];
        if def.flags & 4 != 0 && libc.rand() & 1 == 0 {
            p.ang_vel[2] = -p.ang_vel[2];
        }
        if def.flags & 2 != 0 && p.ang_vel[2] < 0.0 {
            p.uv0 = [0.984_375, 0.015_625, 0.015_625, 0.015_625];
            p.uv1 = [0.984_375, 0.984_375, 0.015_625, 0.984_375];
            p.flags |= P_MIRROR;
        } else {
            p.uv0 = [0.015_625, 0.015_625, 0.015_625, 0.984_375];
            p.uv1 = [0.984_375, 0.015_625, 0.984_375, 0.984_375];
        }
        p.tex_frame = if def.flags & 0x10 != 0 {
            libc.int_range(0, (self.tex_frames - 1.0) as i32) as f32
        } else {
            0.0
        };
        let size: [f32; 3] = std::array::from_fn(|i| r(T_SIZE + i, T_SIZE_R + i));
        p.size_vel0 = (size[1] - size[0]) / (p.life * def.size_mid);
        p.size_vel1 = (size[2] - size[1]) / (p.life * (1.0 - def.size_mid));
        p.size = size[0];
        if def.flags & 0x80 != 0 {
            p.pulse_time = v[T_PULSE_TIME] * 30.0;
            p.pulse_d = v[T_PULSE + 1] - v[T_PULSE];
            p.pulse0 = v[T_PULSE];
            p.scale = v[T_PULSE];
        } else {
            p.scale = 1.0;
        }
        if def.color_mode == 2 {
            if libc.rand() & 1 == 0 {
                p.flags |= P_COLOR_RAMP;
            }
        } else if def.color_mode != 1 {
            p.flags |= P_COLOR_RAMP;
        }
        p.fade_time = p.life * (v[T_FADE1] - v[T_FADE0]);
        if p.fade_time <= 0.0 {
            p.fade_time = p.life;
        }
        let [color, color_r, end, end_r] = self.color;
        let mut col0: V4 =
            std::array::from_fn(|i| vu0.float_range(color[i], color[i] + color_r[i]));
        let mut end: V4 = [
            vu0.float_range(end[0], end[0] + end_r[0]),
            vu0.float_range(end[1], end[1] + end_r[1]),
            vu0.float_range(end[2], end[2] + end_r[2]),
            0.0,
        ];
        col0 = col0.map(|c| c.clamp(0.0, 255.0));
        end = end.map(|c| c.clamp(0.0, 255.0));
        p.col0 = col0;
        p.col = [col0[0], col0[1], col0[2], 0.0];
        p.col_delta = std::array::from_fn(|i| end[i] - p.col[i]);
        p.fade_in = p.life * def.fade_in;
        p.fade_out = p.life * (1.0 - def.fade_out);
        if def.flags & 0x100 != 0 {
            p.mul_time = v[T_MUL_TIME] * 30.0;
            for c in 0..3 {
                p.mul_d[c] = v[T_MUL + c * 2 + 1] - v[T_MUL + c * 2];
                p.mul0[c] = v[T_MUL + c * 2];
                if p.mul_d[c] != 0.0 {
                    p.flags |= P_MUL[c];
                }
            }
        }
        Some(p)
    }

    /// `EftPart10_Step`: every particle's delay, colour multiplier pulse (the multipliers of the channels a particle
    /// does not pulse are the last ones set this frame), size pulse (as in the original, its turn clamps the colour
    /// pulse's timer), distance, angles, position, size, colour ramp, fades, sheet frame and corners; expired
    /// particles, and rings not refilled this frame once empty, go back to the pools.
    fn step(&mut self, def: &Part10Def, alpha: f32, pools: &mut Pools) {
        let mut mul = [1.0f32; 3];
        let (mut yaw, mut pitch) = (0.0, 0.0);
        if def.flags & 1 != 0 {
            pitch = wrap(asin_c(-self.dir[1]));
            yaw = wrap(self.dir[0].atan2(self.dir[2]));
        }
        let (fade0, fade1) = (self.v[T_FADE0], self.v[T_FADE1]);
        let kill = self.flags & F_KILL != 0;
        let base_m = rotate_xyz(
            &rotate_xyz(&IDENTITY, [self.yaw, 0.0, 0.0]),
            [0.0, self.pitch, 0.0],
        );
        let mut gi = 0;
        while gi < self.groups.len() {
            let mut k = 0;
            while k < self.groups[gi].ptcls.len() {
                let p = &mut self.groups[gi].ptcls[k];
                if 0.0 < p.delay {
                    p.delay -= 1.0;
                    k += 1;
                    continue;
                }
                let t = p.age / p.life;
                if def.flags & 0x100 != 0 {
                    let u = if 0.0 < p.mul_time {
                        clamp01(p.mul_t / p.mul_time)
                    } else {
                        1.0
                    };
                    for c in 0..3 {
                        if p.flags & P_MUL[c] != 0 {
                            mul[c] = p.mul0[c] + p.mul_d[c] * u;
                        }
                    }
                    p.mul_t += if p.flags & P_MUL_DOWN != 0 { -1.0 } else { 1.0 };
                    if p.mul_t >= p.mul_time || p.mul_t <= 0.0 {
                        p.flags ^= P_MUL_DOWN;
                        p.mul_t = clamp_to(p.mul_t, 0.0, p.mul_time);
                    }
                } else {
                    mul = [1.0; 3];
                }
                if def.flags & 0x80 != 0 {
                    let u = if 0.0 < p.pulse_time {
                        clamp01(p.pulse_t / p.pulse_time)
                    } else {
                        1.0
                    };
                    p.scale = p.pulse0 + p.pulse_d * u;
                    p.pulse_t += if p.flags & P_PULSE_DOWN != 0 {
                        -1.0
                    } else {
                        1.0
                    };
                    if p.pulse_time <= p.pulse_t || p.pulse_t <= 0.0 {
                        p.flags ^= P_PULSE_DOWN;
                        p.mul_t = clamp_to(p.mul_t, 0.0, p.mul_time);
                    }
                }
                p.dist += p.dist_vel;
                for a in 0..3 {
                    p.ang[a] = wrap(p.ang[a] + p.ang_vel[a]);
                }
                let pos = scale3(p.base, p.dist * self.size * self.scale);
                let m = rotate_xyz(&base_m, [wrap(p.ang[0] + self.rot_a + pitch), 0.0, 0.0]);
                let m = rotate_xyz(&m, [0.0, wrap(p.ang[1] + self.rot_b + yaw), 0.0]);
                p.pos = xyz(apply(&m, point(pos)));
                p.size += if t < def.size_mid {
                    p.size_vel0
                } else {
                    p.size_vel1
                };
                if p.flags & P_COLOR_RAMP != 0 {
                    let v = clamp01(p.fade_t / p.fade_time);
                    for c in 0..3 {
                        p.col[c] = (p.col0[c] + p.col_delta[c] * v) * mul[c];
                    }
                    if fade0 <= t && t < fade1 {
                        p.fade_t += 1.0;
                    }
                } else {
                    for c in 0..3 {
                        p.col[c] = p.col0[c] * mul[c];
                    }
                }
                let mut a = if t <= def.fade_in && 0.0 < p.fade_in {
                    p.age / p.fade_in
                } else {
                    1.0
                };
                if def.fade_out < t && 0.0 < p.fade_out {
                    a = 1.0 - (p.age - (p.life - p.fade_out)) / p.fade_out;
                }
                p.col[3] = p.col0[3] * clamp01(a) * alpha;
                p.col = p.col.map(|c| c.clamp(0.0, 255.0));
                if self.flags & F_TEXGRID != 0 {
                    let cell = self.uv[(p.tex_frame as i32).clamp(0, 15) as usize];
                    if p.flags & P_MIRROR != 0 {
                        p.uv0 = [cell[2], cell[1], cell[0], cell[1]];
                        p.uv1 = [cell[2], cell[3], cell[0], cell[3]];
                    } else {
                        p.uv0 = [cell[0], cell[1], cell[2], cell[1]];
                        p.uv1 = [cell[0], cell[3], cell[2], cell[3]];
                    }
                    // A grid sheet has a frame step (a zero would trap the original's division).
                    let step = i32::from(def.tex_step).max(1);
                    if (p.age as i32) % step == 0 {
                        p.tex_frame += 1.0;
                        if self.tex_frames <= p.tex_frame {
                            p.tex_frame = 0.0;
                        }
                    }
                }
                if 0.0 < p.col[3] && 0.0 < p.size {
                    if def.mode != 1 {
                        build_corners(p, self.pos, self.size * self.scale);
                    }
                    p.flags |= P_DRAW;
                } else {
                    p.flags &= !P_DRAW;
                }
                p.age += 1.0;
                if p.life <= p.age || kill {
                    pools.ptcl_used[p.slot] = false;
                    self.groups[gi].ptcls.remove(k);
                } else {
                    k += 1;
                }
            }
            let g = &mut self.groups[gi];
            if g.ptcls.is_empty() && g.flags & G_FILLED == 0 {
                pools.grp_used[g.slot] = false;
                self.groups.remove(gi);
            } else {
                g.flags &= !G_FILLED;
                gi += 1;
            }
        }
    }

    /// `EftPart10_Draw`: every particle to draw, a camera-facing sprite (definition mode 1) or a quad through its
    /// corners, clipped with definition flag 8.
    pub(crate) fn draw(&self, def: &Part10Def, ot: &mut OrderTable, ctx: &DrawCtx) {
        let no_depth = self.flags & F_NODEPTH != 0;
        for p in self.groups.iter().flat_map(|g| g.ptcls.iter()) {
            if p.flags & P_DRAW == 0 {
                continue;
            }
            if def.mode == 1 {
                let half = p.size * self.size * self.scale * p.scale * 0.5;
                let pos = add(p.pos, self.pos);
                if def.flags & 8 != 0 {
                    billboard_clipped(
                        ot,
                        ctx,
                        pos,
                        half,
                        half,
                        p.col,
                        p.offset,
                        [p.uv0[0], p.uv0[1], p.uv1[2], p.uv1[3]],
                        p.ang[2],
                        def.layer,
                        no_depth,
                        self.texture,
                    );
                } else {
                    billboard(
                        ot,
                        ctx,
                        pos,
                        p.col,
                        [half * 16.0, half * 16.0],
                        [(p.offset[0] as i32) << 4, (p.offset[1] as i32) << 4],
                        [p.uv0[0], p.uv0[1], p.uv1[2], p.uv1[3]],
                        def.layer,
                        no_depth,
                        self.texture,
                        p.ang[2],
                    );
                }
            } else {
                let uv = [
                    [p.uv0[0], p.uv0[1]],
                    [p.uv0[2], p.uv0[3]],
                    [p.uv1[0], p.uv1[1]],
                    [p.uv1[2], p.uv1[3]],
                ];
                if def.flags & 8 != 0 {
                    for i in 0..2 {
                        let tri = std::array::from_fn(|k| ClipVtx {
                            pos: point(p.corner[i + k]),
                            uv: [uv[i + k][0], uv[i + k][1], 1.0, 1.0],
                            col: p.col,
                        });
                        let prim = Prim {
                            texture: self.texture,
                            ot_layer: def.layer,
                            z: ZSort::Average(2.0),
                            front: no_depth,
                            clamp: false,
                            alpha_min: 0.01,
                        };
                        queue_tri_clip(ot, ctx, tri, prim);
                    }
                } else {
                    queue_strip(
                        ot,
                        ctx,
                        &p.corner,
                        uv,
                        p.col,
                        def.layer,
                        self.texture,
                        no_depth,
                        false,
                        true,
                    );
                }
            }
        }
    }
}

/// `EFTW_CLAMP`.
fn clamp_to(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if hi < x {
        hi
    } else {
        x
    }
}

/// `EftPart10_BuildCorners`: a square of the particle's size, scaled (the emitter's size and scale times the pulse),
/// moved by its offset, turned by its angle and onto its direction from the emitter, placed at the particle and the
/// emitter.
fn build_corners(p: &mut Ptcl, origin: V3, scale: f32) {
    let h = p.size * 0.5;
    let s = scale * p.scale;
    let mut m = IDENTITY;
    for i in 0..3 {
        m[i][i] = s;
    }
    m[3][0] += p.offset[0];
    m[3][1] += p.offset[1];
    let m = rotate_xyz(&m, [0.0, 0.0, p.ang[2]]);
    let dir = norm(p.pos);
    let pitch = wrap(asin_c(-dir[1]));
    let yaw = wrap(dir[0].atan2(dir[2]));
    let mut m = rotate_xyz(&rotate_xyz(&m, [pitch, 0.0, 0.0]), [0.0, yaw, 0.0]);
    for i in 0..3 {
        m[3][i] += p.pos[i] + origin[i];
    }
    let corners = [[-h, -h], [-h, h], [h, -h], [h, h]];
    p.corner = corners.map(|c| xyz(apply(&m, [c[0], c[1], 0.0, 1.0])));
}

/// `EftPart10_DrawBillboardClipped`: a w x h rectangle (half sizes) moved by `offset`, turned by `rot` in the view
/// plane and placed at `pos`; two clipped triangles in the depth slot of `pos` times 2. Nothing behind the camera.
#[allow(clippy::too_many_arguments)]
pub(crate) fn billboard_clipped(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    pos: V3,
    w: f32,
    h: f32,
    color: V4,
    offset: [f32; 2],
    uv: [f32; 4],
    rot: f32,
    layer: u8,
    no_depth: bool,
    texture: Option<usize>,
) {
    let c = [[-w, -h], [w, -h], [-w, h], [w, h]];
    let mut m = IDENTITY;
    m[3][0] = offset[0];
    m[3][1] = offset[1];
    let m = rotate_xyz(&m, [0.0, 0.0, rot]);
    let mut inv: Mtx = inverse_rt(&ctx.world2view);
    inv[3] = [0.0, 0.0, 0.0, inv[3][3]];
    let m = mtx_mul(&inv, &m);
    let corners = c.map(|c| add(xyz(apply(&m, [c[0], c[1], 0.0, 1.0])), pos));
    let (_, _, sz) = ctx.project(pos);
    if sz < 0 {
        return;
    }
    let z = sz >> 8;
    let st = [
        [uv[0], uv[1]],
        [uv[2], uv[1]],
        [uv[0], uv[3]],
        [uv[2], uv[3]],
    ];
    for i in 0..2 {
        let tri = std::array::from_fn(|k| ClipVtx {
            pos: point(corners[i + k]),
            uv: [st[i + k][0], st[i + k][1], 1.0, 0.0],
            col: color,
        });
        let prim = Prim {
            texture,
            ot_layer: layer,
            z: ZSort::Fixed((z as f32 * 2.0) as i32),
            front: no_depth,
            clamp: false,
            alpha_min: 0.01,
        };
        queue_tri_clip(ot, ctx, tri, prim);
    }
}

/// `EftPart10_DrawBillboard`: a flat quad round the projection of `pos`, `size` half sizes and `off` the centre's
/// offset in 1/16 units scaled by the perspective, turned by `rot` (the height stretched by 1.1666667); nothing
/// below 2 units or with a corner off the GS area (or `pos` on the camera plane).
#[allow(clippy::too_many_arguments)]
pub(crate) fn billboard(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    pos: V3,
    color: V4,
    size: [f32; 2],
    off: [i32; 2],
    uv: [f32; 4],
    layer: u8,
    no_depth: bool,
    texture: Option<usize>,
    rot: f32,
) {
    let v = apply(&ctx.world2screen, point(pos));
    let q = 1.0 / v[3];
    let (sx, sy, mut sz, sw) = (
        (v[0] * q * 16.0) as i32,
        (v[1] * q * 16.0) as i32,
        (v[2] * q) as i32,
        v[3] as i32,
    );
    if sw == 0 {
        return;
    }
    let z = sz >> 8;
    let k = ((SCREEN_DIST * 4096.0) / sw as f32) as i32;
    let w = (size[0] as i32).wrapping_mul(k) >> 12;
    let h = (size[1] as i32).wrapping_mul(k) >> 12;
    let scale_off = |o: i32| {
        if o >= 0 {
            o.wrapping_mul(k) >> 12
        } else {
            -((-o).wrapping_mul(k) >> 12)
        }
    };
    let (ox, oy) = (scale_off(off[0]), scale_off(off[1]));
    if w < 2 || h < 2 {
        return;
    }
    let (s, c) = rot.sin_cos();
    let turn = |x: i32, y: i32| {
        let (x, y) = (x as f32, y as f32);
        [
            (x * c - y * s) as i32,
            ((x * s + y * c) * 1.166_666_7) as i32,
        ]
    };
    let corners = [
        turn(ox - w, oy - h),
        turn(ox - w, h + oy),
        turn(w + ox, oy - h),
        turn(w + ox, h + oy),
    ];
    let off_screen = |x: i32, y: i32| sz <= 0 || x > 0xFFEF || x <= 0 || y > 0xFFEF || y <= 0;
    if corners.iter().any(|cr| off_screen(sx + cr[0], sy + cr[1])) {
        return;
    }
    if no_depth {
        sz = 0xFF_FFFF;
    }
    let zz = (sz >> 8) << 8;
    let rgba = color.map(|c| c as i32 as u8);
    let st = [
        [uv[0], uv[1]],
        [uv[0], uv[3]],
        [uv[2], uv[1]],
        [uv[2], uv[3]],
    ];
    let vert = |i: usize| EffectVertex {
        position: [
            (sx + corners[i][0]) as f32 / 16.0 - OFFSET_X,
            (sy + corners[i][1]) as f32 / 16.0 - OFFSET_Y,
            (zz & 0xFF_FFFF) as f32,
        ],
        stq: [st[i][0], st[i][1], 1.0],
        rgba,
    };
    let l = if layer >= 2 { layer - 2 } else { layer };
    let blend = EffectBlend::of(usize::from(l), layer >= 2);
    for i in 0..2 {
        ot.add(
            depth_slot(z),
            usize::from(l),
            EffectTri {
                texture,
                blend,
                clamp: false,
                vertices: [vert(i), vert(i + 1), vert(i + 2)],
            },
        );
    }
}
