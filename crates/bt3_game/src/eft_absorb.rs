//! The drain glow (`EftAbsorb_*`, src/battle/eft_absorb.c): the emitter set of the character pack's entry 0xB,
//! run at the fighter's node 0x36 while the drain technique holds request 0x38, or at both hands once a blast was
//! absorbed (request 0x37). A kind 9 manager of the fighter's effect layer, up to six glows per fighter.

use crate::eft::{EmitState, Modules, Nodes, SpawnCtx, V3, norm, spawn, sub};
use crate::rng::{LibcRandom, Vu0Random};

/// `EftAbsorbMgr_Init`: six glows per fighter.
const GLOWS: usize = 6;
/// The `SetRef::list` of a fighter's absorb set.
pub(crate) const LIST: usize = 7;

// EFT_ABSORB_*.
const ACTIVE: u32 = 0x1;
const END: u32 = 0x2;
const KILL: u32 = 0x4;
const NEW: u32 = 0x8;
const POSTED: u32 = 0x10;

/// What the glow reads of its fighter.
pub(crate) struct AbsorbChr<'a> {
    pub node_pos: &'a dyn Fn(u32) -> V3,
    /// `BtlCharApi_ObjTestAttr(objId, 0x400)`.
    pub event_400: bool,
}

/// `EftAbsorb`.
#[derive(Clone, Debug)]
struct Glow {
    hands: bool,
    scale: f32,
    flags: u32,
    state: [EmitState; 2],
    pos: [V3; 2],
    dir: [V3; 2],
    nodes: Nodes,
    phase_mask: u32,
    life: f32,
    timer: f32,
}

/// A fighter's glows (`EftChar_GetList(objId, 9)`) and its node-0x36 glow (`EftCharSlot_GetAbsorb`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Absorbs {
    glows: Vec<Glow>,
    /// The glow `EftAbsorb_Stop` ends, by its index.
    slot: Option<usize>,
}

impl Absorbs {
    /// Live glows (both kinds).
    #[cfg(test)]
    pub(crate) fn count(&self) -> (usize, usize) {
        let hands = self.glows.iter().filter(|g| g.hands).count();
        (self.glows.len() - hands, hands)
    }

    /// `EftAbsorb_Init`.
    fn add(&mut self, set: &bt3_data::eft_pack::EmitSet, c: &AbsorbChr, hands: bool) -> bool {
        if self.glows.len() >= GLOWS {
            return false;
        }
        let mut g = Glow {
            hands,
            scale: 1.0,
            flags: ACTIVE | NEW,
            state: [EmitState::new(set), EmitState::new(set)],
            pos: [[0.0; 3]; 2],
            dir: [[0.0; 3]; 2],
            nodes: Nodes::default(),
            phase_mask: 0,
            life: f32::from(set.head.end_frames),
            timer: 0.0,
        };
        if !hands {
            g.pos[0] = (c.node_pos)(0x36);
            g.dir[0] = [0.0, -1.0, 0.0];
            g.phase_mask |= 1;
            g.nodes.set_node(0, 0x36);
        } else {
            g.pos = [(c.node_pos)(0x15), (c.node_pos)(0x23)];
            g.phase_mask |= 2;
            g.nodes.set_node(1, 0x36);
            // The original takes both directions from the first hand here.
            g.dir = [
                norm(sub(g.pos[0], (c.node_pos)(0x14))),
                norm(sub(g.pos[0], (c.node_pos)(0x22))),
            ];
        }
        self.glows.push(g);
        true
    }

    /// `EftAbsorb_Start` (request 0x38 begins): the node-0x36 glow, unless the fighter has one.
    pub(crate) fn start(&mut self, set: &bt3_data::eft_pack::EmitSet, c: &AbsorbChr) {
        if self.slot.is_some() {
            return;
        }
        if self.add(set, c, false) {
            self.slot = Some(self.glows.len() - 1);
        }
    }

    /// `EftAbsorb_Stop` (request 0x38 ends): the node-0x36 glow ends.
    pub(crate) fn stop(&mut self) {
        if let Some(g) = self.slot.and_then(|i| self.glows.get_mut(i))
            && g.flags & ACTIVE != 0
        {
            g.flags |= END;
        }
    }

    /// `EftAbsorb_StartHands` (request 0x37): a glow at each hand.
    pub(crate) fn start_hands(&mut self, set: &bt3_data::eft_pack::EmitSet, c: &AbsorbChr) {
        self.add(set, c, true);
    }

