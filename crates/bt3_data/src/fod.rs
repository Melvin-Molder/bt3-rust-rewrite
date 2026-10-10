use std::io;

use crate::invalid_data;

/// The tag list of a FOD movie (src/sys/flash.c): a converted SWF, little-endian, fields unaligned.
#[derive(Clone, Copy)]
pub struct Fod<'a> {
    data: &'a [u8],
}

/// A tag or record header: code, record count, size of the data that follows the 8 header bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagHeader {
    pub code: u8,
    pub count: u16,
    pub size: u32,
}

impl<'a> Fod<'a> {
    /// `Flash_CheckHeader` + `Flash_Create`: 'F' 'O' 'D' 0x11 "LIT\0", then the tag list.
    pub fn parse(file: &'a [u8]) -> io::Result<Self> {
        if file.len() < 8 || &file[..4] != b"FOD\x11" || &file[4..8] != b"LIT\0" {
            return Err(invalid_data("missing FOD header"));
        }
        Ok(Self { data: &file[8..] })
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.data
    }

    /// `Flash_FindTag`: offset of the first tag with `code`, or `None` when the end tag comes first.
    pub fn find_tag(&self, code: u8) -> io::Result<Option<usize>> {
        let mut ofs = 0usize;
        loop {
            let tag = header(self.data, ofs)?;
            if tag.code == code {
                return Ok(Some(ofs));
            }
            if tag.code == 0 {
                return Ok(None);
            }
            ofs = advance(ofs, tag.size)?;
        }
    }

    /// `Flash_SkipRecords`: offset of record `index` inside the tag at `tag`.
    pub fn skip_records(&self, tag: usize, index: u16) -> io::Result<usize> {
        let mut ofs = tag + 8;
        for _ in 0..index {
            ofs = advance(ofs, header(self.data, ofs)?.size)?;
        }
        Ok(ofs)
    }

    /// `Flash_GetRecord`: offset of record `index` of the first tag with `code`.
    pub fn record(&self, code: u8, index: u16) -> io::Result<usize> {
        let tag = self
            .find_tag(code)?
            .ok_or_else(|| invalid_data("FOD tag is missing"))?;
        self.skip_records(tag, index)
    }

    /// `Flash_GetRecordCount`: records in the first tag with `code`, 0 without one.
    pub fn record_count(&self, code: u8) -> io::Result<u16> {
        Ok(match self.find_tag(code)? {
            Some(ofs) => header(self.data, ofs)?.count,
            None => 0,
        })
    }

    pub fn header(&self, ofs: usize) -> io::Result<TagHeader> {
        header(self.data, ofs)
    }
}

fn header(data: &[u8], ofs: usize) -> io::Result<TagHeader> {
    Ok(TagHeader {
        code: byte(data, ofs)?,
        count: u16_at(data, ofs + 2)?,
        size: u32_at(data, ofs + 4)?,
    })
}

fn advance(ofs: usize, size: u32) -> io::Result<usize> {
    ofs.checked_add(8)
        .and_then(|ofs| ofs.checked_add(size as usize))
        .ok_or_else(|| invalid_data("FOD tag size overflow"))
}

/// `Flash_SkipNamed`: offset of named block `count` (NUL-terminated name, u16 size, data); `None` when a block
/// of size 0 comes first.
pub fn skip_named(data: &[u8], start: usize, count: u32) -> io::Result<Option<usize>> {
    let mut ofs = start;
    for _ in 0..count {
        ofs = skip_string(data, ofs)?;
        let size = u16_at(data, ofs)?;
        ofs += 2;
        if size == 0 {
            return Ok(None);
        }
        ofs += usize::from(size);
    }
    Ok(Some(ofs))
}

/// `Flash_FindName`: index of the named block called `name`. Like the original, the size-0 block that ends the
/// list is compared too.
pub fn find_name(data: &[u8], start: usize, name: &[u8]) -> io::Result<Option<u32>> {
    let mut ofs = start;
    let mut size = 0u16;
    let mut index = 0u32;
    loop {
        ofs += usize::from(size);
        let (block_name, next) = read_string(data, ofs)?;
        ofs = next;
        if block_name == name {
            return Ok(Some(index));
        }
        index += 1;
        size = u16_at(data, ofs)?;
        ofs += 2;
        if size == 0 {
            return Ok(None);
        }
    }
}

/// `Flash_ReadString`: the NUL-terminated string at `ofs` and the offset after its NUL.
pub fn read_string(data: &[u8], ofs: usize) -> io::Result<(&[u8], usize)> {
    let rest = data
        .get(ofs..)
        .ok_or_else(|| invalid_data("FOD string starts outside the data"))?;
    let len = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| invalid_data("FOD string is not terminated"))?;
    Ok((&rest[..len], ofs + len + 1))
}

