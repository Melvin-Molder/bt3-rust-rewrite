//! Mission 100's result (`MisResult_Run(3)`, src/menu/mission_result.c, progress mode 16, `Ub_Main`): after the
//! mission's battle, Android 17 tells the outcome; the score sheet's lines and then its bonus pages count into the
//! total, the rank letter appears with the guide's verdict, the total is converted (its counter at a tenth) and paid
//! into the Z points whole.
//! A won mission is recorded (best score with its rank and time, cleared); the 30th, 50th and 100th cleared open a
//! reward window (the Survival plate, an item, the last item and the "all missions cleared" fireworks).

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

/// `MisResult_Run(3)`: section 3 of `gMenuArc3`.
pub const ARCHIVE_SECTION: usize = 3;
const FADE_FRAMES: i32 = 0x14;
const MISSION_NUM: usize = 100;
/// `P_UBFLAG_ITEM3`, `P_UBFLAG_ALL_MISSIONS`.
const SAVE_ITEM3: i32 = 1;
const SAVE_ALL_MISSIONS: i32 = 4;

// MISRESULT_*.
const DONE: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;

// MISRESULT_ST_*.
const ST_INTRO: i32 = 0;
const ST_COUNT: i32 = 1;
const ST_PAGES: i32 = 2;
const ST_MONEY: i32 = 3;
const ST_COMPLETE: i32 = 4;
const ST_REWARD: i32 = 5;
const ST_LEAVE: i32 = 6;

/// The fireworks are drawn inside the result window only (`MisResult_ClipScissorOn`).
const FIREWORKS_SCISSOR: [i32; 4] = [0x84, 0x5A, 0x1DC, 0x1A0];

/// `MR->tex[n]` of `MisResult_Init` (and `UbResult_Init`, the same movie).
pub(crate) const TEX: [ImageTexture; 47] = [
    tex(4, 0),
    tex(4, 3),
    tex(4, 28),
    tex(4, 27),
    tex(4, 31),
    tex(4, 32),
    tex(4, 36),
    tex(4, 38),
    tex(4, 37),
    tex(4, 10),
    tex(4, 9),
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
    tex(4, 44),
    None,
];
/// `IconWin_Init(MPACK_AT(res, 9), res 8)` with `IconWin_SetIcon(1)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((9, 1), 1)),
    Some(((9, 1), 0)),
    tex(8, 0),
    Some(((9, 1), 2)),
    tex(8, 2),
];

/// What `MisResult_Init` reads from its unpacked section.
pub struct MissionResultAssets {
    pub data: MissionResultData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct MissionResultData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 12: the screen's text; 11: the guide's mouth keys; 14: the item names.
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    pub item_text: Vec<u8>,
    /// Section 13.
    pub prices: Vec<Price>,
    pub layout: MissionResultLayout,
}

#[derive(Clone, Debug, Default)]
pub struct MissionResultLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
}

impl MissionResultLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
        }
    }
}

impl MissionResultAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&TEX[..], &ICON_TEXTURES])?;
        Ok(Self {
            data: MissionResultData {
                fod: pack_section(res, 1)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 9)?, 2)?.to_vec(),
                text: pack_section(res, 12)?.to_vec(),
                lips: pack_section(res, 11)?.to_vec(),
                item_text: pack_section(res, 14)?.to_vec(),
                prices: ub_score::parse_prices(pack_section(res, 13)?),
                layout: MissionResultLayout::default(),
            },
            background: decode_picture(res, 5)?,
            files,
        })
    }
}

/// `MisResult_MarkCleared`: the mission marked cleared; whether it was new and the missions cleared now (the
/// hundredth sets the "all missions" flag).
fn mark_cleared(save: &mut SaveData, mission: usize) -> (bool, i32) {
    let mut new = false;
    let mut count = 0;
    for i in 0..MISSION_NUM {
        let mut rec = save.mission(i);
        if rec.cleared != 0 {
            count += 1;
        } else if mission == i {
            new = true;
            rec.cleared = 1;
            save.set_mission(i, rec);
            count += 1;
        }
    }
    if count == MISSION_NUM as i32 && new {
        save.set_ub_flags(save.ub_flags() | SAVE_ALL_MISSIONS);
    }
    (new, count)
}

