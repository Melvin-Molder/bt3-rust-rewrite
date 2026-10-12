//! The impacts (`EftImpact_*`, src/battle/eft_impact.c): tasks that run one emitter set of the common effect
//! pack at a point, through the particle modules. Group 0 is the hit sparks the fighters' effect requests start,
//! sized by the fighter (or given a size); group 1 the explosion a projectile leaves where it hits, rising as it
//! plays.

use bt3_data::eft_pack::EmitSet;

use crate::eft::{EmitState, Modules, Nodes, SetAssets, SpawnCtx, V3, norm, spawn};
use crate::rng::{LibcRandom, Vu0Random};

/// `EftImpactMgr_Init`: the common entries of the six hit spark sets, then the eight blast impact sets.
pub(crate) const SET_ENTRIES: [usize; 14] = [
    0xC, 0x20, 0x25, 0x4C, 0x66, 0x73, 0x88, 0x91, 0xA4, 0xB9, 0xF2, 0x11A, 0x137, 0x158,
];
/// The `SetRef::list` of set `k` of `SET_ENTRIES` (after the sides' own lists).
pub(crate) const FIRST_LIST: usize = 7;
/// `gEftImpactList`: 30 tasks.
const TASKS: usize = 30;

// EFT_IMPACT_*.
const ENDING: u32 = 0x01;
const DEAD: u32 = 0x02;
const KILL: u32 = 0x04;
const KILLED: u32 = 0x08;
const STARTED: u32 = 0x10;

/// One of the fourteen sets with its decoded assets.
#[derive(Clone, Debug)]
pub(crate) struct ImpactSet {
    pub set: EmitSet,
    pub assets: SetAssets,
}

/// `EftImpactArg`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImpactArg {
    pub pos: V3,
    pub dir: V3,
    pub id: i32,
    pub chr: usize,
}

/// `EftImpact_GetSet`: the set (index into `SET_ENTRIES`) of an impact id.
fn set_of(group: u8, id: i32) -> usize {
    if group == 0 {
        match id {
            4..=6 => 1,
            7..=10 => 2,
            11..=13 => 3,
            14 => 4,
            15 | 16 => 5,
            _ => 0,
        }
    } else {
        6 + match id {
            0 => 0,
            1 => 1,
            2 => 2,
            3 | 4 => 3,
            5 | 6 => 4,
            7 | 8 => 5,
            9 | 10 => 6,
            11 | 12 => 7,
            _ => 0,
        }
    }
}

/// `EftImpact_GetNodeMask`: the node slots an impact id places at its position.
fn node_mask(group: u8, id: i32) -> u8 {
    const HIT: [u8; 17] = [1, 1, 8, 0x10, 1, 2, 8, 1, 2, 8, 0x10, 1, 2, 8, 1, 1, 2];
    const BLAST: [u8; 13] = [1, 1, 1, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2];
    let table: &[u8] = if group == 0 { &HIT } else { &BLAST };
    usize::try_from(id)
        .ok()
        .and_then(|i| table.get(i))
        .copied()
        .unwrap_or(0)
}

/// `EftImpactWork`.
#[derive(Clone, Debug)]
struct Impact {
    group: u8,
    id: i32,
    chr: usize,
    pos: V3,
    dir: V3,
    scale: f32,
    rise: f32,
    set: usize,
    state: EmitState,
    end_frames: f32,
    mask: u8,
    nodes: Nodes,
    flags: u32,
    time: f32,
}

/// The task list.
#[derive(Clone, Debug, Default)]
pub(crate) struct Impacts {
    tasks: Vec<Impact>,
}

impl Impacts {
    /// Live impacts, and the ids of group `group`'s.
    #[cfg(test)]
    pub(crate) fn ids(&self, group: u8) -> Vec<i32> {
        self.tasks
            .iter()
            .filter(|t| t.group == group)
            .map(|t| t.id)
            .collect()
    }

