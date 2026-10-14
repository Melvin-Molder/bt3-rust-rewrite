//! Ultimate Battle Z's course select (`UbzSel_Run`, src/menu/ubz_select.c, progress mode 28, `Ub_Main`): five
//! courses of up to eight opponents fought in a row, each plate with the course's best rank letter, and the chosen
//! course's sheet (its opponents' chips, best time and score). Confirming the sheet sets up the course's rule and
//! opponents (`UbzSel_SetupBattle`); the player's fighter follows the fighter select (mode 29).

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{BattleSetup, PoolMember, SideSetup};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::ub::VOICE_BASE;
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};

/// `UbzSel_Init`'s file: `gProgress->baseFile + 0x1C`.
pub const FILE_ID: u32 = 0x1C1 + 0x1C;
const FADE_FRAMES: i32 = 0x14;
const IDLE_FRAMES: i32 = 0x708;
/// `N_COURSE_NUM`, `N_CHIP_NUM`, `N_NONE`, `N_RANDOM`, `N_BGM_RANDOM`.
pub const COURSE_NUM: usize = 5;
const CHIP_NUM: usize = 165;
pub(crate) const NONE: i32 = 999;
pub(crate) const RANDOM: i32 = 998;
pub(crate) const BGM_RANDOM: i32 = 0x18;
/// `sUbzFoeTex`: the movie's images of the course's opponents.
const FOE_TEX: [usize; 8] = [15, 18, 19, 20, 21, 22, 23, 24];

// UBZSEL_*.
const CHOSEN: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const FADED_IN: u32 = 8;

/// `gUbzSel->tex[n]` of `UbzSel_Init`.
const TEX: [ImageTexture; 38] = [
    tex(3, 0),
    tex(3, 1),
    tex(3, 13),
    tex(3, 12),
    tex(3, 16),
    tex(3, 17),
    tex(3, 21),
    tex(3, 18),
    tex(3, 9),
    None,
    tex(3, 20),
    tex(3, 19),
    tex(3, 5),
    tex(3, 4),
    tex(3, 6),
    None,
    tex(3, 2),
    tex(3, 3),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    tex(3, 11),
    None,
    tex(3, 15),
    tex(3, 14),
    tex(3, 8),
    tex(3, 7),
    tex(3, 10),
    tex(5, 0),
    tex(5, 1),
    tex(5, 3),
    tex(3, 22),
    tex(3, 23),
    tex(3, 24),
];
/// `IconWin_Init(MPACK_AT(res, 7), res 2)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((7, 1), 1)),
    Some(((7, 1), 0)),
    tex(2, 0),
    Some(((7, 1), 2)),
    tex(2, 1),
];

fn chip_textures() -> Vec<ImageTexture> {
    (1..=CHIP_NUM).map(|n| Some(((6, n), 0))).collect()
}

/// One opponent (`NFoe`, 0x2C bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Foe {
    /// `N_NONE` ends a course's list.
    pub chara: i32,
    pub costume: i32,
    pub cpu_level: i32,
    /// Item id - 1 of the last slot, `N_NONE` none.
    pub last_item: i32,
    /// Item ids - 1, `N_NONE` skipped (the rest move up).
    pub item: [i32; 7],
}

impl Foe {
    /// The 1-based item set of the battle setup.
    pub(crate) fn items(&self) -> [u16; 8] {
        let mut items = [0u16; 8];
        let mut k = 0;
        for &id in &self.item {
            if id != NONE {
                items[k] = (id + 1) as u16;
                k += 1;
            }
        }
        items[7] = if self.last_item == NONE {
            0
        } else {
            (self.last_item + 1) as u16
        };
        items
    }
}

pub(crate) fn words(data: &[u8], at: usize, n: usize) -> Option<Vec<i32>> {
    (0..n)
        .map(|k| {
            data.get(at + k * 4..at + k * 4 + 4)
                .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
        })
        .collect()
}

