//! Memory-card dialog flows (src/sys/memcard_flow.c) over the card layer (`crate::memcard`).
//!
//! One work area runs the confirmation dialog through one of the state machines, one per kind of card
//! operation: saving or loading the system data, the boot load, the new game's save, and the replay files' save,
//! load, scan and question. State 0x23 opens the window, the flow's own states follow, 0x25 closes it, 0x26 waits
//! for the close, then the flow is idle and reports how it ended. `SndOpt_Apply` (the loaded sound options) is
//! not ported.

use crate::btl_replay::{self, Replay};
use crate::char_select::BattleSetup;
use crate::dialog::{Command, Dialog, Pad};
use crate::draw::ScreenQuad;
use crate::font::Font;
use crate::memcard::{self, McCard, SaveFile, TYPE_PS2, is_fatal_error};
use crate::save::SaveData;

/// `MCFLOW_MODE_*`.
pub const MODE_SAVE: i32 = 0;
pub const MODE_LOAD: i32 = 1;
pub const MODE_BOOT_LOAD: i32 = 2;
pub const MODE_NEW_SAVE: i32 = 3;
pub const MODE_SAVE_ALT: i32 = 4;
pub const MODE_REPLAY_SAVE: i32 = 5;
pub const MODE_REPLAY_LOAD: i32 = 6;
pub const MODE_REPLAY_SCAN: i32 = 7;
pub const MODE_REPLAY_ASK: i32 = 8;
pub const MODE_REPLAY_ERR: i32 = 9;

/// `MCFLOW_REPLAY_SLOTS`.
pub const REPLAY_SLOTS: usize = 7;

// MCFLOW_FLAG_*.
const FLAG_DONE: u32 = 8;
pub const FLAG_FOUND: u32 = 0x10;
pub const FLAG_EMPTY: u32 = 0x20;
pub const FLAG_NO_DIR: u32 = 0x40;
const FLAG_REMOVED: u32 = 0x80;
const FLAG_REFUSED: u32 = 0x100;

/// `gProgress->flags`: "continue without saving" was chosen at boot, and a save was loaded.
pub const PROGRESS_NO_SAVE: u32 = 4;
pub const PROGRESS_SAVE_LOADED: u32 = 8;

const PAD_CROSS: u32 = 0x0200;
/// The 120-frame time-out (`MCF_GO`) and the 12 frames an answer waits (`MCF_INPUT`).
const TIMEOUT: i32 = 0x78;
const ANSWER: i32 = 0xC;
/// Free clusters a system save and a replay need.
const NEED_SAVE: i32 = 0x3D;
const NEED_REPLAY: i32 = 0xA0;
/// The icon files' sizes (`McFlow_SetupRequest`).
const ICON_SIZE_SAVE: usize = 0x9C9A;
const ICON_SIZE_REPLAY: usize = 0xBE8A;
/// The card titles, full-width Shift-JIS: "DBZ-BUDOKAI-TENKAICHI3" and "DBZ-TENKAICHI3-REPLAY-0" (the last digit
/// is the slot's).
const TITLE_SAVE: &[u8] = b"\x82\x63\x82\x61\x82\x79\x81\x7C\x82\x61\x82\x74\x82\x63\x82\x6E\x82\x6A\x82\x60\x82\x68\x81\x7C\x82\x73\x82\x64\x82\x6D\x82\x6A\x82\x60\x82\x68\x82\x62\x82\x67\x82\x68\x82\x52";
const TITLE_REPLAY: &[u8] = b"\x82\x63\x82\x61\x82\x79\x81\x7C\x82\x73\x82\x64\x82\x6D\x82\x6A\x82\x60\x82\x68\x82\x62\x82\x67\x82\x68\x82\x52\x81\x7C\x82\x71\x82\x64\x82\x6F\x82\x6B\x82\x60\x82\x78\x81\x7C\x82\x4F";

/// Which done callback (`doneCb[n]` / `modeCb[mode][n]`) the flow ends with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    /// 0: the flow did what it was started for.
    Done,
    /// 1.
    NotDone,
    /// 2: the player answered "no".
    Refused,
}

/// What a frame of the flow reports to its screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// `McFlow_CallDone` (the system data's flows).
    Done(Done),
    /// `McFlow_CallModeDone` for the mode that ended (the replay flows).
    ModeDone(i32, Done),
    /// `McFlow_CallModeRemoved`: the card went away under mode `.0`.
    Removed(i32),
}

/// One entry of the replay slot list (`McFlowSlotInfo`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplaySlot {
    /// Bit 0: the entry was filled in.
    pub flags: i32,
    pub chara: [[i32; 5]; 2],
}

/// The parts of `gProgress` the flows use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub flags: u32,
    /// Bit 0: the replay menu saves the battle's replay (`ZAPROG_REPLAY_SAVE`).
    pub replay_flags: u32,
    pub slots: [ReplaySlot; REPLAY_SLOTS],
}

impl Progress {
    /// `Progress_ClearSession`'s part here: the replay flags and slot list.
    pub fn clear_session(&mut self) {
        self.replay_flags = 0;
        self.slots = [ReplaySlot::default(); REPLAY_SLOTS];
    }

    /// `Progress_ClearTeams`: every slot empty.
    pub fn clear_teams(&mut self) {
        self.slots = [ReplaySlot {
            flags: 0,
            chara: [[btl_replay::NO_CHARA; 5]; 2],
        }; REPLAY_SLOTS];
    }
}

/// What the flows work on beyond the dialog: the card, the save, the progress block and the item table
/// (`Save_SetDefaults`).
pub struct McEnv<'a> {
    pub card: &'a mut McCard,
    pub save: &'a mut SaveData,
    pub progress: &'a mut Progress,
    pub items: &'a [bt3_data::chara_data::ItemInfo],
}

/// The frame's input and output of the dialog.
pub struct McIo<'a> {
    pub dialog: &'a mut Dialog,
    pub pad: Pad,
    pub sounds: &'a mut Vec<(u32, u32)>,
}

/// `gMcFlow`.
pub struct McFlow {
    mode: i32,
    state: i32,
    next: i32,
    timer: i32,
    choice: i32,
    msg: i32,
    flags: u32,
    card_gone: i32,
    index: usize,
    slot: usize,
    req: SaveFile,
    /// The resource file's two icons (system save, replay).
    icons: [Vec<u8>; 2],
    /// `repSave`: the battle's replay as a card file, built at init.
    rep_save: Vec<u8>,
    /// `slotHdr`: the scan's headers.
    slot_hdr: [Vec<u8>; REPLAY_SLOTS],
    /// `saveBuf` and `repLoad`: what the system data's and a replay's reads fill over their frames.
    save_buf: Vec<u8>,
    rep_load: Vec<u8>,
    /// `BattleReplay_Load`'s result: the setup and replay of the file the load flow read.
    pub loaded: Option<(BattleSetup, Replay)>,
}

