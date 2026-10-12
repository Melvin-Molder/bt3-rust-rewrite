//! The bubbles under water (`EftBubble_*`, src/battle/eft_stage_1.c; the second task of stage effect kind 7, on a
//! stage whose pack entry 0x12 has at least six textures): a burst around a fighter that dives in, small bubbles
//! from the body of a fighter in the water and in front of the camera while it is under water, rising and wobbling
//! to the surface. A pool of 100 on libc `rand()`, with textures 5..8 of the water's set.

use crate::battle::{IDENTITY, Mtx, apply, inverse_rt};
use crate::btl_tech_volley::{cos_fast, eft_wrap, sin_fast};
use crate::eft::{
    EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, V3, add, depth_slot,
    mtx_mul, point, rotate_xyz, scale3, xyz,
};
use crate::eft_water::{Scr, on_screen, project};
use crate::rng::LibcRandom;

/// `EFT_BUBBLE_MAX`.
const BUBBLES: usize = 100;
const PI_F: f32 = 3.141_592_7;

/// The model nodes the body's bubbles come from: segments from the first node to the second, or the first
/// alone (-1).
const BONES: [(u32, i32); 20] = [
    (0x0B, 0x0A),
    (0x0A, 0x09),
    (0x09, 0x08),
    (0x08, -1),
    (0x0F, 0x0E),
    (0x0E, 0x0D),
    (0x0D, 0x0C),
    (0x0C, -1),
    (0x12, 0x13),
    (0x13, 0x14),
    (0x14, 0x15),
    (0x15, -1),
    (0x20, 0x21),
    (0x21, 0x22),
    (0x22, 0x23),
    (0x23, -1),
    (0x03, 0x10),
    (0x10, 0x11),
    (0x11, 0x30),
    (0x30, -1),
];

/// The nodes `BONES` names, by index into `BubbleChr::nodes`.
pub(crate) const BONE_NODES: usize = 0x31;

/// The PS2's FPU has no infinities: a result too large for a float stays at the largest one.
fn sat(x: f32) -> f32 {
    x.clamp(-f32::MAX, f32::MAX)
}

/// `EFTB_RAND01`.
fn randf(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// A fighter as the bubbles read it (`BtlCharApi_*`): its position, direction of travel, height, flag 0x11 (in
/// the water) and its model nodes 0..0x30.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BubbleChr {
    pub pos: V3,
    pub dir: V3,
    pub height: f32,
    pub in_water: bool,
    pub nodes: [V3; BONE_NODES],
}

/// `EftBubbleChr`: the fighter a side's emitters follow (0 until its first burst) and its burst.
#[derive(Clone, Copy, Debug, Default)]
struct Emitter {
    obj: usize,
    burst: f32,
    bursting: bool,
}

/// `EftBubble`.
#[derive(Clone, Copy, Debug, Default)]
struct Bubble {
    pos: V3,
    vel: V3,
    drag: f32,
    rot: f32,
    size: f32,
    alpha: f32,
    alpha_step: f32,
    rise: f32,
    rise_grow: f32,
    phase: f32,
    amp: f32,
    phase_step: f32,
    /// The view it belongs to (-1: every view).
    view: i32,
    tex: u8,
    layer: u8,
}

/// The arguments of `EftBubble_Add` (the colour is always 0x80 and the layer 0).
struct Add {
    pos: V3,
    vel: V3,
    drag: f32,
    size: f32,
    alpha: f32,
    rot: f32,
    rise: f32,
    rise_grow: f32,
    phase: f32,
    amp: f32,
    view: i32,
    tex: u8,
    phase_step: f32,
    life: f32,
}

/// `gEftBubble`.
#[derive(Clone, Debug)]
pub(crate) struct Bubbles {
    /// `EftWater_GetSurfaceY`.
    level: f32,
    pub split: bool,
    sin_coef: [f32; 4],
    textures: Vec<usize>,
    /// View 0's world-to-view matrix as the cameras left it (`gBtlCam->views[0]`).
    pub cam: Option<Mtx>,
    chr: [Emitter; 2],
    cam_timer: [f32; 2],
    list: Vec<Bubble>,
}

