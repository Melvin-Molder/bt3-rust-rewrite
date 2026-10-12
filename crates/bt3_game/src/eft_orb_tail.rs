//! The orb tail (`EftOrbTail_*`, src/battle/eft_orb_tail.c, the manager's init in eft_char_parts.c): the ball in
//! the fighter's aura colours a connected shot technique holds between the hands from the owner's event 0x100 to
//! its event 0x200 (`EftShotTech_UpdateAuraBall`). A chain of nodes is laid out behind the ball against its
//! travel and relaxes towards a segment length every frame (a rope); while the fighter moves, batches of streak
//! sprites are born at the head of the chain and travel along it node frame by node frame, fading towards its end
//! and when the fighter stops. A burst starts three particle emitters at the ball.
//!
//! The settings are common effect entries 0x25B (parameters) and 0x25A (colours); the streaks' texture set is
//! 0x259 (its first image under the colour group's palette); the bursts' emitters are 0x25D / 0x25F / 0x260 with
//! the palettes 0x261.. of the colour group and the texture sets 0x25C / 0x25E / 0x25E. Two orbs at most share
//! pools of 128 nodes and 200 streaks. One libc `rand()` per orb (its node count), eleven VU0 draws per streak.

use bt3_data::eft_pack::{PtclDef, PtclDef2};

use crate::battle::{Mtx, apply};
use crate::eft::{DrawCtx, OrderTable, SetAssets, V3, V4, add, norm, point, scale3, sub, xyz};
use crate::rng::{LibcRandom, Vu0Random};

/// `EFT_ORB_NODES`, `EFT_ORB_STREAKS`, the task list's size.
const NODES: usize = 128;
const STREAKS: usize = 200;
const ORBS: usize = 2;

/// The common effect entries.
pub(crate) const TEXTURE_ENTRY: usize = 0x259;
const COLOR_ENTRY: usize = 0x25A;
const PARAM_ENTRY: usize = 0x25B;
/// The bursts' texture sets (`idsA`), emitter definitions (`idsB`) and palettes (`idsC`).
pub(crate) const BURST_TEXTURES: [usize; 3] = [0x25C, 0x25E, 0x25E];
const BURST_DEFS: [usize; 3] = [0x25D, 0x25F, 0x260];
const BURST_PALETTES: usize = 0x261;
/// The colour groups.
pub(crate) const GROUPS: usize = 7;

// EFT_ORB_*.
const ACTIVE: u32 = 0x001;
const END: u32 = 0x002;
const KILL: u32 = 0x004;
const DONE: u32 = 0x008;
const CHAIN: u32 = 0x020;
const MOVING: u32 = 0x040;
const BURST: u32 = 0x080;
const ANIM: u32 = 0x100;
const PLACED: u32 = 0x200;

// EFT_ORB_STREAK_*.
const S_USED: u32 = 0x01;
const S_FADE: u32 = 0x02;
const S_DEAD: u32 = 0x08;
const S_SHOW: u32 = 0x10;

/// `EftOrbTailParam` (common entry 0x25B). Ranges are (base, spread); A / B / C are a streak's values at birth,
/// at the turn and at death.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct OrbParam {
    pub speed_base: [f32; 3],
    pub speed_range: [f32; 3],
    pub size_base: [f32; 3],
    pub size_range: [f32; 3],
    pub turn: f32,
    pub life: [f32; 2],
    pub head_offset: f32,
    pub fade_in: [f32; 2],
    pub fade_out: [f32; 2],
    pub tail_fade: f32,
    pub node_len: f32,
    pub stiffness: f32,
    pub burst_size: [f32; 3],
    pub burst_rate: [f32; 3],
    pub nodes: [u8; 2],
    pub streak_period: u8,
    pub streak_count: u8,
    pub tex_cols: u8,
    pub tex_rows: u8,
    pub ot_z: u8,
}

