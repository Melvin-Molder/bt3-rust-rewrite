//! The menus' message window (`MsgWin`, src/ui/message_window.c) with its text box (`TextBox` preset 5), and the
//! icon window (`IconWin`, src/ui/icon_window.c).

use std::error::Error;

use bt3_data::font::message;

use crate::draw::ScreenQuad;
use crate::flash::{ClipRef, DrawOver, Flash, screen_quad};
use crate::font::{Font, rgba};

/// A text box style (`TextBox_Init` presets, src/ui/menu_util_1.c).
#[derive(Clone, Copy, Debug, Default)]
pub struct TextBox {
    align: i32,
    x: i32,
    y: i32,
    /// `TEXTBOX_FLAG_COLOR`: RGBA, the alpha scaled by the clip's.
    color: Option<[u8; 4]>,
    /// `TEXTBOX_FLAG_MAX_W`: a wider line is shrunk horizontally.
    max_w: Option<i32>,
    /// `TEXTBOX_FLAG_MAX_H`: a taller text is shrunk vertically.
    max_h: Option<i32>,
    /// `TEXTBOX_FLAG_SPACING`: character and line spacing.
    spacing: (i32, i32),
    /// `TEXTBOX_FLAG_LINE_Y`: extra y for texts of one to five rows.
    line_y: Option<[i32; 5]>,
    /// `TextBox_SetClip`: `[x0, y0, x1, y1]`, inclusive.
    clip: Option<[i32; 4]>,
}

