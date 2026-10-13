//! Dragon World Tour's entrant select (`EntrySel_Run(2)`, src/menu/entry_select.c, progress mode 34): over the
//! tournament's backdrop, with its guide, the players choose their entrants one after another as one side of the
//! character select does (the grid on a reel of chips, a cell's forms, an item set with its panel and details page,
//! a costume). The Yamcha Game draws them at random. The entrants then go to the tournament (`gProgress`).

use std::error::Error;

use bt3_data::chara_data::ChrTable;
use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{
    COLS, Cell, ID_CUSTOM, ID_RANDOM, Pick, PictureLoader, chr_grid_move_down, chr_grid_move_left,
    chr_grid_move_right, chr_grid_move_up, chr_grid_next_form, chr_grid_prev_form,
};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::item_panel::{ItemAssets, ItemHelp, ItemPanel, help_textures_in, panel_textures_in};
use crate::logo::ColorFade;
use crate::menu_res::{FileKey, ImageTexture, decode_files, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::tour_bg::TourBg;
use crate::tour_menu::{ENT_ITEMS, ENT_PLAYER, ENT_REC, TourSession, VOICE_BASE};
use crate::windows::{MsgWin, TextBox, TextDraw, draw_texts};

/// `EntrySel_Run(2)`: section 2 of `gMenuArc4`.
pub const ARCHIVE_SECTION: usize = 2;
/// The music of mode 34.
pub const BGM_FILE_ID: u32 = 0x10B1B;
const FACE_FILE: u32 = 0x2F9;
const FADE_FRAMES: i32 = 0x14;
const ENTRY_MAX: usize = 8;
const CHIP_COUNT: usize = 165;
/// The reel's chip images (`EntrySel_SwapRowTex`'s order).
const CHIP_IMAGES: [usize; 7] = [8, 11, 12, 13, 14, 15, 16];
const PORTRAIT: usize = 5;
const YAMCHA: i32 = 4;
/// `ESEL_ID_NONE`: an empty chip.
const ID_NONE: i32 = 0xA4;

// ESEL_STEP_*.
const STEP_CHARA: i32 = 0;
const STEP_FORM: i32 = 1;
const STEP_CUSTOM: i32 = 2;
const STEP_PANEL: i32 = 3;
const STEP_HELP: i32 = 4;
const STEP_COLOR: i32 = 5;

// ESEL_* flags.
const DONE: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;
const NO_CUSTOM: u32 = 0x10;
// ESEL_SEL_*.
const IMAGE_CHANGE: u32 = 1;
const IMAGE_READY: u32 = 2;
const SEL_FORM: u32 = 0x40;
const SEL_REC: u32 = 0x80;

// ESEL_LOAD_*.
const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;
const LOAD_SHOWN: i32 = 4;
const LOAD_ABORT: i32 = 5;
const LOAD_RESTART: i32 = 6;

// ESEL_END_*.
const END_NONE: i32 = 0;
const END_SPEAK: i32 = 1;
const END_WAIT: i32 = 2;
const END_LEAVE: i32 = 3;

/// `texA[n]` (images 5 the portrait, 10..17 the chosen entrants' chips).
const TEX_A: [ImageTexture; 19] = [
    tex(1, 0),
    tex(2, 0),
    tex(9, 0),
    tex(2, 1),
    tex(4, 0),
    None,
    tex(5, 0),
    tex(5, 2),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    tex(2, 2),
];
/// `texB[n]`: the reel (8, 11..16 the chips, 18..24 those before the last change).
const TEX_B: [ImageTexture; 25] = [
    tex(6, 0),
    tex(7, 1),
    tex(6, 1),
    tex(2, 1),
    tex(7, 2),
    tex(7, 4),
    tex(7, 3),
    tex(8, 0),
    None,
    None,
    tex(9, 0),
    None,
    None,
    None,
    None,
    None,
    None,
    tex(9, 1),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];
/// `texC[n]`: the item set and costume plates.
const TEX_C: [ImageTexture; 11] = [
    tex(11, 1),
    tex(15, 1),
    tex(13, 0),
    tex(12, 1),
    tex(9, 1),
    tex(13, 1),
    tex(14, 0),
    tex(12, 0),
    tex(14, 1),
    tex(11, 0),
    tex(15, 0),
];
/// `texD[n]` and the guide's movie, by tournament.
fn guide_textures(tour: usize) -> ([ImageTexture; 9], usize) {
    let mut t = [None; 9];
    let movie = match tour {
        0 => {
            t[0] = tex(17, 0);
            t[1] = tex(17, 1);
            18
        }
        1 | 2 => {
            let s = if tour == 1 { 19 } else { 21 };
            t[0] = tex(s, 0);
            t[2] = tex(s, 1);
            t[1] = tex(s, 3);
            s + 1
        }
        3 => {
            t[0] = tex(23, 0);
            t[1] = tex(23, 1);
            24
        }
        _ => {
            t[0] = tex(26, 0);
            t[2] = tex(26, 1);
            t[1] = tex(26, 3);
            t[6] = tex(28, 0);
            t[8] = tex(28, 1);
            t[7] = tex(28, 3);
            25
        }
    };
    (t, movie)
}
const PANEL_SECTION: usize = 39;
const HELP_SECTION: usize = 35;
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((34, 1), 0)), Some(((34, 1), 1)), None];

fn chip_textures() -> Vec<ImageTexture> {
    (1..=CHIP_COUNT).map(|n| Some(((32, n), 0))).collect()
}

