//! The battle sequence's opening (src/battle/btl_seq.c): stage intro cuts, entrance poses with their lines, the
//! ready wait; with the scripted camera it drives (src/battle/btl_demo_cam.c), the screen cross-fade between cuts
//! (`ScrXfade_*`, src/battle/stg_ambient.c) and the timer they use (`Ramp_*`, src/sys/ramp.c).

use bt3_data::cam_anim::CamAnim;

use crate::battle::{Battle, IDENTITY, Mtx, View, inverse_rt, mul};
use crate::btl_input::InputGate;
use crate::btl_menu::MenuInput;
use crate::font::Font;
use crate::rng::LibcRandom;
use crate::title::TitleSound;

/// `VOICE_CHARA_BASE_ALT`: a fresh save has flag bit 0 set, so the battle lines come from the second set.
const VOICE_CHARA_BASE: u32 = 0xCC32;
const VOICE_CHARA_LINES: u32 = 100;

/// `Ramp`: a linear value over `seconds * 30` steps.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ramp {
    running: bool,
    frames: f32,
    pub(crate) value: f32,
    target: f32,
    step: f32,
}

impl Ramp {
    pub(crate) fn start(&mut self, seconds: f32, from: f32, to: f32) {
        self.running = true;
        self.frames = seconds * 30.0;
        self.value = from;
        self.target = to;
        self.step = (to - from) / self.frames;
    }

    /// `value == target`.
    pub(crate) fn at_target(&self) -> bool {
        self.value == self.target
    }

    /// `Ramp_Step`: 1 when it has ended (or is stopped).
    pub(crate) fn step(&mut self) -> bool {
        if !self.running {
            return true;
        }
        self.frames -= 1.0;
        if self.frames <= 0.0001 {
            self.value = self.target;
            return true;
        }
        self.value += self.step;
        if 0.0 < self.step {
            self.value = self.value.min(self.target);
        } else {
            self.value = self.value.max(self.target);
        }
        false
    }
}

const DEMO_PLAYING: u32 = 1;
const DEMO_STOP: u32 = 2;
const DEMO_AUTO: u32 = 4;
const DEMO_STOP_AT_END: u32 = 8;

/// `gDemoCam`, the animation part.
#[derive(Debug, Default)]
pub(crate) struct DemoCam {
    anim: Option<CamAnim>,
    time: f32,
    flags: u32,
    /// The fighter the animation is relative to (`obj`: its node 0).
    attached: Option<usize>,
    base: Option<Mtx>,
}

impl DemoCam {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn set_anim(&mut self, anim: CamAnim) {
        self.anim = Some(anim);
        self.attached = None;
        self.base = None;
        self.time = 0.0;
    }

    /// `DemoCam_IsActive`: a pending stop resets the camera and gives false.
    pub(crate) fn is_active(&mut self) -> bool {
        if self.anim.is_some() {
            if self.flags & DEMO_STOP != 0 {
                self.reset();
            } else if self.flags & DEMO_PLAYING != 0 {
                return true;
            }
        }
        false
    }

    fn stop(&mut self) {
        if self.anim.is_some() {
            self.flags |= DEMO_STOP;
        }
    }

    pub(crate) fn attached(&self) -> Option<usize> {
        self.attached
    }

    /// `DemoCam_Update` for an animation: advance by 2.0 (holding the end, stopping there for a stage cut), take
    /// the attached fighter's node 0 as the base, then the view: rotations X, Y, Z, the position, the base, inverted.
    pub(crate) fn update(&mut self, base: Option<Mtx>, paused: bool) -> Option<View> {
        let anim = self.anim.as_ref()?;
        if self.flags & DEMO_AUTO != 0 && !paused {
            let t = self.time + 2.0;
            if anim.length < t {
                if self.flags & DEMO_STOP_AT_END != 0 {
                    self.flags |= DEMO_STOP;
                }
                self.time = anim.length;
            } else {
                self.time = t;
            }
        }
        if self.attached.is_some() {
            self.base = base;
        }
        let pose = anim.eval(self.time);
        let rot: [f32; 3] = std::array::from_fn(|i| {
            let mut a = pose[3 + i];
            for _ in 0..2 {
                if a > std::f32::consts::PI {
                    a -= std::f32::consts::TAU;
                }
                if a < -std::f32::consts::PI {
                    a += std::f32::consts::TAU;
                }
            }
            a
        });
        let mut camera = mul(
            &mul(&rotate_x(rot[0]), &rotate_y(rot[1])),
            &rotate_z(rot[2]),
        );
        camera[3] = [pose[0], pose[1], pose[2], 1.0];
        let camera = mul(&camera, &self.base.unwrap_or(IDENTITY));
        Some(View::from_world_to_view(inverse_rt(&camera)))
    }
}

