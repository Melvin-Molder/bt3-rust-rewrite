//! The story battle's script (`BtlScript_*`, src/battle/btl_script.c), its 46 command handlers
//! (`gBtlScriptCommands`, src/battle/btl_script_cmd.c) and what they reach in the fight (`BtlFacade_*`,
//! src/battle/btl_facade.c; `BtlCtrl_*`, src/battle/btl_char_api_2.c).
//!
//! A story battle runs one script file (`0x1FF + script`) on the GSC engine:
//!
//!   - Setup. Action 1000 runs to its end before the battle loads (`BattleLoad_Step` 4): its commands set the
//!     rule and the teams (mode 1), the story number that picks the text, lip and voice files, the text windows,
//!     who says which line, and the rewards of the story's result screen.
//!   - Main task. The first frame starts action -1 (`Battle_ResetWork`); the engine steps it every unpaused frame
//!     (`Gsc_Update`, step 4 of `Battle_Loop`), together with every task it started.
//!   - Triggers. Command 7 lists "start event action A" / "say line L" against battle events. In sequence states 3
//!     and 5 every trigger is tested against the events that are new this frame: a line is said when no other is
//!     being said, an event action waits (`pending`) until the fight lets it start, then replaces the main task.
//!
//! The commands place and move the fighters (their scripted actions 4..10), fix and move the demo camera, fade the
//! screen, play the music, sound effects and voiced lines with their text and mouth movement, change health,
//! gauges and items, force transformations, fusions, member switches and techniques, and end the battle with a
//! result, wait for a button of pad 0 (command 1701) or change the stage (command 8's 3).

use std::sync::Arc;

use bt3_data::face::{LipKey, parse_lip_track};
use bt3_data::gsc::{GscScript, WORD_COMMAND, WORD_END};

use crate::battle::Battle;
use crate::btl_chr::{Chr, Gauge, MemberItems};
use crate::char_select::{BattleSetup, MemberStart, SideSetup};
use crate::font::{Font, Style};
use crate::gsc::{Gsc, GscCommands, PHASE_BEGIN, PHASE_BEGIN_ABORT, PHASE_END, PHASE_UPDATE};
use crate::hist_select::HistReward;
use crate::rng::LibcRandom;
use crate::title::TitleSound;

/// `BTL_SCRIPT_ACTION_SETUP`: run to its end before the battle loads.
const ACTION_SETUP: i32 = 1000;
/// `BTL_SCRIPT_ACTION_EVENT`: the first of the 52 event actions.
const ACTION_EVENT: i32 = 10000;
const EVENT_COUNT: usize = 52;
/// `BTL_SCRIPT_NO_MEMBER`: a line nobody was set to say.
const NO_MEMBER: i32 = 0x7FFF;
/// `BTL_SCRIPT_TRIGGER_WAIT_OFF`: the trigger "the battle's wait flag is off".
const TRIGGER_WAIT_OFF: i32 = 0xFFFF;
/// The story scripts: file `0x1FF + script`.
pub const SCRIPT_FILE: u32 = 0x1FF;
/// The story's texts by language (`gProgress->language`, 0 in the US game) and its lip tracks and voices by the
/// save's voice set (`SAVE_FLAG_VOICE`): 300 lines a story.
const TEXT_FILE: u32 = 0x231;
const LIP_FILE: [u32; 2] = [0x295, 0x2C7];
const VOICE_FILE: [u32; 2] = [0xD48, 0x47E0];
const STORY_LINES: usize = 300;
/// `Bgm_Play(n + 0x10B16)`.
const BGM_FIRST: u32 = 0x10B16;
/// `Voice_PlayDefault`'s volume (`ADX_VOL_MAX`).
const VOICE_VOLUME: i32 = 0x80;
/// `Adx_GetStat`: the stream sounds.
const ADX_PLAYING: i32 = 3;

/// `DEG2RAD`.
fn deg(d: f32) -> f32 {
    d * 3.141_592_7 / 180.0
}

/// `SCRIPT_SIDE`: bit 15 of a "who" operand.
fn script_side(who: i32) -> usize {
    ((who >> 15) & 1) as usize
}

/// `BtlUtil_Clamp(v, lo, hi)`.
fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// `BtlScriptEvent`: what `BtlScript_ScanEvents` found out about an event action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScriptEvent {
    pub present: bool,
    /// From the id: 0 below 10030, 1 below 10040, 2 below 10050, 3 for 10050, 4 for 10051.
    pub group: u8,
    /// The action uses command 0x4B1 (1201, the fixed camera): a scene that interrupts the fight.
    pub cmd4b1: bool,
    /// Bit 15 and kind of the first operand of its (last) command 8: 1 a member switch (0x10..0x14), 2 a
    /// transformation or fusion (0x15..0x17).
    pub side: usize,
    pub kind: u8,
}

/// `BtlScriptLine`: a voiced line said when its trigger fires.
#[derive(Clone, Copy, Debug)]
struct Line {
    enabled: bool,
    once: bool,
    /// +10 a frame after the voice stopped; the text goes at 600.
    timer: i32,
    id: i32,
}

#[derive(Clone, Copy, Debug)]
enum Req {
    Line(Line),
    /// An event action (10000..10051) to start.
    Event(i32),
}

/// `BtlScriptRequest`: a trigger (`side << 15 | event`, or 0xFFFF) and what it does.
#[derive(Clone, Copy, Debug)]
struct Request {
    trigger: i32,
    req: Req,
}

/// `BtlScriptCamKey`.
#[derive(Clone, Copy, Debug, Default)]
struct CamKey {
    used: bool,
    /// The move from the key before, in units of 30 frames.
    time: f32,
    pos: [f32; 3],
    rot: [f32; 3],
}

/// `BtlScriptCam`: the scripted move of the fixed camera through its keys.
#[derive(Clone, Copy, Debug, Default)]
struct ScriptCam {
    active: bool,
    time: i32,
    index: usize,
    key: [CamKey; 9],
}

/// `BtlScriptWindow`: a text window (command 1302); its text is drawn with the bottom line at `y`.
#[derive(Clone, Copy, Debug, Default)]
struct Window {
    style: Style,
    x: i32,
    y: i32,
}

/// `BtlScriptSpeaker`.
#[derive(Clone, Copy, Debug)]
struct Speaker {
    side: usize,
    member: i32,
}

/// The status of the streams the script waits on, as the sound system reports it before the frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamStat {
    /// `Adx_GetStat` of voice players 0 and 1 (0 stopped, 2 decoding, 3 playing, 5 played to its end).
    pub voice: [i32; 2],
    /// `!Bgm_IsStopped()`.
    pub bgm_playing: bool,
}

/// `gBtlScript` with the battle's script handle and main task (`BattleEvents.script` / `mainAction`).
#[derive(Debug, Default)]
pub struct BtlScript {
    gsc: Gsc,
    handle: i32,
    main_action: i32,
    /// `story`: picks the text, lip and voice files; -1 before command 2.
    story: i32,
    /// The save's voice set (`SAVE_FLAG_VOICE`).
    voice_set: bool,
    /// The files commands 701 / 702 asked for, read before the battle loads.
    pub text_file: Option<u32>,
    pub lip_file: Option<u32>,
    /// The text file and the lip file: offset tables, entry `n` (a 16-bit string / a `BObjLipData`) at word
    /// `n + 1`.
    text_data: Vec<u8>,
    lip_data: Vec<u8>,
    /// The text of `text_id`.
    shown: Vec<u16>,
    events: Vec<ScriptEvent>,
    /// `cur.event`: the event action started last (`None` while the main action runs).
    cur: Option<usize>,
    text_window: usize,
    /// The line whose text is drawn, -1 none.
    text_id: i32,
    /// `shakeTime`: a shake kept up (command 1204 -l).
    shake_time: f32,
    cam: ScriptCam,
    /// The event request that fired and waits for the fight to allow it.
    pending: Option<usize>,
    windows: [Window; 2],
    speakers: Vec<Speaker>,
    requests: Vec<Request>,
    /// The line request being said, by `side * 2 + window`.
    voice: [Option<usize>; 4],
    /// `BtlScriptCmdWork.items`: each member's eight item ids (0-based, -1 none) as commands 12..14 left them.
    items: [[[i32; 8]; 5]; 2],
}

/// What the setup action gives besides the script work.
pub struct ScriptSetup {
    pub setup: BattleSetup,
    pub script: BtlScript,
}

/// One member as command 12 set it up.
#[derive(Clone, Copy, Debug)]
struct MemberDef {
    chara: i32,
    costume: i32,
    variant: i32,
    cpu_level: i32,
    health: f32,
    items: [u16; 8],
}

/// The handlers while action 1000 runs: they fill the setup, the work and the rewards.
struct SetupHost<'a> {
    s: &'a mut BtlScript,
    setup: BattleSetup,
    count: [usize; 2],
    members: [[Option<MemberDef>; 5]; 2],
    reward: &'a mut HistReward,
    difficulty: i32,
    libc: &'a mut LibcRandom,
}

impl BtlScript {
    /// `BtlScript_IsSlotBusy`: a line in `side`'s slot `window`.
    pub(crate) fn slot_busy(&self, side: usize, window: usize) -> bool {
        self.voice
            .get(side * 2 + window)
            .is_some_and(Option::is_some)
    }

