//! The roster's phases of a battle frame (src/battle/btl_char_mgr.c), called from `Battle_Loop` /
//! `Battle_Update` in this order: `BtlChars_CheckStart`, `BtlChars_SampleInput`, `BtlChars_UpdateInput`,
//! `BtlChars_UpdateMain`, `BtlChars_PostScene`, `BtlChars_EndFrame`. Every per-fighter step skips a frozen fighter.
//!
//! Not ported in these phases: the stage push-out of the body sphere (`StgCol_UpdateFighter` past its sphere update),
//! the sight test between the fighters (flag 0xBA), holds, partners, member changes, camera cuts, head tracking,
//! effects, sounds and battle events.

use bt3_data::stage_col::{ColBox, PushSphere};

use crate::btl_act::Ctx;
use crate::btl_blast::{Scene, World};
use crate::btl_chr::{
    Chr, Env, SideData, apply_orbit, cam_update_input, clamp_to_stage, length, push_out, scale3,
    sub, update_lock_on,
};
use crate::btl_clash::{Clash, place_on_path};
use crate::btl_hit::Pair;
use crate::btl_input::PadState;
use crate::btl_obj::DEFAULT_STEP;

/// `gBtlChars`.
pub(crate) struct Roster {
    pub chars: [Chr; 2],
    /// `frame`: counts while the fight runs (`BtlChar_FrameMod`).
    pub frame: u32,
    /// `BTL_CHARS_STARTED`.
    pub started: bool,
    /// `randState`: the fighters' generator (`BtlChar_Rand`).
    pub rand: u32,
    /// The ki blasts of the effect scene and its record list.
    pub scene: Scene,
    /// The clash sequences (`clashState`, `clashB`, `clashC`).
    pub clash: Clash,
    cpu: [bool; 2],
}

impl Roster {
    /// `BtlChar_AllocAll` / `BtlChar_ResetAll`.
    pub(crate) fn new(data: [&SideData; 2], env: &Env, cpu: [bool; 2]) -> Self {
        Self {
            chars: [
                Chr::new(0, data[0], env, cpu[0]),
                Chr::new(1, data[1], env, cpu[1]),
            ],
            frame: 0,
            started: false,
            rand: 0,
            scene: Scene::default(),
            clash: Clash::default(),
            cpu,
        }
    }

    /// `BtlChar_ResetAll`: the manager's counters and both fighters from the setup.
    fn reset_all(&mut self, data: [&SideData; 2], env: &Env) {
        let started = self.started;
        let scene = std::mem::take(&mut self.scene);
        let clash = self.clash;
        *self = Self::new(data, env, self.cpu);
        self.started = started;
        self.scene = scene;
        self.clash = clash;
    }

    /// `BtlChars_CheckStart`: the first frame the sequence is in Ready or Fight resets the roster (not in mode 1).
    pub(crate) fn check_start(&mut self, data: [&SideData; 2], env: &Env) -> bool {
        let mut did = false;
        if matches!(env.seq_state, 2 | 3) {
            if !self.started && env.mode != 1 {
                self.reset_all(data, env);
                did = true;
            }
            self.started = true;
        }
        did
    }

    /// `BtlChars_SampleInput` (`BtlInput_Sample`): stage 2, the pad (or the CPU's virtual pad) into the record.
    pub(crate) fn sample_input(&mut self, pads: &[PadState; 2], data: [&SideData; 2], env: &Env) {
        for ((chr, pad), data) in self.chars.iter_mut().zip(pads).zip(data) {
            if chr.is_frozen() {
                continue;
            }
            chr.flags.set_stage(2);
            let class = chr.input.tech_class;
            chr.input.switch = if class < 2 {
                crate::btl_input::SwitchTest::None
            } else if chr.flags.test(0xA2) {
                crate::btl_input::SwitchTest::Release
            } else if chr.flags.test(0xA3) {
                crate::btl_input::SwitchTest::Press
            } else {
                let row = data.tables.tech().prompt_row(class);
                let dir = usize::try_from(row)
                    .ok()
                    .and_then(|r| env.common.prompt_rows.get(r))
                    .map_or(-1, |r| i32::from(r[2] as i8));
                crate::btl_input::SwitchTest::Dir(dir)
            };
            let active = (chr.flags.test(2) || chr.flags.test(3))
                && !chr.flags.test(0x136)
                && !env.gate.time_stopped;
            chr.input.sample(pad, active, env.gate.time_stopped);
        }
    }

