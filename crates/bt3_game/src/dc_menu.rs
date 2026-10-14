//! The Data Center's top menu (`DcMenu_Run(1)`, src/menu/dc_menu.c, progress mode 53): three plates (password,
//! character list, replay) and the guide, Bulma, whose lines chain into each other; and the save prompt it runs
//! when it is left after the save changed (`DcSave`, src/menu/dc_save.c). Only the replay plate leads to a
//! ported screen.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::dc_save::DcSave;
use crate::dialog::{Dialog, DialogAssets};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, Uv, screen_quad};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::mcflow::McEnv;
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::windows::{IconWin, MsgWin};

/// `gMenuArc8`: file `gProgress->baseFile + 0xB`; `DcMenu_Run(1)` unpacks its section 1, `ReplayMenu_Run(4)`
/// section 4.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 0xB;
pub const ARCHIVE_SECTION: usize = 1;
/// `DC_BGM`.
pub const BGM_FILE_ID: u32 = 0x10B18;
/// `DC_VOICE_BASE`.
const VOICE_BASE: u32 = 0x8398;
const PLATES: i32 = 3;
const FADE_FRAMES: i32 = 0x14;
const PAD_CANCEL: u32 = 0x400;

const CHOSEN: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;
const ITEM_LINE: u32 = 0x10;

// The guide's lines.
const LINE_INTRO: i32 = 0;
const LINE_HELLO: i32 = 5;
const LINE_ASK: i32 = 10;
const LINE_PASSWORD: i32 = 11;
const LINE_REPLAY: i32 = 15;
const LINE_LIST: i32 = 22;

/// `menu->view.tex[n]` of `DcMenu_Init`.
const FLASH_TEXTURES: [ImageTexture; 16] = [
    tex(11, 0),
    tex(11, 1),
    tex(11, 2),
    tex(11, 3),
    tex(11, 4),
    tex(11, 5),
    tex(11, 6),
    tex(7, 0),
    tex(7, 1),
    tex(7, 3),
    tex(10, 0),
    tex(8, 0),
    tex(10, 1),
    tex(10, 2),
    tex(10, 3),
    tex(8, 1),
];
/// `MsgWin_Init(MPACK_AT(res, 2), …)` and `IconWin_Init(MPACK_AT(res, 3), res 9)`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((2, 1), 0)), Some(((2, 1), 1)), None];
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((3, 1), 1)),
    Some(((3, 1), 0)),
    tex(9, 0),
    Some(((3, 1), 2)),
    tex(9, 1),
];

/// What `DcMenu_Init` reads from its unpacked section.
pub struct DcMenuAssets {
    pub data: DcMenuData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    /// Section 12, the save prompt's dialog.
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct DcMenuData {
    pub fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    /// Section 4: the message window's lines; section 1: the guide's mouth keys.
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    pub layout: DcMenuLayout,
}

#[derive(Clone, Debug, Default)]
pub struct DcMenuLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
}

impl DcMenuLayout {
    pub fn new(
        background: usize,
        dialog: Vec<Option<usize>>,
        first: impl Fn(FileKey) -> usize,
    ) -> Self {
        Self {
            background,
            flash: image_indices(&FLASH_TEXTURES, &first),
            msg: image_indices(&MSG_TEXTURES, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            dialog,
        }
    }
}

impl DcMenuAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&FLASH_TEXTURES[..], &MSG_TEXTURES, &ICON_TEXTURES])?;
        let dialog = DialogAssets::parse(pack_section(res, 12)?)?;
        Ok(Self {
            data: DcMenuData {
                fod: pack_section(res, 5)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 2)?, 2)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 3)?, 2)?.to_vec(),
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                text: pack_section(res, 4)?.to_vec(),
                lips: pack_section(res, 1)?.to_vec(),
                layout: DcMenuLayout::default(),
            },
            background: decode_picture(res, 6)?,
            files,
            dialog,
        })
    }
}

/// The Data Center's fields of `gProgress` (`ZPROG`), kept between visits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DcProgress {
    /// `dcCursor`: the plate the menu was left on, -1 before the first visit.
    pub cursor: i32,
    /// `dcVisits`.
    pub visits: i32,
    /// `ZPROG_DIRTY`: the save changed (a custom character was stored or deleted).
    pub dirty: bool,
    /// `replayCursor`: the replay slot shown last.
    pub replay_cursor: i32,
}

impl Default for DcProgress {
    fn default() -> Self {
        Self {
            cursor: -1,
            visits: 0,
            dirty: false,
            replay_cursor: 0,
        }
    }
}

