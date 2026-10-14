//! The options screen (`Option_Run(1)`, src/menu/option.c, progress mode 62, `OptMode_Main`): Dende as guide, the
//! top page (save / load, the screen, sound and controller pages, exit), the screen page (the camera type with its
//! picture, the camera shake, the screen position, reset), the sound page (stereo / mono, the music and effect
//! volumes, the sound test, the voice language, reset) and the controller page (each pad's own key assignment and
//! vibration, reset). Every setting is written straight into the save; leaving after a change runs the save flow.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::dialog::{self, Dialog, DialogAssets, Pad};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quad};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::mcflow::{self, Done, McEnv, McFlow, McIo};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_START, PAD_TRIANGLE, PAD_UP, PadRepeat,
    TitleFrame, TitleSound,
};
use crate::windows::{IconWin, MsgWin};

/// `gMenuArc10`: file `gProgress->baseFile + 5`; `Option_Run(1)` unpacks its section 1.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 5;
pub const ARCHIVE_SECTION: usize = 1;
/// `BGM_OPTION`.
pub const BGM_FILE_ID: u32 = 0x10B18;
/// `OPTION_VOICE_BASE`.
const VOICE_BASE: u32 = 0x873A;
const FADE_FRAMES: i32 = 20;
/// `Bgm_Play(id + 0x10B16)` of the sound test.
const BGM_FIRST: u32 = 0x10B16;
/// `MVOICE_IDLE`.
const VOICE_IDLE: i32 = VOICE_PLAYEND;
/// `SAVE_FLAG_VOICE`, each pad's vibration (`SAVE_FLAG_PAD_A`) and own key assignment (`SAVE_FLAG_PAD_B`).
const FLAG_VOICE: i32 = 1;
const fn flag_vib(pad: usize) -> i32 {
    2 << pad
}
const fn flag_keys(pad: usize) -> i32 {
    8 << pad
}
/// The default key assignment.
const DEFAULT_KEYS: [i32; 8] = [2, 1, 0, 3, 4, 5, 6, 7];

// Option.cursor / Option.state (`OPT_ITEM_*`, `OPT_ST_*`).
const ST_SAVED: i32 = -2;
const ST_START: i32 = -1;
const ST_TOP: i32 = 0;
const ITEM_SAVE: i32 = 1;
const ITEM_SCREEN: i32 = 2;
const ITEM_SOUND: i32 = 3;
const ITEM_CTRL: i32 = 4;
const ITEM_EXIT: i32 = 5;
const ST_FADE: i32 = 6;
const ITEM_TYPE: i32 = 7;
const ITEM_SCR2: i32 = 8;
const ITEM_ADJUST: i32 = 9;
const ITEM_SCR_RESET: i32 = 10;
const ST_ADJUST_RESET: i32 = 11;
const ITEM_STEREO: i32 = 12;
const ITEM_VOLUME: i32 = 13;
const ITEM_BGM: i32 = 14;
const ITEM_VOICE: i32 = 15;
const ITEM_SND_RESET: i32 = 16;
const ITEM_KEYS: i32 = 17;
const ITEM_VIB: i32 = 18;
const ITEM_CTRL_RESET: i32 = 19;
const ST_VIB_EDIT: i32 = 20;
const ST_KEYS_PAD: i32 = 21;
const ST_KEYS_EDIT: i32 = 22;

/// `Option_PlateGoto`'s clips (`OPT_CLIP_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plate {
    Row,
    RowScreen,
    RowSound,
    Bottom,
    Pick,
    VibOff,
    KeyOff,
    Bgm,
    VolBgm,
    VolSe,
    RowCtrl,
}

/// `gOption->tex[n]` of `Option_Init`.
const FLASH_TEXTURES: [ImageTexture; 51] = [
    tex(34, 1),
    tex(34, 0),
    tex(17, 0),
    tex(16, 0),
    tex(17, 2),
    tex(17, 1),
    tex(29, 0),
    tex(16, 2),
    tex(27, 0),
    tex(27, 1),
    tex(15, 0),
    tex(14, 0),
    tex(15, 2),
    tex(15, 1),
    tex(14, 2),
    tex(12, 0),
    tex(10, 0),
    tex(11, 0),
    tex(10, 2),
    tex(10, 1),
    tex(11, 1),
    tex(6, 0),
    tex(5, 0),
    tex(6, 2),
    tex(6, 1),
    tex(3, 0),
    tex(3, 1),
    tex(4, 0),
    tex(23, 0),
    tex(23, 1),
    tex(23, 3),
    tex(1, 1),
    tex(1, 0),
    tex(2, 0),
    tex(20, 0),
    tex(28, 0),
    tex(13, 0),
    tex(9, 0),
    tex(13, 1),
    tex(19, 1),
    tex(19, 0),
    tex(19, 9),
    tex(19, 6),
    tex(24, 0),
    tex(26, 0),
    tex(25, 0),
    tex(21, 0),
    tex(21, 1),
    tex(21, 2),
    tex(22, 0),
    tex(21, 3),
];
/// `MsgWin_Init(MPACK_AT(res, 31), …)` and `IconWin_Init(MPACK_AT(res, 32), res 18)`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((31, 1), 0)), Some(((31, 1), 1)), None];
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((32, 1), 1)),
    Some(((32, 1), 0)),
    tex(18, 0),
    Some(((32, 1), 2)),
    tex(18, 1),
];
/// `IconWin_SetIcon(n)`: texture `n + 1` of section 18 (icons 0..4).
const ICON_SET: [ImageTexture; 5] = [tex(18, 1), tex(18, 2), tex(18, 3), tex(18, 4), tex(18, 5)];

/// What `Option_Init` reads from its unpacked section.
pub struct OptionAssets {
    pub data: OptionData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    /// Section 35, `Dialog_Init`'s file.
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct OptionData {
    pub fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    /// Section 33: the message window's lines; section 38: the reset question's messages.
    pub msg_text: Vec<u8>,
    pub dialog_msg: Vec<u8>,
    /// Section 36: mouth keys of the guide's lines.
    pub lips: Vec<u8>,
    /// Section 37: the sound test's music ids (before `BgmList_ApplyUnlocks`).
    pub bgm_ids: Vec<i32>,
    pub layout: OptionLayout,
}

#[derive(Clone, Debug, Default)]
pub struct OptionLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub icon_set: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
}

impl OptionLayout {
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
            icon_set: image_indices(&ICON_SET, &first),
            dialog,
        }
    }
}

impl OptionAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(
            res,
            [
                &FLASH_TEXTURES[..],
                &MSG_TEXTURES,
                &ICON_TEXTURES,
                &ICON_SET,
            ],
        )?;
        let dialog = DialogAssets::parse(pack_section(res, 35)?)?;
        // Section 37: a count (one header word and four more than the list has), the ids from + 0x10.
        let list = pack_section(res, 37)?;
        let word = |at: usize| {
            list.get(at..at + 4)
                .map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()))
        };
        let count = (word(0) - 1 - 4).max(0) as usize;
        let bgm_ids = (0..count).map(|i| word(0x10 + i * 4)).collect();
        Ok(Self {
            data: OptionData {
                fod: pack_section(res, 8)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 31)?, 2)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 32)?, 2)?.to_vec(),
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                msg_text: pack_section(res, 33)?.to_vec(),
                dialog_msg: pack_section(res, 38)?.to_vec(),
                lips: pack_section(res, 36)?.to_vec(),
                bgm_ids,
                layout: OptionLayout::default(),
            },
            background: decode_picture(res, 30)?,
            files,
            dialog,
        })
    }
}

/// `BgmList_ApplyUnlocks` in mode 0x3E: the random entry and the music the save has not unlocked show as locked.
fn apply_unlocks(ids: &mut [i32], save: &SaveData) {
    use crate::char_select::{BGM_LOCKED, BGM_RANDOM};
    for id in ids.iter_mut() {
        if *id == BGM_RANDOM || (*id != BGM_LOCKED && !save.bgm_unlocked(*id)) {
            *id = BGM_LOCKED;
        }
    }
}

fn uv(u0: i32, v0: i32, u1: i32, v1: i32) -> Uv {
    Uv {
        u0,
        v0,
        u1,
        v1,
        tex: 0,
    }
}

/// The caption of a key setting `k` (0..7) on the key page's sheet.
fn key_uv(k: i32) -> Uv {
    let (u0, v0) = match k {
        2 => (0x80, 0x50),
        1 => (0x80, 0x78),
        0 => (0x80, 0x28),
        3 => (0x80, 0),
        5 => (0, 0x28),
        6 => (0, 0x50),
        7 => (0, 0x78),
        _ => (0, 0),
    };
    uv(u0, v0, u0 + 0x80, v0 + 0x28)
}

/// The caption clip (`mc_button_text_%d`) of key row `i`.
const KEY_TEXT: [i32; 8] = [6, 7, 5, 4, 1, 0, 3, 2];

