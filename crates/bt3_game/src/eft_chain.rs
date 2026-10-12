//! Effect pack part kind 18, the chain (`EftChain_*`, src/battle/eft_chain.c): bursts of up to 16 strands of nodes
//! radiating from a point, each a camera-facing ribbon that grows, crawls or drifts, pulses and fades, driven by a
//! three-key parameter block. The nodes come from a pool of 500 shared by every chain.

use bt3_data::eft_pack::{ChainKeys, ChainPrm};

use crate::battle::{IDENTITY, Mtx, apply};
use crate::eft::{
    ClipVtx, DrawCtx, OrderTable, Prim, SetRef, V3, V4, ZSort, add, cross, norm, point,
    queue_strip_colors, queue_tri_clip, rotate_xyz, scale3, sub, xyz,
};
use crate::rng::LibcRandom;

/// `EftChainMgr_Init`: ten chain tasks.
pub(crate) const CHAINS: usize = 10;
/// `EFT_ARC_NODES`: the shared node pool.
pub(crate) const CHAIN_NODES: usize = 500;
/// `EFT_ARC_CHAINS`: strands per chain.
const STRANDS: usize = 16;

const PI_F: f32 = 3.141_592_7;

// EFT_CHAIN_* (EFT_ARC_*).
const C_ALIVE: u32 = 0x1;
const C_ENDING: u32 = 0x2;
pub(crate) const C_DEAD: u32 = 0x4;
const C_ENDLESS: u32 = 0x8;
/// Drawn only in the view that shows its fighter, and in front of everything.
pub(crate) const C_FRONT: u32 = 0x10;
const C_ORIENTED: u32 = 0x20;

// EFT_ARCCH_*.
const S_ON: u32 = 0x1;
const S_GROWN: u32 = 0x2;
const S_SHRUNK: u32 = 0x4;
const S_DEAD: u32 = 0x8;
const S_FADED_IN: u32 = 0x10;
const S_FADED_OUT: u32 = 0x20;
const S_F40: u32 = 0x40;
const S_ENDING: u32 = 0x80;
const S_DRIFT: u32 = 0x100;
const S_MUL_DOWN: u32 = 0x200;
const S_PULSE_DOWN: u32 = 0x400;
const S_VISIBLE: u32 = 0x800;

/// `(f32)rand() / RAND_MAX`.
fn rand01(libc: &mut LibcRandom) -> f32 {
    libc.rand() as f32 / 2_147_483_647.0
}

/// `EftMath_WrapAngle`.
fn wrap(a: f32) -> f32 {
    const TWO_PI: f32 = 6.283_185_3;
    if a >= PI_F {
        let n = (((a - PI_F) / TWO_PI) as i32).abs();
        a - (n as f32 * TWO_PI + TWO_PI)
    } else if a <= -PI_F {
        let n = (((a + PI_F) / TWO_PI) as i32).abs();
        a + (n as f32 * TWO_PI + TWO_PI)
    } else {
        a
    }
}

/// The shared node pool (`gEftChain`): which nodes are in use, and the round-robin cursor.
#[derive(Clone, Debug)]
pub(crate) struct NodePool {
    used: Vec<bool>,
    cursor: usize,
}

impl Default for NodePool {
    fn default() -> Self {
        Self {
            used: vec![false; CHAIN_NODES],
            cursor: 0,
        }
    }
}

impl NodePool {
    /// Nodes in use.
    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.used.iter().filter(|&&u| u).count()
    }
}

/// `EftArcKey`: the parameters in effect (one key, or the blend of two).
#[derive(Clone, Copy, Debug, Default)]
struct Key {
    col_base: V4,
    col_range: V4,
    col2_base: V4,
    col2_range: V4,
    mul_base: [f32; 3],
    mul_range: [f32; 3],
    mul_time: f32,
    spawn_wait: f32,
    f60: f32,
    f64: f32,
    f68: f32,
    f6c: f32,
    f70: f32,
    f74: f32,
    node_count: f32,
    grow_rate: f32,
    shrink_rate: f32,
    life_base: f32,
    life_range: f32,
    f8c: f32,
    width_base: f32,
    width_range: f32,
    pulse_base: f32,
    pulse_range: f32,
    pulse_time: f32,
    fa4: f32,
    end_width: f32,
    end_alpha: f32,
    rad_base: f32,
    rad_range: f32,
    rad2_base: f32,
    rad2_range: f32,
    jitter: f32,
    turn_base: f32,
    turn_range: f32,
    step_x_base: f32,
    step_x_range: f32,
    step_y_base: f32,
    step_y_range: f32,
    crawl_x_base: f32,
    crawl_x_range: f32,
    crawl_y_base: f32,
    crawl_y_range: f32,
    fade_in: f32,
    fade_out: f32,
}

