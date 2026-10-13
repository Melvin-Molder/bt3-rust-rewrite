//! The character body effect (`EftCharaFx_*`, src/battle/eft_trail.c): the fighter effect kind 7, the emitter set of
//! the character pack's entry 9 run on characters 0x37 (two emitters, 1.5 to either side of node 0x38), 0x98
//! (node 0x74) and 0x99 (node 0x3D, at twice the scale), along the way from their first node to the next.
//! `BtlFx_UpdateCharaFx` starts it while request 0x3A holds, or (without request 0x3B) while the fighter is on the
//! flying (flag 0xE) out of the water (0x11), unless its model is new (0x12B); it ends, with its objects, when that
//! is over. One per fighter. Stopped by the pause alone (`BtlScene_IsEffectStopped(.., 6)`). Drawing only.

use crate::eft::{EmitState, Modules, Nodes, SpawnCtx, V3, add, norm, scale3, spawn, sub};
use crate::rng::{LibcRandom, Vu0Random};

/// The `SetRef::list` of a fighter's body effect set, after the charge aura's.
pub(crate) const LIST: usize = 11;

// EFT_CHARAFX_*.
const ACTIVE: u32 = 0x1;
const END: u32 = 0x2;
const KILL: u32 = 0x4;
const ALIVE: u32 = 0x8;
const POSTED: u32 = 0x10;
const KILLED: u32 = 0x20;
const PAIR: u32 = 0x40;

/// What the effect reads of its fighter: its nodes' positions and matrices' X axes.
pub(crate) struct CharaFxChr<'a> {
    pub node_pos: &'a dyn Fn(u32) -> V3,
    pub node_x: &'a dyn Fn(u32) -> V3,
}

/// `EftCharaFx`.
#[derive(Clone, Debug)]
struct Fx {
    flags: u32,
    scale: f32,
    node: u32,
    node_a: u32,
    node_b: u32,
    offset: f32,
    pos: [V3; 2],
    dir: V3,
    state: [EmitState; 2],
    nodes: Nodes,
    phase: u32,
}

impl Fx {
    /// Both positions at the node, the direction from node A to node B.
    fn follow(&mut self, c: &CharaFxChr) {
        let p = (c.node_pos)(self.node);
        self.pos = [p, p];
        self.dir = norm(sub((c.node_pos)(self.node_b), (c.node_pos)(self.node_a)));
    }
}

/// A fighter's body effect (`EftChar_GetList(objId, 7)`, `EftCharSlot` 3) and `chr->charaFxOn`.
#[derive(Clone, Debug, Default)]
pub(crate) struct CharaFxs {
    fx: Option<Fx>,
    pub on: bool,
}

impl CharaFxs {
    #[cfg(test)]
    pub(crate) fn is_running(&self) -> bool {
        self.fx.is_some()
    }

    /// `EftCharaFx_Start` / `EftCharaFx_Init` for character `chara`, unless the fighter's runs.
    pub(crate) fn start(&mut self, set: &bt3_data::eft_pack::EmitSet, chara: i32, c: &CharaFxChr) {
        if self.fx.is_some() {
            return;
        }
        let mut f = Fx {
            flags: ACTIVE,
            scale: 1.0,
            node: 0,
            node_a: 0,
            node_b: 0,
            offset: 0.0,
            pos: [[0.0; 3]; 2],
            dir: [0.0; 3],
            state: [EmitState::new(set), EmitState::new(set)],
            nodes: Nodes::default(),
            phase: 1,
        };
        match chara {
            0x37 => {
                (f.node, f.node_a, f.node_b, f.offset) = (0x38, 0x37, 0x38, 1.5);
                f.flags |= PAIR;
            }
            0x98 => (f.node, f.node_a, f.node_b) = (0x74, 0x74, 0x75),
            0x99 => {
                (f.node, f.node_a, f.node_b) = (0x3D, 0x3D, 0x3E);
                f.scale += f.scale;
            }
            _ => {}
        }
        f.follow(c);
        f.nodes.set_node(0, f.node);
        f.flags |= ALIVE;
        self.fx = Some(f);
    }

    /// `EftCharaFx_Stop`.
    pub(crate) fn stop(&mut self) {
        if let Some(f) = &mut self.fx
            && f.flags & ACTIVE != 0
        {
            f.flags |= END;
        }
    }

    /// `EftCharaFx_Update`: unless paused it follows the fighter (the pair to either side of the node along its
    /// matrix's X axis) and spawns; asked to end, its objects die and it goes in the same update.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        fx: &mut Modules,
        c: &CharaFxChr,
        ctx: &SpawnCtx,
        paused: bool,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        let Some(f) = &mut self.fx else {
            return;
        };
        if !paused {
            f.follow(c);
            f.nodes.refresh(c.node_pos);
            if f.flags & PAIR != 0 {
                let side = scale3(norm((c.node_x)(f.node)), f.offset);
                f.pos[0] = add(f.pos[0], side);
                f.pos[1] = sub(f.pos[1], side);
            }
            for k in 0..2 {
                if f.flags & PAIR != 0 || k == 0 {
                    run_parts(f, k, fx, ctx, libc, vu0);
                }
            }
            if f.flags & END != 0 {
                f.flags |= KILL;
                if f.flags & KILLED == 0 {
                    f.flags |= KILLED;
                    for s in &mut f.state {
                        s.kill_all(fx);
                    }
                }
            }
        }
        if f.flags & KILL != 0 {
            self.fx = None;
        }
    }

    /// `EftCharaFx_PostUpdate`: from the second frame on, the objects alive, and the phases cleared.
    pub(crate) fn post_update(&mut self, set: &bt3_data::eft_pack::EmitSet, modules: &mut Modules) {
        if let Some(f) = &mut self.fx {
            if f.flags & POSTED != 0 {
                if f.state[0].update_alive(set, modules) == 0 {
                    f.flags &= !ALIVE;
                } else {
                    f.flags |= ALIVE;
                }
                f.state[1].update_alive(set, modules);
                f.phase = 0;
            }
            f.flags |= POSTED;
        }
    }

    /// `EftCharaFx_Reset`: its objects die once, and it goes.
    pub(crate) fn reset(&mut self, modules: &mut Modules) {
        if let Some(f) = &mut self.fx
            && f.flags & KILLED == 0
        {
            for s in &mut f.state {
                s.kill_all(modules);
            }
        }
        self.fx = None;
    }
}

/// `EftCharaFx_Emit` of state `k` at position `k`.
fn run_parts(
    f: &mut Fx,
    k: usize,
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
            let flags =
                f.state[k].flags_from_mask(set, kind, n, f.flags & END != 0, false, f.phase);
            if flags != 0 {
                spawn(
                    fx,
                    &mut f.state[k],
                    &f.nodes,
                    ctx,
                    f.pos[k],
                    f.dir,
                    kind,
                    sub,
                    flags,
                    f.scale,
                    libc,
                    vu0,
                );
            }
        }
    }
}
