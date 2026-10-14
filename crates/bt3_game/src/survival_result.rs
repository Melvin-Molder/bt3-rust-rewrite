//! Survival's result (`SurvResult_Run(3)`, src/menu/survival_result.c, progress mode 19, `Ub_Main`): the mission
//! result's sibling for a survival run. The score sheet has a fourth line, the opponents beaten; the course keeps
//! its best total with the rank, time and opponents beaten; ten opponents beaten give the reward item once.

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
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::ub::VOICE_BASE;
use crate::ub_score::{self, MONEY_MAX, PAGE, Price, Score};
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};

/// `SurvResult_Run(3)`: section 3 of `gMenuArc3` (the mission result's).
pub const ARCHIVE_SECTION: usize = 3;
const FADE_FRAMES: i32 = 0x14;
/// `S_UBFLAG_SURV_ITEM`: the reward for ten opponents beaten was given.
const SAVE_SURV_ITEM: i32 = 8;

// SURVRESULT_*.
const DONE: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;

// SURVRESULT_ST_*.
const ST_INTRO: i32 = 0;
const ST_COUNT: i32 = 1;
const ST_PAGES: i32 = 2;
const ST_MONEY: i32 = 3;
const ST_ITEM: i32 = 4;
const ST_LEAVE: i32 = 5;

/// `gSurvResult->tex[n]` of `SurvResult_Init`.
const TEX: [ImageTexture; 43] = [
    tex(4, 0),
    tex(4, 3),
    tex(4, 28),
    tex(4, 27),
    tex(4, 31),
    tex(4, 32),
    tex(4, 36),
    tex(4, 38),
    tex(4, 37),
    tex(4, 33),
    None,
    None,
    tex(4, 35),
    tex(4, 34),
    tex(4, 25),
    tex(4, 19),
    tex(4, 8),
    tex(4, 29),
    tex(4, 22),
    tex(4, 7),
    None,
    tex(4, 18),
    tex(4, 26),
    tex(4, 39),
    tex(4, 20),
    tex(4, 21),
    tex(4, 30),
    tex(4, 23),
    tex(4, 6),
    tex(4, 43),
    tex(4, 16),
    tex(4, 15),
    tex(4, 17),
    tex(4, 13),
    tex(4, 14),
    None,
    tex(4, 12),
    tex(7, 0),
    tex(7, 1),
    tex(7, 3),
    tex(4, 45),
    tex(4, 46),
    tex(4, 47),
];
/// `IconWin_Init(MPACK_AT(res, 9), res 8)` with `IconWin_SetIcon(2)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((9, 1), 1)),
    Some(((9, 1), 0)),
    tex(8, 0),
    Some(((9, 1), 2)),
    tex(8, 3),
];

/// What `SurvResult_Init` reads from its unpacked section.
pub struct SurvivalResultAssets {
    pub data: SurvivalResultData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct SurvivalResultData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 12: the screen's text; 11: the guide's mouth keys; 14: the item names.
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    pub item_text: Vec<u8>,
    /// Section 13.
    pub prices: Vec<Price>,
    pub layout: SurvivalResultLayout,
}

#[derive(Clone, Debug, Default)]
pub struct SurvivalResultLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
}

impl SurvivalResultLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
        }
    }
}

impl SurvivalResultAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&TEX[..], &ICON_TEXTURES])?;
        Ok(Self {
            data: SurvivalResultData {
                fod: pack_section(res, 2)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 9)?, 2)?.to_vec(),
                text: pack_section(res, 12)?.to_vec(),
                lips: pack_section(res, 11)?.to_vec(),
                item_text: pack_section(res, 14)?.to_vec(),
                prices: ub_score::parse_prices(pack_section(res, 13)?),
                layout: SurvivalResultLayout::default(),
            },
            background: decode_picture(res, 5)?,
            files,
        })
    }
}

