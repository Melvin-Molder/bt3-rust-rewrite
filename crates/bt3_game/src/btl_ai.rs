//! The CPU player (src/battle/btl_ai_mgr.c, btl_ai_sense.c, btl_ai_think.c, btl_ai_seq.c): once per frame, for each
//! fighter whose input is injected, "sense" rebuilds a 64-bit situation word from both fighters, "think" walks the
//! rule lists of the common AI data until a rule queues up to four actions, the sequence runner plays the action on
//! top of the stack (a script of steps from the AI data, or one of the coded actions: move 7, rush 0x19, follow-up
//! 0x1A, ki blast 0x36) into a virtual pad, and the pad is handed to the fighter as its injected input.
//!
//! Every random number is a `Rand_Range` of the shared generator, in the original order.
//!
//! Not ported here because the fighter side has nothing to report yet: techniques, skills and ki blasts (no
//! technique, blast or prompt is ever active), transformation, fusion and team switches (never possible), clashes
//! and move-slot effects (never running), the stage 4 / 27 pose bit and the story-mode override.

use bt3_data::ai_data::{AiData, AiRule, AiTables};
use bt3_data::stage_col::StageNav;

use crate::btl_chr::{
    Chr, Env, SideData, add, dot, is_blocked_by_opponent, length, normalize, scale3, spacing, sub,
};
use crate::rng::GameRandom;

type V3 = [f32; 3];

// AI button bits (`BtlAiPad_Set`).
const B_GUARD: u32 = 0x1;
const B_DASH: u32 = 0x2;
const B_BLAST: u32 = 0x4;
const B_RUSH: u32 = 0x8;
const B_UP: u32 = 0x10;
const B_DOWN: u32 = 0x20;
const B_LEFT: u32 = 0x40;
const B_RIGHT: u32 = 0x80;
const B_LOCKON: u32 = 0x100;
const B_CHARGE: u32 = 0x200;
const B_ASCEND: u32 = 0x800;
const B_DESCEND: u32 = 0x1000;
const B_R3: u32 = 0x2000;

// BtlAiSeq.flags.
const ACT_PATH: i32 = 0x02;
const ACT_BLOCKED_AHEAD: i32 = 0x04;
const ACT_BLOCKED: i32 = 0x08;
const ACT_BLOCKED_ID: i32 = 0x10;
const SEQ_TOOK_EFFECT: i32 = 0x40;
const SEQ_CHAIN: i32 = 0x80;
const SEQ_COMBO_SECOND: i32 = 0x100;
const SEQ_FULL_CHARGE: i32 = 0x200;
const SEQ_NO_HIT_END: i32 = 0x400;
const SEQ_RELEASE: i32 = 0x800;
const SEQ_FIRED: i32 = 0x1000;

// Action flags of the AI data.
const SEQF_END_WHEN_HIT: u16 = 0x01;
const SEQF_WAIT_IDLE: u16 = 0x02;
const SEQF_END_IN_CLASH: u16 = 0x08;
const SEQF_END_OPP_SKILL: u16 = 0x10;

// BtlAi.sight.
const SIGHT_BLOCKED: i32 = 1;
const SIGHT_BLOCKED_ID: i32 = 2;

// AiActAtk.flags.
const ATK_WAIT_CLS11: i32 = 1;
const ATK_IDLE: i32 = 2;
const ATK_CHAIN: i32 = 4;

/// `sAiActChargeRows`: base for parameter bit 0 clear / set, random range.
const CHARGE_ROWS: [[i8; 3]; 8] = [
    [0, 0, 12],
    [10, 3, 12],
    [10, 3, 12],
    [0, 0, 0],
    [10, 3, 12],
    [0, 0, 12],
    [10, 3, 12],
    [0, 0, 0],
];

#[derive(Clone, Copy, Debug, Default)]
struct Entry {
    id: u32,
    arg: i8,
}

/// `BtlAiSeq` (work + 0x28).
#[derive(Clone, Debug, Default)]
struct Seq {
    flags: i32,
    depth: i32,
    phase: i32,
    group: i32,
    step: i32,
    stack: [Entry; 8],
    timer: i32,
    wait_timer: i32,
    start_state: i32,
    charge: f32,
    start_dist: f32,
    rule_set: i32,
    fired_set: i32,
    skip: i32,
    dir_x: f32,
    dir_z: f32,
    prev_id: i32,
    fired_rule: i32,
    roll: i32,
    threshold: i32,
    fired_group: i32,
}

impl Seq {
    /// `BtlAiSeq_Reset`.
    fn reset(&mut self) {
        self.flags = 0;
        self.depth = 0;
        self.phase = 0;
        self.group = 0;
        self.step = 0;
        self.stack = [Entry { id: 1, arg: 0 }; 8];
        self.skip = 0;
    }

    /// `SEQ_TOP`: the running entry.
    fn top(&self) -> Entry {
        self.stack[(self.depth - 1).clamp(0, 7) as usize]
    }
}

/// `BtlAiMoveWork` (work + 0xC0).
#[derive(Clone, Debug, Default)]
struct Move {
    dist: [f32; 5],
    /// Goal first; the last point is the next one to reach.
    path: Vec<[f32; 4]>,
    path_timer: i32,
    kind: i32,
    target: V3,
    remain: f32,
    mode: i32,
    dir: i32,
    flags: i32,
    last_pos: V3,
    still_timer: i32,
    stuck_timer: i32,
}

/// `AiActAtk` (work + 0x240).
#[derive(Clone, Debug, Default)]
struct Atk {
    hold: u32,
    press: u32,
    charge: f32,
    state: i32,
    flags: i32,
    wait_timer: i32,
    idle_timer: i32,
    opp_kind: i32,
    step: i32,
    choice: i32,
}

/// `BtlAiOutput` (work + 0x268): the virtual pad.
#[derive(Clone, Debug, Default)]
struct Pad {
    buttons: u32,
    stick: [f32; 2],
    mark: [i32; 14],
    flip: i32,
    acc: [f32; 2],
    timer: i32,
}

/// `BtlAiStatus` (work + 0x2C0).
#[derive(Clone, Debug, Default)]
struct Status {
    flags: u64,
    hit_pending_timer: i32,
    opp_class: i32,
    opp_action: i32,
    react: i32,
    timer18: i32,
    timer1c: i32,
    timer20: i32,
    act10_dist: f32,
}

/// `BtlAiPlan` (work + 0x2E8).
#[derive(Clone, Debug, Default)]
struct Plan {
    cls: i32,
    next: i32,
    slot: i32,
    cond: i32,
    cond_no: i32,
    /// Pre-drawn roll and running sum per condition position.
    rolls: [(i32, i32); 8],
    sub: i32,
    rate: [i8; 14],
    /// `total[4][14]` as one block (a class of 14 reads the next row).
    total: [[i8; 14]; 4],
    cooldown: i32,
    rule_no: u8,
    last_roll: u8,
    next_group: u8,
    range: u8,
}

/// `BtlAiWork`: one side's controller.
#[derive(Clone, Debug, Default)]
pub(crate) struct Work {
    side: usize,
    kind: usize,
    level: i32,
    kit: i32,
    cost_range: [i32; 2],
    /// The character has AI parameters (`param != NULL`).
    active: bool,
    seq: Seq,
    mv: Move,
    atk: Atk,
    pad: Pad,
    st: Status,
    plan: Plan,
}

/// What the AI reads: both fighters, their data, the battle and the AI tables.
pub(crate) struct Ctx<'a, 'd> {
    pub chars: &'a mut [Chr; 2],
    pub data: [&'a SideData<'d>; 2],
    pub env: &'a Env<'a>,
    pub ai: &'a AiData,
    pub tab: &'a AiTables,
    pub nav: &'a StageNav,
    pub rng: &'a mut GameRandom,
}

/// `gBtlAi`: the shared values and both sides.
#[derive(Clone, Debug, Default)]
pub(crate) struct AiMgr {
    work: [Work; 2],
    dist: f32,
    radius_sum: f32,
    sight: i32,
    frame: i32,
}

/// The values of the manager a side reads while it runs.
#[derive(Clone, Copy)]
struct Shared {
    dist: f32,
    radius_sum: f32,
    sight: i32,
}

/// `BtlAi_ScaleByLevel`: lo at level 0, hi at level 29.
fn scale_by_level(level: i32, lo: i32, hi: i32) -> i32 {
    let level = level.max(0);
    lo + ((hi - lo) as f32 / 29.0 * level as f32 + 0.01) as i32
}

/// `LEVEL_IDX`: the five-step level bands.
fn level_idx(level: i32) -> usize {
    if level < 0 {
        0
    } else {
        (level / 6).min(4) as usize
    }
}

/// `Vec3_RotateY`.
fn rotate_y(v: V3, angle: f32) -> V3 {
    let (sn, cs) = angle.sin_cos();
    [v[0] * cs + v[2] * sn, v[1], v[2] * cs - v[0] * sn]
}

fn facing(rot: V3) -> V3 {
    normalize([rot[1].sin(), 0.0, rot[1].cos()])
}

impl AiMgr {
    /// `BtlAiMgr_Init` / `BtlAiMgr_Reset`: both sides from their fighters. `level` / `kind` are the active members'
    /// CPU level and AI type.
    pub(crate) fn new(c: &mut Ctx, level: [i32; 2], kind: [usize; 2]) -> Self {
        let mut mgr = Self::default();
        for side in 0..2 {
            mgr.reset_side(c, side, level[side], kind[side]);
        }
        mgr
    }

    /// `AiThink_ResetSide`.
    fn reset_side(&mut self, c: &mut Ctx, side: usize, level: i32, kind: usize) {
        self.work[side] = Work::default();
        if c.data[side].tables.ai_param.is_none() {
            return;
        }
        let w = &mut self.work[side];
        w.active = true;
        w.side = side;
        w.level = level;
        w.kind = kind;
        let shared = Shared {
            dist: self.dist,
            radius_sum: self.radius_sum,
            sight: self.sight,
        };
        {
            let mut ai = Ai {
                w: &mut *w,
                c: &mut *c,
                g: shared,
                opp_kind: 0,
            };
            // BtlAiMgr_SetLevel and BtlAiMgr_SetType: the rates and totals of the final type and level.
            ai.build_rates();
            ai.build_totals();
            ai.w.plan.cooldown = 0;
            ai.w.seq.reset();
        }
        self.radius_sum = c.data[side].radius() + c.data[side ^ 1].radius();
        let w = &mut self.work[side];
        w.mv.dist[0] = self.radius_sum;
        let d = spacing(&c.chars[side], c.data[side], c.data[side ^ 1], 0) - c.data[side].radius();
        w.mv.dist[1] = d;
        w.mv.dist[2] = d * 5.0;
        w.mv.dist[3] = d * 10.0;
        w.mv.dist[4] = d * 10.0;
        // AiThink_ReadKit.
        let moves = &c.data[side].tables.skill_table;
        let skills = &c.data[side].tables.super_table;
        w.kit = 0;
        for i in 0..2 {
            let k = byte(moves, 0x9E + i) as i8;
            if k == 6 {
                w.kit |= 1;
            }
            if k == 7 {
                w.kit |= 2;
            }
        }
        if w.kit & 3 == 3 {
            w.kit &= !2;
        }
        if c.data[side].tables.params.chara_flags() >> 7 & 1 != 0 {
            w.kit |= 4;
        }
        let (c0, c1) = (word(skills, 0x19C) as i32, word(skills, 0x1A0) as i32);
        w.cost_range = [c0.min(c1) / 1000, if c0 < c1 { c1 } else { c0 } / 1000];
    }

    /// `BtlAiMgr_Update`, between the battle's work update and the fighters' input sampling: nothing outside
    /// sequence states 2 (Ready) and 3 (Fight).
    pub(crate) fn update(&mut self, c: &mut Ctx) {
        if !matches!(c.env.seq_state, 2 | 3) {
            return;
        }
        // BtlAiMgr_UpdateSight.
        let mut a = c.chars[0].pose.pos;
        a = add(a, c.chars[0].pose.disp);
        a[1] -= c.data[0].height() * 0.5;
        let mut b = add(c.chars[1].pose.pos, c.chars[1].pose.disp);
        b[1] -= c.data[1].height() * 0.5;
        self.dist = length(sub(a, b));
        self.sight = 0;
        if let Some(hit) = c.env.col.trace_segment(a, b) {
            self.sight |= SIGHT_BLOCKED;
            if hit.obj != -1 {
                self.sight |= SIGHT_BLOCKED_ID;
            }
        }
        self.frame += 1;
        for i in 0..2 {
            if !(c.chars[i].input.inject_on && self.work[i].active) {
                continue;
            }
            let shared = Shared {
                dist: self.dist,
                radius_sum: self.radius_sum,
                sight: self.sight,
            };
            let opp_kind = self.work[i ^ 1].atk.opp_kind;
            let mut ai = Ai {
                w: &mut self.work[i],
                c: &mut *c,
                g: shared,
                opp_kind,
            };
            ai.sense();
            ai.think();
            ai.run_seq();
            // BtlAi_SendInput.
            let (buttons, stick) = (ai.w.pad.buttons, ai.w.pad.stick);
            let input = &mut ai.c.chars[i].input;
            input.inject_buttons = buttons;
            input.inject_stick = stick;
        }
    }
}

fn byte(b: &[u8], o: usize) -> u8 {
    b.get(o).copied().unwrap_or(0)
}

fn word(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4)
        .map_or(0, |w| u32::from_le_bytes(w.try_into().unwrap()))
}

fn half(b: &[u8], o: usize) -> i16 {
    b.get(o..o + 2)
        .map_or(0, |w| i16::from_le_bytes([w[0], w[1]]))
}

/// One side while it runs.
struct Ai<'w, 'c, 'a, 'd> {
    w: &'w mut Work,
    c: &'c mut Ctx<'a, 'd>,
    g: Shared,
    /// `atk.oppKind` of the other side.
    opp_kind: i32,
}

impl Ai<'_, '_, '_, '_> {
    // ---- fighter queries (BtlCharApi_*) ----

    fn me(&self) -> usize {
        self.w.side
    }

    fn op(&self) -> usize {
        self.w.side ^ 1
    }

    fn chr(&self, i: usize) -> &Chr {
        &self.c.chars[i]
    }

    fn rand(&mut self, n: u32) -> i32 {
        self.c.rng.range(n) as i32
    }

    fn pos(&self, i: usize) -> V3 {
        add(self.chr(i).pose.pos, self.chr(i).pose.disp)
    }

    fn rot(&self, i: usize) -> V3 {
        self.chr(i).pose.rot
    }

    fn anim(&self, i: usize) -> i32 {
        self.chr(i).anim.cur
    }

    fn action(&self, i: usize) -> i32 {
        self.chr(i).act.current
    }

    fn cls(&self, i: usize) -> i8 {
        self.c.ai.class_of(self.anim(i))
    }

    fn state_flags(&self, i: usize) -> u8 {
        self.c.ai.flags_of(self.anim(i))
    }

    fn flag(&self, i: usize, n: u32) -> bool {
        self.chr(i).flags.test(n)
    }

    fn height(&self, i: usize) -> f32 {
        self.c.data[i].height()
    }

    fn radius(&self, i: usize) -> f32 {
        self.c.data[i].radius()
    }

    fn speed(&self, i: usize) -> f32 {
        self.chr(i).pose.speed
    }

    fn ground_y(&self, i: usize) -> f32 {
        self.chr(i).pose.ground_pos[1]
    }

    fn ki(&self, i: usize) -> i32 {
        self.chr(i).gauge.ki
    }