impl McFlow {
    /// `McFlow_Init`: the card resources (common file base + 0x1D, unpacked: word 1 the system icon, word 2 the
    /// replay icon) and the battle's replay as the file the save flow writes (`rep`: its setup and tracks, none
    /// when there was no battle).
    pub fn new(res: Option<&[u8]>, rep: Option<(&BattleSetup, &Replay)>) -> Self {
        let icon = |n: usize, size: usize| {
            res.and_then(|r| {
                let at =
                    (u32::from_le_bytes(r.get(n * 4..n * 4 + 4)?.try_into().ok()?) & !3) as usize;
                Some(r.get(at..at + size)?.to_vec())
            })
            .unwrap_or_else(|| vec![0; size])
        };
        let rep_save = match rep {
            Some((setup, replay)) => {
                btl_replay::file_bytes(&replay.to_block(setup), &btl_replay::teams(setup))
            }
            None => btl_replay::file_bytes(&[], &[[btl_replay::NO_CHARA; 5]; 2]),
        };
        Self {
            mode: 0,
            state: 0,
            next: 0,
            timer: 0,
            choice: 0,
            msg: 0,
            flags: 0,
            card_gone: 0,
            index: 0,
            slot: 0,
            req: SaveFile::default(),
            icons: [icon(1, ICON_SIZE_SAVE), icon(2, ICON_SIZE_REPLAY)],
            rep_save,
            slot_hdr: std::array::from_fn(|_| vec![0; btl_replay::HEADER_SIZE]),
            save_buf: vec![0; crate::save::SIZE],
            rep_load: vec![0; btl_replay::FILE_SIZE],
            loaded: None,
        }
    }

    /// The flow's state (0 when idle).
    #[cfg(test)]
    pub fn state(&self) -> i32 {
        self.state
    }

    /// `gMcFlow->flags` (the scan's found / empty / missing bits).
    #[cfg(test)]
    pub fn flags(&self) -> u32 {
        self.flags
    }

    /// `McFlow_SetSlot`.
    pub fn set_slot(&mut self, slot: usize) {
        self.slot = slot;
    }

    /// `McFlow_SetupRequest`: the system save (`replay` none) or replay slot `n`.
    fn setup_request(&mut self, replay: Option<usize>) {
        match replay {
            None => {
                self.req.icon = self.icons[0].clone();
                self.req.title = TITLE_SAVE.to_vec();
                self.req.title_break = 0x18;
                self.req.set_names("DBZT3", -1, false);
            }
            Some(slot) => {
                self.req.icon = self.icons[1].clone();
                let mut title = TITLE_REPLAY.to_vec();
                title[0x2D] += slot as u8 + 1;
                self.req.title = title;
                self.req.title_break = 0x1E;
                self.req.set_names("DBZT3R", slot as i32, true);
            }
        }
    }

    /// `McFlow_Start`.
    pub fn start(&mut self, mode: i32, env: &mut McEnv) {
        self.mode = mode;
        self.state = 0x23;
        match mode {
            MODE_SAVE | MODE_SAVE_ALT => {
                self.next = if mode == MODE_SAVE { 0xC } else { 0x20 };
                self.setup_request(None);
            }
            MODE_LOAD => {
                self.next = 2;
                self.setup_request(None);
            }
            MODE_BOOT_LOAD | MODE_NEW_SAVE => self.setup_request(None),
            MODE_REPLAY_SAVE | MODE_REPLAY_LOAD => self.setup_request(Some(self.slot)),
            MODE_REPLAY_SCAN => {
                self.next = 0x27;
                self.flags &= !0x70;
                env.progress.clear_teams();
            }
            MODE_REPLAY_ERR => self.next = 0x1B,
            _ => {}
        }
    }

    /// `McFlow_PollCard`: 0 busy, 1 a card is ready, 2 none or an error.
    pub fn poll_card(card: &mut McCard) -> i32 {
        if !card.get_info(0) {
            return 0;
        }
        let ok = card.port[0].card_type == TYPE_PS2 && !is_fatal_error(card.port[0].result);
        card.reset_step();
        if ok { 1 } else { 2 }
    }

    /// `McFlow_Update`: one frame of the running flow, drawn; its state (0 when idle) and what it reported.
    pub fn update(
        &mut self,
        env: &mut McEnv,
        io: &mut McIo,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) -> (i32, Vec<Event>) {
        if self.state == 0 {
            return (0, Vec::new());
        }
        let mut ev = Vec::new();
        let mut f = Frame {
            env,
            io,
            ev: &mut ev,
        };
        match self.mode {
            MODE_SAVE | MODE_SAVE_ALT => self.update_save(&mut f),
            MODE_LOAD => self.update_load(&mut f),
            MODE_BOOT_LOAD => self.update_boot_load(&mut f),
            MODE_NEW_SAVE => self.update_new_save(&mut f),
            MODE_REPLAY_SAVE => self.update_replay_save(&mut f),
            MODE_REPLAY_LOAD => self.update_replay_load(&mut f),
            MODE_REPLAY_SCAN | MODE_REPLAY_ERR => self.update_replay_scan(&mut f),
            MODE_REPLAY_ASK => self.update_replay_ask(&mut f),
            _ => {}
        }
        io.dialog.set_msg(self.msg);
        io.dialog.draw(true, font, quads);
        (self.state, ev)
    }
}

/// One frame's context.
struct Frame<'a, 'b, 'c> {
    env: &'a mut McEnv<'b>,
    io: &'a mut McIo<'c>,
    ev: &'a mut Vec<Event>,
}

impl Frame<'_, '_, '_> {
    fn card(&mut self) -> &mut McCard {
        self.env.card
    }

    fn port(&self) -> memcard::Port {
        self.env.card.port[0]
    }

    /// `MCF_OK_PRESSED() && Dialog_IsOpen()`, with the confirm sound.
    fn ok(&mut self) -> bool {
        if self.io.pad.pressed & PAD_CROSS != 0 && self.io.dialog.is_open() {
            self.io.sounds.push((1, 1));
            return true;
        }
        false
    }

    fn choices(&mut self, on: bool) {
        self.io.dialog.set_choices(on);
    }

    fn cursor(&mut self) {
        self.io.dialog.set_cursor(1);
    }

    /// `McCard_Probe(0)` and whether it came back with a fatal code.
    fn probe_fatal(&mut self) -> bool {
        self.env.card.probe(0);
        is_fatal_error(self.env.card.port[0].result)
    }
}

/// An answer the question states act on.
enum Answer {
    Waiting,
    Yes,
    No,
}

impl McFlow {
    /// `MCF_GO`.
    fn go(&mut self, state: i32) {
        self.state = state;
        self.timer = TIMEOUT;
    }

    /// A question state's common part: "yes" once its 12 frames ran out, "no" for a choice below `no_below` (0:
    /// no or cancel, -1: no only), else the dialog is read (`MCF_INPUT`).
    fn ask(&mut self, f: &mut Frame, msg: i32, no_below: i32) -> Answer {
        f.choices(true);
        self.msg = msg;
        if self.choice > 0 {
            self.timer -= 1;
            if self.timer >= 0 {
                return Answer::Waiting;
            }
            Answer::Yes
        } else if self.choice < no_below {
            Answer::No
        } else {
            self.choice = f.io.dialog.input(false, f.io.pad, f.io.sounds);
            self.timer = ANSWER;
            Answer::Waiting
        }
    }

    /// `MCF_POLL` while the timer runs: false to wait (the card check or the timer still going).
    fn polled(&mut self, f: &mut Frame) -> bool {
        f.card().get_info(0) && self.timer <= 0
    }

