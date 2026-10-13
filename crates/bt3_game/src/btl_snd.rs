//! Fighter sound (src/battle/btl_char_sound.c): fighter code never plays a sound itself. It queues a request
//! (position, near / far distance, owner, kind, id) in the roster's list of four (one more is dropped), and
//! `BtlCharSnd_PlayRequests`, once a frame after the fighters, plays each with a volume and pan from the battle
//! camera. A sample-bank sound's handle is kept per side, one-shot sounds in one ring of four slots (so they can be
//! cut short), looping sounds in another; a looping sound has to be requested every frame, else it is stopped.
//!
//! The streamed sounds and voice lines (kinds 4..7) go to the owner's player with the volume and pan worked out
//! here.

use bt3_data::snd::SeBank;

use crate::battle::{Mtx, inverse_rt};
use crate::battle_seq::{VOICE_CHARA_LINES, voice_chara_base};
use crate::btl_chr::{Chr, Env, SideData, length, sub};
use crate::btl_input::wrap_angle;
use crate::title::TitleSound;

/// `BTL_VOICE_KINDS`.
pub(crate) const VOICE_KINDS: usize = 57;

/// Request kinds.
pub(crate) const KIND_COMMON: i32 = 0;
pub(crate) const KIND_BANK8: i32 = 1;
pub(crate) const KIND_STREAM: i32 = 4;
pub(crate) const KIND_VOICE: i32 = 5;

/// `gBtlSndBankMask`: the sample bank of request kinds 0..3.
const BANK_MASK: [u32; 4] = [4, 8, 0x10, 0x20];

#[derive(Clone, Copy, Debug)]
struct Req {
    pos: [f32; 3],
    owner: i32,
    kind: i32,
    id: i32,
    near: f32,
    far: f32,
}

/// `BtlFlagSound`: a playing sound's handle (< 0: free), kind and id.
#[derive(Clone, Copy, Debug)]
struct Slot {
    handle: i32,
    kind: i32,
    id: i32,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            handle: -1,
            kind: -1,
            id: 0xFF,
        }
    }
}

/// `BtlFlagSoundSet`: four slots and the next one to use.
#[derive(Clone, Copy, Debug, Default)]
struct SlotSet {
    slot: [Slot; 4],
    next: usize,
}

impl SlotSet {
    /// `BtlCharSnd_StoreHandle`: the first free slot from `next` (any with `force`).
    fn store(&mut self, handle: i32, kind: i32, id: i32, force: bool) {
        for i in 0..4 {
            let n = (self.next + i) % 4;
            if self.slot[n].handle < 0 || force {
                self.slot[n] = Slot {
                    handle,
                    kind,
                    id: id & 0xFF,
                };
                self.next = (self.next + 1) % 4;
                break;
            }
        }
    }
}

/// `gBtlChars->snd` with both sides' slot sets, the sample banks' loop flags and the handle counter of
/// `Snd_PlaySeEx`.
#[derive(Clone, Debug, Default)]
pub(crate) struct CharSound {
    reqs: Vec<Req>,
    sets: [SlotSet; 2],
    loops: [SlotSet; 2],
    /// Per request kind 0..3, by sound id: the sample loops (`Snd_IsLoopSe`); empty while the bank is not loaded.
    loop_bits: [Vec<bool>; 4],
    next_handle: u32,
    /// `Voice_Stop(player)` asked for this frame (`BtlChar_StopVoiceOnFlag`).
    stop_voice: [bool; 2],
    /// `Snd_StopHandle` calls of this frame (`BtlCharSnd_Stop`).
    stops: Vec<u32>,
    /// `BtlScript_IsTextShown` in mode 1: the story script shows a line's text (the fighters' sounds are quieter and
    /// their voices are not cut).
    pub(crate) script_text: bool,
}

impl CharSound {
    /// The loop flags of a bank file (`SndHd_GetLoopBits`), for request kind `kind` (0..3); `None` unloads it.
    pub(crate) fn set_bank(&mut self, kind: usize, file: Option<&[u8]>) {
        self.loop_bits[kind] = file
            .and_then(|f| SeBank::parse(f).ok())
            .map(|b| b.vags.iter().map(|v| v.attribute != 0).collect())
            .unwrap_or_default();
    }