    fn blast(&self, i: usize) -> i32 {
        self.chr(i).gauge.blast
    }

    fn hp(&self, i: usize) -> i32 {
        self.chr(i).gauge.health
    }

    fn hp_max(&self, i: usize) -> i32 {
        self.chr(i).gauge.health_max
    }

    /// `BtlSide_IsPoweredUp`: fighter flag 6.
    fn powered(&self, i: usize) -> bool {
        self.flag(i, 6)
    }

    fn act_bits(&self, i: usize) -> u64 {
        self.chr(i).act_bits
    }

    fn param(&self) -> &[u8] {
        self.c.data[self.me()]
            .tables
            .ai_param
            .as_deref()
            .unwrap_or(&[])
    }

    fn pb(&self, o: usize) -> i32 {
        i32::from(byte(self.param(), o) as i8)
    }

    fn profile(&self) -> &[u8] {
        self.c
            .ai
            .profiles
            .get(self.w.kind)
            .and_then(Option::as_deref)
            .unwrap_or(&[])
    }

    /// Profile byte, unsigned / signed.
    fn pu(&self, o: usize) -> i32 {
        i32::from(byte(self.profile(), o))
    }

    fn ps(&self, o: usize) -> i32 {
        i32::from(byte(self.profile(), o) as i8)
    }

    fn scale(&self, lo: i32, hi: i32) -> i32 {
        scale_by_level(self.w.level, lo, hi)
    }

    /// `BtlCharApi_GetChargeRate`.
    fn charge_rate(&self, i: usize) -> f32 {
        let c = self.chr(i);
        0f32.max(c.charge).max(c.blast_charge as f32)
    }

    /// `BtlCharApi_IsAttackHitPending`.
    fn attack_hit_pending(&self, i: usize) -> bool {
        let hit = &self.chr(i).obj.hit;
        if hit.index != !hit.count {
            return true;
        }
        if self.flag(i, 0x60) {
            return false;
        }
        hit.index >= 0
    }

    /// `BtlCharApi_IsAttackHitsDone`.
    fn attack_hits_done(&self, i: usize) -> bool {
        let hit = &self.chr(i).obj.hit;
        hit.index == !hit.count
    }

    /// `BtlCharApi_IsInClashA`.
    fn in_clash_a(&self, i: usize) -> bool {
        matches!(self.action(i), 0x130..=0x132)
    }

    /// `BtlCharApi_AnyHasFlag128`.
    fn any_flag_128(&self) -> bool {
        self.flag(0, 0x128) || self.flag(1, 0x128)
    }

    /// `BtlCharApi_IsAnimFlag2800`.
    fn anim_flag_2800(&self, i: usize) -> bool {
        let f = self.c.env.anim_flags(self.anim(i));
        f & 0x2800 != 0 && f & 0x8000 == 0
    }

    /// `BtlCharApi_GetOppTechniqueKind`: no technique action is ported, so the opponent is never in one.
    fn opp_technique_kind(&self) -> i32 {
        -1
    }

    /// `BtlCharApi_GetOppSkillKind`: the kind of the opponent's move action 0xFD..0x102 (none is ported).
    fn opp_skill_kind(&self) -> i32 {
        -1
    }

    /// `BtlCharApi_FindIncomingBlast`: no ki blast is ever alive.
    fn incoming_blast(&self, _mode: i32) -> i32 {
        -1
    }

    /// `BtlCharApi_GetTechniqueFramesLeft` of a fighter: frames to the next event with mask bit 0.
    fn technique_frames_left(&self, i: usize) -> f32 {
        let obj = &self.chr(i).obj;
        let at = obj.query_event(&self.c.data[i].obj, 1, 0, 1);
        if at < 0 {
            return -1.0;
        }
        if 0.01 < obj.step {
            (at as f32 - obj.frame) / obj.step
        } else {
            -1.0
        }
    }

    /// `BtlCharApi_GetAttackAttr`: the flag word of the attack in use.
    fn attack_attr(&self, i: usize) -> u32 {
        let id = crate::btl_hit::attack_id(self.chr(i), self.c.env);
        usize::try_from(id)
            .ok()
            .filter(|&id| id < bt3_data::chara_data::ATTACK_COUNT)
            .and_then(|id| self.c.data[i].tables.attacks.get(id))
            .map_or(0, |r| r.flags())
    }

    /// `BtlParam_GetBlastLimitA` (no abilities are ported).
    fn blast_limit_a(&self, i: usize) -> i32 {
        i32::from(self.c.data[i].tables.params.u16_at(0x80) as i16)
    }

    /// `BtlCharApi_GetBlastRoom` / `B` with no blast alive.
    fn blast_room(&self, i: usize, b: bool) -> i32 {
        let left = self.blast_limit_a(i) - self.chr(i).blast_shots;
        let room = if b {
            i32::from(self.c.data[i].tables.params.u16_at(0x82) as i16)
        } else {
            self.blast_limit_a(i)
        };
        left.min(room)
    }

    /// `BtlCharApi_GetArmorBreakLevel` without skills or abilities.
    fn armor_break_level(&self, i: usize) -> i32 {
        let (me, opp) = (&self.c.data[i].tables, &self.c.data[i ^ 1].tables);
        let mut level = 0;
        if opp.params.flags() & 4 != 0 {
            level = 1;
        }
        if opp.params.flags() & 8 != 0 {
            level -= 1;
        }
        if me.params.flags() & 4 != 0 {
            level -= 1;
        }
        if me.params.flags() & 8 != 0 {
            level += 1;
        }
        level += i32::from(self.chr(i ^ 1).skill_timer_c > 0);
        for k in 0..5 {
            let ignore = me.attacks.get(k).map_or(0, |r| i32::from(r.s8_at(0x2C)));
            if level - ignore <= 0 {
                return if k < 2 { 0 } else { k as i32 - 1 };
            }
        }
        4
    }

    /// `BtlCharApi_GetVanishStrikesLeft`.
    fn vanish_left(&self, i: usize) -> i32 {
        let c = self.chr(i);
        if c.vanish_limit == 0 {
            -1
        } else {
            c.vanish_limit - c.vanish_count
        }
    }

    /// The move table (`BtlCharApi_GetMoveTable`, skill section): kind, id and stock of slot `n`.
    fn move_kind(&self, i: usize, n: usize) -> i32 {
        i32::from(byte(&self.c.data[i].tables.skill_table, 0x9E + n) as i8)
    }

    fn move_id(&self, i: usize, n: usize) -> i32 {
        i32::from(half(&self.c.data[i].tables.skill_table, 0x10 + n * 2))
    }

    fn move_stock(&self, i: usize, n: usize) -> i32 {
        i32::from(byte(&self.c.data[i].tables.skill_table, 0x94 + n) as i8)
    }

    /// `BtlCharApi_IsMoveFlag100`.
    fn move_flag_100(&self, i: usize, n: usize) -> bool {
        word(&self.c.data[i].tables.skill_table, 0x8 + n * 4) >> 8 & 1 != 0
    }

    /// The technique table (`BtlCharApi_GetSkillTable`, technique section).
    fn skill_tbl(&self, i: usize) -> &[u8] {
        &self.c.data[i].tables.super_table
    }

    fn skill_kind(&self, i: usize, n: usize) -> i32 {
        i32::from(byte(self.skill_tbl(i), 0x165 + n) as i8)
    }

    fn skill_type(&self, i: usize, n: usize) -> i32 {
        i32::from(byte(self.skill_tbl(i), 0x13E + n) as i8)
    }

    fn skill_cost(&self, i: usize, n: usize) -> i32 {
        word(self.skill_tbl(i), 0x19C + n * 4) as i32
    }

    fn skill_flags(&self, i: usize, n: usize) -> u32 {
        word(self.skill_tbl(i), 0xC + n * 4)
    }

    fn skill_state(&self, i: usize, n: usize) -> u32 {
        word(self.skill_tbl(i), n * 4)
    }

    fn skill_id(&self, i: usize, n: usize) -> i32 {
        i32::from(half(self.skill_tbl(i), 0x18 + n * 2))
    }

    fn skill_power(&self, i: usize, n: usize) -> i32 {
        i32::from(half(self.skill_tbl(i), 0x1E + n * 2))
    }

    fn skill_rank(&self, i: usize, n: usize) -> i32 {
        i32::from(byte(self.skill_tbl(i), 0x93 + n) as i8)
    }

    // ---- the virtual pad (btl_ai_sense.c) ----

    /// `BtlAiPad_Clear`.
    fn pad_clear(&mut self, keep: bool) {
        let p = &mut self.w.pad;
        if !keep {
            p.mark = [0; 14];
            p.flip = 0;
            p.timer = 0;
        }
        p.buttons = 0;
        p.stick = [0.0; 2];
        p.acc = [0.0; 2];
    }

    /// `BtlAiPad_AddButtons`: AI button bits to battle button bits.
    fn pad_add(&mut self, bits: u32) {
        let p = &mut self.w.pad;
        p.buttons |= bits
            & (B_GUARD | B_DASH | B_BLAST | B_RUSH | 0xF0 | B_CHARGE | B_ASCEND | B_DESCEND | B_R3);
        if bits & B_LOCKON != 0 {
            p.buttons |= 0x4000;
        }
    }

    /// `BtlAiPad_Set`.
    fn pad_set(&mut self, hold: u32, press: u32, once: u32, special: i16, x: f32, y: f32) {
        for i in 0..14 {
            if self.w.pad.mark[i] & 2 == 0 {
                let bits = once & (1 << i);
                if bits != 0 {
                    self.pad_add(bits);
                    self.w.pad.mark[i] |= 2;
                }
            }
            if self.w.pad.mark[i] & 1 == 0 {
                self.pad_add(press & (1 << i));
            }
            self.pad_add(hold & (1 << i));
        }
        let p = &mut self.w.pad;
        if hold & 0x600 == 0x600 && press & B_R3 != 0 {
            p.buttons = 0x8000;
        }
        match special {
            1 => p.buttons = if p.flip & 1 == 0 { 0x1_0000 } else { 0x4_0000 },
            2 => p.buttons = if p.flip & 1 == 0 { 0x2_0000 } else { 0x4_0000 },
            3 => {
                if p.mark[3] & 4 == 0 {
                    p.timer = 2;
                    p.mark[3] |= 4;
                    p.buttons &= !8;
                }
            }
            _ => {}
        }
        p.stick = [(p.acc[0] + x) * 0.5, (p.acc[1] + y) * 0.5];
        p.acc = p.stick;
        self.track_still();
    }

    /// `BtlAiPad_TrackStill`.
    fn track_still(&mut self) {
        if self.w.pad.buttons & 0x1802 != 0 {
            let pos = self.pos(self.me());
            let len = length(sub(pos, self.w.mv.last_pos));
            if len < self.radius(self.me()) {
                self.w.mv.still_timer += 1;
                if self.w.mv.still_timer > 30 {
                    self.w.mv.flags |= 1;
                }
            }
            self.w.mv.last_pos = pos;
        } else {
            self.w.mv.still_timer = 0;
            self.w.mv.flags &= !1;
        }
    }

    /// `BtlAiPad_EndFrame`.
    fn pad_end_frame(&mut self) {
        let p = &mut self.w.pad;
        if p.timer > 0 {
            p.timer -= 1;
        }
        for m in &mut p.mark {
            *m ^= 1;
        }
        p.flip ^= 1;
    }

    // ---- the sequence runner (btl_ai_seq.c) ----

    /// `BtlAi_RunSeq`.
    fn run_seq(&mut self) {
        self.pad_clear(true);
        if self.w.seq.depth > 0 {
            match self.w.seq.top().id {
                3 => {
                    // BtlAi_RunInstant.
                    match self.w.seq.top().arg {
                        0 => {}
                        1 => self.w.seq.flags |= SEQ_CHAIN,
                        2 => self.note_opponent(0x20000),
                        _ => {}
                    }
                    self.w.seq.depth -= 1;
                }
                7 => self.move_dispatch(),
                0x19 => self.combo_dispatch(),
                0x1A => self.follow_dispatch(),
                0x36 => self.act36_dispatch(),
                _ => match self.w.seq.phase {
                    0 => self.phase_init(),
                    1 => self.phase_start(),
                    2 => self.phase_run(),
                    _ => self.phase_end(),
                },
            }
            self.pad_end_frame();
        }
    }

    fn act_index(&self, id: u32) -> usize {
        (id as usize).min(0x7F)
    }

    /// `BtlAiSeq_IsInterrupted`.
    fn is_interrupted(&self, id: u32) -> bool {
        let flags = self.c.ai.flags[self.act_index(id)];
        let cls = self.cls(self.me());
        if flags & SEQF_END_WHEN_HIT != 0
            && self.w.seq.flags & SEQ_NO_HIT_END == 0
            && (cls as u32).wrapping_sub(1) < 7
        {
            return true;
        }
        if flags & SEQF_END_IN_CLASH != 0 && (cls == 0x20 || self.in_clash_a(self.me())) {
            return true;
        }
        flags & SEQF_END_OPP_SKILL != 0 && self.opp_technique_kind() != -1
    }

    /// `BtlAiSeq_CheckHeight` (mode 0: near the ceiling; 1: near the ground).
    fn check_height(&self, mode: i32) -> bool {
        let me = self.me();
        let alt = self.ground_y(me) - self.pos(me)[1];
        let limit = self.c.env.stage.bounds.top + self.height(me) + 10.0;
        match mode {
            0 => self.pos(me)[1] < limit,
            1 => self.c.env.stage_4_or_27 || alt < 10.0,
            _ => false,
        }
    }

    /// `BtlAiSeq_CheckStageAbort`. The stage 4 / 27 tests read the pose bit, which is never set here.
    fn check_stage_abort(&mut self, id: u32) -> bool {
        let special = self.c.env.stage_4_or_27;
        match id {
            9 | 11 | 21 | 13 | 29 => special,
            15 => self.check_height(self.w.seq.step),
            16 => self.check_height(0),
            17 => self.check_height(1),
            _ => false,
        }
    }

    fn steps(&self, id: u32) -> (usize, usize) {
        let i = self.act_index(id);
        (self.c.ai.first[i] as usize, self.c.ai.count[i] as usize)
    }

    fn step_at(&self, n: usize) -> bt3_data::ai_data::AiStep {
        self.c.ai.steps.get(n).copied().unwrap_or_default()
    }

    /// Phase 0.
    fn phase_init(&mut self) {
        let top = self.w.seq.top();
        let flags = self.c.ai.flags[self.act_index(top.id)];
        let cls = self.cls(self.me());
        self.w.seq.group = 0;
        self.pad_clear(false);
        if self.is_interrupted(top.id) {
            self.w.seq.phase = 3;
            return;
        }
        if flags & SEQF_WAIT_IDLE != 0 && cls != 0 {
            if cls == 10 {
                self.pad_clear(true);
                self.pad_set(0, B_DASH, 0, 0, 0.0, 0.0);
            }
            return;
        }
        self.w.seq.phase = 1;
        self.phase_start();
    }

