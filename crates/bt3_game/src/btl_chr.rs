//! A fighter (`BtlMgrChr`, src/battle/btl_char_mgr.c and the files it drives): its flags and input, the pose
//! block and the object it drives (src/battle/btl_char_pose.c), the motion control (src/battle/btl_anim.c), the
//! movement helpers (src/battle/btl_char_move.c), the active member's gauges (src/battle/btl_char_member.c), the
//! opponent queries (src/battle/btl_opponent.c) and the yaw of the fighter's camera (src/battle/btl_char_cam.c).
//!
//! The action state machine is in btl_act.rs, the hits in btl_hit.rs, the frame phases in btl_mgr.rs.

use std::f32::consts::{FRAC_PI_2, PI};

use bt3_data::chara_data::{CharaEntry, CharaTables, CommonTables};
use bt3_data::eft_pack::EffectPack;
use bt3_data::stage::StageResource;
use bt3_data::stage_col::StageCollision;

use crate::battle::{apply, mul};
use crate::btl_flags::Flags;
use crate::btl_input::{DEFAULT_KEYS, Input, InputGate, wrap_angle};
use crate::btl_obj::{Obj, ObjData, quat_from_euler, quat_to_euler, rot_x, rot_y, rot_z};

/// `BTL_KMH`: km/h in metres per frame.
pub(crate) const fn kmh(x: f32) -> f32 {
    x * 1000.0 / 3600.0 * (1.0 / 30.0)
}

/// `BTL_DEG`.
pub(crate) const fn deg(x: f32) -> f32 {
    x / 180.0 * PI
}

/// `BTL_PITCH_MAX`.
const PITCH_MAX: f32 = FRAC_PI_2 - 0.01;

/// The motion flags `BtlAnim_*` raise (`BTL_ANIM_FLAG_*`).
pub(crate) mod anim_flag {
    pub(crate) const NEW: u32 = 0x2B;
    pub(crate) const NEW_KEEP: u32 = 0x2C;
    pub(crate) const REQ: u32 = 0x2D;
    pub(crate) const REQ_KEEP: u32 = 0x2E;
    pub(crate) const REQ_SUB: u32 = 0x2F;
    pub(crate) const BLEND_REQ: u32 = 0x32;
    pub(crate) const END: u32 = 0x31;
    pub(crate) const BLENDING: u32 = 0x34;
    pub(crate) const NO_BLEND: u32 = 0x37;
    pub(crate) const F30: u32 = 0x30;
}

/// The pose block (fighter + 0x10, `BtlMovePose` / `BtlCtlPose`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pose {
    pub pos: [f32; 3],
    /// Euler angles; y is the model yaw.
    pub rot: [f32; 3],
    /// `dispOfs`: hover bob and shake, added to the position on the way to the object.
    pub disp: [f32; 3],
    /// Movement between snapshots 0 and 1 of the previous frame.
    pub vel: [f32; 3],
    /// Whole movement of the previous frame.
    pub moved: [f32; 3],
    /// Model node 0's local translation and rotation (Euler) of this frame: the root motion.
    pub root_pos: [f32; 3],
    pub root_rot: [f32; 3],
    pub impulse: [f32; 3],
    /// Unit direction of travel.
    pub dir: [f32; 3],
    /// Heading pitch, positive up.
    pub pitch: f32,
    /// Heading yaw (`facing`).
    pub yaw: f32,
    pub speed: f32,
    /// Vertical speed, positive down.
    pub fall_speed: f32,
    pub lean_x: f32,
    pub lean_z: f32,
    pub hover_phase: f32,
    /// `unkAC`: the dash speed factor (1 outside dashes).
    pub dash_boost: f32,
    pub ground_pos: [f32; 3],
    pub ground_nrm: [f32; 4],
    pub ground_flags: u32,
    pub steer_yaw: f32,
    pub steer_pitch: f32,
    pub orbit_near: f32,
    pub orbit_far: f32,
}

/// The active member's gauge block (`BtlMemberGauge`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Gauge {
    pub health: i32,
    pub health_max: i32,
    pub ki: i32,
    pub ki_max: i32,
    pub blast: i32,
    pub blast_max: i32,
    pub max_power: i32,
    pub variant: i32,
    pub low_health: bool,
    pub low_health_idle: bool,
    pub body_changed: bool,
    /// By technique class: how many were started (`used`), and the self-destructs fired / the lock counters of
    /// the flag-0x2000 techniques (`fired` / `techLock`, the same words).
    pub used: [i32; 5],
    pub fired: [i32; 5],
}

/// `BtlMemberCombo`: the hits taken in a row.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Combo {
    pub hits: i32,
    pub damage: i32,
    pub timer: i32,
    pub new_hit: bool,
    pub changed: bool,
}

/// `HitReact` (fighter + 0xFB0): what the last hit does to this fighter.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct React {
    /// Reaction id 1..49; 1 = none.
    pub reaction: i32,
    pub reaction_sub: i32,
    /// Hit from behind.
    pub back: bool,
    pub silent: bool,
    pub scale: f32,
    /// The direction the hit pushes.
    pub yaw: f32,
    pub turn_yaw: f32,
    pub turn_pitch: f32,
    pub launch_a: f32,
    pub launch_b: f32,
    pub damage: i32,
    /// `noBlend`: a throw of a character with parameter flag bit 19 starts its motions without a blend.
    pub no_blend: bool,
}

/// The damage a fighter has queued against its opponent (fighter + 0xD94): a throw's hits, dealt on its motion's
/// events, and its health / ki drain between two frames.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DamageQueue {
    pub hits: i32,
    pub total_start: i32,
    pub total: i32,
    pub per_hit: i32,
    pub flags: u32,
    pub drain_health_start: i32,
    pub drain_health: i32,
    pub drain_health_step: i32,
    pub drain_ki_start: i32,
    pub drain_ki: i32,
    pub drain_ki_step: i32,
    pub drain_from: i32,
    pub drain_to: i32,
}

/// `HitStatus` (fighter + 0xF40).
#[derive(Clone, Copy, Debug)]
pub(crate) struct HitStatus {
    /// The player this fighter's attack hits this frame, -1 none.
    pub target: i32,
    pub last_hit_no: i32,
    pub contact: i32,
}

/// `BtlActAttack`: an attack record's parameters, pending (`nextAttack`, from `BtlAct_PrepareAttack`) and latched
/// by the attack actions (`attack`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Attack {
    /// The record's flags: 1 / 2 / 4 / 8 / 0x100 / 0x200 follow-up inputs, 0x10 held flag 0xDD on a chained
    /// follow-up, 0x20 the cut by camera side, 0x40 the cut ends on contact, 0x80 flag 0xCD, 0x400 end in 0xC.
    pub flags: i32,
    /// One motion per part (the second is the first + 2; -1 for none).
    pub motion: [i32; 2],
    pub parts: i32,
    /// The record's lead-in (`level`: 1..3 request effect bit 0xD).
    pub lead_in: i32,
    /// The camera cut, < 0 for none.
    pub cut: i32,
    /// Follow-up attack records (index from 0x70), < 0 for none.
    pub follow: [i32; 2],
    pub rate: f32,
    pub cancel_at: f32,
    /// The frames until the strike connects, measured by the lead-ins (`leadTime`).
    pub lead_time: f32,
    /// The advance speed per frame (0: stand still).
    pub speed: f32,
    pub chained: bool,
}

/// `ChrCam`: the fighter camera, as far as it is ported.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ChrCam {
    pub yaw: f32,
    pub yaw_step: f32,
    /// Which side of the fighter the camera sits on, -limit..limit.
    pub side: f32,
    pub eye: [f32; 3],
    pub rot: [f32; 3],
    pub target: [f32; 3],
    pub rate: f32,
}

/// What a side's character files give the fighter.
pub(crate) struct SideData<'a> {
    pub obj: ObjData<'a>,
    pub tables: &'a CharaTables,
    pub entry: CharaEntry,
    pub effect: Option<&'a EffectPack>,
    /// The decoded textures and sprite packs of the two blast lists' emitter sets.
    pub blast_assets: [Option<&'a crate::eft::SetAssets>; 2],
    /// The costume of the side's character (`BtlMember_GetActive()->costume`).
    pub costume: i32,
    /// The side's character (`BtlMember_GetActive()->chara`).
    pub chara: i32,
}

impl SideData<'_> {
    /// `BtlCharApi_GetHeight`: the model header's size[0].
    pub(crate) fn height(&self) -> f32 {
        self.obj.model.header.size[0]
    }

    /// `BtlCharApi_GetCenterHeight`: the model header's size[2], what a placement adds to a path point's y.
    pub(crate) fn center_height(&self) -> f32 {
        self.obj.model.header.size[2]
    }

    /// `BtlCharApi_GetRadius`: the parameter's +0xC when positive, else the body data's base radius * 1.8.
    pub(crate) fn radius(&self) -> f32 {
        let r = self.tables.params.f32_at(0xC);
        if r > 0.0 {
            r
        } else {
            self.obj.body.map_or(0.0, |b| b.base_radius) * 1.8
        }
    }

    /// `BtlCharApi_GetCamSideSlope`: the model header's size[3].
    pub(crate) fn cam_side_slope(&self) -> f32 {
        self.obj.model.header.size[3]
    }

    /// `BtlParam_CanFly`: parameter flag 0x1000 grounds the character (no abilities are ported).
    pub(crate) fn can_fly(&self) -> bool {
        self.tables.params.flags() & 0x1000 == 0
    }
}

/// What the whole battle shares with the fighters.
pub(crate) struct Env<'a> {
    pub common: &'a CommonTables,
    /// `gBtlStatCurve`: {value at 80, at 0, at -20} per row.
    pub stat_curve: &'a [[f32; 3]; 13],
    pub stage: &'a StageResource,
    pub col: &'a StageCollision,
    /// `Battle_GetMode`.
    pub mode: i32,
    /// `BtlChar_IsStage4Or27`.
    pub stage_4_or_27: bool,
    /// `BtlSeq_GetState`.
    pub seq_state: i32,
    /// The battle flags the input reads.
    pub gate: InputGate,
}

impl Env<'_> {
    /// `BtlAnim_GetFlags`: 0 for ids outside the table.
    pub(crate) fn anim_flags(&self, motion: i32) -> u32 {
        usize::try_from(motion)
            .ok()
            .and_then(|m| self.common.motion_flags.get(m))
            .copied()
            .unwrap_or(0)
    }

    /// `BtlStat_EvalCurve` at level 0 (no skills, items or abilities are ported): the row's base value.
    pub(crate) fn curve_base(&self, row: usize) -> f32 {
        self.stat_curve[row][1]
    }

    /// `BtlStage_GetInnerRadius`.
    pub(crate) fn inner_radius(&self) -> f32 {
        self.stage.bounds.radius - 100.0
    }
}

/// Motion ids (`chr->motion` and friends).
#[derive(Clone, Copy, Debug)]
pub(crate) struct AnimIds {
    pub cur: i32,
    pub next: i32,
    /// Last frame's motion (`prev974`).
    pub prev: i32,
    pub sub: i32,
}

/// The action bookkeeping (`BtlActState`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ActState {
    pub current: i32,
    pub request: i32,
    /// Read by `BtlAct_GetPrev`; nothing stores it.
    pub prev: i32,
    pub queue: [i32; 4],
}

