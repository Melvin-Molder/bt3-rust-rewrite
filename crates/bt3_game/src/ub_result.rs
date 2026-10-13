//! Ultimate Battle Z's result (`UbResult_Run(3)`, src/menu/ub_result.c, progress modes 27 and 30, `Ub_Main`): the
//! mission result's movie for the two disc modes. A won course keeps its best run and its cleared mark (all five
//! cleared give a reward item once); taking first place on the ladder gives another. The sheet, its pages and the
//! conversion into Z points run as in the other results.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::battle::BattleResult;
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::mission_result::TEX;
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::ub::VOICE_BASE;
use crate::ub_rank::UPWARD;
use crate::ub_score::{self, MONEY_MAX, PAGE, Price, Score};
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};

/// `UbResult_Run(3)`: section 3 of `gMenuArc3` (the mission result's).
pub const ARCHIVE_SECTION: usize = 3;
const FADE_FRAMES: i32 = 0x14;
const COURSE_NUM: usize = 5;
/// `gSaveData->ubFlags` 0x10: the reward for clearing all five courses was given.
const SAVE_ALL_COURSES: i32 = 0x10;

// UBRES_*.
const DONE: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const FADED_IN: u32 = 8;

// UBRES_STEP_*.
const STEP_INTRO: i32 = 0;
const STEP_COUNT: i32 = 1;
const STEP_SHEET: i32 = 2;
const STEP_PAY: i32 = 3;
const STEP_ITEM: i32 = 4;
const STEP_LEAVE: i32 = 5;

/// `IconWin_Init(MPACK_AT(res, 9), res 8)` with `IconWin_SetIcon(3)` (the ladder) or `(4)` (a course).
const ICON_TEXTURES: [ImageTexture; 7] = [
    Some(((9, 1), 1)),
    Some(((9, 1), 0)),
    tex(8, 0),
    Some(((9, 1), 2)),
    tex(8, 4),
    tex(8, 5),
    None,
];

/// What `UbResult_Init` reads from its unpacked section.
pub struct UbResultAssets {
    pub data: UbResultData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct UbResultData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 12: the screen's text; 11: the guide's mouth keys; 14: the item names.
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    pub item_text: Vec<u8>,
    /// Section 13.
    pub prices: Vec<Price>,
    pub layout: UbResultLayout,
}

#[derive(Clone, Debug, Default)]
pub struct UbResultLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    /// The icon window's textures; entries 4 and 5 are the ladder's and the course's icon.
    pub icon: Vec<Option<usize>>,
}

impl UbResultLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
        }
    }
}

impl UbResultAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&TEX[..], &ICON_TEXTURES])?;
        Ok(Self {
            data: UbResultData {
                fod: pack_section(res, 1)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 9)?, 2)?.to_vec(),
                text: pack_section(res, 12)?.to_vec(),
                lips: pack_section(res, 11)?.to_vec(),
                item_text: pack_section(res, 14)?.to_vec(),
                prices: ub_score::parse_prices(pack_section(res, 13)?),
                layout: UbResultLayout::default(),
            },
            background: decode_picture(res, 5)?,
            files,
        })
    }
}

/// `UbResult_MarkCourseCleared`: the course marked cleared; true the first time all five are.
fn mark_course_cleared(save: &mut SaveData, course: usize) -> bool {
    let mut rec = save.course(course);
    rec.cleared = 1;
    save.set_course(course, rec);
    let count = (0..COURSE_NUM)
        .filter(|&i| save.course(i).cleared != 0)
        .count();
    if count == COURSE_NUM && save.ub_flags() & SAVE_ALL_COURSES == 0 {
        save.set_ub_flags(save.ub_flags() | SAVE_ALL_COURSES);
        return true;
    }
    false
}

