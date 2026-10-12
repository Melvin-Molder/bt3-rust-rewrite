//! The objects' secondary motion (`BtlObj_*Chain*`, `BObjChainA_*` / `BObjChainB_*`, src/battle/btl_obj_chain.c):
//! the chains of extra bones a model carries (hair, tails, cloth; node ids from 0x47 up, listed in the character
//! file's section 0x17) bend every frame with the object's movement, the pushes and sway the fighter code adds,
//! the stage's wind and a noise source (a logistic map), and their rotations are written over the sampled pose
//! before the matrices are built again. Type A chains follow a target with stiffness, rest pull, inertia and a
//! hinge limit; type B chains (tails) swing in a plane with yaw limits.

use bt3_data::chara_data::ChainParams;
use bt3_data::model::ModelPart;

use crate::battle::{IDENTITY, Mtx, apply, inverse_rt, mul};
use crate::btl_obj::{quat_mul, quat_to_mtx};
use crate::btl_tech_volley::{mathf_cos, mathf_sin};

type V4 = [f32; 4];
type Q = [f32; 4];

const PI_F: f32 = 3.141_592_7;
const IDENT: Q = [0.0, 0.0, 0.0, 1.0];

/// `BObjLink`.
#[derive(Clone, Copy, Debug)]
struct Link {
    /// The rotation written to the node last frame.
    rot: Q,
    /// A: the smoothed offset; B: last frame's parent rotation.
    offset: V4,
    slot: usize,
    node: u32,
    /// The depth in the chain (`depthA` for type A, `depthB` for type B).
    depth: usize,
    parent: Option<usize>,
    idle: [f32; 3],
    swing: [f32; 3],
}

/// The chains of an object and the secondary motion's state (`chainA`, `chainB`, `chaos`, `push`, `sway`,
/// `pushed`, `move`).
#[derive(Clone, Debug)]
pub(crate) struct Chains {
    a: Vec<Link>,
    b: Vec<Link>,
    chaos: f32,
    push: V4,
    sway: f32,
    pushed: bool,
    /// `BtlObj_SetMoveVec`: the owner's movement this frame.
    pub move_vec: [f32; 3],
}

impl Default for Chains {
    fn default() -> Self {
        Self {
            a: Vec::new(),
            b: Vec::new(),
            chaos: 0.5,
            push: [0.0; 4],
            sway: 0.0,
            pushed: false,
            move_vec: [0.0; 3],
        }
    }
}

fn len3(v: V4) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn scale4(v: V4, k: f32) -> V4 {
    v.map(|c| c * k)
}

fn add4(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| a[i] + b[i])
}

fn sub4(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| a[i] - b[i])
}

/// `Vec3_Normalize`: the direction, w kept; a zero vector gives zero.
fn norm3(v: V4) -> V4 {
    let l = len3(v);
    if l == 0.0 {
        [0.0, 0.0, 0.0, v[3]]
    } else {
        [v[0] / l, v[1] / l, v[2] / l, v[3]]
    }
}

fn dot3(a: V4, b: V4) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `Mtx_MulVec4` of a direction (w 0).
fn turn(m: &Mtx, v: V4) -> V4 {
    apply(m, [v[0], v[1], v[2], 0.0])
}

/// `Vec3_RotateAxis` (Rodrigues, `Mathf_Sin` / `Mathf_Cos`), the source's w kept.
pub(crate) fn rotate_axis(v: V4, k: V4, angle: f32) -> V4 {
    let (s, c) = (mathf_sin(angle), mathf_cos(angle));
    let d = dot3(k, v) * (1.0 - c);
    let x = [
        k[1] * v[2] - k[2] * v[1],
        k[2] * v[0] - k[0] * v[2],
        k[0] * v[1] - k[1] * v[0],
    ];
    [
        v[0] * c + x[0] * s + k[0] * d,
        v[1] * c + x[1] * s + k[1] * d,
        v[2] * c + x[2] * s + k[2] * d,
        v[3],
    ]
}