/// `EftChain_SetKey`: key `k` of the parameters; seconds become frames, the angle rows are scaled by pi.
fn key_of(p: &ChainPrm, keys: &ChainKeys, k: usize) -> Key {
    let r = |n: usize| p.rows[n][k];
    let mul = |c: usize| keys.mul[c][k];
    Key {
        col_base: keys.col_base[k],
        col_range: keys.col_range[k],
        col2_base: keys.col2_base[k],
        col2_range: keys.col2_range[k],
        mul_base: [mul(0)[0], mul(1)[0], mul(2)[0]],
        mul_range: [
            mul(0)[1] - mul(0)[0],
            mul(1)[1] - mul(1)[0],
            mul(2)[1] - mul(2)[0],
        ],
        mul_time: keys.mul_time[k] * 30.0,
        spawn_wait: r(0) * 30.0,
        f60: r(1) * PI_F,
        f64: r(2) * PI_F,
        f68: r(3) * PI_F,
        f6c: r(4) * PI_F,
        f70: r(5) * PI_F,
        f74: r(6) * PI_F,
        node_count: r(7),
        grow_rate: r(8),
        shrink_rate: r(9),
        life_base: r(10) * 30.0,
        life_range: r(11) * 30.0,
        f8c: r(12),
        width_base: r(13),
        width_range: r(14),
        pulse_base: r(15),
        pulse_range: r(16) - r(15),
        pulse_time: r(17) * 30.0,
        fa4: r(18),
        end_width: r(19),
        end_alpha: r(20),
        rad_base: r(21),
        rad_range: r(22),
        rad2_base: r(23),
        rad2_range: r(24),
        jitter: r(25),
        turn_base: r(27),
        turn_range: r(28),
        step_x_base: r(29) * PI_F,
        step_x_range: r(30) * PI_F,
        step_y_base: r(31) * PI_F,
        step_y_range: r(32) * PI_F,
        crawl_x_base: r(33) * PI_F,
        crawl_x_range: r(34) * PI_F,
        crawl_y_base: r(35) * PI_F,
        crawl_y_range: r(36) * PI_F,
        fade_in: r(37) * 30.0,
        fade_out: r(38) * 30.0,
    }
}

/// `EftChain_BlendKeys`: keys 0..1 up to `mid`, then 1..2. As in the original, rows 2, 4 and 6 take the delta of
/// the row before them, row 20 the delta of row 9, and the step and crawl rows are not scaled by pi.
fn blend_keys(p: &ChainPrm, keys: &ChainKeys, time: f32, mid: f32, end: f32) -> Key {
    let (t, k0, k1) = if time < mid {
        (time / mid, 0, 1)
    } else {
        ((time - mid) / (end - mid), 1, 2)
    };
    let v4 =
        |a: &[[f32; 4]; 3]| -> V4 { std::array::from_fn(|i| a[k0][i] + (a[k1][i] - a[k0][i]) * t) };
    let r0 = |n: usize| p.rows[n][k0];
    let d = |n: usize| p.rows[n][k1] - p.rows[n][k0];
    let lerp = |n: usize| r0(n) + d(n) * t;
    let mul: [[f32; 2]; 3] = std::array::from_fn(|c| {
        std::array::from_fn(|j| keys.mul[c][k0][j] + (keys.mul[c][k1][j] - keys.mul[c][k0][j]) * t)
    });
    Key {
        col_base: v4(&keys.col_base),
        col_range: v4(&keys.col_range),
        col2_base: v4(&keys.col2_base),
        col2_range: v4(&keys.col2_range),
        mul_base: [mul[0][0], mul[1][0], mul[2][0]],
        mul_range: [
            mul[0][1] - mul[0][0],
            mul[1][1] - mul[1][0],
            mul[2][1] - mul[2][0],
        ],
        mul_time: (keys.mul_time[k0] + (keys.mul_time[k1] - keys.mul_time[k0]) * t) * 30.0,
        spawn_wait: lerp(0) * 30.0,
        f60: lerp(1) * PI_F,
        f64: (r0(2) + d(1) * t) * PI_F,
        f68: lerp(3) * PI_F,
        f6c: (r0(4) + d(3) * t) * PI_F,
        f70: lerp(5) * PI_F,
        f74: (r0(6) + d(5) * t) * PI_F,
        node_count: lerp(7),
        grow_rate: lerp(8),
        shrink_rate: lerp(9),
        life_base: lerp(10) * 30.0,
        life_range: lerp(11) * 30.0,
        f8c: lerp(12),
        width_base: lerp(13),
        width_range: lerp(14),
        pulse_base: lerp(15),
        pulse_range: lerp(16) - lerp(15),
        pulse_time: lerp(17) * 30.0,
        fa4: lerp(18),
        end_width: lerp(19),
        end_alpha: r0(20) + d(9) * t,
        rad_base: lerp(21),
        rad_range: lerp(22),
        rad2_base: lerp(23),
        rad2_range: lerp(24),
        jitter: lerp(25),
        turn_base: lerp(27),
        turn_range: lerp(28),
        step_x_base: lerp(29),
        step_x_range: lerp(30),
        step_y_base: lerp(31),
        step_y_range: lerp(32),
        crawl_x_base: lerp(33),
        crawl_x_range: lerp(34),
        crawl_y_base: lerp(35),
        crawl_y_range: lerp(36),
        fade_in: lerp(37) * 30.0,
        fade_out: lerp(38) * 30.0,
    }
}