    /// The kind of a bank mask (4, 8, 0x10, 0x20).
    pub(crate) fn kind_of_mask(mask: u32) -> Option<usize> {
        BANK_MASK.iter().position(|&m| m == mask)
    }

    /// `BtlCharSnd_ClearRequests` (start of the frame).
    pub(crate) fn clear(&mut self) {
        self.reqs.clear();
    }

    /// `Voice_Stop(player)` before this frame's requests play.
    pub(crate) fn stop_voice(&mut self, player: usize) {
        self.stop_voice[player & 1] = true;
    }

    /// `BtlCharSnd_Request`: dropped when four are queued.
    pub(crate) fn request(
        &mut self,
        pos: [f32; 3],
        owner: i32,
        kind: i32,
        near: f32,
        far: f32,
        id: i32,
    ) {
        if self.reqs.len() < 4 {
            self.reqs.push(Req {
                pos,
                owner,
                kind,
                id,
                near,
                far,
            });
        }
    }

    /// This frame's requests as (owner, kind, id).
    #[cfg(test)]
    pub(crate) fn requests(&self) -> Vec<(i32, i32, i32)> {
        self.reqs.iter().map(|r| (r.owner, r.kind, r.id)).collect()
    }

    /// `BtlCharSnd_IsRequested`.
    fn is_requested(&self, owner: i32, kind: i32, id: i32) -> bool {
        self.reqs
            .iter()
            .any(|r| r.owner == owner && r.id == id && r.kind == kind)
    }

    /// `Snd_PlaySeEx`'s handle: -1 when the bank is not loaded or the id is out of range.
    /// `Snd_PlaySeEx` of request kind `kind`'s bank with a new handle (-1 when the bank is not loaded).
    pub(crate) fn play_se(
        &mut self,
        kind: usize,
        id: i32,
        vol: i32,
        pan: i32,
        pitch: i32,
        out: &mut Vec<TitleSound>,
    ) -> i32 {
        if self.loop_bits[kind].is_empty() || !(0..0x100).contains(&id) {
            return -1;
        }
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        out.push(TitleSound::SeEx {
            mask: BANK_MASK[kind],
            id: id as u32,
            volume: vol,
            pan,
            pitch,
            handle: self.next_handle,
        });
        self.next_handle as i32
    }

    /// `BtlCharSnd_PlayRequests`: the frame's requests played (a looping sound only when it is not already
    /// playing), then each side's looping sounds nobody asked for this frame stopped. `view` is the battle
    /// camera's world-to-view matrix; `split` puts every sound in the middle at half volume.
    /// `charas`: each side's character (0x56 for a changed body), whose lines kinds 5 / 6 play.
    pub(crate) fn play_requests(
        &mut self,
        view: &Mtx,
        split: bool,
        charas: [i32; 2],
        out: &mut Vec<TitleSound>,
    ) {
        for player in 0..2 {
            if std::mem::take(&mut self.stop_voice[player]) {
                out.push(TitleSound::VoiceStopEx(player as u32));
            }
        }
        out.extend(self.stops.drain(..).map(TitleSound::SeStop));
        let reqs = self.reqs.clone();
        for req in reqs {
            let owner = req.owner.max(0) as usize & 1;
            let (mut vol, pan) = vol_pan(view, req.pos, req.near, req.far, split);
            if self.script_text {
                vol = (vol - 0x20).max(0);
            }
            match req.kind {
                KIND_STREAM => out.push(TitleSound::StreamSeEx {
                    player: owner as u32,
                    id: req.id as u32,
                    volume: vol,
                    pan,
                }),
                // Voice_PlayCharaEx: a line of the owner's character on its voice player.
                5 | 6 => out.push(TitleSound::VoiceEx {
                    player: owner as u32,
                    id: voice_chara_base()
                        + charas[owner].max(0) as u32 * VOICE_CHARA_LINES
                        + req.id as u32,
                    volume: vol,
                    pan,
                }),
                7 => out.push(TitleSound::VoiceEx {
                    player: owner as u32,
                    id: req.id as u32,
                    volume: vol,
                    pan,
                }),
                k @ 0..=3 => {
                    let kind = k as usize;
                    let looping = self.loop_bits[kind]
                        .get(req.id as usize)
                        .copied()
                        .unwrap_or(false);
                    if looping {
                        let playing = self.loops[owner]
                            .slot
                            .iter()
                            .any(|s| s.handle >= 0 && s.id == req.id && s.kind == k);
                        if !playing {
                            let h = self.play_se(kind, req.id, vol, pan, 0, out);
                            self.loops[owner].store(h, k, req.id, false);
                        }
                    } else {
                        let h = self.play_se(kind, req.id, vol, pan, 0, out);
                        self.sets[owner].store(h, k, req.id, true);
                    }
                }
                _ => {}
            }
        }
        for side in 0..2 {
            for i in 0..4 {
                let s = self.loops[side].slot[i];
                if s.handle >= 0 && !self.is_requested(side as i32, s.kind, s.id) {
                    out.push(TitleSound::SeStop(s.handle as u32));
                    self.loops[side].slot[i].handle = -1;
                }
            }
        }
    }

