//! Ki blasts: the shot an animation event fires (`BtlFx_FireKiBlast`, src/battle/btl_char_fx_1.c), the blast task
//! (`EftKiBlast_*`, src/battle/eft_struggle.c), the hit records of the effect scene (`EftHit_*`,
//! src/battle/eft_core.c), their detection against each other, the stage and the fighters (`EftDet_*`,
//! src/battle/eft_detect.c, with the primitives of src/battle/col_primitives.c), the fighters' answers
//! (`BtlColl_*`, src/battle/btl_hit_reaction.c) and the record readers (`BtlKiBlast_*`, src/battle/btl_tech.c).
//!
//! The frame: the fighters fire during their camera pass; then `BtlScene_Update` (the record list emptied, every
//! blast task updated in creation order, `EftHit_ClampToStage`), `EftDet_Update` (clash, stage, fighter passes) and
//! `BtlScene_PostUpdate` (every task reads what the passes reported).
//!
//! Not ported: blast types 2 (ki object), 3 (ki bomb), 4 / 5 (discs) fire nothing, nor does a character whose kind
//! 1 / 2 list belongs to those managers; technique records and beam struggles; breaking stage objects (a blast
//! stops at every object); the particles, impact effects, sounds, voices, camera shake, pad vibration and battle
//! events; abilities.

use std::f32::consts::PI;

use bt3_data::chara_data::KiBlastRecord;
use bt3_data::eft_pack::EmitSet;
use bt3_data::stage_col::{ColBox, ColPoly, POLY_NO_SOLID};

use crate::btl_chr::{Chr, Env, SideData, add, dot, kmh, length, opp_delta, scale3, sub};
use crate::btl_hit::{guard_kind, raise_first_clash};
use crate::btl_input::wrap_angle;
use crate::btl_obj::mask_to_node;
use crate::eft::{EmitState, FxWorld, Modules, Nodes, SetRef, SpawnCtx, spawn};
use crate::rng::LibcRandom;

/// `EFT_HIT_MAX`.
const HIT_MAX: usize = 64;
/// `EftKiBlastMgr_Init`: tasks per list (kind 1, kind 2).
const LIST_SIZE: [usize; 2] = [20, 10];
/// The animation event that fires (`BtlAnim_TestAttr(chr, 4)`).
const EV_FIRE: u64 = 4;

// EFT_TASK_* / EFT_R_HIT_*: the task's tag bits the passes report through.
const T_HIT_CHAR: u32 = 0x1;
const T_GUARDED: u32 = 0x2;
const T_HIT_STAGE: u32 = 0x4;
const T_LOST_CLASH: u32 = 0x8;
const T_OUT: u32 = 0x10;
const T_ABSORBED: u32 = 0x20;
const T_DEFLECTED: u32 = 0x40;
const T_REFLECTED: u32 = 0x80;
const T_KEEP: u32 = 0x1_0000;
const T_END: u32 = 0x3F;
const T_TURN: u32 = 0xC0;

// EFT_KIBLAST_*.
const W_STARTED: u32 = 0x1;
const W_MOVED: u32 = 0x2;
const W_HOMING: u32 = 0x4;
const W_DEFLECTED: u32 = 0x8;
const W_ENDING: u32 = 0x10;
const W_ENDED: u32 = 0x20;
const W_KILL: u32 = 0x40;
const W_TYPED: u32 = 0x80;
const W_STOPPED: u32 = 0x100;
const W_TURNED: u32 = 0x200;
const W_WIDTH2: u32 = 0x800;

// EFT_DET_HIT_*.
const D_STAGE: u32 = 1;
const D_FIGHTER: u32 = 2;
const D_CLASH: u32 = 4;

/// `BTL_REACT_NOFLINCH`.
const REACT_NOFLINCH: i32 = 2;

/// `EftRArg` / `FxHitArg2`: what a shot is fired with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Arg {
    /// Launch direction (`pos` of the request: the aim, turned by the volley spread).
    pub dir: [f32; 3],
    /// `srcId`: who fired it; never changes.
    pub src: usize,
    /// `objId`: the owner, the fighter it cannot hit; flips when deflected or reflected.
    pub owner: usize,
    pub node: u32,
    /// `kind` / `level` of the request: `BtlKiBlast_GetCurrentKind`, 0..12.
    pub kind: i32,
    pub kind_type: i8,
    pub unk1b: u8,
    pub life: i32,
    pub speed: f32,
    pub turn: f32,
    pub radius: f32,
    pub unk2c: f32,
    /// `size`: 0..3, the record's clash level.
    pub level: i32,
    pub can_hit: bool,
    pub unk3c: i32,
    /// `unk41`: the motion's fire events (`BtlObjAnim_QueryEvent(obj, 4, 0, 3)`), the hit-time damage divisor.
    pub hits: i32,
}

/// A sphere of a record's shape (`EftHitSphere`).
#[derive(Clone, Copy, Debug, Default)]
struct Sphere {
    pos: [f32; 3],
    radius: f32,
}

/// The swept shape of a record and what the passes found (`EftDetShape`, type 0: two spheres).
#[derive(Clone, Copy, Debug)]
struct Shape {
    /// This frame's sphere (`a`) and last frame's (`b`).
    a: Sphere,
    b: Sphere,
    /// `EftHit_GetScale`: 0.1 for a blast (the stage test's radius factor).
    scale: f32,
    bounds: ColBox,
    delta: [f32; 3],
    dir: [f32; 3],
    zone: usize,
    hit_flags: u32,
    stage_dist: f32,
    dist: f32,
    hit_pos: [f32; 3],
    contact: [f32; 3],
}

/// One hit record (`EftHitRec`) of a ki blast.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HitRec {
    /// `objId`: the owner (`atk->objId`).
    pub owner: usize,
    /// `level`: `atk->level`, the clash level.
    pub level: i32,
    pub pos: [f32; 3],
    pub prev: [f32; 3],
    /// `vel`: the blast's unit direction.
    pub vel: [f32; 3],
    /// `turned`: deflected or reflected at least once.
    pub turned: bool,
    /// The blast's argument block (`atk`).
    pub atk: Arg,
    /// `seenByAi`.
    pub seen_by_ai: bool,
    task: u32,
    shape: Shape,
}

/// An `EftKiBlast` task: its tag word and position (the task header) and its work.
#[derive(Clone, Debug)]
struct Task {
    id: u32,
    /// `task->flags`: what the passes reported (`T_*`).
    hit: u32,
    /// `task->pos`: where it was reported.
    hit_pos: [f32; 3],
    flags: u32,
    pos: [f32; 3],
    prev: [f32; 3],
    dir: [f32; 3],
    arg: Arg,
    life: i32,
    radius: f32,
    /// `EftEmit_GetTrailWidth`: the set header's first width (the blast never steps that animation).
    trail_width: f32,
    end_timer: f32,
    end_frames: f32,
    /// The blast list (kind 1 or 2) whose emitter set draws it.
    list: usize,
    /// The emitter set's instance state, its node slots and the phases happening this frame (`mask`).
    state: EmitState,
    nodes: Nodes,
    mask: u32,
}

/// The ki blast part of the effect scene: per fighter the kind 1 and kind 2 lists, and this frame's records.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scene {
    lists: [[Vec<Task>; 2]; 2],
    next_id: u32,
    /// `gEftHitList`: emptied by `BtlScene_Update`, read until the next one.
    pub records: Vec<HitRec>,
    /// The particle modules the blasts' emitters start.
    pub fx: Modules,
}

