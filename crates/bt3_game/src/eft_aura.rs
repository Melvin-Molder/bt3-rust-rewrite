//! The aura (`EftAura*`, src/battle/eft_aura.c): while a fighter's ki is at least half full (or it is powered up)
//! flames rise from ten body parts and sparks drift from twelve emitters, from pools of 50 flames and 18 sparks
//! per fighter that both fighters share. A flame bends from its start direction towards its part, grows in phases
//! and fades; near its end it spawns its successor. Bursts (requests 7 / 8) speed and lengthen the flames.
//!
//! The settings are common effect entry 1 (`EftAuraPrm` at its start, `EftAuraCfg` / `EftAuraData` after it), the
//! flame sheets entry 3 (image 0 under the aura type's palette; types 8..10 image 8 under two palettes). The sparks
//! are stepped but never drawn (`EftAura_DrawSparks` has no caller).
//!
//! The pools are singly linked lists with the newest at the head, walked as the original walks them: a flame or
//! spark spawned while the list is stepped goes in front of the walk, and a spark that replaces the list's head
//! is lost with it (`EftAura_StepSparks` relinks the head past both).

use crate::battle::{IDENTITY, Mtx, apply};
use crate::eft::{
    DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, V3, V4, cross,
    depth_slot, dot, norm,
};
use crate::rng::LibcRandom;

/// `EFT_AURA_PARTS` / `EFT_AURA_SPARKS`.
const PARTS: usize = 10;
const SPARKS: usize = 12;
/// `EftAuraMgr_Init`: per fighter.
const FLAMES_PER: usize = 50;
const SPARKS_PER: usize = 18;

const PI_F: f32 = 3.141_592_7;

/// `RANDF`.
fn randf(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    b.get(o..o + 4)
        .map_or(0.0, |s| f32::from_le_bytes(s.try_into().unwrap()))
}

fn i32_at(b: &[u8], o: usize) -> i32 {
    b.get(o..o + 4)
        .map_or(-1, |s| i32::from_le_bytes(s.try_into().unwrap()))
}

fn v4_at(b: &[u8], o: usize) -> V4 {
    std::array::from_fn(|i| f32_at(b, o + i * 4))
}

fn v3(v: V4) -> V3 {
    [v[0], v[1], v[2]]
}

/// `EftAuraPrmKind`.
#[derive(Clone, Copy, Debug, Default)]
struct PrmKind {
    side: [f32; 2],
    away: [f32; 2],
    out: [f32; 2],
    turn: f32,
    accel: f32,
}

/// Common effect entry 1 (`EftAuraData`).
#[derive(Clone, Debug, Default)]
pub(crate) struct AuraCfg {
    // EftAuraPrm.
    end_ratio: f32,
    grow_ratio: f32,
    start_dist: f32,
    spark_rise: f32,
    flame_step: f32,
    lean: f32,
    near: f32,
    life: [f32; 3],
    fade_in: f32,
    fade_out: f32,
    speed: [f32; 6],
    speed_rand: [[f32; 2]; 3],
    stretch: [f32; 4],
    size: [f32; 4],
    stretch_mul: [f32; 3],
    size_mul: [f32; 3],
    stretch_kind: [f32; 4],
    size_kind: [f32; 3],
    stretch_rand: [[f32; 3]; 3],
    end: [[f32; 3]; 3],
    kinds: [PrmKind; 3],
    alpha: [f32; 2],
    alpha_mul: [f32; 4],
    spark_vel: f32,
    spark_vel_alt: f32,
    side_scale: f32,
    drag: f32,
    speed_ref: f32,
    out_rise: f32,
    // EftAuraCfg.
    a_fade_in: f32,
    a_fade_out: f32,
    burst_hold: f32,
    side_power: f32,
    burst_speed: f32,
    size_fade_in: f32,
    size_fade_out: f32,
    alpha_fade_in: f32,
    alpha_fade_out: f32,
    spark_fade_in: f32,
    spark_fade_out: f32,
    spark_life: [f32; 4],
    spark_step: f32,
    spark_rot_speed: f32,
    spark_accel: f32,
    spark_damp: f32,
    spark_size: [[f32; 2]; 4],
    spark_mul: [[f32; 2]; 2],
    // EftAuraData.
    corner: [[V4; 4]; 2],
    corner_b: [[V4; 4]; 2],
    uv: [[V4; 4]; 4],
    /// Per part: its node, second node and lean-away node (negative for none).
    part: [[i32; 3]; PARTS],
    fade_node: [i32; 7],
    /// Per spark emitter: the node it sits on and the node it is pulled towards.
    spark: [[i32; 2]; SPARKS],
    color: Vec<[V4; 13]>,
    color_b: Vec<[V4; 13]>,
}

impl AuraCfg {
    /// Reads the block; `None` when it is too short.
    pub(crate) fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 0xC60 + 5 * 13 * 16 {
            return None;
        }
        let f = |o| f32_at(b, o);
        let a2 = |o| [f(o), f(o + 4)];
        let a3 = |o| [f(o), f(o + 4), f(o + 8)];
        let a4 = |o| [f(o), f(o + 4), f(o + 8), f(o + 12)];
        let quad = |o: usize| -> [V4; 4] { std::array::from_fn(|i| v4_at(b, o + i * 16)) };
        let row = |o: usize| -> [V4; 13] { std::array::from_fn(|i| v4_at(b, o + i * 16)) };
        Some(Self {
            end_ratio: f(0),
            grow_ratio: f(4),
            start_dist: f(8),
            spark_rise: f(0xC),
            flame_step: f(0x10),
            lean: f(0x14),
            near: f(0x18),
            life: a3(0x1C),
            fade_in: f(0x2C),
            fade_out: f(0x30),
            speed: std::array::from_fn(|i| f(0x34 + i * 4)),
            speed_rand: [a2(0x4C), a2(0x54), a2(0x5C)],
            stretch: a4(0x64),
            size: a4(0x74),
            stretch_mul: a3(0x84),
            size_mul: a3(0x90),
            stretch_kind: a4(0x9C),
            size_kind: a3(0xAC),
            stretch_rand: [a3(0xBC), a3(0xC8), a3(0xD4)],
            end: [a3(0xE0), a3(0xEC), a3(0xF8)],
            kinds: std::array::from_fn(|k| {
                let o = 0x104 + k * 0x20;
                PrmKind {
                    side: a2(o),
                    away: a2(o + 8),
                    out: a2(o + 0x10),
                    turn: f(o + 0x18),
                    accel: f(o + 0x1C),
                }
            }),
            alpha: a2(0x164),
            alpha_mul: a4(0x16C),
            spark_vel: f(0x17C),
            spark_vel_alt: f(0x180),
            side_scale: f(0x184),
            drag: f(0x188),
            speed_ref: f(0x18C),
            out_rise: f(0x190),
            a_fade_in: f(0x194),
            a_fade_out: f(0x198),
            burst_hold: f(0x19C),
            side_power: f(0x1A0),
            burst_speed: f(0x1A4),
            size_fade_in: f(0x1A8),
            size_fade_out: f(0x1AC),
            alpha_fade_in: f(0x1B0),
            alpha_fade_out: f(0x1B4),
            spark_fade_in: f(0x1B8),
            spark_fade_out: f(0x1BC),
            spark_life: a4(0x1C0),
            spark_step: f(0x1D0),
            spark_rot_speed: f(0x1D4),
            spark_accel: f(0x1D8),
            spark_damp: f(0x1DC),
            spark_size: std::array::from_fn(|i| a2(0x1E0 + i * 8)),
            spark_mul: [a2(0x200), a2(0x208)],
            corner: [quad(0x210), quad(0x250)],
            corner_b: [quad(0x290), quad(0x2D0)],
            uv: [quad(0x310), quad(0x350), quad(0x390), quad(0x3D0)],
            part: std::array::from_fn(|i| {
                let o = 0x410 + i * 12;
                [i32_at(b, o), i32_at(b, o + 4), i32_at(b, o + 8)]
            }),
            fade_node: std::array::from_fn(|i| i32_at(b, 0x488 + i * 4)),
            spark: std::array::from_fn(|i| [i32_at(b, 0x4A4 + i * 8), i32_at(b, 0x4A8 + i * 8)]),
            color: (0..9).map(|t| row(0x510 + t * 13 * 16)).collect(),
            color_b: (0..5).map(|t| row(0xC60 + t * 13 * 16)).collect(),
        })
    }
}

