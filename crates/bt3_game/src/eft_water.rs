//! The water effects (`EftWater_*`, src/battle/eft_water.c; the stage effect kind 7 of the scene's layer 0, on a
//! stage whose first water record has bit 0 and whose pack has entry 0x12): splashes where a fighter or a hit
//! record crosses the surface, the spray trails of techniques skimming it and the wakes of fighters moving fast in
//! it, all drawn from three particle pools (60 billboard drops, 30 surface rings, 30 sprays) with the stage pack's
//! textures. Nothing happens in split screen.
//!
//! The swirls (`EftWaterMist_*`) are not ported: only `EftWater_AddBurst` makes them, and the executable holds no
//! call of it nor its address.

use crate::battle::{IDENTITY, Mtx, apply, inverse_rt};
use crate::btl_tech_volley::{cos_fast, eft_wrap, sin_fast};
use crate::eft::{
    ClipVtx, DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, V3, V4,
    add, clip_plane, cross, depth_slot, dot, mtx_mul, norm, point, rotate_xyz, scale3, sub, xyz,
};
use crate::rng::LibcRandom;

/// `EFT_WATER_*`: a particle's state.
const WAIT: u32 = 0x1;
const LIVE: u32 = 0x2;
const FADE_IN: u32 = 0x4;
const FADE_OUT: u32 = 0x8;
const ENDING: u32 = 0x40;
const DEAD: u32 = 0x80;
const RISE: u32 = 0x100;

/// The pools' sizes.
const SPLASHES: usize = 10;
const TRAILS: usize = 15;
const DROPS: usize = 60;
const RINGS: usize = 30;
const SPRAYS: usize = 30;

/// `EFT_WATER_SPLASH_*`: what crossed the surface.
pub(crate) const SPLASH_CHAR: u8 = 0;
pub(crate) const SPLASH_BLAST: u8 = 1;
pub(crate) const SPLASH_TECH: u8 = 2;

/// The game runs the particles at 30 steps a second.
const FPS: f32 = 30.0;
const PI_F: f32 = 3.141_592_7;

/// `EFT_RANDF`: libc `rand()` scaled to 0..1.
fn randf(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// A fighter as the water reads it (`BtlCharApi_GetPos` / `GetDir` / `GetSpeed` / `GetHeight` /
/// `IsInTechnique`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WaterChr {
    pub pos: V3,
    pub dir: V3,
    pub speed: f32,
    pub height: f32,
    pub in_technique: bool,
}

/// A hit record as `EftWater_UpdateBlast` reads it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaterRec {
    pub owner: usize,
    /// `type`: a technique's record (`src` set) rather than a ki blast's.
    pub technique: bool,
    /// `src->def->sub` and `id` of a technique's record.
    pub def_sub: i32,
    pub def_id: i32,
    pub pos: V3,
    pub prev: V3,
    /// `flags & 8`: no water effect.
    pub no_water: bool,
    /// The owning task's flags (`EFT_TASK_*`) and hit position.
    pub task_flags: u32,
    pub task_pos: V3,
    /// The third word of the record's shape: the shot's speed.
    pub speed: f32,
    /// `EftHit_GetRadiusA`.
    pub radius_a: f32,
}

/// `EFT_WATER_TASK_*`.
const TASK_HIT_CHAR: u32 = 0x1;
const TASK_HIT_STAGE: u32 = 0x4;
pub(crate) const TASK_DONE: u32 = 0x400;

/// `EftWaterDrop`: a camera-facing billboard flying from `base` (+ the list's origin when drawn).
#[derive(Clone, Copy, Debug, Default)]
struct Drop {
    pos: V3,
    base: V3,
    vel: V3,
    rot: f32,
    size: f32,
    size_vel: f32,
    size_damp: f32,
    gravity: f32,
    dt: f32,
    t: f32,
    delay: f32,
    alpha: f32,
    alpha_step: f32,
    rgb: [u8; 3],
    tex: u8,
    layer: u8,
    flags: u32,
}

/// `EftWaterRing`: a quad lying on the surface.
#[derive(Clone, Copy, Debug, Default)]
struct Ring {
    pos: V3,
    size: f32,
    size_vel: f32,
    size_damp: f32,
    rot: f32,
    delay: f32,
    alpha: f32,
    alpha_max: f32,
    alpha_step: f32,
    fade_time: f32,
    rgb: [u8; 3],
    tex: u8,
    layer: u8,
    flags: u32,
}

/// `EftWaterSpray`: a streak along its flight direction that dissolves by its alpha test reference.
#[derive(Clone, Copy, Debug, Default)]
struct Spray {
    pos: V3,
    base: V3,
    dir: V3,
    vel: V3,
    size: f32,
    size_vel: f32,
    size_damp: f32,
    tilt: f32,
    tilt_vel: f32,
    gravity: f32,
    near: f32,
    width: f32,
    dt: f32,
    t: f32,
    delay: f32,
    roll: f32,
    progress: f32,
    progress_max: f32,
    progress_step: f32,
    rgb: [u8; 3],
    tex: u8,
    layer: u8,
    flags: u32,
}

/// The three particle lists of a splash, a trail or the module itself.
#[derive(Clone, Debug, Default)]
struct Lists {
    drops: Vec<Drop>,
    sprays: Vec<Spray>,
    rings: Vec<Ring>,
}

impl Lists {
    fn is_empty(&self) -> bool {
        self.drops.is_empty() && self.sprays.is_empty() && self.rings.is_empty()
    }

    /// `EftWaterSplash_FadeOut` / `EftWaterTrail_FadeOut`'s lists: everything finishes within 0.1 s.
    fn fade_out(&mut self) {
        let frames = 0.1 * FPS;
        for d in &mut self.drops {
            if d.flags & ENDING == 0 {
                d.alpha_step = if d.flags & RISE != 0 {
                    (128.0 - d.alpha) / frames
                } else {
                    d.alpha / frames
                };
                d.flags |= ENDING;
            }
        }
        for r in &mut self.rings {
            if r.flags & ENDING == 0 {
                r.flags |= ENDING;
                r.alpha_step = r.alpha / frames;
            }
        }
        for s in &mut self.sprays {
            if s.flags & ENDING == 0 {
                s.flags |= ENDING;
                s.progress_step = (s.progress_max - s.progress) / frames;
            }
        }
    }
}

/// `EftWaterSplash` / `EftWaterTrail`: a slot of its pool and its lists. A splash's drops and sprays are
/// relative to `pos`.
#[derive(Clone, Debug, Default)]
struct Group {
    slot: usize,
    pos: V3,
    lists: Lists,
}

/// `EftWaterWake`.
#[derive(Clone, Copy, Debug, Default)]
struct Wake {
    on: bool,
    frame: u32,
}

/// The pools' use: the particles alive in every list.
#[derive(Clone, Copy, Debug, Default)]
struct Pools {
    drops: usize,
    rings: usize,
    sprays: usize,
}

/// `gEftDust` (the water state): the wakes, the splashes, the trails, the lists of no splash, the pools.
#[derive(Clone, Debug)]
pub(crate) struct Water {
    /// `BtlStage_GetWaterLevel`'s level and the stage (`BtlStage_GetId`).
    level: f32,
    stage: i32,
    /// `Battle_IsSplitScreen`.
    pub split: bool,
    /// `Mathf_SinFast`'s coefficients.
    sin_coef: [f32; 4],
    /// The stage pack entry 0x12's textures among the effect textures.
    textures: Vec<usize>,
    wakes: [Wake; 2],
    blast_wait: [u8; 2],
    /// The splash pool's slots in use and the splashes in creation order.
    splash_used: [bool; SPLASHES],
    splashes: Vec<Group>,
    /// The trail pool's flags (`LIVE`, or `ENDING` left on a freed slot) and the trails in creation order.
    trail_flags: [u32; TRAILS],
    trails: Vec<Group>,
    lists: Lists,
    pools: Pools,
}

