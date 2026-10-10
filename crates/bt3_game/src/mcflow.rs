//! Memory-card dialog flows (src/sys/memcard_flow.c) over the card layer (src/sys/memcard.c).
//!
//! No memory card is emulated: the card layer answers what libmc reports for an empty slot 1, so the flows take
//! their "no memory card" paths.

use crate::dialog::{Command, Dialog, Pad};
use crate::draw::ScreenQuad;
use crate::font::Font;

/// `MCFLOW_MODE_NEW_SAVE`.
pub const MODE_NEW_SAVE: i32 = 3;

const FLAG_DONE: u32 = 8;
const FLAG_REFUSED: u32 = 0x100;
const PAD_CROSS: u32 = 0x0200;

/// Which done callback `McFlow_CallDone` picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    /// `doneCb[0]`: the flow did what it was started for.
    Done,
    /// `doneCb[1]`.
    NotDone,
    /// `doneCb[2]`.
    #[allow(dead_code)]
    Refused,
}

/// `gMcCardPort[0]` and the polled-operation step, for an empty slot.
#[derive(Clone, Copy, Debug, Default)]
struct Card {
    step: i32,
    result: i32,
    card_type: i32,
}

impl Card {
    /// `McCard_GetInfo(0)`: step 0 issues sceMcGetInfo, step 1 polls sceMcSync; then it keeps returning 1.
    fn get_info(&mut self) -> bool {
        match self.step {
            0 => {
                self.result = 0;
                // sceMcGetInfo accepts the request; with no card the type comes back 0.
                self.card_type = 0;
                self.step = 1;
                false
            }
            1 => {
                // sceMcSync: the result code of an empty slot.
                self.result = -10;
                self.step = 2;
                true
            }
            _ => true,
        }
    }

    fn reset_step(&mut self) {
        self.step = 0;
    }
}

/// `McCard_IsFatalError`.
fn is_fatal_error(code: i32) -> bool {
    matches!(code, -19..=-11 | -49..=-40 | -59..=-50 | -79..=-70)
}

/// `McFlow` (`gMcFlow`).
#[derive(Debug, Default)]
pub struct McFlow {
    mode: i32,
    state: i32,
    timer: i32,
    choice: i32,
    msg: i32,
    flags: u32,
    card: Card,
}

impl McFlow {
    /// `McFlow_Start(mode)`; only the new-save flow is ported.
    pub fn start(&mut self, mode: i32) {
        self.mode = mode;
        if mode == MODE_NEW_SAVE {
            self.state = 0x23;
        }
    }

    /// `MCF_GO`.
    fn go(&mut self, state: i32) {
        self.state = state;
        self.timer = 0x78;
    }

    /// `MCF_INPUT`.
    fn input(&mut self, dialog: &mut Dialog, pad: Pad, sounds: &mut Vec<(u32, u32)>) {
        self.choice = dialog.input(false, pad, sounds);
        self.timer = 0xC;
    }

