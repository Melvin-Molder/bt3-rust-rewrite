//! The yes / no message window (src/sys/dialog.c).

use std::error::Error;

use bt3_data::font::message;
use bt3_data::gs::{RgbaTexture, decode_sprite_texture};
use bt3_data::pack_section;
use bt3_data::tex::TextureResource;

use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quad};
use crate::font::{Font, rgba};

const FLAG_DECIDED: u32 = 1;
const FLAG_CLOSED: u32 = 2;
const FLAG_OPEN: u32 = 4;
const FLAG_CLOSING: u32 = 8;
const FLAG_CURSOR_ON: u32 = 0x10;
// Flash.flags: the movie hit a trigger "end" action this tick.
const FLASH_END: u32 = 8;

/// Pad bits `Dialog_Input` reads.
const PAD_LEFT: u32 = 0x0001;
const PAD_RIGHT: u32 = 0x0002;
const PAD_CROSS: u32 = 0x0200;
const PAD_TRIANGLE: u32 = 0x0400;

/// `DialogFlashRes`: (part, texture entry) per movie image. Image 0 is `tex0`, hidden by `Dialog_Draw(0)`.
const FLASH_TEXTURES: [Option<(usize, usize)>; 11] = [
    Some((1, 0)),
    Some((4, 0)),
    Some((4, 1)),
    None,
    None,
    None,
    Some((2, 0)),
    Some((3, 0)),
    Some((2, 1)),
    Some((3, 1)),
    None,
];

/// `Dialog_Start` commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    Close,
    #[allow(dead_code)]
    Show,
}

/// What `Dialog_Init` reads from its file: the window movie, its message table and the textures by image.
pub struct DialogAssets {
    pub fod: Vec<u8>,
    pub titles: Vec<u8>,
    pub textures: Vec<Option<RgbaTexture>>,
}

impl DialogAssets {
    pub fn parse(file: &[u8]) -> Result<Self, Box<dyn Error>> {
        let mut parts = std::collections::BTreeMap::new();
        let mut textures = Vec::new();
        for slot in FLASH_TEXTURES {
            textures.push(match slot {
                Some((part, entry)) => {
                    if !parts.contains_key(&part) {
                        let resource = TextureResource::parse(pack_section(file, part)?)?;
                        let decoded = resource
                            .entries
                            .iter()
                            .map(decode_sprite_texture)
                            .collect::<Result<Vec<_>, _>>()?;
                        parts.insert(part, decoded);
                    }
                    Some(
                        parts[&part]
                            .get(entry)
                            .cloned()
                            .ok_or_else(|| format!("dialog part {part} has no texture {entry}"))?,
                    )
                }
                None => None,
            });
        }
        Ok(Self {
            fod: pack_section(file, 5)?.to_vec(),
            titles: pack_section(file, 6)?.to_vec(),
            textures,
        })
    }
}

/// `Dialog` (`gDialog`).
pub struct Dialog {
    flash: Flash,
    /// Per movie image: the clamped texture (the wrapping one follows it).
    textures: Vec<Option<usize>>,
    titles: Vec<u8>,
    msg_table: Option<Vec<u8>>,
    flags: u32,
    cursor: i32,
    def_cursor: i32,
    msg: i32,
    title: i32,
    size: i32,
    layout: i32,
    choices: bool,
}

/// The pad bits of `gPad[port]` the window reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pad {
    pub pressed: u32,
    pub repeat: u32,
}

impl Dialog {
    /// `Dialog_Init(file, msgTbl, size)`.
    pub fn new(
        fod: &[u8],
        titles: Vec<u8>,
        textures: Vec<Option<usize>>,
        msg_table: Option<Vec<u8>>,
        size: i32,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(fod)?;
        flash.play(1);
        Ok(Self {
            flash,
            textures,
            titles,
            msg_table,
            flags: 0,
            cursor: 0,
            def_cursor: 0,
            msg: -1,
            title: -1,
            size,
            layout: 0,
            choices: false,
        })
    }

    /// `Dialog_PlayCursorPlate`.
    fn play_cursor_plate(&mut self, label: &str) {
        let clip = self
            .flash
            .find(None, &format!("mc_menu_plate_{}", self.cursor + 1));
        self.flash.clip_goto_label(clip, label);
    }

