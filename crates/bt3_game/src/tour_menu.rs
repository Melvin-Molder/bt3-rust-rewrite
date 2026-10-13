//! Dragon World Tour's menu (`TourMenu_Run(1)`, src/menu/tour_menu.c, progress mode 33, `Tour_Main`): Mr. Satan
//! as guide, the mode's clock (each tournament is held at some hours, its invitation arriving once with a random
//! level), and the choice of the open tournament (for prizes, at its level, one entrant) or free play (any of the
//! five, any level, one to eight entrants), each tournament described in two lines.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::PictureLoader;
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::windows::{IconWin, MsgWin};

/// `gMenuArc4`: file `gProgress->baseFile + 2`; `TourMenu_Run(1)` unpacks its section 1, `EntrySel_Run(2)` section 2.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 2;
pub const ARCHIVE_SECTION: usize = 1;
/// The music of mode 33.
pub const BGM_FILE_ID: u32 = 0x10B19;
/// `TOUR_VOICE_BASE`.
pub const VOICE_BASE: u32 = 0x85D3;
/// `TourMenu_Init`'s stream sound and the invitation pictures (file 0x3C4 + tournament).
const STREAM_SE: u32 = 0x10BDF;
const INVITE_FILE: u32 = 0x3C4;
const FADE_FRAMES: i32 = 0x14;

// TOUR_* and the save's tourFlags.
pub const TOUR_NUM: i32 = 5;
const SAVE_INVITE_MASK: i32 = 0x1F;
const SAVE_STARTED: i32 = 0x20;
const SAVE_EXPLAINED: i32 = 0x40;
/// `ESEL_ENTRY_MAX`.
const ENTRY_MAX: i32 = 8;

// TOURMENU_LV_*.
const LV_TOP: usize = 0;
const LV_TOUR: usize = 1;
const LV_INFO: usize = 2;
const LV_LEVEL: usize = 3;
const LV_NUM: usize = 4;

// TOURMENU_*.
const DONE: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;
const INVITE: u32 = 0x10;

/// `gTourMenu->tex[n]` of `TourMenu_Init`.
const TEX: [ImageTexture; 31] = [
    tex(2, 1),
    tex(2, 2),
    tex(2, 3),
    tex(2, 0),
    tex(3, 0),
    tex(3, 3),
    tex(3, 1),
    tex(4, 0),
    tex(5, 0),
    tex(4, 2),
    tex(4, 1),
    tex(5, 1),
    tex(12, 1),
    tex(25, 1),
    tex(25, 0),
    tex(25, 2),
    tex(12, 2),
    tex(10, 0),
    tex(9, 0),
    None,
    tex(11, 0),
    tex(13, 0),
    tex(8, 0),
    tex(8, 1),
    tex(8, 2),
    tex(7, 0),
    tex(14, 0),
    tex(6, 0),
    tex(13, 1),
    tex(26, 1),
    tex(26, 0),
];
/// `texB[n]` of the invitation movie: 0..2 and 5 from sections 16 / 17; 3 and 4 the invitation picture's.
const TEX_B: [ImageTexture; 6] = [tex(17, 1), tex(17, 0), tex(16, 0), None, None, tex(16, 1)];
/// `IconWin_Init(MPACK_AT(res, 21), res 22)` and `MsgWin_Init(MPACK_AT(res, 20), …)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((21, 1), 1)),
    Some(((21, 1), 0)),
    tex(22, 0),
    Some(((21, 1), 2)),
    tex(22, 1),
];
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((20, 1), 0)), Some(((20, 1), 1)), None];

/// `TourEntrant` (gProgress + 0x98, 0x28 bytes each): one entrant of the tournament.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TourEntrant {
    /// `TOUR_ENT_*`.
    pub flags: u16,
    pub chara: i32,
    pub costume: i32,
    /// Index of the entrant among the players' choices.
    pub player: i32,
    /// Where its chip sits in the tree.
    pub pos: i32,
    /// Health left after its last battle.
    pub health: f32,
    /// 1-based item ids.
    pub items: [u16; 8],
}