/// `SurvResult` and the frame loop of `SurvResult_Run`.
pub struct SurvivalResultScreen {
    data: SurvivalResultData,
    flash: Flash,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    timer: i32,
    state: i32,
    talk_req: i32,
    talk_last: i32,
    voice_skip: bool,
    voice_line: i32,
    blink: i32,
    mouth: i32,
    pages: usize,
    page: usize,
    outcome: i32,
    new_record: bool,
    got_item: bool,
    page_done: bool,
    menu_cursor: i32,
    score: Score,
    counting: bool,
    step: usize,
    started: bool,
    done: bool,
}

impl SurvivalResultScreen {
    /// `SurvResult_Init(3)` and the first `ColorFade_StartIn`: the sheet of course `course` (`gProgress->cursor`)
    /// from the battle's result, its record and the reward item written into the save. `menu_cursor` is
    /// `gProgress->ubCursor`.
    pub fn new(
        data: &SurvivalResultData,
        course: usize,
        menu_cursor: i32,
        result: &BattleResult,
        save: &mut SaveData,
        libc: &mut LibcRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let blink = libc.rand() % 32;
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let (mut score, outcome, pages) = Score::fill(2, result);
        score.calc_points(&data.prices);
        let total = score.calc_rank(2);
        // UbScore_SaveBestB.
        let mut rec = save.survival(course);
        let new_record = rec.score < total;
        if new_record {
            rec.score = total;
            rec.rank = score.rank as u8;
            rec.time = [0, 1, 2].map(|k| score.clock[k] as u8);
            rec.defeated = score.line[3].value;
            save.set_survival(course, rec);
        }
        let mut got_item = false;
        if score.line[3].value >= 10 && save.ub_flags() & SAVE_SURV_ITEM == 0 {
            save.set_ub_flags(save.ub_flags() | SAVE_SURV_ITEM);
            got_item = true;
            save.add_item(ub_score::reward_item(3) as usize);
        }
        Ok(Self {
            data: data.clone(),
            flash,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            timer: 15,
            state: ST_INTRO,
            talk_req: 0,
            talk_last: 0,
            voice_skip: false,
            voice_line: -1,
            blink,
            mouth: 0,
            pages,
            page: 0,
            outcome,
            new_record,
            got_item,
            page_done: false,
            menu_cursor,
            score,
            counting: false,
            step: 0,
            started: false,
            done: false,
        })
    }

    /// `SurvResult_Run` has returned.
    pub fn is_done(&self) -> bool {
        self.done
    }

    fn leave(&mut self) {
        self.flags |= DONE | LEAVING;
        self.timer = 15;
        self.state = ST_LEAVE;
    }

    fn lines(&self) -> usize {
        self.score.bonus.len() + self.score.line_count
    }

    /// One iteration of `SurvResult_Run`'s loop.
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
        self.update_voice(pressed, voice_stat, &mut frame.sounds);
        self.draw(save, voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & GREETED == 0
            && self.flash.flags() & FLASH_PAD != 0
        {
            self.flags |= GREETED;
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
        } else if self.talk_req == 0 {
            self.input(pressed, repeat, save, random, &mut frame.sounds);
        }
        frame
    }

