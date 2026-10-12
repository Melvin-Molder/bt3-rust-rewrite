//! Technique effects: layer 1 of the effect scene. A technique's start motion raises effect bit 0x3E..0x40
//! (`BtlChar_SpawnFxBits3C`, src/battle/btl_char_fx_1.c), which starts the slot's shot (`EftShot_Start`,
//! src/battle/eft_emit.c): a timeline that turns the fighter's animation events into the technique's event bits
//! and times the charge and the firing (`EftTechEvt_*`, src/battle/eft_obj_tech.c), and an item of the slot's
//! effect type. The plain blast / beam of type 0 (`EftBlast_*`, src/battle/eft_shot_tech.c) fires along the aim,
//! adds a hit record per frame and ends on the timeline's END; its end raises flag 0xA8, the timeline's fire timer
//! flag 0xA7, which the technique actions wait for. The fighters' answers to a technique record (slots 2..4) are
//! here too (`BtlColl_GuardRush`, `BtlColl_HitByRush` and their helpers, src/battle/btl_hit_reaction.c), with
//! `BtlSuper_GetDamage` (src/battle/btl_tech.c).
//!
//! The prop shot of type 6 (`EftPropShot_*`) is the same item as the beam with its differences; the volley of
//! type 1 is in `btl_tech_volley`.
//!
//! The sweep of type 3 and the follow of type 7 are in `btl_tech_sweep`, the rush shot of type 9 in
//! `btl_tech_rush` with the thrown object of type 8, the ring shot of type 4 in `btl_tech_ring`, the multi of type 5
//! (its pieces are discs) in `btl_tech_disc`.
//!
//! Every item drives the emitters of its slot's set (header entry 4 of the slot's effect pack) through the particle
//! modules of `eft` (`update_parts`: `EftEmit_GetFlagsFromReq` and `EftEmit_SpawnOwn`, from the owner's technique
//! events), keeps them alive in its post-update (`EftEmit_UpdateAlive`, a fighter hit marking the phase 6 ones,
//! `EftEmit_MarkKind6`) and kills them when the scene resets its fighter (`EftEmit_KillAll`).
//!
//! Not ported: the skills' shots (slots 0, 1); the items of effect type 2 (their timeline runs, so
//! flag 0xA7 comes, but flag 0xA8 is left to the actions' timeout); the prop shots' models, stage blur, flash and
//! sound; abilities; the battle events.

use std::f32::consts::{FRAC_PI_2, PI};

use bt3_data::eft_pack::{EmitSet, SetHead};

use crate::btl_act::{id_class, is_technique_id};
use crate::btl_blast::{HitRec, Tag, World, hit_dir, home, is_facing, norm};
use crate::btl_chr::{
    Chr, Env, SideData, add, calc_aim_dir, dot, kmh, opp_yaw_from_facing, scale3, sub,
};
use crate::btl_hit::guard_kind;
use crate::btl_input::wrap_angle;
use crate::btl_obj::mask_to_node;
use crate::btl_tech_disc::{Discs, Multi};
use crate::btl_tech_ring::Ring;
use crate::btl_tech_rush::{ObjTech, RushShot};
use crate::btl_tech_sweep::{Follow, Sweep};
use crate::btl_tech_volley::{Shots, Volley, scene_rand_f};
use crate::eft::{EmitState, Modules, Nodes, SetAssets, spawn};

/// A technique slot's effect pack (`effectPacks[slot]`): what the ported effect types read of it.
#[derive(Clone, Debug, Default)]
pub(crate) struct TechPack {
    /// The slot's emitter set (header entry 4, `EftEmit_LoadSet` of every effect type's manager); `None` when it
    /// cannot be read.
    pub blast: Option<EmitSet>,
    /// Its decoded textures and module definitions (slots 2..4 only).
    pub assets: Option<crate::eft::SetAssets>,
    /// The placements of a rush technique's steps on the battle's stage (`BtlSuper_GetStagePlacement`).
    pub places: Option<[[f32; 4]; 5]>,
}

// EFT_TECH_EVT_*: a fighter's technique events of the frame.
pub(crate) const EV_START: u32 = 0x2;
pub(crate) const EV_FIRE: u32 = 0x4;
pub(crate) const EV_END: u32 = 0x8;
pub(crate) const EV_ABORT: u32 = 0x400;

// EFT_TECH_REQ_*.
const REQ_STOP: u32 = 2;
const REQ_RESTART: u32 = 4;
const REQ_EXPIRE: u32 = 8;

/// `EftTechEvt_Init`: timelines (one list entry per fighter).
const TIMELINES: usize = 2;
/// `EftBlastMgr_Init`: items per slot.
const ITEMS: usize = 2;

// EFT_TASK_*: the task tag bits the hit pass reports through (eft_core.h).
pub(crate) const T_HIT_CHAR: u32 = 0x1;
const T_HIT_STAGE: u32 = 0x4;
pub(crate) const T_MULTI: u32 = 0x4000;

/// `BTL_REACT_NOFLINCH`.
const REACT_NOFLINCH: i32 = 2;

/// `EftShotParam` of a technique slot (2..4), `EftShot_BuildParam` with the start's overrides.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TechDef {
    pub id: i32,
    /// The clash level of its records (+0x1E, the clash power).
    pub level: i32,
    /// 1 a technique, 2 an ultimate (slot 4).
    pub kind: i32,
    /// The hit direction kind (+0x13B).
    pub sub: i32,
    /// The hit shape: 0 two spheres, 1 two capsules from the muzzle.
    pub shape: i32,
    /// The shot count; a volley ends once it has fired this many times.
    pub shots: i32,
    /// `volley`: the shots of a volley's firing.
    pub volley: i32,
    /// `maxHits`: more than 0 makes a multi-hit; the hits it lands past it end it.
    pub max_hits: i32,
    /// `unkB`: frames between two hits of a multi-hit.
    pub hit_interval: i32,
    /// The effect type (-1 none, 0 the plain blast / beam).
    pub effect_type: i32,
    pub nodes: [i32; 6],
    /// Frames from the fire to the END event.
    pub life: i32,
    pub scale: f32,
    /// `power`: the beam's weight in a struggle.
    pub power: f32,
    /// `hitScale`: the records' scale (`EftHit_GetScale`), the factor of their radius against the stage.
    pub hit_scale: f32,
    /// Per frame (`BtlSuper_GetShotSpeed`).
    pub speed: f32,
    /// Radians per frame (`BtlSuper_GetShotTurnRate`).
    pub homing: f32,
    /// The technique's first flag word (1: hit records; 2: does not move; 4: nodes from the table; 0x20: keeps
    /// moving after the END / one hit at a time; 0x100 / 0x10000 / 0x200000 / 0x2000 and others).
    pub flags: u32,
}

impl TechDef {
    /// `EftShot_BuildParam` of slot 2..4 and `EftShot_Start`'s life, speed and turn rate from the request
    /// (`BtlSuper_GetShotTime` / `GetShotSpeed` / `GetShotTurnRate`).
    pub(crate) fn of(data: &SideData, slot: i32) -> Self {
        let t = data.tables.tech();
        Self {
            id: t.id(slot),
            level: t.clash_power(slot),
            kind: if slot == 4 { 2 } else { 1 },
            sub: t.hit_dir_kind(slot),
            shape: t.hit_shape(slot),
            shots: t.shots_raw(slot),
            volley: t.hits_b_raw(slot),
            max_hits: t.max_hits(slot),
            hit_interval: t.hit_interval(slot),
            effect_type: t.effect_type(slot),
            nodes: t.effect_nodes(slot),
            life: (t.shot_time(slot) * 30.0) as i32,
            scale: t.shot_scale(slot),
            hit_scale: t.hit_scale(slot),
            power: t.power(slot),
            speed: t.shot_speed(slot) * 10.0 * kmh(1.0),
            homing: t.shot_turn(slot) * 3.141_592_7 / 180.0 * (1.0 / 30.0),
            flags: t.flags_a(slot),
        }
    }

    /// `EftShot_GetLeadTime`: frames from the start to flag 0xA7: 0.85 s; a technique of hit direction kind 5:
    /// 0.8 s; an ultimate 1.3 s (0.8 s of kind 5, 0.7 s for 0x268, none for 0x290).
    fn lead_time(&self) -> i32 {
        let mut t = 0.85f32 * 30.0;
        match self.kind {
            1 if self.sub == 5 => t = 0.8f32 * 30.0,
            2 => {
                t = 0.8f32 * 30.0;
                if self.sub != 5 {
                    t = 1.3f32 * 30.0;
                }
                if self.id == 0x268 {
                    t = 0.7f32 * 30.0;
                }
                if self.id == 0x290 {
                    t = 0.0;
                }
            }
            _ => {}
        }
        t as i32
    }
}

