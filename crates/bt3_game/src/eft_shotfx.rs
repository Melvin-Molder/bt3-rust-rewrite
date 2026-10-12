//! The vanish lines (`EftShotFx_*`, src/battle/eft_shot_fx.c): the black vertical streaks of a teleport, on a
//! fighter's body and in a column around it, with the common effect file's texture set 0x237.
//!
//! Requests 0xC..0xF (and object animation event bit 42) start one of two request slots. Kind 0, the vanish: rings
//! of lines on the body segments on its first two updates and a column of growing lines on the second; kind 1, the
//! reappearance: a column of long lines shrinking while they fade in, then body lines on its third to fifth
//! updates; kinds 2 / 3 the short forms, four staggered sets of body lines at once and no column. A line hangs at
//! its anchor pulled towards the camera, offset in the camera's plane, and fades.

use std::io;

use bt3_data::elf::Elf;

use crate::btl_tech_volley::{cos_fast, sin_fast};
use crate::eft::{
    DrawCtx, OrderTable, V3, add, cross, norm, queue_projected_quad, scale3, sub, wrap_angle, xyz,
};
use crate::rng::LibcRandom;

/// `gEftShotFxSegs`: the body segments.
const SEGS: u32 = 0x2E_CEC0;
const SEG_COUNT: usize = 23;
/// `gEftShotFxRows`: the column's slices.
const ROWS: u32 = 0x2E_D0E8;
const ROW_COUNT: usize = 6;

/// `EftShotFx_Alloc`: the pool.
const LINES: usize = 200;

// EFT_SHOTFX_LINE_*.
const ACTIVE: u32 = 0x02;
const WAIT: u32 = 0x04;
const DONE: u32 = 0x08;
const FADE_IN: u32 = 0x10;
const FADE_OUT: u32 = 0x20;
const GROW: u32 = 0x40;
const SHRINK: u32 = 0x80;

// EFT_SHOTFX_REQ_*.
const REQ_A: u32 = 0x001;
const REQ_B: u32 = 0x002;
const REQ_HOLD: u32 = 0x004;
const REQ_VANISH_COLUMN: u32 = 0x008;
const REQ_APPEAR_COLUMN: u32 = 0x010;
const REQ_BODY_A: u32 = 0x020;
const REQ_BODY_B: u32 = 0x040;
const REQ_BURST: u32 = 0x080;
const REQ_BURST_DONE: u32 = 0x100;

const TWO_PI: f32 = 6.283_185_3;

/// `RAND_MAX`-scaled libc `rand()`.
fn randf(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// One ring row of the body (`EftShotFxSeg`) or the column (`EftShotFxRow`): from a node (or a fraction of the
/// column) to another (or a single ring), rings every `spacing`, `count` lines of length (rand + `len_base`) and
/// ring radius (rand * 0.5 + `radius_base`), both in heights * 0.05.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RingRow<T> {
    pub from: T,
    pub to: Option<T>,
    pub spacing: f32,
    pub len_base: f32,
    pub radius_base: f32,
    pub count: i32,
}

/// The executable's two tables.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tables {
    /// The body segments up to the first of node 4 (where the spawner stops).
    pub segs: Vec<RingRow<u32>>,
    pub rows: Vec<RingRow<f32>>,
}

impl Tables {
    pub(crate) fn read(elf: &Elf) -> io::Result<Self> {
        let mut segs = Vec::new();
        for i in 0..SEG_COUNT as u32 {
            let n = elf.i32s(SEGS + i * 0x18, 2)?;
            let f = elf.f32s(SEGS + i * 0x18 + 8, 3)?;
            let count = elf.i32s(SEGS + i * 0x18 + 0x14, 1)?[0];
            if n[0] == 4 {
                break;
            }
            segs.push(RingRow {
                from: n[0] as u32,
                to: (n[1] >= 0).then_some(n[1] as u32),
                spacing: f[0],
                len_base: f[1],
                radius_base: f[2],
                count,
            });
        }
        let mut rows = Vec::new();
        for i in 0..ROW_COUNT as u32 {
            let f = elf.f32s(ROWS + i * 0x18, 5)?;
            let count = elf.i32s(ROWS + i * 0x18 + 0x14, 1)?[0];
            rows.push(RingRow {
                from: f[0],
                to: (f[1] >= 0.0).then_some(f[1]),
                spacing: f[2],
                len_base: f[3],
                radius_base: f[4],
                count,
            });
        }
        Ok(Self { segs, rows })
    }
}

