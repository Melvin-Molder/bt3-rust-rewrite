//! Effect pack part kind 9, the quad emitter (`EftQuad_*`, src/battle/eft_quad_1.c and eft_quad_2.c): an emitter
//! that every `interval` frames takes quads from a shared pool of 200, each with its own start offset, direction
//! around the emitter's axis, spin, size and speed steps, size and colour oscillations, colour ramp and fades; the
//! quads are built in the emitter's frame and drawn as strips or clipped triangles. The quads draw from the VU0
//! generator (`Rand_FloatRange`) and, for coin flips, libc `rand()`.

use bt3_data::eft_pack::{QuadDef, QuadDef2};

use crate::battle::{IDENTITY, Mtx, apply};
use crate::eft::{
    ClipVtx, DrawCtx, OrderTable, Prim, SetRef, V3, V4, ZSort, add, norm, point, queue_strip,
    queue_tri_clip, scale3, sub, wrap_angle, xyz,
};
use crate::eft_ptcl::{rot_x, rot_y, rot_z};
use crate::rng::{LibcRandom, Vu0Random};

/// `EFT_QUAD_MAX`.
const QUAD_MAX: usize = 200;
/// `EftQuadMgr_Init`: 30 emitter tasks.
pub(crate) const EMITTERS: usize = 30;

const PI: f32 = 3.141_592_7;
const TWO_PI: f32 = 6.283_185_3;

// EFT_QUADEM_*.
const E_ALIVE: u32 = 0x1;
const E_STOPPED: u32 = 0x2;
const E_HOLD: u32 = 0x4;
const E_FADE: u32 = 0x8;
pub(crate) const E_DEAD: u32 = 0x10;
const E_KILL: u32 = 0x20;
pub(crate) const E_VIEW_ONLY: u32 = 0x80;
pub(crate) const E_OWN_ORIGIN: u32 = 0x100;
const E_SHEET: u32 = 0x400;
const E_ANIM: u32 = 0x2000;
const E_LEG2: u32 = 0x4000;

// EFT_QUAD_*.
const Q_USED: u32 = 0x1;
const Q_DEAD: u32 = 0x10;
const Q_STATIC_COLOR: u32 = 0x40;
const Q_VISIBLE: u32 = 0x200;
const Q_SIZE_OSC: u32 = 0x800;
const Q_SIZE_DOWN: u32 = 0x1000;
const Q_COLOR_RAMP: u32 = 0x8000;
const Q_OSC: [u32; 3] = [0x10000, 0x20000, 0x40000];
const Q_OSC_DOWN: u32 = 0x80000;

// EFT_QUADDEF_*.
const D_UV_FLIP: i32 = 0x4;
const D_ANIM: i32 = 0x10;
const D_FACING: i32 = 0x20;
const D_CROSS: i32 = 0x40;
const D_COLOR_OSC: i32 = 0x80;

/// The emitter's current values (`EftQuadCur`) as a flat list, which makes the key deltas plain differences.
const C_SIZE_MUL: usize = 0;
const C_SIZE_MUL_RANGE: usize = 3;
const C_SPEED_MUL: usize = 6;
const C_SPEED_MUL_RANGE: usize = 9;
const C_ARC: usize = 12;
const C_ARC_RANGE: usize = 13;
const C_OFFSET: usize = 14;
const C_LIFE: usize = 20;
const C_LIFE_RANGE: usize = 21;
const C_SPEED: usize = 22;
const C_ROT: usize = 23;
const C_ROT_RANGE: usize = 26;
const C_SPIN: usize = 29;
const C_ROT_VEL: usize = 32;
const C_ROT_VEL_RANGE: usize = 35;
const C_SIZE_OSC: usize = 38;
const C_SIZE_OSC_TIME: usize = 40;
const C_COLOR: usize = 41;
const C_COLOR_RANGE: usize = 45;
const C_END_COLOR: usize = 49;
const C_END_COLOR_RANGE: usize = 53;
const C_RAMP_START: usize = 57;
const C_RAMP_END: usize = 58;
const C_OSC_LO: usize = 59;
const C_OSC_HI: usize = 62;
const C_OSC_TIME: usize = 65;
const C_INTERVAL: usize = 66;
const C_COUNT: usize = 67;
const C_ALL: usize = 68;

type Cur = [f32; C_ALL];

