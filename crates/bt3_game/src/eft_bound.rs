//! The boundary wall (`EftBound*`, src/battle/eft_shot.c; stage effect kind 8, on a stage whose pack has entries
//! 0x15..0x17): in front of each fighter nearing the stage's radius, a curved 6 x 4 grid on the cylinder of the
//! inner radius (plus the stage's addition) centred on the fighter's bearing, fading in as the fighter gets
//! within range and out away from the point nearest to it, its texture scrolling and its alpha pulsing. Off while
//! the fighter is in a technique or has a rush connected.

use bt3_data::stage::StageBound;

use crate::eft::{
    ClipVtx, DrawCtx, OrderTable, Prim, V3, ZSort, cross, dot, norm, point, queue_tri_clip, sub,
};

const COLS: usize = 6;
const ROWS: usize = 4;
const QUADS: usize = (COLS - 1) * (ROWS - 1);

/// A fighter as the wall reads it: node 3, in a technique, flag 0xA1 (a rush connected).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BoundChr {
    pub node3: V3,
    pub in_technique: bool,
    pub rush_connected: bool,
}

/// What the draw tests of a fighter: hidden (object flag 2 clear), locked on (flag 5), its and its opponent's
/// rush.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BoundShow {
    pub hidden: bool,
    pub locked_on: bool,
    pub rush_connected: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Vtx {
    pos: V3,
    st: [f32; 2],
    col: [f32; 4],
}

/// `EftBoundMesh`.
#[derive(Clone, Debug, Default)]
struct Mesh {
    center: V3,
    scroll: [f32; 2],
    phase: f32,
    active: bool,
    vtx: [Vtx; COLS * ROWS],
    quads: [([usize; 4], V3); QUADS],
}

/// `EftBoundChar`: a fighter's wall.
#[derive(Clone, Debug, Default)]
struct Wall {
    radius: f32,
    char_radius: f32,
    angle: f32,
    pos: V3,
    mesh: Mesh,
}

/// `gEftBound` and the fighters' tasks.
#[derive(Clone, Debug)]
pub(crate) struct Bound {
    param: StageBound,
    texture: Option<usize>,
    /// `EftBoundMgr_Enable` (nothing disables it).
    enabled: bool,
    walls: [Wall; 2],
}

impl Bound {
    /// `EftBoundMgr_Init` and `EftBound_Init` of both fighters: the stage's inner radius plus the parameter's,
    /// and each fighter's body radius.
    pub(crate) fn new(
        param: StageBound,
        texture: Option<usize>,
        inner_radius: f32,
        char_radius: [f32; 2],
    ) -> Self {
        let wall = |r| Wall {
            radius: inner_radius + param.radius_add,
            char_radius: r,
            ..Wall::default()
        };
        Self {
            param,
            texture,
            enabled: true,
            walls: char_radius.map(wall),
        }
    }

    /// Whether a fighter's wall shows.
    #[cfg(test)]
    pub(crate) fn active(&self, side: usize) -> bool {
        self.walls[side].mesh.active
    }

    /// `EftBound_Update` of both fighters (`EftBound_UpdateAngle`, `EftBound_BuildWall`); the texture's scroll
    /// and the pulse stand still while time is stopped.
    pub(crate) fn update(&mut self, chars: &[BoundChr; 2], time_stopped: bool) {
        for (w, c) in self.walls.iter_mut().zip(chars) {
            w.pos = c.node3;
            let dir = norm(w.pos);
            w.angle = dir[0].atan2(dir[2]);
            build(w, &self.param, c, time_stopped);
        }
    }

    /// `EftBound_Draw` of both fighters: the quads of a shown wall that face the camera.
    pub(crate) fn draw(&self, ot: &mut OrderTable, ctx: &DrawCtx, show: &[BoundShow; 2]) {
        if !self.enabled {
            return;
        }
        for (side, w) in self.walls.iter().enumerate() {
            let s = show[side];
            if s.hidden
                || (s.locked_on && (s.rush_connected || show[side ^ 1].rush_connected))
                || !w.mesh.active
            {
                continue;
            }
            let m = &w.mesh;
            for (idx, normal) in &m.quads {
                let d = norm(sub(m.vtx[idx[1]].pos, ctx.eye));
                if dot(*normal, d) >= 0.0 {
                    continue;
                }
                self.tri(ot, ctx, [m.vtx[idx[0]], m.vtx[idx[1]], m.vtx[idx[2]]]);
                self.tri(ot, ctx, [m.vtx[idx[2]], m.vtx[idx[1]], m.vtx[idx[3]]]);
            }
        }
    }

    /// `EftBound_DrawTri`: the first corner's whole texture repeats taken off, then the shared clipper with the
    /// average depth doubled.
    fn tri(&self, ot: &mut OrderTable, ctx: &DrawCtx, v: [Vtx; 3]) {
        let (fs, ft) = (v[0].st[0] as i32 as f32, v[0].st[1] as i32 as f32);
        let tri = v.map(|v| ClipVtx {
            pos: point(v.pos),
            uv: [v.st[0] - fs, v.st[1] - ft, 1.0, 0.0],
            col: v.col,
        });
        let prim = Prim {
            texture: self.texture,
            ot_layer: self.param.layer.max(0) as u8,
            z: ZSort::Average(2.0),
            front: false,
            clamp: false,
            alpha_min: 0.01,
        };
        queue_tri_clip(ot, ctx, tri, prim);
    }
}

