//! The stage's drawable model (the stage object, section 1 of a stage file `0x171 + stage`), as
//! `BtlStage_Relocate` lays it out and `StgModel_Cull` / `StgModel_Draw` read it. Every offset in the object is
//! in words from its start; its texture file counts its offsets from there too.

use std::collections::HashSet;
use std::io;

use crate::model::walk_vif;

/// `StgMModel.grp[]`: back (no depth writes), back 2, main, three blended groups, debris.
pub const GROUP_COUNT: usize = 7;
pub const GROUP_MAIN: usize = 2;
pub const GROUP_DEBRIS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageVertex {
    pub position: [f32; 4],
    /// GS colour as floats: 128 is 1.0 for MODULATE.
    pub color: [f32; 4],
    pub stq: [f32; 3],
}

/// One `StgMMesh`: its byte in the visibility table, the VU1 program group 2 checks, and the strips its chain
/// hands to program 4 (one per batch after the object header).
#[derive(Clone, Debug, PartialEq)]
pub struct StageMesh {
    pub vis: u16,
    pub prog: u8,
    pub strips: Vec<Vec<StageVertex>>,
}

/// `StgMTexAnim`.
#[derive(Clone, Debug, PartialEq)]
pub struct TexAnim {
    /// Bit 2: animated; bit 0: loop (otherwise back and forth).
    pub flags: u16,
    pub frames: Vec<i32>,
}

/// `StgMMat`: the meshes that share a texture of the stage texture file.
#[derive(Clone, Debug, PartialEq)]
pub struct StageMaterial {
    pub tex: u16,
    pub meshes: Vec<StageMesh>,
    pub anim: Option<TexAnim>,
    /// Current texture of the animation; bit 15 = going back.
    pub cur: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageModel {
    pub groups: Vec<Vec<StageMaterial>>,
    /// Per visibility byte: whether a part that holds the mesh can be drawn after `BtlStage_Reset` (the
    /// never-culled list, or a part of the culling tree with flag bit 0).
    pub drawable: Vec<bool>,
    /// Byte offset of the texture file header from the stage object (`texOfs`).
    pub texture_header: usize,
}

impl StageModel {
    /// The stage object (`StgModel_InitStage`: the block at header word 1 of the stage file); its offsets reach
    /// past that section, so this runs to the end of the file.
    pub fn object(file: &[u8]) -> io::Result<&[u8]> {
        let start = Reader(file).u32(4)? as usize & !3;
        file.get(start..)
            .ok_or_else(|| invalid_data("stage object outside the file"))
    }

