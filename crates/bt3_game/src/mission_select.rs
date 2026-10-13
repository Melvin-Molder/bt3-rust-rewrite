//! Mission 100's list (`MisSel_Run(2)`, src/menu/mission_select.c, progress mode 14, `Ub_Main`): Android 17 as
//! guide, pages of five missions with their number, name and rank letter, and the window of the chosen mission
//! (its kind of team, its DP limit, the opponents' chips, the best time and score). Confirming the window sets the
//! mission's rule and opponents up (`MisSel_SetupBattle`); the player's side follows the fighter select.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{BattleSetup, MemberStart, SideSetup};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::ub::VOICE_BASE;
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};

/// `MisSel_Run(2)`: section 2 of `gMenuArc3`.
pub const ARCHIVE_SECTION: usize = 2;
const FADE_FRAMES: i32 = 0x14;
const ROWS: i32 = 5;
const CHIP_NUM: usize = 165;
const IDLE_FRAMES: i32 = 0x708;
/// `MISSEL_NONE` and `MISSEL_RANDOM` of the mission tables.
const NONE: i32 = 0x3E7;
const RANDOM: i32 = 0x3E6;
/// `gMisSelFaceTex`: the movie's images of the five opponents' chips.
const FACE_TEX: [usize; 5] = [19, 22, 23, 24, 25];

// MISSEL_*.
const CHOSEN: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;

/// `gMisSel->tex[n]` of `MisSel_Init`.
const TEX: [ImageTexture; 39] = [
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
    tex(3, 2),
    tex(3, 16),
    tex(3, 9),
    tex(3, 6),
    tex(3, 5),
    tex(3, 7),
    None,
    tex(3, 3),
    tex(3, 4),
    None,
    None,
    None,
    None,
    tex(3, 13),
    None,
    tex(3, 24),
    tex(3, 17),
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
/// `IconWin_Init(MPACK_AT(res, 7), res 8)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((7, 1), 1)),
    Some(((7, 1), 0)),
    tex(8, 0),
    Some(((7, 1), 2)),
    tex(8, 1),
];

fn chip_textures() -> Vec<ImageTexture> {
    (1..=CHIP_NUM).map(|n| Some(((15, n), 0))).collect()
}

/// Section 10: one mission (`MisSelDef`, 0x34 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MissionDef {
    pub announcer: i32,
    pub stage_change: i32,
    pub time_limit: i32,
    pub stage: i32,
    pub bgm: i32,
    pub change_allowed: i32,
    /// 0..10: the kind of team (text line 0x148 + kind), which decides the player's team size.
    pub kind: i32,
    /// 1..3: the DP limit (text line 0x156 + dpRule).
    pub dp_rule: i32,
    /// Indices into the opponents.
    pub opp: [i32; 5],
}

/// Section 11: one opponent (`MisSelOpp`, 0x2C bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MissionOpp {
    /// `MISSEL_NONE` ends the team.
    pub chara: i32,
    pub costume: i32,
    pub cpu_level: i32,
    /// 0-based id of the item of the last slot, `MISSEL_NONE` none.
    pub item7: i32,
    /// 0-based ids packed into the first slots, `MISSEL_NONE` none.
    pub item: [i32; 7],
}

/// What `MisSel_Init` reads from its unpacked section.
pub struct MissionSelectAssets {
    pub data: MissionSelectData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct MissionSelectData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 18: the screen's text; 17: the guide's mouth keys.
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    pub missions: Vec<MissionDef>,
    pub opps: Vec<MissionOpp>,
    pub layout: MissionSelectLayout,
}

#[derive(Clone, Debug, Default)]
pub struct MissionSelectLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub chips: Vec<Option<usize>>,
}

impl MissionSelectLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            chips: image_indices(&chip_textures(), &first),
        }
    }
}

fn words(data: &[u8], at: usize, n: usize) -> Option<Vec<i32>> {
    (0..n)
        .map(|k| {
            data.get(at + k * 4..at + k * 4 + 4)
                .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
        })
        .collect()
}

