//! The ki blasts of types 2 and 3 (src/battle/eft_struggle.c, src/battle/eft_chain.c): the thrown object
//! (`EftKiObj`, list kind 1) flies under gravity from the firing node until it ends anywhere, then breaks into
//! fragments for 15 frames; a deflection or reflection turns it (the new direction becoming its whole velocity, as
//! in the original). The bomb (`EftKiBomb`, list kind 2, two per shot) is thrown with a jitter of the VU0
//! generator, falls, bounces off the stage up to three times (halving its speed), then lies until its life is out;
//! it explodes then, after 300 frames, below the stage, within 28 of the opponent or when it hits a fighter: a
//! 15-frame record of radius 28. Both publish blast records of their ki blast's parameters.
//!
//! Not ported: the models, the fragments' and the explosion's drawing and sounds (their libc `rand()` draws are
//! kept).

use crate::btl_blast::{Arg, HitRec, Tag};
use crate::btl_chr::{Chr, add, dot, length, scale3, sub};
use crate::rng::{LibcRandom, Vu0Random};

/// `EftKiObjMgr_Init` / `EftKiBombMgr_Init`: tasks per fighter.
pub(crate) const LIST_MAX: usize = 10;

// EFT_KIBOMB_* / EFT_KIPROP_*.
const K_MOVED: u32 = 0x2;
const K_DEFLECTED: u32 = 0x4;
const K_KILL: u32 = 0x8;
const K_BROKEN: u32 = 0x10;
const K_HITS: u32 = 0x80;
const K_EXPLODE: u32 = 0x100;
const K_REST: u32 = 0x200;
const K_EXPLODED: u32 = 0x400;

// EFT_TASK_* (EFT_R_HIT_*).
const T_HIT_CHAR: u32 = 0x1;
const T_GUARDED: u32 = 0x2;
const T_HIT_STAGE: u32 = 0x4;
const T_OUT: u32 = 0x10;
const T_DEFLECTED: u32 = 0x40;
const T_REFLECTED: u32 = 0x80;

/// A thrown object or a bomb (`EftKiBomb`, the object's head).
#[derive(Clone, Debug)]
pub(crate) struct KiObj {
    pub id: u32,
    pub tag: Tag,
    /// The ki blast's parameters (`arg`); `owner` changes hands when it is deflected.
    pub arg: Arg,
    pub flags: u32,
    pos: [f32; 3],
    prev: [f32; 3],
    /// The velocity (`pose.dir`).
    vel: [f32; 3],
    /// The stage contact's normal (`contact.row[3]`).
    pub contact_nrm: [f32; 3],
    life: f32,
    speed: f32,
    radius: f32,
    gravity: f32,
    max_life: i32,
    bounces: i32,
    blast: f32,
    frag_timer: i32,
    bomb: bool,
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    crate::btl_blast::norm(v)
}

impl KiObj {
    /// `EftKiObj_Init` / `EftKiObj_Launch` (`bomb` false) or `EftKiBomb_Init` / `EftKiBomb_Launch`: at the firing
    /// node, last at node 0x11, along the ki blast's direction times its speed (a bomb's direction jittered later,
    /// `jitter`). The spins are drawn from libc rand() (two draws, kept by the caller).
    pub(crate) fn new(id: u32, me: &Chr, arg: Arg, bomb: bool) -> Self {
        let pos = me.obj.node_pos(arg.node);
        let mut o = Self {
            id,
            tag: Tag::default(),
            arg,
            flags: K_HITS,
            pos,
            prev: me.obj.node_pos(0x11),
            vel: arg.dir,
            contact_nrm: [0.0; 3],
            life: arg.life as f32,
            speed: arg.speed,
            radius: arg.radius * 5.0,
            gravity: 0.5,
            max_life: 0,
            bounces: 0,
            blast: 0.0,
            frag_timer: 15,
            bomb,
        };
        if bomb {
            o.radius = 5.0;
            o.max_life = 300;
            o.blast = 15.0;
        } else {
            o.vel = scale3(o.vel, o.speed);
        }
        o
    }

    /// The bomb's throw: x and z +-0.2, y up to 0.3 upwards from the VU0 generator, normalised, times the speed.
    pub(crate) fn jitter(&mut self, vu0: &mut Vu0Random) {
        let mut d = self.vel;
        d[0] += vu0.float_range(-0.2, 0.2);
        d[1] -= vu0.float_range(0.0, 0.3);
        d[2] += vu0.float_range(-0.2, 0.2);
        self.vel = scale3(norm(d), self.speed);
    }

    /// `EftKiObj_Step` / `EftKiBomb_Move` and the record. `opp_pos` is the opponent's position
    /// (`BtlCharApi_GetPos`), `bottom` the stage's bottom.
    pub(crate) fn update(
        &mut self,
        opp_pos: [f32; 3],
        bottom: f32,
        libc: &mut LibcRandom,
        paused: bool,
    ) -> Option<HitRec> {
        if self.bomb {
            self.step_bomb(opp_pos, bottom, libc);
        } else {
            self.step_obj();
        }
        let rec = if self.flags & K_HITS != 0 && !paused {
            let mut rec = HitRec::blast(
                self.arg,
                self.id,
                self.pos,
                self.prev,
                self.vel,
                self.radius,
            );
            rec.turned = self.flags & 0x20 != 0;
            if self.bomb {
                rec.flags |= 0x202;
                rec.unk54 = self.flags & K_REST != 0;
            } else {
                rec.flags |= 0x400;
            }
            Some(rec)
        } else {
            None
        };
        rec
    }