/// A fighter (0x1600 bytes in the original).
#[derive(Clone, Debug)]
pub(crate) struct Chr {
    pub player: usize,
    pub flags: Flags,
    pub input: Input,
    pub pose: Pose,
    pub obj: Obj,
    pub act: ActState,
    pub prev_action: i32,
    /// `prev964`: last frame's action frame counter.
    pub prev_action_frame: i32,
    /// `actBits`: the action classes the decisions offered this frame.
    pub act_bits: u64,
    /// Per-action scratch, zeroed when an action starts.
    pub work: [i32; 0x14],
    pub action_frame: i32,
    pub hold_action: bool,
    pub anim: AnimIds,
    pub blend_time: f32,
    pub stall: i32,
    pub prev_stall: i32,
    pub anim_rate: f32,
    pub gauge: Gauge,
    pub combo: Combo,
    pub react: React,
    pub status: HitStatus,
    /// The attack-charge level (0..1) and its timers.
    pub charge: f32,
    pub charge_timer: f32,
    pub charge_gauge: i32,
    pub charge_full_frames: i32,
    /// `blastCharge`: progress of the charged ki blast's hold motion (0..1).
    pub blast_charge: f32,
    /// `blastAltHand`: the next plain ki blast uses the second hand.
    pub blast_alt_hand: i32,
    /// `blastNode`: the model node the charged ki blast leaves from.
    pub blast_node: i32,
    /// `aimDir`: the direction the ki blast actions aim (read when the motion fires).
    pub aim_dir: [f32; 3],
    /// `pitch`: the pitch toward the opponent `BtlAct_SetPitchMotion` measured.
    pub aim_pitch: f32,
    /// `deflectDir`: the direction a deflected blast is sent in.
    pub deflect_dir: [f32; 3],
    /// This fighter's live ki blast records by class (definition kinds 0 / 4 / 8, the others), from the last
    /// record list (`BtlMove_CanFireBlast`).
    pub live_blasts: [i32; 2],
    pub rush_step: i32,
    pub dash_count: i32,
    pub dash_limit: i32,
    pub vanish_count: i32,
    pub vanish_limit: i32,
    /// `attackId`: the attack last prepared (read by `BtlAct_QueueComboFinish`, which is not ported).
    pub attack_id: i32,
    /// The member's idle talk count (+0xA0): the neutral state talks (0x43) only while it is 0.
    pub idle_talk_count: i32,
    /// `searchAngle` / `searchRange`: the window the search 0x36 finds the opponent in (zero until a search ends,
    /// then 0.5 rad / 500).
    pub search_angle: f32,
    pub search_range: f32,
    /// The damage queued against the opponent (`queue`).
    pub queue: DamageQueue,
    /// `clashCountB`: the inputs counted in clashes B and C.
    pub clash_count_b: i32,
    /// `clashAnswerFrame`: the clash C answer, -1 none yet, -2 a wrong button, else the frame it came.
    pub clash_answer_frame: i32,
    /// `skillCount3` (fighter + 0xE5C): a skill counter technique 0x268 scales with (the skills are not ported).
    pub skill_count3: i32,
    /// `boostStock` (0..10): raised by the throws of character 0x6E.
    pub boost_stock: i32,
    /// `burstNode`: the node the finisher 0x66's burst effect follows.
    pub burst_node: i32,
    /// `nextAttack` / `attack`: the pending and the latched attack parameters.
    pub next_attack: Attack,
    pub attack: Attack,
    /// `bodyWarpPos`: where flag 0x55 puts the body sphere this frame (`BtlChar_RequestBodyWarp`).
    pub body_warp_pos: [f32; 3],
    pub evasion_count: i32,
    pub blast_shots: i32,
    pub blast_shots_timer: i32,
    pub tech_delay: i32,
    pub stun_timer: i32,
    pub unk_fe4: i32,
    pub shake_timer: i32,
    pub unk_ff0: i32,
    pub blind_timer: i32,
    pub blind_level: i32,
    pub recover_presses: i32,
    pub react_count: [u8; 0x32],
    pub react_timer: [i8; 0x32],
    /// `animCycle` (+0xFB8): the stagger variant.
    pub anim_cycle: i32,
    /// `mashLeft` (+0x1000).
    pub mash_left: i32,
    /// The defence windows (+0x1068..): level evasion, the first-hit dodge, counter, deflect, throw and rush breaks.
    pub unk1068: i32,
    pub dodge_window: i32,
    pub counter_window: i32,
    pub unk1074: i32,
    pub throw_break_window: i32,
    pub rush_break_window: i32,
    pub unk1080: i32,
    pub dodges: i32,
    pub dodges_b: i32,
    pub skill_slot: i32,
    pub skill_timer_c: i32,
    pub skill_timer_d: i32,
    pub no_flinch: i32,
    pub sec_timer: i32,
    pub input_off_timer: i32,
    /// Hit-stop: frames left, the pending length and its delay.
    pub freeze: i32,
    pub freeze_next: i32,
    pub freeze_delay: i32,
    pub snap_pos: [[f32; 3]; 7],
    pub snap_rot: [[f32; 3]; 7],
    pub snap_mask: u32,
    pub cam: ChrCam,
    pub dir_held: i32,
    /// Battle event 0x3C: this fighter landed the battle's first hit (`BtlChar_RaiseFirstClash`).
    pub first_hit: bool,
    /// Battle event 0x59: this fighter lost health.
    pub took_damage: bool,
    /// Effect / sound request bits of this frame (`fxBits`, fighter + 0x1262); nothing draws them yet.
    pub fx_bits: [u8; 0x10],
}

/// Which snapshot `BtlChar_Snapshot` writes besides the one asked for.
const SNAP_LAST: usize = 6;

impl Chr {
    /// `BtlChar_ResetAll` for one fighter: a zeroed fighter, `BtlChar_BindObject` and `BtlChar_Reset`.
    pub(crate) fn new(player: usize, data: &SideData, env: &Env, cpu: bool) -> Self {
        let obj = Obj::new(&data.obj, data.tables.params.radius());
        let mut chr = Self {
            player,
            flags: Flags::default(),
            input: Input::default(),
            pose: Pose::default(),
            obj,
            act: ActState {
                current: 0,
                request: 0,
                prev: 0,
                queue: [0; 4],
            },
            prev_action: 0,
            prev_action_frame: 0,
            act_bits: 0,
            work: [0; 0x14],
            action_frame: 0,
            hold_action: false,
            anim: AnimIds {
                cur: 0,
                next: 0,
                prev: 0,
                sub: -1,
            },
            blend_time: 0.0,
            stall: 0,
            prev_stall: 0,
            anim_rate: 0.0,
            gauge: Gauge::default(),
            combo: Combo::default(),
            react: React::default(),
            status: HitStatus {
                target: 0,
                last_hit_no: 0,
                contact: 0,
            },
            charge: 0.0,
            charge_timer: 0.0,
            charge_gauge: 0,
            charge_full_frames: 0,
            blast_charge: 0.0,
            blast_alt_hand: 0,
            blast_node: 0,
            aim_dir: [0.0; 3],
            aim_pitch: 0.0,
            deflect_dir: [0.0; 3],
            live_blasts: [0; 2],
            rush_step: 0,
            dash_count: 0,
            dash_limit: 0,
            vanish_count: 0,
            vanish_limit: 0,
            attack_id: 0,
            idle_talk_count: 0,
            search_angle: 0.0,
            search_range: 0.0,
            queue: DamageQueue::default(),
            clash_count_b: 0,
            clash_answer_frame: 0,
            skill_count3: 0,
            boost_stock: 0,
            burst_node: 0,
            next_attack: Attack::default(),
            attack: Attack::default(),
            body_warp_pos: [0.0; 3],
            evasion_count: 0,
            blast_shots: 0,
            blast_shots_timer: 0,
            tech_delay: 0,
            stun_timer: 0,
            unk_fe4: 0,
            shake_timer: 0,
            unk_ff0: 0,
            blind_timer: 0,
            blind_level: 0,
            recover_presses: 0,
            react_count: [0; 0x32],
            react_timer: [0; 0x32],
            anim_cycle: 0,
            mash_left: 0,
            unk1068: 0,
            dodge_window: 0,
            counter_window: 0,
            unk1074: 0,
            throw_break_window: 0,
            rush_break_window: 0,
            unk1080: 0,
            dodges: 0,
            dodges_b: 0,
            skill_slot: 0,
            skill_timer_c: 0,
            skill_timer_d: 0,
            no_flinch: 0,
            sec_timer: 0,
            input_off_timer: 0,
            freeze: 0,
            freeze_next: 0,
            freeze_delay: 0,
            snap_pos: [[0.0; 3]; 7],
            snap_rot: [[0.0; 3]; 7],
            snap_mask: 0,
            cam: ChrCam::default(),
            dir_held: -1,
            first_hit: false,
            took_damage: false,
            fx_bits: [0; 0x10],
        };
        // BtlChar_BindObject.
        chr.flags.clear(0xBE);
        chr.flags.clear_range(0x98, 0x99);
        // BtlChar_Reset.
        chr.flags.set_stage(1);
        chr.input.init(&DEFAULT_KEYS);
        chr.act.queue = [-1; 4];
        chr.input.inject_on = cpu;
        chr.place_at_start(data, env);
        chr.flags.set_held(5);
        chr.pose_to_obj(true);
        chr.anim_play(data, 0, 0.0);
        chr.obj.sample_pose(&data.obj);
        chr.obj.calc_matrices(&data.obj);
        // BtlMember_LoadParams(chr, 0, 1, 100 %, variant 0) with no abilities.
        let entry = data.entry;
        let g = &mut chr.gauge;
        g.health_max = entry.health_max.max(10000);
        g.ki_max = 100_000;
        g.blast_max = entry.blast_stocks * 100_000;
        g.health = ((g.health_max as f32 * 100.0 / 100.0) as i32).max(1);
        g.health_max = g.health_max.max(g.health);
        g.ki = entry.ki_start.min(g.ki_max);
        chr
    }

    /// The HUD's view of the active member's gauges (`Hud_PreUpdate`).
    pub(crate) fn hud_gauge(&self) -> crate::hud_gauge::SideGauge {
        let g = &self.gauge;
        crate::hud_gauge::SideGauge {
            hp: g.health,
            ki: g.ki,
            blast: g.blast,
            blast_max: g.blast_max,
            max_power: g.max_power,
            powered_up: self.flags.test(6),
            dead: g.health <= 0,
        }
    }

    /// `BtlCtrl_IsPoseReached`: motions 0x181 / 0x183, or 0x184 played to its end.
    pub(crate) fn pose_reached(&self) -> bool {
        match self.anim.cur {
            0x181 | 0x183 => true,
            0x184 => self.flags.test(anim_flag::END),
            _ => false,
        }
    }

    /// `BtlChar_IsFrozen`.
    pub(crate) fn is_frozen(&self) -> bool {
        self.freeze > 0
    }

    /// `BtlChar_IsFree`: health left, flag 0xBE clear and no stun.
    pub(crate) fn is_free(&self) -> bool {
        self.gauge.health > 0 && !self.flags.test(0xBE) && self.stun_timer <= 0
    }

    /// `BtlChar_IsDead`.
    pub(crate) fn is_dead(&self) -> bool {
        self.gauge.health < 1
    }

    /// `BtlChar_SetFxBit`.
    pub(crate) fn set_fx(&mut self, bit: u32) {
        if let Some(b) = self.fx_bits.get_mut((bit >> 3) as usize) {
            *b |= 1 << (bit & 7);
        }
    }

    // ---- motion control (btl_anim.c) -------------------------------------------------------------------------

