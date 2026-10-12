//! The screen shock waves (`ScrWarp_*`, src/battle/stg_ambient.c): rings that grow from a projected point. The frame
//! is copied at half size into a work buffer and drawn back through each ring: a strip from its inner edge to a
//! jittered middle and one from there to its outer edge, the middle vertices displaced from where their texels
//! come from, so the picture warps; opaque at the middle, an eighth at the edges.

use crate::eft::{DrawCtx, V3};
use crate::post::{PostBuf, PostOp, PostVertex};
use crate::rng::Vu0Random;

/// `SCRWARP_MAX`.
const OBJECTS: usize = 10;
/// `SCRWARP_DIRS`.
const DIRS: usize = 20;
/// `SCRWARP_ALPHA`.
const ALPHA: f32 = 128.0 / 255.0;

/// `ScrWarpObj`.
#[derive(Clone, Copy, Debug)]
struct Wave {
    view: usize,
    /// Bit 1: projected.
    projected: bool,
    life: f32,
    pos: V3,
    screen: [f32; 2],
    radius: f32,
    alpha: f32,
    alpha_vel: f32,
    width: f32,
    width_vel: f32,
    speed: f32,
    speed_acc: f32,
    jitter: [f32; DIRS],
    /// Per direction: the inner, middle and outer vertex in frame pixels, and their texels in the half-size copy.
    xy: [[[f32; 2]; 3]; DIRS],
    uv: [[[f32; 2]; 3]; DIRS],
}

/// `gScrWarp->dir`: sine and cosine of -pi + i * pi / 10 (`Mathf_Sin` / `Mathf_Cos`).
fn dirs() -> [[f32; 2]; DIRS] {
    let mut a = -3.141_592_7f32;
    std::array::from_fn(|_| {
        let d = [
            crate::btl_tech_volley::mathf_sin(a),
            crate::btl_tech_volley::mathf_cos(a),
        ];
        a += 0.314_159_27;
        if 3.141_592_7 <= a {
            a -= 6.283_185_5;
        }
        d
    })
}

/// `gScrWarp`.
#[derive(Clone, Debug, Default)]
pub(crate) struct ScrWarps {
    objs: [Option<Wave>; OBJECTS],
}

