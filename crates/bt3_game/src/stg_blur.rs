//! The radial blur (`StgBlur_*`, src/battle/screen_fx.c): one per view. The frame is copied at half size into a work
//! buffer, then `passes` times copied into the other buffer and drawn over that copy zoomed in (by `scale` times 1,
//! 2, 4, ... texels on every side, shifted towards the centre direction); the last buffer is drawn back over the
//! frame as two rings of strips around the centre, whose three colours' alphas (centre, half way, corners) are the
//! blur's strength. Nothing is drawn while those three alphas are 0. Requests 9 / 0xA of a fighter set its view's
//! (`BtlFx_UpdateScreenFilter`).

use crate::battle::{Mtx, apply};
use crate::post::{PostBuf, PostOp, PostVertex, rgba};

/// `StgBlur`.
#[derive(Clone, Copy, Debug)]
struct Blur {
    center: [f32; 4],
    scale: f32,
    passes: i32,
    shift: [f32; 2],
    /// Colours 0 (centre), 1 (half way), 2 (corners) and 3 (the passes' zoomed copy), packed RGBA.
    color: [u32; 4],
    screen_space: bool,
}

impl Default for Blur {
    /// `StgBlur_Reset`: 3 passes, scale 2, grey colours with alpha 0 (off).
    fn default() -> Self {
        Self {
            center: [0.0, 0.0, 1.0, 0.0],
            scale: 2.0,
            passes: 3,
            shift: [2.0, 2.0],
            color: [0x0080_8080; 4],
            screen_space: true,
        }
    }
}

/// `gStgBlur`: the two views.
#[derive(Clone, Debug, Default)]
pub(crate) struct Blurs {
    views: [Blur; 2],
}

/// `StgBlur_SetColor*Rgba`'s packing.
fn pack(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from_le_bytes([r, g, b, a])
}

impl Blurs {
    /// `StgBlur_SetPasses`.
    pub(crate) fn set_passes(&mut self, view: usize, passes: i32) {
        self.views[view].passes = passes;
    }

    /// `StgBlur_SetColor{0..3}Rgba`.
    pub(crate) fn set_color(&mut self, view: usize, which: usize, rgba: [u8; 4]) {
        self.views[view].color[which] = pack(rgba[0], rgba[1], rgba[2], rgba[3]);
    }

    /// `StgBlur_SetCenter`: the direction normalised (its w kept); a world direction unless `screen_space`.
    pub(crate) fn set_center(&mut self, view: usize, dir: [f32; 3], screen_space: bool) {
        let b = &mut self.views[view];
        let n = crate::eft::norm(dir);
        b.center = [n[0], n[1], n[2], b.center[3]];
        b.screen_space = screen_space;
    }

    /// Whether the view's blur draws (one of the three ring alphas is not 0).
    pub(crate) fn is_on(&self, view: usize) -> bool {
        self.views[view].color[..3].iter().any(|c| c >> 24 != 0)
    }