/// Key `k` of every animated value, in the current values' order.
fn key_values(def: &QuadDef, def2: &QuadDef2, k: usize) -> Cur {
    let mut c = [0.0; C_ALL];
    let v3 = [
        (C_SIZE_MUL, &def.size_mul),
        (C_SIZE_MUL_RANGE, &def.size_mul_range),
        (C_SPEED_MUL, &def.speed_mul),
        (C_SPEED_MUL_RANGE, &def.speed_mul_range),
        (C_ROT, &def.rot),
        (C_ROT_RANGE, &def.rot_range),
        (C_SPIN, &def.spin),
        (C_ROT_VEL, &def.rot_vel),
        (C_ROT_VEL_RANGE, &def.rot_vel_range),
    ];
    for (i, v) in v3 {
        c[i..i + 3].copy_from_slice(&v[k]);
    }
    c[C_ARC] = def.arc[k];
    c[C_ARC_RANGE] = def.arc_range[k];
    c[C_OFFSET..C_OFFSET + 6].copy_from_slice(&def.offset[k]);
    c[C_LIFE] = def.life[k];
    c[C_LIFE_RANGE] = def.life_range[k];
    c[C_SPEED] = def.speed[k];
    c[C_SIZE_OSC..C_SIZE_OSC + 2].copy_from_slice(&def.size_osc[k]);
    c[C_SIZE_OSC_TIME] = def.size_osc_time[k];
    c[C_COLOR..C_COLOR + 4].copy_from_slice(&def2.color[k]);
    c[C_COLOR_RANGE..C_COLOR_RANGE + 4].copy_from_slice(&def2.color_range[k]);
    c[C_END_COLOR..C_END_COLOR + 4].copy_from_slice(&def2.end_color[k]);
    c[C_END_COLOR_RANGE..C_END_COLOR_RANGE + 4].copy_from_slice(&def2.end_color_range[k]);
    c[C_RAMP_START] = def2.ramp_start[k];
    c[C_RAMP_END] = def2.ramp_end[k];
    for ch in 0..3 {
        c[C_OSC_LO + ch] = def2.osc[ch][k][0];
        c[C_OSC_HI + ch] = def2.osc[ch][k][1];
    }
    c[C_OSC_TIME] = def2.osc_time[k];
    c[C_INTERVAL] = f32::from(def.interval[k]);
    c[C_COUNT] = f32::from(def.count[k]);
    c
}

/// A quad of the pool (`EftQuad`). Nothing clears a slot: what its initialisation does not write is left from the
/// slot's last use.
#[derive(Clone, Copy, Debug, Default)]
struct Quad {
    pos: V3,
    origin: V3,
    vel: V3,
    accel: V3,
    color: V4,
    color_ramp: V3,
    color_base: V3,
    alpha_base: f32,
    rot: V3,
    corner: [V4; 4],
    /// Per corner (u, v).
    uv: [[f32; 2]; 4],
    rot_vel: V3,
    fade_in: f32,
    fade_out: f32,
    size_osc_amp: f32,
    color_osc_amp: V3,
    speed: f32,
    speed_mul: f32,
    speed_step: [f32; 2],
    size_osc_base: f32,
    size_osc_time: f32,
    size_osc_period: f32,
    color_mul: V3,
    color_osc_time: f32,
    color_osc_period: f32,
    half_size: f32,
    size: f32,
    size_mul: f32,
    size_step: [f32; 2],
    ramp_len: f32,
    ramp_time: f32,
    life: f32,
    age: f32,
    frame: f32,
    ramp_start: f32,
    ramp_end: f32,
    flags: u32,
    tex: u8,
}

/// `gEftQuadMgr`: the pool and the slot the next search starts at.
#[derive(Clone, Debug)]
pub(crate) struct Pool {
    quads: Vec<Quad>,
    next: usize,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            quads: vec![Quad::default(); QUAD_MAX],
            next: 0,
        }
    }
}

impl Pool {
    /// Quads in use.
    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.quads.iter().filter(|q| q.flags != 0).count()
    }
}

/// The two generators a quad draws from.
pub(crate) struct Rng<'a> {
    pub vu0: &'a mut Vu0Random,
    pub libc: &'a mut LibcRandom,
}

/// An emitter task (`EftQuadWork`).
#[derive(Clone, Debug)]
pub(crate) struct QuadEmitter {
    pub id: u32,
    chr: usize,
    pub set: SetRef,
    pub pair: usize,
    /// The texture table (`tex`); `None`: nothing is drawn.
    pub table: Option<usize>,
    tex_a: u8,
    tex_b: u8,
    pos: V3,
    dir: V3,
    angle: V3,
    size: f32,
    life: f32,
    age: f32,
    pub delay: f32,
    pub stop_delay: f32,
    fade: f32,
    fade_left: f32,
    uv: [[f32; 4]; 16],
    delta: Cur,
    anim_end: f32,
    anim_split: f32,
    cur: Cur,
    frames: u8,
    pub flags: u32,
    /// Pool slots in list order.
    list: Vec<usize>,
}