/// What a scene pass needs of the battle.
pub(crate) struct World<'a, 'd> {
    pub chars: &'a mut [Chr; 2],
    pub data: [&'a SideData<'d>; 2],
    pub env: &'a Env<'d>,
    /// `gBtlChars->randState`.
    pub rand: &'a mut u32,
    /// `BtlChars_IsTimeStopped`.
    pub time_stopped: bool,
    /// libc `rand()` (the effects' random numbers).
    pub libc: &'a mut LibcRandom,
    /// The VU0 R register (`Rand_FloatRange`).
    pub vu0: &'a mut crate::rng::Vu0Random,
    /// `BATTLE_FLAG_PAUSE`: `BtlScene_IsTimeStopped` and every effect's "stopped" test hold.
    pub paused: bool,
}

// ---- vectors ------------------------------------------------------------------------------------------------------

/// `Vec3_Normalize`: a zero vector gives zero.
fn norm(v: [f32; 3]) -> [f32; 3] {
    let len = length(v);
    if len == 0.0 {
        [0.0; 3]
    } else {
        scale3(v, 1.0 / len)
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `Vec3_RotateAxis` / `EftMath_RotateAboutAxis` (Rodrigues).
fn rotate_axis(v: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    let k = dot(axis, v) * (1.0 - c);
    add(
        add(scale3(v, c), scale3(cross(axis, v), s)),
        scale3(axis, k),
    )
}

/// `Vec3_RotateY`.
fn rotate_y(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0] * c + v[2] * s, v[1], v[2] * c - v[0] * s]
}

/// `ColBounds_OfCapsule`.
fn capsule_bounds(a: [f32; 3], b: [f32; 3], r: f32) -> ColBox {
    let mut out = ColBox {
        min: [f32::MAX; 3],
        max: [-f32::MAX; 3],
    };
    for i in 0..3 {
        out.min[i] = (a[i] - r).min(b[i] - r);
        out.max[i] = (a[i] + r).max(b[i] + r);
    }
    out
}

/// `BtlChar_RandF`: the fighters' generator, kept while time is stopped.
pub(crate) fn rand_f(state: &mut u32, time_stopped: bool) -> f32 {
    let next = ((u64::from(*state) * 714_025 + 4096) % 150_889) as u32;
    if !time_stopped {
        *state = next;
    }
    next as f32 / 150_889.0
}

// ---- the record readers (btl_tech.c) -----------------------------------------------------------------------------

/// `BtlKiBlast_GetCurrentKind`: the kind the fighter is firing, from its action, motion and charge; -1 if none.
pub(crate) fn current_kind(me: &Chr) -> i32 {
    let (lo, hi) = (0.3, 0.9);
    let charged = |a: i32| {
        if me.blast_charge < lo {
            a
        } else if me.blast_charge < hi {
            a + 1
        } else {
            a + 2
        }
    };
    if me.act.current == 0x95 {
        return 12;
    }
    match me.anim.cur {
        0x71 | 0x72 => 0,
        0x7B | 0x7C => 1,
        0x7D => charged(1),
        0x77..=0x79 => 4,
        0x84 | 0x85 | 0x87 | 0x88 | 0x8A | 0x8B => 5,
        0x86 | 0x89 | 0x8C => charged(5),
        0x7A => 8,
        0x8D | 0x8E => 9,
        0x8F => charged(9),
        _ => -1,
    }
}

/// `BtlKiBlast_Get`: kind 0 when out of range.
fn record<'a>(data: &'a SideData, kind: i32) -> &'a KiBlastRecord {
    let k = usize::try_from(kind)
        .ok()
        .filter(|&k| k < data.tables.ki_blasts.len())
        .unwrap_or(0);
    &data.tables.ki_blasts[k]
}

/// `BtlKiBlast_GetCurrent`.
fn current<'a>(me: &Chr, data: &'a SideData) -> &'a KiBlastRecord {
    record(data, current_kind(me).max(0))
}

/// `BtlKiBlast_GetFlagsOf`: bits 0 and 1 dropped while the body is changed.
fn flags_of(me: &Chr, rec: &KiBlastRecord) -> u16 {
    if me.gauge.body_changed {
        rec.flags() & !3
    } else {
        rec.flags()
    }
}

/// `BtlKiBlast_GetKiCostOf` (no abilities).
pub(crate) fn ki_cost_of(data: &SideData, kind: i32) -> i32 {
    record(data, kind).ki()
}

/// `BtlKiBlast_GetKiCost`.
pub(crate) fn ki_cost(me: &Chr, data: &SideData) -> i32 {
    current(me, data).ki()
}

/// `BtlKiBlast_GetHitsOf`.
pub(crate) fn hits_of(data: &SideData, kind: i32) -> i32 {
    i32::from(record(data, kind).hits())
}

/// `BtlKiBlast_GetDamageOf` / `GetGuardDamageOf`: times curve row 6, divided by the hits (2 or more for the plain
/// damage, any for the guarded one).
fn damage_of(rec: &KiBlastRecord, env: &Env, guard: bool) -> i32 {
    let base = if guard {
        rec.guard_damage()
    } else {
        rec.damage()
    };
    let mut d = (base as f32 * env.curve_base(6)) as i32;
    let hits = i32::from(rec.hits());
    if (guard && hits > 0) || hits >= 2 {
        d /= hits;
    }
    d
}

/// `BtlKiBlast_GetPushOf` / `GetGuardPushOf`: 10 km/h per frame over the hits.
fn push_of(rec: &KiBlastRecord, guard: bool) -> f32 {
    let base = if guard { rec.guard_push() } else { rec.push() };
    let mut p = base * 10.0 * kmh(1.0);
    let hits = i32::from(rec.hits());
    if hits > 0 {
        p /= hits as f32;
    }
    p
}

/// `BtlParam_GetBlastLimitA` (no abilities).
pub(crate) fn blast_limit_a(data: &SideData) -> i32 {
    i32::from(data.tables.params.u16_at(0x80) as i16)
}

/// `BtlParam_GetBlastLimitB`.
pub(crate) fn blast_limit_b(data: &SideData) -> i32 {
    i32::from(data.tables.params.u16_at(0x82) as i16)
}

/// The class of a kind (`BtlMove_CanFireBlast`): 0 for 0, 4, 8, else 1.
fn kind_class(kind: i32) -> usize {
    usize::from(!matches!(kind, 0 | 4 | 8))
}

/// `BtlMove_CanFireBlast`: fewer live blasts of the class than the character's limit.
pub(crate) fn can_fire(me: &Chr, data: &SideData, class: usize) -> bool {
    let limit = if class == 0 {
        blast_limit_a(data)
    } else {
        blast_limit_b(data)
    };
    me.live_blasts[class] < limit
}

// ---- firing (btl_char_fx_1.c) -------------------------------------------------------------------------------------

/// `BtlCharApi_HasKiBlastType2` / `3`.
fn has_type(data: &SideData, t: i8) -> bool {
    data.tables.ki_blasts.iter().any(|r| r.kind_type() == t)
}

/// The set header of the fighter's kind 1 / 2 list, `None` when the fighter has no such list of ki blasts
/// (`EftChar_Init`: pack bit `kind`, replaced by the ki object / ki bomb managers).
fn list_head<'a>(data: &SideData<'a>, list: usize) -> Option<&'a EmitSet> {
    let pack = data.effect?;
    if pack.kind_bits & (1 << (list + 1)) == 0 {
        return None;
    }
    if (list == 0 && has_type(data, 2)) || (list == 1 && has_type(data, 3)) {
        return None;
    }
    pack.blast_sets[list].as_ref()
}

/// `BtlObj_GetNodeSide`.
fn node_side(node: u32) -> i32 {
    match node {
        0..=7 => 2,
        8..=0xB => 1,
        0xC..=0xF => 0,
        0x10..=0x11 => 2,
        0x12..=0x1F => 1,
        0x20..=0x2D => 0,
        0x2E..=0x34 => 2,
        _ => 3,
    }
}