impl Water {
    /// `EftWater_Init`: empty pools; `level` is the stage's water level (0 without water).
    pub(crate) fn new(level: f32, stage: i32, sin_coef: [f32; 4], textures: Vec<usize>) -> Self {
        Self {
            level,
            stage,
            split: false,
            sin_coef,
            textures,
            wakes: [Wake::default(); 2],
            blast_wait: [0; 2],
            splash_used: [false; SPLASHES],
            splashes: Vec::new(),
            trail_flags: [0; TRAILS],
            trails: Vec::new(),
            lists: Lists::default(),
            pools: Pools::default(),
        }
    }

    /// The particles alive (drops, rings, sprays).
    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        (self.pools.drops, self.pools.rings, self.pools.sprays)
    }

    /// The splashes and trails alive.
    #[cfg(test)]
    pub(crate) fn groups(&self) -> (usize, usize) {
        (self.splashes.len(), self.trails.len())
    }

    /// `EftWater_GetSurfaceY` + -1: where a splash sits.
    fn surface(&self) -> f32 {
        self.level - 1.0
    }

    fn sin(&self, a: f32) -> f32 {
        sin_fast(&self.sin_coef, a)
    }

    fn cos(&self, a: f32) -> f32 {
        cos_fast(&self.sin_coef, a)
    }

    /// `EftWater_SetWake`: starts (`on`) or stops the wake behind fighter `side`.
    pub(crate) fn set_wake(&mut self, side: usize, on: bool) {
        if self.split {
            return;
        }
        let Some(w) = self.wakes.get_mut(side) else {
            return;
        };
        if w.on != on {
            w.on = on;
            w.frame = 0;
        }
    }

    /// `EftWater_Reset`: both wakes stop.
    pub(crate) fn reset(&mut self) {
        for side in 0..2 {
            self.set_wake(side, false);
        }
    }

    /// `EftWater_AddSplashFor`: a splash at `pos` on the surface, sized by the square root of `speed` (of its
    /// magnitude, as the FPU takes it) and, for a fighter or a technique, by the fighter's height.
    pub(crate) fn add_splash_for(
        &mut self,
        height: f32,
        pos: V3,
        kind: u8,
        speed: f32,
        libc: &mut LibcRandom,
    ) {
        if self.split {
            return;
        }
        let at = [pos[0], self.surface(), pos[2]];
        let scale = speed.abs().sqrt() * 0.33;
        match kind {
            SPLASH_CHAR => self.add_splash(at, [7, 4, 3, 2], height * 0.05 * scale, libc),
            SPLASH_BLAST => self.add_splash(at, [4, 3, 2, 2], scale * 0.8, libc),
            SPLASH_TECH => self.add_splash(at, [7, 4, 3, 2], height * 0.05 * scale * 1.2, libc),
            _ => {}
        }
    }

    /// `EftWater_AddSplashAt`: a full splash at `pos` on the surface (a stage piece falling in).
    pub(crate) fn add_splash_at(&mut self, pos: V3, scale: f32, libc: &mut LibcRandom) {
        if self.split {
            return;
        }
        let at = [pos[0], self.surface(), pos[2]];
        self.add_splash(at, [7, 4, 3, 2], scale, libc);
    }

    /// `EftWater_AddSplash`: two rings of sprays, two of drops (each spread evenly from a random angle) and four
    /// surface rings, scaled by `scale`; nothing when the ten splashes are in use.
    fn add_splash(&mut self, pos: V3, counts: [usize; 4], scale: f32, libc: &mut LibcRandom) {
        let Some(slot) = self.splash_used.iter().position(|u| !u) else {
            return;
        };
        self.splash_used[slot] = true;
        let mut g = Group {
            slot,
            pos,
            lists: Lists::default(),
        };
        let [spray_a, spray_b, drop_a, drop_b] = counts;
        let height = 7.0 * scale;
        let origin = [0.0; 3];
        let yaw = randf(libc) * 360.0;
        let step = 360.0 / spray_a as f32;
        let spread = randf(libc) * 0.2;
        for i in 0..spray_a {
            let size_vel = randf(libc) * 2.4 + 5.6;
            let life = randf(libc) * 0.4 + 0.5;
            let pitch = randf(libc) * 12.5 + 62.5;
            let arg = SprayArg {
                dist: 0.0,
                height,
                yaw: yaw + step * i as f32,
                pitch,
                roll: 0.0,
                tilt: [0.8, 0.65],
                speed: 40.0 * scale,
                size: 0.0,
                size_vel: size_vel * scale,
                size_damp: -5.0,
                gravity: 6.4 * scale,
                near: spread + 1.0,
                width: 1.2,
                life,
                delay: 0.0,
                rgb: [0x8C; 3],
                tex: 3,
                layer: 0,
                flags: RISE | LIVE,
            };
            self.spawn_spray(&mut g.lists.sprays, origin, arg);
        }
        let yaw = randf(libc) * 360.0;
        let step = 360.0 / spray_b as f32;
        for i in 0..spray_b {
            let size_vel = randf(libc) * 4.0 + 6.0;
            let life = randf(libc) * 0.4 + 0.8;
            let pitch = randf(libc) * 12.5 + 72.5;
            let arg = SprayArg {
                dist: 2.0,
                height,
                yaw: yaw + step * i as f32,
                pitch,
                roll: 0.0,
                tilt: [0.9, 0.75],
                speed: 40.0 * scale,
                size: 0.0,
                size_vel: size_vel * scale,
                size_damp: -5.0,
                gravity: 6.4 * scale,
                near: spread + 1.2,
                width: 0.7,
                life,
                delay: 0.1,
                rgb: [0x8C; 3],
                tex: 3,
                layer: 0,
                flags: RISE | WAIT,
            };
            self.spawn_spray(&mut g.lists.sprays, origin, arg);
        }
        let yaw = randf(libc) * 360.0;
        let step = 360.0 / drop_a as f32;
        for i in 0..drop_a {
            let speed = randf(libc) * 40.0 + 110.0;
            let size_vel = randf(libc) * 4.0 + 8.0;
            let life = randf(libc) * 0.5 + 0.9;
            let pitch = randf(libc) * 10.0 + 72.0;
            let arg = DropArg {
                yaw: yaw + step * i as f32,
                pitch,
                speed: speed * scale,
                rot: randf(libc) * 360.0,
                size: 2.0 * scale,
                size_vel: size_vel * scale,
                gravity: 11.0 * scale,
                life,
                delay: 0.0,
                rgb: [0x80; 3],
                tex: 4,
                flags: RISE | FADE_IN | LIVE,
            };
            self.spawn_drop(&mut g.lists.drops, origin, arg);
        }
        let yaw = randf(libc) * 360.0;
        let step = 360.0 / drop_b as f32;
        for i in 0..drop_b {
            let size_vel = randf(libc) * 4.0 + 8.0;
            let life = randf(libc) * 0.6 + 1.4;
            let pitch = randf(libc) * 10.0 + 75.0;
            let arg = DropArg {
                yaw: yaw + step * i as f32,
                pitch,
                speed: 55.0 * scale,
                rot: randf(libc) * 360.0,
                size: 2.0 * scale,
                size_vel: size_vel * scale,
                gravity: 3.5 * scale,
                life,
                delay: 0.0,
                rgb: [0x80; 3],
                tex: 1,
                flags: FADE_OUT | LIVE,
            };
            self.spawn_drop(&mut g.lists.drops, origin, arg);
        }
        // The first two live at once (0xE), the others wait for their delay (0xD).
        let ring = |size, size_vel, size_damp, alpha_max, time, delay, c, tex, flags| RingArg {
            size,
            size_vel,
            size_damp,
            alpha_max,
            time,
            delay,
            rgb: [c; 3],
            tex,
            flags,
        };
        let (live, wait) = (FADE_IN | FADE_OUT | LIVE, FADE_IN | FADE_OUT | WAIT);
        for arg in [
            ring(
                scale * 5.0,
                scale * 12.0,
                -4.0,
                96.0,
                1.4,
                0.0,
                0x80,
                0,
                live,
            ),
            ring(
                scale * 8.0,
                scale * 6.0,
                -24.0,
                112.0,
                1.5,
                0.0,
                0xFF,
                2,
                live,
            ),
            ring(
                scale * 10.0,
                scale * 4.0,
                -24.0,
                96.0,
                1.2,
                0.9,
                0xFF,
                2,
                wait,
            ),
            ring(
                scale * 10.0,
                scale * 4.0,
                -24.0,
                88.0,
                1.2,
                0.5,
                0xFF,
                2,
                wait,
            ),
        ] {
            self.spawn_ring(&mut g.lists.rings, pos, arg);
        }
        self.splashes.push(g);
    }

    /// `EftWaterTrail_Spawn`: spray to both sides of a record's direction, two drops along it and a ring, sized by
    /// the fighter's height and the record's horizontal speed; nothing when the fifteen trails are in use.
    fn spawn_trail(&mut self, height: f32, pos: V3, prev: V3, libc: &mut LibcRandom) {
        let Some(slot) = self.trail_flags.iter().position(|&f| f == 0) else {
            return;
        };
        self.trail_flags[slot] = LIVE;
        let mut g = Group {
            slot,
            pos: [0.0; 3],
            lists: Lists::default(),
        };
        let at = [pos[0], self.surface(), pos[2]];
        let mut delta = sub(pos, prev);
        let dir = norm(delta);
        delta[1] = 0.0;
        let mut scale = dot(delta, delta).sqrt().sqrt() * 0.26;
        if scale < 0.9 {
            scale *= 0.65;
        }
        let k = height * 0.05 * scale;
        let yaw = dir[0].atan2(dir[2]) * 180.0 / PI_F;
        for i in 0..2 {
            let size = randf(libc) * 3.0 + 1.0;
            let size_vel = randf(libc) * 3.0 + 5.0;
            let life = randf(libc) * 0.2 + 0.3;
            let pitch = randf(libc) * 10.0 + 50.0;
            let arg = SprayArg {
                dist: 0.0,
                height: 7.0 * k,
                yaw: yaw - 60.0 + i as f32 * 120.0,
                pitch,
                roll: randf(libc) * 360.0,
                tilt: [0.8, 0.6],
                speed: 40.0 * k,
                size,
                size_vel: size_vel * k,
                size_damp: -5.0,
                gravity: 6.4 * k,
                near: 1.0,
                width: 1.5,
                life,
                delay: 0.0,
                rgb: [0x8C; 3],
                tex: 3,
                layer: 0,
                flags: RISE | LIVE,
            };
            self.spawn_spray(&mut g.lists.sprays, at, arg);
        }
        let size_vel = randf(libc) * 3.5 + 6.5;
        let life = randf(libc) * 0.2 + 0.4;
        let pitch = randf(libc) * 10.0 + 60.0;
        let drop_yaw = yaw - 30.0 + (libc.rand() % 2) as f32 * 60.0;
        let arg = DropArg {
            yaw: drop_yaw,
            pitch,
            speed: 90.0 * k,
            rot: randf(libc) * 360.0,
            size: 2.0 * k,
            size_vel: size_vel * k,
            gravity: 9.0 * k,
            life,
            delay: 0.0,
            rgb: [0x80; 3],
            tex: 4,
            flags: RISE | FADE_IN | LIVE,
        };
        self.spawn_drop(&mut g.lists.drops, at, arg);
        let size_vel = randf(libc) * 3.5 + 6.5;
        let life = randf(libc) * 0.3 + 0.6;
        let pitch = randf(libc) * 10.0 + 70.0;
        let arg = DropArg {
            yaw,
            pitch,
            speed: 55.0 * k,
            rot: randf(libc) * 360.0,
            size: 2.0 * k,
            size_vel: size_vel * k,
            gravity: 3.5 * k,
            life,
            delay: 0.0,
            rgb: [0x80; 3],
            tex: 1,
            flags: FADE_OUT | LIVE,
        };
        self.spawn_drop(&mut g.lists.drops, at, arg);
        let arg = RingArg {
            size: 0.0,
            size_vel: 10.0 * k,
            size_damp: -24.0,
            alpha_max: 112.0,
            time: 0.6,
            delay: 0.1,
            rgb: [0xFF; 3],
            tex: 2,
            flags: FADE_IN | FADE_OUT | WAIT,
        };
        self.spawn_ring(&mut g.lists.rings, at, arg);
        self.trails.push(g);
    }

    /// `EftWater_UpdateBlast` (`BtlScene_UpdateRecords`, after the post-updates): the splash where a record crossed
    /// the surface this frame (with sound 0x3C going in, 0x3D coming out for a ki blast; a technique of sub-kind
    /// 3 / 4 only every few crossings), or the trail of a technique skimming it. Returns the record's task flags
    /// to set (`TASK_DONE` once a fighter's hit position was used) and the sound to queue at the crossing.
    pub(crate) fn update_blast(
        &mut self,
        rec: &WaterRec,
        height: f32,
        paused: bool,
        libc: &mut LibcRandom,
    ) -> (u32, Option<(V3, i32)>) {
        if paused || rec.no_water || rec.task_flags & (TASK_HIT_STAGE | TASK_DONE) != 0 {
            return (0, None);
        }
        let mut level = self.level;
        if self.stage == 12 {
            level += 5.0;
        }
        let (mut pos, mut prev) = (rec.pos, rec.prev);
        let mut set = 0;
        if rec.task_flags & TASK_HIT_CHAR != 0 {
            let back = scale3(norm(sub(prev, pos)), rec.speed);
            prev = add(rec.task_pos, back);
            pos = rec.task_pos;
            set = TASK_DONE;
        }
        let dy = (pos[1] - prev[1]).abs().min(20.0);
        let scale = rec.radius_a / 5.0;
        let size = (dy * 0.8 + (libc.rand() as f32 / 2_147_483_647.0 * 3.0 - 1.5)) * scale;
        let down = pos[1] >= level && prev[1] <= level;
        let up = !down && pos[1] <= level && level <= prev[1];
        let mut sound = None;
        if down || up {
            let hit = self.clip_seg(pos, prev);
            if !rec.technique {
                self.add_splash_for(height, hit, SPLASH_BLAST, size, libc);
                sound = Some((hit, if down { 0x3C } else { 0x3D }));
            } else if (3..=4).contains(&rec.def_sub) {
                let wait = &mut self.blast_wait[usize::from(rec.owner != 0)];
                if *wait != 0 {
                    *wait -= 1;
                } else {
                    *wait = def_class(rec.def_sub, rec.def_id);
                    self.add_splash_for(height, hit, SPLASH_TECH, dy, libc);
                }
            } else {
                self.add_splash_for(height, hit, SPLASH_TECH, dy, libc);
            }
        } else if rec.technique {
            let band = if self.stage == 10 {
                pos[1] > level && pos[1] < level + 12.0
            } else {
                level - 6.0 < pos[1] && pos[1] < level + 12.0
            };
            if band && !self.split {
                self.spawn_trail(height, rec.pos, rec.prev, libc);
            }
        }
        (set, sound)
    }

    /// `EftUtil_ClipSegToWater`: the point of `a`-`b` at the surface's height.
    fn clip_seg(&self, a: V3, b: V3) -> V3 {
        // The plane (0, -1, 0, level) against a and the segment's direction.
        let w = self.level;
        let dir = sub(a, b);
        let da = -a[1];
        let dd = -dir[1];
        let t = 1.0 - (da + w) / dd;
        std::array::from_fn(|i| a[i] * t + b[i] * (1.0 - t))
    }

    /// `EftWater_Update`: the technique counters reset when the fighters left their techniques, then (not while
    /// paused) the wakes and every list.
    pub(crate) fn update(&mut self, chars: &[WaterChr; 2], paused: bool, libc: &mut LibcRandom) {
        for (w, c) in self.blast_wait.iter_mut().zip(chars) {
            if !c.in_technique {
                *w = 0;
            }
        }
        if paused {
            return;
        }
        self.update_wakes(chars, libc);
        let level = self.level;
        // The splashes, then the trails (the oldest fade so that at most five stay at full strength).
        let mut i = 0;
        while i < self.splashes.len() {
            update_lists(&mut self.splashes[i].lists, &mut self.pools, level);
            if self.splashes[i].lists.is_empty() {
                let g = self.splashes.remove(i);
                self.splash_used[g.slot] = false;
            } else {
                i += 1;
            }
        }
        let mut fading = 0;
        let mut i = 0;
        while i < self.trails.len() {
            update_lists(&mut self.trails[i].lists, &mut self.pools, level);
            let done = self.trails[i].lists.is_empty();
            let slot = self.trails[i].slot;
            if done {
                self.trail_flags[slot] = 0;
            }
            if self.trails.len() - usize::from(done) >= 6 + fading {
                // EftWaterTrail_FadeOut, once: on a trail just freed it leaves ENDING on its pool slot.
                if self.trail_flags[slot] & ENDING == 0 {
                    self.trails[i].lists.fade_out();
                    self.trail_flags[slot] |= ENDING;
                }
                fading += 1;
            }
            if done {
                self.trails.remove(i);
            } else {
                i += 1;
            }
        }
        update_lists(&mut self.lists, &mut self.pools, level);
    }

    /// `EftWater_UpdateWakes`: behind each fighter whose wake is on, sprays (two on even frames, one to a random
    /// side on odd ones), two drops on even frames and a ring every third frame, sized by its speed and height.
    fn update_wakes(&mut self, chars: &[WaterChr; 2], libc: &mut LibcRandom) {
        for n in 0..2 {
            if !self.wakes[n].on {
                continue;
            }
            let c = chars[n];
            let frame = self.wakes[n].frame;
            let pos = [c.pos[0], self.surface(), c.pos[2]];
            let mut delta = scale3(c.dir, c.speed);
            delta[1] = 0.0;
            let mut scale = dot(delta, delta).sqrt().sqrt() * 0.26;
            if scale < 0.9 {
                scale *= 0.65;
            }
            let k = c.height * 0.05 * scale;
            let yaw = c.dir[0].atan2(c.dir[2]) * 180.0 / PI_F;
            let base_yaw = yaw;
            let height = 7.0 * k;
            let mut lists = std::mem::take(&mut self.lists);
            let mut spray =
                |me: &mut Self, yaw: Option<f32>, tilt_end: f32, libc: &mut LibcRandom| {
                    let size = randf(libc) * 3.0 + 1.0;
                    let size_vel = randf(libc) * 3.0 + 5.0;
                    let life = randf(libc) * 0.2 + 0.3;
                    let pitch = randf(libc) * 10.0 + 50.0;
                    let yaw =
                        yaw.unwrap_or_else(|| base_yaw - 60.0 + (libc.rand() % 2) as f32 * 120.0);
                    let arg = SprayArg {
                        dist: 0.0,
                        height,
                        yaw,
                        pitch,
                        roll: randf(libc) * 360.0,
                        tilt: [0.8, tilt_end],
                        speed: 40.0 * k,
                        size,
                        size_vel: size_vel * k,
                        size_damp: -5.0,
                        gravity: 6.4 * k,
                        near: 1.0,
                        width: 1.5,
                        life,
                        delay: 0.0,
                        rgb: [0x8C; 3],
                        tex: 3,
                        layer: 0,
                        flags: RISE | LIVE,
                    };
                    me.spawn_spray(&mut lists.sprays, pos, arg);
                };
            if frame & 1 == 0 {
                for i in 0..2 {
                    spray(self, Some(yaw - 60.0 + i as f32 * 120.0), 0.6, libc);
                }
            } else {
                // The side is drawn (rand() % 2) after the pitch, before the roll.
                spray(self, None, 0.5, libc);
            }
            if frame & 1 == 0 {
                let size_vel = randf(libc) * 3.5 + 6.5;
                let life = randf(libc) * 0.2 + 0.4;
                let pitch = randf(libc) * 10.0 + 60.0;
                let puff_yaw = yaw - 30.0 + (libc.rand() % 2) as f32 * 60.0;
                let arg = DropArg {
                    yaw: puff_yaw,
                    pitch,
                    speed: 90.0 * k,
                    rot: randf(libc) * 360.0,
                    size: 2.0 * k,
                    size_vel: size_vel * k,
                    gravity: 9.0 * k,
                    life,
                    delay: 0.0,
                    rgb: [0x80; 3],
                    tex: 4,
                    flags: RISE | FADE_IN | LIVE,
                };
                self.spawn_drop(&mut lists.drops, pos, arg);
                let size_vel = randf(libc) * 3.5 + 6.5;
                let life = randf(libc) * 0.3 + 0.6;
                let pitch = randf(libc) * 10.0 + 70.0;
                let arg = DropArg {
                    yaw,
                    pitch,
                    speed: 55.0 * k,
                    rot: randf(libc) * 360.0,
                    size: 2.0 * k,
                    size_vel: size_vel * k,
                    gravity: 3.5 * k,
                    life,
                    delay: 0.0,
                    rgb: [0x80; 3],
                    tex: 1,
                    flags: FADE_OUT | LIVE,
                };
                self.spawn_drop(&mut lists.drops, pos, arg);
            }
            if frame % 3 == 0 {
                let arg = RingArg {
                    size: 0.0,
                    size_vel: k * 10.0,
                    size_damp: -24.0,
                    alpha_max: 112.0,
                    time: 0.6,
                    delay: 0.1,
                    rgb: [0xFF; 3],
                    tex: 2,
                    flags: FADE_IN | FADE_OUT | WAIT,
                };
                self.spawn_ring(&mut lists.rings, pos, arg);
            }
            self.lists = lists;
            self.wakes[n].frame += 1;
        }
    }

    /// `EftWaterDrop_Spawn`: launched from `pos` along yaw / pitch (degrees); with `RISE` its alpha (the alpha
    /// test's reference) counts up from 0, else it fades from 128, over `life` seconds.
    fn spawn_drop(&mut self, list: &mut Vec<Drop>, pos: V3, a: DropArg) {
        if self.pools.drops >= DROPS {
            return;
        }
        self.pools.drops += 1;
        let yaw = eft_wrap(a.yaw * PI_F / 180.0);
        let dir = norm([self.sin(yaw), 0.0, self.cos(yaw)]);
        let pitch = a.pitch * PI_F / 180.0;
        let (sp, cp) = (self.sin(pitch), self.cos(pitch));
        list.push(Drop {
            pos: [0.0; 3],
            base: pos,
            vel: [dir[0] * a.speed * cp, -a.speed * sp, dir[2] * a.speed * cp],
            rot: a.rot,
            size: a.size,
            size_vel: a.size_vel,
            size_damp: -1.0 / 5.0,
            gravity: a.gravity * 9.806_65,
            dt: 1.0 / FPS,
            t: 0.0,
            delay: a.delay * FPS,
            alpha: if a.flags & RISE != 0 { 0.0 } else { 128.0 },
            alpha_step: 128.0 / (a.life * FPS),
            rgb: a.rgb,
            tex: a.tex,
            layer: 0,
            flags: a.flags,
        });
    }

    /// `EftWaterRing_Spawn`: with `FADE_IN` it rises to `alpha_max` over 30% of `time` and falls over the rest.
    fn spawn_ring(&mut self, list: &mut Vec<Ring>, pos: V3, a: RingArg) {
        if self.pools.rings >= RINGS {
            return;
        }
        self.pools.rings += 1;
        let mut r = Ring {
            pos,
            size: a.size,
            size_vel: a.size_vel,
            size_damp: if a.size_damp == 0.0 {
                0.0
            } else {
                1.0 / a.size_damp
            },
            rot: 0.0,
            delay: a.delay * FPS,
            rgb: a.rgb,
            tex: a.tex,
            layer: 0,
            flags: a.flags,
            ..Ring::default()
        };
        if a.flags & FADE_IN != 0 {
            r.alpha_max = a.alpha_max;
            r.alpha = 0.0;
            r.fade_time = a.time * 0.7;
            r.alpha_step = a.alpha_max / (a.time * 0.3 * FPS);
        } else if a.flags & FADE_OUT != 0 {
            r.fade_time = a.time;
            r.alpha = a.alpha_max;
            r.alpha_max = 0.0;
            r.alpha_step = a.alpha_max / (a.time * FPS);
        }
        list.push(r);
    }

    /// `EftWaterSpray_Spawn`: starts `dist` out from `pos` along the yaw, `height` up, and flies along yaw /
    /// pitch; with `RISE` it dissolves (its alpha test reference counting to 96) over 1.6 lives.
    fn spawn_spray(&mut self, list: &mut Vec<Spray>, pos: V3, a: SprayArg) {
        if self.pools.sprays >= SPRAYS {
            return;
        }
        self.pools.sprays += 1;
        let yaw = eft_wrap(a.yaw * PI_F / 180.0);
        let dir = norm([self.sin(yaw), 0.0, self.cos(yaw)]);
        let pitch = a.pitch * PI_F / 180.0;
        let (sp, cp) = (self.sin(pitch), self.cos(pitch));
        let base = add([pos[0], pos[1] + a.height, pos[2]], scale3(dir, a.dist));
        let mut s = Spray {
            pos: [0.0; 3],
            base,
            dir,
            vel: [dir[0] * a.speed * cp, -a.speed * sp, dir[2] * a.speed * cp],
            size: a.size,
            size_vel: a.size_vel,
            size_damp: if a.size_damp != 0.0 {
                1.0 / a.size_damp
            } else {
                0.0
            },
            tilt: a.tilt[0],
            tilt_vel: (a.tilt[1] - a.tilt[0]) / (a.life * FPS),
            gravity: a.gravity * 9.806_65,
            near: a.near,
            width: a.width,
            dt: 1.0 / FPS,
            t: 0.0,
            delay: a.delay * FPS,
            roll: a.roll,
            rgb: a.rgb,
            tex: a.tex,
            layer: a.layer,
            flags: a.flags,
            ..Spray::default()
        };
        if a.flags & RISE != 0 {
            s.progress = 0.0;
            s.progress_max = 96.0;
            s.progress_step = 96.0 / (a.life * 1.6 * FPS);
        }
        list.push(s);
    }

    /// `EftWater_Draw`: the splashes (their drops and sprays around them), the trails, then the module's rings,
    /// sprays and drops.
    pub(crate) fn draw(&self, ot: &mut OrderTable, ctx: &DrawCtx) {
        let mut billboard = inverse_rt(&ctx.world2view);
        billboard[3] = [0.0, 0.0, 0.0, billboard[3][3]];
        let d = Drawer {
            ot: std::cell::RefCell::new(ot),
            ctx,
            billboard,
            // EftUtil_IsCamUnderWater: the camera at or below the level.
            bias: if ctx.eye[1] >= self.level { -500 } else { 500 },
            textures: &self.textures,
        };
        for g in &self.splashes {
            d.drops(&g.lists.drops, g.pos);
            d.sprays(&g.lists.sprays, g.pos);
            d.rings(&g.lists.rings);
        }
        for g in &self.trails {
            d.drops(&g.lists.drops, [0.0; 3]);
            d.sprays(&g.lists.sprays, [0.0; 3]);
            d.rings(&g.lists.rings);
        }
        d.rings(&self.lists.rings);
        d.sprays(&self.lists.sprays, [0.0; 3]);
        d.drops(&self.lists.drops, [0.0; 3]);
    }
}