    /// `BtlChars_UpdateInput`: frame counter, hit-stop, then per fighter the input and the per-frame clears, the
    /// first snapshot and the sequence flags.
    pub(crate) fn update_input(&mut self, data: [&SideData; 2], env: &Env) {
        if self.started && !env.gate.time_stopped {
            self.frame = (self.frame + 1) & 0x3FFF_FFFF;
        }
        // BtlChars_CheckRoundReset: flag 0xF9 is raised only by the member change, not ported.
        self.update_freeze();
        for chr in &mut self.chars {
            begin_frame(chr, env);
        }
        self.snapshot(0);
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            update_seq_flags(me, opp, data[i], data[i ^ 1], env);
        }
    }

    /// `BtlChars_UpdateFreeze`: the hit-stop requests of this frame and the countdowns.
    fn update_freeze(&mut self) {
        let level: [i32; 2] = std::array::from_fn(|i| {
            let f = &self.chars[i].flags;
            if f.test(0x125) {
                2
            } else if f.test(0x126) {
                1
            } else {
                0
            }
        });
        let max = level[0].max(level[1]);
        if max > 0 {
            for chr in &mut self.chars {
                chr.freeze_next = 1;
                chr.freeze_delay = 0;
            }
            for (i, chr) in self.chars.iter_mut().enumerate() {
                if max == level[i] {
                    chr.freeze_next = 0;
                    chr.freeze_delay = 0;
                    if max >= 2 {
                        break;
                    }
                }
                if chr.flags.test(0x127) && max == 1 {
                    chr.freeze_next = 0;
                    chr.freeze_delay = 0;
                }
            }
        }
        for chr in &mut self.chars {
            if chr.freeze > 0 {
                chr.freeze = (chr.freeze - 1).max(0);
            }
            if chr.freeze_next != 0 {
                if chr.freeze_delay != 0 {
                    chr.freeze_delay -= 1;
                } else {
                    chr.freeze = chr.freeze_next;
                    chr.freeze_next = 0;
                }
            }
        }
    }

    /// `BtlChars_Snapshot`.
    fn snapshot(&mut self, n: usize) {
        for chr in &mut self.chars {
            chr.snapshot(n);
        }
    }

    /// `BtlChars_UpdateMain`: the simulation proper, seven passes over the roster.
    pub(crate) fn update_main(&mut self, data: [&SideData; 2], env: &Env) {
        let frame = self.frame;
        let blasts = &self.scene.records;
        let clash = &self.clash.c;
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            if me.is_frozen() {
                continue;
            }
            // BtlMove_UpdateAction: the rates reset, the action state machine, the blend.
            me.anim_set_sub_mix(0.0);
            me.anim_set_step(DEFAULT_STEP);
            Ctx {
                me,
                opp,
                data: data[i],
                opp_data: data[i ^ 1],
                env,
                frame,
                time_stopped: env.gate.time_stopped,
                blasts,
                clash,
            }
            .update();
            me.anim_apply_blend();
            // Of the seven placement requests (flags 0xF3..0xFD) the rush sequences' (0xF6) and the stage path's
            // come from ported actions.
            me.place_relative(data[i], env);
            place_on_path(me, data[i], env, clash);
            me.pose_to_obj(true);
            me.obj.sample_pose(&data[i].obj);
            me.obj.update_events(&data[i].obj);
            me.update_lean();
            me.obj.calc_matrices(&data[i].obj);
            me.obj_to_pose();
        }
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            if !me.is_frozen() {
                me.flags.set_stage(6);
                mirror_flags(me, opp);
            }
        }
        self.update_hold(data);
        self.snapshot(1);
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            if me.is_frozen() {
                continue;
            }
            me.flags.set_stage(7);
            push_out(me, opp, data[i], data[i ^ 1]);
            apply_orbit(me, opp);
            me.pose_to_obj(false);
            me.obj.calc_matrices(&data[i].obj);
            me.obj_to_pose();
        }
        self.snapshot(2);
        for (i, me) in self.chars.iter_mut().enumerate() {
            if me.is_frozen() {
                continue;
            }
            me.flags.set_stage(8);
            clamp_to_stage(me, data[i], env);
            me.pose_to_obj(false);
            me.obj.calc_matrices(&data[i].obj);
            let mut keep_sphere = true;
            if me.flags.test(0x55) {
                me.obj.warp_body(me.body_warp_pos);
                keep_sphere = false;
            }
            let grow = self.started && !env.gate.time_stopped;
            update_fighter_stage(me, data[i], env, keep_sphere, grow);
            update_ground(me, env);
            me.obj_to_pose();
        }
        self.snapshot(3);
        for (i, me) in self.chars.iter_mut().enumerate() {
            if me.is_frozen() {
                continue;
            }
            me.flags.set_stage(9);
            coll_update_ground(me, data[i].height(), env);
            me.pose_to_obj(false);
            me.obj.calc_matrices(&data[i].obj);
        }
        self.snapshot(4);
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            if me.is_frozen() {
                continue;
            }
            cam_update_input(me, opp, data[i]);
            me.obj.calc_matrices(&data[i].obj);
            // BtlFx_UpdateAll: only the technique shots and the ki blasts it starts are ported.
            self.scene
                .request_technique(me, opp, data[i], env.gate.time_stopped);
            self.scene
                .fire(me, opp, data[i], &mut self.rand, env.gate.time_stopped);
            self.scene.spawn_held_disc(me, data[i]);
            me.obj.build_volumes(&data[i].obj);
        }
        self.update_queued_damage(data, env);
        if !self.chars.iter().any(Chr::is_frozen) {
            let mut pair = Pair {
                chars: &mut self.chars,
                data,
                env,
                frame,
            };
            pair.body_hit_update();
            pair.update();
        } else {
            for chr in &mut self.chars {
                chr.status.target = -1;
            }
        }
        for chr in &mut self.chars {
            if !chr.is_frozen() {
                chr.flags.set_stage(10);
            }
        }
        self.clash
            .update(&mut self.chars, data, env, &mut self.rand, self.frame);
        if let Some(winner) = self.clash.struggle_end.take() {
            self.scene.end_struggle(&mut self.chars, winner);
        }
    }

    /// `BtlChars_UpdateHold`: a held fighter (flag 0x97) and its holder (0x8B) are moved so that the holder's node
    /// 0x34 and the held one's hold node meet: the holder with its flag 0x9B, else the held fighter (flag 0x23); by
    /// what is left of the held fighter's blend while it runs.
    fn update_hold(&mut self, data: [&SideData; 2]) {
        let (mut holder, mut held) = (None, None);
        for (i, chr) in self.chars.iter().enumerate() {
            if chr.is_frozen() {
                continue;
            }
            if chr.flags.test(0x8B) {
                holder = Some(i);
            }
            if chr.flags.test(0x97) {
                held = Some(i);
            }
        }
        let (Some(h), Some(d)) = (holder, held) else {
            return;
        };
        if h == d {
            return;
        }
        let at = self.chars[h].obj.node_pos(0x34);
        let held_node = self.chars[d].obj.hit.node.max(0) as u32;
        let pos = self.chars[d].obj.node_pos(held_node);
        let keep = if self.chars[d]
            .flags
            .test(crate::btl_chr::anim_flag::BLENDING)
        {
            1.0 - self.chars[d].obj.blend
        } else {
            1.0
        };
        let (mover, delta) = if self.chars[h].flags.test(0x9B) {
            (h, sub(pos, at))
        } else {
            (d, sub(at, pos))
        };
        let me = &mut self.chars[mover];
        me.pose.pos = crate::btl_chr::add(me.pose.pos, delta.map(|v| v * keep));
        me.pose_to_obj(false);
        me.obj.calc_matrices(&data[mover].obj);
        me.obj_to_pose();
        if mover == d {
            me.flags.set(0x23);
        }
    }

    /// `BtlMembers_UpdateQueuedDamage`: each fighter deals what it queued against its opponent, a share on each of
    /// the motion's damage events (0x200000000), and drains health / ki past the drain's first frame while the
    /// motion is a throw (table flag 8 without 0x10); flag 0x8C deals the rest and empties the queue. The
    /// vibration is not ported.
    fn update_queued_damage(&mut self, data: [&SideData; 2], env: &Env) {
        for i in 0..2 {
            let (me, other) = split(&mut self.chars, i);
            if me.is_frozen() {
                continue;
            }
            if me.anim_test_attr(0x2_0000_0000) {
                let q = &mut me.queue;
                if q.hits >= 2 {
                    other.damage(env, data[i ^ 1], q.per_hit, q.flags);
                    q.total -= q.per_hit;
                } else {
                    other.damage(env, data[i ^ 1], q.total, q.flags);
                    q.total = 0;
                }
                q.hits -= 1;
                other.unk_fe4 = 30;
                if me.flags.test(0x13B) {
                    other.unk_fe4 = 0;
                }
            }
            let mflags = env.anim_flags(me.anim.cur);
            let (active, frame) = if mflags & 8 != 0 && mflags & 0x10 == 0 {
                (true, me.obj.frame as i32)
            } else {
                (false, 0)
            };
            let q = &mut me.queue;
            if active && q.drain_from < frame {
                if q.drain_health > 0 {
                    let amount = if q.drain_health_step < q.drain_health {
                        q.drain_health -= q.drain_health_step;
                        q.drain_health_step
                    } else {
                        std::mem::take(&mut q.drain_health)
                    };
                    let mut flags = q.flags | 1 | 2;
                    if other.combo.hits > 0 {
                        flags |= 8;
                    }
                    let lost = other.damage(env, data[i ^ 1], amount, flags);
                    if !me.flags.test(0x13C) {
                        me.add_health(lost);
                    }
                    other.unk_fe4 = 30;
                }
                let q = &mut me.queue;
                if q.drain_ki > 0 {
                    let amount = if q.drain_ki_step < q.drain_ki {
                        q.drain_ki -= q.drain_ki_step;
                        q.drain_ki_step
                    } else {
                        std::mem::take(&mut q.drain_ki)
                    };
                    let lost = other.drain_ki(amount);
                    if !me.flags.test(0x13C) {
                        me.add_ki(lost);
                    }
                    other.unk_fe4 = 30;
                }
            }
            if me.flags.test(0x8C) {
                if me.queue.total > 0 {
                    other.damage(env, data[i ^ 1], me.queue.total, me.queue.flags);
                }
                me.queue = crate::btl_chr::DamageQueue {
                    drain_to: me.queue.drain_to,
                    ..Default::default()
                };
            }
        }
    }

    /// `BtlScene_Update`, `EftDet_Update` and `BtlScene_PostUpdate` (`Battle_Update`, after the main passes).
    pub(crate) fn update_scene(
        &mut self,
        data: [&SideData; 2],
        env: &Env,
        libc: &mut crate::rng::LibcRandom,
        vu0: &mut crate::rng::Vu0Random,
        paused: bool,
    ) {
        let mut world = World {
            chars: &mut self.chars,
            data,
            env,
            rand: &mut self.rand,
            time_stopped: env.gate.time_stopped,
            libc,
            vu0,
            paused,
            frame: self.frame,
            clash_bias: self.clash.a.bias,
        };
        self.scene.update(&mut world);
    }

    /// `BtlChars_PostScene`: the fighter camera, the low-health state.
    pub(crate) fn post_scene(&mut self, data: [&SideData; 2], env: &Env) {
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            if me.is_frozen() {
                continue;
            }
            me.obj_to_pose();
            cam_update(me, opp, data[i], data[i ^ 1], env);
            let g = &mut me.gauge;
            if g.health < 10000 {
                g.low_health = true;
                g.low_health_idle = data[i].tables.params.chara_flags() & 0x80 == 0
                    && me.skill_timer_c <= 0
                    && me.skill_timer_d <= 0
                    && !me.flags.test(6);
            } else {
                g.low_health_idle = false;
                g.low_health = false;
            }
        }
    }

    /// `BtlChars_EndFrame`: stages 11 and 12, the frame's movement vectors.
    pub(crate) fn end_frame(&mut self) {
        for chr in &mut self.chars {
            if chr.is_frozen() {
                continue;
            }
            chr.flags.set_stage(11);
            if !chr.flags.test(0x24) {
                chr.pose.vel = chr.snap_delta(0, 1);
                chr.pose.moved = crate::btl_chr::sub(chr.pose.pos, chr.snap_pos(0));
            } else {
                chr.pose.vel = [0.0; 3];
                chr.pose.moved = [0.0; 3];
            }
            chr.flags.set_stage(12);
        }
        self.snapshot(5);
    }
}