impl Scene {
    /// `BtlFx_FireKiBlast`: animation event 4 launches the record's hits along the aim from the event's node.
    pub(crate) fn fire(
        &mut self,
        me: &mut Chr,
        opp: &Chr,
        data: &SideData,
        rand: &mut u32,
        time_stopped: bool,
    ) {
        if !me.anim_test_attr(EV_FIRE) {
            return;
        }
        let rec = *current(me, data);
        let spread = rec.spread_mode();
        let node = mask_to_node(me.obj.event_arg(EV_FIRE)) as u32;
        let aim = me.aim_dir;
        let count = i32::from(rec.hits()).max(1);
        let kind = current_kind(me);
        let mut arg = Arg {
            dir: aim,
            src: me.player,
            owner: me.player,
            node,
            kind,
            kind_type: rec.kind_type(),
            unk1b: rec.unk3(),
            life: (rec.time() * 30.0) as i32,
            // BtlKiBlast_ScaleSpeedByAbility: no abilities.
            speed: rec.speed() * 10.0 * kmh(1.0),
            turn: rec.turn() / 30.0 * PI / 180.0,
            radius: rec.radius(),
            unk2c: rec.unk2c(),
            level: if (0..12).contains(&kind) { kind & 3 } else { 0 },
            can_hit: true,
            unk3c: i32::from((flags_of(me, &rec) >> 5) & 1),
            hits: me.obj.query_event(&data.obj, EV_FIRE, 0, 3) & 0xFF,
        };
        if me.gauge.body_changed {
            arg.kind_type = if matches!(kind, 0 | 4 | 8) { 0 } else { 1 };
        }
        for _ in 0..count {
            let mut dir = aim;
            match spread {
                1 => {}
                0 => {
                    let mut r = || rand_f(rand, time_stopped);
                    if !(data.height() * 3.0 < me.height_above_ground()) {
                        dir[0] += (r() - 0.5) * 0.4;
                        dir[1] += (r() - 1.0) * 0.2;
                    } else {
                        dir[0] += (r() - 0.5) * 0.4;
                        dir[1] += (r() - 0.5) * 0.4;
                    }
                    dir[2] += (r() - 0.5) * 0.4;
                }
                2..=6 => {
                    if me.flags.test(5) {
                        let d = (length(opp_delta(me, opp)) * 0.8 / arg.speed * arg.turn * 0.5)
                            .min(0.75)
                            .max(0.2);
                        if spread == 2 {
                            let axis = norm([aim[2], 0.0, -aim[0]]);
                            dir = rotate_axis(dir, axis, d);
                        } else {
                            let mirror = match spread {
                                3 => {
                                    let m = me.flags.test(0x8D);
                                    me.flags.toggle_held(0x8D);
                                    m
                                }
                                4 => true,
                                6 => node_side(node) == 0,
                                _ => false,
                            };
                            dir = rotate_y(dir, if mirror { -d } else { d });
                        }
                    }
                }
                _ => {}
            }
            arg.dir = dir;
            // Types 5 / 4 (discs), 2 (ki object) and 3 (ki bomb) are not ported.
            if !matches!(arg.kind_type, 2..=5) {
                self.launch(me, data, arg);
            }
        }
    }

    /// `EftKiBlast_Fire` and `EftKiBlast_Init`: a task in the owner's kind 1 list (type 0) or kind 2 list, unless the
    /// list is full or missing. The blast starts at the node, its "previous" position at the owner's node 0x11.
    fn launch(&mut self, me: &Chr, data: &SideData, arg: Arg) {
        let list = usize::from(arg.kind_type != 0);
        let Some(head) = list_head(data, list) else {
            return;
        };
        let tasks = &mut self.lists[arg.owner][list];
        if tasks.len() >= LIST_SIZE[list] {
            return;
        }
        let mut arg = arg;
        let mut flags = 0;
        if arg.kind_type != 0 {
            flags |= W_TYPED;
        }
        if head.head.flags & 4 != 0 {
            // EftEmit_GetWidth2 reads the state before EftEmit_InitState: 0.
            arg.speed = 0.0;
            flags |= W_WIDTH2;
        }
        if me.flags.test(5) {
            flags |= W_HOMING;
        }
        let pos = me.obj.node_pos(arg.node);
        let mut nodes = Nodes::default();
        nodes.set_node(1, arg.node);
        nodes.refresh(|n| me.obj.node_pos(n));
        self.next_id += 1;
        tasks.push(Task {
            id: self.next_id,
            hit: 0,
            hit_pos: [0.0; 3],
            flags,
            pos,
            prev: me.obj.node_pos(0x11),
            dir: arg.dir,
            arg,
            life: arg.life,
            radius: arg.radius,
            trail_width: head.head.width[0],
            end_timer: 0.0,
            end_frames: f32::from(head.head.end_frames),
            list,
            state: EmitState::new(head),
            nodes,
            mask: 2,
        });
    }

    /// `BtlScene_Update`, `EftDet_Update` and `BtlScene_PostUpdate` for the ki blasts; then every fighter's live
    /// blast counts for `BtlMove_CanFireBlast`.
    pub(crate) fn update(&mut self, w: &mut World) {
        // EftHit_BeginFrame: the records are kept while time is stopped.
        if !w.paused {
            self.records.clear();
        }
        // BtlScene_IsAnyCharStopped.
        let stopped = w.paused || w.chars.iter().any(Chr::is_frozen);
        for side in 0..2 {
            for list in 0..2 {
                let mut i = 0;
                while i < self.lists[side][list].len() {
                    let task = &mut self.lists[side][list][i];
                    let rec = task_update(task, w, &mut self.fx, stopped);
                    if let Some(rec) = rec
                        && self.records.len() < HIT_MAX
                    {
                        self.records.push(rec);
                    }
                    if task.flags & W_KILL != 0 {
                        self.lists[side][list].remove(i);
                    } else {
                        i += 1;
                    }
                }
            }
        }
        // Layer 3: the particle modules.
        let fxw = FxWorld {
            sets: std::array::from_fn(|s| w.data[s].blast_assets),
            any_frozen: stopped,
        };
        self.fx.update(&fxw, w.libc, w.vu0);
        self.clamp_to_stage(w.env);
        // EftDet_Update: nothing while paused.
        if !w.paused {
            self.prepare(w.env);
            self.clash_pass();
            self.stage_pass(w.env);
            self.fighter_pass(w);
        }
        // BtlScene_PostUpdate: EftHit_UpdateResults has nothing a ki blast reads; the impacts are not ported.
        for side in 0..2 {
            for list in 0..2 {
                for task in &mut self.lists[side][list] {
                    task_post_update(task, w, &mut self.fx);
                }
            }
        }
        for (i, chr) in w.chars.iter_mut().enumerate() {
            chr.live_blasts = [0; 2];
            for rec in self.records.iter().filter(|r| r.atk.src == i) {
                chr.live_blasts[kind_class(rec.atk.kind)] += 1;
            }
        }
    }

    fn task_mut(&mut self, id: u32) -> Option<&mut Task> {
        self.lists
            .iter_mut()
            .flatten()
            .flatten()
            .find(|t| t.id == id)
    }

    /// `EftHit_SetTaskFlag`.
    fn set_task_flag(&mut self, i: usize, bits: u32, pos: [f32; 3]) {
        let id = self.records[i].task;
        if let Some(t) = self.task_mut(id) {
            t.hit |= bits;
            t.hit_pos = pos;
        }
    }

    fn task_flags(&mut self, i: usize) -> u32 {
        let id = self.records[i].task;
        self.task_mut(id).map_or(0, |t| t.hit)
    }

    /// `EftHit_CanHit` for a ki blast record: mode 0 the stage, 1 a fighter, 2 a clash.
    fn can_hit(&mut self, i: usize, mode: u32) -> bool {
        let flags = self.task_flags(i);
        let ok = if mode == 0 {
            flags & (T_KEEP | T_HIT_STAGE) == 0
        } else {
            flags & (T_HIT_CHAR | T_LOST_CLASH) == 0
        };
        ok && flags & T_DEFLECTED == 0
    }