    /// State 0x23 of most flows: the window opens.
    fn open(&mut self, f: &mut Frame, next: i32) {
        f.io.dialog.set_layout(1);
        f.cursor();
        f.io.dialog.set_title(0);
        f.io.dialog.start(Command::Open, f.io.sounds);
        self.choice = 0;
        self.state = next;
    }

    /// States 0x25 / 0x26 of the system data's flows: the window closes, then `McFlow_CallDone`.
    fn close(&mut self, f: &mut Frame, boot: bool) {
        if self.state == 0x25 {
            f.choices(false);
            f.io.dialog.start(Command::Close, f.io.sounds);
            self.state = 0x26;
        }
        f.io.dialog.input(false, f.io.pad, f.io.sounds);
        if f.io.dialog.is_closed() {
            self.state = 0;
            if boot {
                self.flags |= FLAG_DONE;
            }
            f.ev.push(Event::Done(self.call_done()));
        }
    }

    /// States 0x25 / 0x26 of the replay flows: the window closes; when the card went away (checked after a
    /// `watch` result) the scan takes over at its "card removed" message, else `McFlow_CallModeDone`.
    fn close_replay(&mut self, f: &mut Frame, watch: u32) {
        if self.state == 0x25 {
            f.choices(false);
            f.io.dialog.start(Command::Close, f.io.sounds);
            self.state = 0x26;
        }
        f.io.dialog.input(false, f.io.pad, f.io.sounds);
        if self.flags & watch != 0 && f.probe_fatal() {
            self.flags |= FLAG_REMOVED;
            self.flags ^= watch;
            f.ev.push(Event::Removed(self.mode));
            return;
        }
        if f.io.dialog.is_closed() {
            if self.flags & FLAG_REMOVED != 0 {
                self.mode = MODE_REPLAY_SCAN;
                self.state = 0x23;
                self.next = 0x1B;
                self.flags ^= FLAG_REMOVED;
            } else {
                self.state = 0;
                f.ev.push(Event::ModeDone(self.mode, self.call_mode_done()));
            }
            f.card().reset_step();
        }
    }

    /// `McFlow_CallDone`.
    fn call_done(&mut self) -> Done {
        let done = if self.flags & FLAG_DONE != 0 {
            Done::Done
        } else if self.flags & FLAG_REFUSED != 0 {
            Done::Refused
        } else {
            Done::NotDone
        };
        self.flags &= !(FLAG_DONE | FLAG_REFUSED);
        done
    }

    /// `McFlow_CallModeDone`.
    fn call_mode_done(&mut self) -> Done {
        self.call_done()
    }

    /// `Save_SetDefaults(gSaveData)` then `McCard_Checksum` and `Progress_ClearSession`.
    fn new_save(f: &mut Frame) {
        *f.env.save = SaveData::new(f.env.items);
        f.env.progress.clear_session();
    }

    /// The card check of the save flows (state 3): while the timer runs, the card's state; then its files.
    /// `replay` picks the replay's free-space need.
    fn check_card_for_save(
        &mut self,
        f: &mut Frame,
        msg: i32,
        replay: bool,
        after_change: Option<i32>,
    ) {
        f.choices(false);
        self.msg = msg;
        self.timer -= 1;
        if self.timer >= 0 {
            if !self.polled(f) {
                return;
            }
            let p = f.port();
            if p.card_type != TYPE_PS2 || is_fatal_error(p.result) {
                self.go(0x1B);
                return;
            }
            if let Some(change) = after_change
                && p.result != 0
            {
                if f.env.progress.flags & PROGRESS_NO_SAVE != 0 {
                    f.env.progress.flags ^= PROGRESS_NO_SAVE;
                    return;
                }
                if f.probe_fatal() {
                    self.go(0x1B);
                    return;
                }
                self.state = change;
                f.cursor();
                return;
            }
            if p.format == 0 {
                self.state = 0x17;
                f.cursor();
            }
            return;
        }
        let req = self.req.clone();
        if !f.card().check_files(0, &req) {
            let p = f.port();
            if p.card_type != TYPE_PS2 {
                self.go(0x1B);
            } else if p.format == 0 {
                self.state = 0x17;
                f.cursor();
            } else if is_fatal_error(p.result) {
                self.go(0x1B);
            } else if p.result < 0 {
                if p.free < if replay { NEED_REPLAY } else { NEED_SAVE } {
                    self.go(0x1E);
                } else {
                    self.state = 6;
                }
            }
        } else {
            f.card().reset_step();
            self.state = 0x1F;
        }
    }

    /// The format question and the format (states 0x17 / 0x18 / 0x19 / 0x1A of the save flows). `no`: where a
    /// "no" leads (none: the window closes, refused); `after`: the state after the format.
    fn format_states(&mut self, f: &mut Frame, no: Option<i32>, fatal_msg_on_ask: bool) {
        match self.state {
            0x17 => {
                f.choices(true);
                self.msg = 0x18;
                if f.probe_fatal() {
                    f.choices(false);
                    self.msg = 0x1C;
                    self.state = 0x1B;
                    self.timer = TIMEOUT;
                    if fatal_msg_on_ask {
                        self.choice = 0;
                    }
                    return;
                }
                match self.ask(f, 0x18, 0) {
                    Answer::Yes => {
                        self.state = 0x18;
                        self.timer = TIMEOUT;
                        self.choice = 0;
                    }
                    Answer::No => self.say_no(f, no),
                    Answer::Waiting => {}
                }
            }
            0x18 => {
                f.choices(false);
                self.msg = 0x19;
                self.timer -= 1;
                if self.timer >= 0 {
                    if self.polled(f) && is_fatal_error(f.port().result) {
                        self.go(0x1A);
                    }
                    return;
                }
                if !f.card().format(0) {
                    if f.port().result < 0 {
                        self.go(0x1A);
                    }
                } else {
                    self.go(0x19);
                }
            }
            _ => {}
        }
    }

    /// A "no" that either goes to `state` or closes the window refused.
    fn say_no(&mut self, f: &mut Frame, state: Option<i32>) {
        self.choice = 0;
        f.cursor();
        match state {
            Some(s) => self.state = s,
            None => {
                self.state = 0x25;
                self.flags |= FLAG_REFUSED;
            }
        }
    }

