//! The duel mode's menu (`DuelMenu_Run(1)`, src/menu/duel_menu.c, progress mode 38): Vegeta and Nappa as guides,
//! the top list (1P vs COM, 1P vs 2P, COM vs COM, battle settings), the battle type, the DP limit, and the five
//! battle settings with their value picker and "restore defaults" question.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::dialog::{self, Dialog, DialogAssets};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quad};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::windows::{IconWin, MsgWin};

/// gMenuArc5 is file `gProgress->baseFile`; `DuelMenu_Run(1)` unpacks its section 1.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1;
pub const ARCHIVE_SECTION: usize = 1;
pub const BGM_FILE_ID: u32 = 0x10B1B;
const VOICE_BASE: u32 = 0x85C7;
const IDLE_FRAMES: i32 = 0xE10;
const FADE_FRAMES: i32 = 0x14;

const FLAG_CHOSEN: u32 = 1;
const FLAG_LEAVING: u32 = 2;
const FLAG_STARTED: u32 = 4;
const FLAG_GREETED: u32 = 8;
const FLAG_NO_PAD2: u32 = 0x10;

const LEVEL_TOP: usize = 0;
const LEVEL_TYPE: usize = 1;
const LEVEL_DP: usize = 2;
const LEVEL_SETTING: usize = 3;
const LEVEL_VALUE: usize = 4;

/// `DM->tex[n]` of `DuelMenu_Init`.
const FLASH_TEXTURES: [ImageTexture; 49] = [
    tex(2, 0),
    tex(2, 1),
    tex(5, 0),
    tex(5, 1),
    tex(5, 2),
    tex(5, 3),
    tex(5, 4),
    tex(5, 5),
    tex(2, 2),
    tex(8, 2),
    tex(7, 1),
    tex(8, 3),
    tex(6, 0),
    tex(8, 0),
    tex(6, 2),
    tex(6, 1),
    tex(8, 1),
    tex(7, 2),
    tex(12, 1),
    tex(12, 2),
    tex(13, 0),
    tex(13, 1),
    tex(15, 0),
    tex(15, 1),
    tex(16, 0),
    tex(16, 1),
    tex(18, 0),
    tex(18, 1),
    tex(17, 2),
    tex(17, 3),
    tex(17, 4),
    tex(17, 5),
    tex(17, 0),
    tex(17, 1),
    tex(14, 0),
    tex(4, 0),
    tex(4, 1),
    tex(4, 3),
    tex(3, 0),
    tex(3, 1),
    tex(3, 3),
    tex(9, 0),
    tex(10, 0),
    tex(12, 0),
    tex(14, 1),
    tex(11, 1),
    tex(11, 0),
    tex(10, 1),
    tex(10, 2),
];
/// `DM->setRes`: section 10, whose entries 1 and 2 `DuelMenu_SetupValue` swaps into images 47 and 48.
const SET_RES: usize = 10;
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((22, 1), 0)), Some(((22, 1), 1)), None];
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((23, 1), 1)),
    Some(((23, 1), 0)),
    tex(24, 0),
    Some(((23, 1), 2)),
    tex(24, 1),
];

/// What `DuelMenu_Init` reads from its unpacked section.
pub struct DuelMenuAssets {
    pub data: DuelMenuData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    /// Section 25, `Dialog_Init`'s file.
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct DuelMenuData {
    pub fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    /// Section 21: the message window's lines.
    pub msg_text: Vec<u8>,
    /// Section 27: the settings' lines and the dialog's texts.
    pub set_text: Vec<u8>,
    /// Section 26: mouth keys of the voice lines.
    pub lips: Vec<u8>,
    pub layout: DuelMenuLayout,
}

#[derive(Clone, Debug, Default)]
pub struct DuelMenuLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    /// Entry 0 of section 10.
    pub set_res: usize,
    pub msg: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
}

impl DuelMenuLayout {
    pub fn new(
        background: usize,
        dialog: Vec<Option<usize>>,
        first: impl Fn(FileKey) -> usize,
    ) -> Self {
        Self {
            background,
            flash: image_indices(&FLASH_TEXTURES, &first),
            set_res: first((SET_RES, 0)),
            msg: image_indices(&MSG_TEXTURES, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            dialog,
        }
    }
}

impl DuelMenuAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&FLASH_TEXTURES[..], &MSG_TEXTURES, &ICON_TEXTURES])?;
        let dialog = DialogAssets::parse(pack_section(res, 25)?)?;
        Ok(Self {
            data: DuelMenuData {
                fod: pack_section(res, 19)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 22)?, 2)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 23)?, 2)?.to_vec(),
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                msg_text: pack_section(res, 21)?.to_vec(),
                set_text: pack_section(res, 27)?.to_vec(),
                lips: pack_section(res, 26)?.to_vec(),
                layout: DuelMenuLayout::default(),
            },
            background: decode_picture(res, 1)?,
            files,
            dialog,
        })
    }
}

