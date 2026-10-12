//! `StgModel_Draw` and VU1 program 4 (src/battle/stg_model_draw.c, docs/systems/vu1/prog4_prog5.md): the stage's
//! groups, materials and meshes in draw order, each strip transformed, tested against the clip volume and, where a
//! triangle crosses it, clipped into a fan the way the program does.

use bt3_data::stage_model::{GROUP_DEBRIS, GROUP_MAIN, StageModel};

use crate::battle::{View, apply};
use crate::stg_obj::FallingPiece;

/// The near plane of the battle view (`view->nearZ`), which `Stg_FadeByCamDist` measures from.
const NEAR_Z: f32 = 0.3;

/// The GS frame offset of the battle (`View_InitLayout`, full screen).
const OFFSET_X: f32 = 2048.0 - 256.0;
const OFFSET_Y: f32 = 2048.0 - 224.0;

/// One vertex as the GS gets it: screen position in pixels and Z, S / T / Q (already divided by w) and the RGBA
/// integers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageScreenVertex {
    pub position: [f32; 3],
    pub stq: [f32; 3],
    pub rgba: [f32; 4],
}

/// Triangles of one material of one draw group, in draw order.
#[derive(Clone, Debug, PartialEq)]
pub struct StageBatch {
    pub group: usize,
    /// Position of the material in the drawing order of its group.
    pub order: usize,
    pub texture: usize,
    /// Three vertices per triangle.
    pub vertices: Vec<StageScreenVertex>,
}

/// One stage draw for a view. The debris group only draws falling pieces and group 2 only meshes of program 0.
/// Stage 5 skips group 3. Texture animations of group 2 step once per call, as `StgModel_Draw` steps them.
pub fn draw_stage(model: &mut StageModel, stage_id: i32, view: &View) -> Vec<StageBatch> {
    let mut batches = Vec::new();
    let drawable = &model.drawable;
    for (group, materials) in model.groups.iter_mut().enumerate() {
        if group == GROUP_DEBRIS || (group == 3 && stage_id == 5) {
            continue;
        }
        for (order, material) in materials.iter_mut().enumerate() {
            let shown = |mesh: &&bt3_data::stage_model::StageMesh| {
                drawable[usize::from(mesh.vis)] && (group != GROUP_MAIN || mesh.prog == 0)
            };
            if !material.meshes.iter().any(|mesh| shown(&mesh)) {
                continue;
            }
            let texture = if group == GROUP_MAIN {
                step_texture_anim(material)
            } else {
                usize::from(material.tex)
            };
            let mut vertices = Vec::new();
            for mesh in material.meshes.iter().filter(shown) {
                for strip in &mesh.strips {
                    draw_strip(strip, view, &mut vertices);
                }
            }
            if !vertices.is_empty() {
                batches.push(StageBatch {
                    group,
                    order,
                    texture,
                    vertices,
                });
            }
        }
    }
    batches
}