impl OrbParam {
    fn parse(b: &[u8]) -> Option<Self> {
        let f = |o: usize| {
            b.get(o..o + 4)
                .map(|w| f32::from_le_bytes(w.try_into().unwrap()))
        };
        let f3 = |o: usize| Some([f(o)?, f(o + 4)?, f(o + 8)?]);
        Some(Self {
            speed_base: f3(0)?,
            speed_range: f3(0xC)?,
            size_base: f3(0x18)?,
            size_range: f3(0x24)?,
            turn: f(0x30)?,
            life: [f(0x34)?, f(0x38)?],
            head_offset: f(0x3C)?,
            fade_in: [f(0x40)?, f(0x44)?],
            fade_out: [f(0x48)?, f(0x4C)?],
            tail_fade: f(0x50)?,
            node_len: f(0x54)?,
            stiffness: f(0x58)?,
            burst_size: f3(0x5C)?,
            burst_rate: f3(0x68)?,
            nodes: [*b.get(0x74)?, *b.get(0x75)?],
            streak_period: *b.get(0x76)?,
            streak_count: *b.get(0x77)?,
            tex_cols: *b.get(0x78)?,
            tex_rows: *b.get(0x79)?,
            ot_z: *b.get(0x7B)?,
        })
    }
}

/// `EftOrbTailColor` (common entry 0x25A).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct OrbColor {
    pub color: [V4; GROUPS],
    pub alpha_a: f32,
    pub alpha_c: f32,
    pub alpha_turn: f32,
}

impl OrbColor {
    fn parse(b: &[u8]) -> Option<Self> {
        let f = |o: usize| {
            b.get(o..o + 4)
                .map(|w| f32::from_le_bytes(w.try_into().unwrap()))
        };
        let mut color = [[0.0; 4]; GROUPS];
        for (k, c) in color.iter_mut().enumerate() {
            for (j, v) in c.iter_mut().enumerate() {
                *v = f(k * 16 + j * 4)?;
            }
        }
        Some(Self {
            color,
            alpha_a: f(0x70)?,
            alpha_c: f(0x74)?,
            alpha_turn: f(0x78)?,
        })
    }
}

/// The settings of `EftOrbTailMgr_Init`, and the bursts' emitter definitions as an emitter set (`ORB_LIST`):
/// pair `burst * 7 + group` is burst `burst`'s definition with the group's palette block.
#[derive(Clone, Debug, Default)]
pub(crate) struct OrbCfg {
    pub param: OrbParam,
    pub color: OrbColor,
    pub burst_set: SetAssets,
}

impl OrbCfg {
    pub(crate) fn read(common: &[u8]) -> Option<Self> {
        let entry = |n: usize| bt3_data::eft_pack::common_entry(common, n);
        let mut ptcl = Vec::new();
        for def in BURST_DEFS {
            for g in 0..GROUPS {
                let d = entry(def).and_then(|b| PtclDef::parse(b).ok());
                let d2 = entry(BURST_PALETTES + g).and_then(|b| PtclDef2::parse(b).ok());
                ptcl.push(d.zip(d2));
            }
        }
        Some(Self {
            param: OrbParam::parse(entry(PARAM_ENTRY)?)?,
            color: OrbColor::parse(entry(COLOR_ENTRY)?)?,
            burst_set: SetAssets {
                ptcl,
                ..SetAssets::default()
            },
        })
    }

    fn burst_def(&self, burst: usize, group: usize) -> Option<&(PtclDef, PtclDef2)> {
        self.burst_set.ptcl.get(burst * GROUPS + group)?.as_ref()
    }
}

/// `EftOrbTailNode`.
#[derive(Clone, Copy, Debug, Default)]
struct Node {
    mtx: Mtx,
    pos: V3,
    len: f32,
    pitch: f32,
    yaw: f32,
    used: bool,
}

/// `EftOrbTailStreak`; `node` is its segment's place in the chain.
#[derive(Clone, Copy, Debug, Default)]
struct Streak {
    color: V4,
    pos: V3,
    local: V3,
    speed: f32,
    speed_step: [f32; 2],
    roll: f32,
    size: f32,
    size_step: [f32; 2],
    alpha: f32,
    alpha_step: [f32; 2],
    age: f32,
    life: f32,
    dist: f32,
    fade_in: f32,
    fade_out: f32,
    fade_in_len: f32,
    fade_out_len: f32,
    frame: f32,
    flags: u32,
    node: usize,
}

