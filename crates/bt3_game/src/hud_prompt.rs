//! The HUD's prompt part (src/battle/hud_prompt.c): the button prompt (node 24: a pulsing base, an animated button
//! icon of the font's icon sheet or the part's own animated picture, a glow dropping onto it) and the command row
//! (node 26: up to four icons side by side with their glows, and enlarged copies when the command is accepted).
//! `Hud_PreUpdate` asks for them per side from the fighter's flags and technique; outside split screen only side
//! 0's are updated and drawn. Between them the cue (node 25, side 0 only): the pad's icon on a pulsing base, which
//! the story script shows while one of its events waits to start.
//!
//! Not ported: the technique names above the command row, which only the prompts of pad status 1 ask for (a PS2
//! pad reports status 0).

use crate::battle_seq::Ramp;
use crate::hud::{HudBlend, HudPrim, HudSheet, Sprite, Xf, fan, sprite_polygon};

/// The boot file, whose header word 8 is the font's icon file.
pub(crate) const BOOT_FILE: u32 = 5;

/// `gHudPromptIconDefs` (16), `gHudPromptIconDefs16` (17) and `gFontIconDefs`, 12 bytes each.
const OWN_DEFS: u32 = 0x2C_6268;
const OWN_DEFS16: u32 = 0x2C_6328;
const FONT_DEFS: u32 = 0x2C_6838;
/// `gHudPromptCueDefs`: the cue's icon by pad type (4).
const CUE_DEFS: u32 = 0x2C_63F8;
/// The font table entries the button table can name (up to 70).
const FONT_DEF_COUNT: usize = 0x50;

/// `HudPromptIconDef`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct IconDef {
    pub tex: u8,
    pub cell: u8,
    /// Width of the picture in its cell, minus 1.
    pub width: u8,
    /// The first frame of the animation's second half.
    pub pressed: u8,
    pub frames: u16,
    pub time: [i8; 6],
}

impl IconDef {
    fn parse(b: &[u8]) -> Self {
        Self {
            tex: b[0],
            cell: b[1],
            width: b[2],
            pressed: b[3],
            frames: u16::from_le_bytes([b[4], b[5]]),
            time: std::array::from_fn(|k| b[6 + k] as i8),
        }
    }
}

/// The icon definitions the part reads from the executable.
#[derive(Clone, Debug, Default)]
pub(crate) struct PromptDefs {
    pub own: Vec<IconDef>,
    pub own16: Vec<IconDef>,
    pub font: Vec<IconDef>,
    pub cue: Vec<IconDef>,
}

impl PromptDefs {
    pub(crate) fn read(elf: &bt3_data::elf::Elf) -> std::io::Result<Self> {
        let table = |at: u32, n: usize| -> std::io::Result<Vec<IconDef>> {
            Ok(elf
                .read(at, n * 12)?
                .chunks_exact(12)
                .map(IconDef::parse)
                .collect())
        };
        Ok(Self {
            own: table(OWN_DEFS, 16)?,
            own16: table(OWN_DEFS16, 17)?,
            cue: table(CUE_DEFS, 4)?,
            font: table(FONT_DEFS, FONT_DEF_COUNT)?,
        })
    }

    /// `HudPrompt_ApplyIcon`'s lookup: 0..15 and 16..32 in the part's tables, the font's above.
    fn by_icon(&self, icon: i32) -> IconDef {
        let i = icon.max(0) as usize;
        let d = if i < 16 {
            self.own.get(i)
        } else if i < 33 {
            self.own16.get(i - 16)
        } else {
            self.font.get(i)
        };
        d.copied().unwrap_or_default()
    }

    fn font(&self, i: i32) -> IconDef {
        self.font
            .get(i.max(0) as usize)
            .copied()
            .unwrap_or_default()
    }
}

/// `HudPromptIcon`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Icon {
    pub icon: i32,
    /// -1: the run ended (the last frame stays).
    pub frame: i32,
    pub timer: i32,
    pub mode: i32,
    pub last: i32,
    pub restart: bool,
}

/// What `Hud_PreUpdate` asks of a side's button prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonReq {
    /// `HudPrompt_SetButton(button, group)`.
    Set(i32, i32),
    Clear,
}

/// What it asks of the side's command row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommandReq {
    Accept,
    Set {
        icons: Vec<i32>,
        modes: Vec<i32>,
    },
    Clear,
    /// Nothing while paused.
    Keep,
}

/// A side's prompt requests of this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SidePrompt {
    pub button: ButtonReq,
    pub command: CommandReq,
}

impl Default for SidePrompt {
    fn default() -> Self {
        Self {
            button: ButtonReq::Clear,
            command: CommandReq::Keep,
        }
    }
}

