//! Sim Dragon's result (`SimResult_Run(3)`, src/menu/sim_result.c, progress mode 23, `Ub_Main`): Android 18 reads
//! the round's fight. The sheet counts on top of the run's points; a won fight asks whether to go on (yes: the
//! points stay with the run and the next round starts), a lost one is game over, and the seventh round's fight
//! ends the run (its reward item once per save). A run that ends goes into the ranking and its points into the Z
//! points.

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
use crate::sim_top::SimRun;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::ub::VOICE_BASE;
use crate::ub_score::{self, MONEY_MAX, PAGE, Price, Score};
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};

/// `SimResult_Run(3)`: section 3 of `gMenuArc3` (the results' section; its third movie).
pub const ARCHIVE_SECTION: usize = 3;
const FADE_FRAMES: i32 = 0x14;
/// `SIM_LAST_TURN`: the fight of the seventh round.
const LAST_TURN: i32 = 0x45;
/// `SIM_BGM_FIRST` and `SimResult_Init`'s music of each round.
const BGM_FIRST: u32 = 0x10B16;
const ROUND_BGM: [u32; 7] = [4, 1, 2, 0, 6, 4, 8];
/// Android 16: the guide's own lines for him.
const ANDROID_16: i32 = 0x66;

// SIMRESULT_*.
const DONE: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;

// SIMRESULT_ST_*.
const ST_INTRO: i32 = 0;
const ST_COUNT: i32 = 1;
const ST_PAGES: i32 = 2;
const ST_OVER: i32 = 3;
const ST_CONTINUE: i32 = 4;
const ST_TRAIN: i32 = 5;
const ST_CLEAR: i32 = 6;
const ST_MONEY: i32 = 7;
const ST_ITEM: i32 = 8;
const ST_END: i32 = 9;

/// `gSimResult->tex[n]` of `SimResult_Init`.
const TEX: [ImageTexture; 49] = [
    tex(4, 1),
    tex(4, 5),
    tex(4, 4),
    tex(4, 2),
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
    tex(4, 10),
    tex(4, 9),
    tex(4, 11),
    None,
    None,
    tex(4, 40),
    tex(4, 41),
    tex(4, 42),
    tex(7, 5),
    tex(7, 6),
    tex(7, 8),
];
/// `IconWin_Init(MPACK_AT(res, 9), res 8)` with `IconWin_SetIcon(0)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((9, 1), 1)),
    Some(((9, 1), 0)),
    tex(8, 0),
    Some(((9, 1), 2)),
    tex(8, 1),
];

/// What `SimResult_Init` reads from its unpacked section.
pub struct SimResultAssets {
    pub data: SimResultData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct SimResultData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 12: the screen's text; 11: the guide's mouth keys; 14: the item names.
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    pub item_text: Vec<u8>,
    /// Section 13.
    pub prices: Vec<Price>,
    pub layout: SimResultLayout,
}

#[derive(Clone, Debug, Default)]
pub struct SimResultLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
}

impl SimResultLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
        }
    }
}

impl SimResultAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&TEX[..], &ICON_TEXTURES])?;
        Ok(Self {
            data: SimResultData {
                fod: pack_section(res, 3)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 9)?, 2)?.to_vec(),
                text: pack_section(res, 12)?.to_vec(),
                lips: pack_section(res, 11)?.to_vec(),
                item_text: pack_section(res, 14)?.to_vec(),
                prices: ub_score::parse_prices(pack_section(res, 13)?),
                layout: SimResultLayout::default(),
            },
            background: decode_picture(res, 6)?,
            files,
        })
    }
}

/// `SimResult` and the frame loop of `SimResult_Run`.
pub struct SimResultScreen {
    data: SimResultData,
    flash: Flash,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    /// `pick[SIMRESULT_ST_CONTINUE]`: 1 no.
    pick: i32,
    timer: i32,
    state: i32,
    talk: i32,
    talk_last: i32,
    skip: bool,
    voice_line: i32,
    blink: i32,
    mouth: i32,
    page_max: usize,
    page: usize,
    outcome: i32,
    cleared: bool,
    got_item: bool,
    page_shown: bool,
    score: Score,
    counting: bool,
    step: usize,
    /// `SIM_PROG->chara`.
    chara: i32,
    menu_cursor: i32,
    bgm: u32,
    /// `SimResult_Run`'s result: 0 the run goes on.
    result: i32,
    run: SimRun,
    done: bool,
}