impl MissionSelectAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let chips = chip_textures();
        let files = decode_files(res, [&TEX[..], &ICON_TEXTURES, &chips])?;
        let defs = pack_section(res, 10)?;
        let missions = (0..100)
            .map(|n| {
                let w = words(defs, n * 0x34, 13).ok_or("short mission table")?;
                Ok(MissionDef {
                    announcer: w[0],
                    stage_change: w[1],
                    time_limit: w[2],
                    stage: w[3],
                    bgm: w[4],
                    change_allowed: w[5],
                    kind: w[6],
                    dp_rule: w[7],
                    opp: [w[8], w[9], w[10], w[11], w[12]],
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        // As many opponents as the missions name.
        let count = missions
            .iter()
            .flat_map(|m| m.opp)
            .filter(|&o| o >= 0)
            .max()
            .map_or(0, |m| m as usize + 1);
        let table = pack_section(res, 11)?;
        let opps = (0..count)
            .map(|n| {
                let w = words(table, n * 0x2C, 11).ok_or("short opponent table")?;
                Ok(MissionOpp {
                    chara: w[0],
                    costume: w[1],
                    cpu_level: w[2],
                    item7: w[3],
                    item: [w[4], w[5], w[6], w[7], w[8], w[9], w[10]],
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        Ok(Self {
            data: MissionSelectData {
                fod: pack_section(res, 1)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 7)?, 2)?.to_vec(),
                text: pack_section(res, 18)?.to_vec(),
                lips: pack_section(res, 17)?.to_vec(),
                missions,
                opps,
                layout: MissionSelectLayout::default(),
            },
            background: decode_picture(res, 4)?,
            files,
        })
    }
}

/// What a chosen mission leaves for the battle: the rule and side 1 (`MisSel_SetupBattle`), and the team size and
/// DP rule of the player's side.
#[derive(Clone, Debug, PartialEq)]
pub struct MissionSetup {
    pub setup: BattleSetup,
    pub team_size: i32,
    pub dp_rule: i32,
    /// The mission chosen (page * 5 + row).
    pub mission: i32,
}

/// `MisSel` and the frame loop of `MisSel_Run`.
pub struct MissionSelectScreen {
    data: MissionSelectData,
    flash: Flash,
    tex: Vec<Option<usize>>,
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
    page_count: i32,
    page: i32,
    mission: i32,
    idle: i32,
    result: i32,
    done: Option<Option<MissionSetup>>,
}

impl MissionSelectScreen {
    /// `MisSel_Init(2)` and the first `ColorFade_StartIn`: the page and row of the last visit
    /// (`gProgress->missionPage` / `cursor`).
    pub fn new(
        data: &MissionSelectData,
        page: i32,
        cursor: i32,
        save: &SaveData,
        libc: &mut LibcRandom,
    ) -> Result<Self, Box<dyn Error>> {
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
            cursor,
            timer: 0,
            level: 0,
            voice_req: 0,
            voice_last: 0,
            voice_skip: false,
            voice_line: -1,
            blink,
            talk: 0,
            page_count: save.mission_pages(),
            page,
            mission: 0,
            idle: 0,
            result: 1,
            done: None,
        })
    }

    /// `MisSel_Run`'s result once it has returned: the chosen mission's setup, or none when the player backed out.
    pub fn result(&self) -> Option<&Option<MissionSetup>> {
        self.done.as_ref()
    }

    /// The page and row to keep in `gProgress` (`missionPage`, `cursor`).
    pub fn position(&self) -> (i32, i32) {
        (self.page, self.cursor)
    }

    fn def(&self) -> MissionDef {
        self.data
            .missions
            .get(self.mission.max(0) as usize)
            .copied()
            .unwrap_or_default()
    }

    fn opp(&self, i: i32) -> MissionOpp {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.data.opps.get(i).copied())
            .unwrap_or(MissionOpp {
                chara: NONE,
                ..MissionOpp::default()
            })
    }

    /// `MisSel_SetupBattle`: the rule (battle mode 2) and side 1, a computer team of up to five from the tables.
    fn setup_battle(&self, random: &mut GameRandom, libc: &mut LibcRandom) -> MissionSetup {
        let def = self.def();
        let announcer = if def.announcer == RANDOM {
            random.range(8) as i32
        } else {
            def.announcer
        };
        let stage = if def.stage == RANDOM {
            random.range(0x23) as i32
        } else {
            def.stage
        };
        let bgm = if def.bgm == RANDOM { 0x18 } else { def.bgm };
        // BattleSetup_SetRule: music 24 is a random one of 0..23.
        let bgm = if bgm == 24 { libc.rand() % 24 } else { bgm };
        let team_size = match def.kind {
            0 => 1,
            1 => 2,
            2 => 3,
            3 => 4,
            6 | 7 | 10 => 5,
            _ => 0,
        };
        let opps: Vec<MissionOpp> = def
            .opp
            .iter()
            .map(|&o| self.opp(o))
            .take_while(|o| o.chara != NONE)
            .collect();
        let items = |o: &MissionOpp| {
            let mut items = [0u16; 8];
            let mut k = 0;
            for &id in &o.item {
                if id != NONE {
                    items[k] = (id + 1) as u16;
                    k += 1;
                }
            }
            items[7] = if o.item7 == NONE {
                0
            } else {
                (o.item7 + 1) as u16
            };
            items
        };
        let lead = opps.first().copied().unwrap_or_default();
        let mut side1 = SideSetup {
            control: 2,
            pad: 1,
            change_allowed: i32::from(def.change_allowed != 0),
            switch_enabled: 1,
            chara: lead.chara,
            costume: lead.costume,
            cpu_level: lead.cpu_level,
            items: items(&lead),
            team_count: opps.len().max(1) as i32 - 1,
            ..SideSetup::default()
        };
        let mut members = [MemberStart {
            cpu_level: lead.cpu_level,
            health: 100.0,
            variant: 0,
        }; 5];
        for (i, o) in opps.iter().enumerate().skip(1) {
            side1.team[i - 1] = (o.chara, o.costume);
            side1.team_items[i - 1] = items(o);
            members[i].cpu_level = o.cpu_level;
        }
        side1.members = Some(members);
        let setup = BattleSetup {
            wide: false,
            screen_mode: 0,
            mode: 2,
            bgm,
            time_limit: def.time_limit,
            announcer,
            stage,
            stage_change: i32::from(def.stage_change != 0),
            sides: [SideSetup::default(), side1],
            pool: Vec::new(),
            tutorial: 0,
        };
        MissionSetup {
            setup,
            team_size,
            dp_rule: def.dp_rule,
            mission: self.mission,
        }
    }

    /// One iteration of `MisSel_Run`'s loop.
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
        // MisSel_Update.
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

    /// `MisSel_UpdateVoice`: the line the guide was asked for (1 the greeting, 2 the idle line).
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
            1 => self.voice_line = 0x33,
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

    /// `MisSel_SetRank`: a mission's rank letter on its row, enlarged when the row is under the cursor.
    fn set_rank(&mut self, row: i32, rank: i32) {
        let f = &mut self.flash;
        let r = f.find(
            Some(&format!("mc_window_plate{:02}", row + 1)),
            "mc_rankmoji",
        );
        f.clip_set_uv(r, uv(rank * 0x20, 0, rank * 0x20 + 0x20, 0x20));
        if row == self.cursor {
            let r = f.find(None, "mc_rank");
            f.clip_set_uv(r, uv(rank * 0x40, 0, rank * 0x40 + 0x40, 0x40));
        }
    }

    /// `MisSel_Draw`: the page, the window of the mission and its opponents' chips.
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
        {
            let f = &mut self.flash;
            let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_eye");
            anim::blink(f, r, &mut self.blink, 0, random);
            let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_mouth");
            anim::talk(f, r, &mut self.talk, 0, &mut self.lip, voice_stat, random);
            anim::num_draw(
                f,
                |n| format!("mc_pt_suji{n:02}"),
                2,
                self.page_count,
                (0x20, 0x20),
                1,
            );
            anim::num_draw(
                f,
                |n| format!("mc_pt_suji{:02}", n + 2),
                2,
                self.page + 1,
                (0x20, 0x20),
                1,
            );
            let r = f.find(None, "mc_yajirusi_r");
            f.clip_set_uv(r, uv(0x20, 0, 0x40, 0x20));
            if self.page == 0 {
                let r = f.find(None, "mc_yajirusi_l");
                f.clip_set_flags(r, PROP_VISIBLE, false);
            }
            if self.page == self.page_count - 1 {
                let r = f.find(None, "mc_yajirusi_r");
                f.clip_set_flags(r, PROP_VISIBLE, false);
            }
        }
        let row_box = TextBox::preset(0)
            .with_align(0)
            .with_max_width(0xEB)
            .with_line_y([0, -10, 0, 0, 0]);
        for i in 0..ROWS {
            let n = self.page * ROWS + i;
            let rank = i32::from(save.mission(n.max(0) as usize).rank);
            let name = format!("mc_window_plate{:02}", i + 1);
            let f = &mut self.flash;
            anim::num_draw_child(
                f,
                |k| (name.clone(), format!("mc_window_text_off{k}")),
                3,
                n + 1,
                (0x20, 0x20),
                1,
            );
            let r = f.find(Some(&name), "mc_window_text1_0");
            texts.extend(row_box.attach_line(
                f,
                r,
                0x400 + i as u32,
                &self.data.text,
                n + 0xD8,
                font,
            ));
            self.set_rank(i, rank);
        }
        let def = self.def();
        let plain = TextBox::preset(0).with_align(0);
        {
            let f = &mut self.flash;
            let r = f.find(None, "mc_missionkari01");
            texts.extend(plain.attach_line(f, r, 0x405, &self.data.text, def.kind + 0x148, font));
            let r = f.find(None, "mc_missionkari02");
            if def.dp_rule != 0 {
                texts.extend(plain.attach_line(
                    f,
                    r,
                    0x406,
                    &self.data.text,
                    def.dp_rule + 0x156,
                    font,
                ));
            }
        }
        for (i, &image) in FACE_TEX.iter().enumerate() {
            let chara = self.opp(def.opp[i]).chara;
            let sub = if chara == NONE {
                CHIP_NUM
            } else {
                (chara + 1).max(1) as usize
            };
            self.tex[image] = self.data.layout.chips.get(sub - 1).copied().flatten();
        }
        let rec = save.mission(self.mission.max(0) as usize);
        let f = &mut self.flash;
        let r = f.find(Some("mc_mission_plate01"), "mc_misson_plate_text");
        f.clip_set_uv(r, uv(0, 0x20, 0x100, 0x40));
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

    /// `MisSel_ClipGoto(0, 0, label)`.
    fn clip_goto(&mut self, label: &str) {
        let r = self
            .flash
            .find(None, &format!("mc_window_plate{:02}", self.cursor + 1));
        self.flash.clip_goto_label(r, label);
    }

    /// `MisSel_Input`: level 0 up / down the rows, left / right the pages, confirm opens the mission's window;
    /// level 1 confirm chooses it, cancel closes it.
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
        match self.level {
            0 => {
                if repeat & PAD_UP != 0 {
                    self.clip_goto("fl_off_start");
                    self.cursor -= 1;
                    if self.cursor < 0 {
                        self.cursor = ROWS - 1;
                    }
                    self.clip_goto("fl_on_start");
                    sounds.push(TitleSound::Effect(1, 0));
                    self.idle = 0;
                } else if repeat & PAD_DOWN != 0 {
                    self.clip_goto("fl_off_start");
                    self.cursor += 1;
                    if self.cursor >= ROWS {
                        self.cursor = 0;
                    }
                    self.clip_goto("fl_on_start");
                    sounds.push(TitleSound::Effect(1, 0));
                    self.idle = 0;
                } else if repeat & PAD_LEFT != 0 {
                    if self.page != 0 {
                        self.page -= 1;
                        sounds.push(TitleSound::Effect(1, 0));
                        self.idle = 0;
                    }
                } else if repeat & PAD_RIGHT != 0 {
                    if (self.page as u32) < (self.page_count - 1) as u32 {
                        self.page += 1;
                        sounds.push(TitleSound::Effect(1, 0));
                        self.idle = 0;
                    }
                } else if confirm {
                    self.mission = self.page * ROWS + self.cursor;
                    self.clip_goto("fl_rank_out");
                    self.flash
                        .goto_label(&format!("fl_mission_{:02}_in", self.cursor + 1), true);
                    self.level = 1;
                    sounds.push(TitleSound::Effect(1, 1));
                    self.idle = 0;
                } else if cancel {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.result = 0;
                    self.flash.goto_label("fl_mission100_menu_cansel", true);
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
                        .goto_label(&format!("fl_mission_{:02}_cansel", self.cursor + 1), true);
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

    /// Runs the real mission list from the ISO in `rom/`, when it is there: the guide's greeting, the second page,
    /// a mission's window opened and confirmed, and its rule and opponents set up.
    #[test]
    fn real_mission_select_sets_a_mission_up() {
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
        let assets = MissionSelectAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = MissionSelectLayout::new(0, |k| firsts[&k]);
        // Every mission names a real opponent first.
        for m in &data.missions {
            assert!(
                (0..=0xA0).contains(&data.opps[m.opp[0] as usize].chara),
                "{m:?}"
            );
        }
        let save = SaveData::default();
        assert_eq!(save.mission_pages(), 5);
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[2, 7]);
        let mut screen = MissionSelectScreen::new(&data, 0, 0, &save, &mut libc).unwrap();
        let mut font = Font::new(Vec::new(), 20);
        let mut press = |screen: &mut MissionSelectScreen, bits: u32| {
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
        assert_eq!(screen.voice_line, 0x33);
        press(&mut screen, PAD_RIGHT);
        press(&mut screen, PAD_DOWN);
        assert_eq!(screen.position(), (1, 1));
        press(&mut screen, PAD_CROSS);
        assert_eq!(screen.level, 1);
        press(&mut screen, PAD_CROSS);
        for _ in 0..3 {
            press(&mut screen, 0);
        }
        let Some(Some(setup)) = screen.result().cloned() else {
            panic!("no mission chosen");
        };
        assert_eq!(setup.mission, 6);
        let def = data.missions[6];
        assert_eq!(setup.setup.mode, 2);
        assert_eq!(setup.dp_rule, def.dp_rule);
        let side = &setup.setup.sides[1];
        assert_eq!(side.control, 2);
        assert_eq!(side.chara, data.opps[def.opp[0] as usize].chara);
        assert!(setup.team_size >= 1 || [4, 5, 8, 9].contains(&def.kind));
    }
}
