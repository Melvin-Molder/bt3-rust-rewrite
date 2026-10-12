//! The body lightning (`EftBolt*`, src/battle/eft_aura.c and src/battle/eft_rays.c): while a fighter's parameter
//! flags have bit 4 or 8 a task of its own throws bolts between pairs of model nodes, three at a time, alternating
//! between the two sides of the body, each with a violet flash at its root. A bolt is a chain of at most ten
//! joints from a pool of 100 per fighter that both fighters share; it is revealed over a frame, flashes, then is
//! released joint by joint from its root (each drifting and fading) and freed after its 30 frames.
//!
//! The node pairs are common effect entry 4 (`EftBoltPair[8]`), the bolt texture entry 5, the flash sheet entry 6
//! (2 x 2 cells). How often bolts come depends on the aura (`EftAura_IsVisible`, `EftAura_IsBurstRising`) and the
//! power-up glow (`EftGlow_IsActive`), read as they stand when the lightning updates: after the aura's update and
//! before the glow's (layer 3 slots 1, 2, 3).

use crate::battle::IDENTITY;
use crate::eft::{
    DrawCtx, EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, OrderTable, SCREEN_DIST, V3,
    V4, add, cross, depth_slot, dot, norm, scale3, sub,
};
use crate::eft_ptcl::{rot_x, rot_y, rot_z};
use crate::rng::LibcRandom;

/// `EFT_BOLT_COUNT` / `EFT_BOLT_FLASHES`.
const BOLTS: usize = 10;
const FLASHES: usize = 20;
/// `EftBoltMgr_Init`: joints per fighter.
const SEGS_PER: usize = 100;

const PI_F: f32 = 3.141_592_7;
const TWO_PI: f32 = 6.283_185_5;

/// `RANDF`.
fn randf(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// `EFT_DEG`.
fn deg(x: f32) -> f32 {
    x / 180.0 * PI_F
}

/// `EftBoltPair`: a pair of model nodes a bolt can run between.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct BoltPair {
    pub node_a: i32,
    pub node_b: i32,
    pub length: f32,
    /// How far the origin moves from node A towards node B: `follow + follow_range * RANDF()`.
    pub follow: f32,
    pub follow_range: f32,
}

/// Common effect entry 4: the eight node pairs (rows 0..3 the body's middle, 4..7 the limbs).
#[derive(Clone, Debug, Default)]
pub(crate) struct BoltCfg {
    pub pairs: [BoltPair; 8],
}

impl BoltCfg {
    pub(crate) fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 8 * 0x14 {
            return None;
        }
        let i = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Some(Self {
            pairs: std::array::from_fn(|k| {
                let o = k * 0x14;
                BoltPair {
                    node_a: i(o),
                    node_b: i(o + 4),
                    length: f(o + 8),
                    follow: f(o + 0xC),
                    follow_range: f(o + 0x10),
                }
            }),
        })
    }
}

/// The lightning's textures among the effect textures: entry 5's first (the bolts), entry 6's first (the
/// flashes; only kind 0 is ever started).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BoltTextures {
    pub bolt: Option<usize>,
    pub flash: Option<usize>,
}

/// What the lightning reads of a fighter.
#[derive(Clone, Copy)]
pub(crate) struct BoltChr<'a> {
    pub side: usize,
    pub height: f32,
    /// `BtlCharApi_ObjGetParamFlags0` (the parameter's chara flags).
    pub param_flags: u32,
    /// `BtlCharApi_IsModelNew`.
    pub model_new: bool,
    /// `EftAura_IsVisible` / `EftAura_IsBurstRising` / `EftGlow_IsActive`.
    pub aura_visible: bool,
    pub burst_rising: bool,
    pub glow_active: bool,
    pub node_pos: &'a dyn Fn(u32) -> V3,
}

impl BoltChr<'_> {
    /// `BtlCharApi_GetNodePos`.
    fn node(&self, n: i32) -> V3 {
        (self.node_pos)(n as u32)
    }
}

/// `EftBoltSeg`: a joint.
#[derive(Clone, Copy, Debug, Default)]
struct Seg {
    /// 0 free; 1 used, 2 shown, 4 first flash frame done, 8 flash over, 0x10 released, 0x20 thick.
    flags: u32,
    width_a: f32,
    width_b: f32,
    alpha_max: f32,
    /// Relative to the bolt's origin.
    pos: V3,
    rgb: V3,
    alpha: f32,
    next: Option<usize>,
}

/// `EftBolt`.
#[derive(Clone, Copy, Debug, Default)]
struct Bolt {
    head: Option<usize>,
    shown: i32,
    count: i32,
    /// 0 free; 1 active, 2 fully shown, 4 finished.
    flags: u32,
    kind: usize,
    node_a: i32,
    node_b: i32,
    life: f32,
    follow: f32,
    pos: V3,
    start: V3,
    end: V3,
    dir: V3,
    normal: V3,
}

/// `EftBoltFlash`.
#[derive(Clone, Copy, Debug, Default)]
struct Flash {
    flags: u32,
    node_a: i32,
    node_b: i32,
    frame: i32,
    kind: usize,
    life: f32,
    size: f32,
    size_rate: f32,
    alpha_rate: f32,
    follow: f32,
    rot: f32,
    pos: V3,
    vel: V3,
    rgba: V4,
    uv: V4,
}