fn skip_string(data: &[u8], ofs: usize) -> io::Result<usize> {
    Ok(read_string(data, ofs)?.1)
}

/// MSB-first bit reader over `data` starting at a byte offset (`Flash_ReadBits` / `Flash_ReadSBits`).
struct Bits<'a> {
    data: &'a [u8],
    base: usize,
    pos: u32,
}

impl Bits<'_> {
    fn read(&mut self, bits: u8) -> io::Result<u32> {
        let mut value = 0u32;
        for _ in 0..bits {
            let byte = byte(self.data, self.base + (self.pos >> 3) as usize)?;
            value = (value << 1) | u32::from((byte >> (7 - (self.pos & 7))) & 1);
            self.pos += 1;
        }
        Ok(value)
    }

    fn read_signed(&mut self, bits: u8) -> io::Result<i32> {
        if bits == 0 {
            return Ok(0);
        }
        let value = self.read(bits)? as i32;
        Ok(if bits < 32 && (value >> (bits - 1)) & 1 != 0 {
            value | (-1i32 << bits)
        } else {
            value
        })
    }

    /// The byte after the last bit read, rounded up.
    fn end(&self) -> usize {
        self.base + self.pos.div_ceil(8) as usize
    }
}

/// 3x3 matrix: [0] [4] scale, [1] [3] skew, [2] [5] translation in pixels, [8] = 1.
pub type Matrix = [f32; 9];
pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// `Flash_ReadMtx`: a flag byte, then scale / skew pairs in 16.16 and translation in twentieths.
pub fn read_matrix(data: &[u8], ofs: usize) -> io::Result<(Matrix, usize)> {
    let mut m = IDENTITY;
    let flags = byte(data, ofs)?;
    let mut bits = Bits {
        data,
        base: ofs + 1,
        pos: 0,
    };
    if flags & 0x11 != 0 {
        let width = bits.read(5)? as u8;
        if flags & 1 != 0 {
            m[0] = bits.read_signed(width)? as f32 * (1.0 / 65536.0);
        }
        if flags & 0x10 != 0 {
            m[4] = bits.read_signed(width)? as f32 * (1.0 / 65536.0);
        }
    }
    if flags & 0xA != 0 {
        let width = bits.read(5)? as u8;
        if flags & 2 != 0 {
            m[1] = bits.read_signed(width)? as f32 * (1.0 / 65536.0);
        }
        if flags & 8 != 0 {
            m[3] = bits.read_signed(width)? as f32 * (1.0 / 65536.0);
        }
    }
    if flags & 0x24 != 0 {
        let width = bits.read(5)? as u8;
        if flags & 4 != 0 {
            m[2] = bits.read_signed(width)? as f32 * 0.05;
        }
        if flags & 0x20 != 0 {
            m[5] = bits.read_signed(width)? as f32 * 0.05;
        }
    }
    Ok((m, bits.end()))
}

/// Colour transform: channel = channel * mul + add, order r, g, b, a.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cxform {
    pub mul: [f32; 4],
    pub add: [i16; 4],
}

pub const CXFORM_IDENTITY: Cxform = Cxform {
    mul: [1.0; 4],
    add: [0; 4],
};

/// `Flash_ReadCxform`: a flag byte (bits 0..3 multipliers, 4..7 offsets), a 4-bit width, the fields.
pub fn read_cxform(data: &[u8], ofs: usize) -> io::Result<(Cxform, usize)> {
    let mut cx = CXFORM_IDENTITY;
    let flags = byte(data, ofs)?;
    let mut bits = Bits {
        data,
        base: ofs + 1,
        pos: 0,
    };
    if flags != 0 {
        let width = bits.read(4)? as u8;
        for i in 0..4 {
            if (flags >> i) & 1 != 0 {
                cx.mul[i] = f32::from(bits.read_signed(width)? as i16) * (1.0 / 256.0);
            }
        }
        for i in 0..4 {
            if flags & (0x10 << i) != 0 {
                cx.add[i] = bits.read_signed(width)? as i16;
            }
        }
    }
    Ok((cx, bits.end()))
}

/// One SWF action of a DoAction tag, as `FlashTl_TagActions` reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    NextFrame,
    PrevFrame,
    Play,
    Stop,
    GotoFrame(u16),
    GetUrl(Vec<u8>, Vec<u8>),
    WaitForFrame,
    SetTarget(Vec<u8>),
    GotoLabel(Vec<u8>),
    /// Any other code: the original reads nothing more for it.
    Other(u8),
}

