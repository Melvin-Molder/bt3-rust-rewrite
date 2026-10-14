//! The character reference (`CharRef_Run(1)`, src/menu/char_reference.c, progress mode 60, `CharRefMode_Main`):
//! Chi-Chi as guide, a scrolling list of the unlocked characters, and for one of them its large picture (streamed
//! from the disc), the details menu: the profile text, a voice sample, Chi-Chi's spoken comment and the model
//! viewer with a costume choice. The viewer itself (`ChrView`, a 3D scene of its own) belongs to the caller: the
//! screen asks it to show or hide a model and draws nothing while it is shown.

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
use crate::rng::{GameRandom, LibcRandom};
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::windows::{IconWin, MsgWin, TextBox, TextDraw, draw_texts};

/// `gMenuArc9`: file `gProgress->baseFile + 15`; `CharRef_Run(1)` unpacks its section 1.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 15;
pub const ARCHIVE_SECTION: usize = 1;
/// `BGM_CHAR_REF`.
pub const BGM_FILE_ID: u32 = 0x10B18;
/// `CHARREF_VOICE_BASE`.
const VOICE_BASE: u32 = 0x82B7;
/// `CHARREF_IMAGE_FILE`: character 0's large picture.
const IMAGE_FILE: u32 = 0x2F9;
/// `CHARREF_CHARA_MAX`, `CHARREF_ROWS`.
const CHARA_MAX: usize = 161;
const ROWS: i32 = 4;
const FADE_FRAMES: i32 = 0x14;

// CHARREF_ST_*.
const ST_INTRO: i32 = 0;
const ST_LIST: i32 = 1;
const ST_DETAILS: i32 = 2;
const ST_PROFILE: i32 = 3;
const ST_VOICE: i32 = 4;
const ST_COMMENT: i32 = 5;
const ST_COSTUME: i32 = 6;
const ST_VIEW: i32 = 7;
const ST_VIEW_WAIT: i32 = 8;
const ST_LEAVE: i32 = 9;

// CHARREF_LOAD_*.
const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;
const LOAD_SHOWN: i32 = 4;
const LOAD_ABORT: i32 = 5;
const LOAD_RESTART: i32 = 6;

const IMAGE_CHANGE: u32 = 1;
const IMAGE_READY: u32 = 2;

/// The list's scissor (`CharRef_ScissorOn`).
const LIST_CLIP: [i32; 4] = [0, 0x49, 0x200, 0x135];

/// `gCharRef->tex[n]` of `CharRef_Init`; image 2 is the large picture, image 32 Chi-Chi's pose (42 or 43).
const FLASH_TEXTURES: [ImageTexture; 44] = [
    tex(16, 0),
    tex(16, 5),
    None,
    tex(13, 0),
    tex(13, 1),
    tex(13, 2),
    tex(13, 3),
    tex(9, 0),
    tex(8, 0),
    tex(9, 5),
    tex(9, 1),
    tex(14, 0),
    tex(8, 1),
    tex(7, 0),
    tex(16, 3),
    tex(16, 2),
    tex(9, 2),
    None,
    None,
    tex(9, 4),
    tex(9, 3),
    tex(5, 2),
    tex(5, 0),
    tex(5, 1),
    tex(5, 3),
    tex(24, 0),
    tex(16, 1),
    None,
    tex(11, 0),
    tex(11, 1),
    tex(11, 2),
    None,
    tex(12, 0),
    tex(12, 6),
    tex(12, 8),
    tex(12, 1),
    tex(12, 3),
    tex(3, 0),
    tex(3, 1),
    tex(16, 4),
    tex(10, 0),
    tex(2, 0),
    tex(12, 0),
    tex(12, 5),
];
const IMAGE_SLOT: usize = 2;
const POSE_SLOT: usize = 32;
/// `MsgWin_Init(MPACK_AT(res, 18), …)` and `IconWin_Init(MPACK_AT(res, 19), res 4)`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((18, 1), 0)), Some(((18, 1), 1)), None];
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((19, 1), 1)),
    Some(((19, 1), 0)),
    tex(4, 0),
    Some(((19, 1), 2)),
    tex(4, 1),
];

/// What `CharRef_Init` reads from its unpacked section.
pub struct CharRefAssets {
    pub data: CharRefData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct CharRefData {
    pub fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    /// Section 21: the message window's lines; section 20: the mouth keys of Chi-Chi's lines.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    /// Sections 22 (names), 23 (forms) and 25 (per character the voice label, then the profile).
    pub names: Vec<u8>,
    pub forms: Vec<u8>,
    pub profiles: Vec<u8>,
    /// Section 17: the character of each list position (negative: none); 26 / 28: its comment's first line and
    /// line count; 27: Chi-Chi's pose per comment line.
    pub charas: Vec<i32>,
    pub comment_line: Vec<i32>,
    pub comment_num: Vec<i32>,
    pub poses: Vec<i32>,
    pub layout: CharRefLayout,
}

#[derive(Clone, Debug, Default)]
pub struct CharRefLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
}

