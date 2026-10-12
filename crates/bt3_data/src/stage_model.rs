//! The stage's drawable model (the stage object, section 1 of a stage file `0x171 + stage`), as
//! `BtlStage_Relocate` lays it out and `StgModel_Cull` / `StgModel_Draw` read it. Every offset in the object is
//! in words from its start; its texture file counts its offsets from there too.

use std::collections::HashMap;
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

/// One `StgMMesh`: its byte in the visibility table, the VU1 program group 2 checks, the strips its chain hands
/// to program 4 or 7 (one per batch after the object header), and for a debris mesh the node whose matrix it is
/// drawn with (index into `StageModel::nodes`).
#[derive(Clone, Debug, PartialEq)]
pub struct StageMesh {
    pub vis: u16,
    pub prog: u8,
    pub strips: Vec<Vec<StageVertex>>,
    pub node: Option<usize>,
}

/// `StgKey`: a debris piece's key (position, frame, angles in degrees).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageKey {
    pub pos: [f32; 3],
    pub frame: i32,
    pub rot: [f32; 3],
}

/// `StgNode`: the transform node of a debris piece: its matrix as the file holds it, its keys (a single key: a
/// rigid body drives it, `StgNode_IsRigid`) and the frame of its last key.
#[derive(Clone, Debug, PartialEq)]
pub struct StageNode {
    pub mtx: [[f32; 4]; 4],
    pub keys: Vec<StageKey>,
    pub end_frame: i32,
}

