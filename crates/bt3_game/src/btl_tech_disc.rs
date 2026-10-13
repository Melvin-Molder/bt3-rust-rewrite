//! The disc (`EftDisc_*`, src/battle/eft_disc.c and the init / update callbacks in src/battle/eft_body_fx.c): a
//! spinning projectile of layer 3, either a ki blast of type 4 (grown in the hand on request 0x17, thrown by the
//! fire event; one thrown without a held disc grows at once) or 5 (flying from a model node), or a piece of the
//! multi technique effect (type 5, `EftMulti_*`, src/battle/eft_shot_tech.c: pieces grown in the hand and released
//! one by one on FIRE, or each fired from the muzzle). A flying disc homes with a banking term and adds one
//! record a frame (a ki blast disc a blast record of its ki blast's parameters, radius 4.5 times its scale; a piece
//! a technique record, radius three times its set's trail width); any result of the hit pass ends it.
//!
//! The held ki blast discs are listed per fighter (`EftDiscMgr`): the throw takes the first not yet thrown, and
//! the ki blast parameters go to the list's FIRST disc, as in the original; a disc's end unlinks the list's first.
//!
//! A piece drives its technique set's emitters on node slot 5 (`EftDisc_SpawnParts`).
//!
//! A ki blast disc dies when its fighter changes (`BtlCharApi_IsChanging`).
//!
//! The model (`EftDisc_Draw`, layer 1): common effect entry 0x239 with texture set 0x238 for kinds 0..2, 0x23B with
//! 0x23A for kinds 3 / 4 (techniques 0x158 and 0x202 give a piece their pack's set, `EftDisc_SetModelTex`); its
//! alpha grows in and fades out, it spins (libc `rand()`), banks toward the opponent and unrolls. A disc of kinds
//! 0..2 lends its pair's CLUT to entry `tex_a` of its set (`EftDisc_StepTex`): the first such disc of the frame
//! sets it, and it stays until another changes it.

use std::f32::consts::{FRAC_PI_4, PI};

use bt3_data::eft_pack::{EmitSet, SetHead};

use crate::battle::{IDENTITY, Mtx, mul};
use crate::btl_blast::{Arg, HitRec, Tag, World};
use crate::btl_chr::{Chr, SideData, add, dot, length, scale3, sub};
use crate::btl_obj::{node_side, rot_x, rot_y, rot_z};
use crate::btl_tech_fx::{
    EV_ABORT, EV_END, EV_FIRE, EV_START, EmitNodes, T_HIT_CHAR, T_MULTI, TechSrc, aim_dir,
    aim_dir_keep, emit_state, in_rush_sequence, parts_alive, reset_parts, stopped, update_parts,
    update_trail_width,
};
use crate::eft::{EmitState, Modules};

/// `EftDiscMgr_Init`: discs, both fighters together.
const DISCS_MAX: usize = 20;
/// `EftDiscMgr.node`: held list entries.
const NODES_MAX: usize = 20;
/// `EftMulti.piece`.
const PIECES: usize = 10;

// EFT_DISC_*.
const D_ALIVE: u32 = 0x1;
const D_END: u32 = 0x2;
const D_DEAD: u32 = 0x4;
const D_HELD: u32 = 0x8;
const D_FLYING: u32 = 0x20;
const D_SHOWN: u32 = 0x40;
const D_PIECE: u32 = 0x80;
const D_REQ200: u32 = 0x200;
const D_LOST: u32 = 0x400;
const D_GROWN: u32 = 0x800;
const D_AUTO: u32 = 0x1000;
const D_MOVED: u32 = 0x2000;

/// `EftDiscArg`: what a disc is created with.
#[derive(Clone, Copy, Debug, Default)]
struct DiscArg {
    src: Option<TechSrc>,
    dir: [f32; 3],
    pos: [f32; 3],
    scale: f32,
    speed: f32,
    /// Seconds.
    life: f32,
    grow: f32,
    /// The part of the life after which it fades out.
    fade: f32,
    turn: f32,
    bank: f32,
    /// The roll, in turns, and the seconds over which it unrolls.
    roll: f32,
    roll_time: f32,
    /// The texture entry and the entry whose CLUT it takes (`texB`: 0..3 by the ki blast's level).
    tex_a: u8,
    tex_b: u8,
    owner: usize,
    /// 0 held ki blast, 1 thrown ki blast, 2 held technique piece, 3 ki blast from a node, 4 technique piece.
    kind: u8,
    last_hit: bool,
    hand: bool,
}

/// An `EftDisc`.
#[derive(Clone, Debug)]
pub(crate) struct Disc {
    id: u32,
    arg: DiscArg,
    tag: Tag,
    flags: u32,
    /// The ki blast parameters its records carry (`atk`).
    atk: Arg,
    prev: [f32; 3],
    life: f32,
    age: f32,
    grow_time: f32,
    timer: f32,
    size: f32,
    head_set: SetHead,
    trail_width: f32,
    trail_time: f32,
    /// A piece's emitters (its technique slot's set).
    emit: Option<EmitState>,
    /// Its look: the texture set (the technique pack's after `EftDisc_SetModelTex`), the alpha (0..127) and growth,
    /// the spin added each frame to one of two angles, the bank, the roll and its unrolling, the fade, the matrix.
    pack_tex: bool,
    alpha: f32,
    grown: f32,
    spin: f32,
    rot_x: f32,
    rot_y: f32,
    bank_angle: f32,
    roll: f32,
    roll_left: f32,
    roll_time: f32,
    fade_time: f32,
    fade_left: f32,
    mtx: Mtx,
}