    /// Phase 1.
    fn phase_start(&mut self) {
        let top = self.w.seq.top();
        let (first, count) = self.steps(top.id);
        let pick = self.c.ai.pick[self.act_index(top.id)];
        self.pad_clear(false);
        let me = self.me();
        let s = &mut self.w.seq;
        s.timer = 0;
        s.wait_timer = 0;
        s.flags &= !0x1E40;
        s.start_state = self.c.chars[me].anim.cur;
        s.charge = 0f32
            .max(self.c.chars[me].charge)
            .max(self.c.chars[me].blast_charge as f32);
        s.start_dist = self.g.dist;
        s.dir_x = 0.0;
        s.dir_z = 0.0;
        let group = s.group;
        let n = (0..count)
            .filter(|&i| i32::from(self.step_at(first + i).group) == group)
            .count() as i32;
        if n == 0 {
            self.w.seq.phase = 3;
            return;
        }
        let sel = self.pick(pick, n, i32::from(top.arg));
        if sel < 0 {
            self.w.seq.phase = 3;
            return;
        }
        let mut k = 0;
        for i in 0..count {
            if i32::from(self.step_at(first + i).group) == group {
                if sel == k {
                    self.w.seq.step = i as i32;
                    break;
                }
                k += 1;
            }
        }
        self.w.seq.phase = 2;
        if top.id == 10 {
            if self.w.seq.step == 0 {
                self.w.st.act10_dist = self.g.dist;
            }
        } else {
            self.w.st.act10_dist = 0.0;
        }
        self.phase_run();
    }

    fn cur_step(&self) -> bt3_data::ai_data::AiStep {
        let (first, _) = self.steps(self.w.seq.top().id);
        self.step_at(first + self.w.seq.step.max(0) as usize)
    }

    /// Phase 2.
    fn phase_run(&mut self) {
        let top = self.w.seq.top();
        let timeout = i32::from(self.c.ai.timeout[self.act_index(top.id)]);
        let me = self.me();
        let step = self.cur_step();
        if self.w.seq.flags & SEQ_TOOK_EFFECT == 0
            && i32::from(self.cls(me)) == i32::from(step.cls)
            && self.w.seq.start_state != self.anim(me)
        {
            self.w.seq.flags |= SEQ_TOOK_EFFECT;
        }
        if self.step_done(step.handler) {
            self.w.seq.phase = 1;
            self.w.seq.group += 1;
            return;
        }
        if self.is_interrupted(top.id) {
            self.w.seq.phase = 3;
            return;
        }
        if timeout > 0 && self.w.seq.flags & SEQ_TOOK_EFFECT == 0 {
            if !self.any_flag_128() {
                self.w.seq.wait_timer += 1;
            }
            if timeout * 30 < self.w.seq.wait_timer {
                self.w.seq.phase = 3;
                return;
            }
        }
        if self.check_stage_abort(top.id) {
            self.w.seq.phase = 3;
            return;
        }
        // The step chosen before the handler ran (it may pick another for the next frame).
        if self.w.seq.flags & SEQ_RELEASE != 0 {
            self.pad_set(0, 0, 0, 0, 0.0, 0.0);
        } else {
            self.pad_set(step.hold, step.press, step.once, step.special, 0.0, 0.0);
        }
    }

    /// `BtlAiSeq_RollPowerUpChain`.
    fn roll_power_up_chain(&mut self) -> bool {
        let diff = self.vanish_left(self.me());
        if !self.powered(self.me()) {
            return false;
        }
        match self.w.seq.stack[0].id {
            0x2D | 0x2E => {
                if diff != 0 {
                    return false;
                }
            }
            0x22 | 0x24 | 0x26 | 0x29 | 0x2A | 0x2B | 0x32 | 0x35 => {}
            _ => return false,
        }
        self.roll_act_rate(0x4C)
    }

    /// Phase 3.
    fn phase_end(&mut self) {
        let cls = self.cls(self.me());
        if cls == 10 && self.w.seq.top().id != 14 {
            self.pad_clear(true);
            self.pad_set(0, B_DASH, 0, 0, 0.0, 0.0);
            return;
        }
        self.w.seq.depth -= 1;
        if self.w.seq.depth <= 0 {
            if self.roll_power_up_chain() {
                let s = &mut self.w.seq;
                s.phase = 0;
                s.prev_id = s.stack[0].id as i32;
                s.stack[0] = Entry { id: 0x43, arg: 0 };
                s.depth = 1;
                return;
            }
            self.w.seq.flags &= !SEQ_CHAIN;
        }
        self.w.seq.phase = 0;
    }

    // ---- pick functions ----

    fn pick(&mut self, pick: i8, n: i32, arg: i32) -> i32 {
        let sel_of = |arg: i32| if arg < 0 { -arg % 10 } else { arg % 10 };
        match pick {
            0 => self.rand(n as u32),
            1 => {
                if arg < n {
                    arg
                } else {
                    -1
                }
            }
            2 => {
                let mut sel = sel_of(arg);
                if sel == 9 {
                    sel = self.rand(n as u32);
                }
                if sel >= n {
                    return -1;
                }
                self.w.seq.timer = arg / 10 * 30;
                sel
            }
            3 => {
                let mut sel = sel_of(arg);
                let t = if arg < 0 {
                    self.rand((-arg / 10) as u32) + 1
                } else {
                    arg / 10
                };
                let roll = self.rand(100);
                self.w.seq.timer = t;
                if n != 5 {
                    return 0;
                }
                if sel == 8 {
                    self.w.seq.timer = if roll < 30 {
                        -1
                    } else if roll < 60 {
                        10
                    } else {
                        self.rand(10)
                    };
                }
                if (sel as u32).wrapping_sub(8) < 2 || sel == 5 {
                    sel = self.rand(n as u32);
                }
                sel
            }
            4 => {
                match arg {
                    0 => self.w.seq.timer = 100,
                    1 => self.w.seq.timer = 101,
                    2 => self.w.seq.timer = self.w.cost_range[1],
                    3 => self.w.seq.timer = self.w.cost_range[0],
                    _ => {}
                }
                0
            }
            5 => {
                let roll = self.rand(100);
                if self.w.plan.slot == -1 || self.w.plan.slot == 2 {
                    return -1;
                }
                self.w.seq.timer = if roll < 30 {
                    100
                } else if roll < 60 {
                    0
                } else {
                    self.rand(100)
                };
                self.w.plan.slot
            }
            6 => {
                const TBL: [f32; 5] = [45.0, 33.75, 22.5, 11.25, 0.0];
                let r = self.rand(100) as f32 * 0.01;
                let level = self.w.level;
                let base = TBL[level_idx(level)] * 0.5;
                match arg {
                    1 => self.w.seq.timer = (base + base * r) as i32,
                    2 => {
                        self.w.seq.timer = if level < 0 {
                            0
                        } else {
                            ((29 - level) as f32 * (20.0 / 29.0) + 10.0) as i32
                        }
                    }
                    _ => {}
                }
                0
            }
            7 => self.pick_combo_branch(n),
            // Fusion / transformation slots: BtlCharApi_PickFusionSlot / PickTransformSlot draw Rand_Range(100)
            // and find no slot the side can use.
            8 | 9 => {
                self.rand(100);
                0
            }
            10 => {
                // One member: no candidate, the parameter is 0 and the alternative a draw.
                self.rand(100);
                self.w.seq.timer = 0;
                self.rand(n as u32)
            }
            11 => {
                let level = self.w.level;
                if !(level < -1) { 0 } else { -level - 2 }
            }
            // BtlAiPick_PromptButton: no prompt is ever open.
            12 => -1,
            13 => match arg {
                1 => {
                    self.w.seq.timer = 0;
                    0
                }
                2 => {
                    self.w.seq.timer = 15;
                    0
                }
                3 => {
                    self.w.seq.timer = -1;
                    0
                }
                _ => -1,
            },
            14 => self.pick_direction(n, arg),
            15 => {
                self.w.seq.timer = arg;
                0
            }
            _ => 0,
        }
    }

    /// Pick 7.
    fn pick_combo_branch(&mut self, n: i32) -> i32 {
        let kind = self.blast_step();
        let roll = self.rand(100);
        if n == 1 {
            self.w.seq.flags &= !SEQ_COMBO_SECOND;
            return 0;
        }
        let mut sum = 0;
        let mut col = (kind * 3 + 0x10) as usize;
        for i in 0..3 {
            sum += self.scale(self.pu(0x15C + col), self.pu(0x41C + col));
            col += 1;
            if kind == 1 && self.w.seq.flags & SEQ_COMBO_SECOND != 0 && i == 1 {
                return 0;
            }
            if roll < sum {
                if kind == 0 && i == 1 {
                    self.w.seq.flags |= SEQ_COMBO_SECOND;
                }
                if i == 2 && self.w.st.flags & 0x2000_0000_0000_0000 != 0 {
                    return 0;
                }
                return i;
            }
        }
        0
    }

    /// Pick 14.
    fn pick_direction(&mut self, n: i32, arg: i32) -> i32 {
        let mut sel = if arg < 0 { -arg % 10 } else { arg % 10 };
        if n == 1 {
            return 0;
        }
        if n == 8 || n == 4 {
            let mut best = -2;
            let radius = self.c.env.inner_radius();
            let mut range = n * 10;
            let pos = self.pos(self.me());
            let mut far = 0.0;
            if radius * 0.5 < length(pos) || self.c.env.stage_4_or_27 {
                let mut dir = facing(self.rot(self.me()));
                for i in 0..n {
                    let probe = add(scale3(dir, 15.0), pos);
                    let len = length(probe);
                    if far < len {
                        far = len;
                        best = i;
                    }
                    dir = rotate_y(dir, 0.7853981);
                }
                range = if n == 8 { 0x3B } else { 0x21 };
            }
            let roll = self.rand(range as u32);
            let mut sum = 0;
            for i in 0..n {
                let low = if n == 8 {
                    best != -2 && (best == i || (best + 1) % n == i || (best + 7) % n == i)
                } else {
                    best == i
                };
                sum += if low { 3 } else { 10 };
                if roll < sum {
                    sel = i;
                    break;
                }
            }
        }
        if sel >= n {
            return -1;
        }
        self.w.seq.timer = arg / 10 * 30;
        sel
    }

    // ---- step handlers ----

    fn step_done(&mut self, handler: i8) -> bool {
        match handler {
            0 => self.step_reach_class(),
            1 => self.step_charge_ki(),
            2 => self.step_until_blocked(),
            3 => {
                if self.any_flag_128() {
                    return false;
                }
                self.w.seq.timer -= 1;
                self.w.seq.timer < 1
            }
            4 => {
                let blocked = self.step_until_blocked();
                let charged = self.step_charged();
                blocked || charged
            }
            5 => {
                let blocked = self.step_until_blocked();
                let charged = self.step_charged();
                blocked && charged
            }
            6 => self.step_not_hit(),
            7 => self.step_charged(),
            8 => self.step_guard_until_safe(),
            9 => self.step_guard_countdown(),
            10 => {
                let done = self.step_reach_class();
                done && self.chr(self.me()).blast_shots == 0
            }
            11 => {
                if self.w.seq.timer == -1 {
                    return true;
                }
                self.w.seq.timer = -1;
                false
            }
            12 => self.step_facing_opponent(),
            // BtlAiStep_SwitchQueued: one member, the switch target is 0.
            13 => self.w.seq.timer == 0,
            14 => self.step_clash_mash(),
            15 => self.step_reach_class_move_slot(),
            16 => {
                self.w.seq.timer += 1;
                if self.w.seq.timer > 60 {
                    return true;
                }
                !(self.w.mv.dist[1] + self.w.mv.dist[1] < self.g.dist)
            }
            17 => self.step_fire_skill(),
            18 | 19 | 21 => {
                const LOW: [i32; 5] = [15, 7, 5, 3, 3];
                const HIGH: [i32; 5] = [15, 10, 7, 5, 3];
                let hp = self.hp(self.me());
                let t = self.w.seq.timer;
                self.w.seq.step = 1;
                self.w.seq.timer = t - 1;
                if (t as u32).wrapping_sub(2) >= 15 {
                    let i = level_idx(self.w.level);
                    self.w.seq.timer = if hp < 10000 { LOW[i] } else { HIGH[i] };
                    self.w.seq.step = 0;
                }
                match handler {
                    18 => self.step_not_hit(),
                    19 => self.chr(self.me()).stun_timer < 1,
                    _ => !self.flag(self.me(), 0xBE),
                }
            }
            // BtlAiStep_WhileOppSkillFlag4: the opponent is never in a technique.
            20 => true,
            22 => {
                self.w.seq.timer += 1;
                if self.w.seq.timer > 60 {
                    return true;
                }
                if self.w.mv.dist[1] * 3.0 < self.g.dist {
                    return false;
                }
                self.step_reach_class()
            }
            23 => self.step_unk23(),
            _ => false,
        }
    }

    /// Step handler 0.
    fn step_reach_class(&self) -> bool {
        let top = self.w.seq.top();
        let (first, _) = self.steps(top.id);
        let anim = self.anim(self.me());
        let cls = self.cls(self.me());
        let flags = self.state_flags(self.me());
        if self.step_at(first).cls == -1 {
            return true;
        }
        if self.w.seq.start_state == anim && cls != 0 {
            return false;
        }
        if top.id == 0x2E && self.w.seq.flags & SEQ_TOOK_EFFECT != 0 {
            return true;
        }
        if flags & 2 != 0 {
            return false;
        }
        i32::from(cls) == i32::from(self.step_at(first + self.w.seq.step.max(0) as usize).cls)
    }

    /// Step handler 1.
    fn step_charge_ki(&self) -> bool {
        let ki = self.ki(self.me());
        let want = (self.w.seq.timer as f32 / 100.0 * 100000.0) as i32;
        if self.behind_opponent(self.op()) {
            return true;
        }
        if self.w.seq.timer == 101 {
            self.powered(self.me())
        } else {
            !(ki < want)
        }
    }

    /// Step handler 2.
    fn step_until_blocked(&mut self) -> bool {
        let me = self.me();
        let blocked = is_blocked_by_opponent(
            &self.c.chars[me],
            &self.c.chars[me ^ 1],
            self.c.data[me],
            self.c.data[me ^ 1],
        );
        let cls = self.cls(me);
        let flag = self.act_bits(me) & 0x2000_0000;
        if cls == 0 {
            return true;
        }
        self.w.seq.start_dist = self.g.dist;
        if flag != 0 {
            return true;
        }
        blocked
    }

    /// Step handler 6.
    fn step_not_hit(&self) -> bool {
        (self.cls(self.me()) as u32).wrapping_sub(1) >= 7
    }

    /// Step handler 7.
    fn step_charged(&mut self) -> bool {
        let rate = self.charge_rate(self.me());
        let s = &mut self.w.seq;
        if s.flags & SEQ_TOOK_EFFECT != 0 && rate == 0.0 {
            return true;
        }
        if rate < s.charge {
            return true;
        }
        s.charge = rate;
        if s.timer == -1 {
            if rate == 1.0 {
                if s.flags & SEQ_FULL_CHARGE == 0 {
                    s.flags |= SEQ_FULL_CHARGE;
                } else {
                    return true;
                }
            }
            return false;
        }
        s.timer as f32 <= rate * 10.0
    }

    /// Step handler 8.
    fn step_guard_until_safe(&mut self) -> bool {
        let op = self.op();
        let cls = self.cls(op);
        let attacking = self.attack_hit_pending(op);
        let passing = false;
        let flags = self.state_flags(op);
        let kind = self.opp_technique_kind();
        let ret = (kind as u32).wrapping_sub(1) >= 2;
        if self.w.seq.top().id == 0x45 {
            self.w.seq.step = match cls {
                0x10 => 1,
                0x11 => 2,
                _ => 0,
            };
        }
        if self.w.seq.timer == -1 {
            return ret;
        }
        if self.w.seq.timer != 0 {
            return !passing;
        }
        if flags & 2 != 0 {
            return false;
        }
        !attacking
    }

