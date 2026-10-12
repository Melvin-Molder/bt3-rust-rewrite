//! The speed lines (`EftSpdLine_*`, src/battle/eft_speed_line_spawn.c and eft_aura.c): thin camera-facing quads
//! with the common effect file's texture set 0x237, in two lists.
//!
//! Trails (request 0xB, moving faster than half the reference speed): one white segment behind each of eleven body
//! nodes (more along a long limb), at a random point of a disc across the direction of travel; it follows its
//! fighter for its two frames of life, stretched back by the frame's movement. Streaks (animation event 8, a fast
//! limb): fifteen black segments in a ring around the model part, flying back along its movement while they fade.

use crate::eft::{
    ClipVtx, DrawCtx, OrderTable, Prim, V3, ZSort, add, cross, dot, norm, point, queue_tri_clip,
    scale3, sub,
};
use crate::rng::LibcRandom;

/// `EftSpdLine_AllocTrail` / `EftSpdLine_AllocStreak`: the pools.
const TRAILS: usize = 30;
const STREAKS: usize = 40;

const TWO_PI: f32 = 6.283_185_3;
const PI_F: f32 = 3.141_592_7;

/// `EFT_RANDF`: libc `rand()` scaled to 0..1.
fn randf(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// `Mathf_WrapAngle(angle, pi)`: raised by 2 pi while under 2 pi, then lowered into (-pi, pi].
fn wrap(mut a: f32) -> f32 {
    let range = PI_F + PI_F;
    while a < range {
        a += range;
    }
    while PI_F < a {
        a -= range;
    }
    a
}

/// `Mathf_Sin`.
fn sin(a: f32) -> f32 {
    wrap(a).sin()
}

/// `Mathf_Cos`.
fn cos(a: f32) -> f32 {
    sin(a + PI_F / 2.0)
}

/// `Vec3_Length`.
fn length(v: V3) -> f32 {
    dot(v, v).sqrt()
}

/// A body node a trail starts from, with a second node: when the two are more than 6 apart, extra trails follow.
const BODY_NODES: [(u32, Option<u32>); 11] = [
    (0x0A, Some(0x08)),
    (0x08, None),
    (0x0E, Some(0x0C)),
    (0x0C, None),
    (0x13, Some(0x14)),
    (0x14, Some(0x15)),
    (0x21, Some(0x22)),
    (0x22, Some(0x23)),
    (0x10, Some(0x11)),
    (0x11, Some(0x30)),
    (0x30, None),
];

/// What the speed lines read of a fighter.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SpdChr {
    /// `BtlCharApi_GetHeight`.
    pub height: f32,
    /// `BtlCharApi_GetFrameMove`: the whole movement of the previous frame.
    pub moved: V3,
}

/// `EftSpdTrail`.
#[derive(Clone, Copy, Debug)]
struct Trail {
    chr: usize,
    pos: V3,
    tail: V3,
    dir: V3,
    width: f32,
    alpha: f32,
    life: f32,
}

/// `EftSpdStreak`.
#[derive(Clone, Copy, Debug)]
struct Streak {
    chr: usize,
    pos: V3,
    tail: V3,
    dir: V3,
    speed: f32,
    width: f32,
    alpha: f32,
    fade: f32,
}

/// `EftSpdLineWork`: both lists in the order they were added (a freed slot is reused, the list order is kept).
#[derive(Clone, Debug, Default)]
pub(crate) struct SpdLines {
    trails: Vec<Trail>,
    streaks: Vec<Streak>,
    /// `texReady`: a segment was alive at the last update.
    tex_ready: bool,
}

/// The axes across `back`: `side` = back x (0, -1, 0) and `up2` = back x side, both normalised.
fn frame(back: V3) -> (V3, V3) {
    let side = norm(cross(back, [0.0, -1.0, 0.0]));
    let up2 = norm(cross(back, side));
    (side, up2)
}

