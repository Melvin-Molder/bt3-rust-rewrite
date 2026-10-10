//! The main menu (`MainMenu_Run(2)`, src/menu/main_menu.c): a ring of mode plates, four guide characters of whom one
//! talks about the item under the cursor, and the seven dragon balls of the save.

use std::collections::BTreeMap;
use std::error::Error;

use bt3_data::gs::{RgbaTexture, decode_sprite_texture};
use bt3_data::pack_section;
use bt3_data::tex::TextureResource;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quad};
use crate::font::Font;
use crate::logo::{ColorFade, TILES_PER_PICTURE, picture_quads};
use crate::rng::GameRandom;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::windows::{IconWin, MsgWin};

/// `MainMenu_Run(2)` unpacks section 2 of gMenuArc1.
pub const ARCHIVE_SECTION: usize = 2;
const VOICE_BASE: u32 = 0x86F9;
/// `VOICE_LANG2_OFFSET`: a fresh save has flag bit 0 set.
const VOICE_LANG2_OFFSET: u32 = 0x55C;
const ITEM_MAX: i32 = 11;
const ROWS: i32 = 4;
const FADE_FRAMES: i32 = 0x14;

const FLAG_CHOSEN: u32 = 1;
const FLAG_LEAVING: u32 = 2;
const FLAG_STARTED: u32 = 4;
const FLAG_GREETED: u32 = 8;

/// A texture file of the unpacked section: (section, part of its pack or 0 for the section itself).
type FileKey = (usize, usize);

/// `gMainMenu->tex[n]` of `MainMenu_Init`: (section, entry) per movie image.
const FLASH_TEXTURES: [(usize, usize); 50] = [
    (2, 0),
    (2, 1),
    (4, 0),
    (4, 1),
    (3, 1),
    (3, 0),
    (8, 0),
    (7, 0),
    (5, 0),
    (8, 2),
    (8, 1),
    (7, 1),
    (6, 0),
    (15, 1),
    (15, 2),
    (15, 3),
    (25, 3),
    (25, 4),
    (25, 5),
    (25, 0),
    (25, 1),
    (25, 2),
    (13, 0),
    (16, 0),
    (16, 2),
    (17, 0),
    (17, 2),
    (26, 0),
    (26, 1),
    (26, 2),
    (15, 0),
    (14, 0),
    (14, 1),
    (14, 2),
    (18, 2),
    (18, 0),
    (19, 2),
    (19, 0),
    (24, 0),
    (24, 1),
    (24, 2),
    (9, 0),
    (11, 0),
    (10, 0),
    (12, 0),
    (12, 1),
    (12, 2),
    (12, 3),
    (12, 4),
    (12, 5),
];
/// `MsgWin_Init(MPACK_AT(res, 21), …)`: entries 0 and 1 of its part 1; image 2 is the text anchor.
const MSG_TEXTURES: [Option<(FileKey, usize)>; 3] = [Some(((21, 1), 0)), Some(((21, 1), 1)), None];
/// `IconWin_Init(MPACK_AT(res, 22), res 23)`.
const ICON_TEXTURES: [Option<(FileKey, usize)>; 5] = [
    Some(((22, 1), 1)),
    Some(((22, 1), 0)),
    Some(((23, 0), 0)),
    Some(((22, 1), 2)),
    Some(((23, 0), 1)),
];

/// What `MainMenu_Init` reads from its unpacked section.
pub struct MainMenuAssets {
    pub data: MainMenuData,
    /// Section 1 (`gMainMenu->bg`).
    pub background: Vec<RgbaTexture>,
    /// Every texture file the movies use, all entries decoded.
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

/// The movies and texts a `MainMenu_Run` starts from, and where the textures went.
#[derive(Clone, Debug)]
pub struct MainMenuData {
    pub fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 28: the message window's lines.
    pub text: Vec<u8>,
    /// Section 27: mouth keys of every voice line, two languages each.
    pub lips: Vec<u8>,
    pub layout: MainMenuLayout,
}

/// Texture indices: per image the clamped texture of entry 0, entry n at `+ 2n`, each followed by its wrapping twin.
#[derive(Clone, Debug, Default)]
pub struct MainMenuLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
}

