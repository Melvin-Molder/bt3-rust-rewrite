//! The story mode's episode list (`ModeMenu_Run(2)`, src/menu/mode_menu.c, progress mode 7): the chosen saga's
//! guide with voice, a horizontal list of up to 16 episodes of which three plates show, the large picture of the
//! episode under the cursor (streamed from the disc), and once one is chosen its description, scrolled in and read
//! out line by line. Its background (`ModeBg`, src/menu/mode_background.c) is the saga's still picture with a
//! movie of scrolling clouds and smoke; the saga's outro (mode 9) shares it.
//!
//! An episode confirmed sets up the story battle of its script (`BattleSetup_SetScript`).

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::PictureLoader;
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_MASK, PROP_MASK_SET, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_LEFT, PAD_RIGHT, PAD_START, PAD_TRIANGLE, PadRepeat, TitleFrame, TitleSound,
};
use crate::windows::{MsgWin, TextBox, TextDraw, draw_texts};

/// `ModeMenu_Run(2)`: section 2 of `gMenuArc2`.
pub const ARCHIVE_SECTION: usize = 2;
/// File `gProgress->baseFile + 0x10 + saga`: the saga's own pack.
pub const SAGA_FILE: u32 = 0x1C1 + 0x10;
const BGM: u32 = 0x10B1C;
/// `MODEMENU_NARR_VOICE`: the narrated description lines.
const NARR_VOICE: u32 = 0x84B6;
const FADE_FRAMES: i32 = 0x14;
const ROWS: usize = 3;
/// Episodes per saga.
pub(crate) const EPISODES: [i32; 8] = [3, 4, 7, 5, 16, 5, 4, 4];

const FLAG_CHOSEN: u32 = 1;
const FLAG_LEAVING: u32 = 2;
const FLAG_STARTED: u32 = 4;
const FLAG_GREETED: u32 = 8;
const FLAG_SCROLLING: u32 = 0x10;
const FLAG_NEXT: u32 = 0x20;
const FLAG_IMAGE_CHANGE: u32 = 0x40;
const FLAG_IMAGE_READY: u32 = 0x80;

const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;
const LOAD_SHOWN: i32 = 4;
const LOAD_ABORT: i32 = 5;
const LOAD_RESTART: i32 = 6;

const STEP_GUIDE: i32 = 1;
const STEP_GUIDE_WAIT: i32 = 2;
const STEP_OPEN: i32 = 3;
const STEP_OPENED: i32 = 4;
const STEP_NARR_INIT: i32 = 51;
const STEP_NARR_START: i32 = 52;
const STEP_NARR_LINE: i32 = 53;
const STEP_NARR_WAIT: i32 = 54;
const STEP_NARR_END: i32 = 55;
const STEP_GO: i32 = 101;

/// `SAVESLOT_GREET_FIRST`: the guide greets with its other line once.
const SLOT_GREET_FIRST: i32 = 0x40;

/// The saga pack's sections are keyed past the screen's own.
pub(crate) const SAGA: usize = 0x100;

const fn saga_tex(section: usize, entry: usize) -> ImageTexture {
    tex(SAGA + section, entry)
}

/// `ModeMenu_GetLine`: an episode's number through all sagas (with 20..23 rotated by one): its description's line
/// and its battle script.
pub(crate) fn episode_line(saga: i32, item: i32) -> i32 {
    let line = EPISODES[..saga.clamp(0, 8) as usize].iter().sum::<i32>() + item;
    match line {
        20 => 21,
        21 => 22,
        22 => 23,
        23 => 20,
        _ => line,
    }
}

/// `gModeMenu->tex[n]` of `ModeMenu_Init` for saga `saga` (slot 9 is the streamed picture).
fn flash_textures(saga: usize) -> [ImageTexture; 41] {
    let mut t = [None; 41];
    let icons: [[usize; 3]; 8] = [
        [10, 11, 12],
        [13, 14, 15],
        [16, 17, 18],
        [19, 21, 20],
        [22, 23, 24],
        [25, 26, 27],
        [28, 29, 30],
        [31, 32, 33],
    ];
    for (&slot, e) in icons[saga.min(7)].iter().zip([0, 1, 3]) {
        t[slot] = saga_tex(7, e);
    }
    for (slot, e) in [
        (8, 0),
        (7, 1),
        (34, 2),
        (38, 4),
        (0, 5),
        (6, 6),
        (5, 7),
        (3, 8),
    ] {
        t[slot] = tex(1, e);
    }
    t[2] = tex(2, 0);
    t[4] = saga_tex(2, 0);
    t[36] = saga_tex(2, 1);
    t[37] = saga_tex(3, 0);
    t[1] = saga_tex(3, 1);
    t[35] = tex(7, 1);
    t
}

