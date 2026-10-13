//! Sim Dragon's event scripts (`gSimEvent`, src/menu/sim_event*.c): what a turn of the board turns into. A script
//! runs one step a frame while the board waits on it (`seq` its step, `seq_timer` the step's frames); a step that
//! sets the key wait holds it until the confirm button, and the board's commands (`SimDay_Cmd`) play its pictures,
//! choices and stat changes. Scripts 0..2 are the three trainings, 3 the rest, 4 the full heal, 5..36 the random
//! events; three of these are mini games: the cards (28), the rocks (32) and the shells (34).

use crate::flash::Uv;
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::sim_day::{
    ATK, DEF, FL_CARD, FL_POPO, FL_SHOT, HP, LEVEL_UP, POINT, ST_SCRIPT, SimDayScreen, TRAIN_BEST,
    TRAIN_WORST, TRAINS, WAIT_KEY,
};
use crate::title::{PAD_CROSS, PAD_LEFT, PAD_RIGHT, TitleSound};

/// `SIMEV_FLAG200`: five turns pass at once (`SIMDAY_SKIP_TURNS`).
const SKIP_TURNS: u32 = 0x200;
/// `SIMDAY_SAME_TURN`: the board comes back without a turn passing.
const SAME_TURN: u32 = 0x100;
/// The mini games' music (`StreamSe_PlayDefault(0, 0x10BE1)`).
const GAME_MUSIC: u32 = 0x10BE1;

// SimDay_Cmd numbers of the scripts.
const MONITOR_OFF: i32 = 5;
const CHARA_IN: i32 = 6;
const CHARA_OUT: i32 = 7;
const SELECT: i32 = 17;
const SURPRISE: i32 = 25;
const SHOW_CHANGE: i32 = 26;
const LEVEL_UP_CMD: i32 = 33;

/// `sSimPopoX`, `sSimPopoSpeed`: the shells' places and the speeds of a swap.
const POPO_X: [i32; 5] = [76, 150, 224, 298, 372];
const POPO_SPEED: [i32; 3] = [6, 10, 14];

/// Pad 0 as the scripts read it.
#[derive(Clone, Copy, Debug, Default)]
pub struct EventPad {
    pub pressed: u32,
    pub repeat: u32,
}

/// The scripts' state kept in the original's globals (`gSimTrainOutcome*`, `gSimCard*`, `gSimRock*`,
/// `gSimPopo*`): it outlives the board screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimGames {
    train_outcome: [i32; 3],
    card_shown: [i32; 10],
    card_order: [i32; 10],
    card_next: i32,
    card_rank: i32,
    card_picked: u8,
    card_wrong: u8,
    rock_x: i32,
    rock_speed: i32,
    rock_left: i32,
    rock_hits: i32,
    rock_flags: u8,
    rock_rank: i32,
    popo_target: i32,
    popo_state: i32,
    popo_dx: i32,
    popo_dy: i32,
    popo_speed: i32,
    popo_a: i32,
    popo_b: i32,
    popo_count: i32,
    popo_won: i32,
    popo_rank: i32,
}

fn uv(u0: i32, v0: i32, u1: i32, v1: i32) -> Uv {
    Uv {
        u0,
        v0,
        u1,
        v1,
        tex: 0,
    }
}

fn train_of(kind: usize, rank: i32) -> crate::sim_day::Train {
    TRAINS[kind][rank.clamp(0, 4) as usize]
}

impl SimDayScreen {
    fn next(&mut self) {
        self.seq += 1;
        self.seq_timer = 0;
    }

    fn goto(&mut self, step: i32) {
        self.seq = step;
        self.seq_timer = 0;
    }

    fn key(&mut self) {
        self.flags |= WAIT_KEY;
    }

    /// A line held until the confirm button, then the next step.
    fn say(&mut self, line: i32) {
        self.msg_line = line;
        self.key();
        self.next();
    }

    /// A line held until the confirm button, then step `step`.
    fn say_goto(&mut self, line: i32, step: i32) {
        self.msg_line = line;
        self.key();
        self.goto(step);
    }

    /// A line with the stat changes `changes` shown, held until the confirm button.
    fn show(&mut self, line: i32, changes: &[(usize, i32)], random: &mut GameRandom) {
        self.msg_line = line;
        for &(stat, amount) in changes {
            self.add_change(stat, amount);
        }
        self.cmd(SHOW_CHANGE, random);
        self.key();
    }

    /// The turn's price (line 0x63): attack and defence lose 0..2 each.
    fn penalty(&mut self, random: &mut GameRandom) {
        let atk = -(random.range(3) as i32);
        let def = -(random.range(3) as i32);
        self.show(0x63, &[(ATK, atk), (DEF, def)], random);
        self.next();
    }

    /// The monitor goes out and comes back on picture `a` (with `b` the character for the next `CHARA_IN`).
    fn faces(&mut self, a: i32, b: i32, random: &mut GameRandom) {
        self.msg_line = -1;
        self.face_a = a;
        self.face_b = b;
        self.cmd(MONITOR_OFF, random);
        self.next();
    }

    /// The opening of the random events: the monitor out on picture 11, "!?" (line 0x62), the surprise icon.
    fn opening(&mut self, random: &mut GameRandom) {
        match self.seq {
            0 => {
                self.face_a = 11;
                self.cmd(MONITOR_OFF, random);
                self.next();
            }
            1 => {
                self.msg_line = 0x62;
                self.se(0x17);
                self.key();
                self.next();
            }
            _ => {
                self.msg_line = -1;
                self.cmd(SURPRISE, random);
                self.next();
            }
        }
    }

    fn chara_out(&mut self, random: &mut GameRandom) {
        self.msg_line = -1;
        self.cmd(CHARA_OUT, random);
        self.next();
    }

