//! The team character / stage / music select of the versus modes (`TeamSel_Run(2)`, src/menu/team_select.c,
//! progress mode 40): the team version of the character select, from the same section of the duel archive. Each
//! side builds a team of up to five members, each chosen on the character reel (forms, saved custom characters,
//! item-set plates, costumes) from the side's member list (movies 1 and 2), never two forms of the same person;
//! in a DP battle (battle type 2) each character has a cost and the team's total may not pass 10 / 15 / 20. Then
//! pad 0 chooses the stage and the music, and the screen writes the battle setup (member 0 the lead).
//!
//! One pad (1P vs COM, COM vs COM) chooses side 0's team, then side 1's (`base`); two pads choose at once. The
//! choices are kept for the next visit in the character select's progress (its pick is member 0), but only the
//! cursors: every member starts empty.

use std::error::Error;

use crate::char_select::{
    BGM_FIRST, BGM_LOCKED, BGM_RANDOM, BGM_RANDOM_FIRST, BattleSetup, COLS, Cell, CharSelData,
    CharSelLayout, CharSelProgress, CharSelResult, FACE_FILE, FADE_FRAMES, ID_CUSTOM, ID_EMPTY,
    ID_RANDOM, Pick, PictureLoader, STAGE_BGM_COL, STAGE_COLS, STAGE_FILE, STAGE_LOCKED,
    STAGE_RANDOM, SideSetup, bgm_list_apply_unlocks, chr_grid_fix_cursor, chr_grid_is_selectable,
    chr_grid_move_down, chr_grid_move_left, chr_grid_move_right, chr_grid_move_up,
    chr_grid_next_form, chr_grid_prev_form, stage_grid_apply_unlocks, stg_grid_move_down,
    stg_grid_move_left, stg_grid_move_right, stg_grid_move_up,
};
use crate::draw::ScreenQuad;
use crate::duel_menu::DuelProgress;
use crate::flash::{
    ClipRef, FLASH_PAD, Flash, PROP_MASK, PROP_MASK_SET, PROP_VISIBLE, Uv, screen_quads,
};
use crate::font::Font;
use crate::item_panel::{ItemHelp, ItemPanel};
use crate::logo::{ColorFade, picture_quads};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};
use bt3_data::chara_data::ChrTable;

const MEMBER_MAX: usize = 5;
/// `TS_CUR_MENU`: the member list's cursor on the "team complete" plate.
const CUR_MENU: usize = 5;
const CUSTOM_ROWS: i32 = 2;

const FLAG_STARTED: u32 = 2;
const FLAG_STAGE: u32 = 4;
const FLAG_DECIDED: u32 = 8;
const FLAG_LEAVING: u32 = 0x10;
const FLAG_STAGE_CHANGE: u32 = 0x20;
const FLAG_STAGE_READY: u32 = 0x40;

const SIDE_FACE_CHANGE: u32 = 1;
const SIDE_FACE_READY: u32 = 2;
const SIDE_FORM: u32 = 0x40;
const SIDE_CUSTOM: u32 = 0x80;
const SIDE_NO_FACE: u32 = 0x400;
const SIDE_PANEL: u32 = 0x800;

const ST_GRID: i32 = 0;
const ST_FORM: i32 = 1;
const ST_PLATE: i32 = 2;
const ST_PANEL: i32 = 3;
const ST_PANEL_HELP: i32 = 4;
const ST_COLOR: i32 = 5;
const ST_TEAM: i32 = 6;
const ST_CUSTOM: i32 = 7;
const ST_DONE: i32 = 8;
const ST_STAGE: i32 = 9;
const ST_BGM: i32 = 10;

const PLAYERS_TWO: i32 = 1;

const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;
const LOAD_IDLE: i32 = 4;
const LOAD_ABORT: i32 = 5;
const LOAD_RESTART: i32 = 6;

/// The kinds of `TeamSel_ClipGoto` that name a clip (kind 10, the music entry, names none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Clip {
    Chip,
    FormChip,
    CustomPlate,
    ColorPlate,
    Team,
    CustomChip,
    StageChip,
}

/// `TeamSelSide`.
#[derive(Clone, Copy, Debug, Default)]
struct Side {
    member: [Pick; MEMBER_MAX],
    /// [0] the reel's seven chips, [1] before the last change.
    chip: [[i32; 7]; 2],
    flags: u32,
    /// The member being chosen; `CUR_MENU` on the "team complete" plate.
    cur: usize,
    cost: i32,
    member_count: usize,
    chara: i32,
    state: i32,
    mask: i32,
}

impl Side {
    /// `member[cur]` as the original reads it: on the menu plate (`cur` 5) that is the words after the members,
    /// the chips.
    fn cur_pick(&self) -> Pick {
        match self.member.get(self.cur) {
            Some(&p) => p,
            None => {
                let c = self.chip[0];
                Pick {
                    col: c[0],
                    row: c[1],
                    form: c[2],
                    custom_col: c[3],
                    custom_row: c[4],
                    custom_plate: c[5],
                    color_plate: c[6],
                    picked: self.chip[1][0],
                    ..Pick::default()
                }
            }
        }
    }

    /// `member[cur]` to change (never on the menu plate, where no state changes a member).
    fn cur_mut(&mut self) -> &mut Pick {
        let cur = self.cur.min(MEMBER_MAX - 1);
        &mut self.member[cur]
    }
}

/// `TeamSelStage`.
#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    col: i32,
    row: i32,
    chip: [[i32; 6]; 2],
    stage: i32,
    state: i32,
    mask: i32,
    /// The music list's cursor, and the entry chosen (and playing).
    bgm_cursor: i32,
    bgm: i32,
}

pub struct TeamSelScreen {
    layout: CharSelLayout,
    name_text: Vec<u8>,
    form_text: Vec<u8>,
    /// 0 the background, 1 / 2 the member lists, 3 / 4 the reels, 5 / 6 the plates.
    flash: [Flash; 7],
    tex: Vec<Option<usize>>,
    team_tex: [Vec<Option<usize>>; 2],
    side_tex: [Vec<Option<usize>>; 2],
    plate_tex: [Vec<Option<usize>>; 2],
    icon: IconWin,
    fade: ColorFade,
    pads: [PadRepeat; 2],
    flags: u32,
    face_state: i32,
    stage_state: i32,
    face_side: usize,
    timer: i32,
    side: [Side; 2],
    /// The member being changed, as it was.
    backup: [Pick; 2],
    stage: Stage,
    master_count: [i32; 2],
    cells: [Vec<Cell>; 2],
    custom_count: [i32; 2],
    custom: [Vec<Cell>; 2],
    rows: [i32; 2],
    plate_count: [i32; 2],
    color_count: [i32; 2],
    players: i32,
    battle_type: i32,
    dp_level: i32,
    dp_max: i32,
    bg: [Option<usize>; 2],
    bg_alpha: [i32; 2],
    face_alpha: [f32; 2],
    /// The side pad 0 is choosing for.
    base: usize,
    stage_ids: Vec<i32>,
    bgm_ids: Vec<i32>,
    stage_count: i32,
    bgm_count: i32,
    face_mask: i32,
    loading_face: Option<usize>,
    loading_stage: Option<usize>,
    chr: ChrTable,
    panels: [ItemPanel; 2],
    help: ItemHelp,
    item_info: usize,
    progress: CharSelProgress,
    running: bool,
    done: Option<CharSelResult>,
}

