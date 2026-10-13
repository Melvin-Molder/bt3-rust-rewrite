//! The blade trails (`EftBlade_*`, src/battle/eft_char_parts.c; layer 3 entry 6): a ribbon behind the weapon of the
//! eight characters of `gEftBladeDef` while it is out (`BtlCharApi_HasWeaponOut`). Each frame the blade's base and
//! tip (in the space of node 0x40) join a ring of 128; when the base moved more than 1 the last entry is replaced by
//! ten Catmull-Rom points through the last four frames. Entries fade by 20 a frame. Drawn as two gouraud triangles
//! per step in the character's colour, the alpha running from the entry to nothing across the ribbon.
//!
//! The manager makes both fighters' trails when the scene starts, drawing libc `rand()` three times each for values
//! nothing reads (`EftBlade_Create`, `EftBlade_Init`).

use crate::battle::{Mtx, apply};
use crate::eft::{
    DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, V3, depth_slot,
    sub,
};
use crate::rng::LibcRandom;

/// The ring's size.
const RING: usize = 128;

/// `gEftBladeDef`: character, blade base and tip in node 0x40's space, colour (RGBA, 0..255) and table layer.
const DEFS: [(i32, V3, V3, [i32; 4], usize); 8] = [
    (
        0x3E,
        [0.0, 0.0, -1.5],
        [0.0, 0.0, -10.5],
        [53, 147, 242, 219],
        1,
    ),
    (
        0x8A,
        [-1.5, 0.0, -1.5],
        [-1.5, 0.0, -14.5],
        [220, 0, 150, 215],
        0,
    ),
    (
        0x27,
        [0.0, 0.0, -1.5],
        [0.0, 0.0, -8.1],
        [53, 147, 242, 219],
        1,
    ),
    (
        0x28,
        [0.0, 0.0, -1.5],
        [0.0, 0.0, -8.1],
        [53, 147, 242, 219],
        1,
    ),
    (
        0x3C,
        [0.0, 0.0, -1.5],
        [0.0, 0.0, -8.5],
        [220, 220, 240, 255],
        0,
    ),
    (
        0x94,
        [0.0, 0.0, -1.5],
        [0.0, 0.0, -10.5],
        [220, 220, 240, 255],
        0,
    ),
    (0x6F, [0.0, 0.0, -1.5], [0.0, 0.0, -16.0], [0, 0, 0, 230], 0),
    (0x61, [0.0, 0.0, -1.5], [0.0, 0.0, -16.0], [0, 0, 0, 230], 0),
];

/// `EftBladeTrail`.
#[derive(Clone, Debug)]
struct Trail {
    edge_a: [V3; RING],
    edge_b: [V3; RING],
    last_a: [V3; 4],
    last_b: [V3; 4],
    alpha: [i32; RING],
    tail: usize,
    head: usize,
    was_out: bool,
}

impl Default for Trail {
    fn default() -> Self {
        Self {
            edge_a: [[0.0; 3]; RING],
            edge_b: [[0.0; 3]; RING],
            last_a: [[0.0; 3]; 4],
            last_b: [[0.0; 3]; 4],
            alpha: [0; RING],
            tail: 0,
            head: 0,
            was_out: false,
        }
    }
}

/// What a trail reads of its fighter.
pub(crate) struct BladeChr<'a> {
    /// `BtlCharApi_GetChara`.
    pub chara: i32,
    /// `BtlCharApi_HasWeaponOut`.
    pub weapon_out: bool,
    /// `BtlCharApi_IsHidden`: object flag 2 off.
    pub hidden: bool,
    /// Node 0x40's matrix.
    pub node_mtx: &'a dyn Fn(u32) -> Option<Mtx>,
}

/// `gEftBlade`'s two trails, and whether the scene's start still owes their libc draws.
#[derive(Clone, Debug)]
pub(crate) struct Blades {
    trails: [Trail; 2],
    pub init_draws_pending: bool,
}

impl Default for Blades {
    fn default() -> Self {
        Self {
            trails: Default::default(),
            init_draws_pending: true,
        }
    }
}

fn find_def(chara: i32) -> Option<&'static (i32, V3, V3, [i32; 4], usize)> {
    DEFS.iter().find(|d| d.0 == chara)
}