    fn select(&mut self, line: i32, random: &mut GameRandom) {
        self.msg_line = line;
        self.cmd(SELECT, random);
        self.next();
    }

    fn done(&mut self) -> bool {
        self.msg_line = -1;
        true
    }

    /// `SimEvent_Run`: one step of script `event`; true when it is over (also for a number past the table).
    pub(crate) fn run_event(
        &mut self,
        pad: EventPad,
        save: &mut SaveData,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
    ) -> bool {
        let r = random;
        match self.event {
            0..=2 => self.ev_train(self.event as usize, r),
            3 => self.ev_rest(r),
            4 => self.ev_heal(r),
            5 => self.ev_nothing(r),
            6 => self.ev_points(r),
            7 => self.ev_gift(r),
            8..=11 => self.ev_kame(self.event, r),
            12 => self.ev_work(r),
            13..=17 => self.ev_deal(self.event, save, r),
            18 => self.ev_nurse(r),
            19..=22 => self.ev_tip(0x86 + self.event - 19, r),
            23 | 24 => self.ev_sale(self.event == 23, r),
            25..=27 => self.ev_ask(self.event, r),
            28 => self.ev_cards(pad, r, libc),
            29 => self.ev_level_up(r),
            30 => self.ev_offer_a(r),
            31 => self.ev_offer_b(r),
            32 => self.ev_rocks(pad, r),
            33 => self.ev_power(r),
            34 => self.ev_shells(pad, r),
            35 => self.ev_robbery(r),
            36 => self.ev_present(r),
            _ => true,
        }
    }

    /// `SimEv00_Roll`..: the outcome (0..5) of a training, one draw of 0..100 against the weights of the times
    /// it was repeated (or the outcome a bought flag sets).
    fn train_roll(&self, kind: usize, random: &mut GameRandom) -> i32 {
        let r = random.range(101) as i32;
        if self.flags & TRAIN_BEST != 0 {
            return 5;
        }
        if self.flags & TRAIN_WORST != 0 {
            return 0;
        }
        let table = self.data.trains.get(kind).copied().unwrap_or_default();
        let weight = table.weight[self.repeat.clamp(0, 2) as usize];
        let mut sum = 0;
        for (i, w) in weight.iter().enumerate() {
            sum += w;
            if r < sum {
                return i as i32;
            }
        }
        5
    }