/// A disc's model as `EftDisc_Draw` queues it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DiscDraw {
    pub owner: usize,
    /// A technique piece (hidden as the effects of type 1, else of type 0).
    pub piece: bool,
    /// Model and set 0x23B / 0x23A (kinds 3, 4), else 0x239 / 0x238.
    pub model_b: bool,
    /// Its technique pack's texture set instead (`EftDisc_SetModelTex`): the side and slot.
    pub pack_tex: Option<(usize, i32)>,
    pub mtx: Mtx,
    /// `EftMesh_SetColor(127, 127, 127, alpha)`.
    pub alpha: u8,
}

/// A held list entry (`EftDiscNode`): 1 used, 0x10 thrown at once, 0x20 thrown.
#[derive(Clone, Copy, Debug, Default)]
struct Node {
    flags: u32,
    disc: Option<u32>,
    next: Option<usize>,
}

/// The discs of the scene and the held lists (`gEftDisc`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Discs {
    list: Vec<Disc>,
    nodes: [Node; NODES_MAX],
    head: [Option<usize>; 2],
    tail: [Option<usize>; 2],
    cursor: usize,
    /// The discs whose spin's libc `rand()` is drawn outside the scene update, at its start.
    pub pending_spin: Vec<u32>,
    /// Which entry's CLUT each entry of set 0x238 is drawn with (`EftDisc_StepTex`; `None` its own), and the entries
    /// stepped this frame.
    clut_a: [Option<u8>; 16],
    stepped_a: u32,
}