    /// `BtlAnim_Play`: starts a motion on the main layer now, raising flag 0x2B; a blend over `blend` seconds.
    pub(crate) fn anim_play(&mut self, data: &SideData, motion: i32, blend: f32) {
        // Motions with table flag 0x20000000 play from the opponent's table (throws): not ported.
        self.obj.play(&data.obj, 0, motion.max(0) as usize, true);
        self.obj.flags &= !0x60;
        self.flags.clear(anim_flag::F30);
        self.flags.clear_range(anim_flag::REQ, anim_flag::END);
        self.flags.set(anim_flag::NEW);
        self.anim.next = -1;
        self.anim.cur = motion;
        self.stall = 0;
        if blend > 0.0 {
            self.anim_set_blend(blend);
            self.anim_apply_blend();
        }
    }

    /// `BtlAnim_Play` of a motion with table flag 0x20000000: the opponent's motion of that id, retargeted.
    pub(crate) fn anim_play_from(
        &mut self,
        data: &SideData,
        opp: &SideData,
        env: &Env,
        motion: i32,
        blend: f32,
    ) {
        let flags = env.anim_flags(motion);
        self.obj
            .play_from(&data.obj, &opp.obj, motion.max(0) as usize, flags);
        self.obj.flags &= !0x60;
        self.flags.clear(anim_flag::F30);
        self.flags.clear_range(anim_flag::REQ, anim_flag::END);
        self.flags.set(anim_flag::NEW);
        self.anim.next = -1;
        self.anim.cur = motion;
        self.stall = 0;
        if blend > 0.0 {
            self.anim_set_blend(blend);
            self.anim_apply_blend();
        }
    }

    /// `BtlAnim_Play` with its table-flag switch: the motion from the opponent's table for flag 0x20000000.
    pub(crate) fn anim_play_any(
        &mut self,
        data: &SideData,
        opp: &SideData,
        env: &Env,
        motion: i32,
        blend: f32,
    ) {
        if env.anim_flags(motion) & 0x2000_0000 != 0 {
            self.anim_play_from(data, opp, env, motion, blend);
        } else {
            self.anim_play(data, motion, blend);
        }
    }

    /// `BtlAnim_InFrameRange`: the last step crossed frames lo..hi.
    pub(crate) fn anim_in_frame_range(&self, lo: f32, hi: f32) -> bool {
        let (f, p) = (self.obj.frame, self.obj.prev_frame);
        if f >= p {
            p <= hi && lo <= f
        } else {
            p <= hi || lo <= f
        }
    }

    /// `BtlMember_DrainKi`: what was taken.
    pub(crate) fn drain_ki(&mut self, amount: i32) -> i32 {
        if amount <= 0 {
            return 0;
        }
        let before = self.gauge.ki;
        self.gauge.max_power = 0;
        self.gauge.ki = (before - amount).max(0);
        before - self.gauge.ki
    }

    /// `BtlAnim_PlayKeep`: the same without restarting the frame, raising flag 0x2C.
    pub(crate) fn anim_play_keep(&mut self, data: &SideData, motion: i32, blend: f32) {
        self.obj.play(&data.obj, 0, motion.max(0) as usize, false);
        self.flags.clear_range(anim_flag::REQ, anim_flag::END);
        self.flags.set(anim_flag::NEW_KEEP);
        self.anim.cur = motion;
        self.anim.next = -1;
        if blend > 0.0 {
            self.anim_set_blend(blend);
            self.anim_apply_blend();
        }
    }

    /// `BtlAnim_SubToMain`.
    fn anim_sub_to_main(&mut self) {
        self.obj.promote_layer();
        self.anim.cur = self.anim.sub;
        self.obj.flags &= !0x60;
        self.flags.clear_range(anim_flag::REQ, anim_flag::END);
        self.flags.set(anim_flag::NEW);
        self.stall = 0;
    }

    /// `BtlAnim_Request`: started by `BtlAnim_FlushRequest` at the next dispatch.
    pub(crate) fn anim_request(&mut self, motion: i32, blend: f32) {
        self.flags.set_held(anim_flag::REQ);
        self.anim.next = motion;
        self.anim_set_blend(blend);
    }

    /// `BtlAnim_RequestKeep`.
    pub(crate) fn anim_request_keep(&mut self, motion: i32, blend: f32) {
        self.flags.set_held(anim_flag::REQ_KEEP);
        self.anim.next = motion;
        self.anim_set_blend(blend);
    }

    /// `BtlAnim_SetBlend`.
    fn anim_set_blend(&mut self, blend: f32) {
        self.flags.set_held(anim_flag::BLEND_REQ);
        self.blend_time = blend;
    }

    /// `BtlAnim_FlushRequest`, `BtlAnim_FlushRequestKeep`, `BtlAnim_FlushSubToMain`.
    pub(crate) fn anim_flush_requests(&mut self, data: &SideData, opp: &SideData, env: &Env) {
        if self.flags.test(anim_flag::REQ) {
            self.anim_play_any(data, opp, env, self.anim.next, self.blend_time);
        }
        if self.flags.test(anim_flag::REQ_KEEP) {
            self.anim_play_keep(data, self.anim.next, self.blend_time);
        }
        if self.flags.test(anim_flag::REQ_SUB) && self.anim.sub >= 0 {
            self.anim_sub_to_main();
        }
    }

    /// `BtlAnim_ApplyBlend`: hands the stored blend time to the object (none with flag 0x37), one step taken off.
    pub(crate) fn anim_apply_blend(&mut self) {
        if self.flags.test(anim_flag::NO_BLEND) {
            self.obj.start_blend(0.0);
            self.flags.clear(anim_flag::BLEND_REQ);
            self.blend_time = 0.0;
        } else if self.flags.test(anim_flag::BLEND_REQ) {
            self.obj.start_blend(self.blend_time);
            self.flags.clear(anim_flag::BLEND_REQ);
            self.blend_time = 0.0;
            self.obj.blend = (self.obj.blend - self.obj.blend_step).max(0.0);
        }
    }

    /// `BtlAnim_PlaySub`: a motion on the second layer unless it is already there.
    pub(crate) fn anim_play_sub(&mut self, data: &SideData, motion: i32) {
        if self.anim.sub != motion {
            self.obj.play(&data.obj, 1, motion.max(0) as usize, false);
            self.anim.sub = motion;
        }
    }

    /// `BtlAnim_SetSubMix`.
    pub(crate) fn anim_set_sub_mix(&mut self, v: f32) {
        self.obj.mix = v;
    }

    /// `BtlAnim_SetStep`.
    pub(crate) fn anim_set_step(&mut self, step: f32) {
        self.obj.step = step;
    }

    /// `BtlAnim_SetDuration`: the whole motion in `seconds`.
    pub(crate) fn anim_set_duration(&mut self, seconds: f32) {
        let length = self.obj.layers[0].length;
        self.obj.step = if seconds < 0.033_333_333 {
            length
        } else {
            length / (seconds * 30.0)
        };
    }

    fn anim_blend_step(&mut self) {
        self.obj.blend -= self.obj.blend_step;
        if self.obj.blend < 0.0 {
            self.obj.blend = 0.0;
            self.flags.clear(anim_flag::BLENDING);
        } else {
            self.flags.set_held(anim_flag::BLENDING);
        }
    }

    fn anim_count_stall(&mut self) {
        self.prev_stall = self.stall;
        if self.obj.prev_frame == self.obj.frame {
            self.stall += 1;
        } else {
            self.stall = 0;
        }
    }

    /// `BtlAnim_Advance`: one step (`flags & 1`: not while blending); at the end clamps, holds flag 0x31, true.
    pub(crate) fn anim_advance(&mut self, flags: u32) -> bool {
        let mut done = false;
        self.anim_blend_step();
        self.obj.prev_frame = self.obj.frame;
        if flags & 1 == 0 || self.obj.blend == 0.0 {
            self.obj.frame += self.obj.step;
            let length = self.obj.layers[0].length;
            if self.obj.frame >= length - 0.1 {
                self.flags.set_held(anim_flag::END);
                done = true;
                self.obj.frame = length;
            }
        }
        self.anim_count_stall();
        done
    }

    /// `BtlAnim_AdvanceThen`: at the end requests `next`.
    pub(crate) fn anim_advance_then(&mut self, next: i32, blend: f32, flags: u32) -> bool {
        let done = self.anim_advance(flags);
        if done {
            self.anim_request(next, blend);
        }
        done
    }

    /// `BtlAnim_AdvanceLoop`: back to frame 0 at the end.
    pub(crate) fn anim_advance_loop(&mut self, flags: u32) {
        self.anim_blend_step();
        self.obj.prev_frame = self.obj.frame;
        if flags & 1 == 0 || self.obj.blend == 0.0 {
            self.obj.frame += self.obj.step;
            if self.obj.frame >= self.obj.layers[0].length - 0.1 {
                self.obj.frame = 0.0;
            }
        }
        self.anim_count_stall();
    }

    /// `BtlAnim_PassedRatio`.
    pub(crate) fn anim_passed_ratio(&self, ratio: f32) -> bool {
        self.anim_passed_frame(self.obj.layers[0].length * ratio)
    }

    /// `BtlAnim_PassedFrame`: the last advance passed `frame`, wrap-around included.
    pub(crate) fn anim_passed_frame(&self, frame: f32) -> bool {
        let (prev, cur) = (self.obj.prev_frame, self.obj.frame);
        if cur >= prev {
            prev <= frame && frame < cur
        } else {
            prev <= frame || frame < cur
        }
    }

    /// `BtlAnim_GetProgress`.
    pub(crate) fn anim_progress(&self) -> f32 {
        self.obj.frame / self.obj.layers[0].length
    }

    /// `BtlAnim_IsNew`.
    pub(crate) fn anim_is_new(&self) -> bool {
        self.flags.test(anim_flag::NEW)
    }

    /// `BtlAnim_TestAttr`: an event of the current frame; nothing while the frame stood still the frame before.
    pub(crate) fn anim_test_attr(&self, mask: u64) -> bool {
        self.prev_stall <= 0 && self.obj.test_event(mask)
    }

    // ---- pose and object (btl_char_pose.c) ---------------------------------------------------------------------

    /// `BtlChar_PoseToObj`: object position = pose + display offset, less the root motion rotated into the world
    /// (not on the frame a motion starts with `keep_root`), then the object matrix.
    pub(crate) fn pose_to_obj(&mut self, keep_root: bool) {
        let old = self.obj.pos;
        let p = self.pose;
        self.obj.pos = add(p.pos, p.disp);
        self.obj.rot = p.rot;
        self.obj.scale = 1.0;
        let apply_root = !(keep_root && self.flags.test(anim_flag::NEW));
        if apply_root {
            let m = mul(
                &mul(
                    &mul(&rot_y(PI), &rot_y(self.obj.rot[1])),
                    &rot_x(self.obj.rot[0]),
                ),
                &rot_z(self.obj.rot[2]),
            );
            let r = p.root_pos;
            let v = apply(&m, [r[0], r[1], r[2], 1.0]);
            self.obj.pos = sub(self.obj.pos, scale3([v[0], v[1], v[2]], self.obj.scale));
        }
        if length(sub(self.obj.pos, old)) < 0.001 {
            self.obj.pos = old;
        }
        self.obj.xf_update();
    }

