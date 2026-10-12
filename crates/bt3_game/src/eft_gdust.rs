//! The ground dust (`EftGndDust*`, src/battle/eft_ground_dust.c): tasks of up to a few sprite particles drawn
//! from two pools (64 and 63 particles) with the common effect file's texture set 0xB. Kind 0, the puff, is what
//! a breaking stage object raises (`EftGndDust_SpawnDebris`): four particles at random points around it flying
//! outwards, coloured from birth to death by the stage's dust record, fading out after their life. The
//! fighters' kinds (slide, dash, burst, landing, impact) are not ported.

use bt3_data::stage::StageDust;

use crate::eft::{DrawCtx, OrderTable, V3, V4, add, draw_sprite, norm, scale3, wrap_angle};
use crate::rng::Vu0Random;

/// `EFT_GDUST_PART_*`.
const PART_FADING: u32 = 0x4;
const PART_DONE: u32 = 0x8;

/// `EftGndDustArg`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DustArg {
    pub pos: V3,
    pub accel: V3,
    pub col_a: [u8; 4],
    pub col_b: [u8; 4],
    pub pool: usize,
    pub blend: u8,
    pub tex: usize,
    pub size: i16,
    pub life: i16,
    pub fade: i16,
    pub spin: i16,
    pub r_min: i16,
    pub r_max: i16,
    pub grow: f32,
    pub bright: f32,
    pub speed: f32,
    pub speed_rand: f32,
    pub drag: f32,
    pub gravity: f32,
    pub scale: f32,
}

impl DustArg {
    /// `EftGndDust_InitTemplate`: the debris template, with the stage's dust record when it is enabled.
    pub(crate) fn debris(stage: Option<&StageDust>) -> Self {
        let mut t = Self {
            pos: [0.0, -100.0, 0.0],
            accel: [0.0, 1.0, 0.0],
            col_a: [80, 55, 35, 80],
            col_b: [80, 65, 45, 30],
            pool: 0,
            blend: 0,
            tex: 0,
            size: 500,
            life: 30,
            fade: 30,
            spin: 0,
            r_min: 17,
            r_max: 40,
            grow: 0.01,
            bright: 1.0,
            speed: 0.5,
            speed_rand: 0.0,
            drag: 0.9,
            gravity: 0.0,
            scale: 1.0,
        };
        if let Some(s) = stage.filter(|s| s.enabled) {
            t.col_a = s.col_a;
            t.col_b = s.col_b;
            t.speed = s.speed;
            t.speed_rand = s.speed_rand;
            t.size = s.size as i16;
            t.r_min = i16::from(s.r_min);
            t.r_max = i16::from(s.r_max);
        }
        t
    }
}

/// `EftGndDustPart`.
#[derive(Clone, Copy, Debug)]
struct Part {
    pos: V3,
    base: V3,
    color: V4,
    vel: V3,
    size: [f32; 2],
    grow: [f32; 2],
    accel: V3,
    col_a: [u8; 4],
    col_b: [u8; 4],
    life: i16,
    life_max: i16,
    fade_len: i16,
    fade: i16,
    scale: i16,
    angle: f32,
    spin: f32,
    drag: f32,
    alpha: f32,
    grow_damp: f32,
    flags: u32,
}

/// A dust task (`EftGndDustEmit`): its argument and particles.
#[derive(Clone, Debug)]
struct Task {
    arg: DustArg,
    parts: Vec<Part>,
}

/// `gEftGndDust`: the debris template, the pools' free counts and the tasks.
#[derive(Clone, Debug)]
pub(crate) struct GndDust {
    tmpl: DustArg,
    free: [usize; 2],
    tasks: Vec<Task>,
}

impl Default for GndDust {
    fn default() -> Self {
        Self::new(None)
    }
}