/// The source of a technique record (`EftHSlot`): the fighter, its slot and the definition.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TechSrc {
    pub side: usize,
    pub slot: i32,
    pub def: TechDef,
}

/// An `EftTechEvtTask`: one technique's events and its two timers.
#[derive(Clone, Copy, Debug)]
struct Timeline {
    side: usize,
    kind: i32,
    sub: i32,
    fire_timer: i32,
    end_timer: i32,
    end_time: i32,
    /// 2 the fire timer runs, 4 the end timer runs, 8 the rush connected, 0x10 stepped this frame.
    flags: u32,
    frame: i32,
}

/// `EftEmitNodes`: the node of each event slot (and its pair), fixed when the slot's event first comes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EmitNodes {
    /// Bit 2i slot i resolved, bit 2i + 1 slot i is a pair; 0x1000 frozen.
    flags: u32,
    flags_c: u32,
    n: [[(i32, [f32; 3]); 2]; 6],
    c: [(i32, [f32; 3]); 6],
}

/// An `EftBlast` item (effect type 0), or an `EftPropShot` (type 6: the same shot carrying a prop model, without
/// the charge's scaling or the speed track).
#[derive(Clone, Debug)]
struct Beam {
    id: u32,
    src: TechSrc,
    prop: bool,
    tag: Tag,
    state: u8,
    /// 1 ending, 2 dead, 4 aborted, 8 END seen, 0x10 fired, 0x20 held by the hit pass, 0x40 started, 0x200 the
    /// speed follows the second width, 0x800 / 0x1000 the charge does not scale it / the 0x268 scale (a prop
    /// shot: technique 0x19A, which hides part of its fighter / flag 0xA8 raised early).
    flags: u32,
    speed: f32,
    scale: f32,
    draw_scale: f32,
    radius: f32,
    ratio: f32,
    timer: f32,
    life: f32,
    dir: [f32; 3],
    origin: [f32; 3],
    head: [f32; 3],
    tail: [f32; 3],
    vel: [f32; 3],
    nodes: EmitNodes,
    /// The emitters (`emit`).
    emit: Option<EmitState>,
    head_set: SetHead,
    /// `EftSetState`'s width animations.
    trail_width: f32,
    trail_time: f32,
    width2: f32,
    time2: f32,
}

/// The technique layer (`gEftShot`, `gEftTechEvt`).
#[derive(Clone, Debug)]
pub(crate) struct TechFx {
    events: [u32; 2],
    requests: [u32; 2],
    timelines: Vec<Timeline>,
    /// `curSlot`: the slot each fighter started last, -1 outside techniques and skills.
    cur_slot: [i32; 2],
    /// The items by fighter and slot.
    beams: [[Vec<Beam>; 5]; 2],
    volleys: [[Vec<Volley>; 5]; 2],
    sweeps: [[Vec<Sweep>; 5]; 2],
    follows: [[Vec<Follow>; 5]; 2],
    rush_shots: [[Vec<RushShot>; 5]; 2],
    obj_techs: [[Vec<ObjTech>; 5]; 2],
    multis: [[Vec<Multi>; 5]; 2],
    rings: [[Vec<Ring>; 5]; 2],
    /// The discs: ki blasts of types 4 / 5 and the multis' pieces (layer 3).
    pub discs: Discs,
    /// The blast objects the volleys fire (layer 3).
    pub shots: Shots,
    /// `gBtlScene->randState`: the scene's generator.
    rand: u32,
}

impl Default for TechFx {
    fn default() -> Self {
        Self {
            events: [0; 2],
            requests: [0; 2],
            timelines: Vec::new(),
            cur_slot: [0; 2],
            beams: Default::default(),
            volleys: Default::default(),
            sweeps: Default::default(),
            follows: Default::default(),
            rush_shots: Default::default(),
            obj_techs: Default::default(),
            multis: Default::default(),
            rings: Default::default(),
            discs: Discs::default(),
            shots: Shots::default(),
            rand: 0,
        }
    }
}

/// `BtlScene_IsCharStopped`.
pub(crate) fn stopped(w: &World, side: usize) -> bool {
    w.paused || w.chars[side].is_frozen()
}

/// `BtlCharApi_IsInTechnique`.
fn in_technique(chr: &Chr) -> bool {
    is_technique_id(chr.act.current)
}

/// `BtlCharApi_IsInSkill`.
fn in_skill(chr: &Chr) -> bool {
    (0xFD..0x103).contains(&chr.act.current)
}

/// `BtlCharApi_IsInRushSequence`.
pub(crate) fn in_rush_sequence(chr: &Chr) -> bool {
    matches!(chr.act.current, 0x12D..=0x12F | 0x139..=0x13B)
}

/// `EftShot_AttrMaskFromBits`: event bits to animation attributes.
fn attr_mask(bits: u32) -> u64 {
    let mut mask = u64::from(bits << 8) & 0x200;
    for (b, m) in [
        (0x4, 0x400),
        (0x8, 0x800),
        (0x10, 0x1000),
        (0x20, 0x2000),
        (0x40, 0x4000),
        (0x80, 0x20_0000),
        (0x100, 0x40_0000),
        (0x200, 0x80_0000),
    ] {
        if bits & b != 0 {
            mask |= m;
        }
    }
    mask
}

/// `EftShot_GetAttrValue`.
fn attr_value(chr: &Chr, bits: u32) -> u32 {
    let mask = attr_mask(bits);
    if chr.anim_test_attr(mask) {
        chr.obj.event_arg(mask)
    } else {
        0
    }
}

/// `EftShot_HasTwoAttrs`: two of the bits 0x10000 / 0x20000 / 0x40000.
fn has_two_attrs(chr: &Chr, bits: u32) -> bool {
    let v = attr_value(chr, bits);
    [0x1_0000, 0x2_0000, 0x4_0000]
        .iter()
        .filter(|&&b| v & b != 0)
        .count()
        >= 2
}

/// `EftShot_GetAttrPair`: 0x20000 the node 0x2D, 0x10000 0x1F, 0x40000 makes the other 0x36 (0x11 none).
fn attr_pair(chr: &Chr, bits: u32) -> (i32, i32) {
    let v = attr_value(chr, bits);
    let (mut a, mut b) = (0x11, 0x11);
    if v & 0x7_0000 == 0 {
        return (a, b);
    }
    if v & 0x2_0000 != 0 {
        a = 0x2D;
    }
    if v & 0x1_0000 != 0 {
        b = 0x1F;
    }
    if v & 0x4_0000 != 0 {
        if v & 0x2_0000 != 0 {
            b = 0x36;
        }
        if v & 0x1_0000 != 0 {
            a = 0x36;
        }
    }
    (a, b)
}

/// The flag word of a slot's definition; the skills' (slots 0, 1) are not ported.
fn slot_flags(data: &SideData, slot: i32) -> u32 {
    if (2..5).contains(&slot) {
        data.tables.tech().flags_a(slot)
    } else {
        0
    }
}

/// `EftShot_GetAttrKind`: the node of an event: 0x11 without a current slot or with two attributes, 0x36 for a
/// definition with flag 0x2000, else the event argument's node (0 without one).
fn attr_kind(chr: &Chr, data: &SideData, cur_slot: i32, bits: u32) -> i32 {
    if cur_slot < 0 || has_two_attrs(chr, bits) {
        return 0x11;
    }
    if slot_flags(data, cur_slot) & 0x2000 != 0 {
        return 0x36;
    }
    let mask = attr_mask(bits);
    if !chr.anim_test_attr(mask) {
        return 0;
    }
    match chr.obj.event_arg(mask) {
        0 => 0,
        v => mask_to_node(v),
    }
}

impl EmitNodes {
    /// The slots as the emitters read them.
    pub(crate) fn spawn_nodes(&self) -> Nodes {
        Nodes::from_slots(
            self.flags & 0xFFF,
            std::array::from_fn(|i| [self.n[i][0].1, self.n[i][1].1]),
            std::array::from_fn(|i| self.c[i].1),
        )
    }

    /// `EftBlastObj_Init`'s slot: slot `i` takes the node the animation attribute of its event names, sampled now
    /// (its resolved bit is left as it is).
    pub(crate) fn set_attr_slot(&mut self, chr: &Chr, data: &SideData, cur_slot: i32, i: usize) {
        const BITS: [u32; 6] = [2, 4, 8, 0x10, 0x20, 0x40];
        let node = attr_kind(chr, data, cur_slot, BITS[i]);
        self.n[i][0] = (node, chr.obj.node_pos(node as u32));
    }

    /// Slot `i`'s first node taken as `node`, sampled now (the volley's shots of technique 0x1C6).
    pub(crate) fn set_slot_node(&mut self, chr: &Chr, i: usize, node: i32) {
        self.n[i][0] = (node, chr.obj.node_pos(node as u32));
    }

