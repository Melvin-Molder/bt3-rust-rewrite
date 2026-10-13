//! The menus' reward window (`GetWin`, src/ui/reward_window.c): the "title_get" movie announcing one reward at a
//! time, with a title strip and an icon by its kind and up to two lines of text or an amount of money.

use std::error::Error;

use bt3_data::pack_section;

use crate::draw::ScreenQuad;
use crate::flash::{Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::menu_res::ImageTexture;
use crate::title::TitleSound;
use crate::windows::{TextBox, TextDraw, draw_texts};

/// `GETWIN_KIND_*` the story result gives: a character, a stage, an item (shown as kinds 3..5 by its type), a
/// saga, an episode, a dragon ball.
pub(crate) const KIND_CHARA: i32 = 0;
pub(crate) const KIND_STAGE: i32 = 1;
pub(crate) const KIND_ITEM: i32 = 2;
pub(crate) const KIND_SAGA: i32 = 7;
pub(crate) const KIND_EPISODE: i32 = 8;
pub(crate) const KIND_DRAGON_BALL: i32 = 9;
/// Kind 6: an amount of money.
pub(crate) const KIND_MONEY: i32 = 6;

/// `gGetWin->tex[n]` of `GetWin_Init(pack)`, the pack being section `pack` of the screen's resource.
pub(crate) fn textures(pack: usize) -> [ImageTexture; 13] {
    let mut t = [None; 13];
    let k = |s: usize, e: usize| Some(((pack, s), e));
    t[0] = k(1, 0);
    t[12] = k(1, 1);
    t[2] = k(1, 2);
    t[1] = k(1, 3);
    t[5] = k(2, 1);
    t[6] = k(2, 0);
    t[8] = k(2, 2);
    t[9] = k(3, 1);
    t[10] = k(3, 2);
    t[11] = k(3, 0);
    t[7] = k(4, 0);
    t
}

/// What `GetWin_Init` reads from its pack.
#[derive(Clone, Debug)]
pub(crate) struct GetWinData {
    /// Section 5 (language 1).
    fod: Vec<u8>,
    /// Sections 8, 7, 9, 11, 11: one per text box.
    texts: [Vec<u8>; 5],
    /// Section 10: per item, its type (+0 of 0x28 bytes).
    item_types: Vec<u8>,
}

impl GetWinData {
    pub(crate) fn parse(pack: &[u8]) -> Result<Self, Box<dyn Error>> {
        Self::parse_lang(pack, 1)
    }

    /// `GetWin_Init(pack, lang)`: the movie of section 5 for language 1, else of section 6.
    pub(crate) fn parse_lang(pack: &[u8], lang: i32) -> Result<Self, Box<dyn Error>> {
        let s =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(pack, n)?.to_vec()) };
        Ok(Self {
            fod: s(if lang == 1 { 5 } else { 6 })?,
            texts: [s(8)?, s(7)?, s(9)?, s(11)?, s(11)?],
            item_types: s(10)?.chunks_exact(0x28).map(|e| e[0]).collect(),
        })
    }
}

/// `GetWin`.
pub(crate) struct GetWin {
    flash: Flash,
    textures: Vec<Option<usize>>,
    texts: [Vec<u8>; 5],
    item_types: Vec<u8>,
    kind: i32,
    box1: usize,
    box2: usize,
    line1: i32,
    line2: i32,
    tall: bool,
    se: u32,
    /// The second line is hung on its clip after the movie is drawn, so a frame shows the line of the frame before.
    line2_draw: Option<TextDraw>,
}

impl GetWin {
    /// `GetWin_Init(pack, 1)`.
    pub(crate) fn new(
        data: &GetWinData,
        textures: Vec<Option<usize>>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        Ok(Self {
            flash,
            textures,
            texts: data.texts.clone(),
            item_types: data.item_types.clone(),
            kind: 0,
            box1: 0,
            box2: 0,
            line1: -1,
            line2: 0,
            tall: false,
            se: 0,
            line2_draw: None,
        })
    }

    /// Text box `n` (`TextBox_Init(.., 0)`, centred 0x100 on; box 1 yellow).
    fn text_box(n: usize) -> TextBox {
        let b = TextBox::preset(0).with_align(1).with_offset(0x100, 0);
        if n == 1 {
            b.with_color([0xFF, 0xFF, 0x00, 0x80])
        } else {
            b
        }
    }

    /// `GetWin_Setup(kind, value)`: what the window announces.
    pub(crate) fn setup(&mut self, kind: i32, value: i32) {
        self.line1 = -1;
        self.line2 = value.abs();
        self.tall = false;
        self.box1 = 0;
        self.box2 = 0;
        let mut kind = kind;
        match kind {
            KIND_ITEM => {
                kind = match self.item_types.get(value.max(0) as usize).copied() {
                    Some(0) => 3,
                    Some(1) => 4,
                    _ => 5,
                };
                self.box2 = 2;
                self.se = 6;
            }
            KIND_SAGA => {
                self.box2 = 3;
                self.se = 7;
            }
            KIND_EPISODE => {
                self.se = 8;
                // The episode's saga (episodes are counted through all of them).
                self.line1 = match self.line2 {
                    0..3 => 0,
                    3..7 => 1,
                    7..14 => 2,
                    14..19 => 3,
                    19..35 => 4,
                    35..40 => 5,
                    40..44 => 6,
                    44..48 => 7,
                    n => n,
                };
                self.box1 = 3;
                self.box2 = 4;
                self.line2 += 8;
                // Font_CountLines is at least 1: any line there is takes the tall layout.
                self.tall = bt3_data::font::message(&self.texts[3], self.line2 as usize).is_ok();
            }
            KIND_CHARA => {
                self.line1 = self.line2;
                self.box1 = 0;
                self.box2 = 1;
                self.se = 7;
            }
            KIND_MONEY => self.se = 6,
            KIND_STAGE => {
                self.line2 += 0x38;
                self.box2 = 3;
                self.se = 7;
            }
            KIND_DRAGON_BALL => {
                self.line2 += 0x5B;
                self.box2 = 4;
                self.se = 8;
            }
            _ => {}
        }
        self.kind = kind;
    }

