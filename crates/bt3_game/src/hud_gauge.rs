//! The HUD's gauge part (src/battle/hud_gauge_1.c, hud_gauge_2.c, hud_gauge_3.c: health, ki, blast stock,
//! powered-up timer, status icons, face) and timer part (src/battle/hud_timer.c), drawn through the GS emulation
//! of hud_gs.rs. One tree of nodes serves both players: side 1 is the same tree drawn mirrored (x' = 511 - x).
//!
//! Not ported yet: the status icons' change animation (`HudGauge_UpdateStatIcons` states 0..5) and the face tint
//! (object flag 0x40000).

use std::collections::HashMap;

use bt3_data::gs::RgbaTexture;

use crate::battle_seq::Ramp;
use crate::hud::{HudSheet, Sprite};
use crate::hud_gs::{GsLayer, GsSprite};
use crate::rng::LibcRandom;

const HP_BAR: i32 = 10000;
const KI_BAR: i32 = 20000;
const POWER_BAR: i32 = 6000;
const BLAST_STOCK: i32 = 100000;

/// The textures the CPU draws with: sheet 0 by (entry, palette offset), each side's face, and each member's face
/// for the team panel (`BattleMember.data`: texture 0 of file 8 + character * 2 + side).
#[derive(Clone, Default)]
pub(crate) struct HudTextures {
    pub(crate) sheet0: HashMap<(i16, u16), RgbaTexture>,
    pub(crate) faces: [Option<RgbaTexture>; 2],
    pub(crate) members: [[Option<RgbaTexture>; 5]; 2],
}

/// What `Hud_PreUpdate` reads of a side (`BtlSide_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SideGauge {
    pub(crate) hp: i32,
    pub(crate) ki: i32,
    pub(crate) blast: i32,
    pub(crate) blast_max: i32,
    pub(crate) max_power: i32,
    pub(crate) powered_up: bool,
    pub(crate) dead: bool,
}

/// Draws sprites of one part: the node origin, the mirror of side 1, the textures.
pub(crate) struct Painter<'a> {
    pub(crate) layer: &'a mut GsLayer,
    pub(crate) textures: &'a HudTextures,
    pub(crate) mirror: bool,
}

impl Painter<'_> {
    fn corners(&self, spr: &Sprite, origin: [i32; 2]) -> ([i32; 2], [i32; 2]) {
        let [x0, x1, y0, y1] = spr.rect.map(i32::from);
        let flip = |x: i32| if self.mirror { 511 - x } else { x };
        (
            [flip(origin[0] + x0), flip(origin[0] + x1)],
            [origin[1] + y0, origin[1] + y1],
        )
    }

    /// `HudSprite_DrawAt` (or `HudSprite_Draw`) through context `ctx`.
    pub(crate) fn draw(&mut self, spr: &Sprite, origin: [i32; 2], ctx: usize, face: Option<usize>) {
        let textures = self.textures;
        let texture = if spr.tex < 0 {
            None
        } else if let Some(side) = face {
            textures.faces[side].as_ref()
        } else {
            textures
                .sheet0
                .get(&(spr.tex, spr.sub))
                .or_else(|| textures.sheet0.get(&(spr.tex, 0)))
        };
        if spr.tex >= 0 && texture.is_none() {
            return;
        }
        self.draw_texture(spr, origin, ctx, texture);
    }

    /// `HudSprite_DrawAt` of a sprite with its texture given.
    pub(crate) fn draw_texture(
        &mut self,
        spr: &Sprite,
        origin: [i32; 2],
        ctx: usize,
        texture: Option<&RgbaTexture>,
    ) {
        if spr.hidden {
            return;
        }
        let (x, y) = self.corners(spr, origin);
        let [u0, u1, v0, v1] = spr.uv.map(|t| f32::from(t) + 0.5);
        let u = if spr.mirror { [u1, u0] } else { [u0, u1] };
        self.layer.draw(
            &GsSprite {
                x,
                y,
                u,
                v: [v0, v1],
                rgba: spr.rgba,
                texture,
            },
            ctx,
        );
    }

    /// `HudSprite_DrawAlphaClear`.
    pub(crate) fn alpha_clear(&mut self, spr: &Sprite, origin: [i32; 2]) {
        let (x, y) = self.corners(spr, origin);
        self.layer.alpha_clear(x, y);
    }

    /// `HudGauge_GsBeginMask`: context 2 writes only alpha (FBA) where the texel's alpha is not 0; context 1
    /// draws where the alpha's top bit is clear.
    pub(crate) fn begin_mask(&mut self) {
        self.layer.ctx[0].test = 0x34000;
        self.layer.ctx[1].fbmsk = 0xFF_FFFF;
        self.layer.ctx[1].fba = true;
        self.layer.ctx[1].test = 0x3000F;
    }

    pub(crate) fn end_mask(&mut self) {
        self.layer.ctx[0].test = 0x30000;
        self.layer.ctx[1].fbmsk = 0;
        self.layer.ctx[1].fba = false;
        self.layer.ctx[1].test = 0x30000;
    }

    /// `HudGauge_GsBeginMask2`: context 1 writes only alpha (FBA) where the texel's alpha is at least 0x7F,
    /// context 2 adds (ALPHA 0x48) where the alpha's top bit is set.
    fn begin_mask2(&mut self) {
        self.layer.ctx[1].alpha = 0x48;
        self.layer.ctx[1].test = 0x3C000;
        self.layer.ctx[0].test = 0x307FB;
        self.layer.ctx[0].fbmsk = 0xFF_FFFF;
        self.layer.ctx[0].fba = true;
        self.layer.ctx[1].fba = true;
    }

    /// `HudGauge_GsEndMask2` (TEST_1 stays as it is).
    fn end_mask2(&mut self) {
        self.layer.ctx[1].fbmsk = 0;
        self.layer.ctx[1].alpha = 0x44;
        self.layer.ctx[0].fbmsk = 0;
        self.layer.ctx[0].fba = false;
        self.layer.ctx[1].fba = false;
        self.layer.ctx[1].test = 0x30000;
    }
}

