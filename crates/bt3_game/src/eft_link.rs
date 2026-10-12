//! Effect pack part kind 15, the sprite chain (`EftLink_*`, src/battle/eft_link_1.c and eft_link_2.c): sprites
//! laid out from one point to another, one every `spacing` (pushed along the chain at random), each turning,
//! drifting off the chain, sized, coloured and faded over its life, and laid out again in place once its life is
//! over while the chain still has room for it. The sprites come from a pool of 200 shared by every chain; they draw
//! from the VU0 generator (and libc `rand()` for coin flips and the sheet frame).

use bt3_data::eft_pack::{LinkDef, LinkDef2};

use crate::battle::{IDENTITY, apply};
use crate::eft::{
    ClipVtx, DrawCtx, OrderTable, Prim, SetRef, V3, V4, ZSort, add, norm, point, queue_strip,
    queue_tri_clip, rotate_xyz, scale3, sub, xyz,
};
use crate::eft_part10::{billboard, billboard_clipped};
use crate::rng::{LibcRandom, Vu0Random};

/// `EftLinkMgr_Init`: six chains.
pub(crate) const CHAINS: usize = 6;
/// `EFT_LINK_NODES`: the sprite pool.
const NODES: usize = 200;

const PI_F: f32 = 3.141_592_7;
const TWO_PI: f32 = 6.283_185_3;

// EFT_LINK_*.
const L_ALIVE: u32 = 0x1;
const L_DEAD: u32 = 0x2;
const L_STOPPED: u32 = 0x4;
const L_STOPPING: u32 = 0x8;
const L_KILLED: u32 = 0x10;
const L_FADING: u32 = 0x20;
const L_SHEET: u32 = 0x100;
const L_NODEPTH: u32 = 0x200;
const L_STOP_ASKED: u32 = 0x1000;
const L_GROWING: u32 = 0x2000;
const L_KEYED: u32 = 0x4000;
const L_KEY2: u32 = 0x8000;
/// `EftLink_Kill`: killed and stopped.
pub(crate) const L_KILL: u32 = L_KILLED | L_STOPPED;

// Sprite flags.
const N_PLACED: u32 = 0x1;
const N_DRAW: u32 = 0x40;
const N_MIRROR: u32 = 0x80;
const N_ON_CHAIN: u32 = 0x400;
const N_RAMP: u32 = 0x800;
const N_PULSE_DOWN: u32 = 0x1_0000;
const N_MUL_DOWN: u32 = 0x2_0000;

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

fn clamp_to(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if hi < x {
        hi
    } else {
        x
    }
}

/// `EftWLinkNode`: one sprite; it stays in the pool between uses (a field a new sprite does not set keeps its
/// last value, as the sheet frame does without definition flag 0x80).
#[derive(Clone, Copy, Debug, Default)]
struct Node {
    corner: [V3; 4],
    ofs: [f32; 2],
    /// The drift offset off the chain.
    dir: V3,
    origin: V3,
    col: V4,
    col0: V4,
    col_delta: V3,
    uv0: V4,
    uv1: V4,
    rot_x: f32,
    rot_z: f32,
    twist: f32,
    spin: f32,
    size: f32,
    size_vel0: f32,
    size_vel1: f32,
    fade_in: f32,
    fade_out: f32,
    age: f32,
    life: f32,
    scale: f32,
    pulse_t: f32,
    pulse_time: f32,
    pulse0: f32,
    pulse_d: f32,
    mul_time: f32,
    mul_t: f32,
    mul_d: [f32; 3],
    mul0: [f32; 3],
    fade_t: f32,
    fade_time: f32,
    tex_frame: f32,
    delay: f32,
    flags: u32,
}

/// The shared pool (`gEftLink`): the sprites, and where the search for a free one starts.
#[derive(Clone, Debug)]
pub(crate) struct Pool {
    nodes: Vec<Node>,
    next: usize,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            nodes: vec![Node::default(); NODES],
            next: 0,
        }
    }
}

impl Pool {
    /// Sprites in use.
    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.nodes.iter().filter(|n| n.flags != 0).count()
    }
}