/// `EftOrbTail`.
#[derive(Clone, Debug)]
struct Orb {
    handle: u32,
    side: usize,
    /// The colour group (0..6).
    group: usize,
    pos: V3,
    prev_pos: V3,
    dir: V3,
    grow_len: f32,
    grown: f32,
    grow_step: f32,
    grow: f32,
    scale: f32,
    chain_len: f32,
    frame: f32,
    tex_frames: f32,
    uv: [[f32; 4]; 16],
    node_count: u8,
    node_max: u8,
    flags: u32,
    /// The chain, head first, and the streaks in their list's order (pool indices).
    nodes: Vec<usize>,
    streaks: Vec<usize>,
    /// The burst emitters (`EftPtcl` handles).
    burst: [Option<u32>; 3],
}

/// What an orb reads of its fighter.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OrbChr {
    /// `BtlCharApi_IsInRushSequence`.
    pub in_rush: bool,
    /// `BtlCharApi_GetSpeed` and `BtlCharApi_GetFallSpeed`.
    pub speed: f32,
    pub fall_speed: f32,
}

/// A burst emitter to start (`EftPtcl_Create` of `EftOrbTail_StartBurst`): the handle the orb keeps, the owner,
/// the definition pair of `OrbCfg::burst_set`, the texture set (0..2 of `BURST_TEXTURES`'s order), position,
/// direction, life (seconds) and size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BurstReq {
    pub handle: u32,
    pub side: usize,
    pub pair: usize,
    pub burst: usize,
    pub pos: V3,
    pub dir: V3,
    pub life: f32,
    pub size: f32,
}

/// What the orbs ask of the particle emitters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EmitReq {
    Start(BurstReq),
    Stop(u32),
}

/// `gEftOrbTail`: the pools and the live orbs.
#[derive(Clone, Debug)]
pub(crate) struct OrbTails {
    nodes: Vec<Node>,
    node_next: usize,
    streaks: Vec<Streak>,
    streak_next: usize,
    orbs: Vec<Orb>,
    next_emitter: u32,
    /// The burst emitters' requests of this update, in order.
    pub(crate) emit: Vec<EmitReq>,
}

impl Default for OrbTails {
    fn default() -> Self {
        Self {
            nodes: vec![Node::default(); NODES],
            node_next: 0,
            streaks: vec![Streak::default(); STREAKS],
            streak_next: 0,
            orbs: Vec::new(),
            next_emitter: 0,
            emit: Vec::new(),
        }
    }
}

fn clamp01(x: f32) -> f32 {
    if x < 0.0 {
        0.0
    } else if 1.0 < x {
        1.0
    } else {
        x
    }
}

fn length(v: V3) -> f32 {
    crate::btl_chr::length(v)
}

