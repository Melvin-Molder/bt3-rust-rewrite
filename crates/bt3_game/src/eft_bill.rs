//! Effect pack part kind 16, one sprite per emitter (`EftBill_*` / `EftLine_*`, src/battle/eft_ground_dust.c): a
//! camera-facing sprite, or a quad along the emitter's direction, with keyed colour, size and spin, size and colour
//! pulses, a texture sheet and fades.

use bt3_data::eft_pack::{BillDef, BillDef2};

use crate::eft::{
    ClipVtx, DrawCtx, OrderTable, Prim, SetRef, V3, V4, ZSort, add, cross, draw_sprite, norm,
    point, quad_depth, queue_strip, queue_tri_clip, scale3, sub, wrap_angle,
};
use crate::rng::LibcRandom;

/// `EftBillMgr_Init`: 40 sprite tasks.
pub(crate) const BILLS: usize = 40;

// EFT_BILL_*.
const B_ALIVE: u32 = 0x1;
pub(crate) const B_ENDING: u32 = 0x2;
pub(crate) const B_DEAD: u32 = 0x4;
const B_VISIBLE: u32 = 0x8;
const B_NO_LIFE: u32 = 0x10;
pub(crate) const B_FRONT: u32 = 0x20;
const B_FADED_IN: u32 = 0x40;
const B_FADED_OUT: u32 = 0x80;
const B_W_DOWN: u32 = 0x100;
const B_H_DOWN: u32 = 0x200;
const B_COL_DOWN: u32 = 0x400;

