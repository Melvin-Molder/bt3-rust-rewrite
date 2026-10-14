//! The Data Center's password screen (`DcPass_Run(3)`, src/menu/dc_password.c, progress mode 54): an on-screen
//! keyboard of two pages and 34 cells of text. A password of this game (34 characters) or of the previous one
//! (32, converted) whose character is on the select screen's grid shows the new character's status page; the
//! player then picks one of the save's 14 custom character slots to store it in (asking before overwriting one).

use std::error::Error;

use bt3_data::gs::RgbaTexture;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{FACE_FILE, PictureLoader};
use crate::dc_menu::DcProgress;
use crate::dialog::{Command, Dialog, DialogAssets, Pad};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::password::{
    ChrPass, ItemParam, OldPass, PassChk, item_bonus, item_params, section_bytes,
};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_START, PAD_TRIANGLE, PAD_UP, PadRepeat,
    TitleFrame, TitleSound,
};
use crate::windows::{MsgWin, TextBox, TextDraw, draw_texts};

/// `DcPass_Run(3)` unpacks section 3 of `gMenuArc8`.
pub const ARCHIVE_SECTION: usize = 3;
/// `DC_VOICE_BASE`.
const VOICE_BASE: u32 = 0x8398;
const FADE_FRAMES: i32 = 20;

/// `PADG_CIRCLE`, `PADG_L1` / `L2`, `PADG_R1` / `R2`.
pub const PAD_CIRCLE: u32 = 0x100;
pub const PAD_L1: u32 = 0x4000;
pub const PAD_L2: u32 = 0x8000;
pub const PAD_R1: u32 = 0x1_0000;
pub const PAD_R2: u32 = 0x2_0000;

// DCPASS_PAGES / ROWS / COLS, DCPASS_TEXT_LEN / LAST, DCPASS_LIST_ROWS, SAVE_REC_COUNT.
const PAGES: i32 = 2;
const ROWS: i32 = 5;
const COLS: i32 = 14;
const TEXT_LEN: usize = 34;
const TEXT_LAST: i32 = 33;
const LIST_ROWS: i32 = 3;
const REC_COUNT: i32 = 14;

// The keyboard's special cells (DCKEY_*), and what they do (DCPASS_KEY_*).
const BUTTON_L: u8 = 0;
const BUTTON_M: u8 = 1;
const BUTTON_R: u8 = 2;
const WIDE_E: u8 = 3;
const ZERO: u8 = 4;
const WIDE_D1: u8 = 5;
const WIDE_D2: u8 = 6;
const KEY_QUIT: u8 = 0;
const KEY_PAGE: u8 = 1;
const KEY_OK: u8 = 2;
const KEY_BACK: u8 = 3;
const KEY_LEFT: u8 = 5;
const KEY_RIGHT: u8 = 6;

// DCPASS_ST_*.
const ST_TYPE: i32 = 0;
const ST_FAILED: i32 = 1;
const ST_SHOWN: i32 = 2;
const ST_LIST: i32 = 3;
const ST_ASK_QUIT: i32 = 4;
const ST_ASK_OVER: i32 = 5;
const ST_STORED: i32 = 6;

// DCPASS_* flags.
const LEAVE: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const FACE_CHANGE: u32 = 8;
const FACE_LOADING: u32 = 0x10;
const FACE_READY: u32 = 0x20;

// The guide's lines.
const LINE_FAILED: i32 = 0x13;
const LINE_WHERE: i32 = 0x14;
const LINE_STORED: i32 = 0x15;

/// The slot list's window (`DcPass_ClipBegin`), `[x0, y0, x1, y1]`.
const LIST_SCISSOR: [i32; 4] = [0xAF, 0x6A, 0x1FF, 0x132];

const fn row(s: &[u8; 12], a: u8, b: u8) -> [u8; 14] {
    let mut r = [a; 14];
    let mut i = 0;
    while i < 12 {
        r[i] = s[i];
        i += 1;
    }
    r[13] = b;
    r
}

const fn lower(r: [u8; 14]) -> [u8; 14] {
    let mut o = r;
    let mut i = 0;
    while i < 14 {
        if o[i].is_ascii_uppercase() {
            o[i] += 0x20;
        }
        i += 1;
    }
    o
}

const ROW_A: [u8; 14] = *b"ABCDEFGHIJKLMN";
const ROW_O: [u8; 14] = row(b"OPQRSTUVWXYZ", WIDE_D1, WIDE_D2);
const ROW_SYM: [u8; 14] = row(b"#$%&@!?-+*()", WIDE_D1, WIDE_D2);
const ROW_DIGITS: [u8; 14] = [
    ZERO, ZERO, b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', WIDE_E, WIDE_E,
];
const ROW_BUTTONS: [u8; 14] = [
    BUTTON_L, BUTTON_L, BUTTON_L, BUTTON_L, BUTTON_M, BUTTON_M, BUTTON_M, BUTTON_M, BUTTON_M,
    BUTTON_M, BUTTON_R, BUTTON_R, BUTTON_R, BUTTON_R,
];

/// `gDcPassKeys[page][row][col]`: capitals, then small letters.
const KEYS: [[[u8; 14]; 5]; 2] = [
    [ROW_A, ROW_O, ROW_SYM, ROW_DIGITS, ROW_BUTTONS],
    [lower(ROW_A), lower(ROW_O), ROW_SYM, ROW_DIGITS, ROW_BUTTONS],
];

/// `DcKeyPos`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct KeyPos {
    page: i32,
    row: i32,
    col: i32,
}

/// `DcPass_GetKey`: the character or code of a cell, the wide "0" key giving '0'. The original indexes the table
/// as one array, so a column or row one past an edge (which the moves look at before wrapping) reads the
/// neighbouring row or page; outside the table it is `None` here, which never matches a key.
fn get_key(p: KeyPos) -> Option<u8> {
    let flat = p.page * ROWS * COLS + p.row * COLS + p.col;
    let flat = usize::try_from(flat).ok()?;
    let key = *KEYS.as_flattened().as_flattened().get(flat)?;
    Some(if key == ZERO { b'0' } else { key })
}

fn same_key(a: KeyPos, b: KeyPos) -> bool {
    matches!((get_key(a), get_key(b)), (Some(x), Some(y)) if x == y)
}

/// `DcPass_Wrap`: a ring of `count`.
fn wrap(value: i32, count: i32) -> i32 {
    if value < 0 {
        count - 1
    } else if value > count - 1 {
        0
    } else {
        value
    }
}

/// `DcPass_WrapPos`.
fn wrap_pos(p: &mut KeyPos) {
    p.page = wrap(p.page, PAGES);
    p.row = wrap(p.row, ROWS);
    p.col = wrap(p.col, COLS);
}

/// `DcPass_GotoButton`: the middle (1) or right (2) button of the bottom row.
fn goto_button(button: u8, p: &mut KeyPos) {
    match button {
        2 => (p.row, p.col) = (ROWS - 1, 12),
        1 => (p.row, p.col) = (ROWS - 1, 6),
        _ => {}
    }
}

