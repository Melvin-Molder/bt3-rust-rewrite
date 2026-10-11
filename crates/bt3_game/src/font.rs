//! The text printer (src/sys/font.c): commands queued with a snapshot of the style, drawn at flush.

use bt3_data::font::{FontFile, Glyph};

use crate::draw::ScreenQuad;

/// `FontStyle`, with the values `Font_InitEx` gives it.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub scale_x: f32,
    pub scale_y: f32,
    pub scale: f32,
    pub color: u32,
    pub spacing_x: i32,
    pub spacing_y: i32,
    pub align: i32,
    pub flags: i32,
    pub shadow_mode: i32,
    pub shadow_color: u32,
    pub shadow_dx: i32,
    pub shadow_dy: i32,
    pub clip: [i32; 4],
}

impl Default for Style {
    fn default() -> Self {
        Self {
            scale_x: 1.0,
            scale_y: 1.0,
            scale: 1.0,
            color: 0x80FF_FFFF,
            spacing_x: 0,
            spacing_y: 0,
            align: 0,
            flags: 0,
            shadow_mode: 0,
            shadow_color: 0x8080_8080,
            shadow_dx: 1,
            shadow_dy: 2,
            clip: [0, 0, 0x1FF, 0x1BF],
        }
    }
}

/// `Font_SetColorRGBA`.
pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16 | u32::from(a) << 24
}

/// A bound font (`FontSlot`): its glyphs and one texture per CLUT number.
pub struct FontSlot {
    pub glyphs: Vec<Glyph>,
    pub textures: Vec<usize>,
}

impl FontSlot {
    pub fn new(file: &FontFile<'_>, textures: Vec<usize>) -> Self {
        Self {
            glyphs: file.glyphs.clone(),
            textures,
        }
    }
}

/// A queued string (`FontCmd` of kind `FONT_CMD_STR16`).
struct Cmd {
    text: Vec<u16>,
    x: i32,
    y: i32,
    home_x: i32,
    scale_x: f32,
    scale_y: f32,
    spacing_x: i32,
    spacing_y: i32,
    color: u32,
    align: i32,
    flags: i32,
    clip: [i32; 4],
    shadow_mode: i32,
    shadow_color: u32,
    shadow_dx: i32,
    shadow_dy: i32,
}

/// The printer's state: the two fonts of the boot file, the style, its stack and the queue.
pub struct Font {
    slots: Vec<FontSlot>,
    /// `gFontLineHeight`: the font bound last.
    line_height: i32,
    pub style: Style,
    stack: Vec<Style>,
    cmds: Vec<Cmd>,
}

impl Font {
    /// `Font_InitEx`: binds the two fonts of the boot file (slot 1 sets the line height).
    pub fn new(slots: Vec<FontSlot>, line_height: i32) -> Self {
        Self {
            slots,
            line_height,
            style: Style::default(),
            stack: Vec::new(),
            cmds: Vec::new(),
        }
    }

    /// `Font_GetGlyphHeight`: the line height of the font bound last.
    pub fn line_height(&self) -> i32 {
        self.line_height
    }

    pub fn push_style(&mut self) {
        self.stack.push(self.style);
    }

    pub fn pop_style(&mut self) {
        if let Some(style) = self.stack.pop() {
            self.style = style;
        }
    }

    /// `Font_FindGlyph`: slot 0 first; a no-break space falls back to a space.
    fn find_glyph(&self, ch: u16) -> Option<(&FontSlot, Glyph)> {
        for slot in &self.slots {
            if let Ok(i) = slot.glyphs.binary_search_by_key(&ch, |g| g.code) {
                return Some((slot, slot.glyphs[i]));
            }
        }
        if ch == 0xA0 {
            return self.find_glyph(u16::from(b' '));
        }
        None
    }

    /// `Font_CountLines`.
    pub fn count_lines(text: &[u16]) -> i32 {
        1 + text.iter().filter(|&&c| is_newline(c)).count() as i32
    }

    /// `Font_GetWidth`: the widest line at scale 1.
    pub fn width(&self, text: &[u16]) -> i32 {
        let style = &self.style;
        let gap = style.spacing_x;
        let (mut max, mut width) = (0, 0);
        for &ch in text {
            if is_newline(ch) {
                if style.flags & 1 != 0 {
                    break;
                }
                width = 0;
            } else if let Some((_, glyph)) = self.find_glyph(ch) {
                width += (glyph.width() as f32 * style.scale_x * style.scale) as i32 + gap;
                max = max.max(width);
            }
        }
        max - gap
    }

    /// `Font_PrintAt`.
    pub fn print_at(&mut self, x: i32, y: i32, text: &[u16]) {
        let s = &self.style;
        self.cmds.push(Cmd {
            text: text.to_vec(),
            x,
            y,
            home_x: x,
            scale_x: s.scale_x * s.scale,
            scale_y: s.scale_y * s.scale,
            spacing_x: s.spacing_x,
            spacing_y: s.spacing_y,
            color: s.color,
            align: s.align,
            flags: s.flags,
            clip: s.clip,
            shadow_mode: s.shadow_mode,
            shadow_color: s.shadow_color,
            shadow_dx: s.shadow_dx,
            shadow_dy: s.shadow_dy,
        });
    }

    /// `Font_FlushAll`: draws and empties the queue.
    pub fn flush(&mut self, quads: &mut Vec<ScreenQuad>) {
        for cmd in std::mem::take(&mut self.cmds) {
            self.draw_cmd(&cmd, quads);
        }
    }

