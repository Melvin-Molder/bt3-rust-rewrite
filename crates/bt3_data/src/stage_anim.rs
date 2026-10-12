//! The animated stage objects of a stage file (`StgModel_BindAnims`, member 6: the offset table at header word 9).
//! Three entries per animation: a texture file, a list of VU1 nodes (`Vu1Node`: a hierarchy of meshes, each with
//! its rest position / rotation, pivots and a VIF chain of program-8 batches) and their key tracks (`Vu1Track`,
//! one per node, one after the other), whose first's u16 at +0xC is the animation's length in frames.

use std::io;

use crate::model::walk_vif;

/// A program-8 batch: TEX0 as stored (`Vu1Node_OrFlags` adds the upload's blocks) and its strip.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimBatch {
    pub tex0: u64,
    pub vertices: Vec<AnimVertex>,
}

/// Position with the weight of the node's own matrix in w, colour (floats), s / t / 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimVertex {
    pub position: [f32; 4],
    pub color: [f32; 4],
    pub stq: [f32; 3],
}

/// `Vu1Node`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimNode {
    /// Matrices to drop from the stack after this node.
    pub pop: u32,
    pub pos: [f32; 4],
    pub rot: [f32; 4],
    pub pivot: [f32; 4],
    pub parent_pivot: [f32; 4],
    pub batches: Vec<AnimBatch>,
}

/// `Vu1Key`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimKey {
    /// The fourth word is the frame number's bits (`Vec4_Set` copies it along).
    pub pos: [f32; 4],
    pub frame: u32,
    pub rot: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageAnim {
    /// Byte offset of the texture file in the stage file (its offsets count from itself).
    pub textures: usize,
    pub nodes: Vec<AnimNode>,
    pub tracks: Vec<Vec<AnimKey>>,
    pub length: u16,
}

fn u32_at(b: &[u8], o: usize) -> io::Result<u32> {
    b.get(o..o + 4)
        .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
        .ok_or_else(|| invalid_data("stage animation data is truncated"))
}

fn f32x4(b: &[u8], o: usize) -> io::Result<[f32; 4]> {
    Ok([
        f32::from_bits(u32_at(b, o)?),
        f32::from_bits(u32_at(b, o + 4)?),
        f32::from_bits(u32_at(b, o + 8)?),
        f32::from_bits(u32_at(b, o + 12)?),
    ])
}

/// The stage file's animations; none when header words 9 and 10 are equal.
pub fn parse_stage_anims(file: &[u8]) -> io::Result<Vec<StageAnim>> {
    let (start, next) = (u32_at(file, 9 * 4)?, u32_at(file, 10 * 4)?);
    if start == next {
        return Ok(Vec::new());
    }
    let table = (start & !3) as usize;
    let count = u32_at(file, table + 8)? as usize;
    let entry = |i: usize| -> io::Result<usize> {
        Ok(table + u32_at(file, table + 0x10 + i * 4)? as usize)
    };
    (0..count / 3)
        .map(|a| {
            let textures = entry(a * 3)?;
            let list = entry(a * 3 + 1)?;
            let mut node_at = list + (u32_at(file, list)? & !3) as usize;
            let mut track_at = entry(a * 3 + 2)?;
            let length = file
                .get(track_at + 0xC..track_at + 0xE)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .ok_or_else(|| invalid_data("stage animation track is truncated"))?;
            let (mut nodes, mut tracks) = (Vec::new(), Vec::new());
            loop {
                let size = u32_at(file, node_at)? as usize;
                let last = u32_at(file, node_at + 8)? != 0;
                let mut batches = Vec::new();
                if u32_at(file, node_at + 0xC)? != 0 {
                    let end = if size > 0xF0 {
                        node_at + size
                    } else {
                        file.len()
                    };
                    let stream = file
                        .get(node_at + 0xF0..end.min(file.len()))
                        .ok_or_else(|| invalid_data("stage animation mesh is outside the file"))?;
                    walk_vif(stream, |m| {
                        let n = (m[2][0] & 0x7FFF) as usize;
                        let f = f32::from_bits;
                        let vertices = (0..n)
                            .filter_map(|i| {
                                let (p, c, t) =
                                    (m.get(3 + i * 3)?, m.get(4 + i * 3)?, m.get(5 + i * 3)?);
                                Some(AnimVertex {
                                    position: p.map(f),
                                    color: c.map(f),
                                    stq: [f(t[0]), f(t[1]), f(t[2])],
                                })
                            })
                            .collect();
                        batches.push(AnimBatch {
                            tex0: u64::from(m[1][0]) | (u64::from(m[1][1]) << 32),
                            vertices,
                        });
                        Ok(())
                    })?;
                }
                nodes.push(AnimNode {
                    pop: u32_at(file, node_at + 4)?,
                    pos: f32x4(file, node_at + 0x10)?,
                    rot: f32x4(file, node_at + 0x20)?,
                    pivot: f32x4(file, node_at + 0x30)?,
                    parent_pivot: f32x4(file, node_at + 0x40)?,
                    batches,
                });
                let keys = file
                    .get(track_at + 0xA..track_at + 0xC)
                    .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
                    .ok_or_else(|| invalid_data("stage animation track is truncated"))?;
                tracks.push(
                    (0..keys)
                        .map(|k| {
                            let o = track_at + 0x10 + k * 0x20;
                            Ok(AnimKey {
                                pos: f32x4(file, o)?,
                                frame: u32_at(file, o + 0xC)?,
                                rot: f32x4(file, o + 0x10)?,
                            })
                        })
                        .collect::<io::Result<_>>()?,
                );
                track_at += 0x10 + keys * 0x20;
                if last || size == 0 {
                    break;
                }
                node_at += size;
            }
            Ok(StageAnim {
                textures,
                nodes,
                tracks,
                length,
            })
        })
        .collect()
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