    /// Step handler 9.
    fn step_guard_countdown(&mut self) -> bool {
        let op = self.op();
        if self.state_flags(op) & 2 != 0 {
            return false;
        }
        if self.attack_hit_pending(op) {
            self.w.seq.timer = 30;
        }
        if self.any_flag_128() {
            return false;
        }
        self.w.seq.timer -= 1;
        self.w.seq.timer < 0
    }

    /// Step handler 12.
    fn step_facing_opponent(&mut self) -> bool {
        let face = facing(self.rot(self.me()));
        if self.w.seq.dir_x == face[0] && self.w.seq.dir_z == face[2] {
            self.pad_clear(false);
        }
        self.w.seq.dir_z = face[2];
        self.w.seq.dir_x = face[0];
        let mut d = sub(self.pos(self.op()), self.pos(self.me()));
        d[1] = 0.0;
        0.0 < dot(face, normalize(d))
    }

    /// Step handler 14: no clash is ported, so the counts are equal and the fighter is in none.
    fn step_clash_mash(&mut self) -> bool {
        let cls = self.c.ai.class_of(self.anim(self.me()));
        let roll = self.rand(100);
        let (lo, hi) = (0x2AC, 0x56C);
        let (a, b) = match self.w.seq.timer {
            1 => (
                self.scale(self.pu(lo + 4), self.pu(hi + 4)),
                self.scale(self.pu(lo + 3), self.pu(hi + 3)),
            ),
            2 => (
                self.scale(self.pu(lo + 6), self.pu(hi + 6)),
                self.scale(self.pu(lo + 5), self.pu(hi + 5)),
            ),
            _ => return true,
        };
        let (va, vb) = (0, 0);
        let a = if va > vb { a } else { b };
        if roll < a {
            self.w.seq.step = (self.w.seq.step + 1) % 4;
        }
        match self.w.seq.timer {
            1 => cls != 0x20,
            2 => !self.in_clash_a(self.me()),
            _ => false,
        }
    }

    /// Step handler 15.
    fn step_reach_class_move_slot(&mut self) -> bool {
        let reached = self.step_reach_class();
        let step = self.w.seq.step;
        if step as u32 >= 2 {
            return true;
        }
        if reached {
            return true;
        }
        let me = self.me();
        if self.move_kind(me, step as usize) == 4 {
            self.w.seq.flags |= 0x400;
        }
        // BtlCharApi_IsMoveSlotActive: no move-slot effect runs.
        self.w.plan.cooldown = 300;
        false
    }

    /// Step handler 17.
    fn step_fire_skill(&mut self) -> bool {
        let me = self.me();
        let reached = self.step_reach_class();
        let rate = 0.0f32;
        let top = self.w.seq.top().id;
        let slot = self.w.plan.slot;
        if slot == -1 {
            return true;
        }
        let slot = slot as usize;
        let cls = [self.cls(me), self.cls(me ^ 1)];
        if !reached && self.w.seq.flags & SEQ_FIRED == 0 {
            if self.w.seq.flags & SEQ_TOOK_EFFECT != 0 {
                if self.skill_cost(me, slot) > self.ki(me) {
                    return true;
                }
                self.w.seq.flags = (self.w.seq.flags ^ SEQ_RELEASE) & !SEQ_TOOK_EFFECT;
            }
            return false;
        }
        self.w.seq.flags |= SEQ_FIRED;
        if top == 0x3F {
            return true;
        }
        self.w.seq.wait_timer = 0;
        if self.skill_kind(me, slot) == 5 && self.c.env.stage_4_or_27 {
            if cls[0] != 0x16 {
                return true;
            }
            // The pose bit is never set: step 2.
            self.w.seq.step = 2;
            return false;
        }
        let kind = self.skill_type(me, slot);
        if kind == 2 {
            if cls[0] != 0x16 {
                return true;
            }
            let opp = i32::from(cls[1]);
            if opp == kind {
                return false;
            }
            if opp != 0x1C && opp != i32::from(cls[0]) && opp != 0x17 {
                if opp == 0x19 || opp == 0x18 || (opp as u32).wrapping_sub(0xF) < 3 {
                    return true;
                }
                if ((rate * 100.0) as i32) < self.w.seq.timer {
                    return false;
                }
            }
        }
        true
    }

    /// Step handler 23.
    fn step_unk23(&mut self) -> bool {
        let reached = self.step_reach_class();
        if !reached {
            return false;
        }
        if !self.c.env.stage_4_or_27 {
            return true;
        }
        let n = match self.w.seq.prev_id {
            0x32 => 7,
            0x2A => 4,
            0x24 => 2,
            0x35 => 8,
            0x26 => 6,
            0x22 => 1,
            0x2B => 5,
            0x29 => 3,
            0x2D | 0x2E => 0,
            _ => return false,
        };
        let me = self.me();
        let slot = self.c.data[me].tables.params.u8_at(0x8F + n) as i8;
        if self.skill_kind(me, slot.max(0) as usize) != 5 {
            return true;
        }
        self.w.seq.step = 1;
        self.cls(me) == 0
    }

    // ---- the coded actions ----

    /// Action 7 (move).
    fn move_dispatch(&mut self) {
        if self.w.seq.depth > 0 && self.is_interrupted(self.w.seq.top().id) {
            self.w.seq.depth = 0;
            return;
        }
        match self.w.seq.phase {
            0 => {
                self.w.seq.phase = 1;
                self.move_start();
            }
            1 => self.move_start(),
            2 => self.move_run(),
            _ => self.move_end(),
        }
    }

    /// `BtlAiMove_PickDir`.
    fn move_pick_dir(&mut self, mode: i32) -> i32 {
        let (me, op) = (self.me(), self.op());
        let pos = self.pos(me);
        let opp = self.pos(op);
        let top = self.c.env.stage.bounds.top + 20.0;
        let rad = self.c.env.inner_radius() - 100.0;
        let mut list = vec![0];
        if mode == 2 {
            if top < pos[1] || top < opp[1] {
                list.push(1);
            }
            if pos[1] < self.ground_y(me) - 20.0 || opp[1] < self.ground_y(op) - 20.0 {
                list.push(2);
            }
        }
        let mut both = true;
        if rad < length(pos) {
            let mut d = sub(opp, pos);
            d[1] = 0.0;
            let d = normalize(d);
            let out = normalize(pos);
            if 0.0 < length(d) && 0.0 < length(out) {
                if !(0.0 < dot(out, d)) {
                    both = false;
                    if 0.0 < dot(out, normalize(rotate_y(d, -1.0471975))) {
                        list.push(3);
                    }
                    if 0.0 < dot(out, normalize(rotate_y(d, 1.0471975))) {
                        list.push(4);
                    }
                }
            } else {
                both = false;
            }
        }
        if both {
            list.push(3);
            list.push(4);
        }
        let i = self.rand(list.len() as u32) as usize;
        list[i]
    }

    /// `BtlAiMove_CalcTarget`.
    fn move_calc_target(&mut self) {
        let (me, op) = (self.me(), self.op());
        let rad = self.c.env.stage.bounds.radius;
        let mut pos = self.pos(me);
        let opp = self.pos(op);
        let a = [pos[0], pos[1], pos[2]];
        let dir = normalize(sub(a, opp));
        let kind = self.w.mv.kind;
        let off = scale3(dir, self.w.mv.dist[kind.clamp(0, 4) as usize]);
        if self.c.env.stage_4_or_27 && kind as u32 >= 2 {
            self.w.mv.target = [0.0, pos[1], 0.0];
        } else if kind == 4 {
            let place = self.c.env.stage.starting_placements.get(me);
            let mut t = place.map_or([0.0; 3], |p| p.position);
            t[1] -= 20.0;
            t[0] = (self.rand(400) - 200) as f32;
            t[2] = (self.rand(400) - 200) as f32;
            self.w.mv.target = t;
        } else {
            self.w.mv.target = add(opp, off);
        }
        pos[1] -= self.height(me) * 0.5;
        self.w.seq.flags &= !(ACT_BLOCKED | ACT_BLOCKED_ID);
        if rad < length(self.w.mv.target) {
            self.w.mv.target = add(pos, scale3(dir, rad - length(pos)));
        }
        if let Some(hit) = self.c.env.col.trace_segment(pos, self.w.mv.target) {
            self.w.seq.flags |= ACT_BLOCKED;
            if hit.obj != -1 {
                self.w.seq.flags |= ACT_BLOCKED_ID;
            }
        }
        let reach = self.radius(me) + self.speed(me) * 3.0;
        let vdir = normalize(self.chr(me).pose.vel);
        let mut ahead = add(pos, scale3(vdir, reach));
        if rad < length(ahead) {
            ahead = add(pos, scale3(vdir, rad - length(pos)));
        }
        self.w.seq.flags &= !ACT_BLOCKED_AHEAD;
        ahead[1] = pos[1];
        if self.c.env.col.trace_segment(pos, ahead).is_some() {
            self.w.seq.flags |= ACT_BLOCKED_AHEAD;
        }
    }

    /// `BtlAiMove_IsNear`.
    fn move_is_near(&self, p: V3, flat: bool) -> bool {
        let mut d = sub(self.pos(self.me()), p);
        if self.w.mv.kind != 1 || flat {
            d[1] = 0.0;
        }
        let len = length(d);
        len < 20.0 || len < self.speed(self.me()) * 5.0
    }

    /// `BtlAiMove_BuildPath`: true when the path is empty.
    fn move_build_path(&mut self) -> bool {
        let pos = self.pos(self.me());
        let t = self.w.mv.target;
        self.w.mv.path = self.c.nav.find_path(pos, [t[0], t[1], t[2], 0.0]);
        self.w.mv.path_timer = 60;
        if let Some(p) = self.w.mv.path.last() {
            self.w.mv.remain = length(sub([p[0], p[1], p[2]], pos));
            false
        } else {
            self.w.mv.remain = 0.0;
            true
        }
    }

    /// `BtlAiMove_Check`.
    fn move_check(&mut self) -> i32 {
        let anim = self.anim(self.me());
        if self.w.mv.path_timer > 0 {
            self.w.mv.path_timer -= 1;
        }
        let p3 = |p: &[f32; 4]| [p[0], p[1], p[2]];
        if self.w.seq.flags & ACT_PATH != 0 {
            if self.w.seq.flags & ACT_BLOCKED == 0 || self.w.mv.path.is_empty() {
                return 0;
            }
            let pos = self.pos(self.me());
            let n = self.w.mv.path.len();
            let skip = n >= 2
                && self
                    .c
                    .env
                    .col
                    .trace_segment(pos, p3(&self.w.mv.path[n - 2]))
                    .is_none();
            if skip || self.move_is_near(p3(&self.w.mv.path[n - 1]), true) {
                self.w.mv.path.pop();
                self.w.mv.stuck_timer = 0;
                let Some(next) = self.w.mv.path.last().copied() else {
                    return 1;
                };
                if (anim as u32).wrapping_sub(0x1F) < 4 {
                    return 4;
                }
                self.w.mv.remain = length(sub(p3(&next), pos));
            }
            if self.w.mv.path_timer == 0 {
                let opp = self.pos(self.op());
                let goal = self.w.mv.path.first().map_or([0.0; 3], p3);
                if 50.0 < length(sub(opp, goal)) && self.move_build_path() {
                    return 0;
                }
            }
            return 3;
        }
        if self.move_is_near(self.w.mv.target, false) && self.g.sight & SIGHT_BLOCKED == 0 {
            return 1;
        }
        if self.w.seq.flags & ACT_BLOCKED == 0 {
            return 2;
        }
        if self.w.seq.flags & ACT_BLOCKED_ID != 0 && self.w.mv.mode == 2 {
            return 2;
        }
        if self.move_build_path() { 0 } else { 3 }
    }

    /// `BtlAiMove_Steer`.
    fn move_steer(&mut self, to: V3, kind: i32) {
        let (me, op) = (self.me(), self.op());
        let cls = self.cls(me);
        let fly = self.flag(me, 0xF);
        let pos = self.pos(me);
        let opp = self.pos(op);
        let dy = to[1] - pos[1];
        let mut d = sub(to, pos);
        d[1] = 0.0;
        let len = length(d).max(self.radius(me));
        let d = normalize(d);
        let yaw = self.chr(me).cam.rot[1];
        let (sn, cs) = yaw.sin_cos();
        let mut x = (cs * d[0] - sn * d[2]).clamp(-1.0, 1.0);
        let mut y = (-(sn * d[0] + cs * d[2])).clamp(-1.0, 1.0);
        let (mut hold, mut press) = (0, 0);
        if self.flag(me, 0x13) || fly {
            press = B_DASH;
        } else {
            hold = B_DASH;
        }
        if kind == 0 {
            if dy < -30.0 {
                if !fly || !(pos[1] < opp[1]) {
                    hold |= B_ASCEND;
                }
            } else if 30.0 < dy {
                hold |= B_DESCEND;
            }
        }
        if self.w.seq.flags & ACT_BLOCKED_AHEAD != 0 && hold & B_DESCEND == 0 {
            hold |= B_ASCEND;
        }
        if cls == 11 {
            press &= !B_DASH;
        }
        if self.flag(me, 0xF) && !self.flag(op, 0xF) && pos[1] > opp[1] {
            hold |= B_ASCEND;
        }
        if len < self.w.mv.dist[2] && kind == 1 && (self.w.mv.kind as u32) < 2 {
            y = 0.0;
            hold |= B_UP;
            x = y;
        }
        if self.flag(me, 0x13) && hold & B_ASCEND == 0 {
            if self.w.mv.kind as u32 >= 2 {
                hold |= B_CHARGE;
            }
            if x.abs() > y.abs() {
                y = 0.0;
            } else {
                x = 0.0;
            }
        }
        self.pad_set(hold, press, 0, 0, x, y);
    }

    /// `BtlAiMove_Hold`.
    fn move_hold(&mut self) {
        let mut hold = B_DASH;
        if self.w.mv.mode == 2 {
            hold = B_DASH | B_CHARGE;
        }
        if self.w.seq.flags & ACT_BLOCKED_AHEAD != 0 {
            hold |= B_ASCEND;
        }
        let y = if self.cls(self.me()) == 10 { -1.0 } else { 0.0 };
        self.pad_set(hold, 0, 0, 0, 0.0, y);
    }

    /// `BtlAiMove_Start`.
    fn move_start(&mut self) {
        let arg = i32::from(self.w.seq.top().arg);
        let v = arg / 10;
        self.pad_clear(false);
        if v == 1 || v == 2 {
            self.w.mv.mode = v;
        }
        self.w.mv.kind = arg % 10;
        let mode = self.w.mv.mode;
        self.w.mv.dir = self.move_pick_dir(mode);
        self.w.mv.remain = self.g.dist;
        self.w.seq.flags &= !ACT_PATH;
        self.w.mv.stuck_timer = 0;
        self.move_calc_target();
        self.w.seq.phase = 2;
        self.move_run();
    }

    /// `BtlAiMove_Run`.
    fn move_run(&mut self) {
        match self.move_check() {
            0 => {
                self.move_calc_target();
                self.w.seq.phase = 1;
            }
            1 => self.w.seq.phase = 3,
            2 => {
                self.w.seq.flags &= !ACT_PATH;
                self.pad_clear(true);
                if self.w.mv.kind == 0 {
                    self.move_hold();
                } else {
                    let t = self.w.mv.target;
                    self.move_steer(t, 1);
                }
            }
            3 => {
                self.w.seq.flags |= ACT_PATH;
                if let Some(p) = self.w.mv.path.last().copied() {
                    self.move_steer([p[0], p[1], p[2]], 0);
                }
            }
            4 => self.pad_set(0, 0, B_ASCEND, 0, 0.0, 0.0),
            _ => {}
        }
        if self.w.mv.flags & 1 != 0 {
            self.w.seq.phase = 3;
            self.w.mv.flags &= !1;
        }
        self.w.mv.stuck_timer += 1;
        if self.w.mv.stuck_timer > 60 {
            self.w.seq.phase = 3;
        }
        if self.w.mv.kind == 4 && self.g.dist < self.w.mv.dist[1] {
            self.w.seq.phase = 3;
        }
    }

