//! Disc Fusion (`DiscFusion_Run(6)`, src/menu/disc_fusion.c, progress mode 24, `Ub_Main`): two plates, one per
//! earlier Budokai Tenkaichi disc. A plate whose disc was recognised this session goes on to its mode (Ultimate
//! Battle Z, modes 25..30); for one that was not, a dialog walks through the disc swap (take this disc out, put the
//! other in, have it identified, the "recognised" movie, put this disc back), following the drive's state.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::dialog::{Command, Dialog, DialogAssets, Pad};
use crate::disc::{self, DiscDrive};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::{GameRandom, LibcRandom};
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::ub::{BGM_FILE_ID, VOICE_BASE};
use crate::windows::{IconWin, MsgWin, TextBox, TextDraw, draw_texts};

/// `DiscFusion_Run(6)`: section 6 of `gMenuArc3`.
pub const ARCHIVE_SECTION: usize = 6;
const FADE_FRAMES: i32 = 0x14;
const DISC_NUM: usize = 2;
const IDLE_FRAMES: i32 = 0x708;
/// `UB_DISC_0`, `UB_DISC_1` of `gProgress->ubFlags`: the disc was recognised this session.
pub const DISC_0: i32 = 1;
pub const DISC_1: i32 = 2;

// The flags (`UBRES_*` bits).
const DONE: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const FADED_IN: u32 = 8;

// DISCFUSION_DLG_*.
const DLG_ASK: i32 = 1;
const DLG_CONFIRM: i32 = 2;
const DLG_OPEN: i32 = 3;
const DLG_INSERT: i32 = 4;
const DLG_READ: i32 = 5;
const DLG_FOUND: i32 = 6;
const DLG_FOUND_ANIM: i32 = 7;
const DLG_BACK_OPEN: i32 = 8;
const DLG_BACK_INSERT: i32 = 9;
const DLG_BACK_READ: i32 = 10;
const DLG_WRONG: i32 = 11;
const DLG_WRONG_OPEN: i32 = 12;
const DLG_BACK_WRONG: i32 = 13;
const DLG_CLOSE: i32 = 14;

/// `DISCFUSION_ID_OWN`, `DISCFUSION_TRIES`, `DISCFUSION_TIMEOUT`.
const ID_OWN: i32 = 2;
const TRIES: i32 = 0x40;
const TIMEOUT: i32 = 0xF0;

/// `texA[n]` (the screen) and `texB[n]` (the "recognised" movie) of `DiscFusion_Init`.
const TEX_A: [ImageTexture; 18] = [
    tex(3, 0),
    tex(3, 1),
    tex(3, 8),
    tex(3, 7),
    tex(3, 13),
    tex(3, 14),
    tex(3, 15),
    tex(3, 9),
    None,
    tex(3, 11),
    tex(3, 10),
    tex(3, 6),
    tex(9, 0),
    tex(9, 1),
    tex(9, 3),
    tex(3, 16),
    tex(3, 17),
    tex(3, 18),
];
const TEX_B: [ImageTexture; 4] = [tex(3, 5), tex(3, 3), tex(3, 4), tex(3, 2)];
/// `IconWin_Init(MPACK_AT(res, 6), res 4)` and `MsgWin_Init(MPACK_AT(res, 7), …)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((6, 1), 1)),
    Some(((6, 1), 0)),
    tex(4, 0),
    Some(((6, 1), 2)),
    tex(4, 1),
];
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((7, 1), 0)), Some(((7, 1), 1)), None];
const DIALOG_SECTION: usize = 5;

/// What `DiscFusion_Init` reads from its unpacked section.
pub struct DiscFusionAssets {
    pub data: DiscFusionData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct DiscFusionData {
    pub fods: [Vec<u8>; 2],
    pub icon_fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    /// Section 10: the guide's lines; 11: their mouth keys; 12: the plates' text and the dialog's messages.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    pub text: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    pub layout: DiscFusionLayout,
}

#[derive(Clone, Debug, Default)]
pub struct DiscFusionLayout {
    pub background: usize,
    pub flash: [Vec<Option<usize>>; 2],
    pub icon: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
}

impl DiscFusionLayout {
    pub fn new(
        background: usize,
        dialog: Vec<Option<usize>>,
        first: impl Fn(FileKey) -> usize,
    ) -> Self {
        let i = |t: &[ImageTexture]| image_indices(t, &first);
        Self {
            background,
            flash: [i(&TEX_A), i(&TEX_B)],
            icon: i(&ICON_TEXTURES),
            msg: i(&MSG_TEXTURES),
            dialog,
        }
    }
}

impl DiscFusionAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&TEX_A[..], &TEX_B, &ICON_TEXTURES, &MSG_TEXTURES])?;
        let dialog = DialogAssets::parse(pack_section(res, DIALOG_SECTION)?)?;
        let sec =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        Ok(Self {
            data: DiscFusionData {
                fods: [sec(1)?, sec(2)?],
                icon_fod: pack_section(pack_section(res, 6)?, 2)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 7)?, 2)?.to_vec(),
                msg_text: sec(10)?,
                lips: sec(11)?,
                text: sec(12)?,
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                layout: DiscFusionLayout::default(),
            },
            background: decode_picture(res, 8)?,
            files,
            dialog,
        })
    }
}

