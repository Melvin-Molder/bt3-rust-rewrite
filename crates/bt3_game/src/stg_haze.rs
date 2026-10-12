//! The depth haze (`StgHaze_*`, src/battle/screen_fx.c), on stages whose haze record (section +0xA0) has bit 0
//! (feature 9; stages 3 and 19). Per view, in `StgFx_DrawPre` after the stage: the depth byte goes through the
//! haze's CLUT into the frame's alpha (black, alpha curve * 128 * strength, the strength 1 at the ground falling to
//! a half at the stage's top by the camera's height), the view is copied at half height into the work page, and
//! drawn back over itself through a grid of rows - 1 strips whose texels jitter, blended by that alpha. The jitter
//! follows random targets drawn with libc `rand()` (`Rand_IntRange`, two per mesh point every `period` frames, not
//! while paused), stored in bytes, so the draw takes from the shared libc stream.
//!
//! As in the original, the stage's parameters give view 1 no curve (`StgHaze_SetParams` writes view 0's keys on
//! each pass), and a strip's lower vertex takes the offset of the next column of its own row, the last column
//! reading on into the next row (and after the last row into the y offsets, then the grid's column count).

use crate::post::{PostBuf, PostOp, PostVertex, rgba};
use crate::rng::LibcRandom;
use crate::stg_depth::Curve;

const COLS: usize = 20;
const ROWS: usize = 50;

/// One view's haze (`StgHaze`).
#[derive(Clone, Debug)]
struct View {
    cols: i32,
    rows: i32,
    amp: [i32; 2],
    color: u32,
    period: i32,
    frame: i32,
    follow: f32,
    curve: Curve,
    target: [[[i8; COLS]; ROWS]; 2],
    /// `curX` then `curY`, as they lie in memory.
    cur: Vec<i8>,
}

impl View {
    /// `StgHaze_Reset`: a 20 x 50 grid, +-128 sixteenths, grey, new targets every frame, followed by a tenth.
    fn new() -> Self {
        Self {
            cols: 20,
            rows: 50,
            amp: [0x80, 0x80],
            color: 0x8080_8080,
            period: 1,
            frame: 0,
            follow: 0.1,
            curve: Curve {
                keys: [[0.0; 2]; 3],
            },
            target: [[[0; COLS]; ROWS]; 2],
            cur: vec![0; COLS * ROWS * 2],
        }
    }

    /// The current offset `k` places after `[axis][row][col]` in memory (past the y offsets: the column count).
    fn cur_at(&self, axis: usize, row: usize, col: usize, k: usize) -> i32 {
        let i = axis * COLS * ROWS + row * COLS + col + k;
        match self.cur.get(i) {
            Some(&v) => i32::from(v),
            None => i32::from(self.cols.to_le_bytes()[(i - self.cur.len()).min(3)] as i8),
        }
    }

    /// `StgHaze_Step`: new targets every `period` frames, then each offset moves toward its target.
    fn step(&mut self, split: bool, libc: &mut LibcRandom) {
        let cols = (if split { self.cols / 2 } else { self.cols }).clamp(0, COLS as i32) as usize;
        let rows = self.rows.clamp(0, ROWS as i32) as usize;
        // A period of 0 (no stage has one) would divide by zero: no new targets.
        if self.frame.checked_rem(self.period) == Some(0) {
            for i in 0..rows {
                for j in 0..cols {
                    self.target[0][i][j] = libc.int_range(-self.amp[0], self.amp[0]) as i8;
                    self.target[1][i][j] = libc.int_range(-self.amp[1], self.amp[1]) as i8;
                }
            }
        }
        for i in 0..rows {
            for j in 0..cols {
                for axis in 0..2 {
                    let c = &mut self.cur[axis * COLS * ROWS + i * COLS + j];
                    let t = i32::from(self.target[axis][i][j]);
                    let d = ((t - i32::from(*c)) as f32 * self.follow) as i32;
                    *c = (i32::from(*c) + d) as i8;
                }
            }
        }
        self.frame += 1;
    }

