//! The scene after a saga is completed (`HistOutro_Run(2)`, src/menu/history_outro.c, progress mode 9): over the
//! saga's background its guide and a partner talk through a scripted dialogue (the saga guide's voice set), then the
//! saga's closing text scrolls in and is read out. The first completion of sagas 1, 4, 5 and 7 raises the shop's
//! stock level; the saga is marked as having had its outro.
//!
//! It leaves for the save screen (mode 10), or for the ending movie (mode 3) when saga 6 is completed while the
//! ending is not yet listed.

use std::error::Error;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, screen_quads};
use crate::font::Font;
use crate::logo::ColorFade;
use crate::menu_res::{ImageTexture, tex};
use crate::mode_menu::{Descr, ModeBg, ModeMenuData, SAGA, SagaData};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{PAD_CROSS, PAD_START, TitleFrame, TitleSound};
use crate::windows::{MsgWin, TextBox, TextDraw, draw_texts};

const FADE_FRAMES: i32 = 0x14;
const BGM: u32 = 0x10B1C;
/// `MODEMENU_NARR_VOICE`.
const NARR_VOICE: u32 = 0x84B6;

const FLAG_CHOSEN: u32 = 1;
const FLAG_LEAVING: u32 = 2;
const FLAG_STARTED: u32 = 4;
const FLAG_BEGUN: u32 = 8;
const FLAG_SCROLLING: u32 = 0x10;

const STEP_CLOSE: i32 = 51;
const STEP_TEXT: i32 = 501;
const STEP_GO: i32 = 551;

/// `SaveSlot.flags` of a saga.
const SLOT_OUTRO_SEEN: i32 = 4;
const SLOT_OUTRO_STARTED: i32 = 0x10;

/// Per saga: episodes (the closing text's description), the guide's voice set, the dialogue's first line and the
/// partner (a saga's guide, 8 Goku).
const SETUP: [(i32, u32, i32, usize); 8] = [
    (3, 0x8537, 8, 1),
    (4, 0x8423, 4, 2),
    (7, 0x83B4, 9, 3),
    (5, 0x83DB, 10, 5),
    (16, 0x8484, 9, 8),
    (5, 0x8440, 10, 6),
    (4, 0x8402, 6, 8),
    (4, 0x8466, 8, 8),
];

/// The three face textures of guide `g` (`1 + 3 * g`; guide 3's middle two the other way round).
fn face_slots(g: usize) -> [usize; 3] {
    let b = 1 + 3 * g;
    if g == 3 {
        [b, b + 2, b + 1]
    } else {
        [b, b + 1, b + 2]
    }
}

/// `gHistOutro->tex[n]` of `HistOutro_Init` for saga `saga`: the saga guide's faces from the saga pack's
/// section 7, the partner's from its section 8, the rest from the screen's and the saga's sections.
pub(crate) fn flash_textures(saga: usize) -> [ImageTexture; 32] {
    let mut t = [None; 32];
    let saga = saga.min(7);
    for (&slot, e) in face_slots(saga).iter().zip([0, 1, 3]) {
        t[slot] = tex(SAGA + 7, e);
    }
    for (&slot, e) in face_slots(SETUP[saga].3).iter().zip([0, 1, 3]) {
        t[slot] = tex(SAGA + 8, e);
    }
    t[29] = tex(1, 7);
    t[30] = tex(SAGA + 2, 0);
    t[28] = tex(SAGA + 3, 0);
    t[0] = tex(4, 0);
    t
}

/// How `HistOutro_Run` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistOutroResult {
    /// 0: the save screen (mode 10).
    Save,
    /// 1: the ending movie (mode 3).
    Ending,
}

pub struct HistOutroScreen {
    textures: Vec<Option<usize>>,
    text: Vec<u8>,
    lips: Vec<u8>,
    descr: Descr,
    bg: ModeBg,
    flash: Flash,
    msg: MsgWin,
    fade: ColorFade,
    lip: LipSync,
    saga: usize,
    partner: usize,
    voice_base: u32,
    line_base: i32,
    flags: u32,
    timer: i32,
    step: i32,
    voice_line: i32,
    speaker: usize,
    blink: [i32; 2],
    talk: [i32; 2],
    scroll: f32,
    scroll_max: f32,
    narr_wait: i32,
    narr_line: i32,
    done: Option<HistOutroResult>,
}

