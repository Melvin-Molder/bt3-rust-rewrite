//! The ground dust (`EftGndDust*`, src/battle/eft_ground_dust.c and eft_char_parts.c): tasks of a few particles
//! drawn from two pools (64 and 63 particles) with the common effect file's texture set 0xB.
//!
//! Kind 0, the puff, is what a breaking stage object raises (`EftGndDust_SpawnDebris`): four particles at random
//! points around it flying outwards, coloured by the stage's dust record. The fighters' kinds take the stage's
//! light colours: 1, the slide (request 0x30), and 2, the dash (0x33), follow their fighter and emit every 5th /
//! 6th frame while switched on, leaving their particles behind when it turns; 3, a burst of dust rising from the
//! body (0x34, or 0x35 under water); 4, the landing ring (0x35); 5, two small puffs at a foot (0x32) or a
//! projectile's impact. A particle lives, then fades; its colour runs from the first colour to the second.

use bt3_data::stage::StageDust;

use crate::eft::{
    DrawCtx, OrderTable, V3, V4, add, cross, dot, draw_sprite, norm, scale3, sub, wrap_angle,
};
use crate::rng::{LibcRandom, Vu0Random};

/// `EFT_GDUST_PART_*`.
const PART_FADING: u32 = 0x4;
const PART_DONE: u32 = 0x8;
const PART_FOLLOW: u32 = 0x10;
const PART_TURN: u32 = 0x20;
const PART_FAR: u32 = 0x40;

/// `EFT_GDUST_*`: an emitter's state.
const EMIT: u32 = 0x2;
const FORCE_DROP: u32 = 0x4;
const DROP: u32 = 0x8;

/// `EFT_RANDF`: libc `rand()` scaled to 0..1.
fn randf(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// `EftGndDustArg`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DustArg {
    pub pos: V3,
    pub dir: V3,
    pub accel: V3,
    pub col_a: [u8; 4],
    pub col_b: [u8; 4],
    /// The owner (-1 none).
    pub chr: i32,
    pub pool: usize,
    pub blend: u8,
    pub tex: usize,
    pub size: i16,
    pub life: i16,
    pub fade: i16,
    pub spin: i16,
    pub r_min: i16,
    pub r_max: i16,
    pub grow: f32,
    pub bright: f32,
    pub speed: f32,
    pub speed_rand: f32,
    pub drag: f32,
    pub gravity: f32,
    pub scale: f32,
}

impl Default for DustArg {
    fn default() -> Self {
        Self {
            pos: [0.0; 3],
            dir: [0.0; 3],
            accel: [0.0; 3],
            col_a: [0; 4],
            col_b: [0; 4],
            chr: -1,
            pool: 0,
            blend: 0,
            tex: 0,
            size: 0,
            life: 0,
            fade: 0,
            spin: 0,
            r_min: 0,
            r_max: 0,
            grow: 0.0,
            bright: 0.0,
            speed: 0.0,
            speed_rand: 0.0,
            drag: 0.0,
            gravity: 0.0,
            scale: 0.0,
        }
    }
}

impl DustArg {
    /// `EftGndDust_InitTemplate`: the debris template, with the stage's dust record when it is enabled.
    pub(crate) fn debris(stage: Option<&StageDust>) -> Self {
        let mut t = Self {
            pos: [0.0, -100.0, 0.0],
            dir: [0.0, -1.0, 0.0],
            accel: [0.0, 1.0, 0.0],
            col_a: [80, 55, 35, 80],
            col_b: [80, 65, 45, 30],
            size: 500,
            life: 30,
            fade: 30,
            r_min: 17,
            r_max: 40,
            grow: 0.01,
            bright: 1.0,
            speed: 0.5,
            drag: 0.9,
            scale: 1.0,
            ..Self::default()
        };
        if let Some(s) = stage.filter(|s| s.enabled) {
            t.col_a = s.col_a;
            t.col_b = s.col_b;
            t.speed = s.speed;
            t.speed_rand = s.speed_rand;
            t.size = s.size as i16;
            t.r_min = i16::from(s.r_min);
            t.r_max = i16::from(s.r_max);
        }
        t
    }
}

/// A fighter as the dust reads it: position, direction of travel, this frame's move, height, body scale
/// (`BtlScene_GetCharScale`: height / 19.35) and model nodes 0..0x30.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DustChr {
    pub pos: V3,
    pub dir: V3,
    pub moved: V3,
    pub height: f32,
    pub scale: f32,
    pub nodes: [V3; 0x31],
}

/// `EftGndDustPart`.
#[derive(Clone, Copy, Debug, Default)]
struct Part {
    pos: V3,
    base: V3,
    dir: V3,
    color: V4,
    vel: V3,
    size: [f32; 2],
    grow: [f32; 2],
    accel: V3,
    col_a: [u8; 4],
    col_b: [u8; 4],
    life: i16,
    life_max: i16,
    fade_len: i16,
    fade: i16,
    scale: i16,
    angle: f32,
    spin: f32,
    speed: f32,
    drag: f32,
    alpha: f32,
    grow_damp: f32,
    flags: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Puff,
    Slide,
    Dash,
    Burst,
    Land,
    Impact,
}

