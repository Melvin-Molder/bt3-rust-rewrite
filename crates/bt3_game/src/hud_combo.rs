//! The HUD's combo part (src/battle/hud_combo.c, sprite sheet 2): per side an event message (one of five
//! pictures) that slides in, stays 1.2 s and slides out; a text line, the name of an entry of the fighter's skill
//! script (drawn with the font after the trees, `HudCombo_DrawText`); the combo damage, counting up to the new
//! total with a rolling last digit; and the hit counter, whose digits pop on every new hit.
//!
//! One tree serves both sides: side 0 is placed from the left edge, side 1 from the right (x' = 372 - x, the text
//! line 512 - x). Every animation stands still while the battle is paused.

use crate::battle_seq::Ramp;
use crate::font::{Font, rgba};
use crate::hud::{HudBlend, HudPrim, HudSheet, Sprite, Xf, fan, sprite_polygon};
use crate::rng::LibcRandom;

const SPR_MESSAGE: usize = 0;
const SPR_PLATE: usize = 1;
const SPR_DAMAGE: usize = 2;
const SPR_HITS: usize = 8;

// The damage and hit count nodes' states.
const ST_IN: i32 = 0;
const ST_SHOWN: i32 = 3;
const ST_POP: i32 = 4;
const ST_OUT: i32 = 7;
const ST_OFF: i32 = 10;

const DAMAGE_MAX: i32 = 199_998;
const HITS_MAX: u32 = 999;

const POW: [u32; 6] = [1, 10, 100, 1000, 10000, 100_000];

/// What `Hud_PreUpdate` reads of a side for the combo part.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SideCombo {
    /// `BtlSide_IsComboHitNew`: a counted hit landed on the opponent this frame (not while it is frozen).
    pub(crate) hit_new: bool,
    /// `BtlCtrl_GetClashCount` (the clash's count, 0 outside one) and `BtlSide_GetComboHits`.
    pub(crate) clash_count: i32,
    pub(crate) hits: i32,
    /// `BtlCtrl_IsOppComboDamageNew` and `BtlSide_GetComboDamage` (-1 while the opponent has flag 0xED).
    pub(crate) damage_new: bool,
    pub(crate) damage: i32,
    /// `BtlSide_IsComboShown`: the opponent's combo timer runs.
    pub(crate) shown: bool,
    /// `BtlCtrl_GetFlagDEtoE2`: the first of flags 0xDE..0xE2 (-1 none or frozen).
    pub(crate) text: i32,
    /// The event messages' flags: the powered-up mode raised (1), flag 0x71 (0), lock-on raised in the fight (2),
    /// flag 0xE3 (3), flag 0x9C raised (4); none while frozen.
    pub(crate) powered_up: bool,
    pub(crate) flag71: bool,
    pub(crate) lock_on: bool,
    pub(crate) flag_e3: bool,
    pub(crate) flag_9c: bool,
}

/// `gHudCombo`.
pub(crate) struct ComboPart {
    sheet: HudSheet,
    spr: [Sprite; 12],
    side: usize,
    msg_state: [i32; 2],
    dmg_state: [i32; 2],
    hit_state: [i32; 2],
    text_state: [i32; 2],
    msg: [i32; 2],
    damage: [i32; 2],
    damage_from: [i32; 2],
    hits: [u32; 2],
    hits_raw: [i32; 2],
    hits_prev: [i32; 2],
    text: [i32; 2],
    msg_ramp: [Ramp; 2],
    text_ramp: [Ramp; 2],
    dmg_ramp: [Ramp; 2],
    hit_ramp: [Ramp; 2],
    count_ramp: [Ramp; 2],
    hide: Ramp,
    last_digit: [u16; 2],
    text_pos: [[i32; 2]; 2],
    text_alpha: [f32; 2],
    /// The damage / hit count nodes are shown.
    dmg_shown: bool,
    hit_shown: bool,
    /// This pass's node offsets: message, text, damage, hits.
    ofs: [[i32; 2]; 4],
    root: [i32; 2],
    /// `BATTLE_FLAG_PAUSE`.
    pub(crate) paused: bool,
}