    /// The position of event slot `i` (`n[i][0].pos`).
    pub(crate) fn pos(&self, i: usize) -> [f32; 3] {
        self.n[i][0].1
    }

    /// The node of event slot `i` (`n[i][0].id`).
    pub(crate) fn node(&self, i: usize) -> i32 {
        self.n[i][0].0
    }

    /// `EftEmit_UpdateNodesReq` / `EftEmit_UpdateNodes`: each slot takes its node the first time its event comes
    /// (from the definition with flag 4, else from the event), then every resolved slot is sampled (an unresolved
    /// one at slot 0's node once that is). ABORT freezes them.
    pub(crate) fn update(
        &mut self,
        chr: &Chr,
        data: &SideData,
        def: &TechDef,
        cur_slot: i32,
        events: u32,
    ) {
        const BITS: [u32; 6] = [2, 4, 8, 0x10, 0x20, 0x40];
        let req = events & 0x47E;
        if req & 0x400 != 0 {
            self.flags |= 0x1000;
        }
        if self.flags & 0x1000 != 0 {
            return;
        }
        let pos = |n: i32| chr.obj.node_pos(n as u32);
        for i in 0..6 {
            let (one, pair) = (1 << (i * 2), 1 << (i * 2 + 1));
            if self.flags & one == 0 && req & BITS[i] != 0 {
                self.flags |= one;
                if def.flags & 4 != 0 {
                    self.n[i][0].0 = def.nodes[i];
                } else if has_two_attrs(chr, BITS[i]) {
                    self.flags |= pair;
                    let (a, b) = attr_pair(chr, BITS[i]);
                    self.n[i][0].0 = a;
                    self.n[i][1].0 = b;
                } else {
                    self.n[i][0].0 = attr_kind(chr, data, cur_slot, BITS[i]);
                }
            }
            let first = self.n[0][0].0;
            for k in 0..2 {
                let bit = if k == 0 { one } else { pair };
                if self.flags & bit != 0 {
                    self.n[i][k].1 = pos(self.n[i][k].0);
                } else if self.flags & 1 != 0 {
                    self.n[i][k].1 = pos(first);
                }
            }
            if def.flags & 4 == 0 && def.flags & 8 != 0 {
                if self.flags_c & (1 << i) == 0 && req & BITS[i] != 0 {
                    self.flags_c |= 1 << i;
                    self.flags |= one;
                    self.c[i].0 = def.nodes[i];
                }
                if self.flags_c & (1 << i) != 0 {
                    self.c[i].1 = pos(self.c[i].0);
                } else if self.flags_c & 1 != 0 {
                    self.c[i].1 = pos(self.c[0].0);
                }
            }
        }
    }
}

/// The emitter set of a fighter's technique slot and its assets.
pub(crate) fn slot_set<'d>(data: &SideData<'d>, slot: i32) -> Option<(&'d EmitSet, &'d SetAssets)> {
    let pack = (*data.tech_packs.get(usize::try_from(slot).ok()?)?)?;
    Some((pack.blast.as_ref()?, pack.assets.as_ref()?))
}

/// `EftEmit_InitState` of an item's slot set; `None` when the slot has none.
pub(crate) fn emit_state(data: &SideData, slot: i32) -> Option<EmitState> {
    slot_set(data, slot).map(|(set, _)| EmitState::new(set))
}

/// An item's `*_UpdateParts` (mode 0): every emitter of its slot's set that `skip` leaves gets its flags from the
/// owner's events (`EftEmit_GetFlagsFromReq`, `ending` / `kill` the item's) and is driven at `pos` along `dir`
/// (`EftEmit_SpawnOwn`), sized by `scale`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_parts(
    fx: &mut Modules,
    w: &mut World,
    src: &TechSrc,
    state: &mut EmitState,
    nodes: &EmitNodes,
    events: u32,
    ending: bool,
    kill: bool,
    pos: [f32; 3],
    dir: [f32; 3],
    scale: f32,
    skip: impl Fn(&EmitSet, u8, usize) -> bool,
) {
    drive_parts(
        fx,
        w,
        src,
        state,
        nodes,
        pos,
        dir,
        scale,
        skip,
        |st, set, kind, n| st.flags_from_req(set, kind, n, ending, kill, events),
    );
}