/// A singly linked pool (`flames` / `flameFree` / `flameUsed` and the sparks'): slots handed out in order up to
/// `max`, then reused from the free list.
#[derive(Clone, Debug)]
struct List<T> {
    items: Vec<T>,
    next: Vec<Option<usize>>,
    free: Option<usize>,
    used: Option<usize>,
    max: usize,
}

/// Where a walk of the used list stands: the head pointer, or the `next` of an item.
#[derive(Clone, Copy, Debug)]
enum Link {
    Head,
    After(usize),
}

impl<T: Default> List<T> {
    fn new(max: usize) -> Self {
        Self {
            items: Vec::new(),
            next: Vec::new(),
            free: None,
            used: None,
            max,
        }
    }

    /// `EftAura_AllocFlame` / `AllocSpark`: linked at the head of the used list (its old contents kept).
    fn alloc(&mut self) -> Option<usize> {
        let i = if let Some(i) = self.free {
            self.free = self.next[i];
            i
        } else if self.items.len() < self.max {
            self.items.push(T::default());
            self.next.push(None);
            self.items.len() - 1
        } else {
            return None;
        };
        self.next[i] = self.used;
        self.used = Some(i);
        Some(i)
    }

    fn get(&self, l: Link) -> Option<usize> {
        match l {
            Link::Head => self.used,
            Link::After(i) => self.next[i],
        }
    }

    fn set(&mut self, l: Link, v: Option<usize>) {
        match l {
            Link::Head => self.used = v,
            Link::After(i) => self.next[i] = v,
        }
    }

    /// `*link = p->next; p->next = free; free = p`.
    fn release(&mut self, l: Link, i: usize) {
        self.set(l, self.next[i]);
        self.next[i] = self.free;
        self.free = Some(i);
    }

    /// The used list from its head.
    fn walk(&self) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.used, |&i| self.next[i])
    }
}

/// `EftAuraFlame`.
#[derive(Clone, Copy, Debug, Default)]
struct Flame {
    flags: u32,
    shape: u8,
    kind: u8,
    tex: u8,
    chr: usize,
    part: u8,
    flip: u8,
    alt: u8,
    end_time: f32,
    time: f32,
    life: f32,
    scale: f32,
    size: f32,
    stretch: f32,
    size_rate: f32,
    stretch_rate: f32,
    end: f32,
    speed: f32,
    follow: f32,
    power: f32,
    offset: V4,
    pos: V4,
    color: V4,
    color_rate: V4,
    dir: V4,
    dir_start: V4,
    dir_turn: V4,
    mv: V4,
}

/// `EftAuraSpark`.
#[derive(Clone, Copy, Debug, Default)]
struct Spark {
    flags: u32,
    chr: usize,
    emitter: u8,
    age: f32,
    life: f32,
    rot: f32,
    rot_speed: f32,
    speed: f32,
    follow: f32,
    pos: V4,
    mv: V4,
    dir: V4,
    color: V4,
    color_rate: V4,
}

/// What the aura reads of its fighter.
pub(crate) struct AuraChr<'a> {
    pub side: usize,
    pub height: f32,
    /// `BtlCharApi_GetAuraType`.
    pub aura_type: i32,
    pub dir: V3,
    pub vel: V3,
    pub speed: f32,
    pub fall_speed: f32,
    /// `BtlCharApi_GetActionFxKind`.
    pub fx_kind: i32,
    pub node_pos: &'a dyn Fn(u32) -> V3,
}

impl AuraChr<'_> {
    fn node(&self, n: i32) -> V4 {
        let v = (self.node_pos)(n as u32);
        [v[0], v[1], v[2], 1.0]
    }
}

/// One fighter's aura (`EftAuraWork`).
#[derive(Clone, Debug, Default)]
struct Aura {
    chr: usize,
    variant: i32,
    flags: u32,
    alt_mask: u32,
    fade_a: i32,
    fade_b: i32,
    aura_type: i32,
    color_count: i32,
    tex_first: i32,
    tex_count: i32,
    parts_started: usize,
    fade: i32,
    burst: i32,
    frame: i32,
    flame_count: i32,
    alpha: f32,
    burst_speed: f32,
    burst_time: f32,
    life_scale: f32,
    level: f32,
    burst_level: f32,
    alpha_fade: f32,
    alpha_fade_rate: f32,
    size_fade: f32,
    size_fade_rate: f32,
    scale: f32,
    pos: V4,
    prev_pos: V4,
    part: [V4; PARTS],
    part_prev: [V4; PARTS],
    part_end: [V4; PARTS],
    part_ref: [V4; PARTS],
    push_centre: V4,
    axis: V4,
    spark_pos: [V4; SPARKS],
    color_start: V4,
    color_end: V4,
    color_alt: [V4; 4],
    spark_color: [V4; 4],
    spark_color_alt: [V4; 4],
    spark_state: u32,
}

/// The aura's textures among the effect textures: entry 3's image 0 under each aura type's palette, image 8 under
/// palettes 8..13 (types 8..10, two each).
#[derive(Clone, Debug, Default)]
pub(crate) struct AuraTextures {
    pub plain: Vec<Option<usize>>,
    pub paired: Vec<Option<usize>>,
}

/// `gEftAuraPool`: the pools and the aura tasks in the order they were created.
#[derive(Clone, Debug)]
pub(crate) struct Auras {
    flames: List<Flame>,
    sparks: List<Spark>,
    tasks: Vec<Aura>,
}

impl Default for Auras {
    fn default() -> Self {
        Self {
            flames: List::new(2 * FLAMES_PER),
            sparks: List::new(2 * SPARKS_PER),
            tasks: Vec::new(),
        }
    }
}

/// `EftAura_GetSparkDir`: against the fighter's velocity, with a random vertical part (one draw).
fn spark_dir(cfg: &AuraCfg, c: &AuraChr, libc: &mut LibcRandom) -> V4 {
    let v = c.vel.map(|x| x * cfg.spark_vel);
    let d = norm([-v[0], cfg.spark_rise * randf(libc), -v[2]]);
    [d[0], d[1], d[2], 1.0]
}

/// `EftAura_GetSparkDirAlt`: straight up / down while rising / falling.
fn spark_dir_alt(cfg: &AuraCfg, c: &AuraChr, libc: &mut LibcRandom) -> V4 {
    let v = c.vel.map(|x| x * cfg.spark_vel_alt);
    let d = match c.fx_kind {
        3 => [0.0, 1.0 - c.fall_speed, 0.0],
        4 => [0.0, -1.0 - c.fall_speed, 0.0],
        _ => [-v[0], cfg.spark_rise * randf(libc), -v[2]],
    };
    let d = norm(d);
    [d[0], d[1], d[2], 1.0]
}

impl Aura {
    /// `EftAura_SetType` / `EftAura_Setup`: the type's colours (types 8..10 the second table, rows 0 / 2 / 3, two
    /// textures each), the scale (height / 19.35, at least 0.7).
    fn setup(&mut self, cfg: &AuraCfg, c: &AuraChr) {
        let ty = c.aura_type;
        self.flags &= !0x80;
        let mut alt = 0;
        if (8..=10).contains(&ty) {
            self.flags |= 0x80;
            self.tex_count = 2;
            (self.tex_first, alt) = match ty {
                9 => (10, 2),
                10 => (12, 3),
                _ => (8, 0),
            };
        }
        self.aura_type = ty;
        self.color_count = if self.flags & 0x80 != 0 { 2 } else { 1 };
        let multi = self.flags & 0x80 != 0;
        let byte = |v: V4| v.map(|x| x * (1.0 / 255.0));
        let row = if multi {
            cfg.color_b.get(alt).copied()
        } else {
            usize::try_from(ty)
                .ok()
                .and_then(|t| cfg.color.get(t))
                .copied()
        }
        .unwrap_or([[0.0; 4]; 13]);
        self.color_start = byte(row[0]);
        self.color_end = byte(row[0]);
        self.color_end[3] = 0.0;
        for i in 0..4 {
            let j = if multi || ty >= 8 { i } else { 0 };
            self.color_alt[i] = byte(row[1 + j]);
            self.spark_color[i] = byte(row[5 + j]);
            self.spark_color_alt[i] = byte(row[9 + j]);
        }
        self.scale = (c.height / 19.35).max(0.7);
    }