/// `gHudGauge`.
pub(crate) struct GaugePart {
    sheet: HudSheet,
    spr: [Sprite; 76],
    side: usize,
    /// By side: the panel, the health node, the ki node.
    shake: [[(u16, u16); 3]; 2],
    /// `ofsX / ofsY` of nodes 1, 3 and 4 (shared by both sides' passes).
    ofs: [[i32; 2]; 3],
    root_pos: [i32; 2],
    panel_pos: [i32; 2],
    hp_shown: [i32; 2],
    hp: [i32; 2],
    aura_alpha: [i32; 2],
    low_hp_pulse: [Ramp; 2],
    blast: [i32; 2],
    blast_max: [i32; 2],
    ki: [i32; 2],
    ki_shown: [i32; 2],
    ki_bar_flash: [Ramp; 2],
    ki_full_pulse: [Ramp; 2],
    max_power: [i32; 2],
    blast_flash: [Ramp; 2],
    stat_mask: [u32; 2],
    stat_pos: [u32; 2],
    stat_to: [[i32; 4]; 2],
    ki_reserve: [i32; 2],
    hp_bar_fade: [Ramp; 2],
    slide: [Ramp; 2],
    slide_all: Ramp,
    /// `BATTLE_FLAG_PAUSE`: every animation stands still.
    pub(crate) paused: bool,
    hp_drain: [i32; 2],
    ki_drain: [i32; 2],
    ki_reserve_pulse: [Ramp; 2],
    powered_up: [bool; 2],
    dead: [bool; 2],
    switching: [bool; 2],
}

impl GaugePart {
    /// `HudGauge_Init` (then `HudGauge_Reset`).
    pub(crate) fn new(sheet: HudSheet) -> Self {
        let mut spr = [Sprite::default(); 76];
        let tex = |spr: &mut Sprite,
                   tex: usize,
                   sub: u16,
                   rect: [i32; 4],
                   uv: [i32; 4],
                   moves: &[[i32; 2]]| {
            spr.init_tex_sub(&sheet, tex, sub);
            spr.set_rect(rect[0], rect[1], rect[2], rect[3]);
            spr.set_uv(uv[0], uv[1], uv[2], uv[3]);
            for m in moves {
                spr.move_by(m[0], m[1]);
            }
        };
        spr[0].init_tex_sub(&sheet, 6, 0);
        let bar = [0, 0xA0, 0, 0x10];
        let (tip_uv, bar_uv) = ([0, 0xA0, 0x30, 0x40], [0, 0xA0, 0, 0x10]);
        for (i, sub, uv, x) in [
            (7, 0, tip_uv, 0xA0),
            (8, 1, bar_uv, 0),
            (5, 0, tip_uv, 0xA0),
            (6, 2, bar_uv, 0),
            (2, 0, tip_uv, 0xA0),
            (3, 1, bar_uv, 0),
            (1, 2, bar_uv, 0),
            (4, 0, bar_uv, 0),
        ] {
            tex(&mut spr[i], 8, sub, bar, uv, &[[x, 0]]);
        }
        for k in 0..6 {
            tex(
                &mut spr[9 + k],
                8,
                1,
                [0, 0x10, 0, 0x10],
                [0, 0x10, 0x10, 0x20],
                &[[9 * k as i32, 0]],
            );
        }
        for k in 0..5 {
            let x = [0x20, 0x18, 0x10, 8, 0][k];
            tex(
                &mut spr[15 + k],
                8,
                3,
                [0, 0xE, 0, 0x11],
                [0x10, 0x1E, 0x10, 0x21],
                &[[x, 0]],
            );
            let x = [0x1C, 0x15, 0xE, 7, 0][k];
            for (first, sub) in [(20, 2), (25, 1), (30, 5)] {
                tex(
                    &mut spr[first + k],
                    8,
                    sub,
                    [0, 0x10, 0, 0x11],
                    [0x10, 0x20, 0x10, 0x21],
                    &[[x, 0]],
                );
            }
        }
        tex(
            &mut spr[35],
            0xE,
            0,
            [-32, 0, 0, 0x20],
            [0, 0x20, 0, 0x20],
            &[[0xA, -6]],
        );
        tex(
            &mut spr[36],
            0xF,
            0,
            [0, 0x30, 0, 0xC],
            [0, 0x30, 0, 0xC],
            &[[0x1B, 0x2E]],
        );
        tex(
            &mut spr[37],
            8,
            0,
            [0, 0x1A, 0, 6],
            [0xC0, 0xDA, 0, 6],
            &[[5, 3]],
        );
        tex(
            &mut spr[38],
            8,
            1,
            [0, 0x1A, 0, 6],
            [0xA1, 0xBB, 0, 6],
            &[[5, 3]],
        );
        for k in 0..4 {
            let x = 0x18 * k as i32;
            tex(
                &mut spr[39 + k],
                0xF,
                0,
                [0, 0x2A, 0, 0x16],
                [0, 0x2A, 0x16, 0x2C],
                &[[x, 0], [0x66, 0x18]],
            );
            spr[43 + k].init_tex_sub(&sheet, 0x13 + k, 0);
            spr[43 + k].move_by(5, -5);
            spr[43 + k].move_by(x, 0);
            tex(
                &mut spr[49 + k],
                0xF,
                0,
                [3, 0x27, 1, 0x15],
                [0, 0x24, 0x2C, 0x40],
                &[[x, 0], [0x66, 0x18]],
            );
        }
        spr[47].init_tex_sub(&sheet, 7, 0);
        tex(
            &mut spr[48],
            0xF,
            0,
            [0, 0x30, 1, 0xB],
            [0, 0x30, 0xC, 0x16],
            &[[0x1B, 0x2E]],
        );
        for (k, uv) in AURA_INIT_UV.iter().enumerate() {
            tex(&mut spr[53 + k], 0x10, 0, [0, 0x40, 0, 0x40], *uv, &[]);
        }
        spr[64].show(true);
        spr[64].init_plain([0x80; 4]);
        spr[64].set_rect(0, 0x40, 0, 0x40);
        for k in 0..5 {
            let x = 8 * k as i32;
            spr[65 + k].show(false);
            tex(
                &mut spr[65 + k],
                0x11,
                0,
                [0, 0x13, 0, 0x16],
                [0, 0x13, 0, 0x16],
                &[[x, 0]],
            );
            spr[70 + k].show(false);
            tex(
                &mut spr[70 + k],
                0x11,
                0,
                [0, 0x13, 0, 0x16],
                [0x20, 0x33, 0, 0x16],
                &[[x, 0]],
            );
        }
        spr[75].show(true);
        tex(
            &mut spr[75],
            0x12,
            0,
            [-32, 0, 0, 0x20],
            [0, 0x20, 0, 0x20],
            &[[0xA, -6]],
        );
        Self {
            sheet,
            spr,
            side: 0,
            shake: [[(0, 0); 3]; 2],
            ofs: [[0; 2]; 3],
            root_pos: [0; 2],
            panel_pos: [-2, 8],
            hp_shown: [0; 2],
            hp: [0; 2],
            aura_alpha: [0; 2],
            low_hp_pulse: [Ramp::default(); 2],
            blast: [0; 2],
            blast_max: [0; 2],
            ki: [0; 2],
            ki_shown: [0; 2],
            ki_bar_flash: [Ramp::default(); 2],
            ki_full_pulse: [Ramp::default(); 2],
            max_power: [0; 2],
            blast_flash: [Ramp::default(); 2],
            stat_mask: [0; 2],
            stat_pos: [0; 2],
            stat_to: [[0; 4]; 2],
            ki_reserve: [0; 2],
            hp_bar_fade: [Ramp::default(); 2],
            slide: [Ramp::default(); 2],
            slide_all: Ramp::default(),
            paused: false,
            hp_drain: [0; 2],
            ki_drain: [0; 2],
            ki_reserve_pulse: [Ramp::default(); 2],
            powered_up: [false; 2],
            dead: [false; 2],
            switching: [false; 2],
        }
    }