/// `EftArcNode`.
#[derive(Clone, Copy, Debug, Default)]
struct Node {
    /// Its entry in the pool.
    slot: usize,
    pos: V3,
    jit: V3,
    world: V3,
    color: V4,
    /// 1 in use, 2 shown.
    flags: u32,
    width_a: f32,
    width_b: f32,
    radius: f32,
    amp: f32,
}

/// `EftArcChain`: one strand.
#[derive(Clone, Debug, Default)]
struct Strand {
    /// Added to every node (never set: zero).
    pos: V3,
    drift: V3,
    color: V4,
    col_a: V4,
    col_b: V4,
    nodes: Vec<Node>,
    flags: u32,
    shown: f32,
    time: f32,
    life: f32,
    shrink_at: f32,
    f7c: f32,
    end_at: f32,
    fade_at: f32,
    rot_x: f32,
    rot_y: f32,
    rot_z: f32,
    step_x: f32,
    step_y: f32,
    crawl_x: f32,
    crawl_y: f32,
    width: f32,
    alpha: f32,
    fade_in_t: f32,
    fade_out_t: f32,
    fade_in: f32,
    fade_out: f32,
    pulse_t: f32,
    mul_t: f32,
}

impl Strand {
    /// `EftChain_AllocNodes`: up to `count` free nodes of the pool, round robin from its cursor.
    fn alloc(&mut self, pool: &mut NodePool, count: i32) {
        self.nodes.clear();
        let mut idx = pool.cursor;
        for _ in 0..CHAIN_NODES {
            if !pool.used[idx] {
                self.nodes.push(Node {
                    slot: idx,
                    ..Node::default()
                });
                pool.cursor = idx + 1;
                if pool.cursor >= CHAIN_NODES {
                    pool.cursor = 0;
                }
                if self.nodes.len() as i32 >= count {
                    break;
                }
            }
            idx += 1;
            if idx >= CHAIN_NODES {
                idx = 0;
            }
        }
    }

    /// `EftChain_FreeNodes`.
    fn free(&mut self, pool: &mut NodePool) {
        for n in self.nodes.drain(..) {
            pool.used[n.slot] = false;
        }
    }
}

/// A chain task (`EftArc` / `EftChainWork`).
#[derive(Clone, Debug)]
pub(crate) struct Chain {
    pub id: u32,
    chr: usize,
    pub set: SetRef,
    pub pair: usize,
    texture: Option<usize>,
    pos: V3,
    dir: V3,
    scale: f32,
    key: Key,
    strands: Vec<Strand>,
    /// The rotation that turns +Z onto the direction.
    mtx: Mtx,
    pub flags: u32,
    live: i32,
    slot: usize,
    node_count: i32,
    time: f32,
    life: f32,
    spawn_wait: f32,
    pub delay: f32,
    pub end_wait: f32,
    key_t: f32,
    key_end: f32,
    key_mid: f32,
    rot_x: [f32; 4],
    rot_y: [f32; 4],
    rot_z: f32,
}

/// `Mtx_RotateX(pitch)` then `Mtx_RotateY(yaw)` of a direction (`Mathf_Asin` clamps).
fn dir_mtx(dir: V3) -> Mtx {
    let pitch = (-dir[1]).clamp(-1.0, 1.0).asin();
    let yaw = dir[0].atan2(dir[2]);
    rotate_xyz(&rotate_xyz(&IDENTITY, [pitch, 0.0, 0.0]), [0.0, yaw, 0.0])
}