/// `*_SpawnParts` with the reset flags (`EftEmit_GetResetFlags`, after a hit moved the item).
#[allow(clippy::too_many_arguments)]
pub(crate) fn reset_parts(
    fx: &mut Modules,
    w: &mut World,
    src: &TechSrc,
    state: &mut EmitState,
    nodes: &EmitNodes,
    pos: [f32; 3],
    dir: [f32; 3],
    scale: f32,
    skip: impl Fn(&EmitSet, u8, usize) -> bool,
) {
    drive_parts(
        fx,
        w,
        src,
        state,
        nodes,
        pos,
        dir,
        scale,
        skip,
        |st, set, kind, n| st.reset_flags(set, kind, n),
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_parts(
    fx: &mut Modules,
    w: &mut World,
    src: &TechSrc,
    state: &mut EmitState,
    nodes: &EmitNodes,
    pos: [f32; 3],
    dir: [f32; 3],
    scale: f32,
    skip: impl Fn(&EmitSet, u8, usize) -> bool,
    mut flags_of: impl FnMut(&mut EmitState, &EmitSet, u8, usize) -> u32,
) {
    let side = src.side;
    let Some((set, assets)) = slot_set(w.data[side], src.slot) else {
        return;
    };
    let ctx = crate::btl_blast::spawn_ctx(w, side, set, assets, src.slot as usize);
    let nodes = nodes.spawn_nodes();
    for kind in 0..0x13u8 {
        if set.head.mask & (1 << kind) == 0 {
            continue;
        }
        let Some(g) = set.groups[usize::from(kind)] else {
            continue;
        };
        for sub in 0..g.count {
            if skip(set, kind, sub) {
                continue;
            }
            let f = flags_of(state, set, kind, g.first + sub);
            if f != 0 {
                spawn(
                    fx, state, &nodes, &ctx, pos, dir, kind, sub, f, scale, w.libc, w.vu0,
                );
            }
        }
    }
}

/// `EftEmit_UpdateAlive` of an item's emitters, then `EftEmit_MarkKind6` when `mark` (a fighter was hit).
pub(crate) fn parts_alive(
    fx: &mut Modules,
    data: &SideData,
    src: &TechSrc,
    state: &mut EmitState,
    mark: bool,
) {
    if let Some((set, _)) = slot_set(data, src.slot) {
        state.update_alive(set, fx);
        if mark {
            state.mark_kind6(set);
        }
    }
}

/// `EftAim_GetDir`: under lock-on at the opponent's node 0x11 within 45 degrees, else along the model's facing.
pub(crate) fn aim_dir(w: &World, side: usize, from: [f32; 3]) -> [f32; 3] {
    let me = &w.chars[side];
    let out = if me.flags.test(5) {
        let q = PI / 4.0;
        calc_aim_dir(me, &w.chars[side ^ 1], w.data[side], 0x11, from, -q, q)
    } else {
        let yaw = me.pose.rot[1];
        [yaw.sin(), 0.0, yaw.cos()]
    };
    norm(out)
}

/// `EftAim_GetDirKeep` (aim mode 0): the same aim, but the facing when it is more than 60 degrees off it.
pub(crate) fn aim_dir_keep(w: &World, side: usize, from: [f32; 3]) -> [f32; 3] {
    let me = &w.chars[side];
    let yaw = me.pose.rot[1];
    let fwd = [yaw.sin(), 0.0, yaw.cos()];
    if !me.flags.test(5) {
        return fwd;
    }
    let q = PI / 4.0;
    let out = norm(calc_aim_dir(
        me,
        &w.chars[side ^ 1],
        w.data[side],
        0x11,
        from,
        -q,
        q,
    ));
    if dot([out[0], 0.0, out[2]], fwd) < 0.5 {
        fwd
    } else {
        out
    }
}

/// `EftEmit_UpdateTrailWidth`: the trail width animation, or its step per frame up to key 2.
pub(crate) fn update_trail_width(width: &mut f32, t: &mut f32, h: &SetHead) {
    if !step_width(width, t, h.width, h.width_time, h.width_split) {
        *width += h.width_step;
        if 0.0 < h.width[2] && h.width[2] < *width {
            *width = h.width[2];
        }
    }
}

/// `EftEmit_UpdateTrailWidth` / `EftEmit_UpdateWidth2`: a width animated from key 0 to key 1 over `split` of
/// `time` seconds, then to key 2.
pub(crate) fn step_width(
    width: &mut f32,
    t: &mut f32,
    keys: [f32; 3],
    time: f32,
    split: f32,
) -> bool {
    if !(0.0 < time) {
        return false;
    }
    let total = time * 30.0;
    let first = total * split;
    *t += 1.0;
    *width = if *t < first {
        (keys[0] + (keys[1] - keys[0]) * (*t / first)).max(0.0)
    } else if *t < total {
        (keys[1] + (keys[2] - keys[1]) * ((*t - first) / (total - first))).max(0.0)
    } else {
        keys[2]
    };
    true
}

impl Beam {
    /// `EftBlast_Init` / `EftPropShot_Init` (with `EftPropShot_InitProp`, whose technique 0x165 draws two numbers
    /// of the scene generator).
    fn new(
        id: u32,
        src: TechSrc,
        prop: bool,
        chr: &Chr,
        data: &SideData,
        set: &EmitSet,
        rand: &mut u32,
        ts: bool,
    ) -> Self {
        let def = src.def;
        let head_set = set.head;
        let mut b = Self {
            id,
            src,
            prop,
            tag: Tag::default(),
            state: 0,
            flags: 0,
            speed: def.speed,
            scale: def.scale,
            draw_scale: 0.0,
            radius: 0.0,
            ratio: 0.0,
            timer: 0.0,
            life: 0.0,
            dir: [0.0; 3],
            origin: [0.0; 3],
            head: [0.0; 3],
            tail: [0.0; 3],
            vel: [0.0; 3],
            nodes: EmitNodes::default(),
            emit: emit_state(data, src.slot),
            head_set,
            trail_width: head_set.width[0],
            trail_time: 0.0,
            width2: head_set.width_b[0],
            time2: 0.0,
        };
        if prop {
            b.draw_scale = b.scale;
            b.radius = b.scale;
            if head_set.flags & 4 != 0 {
                b.speed = b.width2;
                b.flags |= 0x200;
            }
            b.life = f32::from(head_set.end_frames);
            if def.id == 0x165 {
                // The bobbing of the prop in the hand.
                scene_rand_f(rand, ts);
                scene_rand_f(rand, ts);
            }
            if def.id == 0x19A {
                b.flags |= 0x800;
            }
            return b;
        }
        if def.id == 0x268 {
            // BtlCharApi_GetSkillCount3Ratio.
            b.ratio = (chr.skill_count3 as f32 / 3.0).clamp(0.0, 1.0);
            b.scale += b.ratio * 0.3;
            b.flags |= 0x1800;
        }
        if def.id == 0x2CD {
            // BtlCharApi_GetUnkE60Ratio.
            b.ratio = (chr.boost_stock as f32 / 5.0).clamp(0.0, 1.0);
            b.flags |= 0x800;
            b.scale = b.scale * 0.8 + b.ratio.min(0.7);
        }
        b.draw_scale = b.scale;
        b.radius = b.scale;
        if def.id == 0x14B {
            b.flags |= 0x800;
        }
        if head_set.flags & 4 != 0 {
            b.speed = b.width2 * def.scale;
            b.flags |= 0x200;
        }
        b.life = f32::from(head_set.end_frames);
        b
    }

    fn hand(&self) -> [f32; 3] {
        self.nodes.n[0][0].1
    }

    fn muzzle(&self) -> [f32; 3] {
        self.nodes.n[1][0].1
    }

    /// `EftBlast_Update`: charge (scaled by the fighter's charge), fire from the muzzle on FIRE, fly (homing),
    /// end on END; ABORT ends it at once. While it flies it adds a hit record when the definition has flag 1.
    /// Returns the record; the item is dead when flag 2 is set.
    fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        events: u32,
        cur_slot: i32,
    ) -> Option<HitRec> {
        let side = self.src.side;
        if stopped(w, side) {
            return None;
        }
        let def = self.src.def;
        let test = |b: u32| events & b != 0;
        // EftShot_PlayFireSound concerns skills only.
        self.nodes
            .update(&w.chars[side], w.data[side], &def, cur_slot, events);
        if self.flags & 0x40 == 0 {
            if test(EV_START) {
                self.dir = aim_dir(&*w, side, self.hand());
                self.state = 0;
                self.flags |= 0x40;
            } else if test(EV_FIRE) {
                self.state = 0;
                self.flags |= 0x40;
            }
        }
        let mut advance = false;
        if self.flags & 0x40 != 0 {
            match self.state {
                0 => {
                    if !self.prop && self.flags & 0x800 == 0 {
                        self.ratio = w.chars[side].charge;
                        self.draw_scale = self.scale * (self.ratio * 0.5 + 1.0);
                        self.radius = self.scale * (self.ratio * 0.0 + 1.0);
                    }
                    if test(EV_FIRE) {
                        self.flags |= 0x10;
                        self.origin = self.muzzle();
                        self.head = self.origin;
                        self.tail = w.chars[side].obj.node_pos(0x11);
                        self.dir = aim_dir_keep(&*w, side, self.head);
                        self.vel = scale3(self.dir, self.speed);
                        if self.prop {
                            if self.flags & 0x800 != 0 {
                                // BtlCharApi_ObjSetFlag100: model class A's nodes are hidden.
                                w.chars[side].obj.flags |= 0x100;
                            }
                        } else if self.flags & 0x1000 != 0 && self.ratio > 0.0 {
                            self.scale = def.scale + self.ratio * 0.3 * 4.0;
                        }
                        self.state = 1;
                    }
                }
                1 => advance = true,
                2 => {
                    if !self.prop
                        && self.flags & 0x1000 != 0
                        && self.ratio > 0.0
                        && self.draw_scale < self.scale
                    {
                        self.draw_scale = (self.draw_scale + 0.1).min(self.scale);
                    }
                    if self.flags & 0x20 == 0 {
                        self.tail = self.head;
                    }
                    if def.flags & 2 == 0 {
                        if self.flags & 0x20 == 0 {
                            if def.homing > 0.0 {
                                self.dir = home(
                                    &*w.chars,
                                    self.head,
                                    &mut self.dir,
                                    side,
                                    self.speed,
                                    def.homing,
                                );
                                self.vel = scale3(self.dir, self.speed);
                            }
                            self.head = add(self.head, self.vel);
                        } else {
                            self.head = self.tag.pos;
                        }
                    }
                    if test(EV_END) {
                        self.flags |= 9;
                        self.state = 3;
                    }
                }
                3 if self.prop => advance = true,
                3 => {
                    if def.flags & 0x20 != 0 {
                        self.head = add(self.head, self.vel);
                    }
                }
                _ => {}
            }
        }
        if test(EV_ABORT) {
            if self.flags & 8 == 0 {
                self.flags |= 4;
            }
            self.flags |= 1;
        }
        // EftBlast_UpdateParts / EftPropShot_UpdateParts (the prop's model is not ported).
        if let Some(st) = &mut self.emit {
            let scale = if self.prop {
                self.scale
            } else {
                self.draw_scale
            };
            let (ending, kill) = (self.flags & 1 != 0, self.flags & 4 != 0);
            update_parts(
                fx,
                w,
                &self.src,
                st,
                &self.nodes,
                events,
                ending,
                kill,
                self.head,
                self.dir,
                scale,
                |_, _, _| false,
            );
        }
        if !self.prop && self.flags & 0x10 != 0 && self.flags & 0x200 != 0 {
            let h = self.head_set;
            step_width(
                &mut self.width2,
                &mut self.time2,
                h.width_b,
                h.width_b_time,
                h.width_b_split,
            );
            self.speed = self.width2 * def.scale;
            self.vel = scale3(self.dir, self.speed);
        }
        // EftBlast_UpdateBlur (definition flag 0x200): the stage blur is not ported.
        if advance {
            self.state += 1;
        }
        if self.flags & 1 != 0 {
            self.timer += 1.0;
        }
        // Technique 0x19A's prop shot raises flag 0xA8 as soon as it ends, then waits while its fighter's model
        // shows part 3 (`BtlCharApi_ObjHasFlags42`).
        let wait = self.prop && self.flags & 0x800 != 0;
        if wait && self.flags & 3 == 1 && (self.timer >= self.life || self.flags & 4 != 0) {
            if self.flags & 0x1000 == 0 {
                if def.kind != 0 {
                    w.chars[side].flags.set_held(0xA8);
                }
                self.flags |= 0x1000;
            } else if w.chars[side].obj.flags & 0x42 != 0x42 {
                self.flags &= !0x1000;
            }
        }
        if self.flags & 2 != 0 {
            if wait {
                w.chars[side].obj.flags &= !0x100;
            }
            return None;
        }
        if self.flags & 1 != 0 {
            if !(wait && self.flags & 0x1000 != 0)
                && (self.flags & 4 != 0 || self.timer >= self.life)
            {
                self.flags |= 2;
            }
            return None;
        }
        if def.flags & 1 == 0 || self.flags & 0x10 == 0 {
            return None;
        }
        update_trail_width(&mut self.trail_width, &mut self.trail_time, &self.head_set);
        // EftBlast_AddHit and EftHit_Add (nothing while time is stopped).
        if w.paused {
            return None;
        }
        let mut rec = HitRec::technique(
            self.src,
            self.id,
            self.head,
            self.tail,
            self.vel,
            self.muzzle(),
            self.radius * self.trail_width,
        )?;
        rec.origin = self.origin;
        // EftHit_InitMultiHit.
        if def.max_hits > 0 {
            self.tag.flags |= T_MULTI;
            self.tag.flags &= !T_HIT_CHAR;
        }
        Some(rec)
    }

    /// `EftBlast_PostUpdate`: a fighter hit stops the head for definitions with flag 0x10000; result bit 0 holds
    /// the head where the pass put it (the tail follows), bit 2 ends the beam.
    fn post_update(&mut self, w: &World, fx: &mut Modules) {
        if stopped(w, self.src.side) {
            return;
        }
        if let Some(st) = &mut self.emit {
            let hit = self.tag.flags & T_HIT_CHAR != 0;
            parts_alive(fx, w.data[self.src.side], &self.src, st, hit);
        }
        if !self.prop {
            if self.tag.flags & T_HIT_CHAR != 0 {
                if self.src.def.flags & 0x1_0000 != 0 {
                    self.flags &= !0x10;
                }
                self.flags |= 0x2000;
            } else if self.tag.flags & T_HIT_STAGE != 0 {
                self.flags |= 0x4000;
            }
        }
        if self.tag.result & 1 != 0 {
            let d = sub(self.tag.pos, self.head);
            self.head = self.tag.pos;
            self.tail = add(self.tail, d);
            self.flags |= 0x20;
        } else {
            self.flags &= !0x20;
        }
        if self.tag.result & 4 != 0 {
            self.flags |= 1;
        }
    }
}

impl Timeline {
    /// `EftTechEvt_IsRushTech`: a technique of hit direction kind 5, or one whose fighter is in a rush sequence.
    fn is_rush(&self, chars: &[Chr; 2]) -> bool {
        self.kind != 0 && (self.sub == 5 || in_rush_sequence(&chars[self.side]))
    }
}

impl TechFx {
    /// `EftTechEvt_GetEvents`.
    #[cfg(test)]
    pub(crate) fn events(&self, side: usize) -> u32 {
        self.events[side]
    }

    /// `BtlChar_SpawnFxBits3C` and `EftShot_Start`: effect bit 0x3C..0x40 (the highest wins) starts the shot of
    /// slot 0..4: the timeline, and an item of the effect type (type 0 only) under the slot, at most two.
    pub(crate) fn request(
        &mut self,
        me: &Chr,
        opp: &Chr,
        data: &SideData,
        next_id: &mut u32,
        time_stopped: bool,
    ) {
        let mut slot = -1;
        for (bit, s) in [(0x3C, 0), (0x3D, 1), (0x3E, 2), (0x3F, 3), (0x40, 4)] {
            if me.test_fx(bit) {
                slot = s;
            }
        }
        // The skills' shots (slots 0, 1) are not ported.
        if !(2..5).contains(&slot) {
            return;
        }
        let side = me.player;
        let Some(pack) = data.tech_packs[slot as usize] else {
            return;
        };
        let def = TechDef::of(data, slot);
        self.cur_slot[side] = slot;
        // EftTechEvt_Start: the list holds one timeline per fighter.
        if self.timelines.len() < TIMELINES {
            self.timelines.push(Timeline {
                side,
                kind: def.kind,
                sub: def.sub,
                fire_timer: def.lead_time(),
                end_timer: def.life - 1,
                end_time: def.life - 1,
                flags: 0,
                frame: 0,
            });
            self.requests[side] = 0;
        }
        let src = TechSrc { side, slot, def };
        let Some(set) = &pack.blast else {
            return;
        };
        match def.effect_type {
            0 | 6 => {
                let items = &mut self.beams[side][slot as usize];
                if items.len() < ITEMS {
                    *next_id += 1;
                    let prop = def.effect_type == 6;
                    items.push(Beam::new(
                        *next_id,
                        src,
                        prop,
                        me,
                        data,
                        set,
                        &mut self.rand,
                        time_stopped,
                    ));
                }
            }
            3 => {
                let items = &mut self.sweeps[side][slot as usize];
                if items.len() < ITEMS {
                    *next_id += 1;
                    items.push(Sweep::new(*next_id, src, me, opp, data, set));
                }
            }
            8 => {
                let items = &mut self.obj_techs[side][slot as usize];
                if items.len() < ITEMS {
                    *next_id += 1;
                    items.push(ObjTech::new(*next_id, src, data, set));
                }
            }
            9 => {
                let items = &mut self.rush_shots[side][slot as usize];
                if items.len() < ITEMS {
                    *next_id += 1;
                    items.push(RushShot::new(*next_id, src, data, set));
                }
            }
            7 => {
                let items = &mut self.follows[side][slot as usize];
                if items.len() < ITEMS {
                    *next_id += 1;
                    items.push(Follow::new(*next_id, src, data, set));
                }
            }
            4 => {
                let items = &mut self.rings[side][slot as usize];
                if items.len() < ITEMS {
                    *next_id += 1;
                    items.push(Ring::new(*next_id, src, data, set));
                }
            }
            5 => {
                let items = &mut self.multis[side][slot as usize];
                if items.len() < ITEMS {
                    items.push(Multi::new(src, data, set));
                }
            }
            1 => {
                let items = &mut self.volleys[side][slot as usize];
                if items.len() < ITEMS
                    && let Some(v) = Volley::new(src, data, set)
                {
                    items.push(v);
                }
            }
            _ => {}
        }
    }

    /// `EftTechEvt_RequestStop`: the fighter's timers stop for good; a pending END is withdrawn.
    pub(crate) fn request_stop(&mut self, side: usize) {
        self.requests[side] |= REQ_STOP;
        self.events[side] &= !EV_END;
    }

    /// `EftTechEvt_RequestRestart` / `RequestExpire`: unless the END is out.
    pub(crate) fn request_restart(&mut self, side: usize, expire: bool) {
        if self.events[side] & EV_END == 0 {
            self.requests[side] |= if expire { REQ_EXPIRE } else { REQ_RESTART };
        }
    }

    pub(crate) fn tag_mut(&mut self, id: u32) -> Option<&mut Tag> {
        if let Some(t) = self.shots.tag_mut(id) {
            return Some(t);
        }
        if let Some(t) = self.discs.tag_mut(id) {
            return Some(t);
        }
        if let Some(r) = self
            .rings
            .iter_mut()
            .flatten()
            .flatten()
            .find(|r| r.id() == id)
        {
            return Some(&mut r.tag);
        }
        if let Some(s) = self
            .sweeps
            .iter_mut()
            .flatten()
            .flatten()
            .find(|s| s.id() == id)
        {
            return Some(&mut s.tag);
        }
        if let Some(f) = self
            .follows
            .iter_mut()
            .flatten()
            .flatten()
            .find(|f| f.id() == id)
        {
            return Some(&mut f.tag);
        }
        if let Some(r) = self
            .rush_shots
            .iter_mut()
            .flatten()
            .flatten()
            .find(|r| r.id() == id)
        {
            return Some(&mut r.tag);
        }
        if let Some(o) = self
            .obj_techs
            .iter_mut()
            .flatten()
            .flatten()
            .find(|o| o.id() == id)
        {
            return Some(&mut o.tag);
        }
        self.beams
            .iter_mut()
            .flatten()
            .flatten()
            .find(|b| b.id == id)
            .map(|b| &mut b.tag)
    }

    /// Layer 1 of `BtlScene_Update`: the timelines (`EftTechEvt_Update`, the events cleared first), the current
    /// slots (`EftShotChar_Update`), then every item, fighter 0's slots first; a dead item is removed at once, and
    /// one of a technique raises flag 0xA8 (`EftBlast_Term`).
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        records: &mut Vec<HitRec>,
        max: usize,
        next_id: &mut u32,
    ) {
        self.events = [0; 2];
        let mut i = 0;
        while i < self.timelines.len() {
            let mut t = self.timelines[i];
            t.flags &= !0x10;
            let dead = if stopped(w, t.side) {
                false
            } else if t.is_rush(w.chars) {
                self.update_rush(&mut t, w.chars)
            } else {
                self.update_normal(&mut t, w.chars, w.env)
            };
            if dead {
                self.timelines.remove(i);
            } else {
                self.timelines[i] = t;
                i += 1;
            }
        }
        for (side, chr) in w.chars.iter().enumerate() {
            if !in_technique(chr) && !in_skill(chr) {
                self.cur_slot[side] = -1;
            }
        }
        self.shots.cur_slot = self.cur_slot;
        for side in 0..2 {
            for slot in 0..5 {
                let mut k = 0;
                while k < self.beams[side][slot].len() {
                    let beam = &mut self.beams[side][slot][k];
                    let rec = beam.update(w, fx, self.events[side], self.cur_slot[side]);
                    // EftBlast_Term / EftPropShot_Term (not technique 0x19A's prop, which raised it).
                    let a8 = beam.src.def.kind != 0 && !(beam.prop && beam.flags & 0x800 != 0);
                    if let Some(rec) = rec
                        && records.len() < max
                    {
                        records.push(rec);
                    }
                    if beam.flags & 2 != 0 {
                        if a8 {
                            w.chars[side].flags.set_held(0xA8);
                        }
                        self.beams[side][slot].remove(k);
                    } else {
                        k += 1;
                    }
                }
                let mut k = 0;
                while k < self.sweeps[side][slot].len() {
                    let s = &mut self.sweeps[side][slot][k];
                    if let Some(rec) = s.update(w, fx, self.events[side], self.cur_slot[side])
                        && records.len() < max
                    {
                        records.push(rec);
                    }
                    if s.dead() {
                        if s.src().def.kind != 0 {
                            w.chars[side].flags.set_held(0xA8);
                        }
                        self.sweeps[side][slot].remove(k);
                    } else {
                        k += 1;
                    }
                }
                let mut k = 0;
                while k < self.follows[side][slot].len() {
                    let f = &mut self.follows[side][slot][k];
                    if let Some(rec) = f.update(w, fx, self.events[side], self.cur_slot[side])
                        && records.len() < max
                    {
                        records.push(rec);
                    }
                    if f.dead() {
                        // EftFollow_Term.
                        w.chars[side].flags.set_held(0xA8);
                        self.follows[side][slot].remove(k);
                    } else {
                        k += 1;
                    }
                }
                let mut k = 0;
                while k < self.rush_shots[side][slot].len() {
                    let r = &mut self.rush_shots[side][slot][k];
                    let (rec, stop) = r.update(w, fx, self.events[side], self.cur_slot[side]);
                    if let Some(rec) = rec
                        && records.len() < max
                    {
                        records.push(rec);
                    }
                    if r.dead() {
                        // EftRushShot_Term (the stage tint is not ported).
                        if r.src().def.kind != 0 {
                            w.chars[side].flags.set_held(0xA8);
                        }
                        self.rush_shots[side][slot].remove(k);
                    } else {
                        k += 1;
                    }
                    if stop {
                        self.request_stop(side);
                    }
                }
                let mut k = 0;
                while k < self.obj_techs[side][slot].len() {
                    let o = &mut self.obj_techs[side][slot][k];
                    if let Some(rec) = o.update(w, fx, self.events[side], self.cur_slot[side])
                        && records.len() < max
                    {
                        records.push(rec);
                    }
                    if o.dead() {
                        // EftObjTech_Term.
                        if o.src().def.kind != 0 {
                            w.chars[side].flags.set_held(0xA8);
                        }
                        self.obj_techs[side][slot].remove(k);
                    } else {
                        k += 1;
                    }
                }
                let mut k = 0;
                while k < self.rings[side][slot].len() {
                    let r = &mut self.rings[side][slot][k];
                    let rec = r.update(
                        w,
                        fx,
                        self.events[side],
                        self.cur_slot[side],
                        &mut self.requests[side],
                        &mut self.shots,
                        next_id,
                        &mut self.rand,
                    );
                    if let Some(rec) = rec
                        && records.len() < max
                    {
                        records.push(rec);
                    }
                    if r.dead() {
                        // EftRingShot_Term.
                        if !r.done() {
                            w.chars[side].flags.set_held(0xA8);
                        }
                        if r.src().def.kind != 0 {
                            w.chars[side].flags.set_held(0xA9);
                        }
                        self.rings[side][slot].remove(k);
                    } else {
                        k += 1;
                    }
                }
                let mut k = 0;
                while k < self.multis[side][slot].len() {
                    let m = &mut self.multis[side][slot][k];
                    m.update(
                        w,
                        fx,
                        self.events[side],
                        self.cur_slot[side],
                        &mut self.discs,
                        next_id,
                    );
                    if m.dead() {
                        if m.src().def.kind != 0 {
                            w.chars[side].flags.set_held(0xA8);
                        }
                        self.multis[side][slot].remove(k);
                    } else {
                        k += 1;
                    }
                }
                let mut k = 0;
                while k < self.volleys[side][slot].len() {
                    let v = &mut self.volleys[side][slot][k];
                    v.update(
                        w,
                        fx,
                        self.events[side],
                        self.cur_slot[side],
                        &mut self.requests[side],
                        &mut self.shots,
                        next_id,
                        &mut self.rand,
                    );
                    if v.dead() {
                        if v.src().def.kind != 0 {
                            w.chars[side].flags.set_held(0xA8);
                        }
                        self.volleys[side][slot].remove(k);
                    } else {
                        k += 1;
                    }
                }
            }
        }
    }

    /// The reset of a fighter's tasks (`BtlScene_Reset`, the owner tag 0x800 / 0x1000): its items die with their
    /// term callbacks' flags (0xA8 for a technique's, a ring shot's 0xA8 when not yet given and 0xA9), its blast
    /// objects and discs are gone; the scene's generator restarts at 0.
    pub(crate) fn reset_side(&mut self, chars: &mut [Chr; 2], fx: &mut Modules, side: usize) {
        let held = &mut chars[side].flags;
        for slot in 0..5 {
            for mut b in self.beams[side][slot].drain(..) {
                // EftBlast_Reset / EftPropShot_Reset.
                if let Some(st) = &mut b.emit {
                    st.kill_all(fx);
                }
                if b.src.def.kind != 0 && !(b.prop && b.flags & 0x800 != 0) {
                    held.set_held(0xA8);
                }
            }
            let mut kinds = Vec::new();
            for mut v in self.volleys[side][slot].drain(..) {
                v.kill_parts(fx);
                kinds.push(v.src().def.kind);
            }
            for mut s in self.sweeps[side][slot].drain(..) {
                s.kill_parts(fx);
                kinds.push(s.src().def.kind);
            }
            for mut m in self.multis[side][slot].drain(..) {
                m.kill_parts(fx);
                kinds.push(m.src().def.kind);
            }
            if kinds.iter().any(|&k| k != 0) {
                held.set_held(0xA8);
            }
            for mut f in self.follows[side][slot].drain(..) {
                f.kill_parts(fx);
                held.set_held(0xA8);
            }
            for mut r in self.rush_shots[side][slot].drain(..) {
                r.kill_parts(fx);
                if r.src().def.kind != 0 {
                    held.set_held(0xA8);
                }
            }
            for mut o in self.obj_techs[side][slot].drain(..) {
                o.kill_parts(fx);
                if o.src().def.kind != 0 {
                    held.set_held(0xA8);
                }
            }
            for mut r in self.rings[side][slot].drain(..) {
                r.kill_parts(fx);
                if !r.done() {
                    held.set_held(0xA8);
                }
                if r.src().def.kind != 0 {
                    held.set_held(0xA9);
                }
            }
        }
        self.shots.reset_side(fx, side);
        self.discs.reset_side(fx, side);
        self.rand = 0;
    }

    /// Layer 3's discs (`EftDisc_Update`), after the ki blasts.
    pub(crate) fn update_discs(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        records: &mut Vec<HitRec>,
        max: usize,
    ) {
        self.discs.update(w, fx, self.events, records, max);
    }

    /// Layer 3's blast objects (`EftBlastObj_Update`), after the ki blasts.
    pub(crate) fn update_shots(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        records: &mut Vec<HitRec>,
        max: usize,
    ) {
        self.shots.cur_slot = self.cur_slot;
        self.shots.update(w, fx, self.events, records, max);
    }

    /// Layer 3's post-update of the blast objects.
    pub(crate) fn post_update_shots(&mut self, w: &World, fx: &mut Modules) {
        self.shots.post_update(w, fx);
    }

    /// `EftTechEvtTask_UpdateNormal` of a technique: START runs the fire timer, FIRE the end timer, the motion's
    /// END ends it, as does leaving the technique or an interrupting motion (ABORT); END again once the end
    /// timer has run out. Returns whether the timeline dies.
    fn update_normal(&mut self, t: &mut Timeline, chars: &[Chr; 2], env: &Env) -> bool {
        let chr = &chars[t.side];
        let e = &mut self.events[t.side];
        t.frame += 1;
        if chr.anim_test_attr(0x200) {
            *e |= EV_START;
            t.flags |= 2;
        }
        if chr.anim_test_attr(0x400) {
            *e |= EV_FIRE;
            t.flags |= if t.kind != 0 { 4 } else { 0x20 };
        }
        if chr.anim_test_attr(0x800) {
            *e |= EV_END;
            return true;
        }
        // Kind 0 (the skills, with their frame tables) is not ported.
        if !in_technique(chr) || env.anim_flags(chr.anim.cur) & 0x10 != 0 {
            *e |= EV_ABORT;
            return true;
        }
        *e |= extra_events(chr);
        if t.end_timer <= 0 {
            *e |= EV_END;
        }
        false
    }

    /// `EftTechEvtTask_UpdateRush`: every fighter's own animation gives its events; ABORT when the attacker
    /// leaves the technique or, once connected (flag 0xA1), the rush sequence; its motion's END ends it.
    fn update_rush(&mut self, t: &mut Timeline, chars: &[Chr; 2]) -> bool {
        let chr = &chars[t.side];
        if chr.flags.test(0xA1) {
            t.flags |= 8;
        }
        if t.flags & 8 != 0 {
            t.frame += 1;
        }
        if !in_technique(chr) || (t.flags & 8 != 0 && !in_rush_sequence(chr)) {
            self.events[t.side] |= EV_ABORT;
            return true;
        }
        if chr.anim_test_attr(0x800) {
            self.events[t.side] |= EV_END;
            return true;
        }
        for (i, c) in chars.iter().enumerate() {
            let e = &mut self.events[i];
            if c.anim_test_attr(0x200) {
                *e |= EV_START;
                t.flags |= 2;
            }
            if c.anim_test_attr(0x400) {
                *e |= EV_FIRE;
                t.flags |= 4;
            }
            *e |= extra_events(c);
        }
        false
    }

    /// Layer 1 of `BtlScene_PostUpdate`: every item reads what the passes reported.
    pub(crate) fn post_update(&mut self, w: &World, fx: &mut Modules) {
        for side in 0..2 {
            for slot in 0..5 {
                for beam in &mut self.beams[side][slot] {
                    beam.post_update(w, fx);
                }
                for v in &mut self.volleys[side][slot] {
                    v.post_update(w, fx, &self.shots);
                }
                for s in &mut self.sweeps[side][slot] {
                    s.post_update(w, fx);
                }
                for f in &mut self.follows[side][slot] {
                    f.post_update(w, fx);
                }
                for r in &mut self.rush_shots[side][slot] {
                    r.post_update(w, fx);
                }
                let mut stop = false;
                for o in &mut self.obj_techs[side][slot] {
                    stop |= o.post_update(w, fx);
                }
                if stop {
                    // EftTechEvt_RequestStop.
                    self.request_stop(side);
                }
                for r in &mut self.rings[side][slot] {
                    r.post_update(w, fx, &self.shots);
                }
                for m in &mut self.multis[side][slot] {
                    m.post_update(w, fx);
                }
            }
        }
    }

    /// `EftTechEvtTask_Draw` (the first view of the frame draws): the requests, then the timers of the timelines
    /// that are not rushes, while the fighter is not stopped; the fire timer running out raises flag 0xA7.
    pub(crate) fn draw(&mut self, w: &mut World) {
        for t in &mut self.timelines {
            if t.flags & 0x10 != 0 {
                continue;
            }
            t.flags |= 0x10;
            if t.is_rush(w.chars) || stopped(w, t.side) {
                continue;
            }
            let req = &mut self.requests[t.side];
            if *req & REQ_STOP != 0 {
                continue;
            }
            if *req & REQ_RESTART != 0 {
                t.end_timer = t.end_time;
                *req &= !REQ_RESTART;
            }
            if *req & REQ_EXPIRE != 0 {
                t.end_timer = 0;
                *req &= !REQ_EXPIRE;
            }
            if t.flags & 2 != 0 {
                t.fire_timer -= 1;
                if t.fire_timer <= 0 {
                    w.chars[t.side].flags.set_held(0xA7);
                    t.flags &= !2;
                }
            }
            if t.flags & 4 != 0 {
                t.end_timer -= 1;
                if t.end_timer <= 0 {
                    t.end_timer = 0;
                    t.flags &= !4;
                }
            }
        }
    }

    /// The live beams of a fighter (head, tail), for tests.
    #[cfg(test)]
    pub(crate) fn beams_of(&self, side: usize) -> Vec<([f32; 3], [f32; 3])> {
        self.beams[side]
            .iter()
            .flatten()
            .filter(|b| b.flags & 0x10 != 0)
            .map(|b| (b.head, b.tail))
            .collect()
    }
}