/// One tag of a frame block, in the order `FlashTl_RunFrame` meets them.
#[derive(Clone, Debug, PartialEq)]
pub enum FrameTag {
    /// Tag 4: kind 4 shape (identity colour transform) or 6 sprite; other kinds place nothing.
    Place {
        depth: u16,
        kind: u8,
        id: u16,
        matrix: Matrix,
        cxform: Cxform,
    },
    /// Tag 26 with flag 2: a new object; `name` is empty without flag 0x20.
    PlaceNew {
        depth: u16,
        kind: u8,
        id: u16,
        matrix: Matrix,
        cxform: Cxform,
        name: Vec<u8>,
    },
    /// Tag 26 without flag 2: new matrix and / or colour transform for what is at `depth`.
    Move {
        depth: u16,
        matrix: Option<Matrix>,
        cxform: Option<Cxform>,
    },
    /// Tag 5.
    Remove { depth: u16, kind: u8, id: u16 },
    /// Tag 28.
    Remove2 { depth: u16 },
    /// Tag 12.
    Actions(Vec<Action>),
    /// Any other code: the original reads the next u16 as the next code.
    Unknown(u16),
}

/// A frame block: its label and its tags up to ShowFrame (code 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub label: Vec<u8>,
    pub tags: Vec<FrameTag>,
}

/// Parses the frame block at `ofs`; `None` for the size-0 block that ends a timeline.
pub fn parse_frame(data: &[u8], ofs: usize) -> io::Result<Option<Frame>> {
    let (label, mut ofs) = read_string(data, ofs)?;
    let size = u16_at(data, ofs)?;
    ofs += 2;
    if size == 0 {
        return Ok(None);
    }
    let mut tags = Vec::new();
    loop {
        let code = u16_at(data, ofs)?;
        ofs += 2;
        let tag = match code {
            1 => break,
            4 => {
                let depth = u16_at(data, ofs)?;
                let kind = byte(data, ofs + 2)?;
                let id = u16_at(data, ofs + 3)?;
                ofs += 5;
                let (mut matrix, mut cxform) = (IDENTITY, CXFORM_IDENTITY);
                if kind == 4 || kind == 6 {
                    (matrix, ofs) = read_matrix(data, ofs)?;
                }
                if kind == 6 {
                    (cxform, ofs) = read_cxform(data, ofs)?;
                }
                FrameTag::Place {
                    depth,
                    kind,
                    id,
                    matrix,
                    cxform,
                }
            }
            26 => {
                let flags = byte(data, ofs)?;
                let depth = u16_at(data, ofs + 1)?;
                ofs += 3;
                let (mut kind, mut id) = (0, 0);
                if flags & 2 != 0 {
                    kind = byte(data, ofs)?;
                    id = u16_at(data, ofs + 1)?;
                    ofs += 3;
                }
                let mut matrix = None;
                let mut cxform = None;
                if flags & 4 != 0 {
                    let (m, next) = read_matrix(data, ofs)?;
                    matrix = Some(m);
                    ofs = next;
                }
                if flags & 8 != 0 {
                    let (cx, next) = read_cxform(data, ofs)?;
                    cxform = Some(cx);
                    ofs = next;
                }
                if flags & 2 != 0 {
                    let mut name = Vec::new();
                    if flags & 0x20 != 0 {
                        let (s, next) = read_string(data, ofs)?;
                        name = s.to_vec();
                        ofs = next;
                    }
                    FrameTag::PlaceNew {
                        depth,
                        kind,
                        id,
                        matrix: matrix.unwrap_or(IDENTITY),
                        cxform: cxform.unwrap_or(CXFORM_IDENTITY),
                        name,
                    }
                } else {
                    FrameTag::Move {
                        depth,
                        matrix,
                        cxform,
                    }
                }
            }
            5 => {
                let tag = FrameTag::Remove {
                    depth: u16_at(data, ofs)?,
                    kind: byte(data, ofs + 2)?,
                    id: u16_at(data, ofs + 3)?,
                };
                ofs += 5;
                tag
            }
            28 => {
                let tag = FrameTag::Remove2 {
                    depth: u16_at(data, ofs)?,
                };
                ofs += 2;
                tag
            }
            12 => {
                let (actions, next) = parse_actions(data, ofs)?;
                ofs = next;
                FrameTag::Actions(actions)
            }
            other => FrameTag::Unknown(other),
        };
        tags.push(tag);
    }
    Ok(Some(Frame {
        label: label.to_vec(),
        tags,
    }))
}

