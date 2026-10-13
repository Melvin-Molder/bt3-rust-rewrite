//! The transformation / fusion effect (`EftTransform_*`, src/battle/eft_transform.c): one of eight emitter sets of
//! the common effect pack, by the fighter's change kind, around the fighter while it changes. Request 0 starts it
//! (its "before" group), request 1 is the moment the model changes (the fighter turns into a black silhouette for
//! ten frames, then two screen shock waves and the "after" group), request 2 lets it run its end frames. One at a
//! time; layer 3.
//!
//! The flash turns the stage tint's slot 1 on (`StgTint_Start(1, ..)`, by the scene with the silhouette); the
//! silhouette itself is not drawn (the object's colour mode 3 is kept on the object).

use bt3_data::eft_pack::EmitSet;

use crate::eft::{EmitState, Modules, Nodes, SpawnCtx, V3, norm, spawn};
use crate::eft_impact::ImpactSet;
use crate::rng::{LibcRandom, Vu0Random};
use crate::scr_warp::ScrWarps;

/// `EftTransformMgr_Init`: the common entries of the eight sets.
pub(crate) const SET_ENTRIES: [usize; 8] = [0x192, 0x1AA, 0x1C0, 0x1C1, 0x1CB, 0x1E6, 0x1FC, 0x217];
/// The `SetRef::list` of set `k` (after the impacts').
pub(crate) const FIRST_LIST: usize =
    crate::eft_impact::FIRST_LIST + crate::eft_impact::SET_ENTRIES.len();
/// `gEftTransformList`: two tasks.
const TASKS: usize = 2;

// EFT_TF_*.
const END: u32 = 0x001;
const DETACHED: u32 = 0x002;
const DEAD: u32 = 0x004;
const ALIVE: u32 = 0x008;
const FLASH: u32 = 0x010;
const AFTER: u32 = 0x020;
const AFTER_ON: u32 = 0x040;
const ATTACHED: u32 = 0x080;
const TINT: u32 = 0x100;
const SPRITE: u32 = 0x200;
const KILLED: u32 = 0x400;
const JOINED: u32 = 0x800;

/// `EftTransform_GetSet`: kinds 1..4 share set 1.
fn set_of(kind: i32) -> Option<usize> {
    match kind {
        0 => Some(0),
        1..=4 => Some(1),
        5..=10 => Some(kind as usize - 3),
        _ => None,
    }
}

/// `EftTransform_HasStartGroup`: kinds 0, 1 and 7..10 start their first group at once, at node 3.
fn has_start_group(kind: i32) -> bool {
    if kind < 7 { kind < 2 } else { true }
}

/// What the effect reads of its fighter.
pub(crate) struct TransformChr<'a> {
    pub side: usize,
    pub height: f32,
    pub node_pos: &'a dyn Fn(u32) -> V3,
    /// The node's world matrix's third row (`BtlCharApi_GetNodeMtx`, kind 6's direction).
    pub node_z: &'a dyn Fn(u32) -> V3,
    /// `BtlCharApi_ObjTestAttr`: an animation event of this frame.
    pub attr: &'a dyn Fn(u64) -> bool,
    /// `BtlCharApi_ObjGetAttrKind`: the node of an event's value, 0 without.
    pub attr_node: &'a dyn Fn(u64) -> u32,
}

/// `EftTransform`.
#[derive(Clone, Debug)]
struct Transform {
    kind: i32,
    chr: usize,
    step: u8,
    scale: f32,
    pos: V3,
    dir: V3,
    set: usize,
    state: EmitState,
    nodes: Nodes,
    groups: u32,
    flags: u32,
    delay: f32,
    timer: f32,
    end_frames: f32,
    end_timer: f32,
}

/// What a frame of the effect asks of the fighters: their colour mode 3 (the silhouette) on or off.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ModelRequest {
    pub chr: usize,
    pub silhouette: bool,
}

/// `gEftTransform`: the tasks, the one requests 1 and 2 act on, and whether one runs.
#[derive(Clone, Debug, Default)]
pub(crate) struct Transforms {
    list: Vec<Transform>,
    task: Option<usize>,
    busy: bool,
    /// The fighters' colour mode changes of this update, in order.
    pub model_requests: Vec<ModelRequest>,
    /// `EftStage_SetDrawOn` of this update: off at the flash, on as an effect ends.
    pub stage_draw: Option<bool>,
}