    /// The `HudGauge_Set*` calls of `Hud_PreUpdate` for one side (no stat modifiers, no ki reserve yet).
    pub(crate) fn set_side(&mut self, side: usize, g: &SideGauge) {
        self.hp[side] = g.hp;
        // HudGauge_SetKi: gaining a bar restarts its lamp; reaching full ki restarts the pulse.
        if self.ki[side] / KI_BAR < g.ki / KI_BAR {
            self.ki_bar_flash[side].start(1.0, 1.0, 0.0);
            if g.ki > 99999 {
                self.ki_full_pulse[side].start(1.0, 0.0, 1.0);
            }
        }
        self.ki[side] = g.ki;
        self.ki_reserve[side] = 0;
        if self.blast[side] / BLAST_STOCK < g.blast / BLAST_STOCK {
            self.blast_flash[side].start(1.0, 1.0, 0.0);
        }
        self.blast[side] = g.blast;
        self.blast_max[side] = g.blast_max;
        self.max_power[side] = g.max_power;
        self.powered_up[side] = g.powered_up;
        self.dead[side] = g.dead;
    }

    /// `HudGauge_ShakeHp`: the panel and the health node shake `count` frames by up to `amp`.
    pub(crate) fn shake_hp(&mut self, side: usize, count: u16, amp: u16) {
        self.shake[side][0] = (count, amp);
        self.shake[side][1] = (count, amp);
    }

    /// `HudGauge_ShakeKi`: the panel and the ki node.
    pub(crate) fn shake_ki(&mut self, side: usize, count: u16, amp: u16) {
        self.shake[side][0] = (count, amp);
        self.shake[side][2] = (count, amp);
    }

    /// `HudGauge_SetSwitching`: the side's panel slides away (0.3 s) while it switches members, and back with
    /// its shown health and ki reset.
    pub(crate) fn set_switching(&mut self, side: usize, switching: bool) {
        if self.switching[side] != switching {
            if switching {
                self.slide[side].start(0.3, 0.0, 1.0);
            } else {
                self.slide[side].start(0.3, 1.0, 0.0);
                self.hp_shown[side] = 0;
                self.ki_shown[side] = 0;
            }
        }
        self.switching[side] = switching;
    }