/// `MisResult` and the frame loop of `MisResult_Run`.
pub struct MissionResultScreen {
    data: MissionResultData,
    flash: Flash,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    timer: i32,
    state: i32,
    voice_req: i32,
    voice_last: i32,
    voice_skip: bool,
    voice_line: i32,
    blink: i32,
    talk: i32,
    pages: usize,
    page: usize,
    outcome: i32,
    complete: bool,
    new_record: bool,
    reward: i32,
    reward_item: i32,
    page_done: bool,
    mission: usize,
    menu_cursor: i32,
    score: Score,
    counting: bool,
    step: usize,
    started: bool,
    done: bool,
}

impl MissionResultScreen {
    /// `MisResult_Init(3)` and the first `ColorFade_StartIn`: the sheet of mission `mission` (`missionPage * 5 +
    /// cursor`) from the battle's result; a win is recorded in the save. `menu_cursor` is `gProgress->ubCursor`.
    pub fn new(
        data: &MissionResultData,
        mission: usize,
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
        let (mut score, outcome, pages) = Score::fill(1, result);
        score.calc_points(&data.prices);
        let total = score.calc_rank(1);
        let mut screen = Self {
            data: data.clone(),
            flash,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            timer: 15,
            state: ST_INTRO,
            voice_req: 0,
            voice_last: 0,
            voice_skip: false,
            voice_line: -1,
            blink,
            talk: 0,
            pages,
            page: 0,
            outcome,
            complete: false,
            new_record: false,
            reward: 0,
            reward_item: 0,
            page_done: false,
            mission: mission.min(MISSION_NUM - 1),
            menu_cursor,
            score,
            counting: false,
            step: 0,
            started: false,
            done: false,
        };
        if outcome == 0 {
            // UbScore_SaveMissionBest.
            let mut rec = save.mission(screen.mission);
            if rec.score < total {
                rec.score = total;
                rec.rank = screen.score.rank as u8;
                rec.time = [0, 1, 2].map(|k| screen.score.clock[k] as u8);
                save.set_mission(screen.mission, rec);
                screen.new_record = true;
            }
            let (new, cleared) = mark_cleared(save, screen.mission);
            if new {
                match cleared {
                    30 => {
                        screen.reward = 1;
                        save.set_ub_flags(save.ub_flags() | SAVE_ITEM3);
                    }
                    50 => {
                        screen.reward = 2;
                        screen.reward_item = ub_score::reward_item(1);
                        save.add_item(screen.reward_item as usize);
                    }
                    100 => {
                        screen.complete = true;
                        screen.reward = 3;
                        screen.reward_item = ub_score::reward_item(2);
                        save.add_item(screen.reward_item as usize);
                    }
                    _ => {}
                }
            }
        }
        Ok(screen)
    }

    /// `MisResult_Run` has returned.
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

