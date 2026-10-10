//! The GS pixel pipeline as the HUD's gauges and timer use it (src/battle/hud_sprite.c and the parts' GS state
//! packets), run on the CPU into a 512 x 448 layer. Their sprites are axis-aligned rectangles on whole pixels, and
//! their masks live in the frame's alpha plane (DATE, alpha tests that keep the RGB or nothing, FBA, write masks),
//! so the plane is kept here exactly. The colour of each pixel is kept as P + K * (what is behind the HUD): every
//! blend the HUD uses (ALPHA 0x44, 0x48, 0x58) maps that form onto itself, and the layer is drawn over the 3D
//! frame with one blend.

use bt3_data::gs::RgbaTexture;

pub(crate) const WIDTH: usize = 512;
pub(crate) const HEIGHT: usize = 448;

/// One GS context's registers that the HUD changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Context {
    pub(crate) alpha: u32,
    pub(crate) test: u32,
    pub(crate) fba: bool,
    pub(crate) fbmsk: u32,
}

/// `HudGfx_SetEnv`: source alpha blending, tests that always pass, no FBA, no write mask.
pub(crate) const ENV: Context = Context {
    alpha: 0x44,
    test: 0x30000,
    fba: false,
    fbmsk: 0,
};

/// A sprite as `HudSprite_DrawAt` sends it: the four corners' screen x (left / right vertex) and y (top / bottom
/// vertex), their UV in texels (with the + 0.5 of `texel * 16 + 8`), the vertex colour, and the texture.
pub(crate) struct GsSprite<'a> {
    pub(crate) x: [i32; 2],
    pub(crate) y: [i32; 2],
    pub(crate) u: [f32; 2],
    pub(crate) v: [f32; 2],
    pub(crate) rgba: [u8; 4],
    pub(crate) texture: Option<&'a RgbaTexture>,
}

/// The layer: colour as P + K * background (K kept as 1 - K, so a clear layer is all zeros), the alpha plane,
/// and the box colour was written in.
pub(crate) struct GsLayer {
    p: Vec<[f32; 3]>,
    cover: Vec<f32>,
    alpha: Vec<u8>,
    dirty: Option<[usize; 4]>,
    /// The frame is not shown: nothing is drawn (the parts' updates still run).
    skip: bool,
    pub(crate) ctx: [Context; 2],
}

impl GsLayer {
    pub(crate) fn new() -> Self {
        Self {
            p: vec![[0.0; 3]; WIDTH * HEIGHT],
            cover: vec![0.0; WIDTH * HEIGHT],
            alpha: vec![0; WIDTH * HEIGHT],
            dirty: None,
            skip: false,
            ctx: [ENV; 2],
        }
    }

    /// A new frame: what was drawn is cleared, the state is `HudGfx_SetEnv`'s.
    pub(crate) fn begin(&mut self, shown: bool) {
        if let Some([x0, x1, y0, y1]) = self.dirty.take() {
            for y in y0..y1 {
                self.p[y * WIDTH + x0..y * WIDTH + x1].fill([0.0; 3]);
                self.cover[y * WIDTH + x0..y * WIDTH + x1].fill(0.0);
            }
        }
        self.alpha.fill(0);
        self.skip = !shown;
        self.ctx = [ENV; 2];
    }

    /// Whether anything was drawn in colour.
    pub(crate) fn is_empty(&self) -> bool {
        self.dirty.is_none()
    }

