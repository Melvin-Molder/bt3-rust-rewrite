//! The wish screen (`Shen_Main`, src/ui/shen_wish.c, shen_confirm.c, shen_save.c; progress modes 70 and 71): with
//! the seven dragon balls gathered the main menu summons the dragon. Which one comes is drawn once
//! (`Shen_BuildList`); it lists four wishes and grants one, or (the second one) lists seven and grants three. A
//! wish is an item, a stage, a character or money; a confirmation window (with the item's details page) asks
//! first, the reward window announces it. Then the dragon leaves (`ShenScene`, the 3D backdrop) and the save
//! screen runs the memory card's save flow over a black screen (`ShenSave_Run`).

use std::error::Error;

use bt3_data::chara_data::ChrTable;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::dialog::{Dialog, Pad as DialogPad};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, Uv, screen_quads};
use crate::font::Font;
use crate::get_win::{self, GetWin, GetWinData};
use crate::item_panel::{ItemAssets, ItemHelp, help_textures_in};
use crate::logo::ColorFade;
use crate::mcflow::{self, Done, McEnv, McFlow, McIo};
use crate::menu_res::{FileKey, ImageTexture, decode_files, image_indices, tex};
use crate::rng::{GameRandom, LibcRandom, Vu0Random};
use crate::save::SaveData;
use crate::shen_scene::{STATE_ENDED, STATE_LEAVE, STATE_READY, SceneFrame, ShenScene};
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};
use crate::windows::{IconWin, MsgWin, TextBox, TextDraw, draw_texts};

/// `gMenuArc12`: file `gProgress->baseFile + 0x18`; `Shen_Run(2)` and `ShenSave_Run(2)` unpack its section 2.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 0x18;
pub const ARCHIVE_SECTION: usize = 2;
/// `ShenSave_Init`: the dialog file.
pub const DIALOG_SECTION: usize = 10;
/// The music of mode 70.
pub const BGM_FILE_ID: u32 = 0x10B18;
/// `Shen_Say`: the dragon's lines (no mouth keys).
const VOICE_BASE: u32 = 0x85B7;
/// `Pad_SetRepeat(20, 1)` while the screen runs.
const REPEAT_DELAY: i32 = 20;
const REPEAT_INTERVAL: i32 = 1;
const FADE_FRAMES: i32 = 20;

// SHEN_WISH_*.
const WISH_ITEM: u32 = 0;
const WISH_STAGE: u32 = 1;
const WISH_CHARA: u32 = 2;
const WISH_MONEY: u32 = 3;

// SHEN_DRAGON_*.
const DRAGON_1: usize = 1;
const DRAGON_2: usize = 2;

// SHEN_STATE_*.
const ST_SELECT: i32 = 0;
const ST_CONFIRM: i32 = 1;
const ST_GET: i32 = 2;
const ST_LEAVE: i32 = 3;

// SHEN_FLAG_*.
const FLAG_SHOWN: u32 = 1;
const FLAG_EXIT: u32 = 2;
const FLAG_FADING: u32 = 4;
const FLAG_OPENING: u32 = 8;
const FLAG_UNUSED40: u32 = 0x40;
const FLAG_GREETED: u32 = 0x100;
const FLAG_LIST_ON: u32 = 0x200;
const FLAG_BYE: u32 = 0x400;

/// The list's scissor (`Shen_ScissorList`: `Sprite_SetScissor(0, 0x200, 0x6A, 0x11E)`).
const LIST_CLIP: [i32; 4] = [0, 0x6A, 0x200, 0x11E];

/// `work->tex[n]` of `Shen_Init`: section 4's seven textures, [3] none.
const FLASH_TEXTURES: [ImageTexture; 8] = [
    tex(4, 0),
    tex(4, 1),
    tex(4, 2),
    None,
    tex(4, 3),
    tex(4, 4),
    tex(4, 5),
    tex(4, 6),
];
/// `IconWin_Init(MPACK_AT(res, 8), res 5)` and `MsgWin_Init(MPACK_AT(res, 9), …)`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((8, 1), 1)),
    Some(((8, 1), 0)),
    tex(5, 0),
    Some(((8, 1), 2)),
    tex(5, 1),
];
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((9, 1), 0)), Some(((9, 1), 1)), None];
/// `GetWin_Init(MPACK_AT(res, 11), 0)`.
const GET_WIN_PACK: usize = 11;
/// `ShenCfm_Init(MPACK_AT(res, 6))`: its textures, in the confirmation pack (`tex[n]`), and the item page's
/// (`ItemHelp_Init(MPACK_AT(pack, 4))`).
const CFM_SECTION: usize = 6;
const CFM_TEXTURES: [ImageTexture; 6] = [
    tex(2, 0),
    tex(2, 1),
    tex(2, 2),
    tex(3, 0),
    tex(2, 3),
    tex(3, 1),
];
const CFM_HELP_SECTION: usize = 4;

