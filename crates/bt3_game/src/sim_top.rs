//! Sim Dragon's entry screen (`SimTop_Run(5)`, src/menu/sim_top.c, progress mode 20, `Ub_Main`): Android 18 as
//! guide and three plates: start a run (the run's state in `gProgress` reset), the ranking of the ten best runs
//! (portrait, score and the star of a finished ladder), and the eleven pages of how to play.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound,
};
use crate::ub::VOICE_BASE;
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};

/// `SimTop_Run(5)`: section 5 of `gMenuArc3`.
pub const ARCHIVE_SECTION: usize = 5;
const FADE_FRAMES: i32 = 0x14;
const ROWS: i32 = 3;
const RANK_TOP_MAX: i32 = 5;
const HELP_PAGES: i32 = 10;
const IDLE_FRAMES: i32 = 0xE10;
const FACE_NUM: usize = 165;
/// `gSimTopFaceTex` and `SIMTOP_TEX_CURSOR_FACE`: the movie's images of the ranking rows' portraits.
const FACE_TEX: [usize; 5] = [20, 22, 23, 24, 25];
const CURSOR_FACE: usize = 26;
/// `gSimTopPlateVoice`, `gSimTopIdleVoice`.
const PLATE_VOICE: [i32; 3] = [1, 2, 3];
const IDLE_VOICE: [i32; 2] = [4, 5];

// SIMTOP_*.
const CHOSEN: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;

// SIMTOP_LV_*.
const LV_MENU: i32 = 0;
const LV_RANKING: i32 = 1;
const LV_HELP: i32 = 2;

/// `gSimTop->tex[n]` of `SimTop_Init`.
const TEX: [ImageTexture; 40] = [
    tex(2, 0),
    tex(2, 1),
    tex(2, 14),
    tex(2, 13),
    tex(2, 21),
    tex(2, 22),
    tex(2, 23),
    tex(2, 15),
    tex(2, 18),
    tex(2, 17),
    tex(2, 16),
    tex(2, 2),
    tex(2, 19),
    tex(2, 11),
    tex(2, 12),
    tex(2, 5),
    tex(2, 4),
    tex(2, 20),
    tex(2, 3),
    tex(2, 9),
    None,
    tex(2, 10),
    None,
    None,
    None,
    None,
    None,
    tex(2, 7),
    tex(2, 6),
    tex(2, 24),
    tex(2, 8),
    None,
    tex(5, 5),
    tex(5, 6),
    tex(5, 8),
    tex(2, 25),
    tex(2, 26),
    tex(2, 27),
    tex(2, 28),
    tex(2, 29),
];
/// `IconWin_Init(MPACK_AT(res, 6), res 3)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((6, 1), 1)),
    Some(((6, 1), 0)),
    tex(3, 0),
    Some(((6, 1), 2)),
    tex(3, 1),
];

fn face_textures() -> Vec<ImageTexture> {
    (1..=FACE_NUM).map(|n| Some(((7, n), 0))).collect()
}

/// The state of a Sim Dragon run kept in `gProgress` (`SimRun`, 0x64C..0x684).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimRun {
    /// 0..6: the stars shown; caps attack and defence.
    pub level: i32,
    /// Attack, defence, health in per cent, points / 100.
    pub stat: [i32; 4],
    /// Item ids carried into the fight, -1 none (a ring from `item_head`).
    pub item: [i32; 3],
    pub item_head: i32,
    /// Turns played: / 10 the round, % 10 the turn of the round.
    pub turn: i32,
    /// Turns until button 3 of the board is open again.
    pub wait: i32,
    /// Bit n: button n of the board is greyed out.
    pub off: i32,
}

impl Default for SimRun {
    /// `SimTop_Init`'s new run: level 0, health 100 %, no items, turn 0.
    fn default() -> Self {
        Self {
            level: 0,
            stat: [0, 0, 100, 0],
            item: [-1; 3],
            item_head: 0,
            turn: 0,
            wait: 0,
            off: 0,
        }
    }
}

