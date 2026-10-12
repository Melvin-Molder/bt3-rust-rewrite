//! The weather particles (`EftWeather*`, src/battle/eft_stage_2.c; stage effect kind 2, on a stage whose pack has
//! entry 0xB and whose weather record is on: stages 1, 5, 8, 14, 19, 21, 25, 37): up to 30 snowflakes, leaves or
//! ash drifting with the stage's wind (plus a slow sine) and a random walk, kept in an 80-wide box 55 in front of
//! the camera, fading with their distance to the camera, after wrapping round the box, while the camera moves,
//! with height and, by the record's flags, near y = 0; drawn as sprites of the pack's textures. Their movement
//! and its random numbers happen while drawing.

use bt3_data::stage::StageWeather;

use crate::battle::{apply, inverse_rt};
use crate::eft::{DrawCtx, OrderTable, V3, V4, add, draw_sprite, point, sub, xyz};
use crate::rng::LibcRandom;

/// `EFT_WEATHER_PTCL_MAX`.
const PARTS: usize = 30;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Part {
    pos: V3,
    vel: V3,
    color: V4,
    /// 3 after wrapping round the box, counting down: it fades in again.
    wrap: i32,
    tex: usize,
}

/// `gEftWeather` and its particle task.
#[derive(Clone, Debug)]
pub(crate) struct Weather {
    textures: Vec<usize>,
    wind: V3,
    wind_now: V3,
    /// Degrees, 0..359.
    angle: i32,
    count: usize,
    flags: u16,
    size: f32,
    /// `view.camPos[0]` / `view.moveCnt[0]`.
    cam_pos: V3,
    move_cnt: i32,
    parts: [Part; PARTS],
}

impl Weather {
    /// `EftWeather_Init` with a weather record that is on, before its particles' scatter.
    pub(crate) fn new(rec: &StageWeather, textures: Vec<usize>) -> Option<Self> {
        (rec.on && !textures.is_empty()).then(|| Self {
            textures,
            wind: rec.wind,
            wind_now: [0.0; 3],
            angle: 0,
            count: usize::from(rec.count).min(PARTS),
            flags: rec.ptcl_flags,
            size: rec.size,
            cam_pos: [0.0; 3],
            move_cnt: 0,
            parts: [Part::default(); PARTS],
        })
    }

    /// `EftWeatherPtcl_Reset` (also its init): every particle within 51 of the origin, at rest, transparent, with
    /// a random texture.
    pub(crate) fn reset(&mut self, libc: &mut LibcRandom) {
        let n = self.textures.len() as i32;
        for p in &mut self.parts {
            for c in &mut p.pos {
                *c = ((libc.rand() & 0x3FF) - 0x1FF) as f32 * 0.1;
            }
            p.vel = [0.0; 3];
            p.color = [128.0, 128.0, 128.0, 0.0];
            p.wrap = 0;
            p.tex = (libc.rand() % n) as usize;
        }
    }

    /// `EftWeather_Update` (paused or not): this frame's wind, the base plus a sine on x and z, its angle going
    /// round by 0 to 3 degrees.
    pub(crate) fn update(&mut self, libc: &mut LibcRandom) {
        let a = self.angle as f32 * 3.141_592_7 / 180.0;
        self.wind_now = self.wind;
        self.wind_now[0] += a.sin() * 0.3;
        self.wind_now[2] += a.sin() * 0.3;
        self.angle = (self.angle + (libc.rand() & 3)) % 360;
    }

    /// `EftWeatherPtcl_Draw`: the particles move (not while paused), then draw. `tint`: the stage tint's slot 0
    /// colour and the stage's darkest alpha, when it is on. `second`: in split screen each view moves and draws
    /// half the particles, the second view those of the table's second half.
    pub(crate) fn draw(
        &mut self,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
        tint: Option<([i32; 4], f32)>,
        paused: bool,
        libc: &mut LibcRandom,
        second: Option<bool>,
    ) {
        let range = match second {
            Some(second) => {
                let start = if second { PARTS / 2 } else { 0 };
                start..start + self.count / 2
            }
            None => 0..self.count,
        };
        if !paused {
            self.step(ctx, tint, libc, range.clone());
        }
        let s = self.size * 16.0;
        for p in &self.parts[range] {
            let tex = self.textures.get(p.tex).copied();
            draw_sprite(
                ot,
                ctx,
                p.pos,
                p.color,
                [s, s],
                [0.0, 0.0, 1.0, 1.0],
                0.0,
                0,
                false,
                tex,
            );
        }
    }