    /// `GetWin_Open`.
    pub(crate) fn open(&mut self, sounds: &mut Vec<TitleSound>) {
        self.flash.goto_label("fl_get_in", true);
        sounds.push(TitleSound::Effect(2, self.se));
    }

    /// `GetWin_Close`.
    pub(crate) fn close(&mut self) {
        self.flash.goto_label("fl_get_out", true);
    }

    /// `GetWin_Next`.
    pub(crate) fn next(&mut self, sounds: &mut Vec<TitleSound>) {
        self.flash.goto_label("fl_next_get", true);
        sounds.push(TitleSound::Effect(2, self.se));
    }

    /// `GetWin_IsAnimDone`: the movie's trigger 0 fired this frame.
    pub(crate) fn is_anim_done(&self) -> bool {
        self.flash.trig() & 1 != 0
    }

    /// `GetWin_Draw`.
    pub(crate) fn draw(&mut self, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        self.flash.advance();
        let kind = self.kind;
        let f = &mut self.flash;
        let strip = match kind {
            3..=5 | 9 => 0x80,
            1 | 8 => 0x40,
            KIND_MONEY => 0xC0,
            _ => 0,
        };
        let clip = f.find(None, "title_get");
        f.clip_set_uv(
            clip,
            Uv {
                u0: 0,
                v0: strip,
                u1: 0x200,
                v1: strip + 0x40,
                tex: 0,
            },
        );
        f.clip_set_tex(clip, i32::from((7..=9).contains(&kind)));
        let (u0, v0, u1, v1) = match kind {
            0 => (0x40, 0, 0x80, 0x40),
            1 => (0x80, 0, 0xC0, 0x40),
            3 => (0xC0, 0, 0x100, 0x40),
            4 => (0, 0x40, 0x40, 0x80),
            5 => (0x40, 0x40, 0x80, 0x80),
            6 => (0x80, 0x40, 0xC0, 0x80),
            8 => (0, 0, 0x80, 0x80),
            9 => (0xC0, 0x40, 0x100, 0x80),
            _ => (0, 0, 0x40, 0x40),
        };
        let icon = if kind != 8 {
            "mc_icon_get_1_loop"
        } else {
            "mc_icon_get_2_loop"
        };
        let clip = f.find(None, icon);
        f.clip_set_uv(
            clip,
            Uv {
                u0,
                v0,
                u1,
                v1,
                tex: 0,
            },
        );
        let clip = f.find(None, "mc_icon_get_2_loop");
        f.clip_set_flags(clip, PROP_VISIBLE, kind == 8);
        let clip = f.find(None, "mc_icon_get_1_loop");
        f.clip_set_flags(clip, PROP_VISIBLE, kind != 8);
        for i in 0..7 {
            let clip = f.find(None, &format!("mc_pay_num_{i}"));
            f.clip_set_flags(clip, PROP_VISIBLE, kind == KIND_MONEY);
        }
        if kind == KIND_MONEY {
            crate::anim::num_draw(
                f,
                |n| format!("mc_pay_num_{n}"),
                7,
                self.line2,
                (0x20, 0x20),
                0,
            );
        }
        let clip = f.find(None, "mc_battle_text_base");
        f.clip_set_flags(clip, PROP_VISIBLE, kind == 8);
        let mut texts: Vec<TextDraw> = Vec::new();
        if self.line1 >= 0 {
            let clip = f.find(None, "mc_dammy_text_1");
            texts.extend(Self::text_box(self.box1).attach_line(
                f,
                clip,
                0,
                &self.texts[self.box1],
                self.line1,
                font,
            ));
        } else {
            for name in ["mc_text_base_1", "mc_dammy_text_1"] {
                let clip = f.find(None, name);
                f.clip_set_flags(clip, PROP_VISIBLE, false);
            }
        }
        texts.extend(self.line2_draw.take());
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = f.draw_with_overs(&has_texture);
        quads.extend(screen_quads(&flash_quads, textures));
        draw_texts(&texts, &overs, font);
        if self.line2 >= 0 {
            let mut b = Self::text_box(self.box2);
            if kind == 8 {
                if self.tall {
                    b = b.with_line_y([0xC, 0, 0, 0, 0]);
                }
                let clip = f.find(None, "mc_text_base_2");
                f.clip_set_flags(clip, PROP_VISIBLE, false);
            }
            let clip = f.find(None, "mc_dammy_text_2");
            if kind != KIND_MONEY {
                self.line2_draw =
                    b.attach_line(f, clip, 1, &self.texts[self.box2], self.line2, font);
            }
        } else {
            let clip = f.find(None, "mc_text_base_2");
            f.clip_set_flags(clip, PROP_VISIBLE, kind == KIND_MONEY);
            let clip = f.find(None, "mc_dammy_text_2");
            f.clip_set_flags(clip, PROP_VISIBLE, false);
        }
        font.flush(quads);
    }
}