/// The duel fields of `gProgress`, kept between visits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DuelProgress {
    /// 0 1P vs COM, 1 1P vs 2P, 2 COM vs COM.
    pub players: i32,
    /// 0 single, 1 team, 2 DP battle.
    pub battle_type: i32,
    pub dp_limit: i32,
}

/// How `DuelMenu_Run` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuelMenuResult {
    /// 0: back to the main menu (progress mode 4).
    Back,
    /// 1: on to the character select, mode 39 (single) or 40 (team and DP battles).
    Select(i32),
}

/// `DuelReset`: the "restore the default settings?" question.
#[derive(Clone, Copy, Debug, Default)]
struct Reset {
    state: i32,
    answer: i32,
    timer: i32,
    msg: i32,
}

pub struct DuelMenuScreen {
    layout: DuelMenuLayout,
    textures: Vec<Option<usize>>,
    msg_text: Vec<u8>,
    set_text: Vec<u8>,
    lips: Vec<u8>,
    flash: Flash,
    msg: MsgWin,
    icon: IconWin,
    dialog: Dialog,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    sel: [i32; 5],
    timer: i32,
    level: usize,
    voice_line: i32,
    idle: i32,
    rule: usize,
    column: usize,
    value: [i32; 2],
    value_count: i32,
    start_state: i32,
    talker: i32,
    blink: [i32; 2],
    talk: [i32; 2],
    icon_timer: i32,
    icon_frame: i32,
    smoke: [f32; 6],
    cloud: f32,
    reset: Reset,
    /// `gSaveData->rule`.
    rules: [i32; 6],
    progress: DuelProgress,
    result: i32,
    done: Option<DuelMenuResult>,
}