    /// `BtlAiMove_End`.
    fn move_end(&mut self) {
        let anim = self.anim(self.me());
        let cls = self.cls(self.me());
        self.pad_clear(true);
        if cls == 10 {
            self.pad_set(0, B_DASH, 0, 0, 0.0, 0.0);
        }
        if (anim as u32).wrapping_sub(0x1F) < 4 {
            self.pad_set(B_ASCEND, 0, 0, 0, 0.0, 0.0);
        }
        if cls == 0 {
            self.w.seq.depth -= 1;
        }
    }

    /// `BtlAiAtk_Reset`.
    fn atk_reset(&mut self) {
        let anim = self.anim(self.me());
        let a = &mut self.w.atk;
        a.hold = 0;
        a.press = 0;
        a.charge = 0.0;
        a.state = anim;
        a.flags &= !7;
        a.wait_timer = 15;
        a.idle_timer = 15;
    }

    /// `BtlAiAtk_RollCharge`.
    fn roll_charge(&mut self, base: i8, range: i8) -> f32 {
        const CHANCE: [i32; 5] = [10, 20, 30, 40, 50];
        let roll = self.rand(100);
        let kind = self.opp_kind;
        let thr = CHANCE[level_idx(self.w.level)] + if kind == 2 { 20 } else { 0 };
        if roll < thr {
            if kind == 2 {
                if self.rand(100) < thr {
                    if range > 0 { 1.0 } else { 0.0 }
                } else {
                    1.1
                }
            } else {
                f32::from(base) * 0.1
            }
        } else if range != 0 {
            self.rand(i32::from(range) as u32) as f32 * 0.1
        } else {
            0.0
        }
    }

    /// `BtlAiAtk_SetInput`.
    fn atk_set_input(&mut self, kind: u32) {
        let me = self.me();
        let action = self.action(me);
        let anim = self.anim(me);
        let bit = (byte(self.param(), 0) & 1) as usize;
        let params = &self.c.data[me].tables.params;
        let mut row = 0usize;
        if (anim as u32).wrapping_sub(0x37) < 4 || (anim as u32).wrapping_sub(0x3C) < 4 {
            let n = if anim < 0x3C {
                anim - 0x37
            } else {
                anim - 0x3C
            } as usize;
            row = params.u8_at(0x84 + if n >= 4 { 0 } else { n }) as usize;
        } else if anim == 0x1B {
            if kind == 0 {
                let o = if action == 0x1F { 0x8D } else { 0x8A };
                row = params.u8_at(o) as usize / 2;
            }
        } else {
            return;
        }
        let row = CHARGE_ROWS[row.min(7)];
        match kind {
            0 => {
                let c = self.roll_charge(row[bit], row[2]);
                self.w.atk.charge = c;
                if c != 0.0 {
                    self.w.atk.hold |= B_BLAST;
                } else {
                    self.w.atk.press |= B_BLAST;
                }
            }
            1 | 2 => {
                let t = self.roll_charge(CHARGE_ROWS[0][bit], CHARGE_ROWS[0][2]);
                self.w.atk.charge = t;
                self.w.atk.hold |= if kind == 1 { B_UP } else { B_DOWN };
                if t == 0.0 {
                    self.w.atk.press |= B_BLAST;
                } else {
                    self.w.atk.hold |= B_BLAST;
                }
            }
            3 => {
                let mut c = self.roll_charge(CHARGE_ROWS[6][bit], CHARGE_ROWS[6][2]);
                if c == 0.0 {
                    c = 0.1;
                }
                self.w.atk.charge = c;
                self.w.atk.hold |= B_RUSH;
                let dir = self.rand(4);
                self.w.atk.hold |= 1 << (dir + 4);
            }
            4 => self.w.atk.hold |= B_GUARD,
            5 => {
                self.w.atk.press |= B_GUARD;
                self.w.atk.hold |= if self.rand(2) == 0 { B_LEFT } else { B_RIGHT };
            }
            6 => self.w.atk.press |= B_DASH,
            7 => {
                self.w.atk.flags |= ATK_WAIT_CLS11;
                self.w.atk.hold |= B_UP;
                self.w.atk.press |= B_DASH;
            }
            8 => {
                self.w.atk.press |= B_DASH;
                self.w.atk.hold |= if self.rand(2) == 0 { B_LEFT } else { B_RIGHT };
            }
            _ => {}
        }
    }

    /// `BtlAiAtk_PickOption`: one of 11 options from the 12-byte row of the rush step.
    fn atk_pick_option(&mut self, lo: usize, hi: usize) -> bool {
        let anim = self.anim(self.me());
        let step = if anim < 0x3C {
            anim - 0x37
        } else {
            anim - 0x3C
        };
        let roll = self.rand(100) + 1;
        self.w.atk.choice = -1;
        let base = (step * 12) as isize;
        let at = |o: usize, k: isize| (o as isize + base + k).max(0) as usize;
        let mut sum = 0;
        for k in 0..11 {
            sum += self.scale(self.pb(at(lo, k)), self.pb(at(hi, k)));
            if sum >= roll {
                match k {
                    0 => self.atk_set_input(0),
                    1 => self.atk_set_input(1),
                    2 => self.atk_set_input(2),
                    4 => self.atk_set_input(3),
                    6 => self.atk_set_input(5),
                    7 => {
                        if self.w.st.flags & 0x2000_0000_0000_0000 != 0 {
                            self.atk_set_input(3);
                        } else {
                            self.atk_set_input(6);
                        }
                    }
                    8 => self.atk_set_input(7),
                    9 => self.atk_set_input(8),
                    10 => self.w.atk.flags |= ATK_IDLE,
                    _ => {}
                }
                self.w.atk.choice = k as i32;
                return true;
            }
        }
        self.w.atk.choice = 11;
        false
    }

    /// `BtlAiAtk_PickLastStep`.
    fn atk_pick_last_step(&mut self, lo: usize, hi: usize) -> bool {
        let roll = self.rand(100) + 1;
        let mut sum = self.scale(self.pb(lo), self.pb(hi));
        if sum >= roll {
            let s = &mut self.w.seq;
            s.phase = 0;
            s.stack[0] = Entry { id: 0x21, arg: 0 };
            s.depth = 1;
            return true;
        }
        sum += self.scale(self.pb(lo + 1), self.pb(hi + 1));
        if sum >= roll {
            self.w.atk.press |= B_DASH;
            return true;
        }
        false
    }

    /// Action 0x19.
    fn combo_dispatch(&mut self) {
        if self.is_interrupted(self.w.seq.top().id) {
            self.w.seq.phase = 3;
        }
        if self.sense_range() != 0 {
            self.w.seq.phase = 3;
        }
        match self.w.seq.phase {
            0 => {
                // BtlAiCombo_Init.
                let cls = self.cls(self.me());
                self.pad_set(0, B_RUSH, 0, 0, 0.0, 0.0);
                if (cls as u32).wrapping_sub(13) < 2 {
                    self.w.seq.phase = 1;
                    self.combo_start();
                }
            }
            1 => self.combo_start(),
            2 => self.atk_run(),
            _ => self.atk_end(),
        }
    }

    /// `BtlAiCombo_Start`.
    fn combo_start(&mut self) {
        let (me, op) = (self.me(), self.op());
        let anim = self.anim(me);
        let cls = [self.cls(me), self.cls(op)];
        let step = if anim < 0x3C {
            anim - 0x37
        } else {
            anim - 0x3C
        };
        self.atk_reset();
        self.w.atk.opp_kind = 0;
        self.w.atk.step = step;
        if (cls[0] as u8).wrapping_sub(13) >= 2 {
            self.w.seq.phase = 3;
            self.atk_end();
            return;
        }
        if step == 4 {
            if cls[0] == 13 {
                self.atk_pick_last_step(0x44, 0x13C);
            }
            if self.w.seq.stack[0].id == 0x21 {
                return;
            }
        } else {
            let picked = if cls[1] == 0x1A {
                self.w.atk.opp_kind = 2;
                self.atk_pick_option(0x88, 0x180)
            } else if (cls[1] as u8).wrapping_sub(13) < 2 {
                self.w.atk.opp_kind = 1;
                self.atk_pick_option(0xB8, 0x1B0)
            } else if cls[0] == 13 {
                self.atk_pick_option(0x14, 0x10C)
            } else if cls[0] == 14 {
                self.atk_pick_option(0x50, 0x148)
            } else {
                false
            };
            if !picked {
                self.w.atk.press |= B_RUSH;
            }
        }
        self.w.seq.phase = 2;
        self.pad_clear(false);
    }

    /// `BtlAiAtk_Run`.
    fn atk_run(&mut self) {
        let me = self.me();
        let anim = self.anim(me);
        let held = self.charge_rate(me);
        let top = self.w.seq.top().id;
        let cls = self.cls(me);
        self.pad_clear(true);
        if self.w.atk.flags & ATK_IDLE != 0 {
            if cls != 0 {
                return;
            }
            self.w.atk.idle_timer -= 1;
            if self.w.atk.idle_timer >= 0 {
                return;
            }
            self.w.seq.phase = 3;
            return;
        }
        if anim != self.w.atk.state {
            if self.w.atk.hold & (B_BLAST | B_RUSH) != 0 {
                if self.w.atk.charge <= held || cls == 0 {
                    self.w.atk.hold = 0;
                }
                if cls == 8 {
                    self.w.seq.phase = 3;
                }
            } else if self.w.atk.flags & ATK_WAIT_CLS11 != 0 {
                if cls == 11 {
                    self.w.seq.phase = 3;
                }
            } else {
                self.w.seq.phase = if top == 0x1A { 3 } else { 1 };
                return;
            }
        } else if self.w.atk.wait_timer > 0 {
            self.w.atk.wait_timer -= 1;
        } else {
            self.w.seq.phase = 3;
        }
        let (hold, press) = (self.w.atk.hold, self.w.atk.press);
        self.pad_set(hold, press, 0, 0, 0.0, 0.0);
    }

    /// `BtlAiAtk_End`.
    fn atk_end(&mut self) {
        self.w.seq.depth = 0;
        self.w.seq.phase = 0;
    }

    /// Action 0x1A.
    fn follow_dispatch(&mut self) {
        if self.is_interrupted(self.w.seq.top().id) {
            self.w.seq.phase = 3;
        }
        match self.w.seq.phase {
            0 => {
                self.w.seq.phase = 1;
                self.follow_start();
            }
            1 => self.follow_start(),
            2 => self.atk_run(),
            _ => {
                // BtlAiFollow_End.
                if self.cls(self.me()) != 11 {
                    if self.w.atk.flags & ATK_CHAIN != 0 {
                        let s = &mut self.w.seq;
                        s.phase = 0;
                        s.stack[0] = Entry { id: 0x19, arg: 0 };
                        s.depth = 1;
                    } else {
                        self.w.seq.depth = 0;
                        self.w.seq.phase = 0;
                    }
                }
            }
        }
    }

    /// `BtlAiFollow_Start`.
    fn follow_start(&mut self) {
        let roll = self.rand(100) + 1;
        let action = self.action(self.me());
        let dir = self.rand(3);
        let (lo, hi) = if action == 0x1F {
            (0x48, 0x140)
        } else {
            (0x80, 0x178)
        };
        self.atk_reset();
        self.w.seq.phase = 2;
        self.pad_clear(false);
        let mut sum = 0;
        for k in 0..5 {
            sum += self.scale(self.pb(lo + k), self.pb(hi + k));
            if sum >= roll {
                match k {
                    0 | 4 => self.atk_set_input(0),
                    1 => self.atk_set_input(1),
                    2 => self.atk_set_input(2),
                    _ => {
                        self.w.atk.press |= B_RUSH;
                        if action == 0x1F {
                            match dir {
                                0 => self.w.atk.hold |= B_UP,
                                1 => self.w.atk.hold |= B_DOWN,
                                _ => {}
                            }
                            self.w.atk.flags |= ATK_CHAIN;
                        }
                    }
                }
                return;
            }
        }
        self.w.atk.press |= B_RUSH;
    }

    /// Action 0x36.
    fn act36_dispatch(&mut self) {
        if self.is_interrupted(self.w.seq.top().id) {
            self.w.seq.phase = 3;
        }
        match self.w.seq.phase {
            0 => {
                // BtlAiAct36_Init.
                let cls = self.cls(self.me());
                let w = self.act_bits(self.me());
                self.pad_set(0, B_BLAST, 0, 0, 0.0, 0.0);
                if w & 0x1000 == 0 {
                    self.w.seq.phase = 3;
                } else if cls == 0x12 {
                    self.w.seq.phase = 1;
                    self.act36_start();
                }
            }
            1 => self.act36_start(),
            2 => self.atk_run(),
            _ => self.atk_end(),
        }
    }

    /// `BtlAiAct36_Start`.
    fn act36_start(&mut self) {
        let me = self.me();
        let roll = self.rand(100) + 1;
        let bit = usize::from(byte(self.param(), 0) & 2 != 0);
        let row = (self.chr(me).blast_shots * 2).max(0) as usize;
        let (lo, hi) = (row + 0xE6, row + 0x1DE);
        let can_press = self.blast_room(me, false) != 0;
        let can_hold = self.blast_room(me, true) != 0;
        let w = self.act_bits(me);
        self.atk_reset();
        self.w.seq.phase = 2;
        self.pad_clear(false);
        if self.scale(self.pb(lo), self.pb(hi)) < roll || (!can_press && !can_hold) {
            self.w.seq.phase = 3;
        } else if self.scale(self.pb(lo + 1), self.pb(hi + 1)) >= roll
            && can_hold
            && w & 0x2000 != 0
        {
            let c = self.roll_charge(CHARGE_ROWS[6][bit], CHARGE_ROWS[6][2]);
            self.w.atk.charge = c;
            self.w.atk.hold |= B_BLAST;
        } else if !can_press || w & 0x1000 == 0 {
            self.w.seq.phase = 3;
        } else {
            self.w.atk.press |= B_BLAST;
        }
    }

    // ---- sense (btl_ai_sense.c) ----

    /// `BtlAiSense_GetRange`: 0 close, 1 middle, 2 far, -1 not sensed.
    fn sense_range(&self) -> i32 {
        let f = self.w.st.flags;
        if f & 0x10 != 0 {
            0
        } else if f & 0x20 != 0 {
            1
        } else if f & 0x40 != 0 {
            2
        } else {
            -1
        }
    }

    /// `BtlAiSense_IsBehindOpponent` of side `s`: side s's opponent faces along the line from s to it.
    fn behind_opponent(&self, s: usize) -> bool {
        let face = facing(self.rot(s ^ 1));
        let mut v = sub(self.pos(s ^ 1), self.pos(s));
        v[1] = 0.0;
        0.0 < dot(face, normalize(v))
    }

    /// `BtlAi_NoteOpponent`.
    fn note_opponent(&mut self, react: i32) {
        let anim = self.anim(self.op());
        self.w.st.opp_action = anim;
        self.w.st.opp_class = i32::from(self.c.ai.class_of(anim));
        self.w.st.react |= react;
    }

