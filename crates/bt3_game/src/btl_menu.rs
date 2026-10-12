//! The battle menus (src/battle/btl_menu.c, src/battle/pause_menu.c): a tree of menus and items read from the
//! executable's data (0x2C4E70..0x2C6070), the engine that runs and draws one tree, and the pause-menu wrapper that
//! turns a picked "result" item into the battle result.
//!
//! The versus pause menu (`PauseMenu_Update(pad, 0)` while the fight is paused, with its skill-list page) and
//! result menu (`PauseMenu_Update(pad, 1)` from the end state) are wired up. The controls page (item type 4) and
//! the CPU level menu, which no versus tree has, are not ported.

use std::collections::HashMap;
use std::io;

use bt3_data::elf::Elf;

use crate::btl_text::{self, Painter, SkillList, TextCtx};
use crate::font::{Font, rgba};
use crate::hud::{HudBlend, HudPrim, HudSheet, HudVertex};
use crate::title::TitleSound;

/// `gPauseMenuVersus` / `gPauseMenuResultVersus`.
pub(crate) const PAUSE_VERSUS: u32 = 0x2C_5280;
pub(crate) const RESULT_VERSUS: u32 = 0x2C_5408;
/// `gPauseMenuReplayRec` / `gPauseMenuResultReplayRec`: the trees while the battle's own replay plays.
pub(crate) const PAUSE_REPLAY: u32 = 0x2C_5560;
pub(crate) const RESULT_REPLAY: u32 = 0x2C_56E8;
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
    /// Every item's id, menu by menu.
    #[cfg(test)]
    pub(crate) fn item_ids(&self) -> Vec<i32> {
        self.menus
            .iter()
            .flat_map(|m| m.items.iter().map(|i| i.id))
            .collect()
    }

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

    /// `BtlMenu_IsSkillList`.
    fn is_skills(&self, m: usize) -> bool {
        self.menus[m]
            .items
            .first()
            .is_some_and(|i| i.kind == ITEM_SKILLS)
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
    /// `gameRepeat`: the directions that went down or repeat this frame.
    pub rep_up: bool,
    pub rep_down: bool,
    pub rep_left: bool,
    pub rep_right: bool,
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
    pub(crate) trees: [Tree; 2],
    /// `gBtlMenu->active`: the menu is shown.
    pub(crate) active: bool,
    pad: i32,
    /// `gBtlMenu->top`: the tree being shown.
    top: Option<usize>,
    /// `gBtlMenu->list`: each controller port's skill list.
    lists: [SkillList; 2],
    /// Each side's skill-list script (`BtlMenu_SetScript` of the fighter's character file entry 0x2D).
    pub(crate) scripts: [Option<Vec<u16>>; 2],
    /// What the scripts' line tests read, per side.
    pub(crate) text_ctx: [TextCtx; 2],
    /// `FontIcon_ResetAnim` was called during the update (the caller resets the font's clock).
    pub(crate) reset_anim: bool,
}

impl PauseMenu {
    pub(crate) fn new(pause: Tree, result: Tree) -> Self {
        Self {
            trees: [pause, result],
            // BtlMenu_Init.
            active: true,
            pad: 0,
            top: None,
            lists: [SkillList::default(); 2],
            scripts: [None, None],
            text_ctx: [TextCtx {
                pad_type: 0,
                chara_bits: [0; 3],
                body_changed: false,
            }; 2],
            reset_anim: false,
        }
    }