/// `gModeBg->tex[n]` of `ModeBg_Init`: section 10 of the saga's pack, by saga (saga 3 has no movie).
fn bg_textures(saga: usize) -> [ImageTexture; 14] {
    let mut t = [None; 14];
    let map: &[(usize, usize)] = match saga {
        0 => &[(0, 3), (1, 2), (2, 1), (3, 0), (4, 4), (5, 5)],
        1 => &[(0, 2), (1, 1), (2, 0), (3, 3), (4, 5), (5, 4)],
        4 => &[(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5)],
        2 | 5 => &[(0, 2), (1, 1), (2, 0)],
        6 => &[
            (0, 12),
            (1, 11),
            (2, 10),
            (10, 0),
            (11, 1),
            (12, 2),
            (6, 3),
            (7, 4),
            (8, 5),
            (9, 6),
            (3, 7),
            (4, 8),
            (5, 9),
        ],
        7 => &[(0, 3), (1, 0), (2, 1), (3, 2)],
        _ => &[],
    };
    for &(slot, e) in map {
        t[slot] = saga_tex(10, e);
    }
    t
}

/// `MsgWin_Init(MPACK_AT(res, 8))`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((8, 1), 0)), Some(((8, 1), 1)), None];

/// One episode's description (`ModeMenuDescr`, the saga pack's section 6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Descr {
    pub count: i32,
    pub line: i32,
    pub delay: [i32; 6],
}

/// What `ModeMenu_Init` reads from its section of `gMenuArc2`.
pub struct ModeMenuAssets {
    pub data: ModeMenuData,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct ModeMenuData {
    pub fod: Vec<u8>,
    /// Section 5: the saga outro's movie (`HistOutro_Run(2)` shares the section).
    pub outro_fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    /// Section 6: the descriptions' lines.
    pub text: Vec<u8>,
    pub msg: Vec<Option<usize>>,
}

impl ModeMenuAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let screen: Vec<ImageTexture> = flash_textures(0)
            .into_iter()
            .chain(crate::hist_outro::flash_textures(0))
            .filter(|t| t.is_some_and(|((s, _), _)| s < SAGA))
            .collect();
        let files = decode_files(res, [&screen[..], &MSG_TEXTURES])?;
        Ok(Self {
            data: ModeMenuData {
                fod: pack_section(res, 3)?.to_vec(),
                outro_fod: pack_section(res, 5)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 8)?, 2)?.to_vec(),
                text: pack_section(res, 6)?.to_vec(),
                msg: Vec::new(),
            },
            files,
        })
    }

    /// The message window's texture indices.
    pub fn msg_layout(first: impl Fn(FileKey) -> usize) -> Vec<Option<usize>> {
        image_indices(&MSG_TEXTURES, &first)
    }
}

/// What a saga's own pack gives the episode list and the outro.
pub struct SagaAssets {
    pub data: SagaData,
    pub background: Vec<RgbaTexture>,
    /// Keyed `SAGA + section`.
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct SagaData {
    pub saga: usize,
    /// Section 9: the background movie (none for saga 3).
    pub bg_fod: Option<Vec<u8>>,
    /// Section 4: the guide's message lines; section 5: its voice lines' mouth keys.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    pub(crate) descr: Vec<Descr>,
    pub layout: SagaLayout,
}

#[derive(Clone, Debug, Default)]
pub struct SagaLayout {
    pub background: usize,
    /// The episode list's movie (the screen's textures and the saga's) and the background movie's.
    pub flash: Vec<Option<usize>>,
    pub bg: Vec<Option<usize>>,
    /// The outro's movie.
    pub outro: Vec<Option<usize>>,
}

impl SagaLayout {
    /// `first` gives the screen's files and, keyed `SAGA + section`, the saga's.
    pub fn new(saga: usize, background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&flash_textures(saga), &first),
            bg: image_indices(&bg_textures(saga), &first),
            outro: image_indices(&crate::hist_outro::flash_textures(saga), &first),
        }
    }
}