    /// `McFlow_Update`: runs the flow and returns its state (0 when idle) and the callback it finished with.
    pub fn update(
        &mut self,
        dialog: &mut Dialog,
        font: &mut Font,
        save_loaded: bool,
        pad: Pad,
        sounds: &mut Vec<(u32, u32)>,
        quads: &mut Vec<ScreenQuad>,
    ) -> (i32, Option<Done>) {
        if self.state == 0 {
            return (0, None);
        }
        let mut done = None;
        if self.mode == MODE_NEW_SAVE {
            done = self.update_new_save(dialog, save_loaded, pad, sounds);
            dialog.set_msg(self.msg);
            dialog.draw(true, font, quads);
        }
        (self.state, done)
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

    /// `McFlow_UpdateNewSave` (mode 3, New Game on the title screen), for the states an empty slot reaches.
    fn update_new_save(
        &mut self,
        dialog: &mut Dialog,
        save_loaded: bool,
        pad: Pad,
        sounds: &mut Vec<(u32, u32)>,
    ) -> Option<Done> {
        match self.state {
            0x23 => {
                dialog.set_layout(1);
                dialog.set_cursor(1);
                dialog.set_title(0);
                dialog.start(Command::Open, sounds);
                self.choice = 0;
                self.state = if save_loaded { 1 } else { 0xA };
            }
            // "Start a new game? Data not saved will be lost." (a save is loaded)
            1 => {
                dialog.set_choices(true);
                self.msg = 3;
                if self.choice > 0 {
                    self.timer -= 1;
                    if self.timer >= 0 {
                        return None;
                    }
                    self.state = 0xA;
                    self.timer = 0x78;
                    self.choice = 0;
                    dialog.set_cursor(1);
                } else if self.choice < 0 {
                    self.state = 0x25;
                    self.choice = 0;
                } else {
                    self.input(dialog, pad, sounds);
                }
            }
            // "Start a new game without saving?"
            9 => {
                dialog.set_choices(true);
                self.msg = 8;
                if self.choice > 0 {
                    self.timer -= 1;
                    if self.timer >= 0 {
                        return None;
                    }
                    self.state = 0x25;
                    self.timer = 0x78;
                    self.choice = 0;
                    self.flags |= FLAG_DONE;
                    // Save_SetDefaults, McCard_Checksum, SndOpt_Apply and Progress_ClearSession: nothing is
                    // saved yet, so the defaults are what the game already runs with.
                } else if self.choice < 0 {
                    self.state = 0x25;
                    self.choice = 0;
                } else {
                    self.input(dialog, pad, sounds);
                }
            }
            // "Do you wish to create a save file?"
            0xA => {
                dialog.set_choices(true);
                self.msg = 9;
                if self.choice > 0 {
                    self.timer -= 1;
                    if self.timer >= 0 {
                        return None;
                    }
                    self.state = 3;
                    self.timer = 0x78;
                    self.choice = 0;
                    self.card.reset_step();
                } else if self.choice < 0 {
                    self.state = 9;
                    self.choice = 0;
                    dialog.set_cursor(1);
                } else {
                    self.input(dialog, pad, sounds);
                }
            }
            // "Checking memory card (PS2) in MEMORY CARD slot 1..."
            3 => {
                dialog.set_choices(false);
                self.msg = 5;
                self.timer -= 1;
                if self.timer >= 0 {
                    // MCF_POLL.
                    if !self.card.get_info() || self.timer > 0 {
                        return None;
                    }
                    if self.card.card_type != 2 || is_fatal_error(self.card.result) {
                        self.go(0x1B);
                    }
                    // A formatted card goes on to McCard_CheckFiles; an empty slot never gets there.
                }
            }
            // "No memory card (PS2) inserted in MEMORY CARD slot 1."
            0x1B => {
                dialog.set_choices(false);
                self.msg = 0x1C;
                if pad.pressed & PAD_CROSS != 0 && dialog.is_open() {
                    sounds.push((1, 1));
                    dialog.set_cursor(1);
                    self.state = 9;
                    self.choice = 0;
                }
            }
            0x25 | 0x26 => {
                if self.state == 0x25 {
                    dialog.set_choices(false);
                    dialog.start(Command::Close, sounds);
                    self.state = 0x26;
                }
                dialog.input(false, pad, sounds);
                if dialog.is_closed() {
                    self.state = 0;
                    return Some(self.call_done());
                }
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Card, is_fatal_error};

    #[test]
    fn empty_slot_is_not_fatal_and_answers_on_the_second_poll() {
        let mut card = Card::default();
        assert!(!card.get_info());
        assert!(card.get_info());
        assert!(card.get_info());
        assert_eq!((card.card_type, card.result), (0, -10));
        assert!(!is_fatal_error(card.result));
        assert!(is_fatal_error(-11) && is_fatal_error(-79) && !is_fatal_error(-20));
    }
}