impl DuelMenuScreen {
    /// `DuelMenu_Init` and the first `ColorFade_StartIn`.
    pub fn new(
        data: &DuelMenuData,
        progress: DuelProgress,
        rules: [i32; 6],
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            true,
        )?;
        msg.open();
        let mut dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            Some(data.set_text.clone()),
            0,
        )?;
        dialog.set_layout(0);
        let mut blink = [0; 2];
        for b in &mut blink {
            *b = random.range(0x20) as i32;
        }
        Ok(Self {
            layout: data.layout.clone(),
            textures: data.layout.flash.clone(),
            msg_text: data.msg_text.clone(),
            set_text: data.set_text.clone(),
            lips: data.lips.clone(),
            flash,
            msg,
            icon,
            dialog,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            sel: [
                progress.players,
                progress.battle_type,
                progress.dp_limit,
                0,
                0,
            ],
            timer: 0,
            level: LEVEL_TOP,
            voice_line: -1,
            idle: 0,
            rule: 0,
            column: 0,
            value: [0; 2],
            value_count: 0,
            start_state: 0,
            talker: 0,
            blink,
            talk: [0; 2],
            icon_timer: 0,
            icon_frame: 0,
            smoke: [0.0; 6],
            cloud: 0.0,
            reset: Reset::default(),
            rules,
            progress,
            result: 1,
            done: None,
        })
    }

    pub fn result(&self) -> Option<DuelMenuResult> {
        self.done
    }

    pub fn progress(&self) -> DuelProgress {
        self.progress
    }

    pub fn rules(&self) -> [i32; 6] {
        self.rules
    }

    /// `DM_SAY(who, n)`.
    fn say(&mut self, who: i32, line: i32, sounds: &mut Vec<TitleSound>) {
        self.talker = who;
        self.voice_line = line;
        anim::play_with_subtitle(&self.lips, VOICE_BASE, line, &mut self.lip, sounds);
    }

    /// `DuelMenu_Idle`: after 3600 frames without input Nappa speaks line 8.
    fn idle(&mut self, sounds: &mut Vec<TitleSound>) {
        self.idle += 1;
        if self.idle == IDLE_FRAMES {
            self.say(1, 8, sounds);
            self.idle = 0;
        }
    }

    /// One frame of the `DuelMenu_Run` loop. `pad2` is whether a controller is in port 2.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        pad2: bool,
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
        let sounds = &mut frame.sounds;
        self.fade.update();
        self.update(pad2, voice_stat, sounds);
        self.update_start(pressed, voice_stat, sounds);
        self.draw(voice_stat, random, font, &mut frame.quads);
        self.update_reset(pressed, repeat, font, &mut frame.quads, sounds);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & FLAG_GREETED == 0
            && self.flash.flags() & FLASH_PAD != 0
        {
            self.flags |= FLAG_GREETED;
            self.say(0, 0, sounds);
        }
        if self.fade.is_fading_out() {
            sounds.push(TitleSound::FadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = Some(if self.result == 0 {
                DuelMenuResult::Back
            } else if self.progress.battle_type != 0 {
                DuelMenuResult::Select(40)
            } else {
                DuelMenuResult::Select(39)
            });
            return frame;
        }
        if self.flags & FLAG_LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.progress = DuelProgress {
                    players: self.sel[LEVEL_TOP],
                    battle_type: self.sel[LEVEL_TYPE],
                    dp_limit: self.sel[LEVEL_DP],
                };
            }
        } else if self.start_state == 0 && self.reset.state == 0 {
            self.input(pressed, repeat, sounds);
        }
        frame
    }

    /// `DuelMenu_Update`.
    fn update(&mut self, pad2: bool, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if self.flags & FLAG_NO_PAD2 != 0 {
            if pad2 {
                self.flags ^= FLAG_NO_PAD2;
            }
        } else if !pad2 {
            self.flags |= FLAG_NO_PAD2;
        }
        if self.flags & FLAG_GREETED != 0 && self.voice_line == 0 && voice_stat == VOICE_PLAYEND {
            self.say(0, self.sel[LEVEL_TOP] + 1, sounds);
        }
        self.flash.advance();
    }

    /// `DuelMenu_UpdateStart`: after the last choice a guide says a line, then the screen leaves.
    fn update_start(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        match self.start_state {
            1 => {
                match self.level {
                    LEVEL_TYPE => match self.sel[LEVEL_TYPE] {
                        0 => self.say(0, 9, sounds),
                        1 => self.say(1, 10, sounds),
                        _ => {}
                    },
                    LEVEL_DP => self.say(0, 11, sounds),
                    _ => {}
                }
                self.start_state += 1;
            }
            2 => {
                if voice_stat == VOICE_PLAYEND {
                    self.start_state += 1;
                } else if pressed & PAD_CROSS != 0 {
                    self.start_state += 1;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            3 => {
                self.flags |= FLAG_CHOSEN | FLAG_LEAVING;
                self.timer = 15;
                self.start_state = 0;
            }
            _ => {}
        }
    }

    /// `DuelMenu_UpdateReset`: on yes `Save_ResetRules`, then the dialog closes.
    fn update_reset(
        &mut self,
        pressed: u32,
        repeat: u32,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.reset.state == 0 {
            return;
        }
        let mut effects = Vec::new();
        let pad = dialog::Pad { pressed, repeat };
        if self.reset.state == 1 {
            self.reset.state = 2;
            self.dialog.set_layout(0);
            self.dialog.start(dialog::Command::Open, &mut effects);
            self.dialog.set_cursor(1);
            self.reset.answer = 0;
        }
        match self.reset.state {
            2 => {
                self.dialog.set_choices(true);
                self.dialog.start(dialog::Command::Open, &mut effects);
                self.reset.msg = 1;
                if self.reset.answer > 0 {
                    self.reset.timer -= 1;
                    if self.reset.timer < 0 {
                        self.rules = crate::save::DEFAULT_RULES;
                        self.reset.timer = 0x78;
                        self.reset.state = 3;
                    }
                } else if self.reset.answer < 0 {
                    self.dialog.set_cursor(1);
                    self.reset.timer = 0x78;
                    self.reset.state = 3;
                } else {
                    self.reset.answer = self.dialog.input(true, pad, &mut effects);
                    self.reset.timer = 12;
                }
            }
            3 => {
                self.dialog.set_choices(false);
                self.clip_goto(self.level, "fl_on_start");
                self.dialog.start(dialog::Command::Close, &mut effects);
                self.dialog.input(false, pad, &mut effects);
                self.reset.state = 4;
            }
            4 => {
                self.dialog.input(false, pad, &mut effects);
                if self.dialog.is_closed() {
                    self.reset.state = 0;
                }
            }
            _ => {}
        }
        self.dialog.set_msg(self.reset.msg);
        self.dialog.draw(true, font, quads);
        sounds.extend(
            effects
                .into_iter()
                .map(|(bank, id)| TitleSound::Effect(bank, id)),
        );
    }

    /// `DuelMenu_SetupValue`: the rule(s), value count and value text of the setting under the cursor.
    fn setup_value(&mut self) {
        let (rule, count, set_tex) = match self.sel[LEVEL_SETTING] {
            0 => (0, 5, Some(1)),
            1 => (1, 5, Some(1)),
            2 => (2, 7, Some(1)),
            3 => (3, 2, Some(2)),
            4 => (5, 2, Some(2)),
            _ => return,
        };
        self.rule = rule;
        self.value_count = count;
        self.column = 0;
        self.value = [
            self.rules[rule],
            if self.sel[LEVEL_SETTING] == 3 {
                self.rules[rule + 1]
            } else {
                0
            },
        ];
        let entry = |e: usize| self.layout.set_res + 2 * e;
        (self.textures[47], self.textures[48]) = match set_tex {
            Some(1) => (Some(entry(1)), None),
            _ => (None, Some(entry(2))),
        };
    }

    /// `DuelMenu_ClipGoto(level, label)`: the plate under the cursor of a level.
    fn clip_goto(&mut self, level: usize, label: &str) {
        let sel = self.sel[level];
        let name = match level {
            LEVEL_TOP => format!("mc_menu_plate_{}", sel + 1),
            LEVEL_TYPE => format!("mc_mode_{}", sel + 1),
            LEVEL_DP => format!("mc_dp_{}", sel + 1),
            LEVEL_SETTING => {
                let clip = self
                    .flash
                    .find(None, &format!("mc_battle_set_text_{}", sel + 1));
                self.flash.clip_goto_label(clip, label);
                let names = match sel {
                    3 => vec![
                        format!("mc_set_{}as", sel + 1),
                        format!("mc_set_{}bs", sel + 1),
                    ],
                    5 => Vec::new(),
                    _ => vec![format!("mc_set_{}", sel + 1)],
                };
                for name in names {
                    let clip = self.flash.find(None, &name);
                    self.flash.clip_goto_label(clip, label);
                }
                return;
            }
            _ => match sel {
                3 => format!(
                    "mc_set_{}{}_{}",
                    sel + 1,
                    if self.column != 0 { 'b' } else { 'a' },
                    self.value[self.column] + 1
                ),
                5 => return,
                _ => format!("mc_set_{}_{}", sel + 1, self.value[0] + 1),
            },
        };
        let clip = self.flash.find(None, &name);
        self.flash.clip_goto_label(clip, label);
    }

    fn set_uv(&mut self, parent: &str, clip: &str, uv: Uv) {
        let r = self.flash.find(Some(parent), clip);
        self.flash.clip_set_uv(r, uv);
    }

    /// `DuelMenu_ShowValues`: shows or hides the value plates of one setting and gives the shown ones their text.
    fn show_values(&mut self, item: i32, show: bool) {
        let n = item + 1;
        match item {
            0..=2 => {
                for i in 0..if item == 2 { 7 } else { 5 } {
                    let name = format!("mc_set_{n}_{}", i + 1);
                    let clip = self.flash.find(None, &name);
                    self.flash.clip_set_flags(clip, PROP_VISIBLE, show);
                    if show {
                        let uv = row_uv(i, 0x100);
                        self.set_uv(&name, &format!("mc_set_{n}_off"), uv);
                        self.set_uv(&name, &format!("mc_set_{n}_on"), uv);
                    }
                }
            }
            3 => {
                let clip = self.flash.find(None, "set_4_1p2p");
                self.flash.clip_set_flags(clip, PROP_VISIBLE, show);
                for side in ['a', 'b'] {
                    for j in 0..2 {
                        let name = format!("mc_set_{n}{side}_{}", j + 1);
                        let clip = self.flash.find(None, &name);
                        self.flash.clip_set_flags(clip, PROP_VISIBLE, show);
                        if show {
                            let uv = row_uv(j, 0x100);
                            self.set_uv(&name, "mc_set_onoff_off", uv);
                            self.set_uv(&name, "mc_set_onoff_on", uv);
                        }
                    }
                }
            }
            _ => {
                for i in 0..2 {
                    let name = format!("mc_set_{n}_{}", i + 1);
                    let clip = self.flash.find(None, &name);
                    self.flash.clip_set_flags(clip, PROP_VISIBLE, show);
                    if show {
                        let uv = row_uv(i, 0x100);
                        self.set_uv(&name, "mc_set_onoff_off", uv);
                        self.set_uv(&name, "mc_set_onoff_on", uv);
                    }
                }
            }
        }
    }

    /// `DuelMenu_Draw`.
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
        for (i, guide) in ["bejita", "nappa"].into_iter().enumerate() {
            let parent = format!("mc_guide_{guide}");
            let eye = self
                .flash
                .find(Some(&parent), &format!("mc_guide_{guide}_eye"));
            anim::blink(&mut self.flash, eye, &mut self.blink[i], 0, random);
            let mouth = self
                .flash
                .find(Some(&parent), &format!("mc_guide_{guide}_mouth"));
            if self.talker == i as i32 {
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
        for i in 0..4 {
            let uv = row_uv(i, 0x200);
            let name = format!("mc_menu_plate_{}", i + 1);
            if i == 1 {
                let plate = self.flash.find(None, &name);
                let color = if self.flags & FLAG_NO_PAD2 != 0 {
                    0.4
                } else {
                    1.0
                };
                self.flash.clip_set_color(plate, color);
            }
            self.set_uv(&name, "mc_menu_text_off", uv);
            self.set_uv(&name, "mc_menu_text_on", uv);
        }
        for i in 0..3 {
            let uv = row_uv(i, 0x200);
            let name = format!("mc_mode_{}", i + 1);
            self.set_uv(&name, "mc_mode_text_off", uv);
            self.set_uv(&name, "mc_mode_text_on", uv);
        }
        let anime = self.flash.find(None, "mc_menu_plate_anime");
        anim::sheet(
            &mut self.flash,
            anime,
            &mut self.icon_timer,
            &mut self.icon_frame,
            Uv {
                u0: 0,
                v0: 0,
                u1: 0x40,
                v1: 0x10,
                tex: 0,
            },
            1,
            4,
            4,
        );
        for i in 0..3 {
            let uv = row_uv(i, 0x100);
            let name = format!("mc_dp_{}", i + 1);
            self.set_uv(&name, "mc_dp_value_off", uv);
            self.set_uv(&name, "mc_dp_value_on", uv);
        }
        for i in 0..6 {
            let uv = row_uv(i, 0x100);
            let name = format!("mc_battle_set_text_{}", i + 1);
            self.set_uv(&name, "mc_battle_set_text_off", uv);
            self.set_uv(&name, "mc_battle_set_text_on", uv);
        }
        for (i, rule) in [0usize, 1, 2, 3, 5].into_iter().enumerate() {
            let n = i + 1;
            match i {
                0..=2 => {
                    let uv = row_uv(self.rules[rule], 0x100);
                    let name = format!("mc_set_{n}");
                    self.set_uv(&name, &format!("mc_set_{n}_off"), uv);
                    self.set_uv(&name, &format!("mc_set_{n}_on"), uv);
                }
                3 => {
                    for (j, side) in ['a', 'b'].into_iter().enumerate() {
                        let uv = row_uv(self.rules[rule + j], 0x80);
                        let name = format!("mc_set_{n}{side}s");
                        self.set_uv(&name, &format!("mc_set_{n}{side}s_off"), uv);
                        self.set_uv(&name, &format!("mc_set_{n}{side}s_on"), uv);
                    }
                }
                _ => {
                    let uv = row_uv(self.rules[rule], 0x100);
                    let name = format!("mc_set_{n}");
                    self.set_uv(&name, "mc_set_onoff_off", uv);
                    self.set_uv(&name, "mc_set_onoff_on", uv);
                }
            }
        }
        for i in 0..5 {
            self.show_values(i, i == self.sel[LEVEL_VALUE]);
        }
        let cloud = self.flash.find(None, "mc_kumo");
        let wide = Uv {
            u0: 0,
            v0: 0,
            u1: 0x200,
            v1: 0x100,
            tex: 0,
        };
        anim::scroll(&mut self.flash, cloud, wide, &mut self.cloud, -0.284_444_45);
        const SPEED: [f32; 6] = [
            -1.219_047_6,
            1.422_222_2,
            1.422_222_2,
            1.706_666_7,
            1.422_222_2,
            1.422_222_2,
        ];
        for i in 0..6 {
            let clip = self.flash.find(None, &format!("mc_kemuri_{}", i + 1));
            if i < 2 {
                anim::scroll(&mut self.flash, clip, wide, &mut self.smoke[i], SPEED[i]);
            } else {
                let tall = Uv {
                    u0: 0,
                    v0: 0,
                    u1: 0x100,
                    v1: 0x200,
                    tex: 0,
                };
                anim::scroll_y(&mut self.flash, clip, tall, &mut self.smoke[i], SPEED[i]);
            }
        }
        let textures = &self.textures;
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

    /// `DuelMenu_Input`.
    fn input(&mut self, pressed: u32, repeat: u32, sounds: &mut Vec<TitleSound>) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & FLAG_STARTED == 0 {
            self.clip_goto(LEVEL_TOP, "fl_on_start");
            self.flags |= FLAG_STARTED;
        }
        let level = self.level;
        let up = repeat & PAD_UP != 0;
        let down = !up && repeat & PAD_DOWN != 0;
        let cross = pressed & PAD_CROSS != 0;
        let triangle = pressed & PAD_TRIANGLE != 0;
        match level {
            LEVEL_TOP if up || down => {
                self.idle = 0;
                self.clip_goto(level, "fl_off_start");
                let no_pad2 = self.flags & FLAG_NO_PAD2 != 0;
                let cur = &mut self.sel[level];
                if up {
                    *cur -= 1;
                    if no_pad2 && *cur == 1 {
                        *cur = 0;
                    }
                    if *cur < 0 {
                        *cur = 3;
                    }
                } else {
                    *cur += 1;
                    if no_pad2 && *cur == 1 {
                        *cur = 2;
                    }
                    if *cur >= 4 {
                        *cur = 0;
                    }
                }
                self.clip_goto(level, "fl_on_start");
                self.say(0, self.sel[LEVEL_TOP] + 1, sounds);
                sounds.push(TitleSound::Effect(1, 0));
            }
            LEVEL_TOP if cross => {
                self.idle = 0;
                let cur = self.sel[level];
                if cur == 1 && self.flags & FLAG_NO_PAD2 != 0 {
                    sounds.push(TitleSound::Effect(1, 7));
                } else if cur == 3 {
                    self.clip_goto(level, "fl_ok");
                    self.flash.goto_label(&format!("fl_mode_{}", cur + 1), true);
                    self.level = LEVEL_SETTING;
                    self.icon.close();
                    self.msg.set_text(self.set_text.clone());
                    self.voice_line = self.sel[LEVEL_SETTING] + 5;
                    // Voice_StopWithLip.
                    sounds.push(TitleSound::VoiceStop);
                    self.lip.clear();
                    self.clip_goto(self.level, "fl_on_start");
                    sounds.push(TitleSound::Effect(1, 1));
                } else {
                    self.clip_goto(level, "fl_ok");
                    self.flash.goto_label(&format!("fl_mode_{}", cur + 1), true);
                    self.level = LEVEL_TYPE;
                    self.say(0, self.sel[LEVEL_TYPE] + 5, sounds);
                    self.clip_goto(self.level, "fl_on_start");
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            LEVEL_TOP if triangle => {
                self.idle = 0;
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.result = 0;
                self.flash.goto_label("fl_exit", true);
                sounds.push(TitleSound::Effect(1, 2));
            }
            LEVEL_TOP => self.idle(sounds),
            LEVEL_TYPE | LEVEL_DP if up || down => {
                self.idle = 0;
                self.clip_goto(level, "fl_off_start");
                let cur = &mut self.sel[level];
                *cur = (*cur + if up { 2 } else { 1 }) % 3;
                self.clip_goto(level, "fl_on_start");
                if level == LEVEL_TYPE {
                    self.say(0, self.sel[LEVEL_TYPE] + 5, sounds);
                }
                sounds.push(TitleSound::Effect(1, 0));
            }
            LEVEL_TYPE | LEVEL_DP if cross => {
                self.idle = 0;
                if self.sel[LEVEL_TOP] == 1 && self.flags & FLAG_NO_PAD2 != 0 {
                    sounds.push(TitleSound::Effect(1, 7));
                    return;
                }
                self.clip_goto(level, "fl_ok");
                if level == LEVEL_TYPE && self.sel[level] == 2 {
                    self.level = LEVEL_DP;
                    self.clip_goto(self.level, "fl_on_start");
                    self.flash
                        .goto_label(&format!("fl_dp_{}", self.sel[LEVEL_TOP] + 1), true);
                } else {
                    self.start_state = 1;
                }
                sounds.push(TitleSound::Effect(1, 1));
            }
            LEVEL_TYPE if triangle => {
                self.idle = 0;
                self.clip_goto(level, "fl_off_start");
                self.level = LEVEL_TOP;
                self.clip_goto(self.level, "fl_on_start");
                let cur = self.sel[self.level];
                self.flash
                    .goto_label(&format!("fl_mode_{}_cansel", cur + 1), true);
                self.say(0, self.sel[LEVEL_TOP] + 1, sounds);
                sounds.push(TitleSound::Effect(1, 2));
            }
            LEVEL_DP if triangle => {
                self.idle = 0;
                self.clip_goto(level, "fl_off_start");
                self.level = LEVEL_TYPE;
                self.clip_goto(self.level, "fl_on_start");
                self.flash
                    .goto_label(&format!("fl_dp_{}_cansel", self.sel[LEVEL_TOP] + 1), true);
                sounds.push(TitleSound::Effect(1, 2));
            }
            LEVEL_TYPE | LEVEL_DP => self.idle(sounds),
            LEVEL_SETTING if up || down => {
                self.idle = 0;
                self.clip_goto(level, "fl_off_start");
                let cur = &mut self.sel[level];
                *cur = (*cur + if up { 5 } else { 1 }) % 6;
                self.clip_goto(level, "fl_on_start");
                self.voice_line = self.sel[LEVEL_SETTING] + 5;
                sounds.push(TitleSound::Effect(1, 0));
            }
            LEVEL_SETTING if cross => {
                self.idle = 0;
                self.clip_goto(level, "fl_ok");
                if self.sel[level] == 5 {
                    self.reset.state = 1;
                } else {
                    self.setup_value();
                    self.level = LEVEL_VALUE;
                    self.sel[LEVEL_VALUE] = self.sel[LEVEL_SETTING];
                    self.clip_goto(LEVEL_VALUE, "fl_on_start");
                    if self.sel[LEVEL_VALUE] == 3 {
                        self.column ^= 1;
                        self.clip_goto(LEVEL_VALUE, "fl_ok");
                        self.column ^= 1;
                    }
                    self.flash.goto_label("fl_value", true);
                }
                sounds.push(TitleSound::Effect(1, 1));
            }
            LEVEL_SETTING if triangle => {
                self.idle = 0;
                self.clip_goto(level, "fl_off_start");
                self.level = LEVEL_TOP;
                self.clip_goto(self.level, "fl_on_start");
                let cur = self.sel[self.level];
                self.flash
                    .goto_label(&format!("fl_mode_{}_cansel", cur + 1), true);
                self.icon.open();
                self.msg.set_text(self.msg_text.clone());
                self.say(0, self.sel[LEVEL_TOP] + 1, sounds);
                sounds.push(TitleSound::Effect(1, 2));
            }
            LEVEL_VALUE if up || down => {
                self.idle = 0;
                self.clip_goto(level, "fl_off_start");
                let count = self.value_count;
                let value = &mut self.value[self.column];
                *value = (*value + if up { count - 1 } else { 1 }) % count;
                self.clip_goto(level, "fl_on_start");
                sounds.push(TitleSound::Effect(1, 0));
            }
            LEVEL_VALUE if repeat & (PAD_LEFT | PAD_RIGHT) != 0 => {
                self.idle = 0;
                if self.sel[LEVEL_SETTING] == 3 {
                    self.clip_goto(level, "fl_ok");
                    self.column ^= 1;
                    self.clip_goto(level, "fl_on_start");
                    sounds.push(TitleSound::Effect(1, 0));
                }
            }
            LEVEL_VALUE if cross || triangle => {
                self.idle = 0;
                self.clip_goto(level, "fl_off_start");
                if self.sel[level] == 3 {
                    self.column ^= 1;
                    self.clip_goto(level, "fl_off_start");
                    self.column ^= 1;
                    if cross {
                        self.rules[self.rule + 1] = self.value[1];
                    }
                }
                if cross {
                    self.rules[self.rule] = self.value[0];
                }
                self.level = LEVEL_SETTING;
                self.clip_goto(self.level, "fl_on_start");
                self.flash.goto_label("fl_value_cansel", true);
                sounds.push(TitleSound::Effect(1, if cross { 1 } else { 2 }));
            }
            _ => {}
        }
    }
}

/// `DM_ROW_UV(n, w)`: row `n` of a strip of 0x20-high texts.
fn row_uv(n: i32, w: i32) -> Uv {
    Uv {
        u0: 0,
        v0: n * 0x20,
        u1: w,
        v1: n * 0x20 + 0x20,
        tex: 0,
    }
}

#[cfg(test)]
mod tests {
    /// Runs the real duel menu from the ISO in `rom/`, when it is there.
    #[test]
    fn real_duel_menu_reaches_single_battle() {
        use super::{
            ARCHIVE_FILE_ID, ARCHIVE_SECTION, DuelMenuAssets, DuelMenuLayout, DuelMenuResult,
            DuelMenuScreen, DuelProgress, LEVEL_SETTING, LEVEL_TYPE, LEVEL_VALUE,
        };
        use crate::anim::{VOICE_PLAYEND, VOICE_PLAYING};
        use crate::save::DEFAULT_RULES;
        use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, TitleSound};
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe, pack_section};

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
        let assets = DuelMenuAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let dialog = assets
            .dialog
            .textures
            .iter()
            .map(|t| t.as_ref().map(|_| 0))
            .collect();
        let mut data = assets.data;
        data.layout = DuelMenuLayout::new(0, dialog, |key| firsts[&key]);
        let mut font = crate::font::Font::new(Vec::new(), 20);
        let mut random = crate::rng::GameRandom::from_key(&[9, 9, 9, 9]);
        let mut menu =
            DuelMenuScreen::new(&data, DuelProgress::default(), DEFAULT_RULES, &mut random)
                .unwrap();
        let mut stat = 0;
        let mut step = |menu: &mut DuelMenuScreen, bits: u32, stat: &mut i32| {
            let out = menu.run_frame(bits, bits, false, *stat, &mut random, &mut font);
            for sound in &out.sounds {
                if matches!(sound, TitleSound::Voice(_)) {
                    *stat = VOICE_PLAYING;
                }
            }
            out.sounds
        };
        let mut greeted = None;
        for frame in 0..300 {
            if step(&mut menu, 0, &mut stat)
                .iter()
                .any(|s| matches!(s, TitleSound::Voice(_)))
            {
                greeted = Some(frame);
                break;
            }
        }
        println!("greeted at {greeted:?}");
        assert!(greeted.is_some());
        // Without a second controller "1P vs 2P" is drawn at 0.4.
        let quads = menu.flash.draw(&|_| true);
        assert!(
            quads
                .iter()
                .any(|q| q.pt[0][1] == 160.0 && q.rgba[..3] == [51, 51, 51])
        );
        // Without a second controller the cursor skips "1P vs 2P".
        step(&mut menu, PAD_DOWN, &mut stat);
        assert_eq!(menu.sel[0], 2);
        step(&mut menu, PAD_UP, &mut stat);
        assert_eq!(menu.sel[0], 0);
        // Battle settings, COM level, one up, confirm.
        step(&mut menu, PAD_UP, &mut stat);
        step(&mut menu, PAD_CROSS, &mut stat);
        assert_eq!(menu.level, LEVEL_SETTING);
        for _ in 0..30 {
            step(&mut menu, 0, &mut stat);
        }
        step(&mut menu, PAD_DOWN, &mut stat);
        step(&mut menu, PAD_CROSS, &mut stat);
        assert_eq!(menu.level, LEVEL_VALUE);
        for _ in 0..30 {
            step(&mut menu, 0, &mut stat);
        }
        step(&mut menu, PAD_UP, &mut stat);
        step(&mut menu, PAD_CROSS, &mut stat);
        assert_eq!(menu.rules()[1], 1);
        for _ in 0..30 {
            step(&mut menu, 0, &mut stat);
        }
        step(&mut menu, PAD_TRIANGLE, &mut stat);
        for _ in 0..30 {
            step(&mut menu, 0, &mut stat);
        }
        // 1P vs COM, single battle.
        step(&mut menu, PAD_DOWN, &mut stat);
        assert_eq!(menu.sel[0], 0);
        step(&mut menu, PAD_CROSS, &mut stat);
        assert_eq!(menu.level, LEVEL_TYPE);
        for _ in 0..30 {
            step(&mut menu, 0, &mut stat);
        }
        step(&mut menu, PAD_CROSS, &mut stat);
        for _ in 0..10 {
            step(&mut menu, 0, &mut stat);
        }
        stat = VOICE_PLAYEND;
        for _ in 0..60 {
            step(&mut menu, 0, &mut stat);
        }
        assert_eq!(menu.result(), Some(DuelMenuResult::Select(39)));
        assert_eq!(menu.progress(), DuelProgress::default());
    }
}