    pub fn parse(s: &[u8]) -> io::Result<Self> {
        let r = Reader(s);
        let mut groups = Vec::with_capacity(GROUP_COUNT);
        let group_count = r.u32(0x18)? as usize;
        let groups_at = r.ptr(0x1C)?;
        for g in 0..group_count.min(GROUP_COUNT) {
            let group = groups_at + g * 8;
            let count = usize::from(r.u16(group + 2)?);
            let mats = r.ptr(group + 4)?;
            let mut materials = Vec::with_capacity(count);
            for m in 0..count {
                let mat = mats + m * 0x10;
                let n = usize::from(r.u16(mat + 2)?);
                let list = r.ptr(mat + 4)?;
                let anim = match r.u32(mat + 8)? {
                    0 => None,
                    words => {
                        let at = words as usize * 4;
                        let frames = r.ptr(at + 4)?;
                        Some(TexAnim {
                            flags: r.u16(at)?,
                            frames: (0..usize::from(r.u16(at + 2)?))
                                .map(|i| r.u32(frames + i * 4).map(|v| v as i32))
                                .collect::<io::Result<_>>()?,
                        })
                    }
                };
                let meshes = (0..n)
                    .map(|k| {
                        let mesh = list + k * 0x10;
                        Ok(StageMesh {
                            vis: r.u16(mesh)?,
                            prog: r.u8(mesh + 3)?,
                            strips: parse_chain(s.get(r.ptr(mesh + 8)?..).unwrap_or(&[]))?,
                        })
                    })
                    .collect::<io::Result<_>>()?;
                materials.push(StageMaterial {
                    tex: r.u16(mat)?,
                    meshes,
                    anim,
                    cur: r.u16(mat + 0xC)?,
                });
            }
            groups.push(materials);
        }
        let drawable = drawable_meshes(&r)?;
        Ok(Self {
            groups,
            drawable,
            texture_header: r.ptr(0x54)?,
        })
    }
}

/// The program 4 strips of a mesh chain: the first MSCNT runs `object_header`, every later one a batch (A+D tag
/// and data at +0/+1, the strip tag at +2, vertices of position, colour and s/t/q from +3).
fn parse_chain(chain: &[u8]) -> io::Result<Vec<Vec<StageVertex>>> {
    let mut strips = Vec::new();
    let mut header = true;
    walk_vif(chain, |memory| {
        if std::mem::take(&mut header) {
            return Ok(());
        }
        let n = (memory[2][0] & 0x7FFF) as usize;
        if 3 + n * 3 > memory.len() {
            return Err(invalid_data("stage batch exceeds VU1 memory"));
        }
        let f = |q: [u32; 4]| q.map(f32::from_bits);
        strips.push(
            (0..n)
                .map(|i| {
                    let stq = f(memory[5 + i * 3]);
                    StageVertex {
                        position: f(memory[3 + i * 3]),
                        color: f(memory[4 + i * 3]),
                        stq: [stq[0], stq[1], stq[2]],
                    }
                })
                .collect(),
        );
        Ok(())
    })?;
    Ok(strips)
}

/// `BtlStage_ResetObjs(0)` then what `StgModel_Cull` can reach: every object's whole-model parts (0, 2, 4) get
/// flag bit 0 and its broken-model parts (1, 3, 5) lose it; the culling tree draws the parts with the bit, the
/// first never-culled list is drawn whatever the flags. Debris is only drawn while it falls.
fn drawable_meshes(r: &Reader) -> io::Result<Vec<bool>> {
    let mut flagged = HashSet::new();
    let mut cleared = HashSet::new();
    let objs = r.ptr(0x48)?;
    for i in 0..r.u32(0x44)? as usize {
        for k in 0..6 {
            let part = r.u32(objs + i * 0x50 + 0xC + k * 4)? as usize * 4;
            if part != 0 {
                if k % 2 == 0 {
                    flagged.insert(part);
                } else {
                    cleared.insert(part);
                }
            }
        }
    }
    let has_flag = |part: usize| -> io::Result<bool> {
        Ok(flagged.contains(&part) || (!cleared.contains(&part) && r.u16(part + 2)? & 1 != 0))
    };
    let mut drawable = vec![false; r.u32(0x28)? as usize];
    let mut mark = |part: usize| -> io::Result<()> {
        let list = r.ptr(part + 4)?;
        for j in 0..usize::from(r.u16(part)?) {
            let vis = usize::from(r.u16(r.ptr(list + j * 4)?)?);
            if let Some(slot) = drawable.get_mut(vis) {
                *slot = true;
            }
        }
        Ok(())
    };
    if r.u32(0x20)? != 0 {
        let always = r.ptr(0x24)?;
        let items = r.ptr(always + 4)?;
        for i in 0..r.u32(always)? as usize {
            mark(items + i * 0x20)?;
        }
    }
    let mut cells = vec![r.ptr(0x50)?];
    while let Some(cell) = cells.pop() {
        let list = r.u32(cell + 0x3C)? as usize * 4;
        for i in 0..r.u32(cell + 0x38)? as usize {
            let entry = r.u32(list + i * 4)?;
            if entry != 0 && entry != u32::MAX && has_flag(entry as usize * 4)? {
                mark(entry as usize * 4)?;
            }
        }
        for c in 0..8 {
            let child = r.u32(cell + 4 + c * 4)?;
            if child != 0 {
                cells.push(child as usize * 4);
            }
        }
    }
    Ok(drawable)
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn u8(&self, o: usize) -> io::Result<u8> {
        self.0
            .get(o)
            .copied()
            .ok_or_else(|| invalid_data("stage object is truncated"))
    }

    fn u16(&self, o: usize) -> io::Result<u16> {
        Ok(u16::from_le_bytes([self.u8(o)?, self.u8(o + 1)?]))
    }

    fn u32(&self, o: usize) -> io::Result<u32> {
        self.0
            .get(o..o + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| invalid_data("stage object is truncated"))
    }

    /// A word offset turned into a byte offset.
    fn ptr(&self, o: usize) -> io::Result<usize> {
        Ok(self.u32(o)? as usize * 4)
    }
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Iso9660;
    use crate::afs::{AfsArchive, partition_entry_for_id};
    use crate::gs::decode_sprite_texture;
    use crate::tex::TextureResource;

    #[test]
    fn every_stage_file_parses_and_its_textures_decode() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&path) else {
            return;
        };
        for file_id in 0x171..0x198 {
            let location = partition_entry_for_id(file_id);
            let archive =
                AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition))
                    .unwrap();
            let file = archive.read_entry(&mut image, location.index).unwrap();
            let object = StageModel::object(&file).unwrap();
            let model = StageModel::parse(object)
                .unwrap_or_else(|e| panic!("stage file {file_id:#X}: {e}"));
            assert!(model.drawable.iter().any(|&d| d), "stage file {file_id:#X}");
            let textures = TextureResource::parse_at(object, model.texture_header).unwrap();
            for (i, entry) in textures.entries.iter().enumerate() {
                decode_sprite_texture(entry)
                    .unwrap_or_else(|e| panic!("stage file {file_id:#X} texture {i}: {e}"));
            }
            for material in model.groups.iter().flatten() {
                assert!(usize::from(material.tex) < textures.entries.len());
            }
        }
    }
}
