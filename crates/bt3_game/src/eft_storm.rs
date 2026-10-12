//! The storm (`EftStorm_*`, src/battle/eft_shot.c; stage effect kind 3, on a stage whose pack has entry 0xE:
//! stage 15): four lightning bolts on the horizon, about 5000 away and 700 up, each living 16 to 31 frames with
//! one or two camera-facing sprites of the pack's textures (additive, fading with their life), and 64 rain drops
//! falling 19.6 a frame, kept in a 200-wide cube 150 in front of the camera and drawn as grey streaks that lean
//! against the camera's movement. The rain's wrapping draws random numbers while drawing.

use crate::battle::{Mtx, apply, inverse_rt};
use crate::eft::{
    DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, V3, add,
    depth_slot, norm, point, scale3, sub, xyz,
};
use crate::rng::LibcRandom;

const BOLTS: usize = 4;
const DROPS: usize = 64;

/// `EftStormBolt`.
#[derive(Clone, Copy, Debug, Default)]
struct Bolt {
    pos: V3,
    life: i32,
    life_max: i32,
    tex_a: usize,
    tex_b: usize,
    wait: i32,
    /// 1: the second sprite only.
    kind: i32,
}

/// `gEftStorm`.
#[derive(Clone, Debug)]
pub(crate) struct Storm {
    /// Stage pack entry 0xE's textures among the effect textures.
    textures: Vec<usize>,
    bolts: [Bolt; BOLTS],
    drops: [V3; DROPS],
    /// The camera's position at the last draw (`prevCam[0]`).
    prev_cam: V3,
}

impl Storm {
    /// `EftStorm_Init` before its random draws (`init`).
    pub(crate) fn new(textures: Vec<usize>) -> Self {
        Self {
            textures,
            bolts: [Bolt::default(); BOLTS],
            drops: [[0.0; 3]; DROPS],
            prev_cam: [0.0; 3],
        }
    }

    /// `EftStorm_Init`: the bolts (`EftStorm_Reset`), then the drops scattered in a 1023-wide cube around the
    /// origin.
    pub(crate) fn init(&mut self, libc: &mut LibcRandom) {
        self.reset(libc);
        for d in &mut self.drops {
            for c in d.iter_mut() {
                *c = ((libc.rand() & 0x3FF) - 0x1FF) as f32;
            }
        }
    }

    /// `EftStorm_Reset`: every bolt placed again.
    pub(crate) fn reset(&mut self, libc: &mut LibcRandom) {
        for i in 0..BOLTS {
            self.spawn(i, libc);
        }
    }

    /// The bolts' lives.
    #[cfg(test)]
    pub(crate) fn lives(&self) -> [i32; BOLTS] {
        self.bolts.map(|b| b.life)
    }

    /// `EftStorm_SpawnBolt`: on the horizon 4489 to 5511 away (one sprite nearer than 5000, else one or two),
    /// 700 up, waiting a frame, living 16 to 31; dropped when within 700 of a live one.
    fn spawn(&mut self, i: usize, libc: &mut LibcRandom) {
        let dist = ((libc.rand() & 0x3FF) - 0x1FF) as f32 + 5000.0;
        let mut x = ((libc.rand() & 0x1FF) - 0xFF) as f32;
        let z = ((libc.rand() & 0xFF) - 0x1F) as f32;
        if x == 0.0 && z == 0.0 {
            x = 1.0;
        }
        let mut pos = scale3(norm([x, 0.0, z]), dist);
        let life = ((libc.rand() & 0x1F) + 0x20) >> 1;
        let tex_a = (libc.rand() as u32 % 3) as usize;
        let r = libc.rand();
        let r2 = libc.rand() & 1;
        pos[1] = -700.0;
        let tex_b = ((r & 1) + r2 + 3) as usize;
        libc.rand();
        let kind = if dist < 5000.0 { 1 } else { libc.rand() & 1 };
        let mut b = Bolt {
            pos,
            life,
            life_max: life,
            tex_a,
            tex_b,
            wait: 1,
            kind,
        };
        let near = self.bolts.iter().enumerate().any(|(k, o)| {
            if k == i || o.life == 0 {
                return false;
            }
            let d = sub(o.pos, b.pos);
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 490_000.0
        });
        if near {
            b.life = 0;
        }
        self.bolts[i] = b;
    }

    /// `EftStorm_Update` (not while paused): the bolts wait, live and are placed again; the rain falls.
    pub(crate) fn update(&mut self, paused: bool, libc: &mut LibcRandom) {
        if paused {
            return;
        }
        for i in 0..BOLTS {
            let b = &mut self.bolts[i];
            if b.wait != 0 {
                b.wait -= 1;
            } else if b.life != 0 {
                b.life -= 1;
            } else {
                self.spawn(i, libc);
            }
        }
        for d in &mut self.drops {
            d[1] += 19.6;
        }
    }

