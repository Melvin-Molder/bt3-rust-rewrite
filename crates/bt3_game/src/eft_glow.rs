//! The power-up glow (`EftGlow*`, src/battle/eft_glow.c and the first part of eft_trail.c): while a fighter is
//! powered up (request 4, until it ends; request 5 removes it) an emitter walks ten configured body nodes and
//! spawns camera-facing textured quads around the fighter's axis, at most 0x41 alive, from a pool of 70 shared by
//! both fighters; each quad follows its node along a spline profile, lags the fighter's turn by a few frames,
//! animates its size in two stages and fades. The settings are common effect entry 7, the textures entries 9 (one
//! image under the palette of the aura type) and 8 (the paired quads').

use crate::battle::{IDENTITY, Mtx, apply};
use crate::eft::{
    DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, V3, V4, add,
    cross, depth_slot, dot, norm, rotate_xyz, scale3, sub,
};
use crate::rng::LibcRandom;

/// `EftGlow_AllocPart`: the quads handed out from the pool.
const POOL: usize = 70;
/// The quads an emitter keeps alive at most.
const LIVE_MAX: i32 = 0x41;

const PI_F: f32 = 3.141_592_7;

// EFT_GLOW_* (the emitter's flags).
const ACTIVE: u32 = 0x01;
const FADING: u32 = 0x02;
const KILL: u32 = 0x04;
const PAIRED: u32 = 0x08;
const TEXTURED: u32 = 0x20;

/// `(f32)rand() / RAND_MAX`.
fn randf(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    b.get(o..o + 4)
        .map_or(0.0, |s| f32::from_le_bytes(s.try_into().unwrap()))
}

fn v4_at(b: &[u8], o: usize) -> V4 {
    std::array::from_fn(|i| f32_at(b, o + i * 4))
}

/// A row of a kind's random ranges (`EftPGlowKind`).
#[derive(Clone, Copy, Debug, Default)]
struct Kind {
    dir_y: [f32; 2],
    dist: [f32; 2],
    size: [f32; 2],
    width_scale: [f32; 2],
    taper: [f32; 2],
    push: [f32; 2],
    offset: [f32; 3],
}

/// A configured node (`EftPGlowNode` / `EftPGlowNodeB`).
#[derive(Clone, Copy, Debug, Default)]
struct Node {
    node: u32,
    shape: i32,
    length: f32,
    width: f32,
    pair_length: f32,
    pair_width: f32,
}

/// Common effect entry 7 (`EftPGlowCfg2` at its start, `EftPGlowCfg` / `EftGlowCfg` after it).
#[derive(Clone, Debug, Default)]
pub(crate) struct GlowCfg {
    stage: f32,
    anim_frames: f32,
    point_y: f32,
    drift_base: f32,
    drift_rand: f32,
    move_scale: f32,
    trail_push: f32,
    life_base: f32,
    life_rand: f32,
    width: [f32; 3],
    trail_width: f32,
    length: [f32; 3],
    trail_length: f32,
    width_mul: [f32; 2],
    length_mul: [f32; 2],
    width_side: [f32; 3],
    length_side: [f32; 3],
    trail_width_by: [[f32; 4]; 2],
    trail_length_by: [[f32; 4]; 2],
    kinds: [Kind; 4],
    fade_in: f32,
    fade_out: f32,
    off: [f32; 3],
    vtx: [[V4; 4]; 2],
    /// By `flipB * 2 + flipA`.
    uv_a: [[V4; 4]; 4],
    uv_b: [[V4; 4]; 2],
    nodes: [Node; 10],
    nodes_b: [Node; 10],
    /// Rows of five colours (0..255) by aura type, and the textured variant's.
    color: Vec<V4>,
    color_tex: Vec<V4>,
    /// `gEftGlow->point` / `dir`: the profile.
    point: [V4; 20],
    dir: [V4; 20],
}

/// `EFT_GLOW_ROW`.
const ROW: usize = 5;