/// `EftBoltWork`: one fighter's task.
#[derive(Clone, Debug, Default)]
struct Work {
    chr: usize,
    /// 1 alive, 2 stop asked, 4 dead, 8 the model changed.
    flags: u32,
    bolts: [Bolt; BOLTS],
    flashes: [Flash; FLASHES],
    param_flags: u32,
    side: usize,
    bolt_count: i32,
    order_pos: usize,
    order: [usize; 4],
    angle: [f32; 2],
    scale: f32,
}

impl Work {
    /// `EftBolt_ReadChar`.
    fn read_char(&mut self, c: &BoltChr) {
        self.param_flags = c.param_flags;
        self.scale = c.height / 19.35;
    }
}

/// `gEftBoltPool`: the joint pool and the tasks in the order they were created.
#[derive(Clone, Debug)]
pub(crate) struct Bolts {
    segs: Vec<Seg>,
    seg_next: usize,
    tasks: Vec<Work>,
    /// `EftAuraMgr`'s `started`: the lightning was set up for the fighters.
    started: bool,
}

impl Default for Bolts {
    fn default() -> Self {
        Self {
            segs: vec![Seg::default(); 2 * SEGS_PER],
            seg_next: 0,
            tasks: Vec::new(),
            started: false,
        }
    }
}

impl Bolts {
    /// Live bolts and flashes of the fighter, and whether it has a task.
    #[cfg(test)]
    pub(crate) fn state(&self, chr: usize) -> (usize, usize, bool) {
        self.tasks
            .iter()
            .find(|w| w.chr == chr)
            .map_or((0, 0, false), |w| {
                (
                    w.bolts.iter().filter(|b| b.flags != 0).count(),
                    w.flashes.iter().filter(|f| f.flags != 0).count(),
                    true,
                )
            })
    }

    /// Joints in use.
    #[cfg(test)]
    fn segs_used(&self) -> usize {
        self.segs.iter().filter(|s| s.flags != 0).count()
    }

    /// `EftAuraMgr_Update`'s lightning part: on the first frame, and whenever a fighter's model changed,
    /// `EftAura_UpdateLightning` enables it with parameter flag 4 or 8, else disables it. (The stage of the port is
    /// always ready, so the first frame is the only one.)
    pub(crate) fn manage(&mut self, chars: &[BoltChr; 2]) {
        for c in chars {
            if !self.started || c.model_new {
                if c.param_flags & 0xC != 0 {
                    self.enable(c);
                } else {
                    self.disable(c.side);
                }
            }
        }
        self.started = true;
    }

    /// `EftBolt_Enable`: a task for the fighter unless it has one (one that is stopping keeps stopping).
    fn enable(&mut self, c: &BoltChr) {
        if self.tasks.iter().any(|w| w.chr == c.side) {
            return;
        }
        // EftBoltTask_Init.
        let mut w = Work {
            chr: c.side,
            flags: 1,
            ..Work::default()
        };
        w.read_char(c);
        self.tasks.push(w);
    }

    /// `EftBolt_Disable`: no new bolts; the task dies when the live ones are gone.
    fn disable(&mut self, chr: usize) {
        if let Some(w) = self.tasks.iter_mut().find(|w| w.chr == chr)
            && w.flags & 1 != 0
        {
            w.flags |= 2;
        }
    }

    /// `EftBoltTask_Update` of every task: the fighter again after a model change; unless paused new bolts (not
    /// once stopping), the bolts and the flashes; a stopping task dies when none is left.
    pub(crate) fn update(
        &mut self,
        cfg: &BoltCfg,
        chars: &[BoltChr; 2],
        paused: bool,
        libc: &mut LibcRandom,
    ) {
        let mut t = 0;
        while t < self.tasks.len() {
            let c = &chars[self.tasks[t].chr];
            if c.model_new {
                self.tasks[t].flags |= 8;
            }
            if self.tasks[t].flags & 8 != 0 {
                self.tasks[t].read_char(c);
                self.tasks[t].flags &= !8;
            }
            if !paused {
                if self.tasks[t].flags & 2 == 0 {
                    self.spawn(t, cfg, c, libc);
                }
                self.update_all(t, c, libc);
                update_flashes(&mut self.tasks[t], c);
                let w = &mut self.tasks[t];
                if w.flags & 2 != 0
                    && !w.bolts.iter().any(|b| b.flags != 0)
                    && !w.flashes.iter().any(|f| f.flags != 0)
                {
                    w.flags |= 4;
                }
            }
            if self.tasks[t].flags & 4 != 0 {
                // EftBoltTask_Term: the first bolt's joints only.
                let head = self.tasks[t].bolts[0].head.take();
                self.free_segs(head);
                self.tasks.remove(t);
            } else {
                t += 1;
            }
        }
    }

    /// `EftBolt_AllocSegs`: up to `count` free joints, searching once around the ring from where the last search
    /// stopped, chained into the bolt. They are taken only when the bolt is shaped.
    fn alloc_segs(&mut self, bolt: &mut Bolt, count: i32) {
        let max = self.segs.len();
        let mut prev: Option<usize> = None;
        let mut got = 0;
        let mut idx = self.seg_next;
        for _ in 0..max {
            if self.segs[idx].flags == 0 {
                self.segs[idx].next = None;
                match prev {
                    None => bolt.head = Some(idx),
                    Some(p) => self.segs[p].next = Some(idx),
                }
                prev = Some(idx);
                self.seg_next = idx + 1;
                if self.seg_next >= max {
                    self.seg_next = 0;
                }
                got += 1;
                if got >= count {
                    break;
                }
            }
            idx += 1;
            if idx >= max {
                idx = 0;
            }
        }
        bolt.count = got;
        bolt.shown = 0;
    }