fn split(chars: &mut [Chr; 2], i: usize) -> (&mut Chr, &mut Chr) {
    let (lo, hi) = chars.split_at_mut(1);
    if i == 0 {
        (&mut lo[0], &mut hi[0])
    } else {
        (&mut hi[0], &mut lo[0])
    }
}

/// `BtlChar_BeginFrame`: stage 3, last frame's values kept, the per-frame state cleared, the input consumed.
fn begin_frame(chr: &mut Chr, env: &Env) {
    if chr.is_frozen() {
        return;
    }
    chr.flags.set_stage(3);
    chr.obj_to_pose();
    chr.pose.lean_x = 0.0;
    chr.pose.lean_z = 0.0;
    chr.snap_mask = 0;
    chr.prev_action = chr.act.current;
    chr.prev_action_frame = chr.action_frame;
    chr.anim.prev = chr.anim.cur;
    chr.obj.rate = 1.0;
    chr.anim_rate = 1.0;
    chr.combo.new_hit = false;
    chr.combo.changed = false;
    chr.act_bits = 0;
    chr.frames_left_override = -1;
    chr.fx_bits = [0; 0x10];
    chr.dash_limit = 1;
    chr.vanish_limit = 1;
    chr.dir_held = -1;
    chr.input.tech_class = -1;
    let blocked = chr.flags.test(0x11E) || chr.flags.test(0x11F) || chr.flags.test(0xB1);
    chr.input.update(&env.gate, blocked);
}

