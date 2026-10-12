//! The pan blur (`StgPanBlur_*`, src/sys/gfx_post.c), on stages with flag 8 (feature 0xD), in `StgFx_DrawPre`
//! after the fog: while a battle camera moves sideways (its position, height ignored, between 1 and 2.5 or past 4
//! units a frame raises the strength by 0.1, else it falls by 0.15) the frame is copied at half size, shrunk again
//! with two copies shifted toward the side the camera turns (by the cross product of its last two directions from
//! the origin), and stretched back over the view, blended by the depth byte's CLUT (alpha 0x80 - byte / 2: the near
//! stage more) times the strength. Each view follows its own camera (full screen: view 0's, the fighter 0 camera);
//! the strengths step only while the battle runs.
//!
//! The original carries the depth alpha through the shrunk copies (the texture alpha of each); here the copies
//! carry colour only and the CLUT, scaled by the strength, weighs the final blend.

use crate::battle::{Mtx, inverse_rt};
use crate::post::{PostBuf, PostOp};

/// `StgPanBlurView`.
#[derive(Clone, Copy, Debug)]
struct View {
    cur: [f32; 3],
    prev: [f32; 3],
    max_alpha: f32,
    strength: f32,
    side: f32,
    /// Each layer's sideways shift and alpha.
    layers: [(f32, i32); 2],
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l == 0.0 { [0.0; 3] } else { v.map(|c| c / l) }
}

impl View {
    /// `StgPanBlur_UpdateView` with the camera's world-to-view matrix: false once the strength has run out.
    fn update(&mut self, world2view: &Mtx) -> bool {
        self.prev = self.cur;
        let m = inverse_rt(world2view);
        self.cur = [m[3][0], 0.0, m[3][2]];
        self.prev[1] = 0.0;
        let (a, b) = (norm(self.prev), norm(self.cur));
        // Vec3_Cross(side, dirPrev, dirCur): its y.
        let side_y = a[2] * b[0] - a[0] * b[2];
        let mv = [
            self.prev[0] - self.cur[0],
            self.prev[1] - self.cur[1],
            self.prev[2] - self.cur[2],
        ];
        let dist = (mv[0] * mv[0] + mv[1] * mv[1] + mv[2] * mv[2]).sqrt();
        if dist >= 1.0 {
            if dist < 2.5 {
                self.strength += 0.1;
            }
            if (2.5..4.0).contains(&dist) {
                self.strength -= 0.15;
            }
            if dist > 4.0 {
                self.strength += 0.1;
            }
        } else {
            self.strength -= 0.15;
        }
        self.strength = self.strength.min(1.0);
        if self.strength < 0.0 {
            self.strength = 0.0;
            return false;
        }
        self.side = (side_y * 100.0).clamp(-1.0, 1.0);
        for i in 0..2 {
            let a = ((self.max_alpha * self.strength) as i32 - i as i32 * 32).max(0);
            self.layers[i] = ((i + 1) as f32 * self.side, a);
        }
        true
    }
}

/// `gStgPanBlur`: the two views.
#[derive(Clone, Debug)]
pub(crate) struct PanBlur {
    views: [View; 2],
}

impl Default for PanBlur {
    /// `StgPanBlur_Init`: no strength, the depth alpha's maximum 101.
    fn default() -> Self {
        let v = View {
            cur: [0.0; 3],
            prev: [0.0; 3],
            max_alpha: 101.0,
            strength: 0.0,
            side: 0.0,
            layers: [(0.0, 0); 2],
        };
        Self { views: [v; 2] }
    }
}

/// `StgPanBlur_BuildClut` (alpha 0x80 - byte / 2) under the strength's alpha: the final blend's weight.
fn mask(strength_alpha: i32) -> Box<[u8; 256]> {
    Box::new(std::array::from_fn(|i| {
        let a = (0x80 - (i as i32 >> 1)).max(0);
        ((a * strength_alpha) >> 7) as u8
    }))
}

impl PanBlur {
    /// The strengths (for tests).
    #[cfg(test)]
    pub(crate) fn strength(&self, view: usize) -> f32 {
        self.views[view].strength
    }

