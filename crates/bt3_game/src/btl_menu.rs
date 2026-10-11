//! The battle menus (src/battle/btl_menu.c, src/battle/pause_menu.c): a tree of menus and items read from the
//! executable's data (0x2C4E70..0x2C6070), the engine that runs and draws one tree, and the pause-menu wrapper that
//! turns a picked "result" item into the battle result.
//!
//! Only the result menu of the versus mode is wired up (`PauseMenu_Update(pad, 1)` from the end state). The
//! skill-list and controls pages (item types 3 / 4), the menu sounds' bank and the font's button icons are not
//! ported; no shipped result menu uses the two pages.

use std::collections::HashMap;
use std::io;

use bt3_data::elf::Elf;

use crate::font::{Font, rgba};
use crate::hud::{HudBlend, HudPrim, HudSheet, HudVertex};
use crate::title::TitleSound;

/// `gPauseMenuVersus` / `gPauseMenuResultVersus`.
pub(crate) const PAUSE_VERSUS: u32 = 0x2C_5280;
pub(crate) const RESULT_VERSUS: u32 = 0x2C_5408;
/// The yes / no box and the CPU level menu, whose cursors the item callbacks set.
const CONFIRM_MENU: u32 = 0x2C_4F00;
/// Item callbacks by address.
const FUNC_CONFIRM: u32 = 0x21_2BA0;
const FUNC_SKILL_LIST: u32 = 0x21_2C08;
const FUNC_CPU_LEVEL: u32 = 0x21_2DC0;

/// `BtlMenuItem.type`.
const ITEM_VALUE: i32 = 0;
const ITEM_RESULT: i32 = 1;
const ITEM_BACK: i32 = 2;
const ITEM_SKILLS: i32 = 3;
const ITEM_CONTROLS: i32 = 4;

/// `BtlMenu.mode`.
const MODE_TOGGLE: i32 = 0;
const MODE_RADIO: i32 = 1;
const MODE_SINGLE: i32 = 2;

/// `BtlMenu.state`.
const STATE_OPEN: i32 = 0;
const STATE_OPENING: i32 = 1;
const STATE_CLOSE: i32 = 2;
const STATE_CLOSING: i32 = 3;
pub(crate) const STATE_ACTIVE: i32 = 4;

/// Menu sounds (`Snd_PlaySe(1, id)`).
const SE_MOVE: u32 = 0;
const SE_DECIDE: u32 = 1;
const SE_OPEN: u32 = 4;
const SE_CLOSE: u32 = 5;

/// The screen (`0x200 x 0x1C0`).
const SCREEN_W: i32 = 0x200;
const SCREEN_H: i32 = 0x1C0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Func {
    Confirm,
    SkillList,
    CpuLevel,
    Other,
}

/// `BtlMenuItem`.
#[derive(Clone, Debug)]
struct Item {
    /// Text per controller port; negative: row `!n` of sheet entry 0.
    text: [i16; 2],
    id: i32,
    sub: Option<usize>,
    func: Option<Func>,
    kind: i32,
    value: i32,
    disabled: bool,
    active: bool,
    called: bool,
    selected: bool,
    picked: bool,
}

/// `BtlMenu`.
#[derive(Clone, Debug)]
struct Menu {
    items: Vec<Item>,
    scale: f32,
    pad_x: i32,
    pad_y: i32,
    mode: i32,
    open: i32,
    cursor: i32,
    state: i32,
    w: f32,
    h: f32,
    dw: f32,
    dh: f32,
    frame: i32,
}

/// One menu tree; menu 0 is the top. A menu used from several items (the yes / no box) is one shared entry.
#[derive(Clone, Debug)]
pub(crate) struct Tree {
    menus: Vec<Menu>,
    confirm: Option<usize>,
}

impl Tree {
    /// Reads the tree at `addr` of the executable's data.
    pub(crate) fn read(elf: &Elf, addr: u32) -> io::Result<Self> {
        let mut tree = Tree {
            menus: Vec::new(),
            confirm: None,
        };
        let mut seen = HashMap::new();
        tree.read_menu(elf, addr, &mut seen, 0)?;
        tree.confirm = seen.get(&CONFIRM_MENU).copied();
        Ok(tree)
    }

