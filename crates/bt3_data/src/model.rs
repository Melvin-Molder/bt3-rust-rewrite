//! Battle character models (`BTL_FILE_CHARA`, file `0x590 + chara * 10 + costume`).
//!
//! The file is a pack; `BtlObj_BindTables` takes section 3 as the model and section 0xC as its texture file.
//! The model starts with a header (`ObjMdlFile` / `ObjDrawModel`) and holds a list of mesh records
//! (`ObjMdlMesh`), one per model part. A record's VIF stream (record + 0x60) is what `Vu1Pkt_CallProg0` sends
//! to VU1 program 0: per batch, a five-quadword header (GIF tag, TEX0_1, TEX0_2 as A+D, the two strip tags)
//! unpacked to TOPS + 0 and the vertices (position with bone-A weight, normal, s / t / 1) to TOPS + 5, then
//! an MSCNT that runs the batch.

use std::io;

use crate::pack_section;

/// `BtlObj_BindTables`: `mdl->file = section 3`, `mdl->texFile = section 0xC`.
pub const MODEL_SECTION: usize = 3;
pub const TEXTURE_SECTION: usize = 0xC;

const HEADER_SIZE: usize = 0x74;
const RECORD_HEADER: usize = 0x60;

#[derive(Clone, Debug, PartialEq)]
pub struct ModelHeader {
    /// +0x0C, bit 31 is the "bound" flag the game sets at run time.
    pub flags: u32,
    /// +0x18..+0x28: height (`obj + 0xFF4`), +0x1C, the centre height and the camera side slope.
    pub size: [f32; 4],
    pub face_tex: u8,
    pub fade_tex: u8,
    /// Textures `[tex_first, tex_first + tex_count)` are the material textures, uploaded every frame.
    pub tex_first: u8,
    pub tex_count: u8,
    /// Range uploaded to `clutTbp` (0x3D00): the shading CLUTs.
    pub clut_first: u8,
    pub clut_count: u8,
    pub fade_first: u8,
    pub fade_count: u8,
    pub tex_no_alpha: u8,
    /// +0x70: offset of the seam section (CPU-skinned triangles), 0 when there is none.
    pub seam_offset: u32,
}

/// One VU1 batch: a triangle strip drawn twice (model texture, then the shading ramp).
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    /// TEX0_1 (model texture) and TEX0_2 (shading ramp) as stored in the file, before `MdlTex_RebaseChain`.
    pub tex0: [u64; 2],
    pub vertices: Vec<Vertex>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    /// x, y, z and the weight of bone A in w.
    pub position: [f32; 4],
    pub normal: [f32; 3],
    pub st: [f32; 2],
}