/// `EftDisc_Home`: `EftAim_Home` with a banking term: the turn is reduced by |cur.y to.z - cur.z to.y| times
/// `bank` (at most to nothing).
#[allow(clippy::too_many_arguments)]
fn disc_home(
    chars: &[Chr; 2],
    pos: [f32; 3],
    dir: &mut [f32; 3],
    owner: usize,
    speed: f32,
    max_turn: f32,
    bank: f32,
) -> [f32; 3] {
    let norm = crate::btl_blast::norm;
    *dir = norm(*dir);
    if !chars[owner].flags.test(5) {
        return *dir;
    }
    let opp = &chars[owner ^ 1];
    let mut target = opp.obj.node_pos(0x11);
    let frames = length(sub(target, pos)) / speed;
    target = add(target, scale3(opp.pose.moved, frames.min(15.0) * 0.5));
    let to = norm(sub(target, pos));
    let cur = norm(*dir);
    let d = dot(to, cur);
    if d > 0.0 {
        let mut angle = d.min(1.0).acos();
        if frames > 2.0 {
            angle /= frames * 0.5;
        }
        let t = if max_turn < angle { max_turn } else { angle };
        let a = cross(cur, to);
        let axis = norm(a);
        let k = ((cur[1] * to[2] - cur[2] * to[1]) * bank).clamp(-1.0, 1.0);
        norm(rotate_axis(cur, axis, t * (1.0 - k.abs())))
    } else {
        cur
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `EftDisc_RotateAboutAxis` (Rodrigues).
fn rotate_axis(v: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    add(
        add(scale3(v, c), scale3(cross(axis, v), s)),
        scale3(axis, dot(axis, v) * (1.0 - c)),
    )
}

/// The hand of a firing node (`BtlObj_GetNodeSide`): 0 for side 0, else 1.
fn hand_of(node: i32) -> bool {
    node_side(node) != 0
}

impl Discs {
    fn find(&mut self, id: Option<u32>) -> Option<&mut Disc> {
        let id = id?;
        self.list
            .iter_mut()
            .find(|d| d.id == id && d.flags & D_ALIVE != 0)
    }

    /// `EftDisc_IsAlive`.
    fn alive(&self, id: Option<u32>) -> bool {
        id.is_some_and(|id| {
            self.list
                .iter()
                .any(|d| d.id == id && d.flags & D_ALIVE != 0)
        })
    }

    /// `EftDisc_Init`'s spin of kinds 0..2 from its libc `rand()`: 0.1 to 0.11 turns of pi a frame.
    pub(crate) fn set_spin(&mut self, id: u32, r: i32) {
        if let Some(d) = self.list.iter_mut().find(|d| d.id == id) {
            d.spin = (r as f32 / 2_147_483_647.0 * 0.01 + 0.1) * PI;
        }
    }

    /// `EftDisc_Draw` of every disc not dead, its model shown (whether its fighter hides it is the caller's).
    pub(crate) fn draws(&self) -> impl Iterator<Item = DiscDraw> + '_ {
        self.list
            .iter()
            .filter(|d| d.flags & D_DEAD == 0 && d.flags & D_SHOWN != 0)
            .map(|d| DiscDraw {
                owner: d.arg.owner,
                piece: d.flags & D_PIECE != 0,
                model_b: d.arg.kind >= 3,
                pack_tex: d
                    .pack_tex
                    .then(|| d.arg.src.map(|s| (s.side, s.slot)))
                    .flatten(),
                mtx: d.mtx,
                alpha: d.alpha as u8,
            })
    }

    /// The entry whose CLUT entry `entry` of set 0x238 is drawn with.
    pub(crate) fn clut_a(&self, entry: usize) -> usize {
        self.clut_a
            .get(entry)
            .copied()
            .flatten()
            .map_or(entry, usize::from)
    }

    pub(crate) fn tag_mut(&mut self, id: u32) -> Option<&mut Tag> {
        self.list
            .iter_mut()
            .find(|d| d.id == id)
            .map(|d| &mut d.tag)
    }

    /// A fighter's discs reset (`EftDisc_Reset`, then `EftDisc_Term`).
    pub(crate) fn reset_side(&mut self, fx: &mut Modules, side: usize) {
        let mut i = 0;
        while i < self.list.len() {
            if self.list[i].arg.owner == side {
                let mut d = self.list.remove(i);
                if let Some(st) = &mut d.emit {
                    st.kill_all(fx);
                }
                if d.flags & D_PIECE == 0 && d.arg.kind < 2 {
                    self.pop_held(d.arg.owner);
                }
            } else {
                i += 1;
            }
        }
    }

    /// `EftDiscMgr_AllocNode`: the first free entry from the cursor.
    fn alloc_node(&mut self) -> Option<usize> {
        if self.cursor >= NODES_MAX {
            self.cursor = 0;
        }
        let start = self.cursor;
        let mut i = start;
        loop {
            let n = i;
            i = (i + 1) % NODES_MAX;
            if self.nodes[n].flags == 0 {
                self.cursor = i;
                return Some(n);
            }
            if i == start {
                return None;
            }
        }
    }

    /// `EftDiscMgr_PushHeld`.
    fn push_held(&mut self, disc: u32, owner: usize, kind: u8) -> bool {
        let Some(n) = self.alloc_node() else {
            return false;
        };
        self.nodes[n] = Node {
            flags: 1 | if kind == 1 { 0x10 } else { 0 },
            disc: Some(disc),
            next: None,
        };
        match self.tail[owner] {
            Some(t) if self.head[owner].is_some() => self.nodes[t].next = Some(n),
            _ => self.head[owner] = Some(n),
        }
        self.tail[owner] = Some(n);
        true
    }

    /// `EftDiscMgr_PopHeld`: the first entry of the fighter's list is freed.
    fn pop_held(&mut self, owner: usize) {
        if let Some(n) = self.head[owner] {
            self.head[owner] = self.nodes[n].next;
            self.nodes[n] = Node::default();
        }
    }

    /// `EftDisc_SetHeldAtk`: the ki blast parameters go to the fighter's first held disc.
    fn set_held_atk(&mut self, owner: usize, atk: &Arg) {
        let id = self.head[owner].and_then(|n| self.nodes[n].disc);
        if let Some(d) = self.find(id) {
            d.atk = *atk;
        }
    }

    /// `EftDisc_Create` / `BtlTaskList_AddTail` with `EftDisc_Init`; `None` when the list is full.
    fn create(
        &mut self,
        arg: DiscArg,
        data: &SideData,
        set: Option<&EmitSet>,
        next_id: &mut u32,
    ) -> Option<u32> {
        if self.list.len() >= DISCS_MAX {
            return None;
        }
        *next_id += 1;
        let id = *next_id;
        let head_set = set.map(|s| s.head).unwrap_or_default();
        let life = arg.life * 30.0;
        let (roll_left, roll_time) = if arg.roll_time < 0.0 {
            (-1.0, 1.0)
        } else {
            (arg.roll_time * 30.0, arg.roll_time * 30.0)
        };
        let mut d = Disc {
            id,
            arg,
            tag: Tag::default(),
            flags: 0,
            atk: Arg::default(),
            prev: [0.0; 3],
            life,
            age: 0.0,
            grow_time: arg.grow * 30.0,
            timer: 0.0,
            size: data.height() / 19.35,
            head_set,
            trail_width: head_set.width[0],
            trail_time: 0.0,
            emit: None,
            pack_tex: false,
            alpha: 0.0,
            grown: 0.0,
            spin: 0.0,
            rot_x: 0.0,
            rot_y: 0.0,
            bank_angle: 0.0,
            roll: eft_wrap(arg.roll * 6.283_185_3),
            roll_left,
            roll_time,
            fade_time: life * (1.0 - arg.fade),
            fade_left: life * (1.0 - arg.fade),
            mtx: IDENTITY,
        };
        if arg.kind < 3 && !arg.hand {
            d.roll = -d.roll;
        }
        match arg.kind {
            0 | 1 => {
                self.pending_spin.push(id);
                if !self.push_held(id, arg.owner, arg.kind) {
                    d.flags |= D_DEAD;
                }
                if arg.kind == 1 {
                    d.flags |= D_AUTO;
                }
                d.flags |= D_HELD;
                d.size *= 1.5;
            }
            2 => {
                self.pending_spin.push(id);
                d.flags |= D_PIECE | D_HELD;
            }
            3 | 4 => {
                d.rot_y = PI;
                d.grown = 1.0;
                d.flags |= if arg.kind == 3 {
                    D_FLYING
                } else {
                    D_PIECE | D_FLYING
                };
            }
            _ => {}
        }
        if d.flags & D_PIECE != 0 {
            d.emit = arg.src.and_then(|s| emit_state(data, s.slot));
        }
        d.flags |= D_ALIVE;
        self.list.push(d);
        Some(id)
    }

    /// The texture pair (and record class) of a ki blast's level.
    fn ki_arg(atk: &Arg, kind: u8) -> DiscArg {
        DiscArg {
            src: None,
            dir: atk.dir,
            pos: [0.0; 3],
            scale: atk.radius,
            speed: atk.speed,
            life: atk.life as f32 / 30.0,
            grow: 0.0,
            fade: 0.7,
            turn: atk.turn,
            bank: 1.5,
            roll: 0.0,
            roll_time: 0.0,
            tex_a: 0,
            tex_b: level_pair(atk.unk1b),
            owner: atk.owner,
            kind,
            last_hit: false,
            hand: hand_of(atk.node as i32),
        }
    }

    /// `EftDisc_SpawnHeld` (request 0x17 of a ki blast of type 4): a disc growing in the firing hand.
    pub(crate) fn spawn_held(&mut self, me: &Chr, data: &SideData, next_id: &mut u32) {
        let mut node = me.blast_node;
        if node == 0x15 {
            node = 0x1F;
        }
        if node == 0x23 {
            node = 0x2D;
        }
        let arg = DiscArg {
            src: None,
            dir: me.aim_dir,
            pos: [0.0; 3],
            scale: 1.0,
            speed: 0.0,
            life: 1.0,
            grow: 0.3,
            fade: 0.7,
            turn: 1.0,
            bank: 1.5,
            roll: 0.0,
            roll_time: 0.0,
            tex_a: 0,
            // BtlKiBlast_GetUnk3: the ki blast record's byte 3.
            tex_b: level_pair(crate::btl_blast::current(me, data).unk3()),
            owner: me.player,
            kind: 0,
            last_hit: false,
            hand: hand_of(node),
        };
        self.create(arg, data, None, next_id);
    }

    /// `EftDisc_Throw` (the fire event of a ki blast of type 4): the fighter's first held disc not yet thrown
    /// flies with the ki blast's parameters; without one (or with one that lost its owner) a new disc is thrown.
    pub(crate) fn throw(&mut self, atk: &Arg, data: &SideData, next_id: &mut u32) {
        let owner = atk.owner;
        let mut n = self.head[owner];
        while let Some(k) = n {
            if self.nodes[k].flags & 0x30 == 0 {
                break;
            }
            n = self.nodes[k].next;
        }
        let Some(k) = n else {
            return self.spawn_thrown(atk, data, next_id);
        };
        let id = self.nodes[k].disc;
        let usable = self
            .list
            .iter()
            .find(|d| Some(d.id) == id)
            .map(|d| (d.flags & D_LOST == 0, d.flags & D_ALIVE != 0));
        match usable {
            None | Some((false, _)) => self.spawn_thrown(atk, data, next_id),
            Some((true, true)) => {
                self.set_held_atk(owner, atk);
                let d = self.find(id).expect("alive");
                d.arg.dir = atk.dir;
                d.arg.life = atk.life as f32 / 30.0;
                d.life = atk.life as f32;
                d.age = 0.0;
                d.fade_time = atk.life as f32 * (1.0 - d.arg.fade);
                d.fade_left = d.fade_time;
                d.arg.speed = atk.speed;
                d.arg.turn = atk.turn;
                d.flags |= D_FLYING;
                d.flags &= !D_HELD;
                self.nodes[k].flags |= 0x20;
            }
            Some((true, false)) => {}
        }
    }

    /// `EftDisc_SpawnThrown`: a disc thrown as soon as it is grown (at once); the ki blast parameters go to the
    /// fighter's first held disc.
    fn spawn_thrown(&mut self, atk: &Arg, data: &SideData, next_id: &mut u32) {
        let arg = Self::ki_arg(atk, 1);
        if self.create(arg, data, None, next_id).is_some() {
            self.set_held_atk(atk.owner, atk);
        }
    }

    /// `EftDisc_SpawnFromNode` (the fire event of a ki blast of type 5): a disc flying from the event's node; its
    /// life is the ki blast's frames taken as seconds.
    pub(crate) fn spawn_from_node(
        &mut self,
        me: &Chr,
        atk: &Arg,
        data: &SideData,
        next_id: &mut u32,
    ) {
        let mut arg = Self::ki_arg(atk, 3);
        arg.pos = me.obj.node_pos(atk.node);
        arg.life = atk.life as f32;
        arg.grow = 0.05;
        arg.bank = 0.0;
        arg.roll = -0.42;
        arg.tex_b = 0;
        arg.hand = false;
        if let Some(id) = self.create(arg, data, None, next_id)
            && let Some(d) = self.find(Some(id))
        {
            d.atk = *atk;
        }
    }

    /// The discs' updates (layer 3, before the blast objects): each one's record; a dead one is removed.
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        events: [u32; 2],
        records: &mut Vec<HitRec>,
        max: usize,
    ) {
        // EftDiscMgr_Update: no entry stepped yet this frame.
        self.stepped_a = 0;
        let mut i = 0;
        while i < self.list.len() {
            let owner = self.list[i].arg.owner;
            let rec = self.list[i].update(w, fx, events[owner]);
            if let Some(rec) = rec
                && records.len() < max
            {
                records.push(rec);
            }
            // EftDisc_StepTex: a disc of kinds 0..2 on set 0x238 gives its entry its pair's CLUT (the other sets'
            // pieces only take entry 0's own).
            let d = &self.list[i];
            if d.flags & D_DEAD == 0 && d.arg.kind < 3 && !d.pack_tex {
                let a = usize::from(d.arg.tex_a);
                if a < self.clut_a.len() && self.stepped_a & (1 << a) == 0 {
                    self.clut_a[a] = Some(d.arg.tex_b);
                    self.stepped_a |= 1 << a;
                }
            }
            if self.list[i].flags & D_ALIVE == 0 {
                let d = self.list.remove(i);
                // EftDisc_Term.
                if d.flags & D_PIECE == 0 && d.arg.kind < 2 {
                    self.pop_held(d.arg.owner);
                }
            } else {
                i += 1;
            }
        }
    }

    /// `EftDisc_PostUpdate`: any result ends the disc; a piece's emitters, given the reset flags when the pass moved
    /// it.
    pub(crate) fn post_update(&mut self, w: &mut World, fx: &mut Modules) {
        for d in &mut self.list {
            if d.stopped(w) {
                continue;
            }
            if d.tag.result & 0x3F != 0 {
                d.flags |= D_DEAD;
            }
            if d.flags & D_PIECE != 0
                && let (Some(src), Some(st)) = (d.arg.src, &mut d.emit)
            {
                parts_alive(fx, w.data[src.side], &src, st, false);
                if d.tag.result & 1 != 0 {
                    let skip = piece_skip(d.flags);
                    let a = d.arg;
                    reset_parts(
                        fx,
                        w,
                        &src,
                        st,
                        &EmitNodes::default(),
                        a.pos,
                        a.dir,
                        a.scale,
                        skip,
                    );
                }
            }
        }
    }

    /// The live discs' positions of a fighter, for tests.
    #[cfg(test)]
    pub(crate) fn of(&self, side: usize) -> Vec<[f32; 3]> {
        self.list
            .iter()
            .filter(|d| d.arg.owner == side)
            .map(|d| d.arg.pos)
            .collect()
    }
}