/// `EftRec_GetDefClass`: how many crossings of a sub-kind 3 / 4 technique pass between two splashes.
fn def_class(sub: i32, id: i32) -> u8 {
    let mut c = match sub {
        3 => 3,
        4 => 2,
        _ => 0,
    };
    if matches!(id, 0x1C8 | 0x159 | 0x270) {
        c = 3;
    }
    if id == 0x16F {
        c = 4;
    }
    c
}

/// The arguments of `EftWaterDrop_Spawn` (the rotation speed is always 0, the size damping -5).
struct DropArg {
    yaw: f32,
    pitch: f32,
    speed: f32,
    rot: f32,
    size: f32,
    size_vel: f32,
    gravity: f32,
    life: f32,
    delay: f32,
    rgb: [u8; 3],
    tex: u8,
    flags: u32,
}

/// The arguments of `EftWaterRing_Spawn` (the rotation is always 0).
struct RingArg {
    size: f32,
    size_vel: f32,
    size_damp: f32,
    alpha_max: f32,
    time: f32,
    delay: f32,
    rgb: [u8; 3],
    tex: u8,
    flags: u32,
}

/// The arguments of `EftWaterSpray_Spawn`.
struct SprayArg {
    dist: f32,
    height: f32,
    yaw: f32,
    pitch: f32,
    roll: f32,
    tilt: [f32; 2],
    speed: f32,
    size: f32,
    size_vel: f32,
    size_damp: f32,
    gravity: f32,
    near: f32,
    width: f32,
    life: f32,
    delay: f32,
    rgb: [u8; 3],
    tex: u8,
    layer: u8,
    flags: u32,
}

