//! The replay list of the Data Center (`ReplayMenu_Run(4)`, src/menu/replay_menu.c, progress mode 56): the seven
//! replay files of the memory card, one at a time with its number and the two teams' character chips. Entered
//! from the Data Center menu, confirm loads the slot's replay and the battle plays it; entered after a battle
//! that asked to save its replay (`replay_flags` bit 0), confirm saves it into the slot and the screen goes back
//! to the select screen. The card work is the McFlow modes 5..9.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::btl_replay::Replay;
use crate::char_select::BattleSetup;
use crate::dialog::{Dialog, DialogAssets, Pad};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quad};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::mcflow::{self, Done, Event, McEnv, McFlow, McIo, REPLAY_SLOTS};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::title::{PAD_CROSS, PAD_DOWN, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound};

/// `ReplayMenu_Run(4)` unpacks section 4 of `gMenuArc8`.
pub const ARCHIVE_SECTION: usize = 4;
const FADE_FRAMES: i32 = 20;
const LEAVE_FRAMES: i32 = 30;
/// Section 8: a pack of the characters' chips (character id + 1).
const CHIP_SECTION: usize = 8;
const CHIP_COUNT: usize = 0xA5;

// REPLAY_*.
const LEAVE: u32 = 1;
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const SCANNED: u32 = 8;
const PLAYABLE: u32 = 0x10;

/// `REPLAY_BUSY_*`: which card flow runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Busy {
    None,
    Scan,
    File,
    Ask,
    Removed,
}

/// How the screen ended (`REPLAY_RESULT_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayMenuResult {
    /// 0: back to the Data Center menu.
    Back,
    /// 1: a replay was loaded; the battle plays it.
    Play,
    /// 39 / 40: after saving, the progress mode to go back to.
    Select(i32),
}

/// `gReplayMenu->tex[n]` (images 0..19), the chips (20..29, set per frame) and `texB[n]` (30..38) of
/// `ReplayMenu_Init`.
const FLASH_TEXTURES: [ImageTexture; 39] = [
    tex(7, 0),
    tex(7, 1),
    tex(7, 2),
    tex(7, 3),
    tex(7, 4),
    tex(7, 5),
    tex(7, 6),
    tex(3, 0),
    tex(3, 1),
    tex(4, 0),
    tex(4, 1),
    tex(3, 2),
    tex(3, 3),
    tex(3, 4),
    tex(4, 2),
    tex(3, 5),
    tex(3, 6),
    tex(3, 7),
    tex(4, 3),
    tex(2, 0),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    tex(4, 4),
    tex(3, 8),
    tex(3, 9),
    tex(4, 5),
    tex(3, 10),
    tex(4, 6),
    tex(3, 11),
    tex(3, 12),
    tex(3, 13),
];
const CHIP_IMAGE: usize = 20;

/// What `ReplayMenu_Init` reads from its unpacked section.
pub struct ReplayMenuAssets {
    pub data: ReplayMenuData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct ReplayMenuData {
    pub fod: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    pub layout: ReplayMenuLayout,
}

#[derive(Clone, Debug, Default)]
pub struct ReplayMenuLayout {
    pub background: usize,
    pub flash: Vec<Option<usize>>,
    /// Per character id: its chip file's entry 0.
    pub chips: Vec<usize>,
    pub dialog: Vec<Option<usize>>,
}

impl ReplayMenuLayout {
    pub fn new(
        background: usize,
        dialog: Vec<Option<usize>>,
        first: impl Fn(FileKey) -> usize,
    ) -> Self {
        Self {
            background,
            flash: image_indices(&FLASH_TEXTURES, &first),
            chips: (1..=CHIP_COUNT).map(|n| first((CHIP_SECTION, n))).collect(),
            dialog,
        }
    }
}

impl ReplayMenuAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let chips: Vec<ImageTexture> = (1..=CHIP_COUNT)
            .map(|n| Some(((CHIP_SECTION, n), 0)))
            .collect();
        let files = decode_files(res, [&FLASH_TEXTURES[..], &chips])?;
        let dialog = DialogAssets::parse(pack_section(res, 5)?)?;
        Ok(Self {
            data: ReplayMenuData {
                fod: pack_section(res, 1)?.to_vec(),
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                layout: ReplayMenuLayout::default(),
            },
            background: decode_picture(res, 6)?,
            files,
            dialog,
        })
    }
}