impl StageNode {
    /// `StgNode_IsRigid`.
    pub fn is_rigid(&self) -> bool {
        self.keys.len() == 1
    }
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

/// `StgPart`: a group of meshes (by visibility byte) with its runtime flag bit 0 (drawn by the culling tree), its
/// bounding sphere and its transform node (index into `StageModel::nodes`).
#[derive(Clone, Debug, PartialEq)]
pub struct StagePart {
    pub vis: Vec<u16>,
    pub on: bool,
    pub radius: f32,
    pub center: [f32; 3],
    pub node: Option<usize>,
}

/// `StgObj` of the model: a destructible object's parts (0, 2, 4 the whole model, 1, 3, 5 the broken one; indices
/// into `parts`), its debris pieces and the length of their animation (`animEnd`, 0x94 frames when 0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageObject {
    pub parts: [Option<usize>; 6],
    pub pieces: Vec<usize>,
    pub anim_end: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageModel {
    pub groups: Vec<Vec<StageMaterial>>,
    /// Per visibility byte: whether a part that holds the mesh can be drawn (the never-culled list, or a part of
    /// the culling tree with flag bit 0); `refresh_drawable` rebuilds it from the parts' flags.
    pub drawable: Vec<bool>,
    /// Byte offset of the texture file header from the stage object (`texOfs`).
    pub texture_header: usize,
    pub parts: Vec<StagePart>,
    pub objects: Vec<StageObject>,
    pub nodes: Vec<StageNode>,
    /// The meshes of the never-culled list.
    always: Vec<u16>,
    /// The parts of the culling tree.
    tree: Vec<usize>,
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
        let mut nodes = Vec::new();
        let mut node_index = HashMap::new();
        let mut node_at = |node: usize| -> io::Result<Option<usize>> {
            if node == 0 {
                return Ok(None);
            }
            if let Some(&i) = node_index.get(&node) {
                return Ok(Some(i));
            }
            let mtx = std::array::from_fn(|i| {
                std::array::from_fn(|j| r.f32(node + (i * 4 + j) * 4).unwrap_or(0.0))
            });
            let count = r.u32(node + 0x40)? as usize;
            let keys_at = r.ptr(node + 0x44)?;
            let keys = (0..count)
                .map(|k| {
                    let o = keys_at + k * 0x20;
                    Ok(StageKey {
                        pos: [r.f32(o)?, r.f32(o + 4)?, r.f32(o + 8)?],
                        frame: r.u32(o + 0xC)? as i32,
                        rot: [r.f32(o + 0x10)?, r.f32(o + 0x14)?, r.f32(o + 0x18)?],
                    })
                })
                .collect::<io::Result<_>>()?;
            nodes.push(StageNode {
                mtx,
                keys,
                end_frame: r.u32(node + 0x48)? as i32,
            });
            node_index.insert(node, nodes.len() - 1);
            Ok(Some(nodes.len() - 1))
        };
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
                        let node = if g == GROUP_DEBRIS {
                            node_at(r.ptr(mesh + 0xC)?)?
                        } else {
                            None
                        };
                        Ok(StageMesh {
                            vis: r.u16(mesh)?,
                            prog: r.u8(mesh + 3)?,
                            strips: parse_chain(
                                s.get(r.ptr(mesh + 8)?..).unwrap_or(&[]),
                                g != GROUP_DEBRIS,
                            )?,
                            node,
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
        let mut parts = Vec::new();
        let mut index = HashMap::new();
        let mut part_at = |part: usize| -> io::Result<usize> {
            if let Some(&i) = index.get(&part) {
                return Ok(i);
            }
            let list = r.ptr(part + 4)?;
            let vis = (0..usize::from(r.u16(part)?))
                .map(|j| r.u16(r.ptr(list + j * 4)?))
                .collect::<io::Result<_>>()?;
            parts.push(StagePart {
                vis,
                on: r.u16(part + 2)? & 1 != 0,
                radius: r.f32(part + 0xC)?,
                center: [
                    r.f32(part + 0x10)?,
                    r.f32(part + 0x14)?,
                    r.f32(part + 0x18)?,
                ],
                node: node_at(r.ptr(part + 8)?)?,
            });
            index.insert(part, parts.len() - 1);
            Ok(parts.len() - 1)
        };
        let mut always = Vec::new();
        if r.u32(0x20)? != 0 {
            let list = r.ptr(0x24)?;
            let items = r.ptr(list + 4)?;
            for i in 0..r.u32(list)? as usize {
                let part = items + i * 0x20;
                let list = r.ptr(part + 4)?;
                for j in 0..usize::from(r.u16(part)?) {
                    always.push(r.u16(r.ptr(list + j * 4)?)?);
                }
            }
        }
        let mut tree = Vec::new();
        let mut cells = vec![r.ptr(0x50)?];
        while let Some(cell) = cells.pop() {
            let list = r.ptr(cell + 0x3C)?;
            for i in 0..r.u32(cell + 0x38)? as usize {
                let entry = r.u32(list + i * 4)?;
                if entry != 0 && entry != u32::MAX {
                    let p = part_at(entry as usize * 4)?;
                    if !tree.contains(&p) {
                        tree.push(p);
                    }
                }
            }
            for c in 0..8 {
                let child = r.u32(cell + 4 + c * 4)?;
                if child != 0 {
                    cells.push(child as usize * 4);
                }
            }
        }
        let objs = r.ptr(0x48)?;
        let objects = (0..r.u32(0x44)? as usize)
            .map(|i| {
                let o = objs + i * 0x50;
                let mut obj = StageObject {
                    anim_end: r.u32(o + 0x2C)? as i32,
                    ..StageObject::default()
                };
                for k in 0..6 {
                    let part = r.ptr(o + 0xC + k * 4)?;
                    if part != 0 {
                        obj.parts[k] = Some(part_at(part)?);
                    }
                }
                let pieces = r.ptr(o + 0x28)?;
                for j in 0..r.u32(o + 0x24)? as usize {
                    let piece = r.ptr(pieces + j * 4)?;
                    obj.pieces.push(part_at(piece)?);
                }
                Ok(obj)
            })
            .collect::<io::Result<_>>()?;
        let mut model = Self {
            groups,
            drawable: vec![false; r.u32(0x28)? as usize],
            texture_header: r.ptr(0x54)?,
            parts,
            objects,
            nodes,
            always,
            tree,
        };
        for i in 0..model.objects.len() {
            model.set_broken(i, false);
        }
        model.refresh_drawable();
        Ok(model)
    }

    /// `BtlStage_BreakObj` / `BtlStage_ResetObjs`' part flags: a broken object draws its broken model (parts 1, 3,
    /// 5), a whole one its whole model (0, 2, 4).
    pub fn set_broken(&mut self, obj: usize, broken: bool) {
        let Some(o) = self.objects.get(obj) else {
            return;
        };
        for (k, part) in o.parts.iter().enumerate() {
            if let Some(&p) = part.as_ref() {
                self.parts[p].on = (k % 2 == 1) == broken;
            }
        }
    }

    /// What `StgModel_Cull` can reach: the never-culled list whatever the flags, and the parts of the culling tree
    /// with flag bit 0. Debris is only drawn while it falls.
    pub fn refresh_drawable(&mut self) {
        self.drawable.fill(false);
        let mut mark = |vis: u16| {
            if let Some(slot) = self.drawable.get_mut(usize::from(vis)) {
                *slot = true;
            }
        };
        for &vis in &self.always {
            mark(vis);
        }
        for &p in &self.tree {
            if self.parts[p].on {
                for &vis in &self.parts[p].vis {
                    mark(vis);
                }
            }
        }
    }
}

/// The strips of a mesh chain: for program 4 the first MSCNT runs `object_header` (`header`), every other one a
/// batch (A+D tag and data at +0/+1, the strip tag at +2, vertices of position, colour and s/t/q from +3);
/// program 7's call packet runs its setup itself, so every MSCNT of a debris chain is a batch.
fn parse_chain(chain: &[u8], header: bool) -> io::Result<Vec<Vec<StageVertex>>> {
    let mut strips = Vec::new();
    let mut header = header;
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

    fn f32(&self, o: usize) -> io::Result<f32> {
        Ok(f32::from_bits(self.u32(o)?))
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
            // The model's objects have definitions in the stage data (which may list more); breaking them swaps
            // meshes in or out.
            let data = crate::stage::StageResource::parse(&file).unwrap();
            assert!(
                model.objects.len() <= data.object_count,
                "stage file {file_id:#X}"
            );
            // Every debris mesh has strips and hangs on a node with keys, as every object's piece does.
            for mesh in model
                .groups
                .get(GROUP_DEBRIS)
                .into_iter()
                .flatten()
                .flat_map(|m| &m.meshes)
            {
                assert!(!mesh.strips.is_empty(), "stage file {file_id:#X}");
                let node = mesh.node.expect("a debris mesh has a node");
                assert!(
                    !model.nodes[node].keys.is_empty(),
                    "stage file {file_id:#X}"
                );
            }
            for o in &model.objects {
                for &p in &o.pieces {
                    let node = model.parts[p].node.expect("a piece has a node");
                    assert!(
                        !model.nodes[node].keys.is_empty(),
                        "stage file {file_id:#X}"
                    );
                }
            }
            let mut broken = model.clone();
            for i in 0..broken.objects.len() {
                broken.set_broken(i, true);
            }
            broken.refresh_drawable();
            assert_eq!(
                broken.drawable != model.drawable,
                !model.objects.is_empty(),
                "stage file {file_id:#X}"
            );
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
