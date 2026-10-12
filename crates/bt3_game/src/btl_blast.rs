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
//! The technique effects (layer 1, `btl_tech_fx`) add their records before the ki blasts; the passes and the
//! fighters' answers take both kinds (`EftHit_CanHit`, `EftHit_Clash`, `EftHit_CalcResult`).
//!
//! The ki blasts of types 4 / 5 are discs (`btl_tech_disc`), those of types 2 / 3 thrown objects and bombs
//! (`btl_ki_obj`).
//!
//! The fighters' answers play their sounds and voices (`BtlColl_*`), the shots their fire and stage-contact sounds.
//!
//! Not ported: the struggle's spark; the particles, impact effects, camera shake, pad vibration and battle events.

use std::f32::consts::PI;

use bt3_data::chara_data::KiBlastRecord;
use bt3_data::eft_pack::EmitSet;
use bt3_data::stage_col::{ColBox, ColPoly, POLY_NO_SOLID};

use crate::btl_chr::{Chr, Env, SideData, add, dot, kmh, length, opp_delta, scale3, sub};
use crate::btl_hit::{guard_kind, raise_first_clash};
use crate::btl_input::wrap_angle;
use crate::btl_ki_obj::{KiObj, LIST_MAX};
use crate::btl_obj::mask_to_node;
use crate::btl_tech_fx::{
    TechFx, TechSrc, absorb_amount, guard_rush, guard_strike, hit_by_rush, hit_by_strike,
    tech_outclasses, try_dodge_tech,
};
use crate::eft::{EmitState, FxWorld, Modules, Nodes, SetAssets, SetRef, SpawnCtx, spawn};
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
const T_STRUGGLE: u32 = 0x100;
const T_IMPACT_DONE: u32 = 0x200;
const T_MULTI: u32 = 0x4000;
const T_MULTI_CONTACT: u32 = 0x8000;
const T_KEEP: u32 = 0x1_0000;
const T_NO_MODE1: u32 = 0x2_0000;
const T_NO_MODE2: u32 = 0x4_0000;
const T_END: u32 = 0x3F;

// EFT_HIT_FLAG_*.
const H_NO_IMPACT: u32 = 0x2;
const H_KIND9: u32 = 0x8;
pub(crate) const H_LAST: u32 = 0x10;
const H_ALWAYS: u32 = 0x20;
const H_NEVER: u32 = 0x40;
const H_NO_MODE1: u32 = 0x80;
const H_GROUND: u32 = 0x400;
pub(crate) const H_NO_LAST: u32 = 0x800;
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

/// `EftRArg` / `FxHitArg2`: what a shot is fired with (a technique record carries a blank one).
#[derive(Clone, Copy, Debug, Default)]
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
    /// `unk1B`: ki blast record byte 3 (the hit record's `unk4`; not read yet).
    #[allow(dead_code)]
    pub unk1b: u8,
    pub life: i32,
    pub speed: f32,
    pub turn: f32,
    pub radius: f32,
    /// `unk2C`: ki blast record +0x2C (not read yet).
    #[allow(dead_code)]
    pub unk2c: f32,
    /// `size`: 0..3, the record's clash level.
    pub level: i32,
    pub can_hit: bool,
    /// `unk3C`: bit 0x20 of the ki blast record's flags (not read yet).
    #[allow(dead_code)]
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

/// The swept shape of a record and what the passes found (`EftDetShape`): type 0 two spheres, this frame's (`a`)
/// and last frame's (`b`); type 1 two capsules from the muzzle `a_start`, to `a.pos` (the head) and to `b.pos`
/// (the tail), of `a`'s radius.
#[derive(Clone, Copy, Debug)]
struct Shape {
    kind: u8,
    a: Sphere,
    b: Sphere,
    a_start: [f32; 3],
    /// `EftHit_GetScale`: 0.1 for a blast, the definition's hit scale for a technique (the stage test's radius
    /// factor).
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
    /// `origin` (`pose.start`): where the shot started (a beam's muzzle when it fired).
    pub origin: [f32; 3],
    /// The technique of a technique record (`src`; `None` for a ki blast).
    pub tech: Option<TechSrc>,
    /// `EFT_HIT_FLAG_*` (0x10: the technique's last hit).
    pub flags: u32,
    /// `unk54`: the technique's definition flag 2; the deflects, absorbs and the AI ignore the record.
    pub unk54: bool,
    task: u32,
    shape: Shape,
}

/// The task header fields the passes report through (`BtlTask`: the tag word, the position, the result and the
/// multi-hit counters).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Tag {
    /// `EFT_TASK_*`.
    pub flags: u32,
    pub pos: [f32; 3],
    /// `EftHit_CalcResult`: 0 none, 1 in contact (hold), 3 struggle, 4 hit, 5 finished.
    pub result: i32,
    pub count_a: i32,
    pub hit_count: i32,
}

impl HitRec {
    /// `EftHit_GetNew` / `EftHit_Add` of a technique record: two spheres at `pos` and `prev` (shape 0) or two
    /// capsules from `start` (shape 1) of `radius`; `None` for another shape.
    pub(crate) fn technique(
        src: TechSrc,
        task: u32,
        pos: [f32; 3],
        prev: [f32; 3],
        vel: [f32; 3],
        start: [f32; 3],
        radius: f32,
    ) -> Option<Self> {
        Self::technique_shape(src, task, pos, prev, vel, start, radius, src.def.shape)
    }

    /// The same with the shape given (a disc piece's records are always spheres).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn technique_shape(
        src: TechSrc,
        task: u32,
        pos: [f32; 3],
        prev: [f32; 3],
        vel: [f32; 3],
        start: [f32; 3],
        radius: f32,
        shape: i32,
    ) -> Option<Self> {
        let kind = match shape {
            0 => 0,
            1 => 1,
            _ => return None,
        };
        let a = Sphere { pos, radius };
        let b = Sphere { pos: prev, radius };
        Some(Self {
            owner: src.side,
            level: src.def.level,
            pos,
            prev,
            vel,
            turned: false,
            atk: Arg::default(),
            seen_by_ai: false,
            origin: start,
            tech: Some(src),
            flags: if src.def.sub == 9 { H_KIND9 } else { 0 },
            unk54: src.def.flags & 2 != 0,
            task,
            shape: Shape {
                kind,
                a,
                b,
                a_start: start,
                scale: src.def.hit_scale,
                bounds: capsule_bounds(start, pos, radius),
                delta: [0.0; 3],
                dir: [0.0; 3],
                zone: 0,
                hit_flags: 0,
                stage_dist: f32::MAX,
                dist: f32::MAX,
                hit_pos: [0.0; 3],
                contact: [0.0; 3],
            },
        })
    }

    /// `EftHit_GetMaxHits`.
    fn max_hits(&self) -> i32 {
        self.tech.map_or(0, |t| t.def.max_hits)
    }

    /// `EftHit_HasDefFlag20`.
    fn def_flag20(&self) -> bool {
        self.tech.is_some_and(|t| t.def.flags & 0x20 != 0)
    }
}

/// An `EftKiBlast` task: its tag word and position (the task header) and its work.
#[derive(Clone, Debug)]
struct Task {
    id: u32,
    /// What the passes reported (`T_*`) and where.
    tag: Tag,
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
    /// The technique effects (layer 1).
    pub tech: TechFx,
    /// The beam struggle (`EftStruggle`, layer 3).
    pub struggle: Option<Struggle>,
    /// The thrown objects (kind 1) and bombs (kind 2) of the ki blasts of types 2 / 3, by fighter, in the place of
    /// those lists.
    pub ki_objs: [[Vec<KiObj>; 2]; 2],
    /// The effect generators' draws made in the fighters' pass, taken in order at the start of the scene update.
    deferred: Vec<Deferred>,
    /// The ground dust (`gEftGndDust`).
    pub gdust: crate::eft_gdust::GndDust,
    /// The speed lines (`gEftSpdLine`, layer 3; kept through a reset).
    pub spdlines: crate::eft_spdline::SpdLines,
    /// The vanish lines (`gEftShotFx`, layer 3; kept through a reset).
    pub shotfx: crate::eft_shotfx::ShotFx,
    /// The aura (`gEftAuraPool`, layer 3).
    pub auras: crate::eft_aura::Auras,
    /// The body lightning (`gEftBoltPool`, layer 3).
    pub bolts: crate::eft_bolt::Bolts,
    /// The power-up glow (`gEftGlow`, layer 3).
    pub glows: crate::eft_glow::Glows,
    /// The impacts (`gEftImpactList`, layer 3).
    pub impacts: crate::eft_impact::Impacts,
    /// The transformation effect (`gEftTransformList`, layer 3).
    pub transforms: crate::eft_transform::Transforms,
    /// Each fighter's drain glows (its kind 9 manager, layer 2).
    pub absorbs: [crate::eft_absorb::Absorbs; 2],
    /// Each fighter's shocks (its kind 4 manager, layer 2).
    pub shocks: [crate::eft_shock::Shocks; 2],
    /// Each fighter's charge auras (its kind 0 manager, layer 2).
    pub charges: [crate::eft_charge::Charges; 2],
    /// Each fighter's rush-finish bursts (its kind 5 manager, layer 2).
    pub rush_bursts: [crate::eft_rushburst::RushBursts; 2],
    /// The screen shock waves (`gScrWarp`).
    pub warps: crate::scr_warp::ScrWarps,
    /// The stage effects of layer 0.
    pub stage: crate::stage_fx::StageFx,
    /// The stage tint (`gStgTint`; a member of the screen effects' group, kept here for the requests that start
    /// it).
    pub tints: crate::stg_tint::Tints,
    /// The full-screen flash and the blinding overlay (`gEftFlash`, `gEftBlind`, layer 3).
    pub flashes: crate::eft_flash::Flashes,
    /// The radial blur of the two views (`gStgBlur`; a member of the screen effects' group, kept here for the
    /// fighters and techniques that set it).
    pub blurs: crate::stg_blur::Blurs,
}

/// A generator draw made outside the scene update.
#[derive(Clone, Copy, Debug)]
enum Deferred {
    /// libc `rand()`.
    Libc,
    /// A bomb's throw: three VU0 draws, then two libc ones.
    Bomb(usize, u32),
}