    /// `BtlChar_ObjToPose`: the pose follows node 0's world position; the root motion is read back.
    pub(crate) fn obj_to_pose(&mut self) {
        self.pose.pos = sub(self.obj.root_pos, self.pose.disp);
        self.pose.rot = self.obj.rot;
        match self.obj.node(0) {
            Some(n) => {
                self.pose.root_pos = n.pos;
                self.pose.root_rot = quat_to_euler(n.rot);
            }
            None => {
                self.pose.root_pos = [0.0; 3];
                self.pose.root_rot = [0.0; 3];
            }
        }
    }

    /// `BtlChar_ApplyRootYaw`: under flag 0x33 (not 0x94) the root node's yaw turns the fighter.
    pub(crate) fn apply_root_yaw(&mut self) {
        if self.flags.test(0x33) && !self.flags.test(0x94) {
            let yaw = self.pose.root_rot[1];
            self.pose.rot[1] = wrap_angle(yaw + self.pose.rot[1]);
            self.pose.yaw = self.pose.rot[1];
            self.pose.pitch *= yaw.cos();
        }
    }

    /// `BtlChar_UpdateLean`: the lean quaternion from the two lean angles.
    pub(crate) fn update_lean(&mut self) {
        self.obj.lean = quat_from_euler([-self.pose.lean_x, 0.0, self.pose.lean_z]);
    }

    /// `BtlChar_Place`: pose, object, camera eye and yaw; flags 0xCD, 0x24, 0x3F.
    pub(crate) fn place(&mut self, data: &SideData, pos: [f32; 3], rot: [f32; 3], area: i32) {
        let p = &mut self.pose;
        p.pos = pos;
        p.rot = rot;
        p.vel = [0.0; 3];
        p.moved = [0.0; 3];
        p.yaw = rot[1];
        p.pitch = 0.0;
        p.speed = 0.0;
        p.fall_speed = 0.0;
        p.dir = [rot[1].sin(), 0.0, rot[1].cos()];
        self.obj.pos = pos;
        self.obj.root_pos = pos;
        self.obj.rot = rot;
        self.obj.area = area;
        self.cam.eye = [pos[0], pos[1] - data.height(), pos[2]];
        self.cam.yaw = rot[1];
        self.flags.set(0xCD);
        self.flags.set(0x24);
        self.flags.set(0x3F);
        self.obj.reset_body();
    }

    /// `BtlChar_PlaceAtStart` (`BtlStage_GetStartPlace(player, ..., 0)`): the stage's starting place, facing its
    /// target.
    pub(crate) fn place_at_start(&mut self, data: &SideData, env: &Env) {
        let place = env.stage.starting_placements.get(self.player);
        let pos = place.map_or([0.0; 3], |p| p.position);
        let target = place.map_or([0.0, 0.0, 1.0], |p| p.target);
        let yaw = (target[0] - pos[0]).atan2(target[2] - pos[2]);
        let area = env.col.find_zone_near(-1, pos) as i32;
        self.place(data, pos, [0.0, yaw, 0.0], area);
    }

    /// `BtlChar_Snapshot`: snapshot `n` and the latest one.
    pub(crate) fn snapshot(&mut self, n: usize) {
        for i in [n, SNAP_LAST] {
            self.snap_pos[i] = self.pose.pos;
            self.snap_rot[i] = self.pose.rot;
            self.snap_mask |= 1 << i;
        }
    }

    /// `BtlChar_GetSnapPos`: snapshot `n`, or the current position.
    pub(crate) fn snap_pos(&self, n: usize) -> [f32; 3] {
        if self.snap_mask & (1 << n) != 0 {
            self.snap_pos[n]
        } else {
            self.pose.pos
        }
    }

    /// `BtlChar_GetSnapRot`.
    pub(crate) fn snap_rot(&self, n: usize) -> [f32; 3] {
        if self.snap_mask & (1 << n) != 0 {
            self.snap_rot[n]
        } else {
            self.pose.rot
        }
    }

    /// `BtlChar_GetSnapDelta`.
    pub(crate) fn snap_delta(&self, from: usize, to: usize) -> [f32; 3] {
        sub(self.snap_pos(to), self.snap_pos(from))
    }

    // ---- gauges (btl_char_member.c) ------------------------------------------------------------------------------

    /// `BtlMember_AddHealth`.
    pub(crate) fn add_health(&mut self, amount: i32) {
        if amount > 0 {
            self.gauge.health = (self.gauge.health + amount).min(self.gauge.health_max);
        }
    }

    /// `BtlMember_HasHealth`.
    pub(crate) fn has_health(&self, amount: i32) -> bool {
        self.gauge.health >= amount
    }

    /// `BtlMember_GetHealthRatio`.
    pub(crate) fn health_ratio(&self) -> f32 {
        if self.gauge.health_max > 0 {
            self.gauge.health as f32 / self.gauge.health_max as f32
        } else {
            0.0
        }
    }

    /// `BtlMove_TestContact`: the ground (flag 0xF) and the last stage polygon (flag 0x1F without 0x20), as a
    /// floor (normal within 45 degrees of straight up, `floor`) or a wall; true when one of the wanted kind is
    /// there and the direction of travel points into it by more than cos(angle) (-2 pi: any direction).
    fn test_contact(&self, floor: bool, angle: f32) -> bool {
        for i in 0..2 {
            let n = match i {
                0 if self.flags.test(0xF) => self.pose.ground_nrm,
                1 if self.flags.test(0x1F) && !self.flags.test(0x20) => self.obj.work.contact_nrm,
                _ => continue,
            };
            let is_floor = n[1] <= -0.707;
            if is_floor != floor {
                continue;
            }
            if angle == -6.283_185_3 {
                return true;
            }
            let d = -(n[0] * self.pose.dir[0] + n[1] * self.pose.dir[1] + n[2] * self.pose.dir[2]);
            if angle.cos() < d {
                return true;
            }
        }
        false
    }

    /// `BtlMove_IsHeadingIntoFloor`.
    pub(crate) fn is_heading_into_floor(&self, angle: f32) -> bool {
        self.test_contact(true, angle)
    }

    /// `BtlMove_IsHeadingIntoWall`.
    pub(crate) fn is_heading_into_wall(&self, angle: f32) -> bool {
        self.test_contact(false, angle)
    }

    /// `BtlMove_DeflectAtStageLimit`: at the side limit (flag 0x14) heads along it (by the turn of the last
    /// movement, or the reverse with `back`); at the top / bottom (0x15 / 0x16) drops the pitch into it.
    pub(crate) fn deflect_at_stage_limit(&mut self, back: bool) {
        let p = &mut self.pose;
        if self.flags.test(0x14) && 0.0 < (p.pos[0] * p.pos[0] + p.pos[2] * p.pos[2]).sqrt() {
            let mut a = if 0.0 < p.pos[0] * p.vel[2] - p.pos[2] * p.vel[0] {
                p.pos[2].atan2(-p.pos[0])
            } else {
                (-p.pos[2]).atan2(p.pos[0])
            };
            if back {
                a = wrap_angle(a + PI);
            }
            p.yaw = a;
        }
        let up = self.flags.test(0x15);
        let down = self.flags.test(0x16);
        let p = &mut self.pose;
        if up && ((back && 0.0 < p.pitch) || (!back && p.pitch < 0.0)) {
            p.pitch = 0.0;
        }
        if down && ((back && p.pitch < 0.0) || (!back && 0.0 < p.pitch)) {
            p.pitch = 0.0;
        }
    }

    /// `BtlMove_SetLeanZ`.
    pub(crate) fn set_lean_z(&mut self, angle: f32) {
        self.pose.lean_z = angle;
    }

    /// `BtlMember_AddKi`.
    pub(crate) fn add_ki(&mut self, amount: i32) {
        self.gauge.ki = (self.gauge.ki + amount).clamp(0, self.gauge.ki_max);
    }

    /// `BtlMember_HasKi`.
    pub(crate) fn has_ki(&self, amount: i32) -> bool {
        self.gauge.ki >= amount
    }

    /// `BtlMember_SpendKi`: not in the powered-up mode (flag 6) unless forced.
    /// Any spending empties the max power gauge; returns true when the ki ran out.
    pub(crate) fn spend_ki(&mut self, amount: i32, force: bool) -> bool {
        if self.flags.test(6) && !force {
            return false;
        }
        self.gauge.ki -= amount;
        let ran_out = self.gauge.ki < 0;
        if ran_out {
            self.gauge.ki = 0;
        }
        self.gauge.max_power = 0;
        ran_out
    }

    /// `BtlMember_IsKiEmpty`.
    pub(crate) fn is_ki_empty(&self) -> bool {
        self.gauge.ki <= 0
    }

    /// `BtlMember_AddBlast`.
    pub(crate) fn add_blast(&mut self, amount: i32) {
        self.gauge.blast = (self.gauge.blast + amount).clamp(0, self.gauge.blast_max);
    }

    /// `BtlMember_Damage`: the one place health goes. `flags` are the original's `BTL_DMG_*` bits; the abilities
    /// they test are not ported. Returns the health lost.
    pub(crate) fn damage(&mut self, env: &Env, data: &SideData, amount: i32, flags: u32) -> i32 {
        // BTL_DMG_* (include/battle/btl_char_member.h).
        const NO_DEFENSE: u32 = 0x1;
        const EXACT: u32 = 0x2;
        const DRAIN: u32 = 0x4;
        const NO_HIT: u32 = 0x8;
        const NO_TOTAL: u32 = 0x10;
        const NO_TIMER: u32 = 0x20;
        const ANY_STATE: u32 = 0x40;
        const NO_HALVE: u32 = 0x80;
        const NO_SCALING: u32 = 0x100;
        const FORCE: u32 = 0x200;
        const NO_KILL: u32 = 0x400;
        let before = self.gauge.health;
        let mut amount = amount;
        if flags & NO_HALVE == 0 && env.anim_flags(self.anim.cur) & 0x800 != 0 {
            amount /= 2;
        }
        if flags & NO_SCALING == 0 {
            let extra = self.combo.hits - 4;
            if extra > 0 {
                let pct = (100 - extra * 3).max(50);
                amount = amount * pct / 100;
            }
        }
        if flags & NO_DEFENSE == 0 {
            let scaled = (amount as f32 * data.tables.params.f32_at(0x7C)) as i32;
            amount = if scaled <= 0 && amount > 0 { 1 } else { scaled };
            let scaled = (amount as f32 * env.curve_base(7)) as i32;
            amount = if scaled <= 0 && amount > 0 { 1 } else { scaled };
        }
        if amount <= 0 {
            if flags & FORCE == 0 {
                return 0;
            }
            amount = 0;
        }
        if !self.flags.test(3) && flags & ANY_STATE == 0 {
            return 0;
        }
        if flags & EXACT == 0 && amount >= 10 {
            let rem = amount % 10;
            if rem != 0 {
                amount = amount - rem + 10;
            }
        }
        let health = self.gauge.health;
        if matches!(env.mode, 5 | 6) || flags & (DRAIN | NO_KILL) != 0 {
            amount = amount.min(health - 1);
        }
        self.gauge.health = health - amount;
        if amount > 0 || flags & FORCE != 0 {
            let c = &mut self.combo;
            if flags & NO_HIT != 0 {
                if flags & NO_TOTAL == 0 {
                    c.damage += amount;
                    if flags & NO_TIMER == 0 {
                        c.timer = ((c.damage as f32 * 30.0 * 0.0001) as i32 + 30).min(90);
                        c.changed = true;
                    }
                }
            } else {
                c.hits += 1;
                c.damage = if c.hits == 1 {
                    amount
                } else {
                    c.damage + amount
                };
                c.timer = ((c.damage as f32 * 30.0 * 0.0001) as i32 + 30).min(90);
                c.new_hit = true;
                c.changed = true;
            }
        }
        if self.gauge.health < 0 {
            self.gauge.health = 0;
        }
        if amount > 0 {
            self.took_damage = true;
        }
        if self.gauge.health == 0 {
            self.gauge.ki = 0;
            self.gauge.blast = 0;
            self.gauge.max_power = 0;
            self.stun_timer = 0;
            self.flags.clear(0xBE);
        }
        before - self.gauge.health
    }