    fn read_menu(
        &mut self,
        elf: &Elf,
        addr: u32,
        seen: &mut HashMap<u32, usize>,
        depth: usize,
    ) -> io::Result<usize> {
        if let Some(&i) = seen.get(&addr) {
            return Ok(i);
        }
        if depth > 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "menu tree too deep",
            ));
        }
        let m = elf.read(addr, 0x34)?.to_vec();
        let word = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let index = self.menus.len();
        seen.insert(addr, index);
        self.menus.push(Menu {
            items: Vec::new(),
            scale: f32::from_bits(word(&m, 8)),
            pad_x: i32::from(i16::from_le_bytes([m[0xC], m[0xD]])),
            pad_y: i32::from(i16::from_le_bytes([m[0xE], m[0xF]])),
            mode: word(&m, 0x10) as i32,
            open: word(&m, 0x14) as i32,
            cursor: word(&m, 0x18) as i32,
            state: word(&m, 0x1C) as i32,
            w: f32::from_bits(word(&m, 0x20)),
            h: f32::from_bits(word(&m, 0x24)),
            dw: 0.0,
            dh: 0.0,
            frame: 0,
        });
        let (items, count) = (word(&m, 0), word(&m, 4));
        let mut list = Vec::new();
        for i in 0..count.min(32) {
            let it = elf.read(items + i * 0x30, 0x30)?.to_vec();
            let sub = match word(&it, 8) {
                0 => None,
                a => Some(self.read_menu(elf, a, seen, depth + 1)?),
            };
            list.push(Item {
                text: [
                    i16::from_le_bytes([it[0], it[1]]),
                    i16::from_le_bytes([it[2], it[3]]),
                ],
                id: word(&it, 4) as i32,
                sub,
                func: match word(&it, 0xC) {
                    0 => None,
                    FUNC_CONFIRM => Some(Func::Confirm),
                    FUNC_SKILL_LIST => Some(Func::SkillList),
                    FUNC_CPU_LEVEL => Some(Func::CpuLevel),
                    _ => Some(Func::Other),
                },
                kind: word(&it, 0x14) as i32,
                value: word(&it, 0x18) as i32,
                disabled: word(&it, 0x1C) != 0,
                active: word(&it, 0x20) != 0,
                called: word(&it, 0x24) != 0,
                selected: word(&it, 0x28) != 0,
                picked: word(&it, 0x2C) != 0,
            });
        }
        self.menus[index].items = list;
        Ok(index)
    }

    fn is_page(&self, m: usize) -> bool {
        self.menus[m]
            .items
            .first()
            .is_some_and(|i| i.kind == ITEM_SKILLS || i.kind == ITEM_CONTROLS)
    }

    /// `BtlMenu_FindItem`: depth first; (menu, item).
    fn find_item(&self, m: usize, id: i32) -> Option<(usize, usize)> {
        for (i, item) in self.menus[m].items.iter().enumerate() {
            if item.id == id {
                return Some((m, i));
            }
            if let Some(sub) = item.sub {
                if let Some(found) = self.find_item(sub, id) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// `BtlMenu_SetValue`.
    fn set_value(&mut self, m: usize, id: i32, value: i32) {
        if let Some((m, i)) = self.find_item(m, id) {
            self.menus[m].items[i].value = value;
        }
    }

    /// `BtlMenu_Reset`.
    fn reset(&mut self, m: usize) {
        let menu = &mut self.menus[m];
        menu.open = -1;
        menu.cursor = 0;
        menu.state = STATE_OPEN;
        let subs: Vec<usize> = menu
            .items
            .iter_mut()
            .filter_map(|item| {
                item.active = false;
                item.value = 0;
                item.picked = false;
                item.sub
            })
            .collect();
        for sub in subs {
            self.reset(sub);
        }
    }

    /// `BtlMenu_FindResult`: a picked item of type "result"; found below the top menu, the top menu's item under
    /// the cursor (the entry the box was opened from).
    fn find_result(&self, mut top: Option<usize>, m: usize) -> Option<(usize, usize)> {
        for (i, item) in self.menus[m].items.iter().enumerate() {
            if item.kind == ITEM_RESULT && item.picked {
                return Some(match top {
                    None => (m, i),
                    Some(t) => (t, self.menus[t].cursor.max(0) as usize),
                });
            }
            if let Some(sub) = item.sub {
                if top.is_none() {
                    top = Some(m);
                }
                if let Some(found) = self.find_result(top, sub) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn has_open_sub(&self, m: usize) -> bool {
        let menu = &self.menus[m];
        menu.items
            .iter()
            .enumerate()
            .any(|(i, item)| item.sub.is_some() && menu.open == i as i32)
    }
}

/// The owner pad's buttons that went down this frame (`gPad[pad].gamePressed`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MenuInput {
    pub up: bool,
    pub down: bool,
    pub cross: bool,
    pub triangle: bool,
    pub start: bool,
}

/// The menu texts of the HUD file (`BtlMenu_GetText(n)`: entry n of its offset table, 16-bit characters up to a
/// zero).
#[derive(Clone, Debug, Default)]
pub(crate) struct MenuTexts(Vec<Vec<u16>>);

impl MenuTexts {
    pub(crate) fn read(file: &[u8], count: usize) -> Self {
        let word = |o: usize| {
            file.get(o..o + 4)
                .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
        };
        Self(
            (0..count)
                .map(|n| {
                    let start = word(n * 4) & !3;
                    file.get(start..)
                        .unwrap_or_default()
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|&c| c != 0)
                        .take(0x200)
                        .collect()
                })
                .collect(),
        )
    }

    /// `BtlMenu_GetItemText(n)`.
    fn item(&self, n: i32) -> &[u16] {
        usize::try_from(n + 8)
            .ok()
            .and_then(|i| self.0.get(i))
            .map_or(&[], Vec::as_slice)
    }
}

/// What a menu draws with: the font, the sheet (HUD file entry 7) and the texts.
pub(crate) struct MenuDraw<'a> {
    pub font: Option<&'a mut Font>,
    pub sheet: &'a HudSheet,
    pub texts: &'a MenuTexts,
}

/// `gBtlMenu` with the pause menu's two trees (`PauseMenu_GetMenu`).
#[derive(Clone, Debug)]
pub(crate) struct PauseMenu {
    trees: [Tree; 2],
    /// `gBtlMenu->active`: the menu is shown.
    pub(crate) active: bool,
    pad: i32,
    /// `gBtlMenu->top`: the tree being shown.
    top: Option<usize>,
}

impl PauseMenu {
    pub(crate) fn new(pause: Tree, result: Tree) -> Self {
        Self {
            trees: [pause, result],
            // BtlMenu_Init.
            active: true,
            pad: 0,
            top: None,
        }
    }

    /// `PauseMenu_Reset`: both trees closed.
    pub(crate) fn reset(&mut self) {
        for tree in &mut self.trees {
            tree.reset(0);
        }
        self.active = false;
    }

    /// `PauseMenu_Update(pad, which)`: one frame for controller port `pad`. Returns `Ok(true)` while the menu
    /// stays open, and a picked result's reason bits (`BattleResult_Set(BTL_RESULT_ABORT, reason)`).
    pub(crate) fn update(
        &mut self,
        pad: i32,
        which: usize,
        input: MenuInput,
        font: Option<&Font>,
        texts: &MenuTexts,
        sounds: &mut Vec<TitleSound>,
    ) -> (bool, Option<i32>) {
        let mut run = Run {
            tree: &mut self.trees[which],
            work_active: &mut self.active,
            work_pad: &mut self.pad,
            top: 0,
            which,
            font,
            texts,
            sounds: &mut *sounds,
        };
        if run.update(None, 0, pad, input) {
            self.active = false;
            return (false, None);
        }
        self.top = Some(which);
        let Some((m, i)) = self.trees[which].find_result(None, 0) else {
            return (true, None);
        };
        sounds.push(TitleSound::Effect(1, SE_DECIDE));
        let reason = match self.trees[which].menus[m].items[i].id {
            0xC | 0x13 => Some(0x8000),
            0xD => Some(0x10),
            0xE => Some(0x80),
            0xF => Some(8),
            0x10 => Some(0x100),
            0x12 => Some(0x200),
            0x11 => Some(0x400),
            0x14 => Some(if pad == 0 { 0x20 } else { 0x40 }),
            0x17 => Some(0x1000),
            0x18 | 0x19 => Some(0x1_0000),
            0x1A | 0x15 => Some(0x20),
            _ => None,
        };
        (true, reason)
    }

    /// `PauseMenu_Draw` / `BtlMenu_DrawAll`: the deepest open menu of the shown tree.
    pub(crate) fn draw(&self, d: &mut MenuDraw, out: &mut Vec<HudPrim>) {
        let Some(which) = self.top else {
            return;
        };
        if !self.active {
            return;
        }
        draw_menu(&self.trees[which], 0, self.pad, d, out);
    }
}

/// `BtlMenu_Update` and the item callbacks over one tree.
struct Run<'r> {
    tree: &'r mut Tree,
    work_active: &'r mut bool,
    work_pad: &'r mut i32,
    top: usize,
    which: usize,
    font: Option<&'r Font>,
    texts: &'r MenuTexts,
    sounds: &'r mut Vec<TitleSound>,
}

impl Run<'_> {
    fn se(&mut self, id: u32) {
        self.sounds.push(TitleSound::Effect(1, id));
    }

    /// `BtlMenu_GetWidth`: the widest item text (at least 128) times the scale, plus the margins; 0 for none.
    fn width(&self, m: Option<usize>) -> i32 {
        menu_width(self.tree, m, *self.work_pad, self.font, self.texts)
    }

    fn height(&self, m: Option<usize>) -> i32 {
        menu_height(self.tree, m, self.font)
    }

    /// `BtlMenu_FixCursor`.
    fn fix_cursor(&mut self, m: usize) {
        if self.tree.is_page(m) {
            return;
        }
        let menu = &mut self.tree.menus[m];
        let count = menu.items.len() as i32;
        if count == 0 {
            return;
        }
        let start = menu.cursor;
        loop {
            let mut c = menu.cursor;
            if c < 0 {
                c = count - 1;
                menu.cursor = c;
            }
            if c >= count {
                c = 0;
                menu.cursor = c;
            }
            if !menu.items[c as usize].disabled {
                break;
            }
            let n = c + 1;
            menu.cursor = n;
            if n == start {
                break;
            }
        }
    }

    /// `BtlMenu_CallItem`.
    fn call_item(&mut self, m: usize, i: usize, leaf: bool) {
        let item = &self.tree.menus[m].items[i];
        let (Some(func), true) = (item.func, item.active) else {
            return;
        };
        let selected = item.selected;
        let result = self.call_func(func, m, selected);
        let item = &mut self.tree.menus[m].items[i];
        if result != 0 {
            item.active = false;
        }
        item.selected = false;
        item.called = true;
        if leaf {
            *self.work_active = result != 0;
        }
    }

    /// The item callbacks of pause_menu.c. The skill list and the CPU level menu are not reachable from the result
    /// menus and do nothing here.
    fn call_func(&mut self, func: Func, _m: usize, selected: bool) -> i32 {
        if func == Func::Confirm && selected {
            // PauseMenu_ConfirmFunc: on the shown tree's top.
            self.tree.set_value(self.top, 1, 0);
            self.tree.set_value(self.top, 0, 0);
            if let Some(c) = self.tree.confirm {
                self.tree.menus[c].cursor = 2;
            }
        }
        0
    }

    /// `BtlMenu_Update`: returns true once the menu finished closing.
    fn update(&mut self, parent: Option<usize>, m: usize, pad: i32, press: MenuInput) -> bool {
        *self.work_active = true;
        *self.work_pad = pad;
        self.fix_cursor(m);
        let open = self.tree.menus[m].open;
        if open >= 0 {
            if let Some(sub) = self.tree.menus[m].items[open as usize].sub {
                self.call_item(m, open as usize, false);
                if self.update(Some(m), sub, pad, press) {
                    let menu = &mut self.tree.menus[m];
                    menu.items[open as usize].active = false;
                    menu.open = -1;
                }
                return false;
            }
            self.tree.menus[m].open = -1;
        }
        let cursor = self.tree.menus[m].cursor.max(0) as usize;
        let busy = {
            let item = &self.tree.menus[m].items[cursor];
            item.active && item.func.is_some()
        };
        if !busy {
            let sound = !self.tree.is_page(m);
            match self.tree.menus[m].state {
                STATE_OPEN | STATE_OPENING => {
                    if self.tree.menus[m].state == STATE_OPEN {
                        self.se(SE_OPEN);
                        let (pw, ph) = (self.width(parent), self.height(parent));
                        let (w, h) = (self.width(Some(m)), self.height(Some(m)));
                        let menu = &mut self.tree.menus[m];
                        menu.state = STATE_OPENING;
                        menu.frame = 0;
                        menu.w = pw as f32;
                        menu.h = ph as f32;
                        menu.dw = (w - pw) as f32 / 6.0;
                        menu.dh = (h - ph) as f32 / 6.0;
                    }
                    let (w, h) = (self.width(Some(m)), self.height(Some(m)));
                    let menu = &mut self.tree.menus[m];
                    menu.w += menu.dw;
                    menu.h += menu.dh;
                    menu.frame += 1;
                    if menu.frame >= 6 {
                        menu.w = w as f32;
                        menu.h = h as f32;
                        menu.state = STATE_ACTIVE;
                    }
                }
                STATE_CLOSE | STATE_CLOSING => {
                    if self.tree.menus[m].state == STATE_CLOSE {
                        self.se(SE_CLOSE);
                        let (pw, ph) = (self.width(parent), self.height(parent));
                        let (w, h) = (self.width(Some(m)), self.height(Some(m)));
                        let menu = &mut self.tree.menus[m];
                        menu.state = STATE_CLOSING;
                        menu.frame = 0;
                        menu.w = w as f32;
                        menu.h = h as f32;
                        menu.dw = (pw - w) as f32 / 6.0;
                        menu.dh = (ph - h) as f32 / 6.0;
                    }
                    let (pw, ph) = (self.width(parent), self.height(parent));
                    let menu = &mut self.tree.menus[m];
                    menu.w += menu.dw;
                    menu.h += menu.dh;
                    menu.frame += 1;
                    if menu.frame >= 6 {
                        menu.w = pw as f32;
                        menu.h = ph as f32;
                        menu.state = STATE_OPEN;
                        return true;
                    }
                }
                _ => self.active_input(m, pad, press, sound),
            }
        }
        // end: the leaf callback of the item under the cursor.
        let cursor = self.tree.menus[m].cursor.max(0) as usize;
        if self.tree.menus[m].items[cursor].sub.is_none() {
            self.call_item(m, cursor, true);
        }
        false
    }

    /// The active state of `BtlMenu_Update`.
    fn active_input(&mut self, m: usize, pad: i32, press: MenuInput, sound: bool) {
        if pad >= 0 {
            let is_top = m == self.top;
            if (!is_top && press.triangle)
                || (is_top && (press.start || press.triangle) && self.which == 0)
            {
                self.tree.menus[m].state = STATE_CLOSE;
                return;
            }
        }
        let mut step = 0;
        let old = self.tree.menus[m].cursor;
        if pad >= 0 {
            if press.up {
                if sound {
                    self.se(SE_MOVE);
                }
                step = -1;
            }
            if press.down {
                if sound {
                    self.se(SE_MOVE);
                }
                step = 1;
            }
        }
        let count = self.tree.menus[m].items.len() as i32;
        if step != 0 {
            let menu = &mut self.tree.menus[m];
            menu.cursor += step;
            loop {
                if menu.cursor < 0 {
                    menu.cursor = count - 1;
                }
                if menu.cursor >= count {
                    menu.cursor = 0;
                }
                if menu.cursor == old || !menu.items[menu.cursor as usize].disabled {
                    break;
                }
                menu.cursor += step;
            }
            return;
        }
        if pad < 0 || !press.cross {
            return;
        }
        let cursor = self.tree.menus[m].cursor.max(0) as usize;
        self.tree.menus[m].items[cursor].picked = true;
        let (sub, func, kind) = {
            let item = &self.tree.menus[m].items[cursor];
            (item.sub, item.func, item.kind)
        };
        if let Some(sub) = sub {
            self.tree.menus[m].open = cursor as i32;
            self.update(Some(m), sub, pad, press);
            if func.is_some() {
                let item = &mut self.tree.menus[m].items[cursor];
                item.active = true;
                item.selected = true;
            }
        } else if func.is_some() {
            let item = &mut self.tree.menus[m].items[cursor];
            item.active = true;
            item.selected = true;
        } else {
            let menu = &mut self.tree.menus[m];
            match kind {
                ITEM_BACK => menu.state = STATE_CLOSE,
                ITEM_VALUE => match menu.mode {
                    MODE_RADIO => {
                        for item in &mut menu.items {
                            item.value = 0;
                        }
                        menu.items[cursor].value = 1;
                    }
                    MODE_SINGLE => {
                        menu.items[cursor].value ^= 1;
                        if menu.items[cursor].value != 0 {
                            for (i, item) in menu.items.iter_mut().enumerate() {
                                if i != cursor {
                                    item.value = 0;
                                }
                            }
                        }
                    }
                    MODE_TOGGLE => menu.items[cursor].value ^= 1,
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn menu_width(
    tree: &Tree,
    m: Option<usize>,
    pad: i32,
    font: Option<&Font>,
    texts: &MenuTexts,
) -> i32 {
    let Some(m) = m else {
        return 0;
    };
    let menu = &tree.menus[m];
    let glyph = font.map_or(0, Font::line_height);
    let w = match menu.items.first().map(|i| i.kind) {
        Some(ITEM_SKILLS) => glyph * 20,
        Some(ITEM_CONTROLS) => glyph * 22,
        _ => menu
            .items
            .iter()
            .map(|item| {
                let text = texts.item(i32::from(item.text[(pad & 1) as usize]));
                font.map_or(0, |f| f.width(text))
            })
            .max()
            .unwrap_or(0)
            .max(0x80),
    };
    (w as f32 * menu.scale) as i32 + menu.pad_x * 2
}

fn menu_height(tree: &Tree, m: Option<usize>, font: Option<&Font>) -> i32 {
    let Some(m) = m else {
        return 0;
    };
    let menu = &tree.menus[m];
    let glyph = font.map_or(0, Font::line_height);
    let h = match menu.items.first().map(|i| i.kind) {
        Some(ITEM_SKILLS) => glyph * 18,
        Some(ITEM_CONTROLS) => glyph * 15,
        _ => menu.items.len() as i32 * 28,
    };
    (h as f32 * menu.scale) as i32 + menu.pad_y * 2
}

/// A GS sprite of the menu sheet: corners in pixels, UV in texels (`BtlMenu_PutSprite`), PRIM 0x156 with the
/// default ALPHA 0x44.
fn sprite(sheet: &HudSheet, tex: usize, rect: [f32; 4], uv: [f32; 4], color: [u8; 4]) -> HudPrim {
    let [w, h] = sheet
        .sizes
        .get(tex)
        .map_or([1.0, 1.0], |s| s.map(|v| v.max(1) as f32));
    let [x0, y0, x1, y1] = rect;
    let [u0, v0, u1, v1] = uv;
    let v = |x: f32, y: f32, u: f32, t: f32| HudVertex {
        pos: [x, y],
        uv: [(u + 0.5) / w, (t + 0.5) / h],
        mask_uv: [0.0; 2],
    };
    let quad = [
        v(x0, y0, u0, v0),
        v(x1, y0, u1, v0),
        v(x1, y1, u1, v1),
        v(x0, y1, u0, v1),
    ];
    HudPrim {
        texture: Some(sheet.base + tex),
        vertices: vec![quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]],
        rgba: color,
        blend: HudBlend::Alpha,
    }
}

const GREY: [u8; 4] = [0x80, 0x80, 0x80, 0x80];

/// `BtlMenu_DrawFrame` / `BtlMenu_PutMirrored` with four quarters: the top-left corner of sheet entry kind + 1,
/// mirrored about the box centre (the texture clamps past its edge).
fn draw_frame(
    sheet: &HudSheet,
    out: &mut Vec<HudPrim>,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    kind: usize,
) {
    let (hw, hh) = ((x1 - x0) / 2, (y1 - y0) / 2);
    let (cx, cy) = ((x0 + hw) as f32, (y0 + hh) as f32);
    let (w, h) = (hw as f32, hh as f32);
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        out.push(sprite(
            sheet,
            kind + 1,
            [cx + sx * w, cy + sy * h, cx, cy],
            [0.0, 0.0, w, h],
            GREY,
        ));
    }
}

/// `BtlMenu_Draw`: the growing box while a menu opens or closes; when active, the deepest open menu.
fn draw_menu(tree: &Tree, m: usize, pad: i32, d: &mut MenuDraw, out: &mut Vec<HudPrim>) {
    let menu = &tree.menus[m];
    let page = tree.is_page(m);
    match menu.state {
        STATE_OPEN..=STATE_CLOSING => {
            let (mut w, mut h) = (menu.w as i32, menu.h as i32);
            if w < 4 && h >= 4 {
                w = 4;
            }
            if h < 4 && w >= 4 {
                h = 4;
            }
            let (x, y) = ((SCREEN_W - w) >> 1, (SCREEN_H - h) >> 1);
            draw_frame(d.sheet, out, x, x + w, y, y + h, usize::from(page));
        }
        STATE_ACTIVE => {
            if !tree.has_open_sub(m) {
                let font = d.font.as_deref();
                let w = menu_width(tree, Some(m), pad, font, d.texts);
                let h = menu_height(tree, Some(m), font);
                let (x, y) = ((SCREEN_W - w) >> 1, (SCREEN_H - h) >> 1);
                // The two pages are not ported: their box only.
                draw_frame(d.sheet, out, x, x + w, y, y + h, usize::from(page));
                if !page {
                    let ty = y + menu.pad_y;
                    if let Some(font) = d.font.as_deref_mut() {
                        font.push_style();
                        let line = font.line_height();
                        let s = &mut font.style;
                        s.scale = menu.scale;
                        s.spacing_x = 0;
                        s.spacing_y = 0x1C - line;
                        s.align = 1;
                        s.shadow_mode = 2;
                        s.shadow_color = rgba(0, 0, 0, 0x40);
                        s.shadow_dx = 1;
                        s.shadow_dy = 2;
                    }
                    // Font_SetHome(0x100, ty + 0x2A); Font_NewLine after each text.
                    let mut cy = ty + 0x2A;
                    for (i, item) in menu.items.iter().enumerate() {
                        let text = i32::from(item.text[(pad & 1) as usize]);
                        if text >= 0 {
                            if let Some(font) = d.font.as_deref_mut() {
                                font.style.color = if menu.cursor == i as i32 {
                                    rgba(0xF4, 0xFF, 0x3F, 0x80)
                                } else {
                                    rgba(0x89, 0xC3, 0xE5, 0x80)
                                };
                                font.print_at(0x100, cy, d.texts.item(text));
                                let s = font.style;
                                cy += ((font.line_height() + s.spacing_y) as f32
                                    * (s.scale_y * s.scale))
                                    as i32;
                            }
                        } else {
                            // BtlMenu_DrawPart: row !text (32 texels) of sheet entry 0, centred.
                            let part = !text as f32;
                            let (cx, cy0) = ((x + w / 2) as f32, (ty + 0xE) as f32);
                            out.push(sprite(
                                d.sheet,
                                0,
                                [cx - 128.0, cy0 - 16.0, cx + 128.0, cy0 + 16.0],
                                [0.0, 32.0 * part, 256.0, 32.0 * (part + 1.0)],
                                GREY,
                            ));
                        }
                    }
                    if let Some(font) = d.font.as_deref_mut() {
                        font.pop_style();
                    }
                }
            }
            for (i, item) in menu.items.iter().enumerate() {
                if let (Some(sub), true) = (item.sub, menu.open == i as i32) {
                    draw_menu(tree, sub, pad, d, out);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        text: i16,
        id: i32,
        sub: Option<usize>,
        func: Option<Func>,
        kind: i32,
        disabled: bool,
    ) -> Item {
        Item {
            text: [text; 2],
            id,
            sub,
            func,
            kind,
            value: 0,
            disabled,
            active: false,
            called: false,
            selected: false,
            picked: false,
        }
    }

    fn menu(items: Vec<Item>) -> Menu {
        Menu {
            items,
            scale: 1.0,
            pad_x: 32,
            pad_y: 24,
            mode: MODE_SINGLE,
            open: -1,
            cursor: 0,
            state: STATE_OPEN,
            w: 0.0,
            h: 0.0,
            dw: 0.0,
            dh: 0.0,
            frame: 0,
        }
    }

    /// The versus result menu's shape: a title, "fight again", one entry with the shared yes / no box.
    fn result_menu() -> PauseMenu {
        let top = menu(vec![
            item(-1, 1, None, None, ITEM_BACK, true),
            item(3, 0x13, None, None, ITEM_RESULT, false),
            item(5, 0xD, Some(1), Some(Func::Confirm), ITEM_VALUE, false),
        ]);
        let yes_no = menu(vec![
            item(-3, 1, None, None, ITEM_BACK, true),
            item(0, 0, None, None, ITEM_RESULT, false),
            item(1, 1, None, None, ITEM_BACK, false),
        ]);
        let tree = Tree {
            menus: vec![top, yes_no],
            confirm: Some(1),
        };
        PauseMenu::new(tree.clone(), tree)
    }

    fn frame(m: &mut PauseMenu, input: MenuInput) -> Option<i32> {
        let texts = MenuTexts::default();
        let mut sounds = Vec::new();
        m.update(0, 1, input, None, &texts, &mut sounds).1
    }

    #[test]
    fn opening_takes_six_frames_and_skips_the_title() {
        let mut m = result_menu();
        for _ in 0..6 {
            assert_eq!(frame(&mut m, MenuInput::default()), None);
        }
        assert_eq!(m.trees[1].menus[0].state, STATE_ACTIVE);
        // The disabled title is stepped over.
        assert_eq!(m.trees[1].menus[0].cursor, 1);
    }

    #[test]
    fn fight_again_restarts_and_a_confirmed_entry_waits_for_yes() {
        let mut m = result_menu();
        for _ in 0..6 {
            frame(&mut m, MenuInput::default());
        }
        let cross = MenuInput {
            cross: true,
            ..MenuInput::default()
        };
        assert_eq!(frame(&mut m, cross), Some(0x8000));

        let mut m = result_menu();
        for _ in 0..6 {
            frame(&mut m, MenuInput::default());
        }
        frame(
            &mut m,
            MenuInput {
                down: true,
                ..MenuInput::default()
            },
        );
        assert_eq!(frame(&mut m, cross), None);
        // The box opens over six frames with the cursor put on "No"; up moves to "Yes".
        for _ in 0..6 {
            assert_eq!(frame(&mut m, MenuInput::default()), None);
        }
        assert_eq!(m.trees[1].menus[1].cursor, 2);
        frame(
            &mut m,
            MenuInput {
                up: true,
                ..MenuInput::default()
            },
        );
        assert_eq!(frame(&mut m, cross), Some(0x10));
    }
}