/// `EftWaterDrop_UpdateList`, `EftWaterSpray_UpdateList` and `EftWaterRing_UpdateList` of one owner: the dead
/// leave their lists and go back to their pools.
fn update_lists(l: &mut Lists, pools: &mut Pools, level: f32) {
    let before = l.drops.len();
    l.drops.retain_mut(|d| !step_drop(d));
    pools.drops -= before - l.drops.len();
    let before = l.sprays.len();
    l.sprays.retain_mut(|s| !step_spray(s));
    pools.sprays -= before - l.sprays.len();
    let before = l.rings.len();
    l.rings.retain_mut(|r| !step_ring(r, level));
    pools.rings -= before - l.rings.len();
}

/// `EftWaterDrop_Update`: true once it is dead.
fn step_drop(d: &mut Drop) -> bool {
    if d.flags & WAIT != 0 {
        d.delay -= 1.0;
        if d.delay <= 0.0 {
            d.delay = 0.0;
            d.flags = (d.flags & !WAIT) | LIVE;
        }
    }
    if d.flags & LIVE != 0 {
        let t = d.t;
        d.pos = [
            d.base[0] + d.vel[0] * t,
            d.base[1] + d.vel[1] * t + d.gravity * t * t,
            d.base[2] + d.vel[2] * t,
        ];
        d.size += d.size_vel;
        d.size_vel += d.size_vel * d.size_damp;
        d.t += d.dt;
        if d.flags & ENDING == 0 {
            if d.flags & FADE_IN != 0 {
                d.alpha += d.alpha_step;
                if d.alpha >= 128.0 {
                    d.alpha = 128.0;
                    if d.flags & RISE != 0 {
                        d.flags |= ENDING;
                    }
                }
            } else if d.flags & FADE_OUT != 0 {
                d.alpha -= d.alpha_step;
                if d.alpha <= 0.0 {
                    d.alpha = 0.0;
                    d.flags |= ENDING;
                }
            }
        }
    }
    if d.flags & ENDING != 0 {
        if d.flags & RISE != 0 {
            d.alpha += d.alpha_step;
            if d.alpha >= 128.0 {
                d.alpha = 128.0;
                d.flags |= DEAD;
            }
        } else if d.flags & FADE_OUT != 0 {
            d.alpha -= d.alpha_step;
            if d.alpha <= 0.0 {
                d.alpha = 0.0;
                d.flags |= DEAD;
            }
        }
    }
    d.flags & DEAD != 0
}