    /// `BtlCharSnd_Stop`: cuts short side `side`'s one-shot sound of that kind and id (unless requested already
    /// this frame); `Snd_StopHandle` goes out with this frame's sounds.
    pub(crate) fn stop(&mut self, side: usize, kind: i32, id: i32) {
        for i in 0..4 {
            let s = self.sets[side & 1].slot[i];
            if s.handle >= 0
                && !self.is_requested(side as i32, s.kind, s.id)
                && s.kind == kind
                && s.id == id & 0xFF
            {
                self.stops.push(s.handle as u32);
                self.sets[side & 1].slot[i].handle = -1;
            }
        }
    }
}

/// `BtlCharSnd_CalcVolPan`: volume 0..0x80 falling from `near` to `far` from the camera, pan -0x40..0x40 by the
/// sine of the angle from the camera's forward axis; 0x40 in the middle in split screen.
fn vol_pan(view: &Mtx, pos: [f32; 3], near: f32, far: f32, split: bool) -> (i32, i32) {
    if split {
        return (0x40, 0);
    }
    let cam = inverse_rt(view);
    let d = sub(pos, [cam[3][0], cam[3][1], cam[3][2]]);
    let dist = length(d);
    let fwd = cam[2];
    let ang = if (d[0].abs() < 0.001 && d[2].abs() < 0.001)
        || (fwd[0].abs() < 0.001 && fwd[2].abs() < 0.001)
    {
        0.0
    } else {
        wrap_angle(d[0].atan2(d[2]) - fwd[0].atan2(fwd[2]))
    };
    let v = (((1.0 - (dist - near) / (far - near)) * 128.0) as i32).clamp(0, 0x80);
    let p = ((ang.sin() * 64.0) as i32).clamp(-0x40, 0x40);
    (v, p)
}

/// `BtlChar_GetHitSoundLine`: the swing sound of an attack's hit sound level, -1 for none.
fn hit_sound_line(level: i32, frame: u32) -> i32 {
    match level {
        1..=4 => (frame % 2) as i32,
        5 | 9..=12 => (frame % 3) as i32 + 6,
        6 => 0x28,
        7 => 0x29,
        8 => 0x2A,
        _ => -1,
    }
}