    /// A controller port's skill list.
    #[cfg(test)]
    pub(crate) fn skill_list(&self, pad: usize) -> SkillList {
        self.lists[pad]
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
        // BtlMenu_SetScript: the script of the pad's side.
        let side = (pad & 1) as usize;
        let mut run = Run {
            tree: &mut self.trees[which],
            work_active: &mut self.active,
            work_pad: &mut self.pad,
            top: 0,
            which,
            font,
            texts,
            sounds: &mut *sounds,
            list: &mut self.lists[side],
            script: self.scripts[side].as_deref(),
            text_ctx: self.text_ctx[side],
            reset_anim: &mut self.reset_anim,
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
        let side = (self.pad & 1) as usize;
        let skills = Skills {
            list: &self.lists[side],
            script: self.scripts[side].as_deref(),
            ctx: self.text_ctx[side],
        };
        draw_menu(&self.trees[which], 0, self.pad, d, &skills, out);
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
    list: &'r mut SkillList,
    script: Option<&'r [u16]>,
    text_ctx: TextCtx,
    reset_anim: &'r mut bool,
}

/// The skill list a page draws.
struct Skills<'s> {
    list: &'s SkillList,
    script: Option<&'s [u16]>,
    ctx: TextCtx,
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

    /// `PauseMenu_SkillListFunc`: left / right turn the page (wrapping), up / down move the entry cursor (held
    /// at the ends) and scroll the seven-line window, on the owner pad's repeat word.
    fn skill_list_func(&mut self, press: MenuInput) {
        let list = &mut *self.list;
        let last = list.pages - 1;
        let mut page = list.page;
        let mut sounds = 0;
        if press.rep_right {
            page += 1;
            sounds += 1;
        }
        if press.rep_left {
            page -= 1;
            sounds += 1;
        }
        if page > last {
            page = 0;
        }
        if page < 0 {
            page = last;
        }
        let mut step = 0;
        if press.rep_up {
            step = -1;
        }
        if press.rep_down {
            step += 1;
        }
        let pi = page.clamp(0, 9) as usize;
        let mut scroll = list.scroll[pi];
        let mut cursor = list.cursor[pi];
        let max = list.count[pi] - 1;
        let lim = scroll + 6;
        let old = cursor;
        if step != 0 {
            if (step < 0 && cursor != 0) || (step > 0 && cursor != max) {
                sounds += 1;
            }
            cursor += step;
        }
        if cursor > max {
            cursor = max;
        }
        if cursor < 0 {
            cursor = 0;
        }
        if cursor < scroll {
            scroll = cursor;
        }
        if cursor > lim {
            scroll = cursor - 6;
        }
        list.scroll[pi] = scroll;
        list.cursor[pi] = cursor;
        list.page = page;
        if cursor != old {
            *self.reset_anim = true;
        }
        for _ in 0..sounds {
            self.se(SE_MOVE);
        }
    }

    /// `BtlMenu_CallItem`.
    fn call_item(&mut self, m: usize, i: usize, leaf: bool, press: MenuInput) {
        let item = &self.tree.menus[m].items[i];
        let (Some(func), true) = (item.func, item.active) else {
            return;
        };
        let selected = item.selected;
        let result = self.call_func(func, m, selected, press);
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

    /// The item callbacks of pause_menu.c (the CPU level menu, in no versus tree, does nothing here).
    fn call_func(&mut self, func: Func, m: usize, selected: bool, press: MenuInput) -> i32 {
        if func == Func::SkillList && self.tree.menus[m].state == STATE_ACTIVE {
            self.skill_list_func(press);
        }
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
        // The controller type (Pad_GetLastStatus: 0 once a pad was read) and BtlText_CountEntries.
        btl_text::count_entries(self.list, self.script, &self.text_ctx);
        self.fix_cursor(m);
        let open = self.tree.menus[m].open;
        if open >= 0 {
            if let Some(sub) = self.tree.menus[m].items[open as usize].sub {
                self.call_item(m, open as usize, false, press);
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
            self.call_item(m, cursor, true, press);
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

/// `BtlText_PutSprite` under a scissor (`SCISSOR_1`, inclusive bounds): sheet entry `tex`, texels `uv` over the
/// pixels `rect` (either may run backwards), cut to the scissor with the texels following.
pub(crate) fn push_cut(
    sheet: &HudSheet,
    out: &mut Vec<HudPrim>,
    tex: usize,
    rect: [i32; 4],
    uv: [i32; 4],
    scissor: [i32; 4],
) {
    let [x0, y0, x1, y1] = rect.map(|v| v as f32);
    let [u0, v0, u1, v1] = uv.map(|v| v as f32);
    let (sx0, sy0) = (scissor[0] as f32, scissor[1] as f32);
    let (sx1, sy1) = ((scissor[2] + 1) as f32, (scissor[3] + 1) as f32);
    let cut = |a0: f32, a1: f32, t0: f32, t1: f32, lo: f32, hi: f32| -> Option<[f32; 4]> {
        let (min, max) = (a0.min(a1), a0.max(a1));
        let (n0, n1) = (min.max(lo), max.min(hi));
        if n0 >= n1 {
            return None;
        }
        let t = |a: f32| t0 + (a - a0) / (a1 - a0) * (t1 - t0);
        Some(if a0 <= a1 {
            [n0, n1, t(n0), t(n1)]
        } else {
            [n1, n0, t(n1), t(n0)]
        })
    };
    let (Some([nx0, nx1, nu0, nu1]), Some([ny0, ny1, nv0, nv1])) =
        (cut(x0, x1, u0, u1, sx0, sx1), cut(y0, y1, v0, v1, sy0, sy1))
    else {
        return;
    };
    out.push(sprite(
        sheet,
        tex,
        [nx0, ny0, nx1, ny1],
        [nu0, nv0, nu1, nv1],
        GREY,
    ));
}

/// `BtlMenu_Draw` of the skill-list page in the box (x, y, w, h): the lower and upper frames, every page's tab
/// with the current one on top of the page bar, the list and the details (`BtlText_DrawList`), the scroll bar.
fn draw_skill_page(
    tree: &Tree,
    m: usize,
    [x, y, w, h]: [i32; 4],
    d: &mut MenuDraw,
    skills: &Skills,
    out: &mut Vec<HudPrim>,
) {
    let menu = &tree.menus[m];
    let (pad_x, pad_y) = (menu.pad_x, menu.pad_y);
    let list = skills.list;
    let x1 = x + w;
    draw_frame(d.sheet, out, x, x1, y + 0x10C, y + h, 1);
    draw_frame(d.sheet, out, x, x1, y, y + 0x111, 1);
    // BtlMenu_DrawPageTab: sheet entry 3 with the page's palette (entry 0x26 + page), 50 pixels per tab.
    let tab = |out: &mut Vec<HudPrim>, page: i32| {
        let tx = x + page * 50 + 20;
        put_mirrored(
            d.sheet,
            out,
            [tx, tx + 50, y + 6, y + 0x19],
            3,
            (page + 0x26) as usize,
            true,
        );
    };
    for i in (0..list.pages).rev() {
        tab(out, i);
    }
    // BtlMenu_DrawPageBar: sheet entry 4 with the page's palette.
    put_mirrored(
        d.sheet,
        out,
        [x + 4, x1 - 3, y + 0x14, y + 0x39],
        4,
        (list.page.max(0) + 0x26) as usize,
        false,
    );
    tab(out, list.page);
    let Some(font) = d.font.as_deref_mut() else {
        return;
    };
    let mut paint = Painter {
        sheet: d.sheet,
        out,
        scissor: [0, 0, 0x1FF, 0x1BF],
    };
    let (lx0, lx1) = (x + pad_x, x - pad_x + w);
    btl_text::draw_list(
        font,
        &mut paint,
        list,
        skills.script,
        &skills.ctx,
        [lx0, lx1, y + 0x3E, y + 0x102],
        0,
    );
    btl_text::draw_list(
        font,
        &mut paint,
        list,
        skills.script,
        &skills.ctx,
        [lx0, lx1, y + 0x11B, y - pad_y + h],
        1,
    );
    btl_text::draw_scroll_bar(&mut paint, list, lx1, y + 0x3E, y + 0x100);
}

/// `BtlMenu_PutMirrored`: copies of sheet entry `tex` (seen through entry `clut`'s palette) mirrored about the box
/// centre: with `halves` a left and a right copy, else four quarters.
#[allow(clippy::too_many_arguments)]
fn put_mirrored(
    sheet: &HudSheet,
    out: &mut Vec<HudPrim>,
    [x0, x1, y0, y1]: [i32; 4],
    tex: usize,
    clut: usize,
    halves: bool,
) {
    let (hw, hh) = ((x1 - x0) / 2, (y1 - y0) / 2);
    let (cx, cy) = ((x0 + hw) as f32, (y0 + hh) as f32);
    let (w, h) = (hw as f32, hh as f32);
    let texture = sheet.view(tex, clut);
    let mut put = |rect: [f32; 4], uv: [f32; 4]| {
        let mut prim = sprite(sheet, tex, rect, uv, GREY);
        prim.texture = texture;
        out.push(prim);
    };
    if halves {
        for sx in [-1.0, 1.0] {
            put(
                [cx + sx * w, cy - h, cx, cy + h],
                [0.0, 0.0, w * 2.0, h * 2.0],
            );
        }
    } else {
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            put([cx + sx * w, cy + sy * h, cx, cy], [0.0, 0.0, w, h]);
        }
    }
}

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
fn draw_menu(
    tree: &Tree,
    m: usize,
    pad: i32,
    d: &mut MenuDraw,
    skills: &Skills,
    out: &mut Vec<HudPrim>,
) {
    let menu = &tree.menus[m];
    let page = tree.is_page(m);
    let is_skills = tree.is_skills(m);
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
            draw_frame(d.sheet, out, x, x + w, y, y + h, usize::from(is_skills));
        }
        STATE_ACTIVE => {
            if !tree.has_open_sub(m) {
                let font = d.font.as_deref();
                let w = menu_width(tree, Some(m), pad, font, d.texts);
                let h = menu_height(tree, Some(m), font);
                let (x, y) = ((SCREEN_W - w) >> 1, (SCREEN_H - h) >> 1);
                if is_skills {
                    draw_skill_page(tree, m, [x, y, w, h], d, skills, out);
                } else {
                    // The controls page (no versus tree has one) draws its box only.
                    draw_frame(d.sheet, out, x, x + w, y, y + h, 0);
                }
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
            // FontIcon_ResetAnim behind every plain menu: its icons stand on their first frame.
            if !tree.has_open_sub(m)
                && !page
                && let Some(font) = d.font.as_deref_mut()
            {
                font.reset_icon_anim();
            }
            for (i, item) in menu.items.iter().enumerate() {
                if let (Some(sub), true) = (item.sub, menu.open == i as i32) {
                    draw_menu(tree, sub, pad, d, skills, out);
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
