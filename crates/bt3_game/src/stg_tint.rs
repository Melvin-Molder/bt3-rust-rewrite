//! The stage tint (`StgTint_*`, src/battle/screen_fx.c): four full-screen colour layers, each ramping linearly
//! between two colours. Slot 0 darkens the stage behind the fighters while any fighter holds request 0x12 (its
//! colours from the stage file's tint section when it has one), slot 1 is the transformation flash's white, both
//! drawn after the stage and before the fighters (`StgTint_DrawBack`); slots 3 and 2 are drawn over everything
//! (`StgTint_DrawFront`). The ramps step in the first draw pass, slots 0..2 not while the battle is paused.

use crate::battle_seq::Ramp;
use crate::eft::{EffectBlend, EffectTri, EffectVertex};

// STG_TINT_*.
const ON: u8 = 1;
const OUT: u8 = 2;
const IN: u8 = 4;
const DONE: u8 = 8;

type V4 = [f32; 4];

/// `StgTint`.
#[derive(Clone, Copy, Debug, Default)]
struct Tint {
    /// r, g, b 0..255, alpha 0..128.
    color: V4,
    from: V4,
    to: V4,
    delta: V4,
    ramp: Ramp,
    flags: u8,
}

impl Tint {
    /// `StgTint_CalcColor`.
    fn calc(&mut self) {
        let t = self.ramp.value;
        self.color = std::array::from_fn(|i| self.from[i] + t * self.delta[i]);
    }
}

/// `gStgTint`, and slot 0's colour as `StgTint_DrawBack` publishes it (`gStgTintColor0`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Tints {
    slot: [Tint; 4],
    color0: [i32; 4],
    /// `BtlStage_GetList80`: the stage's two colours of slot 0.
    stage: Option<[u8; 8]>,
}

impl Tints {
    pub(crate) fn new(stage: Option<[u8; 8]>) -> Self {
        Self {
            stage,
            ..Self::default()
        }
    }

    /// `StgTint_Start`: dir 0 ramps to the slot's second colour, dir 1 back to the first, over `seconds`. A slot
    /// that is off ignores dir 1; one already going the asked way is left alone; a running one restarts from its
    /// current colour.
    pub(crate) fn start(&mut self, slot: usize, dir: usize, seconds: f32) {
        let mut colors: [[V4; 2]; 4] = [
            [[0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 100.0]],
            [[255.0, 255.0, 255.0, 0.0], [255.0, 255.0, 255.0, 128.0]],
            [[255.0, 255.0, 255.0, 0.0], [255.0, 255.0, 255.0, 128.0]],
            [[0.0, 0.0, 0.0, 128.0], [0.0, 0.0, 0.0, 0.0]],
        ];
        if let Some(s) = self.stage {
            colors[0][0] = std::array::from_fn(|i| f32::from(s[i]));
            colors[0][1] = std::array::from_fn(|i| f32::from(s[4 + i]));
        }
        let slot = if slot >= 4 { 0 } else { slot };
        let t = &mut self.slot[slot];
        if t.flags & ON == 0 {
            if dir == 1 {
                return;
            }
            t.flags = ON | OUT;
            t.from = colors[slot][dir];
        } else {
            if dir == 0 {
                if t.flags & OUT != 0 {
                    return;
                }
                t.flags = ON | OUT;
            } else {
                if t.flags & IN != 0 {
                    return;
                }
                t.flags = ON | IN;
            }
            t.from = t.color;
        }
        t.to = colors[slot][dir ^ 1];
        t.delta = std::array::from_fn(|i| t.to[i] - t.from[i]);
        t.ramp.start(seconds, 0.0, 1.0);
    }

    /// `StgTint_IsOn`.
    pub(crate) fn is_on(&self, slot: usize) -> bool {
        self.slot[if slot >= 4 { 0 } else { slot }].flags & ON != 0
    }

    /// `StgTint_GetColor0`: slot 0's colour as the last back pass drew it (0 when off).
    #[cfg(test)]
    pub(crate) fn color0(&self) -> [i32; 4] {
        self.color0
    }

    /// The stage tint section's last byte (slot 0's darkest alpha), 100 without one: what the weather scales its
    /// alpha against.
    pub(crate) fn max_alpha(&self) -> f32 {
        self.stage.map_or(100.0, |s| f32::from(s[7]))
    }

    /// What the weather reads (`StgTint_IsOn(0)`, `StgTint_GetColor0` and the stage's darkest alpha), when slot
    /// 0 is on.
    pub(crate) fn weather(&self) -> Option<([i32; 4], f32)> {
        self.is_on(0).then(|| (self.color0, self.max_alpha()))
    }

    /// `EftStage_GetTintScale`: 1 - slot 0's alpha / 100, within 0.5..1.
    pub(crate) fn scale(&self) -> f32 {
        (1.0 - self.color0[3] as f32 / 100.0).clamp(0.5, 1.0)
    }

    /// `StgTint_Update`: the ramps of the slots that are on step (slots 0..2 not while paused); a ramp back that
    /// ends marks its slot done.
    pub(crate) fn update(&mut self, paused: bool) {
        for (i, t) in self.slot.iter_mut().enumerate() {
            if i != 3 && paused {
                continue;
            }
            if t.flags & ON != 0 && t.ramp.step() && t.flags & IN != 0 {
                t.flags |= DONE;
                t.ramp.stop();
            }
        }
    }

