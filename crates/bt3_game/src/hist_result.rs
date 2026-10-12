//! The result screen after a story battle (`HistResult_Run(3)`, src/menu/history_result.c, progress mode 8). A win
//! marks the episode cleared, counts the points of the chosen level up and pays them into the money, then shows
//! one reward window (`GetWin`) per thing the battle gave (items, characters, stages, a newly listed saga, new
//! episodes, the dragon ball found), each already written to the save; a defeat only shows its caption.
//!
//! It leaves for the save screen (mode 10), for the saga's outro when the win cleared every episode of a saga
//! whose outro was not seen (mode 9), or after a defeat back to the episode list (mode 7).

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, VOICE_PLAYEND};
use crate::char_select::PictureLoader;
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_ADD, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::get_win::{
    GetWin, GetWinData, KIND_CHARA, KIND_DRAGON_BALL, KIND_EPISODE, KIND_ITEM, KIND_SAGA,
    KIND_STAGE,
};
use crate::hist_select::HistProgress;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::mode_menu::EPISODES;
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{PAD_CROSS, TitleFrame, TitleSound};

/// `HistResult_Run(3)`: section 3 of `gMenuArc2`.
pub const ARCHIVE_SECTION: usize = 3;
/// The four pictures behind the screen (three for a win, the last for a defeat).
const BG_FILE: u32 = 0x3FE;
const FADE_FRAMES: i32 = 0x14;
/// The jingles: a defeat, a win, the win that clears a saga.
const JINGLE_LOST: u32 = 0x10BC8;
const JINGLE_WON: u32 = 0x10BC7;
const JINGLE_CLEARED: u32 = 0x10BC9;
const MENU_BGM: u32 = 0x10B18;
/// Points are doubled while this item is owned.
const DOUBLE_POINTS_ITEM: usize = 0x88;

const FLAG_CHOSEN: u32 = 1;
const FLAG_LEAVING: u32 = 2;
const FLAG_STARTED: u32 = 4;
const FLAG_FADED_IN: u32 = 8;
const FLAG_LOST: u32 = 0x20;
const FLAG_OUT: u32 = 0x40;

/// `SaveSlot.flags` of a saga.
const SLOT_LISTED: i32 = 1;
const SLOT_NEW: i32 = 2;
const SLOT_OUTRO_SEEN: i32 = 4;
const SLOT_NEW_EPISODE: i32 = 0x40;
/// `MUNLOCK_HIST_NEW_SAGA`.
const UNLOCK_NEW_SAGA: i32 = 0x200;

/// `gHistResult->tex[n]` of `HistResult_Init` from the screen's sections (slot 4 is the saga's, 9..13 the
/// streamed background's).
const FLASH_TEXTURES: [ImageTexture; 14] = {
    let mut t = [None; 14];
    t[3] = tex(2, 0);
    t[0] = tex(2, 1);
    t[1] = tex(2, 2);
    t[8] = tex(2, 3);
    t[5] = tex(2, 4);
    t[7] = tex(2, 5);
    t[6] = tex(3, 0);
    t[2] = tex(3, 1);
    t
};

/// What `HistResult_Init` reads from its section.
pub struct HistResultAssets {
    pub data: HistResultData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct HistResultData {
    pub fod: Vec<u8>,
    get_win: GetWinData,
    pub layout: HistResultLayout,
}

#[derive(Clone, Debug, Default)]
pub struct HistResultLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub get_win: Vec<Option<usize>>,
}

impl HistResultLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&FLASH_TEXTURES, &first),
            get_win: image_indices(&crate::get_win::textures(5), &first),
        }
    }
}

impl HistResultAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&FLASH_TEXTURES[..], &crate::get_win::textures(5)[..]])?;
        Ok(Self {
            data: HistResultData {
                fod: pack_section(res, 4)?.to_vec(),
                get_win: GetWinData::parse(pack_section(res, 5)?)?,
                layout: HistResultLayout::default(),
            },
            background: decode_picture(res, 1)?,
            files,
        })
    }
}

/// How `HistResult_Run` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistResultResult {
    /// 0: a defeat, back to the episode list (mode 7).
    Lost,
    /// 1: a win, on to the save screen (mode 10).
    Won,
    /// 2: the win cleared the saga: its outro (mode 9).
    Cleared,
}

