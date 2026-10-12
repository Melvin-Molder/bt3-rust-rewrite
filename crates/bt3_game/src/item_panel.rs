//! The item panel of a character select (`ItemPanel_*`, src/menu/item_panel.c), one per side: the set name, the
//! character and form names, the slots taken and available, and the eight rows of the set's items with their
//! kind and cost icons; and the item details page (`ItemHelp_*`, src/menu/item_help.c) a row opens: the item's
//! picture, name and description.

use std::error::Error;

use bt3_data::chara_data::ChrTable;

use crate::draw::ScreenQuad;
use crate::flash::{ClipRef, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::menu_res::ImageTexture;
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, TitleSound};
use crate::windows::{TextBox, TextDraw, draw_texts};

/// `ITEMPANEL_ROWS`, `ITEMPANEL_BACK`.
const ROWS: usize = 8;
const BACK: i32 = 7;

/// `ITEMPANEL_*`.
const STARTED: u32 = 4;
const SHOWN: u32 = 8;
const CLOSING: u32 = 0x10;

/// The panel's section of the character select's pack.
pub const PANEL_SECTION: usize = 53;
/// The details page's section.
pub const HELP_SECTION: usize = 51;

fn t(n: usize, e: usize) -> ImageTexture {
    Some(((PANEL_SECTION, n), e))
}

/// `ItemPanel_Init`'s textures (`p->tex[n]`) for one side.
pub fn panel_textures(side: usize) -> [ImageTexture; 21] {
    let s = side != 0;
    let mut tex = [None; 21];
    tex[12] = t(16, 0);
    tex[19] = t(1, if s { 1 } else { 0 });
    tex[20] = t(1, if s { 3 } else { 2 });
    tex[8] = t(2, 1);
    tex[7] = t(3, 0);
    tex[11] = t(3, 1);
    tex[13] = t(6, 1);
    tex[14] = if s { t(6, 3) } else { t(6, 0) };
    tex[15] = t(7, 0);
    tex[16] = t(8, 1);
    tex[17] = t(8, 2);
    tex[18] = t(8, 3);
    tex[0] = t(11, 0);
    tex[1] = t(11, 0);
    tex[3] = t(9, 0);
    tex[5] = t(12, 0);
    tex[4] = t(12, 1);
    tex[6] = t(10, 3);
    tex[10] = t(10, 4);
    tex
}

/// `ItemHelp_Init`'s textures (`gItemHelp->tex[n]`).
pub fn help_textures() -> [ImageTexture; 7] {
    help_textures_in(HELP_SECTION)
}

/// `ItemHelp_Init(MPACK_AT(res, section))`'s textures, for the screens that keep the page in another section.
pub fn help_textures_in(section: usize) -> [ImageTexture; 7] {
    let h = |n: usize, e: usize| Some(((section, n), e));
    let mut tex = [None; 7];
    tex[1] = h(1, 0);
    tex[2] = h(1, 4);
    tex[3] = h(1, 8);
    tex[0] = h(1, 9);
    tex[6] = h(3, 0);
    tex[5] = h(3, 1);
    tex
}

/// What the panel and the page read from the character select's pack: their movies and texts.
#[derive(Clone, Debug, Default)]
pub struct ItemAssets {
    /// Sub-sections 4 / 5 of section 53 (the panel of each side) and 2 of section 51 (the page).
    pub panel_fods: [Vec<u8>; 2],
    pub help_fod: Vec<u8>,
    /// Sub-sections 13, 14, 15 of section 53: character names, form names, item names.
    pub panel_text: [Vec<u8>; 3],
    /// Sub-sections 6 and 5 of section 51: item names and descriptions.
    pub help_name: Vec<u8>,
    pub help_desc: Vec<u8>,
}

impl ItemAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let sub = |s: usize, n: usize| -> Result<Vec<u8>, Box<dyn Error>> {
            Ok(bt3_data::pack_section(bt3_data::pack_section(res, s)?, n)?.to_vec())
        };
        Ok(Self {
            panel_fods: [sub(PANEL_SECTION, 4)?, sub(PANEL_SECTION, 5)?],
            help_fod: sub(HELP_SECTION, 2)?,
            panel_text: [
                sub(PANEL_SECTION, 13)?,
                sub(PANEL_SECTION, 14)?,
                sub(PANEL_SECTION, 15)?,
            ],
            help_name: sub(HELP_SECTION, 6)?,
            help_desc: sub(HELP_SECTION, 5)?,
        })
    }
}

