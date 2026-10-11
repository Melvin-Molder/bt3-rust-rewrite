//! The clash sequences between the two fighters (src/battle/btl_clash.c, `BtlChar_PlaceOnPath` of
//! src/battle/btl_char_pose.c): started when both fighters are in a clash action at once, they decide who won and
//! tell the actions by flags.
//!
//! Clash A (states 1..5): both in a beam struggle (0x130..0x132); for 106 frames the one counting more inputs
//! gains the lead, then the leader's lead grows to 71: the winner gets held flags 0xC1 / 0xC3, the loser 0xC2 and
//! the damage of both techniques (the winner's whole plus half the loser's); the struggle effect ends, the loser's
//! effects reset (a throw technique's win would throw: not ported). Clash B (states 6..8): both in 0xFA, trading blows for 76 frames; the one who counted more inputs wins
//! (flags 0xBF / 0xC0). Clash C (states 9..11): both in 0xFB, then 0xFC exchanges placed on the stage paths; each
//! exchange (both raising 0xC4) scores one for the fighter whose answer is right, and after seven with a leader
//! the winner gets 0xC7, the loser 0xC8. Not ported: the clash cameras (`ChrCam_SetCut` / `RequestCut`), the voices and the streamed sounds.

use crate::btl_chr::{Chr, Env, SideData};
use crate::btl_hit::raise_first_clash;
use crate::btl_tech_fx::tech_damage;

/// `BtlClashC` (roster + 0x7C): read by the exchange action and the placement.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClashC {
    /// 1, or the story mode's difficulty (not ported).
    pub level: i32,
    pub count: i32,
    /// The stage path the pair is placed on (1 .. paths - 1) and its point.
    pub path: i32,
    pub point: i32,
    /// Exchanges left before the path changes.
    pub hold: i32,
    /// The button the exchange asks for (0..3: Circle, Cross, Triangle, Square), and the last one.
    pub pick: i32,
    pub prev_pick: i32,
}

/// `BtlClashA` (roster + 0x50).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClashA {
    pub timer: i32,
    pub winner: i32,
    /// > 0: fighter 0 is ahead.
    pub lead: i32,
    /// `clashBias`: the lead squashed into -0.8..0.8; where the struggle point is.
    pub bias: f32,
}

/// `BtlClash_CalcBias`: x / (|x| + 1) with x = lead / 20, at most 0.8 either way.
fn calc_bias(lead: i32) -> f32 {
    let x = lead as f32 * 0.05;
    (x / (x.abs() + 1.0)).clamp(-0.8, 0.8)
}

/// `gBtlChars->clashState` with the clash A, B and C blocks.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Clash {
    pub state: i32,
    pub a: ClashA,
    /// `EftStruggle_End` asked by clash A this frame: the winner, -1 both lose.
    pub struggle_end: Option<i32>,
    pub b_timer: i32,
    pub b_winner: i32,
    pub c: ClashC,
}

/// `BtlAct_CountAndMarkOpponent`: one more input counted, and a new hit on the opponent's combo.
pub(crate) fn count_and_mark(me: &mut Chr, opp: &mut Chr) {
    me.clash_count_b += 1;
    opp.combo.new_hit = true;
}

/// `BtlChar_Rand`: the fighters' generator, kept while time is stopped.
fn rand(state: &mut u32, time_stopped: bool) -> u32 {
    let next = ((u64::from(*state) * 714_025 + 4096) % 150_889) as u32;
    if !time_stopped {
        *state = next;
    }
    next
}

/// `a % b` with the original's unsigned division; a stage with too few paths would divide by zero there (the
/// remainder is then taken as `a`).
fn rem(a: u32, b: i32) -> i32 {
    match u32::try_from(b) {
        Ok(b) if b != 0 => (a % b) as i32,
        _ => a as i32,
    }
}

impl Clash {
    fn both_in(chars: &[Chr; 2], action: i32) -> bool {
        chars[0].act.current == action && chars[1].act.current == action
    }

    /// `BtlCharApi_IsInClashA`.
    pub(crate) fn in_clash_a(chr: &Chr) -> bool {
        (0x130..=0x132).contains(&chr.act.current)
    }

    /// `BtlClash_HoldClashA`: a fighter in clash A gets held flag 0xC3.
    fn hold_clash_a(chars: &mut [Chr; 2]) {
        for c in chars.iter_mut() {
            if Self::in_clash_a(c) {
                c.flags.set_held(0xC3);
            }
        }
    }

    /// `BtlClash_EndActionFA` / `EndActionFC`: a fighter still in the action gets the flag that ends it.
    fn end_action(chr: &mut Chr, action: i32, flag: u32) {
        if chr.act.current == action {
            chr.flags.set(flag);
        }
    }

