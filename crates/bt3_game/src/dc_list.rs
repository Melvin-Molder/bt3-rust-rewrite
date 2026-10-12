//! The Data Center's custom character list (`DcList_Run(2)`, src/menu/dc_list.c, progress mode 55): the save's
//! 14 custom characters on a scrolling list. Confirm on one opens its details (status page and picture) with a
//! menu of three: its items (each with the item details page, `ItemHelp`), its password (`PassWin`,
//! src/menu/password_window.c, made by `ChrPass_Encode`) and deleting it.

use std::error::Error;

use bt3_data::chara_data::ChrTable;
use bt3_data::gs::RgbaTexture;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{FACE_FILE, PictureLoader};
use crate::dc_menu::DcProgress;
use crate::dialog::{Command, Dialog, DialogAssets, Pad};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::item_panel::{ItemAssets, ItemHelp, help_textures_in};
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::password::{ChrPass, ItemParam, item_bonus, item_params, section_bytes};
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::windows::{MsgWin, TextBox, TextDraw, draw_texts};

/// `DcList_Run(2)` unpacks section 2 of `gMenuArc8`.
pub const ARCHIVE_SECTION: usize = 2;
/// `DC_VOICE_BASE`.
const VOICE_BASE: u32 = 0x8398;
const FADE_FRAMES: i32 = 0x14;
/// `SAVE_REC_COUNT`, `DC_ROWS`, `DC_TOP_MAX`, `DCLIST_ITEM_ROWS`, `SAVE_ITEM_COUNT`.
const REC_COUNT: i32 = 14;
const ROWS: i32 = 3;
const TOP_MAX: i32 = 11;
const ITEM_ROWS: i32 = 8;
const ITEM_COUNT: u16 = 350;

// DCLIST_ST_*.
const ST_LIST: i32 = 0;
const ST_MENU: i32 = 1;
const ST_ITEMS: i32 = 2;
const ST_ITEM_HELP: i32 = 3;
const ST_PASSWORD: i32 = 4;
const ST_DIALOG: i32 = 5;
const ST_DELETED: i32 = 6;

// DCLIST_* flags.
const FACE_CHANGE: u32 = 1;
const FACE_LOADING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;
const LEAVE: u32 = 0x10;
const LEAVING: u32 = 0x20;
const FACE_READY: u32 = 0x40;

// The guide's lines.
const LINE_LIST: i32 = 0x1A;
const LINE_DELETED: i32 = 0x1B;

/// `DcList_SetListScissor`, `[x0, y0, x1, y1]`.
const LIST_SCISSOR: [i32; 4] = [0xAF, 0x6A, 0x1FF, 0x139];

/// `list->view.tex0[n]` (the list movie) of `DcList_Init`; image 7 is the character's picture.
const FLASH0_TEXTURES: [ImageTexture; 38] = [
    tex(19, 0),
    tex(19, 1),
    tex(19, 2),
    tex(19, 3),
    tex(19, 4),
    tex(19, 5),
    tex(19, 6),
    None,
    tex(3, 0),
    tex(3, 1),
    tex(3, 2),
    tex(3, 3),
    tex(4, 0),
    tex(20, 0),
    tex(4, 1),
    tex(3, 4),
    tex(4, 2),
    tex(3, 5),
    tex(3, 6),
    tex(4, 3),
    tex(3, 7),
    tex(3, 8),
    None,
    None,
    tex(3, 9),
    tex(3, 10),
    tex(4, 4),
    tex(3, 11),
    tex(3, 12),
    tex(3, 13),
    None,
    None,
    tex(3, 14),
    tex(3, 15),
    tex(3, 16),
    tex(2, 0),
    tex(2, 1),
    tex(2, 3),
];
const FACE_IMAGE: usize = 7;
/// `list->view.tex1[n]` (the item page).
const FLASH1_TEXTURES: [ImageTexture; 17] = [
    tex(7, 0),
    tex(7, 1),
    tex(7, 2),
    tex(8, 0),
    tex(7, 3),
    tex(7, 4),
    tex(7, 5),
    tex(6, 0),
    tex(6, 0),
    None,
    tex(7, 6),
    tex(7, 7),
    tex(7, 8),
    tex(7, 9),
    tex(7, 10),
    None,
    None,
];
/// `MsgWin_Init(MPACK_AT(res, 11), …)`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((11, 1), 0)), Some(((11, 1), 1)), None];
/// `PassWin_Init(MPACK_AT(res, 10))`: `gPassWin->tex[n]`.
const PASS_TEXTURES: [ImageTexture; 4] = [
    Some(((10, 2), 0)),
    Some(((10, 2), 1)),
    Some(((10, 3), 0)),
    Some(((10, 2), 2)),
];
/// `ItemHelp_Init(MPACK_AT(res, 16))`.
const HELP_SECTION: usize = 16;