/// The extra events of the animation: 0x1000 / 0x2000 / 0x4000 and 0x200000 / 0x400000 / 0x800000.
fn extra_events(chr: &Chr) -> u32 {
    [
        (0x1000, 0x10),
        (0x2000, 0x20),
        (0x4000, 0x40),
        (0x20_0000, 0x80),
        (0x40_0000, 0x100),
        (0x80_0000, 0x200),
    ]
    .iter()
    .filter(|&&(a, _)| chr.anim_test_attr(a))
    .fold(0, |e, &(_, b)| e | b)
}

// ---- the fighters' answers to a technique record (btl_hit_reaction.c) ----------------------------------------

/// `BtlUtil_RoundUp`.
fn round_up(n: i32, unit: i32) -> i32 {
    let r = n % unit;
    if r != 0 { n + unit - r } else { n }
}

/// `BtlParam_GetUnkC7Scale` (no abilities).
fn unk_c7_scale(data: &SideData) -> f32 {
    f32::from(data.tables.params.u8_at(0xC7)) * 0.01
}

/// `BtlSuper_GetDamage` of slot 2..4: of one hit (or all with `total`), plain or guarded; times curve row 9
/// (slots 2, 3) or 10 (slot 4) at level 0, then the modifiers, each rounded up to 10: flag 0xA4 3 / 10, the
/// powered-up state + 1 / 5 (not slot 4), held flag 0x9C + 1 / 10, flag 0x98 + 3 / 10 with technique flag 0x8000,
/// the charge, and the cases of techniques 0x268, 0x2CD and 0x2D5. The abilities are not ported.
pub(crate) fn tech_damage(
    atk: &Chr,
    data: &SideData,
    opp_data: &SideData,
    env: &Env,
    slot: i32,
    guard: bool,
    total: bool,
) -> i32 {
    let t = data.tables.tech();
    let flags = t.flags(slot);
    let raw = if guard {
        t.guard_damage(slot)
    } else {
        t.damage(slot)
    };
    let row = if slot == 4 { 10 } else { 9 };
    let mut d = (raw as f32 * env.curve_base(row)) as i32;
    if !total {
        d /= t.hit_count(slot);
    }
    d = round_up(d, 10);
    if atk.flags.test(0xA4) {
        d = round_up(d * 3 / 10, 10);
    }
    if atk.flags.test(6) && slot != 4 {
        d = round_up(d + d / 5, 10);
    }
    if atk.flags.test(0x9C) {
        d = round_up(d + d / 10, 10);
    }
    if flags & 0x8000 != 0 && atk.flags.test(0x98) {
        d = round_up(d + d * 3 / 10, 10);
    }
    d = (d as f32 + d as f32 * (atk.charge * (t.charge_scale(slot) - 1.0))) as i32;
    match t.id(slot) {
        0x268 => {
            match atk.skill_count3 {
                1 => d += d / 2,
                2 => d += d * 13 / 10,
                3 => d += d * 25 / 10,
                _ => {}
            }
            d = round_up(d, 10);
        }
        0x2CD => d = round_up(d + d * atk.boost_stock / 5, 10),
        0x2D5 => d = round_up((d as f32 * unk_c7_scale(opp_data)) as i32, 10),
        _ => {}
    }
    d
}

