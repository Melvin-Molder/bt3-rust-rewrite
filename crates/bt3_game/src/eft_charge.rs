//! The charge aura (`EftCharge_*`, src/battle/eft_struggle.c): the emitter set of the character pack's entry 2 at
//! the fighter's node 3, pointing up, while request 7 (charging ki) holds; request 8 adds its burst group and a
//! screen shock wave. A kind 0 manager of the fighters' effect layer, four auras per fighter.

use bt3_data::eft_pack::EmitSet;

use crate::eft::{EmitState, Modules, Nodes, SpawnCtx, V3, spawn};
use crate::rng::{LibcRandom, Vu0Random};
use crate::scr_warp::ScrWarps;

/// `EftChargeMgr_Init`: four auras per fighter.
const AURAS: usize = 4;
/// The `SetRef::list` of a fighter's charge set.
pub(crate) const LIST: usize = 10;

// EFT_CHARGE_*.
const ACTIVE: u32 = 0x01;
const END: u32 = 0x02;
const KILL: u32 = 0x04;
const BURST: u32 = 0x08;
const BURST_DONE: u32 = 0x10;
const ALIVE: u32 = 0x20;
const POSTED: u32 = 0x40;

/// `EftCharge`.
#[derive(Clone, Debug)]
struct Charge {
    flags: u32,
    scale: f32,
    pos: V3,
    state: EmitState,
    nodes: Nodes,
    mask: u32,
    life: f32,
    timer: f32,
}

/// A fighter's auras and its slot (`EftCharSlot_Get0`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Charges {
    list: Vec<Charge>,
    slot: Option<usize>,
}

impl Charges {
    /// Live auras.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.list.len()
    }

    /// `EftCharge_IsActive`: the fighter's slot holds an aura.
    pub(crate) fn is_active(&self) -> bool {
        self.slot
            .and_then(|i| self.list.get(i))
            .is_some_and(|c| c.flags & ACTIVE != 0)
    }

    /// `EftCharge_Start`: an aura at node 3 (scale height / 19.35, at least 0.5), unless the fighter has one.
    pub(crate) fn start(&mut self, set: &EmitSet, height: f32, node3: V3) {
        if self.slot.is_some() || self.list.len() >= AURAS {
            return;
        }
        let mut nodes = Nodes::default();
        nodes.set_node(0, 3);
        self.list.push(Charge {
            flags: ACTIVE | ALIVE,
            scale: (height / 19.35).max(0.5),
            pos: node3,
            state: EmitState::new(set),
            nodes,
            mask: 1,
            life: f32::from(set.head.end_frames),
            timer: 0.0,
        });
        self.slot = Some(self.list.len() - 1);
    }

    fn slot_mut(&mut self) -> Option<&mut Charge> {
        self.slot
            .and_then(|i| self.list.get_mut(i))
            .filter(|c| c.flags & ACTIVE != 0)
    }

    /// `EftCharge_Burst` (request 8).
    pub(crate) fn burst(&mut self) {
        if let Some(c) = self.slot_mut() {
            c.flags |= BURST;
        }
    }

    /// `EftCharge_Stop` (request 7 ended).
    pub(crate) fn stop(&mut self) {
        if let Some(c) = self.slot_mut() {
            c.flags |= END;
        }
    }

    /// `EftCharge_Update` of every aura (not while paused): it follows node 3; the burst adds group 1 and a shock
    /// wave once; an ending aura lasts the set's end frames, then frees the fighter's slot.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        fx: &mut Modules,
        warps: &mut ScrWarps,
        side: usize,
        node_pos: &dyn Fn(u32) -> V3,
        ctx: &SpawnCtx,
        paused: bool,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        let mut i = 0;
        while i < self.list.len() {
            let c = &mut self.list[i];
            if !paused {
                c.pos = node_pos(3);
                if c.flags & BURST != 0 && c.flags & BURST_DONE == 0 {
                    c.mask |= 2;
                    c.nodes.set_node(1, 3);
                    warps.spawn(side, c.pos, 0.8, 10.0, 50.0, 10.0, 0.5, vu0);
                    c.flags |= BURST_DONE;
                }
                c.nodes.refresh(node_pos);
                spawn_parts(c, fx, ctx, libc, vu0);
                if c.flags & END != 0 {
                    c.timer += 1.0;
                    if c.timer >= c.life {
                        c.flags |= KILL;
                    }
                }
            }
            if c.flags & KILL != 0 {
                self.list.remove(i);
                // EftCharge_Term: EftCharSlot_Clear0.
                self.slot = None;
            } else {
                i += 1;
            }
        }
    }

    /// `EftCharge_PostUpdate`: from the second frame on, whether anything is alive; the phases clear.
    pub(crate) fn post_update(&mut self, set: &EmitSet, fx: &mut Modules) {
        for c in &mut self.list {
            if c.flags & POSTED != 0 {
                if c.state.update_alive(set, fx) == 0 {
                    c.flags &= !ALIVE;
                } else {
                    c.flags |= ALIVE;
                }
                c.mask = 0;
            }
            c.flags |= POSTED;
        }
    }

    /// `EftCharge_Reset`.
    pub(crate) fn reset(&mut self, fx: &mut Modules) {
        for c in &mut self.list {
            c.state.kill_all(fx);
        }
        self.list.clear();
        self.slot = None;
    }
}