/// `gReplayMenu` and the frame loop of `ReplayMenu_Run`.
pub struct ReplayMenuScreen {
    layout: ReplayMenuLayout,
    flash: Flash,
    dialog: Dialog,
    mcflow: McFlow,
    fade: ColorFade,
    pad: PadRepeat,
    busy: Busy,
    cursor: i32,
    timer: i32,
    flags: u32,
    mc_state: i32,
    result: ReplayMenuResult,
    /// Image 20..29's textures: the chips of the slot shown.
    textures: Vec<Option<usize>>,
    /// `ZAPROG->battleType`: where a save goes back to.
    battle_type: i32,
    done: Option<ReplayMenuResult>,
}

impl ReplayMenuScreen {
    /// `ReplayMenu_Init` and the first `ColorFade_StartIn`. `rep`: the battle's replay, which a save writes.
    pub fn new(
        data: &ReplayMenuData,
        card_res: Option<&[u8]>,
        rep: Option<(&BattleSetup, &Replay, &[u8])>,
        cursor: i32,
        battle_type: i32,
        mc: &mut McEnv,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Flash::create(&data.fod)?;
        flash.play(1);
        let dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            None,
            0,
        )?;
        mc.progress.clear_teams();
        Ok(Self {
            layout: data.layout.clone(),
            flash,
            dialog,
            mcflow: McFlow::new(card_res, rep),
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            busy: Busy::None,
            cursor,
            timer: LEAVE_FRAMES,
            flags: 0,
            mc_state: 0,
            result: ReplayMenuResult::Back,
            textures: data.layout.flash.clone(),
            battle_type,
            done: None,
        })
    }

    /// How the screen ended, once it has.
    pub fn result(&self) -> Option<ReplayMenuResult> {
        self.done
    }

    /// `ZAPROG->replayCursor` as the screen leaves it.
    pub fn cursor(&self) -> i32 {
        self.cursor
    }

    /// The replay the load flow read (`BattleReplay_Load`).
    pub fn take_loaded(&mut self) -> Option<(BattleSetup, Replay)> {
        self.mcflow.loaded.take()
    }

    fn saving(mc: &McEnv) -> bool {
        mc.progress.replay_flags & 1 != 0
    }

    /// One frame of the `ReplayMenu_Run` loop. Leaving clears the save mode.
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
        self.update(mc);
        self.draw(pressed, repeat, font, mc, &mut frame);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.check_leave(&mut frame.sounds) {
            mc.progress.replay_flags &= !1;
            self.done = Some(self.result);
            return frame;
        }
        self.input(pressed, repeat, mc, &mut frame.sounds);
        self.watch_card(mc);
        frame
    }

    /// `ReplayMenu_PlayNoData`.
    fn play_no_data(&mut self, label: &str) {
        let clip = self.flash.find(None, "mc_no_data");
        self.flash.clip_goto_label(clip, label);
    }

    /// `ReplayMenu_LightPlate`.
    fn light_plate(&mut self, on: bool) {
        let clip = self.flash.find(None, "mc_menu_play_plate");
        self.flash
            .clip_goto_label(clip, if on { "fl_on_start" } else { "fl_off_start" });
    }

    /// `ReplayMenu_SetPlayable`.
    fn set_playable(&mut self, on: bool, mc: &McEnv) {
        if on {
            self.flags |= PLAYABLE;
        } else {
            self.flags &= !PLAYABLE;
        }
        if !Self::saving(mc) {
            self.light_plate(on);
        }
    }

    /// `ReplayMenu_RefreshSlot`: the "no data" plate shown or hidden for the slot shown.
    fn refresh_slot(&mut self, mc: &McEnv) {
        let has = mc.progress.slots[self.cursor as usize].flags & 1 != 0;
        let playable = self.flags & PLAYABLE != 0;
        match (playable, has) {
            (false, false) => {
                self.play_no_data("fl_no_data_out_in");
                return;
            }
            (true, true) => return,
            (false, true) => self.play_no_data("fl_no_data_out"),
            (true, false) => self.play_no_data("fl_no_data_in"),
        }
        self.set_playable(has, mc);
    }

    /// `ReplayMenu_Leave`: back to the select screen in save mode, else to the Data Center.
    fn leave(&mut self, mc: &McEnv) {
        self.flags |= LEAVE;
        self.result = if Self::saving(mc) {
            ReplayMenuResult::Select(if self.battle_type != 0 { 40 } else { 39 })
        } else {
            ReplayMenuResult::Back
        };
    }

    /// The mode callbacks of `ReplayMenu_Init`.
    fn on_event(&mut self, event: Event, mc: &mut McEnv) {
        use Done::{Done as Ok, NotDone, Refused};
        match event {
            // ReplayMenu_OnCardRemoved, for every mode.
            Event::Removed(_) => {
                mc.progress.clear_teams();
                self.refresh_slot(mc);
            }
            Event::ModeDone(mode, done) => match (mode, done) {
                // Card scan.
                (mcflow::MODE_REPLAY_SCAN, Ok) => {
                    self.busy = Busy::None;
                    self.flags |= SCANNED;
                    self.refresh_slot(mc);
                }
                (mcflow::MODE_REPLAY_SCAN, NotDone) => {
                    mc.progress.clear_teams();
                    self.refresh_slot(mc);
                    self.busy = Busy::None;
                    self.leave(mc);
                }
                // "Leave without saving the replay?": yes (or a failure) leaves, no stays.
                (mcflow::MODE_REPLAY_ASK, Ok | NotDone) => {
                    self.busy = Busy::None;
                    self.refresh_slot(mc);
                    self.leave(mc);
                }
                (mcflow::MODE_REPLAY_ASK, Refused) => self.busy = Busy::None,
                // "The card was removed".
                (mcflow::MODE_REPLAY_ERR, Ok) => {
                    self.busy = Busy::None;
                    self.refresh_slot(mc);
                }
                (mcflow::MODE_REPLAY_ERR, NotDone) => {
                    self.busy = Busy::None;
                    self.leave(mc);
                }
                // The load: the battle plays it.
                (mcflow::MODE_REPLAY_LOAD, Ok) => {
                    self.result = ReplayMenuResult::Play;
                    self.flags |= LEAVE;
                }
                // The save: back to the select screen.
                (mcflow::MODE_REPLAY_SAVE, Ok) => {
                    self.result =
                        ReplayMenuResult::Select(if self.battle_type != 0 { 40 } else { 39 });
                    self.flags |= LEAVE;
                }
                (mcflow::MODE_REPLAY_LOAD | mcflow::MODE_REPLAY_SAVE, NotDone) => {
                    self.busy = Busy::None;
                    mc.progress.clear_teams();
                    self.refresh_slot(mc);
                    self.busy = Busy::None;
                    self.leave(mc);
                }
                (mcflow::MODE_REPLAY_LOAD | mcflow::MODE_REPLAY_SAVE, Refused) => {
                    self.busy = Busy::None
                }
                _ => {}
            },
            Event::Done(_) => {}
        }
    }

    /// `ReplayMenu_Update`: the movie; on the first frame the card scan.
    fn update(&mut self, mc: &mut McEnv) {
        self.flash.advance();
        if self.flags & STARTED == 0 {
            mc.progress.clear_teams();
            self.play_no_data("fl_no_data_in");
            self.light_plate(Self::saving(mc));
            self.busy = Busy::Scan;
            self.dialog.set_layout(1);
            self.mcflow.start(mcflow::MODE_REPLAY_SCAN, mc);
            self.flags |= STARTED;
        }
    }

    fn set_uv(&mut self, parent: Option<&str>, name: &str, col: i32, row: i32, w: i32, h: i32) {
        let clip = self.flash.find(parent, name);
        self.flash.clip_set_uv(
            clip,
            Uv {
                u0: col * w,
                v0: row * h,
                u1: col * w + w,
                v1: row * h + h,
                tex: 0,
            },
        );
    }

    /// `ReplayMenu_Draw`: the arrows, the slot number, the plate's caption, the chips, the picture, the movie,
    /// then the card flow.
    fn draw(
        &mut self,
        pressed: u32,
        repeat: u32,
        font: &mut Font,
        mc: &mut McEnv,
        frame: &mut TitleFrame,
    ) {
        self.set_uv(
            Some("mc_yajirusi_up"),
            "mc_yajirusi_icon_up",
            0,
            1,
            0x20,
            0x20,
        );
        self.set_uv(
            Some("mc_yajirusi_down"),
            "mc_yajirusi_icon_down",
            1,
            1,
            0x20,
            0x20,
        );
        // The arrow that leads nowhere is hidden (on the other slots the original looks a name up that is not
        // there).
        let hide = match self.cursor {
            0 => Some(("mc_yajirusi_up", "mc_yajirusi_icon_up")),
            c if c == REPLAY_SLOTS as i32 - 1 => {
                Some(("mc_yajirusi_down", "mc_yajirusi_icon_down"))
            }
            _ => None,
        };
        if let Some((parent, name)) = hide {
            let clip = self.flash.find(Some(parent), name);
            self.flash.clip_set_flags(clip, PROP_VISIBLE, false);
        }
        // The slot number, two digits from a four-column sheet.
        let mut num = self.cursor + 1;
        for i in (0..2).rev() {
            let p = 10i32.pow(i as u32);
            let digit = num / p;
            num -= digit * p;
            self.set_uv(
                None,
                &format!("mc_list_num_{i}"),
                digit % 4,
                digit / 4,
                0x20,
                0x20,
            );
        }
        // The plate's caption: "play", or "save" in save mode.
        let row = i32::from(Self::saving(mc));
        for child in ["mc_menu_text1_on", "mc_menu_text1_off"] {
            self.set_uv(Some("mc_menu_play_plate"), child, 0, row, 0x100, 0x20);
        }
        // ReplayMenu_DrawChips: the teams of the slot shown on the ten plates, each chip's entry 1 when the slot
        // holds a replay.
        let slot = mc.progress.slots[self.cursor as usize];
        for (team, members) in slot.chara.iter().enumerate() {
            for (i, &chara) in members.iter().enumerate() {
                let n = i + team * 5;
                self.textures[CHIP_IMAGE + n] = usize::try_from(chara)
                    .ok()
                    .and_then(|c| self.layout.chips.get(c).copied());
                let clip = self.flash.find(None, &format!("mc_chara_base_{n}"));
                self.flash.clip_set_tex(clip, slot.flags & 1);
            }
        }
        frame.quads.extend(
            picture_quads(self.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let textures = &self.textures;
        let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
        frame.quads.extend(
            self.flash
                .draw(&has_texture)
                .iter()
                .map(|q| screen_quad(q, textures)),
        );
        let mut effects = Vec::new();
        let mut io = McIo {
            dialog: &mut self.dialog,
            pad: Pad { pressed, repeat },
            sounds: &mut effects,
        };
        let (state, events) = self.mcflow.update(mc, &mut io, font, &mut frame.quads);
        self.mc_state = state;
        for e in events {
            self.on_event(e, mc);
        }
        frame
            .sounds
            .extend(effects.into_iter().map(|(b, id)| TitleSound::Effect(b, id)));
    }

    /// `ReplayMenu_WatchCard`: while no flow runs, a card that appears is scanned and one that goes away is
    /// reported.
    fn watch_card(&mut self, mc: &mut McEnv) {
        if self.busy != Busy::None || self.flags & LEAVE != 0 || self.mc_state != 0 {
            return;
        }
        match McFlow::poll_card(mc.card) {
            2 if self.flags & SCANNED != 0 => {
                mc.progress.clear_teams();
                self.refresh_slot(mc);
                self.busy = Busy::Removed;
                self.dialog.set_layout(1);
                self.mcflow.start(mcflow::MODE_REPLAY_ERR, mc);
            }
            1 if self.flags & SCANNED == 0 => {
                self.flags |= SCANNED;
                self.busy = Busy::Scan;
                self.dialog.set_layout(1);
                self.mcflow.start(mcflow::MODE_REPLAY_SCAN, mc);
            }
            _ => {}
        }
    }

    /// `ReplayMenu_Input`: up / down change the slot, confirm saves or loads, triangle leaves (asking first in
    /// save mode).
    fn input(&mut self, pressed: u32, repeat: u32, mc: &mut McEnv, sounds: &mut Vec<TitleSound>) {
        if self.busy == Busy::Ask && self.dialog.is_closed() {
            self.busy = Busy::None;
        }
        if self.flash.flags() & FLASH_PAD == 0
            || self.flags & LEAVE != 0
            || self.mc_state != 0
            || self.busy != Busy::None
        {
            return;
        }
        if repeat & (PAD_UP | PAD_DOWN) != 0 {
            let to = self.cursor + if repeat & PAD_UP != 0 { -1 } else { 1 };
            self.cursor = to.clamp(0, REPLAY_SLOTS as i32 - 1);
            if self.cursor == to {
                sounds.push(TitleSound::Effect(1, 0));
                self.refresh_slot(mc);
                self.flash.goto_label("fl_next_file", true);
            }
        } else if pressed & PAD_CROSS != 0 {
            let has = mc.progress.slots[self.cursor as usize].flags & 1 != 0;
            if Self::saving(mc) || has {
                self.busy = Busy::File;
                self.dialog.set_layout(1);
                self.mcflow.set_slot(self.cursor as usize);
                let mode = if Self::saving(mc) {
                    mcflow::MODE_REPLAY_SAVE
                } else {
                    mcflow::MODE_REPLAY_LOAD
                };
                self.mcflow.start(mode, mc);
                let clip = self.flash.find(None, "mc_menu_play_plate");
                self.flash.clip_goto_label(clip, "fl_ok");
                sounds.push(TitleSound::Effect(1, 1));
            } else {
                sounds.push(TitleSound::Effect(1, 7));
            }
        } else if pressed & PAD_TRIANGLE != 0 {
            if Self::saving(mc) {
                self.busy = Busy::Ask;
                self.dialog.set_layout(1);
                self.mcflow.start(mcflow::MODE_REPLAY_ASK, mc);
            } else {
                self.flags |= LEAVE;
                self.result = ReplayMenuResult::Back;
            }
            sounds.push(TitleSound::Effect(1, 2));
        }
    }

    /// `ReplayMenu_CheckLeave`: once left, the fade out and 30 frames; true at their end.
    fn check_leave(&mut self, sounds: &mut Vec<TitleSound>) -> bool {
        if self.flags & LEAVE == 0 || self.busy == Busy::Ask {
            return false;
        }
        if self.flags & LEAVING == 0 {
            self.flags |= LEAVING;
            self.flash.goto_label("fl_list_out", true);
            self.fade = ColorFade::start_out(true, FADE_FRAMES);
        }
        self.timer -= 1;
        if self.timer == 0 {
            return true;
        }
        if self.fade.is_fading_out() {
            sounds.push(TitleSound::BgmFadeOutStep);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::{Libmc, McCard};
    use crate::title::PAD_LEFT;

    /// The replay list from the ISO in `rom/`, when it is there (texture indices made up).
    fn real_data() -> Option<ReplayMenuData> {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};
        let iso = bt3_data::rom::for_tests();
        let mut image = Iso9660::open(&iso).ok()?;
        let location = partition_entry_for_id(crate::dc_menu::ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).ok()?;
        let pack = archive.read_entry(&mut image, location.index).ok()?;
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).ok()?).ok()?;
        let assets = ReplayMenuAssets::parse(&res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        let dialog = vec![None; assets.dialog.textures.len()];
        data.layout = ReplayMenuLayout::new(0, dialog, |key| firsts[&key]);
        Some(data)
    }

    /// Runs the screen until it ends: questions answered "yes", confirm pressed now and then.
    fn run(screen: &mut ReplayMenuScreen, mc: &mut McEnv) -> ReplayMenuResult {
        let mut font = Font::new(Vec::new(), 20);
        for frame in 0..4000 {
            let pressed = if screen.dialog.is_open() && screen.dialog.cursor() != 0 {
                PAD_LEFT
            } else if frame % 6 == 0 {
                PAD_CROSS
            } else {
                0
            };
            screen.run_frame(pressed, pressed, &mut font, mc);
            if let Some(result) = screen.result() {
                return result;
            }
        }
        panic!("the replay list never ended");
    }

    #[test]
    fn real_replay_list_loads_and_saves_replays() {
        let Some(data) = real_data() else {
            return;
        };
        let root = std::env::temp_dir().join(format!("bt3-replay-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // A replay in slot 1 (the files the save flow writes).
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = 7;
        setup.sides[1].chara = 12;
        let mut replay = Replay::default();
        replay.record(0, 0x20, [0x7F, 0x7F]);
        let file = crate::btl_replay::file_bytes(
            &replay.to_block(&setup, &[]),
            &crate::btl_replay::teams(&setup),
        );
        let dir = root.join("BASLUS-21678DBZT3R00");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("BASLUS-21678DBZT3R00"), &file).unwrap();
        std::fs::write(dir.join("icon.sys"), [0u8; 0x3C4]).unwrap();
        std::fs::write(dir.join("dbzsmr.ico"), [0u8; 0x10]).unwrap();

        let mut card = McCard::new(Libmc::new(Some(root.clone())));
        let mut save = crate::save::SaveData::default();
        let mut progress = mcflow::Progress::default();
        let mut mc = McEnv {
            card: &mut card,
            save: &mut save,
            progress: &mut progress,
            items: &[],
        };
        // From the Data Center: the scan lists slot 1, confirm loads it for the battle.
        let mut screen = ReplayMenuScreen::new(&data, None, None, 0, 0, &mut mc).unwrap();
        assert_eq!(run(&mut screen, &mut mc), ReplayMenuResult::Play);
        assert_eq!(mc.progress.slots[0].chara[0][0], 7);
        let (back, played) = screen.take_loaded().unwrap();
        assert_eq!((back.sides[0].chara, back.sides[1].chara), (7, 12));
        assert!(played.loaded && played.active);

        // After a battle: save mode writes the battle's replay into the slot shown and goes back to the select.
        mc.progress.replay_flags = 1;
        setup.sides[0].chara = 30;
        let mut screen =
            ReplayMenuScreen::new(&data, None, Some((&setup, &replay, &[])), 3, 0, &mut mc)
                .unwrap();
        assert_eq!(run(&mut screen, &mut mc), ReplayMenuResult::Select(39));
        assert_eq!(mc.progress.replay_flags & 1, 0);
        assert_eq!(mc.progress.slots[3].chara[0][0], 30);
        assert!(
            root.join("BASLUS-21678DBZT3R03")
                .join("BASLUS-21678DBZT3R03")
                .is_file()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
