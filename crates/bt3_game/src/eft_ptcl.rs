//! Effect pack part kind 5, the particle emitter (`EftPtcl_*`, src/battle/eft_particle.c): an emitter that every
//! `interval` frames starts particles from a shared pool of 500, each with its own life, path (a cone around the
//! emitter's direction, a start offset, a drift), staged speed / width / height, colours, fades and spin; drawn as
//! quads along their axis, squares or sprites. Every random value is a libc rand().

use bt3_data::eft_pack::{PtclDef, PtclDef2};

use crate::battle::{IDENTITY, Mtx, apply};
use crate::eft::{
    ClipVtx, DrawCtx, OrderTable, Prim, SetRef, V3, V4, ZSort, add, cross, draw_sprite, norm,
    point, quad_depth, queue_strip, queue_tri_clip, scale3, sub, wrap_angle, xyz,
};
use crate::rng::LibcRandom;

/// `EFT_PTCL_MAX`.
pub(crate) const PTCL_MAX: usize = 500;
/// `EftPtclMgr_Init`: 60 emitter tasks.
pub(crate) const EMITTERS: usize = 60;

// EFT_PTCL_*.
const W_ALIVE: u32 = 0x1;
pub(crate) const W_STOPPING: u32 = 0x2;
pub(crate) const W_DEAD: u32 = 0x4;
const W_ENDLESS: u32 = 0x8;
pub(crate) const W_FRONT: u32 = 0x10;
const W_SHEET: u32 = 0x20;
pub(crate) const W_DETACH: u32 = 0x40;
const W_LINGER: u32 = 0x100;
const W_FLIP: u32 = 0x200;

// EFT_UPTCL_*.
const P_FADED_IN: u32 = 0x2;
const P_FADED_OUT: u32 = 0x4;
const P_STAGE2: u32 = 0x8;
const P_WIDTH_DOWN: u32 = 0x10;
const P_HEIGHT_DOWN: u32 = 0x20;
const P_COLOR_DOWN: u32 = 0x40;
const P_REVERSE_SPIN: u32 = 0x80;
const P_DETACHED: u32 = 0x100;
const P_VISIBLE: u32 = 0x400;

