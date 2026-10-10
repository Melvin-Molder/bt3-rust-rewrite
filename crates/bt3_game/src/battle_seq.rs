//! The battle sequence's opening (src/battle/btl_seq.c): stage intro cuts, entrance poses with their lines, the
//! ready wait; with the scripted camera it drives (src/battle/btl_demo_cam.c), the screen cross-fade between cuts
//! (`ScrXfade_*`, src/battle/stg_ambient.c) and the timer they use (`Ramp_*`, src/sys/ramp.c).

use bt3_data::cam_anim::CamAnim;

use crate::battle::{Battle, IDENTITY, Mtx, View, inverse_rt, mul};
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
    pub(crate) fn update(&mut self, base: Option<Mtx>) -> Option<View> {
        let anim = self.anim.as_ref()?;
        if self.flags & DEMO_AUTO != 0 {
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
}

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
}

impl Seq {
    /// `BtlSeq_Reset`: the default table starts at the stage intro; modes 5..7 have no intro or talk.
    pub(crate) fn new(mode: i32) -> Self {
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
        }
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
                self.seq.step = 0;
                self.seq.timer.start(0.8, 0.0, 1.0);
            }
            SeqState::Fight => {}
        }
    }

    fn seq_exit(&mut self) {
        match self.seq.state {
            SeqState::StageIntro => {
                if self.demo.is_active() {
                    self.demo.stop();
                }
                self.xfade.capture_and_start();
            }
            SeqState::IntroTalk => {
                self.demo.stop();
                let sides = self.seq.side;
                for side in sides {
                    self.fighters[side].end_entrance();
                }
            }
            SeqState::Ready | SeqState::Fight => {}
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
                    self.fighters[side].start_entrance();
                    self.seq.timer.start(10.0, 0.0, 1.0);
                    self.play_entrance_cut(side);
                    let voice = VOICE_CHARA_BASE
                        + (self.seq.chara[i].max(0) as u32 * VOICE_CHARA_LINES
                            + self.seq.line[i] as u32);
                    input.sounds.push(TitleSound::Voice(voice));
                    self.seq.step += 1;
                }
            }
            SeqState::Ready | SeqState::Fight => {}
        }
    }

    /// `BtlSeq_Update`: the state's update and the state change it asks for.
    pub(crate) fn seq_update(&mut self, input: &mut SeqInput) {
        let next = match self.seq.state {
            SeqState::StageIntro => self.stage_intro_update(input),
            SeqState::IntroTalk => self.intro_talk_update(input),
            SeqState::Ready => self.ready_update(),
            SeqState::Fight => SeqState::Fight,
        };
        if next != self.seq.state {
            self.seq_exit();
            self.seq.state = next;
            self.seq_enter(input);
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
        if let Some(anim) = &self.assets.fighters[side].entrance_cut {
            self.demo.set_anim(anim.clone());
            self.demo.attached = Some(side);
            self.demo.flags = DEMO_PLAYING | DEMO_AUTO;
        }
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
                if self.fighters[self.seq.side[0]].pose_reached()
                    && (stopped || self.seq.timer.step())
                {
                    self.xfade.capture_and_start();
                    self.seq.step += 1;
                }
            }
            2 | 3 => {
                if self.fighters[self.seq.side[1]].pose_reached()
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
    fn ready_update(&mut self) -> SeqState {
        match self.seq.step {
            0 => {
                if self.seq.timer.step() {
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
