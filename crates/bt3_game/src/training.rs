//! The training menu (`Train_Run(1)`, src/menu/training.c, progress mode 44, `TrainMode_Main`): Great Saiyaman
//! on the top menu ("training" / "character select"), Videl on the three classes and their lesson lists. A
//! lesson is introduced, explained page by page (a text line and a picture streamed from the disc each), and
//! then, by its flags, played as a tutorial battle (mode 5) or marked cleared at once. One bit per lesson in the
//! save's `trainClear[class]` says it was cleared. The character select of free training (mode 45,
//! `CharSel_Run(2)`) is the character select's.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND, VOICE_PLAYING};
use crate::char_select::{BattleSetup, PictureLoader, SideSetup};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::windows::{IconWin, MsgWin, TextBox, TextDraw, draw_texts};

/// `gMenuArc6`: file `gProgress->baseFile + 13`; `Train_Run(1)` unpacks its section 1, the training's character
/// select (`CharSel_Run(2)` in mode 45) section 2.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 13;
pub const ARCHIVE_SECTION: usize = 1;
pub const CHAR_SELECT_SECTION: usize = 2;
/// `BGM_TRAIN`.
pub const BGM_FILE_ID: u32 = 0x10B1A;
/// `TRAIN_VOICE_BASE_A` (the top menu's lines, Great Saiyaman) and `TRAIN_VOICE_BASE_B` (Videl's).
const VOICE_BASE_A: u32 = 0x87BD;
const VOICE_BASE_B: u32 = 0x8278;
/// The explanation picture of page `n`: file `0x402 + n`.
const IMAGE_FILE: u32 = 0x402;
const CLASS_NUM: usize = 3;
/// `TRAIN_CLASS0_NUM`..`TRAIN_CLASS2_NUM`: the lesson records of each class.
const CLASS_LESSONS: [usize; CLASS_NUM] = [13, 11, 14];
const ROWS: i32 = 3;
const FADE_FRAMES: i32 = 0x14;
const IDLE_FRAMES: i32 = 0xE10;

// TRAIN_LV_*.
const LV_TOP: i32 = 0;
const LV_CLASS: i32 = 1;
const LV_LESSON: i32 = 2;
const LV_INTRO: i32 = 3;
const LV_PAGES: i32 = 4;
const LV_CLEAR: i32 = 5;

// TRAIN_* flags.
const IMAGE_CHANGE: u32 = 1;
const IMAGE_LOADING: u32 = 2;
const LEAVING: u32 = 8;
const FADING: u32 = 0x10;
const STARTED: u32 = 0x20;
const GREETED: u32 = 0x40;
const IDLE_SAID: u32 = 0x80;
const CLEAR_SAVED: u32 = 0x200;

// TRAIN_LESSON_*.
const LESSON_TUTORIAL: u8 = 1;
const LESSON_BATTLE: u8 = 2;
const LESSON_LAST: u8 = 4;

// MPTRAIN_*: why the menu was left, read when it opens again.
pub const MPTRAIN_TUTORIAL: i32 = 1;
pub const MPTRAIN_BATTLE: i32 = 2;
pub const MPTRAIN_FIRST_CLEAR: i32 = 4;
pub const MPTRAIN_AGAIN: i32 = 8;
pub const MPTRAIN_SELECT: i32 = 0x10;

/// The list's scissor (`Train_ScissorOn`: `Sprite_SetScissor(0, 0x200, 0x4F, 0x12F)`).
const LIST_CLIP: [i32; 4] = [0, 0x4F, 0x200, 0x12F];

/// `gTrain->tex[n]` of `Train_Init`; image 9 is the explanation picture.
const FLASH_TEXTURES: [ImageTexture; 48] = {
    let mut t: [ImageTexture; 48] = [None; 48];
    t[0] = tex(7, 0);
    t[5] = tex(7, 1);
    t[6] = tex(7, 2);
    t[7] = tex(7, 3);
    t[18] = tex(7, 4);
    t[19] = tex(7, 5);
    t[21] = tex(7, 6);
    t[22] = tex(7, 7);
    t[23] = tex(7, 8);
    t[24] = tex(7, 9);
    t[27] = tex(7, 10);
    t[28] = tex(7, 11);
    t[29] = tex(7, 12);
    t[30] = tex(7, 13);
    t[32] = tex(7, 14);
    t[33] = tex(7, 15);
    t[1] = tex(6, 0);
    t[2] = tex(6, 1);
    t[3] = tex(6, 2);
    t[4] = tex(6, 3);
    t[25] = tex(9, 0);
    t[26] = tex(9, 4);
    t[31] = tex(8, 0);
    t[10] = tex(4, 0);
    t[12] = tex(4, 2);
    t[11] = tex(4, 4);
    t[13] = tex(3, 0);
    t[16] = tex(3, 2);
    t[14] = tex(3, 4);
    t[17] = tex(3, 6);
    t[15] = tex(3, 8);
    t
};
const IMAGE_SLOT: usize = 9;
/// `MsgWin_Init(MPACK_AT(res, 11), …)` and `IconWin_Init(MPACK_AT(res, 10), res 5)`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((11, 1), 0)), Some(((11, 1), 1)), None];
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((10, 1), 1)),
    Some(((10, 1), 0)),
    tex(5, 0),
    Some(((10, 1), 2)),
    tex(5, 1),
];
/// `IconWin_SetIcon(0)` and `IconWin_SetIcon(2)`: textures 1 and 3 of the icon resource.
const ICONS: [ImageTexture; 2] = [tex(5, 1), tex(5, 3)];

/// `TrainLesson`: a 0x10-byte record of section 22.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lesson {
    /// The line of its name in the list text, and the key of `Train_Leave`'s table.
    pub id: u16,
    pub page_num: u8,
    pub flags: u8,
    pub first_page: i32,
    /// The line said when the cursor is on it.
    pub voice: i32,
    pub stage: u8,
    pub bgm: u8,
    /// Side 0's (the pad) and side 1's (the computer) character.
    pub chara: [u8; 2],
}

impl Lesson {
    fn parse(b: &[u8]) -> Self {
        let w = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        Self {
            id: u16::from_le_bytes([b[0], b[1]]),
            page_num: b[2],
            flags: b[3],
            first_page: w(4),
            voice: w(8),
            stage: b[0xC],
            bgm: b[0xD],
            chara: [b[0xE], b[0xF]],
        }
    }
}