/// `EftWaterRing_Update`: it follows the surface (4 above the level), grows, fades in, then out.
fn step_ring(r: &mut Ring, level: f32) -> bool {
    if r.flags & WAIT != 0 {
        r.delay -= 1.0;
        if r.delay <= 0.0 {
            r.delay = 0.0;
            r.flags = (r.flags | LIVE) & !WAIT;
        }
    }
    if r.flags & LIVE != 0 {
        r.pos[1] = level - 4.0;
        r.size += r.size_vel;
        r.size_vel += r.size_vel * r.size_damp;
        if r.flags & ENDING == 0 {
            if r.flags & FADE_IN != 0 {
                r.alpha += r.alpha_step;
                if r.alpha >= r.alpha_max {
                    r.alpha = r.alpha_max;
                    r.flags &= !FADE_IN;
                    if r.flags & FADE_OUT != 0 {
                        r.alpha_max = 0.0;
                        r.alpha_step = r.alpha / (r.fade_time * FPS);
                    }
                }
            } else if r.flags & FADE_OUT != 0 {
                r.alpha -= r.alpha_step;
                if r.alpha <= r.alpha_max {
                    r.alpha = r.alpha_max;
                    r.flags = (r.flags & !FADE_OUT) | ENDING;
                }
            }
        }
    }
    if r.flags & ENDING != 0 {
        r.alpha -= r.alpha_step;
        if r.alpha <= 0.0 {
            r.alpha = 0.0;
            r.flags |= DEAD;
        }
    }
    r.flags & DEAD != 0
}