/// `DcMenu` and the frame loop of `DcMenu_Run`.
pub struct DcMenuScreen {
    layout: DcMenuLayout,
    lips: Vec<u8>,
    flash: Flash,
    msg: MsgWin,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    save: DcSave,
    blink: i32,
    talk: i32,
    scroll: f32,
    cursor: i32,
    result: i32,
    voice_line: i32,
    visits: i32,
    flags: u32,
    done: Option<i32>,
}

impl DcMenuScreen {
    /// `DcMenu_Init` and the first `ColorFade_StartIn`.
    pub fn new(data: &DcMenuData, card_res: Option<&[u8]>) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.text.clone(),
            false,
        )?;
        msg.open();
        let dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            None,
            0,
        )?;
        Ok(Self {
            layout: data.layout.clone(),
            lips: data.lips.clone(),
            flash,
            msg,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            save: DcSave::new(dialog, card_res),
            blink: 0,
            talk: 0,
            scroll: 0.0,
            cursor: 0,
            result: 0,
            voice_line: -1,
            visits: 0,
            flags: 0,
            done: None,
        })
    }

    /// The mode chosen (54..56), or 0 to leave for the main menu, once the screen is over.
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// `ZPROG->dcCursor` as the screen leaves it.
    #[cfg(test)]
    pub fn cursor(&self) -> i32 {
        self.cursor
    }

    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        anim::play_with_subtitle(&self.lips, VOICE_BASE, line, &mut self.lip, sounds);
        self.voice_line = line;
    }

    /// One frame of the `DcMenu_Run` loop; `voice_stat` is `Voice_GetStat(0)` at its start.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        dc: &mut DcProgress,
        random: &mut GameRandom,
        font: &mut Font,
        mc: &mut McEnv,
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
        self.update(dc, mc, random, &mut frame.sounds);
        self.draw(pressed, repeat, voice_stat, random, font, mc, &mut frame);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.check_leave(&mut frame.sounds) {
            dc.cursor = self.cursor;
            // DcSave_Term: a save that was started flips the "save changed" flag back.
            if self.save.is_started() {
                dc.dirty = !dc.dirty;
            }
            self.done = Some(self.result);
            return frame;
        }
        self.input(
            pressed,
            repeat,
            voice_stat,
            dc,
            random,
            mc,
            &mut frame.sounds,
        );
        frame
    }

    /// `DcMenu_Start`: the guide's first blink, the opening line (the explanation on the very first visit, which
    /// sets the save's `dcFlags` bit 0; a greeting later) and the cursor where the menu was left, with that
    /// plate's line.
    fn start(
        &mut self,
        dc: &DcProgress,
        save: &mut crate::save::SaveData,
        random: &mut GameRandom,
    ) {
        self.blink = random.range(0x20) as i32;
        const DC_FLAGS: usize = 0x1208;
        let flags = i32::from_le_bytes(save.image[DC_FLAGS..DC_FLAGS + 4].try_into().unwrap());
        if flags & 1 == 0 {
            self.voice_line = LINE_INTRO;
            save.image[DC_FLAGS..DC_FLAGS + 4].copy_from_slice(&(flags | 1).to_le_bytes());
        } else {
            self.voice_line = LINE_HELLO;
        }
        self.visits = dc.visits;
        self.cursor = dc.cursor;
        match self.cursor {
            -1 => self.cursor = 0,
            0 => self.voice_line = LINE_PASSWORD,
            1 => self.voice_line = LINE_LIST,
            2 => self.voice_line = LINE_REPLAY,
            _ => {}
        }
    }

    /// `DcMenu_Update`: the movie, the start, and the opening line once the movie takes input.
    fn update(
        &mut self,
        dc: &DcProgress,
        mc: &mut McEnv,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        self.flash.advance();
        if self.flags & STARTED == 0 {
            self.start(dc, mc.save, random);
            self.flags |= STARTED;
        }
        if self.flags & GREETED == 0 && self.flash.flags() & FLASH_PAD != 0 {
            self.light_plate(true);
            let line = self.voice_line;
            self.say(line, sounds);
            self.flags |= GREETED;
        }
        if self.flags & ITEM_LINE == 0
            && matches!(self.voice_line, LINE_PASSWORD | LINE_REPLAY | LINE_LIST)
        {
            self.flags |= ITEM_LINE;
        }
    }

    /// `DcMenu_LightPlate`: the cursor wrapped, its plate lit or dimmed.
    fn light_plate(&mut self, on: bool) {
        self.cursor = if self.cursor < 0 {
            PLATES - 1
        } else if self.cursor > PLATES - 1 {
            0
        } else {
            self.cursor
        };
        let clip = self
            .flash
            .find(None, &format!("mc_menu_plate_{}", self.cursor + 1));
        self.flash
            .clip_goto_label(clip, if on { "fl_on_start" } else { "fl_off_start" });
    }

    /// `DcMenu_Draw`: the guide's eyes and mouth, the scrolling backdrop, the plates' captions, then the
    /// picture, the icon window, the movie, the message window and the save prompt.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        pressed: u32,
        repeat: u32,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        mc: &mut McEnv,
        frame: &mut TitleFrame,
    ) {
        let eye = self.flash.find(None, "mc_guide_blma_eye");
        anim::blink(&mut self.flash, eye, &mut self.blink, 0, random);
        let mouth = self.flash.find(None, "mc_guide_blma_mouth");
        if voice_stat != VOICE_PLAYEND && !self.save.is_started() {
            anim::talk(
                &mut self.flash,
                mouth,
                &mut self.talk,
                0,
                &mut self.lip,
                voice_stat,
                random,
            );
        } else {
            anim::show_next(&mut self.flash, mouth, 0);
        }
        let bg = self.flash.find(None, "mc_compane_3");
        let uv = Uv {
            u0: 0,
            v0: 0,
            u1: 0x40,
            v1: 0x40,
            tex: 0,
        };
        anim::scroll(&mut self.flash, bg, uv, &mut self.scroll, -0.152_380_95);
        // DcMenu_DrawPlates: rows 0, 2, 1 of the captions' sheet.
        for i in 0..PLATES {
            let row = [0, 2, 1][i as usize];
            let uv = Uv {
                u0: 0,
                v0: row * 0x20,
                u1: 0x200,
                v1: row * 0x20 + 0x20,
                tex: 0,
            };
            let name = format!("mc_menu_plate_{}", i + 1);
            for child in ["mc_menu_text_on", "mc_menu_text_off"] {
                let clip = self.flash.find(Some(&name), child);
                self.flash.clip_set_uv(clip, uv);
            }
        }
        frame.quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        self.icon.draw(&mut frame.quads);
        let textures = &self.layout.flash;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        frame.quads.extend(
            self.flash
                .draw(&has_texture)
                .iter()
                .map(|q| screen_quad(q, textures)),
        );
        self.msg.draw(self.voice_line, font, &mut frame.quads);
        self.save.update(
            pressed,
            repeat,
            font,
            mc,
            &mut frame.quads,
            &mut frame.sounds,
        );
    }

    /// `DcMenu_UpdateVoice`: the line being said decides which follows when it ends or confirm is pressed.
    fn update_voice(
        &mut self,
        pressed: u32,
        voice_stat: i32,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        let ended = voice_stat == VOICE_PLAYEND;
        let next = ended || pressed & PAD_CROSS != 0;
        let follow = match self.voice_line {
            0..=3 if next => Some(self.voice_line + 1),
            4 | 6..=9 if next => Some(LINE_ASK),
            LINE_HELLO if next => Some(if self.visits < 6 {
                6
            } else {
                let n = (self.visits - 5).min(3);
                6 + random.range(n as u32 + 1) as i32
            }),
            LINE_ASK if next => Some(match self.cursor {
                0 => LINE_PASSWORD,
                1 => LINE_LIST,
                _ => LINE_REPLAY,
            }),
            LINE_PASSWORD if ended => Some(12 + random.range(3) as i32),
            LINE_REPLAY if ended => Some(16 + random.range(3) as i32),
            LINE_LIST if ended => Some(23 + random.range(3) as i32),
            _ => None,
        };
        if let Some(line) = follow {
            // DCMENU_CLICK on the lines confirm can skip.
            if !matches!(self.voice_line, LINE_PASSWORD | LINE_REPLAY | LINE_LIST)
                && pressed & PAD_CROSS != 0
            {
                sounds.push(TitleSound::Effect(1, 1));
            }
            self.say(line, sounds);
        }
    }

    /// `DcMenu_Input`: up / down move over the plates, confirm picks the mode, cancel leaves (with the save
    /// prompt when the save changed).
    #[allow(clippy::too_many_arguments)]
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        voice_stat: i32,
        dc: &DcProgress,
        random: &mut GameRandom,
        mc: &mut McEnv,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash.flags() & FLASH_PAD == 0 || self.flags & CHOSEN != 0 {
            return;
        }
        if self.save.state() != 0 {
            return;
        }
        if repeat & (PAD_UP | PAD_DOWN) != 0 {
            self.light_plate(false);
            self.cursor += if repeat & PAD_UP != 0 { -1 } else { 1 };
            self.light_plate(true);
            let line = match self.cursor {
                0 => LINE_PASSWORD,
                1 => LINE_LIST,
                _ => LINE_REPLAY,
            };
            self.say(line, sounds);
            sounds.push(TitleSound::Effect(1, 0));
        } else if pressed & PAD_CROSS != 0 {
            let clip = self
                .flash
                .find(None, &format!("mc_menu_plate_{}", self.cursor + 1));
            self.flash.clip_goto_label(clip, "fl_ok");
            self.flash.clip_goto_label(clip, "fl_on_loop");
            self.flags |= CHOSEN;
            self.result = 54 + self.cursor;
            sounds.push(TitleSound::Effect(1, 1));
            self.msg.close();
        } else if pressed & PAD_CANCEL != 0 {
            self.result = 0;
            self.flags |= CHOSEN;
            sounds.push(TitleSound::Effect(1, 2));
            if dc.dirty {
                self.save.start(mc);
            }
        }
        self.update_voice(pressed, voice_stat, random, sounds);
    }

    /// `DcMenu_CheckLeave`: after a choice, once the save prompt is over, the fade out; true when it ended.
    fn check_leave(&mut self, sounds: &mut Vec<TitleSound>) -> bool {
        if self.flags & CHOSEN == 0 {
            return false;
        }
        if self.flags & LEAVING == 0 && self.save.is_done() {
            self.flags |= LEAVING;
            self.flash.goto_label("fl_out", true);
            self.icon.close();
            self.msg.close();
            self.fade = ColorFade::start_out(true, FADE_FRAMES);
        }
        if self.save.is_started() {
            sounds.push(TitleSound::VoiceFadeOutStep);
        }
        if self.flags & LEAVING != 0 {
            if self.fade.is_fading_out() {
                sounds.push(TitleSound::FadeOutStep);
            } else {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::{Libmc, McCard};

    /// The Data Center's top menu from the ISO in `rom/`: on the first visit the guide explains (the save's flag
    /// is set); the replay plate leads to mode 56.
    #[test]
    fn real_data_center_leads_to_the_replay_list() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};
        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = DcMenuAssets::parse(&res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = DcMenuLayout::new(0, vec![None; assets.dialog.textures.len()], |key| {
            firsts[&key]
        });
        let mut card = McCard::new(Libmc::new(None));
        let mut save = crate::save::SaveData::default();
        let mut progress = crate::mcflow::Progress::default();
        let mut mc = McEnv {
            card: &mut card,
            save: &mut save,
            progress: &mut progress,
            items: &[],
        };
        let mut dc = DcProgress::default();
        let mut screen = DcMenuScreen::new(&data, None).unwrap();
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[5]);
        let mut sounds = Vec::new();
        let mut step = |screen: &mut DcMenuScreen, mc: &mut McEnv, dc: &mut DcProgress, p: u32| {
            screen
                .run_frame(p, p, VOICE_PLAYEND, dc, &mut random, &mut font, mc)
                .sounds
        };
        let mut ready = false;
        for _ in 0..300 {
            sounds.extend(step(&mut screen, &mut mc, &mut dc, 0));
            if screen.flash.flags() & FLASH_PAD != 0 && screen.flags & GREETED != 0 {
                ready = true;
                break;
            }
        }
        assert!(ready);
        assert!(sounds.contains(&TitleSound::Voice(VOICE_BASE + 0x55C)));
        let dc_flags = i32::from_le_bytes(mc.save.image[0x1208..0x120C].try_into().unwrap());
        assert_eq!(dc_flags & 1, 1);
        step(&mut screen, &mut mc, &mut dc, PAD_DOWN);
        step(&mut screen, &mut mc, &mut dc, 0);
        step(&mut screen, &mut mc, &mut dc, PAD_DOWN);
        step(&mut screen, &mut mc, &mut dc, 0);
        assert_eq!(screen.cursor(), 2);
        step(&mut screen, &mut mc, &mut dc, PAD_CROSS);
        for _ in 0..100 {
            if screen.result().is_some() {
                break;
            }
            step(&mut screen, &mut mc, &mut dc, 0);
        }
        assert_eq!(screen.result(), Some(56));
        assert_eq!(dc.cursor, 2);
    }
}