/// What `SimTop_Init` reads from its unpacked section.
pub struct SimTopAssets {
    pub data: SimTopData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct SimTopData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 8: the descriptions; 9: the guide's mouth keys.
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    pub layout: SimTopLayout,
}

#[derive(Clone, Debug, Default)]
pub struct SimTopLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub faces: Vec<Option<usize>>,
}

impl SimTopLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            faces: image_indices(&face_textures(), &first),
        }
    }
}

impl SimTopAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let faces = face_textures();
        let files = decode_files(res, [&TEX[..], &ICON_TEXTURES, &faces])?;
        Ok(Self {
            data: SimTopData {
                fod: pack_section(res, 1)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 6)?, 2)?.to_vec(),
                text: pack_section(res, 8)?.to_vec(),
                lips: pack_section(res, 9)?.to_vec(),
                layout: SimTopLayout::default(),
            },
            background: decode_picture(res, 4)?,
            files,
        })
    }
}

/// `SimTop` and the frame loop of `SimTop_Run`.
pub struct SimTopScreen {
    data: SimTopData,
    flash: Flash,
    tex: Vec<Option<usize>>,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    cursor_plate: i32,
    timer: i32,
    level: i32,
    voice_req: i32,
    voice_last: i32,
    voice_skip: bool,
    voice_line: i32,
    blink: i32,
    mouth: i32,
    idle: i32,
    star_timer: i32,
    star_frame: i32,
    top: i32,
    cursor: i32,
    page: i32,
    icon_frame: i32,
    result: i32,
    done: Option<i32>,
}