    /// `EftHit_ClampToStage`: a record beyond the stage's inner radius reports "out" once, pulled back onto it.
    fn clamp_to_stage(&mut self, env: &Env) {
        for i in 0..self.records.len() {
            let radius = env.inner_radius();
            let p = self.records[i].pos;
            if radius < p[0].hypot(p[2]) && self.task_flags(i) & T_OUT == 0 {
                let len = p[0].hypot(p[2]);
                let k = env.inner_radius() / len;
                let pos = [p[0] * k, p[1], p[2] * k];
                let id = self.records[i].task;
                if let Some(t) = self.task_mut(id) {
                    t.hit_pos = pos;
                    t.hit |= T_OUT;
                }
                self.records[i].shape.a.pos = pos;
            }
        }
    }

    /// `EftDet_PrepareAll` with `EftDet_PrepareSpheres`.
    fn prepare(&mut self, env: &Env) {
        for rec in &mut self.records {
            let s = &mut rec.shape;
            s.hit_flags = 0;
            s.stage_dist = f32::MAX;
            s.dist = f32::MAX;
            s.bounds = capsule_bounds(s.b.pos, s.a.pos, s.a.radius);
            s.delta = sub(s.a.pos, s.b.pos);
            s.dir = norm(s.delta);
            s.zone = env.col.find_zone_near(s.zone as i32, s.a.pos);
        }
    }

    /// `EftDet_ClashPass` for two ki blasts: the lower level loses (both on a tie); `EftDet_SpheresVsSpheres`.
    fn clash_pass(&mut self) {
        let n = self.records.len();
        for i in 0..n.saturating_sub(1) {
            if !self.can_hit(i, 2) {
                continue;
            }
            for j in i..n {
                if self.records[i].owner == self.records[j].owner || !self.can_hit(j, 2) {
                    continue;
                }
                let (a, b) = (self.records[i].shape, self.records[j].shape);
                let Some((ha, hb)) = spheres_vs_spheres(&a, &b) else {
                    continue;
                };
                self.records[i].shape.hit_pos = ha.0;
                self.records[i].shape.contact = ha.1;
                self.records[j].shape.hit_pos = hb.0;
                self.records[j].shape.contact = hb.1;
                // EftHit_Clash / EftHit_CompareLevel.
                let (la, lb) = (self.records[i].level, self.records[j].level);
                let result = if la > lb {
                    2
                } else if la < lb {
                    1
                } else {
                    3
                };
                self.records[i].shape.hit_flags |= D_CLASH;
                if result & 1 != 0 {
                    let p = self.records[i].shape.hit_pos;
                    self.set_task_flag(i, T_LOST_CLASH, p);
                }
                if result & 2 != 0 {
                    let p = self.records[j].shape.hit_pos;
                    self.set_task_flag(j, T_LOST_CLASH, p);
                }
            }
        }
    }

    /// `EftDet_StagePass` with `EftDet_TestStage`: the swept sphere (a tenth of the radius, at least 0.08) against
    /// the zone's triangles; the nearest contact stops the blast (task flag 4).
    fn stage_pass(&mut self, env: &Env) {
        for i in 0..self.records.len() {
            if !self.can_hit(i, 0) {
                continue;
            }
            let s = &mut self.records[i].shape;
            let r = (s.a.radius * s.scale).max(0.08);
            let sweep = Sweep::new(s.b.pos, s.a.pos, r);
            let bounds = capsule_bounds(s.b.pos, s.a.pos, r);
            let mut best = sweep.length.max(r) + 0.001;
            let mut found = None;
            env.col
                .query_zone(s.zone, &bounds, &mut |poly: &ColPoly, v: [[f32; 3]; 3]| {
                    if poly.flags & POLY_NO_SOLID != 0 {
                        return false;
                    }
                    match sweep.test_tri(&v, poly.nrm) {
                        Some((hit, dist)) if dist < best => {
                            best = dist;
                            let n = [poly.nrm[0], poly.nrm[1], poly.nrm[2]];
                            let contact = if best < 0.01 {
                                hit
                            } else {
                                sub(hit, scale3(n, r))
                            };
                            found = Some((hit, contact));
                            true
                        }
                        _ => false,
                    }
                });
            // Stage objects do not break yet: a blast stops at them as at the static mesh.
            let Some((hit, contact)) = found else {
                continue;
            };
            s.hit_pos = hit;
            s.contact = contact;
            s.hit_flags |= D_STAGE;
            s.stage_dist = length(sub(s.b.pos, hit));
            // EftHit_IsStoppedByHit is 1 for a blast; the contact sound and EftKiBomb_SetContact are not ported.
            self.set_task_flag(i, T_HIT_STAGE, contact);
        }
    }

    /// `EftDet_FighterPass`: each record against the fighter it may hit (owner ^ 1). A contact behind the stage
    /// contact is dropped; then the fighter dodges, deflects, reflects, absorbs, guards or is hit.
    fn fighter_pass(&mut self, w: &mut World) {
        for i in 0..self.records.len() {
            if !self.can_hit(i, 1) {
                continue;
            }
            let target = self.records[i].owner ^ 1;
            let Some((hit_pos, contact, dist)) =
                spheres_vs_fighter(&w.chars[target], &self.records[i].shape)
            else {
                continue;
            };
            let s = &mut self.records[i].shape;
            s.hit_pos = hit_pos;
            s.contact = contact;
            s.dist = dist;
            if s.hit_flags & D_STAGE != 0 && !(s.dist < s.stage_dist) {
                continue;
            }
            s.hit_flags |= D_FIGHTER;
            let rec = self.records[i];
            let contact = rec.shape.contact;
            if try_dodge(w, target, &rec) {
                continue;
            }
            if try_deflect(w, target, &rec) {
                self.set_task_flag(i, T_DEFLECTED, contact);
            } else if try_reflect(w, target, &rec) {
                self.set_task_flag(i, T_REFLECTED, contact);
            } else if try_absorb(w, target, &rec) {
                self.set_task_flag(i, T_ABSORBED, contact);
            } else if guard_blast(w, target, &rec) {
                self.set_task_flag(i, T_GUARDED, contact);
            } else if hit(w, target, &rec) {
                self.set_task_flag(i, T_HIT_CHAR, contact);
            }
        }
    }
}

// ---- the task (eft_struggle.c) -------------------------------------------------------------------------------------

/// `EftKiBlast_Update`: homing, one step along the direction (not on the first frame), the lifetime, the end
/// animation, and this frame's hit record while the blast is neither ending nor ended. Nothing while any fighter
/// is in hit-stop.
fn task_update(t: &mut Task, w: &mut World, fx: &mut Modules, stopped: bool) -> Option<HitRec> {
    let mut rec = None;
    if !stopped {
        if t.flags & W_STOPPED == 0 {
            if t.flags & W_HOMING != 0 {
                t.dir = home(
                    &*w.chars,
                    t.pos,
                    &mut t.dir,
                    t.arg.owner,
                    t.arg.speed,
                    t.arg.turn,
                );
            }
            if t.flags & W_STARTED != 0 {
                if t.flags & W_MOVED != 0 {
                    t.prev = t.pos;
                }
                if t.flags & W_ENDING == 0 {
                    t.pos = add(t.pos, scale3(t.dir, t.arg.speed));
                }
                t.flags |= W_MOVED;
            }
            t.flags |= W_STARTED;
        }
        t.life -= 1;
        if t.life <= 0 {
            t.flags |= W_ENDING;
        }
        spawn_parts(t, w, fx, false);
        if t.flags & W_ENDED != 0 {
            t.flags |= W_KILL;
        } else if t.flags & W_ENDING != 0 {
            if t.end_timer >= t.end_frames {
                t.flags |= W_ENDED;
            }
            t.end_timer += 1.0;
        } else if t.arg.can_hit {
            rec = Some(add_hit_record(t));
        }
    }
    rec
}