    /// `EftBolt_FreeSegs`.
    fn free_segs(&mut self, head: Option<usize>) {
        let mut seg = head;
        while let Some(i) = seg {
            seg = self.segs[i].next;
            self.segs[i].flags = 0;
        }
    }

    fn chain(&self, head: Option<usize>) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(head, |&i| self.segs[i].next)
    }

    /// `EftBolt_Shape`: the joints between `start` and `end` (world points; the joints are kept relative to
    /// `start`): a random zigzag across the bolt's normal that bulges along it, with colour, widths and alpha.
    fn shape(&mut self, bolt: &Bolt, start: V3, end: V3, scale: f32, libc: &mut LibcRandom) {
        let mut amp = 1.0f32;
        let r1 = (randf(libc) + amp) * scale;
        let r2 = (randf(libc) + amp) * scale;
        let mut count = bolt.count;
        let dir = norm(sub(end, start));
        let side = cross(dir, bolt.normal);
        let up = cross(side, dir);
        let base = start;
        let bright = 1.0f32;
        amp = scale;
        let p1 = add(end, scale3(up, r1));
        let mut off = scale3(up, r2);
        let mut thick = 1;
        let mut n = 0;
        let mut link = bolt.head;
        while let Some(i) = link {
            self.segs[i].flags |= 1;
            if n == 0 {
                let a = randf(libc);
                let r = (a - randf(libc)) * 0.2 * scale;
                off = add(off, scale3(side, r));
            } else {
                count -= 1;
                let r = (randf(libc) * 0.5 + 0.3) * amp;
                amp = -amp;
                if count <= 0 {
                    count = 1;
                }
                off = add(off, scale3(side, r));
                let step = sub(p1, add(base, off)).map(|x| x / count as f32);
                // Integer quotients, as the original divides.
                let mut t = (1.0 - ((count - 1) / bolt.count) as f32) * 0.6 + 0.4;
                t *= randf(libc) * 0.2 + 0.85;
                off = add(off, scale3(step, t));
            }
            let seg = &mut self.segs[i];
            seg.pos = off;
            let t = (count / bolt.count) as f32;
            let r = (randf(libc) * 0.5 + 0.3) * t * amp;
            off = add(off, scale3(side, r));
            let r = (randf(libc) * 0.4 + 0.1) * t * amp;
            off = add(off, scale3(up, r));
            let r = (randf(libc) * 0.4 + 0.6) * scale;
            off = add(off, scale3(up, r));
            seg.rgb = [
                bright * 50.0 / 255.0,
                bright * 30.0 / 255.0,
                bright * 150.0 / 255.0,
            ];
            seg.alpha_max = 150.0 / 255.0;
            seg.alpha = 0.0;
            if n == 0 || seg.next.is_none() {
                seg.width_a = 0.02;
                seg.width_b = 0.02;
            } else {
                thick += 1;
                seg.flags |= 0x20;
                if thick == 1 {
                    seg.width_a = randf(libc) * 0.03 + 0.05;
                    seg.width_b = randf(libc) * 0.03 + 0.05;
                } else {
                    seg.width_a = randf(libc) * 0.05 + 0.11;
                    seg.width_b = randf(libc) * 0.05 + 0.11;
                }
                if thick >= 3 {
                    thick = 0;
                }
            }
            n += 1;
            seg.width_a *= scale * 3.0;
            seg.width_b *= scale * 3.0;
            link = seg.next;
        }
    }

    /// `EftBolt_Step`: reveals the joints (from the root; once the bolt is fully shown they are released from
    /// the root on), gives each a two-frame flash, then lets the released ones drift and fade. True when the last
    /// joint has faded.
    fn step(&mut self, bolt: &Bolt, scale: f32, libc: &mut LibcRandom) -> bool {
        let mut done = false;
        let mut n = 0;
        let mut link = bolt.head;
        while let Some(i) = link {
            let next = self.segs[i].next;
            let next_pos = next.map(|j| self.segs[j].pos);
            let seg = &mut self.segs[i];
            if bolt.flags & 2 != 0 {
                seg.flags |= 2;
                if seg.flags & 0x10 == 0 && n <= bolt.count - bolt.shown {
                    seg.flags |= 0x10;
                }
            } else if n <= bolt.shown {
                seg.flags |= 2;
            }
            let flags = seg.flags;
            if flags & 8 == 0 {
                if flags & 2 != 0 {
                    let k = if flags & 4 == 0 {
                        seg.alpha = seg.alpha_max * 0.35;
                        seg.flags = flags | 4;
                        if bolt.kind < 4 { 5.5 } else { 2.75 }
                    } else {
                        seg.alpha = seg.alpha_max;
                        seg.flags = flags | 8;
                        if bolt.kind < 4 { 0.3 } else { 0.61 }
                    };
                    if flags & 0x20 != 0 {
                        seg.width_a *= k;
                        seg.width_b *= k;
                    }
                }
            } else if flags & 0x10 != 0 {
                seg.width_a *= 0.8;
                seg.width_b *= 0.8;
                let d = match next_pos {
                    Some(p) => sub(p, seg.pos),
                    None => {
                        let t = randf(libc) * 0.45;
                        let mut d = scale3(bolt.dir, t);
                        let a = randf(libc) * 0.35;
                        let t = a - randf(libc) * 0.35 - 0.3;
                        d = add(d, scale3(bolt.normal, t));
                        scale3(norm(d), scale * 1.1)
                    }
                };
                seg.pos = add(seg.pos, d);
                seg.alpha -= seg.alpha_max * 0.25;
                let limit = seg.alpha_max * 0.1;
                if seg.alpha <= limit {
                    seg.alpha = limit;
                    if n >= bolt.count - 1 {
                        done = true;
                    }
                }
            }
            link = next;
            n += 1;
        }
        done
    }

    /// `EftBolt_Start`: a bolt between two model nodes, its ends along the two directions, and a chain of joints
    /// sized to its length (3..10). False when fewer than 3 joints were free (the bolt stays active, empty).
    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        w: &mut Work,
        b: usize,
        c: &BoltChr,
        kind: usize,
        node_a: i32,
        node_b: i32,
        dir_a: V3,
        dir_b: V3,
        length: f32,
        follow: f32,
        libc: &mut LibcRandom,
    ) -> bool {
        let mut bolt = w.bolts[b];
        bolt.kind = kind;
        bolt.node_b = node_b;
        bolt.flags |= 1;
        bolt.node_a = node_a;
        bolt.life = 30.0;
        bolt.pos = c.node(node_a);
        // The original's z is 1, not along the direction.
        bolt.start = [dir_a[0] * length, dir_a[1] * length, 1.0];
        bolt.end = [dir_b[0] * length, dir_b[1] * length, 1.0];
        bolt.follow = follow;
        let tmp = sub(c.node(bolt.node_b), bolt.pos);
        let o = add(bolt.pos, scale3(tmp, bolt.follow));
        let p0 = add(o, bolt.start);
        let p1 = add(o, bolt.end);
        let d = sub(p1, p0);
        bolt.dir = norm(d);
        bolt.normal = norm(scale3(add(dir_a, dir_b), 0.5));
        let t = w.scale * 0.6;
        bolt.start = sub(bolt.start, scale3(bolt.normal, t));
        bolt.end = sub(bolt.end, scale3(bolt.normal, t));
        let n = ((dot(d, d).sqrt() / w.scale + 3.0) as i32).min(10);
        self.alloc_segs(&mut bolt, n);
        let ok = bolt.count >= 3;
        if ok {
            self.shape(&bolt, p0, p1, w.scale, libc);
            w.bolt_count += 1;
        } else {
            self.free_segs(bolt.head.take());
        }
        w.bolts[b] = bolt;
        ok
    }

    /// `EftBolt_UpdateAll`: every live bolt follows its nodes and steps its joints; it is revealed in one frame,
    /// then retracts by 2 or 3 joints a frame, and is freed the frame after it finished (faded or 30 frames).
    fn update_all(&mut self, t: usize, c: &BoltChr, libc: &mut LibcRandom) {
        let scale = self.tasks[t].scale;
        for i in 0..BOLTS {
            let mut bolt = self.tasks[t].bolts[i];
            if bolt.flags & 1 == 0 {
                continue;
            }
            if bolt.flags & 4 == 0 {
                bolt.pos = c.node(bolt.node_a);
                let tmp = sub(c.node(bolt.node_b), bolt.pos);
                bolt.pos = add(bolt.pos, scale3(tmp, bolt.follow));
                if self.step(&bolt, scale, libc) {
                    bolt.flags |= 4;
                }
                if bolt.flags & 2 == 0 && bolt.shown < bolt.count {
                    bolt.shown += bolt.count;
                    if bolt.shown >= bolt.count {
                        bolt.shown = bolt.count;
                        bolt.flags |= 2;
                    }
                }
                if bolt.flags & 2 != 0 && bolt.shown > 0 {
                    bolt.shown = (bolt.shown - libc.rand() % 2 - 2).max(0);
                }
                bolt.life -= 1.0;
                if bolt.life <= 0.0 {
                    bolt.flags |= 4;
                }
            } else {
                self.free_segs(bolt.head.take());
                bolt.flags = 0;
                self.tasks[t].bolt_count -= 1;
            }
            self.tasks[t].bolts[i] = bolt;
        }
    }

    /// `EftBolt_Spawn`: whether bolts come this frame (always with none alive or a rising aura burst; else 1 in
    /// 12 with a visible aura, 1 in 6 with the glow, 1 in 18 without; parameter flag 8 another 1 in 20), then up
    /// to three, alternating sides: the first of a shuffled 0..3 order, then two limb pairs of the side.
    fn spawn(&mut self, t: usize, cfg: &BoltCfg, c: &BoltChr, libc: &mut LibcRandom) {
        let mut w = std::mem::take(&mut self.tasks[t]);
        let mut count = 0;
        let mut side = 0;
        let mut spawned = false;
        let mut kinds = [0usize; 3];
        if c.aura_visible {
            if c.burst_rising || w.bolt_count <= 0 || libc.rand() % 12 == 0 {
                count = 3;
            }
        } else if c.glow_active {
            if w.bolt_count <= 0 || libc.rand() % 6 == 0 {
                count = 3;
            }
        } else if w.bolt_count <= 0 || libc.rand() % 18 == 0 {
            count = 3;
        }
        if w.param_flags & 8 != 0 && libc.rand() % 20 == 0 {
            count = 3;
        }
        if count > 0 {
            spawned = true;
            side = w.side;
            w.side = side ^ 1;
            if w.order_pos == 0 {
                let mut pick = [0, 1, 2, 3];
                for left in (1..=4).rev() {
                    let n = (libc.rand() % left as i32) as usize;
                    w.order[4 - left] = pick[n];
                    pick.copy_within(n + 1..left, n);
                }
            }
            kinds = [
                w.order[w.order_pos],
                if side != 0 { 5 } else { 4 },
                if side != 0 { 6 } else { 7 },
            ];
        }
        let mut n = 0;
        while count != 0 {
            let Some(b) = w.bolts.iter().position(|b| b.flags == 0) else {
                break;
            };
            let kind = kinds[n % 3];
            n += 1;
            let mut ang = w.angle[side];
            let (d_ang, turn) = if side == 0 {
                let d = -(randf(libc) * 0.35 + 0.05) * PI_F;
                (d, -(randf(libc) * deg(36.0) + deg(90.0)))
            } else {
                let d = (randf(libc) * 0.35 + 0.05) * PI_F;
                (d, randf(libc) * deg(36.0) + deg(90.0))
            };
            let pair = cfg.pairs[kind];
            let follow = pair.follow + pair.follow_range * randf(libc);
            let length = if c.glow_active {
                pair.length * 1.4 * w.scale
            } else {
                pair.length * w.scale
            };
            let az = if kind < 4 {
                randf(libc) * 0.3 + -0.9
            } else {
                randf(libc) * 0.1 + -0.6
            };
            ang += d_ang;
            let a = norm([0.0, 1.0, az]);
            ang = wrap(ang);
            let dir_a = turn_z([a[0], a[1], a[2], 1.0], ang);
            let bz = if kind < 4 {
                0.4 - randf(libc) * 0.25
            } else {
                0.5 - randf(libc) * 0.2
            };
            ang += turn;
            let bv = norm([0.0, 1.0, bz]);
            ang = wrap(ang);
            let dir_b = turn_z([bv[0], bv[1], bv[2], 1.0], ang);
            let d = norm(sub(c.node(pair.node_a), c.node(pair.node_b)));
            let rx = d[1].asin();
            let mut ry = d[0].atan2(d[2]) + PI_F;
            if ry >= PI_F {
                ry -= TWO_PI;
            }
            let (s, co) = ry.sin_cos();
            let m = rot_y(&rot_x(&IDENTITY, rx), s, co);
            let dir_a = norm(v3(crate::battle::apply(&m, dir_a)));
            let dir_b = norm(v3(crate::battle::apply(&m, dir_b)));
            if !self.start(
                &mut w,
                b,
                c,
                kind,
                pair.node_a,
                pair.node_b,
                dir_a,
                dir_b,
                length,
                follow,
                libc,
            ) {
                break;
            }
            let vel = scale3(dir_a, length * 0.8);
            let size = if c.glow_active {
                (randf(libc) * 0.25 + 0.7) * 0.42 * w.scale * 1.4
            } else {
                (randf(libc) * 0.25 + 0.7) * 0.42 * w.scale
            };
            add_flash(&mut w, 0, pair.node_a, pair.node_b, vel, size, follow, libc);
            w.angle[side] = ang;
            count -= 1;
            side ^= 1;
        }
        if spawned {
            w.order_pos += 1;
            if w.order_pos >= 4 {
                w.order_pos = 0;
            }
        }
        self.tasks[t] = w;
    }

    /// `EftBoltTask_Draw` of every task (not of a hidden fighter): `EftBolt_DrawAll`, then
    /// `EftBolt_DrawFlashes`.
    pub(crate) fn draw(
        &self,
        tex: &BoltTextures,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
        shown: impl Fn(usize) -> Option<f32>,
    ) {
        for w in &self.tasks {
            let Some(alpha) = shown(w.chr) else {
                continue;
            };
            for bolt in w.bolts.iter().filter(|b| b.flags & 1 != 0) {
                self.draw_bolt(tex.bolt, bolt, alpha, ot, ctx);
            }
            for f in w.flashes.iter().filter(|f| f.flags & 1 != 0) {
                let c = f.rgba;
                draw_rot(
                    ot,
                    ctx,
                    [
                        (c[0] * 255.0) as u32 as u8,
                        (c[1] * 255.0) as u32 as u8,
                        (c[2] * 255.0) as u32 as u8,
                        (c[3] * 255.0 * alpha) as u32 as u8,
                    ],
                    f.pos,
                    f.uv,
                    f.rot,
                    0x40,
                    (f.size * 4096.0) as u32,
                    tex.flash,
                );
            }
        }
    }

    /// `EftBolt_Draw`: for each shown joint with a successor a camera-facing quad from the edge the previous
    /// quad ended on to this joint's edge at the successor's widths (the first quad has no length: both its edges
    /// lie on the first joint), coloured per joint.
    fn draw_bolt(
        &self,
        texture: Option<usize>,
        bolt: &Bolt,
        alpha: f32,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
    ) {
        let org = add(bolt.pos, bolt.start);
        let mut first = true;
        let mut prev = [[0.0f32; 3]; 2];
        let mut n = 0i32;
        for i in self.chain(bolt.head) {
            let seg = &self.segs[i];
            let Some(next) = seg.next.map(|j| &self.segs[j]) else {
                continue;
            };
            if seg.flags & 2 == 0 {
                continue;
            }
            let p = add(org, seg.pos);
            let side = norm(cross(sub(next.pos, seg.pos), sub(p, ctx.eye)));
            let mut quad = [[0.0f32; 3]; 4];
            if first {
                quad[0] = add(p, scale3(side, seg.width_b));
                quad[1] = add(p, scale3(side, -seg.width_a));
            } else {
                quad[0] = prev[0];
                quad[1] = prev[1];
            }
            let n1 = n + 1;
            quad[2] = add(p, scale3(side, next.width_b));
            first = false;
            quad[3] = add(p, scale3(side, -next.width_a));
            prev = [quad[2], quad[3]];
            let (a, b) = ((n % 2) as f32, (n1 % 2) as f32);
            let uv = [[a, b], [a, a], [b, b], [b, a]];
            let mut scr = [[0i32; 4]; 4];
            let mut stq = [[0.0f32; 3]; 4];
            let mut clipped = false;
            for q in 0..4 {
                let s = crate::battle::apply(
                    &ctx.world2screen,
                    [quad[q][0], quad[q][1], quad[q][2], 1.0],
                );
                let iq = 1.0 / s[3];
                scr[q] = [
                    (s[0] * iq * 16.0) as i32,
                    (s[1] * iq * 16.0) as i32,
                    (s[2] * iq) as i32,
                    s[3] as i32,
                ];
                let r = 1.0 / scr[q][3] as f32;
                stq[q] = [uv[q][0] * r, uv[q][1] * r, r];
                if scr[q][0] as u32 > 0xFFF0 || scr[q][1] as u32 > 0xFFF0 || scr[q][2] < 0 {
                    clipped = true;
                    break;
                }
            }
            if !clipped {
                let col = |s: &Seg| {
                    [
                        (s.rgb[0] * 255.0) as i32 as u8,
                        (s.rgb[1] * 255.0) as i32 as u8,
                        (s.rgb[2] * 255.0) as i32 as u8,
                        (s.alpha * 255.0 * alpha) as i32 as u8,
                    ]
                };
                let cols = [col(seg), col(seg), col(next), col(next)];
                // The fourth term is corner 2's w: the original's typo for corner 3's z.
                let z = scr[0][2]
                    .wrapping_add(scr[1][2])
                    .wrapping_add(scr[2][2])
                    .wrapping_add(scr[2][3])
                    >> 10;
                let vert = |q: usize| EffectVertex {
                    position: [
                        scr[q][0] as f32 / 16.0 - OFFSET_X,
                        scr[q][1] as f32 / 16.0 - OFFSET_Y,
                        (scr[q][2] & 0xFF_FFFF) as f32,
                    ],
                    stq: stq[q],
                    rgba: cols[q],
                };
                for q in 0..2 {
                    ot.add(
                        depth_slot(z),
                        1,
                        EffectTri {
                            texture,
                            blend: EffectBlend::of(1, false),
                            clamp: false,
                            zwrite: false,
                            alpha_ref: 0,
                            vertices: [vert(q), vert(q + 1), vert(q + 2)],
                        },
                    );
                }
            }
            n = n1;
        }
    }
}