    /// `StgBlur_Draw` of a full-screen view: `world2view` turns a world centre into the view.
    /// `split`: `StgBlur_Draw(1, view)`, the view's half of the screen through a quarter of the buffers.
    pub(crate) fn draw(&self, view: usize, world2view: &Mtx, split: bool) -> Vec<PostOp> {
        let b = &self.views[view];
        if !self.is_on(view) {
            return Vec::new();
        }
        let (src_h, h) = (448i32, 224i32);
        let (width, half, x_ofs) = if split {
            (256i32, 128i32, if view == 1 { 256 } else { 0 })
        } else {
            (512, 256, 0)
        };
        let c = if b.screen_space {
            b.center
        } else {
            let mut m = *world2view;
            m[3] = [0.0, 0.0, 0.0, 1.0];
            apply(&m, b.center)
        };
        let (hf, h_f) = (half as f32, h as f32);
        let mut ops = vec![PostOp::sprite(
            PostBuf::Frame,
            PostBuf::Work(0),
            false,
            [0.0, 0.0, hf, h_f],
            [x_ofs as f32, 0.0, (x_ofs + width) as f32, src_h as f32],
            [0x80; 4],
        )];
        // With no pass the rings read texture block 0 (the game never does that): the first buffer here.
        let mut last = PostBuf::Work(0);
        let mut f = 1.0f32;
        for i in 0..b.passes.max(0) {
            let grow = (f * b.scale) as i32;
            let mut dx = (f as i32 as f32 * b.shift[0] * c[0]) as i32;
            let mut dy = (f * b.shift[1] * c[1]) as i32;
            let (mut nx, mut ny) = (0, 0);
            if dx < 0 {
                nx = -dx;
                dx = 0;
            }
            if dy < 0 {
                ny = -dy;
                dy = 0;
            }
            let (ua, ub, va, vb) = (grow + dx, grow + nx, grow + dy, grow + ny);
            let src = PostBuf::Work((i & 1) as usize);
            let dst = PostBuf::Work(1 - (i & 1) as usize);
            // The copy half a texel off (filtered), then the zoomed copy over it.
            ops.push(PostOp::sprite(
                src,
                dst,
                false,
                [0.0, 0.0, hf, h_f],
                [0.5, 0.5, hf + 0.5, h_f + 0.5],
                [0x80; 4],
            ));
            ops.push(PostOp::sprite(
                src,
                dst,
                true,
                [0.0, 0.0, hf, h_f],
                [
                    ua as f32 + 0.5,
                    va as f32 + 0.5,
                    (half - ub) as f32 + 0.5,
                    (h - vb) as f32 + 0.5,
                ],
                rgba(b.color[3]),
            ));
            last = dst;
            f += f;
        }
        // The rings: the centre, the points half way to the corners, the corners; texels at half the positions.
        let cx = ((c[0] * 0.5 + 0.5) * width as f32) as i32;
        let cy = ((c[1] * 0.5 + 0.5) * src_h as f32) as i32;
        let (hx, hy) = (cx / 2, cy / 2);
        let (rx, ry) = (width - cx, src_h - cy);
        let (ru, rv) = (half - hx, h - hy);
        let out = [
            [0, 0, 0, 0],
            [width, 0, half, 0],
            [width, src_h, half, h],
            [0, src_h, 0, h],
        ];
        let inner = [
            [hx, hy, hx / 2, hy / 2],
            [cx + rx / 2, hy, hx + ru / 2, hy / 2],
            [cx + rx / 2, cy + ry / 2, hx + ru / 2, hy + rv / 2],
            [hx, cy + ry / 2, hx / 2, hy + rv / 2],
        ];
        let vert = |p: [i32; 4], color: u32| PostVertex {
            xy: [(p[0] + x_ofs) as f32, p[1] as f32],
            uv: [p[2] as f32 + 0.5, p[3] as f32 + 0.5],
            rgba: rgba(color),
        };
        let mut rings = PostOp {
            source: last,
            target: PostBuf::Frame,
            blend: true,
            mask: None,
            tris: Vec::new(),
        };
        let centre = [cx, cy, hx, hy];
        let mut strip = Vec::new();
        for i in 0..5 {
            strip.push(vert(centre, b.color[0]));
            strip.push(vert(inner[i % 4], b.color[1]));
        }
        rings.strip(&strip);
        strip.clear();
        for i in 0..5 {
            strip.push(vert(inner[i % 4], b.color[1]));
            strip.push(vert(out[i % 4], b.color[2]));
        }
        rings.strip(&strip);
        ops.push(rings);
        ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::IDENTITY;

    #[test]
    fn the_blur_is_off_until_a_ring_has_alpha_then_zooms_through_both_buffers() {
        let mut b = Blurs::default();
        assert!(b.draw(0, &IDENTITY, false).is_empty());
        // Only the passes' colour: still off.
        b.set_color(0, 3, [0x80, 0x80, 0x80, 0x40]);
        assert!(b.draw(0, &IDENTITY, false).is_empty());
        b.set_color(0, 2, [0x80, 0x80, 0x80, 0x80]);
        b.set_center(0, [0.0, 0.0, 2.0], false);
        let ops = b.draw(0, &IDENTITY, false);
        // The copy, two draws per pass, the rings.
        assert_eq!(ops.len(), 1 + 3 * 2 + 1);
        assert_eq!(
            (ops[0].source, ops[0].target),
            (PostBuf::Frame, PostBuf::Work(0))
        );
        let pass = |i: usize| (ops[1 + i * 2].source, ops[1 + i * 2].target);
        assert_eq!(pass(0), (PostBuf::Work(0), PostBuf::Work(1)));
        assert_eq!(pass(1), (PostBuf::Work(1), PostBuf::Work(0)));
        assert_eq!(pass(2), (PostBuf::Work(0), PostBuf::Work(1)));
        // Pass 2 (f = 4): 8 texels in on every side (a centre straight ahead does not shift).
        let z = &ops[6];
        assert!(z.blend);
        assert_eq!(z.tris[0][0].uv, [8.5, 8.5]);
        assert_eq!(z.tris[1][2].uv, [248.5, 216.5]);
        assert_eq!(z.tris[0][0].rgba, [0x80, 0x80, 0x80, 0x40]);
        // The rings: two strips of 10 (8 triangles each), from the last buffer into the frame, centred.
        let r = ops.last().unwrap();
        assert_eq!(
            (r.source, r.target, r.tris.len()),
            (PostBuf::Work(1), PostBuf::Frame, 16)
        );
        assert_eq!(r.tris[0][0].xy, [256.0, 224.0]);
        assert_eq!(r.tris[0][0].rgba[3], 0);
        assert_eq!(r.tris[8][1].rgba[3], 0x80);
    }

    #[test]
    fn a_world_centre_is_turned_into_the_view_and_shifts_the_zoom() {
        let mut b = Blurs::default();
        b.set_color(0, 1, [0x80, 0x80, 0x80, 0x20]);
        b.set_color(0, 3, [0x80, 0x80, 0x80, 0x40]);
        // To the right in the view: the zoomed texels move right by f * 2 * 1.
        b.set_center(0, [3.0, 0.0, 0.0], false);
        let ops = b.draw(0, &IDENTITY, false);
        let z = &ops[2];
        assert_eq!(z.tris[0][0].uv, [4.5, 2.5]);
        assert_eq!(z.tris[1][2].uv, [254.5, 222.5]);
        let r = ops.last().unwrap();
        assert_eq!(r.tris[0][0].xy, [512.0, 224.0]);
    }
}