    /// `BtlClash_Update`: starts a sequence when both fighters enter a clash action, then steps it. `frame` is the
    /// roster's frame counter (`BtlChar_FrameMod`).
    pub(crate) fn update(
        &mut self,
        chars: &mut [Chr; 2],
        data: [&SideData; 2],
        env: &Env,
        rand_state: &mut u32,
        frame: u32,
    ) {
        let both_a = |c: &[Chr; 2]| Self::in_clash_a(&c[0]) && Self::in_clash_a(&c[1]);
        if self.state == 0 {
            if both_a(chars) {
                self.state = 1;
            }
            if Self::both_in(chars, 0xFA) {
                self.state = 6;
            }
            if Self::both_in(chars, 0xFB) {
                self.state = 9;
            }
        }
        match self.state {
            0 => {
                Self::hold_clash_a(chars);
                for c in chars.iter_mut() {
                    Self::end_action(c, 0xFA, 0xBF);
                }
            }
            1..=5 => {
                self.state = self.update_a(chars, data, env, frame);
                if !both_a(chars) {
                    Self::hold_clash_a(chars);
                    self.state = 0;
                }
            }
            6..=8 => self.state = self.update_b(chars, frame),
            9..=11 => self.state = self.update_c(chars, env, rand_state),
            _ => {}
        }
    }

    /// `BtlClash_UpdateA`. The cameras, voices and streamed sounds are not ported.
    fn update_a(
        &mut self,
        chars: &mut [Chr; 2],
        data: [&SideData; 2],
        env: &Env,
        frame: u32,
    ) -> i32 {
        let (count0, count1) = (chars[0].clash_count_a, chars[1].clash_count_a);
        let a = &mut self.a;
        let mut state = self.state;
        match state {
            1 => {
                a.timer = 0;
                a.lead = 0;
                state += 1;
            }
            2 => {
                if a.timer >= 0x3C {
                    if count1 < count0 {
                        a.lead += 1;
                    }
                    if count0 < count1 {
                        a.lead -= 1;
                    }
                }
                a.timer += 1;
                if a.timer >= 0x6A {
                    a.timer = 0;
                    state += 1;
                }
            }
            3 => {
                if a.timer == 0 && a.lead == 0 {
                    a.lead = if frame % 2 != 0 { 1 } else { -1 };
                }
                a.timer += 1;
                if a.timer >= 0x10 {
                    a.timer = 0;
                    state += 1;
                }
            }
            4 => {
                a.lead += if a.lead > 0 { 4 } else { -4 };
                if a.lead.abs() >= 0x47 {
                    a.timer = 0;
                    state += 1;
                }
            }
            5 => {
                if a.timer == 0 {
                    let w = if a.lead > 0 || (a.lead == 0 && frame % 2 != 0) {
                        0
                    } else {
                        1
                    };
                    let l = w ^ 1;
                    let cls = crate::btl_act::id_class(chars[w].act.current);
                    let lcls = crate::btl_act::id_class(chars[l].act.current);
                    chars[w].flags.set_held(0xC1);
                    chars[w].flags.set_held(0xC3);
                    chars[l].flags.set_held(0xC2);
                    if data[w].tables.tech().is_throw(cls) {
                        // BtlColl_StartThrow of a technique is not ported.
                        chars[w].flags.set_held(0xA0);
                        self.struggle_end = Some(w as i32);
                    } else {
                        let mut dmg =
                            tech_damage(&chars[w], data[w], data[l], env, cls, false, true);
                        dmg += tech_damage(&chars[l], data[l], data[w], env, lcls, false, true) / 2;
                        let flags = match cls {
                            2 => 0x80_0000,
                            3 => 0x100_0000,
                            4 => 0x200_0000,
                            _ => 0,
                        };
                        chars[l].damage(env, data[l], dmg, flags);
                        self.struggle_end = Some(-1);
                    }
                    let (lo, hi) = chars.split_at_mut(1);
                    let (winner, loser) = if w == 0 {
                        (&mut lo[0], &mut hi[0])
                    } else {
                        (&mut hi[0], &mut lo[0])
                    };
                    raise_first_clash(winner, loser);
                    self.a.winner = w as i32;
                }
                self.a.lead = 0;
                state = 0;
            }
            _ => {}
        }
        self.a.bias = calc_bias(self.a.lead);
        state
    }

    /// `BtlClash_UpdateB`.
    fn update_b(&mut self, chars: &mut [Chr; 2], frame: u32) -> i32 {
        let mut state = self.state;
        match state {
            6 => {
                self.b_timer = 0;
                state = 7;
            }
            7 => {
                self.b_timer += 1;
                if self.b_timer >= 0x4C {
                    self.b_timer = 0;
                    state = 8;
                }
                if !Self::both_in(chars, 0xFA) {
                    state = 0;
                    for c in chars.iter_mut() {
                        Self::end_action(c, 0xFA, 0xBF);
                    }
                }
            }
            8 => {
                if self.b_timer == 0 {
                    let (a, b) = (chars[0].clash_count_b, chars[1].clash_count_b);
                    let win = if b < a {
                        0
                    } else if a < b {
                        1
                    } else {
                        usize::from(frame % 2 == 0)
                    };
                    let (lo, hi) = chars.split_at_mut(1);
                    let (winner, loser) = if win == 0 {
                        (&mut lo[0], &mut hi[0])
                    } else {
                        (&mut hi[0], &mut lo[0])
                    };
                    winner.flags.set(0xBF);
                    loser.flags.set(0xC0);
                    raise_first_clash(winner, loser);
                    self.b_winner = win as i32;
                }
                self.b_timer += 1;
                if !Self::both_in(chars, 0xFA) {
                    state = 0;
                }
            }
            _ => {}
        }
        state
    }