/// `Option` and the frame loop of `Option_Run`.
pub struct OptionScreen {
    layout: OptionLayout,
    lips: Vec<u8>,
    flash: Flash,
    msg: MsgWin,
    icon: IconWin,
    /// The reset question (`Dialog_Init(35)` with the message table of section 38) and the memory card flow's.
    dialog: Dialog,
    mc_dialog: Dialog,
    mcflow: McFlow,
    fade: ColorFade,
    pad_repeat: PadRepeat,
    lip: LipSync,
    cursor: i32,
    state: i32,
    page: i32,
    bgm_volume: i32,
    se_volume: i32,
    bgm_cursor: i32,
    bgm_top: i32,
    bgm_bottom: i32,
    bgm_extra: i32,
    value: i32,
    voice_line: i32,
    picker: i32,
    pad: usize,
    from_page: i32,
    ctrl_kind: i32,
    screen_x: i32,
    screen_y: i32,
    kind: i32,
    kind_prev: i32,
    blink: i32,
    talk: i32,
    reset_step: i32,
    key_row: i32,
    mark: (i32, i32),
    key_old: i32,
    value_old: i32,
    key_cursor: i32,
    key_edit: bool,
    key_held: bool,
    mc_busy: bool,
    started: bool,
    greeted: bool,
    dirty: bool,
    bgm_ids: Vec<i32>,
    /// `key[2][8]`: the default assignment, which the key page shows while a pad keeps it.
    keys: [[i32; 8]; 2],
    done: bool,
}

