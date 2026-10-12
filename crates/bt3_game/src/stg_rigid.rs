//! The rigid bodies of the falling stage debris (src/battle/stg_rigid.c over the sphere dynamics of
//! src/sys/rigid.c): a pool of 128 numbered bodies, the active ones stepped in creation order, ten sub-steps of
//! gravity and drag a frame, kept inside the stage cylinder.
//!
//! The stage's debris bodies have no hit buffer (`StgRigid_Create(.., 0)`), so they never collect stage triangles,
//! never take a contact force, never come to rest and keep the open-air drag; that part of the module is not
//! ported.

use crate::rng::{LibcRandom, Vu0Random};

/// `STG_RIGID_MAX`: the pool; `StgRigid_AllocNode` hands out at most one less.
const MAX: usize = 128;

/// `STG_RIGID_DT`: 1 / 60 and a little, so that ten steps pass the 1 / 6 of a frame.
const DT: f32 = 1.0 / 60.0 + 0.00001;

/// The drag in the open air (`StgRigidList_BeginFrame`).
const DAMP: f32 = 0.0015;

type Mat3 = [[f32; 3]; 3];

/// `Mat3_MulVecRows`: v * m (body to world).
fn mul_rows(v: [f32; 3], m: &Mat3) -> [f32; 3] {
    std::array::from_fn(|j| v[0] * m[0][j] + v[1] * m[1][j] + v[2] * m[2][j])
}

/// `Mat3_MulVecCols`: m * v (world to body).
fn mul_cols(v: [f32; 3], m: &Mat3) -> [f32; 3] {
    std::array::from_fn(|i| v[0] * m[i][0] + v[1] * m[i][1] + v[2] * m[i][2])
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 { v.map(|c| c / l) } else { v }
}

/// `Mat3_Orthonormalize`: row 2 kept (normalised), row 0 = row 1 x row 2, row 1 = row 2 x row 0.
fn orthonormalize(m: &mut Mat3) {
    let z = normalize(m[2]);
    m[2] = z;
    let x = normalize(cross(m[1], z));
    m[0] = x;
    m[1] = cross(z, x);
}

/// `RigidBody`: a sphere of uniform density.
#[derive(Clone, Copy, Debug)]
struct Body {
    mass: f32,
    inertia: [f32; 3],
    center: [f32; 3],
    pos: [f32; 3],
    rot: Mat3,
    vel: [f32; 3],
    ang_vel: [f32; 3],
    force: [f32; 3],
    torque: [f32; 3],
    ang_vel_body: [f32; 3],
    radius: f32,
}

impl Body {
    /// `Rigid_Init` (the density cancels out of every motion the stage gives it) then `Rigid_SetTransform` with
    /// no rotation.
    fn new(pos: [f32; 3], radius: f32) -> Self {
        let mass = radius * 2.356_194_5 * radius * radius;
        let inertia = mass * radius * radius * 2.0 / 5.0 * 20.0;
        Self {
            mass,
            inertia: [inertia; 3],
            center: [0.0; 3],
            pos,
            rot: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            vel: [0.0; 3],
            ang_vel: [0.0; 3],
            force: [0.0; 3],
            torque: [0.0; 3],
            ang_vel_body: [0.0; 3],
            radius,
        }
    }

    /// `Rigid_UpdateCenter`: the centre of mass is the body origin.
    fn update_center(&mut self) {
        let c = mul_rows([0.0; 3], &self.rot);
        self.center = std::array::from_fn(|k| c[k] + self.pos[k]);
    }

    /// `Rigid_AddForceAt`.
    fn add_force_at(&mut self, point: [f32; 3], f: [f32; 3]) {
        let arm = std::array::from_fn(|k| point[k] - self.center[k]);
        let t = cross(arm, f);
        for k in 0..3 {
            self.force[k] += f[k];
            self.torque[k] += t[k];
        }
    }

    /// `Rigid_AddGravity`: +Y is down.
    fn add_gravity(&mut self) {
        let center = self.center;
        self.add_force_at(center, [0.0, self.mass * 9.8, 0.0]);
    }

    /// `Rigid_Damp`.
    fn damp(&mut self, k: f32) {
        for c in 0..3 {
            self.vel[c] -= self.vel[c] * k;
            self.ang_vel_body[c] -= self.ang_vel_body[c] * k;
        }
    }