/// `Mtx_RotateX` on the identity: rows (1, 0, 0), (0, c, s), (0, -s, c).
fn rotate_x(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[1] = [0.0, c, s, 0.0];
    m[2] = [0.0, -s, c, 0.0];
    m
}

fn rotate_y(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[0] = [c, 0.0, -s, 0.0];
    m[2] = [s, 0.0, c, 0.0];
    m
}

fn rotate_z(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[0] = [c, s, 0.0, 0.0];
    m[1] = [-s, c, 0.0, 0.0];
    m
}

/// `gScrXfade`: a captured frame drawn over the following ones, fading out.
#[derive(Debug, Default)]
pub(crate) struct ScrXfade {
    ramp: Ramp,
    alpha: i32,
    request: bool,
}

impl ScrXfade {
    fn request_capture(&mut self) {
        self.request = true;
    }

    fn start(&mut self, seconds: f32) {
        self.ramp.start(seconds, 0.0, 1.0);
        self.request = true;
    }

    fn capture_and_start(&mut self) {
        self.request_capture();
        self.start(1.0);
    }

    /// `ScrXfade_PostDraw` then `ScrXfade_Update` (the overlay pass): whether this frame is captured, and the
    /// alpha (0..128) the captured frame is drawn with.
    pub(crate) fn draw_pass(&mut self) -> (bool, i32) {
        let capture = std::mem::take(&mut self.request);
        if self.ramp.step() {
            self.alpha = 0;
            self.request = false;
        } else {
            self.alpha = (128.0 - self.ramp.value * 128.0) as i32;
        }
        (capture, self.alpha)
    }
}

/// `BtlSeq` states (`BTL_SEQ_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeqState {
    StageIntro,
    IntroTalk,
    Ready,
    Fight,
    /// The finish announcement (K.O. / time up), 3.5 s.
    Finish,
    /// The winner's (or the defeated player's) pose and line.
    WinTalk,
    /// Result handling and the fade out.
    End,
    /// `BTL_SEQ_EXIT`: the battle scene is left.
    Exit,
}

/// `BattleResult.winner` bits.
pub(crate) const WIN_P1: u32 = 1;
pub(crate) const WIN_P2: u32 = 2;
pub(crate) const DRAW: u32 = 4;
pub(crate) const ABORT: u32 = 8;
/// `BattleResult.reason` bits.
pub(crate) const REASON_KO: u32 = 1;
pub(crate) const REASON_TIME_UP: u32 = 2;
pub(crate) const REASON_FLAG7: u32 = 4;
/// Either bit: restart the battle instead of leaving.
pub(crate) const REASON_RESTART: u32 = 0x1_8000;

/// `gBtlSeq` and its handler context.
#[derive(Debug)]
pub(crate) struct Seq {
    pub(crate) state: SeqState,
    step: i32,
    timer: Ramp,
    side: [usize; 2],
    chara: [i32; 2],
    line: [i32; 2],
    /// `BTL_CHARS_STARTED`: the roster was reset on the first Ready / Fight frame.
    pub(crate) started: bool,
    clock: Clock,
    /// `Battle_GetTimeLimit` in seconds, -1 for none.
    time_limit: i32,
    /// `BATTLE_FLAG_DEMO`: the fighters' input is neutral.
    demo_flag: bool,
    /// `BATTLE_FLAG_READY`: the fighters' input is masked.
    ready_flag: bool,
    /// `BattleResult`: the winner bits and the reason bits.
    pub(crate) winner: u32,
    pub(crate) reason: u32,
    /// `BATTLE_FLAG_RESTART`: the battle restarts once the sequence has left.
    pub(crate) restart: bool,
    /// The sequence has left without a restart: the battle is over (`BtlGame_Update` returns 1).
    pub(crate) left: bool,
    /// `gBtlPause.pad` (`BtlPause_GetMenuPad`): the controller port that drives the pause and result menus.
    pub(crate) result_pad: usize,
    /// `BATTLE_FLAG_PAUSE`: the simulation stands still.
    pub(crate) paused: bool,
    /// `BATTLE_FLAG_PAUSE_MENU`: the pause menu is open.
    pub(crate) pause_menu: bool,
}