impl OptionScreen {
    /// `Option_Init` and the first `ColorFade_StartIn`.
    pub fn new(
        data: &OptionData,
        save: &SaveData,
        card_res: Option<&[u8]>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut bgm_ids = data.bgm_ids.clone();
        apply_unlocks(&mut bgm_ids, save);
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            Some(data.dialog_msg.clone()),
            0,
        )?;
        let mc_dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            None,
            0,
        )?;
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            false,
        )?;
        msg.open();
        let (bgm_volume, se_volume) = save.volumes();
        Ok(Self {
            layout: data.layout.clone(),
            lips: data.lips.clone(),
            flash,
            msg,
            icon,
            dialog,
            mc_dialog,
            mcflow: McFlow::new(card_res, None),
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad_repeat: PadRepeat::default(),
            lip: LipSync::default(),
            cursor: ITEM_SAVE,
            state: ST_START,
            page: 0,
            bgm_volume,
            se_volume,
            bgm_cursor: 0,
            bgm_top: 0,
            bgm_bottom: 6,
            bgm_extra: 0,
            value: 0,
            voice_line: 0,
            picker: 0,
            pad: 0,
            from_page: 0,
            ctrl_kind: 0,
            screen_x: 0,
            screen_y: 0,
            kind: 0,
            kind_prev: 0,
            blink: 0,
            talk: 0,
            reset_step: 0,
            key_row: 5,
            mark: (0, 0),
            key_old: -1,
            value_old: 0,
            key_cursor: 0,
            key_edit: false,
            key_held: false,
            mc_busy: false,
            started: false,
            greeted: false,
            dirty: false,
            bgm_ids,
            keys: [DEFAULT_KEYS; 2],
            done: false,
        })
    }

    /// The screen is over (`Option_Run` returned): back to the main menu.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// `OPT_VOICE(line)`.
    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        anim::play_with_subtitle(&self.lips, VOICE_BASE, line, &mut self.lip, sounds);
    }

    /// One iteration of `Option_Run`'s loop: the frame (`ColorFade_Update`, `Option_Draw`), then the condition
    /// `mcBusy || Option_Input()` for the next. `voice_stat` is `Voice_GetStat(0)`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        mc: &mut McEnv,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done {
            return frame;
        }
        let repeat = self.pad_repeat.update(held, pressed);
        self.fade.update();
        self.draw(pressed, repeat, voice_stat, random, font, mc, &mut frame);
        if !self.mc_busy {
            let pad = Input {
                held,
                pressed,
                repeat,
            };
            if !self.input(pad, voice_stat, mc, &mut frame) {
                self.done = true;
            }
        }
        frame
    }

    /// `Option_PlateGoto`: one of the screen's plates to a label of its timeline.
    fn plate(&mut self, plate: Plate, label: &str, save: &SaveData) {
        let flags = save.flags();
        let name = match plate {
            Plate::Row => format!("mc_menu_plate_{}", self.cursor),
            Plate::RowScreen => format!("mc_menu_plate_{}", self.cursor - 6),
            Plate::RowSound => format!("mc_menu_plate_{}", self.cursor - 11),
            Plate::Bottom => "mc_bottom_plate_1".to_owned(),
            Plate::Pick => format!("mc_select_plate_{}", self.value + 1),
            Plate::VibOff => format!(
                "mc_select_plate_{}",
                if flags & flag_vib(self.pad) != 0 {
                    3
                } else {
                    4
                }
            ),
            Plate::KeyOff => format!(
                "mc_select_plate_{}",
                if flags & flag_keys(self.pad) != 0 {
                    4
                } else {
                    3
                }
            ),
            Plate::Bgm => format!("mc_bgm_plate_{}", self.bgm_cursor - self.bgm_top + 1),
            Plate::VolBgm => format!("mc_vol_plate_bgm_{}", save.volumes().0),
            Plate::VolSe => format!("mc_vol_plate_se_{}", save.volumes().1),
            Plate::RowCtrl => format!("mc_menu_plate_{}", self.cursor - 16),
        };
        let clip = self.flash.find(None, &name);
        self.flash.clip_goto_label(clip, label);
    }

    fn goto(&mut self, label: &str) {
        self.flash.goto_label(label, true);
    }

    /// `Option_SetKeyMark`: where the cursor mark and the arrows go for row `row` of the key table.
    fn set_key_mark(&mut self, row: i32) {
        self.mark = match row {
            0 => (0xB4, 0x50),
            1 => (0xB4, 0x78),
            2 => (0, 0x78),
            3 => (0, 0x50),
            4 => (0, 0x28),
            5 => (0, 0),
            6 => (0xB4, 0x28),
            7 => (0xB4, 0),
            _ => self.mark,
        };
    }

    /// `Option_DimPickers`.
    fn dim_pickers(&mut self) {
        for i in 1..=4 {
            let clip = self.flash.find(None, &format!("mc_select_plate_{i}"));
            self.flash.clip_goto_label(clip, "fl_off_start");
        }
    }

    /// The row's line on the sound page.
    fn sound_line(&self) -> i32 {
        match self.cursor {
            ITEM_STEREO => 11,
            ITEM_VOLUME => self.cursor,
            ITEM_BGM => 17,
            ITEM_SND_RESET => 10,
            _ => 18,
        }
    }

    /// The row's line on the controller page.
    fn ctrl_line(&self) -> i32 {
        match self.cursor {
            ITEM_KEYS => 23,
            ITEM_VIB => 24,
            _ => 10,
        }
    }

    /// Up / down over a page's rows `first..=last`, `last` being the bottom plate (`bottom`: its rows from).
    #[allow(clippy::too_many_arguments)]
    fn move_rows(
        &mut self,
        up: bool,
        first: i32,
        last: i32,
        bottom_from: i32,
        row: Plate,
        save: &SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        let plate_of = |c: i32| if c >= bottom_from { Plate::Bottom } else { row };
        self.plate(plate_of(self.cursor), "fl_off_start", save);
        if up {
            self.cursor -= 1;
            if self.cursor < first {
                self.cursor = last;
            }
        } else {
            self.cursor += 1;
            if self.cursor > last {
                self.cursor = first;
            }
        }
        self.plate(plate_of(self.cursor), "fl_on_start", save);
        sounds.push(TitleSound::Effect(1, 0));
    }

    /// `Option_Input`: pad 0, one state a frame, nothing until the movie accepts input; false once the screen
    /// has faded out.
    fn input(
        &mut self,
        pad: Input,
        voice_stat: i32,
        mc: &mut McEnv,
        frame: &mut TitleFrame,
    ) -> bool {
        if self.flash.flags() & FLASH_PAD == 0 {
            return true;
        }
        let Input {
            held,
            pressed,
            repeat,
        } = pad;
        let save = &mut *mc.save;
        let sounds = &mut frame.sounds;
        let se = |sounds: &mut Vec<TitleSound>, id: u32| sounds.push(TitleSound::Effect(1, id));
        match self.state {
            ST_START => {
                if !self.started {
                    self.plate(Plate::Row, "fl_on_start", save);
                    self.started = true;
                    self.say(self.voice_line, sounds);
                    self.state = ST_TOP;
                }
            }
            ST_TOP => {
                if !self.greeted && voice_stat == VOICE_IDLE {
                    self.voice_line = 1;
                    self.say(1, sounds);
                    self.greeted = true;
                }
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    self.move_rows(
                        repeat & PAD_UP != 0,
                        ITEM_SAVE,
                        ITEM_EXIT,
                        ITEM_EXIT,
                        Plate::Row,
                        save,
                        sounds,
                    );
                    self.voice_line = self.cursor;
                    self.say(self.voice_line, sounds);
                    self.greeted = true;
                } else if pressed & PAD_CROSS != 0 {
                    // The original tests the state, which is 0 here: the row always goes to "fl_ok".
                    self.plate(Plate::Row, "fl_ok", save);
                    self.state = self.cursor;
                    se(sounds, 1);
                    match self.state {
                        ITEM_SAVE => {
                            if !self.greeted {
                                self.voice_line = 1;
                                self.say(1, sounds);
                                self.greeted = true;
                            }
                            self.goto("fl_value_in");
                            self.value = 1;
                            self.plate(Plate::Pick, "fl_off_start", save);
                            self.value = 0;
                            self.plate(Plate::Pick, "fl_on_start", save);
                            self.picker = 0;
                        }
                        ITEM_SCREEN | ITEM_SOUND | ITEM_CTRL => {
                            self.goto("fl_menu_change");
                            self.plate(Plate::Row, "fl_off_start", save);
                            let (cursor, line, plate, page) = match self.state {
                                ITEM_SCREEN => (ITEM_TYPE, ITEM_TYPE, Plate::RowScreen, 1),
                                ITEM_SOUND => (ITEM_STEREO, 11, Plate::RowSound, 2),
                                _ => (ITEM_KEYS, 23, Plate::RowCtrl, 3),
                            };
                            self.cursor = cursor;
                            self.voice_line = line;
                            self.plate(plate, "fl_on_start", save);
                            self.from_page = 0;
                            self.page = page;
                            self.say(line, sounds);
                        }
                        _ => {}
                    }
                    self.greeted = true;
                } else if pressed & PAD_TRIANGLE != 0 {
                    if self.cursor == ITEM_EXIT {
                        if self.dirty {
                            self.state = ST_SAVED;
                            self.mcflow.start(mcflow::MODE_SAVE_ALT, mc);
                            self.mc_busy = true;
                            se(sounds, 2);
                            return true;
                        }
                        self.fade = ColorFade::start_out(true, FADE_FRAMES);
                        self.state = ITEM_EXIT;
                    } else {
                        self.plate(Plate::Row, "fl_off_start", save);
                        self.cursor = ITEM_EXIT;
                        self.voice_line = self.cursor;
                        self.plate(Plate::Bottom, "fl_on_start", save);
                    }
                    se(sounds, 2);
                    self.say(self.cursor, sounds);
                }
            }
            ITEM_SCREEN => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    self.move_rows(
                        repeat & PAD_UP != 0,
                        ITEM_TYPE,
                        ITEM_SCR_RESET,
                        ITEM_SCR_RESET,
                        Plate::RowScreen,
                        save,
                        sounds,
                    );
                    self.voice_line = self.cursor;
                    self.say(self.voice_line, sounds);
                } else if pressed & PAD_CROSS != 0 {
                    match self.cursor {
                        ITEM_TYPE => {
                            self.plate(Plate::RowScreen, "fl_ok", save);
                            self.state = self.cursor;
                            self.kind = save.cam_dist_mode();
                            self.value_old = self.kind;
                            self.goto("fl_type_in");
                            se(sounds, 1);
                        }
                        ITEM_SCR2 => {
                            self.value = 0;
                            self.plate(Plate::Pick, "fl_off_start", save);
                            self.value = 1;
                            self.plate(Plate::Pick, "fl_off_start", save);
                            self.value = save.cam_shake_off();
                            self.plate(Plate::Pick, "fl_on_start", save);
                            self.value_old = self.value;
                            self.plate(Plate::RowScreen, "fl_ok", save);
                            self.state = self.cursor;
                            self.picker = 4;
                            self.goto("fl_value_in");
                            se(sounds, 1);
                        }
                        ITEM_ADJUST => {
                            (self.screen_x, self.screen_y) = save.screen_pos();
                            self.plate(Plate::RowScreen, "fl_ok", save);
                            self.icon.close();
                            self.msg.close();
                            self.state = self.cursor;
                            self.goto("fl_layout_in");
                            se(sounds, 1);
                        }
                        ITEM_SCR_RESET => {
                            self.plate(Plate::Bottom, "fl_ok", save);
                            self.state = self.cursor;
                        }
                        _ => {}
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.plate(Plate::RowScreen, "fl_off_start", save);
                    self.plate(Plate::Bottom, "fl_off_start", save);
                    self.back_to_top(ITEM_SCREEN, 1, save, sounds);
                    self.say(self.cursor, sounds);
                }
            }
            ITEM_TYPE => {
                if pressed & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.kind_prev = self.kind;
                    if pressed & PAD_LEFT != 0 {
                        self.kind -= 1;
                        if self.kind < 0 {
                            self.kind = 2;
                        }
                        self.goto("fl_type_right");
                    } else {
                        self.kind += 1;
                        if self.kind > 2 {
                            self.kind = 0;
                        }
                        self.goto("fl_type_left");
                    }
                    se(sounds, 0);
                } else if pressed & PAD_CROSS != 0 {
                    if self.value_old != self.kind {
                        self.dirty = true;
                    }
                    save.set_cam_dist_mode(self.kind);
                    self.state = ITEM_SCREEN;
                    self.plate(Plate::RowScreen, "fl_on_start", save);
                    self.goto("fl_type_out");
                    se(sounds, 1);
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.state = ITEM_SCREEN;
                    self.plate(Plate::RowScreen, "fl_on_start", save);
                    self.goto("fl_type_out");
                    se(sounds, 2);
                    self.say(self.cursor, sounds);
                }
            }
            ITEM_ADJUST => {
                let (x, y) = save.screen_pos();
                if repeat & PAD_LEFT != 0 && x > -16 {
                    save.set_screen_pos(x - 1, y);
                } else if repeat & PAD_RIGHT != 0 && x < 16 {
                    save.set_screen_pos(x + 1, y);
                } else if repeat & PAD_UP != 0 && y > -16 {
                    save.set_screen_pos(x, y - 1);
                } else if repeat & PAD_DOWN != 0 && y < 16 {
                    save.set_screen_pos(x, y + 1);
                } else if pressed & PAD_START != 0 {
                    self.state = ST_ADJUST_RESET;
                } else if pressed & PAD_CROSS != 0 {
                    // The original counts a change only when both axes moved.
                    if x != self.screen_x && y != self.screen_y {
                        self.dirty = true;
                    }
                    self.plate(Plate::RowScreen, "fl_on_start", save);
                    self.goto("fl_layout_out");
                    self.state = ITEM_SCREEN;
                    self.icon.open();
                    self.msg.open();
                    se(sounds, 1);
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.goto("fl_layout_out");
                    save.set_screen_pos(self.screen_x, self.screen_y);
                    self.state = ITEM_SCREEN;
                    self.msg.open();
                    self.icon.open();
                    self.plate(Plate::RowScreen, "fl_on_start", save);
                    se(sounds, 2);
                    self.say(self.cursor, sounds);
                }
            }
            ITEM_SOUND => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    self.move_rows(
                        repeat & PAD_UP != 0,
                        ITEM_STEREO,
                        ITEM_SND_RESET,
                        ITEM_SND_RESET,
                        Plate::RowSound,
                        save,
                        sounds,
                    );
                    self.voice_line = self.sound_line();
                    self.say(self.voice_line, sounds);
                } else if pressed & PAD_CROSS != 0 {
                    match self.cursor {
                        ITEM_STEREO => {
                            self.value = 0;
                            self.plate(Plate::Pick, "fl_off_start", save);
                            self.value = 1;
                            self.plate(Plate::Pick, "fl_off_start", save);
                            self.plate(Plate::RowSound, "fl_ok", save);
                            self.value = save.sound_mode();
                            self.value_old = self.value;
                            self.picker = 3;
                            self.state = self.cursor;
                            self.goto("fl_value_in");
                            self.plate(Plate::Pick, "fl_on_start", save);
                            se(sounds, 1);
                        }
                        ITEM_VOLUME => {
                            self.plate(Plate::RowSound, "fl_ok", save);
                            self.value = 0;
                            self.picker = 7;
                            (self.bgm_volume, self.se_volume) = save.volumes();
                            self.plate(Plate::VolBgm, "fl_on_start", save);
                            self.plate(Plate::VolSe, "fl_off_start", save);
                            self.state = self.cursor;
                            self.goto("fl_value_in");
                            se(sounds, 1);
                        }
                        ITEM_BGM => {
                            self.plate(Plate::RowSound, "fl_ok", save);
                            self.msg.close();
                            self.state = self.cursor;
                            self.goto("fl_bgm_in");
                            self.plate(Plate::Bgm, "fl_on_start", save);
                            sounds.push(TitleSound::BgmStop);
                            se(sounds, 1);
                        }
                        ITEM_VOICE => {
                            self.plate(Plate::RowSound, "fl_ok", save);
                            self.value = 0;
                            self.plate(Plate::Pick, "fl_off_start", save);
                            self.value = 1;
                            self.plate(Plate::Pick, "fl_off_start", save);
                            self.value = (save.flags() ^ 1) & 1;
                            self.plate(Plate::Pick, "fl_on_start", save);
                            self.state = self.cursor;
                            self.picker = 6;
                            self.goto("fl_value_in");
                            se(sounds, 1);
                        }
                        ITEM_SND_RESET => {
                            self.plate(Plate::Bottom, "fl_ok", save);
                            self.state = self.cursor;
                        }
                        _ => {}
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    let plate = if self.cursor != ITEM_SND_RESET {
                        Plate::RowSound
                    } else {
                        Plate::Bottom
                    };
                    self.plate(plate, "fl_off_start", save);
                    self.state = ST_TOP;
                    self.cursor = ITEM_SOUND;
                    self.voice_line = self.cursor;
                    self.page = 0;
                    self.say(self.voice_line, sounds);
                    self.from_page = 3;
                    self.plate(Plate::Row, "fl_on_start", save);
                    self.goto("fl_menu_change");
                    se(sounds, 2);
                }
            }
            ITEM_VOLUME => self.input_volume(pressed, repeat, save, sounds),
            ITEM_BGM => {
                if repeat & PAD_UP != 0 {
                    if self.bgm_cursor > 0 {
                        self.plate(Plate::Bgm, "fl_off_start", save);
                        self.bgm_cursor -= 1;
                        if self.bgm_top > self.bgm_cursor {
                            self.bgm_extra = self.bgm_bottom - 1;
                            self.bgm_top -= 1;
                            self.bgm_bottom -= 1;
                            self.goto("fl_bgm_down");
                        }
                        self.plate(Plate::Bgm, "fl_on_start", save);
                        se(sounds, 0);
                    }
                } else if repeat & PAD_DOWN != 0 {
                    if self.bgm_cursor < self.bgm_ids.len() as i32 - 1 {
                        self.plate(Plate::Bgm, "fl_off_start", save);
                        self.bgm_cursor += 1;
                        if self.bgm_bottom <= self.bgm_cursor {
                            self.bgm_extra = self.bgm_top;
                            self.bgm_top += 1;
                            self.bgm_bottom += 1;
                            self.goto("fl_bgm_up");
                        }
                        self.plate(Plate::Bgm, "fl_on_start", save);
                        se(sounds, 0);
                    }
                } else if pressed & PAD_CROSS != 0 {
                    self.plate(Plate::Bgm, "fl_ok", save);
                    let id = self.bgm_id(self.bgm_cursor);
                    if id != crate::char_select::BGM_LOCKED {
                        sounds.push(TitleSound::Bgm(BGM_FIRST + id.max(0) as u32));
                        se(sounds, 1);
                    } else {
                        se(sounds, 7);
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.msg.open();
                    self.state = ITEM_SOUND;
                    self.cursor = ITEM_BGM;
                    self.goto("fl_bgm_cansel");
                    self.plate(Plate::RowSound, "fl_on_start", save);
                    sounds.push(TitleSound::Bgm(BGM_FILE_ID));
                    se(sounds, 2);
                    self.say(17, sounds);
                }
            }
            ITEM_CTRL => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    self.move_rows(
                        repeat & PAD_UP != 0,
                        ITEM_KEYS,
                        ITEM_CTRL_RESET,
                        ITEM_CTRL_RESET,
                        Plate::RowCtrl,
                        save,
                        sounds,
                    );
                    self.voice_line = self.ctrl_line();
                    self.say(self.voice_line, sounds);
                } else if pressed & PAD_CROSS != 0 {
                    self.state = self.cursor;
                    self.value = 0;
                    self.pad = 0;
                    self.dim_pickers();
                    match self.state {
                        ITEM_VIB => {
                            self.plate(Plate::RowCtrl, "fl_ok", save);
                            self.ctrl_kind = 0;
                            self.picker = 2;
                            self.goto("fl_value_in");
                            self.plate(Plate::Pick, "fl_on_start", save);
                            self.plate(Plate::VibOff, "fl_ok", save);
                            se(sounds, 1);
                        }
                        ITEM_KEYS => {
                            self.plate(Plate::RowCtrl, "fl_ok", save);
                            self.ctrl_kind = 1;
                            self.picker = 2;
                            self.goto("fl_ctrl_in");
                            self.plate(Plate::Pick, "fl_on_start", save);
                            self.pad = 0;
                            self.plate(Plate::KeyOff, "fl_ok", save);
                            se(sounds, 1);
                        }
                        _ => self.plate(Plate::Bottom, "fl_ok", save),
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.from_page = 4;
                    self.plate(Plate::RowCtrl, "fl_off_start", save);
                    self.plate(Plate::Bottom, "fl_off_start", save);
                    self.state = ST_TOP;
                    self.cursor = ITEM_CTRL;
                    self.voice_line = self.cursor;
                    self.say(self.voice_line, sounds);
                    self.page = 0;
                    self.plate(Plate::Row, "fl_on_start", save);
                    self.goto("fl_menu_change");
                    se(sounds, 2);
                }
            }
            ITEM_KEYS => self.input_keys(pressed, save, sounds),
            ST_KEYS_PAD => self.input_keys_pad(pressed, save, sounds),
            ST_KEYS_EDIT => self.input_keys_edit(held, pressed, save, sounds),
            ITEM_VIB => {
                if pressed & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.plate(Plate::VibOff, "fl_off_start", save);
                    self.plate(Plate::Pick, "fl_off_start", save);
                    self.value ^= 1;
                    self.pad = self.value as usize;
                    self.plate(Plate::Pick, "fl_on_start", save);
                    self.plate(Plate::VibOff, "fl_ok", save);
                    se(sounds, 0);
                } else if pressed & PAD_DOWN != 0 {
                    self.plate(Plate::Pick, "fl_ok", save);
                    self.plate(Plate::VibOff, "fl_on_start", save);
                    self.pad = self.value as usize;
                    self.state = ST_VIB_EDIT;
                    self.value = i32::from(save.flags() & flag_vib(self.pad) != 0);
                    self.value_old = self.value;
                    se(sounds, 1);
                    self.voice_line = if self.value != 0 { 39 } else { 40 };
                    self.say(self.voice_line, sounds);
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.state = ITEM_CTRL;
                    self.cursor = ITEM_VIB;
                    self.goto("fl_value_out");
                    self.plate(Plate::RowCtrl, "fl_on_start", save);
                    self.voice_line = 24;
                    self.say(self.voice_line, sounds);
                    se(sounds, 2);
                }
            }
            ST_VIB_EDIT => {
                if pressed & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.plate(Plate::VibOff, "fl_off_start", save);
                    self.value ^= 1;
                    save.set_flags(save.flags() ^ flag_vib(self.pad));
                    self.plate(Plate::VibOff, "fl_on_start", save);
                    se(sounds, 0);
                    self.voice_line = if self.value != 0 { 39 } else { 40 };
                    self.say(self.voice_line, sounds);
                } else if pressed & PAD_UP != 0 {
                    if self.value_old != self.value {
                        self.dirty = true;
                    }
                    self.plate(Plate::VibOff, "fl_ok", save);
                    self.state = ITEM_VIB;
                    self.value = self.pad as i32;
                    self.plate(Plate::Pick, "fl_on_start", save);
                    se(sounds, 2);
                }
            }
            ITEM_SAVE | ITEM_SCR2 | ITEM_STEREO | ITEM_VOICE => {
                self.input_picker(pressed, mc, sounds);
            }
            ITEM_SCR_RESET | ST_ADJUST_RESET | ITEM_SND_RESET | ITEM_CTRL_RESET => {
                self.update_reset(pressed, repeat, save, sounds);
            }
            ITEM_EXIT => {
                if self.dirty {
                    self.state = ST_SAVED;
                    self.mcflow.start(mcflow::MODE_SAVE_ALT, mc);
                    self.mc_busy = true;
                } else {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.state = ST_FADE;
                }
            }
            ST_SAVED => {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.state = ST_FADE;
            }
            ST_FADE => {
                if self.fade.is_fading_out() {
                    sounds.push(TitleSound::FadeOutStep);
                }
                if self.fade.is_out_done() {
                    return false;
                }
            }
            _ => {}
        }
        true
    }

    /// The screen and sound pages' triangle back to the top page.
    fn back_to_top(
        &mut self,
        cursor: i32,
        from: i32,
        save: &SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        self.state = ST_TOP;
        self.cursor = cursor;
        self.voice_line = self.cursor;
        self.page = 0;
        self.from_page = from;
        self.plate(Plate::Row, "fl_on_start", save);
        self.goto("fl_menu_change");
        sounds.push(TitleSound::Effect(1, 2));
    }

    /// The music id of row `i` of the sound test (`bgmIds[i]`).
    fn bgm_id(&self, i: i32) -> i32 {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.bgm_ids.get(i).copied())
            .unwrap_or(crate::char_select::BGM_LOCKED)
    }

    /// The volume window: up / down pick the music or the effects, left / right change the level (the music's
    /// heard at once), confirm keeps it, triangle puts both back.
    fn input_volume(
        &mut self,
        pressed: u32,
        repeat: u32,
        save: &mut SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        let se = |sounds: &mut Vec<TitleSound>, id: u32| sounds.push(TitleSound::Effect(1, id));
        if pressed & PAD_UP != 0 && self.value != 0 {
            self.value = 0;
            self.plate(Plate::VolBgm, "fl_on_start", save);
            self.plate(Plate::VolSe, "fl_off_start", save);
            se(sounds, 0);
        } else if pressed & PAD_DOWN != 0 && self.value != 1 {
            self.value = 1;
            self.plate(Plate::VolBgm, "fl_off_start", save);
            self.plate(Plate::VolSe, "fl_on_start", save);
            se(sounds, 0);
        } else if pressed & PAD_TRIANGLE != 0 {
            self.plate(Plate::VolBgm, "fl_off_start", save);
            self.plate(Plate::VolSe, "fl_off_start", save);
            self.plate(Plate::RowSound, "fl_on_start", save);
            // Bgm_SetVolume(0x40) with the restored level: the main loop applies a changed volume.
            save.set_volumes(self.bgm_volume, self.se_volume);
            self.state = ITEM_SOUND;
            self.goto("fl_value_out");
            se(sounds, 2);
            self.say(self.cursor, sounds);
        } else {
            let music = self.value == 0;
            let plate = if music { Plate::VolBgm } else { Plate::VolSe };
            let (bgm, eff) = save.volumes();
            let cur = if music { bgm } else { eff };
            let set = |save: &mut SaveData, v: i32| {
                if music {
                    save.set_volumes(v, eff);
                } else {
                    save.set_volumes(bgm, v);
                }
            };
            if repeat & PAD_LEFT != 0 {
                self.plate(plate, "fl_off_start", save);
                set(save, if cur - 1 < 0 { 9 } else { cur - 1 });
                self.plate(plate, "fl_on_start", save);
                se(sounds, 0);
            } else if repeat & PAD_RIGHT != 0 {
                self.plate(plate, "fl_off_start", save);
                set(save, if cur + 1 > 9 { 0 } else { cur + 1 });
                self.plate(plate, "fl_on_start", save);
                se(sounds, 0);
            } else if pressed & PAD_CROSS != 0 {
                let kept = if music {
                    &mut self.bgm_volume
                } else {
                    &mut self.se_volume
                };
                if *kept != cur {
                    self.dirty = true;
                }
                *kept = cur;
                self.plate(plate, "fl_ok", save);
                se(sounds, 1);
            }
        }
    }

    /// The key page's pad picker: left / right the pad, down into its own-assignment switch, triangle back.
    fn input_keys(&mut self, pressed: u32, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        let se = |sounds: &mut Vec<TitleSound>, id: u32| sounds.push(TitleSound::Effect(1, id));
        if pressed & (PAD_LEFT | PAD_RIGHT) != 0 {
            self.plate(Plate::KeyOff, "fl_off_start", save);
            self.plate(Plate::Pick, "fl_off_start", save);
            self.value ^= 1;
            self.pad = self.value as usize;
            self.plate(Plate::Pick, "fl_on_start", save);
            self.plate(Plate::KeyOff, "fl_ok", save);
            se(sounds, 0);
        } else if pressed & PAD_DOWN != 0 {
            self.plate(Plate::Pick, "fl_ok", save);
            self.plate(Plate::KeyOff, "fl_on_start", save);
            self.pad = self.value as usize;
            self.value = i32::from(save.flags() & flag_keys(self.pad) != 0);
            self.value_old = self.value;
            self.state = ST_KEYS_PAD;
            se(sounds, 1);
            self.voice_line = if self.value != 0 { 31 } else { 30 };
            self.say(self.voice_line, sounds);
        } else if pressed & PAD_TRIANGLE != 0 {
            self.state = ITEM_CTRL;
            self.cursor = ITEM_KEYS;
            self.voice_line = 23;
            self.say(self.voice_line, sounds);
            self.plate(Plate::RowCtrl, "fl_on_start", save);
            self.goto("fl_ctrl_out");
            se(sounds, 2);
        }
    }

    /// A pad's own-assignment switch: left / right toggles it, up applies (the default, or the edited copy) and
    /// goes back, confirm with the switch on edits the keys.
    fn input_keys_pad(&mut self, pressed: u32, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        let se = |sounds: &mut Vec<TitleSound>, id: u32| sounds.push(TitleSound::Effect(1, id));
        let pad = self.pad;
        if pressed & (PAD_LEFT | PAD_RIGHT) != 0 && !self.key_edit {
            self.plate(Plate::KeyOff, "fl_off_start", save);
            self.value ^= 1;
            save.set_flags(save.flags() ^ flag_keys(pad));
            self.plate(Plate::KeyOff, "fl_on_start", save);
            se(sounds, 0);
            self.voice_line = if self.value != 0 { 31 } else { 30 };
            self.say(self.voice_line, sounds);
        } else if pressed & PAD_UP != 0 {
            // The original compares the two tables' addresses: always a change.
            self.dirty = true;
            for k in 0..8 {
                let v = if self.value == 0 {
                    DEFAULT_KEYS[k]
                } else {
                    save.key_edit(pad, k)
                };
                save.set_key(pad, k, v);
            }
            self.plate(Plate::KeyOff, "fl_ok", save);
            self.value = pad as i32;
            self.state = ITEM_KEYS;
            self.plate(Plate::Pick, "fl_on_start", save);
            se(sounds, 2);
        } else if save.flags() & (flag_keys(0) | flag_keys(1)) != 0
            && pressed & PAD_CROSS != 0
            && self.value != 0
        {
            self.plate(Plate::KeyOff, "fl_ok", save);
            self.state = ST_KEYS_EDIT;
            self.key_edit = true;
            self.key_row = 3;
            self.key_cursor = 2;
            self.set_key_mark(self.key_row);
            se(sounds, 1);
        }
    }

    /// The key editor: the cursor over two columns of four, confirm held with left / right changes the row's
    /// button; releasing it swaps the button with the row that had it.
    fn input_keys_edit(
        &mut self,
        held: u32,
        pressed: u32,
        save: &mut SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        let se = |sounds: &mut Vec<TitleSound>, id: u32| sounds.push(TitleSound::Effect(1, id));
        let pad = self.pad;
        self.key_held = false;
        let free = self.key_old < 0;
        if pressed & PAD_LEFT != 0 && free {
            self.key_cursor -= 4;
            if self.key_cursor < 0 {
                self.key_cursor += 8;
            }
            se(sounds, 0);
        } else if pressed & PAD_RIGHT != 0 && free {
            self.key_cursor += 4;
            if self.key_cursor > 7 {
                self.key_cursor -= 8;
            }
            se(sounds, 0);
        } else if pressed & PAD_UP != 0 && free {
            if self.key_cursor == 2 || self.key_cursor == 6 {
                self.key_cursor += 1;
            } else {
                self.key_cursor -= 1;
            }
            se(sounds, 0);
        } else if pressed & PAD_DOWN != 0 && free {
            if self.key_cursor == 3 || self.key_cursor == 7 {
                self.key_cursor -= 1;
            } else {
                self.key_cursor += 1;
            }
            se(sounds, 0);
        } else if pressed & PAD_TRIANGLE != 0 && free {
            // Address comparisons again: always a change.
            self.dirty = true;
            self.key_edit = false;
            self.state = ST_KEYS_PAD;
            self.plate(Plate::KeyOff, "fl_on_start", save);
            se(sounds, 2);
        }
        if pressed != 0 {
            self.key_row = match self.key_cursor {
                0 => 5,
                1 => 4,
                2 => 3,
                3 => 2,
                4 => 7,
                5 => 6,
                6 => 0,
                7 => 1,
                _ => self.key_row,
            };
            self.set_key_mark(self.key_row);
        }
        let row = self.key_row.clamp(0, 7) as usize;
        if held & PAD_CROSS != 0 {
            self.key_held = true;
            if self.key_old < 0 {
                self.key_old = save.key_edit(pad, row);
            }
        }
        if held & PAD_CROSS != 0 && pressed & PAD_LEFT != 0 {
            let v = save.key_edit(pad, row) - 1;
            save.set_key_edit(pad, row, if v < 0 { 3 } else { v });
            se(sounds, 0);
        } else if held & PAD_CROSS != 0 && pressed & PAD_RIGHT != 0 {
            let v = save.key_edit(pad, row) + 1;
            save.set_key_edit(pad, row, if v > 3 { 0 } else { v });
            se(sounds, 0);
        }
        if !self.key_held && self.key_old >= 0 {
            let now = save.key_edit(pad, row);
            if self.key_old != now
                && let Some(i) = (0..8).find(|&i| i != row && save.key_edit(pad, i) == now)
            {
                save.set_key_edit(pad, i, self.key_old);
            }
            self.key_old = -1;
        }
    }

    /// The two-way pickers: save / load, the camera shake, stereo / mono, the voice language.
    fn input_picker(&mut self, pressed: u32, mc: &mut McEnv, sounds: &mut Vec<TitleSound>) {
        let se = |sounds: &mut Vec<TitleSound>, id: u32| sounds.push(TitleSound::Effect(1, id));
        if pressed & (PAD_LEFT | PAD_RIGHT) != 0 {
            self.plate(Plate::Pick, "fl_off_start", mc.save);
            self.value ^= 1;
            self.plate(Plate::Pick, "fl_on_start", mc.save);
            se(sounds, 0);
        } else if pressed & PAD_CROSS != 0 {
            self.plate(Plate::Pick, "fl_ok", mc.save);
            se(sounds, 1);
            match self.state {
                ITEM_STEREO => {
                    if self.value_old != self.value {
                        self.dirty = true;
                    }
                    self.state = ITEM_SOUND;
                    // SndOpt_Apply: the main loop takes the save's sound options.
                    mc.save.set_sound_mode(self.value);
                    self.goto("fl_value_out");
                    self.plate(Plate::RowSound, "fl_on_start", mc.save);
                }
                ITEM_SCR2 => {
                    if self.value_old != self.value {
                        self.dirty = true;
                    }
                    mc.save.set_cam_shake_off(self.value);
                    self.state = ITEM_SCREEN;
                    self.cursor = ITEM_SCR2;
                    self.goto("fl_value_out");
                    self.plate(Plate::RowScreen, "fl_on_start", mc.save);
                }
                ITEM_SAVE => {
                    let mode = if self.value == 0 {
                        mcflow::MODE_SAVE
                    } else {
                        mcflow::MODE_LOAD
                    };
                    self.mcflow.start(mode, mc);
                    self.mc_busy = true;
                }
                ITEM_VOICE => {
                    self.dirty = true;
                    self.state = ITEM_SOUND;
                    self.goto("fl_value_out");
                    self.plate(Plate::RowSound, "fl_on_start", mc.save);
                    let flags = mc.save.flags();
                    mc.save.set_flags(if self.value != 0 {
                        flags & !FLAG_VOICE
                    } else {
                        flags | FLAG_VOICE
                    });
                }
                _ => {}
            }
        } else if pressed & PAD_TRIANGLE != 0 {
            se(sounds, 2);
            self.plate(Plate::Pick, "fl_off_start", mc.save);
            match self.state {
                ITEM_STEREO => {
                    self.value = mc.save.sound_mode();
                    self.plate(Plate::Pick, "fl_on_start", mc.save);
                    self.state = ITEM_SOUND;
                    self.plate(Plate::RowSound, "fl_on_start", mc.save);
                    self.voice_line = 11;
                    self.say(self.voice_line, sounds);
                }
                ITEM_SCR2 => {
                    self.value = mc.save.cam_shake_off();
                    self.plate(Plate::Pick, "fl_on_start", mc.save);
                    self.state = ITEM_SCREEN;
                    self.plate(Plate::RowScreen, "fl_on_start", mc.save);
                    self.say(self.cursor, sounds);
                }
                ITEM_SAVE => {
                    self.state = ST_TOP;
                    self.cursor = ITEM_SAVE;
                    self.plate(Plate::Row, "fl_on_start", mc.save);
                    self.say(self.cursor, sounds);
                }
                ITEM_VOICE => {
                    self.state = ITEM_SOUND;
                    self.cursor = ITEM_VOICE;
                    self.plate(Plate::RowSound, "fl_on_start", mc.save);
                    self.say(18, sounds);
                }
                _ => {}
            }
            self.goto("fl_value_out");
        }
    }

    /// `Option_UpdateReset`: the "reset to defaults?" question of the three pages and of the screen position.
    fn update_reset(
        &mut self,
        pressed: u32,
        repeat: u32,
        save: &mut SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        let mut effects = Vec::new();
        let pad = Pad { pressed, repeat };
        match self.reset_step {
            0 => {
                self.dialog.set_cursor(1);
                self.dialog.set_msg(0);
                self.dialog.set_choices(true);
                self.dialog.start(dialog::Command::Open, &mut effects);
                self.reset_step = 1;
            }
            1 => {
                let answer = self.dialog.input(true, pad, &mut effects);
                if answer < 0 {
                    self.reset_step = 3;
                } else if answer > 0 {
                    self.reset_step = 2;
                }
            }
            2 => {
                match self.state {
                    ITEM_SCR_RESET => {
                        if save.screen_pos() != (0, 0)
                            || save.cam_dist_mode() != 0
                            || save.cam_shake_off() != 0
                        {
                            self.dirty = true;
                        }
                        save.set_screen_pos(0, 0);
                        save.set_cam_dist_mode(0);
                        save.set_cam_shake_off(0);
                    }
                    ST_ADJUST_RESET => {
                        if save.screen_pos() != (0, 0) {
                            self.dirty = true;
                        }
                        save.set_screen_pos(0, 0);
                    }
                    ITEM_SND_RESET => {
                        if save.sound_mode() != 0
                            || save.volumes() != (9, 9)
                            || save.flags() & FLAG_VOICE == 0
                        {
                            self.dirty = true;
                        }
                        save.set_sound_mode(0);
                        save.set_volumes(9, 9);
                        save.set_flags(save.flags() | FLAG_VOICE);
                    }
                    _ => {
                        let flags = save.flags() & !(flag_keys(0) | flag_keys(1));
                        save.set_flags(flags | flag_vib(0) | flag_vib(1));
                        for pad in 0..2 {
                            for (k, v) in DEFAULT_KEYS.iter().enumerate() {
                                save.set_key_edit(pad, k, *v);
                            }
                        }
                    }
                }
                self.reset_step = 3;
            }
            3 => {
                self.dialog.set_choices(false);
                self.dialog.start(dialog::Command::Close, &mut effects);
                self.reset_step = 4;
            }
            4 => {
                if self.dialog.is_closed() {
                    self.state = match self.state {
                        ITEM_SCR_RESET => ITEM_SCREEN,
                        ST_ADJUST_RESET => ITEM_ADJUST,
                        ITEM_SND_RESET => ITEM_SOUND,
                        _ => ITEM_CTRL,
                    };
                    if self.state != ITEM_ADJUST {
                        self.plate(Plate::Bottom, "fl_on_start", save);
                    }
                    self.reset_step = 0;
                }
            }
            _ => {}
        }
        sounds.extend(
            effects
                .into_iter()
                .map(|(bank, id)| TitleSound::Effect(bank, id)),
        );
    }

    fn set_text_uv(&mut self, parent: &str, child: &str, uv: Uv, tex: Option<i32>) {
        let r = self.flash.find(Some(parent), child);
        self.flash.clip_set_uv(r, uv);
        if let Some(t) = tex {
            self.flash.clip_set_tex(r, t);
        }
    }

    /// `Option_SetMenuText`: both text clips of a menu plate, with their first picture.
    fn menu_text(&mut self, n: i32, uv: Uv) {
        let name = format!("mc_menu_plate_{n}");
        self.set_text_uv(&name, "mc_menu_text_off", uv, Some(0));
        self.set_text_uv(&name, "mc_menu_text_on", uv, Some(0));
    }

    /// A menu plate's text clips from the second picture.
    fn menu_text2(&mut self, n: i32, uv: Uv) {
        let name = format!("mc_menu_plate_{n}");
        self.set_text_uv(&name, "mc_menu_text_off", uv, Some(1));
        self.set_text_uv(&name, "mc_menu_text_on", uv, Some(1));
    }

    /// `Option_SetBottomText` (`tex` 0), or the rectangle alone.
    fn bottom_text(&mut self, n: i32, uv: Uv, tex: Option<i32>) {
        let name = format!("mc_bottom_plate_{n}");
        self.set_text_uv(&name, "mc_bottom_text_off", uv, tex);
        self.set_text_uv(&name, "mc_bottom_text_on", uv, tex);
    }

    fn hide(&mut self, parent: Option<&str>, name: &str) {
        let r = self.flash.find(parent, name);
        self.flash.clip_set_flags(r, PROP_VISIBLE, false);
    }

    fn hide_plate(&mut self, n: i32) {
        self.hide(None, &format!("mc_menu_plate_{n}"));
    }

    /// `Option_Draw`: every clip set up from the state, then the picture, the movie, the windows, the fade, and the
    /// memory card flow or the reset question.
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
        frame.quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        self.flash.advance();
        let eye = self.flash.find(None, "mc_dende_eye");
        let mouth = self.flash.find(None, "mc_dende_mouth");
        anim::blink(&mut self.flash, eye, &mut self.blink, 0, random);
        anim::talk(
            &mut self.flash,
            mouth,
            &mut self.talk,
            0,
            &mut self.lip,
            voice_stat,
            random,
        );
        // The icon of the page the cursor is in.
        let s = self.state;
        let icon = if s == ITEM_SCREEN || (ITEM_TYPE..=ST_ADJUST_RESET).contains(&s) {
            1
        } else if s == ITEM_SOUND || ((ITEM_STEREO..=ITEM_SND_RESET).contains(&s) && s != ITEM_BGM)
        {
            2
        } else if s == ITEM_BGM {
            3
        } else if s == ITEM_CTRL || s >= ITEM_KEYS {
            4
        } else {
            0
        };
        self.icon
            .set_texture(4, self.layout.icon_set.get(icon).copied().flatten());
        let save = &*mc.save;
        match self.page {
            0 => self.draw_top_page(),
            1 => self.draw_screen_page(save),
            2 => self.draw_sound_page(),
            _ => self.draw_ctrl_page(save),
        }
        self.draw_pickers();
        self.draw_volume_and_list();
        let textures = &self.layout.flash;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        frame.quads.extend(
            self.flash
                .draw(&has_texture)
                .iter()
                .map(|q| screen_quad(q, textures)),
        );
        self.icon.draw(&mut frame.quads);
        self.msg.draw(self.voice_line, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.mc_busy {
            let mut effects = Vec::new();
            let mut io = McIo {
                dialog: &mut self.mc_dialog,
                pad: Pad { pressed, repeat },
                sounds: &mut effects,
            };
            let (state, events) = self.mcflow.update(mc, &mut io, font, &mut frame.quads);
            // Option_OnSaved, the flow's "done" callback.
            if events
                .iter()
                .any(|e| matches!(e, mcflow::Event::Done(Done::Done)))
            {
                self.dirty = false;
            }
            self.mc_busy = state != 0;
            if !self.mc_busy {
                self.plate(Plate::Pick, "fl_on_start", mc.save);
            }
            frame
                .sounds
                .extend(effects.into_iter().map(|(b, id)| TitleSound::Effect(b, id)));
        } else {
            self.dialog.draw(true, font, &mut frame.quads);
        }
    }

    fn draw_top_page(&mut self) {
        for i in 0..4 {
            self.menu_text(i + 1, uv(0, i * 0x20, 0x200, i * 0x20 + 0x20));
        }
        match self.from_page {
            1 => {
                for i in 0..3 {
                    self.menu_text(i + 5, uv(0, i * 0x20 + 0x80, 0x200, i * 0x20 + 0xA0));
                }
                self.hide_plate(8);
            }
            2 | 3 => {
                self.menu_text(5, uv(0, 0xE0, 0x200, 0x100));
                for i in 0..3 {
                    self.menu_text2(i + 6, uv(0, i * 0x20, 0x200, i * 0x20 + 0x20));
                }
                if self.from_page == 2 {
                    self.hide_plate(8);
                }
            }
            _ => {
                for i in 0..2 {
                    self.menu_text2(i + 5, uv(0, i * 0x20 + 0x60, 0x200, i * 0x20 + 0x80));
                }
                self.hide_plate(7);
                self.hide_plate(8);
            }
        }
        self.bottom_text(1, uv(0, 0, 0x100, 0x20), None);
        self.bottom_text(2, uv(0, 0x20, 0x100, 0x40), Some(0));
    }

    fn draw_screen_page(&mut self, save: &SaveData) {
        for i in 0..3 {
            self.menu_text(i + 1, uv(0, i * 0x20 + 0x80, 0x200, i * 0x20 + 0xA0));
        }
        self.hide_plate(4);
        for i in 0..4 {
            self.menu_text(i + 5, uv(0, i * 0x20, 0x200, i * 0x20 + 0x20));
        }
        let bg = self.flash.find(None, "mc_layout_bg");
        self.flash.clip_set_tex(bg, save.cam_dist_mode() * 2);
        let window = self.flash.find(None, "mc_layout_window");
        self.flash.clip_set_tex(window, 0);
        self.bottom_text(1, uv(0, 0x20, 0x100, 0x40), Some(0));
        self.bottom_text(2, uv(0, 0, 0x100, 0x20), Some(0));
        for (n, kind) in [(1, self.kind), (2, self.kind_prev)] {
            let text = self.flash.find(None, &format!("mc_type_text_{n}"));
            self.flash
                .clip_set_uv(text, uv(0, kind * 0x28, 0x200, kind * 0x28 + 0x28));
            let image = self.flash.find(None, &format!("mc_type_image_{n}"));
            self.flash.clip_set_uv(image, uv(0, 0, 0x100, 0x100));
            self.flash.clip_set_tex(image, kind);
        }
    }

    fn draw_sound_page(&mut self) {
        self.menu_text(1, uv(0, 0xE0, 0x200, 0x100));
        for i in 0..3 {
            self.menu_text2(i + 2, uv(0, i * 0x20, 0x200, i * 0x20 + 0x20));
        }
        for i in 0..4 {
            self.menu_text(i + 5, uv(0, i * 0x20, 0x200, i * 0x20 + 0x20));
        }
        self.bottom_text(1, uv(0, 0x20, 0x100, 0x40), Some(0));
        self.bottom_text(2, uv(0, 0, 0x100, 0x20), Some(0));
    }

    fn draw_ctrl_page(&mut self, save: &SaveData) {
        for i in 0..2 {
            self.menu_text2(i + 1, uv(0, i * 0x20 + 0x60, 0x200, i * 0x20 + 0x80));
        }
        for i in 0..4 {
            self.menu_text(i + 5, uv(0, i * 0x20, 0x200, i * 0x20 + 0x20));
        }
        self.hide_plate(3);
        self.hide_plate(4);
        self.bottom_text(1, uv(0, 0x20, 0x100, 0x40), None);
        self.bottom_text(2, uv(0, 0, 0x100, 0x20), None);
        // The eight key names: the assignment being edited.
        let mark = self.flash.find(None, "mc_button_mark_off");
        self.flash.clip_set_uv(mark, uv(0x30, 0, 0x60, 0xA0));
        let pad = self.pad.min(1);
        for i in 0..8 {
            let r = self
                .flash
                .find(None, &format!("mc_button_text_{}", KEY_TEXT[i]));
            self.flash.clip_set_uv(r, key_uv(save.key_edit(pad, i)));
        }
        // The default assignment instead, while the page offers it.
        let s = self.state;
        if matches!(s, ITEM_KEYS | ST_KEYS_PAD | ITEM_CTRL) {
            let own = save.flags() & flag_keys(if self.value == 0 { 0 } else { 1 }) != 0;
            let skip = (matches!(s, ITEM_KEYS | ITEM_CTRL) && own)
                || (s == ST_KEYS_PAD && self.value != 0);
            if !skip {
                for i in 0..8 {
                    let r = self
                        .flash
                        .find(None, &format!("mc_button_text_{}", KEY_TEXT[i]));
                    self.flash.clip_set_uv(r, key_uv(self.keys[pad][i]));
                }
            }
        }
        // The mark on the row being edited.
        let on = self.flash.find(None, "mc_button_mark_on");
        if self.key_edit {
            let (u0, v0) = match self.key_row {
                0 => (0x30, 0x50),
                1 => (0x30, 0x78),
                2 => (0, 0x78),
                3 => (0, 0x50),
                4 => (0, 0x28),
                6 => (0x30, 0x28),
                7 => (0x30, 0),
                _ => (0, 0),
            };
            self.flash.clip_set_uv(on, uv(u0, v0, u0 + 0x30, v0 + 0x28));
            self.flash.clip_set_offset(on, self.mark.0, self.mark.1);
            self.flash.clip_set_flags(on, PROP_VISIBLE, true);
        } else {
            self.flash.clip_set_flags(on, PROP_VISIBLE, false);
        }
    }

    /// The two-way pickers' captions and the pad pickers'.
    fn draw_pickers(&mut self) {
        let base = match self.picker {
            4 | 2 => 0,
            0 => 0x40,
            3 => 0x80,
            _ => 0xC0,
        };
        for i in 0..2 {
            let name = format!("mc_select_plate_{}", i + 1);
            let r = uv(0, base + i * 0x20, 0x100, base + i * 0x20 + 0x20);
            if self.picker < 7 {
                let tex = (self.picker == 2).then_some(1);
                self.set_text_uv(&name, "mc_select_text_off", r, tex);
                self.set_text_uv(&name, "mc_select_text_on", r, tex);
            } else {
                self.hide(None, &name);
            }
        }
        let ctrl = if self.ctrl_kind != 0 { 0x40 } else { 0 };
        for i in 0..2 {
            let name = format!("mc_select_plate_{}", i + 3);
            let r = uv(0, ctrl + i * 0x20, 0x100, ctrl + i * 0x20 + 0x20);
            if !matches!(self.picker, 3 | 4 | 0 | 6) && self.picker < 7 {
                let tex = Some(i32::from(self.ctrl_kind != 0));
                self.set_text_uv(&name, "mc_select_text_off", r, tex);
                self.set_text_uv(&name, "mc_select_text_on", r, tex);
            } else {
                self.hide(None, &name);
            }
        }
    }

    /// The volume window, the sound test's list and scroll bar, the volume marks and the arrows.
    fn draw_volume_and_list(&mut self) {
        let mut num = 0;
        'rows: for i in 0..3 {
            for col in 0..4 {
                let r = uv(col * 0x20, i * 0x20, col * 0x20 + 0x20, i * 0x20 + 0x20);
                for kind in ["bgm", "se"] {
                    let name = format!("mc_vol_plate_{kind}_{num}");
                    if self.picker < 7 {
                        self.hide(None, &name);
                    } else {
                        self.set_text_uv(&name, "mc_vol_num_off", r, None);
                        self.set_text_uv(&name, "mc_vol_num_on", r, None);
                    }
                }
                num += 1;
                if num >= 10 {
                    break 'rows;
                }
            }
        }
        if self.picker < 7 {
            for name in ["mc_vol_now_bgm", "mc_vol_now_se", "mc_vol_window"] {
                self.hide(None, name);
            }
        }
        let count = self.bgm_ids.len() as i32;
        if self.state == ITEM_BGM {
            let scissor = Some([0, 0x45, 0x200, 0x174]);
            for i in 0..6 {
                let name = format!("mc_bgm_plate_{}", i + 1);
                if i == 0 || i == 5 {
                    let r = self.flash.find(None, &name);
                    self.flash.clip_set_scissor(r, scissor);
                }
                let id = self.bgm_id(self.bgm_top + i);
                let r = uv(0, id % 8 * 0x20, 0x200, id % 8 * 0x20 + 0x20);
                self.set_text_uv(&name, "mc_bgm_text_off", r, Some(id / 8));
                self.set_text_uv(&name, "mc_bgm_text_on", r, Some(id / 8));
            }
            let r = self.flash.find(None, "mc_bgm_plate_7");
            self.flash.clip_set_scissor(r, scissor);
            let id = self.bgm_id(self.bgm_extra);
            let r = uv(0, id % 8 * 0x20, 0x200, id % 8 * 0x20 + 0x20);
            self.set_text_uv("mc_bgm_plate_7", "mc_bgm_text_off", r, Some(id / 8));
            self.set_text_uv("mc_bgm_plate_7", "mc_bgm_text_on", r, Some(id / 8));
            if count > 0 {
                let y = (255.499_99 / count as f32 * self.bgm_top as f32) as i32;
                let scale = 6.0 / count as f32 * 7.3;
                let bar = self.flash.find(None, "mc_bgm_scroll_bar");
                self.flash.clip_set_scale(bar, 1.0, scale);
                self.flash.clip_set_offset(bar, 0, y);
            }
        }
        let mark = uv(0, 0, 0x40, 0x80);
        for (name, v) in [
            ("mc_vol_now_bgm", self.bgm_volume),
            ("mc_vol_now_se", self.se_volume),
        ] {
            let r = self.flash.find(None, name);
            self.flash.clip_set_uv(r, mark);
            self.flash.clip_set_offset(r, v * 0x23, 0);
        }
        let up = self
            .flash
            .find(Some("mc_yajirusi_up"), "mc_yajirusi_icon_up");
        self.flash
            .clip_set_flags(up, PROP_VISIBLE, self.bgm_top != 0);
        self.flash.clip_set_uv(up, uv(0, 0x20, 0x20, 0x40));
        let down = self
            .flash
            .find(Some("mc_yajirusi_down"), "mc_yajirusi_icon_down");
        self.flash
            .clip_set_flags(down, PROP_VISIBLE, self.bgm_bottom < count);
        self.flash.clip_set_uv(down, uv(0x20, 0x20, 0x40, 0x40));
        let right = self
            .flash
            .find(Some("mc_yajirusi_right"), "mc_yajirusi_icon_right");
        self.flash.clip_set_uv(right, uv(0x20, 0, 0x40, 0x20));
        let left = self.flash.find(None, "mc_yajirusi_left");
        if self.state == ITEM_VOLUME && self.value == 1 {
            self.flash.clip_set_offset(right, 0, 0x70);
            self.flash.clip_set_offset(left, 0, 0x70);
        } else if self.state == ST_KEYS_EDIT && self.key_held {
            let right = self.flash.find(None, "mc_yajirusi_right");
            for r in [right, left] {
                self.flash.clip_set_flags(r, PROP_VISIBLE, true);
                self.flash.clip_set_offset(r, self.mark.0, self.mark.1);
            }
        } else {
            self.flash.clip_set_offset(right, 0, 0);
            self.flash.clip_set_offset(left, 0, 0);
        }
        if self.state != ITEM_VOLUME && self.state != ITEM_TYPE && !self.key_held {
            let right = self.flash.find(None, "mc_yajirusi_right");
            self.flash.clip_set_flags(left, PROP_VISIBLE, false);
            self.flash.clip_set_flags(right, PROP_VISIBLE, false);
        }
    }
}