/// What `EntrySel_Init` reads from its unpacked section.
pub struct EntrySelAssets {
    pub data: EntrySelData,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct EntrySelData {
    /// Sections 3, 10, 16: the picture and entry list, the reel, the plates; the five guides' movies.
    pub fods: [Vec<u8>; 3],
    pub guide_fods: Vec<Vec<u8>>,
    pub msg_fod: Vec<u8>,
    /// Section 33: the guides' lines; 36: their mouth keys; 30 / 31: character and form names.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    pub names: Vec<u8>,
    pub forms: Vec<u8>,
    /// Section 29: the master character grid.
    pub(crate) master: Vec<Cell>,
    pub items: ItemAssets,
    pub layout: EntrySelLayout,
}

#[derive(Clone, Debug, Default)]
pub struct EntrySelLayout {
    pub flash: [Vec<Option<usize>>; 3],
    pub guide: Vec<Vec<Option<usize>>>,
    pub msg: Vec<Option<usize>>,
    pub panel: Vec<Option<usize>>,
    pub help: Vec<Option<usize>>,
    pub chips: Vec<Option<usize>>,
}

impl EntrySelLayout {
    pub fn new(first: impl Fn(FileKey) -> usize) -> Self {
        let i = |t: &[ImageTexture]| image_indices(t, &first);
        Self {
            flash: [i(&TEX_A), i(&TEX_B), i(&TEX_C)],
            guide: (0..5).map(|t| i(&guide_textures(t).0)).collect(),
            msg: i(&MSG_TEXTURES),
            panel: i(&panel_textures_in(PANEL_SECTION, 0)),
            help: i(&help_textures_in(HELP_SECTION)),
            chips: i(&chip_textures()),
        }
    }
}

impl EntrySelAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let guides: Vec<[ImageTexture; 9]> = (0..5).map(|t| guide_textures(t).0).collect();
        let panel = panel_textures_in(PANEL_SECTION, 0);
        let help = help_textures_in(HELP_SECTION);
        let chips = chip_textures();
        let mut tables: Vec<&[ImageTexture]> =
            vec![&TEX_A, &TEX_B, &TEX_C, &MSG_TEXTURES, &panel, &help, &chips];
        tables.extend(guides.iter().map(|g| &g[..]));
        let files = decode_files(res, tables)?;
        let sec =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        let grid = pack_section(res, 29)?;
        let word = |at: usize| -> Result<i32, Box<dyn Error>> {
            Ok(i32::from_le_bytes(
                grid.get(at..at + 4).ok_or("short grid")?.try_into()?,
            ))
        };
        let master = (0..word(0)? as usize)
            .map(|i| {
                let at = 0x10 + i * 0x24;
                Ok(Cell {
                    id: word(at)?,
                    form_count: word(at + 4)?,
                    form: std::array::from_fn(|k| word(at + 8 + k * 4).unwrap_or(0)),
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        Ok(Self {
            data: EntrySelData {
                fods: [sec(3)?, sec(10)?, sec(16)?],
                guide_fods: (0..5)
                    .map(|t| sec(guide_textures(t).1))
                    .collect::<Result<_, _>>()?,
                msg_fod: pack_section(pack_section(res, 34)?, 2)?.to_vec(),
                msg_text: sec(33)?,
                lips: sec(36)?,
                names: sec(30)?,
                forms: sec(31)?,
                master,
                items: ItemAssets::parse_in(res, PANEL_SECTION, HELP_SECTION)?,
                layout: EntrySelLayout::default(),
            },
            files,
        })
    }
}

/// `ESelState` beside the entries: what the reel shows and the step being made.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    row_chara: [i32; 7],
    flags: u32,
    image: i32,
    cur: usize,
    step: i32,
    row_anim: i32,
}

/// `ESel` and the frame loop of `EntrySel_Run`.
pub struct EntrySelScreen {
    data: EntrySelData,
    bg: TourBg,
    flash: [Flash; 4],
    tex: [Vec<Option<usize>>; 4],
    msg: MsgWin,
    panel: ItemPanel,
    help: ItemHelp,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    chr: ChrTable,
    tour: i32,
    entry_num: usize,
    flags: u32,
    timer: i32,
    voice_line: i32,
    load_state: i32,
    loading: Option<usize>,
    help_item: i32,
    end_step: i32,
    entry: [Pick; ENTRY_MAX],
    sel: State,
    grid: Vec<Cell>,
    rows: i32,
    custom_count: i32,
    color_count: i32,
    image_alpha: f32,
    talker: usize,
    blink: [i32; 2],
    talk: [i32; 2],
    result: i32,
    session: TourSession,
    done: Option<i32>,
}

impl EntrySelScreen {
    /// `EntrySel_Init(2)` and the first `ColorFade_StartIn`. `last` is `gProgress + 0x440`, the first entrant's
    /// cursor of the last visit (reset when its cell cannot be chosen).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &EntrySelData,
        bg: TourBg,
        session: &TourSession,
        chr: &ChrTable,
        save: &SaveData,
        last: &mut Pick,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let tour = session.tour.clamp(0, 4);
        let make = |fod: &[u8]| -> Result<Flash, Box<dyn Error>> {
            let mut f = Flash::create(fod)?;
            f.play(1);
            Ok(f)
        };
        let mut guide = make(&data.guide_fods[tour as usize])?;
        guide.goto_label("fl_guide_in", true);
        let flash = [
            make(&data.fods[0])?,
            make(&data.fods[1])?,
            make(&data.fods[2])?,
            guide,
        ];
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            false,
        )?;
        msg.open();
        // ChrGrid_Build in mode 34: no custom character cell.
        let (grid, _, _) = crate::char_select::chr_grid_build_with(
            &data.master,
            save,
            crate::char_select::GRID_NO_CUSTOM,
        );
        let rows = (grid.len() as i32 + COLS as i32 - 1) / COLS as i32;
        let mut entry = [Pick::default(); ENTRY_MAX];
        entry[0] = *last;
        let selectable = |c: &Cell| c.id < crate::char_select::ID_LOCKED || c.id == ID_CUSTOM;
        let index = (entry[0].col + entry[0].row * COLS as i32).max(0) as usize;
        if !grid.get(index).is_some_and(selectable) {
            entry[0] = Pick::default();
            *last = entry[0];
        }
        if tour == YAMCHA {
            // The Yamcha Game draws its entrants.
            entry[0] = Pick::default();
            for e in entry
                .iter_mut()
                .take(session.entry_num.clamp(0, ENTRY_MAX as i32) as usize)
            {
                loop {
                    let n = random.range(grid.len() as u32) as usize;
                    let cell = grid[n];
                    if cell.id <= 0xA0 {
                        e.col = (n % COLS) as i32;
                        e.row = (n / COLS) as i32;
                        e.custom_plate = random.range(4) as i32;
                        if cell.form_count != 0 {
                            e.form = random.range(cell.form_count as u32) as i32;
                        }
                        let costumes = chr.costumes.get(cell.id as usize).copied().unwrap_or(1);
                        e.color_plate = random.range(costumes as u32) as i32;
                        break;
                    }
                }
            }
        }
        let mut tex = [
            data.layout.flash[0].clone(),
            data.layout.flash[1].clone(),
            data.layout.flash[2].clone(),
            data.layout.guide[tour as usize].clone(),
        ];
        let mut session = session.clone();
        session.entrants = Default::default();
        let mut screen = Self {
            data: data.clone(),
            bg,
            flash,
            tex: std::mem::take(&mut tex),
            msg,
            panel: ItemPanel::new(0, &data.items, data.layout.panel.clone())?,
            help: ItemHelp::new(&data.items, data.layout.help.clone())?,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            chr: chr.clone(),
            tour,
            entry_num: session.entry_num.clamp(0, ENTRY_MAX as i32) as usize,
            flags: 0,
            timer: 0,
            voice_line: -1,
            load_state: LOAD_SHOWN,
            loading: None,
            help_item: 0,
            end_step: END_NONE,
            entry,
            sel: State::default(),
            grid,
            rows,
            custom_count: 0,
            color_count: 0,
            image_alpha: 0.0,
            talker: 0,
            blink: [0; 2],
            talk: [0; 2],
            result: 1,
            session,
            done: None,
        };
        screen.swap_row_tex();
        screen.sel.image = screen.sel.row_chara[screen.cur().col.clamp(0, 6) as usize];
        for b in &mut screen.blink {
            *b = random.range(0x20) as i32;
        }
        screen.tex[0][PORTRAIT] = loader.portrait(FACE_FILE + screen.sel.image.max(0) as u32);
        screen.sel.flags |= IMAGE_READY;
        Ok(screen)
    }