/// `BtlClock`: hours, minutes, seconds and milliseconds of fight.
#[derive(Clone, Copy, Debug, Default)]
struct Clock {
    ticks: u32,
    ms: i32,
    seconds: i32,
    minutes: i32,
    hours: i32,
    time_left: i32,
}

impl Clock {
    /// `BtlClock_Tick`: 34, 32, 34 ms in turn (100 ms per three 30 Hz frames), stopping at 9:59:59.999.
    fn tick(&mut self) {
        self.ms += [34, 32, 34][(self.ticks % 3) as usize];
        self.ticks += 1;
        if self.hours >= 9 && self.minutes >= 59 && self.seconds >= 59 && self.ms >= 999 {
            (self.hours, self.minutes, self.seconds, self.ms) = (9, 59, 59, 999);
            return;
        }
        if self.ms >= 1000 {
            self.ms -= 1000;
            self.seconds += 1;
        }
        if self.seconds >= 60 {
            self.seconds -= 60;
            self.minutes += 1;
        }
        if self.minutes >= 60 {
            self.minutes -= 60;
            self.hours += 1;
        }
    }
}

impl Seq {
    /// `BtlSeq_Reset`: the default table starts at the stage intro; modes 5..7 have no intro or talk.
    pub(crate) fn new(mode: i32, time_limit: i32) -> Self {
        Self {
            state: if (5..=7).contains(&mode) {
                SeqState::Ready
            } else {
                SeqState::StageIntro
            },
            step: 0,
            timer: Ramp::default(),
            side: [0, 1],
            chara: [0; 2],
            line: [0; 2],
            started: false,
            // BtlSeq_ResetClocks.
            clock: Clock {
                time_left: time_limit,
                ..Clock::default()
            },
            time_limit,
            demo_flag: false,
            ready_flag: false,
            winner: 0,
            reason: 0,
            restart: false,
            left: false,
            result_pad: 0,
            paused: false,
            pause_menu: false,
        }
    }

    /// `BtlSeq_GetState`.
    pub(crate) fn state_id(&self) -> i32 {
        match self.state {
            SeqState::StageIntro => 0,
            SeqState::IntroTalk => 1,
            SeqState::Ready => 2,
            SeqState::Fight => 3,
            SeqState::Finish => 4,
            SeqState::WinTalk => 5,
            SeqState::End => 6,
            SeqState::Exit => 99,
        }
    }

    /// `BattleResult_GetWinnerSide`.
    pub(crate) fn winner_side(&self) -> usize {
        usize::from(self.winner & WIN_P1 == 0 && self.winner & WIN_P2 != 0)
    }

    /// What the battle flags let through to the fighters' input (`BtlInput_Fetch`).
    pub(crate) fn input_gate(&self, stage_id: i32) -> InputGate {
        InputGate {
            block: self.demo_flag,
            filter: self.ready_flag,
            stage_4_or_27: matches!(stage_id, 4 | 27),
            time_stopped: false,
        }
    }

    /// `BtlSeq_GetTimeLeft`: seconds left, -1 without a limit.
    pub(crate) fn time_left(&self) -> i32 {
        if self.time_limit < 0 {
            -1
        } else {
            self.clock.time_left
        }
    }

    /// `BtlSeq_TickClocks` (from `BtlSeq_CheckBattleEnd`): whether the time limit was reached.
    fn tick_clock(&mut self) -> bool {
        self.clock.tick();
        if self.time_limit < 0 {
            return false;
        }
        let elapsed = self.clock.minutes * 60 + self.clock.seconds;
        if elapsed >= self.time_limit {
            self.clock.time_left = 0;
            return true;
        }
        self.clock.time_left = self.time_limit - elapsed;
        false
    }
}

/// What the sequence needs from the frame's input and sound state.
pub(crate) struct SeqInput<'a> {
    /// `BtlPause_CheckStart`: START went down on a playing pad.
    pub start: bool,
    /// Voice player status (`Voice_IsStopped` is 0 or play end).
    pub voice_stat: i32,
    pub libc: &'a mut LibcRandom,
    pub sounds: &'a mut Vec<TitleSound>,
    /// Per controller port, the menu buttons that went down this frame.
    pub menu: [MenuInput; 2],
    /// The text printer, for the menus' box sizes.
    pub font: Option<&'a Font>,
}

