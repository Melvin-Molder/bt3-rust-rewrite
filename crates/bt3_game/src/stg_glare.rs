//! The sky glare (`StgGlare_*` and `GfxPost_DrawGlow`, src/sys/gfx_post.c), on stages whose glare record (section
//! +0x48) has bit 0, first among `StgFx_DrawPre`'s passes after the depth tint. The depth byte through the glare's
//! CLUT (alpha 0x80 for the sky's 0xFF, the record's alpha elsewhere) weighs the frame, halved into a work buffer
//! over black; that is shrunk to a quarter, then to an eighth one texel on, passed back and forth ten times between
//! two eighth-size buffers (every other pass one texel on, the bilinear filter blurring the rest), grown back
//! through the quarter (tinted by the glare's level) and the half, and added over the frame.
//!
//! The level is the glow's brightness: with the record's bit 1 it rises by the step a frame (to the maximum, then
//! settling to the hold level) while the stage light's point is on the view, and falls by 1 (to the minimum) while
//! it is not; without it, it stays at the maximum. It steps once a drawn frame, paused or not.

use crate::post::{PostBuf, PostOp};

/// `StgGlare`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Glare {
    enabled: bool,
    track: bool,
    level: i32,
    state: u32,
    max: i32,
    hold: i32,
    min: i32,
    step: i32,
    alpha: u8,
}

impl Default for Glare {
    /// `StgGlare_Init`: all zero, then `StgGlare_Reset`.
    fn default() -> Self {
        Self {
            enabled: false,
            track: false,
            level: 0,
            state: 0,
            max: 0,
            hold: 0,
            min: 0,
            step: 0,
            alpha: 0,
        }
    }
}

impl Glare {
    /// `StgFx_Reset`'s `StgGlare_Reset` (a glare that is off starts its level at its step) then
    /// `StgGlare_LoadParams` of the stage's record (off without one: alpha 0x40, step 0x20, max 0x80, hold 0x50,
    /// min 4).
    pub(crate) fn load(&mut self, param: Option<&[u8; 12]>) {
        if !self.enabled {
            self.state = 0;
            self.level = self.step;
        }
        match param {
            None => {
                self.alpha = 0x40;
                self.step = 0x20;
                self.max = 0x80;
                self.hold = 0x50;
                self.min = 4;
                self.track = false;
                self.enabled = false;
            }
            Some(p) => {
                let flags = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                self.enabled = flags & 1 != 0;
                self.track = flags & 2 != 0;
                self.step = i32::from_le_bytes([p[4], p[5], p[6], p[7]]);
                self.max = i32::from(p[8]);
                self.hold = i32::from(p[9]);
                self.min = i32::from(p[10]);
                self.alpha = p[11];
            }
        }
    }

    /// The level (for tests).
    #[cfg(test)]
    pub(crate) fn level(&self) -> i32 {
        self.level
    }

    /// `StgGlare_Update`: `light_on_view` whether the stage light's point projects in front of the camera and inside
    /// the view.
    pub(crate) fn update(&mut self, light_on_view: bool) {
        if !self.enabled {
            return;
        }
        if self.track {
            if light_on_view {
                if self.state < 2 {
                    self.state = 1;
                    self.level += self.step;
                    if self.level > self.max {
                        self.level = self.max;
                        self.state = 2;
                    }
                } else {
                    self.level -= self.step;
                    if self.level < self.hold {
                        self.level = self.hold;
                    }
                }
            } else {
                self.level -= 1;
                self.state = 0;
                if self.level < self.min {
                    self.level = self.min;
                }
            }
        } else {
            self.level = self.max;
        }
    }

    /// `StgGlare_Draw`: the glow pass when the glare is on.
    pub(crate) fn draw(&self) -> Vec<PostOp> {
        if !self.enabled {
            return Vec::new();
        }
        let alpha = self.alpha;
        let mask: Box<[u8; 256]> =
            Box::new(std::array::from_fn(
                |i| if i == 0xFF { 0x80 } else { alpha },
            ));
        glow_ops(mask, false, self.level.clamp(0, 255) as u8)
    }
}

