//! The effect mesh of the effect and stage packs (`EftMeshData`, src/battle/eft_mesh.c): groups of triangles over a
//! vertex table, each triangle with a texture of the owner's set; and the stage's scrolling sheet (stage pack
//! entry 7, `EftStageScroll_Load`), a mesh behind four words.

use std::io;

/// `EftMeshVtx`: position, colour (128 = 1.0) and texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshVertex {
    pub pos: [f32; 4],
    pub color: [f32; 4],
    pub uv: [f32; 4],
}

/// `EftMeshTri`: three vertex indices and a texture index (< 0: untextured).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshTri {
    pub v: [usize; 3],
    pub tex: i32,
}

/// `EftMeshData`: the triangles of every group in draw order, and the vertices.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectMesh {
    pub tris: Vec<MeshTri>,
    pub verts: Vec<MeshVertex>,
}

impl EffectMesh {
    /// The groups (`numGroups` at +4, 16 bytes each from +0x10: count, -, first triangle, -), the triangle
    /// table at the offset at +8 and the vertex table at +0xC.
    pub fn parse(data: &[u8]) -> io::Result<Self> {
        let groups = count(read_i32(data, 4)?)?;
        let tri_at = offset(read_i32(data, 8)?)?;
        let vtx_at = offset(read_i32(data, 0xC)?)?;
        let mut tris = Vec::new();
        let mut max_vertex = 0;
        for g in 0..groups {
            let at = 0x10 + g * 0x10;
            let n = count(read_i32(data, at)?)?;
            let first = count(read_i32(data, at + 8)?)?;
            for k in first..first + n {
                let o = tri_at + k * 0x10;
                let v = [
                    count(read_i32(data, o)?)?,
                    count(read_i32(data, o + 4)?)?,
                    count(read_i32(data, o + 8)?)?,
                ];
                max_vertex = max_vertex.max(v[0].max(v[1]).max(v[2]) + 1);
                tris.push(MeshTri {
                    v,
                    tex: read_i32(data, o + 0xC)?,
                });
            }
        }
        let verts = (0..max_vertex)
            .map(|i| {
                let o = vtx_at + i * 0x30;
                let quad = |at: usize| -> io::Result<[f32; 4]> {
                    Ok([
                        read_f32(data, at)?,
                        read_f32(data, at + 4)?,
                        read_f32(data, at + 8)?,
                        read_f32(data, at + 12)?,
                    ])
                };
                Ok(MeshVertex {
                    pos: quad(o)?,
                    color: quad(o + 0x10)?,
                    uv: quad(o + 0x20)?,
                })
            })
            .collect::<io::Result<_>>()?;
        Ok(Self { tris, verts })
    }
}

/// Stage pack entry 7 (`EftStageScroll_Load`): the sheet's texture size, its scroll speed (in 1 / 100000 of the
/// size a frame), its direction in degrees (negative: random) and its mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct StageScroll {
    pub size: i32,
    pub speed: i32,
    pub angle: i32,
    pub mesh: EffectMesh,
}

impl StageScroll {
    pub fn parse(entry: &[u8]) -> io::Result<Self> {
        Ok(Self {
            size: read_i32(entry, 0)?,
            speed: read_i32(entry, 8)?,
            angle: read_i32(entry, 0xC)?,
            mesh: EffectMesh::parse(entry.get(0x10..).unwrap_or_default())?,
        })
    }
}

fn read_i32(data: &[u8], at: usize) -> io::Result<i32> {
    data.get(at..at + 4)
        .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "effect mesh is truncated"))
}

fn read_f32(data: &[u8], at: usize) -> io::Result<f32> {
    read_i32(data, at).map(|v| f32::from_bits(v as u32))
}

fn count(v: i32) -> io::Result<usize> {
    usize::try_from(v)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "negative mesh count"))
}

fn offset(v: i32) -> io::Result<usize> {
    count(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_groups_triangles_and_vertices() {
        let mut d = vec![0u8; 0x200];
        let put =
            |d: &mut Vec<u8>, at: usize, v: u32| d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        // Header of the sheet: size 256, speed 30, angle -1.
        put(&mut d, 0, 256);
        put(&mut d, 8, 30);
        put(&mut d, 0xC, -1i32 as u32);
        // The mesh at 0x10: one group of two triangles from triangle 1.
        let m = 0x10;
        put(&mut d, m + 4, 1);
        put(&mut d, m + 8, 0x40);
        put(&mut d, m + 0xC, 0x90);
        put(&mut d, m + 0x10, 2);
        put(&mut d, m + 0x18, 1);
        for (k, v) in [[9, 9, 9, 0], [0, 1, 2, 0], [2, 1, 3, 1]]
            .iter()
            .enumerate()
        {
            for (i, x) in v.iter().enumerate() {
                put(&mut d, m + 0x40 + k * 0x10 + i * 4, *x);
            }
        }
        put(&mut d, m + 0x90 + 3 * 0x30, 5.0f32.to_bits());
        put(&mut d, m + 0x90 + 3 * 0x30 + 0x20, 0.5f32.to_bits());
        let s = StageScroll::parse(&d).unwrap();
        assert_eq!((s.size, s.speed, s.angle), (256, 30, -1));
        assert_eq!(s.mesh.tris.len(), 2);
        assert_eq!(
            s.mesh.tris[1],
            MeshTri {
                v: [2, 1, 3],
                tex: 1
            }
        );
        assert_eq!(s.mesh.verts.len(), 4);
        assert_eq!(s.mesh.verts[3].pos[0], 5.0);
        assert_eq!(s.mesh.verts[3].uv[0], 0.5);
    }
}