/// What `Train_Init` reads from its unpacked section.
pub struct TrainAssets {
    pub data: TrainData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct TrainData {
    pub fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Sections 14 (the top menu's lines) and 15 (the classes' and lessons').
    pub msg_text: [Vec<u8>; 2],
    /// Sections 12 and 13: the mouth keys of the lines of voice bases A and B.
    pub subtitles: [Vec<u8>; 2],
    /// Section 16: the lesson names (also the text of line 0x28); section 17: the explanation pages.
    pub text: Vec<u8>,
    pub page_text: Vec<u8>,
    /// Section 22: the three classes' lessons one after another.
    pub lessons: Vec<Lesson>,
    /// Sections 24 and 26: page ids left out of the lists, and pages that keep the picture before them.
    pub skip: Vec<i32>,
    pub no_image: Vec<i32>,
    pub layout: TrainLayout,
}

#[derive(Clone, Debug, Default)]
pub struct TrainLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub icons: Vec<Option<usize>>,
}

impl TrainLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&FLASH_TEXTURES, &first),
            msg: image_indices(&MSG_TEXTURES, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            icons: image_indices(&ICONS, &first),
        }
    }
}

/// A count word, then the entries from byte 0x10.
fn table(res: &[u8], section: usize) -> Result<Vec<i32>, Box<dyn Error>> {
    let s = pack_section(res, section)?;
    let word = |at: usize| {
        s.get(at..at + 4)
            .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
    };
    let n = word(0).ok_or("short table")? as u32 as usize;
    (0..n)
        .map(|i| word(0x10 + i * 4).ok_or_else(|| "short table".into()))
        .collect()
}

impl TrainAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(
            res,
            [&FLASH_TEXTURES[..], &MSG_TEXTURES, &ICON_TEXTURES, &ICONS],
        )?;
        let total: usize = CLASS_LESSONS.iter().sum();
        let s = pack_section(res, 22)?;
        let lessons = (0..total)
            .map(|i| {
                s.get(i * 0x10..i * 0x10 + 0x10)
                    .map(Lesson::parse)
                    .ok_or("short lesson table")
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            data: TrainData {
                fod: pack_section(res, 1)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 11)?, 2)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 10)?, 2)?.to_vec(),
                msg_text: [
                    pack_section(res, 14)?.to_vec(),
                    pack_section(res, 15)?.to_vec(),
                ],
                subtitles: [
                    pack_section(res, 12)?.to_vec(),
                    pack_section(res, 13)?.to_vec(),
                ],
                text: pack_section(res, 16)?.to_vec(),
                page_text: pack_section(res, 17)?.to_vec(),
                lessons,
                skip: table(res, 24)?,
                no_image: table(res, 26)?,
                layout: TrainLayout::default(),
            },
            background: decode_picture(res, 2)?,
            files,
        })
    }
}

/// A listed lesson (`Train_BuildLists`): its pages after the skip list, name line and voice line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Listed {
    pages: Vec<i32>,
    name_line: i32,
    voice: i32,
}

/// `Train_BuildLists`: per class, the lessons that keep at least one page after the skip list.
fn build_lists(data: &TrainData) -> [Vec<Listed>; CLASS_NUM] {
    let mut start = 0;
    std::array::from_fn(|class| {
        let mut out = Vec::new();
        let mut i = start;
        while let Some(p) = data.lessons.get(i) {
            let first = p.first_page as u32;
            let pages: Vec<i32> = (first..first + u32::from(p.page_num))
                .filter(|&page| !data.skip.iter().any(|&s| s as u32 == page))
                .map(|page| page as i32)
                .collect();
            if !pages.is_empty() {
                out.push(Listed {
                    pages,
                    name_line: i32::from(p.id),
                    voice: p.voice,
                });
            }
            if p.flags & LESSON_LAST != 0 {
                break;
            }
            i += 1;
        }
        start += CLASS_LESSONS[class];
        out
    })
}

/// `gProgress`'s training block: why the menu was left and its cursors (`Train.sel`, `Train.saved`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrainProgress {
    /// `MPTRAIN_*`.
    pub flags: i32,
    pub cursor: [i32; 5],
    /// `gProgress->tutorial`: the tutorial number, the lesson's index plus 0 / 13 / 24 by class.
    pub tutorial: i32,
}

/// How `Train_Run` ended.
#[derive(Clone, Debug, PartialEq)]
pub enum TrainResult {
    /// 0: back to the main menu.
    Back,
    /// 0x2D: the character select of free training (mode 45).
    Select,
    /// 1: a lesson, with the battle `Train_Leave` set up for it (`None` when the lesson's flags set none).
    Lesson(Option<BattleSetup>),
}

/// `Train` and the frame loop of `Train_Run`.
pub struct TrainScreen {
    data: TrainData,
    textures: Vec<Option<usize>>,
    flash: Flash,
    msg: MsgWin,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    lists: [Vec<Listed>; CLASS_NUM],
    blink: [i32; 2],
    talk: [i32; 2],
    pose: [i32; 2],
    cloud: f32,
    result: i32,
    voice_line: i32,
    page: i32,
    sel: [i32; 2],
    saved: [i32; CLASS_NUM],
    level: i32,
    top: i32,
    row: i32,
    extra: i32,
    extra2: i32,
    idle: i32,
    image_alpha: f32,
    flags: u32,
    page_num: i32,
    loading: Option<usize>,
    progress: TrainProgress,
    done: Option<TrainResult>,
}

