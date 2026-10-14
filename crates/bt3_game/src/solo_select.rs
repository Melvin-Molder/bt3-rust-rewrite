//! Ultimate Battle's one-fighter select (`SoloSel_Run(4)`, src/menu/solo_select.c, progress modes 15, 18, 21, 25
//! and 29, `Ub_Main`): Androids 17 and 18 as guides, the character grid on a reel of chips with the streamed
//! portrait of the character under the cursor, a cell's forms, the item set (only "no items" in Sim Dragon) with
//! its panel and details page, and the costume. The choice is kept in `gProgress` (the versus select's side 0 pick,
//! the group's first fighter).

use std::error::Error;

use bt3_data::chara_data::ChrTable;
use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{
    COLS, Cell, GRID_NO_CUSTOM, GRID_NO_FORMS, GRID_NO_RANDOM, ID_CUSTOM, ID_LOCKED, Pick,
    PictureLoader, chr_grid_build_with, chr_grid_move_down, chr_grid_move_left,
    chr_grid_move_right, chr_grid_move_up, chr_grid_next_form, chr_grid_prev_form,
};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::item_panel::{ItemAssets, ItemHelp, ItemPanel, help_textures_in, panel_textures_in};
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::ub::VOICE_BASE;
use crate::windows::{MsgWin, TextBox, TextDraw, draw_texts};

/// `SoloSel_Run(4)`: section 4 of `gMenuArc3`.
pub const ARCHIVE_SECTION: usize = 4;
const FACE_FILE: u32 = 0x2F9;
const FADE_FRAMES: i32 = 0x14;
const CHIP_COUNT: usize = 165;
/// The reel's chip images (`SoloSel_SwapRowTex`'s order).
const CHIP_IMAGES: [usize; 7] = [8, 11, 12, 13, 14, 15, 16];
const PORTRAIT: usize = 17;
/// `LCHR_ID_NONE`: an empty chip.
const ID_NONE: i32 = 0xA4;
/// Sim Dragon's fighter select.
const MODE_SIM: i32 = 21;

// SOLOSEL_STEP_*.
const STEP_CHARA: i32 = 0;
const STEP_FORM: i32 = 1;
const STEP_CUSTOM: i32 = 2;
const STEP_PANEL: i32 = 3;
const STEP_HELP: i32 = 4;
const STEP_COLOR: i32 = 5;

// SOLOSEL_*.
const STARTED: u32 = 2;
const DONE: u32 = 8;
const LEAVING: u32 = 0x10;
const NO_CUSTOM: u32 = 0x40;
const GREETED: u32 = 0x80;
// SOLOSEL_SEL_*.
const IMAGE_CHANGE: u32 = 1;
const IMAGE_READY: u32 = 2;
const SEL_FORM: u32 = 0x40;
const SEL_PANEL: u32 = 0x800;

// SOLOSEL_LOAD_*.
const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;
const LOAD_SHOWN: i32 = 4;
const LOAD_ABORT: i32 = 5;
const LOAD_RESTART: i32 = 6;

// SOLOSEL_END_*.
const END_NONE: i32 = 0;
const END_SPEAK: i32 = 1;
const END_WAIT: i32 = 2;
const END_LEAVE: i32 = 3;

/// `texA[n]` (movie 0: the picture and the guides; 17 the portrait).
const TEX_A: [ImageTexture; 20] = [
    tex(33, 0),
    tex(33, 1),
    tex(34, 0),
    tex(34, 1),
    tex(34, 3),
    None,
    None,
    None,
    tex(1, 0),
    tex(2, 0),
    None,
    tex(33, 2),
    tex(33, 3),
    tex(33, 4),
    tex(34, 5),
    tex(34, 6),
    tex(34, 8),
    None,
    tex(2, 2),
    None,
];
/// `texB[n]` (movie 1: the reel; 8, 11..16 the chips, 18..24 those before the last change).
const TEX_B: [ImageTexture; 25] = [
    tex(6, 0),
    tex(7, 1),
    tex(6, 1),
    tex(9, 0),
    tex(7, 2),
    tex(7, 4),
    tex(7, 3),
    tex(8, 0),
    None,
    None,
    tex(10, 0),
    None,
    None,
    None,
    None,
    None,
    None,
    tex(10, 1),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];