/// `count` opponents of a table.
pub(crate) fn parse_foes(data: &[u8], count: usize) -> Result<Vec<Foe>, Box<dyn Error>> {
    (0..count)
        .map(|n| {
            let w = words(data, n * 0x2C, 11).ok_or("short opponent table")?;
            Ok(Foe {
                chara: w[0],
                costume: w[1],
                cpu_level: w[2],
                last_item: w[3],
                item: [w[4], w[5], w[6], w[7], w[8], w[9], w[10]],
            })
        })
        .collect()
}

/// The rules of one course (`NCourse`, 0x38 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Course {
    pub announcer: i32,
    pub stage_change: i32,
    pub time_limit: i32,
    pub stage: i32,
    pub bgm: i32,
    pub change_allowed: i32,
    /// Indices into the opponents.
    pub foe: [i32; 8],
}

/// What `UbzSel_Init` reads from its file.
pub struct UbzSelectAssets {
    pub data: UbzSelectData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct UbzSelectData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 10: the course names.
    pub text: Vec<u8>,
    pub courses: Vec<Course>,
    pub foes: Vec<Foe>,
    pub layout: UbzSelectLayout,
}

#[derive(Clone, Debug, Default)]
pub struct UbzSelectLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub chips: Vec<Option<usize>>,
}

impl UbzSelectLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            chips: image_indices(&chip_textures(), &first),
        }
    }
}

impl UbzSelectAssets {
    /// From the unpacked file.
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let chips = chip_textures();
        let files = decode_files(res, [&TEX[..], &ICON_TEXTURES, &chips])?;
        let table = pack_section(res, 8)?;
        let courses = (0..COURSE_NUM)
            .map(|n| {
                let w = words(table, n * 0x38, 14).ok_or("short course table")?;
                Ok(Course {
                    announcer: w[0],
                    stage_change: w[1],
                    time_limit: w[2],
                    stage: w[3],
                    bgm: w[4],
                    change_allowed: w[5],
                    foe: std::array::from_fn(|k| w[6 + k]),
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        let count = courses
            .iter()
            .flat_map(|c| c.foe)
            .filter(|&f| f >= 0)
            .max()
            .map_or(0, |m| m as usize + 1);
        Ok(Self {
            data: UbzSelectData {
                fod: pack_section(res, 1)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 7)?, 2)?.to_vec(),
                text: pack_section(res, 10)?.to_vec(),
                courses,
                foes: parse_foes(pack_section(res, 9)?, count)?,
                layout: UbzSelectLayout::default(),
            },
            background: decode_picture(res, 4)?,
            files,
        })
    }
}

/// `UbzSel` and the frame loop of `UbzSel_Run`.
pub struct UbzSelectScreen {
    data: UbzSelectData,
    flash: Flash,
    tex: Vec<Option<usize>>,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    cursor: i32,
    timer: i32,
    step: i32,
    voice_req: i32,
    voice_prev: i32,
    voice_skip: bool,
    voice_line: i32,
    blink: i32,
    talk: i32,
    idle: i32,
    result: i32,
    done: Option<Option<BattleSetup>>,
}

impl UbzSelectScreen {
    /// `UbzSel_Init` and the first `ColorFade_StartIn`.
    pub fn new(data: &UbzSelectData, libc: &mut LibcRandom) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let blink = libc.rand() % 32;
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        Ok(Self {
            data: data.clone(),
            flash,
            tex: data.layout.flash.clone(),
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            cursor: 0,
            timer: 0,
            step: 0,
            voice_req: 0,
            voice_prev: 0,
            voice_skip: false,
            voice_line: -1,
            blink,
            talk: 0,
            idle: 0,
            result: 1,
            done: None,
        })
    }

    /// `UbzSel_Run`'s result once it has returned: the chosen course's setup (the player's fighter still to be
    /// added), or none when the player backed out.
    pub fn result(&self) -> Option<&Option<BattleSetup>> {
        self.done.as_ref()
    }

    /// The course chosen (`gProgress->ubChoice`).
    pub fn choice(&self) -> i32 {
        self.cursor
    }

    fn course(&self) -> Course {
        self.data.courses[self.cursor.clamp(0, COURSE_NUM as i32 - 1) as usize]
    }