    /// `EftStorm_Draw`: the bolts, then the rain (wrapped around the camera first).
    /// `bright`: `EftStage_GetTintScale`.
    pub(crate) fn draw(
        &mut self,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
        bright: f32,
        libc: &mut LibcRandom,
    ) {
        let cam = inverse_rt(&ctx.world2view);
        self.draw_bolts(ot, ctx, &cam, bright);
        // EftStorm_DrawRain: the streak leans against the camera's movement since the last draw.
        let center = xyz(apply(&cam, [0.0, 0.0, 150.0, 1.0]));
        let eye = xyz(cam[3]);
        let streak = scale3(norm(add([0.0, 100.0, 0.0], sub(self.prev_cam, eye))), 20.0);
        self.prev_cam = eye;
        self.wrap_rain(center, libc);
        for d in &self.drops {
            rain_line(ot, ctx, *d, add(*d, streak));
        }
    }

    /// `EftStorm_DrawBolts`: corners (-4, -1.5), (4, 8), (-8, -3), (8, 0) times 150 turned with the camera
    /// around each live bolt; a bolt with a corner off the GS area is skipped.
    fn draw_bolts(&self, ot: &mut OrderTable, ctx: &DrawCtx, cam: &Mtx, bright: f32) {
        let col = bright * 128.0;
        let corners = [[-4.0, -1.5], [4.0, 8.0], [-8.0, -3.0], [8.0, -0.0]]
            .map(|[x, y]| xyz(apply(cam, [x * 150.0, y * 150.0, 0.0, 0.0])));
        for b in self.bolts.iter().filter(|b| b.life != 0 && b.wait == 0) {
            let mut scr = [(0i32, 0i32, 0i32); 4];
            let mut ok = true;
            for (j, c) in corners.iter().enumerate() {
                let v = apply(&ctx.world2screen, point(add(*c, b.pos)));
                let q = 1.0 / v[3];
                scr[j] = (
                    (v[0] * q * 16.0) as i32,
                    (v[1] * q * 16.0) as i32,
                    (v[2] * q) as i32,
                );
                if !valid(scr[j]) {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            let alpha = ((b.life << 7) / b.life_max) as u8;
            let rgba = [col as u32 as u8, col as u32 as u8, col as u32 as u8, alpha];
            if b.kind == 0 {
                self.sprite(ot, scr[0], scr[1], b.tex_a, rgba);
            }
            self.sprite(ot, scr[2], scr[3], b.tex_b, rgba);
        }
    }

    /// One lightning sprite: an upright rectangle between two projected corners at the first one's depth, in the
    /// additive layer.
    fn sprite(
        &self,
        ot: &mut OrderTable,
        a: (i32, i32, i32),
        b: (i32, i32, i32),
        tex: usize,
        rgba: [u8; 4],
    ) {
        let xy = [(a.0, a.1), (b.0, a.1), (a.0, b.1), (b.0, b.1)];
        let st = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let vert = |k: usize| EffectVertex {
            position: [
                xy[k].0 as f32 / 16.0 - OFFSET_X,
                xy[k].1 as f32 / 16.0 - OFFSET_Y,
                (a.2 & 0xFF_FFFF) as f32,
            ],
            stq: [st[k][0], st[k][1], 1.0],
            rgba,
        };
        for k in 0..2 {
            ot.add(
                depth_slot(a.2 >> 8),
                1,
                EffectTri {
                    texture: self.textures.get(tex).copied(),
                    blend: EffectBlend::Add,
                    clamp: false,
                    zwrite: false,
                    alpha_ref: 0,
                    vertices: [vert(k), vert(k + 1), vert(k + 2)],
                },
            );
        }
    }

    /// `EftStorm_WrapRain`: a drop leaving the cube re-enters through the opposite face; through the top or the
    /// bottom it also gets a new random place across.
    fn wrap_rain(&mut self, center: V3, libc: &mut LibcRandom) {
        let across =
            |libc: &mut LibcRandom, c: f32| c + ((libc.rand() & 0x3FF) - 0x1FF) as f32 * 0.25;
        for d in &mut self.drops {
            if center[0] + 100.0 < d[0] {
                d[0] -= 200.0;
            }
            if d[0] < center[0] - 100.0 {
                d[0] += 200.0;
            }
            if center[1] + 100.0 < d[1] {
                d[0] = across(libc, center[0]);
                d[1] -= 200.0;
                d[2] = across(libc, center[2]);
            }
            if d[1] < center[1] - 100.0 {
                d[0] = across(libc, center[0]);
                d[1] += 200.0;
                d[2] = across(libc, center[2]);
            }
            if center[2] + 100.0 < d[2] {
                d[2] -= 200.0;
            }
            if d[2] < center[2] - 100.0 {
                d[2] += 200.0;
            }
        }
    }
}

/// `EftScr_IsValid`.
fn valid(p: (i32, i32, i32)) -> bool {
    p.2 > 0 && p.0 > 0 && p.0 <= 0xFFFF && p.1 > 0 && p.1 <= 0xFFFF
}

/// `EftStorm_DrawRainLines` of one drop: a grey line (0x60, alpha 0x20) in the first depth slot, both ends on the
/// GS area. The pass has only triangles: the line is a quad one pixel wide.
fn rain_line(ot: &mut OrderTable, ctx: &DrawCtx, a: V3, b: V3) {
    let mut scr = [(0i32, 0i32, 0i32); 2];
    for (k, p) in [a, b].into_iter().enumerate() {
        let v = apply(&ctx.world2screen, point(p));
        let q = 1.0 / v[3];
        let (x, y) = (v[0] * q, v[1] * q);
        if !(0.0 < x && x < 4096.0 && 0.0 < y && y < 4096.0 && v[3] > 0.0) {
            return;
        }
        scr[k] = ((x * 16.0) as i32, (y * 16.0) as i32, (v[2] * q) as i32);
        if !valid(scr[k]) {
            return;
        }
    }
    let p = scr.map(|s| [s.0 as f32 / 16.0 - OFFSET_X, s.1 as f32 / 16.0 - OFFSET_Y]);
    let (dx, dy) = (p[1][0] - p[0][0], p[1][1] - p[0][1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (nx, ny) = (-dy / len * 0.5, dx / len * 0.5);
    let corner = |k: usize, s: f32| EffectVertex {
        position: [
            p[k][0] + nx * s,
            p[k][1] + ny * s,
            (scr[k].2 & 0xFF_FFFF) as f32,
        ],
        stq: [0.0, 0.0, 1.0],
        rgba: [0x60, 0x60, 0x60, 0x20],
    };
    let quad = [
        corner(0, -1.0),
        corner(0, 1.0),
        corner(1, -1.0),
        corner(1, 1.0),
    ];
    for k in 0..2 {
        ot.add(
            depth_slot(0),
            0,
            EffectTri {
                texture: None,
                blend: EffectBlend::Alpha,
                clamp: false,
                zwrite: false,
                alpha_ref: 0,
                vertices: [quad[k], quad[k + 1], quad[k + 2]],
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;

    #[test]
    fn bolts_live_on_the_horizon_and_the_rain_falls_around_the_camera() {
        let mut s = Storm::new((20..26).collect());
        let mut libc = LibcRandom::new(11);
        s.init(&mut libc);
        for b in &s.bolts {
            let d = (b.pos[0] * b.pos[0] + b.pos[2] * b.pos[2]).sqrt();
            assert!((4488.0..5512.0).contains(&d), "{d}");
            assert_eq!((b.pos[1], b.wait), (-700.0, 1));
            assert!(b.life == 0 || (16..=31).contains(&b.life));
            assert!((0..3).contains(&b.tex_a) && (3..=5).contains(&b.tex_b));
        }
        // Paused: nothing.
        let before = s.lives();
        s.update(true, &mut libc);
        assert_eq!(s.lives(), before);
        for _ in 0..100 {
            s.update(false, &mut libc);
        }
        // Looking along the bolts' line, the rain draws around the camera, in the first depth slot.
        let view = View::new([0.0; 3], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        // Each draw moves a drop by one cube at most: a few bring them all in (a drop wrapped through the top or
        // bottom lands up to 128 across, which the next draw corrects).
        for _ in 0..12 {
            s.draw(&mut OrderTable::new(), &ctx, 1.0, &mut libc);
        }
        let mut ot = OrderTable::new();
        s.draw(&mut ot, &ctx, 1.0, &mut libc);
        assert!(
            s.drops
                .iter()
                .all(|d| d[0].abs() <= 128.0 && (d[2] - 150.0).abs() <= 128.0)
        );
        let tris = ot.into_list();
        let rain = tris.iter().filter(|t| t.texture.is_none()).count();
        assert!(rain > 20, "{rain}");
        assert!(
            tris.iter()
                .filter(|t| t.texture.is_some())
                .all(|t| t.blend == EffectBlend::Add)
        );
    }
}