/// The sounds of `BtlFx_UpdateAll` (`BtlFx_HandleImpactEvents`, `BtlFx_PlaySwingSound`,
/// `BtlFxObj_PlayEventSounds`): a light (0x100) or heavy (0x80) impact event's sound at node 3 and voice kind 1 for
/// the fighter taking the blow (the attacker's opponent, or itself in a motion with flag 0x10; not when the other
/// has flag 0x13B or the speaker is frozen); event 2's swing sound of the attack (not with flag 0x91, nor with
/// 0x60 unless 0x5C); the object's sound events at its node 3. A shown weapon outside a rush sequence turns the
/// impact sounds into 0x4D / 0x36; a visible fighter's camera shakes (5 for 0.1 s, 10 for 0.2 s). The sparks are
/// `btl_fx::update_all`.
pub(crate) fn fx_event_sounds(
    me: &mut Chr,
    opp: &mut Chr,
    data: &SideData,
    env: &Env,
    frame: u32,
    libc: &mut crate::rng::LibcRandom,
) {
    let mut impact = None;
    if me.anim_test_attr(0x100) {
        impact = Some(((frame % 2) as i32 + 2, 0x4D, 5.0, 0.1));
    }
    if me.anim_test_attr(0x80) {
        impact = Some((0x49, 0x36, 10.0, 0.2));
    }
    if let Some((mut sound, weapon, strength, time)) = impact {
        if !crate::btl_tech_fx::in_rush_sequence(me) && has_weapon_out(me, data) {
            sound = weapon;
        }
        if me.obj.flags & 2 != 0 {
            crate::chr_cam::add_shake(me, strength, time);
        }
        me.play_common(&mut env.snd(), sound);
        let (speaker, other) = if env.anim_flags(me.anim.cur) & 0x10 != 0 {
            (&mut *me, &*opp)
        } else {
            (&mut *opp, &*me)
        };
        if !other.flags.test(0x13B) && !speaker.is_frozen() {
            speaker.voice(env, frame, 1, libc);
        }
    }
    if !me.flags.test(0x91) && me.anim_test_attr(2) && (!me.flags.test(0x60) || me.flags.test(0x5C))
    {
        let level = crate::btl_hit::hit_sound_level(me, data, env);
        let line = hit_sound_line(level, frame);
        if line >= 0 {
            me.play_common(&mut env.snd(), line);
        }
    }
    // BtlFxObj_HandleFlagEvents: events 0x20 / 0x40 and bits 43 / 44 show and hide the weapons (object flags 0x20,
    // 0x40); bit 42's vanish lines start in `btl_mgr::movement_fx`.
    for (mask, set, bit) in [
        (0x20, true, 0x20),
        (0x40, false, 0x20),
        (0x800_0000_0000, true, 0x40),
        (0x1000_0000_0000, false, 0x40),
    ] {
        if me.obj.test_event(mask) {
            if set {
                me.obj.flags |= bit;
            } else {
                me.obj.flags &= !bit;
            }
        }
    }
    obj_event_sounds(&me.obj, data.tables.params.flags(), frame, &mut env.snd());
}

/// `BtlFxObj_PlayEventSounds`: an object's sound events at its node 3, heard from 200 to 1500 (`param_flags`: its
/// parameter block's flags, 0x8000 picking sound 0xB over 0x33).
pub(crate) fn obj_event_sounds(
    obj: &crate::btl_obj::Obj,
    param_flags: u32,
    frame: u32,
    snd: &mut CharSound,
) {
    let pos = obj.node_pos(3);
    let obj_id = if param_flags & 0x8000 != 0 { 0xB } else { 0x33 };
    let events: [(u64, i32, i32); 20] = [
        (0x10, KIND_COMMON, (frame % 3) as i32 + 0x28),
        (0x1000_0000, KIND_COMMON, 0x1E),
        (0x2000_0000, KIND_COMMON, (frame % 2) as i32),
        (0x4000_0000, KIND_COMMON, (frame % 3) as i32 + 6),
        (0x8_0000_0000, KIND_COMMON, 0x20),
        (0x10_0000_0000, KIND_COMMON, 0x4D),
        (0x20_0000_0000, KIND_COMMON, 0x25),
        (0x40_0000_0000, KIND_COMMON, obj_id),
        (0x100_0000_0000, KIND_COMMON, 0x30),
        (0x200_0000_0000, KIND_STREAM, 0x8D3B),
        (0x80_0000_0000_0000, KIND_STREAM, 0x8D3D),
        (0x800_0000_0000, KIND_COMMON, 0x48),
        (0x1000_0000_0000, KIND_COMMON, 0x12),
        (0x2000_0000_0000, KIND_COMMON, 0x18),
        (0x4000_0000_0000, KIND_COMMON, 0x2C),
        (0x8000_0000_0000, KIND_COMMON, 0x1F),
        (0x10_0000_0000_0000, KIND_COMMON, 0x31),
        (0x20_0000_0000_0000, KIND_COMMON, (frame % 2) as i32 + 4),
        (0x8000_0000, KIND_BANK8, (frame % 2) as i32),
        (0, 0, 0),
    ];
    for (mask, kind, id) in events {
        if mask != 0 && obj.test_event(mask) {
            snd.request(pos, -1, kind, 200.0, 1500.0, id);
        }
    }
}