impl SagaAssets {
    /// The unpacked file `SAGA_FILE + saga`.
    pub fn parse(saga: usize, res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let section =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        let own: Vec<ImageTexture> = flash_textures(saga)
            .into_iter()
            .chain(bg_textures(saga))
            .chain(crate::hist_outro::flash_textures(saga))
            .filter_map(|t| t.filter(|((s, _), _)| *s >= SAGA))
            .map(|((s, k), e)| Some(((s - SAGA, k), e)))
            .collect();
        let files = decode_files(res, [&own[..]])?
            .into_iter()
            .map(|((s, k), t)| ((s + SAGA, k), t))
            .collect();
        let table = section(6)?;
        let mut descr: Vec<Descr> = table
            .chunks_exact(0x20)
            .map(|e| {
                let w = |i: usize| i32::from_le_bytes(e[i * 4..i * 4 + 4].try_into().unwrap());
                Descr {
                    count: w(0),
                    line: w(1),
                    delay: std::array::from_fn(|k| w(2 + k)),
                }
            })
            .collect();
        // ModeMenu_Init: saga 7's first description has two lines.
        if saga == 7
            && let Some(d) = descr.first_mut()
        {
            d.count = 2;
        }
        Ok(Self {
            data: SagaData {
                saga,
                bg_fod: if saga == 3 { None } else { Some(section(9)?) },
                msg_text: section(4)?,
                lips: section(5)?,
                descr,
                layout: SagaLayout::default(),
            },
            background: decode_picture(res, 1)?,
            files,
        })
    }
}

/// `ModeBg`: the saga's still picture and its movie of scrolling layers.
pub(crate) struct ModeBg {
    saga: usize,
    background: usize,
    flash: Option<Flash>,
    textures: Vec<Option<usize>>,
    scroll_x: [f32; 2],
    scroll_y: [f32; 2],
}

impl ModeBg {
    /// `ModeBg_Init`.
    pub(crate) fn new(saga: &SagaData) -> Result<Self, Box<dyn Error>> {
        let flash = match &saga.bg_fod {
            Some(fod) => {
                let mut f = Flash::create(fod)?;
                f.play(1);
                Some(f)
            }
            None => None,
        };
        Ok(Self {
            saga: saga.saga,
            background: saga.layout.background,
            flash,
            textures: saga.layout.bg.clone(),
            scroll_x: [0.0; 2],
            scroll_y: [0.0; 2],
        })
    }

