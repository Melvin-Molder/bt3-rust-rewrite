//! `PassWin` (src/menu/password_window.c): the window that shows a character's password, one picture a character,
//! made by `ChrPass_Encode`. The Data Center's custom character list and Evolution Z's customising screen open it.

use crate::draw::ScreenQuad;
use crate::flash::{Flash, Uv, screen_quads};
use crate::menu_res::ImageTexture;
use crate::password::ChrPass;
use crate::rng::LibcRandom;

/// `PassWin_Init(MPACK_AT(res, section))`: `gPassWin->tex[n]`.
pub(crate) const fn textures(section: usize) -> [ImageTexture; 4] {
    [
        Some(((section, 2), 0)),
        Some(((section, 2), 1)),
        Some(((section, 3), 0)),
        Some(((section, 2), 2)),
    ]
}

/// `gPassKeys`: the window's sheet, four rows of capitals, symbols and digits (small letters four rows on).
const PASS_KEYS: [&[u8]; 4] = [
    b"ABCDEFGHIJKLMN",
    b"OPQRSTUVWXYZ",
    b"#$%&@!?-+*()",
    b"0123456789",
];

/// `PassWin`.
pub(crate) struct PassWin {
    flash: Flash,
    tex: Vec<Option<usize>>,
    /// `ChrPass_GetText`: the last password made.
    text: String,
}

impl PassWin {
    /// `PassWin_Init`: its movie (playing) and textures.
    pub(crate) fn new(flash: Flash, tex: Vec<Option<usize>>) -> Self {
        Self {
            flash,
            tex,
            text: String::new(),
        }
    }

    /// `ChrPass_GetText`.
    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// `PassWin_OpenEx(items, chara, extraSlots)`: the password (`ChrPass_Encode`) and the window's way in.
    pub(crate) fn open(
        &mut self,
        chara: i32,
        items: [u16; 8],
        extra_slots: i32,
        libc: &mut LibcRandom,
    ) {
        let pass = ChrPass {
            chara,
            items: items.map(i32::from),
            extra_slots,
            unk28: 0,
        };
        self.text = pass.encode(libc);
        self.flash.goto_label("fl_password_in", true);
    }

    /// `PassWin_Close`.
    pub(crate) fn close(&mut self) {
        self.flash.goto_label("fl_password_out", true);
    }

    /// `PassWin_Draw`: the movie advances and each of the 34 cells gets its character's picture.
    pub(crate) fn draw(&mut self, quads: &mut Vec<ScreenQuad>) {
        self.flash.advance();
        let (mut col, mut row) = (0, 0);
        for (i, c) in self.text.bytes().take(34).enumerate() {
            // PassWin_FindKey: case is ignored; a character not on the sheet keeps the last cell.
            let up = c.to_ascii_uppercase();
            for (j, keys) in PASS_KEYS.iter().enumerate() {
                if let Some(k) = keys.iter().position(|&k| k == up) {
                    (col, row) = (k as i32, j as i32);
                    break;
                }
            }
            let r = if c.is_ascii_lowercase() { row + 4 } else { row };
            let name = if i <= 0x10 {
                format!("mc_input_pass_1_{:02}", i + 1)
            } else {
                format!("mc_input_pass_2_{:02}", i - 0x10)
            };
            let clip = self.flash.find(None, &name);
            self.flash.clip_set_uv(
                clip,
                Uv {
                    u0: col * 0x20,
                    v0: r * 0x28,
                    u1: col * 0x20 + 0x20,
                    v1: r * 0x28 + 0x28,
                    tex: 0,
                },
            );
        }
        let tex = &self.tex;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        quads.extend(screen_quads(&self.flash.draw(&has_texture), tex));
    }
}
