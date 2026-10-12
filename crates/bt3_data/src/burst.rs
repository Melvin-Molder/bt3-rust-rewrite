//! The stage-change transition's model (`EftBurstModel`, transition pack entry 2, magic `cmdl`): groups of
//! triangles with per-corner UVs and colours, drawn around the transition's camera, and its own textures.

use std::io;

use crate::tex::TextureEntry;

/// A triangle (`EftBurstTri`, 0x1E bytes): its vertices, the UV of each corner (4.12 fixed point) and its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BurstTri {
    pub vertices: [usize; 3],
    pub uv: [[i16; 2]; 3],
    pub rgba: [[u8; 4]; 3],
}

/// `EftBurstGroup`: flag 0x800 adds the second, environment pass; `positions` are in 4.12 fixed point before the
/// group's scale.
#[derive(Clone, Debug, PartialEq)]
pub struct BurstGroup {
    pub flags: i32,
    pub texture: usize,
    pub scale: f32,
    pub positions: Vec<[i16; 3]>,
    pub tris: Vec<BurstTri>,
}

/// The model: its groups and its textures (in the texture files' entry layout, offsets in bytes).
pub struct BurstModel<'a> {
    pub groups: Vec<BurstGroup>,
    pub textures: Vec<TextureEntry<'a>>,
}

impl<'a> BurstModel<'a> {
    pub fn parse(bytes: &'a [u8]) -> io::Result<Self> {
        if bytes.get(..4) != Some(b"cmdl") {
            return Err(invalid_data("transition model has no cmdl magic"));
        }
        let group_count = count(bytes, 4)?;
        let groups_at = u32_at(bytes, 8)? as usize;
        let tex_count = count(bytes, 0xC)?;
        let tex_at = u32_at(bytes, 0x10)? as usize;
        let mut groups = Vec::new();
        for i in 0..group_count {
            let g = groups_at + i * 0x18;
            let verts_at = u32_at(bytes, g + 8)? as usize;
            let tris_at = u32_at(bytes, g + 0xC)? as usize;
            let tri_count = usize::try_from(i16_at(bytes, g + 0x14)?)
                .map_err(|_| invalid_data("transition model triangle count is negative"))?;
            let mut tris = Vec::new();
            for t in 0..tri_count {
                let o = tris_at + t * 0x1E;
                let vertex = |k: usize| -> io::Result<usize> {
                    usize::try_from(i16_at(bytes, o + k * 2)?)
                        .map_err(|_| invalid_data("transition model vertex index is negative"))
                };
                let uv = |k: usize| -> io::Result<[i16; 2]> {
                    Ok([i16_at(bytes, o + 6 + k * 4)?, i16_at(bytes, o + 8 + k * 4)?])
                };
                let rgba = |k: usize| -> io::Result<[u8; 4]> {
                    let c = o + 0x12 + k * 4;
                    let b = bytes
                        .get(c..c + 4)
                        .ok_or_else(|| invalid_data("transition model colour exceeds the file"))?;
                    Ok([b[0], b[1], b[2], b[3]])
                };
                tris.push(BurstTri {
                    vertices: [vertex(0)?, vertex(1)?, vertex(2)?],
                    uv: [uv(0)?, uv(1)?, uv(2)?],
                    rgba: [rgba(0)?, rgba(1)?, rgba(2)?],
                });
            }
            let vertex_count = tris
                .iter()
                .flat_map(|t| t.vertices)
                .max()
                .map_or(0, |m| m + 1);
            let positions = (0..vertex_count)
                .map(|v| {
                    let o = verts_at + v * 6;
                    Ok([
                        i16_at(bytes, o)?,
                        i16_at(bytes, o + 2)?,
                        i16_at(bytes, o + 4)?,
                    ])
                })
                .collect::<io::Result<_>>()?;
            groups.push(BurstGroup {
                flags: u32_at(bytes, g)? as i32,
                texture: u32_at(bytes, g + 4)? as usize,
                scale: f32::from_bits(u32_at(bytes, g + 0x10)?),
                positions,
                tris,
            });
        }
        let mut textures = Vec::new();
        for i in 0..tex_count {
            let e = tex_at + i * 0x40;
            let block = |offset: usize| -> io::Result<Option<&'a [u8]>> {
                let at = u32_at(bytes, offset)? as usize;
                Ok(bytes.get(at..).filter(|_| at != 0))
            };
            textures.push(TextureEntry {
                pixel_data: block(e)?,
                clut_data: block(e + 4)?,
                pixel_size: u32_at(bytes, e + 8)? as i32,
                clut_size: u32_at(bytes, e + 0xC)? as i32,
                pixel_blt: u32_at(bytes, e + 0x18)?,
                clut_blt: u32_at(bytes, e + 0x1C)?,
                tbp_offset: 0,
                cbp_offset: 0,
                texture_register: u64::from(u32_at(bytes, e + 0x30)?)
                    | (u64::from(u32_at(bytes, e + 0x34)?) << 32),
            });
        }
        Ok(Self { groups, textures })
    }
}

fn count(bytes: &[u8], offset: usize) -> io::Result<usize> {
    usize::try_from(u32_at(bytes, offset)? as i32)
        .map_err(|_| invalid_data("transition model count is negative"))
}

fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let b = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_data("transition model field exceeds the file"))?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()))
}

fn i16_at(bytes: &[u8], offset: usize) -> io::Result<i16> {
    let b = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| invalid_data("transition model field exceeds the file"))?;
    Ok(i16::from_le_bytes(b.try_into().unwrap()))
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