impl GlowCfg {
    /// Reads the block and builds the profile (`EftGlow_BuildTables`); `None` when it is too short.
    pub(crate) fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 0x8F0 + 4 * ROW * 16 {
            return None;
        }
        let f = |o| f32_at(b, o);
        let arr = |o: usize, n: usize| -> Vec<f32> { (0..n).map(|i| f(o + i * 4)).collect() };
        let a2 = |o| [f(o), f(o + 4)];
        let a3 = |o| [f(o), f(o + 4), f(o + 8)];
        let kinds = std::array::from_fn(|k| {
            let o = 0xB4 + k * 0x3C;
            Kind {
                dir_y: a2(o),
                dist: a2(o + 8),
                size: a2(o + 0x10),
                width_scale: a2(o + 0x18),
                taper: a2(o + 0x20),
                push: a2(o + 0x28),
                offset: a3(o + 0x30),
            }
        });
        let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let nodes = std::array::from_fn(|n| {
            let o = 0x490 + n * 0x18;
            Node {
                node: i32_at(o) as u32,
                shape: i32_at(o + 4),
                length: f(o + 8),
                width: f(o + 0xC),
                pair_length: f(o + 0x10),
                pair_width: f(o + 0x14),
            }
        });
        let nodes_b = std::array::from_fn(|n| {
            let o = 0x580 + n * 0x10;
            Node {
                length: f(o),
                width: f(o + 4),
                pair_length: f(o + 8),
                pair_width: f(o + 0xC),
                ..nodes[n]
            }
        });
        let quad = |o: usize| -> [V4; 4] { std::array::from_fn(|i| v4_at(b, o + i * 16)) };
        let tbl = |o: usize, n: usize| -> [f32; 4] {
            std::array::from_fn(|i| arr(o, n).get(i).copied().unwrap_or(0.0))
        };
        let mut cfg = Self {
            stage: f(0),
            anim_frames: f(4),
            point_y: f(8),
            drift_base: f(0xC),
            drift_rand: f(0x10),
            move_scale: f(0x14),
            trail_push: f(0x18),
            life_base: f(0x1C),
            life_rand: f(0x20),
            width: a3(0x2C),
            trail_width: f(0x38),
            length: a3(0x3C),
            trail_length: f(0x48),
            width_mul: a2(0x4C),
            length_mul: a2(0x54),
            width_side: a3(0x5C),
            length_side: a3(0x68),
            trail_width_by: [tbl(0x74, 4), tbl(0x84, 4)],
            trail_length_by: [tbl(0x94, 4), tbl(0xA4, 4)],
            kinds,
            fade_in: f(0x1A8),
            fade_out: f(0x1AC),
            off: a3(0x1B0),
            vtx: [quad(0x290), quad(0x2D0)],
            uv_a: [quad(0x310), quad(0x390), quad(0x350), quad(0x3D0)],
            uv_b: [quad(0x410), quad(0x450)],
            nodes,
            nodes_b,
            color: (0..9 * ROW).map(|i| v4_at(b, 0x620 + i * 16)).collect(),
            color_tex: (0..4 * ROW).map(|i| v4_at(b, 0x8F0 + i * 16)).collect(),
            point: [[0.0; 4]; 20],
            dir: [[0.0; 4]; 20],
        };
        cfg.build_tables(
            f(0x1A4),
            &(0..13)
                .map(|i| v4_at(b, 0x1C0 + i * 16))
                .collect::<Vec<_>>(),
        );
        Some(cfg)
    }

    /// `EftGlow_BuildTables`: 20 samples of the quadratic spline through the scaled control points, 20
    /// directions from 20 to 115 degrees.
    fn build_tables(&mut self, scale: f32, pts: &[V4]) {
        let pts: Vec<V4> = pts
            .iter()
            .map(|p| [p[0] * scale, p[1] * scale, p[2] * scale, p[3]])
            .collect();
        let mut win = [pts[0], pts[1], pts[2], pts[3]];
        for i in 0..10 {
            for j in 0..2 {
                self.point[i * 2 + j] = spline3(&win, j as f32 * 0.5);
            }
            // The original's last shift reads past the thirteen points; that window is not sampled again.
            win = [
                win[1],
                win[2],
                win[3],
                pts.get(4 + i).copied().unwrap_or(win[3]),
            ];
        }
        let mut tbl = [0.0f32; 20];
        for (i, t) in tbl.iter_mut().enumerate() {
            *t = if i < 6 {
                i as f32 * 0.5 / 6.0
            } else {
                (i - 6) as f32 * 0.5 / 14.0 + 0.5
            };
        }
        for (d, t) in self.dir.iter_mut().zip(tbl) {
            let a = (20.0 + t * 95.0) * PI_F / 180.0;
            let n = norm([0.0, -a.cos(), a.sin()]);
            *d = [n[0], n[1], n[2], 1.0];
        }
    }

    fn node(&self, p: &Part) -> &Node {
        if p.flags & (0x80 | 0x100) != 0 {
            &self.nodes_b[usize::from(p.node)]
        } else {
            &self.nodes[usize::from(p.node)]
        }
    }

    /// `EftGlow_CalcLength`.
    fn length_of(&self, p: &Part, stage: usize) -> f32 {
        let mut r = self.length[stage];
        if p.flags & 2 != 0 {
            r = self.trail_length * self.trail_length_by[usize::from(p.pair)][stage];
        } else if p.flags & 0x20 != 0 {
            r *= self.length_side[2];
            r *= self.node(p).pair_length;
        } else {
            if p.flags & 0x40 == 0 {
                r *= self.length_side[0];
            }
            r *= self.node(p).length;
        }
        if p.flags & 0x10 != 0 {
            r *= self.length_mul[0];
        }
        if p.flags & 8 != 0 {
            r *= self.length_mul[1];
        }
        r
    }

    /// `EftGlow_CalcWidth`.
    fn width_of(&self, p: &Part, stage: usize) -> f32 {
        let mut r = self.width[stage];
        if p.flags & 2 != 0 {
            r = self.trail_width * self.trail_width_by[usize::from(p.pair)][stage];
        } else if p.flags & 0x20 != 0 {
            r *= self.width_side[2];
            r *= self.node(p).pair_width;
        } else {
            r *= if p.flags & 0x40 != 0 {
                self.width_side[0]
            } else {
                self.width_side[1]
            };
            r *= self.node(p).width;
        }
        if p.flags & 0x10 != 0 {
            r *= self.width_mul[0];
        }
        if p.flags & 8 != 0 {
            r *= self.width_mul[1];
        }
        r
    }
}

/// `EftGlow_Spline3`: the quadratic spline point of three control points at t (w = 1).
fn spline3(p: &[V4; 4], t: f32) -> V4 {
    let tv = [t * t * t, t * t, t, 1.0];
    // basis rows applied as Mtx_MulVec4 (row vectors times the matrix's rows).
    let basis = [
        [0.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, -2.0, 1.0],
        [0.0, -3.0, 4.0, -1.0],
        [0.0, 2.0, 0.0, 0.0],
    ];
    let b = apply(&basis, tv);
    let pts = [[0.0; 4], p[0], p[1], p[2]];
    let v = apply(&pts, b);
    [v[0] / 2.0, v[1] / 2.0, v[2] / 2.0, 1.0]
}

/// What the glow reads of its fighter.
pub(crate) struct GlowChr<'a> {
    pub side: usize,
    pub height: f32,
    /// `BtlCharApi_GetAuraType`.
    pub aura_type: i32,
    pub param_flags: u16,
    /// `BtlCharApi_GetActionFxKind`.
    pub fx_kind: i32,
    pub dir: V3,
    pub rot: V3,
    pub fall_speed: f32,
    /// `BtlCharApi_GetFrameMove`.
    pub moved: V3,
    pub node_pos: &'a dyn Fn(u32) -> V3,
}

/// A quad (`EftPGlowPart`).
#[derive(Clone, Copy, Debug, Default)]
struct Part {
    flags: u32,
    pair: u8,
    kind: u8,
    tex: u8,
    chr: usize,
    frame: u8,
    flip_a: u8,
    flip_b: u8,
    anim_timer: u8,
    layer: u8,
    node: u8,
    shape: u8,
    life: f32,
    life_max: f32,
    size: f32,
    width: f32,
    length: f32,
    width_scale: f32,
    taper: f32,
    d_width: f32,
    d_length: f32,
    dist: f32,
    angle: f32,
    dir_y: f32,
    pos: V4,
    color: V4,
    d_color: V4,
    offset: V4,
    point: V4,
    dir: V4,
    world_dir: V4,
}