/// `Vec3_RotateX`.
fn rotate_x(v: V4, angle: f32) -> V4 {
    let (s, c) = (mathf_sin(angle), mathf_cos(angle));
    [v[0], v[1] * c - v[2] * s, v[2] * c + v[1] * s, v[3]]
}

/// `Vec3_RotateY`.
fn rotate_y(v: V4, angle: f32) -> V4 {
    let (s, c) = (mathf_sin(angle), mathf_cos(angle));
    [v[0] * c + v[2] * s, v[1], v[2] * c - v[0] * s, v[3]]
}

/// `Mathf_Acos`.
fn acos(x: f32) -> f32 {
    x.clamp(-1.0, 1.0).acos()
}

/// `Quat_FromAxisAngle`: identity for a null axis.
pub(crate) fn q_axis_angle(x: f32, y: f32, z: f32, angle: f32) -> Q {
    let len = (x * x + y * y + z * z).sqrt();
    if len > 0.000_001 {
        let half = angle * 0.5;
        let (s, inv) = (half.sin(), 1.0 / len);
        [x * inv * s, y * inv * s, z * inv * s, half.cos()]
    } else {
        IDENT
    }
}

/// `Quat_SlerpIdentity`: the same axis with the angle scaled by `t`.
fn q_slerp_identity(q: Q, t: f32) -> Q {
    let angle = acos(q[3].abs().min(1.0));
    let s = angle.sin();
    if s < 0.000_001 {
        IDENT
    } else {
        let inv = 1.0 / s;
        let ka = (angle * (1.0 - t)).sin() * inv;
        let kb = (angle * t).sin() * inv;
        [q[0] * kb, q[1] * kb, q[2] * kb, ka + q[3] * kb]
    }
}

/// `Quat_FromVectors`: the shortest rotation from `from` to `to`, scaled by `t`.
pub(crate) fn q_from_vectors(from: V4, to: V4, t: f32) -> Q {
    let (a, b) = (norm3(from), norm3(to));
    let axis = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let s = ((dot3(a, b) + 1.0) * 2.0).sqrt();
    let inv = 1.0 / s;
    let q = [axis[0] * inv, axis[1] * inv, axis[2] * inv, s * 0.5];
    if t != 1.0 { q_slerp_identity(q, t) } else { q }
}

/// `Quat_GetAngle`: 2 acos w.
fn q_angle(q: Q) -> f32 {
    acos(q[3]) * 2.0
}

/// `Quat_Inverse`.
fn q_inverse(q: Q) -> Q {
    let inv = 1.0 / (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]);
    [-q[0] * inv, -q[1] * inv, -q[2] * inv, q[3] * inv]
}

/// `Quat_ConjugateBy`: r q r^-1 (all four components of `q`).
fn q_conjugate_by(q: Q, r: Q) -> Q {
    quat_mul(r, quat_mul(q, q_inverse(r)))
}

/// `Quat_Slerp` along the shorter arc.
pub(crate) fn q_slerp(a: Q, b: Q, t: f32) -> Q {
    let mut qb = b;
    let mut c = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if c < 0.0 {
        qb = b.map(|v| -v);
        c = a[0] * qb[0] + a[1] * qb[1] + a[2] * qb[2] + a[3] * qb[3];
    }
    let angle = acos(c.min(1.0));
    let s = angle.sin();
    if s < 0.000_001 {
        a
    } else {
        let inv = 1.0 / s;
        let ka = (angle * (1.0 - t)).sin() * inv;
        let kb = (angle * t).sin() * inv;
        std::array::from_fn(|i| a[i] * ka + qb[i] * kb)
    }
}

/// `Quat_LimitAngle`: rebuilt with angle `max` when it turns further.
fn q_limit_angle(q: Q, max: f32) -> Q {
    if max.abs() >= 3.141_592_7 || q[3].abs() > 0.999_999 || (max * 0.5).cos() < q[3] {
        q
    } else {
        q_axis_angle(q[0], q[1], q[2], max)
    }
}

