//! The menus' message window (`MsgWin`, src/ui/message_window.c) with its text box (`TextBox` preset 5), and the
//! icon window (`IconWin`, src/ui/icon_window.c).

use std::error::Error;

use bt3_data::font::message;

use crate::draw::ScreenQuad;
use crate::flash::{DrawOver, Flash, screen_quad};
use crate::font::{Font, rgba};

/// What `TextBox_AttachLine` hands to its clip callback (`TextBoxDraw`).
#[derive(Clone, Debug)]
struct TextDraw {
    x: i32,
    y: i32,
    color: [i32; 4],
    shadow: [i32; 4],
    text: Vec<u16>,
}

/// `TextBox_DrawClip`: moves the text by the clip's position and tints it with the clip's colour transform.
fn draw_clip(draw: &TextDraw, over: &DrawOver, font: &mut Font) {
    let tint = |c: i32, i: usize| {
        ((c as f32 * over.mul[i]) as i32 + i32::from(over.add[i])).clamp(0, 0xFF)
    };
    let x = (draw.x as f32 + over.x) as i32;
    let y = (draw.y as f32 + over.y) as i32;
    let color: Vec<u8> = (0..4).map(|i| tint(draw.color[i], i) as u8).collect();
    let shadow: Vec<u8> = (0..4).map(|i| tint(draw.shadow[i], i) as u8).collect();
    if color[3] == 0 {
        return;
    }
    font.push_style();
    font.style.align = 0;
    font.style.scale_x = 1.0;
    font.style.scale_y = 1.0;
    font.style.color = rgba(color[0], color[1], color[2], color[3]);
    font.style.shadow_mode = 2;
    font.style.shadow_dx = 1;
    font.style.shadow_dy = 2;
    font.style.shadow_color = rgba(shadow[0], shadow[1], shadow[2], shadow[3]);
    font.style.clip = [0, 0, 0x1FF, 0x1BF];
    font.style.spacing_x = 0;
    font.style.spacing_y = 0;
    font.print_at(x, y, &draw.text);
    font.pop_style();
}

/// `MsgWin` on the right side (`MsgWin_Init(pack, text, 1)`), showing one line of `text` on "mc_message_text".
pub struct MsgWin {
    flash: Flash,
    textures: Vec<Option<usize>>,
    text: Vec<u8>,
    draw: Option<TextDraw>,
    right: bool,
}

impl MsgWin {
    pub fn new(
        fod: &[u8],
        textures: Vec<Option<usize>>,
        text: Vec<u8>,
        right: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(fod)?;
        flash.play(1);
        Ok(Self {
            flash,
            textures,
            text,
            draw: None,
            right,
        })
    }

    pub fn open(&mut self) {
        let label = if self.right { "fl_r_in" } else { "fl_l_in" };
        self.flash.goto_label(label, true);
    }

    pub fn close(&mut self) {
        let label = if self.right { "fl_r_out" } else { "fl_l_out" };
        self.flash.goto_label(label, true);
    }

    /// `MsgWin_Draw(_, _, line)`.
    pub fn draw(&mut self, line: i32, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        self.flash.advance();
        let clip = self.flash.find(None, "mc_message_text");
        self.attach_line(clip, line);
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash.draw_with_overs(&has_texture);
        quads.extend(flash_quads.iter().map(|q| screen_quad(q, textures)));
        if let Some(draw) = &self.draw {
            for over in &overs {
                draw_clip(draw, over, font);
            }
        }
        font.flush(quads);
    }

    /// `TextBox_AttachLine(flash, ref, 0, 0, line, box)` with preset 5: left aligned, rows offset by 0x20, 0x14, 0xA.
    fn attach_line(&mut self, clip: crate::flash::ClipRef, line: i32) {
        let text = (line >= 0)
            .then(|| message(&self.text, line as usize).ok())
            .flatten();
        let Some(text) = text.filter(|_| clip.found()) else {
            self.flash.clip_set_draw_over(clip, None);
            self.draw = None;
            return;
        };
        let alpha = self.flash.clip_get_alpha(clip);
        let y = match Font::count_lines(&text) {
            1 => 0x20,
            2 => 0x14,
            3 => 0xA,
            _ => 0,
        };
        self.draw = Some(TextDraw {
            x: 0,
            y,
            color: [0xFF, 0xFF, 0xFF, i32::from((alpha * 128.0) as u32 as u8)],
            shadow: [0x20, 0x20, 0x20, i32::from((alpha * 64.0) as u32 as u8)],
            text,
        });
        self.flash.clip_set_draw_over(clip, Some(0));
    }
}

/// `IconWin`: a movie window with an icon picked from a second texture file.
pub struct IconWin {
    flash: Flash,
    textures: Vec<Option<usize>>,
}

impl IconWin {
    pub fn new(fod: &[u8], textures: Vec<Option<usize>>) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(fod)?;
        flash.play(1);
        Ok(Self { flash, textures })
    }

    pub fn open(&mut self) {
        self.flash.goto_label("fl_in", true);
    }

    pub fn close(&mut self) {
        self.flash.goto_label("fl_out", true);
    }

    pub fn draw(&mut self, quads: &mut Vec<ScreenQuad>) {
        self.flash.advance();
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        quads.extend(
            self.flash
                .draw(&has_texture)
                .iter()
                .map(|q| screen_quad(q, textures)),
        );
    }
}