/// `EftKiBlast_AddHitRecord` and `EftHit_Add`: two spheres at this and last frame's position.
fn add_hit_record(t: &Task) -> HitRec {
    let radius = t.radius * t.trail_width;
    let a = Sphere { pos: t.pos, radius };
    let b = Sphere {
        pos: t.prev,
        radius,
    };
    HitRec {
        owner: t.arg.owner,
        level: t.arg.level,
        pos: t.pos,
        prev: t.prev,
        vel: t.dir,
        turned: t.flags & W_TURNED != 0,
        atk: t.arg,
        seen_by_ai: false,
        task: t.id,
        shape: Shape {
            a,
            b,
            scale: 0.1,
            bounds: capsule_bounds(b.pos, a.pos, radius),
            delta: [0.0; 3],
            dir: [0.0; 3],
            zone: 0,
            hit_flags: 0,
            stage_dist: f32::MAX,
            dist: f32::MAX,
            hit_pos: [0.0; 3],
            contact: [0.0; 3],
        },
    }
}

/// `EftKiBlast_PostUpdate`: an end report stops the blast where it was reported; a deflection or reflection (not
/// after a deflection) turns it from there.
fn task_post_update(t: &mut Task, w: &mut World, fx: &mut Modules) {
    if let Some(set) = list_head(w.data[t.arg.src], t.list) {
        t.state.update_alive(set, fx);
    }
    t.mask = 0;
    if t.hit & T_END != 0 {
        t.life = 0;
        t.flags |= W_ENDING;
        if t.flags & W_STOPPED == 0 {
            t.pos = t.hit_pos;
        }
        spawn_parts(t, w, fx, true);
        if t.flags & W_STOPPED == 0 {
            end_trail(t, w);
        }
        t.flags |= W_STOPPED;
    } else if t.hit & T_TURN != 0 && t.flags & W_DEFLECTED == 0 {
        // EftKiBlast_Turn: the lerp it starts with is overwritten below.
        let by = t.arg.owner ^ 1;
        let dir = deflect_dir(w, by);
        if t.hit & T_DEFLECTED != 0 {
            t.dir = dir;
            t.arg.owner ^= 1;
            t.flags = (t.flags & !W_HOMING) | W_DEFLECTED;
        } else if t.hit & T_REFLECTED != 0 {
            t.dir = dir;
            t.arg.owner ^= 1;
            t.life = t.arg.life;
        }
        t.pos = t.hit_pos;
        spawn_parts(t, w, fx, true);
        end_trail(t, w);
        t.flags |= W_TURNED;
        t.hit &= !T_TURN;
    }
}

/// `EftKiBlast_SpawnParts`: every emitter of the set gets this frame's flags (from the phase mask and the end,
/// or the reset flags after a hit or a turn) and is driven at the blast with its radius as the scale.
fn spawn_parts(t: &mut Task, w: &mut World, fx: &mut Modules, reset: bool) {
    let src = t.arg.src;
    let data = w.data[src];
    let (Some(set), Some(assets)) = (list_head(data, t.list), data.blast_assets[t.list]) else {
        return;
    };
    let opp = src ^ 1;
    let ctx = SpawnCtx {
        chr: src,
        set,
        assets,
        set_ref: SetRef {
            side: src,
            list: t.list,
        },
        char_scale: [data.height() / 19.35, w.data[opp].height() / 19.35],
        opp_chara_flags: w.data[opp].tables.params.chara_flags(),
        opp_node3: w.chars[opp].obj.node_pos(3),
    };
    let ending = t.flags & W_ENDING != 0;
    for kind in 0..0x13u8 {
        if set.head.mask & (1 << kind) == 0 {
            continue;
        }
        let Some(g) = set.groups[usize::from(kind)] else {
            continue;
        };
        for sub in 0..g.count {
            let n = g.first + sub;
            let f = if reset {
                t.state.reset_flags(set, kind, n)
            } else {
                t.state.flags_from_mask(set, kind, n, ending, false, t.mask)
            };
            if f != 0 {
                spawn(
                    fx,
                    &mut t.state,
                    &t.nodes,
                    &ctx,
                    t.pos,
                    t.dir,
                    kind,
                    sub,
                    f,
                    t.radius,
                    w.libc,
                );
            }
        }
    }
}

/// `EftKiBlast_EndTrail`: the ribbon emitters (group 17) restart.
fn end_trail(t: &mut Task, w: &World) {
    if let Some(set) = list_head(w.data[t.arg.src], t.list) {
        t.state.end_trail(set);
    }
}

/// `EftAim_Home`: the direction turned toward the opponent's node 0x11, led by half its frame movement times the
/// frames to impact (at most 15), by at most `max_turn` (spread over half the frames left); unchanged when the owner
/// is not locked on or the target is behind. `dir` is normalised in place.
fn home(
    chars: &[Chr; 2],
    pos: [f32; 3],
    dir: &mut [f32; 3],
    owner: usize,
    speed: f32,
    max_turn: f32,
) -> [f32; 3] {
    *dir = norm(*dir);
    if !chars[owner].flags.test(5) {
        return *dir;
    }
    let opp = &chars[owner ^ 1];
    let mut target = opp.obj.node_pos(0x11);
    let frames = length(sub(target, pos)) / speed;
    let lead = frames.min(15.0);
    target = add(target, scale3(opp.pose.moved, lead * 0.5));
    let to = norm(sub(target, pos));
    let cur = norm(*dir);
    let d = dot(to, cur);
    let new = if d > 0.0 {
        let mut angle = d.min(1.0).acos();
        if frames > 2.0 {
            angle /= frames * 0.5;
        }
        let t = if max_turn < angle { max_turn } else { angle };
        let axis = norm(cross(cur, to));
        norm(rotate_axis(cur, axis, t))
    } else {
        cur
    };
    norm(new)
}

/// `BtlCharApi_GetDeflectDir`: the fighter's facing turned by two random angles that depend on its motion, else a
/// cone of 0.3 rad round the direction to its opponent.
fn deflect_dir(w: &mut World, by: usize) -> [f32; 3] {
    let ts = w.time_stopped;
    let mut r = || rand_f(w.rand, ts);
    let chr = &w.chars[by];
    let yaw = chr.pose.rot[1];
    let fwd = [yaw.sin(), 0.0, yaw.cos()];
    let side = [fwd[2], 0.0, -fwd[0]];
    let v = fwd;
    match chr.anim.cur {
        0xF9 => {
            let v = rotate_axis(v, side, r() * 0.3 + 0.3);
            rotate_axis(v, fwd, -(r() * 0.8 + 0.5))
        }
        0xFA => {
            let v = rotate_axis(v, side, r() * 0.3 + 0.3);
            rotate_axis(v, fwd, r() * 0.8 + 0.5)
        }
        0x19 | 0xFB => {
            let v = rotate_axis(v, side, r() * 0.5 + 2.5);
            rotate_axis(v, fwd, r() * 1.8 - 0.9)
        }
        0xFC => {
            let v = rotate_axis(v, side, r() * 0.3 + 0.3);
            rotate_axis(v, fwd, r() * 1.2 - 0.6)
        }
        _ => {
            let fwd = norm(opp_delta(chr, &w.chars[by ^ 1]));
            let side = norm([fwd[2], 0.0, -fwd[0]]);
            let v = rotate_axis(fwd, side, r() * 0.6 - 0.3);
            rotate_axis(v, fwd, r() * 6.283_185_3 - PI)
        }
    }
}

// ---- detection primitives (col_primitives.c) ---------------------------------------------------------------------