impl ScrWarps {
    /// Waves in use.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.objs.iter().flatten().count()
    }

    /// `ScrWarp_Spawn`: a free object (none: nothing) for `view`, its jitter drawn from the VU0 generator (one
    /// draw per direction).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn(
        &mut self,
        view: usize,
        pos: V3,
        seconds: f32,
        radius: f32,
        width: f32,
        speed: f32,
        jitter: f32,
        vu0: &mut Vu0Random,
    ) -> bool {
        let Some(slot) = self.objs.iter_mut().find(|o| o.is_none()) else {
            return false;
        };
        let life = seconds * 30.0;
        let base = 0.5 - jitter * 0.5;
        *slot = Some(Wave {
            view,
            projected: false,
            life,
            pos,
            screen: [0.0; 2],
            radius,
            alpha: ALPHA,
            alpha_vel: -ALPHA / 6.0,
            width,
            width_vel: width * 3.2 / life,
            speed,
            speed_acc: -(speed * 0.3 / life),
            jitter: std::array::from_fn(|_| base + vu0.float01() * jitter),
            xy: [[[0.0; 2]; 3]; DIRS],
            uv: [[[0.0; 2]; 3]; DIRS],
        });
        true
    }

    /// `ScrWarp_UpdateAll` (not while paused): a new wave is placed by its view's camera, dropped when off the
    /// 512 x 448 frame; each grows by its speed, which slows, its ring widening, and fades over its last six
    /// frames; it ends with its life or past a radius of 600. `ScrWarp_Step` rebuilds its vertices (texels kept
    /// at least 2 across and 0 down).
    /// `ctxs`: the view drawn, or split screen's two (a wave is placed by its own view's camera, and view 1's is
    /// then moved 256 left, as the original does).
    pub(crate) fn update(&mut self, ctxs: &[DrawCtx]) {
        let dirs = dirs();
        let two = ctxs.len() == 2;
        for slot in &mut self.objs {
            let Some(o) = slot else {
                continue;
            };
            if !o.projected {
                let ctx = if two { &ctxs[o.view.min(1)] } else { &ctxs[0] };
                let (x, y, z) = ctx.project(o.pos);
                let (x, y) = ((x as i32) as f32, (y as i32) as f32);
                if z <= 0 || !(1792.0 < x && x < 2304.0 && 1824.0 < y && y < 2272.0) {
                    *slot = None;
                    continue;
                }
                o.screen = [x - 1792.0, y - 1824.0];
                if two && o.view == 1 {
                    o.screen[0] -= 256.0;
                }
                o.projected = true;
            }
            o.life -= 1.0;
            if o.life <= 0.0 || 600.0 < o.radius {
                *slot = None;
                continue;
            }
            o.radius += o.speed;
            let r0 = o.radius;
            let r2 = r0 + o.width;
            let r1 = r0 + o.width * 0.5;
            let [sx, sy] = o.screen;
            for (i, d) in dirs.iter().enumerate() {
                let at = |r: f32| [d[0] * r, d[1] * r];
                let rj = r0 + o.width * o.jitter[i];
                for (k, r) in [r0, r1, r2].into_iter().enumerate() {
                    let p = at(r);
                    o.uv[i][k] = [((sx + p[0]) * 0.5).max(2.0), ((sy + p[1]) * 0.5).max(0.0)];
                }
                for (k, r) in [r0, rj, r2].into_iter().enumerate() {
                    let p = at(r);
                    o.xy[i][k] = [sx + p[0], sy + p[1]];
                }
            }
            o.speed += o.speed_acc;
            o.width += o.width_vel;
            if o.life <= 6.0 {
                o.alpha = (o.alpha + o.alpha_vel).max(0.0);
            }
        }
    }

    /// `ScrWarp_Update`'s `ScrWarp_DrawAll` (one view): with any wave in use, the frame copied at half size (half
    /// a texel off) into work buffer 0, then each ring's two strips of 21 vertex pairs from it, the middle at the
    /// wave's alpha and the edges at an eighth of it, the coordinates as integers.
    /// `view`: split screen's view (`ScrWarp_Draw(1, view)`: its half copied into a quarter of the buffer and only
    /// its waves, under the half's scissor), else the whole screen.
    pub(crate) fn draw(&self, view: Option<usize>) -> Vec<PostOp> {
        let mine = |o: &&Wave| view.is_none_or(|v| o.view == v);
        if self.objs.iter().flatten().filter(mine).count() == 0 {
            return Vec::new();
        }
        let (x0, w, tw) = match view {
            Some(v) => (if v == 1 { 256.0 } else { 0.0 }, 256.0, 128.0),
            None => (0.0, 512.0, 256.0),
        };
        let copy = PostOp::sprite(
            PostBuf::Frame,
            PostBuf::Work(0),
            false,
            [0.0, 0.0, tw, 224.0],
            [x0 + 0.5, 0.5, x0 + w + 0.5, 448.5],
            [0x80; 4],
        );
        let mut rings = PostOp {
            source: PostBuf::Work(0),
            target: PostBuf::Frame,
            blend: true,
            tris: Vec::new(),
        };
        for o in self.objs.iter().flatten().filter(mine) {
            let mid = (o.alpha * 255.0) as i32;
            let alpha = [mid / 8, mid, mid / 8];
            for j in 0..2 {
                let mut strip = Vec::with_capacity((DIRS + 1) * 2);
                for k in 0..=DIRS {
                    let m = k % DIRS;
                    for e in [j, j + 1] {
                        strip.push(PostVertex {
                            xy: o.xy[m][e].map(|c| c as i32 as f32),
                            uv: o.uv[m][e].map(|c| c as i32 as f32),
                            rgba: [0x80, 0x80, 0x80, alpha[e] as u8],
                        });
                    }
                }
                rings.strip(&strip);
            }
        }
        vec![copy, rings]
    }

    /// `ScrWarp_Reset`.
    pub(crate) fn reset(&mut self) {
        self.objs = Default::default();
    }

    /// The views of the waves in use (`count[]`).
    #[cfg(test)]
    pub(crate) fn views(&self) -> Vec<usize> {
        self.objs.iter().flatten().map(|o| o.view).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;

    #[test]
    fn a_wave_grows_for_its_life_and_ten_fit() {
        let mut w = ScrWarps::default();
        let mut vu0 = Vu0Random::default();
        let mut check = Vu0Random::default();
        assert!(w.spawn(1, [0.0, 0.0, 100.0], 0.8, 10.0, 50.0, 8.0, 0.4, &mut vu0));
        // Twenty draws of the generator.
        for _ in 0..DIRS {
            check.float01();
        }
        assert_eq!(vu0.float01(), check.float01());
        let o = w.objs[0].unwrap();
        assert!(o.jitter.iter().all(|&j| (0.3..=0.7).contains(&j)));
        assert_eq!(w.views(), [1]);
        let view = View::new([0.0; 3], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut frames = 0;
        while w.count() > 0 {
            w.update(std::slice::from_ref(&ctx));
            frames += 1;
        }
        // 24 frames of life; the 24th update frees it.
        assert_eq!(frames, 24);
        // Behind the camera: dropped at its first update.
        w.spawn(0, [0.0, 0.0, -100.0], 0.8, 10.0, 50.0, 8.0, 0.4, &mut vu0);
        w.update(std::slice::from_ref(&ctx));
        assert_eq!(w.count(), 0);
        for _ in 0..OBJECTS {
            assert!(w.spawn(0, [0.0; 3], 1.0, 1.0, 1.0, 1.0, 0.4, &mut vu0));
        }
        assert!(!w.spawn(0, [0.0; 3], 1.0, 1.0, 1.0, 1.0, 0.4, &mut vu0));
        w.reset();
        assert_eq!(w.count(), 0);
        assert!(w.draw(None).is_empty());
    }

    #[test]
    fn a_ring_is_drawn_from_the_half_size_copy_with_its_middle_displaced() {
        let mut w = ScrWarps::default();
        let mut vu0 = Vu0Random::default();
        let view = View::new([0.0; 3], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        w.spawn(0, [0.0, 0.0, 100.0], 0.8, 10.0, 50.0, 8.0, 0.4, &mut vu0);
        w.update(std::slice::from_ref(&ctx));
        let o = w.objs[0].unwrap();
        // Straight ahead: the frame's centre. Inner radius 10 + 8, outer 18 + 50, the texels at half.
        assert_eq!(o.screen, [256.0, 224.0]);
        let d = dirs();
        assert!((d[0][0] - 0.0).abs() < 1e-6 && d[0][1] == -1.0);
        assert_eq!(o.xy[0][0], [256.0 + d[0][0] * 18.0, 224.0 - 18.0]);
        assert_eq!(
            o.uv[0][2],
            [(256.0 + d[0][0] * 68.0) * 0.5, (224.0 - 68.0) * 0.5]
        );
        // The middle vertex where the jitter puts it; its texel half way.
        let rj = 18.0 + 50.0 * o.jitter[0];
        assert_eq!(o.xy[0][1][1], 224.0 - rj);
        assert_eq!(o.uv[0][1][1], (224.0 - 43.0) * 0.5);
        let ops = w.draw(None);
        assert_eq!(ops.len(), 2);
        assert_eq!(
            (ops[0].source, ops[0].target),
            (PostBuf::Frame, PostBuf::Work(0))
        );
        let r = &ops[1];
        assert_eq!(
            (r.source, r.target, r.blend),
            (PostBuf::Work(0), PostBuf::Frame, true)
        );
        // Two strips of 42 vertices: 80 triangles; the middle at alpha 128 / 255 * 255, the edges an eighth.
        assert_eq!(r.tris.len(), 80);
        assert_eq!(r.tris[0][0].rgba[3], 16);
        assert_eq!(r.tris[0][1].rgba[3], 128);
        assert_eq!(r.tris[0][0].xy, o.xy[0][0].map(|c| c as i32 as f32));
    }
}
