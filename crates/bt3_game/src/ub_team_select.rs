//! Ultimate Battle's team select (`UbTeamSel_Run(4)`, src/menu/ub_team_select.c, progress mode 15 when the mission
//! wants two fighters or more, `Ub_Main`): the one-sided version of the versus team select with Androids 17 and 18
//! as guides. Up to the mission's number of fighters are chosen one after another from the character grid (forms,
//! item set, costume), never two forms of the same person, under the mission's DP limit when it has one; the team
//! plates change or remove a member, the menu plate confirms the team.

use std::error::Error;

use bt3_data::chara_data::ChrTable;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{
    COLS, Cell, GRID_NO_CUSTOM, GRID_NO_RANDOM, ID_CUSTOM, ID_LOCKED, ID_RANDOM, Pick,
    PictureLoader, chr_grid_build_with, chr_grid_move_down, chr_grid_move_left,
    chr_grid_move_right, chr_grid_move_up, chr_grid_next_form, chr_grid_prev_form,
};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::item_panel::{ItemHelp, ItemPanel};
use crate::logo::{ColorFade, picture_quads};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::solo_select::SoloSelectData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::ub::VOICE_BASE;
use crate::windows::{MsgWin, TextBox, TextDraw, draw_texts};

const FACE_FILE: u32 = 0x2F9;
const FADE_FRAMES: i32 = 0x14;
/// The reel's chip images (`UbTeamSel_SetChips`' order).
const CHIP_IMAGES: [usize; 7] = [8, 11, 12, 13, 14, 15, 16];
/// The team movie's images of the five members (`UbTeamSel_SetTeamTex`).
const MEMBER_IMAGES: [usize; 5] = [1, 4, 5, 6, 7];
const PORTRAIT: usize = 17;
/// `UB_ID_EMPTY`: an empty chip.
const ID_EMPTY: i32 = 0xA4;
/// `UB_MEMBER_MAX`, and `UB_CUR_MENU`: the cursor on the menu plate.
pub const MEMBER_MAX: usize = 5;
const CUR_MENU: i32 = 5;

// UBTEAM_STEP_*.
const STEP_CHARA: i32 = 0;
const STEP_FORM: i32 = 1;
const STEP_PLATE: i32 = 2;
const STEP_PANEL: i32 = 3;
const STEP_HELP: i32 = 4;
const STEP_COLOR: i32 = 5;
const STEP_TEAM: i32 = 6;

// UBSEL_*.
const STARTED: u32 = 2;
const DONE: u32 = 8;
const LEAVING: u32 = 0x10;
const DP: u32 = 0x20;
const NO_CUSTOM: u32 = 0x40;
const GREETED: u32 = 0x80;
// UBTEAM_SEL_*.
const FACE_CHANGE: u32 = 1;
const FACE_READY: u32 = 2;
const SEL_FORM: u32 = 0x40;
const SEL_REC: u32 = 0x80;
const NO_FACE: u32 = 0x400;
const SEL_PANEL: u32 = 0x800;

// UBSEL_LOAD_*.
const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;
const LOAD_IDLE: i32 = 4;
const LOAD_ABORT: i32 = 5;
const LOAD_RESTART: i32 = 6;

// UBSEL_END_*.
const END_NONE: i32 = 0;
const END_SPEAK: i32 = 1;
const END_WAIT: i32 = 2;
const END_LEAVE: i32 = 3;

// UBTEAM_CLIP_*.
const CLIP_CHIP: i32 = 0;
const CLIP_FORM_CHIP: i32 = 1;
const CLIP_PLATE: i32 = 2;
const CLIP_COLOR: i32 = 5;
const CLIP_TEAM: i32 = 6;

// Movies.
const FL_FACE: usize = 0;
const FL_TEAM: usize = 1;
const FL_REEL: usize = 2;
const FL_PLATES: usize = 3;

/// `UbTeamState`: the team being chosen.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    team: [Pick; MEMBER_MAX],
    /// The characters on the reel's seven chips, and those before the last change.
    chip: [[i32; 7]; 2],
    flags: u32,
    cur: i32,
    cost: i32,
    member_count: i32,
    chara: i32,
    step: i32,
    mask: i32,
}

/// `UbTeamSel` and the frame loop of `UbTeamSel_Run`.
pub struct UbTeamSelectScreen {
    data: SoloSelectData,
    flash: [Flash; 4],
    tex: [Vec<Option<usize>>; 4],
    msg: MsgWin,
    panel: ItemPanel,
    help: ItemHelp,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    chr: ChrTable,
    flags: u32,
    voice_line: i32,
    load_state: i32,
    loading: Option<usize>,
    timer: i32,
    help_item: i32,
    sel: State,
    backup: Pick,
    grid: Vec<Cell>,
    rows: i32,
    plate_count: i32,
    color_count: i32,
    team_max: i32,
    dp_level: i32,
    dp_max: i32,
    face_alpha: f32,
    end_step: i32,
    talker: usize,
    blink: [i32; 2],
    talk: [i32; 2],
    result: i32,
    done: Option<i32>,
}

