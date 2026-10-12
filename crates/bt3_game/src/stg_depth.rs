//! The passes over the depth buffer's spare byte (src/sys/gfx_post.c, src/sys/gfx_screen.c, src/battle/screen_fx.c).
//!
//! `StgFx_DrawPre` runs right after the stage is drawn, before the battle objects. Its first pass,
//! `GfxDepthFog_Draw`, fills the spare top byte of the 24-bit depth buffer from the depth itself: it reads the
//! depth page as 16-bit pixels and writes each word's low half into the frame's 16-bit view 8 pixels on (the high
//! half of the same 32-bit pixel), the write mask keeping the top byte; the frame's alpha then holds depth bits
//! 8..15, which a ramp CLUT (255 - index) draws into the cleared spare byte where the stage's depth is below
//! 0xFFA8 (greater-than test). So the byte is `255 - (Z >> 8)` for depth below 0xFFA8, 0 nearer, and 255 where no
//! depth was written (the sky). The passes after it read that byte through CLUTs of their own: the depth tint
//! here.

/// `StgCurve`: three keys (x: the byte, z: the output, both 0..255).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Curve {
    pub keys: [[f32; 2]; 3],
}

/// `Vec3_Hermite`'s weights.
fn hermite(p0: [f32; 3], p1: [f32; 3], t0: [f32; 3], t1: [f32; 3], t: f32) -> [f32; 3] {
    let h0 = (t - 1.0) * (t - 1.0) * (2.0 * t + 1.0);
    let h1 = t * t * (3.0 - 2.0 * t);
    let h2 = (1.0 - t) * (1.0 - t) * t;
    let h3 = (t - 1.0) * t * t;
    std::array::from_fn(|k| h0 * p0[k] + h1 * p1[k] + h2 * t0[k] + h3 * t1[k])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// `Vec3_Normalize` scaled to `len`.
fn with_length(a: [f32; 3], len: f32) -> [f32; 3] {
    let l = length(a);
    if l == 0.0 {
        return [0.0; 3];
    }
    a.map(|c| c / l * len)
}

impl Curve {
    /// `StgCurve_Build`: 0 below the first key; each segment sampled at every byte from key to key (both ends:
    /// the second segment starts one entry late) through the Hermite curve, the middle key's tangents along its
    /// neighbours' chord (the end keys' are zero), the output clamped to 0..255 over 255; the last sample's value
    /// to the end. A zero-length segment (no stage has one) is taken as its end key.
    pub(crate) fn build(&self) -> [f32; 256] {
        let pos = self.keys.map(|[x, z]| [x, 0.0, z]);
        // The middle key: tanIn toward the next along prev -> next, tanOut toward the previous along next -> prev.
        let d_in = sub(pos[2], pos[1]);
        let tan_in = with_length(sub(pos[2], pos[0]), length(d_in));
        let d_out = sub(pos[0], pos[1]);
        let tan_out = with_length(sub(pos[0], pos[2]), length(d_out));
        let tan_out_of = [[0.0; 3], tan_out, [0.0; 3]];
        let tan_in_of = [[0.0; 3], tan_in, [0.0; 3]];
        let mut table = [0.0f32; 256];
        let mut count = 0usize;
        for _ in 0..(self.keys[0][0] as i32).max(0) {
            if count < 256 {
                table[count] = 0.0;
            }
            count += 1;
        }
        let mut last = 0.0;
        for i in 0..2 {
            let steps = self.keys[i + 1][0] as i32 - self.keys[i][0] as i32;
            let t0 = tan_out_of[i].map(|c| -c);
            let t1 = tan_in_of[i + 1];
            for s in 0..=steps.max(0) {
                let t = if steps == 0 {
                    1.0
                } else {
                    s as f32 / steps as f32
                };
                let out = hermite(pos[i], pos[i + 1], t0, t1, t);
                let z = out[2].clamp(0.0, 255.0);
                table[count.min(255)] = z / 255.0;
                if count < 0xFF {
                    count += 1;
                }
                last = z;
            }
        }
        for v in table.iter_mut().skip(count) {
            *v = last / 255.0;
        }
        table
    }
}

/// `StgDepthTint`: one medium's tint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DepthTint {
    pub enabled: bool,
    /// Colour scaled by the curve and added (ALPHA 0x48); else a constant colour blended by the curve (0x44).
    pub additive: bool,
    /// Byte 0xFF (the sky) gets a transparent black entry.
    pub clear_last: bool,
    pub color: [f32; 4],
    pub curve: Curve,
}

