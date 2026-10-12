use std::error::Error;

use bt3_data::gs::{RgbaTexture, decode_sprite_texture};
use bt3_data::pack_section;
use bt3_data::tex::TextureResource;

use crate::dialog::{Dialog, DialogAssets, Pad};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_REPEAT, Uv, screen_quad};
use crate::font::Font;
use crate::logo::{ColorFade, TILES_PER_PICTURE, picture_quads};
use crate::mcflow::{self, McFlow};
use crate::rng::GameRandom;

/// `gProgress->baseFile + 1` (gMenuArc1); `Title_Run(1)` unpacks its section 1.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 1;
pub const ARCHIVE_SECTION: usize = 1;
/// `MENU_BGM_TITLE`, which `Progress_Main` starts before `Title_Run`.
pub const MUSIC_FILE_ID: u32 = 0x10B16;
/// `TITLE_VOICE_BASE` plus `VOICE_LANG2_OFFSET`: a fresh save has flag bit 0 set.
const VOICE_BASE: u32 = 0x86F9 + 0x55C;
const IDLE_FRAMES: i32 = 0x708;
const LOGO_WAIT: i32 = 300;
const FADE_FRAMES: i32 = 0x14;
const LEAVE_FRAMES: i32 = 15;

pub const PAD_LEFT: u32 = 0x0001;
pub const PAD_RIGHT: u32 = 0x0002;
pub const PAD_DOWN: u32 = 0x0004;
pub const PAD_UP: u32 = 0x0008;
pub const PAD_CROSS: u32 = 0x0200;
pub const PAD_START: u32 = 0x1000;
pub const PAD_TRIANGLE: u32 = 0x0400;
// Pad_SetRepeat(0x28, 3) in Progress_Main.
const REPEAT_DELAY: i32 = 0x28;
const REPEAT_INTERVAL: i32 = 3;

/// `gTitle->tex[n]` of `Title_Init`: (texture section, entry); images 24 and 25 have none and are not drawn.
const FLASH_TEXTURES: [Option<(usize, usize)>; 26] = [
    Some((10, 0)),
    Some((10, 1)),
    Some((10, 2)),
    Some((7, 0)),
    Some((6, 0)),
    Some((9, 0)),
    Some((8, 0)),
    Some((9, 2)),
    Some((9, 1)),
    Some((8, 1)),
    Some((3, 0)),
    Some((3, 1)),
    Some((3, 2)),
    Some((3, 3)),
    Some((3, 4)),
    Some((3, 5)),
    Some((3, 6)),
    Some((2, 4)),
    Some((2, 0)),
    Some((2, 5)),
    Some((2, 1)),
    Some((2, 3)),
    Some((2, 2)),
    Some((5, 0)),
    None,
    None,
];

/// What `Title_Init` reads from its unpacked section.
pub struct TitleAssets {
    pub fod: Vec<u8>,
    /// Section 1 (`gTitle->bg`), drawn with `Sprite_DrawPicture`.
    pub background: Vec<RgbaTexture>,
    /// Section 14 (`gTitle->logo`).
    pub logo: Vec<RgbaTexture>,
    /// The movie's textures by image index.
    pub flash: Vec<Option<RgbaTexture>>,
    /// Section 12, `Dialog_Init`'s file.
    pub dialog: DialogAssets,
}

impl TitleAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let picture = |section: usize| -> Result<Vec<RgbaTexture>, Box<dyn Error>> {
            let tiles = decode_section(res, section)?;
            if tiles.len() != TILES_PER_PICTURE {
                return Err(
                    format!("title picture section {section} has {} tiles", tiles.len()).into(),
                );
            }
            Ok(tiles)
        };
        let mut sections = std::collections::BTreeMap::new();
        let mut flash = Vec::new();
        for slot in FLASH_TEXTURES {
            flash.push(match slot {
                Some((section, entry)) => {
                    if !sections.contains_key(&section) {
                        sections.insert(section, decode_section(res, section)?);
                    }
                    Some(
                        sections[&section].get(entry).cloned().ok_or_else(|| {
                            format!("title section {section} has no texture {entry}")
                        })?,
                    )
                }
                None => None,
            });
        }
        Ok(Self {
            fod: pack_section(res, 11)?.to_vec(),
            background: picture(1)?,
            logo: picture(14)?,
            flash,
            dialog: DialogAssets::parse(pack_section(res, 12)?)?,
        })
    }
}