/// `BtlChar_UpdateSeqFlags`: held flag 1 (intro), 2 (ready), 3 (fight) or 4 (after) and the lock-on.
fn update_seq_flags(me: &mut Chr, opp: &Chr, data: &SideData, opp_data: &SideData, env: &Env) {
    if me.is_frozen() {
        return;
    }
    for f in 1..=4 {
        me.flags.clear(f);
    }
    match env.seq_state {
        0 | 1 => me.flags.set_held(1),
        2 => me.flags.set_held(2),
        3 => me.flags.set_held(3),
        4..=6 => me.flags.set_held(4),
        _ => {}
    }
    update_lock_on(me, opp, data, opp_data);
    if env.gate.time_stopped {
        me.flags.set_held(0x135);
    } else {
        me.flags.clear(0x135);
    }
    if !me.flags.test(0xAF) {
        me.flags.clear(0x80);
    }
}

/// `StgCol_UpdateFighter`: the body sphere swept from last frame's position to this frame's in steps of at most
/// half its radius, pushed out of the stage after every step; when pushed, the object moves back by the
/// correction and the contact word gets bit 0x20. No object breaks yet (`StgCol_FighterBreakObj`).
fn update_fighter_stage(me: &mut Chr, data: &SideData, env: &Env, keep_sphere: bool, grow: bool) {
    // BtlObjBody_BeginFrame.
    me.obj.work.hit_flags = 0;
    me.obj.swap_body_sphere();
    if keep_sphere {
        me.obj.update_body_sphere();
    } else {
        let (cur, r) = (me.obj.body.cur, me.obj.body.radius);
        me.obj.body.sphere[cur].1 = r;
    }
    let body = &mut me.obj.body;
    if grow {
        body.cur_radius = (body.cur_radius + 1.0).min(body.radius);
    }
    if body.cur_radius < 4.0 {
        body.cur_radius = 4.0;
    }
    let r = body.cur_radius;
    body.sphere[body.prev].1 = r;
    body.sphere[body.cur].1 = r;
    let (cur, prev) = (body.sphere[body.cur], body.sphere[body.prev]);
    let area = me.obj.area;
    let zone = env.col.find_zone_near(area, cur.0) as i32;
    let (start, delta) = if body.reset {
        body.reset = false;
        (cur, [0.0; 3])
    } else {
        (prev, sub(cur.0, prev.0))
    };
    let dist = length(delta);
    // StgCol_SplitStep: steps of at most half a radius.
    let n = (dist / (start.1 * 0.5)) as i32;
    let (steps, step) = if n == 0 {
        (1, delta)
    } else {
        (n + 1, scale3(delta, 1.0 / (n + 1) as f32))
    };
    let mut sphere = PushSphere {
        center: start.0,
        radius: start.1,
        bounds: ColBox::from_center_half(start.0, [0.0; 3]),
        hit: false,
        poly_flags: 0,
        poly_nrm: [0.0; 4],
    };
    // StgCol_MoveFighter: the zone is looked up again every step when the fighter changed zones.
    let cross = zone != area;
    let mut z = area;
    for _ in 0..steps {
        sphere.step(step);
        if cross {
            z = env.col.find_zone_near(z, sphere.center) as i32;
        }
        if let Ok(zi) = usize::try_from(z) {
            env.col.push_sphere(zi, &mut sphere);
        }
    }
    if sphere.hit {
        me.obj.work.hit_flags |= 0x20;
        me.obj.work.contact_flags = sphere.poly_flags;
        me.obj.work.contact_nrm = sphere.poly_nrm;
        let cur_center = me.obj.body.sphere[me.obj.body.cur].0;
        let push = sub(cur_center, sphere.center);
        me.obj.body.push = push;
        me.obj.pos = sub(me.obj.pos, push);
        me.obj.xf_update();
        me.obj.calc_matrices(&data.obj);
        if keep_sphere {
            me.obj.update_body_sphere();
        } else {
            let c = me.obj.body.cur;
            me.obj.body.sphere[c].0 = sphere.center;
        }
    } else {
        me.obj.body.push = [0.0; 3];
    }
    me.obj.area = zone;
}