impl CharRefLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            background,
            flash: image_indices(&FLASH_TEXTURES, &first),
            msg: image_indices(&MSG_TEXTURES, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
        }
    }
}

/// The words of a table section.
fn words(res: &[u8], section: usize, count: usize) -> Result<Vec<i32>, Box<dyn Error>> {
    let s = pack_section(res, section)?;
    Ok((0..count)
        .map(|i| {
            s.get(i * 4..i * 4 + 4)
                .map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()))
        })
        .collect())
}

impl CharRefAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(res, [&FLASH_TEXTURES[..], &MSG_TEXTURES, &ICON_TEXTURES])?;
        let poses = {
            let s = pack_section(res, 27)?;
            s.chunks_exact(4)
                .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
                .collect()
        };
        Ok(Self {
            data: CharRefData {
                fod: pack_section(res, 1)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 18)?, 2)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 19)?, 2)?.to_vec(),
                msg_text: pack_section(res, 21)?.to_vec(),
                lips: pack_section(res, 20)?.to_vec(),
                names: pack_section(res, 22)?.to_vec(),
                forms: pack_section(res, 23)?.to_vec(),
                profiles: pack_section(res, 25)?.to_vec(),
                charas: words(res, 17, CHARA_MAX)?,
                comment_line: words(res, 26, CHARA_MAX)?,
                comment_num: words(res, 28, CHARA_MAX)?,
                poses,
                layout: CharRefLayout::default(),
            },
            background: decode_picture(res, 15)?,
            files,
        })
    }
}

/// `CharRefEntry`: a listed (unlocked) character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Entry {
    chara: i32,
    comment_line: i32,
    comment_num: i32,
}

/// What the screen asks of the model viewer this frame (`ChrView_Show` / `ChrView_Hide`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerRequest {
    Show { chara: i32, costume: i32 },
    Hide,
}

/// `CharRef` and the frame loop of `CharRef_Run`.
pub struct CharRefScreen {
    data: CharRefData,
    textures: Vec<Option<usize>>,
    flash: Flash,
    msg: MsgWin,
    icon: IconWin,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    entry: Vec<Entry>,
    cursor: i32,
    costume: i32,
    load_state: i32,
    loading: Option<usize>,
    blink: i32,
    talk: i32,
    voice_line: i32,
    menu_cursor: i32,
    row: i32,
    idle: i32,
    comment: i32,
    scroll: i32,
    text_height: i32,
    page_dir: i32,
    comment_base: i32,
    pose: i32,
    scroll_dir: i32,
    last_line: i32,
    state: i32,
    prev_state: i32,
    flags: u32,
    image_alpha: f32,
    /// `ChrTbl` flags and costume counts by character.
    chara_flags: Vec<i32>,
    costumes: Vec<i32>,
    requests: Vec<ViewerRequest>,
    done: bool,
}