/// `BtlCharApi_HasWeaponOut`: a shown weapon of parameter flag 0x40 (object flags 0x22) or 0x80 (0x42).
pub(crate) fn has_weapon_out(me: &Chr, data: &SideData) -> bool {
    let (p, o) = (data.tables.params.flags(), me.obj.flags);
    (p & 0x40 != 0 && o & 0x22 == 0x22) || (p & 0x80 != 0 && o & 0x42 == 0x42)
}

/// The sounds of `BtlFx_UpdateAfterHits` (`BtlChar_SpawnHitFx`, `BtlFx_SpawnFlashReq20`): a landed blow (effect
/// bit 0x1B) by the attack's hit sound level (0x4D / 0x36 when a shown weapon strikes, unless attack flag
/// 0x2000), a guarded one (0x1C) 4 / 5 or 0xC / 0xD, 0x21 and 0x22's 0x24, 0x24's 0x42, a guarding fighter's
/// (0x20) 0x46. The sparks are `btl_fx::after_hits`.
pub(crate) fn after_hit_sounds(me: &Chr, data: &SideData, env: &Env, frame: u32) {
    let odd = (frame % 2) as i32;
    let mut sound = -1;
    if me.test_fx(0x1B) {
        let level = crate::btl_hit::hit_sound_level(me, data, env);
        let weapon =
            has_weapon_out(me, data) && crate::btl_hit::attack_flags(me, data, env) & 0x2000 == 0;
        sound = if weapon {
            if level < 5 { 0x4D } else { 0x36 }
        } else {
            match level {
                1 => odd + 2,
                2 => 0x19,
                3 => 0x22,
                4 => 0x45,
                5 => odd + 9,
                6 => 0x25,
                7 => 0x26,
                8 => 0x27,
                9 => 0x49,
                10 => 0x4E,
                11 => 0x50,
                12 => 0x4F,
                13 => 0x4B,
                14 => 0xE,
                _ => -1,
            }
        };
    }
    if me.test_fx(0x1C) {
        let level = crate::btl_hit::hit_sound_level(me, data, env);
        sound = if level < 5 { odd + 4 } else { odd + 0xC };
    }
    if me.test_fx(0x21) {
        sound = 0x24;
    }
    if me.test_fx(0x24) {
        sound = 0x42;
    }
    if me.test_fx(0x22) {
        sound = if crate::btl_hit::hit_fx_kind(me, data, env) < 4 {
            odd + 0xC
        } else {
            0x24
        };
    }
    let mut snd = env.snd();
    if sound >= 0 {
        me.play_common(&mut snd, sound);
    }
    if me.test_fx(0x20) {
        me.play_common(&mut snd, 0x46);
    }
}

