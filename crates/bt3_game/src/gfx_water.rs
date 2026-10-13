//! The underwater wobble (`GfxWater_*`, src/sys/gfx_screen.c): while a view's camera is below the stage's water
//! level (on a stage whose first water record has bit 0), the view is copied at half height into the work page
//! (frame block 0x150, 512 wide) and drawn back over the screen, opaque and tinted with the record's colour, as five
//! strips of a grid whose texels two cosine waves push around two centres (a fifth and four fifths across). Each
//! view's two wave phases advance while it draws, unless the battle is paused. It runs first in `StgFx_DrawPost`,
//! before the radial blur.

use crate::post::{PostBuf, PostOp, PostVertex, rgba};

/// `GfxWaterLayer`: one view's wave speeds and phases and its tint.
#[derive(Clone, Copy, Debug)]
struct Layer {
    speed_u: f32,
    speed_v: f32,
    phase_u: f32,
    phase_v: f32,
    rgba: u32,
}

/// The two views' layers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wobble {
    layers: [Layer; 2],
}

/// `GfxWater_SetColor`'s packing.
fn pack(c: [u8; 4]) -> u32 {
    u32::from_le_bytes(c)
}

impl Default for Wobble {
    /// `GfxWater_Init` / `GfxWater_Reset`: speeds 0.06 and 0.04, an opaque bluish grey.
    fn default() -> Self {
        let layer = Layer {
            speed_u: 0.06,
            speed_v: 0.04,
            phase_u: 0.0,
            phase_v: 0.0,
            rgba: pack([0x5C, 0x5C, 0x80, 0x80]),
        };
        Self { layers: [layer; 2] }
    }
}

/// Wraps a phase past pi back by 2 pi.
fn advance(phase: f32, speed: f32) -> f32 {
    let p = phase + speed;
    if p > 3.141_592_7 { p - 6.283_185_3 } else { p }
}

impl Wobble {
    /// `GfxWater_LoadStageColor` (`StgFx_Reset`): both layers take the stage's water colour when it has one.
    pub(crate) fn load_stage_color(&mut self, color: Option<[u8; 4]>) {
        if let Some(c) = color {
            for l in &mut self.layers {
                l.rgba = pack(c);
            }
        }
    }

    /// The view's phases (for tests).
    #[cfg(test)]
    pub(crate) fn phases(&self, view: usize) -> [f32; 2] {
        [self.layers[view].phase_u, self.layers[view].phase_v]
    }