    /// `StgTint_Release`: the slots whose way back ended go off.
    pub(crate) fn release(&mut self) {
        for t in &mut self.slot {
            if t.flags & ON != 0 && t.flags & DONE != 0 {
                t.flags = 0;
                t.ramp.stop();
            }
        }
    }

    /// `StgTint_DrawBack`: slots 0 and 1 when on (slot 0's colour published as integers, 0 when off).
    pub(crate) fn draw_back(&mut self, out: &mut Vec<EffectTri>) {
        let t = &mut self.slot[0];
        if t.flags & ON != 0 {
            t.calc();
            rect(out, t.color);
            self.color0 = t.color.map(|c| c as i32);
        } else {
            self.color0 = [0; 4];
        }
        let t = &mut self.slot[1];
        if t.flags & ON != 0 {
            t.calc();
            rect(out, t.color);
        }
    }

    /// `StgTint_DrawFront`: slots 3 and 2, each when its alpha is above 0.001 (on or not).
    pub(crate) fn draw_front(&mut self) -> Vec<[u8; 4]> {
        let mut out = Vec::new();
        for i in [3, 2] {
            let t = &mut self.slot[i];
            t.calc();
            if t.color[3] > 0.001 {
                out.push(rgba(t.color));
            }
        }
        out
    }
}

/// The colour as `GfxPost_DrawTintRect` writes it into RGBAQ (each channel through `u32` to a byte).
fn rgba(c: V4) -> [u8; 4] {
    c.map(|v| v as u32 as u8)
}

/// `GfxPost_DrawTintRect(0x44, color)`: a blended untextured rectangle over the whole frame, the depth test
/// always passing and nothing written to it.
fn rect(out: &mut Vec<EffectTri>, color: V4) {
    let rgba = rgba(color);
    let corner = |x: f32, y: f32| EffectVertex {
        position: [x, y, 16_777_215.0],
        stq: [0.0, 0.0, 1.0],
        rgba,
    };
    let c = [
        corner(0.0, 0.0),
        corner(512.0, 0.0),
        corner(0.0, 448.0),
        corner(512.0, 448.0),
    ];
    for k in 0..2 {
        out.push(EffectTri {
            texture: None,
            blend: EffectBlend::Alpha,
            clamp: false,
            zwrite: false,
            alpha_ref: 0,
            vertices: [c[k], c[k + 1], c[k + 2]],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_0_darkens_and_comes_back_then_goes_off() {
        let mut t = Tints::new(None);
        // Dir 1 on a slot that is off: nothing.
        t.start(0, 1, 0.2);
        assert!(!t.is_on(0));
        t.start(0, 0, 0.2);
        assert!(t.is_on(0));
        let mut out = Vec::new();
        // Six steps of 0.2 s: from no alpha to 100.
        for _ in 0..6 {
            t.update(false);
            out.clear();
            t.draw_back(&mut out);
        }
        assert_eq!(t.color0(), [0, 0, 0, 100]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].vertices[0].rgba, [0, 0, 0, 100]);
        assert_eq!(t.scale(), 0.5);
        // Paused: slot 0 waits.
        t.start(0, 1, 0.2);
        t.update(true);
        t.draw_back(&mut out);
        assert_eq!(t.color0()[3], 100);
        // Back to the first colour; the slot goes off once its ramp ended and was released.
        for _ in 0..6 {
            t.update(false);
            t.release();
        }
        assert!(!t.is_on(0));
        out.clear();
        t.draw_back(&mut out);
        assert!(out.is_empty());
        assert_eq!(t.color0(), [0; 4]);
        assert_eq!(t.scale(), 1.0);
    }

    #[test]
    fn the_stage_gives_slot_0_its_colours_and_the_front_slots_draw_by_alpha() {
        let mut t = Tints::new(Some([10, 20, 30, 0, 40, 50, 60, 80]));
        assert_eq!(t.max_alpha(), 80.0);
        t.start(0, 0, 0.0);
        t.update(false);
        let mut out = Vec::new();
        t.draw_back(&mut out);
        assert_eq!(t.color0(), [40, 50, 60, 80]);
        // Slot 1 at once (the transformation's flash), drawn after slot 0.
        t.start(1, 0, 0.0);
        t.update(false);
        out.clear();
        t.draw_back(&mut out);
        assert_eq!(out[2].vertices[0].rgba, [255, 255, 255, 128]);
        // Nothing in front until slot 2 or 3 has alpha; slot 3 steps while paused.
        assert!(t.draw_front().is_empty());
        t.start(3, 1, 0.0);
        assert!(t.draw_front().is_empty());
        t.start(3, 0, 0.1);
        t.update(true);
        assert_eq!(t.draw_front(), vec![[0, 0, 0, 85]]);
        // Restarting the way it goes changes nothing; the other way starts from the current colour.
        t.start(3, 0, 0.1);
        t.start(3, 1, 0.1);
        t.update(true);
        assert_eq!(t.draw_front(), vec![[0, 0, 0, 99]]);
    }
}