impl SimTopScreen {
    /// `SimTop_Init(5)` (whose reset of the run, the team size and the DP rule the caller makes) and the first
    /// `ColorFade_StartIn`.
    pub fn new(data: &SimTopData) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
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
            cursor_plate: 0,
            timer: 0,
            level: LV_MENU,
            voice_req: 0,
            voice_last: 0,
            voice_skip: false,
            voice_line: -1,
            blink: 0,
            mouth: 0,
            idle: 0,
            star_timer: 0,
            star_frame: 0,
            top: 0,
            cursor: 0,
            page: 0,
            icon_frame: 0,
            result: 1,
            done: None,
        })
    }

    /// `SimTop_Run`'s result once it has returned: 1 a run starts, 0 backed out.
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// One iteration of `SimTop_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
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
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        // SimTop_Update.
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
            self.done = Some(self.result);
            return frame;
        }
        if self.flags & LEAVING != 0 {
            if self.timer == 0 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        } else if self.voice_req == 0 {
            self.input(pressed, repeat, random, &mut frame.sounds);
        }
        frame
    }

    /// `SimTop_UpdateVoice`: lines 0x11..0x15.
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
        if (1..=5).contains(&self.voice_req) {
            self.voice_line = 0x10 + self.voice_req;
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

    /// `SimTop_Draw`: the ranking rows, their portraits, the description.
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
        let face = |chara: i32| {
            usize::try_from(chara)
                .ok()
                .and_then(|c| self.data.layout.faces.get(c).copied())
                .flatten()
        };
        // The six plates: five rows and the one that scrolls in.
        let rows: Vec<(i32, crate::save::SimRank)> = (0..6)
            .map(|i| {
                let row = if i != 5 { self.top + i } else { self.cursor };
                (row, save.sim_rank(row.max(0) as usize))
            })
            .collect();
        for (i, (_, rec)) in rows.iter().enumerate() {
            let image = if i != 5 { FACE_TEX[i] } else { CURSOR_FACE };
            self.tex[image] = face(rec.chara);
        }
        let mut texts: Vec<TextDraw> = Vec::new();
        let f = &mut self.flash;
        let r = f.find(Some("mc_guide_18go"), "mc_guide_18go_eye");
        anim::blink(f, r, &mut self.blink, 0, random);
        let r = f.find(Some("mc_guide_18go"), "mc_guide_18go_mouth");
        anim::talk(f, r, &mut self.mouth, 0, &mut self.lip, voice_stat, random);
        let x = self.icon_frame << 6;
        let r = f.find(None, "mc_icon_play1");
        f.clip_set_uv(r, uv(x, 0, x + 0x40, 0x40));
        self.icon_frame ^= 1;
        for i in 1..3 {
            let name = format!("mc_sim_plate{:02}", i + 1);
            for part in ["mc_friend_text_on", "mc_friend_text_off"] {
                let r = f.find(Some(&name), part);
                f.clip_set_uv(r, uv(0, i << 5, 0x200, (i << 5) + 0x20));
            }
        }
        let r = f.find(None, "mc_menu_yajirusi_up");
        f.clip_set_uv(r, uv(0, 0x20, 0x20, 0x40));
        f.clip_set_flags(r, PROP_VISIBLE, self.top != 0);
        let r = f.find(None, "mc_menu_yajirusi_down");
        f.clip_set_uv(r, uv(0x20, 0x20, 0x40, 0x40));
        f.clip_set_flags(r, PROP_VISIBLE, self.top != 5);
        for (i, (row, rec)) in rows.iter().enumerate() {
            let name = format!("mc_senreki_plate{i:02}");
            let n = row + 1;
            let cell = uv(
                (n % 4) << 6,
                (n / 4) << 6,
                ((n % 4) << 6) + 0x40,
                ((n / 4) << 6) + 0x40,
            );
            for part in ["mc_jyuni_suji", "mc_jyuni_suji_ef"] {
                let r = f.find(Some(&name), part);
                f.clip_set_uv(r, cell);
            }
            anim::num_draw_child(
                f,
                |k| (name.clone(), format!("mc_pt_suji{k:02}")),
                9,
                rec.score.wrapping_mul(100),
                (0x20, 0x20),
                0,
            );
            // SimTop_ShowStar.
            let r = f.find(Some(&name), "mc_icon_star");
            f.clip_set_flags(r, PROP_VISIBLE, rec.cleared != 0);
            anim::sheet(
                f,
                r,
                &mut self.star_timer,
                &mut self.star_frame,
                uv(0, 0, 0x20, 0x20),
                2,
                2,
                0x80,
            );
        }
        let r = f.find(None, "mc_setumeikari");
        let description = TextBox::preset(0).with_align(0).with_spacing(0, 4);
        texts.extend(description.attach_line(f, r, 0x400, &self.data.text, self.page, font));
        let r = f.find(None, "mc_episode_next_2");
        f.clip_set_flags(r, PROP_VISIBLE, self.page < 10);
        let r = f.find(None, "mc_episode_next");
        f.clip_set_flags(r, PROP_VISIBLE, false);
        let tex = &self.tex;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (fq, overs) = f.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&fq, tex));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.icon.draw(quads);
    }

    /// `SimTop_ClipGoto(0, 0, label)`.
    fn clip_goto(&mut self, label: &str) {
        let r = self
            .flash
            .find(None, &format!("mc_sim_plate{:02}", self.cursor_plate + 1));
        self.flash.clip_goto_label(r, label);
    }

    /// `SimTop_Input`: the plates; the ranking scrolls; the how-to-play pages turn.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.clip_goto("fl_on_start");
            self.voice_req = 1;
            self.flags |= STARTED;
        }
        let up = repeat & PAD_UP != 0;
        let down = repeat & PAD_DOWN != 0;
        let right = repeat & PAD_RIGHT != 0;
        let confirm = pressed & PAD_CROSS != 0;
        let cancel = pressed & PAD_TRIANGLE != 0;
        match self.level {
            LV_MENU => {
                if up || down {
                    self.clip_goto("fl_off_start");
                    self.cursor_plate = if up {
                        if self.cursor_plate - 1 < 0 {
                            ROWS - 1
                        } else {
                            self.cursor_plate - 1
                        }
                    } else if self.cursor_plate + 1 >= ROWS {
                        0
                    } else {
                        self.cursor_plate + 1
                    };
                    self.clip_goto("fl_on_start");
                    sounds.push(TitleSound::Effect(1, 0));
                    self.voice_req = PLATE_VOICE[self.cursor_plate as usize];
                    self.idle = 0;
                } else if confirm {
                    match self.cursor_plate {
                        0 => {
                            self.flags |= CHOSEN | LEAVING;
                            self.timer = 15;
                        }
                        1 => {
                            self.level = LV_RANKING;
                            self.flash.goto_label("fl_senreki_plate_in", true);
                        }
                        _ => {
                            self.level = LV_HELP;
                            self.page = 0;
                            self.flash.goto_label("fl_asobikata_plate_in", true);
                        }
                    }
                    sounds.push(TitleSound::VoiceStop);
                    self.lip = LipSync::default();
                    sounds.push(TitleSound::Effect(1, 1));
                    self.idle = 0;
                } else if cancel {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.result = 0;
                    self.flash.goto_label("fl_sim_plate_cansel", true);
                    sounds.push(TitleSound::Effect(1, 2));
                } else {
                    self.idle += 1;
                    if self.idle == IDLE_FRAMES {
                        self.idle = 0;
                        self.voice_req = IDLE_VOICE[random.range(2) as usize];
                    }
                }
            }
            LV_RANKING => {
                if up && self.top != 0 {
                    self.flash.goto_label("fl_senreki_plate_down", true);
                    self.top -= 1;
                    self.cursor = self.top + 5;
                    sounds.push(TitleSound::Effect(1, 0));
                } else if down && (self.top as u32) < RANK_TOP_MAX as u32 {
                    self.flash.goto_label("fl_senreki_plate_up", true);
                    self.top += 1;
                    self.cursor = self.top - 1;
                    sounds.push(TitleSound::Effect(1, 0));
                } else if cancel {
                    self.level = LV_MENU;
                    self.flash.goto_label("fl_senreki_plate_cansel", true);
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            _ => {
                if confirm || right {
                    if self.page < HELP_PAGES {
                        self.page += 1;
                    } else {
                        self.level = LV_MENU;
                        self.flash.goto_label("fl_asobikata_plate_cansel", true);
                    }
                    sounds.push(TitleSound::Effect(1, 1));
                } else if cancel {
                    self.level = LV_MENU;
                    self.flash.goto_label("fl_asobikata_plate_cansel", true);
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

    /// Runs the real entry screen from the ISO in `rom/`, when it is there: the how-to-play pages turned to the
    /// last and closed, then the start plate chosen.
    #[test]
    fn real_sim_top_starts_a_run() {
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
        let assets = SimTopAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = SimTopLayout::new(0, |k| firsts[&k]);
        let save = SaveData::default();
        let mut random = GameRandom::from_key(&[5, 5]);
        let mut screen = SimTopScreen::new(&data).unwrap();
        let mut font = Font::new(Vec::new(), 20);
        let mut press = |screen: &mut SimTopScreen, bits: u32| {
            for i in 0..30 {
                let b = if i == 0 { bits } else { 0 };
                screen.run_frame(b, b, VOICE_PLAYEND, &save, &mut random, &mut font);
                if screen.result().is_some() {
                    break;
                }
            }
        };
        for _ in 0..4 {
            press(&mut screen, 0);
        }
        assert_eq!(screen.voice_line, 0x11);
        press(&mut screen, PAD_UP);
        assert_eq!(screen.cursor_plate, 2);
        press(&mut screen, PAD_CROSS);
        assert_eq!(screen.level, LV_HELP);
        for _ in 0..11 {
            press(&mut screen, PAD_RIGHT);
        }
        assert_eq!(screen.level, LV_MENU);
        assert_eq!(screen.page, HELP_PAGES);
        press(&mut screen, PAD_DOWN);
        assert_eq!(screen.cursor_plate, 0);
        press(&mut screen, PAD_CROSS);
        for _ in 0..3 {
            press(&mut screen, 0);
        }
        assert_eq!(screen.result(), Some(1));
    }
}
