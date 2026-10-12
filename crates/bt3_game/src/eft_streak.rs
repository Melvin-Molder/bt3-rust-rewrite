//! The streak fields (`EftStreak_*`, src/battle/eft_streak.c): the "numbered stage effects" a connected rush
//! starts on the victim's event 0x80 and stops on its event 0x100. One of ten kinds (common effect entries
//! 0x24F..0x258) of up to 255 long quads that run along one axis of the view, spread across it with a hole in the
//! middle, turned about the view axis by the technique's angle; drawn in the world around a point in front of the
//! camera (or at the fighter's distance) or straight onto the screen. Three fields at most share a pool of 360
//! streaks; each streak fades in over the first 40 % of its life and out over the rest, then is rolled again.
//!
//! Colour index 0..6 picks the colour (common entry 0x24E) and the palette; every field of a frame is drawn with
//! the palette of the first one stepped (`EftStreak_StepTexture` builds one TEX0 a frame).

use crate::battle::{Mtx, apply, inverse_rt};
use crate::eft::{
    ClipVtx, DrawCtx, EffectBlend, EffectTri, EffectVertex, OrderTable, Prim, V3, V4, ZSort, add,
    depth_slot, norm, point, scale3, sub, xyz,
};
use crate::rng::Vu0Random;

/// `EFT_STREAK_KINDS`, `EFT_STREAK_MAX`, the task list's size.
pub(crate) const KINDS: usize = 10;
const POOL: usize = 360;
const FIELDS: usize = 3;
/// The kinds' parameter entries, the colour table and the texture set of the common effect file.
pub(crate) const DEF_ENTRY: usize = 0x24F;
pub(crate) const COLOR_ENTRY: usize = 0x24E;
pub(crate) const TEXTURE_ENTRY: usize = 0x24D;
/// The palettes a field can take (colour indices 0..6, and 7's 1).
pub(crate) const PALETTES: usize = 7;

// EFT_STREAK_ITEM_*.
const USED: u32 = 0x02;
const DRAWN: u32 = 0x10;
const EXPIRED: u32 = 0x80;

// EFT_STREAK_*.
const ALIVE: u32 = 0x002;
const STOP: u32 = 0x004;
const DEAD: u32 = 0x008;
const SCREEN: u32 = 0x100;
const WORLD: u32 = 0x200;
const AT_CHAR: u32 = 0x400;

/// `EftStreakDef`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StreakDef {
    pub width: [f32; 2],
    pub length: [f32; 2],
    /// Seconds.
    pub life: [f32; 2],
    pub delay_max: f32,
    pub start_delay: f32,
    pub fade_frames: f32,
    pub pos: [f32; 2],
    pub speed: [f32; 2],
    pub spread: f32,
    pub hole: f32,
    /// 1: ends when either fighter starts a technique; 2: when a camera cut runs.
    pub end_mode: i32,
    /// 0, 1: in the world; 2, 3: on the screen; 1, 3: at the fighter.
    pub space: i32,
    pub blend: u8,
    pub count: u8,
}

impl StreakDef {
    pub(crate) fn parse(b: &[u8]) -> Option<Self> {
        let f = |o: usize| {
            b.get(o..o + 4)
                .map(|w| f32::from_le_bytes(w.try_into().unwrap()))
        };
        let i = |o: usize| {
            b.get(o..o + 4)
                .map(|w| i32::from_le_bytes(w.try_into().unwrap()))
        };
        Some(Self {
            width: [f(0)?, f(4)?],
            length: [f(8)?, f(0xC)?],
            life: [f(0x10)?, f(0x14)?],
            delay_max: f(0x18)?,
            start_delay: f(0x20)?,
            fade_frames: f(0x24)?,
            pos: [f(0x28)?, f(0x2C)?],
            speed: [f(0x30)?, f(0x34)?],
            spread: f(0x38)?,
            hole: f(0x3C)?,
            end_mode: i(0x40)?,
            space: i(0x44)?,
            blend: *b.get(0x48)?,
            count: *b.get(0x49)?,
        })
    }
}

/// The ten kinds and the colour table (`EftStreakMgr_Init`).
#[derive(Clone, Debug, Default)]
pub(crate) struct StreakCfg {
    pub defs: Vec<StreakDef>,
    pub colors: Vec<V4>,
}