/// `DcPass_GetClipPos`: the row and column of a key's clip ("mc_input_code_<row + 1>_<col + 1>").
fn clip_pos(p: KeyPos) -> (i32, i32) {
    let (mut row, mut col) = (p.row, p.col);
    match p.row {
        1 if p.col >= 12 => row += 1,
        3 => {
            col = (col - 2).max(0);
            if p.col >= 12 {
                col = 10;
            }
        }
        4 => col = get_key(p).map_or(0, i32::from),
        _ => {}
    }
    (row, col)
}

/// `DcPass_FindKey`: the cell that types a character (digits by their value on the first page's row 3); a
/// character that is on no key leaves the search's end position.
fn find_key(c: u8) -> KeyPos {
    if c.is_ascii_digit() {
        return KeyPos {
            page: 0,
            row: 3,
            col: i32::from(c - b'0'),
        };
    }
    for page in 0..PAGES {
        for row in 0..ROWS {
            for col in 0..COLS {
                let p = KeyPos { page, row, col };
                if get_key(p) == Some(c) {
                    return p;
                }
            }
        }
    }
    KeyPos {
        page: PAGES,
        row: ROWS,
        col: COLS,
    }
}

/// One move of the keyboard cursor over a wide key: `step` along the column or row while the cell keeps the key
/// of the one before, then the wrap.
fn move_key(p: &mut KeyPos, dcol: i32, drow: i32) {
    loop {
        let before = *p;
        p.col += dcol;
        p.row += drow;
        if !same_key(*p, before) {
            break;
        }
    }
}

/// `DcPassText`.
#[derive(Clone, Copy, Debug)]
struct PassText {
    text: [u8; TEXT_LEN],
    cursor: i32,
}

impl Default for PassText {
    /// `DcPassText_Clear`: 34 spaces.
    fn default() -> Self {
        Self {
            text: [b' '; TEXT_LEN],
            cursor: 0,
        }
    }
}

impl PassText {
    /// `DcPassText_Put`: the key's character at the cursor, which moves on (it stops on the last cell).
    fn put(&mut self, p: KeyPos) {
        self.text[self.cursor as usize] = get_key(p).unwrap_or(b' ');
        if self.cursor < TEXT_LAST {
            self.cursor += 1;
        }
    }

    /// `DcPassText_Delete`: clears the cell under the cursor, or steps back when it is empty; true when there was
    /// nothing left.
    fn delete(&mut self) -> bool {
        if self.cursor == 0 {
            if self.text[0] == b' ' {
                return true;
            }
            self.text[0] = b' ';
            self.cursor -= 1;
        } else if self.text[self.cursor as usize] == b' ' {
            self.cursor -= 1;
            self.text[self.cursor as usize] = b' ';
        } else {
            self.text[self.cursor as usize] = b' ';
            self.cursor -= 1;
        }
        self.cursor = self.cursor.max(0);
        false
    }

    /// `DcPassText_IsFull`.
    fn is_full(&self) -> bool {
        !self.text.contains(&b' ')
    }

    /// `DcPassText_Pack`: the text up to the end, `None` when a cell before the 33rd is empty or only one of the
    /// last two is; two empty last cells leave a 32-character password.
    fn pack(&self) -> Option<String> {
        if self.text[..0x20].contains(&b' ') {
            return None;
        }
        let mut pass = self.text.to_vec();
        if pass[0x20] == b' ' {
            if pass[0x21] == b' ' {
                pass.truncate(0x20);
            } else {
                return None;
            }
        }
        String::from_utf8(pass).ok()
    }
}

/// `ZStatus`: the status page of the decoded character.
#[derive(Clone, Copy, Debug, Default)]
struct Status {
    chara: i32,
    items: [u16; 8],
    level: u16,
    /// `ItemSet_GetBonus`: slots, then the four stat changes.
    val: [i32; 5],
    attr: i32,
}

/// `DcPassList`: the slot list.
#[derive(Clone, Copy, Debug, Default)]
struct PassList {
    chara: [i32; 14],
    top: i32,
    cursor: i32,
    /// The record the fourth plate shows while the list scrolls.
    extra: i32,
}

impl PassList {
    fn slot(&self) -> usize {
        (self.top + self.cursor) as usize
    }

    /// `DcPass_ClampTop`: false when it had to be corrected.
    fn clamp_top(&mut self) -> bool {
        if self.top < 0 {
            self.top = 0;
            return false;
        }
        if self.top > REC_COUNT - LIST_ROWS {
            self.top = REC_COUNT - LIST_ROWS;
            return false;
        }
        true
    }

    /// `DcPass_ClampCursor`: true when it ran off the three rows.
    fn clamp_cursor(&mut self) -> bool {
        if self.cursor < 0 {
            self.cursor = 0;
            return true;
        }
        if self.cursor > LIST_ROWS - 1 {
            self.cursor = LIST_ROWS - 1;
            return true;
        }
        false
    }
}

/// `pass->view.tex0[n]` (the keyboard movie) of `DcPass_Init`.
const FLASH0_TEXTURES: [ImageTexture; 30] = [
    tex(15, 0),
    tex(15, 1),
    tex(15, 2),
    tex(15, 3),
    tex(15, 4),
    tex(15, 5),
    tex(15, 6),
    tex(1, 0),
    tex(2, 0),
    tex(2, 1),
    tex(1, 1),
    tex(2, 2),
    tex(1, 2),
    tex(2, 3),
    tex(1, 3),
    tex(1, 4),
    tex(1, 5),
    tex(2, 4),
    tex(1, 6),
    tex(1, 7),
    tex(1, 8),
    tex(2, 5),
    tex(1, 9),
    tex(2, 6),
    tex(16, 0),
    tex(16, 1),
    tex(16, 2),
    tex(11, 0),
    tex(11, 1),
    tex(11, 3),
];
/// `pass->view.tex1[n]` (the new character's movie); image 7 is the character's picture.
const FLASH1_TEXTURES: [ImageTexture; 37] = [
    tex(15, 0),
    tex(15, 1),
    tex(15, 2),
    tex(15, 3),
    tex(15, 4),
    tex(15, 5),
    tex(15, 6),
    None,
    tex(3, 0),
    tex(3, 1),
    tex(3, 2),
    tex(3, 3),
    tex(4, 0),
    tex(5, 0),
    tex(4, 1),
    tex(3, 4),
    tex(3, 5),
    None,
    None,
    tex(3, 6),
    tex(3, 7),
    tex(4, 2),
    tex(3, 8),
    tex(3, 9),
    tex(3, 10),
    tex(3, 11),
    tex(3, 12),
    None,
    None,
    tex(3, 13),
    tex(16, 0),
    tex(16, 1),
    tex(16, 2),
    tex(4, 3),
    tex(11, 0),
    tex(11, 1),
    tex(11, 3),
];
const FACE_IMAGE: usize = 7;
/// `MsgWin_Init(MPACK_AT(res, 6), …)`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((6, 1), 0)), Some(((6, 1), 1)), None];