/// `Quat_FromMtx`.
fn q_from_mtx(m: &Mtx) -> Q {
    let tr = m[0][0] + 1.0 + m[1][1] + m[2][2];
    if tr > 0.0001 {
        let s = 0.5 / tr.sqrt();
        [
            (m[1][2] - m[2][1]) * s,
            (m[2][0] - m[0][2]) * s,
            (m[0][1] - m[1][0]) * s,
            0.25 / s,
        ]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (m[0][0] + 1.0 - m[1][1] - m[2][2]).sqrt() * 2.0;
        [
            s * 0.25,
            (m[1][0] + m[0][1]) / s,
            (m[2][0] + m[0][2]) / s,
            (m[1][2] - m[2][1]) / s,
        ]
    } else if m[1][1] > m[2][2] {
        let s = (m[1][1] + 1.0 - m[0][0] - m[2][2]).sqrt() * 2.0;
        [
            (m[1][0] + m[0][1]) / s,
            s * 0.25,
            (m[2][1] + m[1][2]) / s,
            (m[2][0] - m[0][2]) / s,
        ]
    } else {
        let s = (m[2][2] + 1.0 - m[0][0] - m[1][1]).sqrt() * 2.0;
        [
            (m[2][0] + m[0][2]) / s,
            (m[2][1] + m[1][2]) / s,
            s * 0.25,
            (m[0][1] - m[1][0]) / s,
        ]
    }
}

/// `BObjChainA_WrapAngle` / `BObjChainB_WrapAngle`: one turn at most into -pi..pi.
fn wrap(a: f32) -> f32 {
    let mut a = a;
    if a < -3.141_592_7 {
        a += 6.283_185_5;
    }
    if a > 3.141_592_7 {
        a -= 6.283_185_5;
    }
    a
}

/// What a chain step reads of the object and writes back: the nodes' matrices by id and their rotations.
pub(crate) trait ChainNodes {
    /// The node's parent and own world matrices, `None` when the model has no such node.
    fn node_mtx(&self, id: u32) -> Option<(Mtx, Mtx)>;
    /// `BtlObj_GetNodeVelocity` (in the object's frame).
    fn velocity(&self, id: u32) -> [f32; 3];
    /// The object's world matrix (`pose.world`).
    fn world(&self) -> Mtx;
    fn set_rot(&mut self, id: u32, rot: Q);
}

/// The links of one table from the bound records: one per record whose node (id >= 0x47) has a slot in `ids`,
/// its depth counted by the links (a record's pops count only on a link) and its parent the link a level up.
fn build(parts: &[ModelPart], ids: &[u8], identity_offset: bool) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    for part in parts {
        let node = u32::from(part.node);
        let slot = if node >= 0x47 {
            ids.iter().position(|&id| u32::from(id) == node)
        } else {
            None
        };
        if let Some(slot) = slot {
            stack.truncate(depth);
            let parent = depth.checked_sub(1).and_then(|d| stack.get(d).copied());
            stack.push(out.len());
            out.push(Link {
                rot: IDENT,
                offset: if identity_offset { IDENT } else { [0.0; 4] },
                slot,
                node,
                depth,
                parent,
                idle: [0.0; 3],
                swing: [0.0; 3],
            });
            depth += 1;
            depth = depth.saturating_sub(usize::from(part.pop));
        }
    }
    out
}

impl Chains {
    /// `BtlObj_InitChains`: the state cleared (the noise at 0.5) and both tables built from the model's bound
    /// records.
    pub(crate) fn new(params: Option<&ChainParams>, parts: &[ModelPart]) -> Self {
        let mut c = Self::default();
        if let Some(p) = params {
            c.a = build(parts, &p.node_a, false);
            c.b = build(parts, &p.node_b, true);
        }
        c
    }