impl ComboPart {
    /// `HudCombo_Init` (and the off states of `HudCombo_Reset`).
    pub(crate) fn new(sheet: HudSheet) -> Self {
        let mut spr = [Sprite::default(); 12];
        spr[SPR_MESSAGE].init_tex_sub(&sheet, 2, 0);
        spr[SPR_PLATE].init_plain([0x80; 4]);
        spr[SPR_PLATE].set_rect(0, 128, 0, 32);
        spr[SPR_DAMAGE].init_tex_sub(&sheet, 2, 0);
        spr[SPR_DAMAGE].set_rect(0, 128, 0, 32);
        spr[SPR_DAMAGE].set_uv(0, 128, 32, 64);
        spr[SPR_HITS].init_tex_sub(&sheet, 2, 0);
        spr[SPR_HITS].set_rect(0, 128, -30, 2);
        spr[SPR_HITS].set_uv(0, 128, 2, 32);
        for i in [3, 4, 5, 6, 7, 9, 10, 11] {
            spr[i].init_tex_sub(&sheet, 0, 0);
        }
        for s in &mut spr {
            s.show(false);
        }
        Self {
            sheet,
            spr,
            side: 0,
            msg_state: [-1; 2],
            dmg_state: [ST_OFF; 2],
            hit_state: [ST_OFF; 2],
            text_state: [-1; 2],
            msg: [-1; 2],
            damage: [0; 2],
            damage_from: [0; 2],
            hits: [0; 2],
            hits_raw: [0; 2],
            hits_prev: [0; 2],
            text: [0; 2],
            msg_ramp: [Ramp::default(); 2],
            text_ramp: [Ramp::default(); 2],
            dmg_ramp: [Ramp::default(); 2],
            hit_ramp: [Ramp::default(); 2],
            count_ramp: [Ramp::default(); 2],
            hide: Ramp::default(),
            last_digit: [0; 2],
            text_pos: [[-1; 2]; 2],
            text_alpha: [0.0; 2],
            dmg_shown: false,
            hit_shown: false,
            ofs: [[0; 2]; 4],
            root: [0; 2],
            paused: false,
        }
    }

    /// The combo part of `Hud_PreUpdate` for one side: the hits and damage (not while paused), the text line and
    /// the messages.
    pub(crate) fn set_side(&mut self, side: usize, c: &SideCombo, paused: bool) {
        if !paused {
            if c.hit_new {
                let v = if c.clash_count != 0 {
                    c.clash_count
                } else {
                    c.hits
                };
                self.set_hits(side, v as u32);
            }
            if c.damage_new && c.damage >= 0 {
                self.set_damage(side, c.damage, c.hit_new);
            }
            if !c.shown {
                self.set_hits(side, 0);
                self.set_damage(side, -1, c.hit_new);
            }
        }
        if c.text >= 0 {
            self.text_state[side] = 0;
            self.text[side] = c.text;
        }
        for (on, m) in [
            (c.powered_up, 1),
            (c.flag71, 0),
            (c.lock_on, 2),
            (c.flag_e3, 3),
            (c.flag_9c, 4),
        ] {
            if on {
                // HudCombo_SetMessage.
                self.msg_state[side] = 0;
                self.msg[side] = m;
            }
        }
    }