impl Battle {
    /// `BtlSeq_Reset` after `BtlSeq_Init`: both voices stopped, the first state entered.
    pub(crate) fn seq_start(&mut self, input: &mut SeqInput) {
        input.sounds.push(TitleSound::VoiceStop);
        self.seq_enter(input);
    }

    fn seq_enter(&mut self, input: &mut SeqInput) {
        match self.seq.state {
            SeqState::StageIntro => self.seq.step = 0,
            SeqState::IntroTalk => self.intro_talk_setup(input),
            SeqState::Ready => {
                self.seq.demo_flag = true;
                self.seq.step = 0;
                self.seq.timer.start(0.8, 0.0, 1.0);
                self.hud.show_all(true);
            }
            SeqState::Fight => {
                self.seq.demo_flag = false;
                self.seq.ready_flag = false;
            }
            SeqState::Finish => self.finish_enter(input),
            SeqState::WinTalk => self.win_talk_enter(input),
            SeqState::End => self.end_enter(),
            SeqState::Exit => {}
        }
    }

    fn seq_exit(&mut self, input: &mut SeqInput) {
        match self.seq.state {
            SeqState::StageIntro => {
                if self.demo.is_active() {
                    self.demo.stop();
                }
                self.xfade.capture_and_start();
            }
            SeqState::IntroTalk => {
                self.demo.stop();
                // BtlCtrl_EndEntrance: flag 0xF0 sends action 1 to the neutral state.
                let sides = self.seq.side;
                for side in sides {
                    self.roster.chars[side].flags.set(0xF0);
                }
            }
            SeqState::Ready => self.hud.show_notice(1, input.sounds),
            // BtlSeq_StopTalk (not in mode 1).
            SeqState::WinTalk if self.assets.mode != 1 => input.sounds.push(TitleSound::VoiceStop),
            SeqState::End => {
                if self.demo.is_active() {
                    self.demo.stop();
                }
            }
            _ => {}
        }
    }

    /// `BtlSeq_PreUpdate`.
    pub(crate) fn seq_pre_update(&mut self, input: &mut SeqInput) {
        match self.seq.state {
            SeqState::StageIntro => {
                if matches!(self.seq.step, 0 | 2 | 4) {
                    self.play_stage_cut(self.seq.step as usize / 2);
                    self.seq.step += 1;
                }
            }
            SeqState::IntroTalk => {
                if matches!(self.seq.step, 0 | 2) {
                    let i = self.seq.step as usize / 2;
                    let side = self.seq.side[i];
                    // BtlCtrl_StartEntrance: flag 0xEF forces action 1.
                    self.roster.chars[side].flags.set(0xEF);
                    self.seq.timer.start(10.0, 0.0, 1.0);
                    self.play_entrance_cut(side);
                    let voice = VOICE_CHARA_BASE
                        + (self.seq.chara[i].max(0) as u32 * VOICE_CHARA_LINES
                            + self.seq.line[i] as u32);
                    input.sounds.push(TitleSound::Voice(voice));
                    self.seq.step += 1;
                }
            }
            SeqState::WinTalk => self.win_talk_pre_update(input),
            _ => {}
        }
    }

    /// `BtlSeq_Update`: the state's update and the state change it asks for.
    pub(crate) fn seq_update(&mut self, input: &mut SeqInput) {
        let next = match self.seq.state {
            SeqState::StageIntro => self.stage_intro_update(input),
            SeqState::IntroTalk => self.intro_talk_update(input),
            SeqState::Ready => self.ready_update(input),
            SeqState::Fight => self.fight_update(input),
            SeqState::Finish => self.finish_update(),
            SeqState::WinTalk => self.win_talk_update(input),
            SeqState::End => self.end_update(input),
            SeqState::Exit => SeqState::Exit,
        };
        // A state the mode's table leaves out is never entered: modes 5..7 have no winner scene.
        let next = if next == SeqState::WinTalk && (5..=7).contains(&self.assets.mode) {
            SeqState::End
        } else {
            next
        };
        if next != self.seq.state {
            self.seq_exit(input);
            self.seq.state = next;
            if next == SeqState::Exit {
                // A rematch asked for from the result menu restarts the battle (BATTLE_FLAG_RESTART); anything
                // else leaves it.
                if self.seq.reason & REASON_RESTART != 0
                    || (self.assets.mode == 6 && self.seq.reason & REASON_FLAG7 != 0)
                {
                    self.seq.restart = true;
                } else {
                    self.seq.left = true;
                }
                return;
            }
            self.seq_enter(input);
        }
    }