impl HistOutroScreen {
    /// `HistOutro_Init(2)` (its music, the first completion's count, the outro marked seen) and
    /// `ColorFade_StartIn`.
    pub fn new(
        data: &ModeMenuData,
        saga: &SagaData,
        save: &mut SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) -> Result<Self, Box<dyn Error>> {
        let s = saga.saga.min(7);
        let (item_max, voice_base, line_base, partner) = SETUP[s];
        let mut flash = Flash::create(&data.outro_fod)?;
        flash.play(1);
        let msg = MsgWin::new(&data.msg_fod, data.msg.clone(), saga.msg_text.clone(), true)?;
        let blink = [0; 2].map(|_| random.range(0x20) as i32);
        let (bgm, ambient) = match s {
            0 => (BGM, Some(0x10BDC)),
            1 => (BGM, Some(0x10BDD)),
            2 => (BGM, Some(0x10BDE)),
            5 => (0x10B1E, None),
            6 => (0x10B28, None),
            _ => (BGM, None),
        };
        sounds.push(TitleSound::Bgm(bgm));
        sounds.extend(ambient.map(TitleSound::StreamSe));
        // OUTRO_COUNT: the first completion of these sagas raises the shop's stock.
        if matches!(s, 1 | 4 | 5 | 7) && save.slot_flags(s) & SLOT_OUTRO_SEEN == 0 {
            save.raise_stock_level();
        }
        save.set_slot_flags(s, save.slot_flags(s) | SLOT_OUTRO_SEEN);
        Ok(Self {
            textures: saga.layout.outro.clone(),
            text: data.text.clone(),
            lips: saga.lips.clone(),
            descr: saga
                .descr
                .get(item_max as usize)
                .copied()
                .unwrap_or_default(),
            bg: ModeBg::new(saga)?,
            flash,
            msg,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            lip: LipSync::default(),
            saga: s,
            partner,
            voice_base,
            line_base,
            flags: 0,
            timer: 0,
            step: 0,
            voice_line: -1,
            speaker: 0,
            blink,
            talk: [0; 2],
            scroll: 0.0,
            scroll_max: 0.0,
            narr_wait: 0,
            narr_line: 0,
            done: None,
        })
    }

    pub fn result(&self) -> Option<HistOutroResult> {
        self.done
    }

    /// `HistOutro_GuideGoto(0, side, out)`: the saga's guide (side 0, right) or the partner (left) walks in or out.
    fn guide_goto(&mut self, side: usize, out: bool) {
        let (guide, label) = if side == 0 {
            (self.saga, if out { "fl_right_out" } else { "fl_right_in" })
        } else {
            (self.partner, if out { "fl_left_out" } else { "fl_left_in" })
        };
        let clip = self.flash.find(None, &format!("mc_guide_{guide:02}"));
        self.flash.clip_goto_label(clip, label);
    }

    /// `OUTRO_PLAY()`.
    fn play(&mut self, sounds: &mut Vec<TitleSound>) {
        anim::play_with_subtitle(
            &self.lips,
            self.voice_base,
            self.voice_line,
            &mut self.lip,
            sounds,
        );
    }

    /// One frame of the `HistOutro_Run` loop. `voice_stat` is `Voice_GetStat(0)`.
    pub fn run_frame(
        &mut self,
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
        if self.done.is_some() {
            return frame;
        }
        let sounds = &mut frame.sounds;
        self.fade.update();
        self.update();
        self.update_step(pressed, voice_stat, save, sounds);
        self.draw(voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & FLAG_BEGUN == 0
            && self.flash.flags() & FLASH_PAD != 0
        {
            self.flags |= FLAG_BEGUN;
            self.step = 1;
        }
        if self.fade.is_fading_out() {
            sounds.push(TitleSound::VoiceFadeOutStep);
            sounds.push(TitleSound::BgmFadeOutStep);
            sounds.push(TitleSound::StreamSeFadeOutStep(0));
            return frame;
        }
        if self.fade.is_out_done() {
            // The ending movie is due when saga 6 is done while the ending is not listed yet.
            self.done = Some(if self.saga == 6 && save.slot_flags(8) & 1 == 0 {
                HistOutroResult::Ending
            } else {
                HistOutroResult::Save
            });
            return frame;
        }
        if self.flags & FLAG_LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        } else if self.step == 0 {
            self.input(pressed, sounds);
        }
        frame
    }