/// What `DcPass_Init` reads from its unpacked section.
pub struct DcPassAssets {
    pub data: DcPassData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct DcPassData {
    pub fods: [Vec<u8>; 2],
    pub msg_fod: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    /// Section 21: the dialog's questions; section 8: the guide's lines; section 9: her mouth keys.
    pub dialog_msgs: Vec<u8>,
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    /// Sections 13 / 14: character and form names.
    pub names: Vec<u8>,
    pub forms: Vec<u8>,
    /// Section 12: the item parameters; 19: the password pack; 20: the select screen's grid (its ids).
    pub items: Vec<ItemParam>,
    pub pass_pack: Vec<u8>,
    pub grid: Vec<i32>,
    pub layout: DcPassLayout,
}

#[derive(Clone, Debug, Default)]
pub struct DcPassLayout {
    pub background: usize,
    pub flash: [Vec<Option<usize>>; 2],
    pub msg: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
}

impl DcPassLayout {
    pub fn new(
        background: usize,
        dialog: Vec<Option<usize>>,
        first: impl Fn(FileKey) -> usize,
    ) -> Self {
        Self {
            background,
            flash: [
                image_indices(&FLASH0_TEXTURES, &first),
                image_indices(&FLASH1_TEXTURES, &first),
            ],
            msg: image_indices(&MSG_TEXTURES, &first),
            dialog,
        }
    }
}

/// `DcGridCell` ids of `character_select_order` (a count, then 0x24-byte cells from 0x10).
fn grid_ids(b: &[u8]) -> Vec<i32> {
    let word = |at: usize| {
        b.get(at..at + 4)
            .map(|w| i32::from_le_bytes(w.try_into().unwrap()))
    };
    let count = word(0).unwrap_or(0).max(0) as usize;
    (0..count).map_while(|i| word(0x10 + i * 0x24)).collect()
}

impl DcPassAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&FLASH0_TEXTURES[..], &FLASH1_TEXTURES, &MSG_TEXTURES])?;
        let dialog = DialogAssets::parse(section_bytes(res, 7)?)?;
        let sec =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(section_bytes(res, n)?.to_vec()) };
        Ok(Self {
            data: DcPassData {
                fods: [sec(17)?, sec(18)?],
                msg_fod: bt3_data::pack_section(section_bytes(res, 6)?, 2)?.to_vec(),
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                dialog_msgs: sec(21)?,
                text: sec(8)?,
                lips: sec(9)?,
                names: sec(13)?,
                forms: sec(14)?,
                items: item_params(section_bytes(res, 12)?),
                pass_pack: sec(19)?,
                grid: grid_ids(section_bytes(res, 20)?),
                layout: DcPassLayout::default(),
            },
            background: decode_picture(res, 10)?,
            files,
            dialog,
        })
    }
}

/// `DcPass` and the frame loop of `DcPass_Run`.
pub struct DcPassScreen {
    layout: DcPassLayout,
    data: DcPassData,
    chk: PassChk,
    flash: [Flash; 2],
    tex: [Vec<Option<usize>>; 2],
    msg: MsgWin,
    dialog: Dialog,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    key: KeyPos,
    text: PassText,
    status: Status,
    list: PassList,
    /// The line each plate's text boxes last had: a box keeps drawing it until another is attached.
    rows: [Option<i32>; 4],
    state: i32,
    next_state: i32,
    flags: u32,
    movie: usize,
    voice_line: i32,
    blink: i32,
    talk: i32,
    scroll: f32,
    face_alpha: f32,
    done: bool,
}