    /// `EftAura_Start`.
    fn start(&mut self, reset: bool) {
        self.flags |= 1;
        self.flags |= if self.variant != 0 { 0x20 } else { 0x10 };
        if reset {
            self.size_fade_rate = 0.0;
            self.burst = 0;
            self.burst_level = 0.0;
            self.fade_a = 0;
            self.alpha_fade_rate = 0.0;
            self.fade_b = 0;
            self.life_scale = 1.0;
            self.alpha_fade = 1.0;
            self.size_fade = 1.0;
            self.fade = 1;
        }
    }

    /// `EftAura_GetFlameSideDir`.
    fn side_dir(
        &self,
        cfg: &AuraCfg,
        c: &AuraChr,
        power: f32,
        away: f32,
        side: f32,
        libc: &mut LibcRandom,
    ) -> V4 {
        let mut side = side + cfg.side_power * power * cfg.side_scale;
        let d = norm([c.dir[0], 0.0, c.dir[2]]);
        if libc.rand() & 1 != 0 {
            side = -side;
        }
        let d = d.map(|x| x * side);
        let o = norm([
            self.push_centre[0] - self.pos[0],
            0.0 - self.pos[1],
            self.push_centre[2] - self.pos[2],
        ]);
        let o = o.map(|x| x * away);
        [o[0] + d[0], o[1], o[2] + d[2], 1.0]
    }

    /// `EftAura_GetFlameDragDir`.
    fn drag_dir(&self, cfg: &AuraCfg, pos: V4, part: usize) -> V4 {
        let now = [
            pos[0] - self.pos[0],
            pos[1] - self.pos[1],
            pos[2] - self.pos[2],
        ];
        let pp = self.part_prev[part];
        let before = [
            pp[0] - self.prev_pos[0],
            pp[1] - self.prev_pos[1],
            pp[2] - self.prev_pos[2],
        ];
        let d = norm([now[0] - before[0], now[1] - before[1], now[2] - before[2]]);
        let d = d.map(|x| x * dot(d, d).sqrt() * cfg.drag);
        [-d[0], -d[1], -d[2], 1.0]
    }

    /// `EftAura_GetFlameOutDir` (one draw).
    fn out_dir(
        &self,
        cfg: &AuraCfg,
        c: &AuraChr,
        part: usize,
        power: f32,
        amount: f32,
        libc: &mut LibcRandom,
    ) -> V4 {
        let mut ratio = c.speed * cfg.speed_ref;
        let t = randf(libc);
        let mut amount = amount + cfg.side_power * power;
        if 1.0 < ratio {
            ratio = 1.0;
        }
        amount *= 1.0 - ratio;
        let from = [
            self.axis[0] + (self.pos[0] - self.axis[0]) * t,
            self.pos[1],
            self.axis[2] + (self.pos[2] - self.axis[2]) * t,
        ];
        let p = self.part[part];
        let d = norm([p[0] - from[0], p[1] - from[1], p[2] - from[2]]);
        [
            d[0] * amount,
            d[1] * amount * cfg.out_rise,
            d[2] * amount,
            1.0,
        ]
    }

    /// `EftAura_GetPartOffset`.
    fn part_offset(&self, part: usize, radial: f32, vertical: f32) -> V4 {
        let p = self.part[part];
        let d = norm([p[0] - self.axis[0], p[1] - self.pos[1], p[2] - self.axis[2]]);
        [d[0] * radial, d[1] * vertical, d[2] * radial, 0.0]
    }

    /// `EftAura_GetFlameFlags`.
    fn flame_flags(&self, second: bool, alt: bool) -> u32 {
        let mut f = if second {
            0x800
        } else if alt {
            0x400
        } else {
            0
        };
        if self.flags & 0x80 != 0 {
            f |= 0x100;
        }
        if matches!(self.aura_type, 9 | 10) {
            f |= 0x200;
        }
        match self.burst {
            1 => f |= 0x20,
            2 => f |= 0x40,
            _ => {}
        }
        if self.flags & 0x20 != 0 {
            f |= 0x1000;
        }
        f
    }
}

/// `EftAura_GetFlameLife` (one draw).
fn flame_life(cfg: &AuraCfg, flags: u32, burst: f32, libc: &mut LibcRandom) -> f32 {
    let p = cfg.life;
    let mut life = if flags & 0x200 != 0 {
        p[2] + p[2] * randf(libc)
    } else {
        p[0] + p[1] * randf(libc)
    };
    if flags & 0x20 != 0 {
        life *= 1.0 - burst * 0.15;
    } else if flags & 0x80 != 0 {
        life *= 0.85;
    }
    life
}

/// `EftAura_GetFlameSpeed` (one draw).
fn flame_speed(
    cfg: &AuraCfg,
    flags: u32,
    c: &AuraChr,
    level: f32,
    burst: f32,
    libc: &mut LibcRandom,
) -> f32 {
    let p = cfg.speed;
    let mut speed = (c.speed * p[2] + p[0]) * level;
    if flags & 0x100 != 0 && flags & 0x800 != 0 {
        speed += p[3];
    }
    if flags & 0x80 != 0 {
        speed += p[4];
    } else if flags & 0x20 != 0 {
        speed += p[5] * burst;
    }
    let r = if flags & 0x800 == 0 {
        if flags & 0x400 != 0 {
            cfg.speed_rand[1]
        } else {
            cfg.speed_rand[0]
        }
    } else {
        cfg.speed_rand[2]
    };
    speed * (r[0] + r[1] * randf(libc))
}

/// `EftAura_GetFlameAlpha`.
fn flame_alpha(cfg: &AuraCfg, flags: u32, alpha: f32) -> f32 {
    let mut a = alpha
        * if flags & 0x800 == 0 {
            cfg.alpha[0]
        } else {
            cfg.alpha[1]
        };
    a *= if flags & 0x80 != 0 {
        cfg.alpha_mul[2]
    } else if flags & 0x20 != 0 {
        cfg.alpha_mul[3]
    } else if flags & 0x800 == 0 {
        cfg.alpha_mul[0]
    } else {
        cfg.alpha_mul[1]
    };
    a
}

/// `EftAura_GetFlameStretch` (one draw unless invisible).
fn flame_stretch(
    cfg: &AuraCfg,
    f: &Flame,
    c: &AuraChr,
    phase: usize,
    libc: &mut LibcRandom,
) -> f32 {
    if f.flags & 0x1000 != 0 {
        return 0.0;
    }
    let k = &cfg.stretch_kind;
    let mut v = cfg.stretch[phase]
        * if f.flags & 0x100 != 0 {
            if f.flags & 0x800 == 0 { k[2] } else { k[3] }
        } else if f.flags & 0x800 == 0 {
            k[0]
        } else {
            k[1]
        };
    if f.flags & 0x80 != 0 {
        v *= cfg.stretch_mul[0];
    } else if f.flags & 0x20 != 0 {
        v *= cfg.stretch_mul[1];
    } else if f.flags & 0x40 != 0 {
        v *= cfg.stretch_mul[2];
    }
    let r = cfg.stretch_rand[usize::from(f.kind)];
    let mut k = r[0] + r[1] * randf(libc);
    k += c.speed * r[2];
    if 1.0 < k {
        k = 1.0;
    }
    v * k
}

/// `EftAura_GetFlameSize`.
fn flame_size(cfg: &AuraCfg, f: &Flame, phase: usize) -> f32 {
    if f.flags & 0x1000 != 0 {
        return 0.0;
    }
    let mut v = cfg.size[phase]
        * if f.flags & 0x100 != 0 {
            cfg.size_kind[2]
        } else {
            cfg.size_kind[0]
        };
    if f.flags & 0x80 != 0 {
        v *= cfg.size_mul[0];
    } else if f.flags & 0x20 != 0 {
        v *= cfg.size_mul[1];
    } else if f.flags & 0x40 != 0 {
        v *= cfg.size_mul[2];
    }
    v
}

