//! The ki blast's charge glow (`EftBlastCharge_*`, src/battle/eft_struggle.c): the fighter effect kind 3. Request
//! 0x17 of a ki blast that is not a disc, a thrown object nor a bomb (`BtlFx_SpawnHitSparkReq17`) runs the emitter
//! set of the character pack's entry 4 (the kind 2 ki blasts' set) at the firing hand while the fighter charges
//! (flag 0x8E), then one more step with the end flag. A character with a ki blast of type 3 has no kind 3 manager.
//! Ten per fighter. Stopped while any fighter is stopped (`BtlScene_IsEffectStopped(.., 0)`). Drawing only.

use crate::eft::{EmitState, Modules, Nodes, SpawnCtx, V3, spawn};
use crate::rng::{LibcRandom, Vu0Random};

/// `EftBlastChargeMgr_Init`: tasks per fighter.
const TASKS: usize = 10;

// EFT_BCHARGE_*.
const END: u32 = 0x2;
const KILL: u32 = 0x4;
const POSTED: u32 = 0x10;

/// `EftBlastCharge`.
#[derive(Clone, Debug)]
struct Glow {
    node: u32,
    flags: u32,
    scale: f32,
    pos: V3,
    dir: V3,
    state: EmitState,
    nodes: Nodes,
    mask: u32,
}

/// A fighter's glows (`EftChar_GetList(objId, 3)`).
#[derive(Clone, Debug, Default)]
pub(crate) struct BlastCharges {
    list: Vec<Glow>,
}

impl BlastCharges {
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.list.len()
    }

    /// `EftBlastCharge_Start` / `EftBlastCharge_Init`: at the firing node `node`, along `dir` (the aim), at the
    /// fighter's scale (`height` over 19.35, at least 0.5).
    pub(crate) fn start(
        &mut self,
        set: &bt3_data::eft_pack::EmitSet,
        node: u32,
        node_pos: V3,
        dir: V3,
        height: f32,
    ) {
        if self.list.len() >= TASKS {
            return;
        }
        let scale = height / 19.35;
        let mut g = Glow {
            node,
            flags: 1,
            scale: if scale < 0.5 { 0.5 } else { scale },
            pos: node_pos,
            dir,
            state: EmitState::new(set),
            nodes: Nodes::default(),
            mask: 1,
        };
        g.nodes.set_node(0, node);
        self.list.push(g);
    }

    /// `EftBlastCharge_Update` of every glow: unless stopped it follows its node and spawns, ending once the fighter
    /// no longer charges (`charging`: flag 0x8E); an ending glow spawns once more with the end flag, then dies.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        fx: &mut Modules,
        node_pos: &dyn Fn(u32) -> V3,
        charging: bool,
        ctx: &SpawnCtx,
        stopped: bool,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        let mut i = 0;
        while i < self.list.len() {
            let g = &mut self.list[i];
            if !stopped {
                g.pos = node_pos(g.node);
                g.nodes.refresh(node_pos);
                run_parts(g, fx, ctx, libc, vu0);
                if !charging {
                    g.flags |= END;
                }
            }
            if g.flags & KILL != 0 {
                self.list.remove(i);
                continue;
            }
            if g.flags & END != 0 {
                run_parts(g, fx, ctx, libc, vu0);
                g.flags |= KILL;
            }
            i += 1;
        }
    }

    /// `EftBlastCharge_PostUpdate`: from the second frame on, the objects alive, and the phases cleared.
    pub(crate) fn post_update(&mut self, set: &bt3_data::eft_pack::EmitSet, fx: &mut Modules) {
        for g in &mut self.list {
            if g.flags & POSTED != 0 {
                g.state.update_alive(set, fx);
                g.mask = 0;
            }
            g.flags |= POSTED;
        }
    }

    /// `EftBlastCharge_Reset`: the glows die (their objects live on).
    pub(crate) fn reset(&mut self) {
        self.list.clear();
    }
}

/// `EftBlastCharge_SpawnParts`.
fn run_parts(
    g: &mut Glow,
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
        let Some(grp) = set.groups[usize::from(kind)] else {
            continue;
        };
        for sub in 0..grp.count {
            let n = grp.first + sub;
            let f = g
                .state
                .flags_from_mask(set, kind, n, g.flags & END != 0, false, g.mask);
            if f != 0 {
                spawn(
                    fx,
                    &mut g.state,
                    &g.nodes,
                    ctx,
                    g.pos,
                    g.dir,
                    kind,
                    sub,
                    f,
                    g.scale,
                    libc,
                    vu0,
                );
            }
        }
    }
}