impl Bubbles {
    /// `EftBubble_Init`: none without six textures in the set.
    pub(crate) fn new(level: f32, sin_coef: [f32; 4], textures: Vec<usize>) -> Option<Self> {
        (textures.len() >= 6).then(|| Self {
            level,
            split: false,
            sin_coef,
            textures,
            cam: None,
            chr: [Emitter::default(); 2],
            cam_timer: [0.0; 2],
            list: Vec::new(),
        })
    }

    /// The bubbles alive.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.list.len()
    }

    /// The bubbles a side's burst has left to emit.
    #[cfg(test)]
    pub(crate) fn burst(&self, side: usize) -> f32 {
        self.chr[side].burst
    }

    fn cam_pos(&self) -> Option<V3> {
        self.cam.map(|m| xyz(inverse_rt(&m)[3]))
    }

    /// `EftBubble_StartBurst`: the bubbles of fighter `side` diving in, 5 per unit of its speed, 40 to 60.
    pub(crate) fn start_burst(&mut self, side: usize, speed: f32) {
        let Some(c) = self.chr.get_mut(side) else {
            return;
        };
        if c.bursting {
            return;
        }
        c.obj = side;
        c.burst = (speed * 5.0).clamp(40.0, 60.0);
        c.bursting = true;
    }

    /// `EftBubble_Update`: the bursts, the camera's and the body's bubbles, then every bubble moves (nothing while
    /// paused).
    pub(crate) fn update(&mut self, chars: &[BubbleChr; 2], paused: bool, libc: &mut LibcRandom) {
        if paused {
            return;
        }
        for i in 0..2 {
            // EftBubble_UpdateBurst: `burst` bubbles, 8 fewer each frame.
            let c = self.chr[i];
            if c.bursting {
                if c.burst > 0.0 {
                    self.emit_burst(&chars[c.obj], c.burst as i32, libc);
                    self.chr[i].burst -= 8.0;
                } else {
                    self.chr[i].burst = 0.0;
                    self.chr[i].bursting = false;
                }
            }
        }
        // EftBubble_UpdateAmbient (one view: the second is split screen's, where nothing happens).
        if !self.split
            && let Some(cam) = self.cam
            && self.cam_pos().is_some_and(|p| p[1] >= self.level)
        {
            self.cam_timer[0] += if libc.rand() & 3 != 0 { 0.05 } else { 0.1 };
            if self.cam_timer[0] > 1.0 {
                self.cam_timer[0] -= 1.0;
                self.emit_camera(&cam, 0, libc);
            }
            let c = &chars[self.chr[0].obj];
            if c.in_water {
                self.emit_body(c, 120, 0, libc);
            }
        }
        let cam_y = self.cam_pos().map(|p| p[1]);
        let (level, coef) = (self.level, self.sin_coef);
        self.list.retain_mut(|b| !step(b, level, cam_y, &coef));
    }

    /// `EftBubble_EmitBody`: along 20 segments of the body, points about a quarter of the height apart (at
    /// least 6), each with a 1 in `chance` chance of a small bubble drifting along the fighter's direction.
    fn emit_body(&mut self, c: &BubbleChr, chance: i32, view: i32, libc: &mut LibcRandom) {
        let h = c.height * 0.05;
        let spacing = (c.height * 0.25).max(6.0);
        for (from, to) in BONES {
            let base = c.nodes[from as usize];
            let (step, n) = match usize::try_from(to) {
                Ok(to) => {
                    let b = crate::eft::sub(c.nodes[to], base);
                    let len = crate::eft::dot(b, b).sqrt();
                    if spacing < len {
                        let n = (len / spacing + 1.0) as i32;
                        (scale3(b, 1.0 / n as f32), n)
                    } else {
                        ([0.0; 3], 1)
                    }
                }
                Err(_) => ([0.0; 3], 1),
            };
            for j in 0..n {
                if libc.rand() % chance != 0 {
                    continue;
                }
                let size = randf(libc) * 0.4 + 0.4;
                let amp = randf(libc) * 0.2 + 0.1;
                let rise = randf(libc) * -0.05 - 0.05;
                let life = randf(libc) * 0.4 + 0.6;
                let vel = scale3(c.dir, randf(libc) * 0.3 + 0.7);
                let pos = add(base, scale3(step, j as f32));
                let rot = randf(libc) * 360.0;
                let phase = randf(libc) * PI_F;
                self.add(Add {
                    pos,
                    vel,
                    drag: -8.0,
                    size: size * h,
                    alpha: 128.0,
                    rot,
                    rise: rise * h,
                    rise_grow: 6.0,
                    phase,
                    amp: amp * h,
                    view,
                    tex: 5,
                    phase_step: 0.25,
                    life,
                });
            }
        }
    }

    /// `EftBubble_EmitBurst`: `count` big bubbles. The original's offset vector is never initialised (the
    /// random direction is computed into a copy), so it carries from one bubble to the next: each lies at the
    /// fighter's position plus the last one's position times its distance. Here it starts from zero. The
    /// direction shrinks bubble after bubble the same way.
    fn emit_burst(&mut self, c: &BubbleChr, count: i32, libc: &mut LibcRandom) {
        let h = c.height * 0.05;
        let mut p = [0.0f32; 3];
        let mut dir = c.dir;
        for _ in 0..count {
            let size = randf(libc) * 4.5 + 3.0;
            let rise = randf(libc) * -0.2 - 0.2;
            let amp = randf(libc) * 0.5 + 0.3;
            let dist = randf(libc) * 10.0;
            // EftBubble_RandDir: two draws, its result lost.
            randf(libc);
            randf(libc);
            p = add(scale3(p, dist * h), c.pos).map(sat);
            dir = scale3(dir, randf(libc) * 0.3 + 0.7);
            let rot = randf(libc) * 360.0;
            let phase = randf(libc) * PI_F;
            let tex = if libc.rand() & 3 != 0 { 7 } else { 8 };
            self.add(Add {
                pos: p,
                vel: dir,
                drag: -8.0,
                size: size * h,
                alpha: 112.0,
                rot,
                rise: rise * h,
                rise_grow: 6.0,
                phase,
                amp: amp * h,
                view: -1,
                tex,
                phase_step: 0.5,
                life: -1.0,
            });
        }
    }

    /// `EftBubble_EmitCamera`: one bubble 10 in front of the camera and 20 below it, at a random place of the
    /// screen; one in six big.
    fn emit_camera(&mut self, world2view: &Mtx, view: i32, libc: &mut LibcRandom) {
        let m = inverse_rt(world2view);
        let mut pos = add(scale3(xyz(m[2]), 10.0), xyz(m[3]));
        pos[1] += 20.0;
        let x = (libc.rand() % 2001 - 1000) as f32 * 0.015;
        let y = (libc.rand() % 2001 - 1000) as f32 * 0.015;
        pos = add(pos, scale3(xyz(m[0]), x));
        pos = add(pos, scale3(xyz(m[1]), y));
        let (tex, size) = if libc.rand() % 6 != 0 {
            (5, randf(libc) * 0.15 + 0.15)
        } else {
            (6, randf(libc) * 1.5 + 1.5)
        };
        let phase = randf(libc) * PI_F;
        self.add(Add {
            pos,
            vel: [0.0; 3],
            drag: 0.0,
            size,
            alpha: 128.0,
            rot: 0.0,
            rise: -0.3,
            rise_grow: 0.0,
            phase,
            amp: 0.1,
            view,
            tex,
            phase_step: 0.25,
            life: -1.0,
        });
    }

    /// `EftBubble_Add`: a `life` above 0 fades it out over that many seconds.
    fn add(&mut self, a: Add) {
        if self.list.len() >= BUBBLES {
            return;
        }
        let inv = |x: f32| if x == 0.0 { 0.0 } else { 1.0 / x };
        self.list.push(Bubble {
            pos: a.pos,
            vel: a.vel,
            drag: inv(a.drag),
            rot: a.rot,
            size: a.size,
            alpha: a.alpha,
            alpha_step: if a.life <= 0.0 {
                0.0
            } else {
                -a.alpha / (a.life * 30.0)
            },
            rise: a.rise,
            rise_grow: inv(a.rise_grow),
            phase: a.phase,
            amp: a.amp,
            phase_step: a.phase_step,
            view: a.view,
            tex: a.tex,
            layer: 0,
        });
    }

    /// `EftBubble_Draw`: camera-facing quads of the bubbles of the view drawn (view 0, or every view).
    pub(crate) fn draw(&self, ot: &mut OrderTable, world2view: &Mtx, world2screen: &Mtx) {
        let mut cam = inverse_rt(world2view);
        cam[3] = [0.0, 0.0, 0.0, cam[3][3]];
        for b in self.list.iter().filter(|b| b.view == 0 || b.view == -1) {
            let mut m = cam;
            if b.rot != 0.0 {
                let rz = rotate_xyz(&IDENTITY, [0.0, 0.0, eft_wrap(b.rot * PI_F / 180.0)]);
                m = mtx_mul(&m, &rz);
            }
            m[3] = point(b.pos);
            let h = b.size * 0.5;
            let corners = [[-h, -h, 0.0], [h, -h, 0.0], [-h, h, 0.0], [h, h, 0.0]];
            let scr: Vec<Scr> = corners
                .iter()
                .map(|&c| project(world2screen, xyz(apply(&m, point(c)))))
                .collect();
            if !scr.iter().all(|v| on_screen(v, 2)) {
                continue;
            }
            let z = scr
                .iter()
                .fold(0i32, |s, v| s.wrapping_add(v.z))
                .wrapping_shr(10);
            let st = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
            let rgba = [0x80, 0x80, 0x80, b.alpha as u32 as u8];
            let vert = |k: usize| EffectVertex {
                position: [
                    scr[k].x as f32 / 16.0 - OFFSET_X,
                    scr[k].y as f32 / 16.0 - OFFSET_Y,
                    (scr[k].z & 0xFF_FFFF) as f32,
                ],
                stq: [st[k][0], st[k][1], 1.0],
                rgba,
            };
            let l = if b.layer >= 2 { b.layer - 2 } else { b.layer };
            for k in 0..2 {
                ot.add(
                    depth_slot(z),
                    usize::from(l),
                    EffectTri {
                        texture: self.textures.get(usize::from(b.tex)).copied(),
                        blend: EffectBlend::of(usize::from(l), b.layer >= 2),
                        clamp: false,
                        zwrite: false,
                        alpha_ref: 0,
                        vertices: [vert(k), vert(k + 1), vert(k + 2)],
                    },
                );
            }
        }
    }
}