impl Disc {
    /// `BtlScene_IsEffectStopped`: kind 0 for a ki blast disc (any fighter stopped or in a rush sequence), 1 for a
    /// piece (its fighter stopped).
    fn stopped(&self, w: &World) -> bool {
        if self.flags & D_PIECE != 0 {
            stopped(w, self.arg.owner)
        } else {
            w.paused || w.chars.iter().any(|c| c.is_frozen() || in_rush_sequence(c))
        }
    }

    /// `EftDisc_Update`: a held disc grows and follows the hand (a ki blast disc of kind 0 is lost once its fighter
    /// stops charging, flag 0x8E), one thrown at once leaves when grown; a flying one homes and moves, ends at its
    /// life, and adds its record.
    fn update(&mut self, w: &mut World, fx: &mut Modules, events: u32) -> Option<HitRec> {
        let owner = self.arg.owner;
        let mut hold = false;
        if self.flags & D_PIECE == 0 {
            // BtlCharApi_IsChanging.
            if matches!(w.chars[owner].act.current, 0xEC..=0xF8 | 0x103 | 0x104) {
                self.flags |= D_DEAD;
            }
            if self.arg.kind == 0 && self.flags & D_HELD != 0 && !w.chars[owner].flags.test(0x8E) {
                self.flags |= D_LOST | D_DEAD;
            }
        } else {
            hold = self.arg.src.is_some_and(|s| s.def.flags & 2 != 0);
        }
        if self.stopped(w) || hold {
            return None;
        }
        let me = &w.chars[owner];
        let mut pitch = 0.0;
        if self.flags & D_HELD != 0 {
            if self.arg.kind < 3 {
                if self.timer < self.grow_time {
                    let t = (self.timer / self.grow_time).clamp(0.0, 1.0);
                    self.grown = t;
                    self.alpha = t * 127.0;
                } else {
                    self.alpha = 127.0;
                    self.grown = 1.0;
                    self.flags |= D_GROWN;
                }
                let (a, b) = if self.arg.hand {
                    (0x14, 0x15)
                } else {
                    (0x22, 0x23)
                };
                let (v0, v1) = (me.obj.node_pos(a), me.obj.node_pos(b));
                let c = scale3(crate::btl_blast::norm(sub(v1, v0)), self.size * 3.0);
                self.arg.pos = add(v1, c);
                self.arg.pos[1] += eft_wrap(self.timer * 0.04 * PI).cos();
            }
            if self.flags & D_AUTO != 0 && self.flags & D_GROWN != 0 {
                self.flags &= !D_HELD;
                self.flags |= D_FLYING;
            }
        }
        if self.flags & D_FLYING != 0 {
            let a = &mut self.arg;
            a.dir = disc_home(&*w.chars, a.pos, &mut a.dir, owner, a.speed, a.turn, a.bank);
            self.prev = if self.flags & D_MOVED != 0 {
                a.pos
            } else {
                me.obj.node_pos(0x11)
            };
            a.pos = add(a.pos, scale3(a.dir, a.speed));
            pitch = eft_wrap((-a.dir[1]).clamp(-1.0, 1.0).asin());
            if a.kind >= 3 {
                self.alpha = if self.timer < self.grow_time {
                    (self.timer / self.grow_time).clamp(0.0, 1.0) * 127.0
                } else {
                    127.0
                };
            }
            if self.life - self.fade_time <= self.age || self.flags & D_END != 0 {
                self.alpha = (self.fade_left / self.fade_time).clamp(0.0, 1.0) * 127.0;
                self.fade_left -= 1.0;
            }
            if self.arg.kind < 3 {
                // The roll unwinds over its time; the bank leans toward the opponent ahead (when locked on).
                if 0.0 < self.roll_left {
                    self.roll *= (self.roll_left / self.roll_time).clamp(0.0, 1.0);
                    self.roll_left -= 1.0;
                }
                if me.flags.test(5) {
                    let opp = &w.chars[owner ^ 1];
                    let to = crate::btl_blast::norm(sub(opp.obj.node_pos(3), self.arg.pos));
                    let d = self.arg.dir;
                    if 0.0 <= dot(d, to) {
                        let side = d[2] * to[0] - d[0] * to[2];
                        self.bank_angle = (side * PI * 1.2).clamp(-FRAC_PI_4, FRAC_PI_4);
                    }
                }
            }
            self.age += 1.0;
            if self.life <= self.age {
                self.flags |= D_DEAD;
            }
            self.flags |= D_MOVED;
        }
        if self.grown <= 0.0 || self.alpha <= 0.0 {
            self.flags &= !D_SHOWN;
        } else {
            self.flags |= D_SHOWN;
        }
        if self.arg.kind < 3 {
            self.rot_y = eft_wrap(-(self.rot_y + self.spin));
        } else {
            self.rot_x = eft_wrap(self.rot_x + self.spin);
        }
        self.mtx = self.model_mtx(pitch);
        if self.flags & D_END != 0 && self.flags & D_SHOWN == 0 {
            self.flags |= D_DEAD;
        }
        let mut rec = None;
        if self.flags & D_PIECE != 0 {
            self.spawn_parts(w, fx, events);
            update_trail_width(&mut self.trail_width, &mut self.trail_time, &self.head_set);
        }
        if self.flags & D_FLYING != 0 && self.flags & (D_END | D_DEAD) == 0 && !w.paused {
            rec = self.add_hit();
        }
        if self.flags & D_DEAD != 0 {
            self.flags &= !D_ALIVE;
        }
        self.timer += 1.0;
        rec
    }