/// `EftWaterSpray_Update`: it flies, grows, tilts and dissolves.
fn step_spray(s: &mut Spray) -> bool {
    if s.flags & WAIT != 0 {
        s.delay -= 1.0;
        if s.delay <= 0.0 {
            s.delay = 0.0;
            s.flags = (s.flags | LIVE) & !WAIT;
        }
    }
    if s.flags & LIVE != 0 {
        let t = s.t;
        s.pos = [
            s.base[0] + s.vel[0] * t,
            s.base[1] + s.vel[1] * t + s.gravity * t * t,
            s.base[2] + s.vel[2] * t,
        ];
        s.size += s.size_vel;
        s.size_vel += s.size_vel * s.size_damp;
        s.tilt += s.tilt_vel;
        s.t += s.dt;
        if s.flags & ENDING == 0 && s.flags & RISE != 0 {
            s.progress += s.progress_step;
            if s.progress >= s.progress_max {
                s.progress = s.progress_max;
                s.flags |= ENDING;
            }
        }
    }
    if s.flags & ENDING != 0 && s.flags & RISE != 0 {
        s.progress += s.progress_step;
        if s.progress >= s.progress_max {
            s.progress = s.progress_max;
            s.flags |= DEAD;
        }
    }
    s.flags & DEAD != 0
}

/// A projected vertex: GS x, y in 1/16 pixel and the integer z (`Mtx_ProjectPoint`), and Q.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Scr {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub q: f32,
}

pub(crate) fn project(m: &Mtx, p: V3) -> Scr {
    let v = apply(m, point(p));
    let q = 1.0 / v[3];
    Scr {
        x: (v[0] * q * 16.0) as i32,
        y: (v[1] * q * 16.0) as i32,
        z: (v[2] * q) as i32,
        q,
    }
}

/// `EftWater_IsOnScreen`'s boxes: 2 a little wider than the screen (the drops), else the GS range.
pub(crate) fn on_screen(v: &Scr, margin: u32) -> bool {
    let (x0, x1, y0, y1) = match margin {
        2 => (0x6800, 0x97FF, 0x6B00, 0x94FF),
        _ => (0, 0xFFEF, 0, 0xFFEF),
    };
    v.z > 0 && v.x <= x1 && v.x > x0 && v.y <= y1 && v.y > y0
}

/// The draw state of one frame.
struct Drawer<'a, 'o> {
    ot: std::cell::RefCell<&'a mut OrderTable>,
    ctx: &'a DrawCtx,
    /// `gEftDust->billboard`: the inverse view rotation.
    billboard: Mtx,
    /// The depth slots' bias: in front of the surface's own polygons (500 nearer), or behind them seen from under
    /// the water.
    bias: i32,
    textures: &'o [usize],
}

impl Drawer<'_, '_> {
    fn texture(&self, tex: u8) -> Option<usize> {
        self.textures.get(usize::from(tex)).copied()
    }

    /// `EftWaterOt_Add` of the two triangles of a strip; an alpha test reference around them when not 0.
    #[allow(clippy::too_many_arguments)]
    fn strip(
        &self,
        scr: &[Scr; 4],
        st: [[f32; 2]; 4],
        q_one: bool,
        rgba: [u8; 4],
        alpha_ref: u8,
        tex: u8,
        layer: u8,
    ) {
        let sum = scr
            .iter()
            .fold(0i32, |s, v| s.wrapping_add(v.z))
            .wrapping_shr(10);
        let z = sum + self.bias;
        let vert = |k: usize| {
            let q = if q_one { 1.0 } else { scr[k].q };
            EffectVertex {
                position: [
                    scr[k].x as f32 / 16.0 - OFFSET_X,
                    scr[k].y as f32 / 16.0 - OFFSET_Y,
                    (scr[k].z & 0xFF_FFFF) as f32,
                ],
                stq: [st[k][0] * q, st[k][1] * q, q],
                rgba,
            }
        };
        let l = if layer >= 2 { layer - 2 } else { layer };
        let blend = EffectBlend::of(usize::from(l), layer >= 2);
        let mut ot = self.ot.borrow_mut();
        for k in 0..2 {
            ot.add(
                depth_slot(z),
                usize::from(l),
                EffectTri {
                    texture: self.texture(tex),
                    blend,
                    clamp: false,
                    zwrite: false,
                    alpha_ref,
                    vertices: [vert(k), vert(k + 1), vert(k + 2)],
                },
            );
        }
    }

    /// `EftWaterDrop_DrawList`: a rising drop dissolves (alpha 0x60, its alpha the test's reference), the others
    /// fade.
    fn drops(&self, list: &[Drop], origin: V3) {
        for d in list.iter().filter(|d| d.flags & LIVE != 0) {
            let pos = add(d.pos, origin);
            let (a, aref) = if d.flags & RISE != 0 {
                (0x60, d.alpha as u32 as u8)
            } else {
                (d.alpha as u32 as u8, 0)
            };
            self.billboard(pos, d.size, d.rot, d.rgb, a, aref, d.tex, d.layer);
        }
    }

