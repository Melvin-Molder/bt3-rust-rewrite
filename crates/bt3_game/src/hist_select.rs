//! The story mode's saga select (`HistSel_Run(1)`, src/menu/history_select.c, progress mode 6): Goku as guide, a
//! two-item menu (episodes / level), a ring of the unlocked sagas of which three plates show, three level plates
//! and the completion percentage; and the guide script (`HistGuide_Update`, src/menu/history_guide.c) that takes
//! the pad away while Goku explains the mode, a saga's guide introduces it, or a guest visits.
//!
//! A saga chosen leaves for its episode list (mode 7), the ending item for the ending movie (mode 3), cancel for
//! the main menu.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::windows::{IconWin, MsgWin};

/// `gMenuArc2`: file `gProgress->baseFile + 0xC`; the saga select is its section 1.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 0xC;
pub const ARCHIVE_SECTION: usize = 1;
/// `Hist_Main` plays it before the saga select.
pub const BGM_FILE_ID: u32 = 0x10B18;
const AMBIENT_SE: u32 = 0x10BDB;
const FADE_FRAMES: i32 = 0x14;
/// The voice sets of the eight sagas' guides and of the screen's own (Goku).
const VOICE_BASE: [u32; 9] = [
    0x8537, 0x8423, 0x83B4, 0x83DB, 0x8484, 0x8440, 0x8402, 0x8466, 0x855B,
];
const GOKU: usize = 8;
/// Episodes per saga.
const EPISODES: [u32; 8] = [3, 4, 7, 5, 16, 5, 4, 4];

const ITEM_ENDING: i32 = 8;
const ITEM_NONE: i32 = 9;

const FLAG_CHOSEN: u32 = 1;
const FLAG_LEAVING: u32 = 2;
const FLAG_STARTED: u32 = 4;
const FLAG_GREETED: u32 = 8;
const FLAG_FIRST_VISIT: u32 = 0x10;
const FLAG_EVENT: u32 = 0x20;
const FLAG_COMPLETE: u32 = 0x40;

/// `SaveSlot.flags` of a saga (`MSLOT_*`).
const SLOT_NEW: i32 = 2;
const SLOT_OUTRO_SEEN: i32 = 4;
const SLOT_INTRODUCED: i32 = 8;
const SLOT_EVENT_DONE: i32 = 0x20;
const SLOT_NEW_EPISODE: i32 = 0x40;

/// `unlockFlags` of the guide's speeches (`MUNLOCK_HIST_*`).
const UNLOCK_INTRO: i32 = 0x80;
const UNLOCK_COMPLETE: i32 = 0x100;
const UNLOCK_NEW_SAGA: i32 = 0x200;

/// `gHistSel->tex[n]` of `HistSel_Init`.
fn flash_textures() -> [ImageTexture; 83] {
    let mut t = [None; 83];
    for (slot, e) in [(30, 0), (29, 1), (22, 2), (0, 3)] {
        t[slot] = tex(2, e);
    }
    for e in 0..4 {
        t[1 + e] = tex(3, e);
    }
    for e in 0..6 {
        t[23 + e] = tex(4, e);
    }
    for (slot, e) in [(74, 0), (75, 1), (5, 2), (71, 3), (70, 4), (73, 5), (72, 6)] {
        t[slot] = tex(5, e);
    }
    // `MTEX(res, i) + 7 * 0x40`: entries 7..22 (texture records are 0x40 bytes).
    for i in 0..16 {
        t[6 + i] = tex(5, i + 7);
    }
    for (slot, e) in [(31, 0), (32, 1), (33, 3)] {
        t[slot] = tex(6, e);
    }
    t[36] = tex(7, 0);
    for (slot, e) in [
        (44, 0),
        (40, 1),
        (43, 2),
        (42, 3),
        (37, 4),
        (76, 5),
        (79, 6),
        (78, 7),
        (81, 8),
    ] {
        t[slot] = tex(8, e);
    }
    for (slot, e) in [
        (39, 0),
        (82, 1),
        (34, 2),
        (35, 3),
        (41, 4),
        (45, 5),
        (38, 6),
        (77, 8),
    ] {
        t[slot] = tex(9, e);
    }
    // The sagas' guides' icons, sections 33..40 (saga 3's middle two the other way round).
    let icons: [[usize; 3]; 8] = [
        [46, 47, 48],
        [49, 50, 51],
        [52, 53, 54],
        [55, 57, 56],
        [58, 59, 60],
        [61, 62, 63],
        [64, 65, 66],
        [67, 68, 69],
    ];
    for (i, slots) in icons.iter().enumerate() {
        for (&slot, e) in slots.iter().zip([0, 1, 3]) {
            t[slot] = tex(33 + i, e);
        }
    }
    t[80] = tex(12, 1);
    t
}

/// `MsgWin_Init(MPACK_AT(res, 14))` and `IconWin_Init(MPACK_AT(res, 13), res 11)`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((14, 1), 0)), Some(((14, 1), 1)), None];
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((13, 1), 1)),
    Some(((13, 1), 0)),
    tex(11, 0),
    Some(((13, 1), 2)),
    tex(11, 1),
];