impl StreakCfg {
    /// From the common effect file.
    pub(crate) fn read(common: &[u8]) -> Option<Self> {
        let entry = |n: usize| bt3_data::eft_pack::common_entry(common, n);
        let defs = (0..KINDS)
            .map(|k| entry(DEF_ENTRY + k).and_then(StreakDef::parse))
            .collect::<Option<Vec<_>>>()?;
        let colors = entry(COLOR_ENTRY)?
            .chunks_exact(16)
            .take(8)
            .map(|c| {
                std::array::from_fn(|k| f32::from_le_bytes(c[k * 4..k * 4 + 4].try_into().unwrap()))
            })
            .collect();
        Some(Self { defs, colors })
    }
}

/// `EftStreak`.
#[derive(Clone, Copy, Debug, Default)]
struct Item {
    color: V4,
    width: f32,
    length: f32,
    pos: f32,
    speed: f32,
    offset: f32,
    time: f32,
    life: f32,
    delay: f32,
    flags: u32,
}

/// `EftStreakWork`.
#[derive(Clone, Debug)]
struct Field {
    handle: u32,
    kind: usize,
    color: usize,
    side: usize,
    angle: f32,
    cover: f32,
    timer: f32,
    start_delay: f32,
    fade: f32,
    alpha: f32,
    spawned: u8,
    flags: u32,
    /// The streaks, in the order they were linked.
    items: Vec<usize>,
}

/// What a field reads of its fighter.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StreakChr {
    /// `BtlCharApi_IsInTechnique`.
    pub in_technique: bool,
    /// `BtlCharApi_GetNodePos(.., 3)` and `BtlCharApi_GetHeight`.
    pub node3: V3,
    pub height: f32,
}

/// `gEftStreak`: the pool and the live fields.
#[derive(Clone, Debug)]
pub(crate) struct Streaks {
    pool: Vec<Item>,
    next: usize,
    fields: Vec<Field>,
    /// The colour index whose palette this frame's fields are drawn with.
    palette: Option<usize>,
}

impl Default for Streaks {
    fn default() -> Self {
        Self {
            pool: vec![Item::default(); POOL],
            next: 0,
            fields: Vec::new(),
            palette: None,
        }
    }
}

impl Streaks {
    /// `EftStreak_Start`: field `kind` for a fighter under `handle`, turned by `angle`; colour 0..6 picks the
    /// colour, 7 means 1, anything else 2. Nothing for a kind out of range or when three fields run.
    pub(crate) fn start(
        &mut self,
        cfg: &StreakCfg,
        handle: u32,
        side: usize,
        kind: i32,
        color: i32,
        angle: f32,
    ) -> bool {
        self.try_start(cfg, handle, side, kind, color, angle)
            .is_some()
    }

    fn try_start(
        &mut self,
        cfg: &StreakCfg,
        handle: u32,
        side: usize,
        kind: i32,
        color: i32,
        angle: f32,
    ) -> Option<()> {
        let kind = usize::try_from(kind).ok().filter(|&k| k < KINDS)?;
        let def = cfg.defs.get(kind)?;
        if self.fields.len() >= FIELDS {
            return None;
        }
        let color = match color {
            c if c < 7 => (c & 0xFF) as usize,
            7 => 1,
            _ => 2,
        };
        // EftStreak_Init.
        let mut flags = ALIVE;
        flags |= match def.space {
            2 | 3 => SCREEN,
            0 | 1 => WORLD,
            _ => 0,
        };
        if matches!(def.space, 1 | 3) {
            flags |= AT_CHAR;
        }
        self.fields.push(Field {
            handle,
            kind,
            color,
            side,
            angle,
            cover: 1.0,
            timer: 0.0,
            start_delay: def.start_delay,
            fade: def.fade_frames,
            alpha: 1.0,
            spawned: 0,
            flags,
            items: Vec::new(),
        });
        Some(())
    }