/// `HistResult_SplitEpisode`: an episode counted through all sagas as saga and episode (20..23 rotated back).
fn split_episode(id: i32) -> (usize, i32) {
    let mut id = match id {
        20 => 23,
        21 => 20,
        22 => 21,
        23 => 22,
        n => n,
    };
    let mut saga = 0;
    for &count in &EPISODES {
        if id < count {
            break;
        }
        saga += 1;
        id -= count;
    }
    (saga, id)
}

/// `HistResult_IsMenuCleared`: every episode of the saga has its cleared bit.
fn saga_cleared(save: &SaveData, saga: usize) -> bool {
    let cleared = save.slot_val(saga, 1);
    (0..EPISODES[saga.min(7)]).all(|i| cleared & (1 << i) != 0)
}

pub struct HistResultScreen {
    layout: HistResultLayout,
    textures: Vec<Option<usize>>,
    flash: Flash,
    get_win: GetWin,
    fade: ColorFade,
    saga: usize,
    item: i32,
    flags: u32,
    timer: i32,
    state: i32,
    points: i32,
    points_total: i32,
    se_state: i32,
    jingle: u32,
    scroll: [f32; 4],
    rewards: Vec<(i32, i32)>,
    reward_index: usize,
    result: HistResultResult,
    done: Option<HistResultResult>,
}

