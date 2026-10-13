//! Evolution Z's customising screen (`EvoZ_Run(3)`, src/menu/evo_z_1.c, evo_z_2.c, evo_z_3.c, progress mode 49):
//! the character grid on a reel of chips with the portrait of the character under the cursor; for a character,
//! three item sets of `gSaveData->custom[cell]` to edit slot by slot from the owned items (with the details page),
//! the password of a set (`PassWin`), and the Z points paid into the character's experience, which raises its level
//! and with it its item slots.

use std::error::Error;

use bt3_data::chara_data::ChrTable;
use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim;
use crate::char_select::{
    COLS, Cell, PictureLoader, chr_grid_move_down, chr_grid_move_left, chr_grid_move_right,
    chr_grid_move_up,
};
use crate::dialog::{Command, Dialog, DialogAssets, Pad as DialogPad};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::item_panel::{ItemAssets, ItemHelp, help_textures_in};
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, image_indices, tex};
use crate::pass_win::PassWin;
use crate::password::{ItemParam, item_bonus};
use crate::rng::LibcRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::windows::{TextBox, TextDraw, draw_texts};

/// `EvoZ_Run(3)`: section 3 of `gMenuArc7`.
pub const ARCHIVE_SECTION: usize = 3;
/// `EvoZ_Load`: the background picture (file 0x3C0) and the portraits (file 0x2F9 + character).
const BG_FILE: u32 = 0x3C0;
const FACE_FILE: u32 = 0x2F9;
const ITEM_MAX: usize = 350;
const TABS: usize = 4;
const ROWS: i32 = 6;
const FADE_FRAMES: i32 = 0x14;
/// `PADG_L1` / `PADG_R1`: a page up / down (pad 0 is taken as a pad of status other than 1).
const PAD_L1: u32 = 0x4000;
const PAD_R1: u32 = 0x1_0000;
/// The chips of the reel (`EvoZ_SetRowChips`'s order): `tex1` images of the seven columns.
const CHIP_IMAGES: [usize; 7] = [8, 11, 12, 13, 14, 15, 16];
const CHIP_COUNT: usize = 165;
const PORTRAIT: usize = 42;

// EVOZ_ST_*.
const ST_GRID: i32 = 0;
const ST_MENU: i32 = 1;
const ST_SLOTS: i32 = 3;
const ST_PASSWORD: i32 = 4;
const ST_ZP: i32 = 5;
const ST_PAYING: i32 = 6;
const ST_LEVEL_UP: i32 = 7;
const ST_HELP: i32 = 9;
const ST_LIST: i32 = 10;

// EVOZ_* flags.
const STARTED: u32 = 4;
const HELP_SLOT: u32 = 8;
const PAYING: u32 = 0x20;
const LIMIT_UP: u32 = 0x40;
const LIST_BUSY: u32 = 0x80;
const REEL_READY: u32 = 0x100;
// EVOZ_SIDE_*.
const FACE_CHANGE: u32 = 1;
const FACE_READY: u32 = 2;

/// `EvoZ_SetListScissor`.
const LIST_CLIP: [i32; 4] = [0, 0xC6, 0x200, 0x19C];

/// `ez->tex0[n]` of `EvoZ_Load` (image 42 the portrait).
const TEX0: [ImageTexture; 43] = [
    tex(12, 0),
    tex(5, 0),
    tex(5, 1),
    tex(5, 2),
    tex(6, 0),
    tex(5, 3),
    tex(6, 1),
    tex(7, 0),
    tex(7, 1),
    tex(7, 2),
    tex(11, 2),
    tex(7, 4),
    tex(7, 3),
    tex(7, 5),
    tex(9, 0),
    tex(9, 1),
    tex(9, 2),
    tex(3, 0),
    tex(4, 0),
    tex(3, 2),
    tex(3, 1),
    tex(4, 1),
    tex(3, 3),
    tex(4, 2),
    tex(22, 1),
    tex(3, 4),
    tex(4, 3),
    tex(8, 0),
    tex(8, 1),
    tex(8, 2),
    tex(17, 0),
    tex(17, 2),
    None,
    None,
    tex(10, 0),
    tex(11, 0),
    tex(10, 1),
    tex(10, 2),
    tex(10, 3),
    tex(10, 4),
    tex(11, 1),
    tex(16, 0),
    None,
];
/// `ez->tex1[n]`: the reel (images 8, 11..16 the chips, 18..24 the chips before the last change).
const TEX1: [ImageTexture; 25] = [
    tex(18, 0),
    tex(19, 1),
    tex(18, 1),
    tex(21, 0),
    tex(19, 2),
    tex(19, 4),
    tex(19, 3),
    tex(20, 0),
    None,
    None,
    tex(22, 0),
    None,
    None,
    None,
    None,
    None,
    None,
    tex(22, 1),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];
/// `ez->tex2[n]`: the slot list and the item list.
const TEX2: [ImageTexture; 26] = [
    tex(14, 1),
    tex(14, 0),
    tex(11, 0),
    tex(10, 1),
    tex(10, 2),
    tex(10, 3),
    tex(14, 2),
    tex(30, 0),
    tex(22, 1),
    tex(15, 0),
    tex(25, 4),
    tex(26, 0),
    tex(28, 0),
    tex(28, 0),
    None,
    tex(29, 0),
    tex(27, 1),
    tex(27, 0),
    tex(26, 1),
    tex(25, 0),
    tex(25, 2),
    tex(25, 1),
    tex(25, 3),
    tex(24, 1),
    tex(24, 0),
    tex(24, 2),
];
const HELP_SECTION: usize = 2;
const PASS_SECTION: usize = 38;
const PASS_TEXTURES: [ImageTexture; 4] = crate::pass_win::textures(PASS_SECTION);
const DIALOG_SECTION: usize = 32;