/// `EftBubble_Step`: true once it faded out, reached the surface or its view's camera left the water.
fn step(b: &mut Bubble, level: f32, cam_y: Option<f32>, coef: &[f32; 4]) -> bool {
    b.pos = add(b.pos, b.vel).map(sat);
    b.vel[0] += b.vel[0] * b.drag;
    b.pos[1] = sat(b.pos[1] + b.rise);
    b.vel[1] += b.vel[1] * b.drag;
    b.rise = sat(b.rise + sat(b.rise * b.rise_grow));
    b.vel[2] += b.vel[2] * b.drag;
    b.pos[0] += cos_fast(coef, b.phase) * b.amp;
    b.pos[2] += sin_fast(coef, b.phase) * b.amp;
    b.phase += b.phase_step;
    if b.phase > PI_F {
        b.phase -= 6.283_185_3;
    }
    b.alpha += b.alpha_step;
    if b.alpha <= 0.0 {
        b.alpha = 0.0;
        return true;
    }
    if b.pos[1] - b.size * 0.5 < level {
        return true;
    }
    b.view != -1 && cam_y.is_some_and(|y| y < level)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;

    fn chr(pos: V3, in_water: bool) -> BubbleChr {
        BubbleChr {
            pos,
            dir: [0.0, 0.0, 1.0],
            height: 20.0,
            in_water,
            nodes: std::array::from_fn(|i| [pos[0], pos[1] - i as f32, pos[2]]),
        }
    }

    #[test]
    fn needs_six_textures() {
        assert!(Bubbles::new(0.0, [0.0; 4], vec![0; 5]).is_none());
        assert!(Bubbles::new(0.0, [0.0; 4], vec![0; 9]).is_some());
    }

    #[test]
    fn a_burst_runs_down_by_eight_a_frame_and_the_bubbles_rise_to_the_surface() {
        let mut b = Bubbles::new(0.0, [0.0; 4], (0..9).collect()).unwrap();
        let mut libc = LibcRandom::new(4);
        let chars = [chr([0.0, 100.0, 0.0], true), chr([0.0, 100.0, 0.0], false)];
        // Speed 4: 5 * 4 = 20, raised to 40.
        b.start_burst(0, 4.0);
        assert_eq!(b.chr[0].burst, 40.0);
        // Paused: nothing.
        b.update(&chars, true, &mut libc);
        assert_eq!(b.count(), 0);
        b.update(&chars, false, &mut libc);
        assert_eq!(b.chr[0].burst, 32.0);
        assert!(b.count() > 0);
        // A second start while it runs does nothing.
        b.start_burst(0, 20.0);
        assert_eq!(b.chr[0].burst, 32.0);
        for _ in 0..5 {
            b.update(&chars, false, &mut libc);
        }
        assert!(!b.chr[0].bursting);
        // The pool holds 100.
        assert!(b.count() <= BUBBLES);
        // They rise (y falls) and are gone above the surface.
        for _ in 0..2000 {
            b.update(&chars, false, &mut libc);
        }
        assert_eq!(b.count(), 0);
    }

    #[test]
    fn under_water_the_camera_and_the_body_make_bubbles_that_draw() {
        let mut b = Bubbles::new(0.0, [0.0; 4], (10..19).collect()).unwrap();
        let mut libc = LibcRandom::new(8);
        let chars = [
            chr([0.0, 60.0, 300.0], true),
            chr([0.0, 60.0, 300.0], false),
        ];
        // The camera above the water: nothing.
        let above = View::new([0.0, -20.0, 0.0], [0.0; 3]);
        b.cam = Some(above.world2view);
        for _ in 0..60 {
            b.update(&chars, false, &mut libc);
        }
        assert_eq!(b.count(), 0);
        // Under it (+y down): the camera's timer and the body's 1 in 120 chances.
        let under = View::new([0.0, 50.0, 0.0], [0.0; 3]);
        b.cam = Some(under.world2view);
        for _ in 0..60 {
            b.update(&chars, false, &mut libc);
        }
        assert!(b.count() > 3, "{}", b.count());
        let mut ot = OrderTable::new();
        b.draw(&mut ot, &under.world2view, &under.world2screen);
        let tris = ot.into_list();
        assert!(!tris.is_empty());
        assert!(tris.iter().all(|t| matches!(t.texture, Some(15 | 16))));
        // Back above the water, the view's bubbles end.
        b.cam = Some(above.world2view);
        b.update(&chars, false, &mut libc);
        assert_eq!(b.count(), 0);
    }
}