    /// `EftDisc_SpawnParts`: a piece's emitters on node slot 5 (held: not of phase 1; flying: not of phase 0) at
    /// the piece, without node slots.
    fn spawn_parts(&mut self, w: &mut World, fx: &mut Modules, events: u32) {
        let (Some(src), Some(st)) = (self.arg.src, &mut self.emit) else {
            return;
        };
        let a = self.arg;
        update_parts(
            fx,
            w,
            &src,
            st,
            &EmitNodes::default(),
            events,
            self.flags & (D_END | D_DEAD) != 0,
            self.flags & D_REQ200 != 0,
            a.pos,
            a.dir,
            a.scale,
            piece_skip(self.flags),
        );
    }

    /// The model's matrix: its size, the spin, the bank and roll, the pitch and yaw of its direction, its place.
    fn model_mtx(&self, pitch: f32) -> Mtx {
        let a = &self.arg;
        let yaw = eft_wrap(a.dir[0].atan2(a.dir[2]));
        let s = a.scale * self.size * self.grown;
        let mut m = IDENTITY;
        for (i, row) in m.iter_mut().enumerate().take(3) {
            row[i] *= s;
        }
        let m = mul(&m, &rot_y(self.rot_y));
        let m = mul(&m, &rot_x(self.rot_x));
        let m = mul(&m, &rot_z(eft_wrap(self.bank_angle + self.roll)));
        let m = mul(&m, &rot_x(pitch));
        let mut m = mul(&m, &rot_y(yaw));
        for k in 0..3 {
            m[3][k] += a.pos[k];
        }
        m
    }