/// `TOUR_ENT_PLAYER`, `TOUR_ENT_REC`, `TOUR_ENT_ITEMS`.
pub const ENT_PLAYER: u16 = 1;
pub const ENT_REC: u16 = 0x10;
pub const ENT_ITEMS: u16 = 0x20;
/// `TOUR_ENTRANT_MAX`.
pub const ENTRANT_MAX: usize = 17;

/// The tournament's block of `gProgress` (`TourSession`, 0x7C..0x440): what the menu chose, and the entrants.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TourSession {
    /// 0 the open tournament (prizes), 1 free play.
    pub entry: i32,
    pub tour: i32,
    pub level: i32,
    /// Entrants the players choose, 1..8.
    pub entry_num: i32,
    pub entrants: [TourEntrant; ENTRANT_MAX],
}

/// What `TourMenu_Init` reads from its unpacked section.
pub struct TourMenuAssets {
    pub data: TourMenuData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct TourMenuData {
    pub fods: [Vec<u8>; 2],
    pub icon_fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    /// Section 19: the guide's lines; 23: their mouth keys.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    /// Section 27, `TourInfo.prize[2][3]` of each tournament: the two prize sums at the three levels.
    pub prizes: Vec<[[i32; 3]; 2]>,
    pub layout: TourMenuLayout,
}

#[derive(Clone, Debug, Default)]
pub struct TourMenuLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub tex_b: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
}

impl TourMenuLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&TEX, &first),
            tex_b: image_indices(&TEX_B, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            msg: image_indices(&MSG_TEXTURES, &first),
        }
    }
}

impl TourMenuAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&TEX[..], &TEX_B, &ICON_TEXTURES, &MSG_TEXTURES])?;
        let info = pack_section(res, 27)?;
        let word = |at: usize| -> Result<i32, Box<dyn Error>> {
            Ok(i32::from_le_bytes(
                info.get(at..at + 4)
                    .ok_or("short tournament info")?
                    .try_into()?,
            ))
        };
        let prizes = (0..TOUR_NUM as usize)
            .map(|t| {
                let at = t * 0x64 + 0x10;
                Ok([
                    [word(at)?, word(at + 4)?, word(at + 8)?],
                    [word(at + 12)?, word(at + 16)?, word(at + 20)?],
                ])
            })
            .collect::<Result<_, Box<dyn Error>>>()?;
        Ok(Self {
            data: TourMenuData {
                fods: [
                    pack_section(res, 15)?.to_vec(),
                    pack_section(res, 18)?.to_vec(),
                ],
                icon_fod: pack_section(pack_section(res, 21)?, 2)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 20)?, 2)?.to_vec(),
                msg_text: pack_section(res, 19)?.to_vec(),
                lips: pack_section(res, 23)?.to_vec(),
                prizes,
                layout: TourMenuLayout::default(),
            },
            background: decode_picture(res, 1)?,
            files,
        })
    }
}

/// `TourMenu` and the frame loop of `TourMenu_Run`.
pub struct TourMenuScreen {
    layout: TourMenuLayout,
    lips: Vec<u8>,
    prizes: Vec<[[i32; 3]; 2]>,
    flash: [Flash; 2],
    tex: [Vec<Option<usize>>; 2],
    icon: IconWin,
    msg: MsgWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    flags: u32,
    cursor: [i32; 5],
    leave_timer: i32,
    icon_timer: i32,
    icon_frame: i32,
    level: usize,
    seq: i32,
    voice_line: i32,
    blink: i32,
    talk: i32,
    result: i32,
    session: TourSession,
    done: Option<i32>,
}