    /// Flows 0 and 4: save the system data.
    fn update_save(&mut self, f: &mut Frame) {
        match self.state {
            0x23 => {
                let next = self.next;
                self.open(f, next);
            }
            0x20 | 0xC => {
                let msg = if self.state == 0x20 { 0x20 } else { 0xD };
                match self.ask(f, msg, 0) {
                    Answer::Yes => {
                        self.state = 3;
                        self.timer = TIMEOUT;
                        self.choice = 0;
                        f.card().reset_step();
                    }
                    Answer::No => self.say_no(f, Some(0x12)),
                    Answer::Waiting => {}
                }
            }
            3 => self.check_card_for_save(f, 5, false, Some(0xE)),
            0xE => match self.ask(f, 0xF, -1) {
                Answer::Yes => {
                    self.state = 3;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    f.env.card.probe(0);
                }
                Answer::No => self.say_no(f, Some(0x12)),
                Answer::Waiting => {}
            },
            0x17 | 0x18 => self.format_states(f, Some(0x12), true),
            0xD | 0x21 => {
                f.choices(true);
                self.msg = 0xE;
                if f.probe_fatal() {
                    self.state = 0x1B;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    return;
                }
                let yes = if self.state == 0xD { 0xF } else { 0x22 };
                match self.ask(f, 0xE, 0) {
                    Answer::Yes => {
                        self.state = yes;
                        self.timer = TIMEOUT;
                        self.choice = 0;
                    }
                    Answer::No => self.say_no(f, Some(0x12)),
                    Answer::Waiting => {}
                }
            }
            0xB | 0x22 => {
                f.choices(false);
                self.msg = if self.state == 0xB { 0xA } else { 0x10 };
                if self.create(f, None) {
                    self.state = 0xF;
                }
            }
            0xF => {
                f.choices(false);
                self.msg = 0x10;
                let data = f.env.save.to_image();
                self.write(f, &data, None, 0x10);
            }
            6 => {
                f.choices(false);
                self.msg = 5;
                let dir = "BASLUS-21678DBZT3";
                if !f.card().find_dir(0, dir) {
                    if is_fatal_error(f.port().result) {
                        self.go(0x1B);
                    }
                    return;
                }
                self.go(0xB);
            }
            0x1E => self.message_then(f, 0x1F, 0x12),
            0x10 => {
                f.choices(false);
                self.msg = 0x11;
                if f.ok() {
                    self.flags |= FLAG_DONE;
                    self.state = 0x25;
                    return;
                }
                self.timer -= 1;
                if self.timer < 0 {
                    self.flags |= FLAG_DONE;
                    self.state = 0x25;
                }
            }
            0x11 => self.message_then(f, 0x12, 0x12),
            0x19 => {
                f.choices(false);
                self.msg = 0x1A;
                if f.probe_fatal() {
                    self.state = 0x1B;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    return;
                }
                self.timer -= 1;
                if self.timer < 0 {
                    self.go(0xB);
                }
            }
            0x1A => self.message_then(f, 0x1B, 0x12),
            0x1B => self.message_then(f, 0x1C, 0x12),
            0x12 => match self.ask(f, 0x13, -1) {
                Answer::Yes => {
                    self.state = 0x25;
                    self.choice = 0;
                }
                Answer::No => {
                    self.state = self.next;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    f.cursor();
                    f.card().reset_step();
                }
                Answer::Waiting => {}
            },
            0x1F => self.calc_need(f, 5, false, 0x21, 0xD),
            0x25 | 0x26 => self.close(f, false),
            _ => {}
        }
    }

    /// An OK message that goes on to `then` (with the cursor on the first choice).
    fn message_then(&mut self, f: &mut Frame, msg: i32, then: i32) {
        f.choices(false);
        self.msg = msg;
        if f.ok() {
            self.state = then;
            self.choice = 0;
            f.cursor();
        }
    }

    /// `McCard_CreateSave` while the timer counts down first (`MCF_POLL`): true once made; errors go to 0x11.
    /// `replay`: the replay's states.
    fn create(&mut self, f: &mut Frame, _replay: Option<()>) -> bool {
        self.timer -= 1;
        if self.timer >= 0 {
            if self.polled(f) && is_fatal_error(f.port().result) {
                self.go(0x11);
            }
            return false;
        }
        let req = self.req.clone();
        if !f.card().create_save(0, &req) {
            if f.port().result < 0 {
                self.go(0x11);
            }
            return false;
        }
        true
    }

    /// `McCard_WriteSave` of `data` after the timer (`MCF_POLL`); errors go to 0x11, the end to `done`.
    fn write(&mut self, f: &mut Frame, data: &[u8], replay_slot: Option<usize>, done: i32) {
        self.timer -= 1;
        if self.timer >= 0 {
            if self.polled(f) && is_fatal_error(f.port().result) {
                self.go(0x11);
            }
            return;
        }
        let req = self.req.clone();
        if !f.card().write_save(0, data, &req) {
            if f.port().result < 0 {
                self.go(0x11);
            }
        } else {
            if let Some(slot) = replay_slot {
                self.copy_slot_info(f, slot, None);
            }
            self.go(done);
        }
    }

    /// `McCard_CalcNeed` (state 0x1F): with clusters still needed, to `need` (or "full" 0x1E), else `fits`.
    fn calc_need(&mut self, f: &mut Frame, msg: i32, replay: bool, need: i32, fits: i32) {
        f.choices(false);
        self.msg = msg;
        let req = self.req.clone();
        if !f.card().calc_need(0, &req, replay) {
            let p = f.port();
            if p.card_type != TYPE_PS2 {
                self.go(0x1B);
            } else if p.format == 0 {
                self.state = 0x17;
                f.cursor();
            } else if is_fatal_error(p.result) {
                self.go(0x1B);
            }
            return;
        }
        let p = f.port();
        if p.size > 0 {
            if p.free < p.size {
                self.go(0x1E);
                return;
            }
            self.go(need);
        } else {
            self.go(fits);
        }
        f.cursor();
    }

    /// `McFlow_ClearSlotInfo`: the slot present with empty teams.
    fn clear_slot_info(f: &mut Frame, slot: usize) {
        let s = &mut f.env.progress.slots[slot];
        s.flags |= 1;
        s.chara = [[btl_replay::NO_CHARA; 5]; 2];
    }

    /// `McFlow_CopySlotInfo`: the teams of the saved replay (`hdr` none) or of a header read.
    fn copy_slot_info(&mut self, f: &mut Frame, slot: usize, hdr: Option<usize>) {
        let bytes = match hdr {
            None => &self.rep_save[..btl_replay::HEADER_SIZE],
            Some(i) => self.slot_hdr[i].as_slice(),
        };
        let word = |k: usize| {
            bytes
                .get(0x10 + k * 4..0x14 + k * 4)
                .map_or(btl_replay::NO_CHARA, |b| {
                    i32::from_le_bytes(b.try_into().unwrap())
                })
        };
        let s = &mut f.env.progress.slots[slot];
        s.flags |= 1;
        s.chara = std::array::from_fn(|t| std::array::from_fn(|k| word(t * 5 + k)));
    }

    /// Flow 1: load the system data after asking.
    fn update_load(&mut self, f: &mut Frame) {
        match self.state {
            0x23 => self.open(f, 2),
            2 => match self.ask(f, 4, 0) {
                Answer::Yes => {
                    self.state = 4;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    f.card().reset_step();
                }
                Answer::No => self.say_no(f, Some(0x16)),
                Answer::Waiting => {}
            },
            4 => self.check_card_for_load(f, 5, false),
            6 => {
                f.choices(false);
                self.msg = 5;
                if !f.card().find_dir(0, "BASLUS-21678DBZT3") {
                    if is_fatal_error(f.port().result) {
                        self.go(0x1B);
                    }
                    return;
                }
                self.go(if f.port().result == 1 { 0x1D } else { 0x1C });
            }
            0x13 => self.read_save(f, false),
            0x14 => {
                f.choices(false);
                self.msg = 0x15;
                if f.ok() {
                    self.flags |= FLAG_DONE;
                    self.state = 0x25;
                    return;
                }
                self.timer -= 1;
                if self.timer < 0 {
                    self.flags |= FLAG_DONE;
                    self.state = 0x25;
                }
            }
            0x15 => self.message_then(f, 0x16, 0x16),
            0x1D => self.message_then(f, 0x1E, 0x16),
            0x1B => self.message_then(f, 0x1C, 0x16),
            0x1C => self.message_then(f, 0x1D, 0x16),
            0x16 => match self.ask(f, 0x17, -1) {
                Answer::Yes => {
                    self.state = 0x25;
                    self.choice = 0;
                }
                Answer::No => {
                    self.state = 2;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    f.cursor();
                    f.card().reset_step();
                }
                Answer::Waiting => {}
            },
            0x25 | 0x26 => self.close(f, false),
            _ => {}
        }
    }