    /// `Dialog_Draw(visible)`: advances the window, prints its texts and draws both.
    pub fn draw(&mut self, visible: bool, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        self.flash.advance();
        if self.flags & FLAG_CLOSING != 0 && self.flash.flags() & FLASH_END != 0 {
            self.flags |= FLAG_CLOSED;
        }
        if self.flags & FLAG_CURSOR_ON == 0 && self.flash.flags() & FLASH_PAD != 0 {
            self.play_cursor_plate("fl_on_start");
            self.flags |= FLAG_CURSOR_ON;
        }
        for i in 0..2 {
            let name = format!("mc_menu_plate_{}", i + 1);
            let plate = self.flash.find(None, &name);
            if self.choices {
                self.flash.clip_set_flags(plate, PROP_VISIBLE, true);
                let uv = Uv {
                    u0: 0,
                    v0: i * 0x20,
                    u1: 0x80,
                    v1: i * 0x20 + 0x20,
                    tex: 0,
                };
                for child in ["mc_menu_text_off", "mc_menu_text_on"] {
                    let clip = self.flash.find(Some(&name), child);
                    self.flash.clip_set_uv(clip, uv);
                }
            } else {
                self.flash.clip_set_flags(plate, PROP_VISIBLE, false);
            }
        }
        match self.layout {
            0 => self.draw_text(font),
            1 => {
                self.draw_title(font);
                self.draw_body(font);
            }
            _ => {}
        }
        let textures = &self.textures;
        let has_texture = |image: usize| {
            (visible || image != 0) && textures.get(image).is_some_and(Option::is_some)
        };
        for quad in self.flash.draw(&has_texture) {
            quads.push(screen_quad(&quad, textures));
        }
        font.flush(quads);
    }

    /// The anchor clip's position and alpha.
    fn anchor(&self, name: &str) -> Option<(i32, i32, f32)> {
        let clip = self.flash.find(None, name);
        let (x, y) = self.flash.clip_get_pos(clip)?;
        Some((x, y, self.flash.clip_get_alpha(clip)))
    }

    /// `Dialog_DrawText`: layout 0, a text of the caller's table.
    fn draw_text(&self, font: &mut Font) {
        let Some(table) = &self.msg_table else { return };
        if self.msg < 0 {
            return;
        }
        let anchor = if self.size == 0 {
            "mc_dummy_text_1"
        } else {
            "mc_dummy_text_4"
        };
        let Some((x, mut y, alpha)) = self.anchor(anchor) else {
            return;
        };
        let Ok(text) = message(table, self.msg as usize) else {
            return;
        };
        y += match (self.size, Font::count_lines(&text)) {
            (0, 1) => 0x41,
            (0, 2) => 0x34,
            (0, 3) => 0x27,
            (0, 4) => 0x1A,
            (0, 5) => 0xD,
            (1, 6) => 0x1A,
            (1, 7) => 0xD,
            _ => 0,
        };
        print(font, x, y, alpha, &text);
    }

    /// `Dialog_DrawTitle`: layout 1, heading `title` of the file's own table.
    fn draw_title(&self, font: &mut Font) {
        if self.title < 0 {
            return;
        }
        let Some((x, y, alpha)) = self.anchor("mc_dummy_text_2") else {
            return;
        };
        if let Ok(text) = message(&self.titles, self.title as usize) {
            print(font, x, y, alpha, &text);
        }
    }

    /// `Dialog_DrawBody`: layout 1, text `msg` of the file's own table.
    fn draw_body(&self, font: &mut Font) {
        if self.msg < 0 {
            return;
        }
        let Some((x, mut y, alpha)) = self.anchor("mc_dummy_text_3") else {
            return;
        };
        let Ok(text) = message(&self.titles, self.msg as usize) else {
            return;
        };
        y += match Font::count_lines(&text) {
            1 => 0x28,
            2 => 0x20,
            3 => 0x14,
            4 => 0xA,
            5 => 4,
            _ => 0,
        };
        print(font, x, y, alpha, &text);
    }