impl GndDust {
    /// `EftGndDustMgr_Init`: both pools free and the debris template.
    pub(crate) fn new(stage: Option<&StageDust>) -> Self {
        Self {
            tmpl: DustArg::debris(stage),
            free: [64, 63],
            tasks: Vec::new(),
        }
    }

    /// `BtlScene_Reset`: every puff ends (`EftGndDustPuff_Reset`), its particles back in their pools.
    pub(crate) fn reset(&mut self) {
        for t in self.tasks.drain(..) {
            self.free[t.arg.pool] += t.parts.len();
        }
    }

    /// The particles alive.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.tasks.iter().map(|t| t.parts.len()).sum()
    }

    /// `EftGndDust_SpawnDebris`: a puff of the template at `pos` with the brightness `bright`.
    pub(crate) fn spawn_debris(&mut self, pos: V3, bright: f32, vu0: &mut Vu0Random) {
        let arg = DustArg {
            pos,
            scale: 1.0,
            bright,
            ..self.tmpl
        };
        self.spawn_puff(arg, vu0);
    }

    /// `EftGndDustPuff_Init`: four particles (while the pool lasts) in random directions at a random distance
    /// between the radii, flying outwards at the speed and its random part.
    fn spawn_puff(&mut self, arg: DustArg, vu0: &mut Vu0Random) {
        let mut parts = Vec::new();
        for _ in 0..4 {
            if self.free[arg.pool] == 0 {
                break;
            }
            self.free[arg.pool] -= 1;
            let k = arg.gravity * arg.scale + arg.speed_rand;
            let mut dir = [0.0; 3];
            for d in &mut dir {
                *d = vu0.float01() * 2.0 - 1.0;
            }
            let dir = norm(dir);
            let (r_min, r_max) = (
                f32::from(arg.r_min) * arg.scale,
                f32::from(arg.r_max) * arg.scale,
            );
            let mut pos = [0.0; 3];
            for i in 0..3 {
                pos[i] = arg.pos[i] + dir[i] * (r_min + vu0.float01() * (r_max - r_min));
            }
            let speed = arg.speed + arg.speed_rand * vu0.float01();
            let spin = f32::from(arg.spin) * 0.1;
            let spin = if vu0.float01() > 0.5 { -spin } else { spin };
            parts.push(Part {
                pos,
                base: [0.0; 3],
                color: [128.0; 4],
                vel: scale3(dir, speed),
                size: [1.0; 2],
                grow: [arg.grow; 2],
                accel: scale3(arg.accel, k),
                col_a: arg.col_a,
                col_b: arg.col_b,
                life: arg.life,
                life_max: arg.life,
                fade_len: arg.fade,
                fade: arg.fade,
                scale: arg.size,
                angle: 0.0,
                spin,
                drag: arg.drag,
                alpha: 1.0,
                grow_damp: 1.0,
                flags: 0,
            });
        }
        self.tasks.push(Task { arg, parts });
    }

    /// `EftGndDustPuff_Update` (not while time is stopped): the particles step; a task ends with its last one.
    pub(crate) fn update(&mut self, time_stopped: bool) {
        let mut k = 0;
        while k < self.tasks.len() {
            let task = &mut self.tasks[k];
            if !time_stopped {
                let before = task.parts.len();
                task.parts.retain_mut(step);
                self.free[task.arg.pool] += before - task.parts.len();
            }
            if self.tasks[k].parts.is_empty() {
                self.tasks.remove(k);
            } else {
                k += 1;
            }
        }
    }

    /// `EftGndDustPuff_Draw`: one camera-facing sprite per particle, its colour times the brightness, of the
    /// size times the particle's growth, with the task's texture.
    pub(crate) fn draw(&self, ot: &mut OrderTable, ctx: &DrawCtx, textures: &[usize]) {
        for t in &self.tasks {
            let tex = textures.get(t.arg.tex).copied();
            for p in &t.parts {
                let b = t.arg.bright;
                let color = [
                    p.color[0] * b,
                    p.color[1] * b,
                    p.color[2] * b,
                    p.color[3] * p.alpha,
                ];
                let s = f32::from(p.scale) * t.arg.scale;
                draw_sprite(
                    ot,
                    ctx,
                    add(p.pos, p.base),
                    color,
                    [s * p.size[0], s * p.size[1]],
                    [0.0, 0.0, 1.0, 1.0],
                    p.angle,
                    t.arg.blend,
                    false,
                    tex,
                );
            }
        }
    }
}

