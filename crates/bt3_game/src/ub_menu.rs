//! Ultimate Battle's menu (`UbMenu_Run(1)`, src/menu/ub_menu.c, progress mode 13, `Ub_Main`): Androids 17 and 18
//! as guides commenting on four plates (Sim Dragon, Mission 100, Survival, Disc Fusion), the third closed until
//! thirty missions are cleared; their greeting, idle dialogues and goodbye lines.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::{GameRandom, LibcRandom};
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::ub::VOICE_BASE;
use crate::windows::{IconWin, MsgWin};

/// `UbMenu_Run(1)`: section 1 of `gMenuArc3`.
pub const ARCHIVE_SECTION: usize = 1;
const FADE_FRAMES: i32 = 0x14;
const ITEM_NUM: i32 = 4;
const IDLE_FRAMES: i32 = 0x708;
/// `P_UBFLAG_ITEM3`: thirty missions cleared open the third plate.
const SAVE_ITEM3: i32 = 1;

// UBMENU_*.
const CHOSEN: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;

/// The dialogues started after 1800 idle frames and when the screen is left with cancel.
const IDLE_SCRIPT: [i32; 3] = [21, 41, 61];
const LEAVE_SCRIPT: [i32; 3] = [81, 101, 121];

/// `gUbMenu->tex[n]` of `UbMenu_Init`.
const TEX: [ImageTexture; 17] = [
    tex(3, 0),
    tex(3, 1),
    tex(3, 3),
    tex(3, 6),
    tex(3, 5),
    tex(3, 4),
    tex(3, 2),
    tex(3, 7),
    tex(5, 0),
    tex(5, 1),
    tex(5, 3),
    tex(5, 5),
    tex(5, 6),
    tex(5, 8),
    tex(3, 8),
    tex(3, 9),
    tex(3, 10),
];
/// `IconWin_Init(MPACK_AT(res, 7), res 4)` and `MsgWin_Init(MPACK_AT(res, 6), …)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((7, 1), 1)),
    Some(((7, 1), 0)),
    tex(4, 0),
    Some(((7, 1), 2)),
    tex(4, 1),
];
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((6, 1), 0)), Some(((6, 1), 1)), None];

/// What `UbMenu_Init` reads from its unpacked section.
pub struct UbMenuAssets {
    pub data: UbMenuData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct UbMenuData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    /// Section 8: the guides' lines; 9: their mouth keys.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    pub layout: UbMenuLayout,
}

#[derive(Clone, Debug, Default)]
pub struct UbMenuLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
}

impl UbMenuLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            msg: image_indices(&MSG_TEXTURES, &first),
        }
    }
}

impl UbMenuAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&TEX[..], &ICON_TEXTURES, &MSG_TEXTURES])?;
        Ok(Self {
            data: UbMenuData {
                fod: pack_section(res, 1)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 7)?, 2)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 6)?, 2)?.to_vec(),
                msg_text: pack_section(res, 8)?.to_vec(),
                lips: pack_section(res, 9)?.to_vec(),
                layout: UbMenuLayout::default(),
            },
            background: decode_picture(res, 2)?,
            files,
        })
    }
}

/// `UbMenu` and the frame loop of `UbMenu_Run`.
pub struct UbMenuScreen {
    layout: UbMenuLayout,
    lips: Vec<u8>,
    flash: Flash,
    icon: IconWin,
    msg: MsgWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    cursor: i32,
    /// `gUbMenuPlate3Open`: the third plate as it was when the screen was entered.
    plate3_open: bool,
    timer: i32,
    idle: i32,
    icon_frame: i32,
    voice_line: i32,
    voice_skip: bool,
    script: i32,
    script_last: i32,
    talker: usize,
    blink: [i32; 2],
    talk: [i32; 2],
    result: i32,
    done: Option<i32>,
}

impl UbMenuScreen {
    /// `UbMenu_Init(1)` and the first `ColorFade_StartIn`; `cursor` is `gProgress->ubCursor`, `ub_flags` the save's.
    pub fn new(
        data: &UbMenuData,
        cursor: i32,
        ub_flags: i32,
        libc: &mut LibcRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
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
        Ok(Self {
            layout: data.layout.clone(),
            lips: data.lips.clone(),
            flash,
            icon,
            msg,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            cursor,
            plate3_open: ub_flags & SAVE_ITEM3 != 0,
            timer: 0,
            idle: 0,
            icon_frame: 0,
            voice_line: -1,
            voice_skip: false,
            script: 0,
            script_last: 0,
            talker: 0,
            blink,
            talk: [0; 2],
            result: 1,
            done: None,
        })
    }