    /// `BtlAiSense_Update`.
    fn sense(&mut self) {
        let opp_cls = i32::from(self.cls(self.op()));
        let mut react = self.w.st.react & !0xB;
        self.w.st.flags = 0;
        if opp_cls != self.w.st.opp_class {
            react &= !0x1F_FFE0;
        }
        self.w.st.react = react;
        self.sense_basic();
        // BtlAiSense_NoteOppState.
        if matches!(self.anim(self.op()), 0x97 | 0x189) {
            self.w.st.react |= 4;
        }
        if self.sense_busy() {
            return;
        }
        self.sense_reactions();
        self.sense_derived();
    }

    /// `BtlAiSense_Basic`.
    fn sense_basic(&mut self) {
        let me = self.me();
        let seq = self.c.env.seq_state;
        let ratio = self.ki(me) as f32 / 100000.0;
        let cls = self.cls(me);
        let mut f = 0u64;
        if seq == 3 {
            f |= 2;
        } else if seq == 2 {
            f |= 4;
        }
        if !self.flag(me, 5) {
            f |= 8;
        }
        let r2 = self.radius(me) * 2.0;
        if self.w.mv.dist[1] + r2 > self.g.dist {
            f |= 0x10;
        } else if self.w.mv.dist[2] + r2 > self.g.dist {
            f |= 0x20;
        } else {
            f |= 0x40;
        }
        self.w.st.react |= 1;
        if self.behind_opponent(me) && f & 0x10 != 0 {
            self.w.st.react &= !1;
        }
        let pct = ratio * 100.0;
        f |= if pct < 20.0 {
            0x0080_0000_0000_0000
        } else if pct < 60.0 {
            0x0040_0000_0000_0000
        } else if pct < 100.0 {
            0x0020_0000_0000_0000
        } else {
            0x0010_0000_0000_0000
        };
        if self.sense_bit56() {
            f |= 0x0100_0000_0000_0000;
        }
        if self.sense_bit57() {
            f |= 0x0200_0000_0000_0000;
        }
        if self.sense_bit58() {
            f |= 0x0400_0000_0000_0000;
        }
        if self.armor_break_level(me) >= 4 {
            f |= 0x1000_0000_0000_0000;
        }
        if self.c.data[me ^ 1].tables.params.size_class() == 4 {
            f |= 0x2000_0000_0000_0000;
        }
        if cls == 0x20 {
            f |= 0x20_0000;
        }
        if self.in_clash_a(me) {
            f |= 0x40_0000;
        }
        if self.w.seq.flags & SEQ_CHAIN != 0 {
            f |= 0x0800_0000_0000_0000;
        }
        if self.sense_bit32() {
            f |= 0x1_0000_0000;
        }
        // BtlAiSense_CheckSwitch: no prompt is open.
        let r = self.sense_act10();
        if r > 0 {
            if r == 2 {
                f |= 0x8000_0000_0000_0000;
            }
            self.w.st.react |= 8;
        }
        self.w.st.flags |= f;
    }

    /// Bit 56: transformation is not ported, so the fighter cannot transform; only the timer runs.
    fn sense_bit56(&mut self) -> bool {
        let me = self.me();
        let ratio = self.hp(me) as f32 / self.hp_max(me) as f32;
        let limit = self.scale(self.pb(0xF), self.pb(0x107));
        if matches!(self.action(me), 0xEC..=0xF2) {
            self.w.st.timer18 = 600;
            return false;
        }
        self.w.st.timer18 -= 1;
        if self.w.st.timer18 > 0 {
            return false;
        }
        self.w.st.timer18 = -1;
        let _ = (limit, ratio);
        false
    }

    /// Bit 57: fusion is never possible.
    fn sense_bit57(&mut self) -> bool {
        if matches!(self.action(self.me()), 0xEC..=0xF2) {
            self.w.st.timer1c = 600;
        }
        false
    }

    /// Bit 58: one member, so no switch.
    fn sense_bit58(&mut self) -> bool {
        false
    }

    /// Bit 32: with no technique or blast, only the attribute / progress test of the opponent's attack.
    fn sense_bit32(&self) -> bool {
        let op = self.op();
        let progress = self.technique_frames_left(op);
        let attr = self.attack_attr(op);
        let kind = self.opp_technique_kind();
        let sub = self.opp_skill_kind();
        if (kind as u32).wrapping_sub(1) < 2 || kind == 5 {
            if self.radius(self.me()) + self.speed(op) < self.g.dist {
                return false;
            }
        } else if (kind as u32).wrapping_sub(3) < 2 || kind == 6 {
            if self.incoming_blast(2) == -1 {
                return false;
            }
        } else if sub == 0 {
            if self.incoming_blast(3) == -1 {
                return false;
            }
        } else {
            if attr & 1 == 0 {
                return false;
            }
            if progress < 1.0 || 3.0 <= progress {
                return false;
            }
        }
        let ready = self.chr(self.me()).dodge_window < -29;
        ready && self.w.st.react & 0x40 == 0
    }

    /// `BtlAiSense_CheckAct10`.
    fn sense_act10(&self) -> i32 {
        if self.w.seq.depth == 0 || self.w.seq.stack[0].id != 10 {
            return 0;
        }
        if self.w.st.act10_dist * 0.5 > self.g.dist && self.g.dist > self.w.mv.dist[1] {
            return 2;
        }
        1
    }

    /// `BtlAiSense_IsBusy`.
    fn sense_busy(&mut self) -> bool {
        let me = self.me();
        if self.state_flags(me) & 1 != 0 {
            return true;
        }
        let cls = i32::from(self.cls(me)) - 15;
        if (cls as u32) < 3 {
            if self.attack_hit_pending(me) {
                self.w.st.hit_pending_timer = 4;
                return true;
            }
            self.w.st.hit_pending_timer -= 1;
            if self.w.st.hit_pending_timer > 0 {
                return true;
            }
            self.w.st.hit_pending_timer = 0;
        }
        false
    }

    /// `BtlAiSense_Reactions`.
    fn sense_reactions(&mut self) {
        let (me, op) = (self.me(), self.op());
        let cls = [self.cls(me), self.cls(op)];
        let anim_op = self.anim(op);
        let w = self.act_bits(me);
        let bottom = self.w.seq.stack[0].id;
        let depth = self.w.seq.depth;
        // Bit 51 (flag 0xBE) and bit 50 (stun).
        if self.flag(me, 0xBE) {
            if bottom != 0x52 {
                self.w.st.flags |= 0x0008_0000_0000_0000;
            }
            return;
        }
        if self.chr(me).stun_timer != 0 {
            if bottom != 0x51 {
                self.w.st.flags |= 0x0004_0000_0000_0000;
            }
            return;
        }
        let mut f = 0u64;
        if cls[0] == 2 && !(depth > 0 && bottom == 0x55) {
            f |= 0x400_0000_0000;
        }
        if cls[0] == 4 && self.w.st.react & 0x20000 == 0 {
            f |= 0x0001_0000_0000_0000;
        }
        if cls[0] == 6 && (depth == 0 || bottom != 0x50) {
            f |= 0x0002_0000_0000_0000;
        }
        // Bit 43.
        if w & 0x200_0000_0000 != 0 && w & 0x1C00_0000_0000 == 0 && self.w.st.react & 0x80000 != 0 {
            f |= 0x800_0000_0000;
        }
        if (cls[0] as u8 as u32).wrapping_sub(1) < 7 {
            self.w.seq.flags &= !SEQ_CHAIN;
        }
        if self.sense_bit40(cls[1], anim_op, w) {
            f |= 0x100_0000_0000;
        }
        // Bit 41.
        if self.behind_opponent(op)
            && !(self.w.mv.dist[1] < self.g.dist)
            && !(depth > 0 && matches!(bottom, 6 | 0x5C | 0x50))
        {
            f |= 0x200_0000_0000;
        }
        // Bit 28.
        if self.w.st.flags & 0x10 != 0
            && (cls[0] as u8).wrapping_sub(13) >= 11
            && !self.behind_opponent(op)
            && (cls[1] as u8 as u32).wrapping_sub(13) < 5
            && self.attack_hits_done(me)
            && self.w.st.react & 0x20 == 0
        {
            f |= 0x1000_0000;
            self.w.st.react |= 2;
        }
        // Bit 29.
        if self.w.st.react & 0x200 == 0
            && (i32::from(cls[1]) as u32).wrapping_sub(15) < 3
            && self.w.st.flags & 0x10 != 0
        {
            f |= 0x2000_0000;
            self.w.st.react |= 0x200;
            self.w.st.opp_class = i32::from(cls[1]);
        }
        if self.incoming_blast(0) != -1 {
            f |= 0x4000_0000;
        }
        // Bit 35 needs a technique kind; bit 31.
        let progress = self.technique_frames_left(op);
        let kind = self.opp_technique_kind();
        if kind != 0
            && self.w.st.react & 0x400 == 0
            && progress != -1.0
            && self.chr(me).counter_window == -30
            && !(1.0 < progress)
        {
            f |= 0x8_0000_0000;
        }
        let bit31 = if kind == 0 {
            !(progress < 0.0) && !(1.0 < progress)
        } else {
            cls[1] == 0x18 && !(progress < 1.0 || 3.0 <= progress)
        };
        if bit31 && self.w.st.react & 0x80 == 0 {
            f |= 0x8000_0000;
        }
        if self.w.st.react & 0x8000 == 0 {
            if w & 0x400_0000_0000 != 0 {
                f |= 0x4000_0000_0000;
            }
            if w & 0x800_0000_0000 != 0 {
                f |= 0x8000_0000_0000;
            }
        }
        if w & 0x1000_0000_0000 != 0 && self.w.st.react & 0x4000 == 0 {
            let id = if depth > 0 { self.w.seq.top().id } else { 0 };
            if id.wrapping_sub(0x53) >= 2 {
                f |= 0x2000_0000_0000;
            }
        }
        if w & 0x2000_0000_0000 != 0 && self.w.st.react & 0x100 == 0 {
            f |= 0x2_0000_0000;
        }
        // Bit 34.
        let step = if anim_op < 0x3C {
            anim_op - 0x37
        } else {
            anim_op - 0x3C
        };
        if cls[0] == 0x1A
            && (step as u32) < 4
            && !(depth != 0 && self.w.seq.top().id == 0x47)
            && self.flag(me, 0x66)
        {
            f |= 0x4_0000_0000;
        }
        if (cls[0] as u8 as u32).wrapping_sub(1) < 6 {
            f |= 0x1000_0000_0000;
        }
        // Bits 36..39 need a technique of the opponent.
        self.w.st.flags |= f;
    }

    /// `BtlAiSense_CheckBit40`.
    fn sense_bit40(&mut self, opp_cls: i8, opp_anim: i32, w: u64) -> bool {
        let op = self.op();
        if !self.powered(op) || !self.behind_opponent(op) || (opp_cls as u32).wrapping_sub(13) >= 2
        {
            self.w.st.react &= !0x10;
            return false;
        }
        if w & 0x4000_0000_0000 == 0 {
            return false;
        }
        if self.w.st.react & 0x10 != 0 {
            return false;
        }
        if self.w.st.react & 0x10_0000 != 0 {
            if self.w.st.opp_action == opp_anim {
                return false;
            }
            self.w.st.react &= !0x10_0000;
        }
        !(self.w.seq.depth > 0 && self.w.seq.stack[0].id == 0x5C)
    }

    /// `BtlAiSense_Derived`.
    fn sense_derived(&mut self) {
        let (me, op) = (self.me(), self.op());
        let cls1 = self.cls(op);
        let cls0 = self.cls(me);
        let w = self.act_bits(me);
        let opp_stun = self.chr(op).stun_timer;
        let mut f = self.w.st.flags;
        f |= if self.w.st.react & 1 != 0 {
            0x2000
        } else {
            0x4000
        };
        if self.powered(me) {
            f |= 0x8000;
        }
        if f & 0x10 != 0 {
            f |= 0x80;
            if f & 0x8000 != 0 {
                f |= 0x400;
            }
        } else if f & 0x20 != 0 {
            f |= 0x100;
            if f & 0x8000 != 0 {
                f |= 0x800;
            }
        } else if f & 0x40 != 0 {
            f |= 0x200;
            if f & 0x8000 != 0 {
                f |= 0x1000;
            }
        }
        // BtlAiSense_IsSteep.
        let (pos, opp) = (self.pos(me), self.pos(op));
        let dist = length(sub(opp, pos));
        if !(dist < 30.0) {
            let pitch = -(opp[1] - pos[1]).atan2(dist);
            if if pitch < 0.0 {
                !(-0.7 < pitch)
            } else {
                !(pitch < 0.7)
            } {
                f |= 0x80_0000;
            }
        }
        if self.g.sight & 1 != 0 {
            f |= 0x100_0000;
        }
        if w & 0x3F_B008_09FF != 0 {
            f |= 0x200_0000;
        }
        if cls1 == 6 || opp_stun > 0 {
            f |= 0x1_0000;
        }
        if (cls1 as u8).wrapping_sub(3) < 5 && cls1 != 6 {
            f |= if self.w.st.react & 4 != 0 {
                0x4_0000
            } else {
                0x2_0000
            };
        } else {
            self.w.st.react &= !4;
        }
        if f & 0x40 != 0 && (cls1 == 7 || cls1 == 2) {
            f |= 0x8_0000;
        }
        if f & 0x30 != 0 && (cls1 == 2 || cls1 == 5) {
            f |= 0x10_0000;
        }
        if cls0 == 11 {
            f |= 0x400_0000;
        }
        self.w.st.flags = f;
    }

    // ---- think (btl_ai_think.c) ----

    /// `BtlAiMgr_BuildRates`.
    fn build_rates(&mut self) {
        for i in 0..14 {
            self.w.plan.rate[i] = 0;
            let r = self.find_basic_column(i as i32, 3);
            if r != -1 {
                self.w.plan.rate[i] = self.pair_rate(i as i32, r, 4, 0x2C4, false) as i8;
            }
        }
    }

    /// `AiThink_BuildTotals`.
    fn build_totals(&mut self) {
        for i in 0..14 {
            self.w.plan.total[0][i] = self.sum_pair_rates(i, 0x74, 0x334, AiTables::A) as i8;
            self.w.plan.total[1][i] = self.sum_pair_rates(i, 0xE4, 0x3A4, AiTables::B) as i8;
            self.w.plan.total[2][i] = self.sum_quad_rates(i, 0x178, 0x438, AiTables::C) as i8;
        }
        for j in 0..4 {
            self.w.plan.total[3][j] = self.sum_sub_rates(j) as i8;
        }
    }

    /// `BtlAi_ScaleByGauge`.
    fn scale_by_gauge(&self, lo: &mut i32, hi: &mut i32) {
        let limit = self.scale(self.pu(0x2AC), self.pu(0x56C)) as f32;
        let pct = self.chr(self.me()).gauge.max_power as f32 / 30000.0 * 100.0;
        if limit < pct {
            let k = 1.0 - (pct - limit) / (100.0 - limit);
            *lo = (*lo as f32 * k) as i32;
            *hi = (*hi as f32 * k) as i32;
        }
    }

    /// `BtlAi_GetLowGaugeFactor`.
    fn low_gauge_factor(&self) -> f32 {
        let pct = self.ki(self.me()) as f32 / 100000.0 * 100.0;
        let (lo, hi) = if pct < 20.0 {
            (self.pu(0x2B7), self.pu(0x577))
        } else if pct < 40.0 {
            (self.pu(0x2B6), self.pu(0x576))
        } else {
            return 1.0;
        };
        self.scale(lo, hi) as f32 * 0.1
    }