    /// `EftImpact_Init`: a hit spark without a size takes its fighter's scale (at least 0.7) * 0.75, * 1.2 for
    /// id 1. Nothing when the list is full or the set did not load.
    fn add(
        &mut self,
        sets: &[ImpactSet],
        group: u8,
        arg: &ImpactArg,
        scale: f32,
        rise: f32,
        char_scale: f32,
    ) -> bool {
        if self.tasks.len() >= TASKS {
            return false;
        }
        let k = set_of(group, arg.id);
        let Some(s) = sets.get(k) else {
            return false;
        };
        let scale = if group == 0 && scale <= 0.0 {
            let mut s = char_scale.max(0.7) * 0.75;
            if arg.id == 1 {
                s *= 1.2;
            }
            s
        } else {
            scale
        };
        let mask = node_mask(group, arg.id);
        let mut nodes = Nodes::default();
        for i in 0..6 {
            if (mask >> i) & 1 != 0 {
                nodes.set_pos(i, arg.pos);
            }
        }
        self.tasks.push(Impact {
            group,
            id: arg.id,
            chr: arg.chr,
            pos: arg.pos,
            dir: norm(arg.dir),
            scale,
            rise: if group == 0 { 0.0 } else { rise },
            set: k,
            state: EmitState::new(&s.set),
            end_frames: f32::from(s.set.head.end_frames),
            mask,
            nodes,
            flags: 0,
            time: 0.0,
        });
        true
    }

    /// `EftImpact_SpawnHit`: a hit spark sized by its fighter (`char_scale`: `BtlScene_GetCharScale`).
    pub(crate) fn spawn_hit(
        &mut self,
        sets: &[ImpactSet],
        arg: &ImpactArg,
        char_scale: f32,
    ) -> bool {
        self.add(sets, 0, arg, 0.0, 0.0, char_scale)
    }

    /// `EftImpact_SpawnHitScaled`.
    pub(crate) fn spawn_hit_scaled(
        &mut self,
        sets: &[ImpactSet],
        arg: &ImpactArg,
        scale: f32,
    ) -> bool {
        self.add(sets, 0, arg, scale, 0.0, 0.0)
    }

    /// `EftImpact_SpawnBlast`: what a projectile leaves where it hits, rising `rise` a frame.
    pub(crate) fn spawn_blast(
        &mut self,
        sets: &[ImpactSet],
        arg: &ImpactArg,
        scale: f32,
        rise: f32,
    ) -> bool {
        self.add(sets, 1, arg, scale, rise, 0.0)
    }