/// `ColSphere_SweepSphere`: the fraction of the frame at which two moving spheres first touch; 0 when they overlap
/// at the start. Not limited to 0..1.
fn sweep_sphere(a: Sphere, b: Sphere, move_a: [f32; 3], move_b: [f32; 3]) -> Option<f32> {
    let d = sub(b.pos, a.pos);
    let v = sub(move_b, move_a);
    let r = b.radius + a.radius;
    let qc = dot(d, d) - r * r;
    if qc < 0.0 {
        return Some(0.0);
    }
    let qa = dot(v, v);
    if qa < 1.175_494_4e-36 {
        return None;
    }
    let qb = dot(v, d);
    if qb >= 0.0 {
        return None;
    }
    let disc = qb * qb - qa * qc;
    if disc < 0.0 {
        return None;
    }
    Some((-qb - disc.sqrt()) / qa)
}

/// `ColSphere_TestSphere`.
fn test_sphere(a: Sphere, b: Sphere) -> bool {
    let d = sub(a.pos, b.pos);
    let r = a.radius + b.radius;
    dot(d, d) <= r * r
}

/// `ColSphere_SeparateSphere`: each centre moved half the overlap plus 0.001 apart, and the unit direction a -> b.
fn separate_sphere(a: Sphere, b: Sphere) -> Option<([f32; 3], [f32; 3], [f32; 3])> {
    let d = sub(b.pos, a.pos);
    let dist = dot(d, d).sqrt();
    let nrm = if dist != 0.0 {
        scale3(d, 1.0 / dist)
    } else {
        [1.0, 0.0, 0.0]
    };
    let r = a.radius + b.radius;
    if dist > r && (dist - r).abs() > 0.001 {
        return None;
    }
    let push = (r - dist) * 0.5 + 0.001;
    Some((
        sub(a.pos, scale3(nrm, push)),
        add(b.pos, scale3(nrm, push)),
        nrm,
    ))
}

/// `EftDet_SpheresVsSpheres`: last frame's spheres of both records swept over the frame. Each record's centre
/// and contact point.
#[allow(clippy::type_complexity)]
fn spheres_vs_spheres(
    a: &Shape,
    b: &Shape,
) -> Option<(([f32; 3], [f32; 3]), ([f32; 3], [f32; 3]))> {
    if !a.bounds.overlaps(&b.bounds) {
        return None;
    }
    let (sa, sb) = (a.b, b.b);
    let t = sweep_sphere(sa, sb, a.delta, b.delta)?;
    if t <= 0.000_001 || 1.0 <= t {
        let (pa, pb, nrm) = separate_sphere(sa, sb)?;
        let contact = add(pa, scale3(nrm, sa.radius));
        return Some(((pa, contact), (pb, contact)));
    }
    let pa = add(sa.pos, scale3(a.delta, t));
    let pb = add(sb.pos, scale3(b.delta, t));
    Some((
        (pa, add(pa, scale3(a.dir, sa.radius))),
        (pb, add(pb, scale3(b.dir, sb.radius))),
    ))
}

/// `EftDet_SpheresVsFighter`: last frame's sphere swept along the movement against every body part sphere; the
/// earliest contact wins, a part it already overlaps ends the search. The centre, the contact point and the
/// distance from the start.
fn spheres_vs_fighter(chr: &Chr, s: &Shape) -> Option<([f32; 3], [f32; 3], f32)> {
    let body = &chr.obj.body;
    let hit_box = ColBox {
        min: body.center_bounds.min,
        max: body.center_bounds.max,
    };
    if !s.bounds.overlaps(&hit_box) {
        return None;
    }
    let b = s.b;
    let mut best = 1.0;
    let mut found = None;
    for part in &body.boxes {
        let p = Sphere {
            pos: part.center,
            radius: part.radius,
        };
        let Some(t) = sweep_sphere(b, p, s.delta, [0.0; 3]) else {
            continue;
        };
        if t <= 0.000_001 || 1.0 <= t {
            if test_sphere(b, p) {
                found = Some((b.pos, b.pos));
                break;
            }
        } else if t < best {
            let hit = add(b.pos, scale3(s.delta, t));
            found = Some((hit, add(hit, scale3(s.dir, b.radius))));
            best = t;
        }
    }
    let (hit, contact) = found?;
    Some((hit, contact, length(sub(b.pos, hit))))
}

/// `ColSweep`: a sphere moved along a unit direction (`ColSweep_FromCapsule`).
struct Sweep {
    start: [f32; 3],
    radius: f32,
    dir: [f32; 3],
    length: f32,
}

impl Sweep {
    fn new(a: [f32; 3], b: [f32; 3], radius: f32) -> Self {
        let delta = sub(b, a);
        let length = dot(delta, delta).sqrt();
        let dir = if length > 0.0 {
            scale3(delta, 1.0 / length)
        } else {
            [0.0; 3]
        };
        Self {
            start: a,
            radius,
            dir,
            length,
        }
    }

    /// `ColSweep_TestTri`: the distance travelled until the sphere first touches the triangle (face, then corners,
    /// then edges) and the centre there; back-facing or parallel triangles are ignored. Not limited to the length.
    fn test_tri(&self, v: &[[f32; 3]; 3], nrm: [f32; 4]) -> Option<([f32; 3], f32)> {
        let (c, r, dir) = (self.start, self.radius, self.dir);
        let n = [nrm[0], nrm[1], nrm[2]];
        if dot(dir, n) > -0.001 {
            return None;
        }
        let mut best = f32::MAX;
        let mut found = false;
        let mut h = dot(n, c) - nrm[3];
        if h < -r {
            return None;
        }
        if h > r {
            h -= r;
            let dn = dot(n, dir);
            if dn != 0.0 {
                h = -h / dn;
                let p = add(c, scale3(dir, h));
                if tri_contains(v, p) && h < best {
                    best = h;
                    found = true;
                }
            }
        }
        for &p in v {
            let q = sub(p, dir);
            if let Some((t0, t1)) = line_vs_sphere(c, r, p, q) {
                let m = t0.min(t1);
                if m < 0.0 {
                    continue;
                }
                if m < best {
                    best = m;
                    found = true;
                }
            }
        }
        for i in 0..3 {
            let a = v[i];
            let b = v[(i + 1) % 3];
            let pl = plane_from_points(a, b, sub(b, dir));
            let pn = [pl[0], pl[1], pl[2]];
            let d = dot(pn, c) + pl[3];
            if d > r || d < -r {
                continue;
            }
            let rad = (r * r - d * d).sqrt();
            let proj = sub(c, scale3(pn, d));
            let near = line_nearest(proj, a, b);
            let n2 = norm(sub(near, proj));
            let hit = add(scale3(n2, rad), proj);
            let (ax, ay, az) = (pl[0].abs(), pl[1].abs(), pl[2].abs());
            let (i0, i1) = if ax > ay && ax > az {
                (1, 2)
            } else if ay > az {
                (0, 2)
            } else {
                (0, 1)
            };
            let hit2 = add(hit, dir);
            let Some(t) = line2_intersect(
                [hit[i0], hit[i1]],
                [hit2[i0], hit2[i1]],
                [a[i0], a[i1]],
                [b[i0], b[i1]],
            ) else {
                continue;
            };
            if t < 0.0 {
                continue;
            }
            let at = add(hit, scale3(dir, t));
            if dot(sub(a, at), sub(b, at)) > 0.0 {
                continue;
            }
            if t > best {
                continue;
            }
            best = t;
            found = true;
        }
        found.then(|| (add(c, scale3(dir, best)), best))
    }
}