    /// `HudCombo_SetDamage`: negative slides the number out; else it counts from where the last count stands
    /// (from 0 for a new combo) to the new value in 0.6 s, or difference / 9500 s above 6000, at most 1.5 s.
    fn set_damage(&mut self, side: usize, damage: i32, is_new: bool) {
        let damage = damage.min(DAMAGE_MAX);
        if damage < 0 {
            if (self.dmg_state[side] - ST_OUT) as u32 >= 4 {
                self.dmg_state[side] = ST_OUT;
            } else {
                return;
            }
        } else {
            let ramp = self.count_ramp[side];
            if is_new && self.hits_raw[side] <= self.hits_prev[side] {
                self.damage_from[side] = 0;
            } else if ramp.value != ramp.target() {
                self.damage_from[side] +=
                    (ramp.value * (self.damage[side] - self.damage_from[side]) as f32) as i32;
            } else {
                self.damage_from[side] = self.damage[side];
            }
            self.damage[side] = damage;
            let diff = self.damage[side] - self.damage_from[side];
            let mut seconds = 0.6f32;
            if diff > 6000 {
                seconds = diff as f32 / 9500.0;
            }
            seconds = seconds.min(1.5);
            self.count_ramp[side].start(seconds, 0.0, 1.0);
            self.dmg_state[side] = if (self.dmg_state[side] as u32) < 6 {
                ST_POP
            } else {
                ST_IN
            };
        }
        self.spr[SPR_DAMAGE].show(true);
    }

    /// `HudCombo_SetHits`: below 2 the counter slides out; a lower count slides in anew, a higher one pops.
    fn set_hits(&mut self, side: usize, hits: u32) {
        self.hits_raw[side] = hits as i32;
        let hits = hits.min(HITS_MAX);
        if hits < 2 {
            if (self.hit_state[side] - ST_OUT) as u32 >= 4 {
                self.hit_state[side] = ST_OUT;
            } else {
                return;
            }
        } else {
            self.hit_state[side] = if hits < self.hits[side] {
                ST_IN
            } else if (self.hit_state[side] as u32) < 6 {
                ST_POP
            } else {
                ST_IN
            };
            self.hits_prev[side] = self.hits[side] as i32;
            self.hits[side] = hits;
        }
        self.spr[SPR_HITS].show(true);
    }

    /// Steps a ramp unless paused; whether it has ended.
    fn step(paused: bool, ramp: &mut Ramp) -> bool {
        !paused && ramp.step()
    }

    /// The node offset of a side: `x` from the left edge, or from 372 (`right`) on side 1.
    fn place(&self, pos: [f32; 2], right: i32) -> [i32; 2] {
        if self.side == 0 {
            [pos[0] as i32, pos[1] as i32]
        } else {
            [right - pos[0] as i32, pos[1] as i32]
        }
    }

    /// `HudCombo_SelectSide`, `HudNode_Update` and `HudNode_Draw` of the tree for one side.
    pub(crate) fn run(&mut self, side: usize, libc: &mut LibcRandom, out: &mut Vec<HudPrim>) {
        self.side = side;
        // HudCombo_UpdateRoot.
        if !self.paused && side == 0 {
            self.hide.step();
        }
        let h = self.hide.value;
        self.root = if side == 0 {
            [(h * -256.0) as i32, (h * 0.0) as i32]
        } else {
            [(h * 256.0) as i32, (h * 0.0) as i32]
        };
        self.update_message();
        self.update_text();
        self.update_damage(libc);
        self.update_hits();
        // Node 1 at (0, 85); the message at (0, 16), the text at (0, 4), the counters at (0, 80).
        let base = [self.root[0], self.root[1] + 85];
        let at = |o: [i32; 2], y: i32| {
            Xf::translate((base[0] + o[0]) as f32, (base[1] + y + o[1]) as f32)
        };
        let message = at(self.ofs[0], 16);
        self.draw(out, &message, &[SPR_MESSAGE]);
        // HudCombo_StoreTextPos: the plate is not drawn; its place and alpha are noted for the text line.
        if self.spr[SPR_PLATE].hidden {
            self.text_pos[side] = [-1, -1];
        } else {
            self.text_alpha[side] = f32::from(self.spr[SPR_PLATE].rgba[3]) / 128.0;
            self.text_pos[side] = [base[0] + self.ofs[1][0], base[1] + 4 + self.ofs[1][1]];
        }
        if self.dmg_shown {
            let xf = at(self.ofs[2], 80);
            self.draw(out, &xf, &[2, 3, 4, 5, 6, 7]);
        }
        if self.hit_shown {
            let xf = at(self.ofs[3], 80);
            self.draw(out, &xf, &[8, 9, 10, 11]);
        }
    }