/// `BtlOpp_MirrorFlags` (`BtlChar_UpdateStage6`): what the opponent is doing, as this fighter's flags 0xAC..0xB9
/// and 0xD6 (in a technique: 0xAF; dead: 0xB1). The camera cut its flag 0x27 ends is not ported.
fn mirror_flags(me: &mut Chr, opp: &Chr) {
    me.flags.clear_range(0xAC, 0xB9);
    for (o, m) in [(0xB, 0xAC), (0x1A, 0xAD), (0x3F, 0xAE), (0x85, 0xB3)] {
        if opp.flags.test(o) {
            me.flags.set_held(m);
        }
    }
    if crate::btl_act::is_technique_id(opp.act.current) {
        me.flags.set_held(0xAF);
    }
    if (0xFD..0x103).contains(&opp.act.current) {
        me.flags.set_held(0xB0);
    }
    if opp.is_dead() {
        me.flags.set_held(0xB1);
    }
    if opp.flags.test(0xCC) {
        me.flags.set_held(0xB4);
    }
    if opp.flags.test(0x26) {
        me.flags.set_held(0xB2);
    }
    for (o, m) in [
        (0x90, 0xB5),
        (0x16, 0xB6),
        (0x17, 0xB7),
        (0x28, 0xD6),
        (0xDC, 0xB8),
        (0x92, 0xB9),
    ] {
        if opp.flags.test(o) {
            me.flags.set(m);
        }
    }
}