fn decode_section(res: &[u8], section: usize) -> Result<Vec<RgbaTexture>, Box<dyn Error>> {
    Ok(TextureResource::parse(pack_section(res, section)?)?
        .entries
        .iter()
        .map(decode_sprite_texture)
        .collect::<Result<Vec<_>, _>>()?)
}

/// Where the title's textures sit in the renderer's texture list.
#[derive(Clone, Debug)]
pub struct TextureLayout {
    pub background: usize,
    pub logo: usize,
    /// Per image: a clamped texture, followed by the same texture with wrapping (`FLASH_QUAD_REPEAT`).
    pub flash: Vec<Option<usize>>,
    /// The dialog movie's textures, the same way.
    pub dialog: Vec<Option<usize>>,
}

/// What a `Title_Run` starts from once the archive is loaded.
#[derive(Clone, Debug)]
pub struct TitleData {
    pub fod: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    pub layout: TextureLayout,
    /// `McFlow_Init`'s card resources (common file base + 0x1D, unpacked): the save icons.
    pub card_res: Option<Vec<u8>>,
}

/// `Pad_CalcRepeat` with `gPad[0]`'s timer and last buttons.
#[derive(Clone, Copy, Debug, Default)]
pub struct PadRepeat {
    timer: i32,
    last: u32,
}

impl PadRepeat {
    pub fn update(&mut self, held: u32, pressed: u32) -> u32 {
        let mut result = 0;
        if held == self.last && held != 0 {
            self.timer -= 1;
            if self.timer < 0 {
                self.timer = REPEAT_INTERVAL;
                result = held;
            }
        } else {
            self.last = held;
            self.timer = REPEAT_DELAY;
        }
        if pressed != 0 {
            result = pressed;
        }
        result
    }
}

/// How `Title_Run` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleResult {
    /// 0: on to the main menu.
    MainMenu,
    /// 1: `Demo_SetupBattle` set up an attract battle (`take_demo`).
    Demo,
    /// 2: the opening movie again.
    OpeningMovie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleSound {
    /// `Voice_Play(0, id, 0x80, 0)`.
    Voice(u32),
    /// `Voice_Stop(0)`.
    VoiceStop,
    /// `Voice_Stop(player)` (players 0 and 1).
    VoiceStopEx(u32),
    /// `MapBgm_Play(id)`: the second music player, looping (volume 0x40 until set).
    MapBgm(u32),
    /// `MapBgm_SetVolume`.
    MapBgmVolume(i32),
    /// `MapBgm_Stop`.
    MapBgmStop,
    /// `Bgm_Play(id)`.
    Bgm(u32),
    /// `Bgm_FadeOutStep()` alone.
    BgmFadeOutStep,
    /// `Voice_FadeOutStep(0)` and `Bgm_FadeOutStep()`.
    FadeOutStep,
    /// `Voice_FadeOutStep(0)` alone.
    VoiceFadeOutStep,
    /// `Snd_PlaySe(bank, id)`.
    Effect(u32, u32),
    /// `Snd_PlaySeEx(mask, id, volume, pan, pitch)`, the sound known by `handle`.
    SeEx {
        mask: u32,
        id: u32,
        volume: i32,
        pan: i32,
        pitch: i32,
        handle: u32,
    },
    /// `Snd_StopHandle`.
    SeStop(u32),
    /// `Snd_StopBank(mask)`.
    SeStopBanks(u32),
    /// `StreamSe_PlayDefault(0, id)`.
    StreamSe(u32),
    /// `StreamSe_Play(player, id, volume, pan)` (players 0 and 1).
    StreamSeEx {
        player: u32,
        id: u32,
        volume: i32,
        pan: i32,
    },
    /// `Voice_Play(player, id, volume, pan)` (players 0 and 1).
    VoiceEx {
        player: u32,
        id: u32,
        volume: i32,
        pan: i32,
    },
    /// `Adx_PauseSeVoice` (true) / `Adx_ResumeSeVoice` (false): the streamed SE and voice players.
    PauseSeVoice(bool),
}

pub struct TitleFrame {
    pub quads: Vec<ScreenQuad>,
    pub sounds: Vec<TitleSound>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The first loop: the still picture.
    Logo {
        wait: i32,
    },
    /// The second loop: the movie and the menu.
    Menu,
    Done(TitleResult),
}