/// Sprites: 0 base, 1 icon, 2 second picture, 3 glow (node 24); 4 base, 5 icon (node 25); 6..9 the row's icons,
/// 10..13 their glows, 14..17 / 18..21 the enlarged copies (node 26).
const SPR_COUNT: usize = 22;
const BUTTON_NODE: [i32; 2] = [64, 384];
const CUE_NODE: [i32; 2] = [256, 384];
const COMMAND_NODE: [i32; 2] = [60, 400];
/// `HudPrompt_UpdateButton`'s icon table: row = group, column = button.
const BUTTON_ICONS: [i32; 16] = [
    22, 21, 20, 23, 54, 55, 69, 70, 37, 43, 41, 39, 40, 44, 38, 42,
];

/// `gHudPrompt`.
#[derive(Debug)]
pub(crate) struct PromptPart {
    sheet: HudSheet,
    icons: HudSheet,
    defs: PromptDefs,
    spr: [Sprite; SPR_COUNT],
    /// Which sprites draw from the icon sheet.
    from_icons: [bool; SPR_COUNT],
    btn_state: [i32; 2],
    btn_pulse: [Ramp; 2],
    btn_ramp: [Ramp; 2],
    btn_blink: [Ramp; 2],
    btn_group: [i32; 2],
    btn_value: [i32; 2],
    btn_icon: [Icon; 2],
    btn_flip: [bool; 2],
    btn_kind: [i32; 2],
    /// 0 on screen .. 1 away (`HudPrompt_SlideOut` / `SlideIn`).
    slide: Ramp,
    /// `cueState`: 0 start, 1 fading in, 2 shown, 3 fading out, 4 / -1 off.
    cue_state: i32,
    cue_ramp: Ramp,
    cue_pulse: Ramp,
    /// `cueDim`: the fighter cannot act; half alpha, no pulse.
    cue_dim: bool,
    cmd_on: [bool; 2],
    cmd_state: [i32; 2],
    row: [[Icon; 4]; 2],
    row_count: [usize; 2],
    cmd_ramp: [Ramp; 2],
    cmd_blink: [Ramp; 2],
    side: usize,
    pub(crate) paused: bool,
}

impl PromptPart {
    /// `HudPrompt_Init`: `sheet` is HUD sheet 3, `icons` the font's icon sheet.
    pub(crate) fn new(sheet: HudSheet, icons: HudSheet, defs: PromptDefs) -> Self {
        let mut spr = [Sprite::default(); SPR_COUNT];
        for (i, tex) in [(0, 0), (4, 8), (1, 1), (5, 1), (2, 1), (3, 7)] {
            spr[i].init_tex_sub(&sheet, tex, 0);
            spr[i].center();
            spr[i].show(false);
        }
        for s in &mut spr[6..14] {
            s.set_color(0xFF, 0xFF, 0xFF, 0x80);
            s.set_rect(0, 32, 0, 32);
            s.show(false);
        }
        Self {
            sheet,
            icons,
            defs,
            spr,
            from_icons: [false; SPR_COUNT],
            btn_state: [-1; 2],
            btn_pulse: [Ramp::default(); 2],
            btn_ramp: [Ramp::default(); 2],
            btn_blink: [Ramp::default(); 2],
            btn_group: [0; 2],
            btn_value: [0; 2],
            btn_icon: [Icon::default(); 2],
            btn_flip: [false; 2],
            btn_kind: [0; 2],
            slide: Ramp::default(),
            cue_state: -1,
            cue_ramp: Ramp::default(),
            cue_pulse: Ramp::default(),
            cue_dim: false,
            cmd_on: [false; 2],
            cmd_state: [-1; 2],
            row: [[Icon::default(); 4]; 2],
            row_count: [0; 2],
            cmd_ramp: [Ramp::default(); 2],
            cmd_blink: [Ramp::default(); 2],
            side: 0,
            paused: false,
        }
    }

    /// `Ramp_Step` unless paused (0 then).
    fn step(paused: bool, ramp: &mut Ramp) -> bool {
        !paused && ramp.step()
    }