    /// `HistOutro_Update`: the guides sent in when the movie asks; the closing text's scroll.
    fn update(&mut self) {
        self.flash.advance();
        if self.flags & FLAG_STARTED == 0 && self.flash.trig() & 2 != 0 {
            self.msg.open();
            match self.saga {
                4 => {
                    self.guide_goto(0, false);
                    self.guide_goto(1, false);
                }
                7 => self.guide_goto(1, false),
                _ => self.guide_goto(0, false),
            }
        }
        if self.flags & FLAG_SCROLLING != 0 {
            self.scroll += 0.291_666_67;
            if self.scroll >= self.scroll_max {
                self.scroll = self.scroll_max;
                self.flags &= !FLAG_SCROLLING;
            }
        }
    }

    /// `HistOutro_Input`: once the script has ended, confirm finishes the scroll, then leaves; START leaves.
    fn input(&mut self, pressed: u32, sounds: &mut Vec<TitleSound>) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        self.flags |= FLAG_STARTED;
        if pressed & PAD_CROSS != 0 {
            if self.flags & FLAG_SCROLLING != 0 {
                sounds.push(TitleSound::VoiceStop);
                self.lip.clear();
                self.scroll = self.scroll_max;
                self.flags &= !FLAG_SCROLLING;
            } else {
                self.flags |= FLAG_CHOSEN | FLAG_LEAVING;
                self.timer = 15;
            }
            sounds.push(TitleSound::Effect(1, 1));
        } else if pressed & PAD_START != 0 {
            self.flags |= FLAG_CHOSEN | FLAG_LEAVING;
            self.timer = 15;
            sounds.push(TitleSound::Effect(1, 1));
        }
    }

    /// `HistOutro_UpdateStep`: the dialogue of the saga (step `101 + 50 * saga`), then the closing text.
    fn update_step(
        &mut self,
        pressed: u32,
        voice_stat: i32,
        save: &mut SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        let step = self.step;
        if step == 0 {
            return;
        }
        let say = |s: &mut Self, who: usize, first: bool, sounds: &mut Vec<TitleSound>| {
            s.speaker = who;
            if first {
                s.voice_line = s.line_base;
            } else {
                s.voice_line += 1;
            }
            s.play(sounds);
            s.step += 1;
        };
        // Waits for the line to end; confirm goes on at once (the voice goes on).
        let wait = |s: &mut Self, sounds: &mut Vec<TitleSound>| {
            if voice_stat == VOICE_PLAYEND {
                s.step += 1;
            } else if pressed & PAD_CROSS != 0 {
                s.step += 1;
                sounds.push(TitleSound::Effect(1, 1));
            }
        };
        let walk = |s: &mut Self, side: usize, out: bool| {
            s.guide_goto(side, out);
            s.step += 1;
        };
        match step {
            1 => {
                save.set_slot_flags(self.saga, save.slot_flags(self.saga) | SLOT_OUTRO_STARTED);
                self.step += 1;
                self.voice_line = -1;
            }
            2 => self.step = self.saga as i32 * 50 + 101,
            STEP_CLOSE => {
                self.flash.step_frames(1);
                self.msg.close();
                self.step = STEP_TEXT;
            }
            // The dialogues' first lines.
            101 | 151 | 201 | 251 | 301 | 351 | 401 => say(self, 0, true, sounds),
            451 => say(self, 1, true, sounds),
            // The next lines, by the saga's guide (0) or the partner (1).
            103 | 109 | 115 | 117 | 119 | 153 | 155 | 159 | 163 | 207 | 213 | 221 | 253 | 255
            | 261 | 275 | 305 | 307 | 311 | 313 | 315 | 319 | 357 | 373 | 403 | 405 | 455 | 461 => {
                say(self, 0, false, sounds)
            }
            107 | 111 | 113 | 121 | 123 | 161 | 167 | 169 | 205 | 209 | 211 | 215 | 217 | 259
            | 263 | 265 | 267 | 269 | 271 | 303 | 309 | 317 | 359 | 361 | 363 | 365 | 367 | 369
            | 409 | 411 | 413 | 415 | 417 | 419 | 421 | 457 | 459 | 463 => {
                say(self, 1, false, sounds)
            }
            102 | 104 | 108 | 110 | 112 | 114 | 116 | 118 | 120 | 122 | 124 | 152 | 154 | 156
            | 160 | 162 | 164 | 168 | 170 | 202 | 206 | 208 | 210 | 212 | 214 | 216 | 218 | 222
            | 252 | 254 | 256 | 260 | 262 | 264 | 266 | 268 | 270 | 272 | 276 | 302 | 304 | 306
            | 308 | 310 | 312 | 314 | 316 | 318 | 320 | 352 | 356 | 358 | 360 | 362 | 364 | 366
            | 368 | 370 | 374 | 402 | 404 | 406 | 410 | 412 | 414 | 416 | 418 | 420 | 422 | 452
            | 456 | 458 | 460 | 462 | 464 => wait(self, sounds),
            // The guides walking in and out.
            105 | 157 | 203 | 257 | 353 => walk(self, 1, false),
            165 => walk(self, 0, true),
            219 | 273 | 371 => walk(self, 1, true),
            453 => walk(self, 0, false),
            407 => {
                self.guide_goto(0, true);
                walk(self, 1, false);
            }
            223 | 277 | 375 => {
                self.guide_goto(0, true);
                self.step = STEP_CLOSE;
            }
            125 | 321 | 465 => {
                self.guide_goto(0, true);
                self.guide_goto(1, true);
                self.step = STEP_CLOSE;
            }
            171 | 423 => {
                self.guide_goto(1, true);
                self.step = STEP_CLOSE;
            }
            // A line of the voice set this dialogue leaves out.
            355 => {
                self.voice_line += 1;
                self.step += 2;
            }
            106 | 158 | 166 | 204 | 220 | 258 | 274 | 354 | 372 | 408 | 454 => {
                if self.flash.trig() & 4 != 0 {
                    self.step += 1;
                }
            }
            // The closing text, scrolled in and read out.
            STEP_TEXT => {
                self.scroll = 0.0;
                self.scroll_max = self.descr.count as f32 * 40.0 + 230.0;
                self.narr_wait = 0;
                self.narr_line = 0;
                self.step += 1;
            }
            502 => {
                if self.flash.trig() & 1 != 0 {
                    self.flags |= FLAG_SCROLLING;
                    self.step += 1;
                }
            }
            503 => {
                let delay = self
                    .descr
                    .delay
                    .get(self.narr_line as usize)
                    .copied()
                    .unwrap_or(0);
                let waited = self.narr_wait;
                self.narr_wait += 1;
                if delay <= waited {
                    anim::play_with_subtitle(
                        &[],
                        NARR_VOICE,
                        self.descr.line + self.narr_line,
                        &mut self.lip,
                        sounds,
                    );
                    self.narr_wait = 0;
                    self.step += 1;
                }
            }
            504 => {
                if voice_stat == VOICE_PLAYEND {
                    self.narr_line += 1;
                    if self.descr.count <= self.narr_line {
                        self.step += 1;
                    } else {
                        self.step -= 1;
                    }
                } else if pressed & PAD_START != 0 {
                    self.step = STEP_GO;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            505 => self.step = 0,
            STEP_GO => {
                self.step = 0;
                self.flags |= FLAG_CHOSEN | FLAG_LEAVING;
                self.timer = 15;
            }
            _ => {}
        }
    }

    /// `HistOutro_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        self.bg.draw(quads);
        for i in 0..2 {
            let guide = if i != 0 { self.partner } else { self.saga };
            let name = format!("mc_guide_{guide:02}");
            let f = &mut self.flash;
            let eye = f.find(Some(&name), &format!("{name}_eye"));
            anim::blink(f, eye, &mut self.blink[i], 0, random);
            let mouth = f.find(Some(&name), &format!("{name}_mouth"));
            if self.speaker == i {
                anim::talk(
                    f,
                    mouth,
                    &mut self.talk[i],
                    0,
                    &mut self.lip,
                    voice_stat,
                    random,
                );
            } else {
                anim::show_next(f, mouth, 0);
            }
        }
        let mut texts: Vec<TextDraw> = Vec::new();
        let boxed = TextBox::preset(0).with_align(1);
        for i in 0..3 {
            let clip = self.flash.find(None, &format!("mc_episode_text_{}", i + 1));
            let line = if i >= self.descr.count {
                -1
            } else {
                self.descr.line + i
            };
            texts.extend(boxed.attach_line_at(
                &mut self.flash,
                clip,
                i as u32,
                &self.text,
                line,
                (0, -(self.scroll as i32)),
                font,
            ));
        }
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&flash_quads, textures));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.msg.draw(self.voice_line, font, quads);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode_menu::{ModeMenuAssets, SAGA_FILE, SagaAssets, SagaLayout};

    /// Runs saga 1's real outro from the ISO in `rom/`, when it is there: its dialogue (the guide's voice set from
    /// line 4), the closing text read out, and the first completion's count.
    #[test]
    fn real_outro_plays_the_dialogue_and_the_closing_text() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe, pack_section};

        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let mut read = |id: u32| {
            let location = partition_entry_for_id(id);
            let archive =
                AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition))
                    .unwrap();
            archive.read_entry(&mut image, location.index).unwrap()
        };
        let pack = read(crate::hist_select::ARCHIVE_FILE_ID);
        let res =
            bpe::decode(pack_section(&pack, crate::mode_menu::ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = ModeMenuAssets::parse(&res).unwrap();
        let saga_assets =
            SagaAssets::parse(1, &bpe::decode(&read(SAGA_FILE + 1)).unwrap()).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in assets.files.iter().chain(&saga_assets.files) {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.msg = ModeMenuAssets::msg_layout(|key| firsts[&key]);
        let mut saga = saga_assets.data;
        saga.layout = SagaLayout::new(1, 5, |key| firsts[&key]);
        assert!(saga.layout.outro[face_slots(2)[0]].is_some());
        let mut save = SaveData::default();
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[9]);
        let mut sounds = Vec::new();
        let mut screen =
            HistOutroScreen::new(&data, &saga, &mut save, &mut random, &mut sounds).unwrap();
        assert_ne!(save.slot_flags(1) & SLOT_OUTRO_SEEN, 0);
        let mut voices = Vec::new();
        for _ in 0..3000 {
            let f = screen.run_frame(0, VOICE_PLAYEND, &mut save, &mut random, &mut font);
            voices.extend(f.sounds.iter().filter_map(|s| match s {
                TitleSound::Voice(id) => Some(*id),
                _ => None,
            }));
            if screen.step == 0 && screen.flags & FLAG_BEGUN != 0 && !voices.is_empty() {
                break;
            }
        }
        let alt = if crate::sound::options().alt_voice {
            anim::VOICE_LANG2_OFFSET
        } else {
            0
        };
        // Saga 1's eight lines (steps 151..169), from line 4 of the guide's set, then the closing text's lines.
        let dialogue: Vec<u32> = (0..8).map(|n| 0x8423 + 4 + n + alt).collect();
        assert_eq!(&voices[..8], &dialogue[..]);
        let narration = saga.descr[4];
        assert_eq!(voices.len(), 8 + narration.count as usize);
        assert_eq!(voices[8], NARR_VOICE + narration.line as u32 + alt);
        assert_ne!(save.slot_flags(1) & SLOT_OUTRO_STARTED, 0);
        // Confirm leaves; saga 1 leads to the save screen.
        for _ in 0..200 {
            screen.run_frame(PAD_CROSS, VOICE_PLAYEND, &mut save, &mut random, &mut font);
            if screen.result().is_some() {
                break;
            }
        }
        assert_eq!(screen.result(), Some(HistOutroResult::Save));
    }
}
