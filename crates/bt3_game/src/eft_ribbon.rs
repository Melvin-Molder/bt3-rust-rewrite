//! Effect pack part kind 17, the ribbon (`EftRibbon_*`, src/battle/eft_ribbon.c): a strip of camera-facing quads
//! through a list of nodes from a shared pool of 500, spread between two points (kind 0) or trailing behind one
//! point (kinds 1 and 2). Colour, colour pulse, width and scroll come from three keys of its pack blocks.

use bt3_data::eft_pack::{RibbonAnim, RibbonPrm};

use crate::eft::{
    ClipVtx, DrawCtx, OrderTable, Prim, SetRef, V3, V4, ZSort, add, cross, dot, draw_sprite, norm,
    point, queue_tri_clip, scale3, sub,
};
use crate::rng::LibcRandom;

/// `EFT_RBN_NODES`.
pub(crate) const RIBBON_NODES: usize = 500;
/// `EftRibbonMgr_Init`: 40 ribbon tasks.
pub(crate) const RIBBONS: usize = 40;

// EFT_RIBBON_*.
const R_ALIVE: u32 = 0x1;
pub(crate) const R_STOP: u32 = 0x2;
pub(crate) const R_DEAD: u32 = 0x4;
const R_VISIBLE: u32 = 0x8;
const R_NO_LIFE: u32 = 0x10;
const R_SCROLL: u32 = 0x20;
const R_MIRROR: u32 = 0x40;
const R_FADED_IN: u32 = 0x80;
const R_FADED_OUT: u32 = 0x100;
const R_WIDTH_DN: u32 = 0x200;
const R_PULSE_DN: u32 = 0x400;
const R_TIMED_OUT: u32 = 0x1000;

/// `EftRbnCur`: the animated values.
#[derive(Clone, Copy, Debug, Default)]
struct Cur {
    color: V4,
    pulse: [f32; 3],
    pulse_amp: [f32; 3],
    pulse_time: f32,
    width: f32,
    width_base: f32,
    width_amp: f32,
    width_time: f32,
    alpha: f32,
    scroll: f32,
}

/// `EftRbnNode`.
#[derive(Clone, Copy, Debug, Default)]
struct Node {
    tex: u8,
    pos: V3,
    color: V4,
    color2: V4,
}

/// An `EftRibbon` task.
#[derive(Clone, Debug)]
pub(crate) struct Ribbon {
    pub id: u32,
    chr: usize,
    pub set: SetRef,
    pub pair: usize,
    /// The texture of each of its four slots (resource entries `texBase + i`).
    tex: [Option<usize>; 4],
    pub flags: u32,
    kind: u8,
    blend: u8,
    pos: V3,
    size: f32,
    life: f32,
    pub delay: f32,
    pub fade_delay: f32,
    fade_time: f32,
    pub max_nodes: i32,
    want_nodes: i32,
    nodes: Vec<Node>,
    cur: Cur,
    end: V3,
    /// `color` / `scaleA` and `color2` / `scaleB`.
    color: V4,
    color2: V4,
    anim_frame: f32,
    anim_time: f32,
    anim_split: f32,
    width: f32,
    width_frame: f32,
    pulse_frame: f32,
    fade_in_frame: f32,
    fade_out_frame: f32,
    fade_in_time: f32,
    fade_out_time: f32,
    alpha: f32,
    scroll: f32,
    scroll_pos: f32,
    pub updated: bool,
}

/// `EftRibbon_LoadKey`.
fn load_key(cur: &mut Cur, prm: &RibbonPrm, anim: &RibbonAnim, key: usize) {
    cur.color = anim.color[key];
    for c in 0..3 {
        cur.pulse[c] = anim.pulse[c][key][0];
        cur.pulse_amp[c] = anim.pulse[c][key][1] - anim.pulse[c][key][0];
    }
    cur.pulse_time = anim.pulse_time[key] * 30.0;
    cur.width = prm.width[key];
    cur.width_base = prm.width_lo[key];
    cur.width_amp = prm.width_hi[key] - prm.width_lo[key];
    cur.width_time = prm.width_time[key] * 30.0;
    if prm.flags & 4 != 0 {
        cur.alpha = prm.alpha[key];
        cur.scroll = prm.scroll[key];
    }
}