    /// `HudPrompt_ApplyIconDef`: advances an icon's animation (mode 0 the frames before `pressed`, 1 from it on, 3
    /// frame 0, 4 none, else all; a frame lasts `time[frame]` counts of the timer; a time of 0, frame 6 or the
    /// end stops the run on its last frame, which restarts at once with `restart`, else at a timer of 20) and puts
    /// its cell on the sprite.
    fn apply_def(spr: &mut Sprite, def: &IconDef, icon: &mut Icon) {
        let pressed = i32::from(def.pressed).min(i32::from(def.frames));
        let all = i32::from(def.frames) + 1;
        let (lo, hi) = match icon.mode {
            0 => (0, pressed),
            4 => (pressed, pressed),
            1 => (pressed, all),
            3 => (0, 0),
            _ => (0, all),
        };
        let mut frame = icon.frame;
        if frame >= 0 && frame < lo {
            icon.frame = lo;
            frame = lo;
            icon.timer = 0;
        }
        if icon.restart && frame < 0 {
            icon.frame = lo;
            frame = lo;
            icon.timer = 0;
        }
        if frame < 0 {
            if icon.timer >= 20 {
                icon.frame = lo;
                frame = lo;
                icon.timer = 0;
            }
        } else if icon.timer >= i32::from(def.time.get(frame as usize).copied().unwrap_or(0)) {
            let next = frame + 1;
            icon.last = frame;
            icon.frame = next;
            icon.timer = 0;
            let ended =
                next >= 6 || next >= hi || def.time.get(next as usize).copied().unwrap_or(0) == 0;
            if ended {
                icon.frame = -1;
                frame = -1;
            } else {
                frame = next;
            }
        }
        let f = if frame < 0 { icon.last } else { frame };
        let cell = i32::from(def.cell);
        let w = (i32::from(def.width) + 1).min(cell);
        let (x, y) = ((f % 2) * cell, (f / 2) * cell);
        spr.set_tex(i16::from(def.tex), 0);
        spr.set_rect(x, x + w, y, y + cell);
        spr.center();
        spr.set_uv(x, x + w, y, y + cell);
    }

    /// `HudPrompt_SetButton`: `button` = column + 4 * style (0 pressed and released, 1 held, 2 released with the
    /// glow) or 12 for the animated picture; ignored while the same prompt shows or the row is active.
    pub(crate) fn set_button(&mut self, side: usize, button: i32, group: i32) {
        self.btn_group[side] = group;
        if (1..39).contains(&self.btn_state[side]) && self.btn_kind[side] == button {
            return;
        }
        if self.cmd_state[side] >= 0 {
            return;
        }
        self.btn_kind[side] = button;
        match button {
            0..=11 => {
                self.btn_state[side] = [1, 4, 7][button as usize / 4];
                self.btn_value[side] = [0, 1, 2, 3][button as usize % 4];
            }
            12 => {
                if group != 1 {
                    self.spr[1].init_tex_sub(&self.sheet, 1, 0);
                    self.btn_value[side] = 1;
                    self.spr[1].center();
                    self.btn_state[side] = 10;
                }
            }
            _ => {}
        }
        self.btn_flip[side] = false;
    }

    /// `HudPrompt_ClearButton`: the 0.28 s fade-out of a prompt on show.
    pub(crate) fn clear_button(&mut self, side: usize) {
        if (1..39).contains(&self.btn_state[side]) {
            self.btn_state[side] = 40;
        }
    }

    /// `HudPrompt_SetCommand`: a row of icons with their animation modes; ignored while the button prompt is
    /// active or the same row shows. An icon below 16 with mode 2 restarts at once.
    pub(crate) fn set_command(&mut self, side: usize, icons: &[i32], modes: &[i32]) {
        let count = icons.len().min(4);
        if count == 0 || self.btn_state[side] > 0 {
            return;
        }
        if self.cmd_on[side] {
            let same = (0..count)
                .all(|i| icons[i] == self.row[side][i].icon && modes[i] == self.row[side][i].mode);
            if same {
                return;
            }
        }
        if !self.cmd_on[side] {
            self.cmd_state[side] = 0;
        }
        self.cmd_on[side] = true;
        self.row_count[side] = count;
        for i in 0..4 {
            self.row[side][i] = if i < count {
                Icon {
                    icon: icons[i],
                    frame: 0,
                    timer: 0,
                    mode: modes[i],
                    last: 0,
                    restart: (0..16).contains(&icons[i]) && modes[i] == 2,
                }
            } else {
                Icon::default()
            };
        }
    }

    /// `HudPrompt_ClearCommand`: a 0.3 s fade-out; at once while paused.
    pub(crate) fn clear_command(&mut self, side: usize) {
        if self.cmd_on[side] {
            self.cmd_on[side] = false;
            self.cmd_state[side] = if self.paused { 9 } else { 3 };
        }
    }

    /// `HudPrompt_AcceptCommand`: the row's "accepted" animation.
    pub(crate) fn accept_command(&mut self, side: usize) {
        if self.cmd_on[side] {
            self.cmd_on[side] = false;
            self.cmd_state[side] = 5;
        }
    }