/// `BtlColl_PlayHitSound`: a technique's or skill's hit sound kind 0..4: the impact at the record (none for 0) and
/// the victim's voice kind (0, 0, 1, 3, 3).
pub(crate) fn hit_sound(
    victim: Option<(&mut Chr, &mut crate::rng::LibcRandom)>,
    pos: Option<[f32; 3]>,
    kind: u32,
    env: &Env,
    frame: u32,
) {
    let (sound, voice) = match kind {
        0 => (None, 0),
        1 => (Some(if frame % 2 != 0 { 0x1B } else { 0x11 }), 0),
        2 => (Some(0x47), 1),
        3 => (Some(0x4A), 3),
        4 => (Some(0x12), 3),
        _ => return,
    };
    if let (Some(pos), Some(sound)) = (pos, sound) {
        env.snd()
            .request(pos, -1, KIND_COMMON, 200.0, 1500.0, sound);
    }
    if let Some((chr, libc)) = victim {
        chr.voice(env, frame, voice, libc);
    }
}

/// Where a fighter's sounds come from (`BtlCharApi_GetNodePos(objId, node)`).
fn at(chr: &Chr, node: u32) -> [f32; 3] {
    chr.obj.node_pos(node)
}

impl Chr {
    /// `BtlChar_PlayVoice`: a line of voice kind `kind` (`voiceTbl`: one, or one of several by the frame, never
    /// the last one twice) unless the fighter is dead or the kind is cooling down (in mode 1 not kinds 14..16 and
    /// 34); kinds below 15 are sounds of the fighter's own bank with one talk pattern of the mouth, the others
    /// streamed voice lines with their lip track.
    pub(crate) fn voice(
        &mut self,
        env: &Env,
        frame: u32,
        kind: usize,
        libc: &mut crate::rng::LibcRandom,
    ) {
        if self.is_dead() || kind >= VOICE_KINDS || self.voice_timer[kind] > 0 {
            return;
        }
        if env.mode == 1 && (14..17).contains(&kind) || env.mode == 1 && kind == 34 {
            return;
        }
        let Some(e) = env.common.voices.get(kind).copied() else {
            return;
        };
        let line = if e.count >= 2 {
            let mut line = i32::from(e.first) + (frame % (e.count as u32 - 1)) as i32;
            if line >= self.voice_last[kind] {
                line += 1;
            }
            line
        } else {
            i32::from(e.first)
        };
        let mut snd = env.snd();
        if kind < 15 {
            self.play_own(&mut snd, line);
            self.obj.set_sub_state(4, 0, libc);
        } else {
            self.play_voice(&mut snd, line);
            self.obj.set_sub_state(2, line, libc);
        }
        self.voice_last[kind] = line;
        self.voice_timer[kind] = (e.interval * 30.0) as i32;
    }

    /// `BtlChar_PlayVoiceSingle`: cuts short the own-bank line of a voice kind that has only one.
    pub(crate) fn voice_stop_single(&self, env: &Env, kind: usize) {
        if let Some(e) = env.common.voices.get(kind) {
            if e.count < 2 {
                let bank = self.player as i32 + 2;
                env.snd().stop(self.player, bank, i32::from(e.first));
            }
        }
    }

    /// `BtlChar_TickVoiceTimers`.
    pub(crate) fn tick_voice_timers(&mut self) {
        for t in &mut self.voice_timer {
            if *t > 0 {
                *t -= 1;
            }
        }
    }

    /// `BtlCharSnd_PlayCommon`: a sound of the common battle bank (4) at node 3, heard from 200 to 1500.
    pub(crate) fn play_common(&self, snd: &mut CharSound, id: i32) {
        snd.request(
            at(self, 3),
            self.player as i32,
            KIND_COMMON,
            200.0,
            1500.0,
            id,
        );
    }

    /// `BtlCharSnd_PlayCommonFar`: the same, heard everywhere at full volume.
    pub(crate) fn play_common_far(&self, snd: &mut CharSound, id: i32) {
        snd.request(
            at(self, 3),
            self.player as i32,
            KIND_COMMON,
            100_000.0,
            100_000.0,
            id,
        );
    }

    /// `BtlCharSnd_PlayBank8`: a sound of the stage's bank (8) at node 3.
    pub(crate) fn play_bank8(&self, snd: &mut CharSound, id: i32) {
        snd.request(
            at(self, 3),
            self.player as i32,
            KIND_BANK8,
            200.0,
            1500.0,
            id,
        );
    }