/// `(f32)rand() / RAND_MAX`.
fn rand01(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

// `EftUPtclCur` as a flat list of values, which is what makes blending two keys a plain interpolation.
const C_COLOR: usize = 0;
const C_COLOR_RANGE: usize = 4;
const C_COLOR_END: usize = 8;
const C_COLOR_END_RANGE: usize = 12;
const C_PULSE_LO: usize = 16;
const C_PULSE_RANGE: usize = 19;
const C_PULSE_TIME: usize = 22;
const C_COUNT: usize = 23;
const C_COUNT_RANGE: usize = 24;
const C_INTERVAL: usize = 25;
const C_LIFE: usize = 26;
const C_LIFE_RANGE: usize = 27;
const C_SPLIT: usize = 28;
const C_SPEED: usize = 29;
const C_SPEED_RANGE: usize = 32;
const C_WIDTH: usize = 35;
const C_WIDTH_RANGE: usize = 38;
const C_HEIGHT: usize = 41;
const C_HEIGHT_RANGE: usize = 44;
const C_WP_LO: usize = 47;
const C_WP_RANGE: usize = 48;
const C_WP_TIME: usize = 49;
const C_HP_LO: usize = 50;
const C_HP_RANGE: usize = 51;
const C_HP_TIME: usize = 52;
const C_DRIFT: usize = 53;
const C_GRAVITY: usize = 54;
const C_RADIUS: usize = 55;
const C_RADIUS_RANGE: usize = 56;
const C_RING: usize = 57;
const C_RING_RANGE: usize = 58;
const C_ALONG: usize = 59;
const C_CONE_A: usize = 60;
const C_CONE_B: usize = 61;
const C_ROLL: usize = 62;
const C_ROLL_RANGE: usize = 63;
const C_START_DIST: usize = 64;
const C_START_DIST_RANGE: usize = 65;
const C_FLATTEN: usize = 66;
const C_FADE_IN: usize = 67;
const C_FADE_OUT: usize = 68;
const C_ROT: usize = 69;
const C_ROT_RANGE: usize = 70;
const C_SPIN: usize = 71;
const C_SPIN_RANGE: usize = 72;
const C_SPIN2: usize = 73;
const C_SPIN2_RANGE: usize = 74;
const C_COUNT_ALL: usize = 75;

type Cur = [f32; C_COUNT_ALL];

/// `EftPtcl_SetKey`: the current values at key `k`: seconds become frames, half turns radians, {low, high}
/// {low, range}.
fn key_values(def: &PtclDef, def2: &PtclDef2, k: usize) -> Cur {
    let mut c = [0.0; C_COUNT_ALL];
    c[C_COLOR..C_COLOR + 4].copy_from_slice(&def2.color[k]);
    c[C_COLOR_RANGE..C_COLOR_RANGE + 4].copy_from_slice(&def2.color_range[k]);
    c[C_COLOR_END..C_COLOR_END + 4].copy_from_slice(&def2.color_end[k]);
    c[C_COLOR_END_RANGE..C_COLOR_END_RANGE + 4].copy_from_slice(&def2.color_end_range[k]);
    for i in 0..3 {
        c[C_PULSE_LO + i] = def2.pulse[i][k][0];
        c[C_PULSE_RANGE + i] = def2.pulse[i][k][1] - def2.pulse[i][k][0];
    }
    c[C_PULSE_TIME] = def2.pulse_time[k] * 30.0;
    c[C_COUNT] = f32::from(def.count[k]);
    c[C_COUNT_RANGE] = f32::from(def.count_range[k]);
    c[C_INTERVAL] = def.key(0x28, k) * 30.0;
    c[C_LIFE] = def.key(0x34, k) * 30.0;
    c[C_LIFE_RANGE] = def.key(0x40, k) * 30.0;
    c[C_SPLIT] = def.key(0x4C, k);
    for s in 0..3 {
        c[C_SPEED + s] = def.stage(0x58, k, s);
        c[C_SPEED_RANGE + s] = def.stage(0x7C, k, s);
        c[C_WIDTH + s] = def.stage(0xA0, k, s);
        c[C_WIDTH_RANGE + s] = def.stage(0xC4, k, s);
        c[C_HEIGHT + s] = def.stage(0xE8, k, s);
        c[C_HEIGHT_RANGE + s] = def.stage(0x10C, k, s);
    }
    c[C_WP_LO] = def.key(0x130, k);
    c[C_WP_RANGE] = def.key(0x13C, k) - def.key(0x130, k);
    c[C_WP_TIME] = def.key(0x148, k) * 30.0;
    c[C_HP_LO] = def.key(0x154, k);
    c[C_HP_RANGE] = def.key(0x160, k) - def.key(0x154, k);
    c[C_HP_TIME] = def.key(0x16C, k) * 30.0;
    c[C_FADE_IN] = def.key(0x220, k) * 30.0;
    c[C_FADE_OUT] = def.key(0x22C, k) * 30.0;
    let plain = [
        (C_DRIFT, 0x178),
        (C_GRAVITY, 0x184),
        (C_RADIUS, 0x190),
        (C_RADIUS_RANGE, 0x19C),
        (C_RING, 0x1A8),
        (C_RING_RANGE, 0x1B4),
        (C_ALONG, 0x1C0),
        (C_START_DIST, 0x1FC),
        (C_START_DIST_RANGE, 0x208),
        (C_FLATTEN, 0x214),
    ];
    for (i, o) in plain {
        c[i] = def.key(o, k);
    }
    let turns = [
        (C_CONE_A, 0x1CC),
        (C_CONE_B, 0x1D8),
        (C_ROLL, 0x1E4),
        (C_ROLL_RANGE, 0x1F0),
        (C_ROT, 0x238),
        (C_ROT_RANGE, 0x244),
        (C_SPIN, 0x250),
        (C_SPIN_RANGE, 0x25C),
        (C_SPIN2, 0x268),
        (C_SPIN2_RANGE, 0x274),
    ];
    for (i, o) in turns {
        c[i] = def.key(o, k) * 3.141_592_7;
    }
    c
}

/// A particle (`EftUPtcl`).
#[derive(Clone, Copy, Debug, Default)]
struct Ptcl {
    travel: V3,
    origin: V3,
    rel: V3,
    pos: V3,
    axis: V3,
    accel: V3,
    uv: [f32; 4],
    color: V4,
    color_start: V4,
    color_delta: V4,
    flags: u32,
    anim_t: f32,
    age: f32,
    life: f32,
    split: f32,
    fade_out_at: f32,
    fade_in_t: f32,
    fade_out_t: f32,
    fade_in: f32,
    fade_out: f32,
    speed: f32,
    speed_key: [f32; 3],
    width: f32,
    width_key: [f32; 3],
    height: f32,
    height_key: [f32; 3],
    rot: f32,
    spin: f32,
    wp_t: f32,
    hp_t: f32,
    color_pulse_t: f32,
}

/// An emitter task (`EftPtclWork`).
#[derive(Clone, Debug)]
pub(crate) struct Emitter {
    pub id: u32,
    chr: usize,
    pub set: SetRef,
    pub pair: usize,
    texture: Option<usize>,
    pos: V3,
    dir: V3,
    size: f32,
    cur: Cur,
    mtx: Mtx,
    ang: V3,
    spin: V3,
    particles: Vec<Ptcl>,
    pub flags: u32,
    frames: i32,
    du: f32,
    dv: f32,
    anim_frames: f32,
    life: f32,
    emit_timer: f32,
    pub start_delay: f32,
    pub stop_delay: f32,
    pub linger: f32,
    pub linger_init: f32,
    key_time: f32,
    key_dur: f32,
    key_split: f32,
    roll: f32,
}

/// `Mtx_RotateX(m, identity, pitch)`, then `Mtx_RotateY(m, m, yaw)`.
pub(crate) fn rot_xy(pitch: f32, yaw: f32) -> Mtx {
    let (sx, cx) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    let rx: Mtx = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, cx, sx, 0.0],
        [0.0, -sx, cx, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    rot_y(&rx, sy, cy)
}

pub(crate) fn rot_y(m: &Mtx, s: f32, c: f32) -> Mtx {
    let ry: Mtx = [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    std::array::from_fn(|i| apply(&ry, m[i]))
}

pub(crate) fn rot_z(m: &Mtx, a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let rz: Mtx = [
        [c, s, 0.0, 0.0],
        [-s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    std::array::from_fn(|i| apply(&rz, m[i]))
}

pub(crate) fn rot_x(m: &Mtx, a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let rx: Mtx = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, c, s, 0.0],
        [0.0, -s, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    std::array::from_fn(|i| apply(&rx, m[i]))
}

/// `Mtx_MulVec4` of a point (w = 1).
fn mul_point(m: &Mtx, v: V3) -> V3 {
    xyz(apply(m, point(v)))
}

/// `Mtx_MulVec4` of a direction (w = 0, what `Vec3_Normalize` leaves).
fn mul_dir(m: &Mtx, v: V3) -> V3 {
    xyz(apply(m, [v[0], v[1], v[2], 0.0]))
}

impl Emitter {
    /// `EftPtcl_Init`: the flip, the life, the first key, the orientation, the random angles and the sheet.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u32,
        chr: usize,
        set: SetRef,
        pair: usize,
        texture: Option<usize>,
        pos: V3,
        dir: V3,
        life: f32,
        size: f32,
        def: &PtclDef,
        def2: &PtclDef2,
        libc: &mut LibcRandom,
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
            cur: [0.0; C_COUNT_ALL],
            mtx: IDENTITY,
            ang: [0.0; 3],
            spin: [0.0; 3],
            particles: Vec::new(),
            flags: W_ALIVE,
            frames: 0,
            du: 1.0,
            dv: 1.0,
            anim_frames: 0.0,
            life: 0.0,
            emit_timer: 0.0,
            start_delay: 0.0,
            stop_delay: 0.0,
            linger: 0.0,
            linger_init: 0.0,
            key_time: 0.0,
            key_dur: 0.0,
            key_split: 0.0,
            roll: 0.0,
        };
        match def.flip {
            0 => w.flags |= W_FLIP,
            1 if libc.rand() & 1 == 0 => w.flags |= W_FLIP,
            _ => {}
        }
        if life <= 0.0 {
            w.flags |= W_ENDLESS;
        } else {
            w.life = life * 30.0;
        }
        if def.flags & 1 != 0 {
            w.key_dur = def.key_time * 30.0;
            w.key_split = w.key_dur * def.key_split;
            w.cur = key_values(def, def2, 0);
        } else {
            w.cur = key_values(def, def2, 2);
        }
        w.dir = w.dir.map(|c| c.clamp(-1.0, 1.0));
        let pitch = (-w.dir[1]).asin();
        let yaw = w.dir[0].atan2(w.dir[2]);
        w.mtx = rot_xy(pitch, yaw);
        w.roll = wrap_angle(w.cur[C_ROLL] + w.cur[C_ROLL_RANGE] * rand01(libc));
        let a = def.ang;
        let r = |base: f32, range: f32, libc: &mut LibcRandom| {
            wrap_angle((base + range * rand01(libc)) * 3.141_592_7)
        };
        w.ang[0] = r(a[0], a[1], libc);
        w.spin[0] = r(a[2], a[3], libc);
        w.ang[1] = r(a[4], a[5], libc);
        w.spin[1] = r(a[6], a[7], libc);
        w.ang[2] = r(a[8], a[9], libc);
        w.spin[2] = r(a[10], a[11], libc);
        if def.flags & 4 != 0 {
            w.frames = i32::from(def.cols) * i32::from(def.rows);
        }
        if w.frames >= 2 {
            w.flags |= W_SHEET;
            w.du = 1.0 / f32::from(def.cols);
            w.dv = 1.0 / f32::from(def.rows);
            w.anim_frames = def.anim_time * w.frames as f32 * 30.0;
            if w.anim_frames < w.frames as f32 {
                w.anim_frames = w.frames as f32;
            }
        }
        w
    }

    /// `EftPtcl_IsAlive`.
    pub(crate) fn alive(&self) -> bool {
        self.flags & W_ALIVE != 0
    }

    /// `EftPtcl_SetPos`.
    pub(crate) fn set_pos(&mut self, pos: V3) {
        self.pos = pos;
    }

    /// `EftPtcl_Warp`: moves the emitter and re-bases the particles it already emitted.
    pub(crate) fn warp(&mut self, pos: V3) {
        self.pos = pos;
        for p in &mut self.particles {
            if p.flags & P_DETACHED == 0 {
                p.rel = add(p.travel, p.origin);
                p.pos = add(p.rel, pos);
            } else {
                p.pos = add(add(p.travel, p.origin), p.rel);
            }
        }
    }

    /// `EftPtcl_SetDir`.
    pub(crate) fn set_dir(&mut self, dir: V3) {
        self.dir = norm(dir);
        let pitch = (-self.dir[1]).asin();
        let yaw = self.dir[0].atan2(self.dir[2]);
        self.mtx = rot_xy(pitch, yaw);
    }

    /// `EftPtcl_SetSize`.
    pub(crate) fn set_size(&mut self, size: f32) {
        self.size = size;
    }

    /// `EftPtcl_Stop`.
    pub(crate) fn stop(&mut self) {
        self.flags |= W_STOPPING;
        if self.linger_init > 0.0 {
            self.flags |= W_LINGER;
        }
    }

    /// `EftPtcl_Spawn`: `count` particles from the current values; false when the pool runs out.
    fn spawn(
        &mut self,
        def: &PtclDef,
        count: i32,
        used: &mut usize,
        libc: &mut LibcRandom,
    ) -> bool {
        let c = self.cur;
        for _ in 0..count.max(0) {
            if *used >= PTCL_MAX {
                return false;
            }
            *used += 1;
            let mut p = Ptcl::default();
            let life = c[C_LIFE] + c[C_LIFE_RANGE] * rand01(libc);
            p.life = life;
            p.split = p.life * c[C_SPLIT];
            p.fade_out_at = (p.life - c[C_FADE_OUT]).max(p.age);
            if def.flags & 0x40 != 0 && libc.rand() & 1 == 0 {
                p.flags |= P_REVERSE_SPIN;
            }
            p.rot = wrap_angle(c[C_ROT] + c[C_ROT_RANGE] * rand01(libc));
            p.spin = wrap_angle(c[C_SPIN] + c[C_SPIN_RANGE] * rand01(libc));
            if p.flags & P_REVERSE_SPIN != 0 {
                p.spin = -p.spin;
            }
            if def.flags & 8 != 0 {
                if def.flags & 0x10 != 0 {
                    p.wp_t = c[C_WP_TIME] * rand01(libc);
                }
                if def.flags & 0x10 != 0 {
                    p.hp_t = c[C_HP_TIME] * rand01(libc);
                }
            }
            for i in 0..3 {
                p.speed_key[i] = c[C_SPEED + i] + c[C_SPEED_RANGE + i] * rand01(libc);
                p.width_key[i] = c[C_WIDTH + i] + c[C_WIDTH_RANGE + i] * rand01(libc);
                p.height_key[i] = c[C_HEIGHT + i] + c[C_HEIGHT_RANGE + i] * rand01(libc);
            }
            p.speed = p.speed_key[0];
            p.width = p.width_key[0];
            p.height = p.height_key[0];
            if def.flags & 0x100 != 0 {
                p.axis = norm([0.0, c[C_GRAVITY], 0.0]);
            } else {
                let a = wrap_angle((c[C_CONE_A] - c[C_CONE_B]) * rand01(libc) + c[C_CONE_B]);
                let m = rot_z(&rot_x(&IDENTITY, a), self.roll);
                let mut axis = mul_point(&m, [0.0, 0.0, 1.0]);
                axis[0] *= c[C_FLATTEN] + 1.0;
                p.axis = mul_dir(&self.mtx, norm(axis));
            }
            let a = c[C_START_DIST] + c[C_START_DIST_RANGE] * rand01(libc);
            p.travel = scale3(p.axis, a * self.size);
            let r1 = rand01(libc);
            let r2 = rand01(libc);
            let mut a = (c[C_RADIUS] + c[C_RADIUS_RANGE] * r1) * r2;
            a += c[C_RING] + c[C_RING_RANGE] * rand01(libc);
            a *= self.size;
            match def.origin_kind {
                0 => {
                    let m = rot_y(
                        &rot_x(&IDENTITY, self.ang[0]),
                        self.ang[1].sin(),
                        self.ang[1].cos(),
                    );
                    p.origin = mul_point(&m, [0.0, 0.0, a]);
                }
                1 => {
                    let m = rot_y(&IDENTITY, self.ang[1].sin(), self.ang[1].cos());
                    p.origin = mul_point(&m, [0.0, 0.0, a]);
                }
                2 => {
                    let m = rot_z(
                        &rot_y(&IDENTITY, self.ang[1].sin(), self.ang[1].cos()),
                        self.ang[2],
                    );
                    let o = mul_point(&self.mtx, mul_point(&m, [0.0, a, 0.0]));
                    p.origin = add(scale3(self.dir, c[C_ALONG] * rand01(libc)), o);
                }
                _ => {}
            }
            let accel_dir = match def.accel_kind {
                0 => Some(p.axis),
                1 => Some(norm(p.origin)),
                2 => Some(norm(add(norm(p.origin), p.axis))),
                _ => None,
            };
            if let Some(d) = accel_dir {
                p.accel = scale3(norm(sub(self.dir, d)), c[C_DRIFT]);
            }
            p.accel[1] += c[C_GRAVITY];
            p.uv = [0.0, 1.0, 0.0, 1.0];
            for i in 0..4 {
                p.color_start[i] = c[C_COLOR + i] + c[C_COLOR_RANGE + i] * rand01(libc);
            }
            for i in 0..4 {
                p.color_delta[i] = c[C_COLOR_END + i] + c[C_COLOR_END_RANGE + i] * rand01(libc);
            }
            for i in 0..4 {
                p.color_delta[i] -= p.color_start[i];
            }
            p.color = p.color_start;
            p.fade_in = c[C_FADE_IN];
            if p.fade_in <= 0.0 {
                p.flags |= P_FADED_IN;
            }
            p.fade_out = c[C_FADE_OUT];
            if p.fade_out <= 0.0 {
                p.flags |= P_FADED_OUT;
            } else {
                p.fade_out_t = p.fade_out;
            }
            if def.reverse == 1 {
                let mut spd = 0.0;
                let d = p.life - p.split;
                let mut t = 0.0;
                while t < p.life {
                    p.travel = add(p.travel, scale3(p.axis, spd));
                    spd = if t >= p.split {
                        let k = (t - p.split) / d;
                        p.speed_key[1] + (p.speed_key[2] - p.speed_key[1]) * k
                    } else {
                        let k = t / p.split;
                        p.speed_key[0] + (p.speed_key[1] - p.speed_key[0]) * k
                    };
                    spd *= self.size;
                    t += 1.0;
                }
                p.axis = scale3(p.axis, -1.0);
            }
            self.roll += c[C_ROLL] + c[C_ROLL_RANGE] * rand01(libc);
            self.roll = wrap_angle(self.roll);
            self.ang = add(self.ang, self.spin).map(wrap_angle);
            self.particles.push(p);
        }
        true
    }

    /// `EftPtcl_StepPtcls`: one frame of every particle; one whose life is over goes back to the pool.
    fn step(&mut self, def: &PtclDef, used: &mut usize, libc: &mut LibcRandom) {
        let c = self.cur;
        let (size, pos, flags) = (self.size, self.pos, self.flags);
        let (anim_frames, frames, du, dv) = (self.anim_frames, self.frames, self.du, self.dv);
        let mut i = 0;
        while i < self.particles.len() {
            let p = &mut self.particles[i];
            if !(p.age < p.life) {
                self.particles.remove(i);
                *used -= 1;
                continue;
            }
            let t = (p.age / p.life).min(1.0);
            if p.flags & P_STAGE2 == 0 && p.age >= p.split {
                p.flags |= P_STAGE2;
                p.spin = wrap_angle(c[C_SPIN2] + c[C_SPIN2_RANGE] * rand01(libc));
                if p.flags & P_REVERSE_SPIN != 0 {
                    p.spin = -p.spin;
                }
            }
            p.rot = wrap_angle(p.rot + p.spin);
            let (k, a, b) = if p.flags & P_STAGE2 == 0 {
                (p.age / p.split, 0, 1)
            } else {
                ((p.age - p.split) / (p.life - p.split), 1, 2)
            };
            p.speed = (p.speed_key[a] + (p.speed_key[b] - p.speed_key[a]) * k) * size;
            p.width = (p.width_key[a] + (p.width_key[b] - p.width_key[a]) * k) * size;
            p.height = (p.height_key[a] + (p.height_key[b] - p.height_key[a]) * k) * size;
            if def.flags & 8 != 0 {
                pulse(
                    &mut p.width,
                    &mut p.wp_t,
                    &mut p.flags,
                    P_WIDTH_DOWN,
                    c[C_WP_LO],
                    c[C_WP_RANGE],
                    c[C_WP_TIME],
                );
                pulse(
                    &mut p.height,
                    &mut p.hp_t,
                    &mut p.flags,
                    P_HEIGHT_DOWN,
                    c[C_HP_LO],
                    c[C_HP_RANGE],
                    c[C_HP_TIME],
                );
            }
            p.travel = add(scale3(p.axis, p.speed), p.travel);
            if p.flags & P_DETACHED == 0 {
                p.rel = add(p.travel, p.origin);
                p.pos = add(p.rel, pos);
            } else {
                p.pos = add(add(p.travel, p.origin), p.rel);
            }
            p.axis = norm(add(p.axis, p.accel));
            if flags & W_DETACH != 0 && p.flags & P_DETACHED == 0 {
                p.rel = pos;
                p.flags |= P_DETACHED;
            }
            if flags & W_SHEET != 0 {
                let k = p.anim_t / anim_frames;
                let s = p.anim_t + 1.0;
                let frame = (frames as f32 * k) as i32;
                let cols = i32::from(def.cols);
                p.uv[0] = (frame % cols) as f32 * du;
                p.uv[1] = p.uv[0] + du;
                p.uv[2] = (frame / cols) as f32 * dv;
                p.uv[3] = p.uv[2] + dv;
                p.anim_t = if s >= anim_frames { s - anim_frames } else { s };
            }
            p.color = if flags & W_FLIP != 0 {
                std::array::from_fn(|ch| p.color_delta[ch] * t + p.color_start[ch])
            } else {
                p.color_start
            };
            p.color = p.color.map(|v| v.clamp(0.0, 255.0));
            if p.flags & P_FADED_IN == 0 {
                let k = p.fade_in_t / p.fade_in;
                p.fade_in_t += 1.0;
                if p.fade_in_t >= p.fade_in {
                    p.flags |= P_FADED_IN;
                } else {
                    p.color[3] *= k;
                }
            }
            if p.age >= p.fade_out_at {
                if p.flags & P_FADED_OUT == 0 {
                    let k = p.fade_out_t / p.fade_out;
                    p.fade_out_t -= 1.0;
                    if p.fade_out_t <= 0.0 {
                        p.color[3] = 0.0;
                        p.flags |= P_FADED_OUT;
                    } else {
                        p.color[3] *= k;
                    }
                } else {
                    p.color[3] = 0.0;
                }
            }
            if def.flags & 0x20 != 0 && c[C_PULSE_TIME] > 0.0 {
                let k = p.color_pulse_t / c[C_PULSE_TIME];
                for ch in 0..3 {
                    p.color[ch] *= c[C_PULSE_LO + ch] + c[C_PULSE_RANGE + ch] * k;
                }
                if p.flags & P_COLOR_DOWN == 0 {
                    p.color_pulse_t += 1.0;
                    if p.color_pulse_t >= c[C_PULSE_TIME] {
                        p.color_pulse_t = c[C_PULSE_TIME];
                        p.flags |= P_COLOR_DOWN;
                    }
                } else {
                    p.color_pulse_t -= 1.0;
                    if p.color_pulse_t <= 0.0 {
                        p.color_pulse_t = 0.0;
                        p.flags &= !P_COLOR_DOWN;
                    }
                }
            }
            if p.color[3] <= 0.0 || p.width <= 0.0 || p.height <= 0.0 {
                p.flags &= !P_VISIBLE;
            } else {
                p.flags |= P_VISIBLE;
                match def.shape {
                    0 => {
                        p.width *= 0.5;
                        if def.centred == 1 {
                            p.height *= 0.5;
                        }
                    }
                    1 | 2 => {
                        p.width *= 0.5;
                        p.height *= 0.5;
                        if def.flags & 2 == 0 {
                            p.width *= 16.0;
                            p.height *= 16.0;
                        }
                    }
                    _ => {}
                }
            }
            p.age += 1.0;
            i += 1;
        }
    }

    /// `EftPtcl_Emit`: while the timer is due, count + random particles, limited by the free pool.
    fn emit(&mut self, def: &PtclDef, used: &mut usize, libc: &mut LibcRandom) {
        while self.emit_timer <= 0.0 {
            let free = PTCL_MAX as i32 - *used as i32;
            let mut any = false;
            let mut n = 0;
            if free > 0 {
                n = self.cur[C_COUNT] as i32;
                if self.cur[C_COUNT_RANGE] > 0.0 {
                    n = (n as f32 + self.cur[C_COUNT_RANGE] * rand01(libc)) as i32;
                }
                any = true;
                if free < n {
                    n = free;
                }
            }
            if any {
                self.spawn(def, n, used, libc);
            }
            self.emit_timer += self.cur[C_INTERVAL];
            if !any {
                break;
            }
        }
    }

    /// `EftPtcl_Update`.
    pub(crate) fn update(
        &mut self,
        def: &PtclDef,
        def2: &PtclDef2,
        stopped: bool,
        used: &mut usize,
        libc: &mut LibcRandom,
    ) {
        if !stopped {
            if self.flags & W_STOPPING != 0 && self.stop_delay <= 0.0 {
                if self.flags & W_LINGER != 0 {
                    self.linger -= 1.0;
                    if self.linger <= 0.0 {
                        self.linger = 0.0;
                        self.flags |= W_DEAD;
                    }
                } else if self.particles.is_empty() {
                    self.flags |= W_DEAD;
                }
            }
            if self.start_delay <= 0.0 {
                if self.emit_timer <= 0.0 {
                    if self.flags & W_STOPPING == 0 || self.stop_delay > 0.0 || self.linger > 0.0 {
                        self.emit(def, used, libc);
                    }
                } else {
                    self.emit_timer -= 1.0;
                }
                self.step(def, used, libc);
                if def.flags & 1 != 0 && self.key_time <= self.key_dur {
                    // EftPtcl_BlendKeys: keys 0..1 before the split, 1..2 after.
                    let (t, k0, k1) = if self.key_time < self.key_split {
                        (self.key_time / self.key_split, 0, 1)
                    } else {
                        (
                            (self.key_time - self.key_split) / (self.key_dur - self.key_split),
                            1,
                            2,
                        )
                    };
                    let (a, b) = (key_values(def, def2, k0), key_values(def, def2, k1));
                    self.cur = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
                    self.key_time += 1.0;
                    if self.key_dur <= self.key_time {
                        self.cur = key_values(def, def2, 2);
                    }
                }
                if self.flags & W_ENDLESS == 0 {
                    self.life -= 1.0;
                    if self.life <= 0.0 {
                        self.flags |= W_STOPPING;
                        if self.linger_init > 0.0 {
                            self.flags |= W_LINGER;
                        }
                    }
                }
            } else {
                self.start_delay -= 1.0;
            }
            if self.flags & W_STOPPING != 0 && self.stop_delay > 0.0 {
                self.stop_delay -= 1.0;
            }
        }
    }

    /// `EftPtcl_Term`: the particles go back to the pool.
    pub(crate) fn free(&mut self, used: &mut usize) {
        *used -= self.particles.len();
        self.particles.clear();
    }

    pub(crate) fn chr(&self) -> usize {
        self.chr
    }

    /// `EftPtcl_Draw` by shape (quads along the axis, squares, sprites).
    pub(crate) fn draw(&self, def: &PtclDef, ot: &mut OrderTable, ctx: &DrawCtx) {
        let front = self.flags & W_FRONT != 0;
        for p in self.particles.iter().filter(|p| p.flags & P_VISIBLE != 0) {
            let uv = [p.uv[0], p.uv[2], p.uv[1], p.uv[3]];
            match def.shape {
                0 => {
                    let quad = self.axis_quad(def, p, ctx.eye);
                    if def.flags & 2 != 0 {
                        self.axis_polys(def, p, &quad, ot, ctx);
                    } else {
                        self.axis_strip(def, p, &quad, ot, ctx);
                    }
                }
                1 | 2 => {
                    let (w, h) = if def.shape == 1 {
                        (p.height, p.height)
                    } else {
                        (p.width, p.height)
                    };
                    if def.flags & 2 != 0 {
                        quad_depth(
                            ot,
                            ctx,
                            p.pos,
                            p.color,
                            def.layer,
                            front,
                            self.texture,
                            [w, h],
                            uv,
                            p.rot,
                        );
                    } else {
                        draw_sprite(
                            ot,
                            ctx,
                            p.pos,
                            p.color,
                            [w, h],
                            uv,
                            p.rot,
                            def.layer,
                            front,
                            self.texture,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// The four corners of a particle's quad along its axis, facing the camera.
    fn axis_quad(&self, def: &PtclDef, p: &Ptcl, cam: V3) -> [V3; 4] {
        let side = norm(cross(p.axis, sub(p.pos, cam)));
        let half = scale3(side, p.width);
        let len = scale3(p.axis, p.height);
        let mut q = [[0.0; 3]; 4];
        let (a, b, c, d) = if def.flags & 0x80 != 0 {
            (0, 1, 2, 3)
        } else {
            (2, 3, 0, 1)
        };
        q[a] = add(p.pos, half);
        q[b] = sub(p.pos, half);
        q[c] = add(q[a], len);
        q[d] = add(q[b], len);
        if def.centred != 0 {
            q[a] = sub(q[a], len);
            q[b] = sub(q[b], len);
        }
        q
    }

    fn quad_uv(&self, p: &Ptcl, w: f32) -> [V4; 4] {
        std::array::from_fn(|i| {
            [
                (i % 2) as f32 * self.du + p.uv[0],
                (i / 2) as f32 * self.dv + p.uv[2],
                1.0,
                w,
            ]
        })
    }

    /// `EftPtcl_DrawAxisPolys`: the quad as two clipped triangles, depth slot twice the average.
    fn axis_polys(
        &self,
        def: &PtclDef,
        p: &Ptcl,
        quad: &[V3; 4],
        ot: &mut OrderTable,
        ctx: &DrawCtx,
    ) {
        let uv = self.quad_uv(p, 1.0);
        for i in 0..2 {
            let tri = std::array::from_fn(|k| ClipVtx {
                pos: point(quad[i + k]),
                uv: uv[i + k],
                col: p.color,
            });
            let prim = Prim {
                texture: self.texture,
                ot_layer: def.layer,
                z: ZSort::Average(2.0),
                front: self.flags & W_FRONT != 0,
                clamp: false,
                alpha_min: 0.01,
            };
            queue_tri_clip(ot, ctx, tri, prim);
        }
    }

    /// `EftPtcl_DrawAxisQuads`: the quad projected as a strip without clipping, dropped unless all four corners are
    /// on screen; depth slot from the average of the four depths.
    fn axis_strip(
        &self,
        def: &PtclDef,
        p: &Ptcl,
        quad: &[V3; 4],
        ot: &mut OrderTable,
        ctx: &DrawCtx,
    ) {
        let uv = self.quad_uv(p, 0.0);
        queue_strip(
            ot,
            ctx,
            quad,
            std::array::from_fn(|k| [uv[k][0], uv[k][1]]),
            p.color,
            def.layer,
            self.texture,
            self.flags & W_FRONT != 0,
            false,
            false,
        );
    }
}

/// A ping-pong pulse of a particle's width or height.
fn pulse(v: &mut f32, t: &mut f32, flags: &mut u32, down: u32, lo: f32, range: f32, time: f32) {
    if time <= 0.0 {
        return;
    }
    *v *= lo + range * (*t / time);
    if *flags & down == 0 {
        *t += 1.0;
        if *t >= time {
            *t = time;
            *flags |= down;
        }
    } else {
        *t -= 1.0;
        if *t <= 0.0 {
            *t = 0.0;
            *flags &= !down;
        }
    }
}