/// A dust task (`EftGndDustEmit`): its argument, particles and, for the fighters' kinds, the owner's state.
#[derive(Clone, Debug)]
struct Task {
    id: u32,
    kind: Kind,
    arg: DustArg,
    parts: Vec<Part>,
    tick: i32,
    flags: u32,
    part_flags: u32,
    mov: V3,
    pos: V3,
    dir: V3,
}

impl Task {
    fn new(id: u32, kind: Kind, arg: DustArg) -> Self {
        Self {
            id,
            kind,
            arg,
            parts: Vec::new(),
            tick: 0,
            flags: 0,
            part_flags: 0,
            mov: [0.0; 3],
            pos: [0.0; 3],
            dir: [0.0; 3],
        }
    }

    /// The owner's position and the direction against its travel.
    fn follow(&mut self, c: &DustChr) {
        self.pos = c.pos;
        self.dir = norm(scale3(c.dir, -1.0));
    }
}

/// `gEftGndDust`: the debris template, the stage's light colours, the pools' free counts, the tasks and the
/// fighters' slide and dash emitters.
#[derive(Clone, Debug)]
pub(crate) struct GndDust {
    tmpl: DustArg,
    light: Option<[u8; 8]>,
    free: [usize; 2],
    tasks: Vec<Task>,
    slide: [Option<u32>; 2],
    dash: [Option<u32>; 2],
    next_id: u32,
}

impl Default for GndDust {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl GndDust {
    /// `EftGndDustMgr_Init`: both pools free and the debris template; `light` is the stage's light colours.
    pub(crate) fn new(stage: Option<&StageDust>, light: Option<[u8; 8]>) -> Self {
        Self {
            tmpl: DustArg::debris(stage),
            light,
            free: [64, 63],
            tasks: Vec::new(),
            slide: [None; 2],
            dash: [None; 2],
            next_id: 0,
        }
    }

    /// `BtlScene_Reset`: every task ends, its particles back in their pools.
    pub(crate) fn reset(&mut self) {
        for t in self.tasks.drain(..) {
            self.free[t.arg.pool] += t.parts.len();
        }
        self.slide = [None; 2];
        self.dash = [None; 2];
    }

    /// The particles alive.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.tasks.iter().map(|t| t.parts.len()).sum()
    }