/// What `EvoZ_Load` reads from its unpacked section.
pub struct EvoZAssets {
    pub data: EvoZData,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct EvoZData {
    /// Sections 13, 23, 31: the screen, the reel, the slot / item list.
    pub fods: [Vec<u8>; 3],
    pub pass_fod: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    /// Section 39: the dialog's questions.
    pub dialog_msgs: Vec<u8>,
    /// Sections 34, 35, 37: character names, form names, item names.
    pub text: [Vec<u8>; 3],
    /// Section 33: the master character grid.
    pub(crate) master: Vec<Cell>,
    pub help: ItemAssets,
    pub layout: EvoZLayout,
}

#[derive(Clone, Debug, Default)]
pub struct EvoZLayout {
    pub flash: [Vec<Option<usize>>; 3],
    pub pass: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
    pub help: Vec<Option<usize>>,
    /// Per character id, its chip (section 36, entry id + 1).
    pub chips: Vec<Option<usize>>,
}

impl EvoZLayout {
    pub fn new(dialog: Vec<Option<usize>>, first: impl Fn(FileKey) -> usize) -> Self {
        let i = |t: &[ImageTexture]| image_indices(t, &first);
        Self {
            flash: [i(&TEX0), i(&TEX1), i(&TEX2)],
            pass: i(&PASS_TEXTURES),
            dialog,
            help: i(&help_textures_in(HELP_SECTION)),
            chips: i(&chip_textures()),
        }
    }
}

fn chip_textures() -> Vec<ImageTexture> {
    (1..=CHIP_COUNT).map(|n| Some(((36, n), 0))).collect()
}

impl EvoZAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let help = help_textures_in(HELP_SECTION);
        let chips = chip_textures();
        let files = decode_files(
            res,
            [&TEX0[..], &TEX1, &TEX2, &PASS_TEXTURES, &help, &chips],
        )?;
        let dialog = DialogAssets::parse(pack_section(res, DIALOG_SECTION)?)?;
        let sub = |s: usize, n: usize| -> Result<Vec<u8>, Box<dyn Error>> {
            Ok(pack_section(pack_section(res, s)?, n)?.to_vec())
        };
        let sec =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        // The grid list: a count, then the cells from 0x10.
        let grid = pack_section(res, 33)?;
        let word = |at: usize| {
            grid.get(at..at + 4)
                .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
                .ok_or("short grid")
        };
        let count = word(0)? as usize;
        let master = (0..count)
            .map(|i| {
                let at = 0x10 + i * 0x24;
                Ok(Cell {
                    id: word(at)?,
                    form_count: word(at + 4)?,
                    form: [
                        word(at + 8)?,
                        word(at + 12)?,
                        word(at + 16)?,
                        word(at + 20)?,
                        word(at + 24)?,
                        word(at + 28)?,
                        word(at + 32)?,
                    ],
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        Ok(Self {
            data: EvoZData {
                fods: [sec(13)?, sec(23)?, sec(31)?],
                pass_fod: sub(PASS_SECTION, 1)?,
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                dialog_msgs: sec(39)?,
                text: [sec(34)?, sec(35)?, sec(37)?],
                master,
                help: ItemAssets {
                    help_fod: sub(HELP_SECTION, 2)?,
                    help_name: sub(HELP_SECTION, 6)?,
                    help_desc: sub(HELP_SECTION, 5)?,
                    ..ItemAssets::default()
                },
                layout: EvoZLayout::default(),
            },
            files,
            dialog,
        })
    }
}

/// `EvoZList`: the owned-or-not items of the slot list, in four tabs.
#[derive(Clone, Debug, Default)]
struct List {
    cur: [i32; TABS],
    tab: usize,
    rows: i32,
    rows_f: f32,
    edge: i32,
    count: [i32; TABS],
    ids: [Vec<i32>; TABS],
    top: [i32; TABS],
    bottom: [i32; TABS],
}

impl List {
    fn id(&self, i: i32) -> i32 {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.ids[self.tab].get(i).copied())
            .unwrap_or(0)
    }

    fn row(&self, i: i32) -> i32 {
        self.id(self.top[self.tab] + i)
    }

    fn cur_item(&self) -> i32 {
        self.id(self.cur[self.tab])
    }
}

/// `EvoZDialog`.
#[derive(Clone, Copy, Debug, Default)]
struct Question {
    state: i32,
    answer: i32,
    timer: i32,
    msg: i32,
}

/// `EvoZSide`: the cursor's cell and what the reel shows.
#[derive(Clone, Copy, Debug, Default)]
struct Side {
    col: i32,
    row: i32,
    chip: [i32; 7],
    flags: u32,
    chara: i32,
    mask: i32,
}

/// `EvoZ` and the frame loop of `EvoZ_Run`.
pub struct EvoZScreen {
    text: [Vec<u8>; 3],
    flash: [Flash; 3],
    tex: [Vec<Option<usize>>; 3],
    chips: Vec<Option<usize>>,
    background: Option<usize>,
    pass: PassWin,
    help: ItemHelp,
    dialog: Dialog,
    fade: ColorFade,
    pad: PadRepeat,
    chr: ChrTable,
    params: Vec<ItemParam>,
    flags: u32,
    cur: [i32; 11],
    state: i32,
    face_state: i32,
    loading: Option<usize>,
    list_side: i32,
    help_item: i32,
    zp: i32,
    set: i32,
    last_tab: usize,
    cur_item: i32,
    rows_used: i32,
    question: Question,
    side: Side,
    grid: Vec<Cell>,
    rows: i32,
    face_alpha: f32,
    capacity: i32,
    used: i32,
    kind: i32,
    exp: i32,
    next_exp: i32,
    exp_left: i32,
    bonus: [i32; 5],
    exp_bar: f32,
    list: List,
    se_timer: i32,
    result: i32,
    /// `gProgress->flags |= 1`: the save changed.
    dirty: bool,
    done: Option<i32>,
}

impl EvoZScreen {
    /// `EvoZ_Init(3)` and the first `ColorFade_StartIn`. `pick` is the character select's pick of side 0 (col, row),
    /// kept in `gProgress + 0x440`; it is reset when its cell cannot be chosen.
    pub fn new(
        data: &EvoZData,
        chr: &ChrTable,
        params: Vec<ItemParam>,
        save: &mut SaveData,
        pick: &mut crate::char_select::Pick,
        loader: &mut dyn PictureLoader,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Vec::new();
        for fod in &data.fods {
            let mut f = Flash::create(fod)?;
            f.play(1);
            flash.push(f);
        }
        let flash: [Flash; 3] = flash.try_into().map_err(|_| "three movies")?;
        let mut pass_flash = Flash::create(&data.pass_fod)?;
        pass_flash.play(1);
        let mut dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            Some(data.dialog_msgs.clone()),
            0,
        )?;
        dialog.set_layout(0);
        // ChrGrid_Build in mode 49: no custom character nor random cell, no forms.
        use crate::char_select::{GRID_NO_CUSTOM, GRID_NO_FORMS, GRID_NO_RANDOM};
        let (grid, _, _) = crate::char_select::chr_grid_build_with(
            &data.master,
            save,
            GRID_NO_CUSTOM | GRID_NO_RANDOM | GRID_NO_FORMS,
        );
        let rows = (grid.len() as i32 + COLS as i32 - 1) / COLS as i32;
        let selectable = |c: &Cell| {
            c.id < crate::char_select::ID_LOCKED || c.id == crate::char_select::ID_CUSTOM
        };
        let index = (pick.col + pick.row * COLS as i32).max(0) as usize;
        if !grid.get(index).is_some_and(selectable) {
            *pick = crate::char_select::Pick::default();
        }
        let mut screen = Self {
            text: data.text.clone(),
            flash,
            tex: data.layout.flash.clone(),
            chips: data.layout.chips.clone(),
            background: loader.picture(BG_FILE),
            pass: PassWin::new(pass_flash, data.layout.pass.clone()),
            help: ItemHelp::new(&data.help, data.layout.help.clone())?,
            dialog,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            chr: chr.clone(),
            params,
            flags: 0,
            cur: [0; 11],
            state: ST_GRID,
            face_state: 4,
            loading: None,
            list_side: 0,
            help_item: 0,
            zp: 0,
            set: 0,
            last_tab: 0,
            cur_item: 0,
            rows_used: 0,
            question: Question::default(),
            side: Side {
                col: pick.col,
                row: pick.row,
                ..Side::default()
            },
            grid,
            rows,
            face_alpha: 0.0,
            capacity: 0,
            used: 0,
            kind: 0,
            exp: 0,
            next_exp: 0,
            exp_left: 0,
            bonus: [0; 5],
            exp_bar: 0.0,
            list: List::default(),
            se_timer: 0,
            result: 1,
            dirty: false,
            done: None,
        };
        screen.set_row_chips();
        screen.side.chara = screen.side.chip[screen.side.col.clamp(0, 6) as usize];
        screen.refresh_status(save);
        // The owned-or-not items by tab: tab 0 the first two kinds, tabs 1..3 one kind each.
        let mut ids: [Vec<i32>; TABS] = Default::default();
        for (i, it) in chr.items.iter().enumerate().take(ITEM_MAX) {
            if it.flags & 5 == 0 && it.kind < 3 && it.flags & 0x40 != 0 {
                if it.kind != 2 {
                    ids[0].push(i as i32);
                }
                ids[usize::from(it.kind) + 1].push(i as i32);
            }
        }
        screen.list = List {
            cur: [0; TABS],
            tab: 0,
            rows: ROWS,
            rows_f: 6.16,
            edge: 0,
            count: std::array::from_fn(|t| ids[t].len() as i32),
            ids,
            top: [0; TABS],
            bottom: [ROWS - 1; TABS],
        };
        // File_LoadSync of the portrait.
        screen.tex[0][PORTRAIT] = loader.portrait(FACE_FILE + screen.side.chara.max(0) as u32);
        screen.side.flags |= FACE_READY;
        Ok(screen)
    }

    /// `EvoZ_Run`'s result once it has returned (`EvoMode_Main` ignores it).
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// The save changed (`gProgress->flags |= 1`).
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    fn cell(&self) -> usize {
        (self.side.col + self.side.row * COLS as i32).max(0) as usize
    }

    fn set_ids<'a>(&self, save: &'a SaveData) -> &'a [u16; 8] {
        &save.custom_items[self.cell()][self.set.clamp(0, 2) as usize]
    }

    fn set_ids_mut<'a>(&self, save: &'a mut SaveData) -> &'a mut [u16; 8] {
        let (cell, set) = (self.cell(), self.set.clamp(0, 2) as usize);
        &mut save.custom_items[cell][set]
    }

    fn item(&self, item: i32) -> bt3_data::chara_data::ItemInfo {
        usize::try_from(item)
            .ok()
            .and_then(|i| self.chr.items.get(i).copied())
            .unwrap_or_default()
    }

    /// `EvoZ_GetLevelExp`: the experience the cell's level began at.
    fn level_exp(&self, save: &SaveData) -> i32 {
        let level = i32::from(save.custom_level[self.cell()]);
        if level != 0 {
            self.chr.exp_at(self.side.chara, level - 1)
        } else {
            0
        }
    }

    /// `EvoZ_RefreshStatus`: the panel of the character and item set under the cursor (the set is fitted to the
    /// slots, as `ItemSet_Fit` does in place).
    fn refresh_status(&mut self, save: &mut SaveData) {
        let chara = self.side.chara;
        let cell = self.cell();
        self.kind = (self
            .chr
            .flags
            .get(chara.max(0) as usize)
            .copied()
            .unwrap_or(0)
            ^ 1)
            & 1;
        let level = i32::from(save.custom_level[cell]);
        self.capacity = self.chr.item_slots(chara, level);
        let mut ids = *self.set_ids(save);
        let (used, rows) = self.chr.item_set_fit(&mut ids, self.capacity);
        *self.set_ids_mut(save) = ids;
        self.used = used;
        self.rows_used = rows;
        self.exp = save.custom_exp(cell);
        self.next_exp = self.chr.exp_at(chara, level);
        self.exp_left = self.chr.max_exp(chara) - self.exp;
        self.bonus = item_bonus(&ids, &self.params);
        let base = self.level_exp(save);
        self.exp_bar = (self.exp - base) as f32 / (self.next_exp - base) as f32 * 6.66;
    }

    /// `EvoZ_SetRowChips`: the characters of the cursor's row on the reel's chips.
    fn set_row_chips(&mut self) {
        for i in 0..7 {
            let id = self
                .grid
                .get(i + self.side.row.max(0) as usize * COLS)
                .map_or(crate::char_select::ID_EMPTY, |c| c.id);
            self.side.chip[i] = id;
            let chip = usize::try_from(id)
                .ok()
                .and_then(|id| self.chips.get(id).copied())
                .flatten();
            self.tex[1][18 + i] = self.tex[1][CHIP_IMAGES[i]];
            self.tex[1][CHIP_IMAGES[i]] = chip;
        }
    }

    /// `EvoZ_UpdateFace`: one step of the portrait loader.
    fn update_face(&mut self, loader: &mut dyn PictureLoader) {
        match self.face_state {
            5 => self.face_state = 6,
            6 => {
                self.side.flags &= !FACE_CHANGE;
                self.face_state = 1;
            }
            1 => {
                self.loading = loader.portrait(FACE_FILE + self.side.chara.max(0) as u32);
                self.face_state = 2;
            }
            // File_UpdateRequests: the read is done by the next call.
            2 => self.face_state = 3,
            3 => {
                self.tex[0][PORTRAIT] = self.loading;
                self.side.flags |= FACE_READY;
                self.face_state = 4;
            }
            4 => {
                if self.side.flags & FACE_CHANGE != 0 {
                    self.tex[0][PORTRAIT] = None;
                    self.face_state = 6;
                }
            }
            _ => {}
        }
    }

    /// `EvoZ_RequestFace`.
    fn request_face(&mut self) {
        if self.side.flags & FACE_READY != 0 {
            self.side.flags ^= FACE_READY;
        } else {
            self.face_state = 5;
        }
        self.side.flags |= FACE_CHANGE;
    }

    /// `EvoZ_CanEquip`: owned, allowed for the character, within the slots and of no kind and group another
    /// slot has.
    fn can_equip(&self, save: &SaveData, item: i32) -> bool {
        if save.item(item.max(0) as usize) & 1 == 0 {
            return false;
        }
        if !self.chr.can_equip(item, self.side.chara) {
            return false;
        }
        let used = if self.cur_item != 0 {
            self.used - i32::from(self.item(self.cur_item - 1).slots)
        } else {
            self.used
        };
        if self.capacity < used + i32::from(self.item(item).slots) {
            return false;
        }
        let it = self.item(item);
        let ids = self.set_ids(save);
        !(0..7).any(|i| {
            i as i32 != self.cur[3] && ids[i] != 0 && {
                let o = self.item(i32::from(ids[i]) - 1);
                o.kind == it.kind && o.group == it.group
            }
        })
    }

    /// `EvoZ_ClipGoto`.
    fn clip_goto(&mut self, flash: usize, kind: i32, sub: i32, label: &str) {
        let name = match kind {
            0 => format!("mc_chara_chip_{:03}", self.side.col),
            1 => format!("menu_plate_{}", self.cur[1] + 1),
            2 => "mc_select_plate".to_owned(),
            3 => format!("mc_list_plate_{}", self.cur[3] + 1),
            5 => format!("mc_num_zp_{}", self.cur[5]),
            10 => {
                if sub == 0 {
                    let l = &self.list;
                    format!("mc_list_plate_{}", l.cur[l.tab] - l.top[l.tab] + 2)
                } else {
                    format!("mc_tab_btn_{}", self.list.tab)
                }
            }
            _ => return,
        };
        let r = self.flash[flash].find(None, &name);
        self.flash[flash].clip_goto_label(r, label);
    }

    /// One iteration of `EvoZ_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        save: &mut SaveData,
        loader: &mut dyn PictureLoader,
        libc: &mut LibcRandom,
        font: &mut Font,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done.is_some() {
            return frame;
        }
        self.update_face(loader);
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        self.update(save, &mut frame.sounds);
        self.draw(save, font, &mut frame.quads);
        let pad = Pad { pressed, repeat };
        self.update_dialog(pad, save, font, &mut frame);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_fading_out() {
            return frame;
        }
        if self.fade.is_out_done() {
            if self.face_state == 4 {
                self.done = Some(self.result);
            }
            return frame;
        }
        if self.question.state == 0 && self.flags & LIST_BUSY == 0 {
            self.input(pad, save, libc, &mut frame.sounds);
        }
        frame
    }

    /// `EvoZ_Update`: the movies, the end of a reel change or a list move, and the points paid, 33 a frame.
    fn update(&mut self, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        for i in 0..3 {
            if i != 1 || self.flags & REEL_READY == 0 {
                self.flash[i].advance();
            }
        }
        if self.side.mask != 0 && self.flash[1].trig() & 1 != 0 {
            self.side.mask = 0;
        }
        if self.flags & LIST_BUSY != 0 && self.flash[2].trig() & 1 != 0 {
            self.toggle_list();
        }
        if self.flags & REEL_READY == 0 && self.flash[1].trig() & 2 != 0 {
            self.flags |= REEL_READY;
        }
        if self.flags & PAYING != 0 {
            let cell = self.cell();
            let pay = if self.zp > 33 { 33 } else { self.zp };
            self.zp -= pay;
            save.set_money(save.money() - pay);
            save.set_custom_exp(cell, save.custom_exp(cell) + pay);
            self.exp = save.custom_exp(cell);
            if self.zp == 0 {
                self.exp_left = self.chr.max_exp(self.side.chara) - self.exp;
                self.flags ^= PAYING;
                self.state = ST_ZP;
            }
            if self.exp >= self.next_exp {
                let r = self.flash[0].find(None, "mc_ability_limit_up");
                self.flash[0].clip_goto_label(r, "fl_ability_limit_up_in");
                self.flags |= LIMIT_UP;
                self.state = ST_LEVEL_UP;
                save.custom_level[cell] += 1;
                self.refresh_status(save);
                sounds.push(TitleSound::Effect(2, 0x2F));
            } else {
                let base = self.level_exp(save);
                self.exp_bar = (self.exp - base) as f32 / (self.next_exp - base) as f32 * 6.66;
            }
            self.se_timer -= 1;
            if self.se_timer == -1 {
                sounds.push(TitleSound::Effect(2, 5));
                self.se_timer = 10;
            }
        }
        if self.flags & LIMIT_UP != 0 && self.flash[0].flags() & 8 != 0 {
            self.flags ^= LIMIT_UP;
            self.state = if self.flags & PAYING != 0 {
                ST_PAYING
            } else {
                ST_ZP
            };
        }
    }

    /// `EvoZ_ToggleList`: the slot list or the item list once the list movie has moved.
    fn toggle_list(&mut self) {
        self.list_side ^= 1;
        self.flags &= !LIST_BUSY;
        if self.list_side == 0 {
            self.clip_goto(2, 3, 0, "fl_on_start");
            for i in 0..8 {
                let r = self.flash[2].find(None, &format!("mc_list_plate_{}", i + 1));
                self.flash[2].clip_set_scissor(r, None);
                self.flash[2].clip_set_color(r, 1.0);
            }
        } else {
            self.clip_goto(2, 3, 0, "fl_off_start");
            self.clip_goto(2, 10, 0, "fl_on_start");
        }
    }

    /// `EvoZ_Draw`: the background, `EvoZ_Refresh`, the movies (the reel only until its intro is over), the texts,
    /// the password window and the details page.
    fn draw(&mut self, save: &SaveData, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        if let Some(bg) = self.background {
            quads.extend(picture_quads(bg).into_iter().map(ScreenQuad::from));
        }
        let texts = self.refresh(save, font);
        for i in 0..3 {
            if i != 1 || self.flags & REEL_READY == 0 {
                let tex = &self.tex[i];
                let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
                let (flash_quads, overs) = self.flash[i].draw_with_overs(&has_texture);
                quads.extend(screen_quads(&flash_quads, tex));
                draw_texts(&texts[i], &overs, font);
            }
        }
        font.flush(quads);
        self.pass.draw(quads);
        self.help
            .draw(self.help_item.max(0) as usize, &self.chr, font, quads);
    }

    fn set_uv(&mut self, f: usize, parent: Option<&str>, name: &str, uv: Uv) {
        let r = self.flash[f].find(parent, name);
        self.flash[f].clip_set_uv(r, uv);
    }

    fn set_visible(&mut self, f: usize, parent: Option<&str>, name: &str, on: bool) {
        let r = self.flash[f].find(parent, name);
        self.flash[f].clip_set_flags(r, PROP_VISIBLE, on);
    }

    fn set_tex(&mut self, f: usize, parent: &str, name: &str, t: i32) {
        let r = self.flash[f].find(Some(parent), name);
        self.flash[f].clip_set_tex(r, t);
    }

    #[allow(clippy::too_many_arguments)]
    fn attach(
        &mut self,
        f: usize,
        parent: Option<&str>,
        name: &str,
        line: i32,
        k: usize,
        font: &Font,
        texts: &mut Vec<TextDraw>,
    ) {
        // EvoZ_Init's boxes: 0 names (preset 1), 1 forms (3), the rest item names (6), rows 3..9 clipped.
        let (bx, text) = match k {
            0 => (TextBox::preset(1), 0),
            1 => (TextBox::preset(3), 1),
            3..=9 => (TextBox::preset(6).with_clip(0, 0x200, 0xC6, 0x198), 2),
            _ => (TextBox::preset(6), 2),
        };
        let r = self.flash[f].find(parent, name);
        let t = self.text[text].clone();
        texts.extend(bx.attach_line(&mut self.flash[f], r, 0x400 + k as u32, &t, line, font));
    }

    /// The cost icon of item `item` (0-based) on a clip, shown, or hidden.
    fn cost(&mut self, f: usize, name: &str, item: Option<i32>) {
        let r = self.flash[f].find(Some(name), "mc_icon_item_cost");
        match item.filter(|&i| self.item(i).slots != 0) {
            Some(i) => {
                let e = self.item(i);
                let u0 = i32::from(e.slots) * 32 - 32;
                let v0 = e.class() * 42;
                self.flash[f].clip_set_flags(r, PROP_VISIBLE, true);
                self.flash[f].clip_set_uv(
                    r,
                    Uv {
                        u0,
                        v0,
                        u1: u0 + 32,
                        v1: v0 + 42,
                        tex: 0,
                    },
                );
            }
            None => self.flash[f].clip_set_flags(r, PROP_VISIBLE, false),
        }
    }

    /// The kind icon of item `item` (0-based) on a clip, shown, or hidden.
    fn kind(&mut self, f: usize, name: &str, item: Option<i32>) {
        let r = self.flash[f].find(Some(name), "mc_icon_item_potara");
        match item {
            Some(i) => {
                let u0 = i32::from(self.item(i).kind) * 64;
                self.flash[f].clip_set_flags(r, PROP_VISIBLE, true);
                self.flash[f].clip_set_uv(
                    r,
                    Uv {
                        u0,
                        v0: 0,
                        u1: u0 + 64,
                        v1: 0x40,
                        tex: 0,
                    },
                );
            }
            None => self.flash[f].clip_set_flags(r, PROP_VISIBLE, false),
        }
    }

    /// `EvoZ_Refresh`: everything the three movies show this frame; the texts by movie.
    fn refresh(&mut self, save: &SaveData, font: &Font) -> [Vec<TextDraw>; 3] {
        let mut texts: [Vec<TextDraw>; 3] = Default::default();
        let uv = |u0: i32, v0: i32, u1: i32, v1: i32| Uv {
            u0,
            v0,
            u1,
            v1,
            tex: 0,
        };
        if self.side.flags & FACE_READY != 0 {
            self.face_alpha = (self.face_alpha + 0.075).min(1.0);
        } else {
            self.face_alpha = 0.0;
        }
        let r = self.flash[0].find(None, "mc_single_chara_l");
        self.flash[0].clip_set_alpha(r, self.face_alpha);
        let chara = self.side.chara;
        self.attach(0, None, "mc_name_text_l", chara, 0, font, &mut texts[0]);
        self.attach(0, None, "mc_form_text_l", chara, 1, font, &mut texts[0]);
        for i in 0..3 {
            let name = format!("menu_plate_{}", i + 1);
            if i == 2 {
                let r = self.flash[0].find(None, &name);
                let c = if self.next_exp == 0 { 0.4 } else { 1.0 };
                self.flash[0].clip_set_color(r, c);
            }
            let u = uv(0, i * 32, 0x100, i * 32 + 32);
            self.set_uv(0, Some(&name), "mc_menu_text_off", u);
            self.set_uv(0, Some(&name), "mc_menu_text_on", u);
        }
        let set_cur = self.cur[2];
        for i in 0..2 {
            let arrow = if i != 0 {
                "mc_yajirusi_right"
            } else {
                "mc_yajirusi_left"
            };
            self.set_uv(
                0,
                Some("mc_select_plate"),
                arrow,
                uv(i * 32, 0, i * 32 + 32, 0x20),
            );
            let text = if i != 0 {
                "mc_select_text_off"
            } else {
                "mc_select_text_on"
            };
            let u = uv(0, set_cur * 32, 0x200, set_cur * 32 + 32);
            self.set_uv(0, Some("mc_select_plate"), text, u);
        }
        self.set_uv(0, None, "mc_text_power_2", uv(0, 0, 0x100, 0x20));
        let digit = format!("mc_num_zp_{}", self.cur[5]);
        for i in 0..2 {
            let arrow = if i != 0 {
                "mc_yajirusi_num_dwn"
            } else {
                "mc_yajirusi_num_up"
            };
            self.set_uv(0, Some(&digit), arrow, uv(i * 32, 0x20, i * 32 + 32, 0x40));
        }
        let zp = self.zp;
        anim::num_draw_child(
            &mut self.flash[0],
            |n| (format!("mc_num_zp_{n}"), "mc_num_use_zp".to_owned()),
            5,
            zp,
            (0x20, 0x20),
            1,
        );
        anim::num_draw_child(
            &mut self.flash[0],
            |n| ("mc_status_base_1".to_owned(), format!("mc_pay_num_{n}")),
            7,
            save.money(),
            (0x20, 0x20),
            0,
        );
        let r = self.flash[0].find(None, "mc_status_attribute");
        self.flash[0].clip_set_tex(r, self.kind);
        for i in 0..4 {
            let name = format!("mc_status_ability_base1_{}", i + 1);
            self.set_uv(
                0,
                Some(&name),
                "mc_text_ability_1",
                uv(0, i * 20 + 20, 0x80, i * 20 + 40),
            );
            let b = self.bonus[i as usize + 1];
            for j in 0..3 {
                self.set_visible(
                    0,
                    Some(&name),
                    &format!("mc_status_ability_plus_{}", j + 1),
                    j < b,
                );
                self.set_visible(
                    0,
                    Some(&name),
                    &format!("mc_status_ability_minus_{}", j + 1),
                    b < -j,
                );
            }
        }
        self.set_uv(
            0,
            Some("mc_status_ability_base2_1"),
            "mc_text_ability_2",
            uv(0, 0, 0x80, 0x14),
        );
        for i in 0..7 {
            let on = i < self.bonus[0];
            self.set_visible(
                0,
                Some("mc_status_ability_base2_1"),
                &format!("mc_status_ability_plus_{}", i + 4),
                on,
            );
        }
        let (cap, fits) = (self.capacity, self.used);
        let big = |n: i32, s: i32| uv(n % 4 * s, n / 4 * s, n % 4 * s + s, n / 4 * s + s);
        self.set_uv(0, None, "mc_ability_limit_big_num", big(cap, 64));
        self.set_uv(
            0,
            Some("mc_ability_limit_up"),
            "mc_ability_limit_big_num_1",
            big(cap, 64),
        );
        self.set_uv(0, None, "mc_ability_limit_s_num", big(fits, 32));
        self.set_uv(
            0,
            Some("mc_ability_limit_up"),
            "mc_ability_limit_s_num_1",
            big(fits, 32),
        );
        self.set_uv(0, None, "mc_text_power_1", uv(0, 0x20, 0x100, 0x40));
        let mode = if self.next_exp != 0 { 0 } else { 2 };
        let (exp, next) = (self.exp, self.next_exp);
        anim::num_draw_child(
            &mut self.flash[0],
            |n| ("mc_status_power_base".to_owned(), format!("mc_exp_num_{n}")),
            7,
            exp,
            (0x20, 0x20),
            mode,
        );
        anim::num_draw_child(
            &mut self.flash[0],
            |n| {
                (
                    "mc_status_power_base".to_owned(),
                    format!("mc_next_num_{n}"),
                )
            },
            7,
            next,
            (0x20, 0x20),
            mode,
        );
        let r = self.flash[0].find(None, "mc_exp_bar_point");
        let sx = if self.next_exp != 0 {
            self.exp_bar
        } else {
            6.66
        };
        self.flash[0].clip_set_scale(r, sx, 1.0);
        // The third movie.
        for i in 0..2 {
            let arrow = if i != 0 {
                "mc_yajirusi_right"
            } else {
                "mc_yajirusi_left"
            };
            self.set_uv(
                2,
                Some("mc_custom_yajirusi"),
                arrow,
                uv(i * 32, 0, i * 32 + 32, 0x20),
            );
        }
        let action = self.cur[8];
        self.set_uv(
            2,
            Some("mc_custom_yajirusi"),
            "mc_text_custm",
            uv(0, action * 64, 0x100, action * 64 + 64),
        );
        self.set_uv(2, None, "mc_ability_limit_s_num", big(cap, 32));
        self.set_uv(2, None, "mc_ability_limit_big_num", big(fits, 64));
        if self.list_side == 0 {
            let ids = *self.set_ids(save);
            for i in 0..8usize {
                let name = format!("mc_list_plate_{}", i + 1);
                let id = i32::from(ids[i]);
                self.set_visible(2, Some(&name), "mc_font_new", false);
                let off = if (i as i32) < self.rows_used || i == 7 {
                    if id != 0 { 4 } else { 5 }
                } else {
                    7
                };
                self.set_tex(2, &name, "mc_list_plate_off", off);
                self.set_tex(2, &name, "mc_list_plate_on", if id != 0 { 0 } else { 1 });
                if action == 3 || self.cur[3] == i as i32 {
                    self.set_uv(
                        2,
                        Some(&name),
                        "mc_icon_custom",
                        uv(action * 64, 0, action * 64 + 64, 0x40),
                    );
                    self.set_visible(2, Some(&name), "mc_icon_custom", true);
                } else {
                    self.set_visible(2, Some(&name), "mc_icon_custom", false);
                }
                let item = (id != 0).then_some(id - 1);
                self.cost(2, &name, item);
                self.kind(2, &name, item);
                self.attach(
                    2,
                    Some(&name),
                    "mc_dammy_text",
                    item.unwrap_or(-1),
                    10 + i,
                    font,
                    &mut texts[2],
                );
            }
        } else {
            let name = "mc_list_plate_1";
            self.set_visible(2, Some(name), "mc_font_new", false);
            self.set_tex(2, name, "mc_list_plate_off", 4);
            self.set_visible(2, Some(name), "mc_icon_custom", false);
            let cur = self.cur_item;
            self.set_tex(2, name, "mc_list_plate_off", if cur != 0 { 4 } else { 5 });
            let item = (cur != 0).then_some(cur - 1);
            self.cost(2, name, item);
            self.kind(2, name, item);
            self.attach(
                2,
                Some(name),
                "mc_dammy_text",
                item.unwrap_or(-1),
                2,
                font,
                &mut texts[2],
            );
            for i in 0..=ROWS {
                // Rows 2..7, then plate 8: the row scrolling in or out.
                let (name, item, k) = if i < ROWS {
                    (
                        format!("mc_list_plate_{}", i + 2),
                        self.list.row(i),
                        3 + i as usize,
                    )
                } else {
                    (
                        "mc_list_plate_8".to_owned(),
                        self.list.id(self.list.edge),
                        9,
                    )
                };
                let plate = self.flash[2].find(None, &name);
                self.flash[2].clip_set_flags(plate, 0x400, true);
                if i == 0 || i == ROWS - 1 || i == ROWS {
                    self.flash[2].clip_set_scissor(plate, Some(LIST_CLIP));
                }
                let owned = save.item(item.max(0) as usize) & 1 != 0;
                let c = if self.can_equip(save, item) || !owned {
                    1.0
                } else {
                    0.4
                };
                self.flash[2].clip_set_color(plate, c);
                self.set_visible(2, Some(&name), "mc_icon_custom", false);
                self.set_tex(2, &name, "mc_list_plate_off", if owned { 2 } else { 6 });
                self.kind(2, &name, Some(item));
                self.cost(2, &name, owned.then_some(item));
                let new = save.item(item.max(0) as usize) & 2 != 0;
                self.set_visible(2, Some(&name), "mc_font_new", new);
                let line = if owned { item } else { ITEM_MAX as i32 };
                self.attach(
                    2,
                    Some(&name),
                    "mc_dammy_text",
                    line,
                    k,
                    font,
                    &mut texts[2],
                );
            }
            let tab = self.list.tab;
            for i in 0..TABS {
                let name = format!("mc_tab_btn_{i}");
                let r = self.flash[2].find(None, &name);
                let lit = if tab == 3 { i == 3 } else { i != 3 };
                self.flash[2].clip_set_color(r, if lit { 1.0 } else { 0.4 });
                let u = uv(i as i32 * 64, 0, i as i32 * 64 + 64, 0x40);
                self.set_uv(2, Some(&name), "mc_icon_tab_off", u);
                self.set_uv(2, Some(&name), "mc_icon_tab_on", u);
                self.set_visible(2, Some(&name), "mc_font_new_tab", false);
            }
            self.set_uv(
                2,
                None,
                "mc_tab_text",
                uv(0, tab as i32 * 32, 0x100, tab as i32 * 32 + 32),
            );
            let l = &self.list;
            let (top, bottom, count) = (l.top[tab], l.bottom[tab], l.count[tab]);
            for i in 0..2 {
                let arrow = if i != 0 {
                    "mc_menu_yajirusi_down"
                } else {
                    "mc_menu_yajirusi_up"
                };
                self.set_uv(
                    2,
                    Some("mc_menu_yajirusi"),
                    arrow,
                    uv(i * 32, 0x20, i * 32 + 32, 0x40),
                );
                let shown = if i == 0 {
                    top != 0
                } else {
                    bottom != count - 1
                };
                self.set_visible(2, Some("mc_menu_yajirusi"), arrow, shown);
            }
            let scale = self.list.rows as f32 / count as f32 * self.list.rows_f;
            let y = (self.list.rows_f * 32.0 * top as f32 / count as f32) as i32;
            let r = self.flash[2].find(None, "mc_scroll_bar_point");
            self.flash[2].clip_set_scale(r, 1.0, scale);
            self.flash[2].clip_set_offset(r, 0, y);
            let c = if self.list.rows >= count { 0.4 } else { 1.0 };
            self.flash[2].clip_set_color(r, c);
        }
        // The reel, until its intro is over.
        if self.flags & REEL_READY == 0 {
            self.set_uv(1, None, "mc_plate_text", uv(0, 0, 0x40, 0x20));
            if self.rows >= 2 {
                for i in 0..2 {
                    let (arrow, icon) = if i != 0 {
                        ("mc_yajirusi_down", "mc_yajirusi_icon_down")
                    } else {
                        ("mc_yajirusi_up", "mc_yajirusi_icon_up")
                    };
                    self.set_uv(1, Some(arrow), icon, uv(i * 32, 0x20, i * 32 + 32, 0x40));
                }
            } else {
                for arrow in ["mc_yajirusi_up", "mc_yajirusi_down"] {
                    self.set_visible(1, None, arrow, false);
                }
            }
            let mask = self.side.mask != 0;
            let r = self.flash[1].find(None, "mc_chara_mask");
            self.flash[1].clip_set_flags(r, 0x102, mask);
            for i in 0..14 {
                let r = self.flash[1].find(None, &format!("mc_chara_chip_{i:03}"));
                self.flash[1].clip_set_flags(r, 0x80, self.side.mask as u8 != 0);
            }
        }
        texts
    }

    /// `EvoZ_UpdateDialog`: the questions "remove every item of the set?" (1..4) and "pay the points?" (5..8); a
    /// yes takes effect 12 frames later.
    fn update_dialog(
        &mut self,
        pad: Pad,
        save: &mut SaveData,
        font: &mut Font,
        frame: &mut TitleFrame,
    ) {
        if self.question.state == 0 {
            return;
        }
        let mut effects = Vec::new();
        let dpad = DialogPad {
            pressed: pad.pressed,
            repeat: pad.repeat,
        };
        let q = &mut self.question;
        let state = q.state;
        let first = |d: &mut Dialog, q: &mut Question, next, effects: &mut Vec<(u32, u32)>| {
            q.state = next;
            d.set_layout(0);
            d.start(Command::Open, effects);
            d.set_cursor(1);
            q.answer = 0;
        };
        match state {
            1 | 2 | 5 | 6 => {
                let removing = state <= 2;
                if state == 1 || state == 5 {
                    first(
                        &mut self.dialog,
                        &mut self.question,
                        state + 1,
                        &mut effects,
                    );
                }
                self.dialog.set_choices(true);
                self.dialog.start(Command::Open, &mut effects);
                let q = &mut self.question;
                q.msg = i32::from(removing);
                if q.answer > 0 {
                    q.timer -= 1;
                    if q.timer < 0 {
                        self.dirty = true;
                        if removing {
                            *self.set_ids_mut(save) = [0; 8];
                            self.refresh_status(save);
                            self.clip_goto(2, 3, 0, "fl_off_start");
                            self.cur[3] = 0;
                            self.clip_goto(2, 3, 0, "fl_on_start");
                        } else {
                            self.state = ST_PAYING;
                            self.flags |= PAYING;
                        }
                        let q = &mut self.question;
                        q.timer = 0x78;
                        q.state = if removing { 3 } else { 7 };
                    }
                } else if q.answer < 0 {
                    self.dialog.set_cursor(1);
                    q.timer = 0x78;
                    q.state = if removing { 3 } else { 7 };
                } else {
                    q.answer = self.dialog.input(true, dpad, &mut effects);
                    q.timer = 12;
                }
            }
            3 | 7 => {
                self.dialog.set_choices(false);
                self.dialog.start(Command::Close, &mut effects);
                self.dialog.input(false, dpad, &mut effects);
                self.question.state = state + 1;
            }
            4 | 8 => {
                self.dialog.input(false, dpad, &mut effects);
                if self.dialog.is_closed() {
                    self.question.state = 0;
                }
            }
            _ => {}
        }
        self.dialog.set_msg(self.question.msg);
        self.dialog.draw(true, font, &mut frame.quads);
        frame
            .sounds
            .extend(effects.into_iter().map(|(b, id)| TitleSound::Effect(b, id)));
    }

    /// `EvoZ_RemoveItem`: the slot emptied; the gap closed below the last row.
    fn remove_item(&self, save: &mut SaveData) {
        let c = self.cur[3].clamp(0, 7) as usize;
        let ids = self.set_ids_mut(save);
        ids[c] = 0;
        if c != 7 {
            for i in c..6 {
                ids[i] = ids[i + 1];
            }
            ids[6] = 0;
        }
    }

    /// `EvoZ_Input`.
    fn input(
        &mut self,
        pad: Pad,
        save: &mut SaveData,
        libc: &mut LibcRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash[0].flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.clip_goto(1, 0, 0, "fl_on_start");
            self.flags |= STARTED;
        }
        match self.state {
            ST_GRID => self.input_grid(pad, save, sounds),
            ST_MENU => self.input_menu(pad, save, libc, sounds),
            ST_SLOTS => self.input_slots(pad, save, sounds),
            ST_PASSWORD => {
                if pad.pressed & PAD_TRIANGLE != 0 {
                    self.pass.close();
                    self.clip_goto(0, 1, 0, "fl_on_start");
                    self.clip_goto(0, 2, 0, "fl_on_start");
                    self.state = ST_MENU;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            ST_ZP => self.input_zp(pad, save, sounds),
            ST_PAYING => {
                if pad.pressed & PAD_CROSS != 0 {
                    let cell = self.cell();
                    save.set_money(save.money() - self.zp);
                    save.set_custom_exp(cell, save.custom_exp(cell) + self.zp);
                    self.exp = save.custom_exp(cell);
                    self.flags ^= PAYING;
                    self.zp = 0;
                    if self.exp >= self.next_exp {
                        let r = self.flash[0].find(None, "mc_ability_limit_up");
                        self.flash[0].clip_goto_label(r, "fl_ability_limit_up_in");
                        self.flags |= LIMIT_UP;
                        loop {
                            save.custom_level[cell] += 1;
                            self.next_exp = self
                                .chr
                                .exp_at(self.side.chara, i32::from(save.custom_level[cell]));
                            if !(self.exp >= self.next_exp && self.next_exp != 0) {
                                break;
                            }
                        }
                        self.state = ST_LEVEL_UP;
                        sounds.push(TitleSound::Effect(2, 0x2F));
                    } else {
                        self.state = ST_ZP;
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                    self.refresh_status(save);
                }
            }
            ST_HELP => self.input_help(pad, save, sounds),
            ST_LIST => self.input_list(pad, save, sounds),
            _ => {}
        }
    }

    fn input_grid(&mut self, pad: Pad, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        let moved = |s: &mut Self, save: &mut SaveData, sounds: &mut Vec<TitleSound>, se: u32| {
            s.side.chara = s.side.chip[s.side.col.clamp(0, 6) as usize];
            s.refresh_status(save);
            s.request_face();
            sounds.push(TitleSound::Effect(2, se));
        };
        if pad.repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
            self.clip_goto(1, 0, 0, "fl_off_start");
            let mut col = self.side.col;
            if pad.repeat & PAD_LEFT != 0 {
                chr_grid_move_left(&self.grid, &mut col, self.side.row);
            } else {
                chr_grid_move_right(&self.grid, &mut col, self.side.row);
            }
            self.side.col = col;
            self.clip_goto(1, 0, 0, "fl_on_start");
            moved(self, save, sounds, 0);
        } else if pad.repeat & (PAD_UP | PAD_DOWN) != 0 {
            if self.rows >= 2 {
                let up = pad.repeat & PAD_UP != 0;
                self.flash[1].goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                self.side.mask = 1;
                self.clip_goto(1, 0, 0, "fl_off_start");
                let (mut col, mut row) = (self.side.col, self.side.row);
                if up {
                    chr_grid_move_up(&self.grid, &mut col, &mut row, self.rows);
                } else {
                    chr_grid_move_down(&self.grid, &mut col, &mut row, self.rows);
                }
                (self.side.col, self.side.row) = (col, row);
                self.clip_goto(1, 0, 0, "fl_on_start");
                self.set_row_chips();
                moved(self, save, sounds, if up { 2 } else { 1 });
            }
        } else if pad.pressed & PAD_CROSS != 0 {
            self.flash[0].goto_label("fl_menu_in", true);
            self.flash[1].goto_label("fl_out", true);
            self.clip_goto(1, 0, 0, "fl_ok");
            self.clip_goto(0, 1, 0, "fl_on_start");
            self.clip_goto(0, 2, 0, "fl_on_start");
            self.cur[3] = 0;
            self.state = ST_MENU;
            sounds.push(TitleSound::Effect(1, 1));
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.clip_goto(1, 0, 0, "fl_off_start");
            self.flash[0].goto_label("fl_chara_sele_cansel", true);
            self.fade = ColorFade::start_out(true, FADE_FRAMES);
            self.result = 0;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    fn input_menu(
        &mut self,
        pad: Pad,
        save: &mut SaveData,
        libc: &mut LibcRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if pad.repeat & (PAD_UP | PAD_DOWN) != 0 {
            self.clip_goto(0, 1, 0, "fl_off_start");
            self.cur[1] = (self.cur[1] + if pad.repeat & PAD_UP != 0 { 2 } else { 1 }) % 3;
            self.clip_goto(0, 1, 0, "fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
            if pad.repeat & PAD_LEFT != 0 {
                self.cur[2] -= 1;
                if self.cur[2] < 0 {
                    self.cur[2] = 2;
                }
            } else {
                self.cur[2] += 1;
                if self.cur[2] >= 3 {
                    self.cur[2] = 0;
                }
            }
            self.cur[3] = 0;
            self.set = self.cur[2];
            self.refresh_status(save);
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.pressed & PAD_CROSS != 0 {
            match self.cur[1] {
                0 => {
                    self.clip_goto(0, 1, 0, "fl_ok");
                    self.clip_goto(0, 2, 0, "fl_off_start");
                    self.flash[0].goto_label("fl_custom_list_in", true);
                    self.flash[2].goto_label("fl_costum_list_in", true);
                    self.clip_goto(2, 3, 0, "fl_on_start");
                    self.state = ST_SLOTS;
                    sounds.push(TitleSound::Effect(1, 1));
                }
                1 => {
                    self.clip_goto(0, 1, 0, "fl_ok");
                    self.clip_goto(0, 2, 0, "fl_off_start");
                    let ids = *self.set_ids(save);
                    let level = i32::from(save.custom_level[self.cell()]);
                    self.pass.open(self.side.chara, ids, level, libc);
                    self.state = ST_PASSWORD;
                    sounds.push(TitleSound::Effect(1, 1));
                }
                _ => {
                    if self.next_exp != 0 {
                        self.clip_goto(0, 1, 0, "fl_ok");
                        self.clip_goto(0, 2, 0, "fl_off_start");
                        self.flash[0].goto_label("fl_zp_in", true);
                        self.clip_goto(0, 5, 0, "fl_on_loop");
                        self.zp = 0;
                        self.cur[2] = 3;
                        self.state = ST_ZP;
                        sounds.push(TitleSound::Effect(1, 1));
                    } else {
                        sounds.push(TitleSound::Effect(1, 7));
                    }
                }
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.flash[0].goto_label("fl_menu_cansel", true);
            self.flags &= !REEL_READY;
            self.flash[1].goto_label("fl_in", true);
            self.clip_goto(1, 0, 0, "fl_on_start");
            self.clip_goto(0, 1, 0, "fl_off_start");
            self.clip_goto(0, 2, 0, "fl_off_start");
            self.state = ST_GRID;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    fn input_slots(&mut self, pad: Pad, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        let ids = *self.set_ids(save);
        let id_at = |i: i32| ids.get(i.max(0) as usize).copied().unwrap_or(0);
        if pad.repeat & PAD_UP != 0 {
            self.clip_goto(2, 3, 0, "fl_off_start");
            if self.cur[3] == 7 {
                while self.cur[3] >= self.rows_used {
                    self.cur[3] -= 1;
                }
                if id_at(self.cur[3]) == 0 {
                    loop {
                        self.cur[3] -= 1;
                        if id_at(self.cur[3]) != 0 {
                            self.cur[3] += 1;
                            break;
                        }
                        if self.cur[3] == 0 {
                            break;
                        }
                    }
                }
            } else {
                self.cur[3] -= 1;
                if self.cur[3] < 0 {
                    self.cur[3] = 7;
                }
            }
            self.clip_goto(2, 3, 0, "fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.repeat & PAD_DOWN != 0 {
            self.clip_goto(2, 3, 0, "fl_off_start");
            if self.cur[3] == 7 {
                self.cur[3] = 0;
            } else if id_at(self.cur[3]) == 0 || {
                self.cur[3] += 1;
                self.cur[3] >= self.rows_used
            } {
                self.cur[3] = 7;
            }
            self.clip_goto(2, 3, 0, "fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
            self.cur[8] = (self.cur[8] + if pad.repeat & PAD_LEFT != 0 { 3 } else { 1 }) % 4;
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.pressed & PAD_CROSS != 0 {
            let id = id_at(self.cur[3]);
            match self.cur[8] {
                0 => {
                    self.list.tab = if self.cur[3] == 7 { 3 } else { self.last_tab };
                    self.flash[2].goto_label("fl_list_in", true);
                    self.clip_goto(2, 3, 0, "fl_ok");
                    self.flags |= LIST_BUSY;
                    self.clip_goto(2, 10, 1, "fl_on_start");
                    self.cur_item = i32::from(id);
                    self.state = ST_LIST;
                    sounds.push(TitleSound::Effect(1, 1));
                }
                1 => {
                    self.clip_goto(2, 3, 0, "fl_ok");
                    if id != 0 {
                        self.help.open();
                        self.flags |= HELP_SLOT;
                        self.help_item = i32::from(id) - 1;
                        let i = usize::from(id) - 1;
                        save.set_item(i, save.item(i) & !2);
                        self.state = ST_HELP;
                        sounds.push(TitleSound::Effect(1, 1));
                    } else {
                        sounds.push(TitleSound::Effect(1, 7));
                    }
                }
                2 => {
                    self.clip_goto(2, 3, 0, "fl_ok");
                    if id != 0 {
                        self.dirty = true;
                        self.remove_item(save);
                        self.refresh_status(save);
                        sounds.push(TitleSound::Effect(1, 1));
                    } else {
                        sounds.push(TitleSound::Effect(1, 7));
                    }
                }
                _ => {
                    self.clip_goto(2, 3, 0, "fl_ok");
                    if self.used != 0 || ids[7] != 0 {
                        self.question.state = 1;
                        sounds.push(TitleSound::Effect(1, 1));
                    } else {
                        sounds.push(TitleSound::Effect(1, 7));
                    }
                }
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.flash[0].goto_label("fl_custom_list_cansel", true);
            self.flash[2].goto_label("fl_costum_list_cansel", true);
            self.clip_goto(0, 1, 0, "fl_on_start");
            self.clip_goto(0, 2, 0, "fl_on_start");
            self.clip_goto(2, 3, 0, "fl_off_start");
            self.state = ST_MENU;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    fn input_zp(&mut self, pad: Pad, save: &SaveData, sounds: &mut Vec<TitleSound>) {
        let digit = self.cur[5];
        let step = if digit != 0 {
            10i32.pow(digit as u32)
        } else {
            1
        };
        if pad.repeat & PAD_UP != 0 {
            if self.zp < save.money() && self.zp < self.exp_left {
                self.zp += step;
                if save.money() < self.zp {
                    self.zp = save.money();
                }
                if self.exp_left < self.zp {
                    self.zp = self.exp_left;
                }
                sounds.push(TitleSound::Effect(2, 0x2B));
            }
        } else if pad.repeat & PAD_DOWN != 0 {
            if self.zp > 0 {
                self.zp = (self.zp - step).max(0);
                sounds.push(TitleSound::Effect(2, 0x2B));
            }
        } else if pad.repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
            self.clip_goto(0, 5, 0, "fl_off");
            self.cur[5] = (self.cur[5] + if pad.repeat & PAD_LEFT != 0 { 1 } else { 4 }) % 5;
            self.clip_goto(0, 5, 0, "fl_on_loop");
            sounds.push(TitleSound::Effect(2, 0x2C));
        } else if pad.pressed & PAD_CROSS != 0 {
            if self.zp != 0 {
                self.question.state = 5;
                sounds.push(TitleSound::Effect(2, 0x2D));
            } else {
                sounds.push(TitleSound::Effect(1, 7));
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.flash[0].goto_label("fl_zp_cansel", true);
            self.clip_goto(0, 5, 0, "fl_off");
            self.clip_goto(0, 1, 0, "fl_on_start");
            self.clip_goto(0, 2, 0, "fl_on_start");
            self.cur[2] = self.set;
            self.state = ST_MENU;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    fn input_help(&mut self, pad: Pad, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        if pad.pressed & PAD_CROSS != 0 {
            if self.flags & HELP_SLOT != 0 {
                self.flags ^= HELP_SLOT;
                self.help.close();
                self.clip_goto(2, 3, 0, "fl_on_start");
                self.state = ST_SLOTS;
                sounds.push(TitleSound::Effect(1, 2));
            } else if self.can_equip(save, self.help_item) {
                let id = (self.list.cur_item() + 1) as u16;
                let c = self.cur[3].clamp(0, 7) as usize;
                self.set_ids_mut(save)[c] = id;
                self.cur_item = i32::from(id);
                self.dirty = true;
                self.refresh_status(save);
                self.help.close();
                self.flash[2].goto_label("fl_list_out", true);
                self.flags |= LIST_BUSY;
                self.clip_goto(2, 10, 0, "fl_off_start");
                self.clip_goto(2, 10, 1, "fl_off_start");
                self.state = ST_SLOTS;
                sounds.push(TitleSound::Effect(1, 1));
            } else {
                sounds.push(TitleSound::Effect(1, 7));
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.help.close();
            if self.flags & HELP_SLOT != 0 {
                self.clip_goto(2, 3, 0, "fl_on_start");
                self.flags ^= HELP_SLOT;
                self.state = ST_SLOTS;
            } else {
                self.clip_goto(2, 10, 0, "fl_on_start");
                self.state = ST_LIST;
            }
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    fn input_list(&mut self, pad: Pad, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        let t = self.list.tab;
        if pad.repeat & PAD_L1 != 0 {
            if self.list.cur[t] > 0 {
                self.clip_goto(2, 10, 0, "fl_off_start");
                let l = &mut self.list;
                l.cur[t] -= ROWS;
                if l.cur[t] < l.top[t] {
                    l.edge = l.top[t];
                    if l.cur[t] < ROWS {
                        l.cur[t] = 0;
                        l.top[t] = 0;
                        l.bottom[t] = ROWS - 1;
                    } else {
                        l.top[t] -= ROWS;
                        l.bottom[t] -= ROWS;
                    }
                }
                self.clip_goto(2, 10, 0, "fl_on_start");
                sounds.push(TitleSound::Effect(1, 0));
            }
        } else if pad.repeat & PAD_R1 != 0 {
            if self.list.cur[t] < self.list.count[t] - 1 {
                self.clip_goto(2, 10, 0, "fl_off_start");
                let l = &mut self.list;
                l.cur[t] += ROWS;
                if l.bottom[t] < l.cur[t] {
                    l.edge = l.bottom[t];
                    if l.count[t] - ROWS < l.cur[t] {
                        l.cur[t] = l.count[t] - 1;
                        l.top[t] = l.count[t] - ROWS;
                        l.bottom[t] = l.count[t] - 1;
                    } else {
                        l.top[t] += ROWS;
                        l.bottom[t] += ROWS;
                    }
                }
                self.clip_goto(2, 10, 0, "fl_on_start");
                sounds.push(TitleSound::Effect(1, 0));
            }
        } else if pad.repeat & PAD_UP != 0 {
            if self.list.cur[t] > 0 {
                self.clip_goto(2, 10, 0, "fl_off_start");
                let l = &mut self.list;
                l.cur[t] -= 1;
                if l.cur[t] < l.top[t] {
                    self.flash[2].goto_label("fl_list_down", true);
                    l.edge = l.bottom[t];
                    l.top[t] -= 1;
                    l.bottom[t] -= 1;
                }
                self.clip_goto(2, 10, 0, "fl_on_start");
                sounds.push(TitleSound::Effect(1, 0));
            }
        } else if pad.repeat & PAD_DOWN != 0 {
            if self.list.cur[t] < self.list.count[t] - 1 {
                self.clip_goto(2, 10, 0, "fl_off_start");
                let l = &mut self.list;
                l.cur[t] += 1;
                if l.cur[t] > l.bottom[t] {
                    self.flash[2].goto_label("fl_list_up", true);
                    l.edge = l.top[t];
                    l.top[t] += 1;
                    l.bottom[t] += 1;
                }
                self.clip_goto(2, 10, 0, "fl_on_start");
                sounds.push(TitleSound::Effect(1, 0));
            }
        } else if pad.repeat & (PAD_LEFT | PAD_RIGHT) != 0 && t != 3 {
            self.clip_goto(2, 10, 0, "fl_off_start");
            self.clip_goto(2, 10, 1, "fl_off_start");
            self.list.tab = if pad.repeat & PAD_LEFT != 0 {
                (t + 2) % 3
            } else {
                (t + 1) % 3
            };
            self.clip_goto(2, 10, 0, "fl_on_start");
            self.clip_goto(2, 10, 1, "fl_on_start");
            self.last_tab = self.list.tab;
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.pressed & PAD_CROSS != 0 {
            self.clip_goto(2, 10, 0, "fl_ok");
            let id = self.list.cur_item();
            if save.item(id.max(0) as usize) & 1 != 0 {
                self.help.open();
                self.help_item = id;
                let i = id.max(0) as usize;
                save.set_item(i, save.item(i) & !2);
                self.state = ST_HELP;
                sounds.push(TitleSound::Effect(1, 1));
            } else {
                sounds.push(TitleSound::Effect(1, 7));
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.flash[2].goto_label("fl_list_out", true);
            self.flags |= LIST_BUSY;
            self.clip_goto(2, 10, 0, "fl_off_start");
            self.clip_goto(2, 10, 1, "fl_off_start");
            self.state = ST_SLOTS;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }
}

/// The pad words the screen reads (`gamePressed`, `gameRepeat`).
#[derive(Clone, Copy, Debug)]
struct Pad {
    pressed: u32,
    repeat: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Pictures;

    impl PictureLoader for Pictures {
        fn portrait(&mut self, file_id: u32) -> Option<usize> {
            Some(file_id as usize)
        }

        fn picture(&mut self, file_id: u32) -> Option<usize> {
            Some(file_id as usize)
        }
    }

    /// Runs the real customising screen from the ISO in `rom/`, when it is there: a character of the grid, an
    /// owned item put into its first set's first slot, then Z points paid into its experience through the
    /// confirmation dialog.
    #[test]
    fn real_customising_equips_an_item_and_pays_points() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let mut read = |id: u32| {
            let location = partition_entry_for_id(id);
            let archive =
                AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition))
                    .unwrap();
            archive.read_entry(&mut image, location.index).unwrap()
        };
        let file4 = read(4);
        let chr = ChrTable::parse(&file4, 0xA1);
        let params =
            crate::password::item_params(crate::password::section_bytes(&file4, 2).unwrap());
        let pack = read(crate::evo_top::ARCHIVE_FILE_ID);
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = EvoZAssets::parse(&res).unwrap();
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
        data.layout = EvoZLayout::new(dialog, |k| firsts[&k]);
        let mut save = SaveData::new(&chr.items);
        save.add_money(5000);
        for i in 0..ITEM_MAX {
            save.add_item(i);
        }
        let mut pick = crate::char_select::Pick::default();
        let mut screen =
            EvoZScreen::new(&data, &chr, params, &mut save, &mut pick, &mut Pictures).unwrap();
        assert!(screen.rows > 2, "{}", screen.rows);
        assert!(screen.list.count[0] > 0);
        let mut libc = LibcRandom::new(1);
        let mut font = Font::new(Vec::new(), 20);
        let mut press = |screen: &mut EvoZScreen, save: &mut SaveData, bits: u32| {
            for i in 0..40 {
                let b = if i == 0 { bits } else { 0 };
                screen.run_frame(b, b, save, &mut Pictures, &mut libc, &mut font);
            }
        };
        for _ in 0..4 {
            press(&mut screen, &mut save, 0);
        }
        let chara = screen.side.chara;
        assert_eq!(
            screen.tex[0][PORTRAIT],
            Some((FACE_FILE + chara as u32) as usize)
        );
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.state, ST_MENU);
        // The slots of set 1, then the item list on the first slot.
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.state, ST_SLOTS);
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.state, ST_LIST);
        assert_eq!(screen.list_side, 1);
        // The first item that can go in; its details page, then confirm puts it in.
        let tab = screen.list.tab;
        while !screen.can_equip(&save, screen.list.cur_item()) {
            let before = screen.list.cur[tab];
            press(&mut screen, &mut save, PAD_DOWN);
            assert_ne!(screen.list.cur[tab], before, "no item can be equipped");
        }
        let item = screen.list.cur_item();
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.state, ST_HELP);
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.state, ST_SLOTS);
        let cell = screen.cell();
        assert_eq!(save.custom_items[cell][0][0], item as u16 + 1);
        assert!(screen.dirty());
        // Back to the menu, then the points page: 1000 points paid.
        press(&mut screen, &mut save, PAD_TRIANGLE);
        assert_eq!(screen.state, ST_MENU);
        press(&mut screen, &mut save, PAD_UP);
        assert_eq!(screen.cur[1], 2);
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.state, ST_ZP);
        // Digit 3 (thousands): right goes down from 0 to 4, so left three times.
        for _ in 0..3 {
            press(&mut screen, &mut save, PAD_LEFT);
        }
        assert_eq!(screen.cur[5], 3);
        press(&mut screen, &mut save, PAD_UP);
        let zp = screen.zp;
        assert!(zp > 0 && zp <= 1000, "{zp}");
        let money = save.money();
        press(&mut screen, &mut save, PAD_CROSS);
        assert_ne!(screen.question.state, 0);
        // "Yes" is the first choice; the cursor starts on "no".
        // The window opens (39 frames).
        press(&mut screen, &mut save, 0);
        assert!(screen.dialog.is_open());
        press(&mut screen, &mut save, PAD_LEFT);
        assert_eq!(screen.dialog.cursor(), 0);
        press(&mut screen, &mut save, PAD_CROSS);
        for _ in 0..10 {
            press(&mut screen, &mut save, 0);
        }
        assert_eq!(save.money(), money - zp);
        assert_eq!(save.custom_exp(cell), zp);
        assert_eq!(screen.question.state, 0);
    }
}
