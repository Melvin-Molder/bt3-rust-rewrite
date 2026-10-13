//! Survival's course select (`SurvSel_Run(2)`, src/menu/survival_select.c, progress mode 17, `Ub_Main`): the
//! mission list's sibling with three courses, Android 17 as guide, each course's rank letter, and the record of the
//! course under the cursor (opponents beaten, time, score). Confirming a course's window sets up its rule and the
//! fifty opponents (`SurvSel_SetupBattle`); the player's fighter follows the fighter select (mode 18).

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;
use bt3_data::survival::{COURSES, SurvivalData};

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::BattleSetup;
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::ub::VOICE_BASE;
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};

/// `SurvSel_Run(2)`: section 2 of `gMenuArc3` (the mission list's).
pub const ARCHIVE_SECTION: usize = 2;
const FADE_FRAMES: i32 = 0x14;
const IDLE_FRAMES: i32 = 0x708;

// SURVSEL_*.
const CHOSEN: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;

/// `gSurvSel->tex[n]` of `SurvSel_Init`.
const TEX: [ImageTexture; 29] = [
    tex(3, 0),
    tex(3, 1),
    tex(3, 15),
    tex(3, 14),
    tex(3, 18),
    tex(3, 19),
    tex(3, 23),
    tex(3, 20),
    tex(3, 11),
    None,
    None,
    tex(3, 22),
    tex(3, 21),
    tex(3, 6),
    tex(3, 5),
    tex(3, 7),
    tex(3, 13),
    None,
    tex(3, 17),
    tex(3, 16),
    tex(3, 10),
    tex(3, 8),
    tex(3, 12),
    tex(5, 0),
    tex(5, 1),
    tex(5, 3),
    tex(3, 25),
    tex(3, 26),
    tex(3, 27),
];
/// `IconWin_Init(MPACK_AT(res, 7), res 9)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((7, 1), 1)),
    Some(((7, 1), 0)),
    tex(9, 0),
    Some(((7, 1), 2)),
    tex(9, 1),
];

/// What `SurvSel_Init` reads from its unpacked section.
pub struct SurvivalSelectAssets {
    pub data: SurvivalSelectData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct SurvivalSelectData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 18: the screen's text; 17: the guide's mouth keys.
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    /// Sections 12..14: the courses, their opponents and the random characters.
    pub courses: SurvivalData,
    pub layout: SurvivalSelectLayout,
}

#[derive(Clone, Debug, Default)]
pub struct SurvivalSelectLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
}

impl SurvivalSelectLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
        }
    }
}

impl SurvivalSelectAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&TEX[..], &ICON_TEXTURES])?;
        Ok(Self {
            data: SurvivalSelectData {
                fod: pack_section(res, 2)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 7)?, 2)?.to_vec(),
                text: pack_section(res, 18)?.to_vec(),
                lips: pack_section(res, 17)?.to_vec(),
                courses: SurvivalData::parse(res)?,
                layout: SurvivalSelectLayout::default(),
            },
            background: decode_picture(res, 4)?,
            files,
        })
    }
}

/// `SurvSel` and the frame loop of `SurvSel_Run`.
pub struct SurvivalSelectScreen {
    data: SurvivalSelectData,
    flash: Flash,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    cursor: i32,
    timer: i32,
    level: i32,
    voice_req: i32,
    voice_last: i32,
    voice_skip: bool,
    voice_line: i32,
    blink: i32,
    talk: i32,
    idle: i32,
    result: i32,
    done: Option<Option<BattleSetup>>,
}