/// One side of a beam struggle: a copy of its record.
#[derive(Clone, Copy, Debug)]
struct StruggleSide {
    task: u32,
    start: [f32; 3],
    scale: f32,
    power: f32,
}

/// The beam struggle task (`EftStruggle`): two head-on technique records (fighter 0's first) held where the
/// clash's bias puts the struggle point between their starts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Struggle {
    side: [StruggleSide; 2],
    pub bias: f32,
    pub pos: [f32; 3],
    frames: i32,
    end: bool,
}

/// `Vec3_Lerp(out, a, b, t)`: a t + b (1 - t).
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] * t + b[i] * (1.0 - t))
}

impl Struggle {
    /// Both fighters' powers (side 0 is fighter 0's record).
    pub(crate) fn powers(&self) -> [f32; 2] {
        [self.side[0].power, self.side[1].power]
    }

    /// `EftStruggle_Start` / `EftStruggle_Init`: both sides' powers (the definition's +0x88), the point from the
    /// clash bias. The spark is drawing only.
    fn start(a: &HitRec, b: &HitRec, bias: f32) -> Option<Self> {
        let (r0, r1) = if a.owner == 0 { (a, b) } else { (b, a) };
        let side = |r: &HitRec| {
            r.tech.map(|t| StruggleSide {
                task: r.task,
                start: r.origin,
                scale: t.def.scale,
                power: t.def.power,
            })
        };
        let side = [side(r0)?, side(r1)?];
        let bias = ((bias + 1.0) * 0.5).clamp(0.0, 1.0);
        Some(Self {
            pos: lerp3(side[1].start, side[0].start, bias),
            side,
            bias,
            frames: 0,
            end: false,
        })
    }
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
    /// The roster's frame counter (`BtlChar_FrameMod`).
    pub frame: u32,
    /// `BtlCharApi_GetClashBias`: clash A's bias.
    pub clash_bias: f32,
}

// ---- vectors ------------------------------------------------------------------------------------------------------