    /// `EntrySel_Run`'s result once it has returned: 1 the entrants are chosen, 0 backed out.
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// The tournament's block with the entrants handed over.
    pub fn session(&self) -> TourSession {
        self.session.clone()
    }

    fn cur(&self) -> Pick {
        self.entry[self.sel.cur.min(ENTRY_MAX - 1)]
    }

    fn cur_mut(&mut self) -> &mut Pick {
        &mut self.entry[self.sel.cur.min(ENTRY_MAX - 1)]
    }

    /// `ES_CELL`: the grid cell of the entrant being chosen.
    fn cell(&self) -> Cell {
        let c = self.cur();
        self.grid
            .get((c.row * COLS as i32 + c.col).max(0) as usize)
            .copied()
            .unwrap_or_default()
    }

    fn chip(&self, chara: i32) -> Option<usize> {
        usize::try_from(chara + 1)
            .ok()
            .and_then(|i| self.data.layout.chips.get(i - 1).copied())
            .flatten()
    }

    fn set_row(&mut self, chars: [i32; 7]) {
        for (i, &chara) in chars.iter().enumerate() {
            self.sel.row_chara[i] = chara;
            self.tex[1][18 + i] = self.tex[1][CHIP_IMAGES[i]];
            self.tex[1][CHIP_IMAGES[i]] = self.chip(chara);
        }
    }

    /// `EntrySel_SwapRowTex`: the chips take the characters of the cursor's grid row.
    fn swap_row_tex(&mut self) {
        let row = self.cur().row.max(0) as usize;
        let chars =
            std::array::from_fn(|i| self.grid.get(row * COLS + i).map_or(ID_NONE, |c| c.id));
        self.set_row(chars);
    }

    /// `EntrySel_SetRowTex`: the chips take the forms of the cell (empty chips past the last).
    fn set_row_tex(&mut self) {
        let cell = self.cell();
        let chars = std::array::from_fn(|i| {
            if (i as i32) < cell.form_count {
                cell.form[i]
            } else {
                ID_NONE
            }
        });
        self.set_row(chars);
    }

    /// `EntrySel_SetMemberTex`: the chosen entrants' chips.
    fn set_member_tex(&mut self) {
        for i in 0..ENTRY_MAX {
            self.tex[0][10 + i] = if i < self.sel.cur {
                self.chip(self.entry[i].picked)
            } else {
                None
            };
        }
    }