    /// The card check of the load flows (state 4): the card's state while the timer runs, then whether the
    /// save's files open (read: 0x13, else the directory lookup 6).
    fn check_card_for_load(&mut self, f: &mut Frame, msg: i32, quick: bool) {
        f.choices(false);
        self.msg = msg;
        self.timer -= 1;
        if self.timer >= 0 {
            if !self.polled(f) {
                return;
            }
            let p = f.port();
            if p.card_type != TYPE_PS2 || is_fatal_error(p.result) {
                self.go(0x1B);
            }
            return;
        }
        let req = self.req.clone();
        if !f.card().check_files(0, &req) {
            let p = f.port();
            if p.card_type != TYPE_PS2 || is_fatal_error(p.result) {
                self.go(0x1B);
            } else if p.result < 0 {
                self.state = 6;
            }
        } else if quick {
            self.state = 0x13;
            self.timer = ANSWER;
        } else {
            self.go(0x13);
        }
    }

    /// State 0x13 of the system loads: the file read after the timer, checked, and taken as the save.
    fn read_save(&mut self, f: &mut Frame, boot: bool) {
        f.choices(false);
        self.msg = 0x14;
        self.timer -= 1;
        if self.timer >= 0 {
            if self.polled(f) && is_fatal_error(f.port().result) {
                self.go(0x15);
            }
            return;
        }
        let req = self.req.clone();
        if !f.card().read_file(0, &mut self.save_buf, &req) {
            if f.port().result < 0 {
                self.go(0x15);
            }
            return;
        }
        if f.port().flags & memcard::F_DIFF != 0 || memcard::checksum(&mut self.save_buf, false) {
            self.go(0x1D);
            return;
        }
        *f.env.save = SaveData::from_image(&self.save_buf);
        if !boot {
            f.env.progress.clear_session();
        }
        self.go(0x14);
    }

    /// Flow 2: load the system data without asking (boot).
    fn update_boot_load(&mut self, f: &mut Frame) {
        match self.state {
            0x23 => {
                self.open(f, 5);
                self.timer = TIMEOUT;
            }
            5 | 4 => {
                f.choices(false);
                self.msg = 5;
                let first = self.state == 5;
                self.timer -= 1;
                if self.timer >= 0 {
                    let ready = if first {
                        self.polled(f)
                    } else {
                        f.card().get_info(0)
                    };
                    if !ready {
                        return;
                    }
                    let p = f.port();
                    if (first && p.card_type != TYPE_PS2) || is_fatal_error(p.result) {
                        self.state = 7;
                        f.cursor();
                    }
                    return;
                }
                if first && f.port().format == 0 {
                    self.state = 0x25;
                    return;
                }
                let req = self.req.clone();
                if !f.card().check_files(0, &req) {
                    let p = f.port();
                    if (first && p.card_type != TYPE_PS2) || is_fatal_error(p.result) {
                        self.state = 7;
                        f.cursor();
                    } else if p.result < 0 {
                        self.state = 6;
                    }
                } else {
                    self.go(0x13);
                }
            }
            7 | 8 => {
                let msg = if self.state == 7 { 6 } else { 7 };
                let was = self.state;
                // A "yes" counting down holds the frame.
                let counting = self.choice > 0;
                match self.ask(f, msg, 0) {
                    Answer::Yes => {
                        self.state = 0x25;
                        if was == 7 {
                            f.env.progress.flags |= PROGRESS_NO_SAVE;
                        }
                        return;
                    }
                    Answer::No => {
                        self.choice = 0;
                        f.cursor();
                    }
                    Answer::Waiting if counting => return,
                    Answer::Waiting => {}
                }
                if !f.card().get_info(0) {
                    return;
                }
                f.card().reset_step();
                if was == 8 && self.card_gone == 1 {
                    self.state = 7;
                    f.cursor();
                    return;
                }
                let p = f.port();
                if p.card_type == TYPE_PS2 {
                    if p.format == 0 {
                        self.state = 0x25;
                        return;
                    }
                    if p.result == -1 {
                        self.go(4);
                    }
                } else {
                    self.card_gone = 1;
                    f.env.card.port[0].card_type = 0;
                    f.env.card.port[0].free = 0;
                }
            }
            6 => {
                f.choices(false);
                self.msg = 5;
                if !f.card().find_dir(0, "BASLUS-21678DBZT3") {
                    if is_fatal_error(f.port().result) {
                        self.state = 7;
                        f.cursor();
                    }
                    return;
                }
                let p = f.port();
                if p.result == 1 {
                    self.go(0x1D);
                    if p.free < NEED_SAVE {
                        self.card_gone = 4;
                        self.state = 8;
                        f.cursor();
                    }
                } else if p.free >= NEED_SAVE {
                    self.state = 0x25;
                } else {
                    self.card_gone = 4;
                    self.state = 8;
                    f.cursor();
                }
            }
            0x13 => self.read_save(f, true),
            0x14 => {
                f.choices(false);
                self.msg = 0x15;
                f.env.progress.flags |= PROGRESS_SAVE_LOADED;
                self.timer -= 1;
                if self.timer < 0 {
                    self.state = 0x25;
                }
            }
            0x15 => {
                f.choices(false);
                self.msg = 0x16;
                if f.ok() {
                    self.go(5);
                }
            }
            0x1D => self.message_then(f, 0x1E, 0x16),
            0x16 => match self.ask(f, 0x17, -1) {
                Answer::Yes => {
                    self.state = 0x25;
                    self.choice = 0;
                }
                Answer::No => {
                    self.state = 5;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    f.cursor();
                    f.card().reset_step();
                }
                Answer::Waiting => {}
            },
            0x25 | 0x26 => self.close(f, true),
            _ => {}
        }
    }