    /// `EftImpact_Update` of every task (none while paused: `BtlScene_IsEffectStopped(.., 2)`): a blast impact
    /// rises, the node slots follow, every emitter of the set gets this frame's flags; an ending impact counts its
    /// set's end frames, then dies. `ctx(chr, set)`: what `EftEmit_Spawn` reads of the owner.
    pub(crate) fn update<'s>(
        &mut self,
        fx: &mut Modules,
        sets: &'s [ImpactSet],
        paused: bool,
        ctx: impl Fn(usize, usize, &'s ImpactSet) -> SpawnCtx<'s>,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        if paused {
            return;
        }
        let mut i = 0;
        while i < self.tasks.len() {
            let t = &mut self.tasks[i];
            if t.group != 0 {
                t.pos[1] -= t.rise;
            }
            for i in 0..6 {
                if (t.mask >> i) & 1 != 0 {
                    t.nodes.set_slot_pos(i, t.pos);
                }
            }
            let s = &sets[t.set];
            let c = ctx(t.chr, t.set, s);
            let set = &s.set;
            for kind in 0..19u8 {
                if set.head.mask & (1 << kind) == 0 {
                    continue;
                }
                let Some(g) = set.groups[usize::from(kind)] else {
                    continue;
                };
                for sub in 0..g.count {
                    let n = g.first + sub;
                    let mut f = t.state.flags_from_mask(
                        set,
                        kind,
                        n,
                        t.flags & ENDING != 0,
                        t.flags & KILL != 0,
                        u32::from(t.mask),
                    );
                    if t.flags & KILLED != 0 {
                        f = 2;
                    }
                    if f != 0 {
                        spawn(
                            fx,
                            &mut t.state,
                            &t.nodes,
                            &c,
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
            if t.flags & ENDING != 0 {
                t.time += 1.0;
            }
            if t.flags & DEAD != 0 {
                self.tasks.remove(i);
                continue;
            }
            if t.flags & ENDING != 0 && (t.flags & KILL != 0 || t.time >= t.end_frames) {
                t.flags |= DEAD;
            }
            i += 1;
        }
    }

    /// `EftImpact_PostUpdate` (not while time is stopped): from the second frame on, an impact none of whose
    /// objects is alive is ending, and its node slots are no longer placed.
    pub(crate) fn post_update(&mut self, fx: &mut Modules, sets: &[ImpactSet], paused: bool) {
        if paused {
            return;
        }
        for t in &mut self.tasks {
            if t.flags & STARTED != 0 {
                if t.state.update_alive(&sets[t.set].set, fx) == 0 {
                    t.flags |= ENDING;
                }
                t.mask = 0;
            }
            t.flags |= STARTED;
        }
    }

    /// `EftImpact_Reset`: every impact kills its objects and ends.
    pub(crate) fn reset(&mut self, fx: &mut Modules) {
        for t in &mut self.tasks {
            if t.flags & KILLED == 0 {
                t.flags |= KILLED;
                t.state.kill_all(fx);
            }
        }
        self.tasks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_pick_their_sets_and_node_slots() {
        assert_eq!(
            (0..17).map(|i| set_of(0, i)).collect::<Vec<_>>(),
            [0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 5, 5]
        );
        assert_eq!(
            (0..14).map(|i| set_of(1, i)).collect::<Vec<_>>(),
            [6, 7, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 6]
        );
        assert_eq!(node_mask(0, 3), 0x10);
        assert_eq!(node_mask(1, 4), 2);
        assert_eq!(node_mask(0, 20), 0);
    }

    fn sets() -> Vec<ImpactSet> {
        (0..14)
            .map(|_| ImpactSet {
                set: EmitSet::default(),
                assets: SetAssets::default(),
            })
            .collect()
    }

    #[test]
    fn a_hit_spark_takes_its_fighters_scale_and_the_list_holds_thirty() {
        let sets = sets();
        let mut im = Impacts::default();
        let arg = ImpactArg {
            pos: [1.0, 2.0, 3.0],
            dir: [0.0, 0.0, 2.0],
            id: 1,
            chr: 0,
        };
        assert!(im.spawn_hit(&sets, &arg, 0.5));
        let t = &im.tasks[0];
        // At least 0.7, * 0.75, * 1.2 for id 1.
        assert!((t.scale - 0.7 * 0.75 * 1.2).abs() < 1e-6);
        assert_eq!(t.dir, [0.0, 0.0, 1.0]);
        assert_eq!(t.mask, 1);
        assert!(im.spawn_hit_scaled(&sets, &ImpactArg { id: 6, ..arg }, 3.0));
        assert_eq!(im.tasks[1].scale, 3.0);
        assert!(im.spawn_blast(&sets, &ImpactArg { id: 2, ..arg }, 0.6, 0.5));
        assert_eq!((im.tasks[2].set, im.tasks[2].rise), (8, 0.5));
        while im.spawn_hit(&sets, &arg, 1.0) {}
        assert_eq!(im.tasks.len(), TASKS);
    }

    fn test_ctx(chr: usize, k: usize, s: &ImpactSet) -> SpawnCtx<'_> {
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
    fn an_impact_without_live_objects_ends_after_its_end_frames() {
        let mut sets = sets();
        sets[6].set.head.end_frames = 2;
        let mut im = Impacts::default();
        let mut fx = Modules::default();
        let (mut libc, mut vu0) = (LibcRandom::new(1), Vu0Random::default());
        let arg = ImpactArg {
            pos: [0.0, 10.0, 0.0],
            dir: [1.0, 0.0, 0.0],
            id: 0,
            chr: 1,
        };
        im.spawn_blast(&sets, &arg, 1.0, 0.5);
        let ctx = test_ctx;
        let mut frames = 0;
        while !im.tasks.is_empty() && frames < 10 {
            im.update(&mut fx, &sets, false, ctx, &mut libc, &mut vu0);
            im.post_update(&mut fx, &sets, false);
            frames += 1;
        }
        // Frame 1 starts; frame 2 finds nothing alive (ending); two end frames; removed the frame after.
        assert_eq!(frames, 5);
        // A pause holds it.
        im.spawn_blast(&sets, &arg, 1.0, 0.5);
        im.update(&mut fx, &sets, true, ctx, &mut libc, &mut vu0);
        im.post_update(&mut fx, &sets, true);
        assert_eq!(im.tasks[0].pos[1], 10.0);
        im.update(&mut fx, &sets, false, ctx, &mut libc, &mut vu0);
        assert_eq!(im.tasks[0].pos[1], 9.5);
        im.reset(&mut fx);
        assert!(im.tasks.is_empty());
    }
}