    /// `EftAbsorb_Update` of every glow (not while paused: `BtlScene_IsEffectStopped(.., 3)`): it follows the
    /// fighter's nodes (the node-0x36 glow's phase 0 again on event 0x400) and drives its set's emitters; an
    /// ending glow lasts the set's end frames.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        fx: &mut Modules,
        c: &AbsorbChr,
        ctx: &SpawnCtx,
        paused: bool,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        let mut i = 0;
        while i < self.glows.len() {
            let g = &mut self.glows[i];
            if !paused {
                if !g.hands {
                    g.pos[0] = (c.node_pos)(0x36);
                    if c.event_400 {
                        g.phase_mask |= 1;
                        g.nodes.set_node(0, 0x36);
                    }
                } else {
                    g.pos = [(c.node_pos)(0x15), (c.node_pos)(0x23)];
                    g.phase_mask |= 2;
                    g.nodes.set_node(1, 0x36);
                    g.dir = [
                        norm(sub(g.pos[0], (c.node_pos)(0x14))),
                        norm(sub(g.pos[1], (c.node_pos)(0x22))),
                    ];
                }
                g.nodes.refresh(c.node_pos);
                for k in 0..if g.hands { 2 } else { 1 } {
                    run_set(g, k, fx, ctx, libc, vu0);
                }
                if g.flags & END != 0 {
                    g.timer += 1.0;
                    if g.timer >= g.life {
                        g.flags |= KILL;
                    }
                }
            }
            if g.flags & KILL != 0 {
                self.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// `EftAbsorb_Term`: the node-0x36 glow frees the fighter's slot.
    fn remove(&mut self, i: usize) {
        self.glows.remove(i);
        self.slot = match self.slot {
            Some(s) if s == i => None,
            Some(s) if s > i => Some(s - 1),
            s => s,
        };
    }

    /// `EftAbsorb_PostUpdate`: from the second frame on, once nothing of the first set is alive the hands' glow
    /// ends by itself (the other waits for `stop`).
    pub(crate) fn post_update(&mut self, set: &bt3_data::eft_pack::EmitSet, fx: &mut Modules) {
        for g in &mut self.glows {
            if g.flags & POSTED != 0 {
                if g.state[0].update_alive(set, fx) == 0 {
                    if g.hands {
                        g.flags |= END;
                    } else {
                        g.flags &= !NEW;
                    }
                } else {
                    g.flags |= NEW;
                }
                g.phase_mask = 0;
            }
            g.flags |= POSTED;
        }
    }

    /// `EftAbsorb_Reset`: every glow kills its objects and ends.
    pub(crate) fn reset(&mut self, fx: &mut Modules) {
        for g in &mut self.glows {
            for s in &mut g.state {
                s.kill_all(fx);
            }
        }
        self.glows.clear();
        self.slot = None;
    }
}

/// `EftAbsorb_DrawOne`: every emitter of the set gets this frame's flags from the glow's phases.
fn run_set(
    g: &mut Glow,
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
            let f =
                g.state[k].flags_from_mask(set, kind, n, g.flags & END != 0, false, g.phase_mask);
            if f != 0 {
                spawn(
                    fx,
                    &mut g.state[k],
                    &g.nodes,
                    ctx,
                    g.pos[k],
                    g.dir[k],
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

#[cfg(test)]
mod tests {
    use super::*;
    use bt3_data::eft_pack::EmitSet;

    fn nodes(n: u32) -> V3 {
        [n as f32, 0.0, 0.0]
    }

    #[test]
    fn the_drain_glow_waits_for_its_stop_and_the_hands_end_by_themselves() {
        let mut set = EmitSet::default();
        set.head.end_frames = 3;
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
        let f = nodes;
        let c = AbsorbChr {
            node_pos: &f,
            event_400: false,
        };
        let mut a = Absorbs::default();
        let mut fx = Modules::default();
        let (mut libc, mut vu0) = (LibcRandom::new(1), Vu0Random::default());
        a.start(&set, &c);
        // One node-0x36 glow per fighter.
        a.start(&set, &c);
        a.start_hands(&set, &c);
        assert_eq!(a.count(), (1, 1));
        assert_eq!(a.glows[0].pos[0], [54.0, 0.0, 0.0]);
        assert_eq!(a.glows[1].dir, [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]]);
        let mut frame = |a: &mut Absorbs| {
            a.update(&mut fx, &c, &ctx, false, &mut libc, &mut vu0);
            a.post_update(&set, &mut fx);
        };
        for _ in 0..10 {
            frame(&mut a);
        }
        // The hands' glow ended (nothing alive, then the end frames); the other holds.
        assert_eq!(a.count(), (1, 0));
        assert_eq!(a.slot, Some(0));
        a.stop();
        for _ in 0..3 {
            frame(&mut a);
        }
        assert_eq!(a.count(), (0, 0));
        assert_eq!(a.slot, None);
        a.start(&set, &c);
        assert_eq!(a.count(), (1, 0));
    }
}
