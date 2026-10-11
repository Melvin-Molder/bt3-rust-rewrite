//! Stage collision (src/battle/stg_collision.c, col_box.c, the ground probe of eft_detect.c, the zone lookups of
//! stg.c): the stage is cut into zones, rectangles on the ground plane with a neighbour list; each zone has a
//! collision mesh for its static geometry and records for the destructible objects standing in it (the whole
//! object's mesh, used while it stands, and its remains). A mesh is a box tree over triangles.
//!
//! Offsets in the stage header (file word 1) count words from the header (`BtlStage_Relocate`, and
//! `ColMesh_SetBase` with the same base). Positions are world positions, +Y down.

use std::io;

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// `ColNode`: a box; an inner node's children, or a leaf's polygon (`right == -1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColNode {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub left: i32,
    pub right: i32,
}

/// `ColPoly`: a triangle and its plane (n . p = w).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColPoly {
    pub flags: u32,
    pub vtx: [u32; 3],
    pub nrm: [f32; 4],
}

/// `STGCOL_POLY_NO_SOLID`: skipped by every query.
pub const POLY_NO_SOLID: u32 = 0x0800_0000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColMesh {
    pub nodes: Vec<ColNode>,
    pub polys: Vec<ColPoly>,
    pub vertices: Vec<[f32; 3]>,
}

/// An axis-aligned box (`ColBox`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl ColBox {
    /// `ColBox_SetCenterHalf`.
    pub fn from_center_half(c: [f32; 3], h: [f32; 3]) -> Self {
        Self {
            min: [c[0] - h[0], c[1] - h[1], c[2] - h[2]],
            max: [c[0] + h[0], c[1] + h[1], c[2] + h[2]],
        }
    }

    /// `ColBox_Overlaps` (touching counts).
    pub fn overlaps(&self, o: &ColBox) -> bool {
        (0..3).all(|i| !(self.min[i] > o.max[i]) && !(self.max[i] < o.min[i]))
    }
}

impl ColMesh {
    /// `ColMesh_WalkBox`: the callback for every leaf whose box overlaps `b`, depth first, left first; the OR of
    /// the results.
    pub fn walk_box(
        &self,
        b: &ColBox,
        cb: &mut dyn FnMut(&ColPoly, [[f32; 3]; 3]) -> bool,
    ) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let mut hit = false;
        self.walk_node(0, b, cb, &mut hit);
        hit
    }

    fn walk_node(
        &self,
        idx: i32,
        b: &ColBox,
        cb: &mut dyn FnMut(&ColPoly, [[f32; 3]; 3]) -> bool,
        hit: &mut bool,
    ) {
        let Some(node) = usize::try_from(idx).ok().and_then(|i| self.nodes.get(i)) else {
            return;
        };
        let nb = ColBox {
            min: node.min,
            max: node.max,
        };
        if !b.overlaps(&nb) {
            return;
        }
        if node.right == -1 {
            if let Some(poly) = usize::try_from(node.left)
                .ok()
                .and_then(|i| self.polys.get(i))
            {
                let v = poly
                    .vtx
                    .map(|i| self.vertices.get(i as usize).copied().unwrap_or([0.0; 3]));
                *hit |= cb(poly, v);
            }
            return;
        }
        self.walk_node(node.left, b, cb, hit);
        self.walk_node(node.right, b, cb, hit);
    }
}