impl QuadEmitter {
    /// `EftQuad_Init`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u32,
        chr: usize,
        set: SetRef,
        pair: usize,
        table: Option<usize>,
        pos: V3,
        dir: V3,
        size: f32,
        life: f32,
        def: &QuadDef,
        def2: &QuadDef2,
    ) -> Self {
        let mut e = Self {
            id,
            chr,
            set,
            pair,
            table: None,
            tex_a: 0,
            tex_b: 0,
            pos,
            dir: norm(dir),
            angle: [0.0; 3],
            size,
            life: life * 30.0,
            age: 0.0,
            delay: 0.0,
            stop_delay: 0.0,
            fade: 0.0,
            fade_left: 0.0,
            uv: [[0.0; 4]; 16],
            delta: [0.0; C_ALL],
            anim_end: 0.0,
            anim_split: 0.0,
            cur: [0.0; C_ALL],
            frames: 0,
            flags: 0,
            list: Vec::new(),
        };
        e.cur[C_INTERVAL] = f32::from(def.interval[2]);
        e.cur[C_COUNT] = f32::from(def.count[2]);
        if def.cols >= 2 || def.rows >= 2 {
            e.set_sheet(def.cols, def.rows);
            e.flags |= E_SHEET;
        }
        e.set_tex(table, 0, 0);
        e.flags |= E_ALIVE;
        if def.flags & D_ANIM != 0 {
            e.calc_key_deltas(def, def2);
            e.flags |= E_ANIM;
        } else {
            e.set_last_key(def, def2);
            e.angle = [0.0; 3];
        }
        e
    }

    pub(crate) fn alive(&self) -> bool {
        self.flags & E_ALIVE != 0
    }

    pub(crate) fn chr(&self) -> usize {
        self.chr
    }

    /// `EftQuad_SetTex` / `EftQuad_SetTexPair`.
    pub(crate) fn set_tex(&mut self, table: Option<usize>, image: u8, palette: u8) {
        self.table = table;
        self.tex_a = image;
        self.tex_b = palette;
    }

    /// `EftQuad_SetFade`.
    pub(crate) fn set_fade(&mut self, frames: f32) {
        self.fade = frames;
        self.fade_left = frames;
    }

    /// `EftQuad_SetPos`.
    pub(crate) fn set_pos(&mut self, pos: V3) {
        self.pos = pos;
    }

    /// `EftQuad_SetDir`.
    pub(crate) fn set_dir(&mut self, dir: V3) {
        self.dir = norm(dir);
    }

    /// `EftQuad_SetSize`.
    pub(crate) fn set_size(&mut self, size: f32) {
        self.size = size;
    }

    /// `EftQuad_Stop`: after the stop delay, through the fade, or at once.
    pub(crate) fn stop(&mut self) {
        let mut now = true;
        if self.stop_delay > 0.0 {
            self.flags |= E_HOLD;
            now = false;
        }
        if self.fade > 0.0 {
            self.flags |= E_FADE;
            now = false;
        }
        if now {
            self.flags |= E_STOPPED;
        }
    }

    /// `EftQuad_Kill`: every quad goes at the next step.
    pub(crate) fn kill(&mut self) {
        self.flags |= E_KILL | E_STOPPED;
    }

    /// `EftQuad_Warp`: moves the emitter and rebuilds every quad's corners at the new place.
    pub(crate) fn warp(&mut self, def: &QuadDef, pool: &mut Pool, pos: V3) {
        let pitch = wrap_angle((-self.dir[1]).asin());
        let yaw = wrap_angle(self.dir[0].atan2(self.dir[2]));
        self.pos = pos;
        for &i in &self.list {
            let at = if self.flags & E_OWN_ORIGIN != 0 {
                pool.quads[i].origin
            } else {
                self.pos
            };
            self.build_corners(def, &mut pool.quads[i], at, pitch, yaw);
        }
    }

    /// `EftQuad_SetSheet`: the frames of a cols x rows sheet (each 1..4), row by row.
    fn set_sheet(&mut self, cols: u8, rows: u8) {
        let cols = cols.clamp(1, 4);
        let rows = rows.clamp(1, 4);
        let d = [1.0 / f32::from(cols), 1.0 / f32::from(rows)];
        self.frames = cols * rows;
        let mut n = 0;
        for j in 0..rows {
            for i in 0..cols {
                let (i, j) = (f32::from(i), f32::from(j));
                self.uv[n] = [d[0] * i, d[1] * j, d[0] * i + d[0], d[1] * j + d[1]];
                n += 1;
            }
        }
    }

    /// `EftQuad_SetLastKey`: the third key of every animated value but the interval and the count.
    fn set_last_key(&mut self, def: &QuadDef, def2: &QuadDef2) {
        let last = key_values(def, def2, 2);
        let (interval, count) = (self.cur[C_INTERVAL], self.cur[C_COUNT]);
        self.cur = last;
        self.cur[C_INTERVAL] = interval;
        self.cur[C_COUNT] = count;
    }

    /// `EftQuad_CalcKeyDeltas`: at age 0 the two legs' lengths and key 1 - key 0; once the age passes the first
    /// leg, key 2 - key 1.
    fn calc_key_deltas(&mut self, def: &QuadDef, def2: &QuadDef2) {
        let mut k = 0;
        if self.age <= 0.0 {
            k = 1;
            self.anim_end = def.key_time * 30.0;
            self.anim_split = self.anim_end * def.key_split;
        }
        if self.anim_split <= self.age && self.flags & E_LEG2 == 0 {
            self.flags |= E_LEG2;
            k = 2;
        }
        if k > 0 {
            let (a, b) = (key_values(def, def2, k - 1), key_values(def, def2, k));
            self.delta = std::array::from_fn(|i| b[i] - a[i]);
        }
    }

    /// `EftQuad_Animate`: the leg's first key plus its difference times the progress. Original bug: all three
    /// speed factor ranges advance with the difference of the first.
    fn animate(&mut self, def: &QuadDef, def2: &QuadDef2) {
        let (k, t) = if self.flags & E_LEG2 == 0 {
            (0, self.age / self.anim_split)
        } else {
            (
                1,
                (self.age - self.anim_split) / (self.anim_end - self.anim_split),
            )
        };
        let base = key_values(def, def2, k);
        let mut delta = self.delta;
        delta[C_SPEED_MUL_RANGE + 1] = delta[C_SPEED_MUL_RANGE];
        delta[C_SPEED_MUL_RANGE + 2] = delta[C_SPEED_MUL_RANGE];
        self.cur = std::array::from_fn(|i| base[i] + delta[i] * t);
    }

    /// `EftQuad_Update`.
    pub(crate) fn update(
        &mut self,
        def: &QuadDef,
        def2: &QuadDef2,
        stopped: bool,
        pool: &mut Pool,
        rng: &mut Rng,
    ) {
        if stopped {
            return;
        }
        if 0.0 < self.delay {
            self.delay -= 1.0;
            return;
        }
        if self.flags & E_FADE != 0 && self.flags & E_HOLD == 0 {
            // The fade's ratio is computed and not used.
            self.fade_left -= 1.0;
            if self.fade_left < 0.0 {
                self.flags |= E_DEAD;
            }
        }
        if self.flags & E_ANIM != 0 {
            self.animate(def, def2);
        }
        if self.flags & E_STOPPED == 0 {
            // The interval is never 0 in the game's data (the original would divide by zero).
            let interval = self.cur[C_INTERVAL] as i32;
            if interval != 0 && (self.age as i32) % interval == 0 {
                for _ in 0..self.cur[C_COUNT] as i32 {
                    for i in 0..3 {
                        self.angle[i] = wrap_angle(self.angle[i] + self.cur[C_SPIN + i] * PI);
                    }
                    self.emit(def, pool, rng);
                }
            }
        }
        self.step_all(def, pool);
        if 0.0 < self.life {
            self.life -= 1.0;
            if self.life <= 0.0 {
                self.life = 0.0;
                if 0.0 < self.fade {
                    self.flags |= E_FADE;
                } else {
                    self.flags |= E_STOPPED;
                }
            }
        }
        if self.flags & E_HOLD != 0 {
            self.stop_delay -= 1.0;
            if self.stop_delay < 0.0 {
                self.flags &= !E_HOLD;
                if self.flags & E_FADE == 0 {
                    self.flags |= E_STOPPED;
                }
            }
        }
        self.age += 1.0;
        if self.list.is_empty() && self.flags & E_STOPPED != 0 {
            self.flags |= E_DEAD;
        }
        if self.flags & E_ANIM != 0 {
            self.calc_key_deltas(def, def2);
            if self.age >= self.anim_end {
                self.set_last_key(def, def2);
                self.cur[C_COUNT] = f32::from(def.count[2]);
                self.cur[C_INTERVAL] = f32::from(def.interval[2]);
                self.flags &= !E_ANIM;
            }
        }
    }

    /// `EftQuad_Term`: the quads go back to the pool.
    pub(crate) fn free(&mut self, pool: &mut Pool) {
        for i in self.list.drain(..) {
            pool.quads[i].flags = 0;
        }
        self.flags = 0;
    }

    /// `EftQuad_Emit`: the first slot from the search start on that is free and that the initialisation accepts.
    fn emit(&mut self, def: &QuadDef, pool: &mut Pool, rng: &mut Rng) -> bool {
        if pool.next >= QUAD_MAX {
            pool.next = 0;
        }
        let mut i = pool.next;
        loop {
            let slot = i;
            i += 1;
            if i >= QUAD_MAX {
                i = 0;
            }
            if pool.quads[slot].flags == 0 && self.init_quad(def, &mut pool.quads[slot], rng) {
                pool.quads[slot].flags |= Q_USED;
                self.list.push(slot);
                pool.next = i;
                return true;
            }
            if i == pool.next {
                return false;
            }
        }
    }

    /// `EftQuad_InitQuad`: false when the life comes out as 0 (the slot keeps what was written up to there).
    fn init_quad(&self, def: &QuadDef, p: &mut Quad, rng: &mut Rng) -> bool {
        let c = &self.cur;
        let mut r = |a: f32, range: f32| rng.vu0.float_range(a, a + range);
        let x = r(c[C_OFFSET], c[C_OFFSET + 1]);
        let y = r(c[C_OFFSET + 2], c[C_OFFSET + 3]);
        let z = r(c[C_OFFSET + 4], c[C_OFFSET + 5]);
        p.pos = [x, y, z];
        p.origin = self.pos;
        let a = r(c[C_ARC], c[C_ARC_RANGE]) * PI;
        let spin = wrap_angle(r(0.0, 1.0) * TWO_PI);
        let m = rot_z(&IDENTITY, spin);
        p.vel = xyz(apply(&m, [a.sin(), 0.0, a.cos(), 1.0]));
        p.accel = scale3(norm(sub([0.0, 0.0, 1.0], p.vel)), c[C_SPEED]);
        for i in 0..3 {
            p.rot_vel[i] = r(c[C_ROT_VEL + i], c[C_ROT_VEL_RANGE + i]) * PI;
        }
        for i in 0..3 {
            p.rot[i] = r(c[C_ROT + i], c[C_ROT_RANGE + i]) * TWO_PI;
        }
        drop(r);
        p.rot = add(p.rot, self.angle);
        for (i, flag) in [0x100, 0x200, 0x400].into_iter().enumerate() {
            if def.flags & flag != 0 && rng.libc.rand() & 1 == 0 {
                p.rot[i] = -p.rot[i];
            }
        }
        if def.flags & 2 != 0 && rng.libc.rand() & 1 == 0 {
            p.rot_vel[2] = -p.rot_vel[2];
        }
        if def.flags & 1 != 0 {
            p.rot_vel[2] = -p.rot_vel[2];
        }
        let mut r = |a: f32, range: f32| rng.vu0.float_range(a, a + range);
        p.life = r(c[C_LIFE], c[C_LIFE_RANGE]) * 30.0;
        p.age = 0.0;
        if p.life <= p.age {
            return false;
        }
        p.tex = if self.tex_a == self.tex_b {
            self.tex_a
        } else {
            self.tex_a.wrapping_add(self.tex_b)
        };
        let (lo, hi) = (0.015_625, 0.984_375);
        p.uv = if def.flags & D_UV_FLIP != 0 && p.rot_vel[2] < 0.0 {
            [[hi, lo], [lo, lo], [hi, hi], [lo, hi]]
        } else {
            [[lo, lo], [lo, hi], [hi, lo], [hi, hi]]
        };
        p.frame = rng
            .vu0
            .float_range(0.0, (i32::from(self.frames) - 1) as f32);
        if def.flags & 8 != 0 {
            p.size_osc_period = c[C_SIZE_OSC_TIME] * 30.0;
            p.size_osc_time = 0.0;
            if 0.0 < c[C_SIZE_OSC_TIME] {
                p.size_osc_amp = c[C_SIZE_OSC + 1] - c[C_SIZE_OSC];
                p.size_osc_base = c[C_SIZE_OSC];
                p.flags |= Q_SIZE_OSC;
            }
        }
        if def.flags & D_COLOR_OSC != 0 {
            p.color_osc_period = c[C_OSC_TIME] * 30.0;
            p.color_osc_time = 0.0;
            if p.color_osc_time < p.color_osc_period {
                for ch in 0..3 {
                    p.color_osc_amp[ch] = c[C_OSC_HI + ch] - c[C_OSC_LO + ch];
                    p.color_mul[ch] = c[C_OSC_LO + ch];
                }
                for ch in 0..3 {
                    if p.color_osc_amp[ch] != 0.0 {
                        p.flags |= Q_OSC[ch];
                    }
                }
            }
        } else {
            p.color_mul = [1.0; 3];
        }
        let mut r = |a: f32, range: f32| rng.vu0.float_range(a, a + range);
        let sz: [f32; 3] = std::array::from_fn(|i| r(c[C_SIZE_MUL + i], c[C_SIZE_MUL_RANGE + i]));
        p.size_mul = sz[0];
        p.size_step[0] = (sz[1] - sz[0]) / (p.life * def.speed_split);
        p.size_step[1] = (sz[2] - sz[1]) / (p.life * (1.0 - def.speed_split));
        p.size = r(def.base_size, def.base_size_range);
        p.half_size = p.size;
        let sp: [f32; 3] = std::array::from_fn(|i| r(c[C_SPEED_MUL + i], c[C_SPEED_MUL_RANGE + i]));
        p.speed_mul = sp[0];
        p.speed_step[0] = (sp[1] - sp[0]) / (p.life * def.speed_split);
        p.speed_step[1] = (sp[2] - sp[1]) / (p.life * (1.0 - def.speed_split));
        p.speed = r(def.base_speed, def.base_speed_range);
        drop(r);
        // Colour mode 2 is static on a coin flip, 1 always.
        if def.color_mode == 1 || def.color_mode == 2 && rng.libc.rand() & 1 == 0 {
            p.flags |= Q_STATIC_COLOR;
        }
        p.ramp_start = c[C_RAMP_START];
        p.ramp_end = c[C_RAMP_END];
        p.ramp_time = 0.0;
        p.ramp_len = p.life * (p.ramp_end - p.ramp_start);
        if p.ramp_len != p.life && p.ramp_time < p.ramp_len {
            p.flags |= Q_COLOR_RAMP;
        }
        let mut r = |a: f32, range: f32| rng.vu0.float_range(a, a + range);
        let start: V3 = std::array::from_fn(|i| r(c[C_COLOR + i], c[C_COLOR_RANGE + i]));
        let end: V3 = std::array::from_fn(|i| r(c[C_END_COLOR + i], c[C_END_COLOR_RANGE + i]));
        let start = start.map(|v| v.clamp(0.0, 255.0));
        let end = end.map(|v| v.clamp(0.0, 255.0));
        p.color = [start[0], start[1], start[2], 0.0];
        p.color_base = start;
        p.color_ramp = sub(end, start);
        p.alpha_base = r(c[C_COLOR + 3], c[C_COLOR_RANGE + 3]).clamp(0.0, 255.0);
        p.fade_in = p.life * def.fade_in_end;
        p.fade_out = p.life * (1.0 - def.fade_out_start);
        true
    }

    /// `EftQuad_StepAll`: one frame of every quad, its corners, and the end of those whose life is over (or of all
    /// of them when the emitter was killed).
    fn step_all(&mut self, def: &QuadDef, pool: &mut Pool) {
        self.dir[1] = self.dir[1].clamp(-1.0, 1.0);
        let pitch = wrap_angle((-self.dir[1]).asin());
        let yaw = wrap_angle(self.dir[0].atan2(self.dir[2]));
        let mut k = 0;
        while k < self.list.len() {
            let slot = self.list[k];
            let mut q = pool.quads[slot];
            let t = if q.age >= q.life { 1.0 } else { q.age / q.life };
            if q.flags & Q_DEAD == 0 {
                self.step_quad(def, &mut q, t);
            }
            if q.color[3] <= 0.0 || q.half_size <= 0.0 {
                q.flags &= !Q_VISIBLE;
            } else {
                let at = if self.flags & E_OWN_ORIGIN != 0 {
                    q.origin
                } else {
                    self.pos
                };
                self.build_corners(def, &mut q, at, pitch, yaw);
                q.flags |= Q_VISIBLE;
            }
            let gone = q.flags & Q_DEAD != 0 || self.flags & E_KILL != 0;
            if gone {
                q.flags = 0;
            }
            pool.quads[slot] = q;
            if gone {
                self.list.remove(k);
            } else {
                k += 1;
            }
        }
    }

    fn step_quad(&self, def: &QuadDef, q: &mut Quad, t: f32) {
        let mut osc = 1.0;
        if q.flags & Q_SIZE_OSC != 0 {
            let s = (q.size_osc_time / q.size_osc_period).clamp(0.0, 1.0);
            osc = q.size_osc_base + q.size_osc_amp * s;
            if q.flags & Q_SIZE_DOWN == 0 {
                q.size_osc_time += 1.0;
            } else {
                q.size_osc_time -= 1.0;
            }
            if q.size_osc_time < 0.0 || q.size_osc_period <= q.size_osc_time {
                q.size_osc_time = clamp(q.size_osc_time, 0.0, q.size_osc_period);
                q.flags ^= Q_SIZE_DOWN;
            }
        }
        let leg = if t < def.speed_split { 0 } else { 1 };
        q.speed_mul += q.speed_step[leg];
        q.size_mul += q.size_step[leg];
        if q.speed_mul < 0.0 {
            q.speed_mul = 0.0;
        }
        if q.size_mul < 0.0 {
            q.size_mul = 0.0;
        }
        q.half_size = q.size * q.size_mul * osc;
        let speed = q.speed * q.speed_mul;
        q.rot = add(q.rot, q.rot_vel).map(wrap_angle);
        q.vel = add(q.vel, q.accel);
        q.pos = add(q.pos, scale3(q.vel, speed));
        let mut mul = [1.0f32; 3];
        if def.flags & D_COLOR_OSC != 0 {
            let u = if 0.0 < q.color_osc_period {
                (q.color_osc_time / q.color_osc_period).clamp(0.0, 1.0)
            } else {
                1.0
            };
            for ch in 0..3 {
                mul[ch] = if q.flags & Q_OSC[ch] != 0 {
                    q.color_mul[ch] + q.color_osc_amp[ch] * u
                } else {
                    q.color_mul[ch]
                }
                .clamp(0.0, 1.0);
            }
            if q.flags & Q_OSC_DOWN == 0 {
                q.color_osc_time += 1.0;
                if q.color_osc_period <= q.color_osc_time {
                    q.color_osc_time = q.color_osc_period;
                    q.flags |= Q_OSC_DOWN;
                }
            } else {
                q.color_osc_time -= 1.0;
                if q.color_osc_time <= 0.0 {
                    q.color_osc_time = 0.0;
                    q.flags &= !Q_OSC_DOWN;
                }
            }
        }
        if q.flags & Q_STATIC_COLOR == 0 {
            let mut r = 0.0;
            if q.flags & Q_COLOR_RAMP != 0 {
                if q.ramp_start <= t && t < q.ramp_end {
                    r = q.ramp_time / q.ramp_len;
                    q.ramp_time += 1.0;
                } else if q.ramp_end <= t {
                    r = 1.0;
                }
            } else {
                r = t;
            }
            for ch in 0..3 {
                q.color[ch] = (q.color_base[ch] + q.color_ramp[ch] * r) * mul[ch];
            }
        } else if def.flags & D_COLOR_OSC != 0 {
            for ch in 0..3 {
                q.color[ch] = q.color_base[ch] * mul[ch];
            }
        }
        let mut fade = 1.0;
        if t < def.fade_in_end && 0.0 < q.fade_in {
            fade = q.age / q.fade_in;
            if 1.0 < fade {
                fade = 1.0;
            }
        }
        if def.fade_out_start <= t && 0.0 < q.fade_out {
            fade = 1.0 - (q.age - (q.life - q.fade_out)) / q.fade_out;
            if fade < 0.0 {
                fade = 0.0;
            }
        }
        q.color[3] = q.alpha_base * fade;
        q.color = q.color.map(|v| v.clamp(0.0, 255.0));
        if self.flags & E_SHEET != 0 {
            let mut frame = q.frame as i32 as u32 as u8;
            if frame >= self.frames {
                frame = 0;
            }
            let f = self.uv[usize::from(frame)];
            q.uv = if def.flags & D_UV_FLIP != 0 && q.rot_vel[2] < 0.0 {
                [[f[2], f[1]], [f[0], f[1]], [f[2], f[3]], [f[0], f[3]]]
            } else {
                [[f[0], f[1]], [f[2], f[1]], [f[0], f[3]], [f[2], f[3]]]
            };
            // The step is never 0 in the game's data (the original would divide by zero).
            let step = i32::from(def.frame_step);
            if step != 0 && (q.age as i32) % step == 0 {
                q.frame += 1.0;
                if q.frame as i32 as u32 as u8 >= self.frames {
                    q.frame = 0.0;
                }
            }
        }
        q.age += 1.0;
        if q.life <= q.age {
            q.flags |= Q_DEAD;
        }
    }

    /// `EftQuad_BuildCorners`: a square of the quad's half size spun by its angles, moved to its position (times
    /// the emitter's size), turned to the emitter's direction and moved to `at`, all scaled by the emitter's size.
    fn build_corners(&self, def: &QuadDef, q: &mut Quad, at: V3, pitch: f32, yaw: f32) {
        // Vu0Cur_LoadIdentity, Vu0Cur_ScaleDiagUniform: the diagonal of the identity.
        let mut m: Mtx = IDENTITY;
        for (i, row) in m.iter_mut().enumerate().take(3) {
            row[i] *= self.size;
        }
        let m = rot_z(&m, q.rot[2]);
        let m = rot_x(&m, q.rot[0]);
        let mut m = rot_y(&m, q.rot[1].sin(), q.rot[1].cos());
        let p = scale3(q.pos, self.size);
        for i in 0..3 {
            m[3][i] += p[i];
        }
        let m = rot_x(&m, pitch);
        let mut m = rot_y(&m, yaw.sin(), yaw.cos());
        for i in 0..3 {
            m[3][i] += at[i];
        }
        let h = if def.flags & D_CROSS == 0 {
            q.half_size
        } else {
            q.half_size * 0.5
        };
        let corners = [[-h, -h], [h, -h], [-h, h], [h, h]];
        for (c, xy) in q.corner.iter_mut().zip(corners) {
            *c = apply(&m, [xy[0], xy[1], 0.0, 1.0]);
        }
    }

    /// `EftQuad_Draw` (the caller tests the owner's view): `EftQuad_DrawAllFacing` / `EftQuad_DrawAllSprite`. With
    /// definition flag 0x40 each quad is drawn as four quarter quads, mirrored to meet in the middle.
    pub(crate) fn draw(
        &self,
        def: &QuadDef,
        pool: &Pool,
        table: &[usize],
        ot: &mut OrderTable,
        ctx: &DrawCtx,
    ) {
        if self.table.is_none() {
            return;
        }
        const MIRROR: [[usize; 4]; 4] = [[0, 1, 2, 3], [1, 0, 3, 2], [2, 3, 0, 1], [3, 2, 1, 0]];
        let front = self.flags & E_VIEW_ONLY != 0;
        for &slot in &self.list {
            let q = &pool.quads[slot];
            if q.flags & Q_VISIBLE == 0 {
                continue;
            }
            let texture = table.get(usize::from(q.tex)).copied();
            let corners = q.corner.map(xyz);
            if def.flags & D_CROSS == 0 {
                draw_quad(def, &corners, q.uv, q.color, texture, front, ot, ctx);
            } else {
                for (i, order) in MIRROR.iter().enumerate() {
                    let d = sub(corners[i], corners[3 - i]);
                    let c = corners.map(|c| add(scale3(d, 0.5), c));
                    let uv = order.map(|j| q.uv[j]);
                    draw_quad(def, &c, uv, q.color, texture, front, ot, ctx);
                }
            }
        }
    }
}

