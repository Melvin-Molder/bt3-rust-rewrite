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
use crate::rng::LibcRandom;

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
    /// The character change requests (`change`).
    pub change: crate::btl_change::ChangeQueue,
    cpu: [bool; 2],
    /// `gPad[].vibPower` / `vibSmall`: this frame's vibration requests by pad.
    pub pad_vib: [crate::btl_vib::PadVib; 2],
}

impl Roster {
    /// `BtlChar_AllocAll` / `BtlChar_ResetAll`.
    pub(crate) fn new(data: [&SideData; 2], env: &Env, cpu: [bool; 2]) -> Self {
        let mut roster = Self {
            chars: [
                Chr::new(0, data[0], env, cpu[0]),
                Chr::new(1, data[1], env, cpu[1]),
            ],
            frame: 0,
            started: false,
            rand: 0,
            scene: Scene::default(),
            clash: Clash::default(),
            change: Default::default(),
            cpu,
            pad_vib: Default::default(),
        };
        roster.sync_abilities();
        roster
    }

    /// Each fighter's copy of the opponent's active abilities (`BtlOpp_HasAbility`); they change only with the
    /// active member (a switch or a fusion).
    pub(crate) fn sync_abilities(&mut self) {
        let bits = [self.chars[0].ability_bits(), self.chars[1].ability_bits()];
        self.chars[0].opp_ability = bits[1];
        self.chars[1].opp_ability = bits[0];
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
    pub(crate) fn sample_input(
        &mut self,
        pads: &[PadState; 2],
        data: [&SideData; 2],
        env: &Env,
        replay: &mut crate::btl_replay::Replay,
    ) {
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
            let player = chr.player;
            chr.input.sample(
                pad,
                active,
                env.gate.time_stopped,
                Some((&mut *replay, player)),
            );
        }
    }