impl MainMenuLayout {
    /// `first(file)` is where entry 0 of a texture file was registered.
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        let at = |(file, entry): (FileKey, usize)| first(file) + 2 * entry;
        Self {
            background,
            flash: FLASH_TEXTURES
                .iter()
                .map(|&(section, entry)| Some(at(((section, 0), entry))))
                .collect(),
            msg: MSG_TEXTURES.iter().map(|t| t.map(at)).collect(),
            icon: ICON_TEXTURES.iter().map(|t| t.map(at)).collect(),
        }
    }
}

impl MainMenuAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let mut keys: Vec<FileKey> = FLASH_TEXTURES.iter().map(|&(s, _)| (s, 0)).collect();
        keys.extend(
            MSG_TEXTURES
                .iter()
                .chain(&ICON_TEXTURES)
                .flatten()
                .map(|t| t.0),
        );
        let mut files = BTreeMap::new();
        for key in keys {
            if files.contains_key(&key) {
                continue;
            }
            let section = pack_section(res, key.0)?;
            let file = if key.1 == 0 {
                section
            } else {
                pack_section(section, key.1)?
            };
            files.insert(key, decode_file(file)?);
        }
        let background = decode_file(pack_section(res, 1)?)?;
        if background.len() != TILES_PER_PICTURE {
            return Err(format!("main menu picture has {} tiles", background.len()).into());
        }
        Ok(Self {
            data: MainMenuData {
                fod: pack_section(res, 20)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 21)?, 2)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 22)?, 2)?.to_vec(),
                text: pack_section(res, 28)?.to_vec(),
                lips: pack_section(res, 27)?.to_vec(),
                layout: MainMenuLayout::default(),
            },
            background,
            files: files.into_iter().collect(),
        })
    }
}

fn decode_file(data: &[u8]) -> Result<Vec<RgbaTexture>, Box<dyn Error>> {
    Ok(TextureResource::parse(data)?
        .entries
        .iter()
        .map(decode_sprite_texture)
        .collect::<Result<Vec<_>, _>>()?)
}

/// How `MainMenu_Run` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainMenuResult {
    /// 0: cancelled, back to the title (progress mode 1).
    Title,
    /// 1: `gProgress->mode` was set to this mode.
    Mode(i32),
}

/// `MainMenu` (`gMainMenu`) and the frame loop of `MainMenu_Run`.
pub struct MainMenuScreen {
    layout: MainMenuLayout,
    lips: Vec<u8>,
    flash: Flash,
    msg: MsgWin,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    cursor: i32,
    timer: i32,
    voice_line: i32,
    items: Vec<i32>,
    top: i32,
    extra: i32,
    icon_timer: i32,
    icon_frame: i32,
    guide: i32,
    blink: [i32; 4],
    talk: [i32; 4],
    cloud: [f32; 2],
    /// `gSaveData->unlockFlags`: bits 0..6 are the dragon balls.
    unlock_flags: u32,
    /// `gProgress->mainMenuItem`.
    menu_item: i32,
    mode: i32,
    result: i32,
    done: Option<MainMenuResult>,
}