impl TeamSelScreen {
    /// `TeamSel_Init(2)` and the first `ColorFade_StartIn`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &CharSelData,
        duel: DuelProgress,
        progress: CharSelProgress,
        save: &SaveData,
        chr: ChrTable,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) -> Result<Self, Box<dyn Error>> {
        let fods = [
            &data.fods[0],
            &data.team_fods[0],
            &data.team_fods[1],
            &data.fods[1],
            &data.fods[2],
            &data.fods[3],
            &data.fods[4],
        ];
        let mut flash = Vec::new();
        for fod in fods {
            let mut f = Flash::create(fod)?;
            f.play(1);
            flash.push(f);
        }
        let flash: [Flash; 7] = flash
            .try_into()
            .map_err(|_| "the team select has seven movies")?;
        let layout = data.layout.clone();
        let dp = usize::from(duel.battle_type == 2);
        let mut stage_ids = data.stage_ids.clone();
        stage_grid_apply_unlocks(&mut stage_ids, save);
        let mut bgm_ids = data.bgm_ids.clone();
        bgm_list_apply_unlocks(&mut bgm_ids, save);
        // The last four entries are not offered; the last one kept becomes "random".
        let bgm_count = bgm_ids.len() as i32 - 4;
        bgm_ids[bgm_count as usize - 1] = BGM_RANDOM;
        bgm_ids.truncate(bgm_count.max(0) as usize);
        // ChrGrid_Build in mode 40: a DP battle has no random cell.
        let flags = if duel.battle_type == 2 {
            crate::char_select::GRID_NO_RANDOM
        } else {
            0
        };
        let (grid, custom, custom_count) =
            crate::char_select::chr_grid_build_with(&data.master, save, flags);
        let master_count = grid.len() as i32;
        let rows = (master_count + COLS as i32 - 1) / COLS as i32;
        let mut screen = Self {
            tex: layout.team_flash[usize::from(duel.players == 1)][dp].clone(),
            team_tex: [0, 1].map(|s| layout.team[s][dp].clone()),
            side_tex: [0, 1].map(|s| layout.team_side[s][dp].clone()),
            plate_tex: layout.plate.clone(),
            icon: IconWin::new(&data.icon_fod, layout.icon.clone())?,
            panels: [
                ItemPanel::new(0, &data.items, layout.panel[0].clone())?,
                ItemPanel::new(1, &data.items, layout.panel[1].clone())?,
            ],
            help: ItemHelp::new(&data.items, layout.help.clone())?,
            layout,
            name_text: data.name_text.clone(),
            form_text: data.form_text.clone(),
            flash,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pads: [PadRepeat::default(); 2],
            flags: 0,
            face_state: LOAD_IDLE,
            stage_state: LOAD_IDLE,
            face_side: 0,
            timer: 0,
            side: [Side::default(); 2],
            backup: [Pick::default(); 2],
            stage: Stage::default(),
            master_count: [master_count; 2],
            cells: [grid.clone(), grid],
            custom_count: [custom_count; 2],
            custom: [custom.clone(), custom],
            rows: [rows; 2],
            plate_count: [0; 2],
            color_count: [0; 2],
            players: duel.players,
            battle_type: duel.battle_type,
            dp_level: duel.dp_limit,
            dp_max: 0,
            bg: [None; 2],
            bg_alpha: [0x80, 0],
            face_alpha: [0.0; 2],
            base: 0,
            stage_count: stage_ids.len() as i32,
            stage_ids,
            bgm_count: bgm_ids.len() as i32,
            bgm_ids,
            face_mask: 0,
            loading_face: None,
            loading_stage: None,
            item_info: 0,
            chr,
            progress,
            running: true,
            done: None,
        };
        let s = &mut screen;
        for i in 0..2 {
            s.side[i].member[0] = progress.side[i];
            s.side[i].member[1..].copy_from_slice(&progress.team[i]);
        }
        s.stage.col = progress.stage % STAGE_COLS;
        s.stage.row = progress.stage / STAGE_COLS;
        for (i, &id) in s.bgm_ids.iter().enumerate() {
            if id == progress.bgm {
                s.stage.bgm_cursor = i as i32;
            }
        }
        if s.battle_type == 2 {
            s.dp_max = match s.dp_level {
                0 => 10,
                1 => 15,
                2 => 20,
                _ => 0,
            };
        }
        // A remembered cursor on a cell that cannot be chosen any more goes back to column `side`, row 0.
        for i in 0..2 {
            for j in 0..MEMBER_MAX {
                let m = s.side[i].member[j];
                if !chr_grid_is_selectable(&s.cells[i], (m.col + m.row * COLS as i32) as usize) {
                    let fresh = Pick {
                        col: i as i32,
                        ..Pick::default()
                    };
                    s.side[i].member[j] = fresh;
                    if j == 0 {
                        s.progress.side[i] = fresh;
                    } else {
                        s.progress.team[i][j - 1] = fresh;
                    }
                }
            }
        }
        if s.stage_ids
            .get(progress.stage as usize)
            .is_none_or(|&id| id >= STAGE_LOCKED)
        {
            s.progress.stage = 0;
            s.stage.col = 0;
            s.stage.row = 0;
        }
        s.set_chips(0);
        s.set_chips(1);
        s.set_stage_chips();
        for i in 0..2 {
            s.side[i].chara = s.side[i].chip[0][s.side[i].member[0].col as usize];
        }
        s.stage.stage = s.stage_ids[(s.stage.row * STAGE_COLS + s.stage.col) as usize];
        s.stage.bgm = s.stage.bgm_cursor;
        for i in 0..2 {
            for m in &mut s.side[i].member {
                m.picked = -1;
            }
        }
        for i in 0..2 {
            s.tex[if i != 0 { 22 } else { 23 }] =
                loader.portrait(FACE_FILE + s.side[i].chara as u32);
            s.side[i].flags |= SIDE_FACE_READY;
        }
        let picture = loader.picture(STAGE_FILE + s.stage.stage as u32);
        s.bg = [picture; 2];
        s.flags |= FLAG_STAGE_READY;
        let id = s.bgm_ids[s.stage.bgm as usize];
        let file = if id == BGM_RANDOM {
            BGM_RANDOM_FIRST + random.range(9)
        } else {
            BGM_FIRST + id as u32
        };
        sounds.push(TitleSound::Bgm(file));
        Ok(screen)
    }

    pub fn result(&self) -> Option<CharSelResult> {
        self.done.clone()
    }

    pub fn progress(&self) -> CharSelProgress {
        self.progress
    }

    fn cell(&self, side: usize) -> Cell {
        let p = self.side[side].cur_pick();
        self.cells[side]
            .get((p.row * COLS as i32 + p.col) as usize)
            .copied()
            .unwrap_or(Cell {
                id: ID_EMPTY,
                ..Cell::default()
            })
    }

    fn custom_cell(&self, side: usize) -> Cell {
        let p = self.side[side].cur_pick();
        self.custom[side][(p.custom_col + p.custom_row * COLS as i32) as usize]
    }

    /// `TeamSel_SumCost`: a DP battle's team total, without the member being chosen with `skip_cur`.
    fn sum_cost(&mut self, side: usize, skip_cur: bool) {
        if self.battle_type == 2 {
            let s = &self.side[side];
            let cost = (0..s.member_count)
                .filter(|&i| i != s.cur || !skip_cur)
                .map(|i| self.chr.cost(s.member[i].picked))
                .sum();
            self.side[side].cost = cost;
        }
    }

    /// `TeamSel_IsCharaFree`: no other member is the same person.
    fn is_chara_free(&self, side: usize, chara: i32) -> bool {
        let s = &self.side[side];
        !(0..s.member_count).any(|i| i != s.cur && self.chr.is_related(chara, s.member[i].picked))
    }

    /// `TeamSel_FitsDp`: a DP battle's team can still afford `chara` (the random and custom cells always fit).
    fn fits_dp(&self, side: usize, chara: i32) -> bool {
        if self.battle_type != 2 || chara == ID_RANDOM || chara == ID_CUSTOM {
            return true;
        }
        self.side[side].cost + self.chr.cost(chara) <= self.dp_max
    }

    /// `TeamSel_RemoveMember`: the members behind move up, the last slot is emptied.
    fn remove_member(&mut self, side: usize, idx: usize) {
        let empty = Pick {
            picked: -1,
            ..Pick::default()
        };
        let s = &mut self.side[side];
        if let Some(m) = s.member.get_mut(idx) {
            *m = empty;
        }
        if idx < s.member_count {
            for i in idx..s.member_count - 1 {
                s.member[i] = s.member[i + 1];
            }
            s.member[s.member_count - 1] = empty;
        }
    }

    /// The reel chips: slots 8 and 11..16 of the side's movie, the old textures moved to 18..24.
    fn put_chips(&mut self, side: usize, ids: [i32; 7]) {
        const SLOT: [usize; 7] = [8, 11, 12, 13, 14, 15, 16];
        for (i, &id) in ids.iter().enumerate() {
            let s = &mut self.side[side];
            s.chip[1][i] = s.chip[0][i];
            s.chip[0][i] = id;
            let chip = self.layout.chips.get(id as usize).copied();
            let tex = &mut self.side_tex[side];
            tex[18 + i] = tex[SLOT[i]];
            tex[SLOT[i]] = chip;
        }
    }

    /// `TeamSel_SetChips`: the cursor's row of the grid.
    fn set_chips(&mut self, side: usize) {
        let row = self.side[side].cur_pick().row as usize;
        let ids = std::array::from_fn(|i| {
            self.cells[side]
                .get(i + row * COLS)
                .map_or(ID_EMPTY, |c| c.id)
        });
        self.put_chips(side, ids);
    }

    /// `TeamSel_SetGridFormChips` / `TeamSel_SetFormChips`: the forms of a cell.
    fn set_cell_form_chips(&mut self, side: usize, cell: Cell) {
        let ids = std::array::from_fn(|i| {
            if (i as i32) < cell.form_count {
                cell.form[i]
            } else {
                ID_EMPTY
            }
        });
        self.put_chips(side, ids);
    }

    /// `TeamSel_SetCustomChips`.
    fn set_custom_chips(&mut self, side: usize) {
        let row = self.side[side].cur_pick().custom_row as usize;
        let ids = std::array::from_fn(|i| self.custom[side][i + row * COLS].id);
        self.put_chips(side, ids);
    }

    /// `TeamSel_SetStageChips`.
    fn set_stage_chips(&mut self) {
        const SLOT: [usize; 6] = [8, 11, 12, 13, 14, 15];
        for (i, &slot) in SLOT.iter().enumerate() {
            let id = self.stage_ids[(self.stage.row * STAGE_COLS) as usize + i];
            self.stage.chip[1][i] = self.stage.chip[0][i];
            self.stage.chip[0][i] = id;
            self.tex[41 + i] = self.tex[slot];
            self.tex[slot] = self.layout.stage_chips.get(id as usize).copied();
        }
    }

    /// `TeamSel_SetTeamTex`: the members' chips on the side's member list, and a DP battle's total.
    fn set_team_tex(&mut self, side: usize) {
        const SLOT: [usize; 5] = [1, 4, 5, 6, 7];
        if self.battle_type == 2 {
            self.side[side].cost = 0;
        }
        for (i, &slot) in SLOT.iter().enumerate() {
            self.team_tex[side][slot] = None;
            if i < self.side[side].member_count {
                let id = self.side[side].member[i].picked;
                self.team_tex[side][slot] = self.layout.chips.get(id as usize).copied();
                if self.battle_type == 2 {
                    self.side[side].cost += self.chr.cost(id);
                }
            }
        }
    }

    /// `TeamSel_UpdateFaceLoad`: one step of the portrait loader (none for a side on an empty slot).
    fn update_face_load(&mut self, loader: &mut dyn PictureLoader) {
        let fs = self.face_side;
        match self.face_state {
            LOAD_ABORT => {
                if self.side[fs ^ 1].flags & SIDE_FACE_CHANGE != 0 {
                    self.face_side ^= 1;
                }
                self.face_state = LOAD_RESTART;
            }
            LOAD_RESTART => {
                self.side[fs].flags &= !SIDE_FACE_CHANGE;
                self.face_state = LOAD_REQUEST;
            }
            LOAD_REQUEST => {
                if self.side[fs].chara < 0 {
                    self.side[fs].flags |= SIDE_NO_FACE | SIDE_FACE_READY;
                    self.next_face();
                } else {
                    self.loading_face = loader.portrait(FACE_FILE + self.side[fs].chara as u32);
                    self.face_state = LOAD_READ;
                }
            }
            LOAD_READ => self.face_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.tex[if fs == 0 { 23 } else { 22 }] = self.loading_face;
                self.side[fs].flags |= SIDE_FACE_READY;
                self.next_face();
            }
            LOAD_IDLE => {
                if self.stage_state != LOAD_IDLE {
                    return;
                }
                if self.side[0].flags & SIDE_FACE_CHANGE != 0 {
                    self.tex[23] = None;
                    self.face_side = 0;
                    self.face_state = LOAD_RESTART;
                } else if self.side[1].flags & SIDE_FACE_CHANGE != 0 {
                    self.tex[22] = None;
                    self.face_side = 1;
                    self.face_state = LOAD_RESTART;
                }
            }
            _ => {}
        }
    }

    /// A portrait is done: the other side's, if it changed meanwhile, else idle.
    fn next_face(&mut self) {
        if self.side[self.face_side ^ 1].flags & SIDE_FACE_CHANGE != 0 {
            self.face_side ^= 1;
            self.face_state = LOAD_RESTART;
        } else {
            self.face_state = LOAD_IDLE;
        }
    }

    /// `TeamSel_RequestFace`.
    fn request_face(&mut self, side: usize) {
        if self.side[side].flags & SIDE_FACE_READY != 0 {
            self.side[side].flags ^= SIDE_FACE_READY;
        } else if self.face_side == side {
            self.face_state = LOAD_ABORT;
        }
        self.side[side].flags &= !SIDE_NO_FACE;
        self.side[side].flags |= SIDE_FACE_CHANGE;
    }

    /// `TeamSel_UpdateStageLoad`.
    fn update_stage_load(&mut self, loader: &mut dyn PictureLoader) {
        match self.stage_state {
            LOAD_ABORT => self.stage_state = LOAD_RESTART,
            LOAD_RESTART => {
                self.flags &= !FLAG_STAGE_CHANGE;
                self.stage_state = LOAD_REQUEST;
            }
            LOAD_REQUEST => {
                self.loading_stage = loader.picture(STAGE_FILE + self.stage.stage as u32);
                self.stage_state = LOAD_READ;
            }
            LOAD_READ => self.stage_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.bg[0] = self.loading_stage;
                self.flags |= FLAG_STAGE_READY;
                self.stage_state = LOAD_IDLE;
            }
            LOAD_IDLE => {
                if self.face_state == LOAD_IDLE && self.flags & FLAG_STAGE_CHANGE != 0 {
                    self.stage_state = LOAD_RESTART;
                }
            }
            _ => {}
        }
    }

    /// `TeamSel_RequestStage`: the shown picture stays behind until the new one is in.
    fn request_stage(&mut self) {
        if self.flags & FLAG_STAGE_READY != 0 {
            self.flags ^= FLAG_STAGE_READY;
        } else {
            self.stage_state = LOAD_ABORT;
        }
        self.flags |= FLAG_STAGE_CHANGE;
        self.bg[1] = self.bg[0];
    }

    /// `TeamSel_ClipGoto(flash, side, kind, label)`.
    fn clip_goto(&mut self, flash: usize, side: usize, kind: Clip, label: &str) {
        let s = &self.side[side];
        let p = s.cur_pick();
        let name = match kind {
            Clip::Chip => format!("mc_chara_chip_{:03}", p.col),
            Clip::FormChip => format!("mc_chara_chip_{:03}", p.form),
            Clip::CustomPlate => format!("mc_custom_plate_{}", p.custom_plate + 1),
            Clip::ColorPlate => format!("mc_color_plate_{}", p.color_plate + 1),
            Clip::Team if s.cur == CUR_MENU => "mc_menu_plate".to_owned(),
            Clip::Team => format!("mc_team_{}", s.cur),
            Clip::CustomChip => format!("mc_chara_chip_{:03}", p.custom_col),
            Clip::StageChip if self.stage.col == STAGE_BGM_COL => "mc_bgm_now".to_owned(),
            Clip::StageChip => format!("mc_map_chip_{:02}", self.stage.col),
        };
        let f = &mut self.flash[flash];
        let clip = f.find(None, &name);
        f.clip_goto_label(clip, label);
    }

    fn find(&self, flash: usize, parent: Option<&str>, name: &str) -> ClipRef {
        self.flash[flash].find(parent, name)
    }

    fn set_uv(&mut self, flash: usize, parent: Option<&str>, name: &str, uv: Uv) {
        let clip = self.find(flash, parent, name);
        self.flash[flash].clip_set_uv(clip, uv);
    }

    fn set_flags(&mut self, flash: usize, parent: Option<&str>, name: &str, props: u32, on: bool) {
        let clip = self.find(flash, parent, name);
        self.flash[flash].clip_set_flags(clip, props, on);
    }

    fn set_color(&mut self, flash: usize, parent: Option<&str>, name: &str, color: f32) {
        let clip = self.find(flash, parent, name);
        self.flash[flash].clip_set_color(clip, color);
    }

    /// One frame of the `TeamSel_Run` loop. `pads` are the pressed / held game buttons of pads 0 and 1.
    pub fn run_frame(
        &mut self,
        pads: [(u32, u32); 2],
        loader: &mut dyn PictureLoader,
        save: &SaveData,
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
        self.update_face_load(loader);
        self.update_stage_load(loader);
        let pad: [(u32, u32); 2] = std::array::from_fn(|i| {
            let (pressed, held) = pads[i];
            (pressed, self.pads[i].update(held, pressed))
        });
        self.fade.update();
        self.update();
        self.draw(font, &mut frame.quads);
        font.flush(&mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::BgmFadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            if self.face_state == LOAD_IDLE || self.stage_state == LOAD_IDLE {
                self.done = Some(if self.running {
                    CharSelResult::Battle(self.battle_setup(save, random))
                } else {
                    CharSelResult::Back
                });
            }
            return frame;
        }
        if self.flags & FLAG_LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                // The stage was chosen 60 frames ago: fade out and remember the choices for the next visit.
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                let team = |s: &Side| -> [Pick; 4] { s.member[1..].try_into().unwrap() };
                self.progress = CharSelProgress {
                    side: [self.side[0].member[0], self.side[1].member[0]],
                    team: [team(&self.side[0]), team(&self.side[1])],
                    stage: self.stage.col + self.stage.row * STAGE_COLS,
                    bgm: self.bgm_ids[self.stage.bgm as usize],
                };
            }
        } else {
            self.input(pad, save, random, &mut frame.sounds);
        }
        frame
    }

    /// The end of `TeamSel_Run`: a random stage or music is drawn, then the battle setup (member 0 the lead, all at
    /// the rule's computer level).
    fn battle_setup(&mut self, save: &SaveData, random: &mut GameRandom) -> BattleSetup {
        if self.stage.stage == STAGE_RANDOM {
            loop {
                self.stage.stage =
                    self.stage_ids[random.range(self.stage_count as u32 - 1) as usize];
                if self.stage.stage < STAGE_RANDOM {
                    break;
                }
            }
        }
        let mut bgm = self.bgm_ids[self.stage.bgm as usize];
        if bgm == BGM_RANDOM {
            bgm = random.range(9) as i32 + 8;
        }
        let time_limit = match save.rules[0] {
            0 => 1,
            1 => 2,
            2 => 3,
            3 => 4,
            4 => 0,
            _ => 5,
        };
        let flag = [save.rules[3] ^ 1, save.rules[4] ^ 1];
        let cpu_level = crate::btl_menu::cpu_level_from_setting(save.rules[1]);
        let (control, change) = match self.players {
            0 => ([0, 2], [1, flag[1]]),
            PLAYERS_TWO => ([0, 0], [1, 1]),
            _ => ([2, 2], flag),
        };
        BattleSetup {
            wide: false,
            screen_mode: i32::from(self.players == PLAYERS_TWO),
            mode: 0,
            bgm,
            time_limit,
            announcer: save.rules[2],
            stage: self.stage.stage,
            stage_change: save.rules[5] ^ 1,
            sides: std::array::from_fn(|i| {
                let s = &self.side[i];
                let lead = s.member[0];
                let count = s.member_count.clamp(1, MEMBER_MAX);
                SideSetup {
                    control: control[i],
                    pad: i as i32,
                    change_allowed: change[i],
                    chara: lead.picked,
                    costume: lead.color_plate,
                    cpu_level,
                    items: lead.items,
                    team_items: std::array::from_fn(|k| s.member[k + 1].items),
                    team: std::array::from_fn(|k| {
                        let m = s.member[k + 1];
                        (m.picked, m.color_plate)
                    }),
                    team_count: count as i32 - 1,
                    switch_enabled: 1,
                    members: None,
                }
            }),
            pool: Vec::new(),
            tutorial: 0,
        }
    }

    /// `TeamSel_Update`.
    fn update(&mut self) {
        for f in &mut self.flash {
            f.advance();
        }
        for p in &mut self.panels {
            p.update();
        }
        for i in 0..2 {
            if self.side[i].mask != 0 && self.flash[3 + i].trig() & 1 != 0 {
                self.side[i].mask = 0;
            }
        }
        if self.stage.mask != 0 && self.flash[0].trig() & 1 != 0 {
            self.stage.mask = 0;
        }
        if self.face_mask != 0 && (self.flash[0].trig() as i32) < 0 {
            self.face_mask = 0;
        }
    }

    /// The cell of a cost digit (`mc_dp_num_*`): four 0x20 cells a row.
    fn cost_uv(cost: i32) -> Uv {
        Uv {
            u0: (cost % 4) * 0x20,
            v0: (cost / 4) * 0x20,
            u1: (cost % 4) * 0x20 + 0x20,
            v1: (cost / 4) * 0x20 + 0x20,
            tex: 0,
        }
    }

    /// `TeamSel_Draw`.
    fn draw(&mut self, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        for i in 0..2 {
            if self.flags & FLAG_STAGE_READY != 0 {
                if i == 0 {
                    self.bg_alpha[0] = (self.bg_alpha[0] + 5).min(0x80);
                } else {
                    self.bg_alpha[1] = (self.bg_alpha[1] - 5).max(0);
                }
            } else if i == 0 {
                self.bg_alpha[0] = 0;
                continue;
            } else {
                self.bg_alpha[1] = 0x80;
            }
            if let Some(first) = self.bg[i] {
                let alpha = self.bg_alpha[i] as u8;
                quads.extend(picture_quads(first).into_iter().map(|mut q| {
                    q.rgba[3] = alpha;
                    ScreenQuad::from(q)
                }));
            }
        }

        let mut texts: Vec<TextDraw> = Vec::new();
        for i in 0..2 {
            let lr = if i != 0 { 'r' } else { 'l' };
            let mask = self.face_mask != 0;
            self.set_flags(
                0,
                None,
                &format!("mc_face_mask_{lr}"),
                PROP_MASK_SET | PROP_VISIBLE,
                mask,
            );
            if self.side[i].flags & (SIDE_NO_FACE | SIDE_FACE_READY) == SIDE_FACE_READY {
                self.face_alpha[i] = (self.face_alpha[i] + 0.075).min(1.0);
            } else {
                self.face_alpha[i] = 0.0;
            }
            let chara = self.find(0, None, &format!("mc_single_chara_{lr}"));
            self.flash[0].clip_set_alpha(chara, self.face_alpha[i]);
            self.flash[0].clip_set_flags(chara, PROP_MASK, mask);
            let line = self.side[i].chara;
            let name = self.find(0, None, &format!("mc_name_text_{lr}"));
            let form = self.find(0, None, &format!("mc_form_text_{lr}"));
            let f = &mut self.flash[0];
            texts.extend(TextBox::preset(i as u32 + 1).attach_line(
                f,
                name,
                i as u32,
                &self.name_text,
                line,
                font,
            ));
            texts.extend(TextBox::preset(i as u32 + 3).attach_line(
                f,
                form,
                2 + i as u32,
                &self.form_text,
                line,
                font,
            ));
        }

        let stage = self.stage.stage;
        let clip = self.find(0, None, "mc_map_name");
        self.flash[0].clip_set_uv(
            clip,
            Uv {
                u0: 0,
                v0: (stage % 4) * 0x40,
                u1: 0x200,
                v1: (stage % 4) * 0x40 + 0x40,
                tex: 0,
            },
        );
        self.flash[0].clip_set_tex(clip, stage / 4);
        if self.battle_type == 2 {
            for i in 0..2 {
                let lr = if i != 0 { 'r' } else { 'l' };
                let (x, y) = ((self.dp_level % 2) * 0x40, (self.dp_level / 2) * 0x20);
                self.set_uv(
                    0,
                    None,
                    &format!("mc_dp_max_{lr}"),
                    Uv {
                        u0: x,
                        v0: y,
                        u1: x + 0x40,
                        v1: y + 0x20,
                        tex: 0,
                    },
                );
                let cost = self.side[i].cost;
                crate::anim::num_draw(
                    &mut self.flash[0],
                    |n| format!("mc_dp_total_{lr}_{n}"),
                    2,
                    cost,
                    (0x20, 0x40),
                    1,
                );
            }
        }
        let mask = self.stage.mask != 0;
        self.set_flags(0, None, "mc_map_mask", PROP_MASK_SET | PROP_VISIBLE, mask);
        for i in 0..12 {
            self.set_flags(0, None, &format!("mc_map_chip_{i:02}"), PROP_MASK, mask);
        }
        let arrows = [
            ("mc_yajirusi_up", "mc_yajirusi_icon_up"),
            ("mc_yajirusi_down", "mc_yajirusi_icon_down"),
        ];
        for (i, (parent, icon)) in arrows.into_iter().enumerate() {
            let x = i as i32 * 0x20;
            self.set_uv(
                0,
                Some(parent),
                icon,
                Uv {
                    u0: x,
                    v0: 0x20,
                    u1: x + 0x20,
                    v1: 0x40,
                    tex: 0,
                },
            );
        }
        let bgm = self.bgm_ids[self.stage.bgm_cursor as usize];
        let uv = Uv {
            u0: 0,
            v0: (bgm % 8) * 0x20,
            u1: 0x200,
            v1: (bgm % 8) * 0x20 + 0x20,
            tex: 0,
        };
        for child in ["mc_bgm_now_text_off", "mc_bgm_now_text_on"] {
            let clip = self.find(0, Some("mc_bgm_now"), child);
            self.flash[0].clip_set_uv(clip, uv);
            self.flash[0].clip_set_tex(clip, bgm / 8);
        }

        // The plates.
        for i in 0..2 {
            let f = 5 + i;
            let x = (i as i32 ^ 1) * 0x20;
            self.set_uv(
                f,
                None,
                "mc_yajirusi",
                Uv {
                    u0: x,
                    v0: 0,
                    u1: x + 0x20,
                    v1: 0x20,
                    tex: 0,
                },
            );
            for j in 0..4 {
                let uv = Uv {
                    u0: 0,
                    v0: j * 0x20,
                    u1: 0x100,
                    v1: j * 0x20 + 0x20,
                    tex: 0,
                };
                let name = format!("mc_custom_plate_{}", j + 1);
                let color = if j < self.plate_count[i] { 1.0 } else { 0.3 };
                self.set_color(f, None, &name, color);
                self.set_uv(f, Some(&name), "mc_custom_text_off", uv);
                self.set_uv(f, Some(&name), "mc_custom_text_on", uv);
                let arrow = j != 0 && self.cell(i).id != ID_RANDOM;
                self.set_flags(f, Some(&name), "mc_yajirusi", PROP_VISIBLE, arrow);
            }
            for j in 0..4 {
                let uv = Uv {
                    u0: (j % 2) * 0x40,
                    v0: (j / 2) * 0x20,
                    u1: (j % 2) * 0x40 + 0x40,
                    v1: (j / 2) * 0x20 + 0x20,
                    tex: 0,
                };
                let name = format!("mc_color_plate_{}", j + 1);
                let plate = self.find(f, None, &name);
                if j < self.color_count[i] {
                    self.flash[f].clip_set_flags(plate, PROP_VISIBLE, true);
                    match self.color_count[i] {
                        2 => self.flash[f].clip_set_offset(plate, 0x2D, 0),
                        3 => self.flash[f].clip_set_offset(plate, 0x16, 0),
                        _ => {}
                    }
                    self.set_uv(f, Some(&name), "mc_color_text_off", uv);
                    self.set_uv(f, Some(&name), "mc_color_text_on", uv);
                } else {
                    self.flash[f].clip_set_flags(plate, PROP_VISIBLE, false);
                }
            }
        }

        // The reels: the "1P / 2P / COM" plate, the arrows, and in each chip whether it can be chosen (and in a
        // DP battle its cost).
        for i in 0..2 {
            let f = 3 + i;
            let n = match self.players {
                0 => {
                    if i == 0 {
                        0
                    } else {
                        2
                    }
                }
                1 => i as i32,
                2 => 2,
                _ => 0,
            };
            self.set_uv(
                f,
                None,
                "mc_plate_text",
                Uv {
                    u0: (n % 2) * 0x40,
                    v0: (n / 2) * 0x20,
                    u1: (n % 2) * 0x40 + 0x40,
                    v1: (n / 2) * 0x20 + 0x20,
                    tex: 0,
                },
            );
            if self.rows[i] >= 2 {
                for (j, (parent, icon)) in arrows.into_iter().enumerate() {
                    let x = j as i32 * 0x20;
                    self.set_uv(
                        f,
                        Some(parent),
                        icon,
                        Uv {
                            u0: x,
                            v0: 0x20,
                            u1: x + 0x20,
                            v1: 0x40,
                            tex: 0,
                        },
                    );
                    let show = if self.side[i].flags & SIDE_FORM != 0 {
                        false
                    } else if i == 1 && self.players != PLAYERS_TWO {
                        self.side[0].state == ST_DONE
                    } else {
                        true
                    };
                    self.set_flags(f, None, parent, PROP_VISIBLE, show);
                }
            } else {
                for (parent, _) in arrows {
                    self.set_flags(f, None, parent, PROP_VISIBLE, false);
                }
            }
            let mask = self.side[i].mask != 0;
            self.set_flags(f, None, "mc_chara_mask", PROP_MASK_SET | PROP_VISIBLE, mask);
            for j in 0..14 {
                let name = format!("mc_chara_chip_{j:03}");
                self.set_flags(f, None, &name, PROP_MASK, mask);
                let chara = self.side[i].chip[j / COLS][j % COLS];
                let chara_name = format!("mc_chara_{j:03}");
                let free = |s: &Self| {
                    if chara < ID_RANDOM && !s.is_chara_free(i, chara) {
                        0.4
                    } else {
                        1.0
                    }
                };
                if self.battle_type == 2 {
                    let cost = if chara < ID_RANDOM {
                        self.chr.cost(chara)
                    } else {
                        0
                    };
                    let color = if self.side[i].cost + cost > self.dp_max {
                        0.4
                    } else {
                        free(self)
                    };
                    self.set_color(f, Some(&name), &chara_name, color);
                    let num = self.find(f, Some(&name), &format!("mc_dp_num_{j:03}"));
                    if chara < ID_RANDOM {
                        self.flash[f].clip_set_uv(num, Self::cost_uv(cost));
                        self.flash[f].clip_set_flags(num, PROP_VISIBLE, true);
                    } else {
                        self.flash[f].clip_set_flags(num, PROP_VISIBLE, false);
                    }
                } else {
                    let color = free(self);
                    self.set_color(f, Some(&name), &chara_name, color);
                }
            }
        }

        // The member lists: each slot's plate, and the members' costs.
        for i in 0..2 {
            let f = 1 + i;
            for j in 0..MEMBER_MAX as i32 {
                let name = format!("mc_team_{j}");
                let k = if i == 0 { j } else { j + 5 };
                self.set_uv(
                    f,
                    Some(&name),
                    &format!("mc_team_plate_{j}"),
                    Uv {
                        u0: (k % 4) * 0x40,
                        v0: (k / 4) * 0x48,
                        u1: (k % 4) * 0x40 + 0x40,
                        v1: (k / 4) * 0x48 + 0x48,
                        tex: 0,
                    },
                );
                let member = (j as usize) < self.side[i].member_count;
                let num = self.find(f, Some(&name), &format!("mc_dp_num_{j:03}"));
                if member {
                    let cost = self.chr.cost(self.side[i].member[j as usize].picked);
                    self.flash[f].clip_set_flags(num, PROP_VISIBLE, true);
                    self.flash[f].clip_set_uv(num, Self::cost_uv(cost));
                } else {
                    self.flash[f].clip_set_flags(num, PROP_VISIBLE, false);
                }
                self.set_flags(
                    f,
                    Some(&name),
                    &format!("mc_dp_plate_{j:03}"),
                    PROP_VISIBLE,
                    member,
                );
            }
        }

        for (f, flash) in self.flash.iter().enumerate() {
            let textures: &[Option<usize>] = match f {
                0 => &self.tex,
                1 | 2 => &self.team_tex[f - 1],
                3 | 4 => &self.side_tex[f - 3],
                _ => &self.plate_tex[f - 5],
            };
            let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
            let (flash_quads, overs) = flash.draw_with_overs(&has_texture);
            quads.extend(screen_quads(&flash_quads, textures));
            if f == 0 {
                draw_texts(&texts, &overs, font);
            }
        }
        font.flush(quads);
        self.icon.draw(quads);
        for p in &mut self.panels {
            p.draw(&self.chr, font, quads);
        }
        self.help.draw(self.item_info, &self.chr, font, quads);
    }

    /// `IconWin_SetIcon(icon)`: texture `icon + 1` of the icon file.
    fn set_icon(&mut self, icon: usize) {
        self.icon
            .set_texture(4, Some(self.layout.icons + 2 * (icon + 1)));
    }

    /// The item-set plates, from the reel (`plates` offered: one in a DP battle).
    fn open_plates(&mut self, n: usize, plates: i32) {
        self.plate_count[n] = if self.battle_type == 2 { 1 } else { plates };
        if self.side[n].cur_pick().custom_plate >= self.plate_count[n] {
            self.side[n].cur_mut().custom_plate = 0;
        }
        self.clip_goto(n + 5, n, Clip::CustomPlate, "fl_on_start");
    }

    /// `ChrTbl_WrapCostume(chara, &color)`.
    fn wrap_costume(&mut self, n: usize) -> i32 {
        let count = self
            .chr
            .costumes
            .get(self.side[n].chara as usize)
            .copied()
            .unwrap_or(1);
        if self.side[n].cur_pick().color_plate >= count {
            self.side[n].cur_mut().color_plate = 0;
        }
        count
    }

    /// The reel back on its row of the grid, the portrait of the chip under the cursor.
    fn show_grid_row(&mut self, n: usize) {
        self.set_chips(n);
        self.side[n].chara = self.side[n].chip[0][self.side[n].cur_pick().col as usize];
        self.request_face(n);
    }

    /// `TeamSel_Input`.
    fn input(
        &mut self,
        pad: [(u32, u32); 2],
        save: &SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash[0].flags() & FLASH_PAD == 0 {
            return;
        }
        let pads = if self.players == PLAYERS_TWO { 2 } else { 1 };
        if self.flags & FLAG_STARTED == 0 {
            for i in 0..pads {
                self.clip_goto(i + 3, i, Clip::Chip, "fl_on_start");
            }
            self.flags |= FLAG_STARTED;
        }
        if self.flags & FLAG_STAGE != 0 {
            self.input_stage(pad[0], sounds);
            return;
        }
        let se = |sounds: &mut Vec<TitleSound>, bank: u32, id: u32| {
            sounds.push(TitleSound::Effect(bank, id))
        };
        for i in 0..pads {
            let n = i + self.base;
            // While the other side's item panel is open this pad is ignored.
            if self.side[n ^ 1].flags & SIDE_PANEL != 0 {
                continue;
            }
            let (pressed, repeat) = pad[i];
            let (team, reel, plate) = (n + 1, n + 3, n + 5);
            match self.side[n].state {
                ST_GRID => {
                    if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                        self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
                        let p = self.side[n].cur_pick();
                        let mut col = p.col;
                        if repeat & PAD_LEFT != 0 {
                            chr_grid_move_left(&self.cells[n], &mut col, p.row);
                        } else {
                            chr_grid_move_right(&self.cells[n], &mut col, p.row);
                        }
                        self.side[n].cur_mut().col = col;
                        self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                        self.side[n].chara = self.side[n].chip[0][col as usize];
                        self.request_face(n);
                        se(sounds, 2, 0);
                    } else if repeat & (PAD_UP | PAD_DOWN) != 0 {
                        if self.rows[n] < 2 {
                            continue;
                        }
                        let up = repeat & PAD_UP != 0;
                        self.flash[reel]
                            .goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                        self.side[n].mask = 1;
                        self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
                        let p = self.side[n].cur_pick();
                        let (mut col, mut row) = (p.col, p.row);
                        if up {
                            chr_grid_move_up(&self.cells[n], &mut col, &mut row, self.rows[n]);
                        } else {
                            chr_grid_move_down(&self.cells[n], &mut col, &mut row, self.rows[n]);
                        }
                        let m = self.side[n].cur_mut();
                        (m.col, m.row) = (col, row);
                        self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                        self.show_grid_row(n);
                        se(sounds, 2, if up { 2 } else { 1 });
                    } else if pressed & PAD_CROSS != 0 {
                        let cell = self.cell(n);
                        let chara = self.side[n].chara;
                        if cell.id == ID_RANDOM {
                            // Straight to the plates; the character is drawn when the costume is confirmed.
                            self.flash[plate].goto_label("fl_custom_in", true);
                            self.clip_goto(reel, n, Clip::Chip, "fl_ok");
                            self.open_plates(n, 4);
                            self.color_count[n] = 2;
                            if self.side[n].cur_pick().color_plate >= 2 {
                                self.side[n].cur_mut().color_plate = 0;
                            }
                            self.side[n].state = ST_PLATE;
                            se(sounds, 1, 1);
                        } else if cell.id == ID_CUSTOM {
                            if self.custom_count[n] != 0 {
                                let p = self.side[n].cur_pick();
                                let (mut col, mut row) = (p.custom_col, p.custom_row);
                                chr_grid_fix_cursor(
                                    &self.custom[n],
                                    &mut col,
                                    &mut row,
                                    CUSTOM_ROWS,
                                );
                                let m = self.side[n].cur_mut();
                                (m.custom_col, m.custom_row) = (col, row);
                                self.side[n].flags |= SIDE_CUSTOM;
                                self.flash[reel].goto_label("fl_form", true);
                                self.side[n].mask = 1;
                                self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
                                self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
                                self.set_custom_chips(n);
                                self.side[n].chara = self.side[n].chip[0][col as usize];
                                self.request_face(n);
                                self.side[n].state = ST_CUSTOM;
                                se(sounds, 2, 0x29);
                            } else {
                                se(sounds, 1, 7);
                            }
                        } else if !self.fits_dp(n, chara) {
                            se(sounds, 1, 7);
                        } else if cell.form_count != 0 {
                            self.side[n].flags |= SIDE_FORM;
                            if self.side[n].cur_pick().form >= cell.form_count {
                                self.side[n].cur_mut().form = 0;
                            }
                            self.flash[reel].goto_label("fl_form", true);
                            self.side[n].mask = 1;
                            self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
                            self.clip_goto(reel, n, Clip::FormChip, "fl_on_start");
                            self.set_cell_form_chips(n, cell);
                            self.side[n].chara =
                                self.side[n].chip[0][self.side[n].cur_pick().form as usize];
                            self.request_face(n);
                            self.side[n].state = ST_FORM;
                            se(sounds, 2, 0x29);
                        } else if self.is_chara_free(n, chara) {
                            self.flash[plate].goto_label("fl_custom_in", true);
                            self.clip_goto(reel, n, Clip::Chip, "fl_ok");
                            self.open_plates(n, 4);
                            self.color_count[n] = self.wrap_costume(n);
                            self.side[n].state = ST_PLATE;
                            se(sounds, 1, 1);
                        } else {
                            se(sounds, 1, 7);
                        }
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.side[n].mask = 0;
                        if self.side[n].member_count != 0 {
                            // Back to the member list; the member is as it was.
                            self.flash[reel].goto_label("fl_out", true);
                            self.flash[team].goto_label("fl_in", true);
                            self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
                            self.clip_goto(team, n, Clip::Team, "fl_on_start");
                            *self.side[n].cur_mut() = self.backup[n];
                            self.side[n].chara = self.side[n].cur_pick().picked;
                            self.request_face(n);
                            self.sum_cost(n, false);
                            self.side[n].state = ST_TEAM;
                        } else if self.players != PLAYERS_TWO && self.base == 1 {
                            // Back to the first team.
                            self.clip_goto(i + 4, n, Clip::Chip, "fl_off_start");
                            self.base = 0;
                            let n = i;
                            self.flash[n + 1].goto_label("fl_fast_in", true);
                            self.clip_goto(n + 1, n, Clip::Team, "fl_on_start");
                            self.side[n].state = ST_TEAM;
                        } else {
                            // Leave the screen.
                            self.fade = ColorFade::start_out(true, FADE_FRAMES);
                            self.running = false;
                        }
                        se(sounds, 1, 2);
                    }
                }
                ST_FORM => {
                    if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                        self.clip_goto(reel, n, Clip::FormChip, "fl_off_start");
                        let chips = self.side[n].chip[0];
                        let mut form = self.side[n].cur_pick().form;
                        if repeat & PAD_LEFT != 0 {
                            chr_grid_prev_form(&chips, &mut form);
                        } else {
                            chr_grid_next_form(&chips, &mut form);
                        }
                        self.side[n].cur_mut().form = form;
                        self.clip_goto(reel, n, Clip::FormChip, "fl_on_start");
                        self.side[n].chara = chips[form as usize];
                        self.request_face(n);
                        se(sounds, 2, 0);
                    } else if pressed & PAD_CROSS != 0 {
                        let chara = self.side[n].chara;
                        if !self.fits_dp(n, chara) || !self.is_chara_free(n, chara) {
                            se(sounds, 1, 7);
                        } else {
                            self.flash[plate].goto_label("fl_custom_in", true);
                            self.clip_goto(reel, n, Clip::FormChip, "fl_ok");
                            let plates = if self.side[n].flags & SIDE_CUSTOM != 0 {
                                2
                            } else {
                                4
                            };
                            self.open_plates(n, plates);
                            self.color_count[n] = self.wrap_costume(n);
                            self.side[n].state = ST_PLATE;
                            se(sounds, 1, 1);
                        }
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.side[n].flags ^= SIDE_FORM;
                        self.flash[reel].goto_label("fl_form", true);
                        self.side[n].mask = 1;
                        self.clip_goto(reel, n, Clip::FormChip, "fl_off_start");
                        if self.side[n].flags & SIDE_CUSTOM != 0 {
                            self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
                            self.set_custom_chips(n);
                            self.side[n].chara =
                                self.side[n].chip[0][self.side[n].cur_pick().custom_col as usize];
                            self.request_face(n);
                            self.side[n].state = ST_CUSTOM;
                        } else {
                            self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                            self.show_grid_row(n);
                            self.side[n].state = ST_GRID;
                        }
                        se(sounds, 2, 0x29);
                    }
                }
                ST_PLATE => {
                    if repeat & (PAD_UP | PAD_DOWN) != 0 {
                        let count = self.plate_count[n];
                        if count < 2 {
                            continue;
                        }
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_off_start");
                        let p = &mut self.side[n].cur_mut().custom_plate;
                        *p = if repeat & PAD_UP != 0 {
                            if *p - 1 < 0 { count - 1 } else { *p - 1 }
                        } else if *p + 1 < count {
                            *p + 1
                        } else {
                            0
                        };
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                        se(sounds, 1, 0);
                    } else if pressed & if n == 0 { PAD_RIGHT } else { PAD_LEFT } != 0 {
                        // Towards the middle of the screen: look at the item set of the plate.
                        let p = self.side[n].cur_pick();
                        let cell = self.cell(n);
                        if p.custom_plate <= 0 || cell.id == ID_RANDOM {
                            continue;
                        }
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_ok");
                        let chara = self.side[n].chara;
                        if cell.id == ID_CUSTOM {
                            let slot = (p.custom_col + p.custom_row * COLS as i32) as usize;
                            let items = save.rec_items.get(slot).copied().unwrap_or_default();
                            let level = save.rec_level.get(slot).copied().unwrap_or(0);
                            self.panels[n].set_chara(
                                chara,
                                items,
                                level.into(),
                                0,
                                true,
                                &self.chr,
                            );
                        } else {
                            let slot = (p.row * COLS as i32 + p.col) as usize;
                            let set = (p.custom_plate - 1) as usize;
                            let items = save.custom_items.get(slot).map_or([0; 8], |s| s[set]);
                            let level = save.custom_level.get(slot).copied().unwrap_or(0);
                            self.panels[n].set_chara(
                                chara,
                                items,
                                level.into(),
                                p.custom_plate - 1,
                                false,
                                &self.chr,
                            );
                        }
                        self.panels[n].show(sounds);
                        self.side[n].flags |= SIDE_PANEL;
                        self.side[n].state = ST_PANEL;
                    } else if pressed & PAD_CROSS != 0 {
                        self.flash[plate].goto_label("fl_color_in", true);
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_ok");
                        self.clip_goto(plate, n, Clip::ColorPlate, "fl_on_start");
                        self.side[n].state = ST_COLOR;
                        se(sounds, 1, 1);
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.flash[plate].goto_label("fl_custom_cansel", true);
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_off_start");
                        if self.side[n].flags & SIDE_FORM != 0 {
                            self.clip_goto(reel, n, Clip::FormChip, "fl_on_start");
                            self.side[n].state = ST_FORM;
                        } else if self.side[n].flags & SIDE_CUSTOM != 0 {
                            self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
                            self.side[n].state = ST_CUSTOM;
                        } else {
                            self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                            self.side[n].state = ST_GRID;
                        }
                        se(sounds, 1, 2);
                    }
                }
                ST_PANEL => {
                    let r = self.panels[n].input(pressed, repeat, sounds);
                    if r > 0 {
                        self.help.open();
                        self.item_info = (r - 1) as usize;
                        self.side[n].state = ST_PANEL_HELP;
                    } else if r < 0 {
                        self.panels[n].hide(sounds);
                        self.side[n].flags &= !SIDE_PANEL;
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                        self.side[n].state = ST_PLATE;
                    }
                }
                ST_PANEL_HELP => {
                    if pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                        self.help.close();
                        self.side[n].state = ST_PANEL;
                        se(sounds, 1, 2);
                    }
                }
                ST_COLOR => {
                    if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                        self.clip_goto(plate, n, Clip::ColorPlate, "fl_off_start");
                        let count = self.color_count[n];
                        let c = &mut self.side[n].cur_mut().color_plate;
                        *c = if repeat & PAD_LEFT != 0 {
                            if *c - 1 < 0 { count - 1 } else { *c - 1 }
                        } else if *c + 1 < count {
                            *c + 1
                        } else {
                            0
                        };
                        self.clip_goto(plate, n, Clip::ColorPlate, "fl_on_start");
                        se(sounds, 1, 0);
                    } else if pressed & PAD_CROSS != 0 {
                        self.confirm_member(n, save, random);
                        se(sounds, 1, 1);
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.flash[plate].goto_label("fl_color_cansel", true);
                        self.clip_goto(plate, n, Clip::ColorPlate, "fl_off_start");
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                        self.side[n].state = ST_PLATE;
                        se(sounds, 1, 2);
                    }
                }
                ST_TEAM => {
                    if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                        self.clip_goto(team, n, Clip::Team, "fl_off_start");
                        let (cur, count) = (self.side[n].cur, self.side[n].member_count);
                        self.side[n].cur = if repeat & PAD_LEFT != 0 {
                            match cur.checked_sub(1) {
                                None => CUR_MENU,
                                Some(c) if c > count => count,
                                Some(c) => c,
                            }
                        } else if cur + 1 > CUR_MENU {
                            0
                        } else if cur + 1 > count {
                            CUR_MENU
                        } else {
                            cur + 1
                        };
                        self.clip_goto(team, n, Clip::Team, "fl_on_start");
                        self.side[n].chara = if self.side[n].cur == CUR_MENU {
                            self.side[n].member[0].picked
                        } else {
                            self.side[n].cur_pick().picked
                        };
                        self.request_face(n);
                        se(sounds, 1, 0);
                    } else if pressed & PAD_CROSS != 0 {
                        if self.side[n].cur != CUR_MENU {
                            // Choose (or change) the member under the cursor.
                            self.sum_cost(n, true);
                            self.flash[team].goto_label("fl_out", true);
                            self.flash[reel].goto_label("fl_in", true);
                            self.clip_goto(team, n, Clip::Team, "fl_ok");
                            self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                            self.show_grid_row(n);
                            self.backup[n] = self.side[n].cur_pick();
                            self.side[n].state = ST_GRID;
                            se(sounds, 1, 1);
                        } else if self.side[n].member_count != 0 {
                            // The team is complete.
                            self.flash[team].goto_label("fl_out", true);
                            self.clip_goto(team, n, Clip::Team, "fl_ok");
                            self.side[n].state = ST_DONE;
                            if self.players != PLAYERS_TWO && self.base == 0 {
                                // One pad: on to the second team.
                                self.base = 1;
                                let n = i + 1;
                                self.clip_goto(n + 3, n, Clip::Chip, "fl_on_start");
                                self.side[n].state = ST_GRID;
                            }
                            se(sounds, 1, 1);
                        } else {
                            se(sounds, 1, 7);
                        }
                    } else if pressed & PAD_TRIANGLE != 0 {
                        // Take a member out.
                        let (cur, count) = (self.side[n].cur, self.side[n].member_count);
                        if count == 1 {
                            self.flash[team].goto_label("fl_out", true);
                            self.flash[reel].goto_label("fl_in", true);
                            self.clip_goto(team, n, Clip::Team, "fl_ok");
                            self.side[n].cur = 0;
                            self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                            self.show_grid_row(n);
                            self.side[n].state = ST_GRID;
                        } else if cur == CUR_MENU {
                            self.remove_member(n, count - 1);
                        } else if cur == count {
                            self.remove_member(n, cur - 1);
                            self.clip_goto(team, n, Clip::Team, "fl_off_start");
                            self.side[n].cur -= 1;
                            self.clip_goto(team, n, Clip::Team, "fl_on_start");
                            self.side[n].chara = self.side[n].cur_pick().picked;
                            self.request_face(n);
                        } else {
                            self.remove_member(n, cur);
                            self.side[n].chara = self.side[n].cur_pick().picked;
                            self.request_face(n);
                        }
                        self.side[n].member_count = count.saturating_sub(1);
                        self.set_team_tex(n);
                        se(sounds, 1, 2);
                    }
                }
                ST_CUSTOM => {
                    if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_off_start");
                        let p = self.side[n].cur_pick();
                        let mut col = p.custom_col;
                        if repeat & PAD_LEFT != 0 {
                            chr_grid_move_left(&self.custom[n], &mut col, p.custom_row);
                        } else {
                            chr_grid_move_right(&self.custom[n], &mut col, p.custom_row);
                        }
                        self.side[n].cur_mut().custom_col = col;
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
                        self.side[n].chara = self.side[n].chip[0][col as usize];
                        self.request_face(n);
                        se(sounds, 2, 0);
                    } else if repeat & (PAD_UP | PAD_DOWN) != 0 {
                        let up = repeat & PAD_UP != 0;
                        self.flash[reel]
                            .goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                        self.side[n].mask = 1;
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_off_start");
                        let p = self.side[n].cur_pick();
                        let (mut col, mut row) = (p.custom_col, p.custom_row);
                        if up {
                            chr_grid_move_up(&self.custom[n], &mut col, &mut row, CUSTOM_ROWS);
                        } else {
                            chr_grid_move_down(&self.custom[n], &mut col, &mut row, CUSTOM_ROWS);
                        }
                        let m = self.side[n].cur_mut();
                        (m.custom_col, m.custom_row) = (col, row);
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
                        self.set_custom_chips(n);
                        self.side[n].chara = self.side[n].chip[0][col as usize];
                        self.request_face(n);
                        se(sounds, 2, if up { 2 } else { 1 });
                    } else if pressed & PAD_CROSS != 0 {
                        let chara = self.side[n].chara;
                        let cell = self.custom_cell(n);
                        if !self.fits_dp(n, chara) {
                            se(sounds, 1, 7);
                        } else if cell.form_count != 0 {
                            self.side[n].flags |= SIDE_FORM;
                            if self.side[n].cur_pick().form >= cell.form_count {
                                self.side[n].cur_mut().form = 0;
                            }
                            self.flash[reel].goto_label("fl_form", true);
                            self.side[n].mask = 1;
                            self.clip_goto(reel, n, Clip::CustomChip, "fl_off_start");
                            self.clip_goto(reel, n, Clip::FormChip, "fl_on_start");
                            self.set_cell_form_chips(n, cell);
                            self.side[n].chara =
                                self.side[n].chip[0][self.side[n].cur_pick().form as usize];
                            self.request_face(n);
                            self.side[n].state = ST_FORM;
                            se(sounds, 2, 0x29);
                        } else if self.is_chara_free(n, chara) {
                            self.flash[plate].goto_label("fl_custom_in", true);
                            self.clip_goto(reel, n, Clip::CustomChip, "fl_ok");
                            self.open_plates(n, 2);
                            self.color_count[n] = self.wrap_costume(n);
                            self.side[n].state = ST_PLATE;
                            se(sounds, 1, 1);
                        } else {
                            se(sounds, 1, 7);
                        }
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.side[n].flags ^= SIDE_CUSTOM;
                        self.flash[reel].goto_label("fl_form", true);
                        self.side[n].mask = 1;
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_off_start");
                        self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                        self.show_grid_row(n);
                        self.side[n].state = ST_GRID;
                        se(sounds, 2, 0x29);
                    }
                }
                ST_DONE => {
                    if self.flags & FLAG_STAGE != 0 {
                        continue;
                    }
                    if self.side[n ^ 1].state == ST_DONE {
                        // Both teams are complete: on to the stage.
                        self.flash[0].goto_label("fl_map", true);
                        self.icon.open();
                        self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                        self.flags |= FLAG_STAGE;
                        self.face_mask = 1;
                        self.stage.state = ST_STAGE;
                    } else if pressed & PAD_TRIANGLE != 0 {
                        // Two pads: reopen this side's team while the other is still choosing.
                        self.flash[team].goto_label("fl_fast_in", true);
                        self.clip_goto(team, n, Clip::Team, "fl_on_start");
                        self.side[n].state = ST_TEAM;
                        se(sounds, 1, 2);
                    }
                }
                _ => {}
            }
        }
    }

    /// Cross on the costume: the member is complete (the random cell draws a character no other member is),
    /// with the chosen set's items; a new member moves the cursor on.
    fn confirm_member(&mut self, n: usize, save: &SaveData, random: &mut GameRandom) {
        let (team, reel, plate) = (n + 1, n + 3, n + 5);
        self.side[n].mask = 0;
        self.flash[plate].goto_label("fl_color_ok", true);
        self.flash[reel].goto_label("fl_out", true);
        self.flash[team].goto_label("fl_in", true);
        self.clip_goto(plate, n, Clip::ColorPlate, "fl_ok");
        self.clip_goto(plate, n, Clip::CustomPlate, "fl_off_start");
        let s = self.side[n];
        let p = s.cur_pick();
        let set = (p.custom_plate - 1).max(0) as usize;
        let has_set = p.custom_plate != 0;
        let rec = |slot: i32| {
            save.rec_items
                .get(slot as usize)
                .copied()
                .unwrap_or_default()
        };
        let custom = |slot: usize| save.custom_items.get(slot).map_or([0; 8], |c| c[set]);
        let custom_slot = p.custom_col + p.custom_row * COLS as i32;
        let (chara, items, clip) = if s.flags & SIDE_FORM != 0 {
            let items = match has_set {
                false => [0; 8],
                true if self.cell(n).id == ID_CUSTOM => rec(custom_slot),
                true => custom((p.row * COLS as i32 + p.col) as usize),
            };
            (s.chip[0][p.form as usize], items, Clip::FormChip)
        } else if s.flags & SIDE_CUSTOM != 0 {
            let items = if has_set { rec(custom_slot) } else { [0; 8] };
            (s.chip[0][p.custom_col as usize], items, Clip::CustomChip)
        } else {
            let (chara, slot) = if self.cell(n).id == ID_RANDOM {
                let cells = &self.cells[n];
                loop {
                    let k = loop {
                        let k = random.range(self.master_count[n] as u32) as usize;
                        if cells[k].id < ID_RANDOM {
                            break k;
                        }
                    };
                    let mut chara = cells[k].id;
                    if cells[k].form_count != 0 {
                        chara = cells[k].form[random.range(cells[k].form_count as u32) as usize];
                    }
                    if self.is_chara_free(n, chara) {
                        break (chara, k);
                    }
                }
            } else {
                (
                    s.chip[0][p.col as usize],
                    (p.col + p.row * COLS as i32) as usize,
                )
            };
            let items = if has_set { custom(slot) } else { [0; 8] };
            (chara, items, Clip::Chip)
        };
        let m = self.side[n].cur_mut();
        m.picked = chara;
        m.items = items;
        self.clip_goto(reel, n, clip, "fl_off_start");
        self.clip_goto(team, n, Clip::Team, "fl_off_start");
        let side = &mut self.side[n];
        if side.cur == side.member_count {
            // A new member: the cursor goes to the next free slot.
            side.member_count = side.cur + 1;
            side.cur = side.member_count;
        }
        self.set_team_tex(n);
        self.clip_goto(team, n, Clip::Team, "fl_on_start");
        if self.side[n].cur == CUR_MENU {
            self.side[n].chara = self.side[n].member[0].picked;
            self.request_face(n);
        }
        self.side[n].flags &= !(SIDE_FORM | SIDE_CUSTOM);
        self.side[n].state = ST_TEAM;
    }

    /// The stage and music choice of pad 0.
    fn input_stage(&mut self, (pressed, repeat): (u32, u32), sounds: &mut Vec<TitleSound>) {
        let se = |sounds: &mut Vec<TitleSound>, bank: u32, id: u32| {
            sounds.push(TitleSound::Effect(bank, id))
        };
        match self.stage.state {
            ST_STAGE => {
                let on_music = self.stage.col == STAGE_BGM_COL;
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.clip_goto(0, 0, Clip::StageChip, "fl_off_start");
                    let mut col = self.stage.col;
                    if repeat & PAD_LEFT != 0 {
                        stg_grid_move_left(&self.stage_ids, &mut col, self.stage.row);
                    } else {
                        stg_grid_move_right(&self.stage_ids, &mut col, self.stage.row);
                    }
                    self.stage.col = col;
                    self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                    if col != STAGE_BGM_COL {
                        self.stage.stage =
                            self.stage_ids[(self.stage.row * STAGE_COLS + col) as usize];
                        self.request_stage();
                    }
                    se(sounds, 2, 0);
                } else if repeat & (PAD_UP | PAD_DOWN) != 0 && !on_music {
                    let up = repeat & PAD_UP != 0;
                    self.flash[0].goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                    self.stage.mask = 1;
                    self.clip_goto(0, 0, Clip::StageChip, "fl_off_start");
                    let (mut col, mut row) = (self.stage.col, self.stage.row);
                    // The rows passed are the six columns, as in the original.
                    if up {
                        stg_grid_move_up(&self.stage_ids, &mut col, &mut row, STAGE_COLS);
                    } else {
                        stg_grid_move_down(&self.stage_ids, &mut col, &mut row, STAGE_COLS);
                    }
                    (self.stage.col, self.stage.row) = (col, row);
                    self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                    self.set_stage_chips();
                    self.stage.stage = self.stage_ids[(row * STAGE_COLS + col) as usize];
                    self.request_stage();
                    se(sounds, 2, if up { 2 } else { 1 });
                } else if pressed & PAD_CROSS != 0 {
                    self.stage.mask = 0;
                    self.clip_goto(0, 0, Clip::StageChip, "fl_ok");
                    if on_music {
                        // The music chip: open the music list.
                        self.flash[0].goto_label("fl_bgm", true);
                        self.set_icon(1);
                        self.stage.bgm_cursor = self.stage.bgm;
                        self.stage.state = ST_BGM;
                    } else {
                        // The stage is chosen: the battle starts after 60 frames.
                        self.flash[0].goto_label("fl_vs", true);
                        self.icon.close();
                        self.face_mask = 0;
                        self.flags |= FLAG_DECIDED | FLAG_LEAVING;
                        self.timer = 0x3C;
                    }
                    se(sounds, 1, 1);
                } else if pressed & PAD_TRIANGLE != 0 {
                    // Back to the teams.
                    self.stage.mask = 0;
                    self.flags ^= FLAG_STAGE;
                    self.flash[0].goto_label("fl_map_cansel", true);
                    self.icon.close();
                    self.clip_goto(0, 0, Clip::StageChip, "fl_off_start");
                    let sides: &[usize] = if self.players != PLAYERS_TWO {
                        &[1]
                    } else {
                        &[0, 1]
                    };
                    for &j in sides {
                        self.flash[1 + j].goto_label("fl_fast_in", true);
                    }
                    for &j in sides {
                        self.clip_goto(1 + j, j, Clip::Team, "fl_on_start");
                        self.side[j].state = ST_TEAM;
                    }
                    se(sounds, 1, 2);
                }
            }
            ST_BGM => {
                if repeat & PAD_UP != 0 {
                    self.stage.bgm_cursor -= 1;
                    if self.stage.bgm_cursor < 0 {
                        self.stage.bgm_cursor = self.bgm_count - 1;
                    }
                    se(sounds, 1, 0);
                } else if repeat & PAD_DOWN != 0 {
                    self.stage.bgm_cursor += 1;
                    if self.stage.bgm_cursor > self.bgm_count - 1 {
                        self.stage.bgm_cursor = 0;
                    }
                    se(sounds, 1, 0);
                } else if pressed & PAD_CROSS != 0 {
                    let id = self.bgm_ids[self.stage.bgm_cursor as usize];
                    if id != BGM_LOCKED {
                        self.flash[0].goto_label("fl_bgm_cansel", true);
                        self.set_icon(0);
                        self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                        self.stage.bgm = self.stage.bgm_cursor;
                        if id != BGM_RANDOM {
                            sounds.push(TitleSound::Bgm(BGM_FIRST + id as u32));
                        }
                        self.stage.state = ST_STAGE;
                        se(sounds, 1, 1);
                    } else {
                        se(sounds, 1, 7);
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[0].goto_label("fl_bgm_cansel", true);
                    self.set_icon(0);
                    self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                    self.stage.bgm_cursor = self.stage.bgm;
                    self.stage.state = ST_STAGE;
                    se(sounds, 1, 2);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::char_select::CharSelAssets;

    /// Common file 4's costs and form links from the ISO in `rom/`, when it is there.
    #[test]
    fn real_costs_and_forms_relate_members() {
        use bt3_data::Iso9660;
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(4);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let common = archive.read_entry(&mut image, location.index).unwrap();
        let chr = ChrTable::parse(&common, 0xA1);
        assert_eq!((chr.cost(0), chr.cost(5), chr.cost(0x1C)), (5, 8, 3));
        // Character 3's forms are 4..6 and each names the others; 0 has none.
        assert_eq!(chr.links[3], [4, 5, 6, 0xFF]);
        assert!(chr.is_related(3, 6) && chr.is_related(6, 3) && chr.is_related(0, 0));
        assert!(!chr.is_related(0, 3) && !chr.is_related(0x4F, 0x4F));
    }

    struct NoPictures;

    impl PictureLoader for NoPictures {
        fn portrait(&mut self, _: u32) -> Option<usize> {
            Some(1000)
        }
        fn picture(&mut self, _: u32) -> Option<usize> {
            Some(2000)
        }
    }

    /// Runs the real team select from the ISO in `rom/`, when it is there: a DP battle (limit 10, every character
    /// costing 3), 1P vs COM. Side 0 takes three members and cannot afford a fourth, side 1 takes one, then the
    /// stage.
    #[test]
    fn real_team_select_sets_up_a_dp_team_battle() {
        use crate::duel_menu::ARCHIVE_FILE_ID;
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe, pack_section};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res =
            bpe::decode(pack_section(&pack, crate::char_select::ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = CharSelAssets::parse(&res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = CharSelLayout::new(|key| firsts[&key]);
        let save = SaveData::default();
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[2, 7, 1, 8]);
        let mut sounds = Vec::new();
        let duel = DuelProgress {
            players: 0,
            battle_type: 2,
            dp_limit: 0,
        };
        let mut screen = TeamSelScreen::new(
            &data,
            duel,
            CharSelProgress::default(),
            &save,
            ChrTable {
                costumes: vec![2; 0xA1],
                costs: vec![3; 0xA1],
                ..ChrTable::default()
            },
            &mut NoPictures,
            &mut random,
            &mut sounds,
        )
        .unwrap();
        assert_eq!(screen.dp_max, 10);
        let mut heard = Vec::new();
        macro_rules! step {
            ($bits:expr) => {{
                let frame = screen.run_frame(
                    [($bits, $bits), (0, 0)],
                    &mut NoPictures,
                    &save,
                    &mut random,
                    &mut font,
                );
                heard.extend(frame.sounds.iter().cloned());
                frame
            }};
        }
        macro_rules! wait {
            () => {
                for _ in 0..40 {
                    step!(0);
                }
            };
        }
        wait!();
        assert!(step!(0).quads.len() > 50);
        // A member: from the member list's empty slot (or the start) `moves` chips right, then confirmed through
        // forms, item sets and costume.
        macro_rules! pick {
            ($side:expr, $moves:expr) => {{
                if screen.side[$side].state == ST_TEAM {
                    step!(PAD_CROSS);
                    wait!();
                }
                assert_eq!(screen.side[$side].state, ST_GRID);
                for _ in 0..$moves {
                    step!(PAD_RIGHT);
                    wait!();
                }
                for _ in 0..4 {
                    step!(PAD_CROSS);
                    wait!();
                    if screen.side[$side].state == ST_TEAM {
                        break;
                    }
                }
                assert_eq!(screen.side[$side].state, ST_TEAM, "side {}", $side);
            }};
        }
        pick!(0, 0);
        pick!(0, 1);
        pick!(0, 2);
        let s = screen.side[0];
        println!("side 0: {:X?}, cost {}", s.member.map(|m| m.picked), s.cost);
        assert_eq!((s.member_count, s.cur, s.cost), (3, 3, 9));
        assert!(
            s.member[..3]
                .iter()
                .all(|m| m.picked >= 0 && m.picked < ID_RANDOM)
        );
        assert!(screen.team_tex[0][5].is_some() && screen.team_tex[0][6].is_none());
        // A fourth would pass the limit: refused, and cancel brings the list back as it was.
        step!(PAD_CROSS);
        wait!();
        step!(PAD_RIGHT);
        wait!();
        heard.clear();
        step!(PAD_CROSS);
        wait!();
        assert_eq!(screen.side[0].state, ST_GRID);
        assert!(heard.contains(&TitleSound::Effect(1, 7)));
        step!(PAD_TRIANGLE);
        wait!();
        assert_eq!(screen.side[0].state, ST_TEAM);
        assert_eq!((screen.side[0].member_count, screen.side[0].cost), (3, 9));
        // On to the menu plate: the team is complete, and the one pad goes on to side 1.
        step!(PAD_RIGHT);
        wait!();
        assert_eq!(screen.side[0].cur, CUR_MENU);
        step!(PAD_CROSS);
        wait!();
        assert_eq!((screen.side[0].state, screen.base), (ST_DONE, 1));
        pick!(1, 0);
        step!(PAD_RIGHT);
        wait!();
        step!(PAD_CROSS);
        wait!();
        assert_eq!(screen.side[1].state, ST_DONE);
        wait!();
        assert_eq!(screen.stage.state, ST_STAGE);
        step!(PAD_CROSS);
        for _ in 0..120 {
            step!(0);
        }
        let Some(CharSelResult::Battle(setup)) = screen.result() else {
            panic!("no battle: {:?}", screen.result());
        };
        println!("{setup:?}");
        let members = screen.side[0].member;
        assert_eq!(setup.sides[0].chara, members[0].picked);
        assert_eq!(setup.sides[0].team_count, 2);
        assert_eq!(
            [setup.sides[0].team[0].0, setup.sides[0].team[1].0],
            [members[1].picked, members[2].picked]
        );
        assert_eq!(setup.sides[1].team_count, 0);
        assert_eq!((setup.sides[0].control, setup.sides[1].control), (0, 2));
        assert_eq!(setup.sides[1].cpu_level, 13);
        assert_eq!(screen.progress().team[0][1].picked, members[2].picked);
    }
}