    /// `HudPrompt_UpdateButton` for the selected side.
    fn update_button(&mut self) {
        let side = self.side;
        let paused = self.paused;
        let mut alpha = 1.0f32;
        let sheet = &self.sheet;
        let [base, icon, icon2, glow, ..] = &mut self.spr;
        base.init_tex_sub(sheet, 0, 0);
        base.center();
        glow.init_tex_sub(sheet, 7, 0);
        glow.center();
        glow.move_by(0, -24);
        icon.show(true);
        icon2.show(false);
        base.show(true);
        glow.show(true);
        let mut node_shown = true;
        let state = &mut self.btn_state[side];
        let ramp = &mut self.btn_ramp[side];
        let pulse = &mut self.btn_pulse[side];
        let blink = &mut self.btn_blink[side];
        let group = self.btn_group[side];
        let set_icon = |value: i32, defs: &PromptDefs, icon: &mut Icon| {
            let k = (group * 4 + value).clamp(0, 15) as usize;
            icon.icon = i32::from(defs.font(BUTTON_ICONS[k]).tex);
        };
        let start = |ramp: &mut Ramp, pulse: &mut Ramp, blink: &mut Ramp| {
            ramp.start(0.28, 0.0, 1.0);
            pulse.start(0.1, 1.0, 0.0);
            blink.start(1.5, 1.0, 0.0);
        };
        let value = self.btn_value[side];
        let bicon = &mut self.btn_icon[side];
        match *state {
            1..=3 => {
                glow.show(false);
                if *state == 1 {
                    start(ramp, pulse, blink);
                    *state = 2;
                    set_icon(value, &self.defs, bicon);
                    bicon.timer = 0;
                    bicon.restart = true;
                    bicon.mode = 2;
                }
                set_icon(value, &self.defs, bicon);
                if *state == 2 {
                    if Self::step(paused, ramp) {
                        *state = 3;
                    }
                    alpha = ramp.value;
                } else if !paused {
                    bicon.timer += 1;
                }
            }
            4..=6 => {
                if *state == 4 {
                    start(ramp, pulse, blink);
                    *state = 5;
                    set_icon(value, &self.defs, bicon);
                    bicon.timer = 0;
                    bicon.frame = 1;
                    bicon.restart = true;
                }
                set_icon(value, &self.defs, bicon);
                if *state == 5 {
                    if Self::step(paused, ramp) {
                        *state = 6;
                    }
                    alpha = ramp.value;
                }
            }
            7..=9 => {
                if *state == 7 {
                    start(ramp, pulse, blink);
                    *state = 8;
                    set_icon(value, &self.defs, bicon);
                    bicon.timer = 0;
                    bicon.frame = 0;
                    bicon.restart = true;
                }
                set_icon(value, &self.defs, bicon);
                if *state == 8 {
                    if Self::step(paused, ramp) {
                        *state = 9;
                    }
                    alpha = ramp.value;
                } else {
                    if !paused && blink.step() {
                        blink.start(1.5, 1.0, 0.0);
                    }
                    let drop = blink.value * 2.6 - 0.8;
                    bicon.frame = if 0.9 < drop { 1 } else { 0 };
                    let d = drop.min(1.0).max(0.0);
                    glow.move_by(0, ((1.0 - d) * -16.0) as i32);
                }
            }
            10 | 11 => {
                glow.show(false);
                if *state == 10 {
                    ramp.start(0.28, 0.0, 1.0);
                    pulse.start(0.1, 1.0, 0.0);
                    icon.tex = self.btn_value[side] as i16;
                    *state = 11;
                }
                if Self::step(paused, ramp) {
                    self.btn_value[side] += 1;
                    icon.tex = self.btn_value[side] as i16;
                    *state = 12;
                }
                alpha = ramp.value;
            }
            12..=19 => {
                if *state % 2 == 0 {
                    ramp.start(0.03, 0.0, 1.0);
                    *state += 1;
                }
                glow.show(false);
                if Self::step(paused, ramp) {
                    self.btn_value[side] += 1;
                    icon.tex = self.btn_value[side] as i16;
                    *state += 1;
                    if *state == 20 {
                        self.btn_flip[side] = true;
                    }
                }
            }
            20..=27 => {
                if *state % 2 == 0 {
                    ramp.start(0.03, 0.0, 1.0);
                    *state += 1;
                }
                glow.show(false);
                if Self::step(paused, ramp) {
                    self.btn_value[side] -= 1;
                    icon.tex = self.btn_value[side] as i16;
                    *state += 1;
                    if *state == 28 {
                        *state = 12;
                        self.btn_flip[side] = false;
                    }
                }
            }
            40 | 41 => {
                if *state == 40 {
                    ramp.start(0.28, 1.0, 0.0);
                    *state = 41;
                }
                glow.show(false);
                if Self::step(paused, ramp) {
                    *state = 42;
                }
                alpha = ramp.value;
            }
            _ => {
                node_shown = false;
                for s in [&mut *icon, &mut *icon2, &mut *base, &mut *glow] {
                    s.show(false);
                }
                self.btn_kind[side] = 13;
                *state = 0;
            }
        }
        let kind = self.btn_kind[side];
        self.from_icons[1] = kind != 12;
        if kind != 12 {
            let def = self.defs.font(self.btn_icon[side].icon);
            Self::apply_def(icon, &def, &mut self.btn_icon[side]);
        } else if group != 1 {
            icon.init_tex_sub(sheet, self.btn_value[side].max(0) as usize, 0);
            icon.center();
        } else {
            let v = self.btn_value[side];
            let off = if side == 0 { -14 } else { 14 };
            icon.init_tex_sub(sheet, (v >> 8).max(0) as usize, 0);
            icon.center();
            icon.move_by(off, 0);
            icon.show(true);
            icon2.init_tex_sub(sheet, (v & 0xFF) as usize, 0);
            icon2.center();
            icon2.move_by(-off + if side == 0 { -2 } else { 2 }, 0);
            icon2.show(true);
        }
        if Self::step(paused, pulse) {
            if pulse.value == 1.0 {
                pulse.start(0.1, 1.0, 0.0);
            } else if pulse.value == 0.0 {
                pulse.start(0.1, 0.0, 1.0);
            }
        }
        let mut t = pulse.value * 0.4 + 1.2;
        if kind == 12 {
            t *= 1.2;
        }
        base.scale(t, t);
        let t = if kind != 12 { 1.44 } else { 1.0 };
        icon.scale(t, t);
        let a = (alpha * 128.0) as u8;
        icon.set_color(0x80, 0x80, 0x80, a);
        icon2.set_color(0x80, 0x80, 0x80, a);
        base.set_color(0x80, 0x80, 0x80, a);
        let drop = blink.value * 2.6 - 0.8;
        if drop < 1.0 {
            alpha *= drop;
        }
        let g = alpha * alpha.max(0.0);
        glow.set_color(0x80, 0x80, 0x80, (g * 128.0) as u8);
        let mirror = self.btn_flip[side] ^ (side != 0);
        icon.mirror = mirror;
        icon2.mirror = mirror;
        base.move_by(1, -2);
        if !node_shown {
            for s in [icon, icon2, base, glow] {
                s.show(false);
            }
        }
    }