    /// `BtlCharSnd_PlayOwn`: a sound of the fighter's own bank (0x10 / 0x20) at its head node 0x30.
    pub(crate) fn play_own(&self, snd: &mut CharSound, id: i32) {
        let kind = self.player as i32 + 2;
        snd.request(at(self, 0x30), self.player as i32, kind, 200.0, 1500.0, id);
    }

    /// `BtlCharSnd_PlayStream`: a streamed sound at node 3.
    pub(crate) fn play_stream(&self, snd: &mut CharSound, id: i32) {
        snd.request(
            at(self, 3),
            self.player as i32,
            KIND_STREAM,
            200.0,
            1500.0,
            id,
        );
    }

    /// `BtlCharSnd_PlayVoice` (`BtlChar_PlayVoice`'s request): a voice line of the fighter's character at node 0x30.
    pub(crate) fn play_voice(&self, snd: &mut CharSound, line: i32) {
        let kind = self.player as i32 + KIND_VOICE;
        snd.request(
            at(self, 0x30),
            self.player as i32,
            kind,
            200.0,
            1500.0,
            line,
        );
    }

    /// The voice parts of `BtlChar_UpdateLate`: `BtlChar_StopVoiceOnFlag` (flag 0x129 cuts the side's voice line,
    /// not while story mode's script shows a text), `BtlCharSnd_PlayTechSounds` and `BtlMove_UpdateAnimVoice`.
    pub(crate) fn late_voice(
        &mut self,
        data: &SideData,
        opp_data: &SideData,
        env: &Env,
        frame: u32,
        libc: &mut crate::rng::LibcRandom,
    ) {
        if self.flags.test(0x129) {
            let mut snd = env.snd();
            if !snd.script_text {
                snd.stop_voice(self.player);
            }
        }
        self.play_tech_sounds(&mut env.snd(), data, opp_data, libc);
        self.update_anim_voice(env, frame, libc);
    }

    /// `BtlCharSnd_PlayTechSounds`: the sound (motion events 0x10000 .. 0x80000, own bank) and voice line (events
    /// 0x8000 and 0x80_0000_0000) of the skill or technique in use; against an opponent with character flag 0x80
    /// the technique's flags 0x20 / 0x40 drop the second and third sound. The voice lines play their lip tracks.
    fn play_tech_sounds(
        &mut self,
        snd: &mut CharSound,
        data: &SideData,
        opp_data: &SideData,
        libc: &mut crate::rng::LibcRandom,
    ) {
        let slot = crate::btl_act::id_class(self.act.current);
        let (se, line) = match slot {
            0 => (-1, 0x53),
            1 => (-1, 0x55),
            2 => (0x2D, 0x57),
            3 => (0x31, 0x59),
            4 => (0x35, 0x5B),
            _ => return,
        };
        if se >= 0 {
            let gated = opp_data.tables.params.chara_flags() & 0x80 != 0;
            let tf = data.tables.tech().flags(slot);
            if self.anim_test_attr(0x1_0000) {
                self.play_own(snd, se);
            }
            if self.anim_test_attr(0x2_0000) && (!gated || tf & 0x20 == 0) {
                self.play_own(snd, se + 1);
            }
            if self.anim_test_attr(0x4_0000) && (!gated || tf & 0x40 == 0) {
                self.play_own(snd, se + 2);
            }
            if self.anim_test_attr(0x8_0000) {
                self.play_own(snd, se + 3);
            }
        }
        if self.anim_test_attr(0x8000) {
            self.play_voice(snd, line);
            self.obj.set_sub_state(2, line, libc);
        }
        if self.anim_test_attr(0x80_0000_0000) {
            self.play_voice(snd, line + 1);
            self.obj.set_sub_state(2, line + 1, libc);
        }
    }