    /// `BtlSeqFight_Update` without the pause menu: the end of the battle goes to the finish (state 6 without a
    /// finish scene).
    fn fight_update(&mut self, input: &mut SeqInput) -> SeqState {
        if self.assets.mode != 8 {
            if !self.seq.pause_menu && self.check_open(input) {
                if self.assets.mode == 7 {
                    // BattleResult_Set(BTL_RESULT_ABORT, 0).
                    self.seq.winner = ABORT;
                    self.seq.reason = 0;
                } else {
                    // Mode 1's running script events are not ported.
                    self.seq.pause_menu = true;
                    self.seq.paused = true;
                    input.sounds.push(TitleSound::PauseSeVoice(true));
                }
            }
            if self.seq.pause_menu
                && let Some(menu) = &mut self.menu
            {
                let pad = self.seq.result_pad;
                let (open, reason) = menu.update(
                    pad as i32,
                    0,
                    input.menu[pad],
                    input.font,
                    &self.assets.menu_texts,
                    input.sounds,
                );
                if let Some(reason) = reason {
                    self.seq.winner = ABORT;
                    self.seq.reason = reason as u32;
                }
                if !open {
                    // PauseMenu_Update's close (Adx_ResumeSeVoice), then the flags.
                    input.sounds.push(TitleSound::PauseSeVoice(false));
                    self.seq.pause_menu = false;
                    self.seq.paused = false;
                }
            }
        }
        if self.check_battle_end(input) {
            if self.assets.mode == 7 || self.seq.winner & ABORT != 0 {
                return SeqState::End;
            }
            return SeqState::Finish;
        }
        SeqState::Fight
    }

    /// `BtlPause_CheckOpen`: START on one of the pads that may pause (both in split screen, else pad 0) opens the
    /// menu for that pad. (Mode 7's second test of pad 0 adds nothing here; the pulled-out pad of
    /// `PadWatch_GetMissing` is not emulated.)
    fn check_open(&mut self, input: &SeqInput) -> bool {
        let count = if self.assets.screen_mode == 1 { 2 } else { 1 };
        for i in 0..count {
            if input.menu[i].start {
                self.seq.result_pad = i;
                return true;
            }
        }
        false
    }

    /// `BtlSeq_CheckBattleEnd`: ticks the clock and decides whether the battle is over (never while paused).
    fn check_battle_end(&mut self, input: &mut SeqInput) -> bool {
        if self.seq.winner & 0x1F != 0 {
            return true;
        }
        if self.seq.paused {
            return false;
        }
        if self.roster.chars.iter().any(|c| c.flags.test(0x128)) {
            return false;
        }
        let time_up = self.seq.tick_clock();
        if self.assets.mode == 1 {
            return false;
        }
        if time_up {
            self.seq.reason = REASON_TIME_UP;
            self.judge_by_health(input);
            return true;
        }
        let flag7 = [0, 1].map(|i| self.roster.chars[i].flags.test(7));
        if flag7[0] && flag7[1] {
            self.seq.reason = REASON_FLAG7;
            self.judge_by_health(input);
            return true;
        }
        if flag7[1] || flag7[0] {
            self.seq.winner = if flag7[1] { WIN_P1 } else { WIN_P2 };
            self.seq.reason = REASON_FLAG7;
            return true;
        }
        // BtlCtrl_IsTeamDead: one member per side.
        let dead = [0, 1].map(|i| self.roster.chars[i].gauge.health <= 0);
        if dead[0] && dead[1] {
            self.seq.reason = REASON_KO;
            self.judge_by_health(input);
            return true;
        }
        if dead[1] || dead[0] {
            self.seq.winner = if dead[1] { WIN_P1 } else { WIN_P2 };
            self.seq.reason = REASON_KO;
            return true;
        }
        false
    }