    /// `HudCombo_DrawSprites`.
    fn draw(&self, out: &mut Vec<HudPrim>, xf: &Xf, sprites: &[usize]) {
        for &i in sprites {
            let spr = &self.spr[i];
            if spr.hidden {
                continue;
            }
            out.push(HudPrim {
                texture: usize::try_from(spr.tex).ok().map(|t| self.sheet.base + t),
                vertices: fan(&sprite_polygon(spr, xf, &self.sheet)),
                rgba: spr.rgba,
                blend: HudBlend::Alpha,
            });
        }
    }

    /// `HudCombo_UpdateMessage`: picture 3 + message slides in from its own width in 0.1 s, holds 1.2 s and
    /// slides out; message 4 sits 310 pixels lower.
    fn update_message(&mut self) {
        let side = self.side;
        let msg = self.msg[side];
        let mut pos = [0.0f32; 2];
        if (0..5).contains(&msg) {
            self.spr[SPR_MESSAGE].init_tex_sub(&self.sheet, 3 + msg as usize, 0);
        }
        let s = &self.spr[SPR_MESSAGE];
        let width = -f32::from(s.uv[1] - s.uv[0]);
        self.spr[SPR_MESSAGE].show(true);
        let paused = self.paused;
        let ramp = &mut self.msg_ramp[side];
        let state = &mut self.msg_state[side];
        if *state == 0 {
            ramp.start(0.1, 1.0, 0.0);
            *state = 1;
        }
        match *state {
            1 => {
                if Self::step(paused, ramp) {
                    ramp.start(1.2, 0.0, 1.0);
                    *state = 2;
                    pos[0] = 0.0;
                } else {
                    pos[0] = width * ramp.value;
                }
            }
            2 => {
                if Self::step(paused, ramp) {
                    ramp.start(0.1, 0.0, 1.0);
                    *state = 3;
                }
            }
            3 => {
                if Self::step(paused, ramp) {
                    self.spr[SPR_MESSAGE].show(false);
                    *state = 4;
                } else {
                    pos[0] = width * ramp.value;
                }
            }
            _ => {
                self.spr[SPR_MESSAGE].show(false);
                return;
            }
        }
        pos[0] += 5.0;
        if msg == 4 {
            pos[1] += 310.0;
        }
        self.ofs[0] = self.place(pos, 372);
    }

    /// `HudCombo_UpdateText`: the plate fades in over 0.1 s, holds 1.2 s and fades out.
    fn update_text(&mut self) {
        let side = self.side;
        let mut pos = [0.0f32; 2];
        let mut alpha: u8 = 0x80;
        let s = &self.spr[SPR_PLATE];
        let width = -f32::from(s.uv[1] - s.uv[0]);
        self.spr[SPR_PLATE].show(true);
        let paused = self.paused;
        let ramp = &mut self.text_ramp[side];
        let state = &mut self.text_state[side];
        if *state == 0 {
            ramp.start(0.1, 0.0, 1.0);
            *state = 1;
        }
        match *state {
            1 => {
                if Self::step(paused, ramp) {
                    ramp.start(1.2, 0.0, 1.0);
                    alpha = 0x80;
                    *state = 2;
                    pos[0] = 0.0;
                } else {
                    pos[0] = width * ramp.value;
                    alpha = (ramp.value * 128.0) as u8;
                }
            }
            2 => {
                if Self::step(paused, ramp) {
                    ramp.start(0.1, 1.0, 0.0);
                    *state = 3;
                }
            }
            3 => {
                if Self::step(paused, ramp) {
                    self.spr[SPR_PLATE].show(false);
                    alpha = 0;
                    *state = 4;
                } else {
                    pos[0] = width * ramp.value;
                    alpha = (ramp.value * 128.0) as u8;
                }
            }
            _ => {
                self.spr[SPR_PLATE].show(false);
                return;
            }
        }
        self.spr[SPR_PLATE].set_color(0x80, 0x80, 0x80, alpha);
        pos[0] += 15.0;
        self.ofs[1] = self.place(pos, 512);
    }