    /// `ModeBg_Draw`: the movie advanced, the picture, the movie with its layers scrolled.
    pub(crate) fn draw(&mut self, quads: &mut Vec<ScreenQuad>) {
        if let Some(f) = &mut self.flash {
            f.advance();
        }
        quads.extend(
            picture_quads(self.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let Some(f) = &mut self.flash else {
            return;
        };
        let uv = |u1, v1| Uv {
            u0: 0,
            v0: 0,
            u1,
            v1,
            tex: 0,
        };
        let kumo = |f: &mut Flash, x: &mut f32, v1, dx| {
            let clip = f.find(None, "mc_kumo");
            anim::scroll(f, clip, uv(0x200, v1), x, dx);
        };
        match self.saga {
            0 => {
                let speed = [0.609_523_8, 0.474_074_08, 1.066_666_7, 0.853_333_34];
                for i in 0..2 {
                    let clip = f.find(None, &format!("mc_kumo_{}", i + 1));
                    anim::scroll(f, clip, uv(0x200, 0x80), &mut self.scroll_x[i], speed[i]);
                    let clip = f.find(None, &format!("mc_kemuri_{}", i + 1));
                    anim::scroll_y(f, clip, uv(0x80, 0x200), &mut self.scroll_y[i], speed[i]);
                }
            }
            1 => kumo(f, &mut self.scroll_x[0], 0x100, -0.568_888_9),
            2 | 6 => kumo(f, &mut self.scroll_x[0], 0x100, -0.426_666_67),
            5 => kumo(f, &mut self.scroll_x[0], 0x80, -0.341_333_33),
            _ => {}
        }
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        quads.extend(screen_quads(&f.draw(&has_texture), textures));
    }
}

/// How `ModeMenu_Run` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeMenuResult {
    /// 0: back to the saga select.
    Back,
    /// 1: the episode's story battle, script `n` (`BattleSetup_SetScript`).
    Battle(i32),
}

pub struct ModeMenuScreen {
    textures: Vec<Option<usize>>,
    text: Vec<u8>,
    lips: Vec<u8>,
    descr: Vec<Descr>,
    bg: ModeBg,
    flash: Flash,
    msg: MsgWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    saga: i32,
    flags: u32,
    cursor: i32,
    timer: i32,
    focus: i32,
    voice_base: u32,
    line_base: i32,
    image_base: u32,
    step: i32,
    voice_line: i32,
    blink: i32,
    talk: i32,
    items: Vec<i32>,
    top: i32,
    bottom: i32,
    extra: i32,
    item_max: i32,
    scroll: f32,
    scroll_max: f32,
    narr_wait: i32,
    narr_line: i32,
    load_state: i32,
    loading: Option<usize>,
    image_alpha: f32,
    result: bool,
    item: i32,
    done: Option<ModeMenuResult>,
}

impl ModeMenuScreen {
    /// `ModeMenu_Init(2)` and the first `ColorFade_StartIn` (white coming from the saga select, `prev_mode` 6);
    /// `item` is the episode last played, taken back after a battle (`prev_mode` 8 or 10).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &ModeMenuData,
        saga: &SagaData,
        item: i32,
        prev_mode: i32,
        save: &SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) -> Result<Self, Box<dyn Error>> {
        let s = saga.saga;
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let msg = MsgWin::new(&data.msg_fod, data.msg.clone(), saga.msg_text.clone(), true)?;
        let (item_max, voice_base, line_base, image_base) = match s {
            0 => (3, 0x8537, 0x13, 0x3CE),
            1 => (4, 0x8423, 0xC, 0x3D1),
            2 => (7, 0x83B4, 0x12, 0x3D5),
            3 => (5, 0x83DB, 0x15, 0x3DC),
            4 => (16, 0x8484, 0x13, 0x3E1),
            5 => (5, 0x8440, 0x14, 0x3F1),
            6 => (4, 0x8402, 0x10, 0x3F6),
            _ => (4, 0x8466, 0xE, 0x3FA),
        };
        let listed = save.slot_val(s, 0);
        let mut items: Vec<i32> = (0..item_max).filter(|&i| listed & (1 << i) != 0).collect();
        while items.len() < ROWS {
            items.push(item_max);
        }
        let mut cursor = 0;
        if prev_mode == 10 || prev_mode == 8 {
            if let Some(i) = items.iter().position(|&i| i == item) {
                cursor = i as i32;
            }
        }
        let mut top = cursor;
        let mut bottom = top + 2;
        while bottom >= items.len() as i32 {
            top -= 1;
            bottom -= 1;
        }
        let (bgm, ambient) = match s {
            0 => (BGM, Some(0x10BDC)),
            1 => (BGM, Some(0x10BDD)),
            2 => (BGM, Some(0x10BDE)),
            5 => (0x10B1E, None),
            6 => (0x10B28, None),
            _ => (BGM, None),
        };
        sounds.push(TitleSound::Bgm(bgm));
        sounds.extend(ambient.map(TitleSound::StreamSe));
        Ok(Self {
            textures: saga.layout.flash.clone(),
            text: data.text.clone(),
            lips: saga.lips.clone(),
            descr: saga.descr.clone(),
            bg: ModeBg::new(saga)?,
            flash,
            msg,
            fade: ColorFade::start_in(prev_mode != 6, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            saga: s as i32,
            flags: 0,
            cursor,
            timer: 0,
            focus: 0,
            voice_base,
            line_base,
            image_base,
            step: 0,
            voice_line: -1,
            blink: random.range(0x20) as i32,
            talk: 0,
            items,
            top,
            bottom,
            extra: 0,
            item_max,
            scroll: 0.0,
            scroll_max: 0.0,
            narr_wait: 0,
            narr_line: 0,
            load_state: LOAD_REQUEST,
            loading: None,
            image_alpha: 0.0,
            result: true,
            item,
            done: None,
        })
    }

    pub fn result(&self) -> Option<ModeMenuResult> {
        self.done
    }

    /// `gProgress->subMenuItem`.
    pub fn item(&self) -> i32 {
        self.item
    }

    fn cur_item(&self) -> i32 {
        self.items[self.cursor.clamp(0, self.items.len() as i32 - 1) as usize]
    }

    fn descr(&self, item: i32) -> Descr {
        self.descr
            .get(item.max(0) as usize)
            .copied()
            .unwrap_or_default()
    }

    /// `MODEMENU_VOICE(line)`: a line of the guide's set.
    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        self.voice_line = line;
        anim::play_with_subtitle(&self.lips, self.voice_base, line, &mut self.lip, sounds);
    }