    /// Flow 3: create the system data (new game).
    fn update_new_save(&mut self, f: &mut Frame) {
        match self.state {
            0x23 => {
                let next = if f.env.progress.flags & PROGRESS_SAVE_LOADED != 0 {
                    1
                } else {
                    0xA
                };
                self.open(f, next);
            }
            // "Start a new game? Data not saved will be lost." (a save is loaded)
            1 => match self.ask(f, 3, 0) {
                Answer::Yes => {
                    self.state = 0xA;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    f.cursor();
                }
                Answer::No => {
                    self.state = 0x25;
                    self.choice = 0;
                }
                Answer::Waiting => {}
            },
            // "Start a new game without saving?"
            9 => match self.ask(f, 8, 0) {
                Answer::Yes => {
                    self.state = 0x25;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    self.flags |= FLAG_DONE;
                    Self::new_save(f);
                }
                Answer::No => {
                    self.state = 0x25;
                    self.choice = 0;
                }
                Answer::Waiting => {}
            },
            // "Do you wish to create a save file?"
            0xA => match self.ask(f, 9, 0) {
                Answer::Yes => {
                    self.state = 3;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    f.card().reset_step();
                }
                Answer::No => self.say_no(f, Some(9)),
                Answer::Waiting => {}
            },
            3 => self.check_card_for_save(f, 5, false, None),
            0x17 => {
                f.choices(true);
                self.msg = 0x18;
                if f.probe_fatal() {
                    f.choices(false);
                    self.msg = 0x1C;
                    self.state = 0x1B;
                    self.timer = TIMEOUT;
                    return;
                }
                match self.ask(f, 0x18, 0) {
                    Answer::Yes => {
                        self.state = 0x18;
                        self.timer = TIMEOUT;
                        self.choice = 0;
                    }
                    Answer::No => self.say_no(f, Some(9)),
                    Answer::Waiting => {}
                }
            }
            0x18 => self.format_states(f, Some(9), false),
            0x19 => {
                f.choices(false);
                if f.probe_fatal() {
                    self.go(0x1B);
                    return;
                }
                self.timer -= 1;
                if self.timer >= 0 {
                    self.msg = 0x1A;
                    return;
                }
                self.go(0xB);
            }
            0x1A => self.message_then(f, 0x1B, 9),
            0xD | 0x21 => {
                f.choices(true);
                self.msg = 0xE;
                let d = self.state == 0xD;
                if f.probe_fatal() {
                    self.go(0x1B);
                    if !d {
                        self.choice = 0;
                    }
                    return;
                }
                match self.ask(f, 0xE, 0) {
                    Answer::Yes => {
                        self.state = if d { 0xF } else { 0x22 };
                        self.timer = TIMEOUT;
                        self.choice = 0;
                        if d {
                            Self::new_save(f);
                        }
                    }
                    Answer::No => self.say_no(f, Some(9)),
                    Answer::Waiting => {}
                }
            }
            0xB | 0x22 => {
                f.choices(false);
                self.msg = if self.state == 0xB { 0xA } else { 0x10 };
                if self.create(f, None) {
                    Self::new_save(f);
                    self.state = 0xF;
                }
            }
            0xF => {
                f.choices(false);
                self.msg = 0x10;
                let data = f.env.save.to_image();
                self.write(f, &data, None, 0x10);
            }
            6 => {
                f.choices(false);
                self.msg = 5;
                if !f.card().find_dir(0, "BASLUS-21678DBZT3") {
                    if is_fatal_error(f.port().result) {
                        self.go(0x1B);
                    }
                    return;
                }
                self.go(0xB);
            }
            0x10 => {
                f.choices(false);
                self.msg = 0x11;
                let ok = f.ok();
                self.timer -= 1;
                if self.timer <= 0 || ok {
                    self.state = 0x25;
                    self.flags |= FLAG_DONE;
                }
            }
            0x11 => self.message_then(f, 0x12, 9),
            0x1E => self.message_then(f, 0x1F, 9),
            0x1B => self.message_then(f, 0x1C, 9),
            0x1F => self.calc_need(f, 5, false, 0x21, 0xD),
            0x25 | 0x26 => self.close(f, false),
            _ => {}
        }
    }

    /// The replay flows' "card removed" message: mode callback 3 once, then OK goes to `then`.
    fn removed_message(&mut self, f: &mut Frame, then: i32) {
        f.choices(false);
        self.msg = 0x1C;
        if self.flags & FLAG_REMOVED == 0 {
            f.ev.push(Event::Removed(self.mode));
            self.flags |= FLAG_REMOVED;
        }
        if f.ok() {
            self.state = then;
            self.choice = 0;
            f.cursor();
            self.flags &= !FLAG_REMOVED;
        }
    }

    /// The replay flows' retry question (0x12 / 0x16): "yes" closes the window (with `done` set for the
    /// question flow), "no" starts the scan over.
    fn retry_scan(&mut self, f: &mut Frame, msg: i32, done: bool) {
        match self.ask(f, msg, -1) {
            Answer::Yes => {
                self.state = 0x25;
                self.choice = 0;
                if done {
                    self.flags |= FLAG_DONE;
                }
            }
            Answer::No => {
                self.mode = MODE_REPLAY_SCAN;
                self.state = 0x27;
                self.timer = TIMEOUT;
                self.choice = 0;
                f.cursor();
                f.card().reset_step();
                self.index = 0;
            }
            Answer::Waiting => {}
        }
    }

    /// Flows 7 and 9: read the header of each of the seven replay files into the slot list.
    fn update_replay_scan(&mut self, f: &mut Frame) {
        match self.state {
            0x23 | 0x24 => {
                f.io.dialog.set_layout(1);
                f.cursor();
                f.io.dialog.set_title(0);
                let cmd = if self.state == 0x23 {
                    Command::Open
                } else {
                    Command::Show
                };
                f.io.dialog.start(cmd, f.io.sounds);
                self.choice = 0;
                self.index = 0;
                self.state = self.next;
            }
            0x27 | 0x28 => {
                if self.state == 0x28 {
                    self.index += 1;
                    if self.index >= REPLAY_SLOTS {
                        self.flags |= FLAG_DONE;
                        self.state = 0x25;
                        return;
                    }
                }
                if self.index == 0 {
                    self.flags &= !0x70;
                }
                self.setup_request(Some(self.index));
                self.state = 4;
                self.timer = if self.index != 0 { ANSWER } else { TIMEOUT };
                self.choice = 0;
                f.card().reset_step();
                self.scan_check(f);
            }
            4 => self.scan_check(f),
            6 => {
                f.choices(false);
                self.msg = 0x28;
                let dir = self.req.dir.clone();
                if !f.card().find_dir(0, &dir) {
                    if is_fatal_error(f.port().result) {
                        self.go(0x1B);
                    }
                    return;
                }
                if f.port().result == 1 {
                    Self::clear_slot_info(f, self.index);
                    self.flags |= FLAG_EMPTY;
                } else {
                    self.flags |= FLAG_NO_DIR;
                }
                self.state = 0x28;
            }
            0x13 => {
                f.choices(false);
                self.msg = 0x28;
                self.timer -= 1;
                if self.timer >= 0 {
                    if self.polled(f) && is_fatal_error(f.port().result) {
                        self.state = 0x28;
                    }
                    return;
                }
                let req = self.req.clone();
                let i = self.index;
                if !f.card().read_file(0, &mut self.slot_hdr[i], &req) {
                    if f.port().result < 0 {
                        self.state = 0x28;
                    }
                    return;
                }
                if f.port().flags & memcard::F_DIFF != 0
                    || btl_replay::header_teams(&self.slot_hdr[i]).is_none()
                {
                    Self::clear_slot_info(f, i);
                    self.flags |= FLAG_EMPTY;
                } else {
                    self.flags |= FLAG_FOUND;
                    self.copy_slot_info(f, i, Some(i));
                }
                self.state = 0x28;
            }
            0x1D | 0x1C => {
                f.choices(false);
                self.msg = if self.state == 0x1D { 0x3B } else { 0x3A };
                if f.probe_fatal() {
                    self.msg = 0x1C;
                    self.state = 0x1B;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    return;
                }
                if f.ok() {
                    f.cursor();
                    self.state = self.retry_state(f);
                }
            }
            0x1B => {
                f.choices(false);
                self.msg = 0x1C;
                if self.timer > 0 {
                    self.timer -= 1;
                    return;
                }
                if f.ok() {
                    f.cursor();
                    self.state = self.retry_state(f);
                }
            }
            0x12 | 0x16 => {
                let msg = if self.state == 0x12 { 0x13 } else { 0x17 };
                let timer = if self.state == 0x12 { TIMEOUT } else { ANSWER };
                match self.ask(f, msg, -1) {
                    Answer::Yes => {
                        self.state = 0x25;
                        self.choice = 0;
                    }
                    Answer::No => {
                        self.state = 0x27;
                        self.timer = timer;
                        self.choice = 0;
                        f.cursor();
                        f.card().reset_step();
                        self.index = 0;
                    }
                    Answer::Waiting => {}
                }
            }
            0x25 | 0x26 => self.close_replay(f, FLAG_DONE),
            _ => {}
        }
    }