    /// `EftDisc_AddHit`: two spheres at its position and its last one.
    fn add_hit(&mut self) -> Option<HitRec> {
        let a = &self.arg;
        if self.flags & D_PIECE == 0 {
            let mut atk = self.atk;
            atk.dir = a.dir;
            self.atk = atk;
            return Some(HitRec::blast(
                atk,
                self.id,
                a.pos,
                self.prev,
                a.dir,
                a.scale * 4.5,
            ));
        }
        let src = a.src?;
        if src.def.flags & 1 == 0 {
            return None;
        }
        let mut rec = HitRec::technique_shape(
            src,
            self.id,
            a.pos,
            self.prev,
            a.dir,
            a.pos,
            self.trail_width * 3.0,
            0,
        )?;
        if a.last_hit {
            rec.flags |= 0x10;
        }
        if src.def.max_hits > 0 {
            self.tag.flags |= T_MULTI;
            self.tag.flags &= !T_HIT_CHAR;
        }
        Some(rec)
    }
}

/// The emitters `EftDisc_SpawnParts` leaves out for a piece's flags.
fn piece_skip(flags: u32) -> impl Fn(&EmitSet, u8, usize) -> bool {
    move |set, kind, sub| {
        let Some(g) = set.groups[usize::from(kind)] else {
            return true;
        };
        let p = &set.parts[g.first + sub];
        let held = flags & D_HELD != 0 && p.phase() != 1 && p.node() == 5;
        let flying = flags & D_FLYING != 0 && p.phase() != 0 && p.node() == 5;
        !(held || flying)
    }
}

/// The texture pair of a ki blast's level (1, 2, 4, 6: pairs 0..3; others 0).
fn level_pair(level: u8) -> u8 {
    match level {
        2 => 1,
        4 => 2,
        6 => 3,
        _ => 0,
    }
}

/// `EftMath_WrapAngle`.
fn eft_wrap(a: f32) -> f32 {
    const TWO_PI: f32 = 6.283_185_3;
    const P: f32 = 3.141_592_7;
    if a >= P {
        let n = (((a - P) / TWO_PI) as i32).abs();
        a - (n as f32 * TWO_PI + TWO_PI)
    } else if a <= -P {
        let n = (((a + P) / TWO_PI) as i32).abs();
        a + (n as f32 * TWO_PI + TWO_PI)
    } else {
        a
    }
}

// ---- the multi (eft_shot_tech.c) -------------------------------------------------------------------------------