    /// `EftWeather_Move`: each particle's velocity takes a random step (kept within 0.4); it drifts with the wind
    /// and its velocity; out of the box it wraps round it; then its alpha.
    fn step(
        &mut self,
        ctx: &DrawCtx,
        tint: Option<([i32; 4], f32)>,
        libc: &mut LibcRandom,
        range: std::ops::Range<usize>,
    ) {
        let mut m = ctx.world2view;
        let inv = inverse_rt(&m);
        let cam = xyz(inv[3]);
        // EftWeather_TrackCam: 2 when the camera jumped, 1 while it moves or settles.
        let d = sub(cam, self.cam_pos);
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let mut moving = 0;
        if dist > 15.0 {
            moving = 2;
            self.move_cnt = 0;
        } else if dist > 4.0 {
            moving = 1;
            if self.move_cnt < 15 {
                self.move_cnt += 1;
            }
        } else if self.move_cnt != 0 {
            self.move_cnt -= 1;
            moving = 1;
        }
        self.cam_pos = cam;
        m[3][2] -= 55.0;
        let flags = self.flags;
        let size = self.size;
        for p in &mut self.parts[range] {
            for c in &mut p.vel {
                *c += ((libc.rand() & 0x1FF) - 0xFF) as f32 * (1.0 / 255.0) * 0.02;
            }
            p.vel = p.vel.map(|c| c.clamp(-0.4, 0.4));
            let world = add(add(p.pos, self.wind_now), p.vel);
            let mut v = xyz(apply(&m, point(world)));
            const W: f32 = 40.0;
            let code = |v: &V3| {
                let mut c = 0;
                if v[0] > W {
                    c |= 1;
                }
                if v[0] < -W {
                    c |= 2;
                }
                if v[1] > W {
                    c |= 4;
                }
                if v[1] < -W {
                    c |= 8;
                }
                if v[2] > W {
                    c |= 0x10;
                }
                if v[2] < -W {
                    c |= 0x20;
                }
                c
            };
            let mut c = code(&v);
            let mut n = 0;
            while c != 0 {
                p.wrap = 3;
                for (k, (out, back)) in [(1, 2), (4, 8), (0x10, 0x20)].into_iter().enumerate() {
                    if c & out != 0 {
                        v[k] -= 80.0;
                    }
                    if c & back != 0 {
                        v[k] += 80.0;
                    }
                }
                c = code(&v);
                if n > 0x40 {
                    break;
                }
                n += 1;
            }
            if moving == 2 {
                p.wrap = 0;
            }
            v[2] += 55.0;
            let pos = xyz(apply(&inv, point(v)));
            let mut y = pos[1];
            if flags & 2 != 0 && y >= 0.0 {
                y = 0.3 * -500.0;
            }
            let y = 1.0 - y.max(-500.0) / -500.0;
            let d = sub(cam, pos);
            let mut a = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * (1.0 / 110.0);
            a = if a > 1.0 {
                0.0
            } else if a < 0.5 {
                if a <= 0.1 { 0.0 } else { (a - 0.1) * 2.5 }
            } else {
                1.0 - (a - 0.5) * 2.0
            };
            if p.wrap != 0 {
                a *= (3 - p.wrap) as f32 / 3.0;
                p.wrap -= 1;
            }
            if moving == 1 && self.move_cnt != 0 {
                a *= (30 - self.move_cnt) as f32 / 30.0;
            }
            a *= y;
            if flags & 1 != 0 && pos[1] + (size + 4.0) > 0.0 {
                if pos[1] + size < 0.0 {
                    a *= -((pos[1] + size) * 0.25);
                } else {
                    a = 0.0;
                }
            }
            // With the stage tint on: the weather's colour (0x80 each) darkened by slot 0's alpha, the alpha by
            // its share of the stage's darkest.
            p.color = match tint {
                Some((t, max)) => {
                    let mut c = [0x80i32; 3];
                    let mut k = 1.0f32;
                    if t[3] != 0 {
                        c = c.map(|v| (v - t[3]).max(0));
                        k -= t[3] as f32 / max;
                    }
                    [c[0] as f32, c[1] as f32, c[2] as f32, a * 128.0 * k]
                }
                None => [128.0, 128.0, 128.0, a * 128.0],
            };
            p.pos = pos;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;

    fn rec() -> StageWeather {
        StageWeather {
            wind: [0.1, 0.3, 0.0],
            size: 1.0,
            on: true,
            ptcl_flags: 0,
            count: 20,
        }
    }

    #[test]
    fn the_particles_stay_in_front_of_the_camera_and_draw() {
        assert!(Weather::new(&StageWeather { on: false, ..rec() }, vec![1]).is_none());
        let mut w = Weather::new(&rec(), vec![7, 8, 9]).unwrap();
        let mut libc = LibcRandom::new(13);
        w.reset(&mut libc);
        assert!(
            w.parts
                .iter()
                .all(|p| p.pos.iter().all(|c| c.abs() <= 51.2))
        );
        assert!(w.parts.iter().all(|p| p.tex < 3));
        let view = View::new([0.0, -300.0, 100.0], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut drawn = 0;
        for _ in 0..30 {
            w.update(&mut libc);
            let mut ot = OrderTable::new();
            w.draw(&mut ot, &ctx, None, false, &mut libc, None);
            drawn += ot.into_list().len();
        }
        // Inside the box 55 in front of the camera after the first frames' wraps.
        let eye = [0.0, -300.0, 100.0];
        for p in &w.parts[..20] {
            let d = sub(p.pos, eye);
            assert!(
                d[0].abs() <= 40.5 && d[1].abs() <= 40.5 && (d[2] - 55.0).abs() <= 40.5,
                "{d:?}"
            );
        }
        assert!(drawn > 0);
        // Paused: they do not move.
        let before = w.parts[0].pos;
        w.draw(&mut OrderTable::new(), &ctx, None, true, &mut libc, None);
        assert_eq!(w.parts[0].pos, before);
        // Under the stage tint (slot 0 at alpha 50 of a stage darkest 100): darker by 50, half as opaque.
        let mut again = w.clone();
        let mut libc2 = LibcRandom::new(99);
        w.draw(
            &mut OrderTable::new(),
            &ctx,
            None,
            false,
            &mut LibcRandom::new(99),
            None,
        );
        again.draw(
            &mut OrderTable::new(),
            &ctx,
            Some(([0, 0, 0, 50], 100.0)),
            false,
            &mut libc2,
            None,
        );
        for (p, q) in w.parts[..w.count].iter().zip(&again.parts[..again.count]) {
            assert_eq!(q.color[..3], [78.0, 78.0, 78.0]);
            assert_eq!(q.color[3], p.color[3] * 0.5);
        }
        // Split screen: the first view moves the first half of the particles, the second the table's second half.
        let before = w.parts;
        w.draw(
            &mut OrderTable::new(),
            &ctx,
            None,
            false,
            &mut libc,
            Some(true),
        );
        assert_eq!(w.parts[..PARTS / 2], before[..PARTS / 2]);
        assert_ne!(
            w.parts[PARTS / 2..PARTS / 2 + 10],
            before[PARTS / 2..PARTS / 2 + 10]
        );
        let before = w.parts;
        w.draw(
            &mut OrderTable::new(),
            &ctx,
            None,
            false,
            &mut libc,
            Some(false),
        );
        assert_ne!(w.parts[..10], before[..10]);
        assert_eq!(w.parts[10..], before[10..]);
    }
}