    /// The links' nodes (type A, then type B), for tests.
    #[cfg(test)]
    pub(crate) fn nodes(&self) -> (Vec<(u32, usize)>, Vec<(u32, usize)>) {
        let f = |l: &Vec<Link>| l.iter().map(|l| (l.node, l.depth)).collect();
        (f(&self.a), f(&self.b))
    }

    /// `BtlObj_AddPush`: `v` added while the push is shorter than `max`, kept at most `max`.
    pub(crate) fn add_push(&mut self, v: [f32; 3], max: f32) {
        if max <= 0.0 {
            return;
        }
        if len3(self.push) < max {
            self.push = add4(self.push, [v[0], v[1], v[2], 0.0]);
            let len = len3(self.push);
            if max < len {
                self.push = scale4(self.push, max / len);
            }
        }
        self.pushed = true;
    }

    /// `BtlObj_AddSway`: `add` while below `max`, kept at most `max`.
    pub(crate) fn add_sway(&mut self, add: f32, max: f32) {
        if max <= 0.0 {
            return;
        }
        if self.sway < max {
            self.sway = (self.sway + add).min(max);
        }
        self.pushed = true;
    }

    /// `BtlObj_ChaosRand`: x = 3.9999 x (1 - x), from 0.5 again at 0.
    fn chaos(&mut self) -> f32 {
        if self.chaos == 0.0 {
            self.chaos = 0.5;
        }
        self.chaos = self.chaos * 3.9999 * (1.0 - self.chaos);
        self.chaos
    }

    /// `BtlObj_UpdateChains`: every type A link, every type B link (a parent before its children, a root in its
    /// node's parent frame, a child in its parent link's), then the push and sway die away (x 0.85) unless added
    /// to this frame. `wind` is the stage's (`BtlStage_GetLightDir`), w its strength.
    pub(crate) fn update(
        &mut self,
        nodes: &mut dyn ChainNodes,
        params: Option<&ChainParams>,
        wind: V4,
    ) {
        if let Some(p) = params {
            let mut frames = [IDENTITY; 16];
            for i in 0..self.a.len() {
                let link = self.a[i];
                let parent = if link.depth == 0 {
                    match nodes.node_mtx(link.node) {
                        Some((parent, _)) => parent,
                        None => continue,
                    }
                } else {
                    frames[link.depth - 1]
                };
                if let Some(out) = self.step_a(i, nodes, p, &parent, wind) {
                    frames[link.depth.min(15)] = out;
                }
            }
            let mut frames = [IDENTITY; 8];
            for i in 0..self.b.len() {
                let link = self.b[i];
                let parent = if link.depth == 0 {
                    match nodes.node_mtx(link.node) {
                        Some((parent, _)) => parent,
                        None => continue,
                    }
                } else {
                    frames[link.depth - 1]
                };
                if let Some(out) = self.step_b(i, nodes, p, &parent, wind) {
                    frames[link.depth.min(7)] = out;
                }
            }
        }
        // BtlObj_DecayPush.
        if self.pushed {
            self.pushed = false;
        } else {
            self.push = scale4(self.push, 0.85);
            self.sway *= 0.85;
        }
    }

    /// The velocity both steps start from: the root's node velocity turned into the world (times `rate`) and the
    /// owner's movement (times `rate`), dropped when faster than 1388.8888 / 30.
    fn base_velocity(&self, link: &Link, nodes: &dyn ChainNodes, rate: f32) -> V4 {
        let mut vel = [0.0; 4];
        if link.parent.is_none() {
            let v = nodes.velocity(link.node);
            vel = add4(
                vel,
                scale4(turn(&nodes.world(), [v[0], v[1], v[2], 0.0]), rate),
            );
        }
        let m = self.move_vec;
        vel = add4(vel, scale4([m[0], m[1], m[2], 0.0], rate));
        if 1388.8888 / 30.0 < len3(vel) {
            vel = [0.0; 4];
        }
        vel
    }

    /// The chest's side axis (node 3's world x row), (1, 0, 0) without it.
    fn side(nodes: &dyn ChainNodes) -> V4 {
        nodes
            .node_mtx(3)
            .map_or([1.0, 0.0, 0.0, 0.0], |(_, m)| m[0])
    }