/// A fighter's emitter (`EftPGlow` / `EftGlow`).
#[derive(Clone, Debug, Default)]
struct Emitter {
    flags: u32,
    node_mask: u32,
    param_flags: u16,
    kind: i32,
    step: u8,
    node: i32,
    count: i32,
    aura_type: i32,
    layers: i32,
    tex_count: i32,
    live: i32,
    fade: u8,
    alpha: f32,
    alpha_a: f32,
    alpha_b: f32,
    scale: f32,
    chr: usize,
    frame: [Mtx; 5],
    color_a: V4,
    color_b: V4,
    node_pos: [V4; 10],
    offset: V4,
    color: [V4; 4],
}

/// The glow's textures among the effect textures: entry 9's image under each aura type's palette, the textured
/// variant's two (image 8 under palettes 8 and 9), entry 8's first (the paired quads').
#[derive(Clone, Debug, Default)]
pub(crate) struct GlowTextures {
    pub plain: Vec<Option<usize>>,
    pub textured: [Option<usize>; 2],
    pub pair: Option<usize>,
}

/// `gEftGlow`: the pool (`parts`, the free list as a stack, the active list newest first) and the emitters in
/// the order they were created.
#[derive(Clone, Debug, Default)]
pub(crate) struct Glows {
    parts: Vec<Part>,
    free: Vec<usize>,
    active: Vec<usize>,
    emitters: Vec<Emitter>,
}

impl Glows {
    /// The fighter's quads alive, and whether it has an emitter.
    #[cfg(test)]
    pub(crate) fn state(&self, chr: usize) -> (usize, bool) {
        (
            self.active
                .iter()
                .filter(|&&i| self.parts[i].chr == chr)
                .count(),
            self.emitters.iter().any(|e| e.chr == chr),
        )
    }

    /// `EftGlow_IsActive`.
    pub(crate) fn is_active(&self, chr: usize) -> bool {
        self.emitters
            .iter()
            .any(|e| e.chr == chr && e.flags & ACTIVE != 0)
    }