impl Transforms {
    /// Live effects.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.list.len()
    }

    /// `EftTransform_Start` (request 0): the effect of `kind` unless one runs.
    pub(crate) fn start(&mut self, sets: &[ImpactSet], kind: i32, c: &TransformChr) {
        if self.busy || self.list.len() >= TASKS {
            return;
        }
        let Some(k) = set_of(kind) else {
            return;
        };
        let Some(s) = sets.get(k) else {
            return;
        };
        let mut t = Transform {
            kind,
            chr: c.side,
            step: 0,
            scale: (c.height / 19.35).max(0.5),
            pos: (c.node_pos)(3),
            dir: [0.0, -1.0, 0.0],
            set: k,
            state: EmitState::new(&s.set),
            nodes: Nodes::default(),
            groups: 0,
            flags: ALIVE,
            delay: 10.0,
            timer: 0.0,
            end_frames: f32::from(s.set.head.end_frames),
            end_timer: 0.0,
        };
        if has_start_group(kind) {
            t.groups |= 1;
            t.nodes.set_node(0, 3);
        }
        self.list.push(t);
        self.task = Some(self.list.len() - 1);
        self.busy = true;
    }

    /// `EftTransform_Flash` (request 1).
    pub(crate) fn flash(&mut self) {
        if !self.busy {
            return;
        }
        if let Some(t) = self.task.and_then(|i| self.list.get_mut(i)) {
            t.flags |= FLASH;
        }
    }

    /// `EftTransform_End` (request 2): the effect runs its end frames; another may start.
    pub(crate) fn end(&mut self) {
        if !self.busy {
            return;
        }
        if let Some(t) = self.task.and_then(|i| self.list.get_mut(i)) {
            t.flags |= END | DETACHED;
            self.task = None;
            self.busy = false;
        }
    }

    /// `EftTransform_Update` of every effect (not while paused: `BtlScene_IsEffectStopped(.., 5)`): the node
    /// bindings of its kind, the flash sequence, the "after" group, the emitters; an ended effect dies after its
    /// end frames. `ctx(chr, set)`: what `EftEmit_Spawn` reads of the owner.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update<'s>(
        &mut self,
        fx: &mut Modules,
        warps: &mut ScrWarps,
        sets: &'s [ImpactSet],
        chars: &[TransformChr; 2],
        paused: bool,
        ctx: impl Fn(usize, usize, &'s ImpactSet) -> SpawnCtx<'s>,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        self.model_requests.clear();
        self.stage_draw = None;
        let mut i = 0;
        while i < self.list.len() {
            let t = &mut self.list[i];
            let c = &chars[t.chr];
            if !paused {
                update_nodes(t, c);
                match t.step {
                    0 => t.step = 1,
                    1 => {
                        if t.flags & FLASH != 0 {
                            // EftTransform_OnFlash: kind 6 kills everything, kind 4 starts group 3 at node 3.
                            match t.kind {
                                6 => t.state.kill_all(fx),
                                4 => {
                                    t.nodes.set_node(3, 3);
                                    t.groups |= 8;
                                }
                                _ => {}
                            }
                            t.flags |= TINT;
                            self.stage_draw = Some(false);
                            self.model_requests.push(ModelRequest {
                                chr: t.chr,
                                silhouette: true,
                            });
                            t.step = 2;
                        }
                    }
                    2 => {
                        t.timer += 1.0;
                        if t.delay < t.timer {
                            if t.flags & TINT != 0 {
                                reveal(t, c, warps, vu0, &mut self.model_requests);
                            }
                            t.flags |= AFTER;
                            t.step = 3;
                        }
                    }
                    _ => {}
                }
                if t.flags & AFTER != 0 && t.flags & AFTER_ON == 0 {
                    t.groups |= 2;
                    t.nodes.set_node(1, 3);
                    t.flags |= AFTER_ON;
                }
                t.nodes.refresh(c.node_pos);
                let s = &sets[t.set];
                let sc = ctx(t.chr, t.set, s);
                spawn_parts(t, fx, &s.set, &sc, libc, vu0);
                if t.flags & END != 0 {
                    t.end_timer += 1.0;
                    if t.end_frames <= t.end_timer {
                        t.flags |= DEAD;
                    }
                }
            }
            if t.flags & DEAD != 0 {
                self.term(i, chars, warps, vu0);
            } else {
                i += 1;
            }
        }
    }

    /// `EftTransform_Term`: an effect that dies between flash and reveal undoes them.
    fn term(
        &mut self,
        i: usize,
        chars: &[TransformChr; 2],
        warps: &mut ScrWarps,
        vu0: &mut Vu0Random,
    ) {
        let mut t = self.list.remove(i);
        if t.flags & TINT != 0 {
            let c = &chars[t.chr];
            reveal(&mut t, c, warps, vu0, &mut self.model_requests);
        }
        self.stage_draw = Some(true);
        if t.flags & DETACHED == 0 {
            self.task = None;
            self.busy = false;
        } else if let Some(k) = self.task {
            if k > i {
                self.task = Some(k - 1);
            }
        }
    }

    /// `EftTransform_PostUpdate`: whether anything is alive; this frame's groups clear.
    pub(crate) fn post_update(&mut self, sets: &[ImpactSet], fx: &mut Modules) {
        for t in &mut self.list {
            if t.state.update_alive(&sets[t.set].set, fx) == 0 {
                t.flags &= !ALIVE;
            } else {
                t.flags |= ALIVE;
            }
            t.groups = 0;
        }
    }

    /// `EftTransform_Draw`: kind 5 draws the stage's sun sprite (16 wide) at node 0x36 while toggled on.
    pub(crate) fn sprites(&self) -> Vec<V3> {
        self.list
            .iter()
            .filter(|t| t.kind == 5 && t.flags & SPRITE != 0)
            .map(|t| t.pos)
            .collect()
    }

    /// `EftTransform_Reset`: every effect kills its objects and ends.
    pub(crate) fn reset(&mut self, fx: &mut Modules) {
        for t in &mut self.list {
            if t.flags & KILLED == 0 {
                t.flags |= KILLED;
                t.state.kill_all(fx);
            }
        }
        self.list.clear();
        self.task = None;
        self.busy = false;
    }
}

