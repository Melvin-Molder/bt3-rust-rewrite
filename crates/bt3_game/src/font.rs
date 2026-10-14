//! The text printer (src/sys/font.c): commands queued with a snapshot of the style, drawn at flush; with the icon
//! file bound (`FontIcon_Init`), 16-bit strings may carry the inline tags `<PAD=…>`, `<PADS=a,b,c,d>`, `<COL=…>` and
//! `<UB0>` (controller button icons and colour changes).

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
    /// `color2`: the colour the icons are drawn with.
    pub color2: u32,
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
            color2: 0x8080_8080,
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
    color2: u32,
    /// `tag`: the icon tag callback was installed when the command was queued.
    tags: bool,
}

/// `FontIconDef`: an icon picture.
#[derive(Clone, Copy, Debug, Default)]
pub struct IconDef {
    /// Its texture in the icon file.
    pub tex: u8,
    /// Cell size in texels (0x20, 0x40 or 0x80).
    pub size: u8,
    /// Width in pixels at scale 1.
    pub advance: u8,
    pub frame_count: u16,
    /// Ticks each frame is shown.
    pub frame_time: [i8; 6],
}

/// `FontIconEntry`: a name of a controller type's icon table.
#[derive(Clone, Debug)]
pub struct IconEntry {
    pub name: Vec<u16>,
    pub icon: usize,
    pub scale: f32,
}

/// The inline tags of `gFontTags`, in their matching order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagKind {
    PadSequence,
    Pad,
    Color,
    Ub0,
}

/// What `FontIcon_Init` binds: the tag table, the icon definitions, the four controller types' tables and the
/// icon file's textures (battle texture index and TEX0 width).
#[derive(Clone, Debug, Default)]
pub struct Icons {
    pub tags: Vec<(Vec<u16>, TagKind, bool)>,
    pub defs: Vec<IconDef>,
    pub tables: [Vec<IconEntry>; 4],
    pub textures: Vec<(Option<usize>, i32)>,
}

/// `gFontTags`, `gFontIconTables` and `gFontIconDefs` in the executable, and the tag handlers by address.
const FONT_TAGS: u32 = 0x2C_6450;
const ICON_TABLES: u32 = 0x2C_6B18;
const ICON_DEFS: u32 = 0x2C_6838;
const TAG_PAD: u32 = 0x23_CA90;
const TAG_PAD_SEQUENCE: u32 = 0x23_CCA8;
const TAG_COLOR: u32 = 0x23_CF48;
const TAG_UB0: u32 = 0x23_D078;

impl Icons {
    /// The tables of `FontIcon_Init` from the executable (the textures are attached by the caller).
    pub fn read(elf: &bt3_data::elf::Elf) -> std::io::Result<Self> {
        let u32_at = |addr: u32| -> std::io::Result<u32> {
            Ok(u32::from_le_bytes(elf.read(addr, 4)?.try_into().unwrap()))
        };
        let u16_at = |addr: u32| -> std::io::Result<u16> {
            Ok(u16::from_le_bytes(elf.read(addr, 2)?.try_into().unwrap()))
        };
        let chars = |addr: u32, len: usize| -> std::io::Result<Vec<u16>> {
            (0..len as u32).map(|k| u16_at(addr + k * 2)).collect()
        };
        let mut icons = Icons::default();
        for i in 0..4 {
            let def = FONT_TAGS + i * 0x10;
            let len = usize::from(u16_at(def + 4)?);
            let kind = match u32_at(def + 8)? {
                TAG_PAD => TagKind::Pad,
                TAG_PAD_SEQUENCE => TagKind::PadSequence,
                TAG_COLOR => TagKind::Color,
                TAG_UB0 => TagKind::Ub0,
                _ => continue,
            };
            icons
                .tags
                .push((chars(u32_at(def)?, len)?, kind, u32_at(def + 0xC)? == 1));
        }
        let mut max_icon = 0;
        for (t, table) in icons.tables.iter_mut().enumerate() {
            let mut e = u32_at(ICON_TABLES + t as u32 * 4)?;
            loop {
                let name = u32_at(e)?;
                if name == 0 {
                    break;
                }
                let len = u32_at(name + 8)? as usize;
                let icon = ((u32_at(e + 4)? - ICON_DEFS) / 0xC) as usize;
                max_icon = max_icon.max(icon + 1);
                table.push(IconEntry {
                    name: chars(name, len.min(4))?,
                    icon,
                    scale: f32::from_bits(u32_at(e + 8)?),
                });
                e += 0xC;
            }
        }
        for i in 0..max_icon as u32 {
            let b = elf.read(ICON_DEFS + i * 0xC, 0xC)?;
            icons.defs.push(IconDef {
                tex: b[0],
                size: b[1],
                advance: b[2],
                frame_count: u16::from_le_bytes([b[4], b[5]]),
                frame_time: std::array::from_fn(|k| b[6 + k] as i8),
            });
        }
        Ok(icons)
    }
}

