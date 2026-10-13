//! Evolution Z's top menu (`EvoTop_Run(1)`, src/menu/evo_top.c, progress mode 48, `EvoMode_Main`): Krillin as
//! guide and three plates, the customising screen (mode 49), the item shop (mode 50) and the guide's explanation
//! of the mode (thirteen lines). Leaving it after the save changed offers the system save first (`DcSave`).

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::dc_save::DcSave;
use crate::dialog::{Dialog, DialogAssets};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::mcflow::McEnv;
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::windows::{IconWin, MsgWin};

/// `gMenuArc7`: file `gProgress->baseFile + 4`; `EvoTop_Run(1)` unpacks its section 1.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 4;
pub const ARCHIVE_SECTION: usize = 1;
/// `BGM_EVOZ`.
pub const BGM_FILE_ID: u32 = 0x10B18;
/// `EVOTOP_VOICE_BASE`.
const VOICE_BASE: u32 = 0x86C9;
/// `EvoTop_Init`'s stream sound.
const STREAM_SE: u32 = 0x10BE0;
const FADE_FRAMES: i32 = 20;
const IDLE_FRAMES: i32 = 0xE10;
const LINE_EXPLAIN_FIRST: i32 = 4;
const LINE_EXPLAIN_LAST: i32 = 0x10;
const LINE_IDLE: i32 = 0x11;

// EVOTOP_*.
const CHOSEN: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;
const EXPLAIN: u32 = 0x20;

/// The cloud clips' scissors (`EvoTop_ScissorCloud1` / `2`).
const CLOUD_CLIPS: [[i32; 4]; 2] = [[0, 0x12, 0x100, 0x112], [0x180, 0x12, 0x200, 0x112]];

/// `menu->tex[n]` of `EvoTop_Init`.
const FLASH_TEXTURES: [ImageTexture; 13] = [
    tex(4, 0),
    tex(4, 1),
    tex(4, 2),
    tex(4, 3),
    tex(2, 0),
    tex(2, 1),
    tex(2, 3),
    tex(4, 4),
    tex(5, 0),
    tex(4, 5),
    tex(4, 6),
    tex(4, 7),
    tex(5, 1),
];
/// `IconWin_Init(MPACK_AT(res, 9), res 3)` and `MsgWin_Init(MPACK_AT(res, 8), …)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((9, 1), 1)),
    Some(((9, 1), 0)),
    tex(3, 0),
    Some(((9, 1), 2)),
    tex(3, 1),
];
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((8, 1), 0)), Some(((8, 1), 1)), None];
/// `DcSave_Init(MPACK_AT(res, 11))`.
const DIALOG_SECTION: usize = 11;

/// What `EvoTop_Init` reads from its unpacked section.
pub struct EvoTopAssets {
    pub data: EvoTopData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct EvoTopData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    /// Section 7: the guide's lines; section 10: their mouth keys.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    pub layout: EvoTopLayout,
}

#[derive(Clone, Debug, Default)]
pub struct EvoTopLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
}

impl EvoTopLayout {
    pub fn new(
        background: usize,
        dialog: Vec<Option<usize>>,
        first: impl Fn(FileKey) -> usize,
    ) -> Self {
        Self {
            background,
            flash: image_indices(&FLASH_TEXTURES, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            msg: image_indices(&MSG_TEXTURES, &first),
            dialog,
        }
    }
}

impl EvoTopAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&FLASH_TEXTURES[..], &ICON_TEXTURES, &MSG_TEXTURES])?;
        let dialog = DialogAssets::parse(pack_section(res, DIALOG_SECTION)?)?;
        Ok(Self {
            data: EvoTopData {
                fod: pack_section(res, 6)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 9)?, 2)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 8)?, 2)?.to_vec(),
                msg_text: pack_section(res, 7)?.to_vec(),
                lips: pack_section(res, 10)?.to_vec(),
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                layout: EvoTopLayout::default(),
            },
            background: decode_picture(res, 1)?,
            files,
            dialog,
        })
    }
}

/// `EvoTop` and the frame loop of `EvoTop_Run`.
pub struct EvoTopScreen {
    layout: EvoTopLayout,
    lips: Vec<u8>,
    flash: Flash,
    icon: IconWin,
    msg: MsgWin,
    save: DcSave,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    blink: i32,
    talk: i32,
    cursor: i32,
    voice_line: i32,
    result: i32,
    timer: i32,
    flags: u32,
    cloud: f32,
    done: Option<i32>,
}