/// The reveal: the tint off, the fighter shown again, two shock waves at node 3.
fn reveal(
    t: &mut Transform,
    c: &TransformChr,
    warps: &mut ScrWarps,
    vu0: &mut Vu0Random,
    requests: &mut Vec<ModelRequest>,
) {
    requests.push(ModelRequest {
        chr: t.chr,
        silhouette: false,
    });
    let pos = (c.node_pos)(3);
    for _ in 0..2 {
        warps.spawn(t.chr, pos, 0.6, 10.0, 50.0, 10.0, 0.4, vu0);
    }
    t.flags &= !TINT;
}

/// `EftTransform_UpdateNodes`: where the emitters spawn and which node slots are bound, by kind. The fighter has
/// no partner object here (`BtlCharApi_GetPartnerObjId` gives the fighter).
fn update_nodes(t: &mut Transform, c: &TransformChr) {
    match t.kind {
        5 => {
            if (c.attr)(0x4000) {
                t.flags ^= SPRITE;
            }
            t.pos = (c.node_pos)(0x36);
        }
        6 => {
            if (c.attr)(0x200) {
                t.nodes.set_node(0, (c.attr_node)(0x200));
                t.groups |= 1;
            }
            if (c.attr)(0x400) {
                let node = (c.attr_node)(0x400);
                t.nodes.set_node(1, node);
                t.groups |= 2;
                t.flags |= ATTACHED;
                t.pos = (c.node_pos)(node);
                let z = (c.node_z)(node);
                t.dir = norm([-z[0], -z[1], -z[2]]);
            }
            if (c.attr)(0x1000) {
                t.nodes.set_node(3, 0x2E);
                t.groups |= 8;
            }
            if (c.attr)(0x2000) {
                t.nodes.set_node(4, (c.attr_node)(0x2000));
                t.groups |= 0x10;
            }
            if t.flags & ATTACHED != 0 {
                t.pos = (c.node_pos)(0x36);
            }
        }
        2 | 3 => {
            if t.flags & JOINED == 0 && (c.attr)(0x1000) {
                t.flags |= JOINED;
                t.pos = (c.node_pos)(0x36);
                let (slot, bit) = if t.kind == 2 { (3, 8) } else { (4, 0x10) };
                t.nodes.set_pos(slot, t.pos);
                t.groups |= bit;
            }
        }
        _ => t.pos = (c.node_pos)(3),
    }
}

