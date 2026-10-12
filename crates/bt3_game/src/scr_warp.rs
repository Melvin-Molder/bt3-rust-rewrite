//! The screen shock waves (`ScrWarp_*`, src/battle/stg_ambient.c): rings that grow from a projected point. Their
//! state is kept (the ten objects, the VU0 draws of a spawn, the growth, the frees); the frame redrawn through the
//! ring's displaced coordinates is not drawn yet.

use crate::eft::{DrawCtx, V3};
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
        });
        true
    }

    /// `ScrWarp_UpdateAll` (not while paused): a new wave is placed by its view's camera, dropped when off the
    /// 512 x 448 frame; each grows by its speed, which slows, its ring widening, and fades over its last six
    /// frames; it ends with its life or past a radius of 600.
    pub(crate) fn update(&mut self, ctx: &DrawCtx) {
        for slot in &mut self.objs {
            let Some(o) = slot else {
                continue;
            };
            if !o.projected {
                let (x, y, z) = ctx.project(o.pos);
                let (x, y) = ((x as i32) as f32, (y as i32) as f32);
                if z <= 0 || !(1792.0 < x && x < 2304.0 && 1824.0 < y && y < 2272.0) {
                    *slot = None;
                    continue;
                }
                o.screen = [x - 1792.0, y - 1824.0];
                o.projected = true;
            }
            o.life -= 1.0;
            if o.life <= 0.0 || 600.0 < o.radius {
                *slot = None;
                continue;
            }
            o.radius += o.speed;
            o.speed += o.speed_acc;
            o.width += o.width_vel;
            if o.life <= 6.0 {
                o.alpha = (o.alpha + o.alpha_vel).max(0.0);
            }
        }
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
            w.update(&ctx);
            frames += 1;
        }
        // 24 frames of life; the 24th update frees it.
        assert_eq!(frames, 24);
        // Behind the camera: dropped at its first update.
        w.spawn(0, [0.0, 0.0, -100.0], 0.8, 10.0, 50.0, 8.0, 0.4, &mut vu0);
        w.update(&ctx);
        assert_eq!(w.count(), 0);
        for _ in 0..OBJECTS {
            assert!(w.spawn(0, [0.0; 3], 1.0, 1.0, 1.0, 1.0, 0.4, &mut vu0));
        }
        assert!(!w.spawn(0, [0.0; 3], 1.0, 1.0, 1.0, 1.0, 0.4, &mut vu0));
        w.reset();
        assert_eq!(w.count(), 0);
    }
}
