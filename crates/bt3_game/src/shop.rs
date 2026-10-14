//! Evolution Z's item shop (`Shop_Run(2)`, src/menu/shop.c, progress mode 50): one of two guides (Lunch, calm or
//! wild; they swap after a minute without input), the buy list of what the shop sells at the save's stock level
//! and the collection list of every item, each with four tabs (all, then by item kind). An item is bought through
//! a confirmation window (with its details page); the collection list shows the details of an owned item. The
//! stock level rises once when all collectable items but one are owned; at 100 % the shop says "complete".

use std::error::Error;

use bt3_data::chara_data::{ChrTable, ItemInfo};
use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::item_panel::{ItemAssets, ItemHelp, help_textures_in};
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::windows::{IconWin, MsgWin, TextBox, TextDraw, draw_texts};

/// `Shop_Run(2)`: section 2 of `gMenuArc7`.
pub const ARCHIVE_SECTION: usize = 2;
/// `SHOP_VOICE_BASE`, `SHOP_GUIDE_LINES`.
const VOICE_BASE: u32 = 0x86DB;
const GUIDE_LINES: i32 = 15;
const ITEM_MAX: usize = 350;
const TABS: usize = 4;
const FADE_FRAMES: i32 = 0x14;
/// `PADG_L1` / `PADG_R1`: a page up / down (pad 0 is taken as a pad of status other than 1).
const PAD_L1: u32 = 0x4000;
const PAD_R1: u32 = 0x1_0000;

// WITEM_*.
const ITEM_HIDDEN: i32 = 5;
const ITEM_SOLD: i32 = 0x20;
const ITEM_LISTED: i32 = 0x40;
const ITEM_UNCOUNTED: i32 = 0x100;
// SAVE_ITEM_*.
const OWNED: u8 = 1;
const NEW: u8 = 2;

// The movies.
const FL_MAIN: usize = 0;
const FL_BUY: usize = 1;
const FL_ALL: usize = 2;
const FL_GUIDE: usize = 3;
const FL_CONFIRM: usize = 4;
const FL_COMPLETE: usize = 5;

// SHOP_LV_*.
const LV_MENU: usize = 0;
const LV_BUY: usize = 1;
const LV_ALL: usize = 2;
const LV_HELP: usize = 3;
const LV_CONFIRM: usize = 4;
const LV_BOUGHT: usize = 5;

// SHOP_*.
const STARTED: u32 = 4;
const GREETED: u32 = 8;
const LIST_BUY: u32 = 0x10;
const MSG_BEHIND: u32 = 0x80;

// SHOP_LINE_*.
const LINE_GREET: i32 = 0;
const LINE_GREET_NEW: i32 = 2;
const LINE_BACK: i32 = 4;
const LINE_LIST: i32 = 5;
const LINE_CONFIRM: i32 = 6;
const LINE_THANKS: i32 = 7;
const LINE_NO_MONEY: i32 = 8;
const LINE_COMPLETE: i32 = 9;
const LINE_SOLD_OUT: i32 = 10;
const LINE_OWNED: i32 = 11;
const LINE_ALL_OWNED: i32 = 12;
const LINE_SWAP: i32 = 13;
const LINE_BYE: i32 = 14;

/// `Shop_SetWindowScissor` and `Shop_SetListScissor` by page.
const WINDOW_CLIP: [i32; 4] = [0xA4, 0x3D, 0x1C1, 0x12D];
const LIST_CLIPS: [[i32; 4]; 2] = [[0, 0x86, 0x200, 0x116], [0, 0x86, 0x200, 0x17F]];

/// The six movies' sections and texture tables (`tex0`, `tex1`, `tex2`, `tex3`, `tex4`, `tex5`).
const FODS: [usize; 6] = [6, 17, 19, 9, 29, 33];
const TEX0: [ImageTexture; 15] = [
    tex(31, 0),
    tex(31, 2),
    tex(31, 1),
    tex(31, 3),
    tex(2, 0),
    tex(3, 0),
    tex(2, 2),
    tex(2, 1),
    tex(3, 1),
    tex(4, 0),
    tex(5, 0),
    tex(5, 2),
    tex(5, 1),
    tex(4, 1),
    tex(4, 2),
];
const TEX1: [ImageTexture; 21] = [
    tex(13, 4),
    tex(10, 0),
    tex(11, 0),
    tex(11, 1),
    tex(11, 2),
    tex(12, 0),
    tex(12, 1),
    tex(13, 0),
    tex(13, 2),
    tex(14, 1),
    tex(13, 1),
    tex(13, 3),
    tex(14, 0),
    tex(16, 0),
    tex(16, 0),
    None,
    tex(15, 1),
    tex(15, 0),
    tex(12, 2),
    tex(14, 2),
    tex(24, 1),
];
const TEX2: [ImageTexture; 18] = [
    tex(13, 4),
    tex(18, 1),
    tex(18, 0),
    tex(13, 0),
    tex(13, 2),
    tex(14, 1),
    tex(13, 1),
    tex(13, 3),
    tex(14, 0),
    tex(16, 0),
    tex(16, 0),
    None,
    tex(15, 1),
    tex(15, 0),
    tex(11, 0),
    tex(11, 1),
    tex(11, 2),
    tex(24, 1),
];
const TEX4: [ImageTexture; 6] = [
    tex(27, 0),
    tex(27, 3),
    tex(27, 1),
    tex(28, 0),
    tex(27, 2),
    tex(28, 1),
];
const TEX5: [ImageTexture; 1] = [tex(14, 3)];
/// `Shop_SwapGuide`: the guide movie's textures for guide 0 (section 7) and guide 1 (section 8).
const GUIDE_TEX: [[ImageTexture; 5]; 2] = [
    [tex(7, 0), None, None, tex(7, 1), tex(7, 3)],
    [tex(8, 0), tex(8, 1), tex(8, 3), None, None],
];
/// `IconWin_Init(MPACK_AT(res, 22), res 20)`, `MsgWin_Init(MPACK_AT(res, 21), …)`, `ItemHelp_Init(MPACK_AT(res, 25))`.
const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((22, 1), 1)),
    Some(((22, 1), 0)),
    tex(20, 0),
    Some(((22, 1), 2)),
    tex(20, 1),
];
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((21, 1), 0)), Some(((21, 1), 1)), None];
const HELP_SECTION: usize = 25;