    /// `Rigid_Integrate`: the force and torque into the velocities (Euler's equations in body space).
    fn integrate(&mut self, dt: f32) {
        let k = dt / self.mass;
        for c in 0..3 {
            self.vel[c] += self.force[c] * k;
        }
        let t = mul_cols(self.torque, &self.rot);
        let (w, i) = (self.ang_vel_body, self.inertia);
        let a = [
            (t[0] + (i[1] - i[2]) * w[1] * w[2]) / i[0],
            (t[1] + (i[2] - i[0]) * w[2] * w[0]) / i[1],
            (t[2] + (i[0] - i[1]) * w[0] * w[1]) / i[2],
        ];
        for c in 0..3 {
            self.ang_vel_body[c] += dt * a[c] * 0.999;
        }
        self.ang_vel = mul_rows(self.ang_vel_body, &self.rot);
        self.force = [0.0; 3];
        self.torque = [0.0; 3];
    }

    /// `Rigid_Step`: forward Euler of the position, first-order update of the rotation.
    fn step(&mut self, dt: f32) {
        self.integrate(dt);
        let pos = std::array::from_fn(|k| self.pos[k] + dt * self.vel[k]);
        let a = self.ang_vel.map(|w| dt * w);
        // Mat3_SetSkew.
        let skew = [[0.0, a[2], -a[1]], [-a[2], 0.0, a[0]], [a[1], -a[0], 0.0]];
        let r = self.rot;
        let mut rot: Mat3 = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                r[i][j] + (r[i][0] * skew[0][j] + r[i][1] * skew[1][j] + r[i][2] * skew[2][j])
            })
        });
        orthonormalize(&mut rot);
        self.pos = pos;
        self.rot = rot;
    }

    /// `Rigid_SetVelocity`: the linear velocity, and the angular velocity's X and Z taken out of the body's
    /// (its Y kept), as the original computes it.
    fn set_velocity(&mut self, vel: [f32; 3], spin: [f32; 3]) {
        let b = mul_cols(mul_rows(spin, &self.rot), &self.rot);
        let y = mul_cols([0.0, spin[1], 0.0], &self.rot);
        for k in 0..3 {
            self.ang_vel_body[k] += y[k] - b[k];
        }
        self.vel = vel;
    }

    /// `Rigid_GetMatrix`: rotation rows, position in row 3.
    fn matrix(&self) -> [[f32; 4]; 4] {
        let r = self.rot;
        [
            [r[0][0], r[0][1], r[0][2], 0.0],
            [r[1][0], r[1][1], r[1][2], 0.0],
            [r[2][0], r[2][1], r[2][2], 0.0],
            [self.pos[0], self.pos[1], self.pos[2], 1.0],
        ]
    }

    /// `StgRigid_ClampToStage`: outside the cylinder of the stage's inner radius, x and z pulled back and the
    /// spin halved.
    fn clamp_to_stage(&mut self, inner_radius: f32) {
        let dist = self.pos[0].hypot(self.pos[2]) + self.radius;
        if inner_radius < dist {
            let k = inner_radius / dist;
            self.pos[0] *= k;
            self.pos[2] *= k;
            for c in 0..3 {
                self.ang_vel[c] *= 0.5;
                self.ang_vel_body[c] *= 0.5;
            }
        }
    }
}

/// `gStgRigid`: the pool, the free list (last freed first) and the active list in creation order.
#[derive(Clone, Debug, Default)]
pub(crate) struct StgRigid {
    bodies: Vec<Option<Body>>,
    active: Vec<usize>,
    free: Vec<usize>,
    used: usize,
}

impl StgRigid {
    /// `StgRigid_Reset`: every body forgotten.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// `StgRigid_Create`: a new body at `pos`, unrotated and at rest, at the end of the active list; its id
    /// (1..128), -1 when the pool is used up.
    pub(crate) fn create(&mut self, pos: [f32; 3], radius: f32) -> i32 {
        let n = if let Some(n) = self.free.pop() {
            n
        } else if self.used < MAX - 1 {
            self.used += 1;
            self.used - 1
        } else {
            return -1;
        };
        if self.bodies.len() <= n {
            self.bodies.resize(n + 1, None);
        }
        self.bodies[n] = Some(Body::new(pos, radius));
        self.active.push(n);
        n as i32 + 1
    }