/// An `EftMulti` item.
#[derive(Clone, Debug)]
pub(crate) struct Multi {
    src: TechSrc,
    /// 1 ending, 2 dead, 4 aborted, 8 END seen, 0x40 started, 0x100 all fired, 0x200 held pieces released one by
    /// one (definition flag 0x8000), 0x400 pieces fired from the muzzle.
    flags: u32,
    state: u8,
    timer: f32,
    life: f32,
    release_scale: f32,
    dir: [f32; 3],
    /// The hand (node 0x15) at the last START and at the FIRE that fires a piece from the muzzle: the piece's roll.
    fire_hand: [f32; 3],
    release_hand: [f32; 3],
    /// Where its emitters are driven (`head`, the muzzle from the FIRE).
    head: [f32; 3],
    nodes: EmitNodes,
    emit: Option<EmitState>,
    /// The pieces: the disc and 1 fired, 2 released.
    pieces: [(Option<u32>, u32); PIECES],
    count: i32,
    set: EmitSet,
    dead: bool,
}

impl Multi {
    /// `EftMulti_Init`.
    pub(crate) fn new(src: TechSrc, data: &SideData, set: &EmitSet) -> Self {
        Self {
            src,
            flags: if src.def.flags & 0x8000 != 0 {
                0x200
            } else {
                0x400
            },
            state: 0,
            timer: 0.0,
            life: f32::from(set.head.end_frames),
            release_scale: 0.0,
            dir: [0.0; 3],
            fire_hand: [0.0; 3],
            release_hand: [0.0; 3],
            head: [0.0; 3],
            nodes: EmitNodes::default(),
            emit: emit_state(data, src.slot),
            pieces: [(None, 0); PIECES],
            count: 0,
            set: set.clone(),
            dead: false,
        }
    }

    pub(crate) fn src(&self) -> &TechSrc {
        &self.src
    }

    pub(crate) fn dead(&self) -> bool {
        self.dead
    }

    /// `EftMulti_Reset`: its emitters' objects die.
    pub(crate) fn kill_parts(&mut self, fx: &mut Modules) {
        if let Some(st) = &mut self.emit {
            st.kill_all(fx);
        }
    }

    /// `EftMulti_PostUpdate`: its emitters (it publishes no record, so the hit results never come).
    pub(crate) fn post_update(&mut self, w: &World, fx: &mut Modules) {
        if stopped(w, self.src.side) {
            return;
        }
        if let Some(st) = &mut self.emit {
            parts_alive(fx, w.data[self.src.side], &self.src, st, false);
        }
    }