impl DcPassScreen {
    /// `DcPass_Start`, `DcPass_Init` and the first `ColorFade_StartIn`. `chara_table` is common file 4.
    pub fn new(
        data: &DcPassData,
        chara_table: &[u8],
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = [Flash::create(&data.fods[0])?, Flash::create(&data.fods[1])?];
        for f in &mut flash {
            f.play(1);
        }
        let msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.text.clone(),
            false,
        )?;
        let dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            Some(data.dialog_msgs.clone()),
            0,
        )?;
        Ok(Self {
            layout: data.layout.clone(),
            chk: PassChk::new(&data.pass_pack, chara_table),
            data: data.clone(),
            flash,
            tex: data.layout.flash.clone(),
            msg,
            dialog,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            key: KeyPos::default(),
            text: PassText::default(),
            status: Status::default(),
            list: PassList::default(),
            rows: [None; 4],
            state: ST_TYPE,
            next_state: ST_TYPE,
            flags: 0,
            movie: 0,
            voice_line: 0,
            blink: random.range(0x20) as i32,
            talk: 0,
            scroll: 0.0,
            face_alpha: 0.0,
            done: false,
        })
    }

    /// Whether the screen is over (it always goes back to the Data Center menu).
    pub fn is_done(&self) -> bool {
        self.done
    }

    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        anim::play_with_subtitle(&self.data.lips, VOICE_BASE, line, &mut self.lip, sounds);
        self.voice_line = line;
    }

    /// `Voice_StopWithLip`.
    fn stop_voice(&mut self, sounds: &mut Vec<TitleSound>) {
        sounds.push(TitleSound::VoiceStop);
        self.lip.clear();
    }

    /// One frame of the `DcPass_Run` loop; `voice_stat` is `Voice_GetStat(0)`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        dc: &mut DcProgress,
        save: &mut SaveData,
        random: &mut GameRandom,
        loader: &mut dyn PictureLoader,
        font: &mut Font,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done {
            return frame;
        }
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        for f in &mut self.flash {
            f.advance();
        }
        self.draw(voice_stat, random, font, &mut frame);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.check_leave(&mut frame.sounds) {
            self.done = true;
            return frame;
        }
        self.input(pressed, repeat, voice_stat, dc, save, &mut frame.sounds);
        self.update_face(loader);
        frame
    }

    fn set_uv(&mut self, movie: usize, parent: Option<&str>, name: &str, uv: [i32; 4]) {
        let clip = self.flash[movie].find(parent, name);
        self.flash[movie].clip_set_uv(
            clip,
            Uv {
                u0: uv[0],
                v0: uv[1],
                u1: uv[2],
                v1: uv[3],
                tex: 0,
            },
        );
    }

    fn set_visible(&mut self, parent: &str, name: &str, on: bool) {
        let clip = self.flash[1].find(Some(parent), name);
        self.flash[1].clip_set_flags(clip, PROP_VISIBLE, on);
    }

    /// `DcPass_LightKey`: the position wrapped, its key lit or dimmed.
    fn light_key(&mut self, on: bool) {
        wrap_pos(&mut self.key);
        let (row, col) = clip_pos(self.key);
        let clip = self.flash[0].find(None, &format!("mc_input_code_{}_{:02}", row + 1, col + 1));
        self.flash[0].clip_goto_label(clip, if on { "fl_on_start" } else { "fl_off_start" });
    }

    /// `DcPass_FlashKey`.
    fn flash_key(&mut self) {
        let (row, col) = clip_pos(self.key);
        let clip = self.flash[0].find(None, &format!("mc_input_code_{}_{:02}", row + 1, col + 1));
        self.flash[0].clip_goto_label(clip, "fl_ok");
    }

    /// `DcPass_LightRow`.
    fn light_row(&mut self, on: bool) {
        let clip = self.flash[1].find(None, &format!("mc_menu_plate2_{}", self.list.cursor + 1));
        self.flash[1].clip_goto_label(clip, if on { "fl_on_start" } else { "fl_off_start" });
    }

    /// `DcPass_DrawKeys` and `DcPass_DrawButtons`: the caps of the four rows of keys (their kind taken from the
    /// key under the cursor, for every cell), the three buttons' captions and the keyboard's lettering.
    fn draw_keyboard(&mut self) {
        let key = get_key(self.key).unwrap_or(0);
        let shift = if key.is_ascii_lowercase() { 4 } else { 0 };
        let child = match key {
            WIDE_D1 | WIDE_D2 => "mc_text_input_code_d_on",
            WIDE_E => "mc_text_input_code_e_on",
            _ => "mc_text_input_code_a_on",
        };
        for row in 0..ROWS - 1 {
            for col in 0..COLS {
                let (r, c) = clip_pos(KeyPos {
                    page: self.key.page,
                    row,
                    col,
                });
                let name = format!("mc_input_code_{}_{:02}", r + 1, c + 1);
                let (u, v) = (c * 0x20, (r + shift) * 0x28);
                self.set_uv(0, Some(&name), child, [u, v, u + 0x20, v + 0x28]);
            }
        }
        let page = self.key.page;
        let buttons = [
            ("b", [0, 0x40, 0x100, 0x80]),
            (
                "c",
                if page == 0 {
                    [0, 0xC0, 0x100, 0x100]
                } else {
                    [0, 0x80, 0x100, 0xC0]
                },
            ),
            ("b", [0, 0, 0x100, 0x40]),
        ];
        for (i, (kind, uv)) in buttons.into_iter().enumerate() {
            let name = format!("mc_input_code_5_{:02}", i + 1);
            for state in ["on", "off"] {
                self.set_uv(
                    0,
                    Some(&name),
                    &format!("mc_text_input_code_{kind}_{state}"),
                    uv,
                );
            }
        }
        self.set_uv(
            0,
            None,
            "mc_text_keyboard",
            [0, page << 7, 0x200, (page << 7) + 0x80],
        );
    }

    /// `DcPass_DrawText` and `DcPass_DrawCursor`: every cell the cap of the key that types its character, and the
    /// cursor under the cell the next character goes to.
    fn draw_text(&mut self) {
        let mut text = self.text.text.to_vec();
        text.push(0);
        for (i, &c) in text.iter().enumerate() {
            let p = find_key(c);
            let row = if c.is_ascii_lowercase() {
                p.row + 4
            } else {
                p.row
            };
            let (u, v) = (p.col * 0x20, row * 0x28);
            self.set_uv(0, None, &pass_cell(i as i32), [u, v, u + 0x20, v + 0x28]);
        }
        let cell = self.flash[0].find(None, &pass_cell(self.text.cursor));
        let (x, y) = self.flash[0].clip_get_pos(cell).unwrap_or((0, 0));
        let cursor = self.flash[0].find(None, "mc_input_cursor");
        self.flash[0].clip_set_offset(cursor, 0, 0);
        let (cx, cy) = self.flash[0].clip_get_pos(cursor).unwrap_or((0, 0));
        self.flash[0].clip_set_offset(cursor, x - cx, y - cy + 10);
    }

    /// `DcPass_DrawStatus`: the slot marks beyond the character's slots hidden, the four bars' marks (minus for a
    /// negative value, plus for a positive one) and the attribute icon. The captions go through names that are
    /// not all in the movie ("mc_text_ability_&d", "..._base1_0"), as in the original.
    fn draw_status(&mut self) {
        let s = self.status;
        let mut i = s.val[0];
        while i < 7 {
            self.set_visible(
                "mc_status_ability_base2_1",
                &format!("mc_status_ability_plus_{}", i + 4),
                false,
            );
            i += 1;
        }
        let row = |i: i32| [0, i * 0x14, 0x80, i * 0x14 + 0x14];
        self.set_uv(
            1,
            Some("mc_status_ability_base2_1"),
            "mc_text_ability_&d",
            row(i),
        );
        for i in 0..4 {
            let value = s.val[i as usize + 1];
            let name = format!("mc_status_ability_base1_{}", i + 1);
            for j in 0..4 {
                self.set_visible(&name, &format!("mc_status_ability_minus_{}", j + 1), false);
                self.set_visible(&name, &format!("mc_status_ability_plus_{}", j + 1), false);
                if value < 0 && value < -j {
                    self.set_visible(&name, &format!("mc_status_ability_minus_{}", j + 1), true);
                } else if value > 0 && j < value {
                    self.set_visible(&name, &format!("mc_status_ability_plus_{}", j + 1), true);
                }
            }
            self.set_uv(
                1,
                Some(&format!("mc_status_ability_base1_{i}")),
                "mc_text_ability_1",
                row(i),
            );
        }
        let attr = self.flash[1].find(None, "mc_status_attribute");
        self.flash[1].clip_set_tex(attr, s.attr);
        self.set_uv(
            1,
            Some("mc_status_ability_base1_4"),
            "mc_text_ability_1",
            row(4),
        );
    }

    /// `DcPass_DrawList`: the plates' scissor, the arrows, the scroll bar and the four plates (the name and form
    /// of the character saved there, an empty one dimmed unless the cursor is on it).
    fn draw_list(&mut self, texts: &mut Vec<TextDraw>, font: &Font) {
        let f = &mut self.flash[1];
        for i in 0..4 {
            let plate = f.find(None, &format!("mc_menu_plate2_{}", i + 1));
            f.clip_set_scissor(plate, Some(LIST_SCISSOR));
        }
        self.set_uv(
            1,
            Some("mc_yajirusi_up"),
            "mc_yajirusi_icon_up",
            [0, 0x20, 0x20, 0x40],
        );
        self.set_uv(
            1,
            Some("mc_yajirusi_down"),
            "mc_yajirusi_icon_down",
            [0x20, 0x20, 0x40, 0x40],
        );
        let hide = if self.list.top == 0 {
            Some(("mc_yajirusi_up", "mc_yajirusi_icon_up"))
        } else if self.list.top == REC_COUNT - LIST_ROWS {
            Some(("mc_yajirusi_down", "mc_yajirusi_icon_down"))
        } else {
            None
        };
        if let Some((parent, name)) = hide {
            self.set_visible(parent, name, false);
        }
        let f = &mut self.flash[1];
        let knob = f.find(None, "mc_scroll_bar_point");
        f.clip_set_scale(knob, 1.0, 1.212_857);
        f.clip_set_offset(knob, 0, self.list.top * 0xB5 / REC_COUNT);
        let name_box = TextBox::preset(2).with_clip(0xAF, 0x1FF, 0x6A, 0x132);
        let form_box = TextBox::preset(4).with_clip(0xAF, 0x1FF, 0x6A, 0x132);
        for i in 0..4 {
            let name = format!("mc_menu_plate2_{}", i + 1);
            let chara = if i != 3 {
                self.list.chara[(i + self.list.top) as usize]
            } else {
                self.list
                    .chara
                    .get(self.list.extra as usize)
                    .copied()
                    .unwrap_or(-1)
            };
            let plate = f.find(None, &name);
            if chara == -1 {
                let alpha = if i != 3 && i == self.list.cursor {
                    1.0
                } else {
                    0.5
                };
                f.clip_set_alpha(plate, alpha);
            } else {
                self.rows[i as usize] = Some(chara);
                f.clip_set_alpha(plate, 1.0);
            }
            if let Some(line) = self.rows[i as usize] {
                let tag = 0x100 + i as u32 * 2;
                let clip = f.find(Some(&name), "mc_menu_text1_on");
                texts.extend(name_box.attach_line(f, clip, tag, &self.data.names, line, font));
                let clip = f.find(Some(&name), "mc_menu_text2_on");
                texts.extend(form_box.attach_line(f, clip, tag + 1, &self.data.forms, line, font));
            }
        }
    }

    /// `DcPass_DrawNames`: the new character's name and form on both pairs of plates.
    fn draw_names(&mut self, texts: &mut Vec<TextDraw>, font: &Font) {
        let chara = self.status.chara;
        let f = &mut self.flash[1];
        let boxes = [
            ("mc_name_text_l", 1, false),
            ("mc_form_text_l", 1, true),
            ("mc_name_text_r", 2, false),
            ("mc_form_text_r", 2, true),
        ];
        for (n, (clip_name, preset, form)) in boxes.into_iter().enumerate() {
            let clip = f.find(None, clip_name);
            let text = if form {
                &self.data.forms
            } else {
                &self.data.names
            };
            texts.extend(TextBox::preset(preset).attach_line(
                f,
                clip,
                0x200 + n as u32,
                text,
                chara,
                font,
            ));
        }
    }

    /// `DcPass_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        frame: &mut TitleFrame,
    ) {
        if self.flags & STARTED == 0 {
            self.light_key(true);
            self.light_row(true);
            self.flash[0].goto_label("fl_input_in", true);
            self.flags |= STARTED;
        }
        self.draw_keyboard();
        self.draw_text();
        let face = self.flash[1].find(None, "mc_single_chara_r");
        self.flash[1].clip_set_alpha(face, self.face_alpha);
        self.draw_status();
        let mut texts = Vec::new();
        self.draw_list(&mut texts, font);
        self.draw_names(&mut texts, font);
        // DcPass_AnimGuide on the movie shown, DcPass_AnimBg on both.
        let f = &mut self.flash[self.movie];
        let eye = f.find(None, "mc_guide_blma_eye");
        anim::blink(f, eye, &mut self.blink, 0, random);
        let mouth = f.find(None, "mc_guide_blma_mouth");
        if voice_stat != VOICE_PLAYEND {
            anim::talk(
                f,
                mouth,
                &mut self.talk,
                0,
                &mut self.lip,
                voice_stat,
                random,
            );
        } else {
            anim::show_next(f, mouth, 0);
        }
        let uv = Uv {
            u0: 0,
            v0: 0,
            u1: 0x40,
            v1: 0x40,
            tex: 0,
        };
        for f in &mut self.flash {
            let bg = f.find(None, "mc_compane_3");
            anim::scroll(f, bg, uv, &mut self.scroll, -0.152_380_94);
        }
        frame.quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let tex = &self.tex[self.movie];
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (quads, overs) = self.flash[self.movie].draw_with_overs(&has_texture);
        frame.quads.extend(screen_quads(&quads, tex));
        if self.movie == 1 {
            draw_texts(&texts, &overs, font);
        }
        font.flush(&mut frame.quads);
        self.msg.draw(self.voice_line, font, &mut frame.quads);
        self.dialog.draw(false, font, &mut frame.quads);
    }

    /// `DcPass_Decode`: a 32-character password of the previous game (converted) or a 34-character one of this
    /// game, valid and of a character on the select screen's grid, fills the status page.
    fn decode(&mut self, save: &SaveData) -> bool {
        let Some(text) = self.text.pack() else {
            return false;
        };
        let data = match text.len() {
            32 => {
                let Some(old) = OldPass::decode(&text) else {
                    return false;
                };
                if !self.chk.is_old_valid(&old) {
                    return false;
                }
                self.chk.convert_old(&old)
            }
            34 => {
                let Some(mut data) = ChrPass::decode(&text) else {
                    return false;
                };
                if !self.chk.is_valid(&mut data) {
                    return false;
                }
                data
            }
            _ => return false,
        };
        if !self.data.grid.contains(&data.chara) {
            return false;
        }
        self.status.chara = data.chara;
        self.status.items = data.items.map(|id| id as u16);
        self.status.level = data.extra_slots as u16;
        self.status.val = item_bonus(&self.status.items, &self.data.items);
        self.flags |= FACE_CHANGE;
        self.status.attr = i32::from((self.chk.chara_flags(data.chara) ^ 1) & 1);
        // DcPassList_Refresh.
        self.list.chara = save.rec_chara;
        true
    }

    /// `DcPass_StoreRec`: the new character into the slot under the cursor; the save changed.
    fn store(&mut self, save: &mut SaveData, dc: &mut DcProgress) {
        let slot = self.list.slot();
        self.list.chara[slot] = self.status.chara;
        save.store_rec(
            slot,
            self.status.chara,
            self.status.items,
            self.status.level,
        );
        dc.dirty = true;
    }

    /// `DcPass_UpdateFace`: the decoded character's picture, requested, read (a frame later here) and faded in.
    fn update_face(&mut self, loader: &mut dyn PictureLoader) {
        if self.flags & FACE_CHANGE != 0 {
            self.flags &= !FACE_CHANGE;
            self.flags |= FACE_LOADING;
            self.face_alpha = 0.0;
            self.flags &= !FACE_READY;
        } else if self.flags & FACE_LOADING != 0 {
            self.tex[1][FACE_IMAGE] = loader.portrait(FACE_FILE + self.status.chara as u32);
            self.flags &= !FACE_LOADING;
            self.flags |= FACE_READY;
        }
        if self.flags & FACE_READY != 0 && self.face_alpha < 1.0 {
            self.face_alpha += 0.05;
        }
    }

    /// `DcPass_Leave`.
    fn leave(&mut self) {
        self.flags |= LEAVE;
        self.flash[0].goto_label("fl_input_out", true);
    }

    /// `DcPass_Input`.
    #[allow(clippy::too_many_arguments)]
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        voice_stat: i32,
        dc: &mut DcProgress,
        save: &mut SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash[0].flags() & FLASH_PAD == 0 || self.flags & LEAVE != 0 {
            return;
        }
        // DcPass_SwitchMovie: trigger 0 of the movie shown hands over to the other.
        if self.flash[0].trig() & 1 != 0 {
            self.movie = 1;
            self.flash[1].goto_label("fl_new_character", true);
        } else if self.flash[1].trig() & 1 != 0 {
            self.movie = 0;
            self.flash[0].goto_label("fl_input_in", true);
        }
        let mut effects = Vec::new();
        let pad = Pad { pressed, repeat };
        let idle = voice_stat == VOICE_PLAYEND;
        match self.state {
            ST_TYPE => {
                if self.movie == 1 {
                    return;
                }
                if repeat & (PAD_L1 | PAD_L2) != 0 {
                    if self.text.cursor > 0 {
                        self.text.cursor -= 1;
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if repeat & (PAD_R1 | PAD_R2) != 0 {
                    if self.text.cursor < TEXT_LAST {
                        self.text.cursor += 1;
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if repeat & (PAD_LEFT | PAD_RIGHT | PAD_UP | PAD_DOWN) != 0 {
                    self.light_key(false);
                    let (dc_, dr) = if repeat & PAD_LEFT != 0 {
                        (-1, 0)
                    } else if repeat & PAD_RIGHT != 0 {
                        (1, 0)
                    } else if repeat & PAD_UP != 0 {
                        (0, -1)
                    } else {
                        (0, 1)
                    };
                    move_key(&mut self.key, dc_, dr);
                    self.light_key(true);
                    sounds.push(TitleSound::Effect(1, 0));
                } else if pressed & PAD_CIRCLE != 0 {
                    self.key.page += 1;
                    wrap_pos(&mut self.key);
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_START != 0 {
                    self.light_key(false);
                    goto_button(KEY_OK, &mut self.key);
                    self.light_key(true);
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_CROSS != 0 {
                    self.flash_key();
                    match get_key(self.key).unwrap_or(0) {
                        KEY_PAGE => {
                            self.key.page += 1;
                            wrap_pos(&mut self.key);
                        }
                        KEY_OK => {
                            if self.decode(save) {
                                self.state = ST_SHOWN;
                                self.flash[0].goto_label("fl_new_character", true);
                            } else {
                                self.say(LINE_FAILED, sounds);
                                self.state = ST_FAILED;
                                self.flash[0].goto_label("fl_failure_in", true);
                                self.msg.open();
                            }
                        }
                        KEY_BACK => {
                            if self.text.delete() {
                                self.leave();
                            }
                        }
                        KEY_QUIT => self.leave(),
                        KEY_LEFT => {
                            if self.text.cursor > 0 {
                                self.text.cursor -= 1;
                            }
                        }
                        KEY_RIGHT => {
                            if self.text.cursor < TEXT_LAST {
                                self.text.cursor += 1;
                            }
                        }
                        _ => {
                            let last = self.text.cursor == TEXT_LAST;
                            self.text.put(self.key);
                            if last && self.text.is_full() {
                                self.light_key(false);
                                goto_button(KEY_OK, &mut self.key);
                                self.light_key(true);
                            }
                        }
                    }
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    if self.text.delete() {
                        self.leave();
                    }
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            ST_FAILED => {
                if pressed & PAD_CROSS != 0 || idle {
                    self.state = ST_TYPE;
                    self.flash[0].goto_label("fl_failure_out", true);
                    self.msg.close();
                    if pressed & PAD_CROSS != 0 {
                        sounds.push(TitleSound::Effect(1, 1));
                        self.stop_voice(sounds);
                    }
                }
            }
            ST_SHOWN => {
                if pressed & PAD_CROSS != 0 && self.movie == 1 {
                    self.state = ST_LIST;
                    self.flash[1].goto_label("fl_chara_list_in", true);
                    self.say(LINE_WHERE, sounds);
                    self.msg.open();
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            ST_LIST => {
                let slot = self.list.slot() as i32;
                if repeat & PAD_UP != 0 && slot != 0 {
                    self.light_row(false);
                    self.list.cursor -= 1;
                    if self.list.clamp_cursor() {
                        self.list.top -= 1;
                        if self.list.clamp_top() {
                            self.flash[1].goto_label("fl_chara_list_down", true);
                            self.list.extra = self.list.top + 3;
                        }
                    }
                    self.light_row(true);
                    sounds.push(TitleSound::Effect(1, 0));
                } else if repeat & PAD_DOWN != 0 && slot != REC_COUNT - 1 {
                    self.light_row(false);
                    self.list.cursor += 1;
                    if self.list.clamp_cursor() {
                        self.list.top += 1;
                        if self.list.clamp_top() {
                            self.flash[1].goto_label("fl_chara_list_up", true);
                            self.list.extra = self.list.top - 1;
                        }
                    }
                    self.light_row(true);
                    sounds.push(TitleSound::Effect(1, 0));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.state = ST_ASK_QUIT;
                    self.dialog.set_msg(1);
                    self.dialog.start(Command::Open, &mut effects);
                    self.dialog.set_cursor(1);
                    self.dialog.set_choices(true);
                    self.stop_voice(sounds);
                } else if pressed & PAD_LEFT != 0 && self.list.top + 2 >= 3 {
                    self.list.top -= 3;
                    self.list.clamp_top();
                    self.flash[1].goto_label("fl_chara_list_down", true);
                    self.list.extra = self.list.top + 3;
                    sounds.push(TitleSound::Effect(1, 0));
                } else if pressed & PAD_RIGHT != 0 && self.list.top != REC_COUNT - LIST_ROWS {
                    self.list.top += 3;
                    self.list.clamp_top();
                    self.flash[1].goto_label("fl_chara_list_up", true);
                    self.list.extra = self.list.top - 1;
                    sounds.push(TitleSound::Effect(1, 0));
                }
                if pressed & PAD_CROSS != 0 {
                    // DcPass_ConfirmRow.
                    let clip = self.flash[1]
                        .find(None, &format!("mc_menu_plate2_{}", self.list.cursor + 1));
                    self.flash[1].clip_goto_label(clip, "fl_ok");
                    if self.list.chara[self.list.slot()] == -1 {
                        self.state = ST_STORED;
                        self.store(save, dc);
                        self.say(LINE_STORED, sounds);
                        sounds.push(TitleSound::Effect(1, 1));
                    } else {
                        self.state = ST_ASK_OVER;
                        self.dialog.set_msg(0);
                        self.dialog.start(Command::Open, &mut effects);
                        self.dialog.set_cursor(1);
                        self.dialog.set_choices(true);
                    }
                }
            }
            ST_ASK_QUIT | ST_ASK_OVER => {
                let asked = self.state;
                if self.dialog.is_closed() {
                    self.state = self.next_state;
                    if self.state == ST_TYPE {
                        self.flash[1].goto_label("fl_chara_list_out", true);
                        self.flash[1].goto_label("fl_input_in", true);
                        self.msg.close();
                    } else if self.state == ST_STORED {
                        self.say(LINE_STORED, sounds);
                    }
                }
                match self.dialog.input(false, pad, &mut effects) {
                    1 => {
                        if asked == ST_ASK_QUIT {
                            self.next_state = ST_TYPE;
                        } else {
                            self.next_state = ST_STORED;
                        }
                        self.dialog.start(Command::Close, &mut effects);
                        self.dialog.set_choices(false);
                        if asked == ST_ASK_OVER {
                            self.store(save, dc);
                        }
                    }
                    -2 => {
                        self.next_state = ST_LIST;
                        self.dialog.start(Command::Close, &mut effects);
                        self.dialog.set_choices(false);
                    }
                    _ => {}
                }
            }
            ST_STORED => {
                if pressed & PAD_CROSS != 0 || idle {
                    if self.flags & LEAVE == 0 {
                        self.msg.close();
                        self.flash[1].goto_label("fl_chara_list_out", true);
                        if pressed & PAD_CROSS != 0 {
                            sounds.push(TitleSound::Effect(1, 1));
                        }
                    }
                    self.flags |= LEAVE;
                }
            }
            _ => {}
        }
        sounds.extend(effects.into_iter().map(|(b, id)| TitleSound::Effect(b, id)));
    }

    /// `DcPass_CheckLeave`: once left, the fade out; true when it ended.
    fn check_leave(&mut self, sounds: &mut Vec<TitleSound>) -> bool {
        if self.flags & LEAVE == 0 {
            return false;
        }
        if self.flags & LEAVING == 0 {
            self.flags |= LEAVING;
            self.fade = ColorFade::start_out(true, FADE_FRAMES);
        }
        if self.fade.is_fading_out() {
            sounds.push(TitleSound::FadeOutStep);
            false
        } else {
            true
        }
    }
}

/// "mc_input_pass_<line>_<cell>" of text cell `i` (17 a line).
fn pass_cell(i: i32) -> String {
    if i < 17 {
        format!("mc_input_pass_1_{:02}", i + 1)
    } else {
        format!("mc_input_pass_2_{:02}", i - 16)
    }
}

/// Section `section` of the Data Center's archive and common file 4 from the ISO in `rom/`, when it is there.
#[cfg(test)]
pub(crate) fn real_dc_section(section: usize) -> Option<(Vec<u8>, Vec<u8>)> {
    use bt3_data::afs::{AfsArchive, partition_entry_for_id};
    use bt3_data::{Iso9660, bpe, pack_section};
    let iso = bt3_data::rom::for_tests();
    let mut image = Iso9660::open(&iso).ok()?;
    let mut read = |id: u32| {
        let location = partition_entry_for_id(id);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).ok()?;
        archive.read_entry(&mut image, location.index).ok()
    };
    let pack = read(crate::dc_menu::ARCHIVE_FILE_ID)?;
    let common = read(4)?;
    let res = bpe::decode(pack_section(&pack, section).ok()?).ok()?;
    Some((res, common))
}

/// A picture loader that hands out made-up texture indices.
#[cfg(test)]
pub(crate) struct FakePictures;

#[cfg(test)]
impl PictureLoader for FakePictures {
    fn portrait(&mut self, file_id: u32) -> Option<usize> {
        Some(file_id as usize)
    }

    fn picture(&mut self, _: u32) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::VOICE_PLAYING;
    use crate::rng::LibcRandom;

    /// The password screen from the ISO, its texture indices made up.
    fn real_data() -> Option<(DcPassData, Vec<u8>)> {
        let (res, common) = real_dc_section(ARCHIVE_SECTION)?;
        let assets = DcPassAssets::parse(&res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        let dialog = vec![None; assets.dialog.textures.len()];
        data.layout = DcPassLayout::new(0, dialog, |key| firsts[&key]);
        Some((data, common))
    }

    struct Run {
        dc: DcProgress,
        save: SaveData,
        random: GameRandom,
        font: Font,
    }

    impl Run {
        fn step(&mut self, s: &mut DcPassScreen, pressed: u32) {
            s.run_frame(
                pressed,
                pressed,
                VOICE_PLAYING,
                &mut self.dc,
                &mut self.save,
                &mut self.random,
                &mut FakePictures,
                &mut self.font,
            );
        }

        /// Frames without input until the keyboard movie takes the pad.
        fn settle(&mut self, s: &mut DcPassScreen) {
            for _ in 0..400 {
                self.step(s, 0);
                if s.flash[0].flags() & FLASH_PAD != 0 && s.flash[s.movie].flags() & FLASH_PAD != 0
                {
                    return;
                }
            }
            panic!("the password screen never took the pad");
        }

        /// Types `text` straight into the cells, then START and confirm.
        fn enter(&mut self, s: &mut DcPassScreen, text: &str) {
            s.text = PassText::default();
            for (i, c) in text.bytes().enumerate() {
                s.text.text[i] = c;
            }
            self.step(s, PAD_START);
            self.step(s, 0);
            self.step(s, PAD_CROSS);
        }

        /// From the status page to the slot list, confirm on `slot`, answer a question "yes", and leave.
        fn store(&mut self, s: &mut DcPassScreen, slot: i32) {
            for _ in 0..600 {
                if s.state == ST_LIST {
                    break;
                }
                self.step(s, if s.movie == 1 { PAD_CROSS } else { 0 });
                self.step(s, 0);
            }
            assert_eq!(s.state, ST_LIST);
            self.settle(s);
            for _ in 0..slot {
                self.step(s, PAD_DOWN);
                self.step(s, 0);
            }
            assert_eq!(s.list.slot() as i32, slot);
            self.step(s, PAD_CROSS);
            for frame in 0..1200 {
                if s.is_done() {
                    return;
                }
                let pressed = if s.dialog.is_open() && s.dialog.cursor() != 0 {
                    PAD_LEFT
                } else if frame % 8 == 0 {
                    PAD_CROSS
                } else {
                    0
                };
                self.step(s, pressed);
            }
            panic!("the password screen never ended");
        }
    }

    /// From the ISO in `rom/`: a password of a grid character decodes to its status page and is stored in the
    /// chosen slot (an occupied one after asking); a typo is refused.
    #[test]
    fn real_passwords_store_custom_characters() {
        let Some((data, common)) = real_data() else {
            return;
        };
        // The screen's item table is common file 4's, and the grid lists the select screen's characters.
        let table = bt3_data::chara_data::ChrTable::parse(&common, 0xA1);
        assert_eq!(data.items.len(), 350);
        assert!(
            data.items
                .iter()
                .zip(&table.items)
                .all(|(a, b)| a.info == *b)
        );
        assert_eq!(data.grid.len(), 99);
        let chara = data.grid[3];
        // A slot item (type 1) shows on the status page; ids 1-based.
        let slot_item = data
            .items
            .iter()
            .position(|e| e.info.kind == 1 && e.info.slots == 1 && e.info.flags & 5 == 0)
            .unwrap() as i32
            + 1;
        let pass = ChrPass {
            chara,
            items: [slot_item, 0, 0, 0, 0, 0, 0, 0],
            extra_slots: 0,
            unk28: 0,
        };
        let chk = PassChk::new(&data.pass_pack, &common);
        assert!(chk.is_valid(&mut pass.clone()));
        let text = pass.encode(&mut LibcRandom::new(9));

        let mut run = Run {
            dc: DcProgress::default(),
            save: SaveData::default(),
            random: GameRandom::from_key(&[3]),
            font: Font::new(Vec::new(), 20),
        };
        let mut s = DcPassScreen::new(&data, &common, &mut run.random).unwrap();
        run.settle(&mut s);
        // A typo: the guide says so, and confirm goes back to typing.
        let mut typo = text.clone().into_bytes();
        typo[10] = if typo[10] == b'Q' { b'R' } else { b'Q' };
        run.enter(&mut s, std::str::from_utf8(&typo).unwrap());
        assert_eq!(s.state, ST_FAILED);
        run.settle(&mut s);
        run.step(&mut s, PAD_CROSS);
        assert_eq!(s.state, ST_TYPE);
        run.settle(&mut s);
        run.enter(&mut s, &text);
        assert_eq!(s.state, ST_SHOWN);
        assert_eq!(s.status.chara, chara);
        assert_eq!(s.status.val[0], 1);
        run.store(&mut s, 2);
        assert_eq!(run.save.rec_chara[2], chara);
        assert_eq!(run.save.rec_items[2][0], slot_item as u16);
        assert!(run.dc.dirty);
        // The face came in once the password was read.
        assert_eq!(
            s.tex[1][FACE_IMAGE],
            Some((FACE_FILE + chara as u32) as usize)
        );

        // Again into the same slot: "overwrite?" yes.
        run.dc.dirty = false;
        run.save.rec_chara[2] = 5;
        let mut s = DcPassScreen::new(&data, &common, &mut run.random).unwrap();
        run.settle(&mut s);
        run.enter(&mut s, &text);
        assert_eq!(s.state, ST_SHOWN);
        run.store(&mut s, 2);
        assert_eq!(run.save.rec_chara[2], chara);
        assert!(run.dc.dirty);

        // A password of the previous game: a character it may give, with an item it allows, becomes this game's
        // character with the preset items of its class.
        let old = (0..0xC8)
            .flat_map(|c| (1..0x15F).map(move |item| (c, item)))
            .map(|(c, item)| OldPass {
                chara: c,
                items: [item, 0, 0, 0, 0, 0, 0],
                ..OldPass::default()
            })
            .find(|o| chk.is_old_valid(o) && data.grid.contains(&chk.convert_old(o).chara))
            .unwrap();
        let converted = chk.convert_old(&old);
        let mut s = DcPassScreen::new(&data, &common, &mut run.random).unwrap();
        run.settle(&mut s);
        run.enter(&mut s, &old.encode(0xBEEF));
        assert_eq!(s.state, ST_SHOWN);
        assert_eq!(s.status.chara, converted.chara);
        assert_eq!(s.status.items, converted.items.map(|i| i as u16));
        run.store(&mut s, 13);
        assert_eq!(run.save.rec_chara[13], converted.chara);
    }

    #[test]
    fn the_cursor_crosses_wide_keys_and_wraps() {
        // Right along row 3: the three-cell "0" key is one step, then each digit, then "back" (two cells).
        let mut p = KeyPos {
            page: 0,
            row: 3,
            col: 0,
        };
        move_key(&mut p, 1, 0);
        assert_eq!(p.col, 3);
        p.col = 11;
        move_key(&mut p, 1, 0);
        assert_eq!(p.col, 12);
        move_key(&mut p, 1, 0);
        wrap_pos(&mut p);
        assert_eq!(p.col, 0);
        // Left from the "0" key wraps to the back key's last cell.
        p.col = 1;
        move_key(&mut p, -1, 0);
        wrap_pos(&mut p);
        assert_eq!(p.col, 13);
        // Up through the two-row "left" key, and down off the bottom row back to the top.
        let mut p = KeyPos {
            page: 0,
            row: 2,
            col: 12,
        };
        move_key(&mut p, 0, -1);
        assert_eq!(p.row, 0);
        let mut p = KeyPos {
            page: 1,
            row: 4,
            col: 2,
        };
        move_key(&mut p, 0, 1);
        wrap_pos(&mut p);
        assert_eq!(p.row, 0);
        assert_eq!(get_key(p), Some(b'c'));
    }

    #[test]
    fn keys_have_their_clips_and_codes() {
        let at = |page, row, col| KeyPos { page, row, col };
        assert_eq!(get_key(at(0, 3, 1)), Some(b'0'));
        assert_eq!(get_key(at(0, 4, 5)), Some(KEY_PAGE));
        assert_eq!(get_key(at(1, 4, 12)), Some(KEY_OK));
        assert_eq!(get_key(at(0, 3, 13)), Some(KEY_BACK));
        assert_eq!(get_key(at(0, 1, 12)), Some(KEY_LEFT));
        assert_eq!(clip_pos(at(0, 1, 13)), (2, 13));
        assert_eq!(clip_pos(at(0, 3, 0)), (3, 0));
        assert_eq!(clip_pos(at(0, 3, 7)), (3, 5));
        assert_eq!(clip_pos(at(0, 3, 12)), (3, 10));
        assert_eq!(clip_pos(at(0, 4, 11)), (4, 2));
        assert_eq!(find_key(b'7'), at(0, 3, 7));
        assert_eq!(find_key(b'q'), at(1, 1, 2));
        assert_eq!(find_key(b')'), at(0, 2, 11));
        assert_eq!(find_key(b' '), at(2, 5, 14));
    }

    #[test]
    fn the_text_types_deletes_and_packs() {
        let mut t = PassText::default();
        assert!(t.delete());
        let key = find_key(b'Q');
        for _ in 0..32 {
            t.put(key);
        }
        assert_eq!(t.cursor, 32);
        assert_eq!(t.pack().unwrap().len(), 32);
        t.put(key);
        // 33 cells: 34 characters with a space, which no decoder takes.
        assert!(t.pack().unwrap().ends_with(' '));
        // The 34th without the 33rd: not a password.
        let mut gap = t;
        gap.text[32] = b' ';
        gap.text[33] = b'Q';
        assert_eq!(gap.pack(), None);
        t.put(key);
        assert_eq!(t.cursor, TEXT_LAST);
        assert!(t.is_full());
        assert_eq!(t.pack().unwrap().len(), 34);
        // Backspace on the last (filled) cell clears it and steps back; on an empty one it steps back first.
        assert!(!t.delete());
        assert_eq!((t.cursor, t.text[33]), (32, b' '));
        t.cursor = 33;
        assert!(!t.delete());
        assert_eq!((t.cursor, t.text[32]), (32, b' '));
    }
}
