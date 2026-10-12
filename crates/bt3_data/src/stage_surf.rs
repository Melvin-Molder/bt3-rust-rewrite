//! The stage's animated surfaces (stage pack entry 0xD, `EftSurf_Init` / `EftSurf_Relocate`, src/battle/
//! eft_stage_2.c): the reflecting surface's parameters, then a surface file of version 4 with its textures (records
//! laid out as a texture file's, their offsets counted in bytes from the file) and its meshes, each a list of
//! triangles with a texture, a scroll and flags.

use std::io;

use crate::tex::TextureEntry;

/// `EftSurfParam`: the first 0x40 bytes of the entry.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfParam {
    pub color_a: [f32; 4],
    pub color_b: [f32; 4],
    pub light_dir: [f32; 4],
    pub specular: f32,
    pub anim_speed: i32,
    pub fade_dist: f32,
}

/// `EftSurfVtx`: position, (s, t), colour (r, g, b, alpha; 128 = 1.0).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfVertex {
    pub pos: [f32; 3],
    pub st: [f32; 2],
    pub color: [f32; 4],
}

/// `EftSurfMesh`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfMesh {
    pub tex: usize,
    pub tris: Vec<[SurfVertex; 3]>,
    /// The scroll's step a frame and its first position.
    pub du: f32,
    pub dv: f32,
    pub u: f32,
    pub v: f32,
    pub z_bias: i32,
    /// `EFT_SURF_*`.
    pub flags: u32,
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
    pub center: [f32; 3],
}

/// Stage pack entry 0xD: the parameters, the textures and the meshes.
pub struct SurfFile<'a> {
    pub param: SurfParam,
    pub textures: Vec<TextureEntry<'a>>,
    pub meshes: Vec<SurfMesh>,
}

impl<'a> SurfFile<'a> {
    /// A file that is not version 4 has no textures and no meshes, as `EftSurf_Relocate` empties it.
    pub fn parse(entry: &'a [u8]) -> io::Result<Self> {
        let f = |o: usize| read_f32(entry, o);
        let quad =
            |o: usize| -> io::Result<[f32; 4]> { Ok([f(o)?, f(o + 4)?, f(o + 8)?, f(o + 12)?]) };
        let param = SurfParam {
            color_a: quad(0)?,
            color_b: quad(0x10)?,
            light_dir: quad(0x20)?,
            specular: f(0x30)?,
            anim_speed: read_i32(entry, 0x34)?,
            fade_dist: f(0x38)?,
        };
        let file = entry
            .get(0x40..)
            .ok_or_else(|| invalid_data("surface file is truncated"))?;
        if read_i32(file, 4)? != 4 {
            return Ok(Self {
                param,
                textures: Vec::new(),
                meshes: Vec::new(),
            });
        }
        let count = |o: usize| {
            usize::try_from(read_i32(file, o)?).map_err(|_| invalid_data("negative surface count"))
        };
        let (tex_count, tex_at) = (count(0x10)?, count(0x14)?);
        let (mesh_count, mesh_at) = (count(0x18)?, count(0x1C)?);
        let mut textures = Vec::with_capacity(tex_count);
        for i in 0..tex_count {
            let r = tex_at + i * 0x40;
            let payload = |off: u32, size: i32| -> io::Result<Option<&'a [u8]>> {
                if off == 0 || size <= 0 {
                    return Ok(None);
                }
                let start = off as usize;
                file.get(start..start + size as usize)
                    .map(Some)
                    .ok_or_else(|| invalid_data("surface texture data exceeds the file"))
            };
            let (pixel_size, clut_size) = (read_i32(file, r + 8)?, read_i32(file, r + 12)?);
            textures.push(TextureEntry {
                pixel_data: payload(read_u32(file, r)?, pixel_size)?,
                clut_data: payload(read_u32(file, r + 4)?, clut_size)?,
                pixel_size,
                clut_size,
                pixel_blt: read_u32(file, r + 24)?,
                clut_blt: read_u32(file, r + 28)?,
                tbp_offset: read_i32(file, r + 32)?,
                cbp_offset: read_i32(file, r + 36)?,
                // TCC is set in the file; the draw sets it again (`EftVram_AddTex(.., 1, 0)`), and the texels
                // decode the same either way.
                texture_register: (u64::from(read_u32(file, r + 0x30)?)
                    | u64::from(read_u32(file, r + 0x34)?) << 32)
                    & !(1 << 34),
            });
        }
        let mut meshes = Vec::with_capacity(mesh_count);
        for i in 0..mesh_count {
            let m = mesh_at + i * 0x60;
            let tris_at = count(m + 8)?;
            let tris = (0..count(m + 4)?)
                .map(|k| {
                    let t = tris_at + k * 0x90;
                    let vtx = |n: usize| -> io::Result<SurfVertex> {
                        let o = t + n * 0x30;
                        Ok(SurfVertex {
                            pos: [
                                read_f32(file, o)?,
                                read_f32(file, o + 4)?,
                                read_f32(file, o + 8)?,
                            ],
                            st: [read_f32(file, o + 0x10)?, read_f32(file, o + 0x14)?],
                            color: [
                                read_f32(file, o + 0x20)?,
                                read_f32(file, o + 0x24)?,
                                read_f32(file, o + 0x28)?,
                                read_f32(file, o + 0x2C)?,
                            ],
                        })
                    };
                    Ok([vtx(0)?, vtx(1)?, vtx(2)?])
                })
                .collect::<io::Result<Vec<_>>>()?;
            let v3 = |o: usize| -> io::Result<[f32; 3]> {
                Ok([
                    read_f32(file, o)?,
                    read_f32(file, o + 4)?,
                    read_f32(file, o + 8)?,
                ])
            };
            meshes.push(SurfMesh {
                tex: count(m)?,
                tris,
                du: read_f32(file, m + 0xC)?,
                dv: read_f32(file, m + 0x10)?,
                u: read_f32(file, m + 0x14)?,
                v: read_f32(file, m + 0x18)?,
                z_bias: read_i32(file, m + 0x1C)?,
                flags: read_u32(file, m + 0x20)?,
                box_min: v3(m + 0x30)?,
                box_max: v3(m + 0x40)?,
                center: v3(m + 0x50)?,
            });
        }
        Ok(Self {
            param,
            textures,
            meshes,
        })
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> io::Result<u32> {
    bytes
        .get(offset..offset + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid_data("surface field exceeds the entry"))
}

fn read_i32(bytes: &[u8], offset: usize) -> io::Result<i32> {
    read_u32(bytes, offset).map(|v| v as i32)
}

fn read_f32(bytes: &[u8], offset: usize) -> io::Result<f32> {
    read_u32(bytes, offset).map(f32::from_bits)
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