    /// `EftMulti_Update`: the aim on START (from the start node), its firing aim on FIRE (from the muzzle), the
    /// pieces; ends once all are fired and gone, on the END, or at once on ABORT; dies once ended and empty.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        w: &mut World,
        fx: &mut Modules,
        events: u32,
        cur_slot: i32,
        discs: &mut Discs,
        next_id: &mut u32,
    ) {
        let side = self.src.side;
        if stopped(w, side) {
            return;
        }
        let def = self.src.def;
        let test = |b: u32| events & b != 0;
        self.nodes
            .update(&w.chars[side], w.data[side], &def, cur_slot, events);
        if self.flags & 0x40 == 0 {
            if test(EV_START) {
                self.state = 0;
                self.flags |= 0x40;
            } else if test(EV_FIRE) {
                self.state = 2;
                self.flags |= 0x40;
            }
        }
        let mut advance = false;
        if self.flags & 0x40 != 0 {
            match self.state {
                0 => {
                    if test(EV_START) {
                        self.dir = aim_dir(w, side, self.nodes.pos(0));
                        self.state = 1;
                    }
                }
                2 => {
                    if test(EV_FIRE) {
                        self.head = self.nodes.pos(1);
                        self.dir = aim_dir_keep(w, side, self.head);
                        self.state = 3;
                    }
                }
                4 => {
                    if test(EV_END) {
                        self.flags |= 9;
                        self.state = 5;
                    }
                }
                1 | 3 | 5 => advance = true,
                _ => {}
            }
        }
        let alive = self.update_pieces(w, events, discs, next_id);
        if !alive && self.flags & 0x100 != 0 {
            self.flags |= 1;
        }
        if test(EV_ABORT) {
            if self.flags & 8 == 0 {
                self.flags |= 4;
            }
            self.flags |= 1;
        }
        // EftMulti_UpdateParts: the emitters on slot 5 are the pieces'.
        if let Some(st) = &mut self.emit {
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
                def.scale,
                |set, kind, sub| {
                    set.groups[usize::from(kind)]
                        .is_some_and(|g| set.parts[g.first + sub].node() == 5)
                },
            );
        }
        if advance {
            self.state += 1;
        }
        if self.flags & 1 != 0 {
            self.timer += 1.0;
        }
        if !alive && self.flags & 2 != 0 {
            self.dead = true;
        } else if self.flags & 1 != 0 && (self.flags & 4 != 0 || self.timer >= self.life) {
            self.flags |= 2;
        }
    }

    /// `EftMulti_UpdatePieces`: START (or event 0x40 alone) fires a piece and FIRE releases the next held one; or,
    /// for pieces fired from the muzzle, FIRE fires one. An ending item kills its pieces. Whether any is alive.
    fn update_pieces(
        &mut self,
        w: &mut World,
        events: u32,
        discs: &mut Discs,
        next_id: &mut u32,
    ) -> bool {
        let (mut fire, mut release) = (false, false);
        if events & EV_START != 0 {
            fire = true;
        } else if events & EV_FIRE != 0 {
            release = true;
        } else if events & 0x30 == 0 {
            fire = events & 0x40 != 0;
        }
        if self.flags & 0x200 != 0 {
            if fire {
                self.fire_piece(w, discs, next_id);
            }
            if release
                && let Some(p) = self
                    .pieces
                    .iter_mut()
                    .find(|p| discs.alive(p.0) && p.1 & 2 == 0)
            {
                // EftDisc_Release: a piece flies along its fighter's aim from where it is, shown at once; then
                // EftDisc_SetScale.
                if let Some(d) = discs.find(p.0) {
                    if d.grown <= 0.0 || d.alpha <= 0.0 {
                        d.grown = 1.0;
                        d.alpha = 127.0;
                        d.flags |= D_SHOWN;
                    }
                    d.arg.dir = aim_dir_keep(w, d.arg.owner, d.arg.pos);
                    d.flags |= D_FLYING;
                    d.flags &= !D_HELD;
                    d.arg.scale = self.release_scale;
                }
                p.1 |= 2;
            }
        } else {
            let hand = w.chars[self.src.side].obj.node_pos(0x15);
            if fire {
                self.fire_hand = hand;
            }
            if release {
                self.release_hand = hand;
                self.fire_piece(w, discs, next_id);
            }
        }
        for p in &self.pieces {
            if let Some(d) = discs.find(p.0) {
                if self.flags & 4 != 0 {
                    d.flags |= D_REQ200;
                }
                if self.flags & 1 != 0 {
                    d.flags |= D_DEAD;
                }
            }
        }
        let mut alive = false;
        for p in &mut self.pieces {
            if discs.alive(p.0) {
                alive = true;
            } else {
                p.0 = None;
            }
        }
        alive
    }

    /// `EftMulti_FirePiece`: the next piece, held in a hand (technique-specific growth and hand) or from the muzzle;
    /// the last one of two or more is the technique's last hit.
    fn fire_piece(&mut self, w: &mut World, discs: &mut Discs, next_id: &mut u32) {
        let Some(k) = self.pieces.iter().position(|p| p.0.is_none()) else {
            return;
        };
        self.pieces[k].1 = 0;
        let side = self.src.side;
        let def = self.src.def;
        let mut arg = DiscArg {
            src: Some(self.src),
            dir: self.dir,
            pos: [0.0; 3],
            scale: def.scale,
            speed: def.speed,
            life: def.life as f32 / 30.0,
            grow: 0.1,
            fade: 0.9,
            turn: def.homing,
            bank: 3.5,
            roll: 0.0,
            roll_time: 0.0,
            tex_a: 0,
            tex_b: 0,
            owner: side,
            kind: 2,
            last_hit: false,
            hand: false,
        };
        if self.flags & 0x200 != 0 {
            let c = self.count;
            match def.id {
                0x157 => {
                    arg.tex_b = 1;
                    arg.grow = if c == 0 { 0.25 } else { 0.04 };
                    arg.hand = if c < 2 { true } else { c & 1 != 0 };
                    self.release_scale = 4.0;
                }
                0x193 => {
                    arg.tex_b = 2;
                    arg.hand = c % 2 == 0;
                    self.release_scale = 1.0;
                }
                0x194 => {
                    arg.tex_b = 3;
                    arg.grow = if c == 0 { 0.2 } else { 0.04 };
                    arg.hand = c % 2 == 0;
                    self.release_scale = 5.0;
                }
                0x188 => {
                    arg.grow = 0.35;
                    arg.hand = c & 1 != 0;
                    self.release_scale = 5.0;
                }
                _ => {}
            }
        } else {
            arg.kind = 4;
            arg.grow = 0.0;
            arg.pos = self.nodes.pos(1);
            // The roll: the hand's way from the START to the FIRE, in the fighter's frame.
            let d = sub(self.fire_hand, self.release_hand);
            let yaw = w.chars[side].pose.rot[1];
            let d = crate::battle::apply(&rot_y(-yaw), [d[0], d[1], d[2], 0.0]);
            let d = crate::btl_blast::norm([d[0], d[1], d[2]]);
            arg.roll = d[0].atan2(-d[1]) / 6.283_185_3;
            match def.id {
                0x1A3 => arg.last_hit = true,
                0x202 => arg.roll = 0.0,
                _ => {}
            }
        }
        let id = discs.create(arg, w.data[side], Some(&self.set), next_id);
        for d in std::mem::take(&mut discs.pending_spin) {
            let r = w.libc.rand();
            discs.set_spin(d, r);
        }
        self.pieces[k].0 = id;
        if id.is_none() {
            return;
        }
        // EftDisc_SetModelTex: the pack's texture set (EftMultiMgr_Init loads it for these two).
        if matches!(def.id, 0x158 | 0x202)
            && let Some(d) = discs.find(id)
        {
            d.pack_tex = true;
            d.arg.tex_a = 0;
            d.arg.tex_b = 0;
            if def.id == 0x202 {
                d.spin = (w.libc.rand() as f32 / 2_147_483_647.0 * 0.01 + 0.13) * -PI;
            }
        }
        self.pieces[k].1 |= 1;
        self.count += 1;
        if self.count >= def.shots {
            if def.shots >= 2
                && let Some(d) = discs.find(id)
            {
                d.arg.last_hit = true;
            }
            self.flags |= 0x100;
        }
    }
}