    /// `UbMenu_Run`'s result once it has returned: the plate chosen, 1..4, or 0 backed out.
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// The plate to keep in `gProgress->ubCursor`.
    pub fn cursor(&self) -> i32 {
        self.cursor
    }

    fn play(&mut self, line: i32, talker: usize, sounds: &mut Vec<TitleSound>) {
        self.voice_line = line;
        self.talker = talker;
        anim::play_with_subtitle(&self.lips, VOICE_BASE, line, &mut self.lip, sounds);
    }

    /// The guides' comment on the plate under the cursor: lines 2..5, guide 0 for plates 1 and 3.
    fn say_plate(&mut self, sounds: &mut Vec<TitleSound>) {
        let talker = if self.cursor == 1 || self.cursor == 3 {
            0
        } else {
            1
        };
        self.play(self.cursor + 2, talker, sounds);
    }

    /// `UbMenu_ClipGoto(0, 0, label)`.
    fn clip_goto(&mut self, label: &str) {
        let r = self
            .flash
            .find(None, &format!("mc_menu_plate_{}", self.cursor + 1));
        self.flash.clip_goto_label(r, label);
    }

    fn cursor_up(&mut self) {
        self.cursor -= 1;
        if self.cursor < 0 {
            self.cursor = ITEM_NUM - 1;
        }
    }

    fn cursor_down(&mut self) {
        self.cursor += 1;
        if self.cursor >= ITEM_NUM {
            self.cursor = 0;
        }
    }

    /// One iteration of `UbMenu_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
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
        self.update(voice_stat, &mut frame.sounds);
        self.update_voice(pressed, voice_stat, &mut frame.sounds);
        self.draw(voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & GREETED == 0
            && self.flash.flags() & FLASH_PAD != 0
        {
            self.flags |= GREETED;
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
        } else if self.script == 0 {
            self.input(pressed, repeat, random, &mut frame.sounds);
        }
        frame
    }

    /// `UbMenu_Update`: the leave timer; after a greeting ends, the comment on the current plate.
    fn update(&mut self, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.flags & GREETED != 0
            && (0..2).contains(&self.voice_line)
            && voice_stat == VOICE_PLAYEND
        {
            self.say_plate(sounds);
        }
        self.flash.advance();
    }

    /// `UbMenu_UpdateVoice`: one step of the guides' dialogue, when the line before it has ended or confirm cut
    /// it short.
    fn update_voice(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if !self.voice_skip {
            if self.script == 0 {
                return;
            }
            if self.voice_line != -1
                && voice_stat != VOICE_PLAYEND
                && self.script == self.script_last
            {
                if pressed & PAD_CROSS != 0 {
                    sounds.push(TitleSound::Effect(1, 1));
                    self.voice_skip = true;
                }
                return;
            }
        } else {
            self.voice_skip = false;
        }
        match self.script {
            1 | 11 => {
                let who = usize::from(self.script == 11);
                self.script = 0;
                self.play(who as i32, who, sounds);
            }
            21 => {
                self.play(7, 0, sounds);
                self.script += 1;
            }
            22 => {
                self.play(8, 1, sounds);
                self.script = 0;
            }
            41 => {
                self.play(9, 1, sounds);
                self.script += 1;
            }
            42 => {
                self.play(10, 0, sounds);
                self.script += 1;
            }
            43 => {
                self.play(11, 0, sounds);
                self.script += 1;
            }
            44 => {
                self.play(12, 1, sounds);
                self.script = 0;
            }
            61 => {
                self.play(13, 1, sounds);
                self.script = 0;
            }
            81 => {
                self.play(14, 0, sounds);
                self.script += 1;
            }
            101 => {
                self.play(15, 1, sounds);
                self.script += 1;
            }
            121 => {
                self.play(16, 1, sounds);
                self.script += 1;
            }
            82 | 102 | 122 => {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.script = 0;
            }
            _ => {}
        }
        self.script_last = self.script;
    }

