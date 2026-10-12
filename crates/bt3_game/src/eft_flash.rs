//! The full-screen flash (`EftFlash*`, src/battle/eft_trail.c) and the blinding overlay (`EftBlind_*`,
//! src/battle/eft_char_parts.c), the last two but one modules of effect layer 3. Both queue one sprite over the
//! whole screen in the order table's last slot, layer 1 (added).
//!
//! A flash fades in, holds and fades out; one runs at a time. The manager starts a white one when a fighter is in
//! action 0x104 with flag 8 (its model swap to the damaged variant): its hold lasts until the fighter's variant is
//! set and no rush finish is half way. The blinding overlay is a grey sprite as strong as player 0's blind level
//! (`BtlCharApi_GetBlindRatio`).

use crate::eft::{EffectBlend, EffectTri, EffectVertex, OrderTable};

type V4 = [f32; 4];

/// `gOtLast`.
const LAST_SLOT: usize = 0x1001;

/// `EftFlashArg`: the colour (alpha 128 = 1) and the three times in seconds; `wait` holds until the rush
/// finish reaches state 2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FlashArg {
    pub color: V4,
    pub fade_in: f32,
    pub hold: f32,
    pub fade_out: f32,
    pub wait: bool,
}

/// `EftFlash`: the times in frames.
#[derive(Clone, Copy, Debug)]
struct Flash {
    arg: FlashArg,
    in_max: f32,
    out_max: f32,
    /// 1 clear, 0 the argument's colour.
    fade: f32,
    /// 1 in, 2 hold, 4 out.
    phase: u8,
    frames: i32,
    step: u8,
}

/// What the flash and the overlay read of a fighter.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FlashChr {
    /// `BtlCharApi_IsFlag8Action104`.
    pub flag8_action_104: bool,
    /// `BtlCharApi_GetMemberVariant` is non-zero.
    pub variant: bool,
    /// `BtlCharApi_IsInRushSequence` and `BtlCharApi_GetRushFinishPhase` (-1 outside a rush sequence).
    pub in_rush: bool,
    pub finish_phase: i32,
    /// `BtlCharApi_TestFlag2B`.
    pub flag_2b: bool,
    /// `BtlCharApi_GetBlindRatio`.
    pub blind: f32,
}

/// `gEftFlash` and its one task, and `gEftBlind`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Flashes {
    /// 1 a flash runs, 2 / 4 the manager started it for fighter 0 / 1.
    flags: u32,
    rush_bits: u32,
    rush_state: i32,
    task: Option<Flash>,
    /// `gEftBlind->alpha[0]`, 0..255, and whether it shows.
    blind: f32,
    blind_shown: bool,
}

impl Flashes {
    /// `EftFlash_Start`: nothing while one runs (its list holds one task).
    pub(crate) fn start(&mut self, arg: FlashArg) -> bool {
        if self.task.is_some() {
            return false;
        }
        // EftFlash_Init.
        let mut a = arg;
        a.fade_in *= 30.0;
        a.hold *= 30.0;
        a.fade_out *= 30.0;
        self.task = Some(Flash {
            arg: a,
            in_max: a.fade_in,
            out_max: a.fade_out,
            fade: 1.0,
            phase: 1,
            frames: 0,
            step: 0,
        });
        self.flags = 1;
        self.rush_bits = 0;
        self.rush_state = 0;
        true
    }

    /// Whether a flash runs, and its phase.
    #[cfg(test)]
    pub(crate) fn phase(&self) -> Option<u8> {
        self.task.map(|t| t.phase)
    }

    /// `EftFlashMgr_Update` then `EftFlash_Update`, then `EftBlind_Update` (`time_stopped`:
    /// `BtlScene_IsTimeStopped`; `paused`: the battle's pause flag).
    pub(crate) fn update(&mut self, chars: &[FlashChr; 2], paused: bool, time_stopped: bool) {
        for (i, c) in chars.iter().enumerate() {
            if c.flag8_action_104 && self.flags & 1 == 0 {
                self.start(FlashArg {
                    color: [255.0, 255.0, 255.0, 128.0],
                    fade_in: 0.5,
                    hold: 1.0,
                    fade_out: 0.5,
                    wait: false,
                });
                self.flags |= if i == 0 { 2 } else { 4 };
            }
        }
        if self.flags & 1 != 0 {
            self.rush_state = self.rush_state(chars);
        }
        self.step(chars, paused);
        // EftBlind_Update: player 0's blind level (split screen is not ported).
        if !time_stopped {
            self.blind = (chars[0].blind * 255.0).clamp(0.0, 255.0);
            self.blind_shown = self.blind > 0.0;
        }
    }