    /// `BObjChainB_Step`.
    fn step_b(
        &mut self,
        i: usize,
        nodes: &mut dyn ChainNodes,
        p: &ChainParams,
        parent: &Mtx,
        wind: V4,
    ) -> Option<Mtx> {
        let (lim_a, lim_b, rate, half) = (0.523_598_75f32, 0.087_266_46f32, 0.3f32, 0.5f32);
        let mut link = self.b[i];
        let slot = link.slot;
        let yaw = f32::from(p.yaw[slot]) * PI_F / 180.0;
        let axis = [0.0, mathf_cos(yaw), mathf_sin(yaw), 0.0];
        let gravity = p.gravity_b[slot];
        let swing = p.swing_b[slot];
        let yaw_max = f32::from(p.yaw_max[slot]) * PI_F / 180.0;
        let yaw_min = f32::from(p.yaw_min[slot]) * PI_F / 180.0;
        let prev = link.rot;
        nodes.node_mtx(link.node)?;
        let inv = inverse_rt(parent);
        let vel = self.base_velocity(&link, nodes, rate);
        let mut pull = sub4([0.0; 4], vel);
        let mut force = [0.0; 4];
        let wl = len3(wind);
        if 0.001 < wl {
            force = [
                wind[0] / wl * wind[3] * 0.2,
                wind[1] / wl * wind[3] * 0.2,
                wind[2] / wl * wind[3] * 0.2,
                0.0,
            ];
        }
        match link.parent.map(|u| self.b[u]) {
            None => {
                for (k, c) in [0.1, 0.23, 0.27].into_iter().enumerate() {
                    link.idle[k] = wrap(link.idle[k] + c);
                }
            }
            Some(up) => {
                for k in 0..3 {
                    link.idle[k] = wrap(up.idle[k] - 0.9);
                }
            }
        }
        let sum = mathf_sin(link.idle[0]) * 0.25
            + 0.25
            + (mathf_sin(link.idle[1]) * 0.125 + 0.125)
            + (mathf_sin(link.idle[2]) * 0.125 + 0.125);
        force = scale4(force, sum);
        force = sub4(force, scale4(vel, half));
        force = add4(force, self.push);
        force = add4(force, turn(&inv, scale4(axis, self.sway)));
        let side = Self::side(nodes);
        let m = self.move_vec;
        let amount = len3([m[0], m[1], m[2], 0.0]) * 2.0 + len3(self.push) * 4.0 + self.sway * 4.0;
        for (k, c) in [0.9, 0.7, 1.3].into_iter().enumerate() {
            let d = amount * 0.1 * (self.chaos() * 0.3 + c);
            link.swing[k] = wrap(if link.depth & 1 != 0 {
                link.swing[k] + d
            } else {
                link.swing[k] - d
            });
        }
        let sum = mathf_sin(link.swing[0]) * 0.5 + mathf_sin(link.swing[1]) * 0.5;
        let k = amount * swing * 0.5;
        force = rotate_axis(force, side, (sum * k).clamp(-lim_a, lim_a));
        force = rotate_y(force, (mathf_sin(link.swing[2]) * k).clamp(-lim_b, lim_b));
        pull = add4(pull, force);
        pull[3] = 0.0;
        pull[1] += gravity;
        pull = turn(&inv, pull);
        let t = (len3(pull) * 0.2).max(0.1).min(0.5);
        let side = q_conjugate_by(axis, prev);
        let len = len3(pull);
        let mut dir = if 0.0001 < len {
            scale4(pull, 1.0 / len)
        } else {
            axis
        };
        let q0 = q_from_vectors(side, dir, t);
        dir = q_conjugate_by(side, q0);
        let q1 = q_from_mtx(parent);
        let q3 = quat_mul(q_inverse(q1), link.offset);
        dir = q_conjugate_by(dir, q3);
        link.offset = q1;
        let a0 = axis[2].atan2(axis[1]);
        let a1 = dir[2].atan2(dir[1]);
        let t = wrap(a1 - a0);
        if t < yaw_max {
            dir = rotate_x(dir, yaw_max - t);
        }
        if yaw_min < t {
            dir = rotate_x(dir, yaw_min - t);
        }
        let (lo, hi) = (mathf_sin(-0.1), mathf_sin(0.1));
        dir[0] = dir[0].clamp(lo, hi);
        dir = norm3(dir);
        let rot = q_from_vectors(axis, norm3(dir), 1.0);
        nodes.set_rot(link.node, rot);
        link.rot = rot;
        self.b[i] = link;
        Some(mul(&quat_to_mtx(rot), parent))
    }