    /// `HudGauge_SelectSide`, `HudNode_Update` and `HudNode_Draw` of the tree for one side.
    pub(crate) fn run(
        &mut self,
        side: usize,
        layer: &mut GsLayer,
        textures: &HudTextures,
        libc: &mut LibcRandom,
    ) {
        self.side = side;
        let flip = side != 0;
        for i in [35, 75, 43, 44, 45, 46] {
            self.spr[i].mirror = flip;
        }
        // Updates: root, panel, then the panel's children in order.
        if !self.paused && side == 0 {
            self.slide_all.step();
        }
        self.root_pos = [
            (self.slide_all.value * -256.0) as i32,
            (self.slide_all.value * -224.0) as i32,
        ];
        if !self.paused {
            self.slide[side].step();
        }
        let slide = self.slide[side].value;
        self.panel_pos = [(slide * -256.0) as i32 - 2, (slide * -224.0) as i32 + 8];
        self.update_aura(libc);
        self.update_hp_trail(libc);
        self.update_ki();
        self.update_ki_lamps();
        self.update_max_power();
        self.update_max_power_lamps();
        self.update_blast();
        self.update_stat_icons();
        // Draws.
        let mut paint = Painter {
            layer,
            textures,
            mirror: flip,
        };
        let root = self.root_pos;
        let panel = [
            root[0] + self.panel_pos[0] + self.ofs[0][0],
            root[1] + self.panel_pos[1] + self.ofs[0][1],
        ];
        let at =
            |pos: [i32; 2], ofs: [i32; 2]| [panel[0] + pos[0] + ofs[0], panel[1] + pos[1] + ofs[1]];
        for i in [0, 36, 39, 40, 41, 42] {
            paint.draw(&self.spr[i], panel, 0, None);
        }
        self.draw_aura(&mut paint, at([0, 0], [0, 0]));
        self.draw_hp_bar_count(&mut paint, at([73, 2], [0, 0]));
        self.draw_hp(&mut paint, at([69, 9], self.ofs[1]));
        let ki = at([64, 27], self.ofs[2]);
        for i in 15..30 {
            paint.draw(&self.spr[i], ki, 0, None);
        }
        for i in 65..70 {
            paint.draw(&self.spr[i], [ki[0] - 3, ki[1] - 3], 0, None);
        }
        let power = at([64, 27], [0, 0]);
        for i in 30..35 {
            paint.draw(&self.spr[i], power, 0, None);
        }
        for i in 70..75 {
            paint.draw(&self.spr[i], [power[0] - 3, power[1] - 3], 0, None);
        }
        self.draw_blast(&mut paint, at([29, 46], [0, 0]));
        for i in 43..47 {
            paint.draw(&self.spr[i], at([102, 24], [0, 0]), 0, None);
        }
        // HudGauge_DrawFace: texture `side` of the side's face sheet, mirrored for side 1.
        if textures.faces[side].is_some() {
            let face = &mut self.spr[64];
            if let Some(t) = &textures.faces[side] {
                let (w, h) = (t.width as i32, t.height as i32);
                face.tex = side as i16;
                face.set_rect(0, w, 0, h);
                face.set_uv(0, w, 0, h);
            }
            face.rgba = [0x80; 4];
            face.mirror = flip;
            paint.draw(&self.spr[64], at([16, 3], [0, 0]), 0, Some(side));
        }
    }

    /// `HudGauge_UpdateAura`: the powered-up sparks (a new random frame each, never the same twice running, and a
    /// jitter of +-5), the blue flash, and the red flash of low health.
    fn update_aura(&mut self, libc: &mut LibcRandom) {
        let side = self.side;
        if self.powered_up[side] && !self.dead[side] {
            self.aura_alpha[side] = (self.aura_alpha[side] + 0x10).min(0x80);
        } else {
            self.aura_alpha[side] = (self.aura_alpha[side] - 0x10).max(0);
        }
        for i in 0..10 {
            let spr = &mut self.spr[53 + i];
            let mut n;
            // While paused each spark shows its own frame, without jitter or random draws.
            if self.paused {
                n = i;
            } else {
                loop {
                    n = libc.int_range(0, 9) as usize;
                    if AURA_FRAMES[n] != spr.uv {
                        break;
                    }
                }
            }
            spr.uv = AURA_FRAMES[n];
            let ofs = if self.paused {
                [0, -8]
            } else {
                [libc.int_range(-5, 5), libc.int_range(-5, 5) - 8]
            };
            spr.set_rect(0, 0x40, 0, 0x40);
            spr.move_by((i % 5) as i32 * 46 + ofs[0], (i / 5) as i32 * 20 + ofs[1]);
            spr.rgba = [0x80, 0x80, 0x80, self.aura_alpha[side] as u8];
        }
        let blue = (self.aura_alpha[side] as f32 * 0.007_812_5 * 44.0) as u8;
        self.spr[62].init_plain([0, 0, 0xFF, blue]);
        self.spr[62].set_rect(0, 0x180, 0, 0x6C);
        let ramp = &mut self.low_hp_pulse[side];
        let paused = self.paused;
        if self.hp[side] <= HP_BAR && !self.dead[side] {
            if !paused && ramp.step() {
                if ramp.value == 0.0 {
                    ramp.start(0.5, 0.0, 1.0);
                } else {
                    ramp.start(0.5, 1.0, 0.0);
                }
            }
        } else if !paused && ramp.step() && ramp.value == 1.0 {
            ramp.start(0.5, 1.0, 0.0);
        }
        let red = (ramp.value * 44.0) as u8;
        self.spr[63].init_plain([0xFF, 0, 0, red]);
        self.spr[63].set_rect(0, 0x180, 0, 0x6C);
    }