    // ---- movement (btl_char_move.c) --------------------------------------------------------------------------

    /// `BtlMoveParam_GetSpeed`: the table's 10 km/h, times the dash factor for kinds 4 / 5 and curve row 8 (not
    /// for 15 / 19), at most 4000 km/h.
    pub(crate) fn move_speed(&self, data: &SideData, env: &Env, n: usize) -> f32 {
        let mut v = data.tables.moves.speed[n] * 10.0 * kmh(1.0);
        match n {
            15 | 19 => {}
            4 | 5 => v *= self.pose.dash_boost * env.curve_base(8),
            _ => v *= env.curve_base(8),
        }
        v.min(kmh(4000.0))
    }

    /// `BtlMoveParam_GetTurnRate`: degrees per second as radians per frame, times the dash factor for kind 0.
    pub(crate) fn turn_rate(&self, data: &SideData, n: usize) -> f32 {
        let mut v = data.tables.moves.turn_rate[n] * PI / 180.0 * (1.0 / 30.0);
        if n == 0 {
            v *= self.pose.dash_boost;
        }
        v
    }

    /// `BtlMove_TurnYaw`: turns the heading yaw towards the mode's target by at most `max_step`.
    pub(crate) fn turn_yaw(&mut self, opp: &Chr, mode: u32, max_step: f32) {
        let p = &self.pose;
        let mut target = p.yaw;
        match mode {
            0 => {
                if self.input.is_held(crate::btl_input::DIR_MASK) {
                    let [x, y] = self.input.stick();
                    target = wrap_angle(x.atan2(-y) + self.cam.yaw);
                }
            }
            1 => target = wrap_angle(p.yaw + max_step * self.input.stick()[0]),
            2 | 4 => {
                if self.flags.test(5) {
                    target = opp_yaw(self, opp);
                }
            }
            3 => {
                if self.flags.test(5) {
                    target = wrap_angle(opp_yaw(self, opp) + PI);
                }
            }
            9 => target = opp.snap_rot(6)[1],
            7 => target = (-p.pos[0]).atan2(-p.pos[2]),
            8 => target = p.pos[0].atan2(p.pos[2]),
            _ => {}
        }
        let d = wrap_angle(target - self.pose.yaw).clamp(-max_step, max_step);
        self.pose.yaw = wrap_angle(self.pose.yaw + d);
        self.pose.steer_yaw = 0.0;
    }

    /// `BtlMove_TurnPitch`: modes 0 stick, 1..4 at the opponent, 5 level, 6 keep.
    pub(crate) fn turn_pitch(
        &mut self,
        data: &SideData,
        opp: &Chr,
        opp_data: &SideData,
        mode: u32,
        max_step: f32,
    ) {
        let mut target = self.pose.pitch;
        match mode {
            0 => {
                target = (self.pose.pitch - max_step * self.input.stick()[1])
                    .clamp(-PITCH_MAX, PITCH_MAX)
            }
            5 => target = 0.0,
            1..=4 => {
                if self.flags.test(5) {
                    let mut to = opp_delta(self, opp);
                    to[1] += data.height() * 0.5 - opp_data.height() * 0.5;
                    if mode == 4 {
                        let len = length(to);
                        if len < 50.0 {
                            to[1] *= len / 50.0;
                        }
                    }
                    if mode == 3 {
                        to[1] += to[1];
                    }
                    let eps = 0.01;
                    let vel = self.pose.vel;
                    let opp_vel = opp.pose.vel;
                    let (speed, opp_speed) = (length(vel), length(opp_vel));
                    if eps < speed && eps < opp_speed {
                        let n = scale3(vel, 1.0 / speed);
                        let n_opp = scale3(opp_vel, 1.0 / opp_speed);
                        let n_to = normalize(to);
                        let mut dist = length(to);
                        dist -= (1.0 - n_to[1].abs()) * data.radius();
                        dist -= (1.0 - n_to[1].abs()) * opp_data.radius();
                        dist = dist.max(0.0);
                        let closing = speed - opp_speed * dot(n, n_opp);
                        if eps < closing {
                            let t = dist / closing;
                            if t < 1.0 {
                                to[1] *= t;
                            }
                        }
                    }
                    let n = normalize(to);
                    target = -n[1].clamp(-1.0, 1.0).asin();
                    if mode == 2 {
                        target *= (-self.input.stick()[1]).max(0.0);
                    }
                }
            }
            _ => {}
        }
        let d = wrap_angle(target - self.pose.pitch).clamp(-max_step, max_step);
        self.pose.pitch = wrap_angle(self.pose.pitch + d);
        self.pose.steer_pitch = 0.0;
    }

    /// `BtlMove_TurnModelYaw`: the model yaw follows the heading by |difference| * rate, 1.8..max degrees.
    pub(crate) fn turn_model_yaw(&mut self, max_step: f32, rate: f32) {
        let p = &mut self.pose;
        let d = wrap_angle(p.yaw - p.rot[1]);
        let step = (d.abs() * rate).max(deg(1.8)).min(max_step);
        if d < -step {
            p.rot[1] = wrap_angle(p.rot[1] - step);
        } else if step < d {
            p.rot[1] = wrap_angle(p.rot[1] + step);
        } else {
            p.rot[1] = p.yaw;
        }
    }

    /// `BtlMove_TurnToPoint`: heading yaw and pitch toward `target` by at most the steps.
    pub(crate) fn turn_to_point(&mut self, target: [f32; 3], yaw_step: f32, pitch_step: f32) {
        let p = &mut self.pose;
        let mut d = sub(target, p.pos);
        let eps = 0.001;
        if eps < length(d) {
            d = normalize(d);
        }
        let ang = if eps < d[0].abs() || eps < d[2].abs() {
            d[0].atan2(d[2])
        } else {
            0.0
        };
        p.yaw = wrap_angle(p.yaw + wrap_angle(ang - p.yaw).clamp(-yaw_step, yaw_step));
        let pitch = -d[1].clamp(-1.0, 1.0).asin();
        p.pitch = wrap_angle(p.pitch + wrap_angle(pitch - p.pitch).clamp(-pitch_step, pitch_step));
        p.steer_yaw = 0.0;
        p.steer_pitch = 0.0;
    }

    /// `BtlMove_SetHeading`.
    pub(crate) fn set_heading(&mut self, yaw: f32, pitch: f32) {
        self.pose.yaw = yaw;
        self.pose.rot[1] = yaw;
        self.pose.pitch = pitch;
    }

    /// `BtlMove_SetDirection`: the unit direction of travel by mode (0 / 1 stick and camera, 2 / 3 heading, 4 / 5
    /// reversed, 6 / 7 keep).
    pub(crate) fn set_direction(&mut self, mode: u32) {
        let p = &mut self.pose;
        if length(p.dir) < 0.001 {
            p.dir = [p.yaw.sin(), 0.0, p.yaw.cos()];
        }
        let v = match mode {
            0 | 1 => {
                if self.input.is_held(crate::btl_input::DIR_MASK) {
                    let [x, y] = self.input.stick();
                    let y = -y;
                    let a = -self.cam.yaw;
                    let (s, c) = a.sin_cos();
                    let (sp, cp) = if mode == 1 {
                        p.pitch.sin_cos()
                    } else {
                        (0.0, 1.0)
                    };
                    [cp * (x * c - y * s), -sp, cp * (x * s + y * c)]
                } else {
                    p.dir
                }
            }
            2 => [p.yaw.sin(), 0.0, p.yaw.cos()],
            3 => {
                let (sp, cp) = p.pitch.sin_cos();
                [cp * p.yaw.sin(), -sp, cp * p.yaw.cos()]
            }
            4 => [-p.yaw.sin(), 0.0, -p.yaw.cos()],
            5 => {
                let (sp, cp) = p.pitch.sin_cos();
                [-cp * p.yaw.sin(), sp, -cp * p.yaw.cos()]
            }
            6 => [p.dir[0], 0.0, p.dir[2]],
            _ => p.dir,
        };
        p.dir = normalize(v);
    }

    /// `BtlMove_Advance`: the speed approaches `speed` by `accel`, then pos += dir * speed.
    pub(crate) fn advance(&mut self, speed: f32, accel: f32) {
        let p = &mut self.pose;
        p.speed = approach(p.speed, speed, accel);
        p.pos = add(p.pos, scale3(p.dir, p.speed));
    }

    /// `BtlMove_Step`: heading at once, the model by 36 degrees at most, the direction, the advance.
    pub(crate) fn step(
        &mut self,
        data: &SideData,
        opp: &Chr,
        opp_data: &SideData,
        yaw_mode: u32,
        pitch_mode: u32,
        dir_mode: u32,
        speed: f32,
        accel: f32,
    ) {
        self.turn_yaw(opp, yaw_mode, PI);
        self.turn_pitch(data, opp, opp_data, pitch_mode, PI);
        self.turn_model_yaw(deg(36.0), 0.3);
        self.set_direction(dir_mode);
        self.advance(speed, accel);
    }

    /// `BtlMove_Fall`: 50 km/h more per frame, at most 3000 km/h.
    pub(crate) fn fall(&mut self) {
        let p = &mut self.pose;
        p.fall_speed = (p.fall_speed + kmh(50.0)).min(kmh(3000.0));
        p.pos[1] += p.fall_speed;
    }

    /// `BtlMove_MoveVertical`.
    pub(crate) fn move_vertical(&mut self, speed: f32, accel: f32) {
        let p = &mut self.pose;
        p.fall_speed = approach(p.fall_speed, speed, accel);
        p.pos[1] += p.fall_speed;
    }

    /// `BtlMove_BrakeVertical`.
    pub(crate) fn brake_vertical(&mut self) {
        self.move_vertical(0.0, kmh(100.0));
    }

    /// `BtlMove_ApplyGravity`: in flight (flag 0xE) braked, else falling.
    pub(crate) fn apply_gravity(&mut self) {
        if self.flags.test(0xE) {
            self.brake_vertical();
        } else {
            self.fall();
        }
    }

    /// `BtlMove_SetLeanX`.
    pub(crate) fn set_lean_x(&mut self, angle: f32) {
        self.pose.lean_x = angle.clamp(-PITCH_MAX, PITCH_MAX);
    }

    /// `BtlMove_RequestOrbit`: lock-on only.
    pub(crate) fn request_orbit(&mut self, near: f32, far: f32) {
        if self.flags.test(5) {
            self.flags.set(0x19);
            self.pose.orbit_near = near;
            self.pose.orbit_far = far;
        }
    }

    /// `BtlMove_SetImpulseYaw` (`BtlMove_SetImpulse`: unless the current one is larger).
    pub(crate) fn set_impulse_yaw(&mut self, yaw: f32, speed: f32) {
        let v = scale3([yaw.sin(), 0.0, yaw.cos()], speed);
        if length(self.pose.impulse) < length(v) {
            self.pose.impulse = v;
        }
    }