/// The printer's state: the two fonts of the boot file, the style, its stack and the queue.
pub struct Font {
    slots: Vec<FontSlot>,
    /// `gFontLineHeight`: the font bound last.
    line_height: i32,
    pub style: Style,
    stack: Vec<Style>,
    cmds: Vec<Cmd>,
    pub icons: Option<Icons>,
    /// `gFontIconPadType`: the row of the icon tables in use.
    pub pad_type: usize,
    /// `gFontIconFrame` / `gFontIconTick`: the icons' animation clock.
    icon_frame: i32,
    icon_tick: i32,
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
            icons: None,
            pad_type: 0,
            icon_frame: 0,
            icon_tick: 0,
        }
    }

    /// `FontIcon_ResetAnim`.
    pub fn reset_icon_anim(&mut self) {
        self.icon_frame = 0;
    }

    /// `FontIcon_Tick(vsyncs)`: the icon clock runs at 30 Hz (every second call at one vsync a frame).
    pub fn tick_icons(&mut self, vsyncs: i32) {
        if vsyncs == 1 {
            if self.icon_tick & 1 == 0 {
                self.icon_frame += 1;
            }
            self.icon_tick += 1;
        } else {
            self.icon_frame += 1;
        }
    }

    /// `Font_MeasureWidthEx(str, STR16, scale)`: the widest line (tags are measured as plain characters).
    pub fn width_scaled(&self, text: &[u16], scale: f32) -> i32 {
        let style = &self.style;
        let gap = style.spacing_x;
        let (mut max, mut width) = (0, 0);
        for &ch in text {
            if ch == 0 {
                break;
            }
            if is_newline(ch) {
                if style.flags & 1 != 0 {
                    break;
                }
                width = 0;
            } else if let Some((_, glyph)) = self.find_glyph(ch) {
                width += (glyph.width() as f32 * style.scale_x * scale * style.scale) as i32 + gap;
                max = max.max(width);
            }
        }
        max - gap
    }

    /// `Font_FitScale`: the largest of 1.0, 0.99, … (down to 0.01) at which the text is narrower than `width`.
    pub fn fit_scale(&self, text: &[u16], width: i32) -> f32 {
        let step = 0.01f32;
        let mut scale = 1.0f32;
        while step < scale {
            if self.width_scaled(text, scale) < width {
                break;
            }
            scale -= step;
        }
        scale
    }

    /// `FontIcon_GetPadTagSize`: the largest icon named in a string that begins with `<PAD=`.
    pub fn pad_tag_size(&self, text: &[u16]) -> i32 {
        let Some(icons) = &self.icons else {
            return 0;
        };
        let mut i = 5.min(text.len());
        let arg = copy_arg(text, &mut i, 0x1F, u16::from(b'>'));
        icons.max_size(&arg, self.pad_type)
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

    /// `Font_GetHeight`: the lines at the style's scale and line spacing.
    pub fn height(&self, text: &[u16]) -> i32 {
        let st = &self.style;
        Self::count_lines(text)
            * ((self.line_height as f32 * st.scale_y * st.scale) as i32 + st.spacing_y)
            - st.spacing_y
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
            color2: s.color2,
            tags: self.icons.is_some(),
        });
    }

    /// `Font_FlushAll`: draws and empties the queue.
    pub fn flush(&mut self, quads: &mut Vec<ScreenQuad>) {
        for cmd in std::mem::take(&mut self.cmds) {
            self.draw_cmd(&cmd, quads);
        }
    }

    /// `Font_DrawCmd` for a 16-bit string: the tag callback in front of every character.
    fn draw_cmd(&self, cmd: &Cmd, quads: &mut Vec<ScreenQuad>) {
        let text = &cmd.text;
        let saved = cmd.color;
        let mut color = cmd.color;
        let (mut x, mut y) = (self.align_line(cmd, cmd.x, text), cmd.y);
        let mut i = 0;
        loop {
            if cmd.tags {
                let mut at = Pen {
                    x: &mut x,
                    y: &mut y,
                    color: &mut color,
                };
                self.handle_tag(cmd, saved, &mut at, text, &mut i, quads);
            }
            let ch = text.get(i).copied().unwrap_or(0);
            i += 1;
            if ch == 0 {
                break;
            }
            if is_newline(ch) {
                x = self.align_line(cmd, cmd.home_x, text.get(i..).unwrap_or_default());
                y += ((self.line_height + cmd.spacing_y) as f32 * cmd.scale_y) as i32;
                if cmd.flags & 1 != 0 {
                    break;
                }
            } else {
                x += self.draw_glyph(cmd, color, x, y, ch, quads);
            }
        }
    }

    /// `FontTag_Handle`: at a `<` that starts a known tag, runs it and steps past its `>`, then looks again.
    fn handle_tag(
        &self,
        cmd: &Cmd,
        saved: u32,
        at: &mut Pen,
        text: &[u16],
        i: &mut usize,
        quads: &mut Vec<ScreenQuad>,
    ) {
        let Some(icons) = &self.icons else {
            return;
        };
        if text.get(*i) != Some(&u16::from(b'<')) {
            return;
        }
        for (name, kind, has_arg) in &icons.tags {
            if !tag_match(text.get(*i + 1..).unwrap_or_default(), name) {
                continue;
            }
            let ch = text.get(*i + 1 + name.len()).copied().unwrap_or(0);
            let want = if *has_arg { b'=' } else { b'>' };
            if ch != u16::from(want) {
                break;
            }
            let len = name.len();
            match kind {
                TagKind::Pad => {
                    let mut p = *i + len + 2;
                    let arg = copy_arg(text, &mut p, 0x1F, u16::from(b'>'));
                    self.draw_icons(icons, cmd, &arg, false, at, quads);
                    *i = p;
                }
                TagKind::PadSequence => {
                    let mut p = *i + len + 2;
                    let mut args: [Vec<u16>; 4] = Default::default();
                    for (k, arg) in args.iter_mut().enumerate() {
                        let stop = if k == 3 { b'>' } else { b',' };
                        *arg = copy_arg(text, &mut p, 0x1F, u16::from(stop));
                        if k < 3 {
                            p += 1;
                        }
                    }
                    let arg = args[self.pad_type.min(3)].clone();
                    self.draw_icons(icons, cmd, &arg, true, at, quads);
                    *i = p;
                }
                TagKind::Color => {
                    // `<COL=DEF>` restores the command's colour; `<COL=AARRGGBB>` (capital hex) sets one; a bad
                    // digit leaves the tag to be printed.
                    let mut p = *i + len + 2;
                    let color = if tag_match(text.get(p..).unwrap_or_default(), &[0x44, 0x45, 0x46])
                    {
                        p += 3;
                        saved
                    } else {
                        let mut hex = 0u32;
                        for k in (0..8).rev() {
                            let ch = text.get(p).copied().unwrap_or(0);
                            p += 1;
                            let digit = match ch {
                                0x30..=0x39 => u32::from(ch - 0x30),
                                0x41..=0x46 => u32::from(ch - 0x41 + 10),
                                _ => return,
                            };
                            hex |= digit << (k * 4);
                        }
                        ((hex >> 16) & 0xFF)
                            | ((hex & 0xFF) << 16)
                            | ((hex >> 24) << 24)
                            | (hex & 0xFF00)
                    };
                    *at.color = color;
                    *i = p;
                }
                TagKind::Ub0 => {
                    // Font_DrawCharNow(0xFF10).
                    *at.x += self.draw_glyph(cmd, *at.color, *at.x, *at.y, 0xFF10, quads);
                    *i += len + 1;
                }
            }
            if text.get(*i) != Some(&u16::from(b'>')) {
                break;
            }
            *i += 1;
            self.handle_tag(cmd, saved, at, text, i, quads);
            break;
        }
    }

    /// `FontTag_Pad` / `FontTag_PadSequence`: the named icons side by side, each on its own animation or (a
    /// sequence) one after another; they sit centred on the largest one.
    fn draw_icons(
        &self,
        icons: &Icons,
        cmd: &Cmd,
        arg: &[u16],
        sequence: bool,
        at: &mut Pen,
        quads: &mut Vec<ScreenQuad>,
    ) {
        let pad = self.pad_type.min(3);
        let total = icons.total_length(arg, pad);
        let max = icons.max_size(arg, pad);
        let mut start = 0;
        let mut s = 0;
        while arg.get(s).is_some_and(|&c| c != 0) {
            let Some(entry) = icons.find(&arg[s..], pad) else {
                break;
            };
            let icon = icons.defs[entry.icon];
            let length = anim_length(&icon);
            let frame = if sequence {
                let f = self.icon_frame(&icon, total, start);
                start += length;
                f
            } else {
                self.icon_frame(&icon, length, 0)
            };
            let size = i32::from(icon.size);
            let dy = match size {
                0x20 => (max - 0x20) / 2 - 6,
                0x40 => (max - 0x40) / 2,
                0x80 => (max - 0x80) / 2 - 6,
                _ => (max - size) / 2 - 6,
            };
            self.icon_sprite(
                icons,
                cmd,
                &icon,
                frame,
                [*at.x, *at.y + dy],
                entry.scale,
                quads,
            );
            *at.x += (f32::from(icon.advance) * entry.scale) as i32 + cmd.spacing_x;
            s += entry.name.len();
        }
    }

    /// `FontIcon_GetFrame`: the clock modulo the length, minus the start, walked through the frame times.
    fn icon_frame(&self, icon: &IconDef, length: i32, start: i32) -> i32 {
        let mut frame = 0;
        if icon.frame_count >= 2 {
            // A zero length does not occur (the original would divide by zero).
            let t = self.icon_frame.checked_rem(length).unwrap_or(0) - start;
            if t > 0 {
                let n = i32::from(icon.frame_count).min(6);
                frame = n - 1;
                let mut sum = 0;
                for (k, &time) in icon.frame_time.iter().take(n as usize).enumerate() {
                    if t < sum + i32::from(time) {
                        frame = k as i32;
                        break;
                    }
                    sum += i32::from(time);
                }
            }
        }
        frame
    }

    /// `FontIcon_PutSprite`: a cell of the icon's texture (cells per row = texture width / cell size), drawn at
    /// (int)(cell size * scale) with `color2`, unless it leaves the drawing area; the command's scissor cuts it.
    #[allow(clippy::too_many_arguments)]
    fn icon_sprite(
        &self,
        icons: &Icons,
        cmd: &Cmd,
        icon: &IconDef,
        frame: i32,
        at: [i32; 2],
        scale: f32,
        quads: &mut Vec<ScreenQuad>,
    ) {
        let Some(&(texture, tex_width)) = icons.textures.get(usize::from(icon.tex)) else {
            return;
        };
        let [x, y] = at;
        let size = i32::from(icon.size).max(1);
        let drawn = (size as f32 * scale) as i32;
        let per_row = (tex_width / size).max(1);
        let (gx0, gy0, gx1, gy1) = (x + 0x700, y + 0x720, x + drawn + 0x700, y + drawn + 0x720);
        if !(gx0 > 0 && gx1 < 0x1000 && gy0 > 0 && gy1 < 0x1000) {
            return;
        }
        let (col, row) = (frame % per_row, frame / per_row);
        let [u0, v0, u1, v1] =
            [col * size, row * size, (col + 1) * size, (row + 1) * size].map(|v| v as f32);
        let [x0, y0, x1, y1] = [x, y, x + drawn, y + drawn].map(|v| v as f32);
        let quad = ScreenQuad {
            texture,
            pt: [[x0, y0], [x1, y0], [x0, y1], [x1, y1]],
            uv: [[u0, v0], [u1, v0], [u0, v1], [u1, v1]],
            rgba: cmd.color2.to_le_bytes(),
        };
        quads.extend(scissor(quad, cmd.clip));
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
    fn draw_glyph(
        &self,
        cmd: &Cmd,
        color: u32,
        x: i32,
        y: i32,
        ch: u16,
        quads: &mut Vec<ScreenQuad>,
    ) -> i32 {
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
            // Font_IsInClip, then the command's scissor (SCISSOR_1 = clip - 1, inclusive).
            if c[0] < rect[2] && rect[0] < c[2] && c[1] < rect[3] && rect[1] < c[3] {
                quads.extend(scissor(sprite(rect, glyph, color, texture), c));
            }
        };
        for &(ox0, oy0, ox1, oy1) in offsets {
            put([x + ox0, y + oy0, x1 + ox1, y1 + oy1], cmd.shadow_color);
        }
        put([x, y, x1, y1], color);
        (w as f32 * cmd.scale_x) as i32 + cmd.spacing_x
    }
}