    /// `EftFlash_Update`: fade in, hold, fade out; not while paused. The hold lasts its time and, for the
    /// manager's flash, until the fighter's variant is set and the rush finish is in state 0 or 2; a waiting
    /// flash holds until state 2. 180 frames end the hold regardless.
    fn step(&mut self, chars: &[FlashChr; 2], paused: bool) {
        let (flags, rush_state) = (self.flags, self.rush_state);
        let Some(w) = &mut self.task else {
            return;
        };
        if paused {
            return;
        }
        let mut dead = false;
        match w.step {
            0 => {
                w.arg.fade_in -= 1.0;
                w.phase = 1;
                w.fade = w.arg.fade_in / w.in_max;
                if w.arg.fade_in <= 0.0 {
                    w.arg.fade_in = 0.0;
                    w.step = 1;
                }
            }
            1 => 'hold: {
                w.frames += 1;
                if w.frames >= 180 {
                    w.step = 2;
                }
                if w.arg.wait {
                    if rush_state == 2 {
                        w.step = 2;
                        break 'hold;
                    }
                } else {
                    if rush_state != 0 && rush_state != 2 {
                        break 'hold;
                    }
                    if flags & 2 != 0 && !chars[0].variant {
                        break 'hold;
                    }
                    if flags & 4 != 0 && !chars[1].variant {
                        break 'hold;
                    }
                }
                w.arg.hold -= 1.0;
                w.phase = 2;
                if w.arg.hold <= 0.0 {
                    w.arg.hold = 0.0;
                    w.step = 2;
                }
            }
            _ => {
                w.arg.fade_out -= 1.0;
                w.phase = 4;
                w.fade = 1.0 - w.arg.fade_out / w.out_max;
                if w.arg.fade_out <= 0.0 {
                    w.arg.fade_out = 0.0;
                    dead = true;
                }
            }
        }
        if dead {
            // EftFlash_Term.
            self.task = None;
            self.flags = 0;
        }
    }

    /// `EftFlash_GetRushState`: 0 nothing, 1 a fighter is in rush-finish phase 1, 2 that fighter has flag 0x2B
    /// (when both are in phase 1: both have it). The phase bits are kept between frames, cleared by a fighter
    /// outside a rush sequence (or with phase -1).
    fn rush_state(&mut self, chars: &[FlashChr; 2]) -> i32 {
        for (i, c) in chars.iter().enumerate() {
            if !c.in_rush {
                self.rush_bits = 0;
                continue;
            }
            match c.finish_phase {
                1 => self.rush_bits |= if i == 0 { 1 } else { 2 },
                2 => self.rush_bits |= if i == 0 { 4 } else { 8 },
                -1 => self.rush_bits = 0,
                _ => {}
            }
        }
        match self.rush_bits & 3 {
            _ if self.rush_bits == 0 => 0,
            1 => 1 + i32::from(chars[0].flag_2b),
            2 => 1 + i32::from(chars[1].flag_2b),
            3 => {
                if chars[0].flag_2b {
                    self.rush_bits |= 4;
                }
                if chars[1].flag_2b {
                    self.rush_bits |= 8;
                }
                if self.rush_bits & 0xC == 0xC { 2 } else { 1 }
            }
            _ => 0,
        }
    }

    /// `EftBlind_Draw` (not in a replay) then `EftFlash_Draw`: grey at the blind level, then the flash's colour
    /// blended towards transparent white by its fade, each over the whole screen.
    pub(crate) fn draw(&self, ot: &mut OrderTable, replay: bool) {
        if self.blind_shown && !replay {
            sprite(ot, [128, 128, 128, self.blind as u32 as u8]);
        }
        if let Some(w) = &self.task {
            let clear = [255.0, 255.0, 255.0, 0.0];
            let t = w.fade;
            let c: V4 = std::array::from_fn(|i| clear[i] * t + w.arg.color[i] * (1.0 - t));
            sprite(ot, c.map(|v| v as u32 as u8));
        }
    }
}