    /// `StgPanBlur_Draw`: each view's update (not while paused) with its camera's world-to-view matrix, then the
    /// half-size copy and each view with a strength drawn: the quarter-size copy, the two shifted layers over it,
    /// the stretch back over the view, masked.
    pub(crate) fn draw(&mut self, split: bool, paused: bool, cams: [&Mtx; 2]) -> Vec<PostOp> {
        let n = if split { 2 } else { 1 };
        let mut on = [true; 2];
        if !paused {
            for (i, o) in on.iter_mut().enumerate().take(n) {
                *o = self.views[i].update(cams[i]);
            }
        }
        if !on[..n].iter().any(|&o| o) {
            return Vec::new();
        }
        // StgPanBlur_CopyScreen: the frame at half size.
        let mut ops = vec![PostOp::sprite(
            PostBuf::Frame,
            PostBuf::Work(0),
            false,
            [0.0, 0.0, 256.0, 224.0],
            [0.0, 0.0, 512.0, 448.0],
            [0x80; 4],
        )];
        for i in 0..n {
            if on[i] {
                ops.extend(self.draw_view(i, split));
            }
        }
        ops
    }

    /// `StgPanBlur_DrawView`: the view's part of the copy into a quarter of the other buffer at the strength, the
    /// two layers over it shifted by their offsets (128-pixel units), then back over the view's rectangle.
    fn draw_view(&self, i: usize, split: bool) -> Vec<PostOp> {
        let v = &self.views[i];
        let (src_x, src_w) = if split {
            (if i == 0 { 0.0 } else { 128.0 }, 128.0)
        } else {
            (0.0, 256.0)
        };
        let (qw, qh) = (128.0f32, 112.0f32);
        let strength_alpha = (v.max_alpha * v.strength) as i32;
        let mut ops = vec![PostOp::sprite(
            PostBuf::Work(0),
            PostBuf::Work(1),
            false,
            [0.0, 0.0, qw, qh],
            [src_x, 0.0, src_x + src_w, 224.0],
            [0x80; 4],
        )];
        for &(ofs, alpha) in &v.layers {
            ops.push(PostOp::sprite(
                PostBuf::Work(0),
                PostBuf::Work(1),
                true,
                [ofs, 0.0, qw + ofs, qh],
                [src_x + 0.5, 0.5, src_x + src_w + 0.5, 224.5],
                [0x80, 0x80, 0x80, alpha.clamp(0, 255) as u8],
            ));
        }
        let (x0, w) = if split {
            (if i == 0 { 0.0 } else { 256.0 }, 256.0)
        } else {
            (0.0, 512.0)
        };
        let mut back = PostOp::sprite(
            PostBuf::Work(1),
            PostBuf::Frame,
            true,
            [x0, 0.0, x0 + w, 448.0],
            [0.5, 0.5, qw - 0.5, qh - 0.5],
            [0x80; 4],
        );
        back.mask = Some(mask(strength_alpha));
        back.region = Some([0, 0, qw as u16, qh as u16]);
        ops.push(back);
        ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::IDENTITY;

    fn at(x: f32, z: f32) -> Mtx {
        // A camera at (x, 0, z) looking along z: its world-to-view matrix.
        let mut m = IDENTITY;
        m[3] = [-x, 0.0, -z, 1.0];
        m
    }

    #[test]
    fn the_blur_builds_with_a_sideways_pan_and_fades_when_still() {
        let mut p = PanBlur::default();
        // The first frame jumps from the origin: strength 0.1.
        let ops = p.draw(false, false, [&at(100.0, 0.0), &at(0.0, 0.0)]);
        assert!((p.strength(0) - 0.1).abs() < 1e-6);
        // Copy, quarter copy, two layers, back (masked by the CLUT at the strength's alpha 10).
        assert_eq!(ops.len(), 5);
        let mask = ops[4].mask.as_ref().unwrap();
        assert_eq!(mask[0], ((0x80 * 10) >> 7) as u8);
        assert_eq!(mask[0xFF], ((1 * 10) >> 7) as u8);
        // Moving 2 a frame builds it up, standing still lets it run out.
        for k in 1..5 {
            p.draw(false, false, [&at(100.0 + 2.0 * k as f32, 0.0), &IDENTITY]);
        }
        assert!((p.strength(0) - 0.5).abs() < 1e-5, "{}", p.strength(0));
        let still = at(108.0, 0.0);
        for _ in 0..3 {
            assert!(!p.draw(false, false, [&still, &IDENTITY]).is_empty());
        }
        assert!(p.draw(false, false, [&still, &IDENTITY]).is_empty());
        // Paused: no update, the last layers drawn.
        p.views[0].strength = 0.3;
        assert_eq!(p.draw(false, true, [&still, &IDENTITY]).len(), 5);
    }
}