fn parse_actions(data: &[u8], mut ofs: usize) -> io::Result<(Vec<Action>, usize)> {
    let mut actions = Vec::new();
    loop {
        let code = byte(data, ofs)?;
        ofs += 1;
        let action = match code {
            0 => break,
            4 => Action::NextFrame,
            5 => Action::PrevFrame,
            6 => Action::Play,
            7 => Action::Stop,
            0x81 => {
                ofs += 2;
                Action::GotoFrame(u16_at(data, ofs - 2)?)
            }
            0x83 => {
                let (url, next) = read_string(data, ofs)?;
                let (arg, next) = read_string(data, next)?;
                ofs = next;
                Action::GetUrl(url.to_vec(), arg.to_vec())
            }
            0x8A => {
                ofs += 3;
                Action::WaitForFrame
            }
            0x8B => {
                let (name, next) = read_string(data, ofs)?;
                ofs = next;
                Action::SetTarget(name.to_vec())
            }
            0x8C => {
                let (label, next) = read_string(data, ofs)?;
                ofs = next;
                Action::GotoLabel(label.to_vec())
            }
            other => Action::Other(other),
        };
        actions.push(action);
    }
    Ok((actions, ofs))
}

pub fn byte(data: &[u8], ofs: usize) -> io::Result<u8> {
    data.get(ofs)
        .copied()
        .ok_or_else(|| invalid_data("FOD read outside the data"))
}

pub fn u16_at(data: &[u8], ofs: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes([byte(data, ofs)?, byte(data, ofs + 1)?]))
}

pub fn i16_at(data: &[u8], ofs: usize) -> io::Result<i16> {
    Ok(u16_at(data, ofs)? as i16)
}

pub fn u32_at(data: &[u8], ofs: usize) -> io::Result<u32> {
    data.get(ofs..ofs + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid_data("FOD read outside the data"))
}

pub fn i32_at(data: &[u8], ofs: usize) -> io::Result<i32> {
    Ok(u32_at(data, ofs)? as i32)
}

#[cfg(test)]
mod tests {
    use super::{Fod, IDENTITY, find_name, read_cxform, read_matrix, skip_named};

    fn tag(code: u8, count: u16, data: &[u8]) -> Vec<u8> {
        let mut out = vec![code, 0];
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn finds_tags_and_records() {
        let mut records = tag(0, 0, &[1, 2]);
        records.extend(tag(0, 0, &[3]));
        let mut file = b"FOD\x11LIT\0".to_vec();
        file.extend(tag(3, 2, &records));
        file.extend(tag(0, 0, &[]));
        let fod = Fod::parse(&file).unwrap();
        assert_eq!(fod.find_tag(3).unwrap(), Some(0));
        assert_eq!(fod.find_tag(9).unwrap(), None);
        assert_eq!(fod.record_count(3).unwrap(), 2);
        let second = fod.record(3, 1).unwrap();
        assert_eq!(fod.bytes()[second + 8], 3);
    }

    #[test]
    fn named_blocks_end_at_size_zero() {
        let blocks = b"\0\x02\0abfl_x\0\x01\0c\0\0\0".to_vec();
        assert_eq!(skip_named(&blocks, 0, 1).unwrap(), Some(5));
        assert_eq!(skip_named(&blocks, 0, 2).unwrap(), Some(13));
        assert_eq!(skip_named(&blocks, 0, 3).unwrap(), None);
        assert_eq!(find_name(&blocks, 0, b"fl_x").unwrap(), Some(1));
        assert_eq!(find_name(&blocks, 0, b"fl_y").unwrap(), None);
    }

    #[test]
    fn matrix_reads_scale_and_translation_fields() {
        // flags 0x25: scale x + translation; width 18 for scale (1.0 = 0x10000), width 9 for x=-20 twips, y=40.
        let mut bits = String::new();
        bits += &format!("{:05b}", 18);
        bits += &format!("{:018b}", 0x10000);
        bits += &format!("{:05b}", 9);
        bits += &format!("{:09b}", (-20i32 as u32) & 0x1FF);
        bits += &format!("{:09b}", 40);
        while bits.len() % 8 != 0 {
            bits.push('0');
        }
        let mut data = vec![0x25u8];
        data.extend(
            bits.as_bytes()
                .chunks(8)
                .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 2).unwrap()),
        );
        let (m, end) = read_matrix(&data, 0).unwrap();
        assert_eq!(m[0], 1.0);
        assert_eq!(m[2], -1.0);
        assert_eq!(m[5], 2.0);
        assert_eq!((m[1], m[3], m[4]), (IDENTITY[1], IDENTITY[3], IDENTITY[4]));
        assert_eq!(end, data.len());
    }

    #[test]
    fn cxform_reads_alpha_multiplier() {
        // flags 0x08: alpha multiplier only, width 10, value 0x80 = 0.5.
        let data = [0x08u8, 0b1010_0010, 0b0000_0000];
        let (cx, end) = read_cxform(&data, 0).unwrap();
        assert_eq!(cx.mul, [1.0, 1.0, 1.0, 0.5]);
        assert_eq!(end, 3);
    }
}