/// `ColTri_ContainsPoint`: a point of the triangle's plane inside it, edges included.
fn tri_contains(v: &[[f32; 3]; 3], p: [f32; 3]) -> bool {
    let e0 = sub(v[1], v[0]);
    let e1 = sub(v[2], v[0]);
    let d = sub(p, v[0]);
    let (a, b, c) = (dot(e0, e0), dot(e0, e1), dot(e1, e1));
    let (d0, d1) = (dot(d, e0), dot(d, e1));
    let mut det = b * b - a * c;
    if det == 0.0 {
        det = 0.001;
    }
    let inv = 1.0 / det;
    let u = (b * d1 - c * d0) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let w = (b * d0 - a * d1) * inv;
    if w < 0.0 {
        return false;
    }
    u + w <= 1.0
}

/// `ColSphere_TestLine`: the parameters of the line a + t (b - a) where it cuts the sphere, the larger first (the
/// second is `f32::MAX` for a single root).
fn line_vs_sphere(c: [f32; 3], r: f32, a: [f32; 3], b: [f32; 3]) -> Option<(f32, f32)> {
    let ba = sub(b, a);
    let mut qa = dot(ba, ba);
    let qb = 2.0 * dot(ba, sub(a, c));
    let qc = dot(c, c) + dot(a, a) - 2.0 * dot(c, a) - r * r;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return None;
    }
    if qa == 0.0 {
        qa = 0.0001;
    }
    if disc == 0.0 {
        return Some((-qb / (2.0 * qa), f32::MAX));
    }
    let root = (qb * qb - 4.0 * qa * qc).sqrt();
    Some(((-qb + root) / (2.0 * qa), (-qb - root) / (2.0 * qa)))
}

/// `ColPlane_FromPoints`: unit normal (c - b) x (a - b), w = -(normal . a).
fn plane_from_points(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 4] {
    let n = norm(cross(sub(c, b), sub(a, b)));
    [n[0], n[1], n[2], -dot(a, n)]
}

/// `ColLine_DistSqPoint`'s nearest point on the infinite line a..b.
fn line_nearest(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let d = sub(p, a);
    let l = sub(b, a);
    let mut len = dot(l, l);
    if len == 0.0 {
        len = 0.0001;
    }
    add(a, scale3(l, dot(d, l) / len))
}

/// `ColLine2_Intersect`: t of the crossing of p + t (q - p) with the line r..s; `None` when parallel.
fn line2_intersect(p: [f32; 2], q: [f32; 2], r: [f32; 2], s: [f32; 2]) -> Option<f32> {
    let d1 = [q[0] - p[0], q[1] - p[1]];
    let d2 = [r[0] - s[0], r[1] - s[1]];
    let det = d2[1] * d1[0] - d2[0] * d1[1];
    if det == 0.0 {
        return None;
    }
    Some((d2[0] * (p[1] - r[1]) - d2[1] * (p[0] - r[0])) / det)
}

// ---- the fighter's answers (btl_hit_reaction.c) -------------------------------------------------------------------

/// The record behind a hit and the fighter that fired it (`BtlKiBlast_GetOfHit`).
fn hit_record<'a>(w: &'a World, rec: &HitRec) -> &'a KiBlastRecord {
    record(w.data[rec.atk.src], rec.atk.kind)
}

/// `BtlKiBlast_GetFlagsOfHit`.
fn hit_flags(w: &World, rec: &HitRec) -> u16 {
    hit_flags_of(rec, w.chars, w.data)
}

/// `BtlKiBlast_GetFlagsOfHit` with the fighters and their data.
pub(crate) fn hit_flags_of(rec: &HitRec, chars: &[Chr; 2], data: [&SideData; 2]) -> u16 {
    flags_of(&chars[rec.atk.src], record(data[rec.atk.src], rec.atk.kind))
}

/// `BtlColl_IsFacing`.
fn is_facing(chr: &Chr, yaw: f32) -> bool {
    wrap_angle(chr.pose.rot[1] - yaw).abs() < 1.570_796_3
}

/// `BtlColl_GetHitDir` for a blast: its travel direction (from its position to the fighter's node 3 when it does
/// not move); the yaw and the unit direction.
fn hit_dir(chr: &Chr, rec: &HitRec) -> (f32, [f32; 3]) {
    let mut dir = if length(rec.vel) < 0.01 {
        sub(chr.obj.node_pos(3), rec.pos)
    } else {
        rec.vel
    };
    let yaw = if dir[0].abs() > 0.001 || dir[2].abs() > 0.001 {
        dir = norm(dir);
        dir[0].atan2(dir[2])
    } else {
        if dir[1].abs() > 0.001 {
            dir = norm(dir);
        }
        0.0
    };
    (yaw, dir)
}

/// `BtlColl_TryDodge` for a blast: a fighter in hit-stop or with flag 0xAA lets it pass; flag 0x5A never; else flag
/// 0x44.
fn try_dodge(w: &World, target: usize, _rec: &HitRec) -> bool {
    let chr = &w.chars[target];
    if chr.is_frozen() || chr.flags.test(0xAA) {
        return true;
    }
    if chr.flags.test(0x5A) {
        return false;
    }
    // No technique with a clash power is ported.
    chr.flags.test(0x44)
}

/// `BtlColl_TryDeflect`: a free fighter facing the blast knocks it away with flag 0x50 (kinds 0, 4, 8) or 0x51,
/// or perfectly inside its deflect window; the powered-up skill 0x80 always does.
fn try_deflect(w: &mut World, target: usize, rec: &HitRec) -> bool {
    if hit_flags(w, rec) & 2 != 0 || !w.chars[target].is_free() {
        return false;
    }
    let pflags = w.data[target].tables.params.flags();
    let chr = &mut w.chars[target];
    if !chr.flags.test(0x52)
        && !chr.flags.test(0x54)
        && chr.flags.test(6)
        && w.data[target].tables.params.flags3() & 0x80 != 0
    {
        return true;
    }
    if is_facing(chr, hit_dir(chr, rec).0) {
        return false;
    }
    let perfect = chr.flags.test(0x41) && chr.unk1074 > 0 && pflags & 2 == 0;
    if !perfect {
        let need = if matches!(rec.atk.kind, 0 | 4 | 8) {
            0x50
        } else {
            0x51
        };
        if !chr.flags.test(need) {
            return false;
        }
    }
    chr.add_blast(5000);
    if perfect {
        chr.flags.set(0x6B);
    }
    chr.deflect_dir = deflect_dir_of(chr, rec);
    chr.set_fx(0x2E);
    true
}

/// The direction a knocked-away blast is noted in (`deflectDir`): against its travel, else the fighter's.
fn deflect_dir_of(chr: &Chr, rec: &HitRec) -> [f32; 3] {
    let len = length(rec.vel);
    if len < 0.001 {
        chr.pose.dir
    } else {
        scale3(rec.vel, -1.0 / len)
    }
}

/// `BtlColl_TryReflect`: a free fighter with flag 0x52 facing the blast sends it back.
fn try_reflect(w: &mut World, target: usize, rec: &HitRec) -> bool {
    if hit_flags(w, rec) & 2 != 0 {
        return false;
    }
    let chr = &mut w.chars[target];
    if !chr.flags.test(0x52) || !chr.is_free() || is_facing(chr, hit_dir(chr, rec).0) {
        return false;
    }
    chr.deflect_dir = deflect_dir_of(chr, rec);
    chr.set_fx(0x2E);
    true
}

