//! Effect pack part kind 0, the ray burst (`EftRay_*`, src/battle/eft_chain.c): up to 48 thin quads radiating from
//! a point, in the world facing the camera or on the screen, with the common effect file's ray texture; they shrink
//! away (mode 0) or are re-rolled every frame (modes 1..3), then fade.

use crate::battle::{IDENTITY, Mtx, apply, inverse_rt};
use crate::eft::{
    ClipVtx, DrawCtx, EffectBlend, EffectTri, EffectVertex, OrderTable, Prim, V3, V4, ZSort,
    depth_slot, point, queue_tri_clip, rotate_xyz,
};
use crate::rng::LibcRandom;

/// `EftRayMgr_Init`: four bursts.
pub(crate) const RAYS: usize = 4;
/// `EFT_RAY_MAX`.
const RAY_MAX: usize = 48;

const PI_F: f32 = 3.141_592_7;

// EFT_RAY_MODE_*.
pub(crate) const MODE_SPIN: u32 = 1;
const MODE_TECH: u32 = 2;
pub(crate) const MODE_CUT: u32 = 3;

// EFT_RAY_*.
const R_TIMED: u32 = 0x1;
const R_FADING: u32 = 0x2;
const R_FADED: u32 = 0x4;
const R_DEAD: u32 = 0x8;
const R_AUTOKILL: u32 = 0x10;

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

/// `EftRayArg`: what a burst is created with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RayArg {
    pub pos: V3,
    /// r, g, b, and the alpha each ray takes less up to 15 (modes 1..3 re-roll it below this).
    pub color: [i32; 4],
    /// Seconds before the fade; <= 0: no timed fade.
    pub life: f32,
    pub length: f32,
    pub width: f32,
    pub inner: f32,
    pub jitter: f32,
    pub mode: u32,
    pub count: u32,
    pub chr: usize,
    /// The ordering-table layer.
    pub blend: u8,
    /// 0 in the world, 1 on the screen.
    pub space: u32,
    pub delay: i32,
    pub fade_frames: i32,
    pub auto_kill: bool,
}

/// `EftRay`: one ray.
#[derive(Clone, Copy, Debug, Default)]
struct Ray {
    rgba: [u8; 4],
    angle: f32,
    length: f32,
    width: f32,
    offset: f32,
}

/// A burst (`EftRayWork`).
#[derive(Clone, Debug)]
pub(crate) struct RayBurst {
    pub id: u32,
    chr: usize,
    pub set: crate::eft::SetRef,
    rays: Vec<Ray>,
    pos: V3,
    scale: [f32; 2],
    shrink: f32,
    life: f32,
    alpha: f32,
    alpha_step: f32,
    inner: f32,
    alpha_range: i32,
    blend: u8,
    space: u32,
    mode: u32,
    delay: i32,
    pub flags: u32,
}

impl RayBurst {
    /// `EftRay_Init` / `EftRay_Setup`: the rays spread evenly with up to 4 degrees of noise, their alpha, length,
    /// width and offset at random (five libc draws each, six on the screen). At most 48 rays.
    pub(crate) fn new(
        id: u32,
        set: crate::eft::SetRef,
        arg: &RayArg,
        libc: &mut LibcRandom,
    ) -> Self {
        let mut w = Self {
            id,
            chr: arg.chr,
            set,
            rays: Vec::new(),
            pos: arg.pos,
            scale: [1.0, 1.0],
            shrink: 1.0 / (arg.life * 0.9 * 30.0),
            life: arg.life,
            alpha: 1.0,
            alpha_step: 1.0 / arg.fade_frames as f32,
            inner: arg.inner,
            alpha_range: arg.color[3],
            blend: arg.blend,
            space: arg.space,
            mode: arg.mode,
            delay: arg.delay,
            flags: 0,
        };
        if 0.0 < w.life {
            w.flags |= R_TIMED;
        }
        if arg.auto_kill {
            w.flags |= R_AUTOKILL;
        }
        let count = (arg.count as usize).min(RAY_MAX);
        let step = 360.0 / arg.count as f32;
        for i in 0..count {
            let noise = (libc.rand() % 5) as f32;
            let angle = wrap(step * i as f32 * PI_F / 180.0 + noise * PI_F / 180.0);
            let a = arg.color[3] - libc.rand() % 16;
            let length = arg.length + rand01(libc) * 20.0;
            let mut width = (arg.width * (rand01(libc) * 5.0)).max(0.4);
            if w.space == 1 {
                width = arg.width + rand01(libc) * 4.0;
            }
            let offset = rand01(libc) * arg.jitter;
            w.rays.push(Ray {
                rgba: [
                    arg.color[0] as u8,
                    arg.color[1] as u8,
                    arg.color[2] as u8,
                    a as u8,
                ],
                angle,
                length,
                width,
                offset,
            });
        }
        w
    }