impl TourMenuScreen {
    /// `TourMenu_Init(1)` (the clock starts at 7 on the first visit; the hour's tournament invites once) and the
    /// first `ColorFade_StartIn`.
    pub fn new(
        data: &TourMenuData,
        session: &TourSession,
        save: &mut SaveData,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) -> Result<Self, Box<dyn Error>> {
        if save.tour_flags() & SAVE_STARTED == 0 {
            save.set_tour_hour(7);
        }
        let mut flags = 0;
        let mut invite = 0;
        // TourMenu_CheckInvite: which tournament the hour holds.
        let tour = match save.tour_hour() {
            7..=12 => Some(0),
            13..=18 => Some(1),
            19..=23 => Some(2),
            0..=4 => Some(3),
            5..=6 => Some(4),
            _ => None,
        };
        if let Some(n) = tour
            && save.tour_flags() & (1 << n) == 0
        {
            flags |= INVITE;
            invite = n;
            save.set_tour_flags((save.tour_flags() & !SAVE_INVITE_MASK) | 1 << n);
            save.set_tour_level(random.range(3) as i32);
        }
        let mut f0 = Flash::create(&data.fods[0])?;
        f0.play(1);
        let mut tex_b = data.layout.tex_b.clone();
        if flags & INVITE != 0 {
            // File_LoadSync of the invitation: textures 0 and 1 for images 4 and 3.
            if let Some(first) = loader.texture_file(INVITE_FILE + invite as u32) {
                tex_b[4] = Some(first);
                tex_b[3] = Some(first + 2);
            }
        }
        // The invitation movie waits to be played.
        let f1 = Flash::create(&data.fods[1])?;
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            false,
        )?;
        msg.open();
        sounds.push(TitleSound::StreamSe(STREAM_SE));
        Ok(Self {
            layout: data.layout.clone(),
            lips: data.lips.clone(),
            prizes: data.prizes.clone(),
            flash: [f0, f1],
            tex: [data.layout.flash.clone(), tex_b],
            icon,
            msg,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags,
            cursor: [0; 5],
            leave_timer: 0,
            icon_timer: 0,
            icon_frame: 0,
            level: LV_TOP,
            seq: 0,
            voice_line: -1,
            blink: 0,
            talk: 0,
            result: 1,
            session: session.clone(),
            done: None,
        })
    }

    /// `TourMenu_Run`'s result once it has returned: 1 on to the entrant select, 0 back to the main menu.
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// The choices stored in `gProgress` as the menu leaves.
    pub fn session(&self) -> TourSession {
        self.session.clone()
    }

    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        self.voice_line = line;
        anim::play_with_subtitle(&self.lips, VOICE_BASE, line, &mut self.lip, sounds);
    }

    /// `Voice_StopWithLip`.
    fn stop_voice(&mut self, sounds: &mut Vec<TitleSound>) {
        sounds.push(TitleSound::VoiceStop);
        self.lip = LipSync::default();
    }

    /// `TourMenu_ClipGoto(0, kind, label)`: the plate under the cursor of a level.
    fn clip_goto(&mut self, kind: usize, label: &str) {
        let name = match kind {
            LV_TOP => format!("mc_menu_plate_{:02}", self.cursor[kind] + 1),
            LV_TOUR => format!("mc_taikai_plate_{:02}", self.cursor[kind] + 1),
            LV_INFO => "mc_taikaititle_plate".to_owned(),
            LV_LEVEL => format!("mc_dragonball_off_{:02}", self.cursor[kind] + 1),
            // TOURMENU_LV_NUM leaves the name as it was: no clip of that name.
            _ => String::new(),
        };
        let r = self.flash[0].find(None, &name);
        self.flash[0].clip_goto_label(r, label);
    }

    /// The opening of the menu: line 3 and the plates in.
    fn open_menu(&mut self, sounds: &mut Vec<TitleSound>) {
        self.say(3, sounds);
        self.flash[0].goto_label("fl_menu_in", true);
        self.clip_goto(LV_TOP, "fl_on_start");
    }

    /// One iteration of `TourMenu_Run`'s loop.
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
        if self.done.is_some() {
            return frame;
        }
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        // TourMenu_Update.
        if self.flags & GREETED != 0 && self.voice_line == 3 && voice_stat == VOICE_PLAYEND {
            self.say(self.cursor[LV_TOP] + 0xE, &mut frame.sounds);
        }
        for f in &mut self.flash {
            f.advance();
        }
        self.update_seq(pressed, voice_stat, save, &mut frame.sounds);
        self.draw(save, voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.flags & GREETED == 0 && self.flash[0].flags() & FLASH_PAD != 0 {
            self.flags |= GREETED;
            if save.tour_flags() & SAVE_STARTED == 0 {
                self.seq = 1;
            } else if self.flags & INVITE != 0 {
                self.seq = 0x33;
            } else {
                self.open_menu(&mut frame.sounds);
            }
        }
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::FadeOutStep);
            frame.sounds.push(TitleSound::StreamSeFadeOutStep(0));
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = Some(self.result);
            return frame;
        }
        if self.flags & LEAVING != 0 {
            self.leave_timer -= 1;
            if self.leave_timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.session.entry = self.cursor[LV_TOP];
                self.session.tour = self.cursor[LV_TOUR];
                self.session.level = self.cursor[LV_LEVEL];
                self.session.entry_num = self.cursor[LV_NUM] + 1;
            }
        } else if self.seq == 0 {
            self.input(pressed, repeat, save, &mut frame.sounds);
        }
        frame
    }

    /// `TourMenu_Draw`.
    fn draw(
        &mut self,
        save: &SaveData,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let uv = |u0: i32, v0: i32, u1: i32, v1: i32| Uv {
            u0,
            v0,
            u1,
            v1,
            tex: 0,
        };
        let cur = self.cursor;
        let f = &mut self.flash[0];
        let eye = f.find(None, "mc_guide_satan_eye");
        anim::blink(f, eye, &mut self.blink, 0, random);
        let mouth = f.find(None, "mc_guide_satan_mouth");
        anim::talk(
            f,
            mouth,
            &mut self.talk,
            0,
            &mut self.lip,
            voice_stat,
            random,
        );
        // The clock.
        anim::num_draw_child(
            f,
            |n| ("mc_timer".to_owned(), format!("mc_timer_num{:02}", n + 2)),
            2,
            save.tour_hour(),
            (0x40, 0x40),
            1,
        );
        // The two prizes of the tournament under the cursor, at the open tournament's level.
        let tour = cur[LV_TOUR].clamp(0, TOUR_NUM - 1) as usize;
        let lvl = save.tour_level().clamp(0, 2) as usize;
        let mode = if cur[LV_TOP] == 1 { 2 } else { 0 };
        for i in 0..2 {
            let prize = self.prizes.get(tour).map_or(0, |p| p[i][lvl]);
            anim::num_draw_child(
                f,
                |n| {
                    (
                        format!("mc_menu_text_zenny{i}"),
                        format!("menu_text_num{n}"),
                    )
                },
                7,
                prize,
                (0x20, 0x20),
                mode,
            );
        }
        for i in 0..2 {
            let name = format!("mc_menu_plate_{:02}", i + 1);
            for part in ["mc_menu_text_off", "mc_menu_text_on"] {
                let r = f.find(Some(&name), part);
                f.clip_set_uv(r, uv(0, i * 0x20, 0x200, i * 0x20 + 0x20));
            }
        }
        let ship = f.find(None, "mc_icon_ship");
        anim::sheet(
            f,
            ship,
            &mut self.icon_timer,
            &mut self.icon_frame,
            uv(0, 0, 0x40, 0x40),
            4,
            1,
            4,
        );
        for i in 0..TOUR_NUM {
            let name = format!("mc_taikai_plate_{:02}", i + 1);
            let plate = f.find(None, &name);
            let dim = cur[LV_TOP] != 1 && save.tour_flags() & (1 << i) == 0;
            f.clip_set_color(plate, if dim { 0.5 } else { 1.0 });
            for part in ["mc_taikai_text_off", "mc_taikai_text_on"] {
                let r = f.find(Some(&name), part);
                f.clip_set_uv(r, uv(0, i * 0x20 + 0x40, 0x200, i * 0x20 + 0x60));
            }
            let r = f.find(Some(&name), "mc_taikai_icon_s");
            f.clip_set_uv(r, uv(i * 0x40, 0, i * 0x40 + 0x40, 0x40));
        }
        let t = cur[LV_TOUR];
        let r = f.find(Some("mc_taikaititle_plate"), "mc_taikaititle_text");
        f.clip_set_uv(r, uv(0, (t % 4) * 0x40, 0x200, (t % 4) * 0x40 + 0x40));
        f.clip_set_tex(r, t / 4);
        let r = f.find(Some("mc_taikaititle_plate"), "mc_plate_icon_mark");
        f.clip_set_uv(
            r,
            uv(
                (t % 4) * 0x40,
                (t / 4) * 0x40,
                (t % 4) * 0x40 + 0x40,
                (t / 4) * 0x40 + 0x40,
            ),
        );
        let r = f.find(None, "mc_taikai_icon");
        f.clip_set_tex(r, t);
        for i in 0..2 {
            let name = if i != 0 {
                "mc_yajirusi_l"
            } else {
                "mc_yajirusi_r"
            };
            let r = f.find(Some("mc_yajirusi_u_l"), name);
            f.clip_set_uv(r, uv(i * 0x40, 0, i * 0x40 + 0x40, 0x40));
            f.clip_set_flags(r, PROP_VISIBLE, cur[LV_TOP] == 1);
        }
        for i in 0..3 {
            let name = format!("mc_dragonball_off_{:02}", i + 1);
            for part in ["mc_dragonball_off", "mc_dragonball_on"] {
                let r = f.find(Some(&name), part);
                f.clip_set_uv(r, uv(i * 0x40, 0, i * 0x40 + 0x40, 0x40));
            }
        }
        let n = cur[LV_NUM] + 1;
        let r = f.find(None, "mc_suji_01");
        f.clip_set_uv(
            r,
            uv(
                (n % 4) * 0x40,
                (n / 4) * 0x40,
                (n % 4) * 0x40 + 0x40,
                (n / 4) * 0x40 + 0x40,
            ),
        );
        for i in 0..2 {
            let r = f.find(None, &format!("mc_plate_text_01_{}", i + 1));
            let v0 = (i ^ 1) * 0x20;
            f.clip_set_uv(r, uv(0, v0, 0x100, v0 + 0x20));
        }
        for i in 1..3 {
            let r = f.find(None, &format!("mc_menu_text_01_{i}"));
            f.clip_set_uv(r, uv(0, i * 0x20, 0x100, i * 0x20 + 0x20));
        }
        for i in 0..2 {
            let tex = &self.tex[i];
            let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
            quads.extend(screen_quads(&self.flash[i].draw(&has_texture), tex));
        }
        self.icon.draw(quads);
        self.msg.draw(self.voice_line, font, quads);
    }

    /// `TourMenu_Input`: the five levels; confirming the last starts the send-off (seq 0x1C3).
    fn input(&mut self, pressed: u32, repeat: u32, save: &SaveData, sounds: &mut Vec<TitleSound>) {
        if self.flash[0].flags() & FLASH_PAD == 0 {
            return;
        }
        self.flags |= STARTED;
        let lv = self.level;
        match lv {
            LV_TOP => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    self.clip_goto(LV_TOP, "fl_off_start");
                    self.cursor[lv] = (self.cursor[lv] + 1) % 2;
                    self.clip_goto(LV_TOP, "fl_on_start");
                    self.say(self.cursor[lv] + 0xE, sounds);
                    sounds.push(TitleSound::Effect(1, 0));
                } else if pressed & PAD_CROSS != 0 {
                    self.flash[0].goto_label("fl_taikaisentaku_in", true);
                    self.clip_goto(LV_TOP, "fl_ok");
                    self.clip_goto(LV_TOUR, "fl_on_start");
                    if self.cursor[lv] == 0 {
                        self.say(0x11, sounds);
                    }
                    self.level = LV_TOUR;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.result = 0;
                    self.flash[0].goto_label("fl_menu_cansel", true);
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            LV_TOUR => {
                if repeat & (PAD_UP | PAD_DOWN) != 0 {
                    self.clip_goto(LV_TOUR, "fl_off_start");
                    let c = &mut self.cursor[lv];
                    if repeat & PAD_UP != 0 {
                        *c -= 1;
                        if *c < 0 {
                            *c = TOUR_NUM - 1;
                        }
                    } else {
                        *c += 1;
                        if *c >= TOUR_NUM {
                            *c = 0;
                        }
                    }
                    self.clip_goto(LV_TOUR, "fl_on_start");
                    sounds.push(TitleSound::Effect(1, 0));
                } else if pressed & PAD_CROSS != 0 {
                    // Only the open tournament can be entered for prizes.
                    let open = save.tour_flags() & (1 << self.cursor[LV_TOUR]) != 0;
                    if self.cursor[LV_TOP] == 1 || open {
                        self.flash[0].goto_label("fl_taikai_setumei_in", true);
                        self.clip_goto(LV_TOUR, "fl_ok");
                        self.clip_goto(LV_INFO, "fl_on_start");
                        self.seq = self.cursor[LV_TOUR] * 50 + 151;
                        self.level = LV_INFO;
                        sounds.push(TitleSound::Effect(1, 1));
                    } else {
                        self.say(0x1C, sounds);
                        sounds.push(TitleSound::Effect(1, 7));
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[0].goto_label("fl_taikaisentaku_cancel", true);
                    self.clip_goto(LV_TOUR, "fl_off_start");
                    self.clip_goto(LV_TOP, "fl_on_start");
                    self.level = LV_TOP;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            LV_INFO => {
                if pressed & PAD_CROSS != 0 {
                    self.flash[0].goto_label("fl_nanidoi_in", true);
                    let mut line = 0x1D;
                    if self.cursor[LV_TOP] != 1 {
                        // The open tournament has its level fixed.
                        line += 1;
                        self.cursor[LV_LEVEL] = save.tour_level();
                    }
                    self.clip_goto(LV_LEVEL, "fl_on_start");
                    self.say(line, sounds);
                    self.level = LV_LEVEL;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[0].goto_label("fl_taikai_setumei_cancel", true);
                    self.clip_goto(LV_INFO, "fl_off_start");
                    self.level = LV_TOUR;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            LV_LEVEL => {
                let free = self.cursor[LV_TOP] == 1;
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 && free {
                    self.clip_goto(LV_LEVEL, "fl_off_start");
                    let c = &mut self.cursor[lv];
                    if repeat & PAD_LEFT != 0 {
                        *c -= 1;
                        if *c < 0 {
                            *c = 2;
                        }
                    } else {
                        *c += 1;
                        if *c >= 3 {
                            *c = 0;
                        }
                    }
                    self.clip_goto(LV_LEVEL, "fl_on_start");
                    sounds.push(TitleSound::Effect(1, 0));
                } else if pressed & PAD_CROSS != 0 {
                    self.flash[0].goto_label("fl_ninzu_in", true);
                    let mut line = 0x1F;
                    if !free {
                        // And a single entrant.
                        line += 1;
                        self.cursor[LV_NUM] = 0;
                    }
                    self.clip_goto(LV_LEVEL, "fl_ok");
                    self.say(line, sounds);
                    self.level = LV_NUM;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[0].goto_label("fl_nanidoi_cancel", true);
                    self.clip_goto(LV_LEVEL, "fl_off_start");
                    self.seq = self.cursor[LV_TOUR] * 50 + 151;
                    self.level = LV_INFO;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            LV_NUM => {
                let free = self.cursor[LV_TOP] == 1;
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 && free {
                    let c = &mut self.cursor[lv];
                    if repeat & PAD_LEFT != 0 {
                        *c -= 1;
                        if *c < 0 {
                            *c = ENTRY_MAX - 1;
                        }
                    } else {
                        *c += 1;
                        if *c >= ENTRY_MAX {
                            *c = 0;
                        }
                    }
                    sounds.push(TitleSound::Effect(1, 0));
                } else if pressed & PAD_CROSS != 0 {
                    self.seq = 451;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[0].goto_label("fl_ninzu_cancel", true);
                    let line = if free { 0x1D } else { 0x1E };
                    self.say(line, sounds);
                    self.level = LV_LEVEL;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            _ => {}
        }
    }

    /// `TourMenu_UpdateSeq`: the guide's scripted speech; 0 idle.
    fn update_seq(
        &mut self,
        pressed: u32,
        voice_stat: i32,
        save: &mut SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        let idle = voice_stat == VOICE_PLAYEND;
        // A line to wait for: it ends, or confirm skips it.
        let wait = |s: &mut Self, sounds: &mut Vec<TitleSound>| {
            if idle {
                s.seq += 1;
            } else if pressed & PAD_CROSS != 0 {
                s.seq += 1;
                sounds.push(TitleSound::Effect(1, 1));
            }
        };
        match self.seq {
            0 => {}
            // 1..7: the first visit, three lines.
            1 => {
                self.seq += 1;
                self.say(0, sounds);
            }
            3 | 5 => {
                self.seq += 1;
                self.say(self.voice_line + 1, sounds);
            }
            7 => {
                save.set_tour_flags(save.tour_flags() | SAVE_STARTED);
                if self.flags & INVITE != 0 {
                    self.voice_line = -1;
                    self.stop_voice(sounds);
                    self.seq = 0x33;
                } else {
                    self.open_menu(sounds);
                    self.seq = 0;
                }
            }
            2 | 4 | 6 | 0x36 | 0x66 | 0x68 | 0x1C4 => wait(self, sounds),
            // 0x33..0x39: an invitation arrived.
            0x33 => {
                self.flash[1].play(1);
                self.flash[1].goto_label("fl_taikai_syotai_in", true);
                self.seq += 1;
            }
            0x34 | 0x38 => {
                if self.flash[1].trig() & 1 != 0 {
                    self.seq += 1;
                }
            }
            0x35 => {
                self.seq += 1;
                self.say(4, sounds);
            }
            0x37 => {
                self.flash[1].goto_label("fl_taikai_syotai_out", true);
                self.seq += 1;
            }
            0x39 => {
                if save.tour_flags() & SAVE_EXPLAINED == 0 {
                    self.seq = 0x65;
                } else {
                    self.open_menu(sounds);
                    self.seq = 0;
                }
            }
            // 0x65..0x69: the long explanation, once.
            0x65 => {
                self.seq += 1;
                self.say(5, sounds);
            }
            0x67 => {
                self.seq += 1;
                self.say(9, sounds);
            }
            0x69 => {
                save.set_tour_flags(save.tour_flags() | SAVE_EXPLAINED);
                self.open_menu(sounds);
                self.seq = 0;
            }
            // 151 + 50 * tournament: its description, two lines; cancel goes back.
            0x97 | 0xC9 | 0xFB | 0x12D | 0x15F => {
                let t = (self.seq - 0x97) / 50;
                self.seq += 1;
                self.say(0x12 + t * 2, sounds);
            }
            0x98 | 0xCA | 0xFC | 0x12E | 0x160 => {
                if idle {
                    self.seq += 1;
                } else if pressed & PAD_CROSS != 0 {
                    self.seq += 1;
                    sounds.push(TitleSound::Effect(1, 1));
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.seq = 0x191;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            0x99 | 0xCB | 0xFD | 0x12F | 0x161 => {
                self.seq = 0;
                self.say(self.voice_line + 1, sounds);
            }
            0x191 => {
                self.seq = 0;
                self.voice_line = -1;
                self.stop_voice(sounds);
                self.flash[0].goto_label("fl_taikai_setumei_cancel", true);
                self.clip_goto(LV_INFO, "fl_off_start");
                self.level = LV_TOUR;
            }
            // 0x1C3..0x1C5: all is chosen: the send-off, then leave.
            0x1C3 => {
                self.seq += 1;
                self.say(0x21, sounds);
            }
            0x1C5 => {
                self.seq = 0;
                self.flags |= DONE | LEAVING;
                self.leave_timer = 0xF;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Runs the real tournament menu from the ISO in `rom/`, when it is there, on a new save: the first visit's
    /// lines, the invitation of the hour's tournament (the World Tournament at 7) and the explanation, then free
    /// play of the Cell Games at level 2 with three entrants.
    #[test]
    fn real_tournament_menu_sets_up_free_play() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = TourMenuAssets::parse(&res).unwrap();
        assert!(
            assets.data.prizes.iter().all(|p| p[0][0] > 0),
            "{:?}",
            assets.data.prizes
        );
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = TourMenuLayout::new(0, |k| firsts[&k]);
        let mut save = SaveData::default();
        let mut random = GameRandom::from_key(&[4, 2]);
        let mut font = Font::new(Vec::new(), 20);
        let mut screen = TourMenuScreen::new(
            &data,
            &TourSession::default(),
            &mut save,
            &mut Pictures,
            &mut random,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(save.tour_hour(), 7);
        assert_eq!(save.tour_flags() & SAVE_INVITE_MASK, 1);
        assert_eq!(screen.tex[1][4], Some(INVITE_FILE as usize));
        let lines = std::cell::RefCell::new(std::collections::BTreeSet::new());
        let mut press = |screen: &mut TourMenuScreen, save: &mut SaveData, bits: u32| {
            for i in 0..60 {
                let b = if i == 0 { bits } else { 0 };
                screen.run_frame(b, b, VOICE_PLAYEND, save, &mut random, &mut font);
                lines.borrow_mut().insert(screen.voice_line);
                if screen.result().is_some() {
                    break;
                }
            }
        };
        for _ in 0..20 {
            press(&mut screen, &mut save, 0);
        }
        // The first visit (0..2), the invitation (4), the explanation (5, 9), the menu (3, then the top entry).
        for l in [0, 1, 2, 4, 5, 9] {
            assert!(lines.borrow().contains(&l), "{:?}", lines.borrow());
        }
        assert_eq!(
            save.tour_flags() & (SAVE_STARTED | SAVE_EXPLAINED),
            SAVE_STARTED | SAVE_EXPLAINED
        );
        assert_eq!(screen.seq, 0);
        // Free play; the Cell Games; its two lines; level 2; three entrants.
        press(&mut screen, &mut save, PAD_DOWN);
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_TOUR);
        press(&mut screen, &mut save, PAD_DOWN);
        press(&mut screen, &mut save, PAD_DOWN);
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_INFO);
        for _ in 0..3 {
            press(&mut screen, &mut save, 0);
        }
        assert!(lines.borrow().contains(&0x16) && lines.borrow().contains(&0x17));
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_LEVEL);
        press(&mut screen, &mut save, PAD_LEFT);
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_NUM);
        press(&mut screen, &mut save, PAD_RIGHT);
        press(&mut screen, &mut save, PAD_RIGHT);
        press(&mut screen, &mut save, PAD_CROSS);
        for _ in 0..5 {
            press(&mut screen, &mut save, 0);
        }
        assert_eq!(screen.result(), Some(1));
        let s = screen.session();
        assert_eq!((s.entry, s.tour, s.level, s.entry_num), (1, 2, 2, 3));
    }
}