    /// `SurvResult_Update`: the timer, the movie and its triggers.
    fn update(&mut self, random: &mut GameRandom, sounds: &mut Vec<TitleSound>) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        self.flash.advance();
        let trig = self.flash.trig();
        if trig & 1 != 0 && self.state == ST_INTRO {
            self.started = true;
        }
        if trig & 4 != 0 {
            if self.state == ST_COUNT {
                self.counting = true;
            }
            self.score.set_page(self.page);
        }
        if trig & 8 != 0 {
            match self.state {
                ST_INTRO => {
                    let clip = if self.outcome == 0 {
                        self.talk_req = 1;
                        "mc_win_text"
                    } else {
                        self.talk_req = 6;
                        "mc_lose_text"
                    };
                    let r = self.flash.find(None, clip);
                    self.flash.clip_goto_label(r, "fl_start");
                }
                ST_MONEY => self.flash.goto_label("fl_zpoint_in", true),
                ST_ITEM => self.flash.goto_label("fl_itemget_in", true),
                _ => {}
            }
        }
        if self.page_done && self.timer == 0 {
            if self.page < self.pages {
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
                self.talk_req = if self.score.rank == 4 {
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

    /// `SurvResult_UpdateVoice`: the guide's script (1 won, 6 lost, each ending with a step of its own; 11..15
    /// one-line comments).
    fn update_voice(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if !self.voice_skip {
            if self.talk_req == 0 {
                return;
            }
            if self.voice_line != -1
                && voice_stat != VOICE_PLAYEND
                && self.talk_req == self.talk_last
            {
                if pressed & PAD_CROSS != 0 {
                    sounds.push(TitleSound::Effect(1, 1));
                    self.voice_skip = true;
                }
                return;
            }
        } else {
            self.voice_skip = false;
        }
        match self.talk_req {
            1 => {
                self.voice_line = 0x38;
                self.talk_req += 1;
            }
            6 => {
                self.voice_line = 0x39;
                self.talk_req += 1;
            }
            2 | 7 => {
                self.talk_req = 0;
                self.voice_line = -1;
            }
            11 => self.voice_line = 0x3A,
            12 => self.voice_line = 0x3B,
            13 => self.voice_line = 0x3C,
            14 => self.voice_line = 0x3D,
            15 => self.voice_line = 0x3F,
            _ => {}
        }
        if self.talk_req >= 11 {
            self.talk_req = 0;
        }
        if self.voice_line == -1 {
            return;
        }
        anim::play_with_subtitle(
            &self.data.lips,
            VOICE_BASE,
            self.voice_line,
            &mut self.lip,
            sounds,
        );
        self.talk_last = self.talk_req;
    }

    /// `SurvResult_Draw`.
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
        anim::talk(f, r, &mut self.mouth, 0, &mut self.lip, voice_stat, random);
        let r = f.find(Some("mc_window_plate01"), "mc_window_text1_0");
        let headline = if self.outcome == 0 { 0x141 } else { 0x142 };
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
            !self.score.bonus.is_empty() && self.page < self.pages,
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
        if self.new_record {
            f.clip_set_uv(r, uv(0, 0x40, 0x100, 0x80));
            f.clip_set_flags(r, PROP_VISIBLE, true);
        } else {
            f.clip_set_flags(r, PROP_VISIBLE, false);
        }
        if self.got_item {
            let item_box = TextBox::preset(0)
                .with_align(0)
                .with_line_y([0xC, 0, 0, 0, 0]);
            let r = f.find(Some("mc_item_plate00"), "mc_item_text_kari");
            texts.extend(item_box.attach_line(
                f,
                r,
                0x404,
                &self.data.item_text,
                ub_score::reward_item(3),
                font,
            ));
        }
        let tex = &self.data.layout.flash;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (fq, overs) = f.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&fq, tex));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.icon.draw(quads);
    }