    /// After a page has counted: the next bonus page, or the rank with the guide's verdict.
    fn next_page(&mut self, random: &mut GameRandom, sound: bool, sounds: &mut Vec<TitleSound>) {
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
            if sound {
                sounds.push(TitleSound::Effect(2, 0x13));
            }
            self.voice_req = if self.score.rank == 4 {
                16
            } else if random.range(2) != 0 {
                17
            } else {
                18
            };
        }
        self.page_done = false;
    }

    /// One iteration of `MisResult_Run`'s loop.
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
        } else if self.voice_req == 0 {
            self.input(pressed, repeat, save, random, &mut frame.sounds);
        }
        frame
    }

    /// `MisResult_Update`: the timer, the movie and its triggers.
    fn update(&mut self, random: &mut GameRandom, sounds: &mut Vec<TitleSound>) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        self.flash.advance();
        let trig = self.flash.trig();
        if trig & 1 != 0 && self.state == ST_INTRO {
            self.started = true;
        }
        if trig & 2 != 0 && self.state == ST_COMPLETE {
            self.flash.goto_label("fl_itemget_in", true);
            self.state = ST_REWARD;
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
                        self.voice_req = if self.complete { 11 } else { 1 };
                        "mc_win_text"
                    } else {
                        self.voice_req = 6;
                        "mc_lose_text"
                    };
                    let r = self.flash.find(None, clip);
                    self.flash.clip_goto_label(r, "fl_start");
                }
                ST_MONEY => self.flash.goto_label("fl_zpoint_in", true),
                ST_COMPLETE => {
                    for clip in ["mc_clear_anime", "mc_hanabi"] {
                        let r = self.flash.find(None, clip);
                        self.flash.clip_goto_label(r, "fl_start");
                    }
                }
                ST_REWARD => {
                    self.flash.goto_label("fl_itemget_in", true);
                    sounds.push(TitleSound::Effect(2, 6));
                }
                _ => {}
            }
        }
        if self.page_done && self.timer == 0 {
            self.next_page(random, true, sounds);
        }
    }

    /// `MisResult_UpdateVoice`: the guide's script (1 won, 6 lost, 11 all missions cleared, each ending with a
    /// step of its own; 16..19 one-line comments).
    fn update_voice(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if !self.voice_skip {
            if self.voice_req == 0 {
                return;
            }
            if self.voice_line != -1
                && voice_stat != VOICE_PLAYEND
                && self.voice_req == self.voice_last
            {
                if pressed & PAD_CROSS != 0 {
                    sounds.push(TitleSound::Effect(1, 1));
                    sounds.push(TitleSound::VoiceStop);
                    self.lip = LipSync::default();
                    self.voice_skip = true;
                }
                return;
            }
        } else {
            self.voice_skip = false;
        }
        match self.voice_req {
            1 => {
                self.voice_line = 0x38;
                self.voice_req += 1;
            }
            6 => {
                self.voice_line = 0x39;
                self.voice_req += 1;
            }
            16 => self.voice_line = 0x3A,
            17 => self.voice_line = 0x3B,
            18 => self.voice_line = 0x3C,
            19 => self.voice_line = 0x3D,
            11 => {
                self.voice_line = 0x3E;
                self.voice_req += 1;
            }
            2 | 7 | 12 => {
                self.voice_req = 0;
                self.voice_line = -1;
            }
            _ => {}
        }
        if self.voice_req >= 16 {
            self.voice_req = 0;
        }
        if self.voice_line != -1 {
            anim::play_with_subtitle(
                &self.data.lips,
                VOICE_BASE,
                self.voice_line,
                &mut self.lip,
                sounds,
            );
            self.voice_last = self.voice_req;
        }
    }

    /// `MisResult_Draw`.
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
        let f = &mut self.flash;
        let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_eye");
        anim::blink(f, r, &mut self.blink, 0, random);
        let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_mouth");
        anim::talk(f, r, &mut self.talk, 0, &mut self.lip, voice_stat, random);
        // The mission's name.
        let title = TextBox::preset(0)
            .with_align(0)
            .with_max_width(0xEB)
            .with_line_y([0, -10, 0, 0, 0]);
        let r = f.find(Some("mc_window_plate01"), "mc_window_text1_0");
        texts.extend(title.attach_line(
            f,
            r,
            0x400,
            &self.data.text,
            self.mission as i32 + 0xD8,
            font,
        ));
        for clip in ["mc_win_text02", "mc_win_text03"] {
            let r = f.find(None, clip);
            f.clip_set_uv(r, uv(0, 0x40, 0x200, 0x80));
        }
        // The page arrows.
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
        // The score lines: value and points.
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
        // The three bonus plates of the page.
        let plain = TextBox::preset(0).with_align(0);
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
        // The captions of lines 2..4.
        for i in 1..4 {
            let name = format!("mc_point_plate{:02}", i + 1);
            let r = f.find(Some(&name), "mc_bonus_text");
            f.clip_set_uv(r, uv(0, i << 5, 0x100, (i << 5) + 0x20));
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
        // The rank letter.
        let x = self.score.rank << 6;
        let r = f.find(None, "mc_rankmoji_anime");
        f.clip_set_uv(r, uv(x, 0, x + 0x40, 0x40));
        let r = f.find(None, "mc_newrecord_text");
        if self.new_record {
            f.clip_set_uv(r, uv(0, 0x40, 0x100, 0x80));
            f.clip_set_flags(r, PROP_VISIBLE, true);
        } else {
            f.clip_set_flags(r, PROP_VISIBLE, false);
        }
        let r = f.find(None, "mc_hanabi");
        f.clip_set_scissor(r, Some(FIREWORKS_SCISSOR));
        if self.reward != 0 {
            let centred = TextBox::preset(0).with_align(1);
            let r = f.find(None, "mc_text_kari0");
            texts.extend(centred.attach_line(
                f,
                r,
                0x404,
                &self.data.text,
                self.reward + 0xF,
                font,
            ));
            match self.reward {
                1 => {
                    for clip in ["mc_item_get_text", "mc_item_plate00"] {
                        let r = f.find(None, clip);
                        f.clip_set_flags(r, PROP_VISIBLE, false);
                    }
                    let r = f.find(None, "mc_text_kari1");
                    texts.extend(centred.attach_line(f, r, 0x405, &self.data.text, 0x13, font));
                }
                _ => {
                    let item_box = TextBox::preset(0)
                        .with_align(0)
                        .with_line_y([0xC, 0, 0, 0, 0]);
                    let r = f.find(Some("mc_item_plate00"), "mc_item_text_kari");
                    texts.extend(item_box.attach_line(
                        f,
                        r,
                        0x406,
                        &self.data.item_text,
                        self.reward_item,
                        font,
                    ));
                }
            }
        }
        let tex = &self.data.layout.flash;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (fq, overs) = f.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&fq, tex));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.icon.draw(quads);
    }

    /// `MisResult_Input`.
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
                // The battle was aborted: nothing to show.
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
                    let index =
                        self.step
                            .saturating_sub(if bonus { self.score.line_count } else { 0 });
                    if self
                        .score
                        .count_line(bonus, index, 15, self.menu_cursor, sounds)
                    {
                        self.step += 1;
                        let per_page = if bonus { PAGE } else { self.score.line_count };
                        if self.step.is_multiple_of(per_page.max(1)) || self.step == self.lines() {
                            self.counting = false;
                            self.page_done = true;
                        }
                    }
                }
                if confirm {
                    // Skip the counting.
                    for i in 0..self.lines() {
                        let (bonus, index) = if i < self.score.line_count {
                            (false, i)
                        } else {
                            (true, i - self.score.line_count)
                        };
                        while !self
                            .score
                            .count_line(bonus, index, 9999, self.menu_cursor, sounds)
                        {
                        }
                    }
                    self.page_done = true;
                    self.page = self.pages;
                    self.step = self.lines();
                    self.score.set_page(self.page);
                }
                if self.page_done {
                    self.next_page(random, false, sounds);
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
                            self.voice_req = 19;
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
                            if self.complete {
                                self.state = ST_COMPLETE;
                            } else if self.reward != 0 {
                                self.state = ST_REWARD;
                                sounds.push(TitleSound::Effect(
                                    2,
                                    if self.reward == 1 { 7 } else { 6 },
                                ));
                            } else {
                                self.leave();
                            }
                            sounds.push(TitleSound::Effect(1, 1));
                        }
                    }
                    _ => {}
                }
                if (1..=3).contains(&self.step) && confirm {
                    // Skip the pay-out.
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
            ST_COMPLETE => {
                if confirm {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            ST_REWARD => {
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

    /// Runs the real mission result from the ISO in `rom/`, when it is there: a won mission's sheet counted, the
    /// pages, the pay-out into the save's Z points, and the record and cleared mark kept.
    #[test]
    fn real_mission_result_pays_and_records() {
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
        let assets = MissionResultAssets::parse(&res).unwrap();
        assert_eq!(assets.data.prices.len(), 101);
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = MissionResultLayout::new(0, |k| firsts[&k]);
        let result = BattleResult {
            winner: 1,
            health: [62.5, 0.0],
            max_combo_hits: [12, 3],
            max_combo_damage: [9000, 100],
            event_summary: 0b1_0000_1001,
            clock: [0, 2, 30, 100],
            ..BattleResult::default()
        };
        let mut save = SaveData::default();
        let money = save.money();
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[6, 6]);
        let mut screen =
            MissionResultScreen::new(&data, 7, 1, &result, &mut save, &mut libc).unwrap();
        assert_eq!(screen.outcome, 0);
        assert!(screen.new_record);
        let rec = save.mission(7);
        assert_eq!((rec.cleared, rec.time), (1, [0, 2, 30]));
        assert!(rec.score > 0);
        assert_eq!(screen.pages, 1);
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
        // The whole total went into the Z points.
        assert_eq!(save.money(), money + rec.score);
    }
}