/// `Vec3_Normalize`: a zero vector gives zero.
pub(crate) fn norm(v: [f32; 3]) -> [f32; 3] {
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
pub(crate) fn capsule_bounds(a: [f32; 3], b: [f32; 3], r: f32) -> ColBox {
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

/// `BtlColl_PlayContactSound`: a record that hit the stage: a ki blast's by its type (2: 0x42, 3: 0xC / 0xD,
/// others 0x1B / 0x11 by the frame), a skill's or technique's its hit sound (the technique's second variant).
fn contact_sound(rec: &HitRec, env: &Env, data: [&SideData; 2], frame: u32) {
    match rec.tech {
        None => {
            let sound = match record(data[rec.atk.src], rec.atk.kind).kind_type() {
                2 => 0x42,
                3 => {
                    if frame % 2 != 0 {
                        0xC
                    } else {
                        0xD
                    }
                }
                _ => {
                    if frame % 2 != 0 {
                        0x1B
                    } else {
                        0x11
                    }
                }
            };
            env.snd().request(
                rec.pos,
                -1,
                crate::btl_snd::KIND_COMMON,
                200.0,
                1500.0,
                sound,
            );
        }
        Some(src) => {
            let t = &data[src.side].tables;
            let kind = if src.slot < 2 {
                t.skill().hit_sound(src.slot)
            } else {
                t.tech().hit_sound(src.slot, true)
            };
            crate::btl_snd::hit_sound(None, Some(rec.pos), kind as u32, env, frame);
        }
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

/// `BtlKiBlast_ScaleDamageByAbility`: +50 % (ability 0xA), then halved (0xB).
fn scale_damage(me: &Chr, mut d: i32) -> i32 {
    if me.has_ability(0xA) {
        d += d / 2;
    }
    if me.has_ability(0xB) {
        d /= 2;
    }
    d
}

/// `BtlKiBlast_ScaleSpeedByAbility`: the uncharged kinds 0, 4, 8 x1.6 (ability 0xE), x1.4 (0xD) or x1.2 (0xC).
fn scale_speed(me: &Chr, kind: i32, speed: f32) -> f32 {
    if !matches!(kind, 0 | 4 | 8) {
        speed
    } else if me.has_ability(0xE) {
        speed * 1.6
    } else if me.has_ability(0xD) {
        speed * 1.4
    } else if me.has_ability(0xC) {
        speed * 1.2
    } else {
        speed
    }
}

/// `BtlKiBlast_ScaleKiByAbility`: doubled (ability 0x2A), then halved (0x29).
fn scale_ki(me: &Chr, mut ki: i32) -> i32 {
    if me.has_ability(0x2A) {
        ki *= 2;
    }
    if me.has_ability(0x29) {
        ki /= 2;
    }
    ki
}

/// `BtlKiBlast_MapReact` / `MapReactS800` (`second`): with ability 0x4B, a charged kind (charge level 1..3) of
/// record type 0 or 1 gives reaction 0x17 / 0x1A plus the level less one.
fn map_react(me: &Chr, rec: &KiBlastRecord, react: i32, kind: i32, second: bool) -> i32 {
    if !me.has_ability(0x4B) || !matches!(rec.kind_type(), 0 | 1) || !(0..12).contains(&kind) {
        return react;
    }
    match kind & 3 {
        0 => react,
        level => (if second { 0x1A } else { 0x17 }) + level - 1,
    }
}

/// `BtlKiBlast_GetKiCostOf`.
pub(crate) fn ki_cost_of(me: &Chr, data: &SideData, kind: i32) -> i32 {
    scale_ki(me, record(data, kind).ki())
}

/// `BtlKiBlast_GetKiCost`.
pub(crate) fn ki_cost(me: &Chr, data: &SideData) -> i32 {
    scale_ki(me, current(me, data).ki())
}

/// `BtlKiBlast_GetHitsOf`.
pub(crate) fn hits_of(data: &SideData, kind: i32) -> i32 {
    i32::from(record(data, kind).hits())
}

/// `BtlKiBlast_GetDamageOf` / `GetGuardDamageOf`: times the owner's curve row 6, its abilities, divided by the
/// hits (2 or more for the plain damage, any for the guarded one).
fn damage_of(owner: &Chr, env: &Env, rec: &KiBlastRecord, guard: bool) -> i32 {
    let base = if guard {
        rec.guard_damage()
    } else {
        rec.damage()
    };
    let mut d = scale_damage(owner, (base as f32 * owner.curve(env, 6)) as i32);
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

/// `BtlParam_GetBlastLimitA`: parameter +0x80 less 3 / 2 / 1 by ability 0x11 / 0x10 / 0xF, at least 1 while the
/// parameter's is positive.
pub(crate) fn blast_limit_a(me: &Chr, data: &SideData) -> i32 {
    let base = i32::from(data.tables.params.u16_at(0x80) as i16);
    let mut v = base;
    if me.has_ability(0x11) {
        v -= 3;
    } else if me.has_ability(0x10) {
        v -= 2;
    } else if me.has_ability(0xF) {
        v -= 1;
    }
    if base > 0 && v <= 0 { 1 } else { v }
}

/// `BtlParam_GetBlastLimitB`.
pub(crate) fn blast_limit_b(data: &SideData) -> i32 {
    i32::from(data.tables.params.u16_at(0x82) as i16)
}

/// The class of a kind (`BtlMove_CanFireBlast`): 0 for 0, 4, 8, else 1.
/// The fighters as the water reads them (`BtlCharApi_*`).
pub(crate) fn water_chars(
    chars: &[Chr; 2],
    data: [&SideData; 2],
) -> [crate::eft_water::WaterChr; 2] {
    std::array::from_fn(|i| crate::eft_water::WaterChr {
        pos: chars[i].pose.pos,
        dir: chars[i].pose.dir,
        speed: chars[i].pose.speed,
        height: data[i].height(),
        in_technique: crate::btl_act::is_technique_id(chars[i].act.current),
    })
}

fn kind_class(kind: i32) -> usize {
    usize::from(!matches!(kind, 0 | 4 | 8))
}

/// `BtlMove_CanFireBlast`: fewer live blasts of the class than the character's limit.
pub(crate) fn can_fire(me: &Chr, data: &SideData, class: usize) -> bool {
    let limit = if class == 0 {
        blast_limit_a(me, data)
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
    crate::btl_obj::node_side(node as i32)
}

impl Scene {
    /// `BtlFx_FireKiBlast`: animation event 4 launches the record's hits along the aim from the event's node, with
    /// sound 1 (types 2, 3) or 0xF / 0x10 by the frame.
    pub(crate) fn fire(
        &mut self,
        me: &mut Chr,
        opp: &Chr,
        data: &SideData,
        rand: &mut u32,
        time_stopped: bool,
        snd: &mut crate::btl_snd::CharSound,
        frame: u32,
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
            speed: scale_speed(me, kind, rec.speed() * 10.0 * kmh(1.0)),
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
            match arg.kind_type {
                5 => self
                    .tech
                    .discs
                    .spawn_from_node(me, &arg, data, &mut self.next_id),
                4 => self.tech.discs.throw(&arg, data, &mut self.next_id),
                // EftKiObj_Create: one object; EftKiBomb_Fire: two bombs.
                2 => self.launch_ki_obj(me, data, arg, false),
                3 => {
                    self.launch_ki_obj(me, data, arg, true);
                    self.launch_ki_obj(me, data, arg, true);
                }
                _ => self.launch(me, data, arg),
            }
            self.defer_disc_draws();
        }
        let sound = match arg.kind_type {
            2 | 3 => 1,
            _ => (frame % 2) as i32 + 0xF,
        };
        me.play_common(snd, sound);
    }

    /// The disc's libc draws, in order with the others.
    fn defer_disc_draws(&mut self) {
        for _ in 0..std::mem::take(&mut self.tech.discs.pending_rand) {
            self.deferred.push(Deferred::Libc);
        }
    }

    /// A thrown object or a bomb in the owner's kind 1 / 2 list, unless the list is full or the character's pack has
    /// no such manager.
    fn launch_ki_obj(&mut self, me: &Chr, data: &SideData, arg: Arg, bomb: bool) {
        let list = usize::from(bomb);
        let has = data
            .effect
            .is_some_and(|p| p.kind_bits & (1 << (list + 1)) != 0);
        let objs = &mut self.ki_objs[arg.owner][list];
        if !has || objs.len() >= LIST_MAX {
            return;
        }
        self.next_id += 1;
        objs.push(KiObj::new(self.next_id, me, arg, bomb));
        self.deferred.push(if bomb {
            Deferred::Bomb(arg.owner, self.next_id)
        } else {
            Deferred::Libc
        });
        if !bomb {
            self.deferred.push(Deferred::Libc);
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
            tag: Tag::default(),
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

    /// `EftStruggle_Step`: ends once either fighter left clash A; else the point follows the clash bias and each
    /// beam's head (its task's reported position) is held its power times its scale times 13 from the point,
    /// toward its start.
    fn step_struggle(&mut self, w: &World) {
        let Some(mut s) = self.struggle else {
            return;
        };
        s.frames += 1;
        if !w.chars.iter().all(crate::btl_clash::Clash::in_clash_a) {
            s.end = true;
        }
        if s.end {
            self.struggle = None;
            return;
        }
        s.bias = ((w.clash_bias + 1.0) * 0.5).clamp(0.0, 1.0);
        s.pos = lerp3(s.side[1].start, s.side[0].start, s.bias);
        for side in s.side {
            let v = scale3(norm(sub(side.start, s.pos)), side.power * side.scale * 13.0);
            if let Some(t) = self.tag_mut(side.task) {
                t.pos = add(s.pos, v);
            }
        }
        self.struggle = Some(s);
    }

    /// `EftStruggle_End`: the loser's technique timeline expires and its effects reset (`BtlScene_Reset` of its
    /// layer), which ends its beam; -1: both.
    pub(crate) fn end_struggle(&mut self, chars: &mut [Chr; 2], winner: i32) {
        let Some(s) = &mut self.struggle else {
            return;
        };
        if s.end {
            return;
        }
        s.end = true;
        let losers: &[usize] = match winner {
            0 => &[1],
            1 => &[0],
            _ => &[0, 1],
        };
        for &l in losers {
            self.tech.request_restart(l, true);
        }
        for &l in losers {
            self.reset_side(chars, l);
        }
    }

    /// `BtlScene_Reset(2 / 3)`: every effect task of a fighter is reset (its technique items end as their term
    /// callbacks do, raising flag 0xA8 / 0xA9); the scene's generator restarts.
    pub(crate) fn reset_side(&mut self, chars: &mut [Chr; 2], side: usize) {
        for list in &mut self.lists[side] {
            list.clear();
        }
        for list in &mut self.ki_objs[side] {
            list.clear();
        }
        self.absorbs[side].reset(&mut self.fx);
        self.shocks[side].reset(&mut self.fx);
        self.rush_bursts[side].reset(&mut self.fx);
        self.charges[side].reset(&mut self.fx);
        self.tech.reset_side(chars, &mut self.fx, side);
    }

    /// `BtlFx_SpawnHitSparkReq17` for a ki blast of type 4: request 0x17 grows a disc in the firing hand (the
    /// other types' charge effects are drawing only).
    pub(crate) fn spawn_held_disc(&mut self, me: &Chr, data: &SideData) {
        if !me.test_fx(0x17) {
            return;
        }
        let kind_type = if me.gauge.body_changed {
            1
        } else {
            current(me, data).kind_type()
        };
        if kind_type == 4 {
            self.tech.discs.spawn_held(me, data, &mut self.next_id);
            self.defer_disc_draws();
        }
    }

    /// `BtlChar_SpawnFxBits3C`: the technique shot the fighter's effect bits ask for.
    pub(crate) fn request_technique(
        &mut self,
        me: &Chr,
        opp: &Chr,
        data: &SideData,
        time_stopped: bool,
    ) {
        self.tech
            .request(me, opp, data, &mut self.next_id, time_stopped);
    }

    /// `BtlScene_Update`, `EftDet_Update` and `BtlScene_PostUpdate` for the technique effects and the ki blasts;
    /// then every fighter's live blast counts for `BtlMove_CanFireBlast`, and the technique timelines' timers
    /// (`BtlScene_Draw`).
    pub(crate) fn update(&mut self, w: &mut World) {
        // EftHit_BeginFrame: the records are kept while time is stopped.
        if !w.paused {
            self.records.clear();
        }
        // The generator draws of the discs, objects and bombs the fighters created.
        for d in std::mem::take(&mut self.deferred) {
            match d {
                Deferred::Libc => {
                    w.libc.rand();
                }
                Deferred::Bomb(side, id) => {
                    if let Some(o) = self.ki_objs[side][1].iter_mut().find(|o| o.id == id) {
                        o.jitter(w.vu0);
                    }
                    w.libc.rand();
                    w.libc.rand();
                }
            }
        }
        // Layer 0: the stage effects in their table's order.
        self.stage.update(w.chars, w.data, w.paused, w.libc, w.vu0);
        // Layer 1: the technique effects.
        self.tech.update(
            w,
            &mut self.fx,
            &mut self.records,
            HIT_MAX,
            &mut self.next_id,
        );
        for dir in self.tech.tints.drain(..) {
            self.tints.start(0, dir, 0.2);
        }
        // The techniques' last setting of view 0's blur (EftBlast_UpdateBlur and the others).
        if let Some((alphas, dir)) = self.tech.blur.take() {
            for (i, a) in alphas.into_iter().enumerate() {
                self.blurs.set_color(0, i, [0x80, 0x80, 0x80, a]);
            }
            self.blurs.set_color(0, 3, [0x80, 0x80, 0x80, 0x40]);
            self.blurs.set_center(0, dir, false);
        }
        // The sweeps' marks that ran out start their blast impacts.
        for (arg, scale) in self.tech.blast_impacts.drain(..) {
            self.impacts.spawn_blast(w.env.impact, &arg, scale, 1.0);
        }
        // BtlScene_IsAnyCharStopped.
        let stopped = w.paused || w.chars.iter().any(Chr::is_frozen);
        for side in 0..2 {
            // Kind 0, the charge aura.
            if let Some(set) = charge_set(w.data[side])
                && let Some(assets) = w.data[side].charge_assets
            {
                let ctx = spawn_ctx(w, side, set, assets, crate::eft_charge::LIST);
                let chars = &*w.chars;
                let node_pos = |n| chars[side].obj.node_pos(n);
                self.charges[side].update(
                    &mut self.fx,
                    &mut self.warps,
                    side,
                    &node_pos,
                    &ctx,
                    w.paused,
                    w.libc,
                    w.vu0,
                );
            }
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
                // The thrown objects / bombs in this list's place.
                if !stopped {
                    let opp = &w.chars[side ^ 1];
                    let opp_pos = add(opp.pose.pos, opp.pose.disp);
                    let bottom = w.env.stage.bounds.bottom;
                    let mut i = 0;
                    while i < self.ki_objs[side][list].len() {
                        let o = &mut self.ki_objs[side][list][i];
                        let rec = o.update(opp_pos, bottom, w.libc, w.paused, &mut w.env.snd());
                        if let Some(pos) = o.exploded.take() {
                            let arg = crate::eft_impact::ImpactArg {
                                pos,
                                dir: [0.0, -1.0, 0.0],
                                id: 1,
                                chr: o.arg.owner,
                            };
                            self.impacts.spawn_blast(w.env.impact, &arg, 1.0, 1.0);
                        }
                        if let Some(rec) = rec
                            && self.records.len() < HIT_MAX
                        {
                            self.records.push(rec);
                        }
                        if o.dead() {
                            self.ki_objs[side][list].remove(i);
                        } else {
                            i += 1;
                        }
                    }
                }
            }
            // Kind 4, the shock.
            if let Some(pack) = shock_pack(w.data[side]) {
                let me = &w.chars[side];
                let c = crate::eft_shock::ShockChr {
                    side,
                    pos: me.obj.node_pos(3),
                    dir: me.pose.dir,
                    scale: w.data[side].height() / 19.35,
                };
                self.shocks[side].update(&mut self.fx, &mut self.warps, &c, pack, w.paused, w.vu0);
            }
            // Kind 5, the rush-finish burst.
            if let Some(set) = rush_burst_set(w.data[side])
                && let Some(assets) = w.data[side].rush_burst_assets
            {
                let ctx = spawn_ctx(w, side, set, assets, crate::eft_rushburst::LIST);
                let chars = &*w.chars;
                let node_pos = |n| chars[side].obj.node_pos(n);
                self.rush_bursts[side].update(
                    &mut self.fx,
                    &node_pos,
                    &ctx,
                    w.paused,
                    w.libc,
                    w.vu0,
                );
            }
            // Kind 9, the drain glow (the other kinds are not ported).
            if let Some((_, ctx)) = absorb_ctx(w, side) {
                let chars = &*w.chars;
                let node_pos = |n| chars[side].obj.node_pos(n);
                let c = crate::eft_absorb::AbsorbChr {
                    node_pos: &node_pos,
                    event_400: chars[side].anim_test_attr(0x400),
                };
                self.absorbs[side].update(&mut self.fx, &c, &ctx, w.paused, w.libc, w.vu0);
            }
        }
        // Layer 3 (`gEftLayer3Classes`): the ground dust (BtlScene_IsTimeStopped: paused), the aura, the body
        // lightning, the glow, the
        // speed lines, the
        // vanish lines, the struggle, the discs, the impacts, the blast objects, then the particle modules.
        let dust_chars = std::array::from_fn(|i| crate::btl_mgr::dust_chr(&w.chars[i], w.data[i]));
        self.gdust
            .update(w.paused, Some(&dust_chars), w.libc, w.vu0);
        if let Some(cfg) = w.env.aura {
            let chars = &*w.chars;
            let node_pos = [0, 1].map(|s| move |n: u32| chars[s].obj.node_pos(n));
            let ac = std::array::from_fn(|s| {
                crate::btl_fx::aura_chr(&chars[s], w.data[s], &node_pos[s])
            });
            self.auras.update(cfg, &ac, w.paused, w.libc);
        }
        if let Some(cfg) = w.env.bolt {
            let chars = &*w.chars;
            let node_pos = [0, 1].map(|s| move |n: u32| chars[s].obj.node_pos(n));
            let bc = std::array::from_fn(|s| crate::eft_bolt::BoltChr {
                side: s,
                height: w.data[s].height(),
                param_flags: u32::from(w.data[s].tables.params.chara_flags()),
                model_new: chars[s].flags.test(0x12B),
                aura_visible: self.auras.is_visible(s),
                burst_rising: self.auras.is_burst_rising(s),
                glow_active: self.glows.is_active(s),
                node_pos: &node_pos[s],
            });
            // EftAuraMgr_Update's part (the aura manager runs before its tasks; nothing of it reads them).
            self.bolts.manage(&bc);
            self.bolts.update(cfg, &bc, w.paused, w.libc);
        }
        if let Some(cfg) = w.env.glow {
            let chars = &*w.chars;
            let node_pos = [0, 1].map(|s| move |n: u32| chars[s].obj.node_pos(n));
            let gc = std::array::from_fn(|s| {
                crate::btl_fx::glow_chr(&chars[s], w.data[s], &node_pos[s])
            });
            self.glows.update(cfg, &gc, w.paused, w.libc);
        }
        let spd = std::array::from_fn(|i| crate::eft_spdline::SpdChr {
            height: w.data[i].height(),
            moved: w.chars[i].pose.moved,
        });
        self.spdlines.update(w.paused, &spd, w.libc);
        let chars = &*w.chars;
        let (n0, n1) = (|n| chars[0].obj.node_pos(n), |n| chars[1].obj.node_pos(n));
        let shot = [
            crate::eft_shotfx::ShotChr {
                height: w.data[0].height(),
                node_pos: &n0,
            },
            crate::eft_shotfx::ShotChr {
                height: w.data[1].height(),
                node_pos: &n1,
            },
        ];
        let tables = w.env.fx;
        self.shotfx
            .update(w.paused, &shot, &tables.shotfx, &tables.sin_coef, w.libc);
        if !w.paused {
            self.step_struggle(w);
        }
        self.tech
            .update_discs(w, &mut self.fx, &mut self.records, HIT_MAX);
        // What EftEmit_Spawn reads of each fighter, taken first.
        let scale = [0, 1].map(|s| w.data[s].height() / 19.35);
        let flags = [0, 1].map(|s| w.data[s].tables.params.chara_flags());
        let node3 = [0, 1].map(|s| w.chars[s].obj.node_pos(3));
        let impact_sets = w.env.impact;
        let transform_sets = w.env.transform;
        // EftTransform_Update, then the fighters' colour modes it asked for.
        {
            let chars = &*w.chars;
            let pos = [0, 1].map(|s| move |n: u32| chars[s].obj.node_pos(n));
            let z = [0, 1].map(|s| {
                move |n: u32| {
                    chars[s]
                        .obj
                        .node(n)
                        .map_or([0.0; 3], |m| [m.mtx[2][0], m.mtx[2][1], m.mtx[2][2]])
                }
            });
            let attr = [0, 1].map(|s| move |m: u64| chars[s].anim_test_attr(m));
            let attr_node = [0, 1].map(|s| move |m: u64| attr_kind(&chars[s], m));
            let tc = std::array::from_fn(|s| crate::eft_transform::TransformChr {
                side: s,
                height: w.data[s].height(),
                node_pos: &pos[s],
                node_z: &z[s],
                attr: &attr[s],
                attr_node: &attr_node[s],
            });
            self.transforms.update(
                &mut self.fx,
                &mut self.warps,
                transform_sets,
                &tc,
                w.paused,
                |chr, k, s| crate::eft::SpawnCtx {
                    chr,
                    set: &s.set,
                    assets: &s.assets,
                    set_ref: SetRef {
                        side: chr,
                        list: crate::eft_transform::FIRST_LIST + k,
                    },
                    char_scale: [scale[chr], scale[chr ^ 1]],
                    opp_chara_flags: flags[chr ^ 1],
                    opp_node3: node3[chr ^ 1],
                },
                w.libc,
                w.vu0,
            );
        }
        // The flash: the stage tint's slot 1 (white) at once and the silhouette; the reveal undoes both.
        for r in &self.transforms.model_requests {
            w.chars[r.chr].obj.set_color_mode(3, r.silhouette);
            self.tints.start(1, usize::from(!r.silhouette), 0.0);
        }
        // EftImpact_Update.
        self.impacts.update(
            &mut self.fx,
            impact_sets,
            w.paused,
            |chr, k, s| crate::eft::SpawnCtx {
                chr,
                set: &s.set,
                assets: &s.assets,
                set_ref: SetRef {
                    side: chr,
                    list: crate::eft_impact::FIRST_LIST + k,
                },
                char_scale: [scale[chr], scale[chr ^ 1]],
                opp_chara_flags: flags[chr ^ 1],
                opp_node3: node3[chr ^ 1],
            },
            w.libc,
            w.vu0,
        );
        self.tech
            .update_shots(w, &mut self.fx, &mut self.records, HIT_MAX);
        let fxw = FxWorld {
            sets: std::array::from_fn(|s| {
                world_sets(w.data[s].fx_sets(), impact_sets, transform_sets)
            }),
            any_frozen: stopped,
            stopped: [0, 1].map(|s| crate::btl_tech_fx::stopped(w, s)),
            // The update does not draw.
            clash_a: [false; 2],
            node3: [[0.0; 3]; 2],
            ray_texture: None,
            paused: w.paused,
            in_technique: [0, 1].map(|s| crate::btl_act::is_technique_id(w.chars[s].act.current)),
        };
        self.fx.update(&fxw, w.libc, w.vu0);
        // Layer 3's blinding overlay and full-screen flash.
        let fc = std::array::from_fn(|s| {
            let c = &w.chars[s];
            crate::eft_flash::FlashChr {
                flag8_action_104: c.flags.test(8) && c.act.current == 0x104,
                variant: c.gauge.variant != 0,
                in_rush: crate::btl_tech_fx::in_rush_sequence(c),
                finish_phase: crate::btl_tech_fx::rush_finish_phase(c),
                flag_2b: c.flags.test(0x2B),
                blind: (c.blind_level as f32 / 90.0).clamp(0.0, 1.0),
            }
        });
        self.flashes.update(&fc, w.paused, w.paused);
        self.clamp_to_stage(w.env);
        // EftDet_Update: nothing while paused.
        if !w.paused {
            self.prepare(w.env);
            self.clash_pass(w);
            self.stage_pass(w.env, w.data, w.frame);
            self.fighter_pass(w);
        }
        // BtlScene_PostUpdate: EftHit_UpdateResults, then layer 1's, layer 2's and layer 3's post-updates.
        self.update_results();
        self.tech.post_update(w, &mut self.fx);
        for side in 0..2 {
            if let Some(set) = charge_set(w.data[side]) {
                self.charges[side].post_update(set, &mut self.fx);
            }
            for list in 0..2 {
                for task in &mut self.lists[side][list] {
                    task_post_update(task, w, &mut self.fx);
                }
                for k in 0..self.ki_objs[side][list].len() {
                    if list == 1 {
                        self.ki_objs[side][list][k].post_bomb(w.libc);
                    } else {
                        let mut o = self.ki_objs[side][list][k].clone();
                        o.post_obj_with(w);
                        self.ki_objs[side][list][k] = o;
                    }
                }
            }
            if let Some(set) = rush_burst_set(w.data[side]) {
                self.rush_bursts[side].post_update(set, &mut self.fx);
            }
            if let Some(set) = absorb_set(w.data[side]) {
                self.absorbs[side].post_update(set, &mut self.fx);
            }
        }
        self.tech.discs.post_update(w, &mut self.fx);
        if !transform_sets.is_empty() {
            self.transforms.post_update(transform_sets, &mut self.fx);
        }
        self.impacts
            .post_update(&mut self.fx, impact_sets, w.paused);
        self.tech.post_update_shots(w, &mut self.fx);
        self.update_records(w);
        self.spawn_impacts(w);
        for (i, chr) in w.chars.iter_mut().enumerate() {
            chr.live_blasts = [0; 2];
            for rec in self
                .records
                .iter()
                .filter(|r| r.tech.is_none() && !r.unk54 && r.atk.src == i)
            {
                chr.live_blasts[kind_class(rec.atk.kind)] += 1;
            }
        }
        self.tech.draw(w);
    }

    /// `EftHit_SpawnImpacts` (not while time is stopped): once per task, a record whose task hit something (not
    /// absorbed, nor kept from it by a finishing stage change) leaves its impact. A ki blast's
    /// (`EftHit_SpawnBlastImpact`): blast impact 0 (1 at 0.6 for a blast of type 1) where it was hit, along its
    /// movement, rising 0.2 on a fighter, 1 on the stage, 0.5 otherwise; a thrown object's raises dust on a fighter
    /// or the stage. A technique's (`EftHit_SpawnTechImpact`): its fighter impact on a hit (a multi-hit's on the
    /// first one, a rush's at the opponent's node 0x11 every time), its stage impact on the stage unless the owner
    /// has flag 0xA4, against the movement.
    fn spawn_impacts(&mut self, w: &mut World) {
        if w.paused {
            return;
        }
        for i in 0..self.records.len() {
            let rec = self.records[i];
            let Some(t) = self.rec_tag(i) else {
                continue;
            };
            let f = t.flags;
            if f & T_IMPACT_DONE != 0
                || rec.flags & H_NO_IMPACT != 0
                || f & T_ABSORBED != 0
                || f & T_END == 0
            {
                continue;
            }
            t.flags |= T_IMPACT_DONE;
            let (at, hits) = (t.pos, t.hit_count);
            let sets = w.env.impact;
            let owner = rec.owner;
            match rec.tech {
                None => {
                    let mut arg = crate::eft_impact::ImpactArg {
                        pos: at,
                        dir: norm(sub(rec.pos, rec.prev)),
                        id: 0,
                        chr: owner,
                    };
                    let mut scale = 1.0;
                    if rec.atk.kind_type == 1 {
                        arg.id = 1;
                        scale = 0.6;
                    }
                    let dust = |s: &mut Self, w: &mut World| {
                        let coef = w.env.fx.sin_coef;
                        s.gdust.spawn_impact(owner, at, 1.0, w.libc, w.vu0, &coef);
                    };
                    if f & T_HIT_CHAR != 0 {
                        if rec.flags & H_GROUND != 0 {
                            dust(self, w);
                        } else {
                            self.impacts.spawn_blast(sets, &arg, scale, 0.2);
                        }
                    } else if f & T_HIT_STAGE != 0 {
                        if rec.flags & H_GROUND == 0 {
                            self.impacts.spawn_blast(sets, &arg, scale, 1.0);
                        } else {
                            dust(self, w);
                        }
                    } else {
                        self.impacts.spawn_blast(sets, &arg, 1.0, 0.5);
                    }
                }
                Some(src) => {
                    let def = &src.def;
                    let dir = norm(sub(rec.prev, rec.pos));
                    if f & (T_HIT_CHAR | T_GUARDED) != 0 {
                        if def.impact_fx >= 0 {
                            let rush = if def.kind == 0 {
                                def.sub == 4
                            } else {
                                def.sub == 9
                            };
                            let pos = if f & T_MULTI == 0 || (!rush && hits == 1) {
                                Some(at)
                            } else if rush {
                                Some(w.chars[owner ^ 1].obj.node_pos(0x11))
                            } else {
                                None
                            };
                            if let Some(pos) = pos {
                                let arg = crate::eft_impact::ImpactArg {
                                    pos,
                                    dir,
                                    id: def.impact_fx,
                                    chr: owner,
                                };
                                self.impacts.spawn_blast(sets, &arg, def.impact_scale, 0.5);
                            }
                        }
                    } else if f & T_HIT_STAGE != 0
                        && def.ground_fx >= 0
                        && !w.chars[owner].flags.test(0xA4)
                    {
                        let arg = crate::eft_impact::ImpactArg {
                            pos: at,
                            dir,
                            id: def.ground_fx,
                            chr: owner,
                        };
                        self.impacts.spawn_blast(sets, &arg, def.ground_scale, 1.0);
                    }
                }
            }
        }
    }

    /// `BtlScene_UpdateRecords`: `EftWater_UpdateBlast` on every record (`EftBubble_OnBlastRecord` does nothing).
    fn update_records(&mut self, w: &mut World) {
        if self.stage.water.is_none() {
            return;
        }
        for i in 0..self.records.len() {
            let (flags, pos) = self.rec_tag(i).map_or((0, [0.0; 3]), |t| (t.flags, t.pos));
            let r = &self.records[i];
            let rec = crate::eft_water::WaterRec {
                owner: r.owner,
                technique: r.tech.is_some(),
                def_sub: r.tech.map_or(0, |t| t.def.sub),
                def_id: r.tech.map_or(0, |t| t.def.id),
                pos: r.pos,
                prev: r.prev,
                no_water: r.flags & H_KIND9 != 0,
                task_flags: flags,
                task_pos: pos,
                // The shape's third word: the definition's or the blast's speed.
                speed: r.tech.map_or(r.atk.speed, |t| t.def.speed),
                radius_a: r.shape.a.radius,
            };
            let height = w.data.get(r.owner).map_or(10.0, |d| d.height());
            let Some(water) = &mut self.stage.water else {
                return;
            };
            let (set, sound) = water.update_blast(&rec, height, w.paused, w.libc);
            if set != 0
                && let Some(t) = self.rec_tag(i)
            {
                t.flags |= set;
            }
            if let Some((at, id)) = sound {
                // BtlCharApi_PlaySoundAt: no owner, the common bank.
                w.env.snd().request(at, -1, 0, 200.0, 1500.0, id);
            }
        }
    }

    /// The tag of a record's task: a ki blast or a technique item.
    fn tag_mut(&mut self, id: u32) -> Option<&mut Tag> {
        if let Some(t) = self
            .lists
            .iter_mut()
            .flatten()
            .flatten()
            .find(|t| t.id == id)
        {
            return Some(&mut t.tag);
        }
        if let Some(o) = self
            .ki_objs
            .iter_mut()
            .flatten()
            .flatten()
            .find(|o| o.id == id)
        {
            return Some(&mut o.tag);
        }
        self.tech.tag_mut(id)
    }

    fn rec_tag(&mut self, i: usize) -> Option<&mut Tag> {
        let id = self.records[i].task;
        self.tag_mut(id)
    }

    /// `EftHit_SetTaskFlag`.
    fn set_task_flag(&mut self, i: usize, bits: u32, pos: [f32; 3]) {
        if let Some(t) = self.rec_tag(i) {
            t.flags |= bits;
            t.pos = pos;
        }
    }

    /// `EftHit_GetTaskFlags`.
    pub(crate) fn task_flags(&mut self, i: usize) -> u32 {
        self.rec_tag(i).map_or(0, |t| t.flags)
    }

    /// `EftHit_CanHit`: whether a record may still meet the stage (mode 0), a fighter (1) or another record (2).
    fn can_hit(&mut self, i: usize, mode: u32) -> bool {
        let flags = self.task_flags(i);
        let rflags = self.records[i].flags;
        if mode == 0 {
            if rflags & H_ALWAYS != 0 {
                return self.records[i].tech.is_some() || flags & T_DEFLECTED == 0;
            }
            if rflags & H_NEVER != 0 {
                return false;
            }
        }
        let Some(src) = self.records[i].tech else {
            let ok = if mode == 0 {
                flags & (T_KEEP | T_HIT_STAGE) == 0
            } else {
                flags & (T_HIT_CHAR | T_LOST_CLASH) == 0
            };
            return ok && flags & T_DEFLECTED == 0;
        };
        let def = src.def;
        match mode {
            0 => {
                if flags & T_KEEP != 0 {
                    false
                } else if def.kind != 0 {
                    flags & (T_STRUGGLE | T_HIT_STAGE) == 0
                } else {
                    def.sub == 4
                }
            }
            1 => {
                if self.records[i].flags & H_NO_MODE1 != 0 || flags & T_NO_MODE1 != 0 {
                    false
                } else if flags & T_MULTI != 0 {
                    true
                } else {
                    flags & (T_STRUGGLE | T_LOST_CLASH | T_HIT_CHAR) == 0
                }
            }
            _ => {
                flags & T_NO_MODE2 == 0
                    && flags & (T_STRUGGLE | T_LOST_CLASH | T_HIT_STAGE | T_HIT_CHAR) == 0
            }
        }
    }

    /// `EftHit_IsMultiHit`.
    fn is_multi(&mut self, i: usize) -> bool {
        self.task_flags(i) & T_MULTI != 0
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
                if let Some(t) = self.rec_tag(i) {
                    t.pos = pos;
                    t.flags |= T_OUT;
                }
                let s = &mut self.records[i].shape;
                if s.kind == 0 {
                    s.a.pos = pos;
                } else {
                    s.a_start = pos;
                }
            }
        }
    }

    /// `EftDet_PrepareAll` with `EftDet_PrepareSpheres` / `EftDet_PrepareCapsule`.
    fn prepare(&mut self, env: &Env) {
        for rec in &mut self.records {
            let s = &mut rec.shape;
            s.hit_flags = 0;
            s.stage_dist = f32::MAX;
            s.dist = f32::MAX;
            let start = if s.kind == 0 { s.b.pos } else { s.a_start };
            s.bounds = capsule_bounds(start, s.a.pos, s.a.radius);
            s.delta = sub(s.a.pos, start);
            s.dir = norm(s.delta);
            s.zone = env.col.find_zone_near(s.zone as i32, s.a.pos);
        }
    }

    /// `EftHit_Clash`: 1 record `j` wins, 2 record `i` wins, 3 both lose, 4 a beam struggle. A ki blast loses to a
    /// technique; two blasts compare levels; two techniques first try `EftHit_ClashTech`, which also stops both
    /// fighters' technique timers on a head-on pair.
    fn clash(&mut self, i: usize, j: usize) -> i32 {
        let (a, b) = (self.records[i], self.records[j]);
        let level = || {
            if a.level > b.level {
                2
            } else if a.level < b.level {
                1
            } else {
                3
            }
        };
        match (a.tech, b.tech) {
            (None, None) => level(),
            (None, Some(_)) => 1,
            (Some(_), None) => 2,
            (Some(sa), Some(sb)) => {
                let r = clash_tech(&a, &b, &sa, &sb);
                if r == 4 {
                    self.tech.request_stop(0);
                    self.tech.request_stop(1);
                }
                if r == 3 { level() } else { r }
            }
        }
    }

    /// `EftDet_ClashPass`: every pair of records of different owners; the shapes are swept against each other
    /// (spheres, capsules from their muzzles, or one of each); the clash is decided for every pair and used when
    /// they touch: the loser stops (task flag 8). A beam struggle with no sight (flag 0xBA) or late in the battle
    /// sequence (state 4 on) stops both; else both fighters get held flag 0xAA and, unless a struggle runs, both
    /// records flag 0x100 and the struggle starts (`EftStruggle_Start`).
    fn clash_pass(&mut self, w: &mut World) {
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
                // EftDet_SpheresVsSpheres / CapsuleVsCapsule / CapsuleVsSpheres (the capsule first).
                let touch = match (a.kind, b.kind) {
                    (0, 0) | (1, 1) | (1, 0) => spheres_vs_spheres(&a, &b),
                    (0, 1) => spheres_vs_spheres(&b, &a).map(|(hb, ha)| (ha, hb)),
                    _ => None,
                };
                let result = self.clash(i, j);
                let Some((ha, hb)) = touch else {
                    continue;
                };
                self.records[i].shape.hit_pos = ha.0;
                self.records[i].shape.contact = ha.1;
                self.records[j].shape.hit_pos = hb.0;
                self.records[j].shape.contact = hb.1;
                self.records[i].shape.hit_flags |= D_CLASH;
                let (pi, pj) = (ha.0, hb.0);
                if result & 1 != 0 {
                    self.set_task_flag(i, T_LOST_CLASH, pi);
                }
                if result & 2 != 0 {
                    self.set_task_flag(j, T_LOST_CLASH, pj);
                }
                if result & 4 != 0 {
                    let owner = self.records[i].owner;
                    if w.chars[owner].flags.test(0xBA) || w.env.seq_state >= 4 {
                        self.set_task_flag(i, T_LOST_CLASH, pi);
                        self.set_task_flag(j, T_LOST_CLASH, pj);
                    } else {
                        w.chars[0].flags.set_held(0xAA);
                        w.chars[1].flags.set_held(0xAA);
                        if self.struggle.is_none() {
                            self.set_task_flag(i, T_STRUGGLE, pi);
                            self.set_task_flag(j, T_STRUGGLE, pj);
                            let (a, b) = (self.records[i], self.records[j]);
                            self.struggle = Struggle::start(&a, &b, w.clash_bias);
                        }
                    }
                }
            }
        }
    }

    /// `EftDet_StagePass` with `EftDet_TestStage`: the swept volume (a sphere of the radius times the record's
    /// scale, at least 0.08, from last frame's centre or the tail to the head) against the zone's triangles; the
    /// nearest contact stops the record (task flag 4) unless it is a technique that goes through
    /// (`EftHit_IsStoppedByHit`); a technique's stop shakes the cameras near it. A rush technique's record meets
    /// only the standing stage objects (its sphere not scaled). A standing object the walk met takes damage (1 from
    /// a ki blast, 999999 from a technique); when that breaks it, the record goes on as if nothing was hit.
    fn stage_pass(&mut self, env: &Env, data: [&SideData; 2], frame: u32) {
        for i in 0..self.records.len() {
            if !self.can_hit(i, 0) {
                continue;
            }
            let tech = self.records[i].tech;
            let objects_only = tech.is_some_and(|t| {
                if t.def.kind == 0 {
                    t.def.sub == 4
                } else {
                    t.def.sub == 9
                }
            });
            let s = &mut self.records[i].shape;
            let r = if s.kind == 0 && objects_only {
                s.a.radius
            } else {
                s.a.radius * s.scale
            }
            .max(0.08);
            let sweep = Sweep::new(s.b.pos, s.a.pos, r);
            let bounds = capsule_bounds(s.b.pos, s.a.pos, r);
            let mut best = if s.kind == 0 {
                sweep.length.max(r) + 0.001
            } else {
                sweep.length + 0.001
            };
            let mut found = None;
            let (_, mask) = env.col.query_zone(
                s.zone,
                &bounds,
                env.broken(),
                objects_only,
                &mut |poly: &ColPoly, v: [[f32; 3]; 3]| {
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
                            found = Some((hit, contact, n));
                            true
                        }
                        _ => false,
                    }
                },
            );
            let Some((hit, contact, nrm)) = found else {
                continue;
            };
            s.hit_pos = hit;
            s.contact = contact;
            if mask != 0 {
                let idx = mask.trailing_zeros() as usize;
                let mut objs = env.objs();
                if objs.damage(idx, if tech.is_none() { 1 } else { 999_999 }) {
                    objs.destroy(idx, sub(s.a.pos, s.b.pos), &mut env.snd());
                    continue;
                }
            }
            s.hit_flags |= D_STAGE;
            let from = if s.kind == 0 { s.b.pos } else { s.a_start };
            s.stage_dist = length(sub(from, hit));
            // EftHit_IsStoppedByHit and EftHit_NotifyBlastTask (a bomb keeps the triangle's normal), the contact
            // sound (BtlColl_PlayContactSound) and a technique's camera shake; the vibration is not ported.
            let stops = tech.is_none_or(|t| {
                if t.def.kind == 0 {
                    t.def.sub != 4
                } else {
                    !matches!(t.def.sub, 7 | 9)
                }
            });
            if stops {
                let rec = self.records[i];
                contact_sound(&rec, env, data, frame);
                if rec.tech.is_some() {
                    env.objs().shakes.push(crate::stg_obj::Shake {
                        pos: rec.pos,
                        near: 100.0,
                        far: 1500.0,
                        strength: 3.0,
                        time: 0.5,
                    });
                }
                self.set_task_flag(i, T_HIT_STAGE, contact);
                let id = self.records[i].task;
                if let Some(o) = self
                    .ki_objs
                    .iter_mut()
                    .flatten()
                    .flatten()
                    .find(|o| o.id == id)
                {
                    o.contact_nrm = nrm;
                }
            }
        }
    }

    /// `EftHit_MarkLastHit`: a technique's record is its last hit: a single hit (not of a rush kind 4), or a
    /// multi-hit that has landed its last (or any hit with flag 0x20).
    fn mark_last_hit(&mut self, i: usize) {
        let rec = self.records[i];
        let Some(src) = rec.tech else {
            return;
        };
        if rec.flags & H_NO_LAST != 0 {
            return;
        }
        let last = if self.is_multi(i) {
            let hits = self.rec_tag(i).map_or(0, |t| t.hit_count);
            hits > 0 && (rec.def_flag20() || hits >= rec.max_hits())
        } else {
            src.def.sub != 4
        };
        if last {
            self.records[i].flags |= H_LAST;
        }
    }

    /// `EftDet_FighterPass`: each record against the fighter it may hit (owner ^ 1). A contact behind the stage
    /// contact is dropped; then the fighter dodges, deflects, reflects or absorbs, or a multi-hit counts its frames
    /// in contact, else it guards or is hit.
    fn fighter_pass(&mut self, w: &mut World) {
        for i in 0..self.records.len() {
            if !self.can_hit(i, 1) {
                continue;
            }
            let target = self.records[i].owner ^ 1;
            let shape = self.records[i].shape;
            // EftDet_SpheresVsFighter / CapsuleVsFighter.
            let found = match shape.kind {
                0 | 1 => spheres_vs_fighter(&w.chars[target], &shape),
                _ => None,
            };
            let Some((hit_pos, contact, dist)) = found else {
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
            self.mark_last_hit(i);
            let rec = self.records[i];
            let contact = rec.shape.contact;
            let hit_count = self.rec_tag(i).map_or(0, |t| t.hit_count);
            if try_dodge(w, target, &rec, hit_count) {
                continue;
            }
            if try_deflect(w, target, &rec) {
                self.set_task_flag(i, T_DEFLECTED, contact);
            } else if try_reflect(w, target, &rec) {
                self.set_task_flag(i, T_REFLECTED, contact);
            } else if try_absorb(w, target, &rec) {
                self.set_task_flag(i, T_ABSORBED, contact);
            } else if self.is_multi(i) {
                self.hit_fighter_multi(w, i, target);
            } else if try_guard(w, target, &rec) {
                self.rearm_impact(i);
                self.set_task_flag(i, T_GUARDED, contact);
            } else if hit(w, target, &rec) {
                self.rearm_impact(i);
                self.set_task_flag(i, T_HIT_CHAR, contact);
            }
        }
    }

    /// `EftHit_RearmImpact`: a multi-hit technique may spawn its impact again.
    fn rearm_impact(&mut self, i: usize) {
        if self.records[i].tech.is_some()
            && let Some(t) = self.rec_tag(i)
            && t.flags & T_MULTI != 0
        {
            t.flags &= !T_IMPACT_DONE;
        }
    }

    /// `EftDet_HitFighterMulti`: the task counts its frames in contact; past its hit limit it reports a hit; between
    /// two hits (fewer frames than the interval since the last) it reports the contact (0x8000, not with flag
    /// 0x20); else the fighter guards or is hit, and the hit is counted.
    fn hit_fighter_multi(&mut self, w: &mut World, i: usize, target: usize) {
        let rec = self.records[i];
        let contact = rec.shape.contact;
        let interval = rec.tech.map_or(0, |t| t.def.hit_interval);
        let Some((frames, hits)) = self.rec_tag(i).map(|t| {
            t.count_a += 1;
            (t.count_a, t.hit_count)
        }) else {
            return;
        };
        if rec.max_hits() < hits {
            self.set_task_flag(i, T_HIT_CHAR, contact);
            return;
        }
        let bits = if hits > 0 && frames < interval {
            0
        } else if try_guard(w, target, &rec) {
            T_GUARDED
        } else if hit(w, target, &rec) {
            T_HIT_CHAR
        } else {
            return;
        };
        if bits != 0
            && let Some(t) = self.rec_tag(i)
        {
            t.hit_count += 1;
            t.count_a = 0;
        }
        if !rec.def_flag20() {
            self.set_task_flag(i, T_MULTI_CONTACT, contact);
        }
        if bits != 0 {
            self.rearm_impact(i);
            self.set_task_flag(i, bits, contact);
        }
    }

    /// `EftHit_UpdateResults` / `EftHit_CalcResult`: the result each record's task reads in its post-update.
    fn update_results(&mut self) {
        for i in 0..self.records.len() {
            let rec = self.records[i];
            let Some(t) = self.rec_tag(i) else {
                continue;
            };
            let mut r = 0;
            if t.flags & T_HIT_STAGE != 0 {
                r = 5;
            }
            if t.flags & T_MULTI_CONTACT != 0 {
                r = 1;
                t.flags &= !T_MULTI_CONTACT;
            }
            if t.flags & T_HIT_CHAR != 0 {
                if t.flags & T_MULTI != 0 {
                    if rec.max_hits() < t.hit_count && !rec.def_flag20() {
                        r = 5;
                    }
                } else if rec.def_flag20() {
                    t.flags |= T_KEEP;
                } else {
                    r = 4;
                }
            }
            let f = t.flags;
            if f & T_LOST_CLASH != 0 {
                r = 4;
            }
            if f & T_GUARDED != 0 && f & T_MULTI == 0 {
                r = 5;
            }
            if f & (T_ABSORBED | T_OUT) != 0 {
                r = 5;
            }
            if f & T_STRUGGLE != 0 {
                r = 3;
            }
            if rec.tech.is_some_and(|s| s.def.kind == 0) && f & T_LOST_CLASH == 0 {
                r &= !5;
            }
            t.result = r;
        }
    }
}

/// `EftHit_ClashTech`: two technique records (no skill, the first of hit direction kind 0..2 or 8, neither with
/// flag 0x100 or more than one shot): head-on (a moving toward b's last position against b's movement) is a beam
/// struggle (4), else the one moving more toward the other passes (2: `a` wins, 1: `b`); 3 when the rule does
/// not apply.
fn clash_tech(a: &HitRec, b: &HitRec, sa: &TechSrc, sb: &TechSrc) -> i32 {
    let (da, db) = (&sa.def, &sb.def);
    if da.kind == 0 || db.kind == 0 {
        return 3;
    }
    if !((0..=2).contains(&da.sub) || da.sub == 8) {
        return 3;
    }
    if da.flags & 0x100 != 0 || db.flags & 0x100 != 0 || da.shots >= 2 || db.shots >= 2 {
        return 3;
    }
    let move_a = sub(a.pos, a.prev);
    let move_b = sub(b.pos, b.prev);
    let d = sub(b.prev, a.prev);
    if dot(move_a, d) > 0.0 && dot(move_a, move_b) < 0.0 {
        return 4;
    }
    let t = dot(norm(sub(a.pos, b.pos)), norm(move_a));
    if !(t < dot(norm(sub(b.pos, a.pos)), norm(move_b))) {
        1
    } else {
        2
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
    let mut rec = HitRec::blast(t.arg, t.id, t.pos, t.prev, t.dir, t.radius * t.trail_width);
    rec.turned = t.flags & W_TURNED != 0;
    rec
}

impl HitRec {
    /// `EftHit_Add` of a ki blast record (a blast's or a disc's): two spheres at `pos` and `prev`.
    pub(crate) fn blast(
        atk: Arg,
        task: u32,
        pos: [f32; 3],
        prev: [f32; 3],
        vel: [f32; 3],
        radius: f32,
    ) -> Self {
        let a = Sphere { pos, radius };
        let b = Sphere { pos: prev, radius };
        Self {
            owner: atk.owner,
            level: atk.level,
            pos,
            prev,
            vel,
            turned: false,
            atk,
            seen_by_ai: false,
            origin: pos,
            tech: None,
            flags: 0,
            unk54: false,
            task,
            shape: Shape {
                kind: 0,
                a,
                b,
                a_start: [0.0; 3],
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
}

/// `EftKiBlast_PostUpdate`: an end report stops the blast where it was reported; a deflection or reflection (not
/// after a deflection) turns it from there.
fn task_post_update(t: &mut Task, w: &mut World, fx: &mut Modules) {
    if let Some(set) = list_head(w.data[t.arg.src], t.list) {
        t.state.update_alive(set, fx);
    }
    t.mask = 0;
    if t.tag.flags & T_END != 0 {
        t.life = 0;
        t.flags |= W_ENDING;
        if t.flags & W_STOPPED == 0 {
            t.pos = t.tag.pos;
        }
        spawn_parts(t, w, fx, true);
        if t.flags & W_STOPPED == 0 {
            end_trail(t, w);
        }
        t.flags |= W_STOPPED;
    } else if t.tag.flags & T_TURN != 0 && t.flags & W_DEFLECTED == 0 {
        // EftKiBlast_Turn: the lerp it starts with is overwritten below.
        let by = t.arg.owner ^ 1;
        let dir = deflect_dir(w, by);
        if t.tag.flags & T_DEFLECTED != 0 {
            t.dir = dir;
            t.arg.owner ^= 1;
            t.flags = (t.flags & !W_HOMING) | W_DEFLECTED;
        } else if t.tag.flags & T_REFLECTED != 0 {
            t.dir = dir;
            t.arg.owner ^= 1;
            t.life = t.arg.life;
        }
        t.pos = t.tag.pos;
        spawn_parts(t, w, fx, true);
        end_trail(t, w);
        t.flags |= W_TURNED;
        t.tag.flags &= !T_TURN;
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
    let ctx = spawn_ctx(w, src, set, assets, t.list);
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
                    w.vu0,
                );
            }
        }
    }
}

/// What `EftEmit_Spawn` reads of the owner `side` of the set `list` (`BtlScene_GetCharScale` is the height over
/// 19.35).
pub(crate) fn spawn_ctx<'s>(
    w: &World,
    side: usize,
    set: &'s EmitSet,
    assets: &'s SetAssets,
    list: usize,
) -> SpawnCtx<'s> {
    let opp = side ^ 1;
    SpawnCtx {
        chr: side,
        set,
        assets,
        set_ref: SetRef { side, list },
        char_scale: [w.data[side].height() / 19.35, w.data[opp].height() / 19.35],
        opp_chara_flags: w.data[opp].tables.params.chara_flags(),
        opp_node3: w.chars[opp].obj.node_pos(3),
    }
}