impl TrainScreen {
    /// `Train_Init(1)` and the first `ColorFade_StartIn`.
    pub fn new(
        data: &TrainData,
        progress: TrainProgress,
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        let msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text[0].clone(),
            false,
        )?;
        let blink = std::array::from_fn(|_| random.range(0x20) as i32);
        Ok(Self {
            data: data.clone(),
            textures: data.layout.flash.clone(),
            flash,
            msg,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            lists: build_lists(data),
            blink,
            talk: [0; 2],
            pose: [0; 2],
            cloud: 0.0,
            result: 0,
            voice_line: -1,
            page: 0,
            sel: [0; 2],
            saved: [0; CLASS_NUM],
            level: LV_TOP,
            top: 0,
            row: 0,
            extra: 0,
            extra2: 0,
            idle: 0,
            image_alpha: 0.0,
            flags: 0,
            page_num: 0,
            loading: None,
            progress,
            done: None,
        })
    }

    /// `Train_Run`'s result once it has returned.
    pub fn result(&self) -> Option<TrainResult> {
        self.done.clone()
    }

    /// The training block of `gProgress` as the screen leaves it.
    pub fn progress(&self) -> TrainProgress {
        self.progress
    }

    fn class(&self) -> usize {
        self.sel[1].clamp(0, CLASS_NUM as i32 - 1) as usize
    }

    fn count(&self, class: usize) -> i32 {
        self.lists[class].len() as i32
    }

    fn cur_lesson(&self) -> i32 {
        self.row + self.top
    }

    fn listed(&self, lesson: i32) -> Option<&Listed> {
        usize::try_from(lesson)
            .ok()
            .and_then(|i| self.lists[self.class()].get(i))
    }

    fn voice_of(&self, lesson: i32) -> i32 {
        self.listed(lesson).map_or(0, |l| l.voice)
    }

    fn name_line(&self, lesson: i32) -> i32 {
        self.listed(lesson).map_or(0, |l| l.name_line)
    }

    /// `Train_IsCleared`.
    fn is_cleared(save: &SaveData, class: i32, lesson: i32) -> bool {
        save.train_clear(class as usize) & (1i32.wrapping_shl(lesson as u32)) != 0
    }

    /// `Train_IsClassCleared`.
    fn is_class_cleared(&self, save: &SaveData, class: i32) -> bool {
        let n = usize::try_from(class)
            .ok()
            .filter(|&c| c < CLASS_NUM)
            .map_or(0, |c| self.count(c));
        (0..n).all(|i| Self::is_cleared(save, class, i))
    }

    /// `Train_IsAllCleared`.
    fn is_all_cleared(&self, save: &SaveData) -> bool {
        (0..CLASS_NUM as i32).all(|c| self.is_class_cleared(save, c))
    }

    /// `Train_Say`: the top menu's lines are Great Saiyaman's (voice bank A), the rest Videl's (B).
    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        let (lips, base) = if self.level == LV_TOP {
            (&self.data.subtitles[0], VOICE_BASE_A)
        } else {
            (&self.data.subtitles[1], VOICE_BASE_B)
        };
        anim::play_with_subtitle(lips, base, line, &mut self.lip, sounds);
        self.voice_line = line;
    }

    /// `Train_ClipGoto`: the clip `name` to "fl_on_start" or "fl_off_start".
    fn clip_goto(&mut self, name: &str, on: bool) {
        let r = self.flash.find(None, name);
        self.flash
            .clip_goto_label(r, if on { "fl_on_start" } else { "fl_off_start" });
    }

    /// `Train_PlateGoto`: plate `plate` of the top / class menu.
    fn plate_goto(&mut self, plate: i32, on: bool) {
        self.clip_goto(&format!("mc_menu_plate_{}", plate + 1), on);
    }

    /// `Train_Plate2Goto`: row `plate` of the lesson list.
    fn plate2_goto(&mut self, plate: i32, on: bool) {
        self.clip_goto(&format!("mc_menu_plate_2_{}", plate + 1), on);
    }

    /// `Train_CursorGoto`: the plate under the cursor of the current level.
    fn cursor_goto(&mut self, on: bool) {
        match self.level {
            LV_TOP | LV_CLASS => self.plate_goto(self.sel[self.level as usize], on),
            LV_LESSON => self.plate2_goto(self.saved[self.class()], on),
            _ => {}
        }
    }

    /// `Train_RestoreScroll`: the class's saved lesson on the first row, or the last three.
    fn restore_scroll(&mut self) {
        let class = self.class();
        let saved = self.saved[class];
        self.top = saved;
        let top = self.count(class) - ROWS;
        if top < saved {
            self.top = top;
            self.row = ROWS - (self.count(class) - saved);
        } else {
            self.row = 0;
        }
    }

    /// `Train_SaveCursor`.
    fn save_cursor(&mut self) {
        let class = self.class();
        self.saved[class] = self.cur_lesson();
    }

    /// `Train_CopyCursor(TRAIN_PROG->cursor, gTrain->sel)`: `sel[2]` and `saved[3]` into the progress block.
    fn store_cursor(&mut self) {
        let c = &mut self.progress.cursor;
        c[..2].copy_from_slice(&self.sel);
        c[2..].copy_from_slice(&self.saved);
    }

    /// `Train_CopyCursor(gTrain->sel, TRAIN_PROG->cursor)`.
    fn load_cursor(&mut self) {
        let c = self.progress.cursor;
        self.sel.copy_from_slice(&c[..2]);
        self.saved.copy_from_slice(&c[2..]);
    }

    /// `Train_MoveRow`: the lesson cursor by `dir`, scrolling the list at its edges; false at the list's end.
    fn move_row(&mut self, dir: i32, sounds: &mut Vec<TitleSound>) -> bool {
        let count = self.count(self.class());
        let lesson = self.row + self.top + dir;
        if !(0..=count - 1).contains(&lesson) {
            return false;
        }
        self.plate2_goto(self.row, false);
        self.row += dir;
        if clamp(&mut self.row, 0, ROWS - 1) {
            self.top += dir;
            if !clamp(&mut self.top, 0, count - ROWS) {
                match dir {
                    1 => {
                        self.flash.goto_label("fl_class_menu_up", true);
                        self.extra = self.top - 1;
                    }
                    -1 => {
                        self.flash.goto_label("fl_class_menu_down", true);
                        self.extra = self.top + 3;
                    }
                    _ => {}
                }
            }
        }
        self.plate2_goto(self.row, true);
        sounds.push(TitleSound::Effect(1, 0));
        let line = self.voice_of(self.cur_lesson());
        self.say(line, sounds);
        true
    }

    /// `Train_NextRow`: the cursor to the next lesson, without the scroll animation or the sound.
    fn next_row(&mut self, sounds: &mut Vec<TitleSound>) {
        let count = self.count(self.class());
        self.plate2_goto(self.row, false);
        self.row += 1;
        if clamp(&mut self.row, 0, ROWS - 1) {
            self.top += 1;
        }
        if !clamp(&mut self.top, 0, count - ROWS) {
            self.extra = self.top - 1;
        }
        self.plate2_goto(self.row, true);
        let line = self.voice_of(self.cur_lesson());
        self.say(line, sounds);
    }

    /// `Train_MoveCursor`: up (0) or down (1) on the current level.
    fn move_cursor(&mut self, dir: i32, sounds: &mut Vec<TitleSound>) {
        let step = if dir == 0 { -1 } else { 1 };
        match self.level {
            LV_TOP => {
                self.cursor_goto(false);
                self.sel[0] += step;
                wrap(&mut self.sel[0], 0, 1);
                self.cursor_goto(true);
                sounds.push(TitleSound::Effect(1, 0));
                self.say(self.sel[0] + 2, sounds);
            }
            LV_CLASS => {
                self.cursor_goto(false);
                self.sel[1] += step;
                wrap(&mut self.sel[1], 0, 2);
                self.cursor_goto(true);
                sounds.push(TitleSound::Effect(1, 0));
                self.say(self.sel[1] + 1, sounds);
            }
            LV_LESSON => {
                self.move_row(step, sounds);
            }
            _ => {}
        }
    }

    /// `Train_FindLesson`: the record of the lesson under the cursor, by its name line, within its class.
    fn find_lesson(&self) -> Option<(usize, Lesson)> {
        let class = self.class();
        let start: usize = CLASS_LESSONS[..class].iter().sum();
        let id = self.name_line(self.cur_lesson());
        self.data
            .lessons
            .get(start..start + CLASS_LESSONS[class])?
            .iter()
            .enumerate()
            .find(|(_, p)| i32::from(p.id) == id)
            .map(|(i, p)| (i, *p))
    }

    /// `Train_PageHasImage`: whether the page shown is not one that keeps the picture before it.
    fn page_has_image(&self) -> bool {
        !self.data.no_image.contains(&self.voice_line)
    }

    /// One iteration of `Train_Run`'s loop. `voice_stat` is `Voice_GetStat(0)`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        save: &mut SaveData,
        loader: &mut dyn PictureLoader,
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
        self.draw(voice_stat, save, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.check_leave(voice_stat, pressed, &mut frame.sounds) {
            self.done = Some(match self.result {
                0 => TrainResult::Back,
                1 => TrainResult::Lesson(self.leave()),
                _ => TrainResult::Select,
            });
            return frame;
        }
        let pad = Pad { pressed, repeat };
        self.input(pad, voice_stat, save, random, font, &mut frame.sounds);
        self.update_image(loader);
        frame
    }

    /// `Train_Update`: the movie; the first call opens the menu where the progress flags say it was left, and
    /// once the movie takes input the matching line is said.
    fn update(&mut self, _voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        self.flash.advance();
        if self.flags & STARTED == 0 {
            self.flags |= STARTED;
            self.plate2_goto(4, true);
            if self.progress.flags & MPTRAIN_FIRST_CLEAR != 0 {
                self.load_cursor();
                self.restore_scroll();
                self.msg.set_text(self.data.msg_text[1].clone());
                self.flash.goto_label("fl_clear_in", true);
                self.level = LV_CLEAR;
                self.extra2 = self.cur_lesson();
                self.msg.open();
            } else if self.progress.flags & MPTRAIN_AGAIN != 0 {
                self.load_cursor();
                self.restore_scroll();
                self.msg.set_text(self.data.msg_text[1].clone());
                self.flash.goto_label("fl_clear_in_2", true);
                self.level = LV_LESSON;
                self.restore_scroll();
                self.next_row(sounds);
                self.msg.open();
            } else {
                self.msg.open();
                self.icon.open();
                self.flash.goto_label("fl_in_1", true);
                self.msg.set_text(self.data.msg_text[0].clone());
                self.level = LV_TOP;
            }
        } else if self.flash.flags() & FLASH_PAD != 0 && self.flags & GREETED == 0 {
            self.flags |= GREETED;
            let f = self.progress.flags;
            if f & MPTRAIN_SELECT != 0 {
                self.say(self.sel[0] + 2, sounds);
                self.cursor_goto(true);
            } else if f & MPTRAIN_AGAIN != 0 {
                let line = self.voice_of(self.cur_lesson());
                self.say(line, sounds);
            } else if f & MPTRAIN_FIRST_CLEAR != 0 {
                self.say(0x39, sounds);
            } else {
                self.say(0, sounds);
                self.cursor_goto(true);
            }
            self.progress.flags = 0;
        }
    }

    /// `Train_CheckLeave`: once leaving, the fade out (after the farewell line, or at a button, when going back to
    /// the main menu) with the sound fading along; true when it is over.
    fn check_leave(&mut self, voice_stat: i32, pressed: u32, sounds: &mut Vec<TitleSound>) -> bool {
        if self.flags & LEAVING == 0 {
            return false;
        }
        if self.flags & FADING == 0 {
            if self.voice_line != 6 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.flags |= FADING;
            } else if voice_stat == VOICE_PLAYEND || pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                if pressed & PAD_CROSS != 0 {
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    sounds.push(TitleSound::Effect(1, 2));
                }
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.flags |= FADING;
            }
        }
        if self.flags & FADING != 0 {
            if self.fade.is_fading_out() {
                sounds.push(if self.result == 0 {
                    TitleSound::FadeOutStep
                } else {
                    TitleSound::VoiceFadeOutStep
                });
            } else {
                return true;
            }
        }
        false
    }

    /// `TRAIN_VOICE_DONE(n)`: the line `n` is shown and has ended, or confirm was pressed.
    fn voice_done(&self, pad: Pad, voice_stat: i32, n: i32) -> bool {
        (pad.pressed & PAD_CROSS != 0 || voice_stat == VOICE_PLAYEND) && self.voice_line == n
    }

    /// `Train_Input`: pad 0 on the current level, and the guide's idle lines after 3600 quiet frames.
    fn input(
        &mut self,
        pad: Pad,
        voice_stat: i32,
        save: &mut SaveData,
        random: &mut GameRandom,
        font: &mut Font,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash.flags() & FLASH_PAD == 0 || self.flags & LEAVING != 0 {
            return;
        }
        let idle_voice = voice_stat == VOICE_PLAYEND;
        match self.level {
            LV_TOP => {
                if idle_voice && self.voice_line == 0 {
                    self.say(1, sounds);
                } else if idle_voice && self.voice_line == 1 {
                    self.say(self.sel[0] + 2, sounds);
                }
                if pad.repeat & PAD_UP != 0 {
                    self.move_cursor(0, sounds);
                } else if pad.repeat & PAD_DOWN != 0 {
                    self.move_cursor(1, sounds);
                } else if pad.pressed & PAD_CROSS != 0 {
                    match self.sel[0] {
                        0 => {
                            self.result = 0x2D;
                            self.flags |= LEAVING;
                            self.store_cursor();
                            self.flash.goto_label("fl_cancel_1", true);
                            self.progress.flags |= MPTRAIN_SELECT;
                        }
                        1 => {
                            self.cursor_goto(false);
                            self.level = LV_CLASS;
                            self.flash.goto_label("fl_class_in", true);
                            self.cursor_goto(true);
                            self.say(0, sounds);
                            self.msg.set_text(self.data.msg_text[1].clone());
                        }
                        _ => {}
                    }
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pad.pressed & PAD_TRIANGLE != 0 {
                    self.flash.goto_label("fl_cancel_1", true);
                    self.flags |= LEAVING;
                    self.result = 0;
                    sounds.push(TitleSound::Effect(1, 2));
                    self.say(6, sounds);
                }
            }
            LV_CLASS => {
                if idle_voice && self.voice_line == 0 {
                    self.say(self.sel[1] + 1, sounds);
                }
                if pad.repeat & PAD_UP != 0 {
                    self.move_cursor(0, sounds);
                } else if pad.repeat & PAD_DOWN != 0 {
                    self.move_cursor(1, sounds);
                } else if pad.pressed & PAD_CROSS != 0 {
                    self.cursor_goto(false);
                    self.restore_scroll();
                    self.level = LV_LESSON;
                    self.flash.goto_label("fl_class_menu_in", true);
                    self.icon.close();
                    self.plate2_goto(self.row, true);
                    self.say(9, sounds);
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pad.pressed & PAD_TRIANGLE != 0 {
                    self.cursor_goto(false);
                    self.level = LV_TOP;
                    self.flash.goto_label("fl_class_cancel", true);
                    self.cursor_goto(true);
                    self.say(self.sel[0] + 2, sounds);
                    self.msg.set_text(self.data.msg_text[0].clone());
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            LV_LESSON => {
                if idle_voice && self.voice_line == 9 {
                    let line = self.voice_of(self.cur_lesson());
                    self.say(line, sounds);
                }
                if pad.repeat & PAD_UP != 0 {
                    self.move_cursor(0, sounds);
                } else if pad.repeat & PAD_DOWN != 0 {
                    self.move_cursor(1, sounds);
                } else if pad.pressed & PAD_CROSS != 0 {
                    self.plate2_goto(self.row, false);
                    self.level = LV_INTRO;
                    self.flash.goto_label("fl_class_menu_2", true);
                    sounds.push(TitleSound::Effect(1, 1));
                    let line = random.range(4) as i32 + 0x35;
                    self.say(line, sounds);
                    self.pose[1] = 1;
                    self.extra2 = self.cur_lesson();
                }
                if pad.pressed & PAD_TRIANGLE != 0 {
                    self.plate2_goto(self.row, false);
                    self.level = LV_CLASS;
                    self.flash.goto_label("fl_class_menu_out", true);
                    self.cursor_goto(true);
                    self.save_cursor();
                    self.icon.open();
                    sounds.push(TitleSound::Effect(1, 2));
                    self.say(self.sel[1] + 1, sounds);
                }
            }
            LV_INTRO => {
                if (0x35..=0x38).any(|n| self.voice_done(pad, voice_stat, n)) {
                    if pad.pressed & PAD_CROSS != 0 {
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                    self.level = LV_PAGES;
                    self.flash.goto_label("fl_ss_view_in", true);
                    self.page = 0;
                    self.msg.set_spacing(0, 5);
                    font.pad_type = 0;
                    self.msg.set_text(self.data.page_text.clone());
                    let cur = self.listed(self.cur_lesson()).cloned().unwrap_or_default();
                    self.voice_line = cur.pages.first().copied().unwrap_or(-1);
                    self.page_num = cur.pages.len() as i32;
                    font.reset_icon_anim();
                    self.flags |= IMAGE_CHANGE;
                }
            }
            LV_PAGES => self.input_pages(pad, save, font, sounds),
            LV_CLEAR => self.input_clear(pad, voice_stat, save, sounds),
            _ => {}
        }
        if pad.pressed != 0 || pad.repeat != 0 {
            self.idle = 0;
        }
        if self.idle > IDLE_FRAMES {
            self.idle = 0;
            self.flags |= IDLE_SAID;
            if self.level == LV_TOP {
                let line = random.range(2) as i32 + 4;
                self.say(line, sounds);
                self.pose[0] = i32::from(self.pose[0] == 0);
            } else if self.level < LV_INTRO {
                let line = random.range(2) as i32 + 4;
                self.say(line, sounds);
            }
        } else {
            self.idle += 1;
        }
        // Compared with a new random draw, not with the line that was said.
        if self.flags & IDLE_SAID != 0
            && self.level == LV_TOP
            && voice_stat == VOICE_PLAYEND
            && self.voice_line == random.range(2) as i32 + 4
        {
            sounds.push(TitleSound::Effect(2, 0x28));
            self.flags &= !IDLE_SAID;
        }
    }

    /// `Train_Input` on the explanation pages: confirm turns the page; after the last one a tutorial or battle
    /// lesson asks once more (line 0x28) and then leaves, any other is cleared at once. Cancel goes back.
    fn input_pages(
        &mut self,
        pad: Pad,
        save: &SaveData,
        font: &mut Font,
        sounds: &mut Vec<TitleSound>,
    ) {
        if pad.pressed & PAD_CROSS != 0 {
            self.page += 1;
            self.pose[1] = 0;
            if self.page < self.page_num {
                let cur = self.listed(self.cur_lesson()).cloned().unwrap_or_default();
                self.voice_line = cur.pages.get(self.page as usize).copied().unwrap_or(-1);
                if self.page_has_image() {
                    self.flags |= IMAGE_CHANGE;
                    self.flash.goto_label("fl_ss_view", true);
                }
                font.reset_icon_anim();
                sounds.push(TitleSound::Effect(2, 0x29));
            } else {
                let flags = self.find_lesson().map_or(0, |(_, l)| l.flags);
                sounds.push(TitleSound::Effect(1, 1));
                let plays = flags & (LESSON_TUTORIAL | LESSON_BATTLE) != 0;
                if plays && self.voice_line == 0x28 {
                    self.result = 1;
                    self.save_cursor();
                    self.store_cursor();
                    self.flags |= LEAVING;
                    if Self::is_cleared(save, self.sel[1], self.cur_lesson()) {
                        self.progress.flags |= MPTRAIN_AGAIN;
                    } else {
                        self.progress.flags |= MPTRAIN_FIRST_CLEAR;
                    }
                } else if plays {
                    self.voice_line = 0x28;
                    self.msg.set_text(self.data.text.clone());
                } else {
                    self.voice_line = -1;
                    self.msg.set_spacing(0, 0);
                    self.msg.set_text(self.data.msg_text[1].clone());
                    if Self::is_cleared(save, self.sel[1], self.cur_lesson()) {
                        self.level = LV_LESSON;
                        self.flash.goto_label("fl_ss_view_out", true);
                        self.next_row(sounds);
                        let line = self.voice_of(self.cur_lesson());
                        self.say(line, sounds);
                    } else {
                        self.level = LV_CLEAR;
                        self.flash.goto_label("fl_clear_in_1", true);
                        self.say(0x39, sounds);
                    }
                }
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.pose[1] = 0;
            self.level = LV_LESSON;
            self.flash.goto_label("fl_ss_view_out", true);
            self.plate2_goto(self.row, true);
            sounds.push(TitleSound::Effect(1, 2));
            self.msg.set_spacing(0, 0);
            self.msg.set_text(self.data.msg_text[1].clone());
            let line = self.voice_of(self.cur_lesson());
            self.say(line, sounds);
        }
    }

    /// `Train_Input` on "lesson cleared": the bit is saved, then Videl's lines for a cleared class and for all
    /// three, and back to the list on the next lesson.
    fn input_clear(
        &mut self,
        pad: Pad,
        voice_stat: i32,
        save: &mut SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        let class = self.sel[1];
        if self.flags & CLEAR_SAVED == 0 {
            let c = class as usize;
            let bits = save.train_clear(c) | 1i32.wrapping_shl(self.cur_lesson() as u32);
            save.set_train_clear(c, bits);
            self.flags |= CLEAR_SAVED;
        }
        let mut done = false;
        if self.voice_done(pad, voice_stat, 0x39) {
            if self.is_class_cleared(save, class) {
                self.say(class + 0x3A, sounds);
            } else {
                done = true;
            }
        } else if self.voice_done(pad, voice_stat, class + 0x3A) {
            if self.is_all_cleared(save) {
                self.say(0x3D, sounds);
            } else {
                done = true;
            }
        } else if self.voice_done(pad, voice_stat, 0x3D) {
            if self.is_all_cleared(save) {
                self.say(0x3E, sounds);
            } else {
                done = true;
            }
        } else if self.voice_done(pad, voice_stat, 0x3E) {
            done = true;
        }
        if pad.pressed & PAD_CROSS != 0 {
            sounds.push(TitleSound::Effect(1, 1));
        }
        if done {
            self.level = LV_LESSON;
            self.flash.goto_label("fl_clear_out", true);
            self.flags &= !CLEAR_SAVED;
            self.next_row(sounds);
            let line = self.voice_of(self.cur_lesson());
            self.say(line, sounds);
        }
    }

    /// `Train_UpdateImage`: the picture of the page (file 0x402 + page id) is read, then fades in.
    fn update_image(&mut self, loader: &mut dyn PictureLoader) {
        if self.flags & IMAGE_CHANGE != 0 {
            self.image_alpha = 0.0;
            self.flags &= !IMAGE_CHANGE;
            self.flags |= IMAGE_LOADING;
            self.loading = loader.portrait(IMAGE_FILE.wrapping_add(self.voice_line as u32));
        }
        if self.flags & IMAGE_LOADING != 0 {
            // File_UpdateRequests: the read is done by the next call.
            self.flags &= !IMAGE_LOADING;
            self.textures[IMAGE_SLOT] = self.loading;
        } else {
            self.image_alpha = (self.image_alpha + 0.05).min(1.0);
        }
    }

    /// `Train_Leave` (result 1): the practice battle of a battle lesson and / or the tutorial number of a
    /// tutorial.
    fn leave(&mut self) -> Option<BattleSetup> {
        let (index, rec) = self.find_lesson()?;
        let mut setup = None;
        if rec.flags & LESSON_BATTLE != 0 {
            // Per lesson id: the CPU level of both members and the item put in the last slot.
            const TABLE: [(u16, i32, u16); 8] = [
                (7, -1, 0x89),
                (0x11, 0xF, 0x99),
                (0x15, -1, 0x89),
                (0x16, -1, 0x89),
                (0x19, 0xF, 0x99),
                (0x1A, -1, 0x89),
                (0x1B, 0xF, 0x9A),
                (0x1C, -1, 0x89),
            ];
            let mut items = [0u16; 8];
            let mut cpu_level = -999;
            if let Some(&(_, level, item)) = TABLE.iter().find(|t| t.0 == rec.id) {
                cpu_level = level;
                items[7] = item;
            }
            // BattleSetup_SetRule(0, 5, bgm, 0, 7, stage, 0) and the two sides of one member each.
            let side = |control, pad, chara: u8| SideSetup {
                control,
                pad,
                change_allowed: 1,
                switch_enabled: 1,
                chara: i32::from(chara),
                costume: 0,
                cpu_level,
                items,
                ..SideSetup::default()
            };
            let mut s = BattleSetup {
                wide: false,
                screen_mode: 0,
                mode: 5,
                bgm: i32::from(rec.bgm),
                time_limit: 0,
                announcer: 7,
                stage: i32::from(rec.stage),
                stage_change: 0,
                sides: [side(0, 0, rec.chara[0]), side(2, 1, rec.chara[1])],
                pool: Vec::new(),
                tutorial: 0,
            };
            s.finish();
            self.progress.flags |= MPTRAIN_BATTLE;
            setup = Some(s);
        }
        if rec.flags & LESSON_TUTORIAL != 0 {
            let start: i32 = CLASS_LESSONS[..self.class()].iter().sum::<usize>() as i32;
            self.progress.tutorial = index as i32 + start;
            self.progress.flags |= MPTRAIN_TUTORIAL;
        }
        // gProgress->tutorial as the battle reads it (a battle-only lesson leaves the last one).
        if let Some(s) = &mut setup {
            s.tutorial = self.progress.tutorial;
        }
        setup
    }

    fn set_uv(&mut self, parent: Option<&str>, name: &str, uv: Uv) {
        let r = self.flash.find(parent, name);
        self.flash.clip_set_uv(r, uv);
    }

    fn set_visible(&mut self, parent: Option<&str>, name: &str, on: bool) {
        let r = self.flash.find(parent, name);
        self.flash.clip_set_flags(r, PROP_VISIBLE, on);
    }

    /// `Train_DrawPlates`: the text strips of the six menu plates (texture 0 on the top menu, 2 on the classes).
    fn draw_plates(&mut self) {
        let tex = if self.level == LV_CLASS { 2 } else { 0 };
        for i in 0..6 {
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
                self.flash.clip_set_tex(r, tex);
            }
            self.set_uv(Some(&name), "mc_menu_text_on", uv);
            self.set_uv(Some(&name), "mc_menu_text_off", uv);
            self.set_visible(Some(&name), "mc_icon_wii_style", false);
        }
    }

    /// `Train_DrawArrows`: the arrows' rectangles; those that lead nowhere are hidden.
    fn draw_arrows(&mut self) {
        let arrow = |u0, v0| Uv {
            u0,
            v0,
            u1: u0 + 0x20,
            v1: v0 + 0x20,
            tex: 0,
        };
        self.set_uv(
            Some("mc_yajirusi_up"),
            "mc_yajirusi_icon_up",
            arrow(0, 0x20),
        );
        self.set_uv(
            Some("mc_yajirusi_down"),
            "mc_yajirusi_icon_down",
            arrow(0x20, 0x20),
        );
        self.set_uv(
            Some("mc_yajirusi_right"),
            "mc_yajirusi_icon_right",
            arrow(0x20, 0),
        );
        if self.top == 0 {
            self.set_visible(Some("mc_yajirusi_up"), "mc_yajirusi_icon_up", false);
        } else if self.top == self.count(self.class()) - ROWS {
            self.set_visible(Some("mc_yajirusi_down"), "mc_yajirusi_icon_down", false);
        }
        let more = self.page < self.page_num - 1;
        self.set_visible(None, "mc_ss_next_2", more);
        self.set_visible(None, "mc_ss_next", false);
    }

    /// `Train_DrawClearIcon`: the animated "cleared" icon on the list and the clear screen, the still one on the
    /// lesson's pages.
    fn draw_clear_icon(&mut self, save: &SaveData) {
        match self.level {
            LV_INTRO | LV_PAGES => {
                let cleared = Self::is_cleared(save, self.sel[1], self.cur_lesson());
                self.set_visible(None, "mc_icon_training_clear_in", false);
                self.set_visible(Some("mc_menu_plate_2_5"), "mc_icon_training_clear", cleared);
            }
            LV_CLEAR | LV_LESSON => {
                self.set_visible(None, "mc_icon_training_clear_in", true);
                self.set_visible(Some("mc_menu_plate_2_5"), "mc_icon_training_clear", false);
            }
            _ => {}
        }
    }

    /// `Train_DrawGuides`: each guide's pose, and the eyes and mouth of the one talking (Videl below the top
    /// menu). Great Saiyaman has no eye clip.
    fn draw_guides(&mut self, voice_stat: i32, random: &mut GameRandom) {
        const GUIDE: [&str; 2] = ["mc_guide_saiyaman", "mc_guide_bidel"];
        let eye = |i: usize, c: char| match i {
            0 => String::new(),
            _ => format!("mc_guide_{c}_bidel_eye"),
        };
        let mouth = |i: usize, c: char| match i {
            0 => format!("mc_guide_{c}_saiyaman_mouth"),
            _ => format!("mc_guide_{c}_bidel_mouth"),
        };
        let who = usize::from(self.level != LV_TOP);
        for i in 0..2 {
            let other = if self.pose[i] == 0 { 'b' } else { 'a' };
            let own = (b'a' + self.pose[i] as u8) as char;
            self.set_visible(None, &eye(i, other), false);
            if who == i {
                let r = self.flash.find(None, &eye(i, own));
                anim::blink(&mut self.flash, r, &mut self.blink[i], 0, random);
                self.set_visible(None, &mouth(i, other), false);
                let r = self.flash.find(None, &mouth(i, own));
                if voice_stat == VOICE_PLAYING {
                    anim::talk(
                        &mut self.flash,
                        r,
                        &mut self.talk[i],
                        0,
                        &mut self.lip,
                        voice_stat,
                        random,
                    );
                } else {
                    anim::show_next(&mut self.flash, r, 0);
                }
            }
            let r = self.flash.find(None, GUIDE[i]);
            self.flash.clip_set_tex(r, self.pose[i]);
        }
    }

    /// `Train_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        save: &SaveData,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        let r = self.flash.find(None, "mc_ss_speace");
        self.flash.clip_set_alpha(r, self.image_alpha);
        self.draw_plates();
        self.draw_arrows();
        // Train_DrawList: the lesson names on the five list plates, each with its "cleared" icon.
        let mut texts: Vec<TextDraw> = Vec::new();
        let list_box = TextBox::preset(2)
            .with_line_y([9, 0, 0, 0, 0])
            .with_max_size(0x100, 0x40)
            .with_offset(0xA, 4);
        let text = self.data.text.clone();
        for i in 0..5 {
            let name = format!("mc_menu_plate_2_{}", i + 1);
            let lesson = match i {
                3 => self.extra,
                4 => self.extra2,
                _ => self.top + i,
            };
            let line = self.name_line(lesson);
            let r = self.flash.find(Some(&name), "mc_dammy_text");
            if let Some(t) =
                list_box.attach_line(&mut self.flash, r, 0x400 + i as u32, &text, line, font)
            {
                texts.push(t);
            }
            if i != 4 {
                let cleared = Self::is_cleared(save, self.sel[1], lesson);
                self.set_visible(Some(&name), "mc_icon_training_clear", cleared);
            }
        }
        self.draw_clear_icon(save);
        if self.flags & LEAVING == 0 {
            // Train_SetClip: the two "mc_ss" clips draw inside the list's scissor.
            for name in ["mc_ss_speace", "mc_ss_dammy"] {
                let r = self.flash.find(None, name);
                self.flash.clip_set_scissor(r, Some(LIST_CLIP));
            }
        }
        self.draw_guides(voice_stat, random);
        // Train_DrawBg: the clouds scroll; the background picture.
        let r = self.flash.find(None, "mc_bg_cloud");
        let cloud = Uv {
            u0: 0,
            v0: 0,
            u1: 0x200,
            v1: 0x80,
            tex: 0,
        };
        anim::scroll(&mut self.flash, r, cloud, &mut self.cloud, 0.047407407);
        quads.extend(
            picture_quads(self.data.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        // Train_DrawTitle: the class's title.
        let v0 = self.sel[1] * 0x40;
        self.set_uv(
            None,
            "mc_title_class",
            Uv {
                u0: 0,
                v0,
                u1: 0x100,
                v1: v0 + 0x40,
                tex: 0,
            },
        );
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&flash_quads, textures));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        // Train_DrawIconWin: icon 2 on the class menu, 0 elsewhere.
        let icon = self.data.layout.icons[usize::from(self.level == LV_CLASS)];
        self.icon.set_texture(4, icon);
        self.icon.draw(quads);
        self.msg.draw(self.voice_line, font, quads);
    }
}

/// `Train_Clamp`: true when `v` had to be clamped to `lo..=hi`.
fn clamp(v: &mut i32, lo: i32, hi: i32) -> bool {
    if *v < lo {
        *v = lo;
        true
    } else if *v > hi {
        *v = hi;
        true
    } else {
        false
    }
}

/// `Train_Wrap`.
fn wrap(v: &mut i32, lo: i32, hi: i32) {
    if *v < lo {
        *v = hi;
    } else if *v > hi {
        *v = lo;
    }
}

/// The pad words the input reads (`gamePressed`, `gameRepeat`).
#[derive(Clone, Copy, Debug)]
struct Pad {
    pressed: u32,
    repeat: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_assets() -> Option<TrainAssets> {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};
        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let mut image = Iso9660::open(&iso).ok()?;
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        Some(TrainAssets::parse(&res).unwrap())
    }

    struct Pictures;

    impl PictureLoader for Pictures {
        fn portrait(&mut self, file_id: u32) -> Option<usize> {
            Some(file_id as usize)
        }

        fn picture(&mut self, _: u32) -> Option<usize> {
            None
        }
    }

    /// The lists of the real menu: the skip list leaves out the three last lessons of class 2 (their only pages).
    #[test]
    fn real_lists_leave_out_the_skipped_lessons() {
        let Some(a) = real_assets() else { return };
        let lists = build_lists(&a.data);
        assert_eq!(lists.each_ref().map(|l| l.len()), [13, 11, 10]);
        // Lesson 1 (id 3) loses pages 9 and 10 of its 19.
        assert_eq!(lists[0][0].pages.len(), 17);
        assert!(!lists[0][0].pages.contains(&9));
    }

    /// Runs the real training menu from the ISO in `rom/`, when it is there: the classes, a lesson's pages and
    /// its battle (lesson id 7, the dash tutorial: tutorial 5), then the menu reopened after it with the
    /// "cleared" sequence, which saves the lesson's bit.
    #[test]
    fn real_training_menu_leaves_for_a_lesson_and_clears_it() {
        let Some(assets) = real_assets() else { return };
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = TrainLayout::new(0, |key| firsts[&key]);
        let mut random = GameRandom::from_key(&[3, 1, 4]);
        let mut font = Font::new(Vec::new(), 20);
        let mut save = SaveData::default();
        let mut screen = TrainScreen::new(&data, TrainProgress::default(), &mut random).unwrap();
        let mut voices = Vec::new();
        macro_rules! press {
            ($screen:expr, $save:expr, $bits:expr) => {
                press(
                    &mut $screen,
                    &mut $save,
                    $bits,
                    &mut random,
                    &mut font,
                    &mut voices,
                )
            };
        }
        fn press(
            screen: &mut TrainScreen,
            save: &mut SaveData,
            bits: u32,
            random: &mut GameRandom,
            font: &mut Font,
            voices: &mut Vec<u32>,
        ) {
            for i in 0..40 {
                let b = if i == 0 { bits } else { 0 };
                let out = screen.run_frame(b, b, VOICE_PLAYEND, save, &mut Pictures, random, font);
                voices.extend(out.sounds.into_iter().filter_map(|s| match s {
                    TitleSound::Voice(v) => Some(v),
                    _ => None,
                }));
                if screen.result().is_some() {
                    break;
                }
            }
        }
        for _ in 0..10 {
            press!(screen, save, 0);
        }
        assert_eq!(screen.level, LV_TOP);
        // Great Saiyaman's greeting (bank A).
        let a = [VOICE_BASE_A, VOICE_BASE_A + anim::VOICE_LANG2_OFFSET];
        assert!(a.contains(&voices[0]), "{voices:X?}");
        press!(screen, save, PAD_DOWN);
        assert_eq!(screen.sel[0], 1);
        press!(screen, save, PAD_CROSS);
        assert_eq!(screen.level, LV_CLASS);
        press!(screen, save, PAD_CROSS);
        assert_eq!(screen.level, LV_LESSON);
        for _ in 0..5 {
            press!(screen, save, PAD_DOWN);
        }
        assert_eq!((screen.top, screen.row), (3, 2));
        assert_eq!(screen.name_line(screen.cur_lesson()), 7);
        press!(screen, save, PAD_CROSS);
        // The introduction's line has ended at once: the pages.
        assert_eq!(screen.level, LV_PAGES);
        assert_eq!((screen.voice_line, screen.page_num), (73, 13));
        // The page's picture is file 0x402 + 73.
        assert_eq!(screen.textures[IMAGE_SLOT], Some(0x402 + 73));
        for _ in 0..13 {
            press!(screen, save, PAD_CROSS);
        }
        assert_eq!(screen.voice_line, 0x28);
        press!(screen, save, PAD_CROSS);
        let Some(TrainResult::Lesson(Some(setup))) = screen.result() else {
            panic!("{:?}", screen.result());
        };
        assert_eq!(
            (
                setup.mode,
                setup.stage,
                setup.bgm,
                setup.announcer,
                setup.tutorial
            ),
            (5, 22, 3, 7, 5)
        );
        assert_eq!([setup.sides[0].chara, setup.sides[1].chara], [46, 17]);
        assert_eq!(
            (
                setup.sides[1].control,
                setup.sides[1].cpu_level,
                setup.sides[0].items[7]
            ),
            (2, -1, 0x89)
        );
        let progress = screen.progress();
        assert_eq!(
            progress.flags,
            MPTRAIN_FIRST_CLEAR | MPTRAIN_BATTLE | MPTRAIN_TUTORIAL
        );
        assert_eq!(progress.cursor, [1, 0, 5, 0, 0]);
        // Back from the battle: the "cleared" sequence on the lesson, which saves its bit and moves on.
        let mut screen = TrainScreen::new(&data, progress, &mut random).unwrap();
        voices.clear();
        press!(screen, save, 0);
        // Videl's "cleared" line (0x39, bank B), then the next lesson's.
        let b = [VOICE_BASE_B, VOICE_BASE_B + anim::VOICE_LANG2_OFFSET];
        assert!(
            b.iter().any(|v| voices.contains(&(v + 0x39))),
            "{voices:X?}"
        );
        assert_eq!(save.train_clear(0), 1 << 5);
        assert_eq!(screen.level, LV_LESSON);
        assert_eq!(screen.cur_lesson(), 6);
        assert_eq!(screen.progress().flags, 0);
    }
}
