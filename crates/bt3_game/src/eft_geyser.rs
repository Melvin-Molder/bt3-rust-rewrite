//! The geysers (`EftGeyser_*`, src/battle/eft_stage_1.c and eft_stage_2.c; stage effect kind 6, on a stage with
//! stage flag 1 and records in its MEF0 list 0: stage 3): columns that erupt at fixed places on a timer (a first
//! delay of up to 6 s, then fade in and hold, sink and fade out, wait), drawn as a ribbon of three camera-facing
//! segments with texture 1 of stage pack entry 0x12 scrolling along it, each with a smoke emitter of the smoke
//! manager at its foot. Their steam emitters need the steam manager, which no stage has.

use bt3_data::stage::StageGeyser;

use crate::eft::{
    DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, V3, V4, add,
    cross, depth_slot, norm, point, scale3, sub,
};
use crate::eft_smoke::{Smoke, SmokeArg};
use crate::rng::{LibcRandom, Vu0Random};

/// `EftGeyserParam`: what an eruption starts from, and its current state.
#[derive(Clone, Copy, Debug)]
struct Param {
    pos: V3,
    base: V3,
    color: V4,
    cur_color: V4,
    width: f32,
    height: f32,
    sink: f32,
    scroll: f32,
    hold: i32,
    fade: i32,
    wait: i32,
}

/// `EftGeyser`: a column task.
#[derive(Clone, Debug)]
struct Column {
    init: Param,
    cur: Param,
    alpha_step: f32,
    tex_v: f32,
    delay: i32,
    state: u8,
    /// Its emitter among the smoke manager's.
    smoke: Option<usize>,
}

/// `gEftGeyserMgr` and its columns.
#[derive(Clone, Debug)]
pub(crate) struct Geysers {
    texture: Option<usize>,
    columns: Vec<Column>,
}

impl Geysers {
    /// `EftGeyser_MgrInit` before the columns' inits.
    pub(crate) fn new(records: &[StageGeyser], texture: Option<usize>) -> Self {
        let columns = records
            .iter()
            .map(|r| {
                let color = r.rgba.map(f32::from);
                let p = Param {
                    pos: r.pos,
                    base: [r.pos[0], r.pos[1] + r.height, r.pos[2]],
                    color,
                    cur_color: color,
                    width: r.width,
                    height: r.height,
                    sink: 2.0,
                    scroll: r.scroll,
                    hold: i32::from(r.hold),
                    fade: i32::from(r.fade),
                    wait: i32::from(r.wait),
                };
                Column {
                    init: p,
                    cur: p,
                    alpha_step: 0.0,
                    tex_v: 0.0,
                    delay: 0,
                    state: 0,
                    smoke: None,
                }
            })
            .collect();
        Self { texture, columns }
    }

    /// `EftGeyser_Init` of every column: a first delay of `rand() % 180` frames, the eruption's start, and its
    /// smoke emitter (`EftGeyser_StartSmoke`: the smoke's first record's colours, a random speed and rate) when
    /// the stage has the smoke.
    pub(crate) fn init(
        &mut self,
        mut smoke: Option<&mut Smoke>,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        for c in &mut self.columns {
            c.delay = libc.rand() % 180;
            c.restart(smoke.as_deref_mut());
            let Some(s) = smoke.as_deref_mut() else {
                continue;
            };
            let Some((ambient, diffuse)) = s.colors() else {
                continue;
            };
            let r = libc.rand() as f32 / 2_147_483_647.0 * 5.0 - 2.5;
            let rate = libc.rand() % 2 + 2;
            let arg = SmokeArg {
                pos: c.init.pos,
                ambient,
                diffuse,
                size: c.init.width / 10.0 * 2.0,
                alpha: 64.0,
                speed: (c.init.height / 150.0 + r).abs(),
                damp: 0.985,
                life_base: (c.init.height / 150.0 + 90.0) as i32,
                life_range: 30,
                rate,
            };
            if c.smoke.is_none() {
                c.smoke = Some(s.create(arg, libc, vu0));
            }
        }
    }

    /// The columns' phases.
    #[cfg(test)]
    pub(crate) fn states(&self) -> Vec<u8> {
        self.columns.iter().map(|c| c.state).collect()
    }