    /// Where an error's OK leads in the scan: the "give up saving?" question in the replay save, else the
    /// plain one.
    fn retry_state(&self, f: &Frame) -> i32 {
        if f.env.progress.replay_flags & 1 != 0 {
            0x12
        } else {
            0x16
        }
    }

    /// State 4 of the scan: the card's state while the timer runs, then whether the slot's files open.
    fn scan_check(&mut self, f: &mut Frame) {
        f.choices(false);
        self.msg = 0x28;
        self.timer -= 1;
        if self.timer >= 0 {
            if !self.polled(f) {
                return;
            }
            let p = f.port();
            if p.card_type != TYPE_PS2 || is_fatal_error(p.result) {
                self.go(0x1B);
            }
            return;
        }
        let req = self.req.clone();
        if !f.card().check_files(0, &req) {
            let p = f.port();
            if p.card_type != TYPE_PS2 || is_fatal_error(p.result) {
                self.go(0x1B);
            } else if p.result < 0 {
                self.state = 6;
            }
        } else {
            self.state = 0x13;
            self.timer = ANSWER;
        }
    }

    /// Flow 5: save the replay into slot `slot`.
    fn update_replay_save(&mut self, f: &mut Frame) {
        match self.state {
            0x23 => {
                self.open(f, 0xC);
                f.card().reset_step();
            }
            0xC | 0xD | 0x21 => {
                let (msg, yes) = match self.state {
                    0xC => (0x2F, 3),
                    0xD => (0x30, 0xF),
                    _ => (0x30, 0x22),
                };
                f.choices(true);
                self.msg = msg;
                if f.probe_fatal() {
                    if self.state == 0xC {
                        f.choices(false);
                        self.msg = 0x1C;
                    }
                    self.state = 0x1B;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    return;
                }
                match self.ask(f, msg, 0) {
                    Answer::Yes => {
                        self.state = yes;
                        self.timer = TIMEOUT;
                        self.choice = 0;
                        if yes == 3 {
                            f.card().reset_step();
                        }
                    }
                    Answer::No => self.say_no(f, None),
                    Answer::Waiting => {}
                }
            }
            3 => self.check_card_for_save(f, 0x28, true, None),
            0x17 | 0x18 => self.format_states(f, None, true),
            0xB | 0x22 => {
                f.choices(false);
                self.msg = if self.state == 0xB { 0x2C } else { 0x32 };
                if self.create(f, Some(())) {
                    self.state = 0xF;
                }
            }
            0xF => {
                f.choices(false);
                self.msg = 0x32;
                let data = self.rep_save.clone();
                let slot = self.slot;
                self.write(f, &data, Some(slot), 0x10);
            }
            6 => {
                f.choices(false);
                self.msg = 0x28;
                let dir = self.req.dir.clone();
                if !f.card().find_dir(0, &dir) {
                    if is_fatal_error(f.port().result) {
                        self.go(0x1B);
                    }
                    return;
                }
                self.go(0xB);
            }
            0x1E => self.message_then(f, 0x3C, 0x12),
            0x10 => {
                f.choices(false);
                self.msg = 0x33;
                if f.ok() {
                    self.flags |= FLAG_DONE;
                    self.state = 0x25;
                    return;
                }
                self.timer -= 1;
                if self.timer < 0 {
                    self.flags |= FLAG_DONE;
                    self.state = 0x25;
                }
            }
            0x11 => self.message_then(f, 0x34, 0x12),
            0x19 => {
                f.choices(false);
                self.msg = 0x1A;
                if f.probe_fatal() {
                    self.state = 0x1B;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    return;
                }
                self.timer -= 1;
                if self.timer < 0 {
                    self.go(0xB);
                }
            }
            0x1A => self.message_then(f, 0x1B, 0x12),
            0x1B => self.removed_message(f, 0x12),
            0x12 => self.retry_scan(f, 0x13, false),
            0x1F => self.calc_need(f, 0x28, true, 0x21, 0xD),
            0x25 | 0x26 => self.close_replay(f, FLAG_REFUSED),
            _ => {}
        }
    }

    /// Flow 6: load the replay of slot `slot`.
    fn update_replay_load(&mut self, f: &mut Frame) {
        match self.state {
            0x23 | 2 => {
                if self.state == 0x23 {
                    self.open(f, 2);
                    f.card().reset_step();
                }
                f.choices(true);
                self.msg = 0x36;
                if f.probe_fatal() {
                    f.choices(false);
                    self.msg = 0x1C;
                    self.state = 0x1B;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    return;
                }
                match self.ask(f, 0x36, 0) {
                    Answer::Yes => {
                        self.state = 4;
                        self.timer = TIMEOUT;
                        self.choice = 0;
                        f.card().reset_step();
                    }
                    Answer::No => self.say_no(f, None),
                    Answer::Waiting => {}
                }
            }
            4 => self.check_card_for_load(f, 0x28, false),
            6 => {
                f.choices(false);
                self.msg = 0x28;
                let dir = self.req.dir.clone();
                if !f.card().find_dir(0, &dir) {
                    if is_fatal_error(f.port().result) {
                        self.go(0x1B);
                    }
                    return;
                }
                self.go(if f.port().result == 1 { 0x1D } else { 0x1C });
            }
            0x13 => {
                f.choices(false);
                self.msg = 0x37;
                self.timer -= 1;
                if self.timer >= 0 {
                    if self.polled(f) && is_fatal_error(f.port().result) {
                        self.go(0x15);
                    }
                    return;
                }
                let req = self.req.clone();
                if !f.card().read_file(0, &mut self.rep_load, &req) {
                    if f.port().result == -3 {
                        self.go(0x1D);
                    } else if f.port().result < 0 {
                        self.go(0x15);
                    }
                    return;
                }
                if f.port().flags & memcard::F_DIFF != 0
                    || memcard::checksum(&mut self.rep_load, false)
                {
                    self.go(0x1D);
                    return;
                }
                // Battle_ClearWork, BattleReplay_Load.
                self.loaded = Replay::from_block(
                    &self.rep_load
                        [btl_replay::HEADER_SIZE..btl_replay::HEADER_SIZE + btl_replay::BLOCK_SIZE],
                );
                self.go(0x14);
            }
            0x14 => {
                f.choices(false);
                self.msg = 0x38;
                if f.ok() {
                    self.flags |= FLAG_DONE;
                    self.state = 0x25;
                    return;
                }
                self.timer -= 1;
                if self.timer < 0 {
                    self.flags |= FLAG_DONE;
                    self.state = 0x25;
                }
            }
            0x15 => self.message_then(f, 0x39, 0x16),
            0x1D => self.message_then(f, 0x3B, 0x16),
            0x1C => self.message_then(f, 0x3A, 0x16),
            0x1B => self.removed_message(f, 0x16),
            0x16 => self.retry_scan(f, 0x17, false),
            0x25 | 0x26 => self.close_replay(f, FLAG_REFUSED),
            _ => {}
        }
    }