/// `EftBound_BuildWall`.
fn build(w: &mut Wall, p: &StageBound, c: &BoundChr, time_stopped: bool) {
    let m = &mut w.mesh;
    m.center = [w.angle.sin() * w.radius, w.pos[1], w.angle.cos() * w.radius];
    let d = sub(m.center, w.pos);
    let dist = ((d[0] * d[0] + d[2] * d[2]).sqrt() - w.char_radius).max(0.0);
    let mut fade = ((p.range - dist) / p.range).max(0.0);
    if !time_stopped {
        for k in 0..2 {
            m.scroll[k] += p.scroll[k];
            if m.scroll[k] > 1.0 {
                m.scroll[k] -= 1.0;
            }
        }
        m.phase = crate::btl_tech_volley::eft_wrap(m.phase + p.phase_rate * 6.283_185_3);
    }
    if c.in_technique || c.rush_connected {
        fade = 0.0;
    }
    if !(fade > 0.0) {
        m.active = false;
        return;
    }
    m.active = true;
    let y0 = w.pos[1] - p.height * 0.5;
    let pulse = m.phase.cos().abs();
    for j in 0..ROWS {
        for i in 0..COLS {
            let a = w.angle - p.arc * 0.5 + p.arc * (i as f32 / (COLS as f32 - 1.0));
            let pos = [
                a.sin() * w.radius,
                y0 + j as f32 / (ROWS as f32 - 1.0) * p.height,
                a.cos() * w.radius,
            ];
            let st = [
                m.scroll[0] + a / p.tex_arc,
                m.scroll[1] + pos[1] / p.tex_height,
            ];
            let d = sub(pos, m.center);
            let k = ((p.fade_dist - dot(d, d).sqrt()) / p.fade_dist).clamp(0.0, 1.0);
            let alpha = p.alpha * fade * k;
            let alpha = (alpha + alpha * pulse).min(255.0);
            m.vtx[j * COLS + i] = Vtx {
                pos,
                st,
                col: [p.color[0], p.color[1], p.color[2], alpha],
            };
        }
    }
    // The quads row by row, each with the normal of its first triangle.
    let mut idx = [0, 1, COLS, COLS + 1];
    let mut col = 0;
    for n in 0..QUADS {
        m.quads[n].0 = idx;
        col = (col + 1) % (COLS - 1);
        for i in &mut idx {
            *i += if col == 0 { 2 } else { 1 };
        }
    }
    for q in &mut m.quads {
        let v = |k: usize| m.vtx[q.0[k]].pos;
        let e1 = sub(v(1), v(0));
        let e2 = sub(v(2), v(1));
        q.1 = norm(cross(e2, e1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;

    fn param() -> StageBound {
        StageBound {
            range: 200.0,
            height: 300.0,
            radius_add: 50.0,
            alpha: 60.0,
            fade_dist: 400.0,
            arc: 1.0,
            tex_arc: 0.5,
            tex_height: 100.0,
            scroll: [0.01, 0.02],
            phase_rate: 0.01,
            layer: 1,
            color: [100.0, 150.0, 255.0],
        }
    }

    #[test]
    fn the_wall_shows_near_the_edge_and_faces_the_fighter() {
        // Inner radius 1000 + 50; fighters of radius 10.
        let mut b = Bound::new(param(), Some(3), 1000.0, [10.0; 2]);
        let mut chars = [BoundChr::default(); 2];
        chars[0].node3 = [0.0, -20.0, 500.0];
        chars[1].node3 = [0.0, -20.0, 900.0];
        b.update(&chars, false);
        // 1050 - 500 - 10 is out of range; 1050 - 900 - 10 = 140 is within 200.
        assert!(!b.active(0) && b.active(1));
        let m = &b.walls[1].mesh;
        assert_eq!(m.center, [0.0, -20.0, 1050.0]);
        // The alpha: the parameter's, by the fade (0.3) and the distance from the centre, doubled by the pulse
        // (cos of the phase, 0.0628 rad).
        let fade = (200.0 - 140.0) / 200.0;
        assert!(m.vtx.iter().all(|v| v.col[3] <= 2.0 * 60.0 * fade));
        // The far corners (525 to the side) are past the fade distance.
        assert_eq!(m.vtx[0].col[3], 0.0);
        assert!(m.vtx[2].col[3] > 0.0);
        assert_eq!(m.vtx[0].col[..3], [100.0, 150.0, 255.0]);
        // Quads row by row: five a row.
        assert_eq!(m.quads[0].0, [0, 1, 6, 7]);
        assert_eq!(m.quads[4].0, [4, 5, 10, 11]);
        assert_eq!(m.quads[5].0, [6, 7, 12, 13]);
        // A technique or a connected rush hides it.
        chars[1].in_technique = true;
        b.update(&chars, false);
        assert!(!b.active(1));
        // Seen from the middle of the stage it draws in front of the camera.
        chars[1].in_technique = false;
        b.update(&chars, false);
        let view = View::new([0.0, -20.0, 600.0], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = OrderTable::new();
        b.draw(&mut ot, &ctx, &[BoundShow::default(); 2]);
        let tris = ot.into_list();
        assert!(!tris.is_empty());
        assert!(tris.iter().all(|t| t.texture == Some(3)));
        // Hidden: nothing.
        let mut ot = OrderTable::new();
        let hidden = [
            BoundShow::default(),
            BoundShow {
                hidden: true,
                ..BoundShow::default()
            },
        ];
        b.draw(&mut ot, &ctx, &hidden);
        assert!(ot.into_list().is_empty());
    }
}