    /// `EftGeyser_Update` of every column (not while time is stopped): wait, start, fade in and hold, sink and
    /// fade out, wait; the texture scrolls throughout.
    pub(crate) fn update(&mut self, stopped: bool, mut smoke: Option<&mut Smoke>) {
        if stopped {
            return;
        }
        for c in &mut self.columns {
            match c.state {
                0 => {
                    c.delay -= 1;
                    if c.delay <= 0 {
                        c.state = 1;
                    }
                }
                1 | 2 => {
                    if c.state == 1 {
                        c.restart(smoke.as_deref_mut());
                    }
                    c.cur.base[1] = c.cur.pos[1];
                    c.state = 3;
                }
                3 => {
                    c.cur.cur_color[3] = (c.cur.cur_color[3] + c.alpha_step).min(c.init.color[3]);
                    c.cur.hold -= 1;
                    if c.cur.hold <= 0 {
                        c.cur.hold = 0;
                        c.state = 4;
                    }
                }
                4 => {
                    c.cur.cur_color[3] -= c.alpha_step;
                    c.cur.base[1] += c.cur.sink;
                    if c.cur.cur_color[3] <= 0.0 {
                        c.cur.cur_color[3] = 0.0;
                        c.state = 5;
                    }
                }
                _ => {
                    c.cur.wait -= 1;
                    if c.cur.wait <= 0 {
                        c.cur.wait = 0;
                        c.state = 1;
                    }
                }
            }
            c.tex_v -= c.cur.scroll;
            if c.tex_v > 1.0 {
                c.tex_v -= 1.0;
            }
            if c.tex_v < 0.0 {
                c.tex_v += 1.0;
            }
        }
    }

    /// `EftGeyser_Draw` of every column (`EftGeyser_DrawColumn`).
    /// `bright`: `EftStage_GetTintScale`.
    pub(crate) fn draw(&self, ot: &mut OrderTable, ctx: &DrawCtx, bright: f32) {
        for c in &self.columns {
            c.draw(ot, ctx, self.texture, bright);
        }
    }
}

impl Column {
    /// `EftGeyser_Restart`: the parameters back to their first values, transparent, the smoke at the foot.
    fn restart(&mut self, smoke: Option<&mut Smoke>) {
        self.cur = self.init;
        self.cur.cur_color = self.cur.color;
        self.cur.cur_color[3] = 0.0;
        self.alpha_step = self.init.color[3] / self.cur.fade as f32;
        if let (Some(s), Some(i)) = (smoke, self.smoke) {
            s.set_pos(i, self.init.pos);
        }
    }

    /// Three segments between four points a quarter of the height apart upwards from the base, each turned to
    /// the camera; the texture runs once along them from the scroll position, the top edge transparent. A
    /// segment with a corner off the GS area is skipped, and the texture position only advances past one drawn.
    fn draw(&self, ot: &mut OrderTable, ctx: &DrawCtx, texture: Option<usize>, bright: f32) {
        let w = self.cur.width;
        // The colour's RGB scaled by the stage tint (Vec3_Scale keeps the alpha).
        let c = self.cur.cur_color;
        let mut col = [[c[0] * bright, c[1] * bright, c[2] * bright, c[3]]; 4];
        let pt: [V3; 4] = std::array::from_fn(|i| {
            let b = self.cur.base;
            [b[0], b[1] - self.cur.height * 0.25 * i as f32, b[2]]
        });
        let (dv, mut v, mut v_prev) = (0.333_333_34f32, 0.333_333_34f32, 0.0f32);
        let mut prev: Option<[V3; 2]> = None;
        for n in 0..3 {
            let (p, next) = (pt[n], pt[n + 1]);
            let side = norm(cross(sub(next, p), sub(p, ctx.eye)));
            let [q0, q1] = prev.unwrap_or([add(p, scale3(side, w)), add(p, scale3(side, -w))]);
            let q2 = add(next, scale3(side, w));
            let q3 = add(next, scale3(side, -w));
            prev = Some([q2, q3]);
            let Some(scr) = project_all(ctx, [q0, q1, q2, q3]) else {
                continue;
            };
            let z = scr
                .iter()
                .fold(0i32, |s, v| s.wrapping_add(v.2))
                .wrapping_shr(10);
            let t = self.tex_v;
            let st = [
                [0.0, v_prev + t],
                [1.0, v_prev + t],
                [0.0, v + t],
                [1.0, v + t],
            ];
            if n == 2 {
                col[2][3] = 0.0;
                col[3][3] = 0.0;
            }
            if !(col[0][3] <= 0.0 && col[1][3] <= 0.0 && col[2][3] <= 0.0) {
                queue_quad(ot, &scr, st, &col, texture, z);
            }
            v_prev += dv;
            v += dv;
            if n >= 2 {
                v = 1.0;
            }
        }
    }
}