/// `EftQuad_DrawFacing` (two clipped triangles, 0-1-2 and 1-2-3, at twice the average depth slot) or
/// `EftQuad_DrawSprite` (one strip).
#[allow(clippy::too_many_arguments)]
fn draw_quad(
    def: &QuadDef,
    c: &[V3; 4],
    uv: [[f32; 2]; 4],
    color: V4,
    texture: Option<usize>,
    front: bool,
    ot: &mut OrderTable,
    ctx: &DrawCtx,
) {
    if def.flags & D_FACING != 0 {
        for i in 0..2 {
            let tri = std::array::from_fn(|k| ClipVtx {
                pos: point(c[i + k]),
                uv: [uv[i + k][0], uv[i + k][1], 1.0, 1.0],
                col: color,
            });
            let prim = Prim {
                texture,
                ot_layer: def.layer,
                z: ZSort::Average(2.0),
                front,
                clamp: false,
                alpha_min: 0.01,
            };
            queue_tri_clip(ot, ctx, tri, prim);
        }
    } else {
        queue_strip(
            ot, ctx, c, uv, color, def.layer, texture, front, false, true,
        );
    }
}

/// `EFTY_CLAMP`: `x < lo ? lo : (hi < x ? hi : x)`.
fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if hi < x {
        hi
    } else {
        x
    }
}
