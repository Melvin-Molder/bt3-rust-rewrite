//! The GS scissor of a split-screen view on the CPU: triangles already projected to the 512 x 448 frame are cut
//! to the columns `x0..x1` (`SCISSOR_1` x0, x1 + 1) before they reach the renderer. The GS interpolates the
//! position, STQ and colour linearly in screen space, so the cut vertices are linear blends of the old ones.

use crate::battle::{ModelBatch, ScreenVertex};
use crate::eft::{EffectTri, EffectVertex};
use crate::post::{PostOp, PostVertex};
use crate::stage_draw::{StageBatch, StageScreenVertex};

/// A vertex the scissor can cut.
pub(crate) trait ClipVertex: Copy {
    fn x(&self) -> f32;
    fn set_x(&mut self, x: f32);
    fn lerp(&self, other: &Self, t: f32) -> Self;
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| mix(a[i], b[i], t))
}

impl ClipVertex for ScreenVertex {
    fn x(&self) -> f32 {
        self.position[0]
    }

    fn set_x(&mut self, x: f32) {
        self.position[0] = x;
    }

    fn lerp(&self, o: &Self, t: f32) -> Self {
        Self {
            position: mix3(self.position, o.position, t),
            stq: mix3(self.stq, o.stq, t),
            uq: mix(self.uq, o.uq, t),
        }
    }
}

impl ClipVertex for StageScreenVertex {
    fn x(&self) -> f32 {
        self.position[0]
    }

    fn set_x(&mut self, x: f32) {
        self.position[0] = x;
    }

    fn lerp(&self, o: &Self, t: f32) -> Self {
        Self {
            position: mix3(self.position, o.position, t),
            stq: mix3(self.stq, o.stq, t),
            rgba: std::array::from_fn(|i| mix(self.rgba[i], o.rgba[i], t)),
        }
    }
}

impl ClipVertex for EffectVertex {
    fn x(&self) -> f32 {
        self.position[0]
    }

    fn set_x(&mut self, x: f32) {
        self.position[0] = x;
    }

    fn lerp(&self, o: &Self, t: f32) -> Self {
        Self {
            position: mix3(self.position, o.position, t),
            stq: mix3(self.stq, o.stq, t),
            rgba: std::array::from_fn(|i| {
                mix(f32::from(self.rgba[i]), f32::from(o.rgba[i]), t).round() as u8
            }),
        }
    }
}

impl ClipVertex for PostVertex {
    fn x(&self) -> f32 {
        self.xy[0]
    }

    fn set_x(&mut self, x: f32) {
        self.xy[0] = x;
    }

    fn lerp(&self, o: &Self, t: f32) -> Self {
        Self {
            xy: std::array::from_fn(|i| mix(self.xy[i], o.xy[i], t)),
            uv: std::array::from_fn(|i| mix(self.uv[i], o.uv[i], t)),
            rgba: std::array::from_fn(|i| {
                mix(f32::from(self.rgba[i]), f32::from(o.rgba[i]), t).round() as u8
            }),
        }
    }
}

/// The polygon's part on the kept side of the line `x = edge` (`keep_right`: x >= edge, else x <= edge).
fn cut<V: ClipVertex>(poly: &[V], edge: f32, keep_right: bool) -> Vec<V> {
    let inside = |v: &V| {
        if keep_right {
            v.x() >= edge
        } else {
            v.x() <= edge
        }
    };
    let mut out = Vec::with_capacity(poly.len() + 2);
    for (i, a) in poly.iter().enumerate() {
        let b = &poly[(i + 1) % poly.len()];
        if inside(a) {
            out.push(*a);
        }
        if inside(a) != inside(b) {
            let t = (edge - a.x()) / (b.x() - a.x());
            let mut v = a.lerp(b, t);
            v.set_x(edge);
            out.push(v);
        }
    }
    out
}