/// `BtlColl_GuardRush`: a fighter in a guard motion facing the technique takes the guarded damage (a catch or
/// throw technique is shrugged off by a size class 4 fighter, and only guarded from behind, kind 6); the push on
/// the last hit, the recoil on the attacker. Flag 0x46 lets it through; technique flag 1 (unblockable), and a full
/// charge of a charge type with flag 2, are only guarded by kind 6. The shake, vibration, sound and events are not
/// ported.
pub(crate) fn guard_rush(w: &mut World, target: usize, rec: &HitRec) -> bool {
    let Some(src) = rec.tech else {
        return false;
    };
    let (side, slot) = (src.side, src.slot);
    let t = w.data[side].tables.tech();
    if w.chars[target].flags.test(0x46) {
        return false;
    }
    let kind = guard_kind(&w.chars[target]);
    let alt = rec.flags & 0x10 != 0;
    let mut weak = false;
    if matches!(t.react(slot), 0x1D | 0x1E | 0x22) {
        if w.data[target].tables.params.size_class() == 4 && t.flags(slot) & 0x800_0000 == 0 {
            return true;
        }
        weak = true;
    }
    if t.id(slot) == 0x2D5 && unk_c7_scale(w.data[target]) < 0.001 {
        return true;
    }
    let (yaw, dir) = hit_dir(w, target, rec);
    if kind != 6 && is_facing(&w.chars[target], yaw) {
        return false;
    }
    if t.flags(slot) & 1 != 0 && (weak || kind != 6) {
        return false;
    }
    if t.kind(slot) & 2 != 0
        && w.chars[side].charge >= 0.9999
        && t.flags(slot) & 2 != 0
        && kind != 6
    {
        return false;
    }
    if kind == 0 || kind == 5 {
        return false;
    }
    let env = w.env;
    let dmg = tech_damage(
        &w.chars[side],
        w.data[side],
        w.data[target],
        env,
        slot,
        true,
        false,
    );
    let chr = &mut w.chars[target];
    match t.guard_kind(slot) {
        0 => chr.flags.set(0x66),
        1 => chr.flags.set(0x67),
        _ => {}
    }
    chr.damage(env, w.data[target], dmg, 0xC);
    if alt {
        chr.set_impulse_dir(dir, t.guard_push(slot) * 10.0 * kmh(1.0));
    }
    recoil(w, side, slot);
    true
}

