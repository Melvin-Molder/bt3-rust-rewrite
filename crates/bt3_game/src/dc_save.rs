//! `DcSave` (src/menu/dc_save.c): the "save the game" card flow the menus run when they are left after the save
//! changed (`gProgress->flags` bit 0): the Data Center's top menu and Evolution Z's. Its dialog is the screen's
//! system window; the flow is the memory card's save (mode 0), and either outcome ends it.

use crate::dialog::{Dialog, Pad};
use crate::draw::ScreenQuad;
use crate::font::Font;
use crate::mcflow::{self, McEnv, McFlow, McIo};
use crate::title::TitleSound;

/// `DcSave`.
pub struct DcSave {
    dialog: Dialog,
    mcflow: McFlow,
    started: bool,
    done: bool,
    state: i32,
}

impl DcSave {
    /// `DcSave_Init`: the dialog and the card flow, with `DcSave_OnDone` for both outcomes.
    pub fn new(dialog: Dialog, card_res: Option<&[u8]>) -> Self {
        Self {
            dialog,
            mcflow: McFlow::new(card_res, None),
            started: false,
            done: false,
            state: 0,
        }
    }

    /// `DcSave_Start`: the system save (McFlow mode 0).
    pub fn start(&mut self, mc: &mut McEnv) {
        self.mcflow.start(mcflow::MODE_SAVE, mc);
        self.started = !self.started;
    }

    /// `DcSave_Update`: the card flow, once it was started.
    pub fn update(
        &mut self,
        pressed: u32,
        repeat: u32,
        font: &mut Font,
        mc: &mut McEnv,
        quads: &mut Vec<ScreenQuad>,
        sounds: &mut Vec<TitleSound>,
    ) {
        if !self.started {
            return;
        }
        let mut effects = Vec::new();
        let mut io = McIo {
            dialog: &mut self.dialog,
            pad: Pad { pressed, repeat },
            sounds: &mut effects,
        };
        let (state, events) = self.mcflow.update(mc, &mut io, font, quads);
        self.state = state;
        // DcSave_OnDone, for both outcomes.
        if events.iter().any(|e| matches!(e, mcflow::Event::Done(_))) {
            self.done = true;
        }
        sounds.extend(effects.into_iter().map(|(b, id)| TitleSound::Effect(b, id)));
    }

    /// `DcSave_GetState`: the flow's state (0 idle), or 0 before the save was started.
    pub fn state(&self) -> i32 {
        if self.started { self.state } else { 0 }
    }

    /// `DcSave_IsDone`: no save was started, or the flow has ended.
    pub fn is_done(&self) -> bool {
        !self.started || self.done
    }

    /// `DcSave_IsStarted`.
    pub fn is_started(&self) -> bool {
        self.started
    }
}