/// What the lines read of a fighter.
pub(crate) struct ShotChr<'a> {
    /// `BtlCharApi_GetHeight`.
    pub height: f32,
    /// `BtlCharApi_GetNodePos`.
    pub node_pos: &'a dyn Fn(u32) -> V3,
}

/// `EftShotFxReq`.
#[derive(Clone, Copy, Debug, Default)]
struct Req {
    chr: usize,
    frame_a: i32,
    frame_b: i32,
    flags: u32,
}

/// `EftShotFxLine`.
#[derive(Clone, Copy, Debug)]
struct Line {
    pos: V3,
    off: V3,
    toward: f32,
    len: f32,
    growth: f32,
    accel: f32,
    width: f32,
    alpha: f32,
    alpha_step: f32,
    alpha_end: f32,
    life: f32,
    flags: u32,
    delay: i32,
}

/// `gEftShotFx`: the two request slots and the lines in the order they were added.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShotFx {
    reqs: [Req; 2],
    lines: Vec<Line>,
}

/// The line arguments of `EftShotFx_Add` a spawner varies.
struct LineArg {
    toward: f32,
    len: f32,
    growth: f32,
    accel_div: f32,
    life: f32,
    flags: u32,
    delay: i32,
}

impl ShotFx {
    /// Live lines.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.lines.len()
    }

    /// `EftShotFx_Start`: a free slot (none: nothing) starts kind 0 (vanish), 1 (reappear) or their short forms
    /// 2 / 3.
    pub(crate) fn start(&mut self, chr: usize, kind: u8) {
        let Some(r) = self.reqs.iter_mut().find(|r| r.flags == 0) else {
            return;
        };
        match kind {
            0 | 2 => {
                r.chr = chr;
                r.frame_a = 0;
                r.flags |= REQ_A | REQ_BODY_A | if kind == 2 { REQ_BURST } else { 0 };
            }
            1 | 3 => {
                r.chr = chr;
                r.frame_b = 0;
                r.flags |= REQ_B | REQ_APPEAR_COLUMN | if kind == 3 { REQ_BURST } else { 0 };
            }
            4 => {
                r.chr = chr;
                r.flags = 0;
            }
            _ => {}
        }
    }

    /// `EftShotFx_Add`: nothing when the pool is full. `life` in seconds; the fade spans the whole life.
    fn add(&mut self, pos: V3, off: V3, a: &LineArg, alpha: f32) {
        if self.lines.len() >= LINES {
            return;
        }
        let life = a.life * 30.0;
        let (alpha, alpha_end, alpha_step) = if a.flags & FADE_IN != 0 {
            (0.0, alpha, alpha / life)
        } else if a.flags & FADE_OUT != 0 {
            (alpha, 0.0, alpha / life)
        } else {
            (0.0, 0.0, 0.0)
        };
        self.lines.push(Line {
            pos,
            off,
            toward: a.toward,
            len: a.len,
            growth: a.growth,
            accel: if a.accel_div != 0.0 {
                1.0 / a.accel_div
            } else {
                0.0
            },
            width: 0.3,
            alpha,
            alpha_step,
            alpha_end,
            life,
            flags: a.flags,
            delay: a.delay,
        });
    }

    /// Rings of lines along `start` + `step` * j: each ring at a random start angle (one draw), each line a random
    /// length and radius and alpha 0x60 less 0..15 (three draws).
    #[allow(clippy::too_many_arguments)]
    fn rings(
        &mut self,
        start: V3,
        step: V3,
        steps: i32,
        row: &RingRow<impl Copy>,
        height: f32,
        spread: f32,
        len_scale: f32,
        a: &mut LineArg,
        coef: &[f32; 4],
        libc: &mut LibcRandom,
    ) {
        let len0 = a.len;
        for j in 0..steps {
            let ang_step = TWO_PI / row.count as f32;
            let ang_base = randf(libc) * TWO_PI;
            let jf = j as f32;
            let pos = [
                start[0] + jf * step[0],
                start[1] + jf * step[1],
                start[2] + jf * step[2],
            ];
            for k in 0..row.count {
                let len = (randf(libc) + row.len_base) * (height * 0.05);
                let radius = (randf(libc) * 0.5 + row.radius_base) * (height * 0.05);
                let ang = wrap_angle(ang_base + ang_step * k as f32);
                let off = scale3(
                    [sin_fast(coef, ang), cos_fast(coef, ang), 0.0],
                    radius * spread,
                );
                a.len = len * len_scale;
                let alpha = (0x60 - libc.rand() % 16) as f32;
                self.add(pos, off, a, alpha);
            }
        }
        a.len = len0;
    }

    /// The rings' count and step along `end - start`: one ring per `spacing` when longer, else one.
    fn split(start: V3, end: Option<V3>, spacing: f32) -> (i32, V3) {
        if let Some(end) = end {
            let d = sub(end, start);
            let dist = crate::btl_chr::length(d);
            if dist > spacing {
                let steps = (dist / spacing + 1.0) as i32;
                return (steps, scale3(d, 1.0 / steps as f32));
            }
        }
        (1, [0.0; 3])
    }

    /// `EftShotFx_SpawnBodyLines`: rings of lines along every body segment, waiting `delay` frames, then fading out
    /// over `life` seconds.
    fn spawn_body(
        &mut self,
        c: &ShotChr,
        life: f32,
        delay: i32,
        tables: &Tables,
        coef: &[f32; 4],
        libc: &mut LibcRandom,
    ) {
        let mut a = LineArg {
            toward: c.height * 0.05 * 5.0,
            len: 0.0,
            growth: 0.0,
            accel_div: 0.0,
            life,
            flags: WAIT | FADE_OUT,
            delay,
        };
        for seg in &tables.segs {
            let start = (c.node_pos)(seg.from);
            let (steps, step) = Self::split(start, seg.to.map(|n| (c.node_pos)(n)), seg.spacing);
            self.rings(
                start, step, steps, seg, c.height, 1.5, 2.0, &mut a, coef, libc,
            );
        }
    }

    /// `EftShotFx_SpawnVanishColumn` / `EftShotFx_SpawnAppearColumn`: rings on a column half the fighter's height
    /// tall centred on node 3, the lines growing (by 10 % less each frame) while they fade out, or five times as
    /// long and shrinking (12.5 % less each frame) while they fade in.
    fn spawn_column(
        &mut self,
        c: &ShotChr,
        life: f32,
        appear: bool,
        tables: &Tables,
        coef: &[f32; 4],
        libc: &mut LibcRandom,
    ) {
        let up = [0.0, -1.0, 0.0];
        let height = c.height * 0.5;
        let base = (c.node_pos)(3);
        let top = add(scale3(up, height * 0.5), base);
        let bottom = add(scale3(up, -height * 0.5), base);
        let axis = norm(sub(bottom, top));
        let mut a = LineArg {
            toward: c.height * 0.05 * 5.0,
            len: 0.0,
            growth: c.height * 0.05 * 7.0,
            accel_div: if appear { -8.0 } else { -10.0 },
            life,
            flags: if appear {
                ACTIVE | FADE_IN | SHRINK
            } else {
                ACTIVE | FADE_OUT | GROW
            },
            delay: 0,
        };
        let len_scale = if appear { 10.0 } else { 2.0 };
        for row in &tables.rows {
            let start = add(top, scale3(axis, height * row.from));
            let end = row.to.map(|t| add(top, scale3(axis, height * t)));
            let (steps, step) = Self::split(start, end, row.spacing);
            self.rings(
                start, step, steps, row, c.height, 1.2, len_scale, &mut a, coef, libc,
            );
        }
    }

    /// `EftShotFx_Update`: unless paused, the request slots (in the order of `EftShotFx_UpdateReqs`), then every
    /// line's step; the finished ones leave the list.
    pub(crate) fn update(
        &mut self,
        paused: bool,
        chars: &[ShotChr],
        tables: &Tables,
        coef: &[f32; 4],
        libc: &mut LibcRandom,
    ) {
        if paused {
            return;
        }
        for i in 0..2 {
            let mut r = self.reqs[i];
            if r.flags == 0 || r.flags & REQ_HOLD != 0 {
                continue;
            }
            let Some(c) = chars.get(r.chr) else {
                continue;
            };
            if r.flags & (REQ_BODY_A | REQ_BODY_B) != 0 {
                if r.flags & REQ_BURST != 0 {
                    if r.flags & REQ_BURST_DONE == 0 {
                        for j in 0..4 {
                            self.spawn_body(c, 0.1, j, tables, coef, libc);
                        }
                        r.flags |= REQ_BURST_DONE;
                    }
                } else {
                    self.spawn_body(c, 0.1, 0, tables, coef, libc);
                }
            }
            if r.flags & REQ_A != 0 {
                if r.frame_a == 1 {
                    r.flags |= REQ_VANISH_COLUMN;
                }
                if r.frame_a == 4 {
                    r.flags &= !REQ_A;
                    if r.flags & REQ_B == 0 {
                        r.flags = 0;
                    }
                }
                if r.flags & REQ_VANISH_COLUMN != 0 {
                    if r.flags & REQ_BURST == 0 {
                        self.spawn_column(c, 0.12, false, tables, coef, libc);
                    }
                    r.flags &= !(REQ_VANISH_COLUMN | REQ_BODY_A);
                }
                r.frame_a += 1;
            }
            if r.flags & REQ_B != 0 {
                if r.frame_b == 1 {
                    r.flags |= REQ_BODY_B;
                }
                if r.frame_b == 4 {
                    r.flags &= !(REQ_B | REQ_BODY_B);
                    if r.flags & REQ_A == 0 {
                        r.flags = 0;
                    }
                }
                if r.flags & REQ_APPEAR_COLUMN != 0 {
                    if r.flags & REQ_BURST == 0 {
                        self.spawn_column(c, 0.1, true, tables, coef, libc);
                    }
                    r.flags &= !(REQ_APPEAR_COLUMN | REQ_BODY_B);
                }
                r.frame_b += 1;
            }
            self.reqs[i] = r;
        }
        self.lines.retain_mut(|p| !step(p));
    }

    /// `EftShotFx_Draw`: every active line black with its alpha, its corners straight to the GS.
    pub(crate) fn draw(&self, ot: &mut OrderTable, ctx: &DrawCtx, texture: Option<usize>) {
        let mut cam_rot = crate::battle::inverse_rt(&ctx.world2view);
        cam_rot[3] = [0.0, 0.0, 0.0, cam_rot[3][3]];
        for p in self.lines.iter().filter(|p| p.flags & ACTIVE != 0) {
            let quad = build_quad(p, ctx.eye, &cam_rot);
            queue_projected_quad(ot, ctx, &quad, [0, 0, 0, p.alpha as u32 as u8], texture, 0);
        }
    }
}

