//! The pause menu's skill list (src/battle/btl_seq.c, `BtlText_*`): the fighter's UTF-16 script (character file
//! entry 0x2D + language), counted into pages and entries and drawn as the list of the current page, the
//! selected entry's details and notes, and a scroll bar.
//!
//! Every script line starts with two characters, the second a hex digit: a mask tested against the controller
//! type; then optionally `&ddd` (shown dimmed unless character ddd is unlocked; 086 tests the fighter's body
//! change), then a tag: `$n` page n's title, `*abc` an entry (three icon digits), `%0` / `%1` detail lines, `#`
//! a note, `@` the end.

use crate::font::{Font, rgba};
use crate::hud::{HudPrim, HudSheet};

/// One controller port's list (`BtlTextList` / `BtlMenuList`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SkillList {
    pub pages: i32,
    pub count: [i32; 10],
    pub page: i32,
    pub scroll: [i32; 10],
    pub cursor: [i32; 10],
}

/// What the script's tests read: the controller type, the save's character bits and whether the list's fighter
/// changed body.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextCtx {
    pub pad_type: u32,
    pub chara_bits: [u64; 3],
    pub body_changed: bool,
}

const DOLLAR: u16 = b'$' as u16;
const STAR: u16 = b'*' as u16;
const AT: u16 = b'@' as u16;
const PERCENT: u16 = b'%' as u16;
const HASH: u16 = b'#' as u16;
const AMP: u16 = b'&' as u16;
const ZERO: u16 = b'0' as u16;

fn at(text: &[u16], p: usize) -> u16 {
    text.get(p).copied().unwrap_or(0)
}

/// `BtlText_IsNewline`.
fn is_newline(c: u16) -> bool {
    c == 10 || c == 13
}

/// `BtlText_NextLine`: behind the next newline, or at the terminator.
fn next_line(text: &[u16], mut p: usize) -> usize {
    while at(text, p) != 0 {
        if is_newline(at(text, p)) {
            return p + 1;
        }
        p += 1;
    }
    p
}

/// The line from `p` to its newline (what a print with flag 1 shows).
fn line(text: &[u16], p: usize) -> &[u16] {
    let rest = text.get(p..).unwrap_or_default();
    let end = rest
        .iter()
        .position(|&c| c == 0 || is_newline(c))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// `BtlText_CheckLineMask`: steps over the two leading characters; whether the mask digit has the controller
/// type's bit.
fn check_line_mask(text: &[u16], p: &mut usize, ctx: &TextCtx) -> bool {
    let mask: u32 = match at(text, *p + 1) {
        0x46 => 0xF,
        0x31 => 1,
        0x32 => 2,
        0x34 => 4,
        0x38 => 8,
        0x45 => 0xE,
        0x44 => 0xD,
        0x42 => 0xB,
        0x37 => 7,
        _ => 0,
    };
    *p += 2;
    (mask >> ctx.pad_type) & 1 != 0
}

/// `BtlText_CheckUnlock`: steps over an `&ddd` prefix; false when that character is locked in the save (086:
/// when the fighter changed body).
fn check_unlock(text: &[u16], p: &mut usize, ctx: &TextCtx) -> bool {
    if at(text, *p) != AMP {
        return true;
    }
    let digit = |k: usize| i32::from(at(text, *p + k)) - i32::from(ZERO);
    let id = digit(1) * 100 + digit(2) * 10 + digit(3);
    *p += 4;
    if id == 0x56 {
        !ctx.body_changed
    } else {
        let id = id.max(0) as usize;
        ctx.chara_bits
            .get(id / 64)
            .is_some_and(|w| (w >> (id % 64)) & 1 != 0)
    }
}

/// `BtlText_CountEntries`: the pages and each page's entries.
pub(crate) fn count_entries(list: &mut SkillList, script: Option<&[u16]>, ctx: &TextCtx) {
    let Some(text) = script else {
        return;
    };
    list.pages = 0;
    list.count = [0; 10];
    let mut page: i32 = -1;
    let mut p = 1;
    while at(text, p) != 0 {
        if !check_line_mask(text, &mut p, ctx) {
            p = next_line(text, p);
            continue;
        }
        check_unlock(text, &mut p, ctx);
        let tag = at(text, p);
        if tag == DOLLAR {
            page = i32::from(at(text, p + 1)) - i32::from(ZERO);
        }
        match tag {
            DOLLAR => list.pages += 1,
            STAR => {
                // count[-1] is the word in front of the table: the page count.
                if page < 0 {
                    list.pages += 1;
                } else if let Some(c) = list.count.get_mut(page as usize) {
                    *c += 1;
                }
            }
            AT => break,
            _ => {}
        }
        p = next_line(text, p);
    }
}

/// Where the list draws: the menu sheet, the prims and the sprites' scissor (`SCISSOR_1`, inclusive).
pub(crate) struct Painter<'a> {
    pub sheet: &'a HudSheet,
    pub out: &'a mut Vec<HudPrim>,
    pub scissor: [i32; 4],
}