    /// `HudGauge_UpdateHpTrail`: the shakes, then the trailing health falls towards the real one.
    fn update_hp_trail(&mut self, libc: &mut LibcRandom) {
        let side = self.side;
        for i in 0..3 {
            let (count, amp) = &mut self.shake[side][i];
            if *count != 0 {
                if self.paused {
                    continue;
                }
                *count -= 1;
                let amp = i32::from(*amp);
                self.ofs[i] = [libc.int_range(-amp, amp), libc.int_range(-amp, amp)];
            } else {
                self.ofs[i] = [0, 0];
            }
        }
        if self.paused {
            return;
        }
        self.hp_bar_fade[side].step();
        let (hp, shown) = (self.hp[side], self.hp_shown[side]);
        if hp < shown {
            let drain = &mut self.hp_drain[side];
            if hp < shown - 20000 {
                *drain = (*drain).max(200);
            } else if hp < shown - 10000 {
                *drain = (*drain).max(150);
            } else if hp < shown - 5000 {
                *drain = (*drain).max(120);
            }
            if shown > HP_BAR && shown % HP_BAR > 0 && shown % HP_BAR <= *drain {
                self.hp_bar_fade[side].start(0.2, 1.0, 0.0);
            }
            self.hp_shown[side] = (shown - *drain).max(hp);
        } else {
            self.hp_shown[side] = hp;
            self.hp_drain[side] = 100;
        }
    }

    /// A bar of 17 rows (`HudGauge_UpdateKi` / `UpdateMaxPower`): full, hidden, or cut from the top.
    fn bar(spr: &mut Sprite, value: i32, i: i32, unit: i32, width: i32) {
        if value >= i * unit + unit {
            spr.show(true);
            spr.set_rect(0, width, 0, 0x11);
            spr.set_uv(0x10, 0x10 + width, 0x10, 0x21);
        } else if i * unit >= value {
            spr.show(false);
        } else {
            let n = (value - i * unit) * 17 / unit;
            spr.show(true);
            spr.set_rect(0, width, 0x11 - n, 0x11);
            spr.set_uv(0x10, 0x10 + width, 0x21 - n, 0x21);
        }
        spr.move_by(i * 8, 0);
    }

    /// `HudGauge_UpdateKi`: the trailing ki (400 a frame, at least 606 / 800 for gaps over 20000 / 40000), then
    /// the bars of the trailing ki, the ki and the reserve.
    fn update_ki(&mut self) {
        let side = self.side;
        let (ki, extra) = (self.ki[side], self.ki_reserve[side]);
        let shown = self.ki_shown[side];
        let drain = &mut self.ki_drain[side];
        *drain = (*drain).max(if ki < shown - 40000 {
            800
        } else if ki < shown - 20000 {
            606
        } else {
            400
        });
        if !self.paused && ki < self.ki_shown[side] {
            self.ki_shown[side] -= *drain;
        }
        if ki >= self.ki_shown[side] {
            self.ki_shown[side] = ki;
            *drain = 400;
        }
        let shown = self.ki_shown[side];
        for i in 0..5 {
            Self::bar(&mut self.spr[20 + i], shown, i as i32, KI_BAR, 0xE);
            Self::bar(&mut self.spr[25 + i], ki, i as i32, KI_BAR, 0xE);
        }
        let pulse = &mut self.ki_reserve_pulse[side];
        if pulse.step() && extra != 0 {
            if pulse.value == 0.0 {
                pulse.start(0.3, 0.0, 1.0);
            } else {
                pulse.start(0.3, 1.0, 0.0);
            }
        }
        let red = (pulse.value * 192.0) as u8;
        for i in 0..5 {
            let spr = &mut self.spr[15 + i];
            spr.set_color(red, 0, 0, 0x40);
            Self::bar(spr, extra, i as i32, KI_BAR, 0xE);
        }
    }

    /// `HudGauge_UpdateMaxPower`: the powered-up timer's five bars, 6000 each.
    fn update_max_power(&mut self) {
        let v = self.max_power[self.side];
        for i in 0..5 {
            Self::bar(&mut self.spr[30 + i], v, i as i32, POWER_BAR, 0xE);
        }
    }

    /// `HudGauge_UpdateKiLamps`: the lamp of the bar just filled fades out; at full ki all five pulse.
    fn update_ki_lamps(&mut self) {
        let side = self.side;
        let ki = self.ki[side];
        let bars = ki / KI_BAR;
        // While paused: whether the flash is over, without stepping it.
        let paused = self.paused;
        let a = &mut self.ki_bar_flash[side];
        if if paused { a.at_target() } else { a.step() } {
            let b = &mut self.ki_full_pulse[side];
            if !paused && b.step() {
                if b.value == 0.0 {
                    b.start(1.0, 0.0, 1.0);
                } else {
                    b.start(1.0, 1.0, 0.0);
                }
            }
            if ki > 99999 {
                let alpha = (b.value * 128.0) as u8;
                for spr in &mut self.spr[65..70] {
                    spr.show(true);
                    spr.set_color(0x80, 0x80, 0x80, alpha);
                }
                return;
            }
        }
        let alpha = (self.ki_bar_flash[side].value * 128.0) as u8;
        for (i, spr) in self.spr[65..70].iter_mut().enumerate() {
            if bars == i as i32 + 1 {
                spr.show(true);
                spr.set_color(0x80, 0x80, 0x80, alpha);
            } else {
                spr.show(false);
            }
        }
    }

    /// `HudGauge_UpdateMaxPowerLamps`: a lamp per full bar of the powered-up timer, over the ki lamp.
    fn update_max_power_lamps(&mut self) {
        let side = self.side;
        let bars = self.max_power[side] / POWER_BAR;
        let alpha = (self.ki_full_pulse[side].value * 0.8 * 128.0) as u8;
        for i in 0..5 {
            if bars > i as i32 {
                self.spr[70 + i].show(true);
                self.spr[70 + i].set_color(0x80, 0x80, 0x80, alpha);
                self.spr[65 + i].show(false);
            } else {
                self.spr[70 + i].show(false);
            }
        }
    }