/// What `HistSel_Init` reads from its unpacked section.
pub struct HistSelAssets {
    pub data: HistSelData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct HistSelData {
    pub fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Per guide (8 Goku): the message window's lines (sections 16.. / 15) and the voice lines' mouth keys
    /// (sections 25.. / 24).
    pub msg_text: [Vec<u8>; 9],
    pub lips: [Vec<u8>; 9],
    pub layout: HistSelLayout,
}

#[derive(Clone, Debug, Default)]
pub struct HistSelLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
}

impl HistSelLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&flash_textures(), &first),
            msg: image_indices(&MSG_TEXTURES, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
        }
    }
}

impl HistSelAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&flash_textures()[..], &MSG_TEXTURES, &ICON_TEXTURES])?;
        let section =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        let per_guide = |own: usize, first: usize| -> Result<[Vec<u8>; 9], Box<dyn Error>> {
            let mut v = Vec::new();
            for i in 0..9 {
                v.push(section(if i == GOKU { own } else { first + i })?);
            }
            Ok(v.try_into().unwrap())
        };
        Ok(Self {
            data: HistSelData {
                fod: section(10)?,
                msg_fod: pack_section(pack_section(res, 14)?, 2)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 13)?, 2)?.to_vec(),
                msg_text: per_guide(15, 16)?,
                lips: per_guide(24, 25)?,
                layout: HistSelLayout::default(),
            },
            background: decode_picture(res, 1)?,
            files,
        })
    }
}

/// The story mode's fields of `gProgress`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistProgress {
    /// The saga last chosen (`subMenu`), the episode (`subMenuItem`) and the level the battles are fought at.
    pub sub_menu: i32,
    pub sub_menu_item: i32,
    pub level: i32,
    /// What winning the battle gives (`gProgress->reward`, set by its script's command 16).
    pub reward: HistReward,
}

/// `HistReward`: by level the points; the items, characters, stages and episodes (counted through all sagas)
/// it gives, -1 none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistReward {
    pub points: [i32; 3],
    pub item: [i32; 3],
    pub chara: [i32; 3],
    pub stage: [i32; 3],
    pub episode: [i32; 3],
}

impl Default for HistReward {
    fn default() -> Self {
        Self {
            points: [0; 3],
            item: [-1; 3],
            chara: [-1; 3],
            stage: [-1; 3],
            episode: [-1; 3],
        }
    }
}

/// How `HistSel_Run` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistSelResult {
    /// 0: back to the main menu.
    Back,
    /// 1: the chosen saga's episode list (mode 7).
    Saga,
    /// 2: the ending movie (mode 3).
    Ending,
}

fn row_uv(n: i32, w: i32) -> Uv {
    Uv {
        u0: 0,
        v0: n * 0x20,
        u1: w,
        v1: n * 0x20 + 0x20,
        tex: 0,
    }
}

pub struct HistSelScreen {
    layout: HistSelLayout,
    textures: Vec<Option<usize>>,
    msg_text: [Vec<u8>; 9],
    lips: [Vec<u8>; 9],
    flash: Flash,
    msg: MsgWin,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    /// By focus: [0] the menu item, [1] the list index of the saga, [2] the level.
    cursor: [i32; 3],
    timer: i32,
    voice_line: i32,
    guide: usize,
    idle: i32,
    focus: usize,
    /// The guide script's step (`HistGuide_Update`), 0 none: the pad is read.
    script: i32,
    items: Vec<i32>,
    top: i32,
    extra: i32,
    prev: i32,
    percent: i32,
    /// The saga whose guide stands next to Goku, -1 none.
    guest: i32,
    talker: usize,
    /// The guide whose lines the message window holds.
    msg_guide: usize,
    blink: [i32; 2],
    talk: [i32; 2],
    scroll: [f32; 3],
    se_timer: i32,
    progress: HistProgress,
    result: HistSelResult,
    done: Option<HistSelResult>,
}