    /// `HudPrompt_UpdateCommand` for the selected side.
    fn update_command(&mut self) {
        let side = self.side;
        let paused = self.paused;
        let count = self.row_count[side];
        for i in 0..4 {
            let glow = &mut self.spr[10 + i];
            glow.init_tex_sub(&self.sheet, 7, 0);
            glow.center();
            glow.move_by(0, -22);
        }
        let glow_shown = |icon: &Icon| icon.icon < 16 && (0..2).contains(&icon.mode);
        for i in 0..4 {
            if i < count {
                self.spr[6 + i].show(true);
                let g = glow_shown(&self.row[side][i]);
                self.spr[10 + i].show(g);
            } else {
                for k in [6, 10, 14, 18] {
                    self.spr[k + i].show(false);
                }
            }
        }
        let state = &mut self.cmd_state[side];
        let ramp = &mut self.cmd_ramp[side];
        let blink = &mut self.cmd_blink[side];
        let icons = &mut self.row[side];
        match *state {
            0 | 1 => {
                if *state == 0 {
                    for (i, icon) in icons.iter_mut().enumerate().take(count) {
                        icon.timer = 0;
                        icon.frame = 0;
                        self.spr[6 + i].show(true);
                    }
                    ramp.start(0.1, 0.0, 1.0);
                    blink.start(1.5, 1.0, 0.0);
                    *state = 1;
                }
                if Self::step(paused, ramp) {
                    *state = 2;
                }
            }
            2 => {
                if Self::step(paused, blink) {
                    blink.start(1.5, 1.0, 0.0);
                }
                if !paused {
                    for icon in icons.iter_mut().take(count) {
                        if !(icon.icon < 16 && (icon.mode == 0 || icon.mode == 1)) {
                            icon.timer += 1;
                        }
                    }
                }
            }
            3 => {
                ramp.start(0.3, 1.0, 0.0);
                *state = 4;
                if Self::step(paused, ramp) {
                    *state = 9;
                }
            }
            5 | 6 => {
                if *state == 5 {
                    blink.start(0.8, 1.0, 0.0);
                    *state = 6;
                }
                if Self::step(paused, blink) {
                    ramp.start(0.5, 1.0, 0.0);
                    *state = 7;
                }
            }
            4 | 7 => {
                if Self::step(paused, ramp) {
                    *state = 9;
                }
            }
            _ => {
                if *state == 9 {
                    *state = -1;
                }
                for i in 0..count {
                    self.spr[6 + i].show(false);
                }
                for i in 0..4 {
                    self.spr[10 + i].show(false);
                }
            }
        }
        let drop = (blink.value * 2.6 - 0.8).max(0.0).min(1.0);
        for (i, icon) in icons.iter_mut().enumerate().take(count) {
            if icon.icon < 16 {
                let glow = &mut self.spr[10 + i];
                if icon.mode == 1 {
                    glow.set_color(0x80, 0x80, 0x80, (ramp.value * 128.0) as u8);
                } else if icon.mode == 0 {
                    icon.frame = if 0.9 < drop { 1 } else { 0 };
                    glow.move_by(0, ((1.0 - drop) * -16.0) as i32);
                    glow.set_color(0x80, 0x80, 0x80, (drop * ramp.value * 128.0) as u8);
                }
            }
        }
        let accepted = (5..7).contains(state);
        let mut x = 0;
        for i in 0..count {
            let def = self.defs.by_icon(icons[i].icon);
            Self::apply_def(&mut self.spr[6 + i], &def, &mut icons[i]);
            if accepted {
                let s = (1.0 - blink.value) * 1.5 + 1.0;
                let a = (blink.value * 128.0) as u8;
                self.spr[14 + i] = self.spr[6 + i];
                self.spr[18 + i] = self.spr[10 + i];
                for k in [14, 18] {
                    let g = &mut self.spr[k + i];
                    g.scale(s, s);
                    g.set_color(0x80, 0x80, 0x80, a);
                }
            } else {
                self.spr[14 + i].show(false);
                self.spr[18 + i].show(false);
            }
            for k in [6, 10, 14, 18] {
                self.spr[k + i].move_by(x, 0);
            }
            if i + 1 < count {
                let r = self.spr[6 + i].rect;
                x += i32::from(r[1] - r[0]);
            }
            self.spr[6 + i].set_color(0x80, 0x80, 0x80, (ramp.value * 128.0) as u8);
        }
        if side != 0 {
            for i in 0..count {
                for k in [6, 10, 14, 18] {
                    self.spr[k + i].move_by(-(COMMAND_NODE[0] * 2) - x + 512, 0);
                }
            }
        }
    }