fn v3(v: V4) -> V3 {
    [v[0], v[1], v[2]]
}

/// Into [-pi, pi) one turn at a time, as `EftBolt_Spawn` wraps.
fn wrap(a: f32) -> f32 {
    if a >= PI_F {
        a - TWO_PI
    } else if a <= -PI_F {
        a + TWO_PI
    } else {
        a
    }
}

/// `Mtx_RotateZ(m, identity, a)`, then `Mtx_MulVec4` of `v`.
fn turn_z(v: V4, a: f32) -> V4 {
    crate::battle::apply(&rot_z(&IDENTITY, a), v)
}

/// `EftBolt_AddFlash`: a flash between two nodes, drifting by `vel`; kind 0 violet for 6 frames, the others
/// grey for 3. Nothing when all 20 are in use.
#[allow(clippy::too_many_arguments)]
fn add_flash(
    w: &mut Work,
    kind: usize,
    node_a: i32,
    node_b: i32,
    vel: V3,
    size: f32,
    follow: f32,
    libc: &mut LibcRandom,
) {
    let Some(f) = w.flashes.iter_mut().find(|f| f.flags == 0) else {
        return;
    };
    f.flags |= 1;
    f.kind = kind;
    if kind == 0 {
        f.life = 6.0;
        f.rgba = [50.0 / 255.0, 30.0 / 255.0, 150.0 / 255.0, 150.0 / 255.0];
    } else {
        f.life = 3.0;
        f.rgba = [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 96.0 / 255.0];
    }
    f.node_a = node_a;
    f.node_b = node_b;
    f.vel = vel;
    f.follow = follow;
    f.size = size;
    f.size_rate = -(size * 0.2) / f.life;
    f.alpha_rate = -(f.rgba[3] / (f.life + 1.0));
    f.frame = libc.rand() % 4;
    f.rot = randf(libc) * TWO_PI;
    if f.rot >= PI_F {
        f.rot -= TWO_PI;
    }
}