    /// `HudGauge_UpdateBlast`: the stock digit, its flash, and the fill towards the next stock.
    fn update_blast(&mut self) {
        let side = self.side;
        let (blast, max) = (self.blast[side], self.blast_max[side]);
        let n = (blast / BLAST_STOCK).min(7) as usize;
        let [u0, u1, v0, v1] = BLAST_DIGITS[n];
        for i in [35, 75] {
            let spr = &mut self.spr[i];
            spr.set_rect(-0x20, 0, 0, 0x20);
            spr.set_uv(u0, u1, v0, v1);
            spr.move_by(10, -16);
        }
        let paused = self.paused;
        let ramp = &mut self.blast_flash[side];
        if !paused && ramp.step() {
            if blast >= max {
                if ramp.value == 0.0 {
                    ramp.start(1.0, 0.0, 1.0);
                } else {
                    ramp.start(1.0, 1.0, 0.0);
                }
            } else if ramp.value == 1.0 {
                ramp.start(1.0, 1.0, 0.0);
            }
        }
        let flash = ramp.value;
        self.spr[75].show(flash > 0.0);
        self.spr[75].set_color(0x80, 0x80, 0x80, (flash * 128.0) as u8);
        let fill = (blast % BLAST_STOCK) * 25 / BLAST_STOCK;
        self.spr[37].set_rect(fill, fill + 0x1A, 0, 6);
        self.spr[37].move_by(5, 3);
        for i in [36, 38, 37, 48] {
            self.spr[i].show(blast < max);
        }
    }

    /// `HudGauge_UpdateStatIcons` at rest (state -1): the shown icons bright or dark, in their places.
    fn update_stat_icons(&mut self) {
        let side = self.side;
        let (mask, pos, to) = (
            self.stat_mask[side],
            self.stat_pos[side],
            self.stat_to[side],
        );
        for i in 0..4 {
            let c = if pos & (1 << i) != 0 {
                [0x80, 0x80, 0x80, 0x80]
            } else {
                [0x40, 0x40, 0x40, 0x80]
            };
            let shown = mask & (1 << i) != 0;
            let icon = &mut self.spr[43 + i];
            icon.init_tex_sub(&self.sheet, 0x13 + i, 0);
            icon.show(shown);
            icon.rgba = c;
            icon.move_by(5, -5);
            icon.move_by(to[i], 0);
            let frame = &mut self.spr[39 + i];
            frame.show(shown);
            frame.set_color(0x80, 0x80, 0x80, 0x80);
            frame.set_rect(0, 0x2A, 0, 0x16);
            frame.set_uv(0, 0x2A, 0x16, 0x2C);
            frame.move_by(0x66, 0x18);
            frame.move_by(to[i], 0);
            let shade = &mut self.spr[49 + i];
            shade.show(shown);
            shade.set_rect(3, 0x27, 1, 0x15);
            shade.set_uv(0, 0x24, 0x2C, 0x40);
            shade.move_by(0x66, 0x18);
            shade.move_by(to[i], 0);
        }
    }

    /// `HudGauge_DrawAura`: the panel's silhouette (sprite 47) and pieces into the alpha plane, the red flash and
    /// the sparks added inside it.
    fn draw_aura(&self, paint: &mut Painter, origin: [i32; 2]) {
        paint.alpha_clear(&self.spr[47], origin);
        paint.begin_mask2();
        for i in [47, 48, 49, 50, 51, 52] {
            paint.draw(&self.spr[i], origin, 0, None);
        }
        paint.draw(&self.spr[63], origin, 1, None);
        for i in 53..63 {
            paint.draw(&self.spr[i], origin, 1, None);
        }
        paint.end_mask2();
        paint.alpha_clear(&self.spr[47], origin);
    }

    /// `HudGauge_DrawHpBarCount` with `HudGauge_UpdateHpBarCount`: one marker per full bar in reserve.
    fn draw_hp_bar_count(&mut self, paint: &mut Painter, origin: [i32; 2]) {
        let side = self.side;
        let mut bars = self.hp_shown[side] / HP_BAR;
        if self.hp_shown[side] % HP_BAR == 0 && bars != 0 {
            bars -= 1;
        }
        let fade = self.hp_bar_fade[side].value;
        for i in 0..6 {
            let spr = &mut self.spr[9 + i];
            spr.set_color(0x80, 0x80, 0x80, 0x80);
            if (i as i32) < bars {
                spr.show(true);
            } else if i as i32 == bars && fade != 0.0 {
                spr.show(true);
                spr.set_color(0x80, 0x80, 0x80, (fade * 128.0) as u8);
            } else {
                spr.show(false);
            }
        }
        for i in 9..15 {
            paint.draw(&self.spr[i], origin, 0, None);
        }
    }