    /// `UbMenu_Draw`: the picture, the two guides, the blinking button icon, the plates, the windows.
    fn draw(
        &mut self,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let f = &mut self.flash;
        for i in 0..2 {
            let (clip, eye, mouth) = if i != 0 {
                ("mc_guide_18go", "mc_guide_18go_eye", "mc_guide_18go_mouth")
            } else {
                ("mc_guide_17go", "mc_guide_17go_eye", "mc_guide_17go_mouth")
            };
            let r = f.find(Some(clip), eye);
            anim::blink(f, r, &mut self.blink[i], 0, random);
            let r = f.find(Some(clip), mouth);
            if self.talker == i {
                anim::talk(
                    f,
                    r,
                    &mut self.talk[i],
                    0,
                    &mut self.lip,
                    voice_stat,
                    random,
                );
            } else {
                anim::show_next(f, r, 0);
            }
        }
        let uv = |u0: i32, v0: i32, u1: i32, v1: i32| Uv {
            u0,
            v0,
            u1,
            v1,
            tex: 0,
        };
        let r = f.find(None, "mc_icon_play1");
        let x = self.icon_frame << 6;
        f.clip_set_uv(r, uv(x, 0, x + 0x40, 0x40));
        self.icon_frame ^= 1;
        for i in 0..ITEM_NUM {
            // The closed plate shows the fifth row.
            let row = if i == 2 && !self.plate3_open { 4 } else { i };
            let name = format!("mc_menu_plate_{}", i + 1);
            for part in ["mc_menu_text_off", "mc_menu_text_on"] {
                let r = f.find(Some(&name), part);
                f.clip_set_uv(r, uv(0, row << 5, 0x200, (row << 5) + 0x20));
            }
        }
        let tex = &self.layout.flash;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        quads.extend(screen_quads(&f.draw(&has_texture), tex));
        self.icon.draw(quads);
        self.msg.draw(self.voice_line, font, quads);
    }

    /// `UbMenu_Input`: up / down the plates (the closed one skipped), confirm chooses, cancel leaves after a
    /// goodbye line; 1800 idle frames start a dialogue.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.clip_goto("fl_on_start");
            self.script = if random.range(2) != 0 { 1 } else { 11 };
            self.flags |= STARTED;
        }
        let closed = |s: &Self| s.cursor == 2 && !s.plate3_open;
        if repeat & PAD_UP != 0 {
            self.clip_goto("fl_off_start");
            self.cursor_up();
            if closed(self) {
                self.cursor_up();
            }
            self.clip_goto("fl_on_start");
            self.say_plate(sounds);
            sounds.push(TitleSound::Effect(1, 0));
            self.idle = 0;
        } else if repeat & PAD_DOWN != 0 {
            self.clip_goto("fl_off_start");
            self.cursor_down();
            if closed(self) {
                self.cursor_down();
            }
            self.clip_goto("fl_on_start");
            self.say_plate(sounds);
            sounds.push(TitleSound::Effect(1, 0));
            self.idle = 0;
        } else if pressed & PAD_CROSS != 0 {
            self.flags |= CHOSEN | LEAVING;
            self.timer = 15;
            self.result = self.cursor + 1;
            sounds.push(TitleSound::Effect(1, 1));
            self.idle = 0;
            sounds.push(TitleSound::VoiceStop);
            self.lip = LipSync::default();
        } else if pressed & PAD_TRIANGLE != 0 {
            self.result = 0;
            self.script = LEAVE_SCRIPT[random.range(3) as usize];
            self.flash.goto_label("fl_menu_cancel", true);
            sounds.push(TitleSound::Effect(1, 2));
            self.idle = 0;
        } else {
            self.idle += 1;
            if self.idle == IDLE_FRAMES {
                self.script = IDLE_SCRIPT[random.range(3) as usize];
                self.idle = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the real menu from the ISO in `rom/`, when it is there: the greeting, a comment on the plate, the
    /// closed third plate skipped going down, and confirm choosing the fourth (Disc Fusion).
    #[test]
    fn real_ub_menu_skips_the_closed_plate() {
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
        let assets = UbMenuAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = UbMenuLayout::new(0, |k| firsts[&k]);
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1, 3]);
        let mut screen = UbMenuScreen::new(&data, 1, 0, &mut libc).unwrap();
        let mut font = Font::new(Vec::new(), 20);
        let mut sounds = Vec::new();
        let mut press = |screen: &mut UbMenuScreen, bits: u32| {
            for i in 0..40 {
                let b = if i == 0 { bits } else { 0 };
                let frame = screen.run_frame(b, b, VOICE_PLAYEND, &mut random, &mut font);
                sounds.extend(frame.sounds);
                if screen.result().is_some() {
                    break;
                }
            }
        };
        for _ in 0..6 {
            press(&mut screen, 0);
        }
        assert!(screen.flags & (GREETED | STARTED) == GREETED | STARTED);
        // The greeting, then the comment on plate 2 (line 3).
        assert_eq!(screen.voice_line, 3);
        press(&mut screen, PAD_DOWN);
        assert_eq!(screen.cursor, 3);
        assert_eq!(screen.voice_line, 5);
        press(&mut screen, PAD_CROSS);
        for _ in 0..3 {
            press(&mut screen, 0);
        }
        assert_eq!(screen.result(), Some(4));
        assert_eq!(screen.cursor(), 3);
    }
}