    /// `EftWater_DrawBillboard`: a camera-facing quad of side `size` turned by `rot` degrees; dropped when a
    /// corner is out of the wide screen box.
    #[allow(clippy::too_many_arguments)]
    fn billboard(
        &self,
        pos: V3,
        size: f32,
        rot: f32,
        rgb: [u8; 3],
        a: u8,
        aref: u8,
        tex: u8,
        layer: u8,
    ) {
        let mut m = self.billboard;
        if rot != 0.0 {
            let rz = rotate_xyz(&IDENTITY, [0.0, 0.0, eft_wrap(rot * PI_F / 180.0)]);
            m = mtx_mul(&m, &rz);
        }
        m[3] = point(pos);
        let h = size * 0.5;
        let corners = [[-h, -h, 0.0], [h, -h, 0.0], [-h, h, 0.0], [h, h, 0.0]];
        let mut scr = [Scr {
            x: 0,
            y: 0,
            z: 0,
            q: 1.0,
        }; 4];
        for (k, c) in corners.into_iter().enumerate() {
            scr[k] = project(&self.ctx.world2screen, xyz(apply(&m, point(c))));
            if !on_screen(&scr[k], 2) {
                return;
            }
        }
        let st = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let rgba = [rgb[0], rgb[1], rgb[2], a];
        self.strip(&scr, st, true, rgba, aref, tex, layer);
    }

    /// `EftWaterSpray_DrawList`: a quad along the flight direction pulled up by the tilt, facing the camera, at
    /// `origin` + its position, dissolving by its progress (alpha 0x40).
    fn sprays(&self, list: &[Spray], origin: V3) {
        let up = [0.0, -1.0, 0.0];
        for s in list.iter().filter(|s| s.flags & LIVE != 0) {
            // Vec3_Lerp: up * tilt + dir * (1 - tilt).
            let rot: V3 = std::array::from_fn(|i| up[i] * s.tilt + s.dir[i] * (1.0 - s.tilt));
            let orient = self.facing(rot);
            let pos = add(s.pos, origin);
            self.spray_quad(pos, &orient, s, 0x40, s.progress as u32 as u8);
        }
    }