/// `texC[n]` (movie 2: the item set and costume plates).
const TEX_C: [ImageTexture; 11] = [
    tex(12, 1),
    tex(16, 1),
    tex(14, 0),
    tex(13, 1),
    tex(10, 1),
    tex(14, 1),
    tex(15, 0),
    tex(13, 0),
    tex(15, 1),
    tex(12, 0),
    tex(16, 0),
];
/// `texB[n]` of the team select's team movie (section 21): 3 and 0 the plates, 8..12 the frames; 1, 4..7 the
/// members' chips; 2 the DP plate.
pub(crate) const TEX_TEAM: [ImageTexture; 13] = [
    tex(18, 0),
    None,
    None,
    tex(10, 0),
    None,
    None,
    None,
    None,
    tex(19, 0),
    tex(20, 0),
    tex(19, 2),
    tex(19, 1),
    tex(20, 1),
];
/// The team select's textures of a DP limit: `texA[5..7]` and the DP plate of the reel and the team movie.
pub(crate) const TEX_DP: [ImageTexture; 4] = [tex(3, 0), tex(4, 0), tex(4, 1), tex(5, 2)];
const PANEL_SECTION: usize = 22;
const HELP_SECTION: usize = 24;
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((23, 1), 0)), Some(((23, 1), 1)), None];

fn chip_textures() -> Vec<ImageTexture> {
    (1..=CHIP_COUNT).map(|n| Some(((27, n), 0))).collect()
}

/// What `SoloSel_Init` reads from its unpacked section.
pub struct SoloSelectAssets {
    pub data: SoloSelectData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct SoloSelectData {
    /// Sections 31, 11, 17: the picture and guides, the reel, the plates; 21 the team select's team movie.
    pub fods: [Vec<u8>; 3],
    pub team_fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    /// Section 25: the guides' lines; 26: their mouth keys; 28 / 29: character and form names.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    pub names: Vec<u8>,
    pub forms: Vec<u8>,
    /// Section 30: the master character grid.
    pub(crate) master: Vec<Cell>,
    pub items: ItemAssets,
    pub layout: SoloSelectLayout,
}

#[derive(Clone, Debug, Default)]
pub struct SoloSelectLayout {
    pub background: usize,
    pub flash: [Vec<Option<usize>>; 3],
    pub team: Vec<Option<usize>>,
    pub dp: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub panel: Vec<Option<usize>>,
    pub help: Vec<Option<usize>>,
    pub chips: Vec<Option<usize>>,
}

impl SoloSelectLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        let i = |t: &[ImageTexture]| image_indices(t, &first);
        Self {
            background,
            flash: [i(&TEX_A), i(&TEX_B), i(&TEX_C)],
            team: i(&TEX_TEAM),
            dp: i(&TEX_DP),
            msg: i(&MSG_TEXTURES),
            panel: i(&panel_textures_in(PANEL_SECTION, 0)),
            help: i(&help_textures_in(HELP_SECTION)),
            chips: i(&chip_textures()),
        }
    }
}

impl SoloSelectAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let panel = panel_textures_in(PANEL_SECTION, 0);
        let help = help_textures_in(HELP_SECTION);
        let chips = chip_textures();
        let files = decode_files(
            res,
            [
                &TEX_A[..],
                &TEX_B,
                &TEX_C,
                &TEX_TEAM,
                &TEX_DP,
                &MSG_TEXTURES,
                &panel,
                &help,
                &chips,
            ],
        )?;
        let sec =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        let grid = pack_section(res, 30)?;
        let word = |at: usize| -> Result<i32, Box<dyn Error>> {
            Ok(i32::from_le_bytes(
                grid.get(at..at + 4).ok_or("short grid")?.try_into()?,
            ))
        };
        let master = (0..word(0)?.max(0) as usize)
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
            data: SoloSelectData {
                fods: [sec(31)?, sec(11)?, sec(17)?],
                team_fod: sec(21)?,
                msg_fod: pack_section(pack_section(res, 23)?, 2)?.to_vec(),
                msg_text: sec(25)?,
                lips: sec(26)?,
                names: sec(28)?,
                forms: sec(29)?,
                master,
                items: ItemAssets::parse_in(res, PANEL_SECTION, HELP_SECTION)?,
                layout: SoloSelectLayout::default(),
            },
            background: decode_picture(res, 32)?,
            files,
        })
    }
}