impl OrbTails {
    /// The live orbs.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.orbs.len()
    }

    /// The streaks of the live orbs that show.
    #[cfg(test)]
    pub(crate) fn shown(&self) -> usize {
        self.orbs
            .iter()
            .flat_map(|o| &o.streaks)
            .filter(|&&i| self.streaks[i].flags & S_SHOW != 0)
            .count()
    }

    /// `EftOrbTail_Create` + `EftOrbTail_Init` under `handle`: the aura type picks the colour group (7 is 1,
    /// 8..10 are 2), the fighter's height the scale and the node count. False when two orbs live.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        &mut self,
        cfg: &OrbCfg,
        handle: u32,
        side: usize,
        aura_type: i32,
        height: f32,
        dir: V3,
        libc: &mut LibcRandom,
    ) -> bool {
        if self.orbs.len() >= ORBS {
            return false;
        }
        let p = &cfg.param;
        let group = match aura_type as u8 {
            7 => 1,
            8..=10 => 2,
            k => usize::from(k),
        }
        .min(GROUPS - 1);
        let scale = height / 19.35;
        let base = i32::from(p.nodes[0]);
        let node_max = libc.int_range(base, base + i32::from(p.nodes[1])) as u8;
        let node_max = (f32::from(node_max) * scale) as u8;
        let mut orb = Orb {
            handle,
            side,
            group,
            pos: [0.0; 3],
            prev_pos: [0.0; 3],
            dir,
            grow_len: p.node_len * f32::from(node_max),
            grown: 0.0,
            grow_step: (0..3)
                .map(|k| p.speed_base[k] + p.speed_range[k])
                .sum::<f32>()
                * 0.5,
            grow: 0.0,
            scale,
            chain_len: 0.0,
            frame: 0.0,
            tex_frames: 0.0,
            uv: [[0.0; 4]; 16],
            node_count: 0,
            node_max,
            flags: ACTIVE | 0x10,
            nodes: Vec::new(),
            streaks: Vec::new(),
            burst: [None; 3],
        };
        // EftOrbTail_InitFrames: a cols x rows sheet of at most 16 frames.
        orb.tex_frames = f32::from(p.tex_cols) * f32::from(p.tex_rows);
        if orb.tex_frames > 1.0 && orb.tex_frames <= 16.0 {
            let step = [1.0 / f32::from(p.tex_cols), 1.0 / f32::from(p.tex_rows)];
            let mut n = 0;
            for row in 0..p.tex_rows {
                for col in 0..p.tex_cols {
                    let (c, r) = (f32::from(col), f32::from(row));
                    orb.uv[n] = [
                        step[0] * c,
                        step[1] * r,
                        step[0] * c + step[0],
                        step[1] * r + step[1],
                    ];
                    n += 1;
                }
            }
            orb.flags |= ANIM;
        }
        self.orbs.push(orb);
        true
    }

    /// The fighter of a live orb.
    pub(crate) fn side_of(&self, handle: u32) -> Option<usize> {
        self.orbs
            .iter()
            .find(|o| o.handle == handle && o.flags & ACTIVE != 0)
            .map(|o| o.side)
    }

    fn orb_mut(&mut self, handle: u32) -> Option<&mut Orb> {
        self.orbs
            .iter_mut()
            .find(|o| o.handle == handle && o.flags & ACTIVE != 0)
    }

    /// `EftOrbTail_Burst`: the three burst emitters start on the next update.
    pub(crate) fn burst(&mut self, handle: u32) {
        if let Some(o) = self.orb_mut(handle) {
            o.flags |= BURST;
        }
    }

    /// `EftOrbTail_Kill`: the orb dies on its next update.
    pub(crate) fn kill(&mut self, handle: u32) {
        if let Some(o) = self.orb_mut(handle) {
            o.flags |= KILL;
        }
    }

    /// `EftOrbTail_SetPos`: moves the ball; the first call invents a previous position one unit along node 3
    /// from node 0x2E.
    pub(crate) fn set_pos(&mut self, handle: u32, pos: V3, node2e: V3, node3: V3) {
        if let Some(o) = self.orb_mut(handle) {
            if o.flags & PLACED != 0 {
                o.prev_pos = o.pos;
            } else {
                o.prev_pos = add(pos, norm(sub(node3, node2e)));
                o.flags |= PLACED;
            }
            o.pos = pos;
        }
    }

    /// `EftOrbTail_Reset` of every orb (a battle restart).
    pub(crate) fn reset(&mut self) {
        for k in (0..self.orbs.len()).rev() {
            let o = self.orbs.remove(k);
            self.term(&o);
        }
    }

    /// `EftOrbTail_Term`: the nodes and streaks back to the pools, the burst emitters stopped.
    fn term(&mut self, o: &Orb) {
        for &n in &o.nodes {
            self.nodes[n].used = false;
        }
        for &s in &o.streaks {
            self.streaks[s].flags = 0;
        }
        self.stop_burst(&o.burst);
    }

    /// `EftOrbTail_StopBurst` (the emitter module forgets the ones that are gone).
    fn stop_burst(&mut self, burst: &[Option<u32>; 3]) {
        for h in burst.iter().flatten() {
            self.emit.push(EmitReq::Stop(*h));
        }
    }

    /// `EftOrbTailMgr_Update` and `EftOrbTail_Update` of every orb (`paused`: `BtlScene_IsEffectStopped(.., 2)`).
    pub(crate) fn update(
        &mut self,
        cfg: &OrbCfg,
        chars: &[OrbChr; 2],
        paused: bool,
        vu0: &mut Vu0Random,
    ) {
        let mut k = 0;
        while k < self.orbs.len() {
            let mut o = self.orbs[k].clone();
            if !paused {
                let c = chars[o.side];
                self.step(cfg, &mut o, &c, vu0);
            }
            if o.flags & END != 0 && o.streaks.is_empty() {
                o.flags |= DONE;
            }
            if o.flags & (KILL | DONE) != 0 {
                self.orbs.remove(k);
                self.term(&o);
                continue;
            }
            self.orbs[k] = o;
            k += 1;
        }
    }

    /// The update's step of one orb.
    fn step(&mut self, cfg: &OrbCfg, o: &mut Orb, c: &OrbChr, vu0: &mut Vu0Random) {
        let p = &cfg.param;
        o.dir = sub(o.pos, o.prev_pos);
        if o.grown <= o.grow_len {
            o.grow = clamp01(o.grown / o.grow_len);
            o.grown += o.grow_step;
        }
        let speed = if c.in_rush {
            length(o.dir)
        } else {
            c.speed + c.fall_speed.abs()
        };
        if speed > 0.0 {
            o.flags |= MOVING;
        } else {
            o.flags &= !MOVING;
        }
        o.dir = norm(o.dir);
        if o.flags & (END | KILL) == 0 {
            if o.flags & MOVING != 0 {
                if o.node_count < o.node_max {
                    for _ in 0..o.node_max {
                        self.add_node(p, o);
                    }
                }
                if o.flags & CHAIN != 0 && (o.frame as i32) % i32::from(p.streak_period.max(1)) == 0
                {
                    for _ in 0..p.streak_count {
                        self.add_streak(cfg, o, vu0);
                    }
                }
            }
            if o.flags & BURST != 0 {
                self.start_burst(cfg, o);
                o.flags &= !BURST;
            }
        } else {
            let burst = o.burst;
            self.stop_burst(&burst);
        }
        self.update_nodes(p, o);
        self.update_streaks(cfg, o);
        o.frame += 1.0;
    }

    /// `EftOrbTail_AllocNode`: a free node, searching round-robin from the last one handed out.
    fn alloc_node(&mut self) -> Option<usize> {
        if self.node_next >= NODES {
            self.node_next = 0;
        }
        let start = self.node_next;
        let mut i = start;
        loop {
            let n = i;
            i = (i + 1) % NODES;
            if !self.nodes[n].used {
                self.node_next = i;
                return Some(n);
            }
            if i == start {
                return None;
            }
        }
    }

    /// `EftOrbTail_AddNode`: one segment behind the last node, or the head offset in front of the ball.
    fn add_node(&mut self, p: &OrbParam, o: &mut Orb) {
        let pos = match o.nodes.last() {
            Some(&t) => add(self.nodes[t].pos, scale3(o.dir, -(p.node_len * o.scale))),
            None => add(o.pos, scale3(o.dir, p.head_offset * o.scale)),
        };
        if let Some(n) = self.alloc_node() {
            self.nodes[n] = Node {
                pos,
                used: true,
                ..Node::default()
            };
            o.nodes.push(n);
            o.node_count = o.node_count.wrapping_add(1);
        }
        if o.nodes.len() >= 2 {
            o.flags |= CHAIN;
        }
    }

    /// `EftOrbTail_UpdateNodes`: the head follows the ball, each node is pulled back towards one segment from the
    /// one before, then the lengths and orientations are rebuilt.
    fn update_nodes(&mut self, p: &OrbParam, o: &mut Orb) {
        let seg = p.node_len * o.scale;
        if let Some(&h) = o.nodes.first() {
            self.nodes[h].pos = add(o.pos, scale3(o.dir, p.head_offset * o.scale));
        }
        for w in o.nodes.windows(2) {
            let (a, b) = (self.nodes[w[0]].pos, self.nodes[w[1]].pos);
            let d = sub(b, a);
            let len = length(d);
            let t = if len > 0.0 {
                let d = norm(d);
                if !(seg < len) {
                    add(a, scale3(d, len))
                } else {
                    add(a, scale3(d, seg))
                }
            } else {
                a
            };
            let pull = scale3(sub(t, b), -(p.stiffness * o.scale));
            self.nodes[w[1]].pos = add(t, pull);
        }
        o.chain_len = 0.0;
        let mut d = [0.0; 3];
        for i in 0..o.nodes.len() {
            let n = o.nodes[i];
            if let Some(&next) = o.nodes.get(i + 1) {
                d = sub(self.nodes[next].pos, self.nodes[n].pos);
            } else if i > 0 {
                d = sub(self.nodes[n].pos, self.nodes[o.nodes[i - 1]].pos);
            }
            let len = length(d);
            self.nodes[n].len = len;
            o.chain_len += len;
            if len > 0.0 {
                let u = norm(d);
                self.nodes[n].pitch =
                    crate::btl_tech_volley::eft_wrap((-u[1]).clamp(-1.0, 1.0).asin());
                self.nodes[n].yaw = crate::btl_tech_volley::eft_wrap(u[0].atan2(u[2]));
            } else if i > 0 {
                let prev = self.nodes[o.nodes[i - 1]];
                self.nodes[n].pitch = prev.pitch;
                self.nodes[n].yaw = prev.yaw;
            }
            let node = &mut self.nodes[n];
            node.mtx = crate::eft_ptcl::rot_xy(node.pitch, node.yaw);
            node.mtx[3] = point(node.pos);
        }
    }

    /// `EftOrbTail_AllocStreak`.
    fn alloc_streak(&mut self) -> Option<usize> {
        if self.streak_next >= STREAKS {
            self.streak_next = 0;
        }
        let start = self.streak_next;
        let mut i = start;
        loop {
            let s = i;
            i = (i + 1) % STREAKS;
            if self.streaks[s].flags == 0 {
                self.streak_next = i;
                return Some(s);
            }
            if i == start {
                return None;
            }
        }
    }

    /// `EftOrbTail_AddStreak` + `EftOrbTail_InitStreak`: life, three speed and size keys, the alpha ramp, roll,
    /// fade times and the first frame (eleven VU0 draws), at the head of the chain.
    fn add_streak(&mut self, cfg: &OrbCfg, o: &mut Orb, vu0: &mut Vu0Random) {
        if o.nodes.is_empty() {
            return;
        }
        let Some(i) = self.alloc_streak() else {
            return;
        };
        let p = &cfg.param;
        let col = &cfg.color;
        let mut r = |a: f32, b: f32| vu0.float_range(a, a + b);
        let life = r(p.life[0], p.life[1]) * 30.0 * o.scale;
        let mut keys = |base: [f32; 3], range: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|k| r(base[k], range[k]) * o.scale)
        };
        let speed = keys(p.speed_base, p.speed_range);
        let size = keys(p.size_base, p.size_range);
        let ramp = |k: [f32; 3], turn: f32| {
            [
                (k[1] - k[0]) / (life * turn),
                (k[2] - k[1]) / (life * (1.0 - turn)),
            ]
        };
        let alpha = [col.alpha_a, (col.alpha_c + col.alpha_a) * 0.5, col.alpha_c];
        let roll = vu0.float_range(-3.141_592_7, 3.141_592_7);
        let fade_in_len =
            vu0.float_range(p.fade_in[0], p.fade_in[0] + p.fade_in[1]) * 30.0 * o.scale;
        let fade_out_len =
            vu0.float_range(p.fade_out[0], p.fade_out[0] + p.fade_out[1]) * 30.0 * o.scale;
        let frame = vu0.float_range(0.0, o.tex_frames - 1.0);
        self.streaks[i] = Streak {
            color: col.color[o.group],
            pos: [0.0; 3],
            local: [0.0; 3],
            speed: speed[0],
            speed_step: ramp(speed, p.turn),
            roll,
            size: size[0],
            size_step: ramp(size, p.turn),
            alpha: alpha[0],
            alpha_step: ramp(alpha, col.alpha_turn),
            age: 0.0,
            life,
            dist: 0.0,
            fade_in: 0.0,
            fade_out: fade_out_len,
            fade_in_len,
            fade_out_len,
            frame,
            flags: S_USED,
            node: 0,
        };
        o.streaks.push(i);
    }

    /// `EftOrbTail_UpdateStreaks`: the ramps, the travel along the chain, the fades and the removals; then from
    /// the tail, every streak past a gap of more than 1.5 sizes to the one before it starts fading.
    fn update_streaks(&mut self, cfg: &OrbCfg, o: &mut Orb) {
        let p = &cfg.param;
        let col = &cfg.color;
        let nodes = &self.nodes;
        let next = |i: usize| o.nodes.get(i + 1).copied();
        let mut k = 0;
        while k < o.streaks.len() {
            let s = &mut self.streaks[o.streaks[k]];
            let t = s.age / s.life;
            s.age += 1.0;
            let (a, b) = if t < p.turn { (0, 0) } else { (1, 1) };
            s.size += s.size_step[a];
            s.speed += s.speed_step[b];
            s.alpha += s.alpha_step[usize::from(t >= col.alpha_turn)];
            s.local[2] += s.speed;
            let node = nodes[o.nodes[s.node]];
            if node.len <= s.local[2] {
                let mut rest = s.local[2] - node.len;
                if let Some(n1) = next(s.node) {
                    if nodes[n1].len <= rest {
                        loop {
                            s.node += 1;
                            let cur = nodes[o.nodes[s.node]];
                            s.dist += cur.len;
                            // The segment before the last.
                            if s.node + 2 == o.nodes.len() {
                                s.flags |= S_FADE;
                            }
                            if cur.len <= rest {
                                rest -= cur.len;
                            } else {
                                break;
                            }
                            if next(s.node).is_none() {
                                break;
                            }
                        }
                    } else {
                        s.node += 1;
                        s.dist += nodes[o.nodes[s.node]].len;
                    }
                    s.dist += rest;
                    s.local = [0.0, 0.0, rest];
                }
            }
            let node = nodes[o.nodes[s.node]];
            s.pos = xyz(apply(&node.mtx, point(s.local)));
            let tail_fade = if o.chain_len * p.tail_fade > 0.0 {
                1.0 - clamp01(s.dist / (o.chain_len * (p.tail_fade * o.grow)))
            } else {
                0.0
            };
            if o.flags & ANIM != 0 {
                s.frame += 1.0;
                if o.tex_frames <= s.frame {
                    s.frame = 0.0;
                }
            }
            if !(o.flags & MOVING != 0 && o.flags & END == 0) {
                s.flags |= S_FADE;
            }
            // On the last two segments.
            if s.node + 2 >= o.nodes.len() {
                s.flags |= S_FADE;
            }
            let fade_out = if s.flags & S_FADE != 0 {
                if s.fade_out_len > 0.0 {
                    clamp01(s.fade_out / s.fade_out_len)
                } else {
                    0.0
                }
            } else {
                1.0
            };
            let fade_in = if s.fade_in_len > 0.0 {
                clamp01(s.fade_in / s.fade_in_len)
            } else {
                1.0
            };
            if s.flags & S_FADE != 0 {
                if s.fade_out > 0.0 {
                    s.fade_out -= 1.0;
                }
            } else if s.fade_in < s.fade_in_len {
                s.fade_in += 1.0;
            }
            s.color[3] =
                col.color[o.group][3] * s.alpha * (1.0 - t) * fade_in * fade_out * tail_fade;
            if s.color[3] <= 0.0 || s.size <= 0.0 {
                s.flags &= !S_SHOW;
                if fade_in > 0.0 {
                    s.flags |= S_FADE;
                }
            } else {
                s.flags |= S_SHOW;
            }
            if s.flags & S_FADE != 0 && s.flags & S_SHOW == 0 {
                s.flags |= S_DEAD;
            }
            if s.age >= s.life || s.flags & S_DEAD != 0 {
                s.flags = 0;
                o.streaks.remove(k);
                continue;
            }
            k += 1;
        }
        let mut gap = false;
        for k in (0..o.streaks.len()).rev() {
            let i = o.streaks[k];
            if self.streaks[i].flags & S_FADE == 0 {
                if k > 0 {
                    let prev = self.streaks[o.streaks[k - 1]].pos;
                    if length(sub(self.streaks[i].pos, prev)) > self.streaks[i].size * 1.5 {
                        gap = true;
                    }
                }
                if gap {
                    self.streaks[i].flags |= S_FADE;
                }
            }
        }
    }

    /// `EftOrbTail_StartBurst`: the three emitters at the head offset in front of the ball, along its direction.
    fn start_burst(&mut self, cfg: &OrbCfg, o: &mut Orb) {
        let p = &cfg.param;
        let pos = add(o.pos, scale3(o.dir, p.head_offset * o.scale));
        for i in 0..3 {
            if cfg.burst_def(i, o.group).is_none() {
                o.burst[i] = None;
                continue;
            }
            self.next_emitter = self.next_emitter.wrapping_add(1).max(1);
            let handle = self.next_emitter;
            self.emit.push(EmitReq::Start(BurstReq {
                handle,
                side: o.side,
                pair: i * GROUPS + o.group,
                burst: i,
                pos,
                dir: o.dir,
                life: p.burst_rate[i] * o.scale,
                size: p.burst_size[i] * o.scale,
            }));
            o.burst[i] = Some(handle);
        }
    }

    /// `EftOrbTail_Draw`: every streak that shows as a camera-facing quad of its size, rolled, in its sheet frame,
    /// at twice its depth slot. `textures[g]`: the streak image under colour group `g`'s palette.
    pub(crate) fn draw(
        &self,
        cfg: &OrbCfg,
        textures: &[Option<usize>],
        ot: &mut OrderTable,
        ctx: &DrawCtx,
    ) {
        for o in &self.orbs {
            let texture = textures.get(o.group).copied().flatten();
            for &i in &o.streaks {
                let s = &self.streaks[i];
                if s.flags & S_SHOW == 0 {
                    continue;
                }
                let uv = if o.flags & ANIM != 0 {
                    let f = if s.frame < 0.0 {
                        0
                    } else if o.tex_frames < s.frame {
                        o.tex_frames as usize
                    } else {
                        s.frame as usize
                    };
                    o.uv[f.min(15)]
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                };
                crate::eft::quad_depth_scaled(
                    ot,
                    ctx,
                    s.pos,
                    s.color,
                    cfg.param.ot_z,
                    false,
                    texture,
                    [s.size, s.size],
                    uv,
                    s.roll,
                    2.0,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OrbCfg {
        OrbCfg {
            param: OrbParam {
                speed_base: [1.0; 3],
                size_base: [2.0; 3],
                turn: 0.5,
                life: [1.0, 0.0],
                head_offset: 1.0,
                fade_in: [0.1, 0.0],
                fade_out: [0.1, 0.0],
                tail_fade: 1.0,
                node_len: 1.0,
                stiffness: 0.5,
                burst_rate: [1.0; 3],
                burst_size: [1.0; 3],
                nodes: [8, 0],
                streak_period: 2,
                streak_count: 2,
                tex_cols: 2,
                tex_rows: 2,
                ..OrbParam::default()
            },
            color: OrbColor {
                color: [[128.0; 4]; GROUPS],
                alpha_a: 1.0,
                alpha_c: 0.0,
                alpha_turn: 0.5,
            },
            burst_set: SetAssets::default(),
        }
    }

    #[test]
    fn an_orb_grows_its_chain_feeds_streaks_and_dies_when_killed() {
        let cfg = cfg();
        let mut orbs = OrbTails::default();
        let mut libc = LibcRandom::new(1);
        let mut vu0 = Vu0Random::seeded(0.25);
        assert!(orbs.create(&cfg, 5, 0, 8, 19.35, [0.0, 0.0, 1.0], &mut libc));
        assert!(orbs.create(&cfg, 6, 1, 0, 19.35, [0.0, 0.0, 1.0], &mut libc));
        assert!(!orbs.create(&cfg, 7, 1, 0, 19.35, [0.0, 0.0, 1.0], &mut libc));
        assert_eq!(orbs.orbs[0].group, 2);
        assert_eq!(orbs.orbs[0].node_max, 8);
        assert!(orbs.orbs[0].flags & ANIM != 0);
        orbs.kill(6);
        let moving = [OrbChr {
            speed: 1.0,
            ..OrbChr::default()
        }; 2];
        let mut shown = 0;
        for f in 0..40 {
            let pos = [0.0, 0.0, f as f32];
            orbs.set_pos(5, pos, [0.0, 0.0, -1.0], [0.0, 0.0, 0.0]);
            orbs.update(&cfg, &moving, false, &mut vu0);
            shown = shown.max(orbs.shown());
        }
        assert_eq!(orbs.count(), 1);
        assert_eq!(orbs.orbs[0].nodes.len(), 8);
        assert!(shown > 0);
        // The burst starts three emitters (none here: no definitions), the kill frees the pools.
        orbs.burst(5);
        orbs.kill(5);
        orbs.update(&cfg, &moving, false, &mut vu0);
        assert_eq!(orbs.count(), 0);
        assert!(orbs.nodes.iter().all(|n| !n.used));
        assert!(orbs.streaks.iter().all(|s| s.flags == 0));
    }
}