impl MainMenuScreen {
    /// `MainMenu_Init` and the first `ColorFade_StartIn`.
    pub fn new(
        data: &MainMenuData,
        menu_item: i32,
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.text.clone(),
            true,
        )?;
        msg.open();
        let unlock_flags = 0;
        let items: Vec<i32> = (0..ITEM_MAX)
            .filter(|&i| i != 4 && (i != 10 || unlock_flags & 0x7F == 0x7F))
            .collect();
        let count = items.len() as i32;
        let mut top = count - 1;
        if let Some(i) = items.iter().position(|&item| item == menu_item) {
            top = i as i32 - 1;
            if top < 0 {
                top += count;
            }
        }
        let mut blink = [0; 4];
        for b in &mut blink {
            *b = random.range(0x20) as i32;
        }
        Ok(Self {
            layout: data.layout.clone(),
            lips: data.lips.clone(),
            flash,
            msg,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            cursor: 0,
            timer: 0,
            voice_line: -1,
            items,
            top,
            extra: 0,
            icon_timer: 0,
            icon_frame: 0,
            guide: 0,
            blink,
            talk: [0; 4],
            cloud: [0.0; 2],
            unlock_flags,
            menu_item,
            mode: 0,
            result: 1,
            done: None,
        })
    }

    pub fn result(&self) -> Option<MainMenuResult> {
        self.done
    }

    /// `gProgress->mainMenuItem`, set when a mode was chosen.
    pub fn menu_item(&self) -> i32 {
        self.menu_item
    }

    fn count(&self) -> i32 {
        self.items.len() as i32
    }

    /// `MAINMENU_CUR`: the item under the cursor.
    fn current(&self) -> i32 {
        self.items[((self.top + self.cursor + 1) % self.count()) as usize]
    }

    /// `Voice_PlayWithSubtitle(subtitles, MAINMENU_VOICE_BASE, line)` with save flag bit 0 set.
    fn play_voice(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        sounds.push(TitleSound::Voice(
            VOICE_BASE + line as u32 + VOICE_LANG2_OFFSET,
        ));
        let word = |i: usize| {
            self.lips
                .get(i * 4..i * 4 + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize & !3)
        };
        if let Some(ofs) = word(line as usize * 2 + 2)
            && let Some(data) = self.lips.get(ofs..)
        {
            self.lip.start(data);
        }
    }

    /// One frame of the `MainMenu_Run` loop; `voice_stat` is `Voice_GetStat(0)` at its start.
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
        self.update(voice_stat, random, &mut frame.sounds);
        self.draw(voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & FLAG_GREETED == 0
            && self.flash.flags() & FLASH_PAD != 0
        {
            self.flags |= FLAG_GREETED;
            self.guide = random.range(4) as i32;
            self.voice_line = if self.unlock_flags & 0x7F == 0x7F {
                self.guide + 0x3C
            } else {
                self.guide + 0xC
            };
            self.play_voice(self.voice_line, &mut frame.sounds);
        }
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::FadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = Some(if self.result != 0 {
                MainMenuResult::Mode(self.mode)
            } else {
                MainMenuResult::Title
            });
            return frame;
        }
        if self.flags & FLAG_LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.menu_item = self.current();
            }
        } else {
            self.input(pressed, repeat, random, &mut frame.sounds);
        }
        frame
    }

    /// `MainMenu_Update`: once the greeting has ended, a random guide reads out the current item.
    fn update(&mut self, voice_stat: i32, random: &mut GameRandom, sounds: &mut Vec<TitleSound>) {
        if self.flags & FLAG_GREETED != 0
            && (self.voice_line == self.guide + 0xC || self.voice_line == self.guide + 0x3C)
            && voice_stat == VOICE_PLAYEND
        {
            self.guide = random.range(4) as i32;
            self.voice_line = self.current() * 4 + self.guide + 0x10;
            self.play_voice(self.voice_line, sounds);
        }
        self.flash.advance();
    }

    /// `MainMenu_PlateGoto(0, label)`: the plate under the cursor.
    fn plate_goto(&mut self, label: &str) {
        let clip = self
            .flash
            .find(None, &format!("mc_menu_plate_{}", self.cursor + 2));
        self.flash.clip_goto_label(clip, label);
    }

    /// The text, icon and animated icon of `item` on plate `name`.
    fn set_plate(&mut self, name: &str, item: i32) -> Uv {
        let text = Uv {
            u0: 0,
            v0: (item % 8) * 0x20,
            u1: 0x200,
            v1: (item % 8) * 0x20 + 0x20,
            tex: 0,
        };
        for child in ["mc_menu_text_off", "mc_menu_text_on"] {
            let clip = self.flash.find(Some(name), child);
            self.flash.clip_set_uv(clip, text);
            self.flash.clip_set_tex(clip, item / 8);
        }
        let icon = Uv {
            u0: (item % 4) * 0x40,
            v0: (item / 4) * 0x40,
            u1: (item % 4) * 0x40 + 0x40,
            v1: (item / 4) * 0x40 + 0x40,
            tex: 0,
        };
        let clip = self.flash.find(Some(name), "mc_icon_off");
        self.flash.clip_set_uv(clip, icon);
        let play = Uv {
            u0: 0,
            v0: (item % 10) * 0x30,
            u1: 0x30,
            v1: (item % 10) * 0x30 + 0x30,
            tex: 0,
        };
        let clip = self.flash.find(Some(name), "mc_icon_play");
        self.flash.clip_set_uv(clip, play);
        self.flash.clip_set_tex(clip, item / 10);
        play
    }

    /// `MainMenu_Draw`.
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
        for i in 0..4 {
            let name = match i {
                0 => "gohan",
                1 => "bidel",
                2 => "goten",
                _ => "torankusu",
            };
            let eye = self.flash.find(None, &format!("mc_guide_{name}_eye"));
            let mouth = self.flash.find(None, &format!("mc_guide_{name}_mouth"));
            anim::blink(&mut self.flash, eye, &mut self.blink[i], 0, random);
            if self.guide == i as i32 {
                anim::talk(
                    &mut self.flash,
                    mouth,
                    &mut self.talk[i],
                    0,
                    &mut self.lip,
                    voice_stat,
                    random,
                );
            } else {
                anim::show_next(&mut self.flash, mouth, 0);
            }
        }

        // Columns of each item's animated icon sheet.
        const COLS: [i32; ITEM_MAX as usize] = [4, 4, 4, 5, 5, 5, 5, 4, 5, 5, 4];
        for i in 0..6 {
            let item = self.items[((self.top + i) % self.count()) as usize];
            let name = format!("mc_menu_plate_{}", i + 1);
            let play = self.set_plate(&name, item);
            let current = self.current();
            if item == current {
                let clip = self.flash.find(Some(&name), "mc_icon_play");
                anim::sheet(
                    &mut self.flash,
                    clip,
                    &mut self.icon_timer,
                    &mut self.icon_frame,
                    play,
                    COLS[current as usize],
                    1,
                    if current == 3 { 3 } else { 6 },
                );
            }
        }
        let extra = self.items[self.extra as usize];
        self.set_plate("mc_menu_plate_7", extra);

        for (i, name) in ["mc_menu_yajirusi_up", "mc_menu_yajirusi_down"]
            .iter()
            .enumerate()
        {
            let u0 = i as i32 * 0x20;
            let clip = self.flash.find(None, name);
            self.flash.clip_set_uv(
                clip,
                Uv {
                    u0,
                    v0: 0x20,
                    u1: u0 + 0x20,
                    v1: 0x40,
                    tex: 0,
                },
            );
        }

        for i in 0..7 {
            let name = format!("mc_ch_a_db_{}", i + 1);
            let ball = self.flash.find(None, &name);
            if self.unlock_flags & (1 << i) != 0 {
                self.flash.clip_set_flags(ball, PROP_VISIBLE, true);
                let clip = self.flash.find(Some(&name), "mc_dragonball");
                self.flash.clip_set_uv(
                    clip,
                    Uv {
                        u0: (i % 4) * 0x40,
                        v0: (i / 4) * 0x40,
                        u1: (i % 4) * 0x40 + 0x40,
                        v1: (i / 4) * 0x40 + 0x40,
                        tex: 0,
                    },
                );
            } else {
                self.flash.clip_set_flags(ball, PROP_VISIBLE, false);
            }
        }

        for (i, (name, dx)) in [
            ("mc_bg_cloud_a", -0.355_555_56f32),
            ("mc_bg_cloud", -0.189_629_63),
        ]
        .into_iter()
        .enumerate()
        {
            let clip = self.flash.find(None, name);
            let uv = Uv {
                u0: 0,
                v0: 0,
                u1: 0x200,
                v1: 0x100,
                tex: 0,
            };
            anim::scroll(&mut self.flash, clip, uv, &mut self.cloud[i], dx);
        }

        let textures = &self.layout.flash;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        quads.extend(
            self.flash
                .draw(&has_texture)
                .iter()
                .map(|q| screen_quad(q, textures)),
        );
        self.icon.draw(quads);
        self.msg.draw(self.voice_line, font, quads);
    }

    /// `MAINMENU_CHOOSE(next)` (the choice counter `gSaveData->tourHour` is not kept yet).
    fn choose(&mut self, mode: i32) {
        self.flags |= FLAG_CHOSEN | FLAG_LEAVING;
        self.timer = 15;
        self.mode = mode;
        self.flash.goto_label("fl_out", true);
    }

    /// `MainMenu_Input`.
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
        if self.flags & FLAG_STARTED == 0 {
            self.plate_goto("fl_on_start");
            if self.unlock_flags & 0x7F == 0x7F {
                for i in 0..7 {
                    let clip = self.flash.find(None, &format!("mc_ch_a_db_{}", i + 1));
                    self.flash.clip_goto_label(clip, "fl_loop");
                }
            }
            self.flags |= FLAG_STARTED;
        }
        let count = self.count();
        if repeat & (PAD_UP | PAD_DOWN) != 0 {
            self.plate_goto("fl_off_start");
            self.icon_timer = 0;
            self.icon_frame = 0;
            if repeat & PAD_UP != 0 {
                self.cursor -= 1;
                if self.cursor < 0 {
                    self.cursor = 0;
                    self.flash.goto_label("fl_menu_down", true);
                    self.top -= 1;
                    if self.top < 0 {
                        self.top += count;
                    }
                    self.extra = (self.top + 6) % count;
                }
            } else {
                self.cursor += 1;
                if self.cursor >= ROWS {
                    self.cursor = ROWS - 1;
                    self.flash.goto_label("fl_menu_up", true);
                    self.extra = self.top;
                    self.top += 1;
                    if self.top >= count {
                        self.top -= count;
                    }
                }
            }
            self.plate_goto("fl_on_start");
            self.guide = random.range(4) as i32;
            self.voice_line = self.current() * 4 + self.guide + 0x10;
            self.play_voice(self.voice_line, sounds);
            sounds.push(TitleSound::Effect(1, 0));
        } else if pressed & PAD_CROSS != 0 {
            let mode = match self.current() {
                0 => Some(6),
                1 => Some(13),
                2 => Some(33),
                3 => Some(38),
                5 => Some(48),
                6 => Some(44),
                7 => Some(53),
                8 => Some(60),
                9 => Some(62),
                10 => Some(70),
                _ => None,
            };
            if let Some(mode) = mode {
                self.choose(mode);
            }
            self.plate_goto("fl_ok");
            sounds.push(TitleSound::Effect(1, 1));
        } else if pressed & PAD_TRIANGLE != 0 {
            self.flash.goto_label("fl_cancel", true);
            self.icon.close();
            self.msg.close();
            self.fade = ColorFade::start_out(true, FADE_FRAMES);
            self.result = 0;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }
}

