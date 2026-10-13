//! Battle replays (src/battle/btl_replay.c; `gBattleReplay`, src/battle/battle_load.c). Each fighter's input
//! record (`BtlInput_BuildRecord`: stick bytes and button word) goes into its track on every frame it takes input,
//! up to 9000 frames; the result menu's "view replay" (reason 0x10000) restarts the battle with the replay active,
//! and the tracks then give the input back frame by frame (past their end, neutral input and the end flag, which
//! ends the battle with winner bit 0x10, reason 0x40000). A rematch (reason 0x8000) records anew. While a replay
//! plays, pad 0 left / right chooses the side whose camera is shown, up / down the HUD's replay mode (0 the
//! caption only, 1 everything with the replay mark, 2 nothing); the pads do not vibrate and the blind overlay is
//! not drawn. Saving a replay (reason 0x1000) leaves for the replay list in its save mode.

use crate::char_select::{BattleSetup, MemberStart, SideSetup};

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
    /// `loaded`: the replay came from the memory card (`BattleReplay_Load`).
    pub loaded: bool,
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

/// `sizeof(BtlReplay)`: the setup and the data block the card saves.
pub(crate) const BLOCK_SIZE: usize = 0x1_ABA8;
/// A replay file on the card: its header, the block and padding (`McFlowReplayFile`).
pub(crate) const FILE_SIZE: usize = 0x1_AC00;
/// `McFlowHeader`'s size.
pub(crate) const HEADER_SIZE: usize = 0x38;
/// `MCFLOW_NO_CHARA`: an empty team member.
pub(crate) const NO_CHARA: i32 = 0xA4;
const SETUP_SIZE: usize = 0x5A8;
const TRACK_SIZE: usize = 0xD2F8;
const MAGIC: u32 = 0x736C_7462;
const VERSION: u32 = 7;
/// `BTL_REPLAY_END` in the data's flag word.
const FLAG_END: u32 = 1;