    /// `BtlSeq_JudgeByHealth`: more health wins; a tie goes to the side that landed the battle's first hit
    /// (event 0x3C), a draw after a mode-0 time up, side 1 in mode 8, else `rand()`.
    fn judge_by_health(&mut self, input: &mut SeqInput) {
        let health = [0, 1].map(|i| self.roster.chars[i].gauge.health);
        self.seq.winner = if health[0] == health[1] {
            if self.roster.chars[0].first_hit {
                WIN_P1
            } else if self.roster.chars[1].first_hit {
                WIN_P2
            } else if self.seq.reason & REASON_TIME_UP != 0 && self.assets.mode == 0 {
                DRAW
            } else if self.assets.mode == 8 || input.libc.rand() & 1 == 0 {
                WIN_P2
            } else {
                WIN_P1
            }
        } else if health[1] < health[0] {
            WIN_P1
        } else {
            WIN_P2
        };
    }

    /// `BtlSeqFinish_Enter`: the fighters held, 3.5 s, the announcement of the reason (K.O. 2, or 3 when the
    /// winner took no damage; time up 5; reason bit 2 gives 4).
    fn finish_enter(&mut self, input: &mut SeqInput) {
        self.seq.demo_flag = true;
        self.seq.timer.start(3.5, 0.0, 1.0);
        if self.seq.reason & REASON_KO != 0 {
            let winner = self.seq.winner_side();
            let id = if self.roster.chars[winner].took_damage {
                2
            } else {
                3
            };
            self.hud.show_notice(id, input.sounds);
        } else if self.seq.reason & REASON_TIME_UP != 0 {
            self.hud.show_notice(5, input.sounds);
        } else if self.seq.reason & REASON_FLAG7 != 0 {
            self.hud.show_notice(4, input.sounds);
        }
    }

    /// `BtlSeqFinish_Update`: the winner scene when a side won and the mode is not 8, else the end.
    fn finish_update(&mut self) -> SeqState {
        if self.seq.timer.step() {
            if self.assets.mode != 8 && self.seq.winner & (WIN_P1 | WIN_P2) != 0 {
                return SeqState::WinTalk;
            }
            return SeqState::End;
        }
        SeqState::Finish
    }

    /// `BattleResult_IsPlayerWin`: side 0 won (always in modes 8 and 1).
    fn player_won(&self) -> bool {
        matches!(self.assets.mode, 8 | 1) || self.seq.winner & WIN_P1 != 0
    }

    /// `BtlSeqWinTalk_Enter` with `BtlSeqWinTalk_Setup`: the winner's line (its special line against the loser,
    /// else line 3 or 4) when the player won, else the defeated player's line 0x2A; the gauges and the clock hidden.
    fn win_talk_enter(&mut self, input: &mut SeqInput) {
        self.seq.demo_flag = true;
        self.xfade.capture_and_start();
        let winner = self.seq.winner_side();
        let loser = usize::from(winner == 0);
        if self.player_won() {
            let chara = self.assets.fighters[winner].chara;
            let opp = self.assets.fighters[loser].chara;
            let special = self.assets.fighters[winner]
                .talk
                .get(opp.max(0) as usize * 4 + 2)
                .copied()
                .filter(|&b| b != 0xFF);
            self.seq.side[0] = winner;
            self.seq.chara[0] = chara;
            self.seq.line[0] = match special {
                Some(b) => i32::from(b) + 0x1E,
                None => input.libc.rand() % 2 + 3,
            };
            self.hud.show_notice(6, input.sounds);
        } else {
            self.seq.side[0] = loser;
            self.seq.chara[0] = self.assets.fighters[loser].chara;
            self.seq.line[0] = 0x2A;
            self.hud.show_notice(7, input.sounds);
        }
        self.seq.step = 0;
        self.seq.timer.start(10.0, 0.0, 1.0);
        self.hud.show_gauges(false);
        self.hud.show_timer(false);
    }

    /// `BtlSeqWinTalk_PreUpdate`: the pose, its camera and the line; then a 1.5 s hold.
    fn win_talk_pre_update(&mut self, input: &mut SeqInput) {
        match self.seq.step {
            0 => {
                let side = self.seq.side[0];
                if self.player_won() {
                    // BtlCtrl_StartWinPose: flag 0xF1 forces action 2.
                    self.roster.chars[side].flags.set(0xF1);
                    self.play_fighter_cut(side, 1);
                } else {
                    // BtlCtrl_StartLosePose: flag 0xF2 forces action 3.
                    self.roster.chars[side].flags.set(0xF2);
                    self.play_fighter_cut(side, 2);
                }
                self.seq.timer.start(10.0, 0.0, 1.0);
                let voice = VOICE_CHARA_BASE
                    + (self.seq.chara[0].max(0) as u32 * VOICE_CHARA_LINES
                        + self.seq.line[0] as u32);
                input.sounds.push(TitleSound::Voice(voice));
                self.seq.step += 1;
            }
            2 => {
                self.seq.timer.start(1.5, 0.0, 1.0);
                self.seq.step += 1;
            }
            _ => {}
        }
    }