/// `ItemPanel` of one side.
pub struct ItemPanel {
    side: usize,
    flash: Flash,
    tex: Vec<Option<usize>>,
    text: [Vec<u8>; 3],
    flags: u32,
    chara: i32,
    /// The set-name strip's row: the item set + 1.
    set: i32,
    /// The slots available, the slots taken and the rows usable (`ItemSet_Fit`).
    limit: i32,
    used: i32,
    count: i32,
    cursor: i32,
    items: [u16; 8],
}

impl ItemPanel {
    /// `ItemPanel_Init`: the side's movie, playing.
    pub fn new(
        side: usize,
        assets: &ItemAssets,
        tex: Vec<Option<usize>>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&assets.panel_fods[side])?;
        flash.play(1);
        Ok(Self {
            side,
            flash,
            tex,
            text: assets.panel_text.clone(),
            flags: 0,
            chara: 0,
            set: 0,
            limit: 0,
            used: 0,
            count: 0,
            cursor: 0,
            items: [0; 8],
        })
    }

    /// `ItemPanel_ClipGoto`: the set plate (kind 0) or the list plate under the cursor (kind 1) to a label.
    fn clip_goto(&mut self, kind: u32, label: &str) {
        let name = if kind == 0 {
            "mc_select_plate".to_owned()
        } else {
            format!("mc_list_plate_{}", self.cursor + 1)
        };
        let r = self.flash.find(None, &name);
        self.flash.clip_goto_label(r, label);
    }

    /// `ItemPanel_SetChara`: item set `set` of a character slot, or the set of a saved custom character; its
    /// slots (the character's base slots and the slot's level) and how much of the set fits.
    pub fn set_chara(
        &mut self,
        chara: i32,
        items: [u16; 8],
        level: i32,
        set: i32,
        from_rec: bool,
        table: &ChrTable,
    ) {
        self.items = items;
        self.chara = chara;
        self.set = set + 1;
        self.limit = table.item_slots(chara, level);
        let (used, count) = table.item_set_fit(&mut self.items, self.limit);
        self.used = used;
        self.count = count;
        if from_rec {
            self.used = 10;
            self.limit = 10;
        }
        self.cursor = 0;
    }

    /// `ItemPanel_Show`.
    pub fn show(&mut self, sounds: &mut Vec<TitleSound>) {
        self.flash.goto_label("fl_evo_in", true);
        self.flags |= SHOWN;
        self.flags &= !CLOSING;
        if self.flags & STARTED != 0 {
            self.clip_goto(0, "fl_on_start");
            self.clip_goto(1, "fl_on_start");
        }
        sounds.push(TitleSound::Effect(2, 4));
    }

    /// `ItemPanel_Hide`: the closing animation; `update` hides it when it ends.
    pub fn hide(&mut self, sounds: &mut Vec<TitleSound>) {
        self.flash.goto_label("fl_evo_cansel", true);
        self.flags |= CLOSING;
        sounds.push(TitleSound::Effect(2, 4));
    }

    /// `ItemPanel_Update`.
    pub fn update(&mut self) {
        if self.flags & SHOWN == 0 {
            return;
        }
        self.flash.advance();
        if self.flags & STARTED == 0 {
            self.clip_goto(0, "fl_on_start");
            self.clip_goto(1, "fl_on_start");
            self.flags |= STARTED;
        }
        if self.flags & CLOSING != 0 && self.flash.trig() & 2 != 0 {
            self.flags &= !(SHOWN | CLOSING);
        }
    }

    /// `ItemPanel_Input`: up / down move the cursor round the usable rows and "back"; confirm gives the item id
    /// under it (0 on an empty row); cancel, or the way back (left on side 0, right on side 1), gives -1.
    pub fn input(&mut self, pressed: u32, repeat: u32, sounds: &mut Vec<TitleSound>) -> i32 {
        let se = |sounds: &mut Vec<TitleSound>, id: u32| sounds.push(TitleSound::Effect(1, id));
        let back = if self.side == 0 { PAD_LEFT } else { PAD_RIGHT };
        if repeat & PAD_UP != 0 {
            self.clip_goto(1, "fl_off_start");
            if self.cursor == BACK {
                self.cursor = self.count - 1;
            } else {
                self.cursor -= 1;
                if self.cursor < 0 {
                    self.cursor = BACK;
                }
            }
            self.clip_goto(1, "fl_on_start");
            se(sounds, 0);
        } else if repeat & PAD_DOWN != 0 {
            self.clip_goto(1, "fl_off_start");
            if self.cursor == BACK {
                self.cursor = 0;
            } else {
                self.cursor += 1;
                if self.cursor >= self.count {
                    self.cursor = BACK;
                }
            }
            self.clip_goto(1, "fl_on_start");
            se(sounds, 0);
        } else if pressed & PAD_CROSS != 0 {
            self.clip_goto(1, "fl_ok");
            let id = self.row(self.cursor);
            if id != 0 {
                se(sounds, 1);
                return i32::from(id);
            }
            se(sounds, 7);
        } else if pressed & (PAD_TRIANGLE | back) != 0 {
            self.clip_goto(1, "fl_off_start");
            self.clip_goto(0, "fl_off_start");
            return -1;
        }
        0
    }

    fn row(&self, i: i32) -> u16 {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.items.get(i))
            .copied()
            .unwrap_or(0)
    }

    fn set_uv(&mut self, parent: Option<&str>, name: &str, uv: Uv) -> ClipRef {
        let r = self.flash.find(parent, name);
        self.flash.clip_set_uv(r, uv);
        r
    }

    /// `ItemPanel_Draw` (while shown): the arrow and the set name by side and set, the names, the two slot
    /// numbers, and per row its plates, cursor, cost and kind icons and the item's name.
    pub fn draw(&mut self, table: &ChrTable, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        if self.flags & SHOWN == 0 {
            return;
        }
        let uv = |u0: i32, v0: i32, u1: i32, v1: i32| Uv {
            u0,
            v0,
            u1,
            v1,
            tex: 0,
        };
        let x = self.side as i32 * 0x20;
        let arrow = if self.side != 0 {
            "mc_yajirusi_right"
        } else {
            "mc_yajirusi_left"
        };
        self.set_uv(Some("mc_select_plate"), arrow, uv(x, 0, x + 0x20, 0x20));
        let y = self.set * 0x20;
        self.set_uv(
            Some("mc_select_plate"),
            "mc_select_text_off",
            uv(0, y, 0x100, y + 0x20),
        );
        self.set_uv(
            Some("mc_select_plate"),
            "mc_select_text_on",
            uv(0, y, 0x100, y + 0x20),
        );
        let mut texts: Vec<TextDraw> = Vec::new();
        let side = self.side as u32;
        let name = self.flash.find(None, "mc_name_text");
        texts.extend(TextBox::preset(1 + side).attach_line(
            &mut self.flash,
            name,
            0,
            &self.text[0],
            self.chara,
            font,
        ));
        let form = self.flash.find(None, "mc_form_text");
        texts.extend(TextBox::preset(3 + side).attach_line(
            &mut self.flash,
            form,
            1,
            &self.text[1],
            self.chara,
            font,
        ));
        let (u, v) = ((self.used % 4) * 0x40, (self.used / 4) * 0x40);
        self.set_uv(
            None,
            "mc_ability_limit_big_num",
            uv(u, v, u + 0x40, v + 0x40),
        );
        let (u, v) = ((self.limit % 4) * 0x20, (self.limit / 4) * 0x20);
        self.set_uv(None, "mc_ability_limit_s_num", uv(u, v, u + 0x20, v + 0x20));
        for i in 0..ROWS as i32 {
            let plate = format!("mc_list_plate_{}", i + 1);
            let id = self.row(i);
            let off = self.flash.find(Some(&plate), "mc_list_plate_off");
            let tex = if i < self.count || i == BACK {
                if id != 0 { 4 } else { 5 }
            } else {
                7
            };
            self.flash.clip_set_tex(off, tex);
            let on = self.flash.find(Some(&plate), "mc_list_plate_on");
            self.flash.clip_set_tex(on, if id != 0 { 0 } else { 1 });
            let cursor = self.flash.find(Some(&plate), "mc_icon_custom");
            if self.cursor == i {
                self.flash.clip_set_uv(cursor, uv(0x40, 0, 0x80, 0x40));
                self.flash.clip_set_flags(cursor, PROP_VISIBLE, true);
            } else {
                self.flash.clip_set_flags(cursor, PROP_VISIBLE, false);
            }
            let item = (id != 0)
                .then(|| table.items.get(usize::from(id) - 1))
                .flatten();
            let cost = self.flash.find(Some(&plate), "mc_icon_item_cost");
            match item.filter(|e| e.slots != 0) {
                Some(e) => {
                    let (u, v) = (i32::from(e.slots) * 0x20 - 0x20, e.class() * 42);
                    self.flash.clip_set_flags(cost, PROP_VISIBLE, true);
                    self.flash.clip_set_uv(cost, uv(u, v, u + 0x20, v + 42));
                }
                None => self.flash.clip_set_flags(cost, PROP_VISIBLE, false),
            }
            let kind = self.flash.find(Some(&plate), "mc_icon_item_potara");
            match item {
                Some(e) => {
                    let u = i32::from(e.kind) * 0x40;
                    self.flash.clip_set_flags(kind, PROP_VISIBLE, true);
                    self.flash.clip_set_uv(kind, uv(u, 0, u + 0x40, 0x40));
                }
                None => self.flash.clip_set_flags(kind, PROP_VISIBLE, false),
            }
            if id != 0 {
                let line = self.flash.find(Some(&plate), "mc_dammy_text");
                texts.extend(TextBox::preset(6).attach_line(
                    &mut self.flash,
                    line,
                    2 + i as u32,
                    &self.text[2],
                    i32::from(id) - 1,
                    font,
                ));
            }
        }
        let tex = &self.tex;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&flash_quads, tex));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
    }
}