impl CharRefScreen {
    /// `CharRef_Init` (`CharRef_BuildList` over the save's characters) and the first `ColorFade_StartIn`.
    pub fn new(
        data: &CharRefData,
        save: &crate::save::SaveData,
        chr: &bt3_data::chara_data::ChrTable,
        loader: &mut dyn PictureLoader,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        flash.goto_label("fl_chara_list_in", true);
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            false,
        )?;
        msg.open();
        // CharRef_BuildList: the unlocked characters in the table's order.
        let entry = (0..CHARA_MAX)
            .filter_map(|i| {
                let chara = *data.charas.get(i)?;
                (chara >= 0 && save.chara_unlocked(chara)).then(|| Entry {
                    chara,
                    comment_line: data.comment_line.get(i).copied().unwrap_or(0),
                    comment_num: data.comment_num.get(i).copied().unwrap_or(0),
                })
            })
            .collect();
        let mut textures = data.layout.flash.clone();
        textures[IMAGE_SLOT] = loader.portrait(IMAGE_FILE);
        Ok(Self {
            data: data.clone(),
            textures,
            flash,
            msg,
            icon,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            entry,
            cursor: 0,
            costume: 0,
            load_state: LOAD_SHOWN,
            loading: None,
            blink: 0,
            talk: 0,
            voice_line: -1,
            menu_cursor: 0,
            row: 0,
            idle: 0,
            comment: 0,
            scroll: 0,
            text_height: 0,
            page_dir: 0,
            comment_base: 9,
            pose: 0,
            scroll_dir: 0,
            last_line: 0,
            state: ST_INTRO,
            prev_state: 0,
            flags: 0,
            image_alpha: 1.0,
            chara_flags: chr.flags.clone(),
            costumes: chr.costumes.clone(),
            requests: Vec::new(),
            done: false,
        })
    }

    /// `CharRef_Run` has returned: back to the main menu.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The model viewer has the screen (`CharRef_Run` runs `ChrView_Update` instead of `CharRef_Draw`).
    pub fn viewer_shown(&self) -> bool {
        self.state == ST_VIEW
    }

    /// The viewer requests of the last frame.
    pub fn take_viewer_requests(&mut self) -> Vec<ViewerRequest> {
        std::mem::take(&mut self.requests)
    }

    fn count(&self) -> i32 {
        self.entry.len() as i32
    }

    fn chara(&self, i: i32) -> i32 {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.entry.get(i))
            .map_or(0, |e| e.chara)
    }

    fn cur(&self) -> Entry {
        usize::try_from(self.cursor)
            .ok()
            .and_then(|i| self.entry.get(i).copied())
            .unwrap_or_default()
    }

    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        anim::play_with_subtitle(&self.data.lips, VOICE_BASE, line, &mut self.lip, sounds);
    }

    /// `Voice_StopWithLip`.
    fn stop_voice(&mut self, sounds: &mut Vec<TitleSound>) {
        sounds.push(TitleSound::VoiceStop);
        self.lip = LipSync::default();
    }

    /// `Voice_PlayChara(0, chara, rand() % 2)`: one of the character's two battle intro lines.
    fn play_chara(&mut self, libc: &mut LibcRandom, sounds: &mut Vec<TitleSound>) {
        let line = libc.rand() % 2;
        let chara = self.cur().chara.max(0) as u32;
        sounds.push(TitleSound::Voice(
            crate::battle_seq::voice_chara_base()
                + chara * crate::battle_seq::VOICE_CHARA_LINES
                + line as u32,
        ));
    }

    /// One iteration of `CharRef_Run`'s loop: the picture loader, the frame (the screen, or nothing while the
    /// model viewer has it), then the state's handler. `voice_stat` is `Voice_GetStat(0)`, `viewer_visible`
    /// `ChrView_IsVisible()`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        viewer_visible: bool,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
        font: &mut Font,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done {
            return frame;
        }
        self.update_image(loader);
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        if self.state != ST_VIEW {
            self.draw(voice_stat, random, font, &mut frame.quads);
        }
        let sounds = &mut frame.sounds;
        if self.state == ST_LEAVE {
            if self.fade.is_fading_out() {
                sounds.push(TitleSound::FadeOutStep);
            } else if self.fade.is_out_done() {
                if self.load_state == LOAD_SHOWN {
                    self.done = true;
                    // CharRef_Term: ChrView_Term.
                    self.requests.push(ViewerRequest::Hide);
                    return frame;
                }
                self.update_image(loader);
            }
            return frame;
        }
        let pad = Pad {
            held,
            pressed,
            repeat,
        };
        match self.state {
            ST_INTRO => {
                if self.flash.flags() & FLASH_PAD != 0 {
                    self.clip_goto(0, "fl_on_start");
                    self.state = ST_LIST;
                    self.voice_line = 0;
                    self.say(0, sounds);
                }
            }
            ST_LIST => self.state_list(pad, voice_stat, random, sounds),
            ST_DETAILS => self.state_details(pad, font, libc, sounds),
            ST_PROFILE => self.state_profile(pad, sounds),
            ST_VOICE => {
                if self.flash.flags() & FLASH_PAD != 0 {
                    if pressed & PAD_CROSS != 0 {
                        sounds.push(TitleSound::VoiceStop);
                        self.play_chara(libc, sounds);
                    }
                    if pressed & PAD_TRIANGLE != 0 {
                        self.flash.goto_label("fl_voise_out", true);
                        self.clip_goto(1, "fl_on_start");
                        self.state = ST_DETAILS;
                        sounds.push(TitleSound::Effect(1, 2));
                        sounds.push(TitleSound::VoiceStop);
                    }
                }
            }
            ST_COMMENT => self.state_comment(pad, voice_stat, sounds),
            ST_COSTUME => self.state_costume(pad, sounds),
            ST_VIEW => {
                if self.flash.flags() & FLASH_PAD != 0 && pressed & PAD_TRIANGLE != 0 {
                    self.requests.push(ViewerRequest::Hide);
                    self.clip_goto(2, "fl_on_start");
                    self.state = ST_COSTUME;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            ST_VIEW_WAIT => {
                if self.flash.flags() & FLASH_PAD != 0 && viewer_visible {
                    self.state = ST_VIEW;
                }
            }
            _ => {}
        }
        frame
    }

    /// `CharRef_ClipGoto`: kind 0 the list row under the cursor, 1 the details item, 2 the fifth plate.
    fn clip_goto(&mut self, kind: i32, label: &str) {
        let name = match kind {
            1 => format!("mc_menu_plate_{}", self.menu_cursor + 1),
            0 => format!("mc_menu_plate2_{}", self.row + 1),
            _ => "mc_menu_plate_5".to_owned(),
        };
        let clip = self.flash.find(None, &name);
        self.flash.clip_goto_label(clip, label);
    }

    /// `CharRef_StateList`: up / down one character (the list scrolls at its ends), left / right four, confirm the
    /// details, cancel leaves; Chi-Chi says one of her idle lines after 300 quiet frames.
    fn state_list(
        &mut self,
        pad: Pad,
        voice_stat: i32,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        let count = self.count();
        if pad.repeat & PAD_UP != 0 {
            self.clip_goto(0, "fl_off_start");
            self.row -= 1;
            if self.row < 0 {
                self.row = 0;
                self.scroll_dir = 8;
                self.flash.goto_label("fl_chara_list_down", true);
            }
            self.cursor -= 1;
            if self.cursor < 0 {
                self.cursor = count - 1;
            }
            self.clip_goto(0, "fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
            self.idle = 0;
        } else if pad.repeat & PAD_DOWN != 0 {
            self.clip_goto(0, "fl_off_start");
            self.row += 1;
            if self.row >= ROWS {
                self.row = ROWS - 1;
                self.scroll_dir = 4;
                self.flash.goto_label("fl_chara_list_up", true);
            }
            self.cursor += 1;
            if self.cursor > count - 1 {
                self.cursor = 0;
            }
            self.clip_goto(0, "fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
            self.idle = 0;
        } else if pad.repeat & PAD_LEFT != 0 {
            self.cursor -= 4;
            if self.cursor < 0 {
                self.cursor = count - 1;
            }
            sounds.push(TitleSound::Effect(1, 0));
            self.idle = 0;
        } else if pad.repeat & PAD_RIGHT != 0 {
            self.cursor += 4;
            if self.cursor > count - 1 {
                self.cursor = 0;
            }
            sounds.push(TitleSound::Effect(1, 0));
            self.idle = 0;
        } else if pad.pressed & PAD_CROSS != 0 {
            self.clip_goto(0, "fl_ok");
            self.prev_state = self.state;
            self.state = ST_DETAILS;
            self.msg.close();
            self.icon.close();
            self.clip_goto(1, "fl_on_start");
            self.change_image();
            self.flash.goto_label("fl_chara_details_in", true);
            self.idle = 0;
            sounds.push(TitleSound::Effect(1, 1));
            if voice_stat != VOICE_PLAYEND {
                self.stop_voice(sounds);
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.fade = ColorFade::start_out(true, FADE_FRAMES);
            self.state = ST_LEAVE;
            self.idle = 0;
            sounds.push(TitleSound::Effect(1, 2));
        } else if voice_stat == VOICE_PLAYEND {
            self.idle += 1;
            if self.idle > 300 {
                self.idle = 0;
                loop {
                    self.voice_line = random.range(7) as i32 + 2;
                    if self.voice_line != self.last_line {
                        self.last_line = self.voice_line;
                        break;
                    }
                }
                self.say(self.voice_line, sounds);
            }
        }
    }

    /// `CR_SAY_COMMENT`: line `comment` of the character's comment with the pose that goes with it.
    fn say_comment(&mut self, sounds: &mut Vec<TitleSound>) {
        self.pose = self.comment_pose();
        let line = self.voice_line + self.comment_base + self.comment;
        self.say(line, sounds);
    }

    fn comment_pose(&self) -> i32 {
        usize::try_from(self.comment_base + self.comment - 9)
            .ok()
            .and_then(|i| self.data.poses.get(i).copied())
            .unwrap_or(0)
    }

    /// `CharRef_StateDetails`: up / down the four items, left / right the previous / next character.
    fn state_details(
        &mut self,
        pad: Pad,
        font: &Font,
        libc: &mut LibcRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        let count = self.count();
        if pad.repeat & PAD_UP != 0 {
            self.clip_goto(1, "fl_off_start");
            self.menu_cursor -= 1;
            if self.menu_cursor < 0 {
                self.menu_cursor = 3;
            }
            self.clip_goto(1, "fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.repeat & PAD_DOWN != 0 {
            self.clip_goto(1, "fl_off_start");
            self.menu_cursor += 1;
            if self.menu_cursor >= 4 {
                self.menu_cursor = 0;
            }
            self.clip_goto(1, "fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.repeat & PAD_LEFT != 0 {
            self.cursor -= 1;
            if self.cursor < 0 {
                self.cursor = count - 1;
            }
            self.page_dir = 0;
            sounds.push(TitleSound::Effect(1, 0));
            self.flash.goto_label("fl_page_down", true);
            self.change_image();
        } else if pad.repeat & PAD_RIGHT != 0 {
            self.cursor += 1;
            if self.cursor > count - 1 {
                self.cursor = 0;
            }
            self.page_dir = 1;
            sounds.push(TitleSound::Effect(1, 0));
            self.flash.goto_label("fl_page_up", true);
            self.change_image();
        } else if pad.pressed & PAD_CROSS != 0 {
            self.clip_goto(1, "fl_ok");
            match self.menu_cursor {
                0 => {
                    self.scroll = 0;
                    self.measure_text(font);
                    self.state = ST_PROFILE;
                    self.flash.goto_label("fl_chara_1_in", true);
                }
                1 => {
                    self.play_chara(libc, sounds);
                    self.state = ST_VOICE;
                    self.flash.goto_label("fl_voise_in", true);
                }
                2 => {
                    self.voice_line = 0;
                    self.comment_base = 9 + self.cur().comment_line;
                    self.comment = 0;
                    self.state = ST_COMMENT;
                    self.msg.set_side(true);
                    self.msg.open();
                    self.say_comment(sounds);
                    self.flash.goto_label("fl_chichi_comment_in", true);
                }
                _ => {
                    self.costume = 0;
                    self.menu_cursor = 5;
                    self.clip_goto(1, "fl_off_start");
                    self.clip_goto(2, "fl_on_start");
                    self.state = ST_COSTUME;
                    self.flash.goto_label("fl_model_view_in", true);
                }
            }
            sounds.push(TitleSound::Effect(1, 1));
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.msg.set_side(false);
            self.flash.goto_label("fl_chara_details_cancel", true);
            self.clip_goto(1, "fl_off_start");
            self.menu_cursor = 0;
            self.msg.open();
            self.icon.open();
            self.state = ST_LIST;
            self.pose = 0;
            self.say(1, sounds);
            sounds.push(TitleSound::Effect(1, 2));
            self.voice_line = 1;
        }
    }

    /// `CharRef_StateProfile`: up / down scroll the text while held, cancel goes back.
    fn state_profile(&mut self, pad: Pad, sounds: &mut Vec<TitleSound>) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        if pad.held & PAD_UP != 0 {
            if self.scroll > 0 {
                self.scroll -= 2;
            }
        } else if pad.held & PAD_DOWN != 0 {
            if self.text_height - 0x6E >= self.scroll {
                self.scroll += 2;
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.flash.goto_label("fl_chara_1_out", true);
            self.clip_goto(1, "fl_on_start");
            self.state = ST_DETAILS;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    /// `CharRef_StateComment`: each line follows the last by itself or on confirm; cancel goes back.
    fn state_comment(&mut self, pad: Pad, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        let more = self.comment < self.cur().comment_num - 1;
        let back = |s: &mut Self| {
            s.msg.close();
            s.flash.goto_label("fl_chichi_comment_out", true);
            s.clip_goto(1, "fl_on_start");
            s.prev_state = s.state;
            s.state = ST_DETAILS;
        };
        if voice_stat == VOICE_PLAYEND {
            if more {
                self.comment += 1;
                self.say_comment(sounds);
            } else {
                back(self);
                sounds.push(TitleSound::Effect(1, 2));
            }
        } else if pad.pressed & PAD_CROSS != 0 {
            if more {
                self.comment += 1;
                self.pose = self.comment_pose();
                sounds.push(TitleSound::Effect(1, 1));
                let line = self.voice_line + self.comment_base + self.comment;
                self.say(line, sounds);
            } else {
                back(self);
                sounds.push(TitleSound::Effect(1, 1));
                self.stop_voice(sounds);
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            back(self);
            self.stop_voice(sounds);
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    /// `CharRef_StateCostume`: up / down over the character's costumes, confirm shows the model, cancel goes back.
    fn state_costume(&mut self, pad: Pad, sounds: &mut Vec<TitleSound>) {
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        let costumes = usize::try_from(self.cur().chara)
            .ok()
            .and_then(|c| self.costumes.get(c).copied())
            .unwrap_or(1);
        if pad.pressed & PAD_UP != 0 {
            self.costume -= 1;
            if self.costume < 0 {
                self.costume = costumes - 1;
            }
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.pressed & PAD_DOWN != 0 {
            self.costume += 1;
            if self.costume > costumes - 1 {
                self.costume = 0;
            }
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.pressed & PAD_CROSS != 0 {
            self.clip_goto(2, "fl_ok");
            self.state = ST_VIEW_WAIT;
            self.requests.push(ViewerRequest::Show {
                chara: self.cur().chara,
                costume: self.costume,
            });
            sounds.push(TitleSound::Effect(1, 1));
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.clip_goto(2, "fl_off_start");
            self.menu_cursor = 3;
            self.flash.goto_label("fl_model_view_out", true);
            self.clip_goto(1, "fl_on_start");
            self.state = ST_DETAILS;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    /// `CharRef_MeasureText`: the profile's height (`Font_GetHeight` + 4), for the scroll limit.
    fn measure_text(&mut self, font: &Font) {
        let line = self.cur().chara * 2 + 1;
        let text =
            bt3_data::font::message(&self.data.profiles, line.max(0) as usize).unwrap_or_default();
        let st = &font.style;
        let height = Font::count_lines(&text)
            * ((font.line_height() as f32 * st.scale_y * st.scale) as i32 + st.spacing_y)
            - st.spacing_y;
        self.text_height = height + 4;
    }

    /// `CharRef_UpdateImage`: one step of the large picture's background loader.
    fn update_image(&mut self, loader: &mut dyn PictureLoader) {
        match self.load_state {
            LOAD_SHOWN => {
                if self.flags & IMAGE_CHANGE != 0 {
                    self.load_state = LOAD_RESTART;
                }
            }
            LOAD_RESTART => {
                self.flags &= !IMAGE_CHANGE;
                self.load_state = LOAD_REQUEST;
            }
            LOAD_REQUEST => {
                let chara = self.cur().chara.max(0) as u32;
                self.loading = loader.portrait(IMAGE_FILE + chara);
                self.load_state = LOAD_READ;
            }
            LOAD_READ => self.load_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.textures[IMAGE_SLOT] = self.loading;
                self.flags |= IMAGE_READY;
                self.load_state = LOAD_SHOWN;
            }
            LOAD_ABORT => self.load_state = LOAD_RESTART,
            _ => {}
        }
    }

    /// `CharRef_ChangeImage`.
    fn change_image(&mut self) {
        if self.flags & IMAGE_READY != 0 {
            self.flags ^= IMAGE_READY;
        } else {
            self.load_state = LOAD_ABORT;
        }
        self.flags |= IMAGE_CHANGE;
    }

    fn set_uv(&mut self, parent: Option<&str>, name: &str, uv: Uv) {
        let r = self.flash.find(parent, name);
        self.flash.clip_set_uv(r, uv);
    }

    fn base_plate_tex(&self, chara: i32) -> i32 {
        let flags = usize::try_from(chara)
            .ok()
            .and_then(|c| self.chara_flags.get(c).copied())
            .unwrap_or(0);
        (flags ^ 1) & 1
    }

    /// `CharRef_Draw`.
    fn draw(
        &mut self,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        let state = self.state;
        let pose = self.pose != 0;
        let chara = self.cur().chara;
        quads.extend(
            picture_quads(self.data.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        self.flash.advance();
        self.textures[POSE_SLOT] = self.data.layout.flash[if pose { 43 } else { 42 }];
        let (on, off) = if pose { ("b", "a") } else { ("a", "b") };
        let eye = self.flash.find(None, &format!("mc_guide_{on}_chichi_eye"));
        anim::blink(&mut self.flash, eye, &mut self.blink, 0, random);
        let mouth = self
            .flash
            .find(None, &format!("mc_guide_{on}_chichi_mouth"));
        anim::talk(
            &mut self.flash,
            mouth,
            &mut self.talk,
            0,
            &mut self.lip,
            voice_stat,
            random,
        );
        for part in ["mouth", "eye"] {
            let r = self
                .flash
                .find(None, &format!("mc_guide_{off}_chichi_{part}"));
            self.flash.clip_set_flags(r, PROP_VISIBLE, false);
        }
        let mut texts: Vec<TextDraw> = Vec::new();
        let mut tag = 0x400;
        let mut attach = |flash: &mut Flash,
                          parent: Option<&str>,
                          name: &str,
                          text: &[u8],
                          line: i32,
                          bx: TextBox,
                          dy: i32,
                          font: &Font| {
            let clip = flash.find(parent, name);
            tag += 1;
            if let Some(t) = bx.attach_line_at(flash, clip, tag, text, line, (0, dy), font) {
                texts.push(t);
            }
        };
        let names = self.data.names.clone();
        let forms = self.data.forms.clone();
        let profiles = self.data.profiles.clone();
        attach(
            &mut self.flash,
            None,
            "mc_name_text_1",
            &names,
            chara,
            TextBox::preset(2),
            0,
            font,
        );
        attach(
            &mut self.flash,
            None,
            "mc_form_text_1",
            &forms,
            chara,
            TextBox::preset(4),
            0,
            font,
        );
        let line = chara * 2;
        attach(
            &mut self.flash,
            None,
            "mc_voice_text_1",
            &profiles,
            line,
            TextBox::preset(2),
            0,
            font,
        );
        for n in [1, 4, 5] {
            let r = self.flash.find(None, &format!("mc_menu_plate2_{n}"));
            self.flash
                .clip_set_scissor(r, (state == ST_LIST).then_some(LIST_CLIP));
        }
        let count = self.count();
        let list_box = |preset| TextBox::preset(preset).with_clip(0, 0x200, 0x49, 0x135);
        for i in 0..5 {
            let mut n = self.cursor - self.row + i;
            if n > count - 1 {
                n -= count;
            } else if n < 0 {
                n += count;
            }
            let plate = format!("mc_menu_plate2_{}", i + 1);
            let c = self.chara(n);
            attach(
                &mut self.flash,
                Some(&plate),
                "mc_name_text",
                &names,
                c,
                list_box(2),
                0,
                font,
            );
            attach(
                &mut self.flash,
                Some(&plate),
                "mc_form_text",
                &forms,
                c,
                list_box(4),
                0,
                font,
            );
        }
        if self.scroll_dir == 4 {
            let mut n = self.cursor - self.row - 1;
            if self.cursor < 3 {
                n = count + self.cursor - 4;
            }
            // The loop's counter has reached 5: the fifth plate.
            let c = self.chara(n);
            attach(
                &mut self.flash,
                Some("mc_menu_plate2_5"),
                "mc_name_text",
                &names,
                c,
                list_box(2),
                0,
                font,
            );
            attach(
                &mut self.flash,
                Some("mc_menu_plate2_5"),
                "mc_form_text",
                &forms,
                c,
                list_box(4),
                0,
                font,
            );
        }
        for i in 0..5 {
            let name = format!("mc_menu_plate_{}", i + 1);
            let uv = Uv {
                u0: 0,
                v0: i * 0x20,
                u1: 0x100,
                v1: i * 0x20 + 0x20,
                tex: 0,
            };
            self.set_uv(Some(&name), "mc_menu_text_off", uv);
            self.set_uv(Some(&name), "mc_menu_text_on", uv);
        }
        if self.flags & IMAGE_READY != 0 {
            self.image_alpha = (self.image_alpha + 0.075).min(1.0);
        } else {
            self.image_alpha = 0.0;
        }
        let image = self.flash.find(None, "mc_single_chara_l");
        self.flash.clip_set_alpha(image, self.image_alpha);
        let arrow = |u0, v0| Uv {
            u0,
            v0,
            u1: u0 + 0x20,
            v1: v0 + 0x20,
            tex: 0,
        };
        self.set_uv(None, "mc_yajirusi_icon_right", arrow(0x20, 0));
        self.set_uv(None, "mc_yajirusi_icon_down", arrow(0x20, 0x20));
        self.set_uv(None, "mc_yajirusi_icon_up", arrow(0, 0x20));
        if count > 0 {
            let scale = 4.0 / count as f32 * 7.0;
            let y = ((self.cursor - self.row) * 0xE0 / count).clamp(0, 0xDA);
            let bar = self.flash.find(None, "mc_scroll_bar_point");
            self.flash.clip_set_scale(bar, 1.0, scale);
            self.flash.clip_set_offset(bar, 0, y);
        }
        let c = self.costume + 1;
        let num = match c {
            0 => Uv {
                u0: 0,
                v0: 0,
                u1: c + 0x20,
                v1: c + 0x20,
                tex: 0,
            },
            1..=3 => arrow(c * 0x20, 0),
            // The rectangle the up arrow got.
            _ => arrow(0, 0x20),
        };
        self.set_uv(Some("mc_menu_plate_5"), "mc_num", num);
        let tex = self.base_plate_tex(chara);
        let base = self.flash.find(None, "mc_ch_base_1");
        self.flash.clip_set_tex(base, tex);
        let y = if self.page_dir != 0 {
            self.cursor - 1
        } else {
            self.cursor + 1
        };
        let next = if y >= count {
            self.chara(0)
        } else if y < 0 {
            self.chara(count - 1)
        } else {
            self.chara(y)
        };
        let tex = self.base_plate_tex(next);
        for n in [2, 3] {
            let r = self.flash.find(None, &format!("mc_ch_base_{n}"));
            self.flash.clip_set_tex(r, tex);
        }
        if state == ST_PROFILE {
            let up = self.flash.find(None, "mc_yajirusi_icon_up");
            self.flash.clip_set_flags(up, PROP_VISIBLE, self.scroll > 0);
            let down = self.flash.find(None, "mc_yajirusi_icon_down");
            self.flash
                .clip_set_flags(down, PROP_VISIBLE, self.scroll < self.text_height - 0x6E);
        }
        let profile = TextBox::preset(0).with_clip(0, 0x200, 0x128, 0x198);
        attach(
            &mut self.flash,
            None,
            "mc_dammy_text",
            &profiles,
            line + 1,
            profile,
            -self.scroll,
            font,
        );
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&flash_quads, textures));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.icon.draw(quads);
        let line = match state {
            ST_INTRO | ST_LIST | ST_LEAVE => self.voice_line,
            ST_DETAILS if self.prev_state == ST_LIST => self.voice_line,
            _ => self.voice_line + self.comment_base + self.comment,
        };
        self.msg.draw(line, font, quads);
        quads.extend(self.fade.quad().map(ScreenQuad::from));
    }
}

/// The pad words a state reads.
#[derive(Clone, Copy, Debug)]
struct Pad {
    held: u32,
    pressed: u32,
    repeat: u32,
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

    /// Runs the real character reference from the ISO in `rom/`, when it is there: the list, a character's
    /// picture and profile, then its model in the viewer (the backdrop and the model drawn), and back out.
    #[test]
    fn real_character_reference_shows_a_model() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let mut read = |id: u32| -> Result<Vec<u8>, String> {
            let location = partition_entry_for_id(id);
            let archive =
                AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition))
                    .map_err(|e| e.to_string())?;
            archive
                .read_entry(&mut image, location.index)
                .map_err(|e| e.to_string())
        };
        let pack = read(ARCHIVE_FILE_ID).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = CharRefAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = CharRefLayout::new(0, |key| firsts[&key]);
        let chr = bt3_data::chara_data::ChrTable::parse(&read(4).unwrap(), 0xA1);
        let save = crate::save::SaveData::default();
        let mut screen = CharRefScreen::new(&data, &save, &chr, &mut Pictures).unwrap();
        assert!(screen.count() > 50, "{}", screen.count());
        let mut iso2 = Iso9660::open(&iso).unwrap();
        let cnf = iso2.read_file("SYSTEM.CNF").unwrap();
        let exe = bt3_data::elf::boot_path(&cnf).unwrap();
        let elf = bt3_data::elf::Elf::parse(iso2.read_file(&exe).unwrap()).unwrap();
        let face = crate::btl_face::FaceTables::read(&elf).unwrap();
        let mut viewer = crate::chr_view::ChrView::new(&mut read, face, &chr).unwrap();
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[2, 7, 1, 8]);
        let mut libc = LibcRandom::new(1);
        let drawn = std::cell::Cell::new((0, 0));
        let mut step = |screen: &mut CharRefScreen,
                        viewer: &mut crate::chr_view::ChrView,
                        bits: u32| {
            let shown = screen.viewer_shown();
            if !shown {
                viewer.step_job(&mut read);
            }
            screen.run_frame(
                bits,
                bits,
                VOICE_PLAYEND,
                viewer.is_visible(),
                &mut Pictures,
                &mut random,
                &mut libc,
                &mut font,
            );
            if shown {
                let out = viewer.update([0.3, 0.0], 0.0, &mut libc);
                drawn.set((out.stage.len(), out.models.len()));
            }
            for r in screen.take_viewer_requests() {
                match r {
                    ViewerRequest::Show { chara, costume } => viewer.show(chara, costume, false),
                    ViewerRequest::Hide => viewer.hide(),
                }
            }
        };
        let mut press =
            |screen: &mut CharRefScreen, viewer: &mut crate::chr_view::ChrView, bits: u32| {
                step(screen, viewer, bits);
                for _ in 0..40 {
                    step(screen, viewer, 0);
                }
            };
        for _ in 0..200 {
            press(&mut screen, &mut viewer, 0);
            if screen.state == ST_LIST {
                break;
            }
        }
        assert_eq!(screen.state, ST_LIST);
        press(&mut screen, &mut viewer, PAD_DOWN);
        assert_eq!(screen.cursor, 1);
        press(&mut screen, &mut viewer, PAD_CROSS);
        assert_eq!(screen.state, ST_DETAILS);
        // The picture of the character is streamed in and fades in.
        assert_eq!(
            screen.textures[IMAGE_SLOT],
            Some((IMAGE_FILE + screen.cur().chara as u32) as usize)
        );
        assert_eq!(screen.image_alpha, 1.0);
        // The profile, then back.
        press(&mut screen, &mut viewer, PAD_CROSS);
        assert_eq!(screen.state, ST_PROFILE);
        assert!(screen.text_height > 0);
        press(&mut screen, &mut viewer, PAD_TRIANGLE);
        // The model: the costume page, then the viewer.
        for _ in 0..3 {
            press(&mut screen, &mut viewer, PAD_DOWN);
        }
        assert_eq!(screen.menu_cursor, 3);
        press(&mut screen, &mut viewer, PAD_CROSS);
        assert_eq!(screen.state, ST_COSTUME);
        press(&mut screen, &mut viewer, PAD_CROSS);
        assert_eq!(screen.state, ST_VIEW);
        assert!(viewer.is_visible());
        let drawn = drawn.get();
        assert!(drawn.0 > 0 && drawn.1 > 0, "{drawn:?}");
        // The backdrop's ambience: stage 5's loop at volume 0x50 (8 on the player).
        let sounds = viewer.take_sounds();
        assert!(
            sounds
                .iter()
                .any(|s| matches!(s, TitleSound::MapBgm(0x10BA9))),
            "{sounds:?}"
        );
        assert!(
            matches!(sounds.last(), Some(TitleSound::MapBgmVolume(8))),
            "{sounds:?}"
        );
        press(&mut screen, &mut viewer, PAD_TRIANGLE);
        assert_eq!(screen.state, ST_COSTUME);
        assert!(!viewer.is_visible());
        assert!(matches!(
            viewer.take_sounds().last(),
            Some(TitleSound::MapBgmStop)
        ));
        press(&mut screen, &mut viewer, PAD_TRIANGLE);
        press(&mut screen, &mut viewer, PAD_TRIANGLE);
        assert_eq!(screen.state, ST_LIST);
        press(&mut screen, &mut viewer, PAD_TRIANGLE);
        assert!(screen.is_done());
    }
}