impl Painter<'_> {
    /// `BtlText_DrawPart`: a w x h rectangle of sheet entry `part` at (x, y); a negative h flips it.
    fn part(&mut self, x: i32, y: i32, w: i32, h: i32, part: usize) {
        let rect = [x, y, x + w, y + h];
        let uv = [0, 0, w, h.abs()];
        crate::btl_menu::push_cut(self.sheet, self.out, part, rect, uv, self.scissor);
    }
}

/// `BTLTEXT_COL`: a colour component scaled by the line alpha.
fn col(alpha: f32, v: f32) -> u8 {
    (alpha * v) as u32 as u8
}

/// `BtlText_DrawPageIcon`: the icon of a page title, digit 0..8 (7 and 8 share one), as wide as the list.
fn page_icon(font: &Font, paint: &mut Painter, x: i32, y: i32, digit: u16) {
    let w = font.line_height() * 20;
    let part = match i32::from(digit) - i32::from(ZERO) {
        d @ 0..=6 => d as usize,
        7 | 8 => 7,
        _ => 0,
    };
    paint.part(x, y, w, 0x20, part + 7);
}

/// `Font_SetClip` and `BtlText_PutScissor` with the same box.
fn clip(font: &mut Font, paint: &mut Painter, c: [i32; 4]) {
    font.style.clip = c;
    paint.scissor = c;
}

/// Prints a line scaled down to fit when it is too wide (`Font_FitScale` of `measured` against `width`).
fn print_fit(font: &mut Font, x: i32, y: i32, measured: &[u16], text: &[u16], width: i32) {
    let fit = font.fit_scale(measured, width);
    if fit < 1.0 {
        font.push_style();
        font.style.scale_x *= fit;
        font.print_at(x, y, text);
        font.pop_style();
    } else {
        font.print_at(x, y, text);
    }
}