    /// `ModeMenu_UpdateImage`: one step of the background loader of the episode's picture.
    fn update_image(&mut self, loader: &mut dyn PictureLoader) {
        match self.load_state {
            LOAD_ABORT => self.load_state = LOAD_RESTART,
            LOAD_RESTART => {
                self.flags &= !FLAG_IMAGE_CHANGE;
                self.load_state = LOAD_REQUEST;
            }
            LOAD_REQUEST => {
                self.loading = loader.portrait(self.image_base + self.cur_item() as u32);
                self.load_state = LOAD_READ;
            }
            LOAD_READ => self.load_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.textures[9] = self.loading;
                self.flags |= FLAG_IMAGE_READY;
                self.load_state = LOAD_SHOWN;
            }
            LOAD_SHOWN => {
                if self.flags & FLAG_IMAGE_CHANGE != 0 {
                    self.textures[9] = None;
                    self.load_state = LOAD_RESTART;
                }
            }
            _ => {}
        }
    }

    /// `ModeMenu_ChangeImage`.
    fn change_image(&mut self) {
        if self.flags & FLAG_IMAGE_READY != 0 {
            self.flags ^= FLAG_IMAGE_READY;
        } else {
            self.load_state = LOAD_ABORT;
        }
        self.flags |= FLAG_IMAGE_CHANGE;
    }

    /// `ModeMenu_PlateGoto(0, 0, label)`.
    fn plate_goto(&mut self, label: &str) {
        let clip = self.flash.find(
            None,
            &format!("mc_menu_plate_{}", self.cursor - self.top + 1),
        );
        self.flash.clip_goto_label(clip, label);
    }

    /// One frame of the `ModeMenu_Run` loop. `voice_stat` is `Voice_GetStat(0)`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        loader: &mut dyn PictureLoader,
        save: &mut SaveData,
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
        self.update_image(loader);
        let repeat = self.pad.update(held, pressed);
        let sounds = &mut frame.sounds;
        self.fade.update();
        self.update(voice_stat, sounds);
        self.update_step(pressed, voice_stat, sounds);
        self.draw(voice_stat, save, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & FLAG_GREETED == 0
            && self.flash.flags() & FLASH_PAD != 0
        {
            self.flags |= FLAG_GREETED;
            let s = self.saga as usize;
            let flags = save.slot_flags(s);
            let line = if flags & SLOT_GREET_FIRST != 0 {
                save.set_slot_flags(s, flags & !SLOT_GREET_FIRST);
                self.line_base + 3
            } else {
                self.line_base
            };
            self.say(line, sounds);
        }
        if self.fade.is_fading_out() {
            sounds.push(TitleSound::VoiceFadeOutStep);
            sounds.push(TitleSound::BgmFadeOutStep);
            sounds.push(TitleSound::StreamSeFadeOutStep(0));
            return frame;
        }
        if self.fade.is_out_done() {
            if self.load_state == LOAD_SHOWN {
                self.done = Some(if self.result {
                    // The episode is no longer new; its script is the battle's.
                    let (s, item) = (self.saga as usize, self.cur_item());
                    save.set_slot_val(s, 2, save.slot_val(s, 2) & !(1 << item));
                    ModeMenuResult::Battle(episode_line(self.saga, item))
                } else {
                    ModeMenuResult::Back
                });
            }
        } else if self.flags & FLAG_LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.item = self.cur_item();
            }
        } else if self.step == 0 {
            self.input(pressed, repeat, sounds);
        }
        frame
    }

    /// `ModeMenu_Update`: after the greeting the guide presents the episode; the description's scroll.
    fn update(&mut self, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if self.flags & FLAG_GREETED != 0
            && (self.voice_line == self.line_base || self.voice_line == self.line_base + 3)
            && voice_stat == VOICE_PLAYEND
        {
            self.say(self.cur_item() + self.line_base + 4, sounds);
        }
        self.flash.advance();
        if self.flags & FLAG_STARTED == 0 && self.flash.trig() & 2 != 0 {
            self.msg.open();
        }
        if self.flags & FLAG_SCROLLING != 0 {
            self.scroll += 0.233_333_33;
            if self.scroll >= self.scroll_max {
                self.scroll = self.scroll_max;
                self.flags &= !FLAG_SCROLLING;
                self.flags |= FLAG_NEXT;
            }
        }
    }

    /// `ModeMenu_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        save: &SaveData,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        self.bg.draw(quads);
        let f = &mut self.flash;
        let name = format!("mc_guide_{:02}", self.saga);
        let eye = f.find(Some(&name), &format!("{name}_eye"));
        anim::blink(f, eye, &mut self.blink, 0, random);
        let mouth = f.find(Some(&name), &format!("{name}_mouth"));
        anim::talk(
            f,
            mouth,
            &mut self.talk,
            0,
            &mut self.lip,
            voice_stat,
            random,
        );
        if self.flags & FLAG_IMAGE_READY != 0 {
            self.image_alpha = (self.image_alpha + 0.075).min(1.0);
        } else {
            self.image_alpha = 0.0;
        }
        let clip = f.find(None, "mc_image_l_1");
        f.clip_set_flags(clip, PROP_MASK, true);
        let clip = f.find(None, "mc_battle_mask");
        f.clip_set_alpha(clip, self.image_alpha);
        f.clip_set_flags(clip, PROP_MASK_SET, true);
        let item = self.cur_item();
        let clip = self.flash.find(None, "mc_battle_text_1");
        self.flash.clip_set_alpha(clip, self.image_alpha);
        self.flash.clip_set_uv(
            clip,
            Uv {
                u0: 0,
                v0: (item % 4) * 0x40,
                u1: 0x200,
                v1: (item % 4) * 0x40 + 0x40,
                tex: 0,
            },
        );
        self.flash.clip_set_tex(clip, item / 4);
        let s = self.saga as usize;
        let bit = |k: usize, item: i32| save.slot_val(s, k) & (1 << (item & 31)) != 0;
        let clip = self.flash.find(None, "mc_clear");
        if bit(1, item) {
            self.flash.clip_set_flags(clip, PROP_VISIBLE, true);
            self.flash.clip_set_alpha(clip, self.image_alpha);
        } else {
            self.flash.clip_set_flags(clip, PROP_VISIBLE, false);
        }
        let arrows = [
            ("mc_yajirusi_left", "mc_yajirusi_icon_left", self.top != 0),
            (
                "mc_yajirusi_right",
                "mc_yajirusi_icon_right",
                self.bottom != self.items.len() as i32 - 1,
            ),
        ];
        for (i, (parent, icon, show)) in arrows.into_iter().enumerate() {
            let clip = self.flash.find(Some(parent), icon);
            self.flash.clip_set_flags(clip, PROP_VISIBLE, show);
            let x = i as i32 * 0x20;
            self.flash.clip_set_uv(
                clip,
                Uv {
                    u0: x,
                    v0: 0,
                    u1: x + 0x20,
                    v1: 0x20,
                    tex: 0,
                },
            );
        }
        let plates = (0..ROWS as i32)
            .map(|i| (i + 1, self.top + i))
            .chain((4..6).map(|n| (n, self.extra)));
        for (n, index) in plates {
            let item = self.items[index.clamp(0, self.items.len() as i32 - 1) as usize];
            let name = format!("mc_menu_plate_{n}");
            let clip = self.flash.find(Some(&name), "mc_icon_new");
            let new = item != self.item_max && bit(2, item);
            self.flash.clip_set_flags(clip, PROP_VISIBLE, new);
            let clip = self.flash.find(Some(&name), "mc_image_s");
            self.flash.clip_set_tex(clip, item);
        }
        let mut texts: Vec<TextDraw> = Vec::new();
        let d = self.descr(item);
        let boxed = TextBox::preset(0)
            .with_align(1)
            .with_clip(0, 0x200, 0, 0x1A1);
        for i in 0..3 {
            let clip = self.flash.find(None, &format!("mc_episode_text_{}", i + 1));
            let line = if i >= d.count { -1 } else { d.line + i };
            texts.extend(boxed.attach_line_at(
                &mut self.flash,
                clip,
                i as u32,
                &self.text,
                line,
                (0, -(self.scroll as i32)),
                font,
            ));
        }
        let clip = self.flash.find(None, "mc_episode_next");
        self.flash
            .clip_set_flags(clip, PROP_VISIBLE, self.flags & FLAG_NEXT != 0);
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&flash_quads, textures));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.msg.draw(self.voice_line, font, quads);
    }

    /// `ModeMenu_Input`: the list (focus 0) or the chosen episode's description (focus 1).
    fn input(&mut self, pressed: u32, repeat: u32, sounds: &mut Vec<TitleSound>) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & FLAG_STARTED == 0 {
            self.plate_goto("fl_on_start");
            self.flags |= FLAG_STARTED;
        }
        let se = |sounds: &mut Vec<TitleSound>, bank, id| sounds.push(TitleSound::Effect(bank, id));
        let count = self.items.len() as i32;
        match self.focus {
            0 => {
                if repeat & PAD_LEFT != 0 {
                    if self.cursor > 0 && self.items[self.cursor as usize - 1] != self.item_max {
                        self.plate_goto("fl_off_start");
                        self.cursor -= 1;
                        if self.cursor < self.top {
                            self.flash.goto_label("fl_battle_right", true);
                            self.extra = self.bottom;
                            self.top -= 1;
                            self.bottom -= 1;
                        }
                        self.moved(sounds);
                    }
                } else if repeat & PAD_RIGHT != 0 {
                    if self.cursor < count - 1
                        && self.items[self.cursor as usize + 1] != self.item_max
                    {
                        self.plate_goto("fl_off_start");
                        self.cursor += 1;
                        if self.cursor > self.bottom {
                            self.flash.goto_label("fl_battle_left", true);
                            self.extra = self.top;
                            self.top += 1;
                            self.bottom += 1;
                        }
                        self.moved(sounds);
                    }
                } else if pressed & PAD_CROSS != 0 {
                    self.plate_goto("fl_ok");
                    self.step = STEP_GUIDE;
                    self.scroll = 0.0;
                    self.scroll_max = self.descr(self.cur_item()).count as f32 * 40.0 + 160.0;
                    self.flags &= !FLAG_NEXT;
                    self.focus = 1;
                    se(sounds, 1, 1);
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.result = false;
                    se(sounds, 1, 2);
                }
            }
            _ => {
                if pressed & (PAD_CROSS | PAD_START) != 0 {
                    self.flags |= FLAG_CHOSEN | FLAG_LEAVING;
                    self.timer = 15;
                    se(sounds, 1, 1);
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.cancel_description(sounds);
                    self.focus = 0;
                    se(sounds, 1, 2);
                }
            }
        }
    }

    /// `MODEMENU_MOVED()`: the plate lit, the guide presents the episode, its picture is loaded.
    fn moved(&mut self, sounds: &mut Vec<TitleSound>) {
        self.plate_goto("fl_on_start");
        self.say(self.cur_item() + self.line_base + 4, sounds);
        self.change_image();
        sounds.push(TitleSound::Effect(1, 0));
    }

    /// Back from the description to the list; the guide reads the plate (`cursor + lineBase + 4`, as in the
    /// original: the list index, not the episode).
    fn cancel_description(&mut self, sounds: &mut Vec<TitleSound>) {
        self.plate_goto("fl_on_start");
        self.flash.goto_label("fl_battle_cansel", true);
        self.msg.open();
        self.say(self.cursor + self.line_base + 4, sounds);
    }

    /// `ModeMenu_UpdateStep`: the guide's comment, the description scrolled in and read out.
    fn update_step(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        match self.step {
            STEP_GUIDE => {
                self.say(self.line_base + 1, sounds);
                self.step += 1;
            }
            STEP_GUIDE_WAIT => {
                if voice_stat == VOICE_PLAYEND {
                    self.step += 1;
                } else if pressed & PAD_CROSS != 0 {
                    self.step += 1;
                    sounds.push(TitleSound::VoiceStop);
                    self.lip.clear();
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            STEP_OPEN => {
                self.flash.goto_label("fl_battle_ok", true);
                self.msg.close();
                self.step += 1;
            }
            STEP_OPENED => self.step = STEP_NARR_INIT,
            STEP_NARR_INIT => {
                self.narr_wait = 0;
                self.narr_line = 0;
                self.step += 1;
            }
            STEP_NARR_START => {
                if self.flash.trig() & 1 != 0 {
                    self.flags |= FLAG_SCROLLING;
                    self.step += 1;
                }
            }
            STEP_NARR_LINE => {
                let d = self.descr(self.cur_item());
                let delay = d.delay.get(self.narr_line as usize).copied().unwrap_or(0);
                let waited = self.narr_wait;
                self.narr_wait += 1;
                if delay <= waited {
                    // Voice_PlayWithSubtitle(NULL, ...): the voice alone.
                    anim::play_with_subtitle(
                        &[],
                        NARR_VOICE,
                        d.line + self.narr_line,
                        &mut self.lip,
                        sounds,
                    );
                    self.narr_wait = 0;
                    self.step += 1;
                }
            }
            STEP_NARR_WAIT => {
                if voice_stat == VOICE_PLAYEND {
                    self.narr_line += 1;
                    if self.descr(self.cur_item()).count <= self.narr_line {
                        self.step += 1;
                    } else {
                        self.step -= 1;
                    }
                } else if pressed & PAD_START != 0 {
                    self.step = STEP_GO;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.cancel_description(sounds);
                    self.narr_wait = 0;
                    self.narr_line = 0;
                    self.step += 1;
                    self.focus = 0;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            STEP_NARR_END => self.step = 0,
            STEP_GO => {
                self.step = 0;
                self.flags |= FLAG_CHOSEN | FLAG_LEAVING;
                self.timer = 15;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episodes_are_numbered_through_the_sagas() {
        assert_eq!(episode_line(0, 0), 0);
        assert_eq!(episode_line(1, 2), 5);
        assert_eq!(episode_line(4, 0), 19);
        assert_eq!(episode_line(4, 1), 21);
        assert_eq!(episode_line(4, 4), 20);
        assert_eq!(episode_line(7, 3), 47);
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

    /// Runs the real episode list of saga 0 from the ISO in `rom/`, when it is there: the guide greets, episode 0
    /// is chosen, its description scrolls in and START skips the narration to the battle.
    #[test]
    fn real_episode_list_sets_up_the_first_battle() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let mut read = |id: u32| {
            let location = partition_entry_for_id(id);
            let archive =
                AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition))
                    .unwrap();
            archive.read_entry(&mut image, location.index).unwrap()
        };
        let pack = read(crate::hist_select::ARCHIVE_FILE_ID);
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = ModeMenuAssets::parse(&res).unwrap();
        let saga_res = bpe::decode(&read(SAGA_FILE)).unwrap();
        let saga_assets = SagaAssets::parse(0, &saga_res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in assets.files.iter().chain(&saga_assets.files) {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.msg = ModeMenuAssets::msg_layout(|key| firsts[&key]);
        let mut saga = saga_assets.data;
        saga.layout = SagaLayout::new(0, 5, |key| firsts[&key]);
        println!("descriptions {:?}", &saga.descr[..4]);
        assert_eq!(saga.descr[0].line, 0);
        let mut save = SaveData::default();
        assert_eq!(save.slot_val(0, 2) & 1, 1);
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[5]);
        let mut sounds = Vec::new();
        let mut screen =
            ModeMenuScreen::new(&data, &saga, 0, 6, &save, &mut random, &mut sounds).unwrap();
        assert_eq!(screen.items, [0, 3, 3]);
        assert_eq!(
            sounds,
            [TitleSound::Bgm(BGM), TitleSound::StreamSe(0x10BDC)]
        );
        macro_rules! step {
            ($bits:expr) => {
                screen.run_frame(
                    $bits,
                    $bits,
                    VOICE_PLAYEND,
                    &mut Pictures,
                    &mut save,
                    &mut random,
                    &mut font,
                )
            };
        }
        for _ in 0..120 {
            step!(0);
        }
        assert_eq!(screen.textures[9], Some(0x3CE));
        assert!(step!(0).quads.len() > 30);
        step!(PAD_CROSS);
        assert_eq!(screen.focus, 1);
        // The guide's comment, the description scrolled in, then START during the narration.
        for _ in 0..200 {
            step!(0);
            if screen.step == STEP_NARR_WAIT {
                break;
            }
        }
        assert_eq!(screen.step, STEP_NARR_WAIT);
        // START while the line is still being read.
        screen.run_frame(
            PAD_START,
            PAD_START,
            anim::VOICE_PLAYING,
            &mut Pictures,
            &mut save,
            &mut random,
            &mut font,
        );
        for _ in 0..120 {
            step!(0);
        }
        assert_eq!(screen.result(), Some(ModeMenuResult::Battle(0)));
        assert_eq!(screen.item(), 0);
        assert_eq!(save.slot_val(0, 2) & 1, 0);
    }
}