impl Chain {
    /// `EftChain_Init`: the matrix onto the direction, the four slot rotations, key 0 (keyed) or 2.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u32,
        chr: usize,
        set: SetRef,
        pair: usize,
        texture: Option<usize>,
        pos: V3,
        dir: V3,
        life: f32,
        scale: f32,
        p: &ChainPrm,
        keys: &ChainKeys,
    ) -> Self {
        let mut c = Self {
            id,
            chr,
            set,
            pair,
            texture,
            pos,
            dir,
            scale,
            key: Key::default(),
            strands: vec![Strand::default(); STRANDS],
            mtx: IDENTITY,
            flags: C_ALIVE,
            live: 0,
            slot: 0,
            node_count: 0,
            time: 0.0,
            life: 0.0,
            spawn_wait: 0.0,
            delay: 0.0,
            end_wait: 0.0,
            key_t: 0.0,
            key_end: 0.0,
            key_mid: 0.0,
            rot_x: [0.0, PI_F, 0.0, PI_F],
            rot_y: [0.0, 0.0, PI_F, PI_F],
            rot_z: 0.0,
        };
        if p.kind == 2 || p.flags & 0x10 != 0 {
            c.flags |= C_ORIENTED;
        }
        if life <= 0.0 {
            c.flags |= C_ENDLESS;
        } else {
            c.life = life * 30.0;
        }
        c.mtx = dir_mtx(dir.map(|v| v.clamp(-1.0, 1.0)));
        if p.flags & 1 != 0 {
            c.key_end = p.key_time * 30.0;
            c.key_mid = c.key_end * p.key_mid;
            c.key = key_of(p, keys, 0);
        } else {
            c.key = key_of(p, keys, 2);
        }
        c.node_count = c.key.node_count as i32;
        c
    }

    pub(crate) fn alive(&self) -> bool {
        self.flags & C_ALIVE != 0
    }

    pub(crate) fn chr(&self) -> usize {
        self.chr
    }

    /// `EftChain_Stop`: lets it finish.
    pub(crate) fn stop(&mut self) {
        self.flags |= C_ENDING;
    }

    /// `EftChain_SetPos`: strands already out stay where they are.
    pub(crate) fn set_pos(&mut self, pos: V3) {
        self.pos = pos;
    }

    /// `EftChain_Warp`: the origin, and every node of every strand placed again from it.
    pub(crate) fn warp(&mut self, pos: V3) {
        self.pos = pos;
        for s in self.strands.iter_mut().filter(|s| s.flags & S_ON != 0) {
            for n in &mut s.nodes {
                n.world = add(add(add(pos, s.pos), n.pos), n.jit);
            }
        }
    }

    /// `EftChain_SetDir`: the direction and its rotation.
    pub(crate) fn set_dir(&mut self, dir: V3) {
        self.dir = dir;
        self.mtx = dir_mtx(dir);
    }

    /// `EftChain_SetSize`.
    pub(crate) fn set_size(&mut self, size: f32) {
        self.scale = size;
    }

    /// `EftChain_SetNodeCount`: 3..7.
    pub(crate) fn set_node_count(&mut self, count: i32) {
        self.node_count = count.clamp(3, 7);
    }

    /// `EftChain_Term`: every strand's nodes back to the pool.
    pub(crate) fn free(&mut self, pool: &mut NodePool) {
        self.flags = 0;
        for s in &mut self.strands {
            s.free(pool);
            s.flags = 0;
        }
    }

    /// `EftChain_Update`: the start delay, bursts (until it ends), the strands, the key animation and the life;
    /// once ending, its end wait over and every strand gone, it dies.
    pub(crate) fn update(
        &mut self,
        p: &ChainPrm,
        keys: &ChainKeys,
        stopped: bool,
        pool: &mut NodePool,
        libc: &mut LibcRandom,
    ) {
        if !stopped {
            if self.delay <= 0.0 {
                if self.flags & C_ENDING == 0 || self.end_wait > 0.0 {
                    if self.spawn_wait <= 0.0 {
                        self.spawn(p, pool, libc);
                    } else {
                        self.spawn_wait -= 1.0;
                    }
                }
                self.update_strands(p, pool, libc);
                if p.flags & 1 != 0 && self.key_t <= self.key_end {
                    self.key = blend_keys(p, keys, self.key_t, self.key_mid, self.key_end);
                    self.key_t += 1.0;
                    if self.key_end <= self.key_t {
                        self.key = key_of(p, keys, 2);
                    }
                }
                if self.flags & C_ENDLESS == 0 {
                    self.time += 1.0;
                    if self.life <= self.time {
                        self.flags |= C_ENDING;
                    }
                }
            } else {
                self.delay -= 1.0;
            }
            if self.flags & C_ENDING != 0 {
                if self.end_wait > 0.0 {
                    self.end_wait -= 1.0;
                }
                if self.end_wait <= 0.0 && self.strands.iter().all(|s| s.flags == 0) {
                    self.flags |= C_DEAD;
                }
            }
        }
    }

    /// `EftChain_Spawn`: bursts while the spawn timer is due, each of `count_base` (+ a random part) strands in
    /// free slots; every strand turns the current slot's rotation (mirrored on odd slots) and steps the slot.
    fn spawn(&mut self, p: &ChainPrm, pool: &mut NodePool, libc: &mut LibcRandom) {
        if self.spawn_wait > 0.0 {
            return;
        }
        let mut full = false;
        // The original loops until the bursts are paid for or the strands are full; a set whose bursts cost nothing
        // and whose strands never start would hang it, so the bursts are capped.
        for _ in 0..CHAIN_NODES {
            let mut n = i32::from(p.count_base);
            if p.count_range >= 2 {
                n += libc.rand() % i32::from(p.count_range);
            }
            while n > 0 {
                full = true;
                let mut free = None;
                for i in 0..STRANDS {
                    if self.strands[i].flags == 0 {
                        self.strands[i] = Strand::default();
                        free = Some(i);
                        full = false;
                        break;
                    }
                }
                let Some(i) = free else {
                    break;
                };
                if !self.start_strand(i, p, pool, libc) {
                    break;
                }
                let a = self.key.f60 + self.key.f64 * rand01(libc);
                let b = self.key.f68 + self.key.f6c * rand01(libc);
                let s = self.slot;
                if s & 1 == 0 {
                    self.rot_x[s] += a;
                    self.rot_y[s] += b;
                } else {
                    self.rot_x[s] -= a;
                    self.rot_y[s] -= b;
                }
                self.rot_x[s] = wrap(self.rot_x[s]);
                self.rot_y[s] = wrap(self.rot_y[s]);
                self.slot = (self.slot + 1) % 4;
                n -= 1;
            }
            self.spawn_wait += self.key.spawn_wait;
            if full || self.spawn_wait > 0.0 {
                break;
            }
        }
    }

    /// `EftChain_StartStrand`: nodes from the pool (fewer than 3: given back), a random life and colours, the
    /// direction (kind 2: a random pitch and a yaw advancing per strand; else the slot's rotation), the turn steps
    /// mirrored on odd slots, the node layout, the crawl and the drift.
    fn start_strand(
        &mut self,
        i: usize,
        p: &ChainPrm,
        pool: &mut NodePool,
        libc: &mut LibcRandom,
    ) -> bool {
        let key = self.key;
        let ch = &mut self.strands[i];
        ch.alloc(pool, self.node_count);
        if ch.nodes.len() < 3 {
            ch.nodes.clear();
            return false;
        }
        for n in &ch.nodes {
            pool.used[n.slot] = true;
        }
        ch.flags |= S_ON;
        let life = key.life_base + key.life_range * rand01(libc);
        ch.time = 0.0;
        ch.life = life;
        ch.f7c = life * key.f8c;
        ch.end_at = life * key.fa4;
        ch.fade_at = life - key.fade_out;
        if key.shrink_rate <= ch.time {
            ch.flags |= S_SHRUNK;
        } else {
            ch.shrink_at = life - ch.nodes.len() as f32 / key.shrink_rate;
        }
        ch.shown = 0.0;
        ch.width = 1.0;
        let r = rand01(libc);
        ch.col_a = std::array::from_fn(|c| key.col_base[c] + key.col_range[c] * r);
        let r = rand01(libc);
        let col2: V4 = std::array::from_fn(|c| key.col2_base[c] + key.col2_range[c] * r);
        ch.col_b = std::array::from_fn(|c| col2[c] - ch.col_a[c]);
        ch.color = ch.col_a;
        if p.kind == 2 {
            let a = key.f60 + key.f64 * rand01(libc);
            let b = key.f70 + key.f74 * rand01(libc);
            ch.rot_x = wrap(a);
            ch.rot_y = 0.0;
            ch.rot_z = self.rot_z;
            self.rot_z = wrap(self.rot_z + b);
        } else {
            ch.rot_x = self.rot_x[self.slot];
            ch.rot_y = self.rot_y[self.slot];
            ch.rot_z = self.rot_z;
        }
        let odd = self.slot & 1 != 0;
        let sx = key.step_x_base + key.step_x_range * rand01(libc);
        let sy = key.step_y_base + key.step_y_range * rand01(libc);
        (ch.step_x, ch.step_y) = if odd { (-sx, -sy) } else { (sx, sy) };
        init_strand(ch, &key, self.flags, &self.mtx, self.scale, libc);
        let cx = key.crawl_x_base + key.crawl_x_range * rand01(libc);
        let cy = key.crawl_y_base + key.crawl_y_range * rand01(libc);
        (ch.crawl_x, ch.crawl_y) = if odd { (-cx, -cy) } else { (cx, cy) };
        let r = key.turn_base + key.turn_range * rand01(libc);
        ch.crawl_x *= r;
        ch.crawl_y *= r;
        ch.drift = scale3(ch.drift, r);
        match p.offset_mode {
            0 => ch.flags |= S_DRIFT,
            1 => {
                if libc.rand() & 1 == 0 {
                    ch.flags |= S_DRIFT;
                }
            }
            _ => {}
        }
        ch.fade_in = key.fade_in;
        if ch.fade_in <= 0.0 {
            ch.flags |= S_FADED_IN;
        }
        ch.fade_out = key.fade_out;
        if ch.fade_out <= 0.0 {
            ch.flags |= S_FADED_OUT;
        } else {
            ch.fade_out_t = ch.fade_out;
        }
        self.live += 1;
        true
    }

    /// `EftChain_UpdateStrands`: growth, shrinking, the end phase, the width and colour pulses, the colour drift,
    /// the fades and the nodes; a strand over and faded out is freed on the next frame.
    fn update_strands(&mut self, p: &ChainPrm, pool: &mut NodePool, libc: &mut LibcRandom) {
        let key = self.key;
        for i in 0..STRANDS {
            let ch = &mut self.strands[i];
            if ch.flags & S_ON == 0 {
                continue;
            }
            if ch.flags & S_DEAD != 0 {
                ch.free(pool);
                ch.flags = 0;
                self.live -= 1;
                continue;
            }
            let t = (ch.time / ch.life).min(1.0);
            if ch.flags & S_F40 == 0 && ch.f7c <= ch.time {
                ch.flags |= S_F40;
            }
            let count = ch.nodes.len() as f32;
            if ch.flags & S_GROWN == 0 {
                ch.shown += key.grow_rate;
                if count <= ch.shown {
                    ch.shown = count;
                    ch.flags |= S_GROWN;
                }
            }
            if ch.flags & S_SHRUNK == 0 && ch.shrink_at <= ch.time {
                ch.shown -= key.shrink_rate;
                if ch.shown <= 0.0 {
                    ch.shown = 0.0;
                    ch.flags |= S_SHRUNK;
                }
            }
            if ch.flags & S_ENDING == 0 {
                if ch.end_at <= ch.time {
                    ch.width = key.end_width;
                    ch.alpha = key.end_alpha;
                    ch.flags |= S_ENDING;
                }
            } else {
                ch.width = 1.0;
                ch.alpha = 1.0;
            }
            if p.flags & 4 != 0 && key.pulse_time > 0.0 {
                let u = ch.pulse_t / key.pulse_time;
                ch.width *= key.pulse_base + key.pulse_range * u;
                ping_pong(&mut ch.pulse_t, &mut ch.flags, S_PULSE_DOWN, key.pulse_time);
            }
            ch.color = if ch.flags & S_DRIFT != 0 {
                std::array::from_fn(|c| ch.col_b[c] * t + ch.col_a[c])
            } else {
                ch.col_a
            };
            ch.color = ch.color.map(|c| c.clamp(0.0, 255.0));
            if ch.flags & S_FADED_IN == 0 {
                let u = ch.fade_in_t / ch.fade_in;
                ch.fade_in_t += 1.0;
                if ch.fade_in <= ch.fade_in_t {
                    ch.flags |= S_FADED_IN;
                } else {
                    ch.color[3] *= u;
                }
            }
            if ch.time >= ch.fade_at {
                if ch.flags & S_FADED_OUT == 0 {
                    let u = ch.fade_out_t / ch.fade_out;
                    ch.fade_out_t -= 1.0;
                    if ch.fade_out_t > 0.0 {
                        ch.color[3] *= u;
                    } else {
                        ch.color[3] = 0.0;
                        ch.flags |= S_FADED_OUT;
                    }
                } else {
                    ch.color[3] = 0.0;
                }
            }
            if p.flags & 8 != 0 && key.mul_time > 0.0 {
                let u = ch.mul_t / key.mul_time;
                for c in 0..3 {
                    ch.color[c] *= key.mul_base[c] + key.mul_range[c] * u;
                }
                ping_pong(&mut ch.mul_t, &mut ch.flags, S_MUL_DOWN, key.mul_time);
            }
            if ch.color[3] <= 0.0 || ch.width <= 0.0 || ch.alpha <= 0.0 {
                ch.flags &= !S_VISIBLE;
            } else {
                ch.flags |= S_VISIBLE;
            }
            update_strand(ch, p, &key, self.pos, libc);
            ch.time += 1.0;
            if ch.life <= ch.time && ch.color[3] <= 0.0 {
                ch.flags |= S_DEAD;
            }
        }
    }

    /// `EftChain_Draw` / `EftChain_DrawStrands` (the caller tests the view of a front chain): every strand in use,
    /// as clipped polygons (parameter flag 2) or as projected strips.
    pub(crate) fn draw(&self, p: &ChainPrm, ot: &mut OrderTable, ctx: &DrawCtx) {
        for ch in &self.strands {
            if ch.flags & (S_VISIBLE | S_ON) == 0 {
                continue;
            }
            self.draw_strand(p, ch, ot, ctx);
        }
    }

    /// `EftChain_DrawStrand` / `EftChain_DrawStrandClipped`: a camera-facing ribbon, one quad per shown node and
    /// its successor, widened across the segment by the nodes' widths times the strand's.
    fn draw_strand(&self, p: &ChainPrm, ch: &Strand, ot: &mut OrderTable, ctx: &DrawCtx) {
        const UV: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
        let front = self.flags & C_FRONT != 0;
        let mut prev_end: Option<[V3; 2]> = None;
        for (k, n) in ch.nodes.iter().enumerate() {
            let Some(next) = ch.nodes.get(k + 1) else {
                continue;
            };
            if n.flags & 2 == 0 {
                continue;
            }
            let side = norm(cross(sub(next.world, n.world), sub(n.world, ctx.eye)));
            let [q0, q1] = match prev_end {
                Some(e) => e,
                None => [
                    add(n.world, scale3(side, n.width_a * ch.width)),
                    sub(n.world, scale3(side, n.width_b * ch.width)),
                ],
            };
            let q2 = add(next.world, scale3(side, next.width_a * ch.width));
            let q3 = sub(next.world, scale3(side, next.width_b * ch.width));
            prev_end = Some([q2, q3]);
            let quad = [q0, q1, q2, q3];
            let cols = [n.color, n.color, next.color, next.color];
            if p.flags & 2 != 0 {
                for j in 0..2 {
                    let tri = std::array::from_fn(|i| ClipVtx {
                        pos: point(quad[j + i]),
                        uv: [UV[j + i][0], UV[j + i][1], 1.0, 0.0],
                        col: cols[j + i],
                    });
                    let prim = Prim {
                        texture: self.texture,
                        ot_layer: p.layer,
                        z: ZSort::Average(1.0),
                        front,
                        clamp: false,
                        alpha_min: 0.01,
                    };
                    queue_tri_clip(ot, ctx, tri, prim);
                }
            } else {
                queue_strip_colors(
                    ot,
                    ctx,
                    &quad,
                    UV,
                    cols,
                    p.layer,
                    self.texture,
                    front,
                    false,
                    false,
                );
            }
        }
    }
}