    pub(crate) fn chr(&self) -> usize {
        self.chr
    }

    /// `EftRay_IsAlive`: a burst whose fade is over is reported dead once, and ends itself.
    pub(crate) fn poll_alive(&mut self) -> bool {
        if self.flags & R_FADED != 0 {
            self.flags |= R_DEAD;
            return false;
        }
        true
    }

    /// `EftRay_SetPos`: unless its fade is over.
    pub(crate) fn set_pos(&mut self, pos: V3) {
        if self.flags & (R_FADED | R_DEAD) == 0 {
            self.pos = pos;
        }
    }

    /// `EftRay_Kill`: it fades out and ends itself.
    pub(crate) fn kill(&mut self) {
        self.flags |= R_FADING | R_AUTOKILL;
    }

    /// `EftRay_Update`: the delay, the life, the fade, then the mode (0 shrinks; 1..3 turn the burst by a random
    /// angle and re-roll every alpha; mode 2 ends when either fighter is in a technique, 3 when a technique camera
    /// cut runs). Returns whether the task ends.
    pub(crate) fn update(
        &mut self,
        stopped: bool,
        in_technique: bool,
        cam_cut: bool,
        libc: &mut LibcRandom,
    ) -> bool {
        if stopped {
            return false;
        }
        if self.delay > 0 {
            self.delay -= 1;
            return false;
        }
        if self.flags & R_TIMED != 0 {
            self.life -= 1.0 / 30.0;
            if self.life < 0.0 {
                self.life = 0.0;
                self.flags |= R_FADING;
            }
        }
        if self.flags & R_FADING != 0 {
            self.alpha -= self.alpha_step;
            if self.alpha < 0.0 {
                self.alpha = 0.0;
                self.flags |= R_FADED;
                if self.flags & R_AUTOKILL != 0 {
                    self.flags |= R_DEAD;
                }
            }
        }
        if self.flags & R_DEAD != 0 {
            return true;
        }
        match self.mode {
            0 => self.scale[0] = (self.scale[0] - self.shrink).max(0.0),
            _ => {
                if (self.mode == MODE_TECH && in_technique) || (self.mode == MODE_CUT && cam_cut) {
                    self.life = -1.0;
                    self.alpha = -1.0;
                }
                // EFT_RAY_REROLL (a zero alpha range does not occur: the emitters give 0x70 and 0x80).
                let turn = wrap((libc.rand() % 360) as f32 * PI_F / 180.0);
                let range = self.alpha_range.max(1);
                for r in &mut self.rays {
                    r.rgba[3] = (libc.rand() % range) as u8;
                    r.angle = wrap(r.angle + turn);
                }
            }
        }
        false
    }

    /// `EftRay_DrawRays`: each ray a quad from its near end (inner + offset from the centre) outwards, turned by its
    /// angle about the view axis. A burst with an owner is drawn only in the owner's view (or during a camera cut)
    /// and not while the owner is in clash A; a screen burst of mode 2 / 3 centres on the opponent's node 3.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &self,
        texture: Option<usize>,
        in_view: bool,
        cam_cut: bool,
        clash_a: bool,
        opp_node3: V3,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
    ) {
        if (!cam_cut && !in_view) || clash_a {
            return;
        }
        const BASE: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [-1.0, 0.0], [1.0, 0.0]];
        let mut pos = self.pos;
        let mtx: Mtx = if self.space == 0 {
            let mut m = inverse_rt(&ctx.world2view);
            m[3] = point(pos);
            m
        } else {
            if self.mode == MODE_TECH || self.mode == MODE_CUT {
                let v = apply(&ctx.world2screen, point(opp_node3));
                let q = 1.0 / v[3];
                let (x, y, z) = (
                    (v[0] * q * 16.0) as i32,
                    (v[1] * q * 16.0) as i32,
                    (v[2] * q) as i32,
                );
                if z < 0 {
                    return;
                }
                // Mtx_ProjectPoint's GS units less the frame offset, in pixels.
                pos = [((x - 0x7000) >> 4) as f32, ((y - 0x7200) >> 4) as f32, 0.0];
            }
            let mut m = IDENTITY;
            m[3] = point(pos);
            m
        };
        for ray in &self.rays {
            let rz = rotate_xyz(&IDENTITY, [0.0, 0.0, ray.angle]);
            let corners: [V4; 4] = std::array::from_fn(|j| {
                let v = [
                    BASE[j][0] * ray.width * self.scale[0],
                    BASE[j][1] * ray.length * self.scale[1] - (self.inner + ray.offset),
                    0.0,
                    1.0,
                ];
                apply(&mtx, apply(&rz, v))
            });
            let a = (f32::from(ray.rgba[3]) * self.alpha) as u32 as u8;
            let col = [
                f32::from(ray.rgba[0]),
                f32::from(ray.rgba[1]),
                f32::from(ray.rgba[2]),
                f32::from(a),
            ];
            if self.space == 0 {
                draw_quad_3d(&corners, col, self.blend, texture, ot, ctx);
            } else {
                draw_quad_2d(&corners, col, self.blend, texture, ot);
            }
        }
    }
}

