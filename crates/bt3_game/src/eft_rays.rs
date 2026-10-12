//! Effect pack part kind 2, the rays (`EftRays_*`, src/battle/eft_rays.c): up to 10 textured quads in the camera
//! plane fanned round a point, each with its keyed size and colour, a width flicker, a colour pulse and a spin; the
//! rays come from a pool of 20 shared by every part.

use bt3_data::eft_pack::RaysDef;

use crate::battle::{IDENTITY, Mtx, apply, inverse_rt};
use crate::eft::{
    ClipVtx, DrawCtx, OrderTable, Prim, SetRef, V3, V4, ZSort, point, queue_tri_clip, rotate_xyz,
};
use crate::rng::LibcRandom;

/// `EftRaysMgr_Init`: three parts.
pub(crate) const PARTS: usize = 3;
/// `EftRaysMgr_Init`: rays on the free list.
pub(crate) const RAY_POOL: usize = 20;

const PI_F: f32 = 3.141_592_7;

// EFT_RAYS_*.
const P_ALIVE: u32 = 0x1;
const P_TIMED: u32 = 0x4;
const P_STOP: u32 = 0x8;
pub(crate) const P_DEAD: u32 = 0x10;

/// `(f32)rand() / RAND_MAX`.
fn rand01(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// `EftMath_WrapAngle`.
fn wrap(a: f32) -> f32 {
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

/// `EftRay`: one ray of a part.
#[derive(Clone, Copy, Debug, Default)]
struct Ray {
    idx: usize,
    color: [u8; 4],
    width: f32,
    height: f32,
    ref_height: f32,
    time: f32,
    total: f32,
    split: f32,
    rot: f32,
    len: f32,
    flick_t: f32,
    flick_time: f32,
    len_a: f32,
    len_b: f32,
    flick_down: bool,
    col: [f32; 3],
    pulse_t: f32,
    pulse_time: f32,
    pulse_down: bool,
    delay: i16,
    shown: bool,
}

impl Ray {
    /// `EftRays_SetKey`: the key's colour and size; the key animation restarts.
    fn set_key(&mut self, def: &RaysDef, key: usize) {
        let rd = &def.rays[self.idx];
        self.time = 0.0;
        self.total = rd.key_time * 30.0;
        self.split = self.total * rd.key_split;
        let k = &rd.key[key];
        self.color = k.color;
        self.width = k.width;
        self.height = k.height;
        self.ref_height = rd.key[2].height;
    }

    /// `EftRays_LerpKey`: key 0 to 1 over the first leg, 1 to 2 over the rest (the colour bytes wrap as the original's
    /// unsigned conversion does; the alpha stays).
    fn lerp_key(&mut self, def: &RaysDef) {
        self.time += 1.0;
        if self.total < self.time {
            self.time = self.total;
        }
        let (t, ka, kb) = if self.time < self.split {
            (self.time / self.split, 0, 1)
        } else {
            ((self.time - self.split) / (self.total - self.split), 1, 2)
        };
        let rd = &def.rays[self.idx];
        let (a, b) = (&rd.key[ka], &rd.key[kb]);
        for c in 0..3 {
            let di = i32::from(b.color[c]) - i32::from(a.color[c]);
            self.color[c] = a.color[c].wrapping_add((di as f32 * t) as i32 as u8);
        }
        self.width = a.width + (b.width - a.width) * t;
        self.height = a.height + (b.height - a.height) * t;
    }
}

/// A part (`EftRays`).
#[derive(Clone, Debug)]
pub(crate) struct RaysPart {
    pub id: u32,
    chr: usize,
    pub set: SetRef,
    pub pair: usize,
    texture: Option<usize>,
    rays: Vec<Ray>,
    pos: V3,
    pub delay: f32,
    pub hold: f32,
    fade_time: f32,
    fade: f32,
    life: f32,
    alpha: f32,
    size: f32,
    pub flags: u32,
}

impl RaysPart {
    /// `EftRays_Init`: one ray per entry of the part data from the pool (as many as it has), each with two random
    /// widths, at key 0 (keyed) or 2.
    pub(crate) fn new(
        id: u32,
        chr: usize,
        set: SetRef,
        pair: usize,
        texture: Option<usize>,
        life: f32,
        def: &RaysDef,
        free: &mut usize,
        libc: &mut LibcRandom,
    ) -> Self {
        let mut w = Self {
            id,
            chr,
            set,
            pair,
            texture,
            rays: Vec::new(),
            pos: [0.0; 3],
            delay: 0.0,
            hold: 0.0,
            fade_time: 0.0,
            fade: 0.0,
            life: 0.0,
            alpha: 1.0,
            size: 1.0,
            flags: P_ALIVE,
        };
        if life > 0.0 {
            w.life = life * 30.0;
            w.flags |= P_TIMED;
        }
        for i in 0..def.count.clamp(0, 10) as usize {
            // EftRays_AllocRay: none left, no more rays.
            if *free == 0 {
                break;
            }
            *free -= 1;
            let rd = &def.rays[i];
            let mut ray = Ray {
                idx: i,
                rot: rd.angle * PI_F / 180.0,
                delay: rd.delay as i16,
                flick_time: rd.flick_time * 30.0,
                len: 1.0,
                pulse_time: rd.col_b[3] * 30.0,
                col: [1.0; 3],
                ..Ray::default()
            };
            ray.len_a = rd.len[0] + rd.len[1] * rand01(libc);
            ray.len_b = rd.len[2] + rd.len[3] * rand01(libc);
            ray.set_key(def, if def.flags & 1 != 0 { 0 } else { 2 });
            w.rays.push(ray);
        }
        w
    }

    pub(crate) fn alive(&self) -> bool {
        self.flags & P_ALIVE != 0
    }

    pub(crate) fn chr(&self) -> usize {
        self.chr
    }

    /// `EftRays_SetPos`.
    pub(crate) fn set_pos(&mut self, pos: V3) {
        self.pos = pos;
    }

    /// `EftRays_SetSize`.
    pub(crate) fn set_size(&mut self, size: f32) {
        self.size = size;
    }

    /// `EftRays_SetFade`.
    pub(crate) fn set_fade(&mut self, frames: f32) {
        self.fade = frames;
        self.fade_time = frames;
    }

    /// `EftRays_Stop`: the hold, then the fade.
    pub(crate) fn stop(&mut self) {
        self.flags |= P_STOP;
    }

    /// `EftRays_Term`: its rays back to the pool.
    pub(crate) fn free(&mut self, free: &mut usize) {
        *free += self.rays.len();
        self.rays.clear();
    }

    /// `EftRays_Update`: the delay, the rays and the life; once stopped, the hold and the fade, at whose end it
    /// dies.
    pub(crate) fn update(&mut self, def: &RaysDef, stopped: bool, libc: &mut LibcRandom) {
        if stopped {
            return;
        }
        let mut alpha = 0.0;
        if self.delay <= 0.0 {
            self.step(def, libc);
            if self.flags & P_TIMED != 0 {
                self.life -= 1.0;
                if self.life <= 0.0 {
                    self.flags |= P_STOP;
                }
            }
        } else {
            self.delay -= 1.0;
        }
        if self.flags & P_STOP != 0 {
            if 0.0 < self.hold {
                self.hold -= 1.0;
            } else {
                if 0.0 < self.fade_time {
                    alpha = self.fade / self.fade_time;
                    self.fade -= 1.0;
                }
                if self.fade <= 0.0 {
                    self.flags |= P_DEAD;
                    self.flags &= !P_ALIVE;
                    alpha = 0.0;
                }
                self.alpha = alpha;
            }
        }
    }

    /// `EftRays_Step`: each ray's delay, keys, width flicker (re-rolled at each turn, two libc draws), colour pulse
    /// and spin.
    fn step(&mut self, def: &RaysDef, libc: &mut LibcRandom) {
        for ray in &mut self.rays {
            let rd = &def.rays[ray.idx];
            ray.delay = ray.delay.wrapping_sub(1);
            if ray.delay <= 0 {
                ray.delay = 0;
                ray.shown = true;
            }
            if !ray.shown {
                continue;
            }
            if def.flags & 1 != 0 {
                ray.lerp_key(def);
            }
            if def.flags & 2 != 0 {
                ray.flick_t += if ray.flick_down { -1.0 } else { 1.0 };
                if ray.flick_t >= ray.flick_time {
                    ray.flick_t = ray.flick_time;
                    ray.len_a = rd.len[0] + rd.len[1] * rand01(libc);
                    ray.len_b = rd.len[2] + rd.len[3] * rand01(libc);
                    ray.flick_down = !ray.flick_down;
                } else if ray.flick_t <= 0.0 {
                    ray.flick_t = 0.0;
                    ray.len_a = rd.len[0] + rd.len[1] * rand01(libc);
                    ray.len_b = rd.len[2] + rd.len[3] * rand01(libc);
                    ray.flick_down = !ray.flick_down;
                }
                let t = ray.flick_t / ray.flick_time;
                ray.len = ray.len_a + (ray.len_b - ray.len_a) * t;
            }
            if def.flags & 4 != 0 {
                ray.pulse_t += if ray.pulse_down { -1.0 } else { 1.0 };
                if ray.pulse_t >= ray.pulse_time {
                    ray.pulse_t = ray.pulse_time;
                    ray.pulse_down = !ray.pulse_down;
                } else if ray.pulse_t <= 0.0 {
                    ray.pulse_t = 0.0;
                    ray.pulse_down = !ray.pulse_down;
                }
                let t = ray.pulse_t / ray.pulse_time;
                for c in 0..3 {
                    ray.col[c] = rd.col_a[c] + (rd.col_b[c] - rd.col_a[c]) * t;
                }
            }
            ray.rot = wrap(ray.rot + rd.spin);
        }
    }

    /// `EftRays_Draw` / `EftRays_DrawRay`: each shown ray a quad in the camera plane, scaled by its width (times the
    /// flicker), height and the part's size, its far edge pushed by the height ratio, turned by the ray's angle and
    /// placed at the part; its colour the key's times the pulse, its alpha times the fade.
    pub(crate) fn draw(&self, def: &RaysDef, ot: &mut OrderTable, ctx: &DrawCtx) {
        const CORNER: [V4; 4] = [
            [-1.0, -1.0, 1.0, 1.0],
            [1.0, -1.0, 1.0, 1.0],
            [-1.0, 0.01, 0.0, 1.0],
            [1.0, 0.01, 0.0, 1.0],
        ];
        const UV: [[f32; 2]; 4] = [[0.0, 0.05], [1.0, 0.05], [0.0, 1.0], [1.0, 1.0]];
        let mut cam: Mtx = inverse_rt(&ctx.world2view);
        cam[3] = [0.0, 0.0, 0.0, cam[3][3]];
        for ray in self.rays.iter().filter(|r| r.shown) {
            let ratio = ray.height / ray.ref_height;
            let mut m = IDENTITY;
            m[0][0] = ray.width * ray.len * self.size;
            m[1][1] = ray.height * self.size;
            m[2][2] = ratio * -10.0 * self.size;
            let m = rotate_xyz(&m, [0.0, 0.0, ray.rot]);
            let out: [V4; 4] = std::array::from_fn(|i| {
                let o = apply(&cam, apply(&m, CORNER[i]));
                point([o[0] + self.pos[0], o[1] + self.pos[1], o[2] + self.pos[2]])
            });
            let alpha = f32::from(ray.color[3]) * self.alpha;
            let color: V4 = [
                (f32::from(ray.color[0]) * ray.col[0]).clamp(0.0, 255.0),
                (f32::from(ray.color[1]) * ray.col[1]).clamp(0.0, 255.0),
                (f32::from(ray.color[2]) * ray.col[2]).clamp(0.0, 255.0),
                alpha.clamp(0.0, 255.0),
            ];
            // EftPrim_DrawTriangle takes a negative layer as layer 0 without blending; the technique sets' rays all
            // blend on layer 1.
            let layer = u8::try_from(def.rays[ray.idx].blend).unwrap_or(0);
            for i in 0..2 {
                let tri = std::array::from_fn(|k| ClipVtx {
                    pos: out[i + k],
                    uv: [UV[i + k][0], UV[i + k][1], 1.0, 1.0],
                    col: color,
                });
                let prim = Prim {
                    texture: self.texture,
                    ot_layer: layer,
                    z: ZSort::Average(1.0),
                    front: false,
                    clamp: false,
                    alpha_min: 0.01,
                };
                queue_tri_clip(ot, ctx, tri, prim);
            }
        }
    }
}
