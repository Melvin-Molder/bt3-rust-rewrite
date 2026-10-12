//! The rush-finish burst (`EftRushBurst_*`, src/battle/eft_trail.c): the emitter set of the character pack's
//! entry 7 at a node of the fighter, along the attack's launch direction. Request 0x28 starts it (phase 0),
//! 0x29 adds its second stage (phase 1), 0x2A ends it after the set's end frames. A kind 5 manager of the
//! fighters' effect layer, four bursts per fighter.

use crate::eft::{EmitState, Modules, Nodes, SpawnCtx, V3, norm, spawn};
use crate::rng::{LibcRandom, Vu0Random};

/// `EftRushBurstMgr_Init`: four bursts per fighter.
const BURSTS: usize = 4;
/// The `SetRef::list` of a fighter's rush burst set.
pub(crate) const LIST: usize = 9;

// EFT_RUSHBURST_*.
const ACTIVE: u32 = 0x01;
const END: u32 = 0x02;
const STAGE2: u32 = 0x04;
const STAGE2D: u32 = 0x08;
const KILL: u32 = 0x10;
const ALIVE: u32 = 0x20;
const POSTED: u32 = 0x40;

/// `EftRushBurst`.
#[derive(Clone, Debug)]
struct Burst {
    node: u32,
    scale: f32,
    flags: u32,
    state: EmitState,
    pos: V3,
    dir: V3,
    nodes: Nodes,
    phase: u32,
    life: f32,
    timer: f32,
}

/// A fighter's bursts and its slot (`EftCharSlot_Get2`: the last one started).
#[derive(Clone, Debug, Default)]
pub(crate) struct RushBursts {
    list: Vec<Burst>,
    slot: Option<usize>,
}

impl RushBursts {
    /// Live bursts.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.list.len()
    }

    /// `EftRushBurst_Start` (request 0x28 begins): a burst at `node` along `dir`, the fighter's slot. Nothing when
    /// all four are in use.
    pub(crate) fn start(&mut self, set: &bt3_data::eft_pack::EmitSet, node: u32, pos: V3, dir: V3) {
        if self.list.len() >= BURSTS {
            return;
        }
        let mut nodes = Nodes::default();
        nodes.set_node(0, node);
        self.list.push(Burst {
            node,
            scale: 1.0,
            flags: ACTIVE | ALIVE,
            state: EmitState::new(set),
            pos,
            dir: norm(dir),
            nodes,
            phase: 1,
            life: f32::from(set.head.end_frames),
            timer: 0.0,
        });
        self.slot = Some(self.list.len() - 1);
    }

    /// `EftRushBurst_Stage2` (request 0x29 begins): the slot's burst adds its second stage.
    pub(crate) fn stage2(&mut self) {
        if let Some(b) = self.slot.and_then(|i| self.list.get_mut(i))
            && b.flags & ACTIVE != 0
        {
            b.flags |= STAGE2;
        }
    }

    /// `EftRushBurst_Stop` (request 0x2A begins): the slot's burst ends.
    pub(crate) fn stop(&mut self) {
        if let Some(b) = self.slot.and_then(|i| self.list.get_mut(i))
            && b.flags & ACTIVE != 0
        {
            b.flags |= END;
        }
    }

    /// `EftRushBurst_Update` of every burst (not while paused: `BtlScene_IsEffectStopped(.., 3)`): the second
    /// stage's phase once, the node followed, the set's emitters driven; an ending burst lasts the set's end
    /// frames. A burst that ends frees the fighter's slot (`EftCharSlot_Clear2`, whichever burst it held).
    pub(crate) fn update(
        &mut self,
        fx: &mut Modules,
        node_pos: &dyn Fn(u32) -> V3,
        ctx: &SpawnCtx,
        paused: bool,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        let mut i = 0;
        while i < self.list.len() {
            let b = &mut self.list[i];
            if !paused {
                if b.flags & STAGE2 != 0 && b.flags & STAGE2D == 0 {
                    b.phase |= 2;
                    b.nodes.set_node(1, b.node);
                    b.flags |= STAGE2D;
                }
                b.pos = node_pos(b.node);
                b.nodes.refresh(node_pos);
                emit(b, fx, ctx, libc, vu0);
                if b.flags & END != 0 {
                    b.timer += 1.0;
                    if b.timer >= b.life {
                        b.flags |= KILL;
                    }
                }
            }
            if b.flags & KILL != 0 {
                self.list.remove(i);
                self.slot = None;
            } else {
                i += 1;
            }
        }
    }

    /// `EftRushBurst_PostUpdate`: from the second frame on, whether anything of it is alive; the phases clear.
    pub(crate) fn post_update(&mut self, set: &bt3_data::eft_pack::EmitSet, fx: &mut Modules) {
        for b in &mut self.list {
            if b.flags & POSTED != 0 {
                if b.state.update_alive(set, fx) == 0 {
                    b.flags &= !ALIVE;
                } else {
                    b.flags |= ALIVE;
                }
                b.phase = 0;
            }
            b.flags |= POSTED;
        }
    }

    /// `EftRushBurst_Reset`: every burst kills its objects and ends.
    pub(crate) fn reset(&mut self, fx: &mut Modules) {
        for b in &mut self.list {
            b.state.kill_all(fx);
        }
        self.list.clear();
        self.slot = None;
    }
}

/// `EftRushBurst_Emit`: every emitter of the set gets this frame's flags from the burst's phases.
fn emit(
    b: &mut Burst,
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
            let f = b
                .state
                .flags_from_mask(set, kind, n, b.flags & END != 0, false, b.phase);
            if f != 0 {
                spawn(
                    fx,
                    &mut b.state,
                    &b.nodes,
                    ctx,
                    b.pos,
                    b.dir,
                    kind,
                    sub,
                    f,
                    b.scale,
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
    use bt3_data::eft_pack::EmitSet;

    #[test]
    fn a_burst_takes_its_second_stage_and_ends_on_request() {
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
        let node_pos = |n: u32| [n as f32, 0.0, 0.0];
        let mut r = RushBursts::default();
        let mut fx = Modules::default();
        let (mut libc, mut vu0) = (LibcRandom::new(1), Vu0Random::default());
        r.start(&set, 0x15, [0.0; 3], [0.0, 0.0, 2.0]);
        assert_eq!(r.list[0].dir, [0.0, 0.0, 1.0]);
        let mut frame = |r: &mut RushBursts, post: bool| {
            r.update(&mut fx, &node_pos, &ctx, false, &mut libc, &mut vu0);
            if post {
                r.post_update(&set, &mut fx);
            }
        };
        frame(&mut r, true);
        assert_eq!(r.list[0].pos, [21.0, 0.0, 0.0]);
        // The post-update clears the phases from the second frame on.
        frame(&mut r, true);
        assert_eq!(r.list[0].phase, 0);
        r.stage2();
        frame(&mut r, false);
        assert_eq!(r.list[0].phase, 2);
        assert!(r.list[0].flags & STAGE2D != 0);
        // Nothing ends it but the stop: then the set's two end frames.
        for _ in 0..20 {
            frame(&mut r, true);
        }
        assert_eq!(r.count(), 1);
        r.stop();
        frame(&mut r, true);
        assert_eq!(r.count(), 1);
        frame(&mut r, true);
        assert_eq!(r.count(), 0);
        assert_eq!(r.slot, None);
    }
}
