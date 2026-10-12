//! Battle replays (src/battle/btl_replay.c; `gBattleReplay`, src/battle/battle_load.c). Each fighter's input
//! record (`BtlInput_BuildRecord`: stick bytes and button word) goes into its track on every frame it takes input,
//! up to 9000 frames; the result menu's "view replay" (reason 0x10000) restarts the battle with the replay active,
//! and the tracks then give the input back frame by frame (past their end, neutral input and the end flag, which
//! ends the battle with winner bit 0x10, reason 0x40000). A rematch (reason 0x8000) records anew. While a replay
//! plays, pad 0 left / right chooses the side whose camera is shown, up / down the HUD's replay mode (0 the
//! caption only, 1 everything with the replay mark, 2 nothing); the pads do not vibrate and the blind overlay is
//! not drawn. Saving a replay (reason 0x1000) leaves for a screen that is not ported.

/// `BTL_REPLAY_FRAMES`: five minutes at 30 frames a second.
pub(crate) const FRAMES: usize = 9000;
/// `BattleResult` of a replay that ran out of input.
pub(crate) const WIN_OTHER: u32 = 0x10;
pub(crate) const REASON_END: u32 = 0x4_0000;

/// `BtlReplayTrack`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Track {
    stick: Vec<[u8; 2]>,
    buttons: Vec<u32>,
    pos: usize,
}

/// `gBattleReplay` (its data, flags and active word) and the viewer (`gBtlChars->viewer`, `gBtlReplayHudMode`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Replay {
    pub tracks: [Track; 2],
    /// `BTL_REPLAY_END`.
    pub end: bool,
    pub active: bool,
    viewer_row: i32,
    viewer_side: i32,
    pub hud_mode: i32,
}

impl Replay {
    /// `BtlReplay_Rewind` (the fighters' reset): back to the start; emptied unless it plays.
    pub(crate) fn rewind(&mut self) {
        for t in &mut self.tracks {
            t.pos = 0;
            if !self.active {
                t.stick.clear();
                t.buttons.clear();
            }
        }
    }

    /// `BtlReplay_Record`.
    pub(crate) fn record(&mut self, player: usize, buttons: u32, stick: [u8; 2]) {
        let Some(t) = self.tracks.get_mut(player) else {
            return;
        };
        if t.buttons.len() >= FRAMES {
            self.end = true;
            return;
        }
        t.stick.push(stick);
        t.buttons.push(buttons);
    }

    /// `BtlReplay_Play`: the next recorded frame, or neutral input and the end flag.
    pub(crate) fn play(&mut self, player: usize) -> (u32, [u8; 2]) {
        let Some(t) = self.tracks.get_mut(player) else {
            return (0, [0x7F; 2]);
        };
        if t.pos >= t.buttons.len() {
            self.end = true;
            return (0, [0x7F; 2]);
        }
        let out = (t.buttons[t.pos], t.stick[t.pos]);
        t.pos += 1;
        out
    }

    #[cfg(test)]
    pub(crate) fn len(&self, player: usize) -> usize {
        self.tracks[player].buttons.len()
    }

    #[cfg(test)]
    pub(crate) fn pos(&self, player: usize) -> usize {
        self.tracks[player].pos
    }

    /// `BattleResult_Set`'s replay switch: a rematch records, "view replay" plays.
    pub(crate) fn on_result(&mut self, reason: u32) {
        if reason & 0x8000 != 0 {
            self.active = false;
        }
        if reason & 0x1_0000 != 0 {
            self.active = true;
        }
    }

    /// `BattleReplay_TestDataFlag` with a replay playing: its input ran out.
    pub(crate) fn ran_out(&self) -> bool {
        self.active && self.end
    }

    /// `BtlReplay_UpdateViewer` with pad 0's `gamePressed` (left 1, right 2, down 4, up 8): the side shown, and
    /// while the battle runs without a menu the HUD's replay mode.
    pub(crate) fn update_viewer(&mut self, pressed: u32, paused: bool, menu: bool) {
        if !self.active || self.end {
            return;
        }
        if pressed & 8 != 0 {
            self.viewer_row -= 1;
        }
        if pressed & 4 != 0 {
            self.viewer_row += 1;
        }
        if self.viewer_row < 0 {
            self.viewer_row = 1;
        }
        if self.viewer_row >= 2 {
            self.viewer_row = 0;
        }
        if !paused && !menu {
            if pressed & 8 != 0 {
                self.hud_mode -= 1;
            }
            if pressed & 4 != 0 {
                self.hud_mode += 1;
            }
            if self.hud_mode >= 3 {
                self.hud_mode = 0;
            }
            if self.hud_mode < 0 {
                self.hud_mode = 2;
            }
        }
        self.viewer_row = 0;
        if pressed & 1 != 0 {
            self.viewer_side -= 1;
        }
        if pressed & 2 != 0 {
            self.viewer_side += 1;
        }
        if self.viewer_row == 0 {
            if self.viewer_side < 0 {
                self.viewer_side = 1;
            }
            if self.viewer_side >= 2 {
                self.viewer_side = 0;
            }
        }
    }

    /// `BtlReplay_ResetViewer` (the fighters' reset).
    pub(crate) fn reset_viewer(&mut self) {
        self.viewer_row = 0;
        self.viewer_side = 0;
    }

    /// `BtlCam_GetDefaultView`: the side the viewer chose while a replay plays, else 0.
    pub(crate) fn default_view(&self) -> usize {
        if self.active && self.viewer_row == 0 {
            self.viewer_side as usize
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_record_then_play_back() {
        let mut r = Replay::default();
        r.record(0, 0x10, [0x80, 0x7F]);
        r.record(0, 0x20, [0x7F, 0x7F]);
        r.on_result(0x1_0000);
        r.rewind();
        assert_eq!(r.play(0), (0x10, [0x80, 0x7F]));
        assert_eq!(r.play(0), (0x20, [0x7F, 0x7F]));
        assert!(!r.ran_out());
        assert_eq!(r.play(0), (0, [0x7F; 2]));
        assert!(r.ran_out());
        // A rematch records anew.
        r.on_result(0x8000);
        r.end = false;
        r.rewind();
        assert_eq!(r.len(0), 0);
    }

    #[test]
    fn viewer_cycles_the_side_and_hud_mode() {
        let mut r = Replay {
            active: true,
            ..Replay::default()
        };
        r.update_viewer(2, false, false);
        assert_eq!(r.default_view(), 1);
        r.update_viewer(2, false, false);
        assert_eq!(r.default_view(), 0);
        r.update_viewer(8, false, false);
        assert_eq!(r.hud_mode, 2);
        r.update_viewer(4, true, false);
        assert_eq!(r.hud_mode, 2);
    }
}