// The one-value tracks, in the emitter's order (`val[16]`, then the rest).
const T_ROT_X: usize = 0;
const T_ROT_X_R: usize = 1;
const T_ROT_Z: usize = 2;
const T_ROT_Z_R: usize = 3;
const T_DIR_ANG: usize = 4;
const T_DIR_ANG_R: usize = 5;
const T_SPIN: usize = 6;
const T_SPIN_R: usize = 7;
const T_TWIST: usize = 8;
const T_OFS_X: usize = 9;
const T_OFS_Y: usize = 10;
const T_ALONG_MIN: usize = 11;
const T_ALONG_MAX: usize = 12;
const T_DIST: usize = 13;
const T_DIST_R: usize = 14;
const T_DIST_BASE: usize = 15;
const T_LIFE: usize = 16;
const T_LIFE_R: usize = 17;
const T_WAIT: usize = 18;
const T_WAIT_R: usize = 19;
const T_PULSE_TIME: usize = 20;
const T_MUL_TIME: usize = 21;
const T_FADE0: usize = 22;
const T_FADE1: usize = 23;
const T_SIZE: usize = 24;
const T_SIZE_R: usize = 27;
const T_PULSE: usize = 30;
const T_MUL: usize = 32;
const TRACKS: usize = 38;

fn track(def: &LinkDef, def2: &LinkDef2, t: usize) -> [f32; 3] {
    match t {
        T_ROT_X => def.rot_x,
        T_ROT_X_R => def.rot_x_range,
        T_ROT_Z => def.rot_z,
        T_ROT_Z_R => def.rot_z_range,
        T_DIR_ANG => def.dir_ang,
        T_DIR_ANG_R => def.dir_ang_range,
        T_SPIN => def.spin,
        T_SPIN_R => def.spin_range,
        T_TWIST => def.twist,
        T_OFS_X => def.ofs_x,
        T_OFS_Y => def.ofs_y,
        T_ALONG_MIN => def.along_min,
        T_ALONG_MAX => def.along_max,
        T_DIST => def.dist,
        T_DIST_R => def.dist_range,
        T_DIST_BASE => def.dist_base,
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

/// The colour tracks: start colour, its range, end colour, its range.
fn color_track(def2: &LinkDef2, c: usize) -> &[[f32; 4]; 3] {
    match c {
        0 => &def2.color,
        1 => &def2.color_range,
        2 => &def2.end_color,
        _ => &def2.end_color_range,
    }
}

/// A chain task (`EftLinkWork`).
#[derive(Clone, Debug)]
pub(crate) struct Link {
    pub id: u32,
    chr: usize,
    pub set: SetRef,
    pub pair: usize,
    texture: Option<usize>,
    pos: V3,
    pos2: V3,
    size: f32,
    /// Seconds it runs before it stops by itself; <= 0 until stopped.
    life: f32,
    v: [f32; TRACKS],
    d: [f32; TRACKS],
    color: [V4; 4],
    d_color: [V4; 4],
    key_dur: f32,
    key_split: f32,
    dir: V3,
    time: f32,
    frames: f32,
    delay: f32,
    stop_delay: f32,
    fade: f32,
    fade_max: f32,
    uv: [[f32; 4]; 16],
    grow_time: f32,
    grow_dur: f32,
    scale: f32,
    scale_step: [f32; 2],
    spacing: f32,
    spacing_step: [f32; 2],
    pitch: f32,
    yaw: f32,
    pub flags: u32,
    /// Its sprites, entries of the pool in chain order.
    list: Vec<usize>,
}

impl Link {
    /// `EftLink_Init`: the growth (flag 0x200, three VU0 draws) or the first spacing, the keys (flag 0x400) or key
    /// 2, the sheet's cells.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u32,
        chr: usize,
        set: SetRef,
        pair: usize,
        texture: Option<usize>,
        pos: V3,
        pos2: V3,
        size: f32,
        life: f32,
        def: &LinkDef,
        def2: &LinkDef2,
        vu0: &mut Vu0Random,
    ) -> Self {
        let mut w = Self {
            id,
            chr,
            set,
            pair,
            texture,
            pos,
            pos2,
            size,
            life,
            v: [0.0; TRACKS],
            d: [0.0; TRACKS],
            color: [[0.0; 4]; 4],
            d_color: [[0.0; 4]; 4],
            key_dur: 0.0,
            key_split: 0.0,
            dir: [0.0; 3],
            time: 0.0,
            frames: 0.0,
            delay: 0.0,
            stop_delay: 0.0,
            fade: 0.0,
            fade_max: 0.0,
            uv: [[0.0; 4]; 16],
            grow_time: 0.0,
            grow_dur: 0.0,
            scale: 0.0,
            scale_step: [0.0; 2],
            spacing: 0.0,
            spacing_step: [0.0; 2],
            pitch: 0.0,
            yaw: 0.0,
            flags: 0,
            list: Vec::new(),
        };
        if def.flags & 0x200 != 0 {
            w.init_grow(def, vu0);
            w.flags |= L_GROWING;
        } else {
            w.spacing = def.spacing[0];
            w.scale = 1.0;
        }
        if def.flags & 0x400 != 0 {
            w.init_keys(def, def2);
            w.flags |= L_KEYED;
        } else {
            w.set_key(def, def2, 2);
        }
        if def.cols >= 2 || def.rows >= 2 {
            w.frames = f32::from(def.cols) * f32::from(def.rows);
            let d = [1.0 / f32::from(def.cols), 1.0 / f32::from(def.rows)];
            let mut n = 0;
            for j in 0..def.rows {
                for i in 0..def.cols {
                    // The sheet has room for 16 cells.
                    if let Some(c) = w.uv.get_mut(n) {
                        let (fi, fj) = (f32::from(i), f32::from(j));
                        *c = [d[0] * fi, d[1] * fj, d[0] * fi + d[0], d[1] * fj + d[1]];
                    }
                    n += 1;
                }
            }
            w.flags |= L_SHEET;
        }
        if def.flags & 2 != 0 {
            w.flags |= L_NODEPTH;
        }
        w.flags |= L_ALIVE;
        w
    }

    pub(crate) fn alive(&self) -> bool {
        self.flags & L_ALIVE != 0
    }

    pub(crate) fn chr(&self) -> usize {
        self.chr
    }

    /// `EftLink_Stop`: after its stop delay and / or fade when it has them, else at once; once.
    pub(crate) fn stop(&mut self) {
        if self.flags & L_STOP_ASKED != 0 || !self.alive() {
            return;
        }
        let mut now = true;
        if 0.0 < self.stop_delay {
            self.flags |= L_STOPPING;
            now = false;
        }
        if 0.0 < self.fade_max {
            self.flags |= L_FADING;
            now = false;
        }
        if now {
            self.flags |= L_STOPPED;
        }
        self.flags |= L_STOP_ASKED;
    }

    /// `EftLink_SetFade`.
    pub(crate) fn set_fade(&mut self, frames: f32) {
        if self.alive() {
            self.fade = frames;
            self.fade_max = frames;
        }
    }

    /// `EftLink_SetDelay`: the frames are stored divided by 30 (and counted down a frame at a time).
    pub(crate) fn set_delay(&mut self, frames: f32) {
        if self.alive() {
            self.delay = frames / 30.0;
        }
    }

    /// `EftLink_SetStopDelay`.
    pub(crate) fn set_stop_delay(&mut self, frames: f32) {
        if self.alive() {
            self.stop_delay = frames;
        }
    }

    /// `EftLink_SetPos`.
    pub(crate) fn set_pos(&mut self, pos: V3) {
        if self.alive() {
            self.pos = pos;
        }
    }

    /// `EftLink_SetPos2`.
    pub(crate) fn set_pos2(&mut self, pos: V3) {
        if self.alive() {
            self.pos2 = pos;
        }
    }

    /// `EftLink_SetSize`.
    pub(crate) fn set_size(&mut self, size: f32) {
        if self.alive() {
            self.size = size;
        }
    }

    /// `EftLink_Term`: its sprites back to the pool.
    pub(crate) fn free(&mut self, pool: &mut Pool) {
        for i in self.list.drain(..) {
            pool.nodes[i].flags = 0;
        }
        self.flags = 0;
    }

    /// `EftLink_InitGrow`: three random scales and the steps of scale and spacing between them.
    fn init_grow(&mut self, def: &LinkDef, vu0: &mut Vu0Random) {
        self.grow_time = 0.0;
        self.grow_dur = def.grow_time * 30.0;
        let s: [f32; 3] = std::array::from_fn(|i| {
            vu0.float_range(def.scale[i], def.scale[i] + def.scale_range[i])
        });
        let (first, rest) = (
            self.grow_dur * def.grow_split,
            self.grow_dur * (1.0 - def.grow_split),
        );
        self.scale_step = [(s[1] - s[0]) / first, (s[2] - s[1]) / rest];
        self.scale = s[0];
        self.spacing_step = [
            (def.spacing[1] - def.spacing[0]) / first,
            (def.spacing[2] - def.spacing[1]) / rest,
        ];
        self.spacing = def.spacing[0];
    }

    /// `EftLink_UpdateGrow`.
    fn update_grow(&mut self, def: &LinkDef) {
        let s = usize::from(self.grow_time / self.grow_dur >= def.grow_split);
        self.scale += self.scale_step[s];
        self.spacing += self.spacing_step[s];
        self.grow_time += 1.0;
        if self.grow_dur <= self.grow_time {
            self.flags &= !L_GROWING;
        }
    }

    /// `EftLink_InitKeys`: at time 0 the first interval (keys 0 to 1), from the split the second; every track's
    /// change over it.
    fn init_keys(&mut self, def: &LinkDef, def2: &LinkDef2) {
        let mut k = 0;
        if self.time <= 0.0 {
            k = 1;
            self.key_dur = def.key_time * 30.0;
            self.key_split = self.key_dur * def.key_split;
        }
        if self.key_split <= self.time && self.flags & L_KEY2 == 0 {
            k = 2;
            self.flags |= L_KEY2;
        }
        if k > 0 {
            for t in 0..TRACKS {
                let v = track(def, def2, t);
                self.d[t] = v[k] - v[k - 1];
            }
            for c in 0..4 {
                let v = color_track(def2, c);
                self.d_color[c] = std::array::from_fn(|i| v[k][i] - v[k - 1][i]);
            }
        }
    }

    /// `EftLink_UpdateKeys`: every track at its interval's first key plus its change times the progress (not
    /// clamped).
    fn update_keys(&mut self, def: &LinkDef, def2: &LinkDef2) {
        let (k, t) = if self.flags & L_KEY2 == 0 {
            (0, self.time / self.key_split)
        } else {
            (
                1,
                (self.time - self.key_split) / (self.key_dur - self.key_split),
            )
        };
        for i in 0..TRACKS {
            self.v[i] = track(def, def2, i)[k] + self.d[i] * t;
        }
        for c in 0..4 {
            let v = color_track(def2, c)[k];
            self.color[c] = std::array::from_fn(|i| v[i] + self.d_color[c][i] * t);
        }
    }

    /// `EftLink_SetKey`.
    fn set_key(&mut self, def: &LinkDef, def2: &LinkDef2, k: usize) {
        for t in 0..TRACKS {
            self.v[t] = track(def, def2, t)[k];
        }
        for c in 0..4 {
            self.color[c] = color_track(def2, c)[k];
        }
    }

    /// `EftLink_Update`: nothing while stopped or delayed; else the fade, growth, keys, placement (until it stops),
    /// the sprites, the life and the stop delay. Returns whether the task ends.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        def: &LinkDef,
        def2: &LinkDef2,
        stopped: bool,
        pool: &mut Pool,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
    ) -> bool {
        if stopped {
            return false;
        }
        if self.delay > 0.0 {
            self.delay -= 1.0;
            return false;
        }
        let mut fade = 1.0;
        if self.flags & L_FADING != 0 && self.flags & L_STOPPING == 0 {
            let r = self.fade / self.fade_max;
            if r < 0.0 {
                fade = 0.0;
            } else if r <= 1.0 {
                fade = r;
            }
            self.fade -= 1.0;
            if self.fade < 0.0 {
                self.flags &= !L_ALIVE;
                self.flags |= L_KILLED;
            }
        }
        if self.flags & L_GROWING != 0 {
            self.update_grow(def);
        }
        if self.flags & L_KEYED != 0 {
            self.update_keys(def, def2);
        }
        if self.flags & L_STOPPED == 0 {
            self.place(def, pool, vu0, libc);
        }
        self.step(def, fade, pool);
        self.time += 1.0;
        if self.flags & L_KEYED != 0 {
            self.init_keys(def, def2);
            if self.time >= self.key_dur {
                self.set_key(def, def2, 2);
                self.flags &= !L_KEYED;
            }
        }
        if self.life * 30.0 <= self.time && self.life > 0.0 {
            if self.fade_max > 0.0 {
                self.flags |= L_STOP_ASKED | L_FADING;
            } else {
                self.flags |= L_STOPPED;
            }
        }
        if self.flags & L_STOPPING != 0 {
            self.stop_delay -= 1.0;
            if self.stop_delay <= 0.0 {
                self.flags &= !L_STOPPING;
                if self.flags & L_FADING == 0 {
                    self.flags |= L_STOPPED;
                }
            }
        }
        if self.flags & L_STOPPED != 0 && self.list.is_empty() {
            self.flags &= !L_ALIVE;
            self.flags |= L_DEAD;
        }
        self.flags & (L_KILLED | L_DEAD) != 0
    }

    /// `EftLink_Place`: one sprite every `spacing` from `pos` towards `pos2`, each pushed along the chain at random
    /// (a VU0 draw per sprite); a missing sprite is taken from the pool, one whose life ran out is laid out again.
    fn place(
        &mut self,
        def: &LinkDef,
        pool: &mut Pool,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
    ) {
        let d = sub(self.pos2, self.pos);
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len <= 0.0 {
            return;
        }
        let count = len / self.spacing;
        self.dir = norm(d);
        let a = -self.dir[1];
        self.pitch = wrap(a.clamp(-1.0, 1.0).asin());
        self.yaw = wrap(self.dir[0].atan2(self.dir[2]));
        let mut ni = 0;
        for i in 0..count as i32 {
            let fi = i as f32;
            let mut p = add(self.pos, scale3(self.dir, len * (fi / count)));
            let along = vu0.float_range(self.v[T_ALONG_MIN], self.v[T_ALONG_MAX]);
            p = add(p, scale3(self.dir, along));
            if ni >= self.list.len() && !self.new_node(def, p, fi, pool, vu0, libc) {
                continue;
            }
            let idx = self.list[ni];
            if pool.nodes[idx].flags & N_PLACED == 0 {
                self.init_node(idx, def, pool, vu0, libc);
                let n = &mut pool.nodes[idx];
                n.origin = p;
                n.twist = wrap(self.v[T_TWIST] * TWO_PI * fi);
                n.flags |= N_PLACED;
            }
            pool.nodes[idx].flags |= N_ON_CHAIN;
            ni += 1;
        }
    }

    /// `EftLink_NewNode`: a free sprite of the pool (round robin), started at `pos`, appended.
    fn new_node(
        &mut self,
        def: &LinkDef,
        pos: V3,
        t: f32,
        pool: &mut Pool,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
    ) -> bool {
        if pool.next >= NODES {
            pool.next = 0;
        }
        let start = pool.next;
        let mut i = start;
        loop {
            let idx = i;
            i = (i + 1) % NODES;
            if pool.nodes[idx].flags == 0 {
                self.init_node(idx, def, pool, vu0, libc);
                let n = &mut pool.nodes[idx];
                n.origin = pos;
                n.twist = wrap(self.v[T_TWIST] * TWO_PI * t);
                n.flags |= N_PLACED;
                self.list.push(idx);
                pool.next = i;
                return true;
            }
            if i == start {
                return false;
            }
        }
    }

    /// `EftLink_InitNode`: a sprite from the current track values (18 VU0 draws; libc for the spin sign with flag
    /// 0x100, the sheet frame with 0x80 and the colour ramp with 0x20 | 0x40). As in the original, its mirrored flag
    /// is cleared again: only the UVs stay mirrored.
    fn init_node(
        &self,
        idx: usize,
        def: &LinkDef,
        pool: &mut Pool,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
    ) {
        let v = &self.v;
        let mut r = |a: f32, b: f32| vu0.float_range(a, b);
        let p = &mut pool.nodes[idx];
        p.ofs = [v[T_OFS_X], v[T_OFS_Y]];
        p.rot_x = wrap(r(v[T_ROT_X], v[T_ROT_X] + v[T_ROT_X_R]) * TWO_PI);
        p.rot_z = wrap(r(v[T_ROT_Z], v[T_ROT_Z] + v[T_ROT_Z_R]) * TWO_PI);
        p.twist = 0.0;
        let (spin, spin_r) = (v[T_SPIN], v[T_SPIN_R]);
        p.spin = wrap(r(spin, spin + spin_r) * PI_F);
        if def.flags & 0x100 != 0 {
            let mut flip = true;
            if spin < 0.0 && 0.0 < spin + spin_r {
                flip = false;
            }
            if 0.0 < spin && spin + spin_r < 0.0 {
                flip = false;
            }
            if libc.int_range(0, 0x400) & 1 == 0 && flip {
                p.spin = -p.spin;
            }
        }
        let a = wrap(r(v[T_DIR_ANG], v[T_DIR_ANG] + v[T_DIR_ANG_R]) * TWO_PI);
        let dist = v[T_DIST_BASE] + r(v[T_DIST], v[T_DIST] + v[T_DIST_R]);
        p.dir = [a.cos() * dist, a.sin() * dist, 0.0];
        if def.flags & 8 != 0 && p.spin < p.dir[2] {
            p.uv0 = [0.984_375, 0.015_625, 0.015_625, 0.015_625];
            p.uv1 = [0.984_375, 0.984_375, 0.015_625, 0.984_375];
            p.flags |= N_MIRROR;
        } else {
            p.uv0 = [0.015_625, 0.015_625, 0.015_625, 0.984_375];
            p.uv1 = [0.984_375, 0.015_625, 0.984_375, 0.984_375];
        }
        if def.flags & 0x80 != 0 {
            p.tex_frame = libc.int_range(0, (self.frames - 1.0) as i32) as f32;
        }
        p.life = r(v[T_LIFE], v[T_LIFE] + v[T_LIFE_R]) * 30.0;
        p.age = 0.0;
        p.delay = r(v[T_WAIT], v[T_WAIT] + v[T_WAIT_R]) * 30.0;
        p.fade_in = p.life * def.fade_in;
        p.fade_out = p.life * (1.0 - def.fade_out);
        p.flags = 0;
        if def.flags & 0x20 == 0 {
            p.flags = N_RAMP;
        } else if def.flags & 0x40 != 0 && libc.rand() & 1 == 0 {
            p.flags |= N_RAMP;
        }
        p.fade_time = p.life * (v[T_FADE1] - v[T_FADE0]);
        p.fade_t = 0.0;
        let [c0, c0r, c1, c1r] = self.color;
        let mut col0: V4 = std::array::from_fn(|i| r(c0[i], c0[i] + c0r[i]));
        let mut col1: V4 = std::array::from_fn(|i| r(c1[i], c1[i] + c1r[i]));
        col0 = col0.map(|c| c.clamp(0.0, 255.0));
        col1 = col1.map(|c| c.clamp(0.0, 255.0));
        p.col0 = col0;
        p.col_delta = std::array::from_fn(|i| col1[i] - col0[i]);
        p.col = col0;
        let sz: [f32; 3] =
            std::array::from_fn(|i| r(v[T_SIZE + i], v[T_SIZE + i] + v[T_SIZE_R + i]));
        p.size_vel0 = (sz[1] - sz[0]) / (p.life * def.size_split);
        p.size_vel1 = (sz[2] - sz[1]) / (p.life * (1.0 - def.size_split));
        p.size = sz[0];
        if def.flags & 0x800 != 0 {
            p.pulse_time = v[T_PULSE_TIME] * 30.0;
            p.pulse_t = 0.0;
            p.pulse_d = v[T_PULSE + 1] - v[T_PULSE];
            p.pulse0 = v[T_PULSE];
            p.scale = v[T_PULSE];
        } else {
            p.scale = 1.0;
        }
        if def.flags & 0x1000 != 0 {
            p.mul_time = v[T_MUL_TIME] * 30.0;
            p.mul_t = 0.0;
            for c in 0..3 {
                p.mul_d[c] = v[T_MUL + c * 2 + 1] - v[T_MUL + c * 2];
                p.mul0[c] = v[T_MUL + c * 2];
            }
        }
    }

    /// `EftLink_StepNodes`: every sprite's delay, size pulse, colour multiplier (unlike kind 10, every channel each
    /// time), size, spin, fades, colour ramp, sheet frame and corners; a sprite whose life is over is laid out again
    /// if the chain placed it this frame (and does not stop), else it goes back to the pool.
    fn step(&mut self, def: &LinkDef, alpha: f32, pool: &mut Pool) {
        let mut mul = [1.0f32; 3];
        let (fade0, fade1) = (self.v[T_FADE0], self.v[T_FADE1]);
        let stopped = self.flags & L_STOPPED != 0;
        let mut k = 0;
        while k < self.list.len() {
            let idx = self.list[k];
            let p = &mut pool.nodes[idx];
            if 0.0 < p.delay {
                p.delay -= 1.0;
                k += 1;
                continue;
            }
            let t = clamp_to(p.age / p.life, 0.0, 1.0);
            if def.flags & 0x800 != 0 {
                let u = clamp_to(p.pulse_t / p.pulse_time, 0.0, 1.0);
                p.scale = p.pulse0 + p.pulse_d * u;
                p.pulse_t += if p.flags & N_PULSE_DOWN == 0 {
                    1.0
                } else {
                    -1.0
                };
                if p.pulse_t < 0.0 || p.pulse_time <= p.pulse_t {
                    p.flags ^= N_PULSE_DOWN;
                    p.pulse_t = clamp_to(p.pulse_t, 0.0, p.pulse_time);
                }
            }
            if def.flags & 0x1000 != 0 {
                let u = clamp_to(p.mul_t / p.mul_time, 0.0, 1.0);
                for c in 0..3 {
                    mul[c] = p.mul0[c] + p.mul_d[c] * u;
                }
                p.mul_t += if p.flags & N_MUL_DOWN == 0 { 1.0 } else { -1.0 };
                if p.mul_t < 0.0 || p.mul_time <= p.mul_t {
                    p.flags ^= N_MUL_DOWN;
                    p.mul_t = clamp_to(p.mul_t, 0.0, p.mul_time);
                }
            } else {
                mul = [1.0; 3];
            }
            p.size += if t < def.size_split {
                p.size_vel0
            } else {
                p.size_vel1
            };
            p.rot_z = wrap(p.rot_z + p.spin);
            let mut a = if t < def.fade_in {
                p.age / p.fade_in
            } else {
                1.0
            };
            if def.fade_out <= t {
                a = 1.0 - (p.age - (p.life - p.fade_out)) / p.fade_out;
            }
            if p.flags & N_RAMP != 0 {
                let u = clamp_to(p.fade_t / p.fade_time, 0.0, 1.0);
                for c in 0..3 {
                    p.col[c] = (p.col0[c] + p.col_delta[c] * u) * mul[c];
                }
                if fade0 <= t && t < fade1 {
                    p.fade_t += 1.0;
                }
            } else {
                for c in 0..3 {
                    p.col[c] = p.col0[c] * mul[c];
                }
            }
            p.col[3] = p.col0[3] * a * alpha;
            p.col = p.col.map(|c| c.clamp(0.0, 255.0));
            if self.flags & L_SHEET != 0 {
                // The frame as a byte: below 0 the first, past the last the last.
                let f = if p.tex_frame < 0.0 {
                    0
                } else if self.frames - 1.0 < p.tex_frame {
                    (self.frames - 1.0) as u8
                } else {
                    p.tex_frame as u8
                };
                let cell = self.uv[usize::from(f).min(15)];
                if p.flags & N_MIRROR != 0 {
                    p.uv0 = [cell[2], cell[1], cell[0], cell[1]];
                    p.uv1 = [cell[2], cell[3], cell[0], cell[3]];
                } else {
                    p.uv0 = [cell[0], cell[1], cell[2], cell[1]];
                    p.uv1 = [cell[0], cell[3], cell[2], cell[3]];
                }
                // A sheet has a frame step (a zero would trap the original's division).
                let step = i32::from(def.tex_step).max(1);
                if (self.time as i32) % step == 0 {
                    p.tex_frame += 1.0;
                    if self.frames <= p.tex_frame {
                        p.tex_frame = 0.0;
                    }
                }
            }
            if 0.0 < p.col[3] && 0.0 < p.size {
                if def.flags & 0x10 == 0 {
                    build_corners(
                        p,
                        self.size * self.scale,
                        def,
                        self.scale,
                        self.pitch,
                        self.yaw,
                    );
                }
                p.flags |= N_DRAW;
            } else {
                p.flags &= !N_DRAW;
            }
            p.age += 1.0;
            let mut removed = false;
            if p.life <= p.age {
                let old = p.flags;
                p.flags &= !N_PLACED;
                if stopped || old & N_ON_CHAIN == 0 {
                    p.flags = 0;
                    removed = true;
                }
            }
            p.flags &= !N_ON_CHAIN;
            if removed {
                self.list.remove(k);
            } else {
                k += 1;
            }
        }
    }

    /// `EftLink_Draw`: every sprite to draw, facing the camera (definition flag 0x10: at its spot off the chain) or
    /// as a quad through its corners, clipped with definition flag 4.
    pub(crate) fn draw(&self, def: &LinkDef, pool: &Pool, ot: &mut OrderTable, ctx: &DrawCtx) {
        let m = rotate_xyz(
            &rotate_xyz(&IDENTITY, [self.pitch, 0.0, 0.0]),
            [0.0, self.yaw, 0.0],
        );
        let no_depth = self.flags & L_NODEPTH != 0;
        for &idx in &self.list {
            let n = &pool.nodes[idx];
            if n.flags & N_DRAW == 0 {
                continue;
            }
            if def.flags & 0x10 != 0 {
                let mut off = n.dir;
                if def.flags & 0x2000 == 0 {
                    off = scale3(off, self.size * self.scale);
                }
                let pos = add(xyz(apply(&m, point(off))), n.origin);
                let size = n.size * self.size * self.scale * n.scale * 0.5;
                let rot = wrap(n.rot_z + n.twist);
                let uv = [n.uv0[0], n.uv0[1], n.uv1[2], n.uv1[3]];
                if def.flags & 4 != 0 {
                    billboard_clipped(
                        ot,
                        ctx,
                        pos,
                        size,
                        size,
                        n.col,
                        n.ofs,
                        uv,
                        rot,
                        def.layer,
                        no_depth,
                        self.texture,
                    );
                } else {
                    billboard(
                        ot,
                        ctx,
                        pos,
                        n.col,
                        [size * 16.0, size * 16.0],
                        [(n.ofs[0] as i32) << 4, (n.ofs[1] as i32) << 4],
                        uv,
                        def.layer,
                        no_depth,
                        self.texture,
                        rot,
                    );
                }
            } else {
                let uv = [
                    [n.uv0[0], n.uv0[1]],
                    [n.uv0[2], n.uv0[3]],
                    [n.uv1[0], n.uv1[1]],
                    [n.uv1[2], n.uv1[3]],
                ];
                if def.flags & 4 != 0 {
                    for i in 0..2 {
                        let tri = std::array::from_fn(|k| ClipVtx {
                            pos: point(n.corner[i + k]),
                            uv: [uv[i + k][0], uv[i + k][1], 1.0, 1.0],
                            col: n.col,
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
                        &n.corner,
                        uv,
                        n.col,
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

/// `EftLink_BuildCorners`: a square of the sprite's size moved by its offset, scaled (the chain's size and scale
/// times the pulse; the offset is not), turned by its own angles and twist, moved off the chain by its drift
/// (scaled by the chain's scale unless definition flag 0x2000), turned onto the chain's direction and placed at its
/// spot.
fn build_corners(p: &mut Node, s: f32, def: &LinkDef, chain_scale: f32, pitch: f32, yaw: f32) {
    let h = p.size * 0.5;
    let mut m = IDENTITY;
    m[3][0] += p.ofs[0];
    m[3][1] += p.ofs[1];
    let s = s * p.scale;
    for i in 0..3 {
        m[i][i] *= s;
    }
    let m = rotate_xyz(&m, [0.0, 0.0, p.rot_z]);
    let m = rotate_xyz(&m, [p.rot_x, 0.0, 0.0]);
    let mut m = rotate_xyz(&m, [0.0, 0.0, p.twist]);
    let drift = if def.flags & 0x2000 == 0 {
        scale3(p.dir, chain_scale)
    } else {
        p.dir
    };
    for i in 0..3 {
        m[3][i] += drift[i];
    }
    let mut m = rotate_xyz(&rotate_xyz(&m, [pitch, 0.0, 0.0]), [0.0, yaw, 0.0]);
    for i in 0..3 {
        m[3][i] += p.origin[i];
    }
    let corners = [[-h, -h], [-h, h], [h, -h], [h, h]];
    p.corner = corners.map(|c| xyz(apply(&m, [c[0], c[1], 0.0, 1.0])));
}