/// `EftCharge_SpawnParts`: every emitter of the set gets this frame's flags, at node 3 pointing up.
fn spawn_parts(
    c: &mut Charge,
    fx: &mut Modules,
    ctx: &SpawnCtx,
    libc: &mut LibcRandom,
    vu0: &mut Vu0Random,
) {
    let set = ctx.set;
    for kind in 0..0x13u8 {
        if set.head.mask & (1 << kind) == 0 {
            continue;
        }
        let Some(g) = set.groups[usize::from(kind)] else {
            continue;
        };
        for sub in 0..g.count {
            let n = g.first + sub;
            let f = c
                .state
                .flags_from_mask(set, kind, n, c.flags & END != 0, false, c.mask);
            if f != 0 {
                let up = [0.0, -1.0, 0.0];
                spawn(
                    fx,
                    &mut c.state,
                    &c.nodes,
                    ctx,
                    c.pos,
                    up,
                    kind,
                    sub,
                    f,
                    c.scale,
                    libc,
                    vu0,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_aura_holds_while_charging_and_bursts_once() {
        let mut set = EmitSet::default();
        set.head.end_frames = 2;
        let assets = crate::eft::SetAssets::default();
        let ctx = SpawnCtx {
            chr: 0,
            set: &set,
            assets: &assets,
            set_ref: crate::eft::SetRef {
                side: 0,
                list: LIST,
            },
            char_scale: [1.0; 2],
            opp_chara_flags: 0,
            opp_node3: [0.0; 3],
        };
        let node_pos = |n: u32| [0.0, n as f32, 100.0];
        let mut c = Charges::default();
        let mut fx = Modules::default();
        let mut warps = ScrWarps::default();
        let (mut libc, mut vu0) = (LibcRandom::new(1), Vu0Random::default());
        c.start(&set, 38.7, [0.0; 3]);
        c.start(&set, 38.7, [0.0; 3]);
        assert_eq!(c.count(), 1);
        assert_eq!(c.list[0].scale, 2.0);
        assert!(c.is_active());
        let mut frame = |c: &mut Charges, warps: &mut ScrWarps| {
            c.update(
                &mut fx, warps, 0, &node_pos, &ctx, false, &mut libc, &mut vu0,
            );
            c.post_update(&set, &mut fx);
        };
        frame(&mut c, &mut warps);
        assert_eq!(c.list[0].pos, [0.0, 3.0, 100.0]);
        c.burst();
        frame(&mut c, &mut warps);
        c.burst();
        frame(&mut c, &mut warps);
        assert_eq!(warps.count(), 1);
        for _ in 0..10 {
            frame(&mut c, &mut warps);
        }
        assert_eq!(c.count(), 1);
        c.stop();
        frame(&mut c, &mut warps);
        assert!(c.is_active());
        frame(&mut c, &mut warps);
        assert_eq!(c.count(), 0);
        assert!(!c.is_active());
    }
}