impl UbTeamSelectScreen {
    /// `UbTeamSel_Init(4)` and the first `ColorFade_StartIn`: `team_size` and `dp_rule` are the mission's (from
    /// `gProgress`), `team` is `gProgress->team` (its cursors are kept, an unusable one reset).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &SoloSelectData,
        team_size: i32,
        dp_rule: i32,
        chr: &ChrTable,
        save: &SaveData,
        team: &mut [Pick; MEMBER_MAX],
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let (flags, dp_max) = match dp_rule {
            1 => (DP, 10),
            2 => (DP, 15),
            3 => (DP, 20),
            _ => (0, 0),
        };
        let mut tex = [
            data.layout.flash[0].clone(),
            data.layout.team.clone(),
            data.layout.flash[1].clone(),
            data.layout.flash[2].clone(),
        ];
        if flags & DP != 0 {
            let dp = &data.layout.dp;
            tex[FL_FACE][5] = dp[0];
            tex[FL_FACE][6] = dp[1];
            tex[FL_FACE][7] = dp[2];
            tex[FL_REEL][9] = dp[3];
            tex[FL_TEAM][2] = dp[3];
        }
        let make = |fod: &[u8]| -> Result<Flash, Box<dyn Error>> {
            let mut f = Flash::create(fod)?;
            f.play(1);
            Ok(f)
        };
        let flash = [
            make(&data.fods[0])?,
            make(&data.team_fod)?,
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
        // ChrGrid_Build in modes 13..30: no custom character nor random cell.
        let (grid, _, _) = chr_grid_build_with(&data.master, save, GRID_NO_CUSTOM | GRID_NO_RANDOM);
        let rows = (grid.len() as i32 + COLS as i32 - 1) / COLS as i32;
        let mut sel = State {
            team: *team,
            ..State::default()
        };
        // A remembered cursor on a cell that cannot be chosen any more goes back to the first cell.
        let selectable = |c: &Cell| c.id < ID_LOCKED || c.id == ID_CUSTOM;
        for (j, m) in sel.team.iter_mut().enumerate() {
            let index = (m.col + m.row * COLS as i32).max(0) as usize;
            if !grid.get(index).is_some_and(selectable) {
                *m = Pick::default();
                team[j] = *m;
            }
        }
        let mut screen = Self {
            data: data.clone(),
            flash,
            tex,
            msg,
            panel: ItemPanel::new(0, &data.items, data.layout.panel.clone())?,
            help: ItemHelp::new(&data.items, data.layout.help.clone())?,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            chr: chr.clone(),
            flags,
            voice_line: -1,
            load_state: LOAD_IDLE,
            loading: None,
            timer: 0,
            help_item: 0,
            sel,
            backup: Pick::default(),
            grid,
            rows,
            plate_count: 0,
            color_count: 0,
            team_max: team_size,
            dp_level: (dp_rule - 1).max(0),
            dp_max,
            face_alpha: 0.0,
            end_step: END_NONE,
            talker: 0,
            blink: [0; 2],
            talk: [0; 2],
            result: 1,
            done: None,
        };
        screen.set_chips();
        screen.sel.chara = screen.sel.chip[0][screen.cur().col.clamp(0, 6) as usize];
        for m in &mut screen.sel.team {
            m.picked = -1;
        }
        for b in &mut screen.blink {
            *b = random.range(0x20) as i32;
        }
        screen.tex[FL_FACE][PORTRAIT] = loader.portrait(FACE_FILE + screen.sel.chara.max(0) as u32);
        screen.sel.flags |= FACE_READY;
        Ok(screen)
    }

    /// `UbTeamSel_Run`'s result once it has returned: 1 a team was chosen, 0 backed out.
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// The team (`gProgress->team`) and its size as they were handed over.
    pub fn team(&self) -> ([Pick; MEMBER_MAX], i32) {
        (self.sel.team, self.sel.member_count)
    }

    fn cur(&self) -> Pick {
        self.sel.team[self.sel.cur.clamp(0, MEMBER_MAX as i32 - 1) as usize]
    }

    fn cur_mut(&mut self) -> &mut Pick {
        &mut self.sel.team[self.sel.cur.clamp(0, MEMBER_MAX as i32 - 1) as usize]
    }