    /// The part's update and draw for one side (`HudPrompt_SelectSide`, `HudNode_Update`, `HudNode_Draw`): the
    /// root's slide (stepped on the side 0 pass), the button prompt (mirrored for side 1), then the row.
    pub(crate) fn run(&mut self, side: usize, out: &mut Vec<HudPrim>) {
        self.side = side;
        if !self.paused && side == 0 {
            self.slide.step();
        }
        let root = [
            (self.slide.value * -256.0) as i32,
            (self.slide.value * 224.0) as i32,
        ];
        self.update_button();
        self.update_cue();
        self.update_command();
        let at =
            |node: [i32; 2]| Xf::translate((root[0] + node[0]) as f32, (root[1] + node[1]) as f32);
        let button = at(BUTTON_NODE);
        let button = if side != 0 { button.mirrored() } else { button };
        for i in 0..4 {
            self.push(out, i, &button);
        }
        // HudPrompt_DrawCue: side 0 only.
        if side == 0 {
            let cue = at(CUE_NODE);
            for i in 4..6 {
                self.push(out, i, &cue);
            }
        }
        let command = at(COMMAND_NODE);
        for i in 0..4 {
            for k in [6, 10, 14, 18] {
                self.push(out, k + i, &command);
            }
        }
    }

    fn push(&self, out: &mut Vec<HudPrim>, i: usize, xf: &Xf) {
        let spr = &self.spr[i];
        if spr.hidden {
            return;
        }
        let icon_sheet = matches!(i, 5..=9 | 14..=17) || (i == 1 && self.from_icons[1]);
        let sheet = if icon_sheet { &self.icons } else { &self.sheet };
        let mut poly = sprite_polygon(spr, xf, sheet);
        // HudSprite_SetMirror: u0 and u1 change places.
        if spr.mirror {
            let (a, b) = (poly[0].uv, poly[1].uv);
            poly[0].uv = b;
            poly[1].uv = a;
            let (c, d) = (poly[2].uv, poly[3].uv);
            poly[2].uv = d;
            poly[3].uv = c;
        }
        out.push(HudPrim {
            texture: usize::try_from(spr.tex).ok().map(|t| sheet.base + t),
            vertices: fan(&poly),
            rgba: spr.rgba,
            blend: HudBlend::Alpha,
        });
    }

    /// `HudPrompt_ShowCue`: restarts the cue unless it is coming in or shown.
    pub(crate) fn show_cue(&mut self) {
        if !(0..3).contains(&self.cue_state) {
            self.cue_state = 0;
        }
    }

    /// `HudPrompt_HideCue`: the 0.28 s fade out if it is coming in or shown.
    pub(crate) fn hide_cue(&mut self) {
        if (0..3).contains(&self.cue_state) {
            self.cue_ramp.start(0.28, 1.0, 0.0);
            self.cue_state = 3;
        }
    }

    /// `HudPrompt_SetCueDim`.
    pub(crate) fn set_cue_dim(&mut self, can_act: bool) {
        self.cue_dim = !can_act;
    }

    /// `HudPrompt_SlideOut` / `HudPrompt_SlideIn`.
    pub(crate) fn slide(&mut self, out: bool, seconds: f32) {
        if out {
            self.slide.start(seconds, 0.0, 1.0);
        } else {
            self.slide.start(seconds, 1.0, 0.0);
        }
    }