/// `StgColRec`: an object's mesh in a zone.
#[derive(Clone, Debug, PartialEq)]
pub struct ColRec {
    /// The remains of the object (used once it is broken) rather than the whole object.
    pub broken: bool,
    pub obj: usize,
    pub mesh: ColMesh,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColZone {
    pub center: [f32; 3],
    pub half: [f32; 3],
    pub near: Vec<i32>,
    pub mesh: ColMesh,
    pub recs: Vec<ColRec>,
}

impl ColZone {
    /// `STG_ZONE_OUTSIDE` negated: (x, z) in the rectangle, borders included.
    fn contains(&self, p: [f32; 3]) -> bool {
        let (min_x, min_z) = (self.center[0] - self.half[0], self.center[2] - self.half[2]);
        let (max_x, max_z) = (self.center[0] + self.half[0], self.center[2] + self.half[2]);
        !(max_x < p[0] || max_z < p[2] || p[0] < min_x || p[2] < min_z)
    }
}

/// What the ground probe found (`StgGroundOut`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    /// The ground height, `f32::MAX` when there is none.
    pub y: f32,
    pub nrm: [f32; 4],
    pub flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageCollision {
    pub zones: Vec<ColZone>,
}

impl StageCollision {
    /// Reads the zone table of a stage file.
    pub fn parse(file: &[u8]) -> io::Result<Self> {
        let word = |o: usize| -> io::Result<u32> {
            file.get(o..o + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| invalid_data("stage collision is truncated"))
        };
        let float = |o: usize| word(o).map(f32::from_bits);
        let base = (word(4)? & !3) as usize;
        let at = |words: u32| base + words as usize * 4;
        let mesh = |o: usize| -> io::Result<ColMesh> {
            let count = word(o)? as i32;
            // Some corner zones of stage 0 have nodes but a node offset of -1: they lie beyond the stage radius
            // fighters and cameras are clamped to, so the original never walks them.
            if count <= 0 || word(o + 0xC)? == u32::MAX {
                return Ok(ColMesh::default());
            }
            let (nodes_at, polys_at, vtx_at) =
                (at(word(o + 0xC)?), at(word(o + 0x10)?), at(word(o + 0x14)?));
            let nodes = (0..count as usize)
                .map(|i| {
                    let n = nodes_at + i * 0x20;
                    Ok(ColNode {
                        min: [float(n)?, float(n + 4)?, float(n + 8)?],
                        max: [float(n + 0xC)?, float(n + 0x10)?, float(n + 0x14)?],
                        left: word(n + 0x18)? as i32,
                        right: word(n + 0x1C)? as i32,
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            // The polygon and vertex counts are not stored: take what the leaves name.
            let poly_count = nodes
                .iter()
                .filter(|n| n.right == -1 && n.left >= 0)
                .map(|n| n.left as usize + 1)
                .max()
                .unwrap_or(0);
            let polys = (0..poly_count)
                .map(|i| {
                    let p = polys_at + i * 0x20;
                    Ok(ColPoly {
                        flags: word(p)?,
                        vtx: [word(p + 4)?, word(p + 8)?, word(p + 0xC)?],
                        nrm: [
                            float(p + 0x10)?,
                            float(p + 0x14)?,
                            float(p + 0x18)?,
                            float(p + 0x1C)?,
                        ],
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            let vtx_count = polys
                .iter()
                .flat_map(|p| p.vtx)
                .map(|v| v as usize + 1)
                .max()
                .unwrap_or(0);
            let vertices = (0..vtx_count)
                .map(|i| {
                    let v = vtx_at + i * 0x10;
                    Ok([float(v)?, float(v + 4)?, float(v + 8)?])
                })
                .collect::<io::Result<Vec<_>>>()?;
            Ok(ColMesh {
                nodes,
                polys,
                vertices,
            })
        };
        let count = word(base + 0x38)? as usize;
        let table = at(word(base + 0x3C)?);
        let zones = (0..count)
            .map(|z| {
                let zo = table + z * 0x40;
                let near_count = word(zo + 8)? as usize;
                let near_at = at(word(zo + 0xC)?);
                let near = (0..near_count)
                    .map(|i| word(near_at + i * 4).map(|v| v as i32))
                    .collect::<io::Result<_>>()?;
                let rec_count = word(zo + 0x14)? as usize;
                let recs_at = at(word(zo + 0x18)?);
                let recs = (0..rec_count)
                    .map(|i| {
                        let r = recs_at + i * 0x10;
                        Ok(ColRec {
                            broken: word(r)? != 0,
                            obj: word(r + 4)? as usize,
                            mesh: mesh(at(word(r + 8)?))?,
                        })
                    })
                    .collect::<io::Result<_>>()?;
                Ok(ColZone {
                    center: [float(zo + 0x20)?, float(zo + 0x24)?, float(zo + 0x28)?],
                    half: [float(zo + 0x30)?, float(zo + 0x34)?, float(zo + 0x38)?],
                    near,
                    mesh: mesh(at(word(zo + 4)?))?,
                    recs,
                })
            })
            .collect::<io::Result<_>>()?;
        Ok(Self { zones })
    }

    /// `BtlStage_FindZone`: the first zone containing (x, z).
    pub fn find_zone(&self, p: [f32; 3]) -> Option<usize> {
        self.zones.iter().position(|z| z.contains(p))
    }

    /// `BtlStage_FindZoneNear`: `zone` and its neighbours first, then all; 0 when none contains the point.
    pub fn find_zone_near(&self, zone: i32, p: [f32; 3]) -> usize {
        if let Some(z) = usize::try_from(zone).ok().and_then(|i| self.zones.get(i)) {
            if z.contains(p) {
                return zone as usize;
            }
            for &n in &z.near {
                if self.zones.get(n as usize).is_some_and(|nz| nz.contains(p)) {
                    return n as usize;
                }
            }
        }
        self.find_zone(p).unwrap_or(0)
    }

    /// `StgCol_QueryZone` with every object whole (`BtlStage_Reset`; nothing breaks yet): the zone's static mesh
    /// and the whole objects' meshes, in that order.
    pub fn query_zone(
        &self,
        zone: usize,
        b: &ColBox,
        cb: &mut dyn FnMut(&ColPoly, [[f32; 3]; 3]) -> bool,
    ) -> bool {
        let Some(z) = self.zones.get(zone) else {
            return false;
        };
        let mut hit = z.mesh.walk_box(b, cb);
        for rec in z.recs.iter().filter(|r| !r.broken) {
            hit |= rec.mesh.walk_box(b, cb);
        }
        hit
    }

    /// `StgGround_Probe`: of the solid upward triangles under (x, z) in the zone near `zone`, the nearest one below
    /// `min_y` (the smallest plane height greater than `min_y`). `b` is the search box, stretched 8000 down.
    pub fn ground_probe(&self, zone: i32, b: &ColBox, x: f32, z: f32, min_y: f32) -> Ground {
        let mut out = Ground {
            y: f32::MAX,
            nrm: [0.0; 4],
            flags: 0,
        };
        // StgGround_ExtendBoxDown keeps the top.
        let half_y = ((b.max[1] - b.min[1]) * 0.5 * 2.0 + 8000.0) * 0.5;
        let center = [
            (b.max[0] + b.min[0]) * 0.5,
            b.min[1] + half_y,
            (b.max[2] + b.min[2]) * 0.5,
        ];
        let half = [
            (b.max[0] - b.min[0]) * 0.5,
            half_y,
            (b.max[2] - b.min[2]) * 0.5,
        ];
        let search = ColBox::from_center_half(center, half);
        let box_center = [
            (b.max[0] + b.min[0]) * 0.5,
            (b.max[1] + b.min[1]) * 0.5,
            (b.max[2] + b.min[2]) * 0.5,
        ];
        let zone = self.find_zone_near(zone, box_center);
        self.query_zone(zone, &search, &mut |poly, v| {
            // StgGround_Cb: solid, facing up (+Y is down).
            if poly.flags & POLY_NO_SOLID != 0 || !(poly.nrm[1] < 0.0) {
                return false;
            }
            ground_test_tri(&mut out, x, z, &v, poly, min_y)
        });
        out
    }
}

/// `StgGround_TestTri`: (x, z) inside the triangle seen from above, and its plane's height there between `min_y`
/// and the best so far.
fn ground_test_tri(
    out: &mut Ground,
    px: f32,
    pz: f32,
    v: &[[f32; 3]; 3],
    poly: &ColPoly,
    min_y: f32,
) -> bool {
    let [n0, n1, n2, w] = poly.nrm;
    let (x0, z0) = (v[0][0], v[0][2]);
    let (x1, z1) = (v[1][0], v[1][2]);
    let (x2, z2) = (v[2][0], v[2][2]);
    let y = (w - n0 * px - n2 * pz) / n1;
    let inside = (x2 - x0) * (pz - z0) - (z2 - z0) * (px - x0) <= 0.001
        && (x1 - x2) * (pz - z2) - (z1 - z2) * (px - x2) <= 0.001
        && (x0 - x1) * (pz - z1) - (z0 - z1) * (px - x1) <= 0.001;
    if min_y < y && y < out.y && inside {
        out.y = y;
        out.flags = poly.flags;
        out.nrm = poly.nrm;
        return true;
    }
    false
}

/// `StgColHit` of a segment trace: where the stage blocks it and the unbroken object hit (-1: static geometry).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegHit {
    pub pos: [f32; 3],
    pub obj: i32,
}

/// `StgColCtx` of a segment trace.
struct SegCtx {
    a: [f32; 3],
    b: [f32; 3],
    bounds: ColBox,
    origin: [f32; 3],
    dir: [f32; 3],
    dist: f32,
    hit: bool,
    pos: [f32; 3],
    obj: i32,
}

/// `StgAabb_SetEmpty`.
fn empty_box() -> ColBox {
    ColBox {
        min: [f32::MAX; 3],
        max: [-f32::MAX; 3],
    }
}

/// `ColBox_AddPoint`.
fn add_point(b: &mut ColBox, p: [f32; 3]) {
    for i in 0..3 {
        if p[i] < b.min[i] {
            b.min[i] = p[i];
        }
        if b.max[i] < p[i] {
            b.max[i] = p[i];
        }
    }
}

/// `ColBox_ContainsPoint`, faces included.
fn contains_point(b: &ColBox, p: [f32; 3]) -> bool {
    (0..3).all(|i| !(b.max[i] < p[i]) && !(p[i] < b.min[i]))
}

/// `ColBox_ClipRay`: where the ray enters the box (its origin when inside). Like the original, `out` is written
/// on the way even when the ray misses.
fn clip_ray(b: &ColBox, origin: [f32; 3], dir: [f32; 3], out: &mut [f32; 3]) -> bool {
    let mut side = [2u8; 3];
    let mut plane = [0.0f32; 3];
    let mut inside = true;
    for i in 0..3 {
        if origin[i] < b.min[i] {
            side[i] = 1;
            plane[i] = b.min[i];
            inside = false;
        } else if b.max[i] < origin[i] {
            side[i] = 0;
            plane[i] = b.max[i];
            inside = false;
        }
    }
    if inside {
        *out = origin;
        return true;
    }
    let t: [f32; 3] = std::array::from_fn(|i| {
        if side[i] != 2 && dir[i] != 0.0 {
            (plane[i] - origin[i]) / dir[i]
        } else {
            -1.0
        }
    });
    let mut best = 0;
    for i in 1..3 {
        if t[best] < t[i] {
            best = i;
        }
    }
    if t[best] < 0.0 {
        return false;
    }
    for i in 0..3 {
        if best != i {
            out[i] = origin[i] + t[best] * dir[i];
            if out[i] < b.min[i] && (b.min[i] - out[i]).abs() > 0.001 {
                return false;
            }
            if out[i] > b.max[i] && (out[i] - b.max[i]).abs() > 0.001 {
                return false;
            }
        } else {
            out[i] = plane[i];
        }
    }
    true
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `ColSeg_TestTri`: segment a..b against the front side of the triangle; the crossing point and its fraction.
fn seg_test_tri(a: [f32; 3], b: [f32; 3], v: &[[f32; 3]; 3]) -> Option<([f32; 3], f32)> {
    let e1 = sub(v[1], v[0]);
    let e2 = sub(v[2], v[0]);
    let d = sub(a, b);
    let n = cross(e1, e2);
    let det = dot(d, n);
    if det <= 0.0 {
        return None;
    }
    let w = sub(a, v[0]);
    let dist = dot(w, n);
    if dist < 0.0 || dist > det {
        return None;
    }
    let c = cross(d, w);
    let u = dot(e2, c);
    if u < 0.0 || u > det {
        return None;
    }
    let vv = -dot(e1, c);
    if vv < 0.0 || u + vv > det {
        return None;
    }
    let inv = 1.0 / det;
    let (u, vv) = (u * inv, vv * inv);
    let k = 1.0 - u - vv;
    let pos = std::array::from_fn(|i| v[0][i] * k + v[1][i] * u + v[2][i] * vv);
    Some((pos, dist * inv))
}

impl StageCollision {
    /// The zone's rectangle as a box from y -10000 to 10000 (`StgCol_GetZoneBox`).
    fn zone_box(&self, zone: usize) -> ColBox {
        let z = &self.zones[zone];
        let mut b = empty_box();
        add_point(
            &mut b,
            [z.center[0] - z.half[0], -10000.0, z.center[2] - z.half[2]],
        );
        add_point(
            &mut b,
            [z.center[0] + z.half[0], 10000.0, z.center[2] + z.half[2]],
        );
        b
    }

    /// `StgCol_TraceSegment`: does the stage block the segment a -> b? Every object counts as standing.
    pub fn trace_segment(&self, a: [f32; 3], b: [f32; 3]) -> Option<SegHit> {
        if self.zones.is_empty() {
            return None;
        }
        let mut marks = vec![false; self.zones.len()];
        let from = self.find_zone_near(-1, a);
        let to = self.find_zone_near(-1, b);
        let mut bounds = empty_box();
        add_point(&mut bounds, a);
        add_point(&mut bounds, b);
        let d = sub(b, a);
        let len = dot(d, d).sqrt();
        let dir = if len > 0.0 {
            [d[0] / len, d[1] / len, d[2] / len]
        } else {
            d
        };
        let mut ctx = SegCtx {
            a,
            b,
            bounds,
            origin: a,
            dir,
            dist: f32::MAX,
            hit: false,
            pos: [0.0; 3],
            obj: -1,
        };
        self.trace_zone(from, from, to, &mut ctx, &mut marks);
        ctx.hit.then_some(SegHit {
            pos: ctx.pos,
            obj: ctx.obj,
        })
    }

    /// `StgCol_TraceZone`: the zone when the segment passes through it; without a hit every neighbour that
    /// contains the point where the segment left this one. The first zone with a hit ends the walk.
    fn trace_zone(&self, cur: usize, from: usize, to: usize, ctx: &mut SegCtx, marks: &mut [bool]) {
        if cur >= marks.len() || marks[cur] {
            return;
        }
        marks[cur] = true;
        let last = to == cur;
        let zone_box = self.zone_box(cur);
        if !zone_box.overlaps(&ctx.bounds) {
            return;
        }
        let mut entry = [0.0; 3];
        if !clip_ray(&zone_box, ctx.origin, ctx.dir, &mut entry) {
            return;
        }
        // StgCol_ClipSegToZone: the part of the segment inside the zone.
        let back = [-ctx.dir[0], -ctx.dir[1], -ctx.dir[2]];
        let mut b = empty_box();
        if from == to {
            b = ctx.bounds;
        } else if cur == from {
            add_point(&mut b, ctx.a);
            if clip_ray(&zone_box, ctx.b, back, &mut entry) {
                add_point(&mut b, entry);
            }
        } else if cur == to {
            add_point(&mut b, ctx.b);
            add_point(&mut b, entry);
        } else {
            add_point(&mut b, entry);
            if clip_ray(&zone_box, ctx.b, back, &mut entry) {
                add_point(&mut b, entry);
            }
        }
        let (a, end) = (ctx.a, ctx.b);
        let (hit, obj_mask) = self.query_zone_objects(cur, &b, &mut |poly, v| {
            // StgCol_SegCb: the nearest solid triangle.
            if poly.flags & POLY_NO_SOLID != 0 {
                return false;
            }
            match seg_test_tri(a, end, &v) {
                Some((pos, dist)) if ctx.dist > dist => {
                    ctx.pos = pos;
                    ctx.hit = true;
                    ctx.dist = dist;
                    true
                }
                _ => false,
            }
        });
        if hit {
            if obj_mask != 0 {
                ctx.obj = obj_mask.trailing_zeros() as i32;
            }
        } else if !last {
            for &n in &self.zones[cur].near {
                let Ok(n) = usize::try_from(n) else {
                    continue;
                };
                if n < self.zones.len() && contains_point(&self.zone_box(n), entry) {
                    self.trace_zone(n, from, to, ctx, marks);
                }
            }
        }
    }

    /// `StgCol_QueryZone` in mode 0 with every object standing: whether a callback reported a hit, and the mask
    /// of the objects whose whole meshes did.
    fn query_zone_objects(
        &self,
        zone: usize,
        b: &ColBox,
        cb: &mut dyn FnMut(&ColPoly, [[f32; 3]; 3]) -> bool,
    ) -> (bool, u64) {
        let Some(z) = self.zones.get(zone) else {
            return (false, 0);
        };
        let mut hit = z.mesh.walk_box(b, cb);
        let mut mask = 0u64;
        for rec in z.recs.iter().filter(|r| !r.broken) {
            if rec.mesh.walk_box(b, cb) {
                hit = true;
                if rec.obj < 64 {
                    mask |= 1 << rec.obj;
                }
            }
        }
        (hit, mask)
    }
}

/// `STGNAV_NODE_OFF`: never a start, a goal or a step.
pub const NAV_NODE_OFF: i32 = 1;

/// `StgNavNode` (0x30 bytes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavNode {
    pub pos: [f32; 4],
    pub flags: i32,
    pub obj: i32,
    pub links: [i16; 8],
}

/// The stage's way-point graph (src/battle/stg_nav.c), searched by the AI's move action.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageNav {
    pub nodes: Vec<NavNode>,
}

/// `StgNavEntry`: a node reached, the closed entry it was reached from and the links walked.
#[derive(Clone, Copy, Debug, Default)]
struct NavEntry {
    node: i32,
    from: Option<usize>,
    steps: i32,
}

/// Both search lists hold 0xC00 bytes.
const NAV_LIST: usize = 0x100;

impl StageNav {
    /// `StgNav_Bind`: stage file words 0x10 / 0x11 are the start and end of the graph; version 1 only. A stage
    /// without a graph has no nodes.
    pub fn parse(file: &[u8]) -> io::Result<Self> {
        let word = |o: usize| -> io::Result<u32> {
            file.get(o..o + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| invalid_data("stage navigation is truncated"))
        };
        let (start, end) = (word(0x40)? as usize & !3, word(0x44)? as usize & !3);
        if start == end || word(start)? != 1 {
            return Ok(Self::default());
        }
        let count = word(start + 4)? as usize;
        let mut nodes = Vec::with_capacity(count);
        for i in 0..count {
            let o = start + 0x10 + i * 0x30;
            let rec = file
                .get(o..o + 0x30)
                .ok_or_else(|| invalid_data("stage navigation node is truncated"))?;
            let f = |k: usize| f32::from_le_bytes(rec[k..k + 4].try_into().unwrap());
            let w = |k: usize| i32::from_le_bytes(rec[k..k + 4].try_into().unwrap());
            nodes.push(NavNode {
                pos: [f(0), f(4), f(8), f(0xC)],
                flags: w(0x10),
                obj: w(0x14),
                links: std::array::from_fn(|k| {
                    i16::from_le_bytes([rec[0x20 + k * 2], rec[0x21 + k * 2]])
                }),
            });
        }
        Ok(Self { nodes })
    }

    /// `StgNav_FindNearest`: the usable node nearest to `pos` within 10000; a later node wins a tie.
    pub fn find_nearest(&self, pos: [f32; 3]) -> i32 {
        let mut best = -1;
        let mut best_dist = 10000.0f32;
        for (i, n) in self.nodes.iter().enumerate() {
            if n.flags & NAV_NODE_OFF != 0 {
                continue;
            }
            let d = sub(pos, [n.pos[0], n.pos[1], n.pos[2]]);
            let dist = dot(d, d).sqrt();
            if best_dist < dist {
                continue;
            }
            best_dist = dist;
            best = i as i32;
        }
        best
    }

    /// `StgNav_FindPath`: `to` first, then the goal node and the nodes back towards the start (16 points at most);
    /// empty without a graph, a usable node or a connection.
    pub fn find_path(&self, from: [f32; 3], to: [f32; 4]) -> Vec<[f32; 4]> {
        let mut path = Vec::new();
        if self.nodes.is_empty() {
            return path;
        }
        let start = self.find_nearest(from);
        let goal = self.find_nearest([to[0], to[1], to[2]]);
        if start == -1 || goal == -1 {
            return path;
        }
        let mut open = vec![NavEntry::default(); NAV_LIST];
        let mut closed = vec![NavEntry::default(); NAV_LIST];
        let (mut open_count, mut closed_count) = (1usize, 0usize);
        open[0] = NavEntry {
            node: start,
            from: None,
            steps: 0,
        };
        let mut e = 0usize;
        loop {
            if open[e].node == goal {
                path.push(to);
                if path.len() >= 16 {
                    return path;
                }
                path.push(self.nodes[open[e].node as usize].pos);
                let mut p = open[e];
                while let Some(prev) = p.from {
                    p = closed[prev];
                    if path.len() >= 16 {
                        return path;
                    }
                    path.push(self.nodes[p.node as usize].pos);
                }
                return path;
            }
            if closed_count >= NAV_LIST {
                return path;
            }
            closed[closed_count] = open[e];
            closed_count += 1;
            open_count -= 1;
            let node = self.nodes[open[e].node as usize];
            for &link in &node.links {
                if link == -1 || node.flags & NAV_NODE_OFF != 0 {
                    continue;
                }
                let link = i32::from(link);
                if closed[..closed_count].iter().any(|c| c.node == link)
                    || open[..open_count].iter().any(|o| o.node == link)
                {
                    continue;
                }
                if open_count >= NAV_LIST {
                    break;
                }
                // The entry being expanded is the last open one: the first new entry overwrites it, and the
                // following ones read their step count back from there.
                open[open_count].node = link;
                open[open_count].from = Some(closed_count - 1);
                open[open_count].steps = open[e].steps + 1;
                open_count += 1;
            }
            sort_entries(0, open_count as i32 - 1, &mut open);
            if open_count == 0 {
                return path;
            }
            e = open_count - 1;
        }
    }
}

/// `StgNav_SortEntries`: quicksort by step count, largest first.
fn sort_entries(lo: i32, hi: i32, list: &mut [NavEntry]) {
    if lo >= hi {
        return;
    }
    list.swap(lo as usize, ((lo + hi) / 2) as usize);
    let mut last = lo;
    for i in lo + 1..=hi {
        if list[i as usize].steps > list[lo as usize].steps {
            last += 1;
            list.swap(last as usize, i as usize);
        }
    }
    list.swap(last as usize, lo as usize);
    sort_entries(lo, last - 1, list);
    sort_entries(last + 1, hi, list);
}

/// `ColTri_DistSqPoint` (the Magic Software point / triangle distance): the squared distance and the parameters
/// (s along v0 -> v1, t along v0 -> v2) of the nearest point; `None` for a degenerate triangle (the original's
/// 1e8 with s, t unwritten).
fn tri_dist_sq_point(p: [f32; 3], v: &[[f32; 3]; 3]) -> Option<(f32, f32, f32)> {
    let e0 = sub(v[1], v[0]);
    let e1 = sub(v[2], v[0]);
    let diff = sub(v[0], p);
    let a00 = dot(e0, e0);
    let a01 = dot(e0, e1);
    let a11 = dot(e1, e1);
    let b0 = dot(diff, e0);
    let b1 = dot(diff, e1);
    let c = dot(diff, diff);
    let det = (a00 * a11 - a01 * a01).abs();
    let mut s = a01 * b1 - a11 * b0;
    let mut t = a01 * b0 - a00 * b1;
    if det.abs() < 1e-9 {
        return None;
    }
    let inner = |s: f32, t: f32| {
        s * (a00 * s + a01 * t + 2.0 * b0) + t * (a01 * s + a11 * t + 2.0 * b1) + c
    };
    let d;
    if s + t <= det {
        if s < 0.0 {
            if t < 0.0 && b0 < 0.0 {
                t = 0.0;
                if -b0 >= a00 {
                    s = 1.0;
                    d = a00 + 2.0 * b0 + c;
                } else {
                    s = -b0 / a00;
                    d = b0 * s + c;
                }
            } else {
                s = 0.0;
                if b1 >= 0.0 {
                    t = 0.0;
                    d = c;
                } else if -b1 >= a11 {
                    t = 1.0;
                    d = a11 + 2.0 * b1 + c;
                } else {
                    t = -b1 / a11;
                    d = b1 * t + c;
                }
            }
        } else if t < 0.0 {
            t = 0.0;
            if b0 >= 0.0 {
                s = 0.0;
                d = c;
            } else if -b0 >= a00 {
                s = 1.0;
                d = a00 + 2.0 * b0 + c;
            } else {
                s = -b0 / a00;
                d = b0 * s + c;
            }
        } else {
            let inv = 1.0 / det;
            s *= inv;
            t *= inv;
            d = inner(s, t);
        }
    } else if s < 0.0 {
        let (tmp0, tmp1) = (a01 + b0, a11 + b1);
        if tmp1 > tmp0 {
            let numer = tmp1 - tmp0;
            let denom = a00 - 2.0 * a01 + a11;
            if numer >= denom {
                s = 1.0;
                t = 0.0;
                d = a00 + 2.0 * b0 + c;
            } else {
                s = numer / denom;
                t = 1.0 - s;
                d = inner(s, t);
            }
        } else {
            s = 0.0;
            if tmp1 <= 0.0 {
                t = 1.0;
                d = a11 + 2.0 * b1 + c;
            } else if b1 >= 0.0 {
                t = 0.0;
                d = c;
            } else {
                t = -b1 / a11;
                d = b1 * t + c;
            }
        }
    } else if t < 0.0 {
        let (tmp0, tmp1) = (a01 + b1, a00 + b0);
        if tmp1 > tmp0 {
            let numer = tmp1 - tmp0;
            let denom = a00 - 2.0 * a01 + a11;
            if numer >= denom {
                t = 1.0;
                s = 0.0;
                d = a11 + 2.0 * b1 + c;
            } else {
                t = numer / denom;
                s = 1.0 - t;
                d = inner(s, t);
            }
        } else {
            t = 0.0;
            if tmp1 <= 0.0 {
                s = 1.0;
                d = a00 + 2.0 * b0 + c;
            } else if b0 >= 0.0 {
                s = 0.0;
                d = c;
            } else {
                s = -b0 / a00;
                d = b0 * s + c;
            }
        }
    } else {
        let numer = a11 + b1 - a01 - b0;
        if numer <= 0.0 {
            s = 0.0;
            t = 1.0;
            d = a11 + 2.0 * b1 + c;
        } else {
            let denom = a00 - 2.0 * a01 + a11;
            if numer >= denom {
                s = 1.0;
                t = 0.0;
                d = a00 + 2.0 * b0 + c;
            } else {
                s = numer / denom;
                t = 1.0 - s;
                d = inner(s, t);
            }
        }
    }
    Some((d.abs(), s, t))
}

/// `ColSphere_TestTri`: the triangle's point nearest to the centre when within the radius.
pub fn sphere_test_tri(center: [f32; 3], radius_sq: f32, v: &[[f32; 3]; 3]) -> Option<[f32; 3]> {
    let (d, s, t) = tri_dist_sq_point(center, v)?;
    if d > radius_sq {
        return None;
    }
    // ColTri_GetPoint: v0 + s (v1 - v0) + t (v2 - v0).
    let (e0, e1) = (sub(v[1], v[0]), sub(v[2], v[0]));
    Some(std::array::from_fn(|i| e0[i] * s + v[0][i] + e1[i] * t))
}

/// The body sphere being moved through the stage (`StgColPushCtx`: sphere and bounds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushSphere {
    pub center: [f32; 3],
    pub radius: f32,
    pub bounds: ColBox,
    /// Some triangle pushed the sphere, and the flags and plane of the last one.
    pub hit: bool,
    pub poly_flags: u32,
    pub poly_nrm: [f32; 4],
}

impl PushSphere {
    /// `StgCol_StepSphere`: moves by `step`, bounds of radius + 0.1.
    pub fn step(&mut self, step: [f32; 3]) {
        self.center = [
            self.center[0] + step[0],
            self.center[1] + step[1],
            self.center[2] + step[2],
        ];
        let e = self.radius + 0.1;
        self.bounds = ColBox::from_center_half(self.center, [e; 3]);
    }

    /// `StgCol_PushSphere`: away from a contact point until it lies on the surface, bounds of radius + 1.1.
    fn push(&mut self, point: [f32; 3]) {
        let d = sub(self.center, point);
        let len = dot(d, d).sqrt();
        let k = (self.radius - len) / len;
        self.center = [
            self.center[0] + d[0] * k,
            self.center[1] + d[1] * k,
            self.center[2] + d[2] * k,
        ];
        let e = self.radius + 1.1;
        self.bounds = ColBox::from_center_half(self.center, [e; 3]);
    }
}

impl ColMesh {
    /// `ColMesh_WalkBox` with `StgCol_PushCb`: every solid triangle the sphere touches pushes it out, in walk order;
    /// the walk reads the bounds the pushes keep moving.
    fn push_walk(&self, idx: i32, sphere: &mut PushSphere) -> bool {
        let Some(node) = usize::try_from(idx).ok().and_then(|i| self.nodes.get(i)) else {
            return false;
        };
        let nb = ColBox {
            min: node.min,
            max: node.max,
        };
        if !sphere.bounds.overlaps(&nb) {
            return false;
        }
        if node.right == -1 {
            let Some(poly) = usize::try_from(node.left)
                .ok()
                .and_then(|i| self.polys.get(i))
            else {
                return false;
            };
            if poly.flags & POLY_NO_SOLID != 0 {
                return false;
            }
            let v = poly
                .vtx
                .map(|i| self.vertices.get(i as usize).copied().unwrap_or([0.0; 3]));
            let r = sphere.radius;
            return match sphere_test_tri(sphere.center, r * r, &v) {
                Some(point) => {
                    sphere.push(point);
                    sphere.poly_flags = poly.flags;
                    sphere.poly_nrm = poly.nrm;
                    sphere.hit = true;
                    true
                }
                None => false,
            };
        }
        let a = self.push_walk(node.left, sphere);
        let b = self.push_walk(node.right, sphere);
        a || b
    }
}

impl StageCollision {
    /// `StgCol_QueryZone` (mode 0, every object standing) with `StgCol_PushCb`: the zone's static mesh, then the
    /// whole objects' meshes. Returns whether anything pushed, and the mask of the objects that did.
    pub fn push_sphere(&self, zone: usize, sphere: &mut PushSphere) -> (bool, u64) {
        let Some(z) = self.zones.get(zone) else {
            return (false, 0);
        };
        let mut hit = !z.mesh.nodes.is_empty() && z.mesh.push_walk(0, sphere);
        let mut mask = 0u64;
        for rec in z.recs.iter().filter(|r| !r.broken) {
            if !rec.mesh.nodes.is_empty() && rec.mesh.push_walk(0, sphere) {
                hit = true;
                if rec.obj < 64 {
                    mask |= 1 << rec.obj;
                }
            }
        }
        (hit, mask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_mesh(y: f32) -> ColMesh {
        ColMesh {
            nodes: vec![ColNode {
                min: [-10.0, y, -10.0],
                max: [10.0, y, 10.0],
                left: 0,
                right: -1,
            }],
            polys: vec![ColPoly {
                flags: 0,
                vtx: [0, 1, 2],
                nrm: [0.0, -1.0, 0.0, -y],
            }],
            vertices: vec![[-10.0, y, -10.0], [10.0, y, -10.0], [-10.0, y, 10.0]],
        }
    }

    #[test]
    fn ground_probe_finds_the_floor_under_the_point_only() {
        let col = StageCollision {
            zones: vec![ColZone {
                center: [0.0; 3],
                half: [50.0, 0.0, 50.0],
                near: Vec::new(),
                mesh: flat_mesh(5.0),
                recs: Vec::new(),
            }],
        };
        let b = ColBox::from_center_half([-5.0, -3.0, -5.0], [0.1, 1.0, 0.1]);
        assert_eq!(col.ground_probe(0, &b, -5.0, -5.0, -3.0).y, 5.0);
        // Below the floor, or outside the triangle: nothing.
        assert_eq!(col.ground_probe(0, &b, -5.0, -5.0, 6.0).y, f32::MAX);
        assert_eq!(col.ground_probe(0, &b, 5.0, 5.0, -3.0).y, f32::MAX);
    }
}