    fn find(&mut self, id: i32) -> Option<&mut Body> {
        let n = usize::try_from(id - 1).ok()?;
        if !self.active.contains(&n) {
            return None;
        }
        self.bodies.get_mut(n)?.as_mut()
    }

    /// `StgRigid_Launch`: thrown along the horizontal part of `push` (its squared length over 1200 scales it by
    /// 1200 over that), up by the material's speed (-10 / -17 / -21) times 0.5..1, each component randomly
    /// boosted, and a random spin.
    pub(crate) fn launch(
        &mut self,
        id: i32,
        push: [f32; 3],
        material: i32,
        vu0: &mut Vu0Random,
        libc: &mut LibcRandom,
    ) {
        let mut spin = [
            vu0.float_range(0.0, 1.0) * -0.3,
            vu0.float_range(0.0, 1.0) * -0.35,
            vu0.float_range(0.0, 1.0) * -0.25,
        ];
        let Some(body) = self.find(id) else {
            return;
        };
        let mut vel = [push[0], 0.0, push[2]];
        let d = vel[0] * vel[0] + vel[2] * vel[2];
        if 1200.0 < d {
            let d = 1200.0 / d;
            vel[0] *= d;
            vel[2] *= d;
        }
        let up = match material {
            1 => -17.0,
            2 => -21.0,
            3 => {
                spin = [0.0; 3];
                -10.0
            }
            _ => -10.0,
        };
        vel[1] = up * 0.5 * vu0.float_range(0.0, 1.0) + up * 0.5;
        for s in &mut spin {
            if libc.rand() & 1 != 0 {
                *s = -*s;
            }
        }
        for v in &mut vel {
            if libc.rand() & 1 != 0 {
                *v += *v * vu0.float_range(0.0, 1.0);
            }
        }
        body.set_velocity(vel, spin);
    }

    /// `StgRigid_Release`: off the active list, onto the free list.
    pub(crate) fn release(&mut self, id: i32) {
        let Some(n) = usize::try_from(id - 1).ok() else {
            return;
        };
        if let Some(i) = self.active.iter().position(|&a| a == n) {
            self.active.remove(i);
            self.free.push(n);
        }
    }

    /// `StgRigid_Update` (not while paused): ten sub-steps of gravity and drag, then the stage clamp.
    pub(crate) fn update(&mut self, inner_radius: f32) {
        let mut t = 0.0f32;
        loop {
            t += DT;
            for &n in &self.active {
                if let Some(b) = self.bodies[n].as_mut() {
                    b.update_center();
                    b.add_gravity();
                    b.damp(DAMP);
                    b.step(DT);
                }
            }
            if t > 1.0 / 6.0 {
                break;
            }
        }
        for &n in &self.active {
            if let Some(b) = self.bodies[n].as_mut() {
                b.clamp_to_stage(inner_radius);
            }
        }
    }

    /// `StgRigid_GetMatrix`: the body's world matrix, `None` when the id names none.
    pub(crate) fn matrix(&mut self, id: i32) -> Option<[[f32; 4]; 4]> {
        self.find(id).map(|b| b.matrix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launched_body_rises_falls_and_stays_in_the_stage() {
        let mut r = StgRigid::default();
        let id = r.create([0.0, -50.0, 0.0], 10.0);
        assert_eq!(id, 1);
        let (mut vu0, mut libc) = (Vu0Random::default(), LibcRandom::new(1));
        r.launch(id, [100.0, 0.0, 0.0], 0, &mut vu0, &mut libc);
        let start = r.matrix(id).unwrap()[3];
        r.update(500.0);
        let one = r.matrix(id).unwrap()[3];
        // -Y is up: thrown up at 5..10 (or boosted), it rises and moves along +x.
        assert!(one[1] < start[1] && one[0] > start[0]);
        for _ in 0..200 {
            r.update(500.0);
        }
        let m = r.matrix(id).unwrap();
        assert!(m[3][1] > start[1], "gravity pulls it down");
        assert!(m[3][0].hypot(m[3][2]) <= 500.0);
        // The rotation stays orthonormal.
        let row = |i: usize| [m[i][0], m[i][1], m[i][2]];
        assert!((cross(row(0), row(1))[2] - row(2)[2]).abs() < 1e-4);
        r.release(id);
        assert!(r.matrix(id).is_none());
        assert_eq!(r.create([0.0; 3], 1.0), 1);
    }
}