    /// `EntrySel_UpdateImage`: one step of the large picture's loader.
    fn update_image(&mut self, loader: &mut dyn PictureLoader) {
        match self.load_state {
            LOAD_ABORT => self.load_state = LOAD_RESTART,
            LOAD_RESTART => {
                self.sel.flags &= !IMAGE_CHANGE;
                self.load_state = LOAD_REQUEST;
            }
            LOAD_REQUEST => {
                self.loading = loader.portrait(FACE_FILE + self.sel.image.max(0) as u32);
                self.load_state = LOAD_READ;
            }
            // File_UpdateRequests: the read is done by the next call.
            LOAD_READ => self.load_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.tex[0][PORTRAIT] = self.loading;
                self.sel.flags |= IMAGE_READY;
                self.load_state = LOAD_SHOWN;
            }
            LOAD_SHOWN => {
                if self.sel.flags & IMAGE_CHANGE != 0 {
                    self.tex[0][PORTRAIT] = None;
                    self.load_state = LOAD_RESTART;
                }
            }
            _ => {}
        }
    }

    /// `EntrySel_ChangeImage`.
    fn change_image(&mut self) {
        if self.sel.flags & IMAGE_READY != 0 {
            self.sel.flags ^= IMAGE_READY;
        } else {
            self.load_state = LOAD_ABORT;
        }
        self.sel.flags |= IMAGE_CHANGE;
    }

    /// `EntrySel_ClipGoto(movie, kind, label)`.
    fn clip_goto(&mut self, movie: usize, kind: i32, label: &str) {
        let c = self.cur();
        let name = match kind {
            0 => format!("mc_chara_chip_{:03}", c.col),
            1 => format!("mc_chara_chip_{:03}", c.form),
            2 => format!("mc_custom_plate_{}", c.custom_plate + 1),
            5 => format!("mc_color_plate_{}", c.color_plate + 1),
            7 => format!("mc_entry_text_{}", self.sel.cur),
            _ => return,
        };
        let r = self.flash[movie].find(None, &name);
        self.flash[movie].clip_goto_label(r, label);
    }

    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        self.voice_line = line;
        anim::play_with_subtitle(&self.data.lips, VOICE_BASE, line, &mut self.lip, sounds);
    }

    /// One iteration of `EntrySel_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        save: &SaveData,
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
        self.update_image(loader);
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        // EntrySel_Update.
        for f in &mut self.flash {
            f.advance();
        }
        self.panel.update();
        if self.sel.row_anim != 0 && self.flash[1].trig() & 1 != 0 {
            self.sel.row_anim = 0;
        }
        self.update_end(pressed, voice_stat, random, &mut frame.sounds);
        self.draw(voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & GREETED == 0
            && self.flash[0].flags() & FLASH_PAD != 0
        {
            self.flags |= GREETED;
            let line = match self.tour {
                0 => 0x22,
                1 => 0x7A,
                2 => 0x4D,
                3 => 0x9F,
                _ => 0xCA,
            };
            self.talker = 0;
            self.say(line, &mut frame.sounds);
        }
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::FadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            // The picture loader must be idle before the buffers go.
            if self.load_state == LOAD_SHOWN {
                self.done = Some(self.result);
            }
            return frame;
        }
        if self.flags & LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.hand_over();
            }
        } else if self.end_step == END_NONE {
            self.input(pressed, repeat, save, random, &mut frame.sounds);
        }
        frame
    }

    /// The entrants to the tournament (`gProgress->entrant[]`).
    fn hand_over(&mut self) {
        for i in 0..self.entry_num {
            let e = self.entry[i];
            let t = &mut self.session.entrants[i];
            t.chara = e.picked;
            t.costume = e.color_plate;
            t.player = i as i32;
            t.flags |= ENT_PLAYER;
            if e.custom_plate != 0 {
                t.flags |= ENT_ITEMS;
                t.items = e.items;
                let cell = self
                    .grid
                    .get((e.row * COLS as i32 + e.col).max(0) as usize)
                    .map_or(0, |c| c.id);
                if cell == ID_CUSTOM {
                    t.flags |= ENT_REC;
                }
            }
        }
    }

    /// `EntrySel_UpdateEnd`: once every entrant is chosen, the guide's closing line, then the leave timer.
    fn update_end(
        &mut self,
        pressed: u32,
        voice_stat: i32,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        match self.end_step {
            END_SPEAK => {
                let line = match self.tour {
                    0 => {
                        self.talker = 0;
                        0x23
                    }
                    1 => {
                        self.talker = 0;
                        random.range(2) as i32 + 0x7B
                    }
                    2 => {
                        self.talker = 0;
                        random.range(2) as i32 + 0x4E
                    }
                    3 => {
                        self.talker = 0;
                        0xA0
                    }
                    _ => {
                        self.talker = random.range(2) as usize;
                        (self.talker as i32 ^ 1) + 0xCB
                    }
                };
                self.end_step += 1;
                self.say(line, sounds);
            }
            END_WAIT => {
                if voice_stat == VOICE_PLAYEND {
                    self.end_step += 1;
                } else if pressed & PAD_CROSS != 0 {
                    self.end_step += 1;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            END_LEAVE => {
                self.flags |= DONE | LEAVING;
                self.timer = 15;
                self.end_step = END_NONE;
            }
            _ => {}
        }
    }

    /// `EntrySel_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        self.bg.draw(quads);
        let uv = |u0: i32, v0: i32, u1: i32, v1: i32| Uv {
            u0,
            v0,
            u1,
            v1,
            tex: 0,
        };
        if self.sel.flags & IMAGE_READY != 0 {
            self.image_alpha = (self.image_alpha + 0.075).min(1.0);
        } else {
            self.image_alpha = 0.0;
        }
        let mut texts: Vec<TextDraw> = Vec::new();
        {
            let f = &mut self.flash[0];
            let r = f.find(None, "mc_single_chara_l");
            f.clip_set_alpha(r, self.image_alpha);
            let image = self.sel.image;
            let r = f.find(None, "mc_name_text_l");
            texts.extend(TextBox::preset(1).attach_line(
                f,
                r,
                0x400,
                &self.data.names,
                image,
                font,
            ));
            let r = f.find(None, "mc_form_text_l");
            texts.extend(TextBox::preset(3).attach_line(
                f,
                r,
                0x401,
                &self.data.forms,
                image,
                font,
            ));
            let cur = self.sel.cur as i32;
            for i in 0..ENTRY_MAX as i32 {
                let name = format!("mc_entry_text_{i}");
                let cell = uv(
                    (i % 2) * 0x40,
                    (i / 2) * 0x20,
                    (i % 2) * 0x40 + 0x40,
                    (i / 2) * 0x20 + 0x20,
                );
                let r = f.find(None, &name);
                if i >= self.entry_num as i32 || i < cur {
                    f.clip_set_flags(r, PROP_VISIBLE, false);
                } else {
                    f.clip_set_flags(r, PROP_VISIBLE, true);
                    for part in ["mc_entry_text_off", "mc_entry_text_on"] {
                        let r = f.find(Some(&name), part);
                        f.clip_set_uv(r, cell);
                    }
                }
                let r = f.find(None, &format!("mc_entry_decision_text_{i}"));
                if i < cur {
                    f.clip_set_flags(r, PROP_VISIBLE, true);
                    f.clip_set_uv(r, cell);
                } else {
                    f.clip_set_flags(r, PROP_VISIBLE, false);
                }
            }
        }
        let random_cell = self.cell().id == ID_RANDOM;
        {
            let f = &mut self.flash[2];
            let r = f.find(None, "mc_yajirusi");
            f.clip_set_uv(r, uv(0x20, 0, 0x40, 0x20));
            for i in 0..4 {
                let name = format!("mc_custom_plate_{}", i + 1);
                let r = f.find(None, &name);
                f.clip_set_color(r, if i < self.custom_count { 1.0 } else { 0.3 });
                for part in ["mc_custom_text_off", "mc_custom_text_on"] {
                    let r = f.find(Some(&name), part);
                    f.clip_set_uv(r, uv(0, i * 0x20, 0x100, i * 0x20 + 0x20));
                }
                let r = f.find(Some(&name), "mc_yajirusi");
                f.clip_set_flags(r, PROP_VISIBLE, i != 0 && !random_cell);
            }
            for i in 0..4 {
                let name = format!("mc_color_plate_{}", i + 1);
                let r = f.find(None, &name);
                if i < self.color_count {
                    f.clip_set_flags(r, PROP_VISIBLE, true);
                    match self.color_count {
                        2 => f.clip_set_offset(r, 0x2D, 0),
                        3 => f.clip_set_offset(r, 0x16, 0),
                        _ => {}
                    }
                    let cell = uv(
                        (i % 2) * 0x40,
                        (i / 2) * 0x20,
                        (i % 2) * 0x40 + 0x40,
                        (i / 2) * 0x20 + 0x20,
                    );
                    for part in ["mc_color_text_off", "mc_color_text_on"] {
                        let r = f.find(Some(&name), part);
                        f.clip_set_uv(r, cell);
                    }
                } else {
                    f.clip_set_flags(r, PROP_VISIBLE, false);
                }
            }
        }
        {
            let cur = self.sel.cur as i32;
            let n = if self.entry_num as i32 - 1 < cur {
                cur - 1
            } else {
                cur
            };
            let f = &mut self.flash[1];
            let r = f.find(None, "mc_plate_text");
            f.clip_set_uv(
                r,
                uv(
                    (n % 2) * 0x40,
                    (n / 2) * 0x20,
                    (n % 2) * 0x40 + 0x40,
                    (n / 2) * 0x20 + 0x20,
                ),
            );
            let arrows = [
                ("mc_yajirusi_up", "mc_yajirusi_icon_up"),
                ("mc_yajirusi_down", "mc_yajirusi_icon_down"),
            ];
            if self.rows >= 2 {
                for (i, (parent, icon)) in arrows.iter().enumerate() {
                    let i = i as i32;
                    let r = f.find(Some(parent), icon);
                    f.clip_set_uv(r, uv(i * 0x20, 0x20, i * 0x20 + 0x20, 0x40));
                    let hide = self.sel.flags & SEL_FORM != 0 || self.tour == YAMCHA;
                    f.clip_set_flags(r, PROP_VISIBLE, !hide);
                }
            } else {
                for (parent, _) in arrows {
                    let r = f.find(None, parent);
                    f.clip_set_flags(r, PROP_VISIBLE, false);
                }
            }
            let r = f.find(None, "mc_chara_mask");
            f.clip_set_flags(r, 0x102, self.sel.row_anim != 0);
            for i in 0..14 {
                let r = f.find(None, &format!("mc_chara_chip_{i:03}"));
                f.clip_set_flags(r, 0x80, self.sel.row_anim as u8 != 0);
            }
        }
        {
            let f = &mut self.flash[3];
            let talk = |f: &mut Flash,
                        name: &str,
                        timer: &mut i32,
                        lip: &mut LipSync,
                        random: &mut GameRandom| {
                let r = f.find(None, name);
                anim::talk(f, r, timer, 0, lip, voice_stat, random);
            };
            match self.tour {
                0 => talk(
                    f,
                    "mc_guide_tenkaichi_mouth",
                    &mut self.talk[0],
                    &mut self.lip,
                    random,
                ),
                1 | 2 => {
                    let r = f.find(None, "mc_guide_ceru_01_eye");
                    anim::blink(f, r, &mut self.blink[0], 0, random);
                    talk(
                        f,
                        "mc_guide_ceru_01_mouth",
                        &mut self.talk[0],
                        &mut self.lip,
                        random,
                    );
                }
                3 => talk(
                    f,
                    "mc_guide_anoyo_mouth",
                    &mut self.talk[0],
                    &mut self.lip,
                    random,
                ),
                _ => {
                    for i in 0..2 {
                        let eye = if i != 0 {
                            "mc_guide_puaru_eye01"
                        } else {
                            "mc_guide_yamucha_eye"
                        };
                        let r = f.find(None, eye);
                        anim::blink(f, r, &mut self.blink[i], 0, random);
                        let mouth = if i != 0 {
                            "mc_guide_puaru_mouth01"
                        } else {
                            "mc_guide_yamucha_mouth"
                        };
                        if i == self.talker {
                            talk(f, mouth, &mut self.talk[i], &mut self.lip, random);
                        } else {
                            let r = f.find(None, mouth);
                            anim::show_next(f, r, 0);
                        }
                    }
                }
            }
        }
        let draw = |s: &mut Self,
                    i: usize,
                    quads: &mut Vec<ScreenQuad>,
                    font: &mut Font,
                    texts: &[TextDraw]| {
            let tex = &s.tex[i];
            let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
            let (fq, overs) = s.flash[i].draw_with_overs(&has_texture);
            quads.extend(screen_quads(&fq, tex));
            draw_texts(texts, &overs, font);
        };
        draw(self, 0, quads, font, &texts);
        font.flush(quads);
        draw(self, 1, quads, font, &[]);
        draw(self, 2, quads, font, &[]);
        self.msg.draw(self.voice_line, font, quads);
        draw(self, 3, quads, font, &[]);
        self.panel.draw(&self.chr, font, quads);
        self.help
            .draw(self.help_item.max(0) as usize, &self.chr, font, quads);
    }

    /// `ChrTbl_WrapCostume(image, &costume)`.
    fn wrap_costume(&mut self) -> i32 {
        let count = usize::try_from(self.sel.image)
            .ok()
            .and_then(|c| self.chr.costumes.get(c).copied())
            .unwrap_or(1);
        if self.cur().color_plate >= count {
            self.cur_mut().color_plate = 0;
        }
        count
    }

    /// The item set plates: four, or one with `ESEL_NO_CUSTOM`.
    fn open_custom(&mut self) {
        self.custom_count = if self.flags & NO_CUSTOM != 0 { 1 } else { 4 };
        if self.cur().custom_plate >= self.custom_count {
            self.cur_mut().custom_plate = 0;
        }
    }

    /// `EntrySel_Input`.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        save: &SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash[0].flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.clip_goto(1, 0, "fl_on_start");
            self.clip_goto(0, 7, "fl_loop");
            self.flags |= STARTED;
        }
        let free = self.tour != YAMCHA;
        match self.sel.step {
            STEP_CHARA => {
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 && free {
                    self.clip_goto(1, 0, "fl_off_start");
                    let (mut col, row) = (self.cur().col, self.cur().row);
                    if repeat & PAD_LEFT != 0 {
                        chr_grid_move_left(&self.grid, &mut col, row);
                    } else {
                        chr_grid_move_right(&self.grid, &mut col, row);
                    }
                    self.cur_mut().col = col;
                    self.clip_goto(1, 0, "fl_on_start");
                    self.sel.image = self.sel.row_chara[col.clamp(0, 6) as usize];
                    self.change_image();
                    sounds.push(TitleSound::Effect(2, 0));
                } else if repeat & (PAD_UP | PAD_DOWN) != 0 && free {
                    if self.rows >= 2 {
                        let up = repeat & PAD_UP != 0;
                        self.flash[1]
                            .goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                        self.sel.row_anim = 1;
                        self.clip_goto(1, 0, "fl_off_start");
                        let (mut col, mut row) = (self.cur().col, self.cur().row);
                        if up {
                            chr_grid_move_up(&self.grid, &mut col, &mut row, self.rows);
                        } else {
                            chr_grid_move_down(&self.grid, &mut col, &mut row, self.rows);
                        }
                        (self.cur_mut().col, self.cur_mut().row) = (col, row);
                        self.clip_goto(1, 0, "fl_on_start");
                        self.swap_row_tex();
                        self.sel.image = self.sel.row_chara[col.clamp(0, 6) as usize];
                        self.change_image();
                        sounds.push(TitleSound::Effect(2, if up { 2 } else { 1 }));
                    }
                } else if pressed & PAD_CROSS != 0 {
                    let cell = self.cell();
                    if cell.id == ID_RANDOM {
                        // The random cell: no form, two costumes.
                        self.flash[2].goto_label("fl_custom_in", true);
                        self.clip_goto(1, 0, "fl_ok");
                        self.clip_goto(2, 2, "fl_on_start");
                        self.open_custom();
                        self.color_count = 2;
                        if self.cur().color_plate >= self.color_count {
                            self.cur_mut().color_plate = 0;
                        }
                        self.sel.step = STEP_CUSTOM;
                        sounds.push(TitleSound::Effect(1, 1));
                    } else if cell.form_count != 0 {
                        self.sel.flags |= SEL_FORM;
                        if self.cur().form >= cell.form_count {
                            self.cur_mut().form = 0;
                        }
                        self.flash[1].goto_label("fl_form", true);
                        self.sel.row_anim = 1;
                        self.clip_goto(1, 0, "fl_off_start");
                        self.clip_goto(1, 1, "fl_on_start");
                        self.set_row_tex();
                        self.sel.image = self.sel.row_chara[self.cur().form.clamp(0, 6) as usize];
                        self.change_image();
                        self.sel.step = STEP_FORM;
                        sounds.push(TitleSound::Effect(2, 0x29));
                    } else {
                        self.flash[2].goto_label("fl_custom_in", true);
                        self.clip_goto(1, 0, "fl_ok");
                        self.clip_goto(2, 2, "fl_on_start");
                        self.open_custom();
                        self.color_count = self.wrap_costume();
                        self.sel.step = STEP_CUSTOM;
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    if self.sel.cur != 0 {
                        // Back to the previous entrant.
                        self.clip_goto(1, 0, "fl_off_start");
                        self.clip_goto(0, 7, "fl_stop");
                        self.sel.cur -= 1;
                        self.set_member_tex();
                        self.clip_goto(1, 0, "fl_on_start");
                        self.clip_goto(0, 7, "fl_loop");
                        self.swap_row_tex();
                        self.sel.image = self.sel.row_chara[self.cur().col.clamp(0, 6) as usize];
                        self.change_image();
                    } else {
                        self.fade = ColorFade::start_out(true, FADE_FRAMES);
                        self.result = 0;
                    }
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            STEP_FORM => {
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 && free {
                    self.clip_goto(1, 1, "fl_off_start");
                    let mut form = self.cur().form;
                    if repeat & PAD_LEFT != 0 {
                        chr_grid_prev_form(&self.sel.row_chara, &mut form);
                    } else {
                        chr_grid_next_form(&self.sel.row_chara, &mut form);
                    }
                    self.cur_mut().form = form;
                    self.clip_goto(1, 1, "fl_on_start");
                    self.sel.image = self.sel.row_chara[form.clamp(0, 6) as usize];
                    self.change_image();
                    sounds.push(TitleSound::Effect(2, 0));
                } else if pressed & PAD_CROSS != 0 {
                    self.flash[2].goto_label("fl_custom_in", true);
                    self.clip_goto(1, 1, "fl_ok");
                    self.clip_goto(2, 2, "fl_on_start");
                    self.open_custom();
                    self.color_count = self.wrap_costume();
                    self.sel.step = STEP_CUSTOM;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.sel.flags ^= SEL_FORM;
                    self.flash[1].goto_label("fl_form", true);
                    self.sel.row_anim = 1;
                    self.clip_goto(1, 1, "fl_off_start");
                    self.clip_goto(1, 0, "fl_on_start");
                    self.swap_row_tex();
                    self.sel.image = self.sel.row_chara[self.cur().col.clamp(0, 6) as usize];
                    self.change_image();
                    self.sel.step = STEP_CHARA;
                    sounds.push(TitleSound::Effect(2, 0x29));
                }
            }
            STEP_CUSTOM => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 && free {
                    if self.custom_count >= 2 {
                        self.clip_goto(2, 2, "fl_off_start");
                        let n = self.custom_count;
                        let c = &mut self.cur_mut().custom_plate;
                        if repeat & PAD_UP != 0 {
                            *c -= 1;
                            if *c < 0 {
                                *c = n - 1;
                            }
                        } else {
                            *c += 1;
                            if *c >= n {
                                *c = 0;
                            }
                        }
                        self.clip_goto(2, 2, "fl_on_start");
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if pressed & PAD_RIGHT != 0 {
                    // The items of the set.
                    let c = self.cur();
                    if c.custom_plate > 0 && self.cell().id != ID_RANDOM {
                        self.clip_goto(2, 2, "fl_ok");
                        let slot = (c.col + c.row * COLS as i32).max(0) as usize;
                        let set = (c.custom_plate - 1) as usize;
                        let items = save.custom_items.get(slot).map_or([0; 8], |s| s[set]);
                        let level = i32::from(save.custom_level.get(slot).copied().unwrap_or(0));
                        self.panel.set_chara(
                            self.sel.image,
                            items,
                            level,
                            set as i32,
                            false,
                            &self.chr,
                        );
                        self.panel.show(sounds);
                        self.flash[3].goto_label("fl_guide_out", true);
                        self.msg.close();
                        self.sel.step = STEP_PANEL;
                    }
                } else if pressed & PAD_CROSS != 0 {
                    self.flash[2].goto_label("fl_color_in", true);
                    self.clip_goto(2, 2, "fl_ok");
                    self.clip_goto(2, 5, "fl_on_start");
                    self.sel.step = STEP_COLOR;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[2].goto_label("fl_custom_cansel", true);
                    self.clip_goto(2, 2, "fl_off_start");
                    if self.sel.flags & SEL_FORM != 0 {
                        self.clip_goto(1, 1, "fl_on_start");
                        self.sel.step = STEP_FORM;
                    } else {
                        self.clip_goto(1, 0, "fl_on_start");
                        self.sel.step = STEP_CHARA;
                    }
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            STEP_PANEL => {
                let item = self.panel.input(pressed, repeat, sounds);
                if item > 0 {
                    self.help.open();
                    self.help_item = item - 1;
                    self.sel.step = STEP_HELP;
                } else if item < 0 {
                    self.panel.hide(sounds);
                    self.clip_goto(2, 2, "fl_on_start");
                    self.flash[3].goto_label("fl_guide_in", true);
                    self.msg.open();
                    self.sel.step = STEP_CUSTOM;
                }
            }
            STEP_HELP => {
                if pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                    self.help.close();
                    self.sel.step = STEP_PANEL;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            STEP_COLOR => {
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 && free {
                    self.clip_goto(2, 5, "fl_off_start");
                    let n = self.color_count;
                    let c = &mut self.cur_mut().color_plate;
                    if repeat & PAD_LEFT != 0 {
                        *c -= 1;
                        if *c < 0 {
                            *c = n - 1;
                        }
                    } else {
                        *c += 1;
                        if *c >= n {
                            *c = 0;
                        }
                    }
                    self.clip_goto(2, 5, "fl_on_start");
                    sounds.push(TitleSound::Effect(1, 0));
                } else if pressed & PAD_CROSS != 0 {
                    self.decide(save, random, sounds);
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[2].goto_label("fl_color_cansel", true);
                    self.clip_goto(2, 5, "fl_off_start");
                    self.clip_goto(2, 2, "fl_on_start");
                    self.sel.step = STEP_CUSTOM;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            _ => {}
        }
    }

    /// The costume is confirmed: the entrant's character is fixed (the random cell draws one) and its item set
    /// taken along; then the next entrant, or the closing line.
    fn decide(&mut self, save: &SaveData, random: &mut GameRandom, sounds: &mut Vec<TitleSound>) {
        self.sel.row_anim = 0;
        sounds.push(TitleSound::Effect(1, 1));
        self.flash[2].goto_label("fl_color_ok", true);
        self.clip_goto(2, 5, "fl_ok");
        self.clip_goto(0, 7, "fl_stop");
        self.clip_goto(2, 2, "fl_off_start");
        let c = self.cur();
        let set_items = |slot: usize, custom: i32| -> [u16; 8] {
            if custom != 0 {
                save.custom_items
                    .get(slot)
                    .map_or([0; 8], |s| s[(custom - 1) as usize])
            } else {
                [0; 8]
            }
        };
        if self.sel.flags & SEL_FORM != 0 {
            let chara = self.sel.row_chara[c.form.clamp(0, 6) as usize];
            let items = if self.cell().id == ID_CUSTOM {
                save.rec_items
                    .get((c.custom_col + c.custom_row * 7).max(0) as usize)
                    .copied()
                    .unwrap_or_default()
            } else {
                set_items(
                    (c.row * COLS as i32 + c.col).max(0) as usize,
                    c.custom_plate,
                )
            };
            let e = self.cur_mut();
            e.picked = chara;
            e.items = items;
            self.clip_goto(1, 1, "fl_off_start");
        } else if self.sel.flags & SEL_REC != 0 {
            // Never reached: nothing sets the flag (and clip kind 6 names no clip).
            let chara = self.sel.row_chara[c.custom_col.clamp(0, 6) as usize];
            let items = save
                .rec_items
                .get((c.custom_col + c.custom_row * 7).max(0) as usize)
                .copied()
                .unwrap_or_default();
            let e = self.cur_mut();
            e.picked = chara;
            e.items = items;
        } else {
            let (chara, slot) = if self.cell().id == ID_RANDOM {
                let (slot, id) = loop {
                    let slot = random.range(self.grid.len() as u32) as usize;
                    let id = self.grid[slot].id;
                    if id < ID_RANDOM {
                        break (slot, id);
                    }
                };
                let cell = self.grid[slot];
                let chara = if cell.form_count != 0 {
                    cell.form[random.range(cell.form_count as u32) as usize]
                } else {
                    id
                };
                (chara, slot)
            } else {
                (
                    self.sel.row_chara[c.col.clamp(0, 6) as usize],
                    (c.col + c.row * COLS as i32).max(0) as usize,
                )
            };
            let items = set_items(slot, c.custom_plate);
            let e = self.cur_mut();
            e.picked = chara;
            e.items = items;
            self.clip_goto(1, 0, "fl_off_start");
        }
        self.sel.flags &= !(SEL_FORM | SEL_REC);
        self.sel.cur += 1;
        self.clip_goto(0, 7, "fl_loop");
        self.set_member_tex();
        if self.sel.cur == self.entry_num {
            self.flash[1].goto_label("fl_out", true);
            self.end_step = END_SPEAK;
        } else {
            self.clip_goto(1, 0, "fl_on_start");
            self.swap_row_tex();
            self.sel.image = self.sel.row_chara[self.cur().col.clamp(0, 6) as usize];
            self.change_image();
            self.sel.step = STEP_CHARA;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Pictures;

    impl PictureLoader for Pictures {
        fn portrait(&mut self, file_id: u32) -> Option<usize> {
            Some(file_id as usize)
        }

        fn picture(&mut self, _: u32) -> Option<usize> {
            None
        }
    }

    /// Runs the real entrant select from the ISO in `rom/`, when it is there: two entrants of the World
    /// Tournament chosen (through a cell's forms when it has them, the item set and the costume), the guide's
    /// closing line, and the entrants handed to the tournament.
    #[test]
    fn real_entrant_select_hands_two_entrants_over() {
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
        let chr = ChrTable::parse(&read(4), 0xA1);
        let pack = read(crate::tour_menu::ARCHIVE_FILE_ID);
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = EntrySelAssets::parse(&res).unwrap();
        let bg_res = bpe::decode(&read(crate::tour_bg::FILE_FIRST)).unwrap();
        let bg_assets = crate::tour_bg::TourBgAssets::parse(&bg_res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = EntrySelLayout::new(|k| firsts[&k]);
        let bg = crate::tour_bg::TourBg::new(
            &crate::tour_bg::TourBgData {
                fod: bg_assets.fod,
                background: 0,
                first: 10,
            },
            0,
            None,
        )
        .unwrap();
        let session = TourSession {
            entry_num: 2,
            ..TourSession::default()
        };
        let save = SaveData::default();
        let mut last = Pick::default();
        let mut random = GameRandom::from_key(&[8, 8]);
        let mut screen = EntrySelScreen::new(
            &data,
            bg,
            &session,
            &chr,
            &save,
            &mut last,
            &mut Pictures,
            &mut random,
        )
        .unwrap();
        assert!(screen.rows >= 2);
        let mut font = Font::new(Vec::new(), 20);
        let mut press = |screen: &mut EntrySelScreen, bits: u32| {
            for i in 0..40 {
                let b = if i == 0 { bits } else { 0 };
                screen.run_frame(
                    b,
                    b,
                    VOICE_PLAYEND,
                    &save,
                    &mut Pictures,
                    &mut random,
                    &mut font,
                );
                if screen.result().is_some() {
                    break;
                }
            }
        };
        for _ in 0..4 {
            press(&mut screen, 0);
        }
        assert!(screen.flags & GREETED != 0);
        for n in 0..2 {
            // One cell to the right for the second entrant.
            if n == 1 {
                press(&mut screen, PAD_RIGHT);
            }
            for _ in 0..4 {
                if screen.sel.step == STEP_COLOR {
                    break;
                }
                press(&mut screen, PAD_CROSS);
            }
            assert_eq!(screen.sel.step, STEP_COLOR);
            press(&mut screen, PAD_CROSS);
            assert_eq!(screen.sel.cur, n + 1);
        }
        for _ in 0..10 {
            press(&mut screen, 0);
        }
        assert_eq!(screen.result(), Some(1));
        let s = screen.session();
        for i in 0..2 {
            let e = s.entrants[i];
            assert_eq!(e.flags & ENT_PLAYER, ENT_PLAYER);
            assert_eq!(e.player, i as i32);
            assert!((0..=0xA0).contains(&e.chara), "{e:?}");
        }
        assert_ne!(s.entrants[0].chara, s.entrants[1].chara);
        assert_eq!(s.entrants[2], Default::default());
    }
}