    /// `BtlAi_GetPairRate`: rows of 8 by class.
    fn pair_rate(&self, cls: i32, off: i32, lo: usize, hi: usize, by_gauge: bool) -> i32 {
        if !(0..14).contains(&cls) {
            return 0;
        }
        let at = (off + cls * 8).max(0) as usize;
        let (mut l, mut h) = (self.ps(lo + at), self.ps(hi + at));
        if by_gauge {
            self.scale_by_gauge(&mut l, &mut h);
        }
        if self.c.tab.func(self.w.plan.cond) == 0x26 {
            let k = self.low_gauge_factor();
            l = (l as f32 * k) as i32;
            h = (h as f32 * k) as i32;
        }
        self.scale(l, h)
    }

    /// `BtlAi_GetQuadRate`: rows of 4 by class.
    fn quad_rate(&self, cls: i32, off: i32, lo: usize, hi: usize, by_gauge: bool) -> i32 {
        if !(0..14).contains(&cls) {
            return 0;
        }
        let at = (off + cls * 4).max(0) as usize;
        let (mut l, mut h) = (self.ps(lo + at), self.ps(hi + at));
        if by_gauge {
            self.scale_by_gauge(&mut l, &mut h);
        }
        self.scale(l, h)
    }

    /// `AiThink_GetSubRate` / `GetSubRate4`: rows of 8 by `plan.sub`.
    fn sub_rate(&self, col: i32, lo: usize, hi: usize, rows: i32) -> i32 {
        let sub = self.w.plan.sub;
        if !(0..rows).contains(&sub) {
            return 0;
        }
        let at = (col + sub * 8).max(0) as usize;
        self.scale(self.ps(lo + at), self.ps(hi + at))
    }

    /// `AiThink_FindBasicColumn`.
    fn find_basic_column(&self, cls: i32, code: i32) -> i32 {
        if cls == 14 {
            return -2;
        }
        if code < 0 {
            return -1;
        }
        (0..8)
            .find(|&i| code == i32::from(self.c.tab.col(AiTables::BASIC + (cls * 8 + i) as usize)))
            .unwrap_or(-1)
    }

    /// `AiThink_TestBasic`.
    fn test_basic(&mut self) -> bool {
        let p = &self.w.plan;
        let col = self.find_basic_column(p.cls, p.cond - 35);
        if col < 0 {
            return false;
        }
        let rate = self.pair_rate(p.cls, col, 4, 0x2C4, false);
        let n = self.w.plan.cond_no.clamp(0, 7) as usize;
        let r = &mut self.w.plan.rolls[n];
        r.1 += rate;
        self.w.seq.roll = r.0;
        self.w.seq.threshold = r.1;
        if r.0 < r.1 {
            self.w.plan.last_roll = r.0 as u8;
            return true;
        }
        false
    }

    /// `AiThink_FindWeightColumn`.
    fn find_weight_column(&self) -> i32 {
        const COUNT: [i32; 6] = [8, 8, 4, 8, 4, 8];
        let p = &self.w.plan;
        let code = p.cond - 40;
        if code < 0 || !(0..6).contains(&p.next) {
            return -1;
        }
        for i in 0..COUNT[p.next as usize] {
            let at = match p.next {
                0 => AiTables::A as i32 + i + p.cls * 8,
                1 => AiTables::B as i32 + i + p.cls * 8,
                2 => AiTables::C as i32 + i + p.cls * 4,
                3 => AiTables::D as i32 + i + p.sub * 8,
                4 => AiTables::E as i32 + i + p.cls * 4,
                _ => AiTables::F as i32 + i + p.sub * 8,
            };
            if code == i32::from(self.c.tab.col(at.max(0) as usize)) {
                return i;
            }
        }
        -1
    }

    /// `AiThink_TestWeighted`.
    fn test_weighted(&mut self, by_gauge: bool) -> bool {
        let fun = self.c.tab.func(self.w.plan.cond);
        let me = self.me();
        let hp = self.hp(me);
        let near = self.scale(self.pu(0x2AE), self.pu(0x56E));
        let col = self.find_weight_column();
        if col == -1 {
            return false;
        }
        let cls = self.w.plan.cls;
        let mut rate = match self.w.plan.next {
            0 => self.pair_rate(cls, col, 0x74, 0x334, by_gauge),
            1 => self.pair_rate(cls, col, 0xE4, 0x3A4, by_gauge),
            2 => self.quad_rate(cls, col, 0x178, 0x438, by_gauge),
            3 => self.sub_rate(col, 0x1B0, 0x470, 20),
            4 => self.quad_rate(cls, col, 0x250, 0x510, by_gauge),
            5 => self.sub_rate(col, 0x288, 0x548, 4),
            _ => return false,
        };
        let st = self.w.st.flags;
        if st & 0x3000_0000_0000_0000 != 0 {
            if fun == 0x31 || fun == 0x51 {
                rate = (rate as f32 * 0.5) as i32;
            }
            if st & 0x2000_0000_0000_0000 != 0 {
                if fun == 0x33 || fun == 0x5E {
                    rate = 0;
                }
                if (fun as u32).wrapping_sub(0x5F) < 2 {
                    if self.w.plan.slot == -1 {
                        return false;
                    }
                    if self.skill_flags(me, self.w.plan.slot as usize) & 0x800_0000 == 0 {
                        rate = (rate as f32 * 0.5) as i32;
                    }
                }
            }
        }
        if hp < 10000 && near != 0 && fun == 0x5D {
            rate = near;
        }
        let n = self.w.plan.cond_no.clamp(0, 7) as usize;
        let r = &mut self.w.plan.rolls[n];
        r.1 += rate;
        self.w.seq.roll = r.0;
        self.w.seq.threshold = r.1;
        r.0 < r.1
    }

    /// `AiThink_TestMove`: a usable move slot of kind (id - 92).
    fn test_move(&mut self) -> bool {
        let me = self.me();
        let stock = self.blast(me) / 100000;
        let kind = self.w.plan.cond - 0x5C;
        let hp = self.hp(me);
        let opp_stun = self.chr(me ^ 1).stun_timer;
        if kind as u32 >= 9 {
            return false;
        }
        self.w.plan.slot = -1;
        let mut list = Vec::new();
        for i in 0..2 {
            if kind != self.move_kind(me, i) || self.w.plan.cooldown > 0 {
                continue;
            }
            let blocked = match kind {
                0 => {
                    if self.move_id(me, i) == 1 {
                        !self.flag(me ^ 1, 5)
                    } else {
                        !(opp_stun < 0x1F)
                    }
                }
                // BtlCharApi_IsMoveSlotActive is never true.
                5 => false,
                6 => self.powered(me),
                _ => false,
            };
            let _ = self.move_flag_100(me, i);
            if blocked {
                continue;
            }
            if self.move_id(me, i) == 0x17 && hp < 10000 {
                continue;
            }
            if stock < self.move_stock(me, i) {
                continue;
            }
            list.push(i as i32);
            self.w.plan.slot = i as i32;
        }
        if list.is_empty() {
            return false;
        }
        if !self.test_weighted(false) {
            return false;
        }
        self.w.plan.slot = if list.len() == 1 {
            list[0]
        } else {
            list[self.rand(list.len() as u32) as usize]
        };
        true
    }

    /// `AiThink_TestSkill`: a usable technique slot. The opponent is never in a technique.
    fn test_skill(&mut self) -> bool {
        let me = self.me();
        let fun = self.c.tab.func(self.w.plan.cond);
        let gauge = self.ki(me);
        let kind = self.w.plan.cond - 0x65;
        let hp = self.hp(me);
        let opp_slot = -1;
        if self.chr(me).tech_delay != 0 {
            return false;
        }
        let any_basic = fun == 0x6B;
        let beat_opp = fun == 0x6A;
        let strict = true;
        if kind as u32 >= 7 && !any_basic && !beat_opp {
            return false;
        }
        self.w.plan.slot = -1;
        if fun == 0x5E && self.w.mv.dist[0] * 1.5 < self.g.dist {
            return false;
        }
        let n = if self.powered(me) { 3 } else { 2 };
        let mut list = Vec::new();
        for i in (0..n).rev() {
            let k = self.skill_kind(me, i);
            if any_basic {
                if k != 1 && k != 2 {
                    continue;
                }
            } else if beat_opp {
                if strict && self.skill_state(me, i) & 0x100 != 0 {
                    continue;
                }
                let _ = (opp_slot, self.skill_power(me, i), self.skill_rank(me, i));
            } else if kind != k {
                continue;
            }
            if k == 5 && self.c.env.stage_4_or_27 {
                // The pose bit is never set.
                continue;
            }
            if (self.skill_flags(me, i) & 0x2000 != 0 || self.skill_id(me, i) == 0x280)
                && hp > 10000
            {
                continue;
            }
            if gauge < self.skill_cost(me, i) {
                continue;
            }
            list.push(i as i32);
        }
        if list.is_empty() {
            return false;
        }
        if list.len() == 1 {
            self.w.plan.slot = list[0];
        }
        if list.len() >= 2 {
            self.w.plan.slot = if list[0] == 2 && self.rand(100) < 50 {
                list[0]
            } else {
                list[self.rand(list.len() as u32) as usize]
            };
        }
        if !self.test_weighted(self.powered(me)) {
            self.w.plan.slot = -1;
            return false;
        }
        true
    }

    /// `AiThCond_ActRateByFlags` / `AiThink_RollActRate`: the rate of a condition's byte of the act row.
    fn act_rate(&self, cond: i32) -> Option<i32> {
        let code = cond - 40;
        if code < 0 {
            return None;
        }
        let idx = (0..24)
            .filter(|&i| code == i32::from(self.c.tab.col(AiTables::ACT + i)))
            .last()?;
        Some(self.scale(self.pu(0x154 + idx), self.pu(0x414 + idx)))
    }

    /// `AiThink_RollActRate`.
    fn roll_act_rate(&mut self, cond: i32) -> bool {
        let roll = self.rand(100);
        self.act_rate(cond).is_some_and(|chance| roll < chance)
    }

    fn cond_act_rate_by_flags(&mut self) -> bool {
        let roll = self.rand(100);
        let me = self.me();
        let bits = self.act_bits(me);
        let Some(chance) = self.act_rate(self.w.plan.cond) else {
            return false;
        };
        self.w.seq.roll = roll;
        self.w.seq.threshold = chance;
        if chance < roll {
            return false;
        }
        let cls = [self.cls(me), self.cls(me ^ 1)];
        let need = |bit: u64| bits & bit != 0;
        match self.c.tab.func(self.w.plan.cond) {
            0x38 => need(0x80),
            0x39 => need(0x100),
            0x3A => need(0x1000_0000),
            0x3B => need(4),
            0x3C => need(0x10),
            0x3D => need(0x2000_0000),
            0x3F => need(0x8000_0000),
            0x49 => need(0x1_0000_0000),
            0x40 | 0x41 => need(8),
            0x42 => need(0x2_0000_0000),
            0x43 => need(0x4_0000_0000),
            0x44 => need(2),
            0x45 => need(1),
            0x46 => need(0x20),
            0x47 => need(0x8_0000) && cls[0] == 0x10,
            0x48 => need(0x40),
            0x4A => need(0x10_0000_0000),
            0x4B => need(0x8_0000_0000),
            0x3E => need(0x80) && cls[1] == 3,
            _ => false,
        }
    }

    fn rate_param(&mut self, n: usize) -> (i32, i32) {
        let roll = self.rand(100);
        (roll, self.scale(self.pb(8 + n), self.pb(0x100 + n)))
    }

    fn rate_profile(&mut self, lo: usize, hi: usize) -> bool {
        let chance = self.scale(self.pu(lo), self.pu(hi));
        let roll = self.rand(100);
        self.w.seq.roll = roll;
        self.w.seq.threshold = chance;
        roll < chance
    }

    /// The rule condition functions (`gBtlAiCondFuncs`).
    fn cond(&mut self, fun: i32, arg: u8) -> bool {
        let me = self.me();
        let arg_i = i32::from(arg);
        let eq = |v: bool| i32::from(v) == arg_i;
        match fun {
            0 => false,
            1 => true,
            2 => {
                let roll = self.rand(100);
                self.w.seq.roll = roll;
                self.w.seq.threshold = arg_i;
                roll < arg_i
            }
            3 => self.w.seq.depth == 0,
            4 => !(self.w.level < arg_i),
            5 => self.w.level < arg_i,
            6 => eq(self.w.st.flags >> 4 & 1 != 0),
            7 => self.ki(me) as f32 / 100000.0 * 100.0 >= arg_i as f32,
            8 => !(self.blast(me) / 100000 < arg_i),
            9 => eq(self.powered(me)),
            10 => eq(self.w.st.flags >> 6 & 1 != 0),
            // Stage 4 / 27 with the pose bit clear (it is never set).
            11 => eq(self.c.env.stage_4_or_27),
            12 => eq(false),
            13 => eq(self.w.plan.slot == 2),
            14 => self.w.kit & 1 == arg_i,
            15 => self.w.kit >> 1 & 1 == arg_i,
            16 => self.w.kit >> 2 & 1 == arg_i,
            17 => {
                self.w.plan.sub = arg_i;
                true
            }
            18 => {
                if self.w.seq.depth == 0 {
                    return false;
                }
                let f = self.c.ai.flags[self.act_index(self.w.seq.top().id)];
                i32::from(f >> 2 & 1) == arg_i
            }
            19 => eq(self.w.level < -1),
            20 => self.cond_react(arg),
            21 => eq(self.c.env.seq_state == 2),
            22 => eq(self.c.env.stage_4_or_27),
            23 => {
                const TBL: [i32; 5] = [40, 55, 70, 85, 95];
                let chance = if self.w.level < 0 {
                    0
                } else {
                    TBL[level_idx(self.w.level)]
                };
                let roll = self.rand(100);
                if self.w.st.react & 0x40000 != 0 {
                    return false;
                }
                self.note_opponent(0x40000);
                roll < chance
            }
            24 => eq(self.blast_limit_a(me) > 0),
            25 => {
                let (roll, chance) = self.rate_param(0);
                self.w.seq.roll = roll;
                self.w.seq.threshold = chance;
                if self.w.st.react & 2 != 0 {
                    self.note_opponent(0x20);
                }
                roll < chance
            }
            26 => {
                let (roll, chance) = self.rate_param(5);
                let p = &self.c.data[me].tables.params;
                if p.flags2() & 0x20 == 0 && p.flags() & 2 == 0 {
                    return false;
                }
                roll < chance
            }
            27 => self.incoming_blast(1) == 2 && arg == 1,
            28 => {
                let (roll, chance) = self.rate_param(6);
                self.note_opponent(0x80);
                roll < chance
            }
            29 => {
                let (roll, chance) = self.rate_param(1);
                if self.opp_technique_kind() == -1 {
                    self.note_opponent(0x40);
                }
                self.w.seq.roll = roll;
                self.w.seq.threshold = chance;
                roll < chance
            }
            30 => {
                let (roll, chance) = self.rate_param(4);
                self.note_opponent(0x100);
                roll < chance
            }
            31 => {
                let anim = self.anim(me ^ 1);
                let idx = if anim < 0x3C {
                    anim - 0x37
                } else {
                    anim - 0x3C
                };
                let roll = self.rand(100);
                if idx as u32 >= 4 {
                    return false;
                }
                let chance =
                    self.scale(self.ps(0x2A8 + idx as usize), self.ps(0x568 + idx as usize));
                self.w.seq.roll = roll;
                self.w.seq.threshold = chance;
                roll < chance
            }
            32 => {
                let (roll, chance) = self.rate_param(2);
                self.w.seq.roll = roll;
                self.w.seq.threshold = chance;
                self.w.st.react |= 0x400;
                roll < chance
            }
            33 => {
                let (roll, mut chance) = self.rate_param(8);
                if arg == 0 {
                    self.w.st.timer18 = 90;
                } else if arg == 1 {
                    self.w.st.timer1c = 90;
                    chance += 20;
                }
                self.w.seq.roll = roll;
                self.w.seq.threshold = chance;
                roll < chance
            }
            34 => {
                let roll = self.rand(100);
                self.w.st.timer20 = 90;
                !self.powered(me) && roll < 10
            }
            35..=37 | 39 => self.test_basic(),
            38 => !self.powered(me) && !self.flag(me, 0x98) && self.test_basic(),
            40 => self.c.data[me].tables.params.flags2() & 1 != 0 && self.test_weighted(false),
            41..=53 | 81 | 82 | 101..=105 | 110..=113 | 116 => self.test_weighted(false),
            54 | 55 | 83 | 84 => self.cond_weighted_by_height(fun),
            56..=76 => self.cond_act_rate_by_flags(),
            77 => {
                let roll = self.rand(100);
                let chance = match arg {
                    0 => self.scale(self.pu(0x2B8), self.pu(0x578)),
                    1 => self.scale(self.pu(0x2BB), self.pu(0x57B)),
                    _ => 0,
                };
                if self.c.env.stage_4_or_27 {
                    return false;
                }
                self.w.seq.roll = roll;
                self.w.seq.threshold = chance;
                roll < chance
            }
            78 => self.rate_profile(0x2B9, 0x579),
            79 => self.rate_profile(0x2BA, 0x57A),
            80 => self.rate_profile(0x2BC, 0x57C),
            85..=93 => self.test_move(),
            94..=100 | 106 | 107 => self.test_skill(),
            108 | 109 => {
                let (lo, hi, bit) = if fun == 108 {
                    (0x2B4, 0x574, 0x4000)
                } else {
                    (0x2B5, 0x575, 0x8000)
                };
                let chance = self.scale(self.pu(lo), self.pu(hi));
                let roll = self.rand(100);
                self.note_opponent(bit);
                self.w.seq.roll = roll;
                self.w.seq.threshold = chance;
                roll < chance
            }
            114 | 115 | 117 => {
                let flags3 = self.c.data[me].tables.params.flags3();
                if !self.powered(me) {
                    return false;
                }
                let ok = match fun {
                    0x73 => flags3 & 0x200 != 0,
                    0x72 => flags3 & 0x10 != 0,
                    _ => flags3 & 8 != 0,
                };
                ok && self.test_weighted(false)
            }
            _ => false,
        }
    }