/// The drain glow's set, when the character's pack makes the kind 9 manager (`EftChar_Init`: pack bit 9).
pub(crate) fn absorb_set<'a>(data: &SideData<'a>) -> Option<&'a EmitSet> {
    let pack = data.effect?;
    if pack.kind_bits & (1 << 9) == 0 {
        return None;
    }
    pack.absorb_set.as_ref()
}

/// The charge aura's set, when the character's pack makes the kind 0 manager (pack bit 0).
pub(crate) fn charge_set<'a>(data: &SideData<'a>) -> Option<&'a EmitSet> {
    let pack = data.effect?;
    if pack.kind_bits & 1 == 0 {
        return None;
    }
    pack.charge_set.as_ref()
}

/// The rush-finish burst's set, when the character's pack makes the kind 5 manager (pack bit 5).
pub(crate) fn rush_burst_set<'a>(data: &SideData<'a>) -> Option<&'a EmitSet> {
    let pack = data.effect?;
    if pack.kind_bits & (1 << 5) == 0 {
        return None;
    }
    pack.rush_burst_set.as_ref()
}

/// The shock's animation, when the character's pack makes the kind 4 manager (pack bit 4).
pub(crate) fn shock_pack<'a>(data: &SideData<'a>) -> Option<&'a bt3_data::eft_pack::SprPack> {
    let pack = data.effect?;
    if pack.kind_bits & (1 << 4) == 0 {
        return None;
    }
    data.shock_assets?.spr.first()?.as_ref()
}