    /// `SurvResult_Input`.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        save: &mut SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        let confirm = pressed & PAD_CROSS != 0;
        if self.state == ST_INTRO && self.started {
            if self.outcome == 2 {
                self.leave();
            } else {
                self.flash.goto_label("fl_hyouka_menu_in", true);
                self.state = ST_COUNT;
            }
            self.started = false;
        }
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        self.flags |= STARTED;
        match self.state {
            ST_COUNT => {
                if self.counting {
                    let bonus = self.page != 0;
                    let idx =
                        self.step
                            .saturating_sub(if bonus { self.score.line_count } else { 0 });
                    if self
                        .score
                        .count_line(bonus, idx, 15, self.menu_cursor, sounds)
                    {
                        self.step += 1;
                        if (bonus && (idx + 1).is_multiple_of(PAGE))
                            || (!bonus && self.step == self.score.line_count)
                            || self.step == self.lines()
                        {
                            self.counting = false;
                            self.page_done = true;
                        }
                    }
                }
                if confirm {
                    for i in 0..self.lines() {
                        let (bonus, idx) = if i < self.score.line_count {
                            (false, i)
                        } else {
                            (true, i - self.score.line_count)
                        };
                        while !self
                            .score
                            .count_line(bonus, idx, 9999, self.menu_cursor, sounds)
                        {
                        }
                    }
                    self.page_done = true;
                    self.page = self.pages;
                    self.step = self.lines();
                    self.score.set_page(self.page);
                }
                if self.page_done {
                    if self.page < self.pages {
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
                        self.talk_req = if self.score.line[3].value >= 50 {
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
                if self.step == self.lines() {
                    self.state = ST_PAGES;
                }
            }
            ST_PAGES => {
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
                    if self.page < self.pages {
                        self.page += 1;
                        self.flash.goto_label("fl_battle_cansel_in", true);
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if confirm {
                    self.state = ST_MONEY;
                    self.step = 0;
                    self.timer = 60;
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
            ST_MONEY => {
                let mut money = save.money();
                match self.step {
                    0 => {
                        if self.timer == 0 {
                            self.step += 1;
                            ub_score::plate_goto(&mut self.flash, 0, true);
                            self.score.saved = self.score.total;
                            self.talk_req = 14;
                        }
                    }
                    1 => {
                        if self.score.convert_step(0x40, sounds) {
                            self.step += 1;
                            self.timer = 15;
                            ub_score::plate_goto(&mut self.flash, 0, false);
                            self.score.convert = self.score.saved;
                        }
                    }
                    2 => {
                        if self.timer == 0 {
                            self.step += 1;
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
                            self.step += 1;
                            ub_score::plate_goto(&mut self.flash, 1, false);
                        }
                        money = money.min(MONEY_MAX);
                    }
                    4 => {
                        if confirm {
                            self.flash.goto_label("fl_zpoint_out", true);
                            if self.got_item {
                                self.state = ST_ITEM;
                                sounds.push(TitleSound::Effect(2, 6));
                            } else {
                                self.leave();
                                sounds.push(TitleSound::Effect(1, 1));
                            }
                        }
                    }
                    _ => {}
                }
                if (1..=3).contains(&self.step) && confirm {
                    if self.step < 2 {
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
                    self.step = 4;
                }
                save.set_money(money);
            }
            ST_ITEM => {
                if confirm {
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

    /// Runs the real survival result from the ISO in `rom/`, when it is there: twelve opponents beaten, the
    /// course's record, the reward item, and the pay-out into the Z points.
    #[test]
    fn real_survival_result_records_and_rewards() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(crate::ub::ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = SurvivalResultAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = SurvivalResultLayout::new(0, |k| firsts[&k]);
        let result = BattleResult {
            winner: 2,
            health: [0.0, 40.0],
            max_combo_hits: [20, 3],
            max_combo_damage: [12000, 100],
            frames: 12,
            event_summary: 0b100,
            clock: [0, 9, 5, 0],
            ..BattleResult::default()
        };
        let mut save = SaveData::default();
        let money = save.money();
        let item = ub_score::reward_item(3) as usize;
        assert_eq!(save.item(item) & 1, 0);
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1, 9]);
        let mut screen =
            SurvivalResultScreen::new(&data, 2, 1, &result, &mut save, &mut libc).unwrap();
        assert_eq!(screen.outcome, 1);
        assert!(screen.new_record && screen.got_item);
        assert_eq!(save.item(item) & 1, 1);
        let rec = save.survival(2);
        assert_eq!((rec.defeated, rec.time), (12, [0, 9, 5]));
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
        assert!(screen.is_done(), "state {}", screen.state);
        assert_eq!(save.money(), money + rec.score);
    }
}