/// What `Shop_Init` reads from its unpacked section.
pub struct ShopAssets {
    pub data: ShopData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct ShopData {
    pub fods: Vec<Vec<u8>>,
    pub icon_fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    /// Section 32: the guides' lines; 26: their mouth keys; 30: the item names.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    pub item_text: Vec<u8>,
    pub help: ItemAssets,
    pub layout: ShopLayout,
}

#[derive(Clone, Debug, Default)]
pub struct ShopLayout {
    pub background: usize,
    pub flash: Vec<Vec<Option<usize>>>,
    pub guide: [Vec<Option<usize>>; 2],
    pub icon: Vec<Option<usize>>,
    pub msg: Vec<Option<usize>>,
    pub help: Vec<Option<usize>>,
}

impl ShopLayout {
    pub fn new(background: usize, first: impl Fn(FileKey) -> usize) -> Self {
        let i = |t: &[ImageTexture]| image_indices(t, &first);
        Self {
            background,
            flash: vec![
                i(&TEX0),
                i(&TEX1),
                i(&TEX2),
                i(&GUIDE_TEX[0]),
                i(&TEX4),
                i(&TEX5),
            ],
            guide: [i(&GUIDE_TEX[0]), i(&GUIDE_TEX[1])],
            icon: i(&ICON_TEXTURES),
            msg: i(&MSG_TEXTURES),
            help: i(&help_textures_in(HELP_SECTION)),
        }
    }
}

impl ShopAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let help = help_textures_in(HELP_SECTION);
        let files = decode_files(
            res,
            [
                &TEX0[..],
                &TEX1,
                &TEX2,
                &TEX4,
                &TEX5,
                &GUIDE_TEX[0],
                &GUIDE_TEX[1],
                &ICON_TEXTURES,
                &MSG_TEXTURES,
                &help,
            ],
        )?;
        let sub = |s: usize, n: usize| -> Result<Vec<u8>, Box<dyn Error>> {
            Ok(pack_section(pack_section(res, s)?, n)?.to_vec())
        };
        Ok(Self {
            data: ShopData {
                fods: FODS
                    .iter()
                    .map(|&s| Ok(pack_section(res, s)?.to_vec()))
                    .collect::<Result<_, Box<dyn Error>>>()?,
                icon_fod: sub(22, 2)?,
                msg_fod: sub(21, 2)?,
                msg_text: pack_section(res, 32)?.to_vec(),
                lips: pack_section(res, 26)?.to_vec(),
                item_text: pack_section(res, 30)?.to_vec(),
                help: ItemAssets {
                    help_fod: sub(HELP_SECTION, 2)?,
                    help_name: sub(HELP_SECTION, 6)?,
                    help_desc: sub(HELP_SECTION, 5)?,
                    ..ItemAssets::default()
                },
                layout: ShopLayout::default(),
            },
            background: decode_picture(res, 1)?,
            files,
        })
    }
}

/// `ShopList`: one item list with its four tabs.
#[derive(Clone, Debug)]
struct List {
    cur: [i32; TABS],
    tab: usize,
    rows: i32,
    rows_f: f32,
    extra: i32,
    count: [i32; TABS],
    /// Per tab, the 0-based item numbers (the rest of the 350 zero, as the original's array).
    ids: [Vec<i32>; TABS],
    top: [i32; TABS],
    bottom: [i32; TABS],
}

impl List {
    fn id(&self, tab: usize, i: i32) -> i32 {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.ids[tab].get(i).copied())
            .unwrap_or(0)
    }

    /// `SHOP_ROW_ITEM(k, i)`.
    fn row(&self, i: i32) -> i32 {
        self.id(self.tab, self.top[self.tab] + i)
    }

    /// `SHOP_EXTRA_ITEM(k)`.
    fn extra_item(&self) -> i32 {
        self.id(self.tab, self.extra)
    }

    /// `SHOP_CUR_ITEM(k)`.
    fn cur_item(&self) -> i32 {
        self.id(self.tab, self.cur[self.tab])
    }
}

/// `Shop` and the frame loop of `Shop_Run`.
pub struct ShopScreen {
    layout: ShopLayout,
    lips: Vec<u8>,
    item_text: Vec<u8>,
    flash: Vec<Flash>,
    tex: Vec<Vec<Option<usize>>>,
    icon: IconWin,
    msg: MsgWin,
    help: ItemHelp,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    items: Vec<ItemInfo>,
    flags: u32,
    cursor: [i32; 6],
    level: usize,
    seq: i32,
    voice_line: i32,
    guide: i32,
    blink: i32,
    talk: i32,
    idle: i32,
    help_item: i32,
    item: i32,
    total: i32,
    owned: i32,
    percent: i32,
    cloud: f32,
    list: [List; 2],
    result: i32,
    /// `gProgress->flags |= 1`: something was bought.
    dirty: bool,
    done: Option<i32>,
}

/// `(f32)owned / total * 100`, at least 1 once anything is owned.
fn percent(owned: i32, total: i32) -> i32 {
    let p = (owned as f32 / total as f32 * 100.0) as i32;
    if owned != 0 && p == 0 { 1 } else { p }
}