/// `DiscFusion` and the frame loop of `DiscFusion_Run`.
pub struct DiscFusionScreen {
    data: DiscFusionData,
    flash: [Flash; 2],
    icon: IconWin,
    msg: MsgWin,
    dialog: Dialog,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    cursor: i32,
    timer: i32,
    talk_step: i32,
    talk_prev: i32,
    skip: bool,
    dlg_step: i32,
    voice_line: i32,
    blink: [i32; 2],
    talk: [i32; 2],
    have: [bool; DISC_NUM],
    chosen: i32,
    tries: i32,
    idle: i32,
    toggle: i32,
    wait: i32,
    timeout: i32,
    error: bool,
    /// `gProgress->ubFlags` as the screen leaves it.
    ub_flags: i32,
    result: i32,
    done: Option<i32>,
}

impl DiscFusionScreen {
    /// `DiscFusion_Init(6)` and the first `ColorFade_StartIn`; `ub_flags` is `gProgress->ubFlags`.
    pub fn new(
        data: &DiscFusionData,
        ub_flags: i32,
        libc: &mut LibcRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let make = |fod: &[u8]| -> Result<Flash, Box<dyn Error>> {
            let mut f = Flash::create(fod)?;
            f.play(1);
            Ok(f)
        };
        let flash = [make(&data.fods[0])?, make(&data.fods[1])?];
        let blink = [0; 2].map(|_| libc.rand() % 32);
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            false,
        )?;
        msg.open();
        let dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            Some(data.text.clone()),
            0,
        )?;
        let mut screen = Self {
            data: data.clone(),
            flash,
            icon,
            msg,
            dialog,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            cursor: 0,
            timer: 0,
            talk_step: 0,
            talk_prev: 0,
            skip: false,
            dlg_step: 0,
            voice_line: -1,
            blink,
            talk: [0; 2],
            have: [false; DISC_NUM],
            chosen: 0,
            tries: 0,
            idle: 0,
            toggle: 0,
            wait: 0,
            timeout: 0,
            error: false,
            ub_flags,
            result: 1,
            done: None,
        };
        screen.update_have();
        Ok(screen)
    }

    /// `DiscFusion_Run`'s result once it has returned: 1 a recognised disc's plate was chosen, 0 backed out.
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// The plate chosen (`gProgress->cursor`) and `gProgress->ubFlags`.
    pub fn progress(&self) -> (i32, i32) {
        (self.cursor, self.ub_flags)
    }

    /// `DiscFusion_UpdateHave`.
    fn update_have(&mut self) {
        self.have = [self.ub_flags & DISC_0 != 0, self.ub_flags & DISC_1 != 0];
    }

    /// `DiscFusion_PlateGoto(0, 0, label)`.
    fn plate_goto(&mut self, label: &str) {
        let r = self.flash[0].find(None, &format!("mc_disk_plate{:02}", self.cursor));
        self.flash[0].clip_goto_label(r, label);
    }

    /// One iteration of `DiscFusion_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        drive: &mut DiscDrive,
        random: &mut GameRandom,
        font: &mut Font,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done.is_some() {
            return frame;
        }
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        self.update(voice_stat);
        let mut effects = Vec::new();
        self.update_disc(pressed, repeat, drive, &mut effects, &mut frame.sounds);
        self.update_talk(pressed, voice_stat, &mut frame.sounds);
        self.draw(voice_stat, random, font, &mut frame.quads);
        frame.sounds.extend(
            effects
                .into_iter()
                .map(|(bank, id)| TitleSound::Effect(bank, id)),
        );
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & FADED_IN == 0
            && self.flash[0].flags() & FLASH_PAD != 0
        {
            self.flags |= FADED_IN;
        }
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::FadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = Some(self.result);
            return frame;
        }
        if self.flags & LEAVING != 0 {
            if self.timer == 0 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        } else if self.dlg_step == 0 && self.talk_step == 0 {
            self.input(pressed, repeat, &mut frame.sounds);
        }
        frame
    }

    /// `DiscFusion_Update`: the leave timer; after the greeting, the line of the plate under the cursor.
    fn update(&mut self, voice_stat: i32) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.flags & FADED_IN != 0
            && self.dlg_step == 0
            && (self.voice_line == 0x40 || self.voice_line == 0x41)
            && voice_stat == VOICE_PLAYEND
        {
            self.talk_step = self.cursor + 3;
        }
        for f in &mut self.flash {
            f.advance();
        }
    }

    /// One look at the disc in the drive while the dialog waits for it (`DiscFusion_Identify`).
    fn read_disc(&mut self, drive: &mut DiscDrive) -> Option<i32> {
        if self.wait != 0 {
            self.wait -= 1;
            return None;
        }
        match drive.identify() {
            disc::BUSY => None,
            id @ (0..=2) => Some(id),
            _ => Some(-1),
        }
    }

    /// `DiscFusion_UpdateDisc`: the disc swap, a dialog driven by the drive's state.
    fn update_disc(
        &mut self,
        pressed: u32,
        repeat: u32,
        drive: &mut DiscDrive,
        effects: &mut Vec<(u32, u32)>,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.dlg_step == 0 {
            return;
        }
        let state = drive.state();
        let stopped = state == disc::STOPPED;
        let open = state == disc::TRAY_OPEN;
        let paused = state == disc::PAUSED;
        let no_type = state == disc::NO_DISC;
        let pad = Pad { pressed, repeat };
        match self.dlg_step {
            DLG_ASK => {
                self.dialog.set_choices(true);
                self.dialog.set_cursor(1);
                self.dialog.start(Command::Open, effects);
                self.dlg_step = DLG_CONFIRM;
                sounds.push(TitleSound::FadeOutStep);
            }
            DLG_CONFIRM => {
                self.dialog.set_msg(self.chosen + 0x14);
                let answer = self.dialog.input(false, pad, effects);
                if answer < 0 {
                    self.dlg_step = DLG_CLOSE;
                    self.dialog.start(Command::Close, effects);
                } else if answer > 0 {
                    self.dialog.set_choices(false);
                    self.dlg_step = DLG_OPEN;
                }
            }
            DLG_OPEN => {
                self.dialog.set_msg(0x16);
                if open {
                    self.dlg_step = DLG_INSERT;
                }
            }
            DLG_INSERT => {
                self.dialog.set_msg(self.chosen + 0x17);
                if stopped {
                    self.dlg_step = DLG_READ;
                    self.dialog.set_msg(0x19);
                    self.tries = 0;
                    self.timeout = TIMEOUT;
                }
            }
            DLG_READ | DLG_BACK_READ => {
                let back = self.dlg_step == DLG_BACK_READ;
                if back {
                    self.dialog.set_msg(0x19);
                }
                if paused && let Some(id) = self.read_disc(drive) {
                    let want = if back { ID_OWN } else { self.chosen };
                    if id == want {
                        if back {
                            self.dlg_step = DLG_CLOSE;
                        } else {
                            self.dlg_step = DLG_FOUND;
                        }
                        self.dialog.start(Command::Close, effects);
                        return;
                    }
                    if self.tries == TRIES {
                        if back {
                            self.dlg_step = DLG_BACK_WRONG;
                        } else {
                            self.dialog.set_choices(true);
                            self.dialog.set_cursor(1);
                            self.dlg_step = DLG_WRONG;
                        }
                        self.error = false;
                        return;
                    }
                    self.tries += 1;
                    self.wait = 3;
                }
                if open {
                    self.dlg_step = if back { DLG_BACK_INSERT } else { DLG_INSERT };
                } else if no_type {
                    if self.timeout == 0 {
                        if back {
                            self.dlg_step = DLG_BACK_WRONG;
                        } else {
                            self.dialog.set_choices(true);
                            self.dialog.set_cursor(1);
                            self.dlg_step = DLG_WRONG;
                        }
                        self.error = true;
                    } else {
                        self.timeout -= 1;
                    }
                }
            }
            DLG_FOUND => {
                self.ub_flags |= if self.chosen == 0 { DISC_0 } else { DISC_1 };
                self.update_have();
                self.flash[1].goto_label("fl_start", true);
                self.dlg_step = DLG_FOUND_ANIM;
            }
            DLG_FOUND_ANIM => {
                if self.flash[1].flags() & FLASH_PAD != 0 {
                    self.dialog.start(Command::Open, effects);
                    self.dialog.set_msg(0x1B);
                    self.dlg_step = DLG_BACK_OPEN;
                }
            }
            DLG_BACK_OPEN => {
                if open {
                    self.dlg_step = DLG_BACK_INSERT;
                }
            }
            DLG_BACK_INSERT => {
                self.dialog.set_msg(0x1C);
                if stopped {
                    self.dlg_step = DLG_BACK_READ;
                    self.tries = 0;
                    self.timeout = TIMEOUT;
                }
            }
            DLG_WRONG => {
                self.dialog.set_msg(if self.error { 0x1E } else { 0x1A });
                let answer = self.dialog.input(false, pad, effects);
                if answer < 0 {
                    self.dlg_step = DLG_OPEN;
                    self.dialog.set_choices(false);
                } else if answer > 0 {
                    self.dialog.set_choices(false);
                    self.dlg_step = DLG_WRONG_OPEN;
                }
            }
            DLG_WRONG_OPEN => {
                self.dialog.set_msg(0x16);
                if open {
                    self.dlg_step = DLG_BACK_INSERT;
                }
            }
            DLG_BACK_WRONG => {
                self.dialog.set_msg(if self.error { 0x1F } else { 0x1D });
                if open {
                    self.dlg_step = DLG_BACK_INSERT;
                }
            }
            DLG_CLOSE => {
                if self.dialog.is_closed() {
                    // Adx_StopAll, then the music again.
                    sounds.push(TitleSound::StopAll);
                    sounds.push(TitleSound::Bgm(BGM_FILE_ID));
                    self.dlg_step = 0;
                    let bit = if self.chosen == 0 { DISC_0 } else { DISC_1 };
                    self.talk_step = if self.ub_flags & bit != 0 { 6 } else { 7 };
                }
            }
            _ => {}
        }
    }

    /// `DiscFusion_UpdateTalk`: the guide's lines.
    fn update_talk(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if !self.skip {
            if self.talk_step == 0 {
                return;
            }
            if self.voice_line != -1
                && voice_stat != VOICE_PLAYEND
                && self.talk_step == self.talk_prev
            {
                if pressed & PAD_CROSS != 0 {
                    sounds.push(TitleSound::Effect(1, 1));
                    self.skip = true;
                }
                return;
            }
        } else {
            self.skip = false;
        }
        match self.talk_step {
            1 => self.voice_line = 0x40,
            2 => self.voice_line = 0x41,
            3 => self.voice_line = if self.have[0] { 0x52 } else { 0x42 },
            4 => self.voice_line = if self.have[1] { 0x53 } else { 0x43 },
            5 => self.voice_line = 0x46,
            6 => self.voice_line = 0x47,
            7 => self.voice_line = 0x48,
            _ => {}
        }
        self.talk_step = if self.talk_step == 6 {
            self.cursor + 3
        } else {
            0
        };
        anim::play_with_subtitle(
            &self.data.lips,
            VOICE_BASE,
            self.voice_line,
            &mut self.lip,
            sounds,
        );
        self.talk_prev = self.talk_step;
    }

    /// `DiscFusion_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        quads.extend(
            picture_quads(self.data.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let mut texts: Vec<TextDraw> = Vec::new();
        let plain = TextBox::preset(0).with_align(0);
        {
            let f = &mut self.flash[0];
            let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_eye");
            anim::blink(f, r, &mut self.blink[0], 0, random);
            let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_mouth");
            anim::talk(
                f,
                r,
                &mut self.talk[0],
                0,
                &mut self.lip,
                voice_stat,
                random,
            );
            for i in 0..DISC_NUM {
                let r = f.find(Some(&format!("mc_disk_plate{i:02}")), "mc_disk_text");
                let line = if self.have[i] { 0x15C } else { 0x15A } + i as i32;
                texts.extend(plain.attach_line(
                    f,
                    r,
                    0x400 + i as u32,
                    &self.data.text,
                    line,
                    font,
                ));
            }
            let (x, y) = match self.cursor {
                0 if self.have[0] => (0, 0x40),
                1 if self.have[1] => (0, 0x80),
                _ => (self.toggle * 0x40, 0),
            };
            let r = f.find(None, "mc_icon_play1");
            f.clip_set_uv(
                r,
                Uv {
                    u0: x,
                    v0: y,
                    u1: x + 0x40,
                    v1: y + 0x40,
                    tex: 0,
                },
            );
            self.toggle ^= 1;
        }
        let draw = |s: &Self, i: usize, quads: &mut Vec<ScreenQuad>| {
            let tex = &s.data.layout.flash[i];
            let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
            let (fq, overs) = s.flash[i].draw_with_overs(&has_texture);
            quads.extend(screen_quads(&fq, tex));
            overs
        };
        let overs = draw(self, 0, quads);
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.icon.draw(quads);
        self.msg.draw(self.voice_line, font, quads);
        self.dialog.draw(true, font, quads);
        draw(self, 1, quads);
    }

    /// `DiscFusion_Input`: up / down between the plates, confirm picks one (a disc not recognised starts the swap),
    /// cancel leaves.
    fn input(&mut self, pressed: u32, repeat: u32, sounds: &mut Vec<TitleSound>) {
        if self.flash[0].flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.talk_step = if self.have[0] && self.have[1] { 2 } else { 1 };
            self.plate_goto("fl_on_start");
            self.flags |= STARTED;
        }
        if repeat & (PAD_UP | PAD_DOWN) != 0 {
            self.plate_goto("fl_off_start");
            self.cursor = if repeat & PAD_UP != 0 {
                if self.cursor - 1 < 0 {
                    1
                } else {
                    self.cursor - 1
                }
            } else if self.cursor + 1 >= 2 {
                0
            } else {
                self.cursor + 1
            };
            self.plate_goto("fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
            self.idle = 0;
            self.talk_step = self.cursor + 3;
        } else if pressed & PAD_CROSS != 0 {
            if self.have[self.cursor.clamp(0, 1) as usize] {
                self.flags |= DONE | LEAVING;
                self.timer = 15;
            } else {
                self.chosen = self.cursor;
                self.dlg_step = DLG_ASK;
                sounds.push(TitleSound::VoiceStop);
                self.lip = LipSync::default();
            }
            sounds.push(TitleSound::Effect(1, 1));
            self.idle = 0;
        } else if pressed & PAD_TRIANGLE != 0 {
            self.fade = ColorFade::start_out(true, FADE_FRAMES);
            self.result = 0;
            self.flash[0].goto_label("fl_window_out", true);
            sounds.push(TitleSound::Effect(1, 2));
            self.idle = 0;
        } else {
            self.idle += 1;
            if self.idle == IDLE_FRAMES {
                self.talk_step = 5;
                self.idle = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the real Disc Fusion screen from the ISO in `rom/`, when it is there: the first plate's disc is not
    /// recognised, so confirm starts the swap; with this game's disc put in (as the drive has it), the dialog asks
    /// again after its tries, and the screen goes on once a recognised disc's plate is chosen.
    #[test]
    fn real_disc_fusion_walks_the_swap() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(crate::ub::ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = DiscFusionAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        let dialog = vec![None; assets.dialog.textures.len()];
        data.layout = DiscFusionLayout::new(0, dialog, |k| firsts[&k]);
        let mut drive = DiscDrive::new(iso.clone(), Vec::new());
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[3, 3]);
        let mut screen = DiscFusionScreen::new(&data, 0, &mut libc).unwrap();
        let mut font = Font::new(Vec::new(), 20);
        let mut run =
            |screen: &mut DiscFusionScreen, drive: &mut DiscDrive, bits: u32, n: usize| {
                for i in 0..n {
                    let b = if i == 0 { bits } else { 0 };
                    drive.tick();
                    screen.run_frame(b, b, VOICE_PLAYEND, drive, &mut random, &mut font);
                }
            };
        for _ in 0..4 {
            run(&mut screen, &mut drive, 0, 40);
        }
        assert!(screen.flags & STARTED != 0);
        // Disc 0 is not recognised: the swap's dialog opens and asks.
        run(&mut screen, &mut drive, PAD_CROSS, 60);
        assert_eq!(screen.dlg_step, DLG_CONFIRM);
        run(&mut screen, &mut drive, PAD_UP | PAD_DOWN, 1);
        run(&mut screen, &mut drive, crate::title::PAD_LEFT, 5);
        run(&mut screen, &mut drive, PAD_CROSS, 5);
        assert_eq!(screen.dlg_step, DLG_OPEN);
        // Out and in again with this game's disc: it is not the one asked for.
        drive.toggle_tray();
        run(&mut screen, &mut drive, 0, 2);
        assert_eq!(screen.dlg_step, DLG_INSERT);
        drive.toggle_tray();
        run(&mut screen, &mut drive, 0, 400);
        assert_eq!(screen.dlg_step, DLG_WRONG);
        assert!(!screen.error);
        assert_eq!(screen.progress().1, 0);
    }
}