    /// `GfxWater_DrawView`: view `view`'s copy and wobbled redraw (`split`: its half of the screen, five columns of
    /// the grid).
    pub(crate) fn draw(&mut self, view: usize, split: bool, paused: bool) -> Vec<PostOp> {
        let l = &mut self.layers[view];
        if !paused {
            l.phase_u = advance(l.phase_u, l.speed_u);
            l.phase_v = advance(l.phase_v, l.speed_v);
        }
        let (h, src_h) = (224i32, 448i32);
        // The view's columns (the redraw's scissor is its half, which the strips stay inside).
        let (src_w, w, cols, x0) = if split {
            (256i32, 256i32, 5usize, if view == 0 { 0 } else { 256 })
        } else {
            (512, 512, 10, 0)
        };
        // The grid's texels (1/16 texel), pushed by the two waves (all but the first row), clamped to the copy.
        let mut gu = [[0i32; 10]; 6];
        let mut gv = [[0i32; 10]; 6];
        let (fw, fh) = (w as f32, h as f32);
        for row in 0..6 {
            for col in 0..cols {
                let cx = col as i32 * w / (cols as i32 - 1);
                let cy = h * row as i32 / 5;
                let (mut u, mut v) = (cx << 4, cy << 4);
                if row != 0 {
                    let (fx, fy) = (cx as f32, cy as f32);
                    for (phase, ox, oy) in [
                        (l.phase_u, fw * 0.2, 224.0 * 0.2),
                        (l.phase_v, fw * 0.8, 0.8 * fh),
                    ] {
                        let (dx, dy) = (fx - ox, fy - oy);
                        let dist = (dx * dx + dy * dy).sqrt();
                        let c = (phase + dist * 0.2).cos();
                        u += (c * (dx / dist) * 3.0 * 16.0) as i32;
                        v += (c * (dy / dist) * 3.0 * 16.0) as i32;
                    }
                }
                gu[row][col] = u.clamp(0, w << 4);
                gv[row][col] = v.clamp(0, h << 4);
            }
        }
        // The copy: strips 32 texels wide of the view, at half height, into the work page.
        let mut ops = Vec::new();
        let n = src_w / 32;
        let step = w / n;
        let mut copy = PostOp {
            source: PostBuf::Frame,
            target: PostBuf::Wide,
            blend: false,
            mask: None,
            ids: false,
            add: false,
            region: None,
            tris: Vec::new(),
        };
        for k in 0..n {
            let c = PostOp::sprite(
                PostBuf::Frame,
                PostBuf::Wide,
                false,
                [(k * step) as f32, 0.0, ((k + 1) * step) as f32, h as f32],
                [
                    (x0 + k * 32) as f32,
                    0.0,
                    (x0 + (k + 1) * 32) as f32,
                    src_h as f32,
                ],
                [0x80; 4],
            );
            copy.tris.extend(c.tris);
        }
        ops.push(copy);
        // The redraw: five strips, opaque, tinted.
        let tint = rgba(l.rgba);
        let mut back = PostOp {
            source: PostBuf::Wide,
            target: PostBuf::Frame,
            blend: false,
            mask: None,
            ids: false,
            add: false,
            region: None,
            tris: Vec::new(),
        };
        for row in 0..5 {
            let mut verts = Vec::with_capacity(cols * 2);
            for col in 0..cols {
                let sx = (x0 + col as i32 * src_w / (cols as i32 - 1)) as f32;
                for r in [row, row + 1] {
                    verts.push(PostVertex {
                        xy: [sx, (src_h * r as i32 / 5) as f32],
                        uv: [gu[r][col] as f32 / 16.0, gv[r][col] as f32 / 16.0],
                        rgba: tint,
                    });
                }
            }
            back.strip(&verts);
        }
        ops.push(back);
        ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wobble_copies_at_half_height_and_draws_five_tinted_strips() {
        let mut w = Wobble::default();
        w.load_stage_color(Some([1, 2, 3, 4]));
        let ops = w.draw(0, false, false);
        assert_eq!(ops.len(), 2);
        // Sixteen 32-texel strips, each two triangles, from the frame's 448 rows to 224.
        assert_eq!(
            (ops[0].source, ops[0].target),
            (PostBuf::Frame, PostBuf::Wide)
        );
        assert_eq!(ops[0].tris.len(), 32);
        assert_eq!(ops[0].tris[0][2].xy, [0.0, 224.0]);
        assert_eq!(ops[0].tris[0][2].uv, [0.0, 448.0]);
        // Five strips of ten columns: 18 triangles each, the stage colour, the first row unmoved.
        assert_eq!(
            (ops[1].source, ops[1].target),
            (PostBuf::Wide, PostBuf::Frame)
        );
        assert_eq!(ops[1].tris.len(), 5 * 18);
        assert!(ops[1].tris.iter().flatten().all(|v| v.rgba == [1, 2, 3, 4]));
        assert_eq!(ops[1].tris[0][0].uv, [0.0, 0.0]);
        assert_eq!(ops[1].tris[0][0].xy, [0.0, 0.0]);
        assert_eq!(ops[1].tris[0][1].xy, [0.0, 89.0]);
        // The phases step unless paused, and wrap past pi.
        assert_eq!(w.phases(0), [0.06, 0.04]);
        w.draw(0, false, true);
        assert_eq!(w.phases(0), [0.06, 0.04]);
        for _ in 0..60 {
            w.draw(0, false, false);
        }
        assert!(w.phases(0)[0] < 0.0 && w.phases(0)[1] > 0.0);
        assert_eq!(w.phases(1), [0.0, 0.0]);
        // Split screen: the right view's half, five columns.
        let ops = w.draw(1, true, false);
        assert_eq!(ops[0].tris.len(), 16);
        assert_eq!(ops[0].tris[0][0].uv, [256.0, 0.0]);
        assert_eq!(ops[1].tris.len(), 5 * 8);
        assert_eq!(ops[1].tris[0][0].xy, [256.0, 0.0]);
    }
}