/// `EftGndDust_UpdateParts` for one particle; false once it is done.
fn step(p: &mut Part) -> bool {
    p.life -= 1;
    if p.life <= 0 {
        p.life = 0;
        p.flags |= PART_FADING;
    }
    // EftGndDust_LerpColor: from colA at birth to colB.
    let t = f32::from(p.life) / f32::from(p.life_max);
    for i in 0..4 {
        p.color[i] = f32::from(p.col_a[i]) * t + f32::from(p.col_b[i]) * (1.0 - t);
    }
    for i in 0..2 {
        p.size[i] += p.grow[i];
        p.grow[i] *= p.grow_damp;
    }
    p.angle = wrap_angle(p.angle + p.spin * std::f32::consts::PI / 180.0);
    p.vel = add(scale3(p.vel, p.drag), p.accel);
    p.pos = add(p.pos, p.vel);
    if p.flags & PART_FADING != 0 {
        if p.fade > 0 {
            p.fade -= 1;
            p.alpha = f32::from(p.fade) / f32::from(p.fade_len);
            if p.fade <= 0 {
                p.alpha = 0.0;
                p.flags |= PART_DONE;
            }
        } else {
            p.flags |= PART_DONE;
        }
    }
    p.flags & PART_DONE == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_puff_lives_its_life_and_fade_then_frees_its_particles() {
        let mut d = GndDust::new(None);
        let mut vu0 = Vu0Random::default();
        d.spawn_debris([0.0; 3], 0.7, &mut vu0);
        assert_eq!(d.count(), 4);
        assert_eq!(d.free[0], 60);
        // Each particle starts between the radii (17..40) of the centre.
        for p in &d.tasks[0].parts {
            let r = (p.pos[0].powi(2) + p.pos[1].powi(2) + p.pos[2].powi(2)).sqrt();
            assert!((16.9..40.1).contains(&r), "{r}");
        }
        // Life 30, then the fade: the 30th update starts it (29 left), the 59th ends it.
        for _ in 0..58 {
            d.update(false);
        }
        assert_eq!(d.count(), 4);
        d.update(false);
        assert_eq!(d.count(), 0);
        assert_eq!(d.free[0], 64);
        // Time stopped holds them.
        d.spawn_debris([0.0; 3], 1.0, &mut vu0);
        for _ in 0..100 {
            d.update(true);
        }
        assert_eq!(d.count(), 4);
        d.reset();
        assert_eq!((d.count(), d.free[0]), (0, 64));
    }

    #[test]
    fn a_puff_draws_a_sprite_per_particle_in_front_of_the_camera() {
        use crate::battle::View;
        let mut d = GndDust::new(None);
        let mut vu0 = Vu0Random::default();
        d.spawn_debris([0.0, 0.0, 500.0], 1.0, &mut vu0);
        let view = View::new([0.0; 3], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = OrderTable::new();
        d.draw(&mut ot, &ctx, &[7]);
        let tris = ot.into_list();
        assert_eq!(tris.len(), 8);
        assert!(tris.iter().all(|t| t.texture == Some(7)));
    }

    #[test]
    fn the_pool_limits_the_puffs() {
        let mut d = GndDust::new(None);
        let mut vu0 = Vu0Random::default();
        for _ in 0..17 {
            d.spawn_debris([0.0; 3], 1.0, &mut vu0);
        }
        assert_eq!(d.count(), 64);
        assert!(d.tasks.last().is_some_and(|t| t.parts.is_empty()));
    }
}