    fn foe(&self, i: i32) -> Foe {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.data.foes.get(i).copied())
            .unwrap_or(Foe {
                chara: NONE,
                ..Foe::default()
            })
    }

    /// `UbzSel_SetupBattle`: the course's rule (battle mode 3) and its opponents as the pool of side 1.
    fn setup_battle(&self, random: &mut GameRandom, libc: &mut LibcRandom) -> BattleSetup {
        let course = self.course();
        let announcer = if course.announcer == RANDOM {
            random.range(8) as i32
        } else {
            course.announcer
        };
        let stage = if course.stage == RANDOM {
            random.range(0x23) as i32
        } else {
            course.stage
        };
        let bgm = if course.bgm == RANDOM {
            BGM_RANDOM
        } else {
            course.bgm
        };
        // BattleSetup_SetRule: music 24 is a random one of 0..23.
        let bgm = if bgm == 24 { libc.rand() % 24 } else { bgm };
        let foes: Vec<Foe> = course
            .foe
            .iter()
            .map(|&f| self.foe(f))
            .take_while(|f| f.chara != NONE)
            .collect();
        let first = foes.first().copied().unwrap_or_default();
        let side = |control, pad, change_allowed| SideSetup {
            control,
            pad,
            change_allowed,
            switch_enabled: 1,
            ..SideSetup::default()
        };
        let mut sides = [
            side(0, 0, 1),
            side(2, 1, i32::from(course.change_allowed != 0)),
        ];
        sides[1].chara = first.chara;
        sides[1].cpu_level = first.cpu_level;
        BattleSetup {
            wide: false,
            screen_mode: 0,
            mode: 3,
            bgm,
            time_limit: course.time_limit,
            announcer,
            stage,
            stage_change: i32::from(course.stage_change != 0),
            sides,
            pool: foes
                .iter()
                .map(|f| PoolMember {
                    chara: f.chara,
                    costume: f.costume,
                    cpu_level: f.cpu_level,
                    items: f.items(),
                })
                .collect(),
            tutorial: 0,
        }
    }

    /// One iteration of `UbzSel_Run`'s loop.
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
        // UbzSel_Update.
        if self.timer > 0 {
            self.timer -= 1;
        }
        self.flash.advance();
        self.update_voice(pressed, voice_stat, &mut frame.sounds);
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
            self.done = Some((self.result != 0).then(|| self.setup_battle(random, libc)));
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

    /// `UbzSel_UpdateVoice`: the line the guide was asked for (played without a subtitle table).
    fn update_voice(&mut self, pressed: u32, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if !self.voice_skip {
            if self.voice_req == 0 {
                return;
            }
            if self.voice_line != -1
                && voice_stat != VOICE_PLAYEND
                && self.voice_req == self.voice_prev
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
            1 => self.voice_line = 0x33,
            2 => self.voice_line = 0x35,
            _ => {}
        }
        self.voice_req = 0;
        anim::play_with_subtitle(&[], VOICE_BASE, self.voice_line, &mut self.lip, sounds);
        self.voice_prev = self.voice_req;
    }

    /// `UbzSel_Draw`: the five course plates with their best rank, and the chosen course's sheet.
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
        let course = self.course();
        let charas: Vec<i32> = course.foe.iter().map(|&f| self.foe(f).chara).collect();
        for (i, &image) in FOE_TEX.iter().enumerate() {
            let sub = if charas[i] == NONE {
                CHIP_NUM
            } else {
                (charas[i] + 1).max(1) as usize
            };
            self.tex[image] = self.data.layout.chips.get(sub - 1).copied().flatten();
        }
        let f = &mut self.flash;
        let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_eye");
        anim::blink(f, r, &mut self.blink, 0, random);
        let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_mouth");
        anim::talk(f, r, &mut self.talk, 0, &mut self.lip, voice_stat, random);
        for i in 0..COURSE_NUM {
            let name = format!("mc_window_plate{:02}", i + 1);
            let r = f.find(Some(&name), "mc_window_text1_0");
            texts.extend(plain.attach_line(
                f,
                r,
                0x400 + i as u32,
                &self.data.text,
                i as i32 + 0x143,
                font,
            ));
            let rank = i32::from(save.course(i).rank);
            let r = f.find(Some(&name), "mc_rankmoji");
            f.clip_set_uv(r, uv(rank * 0x20, 0, rank * 0x20 + 0x20, 0x20));
        }
        let r = f.find(Some("mc_mission_plate01"), "mc_mission_plate_text");
        f.clip_set_uv(r, uv(0, 0x20, 0x100, 0x40));
        let rec = save.course(self.cursor.max(0) as usize);
        let rank = i32::from(rec.rank);
        let r = f.find(None, "mc_rank");
        f.clip_set_uv(r, uv(rank * 0x40, 0, rank * 0x40 + 0x40, 0x40));
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
        let tex = &self.tex;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (fq, overs) = f.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&fq, tex));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.icon.draw(quads);
    }

    /// `UbzSel_ClipGoto(0, 0, label)`.
    fn clip_goto(&mut self, label: &str) {
        let r = self
            .flash
            .find(None, &format!("mc_window_plate{:02}", self.cursor + 1));
        self.flash.clip_goto_label(r, label);
    }

    /// `UbzSel_Input`: step 0 up / down the courses, confirm opens the course's sheet; step 1 confirm chooses it,
    /// cancel closes it.
    fn input(&mut self, pressed: u32, repeat: u32, sounds: &mut Vec<TitleSound>) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.clip_goto("fl_on_start");
            self.flags |= STARTED;
        }
        let ok = pressed & PAD_CROSS != 0;
        let cancel = pressed & PAD_TRIANGLE != 0;
        let n = COURSE_NUM as i32;
        match self.step {
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
                } else if ok {
                    self.clip_goto("fl_rank_out");
                    self.flash
                        .goto_label(&format!("fl_menu{}_in", self.cursor), true);
                    self.step = 1;
                    sounds.push(TitleSound::Effect(1, 1));
                    self.idle = 0;
                } else if cancel {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.result = 0;
                    self.flash.goto_label("fl_menu_cansel", true);
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
                if ok {
                    self.flags |= CHOSEN | LEAVING;
                    self.timer = 15;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if cancel {
                    self.step = 0;
                    self.clip_goto("fl_rank_in");
                    self.flash
                        .goto_label(&format!("fl_menu{}_cansel", self.cursor), true);
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

    /// Runs the real course select from the ISO in `rom/`, when it is there: the third course's sheet opened and
    /// confirmed, and its opponents set up as the pool of a mode 3 battle.
    #[test]
    fn real_ubz_select_sets_a_course_up() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let file = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(&file).unwrap();
        let assets = UbzSelectAssets::parse(&res).unwrap();
        for c in &assets.data.courses {
            assert!(
                (0..=0xA0).contains(&assets.data.foes[c.foe[0] as usize].chara),
                "{c:?}"
            );
        }
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = UbzSelectLayout::new(0, |k| firsts[&k]);
        let save = SaveData::default();
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[2, 8]);
        let mut screen = UbzSelectScreen::new(&data, &mut libc).unwrap();
        let mut font = Font::new(Vec::new(), 20);
        let mut press = |screen: &mut UbzSelectScreen, bits: u32| {
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
        press(&mut screen, PAD_DOWN);
        press(&mut screen, PAD_DOWN);
        press(&mut screen, PAD_CROSS);
        assert_eq!(screen.step, 1);
        press(&mut screen, PAD_CROSS);
        for _ in 0..3 {
            press(&mut screen, 0);
        }
        let Some(Some(setup)) = screen.result().cloned() else {
            panic!("no course chosen");
        };
        assert_eq!(screen.choice(), 2);
        assert_eq!(setup.mode, 3);
        let course = data.courses[2];
        let count = course
            .foe
            .iter()
            .take_while(|&&f| data.foes[f as usize].chara != NONE)
            .count();
        assert_eq!(setup.pool.len(), count);
        assert_eq!(
            setup.sides[1].chara,
            data.foes[course.foe[0] as usize].chara
        );
    }
}