/// `EftAura_GetFlameEnd` (one draw).
fn flame_end(cfg: &AuraCfg, f: &Flame, c: &AuraChr, libc: &mut LibcRandom) -> f32 {
    let p = cfg.end[usize::from(f.kind)];
    let v = p[0] + p[1] * randf(libc) - c.speed * p[2];
    v.max(if f.flags & 0x800 == 0 { 0.65 } else { 0.9 })
}

impl Auras {
    /// The fighter's flames and sparks alive, and whether it has an aura.
    #[cfg(test)]
    pub(crate) fn state(&self, chr: usize) -> (usize, usize, bool) {
        (
            self.flames
                .walk()
                .filter(|&i| self.flames.items[i].chr == chr)
                .count(),
            self.sparks
                .walk()
                .filter(|&i| self.sparks.items[i].chr == chr)
                .count(),
            self.tasks.iter().any(|a| a.chr == chr),
        )
    }

    fn task(&mut self, chr: usize) -> Option<&mut Aura> {
        self.tasks.iter_mut().find(|a| a.chr == chr)
    }

    /// `EftAura_IsVisible`.
    pub(crate) fn is_visible(&self, chr: usize) -> bool {
        self.tasks
            .iter()
            .any(|a| a.chr == chr && a.flags & 1 != 0 && !(a.alpha <= 0.0))
    }

    /// `EftAura_Command`: 0 start, 1 stop, 2 burst, 3 burst end, 4 burst peak, 5 fade in, 6 fade out, 7 kill.
    pub(crate) fn command(&mut self, cfg: &AuraCfg, c: &AuraChr, cmd: u8) {
        let chr = c.side;
        match cmd {
            0 => {
                // EftAura_Request(objId, 0).
                if let Some(a) = self.task(chr) {
                    a.variant = 0;
                    a.start(false);
                } else {
                    let mut a = Aura {
                        chr,
                        level: 1.0,
                        flags: 8,
                        ..Aura::default()
                    };
                    a.setup(cfg, c);
                    a.start(true);
                    self.tasks.push(a);
                }
            }
            1 => {
                if let Some(a) = self.task(chr)
                    && a.flags & 1 != 0
                    && a.fade != 3
                    && a.flags & 0x20 == 0
                {
                    a.fade = 3;
                    a.burst = 0;
                    a.burst_time = 0.0;
                }
            }
            2 | 3 => {
                if let Some(a) = self.task(chr)
                    && a.flags & 1 != 0
                {
                    a.burst_time = 0.0;
                    a.burst = if cmd == 2 { 1 } else { 4 };
                }
            }
            4 => {
                if let Some(a) = self.task(chr)
                    && a.flags & 1 != 0
                {
                    a.life_scale = 1.2;
                    a.burst_level = 1.0;
                    a.burst_time = 0.0;
                    a.burst = 2;
                }
            }
            5 | 6 => {
                if let Some(a) = self.task(chr)
                    && a.flags & 1 != 0
                    && a.fade != 3
                {
                    a.fade = if cmd == 5 { 1 } else { 2 };
                }
            }
            _ => {
                if let Some(a) = self.task(chr)
                    && a.flags & 1 != 0
                {
                    a.flags = 2;
                }
            }
        }
    }

    /// `EftAura_SetLevel`.
    pub(crate) fn set_level(&mut self, chr: usize, level: f32) {
        if let Some(a) = self.task(chr) {
            a.level = level;
        }
    }

    /// `EftAuraTask_Update` of every aura: the type again; unless paused the model nodes, the state, the flames
    /// and the sparks; a finished aura frees its particles.
    pub(crate) fn update(
        &mut self,
        cfg: &AuraCfg,
        chars: &[AuraChr; 2],
        paused: bool,
        libc: &mut LibcRandom,
    ) {
        let mut t = 0;
        while t < self.tasks.len() {
            let c = &chars[self.tasks[t].chr];
            self.tasks[t].setup(cfg, c);
            if !paused {
                let a = &mut self.tasks[t];
                for i in 0..SPARKS {
                    a.spark_pos[i] = c.node(cfg.spark[i][0]);
                }
                a.pos = c.node(3);
                a.push_centre = c.node(0x30);
                a.axis = c.node(0x11);
                for i in 0..PARTS {
                    a.part[i] = c.node(cfg.part[i][0]);
                }
                for i in 0..PARTS {
                    a.part_end[i] = c.node(cfg.part[i][1]);
                }
                for i in 0..PARTS {
                    a.part_ref[i] = c.node(cfg.part[i][2]);
                }
                if a.flags & 8 != 0 {
                    a.prev_pos = a.pos;
                    a.part_prev = a.part;
                    a.flags &= !8;
                }
                self.update_state(t, cfg, c, libc);
                // EftAura_UpdateSparks.
                if self.tasks[t].spark_state & 1 == 0 {
                    for i in 0..SPARKS {
                        self.spawn_sparks(t, cfg, c, i, false, libc);
                    }
                    self.tasks[t].spark_state |= 1;
                }
                self.step_sparks(t, cfg, c, libc);
                let a = &mut self.tasks[t];
                a.prev_pos = a.pos;
                a.part_prev = a.part;
                a.frame += 1;
            }
            if self.tasks[t].flags & 2 != 0 {
                // EftAuraTask_Term.
                let chr = self.tasks[t].chr;
                self.free_flames(chr);
                self.free_sparks(chr);
                self.tasks.remove(t);
            } else {
                t += 1;
            }
        }
    }

    fn free_flames(&mut self, chr: usize) {
        let mut l = Link::Head;
        while let Some(i) = self.flames.get(l) {
            if self.flames.items[i].chr != chr {
                l = Link::After(i);
            } else {
                self.flames.release(l, i);
            }
        }
    }

    fn free_sparks(&mut self, chr: usize) {
        let mut l = Link::Head;
        while let Some(i) = self.sparks.get(l) {
            if self.sparks.items[i].chr != chr {
                l = Link::After(i);
            } else {
                self.sparks.release(l, i);
            }
        }
    }

    /// `EftAura_SpawnSpark`: one spark of an emitter (the colour's draw even without a free spark).
    fn spawn_spark(
        &mut self,
        t: usize,
        cfg: &AuraCfg,
        c: &AuraChr,
        emitter: usize,
        follow: f32,
        libc: &mut LibcRandom,
    ) -> bool {
        let slot = self.sparks.alloc();
        let a = &self.tasks[t];
        let color = (libc.rand() % a.color_count.max(1)) as usize;
        let Some(slot) = slot else {
            return false;
        };
        let mut s = self.sparks.items[slot];
        s.chr = c.side;
        s.flags = 0;
        if a.flags & 0x80 != 0 {
            s.flags = 8;
        }
        if matches!(a.aura_type, 9 | 10) {
            s.flags |= 0x10;
        }
        if a.burst == 1 {
            s.flags |= 2;
        }
        s.emitter = emitter as u8;
        s.age = 0.0;
        s.life = if s.flags & 0x10 != 0 {
            cfg.spark_life[2] + cfg.spark_life[3] * randf(libc)
        } else {
            cfg.spark_life[0] + cfg.spark_life[1] * randf(libc)
        };
        s.follow = follow;
        s.mv = [0.0; 4];
        let (alpha, dir, size) = if a.flags & 0x20 == 0 {
            let col = a.spark_color[color];
            s.color = [col[0], col[1], col[2], 0.0];
            let alpha = col[3] * a.level;
            let dir = spark_dir(cfg, c, libc);
            (
                alpha,
                dir,
                cfg.spark_size[if s.flags & 8 != 0 { 1 } else { 0 }],
            )
        } else {
            let col = a.spark_color_alt[color];
            s.color = [col[0], col[1], col[2], 0.0];
            let dir = spark_dir_alt(cfg, c, libc);
            (
                col[3],
                dir,
                cfg.spark_size[if s.flags & 8 != 0 { 3 } else { 2 }],
            )
        };
        s.speed = size[1];
        s.dir = [dir[0], dir[1], dir[2], 1.0];
        s.color_rate = [0.0, 0.0, 0.0, alpha / cfg.spark_fade_in];
        s.rot = 0.0;
        s.rot_speed = cfg.spark_rot_speed * PI_F;
        if libc.rand() & 1 != 0 {
            s.rot_speed = -s.rot_speed;
        }
        if s.flags & 4 != 0 {
            s.rot_speed *= cfg.spark_mul[0][0];
            s.speed *= cfg.spark_mul[0][1];
        } else if s.flags & 2 != 0 {
            s.rot_speed *= cfg.spark_mul[1][0];
            s.speed *= cfg.spark_mul[1][1];
        }
        self.sparks.items[slot] = s;
        true
    }