    /// `BtlMove_SetImpulseDir`.
    pub(crate) fn set_impulse_dir(&mut self, dir: [f32; 3], speed: f32) {
        let v = scale3(dir, speed);
        if length(self.pose.impulse) < length(v) {
            self.pose.impulse = v;
        }
    }

    /// `BtlMove_ApplyImpulse`: pos += impulse, which loses 2000 km/h per second.
    pub(crate) fn apply_impulse(&mut self, time_stopped: bool) {
        if time_stopped {
            return;
        }
        const DECEL: f32 = kmh(2000.0) * (1.0 / 30.0);
        let p = &mut self.pose;
        p.pos = add(p.pos, p.impulse);
        let len = length(p.impulse);
        if len < DECEL {
            p.impulse = [0.0; 3];
        } else {
            p.impulse = scale3(p.impulse, (len - DECEL) / len);
        }
    }

    /// `BtlMove_UpdateHoverOffset`: the display offset: a 2-unit bob while flying idle, the sideways shake.
    pub(crate) fn update_hover_offset(&mut self, time_stopped: bool) {
        let fly = self.flags.test(0xE) && self.flags.test(0x1C);
        let p = &mut self.pose;
        p.disp = [0.0; 3];
        if self.flags.test(0x29) {
            p.hover_phase = 0.0;
        }
        if time_stopped {
            return;
        }
        if fly {
            p.hover_phase = wrap_angle(p.hover_phase + deg(6.0));
        } else {
            if FRAC_PI_2 < p.hover_phase {
                p.hover_phase = PI - p.hover_phase;
            }
            if p.hover_phase < -FRAC_PI_2 {
                p.hover_phase = -PI - p.hover_phase;
            }
            if 0.0 < p.hover_phase {
                p.hover_phase = (p.hover_phase - deg(6.0)).max(0.0);
            }
            if p.hover_phase < 0.0 {
                p.hover_phase = (p.hover_phase + deg(6.0)).min(0.0);
            }
        }
        p.disp[1] = p.hover_phase.sin() * 2.0;
        if self.shake_timer > 0 {
            let k = ((self.shake_timer & 1) as f32 - 0.5) * 0.5;
            p.disp[0] = -p.yaw.cos() * k;
            p.disp[2] = p.yaw.sin() * k;
        }
    }

    // ---- queries (btl_char_action.c) ---------------------------------------------------------------------------

    /// `BtlAct_GetGroundY`.
    pub(crate) fn ground_y(&self) -> f32 {
        self.pose.ground_pos[1]
    }

    /// `BtlAct_GetHeight`: height above the ground (+Y is down).
    pub(crate) fn height_above_ground(&self) -> f32 {
        self.ground_y() - self.pose.pos[1]
    }

    /// `BtlAct_GetHeightRatio`.
    pub(crate) fn height_ratio(&self, data: &SideData, env: &Env) -> f32 {
        let half = data.height();
        let top = env.stage.bounds.top + half * 0.5;
        let bottom = env.stage.bounds.bottom + half * 0.5;
        let ground = self.ground_y().min(bottom);
        (ground - self.pose.pos[1]) / (ground - top)
    }

    /// `BtlAct_GetFramesToGround`.
    pub(crate) fn frames_to_ground(&self) -> f32 {
        if self.flags.test(0xF) {
            return 0.0;
        }
        let fall = self.pose.moved[1];
        if fall < 0.001 {
            return 1_000_000.0;
        }
        let h = self.height_above_ground();
        if h < 0.0 { 0.0 } else { h / fall }
    }

    /// `BtlAct_GetRotYRelCam`.
    pub(crate) fn rot_y_rel_cam(&self) -> f32 {
        wrap_angle(self.pose.rot[1] - self.cam.yaw)
    }

    /// `BtlAct_GetFacingRelCam`.
    pub(crate) fn facing_rel_cam(&self) -> f32 {
        wrap_angle(self.pose.yaw - self.cam.yaw)
    }

    /// `BtlAct_GetVelDirRelCam`: the direction of travel relative to the camera yaw.
    pub(crate) fn vel_dir_rel_cam(&self) -> f32 {
        wrap_angle(self.pose.dir[0].atan2(self.pose.dir[2]) - self.cam.yaw)
    }

    /// `BtlAct_IsAirMotion`.
    pub(crate) fn is_air_motion(&self, env: &Env, use_saved: bool) -> bool {
        if self.flags.test(0x35) {
            return true;
        }
        if self.flags.test(0x36) {
            return false;
        }
        if use_saved && self.react.reaction != 1 {
            return self.react.back;
        }
        env.anim_flags(self.anim.cur) & 0x8000 != 0
    }

    /// `BtlInput_TestAction` with this fighter's flags and facing.
    pub(crate) fn test(&mut self, id: u32, want: bool) -> bool {
        let facing = self.facing_rel_cam();
        self.input.test_action(&mut self.flags, facing, id, want)
    }
}

// ---- the opponent (btl_opponent.c) and two-fighter steps -------------------------------------------------------

/// `BtlOpp_GetDelta`: the opponent's target position (snapshot 6) minus the own position.
pub(crate) fn opp_delta(me: &Chr, opp: &Chr) -> [f32; 3] {
    sub(opp.snap_pos(6), me.pose.pos)
}

/// `BtlMove_SteerAtOpponent`: the homing dash heads for the opponent with an offset the stick steers (pi/2 at
/// full stick, less within two frames of contact), clamped to `yaw_max` / `pitch_max` and approached by
/// `yaw_accel` / `pitch_accel` per frame. Without lock-on it only keeps the heading.
#[allow(clippy::too_many_arguments)]
pub(crate) fn steer_at_opponent(
    me: &mut Chr,
    opp: &Chr,
    data: &SideData,
    opp_data: &SideData,
    close_speed: f32,
    yaw_accel: f32,
    pitch_accel: f32,
    yaw_max: f32,
    pitch_max: f32,
    max_step: f32,
) {
    if !me.flags.test(5) {
        me.turn_yaw(opp, 6, max_step);
        me.turn_pitch(data, opp, opp_data, 6, max_step);
        return;
    }
    let mut to = opp_delta(me, opp);
    to[1] += data.height() * 0.5 - opp_data.height() * 0.5;
    let dist = (length(to) - data.radius() * 0.5 - opp_data.radius() * 0.5).max(0.0);
    let (vel, opp_vel) = (me.pose.vel, opp.pose.vel);
    let (speed, opp_speed) = (length(vel), length(opp_vel));
    let mut close_speed = close_speed;
    if 0.001 < speed && 0.001 < opp_speed {
        close_speed -= opp_speed * (dot(vel, opp_vel) / (speed * opp_speed));
    }
    let t = if 0.001 < close_speed {
        dist / close_speed
    } else {
        1_000_000.0
    };
    let k = if 2.0 < t { 1.0 } else { t * 0.5 };
    let [stick_x, stick_y] = me.input.stick();
    let p = &mut me.pose;
    let goal = (k * 1.570_796_3 * stick_x).clamp(-yaw_max, yaw_max);
    p.steer_yaw = approach(p.steer_yaw, goal, (goal - p.steer_yaw).abs().min(yaw_accel));
    let goal = wrap_angle(to[0].atan2(to[2]) + p.steer_yaw);
    p.yaw = wrap_angle(p.yaw + wrap_angle(goal - p.yaw).clamp(-max_step, max_step));
    let n = normalize(to);
    let base = -n[1].clamp(-1.0, 1.0).asin();
    let goal = (-k * 1.570_796_3 * stick_y).clamp(-pitch_max, pitch_max);
    p.steer_pitch = approach(
        p.steer_pitch,
        goal,
        (goal - p.steer_pitch).abs().min(pitch_accel),
    );
    let goal = wrap_angle(base + p.steer_pitch);
    p.pitch = wrap_angle(p.pitch + wrap_angle(goal - p.pitch).clamp(-max_step, max_step));
    if t < 1.0 {
        p.pitch *= t;
    }
}

/// `BtlOpp_GetGapXZ`: the horizontal distance less both radii, never negative.
pub(crate) fn opp_gap_xz(me: &Chr, opp: &Chr, data: &SideData, opp_data: &SideData) -> f32 {
    (opp_distance_xz(me, opp) - data.radius() - opp_data.radius()).max(0.0)
}

/// `BtlOpp_GetDistance`.
pub(crate) fn opp_distance(me: &Chr, opp: &Chr) -> f32 {
    length(opp_delta(me, opp))
}

/// `BtlOpp_GetDistanceXZ`.
pub(crate) fn opp_distance_xz(me: &Chr, opp: &Chr) -> f32 {
    let d = opp_delta(me, opp);
    d[0].hypot(d[2])
}

/// `BtlOpp_GetYaw`: 0 straight above or below.
pub(crate) fn opp_yaw(me: &Chr, opp: &Chr) -> f32 {
    let d = opp_delta(me, opp);
    if d[0].abs() < 0.0001 && d[2].abs() < 0.0001 {
        0.0
    } else {
        d[0].atan2(d[2])
    }
}

/// `BtlMove_WarpBehindOpponent`, under lock-on only: the opponent's next position (its target position plus its
/// last movement), both radii and `gap` behind it.
pub(crate) fn warp_behind_opponent(
    me: &mut Chr,
    data: &SideData,
    opp: &Chr,
    opp_data: &SideData,
    gap: f32,
) {
    if !me.flags.test(5) {
        return;
    }
    let at = add(opp.snap_pos(6), opp.pose.vel);
    let yaw = opp.snap_rot(6)[1];
    let d = data.radius() + opp_data.radius() + gap;
    me.pose.pos = add(at, [-yaw.sin() * d, 0.0, -yaw.cos() * d]);
}

/// `BtlMove_WarpAheadOfOpponent`, under lock-on only: on the opponent's path `lead` frames ahead (its movement, at
/// least 500 km/h along its facing), both radii further along it in x / z, pulled inside the stage radius.
pub(crate) fn warp_ahead_of_opponent(
    me: &mut Chr,
    data: &SideData,
    opp: &Chr,
    opp_data: &SideData,
    env: &Env,
    lead: f32,
) {
    if !me.flags.test(5) {
        return;
    }
    let min = kmh(500.0);
    let mut at = opp.snap_pos(6);
    at[1] += data.height() * 0.5 - opp_data.height() * 0.5;
    let mut vel = opp.pose.vel;
    if length(vel) < min {
        let yaw = opp.snap_rot(6)[1];
        vel = [-yaw.sin() * min, 0.0, -yaw.cos() * min];
    }
    at = add(at, scale3(vel, lead));
    let mut ofs = vel;
    let len = ofs[0].hypot(ofs[2]);
    if 0.0001 < len {
        let k = (data.radius() + opp_data.radius()) / len;
        ofs[0] *= k;
        ofs[2] *= k;
    }
    me.pose.pos = add(at, ofs);
    let rad = env.inner_radius() - me.obj.body_sphere().1;
    let lim = me.pose.pos[0].hypot(me.pose.pos[2]);
    if rad < lim {
        me.pose.pos = scale3(me.pose.pos, rad / lim);
    }
}

/// `BtlOpp_GetClosingTime`: frames until the two meet at their velocities, 10000000 when they are not closing.
pub(crate) fn opp_closing_time(me: &Chr, opp: &Chr) -> f32 {
    let eps = 0.001;
    let d = opp_delta(me, opp);
    let dist = length(d);
    if dist < eps {
        return 0.0;
    }
    let dir = scale3(d, 1.0 / dist);
    let speed = dot(me.pose.vel, dir);
    if speed < 0.0 {
        return 10_000_000.0;
    }
    let rel = speed - dot(opp.pose.vel, dir);
    if rel < eps {
        return 10_000_000.0;
    }
    dist / rel
}

