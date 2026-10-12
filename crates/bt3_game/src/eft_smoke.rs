//! The stage's smoke emitters (`EftSmoke*`, src/battle/eft_shot.c; stage effect kind 4, on a stage with stage flag
//! 2 and records in its MEF0 list 1): each emitter starts a particle with a chance of 1 in its rate a frame,
//! rising in a cone and slowing sideways, of 40 at most, drawn as flat sprites lit by the stage's light (the
//! ambient colour plus the diffuse one by how much the particle faces the light from 50 above the emitter) and
//! fading over its last 50 frames, with texture 2 of stage pack entry 0x12. An emitter runs 60 to 89 updates at
//! its init, so that the smoke is already there.

use bt3_data::stage::StageSmoke;

use crate::eft::{
    DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, SCREEN_DIST, V3,
    add, depth_slot, dot, norm, point, scale3, sub,
};
use crate::rng::{LibcRandom, Vu0Random};

const PARTS: usize = 40;

/// `EftSmokeArg`.
#[derive(Clone, Copy, Debug)]
struct Arg {
    pos: V3,
    ambient: [f32; 3],
    diffuse: [f32; 3],
    size: f32,
    alpha: f32,
    speed: f32,
    damp: f32,
    life_base: i32,
    life_range: i32,
    rate: i32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Part {
    pos: V3,
    vel: V3,
    life: u16,
}

/// `EftSmoke`: an emitter task.
#[derive(Clone, Debug)]
struct Emitter {
    arg: Arg,
    part: [Part; PARTS],
}

/// `gEftSmokeMgr` and its emitters.
#[derive(Clone, Debug)]
pub(crate) struct Smoke {
    texture: Option<usize>,
    emitters: Vec<Emitter>,
}

impl Smoke {
    /// `EftSmokeMgr_Init` before its emitters' inits: one emitter per record (life 30 frames apart, alpha the
    /// ambient colour's fourth byte).
    pub(crate) fn new(records: &[StageSmoke], texture: Option<usize>) -> Self {
        let emitters = records
            .iter()
            .map(|r| Emitter {
                arg: Arg {
                    pos: r.pos,
                    ambient: [0, 1, 2].map(|i| f32::from(r.ambient[i])),
                    diffuse: [0, 1, 2].map(|i| f32::from(r.diffuse[i])),
                    size: r.size,
                    alpha: f32::from(r.ambient[3]),
                    speed: r.speed,
                    damp: r.damp,
                    life_base: i32::from(r.life_base),
                    life_range: 30,
                    rate: i32::from(r.rate),
                },
                part: [Part::default(); PARTS],
            })
            .collect();
        Self { texture, emitters }
    }

    /// The particles alive.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.emitters
            .iter()
            .flat_map(|e| &e.part)
            .filter(|p| p.life != 0)
            .count()
    }

    /// `EftSmoke_Init` of every emitter: 60 to 89 updates.
    pub(crate) fn init(&mut self, libc: &mut LibcRandom, vu0: &mut Vu0Random) {
        for e in &mut self.emitters {
            let n = libc.rand() % 30 + 60;
            for _ in 0..n {
                e.update(libc, vu0);
            }
        }
    }

    /// `EftSmoke_Update` of every emitter (not while time is stopped).
    pub(crate) fn update(&mut self, stopped: bool, libc: &mut LibcRandom, vu0: &mut Vu0Random) {
        if stopped {
            return;
        }
        for e in &mut self.emitters {
            e.update(libc, vu0);
        }
    }

    /// `EftSmoke_Draw` of every emitter: `light` is the stage's light direction (`BtlStage_GetLightVecB`).
    pub(crate) fn draw(&self, ot: &mut OrderTable, ctx: &DrawCtx, light: V3) {
        let light = norm(light);
        // EftStage_GetTintScale: 1 without a stage tint, which is not ported.
        let bright = 1.0;
        for e in &self.emitters {
            let a = &e.arg;
            let base = [a.pos[0], a.pos[1] - 50.0, a.pos[2]];
            for p in e.part.iter().filter(|p| p.life != 0) {
                let mut alpha = a.alpha;
                if p.life < 50 {
                    alpha = alpha * f32::from(p.life) / 50.0;
                }
                let mut d = sub(p.pos, base);
                d[1] *= 0.8;
                let lit = scale3(a.diffuse, dot(norm(d), light)).map(|c| c.max(0.0));
                let col = add(lit, a.ambient).map(|c| c.min(255.0));
                let size = (a.size * 128.0) as u32;
                draw_flat(
                    ot,
                    ctx,
                    p.pos,
                    [
                        (col[0] * bright) as i32,
                        (col[1] * bright) as i32,
                        (col[2] * bright) as i32,
                        alpha as i32,
                    ],
                    size,
                    self.texture,
                );
            }
        }
    }
}