    /// RGBA8 for the compositor, premultiplied: P in the colour, 1 - K in the alpha.
    pub(crate) fn to_rgba(&self) -> Vec<u8> {
        let mut out = vec![0u8; WIDTH * HEIGHT * 4];
        if let Some([x0, x1, y0, y1]) = self.dirty {
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = y * WIDTH + x;
                    let p = self.p[i];
                    out[i * 4] = p[0] as u8;
                    out[i * 4 + 1] = p[1] as u8;
                    out[i * 4 + 2] = p[2] as u8;
                    out[i * 4 + 3] = (self.cover[i].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
        out
    }

    /// A rectangle in colour 0, alpha 0 through context 1. Under ALPHA 0x44 with no tests that only writes the
    /// alpha plane.
    fn clear_rect(&mut self, x: [i32; 2], y: [i32; 2]) {
        if self.skip {
            return;
        }
        let c = self.ctx[0];
        if c.alpha != 0x44 || c.test & 0x4001 != 0 || c.fbmsk >> 24 == 0xFF {
            self.draw(
                &GsSprite {
                    x,
                    y,
                    u: [0.0; 2],
                    v: [0.0; 2],
                    rgba: [0; 4],
                    texture: None,
                },
                0,
            );
            return;
        }
        let value = if c.fba { 0x80 } else { 0 };
        let span = |a: i32, b: i32, max: usize| {
            (
                a.min(b).clamp(0, max as i32) as usize,
                a.max(b).clamp(0, max as i32) as usize,
            )
        };
        let ((x0, x1), (y0, y1)) = (span(x[0], x[1], WIDTH), span(y[0], y[1], HEIGHT));
        for row in y0..y1 {
            self.alpha[row * WIDTH + x0..row * WIDTH + x1].fill(value);
        }
    }

    /// `HudGfx_ClearAlpha`: 16 strips over the frame in colour 0, alpha 0, through context 1.
    pub(crate) fn clear_alpha(&mut self) {
        self.clear_rect([0, WIDTH as i32], [0, HEIGHT as i32]);
    }

    /// `HudSprite_DrawAlphaClear`: ALPHA_1 0x44, TEST_1 0x30000, FBA_1 off, then the rectangle in colour 0,
    /// alpha 0 through context 1.
    pub(crate) fn alpha_clear(&mut self, x: [i32; 2], y: [i32; 2]) {
        self.ctx[0].alpha = 0x44;
        self.ctx[0].test = 0x30000;
        self.ctx[0].fba = false;
        self.clear_rect(x, y);
    }

    /// One sprite through context `ctx` (0 or 1): DATE, the texture function (MODULATE, TCC), the alpha test,
    /// the blend, the colour clamp, FBA and the write mask, per covered pixel.
    pub(crate) fn draw(&mut self, spr: &GsSprite, ctx: usize) {
        if self.skip {
            return;
        }
        let c = self.ctx[ctx];
        let (x_lo, x_hi) = (spr.x[0].min(spr.x[1]), spr.x[0].max(spr.x[1]));
        let (y_lo, y_hi) = (spr.y[0].min(spr.y[1]), spr.y[0].max(spr.y[1]));
        let (x_lo, x_hi) = (x_lo.max(0), x_hi.min(WIDTH as i32));
        let (y_lo, y_hi) = (y_lo.max(0), y_hi.min(HEIGHT as i32));
        if x_lo >= x_hi || y_lo >= y_hi {
            return;
        }
        let ate = c.test & 1 != 0;
        let atst = (c.test >> 1) & 7;
        let aref = (c.test >> 4) & 0xFF;
        let afail = (c.test >> 12) & 3;
        let date = c.test & 0x4000 != 0;
        let datm = c.test & 0x8000 != 0;
        let [sel_a, sel_b, sel_c, sel_d] = [0, 2, 4, 6].map(|s| (c.alpha >> s) & 3);
        let rgb_masked = c.fbmsk & 0xFF_FFFF == 0xFF_FFFF;
        let alpha_masked = c.fbmsk >> 24 == 0xFF;
        let du = if spr.x[1] != spr.x[0] {
            (spr.u[1] - spr.u[0]) / (spr.x[1] - spr.x[0]) as f32
        } else {
            0.0
        };
        let dv = if spr.y[1] != spr.y[0] {
            (spr.v[1] - spr.v[0]) / (spr.y[1] - spr.y[0]) as f32
        } else {
            0.0
        };
        // (A - B) * C + D as s * Cs + d * Cd, before C.
        let unit = |sel: u32, of: u32| if sel == of { 1.0f32 } else { 0.0 };
        let (s_ab, s_d) = (unit(sel_a, 0) - unit(sel_b, 0), unit(sel_d, 0));
        let (d_ab, d_d) = (unit(sel_a, 1) - unit(sel_b, 1), unit(sel_d, 1));
        let fba = if c.fba { 0x80 } else { 0 };
        // At one texel per pixel, on texel centres, bilinear filtering is a plain fetch.
        let centred = |t: f32| (t - 0.5).fract() == 0.0;
        let direct = du.abs() == 1.0 && dv.abs() == 1.0 && centred(spr.u[0]) && centred(spr.v[0]);
        let cv = spr.rgba.map(f32::from);
        if !rgb_masked {
            let (x0, x1, y0, y1) = (x_lo as usize, x_hi as usize, y_lo as usize, y_hi as usize);
            let d = self.dirty.get_or_insert([x0, x1, y0, y1]);
            *d = [d[0].min(x0), d[1].max(x1), d[2].min(y0), d[3].max(y1)];
        }
        for y in y_lo..y_hi {
            let v = spr.v[0] + (y - spr.y[0]) as f32 * dv;
            for x in x_lo..x_hi {
                let i = y as usize * WIDTH + x as usize;
                let dst_alpha = self.alpha[i];
                if date && ((dst_alpha & 0x80 != 0) != datm) {
                    continue;
                }
                let (cs, a_s) = match spr.texture {
                    Some(tex) => {
                        let u = spr.u[0] + (x - spr.x[0]) as f32 * du;
                        let t = if direct {
                            fetch(tex, (u - 0.5).floor() as i32, (v - 0.5).floor() as i32)
                        } else {
                            sample(tex, u, v)
                        };
                        (
                            [
                                (t[0] * cv[0] / 128.0).floor().min(255.0),
                                (t[1] * cv[1] / 128.0).floor().min(255.0),
                                (t[2] * cv[2] / 128.0).floor().min(255.0),
                            ],
                            (t[3] * cv[3] / 128.0).floor().min(255.0),
                        )
                    }
                    None => ([cv[0], cv[1], cv[2]], cv[3]),
                };
                let as_int = a_s as u32;
                let mut write_alpha = !alpha_masked;
                if ate && !alpha_test(atst, as_int, aref) {
                    match afail {
                        // FB_ONLY
                        1 => {}
                        // RGB_ONLY
                        3 => write_alpha = false,
                        _ => continue,
                    }
                }
                if !rgb_masked {
                    let factor = match sel_c {
                        0 => a_s,
                        1 => f32::from(dst_alpha),
                        _ => 0.0,
                    } / 128.0;
                    let (s, d) = (s_ab * factor + s_d, d_ab * factor + d_d);
                    let p = &mut self.p[i];
                    for ch in 0..3 {
                        p[ch] = (s * cs[ch] + d * p[ch]).clamp(0.0, 255.0);
                    }
                    self.cover[i] = 1.0 - d * (1.0 - self.cover[i]);
                }
                if write_alpha {
                    self.alpha[i] = as_int.min(255) as u8 | fba;
                }
            }
        }
    }
}

fn fetch(tex: &RgbaTexture, x: i32, y: i32) -> [f32; 4] {
    let (w, h) = (tex.width as i32, tex.height as i32);
    let o = (y.rem_euclid(h) * w + x.rem_euclid(w)) as usize * 4;
    let p = &tex.pixels[o..o + 4];
    [
        f32::from(p[0]),
        f32::from(p[1]),
        f32::from(p[2]),
        f32::from(p[3]),
    ]
}

fn alpha_test(atst: u32, a: u32, aref: u32) -> bool {
    match atst {
        0 => false,
        1 => true,
        2 => a < aref,
        3 => a <= aref,
        4 => a == aref,
        5 => a >= aref,
        6 => a > aref,
        _ => a != aref,
    }
}

/// TEX1 0x60 (bilinear) with CLAMP 0 (repeat), at texel position (u, v) - 0.5.
fn sample(tex: &RgbaTexture, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (tex.width as i32, tex.height as i32);
    let (fu, fv) = (u - 0.5, v - 0.5);
    let (x0, y0) = (fu.floor(), fv.floor());
    let (ax, ay) = (fu - x0, fv - y0);
    let texel = |x: i32, y: i32| {
        let o = (y.rem_euclid(h) * w + x.rem_euclid(w)) as usize * 4;
        [0, 1, 2, 3].map(|k| f32::from(tex.pixels[o + k]))
    };
    let (x0, y0) = (x0 as i32, y0 as i32);
    let [t00, t10, t01, t11] = [
        texel(x0, y0),
        texel(x0 + 1, y0),
        texel(x0, y0 + 1),
        texel(x0 + 1, y0 + 1),
    ];
    std::array::from_fn(|k| {
        let top = t00[k] + (t10[k] - t00[k]) * ax;
        let bottom = t01[k] + (t11[k] - t01[k]) * ax;
        top + (bottom - top) * ay
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(pixels: &[[u8; 4]], width: u32) -> RgbaTexture {
        RgbaTexture {
            width,
            height: pixels.len() as u32 / width,
            pixels: pixels.concat(),
        }
    }

    #[test]
    fn one_to_one_sprites_take_whole_texels_and_blend_by_source_alpha() {
        let tex = texture(&[[200, 0, 0, 0x80], [0, 100, 0, 0x40]], 2);
        let mut layer = GsLayer::new();
        layer.draw(
            &GsSprite {
                x: [10, 12],
                y: [5, 6],
                u: [0.5, 2.5],
                v: [0.5, 1.5],
                rgba: [0x80; 4],
                texture: Some(&tex),
            },
            0,
        );
        let at = |x: usize| {
            (
                layer.p[5 * WIDTH + x],
                1.0 - layer.cover[5 * WIDTH + x],
                layer.alpha[5 * WIDTH + x],
            )
        };
        assert_eq!(at(10), ([200.0, 0.0, 0.0], 0.0, 0x80));
        assert_eq!(at(11), ([0.0, 50.0, 0.0], 0.5, 0x40));
        assert_eq!(at(12), ([0.0; 3], 1.0, 0));
    }

    #[test]
    fn a_mask_in_the_alpha_plane_cuts_a_later_sprite() {
        let mut layer = GsLayer::new();
        // HudGauge_GsBeginMask: context 2 writes only alpha (FBA, NOTEQUAL 0 test), context 1 passes where the
        // alpha's top bit is clear.
        layer.ctx[0].test = 0x34000;
        layer.ctx[1] = Context {
            alpha: 0x44,
            test: 0x3000F,
            fba: true,
            fbmsk: 0xFF_FFFF,
        };
        let plain = |x: [i32; 2], rgba: [u8; 4]| GsSprite {
            x,
            y: [0, 1],
            u: [0.0; 2],
            v: [0.0; 2],
            rgba,
            texture: None,
        };
        layer.draw(&plain([4, 8], [0, 0, 0, 1]), 1);
        layer.draw(&plain([0, 10], [90, 90, 90, 0x80]), 0);
        assert_eq!(layer.p[3], [90.0; 3]);
        assert_eq!(layer.p[5], [0.0; 3]);
        assert_eq!(layer.cover[5], 0.0);
        assert_eq!(layer.alpha[5], 0x81);
        // ALPHA 0x48 adds Cs * As and keeps the background.
        layer.ctx[1] = Context {
            alpha: 0x48,
            test: 0x30000,
            fba: false,
            fbmsk: 0,
        };
        layer.draw(&plain([5, 6], [100, 0, 0, 0x40]), 1);
        assert_eq!((layer.p[5], layer.cover[5]), ([50.0, 0.0, 0.0], 0.0));
    }
}