/// `BtlOpp_GetPitchAdjusted`: the pitch toward the opponent, centre to centre.
pub(crate) fn opp_pitch_adjusted(me: &Chr, opp: &Chr, data: &SideData, opp_data: &SideData) -> f32 {
    let mut d = opp_delta(me, opp);
    d[1] += data.height() * 0.5 - opp_data.height() * 0.5;
    let len = length(d);
    if 0.001 < len {
        -(d[1] / len).clamp(-1.0, 1.0).asin()
    } else {
        0.0
    }
}

/// `BtlMove_CalcApproachPoint`: the opponent's position `lead` frames ahead (stopped by the stage on the way,
/// kept between the stage's top and bottom), and the point `dist * dist_scale` plus both radii short of it, `angle`
/// below the horizontal line to it (kept below the top). Returns (point, target).
#[allow(clippy::too_many_arguments)]
pub(crate) fn calc_approach_point(
    me: &Chr,
    opp: &Chr,
    data: &SideData,
    opp_data: &SideData,
    env: &Env,
    dist: f32,
    angle: f32,
    dist_scale: f32,
    lead: f32,
) -> ([f32; 3], [f32; 3]) {
    let at = opp.snap_pos(6);
    let mut tgt = add(at, scale3(opp.pose.vel, lead));
    let scale = data.height();
    let opp_scale = opp_data.height();
    tgt[1] += data.height() * 0.5 - opp_data.height() * 0.5;
    if let Some(hit) = env.col.trace_segment(at, tgt) {
        let e = sub(tgt, hit.pos);
        let len3 = length(e);
        let len_xz = e[0].hypot(e[2]);
        if 0.001 < len_xz {
            let k = len3 / len_xz;
            tgt[0] = hit.pos[0] + e[0] * k;
            tgt[2] = hit.pos[2] + e[2] * k;
        }
        tgt[1] = hit.pos[1];
    }
    let top = env.stage.bounds.top + opp_scale * 0.5;
    if tgt[1] < top {
        tgt[1] = top;
    }
    let bottom = env.stage.bounds.bottom + opp_scale * 0.5;
    if bottom < tgt[1] {
        let mut h = sub(tgt, at);
        let len3 = length(h);
        let len_xz = h[0].hypot(h[2]);
        if 0.001 < len_xz && 0.001 < h[1].abs() {
            let k = (tgt[1] - bottom) / h[1];
            if 0.0 < k {
                let m = (len_xz * (1.0 - k) + len3 * k) / len_xz;
                h[0] *= m;
                h[2] *= m;
                tgt[0] = at[0] + h[0];
                tgt[2] = at[2] + h[2];
            }
        }
        tgt[1] = bottom;
    }
    let mut d = sub(tgt, me.pose.pos);
    d[1] = 0.0;
    let len = length(d);
    let mut d = if 0.01 < len {
        let k = angle.cos() / len;
        [d[0] * k, -angle.sin(), d[2] * k]
    } else {
        [0.0, -angle.sin(), angle.cos()]
    };
    let len = dist_scale * dist + data.radius() + opp_data.radius();
    d = scale3(d, len);
    let mut out = sub(tgt, d);
    let top = env.stage.bounds.top + scale * 0.5;
    if out[1] < top {
        let flat = d[0].hypot(d[2]);
        let rise = top - tgt[1];
        // The EE's sqrt.s takes the root of the magnitude.
        let r = (length(d) * length(d) - rise * rise).abs().sqrt();
        if 0.01 < flat {
            let k = r / flat;
            out = [tgt[0] - d[0] * k, top, tgt[2] - d[2] * k];
        } else {
            out = [tgt[0], top, tgt[2] - r];
        }
    }
    (out, tgt)
}

/// `BtlMove_SnapToOpponent`: moves the fighter so that its body sphere's centre is on the opponent's, stopped, with
/// the body warped there (flag 0x55), no push-out (0x3F) and no velocity this frame (0x24).
pub(crate) fn snap_to_opponent(me: &mut Chr, opp: &Chr) {
    let own = me.obj.body_sphere().0;
    let at = opp.obj.body_sphere().0;
    me.pose.pos = add(me.pose.pos, sub(at, own));
    me.pose.speed = 0.0;
    me.pose.fall_speed = 0.0;
    me.pose.impulse = [0.0; 3];
    me.body_warp_pos = at;
    me.flags.set(0x55);
    me.flags.set(0x3F);
    me.flags.set(0x24);
}

/// `BtlMove_WarpInFrontOfOpponent`, under lock-on only: in front of the opponent's next position, both radii
/// plus `gap` away.
pub(crate) fn warp_in_front_of_opponent(
    me: &mut Chr,
    data: &SideData,
    opp: &Chr,
    opp_data: &SideData,
    gap: f32,
) {
    if !me.flags.test(5) {
        return;
    }
    let at = add(opp.snap_pos(6), opp.pose.vel);
    let yaw = opp.snap_rot(6)[1];
    let d = data.radius() + opp_data.radius() + gap;
    me.pose.pos = add(at, [yaw.sin() * d, 0.0, yaw.cos() * d]);
}

/// `BtlOpp_GetYawFromItsFacing`: the yaw to the opponent relative to the opponent's own facing.
pub(crate) fn opp_yaw_from_its_facing(me: &Chr, opp: &Chr) -> f32 {
    wrap_angle(opp_yaw(me, opp) - opp.snap_rot(6)[1])
}

/// `BtlMove_ClipToRadius`: where the segment from -> to leaves the vertical cylinder of `radius` (in x / z).
fn clip_to_radius(from: [f32; 3], to: [f32; 3], radius: f32) -> Option<[f32; 3]> {
    let p = [from[0], 0.0, from[2]];
    let q = [to[0], 0.0, to[2]];
    let d = sub(q, p);
    let a = dot(d, d);
    let b = 2.0 * dot(p, d);
    let c = dot(p, p) - radius * radius;
    let disc = b * b - a * 4.0 * c;
    if disc < 0.0 || a < 0.001 {
        return None;
    }
    let t = (disc.sqrt() - b) / (a + a);
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    // Vec4_Lerp(out, to, from, t) = to * t + from * (1 - t).
    Some(add(scale3(to, t), scale3(from, 1.0 - t)))
}

/// `BtlMove_CircleOpponent`: flies round the opponent on a 200 circle toward the point opposite `base_yaw` (`side`
/// picks the way round; `lead` leads a moving opponent), clipped to the stage radius; true on the frame the point
/// is reached.
#[allow(clippy::too_many_arguments)]
pub(crate) fn circle_opponent(
    me: &mut Chr,
    opp: &Chr,
    data: &SideData,
    opp_data: &SideData,
    env: &Env,
    side: bool,
    lead: bool,
    base_yaw: f32,
    speed: f32,
) -> bool {
    let mut at = opp.snap_pos(6);
    at[1] += data.height() * 0.5 - opp_data.height() * 0.5;
    let radius = 200.0;
    let d = sub(at, me.pose.pos);
    let mut diff = 0.0;
    if 0.01 < d[0].abs() || 0.01 < d[2].abs() {
        diff = wrap_angle(base_yaw - d[0].atan2(d[2]));
    }
    let mut diff = diff.abs();
    let mut scale = 1.0;
    if diff < 1.570_796_3 {
        scale = 0.0;
        if deg(18.0) < diff {
            scale = (diff - deg(18.0)).sin();
        }
    }
    let t = radius * diff / speed;
    if lead {
        let v = opp.pose.vel;
        if kmh(10.0) < v[0].hypot(v[2]) {
            let k = (wrap_angle(base_yaw - v[0].atan2(v[2])).abs() - 1.570_796_3) / 1.570_796_3;
            if 0.0 < k {
                at = add(at, scale3(v, t * k));
            }
        }
    }
    if !side {
        diff = -diff;
    }
    let a = wrap_angle(diff * 0.5 + wrap_angle(base_yaw + PI));
    let mut tgt = add(
        [a.sin() * radius * scale, 0.0, a.cos() * radius * scale],
        at,
    );
    if let Some(hit) = clip_to_radius(at, tgt, env.inner_radius()) {
        tgt = hit;
    }
    me.turn_to_point(tgt, PI, PI);
    me.turn_model_yaw(1.570_796_3, 0.5);
    me.set_direction(3);
    let p = &mut me.pose;
    p.speed = speed;
    let ahead = dot(sub(tgt, p.pos), p.dir).max(0.0);
    let mut reached = false;
    if ahead < p.speed {
        p.speed = ahead;
        reached = true;
    }
    p.pos = add(p.pos, scale3(p.dir, p.speed));
    reached
}

/// `BtlOpp_GetYawFromFacing`: relative to the own model yaw plus the root yaw.
pub(crate) fn opp_yaw_from_facing(me: &Chr, opp: &Chr) -> f32 {
    wrap_angle(opp_yaw(me, opp) - wrap_angle(me.pose.rot[1] + me.pose.root_rot[1]))
}

/// `BtlChar_GetSpacing`: both heights and a margin (40 in mode 0 under flag 0x13, else 30).
pub(crate) fn spacing(me: &Chr, data: &SideData, opp_data: &SideData, mode: u32) -> f32 {
    let margin = if mode == 0 && me.flags.test(0x13) {
        40.0
    } else {
        30.0
    };
    data.height() + opp_data.height() + margin
}

/// `BtlChar_UpdateLockOn`: flags 0x13 (close) and 5 (lock-on) while the opponent is within the spacing.
pub(crate) fn update_lock_on(me: &mut Chr, opp: &Chr, data: &SideData, opp_data: &SideData) {
    if me.flags.test(0x93) || me.flags.test(0xB2) {
        me.flags.clear(5);
    }
    me.flags.clear(0x13);
    if !me.flags.test(0x1B) && !me.flags.test(0x93) && me.react.reaction < 3 && !me.flags.test(0xB2)
    {
        let range = spacing(me, data, opp_data, 0);
        if opp_distance(me, opp) < range {
            me.flags.set_held(0x13);
            me.flags.set_held(5);
        }
    }
}

/// `BtlMove_IsBlockedByOpponent`: the push-out moved the fighter this frame and the two overlap in height.
pub(crate) fn is_blocked_by_opponent(
    me: &Chr,
    opp: &Chr,
    data: &SideData,
    opp_data: &SideData,
) -> bool {
    if !me.flags.test(0x5E) {
        return false;
    }
    if me.flags.test(0x5F) {
        return true;
    }
    let to = opp_delta(me, opp);
    if 0.0 < to[1] {
        to[1] < opp_data.height() * 0.5
    } else {
        -to[1] < data.height() * 0.5
    }
}