    /// Flow 8: one question about the card (message 0x3D); the answer goes to the mode callbacks.
    fn update_replay_ask(&mut self, f: &mut Frame) {
        match self.state {
            0x23 | 0x29 => {
                if self.state == 0x23 {
                    self.open(f, 0x29);
                    f.card().reset_step();
                }
                f.choices(true);
                self.msg = 0x3D;
                if f.probe_fatal() {
                    f.choices(false);
                    self.msg = 0x1C;
                    self.state = 0x1B;
                    self.timer = TIMEOUT;
                    self.choice = 0;
                    return;
                }
                match self.ask(f, 0x3D, 0) {
                    Answer::Yes => {
                        self.state = 0x25;
                        self.timer = TIMEOUT;
                        self.choice = 0;
                        f.card().reset_step();
                        self.flags |= FLAG_DONE;
                    }
                    Answer::No => self.say_no(f, None),
                    Answer::Waiting => {}
                }
            }
            0x1B => self.removed_message(f, 0x12),
            0x12 => self.retry_scan(f, 0x13, true),
            0x25 | 0x26 => self.close_replay(f, FLAG_REFUSED),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::Libmc;
    use crate::title::{PAD_CROSS as CROSS, PAD_LEFT};

    /// The title screen's dialog from the ISO in `rom/`, when it is there.
    fn real_dialog() -> Option<Dialog> {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe, pack_section};
        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let mut image = Iso9660::open(&iso).ok()?;
        let location = partition_entry_for_id(crate::title::ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).ok()?;
        let pack = archive.read_entry(&mut image, location.index).ok()?;
        let res = bpe::decode(pack_section(&pack, crate::title::ARCHIVE_SECTION).ok()?).ok()?;
        let assets = crate::title::TitleAssets::parse(&res).ok()?;
        let none = vec![None; assets.dialog.textures.len()];
        Dialog::new(
            &assets.dialog.fod,
            assets.dialog.titles.clone(),
            none,
            None,
            0,
        )
        .ok()
    }

    /// Runs a flow to its end, answering every question "yes" and every message with OK.
    fn run(flow: &mut McFlow, env: &mut McEnv, dialog: &mut Dialog) -> Vec<Event> {
        let mut font = Font::new(Vec::new(), 20);
        let mut events = Vec::new();
        for frame in 0..3000 {
            let pressed = if !dialog.is_open() {
                0
            } else if dialog.cursor() != 0 {
                PAD_LEFT
            } else if frame % 4 == 0 {
                CROSS
            } else {
                0
            };
            let mut sounds = Vec::new();
            let mut io = McIo {
                dialog: &mut *dialog,
                pad: Pad {
                    pressed,
                    repeat: pressed,
                },
                sounds: &mut sounds,
            };
            let mut quads = Vec::new();
            let (state, ev) = flow.update(env, &mut io, &mut font, &mut quads);
            events.extend(ev);
            if state == 0 {
                return events;
            }
        }
        panic!("the flow never ended (state {})", flow.state());
    }

    /// New game, boot load, replay save, scan and load against a card in a temporary folder.
    #[test]
    fn real_card_saves_and_loads_the_game_and_a_replay() {
        let Some(mut dialog) = real_dialog() else {
            return;
        };
        let root = std::env::temp_dir().join(format!("bt3-mcflow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut card = McCard::new(Libmc::new(Some(root.clone())));
        let mut save = SaveData::default();
        let mut progress = Progress::default();
        let mut env = McEnv {
            card: &mut card,
            save: &mut save,
            progress: &mut progress,
            items: &[],
        };
        // New game: "create a save file?" yes, the card checked, the save made and written.
        let mut flow = McFlow::new(None, None);
        flow.start(MODE_NEW_SAVE, &mut env);
        assert_eq!(
            run(&mut flow, &mut env, &mut dialog),
            [Event::Done(Done::Done)]
        );
        let dir = root.join("BASLUS-21678DBZT3");
        let mut data = std::fs::read(dir.join("BASLUS-21678DBZT3")).unwrap();
        assert_eq!(data.len(), crate::save::SIZE);
        assert!(!memcard::checksum(&mut data, false));
        assert!(dir.join("icon.sys").is_file() && dir.join("dbzsm.ico").is_file());

        // A new session's boot load finds it and loads it.
        env.save.rules[0] = 1;
        let mut card2 = McCard::new(Libmc::new(Some(root.clone())));
        env.card = &mut card2;
        let mut flow = McFlow::new(None, None);
        flow.start(MODE_BOOT_LOAD, &mut env);
        assert_eq!(
            run(&mut flow, &mut env, &mut dialog),
            [Event::Done(Done::Done)]
        );
        assert_eq!(env.save.rules, crate::save::DEFAULT_RULES);
        assert!(env.progress.flags & PROGRESS_SAVE_LOADED != 0);

        // A replay saved into slot 3, then the scan lists its teams.
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = 4;
        setup.sides[1].chara = 9;
        let mut replay = Replay::default();
        for f in 0..50u32 {
            replay.record(0, f, [0x7F, 0x80]);
            replay.record(1, f * 2, [0x10, 0x7F]);
        }
        let mut flow = McFlow::new(None, Some((&setup, &replay)));
        flow.set_slot(2);
        flow.start(MODE_REPLAY_SAVE, &mut env);
        assert_eq!(
            run(&mut flow, &mut env, &mut dialog),
            [Event::ModeDone(MODE_REPLAY_SAVE, Done::Done)]
        );
        assert!(root.join("BASLUS-21678DBZT3R02").is_dir());
        flow.start(MODE_REPLAY_SCAN, &mut env);
        assert_eq!(
            run(&mut flow, &mut env, &mut dialog),
            [Event::ModeDone(MODE_REPLAY_SCAN, Done::Done)]
        );
        assert!(flow.flags() & FLAG_FOUND != 0 && flow.flags() & FLAG_NO_DIR != 0);
        let slot = env.progress.slots[2];
        assert_eq!(slot.flags & 1, 1);
        assert_eq!(slot.chara[0][0], 4);
        assert_eq!(slot.chara[1][..2], [9, btl_replay::NO_CHARA]);
        assert_eq!(env.progress.slots[0].flags, 0);

        // Loading it gives the setup and the inputs back, playing.
        let mut flow = McFlow::new(None, None);
        flow.set_slot(2);
        flow.start(MODE_REPLAY_LOAD, &mut env);
        assert_eq!(
            run(&mut flow, &mut env, &mut dialog),
            [Event::ModeDone(MODE_REPLAY_LOAD, Done::Done)]
        );
        let (back, mut played) = flow.loaded.take().unwrap();
        assert_eq!((back.sides[0].chara, back.sides[1].chara), (4, 9));
        assert!(played.active && played.loaded);
        played.rewind();
        assert_eq!(played.play(1), (0, [0x10, 0x7F]));
        assert_eq!(played.play(1), (2, [0x10, 0x7F]));
        let _ = std::fs::remove_dir_all(&root);
    }
}