/// `EftBolt_UpdateFlashes`: each follows its nodes plus its drift, shrinks, fades and steps through the four
/// cells of its sheet.
fn update_flashes(w: &mut Work, c: &BoltChr) {
    for f in w.flashes.iter_mut().filter(|f| f.flags & 1 != 0) {
        let a = c.node(f.node_a);
        let tmp = sub(c.node(f.node_b), a);
        f.pos = add(add(a, scale3(tmp, f.follow)), f.vel);
        f.rgba[3] += f.alpha_rate;
        f.size += f.size_rate;
        if f.rgba[3] < 0.0 {
            f.rgba[3] = 0.0;
        }
        let u = (f.frame % 2) as f32 * 0.5;
        let v = (f.frame / 2) as f32 * 0.5;
        f.uv = [u, v, u + 0.5, v + 0.5];
        f.frame += 1;
        if f.frame >= 4 {
            f.frame = 0;
        }
        f.life -= 1.0;
        if f.life < 0.0 {
            f.flags = 0;
        }
    }
}

/// `EftSpr_DrawRot` (no offset, square, context 1, layer 1): a camera-facing sprite of half size `half` units
/// scaled by `size` / 4096 * the screen distance / the depth, rotated by `rot` in the screen plane (y times 7/6,
/// the pixel aspect); nothing below two units or with a corner off the GS area.
#[allow(clippy::too_many_arguments)]
fn draw_rot(
    ot: &mut OrderTable,
    ctx: &DrawCtx,
    rgba: [u8; 4],
    pos: V3,
    uv: V4,
    rot: f32,
    half: u32,
    size: u32,
    texture: Option<usize>,
) {
    let v = crate::battle::apply(&ctx.world2screen, [pos[0], pos[1], pos[2], 1.0]);
    let q = 1.0 / v[3];
    let (x, y, z, sw) = (
        (v[0] * q * 16.0) as i32,
        (v[1] * q * 16.0) as i32,
        (v[2] * q) as i32,
        v[3] as i32,
    );
    let k = (size as f32 * SCREEN_DIST / sw as f32) as u32;
    let w = half.wrapping_mul(k) >> 12;
    if w < 2 {
        return;
    }
    let rot = if rot > PI_F {
        rot - TWO_PI
    } else if rot < -PI_F {
        rot + TWO_PI
    } else {
        rot
    };
    let m = rot_z(&IDENTITY, rot);
    let w = w as f32;
    let corner = |cx: f32, cy: f32| {
        let p = crate::battle::apply(&m, [cx, cy, 0.0, 1.0]);
        (x + p[0] as i32, y + (p[1] * 1.166_666_6) as i32)
    };
    let corners = [corner(-w, -w), corner(-w, w), corner(w, -w), corner(w, w)];
    let off = |x: i32, y: i32| z <= 0 || x > 0xFFEF || x <= 0 || y > 0xFFEF || y <= 0;
    if corners.iter().any(|&(x, y)| off(x, y)) {
        return;
    }
    let st = [
        [uv[0], uv[1]],
        [uv[0], uv[3]],
        [uv[2], uv[1]],
        [uv[2], uv[3]],
    ];
    let vert = |k: usize| EffectVertex {
        position: [
            corners[k].0 as f32 / 16.0 - OFFSET_X,
            corners[k].1 as f32 / 16.0 - OFFSET_Y,
            (z & 0xFF_FFFF) as f32,
        ],
        stq: [st[k][0], st[k][1], 1.0],
        rgba,
    };
    for k in 0..2 {
        ot.add(
            depth_slot(z >> 8),
            1,
            EffectTri {
                texture,
                blend: EffectBlend::of(1, false),
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

    fn cfg() -> BoltCfg {
        let mut c = BoltCfg::default();
        for (k, p) in c.pairs.iter_mut().enumerate() {
            *p = BoltPair {
                node_a: k as i32 + 1,
                node_b: k as i32 + 2,
                length: 3.2,
                follow: 0.5,
                follow_range: 0.7,
            };
        }
        c
    }

    fn node_pos(n: u32) -> V3 {
        [0.0, -(n as f32) * 3.0, 100.0]
    }

    fn chr(side: usize, flags: u32, node_pos: &dyn Fn(u32) -> V3) -> BoltChr<'_> {
        BoltChr {
            side,
            height: 19.35,
            param_flags: flags,
            model_new: false,
            aura_visible: false,
            burst_rising: false,
            glow_active: false,
            node_pos,
        }
    }

    #[test]
    fn the_pairs_read() {
        let mut b = vec![0u8; 8 * 0x14];
        b[0x14..0x18].copy_from_slice(&46i32.to_le_bytes());
        b[0x18..0x1C].copy_from_slice(&3i32.to_le_bytes());
        b[0x1C..0x20].copy_from_slice(&4.6f32.to_le_bytes());
        b[0x24..0x28].copy_from_slice(&0.4f32.to_le_bytes());
        let c = BoltCfg::parse(&b).unwrap();
        assert_eq!(
            c.pairs[1],
            BoltPair {
                node_a: 46,
                node_b: 3,
                length: 4.6,
                follow: 0.0,
                follow_range: 0.4
            }
        );
        assert!(BoltCfg::parse(&b[..0x90]).is_none());
    }

    #[test]
    fn bolts_come_in_threes_flash_retract_and_free_their_joints() {
        let cfg = cfg();
        let mut bolts = Bolts::default();
        let mut libc = LibcRandom::new(3);
        let chars = [chr(0, 4, &node_pos), chr(1, 0, &node_pos)];
        bolts.manage(&chars);
        // Only the fighter with parameter flag 4 gets a task.
        assert!(bolts.state(0).2);
        assert!(!bolts.state(1).2);
        bolts.update(&cfg, &chars, false, &mut libc);
        // None alive: three at once, each with a flash, alternating sides.
        let (live, flashes, _) = bolts.state(0);
        assert_eq!((live, flashes), (3, 3));
        let w = &bolts.tasks[0];
        assert_eq!(w.bolt_count, 3);
        assert_eq!(w.side, 1);
        assert_eq!(w.order_pos, 1);
        let mut order = w.order;
        order.sort();
        assert_eq!(order, [0, 1, 2, 3]);
        // The first is the shuffled body pair, then the side's two limb pairs.
        assert_eq!(w.bolts[0].kind, w.order[0]);
        assert_eq!((w.bolts[1].kind, w.bolts[2].kind), (4, 7));
        for b in &w.bolts[..3] {
            assert!((3..=10).contains(&b.count));
            // Fully shown on the first update, then retracting.
            assert_ne!(b.flags & 2, 0);
            assert!(b.shown < b.count);
            let n = bolts.chain(b.head).count() as i32;
            assert_eq!(n, b.count);
        }
        let used = bolts.segs_used();
        assert_eq!(used as i32, w.bolts.iter().map(|b| b.count).sum::<i32>());
        // A paused frame changes nothing.
        let before = bolts.tasks[0].bolts[0].shown;
        bolts.update(&cfg, &chars, true, &mut libc);
        assert_eq!(bolts.tasks[0].bolts[0].shown, before);
        // Stopped: no new bolts; the live ones finish within their 30 frames, then the task dies and every
        // joint is free.
        let chars = [chr(0, 0, &node_pos), chr(1, 0, &node_pos)];
        let mut new = chars;
        new[0].model_new = true;
        bolts.manage(&new);
        assert_ne!(bolts.tasks[0].flags & 2, 0);
        let mut frames = 0;
        while bolts.state(0).2 {
            bolts.update(&cfg, &chars, false, &mut libc);
            frames += 1;
            assert!(frames <= 32, "the lightning outlived its bolts");
        }
        assert_eq!(bolts.segs_used(), 0);
    }

    #[test]
    fn a_joint_flashes_wide_then_settles_and_fades_once_released() {
        let mut bolts = Bolts::default();
        let mut libc = LibcRandom::new(5);
        let c = chr(0, 4, &node_pos);
        let mut w = Work {
            scale: 1.0,
            ..Work::default()
        };
        assert!(bolts.start(
            &mut w,
            0,
            &c,
            0,
            1,
            2,
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            8.0,
            0.5,
            &mut libc
        ));
        assert_eq!(w.bolt_count, 1);
        let bolt = w.bolts[0];
        let segs: Vec<usize> = bolts.chain(bolt.head).collect();
        // The end joints are thin, the inner ones thick.
        let first = bolts.segs[segs[0]];
        let inner = bolts.segs[segs[1]];
        assert_eq!(first.width_a, 0.02 * 3.0);
        assert_ne!(inner.flags & 0x20, 0);
        // Not shown yet: the first step shows the root joint at 0.35 alpha and 5.5 times as wide (a body pair).
        bolts.step(&bolt, 1.0, &mut libc);
        let s = bolts.segs[segs[0]];
        assert_eq!(s.alpha, s.alpha_max * 0.35);
        assert_eq!(bolts.segs[segs[1]].flags & 2, 0);
        let mut shown = bolt;
        shown.flags |= 2;
        shown.shown = shown.count;
        let w0 = bolts.segs[segs[1]].width_a;
        bolts.step(&shown, 1.0, &mut libc);
        assert_eq!(bolts.segs[segs[1]].width_a, w0 * 5.5);
        bolts.step(&shown, 1.0, &mut libc);
        assert_eq!(bolts.segs[segs[1]].width_a, w0 * 5.5 * 0.3);
        assert_eq!(bolts.segs[segs[1]].alpha, bolts.segs[segs[1]].alpha_max);
        // Released (shown 0): every joint drifts and fades to a tenth; done once the last has.
        shown.shown = 0;
        let mut done = false;
        for _ in 0..8 {
            done = bolts.step(&shown, 1.0, &mut libc);
        }
        assert!(done);
        let s = bolts.segs[segs[1]];
        assert_eq!(s.alpha, s.alpha_max * 0.1);
    }

    #[test]
    fn a_flash_follows_its_nodes_and_cycles_its_cells() {
        let mut w = Work::default();
        let mut libc = LibcRandom::new(1);
        let c = chr(0, 4, &node_pos);
        add_flash(&mut w, 0, 1, 3, [1.0, 0.0, 0.0], 2.0, 0.5, &mut libc);
        let f0 = w.flashes[0].frame;
        update_flashes(&mut w, &c);
        let f = &w.flashes[0];
        assert_eq!(f.pos, [1.0, -6.0, 100.0]);
        let (u, v) = ((f0 % 2) as f32 * 0.5, (f0 / 2) as f32 * 0.5);
        assert_eq!(f.uv, [u, v, u + 0.5, v + 0.5]);
        assert_eq!(f.frame, (f0 + 1) % 4);
        // 6 frames of life: it ends on the seventh update.
        for _ in 0..5 {
            update_flashes(&mut w, &c);
        }
        assert_eq!(w.flashes[0].flags, 1);
        update_flashes(&mut w, &c);
        assert_eq!(w.flashes[0].flags, 0);
    }

    #[test]
    fn a_bolt_draws_in_front_of_the_camera() {
        use crate::battle::View;
        let cfg = cfg();
        let mut bolts = Bolts::default();
        let mut libc = LibcRandom::new(3);
        let chars = [chr(0, 4, &node_pos), chr(1, 0, &node_pos)];
        bolts.manage(&chars);
        bolts.update(&cfg, &chars, false, &mut libc);
        bolts.update(&cfg, &chars, false, &mut libc);
        let view = View::new([0.0, -10.0, 0.0], [0.0; 3]);
        let ctx = DrawCtx::new(view.world2screen, view.world2view);
        let tex = BoltTextures {
            bolt: Some(1),
            flash: Some(2),
        };
        let mut ot = OrderTable::new();
        bolts.draw(&tex, &mut ot, &ctx, |_| Some(1.0));
        let tris = ot.into_list();
        assert!(tris.iter().any(|t| t.texture == Some(1)));
        assert!(tris.iter().any(|t| t.texture == Some(2)));
        assert!(tris.iter().all(|t| t.blend == EffectBlend::Add));
        let mut ot = OrderTable::new();
        bolts.draw(&tex, &mut ot, &ctx, |_| None);
        assert!(ot.into_list().is_empty());
    }
}