/// `Title_Run(1)` (src/menu/title.c).
pub struct TitleScreen {
    layout: TextureLayout,
    flash: Flash,
    fade: ColorFade,
    phase: Phase,
    faded_in: bool,
    started: bool,
    leaving: bool,
    state: i32,
    cursor: i32,
    item_count: i32,
    voice: u32,
    idle: i32,
    timer: i32,
    result: TitleResult,
    /// `Demo_SetupBattle`'s setup when the screen leaves for the attract battle.
    demo: Option<crate::char_select::BattleSetup>,
    cloud: [f32; 2],
    pad: PadRepeat,
    dialog: Dialog,
    mcflow: McFlow,
    mc_state: i32,
}

impl TitleScreen {
    /// `Title_Init`, then the first `ColorFade_StartIn`.
    pub fn new(
        data: &TitleData,
        save_loaded: bool,
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        // Dialog_Init(MPACK_AT(res, 12), NULL, 0); McFlow_Init (its replay block is not saved from here).
        let dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            None,
            0,
        )?;
        Ok(Self {
            layout: data.layout.clone(),
            flash,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            phase: Phase::Logo { wait: 0 },
            faded_in: false,
            started: false,
            leaving: false,
            state: 0,
            cursor: i32::from(save_loaded),
            item_count: 1 + i32::from(save_loaded),
            voice: random.range(6),
            idle: 0,
            timer: 0,
            result: TitleResult::Demo,
            demo: None,
            cloud: [0.0; 2],
            pad: PadRepeat::default(),
            dialog,
            mcflow: McFlow::new(data.card_res.as_deref(), None),
            mc_state: 0,
        })
    }

    /// The attract battle `Title_Idle` set up.
    pub fn take_demo(&mut self) -> Option<crate::char_select::BattleSetup> {
        self.demo.take()
    }

    pub fn result(&self) -> Option<TitleResult> {
        match self.phase {
            Phase::Done(result) => Some(result),
            _ => None,
        }
    }

    /// One frame of whichever `Title_Run` loop is running. `held` and `pressed` are pad 0's game buttons.
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        demo: &mut crate::char_select::DemoProgress,
        random: &mut GameRandom,
        font: &mut Font,
        mc: &mut mcflow::McEnv,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        let repeat = self.pad.update(held, pressed);
        match self.phase {
            Phase::Logo { wait } => {
                self.fade.update();
                frame.quads.extend(
                    picture_quads(self.layout.logo)
                        .into_iter()
                        .map(ScreenQuad::from),
                );
                frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
                if self.fade.is_in_done() {
                    self.faded_in = true;
                }
                if self.fade.is_fading_out() {
                    return frame;
                }
                if self.fade.is_out_done() {
                    self.phase = Phase::Menu;
                    self.fade = ColorFade::start_in(true, FADE_FRAMES);
                    return frame;
                }
                if self.faded_in {
                    let wait = wait + 1;
                    self.phase = Phase::Logo { wait };
                    if wait > LOGO_WAIT {
                        self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    }
                }
            }
            Phase::Menu => {
                self.fade.update();
                // Title_Update.
                self.flash.advance();
                if self.flash.se() & 1 != 0 {
                    frame.sounds.push(TitleSound::Effect(2, 0x37));
                }
                self.draw(&mut frame.quads);
                let mut effects = Vec::new();
                let mut io = mcflow::McIo {
                    dialog: &mut self.dialog,
                    pad: Pad { pressed, repeat },
                    sounds: &mut effects,
                };
                let (state, events) = self.mcflow.update(mc, &mut io, font, &mut frame.quads);
                self.mc_state = state;
                for event in events {
                    match event {
                        // Title_OnCardDone.
                        mcflow::Event::Done(mcflow::Done::Done) => {
                            self.result = TitleResult::MainMenu;
                            self.leave();
                        }
                        // Title_OnCardCancel.
                        mcflow::Event::Done(mcflow::Done::NotDone) => {
                            self.clip_goto(2, "fl_on_start")
                        }
                        _ => {}
                    }
                }
                frame.sounds.extend(
                    effects
                        .into_iter()
                        .map(|(bank, id)| TitleSound::Effect(bank, id)),
                );
                frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
                if self.fade.is_fading_out() {
                    frame.sounds.push(TitleSound::FadeOutStep);
                    return frame;
                }
                if self.fade.is_out_done() {
                    self.phase = Phase::Done(self.result);
                    return frame;
                }
                if self.leaving {
                    self.timer -= 1;
                    if self.timer == -1 {
                        self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    }
                } else if self.mc_state == 0 {
                    self.input(pressed, repeat, demo, random, &mut frame.sounds, mc);
                }
            }
            Phase::Done(_) => {}
        }
        frame
    }

    fn leave(&mut self) {
        self.leaving = true;
        self.timer = LEAVE_FRAMES;
    }

    /// `Title_Idle`.
    fn idle(&mut self, demo: &mut crate::char_select::DemoProgress, random: &mut GameRandom) {
        self.idle += 1;
        if self.idle > IDLE_FRAMES {
            self.idle = 0;
            self.leave();
            if demo.count < 2 {
                self.demo = Some(crate::char_select::BattleSetup::demo(
                    random,
                    &mut demo.pick,
                ));
                self.result = TitleResult::Demo;
                demo.count += 1;
            } else {
                self.result = TitleResult::OpeningMovie;
                demo.count = 0;
            }
        }
    }

    /// `Title_ClipGoto(0, kind, label)`.
    fn clip_goto(&mut self, kind: i32, label: &str) {
        let name = match kind {
            1 => "mc_press_start_plate".to_owned(),
            _ => format!("mc_menu_plate_{}", self.cursor + 1),
        };
        let clip = self.flash.find(None, &name);
        if clip.found() {
            self.flash.clip_goto_label(clip, label);
        }
    }

    /// `Title_Input`.
    fn input(
        &mut self,
        pressed: u32,
        repeat: u32,
        demo: &mut crate::char_select::DemoProgress,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
        mc: &mut mcflow::McEnv,
    ) {
        if self.state == 0 && pressed & PAD_START != 0 {
            self.flash.goto_label("fl_skip", true);
            self.state = 1;
            sounds.push(TitleSound::Effect(1, 1));
            return;
        }
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if !self.started {
            self.clip_goto(1, "fl_on_start");
            self.state = 1;
            self.started = true;
            let line = if random.range(7) != 0 {
                self.voice
            } else {
                0x40
            };
            sounds.push(TitleSound::Voice(VOICE_BASE + line));
        }
        match self.state {
            1 => {
                if pressed & PAD_START != 0 {
                    self.idle = 0;
                    self.flash.goto_label("fl_menu_in", true);
                    self.clip_goto(1, "fl_ok");
                    self.clip_goto(2, "fl_on_start");
                    self.state = 2;
                    sounds.push(TitleSound::Voice(VOICE_BASE + self.voice + 6));
                    sounds.push(TitleSound::Effect(2, 0x38));
                } else {
                    self.idle(demo, random);
                }
            }
            2 => {
                if repeat & PAD_UP != 0 {
                    self.idle = 0;
                    if self.item_count >= 2 {
                        self.clip_goto(2, "fl_off_start");
                        self.cursor -= 1;
                        if self.cursor < 0 {
                            self.cursor = self.item_count - 1;
                        }
                        self.clip_goto(2, "fl_on_start");
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if repeat & PAD_DOWN != 0 {
                    self.idle = 0;
                    if self.item_count >= 2 {
                        self.clip_goto(2, "fl_off_start");
                        self.cursor += 1;
                        if self.cursor >= self.item_count {
                            self.cursor = 0;
                        }
                        self.clip_goto(2, "fl_on_start");
                        sounds.push(TitleSound::Effect(1, 0));
                    }
                } else if pressed & PAD_CROSS != 0 {
                    self.idle = 0;
                    self.clip_goto(2, "fl_ok");
                    if self.cursor == 0 {
                        // McFlow_Start(3) with Title_OnCardDone / Title_OnCardCancel.
                        self.mcflow.start(mcflow::MODE_NEW_SAVE, mc);
                    } else {
                        self.result = TitleResult::MainMenu;
                        self.leave();
                    }
                    sounds.push(TitleSound::Effect(1, 1));
                } else {
                    self.idle(demo, random);
                }
            }
            _ => {}
        }
    }

    /// `Title_Draw`.
    fn draw(&mut self, quads: &mut Vec<ScreenQuad>) {
        quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        for i in 0..2 {
            let clip = self.flash.find(None, &format!("mc_kumo_{}", i + 1));
            let dx = if i != 0 {
                -1.066_666_7f32
            } else {
                -0.711_111_1f32
            };
            self.scroll(
                clip,
                Uv {
                    u0: 0,
                    v0: 0,
                    u1: 0x200,
                    v1: 0x100,
                    tex: 0,
                },
                i,
                dx,
            );
        }
        for i in 0..2 {
            let uv = Uv {
                u0: 0,
                v0: i * 0x20,
                u1: 0x200,
                v1: i * 0x20 + 0x20,
                tex: 0,
            };
            let name = format!("mc_menu_plate_{}", i + 1);
            let plate = self.flash.find(None, &name);
            self.flash
                .clip_set_color(plate, if i < self.item_count { 1.0 } else { 0.5 });
            for child in ["mc_menu_text_off", "mc_menu_text_on"] {
                let clip = self.flash.find(Some(&name), child);
                self.flash.clip_set_uv(clip, uv);
            }
        }
        let layout = &self.layout;
        let has_texture = |image: usize| layout.flash.get(image).is_some_and(Option::is_some);
        for quad in self.flash.draw(&has_texture) {
            quads.push(screen_quad(&quad, &layout.flash));
        }
    }

    /// `FlashAnim_Scroll(flash, ref, uv, &gTitle->cloud[i], NULL, dx, 0)`.
    fn scroll(&mut self, clip: crate::flash::ClipRef, uv: Uv, i: usize, dx: f32) {
        if !clip.found() {
            return;
        }
        let x = &mut self.cloud[i];
        let cell = Uv {
            u0: uv.u0 + *x as i32,
            u1: uv.u1 + *x as i32,
            ..uv
        };
        *x += dx;
        if dx < 0.0 {
            if *x < 0.0 {
                *x += uv.u1 as f32;
            }
        } else if uv.u1 as f32 <= *x {
            *x -= uv.u1 as f32;
        }
        self.flash.clip_set_flags(clip, PROP_REPEAT, true);
        self.flash.clip_set_uv(clip, cell);
    }
}

#[cfg(test)]
mod tests {
    use super::PadRepeat;

    /// Runs the real title from the ISO in `rom/`, when it is there.
    #[test]
    fn real_title_reaches_press_start_and_opens_the_menu() {
        use super::{
            ARCHIVE_FILE_ID, ARCHIVE_SECTION, PAD_CROSS, PAD_LEFT, PAD_START, Phase, TextureLayout,
            TitleAssets, TitleData, TitleResult, TitleScreen,
        };
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe, pack_section};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = TitleAssets::parse(&res).unwrap();
        let mut next = 16;
        let flash = assets
            .flash
            .iter()
            .map(|t| {
                t.as_ref().map(|_| {
                    next += 2;
                    next - 2
                })
            })
            .collect();
        let dialog = assets
            .dialog
            .textures
            .iter()
            .map(|t| t.as_ref().map(|_| 0))
            .collect();
        let layout = TextureLayout {
            background: 0,
            logo: 8,
            flash,
            dialog,
        };
        // An empty card slot: the new game's flow takes its "no memory card" path.
        let mut card = crate::memcard::McCard::new(crate::memcard::Libmc::new(None));
        let mut save = crate::save::SaveData::default();
        let mut progress = crate::mcflow::Progress::default();
        let mut mc = crate::mcflow::McEnv {
            card: &mut card,
            save: &mut save,
            progress: &mut progress,
            items: &[],
        };
        let data = TitleData {
            card_res: None,
            fod: assets.fod.clone(),
            dialog_fod: assets.dialog.fod.clone(),
            dialog_titles: assets.dialog.titles.clone(),
            layout,
        };
        let mut font = crate::font::Font::new(Vec::new(), 20);
        let mut random = crate::rng::GameRandom::from_key(&[1, 2, 3, 4]);
        let mut title = TitleScreen::new(&data, false, &mut random).unwrap();
        let mut demo = crate::char_select::DemoProgress::default();
        let mut frames = 0;
        while matches!(title.phase, Phase::Logo { .. }) {
            title.run_frame(0, 0, &mut demo, &mut random, &mut font, &mut mc);
            frames += 1;
        }
        println!("logo loop: {frames} frames");
        let mut pad_at = None;
        for frame in 0..1000 {
            let out = title.run_frame(0, 0, &mut demo, &mut random, &mut font, &mut mc);
            if frame % 50 == 0 || (pad_at.is_none() && title.started) {
                let xs: Vec<f32> = out
                    .quads
                    .iter()
                    .flat_map(|q| q.pt.iter().map(|p| p[0]))
                    .collect();
                println!(
                    "menu frame {frame}: {} quads, x {:?}..{:?}",
                    out.quads.len(),
                    xs.iter().cloned().fold(f32::INFINITY, f32::min),
                    xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
                );
            }
            if title.started && pad_at.is_none() {
                pad_at = Some(frame);
                println!("press start shown at menu frame {frame}: {:?}", out.sounds);
            }
        }
        assert!(pad_at.is_some(), "the movie never enabled the pad");
        let out = title.run_frame(
            PAD_START,
            PAD_START,
            &mut demo,
            &mut random,
            &mut font,
            &mut mc,
        );
        assert_eq!(title.state, 2);
        println!("after start: {:?}", out.sounds);
        for _ in 0..120 {
            title.run_frame(0, 0, &mut demo, &mut random, &mut font, &mut mc);
        }
        let out = title.run_frame(0, 0, &mut demo, &mut random, &mut font, &mut mc);
        println!("menu open: {} quads", out.quads.len());
        for q in out
            .quads
            .iter()
            .filter(|q| q.texture.is_some_and(|t| t >= 16))
        {
            println!(
                "  tex {:?} pt {:?} uv {:?} rgba {:?}",
                q.texture, q.pt, q.uv, q.rgba
            );
        }

        // New Game with no memory card: create a save? Yes; no card; start without saving? Yes.
        macro_rules! step {
            ($bits:expr) => {
                title
                    .run_frame($bits, $bits, &mut demo, &mut random, &mut font, &mut mc)
                    .sounds
            };
        }
        println!("cross: {:?}", step!(PAD_CROSS));
        let mut opened = None;
        for frame in 0..300 {
            if title.dialog.is_open() {
                opened = Some(frame);
                break;
            }
            step!(0);
        }
        println!("dialog open after {opened:?} frames");
        assert!(opened.is_some(), "the dialog never opened");
        assert_eq!(title.mc_state, 0xA);
        println!("left: {:?}", step!(PAD_LEFT));
        println!("cross: {:?}", step!(PAD_CROSS));
        for _ in 0..13 {
            step!(0);
        }
        assert_eq!(title.mc_state, 3);
        for _ in 0..120 {
            step!(0);
        }
        assert_eq!(title.mc_state, 0x1B);
        step!(PAD_CROSS);
        assert_eq!(title.mc_state, 9);
        step!(PAD_LEFT);
        step!(PAD_CROSS);
        let mut closed_at = None;
        for frame in 0..300 {
            let sounds = step!(0);
            if !sounds.is_empty() {
                println!("frame {frame}: {sounds:?}");
            }
            if title.mc_state == 0 {
                closed_at = Some(frame);
                break;
            }
        }
        println!("closed after {closed_at:?} frames");
        assert!(title.leaving);
        assert_eq!(title.result, TitleResult::MainMenu);

        // Left alone, the screen leaves for the attract battle twice, then for the opening movie.
        let mut demo = crate::char_select::DemoProgress::default();
        for round in 0..3 {
            let mut title = TitleScreen::new(&data, false, &mut random).unwrap();
            let mut result = None;
            for _ in 0..4000 {
                title.run_frame(0, 0, &mut demo, &mut random, &mut font, &mut mc);
                result = title.result();
                if result.is_some() {
                    break;
                }
            }
            let setup = title.take_demo();
            if round < 2 {
                assert_eq!(result, Some(TitleResult::Demo));
                let setup = setup.unwrap();
                assert_eq!((setup.mode, demo.count), (7, round + 1));
                assert!((0..9).contains(&demo.pick));
            } else {
                assert_eq!(result, Some(TitleResult::OpeningMovie));
                assert!(setup.is_none());
                assert_eq!(demo.count, 0);
            }
        }
    }

    #[test]
    fn pad_repeat_waits_the_delay_then_repeats_every_fourth_frame() {
        let mut pad = PadRepeat::default();
        assert_eq!(pad.update(8, 8), 8);
        let fired: Vec<u32> = (0..0x28 + 9).map(|_| pad.update(8, 0)).collect();
        let at: Vec<usize> = fired
            .iter()
            .enumerate()
            .filter(|(_, b)| **b != 0)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(at, [0x28, 0x28 + 4, 0x28 + 8]);
        assert_eq!(pad.update(0, 0), 0);
    }
}