/// A ping-pong timer over `time` frames: up to `time`, then down to 0.
fn ping_pong(t: &mut f32, flags: &mut u32, down: u32, time: f32) {
    if *flags & down == 0 {
        *t += 1.0;
        if time <= *t {
            *t = time;
            *flags |= down;
        }
    } else {
        *t -= 1.0;
        if *t <= 0.0 {
            *t = 0.0;
            *flags &= !down;
        }
    }
}

/// `EftChain_InitStrand`: node radii from a random first to a random last radius along the strand's direction (it
/// turns by the steps per node, onto the chain's direction when oriented), random jitter in proportion to the node
/// spacing, random widths (the two end nodes at a fifth); the drift is the last spacing.
fn init_strand(
    ch: &mut Strand,
    key: &Key,
    flags: u32,
    mtx: &Mtx,
    scale: f32,
    libc: &mut LibcRandom,
) {
    let mut r = key.rad_base + key.rad_range * rand01(libc);
    let step = key.rad2_base + key.rad2_range * rand01(libc);
    let step = (step - r) / ch.nodes.len() as f32;
    r *= scale;
    let step = step * scale;
    let last = ch.nodes.len() - 1;
    let mut prev = [0.0; 3];
    let mut d = [0.0; 3];
    for i in 0..ch.nodes.len() {
        let m = rotate_xyz(&IDENTITY, [ch.rot_x, ch.rot_y, ch.rot_z]);
        let mut dir = xyz(apply(&m, [0.0, 0.0, 1.0, 1.0]));
        if flags & C_ORIENTED != 0 {
            dir = xyz(apply(mtx, point(dir)));
        }
        let n = &mut ch.nodes[i];
        n.flags |= 1;
        n.radius = r;
        n.pos = scale3(dir, r);
        if i == 0 {
            n.jit = [0.0; 3];
            n.amp = 0.0;
        } else {
            d = sub(n.pos, prev);
            n.amp = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * key.jitter;
            n.jit = jitter(n.amp, libc);
        }
        prev = n.pos;
        n.width_a = key.width_base + key.width_range * rand01(libc);
        n.width_b = key.width_base + key.width_range * rand01(libc);
        if i == 0 || i == last {
            n.width_a *= 0.2;
            n.width_b *= 0.2;
        }
        n.color = ch.color;
        r += step;
        ch.rot_x = wrap(ch.rot_x + ch.step_x);
        ch.rot_y = wrap(ch.rot_y + ch.step_y);
    }
    ch.drift = d;
}

