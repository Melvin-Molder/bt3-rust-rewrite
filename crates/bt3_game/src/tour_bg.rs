//! The backdrop of the tournament screens (`TourBg`, src/menu/tour_background.c): a picture and a movie whose cloud
//! layer drifts sideways, from the tournament's file (0x3C9 + tournament). The entrant select and the bracket draw
//! it under everything.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim;
use crate::draw::ScreenQuad;
use crate::flash::{Flash, Uv, screen_quads};
use crate::logo::picture_quads;
use crate::menu_res::{decode_file, decode_picture};

/// The tournament's file: `0x3C9 + tournament`.
pub const FILE_FIRST: u32 = 0x3C9;

/// What `TourBg_Init` reads from the tournament's (unpacked) file.
pub struct TourBgAssets {
    pub fod: Vec<u8>,
    pub background: Vec<RgbaTexture>,
    /// Section 2's textures.
    pub textures: Vec<RgbaTexture>,
}

impl TourBgAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            fod: pack_section(res, 3)?.to_vec(),
            background: decode_picture(res, 1)?,
            textures: decode_file(pack_section(res, 2)?)?,
        })
    }
}

/// The registered backdrop of one tournament: its movie, picture and where section 2's textures went.
#[derive(Clone, Debug)]
pub struct TourBgData {
    pub fod: Vec<u8>,
    pub background: usize,
    /// Texture `n` of section 2 is at `first + 2n`.
    pub first: usize,
}

/// `TourBg`.
pub struct TourBg {
    flash: Flash,
    tex: Vec<Option<usize>>,
    background: usize,
    kind: i32,
    scroll: f32,
}

impl TourBg {
    /// `TourBg_Init(file, kind, tex)`: the movie's textures by tournament; `extra` the caller's three textures for
    /// the movie's last image records.
    pub fn new(
        data: &TourBgData,
        kind: i32,
        extra: Option<[Option<usize>; 3]>,
    ) -> Result<Self, Box<dyn Error>> {
        let e = |n: usize| Some(data.first + 2 * n);
        let mut tex = vec![None; 10];
        match kind {
            0 | 2 | 4 => {
                tex[0] = e(0);
                tex[2] = e(1);
                tex[1] = e(2);
            }
            1 => {
                tex[0] = e(0);
                tex[4] = e(1);
                tex[3] = e(2);
                tex[2] = e(3);
                tex[1] = e(4);
                tex[6] = e(5);
                tex[5] = e(6);
            }
            3 => {
                tex[0] = e(0);
                tex[1] = e(1);
                tex[2] = e(2);
            }
            _ => {}
        }
        if let Some(extra) = extra {
            let at = if kind == 1 { 7 } else { 3 };
            tex[at..at + 3].copy_from_slice(&extra);
        }
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        Ok(Self {
            flash,
            tex,
            background: data.background,
            kind,
            scroll: 0.0,
        })
    }

    /// `TourBg_Draw`: the movie advances; the picture, then the movie with its clouds scrolled (not in the Big
    /// Tournament and the Otherworld Tournament).
    pub fn draw(&mut self, quads: &mut Vec<ScreenQuad>) {
        self.flash.advance();
        quads.extend(
            picture_quads(self.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let r = self.flash.find(None, "mc_bg_move_obj_kumo");
        let uv = Uv {
            u0: 0,
            v0: 0,
            u1: 0x200,
            v1: 0x100,
            tex: 0,
        };
        let speed = match self.kind {
            0 => Some(-0.142_222_22),
            2 => Some(-0.170_666_67),
            4 => Some(0.155_151_51),
            _ => None,
        };
        if let Some(dx) = speed {
            anim::scroll(&mut self.flash, r, uv, &mut self.scroll, dx);
        }
        let tex = &self.tex;
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        quads.extend(screen_quads(&self.flash.draw(&has_texture), tex));
    }
}