/// `ShenWish`: what a wish gives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wish {
    /// 1-based item, stage or character number, or the amount of money.
    pub id: i32,
    pub kind: u32,
}

/// What `Shen_Init` reads from its unpacked section.
pub struct ShenAssets {
    pub data: ShenData,
    /// Files of the screen's own sections, and of the confirmation window's pack (section 6).
    pub files: Vec<(FileKey, Vec<bt3_data::gs::RgbaTexture>)>,
    pub cfm_files: Vec<(FileKey, Vec<bt3_data::gs::RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct ShenData {
    pub fod: Vec<u8>,
    pub icon_fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    pub cfm_fod: Vec<u8>,
    /// Section 1: the dragon's lines and the wish names; section 13: the list rows' text.
    pub msg: Vec<u8>,
    pub font: Vec<u8>,
    /// Section 7: per item, its type (+0 of 0x28 bytes).
    pub item_types: Vec<u8>,
    /// Section 2: the three dragons' wish lists (4, 7, 4).
    pub wishes: [Vec<Wish>; 3],
    pub get_win: GetWinData,
    pub help: ItemAssets,
    pub layout: ShenLayout,
}

#[derive(Clone, Debug, Default)]
pub struct ShenLayout {
    pub flash: Vec<Option<usize>>,
    pub icon: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub get_win: Vec<Option<usize>>,
    pub cfm: Vec<Option<usize>>,
    pub help: Vec<Option<usize>>,
}

impl ShenLayout {
    /// `first` finds the screen's files, `cfm_first` the confirmation pack's.
    pub fn new(first: impl Fn(FileKey) -> usize, cfm_first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            flash: image_indices(&FLASH_TEXTURES, &first),
            icon: image_indices(&ICON_TEXTURES, &first),
            msg: image_indices(&MSG_TEXTURES, &first),
            get_win: image_indices(&get_win::textures(GET_WIN_PACK), &first),
            cfm: image_indices(&CFM_TEXTURES, &cfm_first),
            help: image_indices(&help_textures_in(CFM_HELP_SECTION), &cfm_first),
        }
    }
}

impl ShenAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let gw = get_win::textures(GET_WIN_PACK);
        let files = decode_files(
            res,
            [&FLASH_TEXTURES[..], &ICON_TEXTURES, &MSG_TEXTURES, &gw],
        )?;
        let cfm = pack_section(res, CFM_SECTION)?;
        let help = help_textures_in(CFM_HELP_SECTION);
        let cfm_files = decode_files(cfm, [&CFM_TEXTURES[..], &help])?;
        let list = pack_section(res, 2)?;
        let wish = |i: usize| -> Result<Wish, Box<dyn Error>> {
            let b = list.get(i * 8..i * 8 + 8).ok_or("short wish list")?;
            Ok(Wish {
                id: i32::from_le_bytes(b[0..4].try_into().unwrap()),
                kind: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            })
        };
        let range =
            |from: usize, n: usize| (from..from + n).map(wish).collect::<Result<Vec<_>, _>>();
        let help_pack = pack_section(cfm, CFM_HELP_SECTION)?;
        let sub = |n: usize| -> Result<Vec<u8>, Box<dyn Error>> {
            Ok(pack_section(help_pack, n)?.to_vec())
        };
        Ok(Self {
            data: ShenData {
                fod: pack_section(res, 3)?.to_vec(),
                icon_fod: pack_section(pack_section(res, 8)?, 2)?.to_vec(),
                msg_fod: pack_section(pack_section(res, 9)?, 2)?.to_vec(),
                cfm_fod: pack_section(cfm, 1)?.to_vec(),
                msg: pack_section(res, 1)?.to_vec(),
                font: pack_section(res, 13)?.to_vec(),
                item_types: pack_section(res, 7)?
                    .chunks_exact(0x28)
                    .map(|e| e[0])
                    .collect(),
                wishes: [range(0, 4)?, range(4, 7)?, range(11, 4)?],
                get_win: GetWinData::parse_lang(pack_section(res, GET_WIN_PACK)?, 0)?,
                help: ItemAssets {
                    help_fod: sub(2)?,
                    help_name: sub(6)?,
                    help_desc: sub(5)?,
                    ..ItemAssets::default()
                },
                layout: ShenLayout::default(),
            },
            files,
            cfm_files,
        })
    }
}