/// A random offset of up to `amp` on each axis (`amp * (RANDF() - RANDF())`).
fn jitter(amp: f32, libc: &mut LibcRandom) -> V3 {
    std::array::from_fn(|_| {
        let a = rand01(libc);
        let b = rand01(libc);
        amp * (a - b)
    })
}

/// `EftChain_UpdateStrand`: the first `shown` nodes are shown; once grown, kind 1 crawls (each node takes its
/// successor's place, the last a new one, and the crawl turns; as in the original, rotY becomes the wrapped rotX
/// plus the crawl), the other kinds drift with new jitter; then every node's world position and colour (kinds 0
/// and 2 fade the head).
fn update_strand(ch: &mut Strand, p: &ChainPrm, key: &Key, origin: V3, libc: &mut LibcRandom) {
    let count = ch.nodes.len();
    for i in 0..count {
        let fi = i as f32;
        {
            let n = &mut ch.nodes[i];
            if fi < ch.shown {
                n.flags |= 2;
            } else {
                n.flags &= !2;
            }
        }
        if ch.flags & S_GROWN != 0 {
            if p.kind == 1 {
                if i + 1 < count {
                    let (pos, jit) = (ch.nodes[i + 1].pos, ch.nodes[i + 1].jit);
                    let n = &mut ch.nodes[i];
                    n.pos = pos;
                    n.jit = jit;
                } else {
                    let m = rotate_xyz(&IDENTITY, [ch.rot_x, ch.rot_y, 0.0]);
                    let dir = xyz(apply(&m, [0.0, 0.0, 1.0, 1.0]));
                    let n = &mut ch.nodes[i];
                    let old = n.pos;
                    n.pos = scale3(dir, n.radius);
                    let d = sub(old, n.pos);
                    n.amp = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * key.jitter;
                    n.jit = jitter(n.amp, libc);
                    ch.rot_x += ch.crawl_x;
                    ch.rot_y = wrap(wrap(ch.rot_x) + ch.crawl_y);
                }
            } else {
                let drift = ch.drift;
                let n = &mut ch.nodes[i];
                n.pos = add(n.pos, drift);
                n.jit = jitter(n.amp, libc);
            }
        }
        let n = &mut ch.nodes[i];
        n.world = add(add(add(origin, ch.pos), n.pos), n.jit);
        n.color = ch.color;
        n.color[3] = (n.color[3] * ch.alpha).min(255.0);
        if p.kind == 0 || p.kind == 2 {
            if ch.shown - 1.0 <= fi {
                n.color[3] *= 0.0;
            } else if ch.shown - 2.0 <= fi {
                n.color[3] *= 0.5;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_blend_keeps_the_originals_row_mixups() {
        let mut p = ChainPrm {
            flags: 1,
            kind: 0,
            offset_mode: 0,
            layer: 0,
            count_base: 0,
            count_range: 0,
            key_time: 0.0,
            key_mid: 0.0,
            rows: [[0.0; 3]; 39],
        };
        p.rows[1] = [0.0, 2.0, 2.0];
        p.rows[2] = [10.0, 10.0, 10.0];
        p.rows[9] = [0.0, 4.0, 4.0];
        p.rows[20] = [1.0, 1.0, 1.0];
        p.rows[29] = [0.5, 0.5, 0.5];
        let k = blend_keys(&p, &ChainKeys::default(), 5.0, 10.0, 20.0);
        // Row 2 moves by row 1's change, row 20 by row 9's; the step rows are not scaled by pi.
        assert!((k.f64 - 11.0 * PI_F).abs() < 1e-4);
        assert!((k.end_alpha - 3.0).abs() < 1e-6);
        assert_eq!(k.step_x_base, 0.5);
        // The key itself scales them.
        assert!((key_of(&p, &ChainKeys::default(), 0).step_x_base - 0.5 * PI_F).abs() < 1e-6);
    }
}