/// The technique's recoil on its user (`BtlSuper_GetRecoilDamage`): flag 0x95 when it takes health.
fn recoil(w: &mut World, side: usize, slot: i32) {
    let n = w.data[side].tables.tech().recoil(slot);
    if n > 0 {
        let atk = &mut w.chars[side];
        if atk.damage(w.env, w.data[side], n, 0x42B) != 0 {
            atk.flags.set(0x95);
        }
    }
}

/// The damage flags of a technique hit by slot.
fn hit_damage_flags(slot: i32) -> u32 {
    match slot {
        2 => 0x80_0100,
        3 => 0x100_0100,
        4 => 0x200_0100,
        _ => 0x100,
    }
}

/// `BtlColl_HitByRush`: a technique hits; one with hit direction kind 0 / 2 only from in front of its user. The
/// reaction by the victim's state (0x800) and the last hit, 0 meaning 0xF and 1 nothing; then no flinch, a catch
/// (0x1D / 0x1E, `BtlColl_StartRushCatch`), the rush sequence (0x22, `BtlColl_StartThrow`), or the hit. A catch or
/// a sequence that cannot start lets the record pass. The battle events are not ported.
pub(crate) fn hit_by_rush(w: &mut World, target: usize, rec: &HitRec) -> bool {
    let Some(src) = rec.tech else {
        return false;
    };
    let (side, slot) = (src.side, src.slot);
    let t = w.data[side].tables.tech();
    if w.chars[target].flags.test(0x46) {
        return false;
    }
    let (yaw, dir) = hit_dir(w, target, rec);
    let front = is_facing(&w.chars[target], yaw);
    if matches!(t.hit_dir_kind(slot), 0 | 2)
        && opp_yaw_from_facing(&w.chars[side], &w.chars[target]).abs() > FRAC_PI_2
    {
        return false;
    }
    let alt = rec.flags & 0x10 != 0;
    let s800 = w.env.anim_flags(w.chars[target].anim.cur) & 0x800 != 0;
    let mut react = match (s800, alt) {
        (true, true) => t.react_alt_s800(slot),
        (true, false) => t.react_s800(slot),
        (false, true) => t.react_alt(slot),
        (false, false) => t.react(slot),
    };
    if react == 0 {
        react = 0xF;
    }
    if react == 1 {
        return false;
    }
    let env = w.env;
    if matches!(react, 0x1D | 0x1E | 0x22) {
        let [a, b] = &mut *w.chars;
        let (atk, def) = if side == 0 { (a, b) } else { (b, a) };
        return if react == 0x22 {
            crate::btl_hit::start_throw(atk, def, w.data[side], env, slot, t.flags(slot), w.frame)
        } else {
            crate::btl_hit::start_rush_catch(def, atk, w.data[side], env, slot, react, yaw)
        };
    }
    let dmg = tech_damage(
        &w.chars[side],
        w.data[side],
        w.data[target],
        env,
        slot,
        false,
        false,
    );
    if react == REACT_NOFLINCH || w.chars[target].no_flinch > 0 {
        // BtlColl_ApplyRushNoFlinch.
        let chr = &mut w.chars[target];
        chr.react.back = front;
        chr.react.yaw = yaw;
        chr.react.reaction = REACT_NOFLINCH;
        chr.set_fx(0x1D);
        chr.damage(env, w.data[target], dmg, hit_damage_flags(slot));
        recoil(w, side, slot);
        return true;
    }
    // BtlColl_ApplyRushHit (the stage timer concerns stages 4 / 27).
    let chr = &mut w.chars[target];
    chr.react.reaction = react;
    chr.react.scale = 0.0;
    chr.react.slot = slot;
    chr.flags.set(0x94);
    chr.react.turn_yaw = if front { yaw } else { wrap_angle(yaw + PI) };
    chr.react.turn_pitch = 0.0;
    chr.react.yaw = yaw;
    chr.react.back = front;
    if alt {
        chr.set_impulse_dir(dir, t.push(slot) * 10.0 * kmh(1.0));
    }
    chr.set_fx(0x1D);
    chr.damage(env, w.data[target], dmg, hit_damage_flags(slot));
    chr.flags.set_held(0x1D);
    chr.flags.set_held(0x1E);
    chr.dodge_window = 0;
    if slot == 4 {
        if !chr.flags.test(0x13) && react != 0x22 {
            chr.flags.clear(5);
        }
    } else if !chr.flags.test(0x93) {
        chr.flags.set_held(5);
    }
    match t.kind(slot) {
        7 => w.chars[side].flags.set(0x73),
        8 => recoil(w, side, slot),
        _ => {}
    }
    true
}