/// What `DcList_Init` reads from its unpacked section.
pub struct DcListAssets {
    pub data: DcListData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct DcListData {
    pub fods: [Vec<u8>; 2],
    pub msg_fod: Vec<u8>,
    pub pass_fod: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    /// Section 22: the dialog's question; 13: the guide's lines; 14: her mouth keys.
    pub dialog_msgs: Vec<u8>,
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    /// Sections 17 / 18 / 21: character, form and item names.
    pub names: Vec<u8>,
    pub forms: Vec<u8>,
    pub item_names: Vec<u8>,
    /// Section 15: the item parameters.
    pub items: Vec<ItemParam>,
    /// Section 16: the item details page.
    pub help: ItemAssets,
    pub layout: DcListLayout,
}

#[derive(Clone, Debug, Default)]
pub struct DcListLayout {
    pub background: usize,
    pub flash: [Vec<Option<usize>>; 2],
    pub msg: Vec<Option<usize>>,
    pub pass: Vec<Option<usize>>,
    pub help: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
}

impl DcListLayout {
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
            pass: image_indices(&PASS_TEXTURES, &first),
            help: image_indices(&help_textures_in(HELP_SECTION), &first),
            dialog,
        }
    }
}

impl DcListAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let help_textures = help_textures_in(HELP_SECTION);
        let files = decode_files(
            res,
            [
                &FLASH0_TEXTURES[..],
                &FLASH1_TEXTURES,
                &MSG_TEXTURES,
                &PASS_TEXTURES,
                &help_textures,
            ],
        )?;
        let dialog = DialogAssets::parse(section_bytes(res, 12)?)?;
        let sec =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(section_bytes(res, n)?.to_vec()) };
        let sub = |n: usize, k: usize| -> Result<Vec<u8>, Box<dyn Error>> {
            Ok(section_bytes(section_bytes(res, n)?, k)?.to_vec())
        };
        Ok(Self {
            data: DcListData {
                fods: [sec(5)?, sec(9)?],
                msg_fod: sub(11, 2)?,
                pass_fod: sub(10, 1)?,
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                dialog_msgs: sec(22)?,
                text: sec(13)?,
                lips: sec(14)?,
                names: sec(17)?,
                forms: sec(18)?,
                item_names: sec(21)?,
                items: item_params(section_bytes(res, 15)?),
                help: ItemAssets {
                    help_fod: sub(HELP_SECTION, 2)?,
                    help_name: sub(HELP_SECTION, 6)?,
                    help_desc: sub(HELP_SECTION, 5)?,
                    ..ItemAssets::default()
                },
                layout: DcListLayout::default(),
            },
            background: decode_picture(res, 1)?,
            files,
            dialog,
        })
    }
}

/// `ZSaveRec`, as the list copies it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Rec {
    items: [u16; 8],
    level: u16,
    chara: i32,
}

/// `PassWin`: the window that shows a character's password, one picture a character.
struct PassWin {
    flash: Flash,
    tex: Vec<Option<usize>>,
    /// `ChrPass_GetText`: the last password made.
    text: String,
}

/// `gPassKeys`: the window's sheet, four rows of capitals, symbols and digits (small letters four rows on).
const PASS_KEYS: [&[u8]; 4] = [
    b"ABCDEFGHIJKLMN",
    b"OPQRSTUVWXYZ",
    b"#$%&@!?-+*()",
    b"0123456789",
];

impl PassWin {
    /// `PassWin_OpenEx`: the password of the record (`ChrPass_Encode`) and the window's way in.
    fn open(&mut self, rec: &Rec, libc: &mut LibcRandom) {
        let pass = ChrPass {
            chara: rec.chara,
            items: rec.items.map(i32::from),
            extra_slots: i32::from(rec.level),
            unk28: 0,
        };
        self.text = pass.encode(libc);
        self.flash.goto_label("fl_password_in", true);
    }

    /// `PassWin_Close`.
    fn close(&mut self) {
        self.flash.goto_label("fl_password_out", true);
    }

    /// `PassWin_Draw`: the movie advances and each of the 34 cells gets its character's picture.
    fn draw(&mut self, quads: &mut Vec<ScreenQuad>) {
        self.flash.advance();
        let (mut col, mut row) = (0, 0);
        for (i, c) in self.text.bytes().take(34).enumerate() {
            // PassWin_FindKey: case is ignored; a character not on the sheet keeps the last cell.
            let up = c.to_ascii_uppercase();
            for (j, keys) in PASS_KEYS.iter().enumerate() {
                if let Some(k) = keys.iter().position(|&k| k == up) {
                    (col, row) = (k as i32, j as i32);
                    break;
                }
            }
            let r = if c.is_ascii_lowercase() { row + 4 } else { row };
            let name = if i <= 0x10 {
                format!("mc_input_pass_1_{:02}", i + 1)
            } else {
                format!("mc_input_pass_2_{:02}", i - 0x10)
            };
            let clip = self.flash.find(None, &name);
            self.flash.clip_set_uv(
                clip,
                Uv {
                    u0: col * 0x20,
                    v0: r * 0x28,
                    u1: col * 0x20 + 0x20,
                    v1: r * 0x28 + 0x28,
                    tex: 0,
                },
            );
        }
        let tex = &self.tex;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        quads.extend(screen_quads(&self.flash.draw(&has_texture), tex));
    }
}

/// The lines last attached to the text boxes: a box keeps drawing its line until another is attached.
#[derive(Clone, Copy, Debug)]
struct Boxes {
    rows: [[Option<i32>; 2]; 4],
    items: [Option<i32>; 8],
}

