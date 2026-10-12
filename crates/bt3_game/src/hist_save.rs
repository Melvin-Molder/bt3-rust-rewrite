//! The story mode's save screen (`HistSave_Run(4)`, src/menu/history_save.c, progress mode 10): a black screen
//! that runs the memory card's save flow (`McFlow_Start(0)`) once the fade in is done and leaves 15 frames after
//! the flow reports back (done or not done).
//!
//! It goes back to the episode list after a battle's result (mode 8), to the saga select after the outro or the
//! ending movie (modes 9 and 3).

use crate::dialog::{Dialog, Pad};
use crate::draw::ScreenQuad;
use crate::font::Font;
use crate::logo::ColorFade;
use crate::mcflow::{self, Done, McEnv, McFlow, McIo};
use crate::title::{PadRepeat, TitleFrame, TitleSound};

/// `HistSave_Run(4)`: section 4 of `gMenuArc2`, whose section 1 is the dialog's file.
pub const ARCHIVE_SECTION: usize = 4;
pub const DIALOG_SECTION: usize = 1;
const FADE_FRAMES: i32 = 0x14;
const LEAVE_FRAMES: i32 = 15;

const FLAG_DONE: u32 = 1;
const FLAG_LEAVING: u32 = 2;
const FLAG_STARTED: u32 = 8;

/// Where `HistSave_Run` leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistSaveResult {
    /// 0: the saga select (mode 6).
    Sagas,
    /// 1: the episode list (mode 7).
    Episodes,
}

/// `gHistSave`.
pub struct HistSaveScreen {
    fade: ColorFade,
    dialog: Dialog,
    mcflow: McFlow,
    pad: PadRepeat,
    flags: u32,
    timer: i32,
    result: HistSaveResult,
    done: Option<HistSaveResult>,
}

impl HistSaveScreen {
    /// `HistSave_Init(4)` and `ColorFade_StartIn`; `prev_mode` is the mode the screen comes from.
    pub fn new(dialog: Dialog, card_res: Option<&[u8]>, prev_mode: i32) -> Self {
        Self {
            fade: ColorFade::start_in(true, FADE_FRAMES),
            dialog,
            mcflow: McFlow::new(card_res, None),
            pad: PadRepeat::default(),
            flags: 0,
            timer: 0,
            // HistSave_Run's last switch: after the result back to the episodes; after the ending or the outro (and
            // any other mode, whose result stays 1) the same rule as the original.
            result: match prev_mode {
                3 | 9 => HistSaveResult::Sagas,
                _ => HistSaveResult::Episodes,
            },
            done: None,
        }
    }

    pub fn result(&self) -> Option<HistSaveResult> {
        self.done
    }

    /// One frame of `HistSave_Run`'s loop.
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
        if self.done.is_some() {
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
        // HistSave_OnCardDone is the flow's done callbacks 0 and 1 (done, not done).
        if events
            .iter()
            .any(|e| matches!(e, mcflow::Event::Done(Done::Done | Done::NotDone)))
        {
            self.flags |= FLAG_DONE | FLAG_LEAVING;
            self.timer = LEAVE_FRAMES;
        }
        frame.sounds.extend(
            effects
                .into_iter()
                .map(|(bank, id)| TitleSound::Effect(bank, id)),
        );
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done() && self.flags & FLAG_STARTED == 0 {
            self.flags |= FLAG_STARTED;
            self.mcflow.start(mcflow::MODE_SAVE, mc);
        }
        if self.fade.is_fading_out() {
            return frame;
        }
        if self.fade.is_out_done() {
            self.done = Some(self.result);
            return frame;
        }
        if self.flags & FLAG_LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::{Libmc, McCard};
    use crate::save::SaveData;
    use crate::title::{PAD_CROSS, PAD_LEFT};

    /// Runs the real save screen from the ISO in `rom/`, when it is there, against an empty card in a temporary
    /// folder, answering every question "yes": the save is written and the screen leads back to the episodes.
    #[test]
    fn real_story_save_writes_the_card() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe, pack_section};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let location = partition_entry_for_id(crate::hist_select::ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets =
            crate::dialog::DialogAssets::parse(pack_section(&res, DIALOG_SECTION).unwrap())
                .unwrap();
        let none = vec![None; assets.textures.len()];
        let dialog = Dialog::new(&assets.fod, assets.titles.clone(), none, None, 0).unwrap();
        let root = std::env::temp_dir().join(format!("bt3-hist-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut card = McCard::new(Libmc::new(Some(root.clone())));
        let mut save = SaveData::default();
        save.set_level(2);
        let mut progress = crate::mcflow::Progress::default();
        let mut env = McEnv {
            card: &mut card,
            save: &mut save,
            progress: &mut progress,
            items: &[],
        };
        let mut font = Font::new(Vec::new(), 20);
        let mut screen = HistSaveScreen::new(dialog, None, 8);
        for frame in 0..4000 {
            let pressed = if !screen.dialog.is_open() {
                0
            } else if screen.dialog.cursor() != 0 {
                PAD_LEFT
            } else if frame % 4 == 0 {
                PAD_CROSS
            } else {
                0
            };
            screen.run_frame(pressed, pressed, &mut font, &mut env);
            if screen.result().is_some() {
                break;
            }
        }
        assert_eq!(screen.result(), Some(HistSaveResult::Episodes));
        let file = root.join("BASLUS-21678DBZT3").join("BASLUS-21678DBZT3");
        let written = SaveData::from_image(&std::fs::read(file).unwrap());
        assert_eq!(written.level(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