impl DepthTint {
    /// `StgDepthTint_LoadParams` of one entry of stage section +0x40 (`None`: off, additive grey, a straight curve).
    pub(crate) fn from_param(p: Option<&[u8; 16]>) -> Self {
        match p {
            None => Self {
                enabled: false,
                additive: true,
                clear_last: false,
                color: [0.5, 0.5, 0.5, 1.0],
                curve: Curve {
                    keys: [[0.0, 0.0], [128.0, 128.0], [255.0, 255.0]],
                },
            },
            Some(p) => {
                let flags = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                Self {
                    enabled: flags & 1 != 0,
                    additive: flags & 2 != 0,
                    clear_last: flags & 4 != 0,
                    color: [p[4], p[5], p[6], p[7]].map(|c| f32::from(c) / 255.0),
                    curve: Curve {
                        keys: [
                            [f32::from(p[8]), f32::from(p[9])],
                            [f32::from(p[10]), f32::from(p[11])],
                            [f32::from(p[12]), f32::from(p[13])],
                        ],
                    },
                }
            }
        }
    }

    /// `StgDepthTint_BuildClut`, by byte value: additive, the colour times curve * 128 * alpha with alpha 0x80;
    /// blended, the colour (x 255) with alpha curve * 128 * alpha. With `clear_last` byte 0xFF is transparent black.
    pub(crate) fn clut(&self) -> [[u8; 4]; 256] {
        let table = self.curve.build();
        let [r, g, b, a] = self.color;
        std::array::from_fn(|i| {
            if self.clear_last && i == 0xFF {
                return [0; 4];
            }
            let t = table[i];
            if self.additive {
                [
                    (t * r * 128.0 * a) as i32 as u8,
                    (t * g * 128.0 * a) as i32 as u8,
                    (t * b * 128.0 * a) as i32 as u8,
                    0x80,
                ]
            } else {
                [
                    (r * 255.0) as i32 as u8,
                    (g * 255.0) as i32 as u8,
                    (b * 255.0) as i32 as u8,
                    (t * 128.0 * a) as u32 as u8,
                ]
            }
        })
    }
}

/// A full-screen draw of the depth byte through a CLUT (`GfxPost_DrawDepthClut`): the CLUT by byte value (GS
/// colours, alpha 0x80 = 1.0) and the blend, `(Cs - Cd) * As + Cd` or `Cs * As + Cd`.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthClutDraw {
    pub clut: Vec<[u8; 4]>,
    pub additive: bool,
}

/// `gStgDepthTint`: the tints in the air (0) and under water (1).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DepthTints {
    tints: [DepthTint; 2],
    cluts: [Vec<[u8; 4]>; 2],
}

impl DepthTints {
    /// `StgDepthTint_LoadParams` (`StgFx_Reset`).
    pub(crate) fn new(params: &[[u8; 16]]) -> Self {
        let tints = [0, 1].map(|i| DepthTint::from_param(params.get(i)));
        Self {
            cluts: tints.map(|t| t.clut().to_vec()),
            tints,
        }
    }

    /// `StgDepthTint_Draw`: the medium's tint (under water when the camera is below the stage's water level, never
    /// in split screen) when it is on.
    pub(crate) fn draw(&self, under_water: bool) -> Option<DepthClutDraw> {
        let i = usize::from(under_water);
        self.tints[i].enabled.then(|| DepthClutDraw {
            clut: self.cluts[i].clone(),
            additive: self.tints[i].additive,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_is_flat_before_its_first_key_and_holds_its_last() {
        let c = Curve {
            keys: [[10.0, 0.0], [20.0, 255.0], [30.0, 255.0]],
        };
        let t = c.build();
        assert!(t[..10].iter().all(|&v| v == 0.0));
        // The first segment: key 0 at entry 10, key 1 at 20; the second starts again at 21.
        assert_eq!(t[10], 0.0);
        assert_eq!(t[20], 1.0);
        assert!(t[15] > 0.3 && t[15] < 0.7, "{}", t[15]);
        assert_eq!(t[21], 1.0);
        assert!(t[40..].iter().all(|&v| v == 1.0));
        // The default keys on the diagonal: the end keys' tangents are zero, the middle one's as long as the
        // segment, so half way to it the curve is at 64 - 127 / 8.
        let s = Curve {
            keys: [[0.0, 0.0], [128.0, 128.0], [255.0, 255.0]],
        }
        .build();
        assert_eq!(s[64], 48.125 / 255.0);
        assert_eq!(s[128], 128.0 / 255.0);
    }

    #[test]
    fn the_tint_cluts_follow_the_curve() {
        let mut p = [0u8; 16];
        p[0] = 1 | 4;
        p[4..8].copy_from_slice(&[255, 128, 0, 255]);
        p[8..14].copy_from_slice(&[0, 0, 128, 128, 255, 255]);
        let t = DepthTint::from_param(Some(&p));
        assert!(t.enabled && !t.additive && t.clear_last);
        let clut = t.clut();
        assert_eq!(clut[0], [255, 128, 0, 0]);
        assert_eq!(clut[0xFE][3], 127);
        assert_eq!(clut[0xFF], [0; 4]);
        p[0] = 3;
        let clut = DepthTint::from_param(Some(&p)).clut();
        assert_eq!(clut[0xFF], [128, 64, 0, 0x80]);
        let tints = DepthTints::new(&[p]);
        assert!(tints.draw(false).is_some_and(|d| d.additive));
        assert!(tints.draw(true).is_none());
    }
}