/// `BtlText_DrawList` in the box (x0, y0)-(x1, y1). Mode 0: the current page's title and its entries, scrolled,
/// the selected one highlighted; other modes: the selected entry's detail lines (`%`) and note (`#`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_list(
    font: &mut Font,
    paint: &mut Painter,
    list: &SkillList,
    script: Option<&[u16]>,
    ctx: &TextCtx,
    [x0, x1, y0, y1]: [i32; 4],
    mode: i32,
) {
    let Some(text) = script else {
        return;
    };
    let page_now = list.page.clamp(0, 9) as usize;
    let cursor = list.cursor[page_now];
    let line_h = font.line_height() * 20;
    let mut y = y0;
    if mode == 0 {
        y -= list.scroll[page_now] * 28;
    }
    font.push_style();
    font.style.flags = 1;
    font.style.scale_x = 1.0;
    font.style.scale_y = 1.0;
    font.style.shadow_mode = 2;
    font.style.color = rgba(0xFF, 0xFF, 0xFF, 0x80);
    font.style.shadow_color = rgba(0, 0x10, 0x10, 0x40);
    let list_clip = if mode == 0 {
        [x0, y0, x1, y1]
    } else {
        [x0, y0 - 7, x1, y1 + 7]
    };
    clip(font, paint, list_clip);
    let mut page: i32 = -1;
    let mut entry: i32 = 0;
    let mut alpha = 1.0f32;
    let mut p = 1;
    while at(text, p) != 0 {
        if !check_line_mask(text, &mut p, ctx) {
            p = next_line(text, p);
            continue;
        }
        if !check_unlock(text, &mut p, ctx) {
            alpha = 0.5;
        }
        let tag = at(text, p);
        match tag {
            DOLLAR => {
                page = i32::from(at(text, p + 1)) - i32::from(ZERO);
                entry = -1;
            }
            STAR => entry += 1,
            _ => {}
        }
        if tag != AT && page != list.page {
            p = next_line(text, p);
            continue;
        }
        // BtlText_IsLineDimmed never dims.
        if mode == 0 {
            match tag {
                DOLLAR => {
                    font.push_style();
                    let full = [0, 0, 0x1FF, 0x1BF];
                    paint.scissor = full;
                    font.style.clip = full;
                    font.style.color = rgba(0xFF, 0xFF, 0xFF, 0x80);
                    font.style.shadow_color = rgba(0x56, 0x3D, 0x39, 0x80);
                    font.print_at(x0 + 0x1E, y0 - 0x22, line(text, p + 2));
                    page_icon(font, paint, x0, y0 - 0x28, at(text, p + 1));
                    paint.scissor = list_clip;
                    font.pop_style();
                }
                STAR => {
                    if y0 < y + 0x20 && y < y1 {
                        let icon = [1, 2, 3].map(|k| i32::from(at(text, p + k)) - i32::from(ZERO));
                        if entry == cursor {
                            font.style.color =
                                rgba(col(alpha, 255.0), col(alpha, 247.0), col(alpha, 15.0), 0x80);
                            font.style.shadow_color =
                                rgba(col(alpha, 28.0), col(alpha, 32.0), col(alpha, 21.0), 0x40);
                        } else {
                            font.style.color = rgba(
                                col(alpha, 255.0),
                                col(alpha, 255.0),
                                col(alpha, 255.0),
                                0x80,
                            );
                            font.style.shadow_color =
                                rgba(col(alpha, 32.0), col(alpha, 43.0), col(alpha, 94.0), 0x40);
                        }
                        // The fit is measured from the second icon digit on; the name starts behind the third.
                        print_fit(
                            font,
                            x0 + 0xF,
                            y + 3,
                            line(text, p + 2),
                            line(text, p + 4),
                            (x1 - x0) - 0x44,
                        );
                        let h = line_h - 2;
                        let bar = if entry == cursor { 0x11 } else { 0x10 };
                        paint.part(x0 - 1, y, h, 0x1F, bar);
                        if icon[0] > 0 {
                            paint.part(x1 - 0x35, y - 5, h, 0x1F, 0x13);
                        }
                        if icon[1] > 0 {
                            paint.part(x1 - 0x20, y - 5, h, 0x1F, (icon[1] + 0x1C) as usize);
                        }
                        if icon[2] > 0 {
                            paint.part(x1 - 0x20, y - 5, h, 0x1F, (icon[2] + 0x13) as usize);
                        }
                    }
                    y += 0x1C;
                }
                AT => break,
                _ => {}
            }
        } else {
            match tag {
                PERCENT => {
                    let (c, s) = match at(text, p + 1) {
                        0x30 => ([170.0, 215.0, 255.0], [43.0, 54.0, 64.0]),
                        0x31 => ([239.0, 89.0, 89.0], [64.0, 17.0, 0.0]),
                        _ => ([0.0; 3], [0.0; 3]),
                    };
                    if matches!(at(text, p + 1), 0x30 | 0x31) && entry == cursor {
                        font.style.color =
                            rgba(col(alpha, c[0]), col(alpha, c[1]), col(alpha, c[2]), 0x80);
                        font.style.shadow_color =
                            rgba(col(alpha, s[0]), col(alpha, s[1]), col(alpha, s[2]), 0x40);
                        let shown = line(text, p + 2);
                        print_fit(font, x0 + 0xF, y + 3, shown, shown, (x1 - x0) - 0x1E);
                        y += 0x1C;
                    }
                }
                HASH => {
                    if entry == cursor {
                        font.style.color2 = rgba(
                            col(alpha, 128.0),
                            col(alpha, 128.0),
                            col(alpha, 128.0),
                            0x80,
                        );
                        let note = line(text, p + 1);
                        font.print_at(x0 + 0xF, y, note);
                        y += 0x1C;
                        font.style.color2 = rgba(0x80, 0x80, 0x80, 0x80);
                        if ctx.pad_type == 1 && font.pad_tag_size(note) > 0x20 {
                            y += 0x14;
                        }
                    }
                }
                AT => break,
                _ => {}
            }
        }
        alpha = 1.0;
        p = next_line(text, p);
    }
    font.pop_style();
    paint.scissor = [0, 0, 0x1FF, 0x1BF];
}