    /// `EftGlow_Request`: 0 start (or restart), 1 fade out, 2 remove.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn request(&mut self, cfg: &GlowCfg, c: &GlowChr, cmd: u8, libc: &mut LibcRandom) {
        let at = self.emitters.iter().position(|e| e.chr == c.side);
        match cmd {
            0 => match at {
                None => {
                    // EftGlowTask_Init: EftGlow_Setup, EftGlow_Begin.
                    let mut e = Emitter::default();
                    setup(&mut e, cfg, c);
                    begin(&mut e, cfg, c, libc);
                    self.emitters.push(e);
                }
                Some(i) => begin(&mut self.emitters[i], cfg, c, libc),
            },
            1 => {
                if let Some(e) = at.map(|i| &mut self.emitters[i])
                    && e.flags & ACTIVE != 0
                    && e.flags & FADING == 0
                {
                    e.flags |= FADING;
                    e.fade = 2;
                    // fadeFrom (the alpha's word): 1.
                    e.alpha = 1.0;
                    e.node = 9;
                }
            }
            _ => {
                if let Some(e) = at.map(|i| &mut self.emitters[i])
                    && e.flags & ACTIVE != 0
                {
                    e.flags |= KILL;
                }
            }
        }
    }

    /// `EftGlowTask_Update` of every emitter: the fighter's type again; unless paused its step, the quads';
    /// an emitter done fading (or removed) frees its quads.
    pub(crate) fn update(
        &mut self,
        cfg: &GlowCfg,
        chars: &[GlowChr; 2],
        paused: bool,
        libc: &mut LibcRandom,
    ) {
        let mut i = 0;
        while i < self.emitters.len() {
            let c = &chars[self.emitters[i].chr];
            setup(&mut self.emitters[i], cfg, c);
            if !paused {
                self.step(i, cfg, c, libc);
                let e = &mut self.emitters[i];
                e.step = (e.step + 1).min(2);
                self.step_parts(i, cfg, c, libc);
            }
            if self.emitters[i].flags & KILL != 0 {
                // EftGlowTask_Term: EftGlow_FreeParts.
                let chr = self.emitters[i].chr;
                self.free_parts(chr);
                self.emitters.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// `EftGlow_AllocPart`: the free list, else the next unused quad of the pool; linked at the head of the
    /// active list.
    fn alloc(&mut self) -> Option<usize> {
        let i = if let Some(i) = self.free.pop() {
            i
        } else if self.parts.len() < POOL {
            self.parts.push(Part::default());
            self.parts.len() - 1
        } else {
            return None;
        };
        self.active.insert(0, i);
        self.parts[i].flags = 0;
        Some(i)
    }

    /// `EftGlow_FreeParts`.
    fn free_parts(&mut self, chr: usize) {
        let mut k = 0;
        while k < self.active.len() {
            let i = self.active[k];
            if self.parts[i].chr == chr {
                self.active.remove(k);
                self.free.push(i);
            } else {
                k += 1;
            }
        }
    }

    /// `EftGlow_Step`: the node positions, the orientation history, the emission, the fade.
    fn step(&mut self, i: usize, cfg: &GlowCfg, c: &GlowChr, libc: &mut LibcRandom) {
        let e = &mut self.emitters[i];
        for (p, n) in e.node_pos.iter_mut().zip(&cfg.nodes) {
            let v = (c.node_pos)(n.node);
            *p = [v[0], v[1], v[2], 1.0];
        }
        // EftGlow_UpdateFrames.
        if e.flags & FADING == 0 {
            e.frame[0] = calc_frame(c, e.kind);
        }
        for k in (1..5).rev() {
            e.frame[k] = e.frame[k - 1];
        }
        // EftGlow_Emit.
        let live = e.live;
        if live < LIVE_MAX {
            let mut n = 14;
            if live + n >= LIVE_MAX + 1 {
                n = LIVE_MAX - live;
            }
            for _ in 0..n {
                self.spawn_next(i, cfg, c, libc);
            }
        }
        let e = &mut self.emitters[i];
        match e.fade {
            1 => {
                e.alpha += 1.0 / cfg.fade_in;
                if e.alpha >= 1.0 {
                    e.alpha = 1.0;
                    e.fade = 0;
                }
            }
            2 => {
                e.alpha -= 1.0 / cfg.fade_out;
                if e.alpha <= 0.0 {
                    e.alpha = 0.0;
                    e.flags |= KILL;
                    e.fade = 0;
                }
            }
            _ => {}
        }
    }

    /// `EftGlow_SpawnNext`: the next quad at the emitter's node (and sometimes its partner); the node walk.
    fn spawn_next(&mut self, i: usize, cfg: &GlowCfg, c: &GlowChr, libc: &mut LibcRandom) {
        let (node, n) = (self.emitters[i].node, self.emitters[i].count as f32);
        let wrap_lo = |a: f32| if a <= -180.0 { a + 360.0 } else { a };
        let wrap_hi = |a: f32| if a >= 180.0 { a - 360.0 } else { a };
        let angle = match node {
            3 => wrap_lo(n * -25.0 + -40.0 + randf(libc) * -25.0),
            4 => wrap_hi(n * 25.0 + 40.0 + randf(libc) * 25.0),
            6 | 8 => wrap_lo(n * -35.0 + -20.0 + randf(libc) * -35.0),
            7 | 9 => wrap_hi(n * 35.0 + 20.0 + randf(libc) * 35.0),
            _ => {
                let mut a = n * 25.0 + -25.0 + randf(libc) * 25.0;
                if n >= 2.0 {
                    a += 130.0;
                }
                wrap_hi(a)
            }
        };
        let e = &mut self.emitters[i];
        let off = [
            e.offset[0] * e.scale,
            e.offset[1] * e.scale,
            e.offset[2] * e.scale,
            1.0,
        ];
        let bit = 1 << node;
        let odd = if e.node_mask & bit != 0 {
            e.node_mask &= !bit;
            true
        } else {
            e.node_mask |= bit;
            false
        };
        let side = e.flags & FADING == 0 && odd;
        let Some(p) = self.spawn_part(i, cfg, c, angle, off, side, false, libc) else {
            return;
        };
        let e = &self.emitters[i];
        if e.live < LIVE_MAX {
            let pair = if e.flags & FADING != 0 {
                true
            } else if e.flags & PAIRED == 0 {
                e.count >= 3 || libc.rand() & 1 == 0
            } else {
                libc.rand() % 5 == 0
            };
            if pair && let Some(q) = self.spawn_part(i, cfg, c, angle, off, false, true, libc) {
                self.emitters[i].flags |= PAIRED;
                if self.emitters[i].flags & FADING != 0 {
                    self.pair_part(i, cfg, q, p, libc);
                }
            }
        }
        let e = &mut self.emitters[i];
        e.count += 1;
        if e.count >= 4 {
            e.count = 0;
            if e.flags & FADING != 0 {
                e.node -= 1;
                if e.node < 0 {
                    e.node = 7;
                }
            } else {
                e.flags &= !PAIRED;
                e.node += 1;
                if e.node >= 10 {
                    e.node = 0;
                    e.offset[0] = cfg.off[0] * (randf(libc) - randf(libc));
                    e.offset[1] = cfg.off[1] * (randf(libc) - randf(libc));
                }
            }
        }
    }

    /// `EftGlow_SpawnPart`: one quad at the emitter's node (eleven or more libc draws, the first even without a
    /// free quad).
    #[allow(clippy::too_many_arguments)]
    fn spawn_part(
        &mut self,
        i: usize,
        cfg: &GlowCfg,
        c: &GlowChr,
        angle: f32,
        scale: V4,
        odd: bool,
        pair: bool,
        libc: &mut LibcRandom,
    ) -> Option<usize> {
        let slot = self.alloc();
        let e = &self.emitters[i];
        let shape = cfg.nodes[e.node as usize].shape;
        let idx = (libc.rand() % e.layers.max(1)) as usize;
        let slot = slot?;
        let mut p = Part {
            flags: part_flags(e, pair, odd),
            pair: u8::from(pair),
            chr: c.side,
            node: e.node as u8,
            angle,
            ..Part::default()
        };
        p.kind = if p.flags & 2 != 0 {
            3
        } else if p.flags & 0x20 != 0 {
            2
        } else if p.flags & 0x40 != 0 {
            0
        } else {
            1
        };
        p.life = cfg.life_base + cfg.life_rand * randf(libc);
        if p.flags & 8 != 0 {
            p.life *= 0.9;
        }
        p.life_max = p.life;
        p.width = cfg.width_of(&p, 0);
        p.length = cfg.length_of(&p, 0);
        p.d_width = (cfg.width_of(&p, 1) - p.width) / (p.life * cfg.stage);
        p.d_length = (cfg.length_of(&p, 1) - p.length) / (p.life * cfg.stage);
        if p.flags & 0x10 != 0 {
            p.tex = (libc.rand() % e.tex_count.max(1)) as u8;
        }
        let alpha = part_alpha(e, p.flags, idx);
        let k = cfg.kinds[usize::from(p.kind)];
        p.dir_y = k.dir_y[0] + k.dir_y[1] * randf(libc);
        p.dist = k.dist[0] + k.dist[1] * randf(libc);
        p.size = (k.size[0] + k.size[1] * randf(libc)) * e.scale;
        p.width_scale = k.width_scale[0] + k.width_scale[1] * randf(libc);
        p.taper = k.taper[0] + k.taper[1] * randf(libc);
        p.offset[3] = k.push[0] + k.push[1] * randf(libc);
        p.offset[0] = scale[0] * k.offset[0];
        p.offset[1] = scale[1] * k.offset[1];
        p.offset[2] = scale[2] * k.offset[2];
        let node_rule = |p: &mut Part| match p.node {
            0 => p.dist *= 0.5,
            1 => p.dist *= 0.65,
            3 | 4 => {
                p.offset[2] *= 0.65;
                p.dist *= 0.5;
            }
            _ => {}
        };
        let shape = if p.flags & 2 != 0 {
            let from = if p.flags & 0x20 != 0 {
                p.layer = 1;
                e.color[idx]
            } else {
                p.layer = 0;
                e.color_a
            };
            let target = if p.flags & 0x20 != 0 {
                e.color[idx]
            } else {
                e.color_b
            };
            p.color = [from[0], from[1], from[2], 0.0];
            let inv = 1.0 / p.life_max;
            p.d_color = [
                (target[0] - p.color[0]) * inv,
                (target[1] - p.color[1]) * inv,
                (target[2] - p.color[2]) * inv,
                alpha * (1.0 / cfg.fade_in),
            ];
            p.offset[2] *= randf(libc) * 1.5 + 1.0;
            shape / 2 + libc.rand() % 2
        } else if p.flags & 0x20 != 0 {
            p.layer = 1;
            let col = e.color[idx];
            p.color = [col[0], col[1], col[2], 0.0];
            p.d_color = [0.0, 0.0, 0.0, alpha * (1.0 / cfg.fade_in)];
            if matches!(e.kind, 3 | 4) {
                p.dist *= 0.3;
                node_rule(&mut p);
            }
            p.dist += 0.1;
            p.size *= 1.3;
            shape + libc.rand() % 3
        } else {
            p.layer = 0;
            let s = randf(libc) * 0.2 + 1.0;
            p.color = [e.color_a[0] * s, e.color_a[1] * s, e.color_a[2] * s, 0.0];
            let inv = 1.0 / p.life_max;
            p.d_color = [
                (e.color_b[0] - p.color[0]) * inv,
                (e.color_b[1] - p.color[1]) * inv,
                (e.color_b[2] - p.color[2]) * inv,
                alpha * (1.0 / cfg.fade_in),
            ];
            if matches!(e.kind, 3 | 4) {
                p.dir_y *= 1.7;
                p.dist *= 0.9;
                node_rule(&mut p);
            } else if matches!(p.node, 3 | 4) {
                p.dist *= 0.7;
            }
            p.dist *= 0.85;
            p.size *= 1.6;
            shape + libc.rand() % 4
        };
        p.shape = shape.clamp(0, 19) as u8;
        p.dir = shape_dir(cfg, usize::from(p.shape), p.dir_y);
        p.frame = 0;
        p.flip_a = (libc.rand() % 2) as u8;
        p.flip_b = (libc.rand() % 2) as u8;
        p.anim_timer = (libc.rand() % (cfg.anim_frames as i32).max(1)) as u8;
        self.parts[slot] = p;
        Some(slot)
    }

    /// `EftGlow_PairPart`: `q` becomes the partner of the trailing quad `p` (one libc draw).
    fn pair_part(&mut self, i: usize, cfg: &GlowCfg, q: usize, p: usize, libc: &mut LibcRandom) {
        let e = &self.emitters[i];
        let idx = (libc.rand() % e.layers.max(1)) as usize;
        let src = self.parts[p];
        let mut d = self.parts[q];
        d.flip_a = src.flip_a;
        d.shape = src.shape;
        d.life = src.life * 1.5;
        d.life_max = src.life_max * 1.5;
        d.d_width = (cfg.width_of(&d, 1) - d.width) / (d.life * cfg.stage);
        d.d_length = (cfg.length_of(&d, 1) - d.length) / (d.life * cfg.stage);
        d.dist = src.dist;
        d.dir_y = src.dir_y;
        d.dir = src.dir;
        d.offset = src.offset;
        let s = e.scale * 4.0;
        for k in 0..3 {
            d.offset[k] -= d.dir[k] * s;
            d.d_color[k] = (e.color[idx][k] - d.color[k]) / d.life_max;
        }
        self.parts[q] = d;
    }

    /// `EftGlow_StepParts`: every quad of the fighter follows its node along the profile, turned about the axis
    /// and by the orientation of `frame` frames ago, leaning by the drift; its size and colour step; it dies with
    /// its life.
    fn step_parts(&mut self, i: usize, cfg: &GlowCfg, c: &GlowChr, libc: &mut LibcRandom) {
        self.emitters[i].live = 0;
        let drift = calc_drift(cfg, c, libc);
        let mut k = 0;
        while k < self.active.len() {
            let slot = self.active[k];
            if self.parts[slot].chr != c.side {
                k += 1;
                continue;
            }
            if !(self.parts[slot].life > 0.0) {
                self.active.remove(k);
                self.free.push(slot);
                continue;
            }
            let e = &mut self.emitters[i];
            let p = &mut self.parts[slot];
            let frame = e.frame[usize::from(p.frame)];
            p.frame = (p.frame + 1).min(4);
            let a = apply(&frame, p.offset);
            let np = e.node_pos[usize::from(p.node)];
            p.pos = std::array::from_fn(|j| np[j] + a[j]);
            let pt = cfg.point[usize::from(p.shape)];
            let s = e.scale * p.dist;
            p.point = [pt[0] * s, pt[1] * s * cfg.point_y, pt[2] * s, 1.0];
            p.dir = shape_dir(cfg, usize::from(p.shape), p.dir_y);
            let rot = rotate_xyz(&IDENTITY, [0.0, 0.0, p.angle * PI_F / 180.0]);
            let b = apply(&rot, p.point);
            p.dir = apply(&rot, p.dir);
            let b = apply(&frame, b);
            p.pos = [p.pos[0] + b[0], p.pos[1] + b[1], p.pos[2] + b[2], 1.0];
            p.world_dir = apply(&frame, p.dir);
            if p.flags & 2 == 0 {
                let w = add(
                    [p.world_dir[0], p.world_dir[1], p.world_dir[2]],
                    [drift[0], drift[1], drift[2]],
                );
                let n = norm(w);
                p.world_dir = [n[0], n[1], n[2], p.world_dir[3] + drift[3]];
            }
            p.width = (p.width + p.d_width).max(0.0);
            p.length = (p.length + p.d_length).max(0.0);
            if p.flags & 4 == 0 && cfg.stage < 1.0 - p.life / p.life_max {
                p.d_width = (cfg.width_of(p, 2) - p.width) / p.life;
                p.d_length = (cfg.length_of(p, 2) - p.length) / p.life;
                p.flags |= 4;
            }
            if !(p.life_max - p.life < cfg.fade_in) {
                if p.life <= cfg.fade_out {
                    if p.flags & 1 == 0 {
                        let inv = 1.0 / (p.life + 1.0);
                        let d = p.color[3] - e.color_b[3];
                        p.flags |= 1;
                        p.d_color[3] = -d * inv;
                    }
                } else {
                    p.d_color[3] = 0.0;
                }
            }
            p.color = std::array::from_fn(|j| (p.color[j] + p.d_color[j]).clamp(0.0, 1.0));
            if p.flags & 2 != 0 {
                let t = p.life / p.life_max;
                let s = cfg.trail_push * (t * (t * t * t) * (p.offset[3] * e.scale));
                // Vec3_Scale keeps the direction's w (1), which Vec4_Add adds to the push.
                for j in 0..3 {
                    p.offset[j] += p.dir[j] * s;
                }
                p.offset[3] += p.dir[3];
            }
            p.anim_timer = p.anim_timer.wrapping_add(1);
            if f32::from(p.anim_timer) >= cfg.anim_frames {
                p.anim_timer = 0;
                p.flip_b += 1;
                if p.flip_b >= 2 {
                    p.flip_b = 0;
                }
            }
            p.life -= 1.0;
            e.live += 1;
            k += 1;
        }
    }

    /// `EftGlowTask_Draw` of every emitter (not of a hidden fighter; at 0.3 alpha in its own view with object flag
    /// bit 21): each of its quads but the trailing ones, facing the camera, projected straight to the GS.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &self,
        cfg: &GlowCfg,
        tex: &GlowTextures,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
        alpha_of: impl Fn(usize) -> Option<f32>,
        circle_dash: [bool; 2],
    ) {
        for e in &self.emitters {
            let Some(alpha) = alpha_of(e.chr) else {
                continue;
            };
            for &slot in &self.active {
                let p = &self.parts[slot];
                if p.chr != e.chr || p.flags & 2 != 0 {
                    continue;
                }
                let texture = if p.flags & 0x20 != 0 {
                    tex.pair
                } else if e.flags & TEXTURED != 0 {
                    tex.textured.get(usize::from(p.tex)).copied().flatten()
                } else {
                    usize::try_from(e.aura_type)
                        .ok()
                        .and_then(|t| tex.plain.get(t).copied().flatten())
                };
                let layer = if circle_dash[p.chr] { 1 } else { p.layer };
                draw_part(cfg, e, p, alpha, texture, layer, ot, ctx);
            }
        }
    }
}

/// `EftGlow_Setup` / `EftGlow_SetType`: the variant and colours of the fighter's aura type (8..10 the textured
/// variant, all read as 8 with colour row 0), the scale.
fn setup(e: &mut Emitter, cfg: &GlowCfg, c: &GlowChr) {
    e.param_flags = c.param_flags;
    e.flags &= !TEXTURED;
    let mut ty = c.aura_type;
    if (8..=10).contains(&ty) {
        e.flags |= TEXTURED;
        ty = 8;
        e.tex_count = 2;
    }
    e.aura_type = ty;
    e.layers = if e.flags & TEXTURED != 0 { 2 } else { 1 };
    let byte = |v: V4| v.map(|x| x * 0.003_921_568_6);
    let row =
        |table: &[V4], r: usize, k: usize| table.get(r * ROW + k).copied().unwrap_or([0.0; 4]);
    let (table, r) = if e.flags & TEXTURED != 0 {
        (&cfg.color_tex, 0usize)
    } else {
        (&cfg.color, ty.max(0) as usize)
    };
    e.color_a = byte(row(table, r, 0));
    e.color_b = byte(row(table, r, 0));
    e.color_b[3] = 0.0;
    for i in 0..4 {
        let j = if e.flags & TEXTURED != 0 || ty >= 8 {
            i
        } else {
            0
        };
        e.color[i] = byte(row(table, r, j + 1));
    }
    e.scale = c.height / 19.35;
}

/// `EftGlow_Begin` (four libc draws): the emitter restarts, fading in.
fn begin(e: &mut Emitter, cfg: &GlowCfg, c: &GlowChr, libc: &mut LibcRandom) {
    e.flags = ACTIVE;
    e.kind = c.fx_kind;
    e.step = 0;
    e.chr = c.side;
    e.alpha_a = 48.0 / 255.0;
    e.alpha_b = 32.0 / 255.0;
    e.count = 0;
    for (p, n) in e.node_pos.iter_mut().zip(&cfg.nodes) {
        let v = (c.node_pos)(n.node);
        *p = [v[0], v[1], v[2], 1.0];
    }
    e.offset[0] = cfg.off[0] * (randf(libc) - randf(libc));
    e.offset[1] = cfg.off[1] * (randf(libc) - randf(libc));
    e.offset[2] = cfg.off[2];
    let m = calc_frame(c, e.kind);
    e.frame = [m; 5];
    e.fade = 1;
    e.alpha = 0.0;
    e.live = 0;
    e.scale = c.height / 19.35;
}

/// `EftGlow_CalcFrame`: the pitch of the fighter's direction (with the fall speed -1 / +1 for kinds 3 / 4) and
/// its yaw + pi; kind 5 the line from node 0x2E to node 3.
fn calc_frame(c: &GlowChr, kind: i32) -> Mtx {
    if kind != 5 {
        let mut dir = c.dir;
        if kind == 3 {
            dir[1] += c.fall_speed + -1.0;
        } else if kind == 4 {
            dir[1] += c.fall_speed + 1.0;
        }
        let dir = norm(dir);
        let m = rotate_xyz(&IDENTITY, [dir[1].asin(), 0.0, 0.0]);
        let mut yaw = c.rot[1] + PI_F;
        if yaw > PI_F {
            yaw -= 6.283_185_3;
        }
        rotate_xyz(&m, [0.0, yaw, 0.0])
    } else {
        let d = norm(sub((c.node_pos)(3), (c.node_pos)(0x2E)));
        let m = rotate_xyz(&IDENTITY, [(-d[1]).asin(), 0.0, 0.0]);
        rotate_xyz(&m, [0.0, d[0].atan2(d[2]), 0.0])
    }
}

/// `EftGlow_CalcDrift` (one libc draw): a random length along node 0x30 -> node 3, plus the frame movement
/// scaled and clamped to +-0.5.
fn calc_drift(cfg: &GlowCfg, c: &GlowChr, libc: &mut LibcRandom) -> V4 {
    let s = cfg.drift_base + cfg.drift_rand * randf(libc);
    let d = scale3(norm(sub((c.node_pos)(3), (c.node_pos)(0x30))), s);
    let m = scale3(c.moved, cfg.move_scale).map(|v| v.clamp(-0.5, 0.5));
    [m[0] + d[0], m[1] + d[1], m[2] + d[2], 0.0]
}

/// `EftGlow_GetShapeDir`.
fn shape_dir(cfg: &GlowCfg, idx: usize, y: f32) -> V4 {
    let d = cfg.dir[idx];
    [d[0], d[1] * y, d[2], d[3]]
}

/// `EftGlow_CalcPartFlags`.
fn part_flags(e: &Emitter, pair: bool, odd: bool) -> u32 {
    let mut f = e.flags & FADING;
    if pair {
        f |= 0x20;
    } else if odd {
        f |= 0x40;
    }
    if e.kind == 3 {
        f |= 0x80;
    } else if e.kind == 4 {
        f |= 0x100;
    }
    if e.flags & TEXTURED != 0 {
        f |= 0x10;
    }
    f
}

/// `EftGlow_GetAlpha`.
fn part_alpha(e: &Emitter, flags: u32, idx: usize) -> f32 {
    let mut a = if flags & 2 != 0 {
        if flags & 0x20 != 0 {
            e.alpha_b
        } else {
            e.alpha_a
        }
    } else if flags & 0x20 != 0 {
        e.color[idx][3]
    } else {
        e.color_a[3]
    };
    if flags & 8 != 0 {
        a *= 1.1;
    }
    a
}

/// `EftGlow_MakeFacingMtx` and the drawing of one quad: its direction bent towards the camera by 0.4 of their dot
/// product; four corners projected (`Vu0Cur_ProjectPoint`), the quad dropped when one is off the GS range or
/// behind the camera; a strip at a quarter of the depths' sum.
#[allow(clippy::too_many_arguments)]
fn draw_part(
    cfg: &GlowCfg,
    e: &Emitter,
    p: &Part,
    alpha: f32,
    texture: Option<usize>,
    layer: u8,
    ot: &mut OrderTable,
    ctx: &DrawCtx,
) {
    let v = &ctx.world2view;
    let cam = [v[0][2], v[1][2], v[2][2]];
    let dir = [p.world_dir[0], p.world_dir[1], p.world_dir[2]];
    let d = dot(cam, dir);
    let (t, s) = if d < 0.0 {
        (scale3(cam, -1.0), -d * 0.4)
    } else {
        (cam, d * 0.4)
    };
    let z = norm(add(dir, scale3(norm(sub(dir, t)), s)));
    let x = norm(cross(cam, z));
    let y = norm(cross(z, x));
    let mut m = IDENTITY;
    for r in 0..3 {
        m[r] = [x[r], y[r], z[r], 0.0];
    }
    m[3] = [p.pos[0], p.pos[1], p.pos[2], 1.0];
    let sx = p.size * p.width * p.width_scale;
    let sz = p.size * p.length;
    let mut scr = [(0i32, 0i32, 0i32, 0.0f32); 4];
    let mut stq = [[0.0f32; 3]; 4];
    for k in 0..4 {
        let c = cfg.vtx[usize::from(p.pair)][k];
        let mut vz = c[2] * sz;
        if k >= 2 {
            vz *= p.taper;
        }
        let w = apply(&m, [c[0] * sx, c[1] * p.size, vz, 1.0]);
        let s = apply(&ctx.world2screen, w);
        let q = 1.0 / s[3];
        let (gx, gy, gz, gw) = (
            (s[0] * q * 16.0) as i32,
            (s[1] * q * 16.0) as i32,
            (s[2] * q) as i32,
            s[3] as i32,
        );
        if gx as u32 > 0xFFF0 || gy as u32 > 0xFFF0 || gz < 0 {
            return;
        }
        scr[k] = (gx, gy, gz, 0.0);
        let q = 1.0 / gw as f32;
        let uv = if p.flags & 0x20 != 0 {
            cfg.uv_b[usize::from(p.flip_a)][k]
        } else {
            cfg.uv_a[usize::from(p.flip_b * 2 + p.flip_a)][k]
        };
        stq[k] = [uv[0] * q, uv[1] * q, uv[2] * q];
    }
    let col = [p.color[0], p.color[1], p.color[2]].map(|c| (c * 255.0) as i32 as u8);
    let a = (p.color[3] * 255.0 * e.alpha * alpha) as i32 as u8;
    let rgba = [col[0], col[1], col[2], a];
    let zsum = scr
        .iter()
        .fold(0i32, |s, v| s.wrapping_add(v.2))
        .wrapping_shr(10);
    let vert = |k: usize| EffectVertex {
        position: [
            scr[k].0 as f32 / 16.0 - OFFSET_X,
            scr[k].1 as f32 / 16.0 - OFFSET_Y,
            (scr[k].2 & 0xFF_FFFF) as f32,
        ],
        stq: stq[k],
        rgba,
    };
    let l = usize::from(if layer >= 2 { layer - 2 } else { layer });
    for k in 0..2 {
        ot.add(
            depth_slot(zsum),
            l,
            EffectTri {
                texture,
                blend: EffectBlend::of(l, false),
                clamp: false,
                zwrite: false,
                alpha_ref: 0,
                vertices: [vert(k), vert(k + 1), vert(k + 2)],
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block of entry 7's layout with plain values.
    fn block() -> Vec<u8> {
        let mut b = vec![0u8; 0x8F0 + 20 * 16];
        let mut put = |o: usize, v: f32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put(0, 0.5); // stage
        put(4, 3.0); // animFrames
        put(8, 1.0); // pointY
        put(0x1C, 10.0); // lifeBase
        for o in [0x2C, 0x30, 0x34, 0x3C, 0x40, 0x44] {
            put(o, 1.0);
        }
        for o in [0x5C, 0x60, 0x64, 0x68, 0x6C, 0x70] {
            put(o, 1.0);
        }
        for k in 0..4 {
            put(0xB4 + k * 0x3C + 0x10, 1.0); // size
            put(0xB4 + k * 0x3C + 0x20, 1.0); // taper
        }
        put(0x1A4, 1.0); // shapeScale
        put(0x1A8, 2.0); // fadeIn
        put(0x1AC, 2.0); // fadeOut
        for n in 0..10 {
            b[0x490 + n * 0x18..0x490 + n * 0x18 + 4]
                .copy_from_slice(&(3 + n as i32).to_le_bytes());
            for k in 2..6 {
                let o = 0x490 + n * 0x18 + k * 4;
                b[o..o + 4].copy_from_slice(&1.0f32.to_le_bytes());
            }
        }
        // Colour row 2: base 255, 255, 255, 128.
        for (i, v) in [255.0f32, 255.0, 255.0, 128.0].into_iter().enumerate() {
            let o = 0x620 + 2 * ROW * 16 + i * 4;
            b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    fn chr(node_pos: &dyn Fn(u32) -> V3) -> GlowChr<'_> {
        GlowChr {
            side: 0,
            height: 19.35,
            aura_type: 2,
            param_flags: 0,
            fx_kind: 1,
            dir: [0.0, 0.0, 1.0],
            rot: [0.0; 3],
            fall_speed: 0.0,
            moved: [0.0; 3],
            node_pos,
        }
    }

    #[test]
    fn the_profile_and_the_settings_read() {
        let cfg = GlowCfg::parse(&block()).unwrap();
        assert_eq!(cfg.fade_in, 2.0);
        assert_eq!(cfg.nodes[2].node, 5);
        // Directions from 20 degrees down to 115, unit length.
        let d0 = cfg.dir[0];
        assert!((d0[1] + (20.0f32 * PI_F / 180.0).cos()).abs() < 1e-5);
        assert!(
            cfg.dir
                .iter()
                .all(|d| (dot([d[0], d[1], d[2]], [d[0], d[1], d[2]]) - 1.0).abs() < 1e-5)
        );
        assert!(GlowCfg::parse(&[0; 16]).is_none());
    }

    #[test]
    fn the_glow_fills_its_quads_fades_out_and_frees_them() {
        let cfg = GlowCfg::parse(&block()).unwrap();
        let node_pos = |n: u32| [0.0, -(n as f32), 100.0];
        let c = chr(&node_pos);
        let other = GlowChr {
            side: 1,
            ..chr(&node_pos)
        };
        let chars = [c, other];
        let mut g = Glows::default();
        let mut libc = LibcRandom::new(1);
        g.request(&cfg, &chars[0], 0, &mut libc);
        assert!(g.is_active(0));
        // The colour row's bytes as 0..1 (x 0.0039215686, the original's constant).
        let a = g.emitters[0].color_a;
        assert!(
            (a[0] - 1.0).abs() < 1e-6 && (a[3] - 128.0 / 255.0).abs() < 1e-6,
            "{a:?}"
        );
        for _ in 0..10 {
            g.update(&cfg, &chars, false, &mut libc);
        }
        // Spawning stops at 0x41 alive (counted the frame before, partners on top), within the pool.
        let (live, on) = g.state(0);
        assert!(live > LIVE_MAX as usize / 2 && live <= POOL && on, "{live}");
        // Faded in.
        assert_eq!(g.emitters[0].alpha, 1.0);
        g.request(&cfg, &chars[0], 1, &mut libc);
        assert_eq!(g.emitters[0].node, 9);
        for _ in 0..3 {
            g.update(&cfg, &chars, false, &mut libc);
        }
        // Faded out in two frames: the emitter and its quads are gone.
        assert_eq!(g.state(0), (0, false));
        // A pause holds a new one's quads.
        g.request(&cfg, &chars[0], 0, &mut libc);
        g.update(&cfg, &chars, true, &mut libc);
        assert_eq!(g.state(0), (0, true));
        g.request(&cfg, &chars[0], 2, &mut libc);
        g.update(&cfg, &chars, false, &mut libc);
        assert_eq!(g.state(0), (0, false));
    }

    #[test]
    fn quads_draw_as_strips_with_their_colour() {
        let cfg = GlowCfg::parse(&block()).unwrap();
        let node_pos = |n: u32| [0.0, -(n as f32), 100.0];
        let chars = [
            chr(&node_pos),
            GlowChr {
                side: 1,
                ..chr(&node_pos)
            },
        ];
        let mut g = Glows::default();
        let mut libc = LibcRandom::new(1);
        g.request(&cfg, &chars[0], 0, &mut libc);
        for _ in 0..5 {
            g.update(&cfg, &chars, false, &mut libc);
        }
        let view = crate::battle::View::new([0.0; 3], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let tex = GlowTextures {
            plain: vec![None, None, Some(4)],
            ..GlowTextures::default()
        };
        let mut ot = OrderTable::new();
        g.draw(&cfg, &tex, &mut ot, &ctx, |_| Some(1.0), [false; 2]);
        let tris = ot.into_list();
        assert!(!tris.is_empty());
        assert!(
            tris.iter()
                .all(|t| t.texture == Some(4) || t.texture.is_none())
        );
        // A hidden fighter draws nothing.
        let mut ot = OrderTable::new();
        g.draw(&cfg, &tex, &mut ot, &ctx, |_| None, [false; 2]);
        assert!(ot.into_list().is_empty());
    }
}