/// The pad words `Option_Input` reads.
#[derive(Clone, Copy, Debug)]
struct Input {
    held: u32,
    pressed: u32,
    repeat: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::{Libmc, McCard};

    /// Runs the real options screen from the ISO in `rom/`, when it is there, against an empty card in a
    /// temporary folder: the music volume down one, pad 1's vibration off, then leaving saves both.
    #[test]
    fn real_options_change_the_volume_and_vibration_and_save() {
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
        let assets = OptionAssets::parse(&res).unwrap();
        assert!(assets.data.bgm_ids.len() > 6, "{:?}", assets.data.bgm_ids);
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
        data.layout = OptionLayout::new(0, dialog, |key| firsts[&key]);
        let root = std::env::temp_dir().join(format!("bt3-options-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut card = McCard::new(Libmc::new(Some(root.clone())));
        let mut save = SaveData::default();
        let mut progress = crate::mcflow::Progress::default();
        let mut env = McEnv {
            card: &mut card,
            save: &mut save,
            progress: &mut progress,
            items: &[],
        };
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[3, 1, 4, 1]);
        let mut screen = OptionScreen::new(&data, env.save, None).unwrap();
        let mut step = |screen: &mut OptionScreen, env: &mut McEnv, bits: u32, held: u32| {
            screen.run_frame(
                held | bits,
                bits,
                VOICE_PLAYEND,
                &mut random,
                &mut font,
                env,
            );
        };
        for _ in 0..300 {
            step(&mut screen, &mut env, 0, 0);
            if screen.state == ST_TOP {
                break;
            }
        }
        assert_eq!(screen.state, ST_TOP);
        let mut press = |screen: &mut OptionScreen, env: &mut McEnv, bits: u32| {
            step(screen, env, bits, 0);
            for _ in 0..40 {
                step(screen, env, 0, 0);
            }
        };
        // The sound page, the volume window, the music one down (9 -> 8), kept.
        press(&mut screen, &mut env, PAD_DOWN);
        press(&mut screen, &mut env, PAD_DOWN);
        assert_eq!(screen.cursor, ITEM_SOUND);
        press(&mut screen, &mut env, PAD_CROSS);
        assert_eq!(
            (screen.state, screen.cursor, screen.page),
            (ITEM_SOUND, ITEM_STEREO, 2)
        );
        press(&mut screen, &mut env, PAD_DOWN);
        press(&mut screen, &mut env, PAD_CROSS);
        assert_eq!(screen.state, ITEM_VOLUME);
        press(&mut screen, &mut env, PAD_LEFT);
        press(&mut screen, &mut env, PAD_CROSS);
        press(&mut screen, &mut env, PAD_TRIANGLE);
        assert_eq!(env.save.volumes(), (8, 9));
        assert!(screen.dirty);
        press(&mut screen, &mut env, PAD_TRIANGLE);
        assert_eq!((screen.state, screen.cursor), (ST_TOP, ITEM_SOUND));
        // The controller page, vibration, pad 1 (the left one), off.
        press(&mut screen, &mut env, PAD_DOWN);
        press(&mut screen, &mut env, PAD_CROSS);
        assert_eq!(screen.cursor, ITEM_KEYS);
        press(&mut screen, &mut env, PAD_DOWN);
        press(&mut screen, &mut env, PAD_CROSS);
        assert_eq!(screen.state, ITEM_VIB);
        press(&mut screen, &mut env, PAD_DOWN);
        assert_eq!((screen.state, screen.value), (ST_VIB_EDIT, 1));
        press(&mut screen, &mut env, PAD_LEFT);
        press(&mut screen, &mut env, PAD_UP);
        assert_eq!(env.save.flags() & flag_vib(0), 0);
        press(&mut screen, &mut env, PAD_TRIANGLE);
        press(&mut screen, &mut env, PAD_TRIANGLE);
        assert_eq!((screen.state, screen.cursor), (ST_TOP, ITEM_CTRL));
        // Leaving: the cursor to exit, then the save flow, every question answered "yes".
        press(&mut screen, &mut env, PAD_TRIANGLE);
        assert_eq!(screen.cursor, ITEM_EXIT);
        step(&mut screen, &mut env, PAD_TRIANGLE, 0);
        assert!(screen.mc_busy);
        for frame in 0..4000 {
            let pressed = if !screen.mc_dialog.is_open() {
                0
            } else if screen.mc_dialog.cursor() != 0 {
                PAD_LEFT
            } else if frame % 4 == 0 {
                PAD_CROSS
            } else {
                0
            };
            step(&mut screen, &mut env, pressed, 0);
            if screen.is_done() {
                break;
            }
        }
        assert!(screen.is_done());
        assert!(!screen.dirty);
        let file = root.join("BASLUS-21678DBZT3").join("BASLUS-21678DBZT3");
        let written = SaveData::from_image(&std::fs::read(file).unwrap());
        assert_eq!(written.volumes(), (8, 9));
        assert_eq!(written.flags() & flag_vib(0), 0);
        assert_eq!(written.key(0, 0), 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