/// One mesh record (`ObjDrawPart`).
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPart {
    pub node: u16,
    /// Matrices `BtlObjPose_CalcMatrices` pops after this node (the branches it closes).
    pub pop: u16,
    pub enabled: bool,
    /// Bit 0: drawn by the fade pass.
    pub flags: u16,
    /// Pivots of bones A and B.
    pub offset_a: [f32; 4],
    pub offset_b: [f32; 4],
    /// Translation from the parent node in the rest pose (`BobjBone.rest`).
    pub rest: [f32; 4],
    pub batches: Vec<Batch>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CharaModel {
    pub header: ModelHeader,
    pub parts: Vec<ModelPart>,
    /// The lowest TBP0 / CBP of the parts' first TEX0 values (`MdlTex_RebaseChain`'s first pass): a TEX0's
    /// block minus these is the texture entry's `tbpOfs` / `cbpOfs`.
    pub min_tbp: u32,
    pub min_cbp: u32,
}

impl CharaModel {
    /// Parses section 3 of a character file.
    pub fn parse(model: &[u8]) -> io::Result<Self> {
        if model.len() < HEADER_SIZE || &model[0..4] != b"pmdl" {
            return Err(invalid_data("not a pmdl model"));
        }
        let f = |o: usize| f32::from_bits(u32_at(model, o));
        let header = ModelHeader {
            flags: u32_at(model, 0x0C),
            size: [f(0x18), f(0x1C), f(0x20), f(0x24)],
            face_tex: model[0x4C],
            fade_tex: model[0x4D],
            tex_first: model[0x4E],
            tex_count: model[0x4F],
            clut_first: model[0x50],
            clut_count: model[0x51],
            fade_first: model[0x52],
            fade_count: model[0x53],
            tex_no_alpha: model[0x54],
            seam_offset: u32_at(model, 0x70),
        };
        let mut parts = Vec::new();
        let mut offset = u32_at(model, 0x6C) as usize;
        loop {
            let record = model
                .get(offset..offset + 0x40)
                .ok_or_else(|| invalid_data("mesh record outside the model"))?;
            let next = u32_at(record, 0) as usize;
            let last = u16_at(record, 6) != 0;
            let enabled = u16_at(record, 8) != 0;
            let vec4 = |o: usize| [0, 4, 8, 12].map(|i| f32::from_bits(u32_at(record, o + i)));
            let batches = if enabled {
                let end = if last { model.len() } else { offset + next };
                parse_vif(
                    model
                        .get(offset + RECORD_HEADER..end)
                        .ok_or_else(|| invalid_data("mesh record VIF stream outside the model"))?,
                )?
            } else {
                Vec::new()
            };
            parts.push(ModelPart {
                node: u16_at(record, 0xA),
                pop: u16_at(record, 4),
                enabled,
                flags: u16_at(record, 0xC),
                offset_a: vec4(0x10),
                offset_b: vec4(0x20),
                rest: vec4(0x30),
                batches,
            });
            if last {
                break;
            }
            if next == 0 {
                return Err(invalid_data("mesh record without a successor"));
            }
            offset += next;
        }
        let (mut min_tbp, mut min_cbp) = (0x7FFF_FFFF, 0x7FFF_FFFF);
        for batch in parts.iter().flat_map(|part| &part.batches) {
            min_tbp = min_tbp.min(tex0_tbp(batch.tex0[0]));
            min_cbp = min_cbp.min(tex0_cbp(batch.tex0[0]));
        }
        Ok(Self {
            header,
            parts,
            min_tbp,
            min_cbp,
        })
    }

    /// Section 3 of a character file.
    pub fn from_file(file: &[u8]) -> io::Result<Self> {
        Self::parse(pack_section(file, MODEL_SECTION)?)
    }
}

/// TBP0 of a TEX0 value.
pub fn tex0_tbp(tex0: u64) -> u32 {
    (tex0 & 0x3FFF) as u32
}

/// CBP of a TEX0 value.
pub fn tex0_cbp(tex0: u64) -> u32 {
    ((tex0 >> 37) & 0x3FFF) as u32
}

/// Walks a part's DMA chain (one `ret` packet in the files) and its VIF codes, keeping VU1 memory as
/// program 0 sees it (TOPS-relative), and takes a batch at every MSCAL / MSCNT.
fn parse_vif(stream: &[u8]) -> io::Result<Vec<Batch>> {
    let mut batches = Vec::new();
    let mut memory = vec![[0u32; 4]; 0x400];
    let (mut cl, mut wl) = (1usize, 1usize);
    let mut at = 0usize;
    loop {
        let tag = word(stream, at)?;
        let qwc = (tag & 0xFFFF) as usize;
        let id = (tag >> 28) & 7;
        let end = at + 16 + qwc * 16;
        if end > stream.len() {
            return Err(invalid_data("DMA packet exceeds the mesh record"));
        }
        // The tag's upper two words are VIF codes too (TTE is off).
        let mut p = at + 8;
        while p < end {
            let code = word(stream, p)?;
            p += 4;
            let cmd = (code >> 24) & 0x7F;
            let num = ((code >> 16) & 0xFF) as usize;
            let imm = (code & 0xFFFF) as usize;
            match cmd {
                0x00 | 0x02..=0x07 | 0x10 | 0x11 | 0x13 => {}
                0x01 => {
                    cl = imm & 0xFF;
                    wl = (imm >> 8) & 0xFF;
                }
                0x14 | 0x15 | 0x17 => batches.push(take_batch(&memory)?),
                0x20 => p += 4,
                0x30 | 0x31 => p += 16,
                0x4A => p += if num == 0 { 256 } else { num } * 8,
                0x50 | 0x51 => p += if imm == 0 { 0x10000 } else { imm } * 16,
                0x60..=0x7F => {
                    let vn = (cmd >> 2) & 3;
                    let vl = cmd & 3;
                    let num = if num == 0 { 256 } else { num };
                    let bits = (32 >> vl) * (vn as usize + 1);
                    let size = (num * bits).div_ceil(32) * 4;
                    if vn == 3 && vl == 0 {
                        let base = imm & 0x3FF;
                        for i in 0..num {
                            let address = if wl <= cl {
                                base + (i / wl.max(1)) * cl + i % wl.max(1)
                            } else {
                                base + i
                            };
                            let mut q = [0u32; 4];
                            for (k, slot) in q.iter_mut().enumerate() {
                                *slot = word(stream, p + i * 16 + k * 4)?;
                            }
                            memory[address & 0x3FF] = q;
                        }
                    }
                    p += size;
                }
                _ => return Err(invalid_data("unknown VIF code in a mesh record")),
            }
        }
        at = end;
        // ret / end close the part's chain.
        if id == 6 || id == 7 || at >= stream.len() {
            break;
        }
    }
    Ok(batches)
}

/// The batch program 0 reads at TOP: header at +0..+4, n = tag(+3) & 0x7FFF vertices of three quadwords at +5.
fn take_batch(memory: &[[u32; 4]]) -> io::Result<Batch> {
    let quad64 = |q: [u32; 4]| u64::from(q[0]) | (u64::from(q[1]) << 32);
    let n = (memory[3][0] & 0x7FFF) as usize;
    if 5 + n * 3 > memory.len() {
        return Err(invalid_data("batch exceeds VU1 memory"));
    }
    let f = |v: u32| f32::from_bits(v);
    let vertices = (0..n)
        .map(|i| {
            let [px, py, pz, pw] = memory[5 + i * 3];
            let [nx, ny, nz, _] = memory[6 + i * 3];
            let [s, t, _, _] = memory[7 + i * 3];
            Vertex {
                position: [f(px), f(py), f(pz), f(pw)],
                normal: [f(nx), f(ny), f(nz)],
                st: [f(s), f(t)],
            }
        })
        .collect();
    Ok(Batch {
        tex0: [quad64(memory[1]), quad64(memory[2])],
        vertices,
    })
}

fn word(bytes: &[u8], offset: usize) -> io::Result<u32> {
    bytes
        .get(offset..offset + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid_data("VIF stream is truncated"))
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