    /// `BtlSeqWinTalk_Update`: the line (or 10 s), 1.5 s, then the end; START skips.
    fn win_talk_update(&mut self, input: &mut SeqInput) -> SeqState {
        let stopped = input.voice_stat == 0 || input.voice_stat == crate::anim::VOICE_PLAYEND;
        match self.seq.step {
            0 | 1 => {
                if self.roster.chars[self.seq.side[0]].pose_reached()
                    && (stopped || self.seq.timer.step())
                {
                    self.seq.step = 2;
                }
            }
            2 | 3 => {
                if self.seq.timer.step() {
                    self.seq.step = 4;
                }
            }
            4 => return SeqState::End,
            _ => {}
        }
        if input.start {
            input.sounds.push(TitleSound::VoiceStop);
            return SeqState::End;
        }
        SeqState::WinTalk
    }

    /// `BtlSeqEnd_Enter`: mode 0 with a finished battle opens the result menu, for pad 0 (or the winner's pad in
    /// split screen, `BtlSeq_SetResultPad`); otherwise the 1.2 s fade out.
    fn end_enter(&mut self) {
        self.seq.demo_flag = true;
        if self.seq.winner & ABORT != 0 || self.assets.mode != 0 {
            self.fade = crate::fade::Fade::start_out(1.0);
            self.seq.timer.start(1.2, 0.0, 1.0);
            self.seq.step = 1;
        } else {
            self.seq.step = 0;
            self.seq.result_pad = if self.assets.screen_mode == 1 {
                self.seq.winner_side()
            } else {
                0
            };
        }
    }

    /// `BtlSeqEnd_Update`: step 0 runs the result menu until a picked entry ends the battle, step 1 waits for the
    /// fade.
    fn end_update(&mut self, input: &mut SeqInput) -> SeqState {
        match self.seq.step {
            0 => {
                if let Some(menu) = &mut self.menu {
                    let pad = self.seq.result_pad;
                    let (_, reason) = menu.update(
                        pad as i32,
                        1,
                        input.menu[pad],
                        input.font,
                        &self.assets.menu_texts,
                        input.sounds,
                    );
                    if let Some(reason) = reason {
                        // BattleResult_Set(BTL_RESULT_ABORT, reason).
                        self.seq.winner = ABORT;
                        self.seq.reason = reason as u32;
                    }
                }
                if self.seq.winner & ABORT != 0 {
                    self.fade = crate::fade::Fade::start_out(1.0);
                    self.seq.timer.start(1.2, 0.0, 1.0);
                    self.seq.step = 1;
                }
            }
            1 => {
                if self.seq.timer.step() {
                    return SeqState::Exit;
                }
            }
            _ => {}
        }
        SeqState::End
    }

    /// `DemoCam_PlayObjAnim(side, idx)`: the fighter's camera animation `idx`, relative to its node 0.
    fn play_fighter_cut(&mut self, side: usize, idx: usize) {
        if let Some(Some(anim)) = self.assets.fighters[side].cam_cuts.get(idx) {
            self.demo.set_anim(anim.clone());
            self.demo.attached = Some(side);
            self.demo.flags = DEMO_PLAYING | DEMO_AUTO;
        }
    }

    /// `DemoCam_PlayStageAnim`: the cut in world space, ending the camera at its end.
    fn play_stage_cut(&mut self, cut: usize) {
        if let Some(Some(anim)) = self.assets.stage_cuts.get(cut) {
            self.demo.set_anim(anim.clone());
            self.demo.flags = DEMO_PLAYING | DEMO_AUTO | DEMO_STOP_AT_END;
        }
    }

    /// `DemoCam_PlayObjAnim(side, 0)`: the fighter's camera cut, relative to its node 0, holding its last frame.
    fn play_entrance_cut(&mut self, side: usize) {
        self.play_fighter_cut(side, 0);
    }