    /// `SimEv00`..`SimEv02`: a training: picture N, line 0x59 with one of two sounds, the outcome's line and its
    /// changes of attack, defence and health.
    fn ev_train(&mut self, kind: usize, r: &mut GameRandom) -> bool {
        match self.seq {
            0 => {
                self.games.train_outcome[kind] = self.train_roll(kind, r);
                self.face_a = kind as i32 + 1;
                self.cmd(MONITOR_OFF, r);
                self.next();
            }
            1 => {
                self.msg_line = 0x59;
                let se = if r.range(2) != 0 { 0x15 } else { 0x16 };
                self.se(se);
                self.key();
                self.next();
            }
            2 => {
                let outcome = self.games.train_outcome[kind].clamp(0, 5);
                let t = self.data.trains.get(kind).copied().unwrap_or_default();
                let o = outcome as usize;
                self.msg_line = outcome + 0x5A;
                let pick = |min: i32, max: i32, r: &mut GameRandom| {
                    if min == max {
                        min
                    } else {
                        r.range((max - min + 1) as u32) as i32 + min
                    }
                };
                let atk = pick(t.atk_min[o], t.atk_max[o], r);
                self.add_change(ATK, atk);
                let def = pick(t.def_min[o], t.def_max[o], r);
                self.add_change(DEF, def);
                self.add_change(HP, t.hp[o]);
                self.cmd(SHOW_CHANGE, r);
                self.key();
                self.next();
            }
            3 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv03`, board row 2: after 61 frames half the health + 5 comes back (line 0x60), then attack and defence
    /// lose 1 each.
    fn ev_rest(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0 => {
                self.face_a = 10;
                self.cmd(MONITOR_OFF, r);
                self.next();
            }
            1 => {
                self.se(0x18);
                self.next();
            }
            2 => {
                self.seq_timer += 1;
                if self.seq_timer > 60 {
                    let hp = self.stat(HP) / 2 + 5;
                    self.show(0x60, &[(HP, hp)], r);
                    self.next();
                }
            }
            3 => {
                let atk = !r.range(1) as i32;
                let def = !r.range(1) as i32;
                self.show(0x63, &[(ATK, atk), (DEF, def)], r);
                self.next();
            }
            4 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv04`, board row 3: health back to 100 (line 0x61).
    fn ev_heal(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0 => {
                self.face_a = 10;
                self.cmd(MONITOR_OFF, r);
                self.next();
            }
            1 => {
                let hp = 100 - self.stat(HP);
                self.show(0x61, &[(HP, hp)], r);
                self.next();
            }
            2 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv05`: nothing is found (line 0x64).
    fn ev_nothing(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0 | 1 => self.opening(r),
            2 => self.say(0x64),
            3 => self.penalty(r),
            4 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv06`: a character (a random picture) gives `Rand_Range(turn) * 100 + 100` points.
    fn ev_points(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => {
                self.face_a = r.range(12) as i32;
                self.face_b = 11;
                self.cmd(MONITOR_OFF, r);
                self.next();
            }
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0x65);
            }
            5 => {
                let points = r.range(self.run.turn as u32) as i32 * 100 + 100;
                self.show(0x66, &[(POINT, points)], r);
                self.flags &= !WAIT_KEY;
                self.next();
            }
            6 => {
                self.next();
                self.key();
            }
            7 => self.chara_out(r),
            8 => self.penalty(r),
            9 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv07`: a character offers an item; yes hands one out.
    fn ev_gift(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => {
                self.face_a = r.range(12) as i32;
                self.face_b = 10;
                self.cmd(MONITOR_OFF, r);
                self.next();
            }
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0x67);
            }
            5 => self.select(0x68, r),
            6 => {
                if self.answer != 0 {
                    self.next();
                } else {
                    self.cmd(CHARA_OUT, r);
                    self.goto(8);
                }
            }
            7 => {
                self.cmd(CHARA_OUT, r);
                self.give_item(r);
                self.say(0x69);
            }
            8 => self.penalty(r),
            9 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv08`..`SimEv11`: pictures 0 / 7; a fifth of the health comes back, then (9..11) attack, defence or
    /// both + 1..5 without the turn's price.
    fn ev_kame(&mut self, event: i32, r: &mut GameRandom) -> bool {
        match self.seq {
            0 | 1 => self.opening(r),
            2 => self.faces(0, 7, r),
            3 => {
                self.cmd(CHARA_IN, r);
                self.say(0x6A);
            }
            4 => self.say(0x6B),
            5 => {
                let hp = self.stat(HP) / 5;
                self.show(0x6C, &[(HP, hp)], r);
                self.next();
            }
            6 if event == 8 => self.say(0x6E),
            6 => {
                let mut changes = Vec::new();
                if event != 10 {
                    changes.push((ATK, r.range(5) as i32 + 1));
                }
                if event != 9 {
                    changes.push((DEF, r.range(5) as i32 + 1));
                }
                self.show(0x6D, &changes, r);
                self.next();
            }
            7 if event == 8 => self.chara_out(r),
            7 => self.say(0x6E),
            8 if event == 8 => self.penalty(r),
            8 => return self.done(),
            9 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv12`: pictures 0 / 7; costs 5..20 % of the health, pays 1000..5000 points.
    fn ev_work(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(0, 7, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0x6A);
            }
            5 => self.say(0x6F),
            6 => self.say(0x70),
            7 => {
                let hp = -(self.stat(HP) * (r.range(16) as i32 + 5) / 100);
                self.show(0x71, &[(HP, hp)], r);
                self.next();
            }
            8 => {
                let points = r.range(4001) as i32 + 1000;
                self.show(0x66, &[(POINT, points)], r);
                self.next();
            }
            9 => self.say(0x72),
            10 => self.chara_out(r),
            11 => self.penalty(r),
            12 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv13`..`SimEv17`: pictures 4 / 6 offer a deal for 10000 points (line 0xD7 when short of them): nothing
    /// (13), 30000 points (14), attack and defence + 5 without the turn's price (15), an item (16), one more
    /// mission page (17).
    fn ev_deal(&mut self, event: i32, save: &mut SaveData, r: &mut GameRandom) -> bool {
        // The steps of the closing lines: short of points, declined, the character leaves.
        let (short, declined) = match event {
            13 => (13, 14),
            16 => (17, 18),
            _ => (14, 15),
        };
        let leave = declined + 1;
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(4, 6, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0x73);
            }
            5 => self.say(0x74),
            6 => self.select(0x75, r),
            7 => {
                self.msg_line = -1;
                if self.answer == 0 {
                    self.goto(declined);
                } else {
                    self.next();
                }
            }
            8 => {
                if self.stat(POINT) < 10000 {
                    self.goto(short);
                } else {
                    self.next();
                }
            }
            9 => {
                self.show(0x76, &[(POINT, -10000)], r);
                self.next();
            }
            10 => {
                self.se(0x1D);
                self.say(0x78);
            }
            11 => {
                let (se, line) = match event {
                    13 => (0x22, 0x79),
                    14 => (0x1F, 0x81),
                    15 => (0x1F, 0x7E),
                    16 => (0x1F, 0x80),
                    _ => (0x1F, 0x7B),
                };
                self.se(se);
                self.say(line);
            }
            12 => match event {
                13 => self.say_goto(0x7A, leave),
                14 => {
                    self.show(0x66, &[(POINT, 30000)], r);
                    self.next();
                }
                15 => {
                    self.show(0x7F, &[(ATK, 5), (DEF, 5)], r);
                    self.next();
                }
                16 => self.select(0x68, r),
                _ => {
                    save.set_mission_pages(save.mission_pages() + 1);
                    self.say(0x7C);
                }
            },
            13 if event == 13 => self.say(0xD7),
            13 if event == 16 => {
                if self.answer == 0 {
                    self.goto(16);
                } else {
                    self.next();
                }
            }
            // 15 skips the turn's price.
            13 => self.say_goto(0x7D, if event == 15 { 18 } else { leave }),
            14 if event == 16 => {
                self.give_item(r);
                self.say(0x69);
            }
            15 | 16 if event == 16 => self.say_goto(0x7D, leave),
            n if n == short => self.say(0xD7),
            n if n == declined => self.say(0x77),
            n if n == leave => self.chara_out(r),
            n if n == leave + 1 => self.penalty(r),
            n if n == leave + 2 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv18`: picture 5; health back to 100.
    fn ev_nurse(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(5, 5, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0x82);
            }
            5 => self.say(0x83),
            6 => {
                let hp = 100 - self.stat(HP);
                self.show(0x84, &[(HP, hp)], r);
                self.next();
            }
            7 => self.say(0x85),
            8 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv19`..`SimEv22`: picture 5 pays 300 points (line 0x86..0x89).
    fn ev_tip(&mut self, line: i32, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(5, 5, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0x82);
            }
            5 => self.say(line),
            6 => {
                self.show(0x66, &[(POINT, 300)], r);
                self.next();
            }
            7 => self.say(0x85),
            8 => self.chara_out(r),
            9 => self.penalty(r),
            10 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv23` / `SimEv24`: pictures 6 / 8 sell, for 20000 points, the best (23) or the worst (24) outcome of
    /// the trainings from now on.
    fn ev_sale(&mut self, best: bool, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(6, 8, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0x8A);
            }
            5 => self.say(0x8B),
            6 => self.select(0x8C, r),
            7 => {
                if self.answer != 0 {
                    self.next();
                } else {
                    self.goto(13);
                }
            }
            8 => {
                if self.stat(POINT) < 20000 {
                    self.goto(12);
                } else {
                    self.next();
                }
            }
            9 => {
                self.show(0x8E, &[(POINT, -20000)], r);
                self.next();
            }
            10 => {
                if best {
                    self.flags = (self.flags & !TRAIN_WORST) | TRAIN_BEST;
                } else {
                    self.flags = (self.flags & !TRAIN_BEST) | TRAIN_WORST;
                }
                self.se(0x1F);
                self.say(if best { 0x8F } else { 0x91 });
            }
            11 => self.say_goto(0x90, 14),
            12 => self.say(0xD7),
            13 => self.say(0x8D),
            14 => self.chara_out(r),
            15 => self.penalty(r),
            16 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv25`..`SimEv27`: picture 1 asks; yes = 500 points (25), half the health for 500 points (26), half the
    /// health for attack and defence + 15 and 700 points (27).
    fn ev_ask(&mut self, event: i32, r: &mut GameRandom) -> bool {
        let declined = if event == 26 { 12 } else { 13 };
        let leave = declined + 1;
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(1, 1, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0x92);
            }
            5 => self.say(0x93),
            6 => self.select(0x94, r),
            7 => {
                if self.answer != 0 {
                    self.next();
                } else {
                    self.goto(declined);
                }
            }
            8 => self.say(0x96),
            9 => self.say(0x97),
            10 => self.say(match event {
                25 => 0x98,
                26 => 0x9A,
                _ => 0x9B,
            }),
            11 if event == 25 => self.say(0x99),
            11 if event == 26 => {
                let hp = -(self.stat(HP) / 2);
                self.show(0x9D, &[(HP, hp), (POINT, 500)], r);
                self.goto(13);
            }
            11 => {
                let hp = -(self.stat(HP) / 2);
                self.show(0x9C, &[(ATK, 15), (DEF, 15), (HP, hp), (POINT, 700)], r);
                self.next();
            }
            12 if event == 25 => {
                self.show(0x9D, &[(POINT, 500)], r);
                self.goto(14);
            }
            12 if event == 27 => self.say_goto(0x9D, 16),
            n if n == declined => self.say(0x95),
            n if n == leave => self.chara_out(r),
            n if n == leave + 1 => self.penalty(r),
            n if n == leave + 2 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv28_SelectCard` / `SimEv28_UnselectCard`.
    fn card_goto(&mut self, label: &str) {
        let f = &mut self.flash[FL_CARD];
        let r = f.find(None, &format!("mc_card_anime{:02}", self.cur[ST_SCRIPT]));
        f.clip_goto_label(r, label);
    }

    /// `SimEv28_MoveCursor`: the next place left or right of the cursor whose card was not picked, wrapping.
    fn card_move(&self, right: bool) -> Option<i32> {
        let n = train_of(0, self.games.card_rank).param - 1;
        let mut cur = self.cur[self.state];
        for _ in 0..n {
            if right {
                cur += 1;
                if cur > n {
                    cur = 0;
                }
            } else {
                cur -= 1;
                if cur < 0 {
                    cur = n;
                }
            }
            if self.games.card_picked >> cur & 1 == 0 {
                return Some(cur);
            }
        }
        None
    }

    /// `SimEv28_Deal`: shuffles 0..9 with libc `rand`, deals the first `n` and sorts them for the order to pick in.
    fn card_deal(&mut self, n: usize, libc: &mut LibcRandom) {
        let g = &mut self.games;
        for i in (0..10).rev() {
            g.card_order[i] = i as i32;
        }
        for i in 0..10 {
            let r = (libc.rand() % 10) as usize;
            g.card_order.swap(i, r);
        }
        let n = n.min(10);
        g.card_shown[..n].copy_from_slice(&g.card_order[..n]);
        for i in 0..n {
            for j in (i + 1..n).rev() {
                if g.card_order[j] < g.card_order[j - 1] {
                    g.card_order.swap(j, j - 1);
                }
            }
        }
        g.card_next = 0;
        g.card_picked = 0;
        g.card_wrong = 0;
    }

    /// `SimEv28`: picture 1 asks; yes = the card game at the rank of the first training. The cards lie face up
    /// for a while, then are picked one by one, each the lowest one left; all right pays points and attack, a wrong
    /// pick costs attack. Every call also sets the cards' pictures and the crosses of wrong picks.
    fn ev_cards(&mut self, pad: EventPad, r: &mut GameRandom, libc: &mut LibcRandom) -> bool {
        let ok = pad.pressed & PAD_CROSS != 0;
        let left = pad.repeat & PAD_LEFT != 0;
        let right = pad.repeat & PAD_RIGHT != 0;
        let trig = self.flash[FL_CARD].trig();
        let t = train_of(0, self.games.card_rank);
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(1, 1, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0x92);
            }
            5 => self.select(0x9E, r),
            6 => {
                self.msg_line = -1;
                if self.answer == 0 {
                    self.goto(23);
                } else {
                    self.games.card_rank = self.rank[0];
                    self.next();
                }
            }
            7 => self.say(0xA0),
            8 => {
                self.sounds.push(TitleSound::StreamSe(GAME_MUSIC));
                self.sounds.push(TitleSound::BgmStop);
                let n = train_of(0, self.games.card_rank).param.max(0) as usize;
                self.card_deal(n, libc);
                self.msg_line = 0xA1;
                // The card movie starts.
                self.cmd(0x24, r);
                self.next();
            }
            9 => {
                if trig & 1 != 0 {
                    self.next();
                }
            }
            10 => {
                // The cards turn face up.
                self.cmd(0x25, r);
                self.se(4);
                self.next();
            }
            11 => {
                if trig & 8 != 0 {
                    self.next();
                }
            }
            12 => {
                self.seq_timer += 1;
                if self.seq_timer > t.speed * 60 {
                    self.next();
                }
            }
            13 => {
                // And face down again.
                self.cmd(0x26, r);
                self.se(4);
                self.next();
            }
            14 => {
                if trig & 0x10 != 0 {
                    self.card_goto("fl_select_in");
                    self.next();
                }
            }
            15 => {
                let g = self.games;
                if g.card_next == t.param || g.card_next == -1 {
                    self.seq_timer += 1;
                    if self.seq_timer > 60 {
                        self.goto(17);
                    }
                } else if ok {
                    let cur = self.cur[self.state].clamp(0, 9);
                    self.games.card_picked |= 1 << cur;
                    // The picked card turns over.
                    self.cmd(0x27, r);
                    self.se(4);
                    let g = &mut self.games;
                    if g.card_shown[cur as usize] == g.card_order[g.card_next.clamp(0, 9) as usize]
                    {
                        g.card_next += 1;
                    } else {
                        g.card_next = -1;
                        g.card_wrong |= 1 << cur;
                    }
                    self.next();
                } else if left || right {
                    if let Some(cur) = self.card_move(right) {
                        self.card_goto("fl_off");
                        self.cur[self.state] = cur;
                        self.card_goto("fl_select_in");
                    }
                }
            }
            16 => {
                if trig & 4 != 0 {
                    if self.games.card_next != -1
                        && let Some(place) = self.card_move(true)
                    {
                        self.cur[self.state] = place;
                        self.card_goto("fl_select_in");
                    }
                    self.goto(15);
                }
            }
            17 => {
                if self.games.card_wrong != 0 {
                    self.cmd(0x29, r);
                    self.se(0x20);
                } else {
                    self.cmd(0x28, r);
                    self.se(0x1F);
                }
                self.next();
            }
            18 => {
                if trig & 2 != 0 {
                    self.sounds.push(TitleSound::StreamSeStop(0));
                    self.play_bgm();
                    // The cards leave.
                    self.cmd(0x2A, r);
                    self.next();
                }
            }
            19 => {
                if trig & 0x20 != 0 {
                    // The card movie stops.
                    self.cmd(0x2B, r);
                    if self.games.card_wrong != 0 {
                        self.goto(21);
                    } else {
                        self.next();
                    }
                }
            }
            20 => {
                self.show(0xA2, &[(POINT, t.points), (ATK, t.gain)], r);
                self.goto(22);
            }
            21 => {
                self.show(0xA3, &[(ATK, t.loss)], r);
                self.next();
            }
            22 => self.say_goto(0x9D, 26),
            23 => self.say(0x9F),
            24 => self.chara_out(r),
            25 => self.penalty(r),
            26 => return self.done(),
            _ => {}
        }
        let g = self.games;
        let f = &mut self.flash[FL_CARD];
        for i in 0..train_of(0, g.card_rank).param.clamp(0, 10) as usize {
            let name = format!("mc_card_anime{i:02}");
            let x = g.card_shown[i] % 4 * 64;
            let y = g.card_shown[i] / 4 * 64;
            let c = f.find(Some(&name), "mc_card_num");
            f.clip_set_uv(c, uv(x, y, x + 64, y + 64));
            if g.card_wrong >> i & 1 != 0 {
                let c = f.find(Some(&name), "mc_maru_batsu");
                f.clip_set_uv(c, uv(64, 0, 128, 64));
            }
        }
        let c = f.find(None, "mc_lose_text");
        f.clip_set_uv(c, uv(0, 0x80, 0x200, 0x100));
        if f.se() & 1 != 0 {
            self.se(0x3C);
        }
        false
    }

    /// `SimEv29`: pictures 8 / 9 ask; yes = attack and defence + 15, a level-up, and five turns pass.
    fn ev_level_up(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(8, 9, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0xA4);
            }
            5 => self.say(0xA5),
            6 => self.say(0xA6),
            7 => self.select(0xA7, r),
            8 => {
                self.msg_line = -1;
                if self.answer != 0 {
                    self.next();
                } else {
                    self.goto(18);
                }
            }
            9 => self.say(0xA9),
            10 => self.say(0xAA),
            11 => self.say(0xAB),
            12 | 13 => self.say(0x98),
            14 => self.say(0xAC),
            15 => {
                self.flags |= SKIP_TURNS | LEVEL_UP;
                self.show(0xAD, &[(ATK, 15), (DEF, 15)], r);
                self.flags &= !WAIT_KEY;
                self.next();
            }
            16 => {
                self.cmd(LEVEL_UP_CMD, r);
                self.key();
                self.next();
            }
            17 => self.say_goto(0xAE, 21),
            18 => self.say(0xA8),
            19 => self.chara_out(r),
            20 => self.penalty(r),
            21 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv30`: pictures 6 / 3 ask; yes = picture 2 comes and defence - 5 (no turn's price); no = 1000 points.
    fn ev_offer_a(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(6, 3, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0xAF);
            }
            5 => self.select(0xB0, r),
            6 => {
                self.msg_line = -1;
                if self.answer != 0 {
                    self.next();
                } else {
                    self.goto(13);
                }
            }
            7 => self.say(0xB1),
            8 => {
                self.cmd(CHARA_OUT, r);
                self.next();
            }
            9 => {
                self.face_b = 2;
                self.cmd(CHARA_IN, r);
                self.next();
            }
            10 => self.say(0xB2),
            11 => {
                self.show(0xB3, &[(DEF, -5)], r);
                self.next();
            }
            12 => self.say_goto(0xB4, 17),
            13 => self.say(0xB5),
            14 => {
                self.show(0xB6, &[(POINT, 1000)], r);
                self.next();
            }
            15 => self.chara_out(r),
            16 => self.penalty(r),
            17 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv31`: the same offer the other way round: yes = 1000 points; no = picture 2 and defence - 5.
    fn ev_offer_b(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(6, 3, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0xAF);
            }
            5 => self.select(0xB0, r),
            6 => {
                self.msg_line = -1;
                if self.answer != 0 {
                    self.next();
                } else {
                    self.goto(11);
                }
            }
            7 => self.say(0xB5),
            8 => {
                self.show(0xB6, &[(POINT, 1000)], r);
                self.next();
            }
            9 => self.chara_out(r),
            10 => {
                self.penalty(r);
                self.goto(17);
            }
            11 => self.say(0xB1),
            12 => {
                self.cmd(CHARA_OUT, r);
                self.next();
            }
            13 => {
                self.face_b = 2;
                self.cmd(CHARA_IN, r);
                self.next();
            }
            14 => self.say(0xB2),
            15 => {
                self.show(0xB3, &[(DEF, -5)], r);
                self.next();
            }
            16 => self.say(0xB4),
            17 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv32`: pictures 2 / 0; yes = the rock game at the rank of the second training. The rocks cross the
    /// screen one after another; confirm swings, and a swing whose hit frame finds the rock at x -412..-342
    /// breaks it. All broken pays points and attack, otherwise attack is lost. Every call places the rock.
    fn ev_rocks(&mut self, pad: EventPad, r: &mut GameRandom) -> bool {
        let ok = pad.pressed & PAD_CROSS != 0;
        let trig = self.flash[FL_SHOT].trig();
        let t = train_of(1, self.games.rock_rank);
        let mut done = false;
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(2, 0, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0xB8);
            }
            5 => self.say(0xB9),
            6 => self.select(0xBA, r),
            7 => {
                self.msg_line = -1;
                if self.answer == 0 {
                    self.goto(19);
                } else {
                    let g = &mut self.games;
                    g.rock_rank = self.rank[1];
                    let t = train_of(1, g.rock_rank);
                    g.rock_speed = t.speed * 3 + 3;
                    g.rock_left = t.param;
                    g.rock_hits = 0;
                    g.rock_x = 0;
                    g.rock_flags = 0;
                    self.next();
                }
            }
            8 => self.say(0xBC),
            9 => {
                self.sounds.push(TitleSound::StreamSe(GAME_MUSIC));
                self.sounds.push(TitleSound::BgmStop);
                self.msg_line = 0xBD;
                self.cmd(0x37, r);
                self.next();
            }
            10 => {
                if trig & 1 != 0 {
                    self.next();
                }
            }
            11 => {
                if self.games.rock_flags & 1 == 0 && ok {
                    self.cmd(0x38, r);
                    self.games.rock_flags |= 1;
                    self.se(0x21);
                }
                if trig & 0x20 != 0 {
                    self.games.rock_flags &= !1;
                }
                if trig & 0x10 != 0 && (-412..=-342).contains(&self.games.rock_x) {
                    self.cmd(0x39, r);
                    self.next();
                    self.games.rock_flags &= !1;
                    self.se(0x22);
                    self.games.rock_hits += 1;
                }
                let g = &mut self.games;
                g.rock_x -= g.rock_speed;
                if g.rock_x < -700 {
                    let over = g.rock_left == 0 || {
                        g.rock_x = 0;
                        g.rock_left -= 1;
                        g.rock_left == 0
                    };
                    if over {
                        self.goto(13);
                    }
                }
            }
            12 => {
                if trig & 4 != 0 {
                    let g = &mut self.games;
                    g.rock_x = 0;
                    g.rock_left -= 1;
                    if g.rock_left == 0 {
                        if g.rock_hits == t.param {
                            g.rock_flags |= 2;
                        }
                        self.goto(13);
                    } else {
                        self.cmd(0x3A, r);
                        self.goto(11);
                    }
                }
            }
            13 => {
                if self.games.rock_flags & 2 != 0 {
                    self.cmd(0x3B, r);
                    self.se(0x1F);
                } else {
                    self.cmd(0x3C, r);
                    self.se(0x20);
                }
                self.next();
            }
            14 => {
                if trig & 2 != 0 {
                    self.cmd(0x3D, r);
                    self.next();
                }
            }
            15 => {
                if trig & 8 != 0 {
                    self.sounds.push(TitleSound::StreamSeStop(0));
                    self.play_bgm();
                    self.cmd(0x3E, r);
                    if self.games.rock_flags & 2 != 0 {
                        self.next();
                    } else {
                        self.goto(17);
                    }
                }
            }
            16 => {
                self.show(0xBE, &[(ATK, t.gain), (POINT, t.points)], r);
                self.goto(18);
            }
            17 => {
                self.show(0xC0, &[(ATK, t.loss)], r);
                self.next();
            }
            18 => self.say_goto(0xBF, 22),
            19 => self.say(0xBB),
            20 => self.chara_out(r),
            21 => self.penalty(r),
            22 => done = self.done(),
            _ => {}
        }
        let x = self.games.rock_x;
        let f = &mut self.flash[FL_SHOT];
        let c = f.find(None, "mc_rock00");
        f.clip_set_offset(c, x, 0);
        let c = f.find(None, "mc_lose_text");
        f.clip_set_uv(c, uv(0, 0x80, 0x200, 0x100));
        let c = f.find(None, "mc_count_num00");
        f.clip_set_uv(c, uv(0x100, 0, 0x180, 0x80));
        let c = f.find(None, "mc_count_num01");
        f.clip_set_uv(c, uv(0x80, 0, 0x100, 0x80));
        let se = f.se();
        if se & 1 != 0 {
            self.se(0x3B);
        }
        if se & 2 != 0 {
            self.se(0x3C);
        }
        done
    }

    /// `SimEv33`: pictures 5 / 4 ask; yes = picture 9, attack and defence + 10 for 30 % of the health, and the
    /// turn does not pass.
    fn ev_power(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(5, 4, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0xC1);
            }
            5 => self.select(0xC2, r),
            6 => {
                self.msg_line = -1;
                if self.answer == 0 {
                    self.goto(14);
                } else {
                    self.next();
                }
            }
            7 => self.say(0xC4),
            8 => self.chara_out(r),
            9 => {
                self.face_a = 9;
                self.cmd(MONITOR_OFF, r);
                self.next();
            }
            10 => {
                self.flags |= SAME_TURN;
                let hp = -(self.stat(HP) * 3 / 10);
                self.show(0xC5, &[(ATK, 10), (DEF, 10), (HP, hp)], r);
                self.next();
            }
            11 => self.faces(5, 4, r),
            12 => {
                self.cmd(CHARA_IN, r);
                self.next();
            }
            13 => self.say_goto(0xC6, 17),
            14 => self.say(0xC3),
            15 => self.chara_out(r),
            16 => self.penalty(r),
            17 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimPopo_Swap`: two of the five shells change places (down, across, up); true when done.
    fn popo_swap(&mut self) -> bool {
        let g = &mut self.games;
        let (a, b) = (g.popo_a.clamp(0, 4), g.popo_b.clamp(0, 4));
        let speed = POPO_SPEED[g.popo_speed.clamp(0, 2) as usize];
        let mut done = false;
        match g.popo_state {
            0 => {
                g.popo_dy -= speed;
                if g.popo_dy <= -110 {
                    g.popo_state = 1;
                    g.popo_dy = -110;
                }
            }
            1 => {
                let dist = POPO_X[b as usize] - POPO_X[a as usize];
                g.popo_dx += speed;
                if g.popo_dx >= dist {
                    g.popo_dx = dist;
                    g.popo_state = 2;
                }
            }
            2 => {
                g.popo_dy += speed;
                if g.popo_dy >= 0 {
                    g.popo_dy = 0;
                    g.popo_state = 3;
                }
            }
            3 => {
                done = true;
                g.popo_state = 0;
                g.popo_dy = 0;
                g.popo_dx = 0;
                if a == g.popo_target {
                    g.popo_target = b;
                } else if b == g.popo_target {
                    g.popo_target = a;
                }
            }
            _ => {}
        }
        let (dx, dy) = (g.popo_dx, g.popo_dy);
        let f = &mut self.flash[FL_POPO];
        let c = f.find(None, &format!("mc_popo{a:02}"));
        f.clip_set_offset(c, dx, dy);
        let c = f.find(None, &format!("mc_popo{b:02}"));
        f.clip_set_offset(c, -dx, dy);
        done
    }

    /// `SimPopo_CursorOn` / `SimPopo_CursorOff`.
    fn popo_cursor(&mut self, on: bool) {
        let f = &mut self.flash[FL_POPO];
        let c = f.find(None, &format!("mc_popo{:02}", self.cur[ST_SCRIPT]));
        f.clip_goto_label(
            c,
            if on {
                "fl_select_loop"
            } else {
                "fl_select_off"
            },
        );
    }

    /// `SimEv34`: pictures 5 / 4; yes = the shell game at the rank of the third training. One of five shells
    /// holds the mark; they are swapped in pairs, then one is picked: right pays points and defence, wrong costs
    /// defence. Every call places the mark and the shells' pictures.
    fn ev_shells(&mut self, pad: EventPad, r: &mut GameRandom) -> bool {
        let ok = pad.pressed & PAD_CROSS != 0;
        let left = pad.repeat & PAD_LEFT != 0;
        let right = pad.repeat & PAD_RIGHT != 0;
        let trig = self.flash[FL_POPO].trig();
        let t = train_of(2, self.games.popo_rank);
        let mut done = false;
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(5, 4, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0xC1);
            }
            5 => self.select(0xC7, r),
            6 => {
                if self.answer == 0 {
                    self.goto(24);
                } else {
                    let g = &mut self.games;
                    g.popo_rank = self.rank[2];
                    g.popo_target = r.range(5) as i32;
                    g.popo_count = 0;
                    g.popo_speed = train_of(2, g.popo_rank).speed;
                    g.popo_won = 0;
                    self.next();
                }
            }
            7 => self.say(0xC9),
            8 => {
                self.sounds.push(TitleSound::StreamSe(GAME_MUSIC));
                self.sounds.push(TitleSound::BgmStop);
                self.msg_line = 0xCA;
                self.cmd(0x2D, r);
                self.next();
            }
            9 => {
                if trig & 1 != 0 {
                    self.cmd(0x2E, r);
                    self.next();
                }
            }
            10 | 11 | 13 => {
                self.seq_timer += 1;
                let wait = match self.seq {
                    10 => 120,
                    11 => 10,
                    _ => 60,
                };
                if self.seq_timer > wait {
                    match self.seq {
                        10 => self.cmd(0x2F, r),
                        11 => self.cmd(0x30, r),
                        _ => {}
                    }
                    self.next();
                }
            }
            12 => {
                if trig & 0x10 != 0 {
                    self.next();
                }
            }
            14 => {
                if self.games.popo_count >= t.param {
                    self.goto(16);
                    self.popo_cursor(true);
                } else {
                    let target = self.games.popo_target;
                    let a = if r.range(0x100) < 0xAA {
                        r.range(5) as i32
                    } else {
                        target
                    };
                    let mut b;
                    loop {
                        b = r.range(5) as i32;
                        if !(a == b || b == target) {
                            break;
                        }
                    }
                    let g = &mut self.games;
                    (g.popo_a, g.popo_b) = if b < a { (b, a) } else { (a, b) };
                    self.next();
                    self.se(0x1E);
                    self.games.popo_count += 1;
                }
            }
            15 => {
                if self.popo_swap() {
                    self.goto(14);
                }
            }
            16 => {
                if ok {
                    if self.cur[self.state] == self.games.popo_target {
                        self.games.popo_won = 1;
                    }
                    self.cmd(0x31, r);
                    self.next();
                } else if left || right {
                    self.popo_cursor(false);
                    let s = self.state;
                    self.cur[s] += if left { -1 } else { 1 };
                    if self.cur[s] < 0 {
                        self.cur[s] = 4;
                    } else if self.cur[s] >= 5 {
                        self.cur[s] = 0;
                    }
                    self.popo_cursor(true);
                }
            }
            17 => {
                if trig & 4 != 0 {
                    self.next();
                }
            }
            18 => {
                self.seq_timer += 1;
                if self.seq_timer > 30 {
                    if self.games.popo_won != 0 {
                        self.cmd(0x32, r);
                        self.se(0x1F);
                    } else {
                        self.cmd(0x33, r);
                        self.se(0x20);
                    }
                    self.next();
                }
            }
            19 => {
                if trig & 2 != 0 {
                    self.sounds.push(TitleSound::StreamSeStop(0));
                    self.play_bgm();
                    self.cmd(0x34, r);
                    self.next();
                }
            }
            20 => {
                if trig & 0x20 != 0 {
                    self.cmd(0x35, r);
                    if self.games.popo_won != 0 {
                        self.next();
                    } else {
                        self.goto(23);
                    }
                }
            }
            21 => {
                self.show(0xCB, &[(DEF, t.gain), (POINT, t.points)], r);
                self.next();
            }
            22 => self.say_goto(0xCC, 27),
            23 => {
                self.show(0xCD, &[(DEF, t.loss)], r);
                self.goto(27);
            }
            24 => self.say(0xC8),
            25 => self.chara_out(r),
            26 => self.penalty(r),
            27 => done = self.done(),
            _ => {}
        }
        let target = self.games.popo_target.clamp(0, 4);
        let f = &mut self.flash[FL_POPO];
        let c = f.find(None, "mc_fukidasi_popo");
        f.clip_set_offset(c, POPO_X[target as usize], 0);
        let c = f.find(None, "mc_lose_text");
        f.clip_set_uv(c, uv(0, 0x80, 0x200, 0x100));
        for i in 0..5 {
            let name = format!("mc_popo{i:02}");
            let mark = if i != target {
                uv(0x40, 0, 0x80, 0x40)
            } else {
                uv(0, 0, 0x40, 0x40)
            };
            let c = f.find(Some(&name), "mc_maru_batsu");
            f.clip_set_uv(c, mark);
            let body = if i != target {
                uv(0, 0, 0x40, 0x80)
            } else {
                uv(0x40, 0, 0x80, 0x80)
            };
            let c = f.find(Some(&name), "mc_popo_anime00");
            f.clip_set_uv(c, body);
        }
        if f.se() & 1 != 0 {
            self.se(0x3C);
        }
        done
    }

    /// `SimEv35`: picture 2, a robbery: an item carried is taken; without items but with 100 points or more a
    /// tenth of them goes; with neither defence - 5 and no turn's price.
    fn ev_robbery(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(2, 2, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.se(0x23);
                self.say(0xAF);
            }
            5 => self.say(0xCE),
            6 => self.say(0xCF),
            7 => {
                if self.count_items() == 0 {
                    if self.stat(POINT) >= 100 {
                        self.goto(9);
                    } else {
                        self.goto(10);
                    }
                } else {
                    self.next();
                }
            }
            8 => {
                self.take_item(r);
                self.say_goto(0xD0, 13);
            }
            9 => {
                let points = -(self.stat(POINT) / 10);
                self.show(0xD1, &[(POINT, points)], r);
                self.goto(13);
            }
            10 => {
                self.show(0xB3, &[(DEF, -5)], r);
                self.next();
            }
            11 => self.say(0xB4),
            12 => {
                self.chara_out(r);
                self.goto(16);
            }
            13 => self.say(0xD2),
            14 => self.chara_out(r),
            15 => self.penalty(r),
            16 => return self.done(),
            _ => {}
        }
        false
    }

    /// `SimEv36`: pictures 7 / 3, a gift of 5000 points.
    fn ev_present(&mut self, r: &mut GameRandom) -> bool {
        match self.seq {
            0..=2 => self.opening(r),
            3 => self.faces(7, 3, r),
            4 => {
                self.cmd(CHARA_IN, r);
                self.say(0xAF);
            }
            5 => self.say(0xD3),
            6 => self.say(0xD4),
            7 => {
                self.show(0xD5, &[(POINT, 5000)], r);
                self.next();
            }
            8 => self.say(0xD6),
            9 => self.chara_out(r),
            10 => self.penalty(r),
            11 => return self.done(),
            _ => {}
        }
        false
    }
}