    /// `EftAura_SpawnSparks`: one spark per `sparkStep` of the distance to the emitter's pull node (one only with
    /// `once`).
    fn spawn_sparks(
        &mut self,
        t: usize,
        cfg: &AuraCfg,
        c: &AuraChr,
        emitter: usize,
        once: bool,
        libc: &mut LibcRandom,
    ) {
        let a = &self.tasks[t];
        let step = cfg.spark_step * a.scale;
        let pos = a.spark_pos[emitter];
        let node = cfg.spark[emitter][1];
        let mut count = 1;
        if node >= 0 {
            let n = c.node(node);
            let d = [n[0] - pos[0], n[1] - pos[1], n[2] - pos[2]];
            let len = dot(d, d).sqrt();
            if step < len {
                count = (len / step + 1.0) as i32;
            }
        }
        for _ in 0..count {
            let follow = randf(libc);
            self.spawn_spark(t, cfg, c, emitter, follow, libc);
            if once {
                break;
            }
        }
    }

    /// `EftAura_StepSparks`: each spark follows its emitter (pulled towards its node), drifts, spins and fades; at
    /// the end of its life it is replaced by a new one.
    fn step_sparks(&mut self, t: usize, cfg: &AuraCfg, c: &AuraChr, libc: &mut LibcRandom) {
        let mut l = Link::Head;
        while let Some(i) = self.sparks.get(l) {
            if self.sparks.items[i].chr != c.side {
                l = Link::After(i);
                continue;
            }
            let s = self.sparks.items[i];
            if !(s.age < s.life) {
                self.spawn_sparks(t, cfg, c, usize::from(s.emitter), true, libc);
                // `*link = s->next`: a replacement linked in at the head goes with it when `link` is the head.
                self.sparks.release(l, i);
                continue;
            }
            let a = &self.tasks[t];
            let s = &mut self.sparks.items[i];
            let mut pos = a.spark_pos[usize::from(s.emitter)];
            let node = cfg.spark[usize::from(s.emitter)][1];
            if node >= 0 {
                let n = c.node(node);
                for k in 0..3 {
                    pos[k] += (n[k] - pos[k]) * s.follow;
                }
            }
            pos[3] = 1.0;
            for k in 0..3 {
                s.mv[k] += s.dir[k] * s.speed;
            }
            s.mv[3] += s.dir[3];
            let tt = s.age / s.life;
            s.speed += cfg.spark_accel * tt;
            s.speed *= cfg.spark_damp * tt;
            for k in 0..4 {
                pos[k] += s.mv[k];
            }
            pos[3] = 1.0;
            s.pos = pos;
            s.rot += s.rot_speed;
            if s.rot >= PI_F {
                s.rot -= 6.283_185_3;
            } else if s.rot <= -PI_F {
                s.rot += 6.283_185_3;
            }
            let fade_out = cfg.spark_fade_out;
            if s.life - s.age <= fade_out {
                if s.flags & 1 == 0 {
                    s.flags |= 1;
                    s.color_rate[3] = -s.color[3] / fade_out;
                }
            } else if cfg.spark_fade_in < s.age {
                s.color_rate[3] = 0.0;
            }
            s.color[3] = (s.color[3] + s.color_rate[3]).max(0.0);
            s.age += 1.0;
            l = Link::After(i);
        }
    }

    /// `EftAura_SpawnFlame`: one flame at a part (`second`: the extra flame of kind 2).
    #[allow(clippy::too_many_arguments)]
    fn spawn_flame(
        &mut self,
        t: usize,
        cfg: &AuraCfg,
        c: &AuraChr,
        part: usize,
        pos: V4,
        offset: V4,
        follow: f32,
        alt: bool,
        second: bool,
        libc: &mut LibcRandom,
    ) -> bool {
        let slot = self.flames.alloc();
        let a = &self.tasks[t];
        let color = (libc.rand() % a.color_count.max(1)) as usize;
        let Some(slot) = slot else {
            return false;
        };
        let mut f = self.flames.items[slot];
        let (life_scale, level, burst) = (a.life_scale, a.level, a.burst_level);
        f.chr = c.side;
        f.part = part as u8;
        f.flags = a.flame_flags(second, alt);
        if second {
            f.shape = 1;
            f.kind = 2;
        } else {
            f.shape = 0;
            f.kind = u8::from(f.flags & 0x400 != 0);
        }
        f.life = flame_life(cfg, f.flags, burst, libc);
        f.time = f.life;
        f.offset = offset;
        f.follow = follow;
        f.speed = flame_speed(cfg, f.flags, c, level, burst, libc);
        f.power = life_scale * burst;
        f.scale = a.scale * level;
        f.size = flame_size(cfg, &f, 0);
        f.stretch = flame_stretch(cfg, &f, c, 0, libc);
        let grow = f.life * cfg.grow_ratio;
        f.size_rate = (flame_size(cfg, &f, 1) - f.size) / grow;
        f.stretch_rate = (flame_stretch(cfg, &f, c, 1, libc) - f.stretch) / grow;
        f.end = flame_end(cfg, &f, c, libc);
        let mut dir = spark_dir(cfg, c, libc);
        let drag = a.drag_dir(cfg, pos, part);
        if f.flags & 0x100 != 0 {
            f.tex = (libc.rand() % a.tex_count.max(1)) as u8;
        }
        if f.flags & 0x800 == 0 {
            f.alt = 0;
            let s = randf(libc) * 0.2 + 1.0;
            let cs = a.color_start;
            f.color = [cs[0] * s, cs[1] * s, cs[2] * s, 0.0];
            let inv = 1.0 / f.life;
            for k in 0..3 {
                f.color_rate[k] = (a.color_end[k] - f.color[k]) * inv;
            }
            f.color_rate[3] = flame_alpha(cfg, f.flags, cs[3]) * level / cfg.fade_in;
        } else {
            f.alt = 1;
            let ca = a.color_alt[color];
            f.color = [ca[0], ca[1], ca[2], 0.0];
            let inv = 1.0 / f.life;
            for k in 0..3 {
                f.color_rate[k] = (ca[k] - f.color[k]) * inv;
            }
            f.color_rate[3] = flame_alpha(cfg, f.flags, ca[3]) * level / cfg.fade_in;
        }
        let kind = cfg.kinds[usize::from(f.kind)];
        let r1 = kind.side[0] + kind.side[1] * randf(libc);
        let r2 = kind.away[0] + kind.away[1] * randf(libc);
        let r3 = kind.out[0] + kind.out[1] * randf(libc);
        let side = a.side_dir(cfg, c, f.power, r1, r2, libc);
        let out = a.out_dir(cfg, c, part, f.power, r3, libc);
        let turn = norm([dir[0] + drag[0], dir[1] + drag[1], dir[2] + drag[2]]);
        f.dir_turn = [turn[0], turn[1], turn[2], dir[3] + drag[3]];
        for k in 0..4 {
            dir[k] = (dir[k] + side[k]) + drag[k];
        }
        let n = norm(v3(dir));
        dir = [n[0], n[1], n[2], dir[3]];
        let fd = norm([out[0] + dir[0], out[1] + dir[1], out[2] + dir[2]]);
        f.dir = [fd[0], fd[1], fd[2], out[3] + dir[3]];
        f.dir_start = f.dir;
        let dt = norm([
            f.dir[0] - f.dir_turn[0],
            f.dir[1] - f.dir_turn[1],
            f.dir[2] - f.dir_turn[2],
        ]);
        f.dir_turn = [dt[0], dt[1], dt[2], f.dir[3] - f.dir_turn[3]];
        f.mv = [
            f.dir[0] * cfg.start_dist,
            f.dir[1] * cfg.start_dist,
            f.dir[2] * cfg.start_dist,
            f.dir[3],
        ];
        f.pos = [
            pos[0] + offset[0] + f.mv[0],
            pos[1] + offset[1] + f.mv[1],
            pos[2] + offset[2] + f.mv[2],
            1.0,
        ];
        f.end_time = f.life * cfg.end_ratio;
        f.flip = (libc.rand() % 2) as u8;
        self.flames.items[slot] = f;
        true
    }