fn put(b: &mut [u8], at: usize, v: i32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn get(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The members of a side: the lead, then its team (`memberCount` of them).
fn members(side: &SideSetup) -> Vec<(i32, i32, i32, [u16; 8])> {
    let mut m = vec![(side.chara, side.costume, side.cpu_level, side.items)];
    for i in 0..side.team_count.clamp(0, 4) as usize {
        let (chara, costume) = side.team[i];
        m.push((chara, costume, side.cpu_level, side.team_items[i]));
    }
    m
}

/// `BattleSetup` as the original lays it out (0x5A8 bytes): "btls", version 7, the rule, the two sides and their
/// members with what `BtlMember_ApplyItems` and `BattleSetup_FinishEx` work out from the items of the item table
/// (common file 4, `table`): the bonus levels (clamped to -20..20, or -20..60 for the computer in modes 1..4 and for a side at a pad
/// of a wide setup),
/// the AI type and the ability bits. The option block is left as `isSet` 0 (the battle takes the save's
/// options).
fn setup_bytes(setup: &BattleSetup, table: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; SETUP_SIZE];
    b[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    b[4..8].copy_from_slice(&VERSION.to_le_bytes());
    let rule = [
        setup.mode,
        setup.bgm,
        setup.time_limit,
        setup.announcer,
        setup.stage_change,
        setup.stage,
        0,
        setup.screen_mode,
        setup.stage,
    ];
    for (k, v) in rule.iter().enumerate() {
        put(&mut b, 8 + k * 4, *v);
    }
    for (s, side) in setup.sides.iter().enumerate() {
        let at = 0xC0 + s * 0x270;
        let m = members(side);
        put(&mut b, at, m.len() as i32);
        let wide = if side.control == 2 {
            matches!(setup.mode, 1..=4)
        } else {
            setup.wide
        };
        for (k, (chara, costume, cpu, items)) in m.iter().enumerate() {
            let mt = at + 4 + k * 0x64;
            put(&mut b, mt, *chara);
            put(&mut b, mt + 4, *costume);
            put(&mut b, mt + 0xC, *cpu);
            let health = side.members.map_or(100.0, |start| start[k.min(4)].health);
            b[mt + 0x10..mt + 0x14].copy_from_slice(&health.to_le_bytes());
            for (n, id) in items.iter().enumerate() {
                b[mt + 0x14 + n * 2..mt + 0x16 + n * 2].copy_from_slice(&id.to_le_bytes());
            }
            // BtlMember_ApplyItems (bonus 0 stays 0) and BtlMember_ClampBonus.
            let stats = bt3_data::chara_data::item_set_stats(table, items);
            let derived = crate::battle::member_items(&stats, wide);
            for (n, v) in derived.bonus.iter().enumerate() {
                put(&mut b, mt + 0x28 + n * 4, *v);
            }
            put(&mut b, mt + 0x44, stats.ai_type);
            for (n, v) in derived.ability.iter().enumerate() {
                put(&mut b, mt + 0x48 + n * 4, *v as i32);
            }
        }
        put(&mut b, at + 0x1FC, side.change_allowed);
        put(&mut b, at + 0x200, side.switch_enabled);
        put(&mut b, at + 0x204, side.pad);
        put(&mut b, at + 0x208, side.control);
        for (n, v) in [side.chara, side.costume, 0, side.chara, side.costume, 0]
            .iter()
            .enumerate()
        {
            put(&mut b, at + 0x250 + n * 4, *v);
        }
    }
    put(&mut b, 0x5A0, 1);
    b
}

/// The setup a block holds, or none for one that is not a setup ("btls" version 7).
fn setup_from(b: &[u8]) -> Option<BattleSetup> {
    if b.len() < SETUP_SIZE || get(b, 0) as u32 != MAGIC || get(b, 4) as u32 != VERSION {
        return None;
    }
    let mut setup = BattleSetup {
        mode: get(b, 8),
        bgm: get(b, 0xC),
        time_limit: get(b, 0x10),
        announcer: get(b, 0x14),
        stage_change: get(b, 0x18),
        stage: get(b, 0x1C),
        screen_mode: get(b, 0x24),
        ..BattleSetup::default()
    };
    let mut wide = false;
    for (s, side) in setup.sides.iter_mut().enumerate() {
        let at = 0xC0 + s * 0x270;
        let count = get(b, at).clamp(1, 5) as usize;
        let member = |k: usize| {
            let mt = at + 4 + k * 0x64;
            let items: [u16; 8] = std::array::from_fn(|n| {
                u16::from_le_bytes([b[mt + 0x14 + n * 2], b[mt + 0x15 + n * 2]])
            });
            (get(b, mt), get(b, mt + 4), get(b, mt + 0xC), items)
        };
        let (chara, costume, cpu, items) = member(0);
        side.chara = chara;
        side.costume = costume;
        side.cpu_level = cpu;
        side.items = items;
        side.team_count = count as i32 - 1;
        for k in 1..count {
            let (chara, costume, _, items) = member(k);
            side.team[k - 1] = (chara, costume);
            side.team_items[k - 1] = items;
        }
        side.change_allowed = get(b, at + 0x1FC);
        side.switch_enabled = get(b, at + 0x200);
        side.pad = get(b, at + 0x204);
        side.control = get(b, at + 0x208);
        let health = |k: usize| {
            let mt = at + 4 + k * 0x64 + 0x10;
            f32::from_le_bytes([b[mt], b[mt + 1], b[mt + 2], b[mt + 3]])
        };
        if (0..count).any(|k| health(k) != 100.0) {
            side.members = Some(std::array::from_fn(|k| MemberStart {
                cpu_level: if k < count { member(k).2 } else { cpu },
                health: if k < count { health(k) } else { 100.0 },
                variant: 0,
            }));
        }
        // A bonus above 20 at a pad was clamped wide (BattleSetup_FinishEx(1)).
        if side.control != 2
            && (0..count).any(|k| (0..8).any(|n| get(b, at + 4 + k * 0x64 + 0x28 + n * 4) > 20))
        {
            wide = true;
        }
    }
    setup.wide = wide;
    Some(setup)
}

impl Replay {
    /// `BattleReplay_GetBuffer`'s block: the battle's setup (`BattleSetup_FinishEx` copies it in, `table` the
    /// item table its members' derived values come from), both
    /// tracks (`BtlReplayTrack`: stick bytes, button words, count, position) and the flag words (`loaded` and
    /// the last word cleared).
    pub(crate) fn to_block(&self, setup: &BattleSetup, table: &[u8]) -> Vec<u8> {
        let mut b = setup_bytes(setup, table);
        b.resize(BLOCK_SIZE, 0);
        for (p, t) in self.tracks.iter().enumerate() {
            let at = SETUP_SIZE + p * TRACK_SIZE;
            for (f, (stick, buttons)) in t.stick.iter().zip(&t.buttons).enumerate().take(FRAMES) {
                b[at + f * 2..at + f * 2 + 2].copy_from_slice(stick);
                b[at + 0x4650 + f * 4..at + 0x4654 + f * 4].copy_from_slice(&buttons.to_le_bytes());
            }
            put(&mut b, at + 0xD2F0, t.buttons.len().min(FRAMES) as i32);
            put(&mut b, at + 0xD2F4, t.pos as i32);
        }
        put(
            &mut b,
            SETUP_SIZE + 2 * TRACK_SIZE,
            if self.end { FLAG_END as i32 } else { 0 },
        );
        put(&mut b, 0x1_AB9C, i32::from(self.active));
        b
    }

    /// `BattleReplay_Load`: a block of the right size with a setup gives that setup and the replay, playing
    /// (`active` and `loaded`).
    pub(crate) fn from_block(b: &[u8]) -> Option<(BattleSetup, Self)> {
        if b.len() != BLOCK_SIZE {
            return None;
        }
        let setup = setup_from(b)?;
        let mut replay = Replay {
            active: true,
            loaded: true,
            ..Replay::default()
        };
        for (p, t) in replay.tracks.iter_mut().enumerate() {
            let at = SETUP_SIZE + p * TRACK_SIZE;
            let count = get(b, at + 0xD2F0).clamp(0, FRAMES as i32) as usize;
            t.stick = (0..count)
                .map(|f| [b[at + f * 2], b[at + f * 2 + 1]])
                .collect();
            t.buttons = (0..count)
                .map(|f| get(b, at + 0x4650 + f * 4) as u32)
                .collect();
            t.pos = get(b, at + 0xD2F4).max(0) as usize;
        }
        replay.end = get(b, SETUP_SIZE + 2 * TRACK_SIZE) as u32 & FLAG_END != 0;
        Some((setup, replay))
    }
}

/// The two teams' characters of a setup for a replay file's header (`MCFLOW_NO_CHARA` past a side's count).
pub(crate) fn teams(setup: &BattleSetup) -> [[i32; 5]; 2] {
    setup.sides.each_ref().map(|side| {
        let m = members(side);
        std::array::from_fn(|k| m.get(k).map_or(NO_CHARA, |x| x.0))
    })
}

/// `McFlow_Init`'s replay file: the header (both teams, its own checksum over 0x10..0x38 at 0x08, the whole
/// file's at 0x00) and the block.
pub(crate) fn file_bytes(block: &[u8], teams: &[[i32; 5]; 2]) -> Vec<u8> {
    let mut f = vec![0u8; FILE_SIZE];
    for (k, c) in teams.iter().flatten().enumerate() {
        put(&mut f, 0x10 + k * 4, *c);
    }
    f[HEADER_SIZE..HEADER_SIZE + block.len().min(BLOCK_SIZE)]
        .copy_from_slice(&block[..block.len().min(BLOCK_SIZE)]);
    crate::memcard::checksum(&mut f[8..HEADER_SIZE], true);
    crate::memcard::checksum(&mut f, true);
    f
}

/// The teams of a replay file's header, when its own checksum holds.
pub(crate) fn header_teams(h: &[u8]) -> Option<[[i32; 5]; 2]> {
    if h.len() < HEADER_SIZE {
        return None;
    }
    let mut part = h[8..HEADER_SIZE].to_vec();
    if crate::memcard::checksum(&mut part, false) {
        return None;
    }
    Some(std::array::from_fn(|t| {
        std::array::from_fn(|k| get(h, 0x10 + (t * 5 + k) * 4))
    }))
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

    /// The block the card saves: the setup in the original's layout and both tracks, read back the same, with the
    /// file's header checksums holding.
    #[test]
    fn a_replay_block_round_trips() {
        let mut setup = BattleSetup {
            stage: 12,
            bgm: 9,
            time_limit: 3,
            announcer: 2,
            ..BattleSetup::default()
        };
        setup.sides[0].chara = 5;
        setup.sides[0].items = [3, 4, 0, 0, 0, 0, 0, 0];
        setup.sides[0].team_count = 1;
        setup.sides[0].team[0] = (7, 1);
        setup.sides[1] = SideSetup {
            chara: 9,
            control: 2,
            pad: 1,
            cpu_level: 13,
            change_allowed: 1,
            switch_enabled: 1,
            ..SideSetup::default()
        };
        let mut r = Replay::default();
        r.record(0, 0x10, [0x80, 0x7F]);
        r.record(0, 0x20, [0x7F, 0x7F]);
        r.record(1, 0x40, [0, 0xFE]);
        let block = r.to_block(&setup, &[]);
        assert_eq!(block.len(), BLOCK_SIZE);
        assert_eq!(&block[0..4], b"btls");
        let (back, mut played) = Replay::from_block(&block).unwrap();
        assert_eq!(back, setup);
        assert!(played.active && played.loaded);
        assert_eq!(played.play(0), (0x10, [0x80, 0x7F]));
        assert_eq!(played.play(1), (0x40, [0, 0xFE]));
        assert!(Replay::from_block(&block[1..]).is_none());
        let teams = teams(&setup);
        assert_eq!(teams[0], [5, 7, NO_CHARA, NO_CHARA, NO_CHARA]);
        let mut file = file_bytes(&block, &teams);
        assert_eq!(file.len(), FILE_SIZE);
        assert_eq!(header_teams(&file[..HEADER_SIZE]), Some(teams));
        assert!(!crate::memcard::checksum(&mut file, false));
    }

    /// With the item table of the ISO in `rom/`: a member's derived values are `ItemSet_GetStats`'s, clamped wide
    /// for the computer in modes 1..4; a member without items has none.
    #[test]
    fn real_replay_members_carry_their_item_values() {
        use bt3_data::Iso9660;
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(4);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let table = archive.read_entry(&mut image, location.index).unwrap();
        // The first items whose stat changes are not all zero.
        let item = (1..350u16)
            .find(|&id| {
                let s = bt3_data::chara_data::item_set_stats(&table, &[id, 0, 0, 0, 0, 0, 0, 0]);
                s.stat.iter().any(|&v| v != 0)
            })
            .unwrap();
        let items = [item, item, item, item, item, 0, 0, 0];
        let mut setup = BattleSetup::default();
        setup.sides[0].items = items;
        setup.sides[1].items = items;
        setup.sides[1].control = 2;
        let word = |b: &[u8], at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let stats = bt3_data::chara_data::item_set_stats(&table, &items);
        for mode in [0, 2] {
            setup.mode = mode;
            let b = Replay::default().to_block(&setup, &table);
            for side in 0..2 {
                let mt = 0xC0 + side * 0x270 + 4;
                let wide = mode == 2 && side == 1;
                let want = crate::battle::member_items(&stats, wide);
                let bonus: Vec<i32> = (0..8).map(|n| word(&b, mt + 0x24 + n * 4)).collect();
                assert_eq!(bonus[0], 0);
                assert_eq!(bonus[1..], want.bonus);
                assert_eq!(word(&b, mt + 0x44), stats.ai_type);
                let ability: Vec<u32> =
                    (0..4).map(|n| word(&b, mt + 0x48 + n * 4) as u32).collect();
                assert_eq!(ability, want.ability);
            }
        }
        // A member without items: no bonus, no ability; the AI type of character 0's entry.
        let b = Replay::default().to_block(&BattleSetup::default(), &table);
        let mt = 0xC0 + 4;
        assert!((0..8).all(|n| word(&b, mt + 0x24 + n * 4) == 0));
        let none = bt3_data::chara_data::item_set_stats(&table, &[0; 8]);
        assert_eq!(word(&b, mt + 0x44), none.ai_type);
    }
}