    /// `BtlClash_UpdateC`: every exchange moves the pair to a random point of a random stage path.
    fn update_c(&mut self, chars: &mut [Chr; 2], env: &Env, rand_state: &mut u32) -> i32 {
        let time_stopped = env.gate.time_stopped;
        let mut r = || rand(rand_state, time_stopped) >> 2;
        let paths = &env.stage.paths;
        let span = |n: i32| {
            usize::try_from(n)
                .ok()
                .and_then(|n| paths.get(n))
                .map(|p| (p.first, p.end))
        };
        let c = &mut self.c;
        let mut state = self.state;
        match state {
            9 => {
                c.count = 0;
                c.path = rem(r(), paths.len() as i32 - 1) + 1;
                c.hold = if rem(r(), 10) < 7 { 2 } else { 1 };
                c.point = match span(c.path) {
                    Some((first, end)) => rem(r(), end - first) + first,
                    None => 0,
                };
                c.prev_pick = -1;
                c.pick = (r() & 3) as i32;
                c.level = 1;
                state += 1;
            }
            10 => {
                if chars[0].flags.test(0xC4) && chars[1].flags.test(0xC4) {
                    let scorer = if chars[0].clash_answer_frame >= 0 {
                        0
                    } else {
                        1
                    };
                    let (lo, hi) = chars.split_at_mut(1);
                    let (me, opp) = if scorer == 0 {
                        (&mut lo[0], &mut hi[0])
                    } else {
                        (&mut hi[0], &mut lo[0])
                    };
                    count_and_mark(me, opp);
                    c.count += 1;
                    let (a, b) = (chars[0].clash_count_b, chars[1].clash_count_b);
                    if c.count >= 7 && a != b {
                        let (w, l) = if b < a { (0, 1) } else { (1, 0) };
                        chars[w].flags.set(0xC7);
                        state += 1;
                        chars[l].flags.set(0xC8);
                    } else {
                        c.hold -= 1;
                        if c.hold <= 0 {
                            let prev = c.path;
                            let n = rem(r(), paths.len() as i32 - 2);
                            c.path = n + 1;
                            if c.path >= prev {
                                c.path = n + 2;
                            }
                            c.hold = if rem(r(), 10) < 7 { 2 } else { 1 };
                            c.point = 0;
                            if let Some((first, end)) = span(c.path) {
                                c.point = rem(r(), end - first) + first;
                            }
                        } else {
                            match span(c.path) {
                                Some((first, end)) => {
                                    let prev = c.point;
                                    c.point = rem(r(), end - first - 1) + first;
                                    if c.point >= prev {
                                        c.point += 1;
                                    }
                                }
                                None => c.point = 0,
                            }
                        }
                        c.prev_pick = c.pick;
                        c.pick = (r() & 3) as i32;
                        chars[0].flags.set(0xC5);
                        chars[1].flags.set(0xC5);
                    }
                }
                if !Self::both_in(chars, 0xFB) && !Self::both_in(chars, 0xFC) {
                    state = 0;
                    for ch in chars.iter_mut() {
                        Self::end_action(ch, 0xFC, 0xC6);
                    }
                }
            }
            11 => {
                if !Self::both_in(chars, 0xFC) {
                    state = 0;
                }
            }
            _ => {}
        }
        state
    }
}

/// `BtlChar_PlaceOnPath` (flag 0xF8): beside the clash's point of its stage path, both radii plus 30 off the line
/// from the path's first point, player 0 on one side facing across and the other opposite.
pub(crate) fn place_on_path(me: &mut Chr, data: &SideData, env: &Env, c: &ClashC) {
    if !me.flags.test(0xF8) {
        return;
    }
    let Some(path) = usize::try_from(c.path)
        .ok()
        .and_then(|n| env.stage.paths.get(n))
    else {
        return;
    };
    let index = rem(c.point as u32, path.end - path.first) + path.first;
    let (Some(first), Some(point)) = (
        path.points.first(),
        usize::try_from(index).ok().and_then(|i| path.points.get(i)),
    ) else {
        return;
    };
    let delta = [
        point[0] - first[0],
        point[1] - first[1],
        point[2] - first[2],
    ];
    let side = [delta[2], 0.0, -delta[0]];
    let len = crate::btl_chr::length(side);
    if len < 0.001 {
        return;
    }
    let k = (data.radius() + 30.0) / len;
    let side = side.map(|v| v * k);
    let mut pos = [point[0], point[1] + data.center_height(), point[2]];
    let mut rot = [0.0; 3];
    if me.player == 0 {
        pos = crate::btl_chr::sub(pos, side);
        rot[1] = side[0].atan2(side[2]);
    } else {
        pos = crate::btl_chr::add(pos, side);
        rot[1] = (-side[0]).atan2(-side[2]);
    }
    let area = env.col.find_zone_near(-1, pos) as i32;
    me.place(data, pos, rot, area);
}