/// `GfxPost_DrawGlow(10, 0x80808080, glow)`: the frame weighed by the frame's alpha, which `mask` gives by the
/// depth buffer's spare byte (the stage's depth, or with `ids` the objects' ids), halved into a work buffer over
/// black, shrunk, blurred back and forth, grown back tinted by `level` (`glow`'s grey) and added over the frame.
pub(crate) fn glow_ops(mask: Box<[u8; 256]>, ids: bool, level: u8) -> Vec<PostOp> {
    {
        let sprite = |s, t, xy, uv, rgba| PostOp::sprite(s, t, false, xy, uv, rgba);
        // The corner buffers read only their own texels (CLAMP_1 REGION_CLAMP).
        let within = |mut op: PostOp, w: u16, h: u16| {
            op.region = Some([0, 0, w, h]);
            op
        };
        let (w0, w1) = (PostBuf::Work(0), PostBuf::Work(1));
        let mut ops = Vec::new();
        // The half-size buffer cleared to black, then the frame halved over it weighed by the glare's CLUT.
        ops.push(sprite(
            PostBuf::Frame,
            w0,
            [0.0, 0.0, 256.0, 224.0],
            [0.0, 0.0, 512.0, 448.0],
            [0, 0, 0, 0x80],
        ));
        let mut weighed = PostOp::sprite(
            PostBuf::Frame,
            w0,
            true,
            [0.0, 0.0, 256.0, 224.0],
            [0.0, 0.0, 512.0, 448.0],
            [0x80; 4],
        );
        weighed.mask = Some(mask);
        weighed.ids = ids;
        ops.push(weighed);
        // To a quarter, then an eighth one texel on.
        ops.push(sprite(
            w0,
            w1,
            [0.0, 0.0, 128.0, 112.0],
            [0.0, 0.0, 256.0, 224.0],
            [0x80; 4],
        ));
        ops.push(within(
            sprite(
                w1,
                w0,
                [0.0, 0.0, 64.0, 56.0],
                [1.0, 1.0, 129.0, 113.0],
                [0x80; 4],
            ),
            128,
            112,
        ));
        // The passes between the eighths: the GS samples at the pixel's corner, so the pass without an offset
        // filters half a texel back.
        for i in 0..10 {
            let (s, t, o) = if i & 1 == 0 {
                (w0, w1, 0.5)
            } else {
                (w1, w0, -0.5)
            };
            ops.push(within(
                sprite(
                    s,
                    t,
                    [0.0, 0.0, 64.0, 56.0],
                    [o, o, 64.0 + o, 56.0 + o],
                    [0x80; 4],
                ),
                64,
                56,
            ));
        }
        // Back up, tinted by the level, and added over the frame.
        let l = level;
        ops.push(within(
            sprite(
                w0,
                w1,
                [0.0, 0.0, 128.0, 112.0],
                [0.0, 0.0, 64.0, 56.0],
                [l, l, l, 0x80],
            ),
            64,
            56,
        ));
        ops.push(within(
            sprite(
                w1,
                w0,
                [0.0, 0.0, 256.0, 224.0],
                [0.5, 0.5, 128.5, 112.5],
                [0x80; 4],
            ),
            128,
            112,
        ));
        let mut add = sprite(
            w0,
            PostBuf::Frame,
            [0.0, 0.0, 512.0, 448.0],
            [0.5, 0.5, 256.5, 224.5],
            [0x80; 4],
        );
        add.add = true;
        ops.push(add);
        ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glare_tracks_the_light_and_adds_its_glow() {
        // Stage 0's record: on, tracking, step 3, max 0x78, hold 0x41, min 0xA, alpha 0x5A.
        let p = [3, 0, 0, 0, 3, 0, 0, 0, 0x78, 0x41, 0x0A, 0x5A];
        let mut g = Glare::default();
        g.load(Some(&p));
        assert_eq!(g.level(), 0);
        // Rising by 3 while the light is on the view, to 0x78, then down to the hold level.
        for _ in 0..39 {
            g.update(true);
        }
        assert_eq!(g.level(), 117);
        g.update(true);
        assert_eq!(g.level(), 0x78);
        for _ in 0..20 {
            g.update(true);
        }
        assert_eq!(g.level(), 0x41);
        // Off the view: down by one to the minimum.
        for _ in 0..100 {
            g.update(false);
        }
        assert_eq!(g.level(), 0x0A);
        let ops = g.draw();
        assert_eq!(ops.len(), 17);
        let mask = ops[1].mask.as_ref().unwrap();
        assert_eq!((mask[0], mask[0xFE], mask[0xFF]), (0x5A, 0x5A, 0x80));
        assert!(ops[16].add && ops[16].target == PostBuf::Frame);
        assert_eq!(ops[14].tris[0][0].rgba, [0x0A, 0x0A, 0x0A, 0x80]);
        // Without tracking it sits at the maximum; without a record it is off.
        let mut h = Glare::default();
        h.load(Some(&[1, 0, 0, 0, 3, 0, 0, 0, 0x50, 0, 0, 0x40]));
        h.update(false);
        assert_eq!(h.level(), 0x50);
        let mut off = Glare::default();
        off.load(None);
        assert!(off.draw().is_empty());
    }
}