impl SpdLines {
    /// Live trails and streaks.
    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.trails.len(), self.streaks.len())
    }

    /// `EftSpdLine_AddTrail`: nothing when the pool is full; else a random width scaled by the fighter's height
    /// (one draw), alpha 64, two frames of life.
    fn add_trail(&mut self, chr: usize, pos: V3, dir: V3, height: f32, libc: &mut LibcRandom) {
        if self.trails.len() >= TRAILS {
            return;
        }
        let width = (randf(libc) * 0.1 + 0.05) * (height * 0.05);
        self.trails.push(Trail {
            chr,
            pos,
            tail: pos,
            dir,
            width,
            alpha: 64.0,
            life: 2.0,
        });
    }

    /// `EftSpdLine_AddStreak`: nothing when the pool is full; else alpha 160, fading out in `life` seconds.
    #[allow(clippy::too_many_arguments)]
    fn add_streak(
        &mut self,
        chr: usize,
        pos: V3,
        tail: V3,
        dir: V3,
        speed: f32,
        life: f32,
        width: f32,
    ) {
        if self.streaks.len() >= STREAKS {
            return;
        }
        self.streaks.push(Streak {
            chr,
            pos,
            tail,
            dir,
            speed,
            width,
            alpha: 160.0,
            fade: 160.0 / (life * 30.0),
        });
    }

    /// `EftSpdLine_SpawnBodyTrails`: one trail behind each body node at a random point of a disc of radius
    /// height * 0.05 across the direction of travel, 1.5..6.5 times that behind it (three draws, then the trail's).
    /// A node whose second node is more than 6 away sets how many extra trails follow, and the count stays for the
    /// nodes after it.
    pub(crate) fn spawn_body_trails(
        &mut self,
        chr: usize,
        dir: V3,
        height: f32,
        node_pos: impl Fn(u32) -> V3,
        libc: &mut LibcRandom,
    ) {
        let back = norm(scale3(dir, -1.0));
        let (side, up2) = frame(back);
        let unit = height * 0.05;
        let mut extra = 0.0f32;
        for (node, node2) in BODY_NODES {
            let p0 = node_pos(node);
            if let Some(n2) = node2 {
                let len = length(sub(node_pos(n2), p0));
                if len > 6.0 {
                    extra = len / 6.0;
                }
            }
            for _ in 0..1 + extra as i32 {
                let angle = randf(libc) * TWO_PI;
                let radius = randf(libc) + 0.0;
                let mut pos = add(p0, scale3(side, cos(angle) * radius * unit));
                pos = add(pos, scale3(up2, sin(angle) * radius * unit));
                pos = add(pos, scale3(back, (randf(libc) * 5.0 + 1.5) * unit));
                self.add_trail(chr, pos, back, height, libc);
            }
        }
    }

    /// `EftSpdLine_SpawnPartStreaks`: fifteen streaks around model part `part` flying against `dir`, each at a
    /// random point of a ring 0.1..1.9 sizes out, 1.5..3.5 sizes ahead, 3..6 sizes long (six draws). The size is
    /// the part's size factor * 0.4, or height * 0.04 when that is under 0.8.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_part_streaks(
        &mut self,
        chr: usize,
        dir: V3,
        part_pos: V3,
        size_factor: f32,
        height: f32,
        libc: &mut LibcRandom,
    ) {
        let mut size = size_factor * 0.4;
        if size < 0.8 {
            size = height * 0.05 * 0.8;
        }
        let back = norm(scale3(dir, -1.0));
        let (side, up2) = frame(back);
        for _ in 0..15 {
            let angle = randf(libc) * TWO_PI;
            let radius = (randf(libc) * 1.8 + 0.1) * size;
            let lead = (randf(libc) * 2.0 + 1.5) * size;
            let mut pos = add(part_pos, scale3(side, cos(angle) * radius));
            pos = add(pos, scale3(up2, sin(angle) * radius));
            pos = sub(pos, scale3(back, lead));
            let tail = add(pos, scale3(back, (randf(libc) * 3.0 + 3.0) * size));
            let speed = (randf(libc) * 2.0 + 1.0) * (height * 0.05);
            let life = randf(libc) * 0.09 + 0.06;
            self.add_streak(chr, pos, tail, back, speed, life, 0.2);
        }
    }

    /// `EftSpdLine_Update`: unless paused, every trail moves with its fighter (one draw) and loses a frame, every
    /// streak moves along its direction and fades; the finished ones leave their lists. Then the texture's state.
    pub(crate) fn update(&mut self, paused: bool, chars: &[SpdChr; 2], libc: &mut LibcRandom) {
        if !paused {
            self.trails.retain_mut(|t| {
                let c = chars[t.chr];
                let unit = 0.05;
                t.pos = add(t.pos, scale3(c.moved, c.height * unit * 0.2));
                let k = -((randf(libc) * 0.8 + 0.8) * (c.height * unit));
                t.tail = add(t.pos, scale3(c.moved, k));
                t.life -= 1.0;
                if t.life <= 0.0 {
                    t.life = 0.0;
                    return false;
                }
                true
            });
            self.streaks.retain_mut(|s| {
                let step = scale3(s.dir, s.speed);
                s.pos = add(s.pos, step);
                s.tail = add(s.tail, step);
                s.alpha -= s.fade;
                if s.alpha <= 0.0 {
                    s.alpha = 0.0;
                    return false;
                }
                true
            });
        }
        // EftSpdLine_UpdateTexture.
        self.tex_ready = !self.trails.is_empty() || !self.streaks.is_empty();
    }

    /// `EftSpdLine_Draw`: the trails white, then (once the texture is kept) the streaks black, each with its alpha;
    /// drawn in front while its fighter is in view.
    pub(crate) fn draw(
        &self,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
        texture: Option<usize>,
        in_view: impl Fn(usize) -> bool,
    ) {
        for t in &self.trails {
            let quad = quad(ctx.eye, t.tail, t.pos, t.dir, t.width);
            draw_quad(
                ot,
                ctx,
                &quad,
                texture,
                [255.0, 255.0, 255.0, t.alpha],
                in_view(t.chr),
            );
        }
        if !self.tex_ready {
            return;
        }
        for s in &self.streaks {
            let quad = quad(ctx.eye, s.pos, s.tail, s.dir, s.width * 0.5);
            draw_quad(
                ot,
                ctx,
                &quad,
                texture,
                [0.0, 0.0, 0.0, s.alpha],
                in_view(s.chr),
            );
        }
    }
}