/// `DcList` and the frame loop of `DcList_Run`.
pub struct DcListScreen {
    layout: DcListLayout,
    data: DcListData,
    flash: [Flash; 2],
    tex: [Vec<Option<usize>>; 2],
    msg: MsgWin,
    dialog: Dialog,
    pass: PassWin,
    help: ItemHelp,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    libc: LibcRandom,
    /// `DcChars`.
    rec: [Rec; 14],
    top: i32,
    row: i32,
    extra: i32,
    /// `DcStatus`.
    status: Rec,
    bonus: [i32; 5],
    attr: i32,
    chara_flags: Vec<u16>,
    boxes: Boxes,
    state: i32,
    next_state: i32,
    menu_cursor: i32,
    item_cursor: i32,
    flags: u32,
    voice_line: i32,
    blink: i32,
    talk: i32,
    cloud_x: f32,
    face_alpha: f32,
    done: bool,
}

impl DcListScreen {
    /// `DcList_Reset`, `DcList_Init` and the first `ColorFade_StartIn`. `chara_table` is common file 4.
    pub fn new(
        data: &DcListData,
        save: &SaveData,
        chara_table: &[u8],
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = [Flash::create(&data.fods[0])?, Flash::create(&data.fods[1])?];
        for f in &mut flash {
            f.play(1);
        }
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
            Some(data.dialog_msgs.clone()),
            0,
        )?;
        let mut pass_flash = Flash::create(&data.pass_fod)?;
        pass_flash.play(1);
        let charas = bt3_data::pack_section(chara_table, 1).unwrap_or_default();
        Ok(Self {
            layout: data.layout.clone(),
            data: data.clone(),
            flash,
            tex: data.layout.flash.clone(),
            msg,
            dialog,
            pass: PassWin {
                flash: pass_flash,
                tex: data.layout.pass.clone(),
                text: String::new(),
            },
            help: ItemHelp::new(&data.help, data.layout.help.clone())?,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            libc: LibcRandom::new(1),
            rec: std::array::from_fn(|i| Rec {
                items: save.rec_items[i],
                level: save.rec_level[i],
                chara: save.rec_chara[i],
            }),
            top: 0,
            row: 0,
            extra: -1,
            status: Rec::default(),
            bonus: [0; 5],
            attr: 0,
            chara_flags: (0..0xA1)
                .map(|c| {
                    charas
                        .get(c * 0x3C + 8..c * 0x3C + 10)
                        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
                })
                .collect(),
            boxes: Boxes {
                rows: [[None; 2]; 4],
                items: [None; 8],
            },
            state: ST_LIST,
            next_state: ST_LIST,
            menu_cursor: 0,
            item_cursor: 0,
            flags: 0,
            voice_line: -1,
            blink: random.range(0x20) as i32,
            talk: 0,
            cloud_x: 0.0,
            face_alpha: 0.0,
            done: false,
        })
    }

    /// Whether the screen is over (it always goes back to the Data Center menu).
    pub fn is_done(&self) -> bool {
        self.done
    }

    fn cursor(&self) -> usize {
        (self.top + self.row) as usize
    }

    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        anim::play_with_subtitle(&self.data.lips, VOICE_BASE, line, &mut self.lip, sounds);
        self.voice_line = line;
    }

    /// One frame of the `DcList_Run` loop; `voice_stat` is `Voice_GetStat(0)`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        dc: &mut DcProgress,
        save: &mut SaveData,
        table: &ChrTable,
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
        self.draw(voice_stat, table, random, font, &mut frame);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.check_leave(&mut frame.sounds) {
            self.done = true;
            return frame;
        }
        if self.flash[0].flags() & FLASH_PAD != 0 && self.flags & LEAVE == 0 {
            let mut effects = Vec::new();
            match self.state {
                ST_LIST => self.input_list(pressed, repeat, &mut frame.sounds),
                ST_MENU => self.input_menu(pressed, repeat, &mut effects, &mut frame.sounds),
                ST_ITEMS => self.input_items(pressed, repeat, &mut frame.sounds),
                ST_ITEM_HELP => {
                    if self.cursor_item() != 0 && pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                        self.state = ST_ITEMS;
                        self.help.close();
                        let se = if pressed & PAD_CROSS != 0 { 1 } else { 2 };
                        frame.sounds.push(TitleSound::Effect(1, se));
                    }
                }
                ST_PASSWORD => {
                    if pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                        self.pass.close();
                        self.state = ST_MENU;
                        frame.sounds.push(TitleSound::Effect(1, 2));
                    }
                }
                ST_DIALOG => {
                    self.input_dialog(pressed, repeat, dc, save, &mut effects, &mut frame.sounds)
                }
                ST_DELETED => {
                    if pressed & PAD_CROSS != 0 || voice_stat == VOICE_PLAYEND {
                        self.state = ST_LIST;
                        if pressed & PAD_CROSS != 0 {
                            frame.sounds.push(TitleSound::Effect(1, 1));
                        }
                        self.say(LINE_LIST, &mut frame.sounds);
                    }
                }
                _ => {}
            }
            frame
                .sounds
                .extend(effects.into_iter().map(|(b, id)| TitleSound::Effect(b, id)));
        }
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

    fn show(&mut self, movie: usize, parent: Option<&str>, name: &str, on: bool) {
        let clip = self.flash[movie].find(parent, name);
        self.flash[movie].clip_set_flags(clip, PROP_VISIBLE, on);
    }

    fn goto(&mut self, movie: usize, name: &str, label: &str) {
        let clip = self.flash[movie].find(None, name);
        self.flash[movie].clip_goto_label(clip, label);
    }

    fn light(on: bool) -> &'static str {
        if on { "fl_on_start" } else { "fl_off_start" }
    }

    /// `DcView_LightRow`.
    fn light_row(&mut self, on: bool) {
        self.goto(
            0,
            &format!("mc_menu_plate2_{}", self.row + 1),
            Self::light(on),
        );
    }

    /// `DcView_LightMenu`.
    fn light_menu(&mut self, item: i32, on: bool) {
        self.goto(0, &format!("menu_plate_{}", item + 1), Self::light(on));
    }

    /// `DcList_LightItemRow`.
    fn light_item(&mut self, on: bool) {
        self.goto(
            1,
            &format!("mc_list_plate_{}", self.item_cursor + 1),
            Self::light(on),
        );
    }

    /// `DcList_GetCursorItem`.
    fn cursor_item(&self) -> u16 {
        self.status.items[self.item_cursor as usize]
    }

    /// `DcView_SetRows` and the rest of `DcList_DrawList`: the scissor, the arrows, the scroll bar and the four
    /// plates (name and form of each record, empty ones dimmed unless the cursor is on them).
    fn draw_list(&mut self, texts: &mut Vec<TextDraw>, font: &Font) {
        for i in 0..4 {
            let plate = self.flash[0].find(None, &format!("mc_menu_plate2_{}", i + 1));
            self.flash[0].clip_set_scissor(plate, Some(LIST_SCISSOR));
        }
        self.set_uv(
            0,
            Some("mc_yajirusi_up"),
            "mc_yajirusi_icon_up",
            [0, 0x20, 0x20, 0x40],
        );
        self.set_uv(
            0,
            Some("mc_yajirusi_down"),
            "mc_yajirusi_icon_down",
            [0x20, 0x20, 0x40, 0x40],
        );
        if self.top == 0 {
            self.show(0, Some("mc_yajirusi_up"), "mc_yajirusi_icon_up", false);
        } else if self.top == TOP_MAX {
            self.show(0, Some("mc_yajirusi_down"), "mc_yajirusi_icon_down", false);
        }
        let f = &mut self.flash[0];
        let knob = f.find(None, "mc_scroll_bar_point");
        f.clip_set_scale(knob, 1.0, 1.212_857_1);
        f.clip_set_offset(knob, 0, self.top * 0xB5 / REC_COUNT);
        for i in 0..4 {
            let name = format!("mc_menu_plate2_{}", i + 1);
            // rec[-1].chara is `extra` itself: -1.
            let index = if i != 3 { i + self.top } else { self.extra };
            let chara = usize::try_from(index)
                .ok()
                .and_then(|n| self.rec.get(n))
                .map_or(self.extra, |r| r.chara);
            let plate = f.find(None, &name);
            if chara == -1 {
                f.clip_set_alpha(plate, if i == self.row { 1.0 } else { 0.5 });
            } else {
                self.boxes.rows[i as usize] = [Some(chara); 2];
                f.clip_set_alpha(plate, 1.0);
            }
            let boxes = [
                (TextBox::preset(2), "mc_menu_text1_on", &self.data.names),
                (TextBox::preset(4), "mc_menu_text2_on", &self.data.forms),
            ];
            for (k, (b, child, text)) in boxes.into_iter().enumerate() {
                if let Some(line) = self.boxes.rows[i as usize][k] {
                    let clip = f.find(Some(&name), child);
                    let b = b.with_clip(0xAF, 0x1FF, 0x6A, 0x139);
                    texts.extend(b.attach_line(
                        f,
                        clip,
                        0x100 + i as u32 * 2 + k as u32,
                        text,
                        line,
                        font,
                    ));
                }
            }
        }
    }

    /// `DcView_SetStatus`: the slot marks beyond the slots used hidden, the four bars' marks and the attribute.
    fn draw_status(&mut self) {
        let slots = "mc_status_ability_base2_1";
        let mut i = self.bonus[0];
        while i < 7 {
            self.show(
                0,
                Some(slots),
                &format!("mc_status_ability_plus_{}", i + 4),
                false,
            );
            i += 1;
        }
        let row = |i: i32| [0, i * 0x14, 0x80, i * 0x14 + 0x14];
        self.set_uv(0, Some(slots), "mc_text_ability_1", row(i));
        for i in 0..4 {
            let val = self.bonus[1 + i as usize];
            let base = format!("mc_status_ability_base1_{}", i + 1);
            for k in 0..4 {
                let minus = format!("mc_status_ability_minus_{}", k + 1);
                let plus = format!("mc_status_ability_plus_{}", k + 1);
                self.show(0, Some(&base), &minus, false);
                self.show(0, Some(&base), &plus, false);
                if val < 0 && val < -k {
                    self.show(0, Some(&base), &minus, true);
                } else if val > 0 && k < val {
                    self.show(0, Some(&base), &plus, true);
                }
            }
            self.set_uv(0, Some(&base), "mc_text_ability_1", row(i + 1));
        }
        let attr = self.flash[0].find(None, "mc_status_attribute");
        self.flash[0].clip_set_tex(attr, self.attr);
    }

    /// `DcList_DrawItems`: the eight rows of the item page (name, cursor, cost and kind icons, plate pictures).
    fn draw_items(&mut self, texts: &mut Vec<TextDraw>, font: &Font) {
        for i in 0..ITEM_ROWS {
            let item = self.status.items[i as usize];
            let name = format!("mc_list_plate_{}", i + 1);
            let param = (item != 0)
                .then(|| self.data.items.get(usize::from(item) - 1))
                .flatten()
                .copied()
                .unwrap_or_default();
            if item != 0 {
                self.boxes.items[i as usize] = Some(i32::from(item) - 1);
            }
            if let Some(line) = self.boxes.items[i as usize] {
                let f = &mut self.flash[1];
                let clip = f.find(Some(&name), "mc_dammy_text");
                texts.extend(TextBox::preset(2).attach_line(
                    f,
                    clip,
                    0x400 + i as u32,
                    &self.data.item_names,
                    line,
                    font,
                ));
            }
            self.show(1, Some(&name), "mc_icon_custom", i == self.item_cursor);
            self.set_uv(1, Some(&name), "mc_icon_custom", [0x40, 0, 0x80, 0x40]);
            let (off, on) = if item != 0 {
                if param.info.slots != 0 {
                    let (u, v) = (
                        (i32::from(param.info.slots) - 1) * 0x20,
                        param.info.class() * 0x2A,
                    );
                    self.set_uv(
                        1,
                        Some(&name),
                        "mc_icon_item_cost",
                        [u, v, u + 0x20, v + 0x2A],
                    );
                    self.show(1, Some(&name), "mc_icon_item_cost", true);
                } else {
                    self.show(1, Some(&name), "mc_icon_item_cost", false);
                }
                let u = i32::from(param.info.kind) << 6;
                self.set_uv(
                    1,
                    Some(&name),
                    "mc_icon_item_potara",
                    [u, 0, u + 0x40, 0x40],
                );
                self.show(1, Some(&name), "mc_icon_item_potara", true);
                (1, 0)
            } else {
                self.show(1, Some(&name), "mc_icon_item_cost", false);
                self.show(1, Some(&name), "mc_icon_item_potara", false);
                (5, 3)
            };
            let (off, on) = if i == ITEM_ROWS - 1 {
                (4, 3)
            } else {
                (off, on)
            };
            let f = &mut self.flash[1];
            let clip = f.find(Some(&name), "mc_list_plate_off");
            f.clip_set_tex(clip, off);
            let clip = f.find(Some(&name), "mc_list_plate_on");
            f.clip_set_tex(clip, on);
        }
    }

    /// `DcList_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        table: &ChrTable,
        random: &mut GameRandom,
        font: &mut Font,
        frame: &mut TitleFrame,
    ) {
        if self.flags & STARTED == 0 {
            self.flash[0].goto_label("fl_chara_list_in", true);
            self.flags |= STARTED;
        }
        if self.flags & GREETED == 0 && self.flash[0].flags() & FLASH_PAD != 0 {
            self.light_row(true);
            self.light_menu(self.menu_cursor, true);
            self.light_item(true);
            self.say(LINE_LIST, &mut frame.sounds);
            self.flags |= GREETED;
        }
        let mut texts0 = Vec::new();
        self.draw_list(&mut texts0, font);
        let face = self.flash[0].find(None, "mc_single_chara_r");
        self.flash[0].clip_set_alpha(face, self.face_alpha);
        // DcView_HideNamePlates, DcView_SetMenuText.
        self.show(0, Some("mc_name_plate_l"), "ch_name_plate", false);
        self.show(0, Some("mc_form_plate_l"), "ch_form_plate", false);
        for i in 0..3 {
            let name = format!("menu_plate_{}", i + 1);
            let uv = [0, i * 0x20, 0x100, i * 0x20 + 0x20];
            self.set_uv(0, Some(&name), "mc_menu_text_on", uv);
            self.set_uv(0, Some(&name), "mc_menu_text_off", uv);
        }
        self.draw_status();
        // DcView_SetNameText.
        let chara = self.status.chara;
        let f = &mut self.flash[0];
        let clip = f.find(None, "mc_name_text_l");
        texts0.extend(TextBox::preset(1).attach_line(
            f,
            clip,
            0x200,
            &self.data.names,
            chara,
            font,
        ));
        let clip = f.find(None, "mc_form_text_l");
        texts0.extend(TextBox::preset(3).attach_line(
            f,
            clip,
            0x201,
            &self.data.forms,
            chara,
            font,
        ));
        // DcList_DrawItemPage: the names, the rows, the two limit numbers.
        let mut texts1 = Vec::new();
        let f = &mut self.flash[1];
        let clip = f.find(None, "mc_name_text_l");
        texts1.extend(TextBox::preset(1).attach_line(
            f,
            clip,
            0x300,
            &self.data.names,
            chara,
            font,
        ));
        self.set_uv(1, None, "mc_name_plate_l", [0, 0, 0x100, 0x20]);
        let f = &mut self.flash[1];
        let clip = f.find(None, "mc_form_text_l");
        texts1.extend(TextBox::preset(3).attach_line(
            f,
            clip,
            0x301,
            &self.data.forms,
            chara,
            font,
        ));
        self.draw_items(&mut texts1, font);
        self.set_uv(
            1,
            None,
            "mc_ability_limit_big_num",
            [0x80, 0x80, 0xC0, 0xC0],
        );
        self.set_uv(1, None, "mc_ability_limit_s_num", [0x40, 0x40, 0x60, 0x60]);
        // DcList_UpdateGuide, DcList_ScrollCloud.
        let f = &mut self.flash[0];
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
        let bg = f.find(None, "mc_compane_3");
        let uv = Uv {
            u0: 0,
            v0: 0,
            u1: 0x40,
            v1: 0x40,
            tex: 0,
        };
        anim::scroll(f, bg, uv, &mut self.cloud_x, -0.152_380_95);
        frame.quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        for (movie, texts) in [(0, texts0), (1, texts1)] {
            let tex = &self.tex[movie];
            let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
            let (quads, overs) = self.flash[movie].draw_with_overs(&has_texture);
            frame.quads.extend(screen_quads(&quads, tex));
            draw_texts(&texts, &overs, font);
            font.flush(&mut frame.quads);
            if movie == 0 {
                self.msg.draw(self.voice_line, font, &mut frame.quads);
            }
        }
        self.dialog.draw(false, font, &mut frame.quads);
        self.pass.draw(&mut frame.quads);
        let item = (i32::from(self.cursor_item()) - 1).max(0) as usize;
        self.help.draw(item, table, font, &mut frame.quads);
    }

    /// `DcList_PickRec`: the record under the cursor for the details panel.
    fn pick_rec(&mut self) {
        self.status = self.rec[self.cursor()];
        self.bonus = item_bonus(&self.status.items, &self.data.items);
        let flags = usize::try_from(self.status.chara)
            .ok()
            .and_then(|c| self.chara_flags.get(c))
            .copied()
            .unwrap_or(0);
        self.attr = i32::from((flags ^ 1) & 1);
    }

    /// `DcList_InputList`: up / down one record, left / right three, confirm opens the details, cancel leaves.
    fn input_list(&mut self, pressed: u32, repeat: u32, sounds: &mut Vec<TitleSound>) {
        let cursor = self.cursor() as i32;
        if repeat & PAD_UP != 0 && cursor != 0 {
            self.light_row(false);
            self.row -= 1;
            if self.clamp_row() {
                self.top -= 1;
                if self.clamp_top() {
                    self.flash[0].goto_label("fl_chara_list_down", true);
                    self.extra = self.top + 3;
                }
            }
            self.light_row(true);
            sounds.push(TitleSound::Effect(1, 0));
            return;
        }
        if repeat & PAD_DOWN != 0 && cursor != REC_COUNT - 1 {
            self.light_row(false);
            self.row += 1;
            if self.clamp_row() {
                self.top += 1;
                if self.clamp_top() {
                    self.flash[0].goto_label("fl_chara_list_up", true);
                    self.extra = self.top - 1;
                }
            }
            self.light_row(true);
            sounds.push(TitleSound::Effect(1, 0));
            return;
        }
        if pressed & PAD_CROSS != 0 {
            let row_ok = format!("mc_menu_plate2_{}", self.row + 1);
            if self.rec[self.cursor()].chara >= 0 {
                self.goto(0, &row_ok, "fl_ok");
                self.flash[0].goto_label("fl_chara_details_in", true);
                self.msg.close();
                self.state = ST_MENU;
                self.light_menu(self.menu_cursor, false);
                self.menu_cursor = 0;
                self.light_menu(0, true);
                self.pick_rec();
                self.flags |= FACE_CHANGE;
                sounds.push(TitleSound::Effect(1, 1));
            } else {
                sounds.push(TitleSound::Effect(1, 7));
            }
            self.goto(0, &row_ok, "fl_ok");
            return;
        }
        if pressed & PAD_TRIANGLE != 0 {
            self.flags |= LEAVE;
            self.flash[0].goto_label("fl_chara_list_cancel", true);
            self.msg.close();
            sounds.push(TitleSound::Effect(1, 2));
            return;
        }
        if pressed & PAD_LEFT != 0 && self.top + 2 >= 3 {
            self.top -= 3;
            self.clamp_top();
            self.flash[0].goto_label("fl_chara_list_down", true);
            self.extra = self.top + 3;
            sounds.push(TitleSound::Effect(1, 0));
            return;
        }
        if pressed & PAD_RIGHT != 0 && self.top != TOP_MAX {
            self.top += 3;
            self.clamp_top();
            self.flash[0].goto_label("fl_chara_list_up", true);
            self.extra = self.top - 1;
            sounds.push(TitleSound::Effect(1, 0));
        }
    }

    /// `DcChars_ClampTop`: false when it had to be corrected.
    fn clamp_top(&mut self) -> bool {
        let clamped = self.top.clamp(0, TOP_MAX);
        let ok = clamped == self.top;
        self.top = clamped;
        ok
    }

    /// `DcChars_ClampRow`: true when the list must scroll.
    fn clamp_row(&mut self) -> bool {
        let clamped = self.row.clamp(0, ROWS - 1);
        let ran_off = clamped != self.row;
        self.row = clamped;
        ran_off
    }

    /// `DcList_InputMenu`: 0 the item page, 1 the password, 2 "delete?".
    fn input_menu(
        &mut self,
        pressed: u32,
        repeat: u32,
        effects: &mut Vec<(u32, u32)>,
        sounds: &mut Vec<TitleSound>,
    ) {
        if pressed & PAD_CROSS != 0 {
            self.goto(0, &format!("menu_plate_{}", self.menu_cursor + 1), "fl_ok");
            match self.menu_cursor {
                0 => {
                    self.state = ST_ITEMS;
                    self.flash[1].goto_label("fl_evo_in", true);
                    self.light_item(false);
                    self.item_cursor = 0;
                    while slot_skipped(&self.status.items, self.item_cursor) {
                        self.item_cursor += 1;
                    }
                    self.light_item(true);
                }
                1 => {
                    let rec = self.rec[self.cursor()];
                    self.pass.open(&rec, &mut self.libc);
                    self.state = ST_PASSWORD;
                }
                _ => {
                    self.state = ST_DIALOG;
                    self.dialog.set_msg(2);
                    self.dialog.start(Command::Open, effects);
                    self.dialog.set_cursor(1);
                    self.dialog.set_choices(true);
                }
            }
            sounds.push(TitleSound::Effect(1, 1));
        } else if pressed & PAD_TRIANGLE != 0 {
            self.flash[0].goto_label("fl_chara_details_out", true);
            self.msg.open();
            self.state = ST_LIST;
            sounds.push(TitleSound::Effect(1, 2));
        } else if repeat & (PAD_UP | PAD_DOWN) != 0 {
            self.light_menu(self.menu_cursor, false);
            self.menu_cursor += if repeat & PAD_UP != 0 { -1 } else { 1 };
            self.menu_cursor = match self.menu_cursor {
                c if c < 0 => 2,
                c if c > 2 => 0,
                c => c,
            };
            self.light_menu(self.menu_cursor, true);
            sounds.push(TitleSound::Effect(1, 0));
        }
    }

    /// `DcList_InputItems`: up / down over the rows that hold an item, confirm the item details page.
    fn input_items(&mut self, pressed: u32, repeat: u32, sounds: &mut Vec<TitleSound>) {
        if pressed & PAD_CROSS != 0 {
            self.goto(
                1,
                &format!("mc_list_plate_{}", self.item_cursor + 1),
                "fl_ok",
            );
            if self.cursor_item() != 0 {
                self.help.open();
                self.state = ST_ITEM_HELP;
                sounds.push(TitleSound::Effect(1, 1));
            } else {
                sounds.push(TitleSound::Effect(1, 7));
            }
        } else if pressed & PAD_TRIANGLE != 0 {
            self.state = ST_MENU;
            self.flash[1].goto_label("fl_evo_cansel", true);
            self.light_item(false);
            self.item_cursor = 0;
            self.light_item(true);
            sounds.push(TitleSound::Effect(1, 2));
        } else if repeat & (PAD_UP | PAD_DOWN) != 0 {
            let step = if repeat & PAD_UP != 0 { -1 } else { 1 };
            self.light_item(false);
            loop {
                self.item_cursor = (self.item_cursor + step).rem_euclid(ITEM_ROWS);
                if !slot_skipped(&self.status.items, self.item_cursor) {
                    break;
                }
            }
            self.light_item(true);
            sounds.push(TitleSound::Effect(1, 0));
        }
    }

    /// `DcList_InputDialog`: yes deletes the record; the dialog closes into the next state.
    fn input_dialog(
        &mut self,
        pressed: u32,
        repeat: u32,
        dc: &mut DcProgress,
        save: &mut SaveData,
        effects: &mut Vec<(u32, u32)>,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.dialog.is_closed() {
            self.state = self.next_state;
            if self.state == ST_DELETED {
                self.flash[0].goto_label("fl_chara_details_out", true);
                self.say(LINE_DELETED, sounds);
                self.msg.open();
            }
        }
        match self.dialog.input(false, Pad { pressed, repeat }, effects) {
            1 => {
                // DcList_DeleteRec: only the character goes; the save changed.
                let idx = self.cursor();
                self.rec[idx].chara = -1;
                save.rec_chara[idx] = -1;
                dc.dirty = true;
                self.next_state = ST_DELETED;
                self.dialog.start(Command::Close, effects);
                self.dialog.set_choices(false);
            }
            -2 => {
                self.next_state = ST_MENU;
                self.dialog.start(Command::Close, effects);
                self.dialog.set_choices(false);
            }
            _ => {}
        }
    }

    /// `DcList_UpdateFace`: the picture of the character under the cursor, requested, read (a frame later here)
    /// and faded in.
    fn update_face(&mut self, loader: &mut dyn PictureLoader) {
        if self.flags & FACE_CHANGE != 0 {
            self.status = self.rec[self.cursor()];
            self.face_alpha = 0.0;
            self.flags = (self.flags & !(FACE_CHANGE | FACE_READY)) | FACE_LOADING;
        } else if self.flags & FACE_LOADING != 0 {
            self.tex[0][FACE_IMAGE] = loader.portrait(FACE_FILE + self.status.chara as u32);
            self.flags = (self.flags & !FACE_LOADING) | FACE_READY;
        }
        if self.flags & FACE_READY != 0 && self.face_alpha < 1.0 {
            self.face_alpha += 0.05;
        }
    }

    /// `DcList_CheckLeave`.
    fn check_leave(&mut self, sounds: &mut Vec<TitleSound>) -> bool {
        if self.flags & LEAVE == 0 {
            return false;
        }
        if self.flags & LEAVING == 0 {
            self.fade = ColorFade::start_out(true, FADE_FRAMES);
            self.flags |= LEAVING;
        }
        if self.fade.is_fading_out() {
            sounds.push(TitleSound::FadeOutStep);
            false
        } else {
            true
        }
    }
}