/// `BtlColl_TryAbsorb`: a free fighter with flag 0x54 (or a perfect guard with parameter flag 2) facing a type 0 /
/// 1 blast takes three times its ki cost.
fn try_absorb(w: &mut World, target: usize, rec: &HitRec) -> bool {
    let pflags = w.data[target].tables.params.flags();
    let flags = hit_flags(w, rec);
    let blast = *hit_record(w, rec);
    let chr = &mut w.chars[target];
    let mut alt = false;
    if !chr.flags.test(0x54) {
        alt = chr.flags.test(0x41) && chr.unk1074 > 0 && pflags & 2 != 0;
        if !alt {
            return false;
        }
    }
    if !chr.is_free() || is_facing(chr, hit_dir(chr, rec).0) {
        return false;
    }
    if flags & 2 != 0 || matches!(blast.kind_type(), 2 | 3) {
        return false;
    }
    let amount = blast.ki() * 3;
    // BtlMember_AddKi: the overflow feeds the max power gauge outside the powered-up mode.
    if amount > 0 {
        let g = &mut chr.gauge;
        g.ki += amount;
        let over = (g.ki - g.ki_max).max(0);
        g.ki = g.ki.min(g.ki_max);
        if over > 0 && !chr.flags.test(6) {
            g.max_power = (g.max_power + amount * 30000 / 100_000).min(30000);
        }
    }
    chr.set_fx(0x37);
    if alt {
        chr.flags.set(0x6B);
    }
    true
}

/// `BtlColl_TryGuard` with `BtlColl_GuardBlast`: a fighter in a guard motion takes the guarded damage and a push
/// from a blast it faces (any unblockable blast goes through).
fn guard_blast(w: &mut World, target: usize, rec: &HitRec) -> bool {
    if w.chars[target].flags.test(0x59) {
        return true;
    }
    let flags = hit_flags(w, rec);
    let blast = *hit_record(w, rec);
    let env = w.env;
    let data = w.data[target];
    let chr = &mut w.chars[target];
    if chr.flags.test(0x44) {
        return false;
    }
    let kind = guard_kind(chr);
    let (yaw, dir) = hit_dir(chr, rec);
    if kind != 6 && (flags & 1 != 0 || is_facing(chr, yaw)) {
        return false;
    }
    let mut dmg = damage_of(&blast, env, true);
    if rec.atk.hits >= 2 {
        dmg /= rec.atk.hits;
    }
    if kind == 0 || kind == 5 {
        return false;
    }
    chr.flags.set(0x66);
    chr.damage(env, data, dmg, 0xC);
    chr.set_impulse_dir(dir, push_of(&blast, true));
    true
}

/// `BtlColl_Hit` with `BtlColl_HitByBlast`: a pending reaction that cannot be replaced takes the hit without
/// effect; else the reaction by the victim's state, the damage and the push, and the victim locks on.
fn hit(w: &mut World, target: usize, rec: &HitRec) -> bool {
    if matches!(
        w.chars[target].react.reaction,
        9..=12 | 17 | 19..=22 | 29 | 30 | 34 | 37 | 39..=41
    ) {
        return true;
    }
    let flags = hit_flags(w, rec);
    let blast = *hit_record(w, rec);
    let env = w.env;
    let (data, atk_data) = (w.data[target], w.data[rec.atk.src]);
    let atk_powered = w.chars[rec.atk.src].flags.test(6);
    let chr = &mut w.chars[target];
    let st = env.anim_flags(chr.anim.cur);
    if chr.flags.test(0x44) {
        return false;
    }
    // A fighter using a technique (none is ported) would take it without effect.
    let (yaw, dir) = hit_dir(chr, rec);
    let front = is_facing(chr, yaw);
    let or_plain = |v: i8| i32::from(if v != 0 { v } else { blast.react() });
    let react = if chr.no_flinch > 0 {
        REACT_NOFLINCH
    } else if st & 0x4000 != 0 || chr.act.current == 0xDF {
        or_plain(blast.react_s4000())
    } else if st & 0x800 != 0 {
        // BtlKiBlast_MapReactS800: ability 0x4B is not ported.
        or_plain(blast.react_s800())
    } else if st & 0x200 != 0 {
        or_plain(blast.react_s200())
    } else {
        let (mine, theirs) = (data.tables.params.flags(), atk_data.tables.params.flags());
        let mut n = i32::from(mine & 4 != 0);
        if mine & 8 != 0 {
            n -= 1;
        }
        if theirs & 4 != 0 {
            n -= 1;
        }
        if theirs & 8 != 0 {
            n += 1;
        }
        if chr.flags.test(6) && data.tables.params.flags3() & 0x40 != 0 {
            n += 1;
        }
        if chr.skill_timer_c > 0 {
            n += 1;
        }
        if atk_powered && atk_data.tables.params.flags3() & 0x100 != 0 {
            n -= 1;
        }
        if n > 0 && flags & 0x10 != 0 {
            REACT_NOFLINCH
        } else if st & 0x100 != 0 && flags & 0x10 != 0 {
            let h = &chr.obj.hit;
            if chr.obj.query_event(&data.obj, 1, 0, 3) == 0
                || i32::from(h.index) != !i32::from(h.count)
            {
                REACT_NOFLINCH
            } else {
                i32::from(blast.react())
            }
        } else {
            i32::from(blast.react())
        }
    };
    if react == REACT_NOFLINCH {
        chr.react.reaction = react;
        chr.shake_timer = 3;
    } else {
        chr.react.reaction = react;
        chr.react.scale = 0.0;
        chr.react.turn_yaw = 0.0;
        chr.react.turn_pitch = 0.0;
        chr.react.yaw = yaw;
        chr.react.back = front;
        chr.set_impulse_dir(dir, push_of(&blast, false));
    }
    chr.set_fx(0x1D);
    let mut dmg = damage_of(&blast, env, false);
    if rec.atk.hits >= 2 {
        dmg /= rec.atk.hits;
    }
    chr.damage(env, data, dmg, 0);
    if !chr.flags.test(0x93) {
        chr.flags.set_held(5);
    }
    let (atk, def) = if rec.owner == 0 {
        let [a, b] = &mut *w.chars;
        (a, b)
    } else {
        let [b, a] = &mut *w.chars;
        (a, b)
    };
    raise_first_clash(atk, def);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swept_spheres_touch_at_the_right_fraction() {
        let a = Sphere {
            pos: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let b = Sphere {
            pos: [10.0, 0.0, 0.0],
            radius: 1.0,
        };
        // a moves 10 towards b: they touch after 8 units.
        let t = sweep_sphere(a, b, [10.0, 0.0, 0.0], [0.0; 3]).unwrap();
        assert!((t - 0.8).abs() < 1e-5);
        // Moving apart never touches.
        assert!(sweep_sphere(a, b, [-10.0, 0.0, 0.0], [0.0; 3]).is_none());
        // Overlapping at the start.
        assert_eq!(sweep_sphere(a, a, [1.0, 0.0, 0.0], [0.0; 3]), Some(0.0));
    }

    #[test]
    fn swept_sphere_meets_a_floor_triangle_face_first() {
        // A floor at y = 0 facing up (-Y is up: the normal is (0, -1, 0), n . p = w with w = 0).
        let v = [[-10.0, 0.0, -10.0], [10.0, 0.0, -10.0], [0.0, 0.0, 10.0]];
        let nrm = [0.0, -1.0, 0.0, 0.0];
        let sweep = Sweep::new([0.0, -5.0, 0.0], [0.0, 5.0, 0.0], 1.0);
        let (pos, dist) = sweep.test_tri(&v, nrm).unwrap();
        assert!((dist - 4.0).abs() < 1e-5);
        assert!((pos[1] + 1.0).abs() < 1e-5);
        // Moving away from the face is ignored.
        let back = Sweep::new([0.0, 5.0, 0.0], [0.0, -5.0, 0.0], 1.0);
        assert!(back.test_tri(&v, nrm).is_none());
    }

    #[test]
    fn fighter_generator_matches_the_original_sequence() {
        let mut s = 0;
        let a = rand_f(&mut s, false);
        assert_eq!(s, 4096);
        assert!((a - 4096.0 / 150_889.0).abs() < 1e-7);
        let b = rand_f(&mut s, false);
        assert_eq!(s as u64, (4096u64 * 714_025 + 4096) % 150_889);
        assert!(b < 1.0);
        // Kept while time is stopped.
        let before = s;
        rand_f(&mut s, true);
        assert_eq!(s, before);
    }
}