impl HistResultScreen {
    /// `HistResult_Init(3)` and the first `ColorFade_StartIn`. `won` is the battle result's winner flag, `ball`
    /// its `dragonBallFound` (1-based, 0 none); `saga_picture` the saga pack's section 2 entry 1.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &HistResultData,
        progress: &HistProgress,
        won: bool,
        ball: i32,
        saga_picture: Option<usize>,
        save: &mut SaveData,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let saga = progress.sub_menu.clamp(0, 7) as usize;
        let mut flags = 0;
        let bg = if won {
            let bg = random.range(3) as usize;
            let cleared = save.slot_val(saga, 1) | 1 << (progress.sub_menu_item & 31);
            save.set_slot_val(saga, 1, cleared);
            bg
        } else {
            flags |= FLAG_LOST;
            3
        };
        let mut textures = data.layout.flash.clone();
        textures[4] = saga_picture;
        let first = loader.texture_file(BG_FILE + bg as u32);
        match bg {
            0 => {
                textures[10] = first;
                textures[9] = first.map(|f| f + 2);
            }
            _ => textures[10 + bg] = first,
        }
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let mut screen = Self {
            layout: data.layout.clone(),
            textures,
            flash,
            get_win: GetWin::new(&data.get_win, data.layout.get_win.clone())?,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            saga,
            item: progress.sub_menu_item,
            flags,
            timer: 0,
            state: 0,
            points: 0,
            points_total: 0,
            se_state: 0,
            jingle: JINGLE_LOST,
            scroll: [0.0; 4],
            rewards: Vec::new(),
            reward_index: 0,
            result: HistResultResult::Won,
            done: None,
        };
        if !won {
            screen.points_total = 0;
        } else {
            screen.jingle =
                if saga_cleared(save, saga) && save.slot_flags(saga) & SLOT_OUTRO_SEEN == 0 {
                    JINGLE_CLEARED
                } else {
                    JINGLE_WON
                };
            screen.build_rewards(progress, ball, save);
        }
        Ok(screen)
    }

    pub fn result(&self) -> Option<HistResultResult> {
        self.done
    }

    /// `HistResult_BuildRewards`: what the won battle gives, written to the save as it is listed.
    fn build_rewards(&mut self, progress: &HistProgress, ball: i32, save: &mut SaveData) {
        let rw = &progress.reward;
        // The sagas the new episodes belong to that are not listed yet (each once).
        let mut unlock = [-1i32; 3];
        for i in 0..3 {
            if rw.episode[i] >= 0 {
                let (saga, _) = split_episode(rw.episode[i]);
                if save.slot_flags(saga) & SLOT_LISTED == 0 && !unlock.contains(&(saga as i32)) {
                    unlock[i] = saga as i32;
                }
            }
        }
        let level = progress.level.clamp(0, 2) as usize;
        self.points_total = rw.points[level];
        if save.item(DOUBLE_POINTS_ITEM) & 1 != 0 {
            self.points_total *= 2;
        }
        for &id in &rw.item {
            if id >= 0 && save.item(id as usize) & 1 == 0 {
                self.rewards.push((KIND_ITEM, id));
                save.add_item(id as usize);
            }
        }
        for &id in &rw.chara {
            if id >= 0 && !save.chara_unlocked(id) {
                self.rewards.push((KIND_CHARA, id));
                save.chara_bits[(id >> 6) as usize] |= 1 << (id & 63);
            }
        }
        for &id in &rw.stage {
            if id >= 0 && !save.stage_unlocked(id) {
                self.rewards.push((KIND_STAGE, id));
                save.stage_bits |= 1 << id;
            }
        }
        for &id in &unlock {
            if id >= 0 && save.slot_flags(id as usize) & SLOT_LISTED == 0 {
                self.rewards.push((KIND_SAGA, id));
                let s = id as usize;
                save.set_slot_flags(s, save.slot_flags(s) | SLOT_LISTED | SLOT_NEW);
                save.set_unlock_flags(save.unlock_flags() | UNLOCK_NEW_SAGA);
            }
        }
        for &id in &rw.episode {
            if id >= 0 {
                let (saga, item) = split_episode(id);
                let bit = 1 << (item & 31);
                if save.slot_val(saga, 0) & bit == 0 {
                    self.rewards.push((KIND_EPISODE, id));
                    save.set_slot_val(saga, 0, save.slot_val(saga, 0) | bit);
                    save.set_slot_val(saga, 2, save.slot_val(saga, 2) | bit);
                    save.set_slot_flags(saga, save.slot_flags(saga) | SLOT_NEW_EPISODE);
                }
            }
        }
        if ball > 0 {
            self.rewards.push((KIND_DRAGON_BALL, ball - 1));
            save.set_unlock_flags(save.unlock_flags() | 1 << (ball - 1));
        }
    }

    /// One frame of the `HistResult_Run` loop. `stream_stat` is `StreamSe_GetStat(0)`.
    pub fn run_frame(
        &mut self,
        pressed: u32,
        stream_stat: i32,
        save: &mut SaveData,
        font: &mut Font,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done.is_some() {
            return frame;
        }
        let sounds = &mut frame.sounds;
        self.fade.update();
        self.update();
        self.update_se(stream_stat, sounds);
        self.update_state(pressed, save, sounds);
        self.draw(save, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done() && self.flags & FLAG_FADED_IN == 0 {
            // StreamSe_PlayPausedDefault at Init, resumed here.
            self.flags |= FLAG_FADED_IN;
            self.se_state = 1;
            sounds.push(TitleSound::StreamSe(self.jingle));
        }
        if self.fade.is_fading_out() {
            sounds.push(TitleSound::BgmFadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = Some(self.result);
            return frame;
        }
        if self.flags & FLAG_LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        } else if self.se_state == 0 && self.state == 0 && self.flags & FLAG_OUT == 0 {
            self.input(pressed, save, sounds);
        }
        frame
    }

    /// `HistResult_Update`: once "fl_out" has played, leave.
    fn update(&mut self) {
        self.flash.advance();
        if self.flags & FLAG_OUT != 0 && self.flash.trig() & 1 != 0 {
            self.flags |= FLAG_CHOSEN | FLAG_LEAVING;
            self.timer = 1;
            self.flags ^= FLAG_OUT;
        }
    }

    /// `HistResult_UpdateSe`: the jingle, then the menu music.
    fn update_se(&mut self, stream_stat: i32, sounds: &mut Vec<TitleSound>) {
        match self.se_state {
            1 => self.se_state = 2,
            2 => {
                if stream_stat == VOICE_PLAYEND {
                    self.se_state += 1;
                }
            }
            3 => {
                sounds.push(TitleSound::Bgm(MENU_BGM));
                self.se_state = 0;
            }
            _ => {}
        }
    }

    /// `HistResult_Input`: the point count starts once the movie takes input; confirm then leaves through the
    /// rewards.
    fn input(&mut self, pressed: u32, save: &SaveData, sounds: &mut Vec<TitleSound>) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & FLAG_STARTED == 0 {
            self.flags |= FLAG_STARTED;
            if self.flags & FLAG_LOST == 0 {
                self.state = 1;
                return;
            }
        }
        if pressed & PAD_CROSS != 0 {
            self.result = if self.flags & FLAG_LOST != 0 {
                HistResultResult::Lost
            } else if saga_cleared(save, self.saga)
                && save.slot_flags(self.saga) & SLOT_OUTRO_SEEN == 0
            {
                HistResultResult::Cleared
            } else {
                HistResultResult::Won
            };
            self.state = 51;
            sounds.push(TitleSound::Effect(1, 1));
        }
    }

    /// `HistResult_UpdateState`: the point count and the reward windows.
    fn update_state(&mut self, pressed: u32, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        if self.state == 0 || self.se_state != 0 {
            return;
        }
        let cross = pressed & PAD_CROSS != 0;
        match self.state {
            1 => self.state += 1,
            // Counting up by 33 (confirm: at once).
            2 => {
                if cross {
                    self.points = self.points_total;
                    self.state += 1;
                    sounds.push(TitleSound::Effect(1, 1));
                } else {
                    self.points += 0x21;
                    if self.points >= self.points_total {
                        self.points = self.points_total;
                        self.state += 1;
                    }
                }
            }
            3 => {
                if cross {
                    self.state += 1;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            // Paying out by 33 (confirm: at once).
            4 => {
                if cross || self.points <= 0x20 {
                    save.add_money(self.points);
                    self.points = 0;
                    self.state += 1;
                    if cross {
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                } else {
                    self.points -= 0x21;
                    save.add_money(0x21);
                }
            }
            5 => self.state = 0,
            51 => {
                if self.rewards.is_empty() {
                    self.state = 57;
                } else {
                    self.state += 1;
                    self.reward_index = 0;
                }
            }
            52 => {
                let (kind, id) = self.rewards[self.reward_index];
                self.get_win.setup(kind, id);
                if self.reward_index != 0 {
                    self.get_win.next(sounds);
                } else {
                    self.get_win.open(sounds);
                }
                self.reward_index += 1;
                self.state += 1;
            }
            53 | 56 => {
                if self.get_win.is_anim_done() {
                    self.state += 1;
                }
            }
            54 => {
                if cross {
                    self.state = if self.reward_index == self.rewards.len() {
                        55
                    } else {
                        52
                    };
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            55 => {
                self.state += 1;
                self.get_win.close();
            }
            57 => {
                self.state = 0;
                self.flags |= FLAG_OUT;
                self.flash.goto_label("fl_out", true);
            }
            _ => {}
        }
    }

    /// `HistResult_Draw`.
    fn draw(&mut self, save: &SaveData, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let lost = self.flags & FLAG_LOST != 0;
        let f = &mut self.flash;
        let row = |v0, h| Uv {
            u0: 0,
            v0,
            u1: 0x200,
            v1: v0 + h,
            tex: 0,
        };
        let clip = f.find(None, "mc_result_text_1");
        if lost {
            f.clip_set_flags(clip, PROP_VISIBLE, false);
        } else {
            f.clip_set_uv(clip, row(0, 0x80));
            f.clip_set_flags(clip, PROP_VISIBLE, true);
        }
        let clip = f.find(None, "mc_result_text_2");
        if lost {
            f.clip_set_uv(clip, row(0x80, 0x80));
            f.clip_set_flags(clip, PROP_VISIBLE, true);
        } else {
            f.clip_set_flags(clip, PROP_VISIBLE, false);
        }
        for i in 0..2 {
            let clip = f.find(None, &format!("mc_point_text_{}", i + 1));
            f.clip_set_uv(clip, row(i * 0x20, 0x20));
        }
        anim::num_draw(
            f,
            |n| format!("mc_get_point_{n}"),
            4,
            self.points,
            (0x40, 0x40),
            0,
        );
        anim::num_draw(
            f,
            |n| format!("mc_total_point_{n}"),
            7,
            save.money(),
            (0x40, 0x40),
            0,
        );
        for (i, speed) in [17.066_667, 17.066_667, 8.533_333, 8.533_333]
            .into_iter()
            .enumerate()
        {
            let clip = f.find(None, &format!("mc_line_{}", i + 1));
            f.clip_set_flags(clip, PROP_ADD, true);
            anim::scroll(f, clip, row(0, 0x100), &mut self.scroll[i], -speed);
        }
        let clip = f.find(None, "mc_battle_text");
        f.clip_set_uv(clip, row((self.item % 4) * 0x40, 0x40));
        f.clip_set_tex(clip, self.item / 4);
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        quads.extend(screen_quads(&f.draw(&has_texture), textures));
        self.get_win.draw(font, quads);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hist_select::HistReward;

    #[test]
    fn episodes_split_back_into_sagas() {
        assert_eq!(split_episode(0), (0, 0));
        assert_eq!(split_episode(5), (1, 2));
        assert_eq!(split_episode(21), (4, 1));
        assert_eq!(split_episode(20), (4, 4));
        assert_eq!(split_episode(47), (7, 3));
        for saga in 0..8 {
            for item in 0..EPISODES[saga as usize] {
                let line = crate::mode_menu::episode_line(saga, item);
                assert_eq!(split_episode(line), (saga as usize, item));
            }
        }
    }

    struct Pictures;

    impl PictureLoader for Pictures {
        fn portrait(&mut self, _: u32) -> Option<usize> {
            None
        }
        fn picture(&mut self, _: u32) -> Option<usize> {
            None
        }
        fn texture_file(&mut self, file_id: u32) -> Option<usize> {
            Some(file_id as usize)
        }
    }

    /// Runs the real result screen from the ISO in `rom/`, when it is there: saga 0's first episode won at level
    /// 1 with script 0's rewards (1000 points, episode 1) and a dragon ball.
    #[test]
    fn real_result_pays_the_points_and_lists_the_rewards() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let location = partition_entry_for_id(crate::hist_select::ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = HistResultAssets::parse(&res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = HistResultLayout::new(5, |key| firsts[&key]);
        let progress = HistProgress {
            sub_menu: 0,
            sub_menu_item: 0,
            level: 1,
            reward: HistReward {
                points: [500, 1000, 1500],
                episode: [1, -1, -1],
                ..HistReward::default()
            },
        };
        let mut save = SaveData::default();
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[8]);
        let mut screen = HistResultScreen::new(
            &data,
            &progress,
            true,
            3,
            Some(7),
            &mut save,
            &mut Pictures,
            &mut random,
        )
        .unwrap();
        assert_eq!(screen.points_total, 1000);
        assert_eq!(screen.rewards, [(KIND_EPISODE, 1), (KIND_DRAGON_BALL, 2)]);
        assert_eq!(save.slot_val(0, 0) & 2, 2);
        assert_eq!(save.slot_val(0, 1) & 1, 1);
        assert_ne!(save.unlock_flags() & 4, 0);
        let mut sounds = Vec::new();
        macro_rules! step {
            ($bits:expr) => {
                sounds.extend(
                    screen
                        .run_frame($bits, VOICE_PLAYEND, &mut save, &mut font)
                        .sounds,
                )
            };
        }
        for _ in 0..60 {
            step!(0);
        }
        assert!(sounds.contains(&TitleSound::StreamSe(JINGLE_WON)));
        assert!(sounds.contains(&TitleSound::Bgm(MENU_BGM)));
        // The count runs up to 1000 once the movie takes input, waits for confirm, then pays it into the money.
        for _ in 0..400 {
            step!(0);
            if screen.state == 3 {
                break;
            }
        }
        assert_eq!((screen.state, screen.points), (3, 1000));
        step!(PAD_CROSS);
        for _ in 0..60 {
            step!(0);
        }
        assert_eq!((save.money(), screen.points, screen.state), (1000, 0, 0));
        // Confirm: the two reward windows, then the screen leaves for the save screen.
        for _ in 0..12 {
            step!(PAD_CROSS);
            for _ in 0..40 {
                step!(0);
            }
            if screen.result().is_some() {
                break;
            }
        }
        assert_eq!(screen.reward_index, 2);
        assert_eq!(screen.result(), Some(HistResultResult::Won));
    }
}