/// `BtlMove_PushOut`: a vertical plane between the two at both radii, shared by how much each moved towards the
/// other this frame.
pub(crate) fn push_out(me: &mut Chr, opp: &Chr, data: &SideData, opp_data: &SideData) -> bool {
    if me.flags.test(0x3F) || me.flags.test(0xAE) {
        return false;
    }
    let mut check_height = !me.flags.test(5);
    if me.flags.test(0x1A) || me.flags.test(0xAD) {
        check_height = true;
    }
    let own0 = me.snap_pos(0);
    let opp0 = opp.snap_pos(0);
    let d = sub(opp0, own0);
    if check_height && (d[1] < -data.height() || opp_data.height() < d[1]) {
        return false;
    }
    let len = d[0].hypot(d[2]);
    if len < 0.001 {
        return false;
    }
    let n = [d[0] / len, 0.0, d[2] / len];
    let opp_a = opp.snap_pos(0);
    let opp_b = opp.snap_pos(1);
    let opp_move = [opp_b[0] - opp_a[0], 0.0, opp_b[2] - opp_a[2]];
    let base = if dot(opp_move, n) < 0.0 { opp_a } else { opp_b };
    let plane = [-n[0], 0.0, -n[2]];
    let w = -dot(plane, base) - data.radius() - opp_data.radius();
    let mut mv = me.snap_delta(0, 1);
    let oc = opp.snap_pos(1);
    let mut rel = sub(oc, base);
    mv[1] = 0.0;
    rel[1] = 0.0;
    let h0 = dot(n, mv).max(0.0);
    let h1 = (-dot(n, rel)).max(0.0);
    let total = h0 + h1;
    let ratio = if total < 0.001 { 0.5 } else { h0 / total };
    let depth = (dot(me.pose.pos, plane) + w + h0) * ratio - h0;
    if depth < 0.0 {
        me.pose.pos = add(me.pose.pos, scale3(n, depth));
        me.flags.set(0x5E);
        if check_height {
            me.flags.set(0x5F);
        }
        return true;
    }
    false
}

/// `BtlMove_ApplyOrbit`: under flag 0x19 this frame's movement keeps the distance to the opponent and turns the
/// heading by the angle travelled (fully beyond `far`, not inside `near`).
pub(crate) fn apply_orbit(me: &mut Chr, opp: &Chr) {
    if !(me.flags.test(5) && me.flags.test(0x19)) {
        return;
    }
    let (near, far) = (me.pose.orbit_near, me.pose.orbit_far);
    let base = me.snap_pos(0);
    let cur = me.pose.pos;
    let target = opp.snap_pos(6);
    let mut d_base = sub(target, base);
    d_base[1] = 0.0;
    let mut d_cur = sub(target, cur);
    d_cur[1] = 0.0;
    let len_base = length(d_base);
    let len = length(d_cur);
    let mut w = 1.0;
    if !(far < len) {
        if !(near < len) {
            return;
        }
        w = (len - near) / (far - near);
    }
    if len_base < near {
        return;
    }
    let s = scale3(d_base, 1.0 - dot(d_base, d_cur) / (len_base * len_base));
    let e = add(d_cur, s);
    let len_e = length(e);
    if len_e < 0.001 {
        return;
    }
    me.pose.pos = add(me.pose.pos, scale3(e, (len_e - len_base) / len_e * w));
    let c = (dot(d_base, e) / (len_base * len_e)).min(1.0);
    let mut a = c.acos();
    if w < 1.0 {
        a *= w;
    }
    if d_base[2] * e[0] - d_base[0] * e[2] < 0.0 {
        a = -a;
    }
    // Vec4_RotateEuler(dir, (0, a, 0), dir).
    let dir = me.pose.dir;
    let r = apply(&rot_y(a), [dir[0], dir[1], dir[2], 0.0]);
    me.pose.dir = [r[0], r[1], r[2]];
    me.pose.rot[1] = wrap_angle(me.pose.rot[1] + a);
    me.pose.yaw = wrap_angle(me.pose.yaw + a);
}

/// `BtlMove_ClampToStage`: inside the stage cylinder; flags 0x14 radius, 0x15 top, 0x16 bottom, 0x17 the y = -10
/// floor of stages 4 / 27.
pub(crate) fn clamp_to_stage(me: &mut Chr, data: &SideData, env: &Env) {
    let p = &mut me.pose;
    let r = p.pos[0].hypot(p.pos[2]);
    let lim = env.inner_radius() - me.obj.body_sphere().1;
    if lim < r {
        let k = lim / r;
        p.pos[0] *= k;
        p.pos[2] *= k;
        me.flags.set_held(0x14);
    } else {
        me.flags.clear(0x14);
    }
    if !me.flags.test(0x18) {
        let top = env.stage.bounds.top + data.height() * 0.5;
        if me.pose.pos[1] < top {
            me.pose.pos[1] = top;
            me.flags.set_held(0x15);
        } else {
            me.flags.clear(0x15);
        }
    }
    if !me.is_dead() {
        let bottom = env.stage.bounds.bottom + data.height() * 0.5;
        if bottom < me.pose.pos[1] {
            me.pose.pos[1] = bottom;
            me.flags.set_held(0x16);
        } else {
            me.flags.clear(0x16);
        }
    }
    me.flags.clear(0x17);
    if env.stage_4_or_27 {
        let free = me.is_free();
        if free && !me.flags.test(0x2A) && !me.flags.test(7) && -10.0 < me.pose.pos[1] {
            me.pose.pos[1] = -10.0;
            me.flags.set_held(0x17);
        }
        if me.flags.test(2) {
            me.pose.pos[0] = me.pose.pos[0].clamp(-185.0, 185.0);
            me.pose.pos[2] = me.pose.pos[2].clamp(-185.0, 185.0);
        }
    }
}

/// `BtlMove_AimVerticalAtOpponent` (`BtlMove_CalcVerticalSpeed`).
pub(crate) fn aim_vertical_at_opponent(
    me: &mut Chr,
    opp: &Chr,
    data: &SideData,
    opp_data: &SideData,
    lead: bool,
) {
    let to = opp_delta(me, opp);
    let extra = if lead { opp.pose.moved[1] * 0.5 } else { 0.0 };
    me.pose.fall_speed =
        vertical_speed(to[1] + data.height() * 0.5 - opp_data.height() * 0.5, extra);
}

/// `BtlMove_CalcVerticalSpeed`: the speed that covers `height` braked by 100 km/h per frame, plus `extra`,
/// within +-800 km/h.
fn vertical_speed(height: f32, extra: f32) -> f32 {
    let mut h = height;
    let mut v = 0.0;
    loop {
        if 0.0 < h {
            h -= v;
            v += kmh(100.0);
            if h < 0.0 {
                v += (1.0 - -h / v) * kmh(100.0) - kmh(100.0);
                break;
            }
        } else {
            h -= v;
            v -= kmh(100.0);
            if 0.0 < h {
                v -= (1.0 - -h / v) * kmh(100.0) - kmh(100.0);
                break;
            }
        }
    }
    (v + extra).clamp(-kmh(800.0), kmh(800.0))
}

/// `BtlMove_BeginRiseToOpponent`: once (flag 0x8A) faces the opponent, takes flight and aims for its height.
pub(crate) fn begin_rise_to_opponent(
    me: &mut Chr,
    opp: &Chr,
    data: &SideData,
    opp_data: &SideData,
) {
    if !me.flags.test(0x8A) && me.flags.test(5) {
        me.turn_yaw(opp, 2, PI);
        me.turn_pitch(data, opp, opp_data, 5, PI);
        me.set_direction(2);
        me.flags.set_held(0xE);
        aim_vertical_at_opponent(me, opp, data, opp_data, true);
        me.flags.set_held(0x81);
    }
    me.flags.set_held(0x82);
    me.flags.set_held(0x8A);
}

/// `BtlAct_ScaleSpeedByApproach`.
pub(crate) fn scale_speed_by_approach(me: &Chr, opp: &Chr, speed: f32, range: f32) -> f32 {
    if !me.flags.test(5) {
        return speed;
    }
    let mut to = opp_delta(me, opp);
    to[1] = 0.0;
    let dist = length(to).max(0.01);
    if dist < range {
        let mut dir = normalize(me.pose.dir);
        dir[1] = 0.0;
        let along = (dot(dir, to) / dist).abs();
        speed * along + speed * (1.0 - along) * (dist / range)
    } else {
        speed
    }
}

/// `ChrCam_GetSideLimit`: atan of the side slope, at least 45 degrees.
pub(crate) fn cam_side_limit(data: &SideData) -> f32 {
    data.cam_side_slope().atan().max(0.785_398_2)
}

/// `ChrCam_UpdateInput` with the lock-on (`ChrCam_UpdateLockOnYaw`) and free (`ChrCam_UpdateFreeYaw`) yaw.
pub(crate) fn cam_update_input(me: &mut Chr, opp: &Chr, data: &SideData) {
    if me.flags.test(5) {
        let mut step = wrap_angle(opp_yaw(me, opp) - me.cam.yaw);
        if !me.flags.test(0xCD) {
            step *= 0.25;
        }
        me.cam.yaw = wrap_angle(me.cam.yaw + step);
        me.cam.yaw_step = step;
        if me.flags.test(0xD2) {
            return;
        }
        let limit = cam_side_limit(data);
        let keep = me.flags.raised(5)
            || (!me.input.is_held(crate::btl_input::RS_LEFT)
                && !me.input.is_held(crate::btl_input::RS_RIGHT)
                && me.flags.test(0xD5));
        if !me.flags.raised(5) && me.input.is_held(crate::btl_input::RS_LEFT) {
            me.cam.side = -limit;
        } else if !me.flags.raised(5) && me.input.is_held(crate::btl_input::RS_RIGHT) {
            me.cam.side = limit;
        } else if keep {
            me.cam.side = if 0.0 < me.cam.side { limit } else { -limit };
        } else {
            let mut d = me.cam.yaw_step;
            if FRAC_PI_2 < d {
                d -= PI;
            }
            if d < -FRAC_PI_2 {
                d += PI;
            }
            me.cam.side = (me.cam.side + d).clamp(-limit, limit);
        }
    } else if me.flags.test(0xC9) {
        me.cam.yaw = wrap_angle(me.cam.yaw + me.input.stick()[0] * 0.033_333_333);
        if me.test(0x61, true) {
            me.cam.yaw -= PI / 60.0;
        }
        if me.test(0x62, true) {
            me.cam.yaw += PI / 60.0;
        }
    } else if me.flags.test(0xCA) {
        let d = wrap_angle(me.pose.yaw - me.cam.yaw);
        let step = ((d.abs()).sin() * 0.05).min(0.033_333_333);
        me.cam.yaw = wrap_angle(approach(me.cam.yaw, me.cam.yaw + d, step));
    }
}

/// `BtlUtil_ApproachF`.
pub(crate) fn approach(cur: f32, target: f32, step: f32) -> f32 {
    if cur < target {
        (cur + step).min(target)
    } else {
        (cur - step).max(target)
    }
}

pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale3(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

/// `Vec3_Normalize` (a zero vector stays zero).
pub(crate) fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = length(a);
    if len == 0.0 { a } else { scale3(a, 1.0 / len) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kmh_and_vertical_speed_match_the_original_units() {
        // 3600 km/h is 1000 m/s, 33.3 m per frame.
        assert!((kmh(3600.0) - 1000.0 / 30.0).abs() < 1e-4);
        // BtlMove_CalcVerticalSpeed: the sign of the height, clamped to 800 km/h either way.
        assert!(vertical_speed(10.0, 0.0) > 0.0);
        assert!(vertical_speed(-10.0, 0.0) < 0.0);
        assert_eq!(vertical_speed(1.0e4, 0.0), kmh(800.0));
        assert_eq!(vertical_speed(-1.0e4, 0.0), -kmh(800.0));
    }

    #[test]
    fn approach_stops_at_the_target() {
        assert_eq!(approach(0.0, 1.0, 0.3), 0.3);
        assert_eq!(approach(0.9, 1.0, 0.3), 1.0);
        assert_eq!(approach(1.0, -1.0, 0.5), 0.5);
    }
}