    /// `EftStreak_Stop`: the field starts its fade.
    pub(crate) fn stop(&mut self, handle: u32) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.handle == handle)
            && f.flags & ALIVE != 0
        {
            f.flags |= STOP;
        }
    }

    /// The live fields.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.fields.len()
    }

    /// The streaks of the live fields that are drawn.
    #[cfg(test)]
    pub(crate) fn drawn(&self) -> usize {
        self.fields
            .iter()
            .flat_map(|f| &f.items)
            .filter(|&&i| self.pool[i].flags & DRAWN != 0)
            .count()
    }

    /// `EftStreak_Reset` of every field: none survives a scene reset.
    pub(crate) fn reset(&mut self) {
        for f in std::mem::take(&mut self.fields) {
            self.term(&f);
        }
    }

    /// `EftStreak_Term`: the streaks go back to the pool.
    fn term(&mut self, f: &Field) {
        for &i in &f.items {
            self.pool[i].flags = 0;
        }
    }

    /// `EftStreak_Alloc`: a free streak, searching on from the last one handed out.
    fn alloc(&mut self) -> Option<usize> {
        if self.next >= POOL {
            self.next = 0;
        }
        let mut i = self.next;
        loop {
            let s = i;
            i = (i + 1) % POOL;
            if self.pool[s].flags == 0 {
                self.next = i;
                return Some(s);
            }
            if i == self.next {
                return None;
            }
        }
    }

    /// `EftStreak_Roll`: the colour and the random size, position, speed, life and delay (six draws).
    fn roll(item: &mut Item, def: &StreakDef, color: V4, offset: f32, vu0: &mut Vu0Random) {
        let mut r = |a: [f32; 2]| vu0.float_range(a[0], a[0] + a[1]);
        item.color = color;
        item.width = r(def.width);
        item.length = r(def.length);
        item.pos = r(def.pos);
        item.speed = r(def.speed);
        item.life = r(def.life) * 30.0;
        item.time = 0.0;
        item.delay = vu0.float_range(item.time, def.delay_max);
        item.offset = offset;
    }

    /// `EftStreakMgr_Update` and `EftStreak_Update` of every field (`paused`: `BtlScene_IsEffectStopped(.., 2)`;
    /// `cam_active`: `EftCam_IsActive`).
    pub(crate) fn update(
        &mut self,
        cfg: &StreakCfg,
        chars: &[StreakChr; 2],
        cam_active: bool,
        paused: bool,
        vu0: &mut Vu0Random,
    ) {
        self.palette = None;
        let mut k = 0;
        while k < self.fields.len() {
            let mut f = self.fields[k].clone();
            let def = cfg.defs[f.kind];
            let color = cfg.colors.get(f.color).copied().unwrap_or_default();
            if !paused {
                if f.start_delay >= 0.0 {
                    f.start_delay -= 1.0;
                } else {
                    let end = match def.end_mode {
                        1 => chars[f.side].in_technique || chars[f.side ^ 1].in_technique,
                        2 => cam_active,
                        _ => false,
                    };
                    if end {
                        f.flags |= DEAD;
                    }
                    if f.flags & STOP == 0 {
                        self.spawn_all(&mut f, &def, color, vu0);
                    }
                    if f.flags & STOP != 0 {
                        f.alpha = (f.fade / def.fade_frames).clamp(0.0, 1.0);
                        f.fade -= 1.0;
                        if f.fade < 0.0 {
                            f.flags |= DEAD;
                        }
                    }
                    self.step(&mut f, &def, color, vu0);
                    if f.timer > 0.0 {
                        f.timer -= 1.0;
                        if f.timer < 0.0 {
                            f.flags |= STOP;
                        }
                    }
                }
                if f.flags & DEAD != 0 {
                    self.term(&f);
                    self.fields.remove(k);
                    continue;
                }
            }
            // EftStreak_StepTexture: the first field stepped this frame gives the palette.
            self.palette.get_or_insert(f.color);
            self.fields[k] = f;
            k += 1;
        }
    }

    /// `EftStreak_SpawnAll`: the kind's streaks, evenly spread across the band; stops when the pool is used up.
    fn spawn_all(&mut self, f: &mut Field, def: &StreakDef, color: V4, vu0: &mut Vu0Random) {
        let gap = def.spread / f32::from(def.count);
        while f.spawned < def.count {
            let Some(i) = self.alloc() else {
                break;
            };
            let offset = def.spread * 0.5 - gap * f32::from(f.spawned);
            Self::roll(&mut self.pool[i], def, color, offset, vu0);
            self.pool[i].flags = USED;
            f.items.push(i);
            f.spawned += 1;
        }
    }

    /// `EftStreak_Step`: delay, movement, the fade in and out over the life, whether it is drawn (outside the
    /// hole, alpha above 0); an expired streak is rolled again in place.
    fn step(&mut self, f: &mut Field, def: &StreakDef, color: V4, vu0: &mut Vu0Random) {
        let half = def.spread * (def.hole * f.cover).clamp(0.0, 1.0) * 0.5;
        let fade_in = 0.4;
        for &i in &f.items {
            let s = &mut self.pool[i];
            if s.delay > 0.0 {
                s.delay -= 1.0;
                continue;
            }
            s.pos += s.speed;
            let mut a = 1.0;
            if s.life > 0.0 {
                let t = s.time / s.life;
                a = if t < fade_in {
                    (t / fade_in).clamp(0.0, 1.0)
                } else {
                    (1.0 - (t - fade_in) / (1.0 - fade_in)).clamp(0.0, 1.0)
                };
                s.time += 1.0;
                if s.life <= s.time {
                    s.flags |= EXPIRED;
                }
            }
            s.color[3] = color[3] * f.alpha * a;
            if (s.offset < half && -half < s.offset) || s.color[3] <= 0.0 {
                s.flags &= !DRAWN;
            } else {
                s.flags |= DRAWN;
            }
            if s.flags & EXPIRED != 0 {
                let offset = s.offset;
                Self::roll(s, def, color, offset, vu0);
                s.flags = USED;
            }
        }
    }

    /// `EftStreak_Draw` of every field. `textures[c]`: the set's image under colour index `c`'s palette.
    pub(crate) fn draw(
        &self,
        cfg: &StreakCfg,
        chars: &[StreakChr; 2],
        textures: &[Option<usize>],
        ot: &mut OrderTable,
        ctx: &DrawCtx,
    ) {
        let texture = self
            .palette
            .and_then(|p| textures.get(p).copied().flatten());
        // The camera's rotation (its translation dropped) and its forward axis.
        let mut cam_inv = inverse_rt(&ctx.world2view);
        cam_inv[3] = [0.0, 0.0, 0.0, 1.0];
        let dir = norm(xyz(apply(&cam_inv, [0.0, 0.0, 1.0, 1.0])));
        for f in &self.fields {
            let def = &cfg.defs[f.kind];
            let dist = if f.flags & AT_CHAR != 0 {
                let c = &chars[f.side];
                crate::btl_chr::length(sub(c.node3, ctx.eye)) + c.height * 0.5
            } else {
                30.0
            };
            let center = add(ctx.eye, scale3(dir, dist));
            let (sx, sy, sz) = ctx.project(center);
            let (sx, sy) = ((sx * 16.0) as i32, (sy * 16.0) as i32);
            let mut frame: Mtx = crate::battle::IDENTITY;
            if f.flags & WORLD != 0 {
                frame = cam_inv;
                frame[3] = point(center);
            } else {
                if sz < 0 {
                    continue;
                }
                frame[3] = [
                    ((sx - 0x7000) >> 4) as f32,
                    ((sy - 0x7200) >> 4) as f32,
                    0.0,
                    1.0,
                ];
            }
            let (s, c) = f.angle.sin_cos();
            let base = [[-0.5, -0.5], [-0.5, 0.5], [0.5, -0.5], [0.5, 0.5]];
            for &i in &f.items {
                let it = &self.pool[i];
                if it.flags & DRAWN == 0 {
                    continue;
                }
                let corners: [V3; 4] = std::array::from_fn(|j| {
                    // Translated across and along, then turned about the view axis, then into the frame.
                    let x = base[j][0] * it.width + it.pos;
                    let y = base[j][1] * it.length + it.offset;
                    let (rx, ry) = (x * c - y * s, x * s + y * c);
                    xyz(apply(&frame, [rx, ry, 0.0, 1.0]))
                });
                if f.flags & WORLD != 0 {
                    let front = f.flags & AT_CHAR == 0;
                    let st = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
                    for t in 0..2 {
                        let tri = std::array::from_fn(|k| ClipVtx {
                            pos: point(corners[t + k]),
                            uv: [st[t + k][0], st[t + k][1], 1.0, 1.0],
                            col: it.color,
                        });
                        let prim = Prim {
                            texture,
                            ot_layer: def.blend,
                            z: ZSort::Average(2.0),
                            front,
                            clamp: false,
                            alpha_min: 0.01,
                        };
                        crate::eft::queue_tri_clip(ot, ctx, tri, prim);
                    }
                } else {
                    let z = if f.flags & AT_CHAR != 0 {
                        sz
                    } else {
                        0xFF_FFFF
                    };
                    draw_screen(ot, &corners, it.color, def.blend, 4, z, texture);
                }
            }
        }
    }
}

