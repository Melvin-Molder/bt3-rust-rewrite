//! Ultimate Battle (`Ub_Main`, src/menu/ub.c, progress modes 13..30, main-menu item 1): the group's archive
//! (`gMenuArc3`), its music and guide voices, and the part of `gProgress` its screens share.

/// `gMenuArc3`: file `gProgress->baseFile + 3`.
pub const ARCHIVE_FILE_ID: u32 = 0x1C1 + 3;
/// `UB_BGM`: the music of every screen of the group.
pub const BGM_FILE_ID: u32 = 0x10B1B;
/// `P_VOICE_BASE`: the voice bank of the group's guides.
pub const VOICE_BASE: u32 = 0x8765;

/// The group's block of `gProgress` (0x638..0x68C). Its fighters (`team`, 0x440) are the versus select's side 0
/// pick and team (`CharSelProgress`), written by the fighter and team selects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UbProgress {
    /// `ubCursor` (0x638): the plate the mode 13 menu was left on.
    pub menu_cursor: i32,
    /// `missionPage` (0x63C).
    pub mission_page: i32,
    /// `cursor` (0x640): the disc of the disc fusion, or the row of the mission list.
    pub cursor: i32,
    /// `teamSize` (0x644): the fighters the player chooses.
    pub team_size: i32,
    /// `dpRule` (0x648): 0 no DP limit, 1..3 limits 10 / 15 / 20.
    pub dp_rule: i32,
    /// `simTurn` (0x674).
    pub sim_turn: i32,
    /// `ubFlags` (0x684): the discs recognised this session, the ladder's direction.
    pub flags: i32,
    /// `ubChoice` (0x688).
    pub choice: i32,
    /// The battle setup `MisSel_SetupBattle` wrote (the rule and side 1), for the fighter select's hand-off.
    pub mission: Option<crate::mission_select::MissionSetup>,
}