    /// `EftAura_SpawnFlames`: one flame per `flameStep` of a part's length (one only with `once`) at a random
    /// point of it, each with a possible second flame while fewer than 45 are alive.
    fn spawn_flames(
        &mut self,
        t: usize,
        cfg: &AuraCfg,
        c: &AuraChr,
        part: usize,
        once: bool,
        libc: &mut LibcRandom,
    ) -> bool {
        let a = &self.tasks[t];
        let scale = a.scale;
        let step = cfg.flame_step * scale;
        let pos = a.part[part];
        let mut delta = [0.0, 0.0, 0.0, 1.0];
        let mut count = 1;
        let node = cfg.part[part][1];
        if node >= 0 {
            let n = c.node(node);
            let d = [n[0] - pos[0], n[1] - pos[1], n[2] - pos[2], n[3] - pos[3]];
            let len = dot(v3(d), v3(d)).sqrt();
            delta = d;
            if step < len {
                count = (len / step + 1.0) as i32;
            }
        }
        let mut ret = false;
        for _ in 0..count {
            let tt = if part < 2 {
                randf(libc) * 0.4 + 0.3
            } else {
                randf(libc)
            };
            let _along = delta.map(|x| x * tt);
            let r1 = randf(libc) * 0.05 + 0.3;
            let r2 = randf(libc) * 0.05 + 0.3;
            let a = &mut self.tasks[t];
            let radial = a.part_offset(part, r1, r2).map(|x| x * scale);
            let offset = [radial[0], radial[1], radial[2], 0.0];
            let mut alt = if a.alt_mask & (1 << part) != 0 {
                a.alt_mask &= !(1 << part);
                true
            } else {
                a.alt_mask |= 1 << part;
                false
            };
            if matches!(a.burst, 1 | 2) {
                alt = part >= 6;
            }
            ret = self.spawn_flame(t, cfg, c, part, pos, offset, tt, alt, false, libc);
            if ret {
                self.tasks[t].flame_count += 1;
                if self.tasks[t].flame_count < 45
                    && (part < 2 || libc.rand() & 1 == 0)
                    && self.spawn_flame(t, cfg, c, part, pos, offset, tt, false, true, libc)
                {
                    self.tasks[t].flame_count += 1;
                }
            }
            if once {
                break;
            }
        }
        ret
    }

    /// `EftAura_UpdateState`: the first flames of the ten parts, the burst, the three fades, then the flames.
    fn update_state(&mut self, t: usize, cfg: &AuraCfg, c: &AuraChr, libc: &mut LibcRandom) {
        while self.tasks[t].parts_started < PARTS {
            let p = self.tasks[t].parts_started;
            self.spawn_flames(t, cfg, c, p, false, libc);
            self.tasks[t].parts_started += 1;
        }
        let a = &mut self.tasks[t];
        match a.burst {
            0 => {}
            1 => {
                a.burst_level += 0.15;
                a.life_scale = 1.2;
                if 1.0 <= a.burst_level {
                    a.burst_level = 1.0;
                }
            }
            2 => {
                a.burst_level = 1.0;
                a.burst_time += 1.0;
                if cfg.burst_hold <= a.burst_time {
                    a.burst = 3;
                    a.burst_time = 0.0;
                    a.fade_a = 1;
                    a.fade_b = 1;
                }
            }
            _ => {
                a.burst_level -= 0.1;
                if a.burst_level <= 0.0 {
                    a.burst_level = 0.0;
                    a.life_scale -= 0.2;
                    if a.life_scale <= 1.0 {
                        a.life_scale = 1.0;
                    }
                }
                if a.life_scale <= 1.0 && a.burst_level <= 0.0 {
                    a.burst = 0;
                }
            }
        }
        a.burst_speed = cfg.burst_speed * a.burst_level;
        if a.fade_a == 1 {
            if 0.0 < a.alpha_fade {
                a.alpha_fade -= 1.0 / cfg.alpha_fade_out;
                if a.alpha_fade <= 0.0 {
                    a.alpha_fade = 0.0;
                    a.alpha_fade_rate = 0.0;
                    a.fade_a = 0;
                }
            }
        } else if a.alpha_fade < 1.0 {
            a.alpha_fade_rate += 1.0 / cfg.alpha_fade_in;
            a.alpha_fade += a.alpha_fade_rate;
            if 1.0 <= a.alpha_fade {
                a.alpha_fade = 1.0;
                a.alpha_fade_rate = 0.0;
            }
        }
        if a.fade_b == 1 {
            if 0.0 < a.size_fade {
                a.size_fade -= 1.0 / cfg.size_fade_out;
                if a.size_fade <= 0.0 {
                    a.size_fade = 0.0;
                    a.size_fade_rate = 0.0;
                    a.fade_b = 0;
                }
            }
        } else if a.size_fade < 1.0 {
            a.size_fade_rate += 1.0 / cfg.size_fade_in;
            a.size_fade += a.size_fade_rate;
            if 1.0 <= a.size_fade {
                a.size_fade = 1.0;
                a.size_fade_rate = 0.0;
            }
        }
        if a.fade == 1 {
            a.alpha += 1.0 / cfg.a_fade_in;
            if 1.0 <= a.alpha {
                a.alpha = 1.0;
                a.fade = 0;
            }
        } else if a.fade != 0 {
            a.alpha -= 1.0 / cfg.a_fade_out;
            if a.alpha <= 0.0 {
                if a.fade == 3 {
                    a.flags |= 2;
                }
                a.alpha = 0.0;
                a.fade = 0;
            }
        }
        self.step_flames(t, cfg, c, libc);
    }