impl HistSelScreen {
    /// `HistSel_Init(1)` (with its ambient sound) and the first `ColorFade_StartIn`.
    pub fn new(
        data: &HistSelData,
        progress: HistProgress,
        save: &SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) -> Result<Self, Box<dyn Error>> {
        sounds.push(TitleSound::StreamSe(AMBIENT_SE));
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text[GOKU].clone(),
            true,
        )?;
        msg.open();
        // The listed sagas and the cleared episodes (of 48) as a percentage.
        let mut items = Vec::new();
        let mut cleared = 0;
        for i in 0..9 {
            if save.slot_flags(i) & 1 != 0 {
                items.push(i as i32);
            }
            if i < 8 {
                cleared += (save.slot_val(i, 1) as u32 & ((1 << EPISODES[i]) - 1)).count_ones();
            }
        }
        while items.len() < 3 {
            items.push(ITEM_NONE);
        }
        let mut percent = (cleared as f32 / 48.0 * 100.0) as i32;
        if cleared != 0 && percent == 0 {
            percent = 1;
        }
        let mut flags = 0;
        if percent == 100 && save.unlock_flags() & UNLOCK_COMPLETE == 0 {
            flags |= FLAG_COMPLETE;
        }
        let count = items.len() as i32;
        let cursor1 = items
            .iter()
            .position(|&s| s == progress.sub_menu)
            .map_or(0, |i| i as i32);
        let mut top = cursor1 - 1;
        if top < 0 {
            top += count;
        }
        let blink = [0; 2].map(|_| random.range(0x20) as i32);
        let mut screen = Self {
            layout: data.layout.clone(),
            textures: data.layout.flash.clone(),
            msg_text: data.msg_text.clone(),
            lips: data.lips.clone(),
            flash,
            msg,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags,
            cursor: [0, cursor1, 0],
            timer: 0,
            voice_line: -1,
            guide: GOKU,
            idle: 0,
            focus: 0,
            script: 0,
            items,
            top,
            extra: 0,
            prev: 0,
            percent,
            guest: -1,
            talker: 0,
            msg_guide: GOKU,
            blink,
            talk: [0; 2],
            scroll: [0.0; 3],
            se_timer: 0,
            progress,
            result: HistSelResult::Saga,
            done: None,
        };
        if save.unlock_flags() & UNLOCK_INTRO != 0 {
            screen.roll_event(save, random);
            if screen.guest >= 0 {
                screen.flags |= FLAG_EVENT;
            }
        } else {
            screen.flags |= FLAG_FIRST_VISIT;
        }
        screen.se_timer = random.range(10) as i32 * 60 + 300;
        Ok(screen)
    }

    pub fn result(&self) -> Option<HistSelResult> {
        self.done
    }

    pub fn progress(&self) -> HistProgress {
        self.progress
    }

    /// `HistSel_RollEvent`: each saga whose outro was seen and whose event was not, 4 % in order.
    fn roll_event(&mut self, save: &SaveData, random: &mut GameRandom) {
        for i in 0..8 {
            let flags = save.slot_flags(i);
            if flags & SLOT_OUTRO_SEEN != 0
                && flags & SLOT_EVENT_DONE == 0
                && (random.range(100) as i32) < 4
            {
                self.guest = i as i32;
                return;
            }
        }
    }

    fn item(&self, index: i32) -> i32 {
        let n = self.items.len() as i32;
        self.items[index.rem_euclid(n) as usize]
    }

    /// `HistSel_PlayVoice`: the line of the current guide's voice set, with its mouth keys.
    fn play_voice(&mut self, sounds: &mut Vec<TitleSound>) {
        let g = self.guide.min(GOKU);
        anim::play_with_subtitle(
            &self.lips[g],
            VOICE_BASE[g],
            self.voice_line,
            &mut self.lip,
            sounds,
        );
    }

    /// `Voice_StopWithLip`.
    fn stop_voice(&mut self, sounds: &mut Vec<TitleSound>) {
        sounds.push(TitleSound::VoiceStop);
        self.lip.clear();
    }

    /// `HistSel_Idle`: after 3600 frames without input Goku says one of four lines.
    fn idle(&mut self, random: &mut GameRandom, sounds: &mut Vec<TitleSound>) {
        self.idle += 1;
        if self.idle == 3600 {
            self.guide = GOKU;
            self.talker = 0;
            self.voice_line = random.range(4) as i32 + 10;
            self.play_voice(sounds);
            self.idle = 0;
        }
    }

    /// `HistSel_SayItem`: the saga under the cursor (one of two lines); an empty plate stops the voice.
    fn say_item(&mut self, random: &mut GameRandom, sounds: &mut Vec<TitleSound>) {
        let first = match self.item(self.cursor[1]) {
            0 => 0x16,
            1 => 0x18,
            2 => 0x1A,
            3 => 0x1C,
            4 => 0x20,
            5 => 0x1E,
            6 => 0x22,
            7 => 0x24,
            ITEM_ENDING => {
                self.voice_line = 0x2F;
                self.play_voice(sounds);
                return;
            }
            _ => {
                self.voice_line = -1;
                self.stop_voice(sounds);
                return;
            }
        };
        self.voice_line = random.range(2) as i32 + first;
        self.play_voice(sounds);
    }

    /// `HistSel_PlateGoto(0, kind, label)`: the menu plate (focus 0) or the level plate (focus 2).
    fn plate_goto(&mut self, kind: usize, label: &str) {
        let name = match kind {
            0 => format!("mc_menu_plate_{}", self.cursor[0] + 1),
            _ => format!("mc_level_plate_{}", self.cursor[2] + 1),
        };
        let clip = self.flash.find(None, &name);
        self.flash.clip_goto_label(clip, label);
    }

    /// `HistGuide_ClipGoto`: a saga's guide walks in or out.
    fn guide_goto(&mut self, guide: i32, label: &str) {
        let clip = self.flash.find(None, &format!("mc_guide_{guide:02}"));
        self.flash.clip_goto_label(clip, label);
    }

    /// One frame of the `HistSel_Run` loop. `voice_stat` is `Voice_GetStat(0)`.
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
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
        let repeat = self.pad.update(held, pressed);
        let sounds = &mut frame.sounds;
        self.fade.update();
        self.update(voice_stat, random, sounds);
        self.guide_update(pressed, voice_stat, save, random, sounds);
        self.draw(voice_stat, save, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & FLAG_GREETED == 0
            && self.flash.flags() & FLASH_PAD != 0
        {
            self.flags |= FLAG_GREETED;
            if self.flags & FLAG_COMPLETE != 0 {
                self.script = 1151;
            } else if self.flags & FLAG_FIRST_VISIT != 0 {
                self.script = 1;
            } else if self.flags & FLAG_EVENT != 0 {
                self.script = 651;
            } else {
                self.flash.goto_label("fl_menu_in", true);
                self.guide = GOKU;
                self.talker = 0;
                self.voice_line = random.range(5) as i32 + 14;
                self.play_voice(sounds);
                sounds.push(TitleSound::Effect(2, 0xE));
            }
        }
        if self.fade.is_fading_out() {
            sounds.push(TitleSound::VoiceFadeOutStep);
            sounds.push(TitleSound::BgmFadeOutStep);
            sounds.push(TitleSound::StreamSeFadeOutStep(0));
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = Some(self.result);
            return frame;
        }
        if self.flags & FLAG_LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                // A saga fades out to white.
                self.fade = ColorFade::start_out(self.result != HistSelResult::Saga, FADE_FRAMES);
                self.progress.sub_menu = self.item(self.cursor[1]);
                self.progress.level = save.level();
            }
        } else if self.script == 0 {
            self.input(pressed, repeat, save, random, sounds);
        }
        frame
    }

    /// `HistSel_Update`: after the greeting Goku reads the menu item; the movie's sound and the ambient sound.
    fn update(&mut self, voice_stat: i32, random: &mut GameRandom, sounds: &mut Vec<TitleSound>) {
        if self.flags & FLAG_GREETED != 0
            && (14..=19).contains(&self.voice_line)
            && voice_stat == VOICE_PLAYEND
        {
            self.talker = 0;
            self.voice_line = self.cursor[self.focus] + 5;
            self.play_voice(sounds);
        }
        self.flash.advance();
        if self.flash.se() & 1 != 0 {
            sounds.push(TitleSound::Effect(2, 0xF));
        }
        self.se_timer -= 1;
        if self.se_timer == -1 {
            sounds.push(TitleSound::Effect(2, 0xD));
            self.se_timer = random.range(10) as i32 * 60 + 300;
        }
    }

    /// `HistSel_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        save: &SaveData,
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
        let eye = f.find(Some("mc_guide_gokuu"), "mc_guide_gokuu_eye");
        anim::blink(f, eye, &mut self.blink[0], 0, random);
        let mouth = f.find(Some("mc_guide_gokuu"), "mc_guide_gokuu_mouth");
        if self.talker == 0 {
            anim::talk(
                f,
                mouth,
                &mut self.talk[0],
                0,
                &mut self.lip,
                voice_stat,
                random,
            );
        } else {
            anim::show_next(f, mouth, 0);
        }
        if self.guest >= 0 {
            let name = format!("mc_guide_{:02}", self.guest);
            let eye = f.find(Some(&name), &format!("mc_guide_{:02}_eye", self.guest));
            anim::blink(f, eye, &mut self.blink[1], 0, random);
            let mouth = f.find(Some(&name), &format!("mc_guide_{:02}_mouth", self.guest));
            if self.talker == 1 {
                anim::talk(
                    f,
                    mouth,
                    &mut self.talk[1],
                    0,
                    &mut self.lip,
                    voice_stat,
                    random,
                );
            } else {
                anim::show_next(f, mouth, 0);
            }
        }
        anim::num_draw(
            f,
            |n| format!("mc_tassei_num_{n}"),
            3,
            self.percent,
            (0x20, 0x20),
            0,
        );
        let arrows = [
            ("mc_yajirusi_left", "mc_yajirusi_icon_left", self.top),
            (
                "mc_yajirusi_right",
                "mc_yajirusi_icon_right",
                self.cursor[1] + 1,
            ),
        ];
        for (i, (parent, icon, index)) in arrows.into_iter().enumerate() {
            let clip = self.flash.find(Some(parent), icon);
            let show = self.item(index) != ITEM_NONE;
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
        for (count, plate, text) in [
            (2, "mc_menu_plate", "mc_menu_text"),
            (3, "mc_level_plate", "mc_level_text"),
        ] {
            for i in 0..count {
                let name = format!("{plate}_{}", i + 1);
                for state in ["off", "on"] {
                    let clip = self.flash.find(Some(&name), &format!("{text}_{state}"));
                    self.flash.clip_set_uv(clip, row_uv(i, 0x200));
                }
            }
        }
        for i in 0..3 {
            let clip = self.flash.find(None, &format!("mc_scenario_{}", i + 1));
            self.flash.clip_set_tex(clip, self.item(self.top + i));
        }
        let clip = self.flash.find(None, "mc_scenario_4");
        self.flash.clip_set_tex(clip, self.item(self.extra));
        for (i, index) in [self.cursor[1], self.prev].into_iter().enumerate() {
            let clip = self
                .flash
                .find(None, &format!("mc_scenario_text_{}", i + 1));
            let item = self.item(index);
            self.flash.clip_set_tex(clip, item / 8);
            self.flash.clip_set_uv(clip, row_uv(item % 8, 0x200));
        }
        let clip = self.flash.find(None, "mc_icon_new");
        let new = save.slot_flags(self.item(self.cursor[1]) as usize) & SLOT_NEW != 0;
        self.flash.clip_set_flags(clip, PROP_VISIBLE, new);
        for (i, speed) in [0.355_555_56, 0.355_555_56, 0.533_333_3]
            .into_iter()
            .enumerate()
        {
            let clip = self.flash.find(None, &format!("mc_yuragi_{}", i + 1));
            let uv = Uv {
                u0: 0,
                v0: 0,
                u1: 0x40,
                v1: 0x40,
                tex: 0,
            };
            anim::scroll_y(&mut self.flash, clip, uv, &mut self.scroll[i], speed);
        }
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        quads.extend(screen_quads(&self.flash.draw(&has_texture), textures));
        self.icon.draw(quads);
        let guide = self.guide.min(GOKU);
        if guide != self.msg_guide {
            self.msg_guide = guide;
            self.msg.set_text(self.msg_text[guide].clone());
        }
        self.msg.draw(self.voice_line, font, quads);
    }

    /// `HistSel_Input`: by focus, the menu, the ring of sagas, the level plates.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        save: &mut SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & FLAG_STARTED == 0 {
            self.plate_goto(0, "fl_on_start");
            self.flags |= FLAG_STARTED;
        }
        let se = |sounds: &mut Vec<TitleSound>, bank, id| sounds.push(TitleSound::Effect(bank, id));
        let count = self.items.len() as i32;
        match self.focus {
            0 => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    self.idle = 0;
                    self.plate_goto(0, "fl_off_start");
                    self.cursor[0] = if repeat & PAD_UP != 0 {
                        if self.cursor[0] - 1 < 0 {
                            1
                        } else {
                            self.cursor[0] - 1
                        }
                    } else if self.cursor[0] + 1 >= 2 {
                        0
                    } else {
                        self.cursor[0] + 1
                    };
                    self.plate_goto(0, "fl_on_start");
                    self.voice_line = self.cursor[0] + 5;
                    self.play_voice(sounds);
                    se(sounds, 1, 0);
                } else if pressed & PAD_CROSS != 0 {
                    self.idle = 0;
                    self.plate_goto(0, "fl_ok");
                    if self.cursor[0] == 0 {
                        self.flash.goto_label("fl_scenario_in", true);
                        let unlock = save.unlock_flags();
                        if unlock & UNLOCK_NEW_SAGA != 0 {
                            save.set_unlock_flags(unlock & !UNLOCK_NEW_SAGA);
                            self.voice_line = 0x2A;
                        } else {
                            self.voice_line = random.range(3) as i32 + 0x13;
                        }
                        self.play_voice(sounds);
                        self.focus = 1;
                    } else {
                        self.flash.goto_label("fl_level_in", true);
                        self.cursor[2] = save.level();
                        self.plate_goto(2, "fl_on_start");
                        self.voice_line = self.cursor[2] + 7;
                        self.play_voice(sounds);
                        self.focus = 2;
                    }
                    se(sounds, 1, 1);
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.idle = 0;
                    self.script = 0x33;
                    self.result = HistSelResult::Back;
                    se(sounds, 1, 2);
                } else {
                    self.idle(random, sounds);
                }
            }
            1 => {
                if repeat & PAD_LEFT != 0 {
                    self.idle = 0;
                    if self.item(self.top) == ITEM_NONE {
                        return;
                    }
                    self.flash.goto_label("fl_scenario_right", true);
                    self.extra = (self.cursor[1] + 1) % count;
                    self.prev = self.cursor[1];
                    self.cursor[1] -= 1;
                    if self.cursor[1] < 0 {
                        self.cursor[1] = count - 1;
                    }
                    self.top = (self.cursor[1] - 1).rem_euclid(count);
                    self.say_item(random, sounds);
                    se(sounds, 2, 0xA);
                } else if repeat & PAD_RIGHT != 0 {
                    self.idle = 0;
                    if self.item(self.cursor[1] + 1) == ITEM_NONE {
                        return;
                    }
                    self.flash.goto_label("fl_scenario_left", true);
                    self.extra = self.top;
                    self.prev = self.cursor[1];
                    self.cursor[1] += 1;
                    if self.cursor[1] >= count {
                        self.cursor[1] = 0;
                    }
                    self.top = (self.cursor[1] - 1).rem_euclid(count);
                    self.say_item(random, sounds);
                    se(sounds, 2, 0xA);
                } else if pressed & PAD_CROSS != 0 {
                    self.idle = 0;
                    match self.item(self.cursor[1]) {
                        ITEM_NONE => se(sounds, 1, 7),
                        ITEM_ENDING => {
                            save.set_slot_flags(8, save.slot_flags(8) & !SLOT_NEW);
                            self.result = HistSelResult::Ending;
                            self.flags |= FLAG_CHOSEN | FLAG_LEAVING;
                            self.timer = 15;
                            se(sounds, 2, 0xB);
                        }
                        saga => {
                            self.script = if save.slot_flags(saga as usize) & SLOT_INTRODUCED != 0 {
                                0x65
                            } else {
                                0x97
                            };
                            se(sounds, 2, 0xB);
                        }
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.idle = 0;
                    self.flash.goto_label("fl_scenario_cansel", true);
                    self.plate_goto(0, "fl_on_start");
                    self.voice_line = self.cursor[0] + 5;
                    self.play_voice(sounds);
                    self.focus = 0;
                    se(sounds, 1, 2);
                } else {
                    self.idle(random, sounds);
                }
            }
            _ => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    self.idle = 0;
                    self.plate_goto(2, "fl_off_start");
                    self.cursor[2] = if repeat & PAD_UP != 0 {
                        if self.cursor[2] - 1 < 0 {
                            2
                        } else {
                            self.cursor[2] - 1
                        }
                    } else if self.cursor[2] + 1 >= 3 {
                        0
                    } else {
                        self.cursor[2] + 1
                    };
                    self.plate_goto(2, "fl_on_start");
                    self.voice_line = self.cursor[2] + 7;
                    self.play_voice(sounds);
                    se(sounds, 1, 0);
                } else if pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                    self.idle = 0;
                    let ok = pressed & PAD_CROSS != 0;
                    if ok {
                        save.set_level(self.cursor[2]);
                    }
                    self.flash.goto_label("fl_level_out", true);
                    self.plate_goto(2, if ok { "fl_ok" } else { "fl_off_start" });
                    self.plate_goto(0, "fl_on_start");
                    self.voice_line = self.cursor[0] + 5;
                    self.play_voice(sounds);
                    self.focus = 0;
                    se(sounds, 1, if ok { 1 } else { 2 });
                } else {
                    self.idle(random, sounds);
                }
            }
        }
    }

    /// `HistGuide_Update`: one step of the guide script.
    fn guide_update(
        &mut self,
        pressed: u32,
        voice_stat: i32,
        save: &mut SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        let step = self.script;
        if step == 0 {
            return;
        }
        let saga = self.item(self.cursor[1]);
        // The guide's lines alternate: the next line, by Goku (0) or the guest (1).
        let say = |s: &mut Self, talker: Option<usize>, sounds: &mut Vec<TitleSound>| {
            if let Some(t) = talker {
                s.talker = t;
            }
            s.voice_line += 1;
            s.play_voice(sounds);
            s.script += 1;
        };
        // Waits for the voice to end; confirm skips it.
        let wait = |s: &mut Self, sounds: &mut Vec<TitleSound>| {
            if voice_stat == VOICE_PLAYEND {
                s.script += 1;
            } else if pressed & PAD_CROSS != 0 {
                s.script += 1;
                sounds.push(TitleSound::Effect(1, 1));
            }
        };
        let trig = |s: &mut Self, bit: u32| {
            if s.flash.trig() & bit != 0 {
                s.script += 1;
            }
        };
        let menu_in = |s: &mut Self, sounds: &mut Vec<TitleSound>| {
            s.script = 0;
            s.flash.goto_label("fl_menu_in", true);
            s.guide = GOKU;
            s.voice_line = s.cursor[0] + 5;
            s.play_voice(sounds);
            sounds.push(TitleSound::Effect(2, 0xE));
        };
        let leave = |s: &mut Self| {
            s.result = HistSelResult::Saga;
            s.flags |= FLAG_CHOSEN | FLAG_LEAVING;
            s.timer = 15;
            s.script = 0;
        };
        let close = |s: &mut Self, sounds: &mut Vec<TitleSound>| {
            s.flash.step_frames(1);
            sounds.push(TitleSound::Effect(2, 0xC));
            s.icon.close();
            s.msg.close();
            s.script += 1;
        };
        let start = |s: &mut Self, talker: usize, line: i32, sounds: &mut Vec<TitleSound>| {
            s.voice_line = line;
            s.talker = talker;
            s.play_voice(sounds);
            s.script += 1;
        };
        let guest = |s: &mut Self, label: &str| {
            s.guide_goto(s.guest, label);
            s.script += 1;
        };
        match step {
            // First visit: Goku explains the mode.
            1 => {
                save.set_unlock_flags(save.unlock_flags() | UNLOCK_INTRO);
                self.guide = GOKU;
                start(self, 0, 0, sounds);
            }
            3 | 5 | 7 | 9 => say(self, None, sounds),
            2 | 4 | 6 | 8 | 10 => wait(self, sounds),
            11 => menu_in(self, sounds),
            // Every episode won: his speech at 100 %.
            1151 => {
                save.set_unlock_flags(save.unlock_flags() | UNLOCK_COMPLETE);
                self.guide = GOKU;
                start(self, 0, 0x2C, sounds);
            }
            1152 | 1154 | 1156 | 1158 => wait(self, sounds),
            1153 | 1155 | 1157 => say(self, None, sounds),
            1159 => menu_in(self, sounds),
            // Leaving: goodbye, then the fade out (cancel skips as confirm does).
            51 => {
                self.guide = GOKU;
                start(self, 0, 0x2B, sounds);
            }
            52 => {
                if voice_stat == VOICE_PLAYEND {
                    self.script += 1;
                } else if pressed & PAD_CROSS != 0 {
                    self.script += 1;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.script += 1;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            53 => {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.script = 0;
            }
            // A saga already introduced: Goku comments, the movie closes, leave.
            101 => {
                self.voice_line = random.range(4) as i32 + 0x26;
                self.play_voice(sounds);
                self.script += 1;
            }
            102 => wait(self, sounds),
            103 => {
                self.flash.goto_label("fl_scenario_ok", true);
                self.script += 1;
            }
            104 => trig(self, 1),
            105 | 201 => close(self, sounds),
            106 | 202 => trig(self, 2),
            107 => leave(self),
            // A saga chosen for the first time: its guide walks in and introduces it.
            151 => {
                self.flash.goto_label("fl_scenario_ok", true);
                self.guest = saga;
                self.guide_goto(saga, "fl_in");
                self.guide = saga.clamp(0, GOKU as i32) as usize;
                self.script += 1;
                save.set_slot_flags(
                    saga as usize,
                    save.slot_flags(saga as usize) | SLOT_INTRODUCED,
                );
                self.voice_line = -1;
            }
            152 => {
                if self.flash.trig() & 4 != 0 {
                    self.script = saga * 50 + 251;
                }
            }
            203 => {
                leave(self);
                let flags = save.slot_flags(saga as usize);
                save.set_slot_flags(saga as usize, flags & !(SLOT_NEW | SLOT_NEW_EPISODE));
            }
            // The eight introductions: the guest's line 0, then the lines in turn.
            251 | 301 | 351 | 401 | 451 | 501 | 551 | 601 => start(self, 1, 0, sounds),
            253 | 261 | 305 | 353 | 357 | 361 | 365 | 403 | 409 | 411 | 413 | 453 | 463 | 503
            | 511 | 519 | 553 | 557 | 603 | 609 | 615 => say(self, Some(0), sounds),
            255 | 257 | 259 | 263 | 265 | 303 | 307 | 355 | 359 | 363 | 367 | 405 | 407 | 415
            | 417 | 419 | 455 | 457 | 459 | 461 | 465 | 467 | 505 | 507 | 509 | 513 | 515 | 517
            | 555 | 559 | 561 | 605 | 607 | 611 | 613 => say(self, Some(1), sounds),
            252..=266
            | 302..=308
            | 352..=368
            | 402..=420
            | 452..=468
            | 502..=520
            | 552..=562
            | 602..=616
                if step % 2 == 0 && step != 518 =>
            {
                wait(self, sounds)
            }
            518 => {
                if voice_stat == VOICE_PLAYEND || pressed & PAD_CROSS != 0 {
                    self.guide_goto(self.guest, "fl_out");
                    self.script += 1;
                    if voice_stat != VOICE_PLAYEND {
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                }
            }
            267 | 309 | 369 | 421 | 469 | 521 | 563 | 617 => self.script = 201,
            // An event visit: the guest is already chosen.
            651 => {
                self.guide = self.guest.clamp(0, GOKU as i32) as usize;
                let g = self.guest as usize;
                save.set_slot_flags(g, save.slot_flags(g) | SLOT_EVENT_DONE);
                self.script = self.guest * 50 + 751;
            }
            701 => {
                self.guest = -1;
                self.talker = 0;
                menu_in(self, sounds);
            }
            // The eight event dialogues, the guest walking in and out.
            751 => start(self, 0, 0x1A, sounds),
            801 => start(self, 0, 0x14, sounds),
            853 | 1003 => start(self, 1, 0x1D, sounds),
            901 => start(self, 0, 0x1E, sounds),
            953 => start(self, 1, 0x27, sounds),
            1053 => start(self, 1, 0x18, sounds),
            1103 => start(self, 1, 0x16, sounds),
            761 | 763 | 771 | 773 | 809 | 811 | 819 | 821 | 855 | 859 | 861 | 865 | 873 | 909
            | 913 | 917 | 919 | 921 | 955 | 959 | 967 | 971 | 975 | 1005 | 1007 | 1013 | 1021
            | 1055 | 1059 | 1065 | 1067 | 1069 | 1071 | 1105 | 1109 | 1111 | 1113 | 1119 => {
                say(self, Some(0), sounds)
            }
            755 | 757 | 759 | 765 | 767 | 805 | 807 | 813 | 815 | 857 | 863 | 867 | 869 | 905
            | 907 | 911 | 957 | 961 | 963 | 965 | 969 | 1009 | 1011 | 1015 | 1017 | 1057 | 1061
            | 1107 | 1115 => say(self, Some(1), sounds),
            753 | 803 | 851 | 903 | 951 | 1001 | 1051 | 1101 => guest(self, "fl_in"),
            769 | 817 | 871 | 915 | 973 | 1019 | 1063 | 1117 => guest(self, "fl_out"),
            754 | 770 | 804 | 818 | 852 | 872 | 904 | 916 | 952 | 974 | 1002 | 1020 | 1052
            | 1064 | 1102 | 1118 => trig(self, 4),
            752 | 756 | 758 | 760 | 762 | 764 | 766 | 768 | 772 | 774 | 802 | 806 | 808 | 810
            | 812 | 814 | 816 | 820 | 822 | 854 | 856 | 858 | 860 | 862 | 864 | 866 | 868 | 870
            | 874 | 902 | 906 | 908 | 910 | 912 | 914 | 918 | 920 | 922 | 954 | 956 | 958 | 960
            | 962 | 964 | 966 | 968 | 970 | 972 | 976 | 1004 | 1006 | 1008 | 1010 | 1012 | 1014
            | 1016 | 1018 | 1022 | 1054 | 1056 | 1058 | 1060 | 1062 | 1066 | 1068 | 1070 | 1072
            | 1104 | 1106 | 1108 | 1110 | 1112 | 1114 | 1116 | 1120 => wait(self, sounds),
            775 | 823 | 875 | 923 | 977 | 1023 | 1073 | 1121 => self.script = 701,
            _ => sounds.push(TitleSound::VoiceFadeOutStep),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the real saga select from the ISO in `rom/`, when it is there: a new save hears Goku's explanation,
    /// opens the saga list and picks the first saga, whose guide introduces it before the screen leaves.
    #[test]
    fn real_saga_select_introduces_the_first_saga() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = HistSelAssets::parse(&res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = HistSelLayout::new(5, |key| firsts[&key]);
        let mut save = SaveData::default();
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[6, 6]);
        let mut sounds = Vec::new();
        let mut screen = HistSelScreen::new(
            &data,
            HistProgress::default(),
            &save,
            &mut random,
            &mut sounds,
        )
        .unwrap();
        assert_eq!(sounds, [TitleSound::StreamSe(AMBIENT_SE)]);
        assert_eq!(screen.items, [0, ITEM_NONE, ITEM_NONE]);
        assert_ne!(screen.flags & FLAG_FIRST_VISIT, 0);
        let mut voices = Vec::new();
        macro_rules! step {
            ($bits:expr) => {{
                let f = screen.run_frame(
                    $bits,
                    $bits,
                    VOICE_PLAYEND,
                    &mut save,
                    &mut random,
                    &mut font,
                );
                voices.extend(f.sounds.iter().filter_map(|s| match s {
                    TitleSound::Voice(id) => Some(*id),
                    _ => None,
                }));
                f
            }};
        }
        // The fade in, then Goku's explanation (lines 0..5 of his set, each ended by its voice), then the menu.
        for _ in 0..80 {
            step!(0);
        }
        assert_eq!(screen.script, 0);
        assert_ne!(save.unlock_flags() & UNLOCK_INTRO, 0);
        let base = VOICE_BASE[GOKU]
            + if crate::sound::options().alt_voice {
                anim::VOICE_LANG2_OFFSET
            } else {
                0
            };
        assert_eq!(&voices[..6], (0..6).map(|n| base + n).collect::<Vec<_>>());
        assert!(step!(0).quads.len() > 30);
        // The saga list (once the menu has come in), then saga 0: its guide walks in and introduces it (lines
        // ending with their voices).
        for _ in 0..200 {
            if screen.flash.flags() & FLASH_PAD != 0 {
                break;
            }
            step!(0);
        }
        step!(PAD_CROSS);
        for _ in 0..20 {
            step!(0);
        }
        assert_eq!(screen.focus, 1);
        step!(PAD_CROSS);
        assert_eq!(screen.script, 0x97);
        for _ in 0..600 {
            step!(0);
            if screen.result().is_some() {
                break;
            }
        }
        assert_eq!(screen.result(), Some(HistSelResult::Saga));
        assert_eq!(screen.progress().sub_menu, 0);
        assert_ne!(save.slot_flags(0) & SLOT_INTRODUCED, 0);
        assert_eq!(screen.guest, 0);
        // The introduction is saga 0's guide's set.
        assert!(
            voices
                .iter()
                .any(|&v| (VOICE_BASE[0]..VOICE_BASE[0] + 0x600).contains(&v))
        );
    }
}