    /// `EftUtil_MakeFacingMtx` at the origin: z along `dir` pulled half way to the camera's view axis.
    fn facing(&self, dir: V3) -> Mtx {
        let v = &self.ctx.world2view;
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
        [
            [x[0], x[1], x[2], 0.0],
            [y[0], y[1], y[2], 0.0],
            [z[0], z[1], z[2], 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    /// `EftWater_DrawSprayQuad`: from `size * near` to 0 along the orientation's z, `size * width` wide, rolled
    /// about its middle; dropped when a corner is off the GS range.
    fn spray_quad(&self, pos: V3, orient: &Mtx, s: &Spray, a: u8, aref: u8) {
        let half = s.size * 0.5 * s.width;
        let z_near = s.size * s.near;
        let mut corners = [
            [-half, 0.0, z_near],
            [half, 0.0, z_near],
            [-half, 0.0, 0.0],
            [half, 0.0, 0.0],
        ];
        if s.roll != 0.0 {
            let mid = (corners[0][2] - corners[2][2]) * 0.5;
            let ry = rotate_xyz(&IDENTITY, [0.0, eft_wrap(s.roll * PI_F / 180.0), 0.0]);
            for c in &mut corners {
                c[2] -= mid;
                *c = xyz(apply(&ry, point(*c)));
                c[2] += mid;
            }
        }
        let mut scr = [Scr {
            x: 0,
            y: 0,
            z: 0,
            q: 1.0,
        }; 4];
        for (k, c) in corners.into_iter().enumerate() {
            let w = add(xyz(apply(orient, point(c))), pos);
            scr[k] = project(&self.ctx.world2screen, w);
        }
        if !scr.iter().all(|v| on_screen(v, 4)) {
            return;
        }
        let st = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let rgba = [s.rgb[0], s.rgb[1], s.rgb[2], a];
        self.strip(&scr, st, false, rgba, aref, s.tex, s.layer);
    }

    /// `EftWaterRing_DrawList` / `EftWater_DrawGroundQuad`: a quad of side `size` on the surface, as two clipped
    /// fans.
    fn rings(&self, list: &[Ring]) {
        for r in list.iter().filter(|r| r.flags & LIVE != 0) {
            let h = r.size * 0.5;
            let ry = rotate_xyz(&IDENTITY, [0.0, eft_wrap(r.rot * PI_F / 180.0), 0.0]);
            let corners = [[-h, 0.0, -h], [h, 0.0, -h], [-h, 0.0, h], [h, 0.0, h]].map(|c| {
                let c = if r.rot != 0.0 {
                    xyz(apply(&ry, point(c)))
                } else {
                    c
                };
                point(add(c, r.pos))
            });
            let uv = [
                [0.0, 0.0, 1.0, 0.0],
                [1.0, 0.0, 1.0, 0.0],
                [0.0, 1.0, 1.0, 0.0],
                [1.0, 1.0, 1.0, 0.0],
            ];
            let col: V4 = [
                f32::from(r.rgb[0]),
                f32::from(r.rgb[1]),
                f32::from(r.rgb[2]),
                f32::from(r.alpha as u32 as u8),
            ];
            for i in 0..2 {
                let tri = std::array::from_fn(|k| ClipVtx {
                    pos: corners[i + k],
                    uv: uv[i + k],
                    col,
                });
                self.clipped_fan(tri, r.tex, r.layer);
            }
        }
    }

    /// `EftWater_DrawClippedFan`: the triangle clipped by the view's planes, projected, and its fan queued
    /// (a fan triangle whose three corners are off the GS range is dropped).
    fn clipped_fan(&self, tri: [ClipVtx; 3], tex: u8, layer: u8) {
        let mut poly = tri.to_vec();
        for plane in self.ctx.planes() {
            clip_plane(&mut poly, plane);
        }
        if poly.is_empty() {
            return;
        }
        let mut scr: Vec<Scr> = poly
            .iter()
            .map(|v| project(&self.ctx.world2screen, xyz(v.pos)))
            .collect();
        let l = if layer >= 2 { layer - 2 } else { layer };
        let blend = EffectBlend::of(usize::from(l), layer >= 2);
        let off = |v: &Scr| v.z <= 0 || v.x > 0xFFEF || v.x <= 0 || v.y > 0xFFEF || v.y <= 0;
        for i in 2..poly.len() {
            let z = ((i64::from(scr[0].z) + i64::from(scr[i - 1].z) + i64::from(scr[i].z)) / 3)
                as i32
                >> 8;
            let z = z + self.bias;
            for k in [0, i - 1, i] {
                scr[k].z = scr[k].z.min(0xFF_FFFF);
            }
            if off(&scr[0]) && off(&scr[i - 1]) && off(&scr[i]) {
                continue;
            }
            let vert = |k: usize| {
                let (v, s) = (&poly[k], &scr[k]);
                EffectVertex {
                    position: [
                        s.x as f32 / 16.0 - OFFSET_X,
                        s.y as f32 / 16.0 - OFFSET_Y,
                        (s.z & 0xFF_FFFF) as f32,
                    ],
                    stq: [v.uv[0] * s.q, v.uv[1] * s.q, v.uv[2] * s.q],
                    rgba: v.col.map(|c| c as i32 as u8),
                }
            };
            self.ot.borrow_mut().add(
                depth_slot(z),
                usize::from(l),
                EffectTri {
                    texture: self.texture(tex),
                    blend,
                    clamp: false,
                    zwrite: false,
                    alpha_ref: 0,
                    vertices: [vert(0), vert(i - 1), vert(i)],
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::View;

    const COEF: [f32; 4] = [2.6e-6, -1.98e-4, 8.33e-3, -0.166_666_7];

    fn water() -> Water {
        Water::new(32.0, 1, COEF, vec![10, 11, 12, 13, 14])
    }

    #[test]
    fn a_splash_spawns_its_particles_and_frees_them_when_done() {
        let mut w = water();
        let mut libc = LibcRandom::new(7);
        w.add_splash_for(20.0, [0.0, 50.0, 0.0], SPLASH_CHAR, 16.0, &mut libc);
        // 7 + 4 sprays, 3 + 2 drops, 4 rings.
        assert_eq!(w.counts(), (5, 4, 11));
        assert_eq!(w.groups(), (1, 0));
        assert_eq!(w.splashes[0].pos, [0.0, 31.0, 0.0]);
        let chars = [WaterChr::default(); 2];
        // Paused, nothing moves.
        for _ in 0..10 {
            w.update(&chars, true, &mut libc);
        }
        assert_eq!(w.counts(), (5, 4, 11));
        let mut frames = 0;
        while w.groups().0 > 0 {
            w.update(&chars, false, &mut libc);
            frames += 1;
            assert!(frames < 200);
        }
        // The longest part is the third ring: 27 frames of delay, 11 to fade in, 25 to fade out.
        assert_eq!(frames, 63);
        assert_eq!(w.counts(), (0, 0, 0));
        assert!(!w.splash_used.iter().any(|&u| u));
    }

    #[test]
    fn the_pools_and_the_ten_splashes_limit_the_effects() {
        let mut w = water();
        let mut libc = LibcRandom::new(1);
        for _ in 0..12 {
            w.add_splash_at([0.0; 3], 3.0, &mut libc);
        }
        // Ten splashes; the 30 sprays run out in the third, the 30 rings in the eighth.
        assert_eq!(w.groups().0, 10);
        assert_eq!(w.counts(), (50, 30, 30));
        // Split screen: nothing.
        let mut w = water();
        w.split = true;
        w.add_splash_at([0.0; 3], 3.0, &mut libc);
        assert_eq!(w.groups().0, 0);
    }

    #[test]
    fn a_blast_crossing_the_surface_splashes_with_its_sound() {
        let mut w = water();
        let mut libc = LibcRandom::new(3);
        // A ki blast going down through the level (+y is down).
        let mut rec = WaterRec {
            owner: 0,
            technique: false,
            def_sub: 0,
            def_id: 0,
            pos: [0.0, 40.0, 10.0],
            prev: [0.0, 20.0, 0.0],
            no_water: false,
            task_flags: 0,
            task_pos: [0.0; 3],
            speed: 5.0,
            radius_a: 5.0,
        };
        let (set, sound) = w.update_blast(&rec, 20.0, false, &mut libc);
        assert_eq!(set, 0);
        let (at, id) = sound.unwrap();
        assert_eq!(id, 0x3C);
        assert!(
            (at[1] - 32.0).abs() < 1e-4 && (at[2] - 6.0).abs() < 1e-4,
            "{at:?}"
        );
        assert_eq!(w.groups().0, 1);
        // Coming out: 0x3D.
        (rec.pos, rec.prev) = (rec.prev, rec.pos);
        assert_eq!(
            w.update_blast(&rec, 20.0, false, &mut libc).1.unwrap().1,
            0x3D
        );
        // A record that hit the stage, or paused: nothing.
        rec.task_flags = TASK_HIT_STAGE;
        assert_eq!(w.update_blast(&rec, 20.0, false, &mut libc), (0, None));
        rec.task_flags = 0;
        assert_eq!(w.update_blast(&rec, 20.0, true, &mut libc), (0, None));
        // One that hit a fighter splashes from the hit position once.
        rec.task_flags = TASK_HIT_CHAR;
        rec.task_pos = [0.0, 33.0, 0.0];
        rec.pos = [0.0, 33.0, 0.0];
        rec.prev = [0.0, 30.0, 0.0];
        let (set, sound) = w.update_blast(&rec, 20.0, false, &mut libc);
        assert_eq!(set, TASK_DONE);
        assert!(sound.is_some());
    }

    #[test]
    fn a_technique_skimming_the_surface_leaves_trails_and_rate_limits_its_splashes() {
        let mut w = water();
        let mut libc = LibcRandom::new(5);
        let mut rec = WaterRec {
            owner: 1,
            technique: true,
            def_sub: 1,
            def_id: 0x10,
            pos: [10.0, 30.0, 0.0],
            prev: [0.0, 30.0, 0.0],
            no_water: false,
            task_flags: 0,
            task_pos: [0.0; 3],
            speed: 5.0,
            radius_a: 5.0,
        };
        for _ in 0..20 {
            assert_eq!(w.update_blast(&rec, 20.0, false, &mut libc), (0, None));
        }
        assert_eq!(w.groups(), (0, 15));
        // The trails fade: at most five at full strength.
        let chars = [WaterChr::default(); 2];
        w.update(&chars, false, &mut libc);
        let full = w
            .trails
            .iter()
            .filter(|t| w.trail_flags[t.slot] & ENDING == 0)
            .count();
        assert_eq!(full, 5);
        // A sub-kind 3 technique splashes on one crossing in four (EftRec_GetDefClass 3).
        let mut w = water();
        rec.def_sub = 3;
        rec.prev = [0.0, 20.0, 0.0];
        rec.pos = [0.0, 40.0, 0.0];
        let fighters = [
            WaterChr::default(),
            WaterChr {
                in_technique: true,
                ..WaterChr::default()
            },
        ];
        for _ in 0..8 {
            w.update_blast(&rec, 20.0, false, &mut libc);
            w.update(&fighters, true, &mut libc);
        }
        assert_eq!(w.groups().0, 2);
    }

    #[test]
    fn a_wake_follows_a_fighter_and_its_particles_draw() {
        let mut w = water();
        let mut libc = LibcRandom::new(9);
        let mut chars = [WaterChr::default(); 2];
        chars[0] = WaterChr {
            pos: [0.0, 30.0, 300.0],
            dir: [1.0, 0.0, 0.0],
            speed: 9.0,
            height: 20.0,
            in_technique: false,
        };
        w.set_wake(0, true);
        w.update(&chars, false, &mut libc);
        // Frame 0: two sprays, two drops, a ring.
        assert_eq!(w.counts(), (2, 1, 2));
        w.update(&chars, false, &mut libc);
        assert_eq!(w.counts().2, 3);
        w.set_wake(0, false);
        for _ in 0..60 {
            w.update(&chars, false, &mut libc);
        }
        assert_eq!(w.counts(), (0, 0, 0));
        // Seen from above the surface, a splash in front of the camera draws with the stage textures, the
        // dissolving parts with an alpha test.
        let mut w = water();
        w.add_splash_at([0.0, 0.0, 300.0], 3.0, &mut libc);
        for _ in 0..5 {
            w.update(&chars, false, &mut libc);
        }
        let view = View::new([0.0, -20.0, 0.0], [0.3, 0.0, 0.0]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = OrderTable::new();
        w.draw(&mut ot, &ctx);
        let tris = ot.into_list();
        assert!(!tris.is_empty());
        assert!(tris.iter().all(|t| matches!(t.texture, Some(10..=14))));
        assert!(tris.iter().any(|t| t.alpha_ref > 0));
        assert!(tris.iter().any(|t| t.texture == Some(12)));
    }

    #[test]
    fn the_crossing_point_is_on_the_surface() {
        let w = water();
        let p = w.clip_seg([0.0, 40.0, 8.0], [0.0, 30.0, 4.0]);
        assert!(
            (p[1] - 32.0).abs() < 1e-4 && (p[2] - 4.8).abs() < 1e-4,
            "{p:?}"
        );
    }
}