    fn step_obj(&mut self) {
        if self.flags & K_BROKEN == 0 {
            if self.flags & K_MOVED != 0 {
                self.prev = self.pos;
            }
            self.flags |= K_MOVED;
            self.pos = add(self.pos, self.vel);
            self.vel[1] += self.gravity;
        }
        if self.frag_timer <= 0 {
            self.flags |= K_KILL;
        }
        // EftKiObj_StepFrags: the fragments' timer.
        if self.flags & K_BROKEN != 0 {
            self.frag_timer -= 1;
            if self.frag_timer <= 0 {
                self.frag_timer = 0;
            }
        }
    }

    fn step_bomb(&mut self, opp_pos: [f32; 3], bottom: f32, libc: &mut LibcRandom) {
        if self.flags & K_MOVED != 0 {
            self.prev = self.pos;
        }
        self.flags |= K_MOVED;
        self.pos = add(self.pos, self.vel);
        self.vel[1] += self.gravity;
        self.max_life -= 1;
        if self.max_life <= 0 {
            self.flags |= K_EXPLODE;
        }
        if bottom < self.pos[1] {
            self.flags |= K_EXPLODE;
        }
        if length(sub(opp_pos, self.pos)) < 28.0 {
            self.flags |= K_EXPLODE;
        }
        if self.flags & K_REST != 0 {
            self.life -= 1.0;
        }
        if self.flags & K_EXPLODED == 0 && (self.life <= 0.0 || self.flags & K_EXPLODE != 0) {
            self.life = 0.0;
            // The explosion effect and its sound (libc rand()).
            libc.rand();
            self.radius = 28.0;
            self.flags |= K_EXPLODED;
        }
        if self.flags & K_EXPLODED != 0 {
            self.blast -= 1.0;
            if self.blast <= 0.0 {
                self.blast = 0.0;
                self.flags |= K_KILL;
            }
        }
    }

    pub(crate) fn dead(&self) -> bool {
        self.flags & K_KILL != 0
    }

    /// `EftKiObj_PostUpdate`: an end report breaks the object where it was reported (the fragments draw libc
    /// rand() 20 times); a deflection or reflection, once, turns it (`deflect_dir` draws the fighters' generator).
    pub(crate) fn post_obj(
        &mut self,
        libc: &mut LibcRandom,
        deflect_dir: impl FnOnce(usize) -> [f32; 3],
    ) {
        if self.tag.flags & 0x3F != 0 {
            self.life = 0.0;
            if self.flags & K_BROKEN == 0 {
                self.pos = self.tag.pos;
                for _ in 0..20 {
                    libc.rand();
                }
                self.flags |= K_BROKEN;
                self.flags &= !K_HITS;
            }
        } else if self.tag.flags & 0xC0 != 0 && self.flags & K_DEFLECTED == 0 {
            // EftKiObj_Turn (its position, a lerp toward the last one, is overwritten).
            let dir = deflect_dir(self.arg.owner ^ 1);
            if self.tag.flags & T_DEFLECTED != 0 {
                self.vel = dir;
                self.arg.owner ^= 1;
                self.flags |= K_DEFLECTED;
            } else if self.tag.flags & T_REFLECTED != 0 {
                self.vel = dir;
                self.arg.owner ^= 1;
                self.life = self.arg.life as f32;
            }
            self.pos = self.tag.pos;
            self.flags |= 0x20;
            self.tag.flags &= !0xC0;
        }
    }

    /// `EftKiBomb_HandleHits`: guarded, it explodes without a record; a fighter hit, it explodes; the stage or its
    /// edge bounces it (mirrored on the contact normal, or toward the stage's axis at the edge; speed halved) up to
    /// three times, then it lies there (its resting angle drawn from libc rand()).
    pub(crate) fn post_bomb(&mut self, libc: &mut LibcRandom) {
        let f = self.tag.flags;
        if f & T_GUARDED != 0 {
            self.pos = self.tag.pos;
            self.flags = (self.flags | K_EXPLODE) & !K_HITS;
        } else if f & T_HIT_CHAR != 0 {
            self.pos = self.tag.pos;
            self.flags |= K_EXPLODE;
        } else if f & (T_HIT_STAGE | T_OUT) != 0 {
            let t = self.tag.pos;
            let v = norm(self.vel);
            self.vel = v;
            self.pos = sub(t, scale3(v, self.radius));
            if self.bounces < 3 {
                let n = if f & T_OUT != 0 {
                    norm(scale3(self.pos, -1.0))
                } else {
                    self.contact_nrm
                };
                // EftKiBomb_Reflect: mirrored on the plane of -n, normalised.
                let m = scale3(n, -1.0);
                let d = dot(v, m) * -2.0;
                let refl = norm(add(v, scale3(m, d)));
                self.speed *= 0.5;
                let r = self.radius + 1.0;
                self.pos = add(t, scale3(n, r));
                self.vel = scale3(refl, self.speed);
                self.bounces += 1;
                self.tag.flags &= !(T_HIT_STAGE | T_OUT);
            } else {
                self.vel = [0.0; 3];
                self.gravity = 0.0;
                self.pos = add(t, scale3(self.contact_nrm, self.radius));
                if self.flags & K_REST == 0 {
                    libc.rand();
                    self.tag.flags &= !T_HIT_STAGE;
                }
                self.flags |= K_REST;
            }
        }
    }

    /// `post_obj` with the scene's view of the battle (`BtlCharApi_GetDeflectDir`).
    pub(crate) fn post_obj_with(&mut self, w: &mut crate::btl_blast::World) {
        let mut libc = std::mem::replace(w.libc, LibcRandom::new(0));
        self.post_obj(&mut libc, |by| crate::btl_blast::deflect_dir(w, by));
        *w.libc = libc;
    }

    /// The positions of a fighter's objects, for tests.
    #[cfg(test)]
    pub(crate) fn pos(&self) -> [f32; 3] {
        self.pos
    }
}