    /// `HudCombo_SetHitDigits`: three digits 22 pixels apart, units first; the leading zeros hidden.
    fn set_hit_digits(&mut self) {
        const UVS: [[i32; 4]; 10] = [
            [0x01, 0x1F, 0x00, 0x20],
            [0x21, 0x3F, 0x00, 0x20],
            [0x41, 0x5F, 0x00, 0x20],
            [0x61, 0x7F, 0x00, 0x20],
            [0x01, 0x1F, 0x20, 0x40],
            [0x21, 0x3F, 0x20, 0x40],
            [0x41, 0x5F, 0x20, 0x40],
            [0x61, 0x7F, 0x20, 0x40],
            [0x01, 0x1F, 0x40, 0x60],
            [0x21, 0x3F, 0x40, 0x60],
        ];
        let hits = self.hits[self.side];
        for i in 0..3 {
            let spr = &mut self.spr[SPR_HITS + 1 + i];
            spr.init_tex_sub(&self.sheet, 0, 0);
            spr.set_rect(1, 30, -32, 0);
            let digit = (hits % POW[i + 1] / POW[i]) as usize;
            spr.show(true);
            spr.move_by(-22 * i as i32 + 36, 0);
            let [u0, u1, v0, v1] = UVS[digit];
            spr.set_uv(u0, u1, v0, v1);
        }
        for i in (1..3).rev() {
            let spr = &mut self.spr[SPR_HITS + 1 + i];
            if i32::from(spr.uv[0]) != UVS[0][0] || i32::from(spr.uv[2]) != UVS[0][2] {
                break;
            }
            spr.show(false);
        }
    }

    /// `HudCombo_SetDamageDigits`: five digits 12 pixels apart, units first, of damageFrom + (damage - damageFrom)
    /// * the count (at most 99999); while it counts the units digit is a random one unlike the last.
    fn set_damage_digits(&mut self, libc: &mut LibcRandom) {
        const UVS: [[i32; 4]; 10] = [
            [0x00, 0x20, 0x00, 0x20],
            [0x20, 0x40, 0x00, 0x20],
            [0x40, 0x60, 0x00, 0x20],
            [0x60, 0x80, 0x00, 0x20],
            [0x00, 0x20, 0x20, 0x40],
            [0x20, 0x40, 0x20, 0x40],
            [0x40, 0x60, 0x20, 0x40],
            [0x60, 0x80, 0x20, 0x40],
            [0x00, 0x20, 0x40, 0x60],
            [0x20, 0x40, 0x40, 0x60],
        ];
        let side = self.side;
        if !self.paused {
            self.count_ramp[side].step();
        }
        let ramp = self.count_ramp[side];
        let mut value = if self.damage[side] < 2 {
            self.damage[side] as u32
        } else {
            let from = self.damage_from[side] as u32;
            (from as f32 + (self.damage[side] - self.damage_from[side]) as f32 * ramp.value) as u32
        };
        value = value.min(99999);
        for i in 0..5 {
            let mut digit = (value % POW[i + 1] / POW[i]) as usize;
            if !self.paused
                && i == 0
                && value.wrapping_sub(100) <= 99898
                && ramp.value != ramp.target()
            {
                loop {
                    digit = libc.int_range(0, 9) as usize;
                    if digit != usize::from(self.last_digit[side]) {
                        break;
                    }
                }
                self.last_digit[side] = digit as u16;
            }
            let spr = &mut self.spr[SPR_DAMAGE + 1 + i];
            spr.init_tex_sub(&self.sheet, 1, 0);
            spr.set_rect(1, 31, 1, 31);
            spr.show(true);
            spr.move_by(-12 * i as i32 + 96, 0);
            let [u0, u1, v0, v1] = UVS[digit];
            spr.set_uv(u0 + 1, u1 - 1, v0 + 1, v1 - 1);
        }
        for i in (1..5).rev() {
            let spr = &mut self.spr[SPR_DAMAGE + 1 + i];
            if spr.uv[0] != 1 || spr.uv[2] != 1 {
                break;
            }
            spr.show(false);
        }
    }