#[cfg(test)]
mod tests {
    /// Runs the real main menu from the ISO in `rom/`, when it is there.
    #[test]
    fn real_main_menu_greets_scrolls_and_chooses() {
        use super::{
            ARCHIVE_SECTION, MainMenuAssets, MainMenuLayout, MainMenuResult, MainMenuScreen,
        };
        use crate::anim::{VOICE_PLAYEND, VOICE_PLAYING};
        use crate::title::{ARCHIVE_FILE_ID, PAD_CROSS, PAD_DOWN, TitleSound};
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe, pack_section};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = MainMenuAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = MainMenuLayout::new(0, |key| firsts[&key]);
        let mut font = crate::font::Font::new(Vec::new(), 20);
        let mut random = crate::rng::GameRandom::from_key(&[5, 6, 7, 8]);
        let mut menu = MainMenuScreen::new(&data, 0, &mut random).unwrap();
        assert_eq!(menu.items, [0, 1, 2, 3, 5, 6, 7, 8, 9]);
        let mut voices = Vec::new();
        let mut stat = 0;
        for frame in 0..400 {
            let out = menu.run_frame(0, 0, stat, &mut random, &mut font);
            for sound in out.sounds {
                if let TitleSound::Voice(id) = sound {
                    voices.push((frame, id));
                    stat = VOICE_PLAYING;
                }
            }
            if frame == 200 {
                stat = VOICE_PLAYEND;
            }
        }
        println!(
            "voices {voices:X?}, flags {:X}, guide {}",
            menu.flags, menu.guide
        );
        assert_eq!(voices.len(), 2, "greeting, then the current item");
        let out = menu.run_frame(PAD_DOWN, PAD_DOWN, stat, &mut random, &mut font);
        println!("down: {:?}", out.sounds);
        assert_eq!(menu.current(), 1);
        menu.run_frame(PAD_CROSS, PAD_CROSS, stat, &mut random, &mut font);
        for _ in 0..60 {
            menu.run_frame(0, 0, stat, &mut random, &mut font);
        }
        assert_eq!(menu.result(), Some(MainMenuResult::Mode(13)));
        assert_eq!(menu.menu_item(), 1);
    }
}