    /// `HudPrompt_UpdateCue` (side 0): the icon of the pad's type (status 0 here) on base texture 8, fading in over
    /// 0.28 s, shown, fading out; the base pulses between 1.44 and 1.92 times its size every 0.1 s, or with `cue_dim`
    /// stays at 1.2 with half the alpha.
    fn update_cue(&mut self) {
        if self.side != 0 {
            return;
        }
        let paused = self.paused;
        let def = self.defs.cue.first().copied().unwrap_or_default();
        let w = i32::from(def.width + 1).min(i32::from(def.cell));
        let cell = i32::from(def.cell);
        let icon = &mut self.spr[5];
        icon.init_tex_sub(&self.icons, usize::from(def.tex), 0);
        icon.set_rect(0, w, 0, cell);
        icon.set_uv(0, w, 0, cell);
        icon.center();
        icon.show(true);
        let base = &mut self.spr[4];
        base.init_tex_sub(&self.sheet, 8, 0);
        base.center();
        base.show(true);
        let mut alpha = 1.0;
        if self.cue_state == 0 {
            self.cue_ramp.start(0.28, 0.0, 1.0);
            self.cue_pulse.start(0.1, 1.0, 0.0);
            self.cue_state = 1;
        }
        match self.cue_state {
            1 => {
                if Self::step(paused, &mut self.cue_ramp) {
                    self.cue_state = 2;
                }
                alpha = self.cue_ramp.value;
            }
            2 => {}
            3 => {
                if Self::step(paused, &mut self.cue_ramp) {
                    self.cue_state = 4;
                }
                alpha = self.cue_ramp.value;
            }
            _ => {
                self.spr[4].show(false);
                self.spr[5].show(false);
                self.cue_state = 4;
            }
        }
        if Self::step(paused, &mut self.cue_pulse) {
            if self.cue_pulse.value == 1.0 {
                self.cue_pulse.start(0.1, 1.0, 0.0);
            } else if self.cue_pulse.value == 0.0 {
                self.cue_pulse.start(0.1, 0.0, 1.0);
            }
        }
        let s = if self.cue_dim {
            1.2
        } else {
            (self.cue_pulse.value * 0.4 + 1.2) * 1.2
        };
        if self.cue_dim {
            alpha *= 0.5;
        }
        let a = (alpha * 128.0) as u8;
        let base = &mut self.spr[4];
        base.scale(s, s);
        base.set_color(0x80, 0x80, 0x80, a);
        base.move_by(1, -2);
        let icon = &mut self.spr[5];
        icon.scale(1.2, 1.2);
        icon.move_by(2, 0);
        icon.set_color(0x80, 0x80, 0x80, a);
    }

    pub(crate) fn set_defs(&mut self, defs: PromptDefs) {
        self.defs = defs;
    }