/// `EftTransform_SpawnParts`: every emitter of the set gets this frame's flags from the groups.
fn spawn_parts(
    t: &mut Transform,
    fx: &mut Modules,
    set: &EmitSet,
    ctx: &SpawnCtx,
    libc: &mut LibcRandom,
    vu0: &mut Vu0Random,
) {
    for kind in 0..0x13u8 {
        if set.head.mask & (1 << kind) == 0 {
            continue;
        }
        let Some(g) = set.groups[usize::from(kind)] else {
            continue;
        };
        for sub in 0..g.count {
            let n = g.first + sub;
            let f = t
                .state
                .flags_from_mask(set, kind, n, t.flags & END != 0, false, t.groups);
            if f != 0 {
                spawn(
                    fx,
                    &mut t.state,
                    &t.nodes,
                    ctx,
                    t.pos,
                    t.dir,
                    kind,
                    sub,
                    f,
                    t.scale,
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

    fn sets() -> Vec<ImpactSet> {
        (0..8)
            .map(|_| ImpactSet {
                set: EmitSet::default(),
                assets: crate::eft::SetAssets::default(),
            })
            .collect()
    }

    fn ctx(chr: usize, k: usize, s: &ImpactSet) -> SpawnCtx<'_> {
        SpawnCtx {
            chr,
            set: &s.set,
            assets: &s.assets,
            set_ref: crate::eft::SetRef {
                side: chr,
                list: FIRST_LIST + k,
            },
            char_scale: [1.0; 2],
            opp_chara_flags: 0,
            opp_node3: [0.0; 3],
        }
    }

    #[test]
    fn the_flash_hides_the_fighter_for_ten_frames_then_two_waves() {
        let sets = sets();
        assert_eq!(
            (0..11).map(set_of).collect::<Vec<_>>(),
            [
                Some(0),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7)
            ]
        );
        let node_pos = |n: u32| [0.0, n as f32, 100.0];
        let node_z = |_| [0.0, 0.0, 1.0];
        let attr = |_| false;
        let attr_node = |_| 0;
        let c = TransformChr {
            side: 0,
            height: 5.0,
            node_pos: &node_pos,
            node_z: &node_z,
            attr: &attr,
            attr_node: &attr_node,
        };
        let chars = [
            TransformChr { ..c },
            TransformChr {
                side: 1,
                height: 5.0,
                node_pos: &node_pos,
                node_z: &node_z,
                attr: &attr,
                attr_node: &attr_node,
            },
        ];
        let mut t = Transforms::default();
        let mut fx = Modules::default();
        let mut warps = ScrWarps::default();
        let (mut libc, mut vu0) = (LibcRandom::new(1), Vu0Random::default());
        t.start(&sets, 0, &chars[0]);
        // One at a time.
        t.start(&sets, 1, &chars[1]);
        assert_eq!(t.count(), 1);
        assert_eq!(t.list[0].scale, 0.5);
        let mut frame = |t: &mut Transforms, warps: &mut ScrWarps| {
            t.update(
                &mut fx, warps, &sets, &chars, false, ctx, &mut libc, &mut vu0,
            );
            t.post_update(&sets, &mut fx);
            t.model_requests.clone()
        };
        frame(&mut t, &mut warps);
        t.flash();
        assert_eq!(
            frame(&mut t, &mut warps),
            [ModelRequest {
                chr: 0,
                silhouette: true
            }]
        );
        let mut shown = None;
        for i in 0..12 {
            if !frame(&mut t, &mut warps).is_empty() {
                shown = Some(i);
            }
        }
        // Eleven updates after the flash: the fighter shown, two waves, the "after" group.
        assert_eq!(shown, Some(10));
        assert_eq!(warps.count(), 2);
        assert!(t.list[0].flags & AFTER_ON != 0);
        // The end lets another start; this one runs its end frames (none here) and goes.
        t.end();
        t.start(&sets, 7, &chars[1]);
        assert_eq!(t.count(), 2);
        frame(&mut t, &mut warps);
        assert_eq!(t.count(), 1);
        assert_eq!(t.list[0].kind, 7);
        assert!(t.busy);
        t.reset(&mut fx);
        assert_eq!(t.count(), 0);
    }
}