/// `StgModel_FadeDebris` then `StgModel_DrawDebris` (VU1 program 7): the falling objects' pieces, each mesh
/// moved by its node's matrix, with an alpha that fades over the fall's last 30 frames and near the camera (none
/// within 75 of its near plane, full from 100); a triangle with a vertex outside the clip volume, or closing on a
/// vertex flagged not to draw, is dropped. `eye` is the view's position.
pub fn draw_debris(
    model: &StageModel,
    pieces: &[FallingPiece],
    node_mtx: &[[[f32; 4]; 4]],
    view: &View,
    eye: [f32; 3],
) -> Vec<StageBatch> {
    let mut alpha = vec![0u8; model.drawable.len()];
    for p in pieces {
        let Some(m) = node_mtx.get(p.node) else {
            continue;
        };
        let pos = [m[3][0], m[3][1], m[3][2]];
        let d = [eye[0] - pos[0], eye[1] - pos[1], eye[2] - pos[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - NEAR_Z;
        // Stg_FadeByCamDist.
        let a = if dist <= 0.0 {
            0.0
        } else if dist > 100.0 {
            p.ratio
        } else if dist > 75.0 {
            (dist - 75.0) / 25.0 * p.ratio
        } else {
            0.0
        };
        let a = if a < 1.0 { (a * 128.0) as u8 } else { 0x80 };
        if a == 0 {
            continue;
        }
        for &vis in &model.parts[p.part].vis {
            if let Some(slot) = alpha.get_mut(usize::from(vis)) {
                *slot = a;
            }
        }
    }
    let mut batches = Vec::new();
    let Some(materials) = model.groups.get(GROUP_DEBRIS) else {
        return batches;
    };
    for (order, material) in materials.iter().enumerate() {
        let mut vertices = Vec::new();
        for mesh in &material.meshes {
            let a = alpha.get(usize::from(mesh.vis)).copied().unwrap_or(0);
            let Some(m) = mesh.node.and_then(|n| node_mtx.get(n)) else {
                continue;
            };
            if a == 0 {
                continue;
            }
            for strip in &mesh.strips {
                draw_strip_prog7(strip, m, f32::from(a), view, &mut vertices);
            }
        }
        if !vertices.is_empty() {
            batches.push(StageBatch {
                group: GROUP_DEBRIS,
                order,
                texture: usize::from(material.tex),
                vertices,
            });
        }
    }
    batches
}

/// Program 7's vertex loop over one strip: the position through the mesh's matrix, the colour's alpha replaced,
/// a triangle drawn only when none of its vertices is clipped and its last one is not flagged (`ADC`).
fn draw_strip_prog7(
    strip: &[bt3_data::stage_model::StageVertex],
    local: &[[f32; 4]; 4],
    alpha: f32,
    view: &View,
    out: &mut Vec<StageScreenVertex>,
) {
    let mut history: Vec<(StageScreenVertex, u32)> = Vec::with_capacity(strip.len());
    for vertex in strip {
        let p = vertex.position;
        let world = apply(local, [p[0], p[1], p[2], 1.0]);
        let s = apply(&view.world2screen, world);
        let c = apply(&view.world2clip, world);
        let q = 1.0 / s[3];
        let mut rgba = vertex.color.map(f32::trunc);
        rgba[3] = alpha;
        history.push((
            StageScreenVertex {
                position: [
                    (s[0] * q * 16.0).trunc() / 16.0 - OFFSET_X,
                    (s[1] * q * 16.0).trunc() / 16.0 - OFFSET_Y,
                    (s[2] * q).trunc(),
                ],
                stq: vertex.stq.map(|v| v * q),
                rgba,
            },
            clip_flags(c),
        ));
        let n = history.len();
        let flagged = p[3].to_bits() & 0xFFFF != 0;
        if n < 3 || flagged {
            continue;
        }
        let tri = [&history[n - 3], &history[n - 2], &history[n - 1]];
        if tri.iter().all(|v| v.1 == 0) {
            out.extend(tri.map(|v| v.0));
        }
    }
}

/// The texture a group 2 material draws with: its animation stepped once (looping, or back and forth with bit
/// 15 meaning "going back"), or its own texture.
fn step_texture_anim(material: &mut bt3_data::stage_model::StageMaterial) -> usize {
    let Some(anim) = &material.anim else {
        return usize::from(material.tex);
    };
    if anim.flags & 4 != 0 {
        let frames = &anim.frames;
        let n = frames.len();
        let cur = material.cur;
        if let Some(k) = frames.iter().position(|&f| f == i32::from(cur & 0x7FFF)) {
            material.cur = if anim.flags & 1 != 0 {
                frames[(k + 1) % n] as u16
            } else if cur & 0x8000 != 0 {
                if k == 0 {
                    frames.get(1).copied().unwrap_or(frames[0]) as u16
                } else {
                    frames[k - 1] as u16 | 0x8000
                }
            } else if k == n - 1 {
                frames[n.saturating_sub(2)] as u16 | 0x8000
            } else {
                frames[k + 1] as u16
            };
        }
    }
    usize::from(material.cur as u8)
}

/// A vertex in clip space with what the clipper interpolates.
#[derive(Clone, Copy, Debug)]
struct ClipVertex {
    clip: [f32; 4],
    rgba: [f32; 4],
    stq: [f32; 3],
}

/// Program 4's vertex loop over one strip.
fn draw_strip(
    strip: &[bt3_data::stage_model::StageVertex],
    view: &View,
    out: &mut Vec<StageScreenVertex>,
) {
    let mut history: Vec<(ClipVertex, StageScreenVertex, u32)> = Vec::with_capacity(strip.len());
    for vertex in strip {
        let p = vertex.position;
        let pos = [p[0], p[1], p[2], 1.0];
        let s = apply(&view.world2screen, pos);
        let c = apply(&view.world2clip, pos);
        let q = 1.0 / s[3];
        let rgba = vertex.color.map(f32::trunc);
        let screen = StageScreenVertex {
            position: [
                (s[0] * q * 16.0).trunc() / 16.0 - OFFSET_X,
                (s[1] * q * 16.0).trunc() / 16.0 - OFFSET_Y,
                (s[2] * q).trunc(),
            ],
            stq: vertex.stq.map(|v| v * q),
            rgba,
        };
        let clip = ClipVertex {
            clip: c,
            rgba,
            stq: vertex.stq,
        };
        history.push((clip, screen, clip_flags(c)));
        let n = history.len();
        if n < 3 {
            continue;
        }
        let tri = [&history[n - 3], &history[n - 2], &history[n - 1]];
        if tri.iter().all(|v| v.2 == 0) {
            out.extend(tri.map(|v| v.1));
        } else if tri[0].2 & tri[1].2 & tri[2].2 == 0 {
            // Not wholly outside one plane: drawn again as a clipped fan.
            clip_triangle([tri[0].0, tri[1].0, tri[2].0], view, out);
        }
    }
}

/// Program 6's vertex loop (the ground shadows): separate triangles (position, RGBA, S / T / 1) sent as one strip
/// whose joining triangles are not drawn, so each triangle is drawn whole, clipped into a fan, or dropped when it
/// lies outside one plane, as program 4 draws a strip's.
pub(crate) fn draw_triangles(
    tris: &[[([f32; 3], [f32; 4], [f32; 3]); 3]],
    view: &View,
) -> Vec<StageScreenVertex> {
    let mut out = Vec::new();
    for tri in tris {
        let v = tri.map(|(p, rgba, stq)| {
            let pos = [p[0], p[1], p[2], 1.0];
            let s = apply(&view.world2screen, pos);
            let c = apply(&view.world2clip, pos);
            let q = 1.0 / s[3];
            let rgba = rgba.map(f32::trunc);
            let screen = StageScreenVertex {
                position: [
                    (s[0] * q * 16.0).trunc() / 16.0 - OFFSET_X,
                    (s[1] * q * 16.0).trunc() / 16.0 - OFFSET_Y,
                    (s[2] * q).trunc(),
                ],
                stq: stq.map(|t| t * q),
                rgba,
            };
            (ClipVertex { clip: c, rgba, stq }, screen, clip_flags(c))
        });
        if v.iter().all(|v| v.2 == 0) {
            out.extend(v.map(|v| v.1));
        } else if v[0].2 & v[1].2 & v[2].2 == 0 {
            clip_triangle([v[0].0, v[1].0, v[2].0], view, &mut out);
        }
    }
    out
}

/// The VU `clip` flags of a vertex: bit per plane x+, x-, y+, y-, z+, z- (outside |w|).
fn clip_flags(c: [f32; 4]) -> u32 {
    let w = c[3].abs();
    let mut flags = 0;
    for axis in 0..3 {
        if c[axis] > w {
            flags |= 1 << (axis * 2);
        }
        if c[axis] < -w {
            flags |= 2 << (axis * 2);
        }
    }
    flags
}

/// `clip_triangle` + `emit_fan`: the triangle enlarged by 0.5% about its centroid, cut by z = -w, z = +w, x = -w,
/// x = +w, y = -w, y = +w in turn (Sutherland-Hodgman, linear in clip space), then drawn as a fan.
fn clip_triangle(tri: [ClipVertex; 3], view: &View, out: &mut Vec<StageScreenVertex>) {
    let centroid: [f32; 4] =
        std::array::from_fn(|k| (tri[0].clip[k] + tri[1].clip[k] + tri[2].clip[k]) / 3.0);
    let mut polygon: Vec<ClipVertex> = tri
        .iter()
        .map(|v| ClipVertex {
            clip: std::array::from_fn(|k| centroid[k] + 1.005 * (v.clip[k] - centroid[k])),
            ..*v
        })
        .collect();
    for (axis, sign) in [
        (2, -1.0f32),
        (2, 1.0),
        (0, -1.0),
        (0, 1.0),
        (1, -1.0),
        (1, 1.0),
    ] {
        let inside = |v: &ClipVertex| {
            let w = v.clip[3].abs();
            if sign < 0.0 {
                v.clip[axis] >= -w
            } else {
                v.clip[axis] <= w
            }
        };
        let mut next = Vec::with_capacity(polygon.len() + 2);
        for i in 0..polygon.len() {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            match (inside(&a), inside(&b)) {
                (true, true) => next.push(a),
                (true, false) => {
                    next.push(a);
                    next.push(intersect(&a, &b, axis, sign));
                }
                (false, true) => next.push(intersect(&a, &b, axis, sign)),
                (false, false) => {}
            }
        }
        if next.is_empty() {
            return;
        }
        polygon = next;
    }
    let emit = |v: &ClipVertex| {
        let s = apply(&view.clip2screen, v.clip);
        let q = 1.0 / v.clip[3];
        StageScreenVertex {
            position: [
                (s[0] * q * 16.0).trunc() / 16.0 - OFFSET_X,
                (s[1] * q * 16.0).trunc() / 16.0 - OFFSET_Y,
                (s[2] * q).trunc(),
            ],
            stq: v.stq.map(|t| t * q),
            rgba: v.rgba.map(f32::trunc),
        }
    };
    let fan: Vec<StageScreenVertex> = polygon.iter().map(emit).collect();
    for i in 1..fan.len().saturating_sub(1) {
        out.extend([fan[0], fan[i], fan[i + 1]]);
    }
}

/// `intersect`: d = c - sign * w at both ends, t = |dA / (dB - dA)|.
fn intersect(a: &ClipVertex, b: &ClipVertex, axis: usize, sign: f32) -> ClipVertex {
    let da = a.clip[axis] - sign * a.clip[3];
    let db = b.clip[axis] - sign * b.clip[3];
    let t = (da / (db - da)).abs();
    let lerp4 = |x: [f32; 4], y: [f32; 4]| std::array::from_fn(|k| x[k] + t * (y[k] - x[k]));
    ClipVertex {
        clip: lerp4(a.clip, b.clip),
        rgba: lerp4(a.rgba, b.rgba),
        stq: std::array::from_fn(|k| a.stq[k] + t * (b.stq[k] - a.stq[k])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(x: f32, y: f32, z: f32, w: f32) -> ClipVertex {
        ClipVertex {
            clip: [x, y, z, w],
            rgba: [128.0; 4],
            stq: [0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn crossing_the_x_plane_is_cut_at_the_plane() {
        let a = vertex(0.5, 0.0, 0.0, 1.0);
        let b = vertex(1.5, 0.0, 0.0, 1.0);
        let cut = intersect(&a, &b, 0, 1.0);
        assert!((cut.clip[0] - 1.0).abs() < 1e-6);
        assert_eq!(clip_flags(b.clip), 1);
        assert_eq!(clip_flags(vertex(0.0, -2.0, 0.0, 1.0).clip), 8);
    }
}