/// The drain glow's set and what `EftEmit_Spawn` reads of its fighter.
fn absorb_ctx<'d>(w: &World<'_, 'd>, side: usize) -> Option<(&'d EmitSet, SpawnCtx<'d>)> {
    let data = w.data[side];
    let set = absorb_set(data)?;
    let assets = data.absorb_assets?;
    Some((
        set,
        spawn_ctx(w, side, set, assets, crate::eft_absorb::LIST),
    ))
}

/// A side's emitter sets with the impacts' and the transformation's common sets after them.
pub(crate) fn world_sets<'a>(
    mut own: [Option<&'a SetAssets>; crate::eft::SET_LISTS],
    impact: &'a [crate::eft_impact::ImpactSet],
    transform: &'a [crate::eft_impact::ImpactSet],
) -> [Option<&'a SetAssets>; crate::eft::SET_LISTS] {
    for (k, s) in impact.iter().enumerate() {
        own[crate::eft_impact::FIRST_LIST + k] = Some(&s.assets);
    }
    for (k, s) in transform.iter().enumerate() {
        own[crate::eft_transform::FIRST_LIST + k] = Some(&s.assets);
    }
    own
}

/// `BtlCharApi_ObjGetAttrKind`: the node of an animation event's value, 0 without one.
fn attr_kind(chr: &Chr, mask: u64) -> u32 {
    let bits = chr.obj.event_arg(mask);
    if bits == 0 {
        0
    } else {
        mask_to_node(bits) as u32
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
pub(crate) fn home(
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
pub(crate) fn deflect_dir(w: &mut World, by: usize) -> [f32; 3] {
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

/// The sphere a shape sweeps along its `delta`: last frame's of two spheres, the one at the muzzle of a capsule.
fn swept(s: &Shape) -> Sphere {
    if s.kind == 0 {
        s.b
    } else {
        Sphere {
            pos: s.a_start,
            radius: s.a.radius,
        }
    }
}

/// `EftDet_SpheresVsSpheres` (and `CapsuleVsCapsule` / `CapsuleVsSpheres`): the swept spheres of both records
/// over the frame. Each record's centre and contact point.
#[allow(clippy::type_complexity)]
fn spheres_vs_spheres(
    a: &Shape,
    b: &Shape,
) -> Option<(([f32; 3], [f32; 3]), ([f32; 3], [f32; 3]))> {
    if !a.bounds.overlaps(&b.bounds) {
        return None;
    }
    let (sa, sb) = (swept(a), swept(b));
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

/// `EftDet_SpheresVsFighter` (and `CapsuleVsFighter`): the swept sphere along the movement against every body part
/// sphere; the earliest contact wins, a part it already overlaps ends the search. The centre, the contact point and
/// the distance from the start.
fn spheres_vs_fighter(chr: &Chr, s: &Shape) -> Option<([f32; 3], [f32; 3], f32)> {
    let body = &chr.obj.body;
    let hit_box = ColBox {
        min: body.center_bounds.min,
        max: body.center_bounds.max,
    };
    if !s.bounds.overlaps(&hit_box) {
        return None;
    }
    let b = swept(s);
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
pub(crate) struct Sweep {
    start: [f32; 3],
    radius: f32,
    dir: [f32; 3],
    pub length: f32,
}

impl Sweep {
    pub(crate) fn new(a: [f32; 3], b: [f32; 3], radius: f32) -> Self {
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
    pub(crate) fn test_tri(&self, v: &[[f32; 3]; 3], nrm: [f32; 4]) -> Option<([f32; 3], f32)> {
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
pub(crate) fn is_facing(chr: &Chr, yaw: f32) -> bool {
    wrap_angle(chr.pose.rot[1] - yaw).abs() < 1.570_796_3
}

/// `BtlColl_GetHitDir`: the record's travel direction; from its position to the fighter's node 3 when it does not
/// move (or has definition flag 2); from the attacker to the fighter (raised by half their height difference) for a
/// technique of hit direction kind 3. The yaw and the unit direction.
pub(crate) fn hit_dir(w: &World, target: usize, rec: &HitRec) -> (f32, [f32; 3]) {
    let chr = &w.chars[target];
    let mut from_pos = rec.unk54 || length(rec.vel) < 0.01;
    let mut from_atk = None;
    // A skill's record always comes from its user; a technique's of hit direction kind 3.
    if let Some(src) = rec.tech
        && (src.slot < 2 || w.data[src.side].tables.tech().hit_dir_kind(src.slot) == 3)
    {
        from_pos = false;
        from_atk = Some(src.side);
    }
    let mut dir = if from_pos {
        sub(chr.obj.node_pos(3), rec.pos)
    } else if let Some(side) = from_atk {
        let mut d = opp_delta(chr, &w.chars[side]);
        d[1] += w.data[target].height() * 0.5 - w.data[side].height() * 0.5;
        sub([0.0; 3], d)
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

/// `BtlColl_TryDodge`: a fighter in hit-stop or with flag 0xAA lets a record pass, flag 0x5A never; a fighter in a
/// technique of kind 8 at least as strong as the record does; a ki blast with flag 0x44; a technique's record
/// by `try_dodge_tech`.
fn try_dodge(w: &mut World, target: usize, rec: &HitRec, hit_count: i32) -> bool {
    let chr = &w.chars[target];
    if chr.is_frozen() || chr.flags.test(0xAA) {
        return true;
    }
    if chr.flags.test(0x5A) {
        return false;
    }
    if tech_outclasses(chr, w.data[target], rec.level) {
        return true;
    }
    match rec.tech {
        None => chr.flags.test(0x44),
        Some(src) => try_dodge_tech(w, target, hit_count, src),
    }
}

/// The sound of a ki blast knocked away: 0xC / 0xD by the frame for types 2 and 3, else `other`.
fn deflect_sound(w: &World, target: usize, rec: &HitRec, other: i32) {
    let sound = if matches!(hit_record(w, rec).kind_type(), 2 | 3) {
        if w.frame % 2 != 0 { 0xC } else { 0xD }
    } else {
        other
    };
    w.chars[target].play_common(&mut w.env.snd(), sound);
}

/// `BtlColl_TryDeflect`: a free fighter facing a ki blast knocks it away with flag 0x50 (kinds 0, 4, 8) or 0x51,
/// or perfectly inside its deflect window, with its sound and voice kind 0xB; the powered-up skill 0x80 always
/// does (the sound alone).
fn try_deflect(w: &mut World, target: usize, rec: &HitRec) -> bool {
    if rec.tech.is_some() || rec.unk54 {
        return false;
    }
    if hit_flags(w, rec) & 2 != 0 || !w.chars[target].is_free() {
        return false;
    }
    let pflags = w.data[target].tables.params.flags();
    let (yaw, _) = hit_dir(w, target, rec);
    let chr = &mut w.chars[target];
    if !chr.flags.test(0x52)
        && !chr.flags.test(0x54)
        && chr.flags.test(6)
        && w.data[target].tables.params.flags3() & 0x80 != 0
    {
        deflect_sound(w, target, rec, 0x13);
        return true;
    }
    if is_facing(chr, yaw) {
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
    deflect_sound(w, target, rec, 0x13);
    let chr = &mut w.chars[target];
    chr.voice(w.env, w.frame, 0xB);
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

/// `BtlColl_TryReflect`: a free fighter with flag 0x52 facing a ki blast sends it back, with its sound (0x14
/// for types other than 2 and 3) and voice kind 0xB.
fn try_reflect(w: &mut World, target: usize, rec: &HitRec) -> bool {
    if rec.tech.is_some() || rec.unk54 || hit_flags(w, rec) & 2 != 0 {
        return false;
    }
    let (yaw, _) = hit_dir(w, target, rec);
    let chr = &mut w.chars[target];
    if !chr.flags.test(0x52) || !chr.is_free() || is_facing(chr, yaw) {
        return false;
    }
    deflect_sound(w, target, rec, 0x14);
    let chr = &mut w.chars[target];
    chr.voice(w.env, w.frame, 0xB);
    chr.deflect_dir = deflect_dir_of(chr, rec);
    chr.set_fx(0x2E);
    true
}

/// `BtlColl_TryAbsorb`: a free fighter with flag 0x54 (or, for a ki blast, a perfect guard with parameter flag 2)
/// facing the record takes ki: three times a type 0 / 1 blast's cost, or a technique's cost over its shots
/// (technique flag 4 only); sound 0x37.
fn try_absorb(w: &mut World, target: usize, rec: &HitRec) -> bool {
    if rec.unk54 {
        return false;
    }
    let pflags = w.data[target].tables.params.flags();
    let chr = &w.chars[target];
    let mut alt = false;
    if !chr.flags.test(0x54) {
        alt = chr.flags.test(0x41) && chr.unk1074 > 0 && pflags & 2 != 0 && rec.tech.is_none();
        if !alt {
            return false;
        }
    }
    if !chr.is_free() || is_facing(chr, hit_dir(w, target, rec).0) {
        return false;
    }
    let amount = if rec.tech.is_some() {
        match absorb_amount(w, rec) {
            Some(a) => a,
            None => return false,
        }
    } else {
        let blast = *hit_record(w, rec);
        if hit_flags(w, rec) & 2 != 0 || matches!(blast.kind_type(), 2 | 3) {
            return false;
        }
        scale_ki(&w.chars[rec.atk.src], blast.ki()) * 3
    };
    let chr = &mut w.chars[target];
    chr.play_common(&mut w.env.snd(), 0x37);
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

/// `BtlColl_TryGuard`: flag 0x59 takes any record as guarded; else a ki blast by `guard_blast`, a skill's record
/// (slots 0, 1) by `guard_strike`, a technique's (slots 2..4) by `guard_rush`.
fn try_guard(w: &mut World, target: usize, rec: &HitRec) -> bool {
    if w.chars[target].flags.test(0x59) {
        return true;
    }
    match rec.tech {
        None => guard_blast(w, target, rec),
        Some(src) if src.slot < 2 => guard_strike(w, target, rec),
        Some(_) => guard_rush(w, target, rec),
    }
}

/// `BtlColl_GuardBlast`: a fighter in a guard motion takes the guarded damage, the blast's sound at the record
/// (0x42 for type 2, 0xC / 0xD for type 3, else 0x1B / 0x11) and a push from a blast it faces (any unblockable
/// blast goes through).
fn guard_blast(w: &mut World, target: usize, rec: &HitRec) -> bool {
    let flags = hit_flags(w, rec);
    let blast = *hit_record(w, rec);
    let env = w.env;
    let data = w.data[target];
    let (yaw, dir) = hit_dir(w, target, rec);
    let chr = &mut w.chars[target];
    if chr.flags.test(0x44) {
        return false;
    }
    let kind = guard_kind(chr);
    if kind != 6 && (flags & 1 != 0 || is_facing(chr, yaw)) {
        return false;
    }
    let mut dmg = damage_of(&w.chars[rec.atk.src], env, &blast, true);
    let chr = &mut w.chars[target];
    if rec.atk.hits >= 2 {
        dmg /= rec.atk.hits;
    }
    if kind == 0 || kind == 5 {
        return false;
    }
    chr.flags.set(0x66);
    chr.damage(env, data, dmg, 0xC);
    let odd = w.frame % 2 != 0;
    let sound = match blast.kind_type() {
        2 => 0x42,
        3 => {
            if odd {
                0xC
            } else {
                0xD
            }
        }
        _ => {
            if odd {
                0x1B
            } else {
                0x11
            }
        }
    };
    env.snd().request(
        rec.pos,
        -1,
        crate::btl_snd::KIND_COMMON,
        200.0,
        1500.0,
        sound,
    );
    let chr = &mut w.chars[target];
    chr.set_impulse_dir(dir, push_of(&blast, true));
    true
}

/// `BtlColl_Hit`: a pending reaction that cannot be replaced takes the record without effect; else a ki blast's
/// hit or a technique's (slots 2..4); a hit that lands raises the first clash of the record's owner.
fn hit(w: &mut World, target: usize, rec: &HitRec) -> bool {
    if matches!(
        w.chars[target].react.reaction,
        9..=12 | 17 | 19..=22 | 29 | 30 | 34 | 37 | 39..=41
    ) {
        return true;
    }
    let ret = match rec.tech {
        None => hit_by_blast(w, target, rec),
        Some(src) if src.slot < 2 => hit_by_strike(w, target, rec),
        Some(_) => hit_by_rush(w, target, rec),
    };
    if ret {
        let (atk, def) = if rec.owner == 0 {
            let [a, b] = &mut *w.chars;
            (a, b)
        } else {
            let [b, a] = &mut *w.chars;
            (a, b)
        };
        raise_first_clash(atk, def);
    }
    ret
}

/// `BtlColl_HitByBlast`: the reaction by the victim's state (voice kind 0 when it flinches), the sound at the
/// record (0x42 for type 2, else 0x1B / 0x11), the damage and the push, and the victim locks on.
fn hit_by_blast(w: &mut World, target: usize, rec: &HitRec) -> bool {
    let flags = hit_flags(w, rec);
    let blast = *hit_record(w, rec);
    let env = w.env;
    let (data, atk_data) = (w.data[target], w.data[rec.atk.src]);
    let atk_powered = w.chars[rec.atk.src].flags.test(6);
    // Ability 0x47, or 0x70 in the powered-up mode.
    let a = &w.chars[rec.atk.src];
    let atk_armor_break = a.has_ability(0x47) || (a.has_ability(0x70) && atk_powered);
    let (yaw, dir) = hit_dir(w, target, rec);
    let owner = &w.chars[rec.atk.src];
    let plain = map_react(owner, &blast, i32::from(blast.react()), rec.atk.kind, false);
    let s800 = match blast.react_s800() {
        0 => blast.react(),
        v => v,
    };
    let s800 = map_react(owner, &blast, i32::from(s800), rec.atk.kind, true);
    let chr = &mut w.chars[target];
    let st = env.anim_flags(chr.anim.cur);
    if chr.flags.test(0x44) {
        return false;
    }
    // A fighter using a technique takes it without effect.
    if crate::btl_act::is_technique_id(chr.act.current) {
        return true;
    }
    let front = is_facing(chr, yaw);
    let or_plain = |v: i8| i32::from(if v != 0 { v } else { blast.react() });
    let react = if chr.no_flinch > 0 {
        REACT_NOFLINCH
    } else if st & 0x4000 != 0 || chr.act.current == 0xDF {
        or_plain(blast.react_s4000())
    } else if st & 0x800 != 0 {
        s800
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
        if chr.has_ability(0x45) {
            n += 1;
        }
        if atk_powered && atk_data.tables.params.flags3() & 0x100 != 0 {
            n -= 1;
        }
        if atk_armor_break {
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
                plain
            }
        } else {
            plain
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
        chr.voice(env, w.frame, 0);
    }
    chr.set_fx(0x1D);
    let sound = if blast.kind_type() == 2 {
        0x42
    } else if w.frame % 2 != 0 {
        0x1B
    } else {
        0x11
    };
    env.snd().request(
        rec.pos,
        -1,
        crate::btl_snd::KIND_COMMON,
        200.0,
        1500.0,
        sound,
    );
    let mut dmg = damage_of(&w.chars[rec.atk.src], env, &blast, false);
    let chr = &mut w.chars[target];
    if rec.atk.hits >= 2 {
        dmg /= rec.atk.hits;
    }
    chr.damage(env, data, dmg, 0);
    if !chr.flags.test(0x93) {
        chr.flags.set_held(5);
    }
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
