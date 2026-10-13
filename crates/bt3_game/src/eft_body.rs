//! The body effect (`EftBodyFx_*`, the first module of src/battle/eft_body_fx.c): fighter request 0x1A
//! (`BtlFx_UpdateReq1A`) runs the common effect file's emitter set 0x23F on the fighter's body, at node 3 along the
//! way from node 3 to node 0x2E, at the fighter's scale (at least 0.5); once the request ends it lasts the set's end
//! frames. One per fighter at a time, in one list of three in the order they started (`EftBodyFxMgr_Init`). Stopped
//! by the pause and by its fighter's rush sequence (`BtlScene_IsEffectStopped(.., 4)`). Drawing only.

use crate::eft::{EmitState, Modules, Nodes, SpawnCtx, V3, norm, spawn, sub};
use crate::rng::{LibcRandom, Vu0Random};

/// `EftBodyFxMgr_Init`: the set is common effect entry 0x23F.
pub(crate) const SET_ENTRY: usize = 0x23F;
/// `EftBodyFxMgr_Init`: tasks in the list.
const TASKS: usize = 3;

// EFT_BODYFX_*.
const ACTIVE: u32 = 0x1;
const END: u32 = 0x2;
const KILL: u32 = 0x4;
const ALIVE: u32 = 0x8;
const POSTED: u32 = 0x10;
const RESET: u32 = 0x20;

/// What the effect reads of its fighter.
pub(crate) struct BodyChr<'a> {
    pub node_pos: &'a dyn Fn(u32) -> V3,
    /// `BtlScene_GetCharScale`.
    pub scale: f32,
}

/// `EftBodyFx`.
#[derive(Clone, Debug)]
struct Body {
    side: usize,
    flags: u32,
    scale: f32,
    pos: V3,
    dir: V3,
    state: EmitState,
    nodes: Nodes,
    phase: u32,
    end_time: f32,
    end_life: f32,
}

impl Body {
    /// Node 3 and the way from it to node 0x2E.
    fn follow(&mut self, c: &BodyChr) {
        self.pos = (c.node_pos)(3);
        self.dir = norm(sub((c.node_pos)(0x2E), self.pos));
    }
}

/// The body effects (`gEftBodyFxList`).
#[derive(Clone, Debug, Default)]
pub(crate) struct BodyFxs {
    list: Vec<Body>,
}

impl BodyFxs {
    /// Whether a fighter's effect runs (`gEftBodyFx->task[objId]`).
    pub(crate) fn has(&self, side: usize) -> bool {
        self.list.iter().any(|b| b.side == side)
    }

    /// `EftBodyFx_Start` / `EftBodyFx_Init`: unless the fighter's runs (an ending one included).
    pub(crate) fn start(&mut self, side: usize, set: &bt3_data::eft_pack::EmitSet, c: &BodyChr) {
        if self.has(side) || self.list.len() >= TASKS {
            return;
        }
        let mut b = Body {
            side,
            flags: ACTIVE,
            scale: if c.scale < 0.5 { 0.5 } else { c.scale },
            pos: [0.0; 3],
            dir: [0.0; 3],
            state: EmitState::new(set),
            nodes: Nodes::default(),
            phase: 1,
            end_time: 0.0,
            end_life: f32::from(set.head.end_frames),
        };
        b.follow(c);
        b.nodes.set_node(0, 3);
        b.flags |= ALIVE;
        self.list.push(b);
    }

    /// `EftBodyFx_Stop`: the fighter's effect ends.
    pub(crate) fn stop(&mut self, side: usize) {
        if let Some(b) = self.list.iter_mut().find(|b| b.side == side)
            && b.flags & ACTIVE != 0
        {
            b.flags |= END;
        }
    }

    /// `EftBodyFx_Update` of every effect: unless stopped (`stopped(side)`) it follows its fighter and drives the
    /// set's emitters with this frame's phases; an ending one dies after the set's end frames.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update<'a>(
        &mut self,
        fx: &mut Modules,
        chr: impl Fn(usize) -> BodyChr<'a>,
        ctx: impl Fn(usize) -> SpawnCtx<'a>,
        stopped: impl Fn(usize) -> bool,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        let mut i = 0;
        while i < self.list.len() {
            let b = &mut self.list[i];
            if !stopped(b.side) {
                let c = chr(b.side);
                b.follow(&c);
                b.nodes.refresh(c.node_pos);
                let ctx = ctx(b.side);
                run_parts(b, fx, &ctx, libc, vu0);
                if b.flags & END != 0 {
                    b.end_time += 1.0;
                    if b.end_life <= b.end_time {
                        b.flags |= KILL;
                    }
                }
            }
            if b.flags & KILL != 0 {
                self.list.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// `EftBodyFx_PostUpdate`: from the second frame on, whether anything of it is alive, and the phases cleared.
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

    /// `EftBodyFx_Reset` of a fighter's effect (`BtlScene_Reset`): its objects die with it.
    pub(crate) fn reset_side(&mut self, fx: &mut Modules, side: usize) {
        for b in self.list.iter_mut().filter(|b| b.side == side) {
            if b.flags & RESET == 0 {
                b.flags |= RESET;
                b.state.kill_all(fx);
            }
        }
        self.list.retain(|b| b.side != side);
    }
}

/// `EftBodyFx_UpdateParts`: every emitter of the set gets this frame's flags from the effect's phases.
fn run_parts(
    b: &mut Body,
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
    fn a_body_effect_lasts_its_end_frames_after_the_stop() {
        let mut set = EmitSet::default();
        set.head.end_frames = 3;
        let assets = crate::eft::SetAssets::default();
        let node = |n: u32| [0.0, -(n as f32), 0.0];
        let chr = |_| BodyChr {
            node_pos: &node,
            scale: 0.2,
        };
        let ctx = |side| SpawnCtx {
            chr: side,
            set: &set,
            assets: &assets,
            set_ref: crate::eft::SetRef {
                side,
                list: crate::eft::BODY_LIST,
            },
            char_scale: [1.0; 2],
            opp_chara_flags: 0,
            opp_node3: [0.0; 3],
        };
        let mut b = BodyFxs::default();
        let mut fx = Modules::default();
        let (mut libc, mut vu0) = (LibcRandom::new(1), Vu0Random::default());
        b.start(1, &set, &chr(1));
        // One per fighter; the scale at least 0.5; along node 3 to node 0x2E (up: +y is down).
        b.start(1, &set, &chr(1));
        assert_eq!(b.list.len(), 1);
        assert_eq!(b.list[0].scale, 0.5);
        assert_eq!(b.list[0].dir, [0.0, -1.0, 0.0]);
        let mut frame = |b: &mut BodyFxs, stopped: bool| {
            b.update(&mut fx, chr, ctx, |_| stopped, &mut libc, &mut vu0);
            b.post_update(&set, &mut fx);
        };
        for _ in 0..5 {
            frame(&mut b, false);
        }
        assert!(b.has(1));
        b.stop(1);
        // A stopped effect does not count its end frames.
        frame(&mut b, true);
        frame(&mut b, false);
        frame(&mut b, false);
        assert!(b.has(1));
        frame(&mut b, false);
        assert!(!b.has(1));
    }
}