impl Ribbon {
    /// `EftRibbon_Init` with `EftRibbon_InitNodes`: one libc rand() when the pack asks for a random mirror.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u32,
        chr: usize,
        set: SetRef,
        pair: usize,
        tex_base: &[usize],
        pos: V3,
        life: f32,
        size: f32,
        prm: &RibbonPrm,
        anim: &RibbonAnim,
        libc: &mut LibcRandom,
    ) -> Self {
        let mut w = Self {
            id,
            chr,
            set,
            pair,
            tex: [None; 4],
            flags: R_ALIVE,
            kind: prm.kind,
            blend: prm.blend,
            pos,
            size,
            life: 0.0,
            delay: 0.0,
            fade_delay: 0.0,
            fade_time: 0.0,
            max_nodes: 0,
            want_nodes: 0,
            nodes: Vec::new(),
            cur: Cur::default(),
            end: [0.0; 3],
            color: [0.0; 4],
            color2: [0.0; 4],
            anim_frame: 0.0,
            anim_time: 0.0,
            anim_split: 0.0,
            width: 0.0,
            width_frame: 0.0,
            pulse_frame: 0.0,
            fade_in_frame: 0.0,
            fade_out_frame: 0.0,
            fade_in_time: 0.0,
            fade_out_time: 0.0,
            alpha: 0.0,
            scroll: 0.0,
            scroll_pos: 0.0,
            updated: false,
        };
        if life <= 0.0 {
            w.flags |= R_NO_LIFE;
        } else {
            w.life = life * 30.0;
        }
        if prm.flags & 1 != 0 {
            w.anim_time = prm.anim_time * 30.0;
            w.anim_split = w.anim_time * prm.anim_split;
            load_key(&mut w.cur, prm, anim, 0);
        } else {
            load_key(&mut w.cur, prm, anim, 2);
        }
        // EftRibbon_InitNodes.
        if prm.flags & 4 != 0 {
            w.flags |= R_SCROLL;
        }
        if prm.flags & 2 != 0 && libc.rand() & 1 == 0 {
            w.flags |= R_MIRROR;
        }
        w.end = pos;
        w.width = w.cur.width;
        w.color = [w.cur.color[0], w.cur.color[1], w.cur.color[2], 0.0];
        w.fade_in_time = prm.fade_in * 30.0;
        if w.fade_in_time <= 0.0 {
            w.flags |= R_FADED_IN;
        }
        w.fade_out_time = prm.fade_out * 30.0;
        if w.fade_out_time <= 0.0 {
            w.flags |= R_FADED_OUT;
        } else {
            w.fade_out_frame = w.fade_out_time;
        }
        if w.flags & R_SCROLL != 0 {
            w.alpha = w.cur.alpha;
            w.scroll = w.cur.scroll;
        }
        let (num_tex, max) = match prm.kind {
            0 if w.flags & R_SCROLL != 0 => (4, 0x20),
            0 => (3, 0x20),
            1 => (1, 0x10),
            _ => (4, 0x14),
        };
        w.max_nodes = max;
        for i in 0..num_tex {
            w.tex[i] = tex_base.get(i).copied();
        }
        w
    }

    /// `EftRibbon_SetFadeTime`.
    pub(crate) fn set_fade_time(&mut self, frames: f32) {
        self.fade_time = frames;
        if frames > 0.0 {
            self.fade_out_time = frames;
            self.fade_out_frame = frames;
        }
    }

    /// `EftRibbon_SetPos`.
    pub(crate) fn set_pos(&mut self, pos: V3) {
        self.pos = pos;
    }

    /// `EftRibbon_SetEnd`: with `place`, a kind 0 ribbon lays its nodes out again at once.
    pub(crate) fn set_end(&mut self, pos: V3, place: bool) {
        self.end = pos;
        if place && self.kind == 0 {
            self.place_strip();
        }
    }

    /// `EftRibbon_SetSize`.
    pub(crate) fn set_size(&mut self, size: f32) {
        self.size = size;
    }

    /// `EftRibbon_UpdateKeys`: the animated values between keys 0 and 1 before the split, 1 and 2 after.
    fn update_keys(&mut self, prm: &RibbonPrm, anim: &RibbonAnim) {
        let (t, k0, k1) = if self.anim_frame < self.anim_split {
            (self.anim_frame / self.anim_split, 0, 1)
        } else {
            (
                (self.anim_frame - self.anim_split) / (self.anim_time - self.anim_split),
                1,
                2,
            )
        };
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        let cur = &mut self.cur;
        cur.color = std::array::from_fn(|i| lerp(anim.color[k0][i], anim.color[k1][i]));
        for c in 0..3 {
            let lo = lerp(anim.pulse[c][k0][0], anim.pulse[c][k1][0]);
            let hi = lerp(anim.pulse[c][k0][1], anim.pulse[c][k1][1]);
            cur.pulse[c] = lo;
            cur.pulse_amp[c] = hi - lo;
        }
        cur.pulse_time = lerp(anim.pulse_time[k0], anim.pulse_time[k1]) * 30.0;
        cur.width = lerp(prm.width[k0], prm.width[k1]);
        let lo = lerp(prm.width_lo[k0], prm.width_lo[k1]);
        let hi = lerp(prm.width_hi[k0], prm.width_hi[k1]);
        cur.width_base = lo;
        cur.width_amp = hi - lo;
        cur.width_time = lerp(prm.width_time[k0], prm.width_time[k1]) * 30.0;
        if prm.flags & 4 != 0 {
            cur.alpha = lerp(prm.alpha[k0], prm.alpha[k1]);
            cur.scroll = lerp(prm.scroll[k0], prm.scroll[k1]);
        }
    }

    /// `EftRibbon_PlaceStrip`: kind 0 spreads its nodes evenly from the first point to the second; the first,
    /// middle and last segments use textures 0, 1 and 2.
    fn place_strip(&mut self) {
        let num = self.nodes.len() as f32;
        let step = scale3(sub(self.end, self.pos), 1.0 / num);
        let mut pos = self.pos;
        let n = self.nodes.len() as i32;
        let last = self.nodes.len().saturating_sub(1);
        for (i, node) in self.nodes.iter_mut().enumerate() {
            let i = i as i32;
            node.pos = pos;
            node.tex = if i == 0 {
                0
            } else if i < n - 2 {
                1
            } else {
                2
            };
            node.color = self.color;
            if self.flags & R_SCROLL != 0 {
                node.color2 = self.color2;
                if i == 0 || i as usize == last {
                    node.color2[3] = 0.0;
                }
            }
            if i < n - 2 {
                pos = add(pos, step);
            } else {
                pos = self.end;
            }
        }
    }

    /// `EftRibbon_PlaceTrail1` / `2`: the second point goes into the head and every older position one node
    /// down; kind 2 also uses textures 2, 1, 0 and refreshes the colours.
    fn place_trail(&mut self, kind2: bool) {
        let mut cur = self.end;
        let n = self.nodes.len() as i32;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            let old = node.pos;
            node.pos = cur;
            cur = old;
            if kind2 {
                let i = i as i32;
                node.tex = if i == 0 {
                    2
                } else if i < n - 2 {
                    1
                } else {
                    0
                };
                node.color = self.color;
            } else {
                node.tex = 0;
            }
        }
    }

    /// `EftRibbon_Update`. `nodes_free`: the shared pool's free nodes.
    pub(crate) fn update(
        &mut self,
        prm: &RibbonPrm,
        anim: &RibbonAnim,
        stopped: bool,
        nodes_free: &mut usize,
    ) {
        if !stopped {
            if self.delay <= 0.0 {
                self.width = self.cur.width;
                self.alpha = self.cur.alpha;
                self.scroll = self.cur.scroll;
                if prm.flags & 8 != 0 && self.cur.width_time > 0.0 {
                    let t = self.width_frame / self.cur.width_time;
                    let k = self.cur.width_base + self.cur.width_amp * t;
                    self.width *= k;
                    self.alpha *= k;
                    if self.flags & R_WIDTH_DN == 0 {
                        self.width_frame += 1.0;
                        if self.cur.width_time <= self.width_frame {
                            self.width_frame = self.cur.width_time;
                            self.flags |= R_WIDTH_DN;
                        }
                    } else {
                        self.width_frame -= 1.0;
                        if self.width_frame <= 0.0 {
                            self.width_frame = 0.0;
                            self.flags &= !R_WIDTH_DN;
                        }
                    }
                }
                self.width *= self.size;
                self.alpha *= self.size;
                if self.flags & R_FADED_IN == 0 {
                    let t = self.fade_in_frame / self.fade_in_time;
                    self.fade_in_frame += 1.0;
                    if self.fade_in_time <= self.fade_in_frame {
                        self.flags |= R_FADED_IN;
                    } else {
                        self.width *= t;
                        self.alpha *= t;
                    }
                }
                if self.flags & R_STOP != 0 && self.fade_delay <= 0.0 {
                    if self.flags & R_FADED_OUT == 0 {
                        let t = self.fade_out_frame / self.fade_out_time;
                        self.fade_out_frame -= 1.0;
                        self.width *= t;
                        self.alpha *= t;
                        if self.fade_out_frame <= 0.0 {
                            self.flags |= R_FADED_OUT;
                            self.width = 0.0;
                            self.alpha = 0.0;
                        }
                    } else {
                        self.width = 0.0;
                        self.alpha = 0.0;
                    }
                }
                self.color = self.cur.color;
                self.color2 = self.cur.color;
                if prm.flags & 0x10 != 0 && self.cur.pulse_time > 0.0 {
                    let t = self.pulse_frame / self.cur.pulse_time;
                    for c in 0..3 {
                        let k = self.cur.pulse[c] + self.cur.pulse_amp[c] * t;
                        self.color[c] *= k;
                        self.color2[c] *= k;
                    }
                    if self.flags & R_PULSE_DN == 0 {
                        self.pulse_frame += 1.0;
                        if self.cur.pulse_time <= self.pulse_frame {
                            self.pulse_frame = self.cur.pulse_time;
                            self.flags |= R_PULSE_DN;
                        }
                    } else {
                        self.pulse_frame -= 1.0;
                        if self.pulse_frame <= 0.0 {
                            self.pulse_frame = 0.0;
                            self.flags &= !R_PULSE_DN;
                        }
                    }
                }
                if self.flags & R_SCROLL != 0 {
                    self.scroll_pos += self.scroll;
                    if 1.0 <= self.scroll_pos {
                        self.scroll_pos -= 1.0;
                    } else if self.scroll_pos <= -1.0 {
                        self.scroll_pos += 1.0;
                    }
                }
                if self.color[3] <= 0.0 || self.width <= 0.0 {
                    self.flags &= !R_VISIBLE;
                } else {
                    self.flags |= R_VISIBLE;
                }
                if prm.kind == 0 {
                    let d = sub(self.end, self.pos);
                    self.want_nodes = (dot(d, d).sqrt() / 60.0 + 4.0) as i32;
                } else {
                    self.want_nodes += 1;
                }
                if self.want_nodes > self.max_nodes {
                    self.want_nodes = self.max_nodes;
                }
                let have = self.nodes.len() as i32;
                if have < self.want_nodes {
                    for _ in 0..self.want_nodes - have {
                        // EftRibbon_AddNode: the new node copies itself (the tail is already the new node).
                        if *nodes_free == 0 {
                            break;
                        }
                        *nodes_free -= 1;
                        self.nodes.push(Node::default());
                    }
                }
                match prm.kind {
                    0 => self.place_strip(),
                    1 => self.place_trail(false),
                    _ => self.place_trail(true),
                }
                if prm.flags & 1 != 0 && self.anim_frame <= self.anim_time {
                    self.update_keys(prm, anim);
                    self.anim_frame += 1.0;
                    if self.anim_time <= self.anim_frame {
                        load_key(&mut self.cur, prm, anim, 2);
                    }
                }
                if self.flags & R_NO_LIFE == 0 {
                    self.life -= 1.0;
                    if self.life <= 0.0 {
                        self.flags |= R_STOP;
                        if self.fade_time > 0.0 {
                            self.flags |= R_TIMED_OUT;
                        }
                    }
                }
            } else {
                self.delay -= 1.0;
            }
            if self.flags & R_STOP != 0 {
                if self.fade_delay > 0.0 {
                    self.fade_delay -= 1.0;
                }
                if self.fade_delay <= 0.0 && self.width <= 0.0 {
                    self.flags |= R_DEAD;
                }
            }
        }
        self.updated = true;
    }

    /// `EftRibbon_Term`: the nodes go back to the pool.
    pub(crate) fn free_nodes(&mut self, nodes_free: &mut usize) {
        *nodes_free += self.nodes.len();
        self.nodes.clear();
    }

    /// `EftRibbon_Draw` by kind.
    pub(crate) fn draw(&self, ot: &mut OrderTable, ctx: &DrawCtx) {
        match self.kind {
            0 => self.draw_strip(ot, ctx),
            1 => self.draw_kind1(ot, ctx),
            2 => self.draw_kind2(ot, ctx),
            _ => {}
        }
    }

    /// `EftGfx_DrawPolyScaledZ` with factor 1 for the two triangles (i, i + 1, i + 2) of a quad.
    fn quad(
        &self,
        ot: &mut OrderTable,
        ctx: &DrawCtx,
        pos: &[V3; 4],
        uv: &[V4; 4],
        col: &[V4; 4],
        tex: Option<usize>,
    ) {
        for i in 0..2 {
            let tri = std::array::from_fn(|k| ClipVtx {
                pos: point(pos[i + k]),
                uv: uv[i + k],
                col: col[i + k],
            });
            let prim = Prim {
                texture: tex,
                ot_layer: self.blend,
                z: ZSort::Average(1.0),
                front: false,
                clamp: false,
                alpha_min: 0.01,
            };
            queue_tri_clip(ot, ctx, tri, prim);
        }
    }

    /// The side vector of a segment: across the segment and the direction from the camera.
    fn side(&self, a: V3, b: V3, cam: V3) -> V3 {
        norm(cross(sub(b, a), sub(a, cam)))
    }

    /// `EftRibbon_DrawStrip`: kind 0, each quad starting on the previous one's end points; a scrolling ribbon is
    /// drawn again with its fourth texture, the scrolled texture rectangle and the second colours (on the same
    /// corners: the second layer's own corners are computed and not used).
    fn draw_strip(&self, ot: &mut OrderTable, ctx: &DrawCtx) {
        let ua: [V4; 4] = [
            [0.02, 0.0, 1.0, 0.0],
            [0.02, 0.98, 1.0, 0.0],
            [0.98, 0.0, 1.0, 0.0],
            [0.98, 0.98, 1.0, 0.0],
        ];
        let ub: [V4; 4] = [
            [0.02, 0.98, 1.0, 0.0],
            [0.02, 0.0, 1.0, 0.0],
            [0.98, 0.98, 1.0, 0.0],
            [0.98, 0.0, 1.0, 0.0],
        ];
        let uv = if self.flags & R_MIRROR == 0 { ua } else { ub };
        let uv2 = uv.map(|u| [u[0] + self.scroll_pos, u[1], u[2], u[3]]);
        let cam = ctx.eye;
        let mut prev: Option<[V3; 2]> = None;
        for (n, next) in self.nodes.iter().zip(self.nodes.iter().skip(1)) {
            let half = scale3(self.side(n.pos, next.pos, cam), self.width);
            let [q0, q1] = prev.unwrap_or([sub(n.pos, half), add(n.pos, half)]);
            let quad = [q0, q1, sub(next.pos, half), add(next.pos, half)];
            prev = Some([quad[2], quad[3]]);
            let col = [n.color, n.color, next.color, next.color];
            self.quad(ot, ctx, &quad, &uv, &col, self.tex[usize::from(n.tex)]);
            if self.flags & R_SCROLL != 0 {
                let col = [n.color2, n.color2, next.color2, next.color2];
                self.quad(ot, ctx, &quad, &uv2, &col, self.tex[3]);
            }
        }
    }

    /// `EftRibbon_DrawKind1`: the texture's u runs from 1 at the head to 0 at the tail, one colour for all; a side
    /// that turns against the previous one is flipped.
    fn draw_kind1(&self, ot: &mut OrderTable, ctx: &DrawCtx) {
        let ua: [V4; 4] = [
            [1.0, 0.0, 1.0, 0.0],
            [1.0, 1.0, 1.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0, 0.0],
        ];
        let ub: [V4; 4] = [
            [1.0, 1.0, 1.0, 0.0],
            [1.0, 0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let mut uv = if self.flags & R_MIRROR == 0 { ua } else { ub };
        let num = self.nodes.len() as i32;
        let du = -1.0 / num as f32;
        let (mut u, mut u2) = (1.0f32, du + 1.0);
        let col = self.color;
        let cam = ctx.eye;
        let mut prev: Option<([V3; 2], V3)> = None;
        for (idx, node) in self.nodes.iter().enumerate() {
            if let Some(next) = self.nodes.get(idx + 1) {
                let mut side = self.side(node.pos, next.pos, cam);
                let mut half = scale3(side, self.width);
                let (q0, q1) = match prev {
                    None => (add(node.pos, half), sub(node.pos, half)),
                    Some((p, prev_side)) => {
                        if dot(side, prev_side) < 0.0 {
                            half = scale3(half, -1.0);
                            side = scale3(side, -1.0);
                        }
                        (p[0], p[1])
                    }
                };
                let quad = [q0, q1, add(next.pos, half), sub(next.pos, half)];
                prev = Some(([quad[2], quad[3]], side));
                uv[0][0] = u;
                uv[1][0] = u;
                uv[2][0] = u2;
                uv[3][0] = u2;
                self.quad(
                    ot,
                    ctx,
                    &quad,
                    &uv,
                    &[col; 4],
                    self.tex[usize::from(node.tex)],
                );
            }
            u += du;
            u2 += du;
            if !((idx as i32) < num - 2) {
                u2 = 0.0;
            }
        }
    }

    /// `EftRibbon_DrawKind2`: per-node colours and a fixed strip (u 0.98..0.02); where it bends sharply three
    /// sprites (node, middle, next node) of the fourth texture replace the quad.
    fn draw_kind2(&self, ot: &mut OrderTable, ctx: &DrawCtx) {
        let ua: [V4; 4] = [
            [0.98, 0.0, 1.0, 0.0],
            [0.98, 0.98, 1.0, 0.0],
            [0.02, 0.0, 1.0, 0.0],
            [0.02, 0.98, 1.0, 0.0],
        ];
        let ub: [V4; 4] = [
            [0.98, 0.98, 1.0, 0.0],
            [0.98, 0.0, 1.0, 0.0],
            [0.02, 0.98, 1.0, 0.0],
            [0.02, 0.0, 1.0, 0.0],
        ];
        let uv = if self.flags & R_MIRROR == 0 { ua } else { ub };
        let size = self.width * 9.5;
        let num = self.nodes.len() as i32;
        let cam = ctx.eye;
        let mut prev: Option<([V3; 2], V3)> = None;
        for (idx, node) in self.nodes.iter().enumerate() {
            let Some(next) = self.nodes.get(idx + 1) else {
                continue;
            };
            let mut sprite = false;
            let mut side = self.side(node.pos, next.pos, cam);
            let mut half = scale3(side, self.width);
            let (q0, q1) = match prev {
                None => (sub(node.pos, half), add(node.pos, half)),
                Some((p, prev_side)) => {
                    if dot(side, prev_side) < 0.0 {
                        half = scale3(half, -1.0);
                        side = scale3(side, -1.0);
                    }
                    let d = sub(node.pos, next.pos);
                    if dot(side, prev_side) < 0.82 && dot(d, d).sqrt() > 0.8 {
                        sprite = true;
                        let mid = scale3(add(node.pos, next.pos), 0.5);
                        // idx / numNodes is an integer division.
                        let a = 1.0 - (idx as i32 / num) as f32;
                        let s = size * a;
                        let mut c = node.color;
                        c[3] *= a;
                        for p in [node.pos, mid, next.pos] {
                            draw_sprite(
                                ot,
                                ctx,
                                p,
                                c,
                                [s, s],
                                [0.0, 0.0, 1.0, 1.0],
                                0.0,
                                self.blend,
                                false,
                                self.tex[3],
                            );
                        }
                    }
                    (p[0], p[1])
                }
            };
            let quad = [q0, q1, sub(next.pos, half), add(next.pos, half)];
            prev = Some(([quad[2], quad[3]], side));
            if !sprite {
                let col = [node.color, node.color, next.color, next.color];
                self.quad(ot, ctx, &quad, &uv, &col, self.tex[usize::from(node.tex)]);
            }
        }
    }

    pub(crate) fn chr(&self) -> usize {
        self.chr
    }
}