/// `EftShotFx_Step`: the start delay, the life, the length change, the alpha ramp; true once the life is over.
fn step(p: &mut Line) -> bool {
    if p.flags & WAIT != 0 {
        p.delay -= 1;
        if p.delay <= 0 {
            p.flags = (p.flags & !WAIT) | ACTIVE;
        }
    }
    if p.flags & ACTIVE != 0 {
        p.life -= 1.0;
        if p.life <= 0.0 {
            p.flags |= DONE;
        }
        if p.flags & GROW != 0 {
            p.len += p.growth;
            p.growth += p.growth * p.accel;
        } else if p.flags & SHRINK != 0 {
            p.len -= p.growth;
            p.growth += p.growth * p.accel;
        }
        if p.flags & FADE_IN != 0 {
            p.alpha += p.alpha_step;
            if p.alpha >= p.alpha_end {
                p.alpha = p.alpha_end;
                p.flags &= !FADE_IN;
            }
        } else if p.flags & FADE_OUT != 0 {
            p.alpha -= p.alpha_step;
            if p.alpha <= p.alpha_end {
                p.alpha = p.alpha_end;
                p.flags &= !FADE_OUT;
            }
        }
    }
    p.flags & DONE != 0
}

/// `EftShotFx_BuildQuad`: the centre is the anchor pulled `toward` the camera plus the offset turned out of the
/// camera's plane; the ends half a length above and below, widened across the line of sight.
fn build_quad(p: &Line, eye: V3, cam_rot: &crate::battle::Mtx) -> [V3; 4] {
    let up = [0.0, -1.0, 0.0];
    let mut mid = add(scale3(norm(sub(eye, p.pos)), p.toward), p.pos);
    mid = add(
        mid,
        xyz(crate::battle::apply(
            cam_rot,
            [p.off[0], p.off[1], p.off[2], 1.0],
        )),
    );
    let half = scale3(up, p.len * 0.5);
    let (top, bottom) = (add(mid, half), sub(mid, half));
    let across = |e: V3| scale3(norm(cross(up, norm(sub(eye, e)))), p.width * 0.5);
    let (st, sb) = (across(top), across(bottom));
    [add(top, st), sub(top, st), add(bottom, sb), sub(bottom, sb)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;

    const COEF: [f32; 4] = [2.6e-6, -1.98e-4, 8.33e-3, -0.166_666_7];

    fn tables() -> Tables {
        Tables {
            segs: vec![
                RingRow {
                    from: 3,
                    to: Some(5),
                    spacing: 2.0,
                    len_base: 0.5,
                    radius_base: 0.5,
                    count: 4,
                },
                RingRow {
                    from: 7,
                    to: None,
                    spacing: 2.0,
                    len_base: 0.5,
                    radius_base: 0.5,
                    count: 3,
                },
            ],
            rows: vec![
                RingRow {
                    from: 0.0,
                    to: Some(1.0),
                    spacing: 4.0,
                    len_base: 0.5,
                    radius_base: 1.0,
                    count: 5,
                },
                RingRow {
                    from: 0.5,
                    to: None,
                    spacing: 1.0,
                    len_base: 0.5,
                    radius_base: 1.0,
                    count: 2,
                },
            ],
        }
    }

    fn nodes(n: u32) -> V3 {
        // Node 5 is 5 below node 3: three rings of the first segment.
        match n {
            5 => [0.0, 5.0, 100.0],
            _ => [0.0, 0.0, 100.0],
        }
    }

    fn run(fx: &mut ShotFx, frames: usize, libc: &mut LibcRandom) -> Vec<usize> {
        let f = nodes;
        let chars = [
            ShotChr {
                height: 20.0,
                node_pos: &f,
            },
            ShotChr {
                height: 20.0,
                node_pos: &f,
            },
        ];
        (0..frames)
            .map(|_| {
                fx.update(false, &chars, &tables(), &COEF, libc);
                fx.count()
            })
            .collect()
    }

    #[test]
    fn a_vanish_sets_body_lines_twice_and_a_growing_column() {
        let mut fx = ShotFx::default();
        let mut libc = LibcRandom::new(1);
        fx.start(0, 0);
        // Body: 3 rings of 4 + 1 ring of 3 = 15 lines a set; column (10 tall): 3 rings of 5 + 2 = 17.
        let first = run(&mut fx, 1, &mut libc);
        assert_eq!(first, [15]);
        // The body lines start at once (no delay) and live 3 frames; the column lives 3.6 frames, growing.
        let l = fx.lines[0];
        assert_eq!(l.flags, ACTIVE | FADE_OUT);
        assert!(l.alpha > 0x50 as f32 * 2.0 / 3.0 && l.alpha <= 0x60 as f32 * 2.0 / 3.0 + 1e-4);
        assert!((l.toward - 5.0).abs() < 1e-6);
        let second = run(&mut fx, 1, &mut libc);
        assert_eq!(second, [15 + 15 + 17]);
        let col = *fx.lines.last().unwrap();
        assert_eq!(
            col.flags & (ACTIVE | GROW | FADE_OUT),
            ACTIVE | GROW | FADE_OUT
        );
        // The slot ends after five updates; every line is gone a few frames later.
        let rest = run(&mut fx, 6, &mut libc);
        assert_eq!(fx.reqs[0].flags, 0);
        assert_eq!(*rest.last().unwrap(), 0, "{rest:?}");
    }

    #[test]
    fn a_reappearance_shrinks_its_column_then_sets_body_lines() {
        let mut fx = ShotFx::default();
        let mut libc = LibcRandom::new(1);
        fx.start(1, 1);
        run(&mut fx, 1, &mut libc);
        assert_eq!(fx.count(), 17);
        let l = fx.lines[0];
        assert_eq!(l.flags, ACTIVE | FADE_IN | SHRINK);
        // Fading in from 0; the shrink slows by an eighth a frame.
        assert!(l.alpha > 0.0 && (l.alpha - l.alpha_end / 3.0).abs() < 1e-4);
        assert!((l.accel + 0.125).abs() < 1e-6);
        assert!((l.growth - 7.0 * 0.875).abs() < 1e-4);
        // Frame 1 switches the body lines on; frames 2..4 spawn a set each (3 frames of life).
        let counts = run(&mut fx, 4, &mut libc);
        assert_eq!(counts, [17, 15, 30, 30]);
        assert_eq!(fx.reqs[0].flags, 0);
    }

    #[test]
    fn the_short_forms_burst_four_staggered_sets_and_no_column() {
        let mut fx = ShotFx::default();
        let mut libc = LibcRandom::new(1);
        fx.start(0, 2);
        fx.start(1, 3);
        // A third request finds no slot.
        fx.start(0, 0);
        // The vanish's four sets at once; the reappearance's wait for its frame 2.
        run(&mut fx, 1, &mut libc);
        assert_eq!(fx.count(), 4 * 15);
        let delays: Vec<i32> = fx.lines.iter().map(|l| l.delay).collect();
        assert!(delays.contains(&-1) && delays.contains(&2));
        let counts = run(&mut fx, 5, &mut libc);
        assert!(counts[1] >= 4 * 15, "{counts:?}");
        assert_eq!(fx.reqs.map(|r| r.flags), [0, 0]);
    }

    #[test]
    fn the_pool_holds_two_hundred() {
        let mut fx = ShotFx::default();
        let mut libc = LibcRandom::new(1);
        let f = nodes;
        let c = ShotChr {
            height: 20.0,
            node_pos: &f,
        };
        for _ in 0..20 {
            fx.spawn_body(&c, 0.1, 0, &tables(), &COEF, &mut libc);
        }
        assert_eq!(fx.count(), LINES);
    }

    #[test]
    fn active_lines_draw_black_and_vertical() {
        let mut fx = ShotFx::default();
        let mut libc = LibcRandom::new(1);
        fx.start(0, 2);
        run(&mut fx, 1, &mut libc);
        let view = View::new([0.0; 3], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = OrderTable::new();
        fx.draw(&mut ot, &ctx, Some(3));
        let tris = ot.into_list();
        // Only the active lines: the sets of the later two delays still wait.
        let active = fx.lines.iter().filter(|l| l.flags & ACTIVE != 0).count();
        assert!(active > 0 && active < fx.count());
        assert!(tris.len() <= active * 2 && !tris.is_empty());
        assert!(
            tris.iter().all(
                |t| t.texture == Some(3) && t.vertices.iter().all(|v| v.rgba[..3] == [0, 0, 0])
            )
        );
        let q = build_quad(&fx.lines[0], ctx.eye, &IDENTITY_ROT);
        // Top above bottom (+y is down), the same height across.
        assert!(q[0][1] < q[2][1] && (q[0][1] - q[1][1]).abs() < 1e-4);
    }

    const IDENTITY_ROT: crate::battle::Mtx = crate::battle::IDENTITY;
}