    /// `HudCombo_UpdateDamage`: the digits, then the node's state: slide in (0.1 s), a 10-pixel nudge when the
    /// value changes, slide out.
    fn update_damage(&mut self, libc: &mut LibcRandom) {
        let side = self.side;
        let mut pos = [0.0f32; 2];
        let s = &self.spr[SPR_DAMAGE];
        let width = -f32::from(s.uv[1] - s.uv[0]);
        self.set_damage_digits(libc);
        self.dmg_shown = true;
        let paused = self.paused;
        let ramp = &mut self.dmg_ramp[side];
        let state = &mut self.dmg_state[side];
        match *state {
            0 | 1 => {
                if *state == 0 {
                    ramp.start(0.1, 1.0, 0.0);
                    *state = 1;
                }
                if Self::step(paused, ramp) {
                    *state = ST_SHOWN;
                    pos[0] = 0.0;
                } else {
                    pos[0] = width * ramp.value;
                }
            }
            4 | 5 => {
                if *state == 4 {
                    ramp.start(0.1, 0.0, 1.0);
                    *state = 5;
                }
                if Self::step(paused, ramp) {
                    if ramp.value == 1.0 {
                        ramp.start(0.1, 1.0, 0.0);
                    } else if ramp.value == 0.0 {
                        *state = ST_SHOWN;
                    }
                }
                pos[0] -= ramp.value * 10.0;
            }
            7 | 8 => {
                if *state == 7 {
                    ramp.start(0.1, 0.0, 1.0);
                    *state = 8;
                }
                if Self::step(paused, ramp) {
                    self.dmg_shown = false;
                    *state = ST_OFF;
                } else {
                    pos[0] = width * ramp.value;
                }
            }
            10 => {
                self.dmg_shown = false;
                self.damage_from[side] = 0;
                self.damage[side] = 0;
                return;
            }
            _ => {}
        }
        pos[0] += 5.0;
        self.ofs[2] = self.place(pos, 372);
    }

    /// `HudCombo_UpdateHits`: the digits, then the node's state: slide in, a pop of the digits (height up to 1.5
    /// and back, 0.1 s each way) when the count changes, slide out.
    fn update_hits(&mut self) {
        let side = self.side;
        let mut pos = [0.0f32; 2];
        let mut scale = 1.0f32;
        let s = &self.spr[SPR_HITS];
        let width = -f32::from(s.uv[1] - s.uv[0]);
        self.set_hit_digits();
        self.hit_shown = true;
        let paused = self.paused;
        let ramp = &mut self.hit_ramp[side];
        let state = &mut self.hit_state[side];
        match *state {
            0 | 1 => {
                if *state == 0 {
                    ramp.start(0.1, 1.0, 0.0);
                    *state = 1;
                }
                if Self::step(paused, ramp) {
                    ramp.start(1.2, 0.0, 1.0);
                    *state = ST_SHOWN;
                    pos[0] = 0.0;
                } else {
                    pos[0] = width * ramp.value;
                }
            }
            4 | 5 => {
                if *state == 4 {
                    ramp.start(0.1, 0.0, 1.0);
                    *state = 5;
                }
                if Self::step(paused, ramp) {
                    if ramp.value == 1.0 {
                        ramp.start(0.1, 1.0, 0.0);
                    } else if ramp.value == 0.0 {
                        *state = ST_SHOWN;
                    }
                }
                scale = ramp.value * 0.5 + 1.0;
            }
            7 | 8 => {
                if *state == 7 {
                    ramp.start(0.1, 0.0, 1.0);
                    *state = 8;
                }
                if Self::step(paused, ramp) {
                    self.hit_shown = false;
                    *state = ST_OFF;
                } else {
                    pos[0] = width * ramp.value;
                }
            }
            10 => {
                self.hit_shown = false;
                return;
            }
            _ => {}
        }
        pos[0] += 5.0;
        self.ofs[3] = self.place(pos, 372);
        for i in 0..3 {
            self.spr[SPR_HITS + 1 + i].scale(1.0, scale);
        }
    }