    /// `UT_CELL`: the grid cell under the cursor.
    fn cell(&self) -> Cell {
        let c = self.cur();
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

    fn cost_of(&self, chara: i32) -> i32 {
        usize::try_from(chara)
            .ok()
            .and_then(|c| self.chr.costs.get(c).copied())
            .unwrap_or(0)
    }

    /// `UbTeamSel_SumCost`: the DP total of the team (without the member being chosen with `skip_cur`).
    fn sum_cost(&mut self, skip_cur: bool) {
        if self.flags & DP == 0 {
            return;
        }
        self.sel.cost = (0..self.sel.member_count)
            .filter(|&i| i != self.sel.cur || !skip_cur)
            .map(|i| self.cost_of(self.sel.team[i as usize].picked))
            .sum();
    }

    /// `UbTeamSel_IsCharaFree`: no other member is the same person.
    fn is_chara_free(&self, chara: i32) -> bool {
        !(0..self.sel.member_count).any(|i| {
            i != self.sel.cur && self.chr.is_related(chara, self.sel.team[i as usize].picked)
        })
    }

    /// `UbTeamSel_FitsDp`: the team can still afford `chara`.
    fn fits_dp(&self, chara: i32) -> bool {
        if self.flags & DP == 0 || chara == ID_RANDOM || chara == ID_CUSTOM {
            return true;
        }
        self.sel.cost + self.cost_of(chara) <= self.dp_max
    }

    /// `UbTeamSel_RemoveMember`: the members behind move up, the last slot empties.
    fn remove_member(&mut self, idx: i32) {
        let empty = Pick {
            picked: -1,
            ..Pick::default()
        };
        let idx = idx.clamp(0, MEMBER_MAX as i32 - 1) as usize;
        self.sel.team[idx] = empty;
        let count = self.sel.member_count;
        if (idx as i32) < count {
            for i in idx..(count - 1).max(0) as usize {
                self.sel.team[i] = self.sel.team[i + 1];
            }
            let last = (count - 1).clamp(0, MEMBER_MAX as i32 - 1) as usize;
            self.sel.team[last] = empty;
        }
    }

    fn set_row(&mut self, chars: [i32; 7]) {
        for (i, &chara) in chars.iter().enumerate() {
            self.sel.chip[1][i] = self.sel.chip[0][i];
            self.sel.chip[0][i] = chara;
            self.tex[FL_REEL][18 + i] = self.tex[FL_REEL][CHIP_IMAGES[i]];
            self.tex[FL_REEL][CHIP_IMAGES[i]] = self.chip(chara);
        }
    }

    /// `UbTeamSel_SetChips`: the reel shows the cursor's grid row.
    fn set_chips(&mut self) {
        let row = self.cur().row.max(0) as usize;
        let chars =
            std::array::from_fn(|i| self.grid.get(row * COLS + i).map_or(ID_EMPTY, |c| c.id));
        self.set_row(chars);
    }

    /// `UbTeamSel_SetFormChips`: the reel shows the forms of the cell (empty chips past the last).
    fn set_form_chips(&mut self) {
        let cell = self.cell();
        let chars = std::array::from_fn(|i| {
            if (i as i32) < cell.form_count {
                cell.form[i]
            } else {
                ID_EMPTY
            }
        });
        self.set_row(chars);
    }

    /// `UbTeamSel_SetTeamTex`: the members' chips on the team movie (the slots the team may not use show the
    /// empty chip), and the DP total.
    fn set_team_tex(&mut self) {
        if self.flags & DP != 0 {
            self.sel.cost = 0;
        }
        for (i, &image) in MEMBER_IMAGES.iter().enumerate() {
            self.tex[FL_TEAM][image] = None;
            if (i as i32) < self.sel.member_count {
                let id = self.sel.team[i].picked;
                self.tex[FL_TEAM][image] = self.chip(id);
                if self.flags & DP != 0 {
                    self.sel.cost += self.cost_of(id);
                }
            } else if i as i32 >= self.team_max {
                self.tex[FL_TEAM][image] = self.chip(ID_EMPTY);
            }
        }
    }

    /// `UbTeamSel_UpdateFaceLoad`: one step of the portrait's loader.
    fn update_face_load(&mut self, loader: &mut dyn PictureLoader) {
        match self.load_state {
            LOAD_ABORT => self.load_state = LOAD_RESTART,
            LOAD_RESTART => {
                self.sel.flags &= !FACE_CHANGE;
                self.load_state = LOAD_REQUEST;
            }
            LOAD_REQUEST => {
                if self.sel.chara < 0 {
                    self.sel.flags |= NO_FACE | FACE_READY;
                    self.load_state = LOAD_IDLE;
                } else {
                    self.loading = loader.portrait(FACE_FILE + self.sel.chara as u32);
                    self.load_state = LOAD_READ;
                }
            }
            // File_UpdateRequests: the read is done by the next call.
            LOAD_READ => self.load_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.tex[FL_FACE][PORTRAIT] = self.loading;
                self.sel.flags |= FACE_READY;
                self.load_state = LOAD_IDLE;
            }
            LOAD_IDLE => {
                if self.sel.flags & FACE_CHANGE != 0 {
                    self.tex[FL_FACE][PORTRAIT] = None;
                    self.load_state = LOAD_RESTART;
                }
            }
            _ => {}
        }
    }

    /// `UbTeamSel_RequestFace`.
    fn request_face(&mut self) {
        if self.sel.flags & FACE_READY != 0 {
            self.sel.flags ^= FACE_READY;
        } else {
            self.load_state = LOAD_ABORT;
        }
        self.sel.flags &= !NO_FACE;
        self.sel.flags |= FACE_CHANGE;
    }