/// `EftBlade_Spline`: the Catmull-Rom point between `p[1]` and `p[2]` at `t`.
fn spline(p: &[V3; 4], t: f32) -> V3 {
    let t2 = t * t;
    let t3 = t2 * t;
    std::array::from_fn(|k| {
        let a = p[1][k] - p[0][k];
        let b = p[2][k] - p[1][k];
        let m0 = (a + b) * 0.5;
        let m1 = (b + (p[3][k] - p[2][k])) * 0.5;
        let s = p[1][k] * (2.0 * t3 - 3.0 * t2 + 1.0);
        let m0 = m0 * (t3 - 2.0 * t2 + t);
        let m1 = m1 * (t3 - t2);
        let e = (s + m0) + m1;
        p[2][k] * (-2.0 * t3 + 3.0 * t2) + e
    })
}

impl Blades {
    /// The live entries of a fighter's ring.
    #[cfg(test)]
    pub(crate) fn live(&self, side: usize) -> usize {
        self.trails[side].alpha.iter().filter(|&&a| a > 0).count()
    }

    /// `EftBladeMgr_Init`'s draws: per fighter one in `EftBlade_Create`, two in `EftBlade_Init`.
    pub(crate) fn run_init_draws(&mut self, libc: &mut LibcRandom) {
        if std::mem::take(&mut self.init_draws_pending) {
            for _ in 0..2 * 3 {
                libc.rand();
            }
        }
    }

    /// `EftBlade_Update` of both trails (not while paused): the entries fade, and the blade's ends of this frame
    /// join the ring while the weapon is out.
    pub(crate) fn update(&mut self, chars: [BladeChr; 2], paused: bool) {
        if paused {
            return;
        }
        for (t, c) in self.trails.iter_mut().zip(chars) {
            let mut i = t.tail;
            while i != t.head {
                if t.alpha[i] > 0 {
                    t.alpha[i] -= 0x14;
                    if t.alpha[i] <= 0 {
                        t.alpha[i] = 0;
                        t.tail = i;
                    }
                }
                i = (i + 1) & 0x7F;
            }
            if c.weapon_out {
                let (base, tip) =
                    find_def(c.chara).map_or(([0.0, 0.0, -1.5], [0.0, 0.0, -10.5]), |d| (d.1, d.2));
                let m = (c.node_mtx)(0x40).unwrap_or(crate::battle::IDENTITY);
                let p = |v: V3| {
                    let r = apply(&m, [v[0], v[1], v[2], 1.0]);
                    [r[0], r[1], r[2]]
                };
                add_point(t, p(base), p(tip), true);
            } else {
                t.was_out = false;
            }
        }
    }

    /// `EftBlade_Draw` of both trails, not for a hidden fighter.
    pub(crate) fn draw(&self, chars: [BladeChr; 2], ot: &mut OrderTable, ctx: &DrawCtx) {
        for (t, c) in self.trails.iter().zip(chars) {
            if !c.hidden {
                draw_trail(t, c.chara, ot, ctx);
            }
        }
    }
}