    /// `StgHaze_BuildClut` at `StgHaze_UpdateClut`'s strength: the alpha of each byte.
    fn mask(&self, cam_y: f32, top: f32) -> Box<[u8; 256]> {
        let scale = 1.0 - -cam_y / -top * 0.5;
        let table = self.curve.build();
        Box::new(std::array::from_fn(|i| {
            (table[i] * 128.0 * scale) as u32 as u8
        }))
    }

    /// `StgHaze_Draw`: the copy of the view into the work page and the jittered strips back, masked.
    fn draw(&self, split: bool, view: usize, mask: Box<[u8; 256]>) -> Vec<PostOp> {
        let (h, src_h) = (224i32, 448i32);
        let (width, width_px, x_ofs, cols) = if split {
            (
                256i32,
                256i32,
                if view == 1 { 256 } else { 0 },
                self.cols / 2,
            )
        } else {
            (512, 512, 0, self.cols)
        };
        let rows = self.rows;
        // StgHaze_BeginDraw: the view at half height into the work page.
        let copy = PostOp::sprite(
            PostBuf::Frame,
            PostBuf::Wide,
            false,
            [0.0, 0.0, width_px as f32, h as f32],
            [x_ofs as f32, 0.0, (x_ofs + width) as f32, src_h as f32],
            [0x80; 4],
        );
        let mut back = PostOp {
            source: PostBuf::Wide,
            target: PostBuf::Frame,
            blend: true,
            mask: Some(mask),
            tris: Vec::new(),
        };
        if cols < 2 || rows < 2 {
            return vec![copy];
        }
        let color = rgba(self.color);
        let clamp = |v: i32, max: i32| v.clamp(0, max);
        for row in 0..rows - 1 {
            let y0 = (h * row / (rows - 1)) << 4;
            let v0 = src_h * row / (rows - 1);
            let y1 = (h * (row + 1) / (rows - 1)) << 4;
            let v1 = src_h * (row + 1) / (rows - 1);
            let mut verts = Vec::with_capacity(cols as usize * 2);
            for col in 0..cols {
                let u = col * width / (cols - 1) + x_ofs;
                let px = (col * width_px / (cols - 1)) << 4;
                let (r, c) = (row as usize, col as usize);
                for (k, y, v) in [(0, y0, v0), (1, y1, v1)] {
                    let x = clamp(px + self.cur_at(0, r, c, k), width_px << 4);
                    let y = clamp(y + self.cur_at(1, r, c, k), h << 4);
                    verts.push(PostVertex {
                        xy: [u as f32, v as f32],
                        uv: [(x + 8) as f32 / 16.0, (y + 8) as f32 / 16.0],
                        rgba: color,
                    });
                }
            }
            back.strip(&verts);
        }
        vec![copy, back]
    }
}

/// `gStgHaze`: both views.
#[derive(Clone, Debug)]
pub(crate) struct Haze {
    views: [View; 2],
    on: bool,
}

impl Haze {
    /// `StgHaze_Reset` and `StgHaze_SetParams` of the stage's record (`StgFx_Reset`); on with its bit 0.
    pub(crate) fn new(param: Option<&[u8; 0x18]>) -> Self {
        let mut views = [View::new(), View::new()];
        match param {
            None => {
                for v in &mut views {
                    v.curve = Curve {
                        keys: [[0.0, 128.0], [200.0, 128.0], [255.0, 255.0]],
                    };
                }
            }
            Some(p) => {
                for v in &mut views {
                    v.cols = i32::from(p[0xE]);
                    v.rows = i32::from(p[0xF]);
                    v.amp = [i32::from(p[0x10]), i32::from(p[0x11])];
                    v.color = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
                    v.period = i32::from(p[0x12]);
                    v.follow = f32::from_le_bytes([p[0x14], p[0x15], p[0x16], p[0x17]]);
                }
                views[0].curve = Curve {
                    keys: [
                        [f32::from(p[8]), f32::from(p[9])],
                        [f32::from(p[10]), f32::from(p[11])],
                        [f32::from(p[12]), f32::from(p[13])],
                    ],
                };
            }
        }
        Self {
            on: param.is_some_and(|p| p[0] & 1 != 0),
            views,
        }
    }