/// `ItemHelp`: the details page.
pub struct ItemHelp {
    flash: Flash,
    tex: Vec<Option<usize>>,
    name: Vec<u8>,
    desc: Vec<u8>,
}

impl ItemHelp {
    /// `ItemHelp_Init`.
    pub fn new(assets: &ItemAssets, tex: Vec<Option<usize>>) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&assets.help_fod)?;
        flash.play(1);
        Ok(Self {
            flash,
            tex,
            name: assets.help_name.clone(),
            desc: assets.help_desc.clone(),
        })
    }

    /// `ItemHelp_Open`.
    pub fn open(&mut self) {
        self.flash.goto_label("fl_details_in", true);
    }

    /// `ItemHelp_Close`.
    pub fn close(&mut self) {
        self.flash.goto_label("fl_details_out", true);
    }

    /// `ItemHelp_Draw(item)` (every frame): the movie advances; the item's name, description, kind and cost
    /// icons, its picture's cell of the icon sheet and its picture texture.
    pub fn draw(
        &mut self,
        item: usize,
        table: &ChrTable,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        self.flash.advance();
        let mut texts: Vec<TextDraw> = Vec::new();
        let line = item as i32;
        let name = self.flash.find(None, "mc_dammy_text_1");
        texts.extend(TextBox::preset(6).attach_line(
            &mut self.flash,
            name,
            0,
            &self.name,
            line,
            font,
        ));
        let desc = self.flash.find(None, "mc_dammy_text_2");
        texts.extend(TextBox::preset(0).attach_line(
            &mut self.flash,
            desc,
            1,
            &self.desc,
            line,
            font,
        ));
        let e = table.items.get(item).copied().unwrap_or_default();
        let uv = |u0: i32, v0: i32, u1: i32, v1: i32| Uv {
            u0,
            v0,
            u1,
            v1,
            tex: 0,
        };
        let u = i32::from(e.kind) * 0x40;
        let kind = self.flash.find(None, "mc_icon_item_potara");
        self.flash.clip_set_uv(kind, uv(u, 0, u + 0x40, 0x40));
        let cost = self.flash.find(None, "mc_icon_item_cost");
        if e.slots != 0 {
            let (u, v) = (i32::from(e.slots) * 0x20 - 0x20, e.class() * 0x2A);
            self.flash.clip_set_uv(cost, uv(u, v, u + 0x20, v + 0x2A));
            self.flash.clip_set_flags(cost, PROP_VISIBLE, true);
        } else {
            self.flash.clip_set_flags(cost, PROP_VISIBLE, false);
        }
        let p = i32::from(e.picture);
        let (u, v) = ((p & 1) * 0x40, (p >> 1) * 0x40);
        let icon = self.flash.find(None, "mc_item_details_icon");
        self.flash.clip_set_uv(icon, uv(u, v, u + 0x40, v + 0x40));
        let details = self.flash.find(None, "mc_item_details");
        self.flash.clip_set_tex(details, p);
        let tex = &self.tex;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&flash_quads, tex));
        draw_texts(&texts, &overs, font);
        font.flush(quads);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bt3_data::chara_data::ItemInfo;

    fn table() -> ChrTable {
        let mut items = vec![ItemInfo::default(); 10];
        items[0].slots = 2;
        items[1].slots = 3;
        items[2].slots = 1;
        ChrTable {
            costumes: vec![1; 3],
            base_slots: vec![4, 0, 0],
            items,
            ..ChrTable::default()
        }
    }

    #[test]
    fn a_set_fits_the_slots_and_the_rest_is_dropped() {
        let t = table();
        // Base 4 + level 1 = 5 slots: items 1 (2) and 2 (3) fit, item 3 does not and empties the rest.
        let mut ids = [1, 2, 3, 1, 0, 0, 0, 9];
        let (total, used) = t.item_set_fit(&mut ids, t.item_slots(0, 1));
        assert_eq!((total, used), (5, 2));
        assert_eq!(ids, [1, 2, 0, 0, 0, 0, 0, 9]);
        // An empty set: every slot left as rows.
        let mut ids = [0; 8];
        assert_eq!(t.item_set_fit(&mut ids, 5), (0, 5));
    }
}