/// `(f32)rand() / RAND_MAX`.
fn rand01(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// `EftBillKey` (`EftLineCur`): the current values of the key animation.
#[derive(Clone, Copy, Debug, Default)]
struct Key {
    color: V4,
    pulse_col: [f32; 3],
    pulse_col_amp: [f32; 3],
    pulse_col_time: f32,
    spin: f32,
    w: f32,
    h: f32,
    pulse_w: f32,
    pulse_w_amp: f32,
    pulse_w_time: f32,
    pulse_h: f32,
    pulse_h_amp: f32,
    pulse_h_time: f32,
}

/// `EftLine_Animate` between keys `k0` and `k1` at `t` (`EftLine_SetKey` is `k0 == k1`): seconds become frames,
/// half turns radians, {low, high} pairs {low, range}.
fn key_at(def: &BillDef, def2: &BillDef2, k0: usize, k1: usize, t: f32) -> Key {
    let l = |a: f32, b: f32| a + (b - a) * t;
    let mut k = Key {
        color: std::array::from_fn(|i| l(def2.color[k0][i], def2.color[k1][i])),
        ..Key::default()
    };
    for c in 0..3 {
        let lo = l(def2.pulse[c][k0][0], def2.pulse[c][k1][0]);
        let hi = l(def2.pulse[c][k0][1], def2.pulse[c][k1][1]);
        k.pulse_col[c] = lo;
        k.pulse_col_amp[c] = hi - lo;
    }
    k.pulse_col_time = l(def2.pulse_time[k0], def2.pulse_time[k1]) * 30.0;
    k.spin = l(def.spin[k0], def.spin[k1]) * 3.141_592_7;
    k.w = l(def.w[k0], def.w[k1]);
    k.h = l(def.h[k0], def.h[k1]);
    let lo = l(def.pulse_w[0][k0], def.pulse_w[0][k1]);
    k.pulse_w = lo;
    k.pulse_w_amp = l(def.pulse_w[1][k0], def.pulse_w[1][k1]) - lo;
    k.pulse_w_time = l(def.pulse_w[2][k0], def.pulse_w[2][k1]) * 30.0;
    let lo = l(def.pulse_h[0][k0], def.pulse_h[0][k1]);
    k.pulse_h = lo;
    k.pulse_h_amp = l(def.pulse_h[1][k0], def.pulse_h[1][k1]) - lo;
    k.pulse_h_time = l(def.pulse_h[2][k0], def.pulse_h[2][k1]) * 30.0;
    k
}

/// A sprite task (`EftBill`).
#[derive(Clone, Debug)]
pub(crate) struct Bill {
    pub id: u32,
    chr: usize,
    pub set: SetRef,
    pub pair: usize,
    texture: Option<usize>,
    pos: V3,
    dir: V3,
    size: f32,
    key: Key,
    color: V4,
    uv: [f32; 4],
    pub flags: u32,
    frame: u8,
    frame_count: i32,
    du: f32,
    dv: f32,
    frame_timer: f32,
    frame_time: f32,
    life: f32,
    pub delay: f32,
    pub end_delay: f32,
    key_timer: f32,
    key_time: f32,
    key_split: f32,
    angle: f32,
    spin: f32,
    w: f32,
    h: f32,
    pulse_w_timer: f32,
    pulse_h_timer: f32,
    pulse_col_timer: f32,
    fade_in: f32,
    fade_out: f32,
    fade_in_len: f32,
    fade_out_len: f32,
}

impl Bill {
    /// `EftBill_Init`: the life, the first key, the random angle and spin, the fades and the sheet.
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
        def: &BillDef,
        def2: &BillDef2,
        libc: &mut LibcRandom,
    ) -> Self {
        let mut b = Self {
            id,
            chr,
            set,
            pair,
            texture,
            pos,
            dir,
            size,
            key: Key::default(),
            color: [0.0; 4],
            uv: [0.0; 4],
            flags: B_ALIVE,
            frame: 0,
            frame_count: 0,
            du: 0.0,
            dv: 0.0,
            frame_timer: 0.0,
            frame_time: 0.0,
            life: 0.0,
            delay: 0.0,
            end_delay: 0.0,
            key_timer: 0.0,
            key_time: 0.0,
            key_split: 0.0,
            angle: 0.0,
            spin: 0.0,
            w: 0.0,
            h: 0.0,
            pulse_w_timer: 0.0,
            pulse_h_timer: 0.0,
            pulse_col_timer: 0.0,
            fade_in: 0.0,
            fade_out: 0.0,
            fade_in_len: 0.0,
            fade_out_len: 0.0,
        };
        if life <= 0.0 {
            b.flags |= B_NO_LIFE;
        } else {
            b.life = life * 30.0;
        }
        if def.flags & 1 != 0 {
            b.key_time = def.key_time * 30.0;
            b.key_split = b.key_time * def.key_split;
            b.key = key_at(def, def2, 0, 0, 0.0);
        } else {
            b.key = key_at(def, def2, 2, 2, 0.0);
        }
        b.angle = def.angle_range * rand01(libc) * 3.141_592_7;
        b.spin = def.spin_range * rand01(libc) * 3.141_592_7;
        b.w = b.key.w;
        b.h = b.key.h;
        b.color = [b.key.color[0], b.key.color[1], b.key.color[2], 0.0];
        b.fade_in_len = def.fade_in * 30.0;
        if b.fade_in_len <= 0.0 {
            b.flags |= B_FADED_IN;
        }
        b.fade_out_len = def.fade_out * 30.0;
        if b.fade_out_len <= 0.0 {
            b.flags |= B_FADED_OUT;
        } else {
            b.fade_out = b.fade_out_len;
        }
        if def.flags & 8 != 0 {
            b.frame_count = i32::from(def.cols) * i32::from(def.rows);
            b.frame_time = def.frame_time * 30.0;
            b.du = 1.0 / f32::from(def.cols);
            b.dv = 1.0 / f32::from(def.rows);
        } else {
            b.uv = [0.0, 1.0, 0.0, 1.0];
            b.du = 1.0;
            b.dv = 1.0;
        }
        b
    }

    /// `EftBill_IsAlive`.
    pub(crate) fn alive(&self) -> bool {
        self.flags & B_ALIVE != 0
    }

    pub(crate) fn chr(&self) -> usize {
        self.chr
    }

    /// `EftBill_SetPos`.
    pub(crate) fn set_pos(&mut self, pos: V3) {
        self.pos = pos;
    }

    /// `EftBill_SetDir`.
    pub(crate) fn set_dir(&mut self, dir: V3) {
        self.dir = norm(dir);
    }

    /// `EftBill_SetSize`.
    pub(crate) fn set_size(&mut self, size: f32) {
        self.size = size;
    }

    /// `EftBill_Stop` (flag 0x800 needs `EftBill_SetUnkD8`, which the emitters never call).
    pub(crate) fn stop(&mut self) {
        self.flags |= B_ENDING;
    }

    /// `EftBill_Update`.
    pub(crate) fn update(&mut self, def: &BillDef, def2: &BillDef2, stopped: bool) {
        if !stopped {
            if self.delay <= 0.0 {
                self.step(def, def2);
            } else {
                self.delay -= 1.0;
            }
            if self.flags & B_ENDING != 0 {
                if self.end_delay > 0.0 {
                    self.end_delay -= 1.0;
                }
                if self.end_delay <= 0.0 && self.color[3] <= 0.0 {
                    self.flags |= B_DEAD;
                }
            }
        }
    }

    fn step(&mut self, def: &BillDef, def2: &BillDef2) {
        let key = self.key;
        self.angle = wrap_angle(self.angle + key.spin + self.spin);
        self.w = key.w;
        self.h = key.h;
        if def.flags & 4 != 0 {
            pulse(
                &mut self.w,
                &mut self.pulse_w_timer,
                &mut self.flags,
                B_W_DOWN,
                [key.pulse_w, key.pulse_w_amp, key.pulse_w_time],
            );
            pulse(
                &mut self.h,
                &mut self.pulse_h_timer,
                &mut self.flags,
                B_H_DOWN,
                [key.pulse_h, key.pulse_h_amp, key.pulse_h_time],
            );
        }
        self.w *= self.size;
        self.h *= self.size;
        if def.flags & 8 != 0 {
            // The sheet frame is a byte; a zero column count does not occur with the sheet flag.
            let cols = def.cols.max(1);
            self.uv[0] = f32::from(self.frame % cols) * self.du;
            self.uv[1] = self.uv[0] + self.du;
            self.uv[2] = f32::from(self.frame / cols) * self.dv;
            self.uv[3] = self.uv[2] + self.dv;
            self.frame_timer += 1.0;
            if self.frame_time <= self.frame_timer {
                self.frame_timer -= self.frame_time;
                self.frame = self.frame.wrapping_add(1);
                if i32::from(self.frame) >= self.frame_count {
                    self.frame = 0;
                }
            }
        }
        self.color = key.color;
        if self.flags & B_FADED_IN == 0 {
            let r = self.fade_in / self.fade_in_len;
            self.fade_in += 1.0;
            self.color[3] *= r;
            if self.fade_in_len <= self.fade_in {
                self.flags |= B_FADED_IN;
                self.color[3] = key.color[3];
            }
        }
        if self.flags & B_ENDING != 0 && self.end_delay <= 0.0 {
            if self.flags & B_FADED_OUT == 0 {
                let r = self.fade_out / self.fade_out_len;
                self.fade_out -= 1.0;
                self.color[3] *= r;
                if self.fade_out <= 0.0 {
                    self.color[3] = 0.0;
                    self.flags |= B_FADED_OUT;
                }
            } else {
                self.color[3] = 0.0;
            }
        }
        if def.flags & 0x10 != 0 && key.pulse_col_time > 0.0 {
            let r = self.pulse_col_timer / key.pulse_col_time;
            for c in 0..3 {
                self.color[c] *= key.pulse_col[c] + key.pulse_col_amp[c] * r;
            }
            if self.flags & B_COL_DOWN == 0 {
                self.pulse_col_timer += 1.0;
                if key.pulse_col_time <= self.pulse_col_timer {
                    self.pulse_col_timer = key.pulse_col_time;
                    self.flags |= B_COL_DOWN;
                }
            } else {
                self.pulse_col_timer -= 1.0;
                if self.pulse_col_timer <= 0.0 {
                    self.pulse_col_timer = 0.0;
                    self.flags &= !B_COL_DOWN;
                }
            }
        }
        if self.color[3] <= 0.0 || self.w <= 0.0 || self.h <= 0.0 {
            self.flags &= !B_VISIBLE;
        } else {
            self.flags |= B_VISIBLE;
        }
        if def.flags & 1 != 0 && self.key_timer <= self.key_time {
            // EftLine_Animate: keys 0..1 before the split, 1..2 after.
            self.key = if self.key_timer < self.key_split {
                key_at(def, def2, 0, 1, self.key_timer / self.key_split)
            } else {
                let t = (self.key_timer - self.key_split) / (self.key_time - self.key_split);
                key_at(def, def2, 1, 2, t)
            };
            self.key_timer += 1.0;
            if self.key_time <= self.key_timer {
                self.key = key_at(def, def2, 2, 2, 0.0);
            }
        }
        if self.flags & B_NO_LIFE == 0 {
            self.life -= 1.0;
            if self.life <= 0.0 {
                self.flags |= B_ENDING;
            }
        }
    }

    /// `EftBill_Draw` (the caller tests the owner's view for a front sprite).
    pub(crate) fn draw(&self, def: &BillDef, ot: &mut OrderTable, ctx: &DrawCtx) {
        if self.flags & B_VISIBLE == 0 {
            return;
        }
        let front = self.flags & B_FRONT != 0;
        let [u0, u1, v0, v1] = self.uv;
        match def.shape {
            0 => {
                if def.flags & 2 != 0 {
                    quad_depth(
                        ot,
                        ctx,
                        self.pos,
                        self.color,
                        def.layer,
                        front,
                        self.texture,
                        [self.w * 0.0625, self.h * 0.0625],
                        [u0, v0, u1, v1],
                        self.angle,
                    );
                } else {
                    draw_sprite(
                        ot,
                        ctx,
                        self.pos,
                        self.color,
                        [self.w, self.h],
                        [u0, v0, u1, v1],
                        self.angle,
                        def.layer,
                        front,
                        self.texture,
                    );
                }
            }
            1 => {
                // EftLine_DrawSprite / EftLine_DrawClipped: from pos -/+ the half width across the direction (and
                // the view) to the same two points plus the direction times the length.
                let side = norm(cross(self.dir, sub(self.pos, ctx.eye)));
                let ofs = scale3(side, self.w);
                let len = scale3(self.dir, self.h);
                let p0 = add(self.pos, ofs);
                let p1 = sub(self.pos, ofs);
                let quad = [p0, p1, add(p0, len), add(p1, len)];
                let st: [[f32; 2]; 4] = std::array::from_fn(|i| {
                    [(i % 2) as f32 * self.du + u0, (i / 2) as f32 * self.dv + v0]
                });
                if def.flags & 2 != 0 {
                    self.draw_clipped(def, &quad, st, ot, ctx);
                } else {
                    queue_strip(
                        ot,
                        ctx,
                        &quad,
                        st,
                        self.color,
                        def.layer,
                        self.texture,
                        front,
                        true,
                        false,
                    );
                }
            }
            _ => {}
        }
    }

    /// `EftLine_DrawClipped`: the quad as two clipped triangles at the depth slot of the four projected depths.
    fn draw_clipped(
        &self,
        def: &BillDef,
        quad: &[V3; 4],
        st: [[f32; 2]; 4],
        ot: &mut OrderTable,
        ctx: &DrawCtx,
    ) {
        // The projection's result is not tested. (After a point off the GS area the original stops projecting and
        // sums whatever the rest of its buffer held; here all four depths are summed.)
        let z = quad
            .iter()
            .map(|&c| ctx.project(c).2)
            .fold(0i32, i32::wrapping_add)
            >> 10;
        for i in 0..2 {
            let tri = std::array::from_fn(|k| ClipVtx {
                pos: point(quad[i + k]),
                uv: [st[i + k][0], st[i + k][1], 1.0, 0.0],
                col: self.color,
            });
            let prim = Prim {
                texture: self.texture,
                ot_layer: def.layer,
                z: ZSort::Fixed(z),
                front: self.flags & B_FRONT != 0,
                clamp: false,
                alpha_min: 0.01,
            };
            queue_tri_clip(ot, ctx, tri, prim);
        }
    }
}

/// A ping-pong size pulse: `k` is {low, range, frames per half period}.
fn pulse(v: &mut f32, t: &mut f32, flags: &mut u32, down: u32, k: [f32; 3]) {
    let [lo, range, time] = k;
    if time <= 0.0 {
        return;
    }
    *v *= lo + range * (*t / time);
    if *flags & down == 0 {
        *t += 1.0;
        if time <= *t {
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