/// `Shen_IsOwned` / `ShenCfm_IsOwned`: whether the save has what a wish gives (`n` 0-based).
fn is_owned(save: &SaveData, kind: u32, n: i32) -> bool {
    match kind {
        WISH_ITEM => usize::try_from(n).is_ok_and(|n| save.item(n) & 1 != 0),
        WISH_STAGE => (0..64).contains(&n) && save.stage_bits >> n & 1 != 0,
        WISH_CHARA => save.chara_unlocked(n),
        _ => false,
    }
}

/// `ShenCfm`: yes / details / no, with the item's details page.
struct Confirm {
    flash: Flash,
    tex: Vec<Option<usize>>,
    help: ItemHelp,
    cursor: i32,
    detail: bool,
    kind: u32,
    id: i32,
    has_detail: bool,
}

impl Confirm {
    fn plate(&mut self, n: i32) -> crate::flash::ClipRef {
        self.flash
            .find(None, &format!("mc_reconfir_plate_{}", n + 1))
    }

    /// `ShenCfm_SetPlate`.
    fn set_plate(&mut self, on: bool) {
        let r = self.plate(self.cursor);
        self.flash
            .clip_goto_label(r, if on { "fl_on_start" } else { "fl_off_start" });
    }

    /// `ShenCfm_Open`: the cursor on "no".
    fn open(&mut self, kind: u32, id: i32) {
        self.id = id;
        self.kind = kind;
        self.set_plate(false);
        self.cursor = 1;
        self.set_plate(true);
        self.has_detail = kind == WISH_ITEM;
        self.flash.goto_label("fl_window_in", true);
    }

    /// `ShenCfm_Close`.
    fn close(&mut self) {
        self.flash.goto_label("fl_window_out", true);
    }

    /// `ShenCfm_MoveCursor`.
    fn move_cursor(&mut self, dir: i32, sounds: &mut Vec<TitleSound>) {
        self.set_plate(false);
        self.detail = false;
        self.cursor += dir;
        if self.cursor < 0 {
            self.cursor = 2;
        } else if self.cursor > 2 {
            self.cursor = 0;
        }
        self.set_plate(true);
        sounds.push(TitleSound::Effect(1, 0));
    }

    /// `ShenCfm_Update`: an answer (0 yes, 1 no or cancel) once one is given. A dimmed choice only buzzes;
    /// "details" shows the item page until confirm or cancel.
    fn update(&mut self, pad: Pad, save: &SaveData, sounds: &mut Vec<TitleSound>) -> Option<i32> {
        if self.detail {
            if pad.pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                self.detail = false;
                self.help.close();
            }
            return None;
        }
        if pad.repeat & PAD_UP != 0 {
            self.move_cursor(-1, sounds);
        } else if pad.repeat & PAD_DOWN != 0 {
            self.move_cursor(1, sounds);
        } else if pad.pressed & PAD_CROSS != 0 {
            let r = self.plate(self.cursor);
            self.flash.clip_goto_label(r, "fl_ok");
            let owned = is_owned(save, self.kind, self.id);
            if (!self.has_detail && self.cursor == 2) || (owned && self.cursor == 0) {
                sounds.push(TitleSound::Effect(1, 7));
            } else {
                sounds.push(TitleSound::Effect(1, 1));
                if self.cursor == 2 {
                    self.detail = true;
                    self.help.open();
                } else {
                    return Some(self.cursor);
                }
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            sounds.push(TitleSound::Effect(1, 2));
            return Some(1);
        }
        None
    }

    /// `ShenCfm_Draw`: "details" dimmed for a wish that is not an item, "yes" for one the save has (and left so).
    fn draw(
        &mut self,
        save: &SaveData,
        chr: &ChrTable,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        self.flash.advance();
        for i in 0..3 {
            let name = format!("mc_reconfir_plate_{}", i + 1);
            let uv = Uv {
                u0: 0,
                v0: i << 5,
                u1: 0x100,
                v1: (i << 5) + 0x20,
                tex: 0,
            };
            for part in ["mc_reconfir_text_on", "mc_reconfir_text_off"] {
                let r = self.flash.find(Some(&name), part);
                self.flash.clip_set_uv(r, uv);
            }
        }
        if !self.has_detail {
            let r = self.plate(2);
            self.flash.clip_set_color(r, 0.5);
        }
        if is_owned(save, self.kind, self.id) {
            let r = self.plate(0);
            self.flash.clip_set_color(r, 0.5);
        }
        let tex = &self.tex;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        quads.extend(screen_quads(&self.flash.draw(&has_texture), tex));
        if self.kind == WISH_ITEM {
            self.help.draw(self.id.max(0) as usize, chr, font, quads);
        }
    }
}