/// `EftRay_DrawQuad3D` / `EftRay_DrawClipped`: two clipped triangles, every GS depth at the near limit, each fan
/// triangle in the depth slot of its average depth.
fn draw_quad_3d(
    corners: &[V4; 4],
    col: V4,
    blend: u8,
    texture: Option<usize>,
    ot: &mut OrderTable,
    ctx: &DrawCtx,
) {
    const ST: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0], [1.0, 0.0]];
    for i in 0..2 {
        let tri = std::array::from_fn(|k| ClipVtx {
            pos: corners[i + k],
            uv: [ST[i + k][0], ST[i + k][1], 1.0, 1.0],
            col,
        });
        let prim = Prim {
            texture,
            ot_layer: blend,
            z: ZSort::Average(1.0),
            front: true,
            clamp: false,
            alpha_min: 0.01,
        };
        queue_tri_clip(ot, ctx, tri, prim);
    }
}

/// `EftRay_DrawQuad2D`: a quad on the screen (corners in pixels) cut into four strips along its length, in depth
/// slot 0 with the GS depth at the near limit.
fn draw_quad_2d(
    corners: &[V4; 4],
    col: V4,
    blend: u8,
    texture: Option<usize>,
    ot: &mut OrderTable,
) {
    const SEGS: f32 = 4.0;
    let d = |a: usize, b: usize| -> V3 {
        std::array::from_fn(|i| (corners[a][i] - corners[b][i]) / SEGS)
    };
    let (d0, d1) = (d(2, 0), d(3, 1));
    let mut p0: V3 = [corners[0][0], corners[0][1], corners[0][2]];
    let mut p1: V3 = [corners[1][0], corners[1][1], corners[1][2]];
    let (mut t0, mut t1) = (1.0f32, 1.0f32);
    let step = (0.0 - 1.0) / SEGS;
    let rgba = col.map(|c| c as i32 as u8);
    let l = if blend >= 2 { blend - 2 } else { blend };
    let blend_eq = EffectBlend::of(usize::from(l), blend >= 2);
    let mut t = 1.0 / SEGS;
    while t <= 1.0 {
        let p2: V3 = std::array::from_fn(|i| p0[i] + d0[i]);
        let p3: V3 = std::array::from_fn(|i| p1[i] + d1[i]);
        let (t2, t3) = (t0 + step, t1 + step);
        // Vec4_ToInt, then the GS position in whole pixels.
        let vert = |p: V3, s: f32, tt: f32| EffectVertex {
            position: [p[0] as i32 as f32, p[1] as i32 as f32, 0xFF_FFFF as f32],
            stq: [s, tt, 1.0],
            rgba,
        };
        let v = [
            vert(p0, 0.0, t0),
            vert(p1, 1.0, t1),
            vert(p2, 0.0, t2),
            vert(p3, 1.0, t3),
        ];
        for k in 0..2 {
            ot.add(
                depth_slot(0),
                usize::from(l),
                EffectTri {
                    texture,
                    blend: blend_eq,
                    clamp: false,
                    zwrite: false,
                    vertices: [v[k], v[k + 1], v[k + 2]],
                },
            );
        }
        p0 = p2;
        p1 = p3;
        t0 = t2;
        t1 = t3;
        t += 1.0 / SEGS;
    }
}