/// `SoloSelState`: the choice being made.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    entry: Pick,
    row_chara: [i32; 7],
    flags: u32,
    image: i32,
    step: i32,
    row_anim: i32,
}

/// `SoloSel` and the frame loop of `SoloSel_Run`.
pub struct SoloSelectScreen {
    data: SoloSelectData,
    flash: [Flash; 3],
    tex: [Vec<Option<usize>>; 3],
    msg: MsgWin,
    panel: ItemPanel,
    help: ItemHelp,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    chr: ChrTable,
    mode: i32,
    flags: u32,
    voice_line: i32,
    load_state: i32,
    loading: Option<usize>,
    timer: i32,
    help_item: i32,
    sel: State,
    grid: Vec<Cell>,
    rows: i32,
    custom_count: i32,
    color_count: i32,
    image_alpha: f32,
    end_step: i32,
    talker: usize,
    blink: [i32; 2],
    talk: [i32; 2],
    result: i32,
    done: Option<i32>,
}

impl SoloSelectScreen {
    /// `SoloSel_Init(4)` in progress mode `mode` and the first `ColorFade_StartIn`. `last` is `gProgress + 0x440`,
    /// the last choice (reset when its cell cannot be chosen).
    pub fn new(
        data: &SoloSelectData,
        mode: i32,
        chr: &ChrTable,
        save: &SaveData,
        last: &mut Pick,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let make = |fod: &[u8]| -> Result<Flash, Box<dyn Error>> {
            let mut f = Flash::create(fod)?;
            f.play(1);
            Ok(f)
        };
        let flash = [
            make(&data.fods[0])?,
            make(&data.fods[1])?,
            make(&data.fods[2])?,
        ];
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            true,
        )?;
        msg.open();
        // ChrGrid_Build in modes 13..30: no custom character nor random cell; mode 21 no forms either.
        let mut grid_flags = GRID_NO_CUSTOM | GRID_NO_RANDOM;
        if mode == MODE_SIM {
            grid_flags |= GRID_NO_FORMS;
        }
        let (grid, _, _) = chr_grid_build_with(&data.master, save, grid_flags);
        let rows = (grid.len() as i32 + COLS as i32 - 1) / COLS as i32;
        let mut sel = State {
            entry: *last,
            ..State::default()
        };
        let selectable = |c: &Cell| c.id < ID_LOCKED || c.id == ID_CUSTOM;
        let index = (sel.entry.col + sel.entry.row * COLS as i32).max(0) as usize;
        if !grid.get(index).is_some_and(selectable) {
            sel.entry = Pick::default();
            *last = sel.entry;
        }
        let mut screen = Self {
            data: data.clone(),
            flash,
            tex: data.layout.flash.clone(),
            msg,
            panel: ItemPanel::new(0, &data.items, data.layout.panel.clone())?,
            help: ItemHelp::new(&data.items, data.layout.help.clone())?,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            chr: chr.clone(),
            mode,
            flags: if mode == MODE_SIM { NO_CUSTOM } else { 0 },
            voice_line: -1,
            load_state: LOAD_SHOWN,
            loading: None,
            timer: 0,
            help_item: 0,
            sel,
            grid,
            rows,
            custom_count: 0,
            color_count: 0,
            image_alpha: 0.0,
            end_step: END_NONE,
            talker: 0,
            blink: [0; 2],
            talk: [0; 2],
            result: 1,
            done: None,
        };
        screen.swap_row_tex();
        screen.sel.image = screen.sel.row_chara[screen.sel.entry.col.clamp(0, 6) as usize];
        for b in &mut screen.blink {
            *b = random.range(0x20) as i32;
        }
        screen.tex[0][PORTRAIT] = loader.portrait(FACE_FILE + screen.sel.image.max(0) as u32);
        screen.sel.flags |= IMAGE_READY;
        Ok(screen)
    }

    /// `SoloSel_Run`'s result once it has returned: 1 a fighter was chosen, 0 backed out.
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// The choice (`gProgress + 0x440`) as it was handed over.
    pub fn pick(&self) -> Pick {
        self.sel.entry
    }

    /// `SS_CELL`: the grid cell under the cursor.
    fn cell(&self) -> Cell {
        let c = self.sel.entry;
        self.grid
            .get((c.row * COLS as i32 + c.col).max(0) as usize)
            .copied()
            .unwrap_or_default()
    }

    fn chip(&self, chara: i32) -> Option<usize> {
        usize::try_from(chara)
            .ok()
            .and_then(|c| self.data.layout.chips.get(c).copied())
            .flatten()
    }

    fn set_row(&mut self, chars: [i32; 7]) {
        for (i, &chara) in chars.iter().enumerate() {
            self.sel.row_chara[i] = chara;
            self.tex[1][18 + i] = self.tex[1][CHIP_IMAGES[i]];
            self.tex[1][CHIP_IMAGES[i]] = self.chip(chara);
        }
    }

    /// `SoloSel_SwapRowTex`: the chips take the characters of the cursor's grid row.
    fn swap_row_tex(&mut self) {
        let row = self.sel.entry.row.max(0) as usize;
        let chars =
            std::array::from_fn(|i| self.grid.get(row * COLS + i).map_or(ID_NONE, |c| c.id));
        self.set_row(chars);
    }

    /// `SoloSel_SetRowTex`: the chips take the forms of the cell (empty chips past the last).
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

    /// `SoloSel_UpdateImage`: one step of the large picture's loader.
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

    /// `SoloSel_ChangeImage`.
    fn change_image(&mut self) {
        if self.sel.flags & IMAGE_READY != 0 {
            self.sel.flags ^= IMAGE_READY;
        } else {
            self.load_state = LOAD_ABORT;
        }
        self.sel.flags |= IMAGE_CHANGE;
    }

    /// `SoloSel_ClipGoto(movie, kind, label)`.
    fn clip_goto(&mut self, movie: usize, kind: i32, label: &str) {
        let c = self.sel.entry;
        let name = match kind {
            0 => format!("mc_chara_chip_{:03}", c.col),
            1 => format!("mc_chara_chip_{:03}", c.form),
            2 => format!("mc_custom_plate_{}", c.custom_plate + 1),
            5 => format!("mc_color_plate_{}", c.color_plate + 1),
            _ => return,
        };
        let r = self.flash[movie].find(None, &name);
        self.flash[movie].clip_goto_label(r, label);
    }

    fn say(&mut self, line: i32, talker: usize, sounds: &mut Vec<TitleSound>) {
        self.talker = talker;
        self.voice_line = line;
        anim::play_with_subtitle(&self.data.lips, VOICE_BASE, line, &mut self.lip, sounds);
    }

    /// One iteration of `SoloSel_Run`'s loop.
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
        // SoloSel_Update.
        for f in &mut self.flash {
            f.advance();
        }
        self.panel.update();
        if self.sel.row_anim != 0 && self.flash[1].trig() & 1 != 0 {
            self.sel.row_anim = 0;
        }
        self.update_end(pressed, voice_stat, &mut frame.sounds);
        self.draw(voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & GREETED == 0
            && self.flash[0].flags() & FLASH_PAD != 0
        {
            self.flags |= GREETED;
            if self.mode == MODE_SIM {
                self.say(0x16, 1, &mut frame.sounds);
            } else {
                self.say(0x36, 0, &mut frame.sounds);
            }
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
            }
        } else if self.end_step == END_NONE {
            self.input(pressed, repeat, save, &mut frame.sounds);
        }
        frame
    }

    /// `SoloSel_UpdateEnd`: once the fighter is chosen, the guide's closing line, then the leave timer.
    fn update_end(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        match self.end_step {
            END_SPEAK => {
                if self.mode == MODE_SIM {
                    let line = if self.sel.image == 0x66 { 0x18 } else { 0x17 };
                    self.say(line, 1, sounds);
                } else {
                    self.say(0x37, 0, sounds);
                }
                self.end_step += 1;
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

    /// `SoloSel_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        quads.extend(
            picture_quads(self.data.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let uv = |u0: i32, v0: i32, u1: i32, v1: i32| Uv {
            u0,
            v0,
            u1,
            v1,
            tex: 0,
        };
        let mut texts: Vec<TextDraw> = Vec::new();
        {
            let f = &mut self.flash[0];
            for k in 0..2 {
                // The two guides: both blink, the one talking moves its mouth.
                let name = format!("mc_guide_{:02}go", k + 17);
                let r = f.find(Some(&name), &format!("mc_guide_{:02}go_eye", k + 17));
                anim::blink(f, r, &mut self.blink[k], 0, random);
                let r = f.find(Some(&name), &format!("mc_guide_{:02}go_mouth", k + 17));
                if self.talker == k {
                    anim::talk(
                        f,
                        r,
                        &mut self.talk[k],
                        0,
                        &mut self.lip,
                        voice_stat,
                        random,
                    );
                } else {
                    anim::show_next(f, r, 0);
                }
            }
            if self.sel.flags & IMAGE_READY != 0 {
                self.image_alpha = (self.image_alpha + 0.075).min(1.0);
            } else {
                self.image_alpha = 0.0;
            }
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
        }
        {
            let f = &mut self.flash[2];
            let r = f.find(None, "mc_yajirusi");
            f.clip_set_uv(r, uv(0x20, 0, 0x40, 0x20));
            for i in 0..4 {
                // The item set plates: dimmed when they cannot be chosen.
                let name = format!("mc_custom_plate_{}", i + 1);
                let r = f.find(None, &name);
                f.clip_set_color(r, if i < self.custom_count { 1.0 } else { 0.3 });
                for part in ["mc_custom_text_off", "mc_custom_text_on"] {
                    let r = f.find(Some(&name), part);
                    f.clip_set_uv(r, uv(0, i * 0x20, 0x100, i * 0x20 + 0x20));
                }
                if i == 0 {
                    let r = f.find(Some(&name), "mc_yajirusi");
                    f.clip_set_flags(r, PROP_VISIBLE, false);
                }
            }
            for i in 0..4 {
                // The costume plates.
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
            let f = &mut self.flash[1];
            let r = f.find(None, "mc_plate_text");
            f.clip_set_uv(r, uv(0, 0, 0x40, 0x20));
            let arrows = [
                ("mc_yajirusi_up", "mc_yajirusi_icon_up"),
                ("mc_yajirusi_down", "mc_yajirusi_icon_down"),
            ];
            if self.rows >= 2 {
                for (i, (parent, icon)) in arrows.iter().enumerate() {
                    let i = i as i32;
                    let r = f.find(Some(parent), icon);
                    f.clip_set_uv(r, uv(i * 0x20, 0x20, i * 0x20 + 0x20, 0x40));
                    let r = f.find(None, parent);
                    f.clip_set_flags(r, PROP_VISIBLE, self.sel.flags & SEL_FORM == 0);
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
        let draw = |s: &Self, i: usize, quads: &mut Vec<ScreenQuad>| {
            let tex = &s.tex[i];
            let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
            let (fq, overs) = s.flash[i].draw_with_overs(&has_texture);
            quads.extend(screen_quads(&fq, tex));
            overs
        };
        // The names are queued with movie 0 and flushed over the reel (TextBox_SetNoFlush).
        let overs = draw(self, 0, quads);
        draw_texts(&texts, &overs, font);
        draw(self, 1, quads);
        font.flush(quads);
        draw(self, 2, quads);
        self.msg.draw(self.voice_line, font, quads);
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
        if self.sel.entry.color_plate >= count {
            self.sel.entry.color_plate = 0;
        }
        count
    }

    /// The item set plates: four, or one in Sim Dragon.
    fn open_custom(&mut self) {
        self.custom_count = if self.flags & NO_CUSTOM != 0 { 1 } else { 4 };
        if self.sel.entry.custom_plate >= self.custom_count {
            self.sel.entry.custom_plate = 0;
        }
    }

    /// `SoloSel_Input`.
    fn input(&mut self, pressed: u32, repeat: u32, save: &SaveData, sounds: &mut Vec<TitleSound>) {
        if self.flash[0].flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.clip_goto(1, 0, "fl_on_start");
            self.flags |= STARTED;
        }
        match self.sel.step {
            STEP_CHARA => {
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.clip_goto(1, 0, "fl_off_start");
                    let (mut col, row) = (self.sel.entry.col, self.sel.entry.row);
                    if repeat & PAD_LEFT != 0 {
                        chr_grid_move_left(&self.grid, &mut col, row);
                    } else {
                        chr_grid_move_right(&self.grid, &mut col, row);
                    }
                    self.sel.entry.col = col;
                    self.clip_goto(1, 0, "fl_on_start");
                    self.sel.image = self.sel.row_chara[col.clamp(0, 6) as usize];
                    self.change_image();
                    sounds.push(TitleSound::Effect(2, 0));
                } else if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    if self.rows >= 2 {
                        let up = repeat & PAD_UP != 0;
                        self.flash[1]
                            .goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                        self.sel.row_anim = 1;
                        self.clip_goto(1, 0, "fl_off_start");
                        let (mut col, mut row) = (self.sel.entry.col, self.sel.entry.row);
                        if up {
                            chr_grid_move_up(&self.grid, &mut col, &mut row, self.rows);
                        } else {
                            chr_grid_move_down(&self.grid, &mut col, &mut row, self.rows);
                        }
                        (self.sel.entry.col, self.sel.entry.row) = (col, row);
                        self.clip_goto(1, 0, "fl_on_start");
                        self.swap_row_tex();
                        self.sel.image = self.sel.row_chara[col.clamp(0, 6) as usize];
                        self.change_image();
                        sounds.push(TitleSound::Effect(2, if up { 2 } else { 1 }));
                    }
                } else if pressed & PAD_CROSS != 0 {
                    let cell = self.cell();
                    if cell.form_count != 0 {
                        self.sel.flags |= SEL_FORM;
                        if self.sel.entry.form >= cell.form_count {
                            self.sel.entry.form = 0;
                        }
                        self.flash[1].goto_label("fl_form", true);
                        self.sel.row_anim = 1;
                        self.clip_goto(1, 0, "fl_off_start");
                        self.clip_goto(1, 1, "fl_on_start");
                        self.set_row_tex();
                        self.sel.image =
                            self.sel.row_chara[self.sel.entry.form.clamp(0, 6) as usize];
                        self.change_image();
                        self.sel.step = STEP_FORM;
                        sounds.push(TitleSound::Effect(2, 0x29));
                    } else {
                        self.flash[2].goto_label("fl_custom_in", true);
                        self.clip_goto(1, 0, "fl_ok");
                        self.open_custom();
                        self.color_count = self.wrap_costume();
                        self.clip_goto(2, 2, "fl_on_start");
                        self.sel.step = STEP_CUSTOM;
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.result = 0;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            STEP_FORM => {
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.clip_goto(1, 1, "fl_off_start");
                    let mut form = self.sel.entry.form;
                    if repeat & PAD_LEFT != 0 {
                        chr_grid_prev_form(&self.sel.row_chara, &mut form);
                    } else {
                        chr_grid_next_form(&self.sel.row_chara, &mut form);
                    }
                    self.sel.entry.form = form;
                    self.clip_goto(1, 1, "fl_on_start");
                    self.sel.image = self.sel.row_chara[form.clamp(0, 6) as usize];
                    self.change_image();
                    sounds.push(TitleSound::Effect(2, 0));
                } else if pressed & PAD_CROSS != 0 {
                    self.flash[2].goto_label("fl_custom_in", true);
                    self.clip_goto(1, 1, "fl_ok");
                    self.open_custom();
                    self.clip_goto(2, 2, "fl_on_start");
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
                    self.sel.image = self.sel.row_chara[self.sel.entry.col.clamp(0, 6) as usize];
                    self.change_image();
                    self.sel.step = STEP_CHARA;
                    sounds.push(TitleSound::Effect(2, 0x29));
                }
            }
            STEP_CUSTOM => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    if self.custom_count >= 2 {
                        self.clip_goto(2, 2, "fl_off_start");
                        let n = self.custom_count;
                        let c = &mut self.sel.entry.custom_plate;
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
                    let c = self.sel.entry;
                    if c.custom_plate > 0 {
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
                        self.sel.flags |= SEL_PANEL;
                        self.sel.step = STEP_PANEL;
                        sounds.push(TitleSound::Effect(1, 0));
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
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.clip_goto(2, 5, "fl_off_start");
                    let n = self.color_count;
                    let c = &mut self.sel.entry.color_plate;
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
                    // The fighter is decided: its character fixed, the item set taken along.
                    self.sel.row_anim = 0;
                    self.flash[2].goto_label("fl_color_ok", true);
                    self.flash[1].goto_label("fl_out", true);
                    self.clip_goto(2, 5, "fl_ok");
                    self.clip_goto(2, 2, "fl_off_start");
                    let c = self.sel.entry;
                    let slot = (c.col + c.row * COLS as i32).max(0) as usize;
                    let items = if c.custom_plate != 0 {
                        save.custom_items
                            .get(slot)
                            .map_or([0; 8], |s| s[(c.custom_plate - 1) as usize])
                    } else {
                        [0; 8]
                    };
                    let form = self.sel.flags & SEL_FORM != 0;
                    let at = if form { c.form } else { c.col };
                    self.sel.entry.picked = self.sel.row_chara[at.clamp(0, 6) as usize];
                    self.sel.entry.items = items;
                    self.clip_goto(1, i32::from(form), "fl_off_start");
                    self.end_step = END_SPEAK;
                    sounds.push(TitleSound::Effect(1, 1));
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

    /// Runs the real fighter select from the ISO in `rom/`, when it is there: the guide's greeting, one cell to
    /// the right, through its forms when it has them, the item set and costume, the closing line, and the choice
    /// handed over.
    #[test]
    fn real_solo_select_chooses_a_fighter() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
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
        let pack = read(crate::ub::ARCHIVE_FILE_ID);
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = SoloSelectAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = SoloSelectLayout::new(0, |k| firsts[&k]);
        let save = SaveData::default();
        let mut last = Pick::default();
        let mut random = GameRandom::from_key(&[4, 4]);
        let mut screen = SoloSelectScreen::new(
            &data,
            15,
            &chr,
            &save,
            &mut last,
            &mut Pictures,
            &mut random,
        )
        .unwrap();
        // No random cell in this group.
        assert!(
            screen
                .grid
                .iter()
                .all(|c| c.id != crate::char_select::ID_RANDOM)
        );
        let mut font = Font::new(Vec::new(), 20);
        let mut press = |screen: &mut SoloSelectScreen, bits: u32| {
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
        assert_eq!(screen.voice_line, 0x36);
        press(&mut screen, PAD_RIGHT);
        for _ in 0..4 {
            if screen.sel.step == STEP_COLOR {
                break;
            }
            press(&mut screen, PAD_CROSS);
        }
        assert_eq!(screen.sel.step, STEP_COLOR);
        press(&mut screen, PAD_CROSS);
        assert_eq!(screen.voice_line, 0x37);
        for _ in 0..6 {
            press(&mut screen, 0);
        }
        assert_eq!(screen.result(), Some(1));
        let pick = screen.pick();
        assert!((0..=0xA0).contains(&pick.picked), "{pick:?}");
        assert_eq!(pick.col, 1);
    }
}