    /// Condition 20.
    fn cond_react(&mut self, arg: u8) -> bool {
        const GUARD: [i32; 5] = [20, 40, 60, 80, 95];
        const EVADE: [i32; 5] = [10, 20, 30, 40, 50];
        const BLAST_A: [i32; 5] = [0, 4, 8, 12, 15];
        const BLAST_B: [i32; 5] = [0, 3, 5, 8, 10];
        const BLAST_C: [i32; 5] = [0, 2, 3, 4, 5];
        let lv = |t: &[i32; 5], level: i32| if level < 0 { 0 } else { t[level_idx(level)] };
        let level = self.w.level;
        let roll = self.rand(100);
        let anim = self.anim(self.op());
        let chance = match arg {
            0 | 1 => {
                if arg == 0 {
                    // No prompt is open: falls through to the guard chance.
                    self.note_opponent(0x10000);
                }
                lv(&GUARD, level)
            }
            2 => {
                self.note_opponent(0x80000);
                lv(&EVADE, level)
            }
            3 => {
                self.note_opponent(0x10_0000);
                match anim {
                    0x37 | 0x3C => lv(&BLAST_A, level),
                    0x38 | 0x3D => lv(&BLAST_B, level),
                    0x39 | 0x3E => lv(&BLAST_C, level),
                    _ => {
                        self.note_opponent(0x10);
                        return false;
                    }
                }
            }
            _ => return false,
        };
        roll < chance
    }

    /// `AiThCond_WeightedByHeight`.
    fn cond_weighted_by_height(&mut self, fun: i32) -> bool {
        let me = self.me();
        let height = self.height(me);
        let (pos, opp) = (self.pos(me), self.pos(me ^ 1));
        let dy = opp[1] - pos[1];
        match fun {
            0x36 | 0x53 => {
                if opp[1] < pos[1] - height || self.w.st.flags & 0x30 == 0 {
                    return false;
                }
            }
            0x54 => {
                if opp[1] < pos[1] - height || self.w.st.flags & 0x20 == 0 {
                    return false;
                }
            }
            0x37 => {
                if !self.anim_flag_2800(me ^ 1)
                    || self.g.dist > self.g.radius_sum + self.radius(me)
                    || dy.abs() > height
                    || self.behind_opponent(me)
                {
                    return false;
                }
            }
            _ => {}
        }
        self.test_weighted(false)
    }

    /// `AiThink_IsCodeUsable`.
    fn code_usable(&self, code: i32) -> bool {
        let me = self.me();
        let p = &self.c.data[me].tables.params;
        if code == 0 {
            return p.flags2() & 1 != 0;
        }
        let has_move = |k: i32| (0..2).any(|i| self.move_kind(me, i) == k);
        let has_skill = |k: i32| (0..3).any(|i| self.skill_kind(me, i) == k);
        if (code as u32).wrapping_sub(0x34) < 9 {
            return has_move(code - 0x34);
        }
        if (code as u32).wrapping_sub(0x3D) < 7 {
            return has_skill(code - 0x3D);
        }
        match code {
            0x49 => has_skill(3),
            0x4A => has_skill(1) || has_skill(2),
            0x52 => p.flags3() & 0x10 != 0,
            0x53 => p.flags3() & 0x200 != 0,
            0x54 => p.flags3() & 0x20 != 0,
            0x55 => p.flags3() & 8 != 0,
            _ => true,
        }
    }

    /// `AiThink_SumPairRates`.
    fn sum_pair_rates(&self, cls: usize, lo: usize, hi: usize, codes: usize) -> i32 {
        (0..8)
            .filter(|&c| self.code_usable(i32::from(self.c.tab.col(codes + cls * 8 + c))))
            .map(|c| self.scale(self.ps(lo + cls * 8 + c), self.ps(hi + cls * 8 + c)))
            .sum()
    }

    /// `AiThink_SumQuadRates`.
    fn sum_quad_rates(&self, cls: usize, lo: usize, hi: usize, codes: usize) -> i32 {
        (0..4)
            .filter(|&c| self.code_usable(i32::from(self.c.tab.col(codes + cls * 4 + c))))
            .map(|c| self.scale(self.ps(lo + cls * 4 + c), self.ps(hi + cls * 4 + c)))
            .sum()
    }

    /// `AiThink_SumSubRates`.
    fn sum_sub_rates(&self, sub: usize) -> i32 {
        (0..8)
            .filter(|&c| self.code_usable(i32::from(self.c.tab.col(AiTables::F + sub * 8 + c))))
            .map(|c| self.scale(self.ps(0x288 + sub * 8 + c), self.ps(0x548 + sub * 8 + c)))
            .sum()
    }

    /// `AiThink_GetRollRange`.
    fn roll_range(&self, tbl: usize) -> i32 {
        let p = &self.w.plan;
        let idx = if tbl != 3 { p.cls } else { p.sub };
        let total = i32::from(
            usize::try_from(tbl as i32 * 14 + idx)
                .ok()
                .filter(|&i| i < 56)
                .map(|i| &p.total[i / 14][i % 14])
                .copied()
                .unwrap_or(0),
        );
        let hp = self.hp(self.me());
        let near = self.scale(self.pu(0x2AE), self.pu(0x56E));
        if total == 0 {
            return 100;
        }
        if near == 0 || tbl != 1 || !matches!(p.cls, 8 | 9 | 13) {
            return total;
        }
        if hp < 10001 { total + near } else { total }
    }

    /// `AiThink_GetCond38Range`.
    fn cond38_range(&self) -> i32 {
        let k = self.low_gauge_factor();
        if k == 1.0 {
            return 100;
        }
        let rate = f32::from(
            usize::try_from(self.w.plan.cls)
                .ok()
                .and_then(|c| self.w.plan.rate.get(c))
                .copied()
                .unwrap_or(0),
        );
        (rate * k - rate) as i32 + 100
    }

    /// `AiThink_RollGroupGate`: true skips the group this frame.
    fn roll_group_gate(&mut self) -> bool {
        let roll = self.rand(100);
        let sub = self.w.plan.sub;
        let row = match sub {
            0 => 0,
            1 => 8,
            3 => 0x18,
            _ => return false,
        };
        let sum: i32 = (0..8)
            .map(|c| self.scale(self.ps(0x288 + row + c), self.ps(0x548 + row + c)))
            .sum();
        if roll < sum {
            return false;
        }
        match sub {
            0 => self.note_opponent(0x200),
            1 => self.note_opponent(0x800),
            _ => self.note_opponent(0x1000),
        }
        true
    }

    /// `AiThink_GetGroupClass`.
    fn group_class(group: i32) -> i32 {
        match group {
            7..=12 => group - 7,
            14 => 6,
            16..=20 => group - 9,
            23 => 12,
            24 => 13,
            _ => 14,
        }
    }

    /// `AiThink_EvalRules`.
    fn eval_rules(&mut self, list: usize) {
        let ai = self.c.ai;
        let Some(rules) = ai.rules.get(list) else {
            return;
        };
        let mut res = [false; 8];
        let mut last_group = -1;
        let mut skip_group = -1;
        for (n, rule) in rules.iter().enumerate() {
            self.w.plan.rule_no = n as u8;
            let group = i32::from(rule.group);
            if skip_group == group {
                continue;
            }
            if !(0..64).contains(&group) || self.w.st.flags >> group & 1 == 0 {
                continue;
            }
            self.w.plan.cls = Self::group_class(group);
            match group {
                0x1D => self.w.plan.sub = 0,
                0x24 => self.w.plan.sub = 1,
                0x26 => self.w.plan.sub = 2,
                0x25 => self.w.plan.sub = 3,
                _ => {}
            }
            if last_group != group {
                if matches!(group, 0x1D | 0x24 | 0x25) && self.roll_group_gate() {
                    skip_group = group;
                    continue;
                }
                for (i, r) in res.iter_mut().enumerate() {
                    let mut range = 100;
                    *r = false;
                    let set = self.w.seq.rule_set;
                    if (set as u32).wrapping_sub(1) < 3 {
                        range = self.roll_range((set - 1) as usize);
                    } else if set == 7 && (0..4).contains(&self.w.plan.sub) {
                        range = self.roll_range(3);
                    }
                    if self.c.tab.func(i32::from(rule.cond[0])) == 0x26 {
                        range = self.cond38_range();
                    }
                    self.w.plan.range = range as u8;
                    let roll = self.c.rng.range(range as u32) as i32;
                    self.w.plan.rolls[i] = (roll, 0);
                }
            }
            last_group = group;
            if !self.rule_passes(rule, &mut res) {
                continue;
            }
            let kind = rule.acts[0];
            if kind == 0 {
                return;
            }
            if kind == 4 {
                let cls = self.w.plan.cls;
                self.w.plan.next = i32::from(rule.args[0]) - 0x7F;
                self.w.plan.next_group = group as u8;
                let _ = cls;
                return;
            }
            match group {
                0x24 => self.note_opponent(0x800),
                0x25 => self.note_opponent(0x1000),
                0x26 => self.note_opponent(0x2000),
                _ => {}
            }
            self.w.seq.fired_rule = n as i32;
            self.w.seq.fired_group = group;
            self.w.seq.fired_set = self.w.seq.rule_set;
            self.push_rule(rule);
            return;
        }
    }

    /// The conditions of a rule, left to right, up to the first that fails.
    fn rule_passes(&mut self, rule: &AiRule, res: &mut [bool; 8]) -> bool {
        let mut pass = false;
        for i in 0..8 {
            let c = rule.cond[i];
            if c == 0xFE {
                pass = res[i];
                if !pass {
                    return false;
                }
                continue;
            }
            if c == 0xFF {
                break;
            }
            self.w.plan.cond_no = i as i32;
            self.w.plan.cond = i32::from(c);
            let fun = self.c.tab.func(i32::from(c));
            pass = self.cond(fun, rule.arg[i]);
            res[i] = pass;
            if !pass {
                return false;
            }
        }
        pass
    }

    /// `BtlAiSeq_PushRule`.
    fn push_rule(&mut self, rule: &AiRule) {
        const SKIPS: [i32; 5] = [2, 3, 4, 5, 99999];
        let level = self.w.level;
        let s = &mut self.w.seq;
        s.depth = 0;
        s.phase = 0;
        self.w.mv.path.clear();
        s.skip -= 1;
        if s.skip < 0 {
            s.skip = SKIPS[level_idx(level)];
        }
        for i in 0..4 {
            let id = rule.acts[i];
            if id == 0xFF {
                break;
            }
            if id == 2 && s.skip > 0 && rule.args[i] == 0x80 {
                continue;
            }
            if id == 0x3D {
                self.w.plan.cooldown = 90;
            }
            s.stack[s.depth as usize] = Entry {
                id: u32::from(id),
                arg: rule.args[i].wrapping_add(0x81) as i8,
            };
            s.depth += 1;
        }
    }

    /// `AiThink_GetBlastStep`.
    fn blast_step(&self) -> i32 {
        let anim = self.anim(self.me());
        if self.c.ai.class_of(anim) == 15 {
            return 1;
        }
        if (anim - 0x3C) as u32 >= 4 {
            return 0;
        }
        if self.w.seq.flags & SEQ_COMBO_SECOND != 0 {
            anim - 0x3A
        } else {
            anim - 0x3B
        }
    }

    /// `AiThink_Think`.
    fn think(&mut self) {
        const NEXT: [i32; 6] = [1, 2, 3, 5, 6, 7];
        if self.w.plan.cooldown > 0 {
            self.w.plan.cooldown -= 1;
        }
        self.w.seq.rule_set = 7;
        self.w.plan.next = 5;
        self.eval_rules(7);
        if self.w.seq.depth != 0 || self.w.level < 0 {
            return;
        }
        self.w.seq.rule_set = 4;
        self.eval_rules(4);
        if self.w.seq.depth != 0 {
            return;
        }
        self.w.seq.rule_set = 0;
        self.eval_rules(0);
        let Some(&set) = usize::try_from(self.w.plan.next)
            .ok()
            .and_then(|n| NEXT.get(n))
        else {
            return;
        };
        self.w.seq.rule_set = set;
        self.eval_rules(set as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::{rotate_y, scale_by_level};

    #[test]
    fn level_scaling_interpolates_between_the_level_0_and_29_bytes() {
        assert_eq!(scale_by_level(0, 10, 40), 10);
        assert_eq!(scale_by_level(29, 10, 40), 40);
        assert_eq!(scale_by_level(-1, 10, 40), 10);
        assert_eq!(scale_by_level(13, 0, 29), 13);
        // Truncated toward zero, as the original converts: 40 + (int)(-29.99).
        assert_eq!(scale_by_level(29, 40, 10), 11);
    }

    #[test]
    fn rotate_y_turns_z_towards_x() {
        let v = rotate_y([0.0, 1.0, 1.0], std::f32::consts::FRAC_PI_2);
        assert!((v[0] - 1.0).abs() < 1e-6 && v[1] == 1.0 && v[2].abs() < 1e-6);
    }
}