    /// `EftAura_StepFlames`: each flame follows its part, bends and accelerates, grows, colours and fades; near
    /// its end it spawns its successor, and a finished one is freed (after its successor is spawned).
    fn step_flames(&mut self, t: usize, cfg: &AuraCfg, c: &AuraChr, libc: &mut LibcRandom) {
        let mut l = Link::Head;
        while let Some(i) = self.flames.get(l) {
            if self.flames.items[i].chr != c.side {
                l = Link::After(i);
                continue;
            }
            let f = self.flames.items[i];
            if 0.0 < f.time {
                let mut f = f;
                let a = &self.tasks[t];
                let part = usize::from(f.part);
                let base = a.part[part];
                let mut pos = base;
                if cfg.part[part][1] >= 0 {
                    let end = a.part_end[part];
                    for k in 0..3 {
                        pos[k] += (end[k] - base[k]) * f.follow;
                    }
                }
                for k in 0..4 {
                    pos[k] += f.offset[k];
                }
                let tt = f.time / f.life;
                turn_flame(cfg, &mut f, pos, tt);
                let kind = cfg.kinds[usize::from(f.kind)];
                f.speed = (f.speed + cfg.speed[1] * (1.0 - tt) * kind.accel) * (tt * 0.2 + 0.8);
                let sp = f.speed + a.burst_speed;
                for k in 0..3 {
                    f.mv[k] += f.dir[k] * sp;
                }
                f.pos = [pos[0] + f.mv[0], pos[1] + f.mv[1], pos[2] + f.mv[2], 1.0];
                f.size = (f.size + f.size_rate).max(0.0);
                f.stretch = (f.stretch + f.stretch_rate).max(0.0);
                if f.flags & 8 == 0 && cfg.grow_ratio < 1.0 - f.time / f.life {
                    f.size_rate = (flame_size(cfg, &f, 2) - f.size) / f.time;
                    f.stretch_rate = (flame_stretch(cfg, &f, c, 2, libc) - f.stretch) / f.time;
                    f.flags |= 8;
                }
                if f.flags & 1 == 0 && cfg.fade_in <= f.life - f.time {
                    f.color_rate[3] = 0.0;
                    f.flags |= 1;
                }
                if f.flags & 2 == 0 && f.time <= cfg.fade_out {
                    f.color_rate[3] = -((f.color[3] - a.color_end[3]) * (1.0 / (f.time + 1.0)));
                    f.flags |= 2;
                }
                f.color = std::array::from_fn(|k| (f.color[k] + f.color_rate[k]).clamp(0.0, 1.0));
                f.time -= 1.0;
                let mut successor = false;
                if f.time <= f.end_time {
                    if f.flags & 0x10 == 0 {
                        f.size_rate = (flame_size(cfg, &f, 3) - f.size) / f.time;
                        f.stretch_rate = (flame_stretch(cfg, &f, c, 3, libc) - f.stretch) / f.time;
                        f.flags |= 0x10;
                    }
                    if f.flags & 4 == 0 {
                        if f.flags & 0x800 != 0 {
                            f.flags |= 4;
                        } else {
                            successor = true;
                        }
                    }
                }
                self.flames.items[i] = f;
                if successor && self.spawn_flames(t, cfg, c, usize::from(f.part), true, libc) {
                    self.flames.items[i].flags |= 4;
                }
                l = Link::After(i);
            } else if f.flags & 4 == 0 {
                // The walk stays on the same link: with a successor linked in at the head, the next item is it.
                // (Without a free flame the original would retry forever; the flame is let go instead.)
                if !self.spawn_flames(t, cfg, c, usize::from(f.part), true, libc) {
                    self.flames.items[i].time = 0.0;
                }
                self.flames.items[i].flags |= 4;
            } else {
                self.flames.release(l, i);
                self.tasks[t].flame_count -= 1;
            }
        }
    }

    /// `EftAuraTask_Draw` of every aura: the flames, unless the fighter is hidden, in a technique or a rush
    /// sequence, has a glow, or the aura is hidden or of the undrawn variant.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &self,
        cfg: &AuraCfg,
        tex: &AuraTextures,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
        chars: &[AuraChr; 2],
        shown: impl Fn(usize) -> Option<f32>,
        stage_flag: bool,
    ) {
        for a in &self.tasks {
            let Some(alpha) = shown(a.chr) else {
                continue;
            };
            if a.flags & (4 | 0x20) != 0 {
                continue;
            }
            self.draw_flames(cfg, tex, a, &chars[a.chr], alpha, stage_flag, ot, ctx);
        }
    }

    /// `EftAura_DrawFlames`: each flame a quad standing along its direction, leaning with the camera, its far
    /// corners stretched by its end ratio, its alpha falling off near the body and a tenth at the near corners.
    #[allow(clippy::too_many_arguments)]
    fn draw_flames(
        &self,
        cfg: &AuraCfg,
        tex: &AuraTextures,
        a: &Aura,
        c: &AuraChr,
        alpha: f32,
        stage_flag: bool,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
    ) {
        let v = &ctx.world2view;
        let cam = [v[0][2], v[1][2], v[2][2]];
        let corner = if a.flags & 0x80 != 0 {
            &cfg.corner_b
        } else {
            &cfg.corner
        };
        let nodes: [V4; 7] = std::array::from_fn(|i| c.node(cfg.fade_node[i]));
        let k = a.alpha * a.alpha_fade * alpha;
        for i in self.flames.walk() {
            let f = &self.flames.items[i];
            if f.chr != a.chr {
                continue;
            }
            let mut dir = v3(f.dir);
            let part = usize::from(f.part);
            if cfg.part[part][2] >= 0 {
                let d = dot(cam, dir).abs();
                let r = a.part_ref[part];
                let away = norm([r[0] - a.pos[0], 0.0, r[2] - a.pos[2]]);
                dir[0] += away[0] * d * 0.5;
                dir[2] += away[2] * d * 0.5;
            }
            let m = flame_mtx(cfg, cam, norm(dir), f.pos);
            let w = f.stretch * f.scale * a.size_fade;
            let h = f.size * f.scale * a.size_fade;
            let mut scr = [(0i32, 0i32, 0i32); 4];
            let mut stq = [[0.0f32; 3]; 4];
            let mut fade = [0.0f32; 4];
            let mut clipped = false;
            for q in 0..4 {
                let cc = corner[usize::from(f.shape)][q];
                let mut cz = cc[2] * h;
                if q >= 2 {
                    cz *= f.end;
                }
                let p = apply(&m, [cc[0] * w, cc[1] * f.scale, cz, 1.0]);
                let s = apply(&ctx.world2screen, p);
                let iq = 1.0 / s[3];
                let (x, y, z, gw) = (
                    (s[0] * iq * 16.0) as i32,
                    (s[1] * iq * 16.0) as i32,
                    (s[2] * iq) as i32,
                    s[3] as i32,
                );
                if x as u32 > 0xFFF0 || y as u32 > 0xFFF0 || z < 0 {
                    clipped = true;
                    break;
                }
                scr[q] = (x, y, z);
                let uv = cfg.uv[usize::from(f.shape * 2 + f.flip)][q];
                let r = 1.0 / gw as f32;
                stq[q] = [uv[0] * r, uv[1] * r, uv[2] * r];
                fade[q] = node_fade(cfg, v3(p), &nodes, a.scale);
                if q < 2 {
                    fade[q] *= 0.1;
                }
            }
            if clipped || f.flags & 0x800 != 0 {
                continue;
            }
            let z = scr
                .iter()
                .fold(0i32, |s, v| s.wrapping_add(v.2))
                .wrapping_shr(10);
            let rgb = [f.color[0], f.color[1], f.color[2]].map(|x| (x * 255.0) as u32 as u8);
            let texture = if a.flags & 0x80 != 0 {
                usize::try_from(a.tex_first - 8 + i32::from(f.tex))
                    .ok()
                    .and_then(|t| tex.paired.get(t).copied().flatten())
            } else {
                usize::try_from(a.aura_type)
                    .ok()
                    .and_then(|t| tex.plain.get(t).copied().flatten())
            };
            let layer = if stage_flag { 1 } else { usize::from(f.alt) };
            let vert = |q: usize| EffectVertex {
                position: [
                    scr[q].0 as f32 / 16.0 - OFFSET_X,
                    scr[q].1 as f32 / 16.0 - OFFSET_Y,
                    (scr[q].2 & 0xFF_FFFF) as f32,
                ],
                stq: stq[q],
                rgba: [
                    rgb[0],
                    rgb[1],
                    rgb[2],
                    (fade[q] * (f.color[3] * 255.0 * k)) as u32 as u8,
                ],
            };
            for q in 0..2 {
                ot.add(
                    depth_slot(z),
                    layer,
                    EffectTri {
                        texture,
                        blend: EffectBlend::of(layer, false),
                        clamp: false,
                        zwrite: false,
                        alpha_ref: 0,
                        vertices: [vert(q), vert(q + 1), vert(q + 2)],
                    },
                );
            }
        }
    }
}

/// `EftAura_TurnFlame`: early towards its start direction and turn axis, late towards `pos`.
fn turn_flame(cfg: &AuraCfg, f: &mut Flame, pos: V4, t: f32) {
    let t3 = t * t * t;
    let turn = cfg.kinds[usize::from(f.kind)].turn;
    for k in 0..3 {
        f.dir[k] += f.dir_start[k] * ((1.0 - t3) * 0.1 * turn);
    }
    for k in 0..3 {
        f.dir[k] += f.dir_turn[k] * ((1.0 - t) * 0.08 * turn);
    }
    let d = norm([pos[0] - f.pos[0], pos[1] - f.pos[1], pos[2] - f.pos[2]]);
    for k in 0..3 {
        f.dir[k] += d[k] * (t3 * 0.15 * turn);
    }
    let n = norm(v3(f.dir));
    f.dir = [n[0], n[1], n[2], 1.0];
}