    /// `BtlScript_InitWork`.
    fn new(voice_set: bool) -> Self {
        Self {
            story: -1,
            voice_set,
            events: vec![ScriptEvent::default(); EVENT_COUNT],
            text_id: -1,
            speakers: vec![
                Speaker {
                    side: 0,
                    member: NO_MEMBER,
                };
                STORY_LINES
            ],
            ..Self::default()
        }
    }

    /// `BattleLoad_Step` 4 for a story battle: the script loaded (`Gsc_LoadFile`), action 1000 run to its end
    /// (`Gsc_RunAction`, which also sets `reward`), its event actions scanned (`BtlScript_ScanEvents`) and the setup
    /// finished. `difficulty` is the story's level (`gProgress->difficulty`, 0..2), `voice_set` the save's.
    pub fn setup(
        file: GscScript,
        difficulty: i32,
        voice_set: bool,
        reward: &mut HistReward,
        libc: &mut LibcRandom,
    ) -> ScriptSetup {
        let mut s = Self::new(voice_set);
        s.scan_events(&file);
        let mut gsc = Gsc::default();
        s.handle = gsc.load_file(file);
        let handle = s.handle;
        let mut host = SetupHost {
            s: &mut s,
            setup: BattleSetup {
                mode: 1,
                ..BattleSetup::default()
            },
            count: [1; 2],
            members: [[None; 5]; 2],
            reward,
            difficulty,
            libc,
        };
        gsc.run_action(handle, ACTION_SETUP, &mut host);
        let setup = host.finish();
        s.gsc = gsc;
        ScriptSetup { setup, script: s }
    }

    /// The text file read for command 701 (`BtlScript_RequestTextFile`).
    pub fn set_text_file(&mut self, file: Vec<u8>) {
        self.text_data = file;
    }

    /// The lip file read for command 702 (`BtlScript_RequestFile8`).
    pub fn set_lip_file(&mut self, file: Vec<u8>) {
        self.lip_data = file;
    }