/// `StgGround_UpdateFighter`: the ground under the object's position, below the body sphere's centre.
fn update_ground(me: &mut Chr, env: &Env) {
    let (center, _) = me.obj.body_sphere();
    let pos = [me.obj.pos[0], center[1], me.obj.pos[2]];
    let b = ColBox::from_center_half(pos, [0.1, 1.0, 0.1]);
    let g = env
        .col
        .ground_probe(me.obj.area, &b, pos[0], pos[2], center[1]);
    me.obj.work.ground_pos = [me.obj.pos[0], g.y, me.obj.pos[2]];
    me.obj.work.ground = Some(g);
}

/// `BtlColl_UpdateGround`: the pose takes the ground found; on it (or landing on it) flag 0xF, else not.
fn coll_update_ground(me: &mut Chr, height: f32, env: &Env) {
    let vel = me.snap_delta(0, 1);
    if let Some(g) = me.obj.work.ground
        && g.y <= 100_000.0
    {
        me.pose.ground_pos = me.obj.work.ground_pos;
        me.pose.ground_nrm = g.nrm;
        me.pose.ground_flags = g.flags;
    }
    let mut margin = vel[0].hypot(vel[2]) + 1.0;
    if !me.flags.test(0xF) {
        margin = 0.0;
    }
    if env.stage_4_or_27 && me.pose.ground_flags & 0x80 == 0 {
        margin = 0.0;
    }
    if me.flags.test(0xE) || me.flags.test(0x58) || me.flags.test(0x24) {
        margin = 0.0;
    }
    let mut landed = false;
    let floor = me.pose.ground_pos[1] - me.pose.disp[1];
    if floor - margin < me.pose.pos[1] {
        me.pose.pos[1] = floor;
        if vel[1] > 0.0 {
            me.pose.fall_speed = 0.0;
            landed = true;
        }
    }
    if me.flags.test(0x10) {
        landed = true;
        me.pose.fall_speed = 0.0;
        me.pose.pos[1] = me.pose.ground_pos[1] - me.pose.disp[1];
    }
    let hit = me.obj.work.hit_flags;
    if hit & 0x40 != 0 {
        me.flags.set(0x20);
        me.flags.clear(0xF);
    } else if landed {
        me.flags.set_held(0xF);
    } else {
        me.flags.clear(0xF);
    }
    me.flags.clear(0x11);
    me.flags.clear(0x12);
    if let Some(water) = env.stage.water_level
        && water < me.pose.ground_pos[1]
    {
        let size = height;
        if water < me.pose.pos[1] - size * 0.5 {
            me.flags.set_held(0x11);
        }
        let lo = me.pose.pos[1] - size;
        let hi = me.pose.pos[1] + me.pose.speed * 30.0 * 0.02;
        if lo < water && water < hi {
            me.flags.set_held(0x12);
        }
    }
    if me.flags.raised(0x11) {
        me.pose.fall_speed *= 0.5;
    }
    if hit & 0x20 != 0 {
        me.flags.set_held(0x1F);
    } else {
        me.flags.clear(0x1F);
    }
}