    /// `Font_DrawCmd` for a 16-bit string.
    fn draw_cmd(&self, cmd: &Cmd, quads: &mut Vec<ScreenQuad>) {
        let (mut x, mut y) = (self.align_line(cmd, cmd.x, &cmd.text), cmd.y);
        for (i, &ch) in cmd.text.iter().enumerate() {
            if is_newline(ch) {
                x = self.align_line(cmd, cmd.home_x, &cmd.text[i + 1..]);
                y += ((self.line_height + cmd.spacing_y) as f32 * cmd.scale_y) as i32;
                if cmd.flags & 1 != 0 {
                    break;
                }
            } else {
                x += self.draw_glyph(cmd, x, y, ch, quads);
            }
        }
    }

    /// `Font_AlignLine`.
    fn align_line(&self, cmd: &Cmd, x: i32, text: &[u16]) -> i32 {
        match cmd.align {
            1 => x - self.measure_line(cmd, text) / 2,
            2 => x - self.measure_line(cmd, text),
            _ => x,
        }
    }

    /// `Font_MeasureLine`: the advances up to the first line break.
    fn measure_line(&self, cmd: &Cmd, text: &[u16]) -> i32 {
        text.iter()
            .take_while(|&&c| !is_newline(c))
            .filter_map(|&c| self.find_glyph(c))
            .map(|(_, g)| (g.width() as f32 * cmd.scale_x) as i32 + cmd.spacing_x)
            .sum()
    }

    /// `Font_DrawGlyph`: the shadow or outline sprites, then the glyph; returns the advance.
    fn draw_glyph(&self, cmd: &Cmd, x: i32, y: i32, ch: u16, quads: &mut Vec<ScreenQuad>) -> i32 {
        let Some((slot, glyph)) = self.find_glyph(ch) else {
            return 0;
        };
        let (w, h) = (glyph.width(), glyph.height());
        let x1 = x + (w as f32 * cmd.scale_x) as i32;
        let y1 = y + (h as f32 * cmd.scale_y) as i32;
        let texture = slot.textures.get(glyph.clut()).copied();
        let (dx, dy) = (cmd.shadow_dx, cmd.shadow_dy);
        let offsets: &[(i32, i32, i32, i32)] = match cmd.shadow_mode {
            1 => &[(dx, dy, dx, dy)],
            2 => &[
                (dx, dy, dx, dy),
                (-dx, dy, -dx, dy),
                (dx, -dy, dx, -dy),
                (-dx, -dy, -dx, -dy),
                (dx, 0, dx, 0),
                (-dx, 0, -dx, 0),
                (0, dy, 0, dy),
                (0, -dy, 0, -dy),
            ],
            3 => &[
                (-dx, -dy, -dx, dy),
                (dx, -dy, dx, dy),
                (-dx, -dy, dx, -dy),
                (-dx, dy, dx, dy),
            ],
            _ => &[],
        };
        let mut put = |rect: [i32; 4], color: u32| {
            let c = cmd.clip;
            if c[0] < rect[2] && rect[0] < c[2] && c[1] < rect[3] && rect[1] < c[3] {
                quads.push(sprite(rect, glyph, color, texture));
            }
        };
        for &(ox0, oy0, ox1, oy1) in offsets {
            put([x + ox0, y + oy0, x1 + ox1, y1 + oy1], cmd.shadow_color);
        }
        put([x, y, x1, y1], cmd.color);
        (w as f32 * cmd.scale_x) as i32 + cmd.spacing_x
    }
}

fn is_newline(ch: u16) -> bool {
    ch == u16::from(b'\n') || ch == u16::from(b'\r')
}

/// `Font_PutSprite`: a GS sprite with point-sampled texels `u0..u0 + w` (the half-texel offset puts texel u0 on
/// pixel x0).
fn sprite(rect: [i32; 4], glyph: Glyph, color: u32, texture: Option<usize>) -> ScreenQuad {
    let [x0, y0, x1, y1] = rect.map(|v| v as f32);
    let (u0, v0) = glyph.texel();
    let (u1, v1) = (u0 + glyph.width(), v0 + glyph.height());
    let [u0, v0, u1, v1] = [u0, v0, u1, v1].map(|v| v as f32);
    ScreenQuad {
        texture,
        pt: [[x0, y0], [x1, y0], [x0, y1], [x1, y1]],
        uv: [[u0, v0], [u1, v0], [u0, v1], [u1, v1]],
        rgba: color.to_le_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Font, FontSlot, rgba};
    use bt3_data::font::Glyph;

    fn font() -> Font {
        let glyphs = vec![
            Glyph {
                code: 0x41,
                u: 10 << 10,
                v: 17 << 10,
            },
            Glyph {
                code: 0x42,
                u: 6 << 10,
                v: 17 << 10,
            },
        ];
        Font::new(
            vec![FontSlot {
                glyphs,
                textures: vec![7, 8],
            }],
            20,
        )
    }

    #[test]
    fn centred_lines_and_outline() {
        let mut font = font();
        font.style.align = 1;
        font.style.shadow_mode = 2;
        font.print_at(100, 50, &[0x41, 0x42, 0x0A, 0x42]);
        let mut quads = Vec::new();
        font.flush(&mut quads);
        // Nine sprites per glyph; line 1 is 16 wide, line 2 is 6.
        assert_eq!(quads.len(), 27);
        assert_eq!(quads[8].pt[0], [92.0, 50.0]);
        assert_eq!(quads[17].pt[0], [102.0, 50.0]);
        assert_eq!(quads[26].pt[0], [97.0, 70.0]);
        assert_eq!(quads[0].pt[0], [93.0, 52.0]);
        assert_eq!(quads[8].texture, Some(7));
    }

    #[test]
    fn width_and_colour() {
        let font = font();
        assert_eq!(font.width(&[0x41, 0x42, 0x0A, 0x41]), 16);
        assert_eq!(rgba(0xFF, 0x10, 0x20, 0x40), 0x4020_10FF);
    }
}