    /// `Dialog_Start`; open and close play SE 4 / 5 of bank 1.
    pub fn start(&mut self, command: Command, sounds: &mut Vec<(u32, u32)>) {
        let label = |kind: &str| {
            let size = if self.size == 0 { 's' } else { 'l' };
            format!("fl_window_{size}_{kind}")
        };
        match command {
            Command::Open => {
                if self.flags & FLAG_OPEN != 0 {
                    return;
                }
                self.flags = FLAG_OPEN;
                self.cursor = self.def_cursor;
                let label = label("in");
                self.flash.goto_label(&label, true);
                sounds.push((1, 4));
            }
            Command::Close => {
                if self.flags & FLAG_CLOSING != 0 {
                    return;
                }
                self.flags |= FLAG_CLOSING;
                self.flags &= !FLAG_OPEN;
                let label = label("out");
                self.flash.goto_label(&label, true);
                sounds.push((1, 5));
            }
            Command::Show => {
                self.flags = FLAG_OPEN;
                self.cursor = self.def_cursor;
                let label = label("open");
                self.flash.goto_label(&label, true);
            }
        }
    }

    /// `Dialog_Input`: 1 for the first choice, -2 for the second, -1 for a cancel, 0 otherwise.
    pub fn input(&mut self, allow_cancel: bool, pad: Pad, sounds: &mut Vec<(u32, u32)>) -> i32 {
        if self.flash.flags() & FLASH_PAD == 0 || !self.choices {
            return 0;
        }
        let mut result = 0;
        if pad.repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
            self.play_cursor_plate("fl_off_start");
            self.cursor = if pad.repeat & PAD_LEFT != 0 {
                if self.cursor - 1 < 0 {
                    1
                } else {
                    self.cursor - 1
                }
            } else if self.cursor + 1 >= 2 {
                0
            } else {
                self.cursor + 1
            };
            self.play_cursor_plate("fl_on_start");
            sounds.push((1, 0));
        } else if pad.pressed & PAD_CROSS != 0 {
            match self.cursor {
                0 => {
                    result = 1;
                    self.flags |= FLAG_DECIDED;
                }
                1 => {
                    result = -2;
                    self.flags |= FLAG_DECIDED;
                }
                _ => {}
            }
            self.play_cursor_plate("fl_ok");
            sounds.push((1, 1));
        } else if pad.pressed & PAD_TRIANGLE != 0 && allow_cancel {
            self.play_cursor_plate("fl_off_start");
            result = -1;
            sounds.push((1, 2));
        }
        result
    }

    pub fn set_choices(&mut self, on: bool) {
        self.choices = on;
    }

    pub fn set_layout(&mut self, layout: i32) {
        self.layout = layout;
    }

    pub fn set_msg(&mut self, msg: i32) {
        self.msg = msg;
    }

    pub fn set_title(&mut self, title: i32) {
        self.title = title;
    }

    /// `Dialog_SetCursor`: the window opens on this choice from now on.
    pub fn set_cursor(&mut self, choice: i32) {
        if self.cursor == choice ^ 1 {
            self.play_cursor_plate("fl_off_start");
        }
        self.cursor = choice;
        self.def_cursor = choice;
        self.play_cursor_plate("fl_on_start");
    }

    pub fn is_closed(&self) -> bool {
        self.flags & FLAG_CLOSED != 0
    }

    pub fn is_open(&self) -> bool {
        self.flash.flags() & FLASH_PAD != 0
    }
}

/// The style every dialog text is printed with: centred, white, eight-way dark outline, faded with the anchor.
fn print(font: &mut Font, x: i32, y: i32, alpha: f32, text: &[u16]) {
    font.push_style();
    font.style.align = 1;
    font.style.scale_x = 1.0;
    font.style.scale_y = 1.0;
    font.style.color = rgba(0xFF, 0xFF, 0xFF, (alpha * 128.0) as u32 as u8);
    font.style.shadow_mode = 2;
    font.style.shadow_dx = 1;
    font.style.shadow_dy = 2;
    font.style.shadow_color = ((alpha * 64.0) as u32) << 24 | 0x20_2020;
    font.print_at(x, y, text);
    font.pop_style();
}