/// `ChrCam_Update` with the lock-on and free cameras, smoothed by `ChrCam_GetRate`. No shake, no stage trace, no
/// camera cuts.
fn cam_update(me: &mut Chr, opp: &Chr, data: &SideData, opp_data: &SideData, env: &Env) {
    let height = data.height();
    let (mut eye, rot, mut target) = if me.flags.test(5) {
        crate::battle::lock_on_camera(&crate::battle::LockOn {
            pos: me.pose.pos,
            height,
            side_slope: data.cam_side_slope(),
            opp_pos: opp.snap_pos(6),
            opp_height: opp_data.height(),
            stage_bottom: env.stage.bounds.bottom,
            yaw: me.cam.yaw,
            side: me.cam.side,
        })
    } else {
        // ChrCam_CalcFree.
        let (ofs, pitch) = crate::battle::camera_offset(height);
        let look = [0.0, -(height * 0.7), 0.0];
        let p = me.pose.pos;
        let (s, c) = me.cam.yaw.sin_cos();
        (
            [
                p[0] + s * (ofs[2] - look[2]) + look[2],
                p[1] + (ofs[1] - look[1]) + look[1],
                p[2] + c * (ofs[2] - look[2]) + look[2],
            ],
            [pitch, me.cam.yaw, 0.0],
            [look[0] + p[0], look[1] + p[1], look[2] + p[2]],
        )
    };
    let limit = env.stage.bounds.radius;
    for point in [&mut eye, &mut target] {
        let len = point[0].hypot(point[2]);
        if limit < len {
            let k = limit / len;
            point[0] *= k;
            point[2] *= k;
        }
    }
    let smooth = !me.flags.test(0xCD) && !me.flags.test_prev(0xCE);
    // ChrCam_CalcRate / ChrCam_GetRate.
    let calc_rate = (length(crate::btl_chr::sub(target, eye)) / height * 0.2 + 0.2).min(1.0);
    me.cam.rate = crate::btl_chr::approach(me.cam.rate, calc_rate, 0.4 / 30.0);
    if me.flags.test(0xCF) {
        me.cam.rate = 0.3;
    } else if me.flags.test(0xD0) {
        me.cam.rate = 0.25;
    } else if me.flags.test(0xD1) {
        me.cam.rate = 0.2;
    }
    let rate = me.cam.rate;
    if smooth {
        me.cam.eye = std::array::from_fn(|k| eye[k] * rate + me.cam.eye[k] * (1.0 - rate));
    } else {
        me.cam.eye = eye;
        me.cam.rate = calc_rate;
    }
    me.cam.target = target;
    // ChrCam_KeepBehindHead.
    let head = me.obj.node_pos(0x30);
    crate::battle::keep_behind_head(head, height, &mut me.cam.eye, rot);
    if smooth {
        let d: [f32; 3] =
            std::array::from_fn(|k| crate::btl_input::wrap_angle(rot[k] - me.cam.rot[k]));
        me.cam.rot =
            std::array::from_fn(|k| crate::btl_input::wrap_angle(me.cam.rot[k] + d[k] * rate));
    } else {
        me.cam.rot = rot;
    }
}