/// `UbResult` and the frame loop of `UbResult_Run`.
pub struct UbResultScreen {
    data: UbResultData,
    flash: Flash,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    timer: i32,
    step: i32,
    talk_step: i32,
    talk_prev: i32,
    skip: bool,
    voice_line: i32,
    blink: i32,
    talk: i32,
    page_count: usize,
    page: usize,
    lose: i32,
    new_record: bool,
    got_item: bool,
    page_done: bool,
    /// `gProgress->cursor`: 0 the ladder, 1 a course.
    kind: i32,
    ub_flags: i32,
    menu_cursor: i32,
    score: Score,
    counting: bool,
    count: usize,
    ready: bool,
    done: bool,
}

impl UbResultScreen {
    /// `UbResult_Init(3)` and the first `ColorFade_StartIn`: `kind` is `gProgress->cursor` (0 the ladder, 1 a
    /// course), `choice` `ubChoice` (the course), `ub_flags` `gProgress->ubFlags`, `menu_cursor` `ubCursor`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &UbResultData,
        kind: i32,
        choice: i32,
        ub_flags: i32,
        menu_cursor: i32,
        result: &BattleResult,
        save: &mut SaveData,
        libc: &mut LibcRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let blink = libc.rand() % 32;
        let mut icon_tex = data.layout.icon.clone();
        icon_tex.truncate(5);
        let (mut score, lose, page_count) = Score::fill(3, result);
        score.calc_points(&data.prices);
        let rank = score.calc_rank(3);
        let mut new_record = false;
        let mut got_item = false;
        if kind == 1 {
            // A course.
            icon_tex[4] = data.layout.icon.get(5).copied().flatten();
            let course = choice.clamp(0, COURSE_NUM as i32 - 1) as usize;
            if lose == 0 {
                // UbScore_SaveBestC.
                let mut rec = save.course(course);
                if rec.score < rank {
                    rec.score = rank;
                    rec.rank = score.rank as u8;
                    rec.time = [0, 1, 2].map(|k| score.clock[k] as u8);
                    save.set_course(course, rec);
                    new_record = true;
                }
                if mark_course_cleared(save, course) {
                    save.add_item(ub_score::reward_item(5) as usize);
                    got_item = true;
                }
            }
        } else if lose == 0 && save.ub_rank() == 1 && ub_flags & UPWARD != 0 {
            // The ladder: first place taken.
            got_item = true;
            save.add_item(ub_score::reward_item(4) as usize);
        }
        let mut icon = IconWin::new(&data.icon_fod, icon_tex)?;
        icon.open();
        Ok(Self {
            data: data.clone(),
            flash,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            timer: 15,
            step: STEP_INTRO,
            talk_step: 0,
            talk_prev: 0,
            skip: false,
            voice_line: -1,
            blink,
            talk: 0,
            page_count,
            page: 0,
            lose,
            new_record,
            got_item,
            page_done: false,
            kind,
            ub_flags,
            menu_cursor,
            score,
            counting: false,
            count: 0,
            ready: false,
            done: false,
        })
    }

    /// `UbResult_Run` has returned.
    pub fn is_done(&self) -> bool {
        self.done
    }

    fn leave(&mut self) {
        self.flags |= DONE | LEAVING;
        self.timer = 15;
        self.step = STEP_LEAVE;
    }

    fn lines(&self) -> usize {
        self.score.bonus.len() + self.score.line_count
    }

    /// One iteration of `UbResult_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
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
        if self.done {
            return frame;
        }
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        self.update(random, &mut frame.sounds);
        self.update_talk(pressed, voice_stat, &mut frame.sounds);
        self.draw(save, voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & FADED_IN == 0
            && self.flash.flags() & FLASH_PAD != 0
        {
            self.flags |= FADED_IN;
        }
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::FadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = true;
            return frame;
        }
        if self.flags & LEAVING != 0 {
            if self.timer == 0 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        } else if self.talk_step == 0 {
            self.input(pressed, repeat, save, &mut frame.sounds);
        }
        frame
    }

    /// `UbResult_Update`: the movie and what its triggers start.
    fn update(&mut self, random: &mut GameRandom, sounds: &mut Vec<TitleSound>) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        self.flash.advance();
        let trig = self.flash.trig();
        if trig & 1 != 0 && self.step == STEP_INTRO {
            self.ready = true;
        }
        if trig & 4 != 0 {
            if self.step == STEP_COUNT {
                self.counting = true;
            }
            self.score.set_page(self.page);
        }
        if trig & 8 != 0 {
            match self.step {
                STEP_INTRO => {
                    let clip = if self.lose == 0 {
                        self.talk_step = 1;
                        "mc_win_text"
                    } else {
                        self.talk_step = 6;
                        "mc_lose_text"
                    };
                    let r = self.flash.find(None, clip);
                    self.flash.clip_goto_label(r, "fl_start");
                }
                STEP_PAY => self.flash.goto_label("fl_zpoint_in", true),
                STEP_ITEM => {
                    self.flash.goto_label("fl_itemget_in", true);
                    sounds.push(TitleSound::Effect(2, 6));
                }
                _ => {}
            }
        }
        if self.page_done && self.timer == 0 {
            if self.page < self.page_count {
                self.flash.goto_label(
                    if self.page == 0 {
                        "fl_battlebonus_in"
                    } else {
                        "fl_battlebonus_out_in"
                    },
                    true,
                );
                self.page += 1;
            } else {
                self.flash.goto_label(
                    if self.page == 0 {
                        "fl_hyouka_rank_in"
                    } else {
                        "fl_rank_in"
                    },
                    true,
                );
                sounds.push(TitleSound::Effect(2, 0x13));
                self.talk_step = if self.got_item {
                    15
                } else if self.score.rank == 4 {
                    11
                } else if random.range(2) != 0 {
                    12
                } else {
                    13
                };
            }
            self.page_done = false;
        }
    }

    /// `UbResult_UpdateTalk`: the guide's lines.
    fn update_talk(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if !self.skip {
            if self.talk_step == 0 {
                return;
            }
            if self.voice_line != -1
                && voice_stat != VOICE_PLAYEND
                && self.talk_step == self.talk_prev
            {
                if pressed & PAD_CROSS != 0 {
                    sounds.push(TitleSound::Effect(1, 1));
                    self.skip = true;
                }
                return;
            }
        } else {
            self.skip = false;
        }
        match self.talk_step {
            1 => {
                self.voice_line = 0x38;
                self.talk_step += 1;
            }
            6 => {
                self.voice_line = 0x39;
                self.talk_step += 1;
            }
            2 | 7 => {
                self.talk_step = 0;
                self.voice_line = -1;
            }
            11 => self.voice_line = 0x3A,
            12 => self.voice_line = 0x3B,
            13 => self.voice_line = 0x3C,
            14 => self.voice_line = 0x3D,
            15 => self.voice_line = 0x51,
            _ => {}
        }
        if self.talk_step > 10 {
            self.talk_step = 0;
        }
        if self.voice_line != -1 {
            anim::play_with_subtitle(
                &self.data.lips,
                VOICE_BASE,
                self.voice_line,
                &mut self.lip,
                sounds,
            );
            self.talk_prev = self.talk_step;
        }
    }

    /// `UbResult_Draw`.
    fn draw(
        &mut self,
        save: &SaveData,
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
        let mut texts: Vec<TextDraw> = Vec::new();
        let plain = TextBox::preset(0).with_align(0);
        let f = &mut self.flash;
        let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_eye");
        anim::blink(f, r, &mut self.blink, 0, random);
        let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_mouth");
        anim::talk(f, r, &mut self.talk, 0, &mut self.lip, voice_stat, random);
        let headline = if self.lose != 0 {
            0x142
        } else if self.kind == 0 && self.ub_flags & UPWARD != 0 {
            0x140
        } else {
            0x141
        };
        let r = f.find(Some("mc_window_plate01"), "mc_window_text1_0");
        texts.extend(plain.attach_line(f, r, 0x400, &self.data.text, headline, font));
        for clip in ["mc_win_text02", "mc_win_text03"] {
            let r = f.find(None, clip);
            f.clip_set_uv(r, uv(0, 0x40, 0x200, 0x80));
        }
        let r = f.find(None, "mc_yajirusi_icon_up");
        f.clip_set_uv(r, uv(0, 0x20, 0x20, 0x40));
        f.clip_set_flags(r, PROP_VISIBLE, self.page != 0);
        let r = f.find(None, "mc_yajirusi_icon_down");
        f.clip_set_uv(r, uv(0x20, 0x20, 0x40, 0x40));
        f.clip_set_flags(
            r,
            PROP_VISIBLE,
            !self.score.bonus.is_empty() && self.page < self.page_count,
        );
        for (i, line) in self.score.line[..self.score.line_count].iter().enumerate() {
            let name = format!("mc_point_plate{:02}", i + 1);
            anim::num_draw_child(
                f,
                |n| (name.clone(), format!("mc_percent_suji{n:02}")),
                5,
                line.value,
                (0x20, 0x20),
                0,
            );
            anim::num_draw_child(
                f,
                |n| (name.clone(), format!("mc_pt_suji{n:02}")),
                6,
                line.points.wrapping_mul(100),
                (0x20, 0x20),
                0,
            );
        }
        for i in 0..PAGE {
            let name = format!("mc_bonus_plate{:02}", i + 1);
            let r = f.find(None, &name);
            if (self.page * PAGE + i) as i64 - PAGE as i64 >= self.score.bonus.len() as i64 {
                f.clip_set_flags(r, PROP_VISIBLE, false);
            } else {
                f.clip_set_flags(r, PROP_VISIBLE, true);
                let b = self
                    .score
                    .bonus
                    .get(self.score.shown[i])
                    .copied()
                    .unwrap_or_default();
                let r = f.find(Some(&name), "mc_bonus_karisize");
                texts.extend(plain.attach_line(
                    f,
                    r,
                    0x401 + i as u32,
                    &self.data.text,
                    b.value + 0x20,
                    font,
                ));
                anim::num_draw_child(
                    f,
                    |n| (name.clone(), format!("mc_point_suji{n:02}")),
                    5,
                    b.points.wrapping_mul(100),
                    (0x20, 0x20),
                    0,
                );
            }
        }
        for i in 1..4 {
            let name = format!("mc_point_plate{:02}", i + 1);
            let r = f.find(Some(&name), "mc_bonus_text");
            f.clip_set_uv(r, uv(0, i * 0x20, 0x100, i * 0x20 + 0x20));
            let r = f.find(Some(&name), "mc_pt_percent");
            f.clip_set_flags(r, PROP_VISIBLE, false);
        }
        let clock = self.score.clock;
        for (first, value) in [(4, clock[0]), (2, clock[1]), (0, clock[2])] {
            anim::num_draw(
                f,
                |n| format!("mc_time_suji{:02}", n + first),
                2,
                value,
                (0x20, 0x20),
                1,
            );
        }
        let r = f.find(None, "mc_bonus_title");
        f.clip_set_uv(r, uv(0, 0x80, 0x100, 0xA0));
        let r = f.find(None, "mc_win_text04");
        f.clip_set_uv(r, uv(0, 0x80, 0x200, 0xC0));
        anim::num_draw(
            f,
            |n| format!("mc_henkan_suji{n:02}"),
            6,
            self.score.convert,
            (0x20, 0x20),
            0,
        );
        let r = f.find(None, "mc_time_text03");
        f.clip_set_uv(r, uv(0, 0x40, 0x100, 0x60));
        anim::num_draw(
            f,
            |n| format!("mc_zpoint_suji{n:02}"),
            9,
            save.money(),
            (0x20, 0x20),
            0,
        );
        let r = f.find(None, "mc_time_text02");
        f.clip_set_uv(r, uv(0, 0x20, 0x100, 0x40));
        anim::num_draw(
            f,
            |n| format!("mc_score_suji{n:02}"),
            9,
            self.score.total.wrapping_mul(100),
            (0x20, 0x20),
            0,
        );
        let x = self.score.rank * 0x40;
        let r = f.find(None, "mc_rankmoji_anime");
        f.clip_set_uv(r, uv(x, 0, x + 0x40, 0x40));
        let r = f.find(None, "mc_newrecord_text");
        if self.kind != 0 && self.new_record {
            f.clip_set_uv(r, uv(0, 0x40, 0x100, 0x80));
            f.clip_set_flags(r, PROP_VISIBLE, true);
        } else {
            f.clip_set_flags(r, PROP_VISIBLE, false);
        }
        if self.got_item {
            let item_box = TextBox::preset(0)
                .with_align(0)
                .with_line_y([12, 0, 0, 0, 0]);
            let item = ub_score::reward_item(if self.kind == 0 { 4 } else { 5 });
            let r = f.find(Some("mc_item_plate00"), "mc_item_text_kari");
            texts.extend(item_box.attach_line(f, r, 0x404, &self.data.item_text, item, font));
        }
        let tex = &self.data.layout.flash;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (fq, overs) = f.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&fq, tex));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.icon.draw(quads);
    }

    /// `UbResult_Input`.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        save: &mut SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        let ok = pressed & PAD_CROSS != 0;
        if self.step == STEP_INTRO && self.ready {
            if self.lose == 2 {
                self.leave();
            } else {
                self.flash.goto_label("fl_hyouka_menu_in", true);
                self.step = STEP_COUNT;
            }
            self.ready = false;
        }
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        self.flags |= STARTED;
        match self.step {
            STEP_COUNT => {
                if self.counting {
                    let bonus = self.page != 0;
                    let row =
                        self.count
                            .saturating_sub(if bonus { self.score.line_count } else { 0 });
                    if self
                        .score
                        .count_line(bonus, row, 15, self.menu_cursor, sounds)
                    {
                        self.count += 1;
                        let per_page = if bonus { PAGE } else { self.score.line_count };
                        if self.count.is_multiple_of(per_page.max(1)) || self.count == self.lines()
                        {
                            self.counting = false;
                            self.page_done = true;
                        }
                    }
                }
                if ok {
                    for i in 0..self.lines() {
                        let (bonus, row) = if i < self.score.line_count {
                            (false, i)
                        } else {
                            (true, i - self.score.line_count)
                        };
                        while !self
                            .score
                            .count_line(bonus, row, 9999, self.menu_cursor, sounds)
                        {
                        }
                    }
                    self.page_done = true;
                    self.page = self.page_count;
                    self.count = self.lines();
                    self.score.set_page(self.page);
                }
                if self.page_done {
                    if self.page < self.page_count {
                        self.flash.goto_label(
                            if self.page == 0 {
                                "fl_battlebonus_in"
                            } else {
                                "fl_battlebonus_out_in"
                            },
                            true,
                        );
                        self.page += 1;
                    } else {
                        self.flash.goto_label(
                            if self.page == 0 {
                                "fl_hyouka_rank_in"
                            } else {
                                "fl_rank_in"
                            },
                            true,
                        );
                    }
                    self.page_done = false;
                }
                if self.count == self.lines() {
                    self.step = STEP_SHEET;
                }
            }
            STEP_SHEET => {
                if repeat & PAD_UP != 0 {
                    if self.page > 0 {
                        self.page -= 1;
                        self.flash.goto_label(
                            if self.page != 0 {
                                "fl_battle_cansel_in"
                            } else {
                                "fl_battlebonus_cancel"
                            },
                            true,
                        );
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if repeat & PAD_DOWN != 0 {
                    if self.page < self.page_count {
                        self.page += 1;
                        self.flash.goto_label("fl_battle_cansel_in", true);
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if ok {
                    self.step = STEP_PAY;
                    self.count = 0;
                    self.timer = 0x3C;
                    self.flash.goto_label(
                        if self.page == 0 {
                            "fl_hyoukakarazpoint_trig"
                        } else {
                            "fl_zpoint_trig"
                        },
                        true,
                    );
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            STEP_PAY => {
                let mut money = save.money();
                match self.count {
                    0 => {
                        if self.timer == 0 {
                            self.count += 1;
                            ub_score::plate_goto(&mut self.flash, 0, true);
                            self.score.saved = self.score.total;
                            self.talk_step = 14;
                        }
                    }
                    1 => {
                        if self.score.convert_step(0x40, sounds) {
                            self.count += 1;
                            self.timer = 15;
                            ub_score::plate_goto(&mut self.flash, 0, false);
                            self.score.convert = self.score.saved;
                        }
                    }
                    2 => {
                        if self.timer == 0 {
                            self.count += 1;
                            ub_score::plate_goto(&mut self.flash, 1, true);
                        }
                    }
                    3 => {
                        if ub_score::transfer(
                            &mut self.score.convert,
                            &mut money,
                            0x40,
                            1.0,
                            sounds,
                        ) {
                            self.count += 1;
                            ub_score::plate_goto(&mut self.flash, 1, false);
                        }
                        money = money.min(MONEY_MAX);
                    }
                    4 => {
                        if ok {
                            self.flash.goto_label("fl_zpoint_out", true);
                            if self.got_item {
                                self.step = STEP_ITEM;
                            } else {
                                self.leave();
                            }
                            sounds.push(TitleSound::Effect(1, 1));
                        }
                    }
                    _ => {}
                }
                if (1..=3).contains(&self.count) && ok {
                    if self.count < 2 {
                        while !self.score.convert_step(MONEY_MAX, sounds) {}
                        self.score.convert = self.score.saved;
                    }
                    while !ub_score::transfer(
                        &mut self.score.convert,
                        &mut money,
                        MONEY_MAX,
                        1.0,
                        sounds,
                    ) {}
                    money = money.min(MONEY_MAX);
                    ub_score::plate_goto(&mut self.flash, 0, false);
                    ub_score::plate_goto(&mut self.flash, 1, false);
                    self.count = 4;
                }
                save.set_money(money);
            }
            STEP_ITEM => {
                if ok {
                    self.flash.goto_label("fl_itemget_out", true);
                    self.leave();
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the real result from the ISO in `rom/`, when it is there: a won course's best run and cleared mark
    /// kept, and the pay-out into the Z points.
    #[test]
    fn real_ub_result_records_a_course() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(crate::ub::ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = UbResultAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = UbResultLayout::new(0, |k| firsts[&k]);
        let result = BattleResult {
            winner: 1,
            health: [30.0, 0.0],
            max_combo_hits: [8, 3],
            max_combo_damage: [4000, 100],
            event_summary: 0b10010,
            clock: [0, 4, 12, 0],
            ..BattleResult::default()
        };
        let mut save = SaveData::default();
        let money = save.money();
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[4, 2]);
        let mut screen =
            UbResultScreen::new(&data, 1, 3, 0, 1, &result, &mut save, &mut libc).unwrap();
        assert!(screen.new_record && !screen.got_item);
        let rec = save.course(3);
        assert_eq!((rec.cleared, rec.time), (1, [0, 4, 12]));
        let mut font = Font::new(Vec::new(), 20);
        for _ in 0..80 {
            if screen.is_done() {
                break;
            }
            for i in 0..30 {
                let b = if i == 0 { PAD_CROSS } else { 0 };
                screen.run_frame(b, b, VOICE_PLAYEND, &mut save, &mut random, &mut font);
            }
        }
        assert!(screen.is_done(), "step {}", screen.step);
        assert_eq!(save.money(), money + rec.score);
    }
}