/// `EftSpdLine_BuildTrailQuad` / `EftSpdLine_BuildStreakQuad`: each end widened by `half` across the line of
/// sight from the camera to it.
fn quad(eye: V3, a: V3, b: V3, dir: V3, half: f32) -> [V3; 4] {
    let across = |p: V3| scale3(norm(cross(dir, norm(sub(eye, p)))), half);
    let (sa, sb) = (across(a), across(b));
    [add(a, sa), sub(a, sa), add(b, sb), sub(b, sb)]
}

/// `EftSpdLine_DrawQuad`: corners 0-1-2 and 1-2-3, one colour, the average depth.
fn draw_quad(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    quad: &[V3; 4],
    texture: Option<usize>,
    col: [f32; 4],
    front: bool,
) {
    const UV: [[f32; 4]; 4] = [
        [0.0, 0.0, 1.0, 0.0],
        [1.0, 0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0, 0.0],
        [1.0, 1.0, 1.0, 0.0],
    ];
    for i in 0..2 {
        let tri = std::array::from_fn(|k| ClipVtx {
            pos: point(quad[i + k]),
            uv: UV[i + k],
            col,
        });
        let prim = Prim {
            texture,
            ot_layer: 0,
            z: ZSort::Average(1.0),
            front,
            clamp: false,
            alpha_min: 0.01,
        };
        queue_tri_clip(ot, ctx, tri, prim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(n: u32) -> V3 {
        // A long arm: node 0x14 is 12 away from 0x13 and 0x15.
        match n {
            0x14 => [12.0, 0.0, 0.0],
            _ => [0.0; 3],
        }
    }

    #[test]
    fn body_trails_follow_the_fighter_for_two_frames() {
        let mut lines = SpdLines::default();
        let mut libc = LibcRandom::new(1);
        lines.spawn_body_trails(0, [0.0, 0.0, 1.0], 20.0, nodes, &mut libc);
        // Eleven nodes; from the long arm on, two extra trails each (the count stays): 4 + 7 * 3.
        assert_eq!(lines.counts(), (4 + 7 * 3, 0));
        let t = lines.trails[0];
        // Behind the node (+z is the direction of travel), within the disc.
        assert!(t.pos[2] < 0.0 && t.pos[2] >= -6.5 * 20.0 * 0.05);
        assert!(t.width >= 0.05 && t.width <= 0.15 + 1e-6);
        assert_eq!(t.dir, [0.0, 0.0, -1.0]);
        let chars = [
            SpdChr {
                height: 20.0,
                moved: [0.0, 0.0, 2.0],
            },
            SpdChr::default(),
        ];
        lines.update(false, &chars, &mut libc);
        let moved = lines.trails[0];
        assert!((moved.pos[2] - (t.pos[2] + 2.0 * 0.2)).abs() < 1e-5);
        // The tail is pulled back by 0.8..1.6 heights * 0.05 of the movement.
        let back = moved.pos[2] - moved.tail[2];
        assert!(
            back >= 0.8 * 2.0 - 1e-5 && back <= 1.6 * 2.0 + 1e-5,
            "{back}"
        );
        assert!(lines.tex_ready);
        // A pause keeps them.
        lines.update(true, &chars, &mut libc);
        assert_eq!(lines.counts().0, 25);
        lines.update(false, &chars, &mut libc);
        assert_eq!(lines.counts(), (0, 0));
        assert!(!lines.tex_ready);
    }

    #[test]
    fn the_trail_pool_holds_thirty() {
        let mut lines = SpdLines::default();
        let mut libc = LibcRandom::new(1);
        for _ in 0..3 {
            lines.spawn_body_trails(
                1,
                [1.0, 0.0, 0.0],
                20.0,
                |n| [0.0, n as f32 * -0.1, 0.0],
                &mut libc,
            );
        }
        assert_eq!(lines.counts(), (TRAILS, 0));
    }

    #[test]
    fn part_streaks_fly_back_and_fade() {
        let mut lines = SpdLines::default();
        let mut libc = LibcRandom::new(1);
        // A small part: the size falls back to height * 0.04.
        lines.spawn_part_streaks(0, [0.0, 0.0, 3.0], [0.0; 3], 1.0, 20.0, &mut libc);
        assert_eq!(lines.counts(), (0, 15));
        let size = 20.0 * 0.05 * 0.8;
        for s in &lines.streaks {
            assert_eq!(s.dir, [0.0, 0.0, -1.0]);
            // Ahead of the part, the tail behind the head.
            assert!(s.pos[2] >= 1.5 * size - 1e-5 && s.pos[2] <= 3.5 * size + 1e-5);
            let len = s.pos[2] - s.tail[2];
            assert!(len >= 3.0 * size - 1e-5 && len <= 6.0 * size + 1e-5);
            let r = (s.pos[0] * s.pos[0] + s.pos[1] * s.pos[1]).sqrt();
            assert!(r >= 0.1 * size - 1e-5 && r <= 1.9 * size + 1e-5);
            assert!(s.speed >= 1.0 && s.speed <= 3.0 + 1e-6);
            assert_eq!(s.width, 0.2);
            assert_eq!(s.alpha, 160.0);
            // 0.06..0.15 seconds.
            assert!(s.fade >= 160.0 / 4.5 - 1e-3 && s.fade <= 160.0 / 1.8 + 1e-3);
        }
        let first = lines.streaks[0];
        let chars = [SpdChr::default(); 2];
        lines.update(false, &chars, &mut libc);
        let s = lines.streaks[0];
        assert!((s.pos[2] - (first.pos[2] - first.speed)).abs() < 1e-5);
        assert_eq!(s.alpha, 160.0 - first.fade);
        for _ in 0..4 {
            lines.update(false, &chars, &mut libc);
        }
        assert_eq!(lines.counts(), (0, 0));
    }

    #[test]
    fn streaks_draw_only_once_the_texture_is_kept() {
        let mut lines = SpdLines::default();
        let mut libc = LibcRandom::new(1);
        lines.spawn_part_streaks(0, [1.0, 0.0, 0.0], [0.0, 0.0, 50.0], 4.0, 20.0, &mut libc);
        let view = crate::battle::View::new([0.0; 3], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = OrderTable::new();
        lines.draw(&mut ot, &ctx, Some(0), |_| false);
        assert!(ot.into_list().is_empty());
        lines.update(false, &[SpdChr::default(); 2], &mut libc);
        let mut ot = OrderTable::new();
        lines.draw(&mut ot, &ctx, Some(0), |_| true);
        let tris = ot.into_list();
        assert!(!tris.is_empty());
        // Black, in front.
        assert!(tris.iter().all(|t| {
            t.vertices
                .iter()
                .all(|v| v.rgba[..3] == [0, 0, 0] && v.position[2] == 0xFF_FFFF as f32)
        }));
    }
}
