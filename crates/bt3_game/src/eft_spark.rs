//! The beam struggle's spark (`EftClashSpark_*`, src/battle/eft_struggle.c): the emitter set of common effect
//! entry 0x268 at the struggle point, along the first beam. The struggle starts it, moves it to its point every
//! frame (the point before that frame's move) and lets it end with the struggle; an ending spark lasts the set's
//! end frames. A layer 3 manager of three sparks, updated after the blast objects.

use bt3_data::eft_pack::EmitSet;

use crate::eft::{EmitState, Modules, Nodes, SpawnCtx, V3, norm, spawn};
use crate::rng::{LibcRandom, Vu0Random};

/// The common effect pack's entry of the spark's set.
pub(crate) const SET_ENTRY: usize = 0x268;
/// `EftClashSparkMgr_Init`: a child list of three.
const SPARKS: usize = 3;

// EFT_CSPARK_*.
const ACTIVE: u32 = 0x01;
const END: u32 = 0x02;
const KILL: u32 = 0x04;
const FAST: u32 = 0x08;
const POSTED: u32 = 0x20;

/// `EftClashSpark`.
#[derive(Clone, Debug)]
struct Spark {
    id: u32,
    flags: u32,
    pos: V3,
    dir: V3,
    scale: f32,
    state: EmitState,
    nodes: Nodes,
    /// `EftClashSpark_GetNodeMask(0)`: slot 0, cleared after the first post-update.
    mask: u32,
    life: f32,
    timer: f32,
}

/// The sparks alive.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sparks {
    list: Vec<Spark>,
    next_id: u32,
}

impl Sparks {
    /// Live sparks.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.list.len()
    }

    /// `EftClashSpark_Create` / `EftClashSpark_Init` (kind 0, owner 0, scale 1): a spark at `pos` along `dir`, its
    /// node slot 0 fixed there; `None` when the three are in use.
    pub(crate) fn create(&mut self, set: &EmitSet, pos: V3, dir: V3) -> Option<u32> {
        if self.list.len() >= SPARKS {
            return None;
        }
        self.next_id += 1;
        let mut nodes = Nodes::default();
        nodes.set_pos(0, pos);
        self.list.push(Spark {
            id: self.next_id,
            flags: ACTIVE,
            pos,
            dir: norm(dir),
            scale: 1.0,
            state: EmitState::new(set),
            nodes,
            mask: 1,
            life: f32::from(set.head.end_frames),
            timer: 0.0,
        });
        Some(self.next_id)
    }

    fn get(&mut self, id: Option<u32>) -> Option<&mut Spark> {
        let id = id?;
        self.list
            .iter_mut()
            .find(|s| s.id == id && s.flags & ACTIVE != 0)
    }

    /// `EftClashSpark_Stop`: the spark ends.
    pub(crate) fn stop(&mut self, id: Option<u32>) {
        if let Some(s) = self.get(id) {
            s.flags |= END;
        }
    }

    /// `EftClashSpark_SetPos` and `EftClashSpark_SetScale`.
    pub(crate) fn place(&mut self, id: Option<u32>, pos: V3, scale: f32) {
        if let Some(s) = self.get(id) {
            s.pos = pos;
            s.scale = scale;
        }
    }

    /// `EftClashSpark_Update` of every spark (not while paused: `BtlScene_IsEffectStopped(0, 2)`): the set's
    /// emitters driven at the spark; an ending one dies after the set's end frames.
    pub(crate) fn update(
        &mut self,
        fx: &mut Modules,
        ctx: &SpawnCtx,
        paused: bool,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        if paused {
            return;
        }
        let set = ctx.set;
        let mut i = 0;
        while i < self.list.len() {
            let s = &mut self.list[i];
            // EftEmit_RefreshFixedNodes: the slot holds a position, not a node.
            for kind in 0..0x13u8 {
                if set.head.mask & (1 << kind) == 0 {
                    continue;
                }
                let Some(g) = set.groups[usize::from(kind)] else {
                    continue;
                };
                for sub in 0..g.count {
                    let n = g.first + sub;
                    let f = s.state.flags_from_mask(
                        set,
                        kind,
                        n,
                        s.flags & END != 0,
                        s.flags & FAST != 0,
                        s.mask,
                    );
                    if f != 0 {
                        spawn(
                            fx,
                            &mut s.state,
                            &s.nodes,
                            ctx,
                            s.pos,
                            s.dir,
                            kind,
                            sub,
                            f,
                            s.scale,
                            libc,
                            vu0,
                        );
                    }
                }
            }
            if s.flags & END != 0 {
                s.timer += 1.0;
            }
            if s.flags & KILL != 0 {
                self.list.remove(i);
                continue;
            }
            if s.flags & END != 0 && (s.flags & FAST != 0 || s.timer >= s.life) {
                s.flags |= KILL;
            }
            i += 1;
        }
    }

    /// `EftClashSpark_PostUpdate` (not while paused): from the second frame on, which particles live, and the
    /// node mask cleared.
    pub(crate) fn post_update(&mut self, set: &EmitSet, fx: &mut Modules, paused: bool) {
        if paused {
            return;
        }
        for s in &mut self.list {
            if s.flags & POSTED != 0 {
                s.state.update_alive(set, fx);
                s.mask = 0;
            }
            s.flags |= POSTED;
        }
    }
}