    /// The particles alive of the fighters' kinds.
    #[cfg(test)]
    pub(crate) fn fighter_parts(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.kind != Kind::Puff)
            .map(|t| t.parts.len())
            .sum()
    }

    /// `EftGndDust_GetLightColors`: the stage's two light colours (grey without), alphas raised by 8.
    fn light_colors(&self) -> ([u8; 4], [u8; 4]) {
        let (mut a, mut b) = match self.light {
            Some(c) => ([c[0], c[1], c[2], c[3]], [c[4], c[5], c[6], c[7]]),
            None => ([0x80; 4], [0x80, 0x80, 0x80, 0]),
        };
        a[3] = a[3].wrapping_add(8);
        b[3] = b[3].wrapping_add(8);
        (a, b)
    }

    fn push(&mut self, kind: Kind, arg: DustArg) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.tasks.push(Task::new(id, kind, arg));
        self.tasks.len() - 1
    }

    fn index(&self, id: Option<u32>) -> Option<usize> {
        id.and_then(|id| self.tasks.iter().position(|t| t.id == id))
    }

    /// `EftGndDust_SpawnDebris`: a puff of the template at `pos` with the brightness `bright`.
    pub(crate) fn spawn_debris(&mut self, pos: V3, bright: f32, vu0: &mut Vu0Random) {
        let arg = DustArg {
            pos,
            scale: 1.0,
            bright,
            ..self.tmpl
        };
        self.spawn_puff(arg, vu0);
    }

    /// `EftGndDustPuff_Init`: four particles (while the pool lasts) in random directions at a random distance
    /// between the radii, flying outwards at the speed and its random part.
    fn spawn_puff(&mut self, arg: DustArg, vu0: &mut Vu0Random) {
        let t = self.push(Kind::Puff, arg);
        for _ in 0..4 {
            if self.free[arg.pool] == 0 {
                break;
            }
            self.free[arg.pool] -= 1;
            let k = arg.gravity * arg.scale + arg.speed_rand;
            let mut dir = [0.0; 3];
            for d in &mut dir {
                *d = vu0.float01() * 2.0 - 1.0;
            }
            let dir = norm(dir);
            let (r_min, r_max) = (
                f32::from(arg.r_min) * arg.scale,
                f32::from(arg.r_max) * arg.scale,
            );
            let mut pos = [0.0; 3];
            for i in 0..3 {
                pos[i] = arg.pos[i] + dir[i] * (r_min + vu0.float01() * (r_max - r_min));
            }
            let speed = arg.speed + arg.speed_rand * vu0.float01();
            let spin = f32::from(arg.spin) * 0.1;
            let spin = if vu0.float01() > 0.5 { -spin } else { spin };
            self.tasks[t].parts.push(Part {
                pos,
                dir,
                color: [128.0; 4],
                vel: scale3(dir, speed),
                size: [1.0; 2],
                grow: [arg.grow; 2],
                accel: scale3(arg.accel, k),
                col_a: arg.col_a,
                col_b: arg.col_b,
                life: arg.life,
                life_max: arg.life,
                fade_len: arg.fade,
                fade: arg.fade,
                scale: arg.size,
                spin,
                speed,
                drag: arg.drag,
                alpha: 1.0,
                grow_damp: 1.0,
                ..Part::default()
            });
        }
    }

    /// `EftGndDust_SpawnPiece`: a particle of the task's argument (two VU0 draws: the speed's random part and
    /// the spin's side); none when the pool is empty.
    fn spawn_piece(&mut self, t: usize, flags: u32, size: f32, rot: f32, vu0: &mut Vu0Random) {
        let pool = self.tasks[t].arg.pool;
        if self.free[pool] == 0 {
            return;
        }
        self.free[pool] -= 1;
        let a = self.tasks[t].arg;
        let dir = norm(a.dir);
        let speed = (a.speed + a.speed_rand * vu0.float01()) * a.scale;
        let spin = f32::from(a.spin) * 0.1;
        let spin = if vu0.float01() > 0.5 { -spin } else { spin };
        self.tasks[t].parts.push(Part {
            pos: a.pos,
            dir,
            color: [128.0; 4],
            vel: scale3(dir, speed),
            size: [size; 2],
            grow: [a.grow; 2],
            accel: scale3(a.accel, a.gravity * a.scale + a.speed_rand),
            col_a: a.col_a,
            col_b: a.col_b,
            life: a.life,
            life_max: a.life,
            fade_len: a.fade,
            fade: a.fade,
            scale: a.size,
            angle: rot,
            spin,
            speed,
            drag: a.drag,
            alpha: 1.0,
            grow_damp: 1.0,
            flags: flags | 2,
            ..Part::default()
        });
    }

    /// `EftGndDust_SpawnPieceEx`: a particle of explicit values (one VU0 draw: the spin's side).
    #[allow(clippy::too_many_arguments)]
    fn spawn_piece_ex(
        &mut self,
        t: usize,
        pos: V3,
        dir: V3,
        col: ([u8; 4], [u8; 4]),
        size: [f32; 2],
        grow: [f32; 2],
        speed: f32,
        drag: f32,
        rot: f32,
        spin: f32,
        life: i16,
        fade: i16,
        scale: i16,
        grow_damp: f32,
        gravity: f32,
        flags: u32,
        vu0: &mut Vu0Random,
    ) {
        let pool = self.tasks[t].arg.pool;
        if self.free[pool] == 0 {
            return;
        }
        self.free[pool] -= 1;
        let dir = norm(dir);
        let spin = if vu0.float01() > 0.5 { -spin } else { spin };
        self.tasks[t].parts.push(Part {
            pos,
            dir,
            color: [128.0; 4],
            vel: scale3(dir, speed),
            size,
            grow,
            accel: [0.0, -gravity, 0.0],
            col_a: col.0,
            col_b: col.1,
            life,
            life_max: life,
            fade_len: fade,
            fade,
            scale,
            angle: rot,
            spin,
            speed,
            drag,
            alpha: 1.0,
            grow_damp,
            flags: flags | 2,
            ..Part::default()
        });
    }

    /// The fighters' emitter argument: the light colours, pool 1, texture 0, the fighter's scale.
    fn emitter_arg(&self, side: usize, c: &DustChr) -> DustArg {
        let (col_a, col_b) = self.light_colors();
        DustArg {
            pos: [0.0; 3],
            dir: [0.0, -1.0, 0.0],
            col_a,
            col_b,
            chr: side as i32,
            pool: 1,
            scale: c.scale,
            ..DustArg::default()
        }
    }

    /// `EftGndDust_SetSlide` (request 0x30): the fighter's slide emitter on or off, made the first time.
    pub(crate) fn set_slide(&mut self, side: usize, on: bool, c: &DustChr, vu0: &mut Vu0Random) {
        if self.index(self.slide[side]).is_none() {
            let arg = DustArg {
                size: 50,
                life: 15,
                fade: 10,
                grow: 0.1,
                bright: 1.0,
                speed: 1.1,
                drag: 1.0,
                ..self.emitter_arg(side, c)
            };
            let t = self.push(Kind::Slide, arg);
            self.slide[side] = Some(self.tasks[t].id);
            // EftGndDustSlide_Init.
            self.spawn_piece(t, PART_FOLLOW | PART_TURN, 1.0, 0.0, vu0);
            self.tasks[t].follow(c);
        }
        let t = self.index(self.slide[side]).unwrap();
        let w = &mut self.tasks[t];
        if on {
            w.flags = (w.flags | EMIT) & !FORCE_DROP;
        } else {
            w.flags = (w.flags & !EMIT) | FORCE_DROP;
        }
    }

    /// `EftGndDust_SetDash` (request 0x33): the fighter's dash emitter on or off, made the first time.
    pub(crate) fn set_dash(
        &mut self,
        side: usize,
        on: bool,
        c: &DustChr,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        if self.index(self.dash[side]).is_none() {
            let arg = DustArg {
                accel: [0.0, -1.0, 0.0],
                size: 100,
                life: 25,
                fade: 10,
                spin: 2,
                grow: 0.05,
                bright: 1.0,
                speed: 1.1,
                speed_rand: 0.01,
                drag: 1.0,
                gravity: 0.005,
                ..self.emitter_arg(side, c)
            };
            let t = self.push(Kind::Dash, arg);
            self.dash[side] = Some(self.tasks[t].id);
            // EftGndDustDash_Init: the direction against the fighter's, a piece at a random angle.
            self.tasks[t].follow(c);
            self.tasks[t].arg.dir = self.tasks[t].dir;
            let rot = randf(libc) * 3.141_592_7;
            self.spawn_piece(t, PART_FOLLOW, 1.0, rot, vu0);
        }
        let t = self.index(self.dash[side]).unwrap();
        let w = &mut self.tasks[t];
        if on {
            w.flags |= EMIT;
        } else {
            w.flags &= !EMIT;
        }
    }

    /// `EftGndDust_SpawnBurst` (request 0x34, or 0x35 under water): dust rising from the body
    /// (`EftGndDust_SpawnBodyDust`), at most the fighter's scale 1.
    pub(crate) fn spawn_burst(
        &mut self,
        side: usize,
        c: &DustChr,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        let arg = DustArg {
            accel: [0.0, -1.0, 0.0],
            size: 20,
            life: 15,
            fade: 10,
            spin: 2,
            bright: 1.0,
            scale: c.scale.min(1.0),
            ..self.emitter_arg(side, c)
        };
        let t = self.push(Kind::Burst, arg);
        // EftGndDust_SpawnBodyDust: pieces along ten segments, one per quarter of the height (6 at least), each
        // flying away from node 3. Six libc draws a piece, two of them unused, and the colours' one.
        const SEG: [(usize, i32, bool); 10] = [
            (3, -1, false),
            (0xB, 8, false),
            (0xF, 0xC, false),
            (0x15, -1, false),
            (0x23, -1, false),
            (0x12, 0x15, true),
            (0x20, 0x23, true),
            (8, 0x12, true),
            (0xC, 0x20, true),
            (0x30, -1, false),
        ];
        let h = (c.height * 0.25).max(6.0);
        let center = c.nodes[3];
        for (node, end, skip_short) in SEG {
            let start = c.nodes[node];
            let (step, n) = match usize::try_from(end) {
                Ok(end) => {
                    let delta = sub(c.nodes[end], start);
                    let len = dot(delta, delta).sqrt();
                    if h < len {
                        let n = (len / h + 1.0) as i32;
                        (scale3(delta, 1.0 / n as f32), n)
                    } else {
                        ([0.0; 3], if skip_short { 0 } else { 1 })
                    }
                }
                Err(_) => ([0.0; 3], 1),
            };
            for j in 0..n {
                libc.rand();
                let extra = randf(libc) * 1.5 + h;
                libc.rand();
                let life = randf(libc) * 0.5 + 0.7;
                let pos = add(start, scale3(step, j as f32));
                let dir = norm(sub(pos, center));
                libc.rand();
                let a = &mut self.tasks[t].arg;
                a.pos = pos;
                a.dir = dir;
                a.accel = [0.0, -1.0, 0.0];
                a.life = (life * 30.0) as i16;
                a.fade = 5;
                a.grow = 0.1;
                a.drag = 0.8;
                a.gravity = 0.01;
                a.spin = 0;
                a.speed = 1.5;
                a.speed_rand = 0.0;
                let rot = randf(libc) * 6.283_185_3;
                self.spawn_piece(t, 0, extra, rot, vu0);
            }
        }
    }

    /// The landing tasks alive.
    #[cfg(test)]
    pub(crate) fn lands(&self) -> usize {
        self.tasks.iter().filter(|t| t.kind == Kind::Land).count()
    }

    /// `EftGndDust_SpawnLanding` (request 0x35, scale 1) and `EftGndDust_SpawnLandingScaled` (a connected shot
    /// technique's victim): three low puffs round the point, eight particles thrown outwards and one in the
    /// middle.
    pub(crate) fn spawn_landing(
        &mut self,
        side: usize,
        pos: V3,
        scale: f32,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
        coef: &[f32; 4],
    ) {
        let (col_a, col_b) = self.light_colors();
        let arg = DustArg {
            pos,
            dir: [0.0, -1.0, 0.0],
            accel: [0.0, -1.0, 0.0],
            col_a,
            col_b,
            chr: side as i32,
            pool: 1,
            tex: 2,
            size: 10,
            life: 30,
            fade: 10,
            grow: 0.1,
            bright: 1.0,
            drag: 1.0,
            scale,
            ..DustArg::default()
        };
        let t = self.push(Kind::Land, arg);
        let col = (col_a, col_b);
        let ring = |start: f32, step: f32, i: usize| {
            let ang = crate::btl_tech_volley::eft_wrap(start + step * i as f32);
            norm([
                crate::btl_tech_volley::sin_fast(coef, ang),
                0.0,
                crate::btl_tech_volley::cos_fast(coef, ang),
            ])
        };
        let start = randf(libc) * 6.283_185_3;
        for i in 0..3 {
            let dir = ring(start, 2.094_395_1, i);
            let p = add(pos, scale3(dir, 2.0));
            // Vec3_Lerp: the owner's direction * t + this one * (1 - t).
            let k = randf(libc) * 0.2 + 0.6;
            let v: V3 = std::array::from_fn(|n| arg.dir[n] * k + dir[n] * (1.0 - k));
            self.spawn_piece_ex(
                t,
                p,
                v,
                col,
                [5.0, 6.0],
                [0.0, 1.0],
                0.0,
                1.0,
                0.0,
                0.0,
                arg.life,
                arg.fade,
                10,
                0.7,
                0.0,
                PART_FAR,
                vu0,
            );
        }
        let start = randf(libc) * 6.283_185_3;
        let (mut life, mut fade) = (arg.life, arg.fade);
        for i in 0..8 {
            let dir = ring(start, 0.785_398_16, i);
            let mut p = add(pos, scale3(dir, 2.0));
            p[1] -= 3.0;
            life = arg.life + (libc.rand() % 10) as i16;
            fade = arg.fade + (libc.rand() % 5) as i16;
            let rot = randf(libc) * 6.283_185_3;
            let gravity = (randf(libc) * 0.01 + 0.07) * arg.scale;
            self.spawn_piece_ex(
                t,
                p,
                dir,
                col,
                [1.0; 2],
                [0.0; 2],
                arg.scale * 7.8,
                0.8,
                rot,
                0.3,
                life,
                fade,
                350,
                arg.scale * 0.7,
                gravity,
                0,
                vu0,
            );
        }
        let rot = randf(libc) * 6.283_185_3;
        let gravity = (randf(libc) * 0.01 + 0.03) * arg.scale;
        self.spawn_piece_ex(
            t,
            pos,
            arg.dir,
            col,
            [1.0; 2],
            [0.0; 2],
            arg.scale * 0.8,
            0.8,
            rot,
            0.3,
            life,
            fade,
            350,
            arg.scale * 0.7,
            gravity,
            0,
            vu0,
        );
    }

    /// `EftGndDust_SpawnImpact` (request 0x32 at each foot, a projectile's impact): two small puffs thrown out
    /// from the point.
    pub(crate) fn spawn_impact(
        &mut self,
        side: usize,
        pos: V3,
        scale: f32,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
        coef: &[f32; 4],
    ) {
        let (col_a, col_b) = self.light_colors();
        let arg = DustArg {
            pos,
            dir: [0.0, -1.0, 0.0],
            accel: [0.0, -1.0, 0.0],
            col_a,
            col_b,
            chr: side as i32,
            pool: 1,
            tex: 2,
            size: 10,
            life: 15,
            fade: 5,
            grow: 0.1,
            bright: 1.0,
            drag: 1.0,
            scale: scale.max(0.8),
            ..DustArg::default()
        };
        let t = self.push(Kind::Impact, arg);
        let start = randf(libc) * 6.283_185_3;
        for i in 0..2 {
            let ang = crate::btl_tech_volley::eft_wrap(start + 2.094_395_1 * i as f32);
            let dir = norm([
                crate::btl_tech_volley::sin_fast(coef, ang),
                0.0,
                crate::btl_tech_volley::cos_fast(coef, ang),
            ]);
            let mut p = add(pos, scale3(dir, 2.0));
            p[1] -= randf(libc) * 3.0;
            let life = arg.life + (libc.rand() % 5) as i16;
            let fade = arg.fade + (libc.rand() % 5) as i16;
            let rot = randf(libc) * 6.283_185_3;
            let gravity = (randf(libc) * 0.005 + 0.005) * arg.scale;
            let size = (libc.rand() % 10 + 25) as i16;
            self.spawn_piece_ex(
                t,
                p,
                dir,
                (col_a, col_b),
                [1.0; 2],
                [0.05; 2],
                arg.scale * 0.3,
                0.8,
                rot,
                0.3,
                life,
                fade,
                size,
                1.0,
                gravity,
                0,
                vu0,
            );
        }
    }

    /// The tasks' updates (not while time is stopped): the emitters follow their fighter and emit; every task
    /// steps its particles; one without particles ends (the slide and dash emitters last).
    pub(crate) fn update(
        &mut self,
        time_stopped: bool,
        chars: Option<&[DustChr; 2]>,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        let mut k = 0;
        while k < self.tasks.len() {
            if !time_stopped {
                let kind = self.tasks[k].kind;
                let owner = usize::try_from(self.tasks[k].arg.chr).ok();
                if let (Some(chars), Some(side)) = (chars, owner.filter(|&s| s < 2))
                    && kind != Kind::Puff
                {
                    let c = chars[side];
                    self.tasks[k].follow(&c);
                    if matches!(kind, Kind::Slide | Kind::Dash) {
                        self.tasks[k].arg.scale = c.scale.clamp(0.8, 2.3);
                        if self.tasks[k].flags & EMIT != 0 {
                            self.tasks[k].tick += 1;
                            let tick = self.tasks[k].tick;
                            if kind == Kind::Slide && tick % 5 == 0 {
                                self.spawn_piece(k, PART_FOLLOW | PART_TURN, 1.0, 0.0, vu0);
                                self.spawn_chip(k, libc, vu0);
                            }
                            if kind == Kind::Dash && tick % 6 == 0 {
                                self.tasks[k].arg.dir = self.tasks[k].dir;
                                let rot = randf(libc) * 3.141_592_7;
                                self.spawn_piece(k, PART_FOLLOW, 1.0, rot, vu0);
                            }
                        }
                        let w = &mut self.tasks[k];
                        let mov = norm(c.moved);
                        let force = kind == Kind::Slide && w.flags & FORCE_DROP != 0;
                        if dot(w.mov, mov) < 0.0 || force {
                            w.flags |= DROP;
                            w.part_flags = PART_FOLLOW;
                        } else {
                            w.flags &= !DROP;
                        }
                        w.mov = mov;
                    }
                }
                let w = &mut self.tasks[k];
                let before = w.parts.len();
                let (pos, dir, flags, part_flags) = (w.pos, w.dir, w.flags, w.part_flags);
                w.parts.retain_mut(|p| step(p, pos, dir, flags, part_flags));
                self.free[w.arg.pool] += before - w.parts.len();
            }
            let w = &self.tasks[k];
            if w.parts.is_empty() && !matches!(w.kind, Kind::Slide | Kind::Dash) {
                self.tasks.remove(k);
            } else {
                k += 1;
            }
        }
    }

    /// `EftGndDust_SpawnChip`: a piece 4 to 9 along the flattened direction and 7 down, half way up (one libc
    /// draw).
    fn spawn_chip(&mut self, t: usize, libc: &mut LibcRandom, vu0: &mut Vu0Random) {
        let w = &self.tasks[t];
        let mut d = norm(w.dir);
        d[1] = 0.0;
        let off = scale3(d, randf(libc) * 5.0 + 4.0);
        // The emitter passes its argument as the position: its own (0, 0, 0), the base follows the fighter.
        let mut p = add(w.arg.pos, off);
        p[1] += 7.0;
        let up = [0.0, -1.0, 0.0];
        let q: V3 = std::array::from_fn(|n| up[n] * 0.5 + d[n] * 0.5);
        let col = (w.arg.col_a, w.arg.col_b);
        self.spawn_piece_ex(
            t,
            p,
            q,
            col,
            [0.8, 1.0],
            [0.0, 0.2],
            0.0,
            1.0,
            0.0,
            0.0,
            5,
            10,
            10,
            1.0,
            0.0,
            PART_FOLLOW | PART_FAR,
            vu0,
        );
    }

    /// The tasks' draws: the puffs and the dash as sprites, the others' pieces as sprites or, the far ones,
    /// quads lying along their direction turned to the camera (not during the stage change's transition);
    /// the landing ring's low puffs fade in from 80 to 150 away from the camera.
    pub(crate) fn draw(
        &self,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
        textures: &[usize],
        burst_busy: bool,
    ) {
        let tex = |i: usize| textures.get(i).copied();
        for t in &self.tasks {
            let a = &t.arg;
            for p in &t.parts {
                let far = p.flags & PART_FAR != 0;
                let mut pos = add(p.pos, p.base);
                let mut fade = 1.0;
                if t.kind == Kind::Land && far {
                    let d = sub(pos, ctx.eye);
                    let dist = dot(d, d);
                    fade = (dist - 6400.0) / 16100.0;
                    if 22500.0 < dist {
                        fade = 1.0;
                    }
                    if dist < 6400.0 {
                        fade = 0.0;
                    }
                }
                if t.kind == Kind::Impact {
                    pos = p.pos;
                }
                let b = a.bright;
                let color = [
                    p.color[0] * b,
                    p.color[1] * b,
                    p.color[2] * b,
                    p.color[3] * p.alpha * fade,
                ];
                let s = f32::from(p.scale) * a.scale;
                let size = [s * p.size[0], s * p.size[1]];
                match t.kind {
                    Kind::Puff | Kind::Dash => draw_sprite(
                        ot,
                        ctx,
                        pos,
                        color,
                        size,
                        [0.0, 0.0, 1.0, 1.0],
                        p.angle,
                        a.blend,
                        false,
                        tex(a.tex),
                    ),
                    _ => {
                        let (texture, dir) = match t.kind {
                            Kind::Slide | Kind::Land => (tex(if far { 2 } else { 0 }), p.dir),
                            Kind::Burst => (tex(a.tex), [0.0; 3]),
                            _ => (tex(0), p.dir),
                        };
                        if far {
                            if !burst_busy {
                                draw_along(ot, ctx, pos, color, dir, size, a.blend, texture);
                            }
                        } else {
                            draw_sprite(
                                ot,
                                ctx,
                                pos,
                                color,
                                size,
                                [0.0, 0.0, 1.0, 1.0],
                                p.angle,
                                a.blend,
                                false,
                                texture,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// `EftGndDust_UpdateParts` for one particle; false once it is done.
fn step(p: &mut Part, pos: V3, dir: V3, flags: u32, part_flags: u32) -> bool {
    p.life -= 1;
    if p.life <= 0 {
        p.life = 0;
        p.flags |= PART_FADING;
    }
    if p.flags & PART_FOLLOW != 0 {
        p.base = pos;
    }
    if p.flags & PART_TURN != 0 {
        p.dir = dir;
        p.vel = scale3(p.dir, p.speed);
    }
    // EftGndDust_LerpColor: from colA at birth to colB.
    let t = f32::from(p.life) / f32::from(p.life_max);
    for i in 0..4 {
        p.color[i] = f32::from(p.col_a[i]) * t + f32::from(p.col_b[i]) * (1.0 - t);
    }
    for i in 0..2 {
        p.size[i] += p.grow[i];
        p.grow[i] *= p.grow_damp;
    }
    p.angle = wrap_angle(p.angle + p.spin * std::f32::consts::PI / 180.0);
    p.vel = add(scale3(p.vel, p.drag), p.accel);
    p.pos = add(p.pos, p.vel);
    if flags & DROP != 0 && part_flags & PART_FOLLOW != 0 {
        p.flags &= !PART_FOLLOW;
        p.pos = add(p.pos, p.base);
        p.base = [0.0; 3];
    }
    if p.flags & PART_FADING != 0 {
        if p.fade > 0 {
            p.fade -= 1;
            p.alpha = f32::from(p.fade) / f32::from(p.fade_len);
            if p.fade <= 0 {
                p.alpha = 0.0;
                p.flags |= PART_DONE;
            }
        } else {
            p.flags |= PART_DONE;
        }
    }
    p.flags & PART_DONE == 0
}

/// `EftGndDust_DrawPiece` along its direction (`EftGndDust_MakeFacingMtx`, `EftGndDust_DrawQuad`): a quad `w`
/// wide from the piece to `h` along the direction pulled half way to the camera's axis; none without alpha or
/// with a corner off the GS area.
#[allow(clippy::too_many_arguments)]
fn draw_along(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    pos: V3,
    color: V4,
    dir: V3,
    size: [f32; 2],
    layer: u8,
    texture: Option<usize>,
) {
    if color[3] <= 0.0 {
        return;
    }
    let v = &ctx.world2view;
    let cam = [v[0][2], v[1][2], v[2][2]];
    let d = dot(cam, dir);
    let (a, s) = if d < 0.0 {
        (scale3(cam, -1.0), -d * 0.5)
    } else {
        (cam, d * 0.5)
    };
    let a = scale3(norm(sub(dir, a)), s);
    let z = norm(add(dir, a));
    let x = norm(cross(cam, z));
    let y = norm(cross(x, z));
    let hw = size[0] * 0.5;
    let corners = [
        [-hw, 0.0, size[1]],
        [hw, 0.0, size[1]],
        [-hw, 0.0, 0.0],
        [hw, 0.0, 0.0],
    ];
    let world = corners.map(|c| {
        add(
            add(add(scale3(x, c[0]), scale3(y, c[1])), scale3(z, c[2])),
            pos,
        )
    });
    let rgba = color.map(|c| c as i32 as u8);
    crate::eft::queue_projected_quad(ot, ctx, &world, rgba, texture, layer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;

    const COEF: [f32; 4] = [2.6e-6, -1.98e-4, 8.33e-3, -0.166_666_7];

    fn chr(pos: V3) -> DustChr {
        DustChr {
            pos,
            dir: [0.0, 0.0, 1.0],
            moved: [0.0, 0.0, 1.0],
            height: 20.0,
            scale: 20.0 / 19.35,
            nodes: std::array::from_fn(|i| [pos[0], pos[1] - i as f32, pos[2]]),
        }
    }

    #[test]
    fn a_puff_lives_its_life_and_fade_then_frees_its_particles() {
        let mut d = GndDust::new(None, None);
        let (mut libc, mut vu0) = (LibcRandom::new(1), Vu0Random::default());
        d.spawn_debris([0.0; 3], 0.7, &mut vu0);
        assert_eq!(d.count(), 4);
        assert_eq!(d.free[0], 60);
        // Each particle starts between the radii (17..40) of the centre.
        for p in &d.tasks[0].parts {
            let r = (p.pos[0].powi(2) + p.pos[1].powi(2) + p.pos[2].powi(2)).sqrt();
            assert!((16.9..40.1).contains(&r), "{r}");
        }
        // Life 30, then the fade: the 30th update starts it (29 left), the 59th ends it.
        for _ in 0..58 {
            d.update(false, None, &mut libc, &mut vu0);
        }
        assert_eq!(d.count(), 4);
        d.update(false, None, &mut libc, &mut vu0);
        assert_eq!(d.count(), 0);
        assert_eq!(d.free[0], 64);
        // Time stopped holds them.
        d.spawn_debris([0.0; 3], 1.0, &mut vu0);
        for _ in 0..100 {
            d.update(true, None, &mut libc, &mut vu0);
        }
        assert_eq!(d.count(), 4);
        d.reset();
        assert_eq!((d.count(), d.free[0]), (0, 64));
    }

    #[test]
    fn a_puff_draws_a_sprite_per_particle_in_front_of_the_camera() {
        let mut d = GndDust::new(None, None);
        let mut vu0 = Vu0Random::default();
        d.spawn_debris([0.0, 0.0, 500.0], 1.0, &mut vu0);
        let view = View::new([0.0; 3], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = OrderTable::new();
        d.draw(&mut ot, &ctx, &[7], false);
        let tris = ot.into_list();
        assert_eq!(tris.len(), 8);
        assert!(tris.iter().all(|t| t.texture == Some(7)));
    }

    #[test]
    fn the_pool_limits_the_puffs() {
        let mut d = GndDust::new(None, None);
        let mut vu0 = Vu0Random::default();
        for _ in 0..17 {
            d.spawn_debris([0.0; 3], 1.0, &mut vu0);
        }
        assert_eq!(d.count(), 64);
        assert!(d.tasks.last().is_some_and(|t| t.parts.is_empty()));
    }

    #[test]
    fn the_slide_follows_its_fighter_and_emits_while_on() {
        let mut d = GndDust::new(None, Some([90, 80, 70, 60, 50, 40, 30, 20]));
        let (mut libc, mut vu0) = (LibcRandom::new(3), Vu0Random::default());
        let mut c = chr([0.0, 0.0, 300.0]);
        d.set_slide(0, true, &c, &mut vu0);
        // One piece at the start, colours from the stage's light colours (alpha + 8).
        assert_eq!(d.count(), 1);
        assert_eq!(d.tasks[0].parts[0].col_a, [90, 80, 70, 68]);
        let chars = [c, c];
        for _ in 0..10 {
            d.update(false, Some(&chars), &mut libc, &mut vu0);
        }
        // Every 5th frame a piece and a chip.
        assert_eq!(d.count(), 5);
        // Following: the base is the fighter's position.
        assert_eq!(d.tasks[0].parts[1].base, c.pos);
        // Off: no more pieces; the emitter stays, and the pieces drop off where they are.
        d.set_slide(0, false, &c, &mut vu0);
        c.pos[2] += 50.0;
        let chars = [c, c];
        for _ in 0..40 {
            d.update(false, Some(&chars), &mut libc, &mut vu0);
        }
        assert_eq!(d.count(), 0);
        assert_eq!(d.tasks.len(), 1);
        d.reset();
        assert!(d.slide[0].is_none());
    }

    #[test]
    fn the_landing_and_the_burst_end_with_their_last_piece_and_draw() {
        let mut d = GndDust::new(None, None);
        let (mut libc, mut vu0) = (LibcRandom::new(5), Vu0Random::default());
        let c = chr([0.0, 0.0, 300.0]);
        d.spawn_landing(0, [0.0, 20.0, 300.0], 1.0, &mut libc, &mut vu0, &COEF);
        // 3 low puffs, 8 thrown, 1 in the middle.
        assert_eq!(d.count(), 12);
        d.spawn_burst(0, &c, &mut libc, &mut vu0);
        d.spawn_impact(0, [0.0, 20.0, 300.0], 1.0, &mut libc, &mut vu0, &COEF);
        assert!(d.count() > 14);
        let view = View::new([0.0, -50.0, 0.0], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = OrderTable::new();
        d.draw(&mut ot, &ctx, &[1, 2, 3], false);
        assert!(!ot.into_list().is_empty());
        let chars = [c, c];
        for _ in 0..80 {
            d.update(false, Some(&chars), &mut libc, &mut vu0);
        }
        assert_eq!(d.count(), 0);
        assert!(d.tasks.is_empty());
        assert_eq!(d.free, [64, 63]);
    }
}
