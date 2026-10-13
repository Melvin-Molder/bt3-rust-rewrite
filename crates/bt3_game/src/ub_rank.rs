//! Ultimate Battle Z's ranking ladder (`UbRank_Run`, src/menu/ub_rank.c, progress mode 26, `Ub_Main`): a list of
//! 100 places, five plates on screen, the player holding the save's place. Challenging a place sets up a one-on-one
//! battle with its holder; challenging one above the player's own (from the tenth up) may bring in one of 37
//! intruders instead. Back from a battle for a higher place, the win / lose panel, the player moving up a place
//! (and five more after beating an intruder) as the list scrolls.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{BattleSetup, Pick, PictureLoader, SideSetup};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::ub::VOICE_BASE;
use crate::ubz_select::{Foe, NONE, RANDOM, parse_foes, words};
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};

/// `UbRank_Init`'s file: `gProgress->baseFile + 0x1B`.
pub const FILE_ID: u32 = 0x1C1 + 0x1B;
const FACE_FILE: u32 = 0x2F9;
const FADE_FRAMES: i32 = 0x14;
const IDLE_FRAMES: i32 = 0x708;
const CHIP_NUM: usize = 165;
/// `UBRANK_ROWS`, `UBRANK_PLACES`.
const ROWS: i32 = 5;
const PLACES: i32 = 100;
const INTRUDERS: usize = 0x25;
const STAGES: usize = 0x1C;
const BGMS: usize = 0xB;
/// `sUbRankChipTex`: the movie's images of the six plates' holders.
const CHIP_TEX: [usize; 6] = [13, 15, 16, 17, 18, 19];

/// `NPROG_UB_*` of `gProgress->ubFlags`.
pub const FOUGHT: i32 = 4;
pub const UPWARD: i32 = 8;
pub const INTRUDER: i32 = 0x10;

// UBRANK_*.
const CHOSEN: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const FADED_IN: u32 = 8;
const FACES_READY: u32 = 0x10;
const PANEL_DONE: u32 = 0x20;
const INTRUDE: u32 = 0x40;
const INTRUDE_DONE: u32 = 0x80;
const RESULT: u32 = 0x100;

// UBRANK_ST_*.
const ST_LIST: i32 = 0;
const ST_INTRUDE: i32 = 1;
const ST_VS: i32 = 2;
const ST_CLOSING: i32 = 3;
const ST_WAIT: i32 = 4;
const ST_RESULT: i32 = 5;
const ST_CLIMB: i32 = 6;

// UBRANK_LOAD_*.
const LOAD_NONE: i32 = 0;
const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;

/// `gUbRank->tex[n]` (movie 0, the list).
const TEX: [ImageTexture; 37] = [
    tex(3, 0),
    tex(3, 1),
    tex(3, 18),
    tex(3, 17),
    tex(3, 22),
    tex(3, 23),
    tex(3, 24),
    tex(3, 16),
    tex(3, 7),
    None,
    tex(3, 14),
    tex(3, 13),
    tex(3, 11),
    None,
    tex(3, 12),
    None,
    None,
    None,
    None,
    None,
    tex(3, 15),
    tex(3, 9),
    tex(3, 8),
    tex(3, 25),
    tex(3, 10),
    tex(3, 2),
    tex(3, 3),
    tex(3, 4),
    tex(3, 5),
    tex(3, 6),
    tex(3, 19),
    tex(5, 0),
    tex(5, 1),
    tex(5, 3),
    tex(3, 27),
    tex(3, 28),
    tex(3, 29),
];
/// `texB[n]` (movie 1, the versus panel): 0 and 3 the two chips.
const TEX_B: [ImageTexture; 6] = [None, tex(3, 11), tex(3, 12), None, tex(3, 21), tex(3, 20)];
/// `texC[n]` (movie 2, the closing movie): 10 the opponent's portrait, 11 the player's.
const TEX_C: [ImageTexture; 13] = [
    tex(17, 0),
    tex(17, 1),
    tex(17, 2),
    tex(17, 3),
    tex(17, 4),
    tex(17, 5),
    tex(17, 6),
    tex(17, 7),
    tex(17, 8),
    tex(17, 9),
    None,
    None,
    tex(17, 10),
];
/// The small picture that stands for the player in the list.
const PLAYER_CHIP: [ImageTexture; 1] = [tex(3, 26)];
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