    /// `HudGauge_UpdateHp` (hud_gauge_1.c): the trailing bar, the bar, and the one under them, each 160 pixels
    /// for 10000; a bar exactly full is the top bar.
    fn update_hp(&mut self) {
        let side = self.side;
        let (shown, hp) = (self.hp_shown[side], self.hp[side]);
        let split = |v: i32| {
            let (rem, bars) = (v % HP_BAR, v / HP_BAR);
            if rem == 0 && bars != 0 {
                (HP_BAR, bars - 1)
            } else {
                (rem, bars)
            }
        };
        let (rem, bars) = split(shown);
        if shown != hp {
            self.spr[5].rect = self.spr[6].rect;
            let w = rem * 160 / HP_BAR;
            if w < 3 && rem != 0 && hp != 0 && bars == 0 {
                self.spr[5].move_by(3, 0);
            } else {
                self.spr[5].move_by(w, 0);
            }
            self.spr[6].show(true);
            self.spr[5].show(true);
        } else {
            self.spr[5].show(false);
            self.spr[6].show(false);
        }
        let (rem2, bars2) = split(hp);
        self.spr[7].rect = self.spr[8].rect;
        let mut w = rem2 * 160 / HP_BAR;
        if bars2 < bars {
            w = 0;
        }
        if w < 3 && rem2 != 0 && bars == 0 {
            self.spr[7].move_by(3, 0);
        } else {
            self.spr[7].move_by(w, 0);
        }
        self.spr[8].show(true);
        self.spr[7].show(true);
        let colour = |b: i32| -> u16 {
            if b >= 7 {
                5
            } else if b >= 2 {
                1
            } else if b > 0 {
                4
            } else {
                3
            }
        };
        self.spr[8].set_tex(8, colour(bars));
        self.spr[4].show(true);
        if bars == bars2 {
            self.spr[1].show(false);
            self.spr[2].show(false);
            self.spr[3].show(true);
        } else {
            self.spr[1].show(true);
            self.spr[2].show(true);
            self.spr[3].show(true);
            self.spr[2].rect = self.spr[3].rect;
            if bars == bars2 + 1 {
                self.spr[2].move_by(rem2 * 160 / HP_BAR, 0);
            }
        }
        if bars >= 1 {
            self.spr[3].set_tex(8, colour(bars - 1));
        } else {
            self.spr[3].show(false);
            self.spr[4].show(false);
        }
    }

    /// `HudGauge_DrawHp`: three layers, each through the alpha mask of its tip.
    fn draw_hp(&mut self, paint: &mut Painter, origin: [i32; 2]) {
        paint.alpha_clear(&self.spr[8], origin);
        self.update_hp();
        for group in [[1, 2, 3], [4, 5, 6]] {
            paint.begin_mask();
            paint.draw(&self.spr[group[0]], origin, 0, None);
            paint.draw(&self.spr[group[1]], origin, 1, None);
            paint.draw(&self.spr[group[2]], origin, 0, None);
            paint.end_mask();
            paint.alpha_clear(&self.spr[8], origin);
        }
        paint.begin_mask();
        paint.draw(&self.spr[7], origin, 1, None);
        paint.draw(&self.spr[8], origin, 0, None);
        paint.end_mask();
    }

    /// `HudGauge_DrawBlast`: the fill through the mask of its frame, then the digit and its flash.
    fn draw_blast(&self, paint: &mut Painter, origin: [i32; 2]) {
        paint.alpha_clear(&self.spr[38], origin);
        paint.begin_mask();
        paint.draw(&self.spr[37], origin, 1, None);
        paint.draw(&self.spr[38], origin, 0, None);
        paint.end_mask();
        paint.draw(&self.spr[35], origin, 0, None);
        paint.draw(&self.spr[75], origin, 0, None);
    }
}

/// `HudGauge_UpdateAura`'s ten frames of the spark texture.
const AURA_FRAMES: [[i16; 4]; 10] = [
    [0x00, 0x40, 0x00, 0x40],
    [0x40, 0x80, 0x00, 0x40],
    [0x80, 0xC0, 0x00, 0x40],
    [0xC0, 0x100, 0x00, 0x40],
    [0x00, 0x40, 0x40, 0x80],
    [0x40, 0x80, 0x40, 0x80],
    [0x80, 0xC0, 0x40, 0x80],
    [0xC0, 0x100, 0x40, 0x80],
    [0x00, 0x40, 0x80, 0xC0],
    [0x40, 0x80, 0x80, 0xC0],
];
/// `HudGauge_Init`'s first frames of sprites 53..62.
const AURA_INIT_UV: [[i32; 4]; 10] = [
    [0, 0x40, 0, 0x40],
    [0x40, 0x80, 0, 0x40],
    [0x80, 0xC0, 0, 0x40],
    [0xC0, 0x100, 0, 0x40],
    [0, 0x40, 0x40, 0x80],
    [0x40, 0x80, 0x40, 0x80],
    [0x80, 0xC0, 0x40, 0x80],
    [0xC0, 0x100, 0x40, 0x80],
    [0x40, 0x80, 0, 0x40],
    [0x80, 0xC0, 0, 0x40],
];
/// `HudGauge_UpdateBlast`'s digits 0..7.
const BLAST_DIGITS: [[i32; 4]; 8] = [
    [0x00, 0x20, 0x00, 0x20],
    [0x20, 0x40, 0x00, 0x20],
    [0x40, 0x60, 0x00, 0x20],
    [0x60, 0x80, 0x00, 0x20],
    [0x00, 0x20, 0x20, 0x40],
    [0x20, 0x40, 0x20, 0x40],
    [0x40, 0x60, 0x20, 0x40],
    [0x60, 0x80, 0x20, 0x40],
];

/// `gHudTimer`: the clock at the top of the screen.
pub(crate) struct TimerPart {
    spr: [Sprite; 6],
    value: i32,
    prev: i32,
    pulse: Ramp,
    slide: Ramp,
    /// `BATTLE_FLAG_PAUSE`: the slide and the pulse stand still.
    pub(crate) paused: bool,
}

