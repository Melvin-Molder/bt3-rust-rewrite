use std::io;

use crate::invalid_data;
use crate::tex::TextureEntry;

/// One glyph of a font file (`FontGlyph`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub code: u16,
    /// Bits 0-9 texel x, 10-14 width, 15 low bit of the CLUT number.
    pub u: u16,
    /// Bits 0-9 texel y, 10-14 height, 15 high bit of the CLUT number.
    pub v: u16,
}

impl Glyph {
    pub fn width(&self) -> i32 {
        i32::from((self.u >> 10) & 0x1F)
    }

    pub fn height(&self) -> i32 {
        i32::from((self.v >> 10) & 0x1F)
    }

    pub fn texel(&self) -> (i32, i32) {
        (i32::from(self.u & 0x3FF), i32::from(self.v & 0x3FF))
    }

    /// `((v >> 15) << 1) + (u >> 15)` (Font_DrawGlyph).
    pub fn clut(&self) -> usize {
        usize::from(((self.v >> 15) << 1) + (self.u >> 15))
    }
}

/// A font file of the boot file (`FontData`): "5tnF 2SP" header, texture header at +0xC, glyphs sorted by code.
pub struct FontFile<'a> {
    pub texture: TextureEntry<'a>,
    pub glyphs: Vec<Glyph>,
    pub line_height: i32,
}

impl<'a> FontFile<'a> {
    pub fn parse(data: &'a [u8]) -> io::Result<Self> {
        let tex = u32_at(data, 0xC)? as usize;
        let glyphs_ofs = u32_at(data, 0x14)? as usize;
        let count = u32_at(data, 0x18)? as usize;
        let line_height = u32_at(data, 0x20)? as i32;
        let header = data
            .get(tex..tex + 0x40)
            .ok_or_else(|| invalid_data("font texture header is outside the file"))?;
        // Font_SetFont: the texture's pixel and CLUT offsets are bytes from the font file's start.
        let payload = |ofs: usize, size: usize| -> io::Result<Option<&'a [u8]>> {
            if ofs == 0 {
                return Ok(None);
            }
            data.get(ofs..ofs + size)
                .map(Some)
                .ok_or_else(|| invalid_data("font texture data is outside the file"))
        };
        let pixel_size = u32_at(header, 8)? as usize;
        let clut_size = u32_at(header, 12)? as usize;
        let texture = TextureEntry {
            pixel_data: payload(u32_at(header, 0)? as usize, pixel_size)?,
            clut_data: payload(u32_at(header, 4)? as usize, clut_size)?,
            pixel_size: pixel_size as i32,
            clut_size: clut_size as i32,
            pixel_blt: u32_at(header, 24)?,
            clut_blt: u32_at(header, 28)?,
            tbp_offset: u32_at(header, 32)? as i32,
            cbp_offset: u32_at(header, 36)? as i32,
            extra: [0; 8],
            texture_register: u64::from(u32_at(header, 0x30)?)
                | u64::from(u32_at(header, 0x34)?) << 32,
        };
        let glyphs = (0..count)
            .map(|i| {
                let g = glyphs_ofs + i * 6;
                Ok(Glyph {
                    code: u16_at(data, g)?,
                    u: u16_at(data, g + 2)?,
                    v: u16_at(data, g + 4)?,
                })
            })
            .collect::<io::Result<_>>()?;
        Ok(Self {
            texture,
            glyphs,
            line_height,
        })
    }
}

/// Text `index` of a message table (`DIALOG_TEXT`): the offset of text i is word i + 1, rounded down to a word.
pub fn message(table: &[u8], index: usize) -> io::Result<Vec<u16>> {
    let ofs = (u32_at(table, 4 + index * 4)? as usize) & !3;
    let text = table
        .get(ofs..)
        .ok_or_else(|| invalid_data("message is outside its table"))?;
    Ok(text
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect())
}

/// Number of texts in a message table (word 0).
pub fn message_count(table: &[u8]) -> io::Result<usize> {
    Ok(u32_at(table, 0)? as usize)
}

fn u16_at(data: &[u8], ofs: usize) -> io::Result<u16> {
    data.get(ofs..ofs + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| invalid_data("font read outside the file"))
}

fn u32_at(data: &[u8], ofs: usize) -> io::Result<u32> {
    data.get(ofs..ofs + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid_data("font read outside the file"))
}

#[cfg(test)]
mod tests {
    use super::{Glyph, message};

    #[test]
    fn glyph_fields() {
        // '!' of the first font: u 0x1414, v 0x4400.
        let g = Glyph {
            code: 0x21,
            u: 0x1414,
            v: 0x4400,
        };
        assert_eq!(
            (g.width(), g.height(), g.texel(), g.clut()),
            (5, 17, (0x14, 0), 0)
        );
        assert_eq!(
            Glyph {
                code: 0,
                u: 0x8000,
                v: 0x8000
            }
            .clut(),
            3
        );
    }

    #[test]
    fn reads_messages_by_word_offset() {
        let mut table = vec![1, 0, 0, 0, 9, 0, 0, 0];
        table.extend([b'H', 0, b'i', 0, 0, 0]);
        assert_eq!(
            message(&table, 0).unwrap(),
            [u16::from(b'H'), u16::from(b'i')]
        );
    }
}