    /// `BObjChainA_Step`.
    fn step_a(
        &mut self,
        i: usize,
        nodes: &mut dyn ChainNodes,
        p: &ChainParams,
        parent: &Mtx,
        wind: V4,
    ) -> Option<Mtx> {
        let (eps, rate, half) = (0.001f32, 0.2f32, 0.5f32);
        let mut link = self.a[i];
        let slot = link.slot;
        let mut axis = [p.axis[slot][0], -p.axis[slot][1], -p.axis[slot][2], 0.0];
        let deg = |d: f32| d * PI_F / 180.0;
        let limit = deg(f32::from(p.limit[slot]));
        let hinge_ofs = deg(f32::from(p.hinge_ofs[slot]));
        let hinge_limit = deg(f32::from(p.hinge_limit[slot]));
        let hinge = p.hinge[slot];
        let speed = deg(f32::from(p.speed[slot]));
        let phase = deg(f32::from(p.phase[slot]));
        let swing_max = deg(f32::from(p.swing_max[slot]));
        let stiff = f32::from(p.stiff[slot]) * 0.01;
        let (follow, rest, gravity) = (p.follow[slot], p.rest[slot], p.gravity[slot]);
        let (inertia, damping, swing) = (p.inertia[slot], p.damping[slot], p.swing[slot]);
        let len = len3(axis);
        if eps < len {
            axis = scale4(axis, 1.0 / len);
        }
        let prev = link.rot;
        nodes.node_mtx(link.node)?;
        let inv = inverse_rt(parent);
        let vel = self.base_velocity(&link, nodes, rate);
        let mut pull = sub4([0.0; 4], vel);
        let mut force = [0.0; 4];
        let wl = len3(wind);
        if eps < wl {
            force = [
                wind[0] / wl * wind[3] * rate,
                wind[1] / wl * wind[3] * rate,
                wind[2] / wl * wind[3] * rate,
                0.0,
            ];
        }
        let up = link.parent.map(|u| self.a[u]);
        match up {
            None => {
                for (k, c) in [0.1, 0.23, 0.27].into_iter().enumerate() {
                    link.idle[k] = wrap(link.idle[k] + (speed + c));
                }
            }
            Some(up) => {
                for k in 0..3 {
                    link.idle[k] = wrap(up.idle[k] - 0.9);
                }
            }
        }
        let idle = mathf_sin(link.idle[0] + phase) * 0.25
            + 0.25
            + (mathf_sin(link.idle[1] + phase) * 0.125 + 0.125)
            + (mathf_sin(link.idle[2] + phase) * 0.125 + 0.125);
        force = scale4(force, idle);
        force = sub4(force, scale4(vel, half));
        force = add4(force, self.push);
        force = add4(force, turn(&inv, scale4(axis, self.sway)));
        let mut side = Self::side(nodes);
        let m = self.move_vec;
        let amount = len3([m[0], m[1], m[2], 0.0]) + len3(self.push) * 3.0 + self.sway * 3.0;
        match up {
            None => {
                for (k, c) in [0.9, 0.7, 1.3].into_iter().enumerate() {
                    link.swing[k] = wrap(link.swing[k] + amount * 0.1 * (self.chaos() * 0.3 + c));
                }
            }
            Some(up) => {
                for k in 0..3 {
                    link.swing[k] = wrap(up.swing[k] - 1.570_796_3);
                }
            }
        }
        let sum = mathf_sin(link.swing[0]) * 0.5 + mathf_sin(link.swing[1]) * 0.5;
        let k = amount * swing * 0.1;
        force = rotate_axis(force, side, (sum * k).clamp(-swing_max, swing_max));
        force = rotate_y(
            force,
            (mathf_sin(link.swing[2]) * k).clamp(-swing_max, swing_max),
        );
        pull = add4(pull, force);
        // Vec4_Lerp(offset, vel, offset, damping).
        link.offset = add4(scale4(vel, damping), scale4(link.offset, 1.0 - damping));
        pull = add4(pull, scale4(link.offset, inertia));
        pull[3] = 0.0;
        pull[1] += gravity;
        pull = turn(&inv, pull);
        pull = add4(pull, scale4(axis, rest));
        let len = len3(pull);
        let target = if 0.0001 < len {
            q_from_vectors(axis, scale4(pull, 1.0 / len), follow)
        } else {
            IDENT
        };
        let mut rot = if 0.0 < stiff {
            let t = len3(pull) * 0.05 * stiff;
            let ang = ((1.0 - stiff) * 0.5 + t).max(0.05).min(0.5);
            q_slerp(prev, target, ang)
        } else {
            q_slerp(prev, target, 0.5)
        };
        rot = q_limit_angle(rot, limit);
        if 0.0 < hinge_limit {
            match hinge {
                0 => side = [0.0, axis[2], -axis[1], 0.0],
                1 => side = [-axis[2], 0.0, axis[0], 0.0],
                2 => side = [axis[1], -axis[0], 0.0, 0.0],
                _ => {}
            }
            let twist = q_from_vectors(side, rot, 1.0);
            let mut angle = q_angle(twist);
            if dot3(axis, twist) < 0.0 {
                angle = -angle;
            }
            angle = wrap(angle + hinge_ofs);
            let mut t = PI_F - hinge_limit;
            if t < angle.abs() {
                if angle <= 0.0 {
                    t = -t;
                }
                let angle = t - angle;
                let c = mathf_cos(angle);
                if 0.0 < c {
                    let fix = q_axis_angle(axis[0], axis[1], axis[2], angle);
                    rot = q_slerp_identity(q_conjugate_by(rot, fix), c);
                } else {
                    rot = IDENT;
                }
            }
        }
        nodes.set_rot(link.node, rot);
        link.rot = rot;
        self.a[i] = link;
        Some(mul(&quat_to_mtx(rot), parent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        rots: Vec<(u32, Q)>,
        vel: [f32; 3],
    }

    impl ChainNodes for Fake {
        fn node_mtx(&self, id: u32) -> Option<(Mtx, Mtx)> {
            (id != 0x99).then_some((IDENTITY, IDENTITY))
        }
        fn velocity(&self, _: u32) -> [f32; 3] {
            self.vel
        }
        fn world(&self) -> Mtx {
            IDENTITY
        }
        fn set_rot(&mut self, id: u32, rot: Q) {
            self.rots.push((id, rot));
        }
    }

    fn part(node: u16, pop: u16) -> ModelPart {
        ModelPart {
            node,
            pop,
            enabled: false,
            flags: 0,
            offset_a: [0.0; 4],
            offset_b: [0.0; 4],
            rest: [0.0; 4],
            extent: [0.0; 4],
            batches: Vec::new(),
        }
    }

    fn params() -> ChainParams {
        let mut p = ChainParams::default();
        p.node_a[..4].copy_from_slice(&[0x4F, 0x50, 0x57, 0x58]);
        for s in 0..4 {
            p.axis[s] = [0.0, -1.0, 0.0];
            p.limit[s] = 30;
            p.follow[s] = 0.6;
            p.damping[s] = 0.5;
            p.swing[s] = 1.0;
            p.swing_max[s] = 20;
        }
        p.node_b[0] = 0x47;
        p.yaw[0] = 90;
        p.yaw_max[0] = -45;
        p.yaw_min[0] = 45;
        p
    }

    #[test]
    fn the_links_follow_the_bound_records_hierarchy() {
        // As Goku's: two single links, then a two-link chain (its pop of 2 closes it); a non-chain node between.
        let parts = [
            part(3, 0),
            part(0x57, 0),
            part(0x58, 2),
            part(0x4F, 1),
            part(4, 3),
            part(0x50, 1),
            part(0x47, 0),
        ];
        let c = Chains::new(Some(&params()), &parts);
        let (a, b) = c.nodes();
        assert_eq!(a, [(0x57, 0), (0x58, 1), (0x4F, 0), (0x50, 0)]);
        assert_eq!(b, [(0x47, 0)]);
        assert_eq!(c.a[1].parent, Some(0));
        assert_eq!(c.a[3].parent, None);
        assert!(Chains::new(None, &parts).nodes().0.is_empty());
    }

    #[test]
    fn pushes_and_sway_are_capped_and_die_away() {
        let mut c = Chains::default();
        c.add_push([3.0, 0.0, 4.0], 2.0);
        assert!((len3(c.push) - 2.0).abs() < 1e-6);
        c.add_push([10.0, 0.0, 0.0], 2.0);
        assert!((len3(c.push) - 2.0).abs() < 1e-6);
        c.add_sway(0.2, 2.0);
        c.add_sway(5.0, 2.0);
        assert_eq!(c.sway, 2.0);
        let mut f = Fake {
            rots: Vec::new(),
            vel: [0.0; 3],
        };
        // A frame that pushed keeps them; the next one lets them die away.
        c.update(&mut f, None, [0.0; 4]);
        assert_eq!(c.sway, 2.0);
        c.update(&mut f, None, [0.0; 4]);
        assert_eq!(c.sway, 1.7);
        // The noise: the logistic map from 0.5.
        let mut c = Chains::default();
        assert_eq!(c.chaos(), 0.5 * 3.9999 * 0.5);
    }

    #[test]
    fn a_link_hangs_towards_its_rest_and_swings_when_pushed() {
        let parts = [part(0x4F, 1), part(0x57, 0), part(0x58, 2), part(0x47, 1)];
        let p = params();
        let mut c = Chains::new(Some(&p), &parts);
        let mut f = Fake {
            rots: Vec::new(),
            vel: [0.0; 3],
        };
        // At rest with nothing pushing: a rotation of at most the 30 degree limit.
        c.update(&mut f, Some(&p), [0.0; 4]);
        assert_eq!(f.rots.len(), 4);
        for (_, q) in &f.rots[..3] {
            assert!(q_angle(*q) <= 30f32.to_radians() + 1e-4, "{q:?}");
        }
        let still = f.rots[0].1;
        // Pushed sideways: the hair turns further from where it hung.
        for _ in 0..10 {
            f.rots.clear();
            c.add_push([2.0, 0.0, 0.0], 5.0);
            c.update(&mut f, Some(&p), [1.0, 0.0, 0.0, 1.3]);
        }
        let pushed = f.rots[0].1;
        assert!(
            still.iter().zip(pushed).any(|(a, b)| (a - b).abs() > 1e-3),
            "{still:?} {pushed:?}"
        );
        // A missing node is skipped.
        let mut c = Chains::new(Some(&p), &[part(0x99, 0)]);
        c.a = vec![Link {
            node: 0x99,
            ..c.a.first().copied().unwrap_or(Link {
                rot: IDENT,
                offset: [0.0; 4],
                slot: 0,
                node: 0x99,
                depth: 0,
                parent: None,
                idle: [0.0; 3],
                swing: [0.0; 3],
            })
        }];
        f.rots.clear();
        c.update(&mut f, Some(&p), [0.0; 4]);
        assert!(f.rots.is_empty());
    }
}