    /// The hit count on show and the combo damage it counts to, unless the counter is off (for tests).
    #[cfg(test)]
    pub(crate) fn on_show(&self, side: usize) -> Option<(u32, i32)> {
        (self.hit_state[side] != ST_OFF).then_some((self.hits[side], self.damage[side]))
    }

    /// `HudCombo_DrawText`: each side's text line, the name of entry `text` of the side's skill script, where
    /// the plate was noted, aligned left (side 0) or right (side 1).
    pub(crate) fn draw_text(&self, font: &mut Font, scripts: [Option<&[u16]>; 2]) {
        for side in 0..2 {
            let [x, y] = self.text_pos[side];
            if x < 0 || y < 0 {
                continue;
            }
            let Some(name) =
                scripts[side].and_then(|s| crate::btl_text::entry_name(s, self.text[side]))
            else {
                continue;
            };
            let alpha = self.text_alpha[side];
            font.push_style();
            let st = &mut font.style;
            st.flags = 1;
            st.scale = 0.9;
            st.scale_x = 0.95;
            st.scale_y = 1.0;
            st.color = rgba(0xFF, 0xFF, 0xFF, (alpha * 128.0) as u32 as u8);
            st.shadow_mode = 2;
            st.shadow_color = rgba(0x20, 0x20, 0xFF, (alpha * 64.0) as u32 as u8);
            st.align = if side == 0 { 0 } else { 2 };
            font.print_at(x, y, name);
            font.pop_style();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_pop_and_damage_counts_up() {
        let mut part = ComboPart::new(HudSheet::default());
        let mut libc = LibcRandom::new(1);
        let mut out = Vec::new();
        let side = SideCombo {
            hit_new: true,
            hits: 2,
            damage_new: true,
            damage: 1000,
            shown: true,
            text: -1,
            ..SideCombo::default()
        };
        part.set_side(0, &side, false);
        assert_eq!(part.hit_state[0], ST_IN);
        assert_eq!(part.dmg_state[0], ST_IN);
        for _ in 0..30 {
            part.run(0, &mut libc, &mut out);
        }
        assert_eq!(part.hit_state[0], ST_SHOWN);
        // 1000 counts up over 0.6 s: done after 18 frames.
        assert_eq!(part.spr[SPR_DAMAGE + 4].uv[0], 0x21);
        part.set_side(
            0,
            &SideCombo {
                hits: 3,
                damage: 1500,
                ..side
            },
            false,
        );
        assert_eq!(part.hit_state[0], ST_POP);
        assert_eq!(part.dmg_state[0], ST_POP);
        part.run(0, &mut libc, &mut out);
        assert!(part.spr[SPR_HITS + 1].rect[2] < -32);
        // The combo ends: both slide out.
        part.set_side(
            0,
            &SideCombo {
                hit_new: false,
                damage_new: false,
                shown: false,
                ..side
            },
            false,
        );
        assert_eq!(part.hit_state[0], ST_OUT);
        for _ in 0..10 {
            part.run(0, &mut libc, &mut out);
        }
        assert_eq!(part.hit_state[0], ST_OFF);
        assert!(!part.hit_shown && !part.dmg_shown);
    }
}