    /// `BtlScript_GetText`: line `n`'s text.
    fn text(&self, n: i32) -> Vec<u16> {
        usize::try_from(n)
            .ok()
            .and_then(|n| bt3_data::pack_section(&self.text_data, n + 1).ok())
            .map(|s| {
                s.chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .take_while(|&c| c != 0)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `BtlScript_GetFile8Entry`: line `n`'s lip track.
    fn lip(&self, n: i32) -> Option<Arc<[LipKey]>> {
        let n = usize::try_from(n).ok()?;
        let s = bt3_data::pack_section(&self.lip_data, n + 1).ok()?;
        parse_lip_track(s)
            .ok()
            .filter(|k| !k.is_empty())
            .map(Arc::from)
    }

    /// `BtlScript_ScanEvents`: for every event action whether it exists, its group, whether it uses command 0x4B1,
    /// and the side and kind of its command 8.
    fn scan_events(&mut self, file: &GscScript) {
        for (ai, action) in file.actions.iter().enumerate() {
            let Some(i) = action
                .id
                .checked_sub(ACTION_EVENT)
                .and_then(|i| usize::try_from(i).ok())
                .filter(|&i| i < EVENT_COUNT)
            else {
                continue;
            };
            let ev = &mut self.events[i];
            ev.present = true;
            ev.group = match action.id {
                ..10030 => 0,
                ..10040 => 1,
                ..10050 => 2,
                10050 => 3,
                _ => 4,
            };
            ev.cmd4b1 = false;
            ev.kind = 0;
            let mut at = 0;
            loop {
                let word = file.word(ai, at);
                if word & 0xF == WORD_END {
                    break;
                }
                if word & 0xF == WORD_COMMAND {
                    match word >> 16 {
                        0x4B1 => ev.cmd4b1 = true,
                        8 => {
                            let value = file
                                .value(file.word(ai, at + 1))
                                .map_or(0, i32::from_le_bytes);
                            ev.side = script_side(value);
                            match value & 0xFFF {
                                0x10..=0x14 => ev.kind = 1,
                                0x15..=0x17 => ev.kind = 2,
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                at = file.next_command(ai, at);
            }
        }
    }

    /// `BtlScript_GetCurrentEvent`.
    pub(crate) fn current_event(&self) -> Option<ScriptEvent> {
        self.cur.map(|i| self.events[i])
    }

    fn event_of(&self, action: i32) -> Option<usize> {
        usize::try_from(action - ACTION_EVENT)
            .ok()
            .filter(|&i| i < EVENT_COUNT)
    }

    /// `BtlScript_IsEventRunning`: a task of an event action (10000..10051) runs.
    pub(crate) fn is_event_running(&self) -> bool {
        self.gsc
            .tasks()
            .any(|(_, action)| (ACTION_EVENT..ACTION_EVENT + EVENT_COUNT as i32).contains(&action))
    }

    /// `BtlScript_AbortEvents`.
    pub(crate) fn abort_events(&mut self) {
        let ids: Vec<i32> = self
            .gsc
            .tasks()
            .filter(|&(_, action)| {
                (ACTION_EVENT..ACTION_EVENT + EVENT_COUNT as i32).contains(&action)
            })
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            self.gsc.abort_task(id);
        }
    }

    /// `BtlScript_GetVoiceFile`.
    fn voice_file(&self, line: i32) -> u32 {
        (VOICE_FILE[usize::from(self.voice_set)] as i32 + self.story * STORY_LINES as i32 + line)
            as u32
    }

    /// `BtlScript_SetText`.
    fn set_text(&mut self, window: usize, text: i32) {
        self.text_window = window & 1;
        self.text_id = text;
        self.shown = self.text(text);
    }

    /// `BtlScript_ClearText`.
    fn clear_text(&mut self, text: i32) {
        if self.text_id == text {
            self.text_id = -1;
        }
    }

    /// `BtlScript_IsTextShown`.
    pub(crate) fn is_text_shown(&self) -> bool {
        self.text_id >= 0
    }

    fn speaker(&self, line: i32) -> Speaker {
        usize::try_from(line)
            .ok()
            .and_then(|i| self.speakers.get(i).copied())
            .unwrap_or(Speaker {
                side: 0,
                member: NO_MEMBER,
            })
    }

    /// `BtlScript_SetSpeaker`.
    fn set_speaker(&mut self, line: i32, who: i32) {
        if let Some(s) = usize::try_from(line)
            .ok()
            .and_then(|i| self.speakers.get_mut(i))
        {
            *s = Speaker {
                side: script_side(who),
                member: who & 0xFFF,
            };
        }
    }

    /// `BtlScript_AddLine` / `BtlScript_AddEvent` (50 at most).
    fn add_request(&mut self, trigger: i32, req: Req) {
        if self.requests.len() < 50 {
            self.requests.push(Request { trigger, req });
        }
    }

    /// `BtlScript_StopVoices`: the four line slots freed, both voices stopped (the streamed effects too, which the
    /// story battle does not play).
    fn stop_voices(&mut self, sounds: &mut Vec<TitleSound>) {
        self.voice = [None; 4];
        sounds.push(TitleSound::VoiceStopEx(0));
        sounds.push(TitleSound::VoiceStopEx(1));
    }

    /// `BtlScript_ResetView`: no trigger, no pending event, no text, no camera move, no kept shake, no line.
    fn reset_view(&mut self, sounds: &mut Vec<TitleSound>) {
        self.requests.clear();
        self.pending = None;
        self.text_window = 0;
        self.shake_time = 0.0;
        self.cam.active = false;
        self.text_id = -1;
        self.stop_voices(sounds);
    }

    /// `BtlScript_Restart` and `Battle_ResetWork`'s `Gsc_StartMain`.
    fn start_main(&mut self, sounds: &mut Vec<TitleSound>) {
        self.cur = None;
        self.reset_view(sounds);
        self.main_action = self.gsc.start_main(self.handle);
    }

    /// The running task replaced by an event action (`BtlScript_StartPending` / `StartWaitEvents`).
    fn start_event(&mut self, action: i32, sounds: &mut Vec<TitleSound>) {
        self.cur = self.event_of(action);
        self.gsc.kill_task(self.main_action);
        self.reset_view(sounds);
        self.main_action = self.gsc.start_action(self.handle, action);
    }

    /// `BtlScript_StartWaitEvents` (events 0x5A / 0x5B): the event whose trigger is "wait flag off" at once (the
    /// reset of the view drops the other triggers).
    fn start_wait_events(&mut self, sounds: &mut Vec<TitleSound>) {
        let found = self.requests.iter().find_map(|r| match r.req {
            Req::Event(action) if r.trigger == TRIGGER_WAIT_OFF => Some(action),
            _ => None,
        });
        if let Some(action) = found {
            self.start_event(action, sounds);
        }
    }
}

impl SetupHost<'_> {
    /// `BattleSetup_SetSide` / `SetMemberByItemIds` turned into the menus' setup.
    fn finish(&mut self) -> BattleSetup {
        let mut setup = std::mem::take(&mut self.setup);
        for (side, out) in setup.sides.iter_mut().enumerate() {
            let defs = self.members[side];
            let def = |m: usize| {
                defs[m].unwrap_or(MemberDef {
                    chara: 0,
                    costume: 0,
                    variant: 0,
                    cpu_level: 0,
                    health: 0.0,
                    items: [0; 8],
                })
            };
            let lead = def(0);
            let count = self.count[side].clamp(1, 5);
            *out = SideSetup {
                // Side 0 at the pad, any other the computer.
                control: if side != 0 { 2 } else { 0 },
                pad: 0,
                change_allowed: 0,
                chara: lead.chara,
                costume: lead.costume,
                cpu_level: lead.cpu_level,
                items: lead.items,
                team_items: std::array::from_fn(|k| def(k + 1).items),
                team: std::array::from_fn(|k| {
                    let m = def(k + 1);
                    (m.chara, m.costume)
                }),
                team_count: count as i32 - 1,
                switch_enabled: 0,
                members: Some(std::array::from_fn(|m| {
                    let d = def(m);
                    MemberStart {
                        cpu_level: d.cpu_level,
                        health: d.health,
                        variant: d.variant,
                    }
                })),
            };
        }
        setup.finish();
        setup
    }

    /// Command 1302 (`BtlScriptCmd_SetWindow`): a text window from the default style; only the entry for language 1
    /// is used.
    fn set_window(&mut self, gsc: &mut Gsc) {
        gsc.find_option(b'l');
        if gsc.int() != 1 {
            return;
        }
        gsc.find_option(b'n');
        let n = gsc.int_or(0).clamp(0, 1) as usize;
        let mut w = Window {
            style: Style::default(),
            ..Window::default()
        };
        gsc.find_option(b'd');
        w.x = gsc.int_or(0);
        w.y = gsc.int_or(0);
        gsc.find_option(b'a');
        w.style.scale_x = gsc.float_or(1.0);
        w.style.scale_y = gsc.float_or(1.0);
        gsc.find_option(b's');
        w.style.scale = gsc.float_or(1.0);
        let color = |gsc: &mut Gsc| {
            let r = gsc.int_or(0xFF) as u8;
            let g = gsc.int_or(0xFF) as u8;
            let b = gsc.int_or(0xFF) as u8;
            let a = gsc.int_or(0x80) as u32;
            u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16 | a << 24
        };
        gsc.find_option(b'c');
        w.style.color = color(gsc);
        gsc.find_option(b'p');
        w.style.spacing_x = gsc.int_or(2);
        w.style.spacing_y = gsc.int_or(5);
        gsc.find_option(b'w');
        w.style.align = gsc.int_or(0);
        gsc.find_option(b'T');
        w.style.shadow_mode = gsc.int_or(2);
        gsc.find_option(b'O');
        w.style.shadow_dx = i32::from(gsc.int_or(1) as i16);
        w.style.shadow_dy = i32::from(gsc.int_or(2) as i16);
        gsc.find_option(b'C');
        w.style.shadow_color = color(gsc);
        self.s.windows[n] = w;
    }
}

impl GscCommands for SetupHost<'_> {
    fn command(&mut self, gsc: &mut Gsc, id: u32, phase: u32) -> bool {
        if phase != PHASE_BEGIN && phase != PHASE_BEGIN_ABORT {
            return true;
        }
        match id {
            // Command 2 "story".
            2 => self.s.story = gsc.int(),
            // Command 10 "rule": stage bgm timeLimit announcer stageChange (`Battle_ClearWork`, then
            // `BattleSetup_SetRule(0, 1, ...)`; music 24 is a random one).
            10 => {
                let stage = gsc.int();
                let bgm = gsc.int();
                let time_limit = gsc.int();
                let announcer = gsc.int();
                let stage_change = gsc.int();
                self.setup = BattleSetup {
                    screen_mode: 0,
                    mode: 1,
                    bgm: if bgm == 24 {
                        self.libc.rand() % 24
                    } else {
                        bgm
                    },
                    time_limit,
                    announcer,
                    stage,
                    stage_change,
                    ..BattleSetup::default()
                };
                self.members = [[None; 5]; 2];
            }
            // Command 11 "side": side members.
            11 => {
                let side = gsc.int();
                let count = gsc.int();
                if let Some(c) = usize::try_from(side)
                    .ok()
                    .and_then(|s| self.count.get_mut(s))
                {
                    *c = usize::try_from(count).unwrap_or(1);
                }
            }
            // Command 12 "member": who chara costume variant cpuLevel item7 health item0..item6; the CPU level up by 6
            // / 9 with the difficulty.
            12 => {
                let who = gsc.int();
                let (member, side) = ((who & 0xFFF) as usize, script_side(who));
                let chara = gsc.int();
                let costume = gsc.int();
                let variant = gsc.int();
                let mut cpu_level = gsc.int();
                let item7 = gsc.int();
                let health = gsc.int();
                let mut ids = [0i32; 8];
                for id in ids.iter_mut().take(7) {
                    *id = gsc.int();
                }
                ids[7] = item7;
                if chara >= 0 && member < 5 {
                    cpu_level += match self.difficulty {
                        1 => 6,
                        2 => 9,
                        _ => 0,
                    };
                    self.members[side][member] = Some(MemberDef {
                        chara,
                        costume,
                        variant,
                        cpu_level,
                        health: health as f32,
                        // BattleSetup_SetMemberByItemIds: the ids' low halves, plus one.
                        items: ids.map(|i| (i as u16).wrapping_add(1)),
                    });
                    self.s.items[side][member] = ids;
                }
            }
            15 => set_speakers(self.s, gsc),
            // Command 16 "rewards": 15 numbers into the progress block.
            16 => {
                let r = &mut *self.reward;
                for list in [
                    &mut r.points,
                    &mut r.item,
                    &mut r.stage,
                    &mut r.chara,
                    &mut r.episode,
                ] {
                    for v in list.iter_mut() {
                        *v = gsc.int();
                    }
                }
            }
            // Command 701 "load text" (language 0) and 702 "load lip data" (by the voice set).
            701 => self.s.text_file = Some((TEXT_FILE as i32 + self.s.story) as u32),
            702 => {
                let base = LIP_FILE[usize::from(self.s.voice_set)];
                self.s.lip_file = Some((base as i32 + self.s.story) as u32);
            }
            1302 => self.set_window(gsc),
            _ => {}
        }
        true
    }
}

/// Command 15 "speakers": -v line who, any number.
fn set_speakers(s: &mut BtlScript, gsc: &mut Gsc) {
    let mut i = 0;
    while gsc.find_option_n(b'v', i) {
        i += 1;
        let line = gsc.int();
        let who = gsc.int();
        s.set_speaker(line, who);
    }
}

/// The handlers while the battle runs.
struct ScriptHost<'a> {
    b: &'a mut Battle,
    s: &'a mut BtlScript,
    sounds: &'a mut Vec<TitleSound>,
    libc: &'a mut LibcRandom,
}

/// The gauges of member `m` (the active member's are the fighter's).
fn gauge_mut(chr: &mut Chr, m: usize) -> Option<&mut Gauge> {
    if m == chr.active {
        Some(&mut chr.gauge)
    } else {
        chr.members.get_mut(m).map(|x| &mut x.gauge)
    }
}

impl Battle {
    fn chr(&mut self, side: usize) -> &mut Chr {
        &mut self.roster.chars[side & 1]
    }

    /// `BtlCtrl_PlayMotion`: action 4's motion, blend and loop mode, and flag 0xFA (which enters or restarts it).
    fn play_motion(&mut self, side: usize, motion: i32, looping: bool, blend: f32) {
        let c = self.chr(side);
        c.script_motion = motion;
        c.script_motion_blend = blend;
        c.script_motion_loop = i32::from(looping);
        c.flags.set_held(0xFA);
    }

    /// `BtlCtrl_IsMotionPlaying`: always for a looping motion, else until flag 0x31.
    fn motion_playing(&mut self, side: usize) -> bool {
        let c = self.chr(side);
        c.script_motion_loop == 1 || !c.flags.test(0x31)
    }

    /// `BtlCtrl_StopMotion`: flag 0xFA dropped, 0xFB raised (action 4 leaves).
    fn stop_motion(&mut self, side: usize) {
        let c = self.chr(side);
        c.flags.clear(0xFA);
        c.flags.set(0xFB);
    }

    /// `BtlCtrl_SetPos` / `SetRot`: where flags 0xFC / 0xFD put the fighter.
    fn warp(&mut self, side: usize, pos: Option<[f32; 3]>, rot: Option<[f32; 3]>) {
        let c = self.chr(side);
        if let Some(pos) = pos {
            c.warp_pos = pos;
            c.flags.set_held(0xFC);
        }
        if let Some(rot) = rot {
            c.warp_rot = rot;
            c.flags.set_held(0xFD);
        }
    }

    /// `BtlCtrl_SetAuraOn` / `SetAuraOff`: flag 0xFE or 0xFF held, the other dropped.
    fn set_aura(&mut self, side: usize, on: bool) {
        let c = self.chr(side);
        let (set, clear) = if on { (0xFE, 0xFF) } else { (0xFF, 0xFE) };
        c.flags.set_held(set);
        c.flags.clear(clear);
    }

    /// `BtlCtrl_GetActiveMember`.
    fn active_member(&self, side: usize) -> i32 {
        self.roster.chars[side & 1].active as i32
    }

    /// `BtlFacade_SetMemberItems`: a member's items (-1 the fighting one) replaced in the setup and what derives
    /// from them recomputed (`BattleSide_SetMemberItems` / `BtlMember_ApplyItems`, unclamped), then the fighter
    /// asked to copy them (`BtlCtrl_ReloadMemberBonus` / `ReloadMemberAbilities`: held flags 0x103 + member and
    /// 0x108 + member, taken at the end of its phase 3).
    pub(crate) fn set_member_items(&mut self, side: usize, member: i32, ids: [i32; 8]) {
        let ids = ids.map(|i| (i as u16).wrapping_add(1));
        let stats = bt3_data::chara_data::item_set_stats(&self.assets.chara_table, &ids);
        let s = stats.stat;
        let items = MemberItems {
            bonus: [s[2], s[0], s[1], s[2], s[3], s[3], s[3]],
            ability: stats.ability,
        };
        let c = self.chr(side);
        let m = if member < 0 {
            c.active
        } else {
            member as usize
        };
        if m < 5 {
            c.setup_items[m] = items;
            c.flags.set_held(0x103 + m as u32);
            c.flags.set_held(0x108 + m as u32);
        }
    }

    /// `BtlCtrl_AddHp`: percent of the maximum; the fighting member heals or is damaged (flags 0x42B), another
    /// one is written directly.
    fn add_hp(&mut self, side: usize, member: usize, percent: f32) {
        let data = crate::battle::side_data(&self.assets.fighters[side & 1]);
        let env = self.assets.env(self.seq.state_id(), self.gate());
        let c = &mut self.roster.chars[side & 1];
        let active = c.active;
        let Some(g) = gauge_mut(c, member) else {
            return;
        };
        let amount = (g.health_max as f32 * percent * 0.01) as i32;
        if member == active {
            if amount >= 0 {
                c.add_health(amount);
            } else {
                c.damage(&env, &data, -amount, 0x42B);
            }
        } else {
            g.health = clamp(g.health + amount, 0, g.health_max);
        }
    }

    /// `BtlCtrl_RaiseHp` / `LowerHp`: health at least / at most percent of the maximum (1 for a positive percent).
    fn bound_hp(&mut self, side: usize, member: usize, percent: f32, raise: bool) {
        let Some(g) = gauge_mut(self.chr(side), member) else {
            return;
        };
        let mut target = (g.health_max as f32 * percent * 0.01) as i32;
        if target <= 0 && 0.0 < percent {
            target = 1;
        }
        g.health = if raise {
            clamp(target, g.health, g.health_max)
        } else {
            clamp(target, 0, g.health)
        };
    }

    /// `BtlCtrl_AddKi` / `AddBlast`: bars of 20000 / stocks of 100000, through the fighter for the fighting
    /// member.
    fn add_gauge(&mut self, side: usize, member: usize, amount: i32, blast: bool) {
        let c = self.chr(side);
        if member == c.active {
            match (blast, amount >= 0) {
                (false, true) => c.add_ki(amount),
                (false, false) => {
                    c.drain_ki(-amount);
                }
                (true, _) => c.add_blast(amount),
            }
        } else if let Some(g) = gauge_mut(c, member) {
            if blast {
                g.blast = clamp(g.blast + amount, 0, g.blast_max);
            } else {
                g.ki = clamp(g.ki + amount, 0, g.ki_max);
            }
        }
    }

    /// `BtlCtrl_RaiseKi` / `LowerKi` / `RaiseBlast` / `LowerBlast`.
    fn bound_gauge(&mut self, side: usize, member: usize, value: i32, blast: bool, raise: bool) {
        let Some(g) = gauge_mut(self.chr(side), member) else {
            return;
        };
        let (cur, max) = if blast {
            (&mut g.blast, g.blast_max)
        } else {
            (&mut g.ki, g.ki_max)
        };
        *cur = if raise {
            clamp(value, *cur, max)
        } else {
            clamp(value, 0, *cur)
        };
    }

    /// `BtlCtrl_SetMaxPower`: on, the ki filled, the powered-up timer at 30000 and flag 6 held; off, flag 6 dropped.
    fn set_max_power(&mut self, side: usize, on: bool) {
        let c = self.chr(side);
        if on {
            c.add_ki(100_000);
            c.gauge.max_power = (c.gauge.max_power + 30000).min(30000);
            c.flags.set_held(6);
        } else {
            c.flags.clear(6);
        }
    }

    /// The end of a forced request: flag 0xBE dropped and the stun timer cleared.
    fn release(c: &mut Chr) {
        c.flags.clear(0xBE);
        c.stun_timer = 0;
    }

    /// `BtlCtrl_Transform` (`fuse` false) / `BtlCtrl_Fuse`: the transformation (4 slots) or fusion (3 slots) into
    /// `id`, when allowed without its cost: the blast stocks raised to the cost and flag 0x116 / 0x11A + slot held;
    /// a fusion also refills both members' health.
    fn force_change(&mut self, side: usize, id: i32, fuse: bool) -> bool {
        let data = crate::battle::side_data(&self.assets.fighters[side & 1]);
        let env = self.assets.env(self.seq.state_id(), self.gate());
        let p = &data.tables.params;
        let c = &mut self.roster.chars[side & 1];
        if fuse {
            let Some(slot) = (0..3).find(|&i| p.fusion_result(i) == id) else {
                return false;
            };
            let Some(partner) = crate::btl_act::can_fuse(c, &data, &env, slot, false, false) else {
                return false;
            };
            let g = &mut c.gauge;
            g.blast = clamp(p.fusion_cost(slot), g.blast, g.blast_max);
            g.health = g.health_max;
            c.flags.set_held(0x11A + slot as u32);
            Self::release(c);
            if let Some(g) = gauge_mut(c, partner) {
                g.health = g.health_max;
            }
        } else {
            let Some(slot) = (0..4).find(|&i| p.trans_target(i) == id) else {
                return false;
            };
            if !crate::btl_act::can_transform(c, &data, &env, slot as i32, false, false) {
                return false;
            }
            let g = &mut c.gauge;
            g.blast = clamp(p.trans_cost(slot), g.blast, g.blast_max);
            c.flags.set_held(0x116 + slot as u32);
            Self::release(c);
        }
        true
    }

    /// `BtlCtrl_ChangeMember`: the switch to `member` (switch gauge filled, flag 0x11D), or for a fighter that is
    /// down flag 0x10E (story mode replaces it).
    fn change_member(&mut self, side: usize, member: usize) -> bool {
        let c = self.chr(side);
        if member == c.active {
            return false;
        }
        let dead = c.is_dead();
        if !dead && !crate::btl_act::can_switch(c, false, false) {
            return false;
        }
        // BtlMember_SetSwitchTarget.
        c.switch_target = member;
        if member != c.switch_target() {
            return false;
        }
        if dead {
            c.flags.set_held(0x10E);
        } else {
            c.switch_gauge = 100_000;
            c.flags.set_held(0x11D);
            Self::release(c);
        }
        true
    }

    /// `BtlCtrl_UseTechnique`: kind 0 / 1 a skill (blast stocks raised to its cost; lock-on with its flag 1), 2..4 a
    /// technique (ki raised to its cost, flags 0x11F and 5; kind 4 the powered-up mode with full ki); flags 0x120 +
    /// kind (the input) and 0x11E (the pad off) held.
    fn use_technique(&mut self, side: usize, kind: i32) {
        let data = crate::battle::side_data(&self.assets.fighters[side & 1]);
        let c = &mut self.roster.chars[side & 1];
        match kind {
            0 | 1 => {
                let skill = data.tables.skill();
                let cost = (skill.stocks(kind) - i32::from(c.has_ability(0x15))).max(1) * 100_000;
                c.gauge.blast = clamp(cost, c.gauge.blast, c.gauge.blast_max);
                if skill.flags(kind) & 1 != 0 {
                    c.flags.set_held(5);
                }
            }
            2..=4 => {
                let mut ki = data.tables.tech().ki(kind);
                if c.has_ability(0x2B) {
                    ki /= 2;
                }
                c.gauge.ki = clamp(ki, c.gauge.ki, c.gauge.ki_max);
                if kind == 4 {
                    c.flags.set_held(6);
                    c.gauge.max_power = 30000;
                    c.gauge.ki = c.gauge.ki_max;
                }
                c.flags.set_held(0x11F);
                c.flags.set_held(5);
            }
            _ => return,
        }
        c.flags.set_held(0x120 + kind as u32);
        c.flags.set_held(0x11E);
        Self::release(c);
        c.tech_delay = 0;
    }

    /// `BtlCtrl_ForceReaction`: the waiting action 7 + kind (flag 0x112 + kind).
    fn force_wait(&mut self, side: usize, kind: u32) {
        let c = self.chr(side);
        if kind < 4 {
            c.flags.set_held(0x112 + kind);
            Self::release(c);
        }
    }

    /// `BtlCtrl_ForceFlag11x`: action 5 (kind 0) or 6 (kind 1), the scripted dash.
    fn force_dash(&mut self, side: usize, kind: u32) {
        let c = self.chr(side);
        c.flags.set_held(0x110 + kind.min(1));
        Self::release(c);
    }

    /// `BtlFacade_AreBothInterruptible`.
    fn both_interruptible(&self) -> bool {
        self.roster.chars.iter().all(Chr::is_interruptible)
    }

    /// `BtlStage_GetStartPlace(side, ..., 0)`: the stage's starting place, facing its target.
    fn start_place(&self, side: usize) -> ([f32; 3], [f32; 3]) {
        let place = self.assets.stage.starting_placements.get(side);
        let pos = place.map_or([0.0; 3], |p| p.position);
        let target = place.map_or([0.0, 0.0, 1.0], |p| p.target);
        let yaw = (target[0] - pos[0]).atan2(target[2] - pos[2]);
        (pos, [0.0, yaw, 0.0])
    }

    /// Both fighters at their starting places, standing (looping motion 0).
    fn place_both_at_start(&mut self) {
        for side in 0..2 {
            let (pos, rot) = self.start_place(side);
            self.warp(side, Some(pos), Some(rot));
        }
        for side in 0..2 {
            self.play_motion(side, 0, true, 0.0);
        }
    }

    /// `Voice_IsStopped(player)`: not playing, and not asked to play this frame.
    fn voice_stopped(&self, player: usize) -> bool {
        let p = player & 1;
        !self.voice_req[p] && matches!(self.stream_stat.voice[p], 0 | crate::anim::VOICE_PLAYEND)
    }

    /// `Voice_PlayDefault(player, file)`.
    fn play_voice(&mut self, player: usize, file: u32, sounds: &mut Vec<TitleSound>) {
        self.voice_req[player & 1] = true;
        sounds.push(TitleSound::VoiceEx {
            player: (player & 1) as u32,
            id: file,
            volume: VOICE_VOLUME,
            pan: 0,
        });
    }

    /// `BtlFacade_EndObjSubState3`: the mouth stops talking.
    fn end_talk(&mut self, side: usize) {
        let obj = &mut self.chr(side).obj;
        if obj.mouth_mode() == 3 {
            obj.set_mouth(0);
        }
    }

    /// `BtlFacade_LowerHp`'s events of a percent of 0 or less: the KO, 0x4A for the side and 0x48 for the other.
    fn raise_ko(&mut self, side: usize) {
        self.events.raise(side, 0x4A);
        self.events.raise(side ^ 1, 0x48);
    }

    /// `BtlFacade_SetResult`.
    fn set_script_result(&mut self, kind: i32) {
        use crate::battle_seq::{WIN_P1, WIN_P2};
        let (winner, reason) = match kind {
            0 => (WIN_P2, 1),
            1 => (WIN_P1, 1),
            2 => (WIN_P2, 4),
            3 => (WIN_P1, 4),
            _ => return,
        };
        self.seq.winner = winner;
        self.seq.reason = reason;
    }

    /// `Fade_Start(slot, dir, seconds)` on slot 0 (black) or 1.
    fn start_fade(&mut self, slot: usize, fade_in: bool, seconds: f32) {
        let colors = if slot == 0 {
            [[0.0; 4], [0.0, 0.0, 0.0, 128.0]]
        } else {
            self.assets.fade1_colors
        };
        *self.fade_slot(slot) = crate::fade::Fade::start_colored(colors, fade_in, seconds);
    }

    fn fade_slot(&mut self, slot: usize) -> &mut crate::fade::Fade {
        if slot == 0 {
            &mut self.fade
        } else {
            &mut self.fade1
        }
    }

    /// `BtlEvent_BeginInterrupt`: event 0x4C asked for, the fighters held, the HUD slid out over a second.
    fn begin_interrupt(&mut self) {
        self.events.interrupt = true;
        self.seq.demo_flag = true;
        self.hud.slide(true, 1.0);
    }

    /// Whether the story script runs an event action (`BtlScript_IsEventRunning`).
    pub(crate) fn script_event_running(&self) -> bool {
        self.script.as_ref().is_some_and(|s| s.is_event_running())
    }

    /// `BtlScript_AbortEvents`.
    pub(crate) fn abort_script_events(&mut self) {
        if let Some(s) = &mut self.script {
            s.abort_events();
        }
    }

    /// `BtlScript_GetCurrentEvent`.
    pub(crate) fn script_event(&self) -> Option<ScriptEvent> {
        self.script.as_ref().and_then(|s| s.current_event())
    }

    /// `BtlScript_IsTextShown`.
    pub(crate) fn script_text_shown(&self) -> bool {
        self.script.as_ref().is_some_and(|s| s.is_text_shown())
    }

    /// `Battle_ResetWork` in mode 1: `BtlScript_Restart` and the main action started.
    pub(crate) fn script_start(&mut self, sounds: &mut Vec<TitleSound>) {
        if let Some(s) = &mut self.script {
            s.start_main(sounds);
        }
    }

    /// `BtlEvent_Raise` from the fighters' lists of the frame; events 0x5A / 0x5B also start the script's "wait
    /// flag off" event (`BtlScript_StartWaitEvents`).
    pub(crate) fn take_events(&mut self, sounds: &mut Vec<TitleSound>) {
        for side in 0..2 {
            for i in 0..self.roster.chars[side].events.len() {
                let (s, ev) = self.roster.chars[side].events[i];
                self.events.raise(s, ev);
                if matches!(ev, 0x5A | 0x5B)
                    && let Some(script) = &mut self.script
                {
                    script.start_wait_events(sounds);
                }
            }
            self.roster.chars[side].events.clear();
        }
    }

    /// `BtlEvent_Update` (`Battle_UpdateWork`, unpaused): `BtlEvent_UpdateRequests` (the interrupt's event 0x4C
    /// once both fighters can be interrupted, the wait flag dropped on events 0x4D / 0x4E, the fixed camera off on
    /// event 0x4F), the sets rotated, then the time events of the sub clock.
    pub(crate) fn update_events(&mut self) {
        if self.events.interrupt && self.both_interruptible() {
            self.events.raise(0, 0x4C);
            self.events.raise(1, 0x4C);
            self.seq.demo_flag = false;
            self.events.interrupt = false;
        }
        if self.events.wait_flag
            && (0..2).any(|s| self.events.is_new(s, 0x4D) || self.events.is_new(s, 0x4E))
        {
            self.events.wait_flag = false;
        }
        if (0..2).any(|s| self.events.is_new(s, 0x4F)) {
            self.demo.fixed = false;
        }
        self.events.rotate();
        self.events.raise_time_events(self.seq.sub_seconds());
    }

    /// Step 4 of `Battle_Loop` (unpaused): `Gsc_Update`, then `BtlScript_Update` in sequence states 3 and 5.
    pub(crate) fn script_step(&mut self, sounds: &mut Vec<TitleSound>, libc: &mut LibcRandom) {
        let Some(mut s) = self.script.take() else {
            return;
        };
        let mut gsc = std::mem::take(&mut s.gsc);
        gsc.update(&mut ScriptHost {
            b: self,
            s: &mut s,
            sounds: &mut *sounds,
            libc,
        });
        s.gsc = gsc;
        self.script = Some(s);
        // The events the commands raised through the fighters (damage) are in this frame's set.
        self.take_events(sounds);
        let Some(mut s) = self.script.take() else {
            return;
        };
        if matches!(self.seq.state_id(), 3 | 5) {
            self.update_lines(&mut s);
            self.update_requests(&mut s, sounds);
            self.start_pending(&mut s, sounds);
            self.update_cue(&s);
        }
        self.script = Some(s);
        let shown = self.script_text_shown();
        self.assets
            .snd
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .script_text = shown;
    }

    /// `BtlScript_GetSpeakerSlot`: the speaker's side, and window 0 when it is the fighting member, else 1.
    fn speaker_slot(&self, sp: Speaker) -> (usize, usize) {
        let window =
            usize::from(!(sp.member != NO_MEMBER && self.active_member(sp.side) == sp.member));
        (sp.side, window)
    }

    /// `BtlScript_UpdateLines`: 60 frames after a line's voice stopped its text goes and its slot is freed.
    fn update_lines(&self, s: &mut BtlScript) {
        for i in 0..4 {
            let Some(r) = s.voice[i] else {
                continue;
            };
            let Req::Line(mut line) = s.requests[r].req else {
                continue;
            };
            let (side, _) = self.speaker_slot(s.speaker(line.id));
            if !self.voice_stopped(side) {
                continue;
            }
            line.timer += 10;
            if line.timer > 600 {
                s.clear_text(line.id);
                line.timer = 0;
                if line.once {
                    line.enabled = false;
                }
                s.voice[i] = None;
            }
            s.requests[r].req = Req::Line(line);
        }
    }

    /// `BtlScript_IsTriggered`: 0xFFFF the wait flag off, else an event new this frame on the trigger's side
    /// (with `range`, events 0x16..0x1C also fire on any later one up to 0x1C).
    fn is_triggered(&self, trigger: i32, range: bool) -> bool {
        if trigger == TRIGGER_WAIT_OFF {
            return !self.events.wait_flag;
        }
        let side = script_side(trigger);
        let ev = (trigger & 0xFFF) as u32;
        if range && (0x16..=0x1C).contains(&ev) {
            (ev..=0x1C).any(|e| self.events.is_new(side, e))
        } else {
            self.events.is_new(side, ev)
        }
    }

    /// `BtlScript_UpdateRequests`: lines said, and the event action to start chosen.
    fn update_requests(&mut self, s: &mut BtlScript, sounds: &mut Vec<TitleSound>) {
        for i in 0..s.requests.len() {
            let r = s.requests[i];
            match r.req {
                Req::Line(line) => {
                    if line.enabled
                        && s.voice.iter().all(Option::is_none)
                        && self.is_triggered(r.trigger, false)
                    {
                        // BtlScript_PlayLine.
                        let (side, window) = self.speaker_slot(s.speaker(line.id));
                        if s.voice[side * 2 + window].is_none() {
                            self.play_voice(side, s.voice_file(line.id), sounds);
                            s.voice[side * 2 + window] = Some(i);
                            s.set_text(window, line.id);
                        }
                    }
                }
                Req::Event(action) => {
                    if !self.is_triggered(r.trigger, true) {
                        continue;
                    }
                    let group = s.event_of(action).map_or(0, |e| s.events[e].group);
                    if s.pending.is_none()
                        || (1..=4).contains(&group)
                        || matches!(r.trigger & 0xFFF, 0x48 | 0x4A)
                    {
                        s.pending = Some(i);
                    }
                }
            }
        }
    }

    /// `BtlScript_StartPending`: the pending event replaces the running task once the fight allows it. With
    /// command 0x4B1, at once for the triggers 0xFFFF, 0x22 and 0x49, else when both fighters can be interrupted
    /// and a fighter is down, the event is side 1's or event 0x50 is new on side 0; a switch / transformation
    /// event the same with "its side can act"; any other at once, or with a fighter down once both can be
    /// interrupted.
    fn start_pending(&mut self, s: &mut BtlScript, sounds: &mut Vec<TitleSound>) {
        let Some(p) = s.pending else {
            return;
        };
        let r = s.requests[p];
        let Req::Event(action) = r.req else {
            return;
        };
        let info = s.event_of(action).map(|e| s.events[e]).unwrap_or_default();
        let dead = [0, 1].map(|i| self.roster.chars[i].is_dead());
        let check = |ready: bool| {
            ready && (dead[0] || dead[1] || info.side != 0 || self.events.is_new(0, 0x50))
        };
        let ok = if info.cmd4b1 {
            r.trigger == TRIGGER_WAIT_OFF
                || matches!(r.trigger & 0xFFF, 0x22 | 0x49)
                || check(self.both_interruptible())
        } else if info.kind != 0 {
            check(self.roster.chars[info.side].can_act())
        } else if dead[0] || dead[1] {
            self.both_interruptible()
        } else {
            true
        };
        if ok {
            s.start_event(action, sounds);
        }
    }

    /// `BtlScript_UpdateHud`: while an event of side 0 is pending and nobody is down, the cue, dim while the fighter
    /// cannot act; otherwise it goes.
    fn update_cue(&mut self, s: &BtlScript) {
        let pending = s
            .pending
            .and_then(|p| match s.requests.get(p).map(|r| r.req) {
                Some(Req::Event(action)) => {
                    Some(s.event_of(action).map(|e| s.events[e]).unwrap_or_default())
                }
                _ => None,
            });
        let dead = self.roster.chars.iter().any(Chr::is_dead);
        let cue = match pending {
            Some(info) if !dead && info.side == 0 => Some(self.roster.chars[0].can_act()),
            _ => None,
        };
        self.hud.set_cue(cue);
    }

    /// `BtlScript_UpdateView` (`BtlGame_Draw`, every frame): the kept shake, the camera move, the text.
    pub(crate) fn script_update_view(&mut self, font: Option<&mut Font>) {
        let Some(s) = self.script.as_mut() else {
            return;
        };
        if s.shake_time != 0.0 {
            self.demo.add_shake(s.shake_time);
        }
        let cam = &mut s.cam;
        if cam.active {
            let i = cam.index;
            if i < 8 && cam.key[i + 1].used {
                let (key, next) = (cam.key[i], cam.key[i + 1]);
                let total = (next.time * 300.0) as i32;
                cam.time += 10;
                let t = (cam.time as f32 / total as f32).min(1.0);
                let pos = std::array::from_fn(|k| (next.pos[k] - key.pos[k]) * t + key.pos[k]);
                let rot = std::array::from_fn(|k| (next.rot[k] - key.rot[k]) * t + key.rot[k]);
                self.demo.set_fixed(pos, rot);
                if total < cam.time {
                    cam.time = 0;
                    cam.index += 1;
                }
            } else {
                cam.active = false;
                cam.time = 0;
                cam.index = 0;
            }
        }
        if let Some(font) = font
            && s.text_id >= 0
        {
            let w = s.windows[s.text_window];
            let text = s.shown.as_slice();
            font.push_style();
            font.style = Style {
                clip: font.style.clip,
                flags: font.style.flags,
                color2: font.style.color2,
                ..w.style
            };
            // Font_GetHeight.
            let st = &font.style;
            let height = Font::count_lines(text)
                * ((font.line_height() as f32 * st.scale_y * st.scale) as i32 + st.spacing_y)
                - st.spacing_y;
            font.print_at(w.x, w.y - height, text);
            font.pop_style();
        }
    }
}

impl ScriptHost<'_> {
    fn begin(phase: u32) -> bool {
        phase == PHASE_BEGIN || phase == PHASE_BEGIN_ABORT
    }

    fn vec3(gsc: &mut Gsc) -> [f32; 3] {
        let x = gsc.float();
        let y = gsc.float();
        let z = gsc.float();
        [x, y, z]
    }

    fn rot3(gsc: &mut Gsc) -> [f32; 3] {
        Self::vec3(gsc).map(deg)
    }

    /// Command 1202 "camera move": -l seconds x y z rx ry rz (8 at most) [-w], from the fixed pose.
    fn move_camera(&mut self, gsc: &mut Gsc, phase: u32) -> bool {
        let cam = &mut self.s.cam;
        match phase {
            PHASE_BEGIN => {
                cam.active = false;
                cam.time = 0;
                cam.index = 0;
                for k in &mut cam.key {
                    k.used = false;
                }
                if gsc.find_option(b'l') {
                    let (pos, rot) = self.b.demo.fixed_pose();
                    cam.key[0] = CamKey {
                        used: true,
                        time: 0.0,
                        pos,
                        rot,
                    };
                    let mut i = 0;
                    while gsc.find_option_n(b'l', i) && i < 8 {
                        i += 1;
                        let time = gsc.float();
                        let pos = Self::vec3(gsc);
                        let rot = Self::rot3(gsc);
                        cam.key[i] = CamKey {
                            used: true,
                            time,
                            pos,
                            rot,
                        };
                    }
                    cam.active = true;
                }
                !gsc.find_option(b'w')
            }
            PHASE_UPDATE => !cam.active,
            PHASE_BEGIN_ABORT => {
                // The last key takes the camera's pose; the move does not run.
                cam.active = false;
                let last = (0..8).find(|&k| !cam.key[k + 1].used).unwrap_or(8);
                let (pos, rot) = self.b.demo.fixed_pose();
                cam.key[last].pos = pos;
                cam.key[last].rot = rot;
                false
            }
            _ => false,
        }
    }

    /// Commands 1001 / 1002 "fade in" / "fade out": seconds [-W] [-w]; slot 0 (black) or with -W slot 1 (white);
    /// -w waits for the fade and turns the slot off at the end.
    fn fade(&mut self, gsc: &mut Gsc, phase: u32, fade_in: bool) -> bool {
        match phase {
            PHASE_BEGIN | PHASE_BEGIN_ABORT => {
                let time = gsc.float();
                let slot = i32::from(gsc.find_option(b'W'));
                let wait = gsc.find_option(b'w');
                let w = gsc.work();
                w[0] = slot;
                w[1] = 0;
                w[2] = i32::from(wait);
                self.b.start_fade(slot as usize, fade_in, time);
                !wait
            }
            PHASE_UPDATE => {
                let slot = gsc.work()[0] as usize;
                if self.b.fade_slot(slot).is_done() {
                    let w = gsc.work();
                    let done = w[1] > 0;
                    w[1] += 1;
                    done
                } else {
                    false
                }
            }
            PHASE_END => {
                let w = *gsc.work();
                if w[2] != 0 {
                    *self.b.fade_slot(w[0] as usize) = crate::fade::Fade::default();
                }
                false
            }
            _ => false,
        }
    }

    /// Command 1603 "talk": who line [-h channel] [-w]. The line's voice, its text (window 0 for the fighting
    /// member) and, for the fighting member, its lip track once the voice sounds; -w waits for the voice and 60
    /// frames more.
    fn talk(&mut self, gsc: &mut Gsc, phase: u32) -> bool {
        match phase {
            PHASE_BEGIN => {
                let who = gsc.int();
                let member = who & 0xFFF;
                let side = script_side(who);
                let line = gsc.int();
                let channel = if gsc.find_option(b'h') { gsc.int() } else { 0 };
                let done = !gsc.find_option(b'w');
                let active = self.b.active_member(side) == member;
                self.s.set_text(usize::from(!active), line);
                // Adx_Play: not while the channel's side has a line in its slots.
                let ch = channel as usize & 1;
                if !self.s.slot_busy(ch, 0) && !self.s.slot_busy(ch, 1) {
                    self.b
                        .play_voice(channel as usize, self.s.voice_file(line), self.sounds);
                }
                let lip = active && self.s.lip(line).is_some();
                let w = gsc.work();
                w[0] = channel & 1;
                w[1] = 600;
                w[2] = line;
                w[3] = side as i32;
                w[4] = i32::from(lip);
                w[5] = 0;
                done
            }
            PHASE_UPDATE => {
                let w = *gsc.work();
                let (channel, side) = (w[0] as usize, w[3] as usize);
                if w[4] != 0 && w[5] == 0 && self.b.stream_stat.voice[channel] == ADX_PLAYING {
                    if let Some(keys) = self.s.lip(w[2]) {
                        self.b.chr(side).obj.play_lip(keys, self.libc);
                    }
                    gsc.work()[5] = 1;
                }
                if self.b.voice_stopped(channel) {
                    self.b.end_talk(side);
                    let w = gsc.work();
                    w[1] -= 10;
                    return w[1] <= 0;
                }
                false
            }
            PHASE_END => {
                let line = gsc.work()[2];
                self.s.clear_text(line);
                false
            }
            PHASE_BEGIN_ABORT => {
                if gsc.is_current_begun() {
                    let w = *gsc.work();
                    self.sounds.push(TitleSound::VoiceStopEx(w[0] as u32));
                    self.b.end_talk(w[3] as usize);
                }
                true
            }
            _ => false,
        }
    }

    /// Commands 13 / 14 "player status" / "enemy status": [-C member] [-h n] [-H n] [-f n] [-F n] [-b n] [-B n]
    /// [-0..-6 item] (and for side 1 [-l cpuLevel] [-a item7]).
    fn set_status(&mut self, gsc: &mut Gsc, side: usize) {
        let mut member = self.b.active_member(side);
        if gsc.find_option(b'C') {
            member = gsc.int();
        }
        // BtlFacade_*: member -1 is the fighting one.
        if member < 0 {
            member = self.b.active_member(side);
        }
        let m = member as usize;
        if side == 1 && gsc.find_option(b'l') {
            let level = gsc.int() + self.level_bonus();
            // BtlFacade_SetCpuLevel: both sides.
            self.b.set_ai_level(0, level);
            self.b.set_ai_level(1, level);
        }
        if gsc.find_option(b'h') {
            let v = gsc.int();
            if v > 0 {
                self.b.bound_hp(side, m, v as f32, true);
            } else {
                if -v <= 0 {
                    self.b.raise_ko(side);
                }
                self.b.bound_hp(side, m, -v as f32, false);
            }
        }
        if gsc.find_option(b'H') {
            let v = gsc.int();
            self.b.add_hp(side, m, v as f32);
        }
        if gsc.find_option(b'f') {
            let v = gsc.int();
            self.b.bound_gauge(side, m, v.abs() * 20000, false, v > 0);
        }
        if gsc.find_option(b'F') {
            let v = gsc.int();
            self.b.add_gauge(side, m, v * 20000, false);
        }
        if gsc.find_option(b'b') {
            let v = gsc.int();
            self.b.bound_gauge(side, m, v.abs() * 100_000, true, v > 0);
        }
        // -B goes to the ki like -F, as the original's call does.
        if gsc.find_option(b'B') {
            let v = gsc.int();
            self.b.add_gauge(side, m, v * 20000, false);
        }
        let mut items = self
            .s
            .items
            .get(side)
            .and_then(|s| s.get(m))
            .copied()
            .unwrap_or([0; 8]);
        for (k, letter) in b"0123456".iter().enumerate() {
            if gsc.find_option(*letter) {
                items[k] = gsc.int();
            }
        }
        if side == 1 && gsc.find_option(b'a') {
            items[7] = gsc.int();
        }
        if let Some(slot) = self.s.items.get_mut(side).and_then(|s| s.get_mut(m)) {
            *slot = items;
        }
        self.b.set_member_items(side, member, items);
    }

    /// The CPU level's raise by the difficulty (+6 / +9).
    fn level_bonus(&self) -> i32 {
        match self.b.assets.story_difficulty {
            1 => 6,
            2 => 9,
            _ => 0,
        }
    }

    /// Command 8 "battle": what arg; `what` is side << 15 | the sub-command.
    fn battle(&mut self, gsc: &mut Gsc) {
        // BtlFacade_BeginWait.
        self.b.events.wait_flag = false;
        let what = gsc.int();
        let arg = gsc.int();
        let side = script_side(what);
        let b = &mut *self.b;
        let wait_on = |b: &mut Battle, ok: bool| {
            if ok {
                b.events.wait_flag = true;
            }
        };
        match what & 0xFFF {
            1 | 2 => b.seq_call_extra(),
            // BtlFacade_RequestStageChange.
            3 => b.stage_request_change(),
            4 => {
                for s in 0..2 {
                    b.force_dash(s, 0);
                }
            }
            5..=7 => {
                for s in 0..2 {
                    b.force_dash(s, 1);
                }
            }
            n @ 8..=12 => {
                let kinds = match n {
                    8 => [2, 2],
                    9 => [2, 3],
                    10 => [3, 2],
                    11 => [3, 3],
                    _ => [4, 4],
                };
                for (s, kind) in kinds.into_iter().enumerate() {
                    b.use_technique(s, kind);
                }
            }
            n @ 16..=20 => {
                let ok = b.change_member(side, (n - 16) as usize);
                wait_on(b, ok);
            }
            21 | 22 => {
                let ok = b.force_change(side, arg, false);
                wait_on(b, ok);
            }
            23 => {
                let ok = b.force_change(side, arg, true);
                wait_on(b, ok);
            }
            24 => b.set_max_power(side, true),
            n @ 25..=29 => {
                // BtlFacade_ForceAction01 / 23 / 4: the other side waits, this side uses the technique.
                b.force_wait(side ^ 1, 0);
                let kind = match n {
                    25 => 0,
                    26 => 1,
                    27 => 2,
                    28 => 3,
                    _ => 4,
                };
                b.use_technique(side, kind);
            }
            _ => {}
        }
        b.seq.reset_sub_clock();
    }
}

impl GscCommands for ScriptHost<'_> {
    fn command(&mut self, gsc: &mut Gsc, id: u32, phase: u32) -> bool {
        let begin = Self::begin(phase);
        match id {
            // Command 1 "wait": seconds of task time (300 a second).
            1 => match phase {
                PHASE_UPDATE => gsc.current_time() >= (gsc.float() * 300.0) as i32,
                PHASE_BEGIN_ABORT => true,
                _ => false,
            },
            2 => {
                if begin {
                    self.s.story = gsc.int();
                }
                true
            }
            // Command 4 "event end": the moves and the fixed camera end, but for a mid-fight event with a camera.
            4 => {
                if begin {
                    let ev = self.s.current_event();
                    if ev.is_none_or(|e| e.group == 0 || !e.cmd4b1) {
                        self.b.stop_motion(0);
                        self.b.stop_motion(1);
                        self.b.demo.fixed = false;
                    }
                }
                true
            }
            // Command 5 "scene begin": both fighters at their starting places, standing, for the main action or an
            // event with a camera (which first interrupts the fight and resets the fighters' effects).
            5 => {
                if begin {
                    match self.s.current_event() {
                        Some(e) if e.cmd4b1 => {
                            self.b.begin_interrupt();
                            for side in 0..2 {
                                let b = &mut *self.b;
                                b.roster.scene.reset_side(&mut b.roster.chars, side);
                            }
                            self.b.place_both_at_start();
                        }
                        None => self.b.place_both_at_start(),
                        Some(_) => {}
                    }
                }
                true
            }
            // Command 6 "scene end": the fighters back to the fight (aura off, charge off, no shake) under the same
            // condition; an event with a camera also ends the interrupt (`BtlEvent_EndInterrupt`: the HUD back).
            6 => {
                let ev = self.s.current_event();
                if begin && ev.is_none_or(|e| e.cmd4b1) {
                    for side in 0..2 {
                        self.b.set_aura(side, false);
                        self.b.chr(side).flags.clear(0x101);
                    }
                    self.b.demo.add_shake(0.0);
                    if ev.is_some() {
                        self.b.hud.slide(false, 1.0);
                    }
                }
                true
            }
            // Command 7 "triggers": -a trigger action, -v trigger line.
            7 => {
                if begin {
                    self.s.reset_view(self.sounds);
                    let mut i = 0;
                    while gsc.find_option_n(b'a', i) {
                        i += 1;
                        let trigger = gsc.int();
                        let action = gsc.int();
                        self.s.add_request(trigger, Req::Event(action));
                    }
                    let mut i = 0;
                    while gsc.find_option_n(b'v', i) {
                        i += 1;
                        let trigger = gsc.int();
                        let id = gsc.int();
                        let line = Line {
                            enabled: true,
                            once: true,
                            timer: 0,
                            id,
                        };
                        self.s.add_request(trigger, Req::Line(line));
                    }
                }
                true
            }
            8 => {
                if begin {
                    self.battle(gsc);
                }
                true
            }
            // Command 9 "result": n >= 0 ends the battle with result n, then waits until the finish has passed.
            9 => match phase {
                PHASE_BEGIN | PHASE_BEGIN_ABORT => {
                    gsc.work()[0] = 0;
                    let result = gsc.int();
                    if result >= 0 {
                        self.b.set_script_result(result);
                        false
                    } else {
                        true
                    }
                }
                PHASE_UPDATE => {
                    if self.b.seq.state_id() != 4 {
                        let w = gsc.work();
                        let done = w[0] > 0;
                        w[0] += 1;
                        done
                    } else {
                        false
                    }
                }
                _ => false,
            },
            13 | 14 => {
                if begin {
                    self.set_status(gsc, (id - 13) as usize);
                }
                true
            }
            15 => {
                if begin {
                    set_speakers(self.s, gsc);
                }
                true
            }
            // Commands 801..803 "place": side, then x y z and / or rx ry rz (degrees).
            801..=803 => {
                if begin {
                    let side = script_side(gsc.int());
                    let pos = (id != 803).then(|| Self::vec3(gsc));
                    let rot = (id != 802).then(|| Self::rot3(gsc));
                    self.b.warp(side, pos, rot);
                }
                true
            }
            // Commands 804..810: the scripted action's control flags (hidden 0x100, aura 0xFE / 0xFF, charge 0x101,
            // its burst 0x102); 808 not while the task aborts.
            804..=810 => {
                if begin && !(id == 808 && phase == PHASE_BEGIN_ABORT) {
                    let side = script_side(gsc.int());
                    let flags = &mut self.b.chr(side).flags;
                    match id {
                        804 => flags.clear(0x100),
                        805 => flags.set_held(0x100),
                        806 => self.b.set_aura(side, true),
                        807 => self.b.set_aura(side, false),
                        808 => flags.set_held(0x101),
                        809 => flags.clear(0x101),
                        _ => flags.set_held(0x102),
                    }
                }
                true
            }
            // Command 901 "move": who type [-t blend] [-l] [-w]: the scripted motion (looping with -l); -w without -l
            // waits until it has played.
            901 => match phase {
                PHASE_BEGIN | PHASE_BEGIN_ABORT => {
                    let side = script_side(gsc.int());
                    let motion = gsc.int();
                    let blend = if gsc.find_option(b't') {
                        gsc.float()
                    } else {
                        0.0
                    };
                    let looping = gsc.find_option(b'l');
                    let done = !gsc.find_option(b'w') || looping;
                    self.b.chr(side).flags.clear(0x100);
                    gsc.work()[0] = side as i32;
                    self.b.play_motion(side, motion, looping, blend);
                    done || phase == PHASE_BEGIN_ABORT
                }
                PHASE_UPDATE => {
                    let side = gsc.work()[0] as usize;
                    !self.b.motion_playing(side)
                }
                _ => false,
            },
            902 => {
                if begin {
                    self.b.stop_motion(0);
                    self.b.stop_motion(1);
                }
                true
            }
            1001 => self.fade(gsc, phase, true),
            1002 => self.fade(gsc, phase, false),
            1003 => {
                if begin {
                    for slot in 0..2 {
                        *self.b.fade_slot(slot) = crate::fade::Fade::default();
                    }
                }
                true
            }
            // Command 1201 "camera": x y z rx ry rz: the camera move stopped and the camera fixed there.
            1201 => {
                if begin {
                    let cam = &mut self.s.cam;
                    cam.active = false;
                    cam.time = 0;
                    cam.index = 0;
                    let pos = Self::vec3(gsc);
                    let rot = Self::rot3(gsc);
                    self.b.demo.set_fixed(pos, rot);
                }
                begin
            }
            1202 => self.move_camera(gsc, phase),
            1203 => {
                if begin {
                    self.b.demo.fixed = false;
                }
                true
            }
            // Command 1204 "shake": seconds [-l] (kept up until 1205).
            1204 => {
                if phase == PHASE_BEGIN {
                    let time = gsc.float();
                    self.b.demo.add_shake(time);
                    if gsc.find_option(b'l') {
                        self.s.shake_time = time;
                    }
                }
                begin
            }
            1205 => {
                if begin {
                    self.s.shake_time = 0.0;
                    self.b.demo.add_shake(0.0);
                }
                true
            }
            1501 => {
                if begin {
                    let n = gsc.int();
                    self.sounds
                        .push(TitleSound::Bgm((n + BGM_FIRST as i32) as u32));
                }
                true
            }
            // Command 1502 "music off": [-w]: one fade-out step; -w waits until the stream has stopped.
            1502 => match phase {
                PHASE_BEGIN | PHASE_BEGIN_ABORT => {
                    self.sounds.push(TitleSound::BgmFadeOutStep);
                    !gsc.find_option(b'w')
                }
                PHASE_UPDATE => !self.b.stream_stat.bgm_playing,
                _ => false,
            },
            1601 => {
                if phase == PHASE_BEGIN {
                    let n = gsc.int();
                    self.sounds.push(TitleSound::Effect(4, n as u32));
                }
                true
            }
            // Command 1602 "voice": line [-h] [-w]: the line's voice on player 0, no text.
            1602 => match phase {
                PHASE_BEGIN => {
                    let line = gsc.int();
                    let done = !gsc.find_option(b'w');
                    gsc.work()[0] = 0;
                    if !self.s.slot_busy(0, 0) && !self.s.slot_busy(0, 1) {
                        self.b.play_voice(0, self.s.voice_file(line), self.sounds);
                    }
                    done
                }
                PHASE_UPDATE => self.b.voice_stopped(gsc.work()[0] as usize),
                PHASE_BEGIN_ABORT => {
                    if gsc.is_current_begun() {
                        self.sounds
                            .push(TitleSound::VoiceStopEx(gsc.work()[0] as u32 & 1));
                    }
                    true
                }
                _ => false,
            },
            1603 => self.talk(gsc, phase),
            // Command 1701 "wait button": until pad 0's game button `bit` is pressed, then one more frame.
            1701 => match phase {
                PHASE_BEGIN => {
                    let bit = gsc.int();
                    let w = gsc.work();
                    w[0] = bit;
                    w[1] = 0;
                    false
                }
                PHASE_UPDATE => {
                    let pressed = self.b.pad0_pressed;
                    let w = gsc.work();
                    if w[1] != 0 {
                        true
                    } else {
                        if pressed & 1u32.checked_shl(w[0] as u32).unwrap_or(0) != 0 {
                            w[1] = 1;
                        }
                        false
                    }
                }
                PHASE_BEGIN_ABORT => true,
                _ => false,
            },
            // The setup commands (10..12, 16, 701, 702, 1302) only run in action 1000; 3 and 1301 do nothing.
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Story script 0 (Saga 0's first episode) from the ISO in `rom/`, when it is there: its setup action and its
    /// event actions.
    #[test]
    fn real_script_sets_up_the_story_battle() {
        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = bt3_data::Iso9660::open(&iso) else {
            return;
        };
        let file = crate::read_file(&mut image, SCRIPT_FILE).unwrap();
        let script = GscScript::parse(&file).unwrap();
        let mut reward = HistReward::default();
        let mut libc = LibcRandom::new(0);
        let ScriptSetup { setup, script } =
            BtlScript::setup(script, 0, true, &mut reward, &mut libc);
        assert_eq!(setup.mode, 1);
        assert_eq!(script.story, 0);
        assert_eq!((setup.sides[0].control, setup.sides[1].control), (0, 2));
        assert!(setup.sides.iter().all(|s| s.members.is_some()));
        assert_eq!(script.text_file, Some(TEXT_FILE));
        assert_eq!(script.lip_file, Some(LIP_FILE[1]));
        assert!(reward.points[0] > 0, "{reward:?}");
        // Command 1302 set up the windows of language 1.
        assert!(script.windows.iter().any(|w| w.y != 0));
        // Every script has event actions; the ones with a camera command are scenes.
        assert!(script.events.iter().any(|e| e.present && e.cmd4b1));
        assert_eq!(script.voice_file(3), VOICE_FILE[1] + 3);
    }

    /// Every story script on the ISO: none waits for a button (command 1701) or changes the stage (command 8's
    /// sub-command 3), so those two run in no story battle.
    #[test]
    fn real_story_scripts_never_wait_for_a_button_nor_change_the_stage() {
        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = bt3_data::Iso9660::open(&iso) else {
            return;
        };
        let mut scripts = 0;
        for n in 0..0x200 {
            let Some(script) = crate::read_file(&mut image, SCRIPT_FILE + n)
                .ok()
                .and_then(|f| GscScript::parse(&f).ok())
            else {
                continue;
            };
            scripts += 1;
            for (a, action) in script.actions.iter().enumerate() {
                let mut at = 0;
                while at < action.words.len() {
                    let word = script.word(a, at);
                    if word & 0xF == WORD_COMMAND {
                        let id = word >> 16;
                        assert_ne!(id, 1701, "script {n}");
                        if id == 8 {
                            let what = script.word(a, at + 1);
                            let v = script.value(what).map(i32::from_le_bytes).unwrap_or(-1);
                            assert_ne!(v & 0xFFF, 3, "script {n}");
                        }
                        at = script.next_command(a, at);
                    } else {
                        at += 1;
                    }
                }
            }
        }
        // Files 0x1FF..0x230: the 50 story battles.
        assert_eq!(scripts, 50);
    }
}