impl SurvivalSelectScreen {
    /// `SurvSel_Init(2)` and the first `ColorFade_StartIn` (the cursor starts on the first course).
    pub fn new(data: &SurvivalSelectData, libc: &mut LibcRandom) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let blink = libc.rand() % 32;
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        Ok(Self {
            data: data.clone(),
            flash,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            cursor: 0,
            timer: 0,
            level: 0,
            voice_req: 0,
            voice_last: 0,
            voice_skip: false,
            voice_line: -1,
            blink,
            talk: 0,
            idle: 0,
            result: 1,
            done: None,
        })
    }

    /// `SurvSel_Run`'s result once it has returned: the chosen course's battle setup (the player's fighter still to
    /// be added), or none when the player backed out.
    pub fn result(&self) -> Option<&Option<BattleSetup>> {
        self.done.as_ref()
    }

    /// The course to keep in `gProgress->cursor`.
    pub fn cursor(&self) -> i32 {
        self.cursor
    }

    /// One iteration of `SurvSel_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        save: &SaveData,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
        font: &mut Font,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done.is_some() {
            return frame;
        }
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        // SurvSel_Update.
        if self.timer > 0 {
            self.timer -= 1;
        }
        self.flash.advance();
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
            // SurvSel_SetupBattle; the player's fighter comes with Ub_SetupSolo.
            self.done = Some(if self.result != 0 {
                BattleSetup::survival(
                    &self.data.courses,
                    self.cursor.clamp(0, COURSES as i32 - 1) as usize,
                    (0, 0, [0; 8]),
                    random,
                    libc,
                )
            } else {
                None
            });
            return frame;
        }
        if self.flags & LEAVING != 0 {
            if self.timer == 0 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        } else if self.voice_req == 0 {
            self.input(pressed, repeat, &mut frame.sounds);
        }
        frame
    }

    /// `SurvSel_UpdateVoice`: the line the guide was asked for (1 the greeting, 2 the idle line).
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
                    self.voice_skip = true;
                }
                return;
            }
        } else {
            self.voice_skip = false;
        }
        match self.voice_req {
            1 => self.voice_line = 0x34,
            2 => self.voice_line = 0x35,
            _ => {}
        }
        self.voice_req = 0;
        anim::play_with_subtitle(
            &self.data.lips,
            VOICE_BASE,
            self.voice_line,
            &mut self.lip,
            sounds,
        );
        self.voice_last = self.voice_req;
    }

    /// `SurvSel_Draw`: the three courses with their rank letters and the record of the one under the cursor.
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
        for i in 0..COURSES as i32 {
            let name = format!("mc_window_plate{:02}", i + 1);
            let r = f.find(Some(&name), "mc_window_text1_0");
            texts.extend(plain.attach_line(
                f,
                r,
                0x400 + i as u32,
                &self.data.text,
                i + 0x13C,
                font,
            ));
            let rank = i32::from(save.survival(i as usize).rank);
            let r = f.find(Some(&name), "mc_rankmoji");
            f.clip_set_uv(r, uv(rank * 0x20, 0, rank * 0x20 + 0x20, 0x20));
            if i == self.cursor {
                let r = f.find(None, "mc_rank");
                f.clip_set_uv(r, uv(rank * 0x40, 0, rank * 0x40 + 0x40, 0x40));
            }
        }
        for i in COURSES..5 {
            let r = f.find(None, &format!("mc_window_plate{:02}", i + 1));
            f.clip_set_flags(r, PROP_VISIBLE, false);
        }
        let r = f.find(None, "mc_missionkari01");
        texts.extend(plain.attach_line(f, r, 0x403, &self.data.text, 0x150, font));
        let r = f.find(Some("mc_mission_plate01"), "mc_misson_plate_text");
        f.clip_set_uv(r, uv(0, 0x20, 0x100, 0x40));
        let rec = save.survival(self.cursor.max(0) as usize);
        let r = f.find(None, "mc_gekiha_text01");
        f.clip_set_uv(r, uv(0, 0x60, 0x100, 0x80));
        anim::num_draw(
            f,
            |n| format!("mc_gekiha_suji{n:02}"),
            3,
            rec.defeated,
            (0x20, 0x20),
            0,
        );
        for (first, value) in [(4, rec.time[0]), (2, rec.time[1]), (0, rec.time[2])] {
            anim::num_draw(
                f,
                |n| format!("mc_time_suji{:02}", n + first),
                2,
                i32::from(value),
                (0x20, 0x20),
                1,
            );
        }
        let r = f.find(None, "mc_time_text02");
        f.clip_set_uv(r, uv(0, 0x20, 0x100, 0x40));
        anim::num_draw(
            f,
            |n| format!("mc_score_suji{n:02}"),
            9,
            rec.score.wrapping_mul(100),
            (0x20, 0x20),
            0,
        );
        let tex = &self.data.layout.flash;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (fq, overs) = f.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&fq, tex));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.icon.draw(quads);
    }

    /// `SurvSel_ClipGoto(0, 0, label)`.
    fn clip_goto(&mut self, label: &str) {
        let r = self
            .flash
            .find(None, &format!("mc_window_plate{:02}", self.cursor + 1));
        self.flash.clip_goto_label(r, label);
    }

    /// `SurvSel_Input`: level 0 up / down the courses, confirm opens the course's window; level 1 confirm chooses
    /// it, cancel closes it.
    fn input(&mut self, pressed: u32, repeat: u32, sounds: &mut Vec<TitleSound>) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.voice_req = 1;
            self.clip_goto("fl_on_start");
            self.flags |= STARTED;
        }
        let confirm = pressed & PAD_CROSS != 0;
        let cancel = pressed & PAD_TRIANGLE != 0;
        let n = COURSES as i32;
        match self.level {
            0 => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    self.clip_goto("fl_off_start");
                    self.cursor = if repeat & PAD_UP != 0 {
                        if self.cursor - 1 < 0 {
                            n - 1
                        } else {
                            self.cursor - 1
                        }
                    } else if self.cursor + 1 >= n {
                        0
                    } else {
                        self.cursor + 1
                    };
                    self.clip_goto("fl_on_start");
                    sounds.push(TitleSound::Effect(1, 0));
                    self.idle = 0;
                } else if confirm {
                    self.clip_goto("fl_rank_out");
                    self.flash
                        .goto_label(&format!("fl_survival_{:02}_in", self.cursor + 1), true);
                    self.level = 1;
                    sounds.push(TitleSound::Effect(1, 1));
                    self.idle = 0;
                } else if cancel {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.result = 0;
                    self.flash.goto_label("fl_furvival_menu_cancel", true);
                    sounds.push(TitleSound::Effect(1, 2));
                    self.idle = 0;
                } else {
                    self.idle += 1;
                    if self.idle == IDLE_FRAMES {
                        self.voice_req = 2;
                        self.idle = 0;
                    }
                }
            }
            _ => {
                if confirm {
                    self.flags |= CHOSEN | LEAVING;
                    self.timer = 15;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if cancel {
                    self.level = 0;
                    self.clip_goto("fl_rank_in");
                    self.flash
                        .goto_label(&format!("fl_survival_{:02}_cansel", self.cursor + 1), true);
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
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

    /// Runs the real course select from the ISO in `rom/`, when it is there: the guide's greeting, the second
    /// course's window opened and confirmed, and its survival battle set up.
    #[test]
    fn real_survival_select_sets_a_course_up() {
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
        let assets = SurvivalSelectAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = SurvivalSelectLayout::new(0, |k| firsts[&k]);
        let save = SaveData::default();
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[7, 2]);
        let mut screen = SurvivalSelectScreen::new(&data, &mut libc).unwrap();
        let mut font = Font::new(Vec::new(), 20);
        let mut press = |screen: &mut SurvivalSelectScreen, bits: u32| {
            for i in 0..40 {
                let b = if i == 0 { bits } else { 0 };
                screen.run_frame(
                    b,
                    b,
                    VOICE_PLAYEND,
                    &save,
                    &mut random,
                    &mut libc,
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
        assert_eq!(screen.voice_line, 0x34);
        press(&mut screen, PAD_DOWN);
        press(&mut screen, PAD_CROSS);
        assert_eq!(screen.level, 1);
        press(&mut screen, PAD_CROSS);
        for _ in 0..3 {
            press(&mut screen, 0);
        }
        let Some(Some(setup)) = screen.result().cloned() else {
            panic!("no course chosen");
        };
        assert_eq!(screen.cursor(), 1);
        assert_eq!(setup.mode, 3);
        assert_eq!(setup.pool.len(), 50);
        assert_eq!(setup.sides[1].control, 2);
        assert_eq!(setup.sides[1].chara, setup.pool[0].chara);
    }
}