/// Where a command is drawing and with what colour (`<COL=…>` changes it).
struct Pen<'a> {
    x: &'a mut i32,
    y: &'a mut i32,
    color: &'a mut u32,
}

impl Icons {
    /// `FontIcon_Find`: the icon whose name starts `text`, the longest name winning.
    fn find(&self, text: &[u16], pad: usize) -> Option<&IconEntry> {
        let mut best: Option<&IconEntry> = None;
        for e in &self.tables[pad] {
            if tag_match(text, &e.name) && best.is_none_or(|b| b.name.len() < e.name.len()) {
                best = Some(e);
            }
        }
        best
    }

    /// `FontIcon_GetMaxSize`.
    fn max_size(&self, arg: &[u16], pad: usize) -> i32 {
        let (mut max, mut s) = (0, 0);
        while arg.get(s).is_some_and(|&c| c != 0) {
            let Some(e) = self.find(&arg[s..], pad) else {
                break;
            };
            max = max.max(i32::from(self.defs[e.icon].size));
            s += e.name.len();
        }
        max
    }

    /// `FontIcon_GetTotalLength`.
    fn total_length(&self, arg: &[u16], pad: usize) -> i32 {
        let (mut total, mut s) = (0, 0);
        while arg.get(s).is_some_and(|&c| c != 0) {
            let Some(e) = self.find(&arg[s..], pad) else {
                break;
            };
            total += anim_length(&self.defs[e.icon]);
            s += e.name.len();
        }
        total
    }
}