impl ShopScreen {
    /// `Shop_Init(2)` (the stock level checked, the two lists built, one of the guides drawn) and the first
    /// `ColorFade_StartIn`.
    pub fn new(
        data: &ShopData,
        chr: &ChrTable,
        save: &mut SaveData,
        random: &mut GameRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Vec::new();
        for fod in &data.fods {
            let mut f = Flash::create(fod)?;
            f.play(1);
            flash.push(f);
        }
        let mut icon = IconWin::new(&data.icon_fod, data.layout.icon.clone())?;
        icon.open();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            true,
        )?;
        msg.open();
        let help = ItemHelp::new(&data.help, data.layout.help.clone())?;
        let items: Vec<ItemInfo> = (0..ITEM_MAX)
            .map(|i| chr.items.get(i).copied().unwrap_or_default())
            .collect();
        check_stock_level(&items, save);
        let mut total = 0;
        let mut owned = 0;
        let list = std::array::from_fn(|k| {
            let mut ids: [Vec<i32>; TABS] = Default::default();
            for (i, it) in items.iter().enumerate() {
                if it.flags & ITEM_HIDDEN != 0 || it.kind >= 3 || it.flags & ITEM_LISTED == 0 {
                    continue;
                }
                if k == 0
                    && (it.flags & ITEM_SOLD == 0 || (save.stock_level() as u32) < it.stock_level)
                {
                    continue;
                }
                ids[0].push(i as i32);
                ids[usize::from(it.kind) + 1].push(i as i32);
                if k == 1 && it.flags & ITEM_UNCOUNTED == 0 {
                    total += 1;
                    if save.item(i) & OWNED != 0 {
                        owned += 1;
                    }
                }
            }
            let rows = if k != 0 { 7 } else { 4 };
            List {
                cur: [0; TABS],
                tab: 0,
                rows,
                rows_f: if k != 0 { 7.28 } else { 4.0 },
                extra: 0,
                count: std::array::from_fn(|t| ids[t].len() as i32),
                ids,
                top: [0; TABS],
                bottom: [rows - 1; TABS],
            }
        });
        // Shop_SwapGuide after Rand_Range(2): the other guide talks.
        let guide = random.range(2) as i32 ^ 1;
        let mut tex: Vec<Vec<Option<usize>>> = data.layout.flash.clone();
        tex[FL_GUIDE] = data.layout.guide[guide as usize].clone();
        Ok(Self {
            layout: data.layout.clone(),
            lips: data.lips.clone(),
            item_text: data.item_text.clone(),
            flash,
            tex,
            icon,
            msg,
            help,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            items,
            flags: 0,
            cursor: [0; 6],
            level: LV_MENU,
            seq: 0,
            voice_line: -1,
            guide,
            blink: 0,
            talk: 0,
            idle: 0,
            help_item: 0,
            item: 0,
            total,
            owned,
            percent: percent(owned, total),
            cloud: 0.0,
            list,
            result: 1,
            dirty: false,
            done: None,
        })
    }

    /// `Shop_Run`'s result once it has returned: 0 left by cancel (`EvoMode_Main` ignores it).
    pub fn result(&self) -> Option<i32> {
        self.done
    }

    /// Something was bought (`gProgress->flags |= 1`).
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// `SHOP_SAY(n)`.
    fn say(&mut self, n: i32, sounds: &mut Vec<TitleSound>) {
        self.voice_line = self.guide * GUIDE_LINES + n;
        self.play_line(sounds);
    }

    fn play_line(&mut self, sounds: &mut Vec<TitleSound>) {
        anim::play_with_subtitle(
            &self.lips,
            VOICE_BASE,
            self.voice_line,
            &mut self.lip,
            sounds,
        );
    }

    /// `Shop_PlayVoice`: the page's line.
    fn play_voice(&mut self, sounds: &mut Vec<TitleSound>) {
        match self.cursor[0] {
            0 => self.voice_line = self.guide * GUIDE_LINES + 1,
            1 => self.voice_line = self.guide * GUIDE_LINES + 3,
            _ => {}
        }
        self.play_line(sounds);
    }

    /// `Shop_SwapGuide`.
    fn swap_guide(&mut self) {
        self.guide ^= 1;
        self.tex[FL_GUIDE] = self.layout.guide[self.guide as usize].clone();
    }

    /// `Shop_CanBuy`: not owned, not hidden and sold by the shop.
    fn can_buy(&self, save: &SaveData, item: i32) -> bool {
        let Some(e) = usize::try_from(item).ok().and_then(|i| self.items.get(i)) else {
            return false;
        };
        save.item(item as usize) & OWNED == 0
            && e.flags & ITEM_HIDDEN == 0
            && e.flags & ITEM_SOLD != 0
    }

    /// `Shop_IsSoldOut`.
    fn is_sold_out(&self, save: &SaveData) -> bool {
        let l = &self.list[0];
        (0..l.count[0]).all(|i| save.item(l.id(0, i).max(0) as usize) & OWNED != 0)
    }

    fn item_info(&self, item: i32) -> ItemInfo {
        usize::try_from(item)
            .ok()
            .and_then(|i| self.items.get(i).copied())
            .unwrap_or_default()
    }

    /// `Shop_SetPlate(flash, level, tab, label)`.
    fn set_plate(&mut self, flash: usize, level: usize, tab: bool, label: &str) {
        let name = match level {
            LV_MENU => format!("menu_plate_{}", self.cursor[LV_MENU] + 1),
            LV_BUY | LV_ALL => {
                let l = &self.list[level - 1];
                if tab {
                    format!("mc_tab_btn_{}", l.tab)
                } else {
                    format!("mc_list_plate_{}", l.cur[l.tab] - l.top[l.tab] + 1)
                }
            }
            LV_CONFIRM => format!("mc_reconfir_plate_{}", self.cursor[LV_CONFIRM] + 1),
            _ => return,
        };
        let r = self.flash[flash].find(None, &name);
        self.flash[flash].clip_goto_label(r, label);
    }

    /// One iteration of `Shop_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        save: &mut SaveData,
        chr: &ChrTable,
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
        let pad = Pad { pressed, repeat };
        self.update(voice_stat, &mut frame.sounds);
        self.update_seq(pad, voice_stat, &mut frame.sounds);
        self.draw(save, chr, voice_stat, random, font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & GREETED == 0
            && self.flash[FL_MAIN].flags() & FLASH_PAD != 0
        {
            self.flags |= GREETED;
            let flags = save.shop_flags();
            if flags & 1 != 0 {
                self.voice_line = self.guide * GUIDE_LINES + LINE_GREET_NEW;
                save.set_shop_flags(flags & !1);
            } else {
                self.voice_line = self.guide * GUIDE_LINES + LINE_GREET;
            }
            self.play_line(&mut frame.sounds);
        }
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::VoiceFadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = Some(self.result);
            return frame;
        }
        if self.seq == 0 {
            self.input(pad, save, &mut frame.sounds);
        }
        frame
    }

    /// `Shop_Update`: the page's line once the greeting has ended, and the six movies.
    fn update(&mut self, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        if self.seq == 0
            && self.flags & GREETED != 0
            && (self.voice_line == self.guide * GUIDE_LINES + LINE_GREET
                || self.voice_line == self.guide * GUIDE_LINES + LINE_GREET_NEW)
            && voice_stat == VOICE_PLAYEND
        {
            self.play_voice(sounds);
        }
        for f in &mut self.flash {
            f.advance();
        }
    }

    /// `Shop_UpdateSeq`: the guides' swap (1..), the farewell (51..), "complete" (101..), the message window's
    /// place in the draw order while a list slides (151..).
    fn update_seq(&mut self, pad: Pad, voice_stat: i32, sounds: &mut Vec<TitleSound>) {
        let mut seq = self.seq;
        loop {
            match seq {
                1 => {
                    self.flash[FL_GUIDE].goto_label("fl_out", true);
                    self.seq += 1;
                }
                2 => {
                    if self.flash[FL_GUIDE].trig() & 1 != 0 {
                        self.seq += 1;
                    }
                }
                3 => {
                    self.say(LINE_SWAP, sounds);
                    self.seq += 1;
                }
                4 => {
                    if voice_stat == VOICE_PLAYEND {
                        sounds.push(TitleSound::Effect(2, 0x23));
                        self.swap_guide();
                        self.seq += 1;
                    }
                }
                5 => {
                    self.flash[FL_GUIDE].goto_label("fl_in", true);
                    self.seq += 1;
                }
                6 => {
                    if self.flash[FL_GUIDE].trig() & 1 != 0 {
                        self.say(LINE_GREET, sounds);
                        self.seq += 1;
                    }
                }
                7 => self.seq = 0,
                51 => {
                    self.say(LINE_BYE, sounds);
                    self.seq += 1;
                }
                52 => {
                    if voice_stat == VOICE_PLAYEND {
                        self.seq += 1;
                    } else if pad.pressed & PAD_CROSS != 0 {
                        self.seq += 1;
                        sounds.push(TitleSound::Effect(1, 1));
                    } else if pad.pressed & PAD_TRIANGLE != 0 {
                        self.seq += 1;
                        sounds.push(TitleSound::Effect(1, 2));
                    }
                }
                53 => {
                    self.fade = ColorFade::start_out(true, FADE_FRAMES);
                    self.flash[FL_MAIN].goto_label("fl_out", true);
                    self.seq = 0;
                }
                101 => {
                    self.seq += 1;
                    seq = 102;
                    continue;
                }
                102 => {
                    if self.flash[FL_BUY].trig() & 1 != 0 {
                        self.flash[FL_COMPLETE].goto_label("fl_font_complete_in", true);
                        self.seq += 1;
                    }
                }
                103 => {
                    if self.flash[FL_COMPLETE].trig() & 2 != 0 {
                        self.say(LINE_COMPLETE, sounds);
                        self.seq += 1;
                    }
                }
                104 => {
                    if pad.pressed & PAD_CROSS != 0 {
                        self.flash[FL_COMPLETE].goto_label("fl_font_complete_out", true);
                        self.seq += 1;
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                }
                105 => {
                    if self.flash[FL_COMPLETE].trig() & 2 != 0 {
                        self.seq += 1;
                    }
                }
                106 => {
                    for f in [FL_BUY, FL_GUIDE, FL_MAIN] {
                        self.flash[f].goto_label("fl_list_out", true);
                    }
                    self.set_plate(1, LV_BUY, false, "fl_off_start");
                    self.set_plate(1, LV_BUY, true, "fl_off_start");
                    self.level = LV_MENU;
                    self.seq = 0;
                }
                151 => {
                    self.seq += 1;
                    seq = 152;
                    continue;
                }
                152 => {
                    if self.flash[FL_GUIDE].trig() & 2 != 0 {
                        self.seq += 1;
                    }
                }
                153 => {
                    self.seq = 0;
                    self.flags ^= MSG_BEHIND;
                }
                _ => {}
            }
            break;
        }
    }

    /// `SHOP_COST_UV`: the cost icon by slot count and class, shown.
    fn cost_uv(&mut self, flash: usize, clip: crate::flash::ClipRef, item: i32) {
        let e = self.item_info(item);
        let u0 = i32::from(e.slots) * 0x20 - 0x20;
        let v0 = e.class() * 0x2A;
        let f = &mut self.flash[flash];
        f.clip_set_uv(
            clip,
            Uv {
                u0,
                v0,
                u1: u0 + 0x20,
                v1: v0 + 0x2A,
                tex: 0,
            },
        );
        f.clip_set_flags(clip, PROP_VISIBLE, true);
    }

    /// `SHOP_KIND_UV`.
    fn kind_uv(&self, item: i32) -> Uv {
        let u0 = i32::from(self.item_info(item).kind) * 0x40;
        Uv {
            u0,
            v0: 0,
            u1: u0 + 0x40,
            v1: 0x40,
            tex: 0,
        }
    }

    /// The cost icon of an item, or hidden when it has no cost (or, `shown` false, is not owned).
    fn cost(&mut self, flash: usize, name: &str, item: i32, shown: bool) {
        let r = self.flash[flash].find(Some(name), "mc_icon_item_cost");
        if self.item_info(item).slots != 0 && shown {
            self.cost_uv(flash, r, item);
        } else {
            self.flash[flash].clip_set_flags(r, PROP_VISIBLE, false);
        }
    }

    fn set_tex(&mut self, flash: usize, parent: &str, name: &str, tex: i32) {
        let r = self.flash[flash].find(Some(parent), name);
        self.flash[flash].clip_set_tex(r, tex);
    }

    fn set_visible(&mut self, flash: usize, parent: Option<&str>, name: &str, on: bool) {
        let r = self.flash[flash].find(parent, name);
        self.flash[flash].clip_set_flags(r, PROP_VISIBLE, on);
    }

    #[allow(clippy::too_many_arguments)]
    fn attach(
        &mut self,
        flash: usize,
        name: &str,
        line: i32,
        k: usize,
        font: &Font,
        texts: &mut Vec<TextDraw>,
    ) {
        // TextBox_Init(itemText, 6), clipped to the collection's (boxes 0..7) or the buy list's (8..12) rows.
        let clip = if k < 8 { 0x17F } else { 0x116 };
        let bx = if k < 13 {
            TextBox::preset(6).with_clip(0, 0x200, 0x86, clip)
        } else {
            TextBox::preset(6)
        };
        let r = self.flash[flash].find(Some(name), "mc_dammy_text");
        let text = self.item_text.clone();
        texts.extend(bx.attach_line(
            &mut self.flash[flash],
            r,
            0x400 + k as u32,
            &text,
            line,
            font,
        ));
    }

    /// `Shop_Draw`.
    fn draw(
        &mut self,
        save: &SaveData,
        chr: &ChrTable,
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
        {
            let f = &mut self.flash[FL_MAIN];
            for i in 0..2 {
                let name = format!("menu_plate_{}", i + 1);
                let uv = Uv {
                    u0: 0,
                    v0: i * 0x20,
                    u1: 0x200,
                    v1: i * 0x20 + 0x20,
                    tex: 0,
                };
                for part in ["mc_menu_text_off", "mc_menu_text_on"] {
                    let r = f.find(Some(&name), part);
                    f.clip_set_uv(r, uv);
                }
            }
            anim::num_draw(
                f,
                |n| format!("mc_pay_num_{n}"),
                7,
                save.money(),
                (0x20, 0x20),
                0,
            );
            anim::num_draw(
                f,
                |n| format!("mc_item_num_{n}"),
                3,
                self.percent,
                (0x20, 0x20),
                0,
            );
            let cloud = f.find(None, "mc_bg_cloud");
            f.clip_set_scissor(cloud, Some(WINDOW_CLIP));
            let uv = Uv {
                u0: 0,
                v0: 0,
                u1: 0x200,
                v1: 0x80,
                tex: 0,
            };
            anim::scroll(f, cloud, uv, &mut self.cloud, -0.142_222_22);
            let uron = f.find(None, "mc_ch_uron");
            f.clip_set_scissor(uron, Some(WINDOW_CLIP));
        }
        {
            let b = self.guide != 0;
            let f = &mut self.flash[FL_GUIDE];
            let eye = f.find(
                Some("mc_guide_ranchi"),
                if b {
                    "mc_guide_b_rnh_eye"
                } else {
                    "mc_guide_rnh_eye"
                },
            );
            anim::blink(f, eye, &mut self.blink, 0, random);
            let mouth = f.find(
                Some("mc_guide_ranchi"),
                if b {
                    "mc_guide_b_rnh_mouth"
                } else {
                    "mc_guide_a_rnh_mouth"
                },
            );
            anim::talk(
                f,
                mouth,
                &mut self.talk,
                0,
                &mut self.lip,
                voice_stat,
                random,
            );
        }
        let mut texts = Vec::new();
        for k in 0..2 {
            if self.flags & (LIST_BUY << k) != 0 {
                self.draw_list(k, save, font, &mut texts);
            }
        }
        {
            let f = &mut self.flash[FL_CONFIRM];
            for i in 0..3 {
                let name = format!("mc_reconfir_plate_{}", i + 1);
                let uv = Uv {
                    u0: 0,
                    v0: i * 0x20,
                    u1: 0x100,
                    v1: i * 0x20 + 0x20,
                    tex: 0,
                };
                for part in ["mc_reconfir_text_on", "mc_reconfir_text_off"] {
                    let r = f.find(Some(&name), part);
                    f.clip_set_uv(r, uv);
                }
            }
        }
        self.icon.draw(quads);
        let behind = self.flags & MSG_BEHIND != 0;
        if behind {
            self.msg.draw(self.voice_line, font, quads);
        }
        for f in [FL_MAIN, FL_ALL, FL_BUY, FL_GUIDE] {
            self.draw_flash(f, &texts, font, quads);
        }
        if !behind {
            self.msg.draw(self.voice_line, font, quads);
        }
        font.flush(quads);
        for f in [FL_CONFIRM, FL_COMPLETE] {
            self.draw_flash(f, &texts, font, quads);
        }
        self.help
            .draw(self.help_item.max(0) as usize, chr, font, quads);
    }

    /// `Flash_Draw` of a movie, its texts queued (`TextBox_SetNoFlush`).
    fn draw_flash(
        &mut self,
        f: usize,
        texts: &[TextDraw],
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        let tex = &self.tex[f];
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash[f].draw_with_overs(&has_texture);
        quads.extend(screen_quads(&flash_quads, tex));
        draw_texts(texts, &overs, font);
    }

    /// `Shop_Draw` of one list: its tabs, the plate of the item just bought (buy list), its rows and the extra
    /// plate, the price under the buy list's cursor, the arrows and the scroll bar.
    fn draw_list(&mut self, k: usize, save: &SaveData, font: &Font, texts: &mut Vec<TextDraw>) {
        let fl = if k != 0 { FL_ALL } else { FL_BUY };
        for i in 0..TABS as i32 {
            let name = format!("mc_tab_btn_{i}");
            let uv = Uv {
                u0: i * 0x40,
                v0: 0,
                u1: i * 0x40 + 0x40,
                v1: 0x40,
                tex: 0,
            };
            let f = &mut self.flash[fl];
            for part in ["mc_icon_tab_off", "mc_icon_tab_on"] {
                let r = f.find(Some(&name), part);
                f.clip_set_uv(r, uv);
            }
            self.set_visible(fl, Some(&name), "mc_font_new_tab", false);
        }
        let v0 = self.list[k].tab as i32 * 0x20;
        let r = self.flash[fl].find(None, "mc_tab_text");
        self.flash[fl].clip_set_uv(
            r,
            Uv {
                u0: 0,
                v0,
                u1: 0x100,
                v1: v0 + 0x20,
                tex: 0,
            },
        );
        if k == 0 {
            // Plate 6: the item just bought.
            let p6 = "mc_list_plate_6";
            self.set_tex(fl, p6, "mc_list_plate_off", 2);
            let uv = self.kind_uv(self.item);
            let r = self.flash[fl].find(Some(p6), "mc_icon_item_potara");
            self.flash[fl].clip_set_uv(r, uv);
            self.cost(fl, p6, self.item, true);
            self.set_visible(fl, Some(p6), "mc_font_new", false);
            self.attach(fl, p6, self.item, 13, font, texts);
        }
        let list_clip = LIST_CLIPS[self.cursor[0].clamp(0, 1) as usize];
        let rows = self.list[k].rows;
        for i in 0..rows {
            let name = format!("mc_list_plate_{}", i + 1);
            let plate = self.flash[fl].find(None, &name);
            self.flash[fl].clip_set_flags(plate, 0x400, k != 0);
            if i == 0 || i == rows - 1 {
                self.flash[fl].clip_set_scissor(plate, Some(list_clip));
            }
            let l = &self.list[k];
            let item = l.row(i);
            if k == 0 {
                if l.top[l.tab] + i >= l.count[l.tab] {
                    self.flash[fl].clip_set_color(plate, 1.0);
                    self.set_tex(fl, &name, "mc_list_plate_off", 7);
                    self.set_visible(fl, Some(&name), "mc_icon_item_cost", false);
                    self.attach(fl, &name, -1, 8 + i as usize, font, texts);
                    self.set_visible(fl, Some(&name), "mc_icon_item_potara", false);
                } else {
                    let shade = if self.can_buy(save, item) { 1.0 } else { 0.4 };
                    self.flash[fl].clip_set_color(plate, shade);
                    self.set_tex(fl, &name, "mc_list_plate_off", 2);
                    self.cost(fl, &name, item, true);
                    self.attach(fl, &name, item, 8 + i as usize, font, texts);
                    let uv = self.kind_uv(item);
                    let r = self.flash[fl].find(Some(&name), "mc_icon_item_potara");
                    self.flash[fl].clip_set_uv(r, uv);
                    self.flash[fl].clip_set_flags(r, PROP_VISIBLE, true);
                }
            } else {
                let owned = save.item(item.max(0) as usize) & OWNED != 0;
                self.set_tex(fl, &name, "mc_list_plate_off", if owned { 2 } else { 7 });
                let line = if owned { item } else { ITEM_MAX as i32 };
                self.attach(fl, &name, line, i as usize, font, texts);
                self.cost(fl, &name, item, owned);
                let uv = self.kind_uv(item);
                let r = self.flash[fl].find(Some(&name), "mc_icon_item_potara");
                self.flash[fl].clip_set_uv(r, uv);
            }
            let new = k != 0 && save.item(item.max(0) as usize) & NEW != 0;
            self.set_visible(fl, Some(&name), "mc_font_new", new);
        }
        // The extra plate, seen while the list scrolls by one row.
        let name = format!("mc_list_plate_{}", rows + 1);
        let plate = self.flash[fl].find(None, &name);
        self.flash[fl].clip_set_flags(plate, 0x400, k != 0);
        self.flash[fl].clip_set_scissor(plate, Some(list_clip));
        let l = &self.list[k];
        let item = l.extra_item();
        if k == 0 {
            if l.extra >= l.count[l.tab] {
                self.set_tex(fl, &name, "mc_list_plate_off", 7);
            } else {
                let shade = if self.can_buy(save, item) { 1.0 } else { 0.4 };
                self.flash[fl].clip_set_color(plate, shade);
                self.set_tex(fl, &name, "mc_list_plate_off", 2);
            }
            self.attach(fl, &name, item, 8 + rows as usize, font, texts);
            self.cost(fl, &name, item, true);
        } else {
            let owned = save.item(item.max(0) as usize) & OWNED != 0;
            self.set_tex(fl, &name, "mc_list_plate_off", if owned { 2 } else { 7 });
            let line = if owned { item } else { ITEM_MAX as i32 };
            self.attach(fl, &name, line, rows as usize, font, texts);
            self.cost(fl, &name, item, owned);
        }
        let uv = self.kind_uv(item);
        let r = self.flash[fl].find(Some(&name), "mc_icon_item_potara");
        self.flash[fl].clip_set_uv(r, uv);
        let new = k != 0 && save.item(item.max(0) as usize) & NEW != 0;
        self.set_visible(fl, Some(&name), "mc_font_new", new);
        // The price of the item under the buy list's cursor.
        let price = self.item_info(self.list[0].cur_item()).price;
        anim::num_draw(
            &mut self.flash[fl],
            |n| format!("mc_shop_num_{n}"),
            7,
            price,
            (0x20, 0x20),
            0,
        );
        let l = &self.list[k];
        let (top, bottom, count, tab_rows) =
            (l.top[l.tab], l.bottom[l.tab], l.count[l.tab], l.rows);
        let rows_f = l.rows_f;
        for i in 0..2 {
            let uv = Uv {
                u0: i * 0x20,
                v0: 0x20,
                u1: i * 0x20 + 0x20,
                v1: 0x40,
                tex: 0,
            };
            let arrow = if i != 0 {
                "mc_menu_yajirusi_down"
            } else {
                "mc_menu_yajirusi_up"
            };
            let f = &mut self.flash[fl];
            let r = f.find(Some("mc_menu_yajirusi"), arrow);
            f.clip_set_uv(r, uv);
            let shown = if i == 0 {
                top != 0
            } else {
                !(count < tab_rows || bottom == count - 1)
            };
            f.clip_set_flags(r, PROP_VISIBLE, shown);
        }
        let count = count.max(tab_rows);
        let pos = (rows_f * 32.0 * top as f32 / count as f32) as i32;
        let scale = tab_rows as f32 / count as f32 * rows_f;
        let f = &mut self.flash[fl];
        let r = f.find(None, "mc_scroll_bar_point");
        f.clip_set_scale(r, 1.0, scale);
        f.clip_set_offset(r, 0, pos);
    }

    /// `Shop_Input`.
    fn input(&mut self, pad: Pad, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        if self.flash[FL_MAIN].flags() & FLASH_PAD == 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.set_plate(0, LV_MENU, false, "fl_on_start");
            self.flags |= STARTED;
        }
        match self.level {
            LV_MENU => self.input_menu(pad, save, sounds),
            LV_BUY | LV_ALL => self.input_list(self.level, pad, save, sounds),
            LV_HELP => {
                if pad.pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                    self.help.close();
                    match self.cursor[LV_HELP] as usize {
                        LV_ALL => {
                            self.set_plate(2, LV_ALL, false, "fl_on_start");
                            self.set_plate(2, LV_ALL, true, "fl_on_start");
                        }
                        LV_CONFIRM => self.set_plate(4, LV_CONFIRM, false, "fl_on_start"),
                        _ => {}
                    }
                    self.level = self.cursor[LV_HELP] as usize;
                    sounds.push(TitleSound::Effect(1, 2));
                }
            }
            LV_CONFIRM => self.input_confirm(pad, save, sounds),
            LV_BOUGHT => {
                if pad.pressed & PAD_CROSS != 0 {
                    if self.percent == 100 {
                        self.seq = 101;
                    } else {
                        self.say(LINE_THANKS, sounds);
                    }
                    self.flash[FL_BUY].goto_label("fl_get_out", true);
                    self.level = LV_BUY;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            _ => {}
        }
    }

    fn input_menu(&mut self, pad: Pad, save: &SaveData, sounds: &mut Vec<TitleSound>) {
        if pad.repeat & (PAD_UP | PAD_DOWN) != 0 {
            self.idle = 0;
            self.set_plate(0, LV_MENU, false, "fl_off_start");
            if pad.repeat & PAD_UP != 0 {
                self.cursor[0] -= 1;
                if self.cursor[0] < 0 {
                    self.cursor[0] = 1;
                }
            } else {
                self.cursor[0] += 1;
                if self.cursor[0] >= 2 {
                    self.cursor[0] = 0;
                }
            }
            self.set_plate(0, LV_MENU, false, "fl_on_start");
            self.play_voice(sounds);
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.pressed & PAD_CROSS != 0 {
            self.idle = 0;
            self.set_plate(0, LV_MENU, false, "fl_ok");
            match self.cursor[0] {
                0 => {
                    if self.percent == 100 {
                        self.say(LINE_ALL_OWNED, sounds);
                        sounds.push(TitleSound::Effect(1, 7));
                    } else if self.is_sold_out(save) {
                        self.say(LINE_SOLD_OUT, sounds);
                        sounds.push(TitleSound::Effect(1, 7));
                    } else {
                        for f in [FL_MAIN, FL_BUY, FL_GUIDE] {
                            self.flash[f].goto_label("fl_list_in", true);
                        }
                        self.seq = 151;
                        self.set_plate(1, LV_BUY, false, "fl_on_start");
                        self.set_plate(1, LV_BUY, true, "fl_on_start");
                        self.flags |= LIST_BUY;
                        self.flags &= !(LIST_BUY << 1);
                        self.say(LINE_LIST, sounds);
                        self.level = LV_BUY;
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                }
                1 => {
                    for f in [FL_MAIN, FL_ALL] {
                        self.flash[f].goto_label("fl_list_in", true);
                    }
                    self.msg.close();
                    self.flash[FL_GUIDE].goto_label("fl_out", true);
                    self.flags |= LIST_BUY << 1;
                    self.flags &= !LIST_BUY;
                    self.set_plate(2, LV_ALL, false, "fl_on_start");
                    self.set_plate(2, LV_ALL, true, "fl_on_start");
                    self.level = LV_ALL;
                    sounds.push(TitleSound::Effect(1, 1));
                }
                _ => {}
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.idle = 0;
            self.seq = 51;
            self.result = 0;
            sounds.push(TitleSound::Effect(1, 2));
        } else {
            self.idle += 1;
            if self.idle == 3600 {
                self.seq = 1;
                self.idle = 0;
            }
        }
    }

    /// The buy list (`level` 1, flash 1, pages of 4) or the collection (2, flash 2, pages of 7).
    fn input_list(
        &mut self,
        level: usize,
        pad: Pad,
        save: &mut SaveData,
        sounds: &mut Vec<TitleSound>,
    ) {
        let k = level - 1;
        let fl = level;
        let page = self.list[k].rows;
        let page_up = pad.repeat & PAD_L1 != 0;
        let page_down = pad.repeat & PAD_R1 != 0;
        if page_up {
            let t = self.list[k].tab;
            if self.list[k].cur[t] > 0 {
                self.set_plate(fl, level, false, "fl_off_start");
                // The buy list's second case reads the collection list's top (an original bug).
                let other_top = self.list[1].top[self.list[1].tab];
                let l = &mut self.list[k];
                l.cur[t] -= page;
                if l.cur[t] < l.top[t] {
                    if l.cur[t] < page {
                        l.extra = l.top[t];
                        l.cur[t] = 0;
                        l.top[t] = 0;
                        l.bottom[t] = page - 1;
                    } else {
                        l.extra = if k == 0 { other_top } else { l.top[t] };
                        l.top[t] -= page;
                        l.bottom[t] -= page;
                    }
                }
                self.set_plate(fl, level, false, "fl_on_start");
                sounds.push(TitleSound::Effect(1, 0));
            }
        } else if page_down {
            let t = self.list[k].tab;
            if self.list[k].cur[t] < self.list[k].count[t] - 1 {
                self.set_plate(fl, level, false, "fl_off_start");
                let l = &mut self.list[k];
                l.cur[t] += page;
                // The buy list clamps the cursor first; the collection does not.
                if k == 0 && l.cur[t] > l.count[t] - 1 {
                    l.cur[t] = l.count[t] - 1;
                }
                if l.bottom[t] < l.cur[t] {
                    if l.cur[t] > l.count[t] - (page + 1) {
                        l.cur[t] = l.count[t] - 1;
                        l.extra = l.bottom[t];
                        l.top[t] = l.count[t] - page;
                        l.bottom[t] = l.count[t] - 1;
                    } else {
                        l.extra = l.bottom[t];
                        l.top[t] += page;
                        l.bottom[t] += page;
                    }
                }
                self.set_plate(fl, level, false, "fl_on_start");
                sounds.push(TitleSound::Effect(1, 0));
            }
        } else if pad.repeat & PAD_UP != 0 {
            let t = self.list[k].tab;
            if self.list[k].cur[t] > 0 {
                self.set_plate(fl, level, false, "fl_off_start");
                let l = &mut self.list[k];
                l.cur[t] -= 1;
                if l.cur[t] < l.top[t] {
                    self.flash[fl].goto_label("fl_list_down", true);
                    l.extra = l.bottom[t];
                    l.top[t] -= 1;
                    l.bottom[t] -= 1;
                }
                self.set_plate(fl, level, false, "fl_on_start");
                sounds.push(TitleSound::Effect(1, 0));
            }
        } else if pad.repeat & PAD_DOWN != 0 {
            let t = self.list[k].tab;
            if self.list[k].cur[t] < self.list[k].count[t] - 1 {
                self.set_plate(fl, level, false, "fl_off_start");
                let l = &mut self.list[k];
                l.cur[t] += 1;
                if l.cur[t] > l.bottom[t] {
                    self.flash[fl].goto_label("fl_list_up", true);
                    l.extra = l.top[t];
                    l.top[t] += 1;
                    l.bottom[t] += 1;
                }
                self.set_plate(fl, level, false, "fl_on_start");
                sounds.push(TitleSound::Effect(1, 0));
            }
        } else if pad.repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
            self.set_plate(fl, level, false, "fl_off_start");
            self.set_plate(fl, level, true, "fl_off_start");
            let l = &mut self.list[k];
            l.tab = if pad.repeat & PAD_LEFT != 0 {
                (l.tab + TABS - 1) % TABS
            } else {
                (l.tab + 1) % TABS
            };
            self.set_plate(fl, level, false, "fl_on_start");
            self.set_plate(fl, level, true, "fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.pressed & PAD_CROSS != 0 {
            self.set_plate(fl, level, false, "fl_ok");
            let item = self.list[k].cur_item();
            if k == 0 {
                if self.can_buy(save, item) {
                    self.cursor[LV_CONFIRM] = 1;
                    self.set_plate(4, LV_CONFIRM, false, "fl_on_start");
                    self.flash[FL_CONFIRM].goto_label("fl_window_in", true);
                    self.say(LINE_CONFIRM, sounds);
                    self.level = LV_CONFIRM;
                    sounds.push(TitleSound::Effect(1, 1));
                } else {
                    self.say(LINE_OWNED, sounds);
                    sounds.push(TitleSound::Effect(1, 7));
                }
            } else if save.item(item.max(0) as usize) & OWNED != 0 {
                self.help.open();
                self.cursor[LV_HELP] = self.level as i32;
                self.help_item = item;
                let i = item.max(0) as usize;
                save.set_item(i, save.item(i) & !NEW);
                self.level = LV_HELP;
                sounds.push(TitleSound::Effect(1, 1));
            } else {
                sounds.push(TitleSound::Effect(1, 7));
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            if k == 0 {
                for f in [FL_BUY, FL_GUIDE, FL_MAIN] {
                    self.flash[f].goto_label("fl_list_out", true);
                }
                self.seq = 151;
            } else {
                self.flash[FL_ALL].goto_label("fl_list_out", true);
                self.msg.open();
                self.flash[FL_GUIDE].goto_label("fl_in", true);
                self.flash[FL_MAIN].goto_label("fl_list_out", true);
            }
            self.set_plate(fl, level, false, "fl_off_start");
            self.set_plate(fl, level, true, "fl_off_start");
            self.say(LINE_BACK, sounds);
            self.level = LV_MENU;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    /// The confirmation window: buy, cancel, details.
    fn input_confirm(&mut self, pad: Pad, save: &mut SaveData, sounds: &mut Vec<TitleSound>) {
        if pad.repeat & (PAD_UP | PAD_DOWN) != 0 {
            self.set_plate(4, LV_CONFIRM, false, "fl_off_start");
            let c = &mut self.cursor[LV_CONFIRM];
            if pad.repeat & PAD_UP != 0 {
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
            self.set_plate(4, LV_CONFIRM, false, "fl_on_start");
            sounds.push(TitleSound::Effect(1, 0));
        } else if pad.pressed & PAD_CROSS != 0 {
            match self.cursor[LV_CONFIRM] {
                0 => {
                    let item = self.list[0].cur_item();
                    let price = self.item_info(item).price;
                    if (save.money() as u32) < price as u32 {
                        self.say(LINE_NO_MONEY, sounds);
                        sounds.push(TitleSound::Effect(1, 7));
                    } else {
                        self.set_plate(4, LV_CONFIRM, false, "fl_ok");
                        self.flash[FL_BUY].goto_label("fl_get_in", true);
                        self.flash[FL_CONFIRM].goto_label("fl_window_out", true);
                        self.item = item;
                        save.add_item(item.max(0) as usize);
                        save.add_money(-price);
                        self.owned += 1;
                        self.percent = percent(self.owned, self.total);
                        self.dirty = true;
                        self.level = LV_BOUGHT;
                        sounds.push(TitleSound::Effect(1, 1));
                    }
                }
                1 => {
                    self.flash[FL_CONFIRM].goto_label("fl_window_out", true);
                    self.set_plate(4, LV_CONFIRM, false, "fl_ok");
                    self.level = LV_BUY;
                    sounds.push(TitleSound::Effect(1, 2));
                }
                _ => {
                    self.set_plate(4, LV_CONFIRM, false, "fl_ok");
                    self.help.open();
                    self.cursor[LV_HELP] = self.level as i32;
                    self.help_item = self.list[0].cur_item();
                    self.level = LV_HELP;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
        } else if pad.pressed & PAD_TRIANGLE != 0 {
            self.flash[FL_CONFIRM].goto_label("fl_window_out", true);
            self.set_plate(4, LV_CONFIRM, false, "fl_off_start");
            self.level = LV_BUY;
            sounds.push(TitleSound::Effect(1, 2));
        }
    }
}

/// `Shop_CheckStockLevel`: once (`shopFlags` bit 1), when all collectable items but one are owned, the stock
/// level rises (at most 5) and the next greeting says so (bit 0).
fn check_stock_level(items: &[ItemInfo], save: &mut SaveData) {
    if save.shop_flags() & 2 != 0 {
        return;
    }
    let (mut total, mut owned) = (0, 0);
    for (i, it) in items.iter().enumerate().take(ITEM_MAX) {
        if it.flags & ITEM_HIDDEN != 0
            || it.flags & ITEM_LISTED == 0
            || it.flags & ITEM_UNCOUNTED != 0
        {
            continue;
        }
        total += 1;
        owned += i32::from(save.item(i) & 1);
    }
    if owned == total - 1 {
        let mut level = save.stock_level() + 1;
        if level as u32 >= 6 {
            level = 5;
        }
        save.set_stock_level(level);
        save.set_shop_flags(save.shop_flags() | 3);
    }
}

/// The pad words the shop reads (`gamePressed`, `gameRepeat`).
#[derive(Clone, Copy, Debug)]
struct Pad {
    pressed: u32,
    repeat: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the real shop from the ISO in `rom/`, when it is there: the buy list at stock level 0, the first item
    /// bought (paid for, owned and new, the percentage up), then its details from the collection, and leaving with
    /// the guide's farewell.
    #[test]
    fn real_shop_sells_an_item() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let mut read = |id: u32| {
            let location = partition_entry_for_id(id);
            let archive =
                AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition))
                    .unwrap();
            archive.read_entry(&mut image, location.index).unwrap()
        };
        let chr = ChrTable::parse(&read(4), 0xA1);
        let pack = read(crate::evo_top::ARCHIVE_FILE_ID);
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = ShopAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = ShopLayout::new(0, |k| firsts[&k]);
        let mut save = SaveData::new(&chr.items);
        save.add_money(1_000_000);
        let mut random = GameRandom::from_key(&[9]);
        let mut font = Font::new(Vec::new(), 20);
        let mut screen = ShopScreen::new(&data, &chr, &mut save, &mut random).unwrap();
        eprintln!(
            "buy list {:?}, collection {:?}, {} / {} owned",
            screen.list[0].count, screen.list[1].count, screen.owned, screen.total
        );
        assert!(screen.list[0].count[0] > 0 && screen.list[1].count[0] > screen.list[0].count[0]);
        let mut press = |screen: &mut ShopScreen, save: &mut SaveData, bits: u32| {
            for i in 0..60 {
                let b = if i == 0 { bits } else { 0 };
                screen.run_frame(b, b, VOICE_PLAYEND, save, &chr, &mut random, &mut font);
                if screen.result().is_some() {
                    break;
                }
            }
        };
        for _ in 0..4 {
            press(&mut screen, &mut save, 0);
        }
        assert!(screen.flags & GREETED != 0);
        // The first item the shop sells that is not owned yet.
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_BUY);
        while !screen.can_buy(&save, screen.list[0].cur_item()) {
            press(&mut screen, &mut save, PAD_DOWN);
        }
        let item = screen.list[0].cur_item();
        let price = screen.item_info(item).price;
        let (money, owned) = (save.money(), screen.owned);
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_CONFIRM);
        press(&mut screen, &mut save, PAD_UP);
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_BOUGHT);
        assert_eq!(save.money(), money - price);
        assert_eq!(save.item(item as usize) & 3, 3);
        assert_eq!(screen.owned, owned + 1);
        assert!(screen.dirty());
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_BUY);
        // Back, then the collection: its details mark the item seen.
        press(&mut screen, &mut save, PAD_TRIANGLE);
        assert_eq!(screen.level, LV_MENU);
        press(&mut screen, &mut save, PAD_DOWN);
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_ALL);
        let tab = screen.list[1].tab;
        while screen.list[1].cur_item() != item {
            let before = screen.list[1].cur[tab];
            press(&mut screen, &mut save, PAD_DOWN);
            assert_ne!(screen.list[1].cur[tab], before);
        }
        press(&mut screen, &mut save, PAD_CROSS);
        assert_eq!(screen.level, LV_HELP);
        assert_eq!(save.item(item as usize) & 3, 1);
        press(&mut screen, &mut save, PAD_TRIANGLE);
        assert_eq!(screen.level, LV_ALL);
        press(&mut screen, &mut save, PAD_TRIANGLE);
        // Leaving: the farewell, then the fade.
        press(&mut screen, &mut save, PAD_TRIANGLE);
        for _ in 0..5 {
            press(&mut screen, &mut save, 0);
        }
        assert_eq!(screen.result(), Some(0));
    }
}