    /// `BtlChars_UpdateInput`: frame counter, hit-stop, then per fighter the input and the per-frame clears, the
    /// first snapshot and the sequence flags.
    pub(crate) fn update_input(&mut self, data: [&SideData; 2], env: &Env) {
        if self.started && !env.gate.time_stopped {
            self.frame = (self.frame + 1) & 0x3FFF_FFFF;
        }
        self.check_round_reset(data, env);
        self.update_freeze();
        // BtlCharSnd_ClearRequests.
        env.snd().clear();
        for (chr, data) in self.chars.iter_mut().zip(data) {
            begin_frame(chr, data, env);
        }
        self.snapshot(0);
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            update_seq_flags(me, opp, data[i], data[i ^ 1], env);
        }
    }

    /// `BtlChars_CheckRoundReset`: a fighter's flag 0xF9 (the arriving member stands) puts both back at their start
    /// placements in action 0xF9 (`BtlChar_ResetRound`, which releases a partner) and resets the effect scene.
    fn check_round_reset(&mut self, data: [&SideData; 2], env: &Env) {
        if !self.chars.iter().any(|c| c.flags.test(0xF9)) {
            return;
        }
        for (i, chr) in self.chars.iter_mut().enumerate() {
            crate::btl_partner::release(chr);
            chr.flags.clear(0xE);
            chr.flags.set_held(0xF);
            chr.flags.set_held(5);
            chr.flags.clear(0x125);
            crate::btl_head::reset_look(chr, data[i]);
            chr.place_at_start(data[i], env);
            chr.prev_action = chr.act.current;
            chr.act.current = 0xF9;
            chr.act.request = -1;
            chr.act.queue = [-1; 4];
            chr.react.reaction = 1;
            chr.search_angle = 0.5;
            chr.search_range = 500.0;
            chr.freeze_next = 0;
            chr.freeze_delay = 0;
            chr.freeze = 0;
            if env.mode == 3 && i == 0 {
                chr.add_health(10000);
            }
        }
        // BtlScene_Reset(0x2000): every fighter's effects.
        for side in 0..2 {
            self.scene.reset_side(&mut self.chars, side);
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
    pub(crate) fn update_main(
        &mut self,
        data: [&SideData; 2],
        env: &Env,
        libc: &mut crate::rng::LibcRandom,
        vu0: &mut crate::rng::Vu0Random,
    ) {
        let frame = self.frame;
        let blasts = &self.scene.records;
        let clash = &self.clash.c;
        let change = &mut self.change;
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
                change: &mut *change,
                libc: &mut *libc,
            }
            .update();
            me.anim_apply_blend();
            // BtlObj_SaveNodePositions: relative to node 0's translation with flag 0x2B.
            me.obj.save_node_positions(me.flags.test(0x2B));
            // The placement requests in the original's order (0xF3, the restart's, is not ported).
            me.place_center(data[i], env);
            me.place_relative(data[i], env);
            place_on_path(me, data[i], env, clash);
            me.place_center_high(data[i], env);
            me.place_saved(data[i]);
            me.place_warp(data[i], env);
            me.pose_to_obj(true);
            me.obj.sample_pose(&data[i].obj);
            me.obj.update_events(&data[i].obj);
            me.update_lean();
            crate::btl_fx::face_events(me);
            me.obj.calc_matrices(&data[i].obj);
            me.obj_to_pose();
            crate::btl_head::update_look_offset(me, data[i], env.gate.time_stopped);
        }
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            if !me.is_frozen() {
                me.flags.set_stage(6);
                mirror_flags(me, opp);
                let wind = env.stage.wind.unwrap_or([0.0; 4]);
                crate::btl_partner::update(me, data[i].partner, wind);
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
            // BtlCharApi_IsChanging: no growth during a character change or the struggle's loss.
            let changing = matches!(me.act.current, 0xEC..=0xF8 | 0x103 | 0x104);
            let grow = self.started && !env.gate.time_stopped && !changing;
            update_fighter_stage(me, data[i], env, keep_sphere, grow, &mut *libc);
            update_ground(me, data[i], env, &mut *libc);
            me.obj_to_pose();
        }
        self.snapshot(3);
        for (i, me) in self.chars.iter_mut().enumerate() {
            if me.is_frozen() {
                continue;
            }
            me.flags.set_stage(9);
            coll_update_ground(me, data[i], env);
            me.pose_to_obj(false);
            me.obj.calc_matrices(&data[i].obj);
        }
        self.snapshot(4);
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            if me.is_frozen() {
                continue;
            }
            // BtlChar_UpdateCamera: ChrCam_StartCut, then the controls (ChrCam_UpdateDemo is the battle's).
            crate::chr_cam::start_cut(me, opp, data[i], data[i ^ 1], env);
            cam_update_input(me, opp, data[i]);
            // BtlObj_SetMoveVec, BtlObj_UpdateChains (the stage's wind), then the matrices again.
            me.obj.chains.move_vec = me.pose.vel;
            let wind = env.stage.wind.unwrap_or([0.0; 4]);
            me.obj.update_chains(data[i].tables.chain.as_ref(), wind);
            crate::btl_head::update_head(me, opp, data[i]);
            me.obj.calc_matrices(&data[i].obj);
            // BtlFx_UpdateAll, in its order: the aura, then the technique shots and the ki blasts it starts.
            crate::btl_fx::update_aura(me, data[i], env, &mut self.scene.auras);
            self.scene
                .request_technique(me, opp, data[i], env.gate.time_stopped);
            self.scene.fire(
                me,
                opp,
                data[i],
                &mut self.rand,
                env.gate.time_stopped,
                &mut env.snd(),
                self.frame,
            );
            crate::btl_snd::fx_event_sounds(me, opp, data[i], env, self.frame, libc);
            crate::btl_fx::update_all(me, data[i], env, &mut self.scene.impacts);
            let scene = &mut self.scene;
            crate::btl_fx::power_up_look(
                me,
                data[i],
                env,
                &mut scene.glows,
                &mut scene.auras,
                libc,
            );
            crate::btl_fx::screen_filter(me, &mut self.scene.blurs);
            crate::btl_fx::trigger_obj_1e_1f(me);
            crate::btl_fx::update_eyes_flag137(me);
            crate::btl_fx::update_mouth_flag96(me);
            crate::btl_fx::update_obj_flag80(me, data[i]);
            crate::btl_fx::push_back_on_event_400(me, data[i]);
            crate::btl_fx::obj_mask_event_400(me, data[i]);
            crate::btl_fx::shake_and_vibrate_on_events(me);
            self.scene.spawn_held_disc(me, data[i]);
            // BtlFx_UpdateChargeFx: the charge aura while request 7 holds, its burst on request 8.
            if let Some(set) = crate::btl_blast::charge_set(data[i]) {
                let c = &mut self.scene.charges[i];
                if me.test_fx(7) {
                    if !c.is_active() {
                        c.start(set, data[i].height(), me.obj.node_pos(3));
                    }
                } else if c.is_active() {
                    c.stop();
                }
                if me.test_fx(8) {
                    c.burst();
                }
            }
            transform_fx(me, data[i], env, &mut self.scene.transforms);
            movement_fx(me, opp, data[i], &mut self.scene, libc);
            ground_fx(me, data[i], env, &mut self.scene.gdust, libc, vu0);
            water_fx(
                me,
                data[i],
                env,
                self.scene.stage.water.as_mut(),
                self.scene.stage.bubbles.as_mut(),
                libc,
            );
            rush_burst_fx(me, data[i], &mut self.scene.rush_bursts[i]);
            // BtlFx_UpdateReq38: the drain glow while the drain technique holds request 0x38.
            if let Some(set) = crate::btl_blast::absorb_set(data[i]) {
                let node_pos = |n| me.obj.node_pos(n);
                let c = crate::eft_absorb::AbsorbChr {
                    node_pos: &node_pos,
                    event_400: me.anim_test_attr(0x400),
                };
                if me.fx_new(0x38) {
                    self.scene.absorbs[i].start(set, &c);
                }
                if me.fx_ended(0x38) {
                    self.scene.absorbs[i].stop();
                }
            }
            me.obj.build_volumes(&data[i].obj);
        }
        // BtlFx_UpdateRoster: requests 0x10 / 0x11 become 0x12, whose start and end (by any fighter) darken the
        // stage and bring it back (the stage tint's slot 0).
        for i in 0..2 {
            crate::btl_fx::stage_fx_request(&mut self.chars[i], data[i]);
        }
        let now = self.chars.iter().any(|c| c.test_fx(0x12));
        let prev = self.chars.iter().any(|c| c.test_prev_fx(0x12));
        if now && !prev {
            self.scene.tints.start(0, 0, 0.2);
        }
        if !now && prev {
            self.scene.tints.start(0, 1, 0.2);
        }
        self.update_queued_damage(data, env);
        if !self.chars.iter().any(Chr::is_frozen) {
            let mut pair = Pair {
                chars: &mut self.chars,
                data,
                env,
                frame,
                libc: &mut *libc,
            };
            pair.body_hit_update();
            pair.update();
        } else {
            for chr in &mut self.chars {
                chr.status.target = -1;
            }
        }
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            if me.is_frozen() {
                continue;
            }
            me.flags.set_stage(10);
            crate::btl_partner::update_events(
                me,
                opp,
                data[i].partner,
                env,
                frame,
                &mut self.scene.impacts,
                &mut self.scene.shotfx,
                libc,
            );
            crate::btl_snd::after_hit_sounds(me, data[i], env, frame);
            crate::btl_fx::trigger_obj_1d(me);
            let impacts = &mut self.scene.impacts;
            if let Some(pos) =
                crate::btl_fx::after_hits(me, opp, data[i], data[i ^ 1], env, impacts)
            {
                let [a, b] = &mut self.chars;
                crate::chr_cam::shake_cams_near([a, b], pos, 200.0, 1500.0, 6.0, 0.2);
            }
        }
        let powers = self.scene.struggle.as_ref().map(|s| s.powers());
        self.clash.update(
            &mut self.chars,
            data,
            env,
            &mut self.rand,
            self.frame,
            powers,
            libc,
        );
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
    /// motion is a throw (table flag 8 without 0x10); flag 0x8C deals the rest and empties the queue. Both pads
    /// vibrate on each share.
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
                    me.vibrate(1.0, 0.1);
                    other.vibrate(1.0, 0.1);
                } else {
                    other.damage(env, data[i ^ 1], q.total, q.flags);
                    q.total = 0;
                    me.vibrate(1.0, 0.3);
                    other.vibrate(1.0, 0.3);
                }
                let q = &mut me.queue;
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
    pub(crate) fn post_scene(&mut self, data: [&SideData; 2], env: &Env, libc: &mut LibcRandom) {
        // The shakes the stage objects broken this frame asked for (`BtlStage_BreakObj` shakes at once; the
        // cameras only read them here).
        let shakes = std::mem::take(&mut env.objs().shakes);
        for s in shakes {
            let [a, b] = &mut self.chars;
            crate::chr_cam::shake_cams_near([a, b], s.pos, s.near, s.far, s.strength, s.time);
            if let Some((far, power, time)) = s.rumble {
                let [a, b] = &mut self.chars;
                crate::btl_vib::rumble_near([a, b], s.pos, s.near, far, power, time);
            }
        }
        for i in 0..2 {
            let (me, opp) = split(&mut self.chars, i);
            if me.is_frozen() {
                continue;
            }
            me.obj_to_pose();
            cam_update(me, opp, data[i], data[i ^ 1], env, libc);
            // BtlFx_SpawnReq37: the glow at both hands once a blast was absorbed.
            if me.test_fx(0x37)
                && let Some(set) = crate::btl_blast::absorb_set(data[i])
            {
                let node_pos = |n| me.obj.node_pos(n);
                let c = crate::eft_absorb::AbsorbChr {
                    node_pos: &node_pos,
                    event_400: false,
                };
                self.scene.absorbs[i].start_hands(set, &c);
            }
            crate::btl_fx::post_scene(me, data[i], env, &mut self.scene.impacts);
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

    /// `BtlChars_EndFrame`: the late updates (`BtlChar_UpdateLate`: the voices and technique sounds, this
    /// frame's stat modifiers; the effects are not ported), stages 11 and 12, the frame's movement vectors.
    pub(crate) fn end_frame(
        &mut self,
        data: [&SideData; 2],
        env: &Env,
        libc: &mut crate::rng::LibcRandom,
        replay: bool,
    ) {
        for i in 0..2 {
            let chr = &mut self.chars[i];
            if !chr.is_frozen() {
                chr.late_voice(data[i], data[i ^ 1], env, self.frame, libc);
                chr.set_stat_frame_mods();
            }
        }
        for i in 0..2 {
            let opp_alive = self.chars[i ^ 1].count_alive();
            let chr = &mut self.chars[i];
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
            chr.raise_events(data[i], env, opp_alive);
            // BtlChar_UpdateVibration to the fighter's pad.
            chr.update_vibration(&mut self.pad_vib[chr.player], replay);
            chr.flags.set_stage(12);
        }
        self.snapshot(5);
    }
}

/// `BtlChar_SpawnFxBits0`: requests 0..2 start, flash and end the transformation effect of the fighter's change
/// kind (`form.kind`: 0 none; 1 -> 1, 2 -> 0, 3 -> 5, 4 -> 6, 5..7 -> 2..4, 8..11 -> 7..10).
pub(crate) fn transform_fx(
    me: &Chr,
    data: &SideData,
    env: &Env,
    fx: &mut crate::eft_transform::Transforms,
) {
    let form = me.form.kind;
    if me.test_fx(0) {
        let kind = match form {
            1 => 1,
            2 => 0,
            3 => 5,
            4 => 6,
            5..=7 => form - 3,
            8..=11 => form - 1,
            _ => -1,
        };
        if kind >= 0 && !env.transform.is_empty() {
            let node_pos = |n| me.obj.node_pos(n);
            let none = |_| [0.0; 3];
            let no = |_| false;
            let zero = |_| 0;
            let c = crate::eft_transform::TransformChr {
                side: me.player,
                height: data.height(),
                node_pos: &node_pos,
                node_z: &none,
                attr: &no,
                attr_node: &zero,
            };
            fx.start(env.transform, kind, &c);
        }
    }
    if me.test_fx(1) && form != 0 {
        fx.flash();
    }
    if me.test_fx(2) && form != 0 {
        fx.end();
    }
}

/// `BtlFx_UpdateAimedFxReq28`: the rush-finish burst at the burst node along attack 0x86's launch angles (request
/// 0x28 begins), its second stage (0x29), its end (0x2A); for a character whose pack makes the manager.
pub(crate) fn rush_burst_fx(
    me: &Chr,
    data: &SideData,
    bursts: &mut crate::eft_rushburst::RushBursts,
) {
    use crate::btl_tech_volley::{mathf_cos, mathf_sin};
    let Some(set) = crate::btl_blast::rush_burst_set(data) else {
        return;
    };
    if me.fx_new(0x28) {
        let pitch = crate::btl_hit::launch_angle_b_of(data, 0x86);
        let yaw = crate::btl_input::wrap_angle(
            me.pose.yaw + crate::btl_hit::launch_angle_a_of(data, 0x86),
        );
        let dir = [
            mathf_cos(pitch) * mathf_sin(yaw),
            -mathf_sin(pitch),
            mathf_cos(pitch) * mathf_cos(yaw),
        ];
        let node = me.burst_node as u32;
        bursts.start(set, node, me.obj.node_pos(node), dir);
    }
    if me.fx_new(0x29) {
        bursts.stage2();
    }
    if me.fx_new(0x2A) {
        bursts.stop();
    }
}

/// `BtlFxObj_HandleFlagEvents`' event bit 42 and `BtlFx_SpawnBurstReqC`'s requests 0xC..0xF start the vanish
/// lines (kind 0, and kinds 0..3). `BtlFx_SpawnPartFxEvent8`, `BtlFx_SpawnFastMoveFx` and `BtlFx_SpawnSpeedLines`:
/// animation event 8 sends
/// streaks from each model part its value names, along the part's movement turned into the world; request 0xB,
/// moving faster than half the reference speed, leaves trails behind the body; request 0x15 starts the screen's
/// speed lines (a mode 2 ray burst), longer lived the farther the opponent is off the camera's heading, their
/// inner end farther in with the distance.
pub(crate) fn movement_fx(
    me: &Chr,
    opp: &Chr,
    data: &SideData,
    scene: &mut crate::btl_blast::Scene,
    libc: &mut crate::rng::LibcRandom,
) {
    let side = me.player;
    // BtlFx_SpawnReq6: the shock.
    if me.test_fx(6) && crate::btl_blast::shock_pack(data).is_some() {
        scene.shocks[side].start();
    }
    if me.obj.test_event(0x400_0000_0000) {
        scene.shotfx.start(side, 0);
    }
    for (bit, kind) in [(0xC, 0), (0xD, 1), (0xE, 2), (0xF, 3)] {
        if me.test_fx(bit) {
            scene.shotfx.start(side, kind);
        }
    }
    let height = data.height();
    if me.anim_test_attr(8) {
        let bits = me.obj.event_arg(8);
        for i in 0..0x13 {
            let bit = bits & (1 << i);
            if bit == 0 {
                continue;
            }
            let part = crate::btl_obj::mask_to_node(bit) as u32;
            let v = me.obj.node_velocity(part);
            let d = crate::battle::apply(&me.obj.mtx, [v[0], v[1], v[2], 0.0]);
            // BtlCharApi_GetNodeBoundSize: 1 for a node without a record.
            let size = data
                .obj
                .model
                .parts
                .iter()
                .find(|p| u32::from(p.node) == part)
                .map_or(1.0, |p| p.extent[3]);
            scene.spdlines.spawn_part_streaks(
                side,
                [d[0], d[1], d[2]],
                me.obj.node_pos(part),
                size,
                height,
                libc,
            );
        }
    }
    if me.test_fx(0xB) {
        let p = &me.pose;
        if p.speed > 0.01 && crate::btl_chr::length(p.vel) / p.speed > 0.5 {
            scene
                .spdlines
                .spawn_body_trails(side, p.dir, height, |n| me.obj.node_pos(n), libc);
        }
    }
    if me.test_fx(0x15) {
        let d = crate::btl_chr::opp_delta(me, opp);
        let eps = 0.001;
        let mut angle = 0.0;
        if d[0].abs() > eps || d[2].abs() > eps {
            angle = crate::btl_input::wrap_angle(d[0].atan2(d[2]) - me.cam.yaw);
        }
        let arg = crate::eft_ray::RayArg {
            pos: [0.0; 3],
            color: [0x80, 0x80, 0x80, 0x40],
            life: angle.abs() / 3.141_592_7 * 0.5 + 0.3,
            length: 800.0,
            width: 1.5,
            inner: (-20.0 - crate::btl_chr::length(d) * 0.1).max(-50.0),
            jitter: 50.0,
            mode: crate::eft_ray::MODE_TECH,
            count: 0x30,
            chr: side,
            blend: 1,
            space: 1,
            delay: 0,
            fade_frames: 0,
            auto_kill: true,
        };
        scene.fx.spawn_ray(&arg, libc);
    }
}

/// A fighter as the ground dust reads it (`BtlScene_GetCharScale`: its height over 19.35).
pub(crate) fn dust_chr(me: &Chr, data: &SideData) -> crate::eft_gdust::DustChr {
    crate::eft_gdust::DustChr {
        pos: me.pose.pos,
        dir: me.pose.dir,
        moved: me.pose.moved,
        height: data.height(),
        scale: data.height() / 19.35,
        nodes: std::array::from_fn(|n| me.obj.node_pos(n as u32)),
    }
}

/// `BtlFx_UpdateGroundFx`: requests 0x2F..0x36 (dropped on some grounds): the slide (0x30) and dash (0x33)
/// emitters switched as their bit starts and ends, the body's dust (0x34), the landing ring at the ground point
/// (0x35; the body's dust when the ground is under water), two small puffs at each foot (0x32).
fn ground_fx(
    me: &mut Chr,
    data: &SideData,
    env: &Env,
    dust: &mut crate::eft_gdust::GndDust,
    libc: &mut crate::rng::LibcRandom,
    vu0: &mut crate::rng::Vu0Random,
) {
    if me.pose.ground_flags & 0x3000_0065 != 0 {
        for bit in 0x2F..=0x36 {
            me.clear_fx(bit);
        }
    }
    let c = dust_chr(me, data);
    let side = me.player;
    if me.fx_new(0x30) {
        dust.set_slide(side, true, &c, vu0);
    }
    if me.fx_ended(0x30) {
        dust.set_slide(side, false, &c, vu0);
    }
    if me.fx_new(0x33) {
        dust.set_dash(side, true, &c, libc, vu0);
    }
    if me.fx_ended(0x33) {
        dust.set_dash(side, false, &c, libc, vu0);
    }
    if me.test_fx(0x34) {
        dust.spawn_burst(side, &c, libc, vu0);
    }
    let coef = &env.fx.sin_coef;
    if me.test_fx(0x35) {
        let ground = me.pose.ground_pos;
        if env.stage.water_level.is_some_and(|w| ground[1] > w) {
            dust.spawn_burst(side, &c, libc, vu0);
        } else {
            dust.spawn_landing(side, ground, 1.0, libc, vu0, coef);
        }
    }
    if me.test_fx(0x32) {
        let y = me.pose.ground_pos[1];
        for node in [0xF, 0xB] {
            let mut p = me.obj.node_pos(node);
            p[1] = y;
            dust.spawn_impact(side, p, c.scale, libc, vu0, coef);
        }
    }
}

/// `BtlFx_UpdateWaterFx`: when flag 0x11 (in the water) changes, not while flag 0x24 holds the fighter, a splash
/// on the surface below it sized by its vertical speed and sound 0x3A going in / 0x3B coming out; effect bit 0x19
/// (the wake) while it moves faster than 6.48 with flag 0x12 (at the surface); going in also starts its bubbles'
/// burst.
fn water_fx(
    me: &mut Chr,
    data: &SideData,
    env: &Env,
    mut water: Option<&mut crate::eft_water::Water>,
    bubbles: Option<&mut crate::eft_bubble::Bubbles>,
    libc: &mut crate::rng::LibcRandom,
) {
    if me.flags.test(0x11) != me.flags.test_prev(0x11)
        && !me.flags.test(0x24)
        && let Some(level) = env.stage.water_level
    {
        if let Some(w) = water.as_deref_mut() {
            let at = [me.pose.pos[0], level, me.pose.pos[2]];
            let kind = crate::eft_water::SPLASH_CHAR;
            w.add_splash_for(data.height(), at, kind, me.pose.vel[1], libc);
        }
        let id = if me.flags.test(0x11) { 0x3A } else { 0x3B };
        me.play_common(&mut env.snd(), id);
        if me.flags.test(0x11)
            && let Some(b) = bubbles
        {
            b.start_burst(me.player, me.pose.speed);
        }
    }
    if me.flags.test(0x12) && crate::btl_chr::length(me.pose.vel) > 6.481_480_6 {
        me.set_fx(0x19);
    }
    if let Some(w) = water {
        if me.fx_new(0x19) {
            w.set_wake(me.player, true);
        }
        if me.fx_ended(0x19) {
            w.set_wake(me.player, false);
        }
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
fn begin_frame(chr: &mut Chr, data: &SideData, env: &Env) {
    if chr.is_frozen() {
        return;
    }
    chr.flags.set_stage(3);
    chr.tick_voice_timers();
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
    chr.prev_fx_bits = chr.fx_bits;
    chr.fx_bits = [0; 0x10];
    // BtlChar_SetLookEnabled(chr, 0): the actions that look enable it again.
    chr.look.enabled = false;
    // The dashes and vanishes in a row: the parameter's +0x70 / +0x72 in the powered-up mode, else 1; + 2 / 1
    // with ability 1 / 0 (dashes) and 3 / 2 (vanishes).
    let p = &data.tables.params;
    (chr.dash_limit, chr.vanish_limit) = if chr.flags.test(6) {
        (
            i32::from(p.u16_at(0x70) as i16),
            i32::from(p.u16_at(0x72) as i16),
        )
    } else {
        (1, 1)
    };
    if chr.has_ability(1) {
        chr.dash_limit += 2;
    } else if chr.has_ability(0) {
        chr.dash_limit += 1;
    }
    if chr.has_ability(3) {
        chr.vanish_limit += 2;
    } else if chr.has_ability(2) {
        chr.vanish_limit += 1;
    }
    chr.dir_held = -1;
    chr.input.tech_class = -1;
    let blocked = chr.flags.test(0x11E) || chr.flags.test(0x11F) || chr.flags.test(0xB1);
    chr.input.update(&env.gate, blocked);
    // BtlChar_ApplyBonusRequests / BtlChar_ApplyAbilityRequests: the members whose items the story script changed.
    for i in 0..5 {
        if chr.flags.test(0x103 + i as u32) {
            if let Some(m) = chr.members.get_mut(i) {
                m.items.bonus = chr.setup_items[i].bonus;
            }
            chr.flags.clear(0x103 + i as u32);
        }
    }
    for i in 0..5 {
        if chr.flags.test(0x108 + i as u32) {
            if let Some(m) = chr.members.get_mut(i) {
                m.items.ability = chr.setup_items[i].ability;
            }
            chr.flags.clear(0x108 + i as u32);
        }
    }
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
/// half its radius, pushed out of the stage after every step (the first standing object pushing in a step is
/// tested for breaking, `StgCol_FighterBreakObj`); when pushed, the object moves back by the correction and the
/// contact word gets bit 0x20.
fn update_fighter_stage(
    me: &mut Chr,
    data: &SideData,
    env: &Env,
    keep_sphere: bool,
    grow: bool,
    libc: &mut crate::rng::LibcRandom,
) {
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
            let (_, mask) = env.col.push_sphere(zi, &mut sphere, env.broken());
            if mask != 0 {
                fighter_break_obj(me, data, env, mask.trailing_zeros() as usize, delta, libc);
            }
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

/// `StgGround_UpdateFighter`: the ground under the object's position, below the body sphere's centre; a standing
/// object the probe met, when the ground found is above the model's root node (the fighter is under or inside
/// it), is tested for breaking.
fn update_ground(me: &mut Chr, data: &SideData, env: &Env, libc: &mut crate::rng::LibcRandom) {
    let (center, _) = me.obj.body_sphere();
    let pos = [me.obj.pos[0], center[1], me.obj.pos[2]];
    let b = ColBox::from_center_half(pos, [0.1, 1.0, 0.1]);
    let g = env
        .col
        .ground_probe(me.obj.area, &b, pos[0], pos[2], center[1], env.broken());
    me.obj.work.ground_pos = [me.obj.pos[0], g.y, me.obj.pos[2]];
    me.obj.work.ground = Some(g);
    let root_y = me.obj.nodes.first().map_or(0.0, |n| n.mtx[3][1]);
    if g.obj_mask != 0 && g.y < root_y {
        let idx = g.obj_mask.trailing_zeros() as usize;
        fighter_break_obj(me, data, env, idx, [1.0, 0.0, 1.0], libc);
    }
}

/// `StgCol_FighterBreakObj`: a standing stage object the fighter touched breaks when the fighter can break the
/// stage (`BtlCharApi_TestStageBreakA`: parameter flag 0x800 outside flag 0xA, or flag 9) and the object has type
/// bit 0x100, or else when it is knocked about (flag 0xA, `TestStageBreakB`; not with flags 0x21 / 0x22). A
/// break sets contact bit 0x40, with the damage bits of the object's type (0x800: 200, 0x2000: 600, 0x1000: 1000).
/// In story mode a fighter the pad plays that breaks an item holder finds the hidden dragon ball one `rand()` in 8
/// (`BtlStage_DestroyObj`).
fn fighter_break_obj(
    me: &mut Chr,
    data: &SideData,
    env: &Env,
    idx: usize,
    hit: [f32; 3],
    libc: &mut crate::rng::LibcRandom,
) {
    let mut objs = env.objs();
    let kind = objs.kind(idx);
    let a = (data.tables.params.flags() & 0x800 != 0 && !me.flags.test(0xA)) || me.flags.test(9);
    let breaks = if a {
        kind & 0x100 != 0
    } else {
        !me.flags.test(0x21) && !me.flags.test(0x22) && me.flags.test(0xA)
    };
    if breaks {
        let broke = objs.destroy(idx, hit, &mut env.snd());
        // BtlCharApi_IsInputInjected: not for the computer's fighter.
        if broke && !me.input.inject_on {
            let mut item = env.item.lock().unwrap_or_else(|e| e.into_inner());
            if item.rule != 0 && objs.holds_item(idx) && (libc.rand() >> 8) & 7 == 0 {
                item.found = item.rule;
                item.rule = 0;
                item.new = true;
            }
        }
        me.obj.work.hit_flags |= 0x40;
    }
    if me.obj.work.hit_flags & 0x40 != 0 {
        for (bit, flag) in [(0x800, 0x80), (0x2000, 0x100), (0x1000, 0x200)] {
            if kind & bit != 0 {
                me.obj.work.hit_flags |= flag;
            }
        }
    }
}

/// `BtlColl_UpdateGround`: the pose takes the ground found; on it (or landing on it) flag 0xF, else not.
fn coll_update_ground(me: &mut Chr, data: &SideData, env: &Env) {
    let height = data.height();
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
        // A stage object broken this frame: knocked about (flag 0xA), the fighter takes its damage.
        me.flags.set(0x20);
        me.flags.clear(0xF);
        if me.flags.test(0xA) {
            for (bit, amount) in [(0x80, 200), (0x100, 600), (0x200, 1000)] {
                if hit & bit != 0 {
                    me.damage(env, data, amount, 0);
                }
            }
        }
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

/// `BtlCam_TraceStage` with `StgCol_TraceSphere`: a sphere of radius 2 swept from the target (started two radii
/// behind it) to the eye through the target's zone, then the eye's when they differ; the camera stands where it
/// first touches a triangle the camera may not pass (not `POLY_NO_SOLID` / `POLY_NO_CAMERA`), with the fraction of
/// the way, else at the eye.
pub(crate) fn trace_camera(env: &Env, eye: [f32; 3], target: [f32; 3]) -> ([f32; 3], Option<f32>) {
    use bt3_data::stage_col::{POLY_NO_CAMERA, POLY_NO_SOLID};
    const RADIUS: f32 = 2.0;
    let (a, b) = (target, eye);
    let d = sub(b, a);
    let len = length(d);
    let dir = if len > 0.0 { scale3(d, 1.0 / len) } else { d };
    let start = sub(a, scale3(dir, RADIUS * 2.0));
    let sweep = crate::btl_blast::Sweep::new(start, b, RADIUS);
    let bounds = crate::btl_blast::capsule_bounds(a, b, RADIUS);
    let broken = env.broken();
    let zone_a = env.col.find_zone_near(-1, a);
    let zone_b = env.col.find_zone_near(-1, b);
    let zones = if zone_a != zone_b {
        vec![zone_a, zone_b]
    } else {
        vec![zone_a]
    };
    for zone in zones {
        let mut best = sweep.length;
        let mut found = None;
        let (hit, _) = env
            .col
            .query_zone(zone, &bounds, broken, false, &mut |poly, v| {
                if poly.flags & (POLY_NO_SOLID | POLY_NO_CAMERA) != 0 {
                    return false;
                }
                match sweep.test_tri(&v, poly.nrm) {
                    Some((pos, dist)) if dist < best => {
                        best = dist;
                        found = Some(pos);
                        true
                    }
                    _ => false,
                }
            });
        if hit && let Some(pos) = found {
            let frac = if sweep.length > 0.0 {
                best / sweep.length
            } else {
                0.0
            };
            return (pos, Some(frac));
        }
    }
    (eye, None)
}

/// `ChrCam_TurnToOpponent`: a camera the stage hid turns its yaw towards the opponent from where it stands, the
/// more the nearer the opponent (fully within 50, not from 1000, eased) and the nearer the hit to the target.
fn turn_to_opponent(me: &Chr, opp: &Chr, hit_frac: f32, rot: &mut [f32; 3]) {
    let d = sub(opp.snap_pos(6), me.cam.pos);
    let yaw = d[0].atan2(d[2]);
    let dist = crate::btl_chr::opp_distance(me, opp);
    let mut t = if dist < 50.0 {
        1.0
    } else if dist < 1000.0 {
        1.0 - (dist - 50.0) / 950.0
    } else {
        0.0
    };
    t = t * 2.0 - 1.0;
    t = -t * t.abs() + t * 2.0;
    t = t * 0.5 + 0.5;
    t *= 1.0 - hit_frac;
    if 0.0 < t {
        let w = crate::btl_input::wrap_angle;
        rot[1] = w(rot[1] + w(yaw - rot[1]) * t);
    }
}

/// `ChrCam_Update`: the wanted pose (last frame's with flag 0xCC, the cut, the fixed camera with flags 0xDC /
/// 0xB8, the lock-on camera or the free one), the shake, the stage's horizontal limit, the smoothing by
/// `ChrCam_GetRate`, the eye kept behind the head (follow cameras), the stage trace (`BtlCam_TraceStage`).
fn cam_update(
    me: &mut Chr,
    opp: &Chr,
    data: &SideData,
    opp_data: &SideData,
    env: &Env,
    libc: &mut LibcRandom,
) {
    let height = data.height();
    let time_stopped = env.gate.time_stopped;
    let mut keep_behind = false;
    // The cameras that turn to the opponent when the stage hides the eye (ChrCam_TurnToOpponent).
    let mut turn = false;
    let (mut eye, mut rot, mut target);
    if me.flags.test(0xCC) {
        (eye, rot, target) = (me.cam.pos, me.cam.rot, me.cam.target);
        if me.flags.test_prev(0xD3) {
            me.flags.set(0xD3);
        }
    } else if crate::chr_cam::is_cut_active(me) {
        (eye, rot, target) = crate::chr_cam::calc_cut(me, opp, data);
    } else if me.flags.test(0xDC) || me.flags.test(0xB8) {
        let ofs;
        (eye, rot, target, ofs) = crate::chr_cam::calc_fixed(me, opp, data, opp_data);
        me.cam.yaw_ofs = ofs;
        eye[1] += crate::chr_cam::bob(me, time_stopped);
        turn = true;
    } else if me.flags.test(5) {
        (eye, rot, target) = crate::battle::lock_on_camera(&crate::battle::LockOn {
            pos: me.pose.pos,
            height,
            side_slope: data.cam_side_slope(),
            opp_pos: opp.snap_pos(6),
            opp_height: opp_data.height(),
            stage_bottom: env.stage.bounds.bottom,
            yaw: me.cam.yaw,
            side: me.cam.side,
            dist_mode: me.cam.dist_mode,
            split: env.split,
        });
        eye[1] += crate::chr_cam::bob(me, time_stopped);
        keep_behind = true;
        turn = true;
    } else {
        // ChrCam_CalcFree.
        let (ofs, pitch) = crate::battle::camera_offset(height, me.cam.dist_mode);
        let look = [0.0, -(height * 0.7), 0.0];
        let p = me.pose.pos;
        let (s, c) = me.cam.yaw.sin_cos();
        eye = [
            p[0] + s * (ofs[2] - look[2]) + look[2],
            p[1] + (ofs[1] - look[1]) + look[1],
            p[2] + c * (ofs[2] - look[2]) + look[2],
        ];
        rot = [pitch, me.cam.yaw, 0.0];
        target = [look[0] + p[0], look[1] + p[1], look[2] + p[2]];
        eye[1] += crate::chr_cam::bob(me, time_stopped);
        keep_behind = true;
    }
    // CamShake_Calc / CamShake_Tick.
    let (shake_pos, shake_rot) = me.cam.shake.calc(libc);
    eye = crate::btl_chr::add(eye, shake_pos);
    me.cam.shake.tick();
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
    } else if time_stopped {
        me.cam.rate = 1.0;
    }
    let rate = me.cam.rate;
    if smooth {
        me.cam.eye = std::array::from_fn(|k| eye[k] * rate + me.cam.eye[k] * (1.0 - rate));
    } else {
        me.cam.eye = eye;
        me.cam.rate = calc_rate;
    }
    me.cam.target = target;
    if keep_behind {
        // ChrCam_KeepBehindHead.
        let head = me.obj.node_pos(0x30);
        crate::battle::keep_behind_head(head, height, &mut me.cam.eye, rot);
    }
    let (pos, frac) = trace_camera(env, me.cam.eye, me.cam.target);
    me.cam.pos = pos;
    if let Some(frac) = frac
        && turn
    {
        turn_to_opponent(me, opp, frac, &mut rot);
    }
    if smooth {
        let d: [f32; 3] =
            std::array::from_fn(|k| crate::btl_input::wrap_angle(rot[k] - me.cam.rot[k]));
        me.cam.rot =
            std::array::from_fn(|k| crate::btl_input::wrap_angle(me.cam.rot[k] + d[k] * rate));
    } else {
        me.cam.rot = rot;
    }
    me.cam.rot =
        std::array::from_fn(|k| crate::btl_input::wrap_angle(me.cam.rot[k] + shake_rot[k]));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btl_chr::HiddenItem;

    /// Story mode's hidden item on stage 0 of the ISO in `rom/`, when it is there: a fighter the pad plays that
    /// breaks an item holder finds it on a `rand()` draw of 0 in 8, the computer's never; the battle then plays the
    /// jingle and marks the clock.
    #[test]
    fn real_player_finds_the_hidden_item_in_an_item_holder() {
        let mut setup = crate::char_select::BattleSetup::default();
        setup.sides[1].chara = 1;
        setup.sides[1].control = 2;
        let Some(mut battle) = crate::battle::tests::real_battle(&setup) else {
            return;
        };
        let holders: Vec<usize> = {
            let objs = battle.assets.stage_objs.lock().unwrap();
            (0..64).filter(|&i| objs.holds_item(i)).collect()
        };
        assert!(holders.len() >= 2);
        // A seed whose first draw finds it, and one whose first draw does not.
        let draw = |seed| (crate::rng::LibcRandom::new(seed).rand() >> 8) & 7;
        let hit = (1..1000).find(|&s| draw(s) == 0).unwrap();
        let miss = (1..1000).find(|&s| draw(s) != 0).unwrap();
        *battle.assets.hidden_item.lock().unwrap() = HiddenItem {
            rule: 3,
            ..HiddenItem::default()
        };
        let state = battle.seq.state_id();
        let gate = battle.gate();
        let mut break_with = |battle: &mut crate::battle::Battle, side: usize, idx: usize, seed| {
            let data = crate::battle::side_data(&battle.assets.fighters[side]);
            let env = battle.assets.env(state, gate);
            let me = &mut battle.roster.chars[side];
            // Knocked about (flag 0xA): any object it touches breaks.
            me.flags.set(0xA);
            let mut libc = crate::rng::LibcRandom::new(seed);
            fighter_break_obj(me, &data, &env, idx, [1.0, 0.0, 1.0], &mut libc);
        };
        // The computer's fighter, then a miss: nothing found.
        break_with(&mut battle, 1, holders[0], hit);
        break_with(&mut battle, 0, holders[1], miss);
        assert_eq!(battle.assets.hidden_item.lock().unwrap().found, 0);
        let left = holders[2..].first().copied().unwrap_or(holders[1]);
        break_with(&mut battle, 0, left, hit);
        let item = *battle.assets.hidden_item.lock().unwrap();
        assert_eq!((item.rule, item.found, item.new), (0, 3, true));
        let mut libc = crate::rng::LibcRandom::new(1);
        let mut random = crate::rng::GameRandom::from_key(&[1]);
        let frame = battle.run_frame(
            false,
            [crate::btl_input::PadState::default(); 2],
            0,
            &mut libc,
            &mut random,
            None,
            false,
        );
        assert!(
            frame
                .sounds
                .contains(&crate::title::TitleSound::StreamSe(0x8D3A))
        );
        assert!(!battle.assets.hidden_item.lock().unwrap().new);
    }
}