    /// Whether the stage turns the haze on (`BtlStage_HasFeature(9)`).
    pub(crate) fn is_on(&self) -> bool {
        self.on
    }

    /// `StgHaze_DrawView`: the view's CLUT for the camera height `cam_y` under the stage's `top`, the jitter (not
    /// while paused), the draw.
    pub(crate) fn draw_view(
        &mut self,
        split: bool,
        view: usize,
        cam_y: f32,
        top: f32,
        paused: bool,
        libc: &mut LibcRandom,
    ) -> Vec<PostOp> {
        let v = &mut self.views[view];
        let mask = v.mask(cam_y, top);
        if !paused {
            v.step(split, libc);
        }
        v.draw(split, view, mask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param() -> [u8; 0x18] {
        // Stage 3's record: on, grey, keys (0, 0x14) (0xF9, 0x78) (0xFF, 0x14), 16 x 40, +-200, every frame, 0.1.
        let mut p = [0u8; 0x18];
        p[0] = 1;
        p[4..8].copy_from_slice(&[0x80; 4]);
        p[8..14].copy_from_slice(&[0, 0x14, 0xF9, 0x78, 0xFF, 0x14]);
        p[0xE] = 0x10;
        p[0xF] = 0x28;
        p[0x10] = 0xC8;
        p[0x11] = 0xC8;
        p[0x12] = 1;
        p[0x14..].copy_from_slice(&0.1f32.to_le_bytes());
        p
    }

    #[test]
    fn the_haze_draws_two_numbers_a_point_and_redraws_the_view() {
        let mut h = Haze::new(Some(&param()));
        assert!(h.is_on());
        let mut libc = LibcRandom::new(1);
        let mut check = LibcRandom::new(1);
        let ops = h.draw_view(false, 0, -10.0, -500.0, false, &mut libc);
        for _ in 0..16 * 40 * 2 {
            check.rand();
        }
        assert_eq!(libc.rand(), check.rand());
        // The copy at half height, then 39 strips of 32 vertices (30 triangles each), masked.
        assert_eq!(ops.len(), 2);
        assert_eq!(
            (ops[0].source, ops[0].target),
            (PostBuf::Frame, PostBuf::Wide)
        );
        assert_eq!(ops[1].tris.len(), 39 * 30);
        let mask = ops[1].mask.as_ref().unwrap();
        // The curve's first key 0x14 at byte 0, at the strength 1 - (10 / 500) / 2.
        assert_eq!(mask[0], (0x14 as f32 / 255.0 * 128.0 * 0.99) as u8);
        // Offsets of +-200 wrapped into bytes, a tenth of them followed.
        let first = ops[1].tris[0][0];
        assert_eq!(first.xy, [0.0, 0.0]);
        assert!(first.uv[0] >= 0.5 && first.uv[1] >= 0.5);
        // Paused: no numbers drawn. View 1 has no curve.
        let (mut a, mut b) = (LibcRandom::new(5), LibcRandom::new(5));
        h.draw_view(false, 0, -10.0, -500.0, true, &mut a);
        assert_eq!(a.rand(), b.rand());
        let ops = h.draw_view(true, 1, -10.0, -500.0, true, &mut libc);
        assert!(ops[1].mask.as_ref().unwrap().iter().all(|&a| a == 0));
        assert_eq!(ops[1].tris[0][0].xy, [256.0, 0.0]);
    }
}