/// `EftStreak_DrawScreen`: a quad in pixels cut into `segs` strips across, the texture running from t = 1 to 0;
/// z is its GS depth (0xFFFFFF: the far limit, depth slot 0; else slot z >> 8).
fn draw_screen(
    ot: &mut OrderTable,
    corner: &[V3; 4],
    color: V4,
    blend: u8,
    segs: i32,
    z: i32,
    texture: Option<usize>,
) {
    let fsegs = segs as f32;
    let d = [
        scale3(sub(corner[0], corner[1]), 1.0 / fsegs),
        scale3(sub(corner[2], corner[3]), 1.0 / fsegs),
    ];
    let (mut p1, mut p3) = (corner[1], corner[3]);
    let rgba = color.map(|c| c as i32 as u8);
    let (slot, zz) = if z != 0xFF_FFFF {
        let slot = z >> 8;
        (slot, slot << 8)
    } else {
        (0, 0xFF_FFFF)
    };
    let step = -1.0 / fsegs;
    let (mut ty, mut tw) = (1.0f32, 1.0f32);
    let layer = if blend >= 2 { blend - 2 } else { blend };
    for _ in 0..segs.max(0) {
        let p0 = add(p1, d[0]);
        let p2 = add(p3, d[1]);
        let (tx, tz) = (ty + step, tw + step);
        let vert = |p: V3, s: f32, t: f32| EffectVertex {
            position: [p[0] as i32 as f32, p[1] as i32 as f32, zz as f32],
            stq: [s, t, 1.0],
            rgba,
        };
        let v = [
            vert(p0, 0.0, tx),
            vert(p1, 0.0, ty),
            vert(p2, 1.0, tz),
            vert(p3, 1.0, tw),
        ];
        for k in 0..2 {
            ot.add(
                depth_slot(slot),
                usize::from(layer),
                EffectTri {
                    texture,
                    blend: EffectBlend::of(usize::from(layer), blend >= 2),
                    clamp: false,
                    zwrite: false,
                    alpha_ref: 0,
                    vertices: [v[k], v[k + 1], v[k + 2]],
                },
            );
        }
        (p1, p3) = (p0, p2);
        (ty, tw) = (tx, tz);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(space: i32, end_mode: i32) -> StreakCfg {
        let def = StreakDef {
            width: [1.0, 0.0],
            length: [10.0, 0.0],
            life: [1.0, 0.0],
            delay_max: 0.0,
            start_delay: 1.0,
            fade_frames: 10.0,
            pos: [0.0, 0.0],
            speed: [1.0, 0.0],
            spread: 100.0,
            hole: 0.2,
            end_mode,
            space,
            blend: 1,
            count: 10,
        };
        StreakCfg {
            defs: vec![def; KINDS],
            colors: vec![[128.0, 128.0, 128.0, 128.0]; 8],
        }
    }

    #[test]
    fn a_field_waits_spawns_leaves_its_hole_and_fades_when_stopped() {
        let cfg = cfg(0, 0);
        let mut s = Streaks::default();
        let mut vu0 = Vu0Random::seeded(0.5);
        let chars = [StreakChr::default(); 2];
        assert!(s.start(&cfg, 7, 0, 3, 9, 0.0));
        assert_eq!(s.fields[0].color, 2);
        assert!(!s.start(&cfg, 8, 0, KINDS as i32, 0, 0.0));
        // Two frames of start delay (1, then 0), then all ten at once.
        for _ in 0..2 {
            s.update(&cfg, &chars, false, false, &mut vu0);
            assert!(s.fields[0].items.is_empty());
        }
        s.update(&cfg, &chars, false, false, &mut vu0);
        assert_eq!(s.fields[0].items.len(), 10);
        // Offsets 50, 40, .. -40: the one strictly inside the hole's +-10 is not drawn.
        for _ in 0..5 {
            s.update(&cfg, &chars, false, false, &mut vu0);
        }
        assert_eq!(s.drawn(), 9);
        assert_eq!(s.palette, Some(2));
        // Paused: nothing moves.
        let before = s.pool[s.fields[0].items[0]].pos;
        s.update(&cfg, &chars, false, true, &mut vu0);
        assert_eq!(s.pool[s.fields[0].items[0]].pos, before);
        s.stop(7);
        for _ in 0..10 {
            s.update(&cfg, &chars, false, false, &mut vu0);
        }
        assert_eq!(s.count(), 1);
        s.update(&cfg, &chars, false, false, &mut vu0);
        assert_eq!(s.count(), 0);
        assert!(s.pool.iter().all(|i| i.flags == 0));
    }

    #[test]
    fn end_modes_end_a_field_and_three_fields_fill_the_list() {
        let cfg = cfg(2, 1);
        let mut s = Streaks::default();
        let mut vu0 = Vu0Random::seeded(0.5);
        for h in 0..3 {
            assert!(s.start(&cfg, h, 1, 0, 0, 0.0));
        }
        assert!(!s.start(&cfg, 3, 1, 0, 0, 0.0));
        let mut chars = [StreakChr::default(); 2];
        chars[0].in_technique = true;
        s.update(&cfg, &chars, false, false, &mut vu0);
        assert_eq!(s.count(), 3);
        s.update(&cfg, &chars, false, false, &mut vu0);
        assert_eq!(s.count(), 3);
        // Past the start delay: the opponent's technique ends them.
        s.update(&cfg, &chars, false, false, &mut vu0);
        assert_eq!(s.count(), 0);
    }
}