/// `BtlColl_TryDodge` for a technique record of slot 2..4: flag 0x46 lets it pass; else a free fighter in a
/// guardable motion, before the record's first hit, evades it in its dodge window (free) or with a counted dodge
/// (flags 0x56 / 0x57 too), sideways by the stick (or the frame), raising flag 0x80.
pub(crate) fn try_dodge_tech(w: &mut World, target: usize, hit_count: i32) -> bool {
    let frame = w.frame;
    let anim = w.env.anim_flags(w.chars[target].anim.cur);
    let chr = &mut w.chars[target];
    if chr.flags.test(0x46) {
        return true;
    }
    if !chr.is_free() || anim & 0xB1 == 0 || hit_count != 0 {
        return false;
    }
    let (mut ok, mut used) = (false, false);
    if chr.dodge_window > 0 {
        ok = true;
    } else {
        if chr.dodges > 0 {
            chr.dodges -= 1;
            ok = true;
            used = true;
        }
        if chr.dodges_b > 0 {
            chr.dodges_b -= 1;
            ok = true;
            used = true;
        }
        if chr.flags.test(0x56) {
            ok = true;
            used = true;
        }
        if chr.flags.test(0x57) {
            ok = true;
        }
    }
    if !ok {
        return false;
    }
    let mut dir = frame % 2;
    if chr.input.is_held(0x40) {
        dir = 0;
    }
    if chr.input.is_held(0x80) {
        dir = 1;
    }
    chr.flags.set(if dir == 0 { 0x74 } else { 0x75 });
    // Ability 0x22 (blast gauge on a free dodge) is not ported.
    chr.flags.set_held(0x80);
    if used {
        chr.flags.set(if chr.skill_slot == 0 { 0xDE } else { 0xDF });
    }
    true
}

/// `BtlColl_TryDodge`'s technique guard: a fighter in a technique of kind 8 whose clash power reaches the
/// record's level lets it pass.
pub(crate) fn tech_outclasses(chr: &Chr, data: &SideData, level: i32) -> bool {
    let cur = chr.act.current;
    if !is_technique_id(cur) {
        return false;
    }
    let slot = id_class(cur);
    let t = data.tables.tech();
    (2..5).contains(&slot) && t.kind(slot) == 8 && t.clash_power(slot) >= level
}

/// `BtlColl_TryAbsorb`'s amount for a technique record: technique flag 4 only, its ki cost over its shots.
pub(crate) fn absorb_amount(w: &World, rec: &HitRec) -> Option<i32> {
    let src = rec.tech?;
    let t = w.data[src.side].tables.tech();
    if !(2..5).contains(&src.slot) || t.flags(src.slot) & 4 == 0 {
        return None;
    }
    let mut amount = t.ki(src.slot);
    let n = t.shots(src.slot) * t.hits_b(src.slot);
    if n >= 2 {
        amount /= n;
    }
    Some(amount)
}