    fn stage_intro_update(&mut self, input: &SeqInput) -> SeqState {
        match self.seq.step {
            0..=3 => {
                if !self.demo.is_active() {
                    self.xfade.capture_and_start();
                    self.seq.step += 1;
                }
            }
            4 | 5 => {
                if !self.demo.is_active() {
                    self.seq.step += 1;
                    return SeqState::IntroTalk;
                }
            }
            _ => return SeqState::IntroTalk,
        }
        if input.start {
            return SeqState::IntroTalk;
        }
        SeqState::StageIntro
    }

    /// `BtlSeqIntroTalk_Setup`: the pair's special lines when both have a table entry, else a random line 0 / 1 each.
    fn intro_talk_setup(&mut self, input: &mut SeqInput) {
        let chara = [self.assets.fighters[0].chara, self.assets.fighters[1].chara];
        let entry = |side: usize, opp: i32| -> Option<[u8; 2]> {
            let talk = &self.assets.fighters[side].talk;
            let at = opp.max(0) as usize * 4;
            talk.get(at..at + 2).map(|b| [b[0], b[1]])
        };
        let special = match (entry(0, chara[1]), entry(1, chara[0])) {
            (Some(own), Some(other)) if own[0] != 0xFF => Some((own, other)),
            _ => None,
        };
        let seq = &mut self.seq;
        match special {
            Some((own, other)) if own[1] == 0 => {
                seq.side = [0, 1];
                seq.chara = chara;
                seq.line = [i32::from(own[0]) + 6, i32::from(other[0]) + 6];
            }
            Some((own, other)) => {
                seq.side = [1, 0];
                seq.chara = [chara[1], chara[0]];
                seq.line = [i32::from(other[0]) + 6, i32::from(own[0]) + 6];
            }
            None => {
                seq.side = [0, 1];
                seq.chara = chara;
                let first = input.libc.rand() % 2;
                seq.line = [first, input.libc.rand() % 2];
            }
        }
        seq.step = 0;
        seq.timer.start(10.0, 0.0, 1.0);
    }

    fn intro_talk_update(&mut self, input: &mut SeqInput) -> SeqState {
        let stopped = input.voice_stat == 0 || input.voice_stat == crate::anim::VOICE_PLAYEND;
        match self.seq.step {
            0 | 1 => {
                if self.roster.chars[self.seq.side[0]].pose_reached()
                    && (stopped || self.seq.timer.step())
                {
                    self.xfade.capture_and_start();
                    self.seq.step += 1;
                }
            }
            2 | 3 => {
                if self.roster.chars[self.seq.side[1]].pose_reached()
                    && (stopped || self.seq.timer.step())
                {
                    self.xfade.capture_and_start();
                    self.seq.step += 1;
                }
            }
            4 => return SeqState::Ready,
            _ => {}
        }
        if input.start {
            if self.seq.step != 4 {
                self.xfade.capture_and_start();
            }
            // BtlSeq_StopTalk.
            input.sounds.push(TitleSound::VoiceStop);
            return SeqState::Ready;
        }
        SeqState::IntroTalk
    }

    /// `BtlSeqReady_Update`: 0.8 s, announcement 0 with the fighters released, 2 s more.
    fn ready_update(&mut self, input: &mut SeqInput) -> SeqState {
        match self.seq.step {
            0 => {
                if self.seq.timer.step() {
                    self.hud.show_notice(0, input.sounds);
                    self.seq.demo_flag = false;
                    self.seq.ready_flag = true;
                    self.seq.timer.start(2.0, 0.0, 1.0);
                    self.seq.step += 1;
                }
            }
            _ => {
                if self.seq.timer.step() {
                    return SeqState::Fight;
                }
            }
        }
        SeqState::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_ends_after_seconds_times_thirty_steps() {
        let mut ramp = Ramp::default();
        assert!(ramp.step());
        ramp.start(0.8, 0.0, 1.0);
        let steps = (1..).find(|_| ramp.step()).unwrap();
        assert_eq!(steps, 24);
        assert_eq!(ramp.value, 1.0);
    }

    #[test]
    fn cross_fade_captures_once_and_fades_over_a_second() {
        let mut xfade = ScrXfade::default();
        assert_eq!(xfade.draw_pass(), (false, 0));
        xfade.capture_and_start();
        assert_eq!(xfade.draw_pass(), (true, 123));
        let mut frames = 1;
        while xfade.draw_pass().1 != 0 {
            frames += 1;
        }
        // Alpha 123 on the capture frame, then down to 4; the 30th step ends the ramp.
        assert_eq!(frames, 29);
    }
}