impl EvoTopScreen {
    /// `EvoTop_Init(1)` and the first `ColorFade_StartIn`; `cursor` is `gProgress + 0x7D0` (-1 when the mode was
    /// just entered).
    pub fn new(
        data: &EvoTopData,
        cursor: i32,
        card_res: Option<&[u8]>,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            true,
        )?;
        msg.open();
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            None,
            0,
        )?;
        let blink = random.range(32) as i32;
        sounds.push(TitleSound::StreamSe(STREAM_SE));
        Ok(Self {
            layout: data.layout.clone(),
            lips: data.lips.clone(),
            flash,
            icon,
            msg,
            save: DcSave::new(dialog, card_res),
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            blink,
            talk: 0,
            cursor,
            voice_line: -1,
            result: 0,
            timer: 0,
            flags: 0,
            cloud: 0.0,
            done: None,
        })
    }

    /// `EvoTop_Run`'s result once it has returned: 0 back, 1 customise, 2 the shop.
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// The plate the menu was left on (`gProgress + 0x7D0`).
    pub fn cursor(&self) -> i32 {
        self.cursor
    }

    /// Whether the save flow was started (`DcSave_Term` flips the "save changed" flag back).
    pub fn save_started(&self) -> bool {
        self.save.is_started()
    }

    /// `EvoTop_PlayVoice`.
    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        anim::play_with_subtitle(&self.lips, VOICE_BASE, line, &mut self.lip, sounds);
        self.voice_line = line;
    }

    /// `EvoTop_SetPlate`.
    fn set_plate(&mut self, on: bool) {
        let r = self
            .flash
            .find(None, &format!("mc_menu_plate_{}", self.cursor + 1));
        self.flash
            .clip_goto_label(r, if on { "fl_on_start" } else { "fl_off_start" });
    }

    /// `EvoTop_PlateOk`.
    fn plate_ok(&mut self) {
        let r = self
            .flash
            .find(None, &format!("mc_menu_plate_{}", self.cursor + 1));
        self.flash.clip_goto_label(r, "fl_ok");
    }

    /// One iteration of `EvoTop_Run`'s loop. `dirty` is `gProgress->flags` bit 0 (the save changed).
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        dirty: bool,
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
        self.flash.advance();
        self.draw(pressed, repeat, voice_stat, random, font, mc, &mut frame);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.leave(&mut frame.sounds) {
            self.done = Some(self.result);
            return frame;
        }
        self.input(pressed, repeat, voice_stat, dirty, mc, &mut frame.sounds);
        frame
    }

    /// `EvoTop_Draw`: the movie and the first line when due, the guide's face, the plates, the clouds, then the
    /// picture, the movie, the windows and the save flow.
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
        if self.flags & STARTED == 0 {
            self.flash.goto_label("fl_in", true);
            self.flags |= STARTED;
        }
        if self.flags & GREETED == 0 && self.flash.flags() & FLASH_PAD != 0 {
            match self.cursor {
                -1 => {
                    self.cursor = 0;
                    self.say(0, &mut frame.sounds);
                }
                0..=2 => self.say(self.cursor + 1, &mut frame.sounds),
                _ => {}
            }
            self.set_plate(true);
            self.flags |= GREETED;
        }
        // EvoTop_UpdateFace.
        let eye = self.flash.find(None, "mc_guide_cririn_eye");
        anim::blink(&mut self.flash, eye, &mut self.blink, 0, random);
        let mouth = self.flash.find(None, "mc_guide_cririn_mouth");
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
        // EvoTop_SetPlateText.
        for i in 0..3 {
            let name = format!("mc_menu_plate_{}", i + 1);
            let uv = Uv {
                u0: 0,
                v0: i * 0x20,
                u1: 0x200,
                v1: i * 0x20 + 0x20,
                tex: 0,
            };
            for part in ["mc_menu_text_on", "mc_menu_text_off"] {
                let r = self.flash.find(Some(&name), part);
                self.flash.clip_set_uv(r, uv);
            }
        }
        // EvoTop_SetCloudClips, then both clouds scroll on the one position.
        let uv = Uv {
            u0: 0,
            v0: 0,
            u1: 0x200,
            v1: 0x100,
            tex: 0,
        };
        for (i, clip) in CLOUD_CLIPS.iter().enumerate() {
            let r = self.flash.find(None, &format!("mc_bg_cloud_{}", i + 1));
            self.flash.clip_set_scissor(r, Some(*clip));
        }
        for i in 0..2 {
            let r = self.flash.find(None, &format!("mc_bg_cloud_{}", i + 1));
            anim::scroll(&mut self.flash, r, uv, &mut self.cloud, 0.284_444_45);
        }
        frame.quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let tex = &self.layout.flash;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        frame
            .quads
            .extend(screen_quads(&self.flash.draw(&has_texture), tex));
        self.icon.draw(&mut frame.quads);
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

    /// `EvoTop_Input`: once the movie takes input and no save flow runs.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        voice_stat: i32,
        dirty: bool,
        mc: &mut McEnv,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash.flags() & FLASH_PAD == 0 || self.flags & CHOSEN != 0 {
            return;
        }
        if self.save.state() != 0 {
            return;
        }
        let idle = voice_stat == VOICE_PLAYEND;
        if (pressed != 0 || idle) && self.voice_line == 0 {
            self.say(self.cursor + 1, sounds);
        }
        if repeat != 0 || pressed != 0 {
            self.timer = 0;
        }
        if self.flags & EXPLAIN == 0 {
            if repeat & (PAD_UP | PAD_DOWN) != 0 {
                self.set_plate(false);
                self.cursor += if repeat & PAD_UP != 0 { -1 } else { 1 };
                if self.cursor < 0 {
                    self.cursor = 2;
                } else if self.cursor > 2 {
                    self.cursor = 0;
                }
                self.set_plate(true);
                sounds.push(TitleSound::Effect(1, 0));
                self.say(self.cursor + 1, sounds);
            } else if pressed & PAD_CROSS != 0 {
                self.plate_ok();
                match self.cursor {
                    0 | 1 => {
                        self.flash.goto_label("fl_out", true);
                        self.flags |= CHOSEN;
                        self.result = self.cursor + 1;
                    }
                    _ => {
                        self.flash.goto_label("fl_evo_z_ets_in", true);
                        self.flags |= EXPLAIN;
                        self.say(LINE_EXPLAIN_FIRST, sounds);
                    }
                }
                sounds.push(TitleSound::Effect(1, 1));
            } else if pressed & PAD_TRIANGLE != 0 {
                self.result = 0;
                self.flags |= CHOSEN;
                sounds.push(TitleSound::Effect(1, 2));
                if dirty {
                    self.save.start(mc);
                }
            }
            self.timer += 1;
        } else if pressed & PAD_CROSS != 0 || idle {
            if self.voice_line == 3 {
                self.plate_ok();
            }
            if self.voice_line == LINE_EXPLAIN_LAST {
                self.flags ^= EXPLAIN;
                self.flash.goto_label("fl_evo_z_ets_out", true);
                self.say(3, sounds);
            } else {
                self.say(self.voice_line + 1, sounds);
            }
            if pressed & PAD_CROSS != 0 {
                sounds.push(TitleSound::Effect(1, 1));
            }
        } else if pressed & PAD_TRIANGLE != 0 {
            self.flags ^= EXPLAIN;
            self.say(3, sounds);
            self.flash.goto_label("fl_evo_z_ets_out", true);
            sounds.push(TitleSound::Effect(1, 2));
        }
        if self.timer == IDLE_FRAMES {
            if self.flags & EXPLAIN != 0 {
                self.timer = 0;
            } else {
                self.say(LINE_IDLE, sounds);
            }
        }
        if idle && self.voice_line == LINE_IDLE {
            self.timer = 0;
        }
    }

    /// `EvoTop_Leave`: after the choice, once the save flow is over, the fade (20 frames) and a 30-frame timer;
    /// the voice, the stream sound and (going back) the music fade with it.
    fn leave(&mut self, sounds: &mut Vec<TitleSound>) -> bool {
        if self.flags & CHOSEN == 0 {
            return false;
        }
        if self.flags & LEAVING == 0 && self.save.is_done() {
            let label = if self.result == 0 {
                "fl_cancel"
            } else {
                "fl_out"
            };
            self.flash.goto_label(label, true);
            self.fade = ColorFade::start_out(true, FADE_FRAMES);
            self.msg.close();
            self.icon.close();
            self.timer = 30;
            self.flags |= LEAVING;
        }
        if self.save.is_started() {
            sounds.push(TitleSound::VoiceFadeOutStep);
        }
        if self.flags & LEAVING == 0 {
            return false;
        }
        if self.fade.is_fading_out() {
            sounds.push(if self.result == 0 {
                TitleSound::FadeOutStep
            } else {
                TitleSound::VoiceFadeOutStep
            });
            sounds.push(TitleSound::StreamSeFadeOutStep(0));
        }
        self.timer -= 1;
        self.timer == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::{Libmc, McCard};

    /// Runs the real top menu of Evolution Z from the ISO in `rom/`, when it is there: the greeting, the
    /// guide's explanation (lines 4..16, then back to line 3), the shop's plate, and cancel with the save
    /// unchanged.
    #[test]
    fn real_evolution_z_menu_leads_to_the_shop() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = EvoTopAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let dialog = (0..assets.dialog.textures.len())
            .map(|i| Some(10 + 2 * i))
            .collect();
        let mut data = assets.data;
        data.layout = EvoTopLayout::new(0, dialog, |k| firsts[&k]);
        let mut card = McCard::new(Libmc::new(None));
        let mut save = crate::save::SaveData::default();
        let mut progress = crate::mcflow::Progress::default();
        let mut mc = McEnv {
            card: &mut card,
            save: &mut save,
            progress: &mut progress,
            items: &[],
        };
        let mut random = GameRandom::from_key(&[5, 5]);
        let mut font = Font::new(Vec::new(), 20);
        let mut sounds = Vec::new();
        let new = |random: &mut GameRandom, sounds: &mut Vec<TitleSound>, cursor| {
            EvoTopScreen::new(&data, cursor, None, random, sounds).unwrap()
        };
        let mut random2 = GameRandom::from_key(&[7]);
        let mut screen = new(&mut random2, &mut sounds, -1);
        assert!(sounds.contains(&TitleSound::StreamSe(STREAM_SE)));
        let lines = std::cell::RefCell::new(Vec::new());
        let mut press = |screen: &mut EvoTopScreen, mc: &mut McEnv, bits: u32| {
            for i in 0..40 {
                let b = if i == 0 { bits } else { 0 };
                screen.run_frame(b, b, VOICE_PLAYEND, false, &mut random, &mut font, mc);
                lines.borrow_mut().push(screen.voice_line);
                if screen.result().is_some() {
                    break;
                }
            }
        };
        for _ in 0..10 {
            press(&mut screen, &mut mc, 0);
        }
        assert!(screen.flags & GREETED != 0);
        assert_eq!(screen.cursor, 0);
        // Down twice: the explanation plate; its thirteen lines run by themselves and end on line 3.
        press(&mut screen, &mut mc, PAD_DOWN);
        press(&mut screen, &mut mc, PAD_DOWN);
        assert_eq!(screen.cursor, 2);
        lines.borrow_mut().clear();
        press(&mut screen, &mut mc, PAD_CROSS);
        assert!(screen.flags & EXPLAIN == 0, "{:?}", lines.borrow());
        assert!((LINE_EXPLAIN_FIRST..=LINE_EXPLAIN_LAST).all(|l| lines.borrow().contains(&l)));
        assert_eq!(screen.voice_line, 3);
        // Up: the shop.
        press(&mut screen, &mut mc, PAD_UP);
        press(&mut screen, &mut mc, PAD_CROSS);
        assert_eq!(screen.result(), Some(2));
        assert_eq!(screen.cursor(), 1);
        // Back on the menu on the shop's plate; cancel leaves without the save flow.
        let mut screen = new(&mut random2, &mut sounds, 1);
        for _ in 0..10 {
            press(&mut screen, &mut mc, 0);
        }
        assert_eq!(screen.voice_line, 2);
        press(&mut screen, &mut mc, PAD_TRIANGLE);
        assert_eq!(screen.result(), Some(0));
        assert!(!screen.save_started());
    }
}