/// `EftBlade_AddPoint` (the weapon is out).
fn add_point(t: &mut Trail, a: V3, b: V3, out: bool) {
    let mut head = t.head;
    let p = |k: usize| (head + 0x80 - k) & 0x7F;
    let (p1, p2, p3, p4) = (p(1), p(2), p(3), p(4));
    let mut tail = t.tail;
    if t.was_out != out && !t.was_out {
        t.last_a = [a; 4];
        t.last_b = [b; 4];
    }
    t.was_out = out;
    t.last_a.copy_within(1..4, 0);
    t.last_b.copy_within(1..4, 0);
    t.last_a[3] = a;
    t.last_b[3] = b;
    let next = |head: &mut usize, tail: &mut usize| {
        *head = (*head + 1) & 0x7F;
        if *head == *tail {
            *tail = (*tail + 1) & 0x7F;
        }
    };
    if head == tail || [p1, p2, p3, p4].contains(&tail) {
        t.edge_a[head] = a;
        t.edge_b[head] = b;
        t.alpha[head] = 0x51;
        next(&mut head, &mut tail);
    } else {
        let d = sub(t.last_a[3], t.last_a[2]);
        if !((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= 1.0) {
            let (sa, sb) = (t.edge_a[p1], t.edge_b[p1]);
            t.edge_a[p1] = spline(&t.last_a, 0.1);
            t.edge_b[p1] = spline(&t.last_b, 0.1);
            t.alpha[p1] = 0x4C;
            for (k, alpha) in (2..=9).zip((0x4E..).step_by(2)) {
                let f = k as f32 * 0.1;
                t.edge_a[head] = spline(&t.last_a, f);
                t.edge_b[head] = spline(&t.last_b, f);
                t.alpha[head] = alpha;
                next(&mut head, &mut tail);
            }
            t.edge_a[head] = sa;
            t.edge_b[head] = sb;
            t.alpha[head] = 0x5E;
            next(&mut head, &mut tail);
        }
        t.edge_a[head] = a;
        t.edge_b[head] = b;
        t.alpha[head] = 0x60;
        next(&mut head, &mut tail);
    }
    t.head = head;
    t.tail = tail;
}

/// The ribbon from the tail to the entry before the newest: two triangles per step, culled when a corner is off
/// the GS area, in the depth slot of their first corner (clamped to the table).
fn draw_trail(t: &Trail, chara: i32, ot: &mut OrderTable, ctx: &DrawCtx) {
    let head = t.head;
    let p = |k: usize| (head + 0x80 - k) & 0x7F;
    let mut i = t.tail;
    if [head, p(1), p(2), p(3), p(4)].contains(&i) {
        return;
    }
    let Some(def) = find_def(chara) else {
        return;
    };
    let col = def.3;
    let a_scale = col[3] as f32 / 255.0;
    let layer = def.4;
    while i != head {
        i = (i + 1) & 0x7F;
        if i == p(1) {
            break;
        }
        if t.alpha[i] == 0 {
            continue;
        }
        let n = (i + 1) & 0x7F;
        for k in 0..2 {
            let (pos, alpha) = if k & 1 != 0 {
                ([t.edge_a[i], t.edge_b[i], t.edge_a[n]], [0, t.alpha[i], 0])
            } else {
                (
                    [t.edge_b[i], t.edge_a[n], t.edge_b[n]],
                    [t.alpha[i], 0, t.alpha[n]],
                )
            };
            let scr = pos.map(|v| {
                let r = apply(&ctx.world2screen, [v[0], v[1], v[2], 1.0]);
                let q = 1.0 / r[3];
                (
                    (r[0] * q * 16.0) as i32,
                    (r[1] * q * 16.0) as i32,
                    (r[2] * q) as i32,
                )
            });
            // EftBlade_IsClipped.
            if scr
                .iter()
                .any(|s| s.2 <= 0 || s.0 > 0xFFEF || s.0 <= 0 || s.1 < 1 || s.1 > 0xFFEF)
            {
                continue;
            }
            let vert = |j: usize| EffectVertex {
                position: [
                    scr[j].0 as f32 / 16.0 - OFFSET_X,
                    scr[j].1 as f32 / 16.0 - OFFSET_Y,
                    (scr[j].2 & 0xFF_FFFF) as f32,
                ],
                stq: [0.0, 0.0, 1.0],
                rgba: [
                    col[0] as u8,
                    col[1] as u8,
                    col[2] as u8,
                    (alpha[j] as f32 * a_scale) as u8,
                ],
            };
            let l = if layer >= 2 { layer - 2 } else { layer };
            ot.add(
                depth_slot(scr[0].2 >> 8),
                l,
                EffectTri {
                    texture: None,
                    blend: EffectBlend::of(l, layer >= 2),
                    clamp: false,
                    zwrite: false,
                    alpha_ref: 0,
                    vertices: [vert(0), vert(1), vert(2)],
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_swing_fills_the_ring_with_spline_points_and_fades() {
        let mut t = Trail::default();
        // Five frames still, then a swing: the ring gets one entry a frame, then the eleven of a spline step.
        for _ in 0..5 {
            add_point(&mut t, [0.0; 3], [0.0, 0.0, -10.0], true);
        }
        assert_eq!(t.head, 5);
        add_point(&mut t, [5.0, 0.0, 0.0], [5.0, 0.0, -10.0], true);
        assert_eq!(t.head, 5 + 10);
        assert_eq!(t.alpha[t.head - 1], 0x60);
        assert_eq!(t.alpha[t.head - 2], 0x5E);
        assert_eq!(t.alpha[4], 0x4C);
        // The spline runs over the last segment (both bases at 0), pulled back by the swing's tangent.
        assert!(t.edge_a[5][0] < 0.0 && t.edge_a[5][0] > -1.0);
    }
}