impl TimerPart {
    /// `HudTimer_Init` (then `HudTimer_Reset`).
    pub(crate) fn new(sheet: &HudSheet) -> Self {
        let mut spr = [Sprite::default(); 6];
        spr[0].init_tex_sub(sheet, 3, 0);
        spr[0].set_rect(0, 64, 0, 30);
        spr[0].set_uv(0, 64, 0, 30);
        spr[0].center();
        spr[0].move_by(0, -7);
        for (i, rect) in [(1, [-10, 22]), (2, [-22, 10]), (3, [-22, 10])] {
            spr[i].init_tex_sub(sheet, 0, 0);
            spr[i].set_rect(rect[0], rect[1], -16, 16);
            spr[i].set_uv(0, 32, 0, 32);
        }
        spr[5].init_tex_sub(sheet, 5, 0);
        spr[5].center();
        spr[5].move_by(0, 16);
        spr[4].init_tex_sub(sheet, 4, 0);
        spr[4].center();
        spr[4].show(false);
        spr[4].move_by(0, 26);
        Self {
            spr,
            value: 0,
            prev: 0,
            pulse: Ramp::default(),
            slide: Ramp::default(),
            paused: false,
        }
    }

    /// `HudTimer_SetTime`.
    pub(crate) fn set_time(&mut self, seconds: i32) {
        self.prev = self.value;
        self.value = seconds;
        let frame = &mut self.spr[0];
        frame.set_rect(0, 64, 0, 30);
        frame.set_uv(0, 64, 0, 30);
        frame.center();
        frame.move_by(0, -7);
        self.spr[4].show(false);
    }

    /// The root's and the clock's updates, then the clock's draw.
    pub(crate) fn run(&mut self, layer: &mut GsLayer, textures: &HudTextures) {
        if !self.paused {
            self.slide.step();
        }
        let root = [0, (self.slide.value * -224.0) as i32];
        self.update_digits();
        let mut paint = Painter {
            layer,
            textures,
            mirror: false,
        };
        for spr in &self.spr {
            paint.draw(spr, [root[0] + 256, root[1] + 37], 0, None);
        }
    }

    /// `HudTimer_UpdateDigits`: up to three digits 11 apart, centred, or the two halves of the infinity mark.
    fn update_digits(&mut self) {
        let value = self.value.min(999);
        let digit = |spr: &mut Sprite, d: i32| {
            spr.set_uv(
                (d % 4) * 32,
                (d % 4) * 32 + 32,
                (d / 4) * 32,
                (d / 4) * 32 + 32,
            );
        };
        if value < 0 {
            let s = &mut self.spr[1];
            s.show(true);
            s.set_rect(0, 32, -32, 0);
            s.set_uv(96, 128, 64, 96);
            s.move_by(1, 0);
            let s = &mut self.spr[2];
            s.show(true);
            s.set_rect(-32, 0, -32, 0);
            s.set_uv(64, 96, 64, 96);
            s.move_by(1, 0);
            self.spr[3].show(false);
        } else {
            let mut x = 0;
            let d = value / 100;
            if d == 0 {
                self.spr[3].show(false);
            } else {
                self.spr[3].show(true);
                self.spr[3].set_rect(0, 32, -32, 0);
                digit(&mut self.spr[3], d);
                x = 11;
            }
            let d = value % 100 / 10;
            if x == 0 && d == 0 {
                self.spr[2].show(false);
            } else {
                self.spr[2].show(true);
                self.spr[2].set_rect(x, x + 32, -32, 0);
                digit(&mut self.spr[2], d);
                x += 11;
            }
            self.spr[1].show(true);
            self.spr[1].set_rect(x, x + 32, -32, 0);
            digit(&mut self.spr[1], value % 10);
            let w = -(x + 32) / 2;
            for i in 1..4 {
                self.spr[i].move_by(w, 0);
            }
        }
        if (0..4).contains(&value) {
            if value != self.prev {
                self.pulse.start(0.5, 1.0, 0.0);
            }
            let scale = self.pulse.value + 1.0;
            for i in 1..4 {
                let s = &mut self.spr[i];
                s.set_tex(0, 2);
                s.move_by(0, 16);
                s.scale(scale, scale);
                s.move_by(0, -16);
            }
            if !self.paused {
                self.pulse.step();
            }
        } else {
            let sub = if (0..10).contains(&value) { 1 } else { 0 };
            for i in 1..4 {
                self.spr[i].set_tex(0, sub);
            }
        }
        for i in 1..4 {
            self.spr[i].move_by(-2, 4);
        }
        self.spr[5].show(false);
    }
}

/// `Hud_Draw` for the team panel, the gauges and the timer: `HudGfx_SetEnv`, then the team panel of each side
/// (`team`: outside story mode), per side `HudGfx_ClearAlpha` and the gauge tree, then the clock. With `shown`
/// false the parts update but nothing is drawn.
pub(crate) fn draw_gauges(
    layer: &mut GsLayer,
    shown: bool,
    team: Option<&mut crate::hud_team::TeamPart>,
    gauge: Option<&mut GaugePart>,
    timer: Option<&mut TimerPart>,
    textures: &HudTextures,
    libc: &mut LibcRandom,
) {
    layer.begin(shown);
    if let Some(team) = team {
        for side in 0..2 {
            team.run(side, layer, textures);
        }
    }
    if let Some(gauge) = gauge {
        for side in 0..2 {
            layer.clear_alpha();
            gauge.run(side, layer, textures, libc);
        }
    }
    if let Some(timer) = timer {
        timer.run(layer, textures);
    }
}