impl TextBox {
    pub fn preset(preset: u32) -> Self {
        let yellow = Some([0xFF, 0xFF, 0x00, 0x80]);
        match preset {
            1 => Self {
                align: 2,
                x: 0x100,
                max_w: Some(0xE1),
                ..Self::default()
            },
            2 => Self {
                max_w: Some(0xE1),
                ..Self::default()
            },
            3 => Self {
                align: 2,
                x: 0x100,
                color: yellow,
                max_w: Some(0xD4),
                ..Self::default()
            },
            4 => Self {
                color: yellow,
                max_w: Some(0xD4),
                ..Self::default()
            },
            5 => Self {
                line_y: Some([0x20, 0x14, 0xA, 0, 0]),
                ..Self::default()
            },
            6 => Self {
                max_w: Some(0x160),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }

    /// `TextBox_SetClip(box, x0, x1, y0, y1)`: the text is cut to the rectangle.
    pub fn with_clip(mut self, x0: i32, x1: i32, y0: i32, y1: i32) -> Self {
        self.clip = Some([x0, y0, x1, y1]);
        self
    }

    /// `TextBox_SetMaxSize`.
    pub fn with_max_size(mut self, w: i32, h: i32) -> Self {
        (self.max_w, self.max_h) = (Some(w), Some(h));
        self
    }

    /// `TextBox_SetSpacing`.
    pub fn with_spacing(mut self, x: i32, y: i32) -> Self {
        self.spacing = (x, y);
        self
    }

    /// `TextBox_SetAlign`.
    pub fn with_align(mut self, align: i32) -> Self {
        self.align = align;
        self
    }

    /// `TextBox_SetOffset`.
    pub fn with_offset(mut self, x: i32, y: i32) -> Self {
        (self.x, self.y) = (x, y);
        self
    }

    /// `TextBox_SetColor`.
    pub fn with_color(mut self, rgba: [u8; 4]) -> Self {
        self.color = Some(rgba);
        self
    }

    /// `TextBox_SetLineOffsets`: extra y for texts of one to five rows (all 0 drops them).
    pub fn with_line_y(mut self, rows: [i32; 5]) -> Self {
        self.line_y = (rows != [0; 5]).then_some(rows);
        self
    }

    /// `TextBox_AttachLine(flash, clip, 0, 0, line, box)`: hangs line `line` of `text` on the clip as `tag`. A negative
    /// line or a missing clip takes the text off the clip.
    pub fn attach_line(
        &self,
        flash: &mut Flash,
        clip: ClipRef,
        tag: u32,
        text: &[u8],
        line: i32,
        font: &Font,
    ) -> Option<TextDraw> {
        self.attach_line_at(flash, clip, tag, text, line, (0, 0), font)
    }

    /// `TextBox_AttachLine(flash, clip, x, y, line, box)`: the same, `(x, y)` from the clip's position.
    #[allow(clippy::too_many_arguments)]
    pub fn attach_line_at(
        &self,
        flash: &mut Flash,
        clip: ClipRef,
        tag: u32,
        text: &[u8],
        line: i32,
        (dx, dy): (i32, i32),
        font: &Font,
    ) -> Option<TextDraw> {
        let text = (line >= 0 && clip.found())
            .then(|| message(text, line as usize).ok())
            .flatten();
        let Some(text) = text else {
            flash.clip_set_draw_over(clip, None);
            return None;
        };
        let alpha = flash.clip_get_alpha(clip);
        let scale_x = match self.max_w {
            Some(max) => {
                let width = font.width(&text);
                if max < width {
                    max as f32 / width as f32
                } else {
                    1.0
                }
            }
            None => 1.0,
        };
        let scale_y = match self.max_h {
            Some(max) => {
                let height = font.height(&text);
                if max < height {
                    max as f32 / height as f32
                } else {
                    1.0
                }
            }
            None => 1.0,
        };
        let row_y = match (self.line_y, Font::count_lines(&text)) {
            (Some(rows), n @ 1..=5) => rows[n as usize - 1],
            _ => 0,
        };
        let color = match self.color {
            Some(c) => [c[0], c[1], c[2], (f32::from(c[3]) * alpha) as u32 as u8],
            None => [0xFF, 0xFF, 0xFF, (alpha * 128.0) as u32 as u8],
        };
        flash.clip_set_draw_over(clip, Some(tag));
        Some(TextDraw {
            tag,
            x: self.x + dx,
            y: self.y + row_y + dy,
            align: self.align,
            scale_x,
            scale_y,
            spacing: self.spacing,
            color: color.map(i32::from),
            shadow: [0x20, 0x20, 0x20, i32::from((alpha * 64.0) as u32 as u8)],
            clip: self.clip.unwrap_or([0, 0, 0x1FF, 0x1BF]),
            text,
        })
    }
}

/// What `TextBox_AttachLine` hands to its clip callback (`TextBoxDraw`).
#[derive(Clone, Debug)]
pub struct TextDraw {
    tag: u32,
    x: i32,
    y: i32,
    align: i32,
    scale_x: f32,
    scale_y: f32,
    spacing: (i32, i32),
    color: [i32; 4],
    shadow: [i32; 4],
    clip: [i32; 4],
    text: Vec<u16>,
}

/// `TextBox_DrawClip` for every `drawOver` call whose tag has a text.
pub fn draw_texts(texts: &[TextDraw], overs: &[DrawOver], font: &mut Font) {
    for over in overs {
        for draw in texts.iter().filter(|d| d.tag == over.tag) {
            draw_clip(draw, over, font);
        }
    }
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
    font.style.align = draw.align;
    font.style.scale_x = draw.scale_x;
    font.style.scale_y = draw.scale_y;
    font.style.color = rgba(color[0], color[1], color[2], color[3]);
    font.style.shadow_mode = 2;
    font.style.shadow_dx = 1;
    font.style.shadow_dy = 2;
    font.style.shadow_color = rgba(shadow[0], shadow[1], shadow[2], shadow[3]);
    font.style.clip = draw.clip;
    (font.style.spacing_x, font.style.spacing_y) = draw.spacing;
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
    /// `MsgWin_SetSpacing`.
    spacing: (i32, i32),
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
            spacing: (0, 0),
        })
    }

    pub fn open(&mut self) {
        let label = if self.right { "fl_r_in" } else { "fl_l_in" };
        self.flash.goto_label(label, true);
    }

    /// `MsgWin_SetSide`: the window's side for the next open / close (right: `fl_r_in` / `fl_r_out`).
    pub fn set_side(&mut self, right: bool) {
        self.right = right;
    }

    /// `MsgWin_SetText`.
    pub fn set_text(&mut self, text: Vec<u8>) {
        self.text = text;
    }

    /// `MsgWin_SetSpacing`.
    pub fn set_spacing(&mut self, x: i32, y: i32) {
        self.spacing = (x, y);
    }

    pub fn close(&mut self) {
        let label = if self.right { "fl_r_out" } else { "fl_l_out" };
        self.flash.goto_label(label, true);
    }

    /// `MsgWin_Draw(_, _, line)`.
    pub fn draw(&mut self, line: i32, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        self.flash.advance();
        let clip = self.flash.find(None, "mc_message_text");
        self.draw = TextBox::preset(5)
            .with_spacing(self.spacing.0, self.spacing.1)
            .attach_line(&mut self.flash, clip, 0, &self.text, line, font);
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        let (flash_quads, overs) = self.flash.draw_with_overs(&has_texture);
        quads.extend(flash_quads.iter().map(|q| screen_quad(q, textures)));
        draw_texts(self.draw.as_slice(), &overs, font);
        font.flush(quads);
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

    /// Replaces the texture of one image (`IconWin_SetIcon` swaps image 4).
    pub fn set_texture(&mut self, image: usize, texture: Option<usize>) {
        if let Some(slot) = self.textures.get_mut(image) {
            *slot = texture;
        }
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