impl SimResultScreen {
    /// `SimResult_Init(3)` and the first `ColorFade_StartIn`: `run` is the run's state, `chara` the player's
    /// fighter, `menu_cursor` `ubCursor`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &SimResultData,
        run: SimRun,
        chara: i32,
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
        let (mut score, outcome, page_max) = Score::fill(0, result);
        score.calc_points(&data.prices);
        score.calc_rank(0);
        let mut got_item = false;
        let cleared = run.turn >= LAST_TURN;
        if cleared && !save.sim_cleared() {
            // The reward goes in at once, whatever the fight's outcome.
            save.set_sim_cleared();
            got_item = true;
            save.add_item(ub_score::reward_item(0) as usize);
        }
        score.total = run.stat[3];
        let round = (run.turn / 10).clamp(0, ROUND_BGM.len() as i32 - 1) as usize;
        Ok(Self {
            data: data.clone(),
            flash,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            pick: 0,
            timer: 15,
            state: ST_INTRO,
            talk: 0,
            talk_last: 0,
            skip: false,
            voice_line: -1,
            blink,
            mouth: 0,
            page_max,
            page: 0,
            outcome,
            cleared,
            got_item,
            page_shown: false,
            score,
            counting: false,
            step: 0,
            chara,
            menu_cursor,
            bgm: BGM_FIRST + ROUND_BGM[round],
            result: 1,
            run,
            done: false,
        })
    }

    /// The round's music (`Bgm_Play` of `SimResult_Init`).
    pub fn bgm(&self) -> u32 {
        self.bgm
    }

    /// `SimResult_Run`'s result once it has returned: 0 the run goes on, 1 it is over.
    pub fn result(&self) -> Option<i32> {
        self.done.then_some(self.result)
    }

    /// The run's state as the screen leaves it (the points of a run that goes on).
    pub fn run(&self) -> SimRun {
        self.run
    }

    fn leave(&mut self) {
        self.flags |= DONE | LEAVING;
        self.timer = 15;
        self.state = ST_END;
    }

    fn lines(&self) -> usize {
        self.score.bonus.len() + self.score.line_count
    }

    /// `SimResult_YesNoGoto`.
    fn yes_no_goto(&mut self, label: &str) {
        let r = self
            .flash
            .find(None, &format!("mc_yes_no_text{:02}", self.pick + 1));
        self.flash.clip_goto_label(r, label);
    }

    /// One iteration of `SimResult_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        save: &mut SaveData,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
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
        self.update(&mut frame.sounds);
        self.update_talk(pressed, voice_stat, &mut frame.sounds);
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
        } else if self.talk == 0 {
            self.input(pressed, repeat, save, random, libc, &mut frame.sounds);
        }
        frame
    }

    /// `SimResult_Update`: the movie and what its triggers start.
    fn update(&mut self, sounds: &mut Vec<TitleSound>) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        self.flash.advance();
        let trig = self.flash.trig();
        if trig & 1 != 0 {
            if self.outcome == 2 {
                self.leave();
            } else if self.state == ST_INTRO {
                self.flash.goto_label("fl_hyouka_menu_in", true);
                self.state = ST_COUNT;
            }
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
                        "mc_win_text"
                    } else {
                        "mc_lose_text"
                    };
                    let r = self.flash.find(None, clip);
                    self.flash.clip_goto_label(r, "fl_start");
                }
                ST_CONTINUE => {
                    self.flash.goto_label("fl_zokkou_in", true);
                    self.yes_no_goto("fl_on_start");
                }
                ST_OVER => {
                    self.flash.goto_label("fl_gameover_in", true);
                    self.score.total = 0;
                }
                ST_TRAIN => self.flash.goto_label("fl_syugyo_in", true),
                ST_MONEY => self.flash.goto_label("fl_zpoint_in", true),
                ST_CLEAR => self.flash.goto_label("fl_gameclear_in", true),
                ST_ITEM => {
                    self.flash.goto_label("fl_itemget_in", true);
                    self.talk = if self.chara == ANDROID_16 { 0x15 } else { 0x14 };
                }
                _ => {}
            }
        }
        if self.page_shown && self.timer == 0 {
            if self.page < self.page_max {
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
            }
            self.page_shown = false;
        }
    }

    /// `SimResult_UpdateTalk`: line `talk` starts when the previous one is over (or cut short); 12 and 13 fade the
    /// screen out after them.
    fn update_talk(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if !self.skip {
            if self.talk == 0 {
                return;
            }
            if self.voice_line != -1 && voice_stat != VOICE_PLAYEND && self.talk == self.talk_last {
                if pressed & PAD_CROSS != 0 {
                    sounds.push(TitleSound::Effect(1, 1));
                    self.skip = true;
                }
                return;
            }
        } else {
            self.skip = false;
        }
        match self.talk {
            1..=11 => self.voice_line = 0x1E + self.talk,
            12 | 13 => {
                self.voice_line = 0x1E + self.talk;
                self.talk = 14;
            }
            14 => {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.talk = 0;
                return;
            }
            15..=21 => self.voice_line = 0x1D + self.talk,
            _ => {}
        }
        if self.talk != 14 {
            self.talk = 0;
        }
        anim::play_with_subtitle(
            &self.data.lips,
            VOICE_BASE,
            self.voice_line,
            &mut self.lip,
            sounds,
        );
        self.talk_last = self.talk;
    }

    /// `SimResult_Draw`.
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
        let r = f.find(Some("mc_guide_18go"), "mc_guide_18go_eye");
        anim::blink(f, r, &mut self.blink, 0, random);
        let r = f.find(Some("mc_guide_18go"), "mc_guide_18go_mouth");
        anim::talk(f, r, &mut self.mouth, 0, &mut self.lip, voice_stat, random);
        let headline = if self.outcome == 0 { 0x141 } else { 0x142 };
        let r = f.find(Some("mc_window_plate01"), "mc_window_text1_0");
        texts.extend(plain.attach_line(f, r, 0x403, &self.data.text, headline, font));
        let r = f.find(None, "mc_day_text_kari");
        texts.extend(plain.attach_line(f, r, 0x404, &self.data.text, headline, font));
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
            !self.score.bonus.is_empty() && self.page < self.page_max,
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
                    0x400 + i as u32,
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
        for clip in ["mc_yes_no_text_on", "mc_yes_no_text_off"] {
            let r = f.find(Some("mc_yes_no_text02"), clip);
            f.clip_set_uv(r, uv(0, 0x20, 0x100, 0x40));
        }
        let r = f.find(None, "mc_sim_text_kari");
        texts.extend(plain.attach_line(f, r, 0x405, &self.data.text, 0xF, font));
        let r = f.find(None, "mc_clear_text01");
        f.clip_set_uv(r, uv(0, 0x40, 0x200, 0x80));
        let r = f.find(None, "mc_clear_text02");
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
        if self.got_item {
            let item_box = TextBox::preset(0)
                .with_align(0)
                .with_line_y([12, 0, 0, 0, 0]);
            let r = f.find(Some("mc_item_plate00"), "mc_item_text_kari");
            texts.extend(item_box.attach_line(
                f,
                r,
                0x406,
                &self.data.item_text,
                ub_score::reward_item(0),
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

    /// `SimResult_Input`.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        save: &mut SaveData,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        let up = repeat & PAD_UP != 0;
        let down = repeat & PAD_DOWN != 0;
        let ok = pressed & PAD_CROSS != 0;
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        self.flags |= STARTED;
        match self.state {
            ST_COUNT => {
                if self.counting {
                    let bonus = self.page != 0;
                    let row =
                        self.step
                            .saturating_sub(if bonus { self.score.line_count } else { 0 });
                    if self
                        .score
                        .count_line(bonus, row, 15, self.menu_cursor, sounds)
                    {
                        self.step += 1;
                        let per_page = if bonus { PAGE } else { self.score.line_count };
                        if self.step.is_multiple_of(per_page.max(1)) || self.step == self.lines() {
                            self.counting = false;
                            self.page_shown = true;
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
                    self.page_shown = true;
                    self.page = self.page_max;
                    self.step = self.lines();
                    self.score.set_page(self.page);
                }
                if self.page_shown {
                    if self.page < self.page_max {
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
                        self.talk = if self.outcome == 0 {
                            if self.chara == ANDROID_16 {
                                4
                            } else if self.score.rank == 4 {
                                1
                            } else {
                                libc.rand() % 2 + 2
                            }
                        } else if self.chara == ANDROID_16 {
                            7
                        } else {
                            libc.rand() % 2 + 5
                        };
                    }
                    self.page_shown = false;
                }
                if self.step == self.lines() {
                    self.state = ST_PAGES;
                }
            }
            ST_PAGES => {
                if up {
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
                } else if down {
                    if self.page < self.page_max {
                        self.page += 1;
                        self.flash.goto_label("fl_battle_cansel_in", true);
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if ok {
                    if self.outcome == 0 {
                        if self.cleared {
                            self.state = ST_CLEAR;
                            self.talk = if self.chara == ANDROID_16 {
                                0x13
                            } else {
                                random.range(2) as i32 + 0x11
                            };
                        } else {
                            self.state = ST_CONTINUE;
                            self.talk = if self.chara == ANDROID_16 {
                                11
                            } else {
                                random.range(3) as i32 + 8
                            };
                        }
                    } else {
                        self.state = ST_OVER;
                    }
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
            ST_OVER => {
                if ok {
                    self.flash.goto_label("fl_gameover_out", true);
                    self.flags |= DONE | LEAVING;
                    self.timer = 15;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            ST_CONTINUE => {
                if up {
                    if self.pick != 0 {
                        self.yes_no_goto("fl_off_start");
                        self.pick = 0;
                        self.yes_no_goto("fl_on_start");
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if down {
                    if self.pick == 0 {
                        self.yes_no_goto("fl_off_start");
                        self.pick = 1;
                        self.yes_no_goto("fl_on_start");
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if ok {
                    if self.pick == 0 {
                        // Go on: the counted total stays with the run.
                        self.result = 0;
                        self.talk = random.range(2) as i32 + 12;
                        self.run.stat[3] = self.score.total;
                    } else {
                        self.flash.goto_label("fl_zokkou_out", true);
                        self.state = ST_TRAIN;
                        self.talk = random.range(2) as i32 + 15;
                    }
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            ST_TRAIN | ST_CLEAR => {
                if ok {
                    self.flash.goto_label(
                        if self.state == ST_TRAIN {
                            "fl_syugyo_out"
                        } else {
                            "fl_gameclear_out"
                        },
                        true,
                    );
                    self.state = ST_MONEY;
                    self.step = 0;
                    self.timer = 60;
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
                            save.add_sim_ranking(self.score.saved, self.chara, self.cleared);
                        }
                    }
                    1 => {
                        if self.score.convert_step(0x3F, sounds) {
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
                            0x3F,
                            1.0,
                            sounds,
                        ) {
                            self.step += 1;
                            ub_score::plate_goto(&mut self.flash, 1, false);
                        }
                        money = money.min(MONEY_MAX);
                    }
                    4 => {
                        if ok {
                            self.flash.goto_label("fl_zpoint_out", true);
                            if self.got_item {
                                self.state = ST_ITEM;
                                sounds.push(TitleSound::Effect(2, 6));
                            } else {
                                self.leave();
                            }
                            sounds.push(TitleSound::Effect(1, 1));
                        }
                    }
                    _ => {}
                }
                if (1..=3).contains(&self.step) && ok {
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

    fn real_data() -> Option<SimResultData> {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
        let mut image = Iso9660::open(&iso).ok()?;
        let location = partition_entry_for_id(crate::ub::ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = SimResultAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = SimResultLayout::new(0, |k| firsts[&k]);
        Some(data)
    }

    fn won() -> BattleResult {
        BattleResult {
            winner: 1,
            health: [40.0, 0.0],
            max_combo_hits: [6, 2],
            max_combo_damage: [3000, 100],
            event_summary: 0b10,
            clock: [0, 2, 30, 0],
            ..BattleResult::default()
        }
    }

    /// Runs the real result from the ISO in `rom/`, when it is there: a won fight goes on with the counted points
    /// added to the run's.
    #[test]
    fn real_sim_result_goes_on() {
        let Some(data) = real_data() else {
            return;
        };
        let run = SimRun {
            turn: 9,
            stat: [3, 2, 80, 500],
            ..SimRun::default()
        };
        let mut save = SaveData::default();
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[4, 2]);
        let mut screen =
            SimResultScreen::new(&data, run, 3, 1, &won(), &mut save, &mut libc).unwrap();
        assert_eq!(screen.bgm(), BGM_FIRST + 4);
        let mut font = Font::new(Vec::new(), 20);
        for _ in 0..80 {
            if screen.result().is_some() {
                break;
            }
            for i in 0..30 {
                let b = if i == 0 { PAD_CROSS } else { 0 };
                screen.run_frame(
                    b,
                    b,
                    VOICE_PLAYEND,
                    &mut save,
                    &mut random,
                    &mut libc,
                    &mut font,
                );
            }
        }
        assert_eq!(screen.result(), Some(0), "state {}", screen.state);
        assert!(screen.run().stat[3] > 500);
        assert_eq!(save.sim_rank(0).score, 0);
    }

    /// The seventh round's fight ends the run: the reward item at once, the ranking and the Z points.
    #[test]
    fn real_sim_result_clears_the_run() {
        let Some(data) = real_data() else {
            return;
        };
        let run = SimRun {
            turn: LAST_TURN,
            stat: [3, 2, 80, 700],
            ..SimRun::default()
        };
        let mut save = SaveData::default();
        let money = save.money();
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[4, 2]);
        let mut screen =
            SimResultScreen::new(&data, run, 3, 1, &won(), &mut save, &mut libc).unwrap();
        assert!(screen.got_item && save.sim_cleared());
        let mut font = Font::new(Vec::new(), 20);
        for _ in 0..80 {
            if screen.result().is_some() {
                break;
            }
            for i in 0..30 {
                let b = if i == 0 { PAD_CROSS } else { 0 };
                screen.run_frame(
                    b,
                    b,
                    VOICE_PLAYEND,
                    &mut save,
                    &mut random,
                    &mut libc,
                    &mut font,
                );
            }
        }
        assert_eq!(screen.result(), Some(1), "state {}", screen.state);
        let rank = save.sim_rank(0);
        assert_eq!((rank.chara, rank.cleared), (3, 1));
        assert!(rank.score > 700);
        assert_eq!(save.money(), money + rank.score);
    }
}