    /// `BtlMove_UpdateAnimVoice`: the voices and streamed sounds of the motions flagged 0x12F .. 0x133.
    fn update_anim_voice(&mut self, env: &Env, frame: u32, libc: &mut crate::rng::LibcRandom) {
        let mut kind = None;
        if self.flags.test(0x12F) && self.anim_test_attr(0x8000) {
            kind = Some(0xE);
        }
        if self.flags.test(0x130) {
            if self.anim_test_attr(0x8000) {
                kind = Some(0x34);
            }
            if self.anim_test_attr(0x80_0000_0000) {
                kind = Some(0x35);
            }
            if self.anim_test_attr(0x2_0000) {
                kind = Some(0x36);
            }
            if self.anim_test_attr(0x4_0000) {
                self.play_stream(&mut env.snd(), 0x8D35);
            }
        }
        if self.flags.test(0x131) {
            if self.anim_test_attr(0x80_0000_8000) {
                kind = Some(0x37);
            }
            if self.anim_test_attr(0x4_0000) {
                self.play_stream(&mut env.snd(), 0x8D37);
            }
        }
        if self.flags.test(0x132) && self.anim_test_attr(0x1_0000) {
            self.play_stream(&mut env.snd(), 0x8D42);
        }
        if self.flags.test(0x133) && self.anim_test_attr(0x1_0000) {
            self.play_stream(&mut env.snd(), 0x8D41);
        }
        if let Some(kind) = kind {
            self.voice(env, frame, kind, libc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::IDENTITY;

    #[test]
    fn voice_and_stream_requests_go_to_the_owners_players() {
        let mut s = CharSound::default();
        s.request([0.0; 3], 1, KIND_VOICE + 1, 200.0, 1500.0, 0x53);
        s.request([0.0; 3], 0, KIND_VOICE, 200.0, 1500.0, 4);
        s.request([0.0; 3], 1, 7, 200.0, 1500.0, 0x9000);
        s.request([0.0; 3], 1, KIND_STREAM, 200.0, 1500.0, 0x8D3B);
        let mut out = Vec::new();
        s.play_requests(&IDENTITY, true, [3, 0x56], &mut out);
        let v = |player, id| TitleSound::VoiceEx {
            player,
            id,
            volume: 0x40,
            pan: 0,
        };
        assert_eq!(
            out,
            [
                v(1, voice_chara_base() + 0x56 * 100 + 0x53),
                v(0, voice_chara_base() + 3 * 100 + 4),
                v(1, 0x9000),
                TitleSound::StreamSeEx {
                    player: 1,
                    id: 0x8D3B,
                    volume: 0x40,
                    pan: 0
                },
            ]
        );
    }

    #[test]
    fn four_requests_at_most_and_loops_stop_unless_asked_again() {
        let mut s = CharSound::default();
        s.loop_bits[0] = vec![false, true];
        for _ in 0..5 {
            s.request([0.0; 3], 0, 0, 200.0, 1500.0, 0);
        }
        assert_eq!(s.reqs.len(), 4);
        s.clear();
        // A looping sound plays once while it is asked for, and stops the frame it is not.
        let mut out = Vec::new();
        s.request([0.0; 3], 0, 0, 200.0, 1500.0, 1);
        s.play_requests(&IDENTITY, false, [0; 2], &mut out);
        s.clear();
        s.request([0.0; 3], 0, 0, 200.0, 1500.0, 1);
        s.play_requests(&IDENTITY, false, [0; 2], &mut out);
        assert_eq!(out.len(), 1);
        s.clear();
        s.play_requests(&IDENTITY, false, [0; 2], &mut out);
        assert!(matches!(out.last(), Some(TitleSound::SeStop(1))));
        // Volume falls off from near to far; a sound to the right pans right.
        assert_eq!(
            vol_pan(&IDENTITY, [0.0, 0.0, 100.0], 200.0, 1500.0, false),
            (0x80, 0)
        );
        assert_eq!(
            vol_pan(&IDENTITY, [2000.0, 0.0, 0.0], 200.0, 1500.0, false).0,
            0
        );
        assert!(vol_pan(&IDENTITY, [500.0, 0.0, 1.0], 200.0, 1500.0, false).1 > 0x30);
    }
}