/// One triangle cut to the columns `x0..x1`, as a fan of triangles (none when it lies outside).
pub(crate) fn clip_tri<V: ClipVertex>(tri: [V; 3], [x0, x1]: [f32; 2]) -> Vec<[V; 3]> {
    if tri.iter().all(|v| v.x() >= x0 && v.x() <= x1) {
        return vec![tri];
    }
    let poly = cut(&cut(&tri, x0, true), x1, false);
    (1..poly.len().saturating_sub(1))
        .map(|i| [poly[0], poly[i], poly[i + 1]])
        .collect()
}

/// A triangle list cut to the columns.
pub(crate) fn clip_list<V: ClipVertex>(vertices: &[V], cols: [f32; 2]) -> Vec<V> {
    vertices
        .chunks_exact(3)
        .flat_map(|t| clip_tri([t[0], t[1], t[2]], cols))
        .flatten()
        .collect()
}

pub(crate) fn clip_models(batches: &mut Vec<ModelBatch>, cols: [f32; 2]) {
    for b in batches.iter_mut() {
        b.vertices = clip_list(&b.vertices, cols);
    }
    batches.retain(|b| !b.vertices.is_empty());
}

pub(crate) fn clip_stage(batches: &mut Vec<StageBatch>, cols: [f32; 2]) {
    for b in batches.iter_mut() {
        b.vertices = clip_list(&b.vertices, cols);
    }
    batches.retain(|b| !b.vertices.is_empty());
}

pub(crate) fn clip_effects(tris: Vec<EffectTri>, cols: [f32; 2]) -> Vec<EffectTri> {
    let mut out = Vec::with_capacity(tris.len());
    for t in tris {
        for vertices in clip_tri(t.vertices, cols) {
            out.push(EffectTri {
                vertices,
                ..t.clone()
            });
        }
    }
    out
}

/// The draws into the frame cut to the columns (those into the work buffers have their own scissor).
pub(crate) fn clip_post(ops: &mut [PostOp], cols: [f32; 2]) {
    for op in ops
        .iter_mut()
        .filter(|op| op.target == crate::post::PostBuf::Frame)
    {
        op.tris = op.tris.iter().flat_map(|t| clip_tri(*t, cols)).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, s: f32) -> EffectVertex {
        EffectVertex {
            position: [x, y, 0.0],
            stq: [s, 0.0, 1.0],
            rgba: [0, 0, 0, (s * 100.0) as u8],
        }
    }

    #[test]
    fn a_triangle_across_the_edge_is_cut_and_its_attributes_blended() {
        let cols = [0.0, 255.0];
        // Inside: unchanged.
        let tri = [v(10.0, 0.0, 0.0), v(20.0, 0.0, 1.0), v(10.0, 10.0, 0.0)];
        assert_eq!(clip_tri(tri, cols), vec![tri]);
        // Outside: gone.
        let tri = [v(300.0, 0.0, 0.0), v(400.0, 0.0, 1.0), v(300.0, 10.0, 0.0)];
        assert!(clip_tri(tri, cols).is_empty());
        // Across x = 255: a quad of two triangles, the cut at a blend of the attributes.
        let tri = [v(155.0, 0.0, 0.0), v(355.0, 0.0, 1.0), v(155.0, 10.0, 0.0)];
        let out = clip_tri(tri, cols);
        assert_eq!(out.len(), 2);
        let cut = out[0].iter().find(|p| p.position[0] == 255.0).unwrap();
        assert!((cut.stq[0] - 0.5).abs() < 1e-6);
        assert_eq!(cut.rgba[3], 50);
        assert!(out.iter().flatten().all(|p| p.position[0] <= 255.0));
        // A wide one cut on both sides.
        let tri = [
            v(-100.0, 0.0, 0.0),
            v(400.0, 0.0, 1.0),
            v(150.0, 100.0, 0.5),
        ];
        let out = clip_tri(tri, cols);
        assert!(out.len() >= 2);
        assert!(
            out.iter()
                .flatten()
                .all(|p| (0.0..=255.0).contains(&p.position[0]))
        );
    }
}
