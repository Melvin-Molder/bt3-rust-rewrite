//! The memory card check of the first run (`BootCard_Run`, src/menu/boot_card.c): over a black screen the boot
//! load flow looks for the system save and loads it, or offers to go on without one; 15 frames after it reports
//! back the screen fades out.

use crate::dialog::{Dialog, Pad};
use crate::draw::ScreenQuad;
use crate::font::Font;
use crate::logo::ColorFade;
use crate::mcflow::{self, McEnv, McFlow, McIo};
use crate::title::{PadRepeat, TitleFrame, TitleSound};

/// Menu archive 0 (`gProgress->baseFile + 0x19`) and the section `BootCard_Run(2)` unpacks; its section 1 is the
/// dialog's file.
pub const ARCHIVE_SECTION: usize = 2;
pub const DIALOG_SECTION: usize = 1;
const FADE_FRAMES: i32 = 0x14;
const LEAVE_FRAMES: i32 = 0xF;

const STARTED: u32 = 1;
const DONE: u32 = 2;
const LEAVING: u32 = 4;

/// `gBootCard`.
pub struct BootCardScreen {
    fade: ColorFade,
    dialog: Dialog,
    mcflow: McFlow,
    pad: PadRepeat,
    flags: u32,
    timer: i32,
    done: bool,
}

impl BootCardScreen {
    /// `BootCard_Init`: the dialog of the screen's file and the card flow, then `ColorFade_StartIn`.
    pub fn new(dialog: Dialog, card_res: Option<&[u8]>) -> Self {
        Self {
            fade: ColorFade::start_in(true, FADE_FRAMES),
            dialog,
            mcflow: McFlow::new(card_res, None),
            pad: PadRepeat::default(),
            flags: 0,
            timer: 0,
            done: false,
        }
    }

    /// The screen finished (`ColorFade_IsOutDone`).
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// One frame of `BootCard_Run`'s loop: the fade, the card flow (drawn), the flow started once the fade is in,
    /// the 15 frames after it reported back, the fade out.
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        font: &mut Font,
        mc: &mut McEnv,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done {
            return frame;
        }
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        let mut effects = Vec::new();
        let mut io = McIo {
            dialog: &mut self.dialog,
            pad: Pad { pressed, repeat },
            sounds: &mut effects,
        };
        let (_, events) = self.mcflow.update(mc, &mut io, font, &mut frame.quads);
        // BootCard_OnFlowDone, for both outcomes.
        if events.iter().any(|e| matches!(e, mcflow::Event::Done(_))) {
            self.flags |= DONE | LEAVING;
            self.timer = LEAVE_FRAMES;
        }
        frame.sounds.extend(
            effects
                .into_iter()
                .map(|(bank, id)| TitleSound::Effect(bank, id)),
        );
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done() && self.flags & STARTED == 0 {
            self.flags |= STARTED;
            self.mcflow.start(mcflow::MODE_BOOT_LOAD, mc);
        }
        if self.fade.is_fading_out() {
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = true;
            return frame;
        }
        if self.flags & LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        }
        frame
    }
}