/// `DcRec_IsSlotEmpty` / `DcList_IsRowSkipped`: the row holds no item id, and it is not the last.
fn slot_skipped(items: &[u16; 8], row: i32) -> bool {
    items[row as usize].wrapping_sub(1) >= ITEM_COUNT && row != ITEM_ROWS - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::VOICE_PLAYING;
    use crate::dc_password::{FakePictures, real_dc_section};

    struct Run {
        dc: DcProgress,
        save: SaveData,
        table: ChrTable,
        random: GameRandom,
        font: Font,
    }

    impl Run {
        fn step(&mut self, s: &mut DcListScreen, pressed: u32) {
            s.run_frame(
                pressed,
                pressed,
                VOICE_PLAYING,
                &mut self.dc,
                &mut self.save,
                &self.table,
                &mut self.random,
                &mut FakePictures,
                &mut self.font,
            );
        }

        /// A button, then frames without input until the list movie takes the pad again.
        fn press(&mut self, s: &mut DcListScreen, pressed: u32) {
            self.step(s, pressed);
            for _ in 0..400 {
                self.step(s, 0);
                if s.flash[0].flags() & FLASH_PAD != 0 {
                    return;
                }
            }
            panic!("the list never took the pad");
        }
    }

    /// From the ISO in `rom/`: a saved character's details, item page, password and deletion.
    #[test]
    fn real_list_shows_and_deletes_custom_characters() {
        let Some((res, common)) = real_dc_section(ARCHIVE_SECTION) else {
            return;
        };
        let assets = DcListAssets::parse(&res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = DcListLayout::new(0, vec![None; assets.dialog.textures.len()], |key| {
            firsts[&key]
        });
        assert_eq!(data.items.len(), 350);
        let slot_item = data
            .items
            .iter()
            .position(|e| e.info.kind == 1 && e.info.slots == 1)
            .unwrap() as u16
            + 1;
        let mut save = SaveData::default();
        // Record 2: a character with two items in rows 0 and 3 and two extra slots.
        save.store_rec(2, 12, [slot_item, 0, 0, 5, 0, 0, 0, 0], 2);
        let mut run = Run {
            dc: DcProgress::default(),
            save,
            table: ChrTable::parse(&common, 0xA1),
            random: GameRandom::from_key(&[4]),
            font: Font::new(Vec::new(), 20),
        };
        let mut s = DcListScreen::new(&data, &run.save, &common, &mut run.random).unwrap();
        run.press(&mut s, 0);
        // An empty record cannot be opened.
        run.press(&mut s, PAD_CROSS);
        assert_eq!(s.state, ST_LIST);
        run.press(&mut s, PAD_DOWN);
        run.press(&mut s, PAD_DOWN);
        run.press(&mut s, PAD_CROSS);
        assert_eq!(s.state, ST_MENU);
        assert_eq!(s.status.chara, 12);
        assert_eq!(s.bonus[0], 1);
        run.press(&mut s, 0);
        assert_eq!(s.tex[0][FACE_IMAGE], Some((FACE_FILE + 12) as usize));
        // The item page: the cursor skips the empty rows.
        run.press(&mut s, PAD_CROSS);
        assert_eq!((s.state, s.item_cursor), (ST_ITEMS, 0));
        run.press(&mut s, PAD_DOWN);
        assert_eq!(s.item_cursor, 3);
        run.press(&mut s, PAD_DOWN);
        assert_eq!(s.item_cursor, 7);
        run.press(&mut s, PAD_UP);
        run.press(&mut s, PAD_CROSS);
        assert_eq!(s.state, ST_ITEM_HELP);
        run.press(&mut s, PAD_TRIANGLE);
        run.press(&mut s, PAD_TRIANGLE);
        assert_eq!(s.state, ST_MENU);
        // The password: this game's 34 characters, which give the record back.
        run.press(&mut s, PAD_DOWN);
        run.press(&mut s, PAD_CROSS);
        assert_eq!(s.state, ST_PASSWORD);
        let back = ChrPass::decode(&s.pass.text).unwrap();
        assert_eq!(back.chara, 12);
        assert_eq!(back.items[..4], [i32::from(slot_item), 0, 0, 5]);
        assert_eq!(back.extra_slots, 2);
        run.press(&mut s, PAD_CROSS);
        assert_eq!(s.state, ST_MENU);
        // Delete: "yes", the guide's line, then back to the list.
        run.press(&mut s, PAD_DOWN);
        run.press(&mut s, PAD_CROSS);
        assert_eq!(s.state, ST_DIALOG);
        for _ in 0..200 {
            if s.dialog.is_open() {
                break;
            }
            run.step(&mut s, 0);
        }
        run.press(&mut s, PAD_LEFT);
        run.press(&mut s, PAD_CROSS);
        for _ in 0..200 {
            if s.state == ST_DELETED {
                break;
            }
            run.step(&mut s, 0);
        }
        assert_eq!(s.state, ST_DELETED);
        assert_eq!(run.save.rec_chara[2], -1);
        assert!(run.dc.dirty);
        run.press(&mut s, PAD_CROSS);
        assert_eq!(s.state, ST_LIST);
        run.step(&mut s, PAD_TRIANGLE);
        for _ in 0..100 {
            if s.is_done() {
                break;
            }
            run.step(&mut s, 0);
        }
        assert!(s.is_done());
    }

    #[test]
    fn the_item_cursor_skips_empty_rows_but_not_the_last() {
        let items = [3, 0, 7, 0, 0, 0, 0, 0];
        assert!(!slot_skipped(&items, 0));
        assert!(slot_skipped(&items, 1));
        assert!(!slot_skipped(&items, 7));
        assert!(slot_skipped(&[400, 0, 0, 0, 0, 0, 0, 0], 0));
    }
}