impl Emitter {
    /// With a chance of 1 in the rate, the first free particle starts at the emitter going up in a cone; then
    /// every live particle moves, slows sideways and ages.
    fn update(&mut self, libc: &mut LibcRandom, vu0: &mut Vu0Random) {
        let a = self.arg;
        if libc.rand() % a.rate.max(1) == 0
            && let Some(p) = self.part.iter_mut().find(|p| p.life == 0)
        {
            p.pos = a.pos;
            let x = vu0.float_range(-50.0, 50.0);
            let z = vu0.float_range(-50.0, 50.0);
            p.vel = scale3(norm([x, -100.0, z]), a.speed);
            p.life = (a.life_base + libc.rand() % a.life_range) as u16;
        }
        for p in self.part.iter_mut().filter(|p| p.life != 0) {
            p.pos = add(p.pos, p.vel);
            p.vel[0] *= a.damp;
            p.vel[2] *= a.damp;
            p.life -= 1;
        }
    }
}

/// `EftSpr_DrawFlat`: an upright sprite of half size `size` (screen units at the reference depth, scaled by the
/// screen distance over the depth) and one colour, the texture's corners swapped as the original writes them;
/// dropped below two units or with a corner off the GS area.
fn draw_flat(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    pos: V3,
    rgba: [i32; 4],
    size: u32,
    texture: Option<usize>,
) {
    if size == 0 {
        return;
    }
    let v = crate::battle::apply(&ctx.world2screen, point(pos));
    let q = 1.0 / v[3];
    let (x, y, z, w) = (
        (v[0] * q * 16.0) as i32,
        (v[1] * q * 16.0) as i32,
        (v[2] * q) as i32,
        v[3] as i32,
    );
    let k = (SCREEN_DIST * 4096.0 / w as f32) as u32;
    let half = (size.wrapping_mul(k) >> 12) as i32;
    if half < 2 {
        return;
    }
    let off = |x: i32, y: i32| z <= 0 || x > 0xFFEF || x <= 0 || y > 0xFFEF || y <= 0;
    let corners = [
        (x - half, y - half),
        (x + half, y - half),
        (x - half, y + half),
        (x + half, y + half),
    ];
    if corners.iter().any(|&(x, y)| off(x, y)) {
        return;
    }
    // The ST of corners 1 and 2 are each other's: (0, 0), (0, 1), (1, 0), (1, 1).
    let st = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
    let rgba = rgba.map(|c| c as u8);
    let vert = |k: usize| EffectVertex {
        position: [
            corners[k].0 as f32 / 16.0 - OFFSET_X,
            corners[k].1 as f32 / 16.0 - OFFSET_Y,
            (z & 0xFF_FFFF) as f32,
        ],
        stq: [st[k][0], st[k][1], 1.0],
        rgba,
    };
    for k in 0..2 {
        ot.add(
            depth_slot(z >> 8),
            0,
            EffectTri {
                texture,
                blend: EffectBlend::Alpha,
                clamp: false,
                zwrite: false,
                alpha_ref: 0,
                vertices: [vert(k), vert(k + 1), vert(k + 2)],
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;

    fn record() -> StageSmoke {
        StageSmoke {
            pos: [0.0, 0.0, 400.0],
            damp: 0.95,
            size: 1.0,
            speed: 1.5,
            life_base: 60,
            rate: 2,
            ambient: [40, 40, 40, 100],
            diffuse: [80, 80, 80, 0],
        }
    }

    #[test]
    fn an_emitter_is_already_smoking_after_its_init_and_its_particles_rise() {
        let mut s = Smoke::new(&[record(), record()], Some(4));
        let (mut libc, mut vu0) = (LibcRandom::new(2), Vu0Random::default());
        s.init(&mut libc, &mut vu0);
        // 60..89 updates of a 1 in 2 chance, lives of 60..89: a good part of the 40 each.
        let n = s.count();
        assert!((20..=80).contains(&n), "{n}");
        // They rise (y falls) from the emitter.
        assert!(
            s.emitters[0]
                .part
                .iter()
                .filter(|p| p.life != 0)
                .all(|p| p.pos[1] < 0.0)
        );
        // Time stopped: nothing moves.
        let before = s.emitters[0].part[0].pos;
        s.update(true, &mut libc, &mut vu0);
        assert_eq!(s.emitters[0].part[0].pos, before);
        // Drawn in front of a camera looking at them.
        let view = View::new([0.0, -50.0, 0.0], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = OrderTable::new();
        s.draw(&mut ot, &ctx, [0.0, 1.0, 0.0]);
        let tris = ot.into_list();
        assert!(!tris.is_empty());
        assert!(tris.iter().all(|t| t.texture == Some(4)));
        // The colour: at least the ambient one.
        assert!(tris.iter().all(|t| t.vertices[0].rgba[0] >= 40));
    }
}