/// `BtlText_DrawScrollBar`: the frame, then the thumb (shortened and moved by the cursor when the page has more
/// than seven entries).
pub(crate) fn draw_scroll_bar(paint: &mut Painter, list: &SkillList, x: i32, y0: i32, y1: i32) {
    let visible = 7;
    let page = list.page.clamp(0, 9) as usize;
    let (mut x, mut y0, mut y1) = (x, y0, y1);
    let h = y1 - y0;
    let half = h / 2;
    let mut count = list.count[page];
    let scroll = list.cursor[page];
    paint.part(x, y0, 9, h - half, 5);
    paint.part(x, y1, 9, -half, 5);
    y0 += 2;
    x += 2;
    y1 -= 2;
    let over = visible < count;
    count = (count - 1).max(0);
    let len = y1 - y0;
    if over {
        let unit = len as f32 / (count + 7) as f32;
        y0 += (scroll as f32 * unit) as i32;
        y1 = y0 + (unit * visible as f32) as i32;
    }
    let h = y1 - y0;
    let half = h / 2;
    paint.part(x, y0 - 0x1B, 5, h - half + 0x1B, 6);
    paint.part(x, y1 + 0x1B, 5, -(half + 0x1B), 6);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A script as the character files hold it: a byte-order mark, lines ending in LF.
    fn script(s: &str) -> Vec<u16> {
        std::iter::once(0xFEFF).chain(s.encode_utf16()).collect()
    }

    fn ctx() -> TextCtx {
        TextCtx {
            pad_type: 0,
            chara_bits: [u64::MAX; 3],
            body_changed: false,
        }
    }

    #[test]
    fn counts_pages_and_the_entries_its_controller_sees() {
        // Line masks: F all types, 1 type 0 only, 2 type 1 only.
        let text = script("!F$0One\n!F*000A\n!2*000B\n!1*000C\n!F$1Two\n!F*000D\n!F@\n!F*000E\n");
        let mut list = SkillList::default();
        count_entries(&mut list, Some(&text), &ctx());
        assert_eq!(list.pages, 2);
        assert_eq!(&list.count[..2], &[2, 1]);
    }

    #[test]
    fn a_locked_character_dims_a_line_but_keeps_it() {
        let mut c = ctx();
        c.chara_bits = [0; 3];
        let text = script("!F$0One\n!F&005*000A\n!F@\n");
        let mut list = SkillList::default();
        count_entries(&mut list, Some(&text), &c);
        assert_eq!(list.count[0], 1);
        let mut p = 1 + "!F$0One\n".len() + 2;
        assert!(!check_unlock(&text, &mut p, &c));
        assert_eq!(at(&text, p), STAR);
    }
}