/// What a frame of the wish screen draws: the 3D backdrop under the screen's quads.
pub struct ShenFrame {
    pub scene: SceneFrame,
    pub quads: Vec<ScreenQuad>,
    pub sounds: Vec<TitleSound>,
}

/// `ShenWork` and the frame loop of `Shen_Run`.
pub struct ShenScreen {
    data: ShenData,
    flash: Flash,
    icon: IconWin,
    msg: MsgWin,
    get_win: GetWin,
    cfm: Confirm,
    scene: ShenScene,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    line: i32,
    state: i32,
    answer: i32,
    dragon: usize,
    granted: i32,
    wish_max: i32,
    /// The wish list (`ShenNode`s numbered 0..n), the wish in the first row and the one scrolling out.
    list: Vec<Wish>,
    top: usize,
    out: usize,
    cursor: i32,
    flags: u32,
    timer: i32,
    done: bool,
}

impl ShenScreen {
    /// `Shen_Init(2)` (with `Shen_BuildList`'s draw of the dragon and its scene) and the first
    /// `ColorFade_StartIn`. The balls are used up: `unlockFlags` loses bits 0..6.
    pub fn new(
        data: &ShenData,
        save: &mut SaveData,
        random: &mut GameRandom,
        read: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    ) -> Result<Self, Box<dyn Error>> {
        // Created, not played: the greeting starts the movie.
        let flash = Flash::create(&data.fod)?;
        let icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        let msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg.clone(),
            false,
        )?;
        let mut cfm_flash = Flash::create(&data.cfm_fod)?;
        cfm_flash.play(1);
        let cfm = Confirm {
            flash: cfm_flash,
            tex: data.layout.cfm.clone(),
            help: ItemHelp::new(&data.help, data.layout.help.clone())?,
            cursor: 0,
            detail: false,
            kind: 0,
            id: 0,
            has_detail: false,
        };
        let get_win = GetWin::new(&data.get_win, data.layout.get_win.clone())?;
        // Shen_BuildList: one Rand_Range(100) roll; the third dragon only with bit 0 of slot 8's flags.
        let (from1, from2, end2) = if save.slot_flags(8) & 1 != 0 {
            (40, 60, 100)
        } else {
            (50, 100, 200)
        };
        let roll = random.range(100) as i32;
        let (dragon, wish_max) = if (from2..end2).contains(&roll) {
            (DRAGON_2, 1)
        } else if (from1..from2).contains(&roll) {
            (DRAGON_1, 3)
        } else {
            (0, 1)
        };
        let scene = ShenScene::new(read, dragon)?;
        save.set_unlock_flags(save.unlock_flags() & !0x7F);
        Ok(Self {
            data: data.clone(),
            flash,
            icon,
            msg,
            get_win,
            cfm,
            scene,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            line: 0,
            state: ST_SELECT,
            answer: 0,
            dragon,
            granted: 0,
            wish_max,
            list: data.wishes[dragon].clone(),
            top: 0,
            out: 0,
            cursor: 0,
            flags: 0,
            timer: 0,
            done: false,
        })
    }

    /// `Shen_Run` has returned: on to the save screen.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The backdrop, for its textures.
    pub(crate) fn scene(&self) -> &ShenScene {
        &self.scene
    }

    /// `ShenList_Step`: `steps` nodes on (back when negative) around the circular list.
    fn step(&self, node: usize, steps: i32) -> usize {
        let n = self.list.len() as i32;
        (node as i32 + steps).rem_euclid(n.max(1)) as usize
    }

    fn wish_at(&self, node: usize) -> Wish {
        self.list.get(node).copied().unwrap_or_default()
    }

    /// The wish under the cursor (`ShenList_StepWish(work->top, work->cursor)`).
    fn cur_wish(&self) -> Wish {
        self.wish_at(self.step(self.top, self.cursor))
    }

    /// `Shen_Say`.
    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        anim::play_with_subtitle(&[], VOICE_BASE, line, &mut self.lip, sounds);
        self.line = line;
    }

    /// `Shen_SetPlate`.
    fn set_plate(&mut self, row: i32, on: bool) {
        let r = self.flash.find(None, &format!("mc_menu_plate_{}", row + 1));
        self.flash
            .clip_goto_label(r, if on { "fl_on_start" } else { "fl_off_start" });
    }

    /// `Shen_MoveCursor`: up (0) or down (1); past the four rows the list scrolls by one wish.
    fn move_cursor(&mut self, dir: i32, sounds: &mut Vec<TitleSound>) {
        self.set_plate(self.cursor, false);
        self.cursor += if dir == 0 { -1 } else { 1 };
        let clamped = self.cursor.clamp(0, 3);
        if clamped != self.cursor {
            self.cursor = clamped;
            if dir == 0 {
                self.top = self.step(self.top, -1);
                self.flash.goto_label("fl_list_down", true);
                self.out = self.step(self.top, 4);
            } else {
                self.top = self.step(self.top, 1);
                self.flash.goto_label("fl_list_up", true);
                self.out = self.step(self.top, -1);
            }
        }
        self.set_plate(self.cursor, true);
        sounds.push(TitleSound::Effect(1, 0));
    }

    /// One iteration of `Shen_Run`'s loop (a 30 Hz frame). `voice_stat` is `Voice_GetStat(0)`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        save: &mut SaveData,
        chr: &ChrTable,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
        font: &mut Font,
    ) -> ShenFrame {
        let mut out = ShenFrame {
            scene: SceneFrame::default(),
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done {
            return out;
        }
        // Gfx_BeginFrame: Fade_UpdateAll.
        self.scene.fade.update();
        let repeat = self
            .pad
            .update_with(held, pressed, REPEAT_DELAY, REPEAT_INTERVAL);
        self.fade.update();
        // Shen_Advance.
        self.flash.advance();
        out.scene = self.draw(save, chr, libc, vu0, font, &mut out.quads);
        out.sounds.append(&mut out.scene.sounds);
        out.quads.extend(self.fade.quad().map(ScreenQuad::from));
        // Gfx_EndFrame: Fade_DrawScreen (the scene's fade slot) over everything.
        out.quads
            .extend(self.scene.fade.quad().map(ScreenQuad::from));
        if self.update_exit(&mut out.sounds) {
            self.done = true;
            return out;
        }
        let pad = Pad { pressed, repeat };
        self.update(pad, voice_stat, save, random, &mut out.sounds);
        out
    }

    /// `Shen_Draw`: the windows open once the backdrop is ready; the list, the backdrop, the movie and the
    /// windows.
    fn draw(
        &mut self,
        save: &SaveData,
        chr: &ChrTable,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) -> SceneFrame {
        if self.flags & FLAG_SHOWN == 0 {
            if self.scene.state() == STATE_READY {
                self.flags |= FLAG_SHOWN | FLAG_OPENING;
            }
        } else if self.flags & FLAG_OPENING != 0 {
            self.icon.open();
            self.msg.open();
            self.flags ^= FLAG_OPENING;
        }
        let texts = self.draw_list(save, font);
        let scene = self.scene.update(libc, vu0);
        let tex = &self.data.layout.flash;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&flash_quads, tex));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
        self.icon.draw(quads);
        self.get_win.draw(font, quads);
        self.msg.draw(self.line, font, quads);
        self.cfm.draw(save, chr, font, quads);
        // Dialog_Draw(0): the dialog is never started here.
        scene
    }

    /// `Shen_DrawList`: per row (the fifth: the wish scrolling out) the icon by the kind of wish, the list's
    /// scissor, half brightness for a wish the save has, and its name.
    fn draw_list(&mut self, save: &SaveData, font: &Font) -> Vec<TextDraw> {
        let arrow = |u0, v0| Uv {
            u0,
            v0,
            u1: u0 + 0x20,
            v1: v0 + 0x20,
            tex: 0,
        };
        let up = self
            .flash
            .find(Some("mc_yajirusi_up"), "mc_yajirusi_icon_up");
        self.flash.clip_set_uv(up, arrow(0, 0x20));
        let down = self
            .flash
            .find(Some("mc_yajirusi_down"), "mc_yajirusi_icon_down");
        self.flash.clip_set_uv(down, arrow(0x20, 0x20));
        let mut texts = Vec::new();
        let list_box = TextBox::preset(2).with_clip(0, 0x200, 0x6A, 0x11E);
        let mut node = self.top;
        for i in 0..5 {
            let name = format!("mc_menu_plate_{}", i + 1);
            let wish = self.wish_at(node);
            let (u0, v0) = match wish.kind {
                WISH_ITEM => {
                    let ty = usize::try_from(wish.id - 1)
                        .ok()
                        .and_then(|i| self.data.item_types.get(i).copied())
                        .unwrap_or(0);
                    let cell = match ty {
                        0 => 3,
                        2 => 1,
                        _ => 2,
                    };
                    (cell << 6, 0)
                }
                WISH_STAGE => (0x40, 0x40),
                WISH_CHARA => (0, 0x40),
                _ => (0, 0),
            };
            let uv = Uv {
                u0,
                v0,
                u1: u0 + 0x40,
                v1: v0 + 0x40,
                tex: 0,
            };
            let icon = self.flash.find(Some(&name), "mc_icon_play");
            self.flash.clip_set_uv(icon, uv);
            self.flash.clip_set_scissor(icon, Some(LIST_CLIP));
            let plate = self.flash.find(None, &name);
            self.flash.clip_set_scissor(plate, Some(LIST_CLIP));
            let shade = if is_owned(save, wish.kind, wish.id - 1) {
                0.5
            } else {
                1.0
            };
            self.flash.clip_set_color(plate, shade);
            let text = self.flash.find(Some(&name), "mc_menu_text_off");
            let no = node as i32;
            let line = match self.dragon {
                DRAGON_1 => no + 4,
                DRAGON_2 => no + 11,
                _ => no,
            };
            texts.extend(list_box.attach_line(
                &mut self.flash,
                text,
                0x400 + i,
                &self.data.font,
                line,
                font,
            ));
            node = if i == 3 { self.out } else { self.step(node, 1) };
        }
        texts
    }

    /// `Shen_PressedOk`: confirm on pad 0, with its sound.
    fn pressed_ok(pad: Pad, sounds: &mut Vec<TitleSound>) -> bool {
        if pad.pressed & PAD_CROSS != 0 {
            sounds.push(TitleSound::Effect(1, 1));
            return true;
        }
        false
    }

    /// `Shen_UpdateTalk`: the greeting and the farewell; true while the list may not be used.
    fn update_talk(
        &mut self,
        pad: Pad,
        voice_stat: i32,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) -> bool {
        let d1 = self.dragon == DRAGON_1;
        if self.flags & FLAG_GREETED == 0 {
            let mut line = random.range(2) as i32;
            if !d1 && line == 1 {
                line = 2;
            }
            self.say(if d1 { line + 8 } else { line }, sounds);
            self.flags |= FLAG_GREETED;
            return true;
        }
        let greet = if d1 { 8..=10 } else { 0..=2 };
        let bye = if d1 { 15 } else { 7 };
        if greet.contains(&self.line) {
            if voice_stat == VOICE_PLAYEND || Self::pressed_ok(pad, sounds) {
                self.flash.play(1);
            }
            if self.flags & FLAG_LIST_ON == 0 && self.flash.flags() & FLASH_PAD != 0 {
                self.set_plate(self.cursor, true);
                self.flags |= FLAG_LIST_ON;
            }
            false
        } else if self.line == bye && self.flags & FLAG_BYE == 0 {
            if voice_stat == VOICE_PLAYEND || Self::pressed_ok(pad, sounds) {
                self.msg.close();
                self.icon.close();
                self.timer = 40;
                self.line = -1;
                self.flags |= FLAG_BYE;
            }
            true
        } else if self.flags & FLAG_BYE != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.scene.set_state(STATE_LEAVE);
                self.flags ^= FLAG_BYE;
            }
            true
        } else {
            false
        }
    }

    /// `Shen_Update`: the cursor, the confirmation, granting the wish, the reward window.
    fn update(
        &mut self,
        pad: Pad,
        voice_stat: i32,
        save: &mut SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flags & FLAG_EXIT != 0 || self.scene.state() != STATE_READY {
            return;
        }
        if self.get_win.is_anim_done() {
            return;
        }
        if self.update_talk(pad, voice_stat, random, sounds) {
            return;
        }
        if self.flash.flags() & FLASH_PAD == 0 {
            return;
        }
        match self.state {
            ST_SELECT => {
                if pad.repeat & PAD_UP != 0 {
                    self.move_cursor(0, sounds);
                } else if pad.repeat & PAD_DOWN != 0 {
                    self.move_cursor(1, sounds);
                } else if pad.pressed & PAD_CROSS != 0 {
                    let wish = self.cur_wish();
                    self.state = ST_CONFIRM;
                    let r = self
                        .flash
                        .find(None, &format!("mc_menu_plate_{}", self.cursor + 1));
                    self.flash.clip_goto_label(r, "fl_ok");
                    self.cfm.open(wish.kind, wish.id - 1);
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            ST_CONFIRM => {
                if let Some(answer) = self.cfm.update(pad, save, sounds) {
                    self.answer = answer;
                    match answer {
                        0 => self.grant(save, random, sounds),
                        1 => {
                            self.state = ST_SELECT;
                            self.cfm.close();
                        }
                        _ => {}
                    }
                }
            }
            ST_GET => {
                if pad.pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                    let got = self.cur_wish();
                    // The 0xA0th character comes with the next one, announced after it.
                    if got.kind == WISH_CHARA && got.id == 0xA0 && !is_owned(save, got.kind, got.id)
                    {
                        let id = got.id as usize;
                        save.chara_bits[id / 64] |= 1 << (id % 64);
                        self.get_win.setup(get_win::KIND_CHARA, got.id);
                        self.get_win.next(sounds);
                    } else {
                        self.get_win.close();
                        if self.wish_max == self.granted {
                            let bye = if self.dragon == DRAGON_1 { 15 } else { 7 };
                            self.say(bye, sounds);
                            self.flash.goto_label("fl_out", true);
                            self.state = ST_LEAVE;
                        } else {
                            self.state = ST_SELECT;
                        }
                    }
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            _ => {}
        }
    }

    /// "Yes": the wish is granted, the reward window announces it, and the dragon says one of its two lines.
    fn grant(
        &mut self,
        save: &mut SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        self.granted += 1;
        self.state = ST_GET;
        self.flags |= FLAG_UNUSED40;
        self.cfm.close();
        // Shen_SetupGetWin.
        let wish = self.cur_wish();
        match wish.kind {
            WISH_ITEM => self.get_win.setup(get_win::KIND_ITEM, wish.id - 1),
            WISH_STAGE => self.get_win.setup(get_win::KIND_STAGE, wish.id - 1),
            WISH_CHARA => self.get_win.setup(get_win::KIND_CHARA, wish.id - 1),
            WISH_MONEY => self.get_win.setup(get_win::KIND_MONEY, wish.id),
            _ => {}
        }
        self.get_win.open(sounds);
        let mut line = random.range(2) as i32;
        if line == 0 {
            line = 2;
        }
        self.say(
            if self.dragon == DRAGON_1 {
                line + 11
            } else {
                line + 3
            },
            sounds,
        );
        let n = wish.id - 1;
        match wish.kind {
            WISH_ITEM => {
                if let Ok(n) = usize::try_from(n) {
                    save.add_item(n);
                }
            }
            WISH_STAGE => save.stage_bits |= 1u64.wrapping_shl(n as u32),
            WISH_CHARA => {
                if let Ok(n) = usize::try_from(n)
                    && n / 64 < save.chara_bits.len()
                {
                    save.chara_bits[n / 64] |= 1 << (n % 64);
                }
            }
            WISH_MONEY => save.add_money(wish.id),
            _ => {}
        }
    }

    /// `Shen_UpdateExit`: once the backdrop has ended, the fade out with the sound fading along; true when black.
    fn update_exit(&mut self, sounds: &mut Vec<TitleSound>) -> bool {
        if self.scene.state() == STATE_ENDED && self.flags & FLAG_EXIT == 0 {
            self.flags |= FLAG_EXIT;
        }
        if self.flags & FLAG_EXIT != 0 {
            if self.flags & FLAG_FADING == 0 {
                self.flags |= FLAG_FADING;
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
            if self.fade.is_fading_out() {
                // Voice_FadeOutStep(0), and Bgm_FadeOutStep: the result is never written (0).
                sounds.push(TitleSound::FadeOutStep);
            } else {
                return true;
            }
        }
        false
    }
}

/// The pad words the screen reads (`gamePressed`, `gameRepeat`).
#[derive(Clone, Copy, Debug)]
struct Pad {
    pressed: u32,
    repeat: u32,
}

/// `ShenSave`: the memory card's save flow over a black screen (progress mode 71), then the main menu.
pub struct ShenSaveScreen {
    fade: ColorFade,
    dialog: Dialog,
    mcflow: McFlow,
    pad: PadRepeat,
    flags: u32,
    timer: i32,
    done: bool,
}

impl ShenSaveScreen {
    /// `ShenSave_Init(2)` (the dialog of section 10) and `ColorFade_StartIn`.
    pub fn new(dialog: Dialog, card_res: Option<&[u8]>) -> Self {
        Self {
            fade: ColorFade::start_in(true, FADE_FRAMES),
            dialog,
            mcflow: McFlow::new(card_res, None),
            pad: PadRepeat::default(),
            flags: 0,
            timer: 0,
            done: false,
        }
    }

    /// `ShenSave_Run` has returned.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// One frame of `ShenSave_Run`'s loop: the flow (save, mode 0) starts once the fade in is done; 15 frames
    /// after its "done" callback (either answer) the screen leaves, the first of them starting a fade out.
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        font: &mut Font,
        mc: &mut McEnv,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done {
            return frame;
        }
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        let mut effects = Vec::new();
        let mut io = McIo {
            dialog: &mut self.dialog,
            pad: DialogPad { pressed, repeat },
            sounds: &mut effects,
        };
        let (_, events) = self.mcflow.update(mc, &mut io, font, &mut frame.quads);
        if events
            .iter()
            .any(|e| matches!(e, mcflow::Event::Done(Done::Done | Done::NotDone)))
        {
            self.flags |= 1;
            self.timer = 15;
        }
        frame.sounds.extend(
            effects
                .into_iter()
                .map(|(bank, id)| TitleSound::Effect(bank, id)),
        );
        // ShenSave_UpdateExit.
        if self.flags & 1 != 0 {
            if self.flags & 2 == 0 {
                self.flags |= 2;
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
            self.timer -= 1;
            if self.timer == 0 {
                self.done = true;
                return frame;
            }
        }
        if self.fade.is_in_done() && self.flags & 4 == 0 {
            self.mcflow.start(mcflow::MODE_SAVE, mc);
            self.flags |= 4;
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the real wish screen from the ISO in `rom/`, when it is there: the summoning until the dragon is
    /// there, its greeting, the first wish chosen and confirmed (granted to the save), the reward window, the
    /// farewell and the end of the scene.
    #[test]
    fn real_wish_screen_grants_a_wish() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
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
        let assets = ShenAssets::parse(&res).unwrap();
        eprintln!("{:?}", assets.data.wishes);
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut cfm_firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.cfm_files {
            cfm_firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = ShenLayout::new(|k| firsts[&k], |k| cfm_firsts[&k]);
        let mut save = SaveData::default();
        save.set_unlock_flags(0x7F);
        let chr = ChrTable::parse(&read(4).unwrap(), 0xA1);
        let mut random = GameRandom::from_key(&[1, 2, 3]);
        let mut libc = LibcRandom::new(1);
        let mut vu0 = Vu0Random::default();
        let mut font = Font::new(Vec::new(), 20);
        let mut screen = ShenScreen::new(&data, &mut save, &mut random, &mut read).unwrap();
        assert_eq!(save.unlock_flags() & 0x7F, 0);
        let dragon = screen.dragon;
        let models = std::cell::Cell::new(0);
        let se = std::cell::RefCell::new(Vec::new());
        let mut step = |screen: &mut ShenScreen, save: &mut SaveData, bits: u32| {
            let out = screen.run_frame(
                bits,
                bits,
                VOICE_PLAYEND,
                save,
                &chr,
                &mut random,
                &mut libc,
                &mut vu0,
                &mut font,
            );
            models.set(models.get().max(out.scene.models.len()));
            se.borrow_mut()
                .extend(out.sounds.iter().filter_map(|s| match s {
                    TitleSound::SeEx { id, .. } => Some(*id),
                    _ => None,
                }));
        };
        let mut frames = 0;
        while screen.scene.state() != STATE_READY && frames < 3000 {
            step(&mut screen, &mut save, 0);
            frames += 1;
        }
        eprintln!(
            "dragon {dragon} ready after {frames} frames, se {:X?}",
            se.borrow()
        );
        assert_eq!(screen.scene.state(), STATE_READY);
        assert!(models.get() > 0);
        assert!(se.borrow().contains(&0x3D) && se.borrow().contains(&0x3E));
        // The greeting, then the list takes input.
        for _ in 0..60 {
            step(&mut screen, &mut save, 0);
        }
        assert!(screen.flags & FLAG_LIST_ON != 0);
        let wish = screen.cur_wish();
        step(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.state, ST_CONFIRM);
        for _ in 0..20 {
            step(&mut screen, &mut save, 0);
        }
        // The cursor starts on "no": up to "yes".
        step(&mut screen, &mut save, PAD_UP);
        step(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.state, ST_GET, "{wish:?}");
        assert!(is_owned(&save, wish.kind, wish.id - 1) || wish.kind == WISH_MONEY);
        for _ in 0..90 {
            step(&mut screen, &mut save, 0);
        }
        while screen.state == ST_GET {
            step(&mut screen, &mut save, PAD_CROSS);
            for _ in 0..90 {
                step(&mut screen, &mut save, 0);
            }
            if screen.state == ST_SELECT {
                // Dragon 1 grants three: the same row again is owned; the next one.
                step(&mut screen, &mut save, PAD_DOWN);
                step(&mut screen, &mut save, PAD_CROSS);
                for _ in 0..20 {
                    step(&mut screen, &mut save, 0);
                }
                step(&mut screen, &mut save, PAD_UP);
                step(&mut screen, &mut save, PAD_UP);
                step(&mut screen, &mut save, PAD_CROSS);
                for _ in 0..90 {
                    step(&mut screen, &mut save, 0);
                }
            }
        }
        assert_eq!(screen.state, ST_LEAVE);
        let mut frames = 0;
        while !screen.is_done() && frames < 3000 {
            step(&mut screen, &mut save, 0);
            frames += 1;
        }
        eprintln!("left after {frames} frames, se {:X?}", se.borrow());
        assert!(screen.is_done());
        assert!(se.borrow().contains(&0x3F) && se.borrow().contains(&0x40));
    }
}