/// A sprite (0x7000, 0x7200)..(0x9000, 0x8E00): the whole 512 x 448 screen, at the nearest depth.
fn sprite(ot: &mut OrderTable, rgba: [u8; 4]) {
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
        ot.add(
            LAST_SLOT,
            1,
            EffectTri {
                texture: None,
                blend: EffectBlend::of(1, false),
                clamp: false,
                zwrite: false,
                alpha_ref: 0,
                vertices: [c[k], c[k + 1], c[k + 2]],
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars() -> [FlashChr; 2] {
        [FlashChr::default(); 2]
    }

    #[test]
    fn the_damaged_variants_flash_holds_until_the_variant_is_set() {
        let mut f = Flashes::default();
        let mut c = chars();
        c[1].flag8_action_104 = true;
        f.update(&c, false, false);
        assert_eq!(f.flags, 5);
        // A second request while it runs: nothing new.
        assert!(!f.start(FlashArg {
            color: [0.0; 4],
            fade_in: 0.0,
            hold: 0.0,
            fade_out: 0.0,
            wait: false
        }));
        // 15 frames in (the first counted at once), then holding while fighter 1 has no variant.
        for _ in 0..14 {
            f.update(&c, false, false);
        }
        assert_eq!(f.phase(), Some(1));
        let mut ot = OrderTable::new();
        f.draw(&mut ot, false);
        let tris = ot.into_list();
        assert_eq!(tris[0].vertices[0].rgba, [255, 255, 255, 128]);
        assert_eq!(tris[0].blend, EffectBlend::Add);
        c[1].flag8_action_104 = false;
        for _ in 0..40 {
            f.update(&c, false, false);
        }
        assert_eq!(f.phase(), Some(1));
        // Paused: nothing moves.
        c[1].variant = true;
        f.update(&c, true, false);
        assert_eq!(f.task.unwrap().arg.hold, 30.0);
        // The hold's 30 frames, then 15 out, half way clear.
        for _ in 0..30 {
            f.update(&c, false, false);
        }
        assert_eq!(f.phase(), Some(2));
        for _ in 0..8 {
            f.update(&c, false, false);
        }
        assert_eq!(f.phase(), Some(4));
        let mut ot = OrderTable::new();
        f.draw(&mut ot, false);
        let a = ot.into_list()[0].vertices[0].rgba[3];
        assert!((50..80).contains(&a), "{a}");
        for _ in 0..7 {
            f.update(&c, false, false);
        }
        assert_eq!(f.phase(), None);
        assert_eq!(f.flags, 0);
    }

    #[test]
    fn a_waiting_flash_holds_until_the_rush_finish_reaches_its_second_state() {
        let mut f = Flashes::default();
        let mut c = chars();
        f.start(FlashArg {
            color: [255.0, 255.0, 255.0, 255.0],
            fade_in: 0.1,
            hold: 1.0,
            fade_out: 1.0,
            wait: true,
        });
        // Both fighters in phase 1 of the rush: state 1 until both have flag 0x2B.
        for x in &mut c {
            x.in_rush = true;
            x.finish_phase = 1;
        }
        for _ in 0..10 {
            f.update(&c, false, false);
        }
        assert_eq!(f.rush_state, 1);
        assert_eq!(f.phase(), Some(2));
        c[0].flag_2b = true;
        f.update(&c, false, false);
        assert_eq!(f.rush_state, 1);
        c[1].flag_2b = true;
        f.update(&c, false, false);
        assert_eq!(f.rush_state, 2);
        f.update(&c, false, false);
        assert_eq!(f.phase(), Some(4));
        // Out of the sequence: the bits clear.
        c = chars();
        f.update(&c, false, false);
        assert_eq!(f.rush_state, 0);
    }

    #[test]
    fn the_blinding_overlay_follows_player_0s_blind_level() {
        let mut f = Flashes::default();
        let mut c = chars();
        c[0].blind = 0.5;
        c[1].blind = 1.0;
        f.update(&c, false, false);
        let mut ot = OrderTable::new();
        f.draw(&mut ot, false);
        let tris = ot.into_list();
        assert_eq!(tris.len(), 2);
        assert_eq!(tris[0].vertices[0].rgba, [128, 128, 128, 127]);
        // Time stopped: it keeps its level.
        c[0].blind = 0.0;
        f.update(&c, false, true);
        assert!(f.blind_shown);
        f.update(&c, false, false);
        let mut ot = OrderTable::new();
        f.draw(&mut ot, false);
        assert!(ot.into_list().is_empty());
    }
}