/// `EftAura_BuildFlameMtx`: the third axis along `dir` leaned by the camera's view axis.
fn flame_mtx(cfg: &AuraCfg, view: V3, dir: V3, pos: V4) -> Mtx {
    let d = dot(view, dir);
    let (lean, v) = if d < 0.0 {
        (-d * cfg.lean, view.map(|x| -x))
    } else {
        (d * cfg.lean, view)
    };
    let v = norm([dir[0] - v[0], dir[1] - v[1], dir[2] - v[2]]).map(|x| x * lean);
    let z = norm([dir[0] + v[0], dir[1] + v[1], dir[2] + v[2]]);
    let x = norm(cross(view, z));
    let y = norm(cross(x, z));
    let mut m = IDENTITY;
    for r in 0..3 {
        m[r] = [x[r], y[r], z[r], 0.0];
    }
    m[3] = [pos[0], pos[1], pos[2], 1.0];
    m
}

/// `EftAura_GetNodeFade`: 0.5..1 by the distance to the two nearest of seven nodes (the running nearest, as
/// the original keeps it).
fn node_fade(cfg: &AuraCfg, pos: V3, nodes: &[V4; 7], scale: f32) -> f32 {
    let mut best = [-1.0f32, -1.0];
    let mut dist = 0.0f32;
    for (i, n) in nodes.iter().enumerate() {
        let d = [pos[0] - n[0], pos[1] - n[1], pos[2] - n[2]];
        let t = dot(d, d);
        if i == 0 || t < dist {
            dist = t;
        }
        if best[0] < 0.0 {
            best[0] = dist;
        } else if best[1] < 0.0 {
            if dist < best[0] {
                best[1] = best[0];
                best[0] = dist;
            } else {
                best[1] = dist;
            }
        } else if dist < best[0] {
            best[1] = best[0];
            best[0] = dist;
        } else if dist < best[1] {
            best[1] = dist;
        }
    }
    let (b0, b1) = (best[0].sqrt(), best[1].sqrt());
    let dist = b0 * cfg.near + b1 * (1.0 - cfg.near);
    let limit = scale * 4.5;
    let mut ratio = 1.0;
    if dist < limit {
        let r = dist / limit;
        let r3 = r * r * r;
        ratio = r3 * r3;
    }
    ratio * 0.5 + 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block of entry 1's layout with plain values.
    fn block() -> Vec<u8> {
        let mut b = vec![0u8; 0xC60 + 5 * 13 * 16];
        let mut put = |o: usize, v: f32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put(0, 0.3); // endRatio
        put(4, 0.3); // growRatio
        put(0x10, 2.0); // flameStep
        put(0x1C, 20.0); // life base
        put(0x2C, 3.0); // fadeIn
        put(0x30, 3.0); // fadeOut
        put(0x34, 1.0); // speed[0]
        for o in [0x4C, 0x54, 0x5C] {
            put(o, 1.0);
        }
        for o in (0x64..0x84).step_by(4) {
            put(o, 1.0);
        }
        for o in (0x9C..0xB8).step_by(4) {
            put(o, 1.0);
        }
        for o in [0xBC, 0xC8, 0xD4, 0xE0, 0xEC, 0xF8] {
            put(o, 1.0);
        }
        put(0x164, 1.0);
        put(0x168, 1.0);
        for o in (0x16C..0x17C).step_by(4) {
            put(o, 1.0);
        }
        put(0x194, 4.0); // fadeInTime
        put(0x198, 4.0); // fadeOutTime
        put(0x1A8, 2.0);
        put(0x1AC, 2.0);
        put(0x1B0, 2.0);
        put(0x1B4, 2.0);
        put(0x1B8, 2.0); // sparkFadeIn
        put(0x1BC, 2.0); // sparkFadeOut
        put(0x1C0, 10.0); // sparkLife
        put(0x1D0, 100.0); // sparkStep
        for i in 0..10 {
            b[0x410 + i * 12..0x414 + i * 12].copy_from_slice(&(3 + i as i32).to_le_bytes());
            b[0x414 + i * 12..0x418 + i * 12].copy_from_slice(&(-1i32).to_le_bytes());
            b[0x418 + i * 12..0x41C + i * 12].copy_from_slice(&(-1i32).to_le_bytes());
        }
        for i in 0..13 {
            b[0x4A4 + i * 8..0x4A8 + i * 8].copy_from_slice(&3i32.to_le_bytes());
            b[0x4A8 + i * 8..0x4AC + i * 8].copy_from_slice(&(-1i32).to_le_bytes());
        }
        b
    }

    fn chr(side: usize, node_pos: &dyn Fn(u32) -> V3) -> AuraChr<'_> {
        AuraChr {
            side,
            height: 19.35,
            aura_type: 2,
            dir: [0.0, 0.0, 1.0],
            vel: [0.0; 3],
            speed: 0.0,
            fall_speed: 0.0,
            fx_kind: 1,
            node_pos,
        }
    }

    #[test]
    fn the_settings_read() {
        let cfg = AuraCfg::parse(&block()).unwrap();
        assert_eq!(cfg.part[2], [5, -1, -1]);
        assert_eq!(cfg.spark[0], [3, -1]);
        assert_eq!(cfg.a_fade_in, 4.0);
        assert!(AuraCfg::parse(&[0; 8]).is_none());
    }

    #[test]
    fn the_aura_rises_holds_and_fades_out_with_its_particles() {
        let cfg = AuraCfg::parse(&block()).unwrap();
        let node_pos = |n: u32| [0.0, -(n as f32), 100.0];
        let chars = [chr(0, &node_pos), chr(1, &node_pos)];
        let mut a = Auras::default();
        let mut libc = LibcRandom::new(1);
        a.command(&cfg, &chars[0], 0);
        for _ in 0..10 {
            a.update(&cfg, &chars, false, &mut libc);
            a.command(&cfg, &chars[0], 0);
        }
        let (flames, sparks, on) = a.state(0);
        // Ten parts' flames (and their seconds), twelve emitters' sparks, within the pools.
        assert!(
            on && flames >= 10 && flames <= 2 * FLAMES_PER && sparks == SPARKS,
            "{flames} {sparks}"
        );
        assert!(a.is_visible(0));
        // Stopped: it fades out over the settings' time and frees everything.
        a.command(&cfg, &chars[0], 1);
        for _ in 0..5 {
            a.update(&cfg, &chars, false, &mut libc);
        }
        assert_eq!(a.state(0), (0, 0, false));
        // The pools serve the other fighter.
        a.command(&cfg, &chars[1], 0);
        a.update(&cfg, &chars, false, &mut libc);
        assert!(a.state(1).0 > 0);
        a.command(&cfg, &chars[1], 7);
        a.update(&cfg, &chars, true, &mut libc);
        assert_eq!(a.state(1), (0, 0, false));
    }

    #[test]
    fn a_dying_head_spark_takes_its_replacement_with_it() {
        let mut l: List<Spark> = List::new(4);
        let a = l.alloc().unwrap();
        let b = l.alloc().unwrap();
        // b is the head; a replacement is linked in front of it, then b is released through the head link.
        let r = l.alloc().unwrap();
        l.release(Link::Head, b);
        assert_eq!(l.walk().collect::<Vec<_>>(), [a]);
        assert_eq!(l.free, Some(b));
        // The replacement is in neither list.
        let _ = r;
        assert_eq!(l.items.len(), 3);
    }

    #[test]
    fn the_node_fade_falls_off_near_the_body() {
        let cfg = AuraCfg {
            near: 0.5,
            ..AuraCfg::default()
        };
        let nodes = [[0.0, 0.0, 0.0, 1.0]; 7];
        assert_eq!(node_fade(&cfg, [100.0, 0.0, 0.0], &nodes, 1.0), 1.0);
        assert_eq!(node_fade(&cfg, [0.0; 3], &nodes, 1.0), 0.5);
        let mid = node_fade(&cfg, [2.25, 0.0, 0.0], &nodes, 1.0);
        assert!((mid - (0.5f32.powi(6) * 0.5 + 0.5)).abs() < 1e-6);
    }
}