/// The rules of one place of the ladder or of one intruder (`NRankRule`, 0x1C bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RankRule {
    pub announcer: i32,
    pub stage_change: i32,
    pub time_limit: i32,
    /// `N_RANDOM`: one of the stage list.
    pub stage: i32,
    /// `N_RANDOM`: one of the music list.
    pub bgm: i32,
    pub change_allowed: i32,
    /// Index into the opponents.
    pub foe: i32,
}

fn parse_rules(data: &[u8], count: usize) -> Result<Vec<RankRule>, Box<dyn Error>> {
    (0..count)
        .map(|n| {
            let w = words(data, n * 0x1C, 7).ok_or("short rule table")?;
            Ok(RankRule {
                announcer: w[0],
                stage_change: w[1],
                time_limit: w[2],
                stage: w[3],
                bgm: w[4],
                change_allowed: w[5],
                foe: w[6],
            })
        })
        .collect()
}

/// What `UbRank_Init` reads from its file.
pub struct UbRankAssets {
    pub data: UbRankData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct UbRankData {
    /// Sections 1, 15 and 16: the list, the versus panel, the closing movie.
    pub fods: [Vec<u8>; 3],
    pub icon_fod: Vec<u8>,
    /// Section 14: the names of the places' holders; 18: the guide's mouth keys.
    pub text: Vec<u8>,
    pub lips: Vec<u8>,
    /// Sections 8 and 9: one rule per place (the player's left out), and the intruders'.
    pub rules: Vec<RankRule>,
    pub intruder_rules: Vec<RankRule>,
    /// Sections 10 and 11.
    pub foes: Vec<Foe>,
    pub intruder_foes: Vec<Foe>,
    /// Sections 12 and 13: the stages and music a random rule draws from.
    pub stages: Vec<i32>,
    pub bgms: Vec<i32>,
    pub layout: UbRankLayout,
}

#[derive(Clone, Debug, Default)]
pub struct UbRankLayout {
    pub background: usize,
    pub flash: [Vec<Option<usize>>; 3],
    pub player_chip: Option<usize>,
    pub icon: Vec<Option<usize>>,
    pub chips: Vec<Option<usize>>,
}

impl UbRankLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        let i = |t: &[ImageTexture]| image_indices(t, &first);
        Self {
            background,
            flash: [i(&TEX), i(&TEX_B), i(&TEX_C)],
            player_chip: i(&PLAYER_CHIP)[0],
            icon: i(&ICON_TEXTURES),
            chips: i(&chip_textures()),
        }
    }
}

impl UbRankAssets {
    /// From the unpacked file.
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let chips = chip_textures();
        let files = decode_files(
            res,
            [
                &TEX[..],
                &TEX_B,
                &TEX_C,
                &PLAYER_CHIP,
                &ICON_TEXTURES,
                &chips,
            ],
        )?;
        let rules = parse_rules(pack_section(res, 8)?, PLACES as usize - 1)?;
        let intruder_rules = parse_rules(pack_section(res, 9)?, INTRUDERS)?;
        let count = |rules: &[RankRule]| {
            rules
                .iter()
                .map(|r| r.foe)
                .filter(|&f| f >= 0)
                .max()
                .map_or(0, |m| m as usize + 1)
        };
        let list = |n: usize, count: usize| -> Result<Vec<i32>, Box<dyn Error>> {
            Ok(words(pack_section(res, n)?, 0, count).ok_or("short list")?)
        };
        let sec =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        Ok(Self {
            data: UbRankData {
                fods: [sec(1)?, sec(15)?, sec(16)?],
                icon_fod: pack_section(pack_section(res, 7)?, 2)?.to_vec(),
                text: sec(14)?,
                lips: sec(18)?,
                foes: parse_foes(pack_section(res, 10)?, count(&rules))?,
                intruder_foes: parse_foes(pack_section(res, 11)?, count(&intruder_rules))?,
                rules,
                intruder_rules,
                stages: list(12, STAGES)?,
                bgms: list(13, BGMS)?,
                layout: UbRankLayout::default(),
            },
            background: decode_picture(res, 4)?,
            files,
        })
    }
}