    /// `Hud_PreUpdate`'s prompt calls for one side.
    pub(crate) fn set_side(&mut self, side: usize, req: &SidePrompt) {
        match req.button {
            ButtonReq::Set(button, group) => self.set_button(side, button, group),
            ButtonReq::Clear => self.clear_button(side),
        }
        match &req.command {
            CommandReq::Accept => self.accept_command(side),
            CommandReq::Set { icons, modes } => self.set_command(side, icons, modes),
            CommandReq::Clear => self.clear_command(side),
            CommandReq::Keep => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn state(&self, side: usize) -> (i32, i32) {
        (self.btn_state[side], self.cmd_state[side])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> PromptDefs {
        let def = |tex, width, pressed, frames, time: [i8; 6]| IconDef {
            tex,
            cell: 32,
            width,
            pressed,
            frames,
            time,
        };
        let mut font = vec![IconDef::default(); FONT_DEF_COUNT];
        for i in 20..24 {
            font[i] = def(i as u8, 20, 2, 2, [3, 3, 0, 0, 0, 0]);
        }
        PromptDefs {
            own: (0..16)
                .map(|i| def(20 + i % 4, 23, 1, 2, [3, 3, 0, 0, 0, 0]))
                .collect(),
            own16: (0..17).map(|i| def(4 + i % 4, 16, 1, 1, [0; 6])).collect(),
            font,
            cue: vec![def(24, 20, 0, 0, [0; 6]); 4],
        }
    }

    fn part() -> PromptPart {
        let sheet = |base| HudSheet {
            base,
            sizes: vec![[64, 64]; 64],
            views: Vec::new(),
        };
        PromptPart::new(sheet(0), sheet(100), defs())
    }

    #[test]
    fn cue_fades_in_pulses_dims_and_fades_out() {
        let mut p = part();
        let mut out = Vec::new();
        // Off until shown.
        p.run(0, &mut out);
        assert!(p.spr[4].hidden && p.spr[5].hidden);
        p.show_cue();
        // The fighter cannot act: dim.
        p.set_cue_dim(false);
        p.run(0, &mut out);
        assert_eq!(p.cue_state, 1);
        assert!(!p.spr[4].hidden && !p.spr[5].hidden);
        // 0.28 s in; then shown, dim: half alpha.
        for _ in 0..9 {
            p.run(0, &mut out);
        }
        assert_eq!(p.cue_state, 2);
        assert_eq!(p.spr[4].rgba[3], 64);
        p.set_cue_dim(true);
        p.run(0, &mut out);
        assert_eq!(p.spr[4].rgba[3], 128);
        // Side 1's pass leaves it alone.
        p.hide_cue();
        p.run(1, &mut out);
        assert_eq!(p.cue_state, 3);
        for _ in 0..10 {
            p.run(0, &mut out);
        }
        p.run(0, &mut out);
        assert_eq!(p.cue_state, 4);
        assert!(p.spr[4].hidden);
    }

    #[test]
    fn icon_frames_play_and_stop() {
        let def = IconDef {
            tex: 22,
            cell: 32,
            width: 20,
            pressed: 2,
            frames: 2,
            time: [3, 3, 0, 0, 0, 0],
        };
        let mut spr = Sprite::default();
        let mut icon = Icon {
            mode: 2,
            restart: true,
            ..Icon::default()
        };
        let mut frames = Vec::new();
        for _ in 0..9 {
            PromptPart::apply_def(&mut spr, &def, &mut icon);
            frames.push(spr.uv[0]);
            icon.timer += 1;
        }
        // Frame 0 (u 0) for three counts, frame 1 (u 32) for three; the run ends on it (shown once more) and
        // restarts.
        assert_eq!(frames, [0, 0, 0, 32, 32, 32, 32, 0, 0]);
        assert_eq!((spr.rect[1] - spr.rect[0]), 21);
    }

    #[test]
    fn button_prompt_fades_in_and_out() {
        let mut p = part();
        let mut out = Vec::new();
        // The struggle's mash prompt (flag 0xE6): the part's own animated picture.
        p.set_side(
            0,
            &SidePrompt {
                button: ButtonReq::Set(12, 0),
                command: CommandReq::Keep,
            },
        );
        p.run(0, &mut out);
        assert_eq!(p.state(0).0, 11);
        assert!(out.iter().any(|q| q.texture == Some(1)));
        for _ in 0..12 {
            out.clear();
            p.set_side(
                0,
                &SidePrompt {
                    button: ButtonReq::Set(12, 0),
                    command: CommandReq::Keep,
                },
            );
            p.run(0, &mut out);
        }
        assert!((12..=27).contains(&p.state(0).0));
        // Taken away: 0.28 s fade, then hidden.
        for _ in 0..12 {
            out.clear();
            p.set_side(0, &SidePrompt::default());
            p.run(0, &mut out);
        }
        assert_eq!(p.state(0).0, 0);
        assert!(out.is_empty());
        // A held button (style 1): the font's icon 21 for column 1, at 1.44 times its cell.
        p.set_side(
            0,
            &SidePrompt {
                button: ButtonReq::Set(5, 0),
                command: CommandReq::Keep,
            },
        );
        out.clear();
        p.run(0, &mut out);
        assert_eq!(p.btn_icon[0].icon, 21);
        assert!(out.iter().any(|q| q.texture == Some(121)));
    }

    #[test]
    fn command_row_lays_out_its_icons_and_is_accepted() {
        let mut p = part();
        let mut out = Vec::new();
        // Up, plus, triangle: the switch prompt of row 0.
        let req = SidePrompt {
            button: ButtonReq::Clear,
            command: CommandReq::Set {
                icons: vec![0x10, 0x20, 0],
                modes: vec![2, 2, 3],
            },
        };
        for _ in 0..5 {
            out.clear();
            p.set_side(0, &req);
            p.run(0, &mut out);
        }
        assert_eq!(p.state(0).1, 2);
        let xs: Vec<i16> = (6..9).map(|i| p.spr[i].rect[0]).collect();
        assert!(xs[0] < xs[1] && xs[1] < xs[2], "{xs:?}");
        // While the row shows, a button prompt is not started (the idle prompt rests at state 0).
        p.set_button(0, 0, 0);
        assert_eq!(p.state(0).0, 0);
        // Accepted: the enlarged copies grow and fade (0.8 s), then the row fades out (0.5 s).
        p.set_side(
            0,
            &SidePrompt {
                button: ButtonReq::Clear,
                command: CommandReq::Accept,
            },
        );
        out.clear();
        p.run(0, &mut out);
        assert!(!p.spr[14].hidden);
        let mut frames = 1;
        while p.state(0).1 != -1 && frames < 100 {
            out.clear();
            p.set_side(0, &SidePrompt::default());
            p.run(0, &mut out);
            frames += 1;
        }
        assert_eq!(frames, 40);
        assert!(out.is_empty());
    }
}