/// `Vu0Cur_ProjectPoints`: GS x, y in 1/16 pixel and z of the four corners; none when one is behind the camera or
/// off the GS area.
fn project_all(ctx: &DrawCtx, quad: [V3; 4]) -> Option<[(i32, i32, i32); 4]> {
    let mut out = [(0, 0, 0); 4];
    for (k, c) in quad.into_iter().enumerate() {
        let v = crate::battle::apply(&ctx.world2screen, point(c));
        let q = 1.0 / v[3];
        let (x, y) = (v[0] * q, v[1] * q);
        if !(0.0 < x && x < 4096.0 && 0.0 < y && y < 4096.0 && v[3] > 0.0) {
            return None;
        }
        out[k] = ((x * 16.0) as i32, (y * 16.0) as i32, (v[2] * q) as i32);
    }
    Some(out)
}

/// `EftGeyser_QueueQuad`: a gouraud textured strip with Q 1 (the texture is not corrected for perspective), the
/// texture repeating.
fn queue_quad(
    ot: &mut OrderTable,
    scr: &[(i32, i32, i32); 4],
    st: [[f32; 2]; 4],
    col: &[V4; 4],
    texture: Option<usize>,
    z: i32,
) {
    let vert = |k: usize| EffectVertex {
        position: [
            scr[k].0 as f32 / 16.0 - OFFSET_X,
            scr[k].1 as f32 / 16.0 - OFFSET_Y,
            (scr[k].2 & 0xFF_FFFF) as f32,
        ],
        stq: [st[k][0], st[k][1], 1.0],
        rgba: col[k].map(|c| c as i32 as u8),
    };
    for k in 0..2 {
        ot.add(
            depth_slot(z),
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
    use bt3_data::stage::StageSmoke;

    fn record() -> StageGeyser {
        StageGeyser {
            pos: [0.0, 0.0, 400.0],
            width: 20.0,
            height: 300.0,
            scroll: 0.02,
            hold: 30,
            fade: 10,
            wait: 20,
            rgba: [200, 200, 255, 100],
        }
    }

    #[test]
    fn a_column_erupts_holds_sinks_and_waits_with_its_smoke() {
        let mut g = Geysers::new(&[record()], Some(6));
        let smoke_rec = StageSmoke {
            pos: [0.0; 3],
            damp: 0.9,
            size: 1.0,
            speed: 1.0,
            life_base: 60,
            rate: 3,
            ambient: [10, 20, 30, 40],
            diffuse: [50, 60, 70, 80],
        };
        let mut smoke = Smoke::new(&[smoke_rec], Some(5));
        let (mut libc, mut vu0) = (LibcRandom::new(9), Vu0Random::default());
        smoke.init(&mut libc, &mut vu0);
        g.init(Some(&mut smoke), &mut libc, &mut vu0);
        let c = &g.columns[0];
        assert!((0..180).contains(&c.delay));
        // Its smoke emitter is the manager's second, at the column's foot.
        assert_eq!(c.smoke, Some(1));
        // The first delay, then the start (two updates), the fade in (10) within the hold (30).
        let delay = c.delay.max(1) as usize;
        for _ in 0..delay {
            g.update(false, Some(&mut smoke));
        }
        assert_eq!(g.states(), [1]);
        g.update(false, Some(&mut smoke));
        assert_eq!(g.states(), [3]);
        assert_eq!(g.columns[0].cur.base[1], 0.0);
        for _ in 0..30 {
            g.update(false, Some(&mut smoke));
        }
        assert_eq!(g.states(), [4]);
        assert_eq!(g.columns[0].cur.cur_color[3], 100.0);
        // Drawn while visible, seen from in front.
        let view = View::new([0.0, -100.0, 0.0], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = OrderTable::new();
        g.draw(&mut ot, &ctx, 1.0);
        let tris = ot.into_list();
        assert_eq!(tris.len(), 6);
        assert!(tris.iter().all(|t| t.texture == Some(6)));
        // Fading out over 10 frames while it sinks, then the wait, then again.
        for _ in 0..10 {
            g.update(false, Some(&mut smoke));
        }
        assert_eq!(g.states(), [5]);
        assert_eq!(g.columns[0].cur.base[1], 20.0);
        for _ in 0..20 {
            g.update(false, Some(&mut smoke));
        }
        assert_eq!(g.states(), [1]);
        // Time stopped holds it.
        g.update(true, Some(&mut smoke));
        assert_eq!(g.states(), [1]);
    }
}