    /// `UbTeamSel_ClipGoto(movie, kind, label)`.
    fn clip_goto(&mut self, movie: usize, kind: i32, label: &str) {
        let c = self.cur();
        let name = match kind {
            CLIP_CHIP => format!("mc_chara_chip_{:03}", c.col),
            CLIP_FORM_CHIP => format!("mc_chara_chip_{:03}", c.form),
            CLIP_PLATE => format!("mc_custom_plate_{}", c.custom_plate + 1),
            CLIP_COLOR => format!("mc_color_plate_{}", c.color_plate + 1),
            CLIP_TEAM => {
                if self.sel.cur == CUR_MENU {
                    "mc_menu_plate".to_owned()
                } else {
                    format!("mc_team_{}", self.sel.cur)
                }
            }
            _ => return,
        };
        let r = self.flash[movie].find(None, &name);
        self.flash[movie].clip_goto_label(r, label);
    }

    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        self.talker = 0;
        self.voice_line = line;
        anim::play_with_subtitle(&self.data.lips, VOICE_BASE, line, &mut self.lip, sounds);
    }

    /// One iteration of `UbTeamSel_Run`'s loop.
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
        self.update_face_load(loader);
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        // UbTeamSel_Update.
        for f in &mut self.flash {
            f.advance();
        }
        self.panel.update();
        if self.sel.mask != 0 && self.flash[FL_REEL].trig() & 1 != 0 {
            self.sel.mask = 0;
        }
        self.update_end(pressed, voice_stat, &mut frame.sounds);
        self.draw(voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & GREETED == 0
            && self.flash[FL_FACE].flags() & FLASH_PAD != 0
        {
            self.flags |= GREETED;
            self.say(0x36, &mut frame.sounds);
        }
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::FadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            // The portrait loader must be idle before the buffers go.
            if self.load_state == LOAD_IDLE {
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

    /// `UbTeamSel_UpdateEnd`: once the team is chosen, the guide's closing line, then the leave timer.
    fn update_end(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        match self.end_step {
            END_SPEAK => {
                self.say(0x37, sounds);
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

    /// `UbTeamSel_Draw`.
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
            let f = &mut self.flash[FL_FACE];
            for k in 0..2 {
                // The two guides: only the one talking moves its mouth.
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
            if self.sel.flags & (NO_FACE | FACE_READY) == FACE_READY {
                self.face_alpha = (self.face_alpha + 0.075).min(1.0);
            } else {
                self.face_alpha = 0.0;
            }
            let r = f.find(None, "mc_single_chara_l");
            f.clip_set_alpha(r, self.face_alpha);
            let chara = self.sel.chara;
            let r = f.find(None, "mc_name_text_l");
            texts.extend(TextBox::preset(1).attach_line(
                f,
                r,
                0x400,
                &self.data.names,
                chara,
                font,
            ));
            let r = f.find(None, "mc_form_text_l");
            texts.extend(TextBox::preset(3).attach_line(
                f,
                r,
                0x401,
                &self.data.forms,
                chara,
                font,
            ));
            if self.flags & DP != 0 {
                let (u, v) = ((self.dp_level % 2) * 0x40, (self.dp_level / 2) * 0x20);
                let r = f.find(None, "mc_dp_max_l");
                f.clip_set_uv(r, uv(u, v, u + 0x40, v + 0x20));
                anim::num_draw(
                    f,
                    |n| format!("mc_dp_total_l_{n}"),
                    2,
                    self.sel.cost,
                    (0x20, 0x40),
                    1,
                );
            }
        }
        {
            let f = &mut self.flash[FL_PLATES];
            let r = f.find(None, "mc_yajirusi");
            f.clip_set_uv(r, uv(0x20, 0, 0x40, 0x20));
            for i in 0..4 {
                // The item set plates: dimmed when they cannot be chosen.
                let name = format!("mc_custom_plate_{}", i + 1);
                let r = f.find(None, &name);
                f.clip_set_color(r, if i < self.plate_count { 1.0 } else { 0.3 });
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
        // The reel's chips are dimmed when the character cannot join (DP and the same person).
        let chips: Vec<(i32, i32, bool)> = (0..14)
            .map(|i| {
                let chara = self.sel.chip[i / 7][i % 7];
                let cost = if chara < ID_RANDOM {
                    self.cost_of(chara)
                } else {
                    0
                };
                (chara, cost, chara < ID_RANDOM && self.is_chara_free(chara))
            })
            .collect();
        let costs: Vec<i32> = (0..MEMBER_MAX)
            .map(|i| self.cost_of(self.sel.team[i].picked))
            .collect();
        {
            let dp = self.flags & DP != 0;
            let (cost_now, dp_max) = (self.sel.cost, self.dp_max);
            let f = &mut self.flash[FL_REEL];
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
            f.clip_set_flags(r, 0x102, self.sel.mask != 0);
            for (i, &(chara, cost, free)) in chips.iter().enumerate() {
                let name = format!("mc_chara_chip_{i:03}");
                let r = f.find(None, &name);
                f.clip_set_flags(r, 0x80, self.sel.mask as u8 != 0);
                let r = f.find(Some(&name), &format!("mc_chara_{i:03}"));
                let color = if dp && cost_now + cost > dp_max {
                    0.4
                } else if chara < ID_RANDOM {
                    if free { 1.0 } else { 0.4 }
                } else {
                    1.0
                };
                f.clip_set_color(r, color);
                if dp {
                    let r = f.find(Some(&name), &format!("mc_dp_num_{i:03}"));
                    if chara < ID_RANDOM {
                        let (u, v) = ((cost % 4) * 0x20, (cost / 4) * 0x20);
                        f.clip_set_uv(r, uv(u, v, u + 0x20, v + 0x20));
                        f.clip_set_flags(r, PROP_VISIBLE, true);
                    } else {
                        f.clip_set_flags(r, PROP_VISIBLE, false);
                    }
                }
            }
        }
        {
            let count = self.sel.member_count;
            let f = &mut self.flash[FL_TEAM];
            for (i, &cost) in costs.iter().enumerate() {
                // The team plates and each member's cost.
                let i = i as i32;
                let name = format!("mc_team_{i}");
                let (u, v) = ((i % 4) * 0x40, (i / 4) * 0x48);
                let r = f.find(Some(&name), &format!("mc_team_plate_{i}"));
                f.clip_set_uv(r, uv(u, v, u + 0x40, v + 0x48));
                let r = f.find(Some(&name), &format!("mc_dp_num_{i:03}"));
                if i < count {
                    let (u, v) = ((cost % 4) * 0x20, (cost / 4) * 0x20);
                    f.clip_set_flags(r, PROP_VISIBLE, true);
                    f.clip_set_uv(r, uv(u, v, u + 0x20, v + 0x20));
                } else {
                    f.clip_set_flags(r, PROP_VISIBLE, false);
                }
            }
        }
        let draw = |s: &Self, i: usize, quads: &mut Vec<ScreenQuad>| {
            let tex = &s.tex[i];
            let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
            let (fq, overs) = s.flash[i].draw_with_overs(&has_texture);
            quads.extend(screen_quads(&fq, tex));
            overs
        };
        // The names are queued with movie 0 and flushed over the team and the reel (TextBox_SetNoFlush).
        let overs = draw(self, FL_FACE, quads);
        draw_texts(&texts, &overs, font);
        draw(self, FL_TEAM, quads);
        draw(self, FL_REEL, quads);
        font.flush(quads);
        draw(self, FL_PLATES, quads);
        self.msg.draw(self.voice_line, font, quads);
        self.panel.draw(&self.chr, font, quads);
        self.help
            .draw(self.help_item.max(0) as usize, &self.chr, font, quads);
    }

    /// `ChrTbl_WrapCostume(chara, &color)`.
    fn wrap_costume(&mut self) -> i32 {
        let count = usize::try_from(self.sel.chara)
            .ok()
            .and_then(|c| self.chr.costumes.get(c).copied())
            .unwrap_or(1);
        if self.cur().color_plate >= count {
            self.cur_mut().color_plate = 0;
        }
        count
    }

    /// To the item set plates of the member being chosen.
    fn open_plates(&mut self, from: i32, sounds: &mut Vec<TitleSound>) {
        self.flash[FL_PLATES].goto_label("fl_custom_in", true);
        self.clip_goto(FL_REEL, from, "fl_ok");
        self.plate_count = if self.flags & NO_CUSTOM != 0 { 1 } else { 4 };
        if self.cur().custom_plate >= self.plate_count {
            self.cur_mut().custom_plate = 0;
        }
        self.clip_goto(FL_PLATES, CLIP_PLATE, "fl_on_start");
        self.color_count = self.wrap_costume();
        self.sel.step = STEP_PLATE;
        sounds.push(TitleSound::Effect(1, 1));
    }

    /// The face of the team plate under the cursor (the first member's on the menu plate).
    fn show_cur_face(&mut self) {
        self.sel.chara = if self.sel.cur == CUR_MENU {
            self.sel.team[0].picked
        } else {
            self.cur().picked
        };
        self.request_face();
    }

    /// `UbTeamSel_Input`.
    fn input(&mut self, pressed: u32, repeat: u32, save: &SaveData, sounds: &mut Vec<TitleSound>) {
        if self.flash[FL_FACE].flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.clip_goto(FL_REEL, CLIP_CHIP, "fl_on_start");
            self.flags |= STARTED;
        }
        match self.sel.step {
            STEP_CHARA => {
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.clip_goto(FL_REEL, CLIP_CHIP, "fl_off_start");
                    let (mut col, row) = (self.cur().col, self.cur().row);
                    if repeat & PAD_LEFT != 0 {
                        chr_grid_move_left(&self.grid, &mut col, row);
                    } else {
                        chr_grid_move_right(&self.grid, &mut col, row);
                    }
                    self.cur_mut().col = col;
                    self.clip_goto(FL_REEL, CLIP_CHIP, "fl_on_start");
                    self.sel.chara = self.sel.chip[0][col.clamp(0, 6) as usize];
                    self.request_face();
                    sounds.push(TitleSound::Effect(2, 0));
                } else if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    if self.rows >= 2 {
                        let up = repeat & PAD_UP != 0;
                        self.flash[FL_REEL]
                            .goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                        self.sel.mask = 1;
                        self.clip_goto(FL_REEL, CLIP_CHIP, "fl_off_start");
                        let (mut col, mut row) = (self.cur().col, self.cur().row);
                        if up {
                            chr_grid_move_up(&self.grid, &mut col, &mut row, self.rows);
                        } else {
                            chr_grid_move_down(&self.grid, &mut col, &mut row, self.rows);
                        }
                        (self.cur_mut().col, self.cur_mut().row) = (col, row);
                        self.clip_goto(FL_REEL, CLIP_CHIP, "fl_on_start");
                        self.set_chips();
                        self.sel.chara = self.sel.chip[0][col.clamp(0, 6) as usize];
                        self.request_face();
                        sounds.push(TitleSound::Effect(2, if up { 2 } else { 1 }));
                    }
                } else if pressed & PAD_CROSS != 0 {
                    let chara = self.sel.chara;
                    if !self.fits_dp(chara) {
                        sounds.push(TitleSound::Effect(1, 7));
                    } else if self.cell().form_count != 0 {
                        self.sel.flags |= SEL_FORM;
                        if self.cur().form >= self.cell().form_count {
                            self.cur_mut().form = 0;
                        }
                        self.flash[FL_REEL].goto_label("fl_form", true);
                        self.sel.mask = 1;
                        self.clip_goto(FL_REEL, CLIP_CHIP, "fl_off_start");
                        self.clip_goto(FL_REEL, CLIP_FORM_CHIP, "fl_on_start");
                        self.set_form_chips();
                        self.sel.chara = self.sel.chip[0][self.cur().form.clamp(0, 6) as usize];
                        self.request_face();
                        self.sel.step = STEP_FORM;
                        sounds.push(TitleSound::Effect(2, 0x29));
                    } else if self.is_chara_free(chara) {
                        self.open_plates(CLIP_CHIP, sounds);
                    } else {
                        sounds.push(TitleSound::Effect(1, 7));
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.sel.mask = 0;
                    if self.sel.member_count != 0 {
                        // Back to the team plates: the member being changed is restored.
                        self.flash[FL_REEL].goto_label("fl_out", true);
                        self.flash[FL_TEAM].goto_label("fl_in", true);
                        self.clip_goto(FL_REEL, CLIP_CHIP, "fl_off_start");
                        self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_on_start");
                        let backup = self.backup;
                        *self.cur_mut() = backup;
                        self.sel.chara = self.cur().picked;
                        self.request_face();
                        self.sum_cost(false);
                        self.sel.step = STEP_TEAM;
                    } else {
                        self.fade = ColorFade::start_out(true, FADE_FRAMES);
                        self.result = 0;
                    }
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            STEP_FORM => {
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.clip_goto(FL_REEL, CLIP_FORM_CHIP, "fl_off_start");
                    let mut form = self.cur().form;
                    if repeat & PAD_LEFT != 0 {
                        chr_grid_prev_form(&self.sel.chip[0], &mut form);
                    } else {
                        chr_grid_next_form(&self.sel.chip[0], &mut form);
                    }
                    self.cur_mut().form = form;
                    self.clip_goto(FL_REEL, CLIP_FORM_CHIP, "fl_on_start");
                    self.sel.chara = self.sel.chip[0][form.clamp(0, 6) as usize];
                    self.request_face();
                    sounds.push(TitleSound::Effect(2, 0));
                } else if pressed & PAD_CROSS != 0 {
                    let chara = self.sel.chara;
                    if self.fits_dp(chara) && self.is_chara_free(chara) {
                        self.open_plates(CLIP_FORM_CHIP, sounds);
                    } else {
                        sounds.push(TitleSound::Effect(1, 7));
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.sel.flags ^= SEL_FORM;
                    self.flash[FL_REEL].goto_label("fl_form", true);
                    self.sel.mask = 1;
                    self.clip_goto(FL_REEL, CLIP_FORM_CHIP, "fl_off_start");
                    self.clip_goto(FL_REEL, CLIP_CHIP, "fl_on_start");
                    self.set_chips();
                    self.sel.chara = self.sel.chip[0][self.cur().col.clamp(0, 6) as usize];
                    self.request_face();
                    self.sel.step = STEP_CHARA;
                    sounds.push(TitleSound::Effect(2, 0x29));
                }
            }
            STEP_PLATE => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    if self.plate_count >= 2 {
                        self.clip_goto(FL_PLATES, CLIP_PLATE, "fl_off_start");
                        let n = self.plate_count;
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
                        self.clip_goto(FL_PLATES, CLIP_PLATE, "fl_on_start");
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if pressed & PAD_RIGHT != 0 {
                    // The items of the set.
                    let c = self.cur();
                    if c.custom_plate > 0 {
                        self.clip_goto(FL_PLATES, CLIP_PLATE, "fl_ok");
                        let slot = (c.col + c.row * COLS as i32).max(0) as usize;
                        let set = (c.custom_plate - 1) as usize;
                        let items = save.custom_items.get(slot).map_or([0; 8], |s| s[set]);
                        let level = i32::from(save.custom_level.get(slot).copied().unwrap_or(0));
                        self.panel.set_chara(
                            self.sel.chara,
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
                    self.flash[FL_PLATES].goto_label("fl_color_in", true);
                    self.clip_goto(FL_PLATES, CLIP_PLATE, "fl_ok");
                    self.clip_goto(FL_PLATES, CLIP_COLOR, "fl_on_start");
                    self.sel.step = STEP_COLOR;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[FL_PLATES].goto_label("fl_custom_cansel", true);
                    self.clip_goto(FL_PLATES, CLIP_PLATE, "fl_off_start");
                    if self.sel.flags & SEL_FORM != 0 {
                        self.clip_goto(FL_REEL, CLIP_FORM_CHIP, "fl_on_start");
                        self.sel.step = STEP_FORM;
                    } else {
                        self.clip_goto(FL_REEL, CLIP_CHIP, "fl_on_start");
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
                    self.clip_goto(FL_PLATES, CLIP_PLATE, "fl_on_start");
                    self.sel.step = STEP_PLATE;
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
                    self.clip_goto(FL_PLATES, CLIP_COLOR, "fl_off_start");
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
                    self.clip_goto(FL_PLATES, CLIP_COLOR, "fl_on_start");
                    sounds.push(TitleSound::Effect(1, 0));
                } else if pressed & PAD_CROSS != 0 {
                    self.decide(save, sounds);
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[FL_PLATES].goto_label("fl_color_cansel", true);
                    self.clip_goto(FL_PLATES, CLIP_COLOR, "fl_off_start");
                    self.clip_goto(FL_PLATES, CLIP_PLATE, "fl_on_start");
                    self.sel.step = STEP_PLATE;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            STEP_TEAM => self.input_team(pressed, repeat, sounds),
            _ => {}
        }
    }

    /// The costume is confirmed: the member's character and item set are fixed, and the cursor goes to the next
    /// free slot (or the menu plate when the team is full).
    fn decide(&mut self, save: &SaveData, sounds: &mut Vec<TitleSound>) {
        self.sel.mask = 0;
        self.flash[FL_PLATES].goto_label("fl_color_ok", true);
        self.flash[FL_REEL].goto_label("fl_out", true);
        self.flash[FL_TEAM].goto_label("fl_in", true);
        self.clip_goto(FL_PLATES, CLIP_COLOR, "fl_ok");
        self.clip_goto(FL_PLATES, CLIP_PLATE, "fl_off_start");
        let c = self.cur();
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
        let chara = self.sel.chip[0][at.clamp(0, 6) as usize];
        let m = self.cur_mut();
        m.picked = chara;
        m.items = items;
        self.clip_goto(
            FL_REEL,
            if form { CLIP_FORM_CHIP } else { CLIP_CHIP },
            "fl_off_start",
        );
        self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_off_start");
        if self.sel.cur == self.sel.member_count {
            // A new member.
            self.sel.member_count = self.sel.cur + 1;
            self.sel.cur = self.sel.member_count;
            if self.sel.cur >= self.team_max {
                self.sel.cur = CUR_MENU;
            }
        }
        self.set_team_tex();
        self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_on_start");
        if self.sel.cur == CUR_MENU {
            self.sel.chara = self.sel.team[0].picked;
            self.request_face();
        }
        self.sel.flags &= !(SEL_FORM | SEL_REC);
        self.sel.step = STEP_TEAM;
        sounds.push(TitleSound::Effect(1, 1));
    }

    /// The team plates: left / right between the members and the menu plate, confirm changes a member or confirms
    /// the team, cancel removes a member.
    fn input_team(&mut self, pressed: u32, repeat: u32, sounds: &mut Vec<TitleSound>) {
        if repeat & PAD_LEFT != 0 {
            self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_off_start");
            self.sel.cur -= 1;
            if self.sel.cur < 0 {
                self.sel.cur = CUR_MENU;
            } else if self.sel.cur > self.sel.member_count {
                self.sel.cur = self.sel.member_count;
                if self.sel.cur >= self.team_max {
                    self.sel.cur = self.team_max - 1;
                }
            }
            self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_on_start");
            self.show_cur_face();
            sounds.push(TitleSound::Effect(1, 0));
        } else if repeat & PAD_RIGHT != 0 {
            self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_off_start");
            self.sel.cur += 1;
            if self.sel.cur > CUR_MENU {
                self.sel.cur = 0;
            } else if self.sel.cur > self.sel.member_count || self.sel.cur >= self.team_max {
                self.sel.cur = CUR_MENU;
            }
            self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_on_start");
            self.show_cur_face();
            sounds.push(TitleSound::Effect(1, 0));
        } else if pressed & PAD_CROSS != 0 {
            if self.sel.cur != CUR_MENU {
                // Choose (or change) the member under the cursor.
                self.sum_cost(true);
                self.flash[FL_TEAM].goto_label("fl_out", true);
                self.flash[FL_REEL].goto_label("fl_in", true);
                self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_ok");
                self.clip_goto(FL_REEL, CLIP_CHIP, "fl_on_start");
                self.set_chips();
                self.sel.chara = self.sel.chip[0][self.cur().col.clamp(0, 6) as usize];
                self.request_face();
                self.backup = self.cur();
                self.sel.step = STEP_CHARA;
                sounds.push(TitleSound::Effect(1, 1));
            } else if self.sel.member_count != 0 {
                // The team is complete: the guide's closing line, then the screen leaves.
                self.flash[FL_TEAM].goto_label("fl_out", true);
                self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_ok");
                self.end_step = END_SPEAK;
                sounds.push(TitleSound::Effect(1, 1));
            } else {
                sounds.push(TitleSound::Effect(1, 7));
            }
        } else if pressed & PAD_TRIANGLE != 0 {
            // Remove a member.
            if self.sel.member_count == 1 {
                self.flash[FL_TEAM].goto_label("fl_out", true);
                self.flash[FL_REEL].goto_label("fl_in", true);
                self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_ok");
                self.sel.cur = 0;
                self.clip_goto(FL_REEL, CLIP_CHIP, "fl_on_start");
                self.set_chips();
                self.sel.chara = self.sel.chip[0][self.cur().col.clamp(0, 6) as usize];
                self.request_face();
                self.sel.step = STEP_CHARA;
            } else if self.sel.cur == CUR_MENU {
                self.remove_member(self.sel.member_count - 1);
            } else if self.sel.cur == self.sel.member_count {
                self.remove_member(self.sel.cur - 1);
                self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_off_start");
                self.sel.cur -= 1;
                self.clip_goto(FL_TEAM, CLIP_TEAM, "fl_on_start");
                self.sel.chara = self.cur().picked;
                self.request_face();
            } else {
                self.remove_member(self.sel.cur);
                self.sel.chara = self.cur().picked;
                self.request_face();
            }
            self.sel.member_count -= 1;
            self.set_team_tex();
            sounds.push(TitleSound::Effect(1, 2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solo_select::{SoloSelectAssets, SoloSelectLayout};

    struct Pictures;

    impl PictureLoader for Pictures {
        fn portrait(&mut self, file_id: u32) -> Option<usize> {
            Some(file_id as usize)
        }

        fn picture(&mut self, _: u32) -> Option<usize> {
            None
        }
    }

    /// Runs the real team select from the ISO in `rom/`, when it is there: a team of two under the 10-point DP
    /// limit, the second member one cell to the right, then the menu plate confirms the team.
    #[test]
    fn real_ub_team_select_chooses_two() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe, pack_section};

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
        let pack = read(crate::ub::ARCHIVE_FILE_ID);
        let res =
            bpe::decode(pack_section(&pack, crate::solo_select::ARCHIVE_SECTION).unwrap()).unwrap();
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
        let mut team = [Pick::default(); MEMBER_MAX];
        let mut random = GameRandom::from_key(&[5, 1]);
        let mut screen = UbTeamSelectScreen::new(
            &data,
            2,
            1,
            &chr,
            &save,
            &mut team,
            &mut Pictures,
            &mut random,
        )
        .unwrap();
        assert!(screen.flags & DP != 0 && screen.dp_max == 10);
        let mut font = Font::new(Vec::new(), 20);
        let mut press = |screen: &mut UbTeamSelectScreen, bits: u32| {
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
        for n in 0..2 {
            if n == 1 {
                // The cursor went on to the second slot: choose it.
                assert_eq!((screen.sel.step, screen.sel.cur), (STEP_TEAM, 1));
                press(&mut screen, PAD_CROSS);
                press(&mut screen, PAD_RIGHT);
            }
            for _ in 0..4 {
                if screen.sel.step == STEP_COLOR {
                    break;
                }
                press(&mut screen, PAD_CROSS);
            }
            assert_eq!(screen.sel.step, STEP_COLOR, "member {n}");
            press(&mut screen, PAD_CROSS);
            assert_eq!(screen.sel.member_count, n + 1);
        }
        // Full: the cursor is on the menu plate.
        assert_eq!(screen.sel.cur, CUR_MENU);
        assert!(screen.sel.cost <= 10);
        press(&mut screen, PAD_CROSS);
        for _ in 0..6 {
            press(&mut screen, 0);
        }
        assert_eq!(screen.result(), Some(1));
        let (t, count) = screen.team();
        assert_eq!(count, 2);
        assert!(!chr.is_related(t[0].picked, t[1].picked));
        assert_eq!(t[2].picked, -1);
    }
}