/// `FontIcon_GetAnimLength`.
fn anim_length(icon: &IconDef) -> i32 {
    icon.frame_time
        .iter()
        .take(usize::from(icon.frame_count).min(6))
        .map(|&t| i32::from(t))
        .sum()
}

/// `FontTag_Match`: the first characters of `text` are `name` (the text's end fails).
fn tag_match(text: &[u16], name: &[u16]) -> bool {
    name.iter()
        .enumerate()
        .all(|(k, &c)| text.get(k).is_some_and(|&t| t != 0 && t == c))
}

/// `FontTag_CopyArg`: the characters up to `stop` (at most `max`), leaving `*p` on it.
fn copy_arg(text: &[u16], p: &mut usize, max: usize, stop: u16) -> Vec<u16> {
    let mut out = Vec::new();
    for _ in 0..max {
        match text.get(*p) {
            Some(&c) if c != stop => {
                out.push(c);
                *p += 1;
            }
            _ => break,
        }
    }
    out
}

/// The GS scissor over an axis-aligned sprite: cut to pixels `clip[0]..clip[2]` x `clip[1]..clip[3]`, the texture
/// coordinates following.
pub fn scissor(q: ScreenQuad, clip: [i32; 4]) -> Option<ScreenQuad> {
    let ([x0, y0], [x1, y1]) = (q.pt[0], q.pt[3]);
    let ([u0, v0], [u1, v1]) = (q.uv[0], q.uv[3]);
    let [cx0, cy0, cx1, cy1] = clip.map(|v| v as f32);
    let (nx0, nx1) = (x0.max(cx0), x1.min(cx1));
    let (ny0, ny1) = (y0.max(cy0), y1.min(cy1));
    if nx0 >= nx1 || ny0 >= ny1 {
        return None;
    }
    if (nx0, nx1, ny0, ny1) == (x0, x1, y0, y1) {
        return Some(q);
    }
    let u = |x: f32| u0 + (x - x0) / (x1 - x0) * (u1 - u0);
    let v = |y: f32| v0 + (y - y0) / (y1 - y0) * (v1 - v0);
    let (a0, a1, b0, b1) = (u(nx0), u(nx1), v(ny0), v(ny1));
    Some(ScreenQuad {
        pt: [[nx0, ny0], [nx1, ny0], [nx0, ny1], [nx1, ny1]],
        uv: [[a0, b0], [a1, b0], [a0, b1], [a1, b1]],
        ..q
    })
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

    fn utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// The circle button: a 0x20 cell, 20 pixels wide, two frames of 3 ticks.
    fn icons() -> super::Icons {
        use super::{IconDef, IconEntry, Icons, TagKind};
        let mut icons = Icons {
            tags: vec![
                (utf16("PADS"), TagKind::PadSequence, true),
                (utf16("PAD"), TagKind::Pad, true),
                (utf16("COL"), TagKind::Color, true),
            ],
            defs: vec![IconDef {
                tex: 0,
                size: 0x20,
                advance: 20,
                frame_count: 2,
                frame_time: [3, 3, 0, 0, 0, 0],
            }],
            textures: vec![(Some(9), 64)],
            ..Icons::default()
        };
        icons.tables[0].push(IconEntry {
            name: vec![0x25CB],
            icon: 0,
            scale: 1.0,
        });
        icons
    }

    #[test]
    fn a_pad_tag_draws_its_icon_and_moves_the_text_on() {
        let mut font = font();
        font.icons = Some(icons());
        font.print_at(100, 50, &utf16("<PAD=\u{25CB}>A"));
        let mut quads = Vec::new();
        font.flush(&mut quads);
        assert_eq!(quads.len(), 2);
        // The icon: frame 0 of texture 9, centred on itself (dy -6), in colour 2; then the glyph 20 further on.
        assert_eq!(quads[0].texture, Some(9));
        assert_eq!(
            (quads[0].pt[0], quads[0].pt[3]),
            ([100.0, 44.0], [132.0, 76.0])
        );
        assert_eq!(quads[0].uv[3], [32.0, 32.0]);
        assert_eq!(quads[0].rgba, [0x80; 4]);
        assert_eq!(quads[1].pt[0], [120.0, 50.0]);
        // Four ticks later the icon shows its second frame.
        for _ in 0..4 {
            font.tick_icons(2);
        }
        font.print_at(100, 50, &utf16("<PAD=\u{25CB}>"));
        quads.clear();
        font.flush(&mut quads);
        assert_eq!(quads[0].uv[0], [32.0, 0.0]);
    }

    #[test]
    fn real_icon_tables_and_textures() {
        let path = bt3_data::rom::for_tests();
        let Ok(mut image) = bt3_data::Iso9660::open(&path) else {
            return;
        };
        let cnf = image.read_file("SYSTEM.CNF").unwrap();
        let exe = bt3_data::elf::boot_path(&cnf).unwrap();
        let elf = bt3_data::elf::Elf::parse(image.read_file(&exe).unwrap()).unwrap();
        let icons = super::Icons::read(&elf).unwrap();
        let tags: Vec<String> = icons
            .tags
            .iter()
            .map(|(n, _, _)| String::from_utf16_lossy(n))
            .collect();
        assert_eq!(tags, ["PADS", "PAD", "COL", "UB0"]);
        // Only the first controller type has icons: 28 names, one picture each.
        assert_eq!(
            icons.tables.iter().map(Vec::len).collect::<Vec<_>>(),
            [28, 0, 0, 0]
        );
        assert_eq!(icons.defs.len(), 28);
        // The icon file (boot file 5, word 8) has a texture for every picture.
        let location = bt3_data::afs::partition_entry_for_id(5);
        let archive = bt3_data::afs::AfsArchive::open(
            &mut image,
            format!("DATA/PZS3US{}.AFS", location.partition),
        )
        .unwrap();
        let boot = archive.read_entry(&mut image, location.index).unwrap();
        let start = u32::from_le_bytes(boot[32..36].try_into().unwrap()) as usize & !3;
        let res = bt3_data::tex::TextureResource::parse(&boot[start..]).unwrap();
        let used = icons.defs.iter().map(|d| usize::from(d.tex)).max().unwrap();
        assert!(used < res.entries.len(), "{used} >= {}", res.entries.len());
    }

    #[test]
    fn colour_tags_and_scissor() {
        let mut font = font();
        font.icons = Some(icons());
        font.style.clip = [0, 0, 105, 0x1BF];
        // FontTag_Color reads the digits as AARRGGBB (its comment says RRGGBBAA; the code wins).
        font.print_at(100, 50, &utf16("<COL=80FF0000>A<COL=DEF>A"));
        let mut quads = Vec::new();
        font.flush(&mut quads);
        // The first glyph is red and cut at x 105 (half its texels); the second starts past the clip.
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].rgba, [0xFF, 0, 0, 0x80]);
        assert_eq!((quads[0].pt[0][0], quads[0].pt[1][0]), (100.0, 105.0));
        assert_eq!(quads[0].uv[1][0] - quads[0].uv[0][0], 5.0);
    }
}