/// `UbRank` and the frame loop of `UbRank_Run`.
pub struct UbRankScreen {
    data: UbRankData,
    flash: [Flash; 3],
    tex: [Vec<Option<usize>>; 3],
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    cursor: i32,
    timer: i32,
    state: i32,
    voice_req: i32,
    voice_prev: i32,
    voice_skip: bool,
    voice_line: i32,
    blink: i32,
    talk: i32,
    idle: i32,
    extra: i32,
    load_state: i32,
    loading: [Option<usize>; 2],
    top: i32,
    cur_rule: RankRule,
    cur_foe: Foe,
    won: bool,
    climb: i32,
    wait: i32,
    /// The player's fighter (`gProgress->team[0]`).
    player: Pick,
    /// `gProgress->ubFlags` and `ubChoice` as the screen leaves them.
    ub_flags: i32,
    choice: i32,
    result: i32,
    done: Option<Option<BattleSetup>>,
}

impl UbRankScreen {
    /// `UbRank_Init` and the first `ColorFade_StartIn`. `ub_flags` / `choice` are `gProgress->ubFlags` /
    /// `ubChoice`, `winner` the last battle's winner bits (`BattleResult_GetFlags`); back from a ladder battle the
    /// outcome is taken over.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &UbRankData,
        player: Pick,
        ub_flags: i32,
        choice: i32,
        winner: u32,
        save: &SaveData,
        libc: &mut LibcRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let make = |fod: &[u8]| -> Result<Flash, Box<dyn Error>> { Ok(Flash::create(fod)?) };
        let mut flash = [
            make(&data.fods[0])?,
            make(&data.fods[1])?,
            make(&data.fods[2])?,
        ];
        flash[0].play(1);
        flash[1].play(1);
        let blink = libc.rand() % 32;
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut screen = Self {
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
            state: ST_LIST,
            voice_req: 0,
            voice_prev: 0,
            voice_skip: false,
            voice_line: -1,
            blink,
            talk: 0,
            idle: 0,
            extra: 0,
            load_state: LOAD_NONE,
            loading: [None; 2],
            top: 0,
            cur_rule: RankRule::default(),
            cur_foe: Foe::default(),
            won: false,
            climb: 0,
            wait: 0,
            player,
            ub_flags,
            choice,
            result: 1,
            done: None,
        };
        // The player's place, on which the list is centred.
        let mut place = save.ub_rank();
        if screen.ub_flags & FOUGHT != 0 {
            screen.ub_flags &= !FOUGHT;
            screen.won = winner == 1;
            if screen.ub_flags & UPWARD != 0 {
                screen.flags |= RESULT;
                screen.set_foe(choice, false);
                if screen.won {
                    if place > 0 {
                        place -= 1;
                    }
                    if screen.ub_flags & INTRUDER != 0 {
                        screen.climb = 5;
                    }
                }
            }
        }
        screen.top = if place >= 98 {
            PLACES - ROWS
        } else if place < 2 {
            0
        } else {
            place - 2
        };
        Ok(screen)
    }

    /// `UbRank_Run`'s result once it has returned: the ladder battle's setup, or none when the player backed out.
    pub fn result(&self) -> Option<&Option<BattleSetup>> {
        self.done.as_ref()
    }

    /// `gProgress->ubFlags` and `ubChoice`.
    pub fn progress(&self) -> (i32, i32) {
        (self.ub_flags, self.choice)
    }

    /// `UbRank_PlaceToRule`: the places below the player's own are shifted by one.
    fn place_to_rule(place: i32, save: &SaveData) -> i32 {
        if save.ub_rank() < place {
            place - 1
        } else {
            place
        }
    }

    /// `UbRank_SetFoe`.
    fn set_foe(&mut self, idx: i32, intruder: bool) {
        let (rules, foes) = if intruder {
            (&self.data.intruder_rules, &self.data.intruder_foes)
        } else {
            (&self.data.rules, &self.data.foes)
        };
        self.cur_rule = rules.get(idx.max(0) as usize).copied().unwrap_or_default();
        self.cur_foe = foes
            .get(self.cur_rule.foe.max(0) as usize)
            .copied()
            .unwrap_or_default();
    }

    /// `UbRank_PickFoe`: the place's holder, or (one time in ten, for the tenth place up when above the player's
    /// own) an intruder; true for an intruder.
    fn pick_foe(&mut self, place: i32, save: &SaveData, random: &mut GameRandom) -> bool {
        let intrude = (random.range(0xFFFF) as i32) < 0x199A;
        if place < 10 || !intrude || save.ub_rank() < place {
            self.set_foe(Self::place_to_rule(place, save), false);
            self.ub_flags &= !INTRUDER;
            return false;
        }
        self.set_foe(random.range(INTRUDERS as u32) as i32, true);
        self.ub_flags |= INTRUDER;
        true
    }

    /// `UbRank_ScrollUp`: the list one place towards the first; false at the top.
    fn scroll_up(&mut self, sounds: &mut Vec<TitleSound>) -> bool {
        if self.top != 0 {
            self.top -= 1;
            self.flash[0].goto_label("fl_ranking_down", true);
            sounds.push(TitleSound::Effect(1, 0));
            self.extra = self.top + 5;
            return true;
        }
        false
    }

    fn chip(&self, chara: i32) -> Option<usize> {
        usize::try_from(chara)
            .ok()
            .and_then(|c| self.data.layout.chips.get(c).copied())
            .flatten()
    }

    fn holder(&self, place: i32, save: &SaveData) -> i32 {
        let rule = self
            .data
            .rules
            .get(Self::place_to_rule(place, save).max(0) as usize)
            .copied()
            .unwrap_or_default();
        self.data
            .foes
            .get(rule.foe.max(0) as usize)
            .map_or(NONE, |f| f.chara)
    }

    /// `UbRank_SetupBattle`: the player's fighter against the opponent chosen (battle mode 2, one fighter a side).
    fn setup_battle(&self, random: &mut GameRandom, libc: &mut LibcRandom) -> BattleSetup {
        let rule = self.cur_rule;
        let announcer = if rule.announcer == RANDOM {
            random.range(8) as i32
        } else {
            rule.announcer
        };
        let stage = if rule.stage == RANDOM {
            self.data.stages[random.range(STAGES as u32) as usize]
        } else {
            rule.stage
        };
        let bgm = if rule.bgm == RANDOM {
            self.data.bgms[random.range(BGMS as u32) as usize]
        } else {
            rule.bgm
        };
        // BattleSetup_SetRule: music 24 is a random one of 0..23.
        let bgm = if bgm == 24 { libc.rand() % 24 } else { bgm };
        let foe = self.cur_foe;
        let mut setup = BattleSetup {
            wide: false,
            screen_mode: 0,
            mode: 2,
            bgm,
            time_limit: rule.time_limit,
            announcer,
            stage,
            stage_change: i32::from(rule.stage_change != 0),
            sides: [
                SideSetup {
                    control: 0,
                    pad: 0,
                    change_allowed: 1,
                    switch_enabled: 0,
                    chara: self.player.picked,
                    costume: self.player.color_plate,
                    cpu_level: 0,
                    items: self.player.items,
                    ..SideSetup::default()
                },
                SideSetup {
                    control: 2,
                    pad: 1,
                    change_allowed: i32::from(rule.change_allowed != 0),
                    switch_enabled: 0,
                    chara: foe.chara,
                    costume: foe.costume,
                    cpu_level: foe.cpu_level,
                    items: foe.items(),
                    ..SideSetup::default()
                },
            ],
            pool: Vec::new(),
            tutorial: 0,
        };
        setup.finish();
        setup
    }

    /// One iteration of `UbRank_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        save: &mut SaveData,
        loader: &mut dyn PictureLoader,
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
        self.update_face_load(loader);
        self.update(&mut frame.sounds);
        self.update_voice(pressed, voice_stat, &mut frame.sounds);
        self.draw(save, voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & FADED_IN == 0
            && self.flash[0].flags() & FLASH_PAD != 0
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
            self.input(pressed, repeat, save, random, &mut frame.sounds);
        }
        frame
    }

    /// `UbRank_Update`: the leave timer; the marks of the list and panel movies; the movies advance.
    fn update(&mut self, sounds: &mut Vec<TitleSound>) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.flash[0].trig() & 1 != 0 && self.state == ST_INTRUDE {
            self.flash[1].goto_label("fl_intrude_in", true);
            self.flags |= INTRUDE;
        }
        if self.flash[0].trig() & 2 != 0 && self.state == ST_CLIMB {
            self.flash[1].goto_label("fl_win_player", true);
            self.state = ST_RESULT;
        }
        if self.flash[1].trig() & 1 != 0 {
            match self.state {
                ST_INTRUDE => {
                    self.flags |= INTRUDE_DONE;
                    self.flags &= !INTRUDE;
                }
                ST_VS | ST_RESULT => self.flags |= PANEL_DONE,
                _ => {}
            }
        }
        if self.flash[1].se() & 1 != 0 {
            sounds.push(TitleSound::Effect(2, 0x3A));
        }
        for f in &mut self.flash {
            f.advance();
        }
    }

    /// `UbRank_UpdateVoice`.
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
            1 => self.voice_line = 0x4A,
            2 => self.voice_line = 0x4B,
            3 => self.voice_line = 0x4C,
            4 => self.voice_line = 0x50,
            5 => self.voice_line = 0x51,
            6 => self.voice_line = 0x35,
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
        self.voice_prev = self.voice_req;
    }

    /// `UbRank_UpdateFaceLoad`: the two fighters' large pictures, one step a frame.
    fn update_face_load(&mut self, loader: &mut dyn PictureLoader) {
        match self.load_state {
            LOAD_REQUEST => {
                self.tex[2][11] = None;
                self.tex[2][10] = None;
                let chara = [self.player.picked, self.cur_foe.chara];
                self.loading = chara.map(|c| loader.portrait(FACE_FILE + c.max(0) as u32));
                self.load_state = LOAD_READ;
            }
            // File_UpdateRequests: the reads are done by the next call.
            LOAD_READ => self.load_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.tex[2][11] = self.loading[0];
                self.tex[2][10] = self.loading[1];
                self.flags |= FACES_READY;
                self.load_state = LOAD_NONE;
            }
            _ => {}
        }
    }

    /// `UbRank_Draw`: the six plates of the list, the arrows, the versus panel.
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
        let rank = save.ub_rank();
        let player_chip = self.data.layout.player_chip;
        // The holders of the five plates and of the sixth while the list scrolls.
        let rows: Vec<(i32, i32)> = (0..ROWS)
            .map(|i| self.top + i)
            .chain(std::iter::once(self.extra))
            .map(|place| {
                let chara = if place == rank {
                    self.player.picked
                } else {
                    self.holder(place, save)
                };
                (place, chara)
            })
            .collect();
        for (i, &(place, chara)) in rows.iter().enumerate() {
            self.tex[0][CHIP_TEX[i]] = if place == rank {
                player_chip
            } else {
                self.chip(chara)
            };
        }
        let showing = matches!(self.state, ST_VS | ST_CLOSING | ST_WAIT | ST_RESULT);
        {
            let f = &mut self.flash[0];
            let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_eye");
            anim::blink(f, r, &mut self.blink, 0, random);
            let r = f.find(Some("mc_guide_17go"), "mc_guide_17go_mouth");
            anim::talk(f, r, &mut self.talk, 0, &mut self.lip, voice_stat, random);
            for (i, &(place, chara)) in rows.iter().enumerate() {
                let name = format!("mc_ranking_plate{i:02}");
                anim::num_draw_child(
                    f,
                    |n| (name.clone(), format!("mc_jyuni_suji{n}")),
                    3,
                    place + 1,
                    (0x40, 0x40),
                    0,
                );
                let r = f.find(Some(&name), "mc_charaname_kari");
                texts.extend(plain.attach_line(
                    f,
                    r,
                    0x400 + i as u32,
                    &self.data.text,
                    chara,
                    font,
                ));
                if i < ROWS as usize {
                    let r = f.find(None, &format!("mc_senreki_chara{i}"));
                    let hidden = showing
                        && self.ub_flags & UPWARD != 0
                        && (rank == place || self.choice == place);
                    f.clip_set_flags(r, PROP_VISIBLE, !hidden);
                }
            }
            let r = f.find(Some("mc_yajirusi_up"), "mc_yajirusi_icon_up");
            f.clip_set_flags(r, PROP_VISIBLE, self.top > 0);
            f.clip_set_uv(r, uv(0, 0x20, 0x20, 0x40));
            let r = f.find(Some("mc_yajirusi_down"), "mc_yajirusi_icon_down");
            f.clip_set_flags(r, PROP_VISIBLE, self.top < PLACES - ROWS);
            f.clip_set_uv(r, uv(0x20, 0x20, 0x40, 0x40));
        }
        // The versus panel.
        let offset = |base: i32| (0xC5, (self.choice - self.top) * 0x34 + base);
        match self.state {
            ST_INTRUDE => {
                if self.flags & INTRUDE != 0 {
                    let (x, y) = offset(0x84);
                    self.flash[1].set_offset(x, y);
                    self.tex[1][3] = self.chip(self.holder(self.choice, save));
                    self.tex[1][0] = self.chip(self.cur_foe.chara);
                    self.flash[1].set_flag(4, false);
                }
            }
            ST_VS | ST_CLOSING | ST_WAIT | ST_RESULT => {
                if self.ub_flags & UPWARD != 0 {
                    let (x, y) = offset(0xB8);
                    self.flash[1].set_offset(x, y);
                    let foe = self.chip(self.cur_foe.chara);
                    if rank > self.choice {
                        self.tex[1][3] = foe;
                        self.tex[1][0] = player_chip;
                    } else {
                        self.tex[1][0] = foe;
                        self.tex[1][3] = player_chip;
                    }
                    self.flash[1].set_flag(4, false);
                } else {
                    self.flash[1].set_flag(4, true);
                }
            }
            _ => self.flash[1].set_flag(4, true),
        }
        for i in 0..3 {
            if self.state != ST_WAIT || i == 2 {
                let tex = &self.tex[i];
                let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
                let (fq, overs) = self.flash[i].draw_with_overs(&has_texture);
                quads.extend(screen_quads(&fq, tex));
                if i == 0 {
                    draw_texts(&texts, &overs, font);
                    font.flush(quads);
                }
            }
        }
        if self.state != ST_WAIT {
            self.icon.draw(quads);
        }
    }

    /// `UbRank_ClipGoto(0, 0, label)`.
    fn clip_goto(&mut self, label: &str) {
        let r = self.flash[0].find(None, &format!("mc_ranking_plate{:02}", self.cursor));
        self.flash[0].clip_goto_label(r, label);
    }

    /// `UbRank_Input`: the list, the intruder, the versus panel, the outcome of a battle.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        save: &mut SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        let state = self.state;
        let pos = self.top + self.cursor;
        let ok = pressed & PAD_CROSS != 0;
        let cancel = pressed & PAD_TRIANGLE != 0;
        if self.flash[0].flags() & FLASH_PAD == 0 {
            return;
        }
        let rank = save.ub_rank();
        if self.flags & STARTED == 0 {
            self.cursor = rank - self.top;
            self.clip_goto("fl_on_start");
            self.flags |= STARTED;
            if self.flags & RESULT != 0 {
                self.state = ST_RESULT;
                if self.won {
                    match rank {
                        1 => self.voice_req = 5,
                        2 => self.voice_req = 3,
                        0 => {}
                        _ => self.voice_req = random.range(2) as i32 + 1,
                    }
                    self.flash[1].goto_label("fl_win_player", true);
                } else {
                    self.flash[1].goto_label("fl_win_cpu", true);
                }
            }
            return;
        }
        match state {
            ST_LIST => {
                if repeat & PAD_UP != 0 && rank - 1 < pos {
                    if self.cursor == 1 && self.top != 0 {
                        self.top -= 1;
                        self.flash[0].goto_label("fl_ranking_down", true);
                        sounds.push(TitleSound::Effect(1, 0));
                        self.extra = self.top + 5;
                    } else if self.cursor > 0 {
                        self.clip_goto("fl_off_start");
                        self.cursor -= 1;
                        self.clip_goto("fl_on_start");
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                    self.idle = 0;
                } else if repeat & PAD_DOWN != 0 {
                    if self.cursor < ROWS - 1 {
                        self.clip_goto("fl_off_start");
                        self.cursor += 1;
                        self.clip_goto("fl_on_start");
                        sounds.push(TitleSound::Effect(1, 0));
                    } else if self.top < PLACES - ROWS {
                        self.top += 1;
                        self.flash[0].goto_label("fl_ranking_up", true);
                        sounds.push(TitleSound::Effect(1, 0));
                        self.extra = self.top - 1;
                    }
                    self.idle = 0;
                } else if ok {
                    self.choice = self.top + self.cursor;
                    if self.choice == rank {
                        sounds.push(TitleSound::Effect(1, 7));
                    } else {
                        if self.choice < rank {
                            self.ub_flags |= UPWARD;
                        } else {
                            self.ub_flags &= !UPWARD;
                        }
                        if self.pick_foe(self.choice, save, random) {
                            self.flash[0].goto_label("fl_intrude_in", true);
                            self.state = ST_INTRUDE;
                            self.voice_req = 4;
                        } else {
                            self.state = ST_VS;
                            if self.ub_flags & UPWARD != 0 {
                                self.flash[1].goto_label("fl_vs_action", true);
                            } else {
                                self.flags |= PANEL_DONE;
                            }
                        }
                        self.load_state = LOAD_REQUEST;
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                    self.idle = 0;
                } else if cancel {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.result = 0;
                    self.flash[0].goto_label("fl_ranking_cancel", true);
                    sounds.push(TitleSound::Effect(1, 2));
                    self.idle = 0;
                } else {
                    self.idle += 1;
                    if self.idle == IDLE_FRAMES {
                        self.voice_req = 6;
                        self.idle = 0;
                    }
                }
            }
            ST_INTRUDE => {
                if self.flags & INTRUDE_DONE != 0 {
                    self.state = ST_VS;
                    self.flash[1].goto_label("fl_vs_action", true);
                    self.flags &= !INTRUDE_DONE;
                }
            }
            ST_VS => {
                if self.flags & (FACES_READY | PANEL_DONE) == FACES_READY | PANEL_DONE {
                    self.state = ST_CLOSING;
                    self.flash[2].play(1);
                    self.flags &= !PANEL_DONE;
                }
            }
            ST_CLOSING => {
                self.state = ST_WAIT;
                self.wait = 300;
            }
            ST_WAIT => {
                self.wait -= 1;
                if ok || self.wait == 0 {
                    sounds.push(TitleSound::BgmFadeOutStep);
                    self.flags |= CHOSEN | LEAVING;
                    self.timer = 15;
                    if ok {
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                    self.ub_flags |= FOUGHT;
                }
            }
            ST_RESULT => {
                if self.flags & PANEL_DONE != 0 {
                    self.flags &= !PANEL_DONE;
                    if self.ub_flags & UPWARD != 0 && self.won {
                        save.set_ub_rank(save.ub_rank() - 1);
                    }
                    let rank = save.ub_rank();
                    if self.climb != 0 && rank != 0 {
                        self.set_foe(Self::place_to_rule(rank - 1, save), false);
                        self.climb -= 1;
                        if !self.scroll_up(sounds) {
                            self.flash[1].goto_label("fl_win_player", true);
                        } else {
                            self.choice -= 1;
                            self.state = ST_CLIMB;
                        }
                    } else {
                        self.state = ST_LIST;
                    }
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

    struct Pictures;

    impl PictureLoader for Pictures {
        fn portrait(&mut self, file_id: u32) -> Option<usize> {
            Some(file_id as usize)
        }

        fn picture(&mut self, _: u32) -> Option<usize> {
            None
        }
    }

    /// Runs the real ladder from the ISO in `rom/`, when it is there: from place 100 the player challenges place
    /// 99, the versus panel and the closing movie set the battle up; back from it with a win the player takes
    /// place 99.
    #[test]
    fn real_ub_rank_climbs_a_place() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let file = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(&file).unwrap();
        let assets = UbRankAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = UbRankLayout::new(0, |k| firsts[&k]);
        let mut save = SaveData::default();
        assert_eq!(save.ub_rank(), 99);
        let player = Pick {
            picked: 0,
            ..Pick::default()
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[9, 9]);
        let mut screen = UbRankScreen::new(&data, player, 0, 0, 0, &save, &mut libc).unwrap();
        assert_eq!(screen.top, PLACES - ROWS);
        let mut font = Font::new(Vec::new(), 20);
        macro_rules! press {
            ($screen:expr, $bits:expr, $n:expr) => {
                for i in 0..$n {
                    let b = if i == 0 { $bits } else { 0 };
                    $screen.run_frame(
                        b,
                        b,
                        VOICE_PLAYEND,
                        &mut save,
                        &mut Pictures,
                        &mut random,
                        &mut libc,
                        &mut font,
                    );
                    if $screen.result().is_some() {
                        break;
                    }
                }
            };
        }
        for _ in 0..4 {
            press!(screen, 0, 40);
        }
        assert_eq!(screen.cursor, 4);
        press!(screen, PAD_UP, 20);
        assert_eq!(screen.cursor, 3);
        press!(screen, PAD_CROSS, 40);
        assert_eq!(screen.choice, 98);
        assert!(screen.ub_flags & UPWARD != 0);
        for _ in 0..20 {
            press!(screen, PAD_CROSS, 30);
            if screen.result().is_some() {
                break;
            }
        }
        let Some(Some(setup)) = screen.result().cloned() else {
            panic!("no battle: state {}", screen.state);
        };
        let (flags, choice) = screen.progress();
        assert!(flags & FOUGHT != 0);
        assert_eq!(setup.mode, 2);
        assert_eq!(setup.sides[1].chara, screen.cur_foe.chara);
        // Back from a won battle.
        let mut screen =
            UbRankScreen::new(&data, player, flags, choice, 1, &save, &mut libc).unwrap();
        for _ in 0..12 {
            press!(screen, 0, 30);
        }
        assert_eq!(save.ub_rank(), 98);
        assert_eq!(screen.state, ST_LIST);
        assert!(screen.progress().0 & FOUGHT == 0);
    }
}
